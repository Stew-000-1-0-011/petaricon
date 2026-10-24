# petaricon の「この値を引数にする」 実装計画

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** 部品の中の値を引数(`in property`)にし、使う側の Props からその引数を渡せるようにする。

**Architecture:** slint-doc に新しい操作 `Op::Parameterize` を足し(宣言の追加と行の書き換えを 1 つの履歴の手にする)、session に型の表・断る理由・利用の引数の一覧を置き、glue と Props の画面に「引数にする」と引数の行を足す。

**Tech Stack:** Rust、Slint =1.18.1

**Spec:** `docs/superpowers/specs/2026-10-24-parameters-design.md`

## Global Constraints

- 設計書 1 章のとおり。特に「壊さない」と「書き込む前に確かめる」(2d2186a の `set_property` の検査を使う)。
- 画面の文言は日本語、タップの的は 44px 以上。Slint は `=1.18.1`。
- コミットの形は SDD の common.md のとおり。

## Review Focus

1. 引数にした直後のプレビューが、前と同じ見た目になる(既定値が元の式)。
2. 別ファイルの部品の利用でも、引数が Props に出て、値を渡せる。
3. 断る場合(値なし、`self`/`parent`、型不明、コンパイル不可)に、文書が変わらない。

---

### Task 1: 引数にする(slint-doc の操作と session)

**Files:** Modify `crates/slint-doc/src/{ops.rs,lib.rs}`、`crates/app/src/session.rs`、テスト

- [ ] **Step 1:** テスト: Text の `text` を引数にすると、宣言 1 行と `text: root.<名前>;` だけが変わる/ルート要素のプロパティでも動く/名前の初期値と重複時の `-2`/値なし・`self`・`parent`・すでに引数・型不明は断られて文書が同じ/元に戻すで一手で戻る/コンパイルできない結果は断る
- [ ] **Step 2〜4:** 失敗を確かめる → 実装(`Op::Parameterize { path, property, name, ty }`、型の表、`Session::parameterize`)→ テストが通る
- [ ] **Step 5:** コミット `Turn a property inside a component into a parameter`

### Task 2: 画面(「引数にする」と、利用の Props に引数の行)

**Files:** Modify `crates/app/src/{glue.rs,session.rs}`、`crates/app/ui/*.slint`、`crates/app/README.md`、テスト

- [ ] **Step 1:** Props の行に「引数にする」(部品の中の要素のときだけ)。押すと名前(と型が表にないときは型)を入れる小さなダイアログ
- [ ] **Step 2:** 利用を選ぶと、部品の `in`/`in-out` を Props に並べる(別ファイルは import をたどる)。値の書き方は Props と同じ。空で渡すのをやめる
- [ ] **Step 3:** テスト(`testdata/controller-ui` の一時コピー): 引数にする → 戻る → 利用に引数の行 → 値を入れるとプレビューの文字が変わる/別ファイルの部品
- [ ] **Step 4:** PC 版のスクリーンショット、README、`cargo test --workspace`、clippy、APK
- [ ] **Step 5:** コミット `Show and set component parameters from the editor`
