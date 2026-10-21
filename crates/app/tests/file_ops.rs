//! File menu operations: new file, rename file/project, save as.

use std::cell::Cell;
use std::io;
use std::rc::Rc;

use slint::ComponentHandle;
use storage::{LocalStore, ProjectStore};

const SRC: &str = "export component Main inherits Rectangle {\n    width: 300px;\n    height: 300px;\n    a := Rectangle {\n        x: 10px;\n        y: 10px;\n        width: 100px;\n        height: 100px;\n    }\n}\n";

const NEW_FILE: i32 = 1;
const RENAME_FILE: i32 = 2;
const RENAME_PROJECT: i32 = 3;
const SAVE_AS: i32 = 4;

fn seeded() -> (tempfile::TempDir, LocalStore) {
    let dir = tempfile::tempdir().unwrap();
    let store = LocalStore::new(dir.path().to_path_buf()).unwrap();
    store.create_project("default").unwrap();
    store.write("default", "main.slint", SRC).unwrap();
    store.write("default", "other.slint", SRC).unwrap();
    store.create_project("second").unwrap();
    (dir, store)
}

fn start(store: Box<dyn ProjectStore>) -> app::Editor {
    i_slint_backend_testing::init_no_event_loop();
    let editor = app::Editor::new().unwrap();
    app::glue::start(&editor, store).unwrap();
    editor.window().set_size(slint::LogicalSize::new(480., 800.));
    editor.show().unwrap();
    slint::platform::update_timers_and_animations();
    let _ = editor.window().size();
    editor
}

fn editor_for(store: LocalStore) -> app::Editor {
    start(Box::new(store))
}

fn fresh(dir: &tempfile::TempDir) -> LocalStore {
    LocalStore::new(dir.path().to_path_buf()).unwrap()
}

/// Selects the box and drags it, leaving the document dirty.
fn make_dirty(editor: &app::Editor) {
    editor.invoke_tapped(60., 60.);
    assert!(editor.get_has_selection());
    editor.invoke_drag_finished(40., 40.);
    assert!(editor.get_dirty());
}

fn run(editor: &app::Editor, mode: i32, name: &str, project: &str) {
    editor.invoke_file_op_run(mode, name.into(), project.into());
}

fn strings(m: slint::ModelRc<slint::SharedString>) -> Vec<String> {
    use slint::Model;
    m.iter().map(|s| s.to_string()).collect()
}

#[test]
fn save_as_writes_editor_text_to_new_file_and_leaves_original_alone() {
    let (dir, store) = seeded();
    let editor = editor_for(store);
    make_dirty(&editor);
    editor.set_file_op(SAVE_AS);
    run(&editor, SAVE_AS, "copy", "default");
    assert_eq!(editor.get_error_text().as_str(), "");
    assert_eq!(editor.get_dialog_error().as_str(), "");
    assert_eq!(editor.get_file_op(), 0, "dialog closes on success");
    assert_eq!(editor.get_current_project().as_str(), "default");
    assert_eq!(editor.get_current_file().as_str(), "copy.slint");
    assert!(!editor.get_dirty());
    let s = fresh(&dir);
    let saved = s.read("default", "copy.slint").unwrap();
    assert_ne!(saved, SRC, "the dragged text was written");
    assert!(saved.contains("x: 50px") || saved.contains("x: 52px"), "{saved}");
    assert_eq!(s.read("default", "main.slint").unwrap(), SRC, "original unchanged");
    assert_eq!(s.last_open().unwrap(), Some(("default".into(), "copy.slint".into())));
    assert!(strings(editor.get_files()).contains(&"copy.slint".to_string()));
    // Saving now means nothing to write.
    editor.invoke_save();
    assert_eq!(editor.get_message().as_str(), "No changes");
}

