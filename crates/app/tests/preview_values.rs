//! Preview values and screen sizes (design §1 rules 3 and 4).
use std::path::{Path, PathBuf};

use app::session::{Session, parse_screen_size};
use slint_doc::Document;

fn fixture_dir() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR")).join("../../testdata/controller-ui").canonicalize().unwrap()
}

fn main_window() -> Session {
    let _ = std::panic::catch_unwind(i_slint_backend_testing::init_no_event_loop);
    let path = fixture_dir().join("main.slint");
    let mut s = Session::new(Document::new(path.clone(), std::fs::read_to_string(path).unwrap()));
    s.set_component("MainWindow");
    s
}

fn shows(s: &Session, text: &str) -> bool {
    s.visible_texts().iter().any(|t| t == text)
}

#[test]
fn connected_and_page_show_the_manual_panel() {
    let mut s = main_window();
    assert!(s.rebuild_standalone(480., 800.), "{:?}", s.diagnostics());
    assert!(!shows(&s, "並進"), "manual panel is visible without values");

    assert!(s.set_preview_value("connected", "true"));
    assert!(s.set_preview_value("page", "1"));
    assert!(s.rebuild_standalone(480., 800.), "{:?}", s.diagnostics());
    assert!(s.preview_props().iter().all(|p| p.error.is_empty()));
    assert!(shows(&s, "並進"), "{:?}", s.visible_texts());
    // The .slint text is untouched.
    let path = fixture_dir().join("main.slint");
    assert_eq!(s.doc().text(), std::fs::read_to_string(path).unwrap());
    assert!(!s.doc().is_dirty());
}

#[test]
fn a_bad_expression_is_ignored_with_a_reason_and_the_rest_applies() {
    let mut s = main_window();
    assert!(s.set_preview_value("connected", "true"));
    assert!(s.set_preview_value("page", "1"));
    assert!(s.set_preview_value("rtt-ms", "\"not a number\" +"));
    assert!(s.set_preview_value("demo", "1"));
    assert!(s.rebuild_standalone(480., 800.), "{:?}", s.diagnostics());
    let props = s.preview_props();
    let err = |n: &str| props.iter().find(|p| p.name == n).unwrap().error.clone();
    assert!(!err("rtt-ms").is_empty());
    assert!(!err("demo").is_empty());
    assert!(err("connected").is_empty() && err("page").is_empty());
    assert!(shows(&s, "並進"));
    // Fixing the value clears the reason.
    assert!(s.set_preview_value("rtt-ms", "35"));
    assert!(s.set_preview_value("demo", "false"));
    assert!(s.rebuild_standalone(480., 800.));
    assert!(s.preview_props().iter().all(|p| p.error.is_empty()));
    // Unknown and private properties are not offered.
    assert!(!s.set_preview_value("confirm-leave", "true"));
    assert!(!s.preview_props().iter().any(|p| p.name == "confirm-leave"));
}

#[test]
fn properties_are_listed_with_their_editor_kinds() {
    use app::session::PreviewKind::*;
    let s = main_window();
    let kinds: Vec<(String, app::session::PreviewKind)> =
        s.preview_props().into_iter().map(|p| (p.name, p.kind)).collect();
    let k = |n: &str| kinds.iter().find(|(m, _)| m == n).unwrap_or_else(|| panic!("{n}")).1;
    assert_eq!(k("connected"), Bool);
    assert_eq!(k("page"), Int);
    assert_eq!(k("map-scale"), Float);
    assert_eq!(k("conn-text"), Text);
    assert_eq!(k("beacons"), Raw);
    assert_eq!(k("map-image"), Raw);
    assert_eq!(k("conn-color"), Raw);
}

#[test]
fn typed_inputs_become_expressions() {
    let mut s = main_window();
    s.set_preview_value("conn-text", "接続 \"A\"");
    s.set_preview_value("page", " 1 ");
    s.set_preview_value("connected", "true");
    let v = s.preview();
    assert_eq!(v.value("conn-text"), Some("\"接続 \\\"A\\\"\""));
    assert_eq!(v.value("page"), Some("1"));
    let shown = s.preview_props().into_iter().find(|p| p.name == "conn-text").unwrap().shown;
    assert_eq!(shown, "接続 \"A\"");
    assert!(s.rebuild_standalone(480., 800.), "{:?}", s.diagnostics());
    assert!(s.preview_props().iter().all(|p| p.error.is_empty()));
    // Empty input removes the value.
    s.set_preview_value("page", "");
    assert_eq!(s.preview().value("page"), None);
}

#[test]
fn screen_size_defaults_and_follows_the_choice() {
    let mut s = main_window();
    assert_eq!(s.screen_size(), (360, 640));
    s.set_screen_size((800, 360));
    assert_eq!(s.screen_size(), (800, 360));
    assert_eq!(s.preview().size, Some((800, 360)));
    // Per component.
    s.set_component("ManualPanel");
    assert_eq!(s.screen_size(), (360, 640));
    s.set_component("MainWindow");
    assert_eq!(s.screen_size(), (800, 360));
    assert_eq!(parse_screen_size("914x411"), Some((914, 411)));
    assert_eq!(parse_screen_size(" 1280 × 720 "), Some((1280, 720)));
    assert_eq!(parse_screen_size("0x5"), None);
    assert_eq!(parse_screen_size("abc"), None);
}

#[test]
fn a_rebuild_with_one_bad_value_is_one_compile_after_the_first() {
    let mut s = main_window();
    s.set_preview_value("connected", "true");
    s.set_preview_value("page", "1");
    s.set_preview_value("rtt-ms", "\"x\" +");
    assert!(s.rebuild_standalone(480., 800.));
    let n0 = preview::compile_count();
    for _ in 0..3 {
        assert!(s.rebuild_standalone(480., 800.));
    }
    let per_rebuild = (preview::compile_count() - n0) as f32 / 3.;
    eprintln!("compiles per rebuild with one bad value: {per_rebuild}");
    assert_eq!(per_rebuild, 1.0);
    assert!(!s.preview_props().iter().find(|p| p.name == "rtt-ms").unwrap().error.is_empty());
}

#[test]
fn stored_values_for_undeclared_properties_are_never_compiled() {
    let mut s = main_window();
    let mut p = storage::ComponentPreview::default();
    p.values.push(("x:1px;}Rectangle{y".to_string(), "1".to_string()));
    p.values.push(("confirm-leave".to_string(), "true".to_string()));
    p.values.push(("page".to_string(), "1".to_string()));
    s.set_previews([("MainWindow".to_string(), p)].into());
    let n0 = preview::compile_count();
    assert!(s.rebuild_standalone(480., 800.), "{:?}", s.diagnostics());
    assert_eq!(preview::compile_count() - n0, 1);
    assert!(s.preview_props().iter().all(|p| p.error.is_empty()));
}
