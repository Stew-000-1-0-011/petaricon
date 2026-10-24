//! GitHub sync without any UI: job descriptions, running them on a project folder, and the
//! texts the dialog shows (Japanese). The glue starts a job on a worker thread and applies
//! the outcome on the UI thread.

use std::path::{Path, PathBuf};
use std::sync::Arc;

use ghsync::{
    ApiError, GitHubApi, PullReport, PushReport, SyncError, SyncState, clone_into, keep_local,
    local_changes, pull, push, take_remote,
};

pub type BoxApi = Box<dyn GitHubApi + Send>;
/// Makes the API client for a token. Tests pass a factory that returns a fake.
pub type ApiFactory = Arc<dyn Fn(&str) -> BoxApi + Send + Sync>;

/// The real GitHub over HTTPS.
pub fn http_factory() -> ApiFactory {
    Arc::new(|token| Box::new(ghsync::HttpApi::new(token)))
}

/// The default commit message of 「送る」.
pub const DEFAULT_MESSAGE: &str = "petaricon で編集";

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Job {
    Clone { repo: String, branch: String, folder: String },
    Pull,
    Push { message: String },
    TakeRemote(String),
    KeepLocal(String),
}

pub enum Outcome {
    Cloned(Result<SyncState, SyncError>),
    Pulled(Result<PullReport, SyncError>),
    Pushed(Result<PushReport, SyncError>),
    Resolved { path: String, took_remote: bool, result: Result<(), SyncError> },
}

/// Runs `job` against the project folder `dir`. A clone that fails leaves no folder behind
/// when it created one.
pub fn run(api: &dyn GitHubApi, dir: &Path, job: &Job) -> Outcome {
    match job {
        Job::Clone { repo, branch, folder } => {
            let existed = dir.exists();
            let r = clone_into(api, repo, branch, folder, dir);
            if r.is_err() && !existed {
                let _ = std::fs::remove_dir_all(dir);
            }
            Outcome::Cloned(r)
        }
        Job::Pull => Outcome::Pulled(pull(api, dir)),
        Job::Push { message } => Outcome::Pushed(push(api, dir, message)),
        Job::TakeRemote(path) => Outcome::Resolved {
            path: path.clone(),
            took_remote: true,
            result: take_remote(api, dir, path),
        },
        Job::KeepLocal(path) => Outcome::Resolved {
            path: path.clone(),
            took_remote: false,
            result: keep_local(api, dir, path),
        },
    }
}

/// The token as shown on screen: `…` and its last 4 characters.
pub fn mask_token(token: &str) -> String {
    let t = token.trim();
    let tail: Vec<char> = t.chars().rev().take(4).collect();
    if t.chars().count() <= 8 {
        return "…".to_string();
    }
    format!("…{}", tail.into_iter().rev().collect::<String>())
}

/// Removes every occurrence of the token from a text (defence in depth: no error text should
/// contain it, but none may reach the screen with it).
pub fn redact(text: &str, token: &str) -> String {
    let t = token.trim();
    if t.is_empty() { text.to_string() } else { text.replace(t, "…") }
}

pub fn api_error_text(e: &ApiError) -> String {
    match e {
        ApiError::Network(_) => {
            "通信できませんでした。ネットワークにつながっているか確認して、もう一度お試しください".into()
        }
        ApiError::Unauthorized => "認証に失敗しました。トークンが正しいか、期限が切れていないか、\
             対象のリポジトリに Contents の Read and write 権限があるか確認してください"
            .into(),
        ApiError::NotFound => "見つかりません。リポジトリ(owner/name)・ブランチ・フォルダの名前と、\
             トークンがそのリポジトリを使えるかを確認してください"
            .into(),
        ApiError::RateLimited => {
            "GitHub の利用上限に達しました。しばらく待ってからもう一度お試しください".into()
        }
        ApiError::Truncated => {
            "リポジトリが大きすぎて、ファイルの一覧を取得しきれませんでした".into()
        }
        ApiError::Other(code, msg) => format!("GitHub からエラーが返りました({code}): {msg}"),
    }
}

