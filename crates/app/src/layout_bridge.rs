//! Turns a `tidy::Plan` into the layout description `slint-doc` writes.

use slint_doc::{LayoutKind, LayoutSpec};
use tidy::{CrossAlign, Direction, MainAlign, Plan};

pub fn layout_spec(plan: &Plan) -> LayoutSpec {
    let mut bindings: Vec<(String, String)> = vec![("spacing".into(), format!("{}px", plan.spacing))];
    let p = plan.padding;
    if p.left == p.right && p.left == p.top && p.left == p.bottom {
        bindings.push(("padding".into(), format!("{}px", p.left)));
    } else {
        for (name, v) in [
            ("padding-left", p.left),
            ("padding-right", p.right),
            ("padding-top", p.top),
            ("padding-bottom", p.bottom),
        ] {
            if v != 0 {
                bindings.push((name.into(), format!("{v}px")));
            }
        }
    }
    let main = match plan.main_align {
        MainAlign::Start => "start",
        MainAlign::Center => "center",
        MainAlign::End => "end",
    };
    bindings.push(("alignment".into(), main.into()));
    let cross = match plan.cross_align {
        CrossAlign::Stretch => "stretch",
        CrossAlign::Start => "start",
        CrossAlign::Center => "center",
        CrossAlign::End => "end",
    };
    bindings.push(("cross-axis-alignment".into(), cross.into()));
    LayoutSpec {
        kind: match plan.direction {
            Direction::Horizontal => LayoutKind::Horizontal,
            Direction::Vertical => LayoutKind::Vertical,
        },
        order: plan.order.clone(),
        bindings,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use tidy::Padding;

    fn plan(padding: Padding) -> Plan {
        Plan {
            direction: Direction::Horizontal,
            order: vec![1, 0],
            spacing: 8,
            padding,
            main_align: MainAlign::Start,
            cross_align: CrossAlign::Stretch,
            warnings: vec![],
        }
    }

    fn names(s: &LayoutSpec) -> Vec<&str> {
        s.bindings.iter().map(|(n, _)| n.as_str()).collect()
    }

    #[test]
    fn equal_padding_is_one_binding() {
        let s = layout_spec(&plan(Padding { left: 4, right: 4, top: 4, bottom: 4 }));
        assert_eq!(names(&s), ["spacing", "padding", "alignment", "cross-axis-alignment"]);
        assert_eq!(s.bindings[0].1, "8px");
        assert_eq!(s.bindings[1].1, "4px");
        assert_eq!(s.bindings[2].1, "start");
        assert_eq!(s.bindings[3].1, "stretch");
        assert_eq!(s.order, vec![1, 0]);
        assert_eq!(s.kind, LayoutKind::Horizontal);
    }

    #[test]
    fn uneven_padding_lists_nonzero_sides() {
        let s = layout_spec(&plan(Padding { left: 12, right: 0, top: 8, bottom: 0 }));
        assert_eq!(
            names(&s),
            ["spacing", "padding-left", "padding-top", "alignment", "cross-axis-alignment"]
        );
    }
}
