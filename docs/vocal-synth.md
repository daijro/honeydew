# Vocal Synth update — implementation status

## Working foundation

The playlist Synth menu creates a native Vocal Synth channel (module 53). It uses the
normal rack and FX chain. Its procedural formant oscillator supports vowel selection,
formant/gender, breath, brightness, glide and vibrato. No voicebank is required to play it.
The native piano roll stores per-note lyrics, pitch points and Dynamics/Breath/Formant
values in the HDVo extension. It has next-note typing, pitch drawing, and expression editing.

## Changes in this update

* Native voicebank ZIP inflation and oto indexing run on a bounded single import worker,
  rather than blocking the UI. The browser only draws visible alias rows.
* Index every oto in a bank's pitch subfolders, resolve WAV names relative to each oto,
  keep deterministic ordering, and tolerate empty/malformed subbank oto files.
* Reject NaN/infinite/malformed oto timings and control characters in sample paths.
* Choose black/white lyrics using linear-light contrast, and clip long lyrics inside notes.
* Fix overlapping same-key release, note panning, glide initialization and pitch reset.
  Reject non-finite DSP controls; skip idle voice/filter work.

## Important incomplete functionality

**This is not a complete UTAU renderer.** Imported samples are retained in the original
archive, not extracted or played. The alias browser is explicitly metadata-only.
Per-note pitch, Dynamics, Breathiness and Formant now drive the procedural synth.
Explicit Latin vowels (and standalone Japanese vowels) select its vowel sound. This is
not phonemization or consonant synthesis. Updates are bounded at 100 Hz, smoothed in
individual voices; overlapping different keys are independent. Overlapping same-key
voices currently route to the newest voice. Note-effect transposition/generation and
seek reconstruction of vocal expression are not complete.

The waveform overlay now measures dry solo-note PCM rendered by the procedural synth
on a native worker (8 kHz preview). It is NOT a user voicebank waveform, full-rate final
mix, or post-FX waveform. It omits inter-note glide, automation and other voices.
It is bounded to 64 notes, eight seconds per note, and 512k frames per job; unavailable
previews are blank. Web thumbnails are omitted rather than blocking its UI thread.
The Default header still browses module presets, not loadable voicebanks.
Do not label these as finished vocal rendering or measured audio previews.

## Remaining architecture

1. Add a worker-prepared immutable bank (validated WAV PCM, oto timing, pitch marks,
   prefix.map) behind Arc. Use sample and decoded-frame budgets as well as the ZIP cap.
2. A pure Rust PSOLA renderer consumes per-note phoneme, cents and expression events.
   Offset/cutoff, fixed consonants, preutterance/overlap, loop boundaries and unvoiced
   regions need fixture tests. Crossfade at pitch marks, not arbitrary sample boundaries.
3. Pass vocal metadata through scheduling, seek, loop, edit-adoption and export, with
   per-voice routing for overlapping notes. A global pitch bend cannot represent polyphony.
4. Only after the loader exists, route the Default drawer to a permanent My Voicebanks
   root. Atomically publish extracted banks and manifests. Normal ZIP imports now classify
   oto archives into My Voicebanks, while project ZIPs retain precedence. This reuses
   the generic import's already decoded entries; explicit bank imports use the worker.
   Samples are still archived, not extracted or playable. Never extract symlinks or traversal paths.
5. Cache actual min/max PCM waveform levels off-thread, paint only visible samples;
   remove the schematic overlay when real rendered data is available.
6. Validate on Android with audio underrun counters, sustained multi-voice playback,
   touch/IME tests, import cancellation, seek/loop/export comparisons and APK installation.

## Open-source references and licensing

* OpenUtau: https://github.com/openutau/OpenUtau — MIT; inspected Classic/VoicebankLoader.cs
  for oto/subbank organization. This update does not copy its C# engine or require a
  desktop executable resampler.
* WebTAU: https://github.com/charl1e7/webtau — MIT; the existing oto parser adaptation
  and attribution are retained in THIRD_PARTY_NOTICES.md and licenses/webtau-MIT.txt.
