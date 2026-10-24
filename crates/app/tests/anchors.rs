//! Anchors in the session: reading, switching, keeping them through every geometry edit, and
//! following the container (checked on real preview geometry).

use std::path::PathBuf;

use app::anchor::{Anchor, Axis, Kind};
use app::session::{Corner, Session, SessionError};
use slint_doc::{Document, ElementPath};

const EXPR_MSG: &str = "式で書かれているため動かせません(ソースで直せます)";

fn path(i: &[usize]) -> ElementPath {
    ElementPath { component: "Main".into(), indices: i.to_vec() }
}

fn source(kid: &str) -> String {
    format!(
        "export component Main inherits Rectangle {{\n    width: 400px;\n    height: 300px;\n    box := Rectangle {{\n        x: 20px;\n        y: 20px;\n        width: 200px;\n        height: 100px;\n        kid := Rectangle {{\n{kid}        }}\n    }}\n}}\n"
    )
}

const KID: &str = "            x: 40px;\n            y: 16px;\n            width: 60px;\n            height: 30px;\n";

fn session_of(src: String) -> Session {
    thread_local!(static INIT: () = i_slint_backend_testing::init_no_event_loop());
    INIT.with(|_| {});
    let mut s = Session::new(Document::new(PathBuf::from("/t/main.slint"), src));
    assert!(s.rebuild_standalone(400., 300.), "{:?}", s.diagnostics());
    s.select_path(path(&[0, 0]));
    s
}

fn session() -> Session {
    session_of(source(KID))
}

fn rebuild(s: &mut Session) {
    assert!(s.rebuild_standalone(400., 300.), "{:?}", s.diagnostics());
}

fn local(s: &Session) -> (f32, f32, f32, f32) {
    let r = s.selection_hit().unwrap().local;
    (r.x, r.y, r.w, r.h)
}

fn binding(s: &Session, name: &str) -> String {
    let info = s.doc().element(s.selection().unwrap()).unwrap();
    info.bindings.iter().find(|b| b.name == name).map(|b| b.expr.clone()).unwrap_or_default()
}

/// Sets the container's width/height, rebuilds, and selects the kid again.
fn resize_box(s: &mut Session, name: &str, value: &str) {
    s.select_path(path(&[0]));
    s.set_property(name, value).unwrap();
    rebuild(s);
    s.select_path(path(&[0, 0]));
}

#[test]
fn reads_what_is_written() {
    let s = session();
    assert_eq!(s.anchors(), Some((Anchor::Start(40.0), Anchor::Start(16.0))));
    assert!(s.can_set_anchor());
    let s = session_of(source(
        "            x: parent.width - self.width - 8px;\n            y: (parent.height - self.height) / 2;\n            width: 60px;\n            height: 30px;\n",
    ));
    assert_eq!(s.anchors(), Some((Anchor::End(8.0), Anchor::Center(0.0))));
}

#[test]
fn no_anchors_for_the_root_or_nothing() {
    let mut s = session();
    s.select_path(path(&[]));
    assert_eq!(s.anchors(), None);
    assert!(!s.can_set_anchor());
}

#[test]
fn switching_keeps_the_rectangle_on_both_axes() {
    for h in [Kind::Start, Kind::End, Kind::Stretch, Kind::Center] {
        for v in [Kind::Start, Kind::End, Kind::Stretch, Kind::Center] {
            let mut s = session();
            let before = local(&s);
            s.set_anchor(Axis::H, h).unwrap();
            s.set_anchor(Axis::V, v).unwrap();
            rebuild(&mut s);
            assert_eq!(local(&s), before, "{h:?} {v:?}\n{}", s.doc().text());
            let (ah, av) = s.anchors().unwrap();
            assert_eq!((ah.kind(), av.kind()), (Some(h), Some(v)));
        }
    }
}

