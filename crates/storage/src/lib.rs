//! Project storage: a trait and a local-filesystem implementation.
//!
//! Layout: `{root}/{project}/{file}`, with recovery copies in
//! `{root}/{project}/.recovery/{file}`.

use std::fs;
use std::io;
use std::path::{Path, PathBuf};

const MAIN_TEMPLATE: &str = "export component Main inherits Window {\n    preferred-width: 360px;\n    preferred-height: 640px;\n}\n";

/// Remembers the last open project and file; lives in the store root (dot names are never
/// listed as projects).
const LAST_OPEN: &str = ".last-open";

pub trait ProjectStore {
    fn projects(&self) -> io::Result<Vec<String>>;
    fn create_project(&self, name: &str) -> io::Result<()>;
    fn files(&self, project: &str) -> io::Result<Vec<String>>;
    fn read(&self, project: &str, file: &str) -> io::Result<String>;
    fn write(&self, project: &str, file: &str, text: &str) -> io::Result<()>;
    fn write_recovery(&self, project: &str, file: &str, text: &str) -> io::Result<()>;
    /// Returns the recovery copy, if any. Does not delete it.
    fn take_recovery(&self, project: &str, file: &str) -> io::Result<Option<String>>;
    fn clear_recovery(&self, project: &str, file: &str) -> io::Result<()>;
    /// The project and file that were open last, if remembered.
    fn last_open(&self) -> io::Result<Option<(String, String)>> {
        Ok(None)
    }
    fn set_last_open(&self, _project: &str, _file: &str) -> io::Result<()> {
        Ok(())
    }
}

pub struct LocalStore {
    root: PathBuf,
}

fn check_name(name: &str) -> io::Result<()> {
    if name.is_empty()
        || name.contains(['/', '\\'])
        || name.contains("..")
        || name.starts_with('.')
        || name.contains('\0')
    {
        return Err(io::Error::new(
            io::ErrorKind::InvalidInput,
            format!("invalid name: {name:?}"),
        ));
    }
    Ok(())
}

impl LocalStore {
    pub fn new(root: PathBuf) -> io::Result<Self> {
        fs::create_dir_all(&root)?;
        Ok(Self { root })
    }

    fn project_dir(&self, project: &str) -> io::Result<PathBuf> {
        check_name(project)?;
        Ok(self.root.join(project))
    }

    fn file_path(&self, project: &str, file: &str) -> io::Result<PathBuf> {
        check_name(file)?;
        Ok(self.project_dir(project)?.join(file))
    }

    fn recovery_path(&self, project: &str, file: &str) -> io::Result<PathBuf> {
        check_name(file)?;
        Ok(self.project_dir(project)?.join(".recovery").join(file))
    }
}

fn remove_if_exists(path: &Path) -> io::Result<()> {
    match fs::remove_file(path) {
        Err(e) if e.kind() == io::ErrorKind::NotFound => Ok(()),
        r => r,
    }
}

impl ProjectStore for LocalStore {
    fn projects(&self) -> io::Result<Vec<String>> {
        let mut names = Vec::new();
        for entry in fs::read_dir(&self.root)? {
            let entry = entry?;
            if !entry.file_type()?.is_dir() {
                continue;
            }
            if let Ok(name) = entry.file_name().into_string()
                && !name.starts_with('.')
            {
                names.push(name);
            }
        }
        names.sort();
        Ok(names)
    }

    fn create_project(&self, name: &str) -> io::Result<()> {
        let dir = self.project_dir(name)?;
        fs::create_dir(&dir)?; // AlreadyExists if present
        fs::write(dir.join("main.slint"), MAIN_TEMPLATE)
    }

    fn files(&self, project: &str) -> io::Result<Vec<String>> {
        let mut names = Vec::new();
        for entry in fs::read_dir(self.project_dir(project)?)? {
            let entry = entry?;
            if !entry.file_type()?.is_file() {
                continue;
            }
            if let Ok(name) = entry.file_name().into_string()
                && name.ends_with(".slint")
                && !name.starts_with('.')
            {
                names.push(name);
            }
        }
        names.sort();
        Ok(names)
    }

