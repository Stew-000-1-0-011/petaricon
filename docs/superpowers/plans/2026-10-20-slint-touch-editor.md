# Slint タッチエディタ 実装計画

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** スマホ(Android)単体でも使える、自由配置 → 整形 → 数値調整ができる `.slint` ノーコードエディタを作る。

**Architecture:** `.slint` テキストを唯一の正本とし、`i-slint-compiler` の構文木上で最小差分の編集を行う(`slint-doc`)。表示は `slint-interpreter` で実行時コンパイルし `ComponentContainer` に埋め込む(`preview`)。整形は純粋な幾何計算(`tidy`)。保存先は差し替え可能な `storage`。これらを Slint 製のエディタ UI(`app`)がつなぐ。

**Tech Stack:** Rust (edition 2024), Slint 1.18.1(`slint`, `slint-interpreter`, `i-slint-compiler`, `i-slint-core`, `i-slint-backend-testing`), cargo-apk, Android NDK 29

**Spec:** `docs/superpowers/specs/2026-10-20-slint-touch-editor-design.md`

## Global Constraints

- Slint 系クレートはすべて `=1.18.1` の完全一致で固定する
- 非公開 API(`slint-interpreter` の `internal` / `internal-highlight`、`i-slint-core`)を使ってよいのは `preview` クレートだけ。`app` の `.slint` が `ComponentContainer` を使うための `SLINT_ENABLE_EXPERIMENTAL_FEATURES` は `app/build.rs` で設定する
- `slint-doc` と `tidy` は Slint の UI・interpreter に依存しない
- 編集は触っていない記述を 1 文字も変えない
- グリッドの初期値は 4px
- Android の保存先は `AndroidApp::external_data_path()/projects/`、PC は起動引数のフォルダ(省略時 `./projects`)
- Android: `min_sdk_version = 26`, `target_sdk_version = 35`, ターゲット `aarch64-linux-android`, パッケージ名 `dev.petaricon.editor`, 表示名 `Petaricon Editor`
- release ビルドは `~/.android/debug.keystore`(パスワード `android`)で署名する
- `spike/` 以下は参考のみ。変更も依存もしない
- コミットメッセージの末尾には、セッションのシステムリマインダーが指定する帰属行を付ける

## Review Focus

1. **複数バイト文字(日本語)を含む `.slint`**: オフセットはバイト単位。日本語の `text:` を含むファイルで全操作が壊れないこと → Task 3 と Task 5 にテストを追加
2. **コメントや空行が要素の間にある**: 移動・整形・部品化でコメントが消えたり別の要素に付いたりしないこと → Task 4 と Task 5 にテストを追加
3. **`id := Type { }` 形式の要素**: 経路の解決、移動、部品化で `id :=` が保持されること → Task 2 と Task 6 にテストを追加
4. **コンパイルエラーの状態での操作**: 直前の表示が残り、選択・ヒットテストは直前のインスタンスで動き、編集は受け付けること → Task 9 にテストを追加
5. **子要素が 1 つ、または 0 のコンテナの整形**: 1 つなら縦並び・警告なし、0 なら整形を断ること → Task 1 と Task 13 にテストを追加

---

## ファイル構成

```
Cargo.toml                         ワークスペース(members = crates/*)
crates/tidy/src/lib.rs             整形計画(Rect, Plan, plan())
crates/slint-doc/src/lib.rs        Document, ElementPath, Op, 公開型
crates/slint-doc/src/syntax.rs     構文木の探索(経路解決、子列挙、インデント検出)
crates/slint-doc/src/ops.rs        各 Op のテキスト差分生成
crates/slint-doc/src/history.rs    元に戻す/やり直し
crates/slint-doc/tests/*.rs        Op ごとのゴールデンテスト
crates/preview/src/lib.rs          compile(), Compiled, LiveInstance, Hit, Diagnostic
crates/storage/src/lib.rs          ProjectStore トレイト, LocalStore
crates/app/build.rs                実験的機能を有効にして ui/editor.slint をコンパイル
crates/app/ui/editor.slint         メインウィンドウ(ツールバー、エラー帯、キャンバス、下部パネル)
crates/app/ui/canvas.slint         選択枠、ハンドル、ガイドの重ね描き
crates/app/ui/panels.slint         ツリー、部品、プロパティ、ソースのタブ
crates/app/src/lib.rs              run(), android_main
crates/app/src/main.rs             PC 用の起動
crates/app/src/session.rs          Session(UI 非依存の編集セッション)
crates/app/src/layout_bridge.rs    tidy::Plan → slint_doc::LayoutSpec
crates/app/src/guides.rs           ガイドの計算
crates/app/src/glue.rs             Slint のコールバックと Session の接続
crates/app/src/crashlog.rs         パニックのログ記録
crates/app/tests/session.rs        Session の結合テスト(testing バックエンド)
```

---

### Task 1: ワークスペースと `tidy` クレート

**Files:**
- Create: `Cargo.toml`, `.gitignore`(`target/`), `crates/tidy/Cargo.toml`, `crates/tidy/src/lib.rs`
- Test: `crates/tidy/src/lib.rs` の `#[cfg(test)] mod tests`

**Interfaces:**
- Produces:
  - `pub struct Rect { pub x: f32, pub y: f32, pub w: f32, pub h: f32 }`(`Clone, Copy, Debug, PartialEq`)
  - `pub enum Direction { Horizontal, Vertical }`
  - `pub enum MainAlign { Start, Center, End }`
  - `pub enum CrossAlign { Stretch, Start, Center, End }`
  - `pub struct Padding { pub left: i32, pub right: i32, pub top: i32, pub bottom: i32 }`
  - `pub enum Warning { AmbiguousDirection, Overlap }`
  - `pub struct Plan { pub direction: Direction, pub order: Vec<usize>, pub spacing: i32, pub padding: Padding, pub main_align: MainAlign, pub cross_align: CrossAlign, pub warnings: Vec<Warning> }`
  - `pub fn plan(container_w: f32, container_h: f32, children: &[Rect], grid: i32) -> Option<Plan>`(子が 0 なら `None`)
  - すべて `Debug, Clone, PartialEq`、列挙型は `Copy, Eq` も

- [ ] **Step 1: 失敗するテストを書く**

