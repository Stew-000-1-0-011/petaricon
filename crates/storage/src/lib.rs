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

/// Small `key=value` settings of the editor; lives in the store root.
const SETTINGS: &str = ".settings";

/// The only keys `setting` / `set_setting` accept.
pub const SETTING_KEYS: [&str; 1] = ["tidy_tolerance"];

fn check_setting(key: &str, value: Option<&str>) -> io::Result<()> {
    let bad = |what: String| io::Error::new(io::ErrorKind::InvalidInput, what);
    if !SETTING_KEYS.contains(&key) {
        return Err(bad(format!("unknown setting: {key:?}")));
    }
    if let Some(v) = value
        && (v.contains(['\n', '\r', '\0']) || v.trim() != v)
    {
        return Err(bad(format!("invalid value for {key}: {v:?}")));
    }
    Ok(())
}

pub trait ProjectStore {
    fn projects(&self) -> io::Result<Vec<String>>;
    fn create_project(&self, name: &str) -> io::Result<()>;
    fn files(&self, project: &str) -> io::Result<Vec<String>>;
    /// Creates `{name}.slint` (the extension is appended when missing) from the project
    /// template and returns the final file name. `AlreadyExists` when it is there.
    fn create_file(&self, project: &str, name: &str) -> io::Result<String> {
        self.create_file_with(project, name, MAIN_TEMPLATE)
    }
    /// Like `create_file`, but the new file starts with `text`: no `.bak` is produced and
    /// nothing is left behind when it fails.
    fn create_file_with(&self, project: &str, name: &str, text: &str) -> io::Result<String>;
    /// Renames a file together with its `.bak` and recovery copy; never overwrites. Returns
    /// the final name (`.slint` appended when missing).
    fn rename_file(&self, project: &str, old: &str, new: &str) -> io::Result<String>;
    /// Renames a project directory; never overwrites.
    fn rename_project(&self, old: &str, new: &str) -> io::Result<()>;
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
    /// The stored value of a setting (`None` when never set, unknown or unreadable).
    fn setting(&self, _key: &str) -> Option<String> {
        None
    }
    /// Stores a setting; other settings are kept. Only keys in `SETTING_KEYS` are accepted.
    fn set_setting(&self, key: &str, value: &str) -> io::Result<()> {
        check_setting(key, Some(value))
    }
}

pub struct LocalStore {
    root: PathBuf,
}

