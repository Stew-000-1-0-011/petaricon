//! A property edit never leaves the document broken, and a broken document stays escapable
//! (through the editor window, as the phone drives it).

use slint::{ComponentHandle, Model};
use storage::{LocalStore, ProjectStore};

const GOOD: &str = "export component Main inherits Rectangle {\n    width: 300px;\n    height: 300px;\n    Text { x: 0px; y: 0px; text: \"Hello\"; }\n    Rectangle { x: 0px; y: 100px; width: 40px; height: 40px; }\n}\n";

fn start(dir: &tempfile::TempDir, src: &str) -> app::Editor {
    let store = LocalStore::new(dir.path().to_path_buf()).unwrap();
    store.create_project("default").unwrap();
    store.write("default", "main.slint", src).unwrap();
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

fn saved(dir: &tempfile::TempDir) -> String {
    LocalStore::new(dir.path().to_path_buf()).unwrap().read("default", "main.slint").unwrap()
}

fn row(editor: &app::Editor, name: &str) -> app::PropItem {
    editor.get_props().iter().find(|p| p.name == name).unwrap()
}

#[test]
fn a_refused_value_keeps_the_field_and_shows_the_reason_at_it() {
    let dir = tempfile::tempdir().unwrap();
    let editor = start(&dir, GOOD);
    editor.invoke_tree_selected(1);
    assert_eq!(row(&editor, "text").value.as_str(), "Hello");

    editor.invoke_prop_accepted("text".into(), "=\"Hello\"abc".into());
    assert!(editor.get_error_text().starts_with("この値は使えません: "), "{}", editor.get_error_text());
    assert!(!row(&editor, "text").error.is_empty());
    assert_eq!(row(&editor, "text").value.as_str(), "Hello", "the model is not rewritten under the cursor");
    assert!(!editor.get_dirty());

    editor.invoke_prop_accepted("text".into(), "Hi \"there\"".into());
    assert_eq!(editor.get_error_text().as_str(), "");
    assert!(editor.get_dirty());
    assert_eq!(row(&editor, "text").value.as_str(), "Hi \"there\"");
    assert!(row(&editor, "text").error.is_empty());
    editor.invoke_save();
    assert!(saved(&dir).contains(r#"text: "Hi \"there\"";"#), "{}", saved(&dir));
}

#[test]
fn a_document_that_does_not_compile_can_be_escaped() {
    let dir = tempfile::tempdir().unwrap();
    let editor = start(&dir, &GOOD.replace("width: 40px", "width: \"abc\""));
    let hint = "(元に戻す か ソース で直せます)";
    assert!(editor.get_error_text().ends_with(hint), "{}", editor.get_error_text());
    // The tree lists everything and selects.
    assert_eq!(editor.get_tree_rows().row_count(), 3);
    editor.invoke_tree_selected(1);
    // Deleting the Text (not the broken element) changes the document; the band stays.
    editor.invoke_tree_action(4);
    assert!(editor.get_dirty());
    assert_eq!(editor.get_tree_rows().row_count(), 2);
    assert!(editor.get_error_text().ends_with(hint), "{}", editor.get_error_text());
    // Undo works with the error showing.
    editor.invoke_undo();
    assert!(!editor.get_dirty());
    assert_eq!(editor.get_tree_rows().row_count(), 3);
    // Deleting the broken element itself clears the error.
    editor.invoke_tree_selected(2);
    editor.invoke_tree_action(4);
    assert_eq!(editor.get_error_text().as_str(), "");
    assert_eq!(editor.get_tree_rows().row_count(), 2);
}
