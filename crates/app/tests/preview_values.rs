//! Preview values and screen sizes (design §1 rules 3 and 4).
use std::path::{Path, PathBuf};

use app::session::{Session, SessionError, parse_screen_size};
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

    s.set_preview_value("connected", "true").unwrap();
    s.set_preview_value("page", "1").unwrap();
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
    s.set_preview_value("connected", "true").unwrap();
    s.set_preview_value("page", "1").unwrap();
    s.set_preview_value("rtt-ms", "\"not a number\" +").unwrap();
    s.set_preview_value("demo", "1").unwrap();
    assert!(s.rebuild_standalone(480., 800.), "{:?}", s.diagnostics());
    let props = s.preview_props();
    let err = |n: &str| props.iter().find(|p| p.name == n).unwrap().error.clone();
    assert!(!err("rtt-ms").is_empty());
    assert!(!err("demo").is_empty());
    assert!(err("connected").is_empty() && err("page").is_empty());
    assert!(shows(&s, "並進"));
    // Fixing the value clears the reason.
    s.set_preview_value("rtt-ms", "35").unwrap();
    s.set_preview_value("demo", "false").unwrap();
    assert!(s.rebuild_standalone(480., 800.));
    assert!(s.preview_props().iter().all(|p| p.error.is_empty()));
    // Unknown and private properties are not offered.
    assert!(matches!(s.set_preview_value("confirm-leave", "true"), Err(SessionError::NotPreviewProp(_))));
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
    s.set_preview_value("conn-text", "接続 \"A\"").unwrap();
    s.set_preview_value("page", " 1 ").unwrap();
    s.set_preview_value("connected", "true").unwrap();
    let v = s.preview();
    assert_eq!(v.value("conn-text"), Some("\"接続 \\\"A\\\"\""));
    assert_eq!(v.value("page"), Some("1"));
    let shown = s.preview_props().into_iter().find(|p| p.name == "conn-text").unwrap().shown;
    assert_eq!(shown, "接続 \"A\"");
    assert!(s.rebuild_standalone(480., 800.), "{:?}", s.diagnostics());
    assert!(s.preview_props().iter().all(|p| p.error.is_empty()));
    // Empty input removes the value.
    s.set_preview_value("page", "").unwrap();
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
    s.set_preview_value("connected", "true").unwrap();
    s.set_preview_value("page", "1").unwrap();
    s.set_preview_value("rtt-ms", "\"x\" +").unwrap();
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

#[test]
fn tapping_the_manual_panel_selects_it_not_the_hidden_leave_dialog() {
    let mut s = main_window();
    s.set_preview_value("connected", "true").unwrap();
    s.set_preview_value("page", "1").unwrap();
    assert!(s.rebuild_standalone(800., 360.), "{:?}", s.diagnostics());
    assert!(shows(&s, "並進"));
    // The full-screen LeaveDialog is last in the tree but `visible: false`.
    let hit = s.tap(400., 180.);
    assert!(hit.is_some());
    match s.selected_use() {
        Some(app::session::UseTarget::Imported { component, .. }) => assert_eq!(component, "ManualPanel"),
        other => panic!("{other:?}"),
    }
}

// ---- the same writing rule as the Props fields ----

const RULE: &str = r#"export component Main inherits Rectangle {
    in property <string> label: "x";
    in property <[int]> nums;
    in property <int> n;
    width: 300px;
    height: 300px;
    Text { text: root.label; }
}
"#;

fn rule_session() -> Session {
    let _ = std::panic::catch_unwind(i_slint_backend_testing::init_no_event_loop);
    let mut s = Session::new(Document::new(PathBuf::from("/t/r.slint"), RULE.into()));
    s.set_component("Main");
    s
}

fn shown(s: &Session, n: &str) -> String {
    s.preview_props().into_iter().find(|p| p.name == n).unwrap().shown
}

/// Stores the session's preview in a `.preview` text and loads it into a fresh session.
fn through_file(s: &Session) -> (String, Session) {
    let mut cfg = storage::PreviewConfig::default();
    cfg.set("r.slint", "Main", s.preview());
    let text = cfg.render();
    let mut s2 = rule_session();
    s2.set_previews(storage::PreviewConfig::parse(&text).of_file("r.slint"));
    (text, s2)
}

