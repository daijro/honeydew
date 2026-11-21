BanaForge 0.5.27 — Playlist visual and touch polish

- Activity lights now occupy a dedicated gutter, with square faces and spacing clear of the original controls. White is the default flash; optional persistent track-color matching sits beside the existing visibility switch. Uses real channel output meters, including monitoring, without a new timer or audio processing.
- Larger square white follow-position handle and padded ruler-clipped finger target. Saved anchor, drag clamping, and playback behavior are unchanged.
- Playlist track names and module titles use a bounded prefix with end ellipsis; truncated labels are additionally clipped to their cells, including extremely narrow layouts. Full project/file names are not changed.
- Hide the top-right BanaForge menu button while the channel rack/module interface is visible; restore it when closed.
- Adds multi-scale geometry, settings persistence and existing interaction regressions. Physical Android touch/performance verification remains necessary.

BanaForge 0.5.26 — compact channel controls, Gross Beat layout and help alignment

- Channel Settings: bound Monitor and Record arm controls to compact themed buttons with captions, rather than sizing their discs from the color-wheel row. Recording/monitoring functionality remains connected to the same settings. Keep brightness Value caption and numeric slider within the color column; clamp audio-channel dialog width to the viewport.
- Banana Gross Beat: replace compressed multi-item TIME/VOLUME lists with one mode selector on the active tab, and separate mode, Length and Mix rows. Presets, custom points and processing are unchanged.
- Settings information icons now paint and interact in their actual scrolling parent row, with the page clip. Workspace help chrome is suppressed while menus or other app overlays are open, so it cannot float above Settings navigation. The persisted Show information buttons preference is preserved.
- Adds layout/clip regressions for channel button bounds, narrow color captions, both Gross Beat tabs, scrolling info rows and workspace/menu isolation. Physical Android acceptance remains necessary across display sizes/scales.

BanaForge 0.5.25 — optional contextual information icons

- Settings → User interface → Show information buttons defaults ON. Turn it OFF to hide contextual icons and their touch targets throughout Playlist, Piano Roll, Mixer, Audio Editor and Settings; turn it ON to restore their original positions.
- The preference persists independently of project files. Help content and already-open explanations are preserved. Audio processing, envelope data, other controls and their positions are unchanged.

BanaForge 0.5.24 — PAN runtime restoration, stereo waveform prediction, contextual help

- Reasserts each active audio clip's compiled VOL/PAN/SPEED at render-block boundaries; seek/relaunch restores base settings where no previous envelope step applies. Live envelope handover retains sample phase and sounding voices. PAN remains linear stereo balance: it attenuates the opposite channel, not folds its content into the other side.
- Playlist and Audio Editor waveforms show predicted left output above right when PAN points exist, with quantized VOL×PAN gain. Mono source peaks can supply both output rows without another PCM decode. This is before channel/master effects, not a final-output meter; overview columns retain conservative peak gain. Overlay visibility/colors remain display-only and persisted.
- Small circular information buttons open compact, scrollable, dismissible explanations in Playlist, Piano Roll, Mixer, Audio Editor and relevant advanced Settings rows. Includes actual effects, ON/OFF meaning where applicable and stereo/device limitations. Help does not send audio commands. Further individual plugin/control topics can use the shared catalog.
- Regression suite verifies real mono and stereo PCM, center/left/right balance, live phase preservation, envelope shape/persistence/overlap behavior, help opening/dismissal, and existing editors. Physical Android routes, accessibility mono-audio settings, touch layouts and heavy-project performance still require device testing. See PAN_HELP_TESTING.md.

BanaForge 0.5.21 — waveform quality and easier follow-handle grabs

- New persisted Waveform Quality selector immediately after Sound mode: High Quality (default/current filled detail), Balanced (larger adjacent peak groups), Performance (sparse independent one-device-pixel vertical amplitude bars).
- Applied to Playlist, Audio Editor, sample-picker previews, instrument waveform strips, Slicer and optional Vocal Synth note previews. Reuses existing PCM/summary/voice-preview caches; quality is part of shared geometry-cache keys. Audio processing/quality, clip timing, project data and export are unchanged.
- Controlled optimized Linux renderer tests measured ~66.6% fewer vertices for Balanced and 87.5% fewer for Performance across four views. CPU tessellation was lower; cold-build savings varied substantially, and warm cache hits were effectively unchanged. See WAVEFORM_QUALITY_MEASUREMENTS.md for actual timings, protocol and limitations. These are not Android FPS/GPU/battery measurements.
- Enlarged the invisible horizontal follow-position handle target within the Playlist ruler; the visible yellow handle is unchanged. Existing seek/range/drag ownership regressions and a padded-area press test pass.
- Regression and Android package verification are automated. Physical Android touch/playback/frame-time verification remains unmeasured; no device freeze-free or latency guarantee is made.

