BanaForge 0.5.13 — Omi touch-feedback test build

- Vocal Synth note taps now use ordinary note selection/editing instead of opening the lyric editor. Lyrics remain accessible through the dedicated LYRICS lane.
- Reserve visible floating-action bounds before raw underlying clip/note/drum hit testing, preventing those presses from changing the intended selection before Copy/Delete execute.
- Playback/buffer performance remains unresolved on reported devices. This touch pass does not promise an audio-latency improvement.
- Holding for context/marquee still exists; broader selection ergonomics and stock drum/preset expansion remain follow-up work. No unlicensed third-party sample packs are bundled.

BanaForge 0.5.12 — bounded waveform peak queries

- Adds shared multiresolution waveform peak summaries. Overview columns inspect a bounded number of peak bins rather than rescanning a recording-length base summary during zoom/pan. Close zoom keeps exact sample-detail rendering.
- Overview bins conservatively retain nearby peaks; they are not exact sample boundaries. The hierarchy is shared between clips of one source and uses no additional PCM copies.
- Cached waveform lookups no longer repeat the sample decode/reference lookup for every visible clip.
- Sound Mode remains NATIVE / FAST / SAFE / ULTRA SAFE on Android; raw buffer slider remains desktop-only. No new latency fix or guaranteed device latency is claimed in this revision.
- Physical-device FPS and latency remain unverified. Initial summary generation is still synchronous, so cold-load stalls remain a separate investigation.

BanaForge 0.5.11 — arrangement waveform and mobile audio profiles

- Adaptive waveform geometry: one-pixel detail close up; merged min/max peaks at wider arrangement zoom. Peaks are aggregated, not skipped; reversed audio and partial final columns are handled.
- Mobile Sound Mode restored: NATIVE, FAST, SAFE, ULTRA SAFE. The raw buffer slider remains desktop-only; old slider values no longer override mobile presets.
- Android callback targets: device-selected/native, 256, 1024 and 2048 frames. AAudio low-latency requests and native burst/underrun tuning come from the existing CPAL backend; actual hardware/route latency varies. No per-preset thread-priority or near-zero-latency guarantee.
- This revision requires on-phone FPS, underrun and latency comparison; it does not claim all reported lag is eliminated.

BanaForge 0.5.10 — Banana rack effects test build

## Added
- Selectable native rack modules: Banana Fresh Air (Mid Air, High Air, Trim), Banana Soundgoodizer (Amount and original A–D profiles), Banana Blood Overdrive (Drive, Color, Post Filter, Trim, Mix), and Banana Gross Beat (time/volume presets and editable 16-point snapped delay/volume grids).
- Gross Beat has Through, Half, Tape Stop, Stutter and Custom time modes; Through, Gate, Pump and Custom volume modes. Curves are causal delay envelopes, not arbitrary future-audio warps. History is bounded to eight seconds; longer delay requests clamp to available history.
- All four use original DSP and presets. These are not identical recreations of proprietary products.

## Enhanced
- New native module cards use live channel RGB tinting, or warm yellow/coral accents without a custom channel color. Native parameter automation, reset, undo and project storage are reused.

## Limitations / unresolved
- Initial DSP: no proprietary matching or oversampling-quality claim. Physical-device CPU, aliasing and listening evaluation remain required.
- The Gross Beat editor uses a fixed 16-column lattice and 1/16 value snapping, not a free breakpoint editor. Abrupt read-head jumps use a short decaying discontinuity correction; volume edges are smoothed.
- Language switching, custom backgrounds, the previously reported playback delay and specific DWP wrong-sound case remain unresolved.

## Earlier changes

BanaForge 0.5.9 — feedback implementation preview

