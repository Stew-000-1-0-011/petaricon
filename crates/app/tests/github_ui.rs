//! The GitHub dialog, driven through the window with a fake GitHub (never the real one).

use std::sync::Arc;
use std::time::Duration;

use app::github::{ApiFactory, BoxApi};
use ghsync::fake::Fake;
use ghsync::{ApiError, GitHubApi, NewEntry, TreeEntry, UpdateError};
use slint::{ComponentHandle, Model};
use storage::{LocalStore, ProjectStore};

const SRC: &str = "export component Main inherits Rectangle {\n    width: 300px;\n    height: 300px;\n    a := Rectangle {\n        x: 10px;\n        y: 10px;\n        width: 100px;\n        height: 100px;\n    }\n}\n";
const SRC2: &str = "export component Main inherits Rectangle {\n    width: 300px;\n    height: 300px;\n}\nexport component Extra inherits Rectangle {}\n";
const TOKEN: &str = "github_pat_11SECRETMIDDLEPART0123456789abcd";
const REPO: &str = "o/r";

/// A fake whose failures can be switched on, shared with the worker thread.
struct Shared {
    fake: Arc<Fake>,
    fail: Arc<std::sync::Mutex<Option<ApiError>>>,
    tokens: Arc<std::sync::Mutex<Vec<String>>>,
    poison: Poison,
    gate: Gate,
}

type Poison = Arc<std::sync::Mutex<Option<std::path::PathBuf>>>;

/// The next job blocks in its first call: it signals `reached`, then waits for `release`.
type Gate = Arc<std::sync::Mutex<Option<(std::sync::mpsc::Sender<()>, std::sync::mpsc::Receiver<()>)>>>;

struct Api(Arc<Fake>, Arc<std::sync::Mutex<Option<ApiError>>>, Poison, Gate);

impl Api {
    fn check(&self) -> Result<(), ApiError> {
        match self.1.lock().unwrap().clone() {
            Some(e) => Err(e),
            None => Ok(()),
        }
    }
}

impl GitHubApi for Api {
    fn branch_head(&self, r: &str, b: &str) -> Result<String, ApiError> {
        let gate = self.3.lock().unwrap().take();
        if let Some((reached, release)) = gate {
            reached.send(()).unwrap();
            release.recv().unwrap();
        }
        self.check()?;
        self.0.branch_head(r, b)
    }
    fn commit_tree(&self, r: &str, c: &str) -> Result<String, ApiError> {
        self.0.commit_tree(r, c)
    }
    fn tree_shallow(&self, r: &str, t: &str) -> Result<Vec<TreeEntry>, ApiError> {
        self.0.tree_shallow(r, t)
    }
    fn tree_recursive(&self, r: &str, t: &str) -> Result<Vec<TreeEntry>, ApiError> {
        self.0.tree_recursive(r, t)
    }
    fn blob(&self, r: &str, s: &str) -> Result<Vec<u8>, ApiError> {
        if let Some(dir) = self.2.lock().unwrap().clone() {
            // Leave a half-made project behind, then die.
            std::fs::create_dir_all(&dir).unwrap();
            std::fs::write(dir.join("partial.txt"), "x").unwrap();
            panic!("worker died");
        }
        self.0.blob(r, s)
    }
    fn create_blob(&self, r: &str, b: &[u8]) -> Result<String, ApiError> {
        self.0.create_blob(r, b)
    }
    fn create_tree(&self, r: &str, base: &str, e: Vec<NewEntry>) -> Result<String, ApiError> {
        self.0.create_tree(r, base, e)
    }
    fn create_commit(&self, r: &str, m: &str, t: &str, p: &str) -> Result<String, ApiError> {
        self.0.create_commit(r, m, t, p)
    }
    fn update_ref(&self, r: &str, b: &str, c: &str) -> Result<(), UpdateError> {
        self.0.update_ref(r, b, c)
    }
}

struct Env {
    dir: tempfile::TempDir,
    shared: Shared,
    editor: app::Editor,
}

impl Env {
    fn root(&self) -> std::path::PathBuf {
        self.dir.path().join("projects")
    }
    fn store(&self) -> LocalStore {
        LocalStore::new(self.root()).unwrap()
    }
    fn fake(&self) -> &Fake {
        &self.shared.fake
    }
}

fn remote() -> Fake {
    let f = Fake::new();
    f.commit(
        "dev",
        &[
            ("README.md", Some(b"outside\n")),
            ("crates/ui/main.slint", Some(SRC.as_bytes())),
            ("crates/ui/other.slint", Some(b"export component Other inherits Rectangle {}\n")),
            ("crates/ui/img/pic.bin", Some(&[0, 159, 146, 150, 255])),
        ],
    );
    f
}