#[test]
fn save_as_then_edit_and_save_goes_to_the_new_file() {
    let (dir, store) = seeded();
    let editor = editor_for(store);
    make_dirty(&editor);
    run(&editor, SAVE_AS, "copy.slint", "default");
    let first = fresh(&dir).read("default", "copy.slint").unwrap();
    editor.invoke_drag_finished(20., 20.);
    assert!(editor.get_dirty());
    editor.invoke_save();
    let s = fresh(&dir);
    assert_ne!(s.read("default", "copy.slint").unwrap(), first);
    assert_eq!(s.read("default", "main.slint").unwrap(), SRC);
}

#[test]
fn save_as_into_another_project_switches_to_it() {
    let (dir, store) = seeded();
    let editor = editor_for(store);
    make_dirty(&editor);
    run(&editor, SAVE_AS, "moved", "second");
    assert_eq!(editor.get_current_project().as_str(), "second");
    assert_eq!(editor.get_current_file().as_str(), "moved.slint");
    assert_eq!(editor.get_menu_project().as_str(), "second");
    assert!(!editor.get_dirty());
    let s = fresh(&dir);
    assert!(s.read("second", "moved.slint").unwrap().contains("x: 52px"));
    assert_eq!(s.read("default", "main.slint").unwrap(), SRC);
    assert_eq!(s.last_open().unwrap(), Some(("second".into(), "moved.slint".into())));
}

#[test]
fn save_as_refuses_an_existing_name_without_changes() {
    let (dir, store) = seeded();
    let editor = editor_for(store);
    make_dirty(&editor);
    editor.set_file_op(SAVE_AS);
    run(&editor, SAVE_AS, "other", "default");
    assert!(!editor.get_dialog_error().is_empty());
    assert_eq!(editor.get_file_op(), SAVE_AS, "dialog stays open");
    assert_eq!(editor.get_current_file().as_str(), "main.slint");
    assert!(editor.get_dirty());
    let s = fresh(&dir);
    assert_eq!(s.read("default", "other.slint").unwrap(), SRC);
    assert_eq!(s.read("default", "main.slint").unwrap(), SRC);
}

#[test]
fn invalid_names_are_refused_everywhere() {
    let (dir, store) = seeded();
    let editor = editor_for(store);
    for mode in [NEW_FILE, RENAME_FILE, RENAME_PROJECT, SAVE_AS] {
        for bad in ["", "a/b", "..", ".hidden"] {
            editor.set_dialog_error("".into());
            run(&editor, mode, bad, "default");
            assert!(!editor.get_dialog_error().is_empty(), "mode {mode} {bad:?}");
            assert_eq!(editor.get_current_file().as_str(), "main.slint");
            assert_eq!(editor.get_current_project().as_str(), "default");
        }
    }
    let s = fresh(&dir);
    assert_eq!(s.files("default").unwrap(), vec!["main.slint", "other.slint"]);
    assert_eq!(s.projects().unwrap(), vec!["default", "second"]);
}

#[test]
fn rename_open_file_keeps_editing_in_the_new_file() {
    let (dir, store) = seeded();
    let editor = editor_for(store);
    make_dirty(&editor);
    editor.set_file_op(RENAME_FILE);
    run(&editor, RENAME_FILE, "renamed", "default");
    assert_eq!(editor.get_error_text().as_str(), "");
    assert_eq!(editor.get_file_op(), 0);
    assert_eq!(editor.get_current_file().as_str(), "renamed.slint");
    assert!(!editor.get_dirty(), "saved before renaming");
    let s = fresh(&dir);
    assert!(!dir.path().join("default/main.slint").exists());
    assert!(s.read("default", "renamed.slint").unwrap().contains("x: 52px"));
    assert_eq!(s.last_open().unwrap(), Some(("default".into(), "renamed.slint".into())));
    assert_eq!(strings(editor.get_files()), vec!["other.slint", "renamed.slint"]);
    // Same document: selection survives and further edits save to the new path.
    assert!(editor.get_has_selection());
    let before = s.read("default", "renamed.slint").unwrap();
    editor.invoke_drag_finished(20., 20.);
    assert!(editor.get_dirty());
    editor.invoke_save();
    assert_ne!(s.read("default", "renamed.slint").unwrap(), before);
    assert!(!dir.path().join("default/main.slint").exists(), "old path stays gone");
}

