use std::path::PathBuf;

fn main() -> Result<(), slint::PlatformError> {
    let root = std::env::args_os()
        .nth(1)
        .map(PathBuf::from)
        .unwrap_or_else(|| PathBuf::from("./projects"));
    let store = storage::LocalStore::new(root)
        .map_err(|e| slint::PlatformError::Other(e.to_string()))?;
    app::run(Box::new(store), PathBuf::from("./logs"))
}
