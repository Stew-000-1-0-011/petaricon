use std::path::PathBuf;

use app::session::{Corner, PropKind, Session, SessionError};
use slint_doc::{Document, ElementPath};

const SRC: &str = r#"export component Main inherits Rectangle {
    width: 300px;
    height: 300px;
    a := Rectangle {
        x: 10px;
        y: 10px;
        width: 100px;
        height: 100px;
        b := Rectangle {
            x: 10px;
            y: 10px;
            width: 20px;
            height: 20px;
        }
    }
    VerticalLayout {
        y: 200px;
        height: 50px;
        Text { text: "in"; }
    }
}
"#;

fn path(i: &[usize]) -> ElementPath {
    ElementPath {
        component: "Main".into(),
        indices: i.to_vec(),
    }
}

fn session() -> Session {
    i_slint_backend_testing::init_no_event_loop();
    let mut s = Session::new(Document::new(PathBuf::from("/t/main.slint"), SRC.into()));
    assert!(s.rebuild_standalone(300., 300.));
    s
}

#[test]
fn tap_selects_deepest_then_parent() {
    let mut s = session();
    assert_eq!(s.tap(25., 25.), Some(path(&[0, 0])));
    assert_eq!(s.tap(25., 25.), Some(path(&[0])));
}

#[test]
fn move_snaps_to_grid() {
    let mut s = session();
    s.tap(60., 60.);
    assert_eq!(s.selection(), Some(&path(&[0])));
    s.move_selection(13., 6.).unwrap();
    assert!(
        s.doc().text().contains("x: 24px;\n        y: 16px;"),
        "{}",
        s.doc().text()
    );
}

#[test]
fn move_in_layout_is_refused() {
    let mut s = session();
    assert_eq!(s.tap(10., 210.), Some(path(&[1, 0])));
    let before = s.doc().text().to_string();
    assert!(matches!(
        s.move_selection(5., 5.),
        Err(SessionError::InLayout)
    ));
    assert_eq!(s.doc().text(), before);
}

#[test]
fn resize_top_left_moves_origin() {
    let mut s = session();
    s.tap(60., 60.);
    s.resize_selection(Corner::TopLeft, 8., 8.).unwrap();
    let t = s.doc().text();
    for want in ["x: 20px;", "y: 20px;", "width: 90px;", "height: 90px;"] {
        assert!(t.contains(want), "{want} in {t}");
    }
}

#[test]
fn compile_error_keeps_previous_instance() {
    let mut s = session();
    s.replace_text(format!(
        "{SRC}\nexport component Broken {{ Text {{ text: 1 +; }} }}\n"
    ));
    assert!(!s.rebuild_standalone(300., 300.));
    assert!(!s.diagnostics().is_empty());
    assert!(s.tap(60., 60.).is_some());
    s.move_selection(8., 8.).unwrap();
    assert!(s.doc().text().contains("x: 20px;"), "{}", s.doc().text());
}

#[test]
fn stale_offsets_after_top_edit_still_hit_right_element() {
    let mut s = session();
    s.replace_text(format!(
        "// header comment\n// more\nexport component Broken {{ Text {{ text: 1 +; }} }}\n{SRC}"
    ));
    assert!(!s.rebuild_standalone(300., 300.));
    assert_eq!(s.tap(25., 25.), Some(path(&[0, 0])));
    s.move_selection(8., 8.).unwrap();
    let t = s.doc().text();
    assert!(t.contains("x: 20px;"), "{t}");
    assert!(t.matches("x: 10px;").count() == 1, "{t}");
}

#[test]
fn length_changing_move_then_tap_and_move_again() {
    let mut s = session();
    s.tap(60., 60.);
    s.move_selection(13., 6.).unwrap(); // x: 10px -> 24px, no rebuild
    assert_eq!(s.tap(25., 25.), Some(path(&[0, 0])));
    s.move_selection(8., 8.).unwrap();
    let t = s.doc().text();
    assert!(t.contains("x: 24px;") && t.contains("x: 20px;"), "{t}");
}

