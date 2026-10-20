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

#[test] fn lists_components() {
    let d = Document::new("m.slint".into(), SRC.into());
    let c = d.components();
    assert_eq!(c.iter().map(|c| c.name.as_str()).collect::<Vec<_>>(), ["Chip", "Main"]);
    assert!(!c[0].exported && c[1].exported);
    assert_eq!(c[0].declarations, ["in property <string> label;"]);
}

#[test] fn element_info_with_id_and_bindings() {
    let d = Document::new("m.slint".into(), SRC.into());
    let e = d.element(&ElementPath { component: "Main".into(), indices: vec![0] }).unwrap();
    assert_eq!(e.type_name, "Text");
    assert_eq!(e.id.as_deref(), Some("title"));
    assert_eq!(e.bindings.iter().map(|b| (b.name.as_str(), b.expr.as_str())).collect::<Vec<_>>(),
               [("x", "8px"), ("y", "4px"), ("text", "\"見出し\"")]);
}

#[test] fn tree_kinds() {
    let d = Document::new("m.slint".into(), SRC.into());
    let t = d.tree("Main").unwrap();
    assert_eq!(t.children[1].info.kind, ElementKind::Layout(LayoutKind::Vertical));
    assert_eq!(t.children[1].children[1].info.kind, ElementKind::Repeated);
    assert_eq!(t.children[1].children[1].info.type_name, "Chip");
}

#[test] fn offset_roundtrip() {
    let d = Document::new("m.slint".into(), SRC.into());
    let p = ElementPath { component: "Main".into(), indices: vec![1, 0] };
    let e = d.element(&p).unwrap();
    assert_eq!(d.path_at_offset(e.start), Some(p));
}
