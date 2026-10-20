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
