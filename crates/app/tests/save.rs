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
    assert_eq!(pending_recovery(&store, "p", "main.slint", SRC), None);
    store.write_recovery("p", "main.slint", SRC).unwrap();
    assert_eq!(pending_recovery(&store, "p", "main.slint", SRC), None);
    store.write_recovery("p", "main.slint", "draft").unwrap();
    assert_eq!(pending_recovery(&store, "p", "main.slint", SRC).as_deref(), Some("draft"));
}