BanaForge 0.5.20 — adjustable Playlist playback-follow position

- Enable Continuous scrolling during playback, then drag the yellow double-arrow handle at the top of the Playlist ruler. The selected target is 5–95% across the visible timeline; the existing 25% position is the default.
- The Playlist scrolls using the actual playback snapshot and existing timing lead. Adjustment changes only viewport offset, not transport position, playback state, clips, automation, effects or export processing.
- The handle is separate from the seek knob. Its complete drag/release is isolated from ruler range selection, seek and scroll gestures. Outside the handle, normal ruler/clip interactions remain available. With continuous scrolling off the handle disappears and ordinary scrolling is unchanged.
- The target is saved independently of the follow toggle and restored on return/restart. Other editors retain their existing follow behavior. Existing start/end scroll limits and short-song behavior still apply.
- Automated UI/math/persistence regressions and an installable Android APK are provided through Actions. Physical Android touchscreen/audio-sync validation remains a device acceptance test; see PLAYLIST_FOLLOW_TESTING.md.

BanaForge 0.5.19 — key-label visibility and Android sound profiles

- Show Key Names now hides all keyboard/Piano Roll key-column and note-block names when off, including root/octave text. Colored root markers remain; the existing saved preference is preserved.
- Android output now genuinely uses Oboe: Native requests the recommended low-latency backend (AAudio where supported/recommended, OpenSL ES fallback). Fast explicitly uses OpenSL ES/LowLatency with a 256-frame callback target; Safe and Ultra Safe use OpenSL ES with 1024/2048-frame targets and no low-latency performance request. Device limits determine actual buffer sizes.
- Mode changes restart only real-time output, preserving project data, transport/loop and restored engine controls. A brief gap and reset of active voices/effect tails are possible. Output profiles do not change offline export quality settings.
- BanaForgeAudio logcat diagnostics report the accepted backend, rate, buffer/callback sizes, callback CPU load, and hardware xruns where available (deadline misses otherwise). Settings retain their existing persistent IDs.
- Physical phone touch-to-sound latency, CPU, heavy-project stability, route compatibility and export PCM comparisons remain unmeasured. See docs-testing-0.5.19.md; this release is not a claim of full Android validation.

BanaForge 0.5.18 — connected export feedback and management

- Replaces audio export's title-only busy screen with a responsive, scrollable export screen. A bounded waveform-shaped graphic fills from confirmed timeline frames in the current render pass; actual pass percentage and frame counts are shown. Explicitly not a PCM/audio preview and not an overall percentage/ETA. Stage changes reset the graphic; unknown-duration processing and finalization display no invented percentage.
- Pass one analysis/render, pass two render/encode, and encoder finalization are separate stages. Normal render slices target 8 ms rather than 25 ms, without changing block size, DSP, sample rate or codec quality.
- Cancel drops the render/encoder before output writing. Mixes are only saved on successful encoding; native writes remain atomic. Previously completed individual stems are retained and the cancelled batch is not reported complete. Engine setup, encoder flush, archive compression and atomic save are still indivisible synchronous calls and cannot be interrupted while executing.
- Completion is based on the provider's Saved event, not encoder completion. Shows path/name, extension, encoded byte count for single-file output, OK, Open My Tracks, Export Again and native mobile Share/Send. Web clearly reports a download handoff, not verification that the browser saved it. Folder stem exports do not fabricate a total size.
- Share reuses the existing Android/iOS provider; completed stem batches share the actual files, not their directory. Android declares SEND/SEND_MULTIPLE package queries and reports when no receiving app is available. Device chooser/recipient behavior still requires physical Android validation.
- Render/build/encode errors provide details and a retry/settings action. Final write failures identify the stage. Sharing errors do not relabel a successfully exported file as a failed render.
- WAV/FLAC/MP3/OGG use their existing tested encoders. The settings page now names the actual existing quality preset (WAV 16-bit/float32; FLAC 16/24-bit; MP3 192/320 kbps; Vorbis quality 0.4/1.0). Independent bit-depth/bitrate/mono and export-specific sample-rate controls are NOT added: output remains stereo at project/engine rate. No nonfunctional controls are displayed.
- Encoded output is still buffered in memory, as before. A fully asynchronous, cancellable finalization/write pipeline and independent format settings remain follow-up work; this is not a claim that the entire requested redesign or phone verification is complete.

