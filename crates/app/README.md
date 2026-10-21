# Petaricon Editor

スマホ単体でも使える `.slint` のノーコードエディタです。Rust と Slint で書かれていて、Android と PC の両方で動きます。キャンバスでは `slint-interpreter` が本物の描画を行い、編集結果は人が読んで手直しできる `.slint` テキストとして保存されます。知らない記述は変更せずそのまま残します。

設計は `docs/superpowers/specs/2026-10-20-slint-touch-editor-design.md` を参照してください。

## 基本的な使い方

1. プロジェクトを作る(または開く)。
2. 「Parts」タブから部品を追加し、キャンバスの上でラフに配置する(ドラッグで移動、四隅のハンドルでサイズ変更)。部品は選択中のコンテナ(何も選んでいなければ画面全体)に追加され、追加後もそのコンテナが選択されたままなので、続けて押すと兄弟として並びます。追加した部品を動かすときは、その部品をタップして選びます。
3. コンテナを選んで **Stack** を押す。子要素が `HorizontalLayout` / `VerticalLayout` に整形される。
4. レイアウトのガイドに出る数値(spacing、padding)をタップして、数値で調整する。
5. 繰り返したい部分を選んで **Extract** を押し、`component` に切り出す。必要なら `in property` と `callback` も宣言する。
6. **Save** で保存する。

整形を取り消したいときは **Unstack**(現在の見た目の位置で自由配置に戻す)か **Undo** を使います。

## プロジェクトとファイルの管理

左上のメニューボタン(横線 3 本のアイコン)から操作します(ボタンはどれも 44px 以上)。

