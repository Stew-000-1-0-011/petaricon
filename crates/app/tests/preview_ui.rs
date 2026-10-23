//! Preview values and screen sizes through the editor window and the project's `.preview`.

use slint::{ComponentHandle, Model};
use storage::{LocalStore, ProjectStore};

const SRC: &str = "export component Main inherits Rectangle {\n    in property <bool> on;\n    in-out property <int> n: 1;\n    in property <string> label;\n    in property <[string]> names;\n    property <int> hidden;\n    Rectangle { x: 0px; y: 0px; width: 50px; height: 50px; }\n}\n";

fn start(dir: &tempfile::TempDir) -> app::Editor {
    let store = LocalStore::new(dir.path().to_path_buf()).unwrap();
    if store.projects().unwrap().is_empty() {
        store.create_project("default").unwrap();
        store.write("default", "main.slint", SRC).unwrap();
    }
    thread_local!(static INIT: () = i_slint_backend_testing::init_no_event_loop());
    INIT.with(|_| {});
    let editor = app::Editor::new().unwrap();
    app::glue::start(&editor, Box::new(store)).unwrap();
    editor.window().set_size(slint::LogicalSize::new(360., 760.));
    editor.show().unwrap();
    slint::platform::update_timers_and_animations();
    editor
}

fn rows(editor: &app::Editor) -> Vec<(String, bool, String, String)> {
    editor
        .get_preview_props()
        .iter()
        .map(|r| (r.name.to_string(), r.is_bool, r.value.to_string(), r.error.to_string()))
        .collect()
}

#[test]
fn choosing_800x360_sets_the_canvas_size_and_is_remembered() {
    let dir = tempfile::tempdir().unwrap();
    let editor = start(&dir);
    assert_eq!((editor.get_screen_w(), editor.get_screen_h()), (360, 640));
    editor.invoke_screen_size_chosen(800, 360);
    assert_eq!((editor.get_screen_w(), editor.get_screen_h()), (800, 360));
    // Free entry works too, and a bad one changes nothing.
    editor.invoke_prop_accepted("@size".into(), "914x411".into());
    assert_eq!((editor.get_screen_w(), editor.get_screen_h()), (914, 411));
    editor.invoke_prop_accepted("@size".into(), "wide".into());
    assert_eq!((editor.get_screen_w(), editor.get_screen_h()), (914, 411));
    assert!(!editor.get_error_text().is_empty());
    editor.invoke_screen_size_chosen(800, 360);
    drop(editor);
    let text = std::fs::read_to_string(dir.path().join("default/.preview")).unwrap();
    assert_eq!(text, "size\tmain.slint\tMain\t800\t360\n");
    // A new session starts with the remembered size.
    let again = start(&dir);
    assert_eq!((again.get_screen_w(), again.get_screen_h()), (800, 360));
    // The .slint file was never touched.
    assert_eq!(
        LocalStore::new(dir.path().to_path_buf()).unwrap().read("default", "main.slint").unwrap(),
        SRC
    );
}

#[test]
fn values_are_listed_typed_saved_and_bad_ones_report_a_reason() {
    let dir = tempfile::tempdir().unwrap();
    let editor = start(&dir);
    let r = rows(&editor);
    let names: Vec<&str> = r.iter().map(|x| x.0.as_str()).collect();
    assert_eq!(names, ["on", "n", "label", "names"]);
    assert!(r[0].1 && !r[1].1 && !r[2].1);

    editor.invoke_preview_set("on".into(), "true".into());
    editor.invoke_prop_accepted("@n".into(), "7".into());
    editor.invoke_prop_accepted("@label".into(), "こんにちは".into());
    editor.invoke_prop_accepted("@names".into(), "{ a: 1 }".into());
    let r = rows(&editor);
    assert_eq!(r[0].2, "true");
    assert_eq!(r[2].2, "こんにちは");
    assert!(r[0].3.is_empty() && r[1].3.is_empty() && r[2].3.is_empty());
    assert!(!r[3].3.is_empty(), "wrong element type must be reported: {r:?}");
    assert!(editor.get_error_text().is_empty());

    let text = std::fs::read_to_string(dir.path().join("default/.preview")).unwrap();
    assert!(text.contains("value\tmain.slint\tMain\ton\ttrue\n"), "{text}");
    assert!(text.contains("value\tmain.slint\tMain\tn\t7\n"), "{text}");
    assert!(text.contains("value\tmain.slint\tMain\tlabel\t\"こんにちは\"\n"), "{text}");

    // Tapping the selected 真 clears the value.
    editor.invoke_preview_set("on".into(), "".into());
    assert_eq!(rows(&editor)[0].2, "");
    // The values come back in a new session.
    drop(editor);
    let again = start(&dir);
    let r = rows(&again);
    assert_eq!(r[1].2, "7");
    assert_eq!(r[2].2, "こんにちは");
}

#[test]
fn changing_the_screen_size_fits_the_whole_screen_in_the_canvas() {
    let dir = tempfile::tempdir().unwrap();
    let editor = start(&dir);
    for (w, h) in [(800, 360), (1280, 720), (360, 640)] {
        editor.invoke_screen_size_chosen(w, h);
        slint::platform::update_timers_and_animations();
        let z = editor.get_zoom_level();
        let (vw, vh) = (editor.get_canvas_view_w(), editor.get_canvas_view_h());
        assert!(vw > 100. && vh > 100., "{vw}x{vh}");
        assert!(w as f32 * z <= vw && h as f32 * z <= vh, "{w}x{h} at {z} in {vw}x{vh}");
        assert!(editor.get_canvas_pan_x() >= 0. && editor.get_canvas_pan_y() >= 0.);
    }
}
