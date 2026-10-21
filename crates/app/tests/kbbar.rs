//! The input bar above the on-screen keyboard (driven with PETARICON_FAKE_KEYBOARD's property).

use slint::platform::{PointerEventButton, WindowEvent};
use slint::{ComponentHandle, LogicalPosition, Model};
use storage::{LocalStore, ProjectStore};

const SRC: &str = "export component Main inherits Rectangle {\n    width: 300px;\n    height: 300px;\n    a := Rectangle {\n        x: 10px;\n        y: 10px;\n        width: 100px;\n        height: 100px;\n    }\n}\n";

fn start(h: f32) -> (tempfile::TempDir, app::Editor) {
    let dir = tempfile::tempdir().unwrap();
    let store = LocalStore::new(dir.path().to_path_buf()).unwrap();
    store.create_project("default").unwrap();
    store.write("default", "main.slint", SRC).unwrap();
    i_slint_backend_testing::init_no_event_loop();
    let editor = app::Editor::new().unwrap();
    app::glue::start(&editor, Box::new(store)).unwrap();
    editor
        .window()
        .set_size(slint::LogicalSize::new(360., 760.));
    editor.set_fake_keyboard(h);
    editor.show().unwrap();
    slint::platform::update_timers_and_animations();
    let _ = editor.window().size();
    (dir, editor)
}

fn click(editor: &app::Editor, x: f32, y: f32) {
    let w = editor.window();
    let position = LogicalPosition::new(x, y);
    w.dispatch_event(WindowEvent::PointerMoved { position });
    w.dispatch_event(WindowEvent::PointerPressed {
        position,
        button: PointerEventButton::Left,
    });
    w.dispatch_event(WindowEvent::PointerReleased {
        position,
        button: PointerEventButton::Left,
    });
    slint::platform::update_timers_and_animations();
}

fn saved(dir: &tempfile::TempDir) -> String {
    LocalStore::new(dir.path().to_path_buf())
        .unwrap()
        .read("default", "main.slint")
        .unwrap()
}

#[test]
fn no_keyboard_means_no_bar() {
    let (_d, editor) = start(0.);
    assert!(!editor.get_kb_shown());
    assert!(!editor.get_bar_visible());
}

#[test]
fn bar_opens_for_a_covered_props_field_and_commits_like_the_field() {
    let (dir, editor) = start(300.);
    assert!(editor.get_kb_shown());
    editor.invoke_tapped(60., 60.);
    assert!(editor.get_has_selection());
    editor.set_panel_tab(2);
    slint::platform::update_timers_and_animations();
    // Panel is the bottom 40% (hidden behind the fake keyboard); first row's value field.
    let top = 760.0 - (760.0 - 52.0 - 52.0) * 0.4;
    let row_y = top + 18.0 + 44.0 + 22.0;
    assert!(!editor.get_bar_visible());
    click(&editor, 250., row_y);
    assert!(
        editor.get_bar_visible(),
        "bar should open for a covered field"
    );
    let name = editor.get_props().row_data(0).unwrap();
    assert_eq!(editor.get_bar_text().as_str(), name.value.as_str());

    // Typing in the bar and OK go through the same callbacks as the field itself.
    editor.invoke_prop_accepted(name.name.clone(), "77px".into());
    assert!(editor.get_dirty());
    editor.invoke_save();
    // Whichever property row 0 is, it now holds 77px in the file.
    assert!(saved(&dir).contains("77px"), "{}", saved(&dir));
}

#[test]
fn flush_applies_a_pending_debounced_edit() {
    let (dir, editor) = start(300.);
    editor.invoke_tapped(60., 60.);
    let name = editor.get_props().row_data(0).unwrap();
    editor.invoke_prop_edited(name.name.clone(), "55px".into());
    editor.invoke_prop_flush();
    editor.invoke_save();
    assert!(saved(&dir).contains("55px"), "{}", saved(&dir));
}
