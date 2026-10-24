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

/// The GitHub token; its own file in the store root (not a setting, so it is never listed or
/// echoed with them). Readable by the app only where the platform allows it.
const GITHUB_TOKEN: &str = ".github-token";

/// A project being deleted is first renamed to `{DELETING}{name}` (dot names are never
/// listed); whatever is left of such a folder is removed when the store is opened.
const DELETING: &str = ".deleting-";

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

/// Preview values and screen size of the projects' components; `{project}/.preview`.
///
/// One entry per line, fields separated by a tab (lines that do not parse are ignored):
///
/// ```text
/// value<TAB>{file}<TAB>{component}<TAB>{property}<TAB>{Slint expression}
/// size<TAB>{file}<TAB>{component}<TAB>{width}<TAB>{height}
/// ```
const PREVIEW: &str = ".preview";

/// What the preview shows of one component: Slint expressions for its `in` / `in-out`
/// properties (in the order they were first set) and the screen size in logical pixels.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct ComponentPreview {
    pub values: Vec<(String, String)>,
    pub size: Option<(u32, u32)>,
}

impl ComponentPreview {
    pub fn value(&self, name: &str) -> Option<&str> {
        self.values.iter().find(|(n, _)| n == name).map(|(_, v)| v.as_str())
    }

    /// Sets the expression of a property; an empty one removes it.
    pub fn set_value(&mut self, name: &str, expr: &str) {
        let expr = clean_expr(expr);
        self.values.retain(|(n, _)| n != name);
        if !expr.is_empty() {
            self.values.push((name.to_string(), expr));
        }
    }

    fn is_empty(&self) -> bool {
        self.values.is_empty() && self.size.is_none()
    }
}

/// Slint identifier: the only form a stored property name may take.
fn is_identifier(name: &str) -> bool {
    let mut chars = name.chars();
    chars.next().is_some_and(|c| c.is_ascii_alphabetic() || c == '_')
        && chars.all(|c| c.is_ascii_alphanumeric() || c == '_' || c == '-')
}

/// Newlines and tabs cannot be stored in the line format; they become spaces.
fn clean_expr(expr: &str) -> String {
    expr.trim().replace(['\t', '\n', '\r'], " ")
}

/// The whole `.preview` file: `(file, component)` -> preview.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct PreviewConfig {
    entries: std::collections::BTreeMap<(String, String), ComponentPreview>,
}

impl PreviewConfig {
    pub fn parse(text: &str) -> Self {
        let mut cfg = Self::default();
        for line in text.lines() {
            let f: Vec<&str> = line.split('\t').collect();
            let [kind, file, comp, a, b] = f[..] else { continue };
            if file.is_empty() || comp.is_empty() || a.is_empty() {
                continue;
            }
            match kind {
                "value" if !b.trim().is_empty() && is_identifier(a) => {
                    cfg.entry_mut(file, comp).set_value(a, b);
                }
                "size" => {
                    if let (Ok(w), Ok(h)) = (a.parse::<u32>(), b.parse::<u32>())
                        && (1..=10_000).contains(&w)
                        && (1..=10_000).contains(&h)
                    {
                        cfg.entry_mut(file, comp).size = Some((w, h));
                    }
                }
                _ => {}
            }
        }
        cfg
    }

    pub fn render(&self) -> String {
        let mut out = String::new();
        for ((file, comp), p) in &self.entries {
            for (n, v) in &p.values {
                out += &format!("value\t{file}\t{comp}\t{n}\t{v}\n");
            }
            if let Some((w, h)) = p.size {
                out += &format!("size\t{file}\t{comp}\t{w}\t{h}\n");
            }
        }
        out
    }

    fn entry_mut(&mut self, file: &str, comp: &str) -> &mut ComponentPreview {
        self.entries.entry((file.to_string(), comp.to_string())).or_default()
    }

    pub fn get(&self, file: &str, comp: &str) -> ComponentPreview {
        self.entries.get(&(file.to_string(), comp.to_string())).cloned().unwrap_or_default()
    }

