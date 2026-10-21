//! Geometry of the Tidy overlay marks.

use app::tidy_overlay::{MarkKind, marks};
use tidy::Rect;
use tidy::free::{Anchor, Axis, Relation};

fn r(x: f32, y: f32, w: f32, h: f32) -> Rect {
    Rect { x, y, w, h }
}

#[test]
fn align_is_a_dashed_line_through_the_shared_coordinate_spanning_the_members() {
    let rects = [r(110., 50., 40., 30.), r(110., 120., 40., 30.)];
    let rel = Relation::Align { axis: Axis::X, anchor: Anchor::Start, members: vec![0, 1], coord: 10.0 };
    let m = marks(&[rel], (100., 40.), &rects);
    assert_eq!(m.len(), 1);
    assert_eq!((m[0].kind, m[0].x, m[0].y, m[0].h), (MarkKind::VLine, 110., 50., 100.));
    let rel = Relation::Align { axis: Axis::Y, anchor: Anchor::Start, members: vec![0, 1], coord: 20.0 };
    let rects = [r(110., 60., 40., 30.), r(200., 60., 40., 30.)];
    let m = marks(&[rel], (100., 40.), &rects);
    assert_eq!((m[0].kind, m[0].x, m[0].y, m[0].w), (MarkKind::HLine, 110., 60., 130.));
}

#[test]
fn equal_gap_puts_one_identical_marker_in_each_gap() {
    let rects = [r(0., 0., 10., 10.), r(30., 0., 10., 10.), r(60., 0., 10., 10.)];
    let rel = Relation::EqualGap { axis: Axis::X, members: vec![0, 1, 2], gap: 20. };
    let m = marks(&[rel], (0., 0.), &rects);
    assert_eq!(m.len(), 2);
    assert!(m.iter().all(|k| k.kind == MarkKind::Marker && (k.w, k.h) == (m[0].w, m[0].h)));
    assert_eq!((m[0].x, m[0].y), (20., 5.));
    assert_eq!((m[1].x, m[1].y), (50., 5.));
}

#[test]
fn symmetric_draws_the_axis_and_repeat_connects_top_left_corners() {
    let rects = [r(10., 10., 40., 20.), r(110., 10., 40., 20.), r(60., 80., 40., 20.)];
    let sym = Relation::Symmetric { axis: Axis::X, center: 80., pairs: vec![(0, 1)], singles: vec![2] };
    let m = marks(&[sym], (0., 0.), &rects);
    assert_eq!((m[0].kind, m[0].x, m[0].y, m[0].h), (MarkKind::VLine, 80., 10., 90.));

    let rep = Relation::Repeat { dx: 100., dy: 0., pairs: vec![(0, 1)] };
    let m = marks(&[rep], (0., 0.), &rects);
    assert!(m.iter().all(|k| k.kind == MarkKind::Dot));
    assert_eq!((m[0].x, m[0].y), (10., 10.));
    let last = m.last().unwrap();
    assert_eq!((last.x, last.y), (110., 10.));
}

#[test]
fn same_size_marks_each_member() {
    let rects = [r(0., 0., 10., 10.), r(30., 0., 10., 10.)];
    let rel = Relation::SameSize { axis: Axis::X, members: vec![0, 1], value: 10 };
    let m = marks(&[rel], (0., 0.), &rects);
    assert_eq!(m.len(), 2);
    assert!(m.iter().all(|k| k.kind == MarkKind::Marker));
}
