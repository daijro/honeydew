# Playlist VOL/PAN — implementation and device acceptance

## Architecture inspected first

Audio clips store independent `Event::Envelope` controls by clip and `env_param`: 0 = sampler Level (VOL), 1 = sampler Pan, 2 = Speed. The Audio Editor and Playlist now read/write these same controls via `song::points` and `audio_editor::apply`. There is no second envelope document, track-pan substitution, PCM rewrite, or separate Playlist-only automation.

The existing engine already schedules VOL/PAN events into each audio clip's own sampler pad. The old shape extension `HDSh` belongs to VOL on audio clips. PAN now has independent `pan_shapes` in the optional `HDPn` project chunk, with the same Linear, Square, Smooth and Stairs shapes/tension. Older projects have no `HDPn` and remain linear; readers unaware of the extension can ignore it. Standard envelope events remain written to EVN2. VOL retains its original HDSh meaning.

Before this change, edits to audio-clip lanes caused a playback re-sync, and an Audio Editor drag was only audible after release. VOL/PAN-only changes now qualify for no-resync engine handover when clip timing, sample, speed, lane ordering and other state remain unchanged. New prepared curves take over the old sampler's sounding voices/sample phase; the new envelope values are restored at the actual current content tick. No sample launch, transport restart, or output-stream restart is part of that path. Other structural/timing/speed edits retain their existing re-sync rules.

Live gestures use the existing Audition edit funnel: changed frames update audio without closing the undo transaction, and release creates one final undo step. Focus-loss/pinch cancellation restores the original Playlist/Audio Editor drag points. An external undo or deletion wins over an in-progress Playlist drag. Leaving an editor finalizes an open live gesture. The normal app-level engine preparation/handover still has finite work: this is not a guarantee of zero callback delay, no clicks from intentionally instantaneous gain changes, or device freeze-free operation.

## Playlist use

- Audio clips show only meaningfully changed envelope lanes: yellow volume and cyan pan by default, with no clip text labels. Untouched clips have no envelope overlays. The waveform retains its full area. The PAN center guide appears only for an active PAN envelope. Top PAN means right, bottom means left, middle means centered balance. The indication is normalized sampler gain/balance control, not a dB-linear volume scale.
- Select a clip, then double-tap its envelope area to arm both lanes for point creation. Colored lane outlines indicate armed editing. Tap empty lane space to add, drag an existing point to move. Existing points on selected clips can be dragged without separately arming point creation.
- Tap the label again to exit point-creation mode. Ordinary clip moves remain available outside the lanes; inactive empty lanes retain ordinary body behavior.
- Hold a point (or right-click on desktop) for the existing curve-shape/tension popup. Duplicate-tick controls and Square/Stairs risers show vertical changes at the engine's rounded event ticks.
- Compact rows use translucent lane backgrounds over the waveform; taller rows reserve a separate waveform area above the two lanes. Collapsed tracks retain their compact name-only behavior. Increase track-height zoom for fine touch editing on compact rows.
- VOL continues to shape waveform amplitude using the pre-existing cached envelope/peak pipeline. Changing an envelope never decodes/replaces the PCM or rebuilds source peak summaries.
- The line samples the same quantized automation-event law that playback uses, including the base value before a delayed first point. Curves use a bounded 1 MiB/64-entry display-event cache separate from the existing source/mesh caches. Visible geometry is sampled by visible pixels; stepped transitions retain exact event times. The display is a zoom-dependent summary, not a sample-by-sample output-meter trace.

## Automated verification

`.github/workflows/clip-envelopes.yml` runs focused tests for:

- Actual PCM left/center/right PAN output; simultaneous gain and balance; linear/smooth/instant/stepped changes.
- Independent overlapping clips (combined PCM equals their separate renders added together at safe levels).
- Live no-resync handover preserving deterministic sample phase while changing gain/PAN.
- PAN curve persistence through project write/read and identical rendered output after reopening.
- Playlist point creation, shared data, live drag, cancel, duration preservation and grouped history.
- Audio Editor PAN live transactions and cancellation, plus existing VOL/Audio Editor regressions.
- Display values matching prepared audio events and cache reuse; loop/offset mapping.
- Existing song-point, FLM, audio handover, Playlist interaction and offline-render regressions.

The release benchmark compares eight cached waveform meshes with and without both envelope lanes, at 2/32/128 controls per lane. Fonts/curve caches are warmed, then three repeats measure 64 egui frame-paint batches and 64 CPU tessellations each. It reports actual vertices/indices and incremental CPU work, not Android FPS/GPU execution. See the accompanying measurements document and workflow artifact for measured results.

## Physical Android acceptance (not established by CI)

1. Install the release APK; import mono and stereo audio. Select a clip and arm the lanes using either label. Confirm labels/colors, center guide, waveform visibility, point hit-testing and normal header/body clip movement at several track heights.
2. Draw VOL fade-in/fade-out; test Square changes and duplicate-tick rises/drops. Confirm both audible amplitude and waveform scaling. Shape and tension edits must match in the Audio Editor.
3. Move PAN left → center → right, both slowly and abruptly. Listen with headphones and verify left/right output meters/recorded PCM; clip PAN must not move another clip or track/master PAN.
4. Combine VOL and PAN on several overlapping clips, with offsets/repeats/reversal. Check each clip's independent points, audible balance and gain.
5. While playing, drag points and shape curves from both editors. Confirm transport/sample position continues and live output follows the edit. A deliberately instantaneous gain/PAN jump may click; that is not an output-stream restart.
6. Undo/redo complete gestures, interrupt drags with a pinch/focus loss, switch editors mid-gesture, save/reopen the project and verify both curves and sound.
7. Profile the same heavy project/device/zoom/thermal state with Android frame tracing and audio diagnostics. Desktop synthetic paint/tessellation results do not establish sustained phone responsiveness or audible synchronization.

## Display preferences (0.5.23)

Settings → Edit contains Show Playlist envelopes and VOL / PAN colors. The compact right-side panel includes a waveform preview, independent color swatches (including white, no black preset), #RRGGBB input, and reset. HEX accepts arbitrary user colors, including black if explicitly typed. The panel scrolls on short displays. Preferences are global and persist independently of project files. Hiding overlays also hides editing outlines and blocks new hidden point gestures; ongoing transactions still finalize normally. Audio Editor remains available for editing while Playlist overlays are hidden.

Envelope visibility is derived without allocating from that clip's control events: a lane is meaningful if any control differs from the clip's base sampler parameter. Explicit default-valued points remain saved but are not drawn. No whole-project scan, waveform-cache invalidation, PCM mutation, or engine command occurs for color/visibility changes. Untouched clips return before preparing curves or producing geometry.