/// A window on a store with a `default` project; `token` is stored when given.
fn setup(token: Option<&str>) -> Env {
    i_slint_backend_testing::init_no_event_loop();
    let dir = tempfile::tempdir().unwrap();
    let store = LocalStore::new(dir.path().join("projects")).unwrap();
    store.create_project("default").unwrap();
    if let Some(t) = token {
        store.set_github_token(t).unwrap();
    }
    let shared = Shared {
        fake: Arc::new(remote()),
        fail: Arc::new(std::sync::Mutex::new(None)),
        tokens: Arc::new(std::sync::Mutex::new(Vec::new())),
        poison: Arc::new(std::sync::Mutex::new(None)),
        gate: Arc::new(std::sync::Mutex::new(None)),
    };
    let (fake, fail, tokens, poison, gate) = (
        shared.fake.clone(),
        shared.fail.clone(),
        shared.tokens.clone(),
        shared.poison.clone(),
        shared.gate.clone(),
    );
    let factory: ApiFactory = Arc::new(move |token: &str| -> BoxApi {
        tokens.lock().unwrap().push(token.to_string());
        Box::new(Api(fake.clone(), fail.clone(), poison.clone(), gate.clone()))
    });
    let editor = app::Editor::new().unwrap();
    app::glue::start_with(&editor, Box::new(store), factory).unwrap();
    editor.window().set_size(slint::LogicalSize::new(480., 800.));
    editor.show().unwrap();
    slint::platform::update_timers_and_animations();
    let _ = editor.window().size();
    Env { dir, shared, editor }
}

/// Lets the worker thread finish and the glue pick up its outcome.
fn wait(editor: &app::Editor) {
    for _ in 0..1000 {
        i_slint_backend_testing::mock_elapsed_time(Duration::from_millis(60));
        slint::platform::update_timers_and_animations();
        if !editor.get_gh_busy() {
            return;
        }
        std::thread::sleep(Duration::from_millis(5));
    }
    panic!("the transfer did not finish");
}

fn import(env: &Env, name: &str) {
    let e = &env.editor;
    e.invoke_gh_open_new();
    assert_eq!(e.get_gh_mode(), 2);
    e.invoke_gh_import(REPO.into(), "dev".into(), "crates/ui".into(), name.into());
}

/// Makes `ui` the open, GitHub-linked project.
fn linked(env: &Env) {
    import(env, "ui");
    assert!(env.editor.get_gh_busy());
    wait(&env.editor);
    assert_eq!(env.editor.get_current_project().as_str(), "ui");
}

fn strings(m: slint::ModelRc<slint::SharedString>) -> Vec<String> {
    m.iter().map(|s| s.to_string()).collect()
}

fn make_dirty(editor: &app::Editor) {
    let z = editor.get_zoom_level();
    editor.invoke_tapped(60. * z, 60. * z);
    assert!(editor.get_has_selection());
    editor.invoke_drag_finished(40. * z, 40. * z);
    assert!(editor.get_dirty());
}

/// Every text the window shows about GitHub and errors.
fn screen_texts(e: &app::Editor) -> Vec<String> {
    let mut v = vec![
        e.get_error_text().to_string(),
        e.get_message().to_string(),
        e.get_toast_text().to_string(),
        e.get_dialog_error().to_string(),
        e.get_gh_token_hint().to_string(),
        e.get_gh_link_text().to_string(),
        e.get_gh_status_text().to_string(),
        e.get_gh_result().to_string(),
        e.get_gh_error().to_string(),
        e.get_gh_repo().to_string(),
        e.get_gh_branch().to_string(),
        e.get_gh_folder().to_string(),
        e.get_gh_name().to_string(),
        e.get_gh_message().to_string(),
        e.get_gh_editing().to_string(),
        e.get_source_text().to_string(),
    ];
    v.extend(strings(e.get_gh_conflicts()));
    v
}

#[test]
fn new_project_from_github_appears_in_the_list_and_opens() {
    let env = setup(Some(TOKEN));
    let e = &env.editor;
    assert_eq!(strings(e.get_projects()), vec!["default"]);
    import(&env, "");
    assert!(e.get_gh_busy(), "the transfer runs in the background");
    assert_eq!(e.get_gh_mode(), 2, "the dialog stays while it runs");
    wait(e);
    assert_eq!(e.get_gh_error().as_str(), "");
    assert_eq!(e.get_gh_mode(), 0);
    assert_eq!(strings(e.get_projects()), vec!["default", "ui"], "named after the folder");
    assert_eq!(e.get_current_project().as_str(), "ui");
    assert_eq!(e.get_current_file().as_str(), "main.slint");
    assert_eq!(strings(e.get_files()), vec!["main.slint", "other.slint"]);
    assert!(e.get_message().contains("取り込みました"));
    let dir = env.root().join("ui");
    assert_eq!(std::fs::read(dir.join("img/pic.bin")).unwrap(), [0, 159, 146, 150, 255]);
    assert!(dir.join(".sync").is_file());
    assert!(!dir.join("README.md").exists());
    // The token the worker used came from the store.
    assert_eq!(env.shared.tokens.lock().unwrap().as_slice(), [TOKEN]);
    // The dialog now shows the link.
    e.invoke_gh_open();
    assert!(e.get_gh_linked());
    assert_eq!(e.get_gh_link_text().as_str(), "o/r · dev · crates/ui");
    assert!(e.get_gh_status_text().contains("手元の変更: なし"));
}