```rust
fn r(x: f32, y: f32, w: f32, h: f32) -> Rect { Rect { x, y, w, h } }

#[test] fn empty_is_none() { assert_eq!(plan(300., 200., &[], 4), None); }

#[test] fn single_child_is_vertical_without_warning() {
    let p = plan(300., 200., &[r(20., 30., 100., 40.)], 4).unwrap();
    assert_eq!(p.direction, Direction::Vertical);
    assert!(p.warnings.is_empty());
    assert_eq!(p.order, vec![0]);
}

#[test] fn horizontal_row_with_uneven_gaps() {
    let kids = [r(150., 22., 40., 30.), r(16., 20., 40., 30.), r(66., 18., 40., 30.), r(197., 21., 40., 30.)];
    let p = plan(300., 80., &kids, 4).unwrap();
    assert_eq!(p.direction, Direction::Horizontal);
    assert_eq!(p.order, vec![1, 2, 0, 3]);
    assert_eq!(p.spacing, 12);
    assert_eq!(p.padding, Padding { left: 16, right: 16, top: 20, bottom: 28 });
    assert_eq!(p.main_align, MainAlign::Start);
    assert_eq!(p.cross_align, CrossAlign::Center);
}

#[test] fn vertical_centered_column() {
    let kids = [r(100., 20., 100., 30.), r(80., 58., 140., 30.), r(110., 96., 80., 30.)];
    let p = plan(300., 146., &kids, 4).unwrap();
    assert_eq!(p.direction, Direction::Vertical);
    assert_eq!(p.spacing, 8);
    assert_eq!(p.padding, Padding { left: 80, right: 80, top: 20, bottom: 20 });
    assert_eq!(p.main_align, MainAlign::Center);
    assert_eq!(p.cross_align, CrossAlign::Center);
}

#[test] fn full_width_rows_stretch() {
    let kids = [r(12., 10., 276., 30.), r(12., 48., 275., 30.)];
    let p = plan(300., 200., &kids, 4).unwrap();
    assert_eq!(p.cross_align, CrossAlign::Stretch);
}

#[test] fn overlap_warns_and_clamps() {
    let kids = [r(10., 10., 50., 30.), r(50., 12., 50., 30.)];
    let p = plan(200., 60., &kids, 4).unwrap();
    assert_eq!(p.spacing, 0);
    assert!(p.warnings.contains(&Warning::Overlap));
}

#[test] fn grid_layout_is_ambiguous() {
    let kids = [r(10., 10., 40., 40.), r(60., 10., 40., 40.), r(10., 60., 40., 40.), r(60., 60., 40., 40.)];
    let p = plan(200., 200., &kids, 4).unwrap();
    assert!(p.warnings.contains(&Warning::AmbiguousDirection));
}
```

期待値はすべて下のアルゴリズムから手計算した値(例: 横並びの gaps は 10, 44, 7 → 中央値 10 → `round(2.5) = 3` で 12)。

- [ ] **Step 2: 失敗を確認** — Run: `cargo test -p tidy` / Expected: コンパイルエラー(`plan` 未定義)

- [ ] **Step 3: `plan()` を実装**

アルゴリズム(スペック 4 章の具体値):

```text
snap(v) = max(0, round(v / grid) * grid)      // round は四捨五入(0.5 は切り上げ)
軸の取り方: Horizontal なら main = x/w, cross = y/h。Vertical はその逆
方向:
  子が 1 つ → Vertical
  H候補 = x 順に並べて隣接ペアすべてで y 区間が重なる(重なり量 > 0)
  V候補 = y 順に並べて隣接ペアすべてで x 区間が重なる
  H候補のみ → H、V候補のみ → V
  両方 or どちらでもない → 中心の範囲(max-min)が大きい軸。等しければ V
  どちらでもない場合だけ AmbiguousDirection を警告
order = main 開始座標の昇順(同値は元の添字順)
gaps = 隣接ペアの (次.start - (今.start + 今.size));負があれば Overlap 警告し 0 に
spacing = snap(gaps の中央値)。偶数個は中央 2 つの平均。gaps が空なら 0
lead = 先頭.start, trail = container_main - 末尾.end
main_align: lead > 0 && trail > 0 && |lead - trail| <= max(grid, 0.1 * container_main) → Center
            それ以外で lead <= trail → Start、他は End
main 方向 padding: Start → (lead, min(lead, trail)), End → (min(lead, trail), trail), Center → (min, min)
cross_start = min(子.cross_start), cross_end = container_cross - max(子.cross_end)
cross padding = (cross_start, cross_end)
band = max(子.cross_end) - min(子.cross_start); avail = container_cross - cross_start - cross_end
cross_align: 子が 2 つ以上 かつ 全子の cross size >= 0.9 * band かつ band >= 0.9 * avail → Stretch
             それ以外は Start / Center / End のうち偏差合計が最小のもの(同点は Start, Center, End の順で優先)
             Start 偏差 = 子.cross_start - min_start、End 偏差 = max_end - 子.cross_end、
             Center 偏差 = |子.cross_center - 全子 cross_center の平均|
padding の各値は snap() を通す
```

- [ ] **Step 4: 通過を確認** — Run: `cargo test -p tidy` / Expected: 7 passed

- [ ] **Step 5: コミット** — `git add Cargo.toml .gitignore crates/tidy && git commit -m "Add workspace and tidy layout planner"`

---

### Task 2: `slint-doc` の読み取り(経路、ツリー、要素情報)

**Files:**
- Create: `crates/slint-doc/Cargo.toml`(依存: `i-slint-compiler = { version = "=1.18.1", default-features = false }`, `thiserror`)、`src/lib.rs`, `src/syntax.rs`
- Test: `crates/slint-doc/tests/read.rs`

**Interfaces:**
- Produces(`lib.rs` で公開):
  - `pub struct ElementPath { pub component: String, pub indices: Vec<usize> }`(`Clone, Debug, PartialEq, Eq, Hash`)。`indices` は「要素の子」の添字列。空ならコンポーネントのルート
  - 「要素の子」= `Element` 直下の `SubElement` / `RepeatedElement` / `ConditionalElement` / `MatchElement` ノード(ソース順)
  - `pub enum LayoutKind { Horizontal, Vertical, Grid, Other }`(`HorizontalLayout`/`HorizontalBox` → Horizontal、`VerticalLayout`/`VerticalBox` → Vertical、`GridLayout`/`GridBox` → Grid、`FlexboxLayout` など他の `*Layout` → Other)
  - `pub enum ElementKind { Plain, Layout(LayoutKind), Repeated, Conditional }`
  - `pub struct Binding { pub name: String, pub expr: String }`(`expr` は末尾の `;` を除いた式のテキスト)
  - `pub struct ElementInfo { pub path: ElementPath, pub type_name: String, pub id: Option<String>, pub kind: ElementKind, pub bindings: Vec<Binding>, pub child_count: usize, pub start: u32 }`(`start` は `Element` ノードの開始バイトオフセット。Repeated/Conditional では内側の `Element` の開始。`type_name` は Repeated/Conditional でも内側の型名)
  - `pub struct TreeNode { pub info: ElementInfo, pub children: Vec<TreeNode> }`
  - `pub struct ComponentInfo { pub name: String, pub base: String, pub exported: bool, pub declarations: Vec<String> }`(`declarations` は `PropertyDeclaration` / `CallbackDeclaration` ノードのテキスト)
  - `pub struct Document`、`impl Document { pub fn new(path: PathBuf, text: String) -> Self; pub fn text(&self) -> &str; pub fn path(&self) -> &Path; pub fn components(&self) -> Vec<ComponentInfo>; pub fn tree(&self, component: &str) -> Option<TreeNode>; pub fn element(&self, path: &ElementPath) -> Option<ElementInfo>; pub fn path_at_offset(&self, offset: u32) -> Option<ElementPath> }`
  - `path_at_offset` は `Element` ノードの開始がちょうど `offset` の要素の経路を返す(preview の `Hit::start` との対応付け用)

- [ ] **Step 1: 失敗するテストを書く**(テスト用ソースはファイル先頭の定数 `SRC` に置く)