#[test]
fn plain_preview_text_with_quotes_and_backslashes_round_trips_through_the_file() {
    let mut s = rule_session();
    for typed in ["say \"hi\"", "C:\\dir\\", "a\\\"b", "{ok}", " spaced ", "日本語", "a=b"] {
        s.set_preview_value("label", typed).unwrap();
        let (_, mut s2) = through_file(&s);
        assert_eq!(shown(&s2, "label"), typed);
        assert!(s2.rebuild_standalone(300., 300.), "{typed}: {:?}", s2.diagnostics());
        assert!(s2.visible_texts().iter().any(|t| t == typed), "{typed}: {:?}", s2.visible_texts());
    }
}

#[test]
fn an_equals_expression_is_stored_raw_and_shown_with_the_equals() {
    let mut s = rule_session();
    s.set_preview_value("label", "= \"a\" + \"b\"").unwrap();
    assert_eq!(s.preview().value("label"), Some("\"a\" + \"b\""));
    assert_eq!(shown(&s, "label"), "=\"a\" + \"b\"");
    assert!(s.rebuild_standalone(300., 300.), "{:?}", s.diagnostics());
    assert!(s.visible_texts().iter().any(|t| t == "ab"));
    // The shown text is accepted back unchanged.
    let before = s.preview();
    s.set_preview_value("label", &shown(&s, "label")).unwrap();
    assert_eq!(s.preview(), before);
    // A bad expression is ignored with a reason and the preview still builds.
    s.set_preview_value("label", "=nothing").unwrap();
    assert!(s.rebuild_standalone(300., 300.), "{:?}", s.diagnostics());
    assert!(!s.preview_props().iter().find(|p| p.name == "label").unwrap().error.is_empty());
}

#[test]
fn a_double_equals_is_a_literal_equals_and_a_lone_equals_is_refused() {
    let mut s = rule_session();
    s.set_preview_value("label", "==abc").unwrap();
    assert_eq!(s.preview().value("label"), Some("\"=abc\""));
    assert_eq!(shown(&s, "label"), "==abc");
    let before = s.preview();
    for lone in ["=", "= "] {
        assert!(matches!(s.set_preview_value("label", lone), Err(SessionError::Rejected(_))));
        assert_eq!(s.preview(), before, "a refused input changes nothing");
    }
    // Empty still clears.
    s.set_preview_value("label", "").unwrap();
    assert_eq!(s.preview().value("label"), None);
}

#[test]
fn an_old_preview_file_with_a_quoted_string_shows_the_text() {
    let mut cfg = storage::PreviewConfig::default();
    let mut p = storage::ComponentPreview::default();
    p.set_value("label", "\"abc\"");
    p.set_value("nums", "[1, 2]");
    cfg.set("r.slint", "Main", p);
    let mut s = rule_session();
    s.set_previews(storage::PreviewConfig::parse(&cfg.render()).of_file("r.slint"));
    assert_eq!(shown(&s, "label"), "abc");
    assert_eq!(shown(&s, "nums"), "[1, 2]");
    assert!(s.rebuild_standalone(300., 300.), "{:?}", s.diagnostics());
    assert!(s.visible_texts().iter().any(|t| t == "abc"));
}

#[test]
fn raw_values_take_an_optional_leading_equals() {
    let mut s = rule_session();
    s.set_preview_value("nums", "[1, 2]").unwrap();
    assert_eq!(s.preview().value("nums"), Some("[1, 2]"));
    s.set_preview_value("nums", "=[3, 4]").unwrap();
    assert_eq!(s.preview().value("nums"), Some("[3, 4]"));
    assert_eq!(shown(&s, "nums"), "[3, 4]", "a raw value is shown as the expression");
    assert!(matches!(s.set_preview_value("nums", "="), Err(SessionError::Rejected(_))));
    assert_eq!(s.preview().value("nums"), Some("[3, 4]"));
    // Other kinds keep their input.
    s.set_preview_value("n", " 7 ").unwrap();
    assert_eq!(s.preview().value("n"), Some("7"));
}
