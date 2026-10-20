#[test]
fn panic_is_written_to_a_log_file() {
    let dir = std::env::temp_dir().join(format!("petaricon-crashlog-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&dir);
    app::crashlog::install(&dir);
    let _ = std::panic::catch_unwind(|| panic!("boom for the crash log"));
    let log = std::fs::read_dir(&dir)
        .expect("log dir created")
        .map(|e| e.unwrap().path())
        .find(|p| p.file_name().unwrap().to_string_lossy().starts_with("crash-"))
        .expect("a crash-*.log file");
    let text = std::fs::read_to_string(log).unwrap();
    assert!(text.contains("boom for the crash log"), "{text}");
    assert!(text.contains("stack backtrace") || text.contains("backtrace"), "{text}");
    let _ = std::fs::remove_dir_all(&dir);
}