```rust
const SRC: &str = r#"import { Button } from "std-widgets.slint";
component Chip inherits Rectangle {
    in property <string> label;
    Text { text: root.label; }
}
export component Main inherits Window {
    // header
    title := Text { x: 8px; y: 4px; text: "見出し"; }
    VerticalLayout {
        Button { text: "A"; }
        for i in 3: Chip { label: "c"; }
    }
}
"#;

#[test] fn lists_components() {
    let d = Document::new("m.slint".into(), SRC.into());
    let c = d.components();
    assert_eq!(c.iter().map(|c| c.name.as_str()).collect::<Vec<_>>(), ["Chip", "Main"]);
    assert!(!c[0].exported && c[1].exported);
    assert_eq!(c[0].declarations, ["in property <string> label;"]);
}

#[test] fn element_info_with_id_and_bindings() {
    let d = Document::new("m.slint".into(), SRC.into());
    let e = d.element(&ElementPath { component: "Main".into(), indices: vec![0] }).unwrap();
    assert_eq!(e.type_name, "Text");
    assert_eq!(e.id.as_deref(), Some("title"));
    assert_eq!(e.bindings.iter().map(|b| (b.name.as_str(), b.expr.as_str())).collect::<Vec<_>>(),
               [("x", "8px"), ("y", "4px"), ("text", "\"見出し\"")]);
}

#[test] fn tree_kinds() {
    let d = Document::new("m.slint".into(), SRC.into());
    let t = d.tree("Main").unwrap();
    assert_eq!(t.children[1].info.kind, ElementKind::Layout(LayoutKind::Vertical));
    assert_eq!(t.children[1].children[1].info.kind, ElementKind::Repeated);
    assert_eq!(t.children[1].children[1].info.type_name, "Chip");
}

#[test] fn offset_roundtrip() {
    let d = Document::new("m.slint".into(), SRC.into());
    let p = ElementPath { component: "Main".into(), indices: vec![1, 0] };
    let e = d.element(&p).unwrap();
    assert_eq!(d.path_at_offset(e.start), Some(p));
}
```

- [ ] **Step 2: 失敗を確認** — Run: `cargo test -p slint-doc --test read` / Expected: コンパイルエラー

- [ ] **Step 3: 実装** — `syntax.rs` に `parse(text, path) -> SyntaxNode`(`i_slint_compiler::parser::parse` と `BuildDiagnostics::default()`)、`component_node(root, name)`、`element_children(element) -> Vec<SyntaxNode>`、`inner_element(child) -> SyntaxNode`(Sub/Repeated/Conditional の中の `Element`)、`resolve(root, &ElementPath) -> Option<SyntaxNode>` を置く。`Document` は呼び出しごとに parse する(キャッシュしない。ファイルは小さい)。`id` は `SubElement` の `Identifier` トークン

- [ ] **Step 4: 通過を確認** — Run: `cargo test -p slint-doc --test read` / Expected: 4 passed

- [ ] **Step 5: コミット** — `git commit -m "Add slint-doc read model: paths, tree, element info"`

---

### Task 3: `slint-doc` のプロパティ編集と履歴

**Files:**
- Create: `crates/slint-doc/src/ops.rs`, `src/history.rs`
- Modify: `crates/slint-doc/src/lib.rs`
- Test: `crates/slint-doc/tests/bindings.rs`

**Interfaces:**
- Consumes: Task 2 の `Document`, `ElementPath`, `syntax::resolve`
- Produces:
  - `pub enum Op { SetBindings { path: ElementPath, bindings: Vec<(String, Option<String>)> }, /* 以降の Task で追加 */ }`(`None` は削除)
  - `#[derive(Debug, thiserror::Error)] pub enum EditError { ElementNotFound(ElementPath), ComponentNotFound(String), InvalidTarget(String) }`
  - `impl Document { pub fn apply(&mut self, op: Op) -> Result<Option<ElementPath>, EditError>; pub fn replace_text(&mut self, text: String); pub fn undo(&mut self) -> bool; pub fn redo(&mut self) -> bool; pub fn is_dirty(&self) -> bool; pub fn mark_saved(&mut self) }`
  - `apply` は成功時に操作対象の要素の(操作後の)経路を返す。`SetBindings` では入力の `path` をそのまま返す
  - 1 回の `apply` / `replace_text` は 1 回の undo で戻る。テキストが変わらない操作は履歴に積まない

- [ ] **Step 1: 失敗するテストを書く**

```rust
// SRC は Task 2 と同じ内容をこのファイルにも置く
fn p(i: &[usize]) -> ElementPath { ElementPath { component: "Main".into(), indices: i.to_vec() } }

#[test] fn replaces_existing_binding_only() {
    let mut d = Document::new("m.slint".into(), SRC.into());
    d.apply(Op::SetBindings { path: p(&[0]), bindings: vec![("x".into(), Some("20px".into()))] }).unwrap();
    assert_eq!(d.text(), SRC.replace("x: 8px;", "x: 20px;"));
}

#[test] fn inserts_missing_binding_after_brace() {
    let mut d = Document::new("m.slint".into(), SRC.into());
    d.apply(Op::SetBindings { path: p(&[1, 0]), bindings: vec![("width".into(), Some("80px".into()))] }).unwrap();
    assert_eq!(d.text(), SRC.replace(r#"Button { text: "A"; }"#, r#"Button { width: 80px; text: "A"; }"#));
}

#[test] fn removes_binding_with_its_whitespace() {
    let mut d = Document::new("m.slint".into(), SRC.into());
    d.apply(Op::SetBindings { path: p(&[0]), bindings: vec![("y".into(), None)] }).unwrap();
    assert_eq!(d.text(), SRC.replace(" y: 4px;", ""));
}

#[test] fn multibyte_text_survives() {  // Review Focus 1
    let mut d = Document::new("m.slint".into(), SRC.into());
    d.apply(Op::SetBindings { path: p(&[0]), bindings: vec![("x".into(), Some("1px".into())), ("y".into(), Some("2px".into()))] }).unwrap();
    assert!(d.text().contains(r#"text: "見出し";"#));
    assert!(d.text().contains("x: 1px; y: 2px;"));
}

#[test] fn undo_redo_and_dirty() {
    let mut d = Document::new("m.slint".into(), SRC.into());
    assert!(!d.is_dirty());
    d.apply(Op::SetBindings { path: p(&[0]), bindings: vec![("x".into(), Some("9px".into()))] }).unwrap();
    assert!(d.is_dirty());
    assert!(d.undo()); assert_eq!(d.text(), SRC); assert!(!d.is_dirty());
    assert!(d.redo()); assert!(d.text().contains("x: 9px;"));
    assert!(!d.redo());
}

#[test] fn missing_element_is_error_and_text_unchanged() {
    let mut d = Document::new("m.slint".into(), SRC.into());
    assert!(matches!(d.apply(Op::SetBindings { path: p(&[9]), bindings: vec![] }), Err(EditError::ElementNotFound(_))));
    assert_eq!(d.text(), SRC);
}
```

- [ ] **Step 2: 失敗を確認** — Run: `cargo test -p slint-doc --test bindings` / Expected: コンパイルエラー
- [ ] **Step 3: 実装** — `ops.rs` に `struct TextEdit { start: usize, end: usize, text: String }` と `fn apply_edits(text: &str, edits: Vec<TextEdit>) -> String`(後ろから適用)を置き、各 Op はこれを生成する。既存 binding は `BindingExpression` の範囲を `"{expr};"` で置換(BindingExpression が `;` を含むことは spike で確認済み)、無ければ `LBrace` 直後に `" {name}: {expr};"` を挿入(複数なら指定順)、削除は直前の空白 1 区切りを含めて除く。`history.rs` は `Vec<String>` のスナップショットとカーソル、保存時点のカーソルを持つ
- [ ] **Step 4: 通過を確認** — Run: `cargo test -p slint-doc` / Expected: all passed
- [ ] **Step 5: コミット** — `git commit -m "Add binding edits and undo history to slint-doc"`

