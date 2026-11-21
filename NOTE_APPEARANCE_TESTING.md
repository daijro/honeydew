# Piano Roll Note Appearance — v0.5.28

## Access

- Settings → User interface → Piano Roll Note Style.
- In Piano Roll, select a note → More… → Note Appearance… → Classic / Smooth / Outline / Gradient / Accent.
- The next rendered frame previews the choice. It is an app preference, not project/MIDI metadata. Reopening any roll uses it; settings storage preserves it across app restarts.

## Automated coverage

- Classic default and all five settings round trips; selector labels/store wiring.
- Decoration bounds for tiny/short/long notes, bounded shape count, four-vertex gradients, transparent outline interiors.
- Style switches leave song and selection unchanged and emit no song-edit action. Move/snap behavior tested across all five styles, plus existing Piano Roll selection, resize, overlap/topmost-hit, slide, graph editing, undo-related edit and vocal regression suites.
- Existing Classic geometry tests stay in place; no keyboard, grid, timeline or hit geometry code is changed.

## Android playtest (not claimed performed in CI)

1. Open an existing MIDI project with no new preference: verify Classic is visually unchanged.
2. Try all five styles on adjacent, overlapping, selected, unselected and slide notes; include bright/dark channel colors and both light/dark themes. Outline interiors should show the grid, not a background-colored fill. Ghosts remain dimmer.
3. Check tiny notes and long chords at minimum/maximum zoom, portrait/landscape and supported UI scales. Note names obey Show key names and are omitted only when they cannot fit.
4. Select, move, resize, copy/paste, delete, edit velocity/pan and undo/redo. Shapes must never alter touch targets or MIDI values. Compare saved/exported MIDI data before/after style-only changes.
5. Close/reopen the roll, switch tracks and restart the app: the preference should remain.
6. Compare the same dense project on a low-end device with Classic and each style; no textures, animation, timers or audio-thread work have been added. CI does not establish physical-device frame rate, touch responsiveness or battery impact.