pub fn error_text(e: &SyncError) -> String {
    match e {
        SyncError::Api(a) => api_error_text(a),
        SyncError::Io(e) => format!("ファイルの読み書きに失敗しました: {e}"),
        SyncError::NotLinked => "このプロジェクトは GitHub とつながっていません".into(),
        SyncError::AlreadyLinked => "このプロジェクトはすでに GitHub とつながっています".into(),
        SyncError::BehindRemote => "GitHub のブランチが先に進んでいます。先に「取り込む」を押してください".into(),
        SyncError::ConflictsPending => "両方で変更されたファイルが残っているので送れません。\
             一覧の各ファイルで「リモートの版にする」か「手元の版を残す」を選んでから送ってください"
            .into(),
        SyncError::NothingToPush => "送る変更がありません".into(),
        SyncError::EmptyFolder => {
            "そのフォルダにファイルがありません。ブランチとフォルダのパスを確認してください".into()
        }
        SyncError::NotEmptyDir => {
            "同じ名前のプロジェクトがすでにあります。別のプロジェクト名にしてください".into()
        }
        SyncError::NotAConflict(p) => format!("{p} は「両方で変更」の一覧にありません"),
        SyncError::Corrupt(m) => format!("GitHub から想定外のデータが返りました: {m}"),
    }
}

pub fn pull_summary(r: &PullReport) -> String {
    let mut parts = Vec::new();
    if !r.updated.is_empty() {
        parts.push(format!("更新 {} 件", r.updated.len()));
    }
    if !r.deleted.is_empty() {
        parts.push(format!("削除 {} 件", r.deleted.len()));
    }
    if !r.kept_local.is_empty() {
        parts.push(format!("手元の変更を残したもの {} 件", r.kept_local.len()));
    }
    if !r.conflicts.is_empty() {
        parts.push(format!("両方で変更 {} 件(取り込んでいません)", r.conflicts.len()));
    }
    if parts.is_empty() {
        "取り込みました(新しい変更はありません)".into()
    } else {
        format!("取り込みました: {}", parts.join("、"))
    }
}

pub fn short_sha(sha: &str) -> &str {
    sha.get(..7).unwrap_or(sha)
}

pub fn push_summary(r: &PushReport) -> String {
    let mut parts = Vec::new();
    if r.added > 0 {
        parts.push(format!("追加 {} 件", r.added));
    }
    if r.modified > 0 {
        parts.push(format!("変更 {} 件", r.modified));
    }
    if r.deleted > 0 {
        parts.push(format!("削除 {} 件", r.deleted));
    }
    format!("送りました({}): {}", short_sha(&r.commit), parts.join("、"))
}

/// `2026-10-24 12:30 UTC` from Unix seconds.
pub fn format_time(secs: u64) -> String {
    let days = (secs / 86_400) as i64;
    let rem = secs % 86_400;
    // Civil date from days since 1970-01-01 (Howard Hinnant's algorithm).
    let z = days + 719_468;
    let era = z.div_euclid(146_097);
    let doe = z.rem_euclid(146_097);
    let yoe = (doe - doe / 1460 + doe / 36_524 - doe / 146_096) / 365;
    let doy = doe - (365 * yoe + yoe / 4 - yoe / 100);
    let mp = (5 * doy + 2) / 153;
    let d = doy - (153 * mp + 2) / 5 + 1;
    let m = if mp < 10 { mp + 3 } else { mp - 9 };
    let y = yoe + era * 400 + i64::from(m <= 2);
    format!("{y:04}-{m:02}-{d:02} {:02}:{:02} UTC", rem / 3600, rem % 3600 / 60)
}

/// What the dialog shows about a project.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct View {
    pub linked: bool,
    /// `owner/name · branch · folder`.
    pub link_text: String,
    pub status_text: String,
    pub conflicts: Vec<String>,
}

pub fn view(dir: &Path) -> View {
    match SyncState::load(dir) {
        Ok(Some(state)) => {
            let folder = if state.folder.is_empty() { "(リポジトリの直下)" } else { &state.folder };
            let link_text = format!("{} · {} · {}", state.repo, state.branch, folder);
            let mut lines = vec![format!(
                "最後の同期: {}({})",
                short_sha(&state.base_commit),
                if state.synced_at == 0 { "不明".to_string() } else { format_time(state.synced_at) }
            )];
            match local_changes(dir) {
                Ok(c) if c.is_empty() => lines.push("手元の変更: なし".into()),
                Ok(c) => lines.push(format!("手元の変更: {} 件", c.len())),
                Err(e) => lines.push(error_text(&e)),
            }
            if !state.conflicts.is_empty() {
                lines.push(format!("両方で変更: {} 件(解決するまで送れません)", state.conflicts.len()));
            }
            View { linked: true, link_text, status_text: lines.join("\n"), conflicts: state.conflicts }
        }
        Ok(None) => View::default(),
        Err(e) => View {
            status_text: format!(".sync を読めませんでした: {e}"),
            ..View::default()
        },
    }
}

