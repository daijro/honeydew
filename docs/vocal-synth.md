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
   root. Atomically publish extracted banks and manifests; automatically classify oto
   archives without mistaking song ZIPs for banks. Never extract symlinks or traversal paths.
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
The owner re-enabled Actions. Android build 44018530986 was dispatched successfully;
new commits on this branch also trigger Android and vocal regression workflows.
A build is not a delivered APK until the workflow completes and its artifact is verified.
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