## Added
- Channel Settings now has a circular hue/saturation wheel, brightness control and exact #RRGGBB input. Pure black/white are rejected; colors are saved in a BanaForge extension while the legacy project hue stays intact.
- Pitch names with octaves on readable synth piano-roll note blocks, including sharps (C4, F#4). This is not restricted to Vocal Synth.

## Enhanced
- Side-browser file/preset labels use smaller 14-point text beside bold 18-point folders; touch row heights stay unchanged.
- Dense piano rolls reduce decorative text, vocal curves and waveform overlays beyond 512 visible notes or below readable row heights. Note rectangles, positions, hit targets and audio processing remain unchanged.
- This offline update summary is organized by category and appears after an upgrade on desktop as well as mobile. Help > What's new reopens it.

## Fixed
- Shared parameter sliders also recognize desktop double-click reset, clear the old drag accumulator on reset, and ignore reset gestures while disabled. Touch double-tap reset remains supported.

## Removed
- Removed the mistaken pine-tree animation from UPDATES: “air freshener” referred to the requested Banana Fresh Air exciter, not an easter egg.

## Unresolved / unsuccessful prior fixes
- Playback audio delay: the 0.5.8 audible-clock fix did not resolve the reported device experience. No new audio-latency fix is claimed here.
- DWP wrong sound: existing zone parsing is not proof that the reported file plays correctly. A failing file and expected-sound reference are still needed.
- Localization and background-image selection/opacity are not delivered in this preview.

## Planned — not shipped
- Banana Fresh Air: Mid Air, High Air and Trim.
- Banana Soundgoodizer: one Amount knob with original A–D profiles.
- Banana Blood Overdrive: bass-friendly aggressive drive and tone shaping.
- Banana Gross Beat: editable time/volume envelopes, stutters and tape-stop curves.
- All four use native mobile-sized rack cards and live channel-color tinting. DSP, panels and Banana-specific fallback accents are not yet implemented. See FEATURE_BOARD.md for the specification and acceptance gates.

## Earlier changes

## BanaForge 0.5.8 — priority-fix test build

- Playhead: Android now uses the timestamp-aligned audible transport history, not the render-ahead cursor. UI extrapolation beyond the heard position is disabled. Startup no longer substitutes a future history entry when no sub-block has been heard yet. This uses CPAL playback timestamps, not a direct platform sampleFramesPlayed API; physical-device synchronization remains to be checked.
- Waveforms: unchanged playlist/sample-strip waveform meshes reuse a UI-thread cache (up to 32 entries / 4 MiB of geometry). Weak source references avoid retaining decoded audio after its clips are removed. Existing min/max downsampling and negative missing-file caching remain in place. No measured 60%-zoom Android performance result yet.
- Browser: every preset/file pick explicitly requests a repaint after the active path changes. Folder labels are now bold in the shared side browser.
- DWP: investigation confirms existing multi-zone/root/velocity metadata parsing and worker loading. No new DWP-format fix is claimed; the reported wrong-sound file still needs reproduction.

Still pending from Stheven's report: larger folder text, localization extraction, full channel wheel/hex colors with readability limits, ordinary piano-roll pitch labels, and the audio exciter later clarified as Banana Fresh Air (not an air-freshener easter egg). This is not a claim that the complete report is resolved.

---

BanaForge 0.5.4 keeps menus on top of their dialogs, makes slide notes glide on the Banana synths and audio clips follow tempo automation, lowers Android audio latency, and adds render tags, more Half Time controls and many fixes from the issue tracker.

- Menus. A slider's long-press menu in a plugin window stays on top of the window, and the screen no longer gets darker each time it opens (#118).
- SoundFont Player. The arrows by the SoundFont's name load the previous or next SF2 or SFZ file in its folder, tapping a patch plays a preview, a Transpose knob shifts it by up to two octaves, and on phones the panel is taller (#120, #102, #135).
- Half Time. Length runs from 1/16 bar to 8 bars, Speed plays at 1/2, 1/4 or 2/3, and Smooth softens each loop start (#122).
- Banana plugins. Slide notes glide on Banana Trio, Citrus and the other plugin synths, and Tesla's Send to arranger puts its sample on a new audio track (#125, #117).
- Tempo. Audio clips with Sync on follow tempo automation, their pitch moving with the tempo, and renders follow it too (#126).
- Drum editor. A drag on a graph slider stays in its column unless you clearly move sideways (#127).
- Android audio. Playback asks Android for low-latency audio, Calibrate works on phones, the latency switch reads On while compensation runs, and sound comes back by itself when headphones are plugged or unplugged (#123, #129, #130).
- Songs. A tap on a sample plays it and a long press selects it, the New Song dialog lets you type a name, and the browser keeps its folder after you open a song (#123, #130, #138, #131).
- Browsers. Audio files show their extension, the sample strip draws its waveform like the playlist and shows a stop button and a playhead while it plays, and pickers reopen where you left them (#123, #124, #131).
- Rack. Insert above or below adds a single module even after going back from the Instrument list, and modules no longer show behind the header's corners when you scroll (#123).
- Piano roll. Slide notes have a look of their own, Legato stretches the last note to the end of its bar, and with Generate open you can swipe and pinch the roll behind it (#130, #102, #123, #135).
- Automation. New Clip on effect and master tracks makes a volume automation clip, and the automation editor shows a value ruler when you zoom its values (#124, #135).
- Mixer. Each track shows its level in dB, and the meters fall faster (#133, #135).
- Project. New Album, Genre, Year and Comment fields, and renders to WAV, FLAC, OGG and MP3 carry them with the title and author as tags, with Write tags on export to turn it off (#139).
- Settings. A long theme name fits its box, and Performance Clear resets the counters (#124).
- DW Sampler. The Filter and Pitch envelope amounts read 0 at the centre (#102).
