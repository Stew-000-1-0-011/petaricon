//! Anchors in the session: reading, switching, keeping them through every geometry edit, and
//! following the container (checked on real preview geometry).

use std::path::PathBuf;

use app::anchor::{Anchor, Axis, Kind};
use app::session::{Corner, Session, SessionError};
use slint_doc::{Document, ElementPath};

const EXPR_MSG: &str = "式で書かれているため動かせません(付き方を選ぶと動かせます)";

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

fn expr_x_session() -> Session {
    session_of(source(
        "            x: box.width / 4;\n            y: 16px;\n            width: 60px;\n            height: 30px;\n",
    ))
}

#[test]
fn an_expression_axis_stays_while_the_other_axis_moves() {
    let mut s = expr_x_session();
    assert_eq!(s.anchors(), Some((Anchor::Expr, Anchor::Start(16.0))));
    s.move_selection(8., 8.).unwrap();
    assert_eq!(binding(&s, "x"), "box.width / 4");
    assert_eq!(binding(&s, "y"), "24px");
    let notice = s.take_notice().expect("a notice");
    assert_eq!(notice, "横 は式で書かれているため動かしません(付き方を選ぶと動かせます)");
    assert_eq!(s.take_notice(), None);
    // dragging only along the plain axis says nothing
    rebuild(&mut s);
    s.move_selection(0., 8.).unwrap();
    assert_eq!(s.take_notice(), None);
    assert_eq!(binding(&s, "y"), "32px");
    // dragging only along the expression axis changes nothing but still says so
    rebuild(&mut s);
    let before = s.doc().text().to_string();
    s.move_selection(8., 0.).unwrap();
    assert_eq!(s.doc().text(), before);
    assert!(s.take_notice().is_some());
}

#[test]
fn both_axes_as_expressions_refuse_a_move() {
    let mut s = session_of(source(
        "            x: box.width / 4;\n            y: box.height / 4;\n            width: 60px;\n            height: 30px;\n",
    ));
    let before = s.doc().text().to_string();
    match s.move_selection(8., 8.) {
        Err(e @ SessionError::Anchor(_)) => assert_eq!(e.to_string(), EXPR_MSG),
        other => panic!("{other:?}"),
    }
    assert_eq!(s.doc().text(), before);
}

#[test]
fn resizing_an_expression_edge_is_refused_and_a_switch_replaces_it() {
    let mut s = expr_x_session();
    let before = s.doc().text().to_string();
    for r in [
        s.resize_selection(Corner::BottomRight, 8., 8.),
        s.resize_selection(Corner::TopLeft, 0., 8.),
        s.set_property("x", "5"),
    ] {
        match r {
            Err(e @ SessionError::Anchor(_)) => assert_eq!(e.to_string(), EXPR_MSG),
            other => panic!("{other:?}"),
        }
    }
    assert_eq!(s.doc().text(), before);
    // Choosing an anchor is the way out: the rectangle stays.
    let rect = local(&s);
    s.set_anchor(Axis::H, Kind::End).unwrap();
    rebuild(&mut s);
    assert_eq!(local(&s), rect);
    assert_eq!(s.anchors().unwrap().0.kind(), Some(Kind::End));
    // and clearing a binding stays the user's own decision
    let mut s = expr_x_session();
    s.set_property("x", "").unwrap();
}

#[test]
fn leaving_an_expression_with_an_expression_size_writes_a_length() {
    let mut s = session_of(source(
        "            x: 40px;\n            y: 16px;\n            width: parent.width * 0.3;\n            height: 30px;\n",
    ));
    assert_eq!(s.anchors().unwrap().0, Anchor::Expr);
    let rect = local(&s);
    s.set_anchor(Axis::H, Kind::Center).unwrap();
    assert_eq!(binding(&s, "width"), "60px");
    rebuild(&mut s);
    assert_eq!(local(&s), rect);
}

