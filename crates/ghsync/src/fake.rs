//! An in-memory GitHub for tests (feature `test-support`): commits, trees and blobs by hash.

use std::collections::{BTreeMap, HashMap};
use std::sync::Mutex;

use crate::{
    ApiError, GitHubApi, NewEntry, TreeEntry, UpdateError, git_blob_sha,
};

// ---- fake GitHub -------------------------------------------------------------------

#[derive(Default)]
pub struct Inner {
    pub blobs: HashMap<String, Vec<u8>>,
    /// tree sha -> flat recursive listing (path -> (sha, mode))
    pub trees: HashMap<String, BTreeMap<String, (String, String)>>,
    /// commit sha -> (tree, parent)
    pub commits: HashMap<String, (String, Option<String>)>,
    pub refs: HashMap<String, String>,
    pub counter: u32,
    pub fail_op: Option<&'static str>,
    pub race_on_update: bool,
    pub truncated: bool,
    pub calls: Vec<&'static str>,
}

pub struct Fake(pub Mutex<Inner>);

pub const REPO: &str = "o/r";

impl Default for Fake {
    fn default() -> Self {
        Self::new()
    }
}

impl Fake {
    pub fn new() -> Fake {
        Fake(Mutex::new(Inner::default()))
    }
    pub fn fake_sha(i: &mut Inner, what: &str) -> String {
        i.counter += 1;
        let mut s = format!("{what}{:08}", i.counter);
        while s.len() < 40 {
            s.push('0');
        }
        s
    }
    /// Makes a commit on `branch` (child of its head if any) with the given changes.
    pub fn commit(&self, branch: &str, changes: &[(&str, Option<&[u8]>)]) -> String {
        let with_mode: Vec<_> = changes.iter().map(|(p, c)| (*p, *c, "100644")).collect();
        self.commit_modes(branch, &with_mode)
    }
    pub fn commit_modes(&self, branch: &str, changes: &[(&str, Option<&[u8]>, &str)]) -> String {
        let mut i = self.0.lock().unwrap();
        let parent = i.refs.get(branch).cloned();
        let mut listing = parent
            .as_ref()
            .map(|p| i.trees[&i.commits[p].0].clone())
            .unwrap_or_default();
        for (path, content, mode) in changes {
            match content {
                Some(c) => {
                    let sha = git_blob_sha(c);
                    i.blobs.insert(sha.clone(), c.to_vec());
                    listing.insert(path.to_string(), (sha, mode.to_string()));
                }
                None => {
                    listing.remove(*path);
                }
            }
        }
        let tree = Fake::fake_sha(&mut i, "tree");
        i.trees.insert(tree.clone(), listing);
        let c = Fake::fake_sha(&mut i, "commit");
        i.commits.insert(c.clone(), (tree, parent));
        i.refs.insert(branch.to_string(), c.clone());
        c
    }
    pub fn head(&self, branch: &str) -> String {
        self.0.lock().unwrap().refs[branch].clone()
    }
    pub fn listing(&self, branch: &str) -> BTreeMap<String, Vec<u8>> {
        let i = self.0.lock().unwrap();
        let c = &i.refs[branch];
        i.trees[&i.commits[c].0]
            .iter()
            .map(|(p, (s, _))| (p.clone(), i.blobs[s].clone()))
            .collect()
    }
    pub fn commit_count(&self) -> usize {
        self.0.lock().unwrap().commits.len()
    }
    pub fn parent(&self, commit: &str) -> Option<String> {
        self.0.lock().unwrap().commits[commit].1.clone()
    }
    pub fn fail_on(&self, op: Option<&'static str>) {
        self.0.lock().unwrap().fail_op = op;
    }
    pub fn step(&self, op: &'static str) -> Result<(), ApiError> {
        let mut i = self.0.lock().unwrap();
        i.calls.push(op);
        if i.fail_op == Some(op) {
            return Err(ApiError::Network(format!("fake failure at {op}")));
        }
        Ok(())
    }
}

