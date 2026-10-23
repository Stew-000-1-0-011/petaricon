//! Component uses are single nodes; 「中を開く」 / 「戻る」 move between definitions.

use app::glue::load_session;
use app::nav::{Nav, go_back, open_inside};
use app::session::Session;
use slint_doc::ElementPath;
use slint::ComponentHandle;
use storage::{LocalStore, ProjectStore};

const MAIN: &str = r#"import { Remote } from "remote.slint";
component Local inherits Rectangle {
    width: 100px;
    height: 100px;
    Rectangle {
        x: 10px;
        y: 10px;
        width: 20px;
        height: 20px;
    }
}
export component Main inherits Rectangle {
    width: 300px;
    height: 300px;
    Local {
        x: 0px;
        y: 0px;
    }
    Remote {
        x: 150px;
        y: 0px;
    }
    Rectangle {
        x: 0px;
        y: 200px;
        width: 50px;
        height: 50px;
    }
}
"#;

const REMOTE: &str = r#"component Inner inherits Rectangle {
    width: 30px;
    height: 30px;
    Rectangle {
        x: 5px;
        y: 5px;
        width: 10px;
        height: 10px;
    }
}
export component Remote inherits Rectangle {
    width: 100px;
    height: 100px;
    Rectangle {
        x: 10px;
        y: 10px;
        width: 20px;
        height: 20px;
    }
    Inner {
        x: 50px;
        y: 50px;
    }
}
"#;

fn path(c: &str, i: &[usize]) -> ElementPath {
    ElementPath { component: c.into(), indices: i.to_vec() }
}

struct Fixture {
    _dir: tempfile::TempDir,
    store: LocalStore,
    file: String,
    session: Session,
    nav: Nav,
}

fn fixture() -> Fixture {
    let _ = std::panic::catch_unwind(i_slint_backend_testing::init_no_event_loop);
    let dir = tempfile::tempdir().unwrap();
    let store = LocalStore::new(dir.path().to_path_buf()).unwrap();
    store.create_project("p").unwrap();
    std::fs::write(dir.path().join("p/main.slint"), MAIN).unwrap();
    std::fs::write(dir.path().join("p/remote.slint"), REMOTE).unwrap();
    let mut session = load_session(&store, "p", "main.slint").unwrap();
    session.set_component("Main");
    assert!(session.rebuild_standalone(300., 300.), "{:?}", session.diagnostics());
    Fixture { _dir: dir, store, file: "main.slint".into(), session, nav: Nav::default() }
}

fn load(store: &LocalStore, f: &str) -> Result<Session, String> {
    let mut s = load_session(store, "p", f)?;
    assert!(s.rebuild_standalone(300., 300.), "{:?}", s.diagnostics());
    Ok(s)
}

#[test]
fn tapping_inside_a_same_file_use_selects_the_use() {
    let mut f = fixture();
    assert_eq!(f.session.tap(15., 15.), Some(path("Main", &[0])));
}

#[test]
fn tapping_inside_an_imported_use_selects_the_use() {
    let mut f = fixture();
    assert_eq!(f.session.tap(165., 15.), Some(path("Main", &[1])));
}

#[test]
fn a_use_moves_as_one_node() {
    let mut f = fixture();
    f.session.tap(15., 15.);
    f.session.move_selection(20., 8.).unwrap();
    assert!(f.session.doc().text().contains("x: 20px;\n        y: 8px;"));
}

#[test]
fn plain_elements_are_still_selectable_and_uses_are_detected() {
    let mut f = fixture();
    f.session.tap(10., 210.);
    assert_eq!(f.session.selection(), Some(&path("Main", &[2])));
    assert!(f.session.selected_use().is_none());
    assert!(matches!(
        open_inside(&f.store, "p", &mut f.file, &mut f.session, true, &mut f.nav, &|a| load(&f.store, a)),
        Err(e) if e.contains("利用")
    ));
}

#[test]
fn open_inside_same_file_switches_component_and_back_restores() {
    let mut f = fixture();
    f.session.tap(15., 15.);
    open_inside(&f.store, "p", &mut f.file, &mut f.session, true, &mut f.nav, &|a| load(&f.store, a))
        .unwrap();
    assert_eq!(f.file, "main.slint");
    assert_eq!(f.session.component(), "Local");
    assert!(f.session.selection().is_none());
    assert_eq!(f.nav.depth(), 1);
    go_back(&f.store, "p", &mut f.file, &mut f.session, true, &mut f.nav, &|a| load(&f.store, a))
        .unwrap();
    assert_eq!(f.session.component(), "Main");
    assert_eq!(f.session.selection(), Some(&path("Main", &[0])));
    assert_eq!(f.nav.depth(), 0);
}