* WORLD: https://github.com/mmorise/World — candidate for a future worker renderer,
  not incorporated by this change. Verify its license and transitive dependencies
  at the pinned revision before importing code.

Voicebank licenses are independent from DAW licenses. No Teto or other third-party
voice samples are included. A license filename is not proof of redistribution rights.

## APK

`.github/workflows/android.yml` builds this exact branch into a signed sideloadable APK,
verifies its signature/install metadata and ZIP CRC, and uploads `honeydew-android-apk`.
The owner re-enabled Actions. The final ARM64 base APK from source commit 4b894b3
completed successfully in Android run 44019835588, including apksigner, install metadata,
and ZIP CRC verification. Download its `honeydew-android-apk` artifact:
https://github.com/jellyscoll76-Real/honeydew/actions/runs/44019835588/artifacts/14300368248

This lightweight build excludes Honeydew's large sound library and adds no voicebank.
The procedural voice is built into the native library. GitHub login may be required.
The sandbox cannot copy the artifact locally because its redirect goes to an inaccessible
Azure artifact-storage host. This does not prevent downloading it through GitHub.

Vocal regression run 44019821112 passed both the audio-crate vocal tests and the app's
`vocal_` filtered tests (parser, scheduling, lyrics, waveform, contrast, ZIP auto-routing,
and existing vocal serialization coverage). Windows/macOS native smoke checks passed;
full workspace CI 44019821165 subsequently failed with nine app tests. Failures include
module-browser/rack expectations around the new type 53, the user-folder list, selector
and parameter-range tests, and a piano-roll graph-paint interaction. These have not been
resolved by this update; do not report the full suite as green. No Android device
installation was performed.
The sandbox lacks JDK/Android SDK; their normal download hosts are restricted.
An alternate npm-distributed Rust 1.88 toolchain was obtained for formatting and
isolated DSP tests, but this is not a full Android build environment.

Validation performed: the actual Vocal Synth source, compiled with its Instrument
trait in an isolated rustc harness, passed all six DSP unit tests. This does not
substitute for the full workspace, Android integration or device tests.
Seven parser/indexing tests also passed in an isolated harness (serde and ZIP adapter
excluded, encoding_rs 0.8.35 used for UTF-8/Shift-JIS checks rather than the workspace's
0.8.42). Full dependency-version and ZIP integration checks still need CI. A dedicated
Vocal Synth validation workflow runs the workspace regression tests once Actions
is enabled.
A CI signing key without repository release secrets is ephemeral: subsequent builds may
require uninstalling the previous build. Back up projects before uninstalling.

## Lyric paste

Pasting splits explicit spaces, whitespace and hyphens across selected notes in temporal
order. Without a multi-note selection, it uses consecutive notes from the active note.
A single remaining note keeps the phrase. Overflow is joined on the last target, with
the existing 64-character per-note limit. This is separator-based syllable entry, not
linguistic syllabification. Ordinary typing remains single-note. Procedural voice remains
the bundled default; no additional voicebank download is required.

## Touch, IME and binding follow-up

* Pitch Draw now owns the grid even when a touch misses a curve node; note moves,
  resize handles and floating note actions cannot capture those touches. Nodes use
  a 22-point radius and are searched globally before note-body hits. A semitone is
  one piano-roll row in both painting and gesture conversion.
* HDVo already stores relative normalized time and cents, not screen positions.
  Selected parent anchors now follow note dragging and transpose operations; the
  relative curve and lyric stay unchanged (no file format migration).
* Lyric editing calls android_text::edit, the existing NativeActivity-to-EditText
  bridge. The Java editor requests focus and posts showSoftInput after attachment.
* The existing bounded audio command queue and SetParam routing are retained.
  Reverb Mix uses param 3 for both legacy 12 and current 32; regression tests
  exercise both through the renderer dispatcher. No additional UI/audio locks.
* Prepared per-note expression timelines restore vocal state on seek and adopted
  engine edits, including reset at the next note-on. No render-thread allocation.