#[test]
fn structural_op_while_stale_is_refused() {
    let mut s = session();
    s.replace_text(format!(
        "{SRC}\nexport component Broken {{ Text {{ text: 1 +; }} }}\n"
    ));
    assert!(!s.rebuild_standalone(300., 300.));
    let before = s.doc().text().to_string();
    let r = s.apply(slint_doc::Op::RemoveElement { path: path(&[0]) });
    assert!(matches!(r, Err(SessionError::Compile(_))));
    assert_eq!(s.doc().text(), before);
}

#[test]
fn tap_cycle_wraps_to_first() {
    let mut s = session();
    assert_eq!(s.tap(25., 25.), Some(path(&[0, 0])));
    assert_eq!(s.tap(25., 25.), Some(path(&[0])));
    assert_eq!(s.tap(25., 25.), Some(path(&[0, 0])));
}

#[test]
fn tap_on_empty_area_clears_selection() {
    let mut s = session();
    s.tap(25., 25.);
    assert_eq!(s.tap(900., 900.), None);
    assert_eq!(s.selection(), None);
}

#[test]
fn resize_layout_child_writes_only_size() {
    let mut s = session();
    s.tap(10., 210.);
    s.resize_selection(Corner::BottomRight, 8., 8.).unwrap();
    let t = s.doc().text();
    let line = t.lines().find(|l| l.contains("Text {")).unwrap();
    assert!(
        line.contains("width:") && line.contains("height:"),
        "{line}"
    );
    assert!(!line.contains(" x:") && !line.contains(" y:"), "{line}");
}

#[test]
fn root_move_is_invalid_target() {
    let mut s = session();
    s.apply(slint_doc::Op::Unwrap {
        layout: path(&[1]),
        positions: vec![(0, 200)],
    })
    .unwrap();
    assert_eq!(s.selection(), Some(&path(&[])));
    assert!(matches!(
        s.move_selection(4., 4.),
        Err(SessionError::Edit(slint_doc::EditError::InvalidTarget(_)))
    ));
}

#[test]
fn resize_keeps_four_px_minimum() {
    let mut s = session();
    s.tap(60., 60.);
    s.resize_selection(Corner::BottomRight, -200., -200.)
        .unwrap();
    let t = s.doc().text();
    assert!(
        t.contains("width: 4px;") && t.contains("height: 4px;"),
        "{t}"
    );
}

#[test]
fn rebuild_swaps_snapshot_only_when_the_factory_runs() {
    use slint::ComponentHandle;
    use std::cell::Cell;
    use std::rc::Rc;

    let mut s = session();
    s.tap(60., 60.);
    let before = s.selection_hit().unwrap().local;
    let created = Rc::new(Cell::new(0));
    let c = created.clone();
    s.set_on_instance(move || c.set(c.get() + 1));

    s.move_selection(40., 40.).unwrap();
    let factory = s.rebuild().expect("compiles");
    // The container has not run the factory yet: the old instance and the text it was
    // built from are still what geometry queries see.
    assert_eq!(s.selection_hit().unwrap().local, before);
    assert_eq!(created.get(), 0);

    let editor = app::Editor::new().unwrap();
    editor.set_factory(factory);
    editor.window().set_size(slint::LogicalSize::new(480., 800.));
    editor.show().unwrap();
    slint::platform::update_timers_and_animations();
    // Force layout evaluation of the container.
    let _ = editor.window().size();

    assert_eq!(created.get(), 1, "the factory ran once");
    let after = s.selection_hit().unwrap().local;
    // 10 + 40 snapped to the 4px grid.
    assert_eq!((after.x, after.y), (52., 52.));
}

const PALETTE_SRC: &str = r#"export component Main inherits Rectangle {
    width: 300px;
    height: 300px;
    box := Rectangle {
        width: 200px;
        height: 200px;
        Rectangle {
            x: 8px;
            y: 8px;
            width: 40px;
            height: 40px;
        }
    }
    VerticalLayout {
        Text { text: "a"; }
        Text { text: "b"; }
    }
}

component Card inherits Rectangle { }
"#;

fn palette_session(src: &str) -> Session {
    i_slint_backend_testing::init_no_event_loop();
    let mut s = Session::new(Document::new(PathBuf::from("/t/main.slint"), src.into()));
    assert!(s.rebuild_standalone(300., 300.));
    s
}

