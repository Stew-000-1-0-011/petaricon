use tidy::Rect;
use tidy::free::{Anchor, Axis, Child, Outcome, Relation, tidy};

fn r(x: f32, y: f32, w: f32, h: f32) -> Rect {
    Rect { x, y, w, h }
}
fn c(x: f32, y: f32, w: f32, h: f32) -> Child {
    Child {
        rect: r(x, y, w, h),
        w_explicit: false,
        h_explicit: false,
    }
}
fn ce(x: f32, y: f32, w: f32, h: f32) -> Child {
    Child {
        rect: r(x, y, w, h),
        w_explicit: true,
        h_explicit: true,
    }
}
fn rects(o: &Outcome) -> Vec<(f32, f32, f32, f32)> {
    o.rects.iter().map(|r| (r.x, r.y, r.w, r.h)).collect()
}
fn has_align(o: &Outcome, axis: Axis, anchor: Anchor, members: &[usize], coord: f32) -> bool {
    o.relations.iter().any(|rel| {
        matches!(rel, Relation::Align { axis: a, anchor: an, members: m, coord: co }
            if *a == axis && *an == anchor && m == members && *co == coord)
    })
}

#[test]
fn l_shape_stays_an_l() {
    let kids = [
        c(10., 10., 40., 40.),
        c(62., 12., 40., 40.),
        c(11., 63., 40., 40.),
    ];
    let o = tidy(400., 300., &kids, 16.);
    assert_eq!(
        rects(&o),
        vec![
            (11., 11., 40., 40.),
            (62., 11., 40., 40.),
            (11., 63., 40., 40.)
        ]
    );
    assert_eq!(o.relations.len(), 2, "{:?}", o.relations);
    assert!(has_align(&o, Axis::Y, Anchor::Start, &[0, 1], 11.));
    assert!(has_align(&o, Axis::X, Anchor::Start, &[0, 2], 11.));
}

#[test]
fn near_grid_becomes_exact_grid() {
    let kids = [
        c(43., 38., 60., 60.),
        c(148., 42., 60., 60.),
        c(262., 36., 60., 60.),
        c(37., 143., 60., 60.),
        c(153., 139., 60., 60.),
        c(258., 145., 60., 60.),
    ];
    let o = tidy(400., 300., &kids, 16.);
    let rs = rects(&o);
    // rows aligned
    assert_eq!(rs[0].1, rs[1].1);
    assert_eq!(rs[1].1, rs[2].1);
    assert_eq!(rs[3].1, rs[4].1);
    assert_eq!(rs[4].1, rs[5].1);
    // columns aligned
    assert_eq!(rs[0].0, rs[3].0);
    assert_eq!(rs[1].0, rs[4].0);
    assert_eq!(rs[2].0, rs[5].0);
    // equal gaps
    assert_eq!(rs[1].0 - rs[0].0 - 60., rs[2].0 - rs[1].0 - 60.);
    // sizes untouched, nothing moved more than the tolerance
    for (k, kid) in kids.iter().enumerate() {
        assert_eq!((rs[k].2, rs[k].3), (60., 60.));
        assert!((rs[k].0 - kid.rect.x).abs() <= 16. && (rs[k].1 - kid.rect.y).abs() <= 16.);
    }
    assert!(
        o.relations
            .iter()
            .any(|x| matches!(x, Relation::EqualGap { axis: Axis::X, .. }))
    );
    assert!(
        o.relations
            .iter()
            .any(|x| matches!(x, Relation::Align { axis: Axis::X, .. }))
    );
    assert!(
        o.relations
            .iter()
            .any(|x| matches!(x, Relation::Align { axis: Axis::Y, .. }))
    );
    assert!(
        o.relations
            .iter()
            .any(|x| matches!(x, Relation::Repeat { .. }))
    );
    assert_eq!(
        rs,
        vec![
            (40., 39., 60., 60.),
            (150., 39., 60., 60.),
            (260., 39., 60., 60.),
            (40., 142., 60., 60.),
            (150., 142., 60., 60.),
            (260., 142., 60., 60.),
        ]
    );
}

