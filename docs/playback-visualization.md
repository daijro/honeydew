# Playback visualization and export scope

## Implemented in the 0.5.14 test revision

- Arrangement and sample previews already use adaptive min/max geometry and a shared peak hierarchy. Audio-editor waveforms now use the hierarchy and adaptive column density as well. These are display summaries, never edits to decoded PCM or rendered audio. Procedural Vocal Studio thumbnails are separate and do not become a claim of real imported-bank output.
- Arrangement and audio-editor waveform heights use `drums::level_gain` on the sample slot's base Level parameter. Geometry is clipped to its region. Cached PCM peaks do not depend on gain, so changing gain does not regenerate source summaries. This is **base Level**, not envelope automation, channel faders, post-FX output, LUFS or a prediction of mastered loudness.
- The platform row filter no longer hides mobile Sound Mode. Host tests explicitly pass mobile platform flags through that same filter. This corrects a previous implementation gap: testing only the preset list was insufficient.

## Audio-reactive playback indicator — not implemented

Use existing engine-published channel meters and transport snapshots, not sample decoding or synthesis from the UI. Only visible clips/notes intersecting the playback band should participate. A channel meter is mixed channel output, not independently metered per-clip audio: label that distinction and do not present note velocity as actual loudness. Meter values need finite-value handling, smoothing/release and bounded intensity. Gate the effect on continuous scrolling and an explicit visual-effects preference; support disabling it and reduced motion. Prefer a soft tint/band over full-contrast strobing. Disabled/stopped views should schedule no additional frames. Test muted/solo clips, silence, peaks, simultaneous notes, seek/loop and unsupported meters.

## Visual performance presets — not implemented

Keep these independent of audio Sound Mode and persist a separate preference:

- NATIVE: balanced visual refresh/detail.
- FAST: higher visual refresh target, with higher resource use.
- SAFE: stricter visible-region work and conservative visual detail.
- ULTRA SAFE: reduce periodic playhead/meter/animation refresh substantially, preserving immediate input feedback where practical.

All modes must cull invisible geometry, not reserve correct culling for Safe. A repaint timer alone is **not a hard FPS cap** because active animations/widgets can independently request frames. Before shipping these labels, audit those repaint sources and implement a coordinated visual scheduler. Never sleep, block, allocate or change callback buffers on the audio thread to enforce a visual profile. This can reduce UI competition but cannot guarantee underrun-free audio on overloaded hardware.

## Offline Video Render — not implemented

Requires a deterministic offline transport clock, egui offscreen frame rendering, offline audio rendering, timestamped MP4 video/audio encoding and muxing, cancellation/cleanup, device storage and platform share handling. Proposed options: MP4, resolution/frame rate/quality, project or loop range, progress, completion preview, Save and Share. Frame N must show the same project time as its corresponding audio samples, including tempo changes, loops and render tails. No wall-clock playback or screen-record permission should be required for this offline path. Define missing-resource policy, export-size limits and memory/thermal budgets on mobile. Do not ship a decorative Video Render button or call a realtime screen recorder offline rendering.

Existing audio export and project editing should remain intact. Codec licensing and redistribution need review before selecting a bundled encoder. Physical-device FPS, thermal, dropped-frame, A/V sync and audio-underrun measurements remain release gates; automated compilation does not substitute for them.
