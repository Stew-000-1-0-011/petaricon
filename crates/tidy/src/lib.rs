//! Infers a stack layout (direction, order, spacing, padding, alignment)
//! from the absolute positions of a container's children.

pub mod free;

#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Rect {
    pub x: f32,
    pub y: f32,
    pub w: f32,
    pub h: f32,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Direction {
    Horizontal,
    Vertical,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum MainAlign {
    Start,
    Center,
    End,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum CrossAlign {
    Stretch,
    Start,
    Center,
    End,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Padding {
    pub left: i32,
    pub right: i32,
    pub top: i32,
    pub bottom: i32,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Warning {
    AmbiguousDirection,
    Overlap,
}

#[derive(Debug, Clone, PartialEq)]
pub struct Plan {
    pub direction: Direction,
    pub order: Vec<usize>,
    pub spacing: i32,
    pub padding: Padding,
    pub main_align: MainAlign,
    pub cross_align: CrossAlign,
    pub warnings: Vec<Warning>,
}

fn snap(v: f32, grid: i32) -> i32 {
    let g = grid.max(1) as f32;
    (((v / g) + 0.5).floor() * g).max(0.0) as i32
}

/// Per-axis view of a rect: (main_start, main_size, cross_start, cross_size).
fn axes(r: &Rect, d: Direction) -> (f32, f32, f32, f32) {
    match d {
        Direction::Horizontal => (r.x, r.w, r.y, r.h),
        Direction::Vertical => (r.y, r.h, r.x, r.w),
    }
}

/// True when, sorted along `d`, every adjacent pair overlaps on the cross axis.
fn is_stack(children: &[Rect], d: Direction) -> bool {
    let mut idx: Vec<usize> = (0..children.len()).collect();
    idx.sort_by(|&a, &b| axes(&children[a], d).0.total_cmp(&axes(&children[b], d).0));
    idx.windows(2).all(|p| {
        let (_, _, cs0, cz0) = axes(&children[p[0]], d);
        let (_, _, cs1, cz1) = axes(&children[p[1]], d);
        cs0.max(cs1) < (cs0 + cz0).min(cs1 + cz1)
    })
}

fn center_range(children: &[Rect], d: Direction) -> f32 {
    let centers = children.iter().map(|r| {
        let (s, z, ..) = axes(r, d);
        s + z / 2.0
    });
    let min = centers.clone().fold(f32::INFINITY, f32::min);
    let max = centers.fold(f32::NEG_INFINITY, f32::max);
    max - min
}

pub fn plan(container_w: f32, container_h: f32, children: &[Rect], grid: i32) -> Option<Plan> {
    if children.is_empty() {
        return None;
    }
    let mut warnings = Vec::new();

    let direction = if children.len() == 1 {
        Direction::Vertical
    } else {
        let h = is_stack(children, Direction::Horizontal);
        let v = is_stack(children, Direction::Vertical);
        match (h, v) {
            (true, false) => Direction::Horizontal,
            (false, true) => Direction::Vertical,
            _ => {
                if !h && !v {
                    warnings.push(Warning::AmbiguousDirection);
                }
                if center_range(children, Direction::Horizontal)
                    > center_range(children, Direction::Vertical)
                {
                    Direction::Horizontal
                } else {
                    Direction::Vertical
                }
            }
        }
    };

    let (container_main, container_cross) = match direction {
        Direction::Horizontal => (container_w, container_h),
        Direction::Vertical => (container_h, container_w),
    };

    let mut order: Vec<usize> = (0..children.len()).collect();
    order.sort_by(|&a, &b| {
        axes(&children[a], direction)
            .0
            .total_cmp(&axes(&children[b], direction).0)
    });

    // Spacing: median of gaps, overlaps clamped to 0.
    let mut overlap = false;
    let mut gaps: Vec<f32> = order
        .windows(2)
        .map(|p| {
            let (s0, z0, ..) = axes(&children[p[0]], direction);
            let (s1, ..) = axes(&children[p[1]], direction);
            let gap = s1 - (s0 + z0);
            if gap < 0.0 {
                overlap = true;
                0.0
            } else {
                gap
            }
        })
        .collect();
    if overlap {
        warnings.push(Warning::Overlap);
    }
    gaps.sort_by(f32::total_cmp);
    let spacing = if gaps.is_empty() {
        0
    } else {
        let n = gaps.len();
        let median = if n % 2 == 1 {
            gaps[n / 2]
        } else {
            (gaps[n / 2 - 1] + gaps[n / 2]) / 2.0
        };
        snap(median, grid)
    };

    // Main axis alignment and padding.
    let first = axes(&children[order[0]], direction);
    let last = axes(&children[*order.last().unwrap()], direction);
    let lead = first.0;
    let trail = container_main - (last.0 + last.1);
    let main_align = if lead > 0.0
        && trail > 0.0
        && (lead - trail).abs() <= (grid as f32).max(0.1 * container_main)
    {
        MainAlign::Center
    } else if lead <= trail {
        MainAlign::Start
    } else {
        MainAlign::End
    };
    let m = lead.min(trail);
    let (main_pad_a, main_pad_b) = match main_align {
        MainAlign::Start => (lead, m),
        MainAlign::End => (m, trail),
        MainAlign::Center => (m, m),
    };

    // Cross axis alignment and padding.
    let cross: Vec<(f32, f32)> = children
        .iter()
        .map(|r| {
            let (_, _, s, z) = axes(r, direction);
            (s, z)
        })
        .collect();
    let min_start = cross.iter().map(|c| c.0).fold(f32::INFINITY, f32::min);
    let max_end = cross
        .iter()
        .map(|c| c.0 + c.1)
        .fold(f32::NEG_INFINITY, f32::max);
    let cross_start = min_start;
    let cross_end = container_cross - max_end;
    let band = max_end - min_start;
    let avail = container_cross - cross_start - cross_end;

    let cross_align =
        if children.len() >= 2 && cross.iter().all(|c| c.1 >= 0.9 * band) && band >= 0.9 * avail {
            CrossAlign::Stretch
        } else {
            let mean_center =
                cross.iter().map(|c| c.0 + c.1 / 2.0).sum::<f32>() / cross.len() as f32;
            let start_dev: f32 = cross.iter().map(|c| c.0 - min_start).sum();
            let end_dev: f32 = cross.iter().map(|c| max_end - (c.0 + c.1)).sum();
            let center_dev: f32 = cross
                .iter()
                .map(|c| (c.0 + c.1 / 2.0 - mean_center).abs())
                .sum();
            let mut best = (CrossAlign::Start, start_dev);
            if center_dev < best.1 {
                best = (CrossAlign::Center, center_dev);
            }
            if end_dev < best.1 {
                best = (CrossAlign::End, end_dev);
            }
            best.0
        };

    let (ma, mb) = (snap(main_pad_a, grid), snap(main_pad_b, grid));
    let (ca, cb) = (snap(cross_start, grid), snap(cross_end, grid));
    let padding = match direction {
        Direction::Horizontal => Padding {
            left: ma,
            right: mb,
            top: ca,
            bottom: cb,
        },
        Direction::Vertical => Padding {
            left: ca,
            right: cb,
            top: ma,
            bottom: mb,
        },
    };

    Some(Plan {
        direction,
        order,
        spacing,
        padding,
        main_align,
        cross_align,
        warnings,
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    fn r(x: f32, y: f32, w: f32, h: f32) -> Rect {
        Rect { x, y, w, h }
    }

    #[test]
    fn empty_is_none() {
        assert_eq!(plan(300., 200., &[], 4), None);
    }

    #[test]
    fn single_child_is_vertical_without_warning() {
        let p = plan(300., 200., &[r(20., 30., 100., 40.)], 4).unwrap();
        assert_eq!(p.direction, Direction::Vertical);
        assert!(p.warnings.is_empty());
        assert_eq!(p.order, vec![0]);
    }

    #[test]
    fn horizontal_row_with_uneven_gaps() {
        let kids = [
            r(150., 22., 40., 30.),
            r(16., 20., 40., 30.),
            r(66., 18., 40., 30.),
            r(197., 21., 40., 30.),
        ];
        let p = plan(300., 80., &kids, 4).unwrap();
        assert_eq!(p.direction, Direction::Horizontal);
        assert_eq!(p.order, vec![1, 2, 0, 3]);
        assert_eq!(p.spacing, 12);
        assert_eq!(
            p.padding,
            Padding {
                left: 16,
                right: 16,
                top: 20,
                bottom: 28
            }
        );
        assert_eq!(p.main_align, MainAlign::Start);
        assert_eq!(p.cross_align, CrossAlign::Center);
    }

    #[test]
    fn vertical_centered_column() {
        let kids = [
            r(100., 20., 100., 30.),
            r(80., 58., 140., 30.),
            r(110., 96., 80., 30.),
        ];
        let p = plan(300., 146., &kids, 4).unwrap();
        assert_eq!(p.direction, Direction::Vertical);
        assert_eq!(p.spacing, 8);
        assert_eq!(
            p.padding,
            Padding {
                left: 80,
                right: 80,
                top: 20,
                bottom: 20
            }
        );
        assert_eq!(p.main_align, MainAlign::Center);
        assert_eq!(p.cross_align, CrossAlign::Center);
    }

    #[test]
    fn full_width_rows_stretch() {
        let kids = [r(12., 10., 276., 30.), r(12., 48., 275., 30.)];
        let p = plan(300., 200., &kids, 4).unwrap();
        assert_eq!(p.cross_align, CrossAlign::Stretch);
    }

    #[test]
    fn overlap_warns_and_clamps() {
        let kids = [r(10., 10., 50., 30.), r(50., 12., 50., 30.)];
        let p = plan(200., 60., &kids, 4).unwrap();
        assert_eq!(p.spacing, 0);
        assert!(p.warnings.contains(&Warning::Overlap));
    }

    #[test]
    fn grid_layout_is_ambiguous() {
        let kids = [
            r(10., 10., 40., 40.),
            r(60., 10., 40., 40.),
            r(10., 60., 40., 40.),
            r(60., 60., 40., 40.),
        ];
        let p = plan(200., 200., &kids, 4).unwrap();
        assert!(p.warnings.contains(&Warning::AmbiguousDirection));
    }
}
