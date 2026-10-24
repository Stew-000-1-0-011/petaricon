use slint_doc::*;

fn p(i: &[usize]) -> ElementPath {
    ElementPath { component: "Main".into(), indices: i.to_vec() }
}

fn doc(src: &str) -> Document {
    Document::new("t.slint".into(), src.into())
}

fn param(path: ElementPath, property: &str, name: &str, ty: &str) -> Op {
    Op::Parameterize { path, property: property.into(), name: name.into(), ty: ty.into() }
}

const SRC: &str = "export component Main inherits Rectangle {\n    in property <int> count: 1;\n    width: 100px;\n    title := Text {\n        text: \"Hello\";\n        color: red;\n    }\n}\n";

#[test]
fn changes_one_declaration_line_and_the_binding() {
    let mut d = doc(SRC);
    assert_eq!(d.apply(param(p(&[0]), "text", "title-text", "string")).unwrap(), None);
    let want = SRC
        .replace("text: \"Hello\";", "text: root.title-text;")
        .replace(
            "in property <int> count: 1;\n",
            "in property <int> count: 1;\n    in property <string> title-text: \"Hello\";\n",
        );
    assert_eq!(d.text(), want);
    assert!(d.syntax_errors().is_empty());
    assert!(d.undo());
    assert_eq!(d.text(), SRC);
}

#[test]
fn works_on_the_root_element() {
    let mut d = doc(SRC);
    d.apply(param(p(&[]), "width", "main-width", "length")).unwrap();
    assert!(d.text().contains("in property <length> main-width: 100px;"), "{}", d.text());
    assert!(d.text().contains("    width: root.main-width;"), "{}", d.text());
}

#[test]
fn without_declarations_goes_after_the_brace() {
    let mut d = doc("component Main inherits Rectangle {\n    width: 5px;\n}\n");
    d.apply(param(p(&[]), "width", "w", "length")).unwrap();
    assert_eq!(
        d.text(),
        "component Main inherits Rectangle {\n    in property <length> w: 5px;\n    width: root.w;\n}\n"
    );
}

#[test]
fn refusals_leave_the_document_alone() {
    for (src, prop, name) in [
        (SRC, "opacity", "o"),                                  // no value
        (SRC, "text", "count"),                                 // name taken
        (SRC, "text", "bad name"),                              // not an identifier
        (&SRC.replace("\"Hello\"", "self.width > 1px ? \"a\" : \"b\""), "text", "t"),
        (&SRC.replace("\"Hello\"", "parent.width / 1px"), "text", "t"),
        (&SRC.replace("\"Hello\"", "root.count"), "text", "t"),
    ] {
        let mut d = doc(src);
        let before = d.text().to_string();
        assert!(d.apply(param(p(&[0]), prop, name, "string")).is_err(), "{src} {prop}");
        assert_eq!(d.text(), before);
    }
    assert!(doc(SRC).apply(param(p(&[0]), "text", "t", "str ing")).is_err());
}

#[test]
fn self_and_parent_are_matched_as_tokens() {
    let mut d = doc(&SRC.replace("\"Hello\"", "myself.name + parent-x"));
    d.apply(param(p(&[0]), "text", "t", "string")).unwrap();
    assert!(d.text().contains("text: root.t;"));
}