#[test]
fn switching_back_to_start_writes_plain_lengths() {
    let mut s = session();
    s.set_anchor(Axis::H, Kind::Stretch).unwrap();
    assert_eq!(binding(&s, "width"), "parent.width - 40px - 100px");
    s.set_anchor(Axis::H, Kind::Start).unwrap();
    assert_eq!(binding(&s, "x"), "40px");
    assert_eq!(binding(&s, "width"), "60px");
    s.set_anchor(Axis::V, Kind::Stretch).unwrap();
    s.set_anchor(Axis::V, Kind::End).unwrap();
    assert_eq!(binding(&s, "height"), "30px");
    assert_eq!(binding(&s, "y"), "parent.height - self.height - 54px");
}

#[test]
fn a_switch_is_one_undo_step_and_touches_only_that_axis() {
    let mut s = session();
    let before = s.doc().text().to_string();
    s.set_anchor(Axis::H, Kind::End).unwrap();
    let t = s.doc().text().to_string();
    assert_eq!(binding(&s, "y"), "16px");
    assert_eq!(binding(&s, "height"), "30px");
    assert_eq!(binding(&s, "width"), "60px");
    assert!(t.contains("x: parent.width - self.width - 100px;"), "{t}");
    assert!(s.undo());
    assert_eq!(s.doc().text(), before);
}

#[test]
fn follows_the_container() {
    // (kind, expected (x, width) after the container grows 200 -> 300)
    for (kind, expect) in [
        (Kind::Start, (40.0, 60.0)),
        (Kind::End, (140.0, 60.0)),
        (Kind::Center, (90.0, 60.0)),
        (Kind::Stretch, (40.0, 160.0)),
    ] {
        let mut s = session();
        s.set_anchor(Axis::H, kind).unwrap();
        rebuild(&mut s);
        resize_box(&mut s, "width", "300px");
        let (x, _, w, _) = local(&s);
        assert_eq!((x, w), expect, "{kind:?}");
    }
    for (kind, expect) in [
        (Kind::End, (66.0, 30.0)),
        (Kind::Center, (41.0, 30.0)),
        (Kind::Stretch, (16.0, 80.0)),
    ] {
        let mut s = session();
        s.set_anchor(Axis::V, kind).unwrap();
        rebuild(&mut s);
        resize_box(&mut s, "height", "150px");
        let (_, y, _, h) = local(&s);
        assert_eq!((y, h), expect, "{kind:?}");
    }
}

#[test]
fn follows_the_screen_size_for_a_child_of_the_root() {
    let mut s = session();
    s.select_path(path(&[0]));
    s.set_anchor(Axis::H, Kind::End).unwrap(); // box: 20 + 200 inside 400 -> B = 180
    rebuild(&mut s);
    // the preview's screen is wider: the root follows it, and so does the box
    assert!(s.rebuild_standalone(500., 300.));
    s.select_path(path(&[0]));
    assert_eq!(local(&s).0, 120.0);
}

#[test]
fn dragging_changes_only_the_numbers() {
    let mut s = session();
    s.set_anchor(Axis::H, Kind::End).unwrap();
    s.set_anchor(Axis::V, Kind::Center).unwrap();
    s.move_selection(8., 8.).unwrap();
    assert_eq!(binding(&s, "x"), "parent.width - self.width - 92px");
    assert_eq!(binding(&s, "y"), "(parent.height - self.height) / 2 - 11px");
    assert_eq!(binding(&s, "width"), "60px");
    rebuild(&mut s);
    let (x, y, ..) = local(&s);
    assert_eq!((x, y), (48.0, 24.0));
}

#[test]
fn dragging_a_stretch_moves_both_edges_and_keeps_the_size() {
    let mut s = session();
    s.set_anchor(Axis::H, Kind::Stretch).unwrap();
    s.move_selection(8., 0.).unwrap();
    assert_eq!(binding(&s, "x"), "48px");
    assert_eq!(binding(&s, "width"), "parent.width - 48px - 92px");
    rebuild(&mut s);
    assert_eq!(local(&s).2, 60.0);
}