#[test]
fn import_without_token_asks_for_one_and_sends_nothing() {
    let env = setup(None);
    let e = &env.editor;
    import(&env, "ui");
    assert!(!e.get_gh_busy());
    assert!(e.get_gh_error().contains("トークン"));
    assert_eq!(e.get_gh_editing().as_str(), "token", "the input bar opens for the token");
    assert!(env.fake().0.lock().unwrap().calls.is_empty());
    assert!(env.shared.tokens.lock().unwrap().is_empty());
    assert_eq!(strings(e.get_projects()), vec!["default"]);
}

#[test]
fn bad_input_is_refused_before_any_transfer() {
    let env = setup(Some(TOKEN));
    let e = &env.editor;
    e.invoke_gh_open_new();
    e.invoke_gh_import("nonsense".into(), "dev".into(), "".into(), "".into());
    assert!(e.get_gh_error().contains("owner/name"));
    e.invoke_gh_import(REPO.into(), "".into(), "".into(), "".into());
    assert!(e.get_gh_error().contains("ブランチ"));
    e.invoke_gh_import(REPO.into(), "dev".into(), "x".into(), "../evil".into());
    assert!(e.get_gh_error().contains("プロジェクト名"));
    assert!(!e.get_gh_busy());
    assert!(env.fake().0.lock().unwrap().calls.is_empty());
}

#[test]
fn failed_import_leaves_no_project_and_explains() {
    let env = setup(Some(TOKEN));
    let e = &env.editor;
    e.invoke_gh_open_new();
    e.invoke_gh_import(REPO.into(), "nope".into(), "crates/ui".into(), "ui".into());
    wait(e);
    assert!(e.get_gh_error().contains("見つかりません"), "{}", e.get_gh_error());
    assert_eq!(e.get_gh_mode(), 2);
    assert_eq!(strings(e.get_projects()), vec!["default"]);
    assert!(!env.root().join("ui").exists());
    // An empty folder, and a name that is taken.
    e.invoke_gh_import(REPO.into(), "dev".into(), "crates/none".into(), "ui".into());
    wait(e);
    assert!(e.get_gh_error().contains("ファイルがありません"));
    e.invoke_gh_import(REPO.into(), "dev".into(), "crates/ui".into(), "default".into());
    wait(e);
    assert!(e.get_gh_error().contains("同じ名前"), "{}", e.get_gh_error());
}

#[test]
fn pull_updates_the_open_document_and_clears_undo_only_when_its_text_changed() {
    let env = setup(Some(TOKEN));
    let e = &env.editor;
    linked(&env);
    assert_eq!(strings(e.get_components()), vec!["Main"]);

    // 1. The remote changes another file: the open document and its undo history stay.
    make_dirty(e);
    e.invoke_save();
    assert!(!e.get_dirty());
    env.fake().commit(
        "dev",
        &[("crates/ui/other.slint", Some(b"export component Other2 inherits Rectangle {}\n"))],
    );
    e.invoke_gh_open();
    e.invoke_gh_pull();
    assert!(e.get_gh_busy());
    wait(e);
    assert!(e.get_gh_result().contains("更新 1 件"), "{}", e.get_gh_result());
    e.invoke_undo();
    assert!(e.get_dirty(), "undo still works: the open file was not touched");
    e.invoke_redo();
    e.invoke_save();

    // 2. The remote changes the open file, which has a local change (the dragged box): a
    //    conflict, resolved by taking the remote version.
    env.fake().commit("dev", &[("crates/ui/main.slint", Some(SRC2.as_bytes()))]);
    e.invoke_gh_pull();
    wait(e);
    assert_eq!(strings(e.get_gh_conflicts()), vec!["main.slint"]);
    assert_eq!(strings(e.get_components()), vec!["Main"], "a conflict leaves the document alone");
    e.invoke_gh_take_remote("main.slint".into());
    wait(e);
    assert_eq!(strings(e.get_components()), vec!["Main", "Extra"], "the open document was re-read");
    assert!(!e.get_dirty());
    e.invoke_undo();
    assert!(!e.get_dirty(), "undo history is gone with the replaced text");
    assert!(env.root().join("ui/main.slint.local").is_file(), "the local version is kept");

    // 3. A pull that brings nothing changes nothing.
    e.invoke_gh_pull();
    wait(e);
    assert!(e.get_gh_result().contains("新しい変更はありません"));
}