---

### Task 4: `slint-doc` の要素の追加・削除・移動

**Files:**
- Modify: `crates/slint-doc/src/ops.rs`, `src/lib.rs`, `src/syntax.rs`
- Test: `crates/slint-doc/tests/elements.rs`

**Interfaces:**
- Consumes: Task 3 の `Op`, `apply`
- Produces(`Op` に追加):
  - `InsertElement { parent: ElementPath, index: Option<usize>, source: String }`(`None` は末尾。`source` は 1 要素分のテキスト、例 `Button { text: "OK"; }`。戻り値は新要素の経路)
  - `RemoveElement { path: ElementPath }`(戻り値 `None`)
  - `MoveElement { path: ElementPath, new_parent: ElementPath, index: Option<usize> }`(`index` は移動元を取り除いた後の添字。新しい親がレイアウトなら `x` / `y` を除去する。戻り値は移動後の経路)
  - `syntax::child_indent(element) -> String`(既存の子の行頭インデント。子が無ければ要素自身のインデント + 4 空白)
- 挿入・移動するテキストは、移動先のインデントに合わせて各行のインデントを付け直し、1 要素 1 行以上で直前の兄弟の後ろに改行して置く
- 要素の直前の行コメント(`//` で始まり、要素との間に空行が無いもの)は要素と一緒に移動・削除する
- コンポーネントのルートは削除・移動できない(`InvalidTarget`)。自分の子孫への移動も `InvalidTarget`

- [ ] **Step 1: 失敗するテストを書く** — ソース `SRC2`:

```rust
const SRC2: &str = "export component Main inherits Window {
    box := Rectangle {
        // the stop button
        Text { x: 4px; y: 4px; text: \"停止\"; }

        Rectangle { x: 50px; y: 4px; width: 10px; height: 10px; }
    }
    VerticalLayout {
        Text { text: \"a\"; }
    }
}
";
```

テスト(それぞれ `d.text()` 全体を期待文字列と比較する):
  - `insert_at_end_of_container`: `InsertElement { parent: [0], index: None, source: "Button { }" }` → `Rectangle { x: 50px; ... }` の行の後ろに `        Button { }` の行が増え、戻り値が `[0, 2]`
  - `remove_takes_leading_comment`: `RemoveElement { path: [0, 0] }` → `// the stop button` の行と `Text { x: 4px; ...停止... }` の行が消え、空行と 2 つ目の Rectangle は残る(Review Focus 2)
  - `move_into_layout_strips_xy`: `MoveElement { path: [0, 1], new_parent: [1], index: Some(0) }` → VerticalLayout の先頭に `        Rectangle { width: 10px; height: 10px; }`、戻り値 `[1, 0]`
  - `move_keeps_id`: `MoveElement { path: [0], new_parent: [1], index: None }` → 移動後のテキストに `box := Rectangle {` が含まれ、`d.element(&[1, 1])` の `id` が `Some("box")`(Review Focus 3)
  - `cannot_move_into_descendant`: `MoveElement { path: [0], new_parent: [0, 1], .. }` → `Err(InvalidTarget(_))`、テキスト不変
- [ ] **Step 2: 失敗を確認** — Run: `cargo test -p slint-doc --test elements` / Expected: FAIL
- [ ] **Step 3: 実装**
- [ ] **Step 4: 通過を確認** — Run: `cargo test -p slint-doc` / Expected: all passed
- [ ] **Step 5: コミット** — `git commit -m "Add insert/remove/move element ops to slint-doc"`

---

### Task 5: `slint-doc` のレイアウトでの包み込みと解除

**Files:**
- Modify: `crates/slint-doc/src/ops.rs`, `src/lib.rs`
- Test: `crates/slint-doc/tests/layout.rs`

**Interfaces:**
- Consumes: Task 4 のインデント付け直しとコメント処理
- Produces(`Op` に追加、公開型も追加):
  - `pub struct LayoutSpec { pub kind: LayoutKind, pub order: Vec<usize>, pub bindings: Vec<(String, String)> }`(`kind` は Horizontal か Vertical のみ有効。`order` はコンテナの全ての子の添字の並べ替え)
  - `WrapInLayout { container: ElementPath, layout: LayoutSpec }`: コンテナの子をすべて `order` の順で新しいレイアウト要素(`HorizontalLayout` / `VerticalLayout`)へ移し、各子の `x` / `y` を除く。レイアウトは最初の子があった位置に挿入し、`bindings` を指定順に 1 行ずつ書く。子以外のメンバー(プロパティ、コールバック等)はコンテナに残る。戻り値はレイアウトの経路
  - 拒否条件(`InvalidTarget`): 子が 0、子に Repeated / Conditional がある、`order` が子の並べ替えになっていない、`kind` が Horizontal/Vertical 以外
  - `Unwrap { layout: ElementPath, positions: Vec<(i32, i32)> }`: `layout` は Layout 種別であること、親はレイアウトでないこと、`positions.len()` が子の数と一致すること。レイアウトの子を親の、レイアウトがあった位置へ順に戻し、各子に `x: {px}px; y: {py}px;` を設定し、レイアウト要素(とその binding)を除く。戻り値は親の経路

- [ ] **Step 1: 失敗するテストを書く** — ソース `SRC3`:

```rust
const SRC3: &str = "export component Main inherits Window {
    panel := Rectangle {
        background: #222;
        Text { x: 70px; y: 10px; text: \"右\"; }
        // left one
        Text { x: 10px; y: 12px; text: \"左\"; }
    }
}
";
```

  - `wrap_horizontal`: `WrapInLayout { container: [0], layout: LayoutSpec { kind: Horizontal, order: vec![1, 0], bindings: vec![("spacing".into(), "8px".into()), ("padding".into(), "12px".into())] } }` → テキストが次と完全一致し、戻り値 `[0, 0]`:

```text
export component Main inherits Window {
    panel := Rectangle {
        background: #222;
        HorizontalLayout {
            spacing: 8px;
            padding: 12px;
            // left one
            Text { text: "左"; }
            Text { text: "右"; }
        }
    }
}
```

  - `unwrap_restores_positions`: 上の結果に `Unwrap { layout: [0, 0], positions: vec![(12, 12), (60, 12)] }` → `background: #222;` が残り、`Text { x: 12px; y: 12px; text: "左"; }` と `Text { x: 60px; y: 12px; text: "右"; }` がこの順で `panel` の直下にあり、`HorizontalLayout` の文字列が無い
  - `wrap_rejects_repeated_children`: `for` を含むコンテナで `Err(InvalidTarget(_))`
  - `wrap_then_undo_is_one_step`: `apply` → `undo()` で `SRC3` に戻る
- [ ] **Step 2: 失敗を確認** — Run: `cargo test -p slint-doc --test layout` / Expected: FAIL
- [ ] **Step 3: 実装**
- [ ] **Step 4: 通過を確認** — Run: `cargo test -p slint-doc` / Expected: all passed
- [ ] **Step 5: コミット** — `git commit -m "Add wrap-in-layout and unwrap ops to slint-doc"`

---

### Task 6: `slint-doc` の部品化と宣言の追加

**Files:**
- Modify: `crates/slint-doc/src/ops.rs`, `src/lib.rs`
- Test: `crates/slint-doc/tests/component.rs`

