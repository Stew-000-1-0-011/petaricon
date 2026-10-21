use std::cell::Cell;
use std::io;
use std::path::PathBuf;
use std::rc::Rc;

use app::glue::save_if_dirty;
use app::session::Session;
use slint::ComponentHandle;
use slint_doc::Document;
use storage::{LocalStore, ProjectStore};

const SRC: &str = "export component Main inherits Rectangle {\n    width: 300px;\n    height: 300px;\n    a := Rectangle {\n        x: 10px;\n        y: 10px;\n        width: 100px;\n        height: 100px;\n    }\n}\n";

fn setup() -> (tempfile::TempDir, LocalStore) {
    let dir = tempfile::tempdir().unwrap();
    let store = LocalStore::new(dir.path().to_path_buf()).unwrap();
    store.create_project("p").unwrap();
    store.write("p", "main.slint", SRC).unwrap();
    (dir, store)
}

fn session(text: &str) -> Session {
    Session::new(Document::new(PathBuf::from("p/main.slint"), text.into()))
}

#[test]
fn clean_document_is_not_written_and_bak_survives() {
    let (dir, store) = setup();
    store.write("p", "main.slint", "// v2\n").unwrap(); // .bak now holds SRC
    let mut s = session("// v2\n");
    assert_eq!(save_if_dirty(&store, "p", "main.slint", &mut s, true), Ok(false));
    let bak = std::fs::read_to_string(dir.path().join("p/main.slint.bak")).unwrap();
    assert_eq!(bak, SRC);
}

#[test]
fn dirty_document_is_written_once_and_marked_saved() {
    let (dir, store) = setup();
    let mut s = session(SRC);
    s.replace_text(format!("{SRC}// edited\n"));
    assert!(s.doc().is_dirty());
    assert_eq!(save_if_dirty(&store, "p", "main.slint", &mut s, true), Ok(true));
    assert!(!s.doc().is_dirty());
    assert_eq!(store.read("p", "main.slint").unwrap(), format!("{SRC}// edited\n"));
    // A second save with no new edits must not rotate the backup again.
    assert_eq!(save_if_dirty(&store, "p", "main.slint", &mut s, true), Ok(false));
    let bak = std::fs::read_to_string(dir.path().join("p/main.slint.bak")).unwrap();
    assert_eq!(bak, SRC);
}

#[test]
fn unloaded_session_is_never_saved() {
    let (_dir, store) = setup();
    let mut s = session("");
    s.replace_text("// scribble\n".into());
    assert!(save_if_dirty(&store, "p", "main.slint", &mut s, false).is_err());
    assert_eq!(store.read("p", "main.slint").unwrap(), SRC);
}

struct Flaky {
    inner: LocalStore,
    fail_writes: Rc<Cell<bool>>,
}

impl ProjectStore for Flaky {
    fn projects(&self) -> io::Result<Vec<String>> { self.inner.projects() }
    fn create_project(&self, n: &str) -> io::Result<()> { self.inner.create_project(n) }
    fn files(&self, p: &str) -> io::Result<Vec<String>> { self.inner.files(p) }
    fn create_file(&self, p: &str, n: &str) -> io::Result<String> { self.inner.create_file(p, n) }
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
}

#[test]
fn open_file_aborts_when_saving_the_current_file_fails() {
    i_slint_backend_testing::init_no_event_loop();
    let dir = tempfile::tempdir().unwrap();
    let inner = LocalStore::new(dir.path().to_path_buf()).unwrap();
    inner.create_project("default").unwrap();
    inner.write("default", "main.slint", SRC).unwrap();
    inner.write("default", "other.slint", SRC).unwrap();
    let fail = Rc::new(Cell::new(false));
    let editor = app::Editor::new().unwrap();
    app::glue::start(&editor, Box::new(Flaky { inner, fail_writes: fail.clone() })).unwrap();
    editor.window().set_size(slint::LogicalSize::new(480., 800.));
    editor.show().unwrap();
    slint::platform::update_timers_and_animations();
    let _ = editor.window().size();

    editor.invoke_tapped(60., 60.);
    assert!(editor.get_has_selection());
    editor.invoke_drag_finished(40., 40.);
    assert!(editor.get_dirty());

    fail.set(true);
    editor.invoke_open("default".into(), "other.slint".into());
    assert_eq!(editor.get_current_file().as_str(), "main.slint");
    assert!(editor.get_dirty(), "edits must still be pending");
    assert!(editor.get_error_text().contains("Save failed"));

    fail.set(false);
    editor.invoke_open("default".into(), "other.slint".into());
    assert_eq!(editor.get_current_file().as_str(), "other.slint");
}

