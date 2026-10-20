# Spike: slint-interpreter as a touch editor on Android (throwaway)

使い捨ての検証用コードです。製品コードではありません。

確かめること:

1. slint-interpreter が Android で動き、実行時にコンパイルした `.slint` をエディタ画面に埋め込めるか
2. タップ位置から要素を特定し、位置とサイズを取れるか
3. ドラッグで `x`/`y` を書き換え(構文木上の最小差分)、再コンパイルまでが実用的な速度か

## 画面

- 上部: コンパイル時間、インスタンス化時間、ヒットテスト時間、編集時間
- 中央: `design/design.slint` を interpreter で表示。タップで選択(マゼンタの枠)、ドラッグで移動
- レイアウト内の要素は移動不可として扱う
- 下部: 「Recompile x10」(コンパイル時間の計測)、「Reset design」

## わかったこと(PC, release)

- コンパイル 40〜55 ms、インスタンス化 0.5 ms 未満、ヒットテスト 0.3 ms 未満、ソース編集 0.1 ms
- `element_positions` の座標は、埋め込んだコンポーネントのルート基準
- `ComponentContainer` / `component-factory` は Slint 1.18 では実験的機能(`SLINT_ENABLE_EXPERIMENTAL_FEATURES`)
- 位置取得 API は `internal-highlight` feature の非公開 API

## ビルド

`spike/slint-wgpu-android/README.md` と同じ手順で、`cargo apk build --release --lib --target aarch64-linux-android`。
