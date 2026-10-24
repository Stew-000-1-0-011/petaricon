//! "Make this a parameter": the session rules around `Op::Parameterize`.

use std::path::PathBuf;

use app::session::{Session, SessionError};
use slint_doc::{Document, ElementPath};

const SRC: &str = r#"export component Main inherits Rectangle {
    width: 300px;
    height: 300px;
    Text { x: 0px; y: 0px; text: "Hello"; }
    title := Text { x: 0px; y: 50px; text: "Hi"; color: red; }
    Text { x: 0px; y: 100px; width: parent.width; text: self.x > 0px ? "a" : "b"; }
    Rectangle { x: 0px; y: 150px; width: 10px; height: 10px; clip: true; }
}
"#;

fn path(i: &[usize]) -> ElementPath {
    ElementPath { component: "Main".into(), indices: i.to_vec() }
}

fn init() {
    thread_local!(static INIT: () = i_slint_backend_testing::init_no_event_loop());
    INIT.with(|_| {});
}

fn session() -> Session {
    init();
    let mut s = Session::new(Document::new(PathBuf::from("/t/main.slint"), SRC.into()));
    assert!(s.rebuild_standalone(300., 300.));
    s
}

fn select(s: &mut Session, i: &[usize]) {
    s.select_path(path(i));
}

#[test]
fn text_becomes_a_parameter_in_one_step_and_undo_restores() {
    let mut s = session();
    select(&mut s, &[0]);
    assert_eq!(s.can_parameterize("text"), Ok(("text-text".to_string(), Some("string"))));
    s.parameterize("text", "text-text", None).unwrap();
    let t = s.doc().text().to_string();
    let want = SRC
        .replace("text: \"Hello\";", "text: root.text-text;")
        .replace(
            "    width: 300px;\n",
            "    in property <string> text-text: \"Hello\";\n    width: 300px;\n",
        );
    assert_eq!(t, want);
    assert!(s.rebuild_standalone(300., 300.), "{:?}", s.diagnostics());
    assert!(s.undo());
    assert_eq!(s.doc().text(), SRC);
}

#[test]
fn default_name_uses_id_and_the_property() {
    let mut s = session();
    select(&mut s, &[1]);
    assert_eq!(s.can_parameterize("text").unwrap().0, "title-text");
    assert_eq!(s.can_parameterize("color").unwrap(), ("title-color".to_string(), Some("brush")));
    s.parameterize("text", "title-text", None).unwrap();
    // Already a parameter.
    assert!(s.can_parameterize("text").is_err());
    // The name is taken now.
    select(&mut s, &[0]);
    assert!(s.parameterize("text", "title-text", None).is_err());
}

#[test]
fn suffix_when_the_default_is_taken() {
    let src = SRC.replace("width: 300px;\n", "width: 300px;\n    in property <string> text-text;\n");
    init();
    let mut s = Session::new(Document::new(PathBuf::from("/t/main.slint"), src));
    assert!(s.rebuild_standalone(300., 300.));
    select(&mut s, &[0]);
    assert_eq!(s.can_parameterize("text").unwrap().0, "text-text-2");
}

#[test]
fn refusals_say_why_and_change_nothing() {
    let mut s = session();
    select(&mut s, &[0]);
    // No value.
    let e = s.can_parameterize("opacity").unwrap_err();
    assert!(e.contains("値"), "{e}");
    assert!(s.parameterize("opacity", "o", None).is_err());
    // self / parent.
    select(&mut s, &[2]);
    assert!(s.can_parameterize("width").unwrap_err().contains("self"));
    assert!(s.can_parameterize("text").unwrap_err().contains("self"));
    // Unknown type.
    select(&mut s, &[3]);
    assert_eq!(s.can_parameterize("clip").unwrap().1, None);
    let r = s.parameterize("clip", "r", None);
    assert!(matches!(r, Err(SessionError::Rejected(_))), "{r:?}");
    // ... chosen by the user it works (int).
    s.parameterize("clip", "r", Some("bool")).unwrap();
    assert!(s.doc().text().contains("in property <bool> r: true;"));
    // Bad type / name.
    select(&mut s, &[0]);
    assert!(s.parameterize("text", "x", Some("nope")).is_err());
    assert!(s.parameterize("text", "a b", None).is_err());
    // No selection.
    let mut n = session();
    assert!(n.can_parameterize("text").is_err());
    assert!(n.parameterize("text", "t", None).is_err());
    // Nothing above (but the one success) touched the text.
    assert_eq!(n.doc().text(), SRC);
}

#[test]
fn a_result_that_does_not_compile_is_refused() {
    let mut s = session();
    select(&mut s, &[0]);
    // `text` declared as an int cannot hold "Hello".
    let before = s.doc().text().to_string();
    let r = s.parameterize("text", "t", Some("int"));
    assert!(matches!(r, Err(SessionError::Rejected(_))), "{r:?}");
    assert_eq!(s.doc().text(), before);
    assert!(!s.undo());
}