    fn read(&self, project: &str, file: &str) -> io::Result<String> {
        fs::read_to_string(self.file_path(project, file)?)
    }

    fn write(&self, project: &str, file: &str, text: &str) -> io::Result<()> {
        let path = self.file_path(project, file)?;
        let recovery = self.recovery_path(project, file)?;
        let tmp = path.with_file_name(format!(".{file}.tmp"));
        fs::write(&tmp, text)?;
        if path.exists() {
            fs::rename(&path, path.with_file_name(format!("{file}.bak")))?;
        }
        fs::rename(&tmp, &path)?;
        // The save succeeded; a stale recovery copy is only an inconvenience.
        let _ = remove_if_exists(&recovery);
        Ok(())
    }

    fn write_recovery(&self, project: &str, file: &str, text: &str) -> io::Result<()> {
        let path = self.recovery_path(project, file)?;
        if let Some(dir) = path.parent() {
            fs::create_dir_all(dir)?;
        }
        fs::write(path, text)
    }

    fn take_recovery(&self, project: &str, file: &str) -> io::Result<Option<String>> {
        match fs::read_to_string(self.recovery_path(project, file)?) {
            Ok(s) => Ok(Some(s)),
            Err(e) if e.kind() == io::ErrorKind::NotFound => Ok(None),
            Err(e) => Err(e),
        }
    }

    fn clear_recovery(&self, project: &str, file: &str) -> io::Result<()> {
        remove_if_exists(&self.recovery_path(project, file)?)
    }

    fn last_open(&self) -> io::Result<Option<(String, String)>> {
        let text = match fs::read_to_string(self.root.join(LAST_OPEN)) {
            Ok(t) => t,
            Err(e) if e.kind() == io::ErrorKind::NotFound => return Ok(None),
            Err(e) => return Err(e),
        };
        let mut lines = text.lines();
        let (Some(p), Some(f)) = (lines.next(), lines.next()) else {
            return Ok(None);
        };
        if check_name(p).is_err() || check_name(f).is_err() {
            return Ok(None);
        }
        Ok(Some((p.to_string(), f.to_string())))
    }

