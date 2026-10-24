//! Anchors in the editor window: the chooser, the Stretch size row, the drag notice and
//! 付き方を推定 (also at the session level, on real preview geometry).

use std::path::PathBuf;

use app::anchor::Kind;
use app::session::Session;
use slint::{ComponentHandle, Model};
use slint_doc::{Document, ElementPath};
use storage::{LocalStore, ProjectStore};

fn kids(list: &[(&str, &str, &str, &str)]) -> String {
    let mut out = String::from(
        "export component Main inherits Rectangle {\n    width: 300px;\n    height: 300px;\n    box := Rectangle {\n        x: 0px;\n        y: 0px;\n        width: 300px;\n        height: 300px;\n",
    );
    for (x, y, w, h) in list {
        out += &format!(
            "        Rectangle {{ x: {x}; y: {y}; width: {w}; height: {h}; }}\n"
        );
    }
    out + "    }\n}\n"
}

fn start(dir: &tempfile::TempDir, src: &str) -> app::Editor {
    let store = LocalStore::new(dir.path().to_path_buf()).unwrap();
    store.create_project("default").unwrap();
    store.write("default", "main.slint", src).unwrap();
    thread_local!(static INIT: () = i_slint_backend_testing::init_no_event_loop());
    INIT.with(|_| {});
    let editor = app::Editor::new().unwrap();
    app::glue::start(&editor, Box::new(store)).unwrap();
    editor.window().set_size(slint::LogicalSize::new(360., 760.));
    editor.show().unwrap();
    slint::platform::update_timers_and_animations();
    let _ = editor.window().size();
    editor
}

fn saved(dir: &tempfile::TempDir) -> String {
    LocalStore::new(dir.path().to_path_buf()).unwrap().read("default", "main.slint").unwrap()
}

fn row(editor: &app::Editor, name: &str) -> app::PropItem {
    editor.get_props().iter().find(|p| p.name == name).expect("row")
}

fn rect(editor: &app::Editor) -> (f32, f32, f32, f32) {
    (editor.get_sel_x(), editor.get_sel_y(), editor.get_sel_w(), editor.get_sel_h())
}

const ONE: &[(&str, &str, &str, &str)] = &[("40px", "16px", "60px", "30px")];

#[test]
fn the_chooser_shows_the_anchors_and_switching_keeps_the_rectangle() {
    let dir = tempfile::tempdir().unwrap();
    let editor = start(&dir, &kids(ONE));
    editor.invoke_tree_selected(2);
    assert_eq!((editor.get_anchor_h(), editor.get_anchor_v()), (0, 0));
    let before = rect(&editor);
    for (axis, kind) in [(0, 1), (1, 3), (0, 3), (1, 1), (0, 2), (1, 2), (0, 0), (1, 0)] {
        editor.invoke_anchor_chosen(axis, kind);
        let now = if axis == 0 { editor.get_anchor_h() } else { editor.get_anchor_v() };
        assert_eq!(now, kind);
        assert_eq!(rect(&editor), before, "axis {axis} kind {kind}");
    }
    assert!(editor.get_dirty());
    // the chooser is hidden for the component root
    editor.invoke_tree_selected(0);
    assert_eq!((editor.get_anchor_h(), editor.get_anchor_v()), (-1, -1));
}

#[test]
fn a_stretched_axis_shows_text_instead_of_the_size_field() {
    let dir = tempfile::tempdir().unwrap();
    let editor = start(&dir, &kids(ONE));
    editor.invoke_tree_selected(2);
    assert_eq!(row(&editor, "width").note, "");
    editor.invoke_anchor_chosen(0, 2);
    assert_eq!(row(&editor, "width").note, "左右に付いています");
    assert_eq!(row(&editor, "height").note, "");
    editor.invoke_anchor_chosen(1, 2);
    assert_eq!(row(&editor, "height").note, "上下に付いています");
    assert_eq!(row(&editor, "x").note, "");
    // leaving Stretch brings the field back
    editor.invoke_anchor_chosen(0, 0);
    assert_eq!(row(&editor, "width").note, "");
    assert_eq!(row(&editor, "width").value, "60px");
}

#[test]
fn an_expression_axis_reads_as_shiki_and_a_choice_replaces_it() {
    let dir = tempfile::tempdir().unwrap();
    let editor = start(&dir, &kids(&[("box.width / 4", "16px", "60px", "30px")]));
    editor.invoke_tree_selected(2);
    assert_eq!((editor.get_anchor_h(), editor.get_anchor_v()), (4, 0));
    let before = rect(&editor);
    editor.invoke_anchor_chosen(0, 0);
    assert_eq!(editor.get_anchor_h(), 0);
    assert_eq!(rect(&editor), before);
}

#[test]
fn dragging_a_half_expression_element_says_what_stayed() {
    let dir = tempfile::tempdir().unwrap();
    let editor = start(&dir, &kids(&[("box.width / 4", "16px", "60px", "30px")]));
    editor.invoke_tree_selected(2);
    assert_eq!(editor.get_toast_text(), "");
    editor.invoke_drag_finished(8., 8.);
    let toast = editor.get_toast_text().to_string();
    assert_eq!(toast, "横 は式で書かれているため動かしません(付き方を選ぶと動かせます)");
    // a plain axis alone says nothing new
    editor.invoke_drag_finished(0., 8.);
    assert!(editor.get_dirty());
}

