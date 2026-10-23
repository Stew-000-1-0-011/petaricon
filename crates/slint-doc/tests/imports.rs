use slint_doc::*;

const SRC: &str = r#"import { Button, LineEdit as Edit } from "std-widgets.slint";
import { Chip } from "chip.slint";
import "fonts/x.ttf";
global Theme { in-out property <color> bg; }
component Local inherits Rectangle { }
export component Main inherits Window { Chip { } }
"#;

#[test]
fn globals_are_not_components() {
    let d = Document::new("m.slint".into(), SRC.into());
    let names: Vec<_> = d.components().into_iter().map(|c| c.name).collect();
    assert_eq!(names, ["Local", "Main"]);
    assert_eq!(d.global_names(), ["Theme"]);
    // still addressable
    assert!(d.tree("Local").is_some());
}

#[test]
fn lists_imports_with_aliases() {
    let d = Document::new("m.slint".into(), SRC.into());
    let i = d.imports();
    assert_eq!(i.len(), 2);
    assert_eq!(i[0].path, "std-widgets.slint");
    assert_eq!(
        i[0].names,
        [("Button".to_string(), "Button".to_string()), ("LineEdit".to_string(), "Edit".to_string())]
    );
    assert_eq!(i[1].path, "chip.slint");
    assert_eq!(i[1].names, [("Chip".to_string(), "Chip".to_string())]);
}