**Interfaces:**
- Produces(`Op` に追加):
  - `ExtractComponent { path: ElementPath, name: String }`: 対象(ルート・Repeated・Conditional 以外)を `component {name} inherits {type_name} { ... }` として、対象を含むコンポーネント宣言の直前(空行 1 つを挟む)に挿入する。部品の本体は対象の中身から `x` / `y` / `width` / `height` を除いたもの。元の位置は `{id := }{name} { x..; y..; width..; height..; }`(元にあった幾何 binding だけを元の順で)に置き換える。戻り値は置き換えた要素の経路
  - `name` が識別子として不正(`[A-Za-z_][A-Za-z0-9_-]*` 以外)、または同名のコンポーネントが既にあれば `InvalidTarget`
  - `AddDeclaration { component: String, text: String }`: コンポーネントのルート要素の最後の宣言(`PropertyDeclaration` / `CallbackDeclaration`)の後ろ、無ければ `LBrace` の次の行に `text` を 1 行で挿入する。戻り値 `None`。`text` の構文チェックはしない(コンパイルエラーで知らせる)

- [ ] **Step 1: 失敗するテストを書く**
  - `extract_keeps_geometry_and_id`: `"export component Main inherits Window {\n    ok := Rectangle { x: 5px; width: 40px; background: red; Text { text: \"OK\"; } }\n}\n"` の `[0]` を `name: "OkButton"` で部品化 → テキストが `component OkButton inherits Rectangle {` で始まる宣言を `export component Main` の前に含み、その本体に `background: red;` と `Text { text: "OK"; }` があり `x:` / `width:` は無い。`Main` 内は `ok := OkButton { x: 5px; width: 40px; }`(Review Focus 3)
  - `extract_rejects_duplicate_name`: 既存の `Main` を名前にすると `Err(InvalidTarget(_))`
  - `add_declaration_after_last`: `component Chip inherits Rectangle {\n    in property <string> label;\n}` に `AddDeclaration { component: "Chip", text: "callback clicked();" }` → `in property <string> label;` の次の行が `    callback clicked();`
- [ ] **Step 2: 失敗を確認** — Run: `cargo test -p slint-doc --test component` / Expected: FAIL
- [ ] **Step 3: 実装**
- [ ] **Step 4: 通過を確認** — Run: `cargo test -p slint-doc` / Expected: all passed
- [ ] **Step 5: コミット** — `git commit -m "Add extract-component and declaration ops to slint-doc"`

---

### Task 7: `preview` クレート

**Files:**
- Create: `crates/preview/Cargo.toml`(依存: `slint`, `slint-interpreter`(features `std`, `compat-1-2`, `internal`, `internal-highlight`、default-features off), `i-slint-compiler`, `i-slint-core`, `spin_on`, `tidy`。dev: `i-slint-backend-testing`(feature `internal`))、`src/lib.rs`
- Test: `crates/preview/tests/preview.rs`

**Interfaces:**
- Consumes: `tidy::Rect`
- Produces:
  - `pub struct Diagnostic { pub message: String, pub offset: Option<u32> }`(`offset` は編集中テキスト内のバイト位置。ラッパー部分のエラーは `None`)
  - `pub fn compile(text: &str, path: &Path, component: &str) -> Result<Compiled, Vec<Diagnostic>>`
    - `text` の末尾に `\nexport component __PetariconPreviewRoot inherits Rectangle { {component} { width: 100%; height: 100%; } }\n` を足してコンパイルし、`__PetariconPreviewRoot` を表示対象にする(エクスポートされていない部品も表示するため)
  - `pub struct Compiled { pub compile_ms: f32, /* def */ }`
  - `impl Compiled { pub fn factory(&self, slot: Rc<RefCell<Option<LiveInstance>>>) -> slint::ComponentFactory; pub fn instantiate(&self, width: f32, height: f32) -> LiveInstance }`(`instantiate` はテスト用の単独インスタンス)
  - `#[derive(Clone)] pub struct LiveInstance`
  - `pub struct Hit { pub start: u32, pub rect: tidy::Rect, pub local: tidy::Rect }`(`rect` は表示領域の座標、`local` は親基準でソースの `x` / `y` に対応)
  - `impl LiveInstance { pub fn hits_at(&self, x: f32, y: f32) -> Vec<Hit>; pub fn geometry(&self, start: u32) -> Option<Hit> }`
    - `hits_at` は最前面の最も深い要素から順に、その祖先までを返す。編集中テキスト内(`start < text.len()`)の要素だけ。コンポーネントのルート要素(debug ノードの親が `Component`)は除く。spike の `hit_test` と同じ走査で、子は逆順に先に見る
    - `geometry` は `start` の要素の最初のインスタンス

- [ ] **Step 1: 失敗するテストを書く**(各テストの先頭で `i_slint_backend_testing::init_no_event_loop()` を呼ぶ)
  - `compiles_non_exported_component`: `component Card inherits Rectangle { Text { x: 10px; y: 20px; width: 50px; height: 10px; text: "x"; } }` を `"Card"` でコンパイル → `Ok`
  - `reports_error_offset`: `export component A { Text { text: 1 +; } }` → `Err`、先頭の `offset` が `Some(n)` で `n` が `+` の位置付近(`text.find("+;")` ± 3)
  - `hit_returns_deepest_first`: 上の `Card` を `instantiate(200., 100.)` → `hits_at(15., 25.)` の先頭の `start` が `Text` の `Element` 開始位置、`local == Rect { x: 10., y: 20., w: 50., h: 10. }`。`hits_at(150., 80.)` は空
- [ ] **Step 2: 失敗を確認** — Run: `cargo test -p preview` / Expected: FAIL
- [ ] **Step 3: 実装** — spike `spike/interpreter-editor/src/lib.rs` の `compile` / `install` / `hit_test` を参考に移植する。`Diagnostic` の位置は `slint_interpreter::Diagnostic::line_column()` から編集中テキストのバイト位置へ変換する
- [ ] **Step 4: 通過を確認** — Run: `cargo test -p preview` / Expected: 3 passed
- [ ] **Step 5: コミット** — `git commit -m "Add preview crate wrapping slint-interpreter"`

---

### Task 8: `storage` クレート

**Files:**
- Create: `crates/storage/Cargo.toml`, `src/lib.rs`
- Test: `src/lib.rs` の `mod tests`(`tempfile` を dev 依存に)

**Interfaces:**
- Produces:
  - `pub trait ProjectStore { fn projects(&self) -> io::Result<Vec<String>>; fn create_project(&self, name: &str) -> io::Result<()>; fn files(&self, project: &str) -> io::Result<Vec<String>>; fn read(&self, project: &str, file: &str) -> io::Result<String>; fn write(&self, project: &str, file: &str, text: &str) -> io::Result<()>; fn write_recovery(&self, project: &str, file: &str, text: &str) -> io::Result<()>; fn take_recovery(&self, project: &str, file: &str) -> io::Result<Option<String>>; fn clear_recovery(&self, project: &str, file: &str) -> io::Result<()>; }`
  - `pub struct LocalStore { root: PathBuf }`、`impl LocalStore { pub fn new(root: PathBuf) -> io::Result<Self> }`(root が無ければ作る)
  - レイアウト: `{root}/{project}/{file}`。`files` は `.slint` のみ、名前順。`write` は既存ファイルを `{file}.bak` に移してから一時ファイル経由で書く。復元用は `{root}/{project}/.recovery/{file}`。`write` 成功時に復元用ファイルを消す
  - `create_project` は `main.slint` に `export component Main inherits Window {\n    preferred-width: 360px;\n    preferred-height: 640px;\n}\n` を書く

