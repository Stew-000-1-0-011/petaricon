//! Free-placement tidy: nudges children towards "intended" values (shared
//! sizes, alignments, equal gaps, repeated offsets, symmetry) while leaving
//! the overall arrangement alone.  See
//! `docs/superpowers/specs/2026-10-21-free-tidy-design.md`, sections 2-3.

use crate::Rect;
use std::cmp::Ordering;

#[derive(Clone, Copy, Debug)]
pub struct Child {
    pub rect: Rect,
    pub w_explicit: bool,
    pub h_explicit: bool,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Axis {
    X,
    Y,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Anchor {
    Start,
    Center,
    End,
}

#[derive(Clone, Debug, PartialEq)]
pub enum Relation {
    SameSize {
        axis: Axis,
        members: Vec<usize>,
        value: i32,
    },
    Align {
        axis: Axis,
        anchor: Anchor,
        members: Vec<usize>,
        coord: f32,
    },
    /// `members` are in order along `axis`.
    EqualGap {
        axis: Axis,
        members: Vec<usize>,
        gap: f32,
    },
    Repeat {
        dx: f32,
        dy: f32,
        pairs: Vec<(usize, usize)>,
    },
    Symmetric {
        axis: Axis,
        center: f32,
        pairs: Vec<(usize, usize)>,
        singles: Vec<usize>,
    },
}

#[derive(Clone, Debug)]
pub struct Outcome {
    pub rects: Vec<Rect>,
    pub relations: Vec<Relation>,
}

/// Weight of a relation equality against "stay where you are" (weight 1).
const RELATION_WEIGHT: f64 = 1000.0;
const AXES: [Axis; 2] = [Axis::X, Axis::Y];
const ANCHORS: [Anchor; 3] = [Anchor::Start, Anchor::Center, Anchor::End];

/// Working geometry, `[x, y, w, h]`.
type G = [f64; 4];

fn pos_ix(a: Axis) -> usize {
    if a == Axis::X { 0 } else { 1 }
}
fn size_ix(a: Axis) -> usize {
    if a == Axis::X { 2 } else { 3 }
}
fn other(a: Axis) -> Axis {
    if a == Axis::X { Axis::Y } else { Axis::X }
}
fn pos(g: &[G], i: usize, a: Axis) -> f64 {
    g[i][pos_ix(a)]
}
fn size(g: &[G], i: usize, a: Axis) -> f64 {
    g[i][size_ix(a)]
}
fn center(g: &[G], i: usize, a: Axis) -> f64 {
    pos(g, i, a) + size(g, i, a) / 2.0
}
fn anchor_off(anchor: Anchor, s: f64) -> f64 {
    match anchor {
        Anchor::Start => 0.0,
        Anchor::Center => s / 2.0,
        Anchor::End => s,
    }
}
fn cmp_geo(a: &G, b: &G) -> Ordering {
    for k in 0..4 {
        let o = a[k].total_cmp(&b[k]);
        if o != Ordering::Equal {
            return o;
        }
    }
    Ordering::Equal
}

/// Greedy clustering of sorted values: grow while `value - cluster_min <= t`.
/// Returns clusters (>= `min_len` members) as index lists.
fn cluster(items: &mut [(f64, usize)], t: f64, min_len: usize) -> Vec<Vec<usize>> {
    items.sort_by(|a, b| a.0.total_cmp(&b.0).then(a.1.cmp(&b.1)));
    let mut out = Vec::new();
    let mut cur: Vec<usize> = Vec::new();
    let mut min = 0.0;
    for &(v, i) in items.iter() {
        if !cur.is_empty() && v - min > t {
            if cur.len() >= min_len {
                out.push(std::mem::take(&mut cur));
            }
            cur.clear();
        }
        if cur.is_empty() {
            min = v;
        }
        cur.push(i);
    }
    if cur.len() >= min_len {
        out.push(cur);
    }
    out
}

struct AlignCl {
    anchor: Anchor,
    members: Vec<usize>,
}

/// Step 3.2-1: one alignment cluster per element and axis.
fn align_clusters(g: &[G], a: Axis, t: f64) -> Vec<AlignCl> {
    struct Cand {
        anchor_rank: usize,
        members: Vec<usize>,
        lo: f64,
        hi: f64,
    }
    let n = g.len();
    let mut cands = Vec::new();
    for (rank, &anchor) in ANCHORS.iter().enumerate() {
        let mut items: Vec<(f64, usize)> = (0..n)
            .map(|i| (pos(g, i, a) + anchor_off(anchor, size(g, i, a)), i))
            .collect();
        for members in cluster(&mut items, t, 2) {
            let vals = members
                .iter()
                .map(|&i| pos(g, i, a) + anchor_off(anchor, size(g, i, a)));
            let lo = vals.clone().fold(f64::INFINITY, f64::min);
            let hi = vals.fold(f64::NEG_INFINITY, f64::max);
            cands.push(Cand {
                anchor_rank: rank,
                members,
                lo,
                hi,
            });
        }
    }
    let mut used = vec![false; n];
    let mut chosen = Vec::new();
    loop {
        for c in cands.iter_mut() {
            c.members.retain(|&i| !used[i]);
        }
        let mut best: Option<usize> = None;
        for k in 0..cands.len() {
            if cands[k].members.len() < 2 {
                continue;
            }
            let better = match best {
                None => true,
                Some(b) => {
                    let o = &cands[b];
                    let c = &cands[k];
                    // more members, then narrower range, then anchor order, then coordinate
                    c.members
                        .len()
                        .cmp(&o.members.len())
                        .reverse()
                        .then((c.hi - c.lo).total_cmp(&(o.hi - o.lo)))
                        .then(c.anchor_rank.cmp(&o.anchor_rank))
                        .then(c.lo.total_cmp(&o.lo))
                        == Ordering::Less
                }
            };
            if better {
                best = Some(k);
            }
        }
        let Some(b) = best else { break };
        let mut members = cands[b].members.clone();
        members.sort_unstable();
        for &i in &members {
            used[i] = true;
        }
        chosen.push(AlignCl {
            anchor: ANCHORS[cands[b].anchor_rank],
            members,
        });
        cands[b].members.clear();
    }
    chosen
}

/// One linear equation `sum(coef * p[index]) = b` on a single axis.
struct Eq {
    kind: Kind,
    terms: Vec<(usize, f64)>,
    b: f64,
}

/// Relation families, in the order they are given up when the combined
/// solution would move an element by more than the tolerance.
#[derive(Clone, Copy, PartialEq, Eq)]
enum Kind {
    Align,
    Gap,
    Repeat,
    Symmetric,
}

#[derive(Default)]
struct AxisEqs {
    eqs: Vec<Eq>,
}

impl AxisEqs {
    fn involved(&self, n: usize, on: &[Kind]) -> Vec<bool> {
        let mut v = vec![false; n];
        for e in self.eqs.iter().filter(|e| on.contains(&e.kind)) {
            for &(i, _) in &e.terms {
                v[i] = true;
            }
        }
        v
    }
}

/// Dense Gaussian elimination with partial pivoting; `m` is n x n row-major.
fn solve(mut m: Vec<f64>, mut rhs: Vec<f64>, n: usize) -> Vec<f64> {
    for col in 0..n {
        let mut piv = col;
        for r in col + 1..n {
            if m[r * n + col].abs() > m[piv * n + col].abs() {
                piv = r;
            }
        }
        if piv != col {
            for k in 0..n {
                m.swap(col * n + k, piv * n + k);
            }
            rhs.swap(col, piv);
        }
        let d = m[col * n + col];
        if d.abs() < 1e-12 {
            continue;
        }
        for r in col + 1..n {
            let f = m[r * n + col] / d;
            if f == 0.0 {
                continue;
            }
            for k in col..n {
                m[r * n + k] -= f * m[col * n + k];
            }
            rhs[r] -= f * rhs[col];
        }
    }
    let mut x = vec![0.0; n];
    for r in (0..n).rev() {
        let mut s = rhs[r];
        for k in r + 1..n {
            s -= m[r * n + k] * x[k];
        }
        let d = m[r * n + r];
        x[r] = if d.abs() < 1e-12 { 0.0 } else { s / d };
    }
    x
}

/// Step 3.3: minimise `sum 1000*(a.p - b)^2 + sum (p_i - p0_i)^2`.
fn solve_axis(p0: &[f64], eqs: &[Eq], on: &[Kind]) -> Vec<f64> {
    let n = p0.len();
    let mut m = vec![0.0; n * n];
    let mut rhs = p0.to_vec();
    for i in 0..n {
        m[i * n + i] = 1.0;
    }
    for e in eqs.iter().filter(|e| on.contains(&e.kind)) {
        for &(i, ci) in &e.terms {
            rhs[i] += RELATION_WEIGHT * ci * e.b;
            for &(j, cj) in &e.terms {
                m[i * n + j] += RELATION_WEIGHT * ci * cj;
            }
        }
    }
    solve(m, rhs, n)
}

/// Rounds to an integer; first snaps to 0.1 so that the tiny residual of the
/// soft equalities cannot split members that the solver meant to coincide.
fn round_coord(v: f64) -> f64 {
    ((v * 10.0).round() / 10.0 + 0.5).floor()
}

fn median_round(sorted: &[f64]) -> f64 {
    let n = sorted.len();
    let m = if n % 2 == 1 {
        sorted[n / 2]
    } else {
        (sorted[n / 2 - 1] + sorted[n / 2]) / 2.0
    };
    (m + 0.5).floor()
}

/// A mirror axis found for one axis.
struct Sym {
    center: f64,
    pairs: Vec<(usize, usize)>,
    singles: Vec<usize>,
}

fn find_symmetry(g: &[G], a: Axis, container: f64, t: f64) -> Option<Sym> {
    let n = g.len();
    let o = other(a);
    let lo = (0..n).map(|i| pos(g, i, a)).fold(f64::INFINITY, f64::min);
    let hi = (0..n)
        .map(|i| pos(g, i, a) + size(g, i, a))
        .fold(f64::NEG_INFINITY, f64::max);
    let eval = |c: f64| -> Sym {
        let mut cands: Vec<(f64, usize, usize)> = Vec::new();
        for i in 0..n {
            for j in i + 1..n {
                let cost = (center(g, i, a) + center(g, j, a) - 2.0 * c).abs();
                if cost <= t
                    && (size(g, i, a) - size(g, j, a)).abs() <= t
                    && (size(g, i, o) - size(g, j, o)).abs() <= t
                    && (center(g, i, o) - center(g, j, o)).abs() <= t
                {
                    cands.push((cost, i, j));
                }
            }
        }
        cands.sort_by(|x, y| {
            let kx = key_pair(g, x.1, x.2);
            let ky = key_pair(g, y.1, y.2);
            x.0.total_cmp(&y.0)
                .then_with(|| cmp_geo(&kx.0, &ky.0))
                .then_with(|| cmp_geo(&kx.1, &ky.1))
        });
        let mut taken = vec![false; n];
        let mut pairs = Vec::new();
        for (_, i, j) in cands {
            if !taken[i] && !taken[j] {
                taken[i] = true;
                taken[j] = true;
                pairs.push((i, j));
            }
        }
        let singles = (0..n)
            .filter(|&i| !taken[i] && (center(g, i, a) - c).abs() <= t)
            .collect();
        Sym {
            center: c,
            pairs,
            singles,
        }
    };
    let count = |s: &Sym| s.pairs.len() + s.singles.len();
    let by_container = eval(container / 2.0);
    let by_box = eval((lo + hi) / 2.0);
    // The bounding-box line is derived from the children themselves, so a lone
    // mirrored pair (the two extremes) or lone centred element always "fits".
    // Require real evidence: at least one pair and two relations in total.
    let box_ok = !by_box.pairs.is_empty() && count(&by_box) >= 2;
    let pick = if box_ok && count(&by_box) > count(&by_container) {
        by_box
    } else {
        by_container
    };
    if count(&pick) == 0 { None } else { Some(pick) }
}

fn key_pair(g: &[G], i: usize, j: usize) -> (G, G) {
    if cmp_geo(&g[i], &g[j]) == Ordering::Greater {
        (g[j], g[i])
    } else {
        (g[i], g[j])
    }
}

struct Rep {
    pairs: Vec<(usize, usize)>,
}

/// Step 3.2-3: clusters of near-identical offset vectors.
fn find_repeats(g: &[G], t: f64) -> Vec<Rep> {
    struct V {
        dx: f64,
        dy: f64,
        i: usize,
        j: usize,
    }
    let n = g.len();
    let mut vs: Vec<V> = Vec::new();
    for i in 0..n {
        for j in i + 1..n {
            let (mut a, mut b) = (i, j);
            let mut dx = g[b][0] - g[a][0];
            let mut dy = g[b][1] - g[a][1];
            if dx < 0.0 || (dx == 0.0 && dy < 0.0) {
                std::mem::swap(&mut a, &mut b);
                dx = -dx;
                dy = -dy;
            }
            if dx.hypot(dy) < t {
                continue;
            }
            vs.push(V { dx, dy, i: a, j: b });
        }
    }
    vs.sort_by(|p, q| {
        p.dx.total_cmp(&q.dx)
            .then(p.dy.total_cmp(&q.dy))
            .then_with(|| cmp_geo(&g[p.i], &g[q.i]))
            .then_with(|| cmp_geo(&g[p.j], &g[q.j]))
    });
    let mut used = vec![false; vs.len()];
    let mut out = Vec::new();
    for s in 0..vs.len() {
        if used[s] {
            continue;
        }
        let (sx, sy) = (vs[s].dx, vs[s].dy);
        // Candidates sorted by dx: only a bounded window can be within t.
        let mut cand: Vec<usize> = Vec::new();
        for k in s..vs.len() {
            if vs[k].dx - sx > t {
                break;
            }
            if !used[k] && (vs[k].dy - sy).abs() <= t {
                cand.push(k);
            }
        }
        cand.sort_by(|&p, &q| vs[p].dy.total_cmp(&vs[q].dy).then(p.cmp(&q)));
        // densest dy-window of width t that contains the seed
        let mut best: Vec<usize> = Vec::new();
        for &first in &cand {
            let lo = vs[first].dy;
            if lo > sy {
                break;
            }
            let win: Vec<usize> = cand
                .iter()
                .copied()
                .filter(|&k| vs[k].dy >= lo && vs[k].dy - lo <= t)
                .collect();
            if win.contains(&s) && win.len() > best.len() {
                best = win;
            }
        }
        for &k in &best {
            used[k] = true;
        }
        used[s] = true;
        // A source or target may appear once, otherwise two near-identical
        // elements would be forced onto the same spot.
        let mut srcs = Vec::new();
        let mut dsts = Vec::new();
        let mut pairs = Vec::new();
        for &k in &best {
            let (i, j) = (vs[k].i, vs[k].j);
            if srcs.contains(&i) || dsts.contains(&j) {
                continue;
            }
            srcs.push(i);
            dsts.push(j);
            pairs.push((i, j));
        }
        if pairs.len() >= 2 {
            out.push(Rep { pairs });
        }
    }
    out
}

/// Solves both axes with the given relation families and rounds the result.
/// Sizes come from `g`; positions not touched by any equation keep `orig`.
fn solve_all(orig: &[G], g: &[G], eqs: &[AxisEqs; 2], on: &[Kind]) -> Vec<G> {
    let n = g.len();
    let mut out = g.to_vec();
    for (k, a) in AXES.iter().copied().enumerate() {
        let p0: Vec<f64> = (0..n).map(|i| pos(orig, i, a)).collect();
        let inv = eqs[k].involved(n, on);
        let sol = solve_axis(&p0, &eqs[k].eqs, on);
        for i in 0..n {
            out[i][pos_ix(a)] = if inv[i] { round_coord(sol[i]) } else { p0[i] };
        }
    }
    out
}

pub fn tidy(container_w: f32, container_h: f32, children: &[Child], tolerance: f32) -> Outcome {
    let n = children.len();
    let t = tolerance as f64;
    let mut relations = Vec::new();
    if n == 0 {
        return Outcome {
            rects: Vec::new(),
            relations,
        };
    }
    let orig: Vec<G> = children
        .iter()
        .map(|c| {
            [
                c.rect.x as f64,
                c.rect.y as f64,
                c.rect.w as f64,
                c.rect.h as f64,
            ]
        })
        .collect();
    let mut g = orig.clone();

    // 3.1 shared sizes (explicit sizes only)
    for a in AXES {
        let explicit = |i: usize| {
            if a == Axis::X {
                children[i].w_explicit
            } else {
                children[i].h_explicit
            }
        };
        let mut items: Vec<(f64, usize)> = (0..n)
            .filter(|&i| explicit(i))
            .map(|i| (size(&g, i, a), i))
            .collect();
        for members in cluster(&mut items, t, 2) {
            let vals: Vec<f64> = members.iter().map(|&i| size(&g, i, a)).collect();
            let value = median_round(&vals);
            for &i in &members {
                g[i][size_ix(a)] = value;
            }
            let mut members = members;
            members.sort_unstable();
            relations.push(Relation::SameSize {
                axis: a,
                members,
                value: value as i32,
            });
        }
    }

    let container = |a: Axis| {
        if a == Axis::X {
            container_w as f64
        } else {
            container_h as f64
        }
    };
    let mut eqs = [AxisEqs::default(), AxisEqs::default()];
    let ax = |a: Axis| if a == Axis::X { 0 } else { 1 };

    // 3.2-1 alignment
    let aligns = [
        align_clusters(&g, Axis::X, t),
        align_clusters(&g, Axis::Y, t),
    ];
    for a in AXES {
        for cl in &aligns[ax(a)] {
            let s0 = cl.members[0];
            let off0 = anchor_off(cl.anchor, size(&g, s0, a));
            for &m in &cl.members[1..] {
                let off = anchor_off(cl.anchor, size(&g, m, a));
                eqs[ax(a)].eqs.push(Eq {
                    kind: Kind::Align,
                    terms: vec![(m, 1.0), (s0, -1.0)],
                    b: off0 - off,
                });
            }
        }
    }

    // 3.2-2 equal gaps: elements sharing a row (column) along the other axis
    let mut gap_groups: Vec<(Axis, Vec<usize>)> = Vec::new();
    for a in AXES {
        for cl in &aligns[ax(other(a))] {
            if cl.members.len() < 3 {
                continue;
            }
            let mut order = cl.members.clone();
            order.sort_by(|&p, &q| {
                pos(&g, p, a)
                    .total_cmp(&pos(&g, q, a))
                    .then(size(&g, p, a).total_cmp(&size(&g, q, a)))
            });
            let gaps: Vec<f64> = order
                .windows(2)
                .map(|w| pos(&g, w[1], a) - pos(&g, w[0], a) - size(&g, w[0], a))
                .collect();
            let gmax = gaps.iter().copied().fold(f64::NEG_INFINITY, f64::max);
            let gmin = gaps.iter().copied().fold(f64::INFINITY, f64::min);
            if gmax - gmin > t {
                continue;
            }
            for w in order.windows(3) {
                eqs[ax(a)].eqs.push(Eq {
                    kind: Kind::Gap,
                    terms: vec![(w[0], -1.0), (w[1], 2.0), (w[2], -1.0)],
                    b: size(&g, w[0], a) - size(&g, w[1], a),
                });
            }
            gap_groups.push((a, order));
        }
    }

    // 3.2-3 repeated offsets
    let repeats = find_repeats(&g, t);
    for rep in &repeats {
        let (k, l) = rep.pairs[0];
        for &(i, j) in &rep.pairs[1..] {
            for a in AXES {
                eqs[ax(a)].eqs.push(Eq {
                    kind: Kind::Repeat,
                    terms: vec![(j, 1.0), (i, -1.0), (l, -1.0), (k, 1.0)],
                    b: 0.0,
                });
            }
        }
    }

    // 3.2-4 symmetry
    let mut syms: Vec<(Axis, Sym)> = Vec::new();
    for a in AXES {
        if let Some(s) = find_symmetry(&g, a, container(a), t) {
            for &(i, j) in &s.pairs {
                eqs[ax(a)].eqs.push(Eq {
                    kind: Kind::Symmetric,
                    terms: vec![(i, 1.0), (j, 1.0)],
                    b: 2.0 * s.center - (size(&g, i, a) + size(&g, j, a)) / 2.0,
                });
            }
            for &i in &s.singles {
                eqs[ax(a)].eqs.push(Eq {
                    kind: Kind::Symmetric,
                    terms: vec![(i, 1.0)],
                    b: s.center - size(&g, i, a) / 2.0,
                });
            }
            syms.push((a, s));
        }
    }

    // 3.3 solve and round.  If the combined solution would move some element
    // by more than the tolerance (relations that contradict each other), give
    // up whole relation families, weakest first; alignment alone never does.
    let levels: [&[Kind]; 4] = [
        &[Kind::Align, Kind::Gap, Kind::Repeat, Kind::Symmetric],
        &[Kind::Align, Kind::Gap, Kind::Symmetric],
        &[Kind::Align, Kind::Gap],
        &[Kind::Align],
    ];
    let mut on = levels[3];
    for lv in levels {
        let sol = solve_all(&orig, &g, &eqs, lv);
        let ok = (0..n).all(|i| {
            AXES.iter()
                .all(|&a| (pos(&sol, i, a) - pos(&orig, i, a)).abs() <= t)
        });
        if ok {
            on = lv;
            break;
        }
    }
    g = solve_all(&orig, &g, &eqs, on);
    let gap_groups: Vec<_> = if on.contains(&Kind::Gap) {
        gap_groups
    } else {
        Vec::new()
    };
    let repeats: Vec<Rep> = if on.contains(&Kind::Repeat) {
        repeats
    } else {
        Vec::new()
    };
    let syms: Vec<_> = if on.contains(&Kind::Symmetric) {
        syms
    } else {
        Vec::new()
    };

    // 3.4 report with the final coordinates
    for a in AXES {
        for cl in &aligns[ax(a)] {
            let s0 = cl.members[0];
            let coord = pos(&g, s0, a) + anchor_off(cl.anchor, size(&g, s0, a));
            relations.push(Relation::Align {
                axis: a,
                anchor: cl.anchor,
                members: cl.members.clone(),
                coord: coord as f32,
            });
        }
    }
    for (a, order) in gap_groups {
        let gaps: Vec<f64> = order
            .windows(2)
            .map(|w| pos(&g, w[1], a) - pos(&g, w[0], a) - size(&g, w[0], a))
            .collect();
        let gap = gaps.iter().sum::<f64>() / gaps.len() as f64;
        relations.push(Relation::EqualGap {
            axis: a,
            members: order,
            gap: gap as f32,
        });
    }
    for rep in repeats {
        let k = rep.pairs.len() as f64;
        let dx = rep
            .pairs
            .iter()
            .map(|&(i, j)| g[j][0] - g[i][0])
            .sum::<f64>()
            / k;
        let dy = rep
            .pairs
            .iter()
            .map(|&(i, j)| g[j][1] - g[i][1])
            .sum::<f64>()
            / k;
        relations.push(Relation::Repeat {
            dx: dx as f32,
            dy: dy as f32,
            pairs: rep.pairs,
        });
    }
    for (a, s) in syms {
        relations.push(Relation::Symmetric {
            axis: a,
            center: s.center as f32,
            pairs: s.pairs,
            singles: s.singles,
        });
    }

    let rects = g
        .iter()
        .map(|v| Rect {
            x: v[0] as f32,
            y: v[1] as f32,
            w: v[2] as f32,
            h: v[3] as f32,
        })
        .collect();
    Outcome { rects, relations }
}