BanaForge 0.5.17 — clip-local volume envelope waveform display

- The existing Audio Editor VOL points now reshape the real waveform in both the editor and Playlist. During point drags and curve-popup edits the editor shows the preview immediately; the Playlist refreshes from the committed clip.
- Select VOL point(s), then Set shape for Linear / Smooth / Square / Stairs and tension. Drag points horizontally to change fade duration, vertically to change Level. One undo edit on gesture/popup completion. No separate Playlist automation clip is required.
- Audio VOL shapes are saved in HDSh and used by the existing per-clip playback envelope. PAN/SPEED shapes are unchanged. Overlapping clips retain independent sampler pads and envelope data.
- Cached volume evaluation uses the same quantized automation events and sampler Level law as playback; envelope values replace base Level rather than multiplying it. Base Level is held before a late first point, including later loops; trimmed/offset clips initialize from their envelope at the offset.
- Cached source peaks and original PCM are never changed. Coarse waveform columns use a conservative peak×maximum-gain bound so short peaks are not dropped; they are not a sample-exact post-effect waveform. Above-unity heights are clipped to the waveform region; minimum Level retains the existing −60 dB floor, not digital silence.
- This is point-based editing, not automatic crossfades: dedicated edge fade handles, linked crossfades and live audio preview before gesture commit are not added. Speed-envelope time-warping and post-FX/channel/master gain are not represented in waveform geometry. Device performance/readability remains unverified.

BanaForge 0.5.16 — optional Playlist activity lights

- Settings: Track activity indicators, beside Continuous scrolling during playback. Off by default, saved independently; works during playback with continuous scrolling on or off.
- A small channel-tinted light at the right edge of each visible primary Playlist header follows the existing stereo channel-strip peak snapshot. Silence is dark; stronger levels are brighter. Stopping playback or disabling the option removes the lights.
- Reuses meter snapshots and normal UI refresh: no PCM scanning, audio commands, extra threads, refresh timers or DSP changes. Off-screen headers are culled before meter queries/drawing.
- Scope: one independent light per channel/primary track. Sub-tracks and automation lanes share channel outputs and are not presented as separately metered audio; per-lane metering is not implemented. This avoids falsely claiming isolated activity for lanes sharing a channel.
- Device FPS, visual placement and flash readability still need phone testing.

BanaForge 0.5.15 — progressive real-audio waveform LOD

- Real audio waveform detail now follows source-audio duration per logical screen pixel, accounting for display density. Close zoom keeps one-device-pixel detail; wider views progress through 2/4/8/16/32-logical-pixel blocks instead of stopping at four device pixels. Coordinate systems named beats or frames now produce the same LOD for the same audio span.
- Covers arrangement audio clips (imported and recorded audio share this path), audio editor, file/sample preview, and the sample-waveform strip in the drum editor. Piano Roll notes, MIDI, synth rendering and procedural vocal thumbnails are not changed.
- Cached waveform meshes now share immutable vertex/index buffers via Arc rather than deep-copying them on redraw. The cache permits up to 256 small meshes within the existing 4 MiB geometry budget.
- Peak hierarchies preserve conservative min/max ranges at overview zoom. PCM, recording formats, sample rates, audio buffers, DSP and audio export are untouched.
- First-time decoding/summary generation is still synchronous. Physical-device FPS and thermal improvements remain unmeasured; no blanket lag-free or audio-stability guarantee.

BanaForge 0.5.14 — waveform visualization / mobile settings test build

- Waveform height now follows base sample-slot Level using the sampler's gain law, in arrangement clips and the audio editor. Updates visually without rewriting PCM. Above-unity peaks are clipped to the waveform drawing region. Volume automation, channel gain and post-FX loudness are not represented by this base-level display.
- Audio editor now uses the shared overview peak hierarchy and adaptive column density too. Arrangement and sample preview retain their existing LOD path. Detailed close zoom remains available.
- Fixes the actual mobile settings row filter that still hid Sound Mode. NATIVE / FAST / SAFE / ULTRA SAFE are now exposed on mobile; the raw slider remains desktop-only. This is distinct from the requested future visual performance profiles.
- Audio-reactive flashes, separate visual performance presets and offline MP4 export are NOT included. No placeholder Video Render button or screen-recording substitute is presented as working offline export.
- Physical-device performance/latency is not yet validated; broader settings-suite failures remain tracked.

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
