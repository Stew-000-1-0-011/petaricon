# Spike: Slint + wgpu point cloud on Android (throwaway)

使い捨ての検証用コードです。製品コードではありません。

確かめること: Slint の Android アプリの中に wgpu で描いた 3D 点群を埋め込み、実用的な速度で動かせるか。

- 上半分: wgpu でオフスクリーンテクスチャに点群を描き、Slint の `Image` として表示
- 下半分: Slint のボタン(点数 100k / 300k / 1M)とスライダー(回転速度・ズーム)
- 点群部分はドラッグで視点回転
- 左上: FPS、点数、描画解像度、1 フレームあたりの CPU 時間

## 構成上のポイント

- Slint 1.18.1 の `unstable-wgpu-30` を有効にすると、Android バックエンドは Skia の wgpu(Vulkan)レンダラを使う
- `set_rendering_notifier` の `RenderingSetup` で Slint の `wgpu::Device` / `Queue` を受け取り、同じデバイスで点群を描く
- 描いたテクスチャは `slint::Image::try_from(wgpu::Texture)` でコピーなしに UI へ渡す
- Slint は wgpu の GL バックエンドを避けるので、端末に Vulkan が必要

## ビルド

```sh
rustup target add aarch64-linux-android
cargo install cargo-apk
export ANDROID_HOME=/path/to/android-sdk            # platforms;android-35, build-tools;35.0.0
export ANDROID_NDK_ROOT=$ANDROID_HOME/ndk/<version>  # NDK 29 で確認
cargo apk build --release --lib --target aarch64-linux-android
# => target/release/apk/pointcloud_spike.apk
```

release ビルドも `~/.android/debug.keystore`(パスワード `android`)で署名する設定にしている。

PC で動かす場合は `cargo run`(winit + femtovg-wgpu)。