#[test]
fn open_inside_other_file_follows_import_saves_first_and_back_restores() {
    let mut f = fixture();
    // An unsaved edit in main.slint.
    f.session.tap(165., 15.);
    f.session.move_selection(8., 8.).unwrap();
    assert!(f.session.doc().is_dirty());
    let edited = f.session.doc().text().to_string();
    open_inside(&f.store, "p", &mut f.file, &mut f.session, true, &mut f.nav, &|a| load(&f.store, a))
        .unwrap();
    assert_eq!(f.file, "remote.slint");
    assert_eq!(f.session.component(), "Remote");
    // main.slint was written before leaving.
    assert_eq!(f.store.read("p", "main.slint").unwrap(), edited);
    // Edit remote.slint, unsaved, then go back: it is saved too.
    f.session.tap(15., 15.);
    assert_eq!(f.session.selection(), Some(&path("Remote", &[0])));
    f.session.move_selection(4., 4.).unwrap();
    let remote_edited = f.session.doc().text().to_string();
    go_back(&f.store, "p", &mut f.file, &mut f.session, true, &mut f.nav, &|a| load(&f.store, a))
        .unwrap();
    assert_eq!(f.store.read("p", "remote.slint").unwrap(), remote_edited);
    assert_eq!(f.file, "main.slint");
    assert_eq!(f.session.component(), "Main");
    assert_eq!(f.session.selection(), Some(&path("Main", &[1])));
    assert_eq!(f.session.doc().text(), edited);
    assert!(f.nav.is_empty());
}

#[test]
fn back_works_several_levels_deep_across_files() {
    let mut f = fixture();
    f.session.tap(165., 15.);
    open_inside(&f.store, "p", &mut f.file, &mut f.session, true, &mut f.nav, &|a| load(&f.store, a)).unwrap();
    assert_eq!(f.session.tap(60., 60.), Some(path("Remote", &[1])));
    open_inside(&f.store, "p", &mut f.file, &mut f.session, true, &mut f.nav, &|a| load(&f.store, a)).unwrap();
    assert_eq!((f.file.as_str(), f.session.component()), ("remote.slint", "Inner"));
    assert_eq!(f.nav.depth(), 2);
    go_back(&f.store, "p", &mut f.file, &mut f.session, true, &mut f.nav, &|a| load(&f.store, a)).unwrap();
    assert_eq!((f.file.as_str(), f.session.component()), ("remote.slint", "Remote"));
    assert_eq!(f.session.selection(), Some(&path("Remote", &[1])));
    go_back(&f.store, "p", &mut f.file, &mut f.session, true, &mut f.nav, &|a| load(&f.store, a)).unwrap();
    assert_eq!((f.file.as_str(), f.session.component()), ("main.slint", "Main"));
    assert_eq!(f.session.selection(), Some(&path("Main", &[1])));
    assert!(
        go_back(&f.store, "p", &mut f.file, &mut f.session, true, &mut f.nav, &|a| load(&f.store, a)).is_err()
    );
}

/// A store whose writes fail.
struct NoWrite(LocalStore);

impl ProjectStore for NoWrite {
    fn projects(&self) -> std::io::Result<Vec<String>> { self.0.projects() }
    fn create_project(&self, n: &str) -> std::io::Result<()> { self.0.create_project(n) }
    fn files(&self, p: &str) -> std::io::Result<Vec<String>> { self.0.files(p) }
    fn create_file_with(&self, p: &str, n: &str, t: &str) -> std::io::Result<String> { self.0.create_file_with(p, n, t) }
    fn rename_file(&self, p: &str, o: &str, n: &str) -> std::io::Result<String> { self.0.rename_file(p, o, n) }
    fn rename_project(&self, o: &str, n: &str) -> std::io::Result<()> { self.0.rename_project(o, n) }
    fn read(&self, p: &str, f: &str) -> std::io::Result<String> { self.0.read(p, f) }
    fn write(&self, _: &str, _: &str, _: &str) -> std::io::Result<()> { Err(std::io::Error::other("disk full")) }
    fn write_recovery(&self, p: &str, f: &str, t: &str) -> std::io::Result<()> { self.0.write_recovery(p, f, t) }
    fn take_recovery(&self, p: &str, f: &str) -> std::io::Result<Option<String>> { self.0.take_recovery(p, f) }
    fn clear_recovery(&self, p: &str, f: &str) -> std::io::Result<()> { self.0.clear_recovery(p, f) }
}

