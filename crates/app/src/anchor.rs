//! Anchors: where a free-placed child sticks to its container, per axis, written as plain
//! Slint expressions (the `.slint` text is the only record). See the anchors design spec.

/// Horizontal (`x` / `width` / `parent.width`) or vertical (`y` / `height` / `parent.height`).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Axis {
    H,
    V,
}

/// An axis' anchor. `Start` is left / top, `End` right / bottom, `Stretch` left+right /
/// top+bottom. The numbers are px: A distance to the start edge, B to the end edge, C the
/// offset from the centre. `Expr` is any other expression (never rewritten).
#[derive(Debug, Clone, Copy, PartialEq)]
pub enum Anchor {
    Start(f64),
    End(f64),
    Stretch(f64, f64),
    Center(f64),
    Expr,
}

pub type HAnchor = Anchor;
pub type VAnchor = Anchor;

/// An anchor without its numbers: what the user chooses.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Kind {
    Start,
    End,
    Stretch,
    Center,
}

impl Axis {
    pub fn pos(self) -> &'static str {
        match self {
            Axis::H => "x",
            Axis::V => "y",
        }
    }
    pub fn size(self) -> &'static str {
        match self {
            Axis::H => "width",
            Axis::V => "height",
        }
    }
    fn parent(self) -> &'static str {
        match self {
            Axis::H => "parent.width",
            Axis::V => "parent.height",
        }
    }
    fn own(self) -> &'static str {
        match self {
            Axis::H => "self.width",
            Axis::V => "self.height",
        }
    }
}

impl Anchor {
    pub fn kind(&self) -> Option<Kind> {
        Some(match self {
            Anchor::Start(_) => Kind::Start,
            Anchor::End(_) => Kind::End,
            Anchor::Stretch(..) => Kind::Stretch,
            Anchor::Center(_) => Kind::Center,
            Anchor::Expr => return None,
        })
    }
}

/// `-12.5px` at the start of `s`: the value and what follows.
fn number(s: &str) -> Option<(f64, &str)> {
    let digits = |t: &str| t.bytes().take_while(u8::is_ascii_digit).count();
    let mut end = usize::from(s.starts_with('-'));
    let n = digits(&s[end..]);
    if n == 0 {
        return None;
    }
    end += n;
    if s[end..].starts_with('.') {
        let f = digits(&s[end + 1..]);
        if f == 0 {
            return None;
        }
        end += 1 + f;
    }
    let value = s[..end].parse().ok()?;
    Some((value, s[end..].strip_prefix("px")?))
}

pub fn plain_length(s: &str) -> Option<f64> {
    number(s).filter(|(_, rest)| rest.is_empty()).map(|(v, _)| v)
}

/// The anchor a pair of `x` / `width` (or `y` / `height`) expressions stands for. Empty
/// strings are unwritten bindings. Spaces are ignored.
pub fn read(axis: Axis, pos: &str, size: &str) -> Anchor {
    let strip = |s: &str| s.chars().filter(|c| !c.is_whitespace()).collect::<String>();
    let (p, s) = (strip(pos), strip(size));
    let parent = axis.parent();
    // A size that is neither empty nor a plain length.
    let odd_size = !s.is_empty() && plain_length(&s).is_none();
    if p.is_empty() {
        return if s.contains(parent) { Anchor::Expr } else { Anchor::Start(0.0) };
    }
    if let Some(a) = plain_length(&p) {
        if let Some(rest) = s.strip_prefix(parent).and_then(|r| r.strip_prefix('-')) {
            let stretch = number(rest).and_then(|(a2, r)| {
                let (b, tail) = number(r.strip_prefix('-')?)?;
                (a2 == a && tail.is_empty()).then_some(b)
            });
            return stretch.map_or(Anchor::Expr, |b| Anchor::Stretch(a, b));
        }
        return if s.contains(parent) { Anchor::Expr } else { Anchor::Start(a) };
    }
    if odd_size {
        return Anchor::Expr;
    }
    let end = format!("{}-{}-", parent, axis.own());
    if let Some(rest) = p.strip_prefix(&end) {
        return plain_length(rest).map_or(Anchor::Expr, Anchor::End);
    }
    let centre = format!("({}-{})/2", parent, axis.own());
    if let Some(rest) = p.strip_prefix(&centre) {
        if rest.is_empty() {
            return Anchor::Center(0.0);
        }
        let c = rest
            .strip_prefix('+')
            .and_then(plain_length)
            .or_else(|| rest.strip_prefix('-').and_then(plain_length).map(|v| -v));
        return c.map_or(Anchor::Expr, Anchor::Center);
    }
    Anchor::Expr
}