fn select(s: &mut Session, p: &[usize]) {
    s.select_path(path(p));
}

#[test]
fn palette_adds_into_free_container_without_overlap() {
    let mut s = palette_session(PALETTE_SRC);
    select(&mut s, &[0]);
    s.add_from_palette("Button").unwrap();
    let t = s.doc().text();
    assert!(
        t.starts_with("import { Button } from \"std-widgets.slint\";\n"),
        "{t}"
    );
    let el = s.doc().element(&path(&[0, 1])).unwrap();
    let b = |n: &str| el.bindings.iter().find(|b| b.name == n).map(|b| b.expr.clone());
    assert_eq!(el.type_name, "Button");
    assert_eq!(b("x").as_deref(), Some("56px"));
    assert_eq!(b("y").as_deref(), Some("8px"));
    assert_eq!(s.selection(), Some(&path(&[0, 1])));
    // One undo step removes both the element and the import.
    assert!(s.undo());
    assert_eq!(s.doc().text(), PALETTE_SRC);
}

#[test]
fn palette_adds_std_widget_to_existing_import() {
    let src = format!("import {{ LineEdit, ListView }} from \"std-widgets.slint\";\n{PALETTE_SRC}");
    let mut s = palette_session(&src);
    select(&mut s, &[0]);
    s.add_from_palette("Slider").unwrap();
    let t = s.doc().text();
    assert!(
        t.starts_with("import { LineEdit, ListView, Slider } from \"std-widgets.slint\";\n"),
        "{t}"
    );
    // Already imported: no duplicate.
    s.add_from_palette("Slider").unwrap();
    assert_eq!(s.doc().text().matches("Slider }").count(), 1);
}

#[test]
fn palette_into_non_container_goes_to_parent_and_layout_has_no_xy() {
    let mut s = palette_session(PALETTE_SRC);
    select(&mut s, &[1, 0]); // Text in the VerticalLayout
    s.add_from_palette("Rectangle").unwrap();
    assert_eq!(s.selection(), Some(&path(&[1, 2])));
    let el = s.doc().element(&path(&[1, 2])).unwrap();
    assert!(el.bindings.iter().all(|b| b.name != "x" && b.name != "y"));
    assert!(!s.doc().text().contains("import"));
}

#[test]
fn palette_lists_standard_then_other_components() {
    let s = palette_session(PALETTE_SRC);
    let p = s.palette();
    assert_eq!(p[0], "Rectangle");
    assert_eq!(p[12], "Switch");
    assert_eq!(&p[13..], ["Card"]);
}

#[test]
fn palette_refused_while_stale() {
    let mut s = palette_session(PALETTE_SRC);
    s.replace_text(format!("{PALETTE_SRC}\nexport component Broken {{ Text {{ text: 1 +; }} }}\n"));
    assert!(!s.rebuild_standalone(300., 300.));
    let before = s.doc().text().to_string();
    assert!(matches!(s.add_from_palette("Button"), Err(SessionError::Compile(_))));
    assert_eq!(s.doc().text(), before);
}

#[test]
fn tree_rows_order_and_labels() {
    let s = palette_session(PALETTE_SRC);
    let rows = s.tree_rows();
    let got: Vec<(String, usize, bool)> =
        rows.iter().map(|r| (r.label.clone(), r.depth, r.is_layout)).collect();
    let want = [
        ("Rectangle", 0, false),
        ("box := Rectangle", 1, false),
        ("Rectangle", 2, false),
        ("VerticalLayout", 1, true),
        ("Text", 2, false),
        ("Text", 2, false),
    ];
    assert_eq!(
        got,
        want.iter().map(|(l, d, y)| (l.to_string(), *d, *y)).collect::<Vec<_>>()
    );
    assert_eq!(rows[2].path, path(&[0, 0]));
}