#[test]
fn resizing_changes_the_edge_that_moved() {
    // End: the left handle changes the size and keeps B; the right handle changes B.
    let mut s = session();
    s.set_anchor(Axis::H, Kind::End).unwrap();
    s.resize_selection(Corner::TopLeft, -8., 0.).unwrap();
    assert_eq!(binding(&s, "x"), "parent.width - self.width - 100px");
    assert_eq!(binding(&s, "width"), "68px");
    rebuild(&mut s);
    s.resize_selection(Corner::BottomRight, 8., 8.).unwrap();
    assert_eq!(binding(&s, "x"), "parent.width - self.width - 92px");
    assert_eq!(binding(&s, "width"), "76px");
    rebuild(&mut s);
    assert_eq!((local(&s).0, local(&s).2), (32.0, 76.0));

    // Stretch: a for the left edge, b for the right edge.
    let mut s = session();
    s.set_anchor(Axis::H, Kind::Stretch).unwrap();
    s.resize_selection(Corner::TopLeft, -8., 0.).unwrap();
    assert_eq!(binding(&s, "x"), "32px");
    assert_eq!(binding(&s, "width"), "parent.width - 32px - 100px");
    rebuild(&mut s);
    s.resize_selection(Corner::BottomRight, 8., 0.).unwrap();
    assert_eq!(binding(&s, "width"), "parent.width - 32px - 92px");
    rebuild(&mut s);
    assert_eq!((local(&s).0, local(&s).2), (32.0, 76.0));

    // Center keeps C relative to the new size.
    let mut s = session();
    s.set_anchor(Axis::V, Kind::Center).unwrap();
    s.resize_selection(Corner::BottomRight, 0., 8.).unwrap();
    rebuild(&mut s);
    assert_eq!((local(&s).1, local(&s).3), (16.0, 40.0));
    assert!(binding(&s, "y").starts_with("(parent.height - self.height) / 2"));
}

#[test]
fn start_axes_are_written_as_before() {
    let mut s = session();
    s.move_selection(13., 6.).unwrap();
    assert_eq!(binding(&s, "x"), "52px");
    assert_eq!(binding(&s, "y"), "24px");
    s.resize_selection(Corner::BottomRight, 8., 8.).unwrap();
    assert_eq!(binding(&s, "width"), "68px");
    assert_eq!(binding(&s, "height"), "40px");
}

#[test]
fn an_expression_is_never_overwritten() {
    let src = source(
        "            x: box.width / 4;\n            y: 16px;\n            width: 60px;\n            height: 30px;\n",
    );
    let mut s = session_of(src);
    assert_eq!(s.anchors(), Some((Anchor::Expr, Anchor::Start(16.0))));
    let before = s.doc().text().to_string();
    for r in [
        s.move_selection(8., 8.),
        s.move_selection(8., 0.),
        s.resize_selection(Corner::BottomRight, 8., 8.),
        s.resize_selection(Corner::TopLeft, 0., 8.),
        s.set_anchor(Axis::H, Kind::Start),
        s.set_anchor(Axis::H, Kind::End),
        s.set_property("x", "5"),
    ] {
        match r {
            Err(e @ SessionError::Anchor(_)) => assert_eq!(e.to_string(), EXPR_MSG),
            other => panic!("{other:?}"),
        }
    }
    assert_eq!(s.doc().text(), before);
    // The other axis still moves, as does a drag along it.
    s.move_selection(0., 8.).unwrap();
    assert_eq!(binding(&s, "x"), "box.width / 4");
    assert_eq!(binding(&s, "y"), "24px");
    s.set_anchor(Axis::V, Kind::End).unwrap();
    assert_eq!(binding(&s, "x"), "box.width / 4");
    // Clearing a binding is the user's own decision.
    s.set_property("x", "").unwrap();
}

#[test]
fn typed_numbers_keep_the_anchor() {
    let mut s = session();
    s.set_anchor(Axis::H, Kind::End).unwrap();
    s.set_property("x", "30").unwrap(); // the left edge at 30
    assert_eq!(binding(&s, "x"), "parent.width - self.width - 110px");
    s.set_property("width", "80").unwrap(); // B stays
    assert_eq!(binding(&s, "x"), "parent.width - self.width - 110px");
    assert_eq!(binding(&s, "width"), "80px");
    rebuild(&mut s);
    assert_eq!((local(&s).0, local(&s).2), (10.0, 80.0));

    let mut s = session();
    s.set_anchor(Axis::V, Kind::Stretch).unwrap();
    s.set_property("y", "20").unwrap();
    assert_eq!(binding(&s, "y"), "20px");
    assert_eq!(binding(&s, "height"), "parent.height - 20px - 50px");
    assert!(matches!(s.set_property("height", "40"), Err(SessionError::Anchor(_))));
    // An expression typed on purpose is the user's own text.
    s.set_property("y", "box.height / 8").unwrap();
    assert_eq!(binding(&s, "y"), "box.height / 8");
}

