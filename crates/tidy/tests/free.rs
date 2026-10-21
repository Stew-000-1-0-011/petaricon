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
    assert_relations_hold(&o);
    let al = |axis, members: Vec<usize>, coord| Relation::Align {
        axis,
        anchor: Anchor::Start,
        members,
        coord,
    };
    let rp = |dx, dy, pairs: Vec<(usize, usize)>| Relation::Repeat { dx, dy, pairs };
    let gap = |members: Vec<usize>| Relation::EqualGap {
        axis: Axis::X,
        members,
        gap: 50.,
    };
    // Symmetry about the bounding-box line (x = 179.5) would need half-pixel
    // positions, so it does not hold on the integer grid and is not reported.
    assert_eq!(
        o.relations,
        vec![
            al(Axis::X, vec![2, 5], 260.),
            al(Axis::X, vec![1, 4], 150.),
            al(Axis::X, vec![0, 3], 40.),
            al(Axis::Y, vec![0, 1, 2], 39.),
            al(Axis::Y, vec![3, 4, 5], 142.),
            gap(vec![0, 1, 2]),
            gap(vec![3, 4, 5]),
            // the two rows are chains; their skip pairs (220, 0) are implied
            rp(110., 0., vec![(0, 1), (1, 2), (3, 4), (4, 5)]),
            rp(110., -103., vec![(3, 1), (4, 2)]),
            rp(110., 103., vec![(1, 5), (0, 4)]),
            rp(0., 103., vec![(1, 4), (0, 3), (2, 5)]),
        ]
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
    assert_relations_hold(&a);
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
        let o = tidy(1000., 1000., &shuffled, 16.);
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

// ---- shared checks ---------------------------------------------------------

const HALF: f32 = 0.5 + 1e-6;

fn anchor_of(rc: &Rect, axis: Axis, anchor: Anchor) -> f32 {
    let (p, s) = if axis == Axis::X {
        (rc.x, rc.w)
    } else {
        (rc.y, rc.h)
    };
    match anchor {
        Anchor::Start => p,
        Anchor::Center => p + s / 2.,
        Anchor::End => p + s,
    }
}
fn span(v: &[f32]) -> f32 {
    let lo = v.iter().copied().fold(f32::INFINITY, f32::min);
    let hi = v.iter().copied().fold(f32::NEG_INFINITY, f32::max);
    hi - lo
}

/// Every reported relation must hold on the final rects (to half a pixel).
fn assert_relations_hold(o: &Outcome) {
    let rc = &o.rects;
    for rel in &o.relations {
        match rel {
            Relation::SameSize {
                axis,
                members,
                value,
            } => {
                for &m in members {
                    let s = if *axis == Axis::X { rc[m].w } else { rc[m].h };
                    assert_eq!(s, *value as f32, "{rel:?}");
                }
            }
            Relation::Align {
                axis,
                anchor,
                members,
                coord,
            } => {
                let v: Vec<f32> = members
                    .iter()
                    .map(|&m| anchor_of(&rc[m], *axis, *anchor))
                    .collect();
                assert!(span(&v) <= HALF, "{rel:?} {v:?}");
                assert!(v.iter().all(|x| (x - coord).abs() <= HALF), "{rel:?} {v:?}");
            }
            Relation::EqualGap { axis, members, gap } => {
                let gaps: Vec<f32> = members
                    .windows(2)
                    .map(|w| {
                        anchor_of(&rc[w[1]], *axis, Anchor::Start)
                            - anchor_of(&rc[w[0]], *axis, Anchor::End)
                    })
                    .collect();
                assert!(span(&gaps) <= HALF, "{rel:?} {gaps:?}");
                assert!(
                    gaps.iter().all(|x| (x - gap).abs() <= HALF),
                    "{rel:?} {gaps:?}"
                );
            }
            Relation::Repeat { dx, dy, pairs } => {
                for &(i, j) in pairs {
                    assert!((rc[j].x - rc[i].x - dx).abs() <= HALF, "{rel:?}");
                    assert!((rc[j].y - rc[i].y - dy).abs() <= HALF, "{rel:?}");
                }
            }
            Relation::Symmetric {
                axis,
                center,
                pairs,
                singles,
            } => {
                for &(i, j) in pairs {
                    let m = (anchor_of(&rc[i], *axis, Anchor::Center)
                        + anchor_of(&rc[j], *axis, Anchor::Center))
                        / 2.;
                    assert!((m - center).abs() <= HALF, "{rel:?}");
                }
                for &i in singles {
                    assert!(
                        (anchor_of(&rc[i], *axis, Anchor::Center) - center).abs() <= HALF,
                        "{rel:?}"
                    );
                }
            }
        }
    }
}

struct Lcg(u32);
impl Lcg {
    fn next(&mut self) -> u32 {
        self.0 = self.0.wrapping_mul(1664525).wrapping_add(1013904223);
        self.0 >> 8
    }
    fn range(&mut self, lo: i32, hi: i32) -> f32 {
        (lo + (self.next() % (hi - lo + 1) as u32) as i32) as f32
    }
}

#[test]
fn shuffled_input_gives_identical_output_and_relations_hold() {
    let mut rng = Lcg(2024);
    let mut checked = 0;
    let mut chains_seen = 0;
    for case in 0..60 {
        let mut kids: Vec<Child> = Vec::new();
        if case % 3 == 2 {
            // diagonal / stepped chain (3-5 elements) plus a stray element
            let len = 3 + case % 3;
            let (sx, sy) = (rng.range(-120, 120), rng.range(16, 240));
            for k in 0..len {
                kids.push(c(
                    300. + sx * k as f32 + rng.range(-6, 6),
                    20. + sy * k as f32 / 2. + rng.range(-6, 6),
                    50.,
                    50.,
                ));
            }
            kids.push(c(rng.range(0, 600), rng.range(0, 600), 44., 44.));
        } else if case % 3 == 0 {
            // near-regular grid with jitter and the odd size
            let (rows, cols) = (1 + case % 4, 2 + case % 3);
            for rr in 0..rows {
                for cc in 0..cols {
                    let sz = if (rr + cc) % 5 == 4 { 31. } else { 60. };
                    kids.push(c(
                        30. + cc as f32 * 100. + rng.range(-6, 6),
                        30. + rr as f32 * 90. + rng.range(-6, 6),
                        sz,
                        sz,
                    ));
                }
            }
        } else {
            // cluttered: many elements in a small area, few distinct sizes
            for _ in 0..(6 + case % 9) {
                let w = [30., 40., 44., 90.][(rng.next() % 4) as usize];
                kids.push(Child {
                    rect: r(
                        rng.range(0, 260),
                        rng.range(0, 200),
                        w,
                        [30., 36., 60.][(rng.next() % 3) as usize],
                    ),
                    w_explicit: rng.next().is_multiple_of(2),
                    h_explicit: rng.next().is_multiple_of(2),
                });
            }
        }
        let base = tidy(1000., 1000., &kids, 16.);
        assert_relations_hold(&base);
        if case % 3 == 2
            && repeats(&base)
                .iter()
                .any(|(_, _, p)| p.windows(2).any(|w| w[0].1 == w[1].0))
        {
            chains_seen += 1;
        }
        for (k, rc) in base.rects.iter().enumerate() {
            let src = kids[k].rect;
            assert!(
                (rc.x - src.x).abs() <= 16. && (rc.y - src.y).abs() <= 16.,
                "case {case} el {k}"
            );
        }
        for _ in 0..12 {
            let n = kids.len();
            let mut perm: Vec<usize> = (0..n).collect();
            for i in (1..n).rev() {
                perm.swap(i, (rng.next() as usize) % (i + 1));
            }
            let shuffled: Vec<Child> = perm.iter().map(|&i| kids[i]).collect();
            let o = tidy(1000., 1000., &shuffled, 16.);
            assert_relations_hold(&o);
            for (pos, &orig) in perm.iter().enumerate() {
                assert_eq!(
                    o.rects[pos], base.rects[orig],
                    "case {case} perm {perm:?} element {orig}"
                );
            }
            checked += 1;
        }
    }
    assert_eq!(checked, 720);
    assert!(
        chains_seen >= 10,
        "chain layouts were not exercised: {chains_seen}"
    );
}

#[test]
fn near_vertical_repeat_is_not_split_by_sign() {
    // a column whose x wobbles around 100 (sign of dx flips between pairs)
    let xs = [100., 99., 101., 98., 100.];
    let kids: Vec<Child> = xs
        .iter()
        .enumerate()
        .map(|(k, &x)| c(x, 20. + 80. * k as f32, 40., 40.))
        .collect();
    let o = tidy(1000., 1000., &kids, 16.);
    assert_relations_hold(&o);
    assert!(o.rects.iter().all(|rc| rc.x == o.rects[0].x));
    let best = o
        .relations
        .iter()
        .filter_map(|x| match x {
            Relation::Repeat { dx, dy, pairs } => Some((*dx, *dy, pairs.len())),
            _ => None,
        })
        .max_by_key(|v| v.2)
        .expect("a repeat relation");
    assert_eq!(
        best.2, 4,
        "all four consecutive pairs share one repeat: {:?}",
        o.relations
    );
    assert_eq!((best.0, best.1), (0., 80.));
}

// ---- chains (diagonal / stepped sequences) ---------------------------------

type RepeatInfo = (f32, f32, Vec<(usize, usize)>);

fn repeats(o: &Outcome) -> Vec<RepeatInfo> {
    o.relations
        .iter()
        .filter_map(|x| match x {
            Relation::Repeat { dx, dy, pairs } => Some((*dx, *dy, pairs.clone())),
            _ => None,
        })
        .collect()
}
fn aligns(o: &Outcome) -> Vec<&Relation> {
    o.relations
        .iter()
        .filter(|x| matches!(x, Relation::Align { .. }))
        .collect()
}
fn assert_moves_within(o: &Outcome, kids: &[Child], t: f32) {
    for (rc, k) in o.rects.iter().zip(kids) {
        assert!((rc.x - k.rect.x).abs() <= t && (rc.y - k.rect.y).abs() <= t);
    }
}

#[test]
fn diagonal_chain_gets_equal_steps() {
    let kids = [
        c(43., 16., 60., 60.),
        c(135., 245., 60., 60.),
        c(244., 457., 60., 60.),
        c(338., 686., 60., 60.),
    ];
    let o = tidy(800., 1200., &kids, 16.);
    assert_eq!(
        rects(&o),
        vec![
            (42., 18., 60., 60.),
            (141., 240., 60., 60.),
            (240., 462., 60., 60.),
            (339., 684., 60., 60.)
        ]
    );
    assert_eq!(
        o.relations,
        vec![Relation::Repeat {
            dx: 99.,
            dy: 222.,
            pairs: vec![(0, 1), (1, 2), (2, 3)]
        }]
    );
    assert!(aligns(&o).is_empty());
    assert_relations_hold(&o);
    assert_moves_within(&o, &kids, 16.);
}

#[test]
fn small_x_step_diagonal_is_not_split_into_columns() {
    // x step 15 > T/2: the columns {0,1} and {2,3} must not be formed
    let kids = [
        c(101., 48., 40., 40.),
        c(113., 123., 40., 40.),
        c(132., 187., 40., 40.),
        c(144., 262., 40., 40.),
    ];
    let o = tidy(800., 800., &kids, 16.);
    let rs = rects(&o);
    assert_eq!(
        rs.iter().map(|r| r.0).collect::<Vec<_>>(),
        vec![100., 115., 130., 145.]
    );
    let ys: Vec<f32> = rs.iter().map(|r| r.1).collect();
    assert_eq!(ys[1] - ys[0], ys[2] - ys[1]);
    assert_eq!(ys[2] - ys[1], ys[3] - ys[2]);
    assert!(aligns(&o).is_empty(), "{:?}", o.relations);
    let reps = repeats(&o);
    assert_eq!(reps.len(), 1, "{:?}", o.relations);
    assert_eq!(
        (reps[0].0, reps[0].2.clone()),
        (15., vec![(0, 1), (1, 2), (2, 3)])
    );
    assert_relations_hold(&o);
    assert_moves_within(&o, &kids, 16.);
}

#[test]
fn nearly_vertical_chain_is_a_column_with_equal_steps() {
    let kids = [
        c(100., 22., 40., 40.),
        c(103., 100., 40., 40.),
        c(98., 180., 40., 40.),
        c(101., 258., 40., 40.),
    ];
    let o = tidy(800., 800., &kids, 16.);
    assert_eq!(
        rects(&o),
        vec![
            (101., 22., 40., 40.),
            (101., 101., 40., 40.),
            (101., 180., 40., 40.),
            (101., 259., 40., 40.)
        ]
    );
    assert!(has_align(&o, Axis::X, Anchor::Start, &[0, 1, 2, 3], 101.));
    let reps = repeats(&o);
    assert_eq!(reps.len(), 1, "{:?}", o.relations);
    assert_eq!((reps[0].0, reps[0].1), (0., 79.));
    assert_eq!(reps[0].2, vec![(0, 1), (1, 2), (2, 3)]);
    assert_relations_hold(&o);
}

#[test]
fn staircase_with_alternating_steps_is_not_a_chain() {
    // steps (100,40), (40,100), (100,40): consecutive steps differ by 60 > T
    let kids = [
        c(20., 20., 40., 40.),
        c(120., 60., 40., 40.),
        c(160., 160., 40., 40.),
        c(260., 200., 40., 40.),
    ];
    let o = tidy(1000., 1000., &kids, 16.);
    for (rc, k) in o.rects.iter().zip(&kids) {
        assert_eq!(*rc, k.rect);
    }
    for (_, _, pairs) in repeats(&o) {
        // no pair may share an element: that would be a chain
        for (a, p) in pairs.iter().enumerate() {
            for q in &pairs[a + 1..] {
                assert!(
                    p.0 != q.0 && p.1 != q.1 && p.0 != q.1 && p.1 != q.0,
                    "{:?}",
                    o.relations
                );
            }
        }
    }
    assert_relations_hold(&o);
    assert_moves_within(&o, &kids, 16.);
}

#[test]
fn unrelated_element_next_to_a_chain_is_untouched() {
    let kids = [
        c(43., 16., 60., 60.),
        c(135., 245., 60., 60.),
        c(244., 457., 60., 60.),
        c(338., 686., 60., 60.),
        c(601.3, 333.7, 50., 30.),
    ];
    let o = tidy(800., 1200., &kids, 16.);
    assert_eq!(o.rects[4], kids[4].rect);
    for rel in &o.relations {
        let touches = match rel {
            Relation::Repeat { pairs, .. } => pairs.iter().any(|p| p.0 == 4 || p.1 == 4),
            Relation::Align { members, .. } | Relation::EqualGap { members, .. } => {
                members.contains(&4)
            }
            _ => false,
        };
        assert!(!touches, "{rel:?}");
    }
    assert_eq!(
        o.rects[1],
        Rect {
            x: 141.,
            y: 240.,
            w: 60.,
            h: 60.
        }
    );
    assert_relations_hold(&o);
}

#[test]
fn long_diagonal_reports_one_repeat_without_stride_two_copies() {
    let kids = [
        c(40., 20., 50., 50.),
        c(145., 205., 50., 50.),
        c(238., 410., 50., 50.),
        c(349., 598., 50., 50.),
        c(436., 805., 50., 50.),
    ];
    let o = tidy(1000., 1200., &kids, 16.);
    let reps = repeats(&o);
    assert_eq!(reps.len(), 1, "{:?}", o.relations);
    assert_eq!(reps[0].2, vec![(0, 1), (1, 2), (2, 3), (3, 4)]);
    assert_relations_hold(&o);
    assert_moves_within(&o, &kids, 16.);
}

#[test]
fn long_vertical_column_reports_one_repeat() {
    let kids: Vec<Child> = [101., 98., 103., 100., 99.]
        .iter()
        .enumerate()
        .map(|(k, &x)| {
            c(
                x,
                20. + 80. * k as f32 + [0., 3., -2., 4., -3.][k],
                40.,
                40.,
            )
        })
        .collect();
    let o = tidy(800., 800., &kids, 16.);
    let reps = repeats(&o);
    assert_eq!(reps.len(), 1, "{:?}", o.relations);
    assert_eq!(reps[0].2, vec![(0, 1), (1, 2), (2, 3), (3, 4)]);
    assert!(has_align(
        &o,
        Axis::X,
        Anchor::Start,
        &[0, 1, 2, 3, 4],
        100.
    ));
    assert_relations_hold(&o);
}

#[test]
fn chain_near_the_container_edge_stays_inside() {
    // the unconstrained fit puts the first element at x = -1
    let kids = [
        c(0., 10., 50., 50.),
        c(98., 200., 50., 50.),
        c(203., 390., 50., 50.),
    ];
    let o = tidy(400., 600., &kids, 16.);
    for rc in &o.rects {
        assert!(rc.x >= 0. && rc.y >= 0. && rc.x + rc.w <= 400. && rc.y + rc.h <= 600.);
    }
    assert_eq!(repeats(&o).len(), 1, "{:?}", o.relations);
    assert_relations_hold(&o);
    assert_moves_within(&o, &kids, 16.);
}

#[test]
fn chain_step_is_reported_as_the_exact_integer_step() {
    // fractional input coordinates
    let kids = [
        c(40.3, 20.7, 60., 60.),
        c(139.9, 240.2, 60., 60.),
        c(241.6, 461.1, 60., 60.),
        c(338.2, 683.4, 60., 60.),
    ];
    let o = tidy(800., 1200., &kids, 16.);
    let reps = repeats(&o);
    assert_eq!(reps.len(), 1, "{:?}", o.relations);
    let (dx, dy, pairs) = &reps[0];
    assert_eq!((dx.fract(), dy.fract()), (0., 0.), "{:?}", o.relations);
    for &(i, j) in pairs {
        assert_eq!(o.rects[j].x - o.rects[i].x, *dx);
        assert_eq!(o.rects[j].y - o.rects[i].y, *dy);
    }
}
