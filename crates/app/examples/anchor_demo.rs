//! Desktop demo of the anchor chooser, pins and 付き方を推定 (for the README screenshots).
//!
//! `cargo run -p app --example anchor_demo -- <scene>`: `chooser`, `pin-end`, `pin-stretch`,
//! `before`, `after`, `infer`.

use std::cell::RefCell;
use std::rc::Rc;
use std::time::Duration;

use slint::ComponentHandle;
use storage::{LocalStore, ProjectStore};

const SCREEN: &str = "export component Main inherits Rectangle {\n    background: #f4f6f8;\n    bar := Rectangle { x: 20px; y: 20px; width: 760px; height: 40px; background: #4fc3f7; }\n    btn := Rectangle { x: 660px; y: 90px; width: 120px; height: 44px; background: #ffb74d; }\n    mid := Rectangle { x: 340px; y: 150px; width: 120px; height: 44px; background: #81c784; }\n}\n";
const SPREAD: &str = "export component Main inherits Rectangle {\n    width: 300px;\n    height: 300px;\n    background: #f4f6f8;\n    box := Rectangle {\n        x: 0px;\n        y: 0px;\n        width: 300px;\n        height: 300px;\n        Rectangle { x: 10px; y: 10px; width: 280px; height: 30px; background: #4fc3f7; }\n        Rectangle { x: 250px; y: 60px; width: 40px; height: 30px; background: #ffb74d; }\n        Rectangle { x: 130px; y: 90px; width: 40px; height: 30px; background: #81c784; }\n        Rectangle { x: 14px; y: 120px; width: 40px; height: 30px; background: #e57373; }\n    }\n}\n";

type Step = Box<dyn Fn(&app::Editor)>;

fn main() -> Result<(), slint::PlatformError> {
    let scene = std::env::args().nth(1).unwrap_or_else(|| "chooser".into());
    let root = std::env::temp_dir().join(format!("petaricon-anchor-demo-{scene}"));
    let _ = std::fs::remove_dir_all(&root);
    let store = LocalStore::new(root).map_err(|e| slint::PlatformError::Other(e.to_string()))?;
    store.create_project("default").unwrap();
    let infer = scene == "infer";
    store.write("default", "main.slint", if infer { SPREAD } else { SCREEN }).unwrap();
    app::font::install(&std::env::temp_dir().join("petaricon-fonts"));
    let editor = app::Editor::new()?;
    app::glue::start(&editor, Box::new(store))?;
    editor.window().set_size(slint::LogicalSize::new(1100., 700.));

    let setup: Vec<Step> = vec![
        Box::new(move |e| {
            if !infer {
                e.invoke_screen_size_chosen(800, 360);
            }
        }),
        Box::new(move |e| {
            if !infer {
                e.invoke_tree_selected(1);
                e.invoke_anchor_chosen(0, 2);
                e.invoke_tree_selected(2);
                e.invoke_anchor_chosen(0, 1);
                e.invoke_tree_selected(3);
                e.invoke_anchor_chosen(0, 3);
            }
        }),
    ];
    let scene_steps: Vec<Step> = match scene.as_str() {
        "chooser" => vec![Box::new(|e| { e.invoke_tree_selected(2); e.set_panel_tab(2); })],
        "pin-end" => vec![Box::new(|e| e.invoke_tree_selected(2))],
        "pin-stretch" => vec![Box::new(|e| e.invoke_tree_selected(1))],
        "before" => vec![Box::new(|e| e.invoke_tree_selected(2))],
        "after" => vec![
            Box::new(|e| e.invoke_tree_selected(2)),
            Box::new(|e| e.invoke_screen_size_chosen(914, 411)),
        ],
        "infer" => vec![
            Box::new(|e| e.invoke_tree_selected(1)),
            Box::new(|e| e.invoke_tidy()),
            Box::new(|e| e.invoke_infer_ask()),
        ],
        other => panic!("unknown scene {other}"),
    };
    let steps: Rc<Vec<Step>> = Rc::new(setup.into_iter().chain(scene_steps).collect());
    let next = Rc::new(RefCell::new(0usize));
    let timer = slint::Timer::default();
    let weak = editor.as_weak();
    timer.start(slint::TimerMode::Repeated, Duration::from_millis(500), move || {
        let Some(e) = weak.upgrade() else { return };
        let mut n = next.borrow_mut();
        if *n < steps.len() {
            steps[*n](&e);
            *n += 1;
        }
    });
    editor.run()
}