/// The rule every project and file name must pass.
pub fn check_name(name: &str) -> io::Result<()> {
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

fn with_extension_slint(name: &str) -> String {
    if name.ends_with(".slint") { name.to_string() } else { format!("{name}.slint") }
}

fn rename_if_exists(from: &Path, to: &Path) -> io::Result<()> {
    match fs::rename(from, to) {
        Err(e) if e.kind() == io::ErrorKind::NotFound => Ok(()),
        r => r,
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

    fn create_file_with(&self, project: &str, name: &str, text: &str) -> io::Result<String> {
        check_name(name)?;
        let name = with_extension_slint(name);
        let path = self.file_path(project, &name)?;
        let mut f = fs::OpenOptions::new().write(true).create_new(true).open(&path)?;
        if let Err(e) = io::Write::write_all(&mut f, text.as_bytes()).and_then(|()| f.sync_all()) {
            drop(f);
            let _ = fs::remove_file(&path);
            return Err(e);
        }
        Ok(name)
    }

    fn rename_file(&self, project: &str, old: &str, new: &str) -> io::Result<String> {
        check_name(old)?;
        check_name(new)?;
        let new = with_extension_slint(new);
        let from = self.file_path(project, old)?;
        let to = self.file_path(project, &new)?;
        if old == new {
            return if from.exists() {
                Ok(new)
            } else {
                Err(io::ErrorKind::NotFound.into())
            };
        }
        if !from.exists() {
            return Err(io::ErrorKind::NotFound.into());
        }
        if to.exists() {
            return Err(io::Error::new(
                io::ErrorKind::AlreadyExists,
                format!("{new} already exists"),
            ));
        }
        let from_rec = self.recovery_path(project, old)?;
        let to_rec = self.recovery_path(project, &new)?;
        let to_bak = to.with_file_name(format!("{new}.bak"));
        if to_bak.exists() || to_rec.exists() {
            return Err(io::Error::new(
                io::ErrorKind::AlreadyExists,
                format!("{new} has a backup or recovery copy"),
            ));
        }
        fs::rename(&from, &to)?;
        rename_if_exists(
            &from.with_file_name(format!("{old}.bak")),
            &to_bak,
        )?;
        rename_if_exists(&from_rec, &to_rec)?;
        if self.last_open()? == Some((project.to_string(), old.to_string())) {
            self.set_last_open(project, &new)?;
        }
        Ok(new)
    }

    fn rename_project(&self, old: &str, new: &str) -> io::Result<()> {
        let from = self.project_dir(old)?;
        let to = self.project_dir(new)?;
        if !from.is_dir() {
            return Err(io::ErrorKind::NotFound.into());
        }
        if to.exists() {
            return Err(io::Error::new(
                io::ErrorKind::AlreadyExists,
                format!("{new} already exists"),
            ));
        }
        fs::rename(&from, &to)?;
        if let Some((p, f)) = self.last_open()?
            && p == old
        {
            self.set_last_open(new, &f)?;
        }
        Ok(())
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

    fn setting(&self, key: &str) -> Option<String> {
        check_setting(key, None).ok()?;
        let text = fs::read_to_string(self.root.join(SETTINGS)).ok()?;
        text.lines().find_map(|l| {
            let (k, v) = l.split_once('=')?;
            (k == key).then(|| v.to_string())
        })
    }

    fn set_setting(&self, key: &str, value: &str) -> io::Result<()> {
        check_setting(key, Some(value))?;
        let path = self.root.join(SETTINGS);
        let old = match fs::read_to_string(&path) {
            Ok(t) => t,
            Err(e) if e.kind() == io::ErrorKind::NotFound => String::new(),
            Err(e) => return Err(e),
        };
        let mut out: Vec<String> = old
            .lines()
            .filter(|l| l.split_once('=').is_some_and(|(k, _)| k != key && SETTING_KEYS.contains(&k)))
            .map(str::to_string)
            .collect();
        out.push(format!("{key}={value}"));
        let tmp = self.root.join(".settings.tmp");
        fs::write(&tmp, out.join("\n") + "\n")?;
        fs::rename(&tmp, &path)
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
    fn settings_roundtrip_and_validation() {
        let (d, s) = store();
        assert_eq!(s.setting("tidy_tolerance"), None);
        s.set_setting("tidy_tolerance", "20").unwrap();
        assert_eq!(s.setting("tidy_tolerance").as_deref(), Some("20"));
        s.set_setting("tidy_tolerance", "8").unwrap();
        let text = std::fs::read_to_string(d.path().join("projects/.settings")).unwrap();
        assert_eq!(text, "tidy_tolerance=8\n");
        // survives reopening the store
        let again = LocalStore::new(d.path().join("projects")).unwrap();
        assert_eq!(again.setting("tidy_tolerance").as_deref(), Some("8"));
        for bad in ["../x", "a=b", "", "other", ".settings"] {
            assert_eq!(s.set_setting(bad, "1").unwrap_err().kind(), io::ErrorKind::InvalidInput, "{bad:?}");
            assert_eq!(s.setting(bad), None);
        }
        for v in ["1\n2", "x\ry", " 3"] {
            assert_eq!(s.set_setting("tidy_tolerance", v).unwrap_err().kind(), io::ErrorKind::InvalidInput);
        }
        assert_eq!(s.setting("tidy_tolerance").as_deref(), Some("8"));
        assert!(s.projects().unwrap().is_empty());
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
    fn create_file_uses_template_and_appends_extension() {
        let (_d, s) = store();
        s.create_project("p").unwrap();
        assert_eq!(s.create_file("p", "card").unwrap(), "card.slint");
        assert_eq!(s.create_file("p", "x.slint").unwrap(), "x.slint");
        assert_eq!(s.read("p", "card.slint").unwrap(), s.read("p", "main.slint").unwrap());
        assert_eq!(s.files("p").unwrap(), vec!["card.slint", "main.slint", "x.slint"]);
    }

    #[test]
    fn create_file_refuses_existing_and_invalid_names() {
        let (_d, s) = store();
        s.create_project("p").unwrap();
        s.write("p", "card.slint", "mine").unwrap();
        assert_eq!(s.create_file("p", "card").unwrap_err().kind(), io::ErrorKind::AlreadyExists);
        assert_eq!(s.create_file("p", "main.slint").unwrap_err().kind(), io::ErrorKind::AlreadyExists);
        assert_eq!(s.read("p", "card.slint").unwrap(), "mine");
        for n in ["", "..", "a/b", "a\\b", ".hidden", "x..y", "a\0b"] {
            assert_eq!(s.create_file("p", n).unwrap_err().kind(), io::ErrorKind::InvalidInput, "{n:?}");
        }
        assert_eq!(s.files("p").unwrap(), vec!["card.slint", "main.slint"]);
    }

    #[test]
    fn rename_file_moves_file_bak_and_recovery() {
        let (d, s) = store();
        s.create_project("p").unwrap();
        s.write("p", "a.slint", "one").unwrap();
        s.write("p", "a.slint", "two").unwrap(); // a.slint.bak = one
        s.write_recovery("p", "a.slint", "draft").unwrap();
        assert_eq!(s.rename_file("p", "a.slint", "b").unwrap(), "b.slint");
        assert_eq!(s.read("p", "b.slint").unwrap(), "two");
        let dir = d.path().join("projects/p");
        assert_eq!(std::fs::read_to_string(dir.join("b.slint.bak")).unwrap(), "one");
        assert_eq!(s.take_recovery("p", "b.slint").unwrap().as_deref(), Some("draft"));
        assert!(!dir.join("a.slint").exists());
        assert!(!dir.join("a.slint.bak").exists());
        assert_eq!(s.take_recovery("p", "a.slint").unwrap(), None);
    }

    #[test]
    fn rename_file_without_bak_or_recovery() {
        let (_d, s) = store();
        s.create_project("p").unwrap();
        assert_eq!(s.rename_file("p", "main.slint", "m2.slint").unwrap(), "m2.slint");
        assert_eq!(s.files("p").unwrap(), vec!["m2.slint"]);
    }

    #[test]
    fn rename_file_refuses_existing_target_without_changes() {
        let (d, s) = store();
        s.create_project("p").unwrap();
        s.write("p", "a.slint", "A").unwrap();
        s.write("p", "a.slint", "A2").unwrap();
        s.write("p", "b.slint", "B").unwrap();
        s.write_recovery("p", "a.slint", "draft").unwrap();
        assert_eq!(s.rename_file("p", "a.slint", "b").unwrap_err().kind(), io::ErrorKind::AlreadyExists);
        assert_eq!(s.read("p", "a.slint").unwrap(), "A2");
        assert_eq!(s.read("p", "b.slint").unwrap(), "B");
        assert_eq!(s.take_recovery("p", "a.slint").unwrap().as_deref(), Some("draft"));
        assert_eq!(std::fs::read_to_string(d.path().join("projects/p/a.slint.bak")).unwrap(), "A");
    }

    #[test]
    fn rename_file_refuses_invalid_names_and_missing_source() {
        let (_d, s) = store();
        s.create_project("p").unwrap();
        for n in ["", "..", "a/b", ".hidden", "x..y"] {
            assert_eq!(s.rename_file("p", "main.slint", n).unwrap_err().kind(), io::ErrorKind::InvalidInput, "{n:?}");
            assert_eq!(s.rename_file("p", n, "z").unwrap_err().kind(), io::ErrorKind::InvalidInput, "{n:?}");
        }
        assert_eq!(s.rename_file("p", "nope.slint", "z").unwrap_err().kind(), io::ErrorKind::NotFound);
        assert_eq!(s.files("p").unwrap(), vec!["main.slint"]);
    }

    #[test]
    fn rename_file_follows_last_open_only_for_that_file() {
        let (_d, s) = store();
        s.create_project("p").unwrap();
        s.write("p", "a.slint", "").unwrap();
        s.set_last_open("p", "a.slint").unwrap();
        s.rename_file("p", "a.slint", "b").unwrap();
        assert_eq!(s.last_open().unwrap(), Some(("p".into(), "b.slint".into())));
        s.set_last_open("p", "main.slint").unwrap();
        s.rename_file("p", "b.slint", "c").unwrap();
        assert_eq!(s.last_open().unwrap(), Some(("p".into(), "main.slint".into())));
    }

    #[test]
    fn rename_project_moves_everything_and_follows_last_open() {
        let (_d, s) = store();
        s.create_project("p").unwrap();
        s.write_recovery("p", "main.slint", "draft").unwrap();
        s.set_last_open("p", "main.slint").unwrap();
        s.rename_project("p", "q").unwrap();
        assert_eq!(s.projects().unwrap(), vec!["q"]);
        assert_eq!(s.take_recovery("q", "main.slint").unwrap().as_deref(), Some("draft"));
        assert_eq!(s.last_open().unwrap(), Some(("q".into(), "main.slint".into())));
    }

    #[test]
    fn rename_project_leaves_other_last_open_alone() {
        let (_d, s) = store();
        s.create_project("p").unwrap();
        s.create_project("o").unwrap();
        s.set_last_open("o", "main.slint").unwrap();
        s.rename_project("p", "q").unwrap();
        assert_eq!(s.last_open().unwrap(), Some(("o".into(), "main.slint".into())));
    }

    #[test]
    fn rename_project_refuses_existing_target_and_invalid_names() {
        let (_d, s) = store();
        s.create_project("p").unwrap();
        s.create_project("q").unwrap();
        s.write("q", "main.slint", "mine").unwrap();
        assert_eq!(s.rename_project("p", "q").unwrap_err().kind(), io::ErrorKind::AlreadyExists);
        assert_eq!(s.read("q", "main.slint").unwrap(), "mine");
        assert_eq!(s.projects().unwrap(), vec!["p", "q"]);
        for n in ["", "..", "a/b", ".hidden", "x..y"] {
            assert_eq!(s.rename_project("p", n).unwrap_err().kind(), io::ErrorKind::InvalidInput, "{n:?}");
            assert_eq!(s.rename_project(n, "z").unwrap_err().kind(), io::ErrorKind::InvalidInput, "{n:?}");
        }
        assert_eq!(s.projects().unwrap(), vec!["p", "q"]);
    }

    #[test]
    fn create_file_with_writes_text_directly_without_bak() {
        let (d, s) = store();
        s.create_project("p").unwrap();
        assert_eq!(s.create_file_with("p", "c", "hello").unwrap(), "c.slint");
        assert_eq!(s.read("p", "c.slint").unwrap(), "hello");
        assert!(!d.path().join("projects/p/c.slint.bak").exists());
        assert_eq!(
            s.create_file_with("p", "c", "x").unwrap_err().kind(),
            io::ErrorKind::AlreadyExists
        );
        assert_eq!(s.read("p", "c.slint").unwrap(), "hello");
        assert_eq!(
            s.create_file_with("p", "../x", "x").unwrap_err().kind(),
            io::ErrorKind::InvalidInput
        );
    }

    #[test]
    fn rename_file_refuses_when_target_bak_or_recovery_exist() {
        let (d, s) = store();
        s.create_project("p").unwrap();
        s.write("p", "a.slint", "A").unwrap();
        let dir = d.path().join("projects/p");
        std::fs::write(dir.join("b.slint.bak"), "old bak").unwrap();
        assert_eq!(s.rename_file("p", "a.slint", "b").unwrap_err().kind(), io::ErrorKind::AlreadyExists);
        assert_eq!(std::fs::read_to_string(dir.join("b.slint.bak")).unwrap(), "old bak");
        assert!(dir.join("a.slint").exists() && !dir.join("b.slint").exists());
        std::fs::remove_file(dir.join("b.slint.bak")).unwrap();
        s.write_recovery("p", "b.slint", "old draft").unwrap();
        assert_eq!(s.rename_file("p", "a.slint", "b").unwrap_err().kind(), io::ErrorKind::AlreadyExists);
        assert_eq!(s.take_recovery("p", "b.slint").unwrap().as_deref(), Some("old draft"));
        assert!(dir.join("a.slint").exists() && !dir.join("b.slint").exists());
    }

    #[test]
    fn trait_is_object_safe() {
        let (_d, s) = store();
        let b: Box<dyn ProjectStore> = Box::new(s);
        assert!(b.projects().unwrap().is_empty());
    }
}