#[test]
fn recovery_tick_writes_only_changed_dirty_text_and_never_the_real_file() {
    use app::glue::recovery_tick;
    let (_d, store) = setup();
    let mut s = session(SRC);
    let mut last = None;
    // Clean: nothing written.
    assert_eq!(recovery_tick(&store, "p", "main.slint", &s, true, &mut last), Ok(false));
    assert_eq!(store.take_recovery("p", "main.slint").unwrap(), None);
    s.replace_text(format!("{SRC}// a\n"));
    assert_eq!(recovery_tick(&store, "p", "main.slint", &s, true, &mut last), Ok(true));
    assert_eq!(
        store.take_recovery("p", "main.slint").unwrap().as_deref(),
        Some(format!("{SRC}// a\n").as_str())
    );
    // Same text: no second write.
    assert_eq!(recovery_tick(&store, "p", "main.slint", &s, true, &mut last), Ok(false));
    s.replace_text(format!("{SRC}// b\n"));
    assert_eq!(recovery_tick(&store, "p", "main.slint", &s, true, &mut last), Ok(true));
    assert_eq!(store.read("p", "main.slint").unwrap(), SRC);
    // Saving clears the copy; a later clean tick leaves things alone.
    assert_eq!(save_if_dirty(&store, "p", "main.slint", &mut s, true), Ok(true));
    assert_eq!(store.take_recovery("p", "main.slint").unwrap(), None);
    assert_eq!(recovery_tick(&store, "p", "main.slint", &s, true, &mut last), Ok(false));
    assert_eq!(last, None);
}

#[test]
fn recovery_tick_skips_unloaded_and_pending_recovery_ignores_equal_text() {
    use app::glue::{pending_recovery, recovery_tick};
    let (_d, store) = setup();
    let mut s = session("");
    s.replace_text("x".into());
    let mut last = None;
    assert_eq!(recovery_tick(&store, "p", "main.slint", &s, false, &mut last), Ok(false));
    assert_eq!(pending_recovery(&store, "p", "main.slint", SRC), Ok(None));
    store.write_recovery("p", "main.slint", SRC).unwrap();
    assert_eq!(pending_recovery(&store, "p", "main.slint", SRC), Ok(None));
    assert_eq!(store.take_recovery("p", "main.slint").unwrap(), None, "equal copy is cleared");
    store.write_recovery("p", "main.slint", "draft").unwrap();
    assert_eq!(
        pending_recovery(&store, "p", "main.slint", SRC),
        Ok(Some("draft".to_string()))
    );
}

fn glue_editor(store: LocalStore) -> app::Editor {
    i_slint_backend_testing::init_no_event_loop();
    let editor = app::Editor::new().unwrap();
    app::glue::start(&editor, Box::new(store)).unwrap();
    editor.window().set_size(slint::LogicalSize::new(480., 800.));
    editor.show().unwrap();
    slint::platform::update_timers_and_animations();
    editor
}

fn two_files() -> (tempfile::TempDir, LocalStore) {
    let dir = tempfile::tempdir().unwrap();
    let store = LocalStore::new(dir.path().to_path_buf()).unwrap();
    store.create_project("default").unwrap();
    store.write("default", "main.slint", SRC).unwrap();
    store.write("default", "other.slint", SRC).unwrap();
    (dir, store)
}

fn read_back(dir: &tempfile::TempDir, f: &str) -> String {
    LocalStore::new(dir.path().to_path_buf())
        .unwrap()
        .read("default", f)
        .unwrap()
}

#[test]
fn stale_recovery_of_another_file_does_not_prompt_on_open() {
    let (dir, store) = two_files();
    let editor = glue_editor(store);
    assert!(!editor.get_recovery_open());
    // A stale copy for main.slint appears; opening other.slint must not prompt about it.
    let s = LocalStore::new(dir.path().to_path_buf()).unwrap();
    s.write_recovery("default", "main.slint", "// stale\n").unwrap();
    editor.invoke_open("default".into(), "other.slint".into());
    assert_eq!(editor.get_current_file().as_str(), "other.slint");
    assert!(!editor.get_recovery_open());
    assert!(!editor.get_dirty());
    assert_eq!(read_back(&dir, "other.slint"), SRC);
}

#[test]
fn recovery_prompt_no_clears_copy_and_keeps_text() {
    let (dir, store) = two_files();
    store.write_recovery("default", "other.slint", "// draft\n").unwrap();
    let editor = glue_editor(store);
    // The file with a pending recovery copy is opened first, so the prompt shows at startup.
    assert_eq!(editor.get_current_file().as_str(), "other.slint");
    assert!(editor.get_recovery_open());
    editor.invoke_recovery_answered(false);
    assert!(!editor.get_recovery_open());
    assert!(!editor.get_dirty());
    let s = LocalStore::new(dir.path().to_path_buf()).unwrap();
    assert_eq!(s.take_recovery("default", "other.slint").unwrap(), None);
    assert_eq!(read_back(&dir, "other.slint"), SRC);
}

#[test]
fn recovery_prompt_yes_replaces_text_and_marks_dirty_without_touching_file() {
    let (dir, store) = two_files();
    let draft = SRC.replace("300px", "250px");
    store.write_recovery("default", "other.slint", &draft).unwrap();
    let editor = glue_editor(store);
    assert_eq!(editor.get_current_file().as_str(), "other.slint");
    assert!(editor.get_recovery_open());
    editor.invoke_recovery_answered(true);
    assert!(!editor.get_recovery_open());
    assert!(editor.get_dirty());
    assert_eq!(read_back(&dir, "other.slint"), SRC, "file untouched until saved");
    editor.invoke_save();
    assert_eq!(read_back(&dir, "other.slint"), draft);
}

