use std::path::PathBuf;

use app::session::{Corner, Session, SessionError};
use slint_doc::{Document, ElementPath};

const SRC: &str = r#"export component Main inherits Rectangle {
    width: 300px;
    height: 300px;
    a := Rectangle {
        x: 10px;
        y: 10px;
        width: 100px;
        height: 100px;
        b := Rectangle {
            x: 10px;
            y: 10px;
            width: 20px;
            height: 20px;
        }
    }
    VerticalLayout {
        y: 200px;
        height: 50px;
        Text { text: "in"; }
    }
}
"#;

fn path(i: &[usize]) -> ElementPath {
    ElementPath {
        component: "Main".into(),
        indices: i.to_vec(),
    }
}

fn session() -> Session {
    i_slint_backend_testing::init_no_event_loop();
    let mut s = Session::new(Document::new(PathBuf::from("/t/main.slint"), SRC.into()));
    assert!(s.rebuild_standalone(300., 300.));
    s
}

#[test]
fn tap_selects_deepest_then_parent() {
    let mut s = session();
    assert_eq!(s.tap(25., 25.), Some(path(&[0, 0])));
    assert_eq!(s.tap(25., 25.), Some(path(&[0])));
}

#[test]
fn move_snaps_to_grid() {
    let mut s = session();
    s.tap(60., 60.);
    assert_eq!(s.selection(), Some(&path(&[0])));
    s.move_selection(13., 6.).unwrap();
    assert!(
        s.doc().text().contains("x: 24px;\n        y: 16px;"),
        "{}",
        s.doc().text()
    );
}

#[test]
fn move_in_layout_is_refused() {
    let mut s = session();
    assert_eq!(s.tap(10., 210.), Some(path(&[1, 0])));
    let before = s.doc().text().to_string();
    assert!(matches!(
        s.move_selection(5., 5.),
        Err(SessionError::InLayout)
    ));
    assert_eq!(s.doc().text(), before);
}

#[test]
fn resize_top_left_moves_origin() {
    let mut s = session();
    s.tap(60., 60.);
    s.resize_selection(Corner::TopLeft, 8., 8.).unwrap();
    let t = s.doc().text();
    for want in ["x: 20px;", "y: 20px;", "width: 90px;", "height: 90px;"] {
        assert!(t.contains(want), "{want} in {t}");
    }
}

#[test]
fn compile_error_keeps_previous_instance() {
    let mut s = session();
    s.replace_text(format!(
        "{SRC}\nexport component Broken {{ Text {{ text: 1 +; }} }}\n"
    ));
    assert!(!s.rebuild_standalone(300., 300.));
    assert!(!s.diagnostics().is_empty());
    assert!(s.tap(60., 60.).is_some());
    s.move_selection(8., 8.).unwrap();
    assert!(s.doc().text().contains("x: 20px;"), "{}", s.doc().text());
}

#[test]
fn stale_offsets_after_top_edit_still_hit_right_element() {
    let mut s = session();
    s.replace_text(format!(
        "// header comment\n// more\nexport component Broken {{ Text {{ text: 1 +; }} }}\n{SRC}"
    ));
    assert!(!s.rebuild_standalone(300., 300.));
    assert_eq!(s.tap(25., 25.), Some(path(&[0, 0])));
    s.move_selection(8., 8.).unwrap();
    let t = s.doc().text();
    assert!(t.contains("x: 20px;"), "{t}");
    assert!(t.matches("x: 10px;").count() == 1, "{t}");
}

#[test]
fn length_changing_move_then_tap_and_move_again() {
    let mut s = session();
    s.tap(60., 60.);
    s.move_selection(13., 6.).unwrap(); // x: 10px -> 24px, no rebuild
    assert_eq!(s.tap(25., 25.), Some(path(&[0, 0])));
    s.move_selection(8., 8.).unwrap();
    let t = s.doc().text();
    assert!(t.contains("x: 24px;") && t.contains("x: 20px;"), "{t}");
}

#[test]
fn structural_op_while_stale_is_refused() {
    let mut s = session();
    s.replace_text(format!(
        "{SRC}\nexport component Broken {{ Text {{ text: 1 +; }} }}\n"
    ));
    assert!(!s.rebuild_standalone(300., 300.));
    let before = s.doc().text().to_string();
    let r = s.apply(slint_doc::Op::RemoveElement { path: path(&[0]) });
    assert!(matches!(r, Err(SessionError::Compile(_))));
    assert_eq!(s.doc().text(), before);
}

#[test]
fn tap_cycle_wraps_to_first() {
    let mut s = session();
    assert_eq!(s.tap(25., 25.), Some(path(&[0, 0])));
    assert_eq!(s.tap(25., 25.), Some(path(&[0])));
    assert_eq!(s.tap(25., 25.), Some(path(&[0, 0])));
}

#[test]
fn tap_on_empty_area_clears_selection() {
    let mut s = session();
    s.tap(25., 25.);
    assert_eq!(s.tap(900., 900.), None);
    assert_eq!(s.selection(), None);
}

#[test]
fn resize_layout_child_writes_only_size() {
    let mut s = session();
    s.tap(10., 210.);
    s.resize_selection(Corner::BottomRight, 8., 8.).unwrap();
    let t = s.doc().text();
    let line = t.lines().find(|l| l.contains("Text {")).unwrap();
    assert!(
        line.contains("width:") && line.contains("height:"),
        "{line}"
    );
    assert!(!line.contains(" x:") && !line.contains(" y:"), "{line}");
}

#[test]
fn root_move_is_invalid_target() {
    let mut s = session();
    s.apply(slint_doc::Op::Unwrap {
        layout: path(&[1]),
        positions: vec![(0, 200)],
    })
    .unwrap();
    assert_eq!(s.selection(), Some(&path(&[])));
    assert!(matches!(
        s.move_selection(4., 4.),
        Err(SessionError::Edit(slint_doc::EditError::InvalidTarget(_)))
    ));
}

#[test]
fn resize_keeps_four_px_minimum() {
    let mut s = session();
    s.tap(60., 60.);
    s.resize_selection(Corner::BottomRight, -200., -200.)
        .unwrap();
    let t = s.doc().text();
    assert!(
        t.contains("width: 4px;") && t.contains("height: 4px;"),
        "{t}"
    );
}

#[test]
fn rebuild_swaps_snapshot_only_when_the_factory_runs() {
    use slint::ComponentHandle;
    use std::cell::Cell;
    use std::rc::Rc;

    let mut s = session();
    s.tap(60., 60.);
    let before = s.selection_hit().unwrap().local;
    let created = Rc::new(Cell::new(0));
    let c = created.clone();
    s.set_on_instance(move || c.set(c.get() + 1));

    s.move_selection(40., 40.).unwrap();
    let factory = s.rebuild().expect("compiles");
    // The container has not run the factory yet: the old instance and the text it was
    // built from are still what geometry queries see.
    assert_eq!(s.selection_hit().unwrap().local, before);
    assert_eq!(created.get(), 0);

    let editor = app::Editor::new().unwrap();
    editor.set_factory(factory);
    editor.window().set_size(slint::LogicalSize::new(480., 800.));
    editor.show().unwrap();
    slint::platform::update_timers_and_animations();
    // Force layout evaluation of the container.
    let _ = editor.window().size();

    assert_eq!(created.get(), 1, "the factory ran once");
    let after = s.selection_hit().unwrap().local;
    // 10 + 40 snapped to the 4px grid.
    assert_eq!((after.x, after.y), (52., 52.));
}
