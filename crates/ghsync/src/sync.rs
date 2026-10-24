//! The sync procedures: clone, pull, take_remote, local_changes, push.

use std::collections::BTreeMap;
use std::fmt;
use std::io;
use std::path::{Path, PathBuf};

use crate::api::{ApiError, GitHubApi, NewEntry, UpdateError};
use crate::state::{
    FileEntry, SyncState, git_blob_sha, is_managed, is_safe_rel, local_shas, now_secs, stage,
};

#[derive(Debug)]
pub enum SyncError {
    Api(ApiError),
    Io(io::Error),
    /// The project has no `.sync`.
    NotLinked,
    /// The project already has a `.sync`.
    AlreadyLinked,
    /// The branch moved since the last pull; pull first.
    BehindRemote,
    /// A pull left files changed on both sides; resolve them first.
    ConflictsPending,
    NothingToPush,
    /// No files under the folder on that branch.
    EmptyFolder,
    /// `clone_into` needs a new or empty directory.
    NotEmptyDir,
    /// The path is not waiting in the conflict list.
    NotAConflict(String),
    /// Something GitHub returned did not add up (bad path, checksum).
    Corrupt(String),
}

impl fmt::Display for SyncError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            SyncError::Api(e) => e.fmt(f),
            SyncError::Io(e) => write!(f, "file error: {e}"),
            SyncError::NotLinked => write!(f, "this project is not linked to GitHub"),
            SyncError::AlreadyLinked => write!(f, "this project is already linked to GitHub"),
            SyncError::BehindRemote => write!(f, "the branch has new commits; pull first"),
            SyncError::ConflictsPending => {
                write!(f, "files changed on both sides must be resolved first")
            }
            SyncError::NothingToPush => write!(f, "nothing to push"),
            SyncError::NotEmptyDir => write!(f, "the project folder is not empty"),
            SyncError::NotAConflict(p) => write!(f, "{p} is not in conflict"),
            SyncError::EmptyFolder => write!(f, "no files found in that folder"),
            SyncError::Corrupt(m) => write!(f, "unexpected data: {m}"),
        }
    }
}

impl std::error::Error for SyncError {}

impl From<ApiError> for SyncError {
    fn from(e: ApiError) -> Self {
        SyncError::Api(e)
    }
}

impl From<io::Error> for SyncError {
    fn from(e: io::Error) -> Self {
        SyncError::Io(e)
    }
}

