use std::path::Path;

use preview::compile;
use tidy::Rect;

const CARD: &str = r#"component Card inherits Rectangle { Text { x: 10px; y: 20px; width: 50px; height: 10px; text: "x"; } }"#;

#[test]
fn compiles_non_exported_component() {
    i_slint_backend_testing::init_no_event_loop();
    assert!(compile(CARD, Path::new("/t/card.slint"), "Card").is_ok());
}

#[test]
fn reports_error_offset() {
    i_slint_backend_testing::init_no_event_loop();
    let text = "export component A { Text { text: 1 +; } }";
    let Err(errs) = compile(text, Path::new("/t/a.slint"), "A") else { panic!("expected Err") };
    let at = text.find("+;").unwrap() as i64;
    let n = errs[0].offset.expect("offset") as i64;
    assert!((n - at).abs() <= 3, "offset {n} vs {at}");
}

#[test]
fn hit_returns_deepest_first() {
    i_slint_backend_testing::init_no_event_loop();
    let c = compile(CARD, Path::new("/t/card.slint"), "Card").unwrap();
    let inst = c.instantiate(200., 100.);
    let hits = inst.hits_at(15., 25.);
    let start = CARD.find("Text").unwrap() as u32;
    assert_eq!(hits[0].start, start);
    assert_eq!(hits[0].local, Rect { x: 10., y: 20., w: 50., h: 10. });
    assert!(inst.hits_at(150., 80.).is_empty());
    assert_eq!(inst.geometry(start).unwrap().local, hits[0].local);
}