fn px(v: f64) -> String {
    format!("{}px", v.round() as i64)
}

/// The `x` text and, for `Stretch`, the `width` text. `None` for `Expr` (nothing to write).
/// Numbers are rounded to whole px.
pub fn write(axis: Axis, anchor: Anchor) -> Option<(String, Option<String>)> {
    let (parent, own) = (axis.parent(), axis.own());
    Some(match anchor {
        Anchor::Start(a) => (px(a), None),
        Anchor::End(b) => (format!("{parent} - {own} - {}", px(b)), None),
        Anchor::Stretch(a, b) => (px(a), Some(format!("{parent} - {} - {}", px(a), px(b)))),
        Anchor::Center(c) => {
            let base = format!("({parent} - {own}) / 2");
            let c = c.round() as i64;
            let text = match c {
                0 => base,
                c if c > 0 => format!("{base} + {c}px"),
                c => format!("{base} - {}px", -c),
            };
            (text, None)
        }
        Anchor::Expr => return None,
    })
}

/// The anchor of `kind` that keeps an element at `pos` with `size` inside a parent of length
/// `parent` where it is. Numbers are whole px.
pub fn solve(kind: Kind, pos: f64, size: f64, parent: f64) -> Anchor {
    let a = pos.round();
    match kind {
        Kind::Start => Anchor::Start(a),
        Kind::End => Anchor::End((parent - pos - size).round()),
        Kind::Stretch => Anchor::Stretch(a, (parent - a - size.round()).round()),
        Kind::Center => Anchor::Center((pos - (parent - size) / 2.0).round()),
    }
}

