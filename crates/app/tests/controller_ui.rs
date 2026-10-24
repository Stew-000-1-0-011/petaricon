//! Fixture tests: the oumuamua controller's UI (`testdata/controller-ui/`) must compile in the
//! preview, survive load/save byte-for-byte, and move without collateral edits.

use std::path::{Path, PathBuf};

use app::glue::save_if_dirty;
use app::session::{Session, SessionError};
use slint_doc::{Document, ElementKind, Op, TreeNode};
use storage::{LocalStore, ProjectStore};

fn fixture_dir() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("../../testdata/controller-ui")
        .canonicalize()
        .unwrap()
}

fn slint_files() -> Vec<String> {
    let mut v: Vec<String> = std::fs::read_dir(fixture_dir())
        .unwrap()
        .filter_map(|e| e.ok()?.file_name().into_string().ok())
        .filter(|n| n.ends_with(".slint"))
        .collect();
    v.sort();
    assert!(v.len() >= 12, "fixture files missing: {v:?}");
    v
}

fn load(file: &str) -> Document {
    let path = fixture_dir().join(file);
    Document::new(path.clone(), std::fs::read_to_string(path).unwrap())
}

/// Components that cannot be previewed yet, with the task that fixes them.
const PREVIEW_GAPS: &[(&str, &str)] = &[];

fn is_global(doc: &Document, name: &str) -> bool {
    doc.text().contains(&format!("global {name} "))
}

fn gap(file: &str, comp: &str) -> bool {
    PREVIEW_GAPS.iter().any(|(f, c)| *f == file && *c == comp)
}

#[test]
fn every_component_compiles_in_preview() {
    i_slint_backend_testing::init_no_event_loop();
    let mut failures = Vec::new();
    let mut count = 0;
    for file in slint_files() {
        let doc = load(&file);
        for c in doc.components() {
            if gap(&file, &c.name) || is_global(&doc, &c.name) {
                continue;
            }
            count += 1;
            if let Err(errs) = preview::compile(doc.text(), doc.path(), &c.name) {
                let msgs: Vec<_> = errs.iter().map(|e| e.message.clone()).collect();
                failures.push(format!("{file}::{} -> {msgs:?}", c.name));
            }
        }
    }
    assert!(count >= 20, "only {count} components found");
    assert!(failures.is_empty(), "{}", failures.join("\n"));
}

#[test]
fn previewing_known_gaps() {
    // Placeholder so the gap list is exercised: every listed gap must still fail
    // (otherwise remove it from PREVIEW_GAPS).
    i_slint_backend_testing::init_no_event_loop();
    for (file, comp) in PREVIEW_GAPS {
        let doc = load(file);
        assert!(
            preview::compile(doc.text(), doc.path(), comp).is_err(),
            "{file}::{comp} now compiles; drop it from PREVIEW_GAPS"
        );
    }
}

#[test]
fn load_and_save_without_edits_is_byte_identical() {
    let dir = tempfile::tempdir().unwrap();
    let store = LocalStore::new(dir.path().to_path_buf()).unwrap();
    store.create_project("p").unwrap();
    for file in slint_files() {
        let original = std::fs::read(fixture_dir().join(&file)).unwrap();
        // Into the project exactly as bytes, then through the app's load path.
        std::fs::write(dir.path().join("p").join(&file), &original).unwrap();
        let text = store.read("p", &file).unwrap();
        let mut session = Session::new(Document::new(PathBuf::from(format!("p/{file}")), text));
        for c in session.doc().components() {
            // Selecting each component and parsing it must not change the text.
            session.set_component(&c.name);
            let _ = session.tree_rows();
        }
        // The save path with no edits writes nothing.
        assert_eq!(
            save_if_dirty(&store, "p", &file, &mut session, true),
            Ok(false),
            "{file}"
        );
        assert_eq!(
            std::fs::read(dir.path().join("p").join(&file)).unwrap(),
            original,
            "{file} (no-op)"
        );
        // Force a real write of the unchanged text (what a save after a no-net-change edit does).
        store.write("p", &file, session.doc().text()).unwrap();
        assert_eq!(
            std::fs::read(dir.path().join("p").join(&file)).unwrap(),
            original,
            "{file} (rewritten)"
        );
    }
}

fn find<'a>(
    n: &'a TreeNode,
    parent_kind: Option<ElementKind>,
    pred: &dyn Fn(&TreeNode, Option<ElementKind>) -> bool,
) -> Option<&'a TreeNode> {
    if pred(n, parent_kind) {
        return Some(n);
    }
    n.children
        .iter()
        .find_map(|c| find(c, Some(n.info.kind), pred))
}

