use slint_doc::*;

const SRC3: &str = "export component Main inherits Window {
    panel := Rectangle {
        background: #222;
        Text { x: 70px; y: 10px; text: \"右\"; }
        // left one
        Text { x: 10px; y: 12px; text: \"左\"; }
    }
}
";

const WRAPPED: &str = "export component Main inherits Window {
    panel := Rectangle {
        background: #222;
        HorizontalLayout {
            spacing: 8px;
            padding: 12px;
            // left one
            Text { text: \"左\"; }
            Text { text: \"右\"; }
        }
    }
}
";

fn p(i: &[usize]) -> ElementPath {
    ElementPath {
        component: "Main".into(),
        indices: i.to_vec(),
    }
}

fn wrap_op(order: Vec<usize>) -> Op {
    Op::WrapInLayout {
        container: p(&[0]),
        layout: LayoutSpec {
            kind: LayoutKind::Horizontal,
            order,
            bindings: vec![
                ("spacing".into(), "8px".into()),
                ("padding".into(), "12px".into()),
            ],
        },
    }
}

#[test]
fn wrap_horizontal() {
    let mut d = Document::new("m.slint".into(), SRC3.into());
    let r = d.apply(wrap_op(vec![1, 0])).unwrap();
    assert_eq!(r, Some(p(&[0, 0])));
    assert_eq!(d.text(), WRAPPED);
}

#[test]
fn unwrap_restores_positions() {
    let mut d = Document::new("m.slint".into(), SRC3.into());
    d.apply(wrap_op(vec![1, 0])).unwrap();
    let r = d
        .apply(Op::Unwrap {
            layout: p(&[0, 0]),
            positions: vec![(12, 12), (60, 12)],
        })
        .unwrap();
    assert_eq!(r, Some(p(&[0])));
    let expected = "export component Main inherits Window {
    panel := Rectangle {
        background: #222;
        // left one
        Text { x: 12px; y: 12px; text: \"左\"; }
        Text { x: 60px; y: 12px; text: \"右\"; }
    }
}
";
    // The layout's own attached comments (none here) go with it; the child's comment stays.
    assert_eq!(d.text(), expected);
    assert!(!d.text().contains("HorizontalLayout"));
}

#[test]
fn wrap_rejects_repeated_children() {
    let src = "export component Main inherits Window {
    Rectangle {
        for i in 3: Text { text: \"a\"; }
    }
}
";
    let mut d = Document::new("m.slint".into(), src.into());
    let r = d.apply(Op::WrapInLayout {
        container: p(&[0]),
        layout: LayoutSpec {
            kind: LayoutKind::Vertical,
            order: vec![0],
            bindings: vec![],
        },
    });
    assert!(matches!(r, Err(EditError::InvalidTarget(_))));
    assert_eq!(d.text(), src);
}

#[test]
fn wrap_rejects_bad_input() {
    let mut d = Document::new("m.slint".into(), SRC3.into());
    for order in [vec![0], vec![0, 0], vec![0, 2]] {
        assert!(matches!(
            d.apply(wrap_op(order)),
            Err(EditError::InvalidTarget(_))
        ));
    }
    let r = d.apply(Op::WrapInLayout {
        container: p(&[0]),
        layout: LayoutSpec {
            kind: LayoutKind::Grid,
            order: vec![0, 1],
            bindings: vec![],
        },
    });
    assert!(matches!(r, Err(EditError::InvalidTarget(_))));
    // No children.
    let mut d = Document::new("m.slint".into(), "component A inherits Window { }\n".into());
    let r = d.apply(Op::WrapInLayout {
        container: ElementPath {
            component: "A".into(),
            indices: vec![],
        },
        layout: LayoutSpec {
            kind: LayoutKind::Vertical,
            order: vec![],
            bindings: vec![],
        },
    });
    assert!(matches!(r, Err(EditError::InvalidTarget(_))));
    assert_eq!(d.text(), "component A inherits Window { }\n");
}

#[test]
fn wrap_then_undo_is_one_step() {
    let mut d = Document::new("m.slint".into(), SRC3.into());
    d.apply(wrap_op(vec![1, 0])).unwrap();
    assert!(d.undo());
    assert_eq!(d.text(), SRC3);
}

#[test]
fn unwrap_rejects_invalid() {
    let mut d = Document::new("m.slint".into(), SRC3.into());
    d.apply(wrap_op(vec![1, 0])).unwrap();
    // wrong count, non-layout, layout inside layout
    for (layout, positions) in [(p(&[0, 0]), vec![(0, 0)]), (p(&[0]), vec![(0, 0)])] {
        assert!(matches!(
            d.apply(Op::Unwrap { layout, positions }),
            Err(EditError::InvalidTarget(_))
        ));
    }
    let src = "export component Main inherits Window {
    HorizontalLayout {
        VerticalLayout {
            Text { text: \"a\"; }
        }
    }
}
";
    let mut d = Document::new("m.slint".into(), src.into());
    let r = d.apply(Op::Unwrap {
        layout: p(&[0, 0]),
        positions: vec![(0, 0)],
    });
    assert!(matches!(r, Err(EditError::InvalidTarget(_))));
    assert_eq!(d.text(), src);
}

fn vwrap(order: Vec<usize>) -> Op {
    Op::WrapInLayout {
        container: p(&[0]),
        layout: LayoutSpec {
            kind: LayoutKind::Vertical,
            order,
            bindings: vec![("spacing".into(), "4px".into())],
        },
    }
}

#[test]
fn wrap_vertical_keeps_trailing_comment_with_child() {
    let src = "export component Main inherits Window {
    panel := Rectangle {
        Text { x: 1px; y: 2px; text: \"a\"; } // label
        Text { x: 3px; text: \"b\"; }
    }
}
";
    let mut d = Document::new("m.slint".into(), src.into());
    assert_eq!(d.apply(vwrap(vec![1, 0])).unwrap(), Some(p(&[0, 0])));
    let expected = "export component Main inherits Window {
    panel := Rectangle {
        VerticalLayout {
            spacing: 4px;
            Text { text: \"b\"; }
            Text { text: \"a\"; } // label
        }
    }
}
";
    assert_eq!(d.text(), expected);
    assert!(d.undo());
    assert_eq!(d.text(), src);
}

#[test]
fn wrap_when_first_child_shares_its_line() {
    let src = "export component Main inherits Window {
    panel := Rectangle { Text { x: 1px; text: \"a\"; } Text { y: 3px; text: \"b\"; } }
}
";
    let mut d = Document::new("m.slint".into(), src.into());
    d.apply(vwrap(vec![0, 1])).unwrap();
    let expected = "export component Main inherits Window {
    panel := Rectangle { VerticalLayout {
        spacing: 4px;
        Text { text: \"a\"; }
        Text { text: \"b\"; }
    } }
}
";
    assert_eq!(d.text(), expected);
}

#[test]
fn unwrap_inline_and_trailing_comment_travel() {
    let src = "export component Main inherits Window {
    panel := Rectangle { VerticalLayout { Text { text: \"a\"; } // note
        Text { text: \"b\"; }
    } }
}
";
    let mut d = Document::new("m.slint".into(), src.into());
    d.apply(Op::Unwrap {
        layout: p(&[0, 0]),
        positions: vec![(1, 2), (3, 4)],
    })
    .unwrap();
    let expected = "export component Main inherits Window {
    panel := Rectangle { Text { x: 1px; y: 2px; text: \"a\"; } // note
    Text { x: 3px; y: 4px; text: \"b\"; } }
}
";
    assert_eq!(d.text(), expected);
}

#[test]
fn unwrap_keeps_all_comments() {
    let src = "export component Main inherits Window {
    panel := Rectangle {
        // about the layout
        HorizontalLayout { // trailing brace
            spacing: 8px;
            // loose

            Text { text: \"a\"; } // label a
            // for b
            Text { text: \"b\"; }
            // at the end
        } // after
    }
}
";
    let mut d = Document::new("m.slint".into(), src.into());
    d.apply(Op::Unwrap {
        layout: p(&[0, 0]),
        positions: vec![(1, 2), (3, 4)],
    })
    .unwrap();
    let expected = "export component Main inherits Window {
    panel := Rectangle {
        // about the layout
        Text { x: 1px; y: 2px; text: \"a\"; } // label a
        // for b
        Text { x: 3px; y: 4px; text: \"b\"; }
        // trailing brace
        // loose
        // at the end
        // after
    }
}
";
    assert_eq!(d.text(), expected);
}

#[test]
fn unwrap_then_undo_is_one_step() {
    let mut d = Document::new("m.slint".into(), WRAPPED.into());
    d.apply(Op::Unwrap {
        layout: p(&[0, 0]),
        positions: vec![(1, 1), (2, 2)],
    })
    .unwrap();
    assert!(d.undo());
    assert_eq!(d.text(), WRAPPED);
}

#[test]
fn unwrap_inline_last_child_comment_does_not_swallow_code() {
    let src = "export component Main inherits Window {
    panel := Rectangle { VerticalLayout { Text { text: \"a\"; }
        Text { text: \"b\"; } // c
    } }
}
";
    let mut d = Document::new("m.slint".into(), src.into());
    d.apply(Op::Unwrap {
        layout: p(&[0, 0]),
        positions: vec![(1, 2), (3, 4)],
    })
    .unwrap();
    let expected = "export component Main inherits Window {
    panel := Rectangle { Text { x: 1px; y: 2px; text: \"a\"; }
    Text { x: 3px; y: 4px; text: \"b\"; } // c
    }
}
";
    assert_eq!(d.text(), expected);
    assert!(d.element(&p(&[0, 1])).is_some());
}
