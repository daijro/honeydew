# Lemon Studio's plugin hosts (vendored)

Honeydew hosts VST3, VST2 and CLAP plugins on the desktop with Lemon Studio's plugin hosting crates, taken from
Lemon Studio (https://github.com/ItsAlphaNeon/Lemon-Studio, GPL-3.0-or-later, `LICENSE` beside this file) at commit
`219ed4ee8bfb534b91282e3ef45ded438292eee8` (2026-10-18), `crates/os-host`, `crates/os-clap`, `crates/os-vst3` and
`crates/os-vst2`. Every file keeps its copyright and SPDX header. `os-vst2/LICENSE-vst2sdk.txt` is upstream's notice for
its clean-room VST 2 interface.

They are path dependencies of the desktop build only (the root `Cargo.toml`), not workspace members: their own lints and
tests stay theirs. The tests need Lemon's engine and plugins, so their dev-dependencies are not carried; run them in
Lemon Studio.

## What differs from upstream

To pull upstream fixes, diff each crate against the commit above; these are the only changes.

- `os-engine` is not upstream's engine: it holds only what the hosts use from it, `instrument.rs` (the `Instrument` and
  `HostedEffect` traits, `TransportInfo`, `NoteLevels`, `ChannelLevels`) and `voices.rs` (without its tests).
  `ChannelVoicing` is an empty struct: only Lemon's `os-native` (not carried) reads it, and upstream's needs `os-dsp`.
- `Instrument::midi(offset, [u8; 3])` is new (default: ignored), for pitch bend, channel pressure and MIDI CCs, which
  Honeydew's channels send to their instruments. `os-host`'s `Guarded` forwards it; `os-clap` sends a CLAP MIDI event,
  `os-vst2` a MIDI event (`NoteKind::Raw`), and `os-vst3` the parameter the controller's `IMidiMapping` names, as
  automation. Each change is marked `Honeydew (vendored addition)`.
- `os-host/src/runloop.rs`: `remove` and `modify_fd` do nothing (and return `false`) once the thread's run loop is
  gone. Honeydew keeps its plugin instances in a thread-local made before the run loop's, so at exit they are destroyed
  after it, and a plugin that unregisters a timer or descriptor then (Surge XT does) aborted the app. Marked
  `Honeydew (vendored change)`.
- `os-vst3/src/host.rs` also allows `unknown_lints`: Honeydew's CI pins Rust 1.91, which does not know the lint
  upstream allows there.
- The manifests name their version, edition and license instead of Lemon's workspace keys, point at each other by
  path, drop the dev-dependencies and `os-clap`'s examples, and pin `vst3-sys` to the commit Lemon's `Cargo.lock` has
  for its branch.
- Comments that cited Lemon Studio's design documents (`spec: …`, `§…`) drop the citation; the rest of each comment
  and all code are unchanged.

## Not carried

- `os-native` (FL Studio desktop's own plugin ABI, Windows DLLs) and `os-plugkit` (the editors of Lemon's own plugins,
  on nih-plug): neither hosts VST3, VST2 or CLAP plugins.
- The app side (`os-app`): Honeydew's own code ports what it needs, with Lemon's headers kept: the plugin search and
  scanner (`src/plugins/scan.rs`, from `plugin_db.rs` and `plugin_scan.rs`) and the X11 editor window
  (`src/plugins/window.rs`, from `plugin_window/x11.rs`).
