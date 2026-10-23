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
fn used_subcomponent_is_one_node_its_contents_are_not_hit() {
    i_slint_backend_testing::init_no_event_loop();
    let t = "component Chip inherits Rectangle { Text { x: 3px; y: 4px; width: 30px; height: 10px; text: \"c\"; } }\ncomponent Host inherits Rectangle { Chip { x: 50px; y: 10px; width: 60px; height: 40px; } }";
    let c = compile(t, Path::new("/t/h.slint"), "Host").unwrap();
    let inst = c.instantiate(200., 100.);
    let chip_use = t.rfind("Chip {").unwrap() as u32;
    // A tap on the Text inside the use reports the use only.
    let starts: Vec<u32> = inst.hits_at(55., 16.).iter().map(|h| h.start).collect();
    assert_eq!(starts, vec![chip_use]);
}

#[test]
fn root_geometry_is_the_component_root() {
    i_slint_backend_testing::init_no_event_loop();
    let c = compile(CARD, Path::new("/t/card.slint"), "Card").unwrap();
    let inst = c.instantiate(200., 100.);
    let root = inst.root_geometry().expect("root geometry");
    assert_eq!((root.rect.x, root.rect.y, root.rect.w, root.rect.h), (0., 0., 200., 100.));
}

const PARAM: &str = "component P inherits Rectangle { in property <int> n: 1; in property <string> label; in property <bool> on; Rectangle { x: 0px; y: 0px; width: root.on ? root.n * 10px : 5px; height: 10px; } Text { text: root.label; } }";

fn v(n: &str, e: &str) -> (String, String) {
    (n.to_string(), e.to_string())
}

#[test]
fn values_reach_the_component() {
    i_slint_backend_testing::init_no_event_loop();
    let b = preview::compile_with(PARAM, Path::new("/t/p.slint"), "P", &[v("n", "4"), v("on", "true")]).unwrap();
    assert!(b.dropped.is_empty(), "{:?}", b.dropped);
    let inst = b.compiled.instantiate(200., 100.);
    let start = PARAM.find("Rectangle {").unwrap() as u32;
    // `PARAM` starts with `component P inherits Rectangle {`; the child is the second.
    let start = start + PARAM[start as usize + 1..].find("Rectangle {").unwrap() as u32 + 1;
    assert_eq!(inst.geometry(start).unwrap().rect.w, 40.);
}

#[test]
fn a_bad_value_is_dropped_with_a_reason_and_the_others_apply() {
    i_slint_backend_testing::init_no_event_loop();
    let vals = [v("n", "4"), v("on", "true"), v("label", "1 +"), v("missing", "3"), v("n2", "x; } }")];
    let b = preview::compile_with(PARAM, Path::new("/t/p.slint"), "P", &vals).unwrap();
    let names: Vec<&str> = b.dropped.iter().map(|d| d.0.as_str()).collect();
    assert_eq!(names, ["n2", "label", "missing"]);
    assert!(b.dropped.iter().all(|d| !d.1.is_empty()));
    let inst = b.compiled.instantiate(200., 100.);
    let second = PARAM.rfind("Rectangle {").unwrap() as u32;
    assert_eq!(inst.geometry(second).unwrap().rect.w, 40.);
}

#[test]
fn broken_text_still_fails_even_with_values() {
    i_slint_backend_testing::init_no_event_loop();
    let text = "component P inherits Rectangle { Text { text: 1 +; } }";
    assert!(preview::compile_with(text, Path::new("/t/p.slint"), "P", &[v("n", "1")]).is_err());
}

#[test]
fn hostile_names_and_comment_tricks_cannot_merge_or_escape_bindings() {
    i_slint_backend_testing::init_no_event_loop();
    let vals = [
        v("x:1px;}Rectangle{y", "1"),
        v("n", "1 /*"),
        v("label", "*/ \"a\""),
        v("on", "true // x"),
        v("label2", "\"a\\{1}\""),
        v("n", "4"),
    ];
    let b = preview::compile_with(PARAM, Path::new("/t/p.slint"), "P", &vals).unwrap();
    // `n: 4` is the only good one; every other entry is dropped with a reason.
    assert_eq!(b.dropped.len(), 5, "{:?}", b.dropped);
    assert!(b.dropped.iter().all(|d| !d.1.is_empty()));
    let inst = b.compiled.instantiate(200., 100.);
    let second = PARAM.rfind("Rectangle {").unwrap() as u32;
    // `on` is false, so the width stays 5px; and no element was added by the hostile name.
    assert_eq!(inst.geometry(second).unwrap().rect.w, 5.);
}

#[test]
fn a_known_bad_value_costs_one_compile_after_the_first_time() {
    i_slint_backend_testing::init_no_event_loop();
    let vals = [v("n", "4"), v("on", "true"), v("label", "1 +")];
    let text = format!("{PARAM}\n// cache probe");
    let first = preview::compile_with(&text, Path::new("/t/p.slint"), "P", &vals).unwrap();
    assert_eq!(first.dropped.len(), 1);
    let before = preview::compile_count();
    let again = preview::compile_with(&text, Path::new("/t/p.slint"), "P", &vals).unwrap();
    assert_eq!(again.dropped.len(), 1);
    assert_eq!(preview::compile_count() - before, 1);
}