/// Where an anchor puts an element of `size` in a parent of length `parent`: `(pos, size)`.
pub fn place(anchor: Anchor, size: f64, parent: f64) -> Option<(f64, f64)> {
    Some(match anchor {
        Anchor::Start(a) => (a, size),
        Anchor::End(b) => (parent - size - b, size),
        Anchor::Stretch(a, b) => (a, parent - a - b),
        Anchor::Center(c) => ((parent - size) / 2.0 + c, size),
        Anchor::Expr => return None,
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    const H: Axis = Axis::H;
    const V: Axis = Axis::V;

    fn texts(axis: Axis, a: Anchor) -> (String, String) {
        let (p, s) = write(axis, a).unwrap();
        (p, s.unwrap_or_default())
    }

    #[test]
    fn writes_the_eight_forms() {
        assert_eq!(texts(H, Anchor::Start(12.0)), ("12px".into(), "".into()));
        assert_eq!(texts(H, Anchor::End(8.0)).0, "parent.width - self.width - 8px");
        assert_eq!(
            texts(H, Anchor::Stretch(4.0, 6.0)),
            ("4px".into(), "parent.width - 4px - 6px".into())
        );
        assert_eq!(texts(H, Anchor::Center(0.0)).0, "(parent.width - self.width) / 2");
        assert_eq!(texts(H, Anchor::Center(5.0)).0, "(parent.width - self.width) / 2 + 5px");
        assert_eq!(texts(H, Anchor::Center(-5.0)).0, "(parent.width - self.width) / 2 - 5px");
        assert_eq!(texts(V, Anchor::End(8.0)).0, "parent.height - self.height - 8px");
        assert_eq!(
            texts(V, Anchor::Stretch(4.0, 6.0)),
            ("4px".into(), "parent.height - 4px - 6px".into())
        );
        assert_eq!(texts(V, Anchor::Center(0.0)).0, "(parent.height - self.height) / 2");
        assert_eq!(write(H, Anchor::Expr), None);
    }

    #[test]
    fn write_then_read_round_trips() {
        let all = [
            Anchor::Start(10.0),
            Anchor::Start(-10.0),
            Anchor::End(8.0),
            Anchor::End(-8.0),
            Anchor::Stretch(4.0, 6.0),
            Anchor::Stretch(-4.0, -6.0),
            Anchor::Center(0.0),
            Anchor::Center(7.0),
            Anchor::Center(-7.0),
        ];
        for axis in [H, V] {
            for a in all {
                let (p, s) = write(axis, a).unwrap();
                assert_eq!(read(axis, &p, &s.clone().unwrap_or_default()), a, "{p} / {s:?}");
            }
        }
    }

    #[test]
    fn read_tolerates_spacing_and_decimals() {
        assert_eq!(read(H, "  10px ", ""), Anchor::Start(10.0));
        assert_eq!(read(H, "10.5px", "100px"), Anchor::Start(10.5));
        assert_eq!(read(H, "", ""), Anchor::Start(0.0));
        assert_eq!(read(H, "parent.width-self.width-8px", "20px"), Anchor::End(8.0));
        assert_eq!(read(H, "parent.width  -  self.width  -  -8px", ""), Anchor::End(-8.0));
        assert_eq!(read(H, "(parent.width-self.width)/2+0px", ""), Anchor::Center(0.0));
        assert_eq!(read(H, "( parent.width - self.width ) / 2 + -3px", ""), Anchor::Center(-3.0));
        assert_eq!(read(H, "(parent.width - self.width) / 2 - -3px", ""), Anchor::Center(3.0));
        assert_eq!(read(H, "4px", "parent.width-4px-6px"), Anchor::Stretch(4.0, 6.0));
        assert_eq!(read(V, "4px", "parent.height - 4px - 6px"), Anchor::Stretch(4.0, 6.0));
    }

    #[test]
    fn other_expressions_are_expr() {
        for (p, s) in [
            ("root.x", ""),
            ("parent.width / 2", ""),
            ("parent.width - self.width", ""),
            ("parent.width - self.width - 8", ""),
            ("parent.height - self.height - 8px", ""),
            ("10px + 5px", ""),
            ("10", ""),
            ("(parent.width - self.width) / 2 + 1px + 2px", ""),
            // Stretch needs the same A in both places
            ("4px", "parent.width - 5px - 6px"),
            ("4px", "parent.width"),
            ("", "parent.width - 4px"),
            // the size must stay a plain length next to End / Center
            ("parent.width - self.width - 8px", "root.w"),
            ("(parent.width - self.width) / 2", "parent.width / 2"),
        ] {
            assert_eq!(read(H, p, s), Anchor::Expr, "{p} / {s}");
        }
        // The other axis' names do not count.
        assert_eq!(read(V, "parent.width - self.width - 8px", ""), Anchor::Expr);
        // A size expression that does not involve the parent stays a Start.
        assert_eq!(read(H, "10px", "root.w"), Anchor::Start(10.0));
    }

    #[test]
    fn solve_keeps_the_rectangle() {
        // 20 wide at 30 in a parent of 200
        assert_eq!(solve(Kind::Start, 30.0, 20.0, 200.0), Anchor::Start(30.0));
        assert_eq!(solve(Kind::End, 30.0, 20.0, 200.0), Anchor::End(150.0));
        assert_eq!(solve(Kind::Stretch, 30.0, 20.0, 200.0), Anchor::Stretch(30.0, 150.0));
        assert_eq!(solve(Kind::Center, 30.0, 20.0, 200.0), Anchor::Center(-60.0));
        for kind in [Kind::Start, Kind::End, Kind::Stretch, Kind::Center] {
            let a = solve(kind, 30.0, 20.0, 200.0);
            assert_eq!(place(a, 20.0, 200.0), Some((30.0, 20.0)), "{kind:?}");
        }
        assert_eq!(place(Anchor::End(10.0), 20.0, 300.0), Some((270.0, 20.0)));
        assert_eq!(place(Anchor::Stretch(10.0, 10.0), 20.0, 300.0), Some((10.0, 280.0)));
        assert_eq!(place(Anchor::Center(0.0), 20.0, 300.0), Some((140.0, 20.0)));
    }
}
