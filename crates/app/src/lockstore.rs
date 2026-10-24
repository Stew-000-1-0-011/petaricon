//! A store wrapper that refuses to change the project a GitHub transfer works on.
//!
//! While a pull, push, clone or conflict resolution runs in the background, the project it
//! belongs to is locked: the worker writes (or reads) its files, so nothing else may. The
//! glue refuses edits up front with a message; this wrapper is the backstop below it: every
//! write path of the store (documents, recovery copies, `.preview`, new/renamed files and
//! projects) fails for the locked project. A project being cloned is also hidden: it is not
//! in the project list and cannot be read until the clone has finished.

use std::cell::RefCell;
use std::io;
use std::path::PathBuf;
use std::rc::Rc;

use storage::{ComponentPreview, PreviewConfig, ProjectStore};

/// The text shown when an edit of the locked project is refused.
pub const LOCKED_TEXT: &str = "GitHub と通信中のため、このプロジェクトは編集できません";

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Lock {
    pub project: String,
    /// A clone: the project does not exist for the user until the clone finishes.
    pub hidden: bool,
}

/// The lock, shared by the glue (which sets it) and the store wrapper.
pub type SharedLock = Rc<RefCell<Option<Lock>>>;

pub struct LockedStore {
    inner: Box<dyn ProjectStore>,
    lock: SharedLock,
}

impl LockedStore {
    pub fn new(inner: Box<dyn ProjectStore>, lock: SharedLock) -> Self {
        LockedStore { inner, lock }
    }

    fn locked(&self, project: &str) -> bool {
        self.lock.borrow().as_ref().is_some_and(|l| l.project == project)
    }

    fn hidden(&self, project: &str) -> bool {
        self.lock.borrow().as_ref().is_some_and(|l| l.hidden && l.project == project)
    }

    fn not_found<T>(&self) -> io::Result<T> {
        Err(io::Error::new(io::ErrorKind::NotFound, "取り込み中のプロジェクトです"))
    }
}

macro_rules! guard_write {
    ($self:ident, $project:expr) => {
        if $self.locked($project) {
            return Err(io::Error::new(io::ErrorKind::PermissionDenied, LOCKED_TEXT));
        }
    };
}

