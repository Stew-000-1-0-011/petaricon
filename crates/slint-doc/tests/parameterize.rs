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

#[test]
fn crlf_files_keep_their_line_endings() {
    let src = SRC.replace('\n', "\r\n");
    let mut d = doc(&src);
    d.apply(param(p(&[0]), "text", "title-text", "string")).unwrap();
    let want = src
        .replace("text: \"Hello\";", "text: root.title-text;")
        .replace(
            "in property <int> count: 1;\r\n",
            "in property <int> count: 1;\r\n    in property <string> title-text: \"Hello\";\r\n",
        );
    assert_eq!(d.text(), want);
    // No declaration yet: after the brace.
    let src = "component Main inherits Rectangle {\r\n    width: 5px;\r\n}\r\n";
    let mut d = doc(src);
    d.apply(param(p(&[]), "width", "w", "length")).unwrap();
    assert_eq!(
        d.text(),
        "component Main inherits Rectangle {\r\n    in property <length> w: 5px;\r\n    width: root.w;\r\n}\r\n"
    );
    // Plain AddDeclaration, both places.
    let mut d = doc(&SRC.replace('\n', "\r\n"));
    d.apply(Op::AddDeclaration { component: "Main".into(), text: "in property <int> n;".into() })
        .unwrap();
    assert!(d.text().contains("count: 1;\r\n    in property <int> n;\r\n    width"), "{:?}", d.text());
    assert_eq!(d.text().matches('\n').count(), d.text().matches("\r\n").count());
    let mut d = doc(src);
    d.apply(Op::AddDeclaration { component: "Main".into(), text: "in property <int> n;".into() })
        .unwrap();
    assert_eq!(
        d.text(),
        "component Main inherits Rectangle {\r\n    in property <int> n;\r\n    width: 5px;\r\n}\r\n"
    );
}

#[test]
fn a_multi_line_value_is_copied_as_written() {
    let src = "component Main inherits Rectangle {\n    Text {\n        text: true\n            ? \"a\"\n            : \"b\";\n    }\n}\n";
    let mut d = doc(src);
    d.apply(param(p(&[0]), "text", "t", "string")).unwrap();
    assert_eq!(
        d.text(),
        "component Main inherits Rectangle {\n    in property <string> t: true\n            ? \"a\"\n            : \"b\";\n    Text {\n        text: root.t;\n    }\n}\n"
    );
    assert!(d.syntax_errors().is_empty());
    // A plain declaration stays one line.
    assert!(doc(src)
        .apply(Op::AddDeclaration { component: "Main".into(), text: "in property <int> a;\nb".into() })
        .is_err());
}

#[test]
fn two_way_and_element_declared_properties_are_refused_distinctly() {
    let src = "component Main inherits Rectangle {\n    in-out property <int> n;\n    Text {\n        property <int> foo: 3;\n        text <=> root.label;\n    }\n}\n";
    let mut d = doc(src);
    let e = d.apply(param(p(&[0]), "text", "t", "string")).unwrap_err().to_string();
    assert_eq!(e, "双方向バインディングは引数にできません");
    let e = d.apply(param(p(&[0]), "foo", "t", "int")).unwrap_err().to_string();
    assert_eq!(e, "要素で宣言したプロパティは引数にできません");
    let e = d.apply(param(p(&[]), "n", "t", "int")).unwrap_err().to_string();
    assert_eq!(e, "要素で宣言したプロパティは引数にできません");
    assert_eq!(d.text(), src);
}

#[test]
fn names_clash_with_underscores_bindings_and_builtins() {
    let src = "component Main inherits Rectangle {\n    in property <int> foo_bar;\n    width: 5px;\n    Text { text: \"a\"; }\n}\n";
    for name in ["foo-bar", "foo_bar", "width", "text", "color", "font-size", "font_size"] {
        let mut d = doc(src);
        assert!(d.apply(param(p(&[0]), "text", name, "string")).is_err(), "{name}");
        assert_eq!(d.text(), src);
    }
    assert!(doc(src).apply(param(p(&[0]), "text", "label", "string")).is_ok());
}

#[test]
fn trailing_comment_and_same_line_element() {
    let src = "component Main inherits Rectangle {\n    in property <int> a; // note\n    Text { text: \"a\"; }\n}\n";
    let mut d = doc(src);
    d.apply(param(p(&[0]), "text", "t", "string")).unwrap();
    assert_eq!(
        d.text(),
        "component Main inherits Rectangle {\n    in property <int> a; // note\n    in property <string> t: \"a\";\n    Text { text: root.t; }\n}\n"
    );
    let src = "component Main inherits Rectangle { width: 5px; Text { text: \"a\"; } }\n";
    let mut d = doc(src);
    d.apply(param(p(&[]), "width", "w", "length")).unwrap();
    assert_eq!(
        d.text(),
        "component Main inherits Rectangle { in property <length> w: 5px; width: root.w; Text { text: \"a\"; } }\n"
    );
    assert!(d.syntax_errors().is_empty());
}
