//! Parameters in the editor: make one inside a component, see it on the uses, set it.
//! Works on a temporary copy of `testdata/controller-ui`.

use std::path::{Path, PathBuf};

use app::session::Session;
use slint_doc::{Document, ElementPath};

fn fixture_dir() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR")).join("../../testdata/controller-ui").canonicalize().unwrap()
}

/// A copy of the fixture whose `NoticeBand` has a second text (the one made a parameter).
fn copy() -> tempfile::TempDir {
    let dir = tempfile::tempdir().unwrap();
    for e in std::fs::read_dir(fixture_dir()).unwrap() {
        let e = e.unwrap();
        if e.file_type().unwrap().is_file() {
            std::fs::copy(e.path(), dir.path().join(e.file_name())).unwrap();
        }
    }
    std::fs::create_dir(dir.path().join("fonts")).unwrap();
    for e in std::fs::read_dir(fixture_dir().join("fonts")).unwrap() {
        let e = e.unwrap();
        std::fs::copy(e.path(), dir.path().join("fonts").join(e.file_name())).unwrap();
    }
    let notice = dir.path().join("notice.slint");
    let text = std::fs::read_to_string(&notice).unwrap();
    let text = text.replace(
        "        overflow: elide;\n    }\n}",
        "        overflow: elide;\n    }\n    Text { y: 20px; text: \"Sub\"; }\n}",
    );
    assert!(text.contains("\"Sub\""));
    std::fs::write(&notice, text).unwrap();
    dir
}

fn load(dir: &Path, file: &str, component: &str) -> Session {
    thread_local!(static INIT: () = i_slint_backend_testing::init_no_event_loop());
    INIT.with(|_| {});
    let path = dir.join(file);
    let mut s = Session::new(Document::new(path.clone(), std::fs::read_to_string(path).unwrap()));
    s.set_component(component);
    s
}

fn save(dir: &Path, file: &str, s: &Session) {
    std::fs::write(dir.join(file), s.doc().text()).unwrap();
}

fn shows(s: &Session, text: &str) -> bool {
    s.visible_texts().iter().any(|t| t == text)
}

fn sel(s: &mut Session, component: &str, i: &[usize]) {
    s.select_path(ElementPath { component: component.into(), indices: i.to_vec() });
}

/// Makes the "Sub" text of NoticeBand the parameter `sub` and saves notice.slint.
fn make_parameter(dir: &Path) {
    let mut inner = load(dir, "notice.slint", "NoticeBand");
    assert!(inner.rebuild_standalone(300., 100.), "{:?}", inner.diagnostics());
    sel(&mut inner, "NoticeBand", &[1]);
    let row = inner.properties().into_iter().find(|r| r.name == "text").unwrap();
    assert!(row.can_param, "{row:?}");
    inner.parameterize("text", "sub", None).unwrap();
    save(dir, "notice.slint", &inner);
}

fn notice_use(s: &mut Session) {
    let ok = s.rebuild_standalone(480., 800.);
    assert!(ok, "{:?}", s.diagnostics());
    let (path, _) = (0..40usize)
        .filter_map(|i| {
            let p = ElementPath { component: "MainWindow".into(), indices: vec![i] };
            s.doc().element(&p).filter(|e| e.type_name == "NoticeBand").map(|e| (p, e))
        })
        .next()
        .expect("NoticeBand use in MainWindow");
    s.select_path(path);
}