#[test]
fn rename_moves_bak_so_history_follows() {
    let (dir, store) = seeded();
    let editor = editor_for(store);
    make_dirty(&editor);
    editor.invoke_save(); // main.slint.bak = SRC
    run(&editor, RENAME_FILE, "r", "default");
    assert_eq!(std::fs::read_to_string(dir.path().join("default/r.slint.bak")).unwrap(), SRC);
    assert!(!dir.path().join("default/main.slint.bak").exists());
}

#[test]
fn rename_open_project_keeps_editing_in_the_new_project() {
    let (dir, store) = seeded();
    let editor = editor_for(store);
    make_dirty(&editor);
    editor.set_file_op(RENAME_PROJECT);
    run(&editor, RENAME_PROJECT, "proj2", "default");
    assert_eq!(editor.get_error_text().as_str(), "");
    assert_eq!(editor.get_file_op(), 0);
    assert_eq!(editor.get_current_project().as_str(), "proj2");
    assert_eq!(editor.get_current_file().as_str(), "main.slint");
    assert_eq!(editor.get_menu_project().as_str(), "proj2");
    assert!(!editor.get_dirty());
    assert!(!dir.path().join("default").exists());
    let s = fresh(&dir);
    assert!(s.read("proj2", "main.slint").unwrap().contains("x: 52px"));
    assert_eq!(s.last_open().unwrap(), Some(("proj2".into(), "main.slint".into())));
    assert_eq!(strings(editor.get_projects()), vec!["proj2", "second"]);
    editor.invoke_drag_finished(20., 20.);
    editor.invoke_save();
    assert!(!dir.path().join("default").exists(), "old path stays gone");
    assert_ne!(s.read("proj2", "main.slint").unwrap(), SRC);
}

#[test]
fn rename_other_project_leaves_the_open_document_alone() {
    let (dir, store) = seeded();
    let editor = editor_for(store);
    make_dirty(&editor);
    run(&editor, RENAME_PROJECT, "third", "second");
    assert_eq!(editor.get_current_project().as_str(), "default");
    assert!(editor.get_dirty(), "not saved: the open project was not touched");
    assert_eq!(strings(editor.get_projects()), vec!["default", "third"]);
    assert_eq!(editor.get_menu_project().as_str(), "third");
    assert_eq!(fresh(&dir).read("default", "main.slint").unwrap(), SRC);
}

#[test]
fn collisions_are_refused_with_no_changes() {
    let (dir, store) = seeded();
    let editor = editor_for(store);
    make_dirty(&editor);

    editor.set_file_op(RENAME_FILE);
    run(&editor, RENAME_FILE, "other", "default");
    assert!(!editor.get_dialog_error().is_empty());
    assert_eq!(editor.get_current_file().as_str(), "main.slint");
    assert!(editor.get_dirty(), "a refused rename does not save");

    editor.set_dialog_error("".into());
    run(&editor, RENAME_PROJECT, "second", "default");
    assert!(!editor.get_dialog_error().is_empty());
    assert_eq!(editor.get_current_project().as_str(), "default");

    editor.set_dialog_error("".into());
    run(&editor, NEW_FILE, "other", "default");
    assert!(!editor.get_dialog_error().is_empty());
    assert_eq!(editor.get_current_file().as_str(), "main.slint");

    let s = fresh(&dir);
    assert_eq!(s.read("default", "main.slint").unwrap(), SRC);
    assert_eq!(s.read("default", "other.slint").unwrap(), SRC);
    assert_eq!(s.projects().unwrap(), vec!["default", "second"]);
}

