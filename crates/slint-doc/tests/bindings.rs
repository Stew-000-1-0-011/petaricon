use slint_doc::*;

const SRC: &str = r#"import { Button } from "std-widgets.slint";
component Chip inherits Rectangle {
    in property <string> label;
    Text { text: root.label; }
}
export component Main inherits Window {
    // header
    title := Text { x: 8px; y: 4px; text: "見出し"; }
    VerticalLayout {
        Button { text: "A"; }
        for i in 3: Chip { label: "c"; }
    }
}
"#;

fn p(i: &[usize]) -> ElementPath {
    ElementPath {
        component: "Main".into(),
        indices: i.to_vec(),
    }
}

#[test]
fn replaces_existing_binding_only() {
    let mut d = Document::new("m.slint".into(), SRC.into());
    d.apply(Op::SetBindings {
        path: p(&[0]),
        bindings: vec![("x".into(), Some("20px".into()))],
    })
    .unwrap();
    assert_eq!(d.text(), SRC.replace("x: 8px;", "x: 20px;"));
}

#[test]
fn inserts_missing_binding_after_brace() {
    let mut d = Document::new("m.slint".into(), SRC.into());
    d.apply(Op::SetBindings {
        path: p(&[1, 0]),
        bindings: vec![("width".into(), Some("80px".into()))],
    })
    .unwrap();
    assert_eq!(
        d.text(),
        SRC.replace(
            r#"Button { text: "A"; }"#,
            r#"Button { width: 80px; text: "A"; }"#
        )
    );
}

#[test]
fn removes_binding_with_its_whitespace() {
    let mut d = Document::new("m.slint".into(), SRC.into());
    d.apply(Op::SetBindings {
        path: p(&[0]),
        bindings: vec![("y".into(), None)],
    })
    .unwrap();
    assert_eq!(d.text(), SRC.replace(" y: 4px;", ""));
}

#[test]
fn multibyte_text_survives() {
    let mut d = Document::new("m.slint".into(), SRC.into());
    d.apply(Op::SetBindings {
        path: p(&[0]),
        bindings: vec![
            ("x".into(), Some("1px".into())),
            ("y".into(), Some("2px".into())),
        ],
    })
    .unwrap();
    assert!(d.text().contains(r#"text: "見出し";"#));
    assert!(d.text().contains("x: 1px; y: 2px;"));
}

#[test]
fn undo_redo_and_dirty() {
    let mut d = Document::new("m.slint".into(), SRC.into());
    assert!(!d.is_dirty());
    d.apply(Op::SetBindings {
        path: p(&[0]),
        bindings: vec![("x".into(), Some("9px".into()))],
    })
    .unwrap();
    assert!(d.is_dirty());
    assert!(d.undo());
    assert_eq!(d.text(), SRC);
    assert!(!d.is_dirty());
    assert!(d.redo());
    assert!(d.text().contains("x: 9px;"));
    assert!(!d.redo());
}

#[test]
fn missing_element_is_error_and_text_unchanged() {
    let mut d = Document::new("m.slint".into(), SRC.into());
    assert!(matches!(
        d.apply(Op::SetBindings {
            path: p(&[9]),
            bindings: vec![]
        }),
        Err(EditError::ElementNotFound(_))
    ));
    assert_eq!(d.text(), SRC);
}

#[test]
fn missing_component_is_error() {
    let mut d = Document::new("m.slint".into(), SRC.into());
    let path = ElementPath {
        component: "Nope".into(),
        indices: vec![],
    };
    assert!(matches!(
        d.apply(Op::SetBindings {
            path,
            bindings: vec![]
        }),
        Err(EditError::ComponentNotFound(_))
    ));
}

#[test]
fn unchanged_text_is_not_recorded_and_returns_path() {
    let mut d = Document::new("m.slint".into(), SRC.into());
    let r = d
        .apply(Op::SetBindings {
            path: p(&[0]),
            bindings: vec![("x".into(), Some("8px".into()))],
        })
        .unwrap();
    assert_eq!(r, Some(p(&[0])));
    assert!(!d.is_dirty());
    assert!(!d.undo());
}

#[test]
fn replace_text_is_one_undo_step_and_saved_tracking() {
    let mut d = Document::new("m.slint".into(), SRC.into());
    d.replace_text("export component Main inherits Window { }\n".into());
    assert!(d.is_dirty());
    d.mark_saved();
    assert!(!d.is_dirty());
    assert!(d.undo());
    assert!(d.is_dirty());
    assert!(d.redo());
    assert!(!d.is_dirty());
    // a new edit after undo discards the saved state
    d.undo();
    d.replace_text("export component Main inherits Window { x: 1px; }\n".into());
    d.undo();
    assert!(d.is_dirty());
}

fn set(
    d: &mut Document,
    path: ElementPath,
    b: Vec<(&str, Option<&str>)>,
) -> Result<Option<ElementPath>, EditError> {
    d.apply(Op::SetBindings {
        path,
        bindings: b
            .into_iter()
            .map(|(n, v)| (n.to_string(), v.map(String::from)))
            .collect(),
    })
}

#[test]
fn duplicate_names_are_rejected_without_change() {
    let mut d = Document::new("m.slint".into(), SRC.into());
    for b in [
        vec![("x", Some("1px")), ("x", Some("2px"))],
        vec![("x", Some("1px")), ("x", None)],
        vec![("width", Some("1px")), ("width", Some("2px"))],
    ] {
        assert!(matches!(
            set(&mut d, p(&[0]), b),
            Err(EditError::InvalidTarget(_))
        ));
        assert_eq!(d.text(), SRC);
    }
}

#[test]
fn missing_bindings_inserted_in_given_order() {
    let mut d = Document::new("m.slint".into(), SRC.into());
    set(
        &mut d,
        p(&[1, 0]),
        vec![("width", Some("1px")), ("height", Some("2px"))],
    )
    .unwrap();
    assert_eq!(
        d.text(),
        SRC.replace(
            r#"Button { text: "A"; }"#,
            r#"Button { width: 1px; height: 2px; text: "A"; }"#
        )
    );
}

#[test]
fn removing_missing_binding_is_noop() {
    let mut d = Document::new("m.slint".into(), SRC.into());
    set(&mut d, p(&[0]), vec![("width", None)]).unwrap();
    assert_eq!(d.text(), SRC);
    assert!(!d.is_dirty());
}

#[test]
fn set_and_remove_in_same_call() {
    let mut d = Document::new("m.slint".into(), SRC.into());
    set(
        &mut d,
        p(&[0]),
        vec![("x", Some("1px")), ("y", None), ("width", Some("5px"))],
    )
    .unwrap();
    assert_eq!(
        d.text(),
        SRC.replace(r#"Text { x: 8px; y: 4px;"#, r#"Text { width: 5px; x: 1px;"#)
    );
}

#[test]
fn removing_own_line_binding_keeps_blank_lines() {
    let src = "export component Main inherits Window {\n    Rectangle {\n        x: 1px;\n\n        y: 2px;\n        width: 3px;\n    }\n}\n";
    let mut d = Document::new("m.slint".into(), src.into());
    set(&mut d, p(&[0]), vec![("y", None)]).unwrap();
    assert_eq!(d.text(), src.replace("        y: 2px;\n", ""));
}