fn has(n: &TreeNode, name: &str) -> bool {
    n.info.bindings.iter().any(|b| b.name == name)
}

fn session_for(file: &str, comp: &str) -> Session {
    let _ = std::panic::catch_unwind(i_slint_backend_testing::init_no_event_loop);
    let mut s = Session::new(load(file));
    s.set_component(comp);
    assert!(s.rebuild_standalone(480., 360.), "{:?}", s.diagnostics());
    s
}

fn changed_lines(before: &str, after: &str) -> Vec<(String, String)> {
    let b: Vec<&str> = before.split('\n').collect();
    let a: Vec<&str> = after.split('\n').collect();
    assert_eq!(a.len(), b.len(), "line count changed");
    b.iter()
        .zip(a.iter())
        .filter(|(x, y)| x != y)
        .map(|(x, y)| (x.to_string(), y.to_string()))
        .collect()
}

#[test]
fn moving_a_free_placed_element_changes_only_its_x_and_y() {
    let mut s = session_for("auto.slint", "AutoPanel");
    let tree = s.doc().tree("AutoPanel").unwrap();
    let target = find(&tree, None, &|n, pk| {
        n.info.type_name == "Panel"
            && has(n, "x")
            && has(n, "y")
            && !matches!(pk, Some(ElementKind::Layout(_)))
    })
    .expect("a free-placed Panel with x and y");
    s.select_path(target.info.path.clone());
    // The fixture positions its panels by expressions, which a drag never overwrites (see
    // the next test); this one is about a plain `px` position, so give the panel one.
    let before = s.doc().text().to_string();
    let r = s.move_selection(16., 8.);
    assert!(matches!(r, Err(SessionError::Anchor(_))), "{r:?}");
    assert_eq!(s.doc().text(), before);
    s.apply(Op::SetBindings {
        path: target.info.path.clone(),
        bindings: vec![("x".into(), Some("40px".into())), ("y".into(), Some("40px".into()))],
    })
    .unwrap();
    assert!(s.rebuild_standalone(480., 360.), "{:?}", s.diagnostics());
    let before = s.doc().text().to_string();
    s.move_selection(16., 8.).expect("move");
    let after = s.doc().text().to_string();
    assert_ne!(before, after);
    let diff = changed_lines(&before, &after);
    assert_eq!(diff.len(), 2, "{diff:?}");
    assert!(
        diff[0].0.trim_start().starts_with("x:") && diff[0].1.trim_start().starts_with("x:"),
        "{diff:?}"
    );
    assert!(
        diff[1].0.trim_start().starts_with("y:") && diff[1].1.trim_start().starts_with("y:"),
        "{diff:?}"
    );
}

#[test]
fn moving_an_element_inside_a_layout_is_refused_and_leaves_text_alone() {
    let mut s = session_for("connect.slint", "ConnectScreen");
    let tree = s.doc().tree("ConnectScreen").unwrap();
    let target = find(&tree, None, &|n, pk| {
        matches!(pk, Some(ElementKind::Layout(_))) && n.info.type_name == "Text"
    })
    .expect("a Text inside a layout");
    s.select_path(target.info.path.clone());
    let before = s.doc().text().to_string();
    let r = s.move_selection(5., 5.);
    assert!(matches!(r, Err(SessionError::InLayout)), "{r:?}");
    assert_eq!(s.doc().text(), before);
}

#[test]
fn globals_are_not_offered_as_previewable_components() {
    i_slint_backend_testing::init_no_event_loop();
    for file in slint_files() {
        let doc = load(&file);
        for c in doc.components() {
            assert!(
                !is_global(&doc, &c.name),
                "{file}::{} is a global but listed as a component",
                c.name
            );
        }
    }
}

#[test]
fn main_window_instantiates_and_has_geometry() {
    let mut s = session_for("main.slint", "MainWindow");
    let tree = s.doc().tree("MainWindow").unwrap();
    fn walk(s: &mut Session, n: &TreeNode, out: &mut Vec<(String, bool)>) {
        s.select_path(n.info.path.clone());
        out.push((
            format!("{} {:?}", n.info.type_name, n.info.path.indices),
            s.selection_hit().is_some(),
        ));
        for c in &n.children {
            walk(s, c, out);
        }
    }
    let mut rows = Vec::new();
    walk(&mut s, &tree, &mut rows);
    for r in &rows {
        eprintln!("{r:?}");
    }
    let with = rows.iter().filter(|r| r.1).count();
    assert!(with > 1, "only the root has geometry: {rows:?}");
}