#[test]
fn new_file_is_created_and_opened() {
    let (dir, store) = seeded();
    let editor = editor_for(store);
    editor.set_file_op(NEW_FILE);
    run(&editor, NEW_FILE, "card", "default");
    assert_eq!(editor.get_file_op(), 0);
    assert_eq!(editor.get_current_file().as_str(), "card.slint");
    assert!(!editor.get_dirty());
    assert_eq!(editor.get_error_text().as_str(), "");
    let s = fresh(&dir);
    assert_eq!(
        s.read("default", "card.slint").unwrap(),
        "export component Main inherits Window {\n    preferred-width: 360px;\n    preferred-height: 640px;\n}\n"
    );
    assert_eq!(s.last_open().unwrap(), Some(("default".into(), "card.slint".into())));
    assert_eq!(strings(editor.get_files()), vec!["card.slint", "main.slint", "other.slint"]);
}

#[test]
fn new_file_in_another_project_opens_it_there() {
    let (dir, store) = seeded();
    let editor = editor_for(store);
    run(&editor, NEW_FILE, "n", "second");
    assert_eq!(editor.get_current_project().as_str(), "second");
    assert_eq!(editor.get_current_file().as_str(), "n.slint");
    assert!(fresh(&dir).read("second", "n.slint").is_ok());
}

struct Flaky {
    inner: LocalStore,
    fail_writes: Rc<Cell<bool>>,
    fail_creates: Rc<Cell<bool>>,
}

impl ProjectStore for Flaky {
    fn projects(&self) -> io::Result<Vec<String>> { self.inner.projects() }
    fn create_project(&self, n: &str) -> io::Result<()> { self.inner.create_project(n) }
    fn files(&self, p: &str) -> io::Result<Vec<String>> { self.inner.files(p) }
    fn create_file_with(&self, p: &str, n: &str, t: &str) -> io::Result<String> {
        if self.fail_creates.get() {
            return Err(io::Error::other("no space"));
        }
        self.inner.create_file_with(p, n, t)
    }
    fn rename_file(&self, p: &str, o: &str, n: &str) -> io::Result<String> { self.inner.rename_file(p, o, n) }
    fn rename_project(&self, o: &str, n: &str) -> io::Result<()> { self.inner.rename_project(o, n) }
    fn read(&self, p: &str, f: &str) -> io::Result<String> { self.inner.read(p, f) }
    fn write(&self, p: &str, f: &str, t: &str) -> io::Result<()> {
        if self.fail_writes.get() {
            return Err(io::Error::other("disk full"));
        }
        self.inner.write(p, f, t)
    }
    fn write_recovery(&self, p: &str, f: &str, t: &str) -> io::Result<()> { self.inner.write_recovery(p, f, t) }
    fn take_recovery(&self, p: &str, f: &str) -> io::Result<Option<String>> { self.inner.take_recovery(p, f) }
    fn clear_recovery(&self, p: &str, f: &str) -> io::Result<()> { self.inner.clear_recovery(p, f) }
    fn last_open(&self) -> io::Result<Option<(String, String)>> { self.inner.last_open() }
    fn set_last_open(&self, p: &str, f: &str) -> io::Result<()> { self.inner.set_last_open(p, f) }
}

#[test]
fn rename_is_aborted_when_the_save_before_it_fails() {
    let (dir, inner) = seeded();
    let fail = Rc::new(Cell::new(false));
    let editor = start(Box::new(Flaky { inner, fail_writes: fail.clone(), fail_creates: Rc::new(Cell::new(false)) }));
    make_dirty(&editor);
    fail.set(true);
    for (mode, name) in [(RENAME_FILE, "renamed"), (RENAME_PROJECT, "proj2")] {
        editor.set_file_op(mode);
        editor.set_error_text("".into());
        run(&editor, mode, name, "default");
        assert!(editor.get_error_text().contains("Save failed") || editor.get_dialog_error().contains("Save failed"));
        assert_eq!(editor.get_current_file().as_str(), "main.slint");
        assert_eq!(editor.get_current_project().as_str(), "default");
        assert!(editor.get_dirty());
    }
    let s = fresh(&dir);
    assert_eq!(s.files("default").unwrap(), vec!["main.slint", "other.slint"]);
    assert_eq!(s.projects().unwrap(), vec!["default", "second"]);
}

