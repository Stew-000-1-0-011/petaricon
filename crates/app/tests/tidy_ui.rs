//! Free Tidy through the editor window: toast, overlay lifetime and the tolerance setting.

use slint::{ComponentHandle, Model};
use storage::{LocalStore, ProjectStore};

fn src() -> String {
    let rect = |x: i32, y: i32| format!("        Rectangle {{ x: {x}px; y: {y}px; width: 40px; height: 30px; }}\n");
    format!(
        "export component Main inherits Rectangle {{\n    width: 300px;\n    height: 300px;\n    box := Rectangle {{\n        x: 0px;\n        y: 0px;\n        width: 300px;\n        height: 300px;\n{}{}{}{}    }}\n}}\n",
        rect(10, 10),
        rect(14, 60),
        rect(22, 110),
        rect(12, 200),
    )
}

fn start(dir: &tempfile::TempDir) -> app::Editor {
    let store = LocalStore::new(dir.path().to_path_buf()).unwrap();
    if store.projects().unwrap().is_empty() {
        store.create_project("default").unwrap();
        store.write("default", "main.slint", &src()).unwrap();
    }
    thread_local!(static INIT: () = i_slint_backend_testing::init_no_event_loop());
    INIT.with(|_| {});
    let editor = app::Editor::new().unwrap();
    app::glue::start(&editor, Box::new(store)).unwrap();
    editor.window().set_size(slint::LogicalSize::new(360., 760.));
    editor.show().unwrap();
    slint::platform::update_timers_and_animations();
    let _ = editor.window().size();
    editor.invoke_tree_selected(1);
    editor
}

fn saved(dir: &tempfile::TempDir) -> String {
    LocalStore::new(dir.path().to_path_buf()).unwrap().read("default", "main.slint").unwrap()
}

fn stored(dir: &tempfile::TempDir) -> Option<String> {
    LocalStore::new(dir.path().to_path_buf()).unwrap().setting("tidy_tolerance")
}

#[test]
fn tidy_button_edits_shows_summary_and_overlay_until_the_next_action() {
    let dir = tempfile::tempdir().unwrap();
    let editor = start(&dir);
    editor.invoke_tidy();
    let toast = editor.get_toast_text().to_string();
    assert!(toast.starts_with("そろえ "), "{toast}");
    assert!(editor.get_dirty());
    assert!(editor.get_tidy_marks().row_count() > 0);
    // selection change clears the overlay
    editor.invoke_tapped(5., 5.);
    assert_eq!(editor.get_tidy_marks().row_count(), 0);
    // one undo restores the original file text
    editor.invoke_undo();
    editor.invoke_save();
    assert_eq!(saved(&dir), src());
}

#[test]
fn overlay_clears_on_undo_tab_change_and_next_edit() {
    for clear in 0..3 {
        let dir = tempfile::tempdir().unwrap();
        let editor = start(&dir);
        editor.invoke_tidy();
        assert!(editor.get_tidy_marks().row_count() > 0);
        match clear {
            0 => editor.invoke_undo(),
            1 => editor.set_panel_tab(1),
            _ => editor.invoke_prop_accepted("x".into(), "3".into()),
        }
        slint::platform::update_timers_and_animations();
        assert_eq!(editor.get_tidy_marks().row_count(), 0, "case {clear}");
    }
}

#[test]
fn nothing_to_tidy_says_so() {
    let dir = tempfile::tempdir().unwrap();
    let editor = start(&dir);
    editor.invoke_tidy();
    editor.invoke_save();
    // let the container swap to the rebuilt preview, as the event loop would
    for _ in 0..3 {
        i_slint_backend_testing::mock_elapsed_time(std::time::Duration::from_millis(50));
        slint::platform::update_timers_and_animations();
        let _ = editor.window().take_snapshot();
    }
    // already tidy now
    editor.invoke_tidy();
    assert_eq!(editor.get_toast_text().as_str(), "整える箇所はありませんでした");
    assert_eq!(editor.get_tidy_marks().row_count(), 0);
}

#[test]
fn refusal_is_shown_in_the_error_band_and_changes_nothing() {
    let dir = tempfile::tempdir().unwrap();
    let editor = start(&dir);
    editor.invoke_tree_selected(2); // a leaf rectangle
    editor.invoke_tidy();
    assert!(!editor.get_error_text().is_empty());
    assert!(!editor.get_dirty());
}

#[test]
fn tolerance_steps_by_four_within_bounds_and_persists() {
    let dir = tempfile::tempdir().unwrap();
    let editor = start(&dir);
    assert_eq!(editor.get_tidy_tolerance(), 16);
    editor.invoke_tolerance_step(1);
    assert_eq!(editor.get_tidy_tolerance(), 20);
    assert_eq!(stored(&dir).as_deref(), Some("20"));
    for _ in 0..20 {
        editor.invoke_tolerance_step(1);
    }
    assert_eq!(editor.get_tidy_tolerance(), 48);
    for _ in 0..20 {
        editor.invoke_tolerance_step(-1);
    }
    assert_eq!(editor.get_tidy_tolerance(), 4);
    assert_eq!(stored(&dir).as_deref(), Some("4"));
}

#[test]
fn saved_tolerance_is_read_at_startup_and_used_by_tidy() {
    let dir = tempfile::tempdir().unwrap();
    {
        let store = LocalStore::new(dir.path().to_path_buf()).unwrap();
        store.create_project("default").unwrap();
        // 14 apart: aligned at T=16, not at T=4
        let text = src().replace("x: 14px", "x: 22px").replace("x: 12px", "x: 10px").replace("x: 22px; y: 110px", "x: 10px; y: 110px");
        store.write("default", "main.slint", &text).unwrap();
        store.set_setting("tidy_tolerance", "4").unwrap();
    }
    let editor = start(&dir);
    assert_eq!(editor.get_tidy_tolerance(), 4);
    editor.invoke_tidy();
    assert_eq!(editor.get_toast_text().as_str(), "整える箇所はありませんでした");
    assert!(!editor.get_dirty());
    for _ in 0..3 {
        editor.invoke_tolerance_step(1);
    }
    assert_eq!(editor.get_tidy_tolerance(), 16);
    editor.invoke_tidy();
    assert!(editor.get_dirty(), "T=16 aligns what T=4 left alone");
}

#[test]
fn bad_stored_tolerance_falls_back_to_the_default() {
    for (raw, want) in [("abc", 16), ("100", 48), ("1", 4), ("-5", 4)] {
        let dir = tempfile::tempdir().unwrap();
        std::fs::write(dir.path().join(".settings"), format!("tidy_tolerance={raw}\n")).unwrap();
        let editor = start(&dir);
        assert_eq!(editor.get_tidy_tolerance(), want, "{raw}");
    }
}

#[test]
fn a_later_action_clears_the_previous_error() {
    let dir = tempfile::tempdir().unwrap();
    let editor = start(&dir);
    editor.invoke_tree_selected(2); // a leaf rectangle
    editor.invoke_tidy();
    assert!(!editor.get_error_text().is_empty());
    editor.invoke_zoom(1.25);
    assert_eq!(editor.get_error_text().as_str(), "");
    editor.invoke_tidy();
    assert!(!editor.get_error_text().is_empty());
    editor.invoke_tapped(5., 5.);
    assert_eq!(editor.get_error_text().as_str(), "");
}
