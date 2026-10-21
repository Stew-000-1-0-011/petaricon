//! The input bar above the on-screen keyboard (driven with PETARICON_FAKE_KEYBOARD's property).

use slint::platform::{PointerEventButton, WindowEvent};
use slint::{ComponentHandle, LogicalPosition};
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

/// Selects the box, shows the Props tab and taps the first value field under the fake keyboard.
fn open_bar(editor: &app::Editor) {
    editor.invoke_tapped(60., 60.);
    assert!(editor.get_has_selection());
    editor.set_panel_tab(2);
    slint::platform::update_timers_and_animations();
    // Portrait: toolbar 2 rows (104) + panel is the bottom 40% of the rest; handle 18, tabs 44.
    let top = 760.0 - (760.0 - 104.0) * 0.4;
    click(editor, 250., top + 18.0 + 44.0 + 22.0);
    assert!(editor.get_bar_visible(), "bar should open for a covered field");
}

fn key(editor: &app::Editor, text: impl Into<slint::SharedString>) {
    let text = text.into();
    editor.window().dispatch_event(WindowEvent::KeyPressed { text: text.clone() });
    editor.window().dispatch_event(WindowEvent::KeyReleased { text });
}

/// Replaces the bar's text by typing into it.
fn retype(editor: &app::Editor, value: &str) {
    key(editor, char::from(slint::platform::Key::End));
    for _ in 0..editor.get_bar_text().chars().count() {
        key(editor, char::from(slint::platform::Key::Backspace));
    }
    for c in value.chars() {
        key(editor, c);
    }
    assert_eq!(editor.get_bar_text().as_str(), value);
}

fn elapse(ms: u64) {
    
    i_slint_backend_testing::mock_elapsed_time(std::time::Duration::from_millis(ms));
    slint::platform::update_timers_and_animations();
}

fn saved_after(editor: &app::Editor, dir: &tempfile::TempDir) -> String {
    editor.invoke_save();
    saved(dir)
}

#[test]
fn typing_in_the_bar_applies_once_after_the_debounce() {
    let (dir, editor) = start(300.);
    open_bar(&editor);
    let before = editor.get_bar_text().to_string();
    retype(&editor, "77px");
    assert_eq!(editor.get_bar_text().as_str(), "77px");
    assert!(!editor.get_dirty(), "not applied before the debounce");
    elapse(400);
    assert!(editor.get_dirty());
    assert!(saved_after(&editor, &dir).contains("77px"));
    // One history entry: a single undo restores the original.
    editor.invoke_undo();
    assert_eq!(saved_after(&editor, &dir), SRC, "{before}");
}

#[test]
fn ok_then_keyboard_hide_does_not_apply_twice() {
    let (dir, editor) = start(300.);
    open_bar(&editor);
    retype(&editor, "66px");
    key(&editor, char::from(slint::platform::Key::Return));
    slint::platform::update_timers_and_animations();
    assert!(!editor.get_bar_visible(), "OK closes the bar");
    assert!(saved_after(&editor, &dir).contains("66px"));
    editor.set_fake_keyboard(0.);
    slint::platform::update_timers_and_animations();
    elapse(400);
    editor.invoke_undo();
    assert_eq!(saved_after(&editor, &dir), SRC, "a second apply left another history entry");
}

#[test]
fn keyboard_hide_flushes_a_pending_bar_edit() {
    let (dir, editor) = start(300.);
    open_bar(&editor);
    retype(&editor, "55px");
    assert!(!editor.get_dirty());
    editor.set_fake_keyboard(0.);
    slint::platform::update_timers_and_animations();
    assert!(!editor.get_bar_visible());
    assert!(editor.get_dirty(), "hide applies the pending edit without waiting");
    assert!(saved_after(&editor, &dir).contains("55px"));
    // The stale field must not bring the bar back with the keyboard.
    editor.set_fake_keyboard(300.);
    slint::platform::update_timers_and_animations();
    assert!(!editor.get_bar_visible());
}

#[test]
fn tapping_the_canvas_closes_the_bar_and_leaves_the_new_selection_alone() {
    let (dir, editor) = start(300.);
    open_bar(&editor);
    retype(&editor, "44px");
    // Tap empty canvas: the pending edit goes to the old target, the selection changes.
    editor.invoke_tapped(250., 250.);
    slint::platform::update_timers_and_animations();
    assert!(!editor.get_bar_visible());
    let applied = saved_after(&editor, &dir);
    assert!(applied.contains("44px"), "pending edit goes to the original target");
    // Further typing after the bar is gone changes nothing.
    key(&editor, '9');
    elapse(400);
    assert_eq!(saved_after(&editor, &dir), applied);
    // Keyboard toggling does not resurrect the stale target.
    editor.set_fake_keyboard(0.);
    editor.set_fake_keyboard(300.);
    slint::platform::update_timers_and_animations();
    assert!(!editor.get_bar_visible());
}
