//! The `.sync` file and local file helpers (scanning, hashing, atomic writes).

use std::collections::BTreeMap;
use std::io;
use std::path::{Component, Path, PathBuf};
use std::sync::atomic::{AtomicU64, Ordering};

use serde::{Deserialize, Serialize};
use sha1::{Digest, Sha1};

pub const SYNC_FILE: &str = ".sync";

/// Git's blob object id for `bytes` (`sha1("blob <len>\0" + bytes)`).
pub fn git_blob_sha(bytes: &[u8]) -> String {
    let mut h = Sha1::new();
    h.update(format!("blob {}\0", bytes.len()).as_bytes());
    h.update(bytes);
    h.finalize().iter().map(|b| format!("{b:02x}")).collect()
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct FileEntry {
    /// Git blob sha of the content as last synced.
    pub sha: String,
    /// Git file mode ("100644", "100755").
    pub mode: String,
}

/// Contents of `.sync`.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct SyncState {
    /// `owner/name`.
    pub repo: String,
    pub branch: String,
    /// Folder in the repository ("" = root), no leading or trailing slash.
    pub folder: String,
    /// Commit the files were last brought up to date with.
    pub base_commit: String,
    /// Unix seconds of the last pull or push.
    #[serde(default)]
    pub synced_at: u64,
    /// Synced files by path relative to the folder.
    #[serde(default)]
    pub files: BTreeMap<String, FileEntry>,
    /// Set while a pull left conflicts: the remote commit that pull looked at.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub pending_head: Option<String>,
    /// Paths changed on both sides, waiting for `take_remote`.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub conflicts: Vec<String>,
}

impl SyncState {
    pub fn path(project_dir: &Path) -> PathBuf {
        project_dir.join(SYNC_FILE)
    }

    /// `Ok(None)` if the project is not linked to GitHub.
    pub fn load(project_dir: &Path) -> io::Result<Option<SyncState>> {
        match std::fs::read(Self::path(project_dir)) {
            Ok(bytes) => serde_json::from_slice(&bytes)
                .map(Some)
                .map_err(|e| io::Error::new(io::ErrorKind::InvalidData, format!(".sync: {e}"))),
            Err(e) if e.kind() == io::ErrorKind::NotFound => Ok(None),
            Err(e) => Err(e),
        }
    }

    /// Writes through a temporary file, then replaces.
    pub fn save(&self, project_dir: &Path) -> io::Result<()> {
        let text = serde_json::to_vec_pretty(self).map_err(io::Error::other)?;
        write_atomic(&Self::path(project_dir), &text)
    }

    /// Repository path for a project-relative path.
    pub fn repo_path(&self, rel: &str) -> String {
        if self.folder.is_empty() {
            rel.to_string()
        } else {
            format!("{}/{}", self.folder, rel)
        }
    }
}

/// Files petaricon manages itself; never synced in either direction.
pub fn is_managed(rel: &str) -> bool {
    let mut parts = rel.split('/');
    let first = parts.next().unwrap_or("");
    if first == ".recovery" {
        return true;
    }
    let name = rel.rsplit('/').next().unwrap_or(rel);
    matches!(
        name,
        ".sync" | ".preview" | ".settings" | ".last-open" | ".github-token"
    ) || name.starts_with(".ghsync-tmp")
        || name.ends_with(".bak")
        || name.ends_with(".local")
}

/// A relative path is safe if it has only normal components.
pub fn is_safe_rel(rel: &str) -> bool {
    !rel.is_empty()
        && !rel.contains('\\')
        && Path::new(rel)
            .components()
            .all(|c| matches!(c, Component::Normal(_)))
}

static TMP_COUNTER: AtomicU64 = AtomicU64::new(0);

fn tmp_name(path: &Path) -> PathBuf {
    let n = TMP_COUNTER.fetch_add(1, Ordering::Relaxed);
    path.with_file_name(format!(".ghsync-tmp-{}-{n}", std::process::id()))
}

/// Writes `bytes` to a temp file next to `path` (creating folders). Returns the temp path.
pub fn stage(path: &Path, bytes: &[u8]) -> io::Result<PathBuf> {
    if let Some(parent) = path.parent() {
        std::fs::create_dir_all(parent)?;
    }
    let tmp = tmp_name(path);
    let r = std::fs::write(&tmp, bytes).and_then(|()| std::fs::File::open(&tmp)?.sync_all());
    if let Err(e) = r {
        let _ = std::fs::remove_file(&tmp);
        return Err(e);
    }
    Ok(tmp)
}

pub fn write_atomic(path: &Path, bytes: &[u8]) -> io::Result<()> {
    let tmp = stage(path, bytes)?;
    std::fs::rename(&tmp, path).inspect_err(|_| {
        let _ = std::fs::remove_file(&tmp);
    })
}

/// Syncable local files: relative path (with `/`) -> absolute path.
pub fn scan_local(project_dir: &Path) -> io::Result<BTreeMap<String, PathBuf>> {
    fn walk(dir: &Path, rel: &str, out: &mut BTreeMap<String, PathBuf>) -> io::Result<()> {
        for entry in std::fs::read_dir(dir)? {
            let entry = entry?;
            let Some(name) = entry.file_name().to_str().map(str::to_string) else {
                continue;
            };
            let rel_path = if rel.is_empty() {
                name
            } else {
                format!("{rel}/{name}")
            };
            if is_managed(&rel_path) {
                continue;
            }
            let ft = entry.file_type()?;
            if ft.is_dir() {
                walk(&entry.path(), &rel_path, out)?;
            } else if ft.is_file() {
                out.insert(rel_path, entry.path());
            }
        }
        Ok(())
    }
    let mut out = BTreeMap::new();
    walk(project_dir, "", &mut out)?;
    Ok(out)
}

/// Local files with their blob shas.
pub fn local_shas(project_dir: &Path) -> io::Result<BTreeMap<String, String>> {
    scan_local(project_dir)?
        .into_iter()
        .map(|(rel, abs)| Ok((rel, git_blob_sha(&std::fs::read(abs)?))))
        .collect()
}

pub fn now_secs() -> u64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_secs())
        .unwrap_or(0)
}