#[test]
fn save_as_does_not_need_the_original_to_be_writable() {
    let (dir, inner) = seeded();
    let fail = Rc::new(Cell::new(false));
    let editor = start(Box::new(Flaky { inner, fail_writes: fail.clone(), fail_creates: Rc::new(Cell::new(false)) }));
    make_dirty(&editor);
    run(&editor, SAVE_AS, "copy", "default");
    assert_eq!(editor.get_current_file().as_str(), "copy.slint");
    assert!(!editor.get_dirty());
    assert_eq!(fresh(&dir).read("default", "main.slint").unwrap(), SRC);
}

#[test]
fn save_as_with_failing_create_changes_nothing_and_leaves_no_file() {
    let (dir, inner) = seeded();
    let fail = Rc::new(Cell::new(false));
    let editor = start(Box::new(Flaky {
        inner,
        fail_writes: fail.clone(),
        fail_creates: fail.clone(),
    }));
    make_dirty(&editor);
    fail.set(true);
    editor.set_file_op(SAVE_AS);
    run(&editor, SAVE_AS, "copy", "default");
    assert!(!editor.get_dialog_error().is_empty());
    assert_eq!(editor.get_current_file().as_str(), "main.slint");
    assert!(editor.get_dirty());
    assert!(!dir.path().join("default/copy.slint").exists());
    assert!(!dir.path().join("default/copy.slint.bak").exists());
}

#[test]
fn save_as_leaves_no_bak_for_the_new_file() {
    let (dir, store) = seeded();
    let editor = editor_for(store);
    make_dirty(&editor);
    run(&editor, SAVE_AS, "copy", "default");
    assert_eq!(editor.get_current_file().as_str(), "copy.slint");
    assert!(dir.path().join("default/copy.slint").exists());
    assert!(!dir.path().join("default/copy.slint.bak").exists());
}

#[test]
fn save_as_clears_the_originals_recovery_copy_unconditionally() {
    use app::glue::startup_target;
    let (dir, store) = seeded();
    let editor = editor_for(store);
    make_dirty(&editor);
    let s = fresh(&dir);
    s.write_recovery("default", "main.slint", "// some older draft\n").unwrap();
    run(&editor, SAVE_AS, "copy", "default");
    assert_eq!(editor.get_current_file().as_str(), "copy.slint");
    assert_eq!(s.take_recovery("default", "main.slint").unwrap(), None);
    assert_ne!(startup_target(&s), Ok(("default".into(), "main.slint".into())));
    assert_eq!(s.read("default", "main.slint").unwrap(), SRC);
}

#[test]
fn rename_of_a_clean_file_moves_its_recovery_copy() {
    let (dir, store) = seeded();
    let editor = editor_for(store);
    assert!(!editor.get_dirty());
    // A draft of the open (clean) file: no save happens before the rename, so only the
    // store's rename can carry it over.
    fresh(&dir).write_recovery("default", "main.slint", "// draft\n").unwrap();
    run(&editor, RENAME_FILE, "moved", "default");
    assert_eq!(editor.get_current_file().as_str(), "moved.slint");
    let s = fresh(&dir);
    assert_eq!(s.take_recovery("default", "moved.slint").unwrap().as_deref(), Some("// draft\n"));
    assert_eq!(s.take_recovery("default", "main.slint").unwrap(), None);
    assert_eq!(s.read("default", "moved.slint").unwrap(), SRC);
}

#[test]
fn rename_of_a_clean_project_moves_its_recovery_copy() {
    let (dir, store) = seeded();
    let editor = editor_for(store);
    fresh(&dir).write_recovery("default", "main.slint", "// draft\n").unwrap();
    run(&editor, RENAME_PROJECT, "proj2", "default");
    let s = fresh(&dir);
    assert_eq!(s.take_recovery("proj2", "main.slint").unwrap().as_deref(), Some("// draft\n"));
}
