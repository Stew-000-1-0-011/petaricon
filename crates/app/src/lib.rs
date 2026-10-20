pub mod crashlog;
pub mod glue;
pub mod guides;
pub mod layout_bridge;
pub mod session;

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
    slint::android::init_with_event_listener(app, |event| {
        if let PollEvent::Main(MainEvent::Pause) = event {
            glue::save_now();
        }
    })
    .unwrap();
    let store = storage::LocalStore::new(base.join("projects")).expect("projects dir");
    run(Box::new(store), base.join("logs")).unwrap();
}