#[test]
fn a_clean_remote_change_to_the_open_file_is_loaded() {
    let env = setup(Some(TOKEN));
    let e = &env.editor;
    linked(&env);
    env.fake().commit("dev", &[("crates/ui/main.slint", Some(SRC2.as_bytes()))]);
    e.invoke_gh_open();
    e.invoke_gh_pull();
    wait(e);
    assert_eq!(strings(e.get_components()), vec!["Main", "Extra"]);
    assert_eq!(e.get_gh_error().as_str(), "");
    assert!(!e.get_dirty());
}

#[test]
fn push_saves_the_open_document_first() {
    let env = setup(Some(TOKEN));
    let e = &env.editor;
    linked(&env);
    make_dirty(e);
    e.invoke_gh_open();
    e.invoke_gh_push("my change".into());
    assert!(!e.get_dirty(), "saved before the transfer started");
    wait(e);
    assert_eq!(e.get_gh_error().as_str(), "");
    assert!(e.get_gh_result().contains("変更 1 件"), "{}", e.get_gh_result());
    let remote = env.fake().listing("dev");
    let on_disk = std::fs::read(env.root().join("ui/main.slint")).unwrap();
    assert_ne!(on_disk, SRC.as_bytes());
    assert_eq!(remote["crates/ui/main.slint"], on_disk);
    assert_eq!(remote["README.md"], b"outside\n");
    assert!(e.get_gh_status_text().contains("手元の変更: なし"));
}

#[test]
fn push_with_nothing_to_send_and_default_message() {
    let env = setup(Some(TOKEN));
    let e = &env.editor;
    linked(&env);
    e.invoke_gh_open();
    e.invoke_gh_push("".into());
    wait(e);
    assert!(e.get_gh_error().contains("送る変更がありません"));
    assert_eq!(e.get_gh_message().as_str(), "petaricon で編集");
    std::fs::write(env.root().join("ui/other.slint"), "export component Other inherits Rectangle {}\n// x\n").unwrap();
    e.invoke_gh_push("  ".into());
    wait(e);
    assert!(e.get_gh_result().contains("送りました"));
}

#[test]
fn push_is_refused_when_the_branch_moved() {
    let env = setup(Some(TOKEN));
    let e = &env.editor;
    linked(&env);
    std::fs::write(env.root().join("ui/other.slint"), "mine\n").unwrap();
    env.fake().commit("dev", &[("crates/ui/img/pic.bin", Some(b"new"))]);
    let commits = env.fake().commit_count();
    e.invoke_gh_open();
    e.invoke_gh_push("m".into());
    wait(e);
    assert!(e.get_gh_error().contains("先に「取り込む」"), "{}", e.get_gh_error());
    assert_eq!(env.fake().commit_count(), commits, "nothing was committed");
}

#[test]
fn conflicts_block_push_and_each_file_can_go_either_way() {
    let env = setup(Some(TOKEN));
    let e = &env.editor;
    linked(&env);
    let dir = env.root().join("ui");
    std::fs::write(dir.join("other.slint"), "mine other\n").unwrap();
    std::fs::write(dir.join("img/pic.bin"), b"mine pic").unwrap();
    env.fake().commit(
        "dev",
        &[
            ("crates/ui/other.slint", Some(b"theirs other\n")),
            ("crates/ui/img/pic.bin", Some(b"theirs pic")),
        ],
    );
    e.invoke_gh_open();
    e.invoke_gh_pull();
    wait(e);
    assert_eq!(strings(e.get_gh_conflicts()), vec!["img/pic.bin", "other.slint"]);
    assert!(e.get_gh_status_text().contains("両方で変更: 2 件"));
    assert_eq!(std::fs::read(dir.join("other.slint")).unwrap(), b"mine other\n", "never overwritten");

    let blobs_before = env.fake().0.lock().unwrap().blobs.len();
    e.invoke_gh_push("m".into());
    wait(e);
    assert!(e.get_gh_error().contains("リモートの版にする"), "{}", e.get_gh_error());
    assert!(e.get_gh_error().contains("手元の版を残す"));
    assert_eq!(env.fake().0.lock().unwrap().blobs.len(), blobs_before, "nothing was uploaded");

    e.invoke_gh_take_remote("other.slint".into());
    wait(e);
    assert_eq!(std::fs::read(dir.join("other.slint")).unwrap(), b"theirs other\n");
    assert_eq!(std::fs::read(dir.join("other.slint.local")).unwrap(), b"mine other\n");
    assert_eq!(strings(e.get_gh_conflicts()), vec!["img/pic.bin"]);
    e.invoke_gh_keep_local("img/pic.bin".into());
    wait(e);
    assert!(strings(e.get_gh_conflicts()).is_empty());
    assert_eq!(std::fs::read(dir.join("img/pic.bin")).unwrap(), b"mine pic");

    e.invoke_gh_push("resolve".into());
    wait(e);
    assert_eq!(e.get_gh_error().as_str(), "");
    let remote = env.fake().listing("dev");
    assert_eq!(remote["crates/ui/img/pic.bin"], b"mine pic", "kept version is pushed");
    assert_eq!(remote["crates/ui/other.slint"], b"theirs other\n");
    assert!(!remote.keys().any(|k| k.ends_with(".local")), "the .local copy is never pushed");
}

