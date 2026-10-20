use slint_doc::*;

fn p(i: &[usize]) -> ElementPath {
    ElementPath {
        component: "Main".into(),
        indices: i.to_vec(),
    }
}

fn doc(src: &str) -> Document {
    Document::new("t.slint".into(), src.into())
}

const SRC: &str = "export component Main inherits Window {\n    ok := Rectangle { x: 5px; width: 40px; background: red; Text { text: \"OK\"; } }\n}\n";

#[test]
fn extract_keeps_geometry_and_id() {
    let mut d = doc(SRC);
    let r = d
        .apply(Op::ExtractComponent {
            path: p(&[0]),
            name: "OkButton".into(),
        })
        .unwrap();
    assert_eq!(r, Some(p(&[0])));
    let t = d.text().to_string();
    let decl = t.find("component OkButton inherits Rectangle {").unwrap();
    assert!(decl < t.find("export component Main").unwrap());
    let body = &t[decl..t.find("export component Main").unwrap()];
    assert!(body.contains("background: red;") && body.contains("Text { text: \"OK\"; }"));
    assert!(!body.contains("x:") && !body.contains("width:"));
    assert!(t.contains("    ok := OkButton { x: 5px; width: 40px; }\n"));
    assert!(body.ends_with("}\n\n"));
    d.undo();
    assert_eq!(d.text(), SRC);
}

#[test]
fn extract_multiline_body_and_no_geometry() {
    let src = "component Main inherits Window {\n    // note\n    Rectangle {\n        background: red;\n        Text {\n            text: \"a\";\n        }\n    }\n}\n";
    let mut d = doc(src);
    d.apply(Op::ExtractComponent {
        path: p(&[0]),
        name: "Box".into(),
    })
    .unwrap();
    assert_eq!(
        d.text(),
        "component Box inherits Rectangle {\n    background: red;\n    Text {\n        text: \"a\";\n    }\n}\n\ncomponent Main inherits Window {\n    // note\n    Box { }\n}\n"
    );
}

#[test]
fn extract_rejects() {
    let mut d = doc(SRC);
    for (path, name) in [
        (p(&[0]), "Main"),
        (p(&[0]), "1bad"),
        (p(&[0]), ""),
        (p(&[]), "Foo"),
    ] {
        let r = d.apply(Op::ExtractComponent {
            path,
            name: name.into(),
        });
        assert!(matches!(r, Err(EditError::InvalidTarget(_))), "{name}");
    }
    assert_eq!(d.text(), SRC);
    let mut d = doc("component Main inherits Window {\n    if true : Rectangle { }\n}\n");
    let r = d.apply(Op::ExtractComponent {
        path: p(&[0]),
        name: "Foo".into(),
    });
    assert!(matches!(r, Err(EditError::InvalidTarget(_))));
}

#[test]
fn extract_rejects_duplicate_name() {
    let mut d = doc(SRC);
    let r = d.apply(Op::ExtractComponent {
        path: p(&[0]),
        name: "Main".into(),
    });
    assert!(matches!(r, Err(EditError::InvalidTarget(_))));
}

const CHIP: &str = "component Chip inherits Rectangle {\n    in property <string> label;\n}\n";

#[test]
fn add_declaration_after_last() {
    let mut d = doc(CHIP);
    let r = d
        .apply(Op::AddDeclaration {
            component: "Chip".into(),
            text: "callback clicked();".into(),
        })
        .unwrap();
    assert_eq!(r, None);
    assert_eq!(
        d.text(),
        "component Chip inherits Rectangle {\n    in property <string> label;\n    callback clicked();\n}\n"
    );
}

#[test]
fn add_declaration_without_existing() {
    let mut d = doc("component Chip inherits Rectangle {\n    background: red;\n}\n");
    d.apply(Op::AddDeclaration {
        component: "Chip".into(),
        text: "callback c();".into(),
    })
    .unwrap();
    assert_eq!(
        d.text(),
        "component Chip inherits Rectangle {\n    callback c();\n    background: red;\n}\n"
    );
    let mut d = doc("component Chip inherits Rectangle {\n}\n");
    d.apply(Op::AddDeclaration {
        component: "Chip".into(),
        text: "callback c();".into(),
    })
    .unwrap();
    assert_eq!(
        d.text(),
        "component Chip inherits Rectangle {\n    callback c();\n}\n"
    );
}

#[test]
fn add_declaration_inline_and_missing() {
    let mut d = doc("component Chip inherits Rectangle { background: red; }\n");
    d.apply(Op::AddDeclaration {
        component: "Chip".into(),
        text: "callback c();".into(),
    })
    .unwrap();
    assert_eq!(
        d.text(),
        "component Chip inherits Rectangle { callback c(); background: red; }\n"
    );
    let r = d.apply(Op::AddDeclaration {
        component: "Nope".into(),
        text: "callback c();".into(),
    });
    assert!(matches!(r, Err(EditError::ComponentNotFound(_))));
}
