# nih-plug (vendored)

The core crate of [nih-plug](https://github.com/robbert-vdh/nih-plug) by Robbert van der Helm (ISC, `LICENSE` beside
this file) and its derive macros, at commit `de421011f41a6d10fc8c7a6084e4f4dee0143683`, the revision Lemon Studio
builds its plugins with. Honeydew builds Lemon Studio's plugins into the app (`vendor/lemon-studio`) through it, on every
platform, the browser too.

What differs from upstream:

- Left out: the VST3 and standalone wrappers (`src/wrapper/vst3*` but `subcategories.rs`, `src/wrapper/standalone*`),
  the optional features and their dependencies, the dev-dependencies, the workspace's other crates, examples and
  profiles. `Cargo.toml` says so. The VST3 metadata (`Vst3Plugin`, `src/plugin/vst3.rs`, and `Vst3SubCategory`,
  upstream's files) is kept, without the wrapper, so that a Lemon plugin's `impl Vst3Plugin` builds as it is.
- A browser build (wasm32, no threads) runs background tasks at once on the calling thread instead of on a worker thread
  (`src/event_loop/background_thread.rs`), and takes editors embedded with Lemon Studio's `lemon-web` window API
  (`src/wrapper/clap/wrapper.rs`). On Windows an editor's Win32 window is passed as an X11 window id too: Honeydew
  embeds every editor built on this copy, and the window is the editor's id there (`os_plugkit::embed`). Each change is
  marked `Honeydew (vendored change)` or `(vendored addition)`; other platforms build upstream's code.
