use slint_doc::*;

const SRC2: &str = "export component Main inherits Window {
    box := Rectangle {
        // the stop button
        Text { x: 4px; y: 4px; text: \"停止\"; }

        Rectangle { x: 50px; y: 4px; width: 10px; height: 10px; }
    }
    VerticalLayout {
        Text { text: \"a\"; }
    }
}
";

fn p(i: &[usize]) -> ElementPath {
    ElementPath {
        component: "Main".into(),
        indices: i.to_vec(),
    }
}

fn doc() -> Document {
    Document::new("m.slint".into(), SRC2.into())
}

#[test]
fn insert_at_end_of_container() {
    let mut d = doc();
    let r = d
        .apply(Op::InsertElement {
            parent: p(&[0]),
            index: None,
            source: "Button { }".into(),
        })
        .unwrap();
    assert_eq!(r, Some(p(&[0, 2])));
    assert_eq!(
        d.text(),
        SRC2.replace(
            "height: 10px; }\n",
            "height: 10px; }\n        Button { }\n"
        )
    );
}

#[test]
fn insert_first_and_into_empty() {
    let mut d = doc();
    let r = d
        .apply(Op::InsertElement {
            parent: p(&[1]),
            index: Some(0),
            source: "Button {\n    text: \"x\";\n}".into(),
        })
        .unwrap();
    assert_eq!(r, Some(p(&[1, 0])));
    assert!(d.text().contains(
        "    VerticalLayout {\n        Button {\n            text: \"x\";\n        }\n        Text { text: \"a\"; }"
    ));
    let mut d = Document::new("m.slint".into(), "component A inherits Window { }\n".into());
    d.apply(Op::InsertElement {
        parent: ElementPath { component: "A".into(), indices: vec![] },
        index: None,
        source: "Text { }".into(),
    })
    .unwrap();
    assert_eq!(d.text(), "component A inherits Window {\n    Text { }\n}\n");
}

#[test]
fn insert_rejects_bad_source() {
    let mut d = doc();
    for s in ["", "Button { } Button { }", "Button {"] {
        assert!(matches!(
            d.apply(Op::InsertElement { parent: p(&[0]), index: None, source: s.into() }),
            Err(EditError::InvalidTarget(_))
        ));
    }
    assert_eq!(d.text(), SRC2);
}

#[test]
fn remove_takes_leading_comment() {
    let mut d = doc();
    assert_eq!(d.apply(Op::RemoveElement { path: p(&[0, 0]) }).unwrap(), None);
    assert_eq!(
        d.text(),
        SRC2.replace(
            "        // the stop button\n        Text { x: 4px; y: 4px; text: \"停止\"; }\n",
            ""
        )
    );
    assert!(d.text().contains("{\n\n        Rectangle {"));
}

#[test]
fn remove_root_is_invalid() {
    let mut d = doc();
    assert!(matches!(
        d.apply(Op::RemoveElement { path: p(&[]) }),
        Err(EditError::InvalidTarget(_))
    ));
}

#[test]
fn move_into_layout_strips_xy() {
    let mut d = doc();
    let r = d
        .apply(Op::MoveElement { path: p(&[0, 1]), new_parent: p(&[1]), index: Some(0) })
        .unwrap();
    assert_eq!(r, Some(p(&[1, 0])));
    assert_eq!(
        d.text(),
        "export component Main inherits Window {
    box := Rectangle {
        // the stop button
        Text { x: 4px; y: 4px; text: \"停止\"; }

    }
    VerticalLayout {
        Rectangle { width: 10px; height: 10px; }
        Text { text: \"a\"; }
    }
}
"
    );
}

#[test]
fn move_keeps_id() {
    let mut d = doc();
    let r = d
        .apply(Op::MoveElement { path: p(&[0]), new_parent: p(&[1]), index: None })
        .unwrap();
    assert_eq!(r, Some(p(&[0, 1])));
    assert!(d.text().contains("box := Rectangle {"));
    assert_eq!(d.element(&p(&[0, 1])).unwrap().id.as_deref(), Some("box"));
    assert!(d.text().contains("        box := Rectangle {\n            // the stop button\n            Text {"));
    assert_eq!(d.tree("Main").unwrap().children.len(), 1);
}

#[test]
fn reorder_within_parent() {
    let mut d = doc();
    let r = d
        .apply(Op::MoveElement { path: p(&[0, 0]), new_parent: p(&[0]), index: Some(1) })
        .unwrap();
    assert_eq!(r, Some(p(&[0, 1])));
    assert_eq!(d.element(&p(&[0, 1])).unwrap().bindings.len(), 3);
    assert_eq!(d.element(&p(&[0, 0])).unwrap().bindings.len(), 4);
    assert!(d.text().contains("// the stop button"));
}

#[test]
fn cannot_move_into_descendant() {
    let mut d = doc();
    for (from, to) in [(vec![0], vec![0, 1]), (vec![0], vec![0])] {
        assert!(matches!(
            d.apply(Op::MoveElement { path: p(&from), new_parent: p(&to), index: None }),
            Err(EditError::InvalidTarget(_))
        ));
    }
    assert!(matches!(
        d.apply(Op::MoveElement { path: p(&[]), new_parent: p(&[0]), index: None }),
        Err(EditError::InvalidTarget(_))
    ));
    assert_eq!(d.text(), SRC2);
}

#[test]
fn undo_restores_after_move() {
    let mut d = doc();
    d.apply(Op::MoveElement { path: p(&[0]), new_parent: p(&[1]), index: None }).unwrap();
    assert!(d.undo());
    assert_eq!(d.text(), SRC2);
}
