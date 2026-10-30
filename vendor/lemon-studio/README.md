# Lemon Studio's plugin hosts, effects and plugins (vendored)

Honeydew hosts VST3, VST2 and CLAP plugins on the desktop with Lemon Studio's plugin hosting crates, taken from
Lemon Studio (https://github.com/ItsAlphaNeon/Lemon-Studio, GPL-3.0-or-later, `LICENSE` beside this file) at commit
`219ed4ee8bfb534b91282e3ef45ded438292eee8` (2026-10-18), `crates/os-host`, `crates/os-clap`, `crates/os-vst3` and
`crates/os-vst2`. Every file keeps its copyright and SPDX header. `os-vst2/LICENSE-vst2sdk.txt` is upstream's notice for
its clean-room VST 2 interface.

They are path dependencies (the root `Cargo.toml`), not workspace members: their own lints and tests stay theirs.
`os-vst3` and `os-vst2` are the desktop build's only; `os-host` and `os-clap` every build's, for the built-in plugins. `os-engine` is also a dependency of `crates/audio` on every platform, for the effects. The hosts'
tests need Lemon's engine and plugins, so their dev-dependencies are not carried; run them in Lemon Studio.

## The effects (Lemon Reverb and Lemon Limiter)

`os-engine/src/effects/` is upstream's `crates/os-engine/src/effects/`, every file unmodified (Lemon's own comments and
tests included), at its own commit:

- Effects commit: `a6ce2dd443d75abf424a50fb5dd68526f87bed9d`

Honeydew runs `reverb.rs` and `dynamics.rs` as the built-in effects Lemon Reverb and Lemon Limiter
(`crates/audio/src/lemon.rs`, on every platform); the others come along so that `mod.rs` stays upstream's (they use
`os-dsp` and `miniz_oxide`, so `os-engine` always depends on those), and Lemon Parametric EQ runs `eq` (below), so the
effects and the plugins are synced to the same commit. Honeydew's own code reads them only through their public items,
so an upstream change that keeps those compiles as it is.