/// Like `view`, but without scanning the files: a transfer is writing them right now. Only
/// the `.sync` (always replaced atomically) is read.
pub fn view_busy(dir: &Path) -> View {
    match SyncState::load(dir) {
        Ok(Some(state)) => {
            let folder = if state.folder.is_empty() { "(リポジトリの直下)" } else { &state.folder };
            View {
                linked: true,
                link_text: format!("{} · {} · {}", state.repo, state.branch, folder),
                status_text: "GitHub と通信中です。終わるとここに状態が出ます".into(),
                conflicts: state.conflicts,
            }
        }
        _ => View::default(),
    }
}

/// What the user typed for a new project, checked before anything is sent.
pub struct CloneInput {
    pub repo: String,
    pub branch: String,
    pub folder: String,
    pub name: String,
}

pub fn check_clone_input(repo: &str, branch: &str, folder: &str, name: &str) -> Result<CloneInput, String> {
    let (repo, branch, folder) = (repo.trim(), branch.trim(), folder.trim().trim_matches('/'));
    let ok_repo = repo.split_once('/').is_some_and(|(o, n)| {
        let good = |s: &str| {
            !s.is_empty() && s.chars().all(|c| c.is_ascii_alphanumeric() || matches!(c, '-' | '_' | '.'))
        };
        good(o) && !o.starts_with('.') && good(n) && n != "." && n != ".."
    });
    if !ok_repo {
        return Err("リポジトリは owner/name の形で入力してください(例: octocat/hello)".into());
    }
    if branch.is_empty() {
        return Err("ブランチ名を入力してください".into());
    }
    let name = match name.trim() {
        "" => folder.rsplit('/').next().filter(|s| !s.is_empty()).unwrap_or_else(|| {
            repo.split_once('/').map_or(repo, |(_, n)| n)
        }),
        n => n,
    };
    storage::check_name(name).map_err(|_| {
        format!("{name:?} はプロジェクト名に使えません(/ \\ .. は不可、先頭のドットも不可)")
    })?;
    Ok(CloneInput {
        repo: repo.into(),
        branch: branch.into(),
        folder: folder.into(),
        name: name.into(),
    })
}

/// Where a project lives, or why that is unknown.
pub fn project_dir(store: &dyn storage::ProjectStore, project: &str) -> Result<PathBuf, String> {
    store
        .project_path(project)
        .ok_or_else(|| "この保存先は GitHub 連携に対応していません".to_string())
}

#[cfg(test)]
mod tests {
    use super::*;
    use ghsync::fake::{Fake, REPO};

    fn seeded() -> Fake {
        let f = Fake::new();
        f.commit(
            "main",
            &[
                ("ui/main.slint", Some(b"export component A {}\n")),
                ("ui/b.slint", Some(b"b\n")),
            ],
        );
        f
    }

    #[test]
    fn token_is_masked_to_four_characters() {
        assert_eq!(mask_token("github_pat_11ABCDEFG0123456789abcd"), "…abcd");
        assert_eq!(mask_token("  github_pat_xyz9  "), "…xyz9");
        // Too short to show any part of.
        assert_eq!(mask_token("abcd1234"), "…");
        assert_eq!(mask_token(""), "…");
        assert!(!mask_token("github_pat_11ABCDEFG0123456789abcd").contains("11ABCDEFG"));
    }

    #[test]
    fn redact_removes_the_token_everywhere() {
        let t = "github_pat_SECRET123";
        assert_eq!(redact(&format!("x {t} y {t}"), t), "x … y …");
        assert_eq!(redact("nothing", ""), "nothing");
    }

    #[test]
    fn every_failure_has_a_japanese_text() {
        let all = [
            SyncError::Api(ApiError::Network("x".into())),
            SyncError::Api(ApiError::Unauthorized),
            SyncError::Api(ApiError::NotFound),
            SyncError::Api(ApiError::RateLimited),
            SyncError::Api(ApiError::Truncated),
            SyncError::Api(ApiError::Other(500, "oops".into())),
            SyncError::BehindRemote,
            SyncError::ConflictsPending,
            SyncError::EmptyFolder,
            SyncError::NotEmptyDir,
            SyncError::NothingToPush,
            SyncError::NotLinked,
        ];
        for e in &all {
            let t = error_text(e);
            assert!(!t.is_ascii(), "{e:?}: {t}");
        }
        assert!(error_text(&SyncError::BehindRemote).contains("取り込む"));
        assert!(error_text(&SyncError::ConflictsPending).contains("リモートの版にする"));
        // Network details (which can carry URLs) are not shown.
        assert!(!error_text(&SyncError::Api(ApiError::Network("secret-detail".into()))).contains("secret-detail"));
    }