#[test]
fn a_typed_length_next_to_an_expression_is_written_as_always() {
    let mut s = session_of(source(
        "            x: box.width / 4;\n            y: 16px;\n            width: 100px;\n            height: 30px;\n",
    ));
    s.set_property("width", "120").unwrap();
    assert_eq!(binding(&s, "width"), "120px");
    assert_eq!(binding(&s, "x"), "box.width / 4");
    // but not over the expression itself
    let mut s = session_of(source(
        "            x: 40px;\n            y: 16px;\n            width: parent.width * 0.3;\n            height: 30px;\n",
    ));
    assert!(matches!(s.set_property("width", "120"), Err(SessionError::Anchor(_))));
    s.set_property("x", "50").unwrap();
    assert_eq!(binding(&s, "x"), "50px");
}

#[test]
fn centre_numbers_do_not_drift() {
    // 100 - 31 is odd: the preview puts a centred kid on a half pixel.
    let src = source(
        "            x: 40px;\n            y: 16px;\n            width: 60px;\n            height: 31px;\n",
    );
    let mut s = session_of(src);
    s.grid = 1;
    s.set_anchor(Axis::H, Kind::Center).unwrap();
    s.set_anchor(Axis::V, Kind::Center).unwrap();
    rebuild(&mut s);
    let (h0, v0) = s.anchors().unwrap();
    let (Anchor::Center(ch), Anchor::Center(cv)) = (h0, v0) else { panic!("{h0:?} {v0:?}") };
    let x_text = binding(&s, "x");
    // vertical drags leave the horizontal numbers (and text) alone
    for i in 1..=3 {
        s.move_selection(0., 3.).unwrap();
        rebuild(&mut s);
        assert_eq!(binding(&s, "x"), x_text);
        assert_eq!(s.anchors().unwrap().1, Anchor::Center(cv + 3.0 * f64::from(i)));
    }
    // horizontal +1 / -1 returns to the same number, however often
    let y_text = binding(&s, "y");
    for _ in 0..3 {
        s.move_selection(1., 0.).unwrap();
        rebuild(&mut s);
        assert_eq!(s.anchors().unwrap().0, Anchor::Center(ch + 1.0));
        s.move_selection(-1., 0.).unwrap();
        rebuild(&mut s);
        assert_eq!(s.anchors().unwrap().0, Anchor::Center(ch));
        assert_eq!(binding(&s, "y"), y_text);
    }
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
fn tidy_leaves_an_expression_axis_and_says_so() {
    let body = "        Rectangle { x: 10px; y: 10px; width: 100px; height: 30px; }\n        Rectangle { x: box.width / 25 - 4px; y: 60px; width: 100px; height: 30px; }\n";
    let src = format!(
        "export component Main inherits Rectangle {{\n    width: 300px;\n    height: 300px;\n    box := Rectangle {{\n        x: 0px;\n        y: 0px;\n        width: 300px;\n        height: 300px;\n{body}    }}\n}}\n"
    );
    let mut s = session_of(src);
    s.select_path(path(&[0]));
    let before = s.doc().text().to_string();
    // x = 8 is within tolerance of 10, so tidy wants to move the expression: it stays
    let r = s.tidy_free_selection(16.0).unwrap();
    assert!(r.notice.as_deref().is_some_and(|n| n.contains("Rectangle #2")), "{:?}", r.notice);
    assert!(s.doc().text().contains("x: box.width / 25 - 4px;"));
    let _ = before;
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

#[test]
fn unstacking_refuses_to_write_px_over_an_expression() {
    let mut s = session_of(
        "export component Main inherits Rectangle {\n    width: 300px;\n    height: 300px;\n    VerticalLayout {\n        Rectangle { x: root.width / 2; height: 30px; }\n    }\n}\n".into(),
    );
    s.select_path(path(&[0]));
    let before = s.doc().text().to_string();
    assert!(matches!(s.unstack_selection(), Err(SessionError::Anchor(_))));
    assert_eq!(s.doc().text(), before);
}
