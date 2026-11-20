# Waveform Quality — 0.5.21

## Architecture and scope

Inspection before changes found a shared Playlist/picker/instrument-strip mesh cache (bounded to 4 MiB / 256 entries), a view-keyed Audio Editor peak cache, Slicer's posted peak summaries, and the optional Vocal Synth's asynchronously generated 48-peak note previews. Imported/mic PCM summaries and the power-of-two overview pyramid are built by the existing sample cache. This change does not invalidate those source caches, decode audio, or synthesize new voice previews when switching modes.

Settings → Waveform Quality sits immediately after Sound mode. High Quality is the backwards-compatible default, including when the field is absent in older preferences. Balanced groups three times the existing automatic zoom LOD; Performance groups eight times the automatic LOD and draws one-device-pixel-wide independent centered bars. Balanced retains adjacent filled columns. Full groups and the short last interval retain the original time transform and amplitude scale. Conservative overview summaries may include neighboring peaks, as before. The fixed 48-peak optional vocal preview groups existing peaks by maximum, retaining High's existing line style.

Policy is per egui context, published each frame. Shared mesh cache keys include quality, so returning to a mode can reuse its immutable Arc mesh. The Audio Editor's existing peak-view key naturally changes with group spacing. Slicer groups its already cached summaries; no source PCM rewrite or project mutation occurs. Display geometry is rebuilt only when needed by those existing rendering paths. A first mode switch still has finite UI-thread work; this is not a zero-cost or device freeze-free guarantee.

No audio engine, decoder, project serialization, transport, export, clip duration, gain DSP or timeline conversion is changed. The persisted setting belongs to application preferences, not song data. Fewer displayed groups intentionally sacrifice within-group visual detail, not playback accuracy.

## Automated checks

`.github/workflows/waveform-quality.yml` runs:

- Default/persistence/invalid-field recovery and live selector mapping.
- Per-context isolation and actual mesh vertex reduction across all three modes.
- Cache Arc reuse for each policy and source PCM/summary identity.
- Conservative transient preservation, reversal, device-pixel scaling, horizontal bounds and short tails.
- Audio Editor real painted-quad counts, separated Performance bars and unchanged song/source after switching back.
- Existing waveform mesh/envelope, Audio Editor, settings, picker, Slicer, Playlist interaction and offline-render regressions.
- Follow-handle grab outside its unchanged visible cap, plus existing release ownership and normal-ruler regressions.

## Controlled measurements

The release benchmark `waveform_quality_render_benchmark` is ignored in ordinary test runs. The workflow builds first, then runs three repeats for each quality at four zoom/density combinations, with eight stereo clip meshes per batch. It measures cold geometry generation, warm Arc cache lookups, and egui CPU tessellation separately. Vertex/index counts and raw geometry payload bytes describe actual renderer input, not estimates from setting names.

`tools/analyze-waveform-benchmark.py` checks that both simplified modes reduce actual geometry, reports medians and retains all individual runs. Workflow artifacts include host information, raw logs, a JSON table, process-resource report and Markdown results. Decode/summary generation is excluded from timed sections; process peak RSS includes the test harness and source allocation and is not per-mode application memory. No GPU hardware timing or Android FPS can be inferred from this desktop synthetic benchmark.

## Physical Android acceptance (not established by CI)

1. Install the arm64 release; open the same heavy project with imported/mic clips, stereo/reversed clips and clip envelopes. Compare the same visible zoom and running/stopped transport across all modes.
2. Confirm High's filled appearance; Balanced's lower detail; Performance's visibly independent vertical bars. Check Playlist, Audio Editor, picker preview, instrument strip, Slicer and enabled Vocal Synth previews.
3. Switch repeatedly while playing; verify audio/transport continuity, unchanged clip positions/durations and project content. Confirm preferences survive restart.
4. At fractional/high-density zoom, check isolated transients, tail alignment, reversal, loop boundaries and amplitude envelopes. No mode should horizontally stretch a clip.
5. Drag just outside the visible follow handle. Its expanded hit box is clipped to the ruler, without enlarging the painted cap. Verify normal seeking/range selection outside that target and cancellation when disabled, covered or pinched.
6. Profile the same project/device/thermal state with Android frame tracing and audio diagnostics before claiming device frame-time, power or responsiveness improvements. Automated CPU geometry/tessellation measurements do not replace this step.