#[test]
fn errors_are_shown_in_japanese() {
    let env = setup(Some(TOKEN));
    let e = &env.editor;
    linked(&env);
    e.invoke_gh_open();
    let cases = [
        (ApiError::Network("down".into()), "通信できません"),
        (ApiError::Unauthorized, "認証に失敗"),
        (ApiError::NotFound, "見つかりません"),
        (ApiError::RateLimited, "利用上限"),
        (ApiError::Truncated, "大きすぎ"),
    ];
    for (err, text) in cases {
        *env.shared.fail.lock().unwrap() = Some(err);
        e.invoke_gh_pull();
        wait(e);
        assert!(e.get_gh_error().contains(text), "{}", e.get_gh_error());
        assert_eq!(e.get_gh_result().as_str(), "");
        assert!(!e.get_gh_busy());
    }
    *env.shared.fail.lock().unwrap() = None;
    e.invoke_gh_pull();
    wait(e);
    assert_eq!(e.get_gh_error().as_str(), "");
}

#[test]
fn the_token_never_reaches_the_screen_except_its_last_four_characters() {
    let env = setup(None);
    let e = &env.editor;
    e.invoke_gh_open();
    assert_eq!(e.get_gh_token_hint().as_str(), "");
    e.invoke_gh_token_set(format!("  {TOKEN}\n").into());
    assert_eq!(e.get_gh_token_hint().as_str(), "…abcd");
    assert_eq!(env.store().github_token().as_deref(), Some(TOKEN));
    // Not in the settings file, and not readable as a project.
    let settings = std::fs::read_to_string(env.root().join(".settings")).unwrap_or_default();
    assert!(!settings.contains("SECRET"));
    assert!(!env.store().projects().unwrap().iter().any(|p| p.contains("token")));

    // Even an error text that happens to carry the token is cleaned.
    *env.shared.fail.lock().unwrap() = Some(ApiError::Other(500, format!("echo {TOKEN} back")));
    e.invoke_gh_open_new();
    e.invoke_gh_import(REPO.into(), "dev".into(), "crates/ui".into(), "ui".into());
    wait(e);
    assert!(e.get_gh_error().contains("500"));
    for t in screen_texts(e) {
        assert!(!t.contains("SECRET") && !t.contains("github_pat"), "leaked: {t}");
    }

    // A token with whitespace inside is refused, and the stored one stays.
    e.invoke_gh_token_set("github_pat_a b".into());
    assert!(e.get_gh_error().contains("空白"));
    assert_eq!(env.store().github_token().as_deref(), Some(TOKEN));

    e.invoke_gh_token_clear();
    assert_eq!(e.get_gh_token_hint().as_str(), "");
    assert_eq!(env.store().github_token(), None);
    assert!(!env.root().join(".github-token").exists());
}

#[test]
fn an_unlinked_project_says_so_and_pull_fails_politely() {
    let env = setup(Some(TOKEN));
    let e = &env.editor;
    e.invoke_gh_open();
    assert!(!e.get_gh_linked());
    e.invoke_gh_pull();
    wait(e);
    assert!(e.get_gh_error().contains("つながっていません"), "{}", e.get_gh_error());
}

#[test]
fn only_one_transfer_at_a_time_and_the_dialog_state_resets() {
    let env = setup(Some(TOKEN));
    let e = &env.editor;
    linked(&env);
    e.invoke_gh_open();
    e.invoke_gh_pull();
    assert!(e.get_gh_busy());
    e.invoke_gh_push("m".into());
    assert!(e.get_gh_error().contains("通信中"));
    wait(e);
    e.invoke_gh_closed();
    assert_eq!(e.get_gh_error().as_str(), "");
    assert_eq!(e.get_gh_result().as_str(), "");
}

fn key(editor: &app::Editor, text: impl Into<slint::SharedString>) {
    use slint::platform::WindowEvent;
    let text = text.into();
    editor.window().dispatch_event(WindowEvent::KeyPressed { text: text.clone() });
    editor.window().dispatch_event(WindowEvent::KeyReleased { text });
}

fn type_in_bar(editor: &app::Editor, value: &str) {
    for c in value.chars() {
        key(editor, c);
    }
}

fn elapse(ms: u64) {
    slint::platform::update_timers_and_animations();
    i_slint_backend_testing::mock_elapsed_time(Duration::from_millis(ms));
    slint::platform::update_timers_and_animations();
}