#[test]
fn near_symmetric_pair_and_centered_single() {
    let kids = [
        c(40., 60., 60., 60.),
        c(298., 64., 60., 60.),
        c(172., 200., 50., 50.),
    ];
    let o = tidy(400., 300., &kids, 16.);
    assert_eq!(
        rects(&o),
        vec![
            (41., 62., 60., 60.),
            (299., 62., 60., 60.),
            (175., 200., 50., 50.)
        ]
    );
    assert!(o.relations.contains(&Relation::Symmetric {
        axis: Axis::X,
        center: 200.,
        pairs: vec![(0, 1)],
        singles: vec![2]
    }));
    assert!(has_align(&o, Axis::Y, Anchor::Start, &[0, 1], 62.));
}

#[test]
fn near_equal_spacing_in_a_row() {
    let kids = [
        c(20., 50., 40., 40.),
        c(100., 50., 40., 40.),
        c(179., 50., 40., 40.),
        c(257., 50., 40., 40.),
    ];
    let o = tidy(500., 200., &kids, 16.);
    assert_eq!(
        rects(&o),
        vec![
            (20., 50., 40., 40.),
            (99., 50., 40., 40.),
            (178., 50., 40., 40.),
            (257., 50., 40., 40.)
        ]
    );
    assert!(o.relations.contains(&Relation::EqualGap {
        axis: Axis::X,
        members: vec![0, 1, 2, 3],
        gap: 39.
    }));
}

#[test]
fn conflicting_relations_stay_finite_and_bounded() {
    let kids = [
        c(0., 0., 40., 40.),
        c(50., 10., 40., 40.),
        c(95., 2., 40., 40.),
        c(160., 8., 40., 40.),
        c(12., 30., 40., 40.),
        c(88., 44., 40., 40.),
    ];
    let a = tidy(300., 200., &kids, 16.);
    let b = tidy(300., 200., &kids, 16.);
    assert_eq!(rects(&a), rects(&b));
    for (k, rc) in a.rects.iter().enumerate() {
        for v in [rc.x, rc.y, rc.w, rc.h] {
            assert!(v.is_finite() && v.fract() == 0.);
        }
        assert!((rc.x - kids[k].rect.x).abs() <= 16., "{k}: {:?}", rc);
        assert!((rc.y - kids[k].rect.y).abs() <= 16., "{k}: {:?}", rc);
    }
}

#[test]
fn scattered_elements_do_not_move() {
    let kids = [
        c(10., 10., 40., 40.),
        c(150., 90., 80., 60.),
        c(300., 200., 120., 100.),
    ];
    let o = tidy(600., 700., &kids, 16.);
    assert_eq!(
        rects(&o),
        vec![
            (10., 10., 40., 40.),
            (150., 90., 80., 60.),
            (300., 200., 120., 100.)
        ]
    );
    assert!(o.relations.is_empty(), "{:?}", o.relations);
}

#[test]
fn unchanged_children_keep_fractional_values() {
    let kids = [c(10.25, 10.5, 40.5, 40.), c(150.75, 90., 80., 60.25)];
    let o = tidy(600., 700., &kids, 16.);
    assert_eq!(o.rects[0], kids[0].rect);
    assert_eq!(o.rects[1], kids[1].rect);
}

#[test]
fn only_explicit_sizes_change() {
    let kids = [
        ce(0., 0., 100., 50.),
        Child {
            rect: r(200., 100., 108., 58.),
            w_explicit: true,
            h_explicit: true,
        },
        c(430., 230., 102., 70.),
    ];
    let o = tidy(800., 800., &kids, 16.);
    assert_eq!(
        rects(&o),
        vec![
            (0., 0., 104., 54.),
            (200., 100., 104., 54.),
            (430., 230., 102., 70.)
        ]
    );
    assert!(o.relations.contains(&Relation::SameSize {
        axis: Axis::X,
        members: vec![0, 1],
        value: 104
    }));
    assert!(o.relations.contains(&Relation::SameSize {
        axis: Axis::Y,
        members: vec![0, 1],
        value: 54
    }));
    assert_eq!(o.relations.len(), 2);
}