- [ ] **Step 1: 失敗するテストを書く** — `create_lists_and_reads_main`、`write_keeps_one_bak`(2 回書いて `.bak` が 1 回目の内容)、`recovery_roundtrip`(`write_recovery` → `take_recovery` が `Some`、`write` 後は `None`)、`files_ignores_non_slint`
- [ ] **Step 2: 失敗を確認** — Run: `cargo test -p storage` / Expected: FAIL
- [ ] **Step 3: 実装**
- [ ] **Step 4: 通過を確認** — Run: `cargo test -p storage` / Expected: 4 passed
- [ ] **Step 5: コミット** — `git commit -m "Add storage crate with local project store"`

---

### Task 9: `app` の編集セッション(選択・移動・再構築)

**Files:**
- Create: `crates/app/Cargo.toml`(lib は `cdylib` + `lib`)、`crates/app/src/lib.rs`(当面 `pub mod session;` のみ)、`src/session.rs`
- Test: `crates/app/tests/session.rs`(testing バックエンド)

**Interfaces:**
- Consumes: `slint_doc::{Document, ElementPath, Op}`, `preview::{compile, Compiled, LiveInstance, Hit, Diagnostic}`
- Produces:
  - `#[derive(Debug, thiserror::Error)] pub enum SessionError { Edit(#[from] EditError), NoSelection, InLayout, NotAContainer, NotALayout, NoGeometry, Compile(Vec<Diagnostic>) }`
  - `pub struct Session`(フィールド: `doc: Document`, `component: String`, `live: Rc<RefCell<Option<LiveInstance>>>`, `selection: Option<ElementPath>`, `last_tap: Option<(f32, f32)>`, `pub grid: i32`, `diagnostics: Vec<Diagnostic>`)
  - `impl Session {`
    - `pub fn new(doc: Document) -> Self`(`component` は最初のエクスポートされたコンポーネント、無ければ最初のコンポーネント)
    - `pub fn doc(&self) -> &Document; pub fn component(&self) -> &str; pub fn set_component(&mut self, name: &str); pub fn selection(&self) -> Option<&ElementPath>; pub fn diagnostics(&self) -> &[Diagnostic]`
    - `pub fn rebuild(&mut self) -> Option<slint::ComponentFactory>`(成功なら新しい factory。失敗なら `diagnostics` を更新して `None`、`live` は前のまま)
    - `pub fn rebuild_standalone(&mut self, width: f32, height: f32) -> bool`(テスト用。`instantiate` して `live` に入れる)
    - `pub fn tap(&mut self, x: f32, y: f32) -> Option<ElementPath>`(`hits_at` の結果を `doc.path_at_offset` で経路にし、`path.component == component` のものだけ残す。前回のタップから 8px 以内でかつ現在の選択がその候補列にあれば、その次(親)を選ぶ。末尾まで行ったら先頭へ戻る)
    - `pub fn selection_hit(&self) -> Option<Hit>`
    - `pub fn move_selection(&mut self, dx: f32, dy: f32) -> Result<(), SessionError>`(親がレイアウトなら `InLayout`。新しい位置 = `local` + d をグリッドに丸め、`SetBindings x, y`)
    - `pub fn resize_selection(&mut self, corner: Corner, dx: f32, dy: f32) -> Result<(), SessionError>`(`pub enum Corner { TopLeft, TopRight, BottomLeft, BottomRight }`。動かした角の座標をグリッドに丸め、反対側の辺は固定して幅と高さを決める。幅と高さは 4px 未満にしない。レイアウト内では `width`/`height` だけを変える)
    - `pub fn apply(&mut self, op: Op) -> Result<(), SessionError>`(成功時、戻り値の経路があれば選択をそれにする)
    - `pub fn replace_text(&mut self, text: String); pub fn undo(&mut self) -> bool; pub fn redo(&mut self) -> bool`(undo/redo 後に選択の経路が解決できなければ選択を外す)
  - `}`
  - 選択の経路は、編集後も `ElementPath` のまま保持する(Op の戻り値で更新)

- [ ] **Step 1: 失敗するテストを書く**(ソース: `export component Main inherits Rectangle { width: 300px; height: 300px; a := Rectangle { x: 10px; y: 10px; width: 100px; height: 100px; b := Rectangle { x: 10px; y: 10px; width: 20px; height: 20px; } } VerticalLayout { y: 200px; height: 50px; Text { text: "in"; } } }` を整形して定数に)
  - `tap_selects_deepest_then_parent`: `tap(25., 25.)` → `[0, 0]`、同じ位置で再度 `tap` → `[0]`
  - `move_snaps_to_grid`: `[0]` を選んで `move_selection(13., 6.)` → テキストに `x: 24px; y: 16px;`
  - `move_in_layout_is_refused`: レイアウト内の Text を選んで `move_selection` → `Err(InLayout)`、テキスト不変
  - `resize_top_left_moves_origin`: `[0]` で `resize_selection(TopLeft, 8., 8.)` → `x: 20px; y: 20px; width: 90px; height: 90px;`(角 18 → 丸めて 20、右下 110 は固定)
  - `compile_error_keeps_previous_instance`(Review Focus 4): `replace_text` で壊れたテキストにし `rebuild_standalone` が `false`、`diagnostics()` が非空、その状態でも `tap(25., 25.)` が `Some` を返し、`move_selection` は成功してテキストを書き換える
- [ ] **Step 2: 失敗を確認** — Run: `cargo test -p app --test session` / Expected: FAIL
- [ ] **Step 3: 実装**
- [ ] **Step 4: 通過を確認** — Run: `cargo test -p app --test session` / Expected: 5 passed
- [ ] **Step 5: コミット** — `git commit -m "Add editing session with selection, move and resize"`

---

### Task 10: エディタ画面の骨組みと実機版 1(マイルストーン 1)

**Files:**
- Create: `crates/app/build.rs`(`SLINT_ENABLE_EXPERIMENTAL_FEATURES=1` を設定して `ui/editor.slint` をコンパイル)、`ui/editor.slint`, `ui/canvas.slint`, `ui/panels.slint`(この Task ではタブの枠だけ)、`src/glue.rs`, `src/crashlog.rs`, `src/main.rs`
- Modify: `crates/app/src/lib.rs`, `crates/app/Cargo.toml`(`[package.metadata.android]` を Global Constraints の値で)

**Interfaces:**
- Consumes: Task 8 の `LocalStore`, Task 9 の `Session`
- Produces:
  - `pub fn run(store: Box<dyn ProjectStore>, log_dir: PathBuf) -> Result<(), slint::PlatformError>`
  - `crashlog::install(log_dir: &Path)`(パニック時に `{log_dir}/crash-{unix秒}.log` へメッセージとバックトレースを書く)
  - `editor.slint` の `export component Editor inherits Window` の主なプロパティ・コールバック: `in property <component-factory> factory; in property <string> error-text; in property <bool> has-selection; in property <length> sel-x, sel-y, sel-w, sel-h; in property <[string]> projects, files, components; callback tapped(length, length); callback drag-finished(length, length); callback resize-finished(int /* Corner */, length, length); callback undo(); callback redo(); callback save(); callback open(string /* project */, string /* file */); callback choose-component(string); callback zoom(float /* 倍率の乗数 */);`