#[test]
fn pins_follow_the_selection_anchors() {
    let dir = tempfile::tempdir().unwrap();
    let editor = start(&dir, &kids(ONE));
    editor.invoke_tree_selected(2);
    editor.invoke_anchor_chosen(0, 1);
    editor.invoke_anchor_chosen(1, 2);
    assert_eq!((editor.get_anchor_h(), editor.get_anchor_v()), (1, 2));
    editor.invoke_tapped(5., 5.);
    // tapping elsewhere selects another element: the pins follow the new selection
    assert_ne!(editor.get_anchor_h(), 2);
}

// ---- 付き方を推定 ----

const SPREAD: &[(&str, &str, &str, &str)] = &[
    ("10px", "10px", "280px", "30px"),
    ("250px", "60px", "40px", "30px"),
    ("130px", "90px", "40px", "30px"),
    ("14px", "120px", "40px", "30px"),
];

fn session_of(src: String) -> Session {
    thread_local!(static INIT: () = i_slint_backend_testing::init_no_event_loop());
    INIT.with(|_| {});
    let mut s = Session::new(Document::new(PathBuf::from("/t/main.slint"), src));
    assert!(s.rebuild_standalone(300., 300.), "{:?}", s.diagnostics());
    s
}

fn child(i: usize) -> ElementPath {
    ElementPath { component: "Main".into(), indices: vec![0, i] }
}

fn container() -> ElementPath {
    ElementPath { component: "Main".into(), indices: vec![0] }
}

#[test]
fn inference_lists_each_child_and_apply_is_one_undo_step() {
    let src = kids(SPREAD);
    let mut s = session_of(src.clone());
    let kids_: Vec<_> = (0..4).map(child).collect();
    let list = s.infer_anchors(&container(), &kids_, 16.0).unwrap();
    // the left-top child has nothing to change
    assert_eq!(list.len(), 3, "{list:?}");
    assert_eq!((list[0].h, list[0].v), (Some(Kind::Stretch), None));
    assert_eq!((list[1].h, list[1].v), (Some(Kind::End), None));
    assert_eq!((list[2].h, list[2].v), (Some(Kind::Center), None));
    let rects: Vec<_> = kids_.iter().map(|p| {
        s.select_path(p.clone());
        let r = s.selection_hit().unwrap().local;
        (r.x, r.y, r.w, r.h)
    }).collect();
    assert_eq!(s.apply_anchor_proposals(&list).unwrap(), 3);
    assert!(s.rebuild_standalone(300., 300.));
    // every child is where it was
    for (p, want) in kids_.iter().zip(&rects) {
        s.select_path(p.clone());
        let r = s.selection_hit().unwrap().local;
        assert_eq!((r.x, r.y, r.w, r.h), *want);
    }
    s.select_path(child(0));
    let (h, _) = s.anchors().unwrap();
    assert_eq!(h, app::anchor::Anchor::Stretch(10.0, 10.0));
    s.select_path(child(1));
    assert_eq!(s.anchors().unwrap().0, app::anchor::Anchor::End(10.0));
    // one undo returns the text from before
    assert!(s.undo());
    assert_eq!(s.doc().text(), src);
}

#[test]
fn inference_skips_expression_axes_and_layout_children() {
    let src = "export component Main inherits Rectangle {\n    width: 300px;\n    height: 300px;\n    box := Rectangle {\n        x: 0px;\n        y: 0px;\n        width: 300px;\n        height: 300px;\n        Rectangle { x: box.width - 50px; y: 250px; width: 40px; height: 30px; }\n        VerticalLayout {\n            x: 10px;\n            y: 10px;\n            width: 100px;\n            Rectangle { height: 20px; }\n        }\n    }\n}\n"
        .to_string();
    let s = session_of(src);
    let list = s.infer_anchors(&container(), &[child(0)], 16.0).unwrap();
    // x is an expression: untouched; y (250 of 300, 40 from the bottom edge...) is nearer the bottom
    assert_eq!(list.len(), 1, "{list:?}");
    assert_eq!((list[0].h, list[0].v), (None, Some(Kind::End)));
    // the children of a layout (its container is a layout) are not proposed
    let inner = ElementPath { component: "Main".into(), indices: vec![0, 1] };
    assert!(s.infer_anchors(&inner, &[ElementPath { component: "Main".into(), indices: vec![0, 1, 0] }], 16.0).unwrap().is_empty());
}

#[test]
fn the_infer_action_after_a_tidy_lists_and_applies() {
    let dir = tempfile::tempdir().unwrap();
    let editor = start(&dir, &kids(SPREAD));
    editor.invoke_tree_selected(1);
    assert!(!editor.get_can_infer());
    editor.invoke_tidy();
    assert!(editor.get_can_infer(), "{}", editor.get_toast_text());
    editor.invoke_infer_ask();
    assert!(editor.get_infer_open());
    let rows: Vec<_> = editor.get_infer_rows().iter().collect();
    assert!(rows.iter().any(|r| r.h == "右"), "{:?}", rows.iter().map(|r| (r.label.to_string(), r.h.to_string(), r.v.to_string())).collect::<Vec<_>>());
    assert!(rows.iter().any(|r| r.h == "左右"));
    assert!(rows.iter().any(|r| r.h == "中央"));
    editor.invoke_infer_run();
    assert!(!editor.get_infer_open());
    assert!(!editor.get_can_infer());
    assert!(editor.get_toast_text().contains("付き方を書き換えました"));
    editor.invoke_save();
    let text = saved(&dir);
    assert!(text.contains("parent.width - self.width"), "{text}");
    // the tidy and the inference are two history steps: two undos reach the original
    editor.invoke_undo();
    editor.invoke_undo();
    editor.invoke_save();
    assert_eq!(saved(&dir), kids(SPREAD));
}