impl GitHubApi for Fake {
    fn branch_head(&self, _: &str, branch: &str) -> Result<String, ApiError> {
        self.step("branch_head")?;
        self.0
            .lock()
            .unwrap()
            .refs
            .get(branch)
            .cloned()
            .ok_or(ApiError::NotFound)
    }
    fn commit_tree(&self, _: &str, commit: &str) -> Result<String, ApiError> {
        self.step("commit_tree")?;
        self.0
            .lock()
            .unwrap()
            .commits
            .get(commit)
            .map(|c| c.0.clone())
            .ok_or(ApiError::NotFound)
    }
    fn tree_shallow(&self, _: &str, tree: &str) -> Result<Vec<TreeEntry>, ApiError> {
        self.step("tree_shallow")?;
        let i = self.0.lock().unwrap();
        let (base, prefix) = tree.split_once(':').unwrap_or((tree, ""));
        let mut out: BTreeMap<String, TreeEntry> = BTreeMap::new();
        for (p, (s, m)) in &i.trees[base] {
            let Some(rest) = p.strip_prefix(prefix) else {
                continue;
            };
            match rest.split_once('/') {
                None => {
                    out.insert(
                        rest.into(),
                        TreeEntry {
                            path: rest.into(),
                            sha: s.clone(),
                            mode: m.clone(),
                            kind: kind_of(m),
                        },
                    );
                }
                Some((dir, _)) => {
                    out.insert(
                        dir.into(),
                        TreeEntry {
                            path: dir.into(),
                            sha: format!("{base}:{prefix}{dir}/"),
                            mode: "040000".into(),
                            kind: "tree".into(),
                        },
                    );
                }
            }
        }
        Ok(out.into_values().collect())
    }
    fn tree_recursive(&self, _: &str, tree: &str) -> Result<Vec<TreeEntry>, ApiError> {
        self.step("tree_recursive")?;
        let i = self.0.lock().unwrap();
        // Only a listing of a whole commit tree can be truncated.
        if i.truncated && !tree.contains(':') {
            return Err(ApiError::Truncated);
        }
        let (base, prefix) = tree.split_once(':').unwrap_or((tree, ""));
        Ok(i.trees[base]
            .iter()
            .filter_map(|(p, (s, m))| {
                let rel = p.strip_prefix(prefix)?;
                Some(TreeEntry {
                    path: rel.into(),
                    sha: s.clone(),
                    mode: m.clone(),
                    kind: kind_of(m),
                })
            })
            .collect())
    }
    fn blob(&self, _: &str, sha: &str) -> Result<Vec<u8>, ApiError> {
        self.step("blob")?;
        self.0
            .lock()
            .unwrap()
            .blobs
            .get(sha)
            .cloned()
            .ok_or(ApiError::NotFound)
    }
    fn create_blob(&self, _: &str, bytes: &[u8]) -> Result<String, ApiError> {
        self.step("create_blob")?;
        let sha = git_blob_sha(bytes);
        self.0
            .lock()
            .unwrap()
            .blobs
            .insert(sha.clone(), bytes.to_vec());
        Ok(sha)
    }
    fn create_tree(&self, _: &str, base: &str, entries: Vec<NewEntry>) -> Result<String, ApiError> {
        self.step("create_tree")?;
        let mut i = self.0.lock().unwrap();
        let mut listing = i.trees[base].clone();
        for e in entries {
            match e.sha {
                Some(s) => {
                    listing.insert(e.path, (s, e.mode));
                }
                None => {
                    listing.remove(&e.path);
                }
            }
        }
        let t = Fake::fake_sha(&mut i, "tree");
        i.trees.insert(t.clone(), listing);
        Ok(t)
    }
    fn create_commit(
        &self,
        _: &str,
        _msg: &str,
        tree: &str,
        parent: &str,
    ) -> Result<String, ApiError> {
        self.step("create_commit")?;
        let mut i = self.0.lock().unwrap();
        let c = Fake::fake_sha(&mut i, "commit");
        i.commits
            .insert(c.clone(), (tree.to_string(), Some(parent.to_string())));
        Ok(c)
    }
    fn update_ref(&self, _: &str, branch: &str, commit: &str) -> Result<(), UpdateError> {
        self.step("update_ref").map_err(UpdateError::Other)?;
        let race = std::mem::take(&mut self.0.lock().unwrap().race_on_update);
        if race {
            self.commit(branch, &[("elsewhere.txt", Some(b"x"))]);
        }
        let mut i = self.0.lock().unwrap();
        if i.commits[commit].1.as_ref() != i.refs.get(branch) {
            return Err(UpdateError::NotFastForward);
        }
        i.refs.insert(branch.to_string(), commit.to_string());
        Ok(())
    }
}

fn kind_of(mode: &str) -> String {
    if mode == "160000" {
        "commit".into()
    } else {
        "blob".into()
    }
}
