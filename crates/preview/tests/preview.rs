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
    let Err(errs) = compile(text, Path::new("/t/a.slint"), "A") else {
        panic!("expected Err")
    };
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
    assert_eq!(
        hits[0].local,
        Rect {
            x: 10.,
            y: 20.,
            w: 50.,
            h: 10.
        }
    );
    assert!(inst.hits_at(150., 80.).is_empty());
    assert_eq!(inst.geometry(start).unwrap().local, hits[0].local);
}

#[test]
fn error_offset_with_multibyte_text_on_same_line() {
    i_slint_backend_testing::init_no_event_loop();
    let text = "export component A { Text { text: \"あい\"; x: 1 +; } }";
    let Err(errs) = compile(text, Path::new("/t/a.slint"), "A") else {
        panic!("expected Err")
    };
    let at = text.find("+;").unwrap() as i64;
    let n = errs[0].offset.expect("offset") as i64;
    assert!((n - at).abs() <= 3, "offset {n} vs {at}");
}

#[test]
fn nested_hits_are_ordered_deepest_first() {
    i_slint_backend_testing::init_no_event_loop();
    let t = "component C inherits Rectangle { Rectangle { x: 5px; y: 5px; width: 100px; height: 60px; Rectangle { x: 5px; y: 5px; width: 50px; height: 30px; Text { text: \"t\"; width: 20px; height: 10px; } } } }";
    let c = compile(t, Path::new("/t/c.slint"), "C").unwrap();
    let inst = c.instantiate(200., 100.);
    let rects: Vec<usize> = t.match_indices("Rectangle {").map(|(i, _)| i).collect();
    // rects[0] is the component's base type `inherits Rectangle {`.
    let text_at = t.find("Text").unwrap() as u32;
    let starts: Vec<u32> = inst.hits_at(27., 27.).iter().map(|h| h.start).collect();
    assert_eq!(starts, vec![text_at, rects[2] as u32, rects[1] as u32]);
    // Outside the inner ones but inside the outer rectangle.
    let starts: Vec<u32> = inst.hits_at(90., 50.).iter().map(|h| h.start).collect();
    assert_eq!(starts, vec![rects[1] as u32]);
}

#[test]
fn used_subcomponent_contents_are_hit() {
    i_slint_backend_testing::init_no_event_loop();
    let t = "component Chip inherits Rectangle { Text { x: 3px; y: 4px; width: 30px; height: 10px; text: \"c\"; } }\ncomponent Host inherits Rectangle { Chip { x: 50px; y: 10px; width: 60px; height: 40px; } }";
    let c = compile(t, Path::new("/t/h.slint"), "Host").unwrap();
    let inst = c.instantiate(200., 100.);
    let text_at = t.find("Text").unwrap() as u32;
    let chip_use = t.rfind("Chip {").unwrap() as u32;
    let starts: Vec<u32> = inst.hits_at(55., 16.).iter().map(|h| h.start).collect();
    assert_eq!(starts, vec![text_at, chip_use]);
}

#[test]
fn root_geometry_is_the_component_root() {
    i_slint_backend_testing::init_no_event_loop();
    let c = compile(CARD, Path::new("/t/card.slint"), "Card").unwrap();
    let inst = c.instantiate(200., 100.);
    let root = inst.root_geometry().expect("root geometry");
    assert_eq!((root.rect.x, root.rect.y, root.rect.w, root.rect.h), (0., 0., 200., 100.));
}
