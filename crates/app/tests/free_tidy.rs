//! Free-placement Tidy in the session: apply, undo, refusals, tolerance, summary.

use std::path::PathBuf;

use app::session::{Session, SessionError};
use slint_doc::{Document, ElementPath};

fn path(i: &[usize]) -> ElementPath {
    ElementPath { component: "Main".into(), indices: i.to_vec() }
}

fn session_with(body: &str) -> Session {
    thread_local!(static INIT: () = i_slint_backend_testing::init_no_event_loop());
    INIT.with(|_| {});
    let src = format!(
        "export component Main inherits Rectangle {{\n    width: 300px;\n    height: 300px;\n    box := Rectangle {{\n        x: 0px;\n        y: 0px;\n        width: 300px;\n        height: 300px;\n{body}    }}\n}}\n"
    );
    let mut s = Session::new(Document::new(PathBuf::from("/t/main.slint"), src));
    assert!(s.rebuild_standalone(300., 300.));
    s.select_path(path(&[0]));
    s
}

fn binding(s: &Session, child: usize, name: &str) -> Option<String> {
    let info = s.doc().element(&path(&[0, child]))?;
    info.bindings.iter().find(|b| b.name == name).map(|b| b.expr.clone())
}

fn rect(x: i32, y: i32, w: i32, h: i32) -> String {
    format!("        Rectangle {{ x: {x}px; y: {y}px; width: {w}px; height: {h}px; }}\n")
}

/// An L: a column on the left (slightly ragged) and a row along the bottom.
fn l_shape() -> String {
    [rect(10, 10, 40, 30), rect(13, 60, 40, 30), rect(11, 110, 40, 30), rect(12, 200, 40, 30)]
        .concat()
        + &rect(80, 203, 40, 30)
        + &rect(150, 198, 40, 30)
}

#[test]
fn l_shape_stays_an_l_and_one_undo_restores_the_text() {
    let mut s = session_with(&l_shape());
    let before = s.doc().text().to_string();
    let r = s.tidy_free_selection(16.0).unwrap();
    assert!(r.changed > 0);
    let t = s.doc().text().to_string();
    assert!(!t.contains("Layout"), "{t}");
    let xs: Vec<_> = (0..4).map(|i| binding(&s, i, "x").unwrap()).collect();
    assert!(xs.iter().all(|x| *x == xs[0]), "{xs:?}");
    // the bottom row is aligned on y but is still to the right of the column
    let ys: Vec<_> = (3..6).map(|i| binding(&s, i, "y").unwrap()).collect();
    assert!(ys.iter().all(|y| *y == ys[0]), "{ys:?}");
    assert_ne!(binding(&s, 4, "x"), binding(&s, 3, "x"));
    assert_eq!(s.selection(), Some(&path(&[0])));
    assert!(s.undo());
    assert_eq!(s.doc().text(), before);
    assert!(!s.undo() || s.doc().text() != before, "exactly one step");
}

#[test]
fn undo_is_a_single_step() {
    let mut s = session_with(&l_shape());
    let before = s.doc().text().to_string();
    s.tidy_free_selection(16.0).unwrap();
    assert!(s.undo());
    assert_eq!(s.doc().text(), before);
    assert!(s.redo());
    assert_ne!(s.doc().text(), before);
}

#[test]
fn only_explicit_sizes_change() {
    let body = format!(
        "{}{}{}",
        "        Rectangle { x: 10px; y: 10px; width: 100px; height: 30px; }\n",
        "        Rectangle { x: 10px; y: 60px; width: 104px; height: 30px; }\n",
        "        Text { x: 150px; y: 10px; text: \"a\"; }\n        Text { x: 150px; y: 60px; text: \"abcdef\"; }\n",
    );
    let mut s = session_with(&body);
    s.tidy_free_selection(16.0).unwrap();
    assert_eq!(binding(&s, 0, "width"), binding(&s, 1, "width"));
    assert_eq!(binding(&s, 0, "width").as_deref(), Some("102px"));
    for i in [2, 3] {
        assert_eq!(binding(&s, i, "width"), None);
        assert_eq!(binding(&s, i, "height"), None);
    }
    assert!(s.doc().text().contains("text: \"abcdef\""));
}

#[test]
fn untouched_children_keep_their_text() {
    let body = format!("{}{}", rect(10, 10, 40, 30), rect(10, 100, 40, 30));
    let mut s = session_with(&body);
    let before = s.doc().text().to_string();
    let r = s.tidy_free_selection(16.0).unwrap();
    assert_eq!(r.changed, 0);
    assert_eq!(s.doc().text(), before);
    assert!(!s.undo(), "no history step for a no-op");
}

fn refused(mut s: Session) -> SessionError {
    let before = s.doc().text().to_string();
    let e = s.tidy_free_selection(16.0).unwrap_err();
    assert_eq!(s.doc().text(), before);
    e
}

