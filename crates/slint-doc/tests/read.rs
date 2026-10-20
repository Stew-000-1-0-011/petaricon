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
        if true: Rectangle { }
        Rectangle { }
    }
}
"#;

#[test]
fn lists_components() {
    let d = Document::new("m.slint".into(), SRC.into());
    let c = d.components();
    assert_eq!(
        c.iter().map(|c| c.name.as_str()).collect::<Vec<_>>(),
        ["Chip", "Main"]
    );
    assert!(!c[0].exported && c[1].exported);
    assert_eq!(c[0].declarations, ["in property <string> label;"]);
}

#[test]
fn element_info_with_id_and_bindings() {
    let d = Document::new("m.slint".into(), SRC.into());
    let e = d
        .element(&ElementPath {
            component: "Main".into(),
            indices: vec![0],
        })
        .unwrap();
    assert_eq!(e.type_name, "Text");
    assert_eq!(e.id.as_deref(), Some("title"));
    assert_eq!(
        e.bindings
            .iter()
            .map(|b| (b.name.as_str(), b.expr.as_str()))
            .collect::<Vec<_>>(),
        [("x", "8px"), ("y", "4px"), ("text", "\"見出し\"")]
    );
}

#[test]
fn tree_kinds() {
    let d = Document::new("m.slint".into(), SRC.into());
    let t = d.tree("Main").unwrap();
    assert_eq!(
        t.children[1].info.kind,
        ElementKind::Layout(LayoutKind::Vertical)
    );
    assert_eq!(t.children[1].children[1].info.kind, ElementKind::Repeated);
    assert_eq!(t.children[1].children[1].info.type_name, "Chip");
}

#[test]
fn offset_roundtrip() {
    let d = Document::new("m.slint".into(), SRC.into());
    let p = ElementPath {
        component: "Main".into(),
        indices: vec![1, 0],
    };
    let e = d.element(&p).unwrap();
    assert_eq!(d.path_at_offset(e.start), Some(p));
}

fn walk(n: &TreeNode, out: &mut Vec<ElementInfo>) {
    out.push(n.info.clone());
    for c in &n.children {
        walk(c, out);
    }
}

#[test]
fn every_path_roundtrips_and_element_agrees_with_tree() {
    let d = Document::new("m.slint".into(), SRC.into());
    for comp in ["Chip", "Main"] {
        let mut all = vec![];
        walk(&d.tree(comp).unwrap(), &mut all);
        for info in all {
            assert_eq!(d.path_at_offset(info.start), Some(info.path.clone()));
            assert_eq!(d.element(&info.path).unwrap(), info);
        }
    }
}

#[test]
fn conditional_and_no_id() {
    let d = Document::new("m.slint".into(), SRC.into());
    let p = ElementPath {
        component: "Main".into(),
        indices: vec![1, 2],
    };
    let e = d.element(&p).unwrap();
    assert_eq!(e.kind, ElementKind::Conditional);
    assert_eq!(e.id, None);
    let p = ElementPath {
        component: "Main".into(),
        indices: vec![1, 1],
    };
    assert_eq!(d.element(&p).unwrap().kind, ElementKind::Repeated);
}

#[test]
fn non_element_offset_is_none() {
    let d = Document::new("m.slint".into(), SRC.into());
    assert_eq!(d.path_at_offset(1), None);
}