#[test]
fn the_token_and_the_fields_are_typed_in_the_input_bar() {
    let env = setup(None);
    let e = &env.editor;
    e.invoke_gh_open_new();
    e.invoke_gh_open_bar("token".into());
    slint::platform::update_timers_and_animations();
    assert!(e.get_bar_visible());
    assert_eq!(e.get_bar_text().as_str(), "");
    type_in_bar(e, TOKEN);
    assert_eq!(e.get_gh_token_hint().as_str(), "", "not stored while typing");
    key(e, char::from(slint::platform::Key::Return));
    slint::platform::update_timers_and_animations();
    assert!(!e.get_bar_visible());
    assert_eq!(e.get_gh_token_hint().as_str(), "…abcd");
    assert_eq!(env.store().github_token().as_deref(), Some(TOKEN));
    assert_eq!(e.get_bar_text().as_str(), "", "the token does not stay in the bar");

    // A text field follows every keystroke.
    e.invoke_gh_open_bar("repo".into());
    type_in_bar(e, "o/r");
    assert_eq!(e.get_gh_repo().as_str(), "o/r");
    key(e, char::from(slint::platform::Key::Return));
    slint::platform::update_timers_and_animations();
    assert!(!e.get_bar_visible());
    // Re-opening starts from the current value.
    e.invoke_gh_open_bar("repo".into());
    assert_eq!(e.get_bar_text().as_str(), "o/r");
}

#[test]
fn hiding_the_keyboard_keeps_what_was_typed() {
    let env = setup(None);
    let e = &env.editor;
    e.set_touch_input(true);
    e.set_fake_keyboard(300.);
    e.invoke_gh_open_new();
    e.invoke_gh_open_bar("folder".into());
    slint::platform::update_timers_and_animations();
    elapse(600);
    type_in_bar(e, "crates/ui");
    e.set_fake_keyboard(0.);
    elapse(100);
    elapse(400);
    assert!(!e.get_bar_visible());
    assert_eq!(e.get_gh_folder().as_str(), "crates/ui");
    // The token typed before the keyboard went away is stored, then cleared from the bar.
    e.set_fake_keyboard(300.);
    e.invoke_gh_open_bar("token".into());
    slint::platform::update_timers_and_animations();
    elapse(600);
    type_in_bar(e, TOKEN);
    e.set_fake_keyboard(0.);
    elapse(100);
    elapse(400);
    assert_eq!(e.get_gh_token_hint().as_str(), "…abcd");
    assert_eq!(e.get_bar_text().as_str(), "");
}

/// Blocks the next job in its first call. Returns (wait until it is there, let it go).
fn arm(env: &Env) -> (std::sync::mpsc::Receiver<()>, std::sync::mpsc::Sender<()>) {
    let (reached_tx, reached_rx) = std::sync::mpsc::channel();
    let (release_tx, release_rx) = std::sync::mpsc::channel();
    *env.shared.gate.lock().unwrap() = Some((reached_tx, release_rx));
    (reached_rx, release_tx)
}

fn reached(rx: &std::sync::mpsc::Receiver<()>) {
    rx.recv_timeout(Duration::from_secs(10)).expect("the transfer reached its call");
}

/// Lets the timers run a while (for results that must NOT show up).
fn pump(editor: &app::Editor) {
    for _ in 0..40 {
        i_slint_backend_testing::mock_elapsed_time(Duration::from_millis(60));
        slint::platform::update_timers_and_animations();
        std::thread::sleep(Duration::from_millis(5));
    }
    let _ = editor;
}

const LOCKED: &str = "GitHub と通信中のため、このプロジェクトは編集できません";

fn project_text(env: &Env, project: &str) -> String {
    std::fs::read_to_string(env.root().join(project).join("main.slint")).unwrap()
}