**Syncing.** In the development repo, `tools/sync-lemon-effects.sh [commit]` (default: upstream's `main`) fetches Lemon
Studio, copies the effect files here (removing those upstream removed), prints the diff and pins the new commit above.
Then build, run `cargo test -p honeydew-audio lemon` and the audio tests (`--test rt_alloc` checks that they never
allocate on the audio thread), and commit. Set `LEMON_REPO` to a local clone to skip the fetch. The public repository's
`lemon-effects` workflow checks every week whether upstream's `main` changed these files since the pinned commit, and
opens or updates the issue "Lemon Studio effects changed upstream" (closing it once they match again).

## The plugins (Lemon Trio, Lemon Sampler, Lemon Parametric EQ)

Lemon Studio's own plugins are built into Honeydew on every platform, as in Lemon Studio:
compiled in with their `linked` feature (no exported entry points) and hosted through `os-clap` from their CLAP entry
tables (`src/plugins/builtin.rs`). Their editors draw into the rack cards (`embed`, below). From upstream, at its own
commit:

- Plugins commit: `a6ce2dd443d75abf424a50fb5dd68526f87bed9d`

- `plugins/lemon-3x-osc`, `plugins/lemon-sampler` and `plugins/lemon-parametric-eq-2`: their `src/` (no examples).
- What they use that the hosts did not bring: `crates/os-plugkit`, `os-ui` (with its fonts and icon notice), `os-theme`
  (with its themes), `os-hostlink`, `os-model`, `os-dsp` and `os-paths`, each its `src/`; and from `os-engine`,
  `clock.rs`, `envelope.rs`, `laws.rs` and `sample.rs`, behind the `os-engine` feature `plugins`, which the plugins turn
  on (`honeydew-audio` builds `os-engine` without it).
- They build on nih-plug, vendored beside this folder (`vendor/nih-plug`, its own README).

Honeydew's changes to these files are `honeydew.patch` beside this file, each marked `Honeydew (vendored change)` or
`(vendored addition)` in the code:

- `os-plugkit`: the feature `embed` and its window, `src/embed.rs` (Honeydew's own file): the plugins' editor window API
  over a frame and an input queue that Honeydew keeps, run on the app thread with no thread of its own (the browser has
  none), the editor parked between inputs; menus and the type-in box answered without blocking, by delivering the input
  that asked again with the answer. `gui` is that window with `embed`; `time` is `std::time`, or `web-time` in the
  browser, whose `std::time` has no clock (`vals.rs` uses it).
- The plugins' `editor.rs`: with `embed`, their editor runs on every platform and takes every window handle as an
  embedded editor's id (Win32 too); after its input loop it parks. `os-hostlink`: with its `embed`, the link those
  editors use (`Peer`) on Windows too. `lib.rs`: the VST3 impl only without `linked`
  (Honeydew builds no VST3 wrapper).
- Lemon Sampler: in the browser, a sample's effects are computed at once instead of on a thread, `std::time` is
  `os_plugkit::time`, and the browser and Android have no sample file dialog. Lemon Trio: the browser and Android have
  no file dialog for its custom shape's sample.
- `os-dsp`: in the browser, which cannot compile Signalsmith Stretch (C++), `src/stretch_web.rs` (Honeydew's own file)
  stands in for `stretch.rs`: it resamples, so a stretched sound's pitch follows its speed there.
- The manifests are Honeydew's (as for the hosts), with `crate-type` lib only, nih-plug by path without its VST3 feature,
  rfd 0.17 (Honeydew's own) for Lemon Sampler's and Lemon Trio's dialogs, and lint settings for upstream warnings and
  lints Rust 1.91 does not know.

**Syncing.** `tools/sync-lemon-plugins.sh [commit]` (default: upstream's `main`) replaces these files with upstream's,
applies `honeydew.patch`, prints the diff, shows upstream's manifest changes to carry over by hand, and pins the new
commit above. If the patch no longer applies, port the changes by hand and run `tools/sync-lemon-plugins.sh
--save-patch`, which writes the patch from the vendored files against the pinned commit; run that after any change to
these files too. Then build, run `cargo test --test lemon_plugins` and the web and Android checks, and commit.
`LEMON_REPO` works as for the effects. The `lemon-effects` workflow also opens or updates the issue "Lemon Studio plugins
changed upstream" while upstream's `main` changed these files since the pinned commit.

## What differs from upstream (the hosts)

To pull upstream fixes, diff each crate against the commit above; these are the only changes.

- `os-engine` is not upstream's engine: it holds only what the hosts use from it, `instrument.rs` (the `Instrument` and
  `HostedEffect` traits, `TransportInfo`, `NoteLevels`, `ChannelLevels`) and `voices.rs` (without its tests), and the
  effects above, and with its `plugins` feature what the plugins use (above). `lib.rs` and `Cargo.toml` are Honeydew's. `ChannelVoicing` is an empty struct: only Lemon's `os-native` (not carried)
  reads it, and upstream's needs `os-dsp`.
- `Instrument::midi(offset, [u8; 3])` is new (default: ignored), for pitch bend, channel pressure and MIDI CCs, which
  Honeydew's channels send to their instruments. `os-host`'s `Guarded` forwards it; `os-clap` sends a CLAP MIDI event,
  `os-vst2` a MIDI event (`NoteKind::Raw`), and `os-vst3` the parameter the controller's `IMidiMapping` names, as
  automation. Each change is marked `Honeydew (vendored addition)`.
- `os-host/src/runloop.rs`: `remove` and `modify_fd` do nothing (and return `false`) once the thread's run loop is
  gone. Honeydew keeps its plugin instances in a thread-local made before the run loop's, so at exit they are destroyed
  after it, and a plugin that unregisters a timer or descriptor then (Surge XT does) aborted the app. Marked
  `Honeydew (vendored change)`.
- Issue #33: a plugin's notice that only its latency changed (VST2 `audioMasterIOChanged` with the same channels, VST3
  `restartComponent(kLatencyChanged)` alone) is read on the next `idle`, without suspending and resuming the plugin
  (`os-vst2/src/lib.rs`, `os-vst3/src/lib.rs`): one that announced its latency whenever it resumed was switched off
  and on again on every tick. `os-vst2` has a test for it (`mock::announce_delay`). And `os-host/src/runloop.rs` has
  `polling` and `next_timer`, so the app ticks only while something waits. Each marked `Honeydew (vendored change)` or
  `(vendored addition)`.
- `os-vst3/src/host.rs` also allows `unknown_lints`: Honeydew's CI pins Rust 1.91, which does not know the lint
  upstream allows there.
- The manifests name their version, edition and license instead of Lemon's workspace keys, point at each other by
  path, drop the dev-dependencies and `os-clap`'s examples, and pin `vst3-sys` to the commit Lemon's `Cargo.lock` has
  for its branch.
- Comments that cited Lemon Studio's design documents (`spec: …`, `§…`) drop the citation; the rest of each comment
  and all code are unchanged.

## Not carried

- `os-native` (FL Studio desktop's own plugin ABI, Windows DLLs), which hosts no VST3, VST2 or CLAP plugins, and the
  other plugins (Lemon Reverb and Lemon Limiter run as Honeydew's own effects, above).
- The app side (`os-app`): Honeydew's own code ports what it needs, with Lemon's headers kept: the plugin search and
  scanner (`src/plugins/scan.rs`, from `plugin_db.rs` and `plugin_scan.rs`) and the X11 editor window
  (`src/plugins/window.rs`, from `plugin_window/x11.rs`).