pub type Result<T> = std::result::Result<T, SyncError>;

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ChangeKind {
    Added,
    Modified,
    Deleted,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Change {
    pub path: String,
    pub kind: ChangeKind,
}

#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct PullReport {
    /// Files written from the remote (new or changed).
    pub updated: Vec<String>,
    /// Files removed because the remote removed them.
    pub deleted: Vec<String>,
    /// Changed on both sides; left alone until `take_remote`.
    pub conflicts: Vec<String>,
    /// Changed only locally; kept.
    pub kept_local: Vec<String>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PushReport {
    pub commit: String,
    pub added: usize,
    pub modified: usize,
    pub deleted: usize,
}

type Remote = BTreeMap<String, FileEntry>;

fn load_state(dir: &Path) -> Result<SyncState> {
    SyncState::load(dir)?.ok_or(SyncError::NotLinked)
}

fn normalize_folder(folder: &str) -> String {
    folder.trim().trim_matches('/').to_string()
}

/// Files of the commit under the folder, by project-relative path. Walks the folder's path
/// through shallow trees so only the folder's subtree is listed recursively.
fn remote_files(api: &dyn GitHubApi, repo: &str, folder: &str, commit: &str) -> Result<Remote> {
    let mut tree = api.commit_tree(repo, commit)?;
    for seg in folder.split('/').filter(|s| !s.is_empty()) {
        let next = api
            .tree_shallow(repo, &tree)?
            .into_iter()
            .find(|e| e.kind == "tree" && e.path == seg);
        match next {
            Some(e) => tree = e.sha,
            None => return Ok(Remote::new()),
        }
    }
    let mut out = Remote::new();
    for e in api.tree_recursive(repo, &tree)? {
        if e.kind != "blob" || e.mode == "120000" {
            continue;
        }
        if is_managed(&e.path) || !is_safe_rel(&e.path) {
            continue;
        }
        out.insert(
            e.path,
            FileEntry {
                sha: e.sha,
                mode: e.mode,
            },
        );
    }
    Ok(out)
}

fn download(api: &dyn GitHubApi, repo: &str, entry: &FileEntry) -> Result<Vec<u8>> {
    let bytes = api.blob(repo, &entry.sha)?;
    if git_blob_sha(&bytes) != entry.sha {
        return Err(SyncError::Corrupt(format!(
            "blob {} failed its checksum",
            entry.sha
        )));
    }
    Ok(bytes)
}

fn abs(dir: &Path, rel: &str) -> PathBuf {
    dir.join(rel)
}

/// Stages every write, then renames them into place, then removes. The caller writes
/// `.sync` afterwards. Downloads must already be in memory.
fn apply(dir: &Path, writes: &[(String, Vec<u8>)], removes: &[String]) -> io::Result<()> {
    let mut staged = Vec::new();
    for (rel, bytes) in writes {
        match stage(&abs(dir, rel), bytes) {
            Ok(tmp) => staged.push((tmp, abs(dir, rel))),
            Err(e) => {
                for (tmp, _) in &staged {
                    let _ = std::fs::remove_file(tmp);
                }
                return Err(e);
            }
        }
    }
    for (tmp, dest) in staged {
        std::fs::rename(&tmp, &dest)?;
    }
    for rel in removes {
        match std::fs::remove_file(abs(dir, rel)) {
            Err(e) if e.kind() != io::ErrorKind::NotFound => return Err(e),
            _ => {}
        }
    }
    Ok(())
}

/// Brings a new project directory up to date with `folder` of `repo`'s `branch`.
pub fn clone_into(
    api: &dyn GitHubApi,
    repo: &str,
    branch: &str,
    folder: &str,
    project_dir: &Path,
) -> Result<SyncState> {
    if SyncState::load(project_dir)?.is_some() {
        return Err(SyncError::AlreadyLinked);
    }
    if project_dir.is_dir() && std::fs::read_dir(project_dir)?.next().is_some() {
        return Err(SyncError::NotEmptyDir);
    }
    let folder = normalize_folder(folder);
    if folder
        .split('/')
        .any(|s| !s.is_empty() && (s == "." || s == ".."))
        || folder.contains("//")
        || folder.contains('\\')
    {
        return Err(SyncError::Corrupt(format!("bad folder {folder}")));
    }
    let head = api.branch_head(repo, branch)?;
    let remote = remote_files(api, repo, &folder, &head)?;
    if remote.is_empty() {
        return Err(SyncError::EmptyFolder);
    }
    let mut writes = Vec::new();
    for (rel, entry) in &remote {
        writes.push((rel.clone(), download(api, repo, entry)?));
    }
    std::fs::create_dir_all(project_dir)?;
    apply(project_dir, &writes, &[])?;
    let state = SyncState {
        repo: repo.to_string(),
        branch: branch.to_string(),
        folder,
        base_commit: head,
        synced_at: now_secs(),
        files: remote,
        pending_head: None,
        conflicts: Vec::new(),
    };
    state.save(project_dir)?;
    Ok(state)
}

/// Merges remote changes into the project. Local-only changes stay; files changed on both
/// sides are left alone and listed in `conflicts` (see `take_remote`).
pub fn pull(api: &dyn GitHubApi, project_dir: &Path) -> Result<PullReport> {
    let mut state = load_state(project_dir)?;
    let local = local_shas(project_dir)?;
    let head = api.branch_head(&state.repo, &state.branch)?;
    let mut report = PullReport::default();

    if head == state.base_commit && state.pending_head.is_none() {
        for (rel, entry) in &state.files {
            if local.get(rel).is_some_and(|s| *s != entry.sha) {
                report.kept_local.push(rel.clone());
            }
        }
        return Ok(report);
    }

    let remote = remote_files(api, &state.repo, &state.folder, &head)?;
    if remote.is_empty() {
        // The folder is gone (or unreachable): never delete everything because of that.
        return Err(SyncError::EmptyFolder);
    }
    let mut files = state.files.clone();
    let mut to_write: Vec<(String, FileEntry)> = Vec::new();
    let mut removes = Vec::new();

    for (rel, r) in &remote {
        let base = state.files.get(rel);
        let l = local.get(rel);
        match (base, l) {
            (None, None) => to_write.push((rel.clone(), r.clone())),
            (None, Some(ls)) if *ls == r.sha => {
                files.insert(rel.clone(), r.clone());
            }
            (None, Some(_)) => report.conflicts.push(rel.clone()),
            (Some(b), l) if b.sha == r.sha => {
                // Remote unchanged: local state stays as it is.
                if l.is_some_and(|ls| *ls != b.sha) {
                    report.kept_local.push(rel.clone());
                }
            }
            (Some(_), None) => report.conflicts.push(rel.clone()),
            (Some(b), Some(ls)) if *ls == b.sha => to_write.push((rel.clone(), r.clone())),
            (Some(_), Some(ls)) if *ls == r.sha => {
                files.insert(rel.clone(), r.clone());
            }
            (Some(_), Some(_)) => report.conflicts.push(rel.clone()),
        }
    }
    for (rel, b) in &state.files {
        if remote.contains_key(rel) {
            continue;
        }
        match local.get(rel) {
            None => {
                files.remove(rel);
            }
            Some(ls) if *ls == b.sha => {
                removes.push(rel.clone());
                files.remove(rel);
            }
            Some(_) => report.conflicts.push(rel.clone()),
        }
    }

    // Download everything before touching the disk.
    let mut writes = Vec::new();
    for (rel, entry) in &to_write {
        writes.push((rel.clone(), download(api, &state.repo, entry)?));
    }
    apply(project_dir, &writes, &removes)?;
    for (rel, entry) in to_write {
        report.updated.push(rel.clone());
        files.insert(rel, entry);
    }
    report.deleted = removes;
    report.conflicts.sort();

    state.files = files;
    state.synced_at = now_secs();
    if report.conflicts.is_empty() {
        state.base_commit = head;
        state.pending_head = None;
        state.conflicts.clear();
    } else {
        state.pending_head = Some(head);
        state.conflicts = report.conflicts.clone();
    }
    state.save(project_dir)?;
    Ok(report)
}

fn conflict_context(
    api: &dyn GitHubApi,
    project_dir: &Path,
    path: &str,
) -> Result<(SyncState, String, Remote)> {
    if !is_safe_rel(path) || is_managed(path) {
        return Err(SyncError::Corrupt(format!("bad path {path}")));
    }
    let state = load_state(project_dir)?;
    let pending = match &state.pending_head {
        Some(c) if state.conflicts.iter().any(|x| x == path) => c.clone(),
        _ => return Err(SyncError::NotAConflict(path.to_string())),
    };
    let remote = remote_files(api, &state.repo, &state.folder, &pending)?;
    Ok((state, pending, remote))
}

/// Removes `path` from the conflicts; the base moves to the pulled commit with the last one.
fn finish_conflict(state: &mut SyncState, path: &str) {
    state.conflicts.retain(|c| c != path);
    if state.conflicts.is_empty()
        && let Some(c) = state.pending_head.take()
    {
        state.base_commit = c;
    }
    state.synced_at = now_secs();
}

/// First free `<name>.local`, `<name>.local.2`, ...
fn local_copy_path(dest: &Path) -> PathBuf {
    let first = PathBuf::from(format!("{}.local", dest.display()));
    let mut n = 2;
    let mut p = first.clone();
    while p.exists() {
        p = PathBuf::from(format!("{}.{n}", first.display()));
        n += 1;
    }
    p
}

/// Conflict resolution "remote wins": replaces the local `path` with the remote version,
/// keeping the local one as `<path>.local`.
pub fn take_remote(api: &dyn GitHubApi, project_dir: &Path, path: &str) -> Result<()> {
    let (mut state, _pending, remote) = conflict_context(api, project_dir, path)?;
    let remote_entry = remote.get(path);
    let bytes = match remote_entry {
        Some(e) => Some(download(api, &state.repo, e)?),
        None => None,
    };
    let dest = abs(project_dir, path);
    let keep = local_copy_path(&dest);
    let staged = match &bytes {
        Some(b) => Some(stage(&dest, b)?),
        None => None,
    };
    let moved = dest.exists();
    let result = (|| -> io::Result<()> {
        if moved {
            std::fs::rename(&dest, &keep)?;
        }
        if let Some(tmp) = &staged
            && let Err(e) = std::fs::rename(tmp, &dest)
        {
            if moved {
                let _ = std::fs::rename(&keep, &dest);
            }
            return Err(e);
        }
        Ok(())
    })();
    if let Err(e) = result {
        if let Some(tmp) = &staged {
            let _ = std::fs::remove_file(tmp);
        }
        return Err(e.into());
    }
    match remote_entry {
        Some(e) => {
            state.files.insert(path.to_string(), e.clone());
        }
        None => {
            state.files.remove(path);
        }
    }
    finish_conflict(&mut state, path);
    state.save(project_dir)?;
    Ok(())
}

/// Conflict resolution "mine wins": the local file stays as it is and counts as a local
/// change on top of the pulled commit, so the next push sends it as a fast-forward.
pub fn keep_local(api: &dyn GitHubApi, project_dir: &Path, path: &str) -> Result<()> {
    let (mut state, _pending, remote) = conflict_context(api, project_dir, path)?;
    match remote.get(path) {
        Some(e) => {
            state.files.insert(path.to_string(), e.clone());
        }
        None => {
            state.files.remove(path);
        }
    }
    finish_conflict(&mut state, path);
    state.save(project_dir)?;
    Ok(())
}

/// Local files that differ from what was last synced.
pub fn local_changes(project_dir: &Path) -> Result<Vec<Change>> {
    let state = load_state(project_dir)?;
    let local = local_shas(project_dir)?;
    let mut out = Vec::new();
    for (rel, sha) in &local {
        match state.files.get(rel) {
            None => out.push(Change {
                path: rel.clone(),
                kind: ChangeKind::Added,
            }),
            Some(b) if b.sha != *sha => out.push(Change {
                path: rel.clone(),
                kind: ChangeKind::Modified,
            }),
            Some(_) => {}
        }
    }
    for rel in state.files.keys() {
        if !local.contains_key(rel) {
            out.push(Change {
                path: rel.clone(),
                kind: ChangeKind::Deleted,
            });
        }
    }
    out.sort_by(|a, b| a.path.cmp(&b.path));
    Ok(out)
}

/// Commits the local changes to the branch as one commit.
pub fn push(api: &dyn GitHubApi, project_dir: &Path, message: &str) -> Result<PushReport> {
    let mut state = load_state(project_dir)?;
    if state.pending_head.is_some() || !state.conflicts.is_empty() {
        return Err(SyncError::ConflictsPending);
    }
    let changes = local_changes(project_dir)?;
    if changes.is_empty() {
        return Err(SyncError::NothingToPush);
    }
    if api.branch_head(&state.repo, &state.branch)? != state.base_commit {
        return Err(SyncError::BehindRemote);
    }
    let base_tree = api.commit_tree(&state.repo, &state.base_commit)?;
    let mut entries = Vec::new();
    let mut new_files = state.files.clone();
    let (mut added, mut modified, mut deleted) = (0, 0, 0);
    for c in &changes {
        match c.kind {
            ChangeKind::Deleted => {
                let mode = state.files[&c.path].mode.clone();
                entries.push(NewEntry {
                    path: state.repo_path(&c.path),
                    sha: None,
                    mode,
                });
                new_files.remove(&c.path);
                deleted += 1;
            }
            ChangeKind::Added | ChangeKind::Modified => {
                let bytes = std::fs::read(abs(project_dir, &c.path))?;
                let sha = api.create_blob(&state.repo, &bytes)?;
                if sha != git_blob_sha(&bytes) {
                    return Err(SyncError::Corrupt("blob sha mismatch".into()));
                }
                let mode = match state.files.get(&c.path) {
                    Some(old) => old.mode.clone(),
                    None => "100644".to_string(),
                };
                entries.push(NewEntry {
                    path: state.repo_path(&c.path),
                    sha: Some(sha.clone()),
                    mode: mode.clone(),
                });
                new_files.insert(c.path.clone(), FileEntry { sha, mode });
                if c.kind == ChangeKind::Added {
                    added += 1
                } else {
                    modified += 1
                }
            }
        }
    }
    let tree = api.create_tree(&state.repo, &base_tree, entries)?;
    let commit = api.create_commit(&state.repo, message, &tree, &state.base_commit)?;
    match api.update_ref(&state.repo, &state.branch, &commit) {
        Ok(()) => {}
        Err(UpdateError::NotFastForward) => return Err(SyncError::BehindRemote),
        Err(UpdateError::Other(e)) => return Err(e.into()),
    }
    state.base_commit = commit.clone();
    state.files = new_files;
    state.synced_at = now_secs();
    state.save(project_dir)?;
    Ok(PushReport {
        commit,
        added,
        modified,
        deleted,
    })
}
