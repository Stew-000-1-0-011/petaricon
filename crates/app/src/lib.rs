pub mod anchor;
pub mod crashlog;
pub mod font;
pub mod github;
pub mod glue;
pub mod guides;
pub mod layout_bridge;
pub mod lockstore;
pub mod nav;
pub mod session;
pub mod tidy_overlay;

use std::path::PathBuf;

use storage::ProjectStore;

slint::include_modules!();

/// Runs the editor until its window is closed.
pub fn run(store: Box<dyn ProjectStore>, log_dir: PathBuf) -> Result<(), slint::PlatformError> {
    crashlog::install(&log_dir);
    let editor = Editor::new()?;
    glue::start(&editor, store)?;
    editor.run()
}

#[cfg(target_os = "android")]
#[unsafe(no_mangle)]
fn android_main(app: slint::android::AndroidApp) {
    use slint::android::android_activity::{MainEvent, PollEvent};

    let base = app.external_data_path().expect("no external data path");
    // The GitHub token goes to private storage; `base` is visible over USB.
    let private = app.internal_data_path().expect("no internal data path");
    // Before the Slint backend starts, so its font collection sees the variable.
    match app.internal_data_path() {
        Some(dir) => font::install(&dir.join("fonts")),
        None => font::install(&base.join("fonts")),
    }
    slint::android::init_with_event_listener(app, |event| {
        if let PollEvent::Main(MainEvent::Pause) = event {
            glue::save_now();
        }
    })
    .unwrap();
    let store = storage::LocalStore::with_private_dir(base.join("projects"), private)
        .expect("projects dir");
    run(Box::new(store), base.join("logs")).unwrap();
}
