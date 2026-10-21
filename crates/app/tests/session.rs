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
    assert_eq!(s.tap(25., 25.), Some(path(&[])));
    assert_eq!(s.tap(25., 25.), Some(path(&[0, 0])));
}

#[test]
fn root_reached_by_tapping_again_cannot_be_moved() {
    let mut s = session();
    s.tap(25., 25.);
    s.tap(25., 25.);
    assert_eq!(s.tap(25., 25.), Some(path(&[])));
    let before = s.doc().text().to_string();
    assert!(matches!(
        s.move_selection(4., 4.),
        Err(SessionError::Edit(slint_doc::EditError::InvalidTarget(_)))
    ));
    assert!(matches!(
        s.resize_selection(Corner::BottomRight, 4., 4.),
        Err(SessionError::Edit(slint_doc::EditError::InvalidTarget(_)))
    ));
    assert_eq!(s.doc().text(), before);
}

#[test]
fn selected_root_has_overlay_geometry() {
    let mut s = session();
    s.tap(25., 25.);
    s.tap(25., 25.);
    assert_eq!(s.tap(25., 25.), Some(path(&[])));
    let hit = s.selection_hit().expect("root selection must have geometry for the overlay");
    assert_eq!((hit.rect.x, hit.rect.y), (0., 0.));
    assert!(hit.rect.w > 0. && hit.rect.h > 0.);
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
    s.add_from_palette("Button", None).unwrap();
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
    s.add_from_palette("Slider", None).unwrap();
    let t = s.doc().text();
    assert!(
        t.starts_with("import { LineEdit, ListView, Slider } from \"std-widgets.slint\";\n"),
        "{t}"
    );
    // Already imported: no duplicate.
    s.add_from_palette("Slider", None).unwrap();
    assert_eq!(s.doc().text().matches("Slider }").count(), 1);
}