    /// Every component of `file` that has something stored.
    pub fn of_file(&self, file: &str) -> std::collections::BTreeMap<String, ComponentPreview> {
        self.entries
            .iter()
            .filter(|((f, _), _)| f == file)
            .map(|((_, c), p)| (c.clone(), p.clone()))
            .collect()
    }

    pub fn set(&mut self, file: &str, comp: &str, p: ComponentPreview) {
        let key = (file.to_string(), comp.to_string());
        if p.is_empty() {
            self.entries.remove(&key);
        } else {
            self.entries.insert(key, p);
        }
    }

    fn rename_file(&mut self, old: &str, new: &str) {
        let moved: Vec<_> = self.entries.keys().filter(|(f, _)| f == old).cloned().collect();
        for k in moved {
            if let Some(p) = self.entries.remove(&k) {
                self.entries.insert((new.to_string(), k.1), p);
            }
        }
    }
}

pub trait ProjectStore {
    /// The project's `.preview` (empty when missing, unreadable or corrupt).
    fn preview_config(&self, _project: &str) -> PreviewConfig {
        PreviewConfig::default()
    }
    fn set_preview_config(&self, _project: &str, _cfg: &PreviewConfig) -> io::Result<()> {
        Ok(())
    }
    /// Stores the preview of one component, keeping everything else in `.preview`.
    fn set_component_preview(
        &self,
        project: &str,
        file: &str,
        component: &str,
        p: &ComponentPreview,
    ) -> io::Result<()> {
        let mut cfg = self.preview_config(project);
        cfg.set(file, component, p.clone());
        self.set_preview_config(project, &cfg)
    }

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
    /// Deletes the project with its files and recovery copies, for good. A `last_open` that
    /// points into it is cleared. `NotFound` for an unknown project.
    fn delete_project(&self, _name: &str) -> io::Result<()> {
        Err(io::Error::new(io::ErrorKind::Unsupported, "this store cannot delete projects"))
    }
    /// Where the file lives on disk, so that `import`s and assets next to it resolve in the
    /// preview. `None` for stores that are not backed by files.
    fn location(&self, _project: &str, _file: &str) -> Option<PathBuf> {
        None
    }
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
    /// The project's folder on disk (it need not exist yet), for stores backed by files.
    fn project_path(&self, _project: &str) -> Option<PathBuf> {
        None
    }
    /// The GitHub token, if one is stored.
    fn github_token(&self) -> Option<String> {
        None
    }
    fn set_github_token(&self, _token: &str) -> io::Result<()> {
        Err(io::Error::new(io::ErrorKind::Unsupported, "no place to keep a token"))
    }
    fn clear_github_token(&self) -> io::Result<()> {
        Ok(())
    }
}