#[test]
fn a_pull_runs_in_the_background_and_locks_only_its_project() {
    let env = setup(Some(TOKEN));
    let e = &env.editor;
    linked(&env);
    e.invoke_gh_open();
    env.fake().commit("dev", &[("crates/ui/main.slint", Some(SRC2.as_bytes()))]);
    let (at_call, release) = arm(&env);
    e.invoke_gh_pull();
    reached(&at_call);
    assert!(e.get_gh_busy());
    assert!(e.get_gh_chip().contains("取り込み中"), "{}", e.get_gh_chip());

    // 閉じる works while busy (the dialog's own button just sets the mode and calls this).
    e.set_gh_mode(0);
    e.invoke_gh_closed();
    assert_eq!(e.get_gh_mode(), 0);

    // Every kind of edit of the locked project is refused, with the one message.
    let on_disk = project_text(&env, "ui");
    let z = e.get_zoom_level();
    e.invoke_tapped(60. * z, 60. * z);
    assert!(e.get_has_selection(), "selecting is viewing");
    e.invoke_drag_finished(40. * z, 40. * z);
    assert_eq!(e.get_error_text().as_str(), LOCKED);
    assert!(!e.get_dirty());
    e.invoke_prop_accepted("x".into(), "20".into());
    assert_eq!(e.get_error_text().as_str(), LOCKED);
    e.invoke_palette_picked("Rectangle".into());
    assert_eq!(e.get_error_text().as_str(), LOCKED);
    e.invoke_tree_action(4);
    assert_eq!(e.get_error_text().as_str(), LOCKED);
    e.invoke_undo();
    assert_eq!(e.get_error_text().as_str(), LOCKED);
    e.invoke_stack();
    assert_eq!(e.get_error_text().as_str(), LOCKED);
    e.invoke_save();
    assert_eq!(e.get_error_text().as_str(), LOCKED);
    e.invoke_file_op_run(1, "more".into(), "ui".into());
    assert_eq!(e.get_dialog_error().as_str(), LOCKED);
    e.invoke_file_op_run(3, "renamed".into(), "ui".into());
    assert_eq!(e.get_dialog_error().as_str(), LOCKED);
    assert!(!e.get_dirty());
    assert_eq!(project_text(&env, "ui"), on_disk);
    assert!(!env.root().join("ui/more.slint").exists());
    assert!(env.root().join("ui").is_dir() && !env.root().join("renamed").exists());
    // Viewing is still allowed: switching file/component inside the project.
    e.invoke_open("ui".into(), "main.slint".into());
    assert_eq!(e.get_current_project().as_str(), "ui");
    assert_eq!(e.get_error_text().as_str(), "", "{}", e.get_error_text());

    // Another project is fully usable: switch, edit, save, create.
    e.invoke_open("default".into(), "main.slint".into());
    assert_eq!(e.get_current_project().as_str(), "default");
    let before = project_text(&env, "default");
    e.invoke_palette_picked("Rectangle".into());
    assert!(e.get_dirty(), "{}", e.get_error_text());
    e.invoke_save();
    assert!(!e.get_dirty(), "{}", e.get_error_text());
    assert_ne!(project_text(&env, "default"), before);
    e.invoke_create_project("other".into());
    assert!(env.root().join("other").is_dir());
    // Back in the locked one: still locked.
    e.invoke_open("ui".into(), "main.slint".into());
    e.invoke_tapped(60. * z, 60. * z);
    e.invoke_drag_finished(40. * z, 40. * z);
    assert_eq!(e.get_error_text().as_str(), LOCKED);

    // The result is applied when the call returns, dialog closed or not.
    release.send(()).unwrap();
    wait(e);
    assert_eq!(project_text(&env, "ui"), SRC2);
    assert_eq!(strings(e.get_components()), vec!["Main", "Extra"], "the open project was reloaded");
    assert!(e.get_toast_text().contains("取り込みました"), "{}", e.get_toast_text());
    assert_eq!(e.get_gh_chip().as_str(), "");
    // Unlocked again.
    e.invoke_palette_picked("Rectangle".into());
    assert!(e.get_dirty(), "{}", e.get_error_text());
}

#[test]
fn a_pull_that_ends_with_conflicts_points_to_the_dialog() {
    let env = setup(Some(TOKEN));
    let e = &env.editor;
    linked(&env);
    std::fs::write(env.root().join("ui/other.slint"), "mine\n").unwrap();
    env.fake().commit("dev", &[("crates/ui/other.slint", Some(b"theirs\n"))]);
    let (at_call, release) = arm(&env);
    e.invoke_gh_pull();
    reached(&at_call);
    release.send(()).unwrap();
    wait(e);
    assert!(e.get_toast_text().contains("GitHub"), "{}", e.get_toast_text());
    assert!(e.get_gh_chip().contains("競合 1 件"), "{}", e.get_gh_chip());
    e.invoke_gh_open();
    assert_eq!(strings(e.get_gh_conflicts()), vec!["other.slint"]);
}

#[test]
fn a_push_locks_the_project_and_refuses_save() {
    let env = setup(Some(TOKEN));
    let e = &env.editor;
    linked(&env);
    std::fs::write(env.root().join("ui/other.slint"), "export component Other inherits Rectangle {}\n// x\n").unwrap();
    let (at_call, release) = arm(&env);
    e.invoke_gh_push("m".into());
    reached(&at_call);
    assert!(e.get_gh_chip().contains("送信中"), "{}", e.get_gh_chip());
    e.invoke_save();
    assert_eq!(e.get_error_text().as_str(), LOCKED);
    // The Android pause hook does not write the locked project either.
    assert!(app::glue::save_now());
    release.send(()).unwrap();
    wait(e);
    assert!(e.get_gh_result().contains("送りました"), "{}", e.get_gh_result());
    e.invoke_save();
    assert_eq!(e.get_error_text().as_str(), "");
}

