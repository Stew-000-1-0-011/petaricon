//! The 引数にする dialog belongs to the element it was opened for.

use slint::{ComponentHandle, Model};
use storage::{LocalStore, ProjectStore};

const SRC: &str = "export component Main inherits Rectangle {\n    width: 300px;\n    height: 300px;\n    Text { x: 0px; y: 0px; text: \"Hello\"; }\n    Text { x: 0px; y: 50px; text: \"Other\"; }\n}\n";

fn start(dir: &tempfile::TempDir) -> app::Editor {
    let store = LocalStore::new(dir.path().to_path_buf()).unwrap();
    store.create_project("default").unwrap();
    store.write("default", "main.slint", SRC).unwrap();
    thread_local!(static INIT: () = i_slint_backend_testing::init_no_event_loop());
    INIT.with(|_| {});
    let editor = app::Editor::new().unwrap();
    app::glue::start(&editor, Box::new(store)).unwrap();
    editor.window().set_size(slint::LogicalSize::new(360., 760.));
    editor.show().unwrap();
    slint::platform::update_timers_and_animations();
    let _ = editor.window().size();
    editor
}

fn text_row(editor: &app::Editor) -> app::PropItem {
    editor.get_props().iter().find(|p| p.name == "text").unwrap()
}

#[test]
fn the_dialog_applies_to_the_element_it_was_opened_for() {
    let dir = tempfile::tempdir().unwrap();
    let editor = start(&dir);
    editor.invoke_tree_selected(1);
    assert!(text_row(&editor).can_param);
    editor.invoke_param_ask("text".into());
    assert!(editor.get_param_open());
    editor.invoke_param_run("text-text".into(), "".into());
    assert!(!editor.get_param_open());
    assert!(editor.get_dirty());
}

#[test]
fn a_changed_selection_refuses_and_closes_the_dialog() {
    let dir = tempfile::tempdir().unwrap();
    let editor = start(&dir);
    editor.invoke_tree_selected(1);
    editor.invoke_param_ask("text".into());
    assert!(editor.get_param_open());
    // Something else is selected while the dialog is open.
    editor.invoke_tree_selected(2);
    editor.invoke_param_run("text-text".into(), "".into());
    assert!(!editor.get_param_open());
    assert!(!editor.get_dirty(), "nothing was changed");
    assert!(!editor.get_error_text().is_empty());
    // And the document changing under it does the same.
    editor.invoke_tree_selected(1);
    editor.invoke_param_ask("text".into());
    editor.invoke_prop_accepted("text".into(), "Changed".into());
    editor.invoke_param_run("text-text".into(), "".into());
    assert!(!editor.get_param_open());
    assert!(editor.get_error_text().contains("引数にしませんでした"), "{}", editor.get_error_text());
}
