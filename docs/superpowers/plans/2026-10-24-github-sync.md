# petaricon の GitHub 連携 実装計画

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** GitHub のフォルダを petaricon のプロジェクトに取り込み、編集を同じブランチへ 1 コミットで送れるようにする。

**Architecture:** 新しいクレート `crates/ghsync` に、API 層(`trait GitHubApi` と `ureq` による本物の実装)と同期の手順(取り込む・送る・食い違いの判定・`.sync` の読み書き)を置く。手順は API のトレイトと、プロジェクトのフォルダのパスだけに依存し、Slint に依存しない。app は別スレッドで手順を呼び、結果を画面に出す。

**Tech Stack:** Rust、ureq(rustls、webpki-roots)、serde / serde_json、sha1(git の blob SHA)、base64

**Spec:** `docs/superpowers/specs/2026-10-24-github-sync-design.md`

## Global Constraints

- 設計書のとおり。特に: force push しない/両方で変わったファイルを黙って上書きしない/途中で失敗しても前か完了のどちらかの状態/トークンを画面(末尾 4 文字以外)とログに出さない。
- 管理ファイル(`.sync`・`.preview`・`.settings`・`.recovery`・`*.bak`・`*.local`)は送らない。
- 画面の文言は日本語、タップの的は 44px 以上。
- Slint は `=1.18.1` のまま。
- コミットの形は SDD の common.md のとおり。

## Review Focus

1. git の blob SHA の計算(`blob <len>\0<内容>` の SHA-1)が GitHub と一致する(既知の値でテスト)。
2. 改行や文字コードを変えない(取り込んだ内容をそのままのバイト列で書く)。
3. サブフォルダ(`fonts/`)のファイルが取り込み・送信の両方で正しい相対パスになる。
4. 通信のスレッドが画面のスレッドの文書を直接触らない(保存は通信の前に画面側で済ませる)。

---

### Task 1: `crates/ghsync`(API 層と同期の手順)

**Files:** Create `crates/ghsync/{Cargo.toml,src/lib.rs,src/api.rs,src/sync.rs,src/state.rs}`、`crates/ghsync/tests/sync.rs`

**Interfaces:**
- `pub trait GitHubApi`: `branch_head(repo, branch) -> Result<String /*commit sha*/>`、`commit_tree(repo, commit) -> Result<String /*tree sha*/>`、`tree_recursive(repo, tree) -> Result<Vec<TreeEntry{path, sha, mode, kind}>>`、`blob(repo, sha) -> Result<Vec<u8>>`、`create_blob(repo, bytes) -> Result<String>`、`create_tree(repo, base_tree, entries: Vec<NewEntry{path, sha: Option<String> /*None = 削除*/, mode}>) -> Result<String>`、`create_commit(repo, message, tree, parent) -> Result<String>`、`update_ref(repo, branch, commit) -> Result<(), UpdateError::NotFastForward | Other>`
- `pub struct HttpApi { token }`(ureq。`User-Agent`、`Authorization: Bearer`、`Accept: application/vnd.github+json`、`X-GitHub-Api-Version`。エラーを `ApiError::{Network, Unauthorized, NotFound, RateLimited, Other(status, message)}` に分ける)
- `pub struct SyncState`(`.sync` の内容)と読み書き(一時ファイル → 置き換え)
- `pub fn clone_into(api, repo, branch, folder, project_dir) -> Result<SyncState>`、`pub fn pull(api, project_dir) -> Result<PullReport{updated, deleted, conflicts, kept_local}>`、`pub fn take_remote(api, project_dir, path) -> Result<()>`(手元の版を `<path>.local` に退避して取り込む)、`pub fn local_changes(project_dir) -> Result<Vec<Change>>`、`pub fn push(api, project_dir, message) -> Result<PushReport>`(NotFastForward なら `Err(SyncError::BehindRemote)`)、`pub fn git_blob_sha(bytes) -> String`

- [ ] **Step 1:** テスト(偽の `GitHubApi` をテストの中に作る): 既知の blob SHA(空ファイル `e69de29bb2d1d6434b8b29ae775ad8c2e48c5391`、`hello\n` `ce013625030ba8dba906f756967f9e9ca394464a`)/clone でサブフォルダ込みのファイルが同じバイト列で置かれる/リモートだけの変更の取り込み/手元だけの変更は残る/両方の変更は conflicts に出て上書きされない/take_remote で `.local` に退避/リモートで消えたファイル/push が変更分だけを 1 コミットにする/管理ファイルは送らない/ブランチが進んでいたら BehindRemote で何も変えない/途中で API が失敗しても `.sync` とファイルが前の状態
- [ ] **Step 2〜4:** 失敗を確かめる → 実装 → `cargo test -p ghsync` と clippy
- [ ] **Step 5:** コミット `Add GitHub sync crate`

### Task 2: 画面(メニューの GitHub、新しいプロジェクト、取り込む・送る、トークン)

**Files:** Modify `crates/app/{Cargo.toml,src/glue.rs,src/session.rs(必要なら),ui/*.slint}`、`crates/storage`(トークンの保存)、`crates/app/README.md`

- [ ] **Step 1:** トークンの保存と表示(末尾 4 文字)、プロジェクトごとのリポジトリ・ブランチ・フォルダの入力、「取り込む」「送る」(メッセージ)、状態、「両方で変更」の一覧と「リモートの版にする」、「GitHub から新しいプロジェクト」。通信は別スレッドで、終わったら画面のスレッドで結果を反映(取り込んだら開いている文書を読み直す)。操作の前に開いている文書を保存
- [ ] **Step 2:** テスト(偽の API を差し込めるようにする): 新しいプロジェクト → 一覧に出て開ける/取り込みで開いている文書が更新される/送る前に保存される/トークンがログと画面に出ない(末尾 4 文字以外)
- [ ] **Step 3:** PC 版のスクリーンショット(偽の API で)、README にトークンの作り方(fine-grained、対象リポジトリだけ、Contents: Read and write)と使い方、APK
- [ ] **Step 4:** コミット `Add GitHub sync to the editor`
