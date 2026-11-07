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
tests included), with `os-engine/src/formula.rs` (the formula language Lemon Formula Controller's effect reads), at its
own commit:

- Effects commit: `e357d8509a7d8fb04ce0642d549d32f19757709b`

Honeydew runs `reverb.rs` and `dynamics.rs` as the built-in effects Lemon Reverb and Lemon Limiter
(`crates/audio/src/lemon.rs`, on every platform); the others come along so that `mod.rs` stays upstream's (they use
`os-dsp` and `miniz_oxide`, so `os-engine` always depends on those), and Lemon Parametric EQ runs `eq` (below), so the
effects and the plugins are synced to the same commit. Honeydew's own code reads them only through their public items,
so an upstream change that keeps those compiles as it is. Since 0.4.9 the plugins below run more of them (Lemon Small
FX's, Lemon Delay's, Lemon Vocoder's, Press's `maximus` and Pulp Filter's `love_philter`); the rest are only built.
Three of them allocate on the audio thread (upstream's, kept unmodified; `tests/rt_alloc.rs` leaves their plugins out):
`maximus`'s peak detector grows its queue up to its window (`Detector::start` could reserve it), `love_philter`'s
`flush` (a reset: every stop) makes its half-bands again, and `eq`'s `preroll` (Lemon Parametric EQ's reset) its
buffers.

**Syncing.** In the development repo, `tools/sync-lemon-effects.sh [commit]` (default: upstream's `main`) fetches Lemon
Studio, copies the effect files and `formula.rs` here (removing those upstream removed), prints the diff and pins the
new commit above.
Then build, run `cargo test -p honeydew-audio lemon` and the audio tests (`--test rt_alloc` checks that they never
allocate on the audio thread), and commit. Set `LEMON_REPO` to a local clone to skip the fetch. The public repository's
`lemon-effects` workflow checks every week whether upstream's `main` changed these files since the pinned commit, and
opens or updates the issue "Lemon Studio effects changed upstream" (closing it once they match again).

## The plugins (Lemon Trio, Lemon Sampler, Lemon Parametric EQ)

Lemon Studio's own plugins are built into Honeydew on every platform, as in Lemon Studio:
compiled in with their `linked` feature (no exported entry points) and hosted through `os-clap` from their CLAP entry
tables (`src/plugins/builtin.rs`). Their editors draw into the rack cards (`embed`, below). From upstream, at its own
commit:

- Plugins commit: `e357d8509a7d8fb04ce0642d549d32f19757709b`

- `plugins/lemon-3x-osc`, `plugins/lemon-sampler` and `plugins/lemon-parametric-eq-2`: their `src/` (no examples).
- Since 0.4.9, effects: `plugins/lemon-small-fx` (the plugin, parameters and editor Lemon Studio's small
  effects share) and its ten plugins, one crate each (`lemon-soft-clipper`, `lemon-fast-distortion`, `lemon-balance`,
  `lemon-stereo-enhancer`, `lemon-transient-processor`, `lemon-chorus`, `lemon-stereo-shaper`, `lemon-filter`,
  `lemon-flanger`, `lemon-phaser`), `plugins/lemon-delay`, `plugins/lemon-vocoder` (its carrier a sidechain, below),
  `plugins/press` and `plugins/pulp-filter`.
- `plugins/tesla` (Tesla, Lemon's audio editor and recorder, an effect; since 0.4.9): its `src/`.
- Since 0.4.9, the instruments `plugins/citrus` (Citrus), `plugins/lemon-drum-sampler` (Lemon Drum Sampler)
  and `plugins/lemon-slicer` (Lemon Slicer), their `src/`.
- What they use that the hosts did not bring: `crates/os-plugkit`, `os-ui` (with its fonts and icon notice), `os-theme`
  (with its themes), `os-hostlink`, `os-model`, `os-dsp` and `os-paths`, each its `src/`; and from `os-engine`,
  `clock.rs`, `envelope.rs`, `laws.rs` and `sample.rs`, behind the `os-engine` feature `plugins`, which the plugins turn
  on (`honeydew-audio` builds `os-engine` without it).
- They build on nih-plug, vendored beside this folder (`vendor/nih-plug`, its own README).

Honeydew's changes to these files sit in the crates the plugins share, so that a plugin needs none of its own but the
one its editor gets from the sync (below). They are `honeydew.patch` beside this file, each marked `Honeydew (vendored
change)` or `(vendored addition)` in the code:

- `os-plugkit`: the feature `embed` and its window, `src/embed.rs` (Honeydew's own file): the plugins' editor window API
  over a frame and an input queue that Honeydew keeps, run on the app thread with no thread of its own (the browser has
  none), the editor parked between inputs (`park!`, which ends its input loop); menus, the type-in box and the file to
  open answered without blocking, by delivering the input that asked again with the answer. `gui` is that window with
  `embed`; `time` is `std::time`, or `web-time` in the browser, whose `std::time` has no clock (`vals.rs` uses it);
  `spawn` is `std::thread::spawn`, the work done at once in the browser, which has no threads. The editor also parks
  after each tick (its polling period), so one slower to paint than a tick still hands the app its frame. A frame names
  the pixels that changed since the app took the last (`Frame::changed`), which the card's texture takes alone (#72).
- Files: where the host answers (`embed::host_files`: Android and the browser, which have no file dialog), an editor's
  own file dialog code asks it from its window's `dialog` (`embed::open_file`): Honeydew's own sample picker opens the
  file. The plugins' `rfd` is `host-rfd` (below), so that code builds everywhere unchanged.
- Saving there: `host-rfd`'s save dialog gives the name the editor proposes, and an editor that writes it with
  `os_plugkit::save_file` (Tesla's and Lemon Sampler's `Save sample as`, one line each) hands it to Honeydew
  (`embed::take_saved`), which keeps it in the user folder's `My Recordings`, under a free name (nothing overwritten).
  Elsewhere `save_file` writes the file. `os_plugkit::temp_dir` is `std::env::temp_dir`, which panics in the browser:
  there an empty path, where nothing writes (Tesla's files handed out, inert in Honeydew).
- Tesla: the file it opens is read with `os_engine::sample::read` (a file the host keeps in memory first); with no
  thread for its recording's take (the browser has none), its audio thread builds the take.
- `os-hostlink`: with its `embed`, the link the editors use (`Peer`) on Windows too.
- Lemon Sampler: in the browser, a sample's effects are computed at once instead of on a thread (`os_plugkit::spawn`).
- Lemon Delay: its About menu's line names no other maker's product (Honeydew shows none).
- Lemon Parametric EQ (#95): its embedded editor's spectrum reads the signal behind the newest sample on a clock at
  the sample rate (`analyzer::Reader`), so that blocks written in uneven bursts (a Bluetooth output's) move it on evenly.
- Lemon Drum Sampler and Lemon Slicer: their voices' ended-note lists are drained instead of taken, keep their room,
  and the drum pads' velocity layers are found without making a list, so that a note never allocates on the audio
  thread (`tests/rt_alloc.rs`). Citrus is upstream's: it still allocates its voices (its unison list and each voice's
  buffers) on a note, and its effects copy a delay's input each block while a delay is on.
- `os-engine`'s `sample.rs`: a file the host keeps in memory (`provide`: the browser's, which has no file system) is
  read before the disk.
- `os-dsp`: in the browser, which cannot compile Signalsmith Stretch (C++), `src/stretch_web.rs` (Honeydew's own file)
  stands in for `stretch.rs`: it resamples, so a stretched sound's pitch follows its speed there.
- `os-dsp`'s web `whole_formant` (Tesla's Time stretch / pitch shift) is `whole`: the formants follow the resampling.
- The manifests are Honeydew's (as for the hosts), with `crate-type` lib only, nih-plug by path without its VST3 feature,
  the feature `embed` (`os-plugkit/embed` and `os-hostlink/embed`), `rfd` as `host-rfd`, and lint settings for upstream
  warnings and lints Rust 1.91 does not know.

Beside them, Honeydew's own: `host-rfd`, rfd's `FileDialog` as far as the editors use it, which the plugins' manifests
name `rfd`: rfd's dialog, or the host's picker where it answers (above). And nih-plug (`vendor/nih-plug`) gives an
editor's window as an X11 window id on Windows too, and keeps `Vst3Plugin` without its VST3 wrapper, so that the
plugins' `lib.rs` build as upstream's.

**Each plugin's editor.** Upstream's editors (`src/editor.rs`) share one template, and the sync script embeds each the
same way (`embed_editor` in `tools/sync-lemon-plugins.sh`), instead of a hunk per plugin in the patch:

- every `#[cfg(any(target_os = "linux", target_os = "macos"))]` gains `feature = "embed"`: the Linux and macOS editor
  (`mod gui`, its waker) is built on every platform;
- the X11 window's arm (`ParentWindowHandle::X11Window`) takes the window everywhere but macOS:
  `#[cfg(any(target_os = "linux", all(feature = "embed", not(target_os = "macos"))))]`;
- `os_plugkit::park!(self);` follows the input loop (`while let Some(input) = self.win.next() { … }`);
- `use std::time::` is `use os_plugkit::time::`.

An editor off the template gets what matched and a warning naming it: make the rest by hand, then `--save-patch`.

**Syncing.** `tools/sync-lemon-plugins.sh [commit]` (default: upstream's `main`) replaces these files with upstream's,
embeds each plugin's editor (above), applies `honeydew.patch`, prints the diff, shows upstream's manifest changes to
carry over by hand, and pins the new commit above. If the patch no longer applies, port the changes by hand and run
`tools/sync-lemon-plugins.sh --save-patch`, which writes the patch from the vendored files against the pinned commit's
(their editors embedded); run that after any change to these files too. Then build, run `cargo test --test lemon_plugins` and the web and Android checks, and commit.
`LEMON_REPO` works as for the effects. The `lemon-effects` workflow also opens or updates the issue "Lemon Studio plugins
changed upstream" while upstream's `main` changed these files since the pinned commit.

**Which plugins.** `plugins.txt` beside this file lists them, one upstream `plugins/<name>` per line, with the
`os-engine` files they use (`os-engine/src/<file>`). The plugins and `crates/os-*` their manifests depend on are worked
out from upstream's `Cargo.toml` files (`tools/sync-lemon-plugins.sh --paths` prints them all; the workflow compares
those), and each one's folders are vendored but `examples`, `tests` and `benches`. So vendoring another plugin is a line
in `plugins.txt` and a sync; the script then prints upstream's manifest of each plugin or crate new here, for Honeydew's
own (and a path in the root `Cargo.toml`), and a new `os-engine` file still needs its `mod` in `os-engine/src/lib.rs`.

**Adding a plugin.**

1. Its upstream `plugins/<name>` as a line in `plugins.txt`, with any `os-engine/src/<file>` it uses that is not listed
   yet (and that file's `mod` in `os-engine/src/lib.rs`).
2. `tools/sync-lemon-plugins.sh` at the pinned commit (or a newer one): it vendors the plugin and what it uses, embeds
   its editor, and prints upstream's manifest of each plugin or crate new here.
3. Honeydew's manifest for each new one, from upstream's as `lemon-3x-osc/Cargo.toml` is: version, edition and license
   instead of the workspace keys; `[lib]` with `doctest = false` and no `crate-type`; the features `linked = []` and
   `embed = ["os-plugkit/embed", "os-hostlink/embed"]`; dependencies by path (`nih_plug = { path = "../../nih-plug" }`,
   `os-… = { path = "../os-…" }`, and `rfd = { package = "host-rfd", path = "../host-rfd" }` for rfd); and its
   `[lints.rust]`.
4. In Honeydew: `<name> = { path = "vendor/lemon-studio/<name>", features = ["linked", "embed"] }` in the root
   `Cargo.toml`, and the crate in `src/plugins/builtin.rs`.
5. Build, run `cargo test --test lemon_plugins` and the web and Android checks. Where the sync warned, or a build for one
   platform fails (a thread the browser cannot start: `os_plugkit::spawn`), change the plugin by hand and run
   `tools/sync-lemon-plugins.sh --save-patch`.

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
- A sidechain (0.4.9, Lemon Vocoder's carrier): `os-host`'s `Plugin::sidechain` (an audio input port after the main
  one) and `os-engine`'s `HostedEffect::process_fx_keyed`, which `os-clap` answers by giving that port the key, or
  silence without one: it now gives a plugin a buffer for each of those ports (nih-plug read the port it was not given).
  Each marked `Honeydew (vendored addition)` or `(vendored change)`.
- Since 0.4.9, the controllers' links: `os-clap` reads a plugin's controller outputs (MIDI pitch bend and CC 63
  per output; `HostedEffect::control_fx` in `os-engine`, forwarded by `os-host`'s `Guarded`), as upstream does since
  its commit 6dc00dd; a plugin with a note output port (a controller) never sleeps, so it runs on silence; and an
  effect's input copy and the queue of events it puts out keep their room, so the audio thread never grows them. Each
  marked `Honeydew (vendored addition)` or `(vendored change)`.
- `os-clap/src/lib.rs`: a block's event order and the events left for the next block are kept in the instance with
  their room, so that the CLAP host does not allocate on the audio thread for notes (`tests/rt_alloc.rs`). Marked
  `Honeydew (vendored change)`.
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
