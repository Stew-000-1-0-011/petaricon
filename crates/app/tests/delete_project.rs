//! Deleting a project from the menu: confirm dialog, the open project, the GitHub warning.

use slint::{ComponentHandle, Model};
use storage::{LocalStore, ProjectStore};

const SRC: &str = "export component Main inherits Rectangle {\n    width: 300px;\n    height: 300px;\n}\n";

fn seeded() -> (tempfile::TempDir, LocalStore) {
    let dir = tempfile::tempdir().unwrap();
    let store = LocalStore::new(dir.path().to_path_buf()).unwrap();
    for p in ["default", "second"] {
        store.create_project(p).unwrap();
        store.write(p, "main.slint", SRC).unwrap();
    }
    (dir, store)
}

fn start(store: LocalStore) -> app::Editor {
    i_slint_backend_testing::init_no_event_loop();
    let editor = app::Editor::new().unwrap();
    app::glue::start(&editor, Box::new(store)).unwrap();
    editor.window().set_size(slint::LogicalSize::new(480., 800.));
    editor.show().unwrap();
    slint::platform::update_timers_and_animations();
    let _ = editor.window().size();
    editor
}

fn strings(m: slint::ModelRc<slint::SharedString>) -> Vec<String> {
    m.iter().map(|s| s.to_string()).collect()
}

#[test]
fn deleting_a_project_that_is_not_open_updates_the_list() {
    let (dir, store) = seeded();
    let e = start(store);
    assert_eq!(e.get_current_project().as_str(), "default");
    e.invoke_delete_ask("second".into());
    assert!(e.get_delete_open());
    assert_eq!(e.get_delete_project().as_str(), "second");
    assert_eq!(e.get_delete_warning().as_str(), "", "no .sync: nothing to warn about");
    e.invoke_delete_run("second".into());
    assert!(!e.get_delete_open());
    assert_eq!(strings(e.get_projects()), vec!["default"]);
    assert_eq!(e.get_current_project().as_str(), "default", "the open project stays");
    assert_eq!(e.get_toast_text().as_str(), "削除しました");
    assert!(!dir.path().join("second").exists());
    assert_eq!(e.get_error_text().as_str(), "");
}

#[test]
fn deleting_the_open_project_opens_another() {
    let (dir, store) = seeded();
    let e = start(store);
    e.invoke_delete_ask("default".into());
    e.invoke_delete_run("default".into());
    assert_eq!(strings(e.get_projects()), vec!["second"]);
    assert_eq!(e.get_current_project().as_str(), "second");
    assert_eq!(e.get_current_file().as_str(), "main.slint");
    assert_eq!(e.get_toast_text().as_str(), "削除しました");
    assert!(!dir.path().join("default").exists());
    // Nothing of the old project comes back (recovery timer, last open).
    i_slint_backend_testing::mock_elapsed_time(std::time::Duration::from_secs(10));
    slint::platform::update_timers_and_animations();
    assert!(!dir.path().join("default").exists());
    assert_eq!(
        LocalStore::new(dir.path().to_path_buf()).unwrap().last_open().unwrap(),
        Some(("second".into(), "main.slint".into()))
    );
}

#[test]
fn deleting_the_last_project_starts_over_like_a_fresh_install() {
    let (dir, store) = seeded();
    let e = start(store);
    for p in ["second", "default"] {
        e.invoke_delete_ask(p.into());
        e.invoke_delete_run(p.into());
    }
    assert_eq!(strings(e.get_projects()), vec!["default"]);
    assert_eq!(e.get_current_project().as_str(), "default");
    assert_eq!(e.get_current_file().as_str(), "main.slint");
    assert!(dir.path().join("default/main.slint").is_file());
    assert_eq!(e.get_error_text().as_str(), "");
}

#[test]
fn unsaved_edits_of_the_deleted_open_project_are_dropped() {
    let (dir, store) = seeded();
    store.write("default", "main.slint", "export component Main inherits Rectangle {\n    width: 300px;\n    height: 300px;\n    a := Rectangle {\n        x: 10px;\n        y: 10px;\n        width: 100px;\n        height: 100px;\n    }\n}\n").unwrap();
    let e = start(store);
    let z = e.get_zoom_level();
    e.invoke_tapped(60. * z, 60. * z);
    e.invoke_drag_finished(40. * z, 40. * z);
    assert!(e.get_dirty());
    e.invoke_delete_ask("default".into());
    e.invoke_delete_run("default".into());
    assert!(!e.get_dirty());
    assert_eq!(e.get_current_project().as_str(), "second");
    i_slint_backend_testing::mock_elapsed_time(std::time::Duration::from_secs(10));
    slint::platform::update_timers_and_animations();
    assert!(!dir.path().join("default").exists());
}

#[test]
fn cancelling_changes_nothing() {
    let (dir, store) = seeded();
    let e = start(store);
    e.invoke_delete_ask("second".into());
    assert!(e.get_delete_open());
    // The dialog's キャンセル sets this; nothing is deleted until delete-run.
    e.set_delete_open(false);
    assert_eq!(strings(e.get_projects()), vec!["default", "second"]);
    assert!(dir.path().join("second/main.slint").is_file());
    assert_eq!(e.get_toast_text().as_str(), "");
    assert_eq!(e.get_current_project().as_str(), "default");
}

#[test]
fn an_unknown_project_is_refused() {
    let (_dir, store) = seeded();
    let e = start(store);
    e.invoke_delete_ask("nope".into());
    assert!(!e.get_delete_open());
    assert!(e.get_error_text().contains("nope"));
    e.invoke_delete_run("../x".into());
    assert!(e.get_error_text().contains("削除できませんでした"));
    assert_eq!(strings(e.get_projects()), vec!["default", "second"]);
}
