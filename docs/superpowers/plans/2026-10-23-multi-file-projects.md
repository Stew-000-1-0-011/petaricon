# petaricon の拡充(複数ファイルの画面) 実装計画

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** コントローラの画面ファイルを petaricon で壊さずに開き、利用を 1 つの節として扱い、中を開いて編集し、プレビュー用の値と画面の大きさを選べるようにする。

**Architecture:** 既存の 5 クレート(slint-doc / tidy / preview / storage / app)の延長で作る。プレビューの包み(`preview::compile` の wrapper)にプレビュー用の値を流し込み、当たり判定(`hits_at`)を利用の手前で止める。プレビュー用の値と画面の大きさは `storage` に `.preview` として保存する。

**Tech Stack:** Rust、Slint =1.18.1(slint-interpreter)

**Spec:** `docs/superpowers/specs/2026-10-23-multi-file-projects-design.md`

## Global Constraints

- 設計書 1 章の 6 つの決まりごと。特に「壊さない」(何も変えずに保存すれば 1 バイトも変わらない)。
- Slint は `=1.18.1` のまま。非公開 API は `preview` クレートだけで使う(今の決まり)。
- 画面の文言は日本語。タップの的は 44px 以上。
- コミットの形は SDD の common.md のとおり。

## Review Focus

1. 別ファイルで定義された部品と、同じファイルで定義された部品の両方で、利用が 1 つの節になる。
2. プレビュー用の値の式が誤っているとき・型が合わないときに、プレビュー全体が壊れない。
3. 「中を開く」で別のファイルへ移るときに、未保存の変更を失わない。
4. `.preview` が壊れていても、ないものとして起動できる。

---

### Task 1: コントローラの画面の写しで、今の petaricon を確かめる

**Files:** Create `testdata/controller-ui/`(oumuamua-controller の `crates/app/ui/` の写しとフォント、README に元のコミット)、`crates/app/tests/controller_ui.rs`(または該当クレートのテスト)

- [ ] **Step 1:** すべての部品をプレビューでコンパイルするテスト、何も変えずに保存して一致するテスト、要素を 1 つ動かして差分がその要素の `x`/`y` だけになるテストを書く
- [ ] **Step 2:** 実行して、失敗するものを報告にまとめる(直すのは次のタスク以降。ただし「壊さない」に反する不具合は、このタスクで直す)
- [ ] **Step 3:** コミット `Add controller UI fixtures and round-trip tests`

### Task 2: 利用を 1 つの節にする/中を開く/戻る

**Files:** Modify `crates/preview/src/lib.rs`(`hits_at` を利用の手前で止める)、`crates/app/src/session.rs`、`crates/app/src/glue.rs`、`crates/app/ui/*.slint`(「中を開く」「戻る」)

- [ ] **Step 1:** テスト: 同じファイルの部品の利用と、別ファイルの部品の利用の両方で、利用の中をタップすると利用が選ばれる/「中を開く」で定義へ移り、「戻る」で元のファイル・部品・選択に戻る/移る前に未保存の変更が保存される
- [ ] **Step 2〜4:** 失敗を確かめる → 実装 → テストが通る
- [ ] **Step 5:** コミット `Treat component uses as single nodes and open their definitions`

### Task 3: プレビュー用の値と画面の大きさ

**Files:** Modify `crates/preview/src/lib.rs`(包みに値を流し込む)、`crates/storage/src/lib.rs`(`.preview` の読み書き)、`crates/app/src/{session,glue}.rs`、`crates/app/ui/*.slint`(値のパネル、大きさの選択)

- [ ] **Step 1:** テスト: `MainWindow` の `connected=true`・`page=1` で手動の画面がプレビューに出る/誤った式は無視されて理由が出る/`.preview` の保存と読み込み/壊れた `.preview` は無視/大きさの選択で 800×360 になる
- [ ] **Step 2〜4:** 失敗を確かめる → 実装 → テストが通る
- [ ] **Step 5:** コミット `Add preview values and screen sizes`

### Task 4: 仕上げと APK

**Files:** Modify `crates/app/README.md`、必要な箇所

- [ ] **Step 1:** Task 1 の報告で残っている失敗を直す(素材の読み込みなど)
- [ ] **Step 2:** PC 版で `testdata/controller-ui` を開き、`main.slint` の 800×360 表示、手動の画面への切り替え(プレビュー用の値)、`ManualPanel` を中を開いて編集、戻る、のスクリーンショット
- [ ] **Step 3:** README 更新、`cargo test --workspace`、clippy、APK
- [ ] **Step 4:** コミット `Document multi-file projects`