- 画面: スペック 5 章のとおり。ツールバーの整形・解除・部品化ボタンはこの Task では無効表示。上下の余白は `root.safe-area-insets` を使う
- ドラッグ中は枠だけを動かし、指を離したときに `Session::move_selection` → `rebuild` → `factory` を差し替える(spike と同じ)
- キャンバスのスクロール: 何もない場所のドラッグで表示位置を動かす(選択中の要素の上から始めたドラッグは移動)
- 横長のウィンドウ(幅 > 高さ)では下部パネルを右側に置く
- ファイルメニュー: プロジェクトの選択・新規作成(`create_project`)、ファイルの選択、編集するコンポーネントの選択
- キャンバスの拡大縮小: ±ボタンと `ScaleRotateGestureHandler`(2 本指)の両方。拡大率はデザイン表示の `transform-scale` で表し、タップ座標は拡大率で割ってから `Session::tap` に渡す
- 保存: ツールバーの保存、Android の `MainEvent::Pause`(`slint::android::init_with_event_listener` で受ける)、PC のウィンドウを閉じるとき。保存後 `doc.mark_saved()`
- 起動時: Android は `external_data_path()/projects` を `LocalStore` に、ログは `external_data_path()/logs`。プロジェクトが 1 つも無ければ `default` を作る

- [ ] **Step 1: PC で起動確認** — Run: `cargo run -p app -- ./projects`(Xvfb 上で `xvfb-run`)/ Expected: 既定プロジェクトの `Main` が表示される。スクリプトで要素をタップすると選択枠が出て、ドラッグ後にファイル(`./projects/default/main.slint`)の `x` / `y` が変わる。spike と同じ手順でスクリーンショットを撮って確認する
- [ ] **Step 2: すべてのテストを実行** — Run: `cargo test --workspace` / Expected: all passed
- [ ] **Step 3: APK をビルド** — Run: `cd crates/app && cargo apk build --release --lib --target aarch64-linux-android` / Expected: `target/release/apk/app.apk` ができる(名前は cargo-apk の出力に従う)
- [ ] **Step 4: コミット** — `git commit -m "Add editor shell with canvas, selection, move and save"`
- [ ] **Step 5: 作者に APK を渡し、実機確認を依頼する** — 確認項目: 起動、保存先に `main.slint` が見えること(USB 接続)、タップ選択と親への切り替え、ドラッグ移動、サイズ変更、2 本指の拡大縮小、アプリ切り替え後の自動保存、ステータスバー・ナビゲーションバーと重ならないこと。問題があればここで直す

---

### Task 11: ツリーと部品のタブ

**Files:**
- Modify: `crates/app/ui/panels.slint`, `src/glue.rs`
- Test: `crates/app/tests/session.rs` に追加

**Interfaces:**
- Consumes: `Document::tree`, `Document::components`, `Op::{InsertElement, RemoveElement, MoveElement}`
- Produces:
  - `Session::tree_rows(&self) -> Vec<TreeRow>`(`pub struct TreeRow { pub path: ElementPath, pub depth: usize, pub label: String /* "id := Type" または "Type" */, pub is_layout: bool }`、深さ優先の表示順)
  - `Session::palette(&self) -> Vec<String>`(標準部品 `["Rectangle", "Text", "Image", "TouchArea", "HorizontalLayout", "VerticalLayout", "Button", "CheckBox", "Slider", "SpinBox", "LineEdit", "ComboBox", "Switch"]` に続けて、編集中以外のコンポーネント名)
  - `Session::add_from_palette(&mut self, name: &str) -> Result<(), SessionError>`(スペック 2.1 の 4 番: 選択がコンテナならその末尾、そうでなければ親の末尾。自由配置の親なら、`x`/`y` を既存の子の矩形と重ならない最初の位置(8px から 8px 刻みで右、次に下へ探索)にする。標準部品のうち std-widgets のもの(`Button`, `CheckBox`, `Slider`, `SpinBox`, `LineEdit`, `ComboBox`, `Switch`)はファイル先頭の `import { ... } from "std-widgets.slint";` に名前を足す(無ければ import 行を作る))
  - ツリーの各行の操作: 上へ / 下へ(同じ親の中での並べ替え)、外へ(親の親の、親の直後へ)、中へ(直前の兄弟がコンテナならその末尾へ)、削除、新しいコンテナ(`Rectangle { }` を選択要素の親に追加)
- [ ] **Step 1: 失敗するテストを書く** — `palette_adds_into_free_container_without_overlap`(子 `Rectangle { x: 8px; y: 8px; width: 40px; height: 40px; }` がある自由配置コンテナに `Button` を追加 → 新要素の `x` が `56px`、`y` が `8px`、ファイル先頭に `import { Button } from "std-widgets.slint";`)、`palette_adds_std_widget_to_existing_import`、`tree_rows_order_and_labels`
- [ ] **Step 2: 失敗を確認** — Run: `cargo test -p app --test session` / Expected: FAIL
- [ ] **Step 3: 実装**(テストを通す Session 側と、panels.slint のタブ UI)
- [ ] **Step 4: 通過を確認** — Run: `cargo test --workspace` / Expected: all passed。PC で起動し、ツリー操作と部品追加をスクリーンショットで確認
- [ ] **Step 5: コミット** — `git commit -m "Add tree and palette panels"`

---

### Task 12: プロパティとソースのタブ、エラー帯

**Files:**
- Modify: `crates/app/ui/panels.slint`, `ui/editor.slint`, `src/glue.rs`, `src/session.rs`

**Interfaces:**
- Produces:
  - `Session::properties(&self) -> Vec<PropRow>`(`pub struct PropRow { pub name: String, pub expr: String, pub kind: PropKind }`、`pub enum PropKind { Length, Color, Text, Other }`。先頭に `x, y, width, height`、続いて種類に応じた `spacing, padding, padding-left, padding-right, padding-top, padding-bottom, alignment, cross-axis-alignment`(レイアウト)、`text, color, font-size`(Text)、`background, border-radius, border-width, border-color`(Rectangle)、最後に書かれている他の binding。値が無いものは空文字)
  - `Session::set_property(&mut self, name: &str, value: &str) -> Result<(), SessionError>`(`value` が空なら削除。`Length` で数字だけなら `px` を付ける。`Text` で `"` で始まらなければ文字列リテラルにする)
  - ソースタブ: 選択要素(無ければコンポーネント全体)のテキスト範囲を表示・編集し、確定で `replace_text`(範囲だけ差し替えた全体テキスト)
  - エラー帯: `diagnostics()[0].message` を表示。タップで、ソースタブを開き `offset` の位置を含むコンポーネント全体を表示する
  - 数値入力は 300ms 入力が止まってから `set_property` → `rebuild`(連続入力をまとめる)
- [ ] **Step 1: 失敗するテストを書く** — `properties_put_geometry_first`、`set_length_adds_px`(`"12"` → `x: 12px;`)、`set_text_quotes`(`"停止"` → `text: "停止";`)、`empty_value_removes`
- [ ] **Step 2: 失敗を確認** — Run: `cargo test -p app --test session` / Expected: FAIL
- [ ] **Step 3: 実装**
- [ ] **Step 4: 通過を確認** — Run: `cargo test --workspace` / Expected: all passed。PC で起動し、プロパティ編集・ソース編集・エラー帯からの移動をスクリーンショットで確認
- [ ] **Step 5: コミット** — `git commit -m "Add properties and source panels with error band"`