pub struct LocalStore {
    root: PathBuf,
    /// Where `.github-token` lives (the root unless a private directory was given).
    token_dir: PathBuf,
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
        let store = Self { token_dir: root.clone(), root };
        store.clean_deleting();
        Ok(store)
    }

    /// Like `new`, but the GitHub token is kept in `private` (e.g. Android's internal data
    /// directory, which a USB-connected PC cannot see). A token file left in `root` by an
    /// older version is moved there once, and the old copy removed.
    pub fn with_private_dir(root: PathBuf, private: PathBuf) -> io::Result<Self> {
        fs::create_dir_all(&root)?;
        fs::create_dir_all(&private)?;
        let store = Self { root, token_dir: private };
        store.clean_deleting();
        store.migrate_token()?;
        Ok(store)
    }

    fn migrate_token(&self) -> io::Result<()> {
        let old = self.root.join(GITHUB_TOKEN);
        if self.token_dir == self.root {
            return Ok(());
        }
        let Ok(text) = fs::read_to_string(&old) else { return Ok(()) };
        if self.github_token().is_none() && !text.trim().is_empty() {
            self.set_github_token(&text)?;
        }
        remove_if_exists(&old)
    }

    /// Removes the leftovers of an interrupted `delete_project`. Best effort.
    fn clean_deleting(&self) {
        let Ok(entries) = fs::read_dir(&self.root) else { return };
        for entry in entries.flatten() {
            if entry.file_name().to_string_lossy().starts_with(DELETING) {
                remove_tree(&entry.path());
            }
        }
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

/// Removes a folder (or a stray file or symlink) without following symlinks. Best effort.
fn remove_tree(path: &Path) {
    match fs::symlink_metadata(path) {
        Ok(m) if m.is_dir() => {
            let _ = fs::remove_dir_all(path);
        }
        Ok(_) => {
            let _ = fs::remove_file(path);
        }
        Err(_) => {}
    }
}

fn remove_if_exists(path: &Path) -> io::Result<()> {
    match fs::remove_file(path) {
        Err(e) if e.kind() == io::ErrorKind::NotFound => Ok(()),
        r => r,
    }
}

impl ProjectStore for LocalStore {
    fn preview_config(&self, project: &str) -> PreviewConfig {
        self.project_dir(project)
            .ok()
            .and_then(|d| fs::read(d.join(PREVIEW)).ok())
            .map(|b| PreviewConfig::parse(&String::from_utf8_lossy(&b)))
            .unwrap_or_default()
    }

    fn set_preview_config(&self, project: &str, cfg: &PreviewConfig) -> io::Result<()> {
        let dir = self.project_dir(project)?;
        let path = dir.join(PREVIEW);
        let text = cfg.render();
        if text.is_empty() {
            return remove_if_exists(&path);
        }
        let tmp = dir.join(".preview.tmp");
        fs::write(&tmp, text)?;
        fs::rename(&tmp, &path)
    }

    fn location(&self, project: &str, file: &str) -> Option<PathBuf> {
        self.file_path(project, file).ok()
    }

    fn project_path(&self, project: &str) -> Option<PathBuf> {
        self.project_dir(project).ok()
    }

    fn github_token(&self) -> Option<String> {
        let text = fs::read_to_string(self.token_dir.join(GITHUB_TOKEN)).ok()?;
        let t = text.trim();
        (!t.is_empty()).then(|| t.to_string())
    }

    fn set_github_token(&self, token: &str) -> io::Result<()> {
        let token = token.trim();
        if token.is_empty() || token.contains(|c: char| c.is_whitespace() || c.is_control()) {
            return Err(io::Error::new(io::ErrorKind::InvalidInput, "invalid token"));
        }
        let tmp = self.token_dir.join(".github-token.tmp");
        {
            use std::io::Write;
            let mut opts = fs::OpenOptions::new();
            opts.write(true).create(true).truncate(true);
            #[cfg(unix)]
            std::os::unix::fs::OpenOptionsExt::mode(&mut opts, 0o600);
            let mut f = opts.open(&tmp)?;
            f.write_all(token.as_bytes())?;
            f.write_all(b"\n")?;
        }
        fs::rename(&tmp, self.token_dir.join(GITHUB_TOKEN))
    }

    fn clear_github_token(&self) -> io::Result<()> {
        remove_if_exists(&self.token_dir.join(GITHUB_TOKEN))
    }

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
        let old_cfg = self.preview_config(project);
        let mut new_cfg = old_cfg.clone();
        new_cfg.rename_file(old, &new);
        let moved_preview = new_cfg != old_cfg;
        let from_rec = self.recovery_path(project, old)?;
        let to_rec = self.recovery_path(project, &new)?;
        let to_bak = to.with_file_name(format!("{new}.bak"));
        if to_bak.exists() || to_rec.exists() {
            return Err(io::Error::new(
                io::ErrorKind::AlreadyExists,
                format!("{new} has a backup or recovery copy"),
            ));
        }
        if moved_preview {
            // Before the file moves, so a failure here leaves everything as it was.
            self.set_preview_config(project, &new_cfg)?;
        }
        let moved = (|| {
            fs::rename(&from, &to)?;
            rename_if_exists(&from.with_file_name(format!("{old}.bak")), &to_bak)?;
            rename_if_exists(&from_rec, &to_rec)?;
            if self.last_open()? == Some((project.to_string(), old.to_string())) {
                self.set_last_open(project, &new)?;
            }
            Ok(())
        })();
        if let Err(e) = moved {
            if moved_preview {
                let _ = self.set_preview_config(project, &old_cfg);
            }
            return Err(e);
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

    fn delete_project(&self, name: &str) -> io::Result<()> {
        let dir = self.project_dir(name)?;
        // A real folder only: a symlink is not a project (it is never listed either).
        if !fs::symlink_metadata(&dir)?.is_dir() {
            return Err(io::ErrorKind::NotFound.into());
        }
        // Out of the way under one atomic rename; a crash after it leaves only a hidden
        // folder that the next start removes, never a half-deleted project.
        let gone = self.root.join(format!("{DELETING}{name}"));
        remove_tree(&gone);
        fs::rename(&dir, &gone)?;
        if let Some((p, _)) = self.last_open()?
            && p == name
        {
            remove_if_exists(&self.root.join(LAST_OPEN))?;
        }
        remove_tree(&gone);
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
    fn preview_roundtrip_and_corrupt_lines_ignored() {
        let (d, s) = store();
        s.create_project("p").unwrap();
        assert_eq!(s.preview_config("p"), PreviewConfig::default());
        let mut a = ComponentPreview::default();
        a.set_value("connected", "true");
        a.set_value("page", "1");
        a.set_value("name", "\"a b\"");
        a.size = Some((800, 360));
        s.set_component_preview("p", "main.slint", "MainWindow", &a).unwrap();
        let b = ComponentPreview { values: vec![], size: Some((914, 411)) };
        s.set_component_preview("p", "main.slint", "Other", &b).unwrap();
        assert_eq!(s.preview_config("p").get("main.slint", "MainWindow"), a);
        assert_eq!(s.preview_config("p").get("main.slint", "Other"), b);
        assert_eq!(s.preview_config("p").get("x.slint", "Other"), ComponentPreview::default());
        // Renaming the file keeps its entries.
        s.rename_file("p", "main.slint", "m2.slint").unwrap();
        assert_eq!(s.preview_config("p").get("m2.slint", "MainWindow"), a);
        // Corrupt lines are skipped, good ones kept.
        let path = d.path().join("projects/p/.preview");
        std::fs::write(
            &path,
            "garbage\nvalue\tf.slint\tC\tx\t1\nvalue\tf.slint\tC\tbad name\t2\nvalue\tf.slint\tC\tx:1px;}R{y\t2\nsize\tf.slint\tC\tabc\t3\nsize\tf.slint\tC\t0\t5\nvalue\tf.slint\n\u{0}\u{ff}",
        )
        .unwrap();
        let c = s.preview_config("p").get("f.slint", "C");
        assert_eq!(c.values, vec![("x".to_string(), "1".to_string())]);
        assert_eq!(c.size, None);
        std::fs::write(&path, [0xff, 0xfe, b'\t', 0, b'\n']).unwrap();
        assert_eq!(s.preview_config("p"), PreviewConfig::default());
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
    fn github_token_roundtrip_is_private_and_not_a_setting() {
        let (d, s) = store();
        assert_eq!(s.github_token(), None);
        s.set_github_token("  github_pat_abcd1234\n").unwrap();
        assert_eq!(s.github_token().as_deref(), Some("github_pat_abcd1234"));
        let path = d.path().join("projects/.github-token");
        assert!(path.is_file());
        assert!(!d.path().join("projects/.github-token.tmp").exists());
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            assert_eq!(fs::metadata(&path).unwrap().permissions().mode() & 0o777, 0o600);
        }
        // Not reachable through the settings, and settings writes leave it alone.
        assert_eq!(s.setting(".github-token"), None);
        assert!(s.set_setting(".github-token", "x").is_err());
        s.set_setting("tidy_tolerance", "20").unwrap();
        assert_eq!(s.github_token().as_deref(), Some("github_pat_abcd1234"));
        s.set_github_token("ghp_second").unwrap();
        assert_eq!(s.github_token().as_deref(), Some("ghp_second"));
        for bad in ["", "  ", "a b", "a\nb"] {
            assert_eq!(s.set_github_token(bad).unwrap_err().kind(), io::ErrorKind::InvalidInput);
        }
        assert_eq!(s.github_token().as_deref(), Some("ghp_second"));
        s.clear_github_token().unwrap();
        assert_eq!(s.github_token(), None);
        s.clear_github_token().unwrap();
        assert!(s.projects().unwrap().is_empty(), "the token file is not a project");
    }

    #[test]
    fn token_lives_in_the_private_dir_when_one_is_given() {
        let d = tempfile::tempdir().unwrap();
        let (root, private) = (d.path().join("projects"), d.path().join("private"));
        let s = LocalStore::with_private_dir(root.clone(), private.clone()).unwrap();
        s.set_github_token("ghp_privatetoken").unwrap();
        assert!(private.join(".github-token").is_file());
        assert!(!root.join(".github-token").exists());
        assert_eq!(s.github_token().as_deref(), Some("ghp_privatetoken"));
        s.clear_github_token().unwrap();
        assert!(!private.join(".github-token").exists());
        assert_eq!(s.github_token(), None);
    }

    #[test]
    fn an_old_token_in_the_projects_dir_is_moved_once() {
        let d = tempfile::tempdir().unwrap();
        let (root, private) = (d.path().join("projects"), d.path().join("private"));
        fs::create_dir_all(&root).unwrap();
        fs::write(root.join(".github-token"), "ghp_oldtoken\n").unwrap();
        let s = LocalStore::with_private_dir(root.clone(), private.clone()).unwrap();
        assert_eq!(s.github_token().as_deref(), Some("ghp_oldtoken"));
        assert!(!root.join(".github-token").exists(), "old copy deleted");
        assert!(private.join(".github-token").is_file());
        // A token already in the private dir wins; a stray old file is still removed.
        fs::write(root.join(".github-token"), "ghp_stale\n").unwrap();
        let s = LocalStore::with_private_dir(root.clone(), private).unwrap();
        assert_eq!(s.github_token().as_deref(), Some("ghp_oldtoken"));
        assert!(!root.join(".github-token").exists());
    }

    #[test]
    fn project_path_checks_the_name() {
        let (d, s) = store();
        assert_eq!(s.project_path("p"), Some(d.path().join("projects/p")));
        assert_eq!(s.project_path("../x"), None);
        assert_eq!(s.project_path(".hidden"), None);
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
    fn delete_project_removes_folder_recovery_and_last_open() {
        let (d, s) = store();
        s.create_project("p").unwrap();
        s.create_project("q").unwrap();
        s.write_recovery("p", "main.slint", "draft").unwrap();
        s.write("p", "main.slint", "x").unwrap(); // leaves a .bak too
        fs::create_dir_all(d.path().join("projects/p/sub/deep")).unwrap();
        fs::write(d.path().join("projects/p/sub/deep/f.bin"), [1]).unwrap();
        s.set_last_open("p", "main.slint").unwrap();
        s.delete_project("p").unwrap();
        assert_eq!(s.projects().unwrap(), vec!["q"]);
        assert!(!d.path().join("projects/p").exists());
        assert_eq!(fs::read_dir(d.path().join("projects")).unwrap().count(), 1, "no temp left");
        assert_eq!(s.last_open().unwrap(), None);
        assert_eq!(s.take_recovery("p", "main.slint").unwrap(), None);
        // The name is free again, and a fresh project does not see the old recovery.
        s.create_project("p").unwrap();
        assert_eq!(s.take_recovery("p", "main.slint").unwrap(), None);
    }

    #[test]
    fn delete_project_keeps_other_last_open_and_the_token() {
        let (d, s) = store();
        s.create_project("p").unwrap();
        s.create_project("o").unwrap();
        s.set_github_token("tok").unwrap();
        s.set_setting("tidy_tolerance", "4").unwrap();
        s.set_last_open("o", "main.slint").unwrap();
        s.delete_project("p").unwrap();
        assert_eq!(s.last_open().unwrap(), Some(("o".into(), "main.slint".into())));
        assert_eq!(s.github_token().as_deref(), Some("tok"));
        assert_eq!(s.setting("tidy_tolerance").as_deref(), Some("4"));
        // With a private token dir the token is outside the root and untouched too.
        let private = d.path().join("private");
        let s2 = LocalStore::with_private_dir(d.path().join("projects2"), private.clone()).unwrap();
        s2.create_project("p").unwrap();
        s2.set_github_token("tok2").unwrap();
        s2.delete_project("p").unwrap();
        assert_eq!(s2.github_token().as_deref(), Some("tok2"));
        assert!(private.join(GITHUB_TOKEN).is_file());
    }

    #[test]
    fn delete_project_refuses_bad_and_unknown_names() {
        let (d, s) = store();
        s.create_project("p").unwrap();
        fs::write(d.path().join("projects/file"), "x").unwrap();
        for n in ["", "..", "a/b", ".hidden", "x..y", ".github-token", ".deleting-p"] {
            assert_eq!(s.delete_project(n).unwrap_err().kind(), io::ErrorKind::InvalidInput, "{n:?}");
        }
        assert_eq!(s.delete_project("nope").unwrap_err().kind(), io::ErrorKind::NotFound);
        assert_eq!(s.delete_project("file").unwrap_err().kind(), io::ErrorKind::NotFound);
        assert!(d.path().join("projects/file").is_file());
        assert_eq!(s.projects().unwrap(), vec!["p"]);
    }

    #[cfg(unix)]
    #[test]
    fn delete_project_never_follows_symlinks_out_of_the_root() {
        let (d, s) = store();
        let outside = d.path().join("outside");
        fs::create_dir_all(&outside).unwrap();
        fs::write(outside.join("keep.txt"), "keep").unwrap();
        // A symlink inside a project is removed as a link, not followed.
        s.create_project("p").unwrap();
        std::os::unix::fs::symlink(&outside, d.path().join("projects/p/link")).unwrap();
        s.delete_project("p").unwrap();
        assert_eq!(fs::read_to_string(outside.join("keep.txt")).unwrap(), "keep");
        // A project folder that is itself a symlink is not a project.
        std::os::unix::fs::symlink(&outside, d.path().join("projects/evil")).unwrap();
        assert_eq!(s.delete_project("evil").unwrap_err().kind(), io::ErrorKind::NotFound);
        assert_eq!(fs::read_to_string(outside.join("keep.txt")).unwrap(), "keep");
    }

    #[test]
    fn a_leftover_deleting_folder_is_hidden_and_cleaned_at_start() {
        let (d, s) = store();
        s.create_project("p").unwrap();
        let left = d.path().join("projects/.deleting-q");
        fs::create_dir_all(left.join("sub")).unwrap();
        fs::write(left.join("sub/main.slint"), "x").unwrap();
        assert_eq!(s.projects().unwrap(), vec!["p"]);
        let s = LocalStore::new(d.path().join("projects")).unwrap();
        assert!(!left.exists());
        assert_eq!(s.projects().unwrap(), vec!["p"]);
        // Also cleaned when a stale one is in the way of a new delete.
        fs::create_dir_all(&left).unwrap();
        s.create_project("q").unwrap();
        s.delete_project("q").unwrap();
        assert!(!left.exists());
    }

    #[test]
    fn default_delete_is_unsupported() {
        struct Nope;
        impl ProjectStore for Nope {
            fn projects(&self) -> io::Result<Vec<String>> { Ok(vec![]) }
            fn create_project(&self, _: &str) -> io::Result<()> { Ok(()) }
            fn files(&self, _: &str) -> io::Result<Vec<String>> { Ok(vec![]) }
            fn create_file_with(&self, _: &str, _: &str, _: &str) -> io::Result<String> { Ok(String::new()) }
            fn rename_file(&self, _: &str, _: &str, _: &str) -> io::Result<String> { Ok(String::new()) }
            fn rename_project(&self, _: &str, _: &str) -> io::Result<()> { Ok(()) }
            fn read(&self, _: &str, _: &str) -> io::Result<String> { Ok(String::new()) }
            fn write(&self, _: &str, _: &str, _: &str) -> io::Result<()> { Ok(()) }
            fn write_recovery(&self, _: &str, _: &str, _: &str) -> io::Result<()> { Ok(()) }
            fn take_recovery(&self, _: &str, _: &str) -> io::Result<Option<String>> { Ok(None) }
            fn clear_recovery(&self, _: &str, _: &str) -> io::Result<()> { Ok(()) }
        }
        assert_eq!(Nope.delete_project("p").unwrap_err().kind(), io::ErrorKind::Unsupported);
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