#[test]
fn palette_into_non_container_goes_to_parent_and_layout_has_no_xy() {
    let mut s = palette_session(PALETTE_SRC);
    select(&mut s, &[1, 0]); // Text in the VerticalLayout
    s.add_from_palette("Rectangle", None).unwrap();
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
    assert!(matches!(s.add_from_palette("Button", None), Err(SessionError::Compile(_))));
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
    s.add_from_palette(name, None).unwrap();
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

#[test]
fn text_keeps_surrounding_spaces_and_same_value_is_a_noop() {
    let mut s = session();
    select(&mut s, &[1, 0]);
    s.set_property("text", "Hi ").unwrap();
    assert!(s.doc().text().contains("text: \"Hi \";"));
    s.mark_saved();
    s.set_property("text", "Hi ").unwrap();
    s.set_property("text", "\"Hi \"").unwrap();
    assert!(!s.doc().is_dirty());
    select(&mut s, &[0]);
    s.set_property("x", "10").unwrap();
    assert!(!s.doc().is_dirty());
    s.set_property("background", "").unwrap();
    assert!(!s.doc().is_dirty());
}

fn tidy_session(body: &str) -> Session {
    i_slint_backend_testing::init_no_event_loop();
    let src = format!(
        "export component Main inherits Rectangle {{\n    width: 300px;\n    height: 300px;\n    box := Rectangle {{\n        x: 0px;\n        y: 0px;\n        width: 300px;\n        height: 100px;\n{body}    }}\n}}\n"
    );
    let mut s = Session::new(Document::new(PathBuf::from("/t/main.slint"), src));
    assert!(s.rebuild_standalone(300., 300.));
    s.select_path(path(&[0]));
    s
}

const TWO_KIDS: &str = "        Rectangle { x: 10px; y: 10px; width: 40px; height: 30px; }\n        Rectangle { x: 60px; y: 10px; width: 40px; height: 30px; }\n";

#[test]
fn tidy_wraps_and_selects_layout() {
    let mut s = tidy_session(TWO_KIDS);
    let w = s.tidy_selection().unwrap();
    assert!(w.is_empty());
    let t = s.doc().text();
    assert!(t.contains("HorizontalLayout {"), "{t}");
    assert!(t.contains("spacing:"), "{t}");
    assert!(!t.contains("x: 10px;"), "{t}");
    assert_eq!(s.selection(), Some(&path(&[0, 0])));
}

#[test]
fn tidy_single_child_is_vertical() {
    let mut s = tidy_session("        Rectangle { x: 10px; y: 10px; width: 40px; height: 30px; }\n");
    let w = s.tidy_selection().unwrap();
    assert!(w.is_empty());
    assert!(s.doc().text().contains("VerticalLayout {"));
}

#[test]
fn tidy_empty_container_is_refused() {
    let mut s = tidy_session("");
    let before = s.doc().text().to_string();
    assert!(matches!(s.tidy_selection(), Err(SessionError::NotAContainer)));
    assert_eq!(s.doc().text(), before);
}

#[test]
fn untidy_restores_visual_positions() {
    let mut s = tidy_session(TWO_KIDS);
    s.tidy_selection().unwrap();
    assert!(s.rebuild_standalone(300., 300.));
    s.untidy_selection().unwrap();
    let t = s.doc().text();
    assert!(!t.contains("Layout"), "{t}");
    assert!(t.contains("x: 12px;") && t.contains("y: 12px;"), "{t}");
    assert!(t.contains("x: 64px;"), "{t}");
    assert_eq!(s.selection(), Some(&path(&[0])));
}

#[test]
fn guides_and_guide_edit() {
    let mut s = tidy_session(TWO_KIDS);
    s.tidy_selection().unwrap();
    assert!(s.rebuild_standalone(300., 300.));
    s.select_path(path(&[0]));
    let g = s.guides().expect("guides for the container");
    assert_eq!(g.gaps.len(), 1);
    s.set_guide_property("spacing", "20").unwrap();
    assert!(s.doc().text().contains("spacing: 20px;"), "{}", s.doc().text());
    assert_eq!(s.selection(), Some(&path(&[0, 0])));
}

#[test]
fn tidy_untidy_tidy_round_trip_is_stable() {
    let mut s = tidy_session(TWO_KIDS);
    s.tidy_selection().unwrap();
    let first = s.doc().text().to_string();
    assert!(s.rebuild_standalone(300., 300.));
    s.untidy_selection().unwrap();
    assert!(s.rebuild_standalone(300., 300.));
    s.select_path(path(&[0]));
    s.tidy_selection().unwrap();
    assert_eq!(s.doc().text(), first);
}

fn assert_tidy_refused(child: &str) {
    let mut s = tidy_session(&format!("{TWO_KIDS}{child}"));
    let before = s.doc().text().to_string();
    let err = s.tidy_selection().unwrap_err();
    assert!(
        matches!(&err, SessionError::Edit(slint_doc::EditError::InvalidTarget(m)) if m.contains("for / if")),
        "{err:?}"
    );
    assert_eq!(s.doc().text(), before);
}

#[test]
fn tidy_refuses_for_children() {
    assert_tidy_refused("        for i in 3: Rectangle { width: 10px; height: 10px; }\n");
}

#[test]
fn tidy_refuses_if_children() {
    assert_tidy_refused("        if true: Rectangle { width: 10px; height: 10px; }\n");
}

#[test]
fn untidy_of_non_layout_is_refused() {
    let mut s = tidy_session(TWO_KIDS);
    let before = s.doc().text().to_string();
    assert!(matches!(s.untidy_selection(), Err(SessionError::NotALayout)));
    s.select_path(path(&[0, 0]));
    assert!(matches!(s.untidy_selection(), Err(SessionError::NotALayout)));
    assert_eq!(s.doc().text(), before);
}

#[test]
fn guides_and_untidy_agree_on_the_layout() {
    let mut s = tidy_session(TWO_KIDS);
    s.tidy_selection().unwrap();
    assert!(s.rebuild_standalone(300., 300.));
    s.select_path(path(&[0]));
    assert_eq!(s.guide_layout_path(), Some(path(&[0, 0])));
    assert!(s.guides().is_some());
    s.untidy_selection().unwrap();
    assert!(!s.doc().text().contains("Layout"));
}

#[test]
fn failed_guide_edit_keeps_selection() {
    let mut s = tidy_session(TWO_KIDS);
    s.tidy_selection().unwrap();
    assert!(s.rebuild_standalone(300., 300.));
    s.select_path(path(&[0]));
    // No layout under this selection: the edit fails and the selection stays.
    s.select_path(path(&[0, 0, 0]));
    assert!(s.set_guide_property("spacing", "4").is_err());
    assert_eq!(s.selection(), Some(&path(&[0, 0, 0])));
}

#[test]
fn extract_selection_switches_selection_to_usage() {
    let mut s = session();
    s.tap(25., 25.);
    assert_eq!(s.selection(), Some(&path(&[0, 0])));
    s.extract_selection("Inner").unwrap();
    assert_eq!(s.selection(), Some(&path(&[0, 0])));
    let names: Vec<String> = s.doc().components().into_iter().map(|c| c.name).collect();
    assert!(names.contains(&"Inner".to_string()), "{names:?}");
    assert!(s.doc().text().contains("Inner {"), "{}", s.doc().text());
}

#[test]
fn extract_selection_without_selection_or_bad_name_fails() {
    let mut s = session();
    assert!(matches!(s.extract_selection("Inner"), Err(SessionError::NoSelection)));
    s.tap(25., 25.);
    assert!(s.extract_selection("Main").is_err());
    assert!(s.extract_selection("1bad").is_err());
}

#[test]
fn declaration_text_shapes() {
    use app::session::{DeclKind, declaration_text};
    assert_eq!(declaration_text(DeclKind::InProperty, "string", "label"), "in property <string> label;");
    assert_eq!(declaration_text(DeclKind::InOutProperty, "int", "n"), "in-out property <int> n;");
    assert_eq!(declaration_text(DeclKind::OutProperty, "bool", "ok"), "out property <bool> ok;");
    assert_eq!(declaration_text(DeclKind::Callback, "string", "clicked"), "callback clicked();");
}

#[test]
fn extract_then_add_declarations_and_name_errors() {
    use app::session::DeclKind;
    let mut s = session();
    assert!(s.component_name_error("Main").is_some());
    assert!(s.component_name_error("9x").is_some());
    assert!(s.component_name_error("").is_some());
    assert!(s.component_name_error("Card").is_none());
    s.tap(25., 25.);
    s.extract_selection("Card").unwrap();
    s.add_declaration("Card", DeclKind::InProperty, "string", "label").unwrap();
    s.add_declaration("Card", DeclKind::Callback, "string", "clicked").unwrap();
    let t = s.doc().text();
    assert!(t.contains("in property <string> label;"), "{t}");
    assert!(t.contains("callback clicked();"), "{t}");
    assert!(s.component_name_error("Card").is_some());
    assert!(s.add_declaration("Card", DeclKind::InProperty, "string", "a b").is_err());
    assert!(s.add_declaration("Card", DeclKind::InProperty, "nope", "x").is_err());
    assert!(s.rebuild_standalone(300., 300.), "{:?}", s.diagnostics());
}

// ---- palette placement hint ----

fn local_xy(s: &Session, p: &[usize]) -> (String, String) {
    let el = s.doc().element(&path(p)).unwrap();
    let b = |n: &str| el.bindings.iter().find(|b| b.name == n).map(|b| b.expr.clone()).unwrap();
    (b("x"), b("y"))
}

#[test]
fn palette_hint_places_near_the_visible_origin() {
    let mut s = palette_session(PALETTE_SRC);
    select(&mut s, &[0]); // box: 200x200 at (50, 50) in the 300x300 root
    s.add_from_palette("Rectangle", Some((146., 146.))).unwrap();
    assert_eq!(local_xy(&s, &[0, 1]), ("96px".into(), "96px".into()));
}

#[test]
fn palette_hint_is_converted_to_the_parents_coordinates_and_clamped() {
    let mut s = palette_session(PALETTE_SRC);
    select(&mut s, &[0]);
    // Left/above the container: clamped to 8.
    s.add_from_palette("Rectangle", Some((-50., -50.))).unwrap();
    assert_eq!(local_xy(&s, &[0, 1]), ("56px".into(), "8px".into()), "overlaps (8,8) child, scans on");
    // Far right/below: clamped inside the 200x200 container.
    select(&mut s, &[0]);
    s.add_from_palette("Rectangle", Some((900., 900.))).unwrap();
    assert_eq!(local_xy(&s, &[0, 2]), ("160px".into(), "160px".into()));
}

#[test]
fn palette_full_container_cascades_instead_of_stacking_at_8_8() {
    let src = "export component Main inherits Rectangle {\n    width: 300px;\n    height: 300px;\n    Rectangle {\n        x: 4px;\n        y: 4px;\n        width: 300px;\n        height: 300px;\n    }\n}\n";
    let mut s = palette_session(src);
    s.add_from_palette("Rectangle", None).unwrap();
    assert_eq!(local_xy(&s, &[1]), ("12px".into(), "12px".into()));
    assert!(s.rebuild_standalone(300., 300.));
    s.select_path(path(&[]));
    s.add_from_palette("Rectangle", None).unwrap();
    assert_eq!(local_xy(&s, &[2]), ("20px".into(), "20px".into()));
}

// ---- stale preview after source edits ----

#[test]
fn source_edit_adding_a_broken_sibling_does_not_retarget_edits() {
    let mut s = session();
    // A new first child shifts every path; the source no longer compiles.
    let edited = SRC.replacen(
        "    a := Rectangle {",
        "    Rectangle { width: 5px; foo: 1 +; }\n    a := Rectangle {",
        1,
    );
    s.replace_text(edited.clone());
    assert!(!s.rebuild_standalone(300., 300.));
    assert_eq!(s.tap(60., 60.), None);
    assert_eq!(s.selection(), None);
    assert!(matches!(s.move_selection(8., 8.), Err(SessionError::NoSelection)));
    // Selecting through the tree must not let geometry or property edits through either.
    s.select_path(path(&[0]));
    assert!(matches!(s.move_selection(8., 8.), Err(SessionError::Compile(_))));
    assert!(matches!(
        s.resize_selection(Corner::BottomRight, 8., 8.),
        Err(SessionError::Compile(_))
    ));
    assert!(matches!(s.set_property("x", "30"), Err(SessionError::Compile(_))));
    assert!(matches!(s.set_guide_property("spacing", "4"), Err(SessionError::Compile(_))));
    assert!(matches!(s.untidy_selection(), Err(SessionError::Compile(_))));
    assert!(matches!(s.tidy_selection(), Err(SessionError::Compile(_))));
    assert_eq!(s.doc().text(), edited);
}

// ---- import edge cases ----

#[test]
fn import_with_multibyte_comment_before_closing_brace() {
    let h = "import { Button // ボタン\n} from \"std-widgets.slint\";\n";
    let (src, out) = add_widget(h, "LineEdit");
    let want = src.replacen("Button", "Button,\n LineEdit", 1);
    assert_eq!(without_new_element(&out), want);
}

#[test]
fn import_with_multibyte_block_comment_after_trailing_comma() {
    let h = "import { Button, /* 日本語 */ } from \"std-widgets.slint\";\n";
    let (_, out) = add_widget(h, "LineEdit");
    assert!(out.contains("LineEdit"), "{out}");
}

#[test]
fn comment_mentioning_std_widgets_is_not_an_import() {
    let h = "// uses \"std-widgets.slint\" later\nimport { X } from \"other.slint\"; /* import { Y } from \"std-widgets.slint\"; */\n";
    let (src, out) = add_widget(h, "Button");
    assert_eq!(
        without_new_element(&out),
        format!("import {{ Button }} from \"std-widgets.slint\";\n{src}")
    );
}