#[test]
fn tree_actions_move_keep_selection_and_delete() {
    use app::session::TreeAction::*;
    let mut s = palette_session(PALETTE_SRC);
    // Down: box swaps with the layout.
    s.tree_action(&path(&[0]), Down).unwrap();
    assert_eq!(s.selection(), Some(&path(&[1])));
    assert_eq!(s.doc().element(&path(&[1])).unwrap().id.as_deref(), Some("box"));
    s.tree_action(&path(&[1]), Up).unwrap();
    assert_eq!(s.selection(), Some(&path(&[0])));
    assert!(s.tree_action(&path(&[0]), Up).is_err());
    // In: the layout goes into box (previous sibling container).
    s.tree_action(&path(&[1]), In).unwrap();
    assert_eq!(s.selection(), Some(&path(&[0, 1])));
    // Out: and back after box.
    s.tree_action(&path(&[0, 1]), Out).unwrap();
    assert_eq!(s.selection(), Some(&path(&[1])));
    // New container next to the selection, in its parent.
    s.tree_action(&path(&[1]), NewContainer).unwrap();
    assert_eq!(s.selection(), Some(&path(&[2])));
    assert_eq!(s.doc().element(&path(&[2])).unwrap().type_name, "Rectangle");
    s.tree_action(&path(&[2]), Delete).unwrap();
    assert!(s.doc().element(&path(&[2])).is_none());
    assert!(s.tree_action(&path(&[]), Delete).is_err());
}

fn add_widget(header: &str, name: &str) -> (String, String) {
    let src = format!("{header}{PALETTE_SRC}");
    i_slint_backend_testing::init_no_event_loop();
    // Not rebuilt: some headers (missing `other.slint`, empty import) do not compile, and
    // the text edit does not need geometry.
    let mut s = Session::new(Document::new(PathBuf::from("/t/main.slint"), src.clone()));
    select(&mut s, &[0]);
    s.add_from_palette(name).unwrap();
    (src, s.doc().text().to_string())
}

/// The result minus the one inserted element must be the source plus an import change.
fn without_new_element(text: &str) -> String {
    let start = text.find("Button {").or_else(|| text.find("LineEdit {")).unwrap();
    let line_start = text[..start].rfind('\n').unwrap() + 1;
    let end = start + text[start..].find("}\n").unwrap() + 2;
    format!("{}{}", &text[..line_start], &text[end..])
}

#[test]
fn import_none_header_comment_kept() {
    let (src, out) = add_widget("// header\n\n", "Button");
    assert_eq!(
        without_new_element(&out),
        format!("import {{ Button }} from \"std-widgets.slint\";\n{src}")
    );
}

#[test]
fn import_multiline_trailing_comma() {
    let h = "import {\n  Button,\n  Slider,\n} from \"std-widgets.slint\";\n";
    let (src, out) = add_widget(h, "LineEdit");
    let want = src.replacen("  Slider,\n}", "  Slider,\n  LineEdit,\n}", 1);
    assert_eq!(without_new_element(&out), want);
}

#[test]
fn import_multiline_without_trailing_comma() {
    let h = "import {\n\tButton,\n\tSlider\n} from \"std-widgets.slint\";\n";
    let (src, out) = add_widget(h, "LineEdit");
    let want = src.replacen("\tSlider\n}", "\tSlider,\n\tLineEdit\n}", 1);
    assert_eq!(without_new_element(&out), want);
}

#[test]
fn import_empty_braces() {
    let (src, out) = add_widget("import { } from \"std-widgets.slint\";\n", "Button");
    assert_eq!(
        without_new_element(&out),
        src.replacen("{ }", "{ Button }", 1)
    );
}

#[test]
fn import_other_module_only() {
    let h = "import { X } from \"other.slint\";\n";
    let (src, out) = add_widget(h, "Button");
    assert_eq!(
        without_new_element(&out),
        format!("import {{ Button }} from \"std-widgets.slint\";\n{src}")
    );
}

#[test]
fn import_alias_counts_as_present() {
    let h = "import { Button as B } from \"std-widgets.slint\";\n";
    let (src, out) = add_widget(h, "Button");
    assert_eq!(without_new_element(&out), src);
}

// ---- properties and source ----