impl ProjectStore for LockedStore {
    fn preview_config(&self, project: &str) -> PreviewConfig {
        self.inner.preview_config(project)
    }
    fn set_preview_config(&self, project: &str, cfg: &PreviewConfig) -> io::Result<()> {
        guard_write!(self, project);
        self.inner.set_preview_config(project, cfg)
    }
    fn set_component_preview(
        &self,
        project: &str,
        file: &str,
        component: &str,
        p: &ComponentPreview,
    ) -> io::Result<()> {
        guard_write!(self, project);
        self.inner.set_component_preview(project, file, component, p)
    }
    fn projects(&self) -> io::Result<Vec<String>> {
        let mut all = self.inner.projects()?;
        all.retain(|p| !self.hidden(p));
        Ok(all)
    }
    fn create_project(&self, name: &str) -> io::Result<()> {
        if self.locked(name) {
            return Err(io::Error::from(io::ErrorKind::AlreadyExists));
        }
        self.inner.create_project(name)
    }
    fn files(&self, project: &str) -> io::Result<Vec<String>> {
        if self.hidden(project) {
            return self.not_found();
        }
        self.inner.files(project)
    }
    fn create_file(&self, project: &str, name: &str) -> io::Result<String> {
        guard_write!(self, project);
        self.inner.create_file(project, name)
    }
    fn create_file_with(&self, project: &str, name: &str, text: &str) -> io::Result<String> {
        guard_write!(self, project);
        self.inner.create_file_with(project, name, text)
    }
    fn rename_file(&self, project: &str, old: &str, new: &str) -> io::Result<String> {
        guard_write!(self, project);
        self.inner.rename_file(project, old, new)
    }
    fn rename_project(&self, old: &str, new: &str) -> io::Result<()> {
        guard_write!(self, old);
        if self.locked(new) {
            return Err(io::Error::from(io::ErrorKind::AlreadyExists));
        }
        self.inner.rename_project(old, new)
    }
    fn delete_project(&self, name: &str) -> io::Result<()> {
        guard_write!(self, name);
        self.inner.delete_project(name)
    }
    fn location(&self, project: &str, file: &str) -> Option<PathBuf> {
        self.inner.location(project, file)
    }
    fn read(&self, project: &str, file: &str) -> io::Result<String> {
        if self.hidden(project) {
            return self.not_found();
        }
        self.inner.read(project, file)
    }
    fn write(&self, project: &str, file: &str, text: &str) -> io::Result<()> {
        guard_write!(self, project);
        self.inner.write(project, file, text)
    }
    fn write_recovery(&self, project: &str, file: &str, text: &str) -> io::Result<()> {
        guard_write!(self, project);
        self.inner.write_recovery(project, file, text)
    }
    fn take_recovery(&self, project: &str, file: &str) -> io::Result<Option<String>> {
        self.inner.take_recovery(project, file)
    }
    fn clear_recovery(&self, project: &str, file: &str) -> io::Result<()> {
        guard_write!(self, project);
        self.inner.clear_recovery(project, file)
    }
    fn last_open(&self) -> io::Result<Option<(String, String)>> {
        self.inner.last_open()
    }
    fn set_last_open(&self, project: &str, file: &str) -> io::Result<()> {
        self.inner.set_last_open(project, file)
    }
    fn setting(&self, key: &str) -> Option<String> {
        self.inner.setting(key)
    }
    fn set_setting(&self, key: &str, value: &str) -> io::Result<()> {
        self.inner.set_setting(key, value)
    }
    fn project_path(&self, project: &str) -> Option<PathBuf> {
        self.inner.project_path(project)
    }
    fn github_token(&self) -> Option<String> {
        self.inner.github_token()
    }
    fn set_github_token(&self, token: &str) -> io::Result<()> {
        self.inner.set_github_token(token)
    }
    fn clear_github_token(&self) -> io::Result<()> {
        self.inner.clear_github_token()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use storage::LocalStore;

    fn setup(hidden: bool) -> (tempfile::TempDir, LockedStore, SharedLock) {
        let d = tempfile::tempdir().unwrap();
        let inner = LocalStore::new(d.path().join("p")).unwrap();
        inner.create_project("a").unwrap();
        inner.create_project("b").unwrap();
        let lock: SharedLock = Rc::new(RefCell::new(Some(Lock { project: "a".into(), hidden })));
        (d, LockedStore::new(Box::new(inner), lock.clone()), lock)
    }

    #[test]
    fn every_write_to_the_locked_project_fails_and_other_projects_are_free() {
        let (_d, s, lock) = setup(false);
        let denied = |r: io::Result<()>| r.unwrap_err().kind() == io::ErrorKind::PermissionDenied;
        assert!(denied(s.write("a", "main.slint", "x")));
        assert!(denied(s.write_recovery("a", "main.slint", "x")));
        assert!(denied(s.clear_recovery("a", "main.slint")));
        assert!(denied(s.create_file("a", "n").map(|_| ())));
        assert!(denied(s.create_file_with("a", "n", "x").map(|_| ())));
        assert!(denied(s.rename_file("a", "main.slint", "m").map(|_| ())));
        assert!(denied(s.rename_project("a", "c")));
        assert!(denied(s.delete_project("a")));
        assert!(s.projects().unwrap().contains(&"a".to_string()));
        assert!(s.rename_project("b", "a").is_err());
        assert!(denied(s.set_component_preview("a", "main.slint", "Main", &Default::default())));
        // Reading and the other project are fine.
        assert!(s.read("a", "main.slint").is_ok());
        assert!(s.write("b", "main.slint", "x").is_ok());
        assert_eq!(s.projects().unwrap(), vec!["a", "b"]);
        assert!(s.delete_project("b").is_ok(), "other projects can go");
        assert_eq!(s.projects().unwrap(), vec!["a"]);
        *lock.borrow_mut() = None;
        assert!(s.write("a", "main.slint", "x").is_ok());
    }

    #[test]
    fn a_hidden_project_is_not_listed_or_readable() {
        let (_d, s, _lock) = setup(true);
        assert_eq!(s.projects().unwrap(), vec!["b"]);
        assert!(s.files("a").is_err());
        assert!(s.read("a", "main.slint").is_err());
        assert!(s.create_project("a").is_err());
    }
}
