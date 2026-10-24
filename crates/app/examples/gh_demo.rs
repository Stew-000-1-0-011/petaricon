//! Desktop demo of the GitHub dialog against an in-memory fake GitHub (no network, no token).
//!
//! `cargo run -p app --example gh_demo -- <scene>` where scene is one of `new`, `dialog`,
//! `conflicts`, `pushed`. Used to take the screenshots of the README.

use std::cell::RefCell;
use std::rc::Rc;
use std::sync::Arc;
use std::time::Duration;

use app::github::{ApiFactory, BoxApi};
use ghsync::fake::Fake;
use ghsync::{ApiError, GitHubApi, NewEntry, TreeEntry, UpdateError};
use slint::ComponentHandle;
use storage::{LocalStore, ProjectStore};

const DEMO_TOKEN: &str = "github_pat_11DEMODEMODEMODEMO_0123456789abcd";
const MAIN: &str = "export component Main inherits Window {\n    preferred-width: 360px;\n    preferred-height: 640px;\n    Rectangle { x: 20px; y: 20px; width: 120px; height: 60px; background: #4fc3f7; }\n}\n";

struct Api(Arc<Fake>);

macro_rules! delegate {
    ($($name:ident($($a:ident: $t:ty),*) -> $r:ty;)*) => {
        $(fn $name(&self, $($a: $t),*) -> $r { self.0.$name($($a),*) })*
    };
}

impl GitHubApi for Api {
    delegate! {
        branch_head(r: &str, b: &str) -> Result<String, ApiError>;
        commit_tree(r: &str, c: &str) -> Result<String, ApiError>;
        tree_shallow(r: &str, t: &str) -> Result<Vec<TreeEntry>, ApiError>;
        tree_recursive(r: &str, t: &str) -> Result<Vec<TreeEntry>, ApiError>;
        blob(r: &str, s: &str) -> Result<Vec<u8>, ApiError>;
        create_blob(r: &str, b: &[u8]) -> Result<String, ApiError>;
        create_tree(r: &str, base: &str, e: Vec<NewEntry>) -> Result<String, ApiError>;
        create_commit(r: &str, m: &str, t: &str, p: &str) -> Result<String, ApiError>;
        update_ref(r: &str, b: &str, c: &str) -> Result<(), UpdateError>;
    }
}

type Step = Box<dyn Fn(&app::Editor)>;

fn main() -> Result<(), slint::PlatformError> {
    let scene = std::env::args().nth(1).unwrap_or_else(|| "dialog".into());
    let root = std::env::temp_dir().join(format!("petaricon-gh-demo-{scene}"));
    let _ = std::fs::remove_dir_all(&root);
    let store = LocalStore::new(root.clone()).map_err(|e| slint::PlatformError::Other(e.to_string()))?;
    store.create_project("default").unwrap();
    store.set_github_token(DEMO_TOKEN).unwrap();
    app::font::install(&std::env::temp_dir().join("petaricon-fonts"));

    let fake = Arc::new(Fake::new());
    fake.commit(
        "ccr-1a4a2126-jaf9ht",
        &[
            ("crates/app/ui/main.slint", Some(MAIN.as_bytes())),
            ("crates/app/ui/panel.slint", Some(b"export component Panel inherits Rectangle {}\n")),
            ("crates/app/ui/theme.slint", Some(b"export global Theme { out property <color> accent: #4fc3f7; }\n")),
        ],
    );
    let f2 = fake.clone();
    let factory: ApiFactory = Arc::new(move |_| -> BoxApi { Box::new(Api(f2.clone())) });
    let editor = app::Editor::new()?;
    app::glue::start_with(&editor, Box::new(store), factory)?;
    editor.window().set_size(slint::LogicalSize::new(480., 760.));

    let (repo, branch, folder) = ("Stew-000-1-0-011/oumuamua-controller", "ccr-1a4a2126-jaf9ht", "crates/app/ui");
    let import: Step = Box::new(move |e| {
        e.invoke_gh_open_new();
        e.invoke_gh_import(repo.into(), branch.into(), folder.into(), "".into());
    });
    let proj = root.join("ui");
    let fake2 = fake.clone();
    let steps: Vec<Step> = match scene.as_str() {
        "new" => vec![Box::new(move |e| {
            e.invoke_gh_open_new();
            e.set_gh_repo(repo.into());
            e.set_gh_branch(branch.into());
            e.set_gh_folder(folder.into());
        })],
        "dialog" => vec![import, Box::new(|e| e.invoke_gh_open())],
        "conflicts" => vec![
            import,
            Box::new(|e| e.invoke_gh_open()),
            Box::new(move |_| {
                std::fs::write(proj.join("panel.slint"), "export component Panel inherits Rectangle { background: red; }\n").unwrap();
                std::fs::write(proj.join("theme.slint"), "// my theme\nexport global Theme { out property <color> accent: green; }\n").unwrap();
                fake2.commit("ccr-1a4a2126-jaf9ht", &[
                    ("crates/app/ui/panel.slint", Some(b"export component Panel inherits Rectangle { background: blue; }\n")),
                    ("crates/app/ui/theme.slint", Some(b"export global Theme { out property <color> accent: orange; }\n")),
                    ("crates/app/ui/main.slint", Some(MAIN.replace("120px", "140px").as_bytes())),
                ]);
            }),
            Box::new(|e| e.invoke_gh_pull()),
        ],
        "pushed" => vec![
            import,
            Box::new(|e| e.invoke_gh_open()),
            Box::new(move |_| {
                std::fs::write(proj.join("panel.slint"), "export component Panel inherits Rectangle { background: red; }\n").unwrap();
            }),
            Box::new(|e| e.invoke_gh_open()),
            Box::new(|e| { e.set_gh_message("パネルの色を変更".into()); e.invoke_gh_push("パネルの色を変更".into()); }),
        ],
        other => panic!("unknown scene {other}"),
    };
    let steps = Rc::new(steps);
    let next = Rc::new(RefCell::new(0usize));
    let timer = slint::Timer::default();
    let weak = editor.as_weak();
    timer.start(slint::TimerMode::Repeated, Duration::from_millis(400), move || {
        let Some(e) = weak.upgrade() else { return };
        let mut n = next.borrow_mut();
        if !e.get_gh_busy() && *n < steps.len() {
            steps[*n](&e);
            *n += 1;
        }
    });
    editor.run()
}
