//! Writes panics (message, location and backtrace) to `{log_dir}/crash-{unix seconds}.log`.

use std::backtrace::Backtrace;
use std::fs;
use std::path::Path;
use std::time::{SystemTime, UNIX_EPOCH};

pub fn install(log_dir: &Path) {
    let dir = log_dir.to_path_buf();
    let previous = std::panic::take_hook();
    std::panic::set_hook(Box::new(move |info| {
        let secs = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .map(|d| d.as_secs())
            .unwrap_or(0);
        let report = format!("{info}\n\n{}\n", Backtrace::force_capture());
        let _ = fs::create_dir_all(&dir);
        let _ = fs::write(dir.join(format!("crash-{secs}.log")), report);
        previous(info);
    }));
}