    fn set_last_open(&self, project: &str, file: &str) -> io::Result<()> {
        check_name(project)?;
        check_name(file)?;
        fs::write(self.root.join(LAST_OPEN), format!("{project}\n{file}\n"))
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::io;

    fn store() -> (tempfile::TempDir, LocalStore) {
        let dir = tempfile::tempdir().unwrap();
        let s = LocalStore::new(dir.path().join("projects")).unwrap();
        (dir, s)
    }

    #[test]
    fn create_lists_and_reads_main() {
        let (_d, s) = store();
        s.create_project("b").unwrap();
        s.create_project("a").unwrap();
        assert_eq!(s.projects().unwrap(), vec!["a", "b"]);
        assert_eq!(s.files("a").unwrap(), vec!["main.slint"]);
        assert_eq!(
            s.read("a", "main.slint").unwrap(),
            "export component Main inherits Window {\n    preferred-width: 360px;\n    preferred-height: 640px;\n}\n"
        );
        assert_eq!(
            s.create_project("a").unwrap_err().kind(),
            io::ErrorKind::AlreadyExists
        );
    }

    #[test]
    fn write_keeps_one_bak() {
        let (d, s) = store();
        s.create_project("p").unwrap();
        s.write("p", "main.slint", "one").unwrap();
        s.write("p", "main.slint", "two").unwrap();
        assert_eq!(s.read("p", "main.slint").unwrap(), "two");
        let bak = d.path().join("projects/p/main.slint.bak");
        assert_eq!(std::fs::read_to_string(bak).unwrap(), "one");
    }

    #[test]
    fn recovery_roundtrip() {
        let (_d, s) = store();
        s.create_project("p").unwrap();
        assert_eq!(s.take_recovery("p", "main.slint").unwrap(), None);
        s.write_recovery("p", "main.slint", "draft").unwrap();
        assert_eq!(s.take_recovery("p", "main.slint").unwrap().as_deref(), Some("draft"));
        // reading does not delete
        assert_eq!(s.take_recovery("p", "main.slint").unwrap().as_deref(), Some("draft"));
        s.write("p", "main.slint", "saved").unwrap();
        assert_eq!(s.take_recovery("p", "main.slint").unwrap(), None);
        s.write_recovery("p", "main.slint", "again").unwrap();
        s.clear_recovery("p", "main.slint").unwrap();
        assert_eq!(s.take_recovery("p", "main.slint").unwrap(), None);
        s.clear_recovery("p", "main.slint").unwrap();
    }

    #[test]
    fn write_succeeds_even_if_recovery_cleanup_fails() {
        let (d, s) = store();
        s.create_project("p").unwrap();
        // A directory where the recovery file should be makes remove_file fail.
        std::fs::create_dir_all(d.path().join("projects/p/.recovery/main.slint")).unwrap();
        s.write("p", "main.slint", "saved").unwrap();
        assert_eq!(s.read("p", "main.slint").unwrap(), "saved");
    }

    #[test]
    fn last_open_roundtrip() {
        let (d, s) = store();
        assert_eq!(s.last_open().unwrap(), None);
        s.create_project("p").unwrap();
        s.set_last_open("p", "main.slint").unwrap();
        assert_eq!(s.last_open().unwrap(), Some(("p".into(), "main.slint".into())));
        assert_eq!(s.projects().unwrap(), vec!["p"]);
        assert!(s.set_last_open("../x", "main.slint").is_err());
        std::fs::write(d.path().join("projects/.last-open"), "../evil\nx\n").unwrap();
        assert_eq!(s.last_open().unwrap(), None);
    }

    #[test]
    fn files_ignores_non_slint() {
        let (d, s) = store();
        s.create_project("p").unwrap();
        s.write("p", "b.slint", "").unwrap();
        s.write("p", "a.slint", "").unwrap();
        s.write("p", "notes.txt", "").unwrap();
        s.write("p", "a.slint", "x").unwrap(); // creates a.slint.bak
        std::fs::create_dir(d.path().join("projects/p/sub.slint")).unwrap();
        assert_eq!(s.files("p").unwrap(), vec!["a.slint", "b.slint", "main.slint"]);
    }

    #[test]
    fn projects_excludes_dot_dirs_and_files() {
        let (d, s) = store();
        s.create_project("p").unwrap();
        std::fs::create_dir(d.path().join("projects/.hidden")).unwrap();
        std::fs::write(d.path().join("projects/file.txt"), "").unwrap();
        assert_eq!(s.projects().unwrap(), vec!["p"]);
    }

    #[test]
    fn rejects_unsafe_names() {
        let (d, s) = store();
        s.create_project("p").unwrap();
        let bad = ["", "..", "a/b", "a\\b", "../x", ".hidden", "x..y"];
        for n in bad {
            let k = |r: io::Result<()>| r.unwrap_err().kind();
            assert_eq!(k(s.create_project(n)), io::ErrorKind::InvalidInput, "{n:?}");
            assert_eq!(s.files(n).unwrap_err().kind(), io::ErrorKind::InvalidInput, "{n:?}");
            assert_eq!(s.read(n, "main.slint").unwrap_err().kind(), io::ErrorKind::InvalidInput);
            assert_eq!(s.read("p", n).unwrap_err().kind(), io::ErrorKind::InvalidInput);
            assert_eq!(k(s.write("p", n, "x")), io::ErrorKind::InvalidInput);
            assert_eq!(k(s.write(n, "main.slint", "x")), io::ErrorKind::InvalidInput);
            assert_eq!(k(s.write_recovery("p", n, "x")), io::ErrorKind::InvalidInput);
            assert_eq!(s.take_recovery("p", n).unwrap_err().kind(), io::ErrorKind::InvalidInput);
            assert_eq!(k(s.clear_recovery("p", n)), io::ErrorKind::InvalidInput);
        }
        assert!(!d.path().join("x").exists());
    }

    #[test]
    fn trait_is_object_safe() {
        let (_d, s) = store();
        let b: Box<dyn ProjectStore> = Box::new(s);
        assert!(b.projects().unwrap().is_empty());
    }
}