#[test]
fn refuses_a_layout() {
    let mut s = session_with("        VerticalLayout { Rectangle { width: 10px; height: 10px; } }\n");
    s.select_path(path(&[0, 0]));
    let e = refused(s);
    assert!(matches!(e, SessionError::TidyOnLayout), "{e:?}");
    assert!(e.to_string().contains("Unstack"));
}

#[test]
fn refuses_a_leaf_and_an_empty_container() {
    let mut s = session_with("        Text { text: \"x\"; }\n");
    s.select_path(path(&[0, 0]));
    assert!(matches!(refused(s), SessionError::NotAContainer));
    let s = session_with("");
    assert!(matches!(refused(s), SessionError::NoChildren));
}

#[test]
fn refuses_for_and_if_children() {
    for child in [
        "        for i in 3: Rectangle { width: 10px; height: 10px; }\n",
        "        if true: Rectangle { width: 10px; height: 10px; }\n",
    ] {
        let s = session_with(&format!("{}{child}", rect(10, 10, 40, 30)));
        let e = refused(s);
        assert!(
            matches!(&e, SessionError::Edit(slint_doc::EditError::InvalidTarget(m)) if m.contains("for / if")),
            "{e:?}"
        );
    }
}

#[test]
fn refuses_while_the_preview_is_stale() {
    let mut s = session_with(&l_shape());
    let t = s.doc().text().replace("height: 300px;\n        Rectangle", "height: 300px;\n        foo: 1;\n        Rectangle");
    s.replace_text(t);
    assert!(!s.rebuild_standalone(300., 300.));
    s.select_path(path(&[0]));
    assert!(matches!(refused(s), SessionError::Compile(_)));
}

#[test]
fn refuses_without_selection() {
    let mut s = session_with(&l_shape());
    s.set_component("Main");
    assert!(matches!(s.tidy_free_selection(16.0), Err(SessionError::NoSelection)));
}

#[test]
fn tolerance_decides_what_is_aligned() {
    let body = format!("{}{}", rect(10, 10, 40, 30), rect(22, 100, 40, 30));
    let mut loose = session_with(&body);
    let r = loose.tidy_free_selection(16.0).unwrap();
    assert!(r.changed > 0);
    assert_eq!(binding(&loose, 0, "x"), binding(&loose, 1, "x"));

    let mut tight = session_with(&body);
    let before = tight.doc().text().to_string();
    let r = tight.tidy_free_selection(4.0).unwrap();
    assert_eq!(r.changed, 0);
    assert_eq!(tight.doc().text(), before);
}

#[test]
fn summary_lists_kinds_and_no_change_message() {
    let mut s = session_with(&l_shape());
    let r = s.tidy_free_selection(16.0).unwrap();
    let text = r.summary();
    assert!(text.starts_with("そろえ "), "{text}");
    assert!(text.contains('・') || text.matches(char::is_numeric).count() >= 1);
    assert!(!text.contains(" 0"), "zero kinds are omitted: {text}");

    let mut s = session_with(&format!("{}{}", rect(10, 10, 40, 30), rect(10, 100, 40, 30)));
    let r = s.tidy_free_selection(16.0).unwrap();
    assert_eq!(r.summary(), "整える箇所はありませんでした");
}

#[test]
fn report_maps_members_to_child_paths() {
    let mut s = session_with(&l_shape());
    let r = s.tidy_free_selection(16.0).unwrap();
    assert_eq!(r.container, path(&[0]));
    assert_eq!(r.children.len(), 6);
    assert_eq!(r.children[2], path(&[0, 2]));
}

#[test]
fn text_already_holding_the_rendered_value_is_not_a_change() {
    // Geometry says 10.4 (fractional literal), the tidy result renders as 10px = the written text.
    let body = "        Rectangle { x: 10px; y: 10px; width: 40px; height: 30px; }\n        Rectangle { x: 10px; y: 100px; width: 40px; height: 30px; }\n";
    let mut s = session_with(body);
    let before = s.doc().text().to_string();
    let r = s.tidy_free_selection(16.0).unwrap();
    assert_eq!((r.changed, s.doc().text()), (0, before.as_str()));
    assert!(!s.undo());
}

#[test]
fn changed_counts_only_text_changes() {
    let mut s = session_with(&l_shape());
    let before = s.doc().text().to_string();
    let r = s.tidy_free_selection(16.0).unwrap();
    let after = s.doc().text().to_string();
    let differing = before.lines().zip(after.lines()).filter(|(a, b)| a != b).count();
    assert_eq!(r.changed, differing);
}

#[test]
fn refuses_when_the_preview_is_not_built_from_the_current_text() {
    let mut s = session_with(&l_shape());
    s.set_property("width", "301").unwrap(); // edit without rebuilding
    let before = s.doc().text().to_string();
    let e = s.tidy_free_selection(16.0).unwrap_err();
    assert!(matches!(e, SessionError::PreviewUpdating), "{e:?}");
    assert!(e.to_string().contains("プレビューを更新中"));
    assert_eq!(s.doc().text(), before);
}