#[test]
fn properties_put_geometry_first() {
    let mut s = session();
    select(&mut s, &[0]);
    let rows = s.properties();
    let names: Vec<&str> = rows.iter().map(|r| r.name.as_str()).collect();
    assert_eq!(
        names,
        ["x", "y", "width", "height", "background", "border-radius", "border-width", "border-color"]
    );
    assert_eq!(rows[0].expr, "10px");
    assert_eq!(rows[4].expr, "");
    assert_eq!(rows[0].kind, PropKind::Length);
    assert_eq!(rows[4].kind, PropKind::Color);
    select(&mut s, &[1]);
    let names: Vec<String> = s.properties().into_iter().map(|r| r.name).collect();
    assert_eq!(&names[4..6], ["spacing", "padding"]);
    assert!(names.contains(&"y".to_string()));
    select(&mut s, &[1, 0]);
    let rows = s.properties();
    assert_eq!(&rows[4].name, "text");
    assert_eq!(rows[4].expr, "\"in\"");
}

#[test]
fn set_length_adds_px() {
    let mut s = session();
    select(&mut s, &[0]);
    s.set_property("x", "12").unwrap();
    assert!(s.doc().text().contains("x: 12px;"));
    s.set_property("y", "-3.5").unwrap();
    assert!(s.doc().text().contains("y: -3.5px;"));
    s.set_property("width", "50%").unwrap();
    assert!(s.doc().text().contains("width: 50%;"));
}

#[test]
fn set_text_quotes() {
    let mut s = session();
    select(&mut s, &[1, 0]);
    s.set_property("text", "停止").unwrap();
    assert!(s.doc().text().contains("text: \"停止\";"));
    s.set_property("text", "a\"b\\c").unwrap();
    assert!(s.doc().text().contains(r#"text: "a\"b\\c";"#));
    s.set_property("text", "\"done\"").unwrap();
    assert!(s.doc().text().contains("text: \"done\";"));
    s.set_property("color", "#ff0000").unwrap();
    assert!(s.doc().text().contains("color: #ff0000;"));
}

#[test]
fn empty_value_removes() {
    let mut s = session();
    select(&mut s, &[0]);
    s.set_property("x", "").unwrap();
    assert!(!s.doc().text().contains("        x: 10px;\n        y: 10px;\n        width: 100px"));
    assert!(s.doc().text().contains("a := Rectangle {\n        y: 10px;"));
    assert_eq!(s.properties()[0].expr, "");
}

#[test]
fn set_property_without_selection_is_refused() {
    let mut s = session();
    assert!(matches!(s.set_property("x", "1"), Err(SessionError::NoSelection)));
}

#[test]
fn source_range_is_selection_or_component() {
    let mut s = session();
    let whole = s.source_range().unwrap();
    assert!(s.doc().text()[whole.clone()].starts_with("export component Main"));
    assert!(s.doc().text()[whole].ends_with('}'));
    select(&mut s, &[0, 0]);
    let r = s.source_range().unwrap();
    assert!(s.doc().text()[r].starts_with("b := Rectangle {"));
}

#[test]
fn replace_source_touches_only_the_range() {
    let mut s = session();
    select(&mut s, &[0, 0]);
    let r = s.source_range().unwrap();
    let original = s.doc().text()[r.clone()].to_string();
    let before = s.doc().text().to_string();
    let new = original.replace("width: 20px;", "width: 21px;");
    s.replace_source(r.start, &original, &new).unwrap();
    assert_eq!(s.doc().text(), before.replacen("width: 20px;", "width: 21px;", 1));
    // stale: the same range no longer holds `original`
    assert!(matches!(
        s.replace_source(r.start, &original, "x"),
        Err(SessionError::StaleSource)
    ));
}

#[test]
fn component_range_at_offset() {
    let src = "component A inherits Rectangle { }\nexport component B inherits Rectangle { Text { text: \"x\"; } }\n";
    let s = Session::new(Document::new(PathBuf::from("/t/m.slint"), src.into()));
    let off = src.find("Text").unwrap() as u32;
    let r = s.component_range_at(Some(off));
    assert!(src[r].starts_with("export component B"));
    assert_eq!(s.component_range_at(None), 0..src.len());
}