#[test]
fn a_parameter_made_inside_shows_on_the_use_and_a_value_changes_the_preview() {
    let dir = copy();
    make_parameter(dir.path());
    let mut main = load(dir.path(), "main.slint", "MainWindow");
    main.set_preview_value("notice", "N").unwrap();
    notice_use(&mut main);
    assert!(main.rebuild_standalone(480., 800.));
    assert!(shows(&main, "Sub"), "{:?}", main.visible_texts());

    let rows = main.properties();
    let names: Vec<&str> = rows.iter().map(|r| r.name.as_str()).collect();
    // geometry first, then the component's in properties in declaration order
    assert_eq!(&names[..4], ["x", "y", "width", "height"]);
    assert_eq!(&names[4..6], ["text", "sub"], "{names:?}");
    let sub = rows.iter().find(|r| r.name == "sub").unwrap();
    assert_eq!((sub.ty.as_str(), sub.expr.as_str(), sub.shown.as_str()), ("string", "", ""));
    assert_eq!(sub.placeholder, "Sub");

    // A plain string, written as a literal.
    main.set_property("sub", "Hello \"x\"").unwrap();
    assert!(main.doc().text().contains(r#"sub: "Hello \"x\"";"#));
    assert!(main.rebuild_standalone(480., 800.), "{:?}", main.diagnostics());
    assert!(shows(&main, "Hello \"x\""), "{:?}", main.visible_texts());
    assert!(!shows(&main, "Sub"));
    let sub = main.properties().into_iter().find(|r| r.name == "sub").unwrap();
    assert_eq!(sub.shown, "Hello \"x\"");

    // `=` writes an expression.
    main.set_property("sub", "=root.notice").unwrap();
    assert!(main.doc().text().contains("sub: root.notice;"));

    // Empty removes the binding; the default shows again.
    main.set_property("sub", "").unwrap();
    assert!(!main.doc().text().contains("sub:"));
    assert!(main.rebuild_standalone(480., 800.));
    assert!(shows(&main, "Sub"), "{:?}", main.visible_texts());
}

#[test]
fn out_properties_and_callbacks_are_not_listed_and_a_broken_import_shows_nothing() {
    let dir = copy();
    // LeaveDialog has callbacks only.
    let mut main = load(dir.path(), "main.slint", "MainWindow");
    assert!(main.rebuild_standalone(480., 800.));
    let leave = (0..40usize)
        .map(|i| ElementPath { component: "MainWindow".into(), indices: vec![i] })
        .find(|p| main.doc().element(p).is_some_and(|e| e.type_name == "LeaveDialog"))
        .unwrap();
    main.select_path(leave);
    let names: Vec<String> = main.properties().into_iter().map(|r| r.name).collect();
    assert!(!names.iter().any(|n| n == "cancel" || n == "accept"), "{names:?}");

    // The definition cannot be read: no parameter rows (the geometry rows stay).
    std::fs::remove_file(dir.path().join("notice.slint")).unwrap();
    notice_use_no_build(&mut main);
    let rows = main.properties();
    let names: Vec<&str> = rows.iter().map(|r| r.name.as_str()).collect();
    assert_eq!(names[..4], ["x", "y", "width", "height"]);
    // Only the written bindings remain (no typed parameter rows).
    assert!(rows.iter().all(|r| r.ty.is_empty()), "{rows:?}");
}

fn notice_use_no_build(s: &mut Session) {
    let p = (0..40usize)
        .map(|i| ElementPath { component: "MainWindow".into(), indices: vec![i] })
        .find(|p| s.doc().element(p).is_some_and(|e| e.type_name == "NoticeBand"))
        .unwrap();
    s.select_path(p);
}

#[test]
fn a_component_of_the_same_file_works_and_a_parameter_of_an_inner_use_can_be_passed_on() {
    let dir = copy();
    let src = "component Card inherits Rectangle {\n    in property <int> count: 3;\n    in-out property <string> title: \"T\";\n    out property <int> twice: root.count * 2;\n    callback pressed();\n    Text { text: root.title; }\n}\nexport component Main inherits Rectangle {\n    width: 300px;\n    height: 300px;\n    Card { title: \"Given\"; }\n}\n";
    std::fs::write(dir.path().join("t.slint"), src).unwrap();
    let mut s = load(dir.path(), "t.slint", "Main");
    assert!(s.rebuild_standalone(300., 300.), "{:?}", s.diagnostics());
    sel(&mut s, "Main", &[0]);
    let rows = s.properties();
    let names: Vec<&str> = rows.iter().map(|r| r.name.as_str()).collect();
    assert_eq!(names, ["x", "y", "width", "height", "count", "title"]);
    let title = &rows[5];
    assert_eq!((title.shown.as_str(), title.ty.as_str()), ("Given", "string"));
    assert!(title.can_param);
    assert!(!rows[4].can_param, "no value written");
    assert_eq!(rows[4].placeholder, "3");
    // Passing the parameter on: the declared type is used (not the name table).
    s.set_property("count", "7").unwrap();
    assert_eq!(s.can_parameterize("count"), Ok(("card-count".to_string(), Some("int"))));
    s.parameterize("count", "card-count", None).unwrap();
    assert!(s.doc().text().contains("in property <int> card-count: 7;"));
    assert!(s.doc().text().contains("count: root.card-count;"));
}

#[test]
fn open_inside_make_parameter_go_back_and_the_use_shows_it() {
    use app::glue::load_session;
    use app::nav::{Nav, go_back, open_inside};
    use storage::{LocalStore, ProjectStore};

    thread_local!(static INIT: () = i_slint_backend_testing::init_no_event_loop());
    INIT.with(|_| {});
    let copy = copy();
    let root = tempfile::tempdir().unwrap();
    let store = LocalStore::new(root.path().to_path_buf()).unwrap();
    store.create_project("p").unwrap();
    let proj = root.path().join("p");
    std::fs::create_dir_all(proj.join("fonts")).unwrap();
    for e in walk(copy.path()) {
        let rel = e.strip_prefix(copy.path()).unwrap();
        std::fs::copy(&e, proj.join(rel)).unwrap();
    }
    let load = |f: &str| {
        let mut s = load_session(&store, "p", f)?;
        s.rebuild_standalone(480., 800.);
        Ok::<_, String>(s)
    };
    let mut file = "main.slint".to_string();
    let mut session = load(&file).unwrap();
    session.set_component("MainWindow");
    session.set_preview_value("notice", "N").unwrap();
    notice_use(&mut session);
    let mut nav = Nav::default();
    open_inside(&store, "p", &mut file, &mut session, true, &mut nav, &|f| load(f)).unwrap();
    assert_eq!(file, "notice.slint");
    assert!(session.rebuild_standalone(300., 100.));
    sel(&mut session, "NoticeBand", &[1]);
    session.parameterize("text", "sub", None).unwrap();
    go_back(&store, "p", &mut file, &mut session, true, &mut nav, &|f| load(f)).unwrap();
    assert_eq!(file, "main.slint");
    // (The glue stores preview values; this test has no store for them.)
    session.set_preview_value("notice", "N").unwrap();
    assert!(session.rebuild_standalone(480., 800.));
    assert!(session.properties().iter().any(|r| r.name == "sub" && r.ty == "string"));
    session.set_property("sub", "From use").unwrap();
    assert!(session.rebuild_standalone(480., 800.));
    assert!(shows(&session, "From use"), "{:?}", session.visible_texts());
}

fn walk(dir: &Path) -> Vec<PathBuf> {
    let mut out = Vec::new();
    for e in std::fs::read_dir(dir).unwrap() {
        let p = e.unwrap().path();
        if p.is_dir() {
            out.extend(walk(&p));
        } else {
            out.push(p);
        }
    }
    out
}

#[test]
fn a_use_lists_geometry_then_parameters_then_stray_bindings_without_duplicates() {
    let dir = copy();
    let src = "component Card inherits Rectangle {\n    in property <string> title: \"T\";\n    in property <int> count: 3;\n    Text { text: root.title; }\n}\nexport component Main inherits Rectangle {\n    width: 300px;\n    height: 300px;\n    Card { x: 5px; title: \"Given\"; opacity: 0.5; }\n}\n";
    std::fs::write(dir.path().join("t.slint"), src).unwrap();
    let mut s = load(dir.path(), "t.slint", "Main");
    assert!(s.rebuild_standalone(300., 300.), "{:?}", s.diagnostics());
    sel(&mut s, "Main", &[0]);
    let rows = s.properties();
    let got: Vec<(&str, &str, &str)> =
        rows.iter().map(|r| (r.name.as_str(), r.ty.as_str(), r.expr.as_str())).collect();
    assert_eq!(
        got,
        [
            ("x", "", "5px"),
            ("y", "", ""),
            ("width", "", ""),
            ("height", "", ""),
            ("title", "string", "\"Given\""),
            ("count", "int", ""),
            ("opacity", "", "0.5"),
        ]
    );
}
