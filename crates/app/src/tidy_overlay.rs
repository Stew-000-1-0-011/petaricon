//! Marks drawn over the canvas after a free Tidy, in design-area coordinates.

use tidy::Rect;
use tidy::free::{Axis, Relation};

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum MarkKind {
    /// Dashed horizontal line of length `w` starting at (`x`, `y`).
    HLine,
    /// Dashed vertical line of length `h` starting at (`x`, `y`).
    VLine,
    /// Small square centred on (`x`, `y`).
    Marker,
    /// Small dot centred on (`x`, `y`); a connector is a row of dots.
    Dot,
}

#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Mark {
    pub kind: MarkKind,
    pub x: f32,
    pub y: f32,
    pub w: f32,
    pub h: f32,
}

const DOT_STEP: f32 = 10.0;
const MAX_DOTS: usize = 60;

fn line(axis: Axis, at: f32, from: f32, to: f32) -> Mark {
    let len = (to - from).max(0.0);
    match axis {
        // Aligned x: a vertical line.
        Axis::X => Mark { kind: MarkKind::VLine, x: at, y: from, w: 0.0, h: len },
        Axis::Y => Mark { kind: MarkKind::HLine, x: from, y: at, w: len, h: 0.0 },
    }
}

fn span(rects: &[Rect], members: impl Iterator<Item = usize> + Clone, axis: Axis) -> (f32, f32) {
    // Extent along the axis perpendicular to the line.
    let (lo, hi) = members.clone().filter_map(|i| rects.get(i)).fold(
        (f32::INFINITY, f32::NEG_INFINITY),
        |(lo, hi), r| match axis {
            Axis::X => (lo.min(r.y), hi.max(r.y + r.h)),
            Axis::Y => (lo.min(r.x), hi.max(r.x + r.w)),
        },
    );
    if lo > hi { (0.0, 0.0) } else { (lo, hi) }
}

fn marker(x: f32, y: f32) -> Mark {
    Mark { kind: MarkKind::Marker, x, y, w: 8.0, h: 8.0 }
}

/// `origin` is the container's top-left in the design area; `rects` are the children's
/// design-area rectangles after the tidy; relations use container-relative coordinates.
pub fn marks(relations: &[Relation], origin: (f32, f32), rects: &[Rect]) -> Vec<Mark> {
    let mut out = Vec::new();
    let off = |axis: Axis| if axis == Axis::X { origin.0 } else { origin.1 };
    for r in relations {
        match r {
            Relation::Align { axis, anchor: _, members, coord } => {
                let (a, b) = span(rects, members.iter().copied(), *axis);
                out.push(line(*axis, off(*axis) + coord, a, b));
            }
            Relation::Symmetric { axis, center, pairs, singles } => {
                let all = pairs.iter().flat_map(|&(i, j)| [i, j]).chain(singles.iter().copied());
                let (a, b) = span(rects, all.collect::<Vec<_>>().into_iter(), *axis);
                out.push(line(*axis, off(*axis) + center, a, b));
            }
            Relation::EqualGap { axis, members, .. } => {
                for w in members.windows(2) {
                    let (Some(p), Some(q)) = (rects.get(w[0]), rects.get(w[1])) else { continue };
                    let (cx, cy) = match axis {
                        Axis::X => (
                            ((p.x + p.w) + q.x) / 2.0,
                            (p.y + p.h / 2.0 + q.y + q.h / 2.0) / 2.0,
                        ),
                        Axis::Y => (
                            (p.x + p.w / 2.0 + q.x + q.w / 2.0) / 2.0,
                            ((p.y + p.h) + q.y) / 2.0,
                        ),
                    };
                    out.push(marker(cx, cy));
                }
            }
            Relation::Repeat { pairs, .. } => {
                for &(i, j) in pairs {
                    let (Some(p), Some(q)) = (rects.get(i), rects.get(j)) else { continue };
                    let (dx, dy) = (q.x - p.x, q.y - p.y);
                    let n = ((dx.hypot(dy) / DOT_STEP).round() as usize).clamp(1, MAX_DOTS);
                    for k in 0..=n {
                        let t = k as f32 / n as f32;
                        out.push(Mark {
                            kind: MarkKind::Dot,
                            x: p.x + dx * t,
                            y: p.y + dy * t,
                            w: 4.0,
                            h: 4.0,
                        });
                    }
                }
            }
            Relation::SameSize { members, .. } => {
                for &i in members {
                    if let Some(r) = rects.get(i) {
                        out.push(marker(r.x + r.w - 6.0, r.y + r.h - 6.0));
                    }
                }
            }
        }
    }
    out
}