#[test]
fn a_failed_save_keeps_everything_where_it_is() {
    let mut f = fixture();
    f.session.tap(165., 15.);
    f.session.move_selection(8., 8.).unwrap();
    let broken = NoWrite(LocalStore::new(f._dir.path().to_path_buf()).unwrap());
    let r = open_inside(&broken, "p", &mut f.file, &mut f.session, true, &mut f.nav, &|a| load(&f.store, a));
    assert!(r.unwrap_err().contains("保存に失敗しました"));
    assert_eq!(f.file, "main.slint");
    assert_eq!(f.session.component(), "Main");
    assert!(f.nav.is_empty());
    assert!(f.session.doc().is_dirty());
}

#[test]
fn the_editor_buttons_follow_selection_and_stack() {
    i_slint_backend_testing::init_no_event_loop();
    let dir = tempfile::tempdir().unwrap();
    let store = LocalStore::new(dir.path().to_path_buf()).unwrap();
    store.create_project("p").unwrap();
    store.write("p", "main.slint", MAIN).unwrap();
    store.write("p", "remote.slint", REMOTE).unwrap();
    let editor = app::Editor::new().unwrap();
    app::glue::start(&editor, Box::new(store)).unwrap();
    editor.window().set_size(slint::LogicalSize::new(480., 800.));
    editor.show().unwrap();
    slint::platform::update_timers_and_animations();
    let _ = editor.window().size();
    editor.invoke_choose_component("Main".into());
    slint::platform::update_timers_and_animations();

    assert!(!editor.get_can_open_inside() && !editor.get_can_go_back());
    editor.invoke_tapped(165., 15.);
    assert!(editor.get_can_open_inside());
    editor.invoke_open_inside();
    assert_eq!(editor.get_current_file().as_str(), "remote.slint");
    assert_eq!(editor.get_current_component().as_str(), "Remote");
    assert!(editor.get_can_go_back());
    assert!(!editor.get_can_open_inside());
    editor.invoke_go_back();
    assert_eq!(editor.get_current_file().as_str(), "main.slint");
    assert_eq!(editor.get_current_component().as_str(), "Main");
    assert!(editor.get_can_open_inside(), "the use is selected again");
    assert!(!editor.get_can_go_back());
}

#[test]
fn import_of_a_missing_file_is_an_error_and_changes_nothing() {
    let mut f = fixture();
    std::fs::remove_file(f._dir.path().join("p/remote.slint")).unwrap();
    f.session.tap(165., 15.);
    let r = open_inside(&f.store, "p", &mut f.file, &mut f.session, true, &mut f.nav, &|a| load(&f.store, a));
    assert!(r.is_err());
    assert_eq!(f.file, "main.slint");
    assert!(f.nav.is_empty());
}

#[test]
fn nonnormalized_and_symlinked_paths_still_have_geometry() {
    let _ = std::panic::catch_unwind(i_slint_backend_testing::init_no_event_loop);
    let dir = tempfile::tempdir().unwrap();
    std::fs::create_dir(dir.path().join("real")).unwrap();
    std::fs::write(dir.path().join("real/main.slint"), MAIN).unwrap();
    std::fs::write(dir.path().join("real/remote.slint"), REMOTE).unwrap();
    std::os::unix::fs::symlink(dir.path().join("real"), dir.path().join("link")).unwrap();
    for rel in ["real/../real/main.slint", "link/main.slint", "./link/../real/main.slint"] {
        let p = dir.path().join(rel);
        let mut s = Session::new(slint_doc::Document::new(p.clone(), MAIN.into()));
        s.set_component("Main");
        assert!(s.rebuild_standalone(300., 300.), "{rel}: {:?}", s.diagnostics());
        s.tap(15., 15.);
        assert_eq!(s.selection(), Some(&path("Main", &[0])), "{rel}");
        assert!(s.selection_hit().is_some(), "{rel}");
    }
}