- **Projects**: 一覧から選んで切り替え、名前を入力して **Create** で新規作成、**Rename** で選択中のプロジェクトの名前を変更します。
- **Files**: **New file** で新しい `.slint` を作ってそのまま開きます(`.slint` は省略できます)。**Rename** は開いているファイルの名前を変更します。
- **Save as...**: 今のエディタの内容を、名前と保存先プロジェクト(初期値は現在のプロジェクト)を指定して新しいファイルに書き、そのファイルに切り替えます。元のファイルは、未保存の変更があっても最後に保存した内容のまま変わりません。
- 同じ名前が既にあるときや使えない名前(空、`/`、`\`、`..`、先頭が `.`)はダイアログにエラーを出して、何も変更しません。上書きはしません。
- 開いているファイル/プロジェクトの名前変更では、先に保存します(保存に失敗したら名前変更は中止)。名前変更後も同じ文書のまま編集を続けられ、`.bak` と復元用コピーも新しい名前に移ります。

## ソフトキーボードと入力バー

Android のソフトキーボードが入力欄を隠すとき、キーボードのすぐ上に入力バー(項目名・入力欄・OK、高さ 52px / 部品は 44px)を出します。画面全体は動きません。

- タッチ操作(Android。デバッグビルドでは `PETARICON_FAKE_KEYBOARD` 指定時も同じ動作): Props の値欄とガイド数値は文字入力欄ではなく、タップするとすぐ入力バーが開き、バーの入力欄が最初で唯一のフォーカス先になります(Android は入力欄から入力欄へフォーカスを移すたびにキーボードを隠して出し直すため、元の欄にフォーカスしてからバーへ移すとキーボードが消えてしまうのを避けるためです)。キーボードが出ていればその上、まだなら画面下端(下の安全領域の上)にバーを置き、キーボードが出ると追従します。キーボードが 300 ms 以上隠れ続けたときだけバーを閉じ(未反映の入力は先に反映)、それより短い一時的な隠れは無視します。バーを開いてから 500 ms 以内の隠れでは閉じません。キーボードに隠れない位置でも、タッチ操作では常にバー経由で入力します。
- 対象: Props タブの値欄と、キャンバス上のガイド数値の入力(スマホなどのタッチ端末では常に、PC ではキーボードに隠れる位置にあるときだけ)。バーの入力欄に自動でフォーカスが移り、元の欄と同じ文字から始まります。
- 入力は元の欄と同じ経路に流れます(Props は 300 ms のデバウンス後に反映、OK / Enter は即反映)。OK / Enter でバーは閉じます。キーボードを閉じる(戻る)とバーも閉じ、未反映の入力(ガイド数値も)はその時点で反映します。選択の変更・Undo/Redo・タブ切り替えなどで対象が変わるとバーは閉じ、未反映の入力は元の対象に反映されます(別の要素には適用されません)。バーの外をタップしても閉じません(OK かキーボードを閉じるまで)。
- Source タブ: キーボードが出ている間、Source の編集欄(最小 60px、約 3 行)だけがキーボードの上に収まるよう縮み、Apply はタイトルの横に移ります(キャンバスとツールバーは動きません)。ただしパネルが低くキーボードの上に残る高さが約 116px 未満のときは収まりきりません。その場合は先にパネルのハンドルを上にドラッグして高くしてください(自動では広げません)。
- ファイル名・Extract のダイアログは、キーボードに隠れる場合にダイアログごとキーボードの上へ持ち上がります(Save as の一覧は縮みます)。
- PC(キーボードなし)では何も変わりません。

デバッグ用(デバッグビルドのみ。リリースビルド/APK では無効): 環境変数 `PETARICON_FAKE_KEYBOARD=<高さ(論理px)>` を付けて起動すると、その高さのキーボードが出ているものとして動きます(灰色の帯を描画)。

```sh
PETARICON_FAKE_KEYBOARD=300 cargo run -p app -- <プロジェクトのフォルダ>
```

## PC で動かす

```sh
cargo run -p app -- <プロジェクトのフォルダ>
```

フォルダを省略すると `./projects` を使います。ログは `./logs/` に出ます。

## APK のビルド

必要なもの: Rust のターゲット `aarch64-linux-android`、`cargo-apk`、Android SDK と NDK。

```sh
export ANDROID_HOME=<Android SDK のパス>
export ANDROID_NDK_ROOT=<NDK のパス>
cd crates/app
cargo apk build --release --lib --target aarch64-linux-android
```

出力は `target/release/apk/app.apk` です。release ビルドは `~/.android/debug.keystore`(パスワード `android`)で署名します(`crates/app/Cargo.toml` の `package.metadata.android.signing.release`)。署名のパスは `Cargo.toml` に絶対パス(`/root/.android/debug.keystore`)で書かれているので、別の環境でビルドするときは自分のキーストアのパスに書き換えてください。キーストアがなければ次で作ります。

```sh
keytool -genkey -v -keystore ~/.android/debug.keystore -storepass android -alias androiddebugkey \
  -keypass android -keyalg RSA -keysize 2048 -validity 10000 -dname "CN=Android Debug,O=Android,C=US"
```

パッケージ名は `dev.petaricon.editor`、表示名は `Petaricon Editor`、`min_sdk_version` は 26、`target_sdk_version` は 35 です。

## 実機へのインストール

USB デバッグを有効にして接続し、次を実行します。

```sh
adb install -r target/release/apk/app.apk
```

APK ファイルをスマホへコピーして、ファイルアプリから開いてもかまいません(提供元不明のアプリの許可が必要です)。

## 保存先と USB での受け渡し

- Android: `Android/data/dev.petaricon.editor/files/projects/<プロジェクト名>/*.slint`。権限は不要です。PC と USB 接続して(ファイル転送モード)、この場所を開くと `.slint` を読み書きできます。
- PC: 起動引数で指定したフォルダ(省略時 `./projects`)。

保存のたびに直前の版を `<ファイル名>.slint.bak` として 1 世代残します。未保存の内容は数秒ごとに `<プロジェクト>/.recovery/<ファイル名>` へ書き出され、次回起動時に復元するか確認されます。アプリがバックグラウンドに回ったときにも自動で保存します。起動時は、復元用コピーが残っているファイルを(どのプロジェクトでも)最初に開き、なければ最後に開いていたファイルを開きます(保存先フォルダの `.last-open`)。ウィンドウを閉じるときに保存できなかった場合は、ウィンドウを閉じずにエラーを表示し、復元用コピーを書き出します。

## ログ

Rust のパニックは `crash-<unix 秒>.log` としてログフォルダに記録されます。

- Android: `Android/data/dev.petaricon.editor/files/logs/`
- PC: `./logs/`(起動したカレントディレクトリ)

## 既知の制約

- `ComponentContainer` / `component-factory` は Slint 1.18 では実験的機能です。`app` の `.slint` が使うため、`crates/app/build.rs` で `SLINT_ENABLE_EXPERIMENTAL_FEATURES` を設定しています。
- 要素の位置の取得に `slint-interpreter` の非公開 API(`internal` / `internal-highlight`)を使っています。使ってよいのは `preview` クレートだけです。
- Slint 系クレートは `=1.18.1` に完全一致で固定しています。更新は意図して行い、`preview` のテストで確認してください。
- 作らないもの: GitHub 連携、`for` / `if` / アニメーション / `states` の GUI 編集(テキストでは書けます)、GridLayout への整形、複数選択。子に `for` / `if` を含むコンテナの整形は断られます。
- ライセンス: `slint-interpreter` と `i-slint-compiler` は GPLv3 または Slint ロイヤリティフリーライセンスです。個人利用向けです。配布・販売する場合は事前に確認してください。

## クレート構成

| クレート | 役割 |
|---|---|
| `slint-doc` | `.slint` テキストの編集操作と履歴(Slint の UI に依存しない) |
| `tidy` | 整形の幾何計算(Slint に依存しない) |
| `preview` | interpreter の包み。コンパイル、埋め込み、ヒットテスト |
| `storage` | 保存先(ローカルファイル) |
| `app` | エディタ画面と、Android / PC の起動部分 |