#[test]
fn a_clone_in_progress_is_hidden_and_cannot_be_opened() {
    let env = setup(Some(TOKEN));
    let e = &env.editor;
    let (at_call, release) = arm(&env);
    import(&env, "ui");
    reached(&at_call);
    assert!(e.get_gh_busy());
    assert_eq!(strings(e.get_projects()), vec!["default"]);
    e.invoke_open("ui".into(), "main.slint".into());
    assert_eq!(e.get_current_project().as_str(), "default");
    e.invoke_create_project("ui".into());
    assert_eq!(strings(e.get_projects()), vec!["default"]);
    assert!(!e.get_error_text().is_empty());
    // The user may keep working elsewhere; closing the dialog is fine.
    e.set_gh_mode(0);
    e.invoke_gh_closed();
    release.send(()).unwrap();
    wait(e);
    // The dialog was closed: the user is not pulled into the new project.
    assert_eq!(e.get_current_project().as_str(), "default");
    assert_eq!(strings(e.get_projects()), vec!["default", "ui"]);
    assert!(e.get_toast_text().contains("取り込みました"), "{}", e.get_toast_text());
    e.invoke_open("ui".into(), "main.slint".into());
    assert_eq!(e.get_current_project().as_str(), "ui");
}

#[test]
fn a_second_transfer_is_refused_while_one_runs() {
    let env = setup(Some(TOKEN));
    let e = &env.editor;
    linked(&env);
    let (at_call, release) = arm(&env);
    e.invoke_gh_pull();
    reached(&at_call);
    e.invoke_gh_push("m".into());
    assert!(e.get_gh_error().contains("通信中"), "{}", e.get_gh_error());
    e.invoke_gh_pull();
    assert!(e.get_gh_error().contains("通信中"), "{}", e.get_gh_error());
    assert_eq!(env.shared.tokens.lock().unwrap().len(), 2, "no third worker was started");
    release.send(()).unwrap();
    wait(e);
    pump(e);
    assert!(!e.get_gh_busy());
}

#[test]
fn a_result_after_switching_projects_does_not_touch_the_new_one() {
    let env = setup(Some(TOKEN));
    let e = &env.editor;
    linked(&env);
    let default_components = {
        e.invoke_open("default".into(), "main.slint".into());
        strings(e.get_components())
    };
    e.invoke_open("ui".into(), "main.slint".into());
    e.invoke_gh_open();
    env.fake().commit("dev", &[("crates/ui/main.slint", Some(SRC2.as_bytes()))]);
    e.invoke_gh_pull();
    assert!(e.get_gh_busy());
    e.invoke_open("default".into(), "main.slint".into());
    wait(e);
    assert_eq!(e.get_current_project().as_str(), "default");
    assert_eq!(strings(e.get_components()), default_components, "no reload into the new project");
    assert!(!e.get_dirty());
    // The pull itself still landed in the project it was for.
    let ui = std::fs::read_to_string(env.root().join("ui/main.slint")).unwrap();
    assert_eq!(ui, SRC2);
}

#[test]
fn a_worker_that_dies_mid_clone_leaves_no_new_folder_but_keeps_an_old_one() {
    let env = setup(Some(TOKEN));
    let e = &env.editor;
    *env.shared.poison.lock().unwrap() = Some(env.root().join("ui"));
    import(&env, "ui");
    wait(e);
    assert!(e.get_gh_error().contains("途中で止まりました"), "{}", e.get_gh_error());
    assert!(!env.root().join("ui").exists(), "the half-made folder is removed");
    assert_eq!(strings(e.get_projects()), vec!["default"]);
    // A folder that was already there (empty) is not the job's to delete.
    std::fs::create_dir(env.root().join("ui")).unwrap();
    import(&env, "ui");
    wait(e);
    assert!(env.root().join("ui").is_dir());
}

#[test]
fn anchor_edits_are_refused_while_the_project_is_locked() {
    let env = setup(Some(TOKEN));
    let e = &env.editor;
    linked(&env);
    e.invoke_gh_open();
    env.fake().commit("dev", &[("crates/ui/main.slint", Some(SRC2.as_bytes()))]);
    let (at_call, release) = arm(&env);
    e.invoke_gh_pull();
    reached(&at_call);
    let on_disk = project_text(&env, "ui");
    let z = e.get_zoom_level();
    e.invoke_tapped(60. * z, 60. * z);
    assert!(e.get_has_selection());
    e.invoke_anchor_chosen(0, 1);
    assert_eq!(e.get_error_text().as_str(), LOCKED);
    assert!(!e.get_dirty());
    e.invoke_anchor_chosen(1, 3);
    assert_eq!(e.get_error_text().as_str(), LOCKED);
    e.invoke_infer_run();
    assert_eq!(e.get_error_text().as_str(), LOCKED);
    assert!(!e.get_dirty());
    assert_eq!(project_text(&env, "ui"), on_disk);
    release.send(()).unwrap();
    wait(e);
}