---

### Task 13: 整形・解除とガイド表示(マイルストーン 2)

**Files:**
- Create: `crates/app/src/layout_bridge.rs`, `src/guides.rs`
- Modify: `crates/app/src/session.rs`, `src/glue.rs`, `ui/canvas.slint`, `ui/editor.slint`
- Test: `crates/app/tests/session.rs` に追加、`src/guides.rs` の `mod tests`

**Interfaces:**
- Consumes: `tidy::plan`, `Op::{WrapInLayout, Unwrap}`, `LiveInstance::geometry`
- Produces:
  - `layout_bridge::layout_spec(plan: &tidy::Plan) -> slint_doc::LayoutSpec`(binding の順: `spacing`、padding(4 辺同値なら `padding`、そうでなければ `padding-left`, `padding-right`, `padding-top`, `padding-bottom` のうち 0 でないもの)、`alignment`(`start` / `center` / `end`、Start は省略しない)、`cross-axis-alignment`(`stretch` / `start` / `center` / `end`))。値は `{n}px` とキーワード
  - `Session::tidy_selection(&mut self) -> Result<Vec<tidy::Warning>, SessionError>`(選択がコンテナでなければ `NotAContainer`。子の `local` 矩形を `geometry` で集め `tidy::plan(container.w, container.h, .., grid)`。`None`(子 0)なら `NotAContainer` を返す)
  - `Session::untidy_selection(&mut self) -> Result<(), SessionError>`(選択がレイアウト、またはレイアウトを子に 1 つだけ持つコンテナなら、そのレイアウトについて、各子の `rect` からレイアウトの親の `rect` の原点を引いてグリッドに丸めた位置で `Unwrap`。それ以外は `NotALayout`)
  - `guides::compute(layout: &Hit, kind: LayoutKind, children: &[Hit], bindings: &[Binding]) -> Guides`
  - `pub struct Guides { pub outline: tidy::Rect, pub direction: Direction, pub padding: Vec<GuideLabel>, pub gaps: Vec<GuideLabel>, pub align_line: Option<(f32, f32, f32, f32)> }`、`pub struct GuideLabel { pub area: tidy::Rect, pub value: i32, pub property: String /* 例 "padding-left", "spacing" */ }`
  - `Session::guides(&self) -> Option<Guides>`(選択がレイアウト、またはレイアウトを子に持つコンテナのとき)
  - canvas.slint: 外枠と揃え線は 6px の線分と 4px の間隔の繰り返しで描く。padding 領域は半透明の塗り、gap は斜線の代わりに半透明の塗りと数値。数値をタップすると数値入力を出し、確定で `set_property(property, value)`
  - ツールバーの整形・解除ボタンを有効にする。警告はトースト(画面下部に 3 秒)で表示: AmbiguousDirection → 「先に階層を分けると、きれいに整形できます」、Overlap → 「重なっている要素があります」
- [ ] **Step 1: 失敗するテストを書く**
  - `tidy_wraps_and_selects_layout`: 子 2 つの自由配置コンテナを選んで `tidy_selection` → テキストに `HorizontalLayout {` と `spacing:`、子の `x:` が消え、選択がレイアウトの経路
  - `tidy_single_child_is_vertical`(Review Focus 5): 子 1 つ → `VerticalLayout {`、警告なし
  - `tidy_empty_container_is_refused`(Review Focus 5): 子 0 → `Err(NotAContainer)`、テキスト不変
  - `untidy_restores_visual_positions`: 整形直後に `rebuild_standalone` → `untidy_selection` → 子の `x` / `y` が整形後の見た目の位置(グリッド丸め)
  - `guides_report_spacing_and_padding`(guides.rs): 既知の矩形から `gaps[0].value == 8`、`padding` に `padding-left` 12 を含む
- [ ] **Step 2: 失敗を確認** — Run: `cargo test --workspace` / Expected: FAIL
- [ ] **Step 3: 実装**
- [ ] **Step 4: 通過を確認** — Run: `cargo test --workspace` / Expected: all passed。PC で、ラフ配置 → 整形 → ガイドの数値をタップして変更 → 解除、をスクリーンショットで確認
- [ ] **Step 5: コミットして APK を渡す** — `git commit -m "Add tidy, untidy and layout guides"`。APK をビルドし、実機確認を依頼する(整形結果、ガイドの見え方、数値タップでの調整、解除)

---

### Task 14: 部品化と復元

**Files:**
- Modify: `crates/app/src/session.rs`, `src/glue.rs`, `src/lib.rs`, `ui/editor.slint`, `ui/panels.slint`

**Interfaces:**
- Consumes: `Op::{ExtractComponent, AddDeclaration}`, `ProjectStore::{write_recovery, take_recovery, clear_recovery}`
- Produces:
  - `Session::extract_selection(&mut self, name: &str) -> Result<(), SessionError>`
  - 部品化ダイアログ: 名前入力 → 実行。続けて「プロパティ/コールバックを追加」で、種類(`in property` / `in-out property` / `out property` / `callback`)、型(`string`, `int`, `float`, `bool`, `length`, `color`, `brush`, `image` から選択。callback は引数なし)、名前を選び `AddDeclaration { component: 新しい部品名, text }`。text は `in property <string> label;` / `callback clicked();` の形
  - 部品化後はコンポーネント選択をその部品に切り替えられる(`choose-component`)
  - 復元: 編集中、最後の変更から 5 秒ごとに未保存なら `write_recovery`。起動時やファイルを開くときに `take_recovery` が `Some` なら「保存されていない変更があります。復元しますか?」を出し、はいなら `replace_text`、いいえなら `clear_recovery`
- [ ] **Step 1: 失敗するテストを書く** — `extract_selection_switches_selection_to_usage`(部品化後の選択が置き換えた要素の経路、`doc().components()` に新しい名前)
- [ ] **Step 2: 失敗を確認** — Run: `cargo test -p app --test session` / Expected: FAIL
- [ ] **Step 3: 実装**
- [ ] **Step 4: 通過を確認** — Run: `cargo test --workspace` / Expected: all passed。PC で部品化と宣言追加、復元ダイアログ(プロセスを kill して再起動)をスクリーンショットで確認
- [ ] **Step 5: コミット** — `git commit -m "Add extract-component dialog and crash recovery"`

---

### Task 15: 仕上げと最終の実機確認

**Files:**
- Create: `crates/app/README.md`(ビルド方法、保存先、USB での受け渡し方、既知の制約: 実験的機能と非公開 API、Slint の固定バージョン)
- Modify: 必要に応じて全体

- [ ] **Step 1: 全テストと警告の確認** — Run: `cargo test --workspace && cargo clippy --workspace -- -D warnings` / Expected: すべて成功
- [ ] **Step 2: スペックとの突き合わせ** — スペック 2.1 の 10 項目、5 章の画面要素、6 章のデータ保護それぞれについて、動作を PC のスクリーンショットで確認する
- [ ] **Step 3: APK をビルドして渡す** — 実機での通し確認を依頼する: 新規プロジェクト → 部品を追加 → ラフに配置 → 整形 → ガイドで数値調整 → 部品化 → 保存 → USB で PC から `.slint` が見えること
- [ ] **Step 4: コミット** — `git commit -m "Add editor README and finish first version"`