#[test]
fn a_layout_child_has_no_anchors_to_set() {
    let mut s = session_of(
        "export component Main inherits Rectangle {\n    width: 300px;\n    height: 300px;\n    VerticalLayout {\n        Rectangle { height: 30px; }\n    }\n}\n".into(),
    );
    s.select_path(path(&[0, 0]));
    assert!(!s.can_set_anchor());
    assert!(matches!(s.set_anchor(Axis::H, Kind::End), Err(SessionError::InLayout)));
    assert!(matches!(s.anchors(), Some((Anchor::Start(_), Anchor::Start(_)))));
}

#[test]
fn tidy_keeps_the_anchor_and_changes_its_numbers() {
    let body = "        Rectangle { x: 10px; y: 10px; width: 100px; height: 30px; }\n        Rectangle { x: parent.width - self.width - 192px; y: 60px; width: 100px; height: 30px; }\n";
    let src = format!(
        "export component Main inherits Rectangle {{\n    width: 300px;\n    height: 300px;\n    box := Rectangle {{\n        x: 0px;\n        y: 0px;\n        width: 300px;\n        height: 300px;\n{body}    }}\n}}\n"
    );
    let mut s = session_of(src);
    s.select_path(path(&[0]));
    let r = s.tidy_free_selection(16.0).unwrap();
    assert!(r.changed > 0);
    s.select_path(path(&[0, 1]));
    assert!(binding(&s, "x").starts_with("parent.width - self.width - "), "{}", binding(&s, "x"));
    rebuild(&mut s);
    let x = local(&s).0;
    s.select_path(path(&[0, 0]));
    assert_eq!(local(&s).0, x, "aligned on the left edge");
}

#[test]
fn tidy_refuses_to_move_an_expression() {
    let body = "        Rectangle { x: 10px; y: 10px; width: 100px; height: 30px; }\n        Rectangle { x: box.width / 25 - 4px; y: 60px; width: 100px; height: 30px; }\n";
    let src = format!(
        "export component Main inherits Rectangle {{\n    width: 300px;\n    height: 300px;\n    box := Rectangle {{\n        x: 0px;\n        y: 0px;\n        width: 300px;\n        height: 300px;\n{body}    }}\n}}\n"
    );
    let mut s = session_of(src);
    s.select_path(path(&[0]));
    let before = s.doc().text().to_string();
    // x = 8 is within tolerance of 10, so tidy wants to move the expression
    assert!(matches!(s.tidy_free_selection(16.0), Err(SessionError::Anchor(_))));
    assert_eq!(s.doc().text(), before);
}

#[test]
fn stacking_and_unstacking_refuse_to_drop_an_anchor() {
    let mut s = session();
    s.set_anchor(Axis::H, Kind::End).unwrap();
    rebuild(&mut s);
    s.select_path(path(&[0]));
    let before = s.doc().text().to_string();
    assert!(matches!(s.stack_selection(), Err(SessionError::Anchor(_))));
    assert_eq!(s.doc().text(), before);
}

#[test]
fn unstacking_refuses_a_child_with_an_anchor() {
    let mut s = session_of(
        "export component Main inherits Rectangle {\n    width: 300px;\n    height: 300px;\n    VerticalLayout {\n        Rectangle { x: parent.width - self.width - 5px; height: 30px; }\n    }\n}\n".into(),
    );
    s.select_path(path(&[0]));
    let before = s.doc().text().to_string();
    assert!(matches!(s.unstack_selection(), Err(SessionError::Anchor(_))));
    assert_eq!(s.doc().text(), before);
}
