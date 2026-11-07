# winit (vendored)

[winit](https://github.com/rust-windowing/winit) 0.30.13 from crates.io (Apache-2.0, `LICENSE` beside this file),
patched in through `[patch.crates-io]` in the root `Cargo.toml`. Its examples, tests and dev-dependencies are left out;
`Cargo.toml` says so.

What differs from upstream, all in `src/platform_impl/android/mod.rs` and marked `Honeydew (vendored change)` or
`(vendored addition)`: winit 0.30's Android backend sends every pointer as a touch and drops a mouse's wheel, hover and
buttons (its `TODO mouse events`). With a mouse it now sends `MouseWheel`, `CursorMoved`, `CursorLeft` and the right,
middle, back and forward buttons as `MouseInput`, as the desktop does; the main button stays a touch, while it is down
(issue #15: scrolling, zooming and context menus with a mouse on Android). The iOS backend also supports the single-scene lifecycle selected by Honeydew's
`UIApplicationSceneManifest`: `src/platform_impl/ios/scene.rs` registers `WinitSceneDelegate`, attaches windows
created before scene connection, and leaves apps without a scene manifest on the legacy path. The small hooks are
in `ios/event_loop.rs`, `ios/window.rs` and `ios/mod.rs`, with the needed UIKit features in `Cargo.toml`. This is
required to launch builds made with SDK 27 on iPadOS/iOS 27. Other platforms build upstream's code.

To update: copy the new release's `src`, `build.rs` and `LICENSE` here, its `Cargo.toml` without the example and test
targets and the dev-dependencies, and carry the marked change over.