    #[test]
    fn times_are_utc() {
        assert_eq!(format_time(0), "1970-01-01 00:00 UTC");
        assert_eq!(format_time(1_761_300_000), "2025-10-24 10:00 UTC");
        assert_eq!(format_time(1_709_251_199 + 1), "2024-03-01 00:00 UTC");
    }

    #[test]
    fn clone_input_is_checked() {
        let ok = check_clone_input(" o/r ", "main", "/crates/app/ui/", "").unwrap();
        assert_eq!((ok.repo.as_str(), ok.folder.as_str(), ok.name.as_str()), ("o/r", "crates/app/ui", "ui"));
        let root = check_clone_input("o/proj", "main", "", "").unwrap();
        assert_eq!(root.name, "proj");
        assert_eq!(check_clone_input("o/r", "main", "x", "mine").unwrap().name, "mine");
        for bad in ["", "o", "o/", "/r", "o/r/x", "o r/x", "../x"] {
            assert!(check_clone_input(bad, "main", "", "").is_err(), "{bad}");
        }
        assert!(check_clone_input("o/r", "", "", "").is_err());
        assert!(check_clone_input("o/r", "main", "", "../x").is_err());
        assert!(check_clone_input("o/r", "main", "", ".hidden").is_err());
    }

    #[test]
    fn run_clone_pull_push_cycle() {
        let f = seeded();
        let d = tempfile::tempdir().unwrap();
        let dir = d.path().join("p");
        let job = Job::Clone { repo: REPO.into(), branch: "main".into(), folder: "ui".into() };
        let Outcome::Cloned(Ok(st)) = run(&f, &dir, &job) else { panic!("clone") };
        assert_eq!(st.folder, "ui");
        assert!(view(&dir).linked);
        assert_eq!(view(&dir).link_text, "o/r · main · ui");
        assert!(view(&dir).status_text.contains("手元の変更: なし"));
        std::fs::write(dir.join("main.slint"), "changed\n").unwrap();
        assert!(view(&dir).status_text.contains("手元の変更: 1 件"));
        let Outcome::Pushed(Ok(r)) = run(&f, &dir, &Job::Push { message: "m".into() }) else {
            panic!("push")
        };
        assert_eq!(r.modified, 1);
        assert!(push_summary(&r).contains("変更 1 件"));
        f.commit("main", &[("ui/b.slint", Some(b"b2\n"))]);
        let Outcome::Pulled(Ok(p)) = run(&f, &dir, &Job::Pull) else { panic!("pull") };
        assert_eq!(p.updated, vec!["b.slint"]);
        assert_eq!(pull_summary(&p), "取り込みました: 更新 1 件");
    }

    #[test]
    fn failed_clone_leaves_no_folder_but_keeps_an_existing_one() {
        let f = Fake::new();
        let d = tempfile::tempdir().unwrap();
        let dir = d.path().join("new");
        let job = Job::Clone { repo: REPO.into(), branch: "nope".into(), folder: "ui".into() };
        let Outcome::Cloned(Err(SyncError::Api(ApiError::NotFound))) = run(&f, &dir, &job) else {
            panic!()
        };
        assert!(!dir.exists());
        let empty = d.path().join("empty");
        std::fs::create_dir(&empty).unwrap();
        run(&f, &empty, &job);
        assert!(empty.is_dir());
    }

    #[test]
    fn view_lists_conflicts() {
        let f = seeded();
        let d = tempfile::tempdir().unwrap();
        let job = Job::Clone { repo: REPO.into(), branch: "main".into(), folder: "ui".into() };
        run(&f, d.path(), &job);
        std::fs::write(d.path().join("b.slint"), "mine\n").unwrap();
        f.commit("main", &[("ui/b.slint", Some(b"theirs\n"))]);
        let Outcome::Pulled(Ok(p)) = run(&f, d.path(), &Job::Pull) else { panic!() };
        assert_eq!(p.conflicts, vec!["b.slint"]);
        let v = view(d.path());
        assert_eq!(v.conflicts, vec!["b.slint"]);
        assert!(v.status_text.contains("両方で変更: 1 件"));
        let Outcome::Pushed(Err(SyncError::ConflictsPending)) =
            run(&f, d.path(), &Job::Push { message: "m".into() })
        else {
            panic!()
        };
    }

    #[test]
    fn unlinked_project_view_is_empty() {
        let d = tempfile::tempdir().unwrap();
        assert_eq!(view(d.path()), View::default());
    }
}