#[test]
fn startup_opens_the_file_with_a_recovery_copy_in_any_project() {
    use app::glue::startup_target;
    let (dir, store) = two_files();
    store.create_project("aaa").unwrap();
    store.write("aaa", "z.slint", SRC).unwrap();
    store.write_recovery("aaa", "z.slint", "// draft\n").unwrap();
    assert_eq!(startup_target(&store), Ok(("aaa".into(), "z.slint".into())));
    // A copy equal to the file is not a recovery.
    store.write_recovery("aaa", "z.slint", SRC).unwrap();
    assert_eq!(startup_target(&store), Ok(("default".into(), "main.slint".into())));
    drop(dir);
}

#[test]
fn startup_reopens_the_last_open_file_and_falls_back_when_it_is_gone() {
    use app::glue::startup_target;
    let (_d, store) = two_files();
    assert_eq!(startup_target(&store), Ok(("default".into(), "main.slint".into())));
    store.set_last_open("default", "other.slint").unwrap();
    assert_eq!(startup_target(&store), Ok(("default".into(), "other.slint".into())));
    store.set_last_open("default", "gone.slint").unwrap();
    assert_eq!(startup_target(&store), Ok(("default".into(), "main.slint".into())));
    // Recovery wins over last-open.
    store.set_last_open("default", "other.slint").unwrap();
    store.write_recovery("default", "main.slint", "// draft\n").unwrap();
    assert_eq!(startup_target(&store), Ok(("default".into(), "main.slint".into())));
}

#[test]
fn opening_a_file_remembers_it_for_next_start() {
    let (dir, store) = two_files();
    let editor = glue_editor(store);
    editor.invoke_open("default".into(), "other.slint".into());
    let s = LocalStore::new(dir.path().to_path_buf()).unwrap();
    assert_eq!(s.last_open().unwrap(), Some(("default".into(), "other.slint".into())));
}

#[test]
fn save_without_changes_says_no_changes() {
    let (_d, store) = two_files();
    let editor = glue_editor(store);
    editor.invoke_save();
    assert_eq!(editor.get_message().as_str(), "No changes");
    editor.invoke_tapped(60., 60.);
    editor.invoke_drag_finished(40., 40.);
    editor.invoke_save();
    assert_eq!(editor.get_message().as_str(), "Saved");
}

#[test]
fn failed_save_writes_a_recovery_copy() {
    use app::glue::save_or_recover;
    let dir = tempfile::tempdir().unwrap();
    let inner = LocalStore::new(dir.path().to_path_buf()).unwrap();
    inner.create_project("p").unwrap();
    inner.write("p", "main.slint", SRC).unwrap();
    let fail = Rc::new(Cell::new(true));
    let store = Flaky { inner, fail_writes: fail };
    let mut s = session(SRC);
    s.replace_text(format!("{SRC}// edited\n"));
    let r = save_or_recover(&store, "p", "main.slint", &mut s, true);
    assert!(r.unwrap_err().contains("Save failed"));
    assert!(s.doc().is_dirty());
    assert_eq!(
        store.take_recovery("p", "main.slint").unwrap().as_deref(),
        Some(format!("{SRC}// edited\n").as_str())
    );
    assert_eq!(store.read("p", "main.slint").unwrap(), SRC);
}

#[test]
fn closing_with_a_failing_save_keeps_the_window_and_shows_the_error() {
    use slint::platform::WindowEvent;
    i_slint_backend_testing::init_no_event_loop();
    let dir = tempfile::tempdir().unwrap();
    let inner = LocalStore::new(dir.path().to_path_buf()).unwrap();
    inner.create_project("default").unwrap();
    inner.write("default", "main.slint", SRC).unwrap();
    let fail = Rc::new(Cell::new(false));
    let editor = app::Editor::new().unwrap();
    app::glue::start(&editor, Box::new(Flaky { inner, fail_writes: fail.clone() })).unwrap();
    editor.window().set_size(slint::LogicalSize::new(480., 800.));
    editor.show().unwrap();
    slint::platform::update_timers_and_animations();
    editor.invoke_tapped(60., 60.);
    editor.invoke_drag_finished(40., 40.);
    assert!(editor.get_dirty());
    fail.set(true);
    editor.window().dispatch_event(WindowEvent::CloseRequested);
    assert!(editor.window().is_visible(), "window must stay open");
    assert!(editor.get_error_text().contains("Save failed"));
    let s = LocalStore::new(dir.path().to_path_buf()).unwrap();
    assert!(s.take_recovery("default", "main.slint").unwrap().is_some());
    // Once saving works, closing hides the window.
    fail.set(false);
    editor.window().dispatch_event(WindowEvent::CloseRequested);
    assert!(!editor.window().is_visible());
}