* The selector, procedural DSP formants and lyric-vowel mapping use A/E/I/O/U.
  The DSP now uses the same round-to-nearest menu quantization as the UI. Existing
  non-default saved Vowel knob values will select the newly ordered vowel.
* GitHub reported this repository is already public (isPrivate: false).

### Verified build: 55ff23f (2026-11-17)

* ARM64/no-bundled-sounds APK run 44033327774 succeeded, including independent
  APK signature, install metadata and ZIP-content checks. Download:
  https://github.com/jellyscoll76-Real/honeydew/actions/runs/44033327774/artifacts/14305969428
* Artifact archive: 25,823,926 bytes; SHA-256
  `60b79045c67358c4f4a90f17f84075702559b14a55f95fa092d280a1044fb42e`.
  This is the archive digest, not a separately measured APK digest.
* Vocal regression run 44033234805 passed. Full CI 44033234712 passed formatting,
  strict workspace/all-target Clippy, Windows and macOS parallel-engine smoke
  tests, but failed nine workspace tests: three instrument parameter snapshots,
  three module browser expectations, rack module-ID boundary, user folder
  expectations, and graph touch-painting. Full workspace CI is NOT green.
* Vocal drags now send coalesced auditions (at most one per 100 ms of changed
  input) using the existing engine handover; release publishes the final curve.
  Large bends are no longer clipped to the parent note's immediate row.
* Reverb dispatch tests validate parameter routing, NOT measured audible wet/dry
  response. Physical Android touch/IME behavior and audible playback still need
  device verification. User-bank sample singing remains unimplemented; the
  built-in procedural voice is still the only singing renderer.

## Device-report follow-up (2026-11-17, source 5741cdf)

* Android text completion no longer requests an egui repaint while holding the
  native editor mutex. This removes a callback/UI lock-order inversion; the
  reported physical-device Done crash has not been reproduced here.
* Android snapshots flag a callback heartbeat older than three seconds as a
  failed stream, allowing the existing automatic reopen path to recover silent
  callback stoppages. This is recovery, not a diagnosis of every audio lockout.
* Android's visual playhead now uses buffer transport snapshots instead of
  latency-compensated heard history. Count-in still gates interpolation. This
  intentionally trades Bluetooth latency compensation for immediate buffer sync
  on Android. Device startup timing remains to be measured.
* Vocal tools follow the first current rack instrument, not channel creation
  kind. Leaving Vocal Synth clears pitch and lyric gestures. Vocal Synth is now
  in the standard SYNTH picker. My Folders lists mixed file types in My Songs
  and My Samples; song-only and preset pickers retain their extension filters.
* ZIP bank detection ignores bundled demo MIDI/audio when deciding whether the
  ZIP is a project; actual FLM/template/FLP projects still take priority.
  Import remains metadata registration, not sample-bank singing playback.
* Bordered dark lyric blocks, opaque cyan waveforms with dark outlines, and a
  last-painted, outlined pitch overlay remain readable independently of track
  color. Smooth per-note segments use the same eased interpolation in audio and
  visualization; node positions remain explicit. No cross-note resampler added.
  FLATTEN BENDS resets selected pitch data without deleting lyrics. This is an
  incremental editor update, not complete desktop Vocaloid control parity.

### Validation / download

* APK run 44044374537 passed build and independent signature/install/ZIP checks:
  https://github.com/jellyscoll76-Real/honeydew/actions/runs/44044374537/artifacts/14311990084
* Artifact ZIP: 25,830,390 bytes, SHA-256
  `44210d4abea446245d64adcdc0abd0babebfb85142c8251e120d42d08e2d0d68`.
* Vocal and dedicated My Folders regressions passed (44044375624). Strict
  Clippy, formatting, and macOS/Windows engine smoke tests passed (44044375615).
* Full workspace tests still fail seven cases: three instrument parameter
  snapshots (including stale lowercase vowel order), two rack expectations,
  user-folder expectations, and graph touch-painting. Browser menu regressions
  no longer appear in the failures. Full CI is not green. No device install,
  audio measurement, or physical IME validation was performed in this sandbox.
