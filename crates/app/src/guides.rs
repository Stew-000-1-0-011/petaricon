//! Geometry of the layout guides drawn over the canvas (design-area coordinates).

use preview::Hit;
use slint_doc::{Binding, LayoutKind};
use tidy::{Direction, Rect};

/// A tappable number: `area` is the band it describes, `value` the number shown, `property`
/// the layout property tapping edits.
#[derive(Debug, Clone, PartialEq)]
pub struct GuideLabel {
    pub area: Rect,
    pub value: i32,
    pub property: String,
}

#[derive(Debug, Clone, PartialEq)]
pub struct Guides {
    pub outline: Rect,
    pub direction: Direction,
    pub padding: Vec<GuideLabel>,
    pub gaps: Vec<GuideLabel>,
    /// Reference line of the cross-axis alignment as `(x1, y1, x2, y2)`; none when stretched.
    pub align_line: Option<(f32, f32, f32, f32)>,
}

fn px(bindings: &[Binding], name: &str) -> Option<i32> {
    let b = bindings.iter().find(|b| b.name == name)?;
    let n = b.expr.trim().strip_suffix("px")?.trim();
    n.parse::<f32>().ok().map(|v| v.round() as i32)
}

fn word<'a>(bindings: &'a [Binding], name: &str) -> Option<&'a str> {
    bindings.iter().find(|b| b.name == name).map(|b| b.expr.trim())
}

fn rect(x: f32, y: f32, w: f32, h: f32) -> Rect {
    Rect { x, y, w: w.max(0.0), h: h.max(0.0) }
}

pub fn compute(layout: &Hit, kind: LayoutKind, children: &[Hit], bindings: &[Binding]) -> Guides {
    let direction = match kind {
        LayoutKind::Vertical => Direction::Vertical,
        _ => Direction::Horizontal,
    };
    let o = layout.rect;
    let all = px(bindings, "padding").unwrap_or(0);
    let side = |n: &str| px(bindings, n).unwrap_or(all);
    let (pl, pr, pt, pb) = (
        side("padding-left"),
        side("padding-right"),
        side("padding-top"),
        side("padding-bottom"),
    );
    let spacing = px(bindings, "spacing").unwrap_or(0);

    let (l, r, t, b) = (pl as f32, pr as f32, pt as f32, pb as f32);
    let mid_h = o.h - t - b;
    let padding = vec![
        GuideLabel { area: rect(o.x, o.y + t, l, mid_h), value: pl, property: "padding-left".into() },
        GuideLabel {
            area: rect(o.x + o.w - r, o.y + t, r, mid_h),
            value: pr,
            property: "padding-right".into(),
        },
        GuideLabel { area: rect(o.x, o.y, o.w, t), value: pt, property: "padding-top".into() },
        GuideLabel {
            area: rect(o.x, o.y + o.h - b, o.w, b),
            value: pb,
            property: "padding-bottom".into(),
        },
    ];

    let gaps = children
        .windows(2)
        .map(|p| {
            let (a, c) = (p[0].rect, p[1].rect);
            let area = match direction {
                Direction::Horizontal => {
                    let top = a.y.min(c.y);
                    rect(a.x + a.w, top, c.x - (a.x + a.w), (a.y + a.h).max(c.y + c.h) - top)
                }
                Direction::Vertical => {
                    let left = a.x.min(c.x);
                    rect(left, a.y + a.h, (a.x + a.w).max(c.x + c.w) - left, c.y - (a.y + a.h))
                }
            };
            GuideLabel { area, value: spacing, property: "spacing".into() }
        })
        .collect();

    let align_line = if children.is_empty() {
        None
    } else {
        let inner = rect(o.x + l, o.y + t, o.w - l - r, mid_h);
        let cross = |c: &Rect| match direction {
            Direction::Horizontal => (c.y, c.h),
            Direction::Vertical => (c.x, c.w),
        };
        let min = children.iter().map(|c| cross(&c.rect).0).fold(f32::INFINITY, f32::min);
        let max = children
            .iter()
            .map(|c| cross(&c.rect).0 + cross(&c.rect).1)
            .fold(f32::NEG_INFINITY, f32::max);
        let at = match word(bindings, "cross-axis-alignment") {
            Some("start") => Some(min),
            Some("end") => Some(max),
            Some("center") => Some((min + max) / 2.0),
            _ => None,
        };
        at.map(|v| match direction {
            Direction::Horizontal => (inner.x, v, inner.x + inner.w, v),
            Direction::Vertical => (v, inner.y, v, inner.y + inner.h),
        })
    };

    Guides { outline: o, direction, padding, gaps, align_line }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn hit(x: f32, y: f32, w: f32, h: f32) -> Hit {
        let r = rect(x, y, w, h);
        Hit { start: 0, rect: r, local: r }
    }

    fn b(name: &str, expr: &str) -> Binding {
        Binding { name: name.into(), expr: expr.into() }
    }

    #[test]
    fn guides_report_spacing_and_padding() {
        let layout = hit(0.0, 0.0, 200.0, 50.0);
        let kids = [hit(12.0, 12.0, 50.0, 26.0), hit(70.0, 12.0, 50.0, 26.0)];
        let bindings = [b("spacing", "8px"), b("padding-left", "12px"), b("padding", "4px")];
        let g = compute(&layout, LayoutKind::Horizontal, &kids, &bindings);
        assert_eq!(g.gaps.len(), 1);
        assert_eq!(g.gaps[0].value, 8);
        assert_eq!(g.gaps[0].area, rect(62.0, 12.0, 8.0, 26.0));
        assert!(g.padding.iter().any(|p| p.property == "padding-left" && p.value == 12));
        // `padding` applies to the sides without their own binding.
        assert!(g.padding.iter().any(|p| p.property == "padding-top" && p.value == 4));
        assert_eq!(g.direction, Direction::Horizontal);
        assert_eq!(g.align_line, None);
    }

    #[test]
    fn missing_bindings_are_zero_and_center_line_is_reported() {
        let layout = hit(10.0, 10.0, 100.0, 100.0);
        let kids = [hit(10.0, 10.0, 100.0, 20.0), hit(10.0, 40.0, 60.0, 20.0)];
        let g = compute(&layout, LayoutKind::Vertical, &kids, &[b("cross-axis-alignment", "center")]);
        assert_eq!(g.gaps[0].value, 0);
        assert!(g.padding.iter().all(|p| p.value == 0));
        assert_eq!(g.align_line, Some((60.0, 10.0, 60.0, 110.0)));
    }
}
