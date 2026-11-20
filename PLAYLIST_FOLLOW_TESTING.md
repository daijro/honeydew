# BanaForge 0.5.20 — adjustable Playlist follow position

## Usage

1. Enable **Settings → Continuous scrolling during playback**.
2. In the Playlist ruler, drag the small yellow **↔** handle horizontally.
3. The follow target can be set from 5% to 95% of the visible timeline width; the original default is 25%.

The handle is a screen-space view preference, not a seek control. The existing playhead box and ruler taps outside this dedicated handle retain their navigation behavior. The handle remains available while stopped if continuous scrolling is enabled, but automatic scrolling still runs only during playback. Disable continuous scrolling to hide the handle and restore ordinary Playlist scrolling. The chosen target remains saved for the next time it is enabled, including app restarts.

The Playlist's existing limits still apply: it does not scroll before project time zero, and songs that fit entirely in the viewport do not force-scroll. Near these limits the real playhead can therefore appear away from the target. The handle always marks the desired screen position, not an invented playback time. Other editors retain their original follow positions.

## Implementation boundary

Only the Playlist's horizontal viewport offset is computed differently: `scroll = actual_playback_beat - target_fraction * visible_beat_span`, constrained to the existing scroll range. It uses the same transport snapshot and visual timing lead as before. There are no new Seek, Play, Stop, audio restart, DSP or export calls. Drag ownership suppresses the overlapping range selector, seek knob and viewport gestures for that complete gesture, including release; other ruler gestures are unchanged.

The app saves `playlistFollowPosition` in thousandths, independently of the on/off toggle. Legacy settings default to 250; loaded values are constrained to 50–950. No per-frame audio work is added.

## Automated checks

The Playlist follow cursor validation workflow exercises:

- Left/right dragging and offset changes at a fixed real playback tick, without emitting seek/playhead/edit actions.
- Unchanged playback flag, channel/clip data and loop during adjustment.
- Several zoom levels and large project positions, with scroll-range clamps.
- Toggle disable/re-enable retention and settings save/load/default/range validation.
- Existing Playlist clip editing, selection, ruler gestures, scrolling and timeline regressions.
- Existing offline-render and audio-device regressions.

Check the workflow result for the tested SHA; this file is not itself a passing result.

## Physical Android acceptance test (not run in this environment)

- Play a long project with audio clips, synth notes, automation and effects. Drag the yellow handle left/right repeatedly. Listen for discontinuities and verify the ruler/playhead stays on the same audible beat without restarting.
- Repeat at close/far zoom and with expanded track headers, rack open, portrait/landscape, and different UI scales.
- Tap/long-press elsewhere in the ruler, select/move clips, scroll vertically and pinch-zoom. Verify those operations still work and do not inadvertently adjust the anchor.
- Disable continuous scrolling, navigate/scroll normally, re-enable, switch away from/back to Playlist, then restart the app. Verify the selected target returns.
- Test focus loss, a second finger during adjustment, and project start/end limits. Do not interpret scroll clamping as loss of audio synchronization.

No physical Android audio or touchscreen timing validation is claimed by the automated build.