#[test]
fn tolerance_boundary() {
    // exactly T apart aligns
    let o = tidy(
        1000.,
        1000.,
        &[c(10., 10., 40., 40.), c(26., 200., 40., 40.)],
        16.,
    );
    assert_eq!(rects(&o), vec![(18., 10., 40., 40.), (18., 200., 40., 40.)]);
    assert_eq!(o.relations.len(), 1);
    // T + 1 does not
    let o = tidy(
        1000.,
        1000.,
        &[c(10., 10., 40., 40.), c(27., 200., 40., 40.)],
        16.,
    );
    assert_eq!(rects(&o), vec![(10., 10., 40., 40.), (27., 200., 40., 40.)]);
    assert!(o.relations.is_empty());
    // same on the y axis
    let o = tidy(
        1000.,
        1000.,
        &[c(10., 10., 40., 40.), c(200., 26., 40., 40.)],
        16.,
    );
    assert_eq!(rects(&o), vec![(10., 18., 40., 40.), (200., 18., 40., 40.)]);
    let o = tidy(
        1000.,
        1000.,
        &[c(10., 10., 40., 40.), c(200., 27., 40., 40.)],
        16.,
    );
    assert_eq!(rects(&o), vec![(10., 10., 40., 40.), (200., 27., 40., 40.)]);
}

#[test]
fn input_order_does_not_matter() {
    let kids = vec![
        c(43., 38., 60., 60.),
        c(148., 42., 60., 60.),
        c(262., 36., 60., 60.),
        c(38., 143., 60., 60.),
        c(153., 139., 60., 60.),
        c(257., 145., 60., 60.),
        c(300., 250., 30., 30.),
    ];
    let base = tidy(400., 300., &kids, 16.);
    let n = kids.len();
    let perms: Vec<Vec<usize>> = vec![
        (0..n).rev().collect(),
        (0..n).map(|i| (i + 3) % n).collect(),
        vec![4, 0, 6, 2, 5, 1, 3],
    ];
    for perm in perms {
        let shuffled: Vec<Child> = perm.iter().map(|&i| kids[i]).collect();
        let o = tidy(400., 300., &shuffled, 16.);
        for (pos, &orig) in perm.iter().enumerate() {
            assert_eq!(
                o.rects[pos], base.rects[orig],
                "perm {:?} element {}",
                perm, orig
            );
        }
    }
}

#[test]
fn sixty_children_are_fast() {
    let mut kids = Vec::new();
    let mut seed = 12345u32;
    let mut jit = || {
        seed = seed.wrapping_mul(1664525).wrapping_add(1013904223);
        ((seed >> 16) % 9) as f32 - 4.
    };
    for row in 0..6 {
        for col in 0..10 {
            kids.push(c(
                20. + col as f32 * 90. + jit(),
                20. + row as f32 * 70. + jit(),
                50.,
                40.,
            ));
        }
    }
    let t = std::time::Instant::now();
    let o = tidy(1000., 500., &kids, 16.);
    let el = t.elapsed();
    let limit = if cfg!(debug_assertions) { 3000 } else { 50 };
    assert!(el.as_millis() < limit, "took {:?}", el);
    assert_eq!(o.rects.len(), 60);
    for rc in &o.rects {
        assert!(rc.x.is_finite() && rc.y.is_finite());
    }
}

#[test]
fn empty_input() {
    let o = tidy(100., 100., &[], 16.);
    assert!(o.rects.is_empty() && o.relations.is_empty());
}
