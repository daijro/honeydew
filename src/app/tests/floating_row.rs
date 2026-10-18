//! The floating row in the three editors, in the app, the same in each: a zoom hides it, a release on the selection
//! brings it back. A tap on a showing selection deselects it, so a long press brings it back.

use super::*;

/// A two-finger pinch by 1.5 around `at`, which keeps the point under it in place.
fn zoom(rig: &mut Rig, at: Pos2) {
    rig.two_finger_zoom(at);
}

/// A long press at `at`: released in place 0.4 s after the press (the rig's frames are 0.1 s apart).
fn long_press(rig: &mut Rig, at: Pos2) {
    rig.press(at);
    for _ in 0..3 {
        rig.frame(vec![]);
    }
    rig.release(at);
}

/// Selects with `select` at `at`, then checks the row shows, a zoom hides it and a long press at the same place
/// (still on the selection) brings it back.
fn hides_and_comes_back(rig: &mut Rig, editor: &str, at: Pos2, select: fn(&mut Rig, Pos2)) {
    select(rig, at);
    rig.frame(vec![]);
    assert!(rig.painted("Copy"), "{editor}: shown");
    zoom(rig, at);
    assert!(!rig.painted("Copy"), "{editor}: hidden by the zoom");
    rig.frame(vec![]);
    assert!(!rig.painted("Copy"), "{editor}: still hidden");
    long_press(rig, at);
    rig.frame(vec![]);
    assert!(rig.painted("Copy"), "{editor}: back");
}

#[test]
fn every_editor_hides_its_floating_row_on_a_zoom_and_shows_it_again_on_the_selection() {
    let mut rig = Rig::with_drums_and_synth();
    // The playlist: a tap on a clip selects it (on the synth, so that the drum editor's grid stays as it was).
    rig.app.song.channels[2].lanes[0].add_clip(0, 16.0);
    rig.frame(vec![]);
    hides_and_comes_back(&mut rig, "playlist", lane(2), Rig::click);
    // The piano roll: a tap on an empty cell adds a note, a long press selects it. Near the grid's left edge, so
    // that the zoom's scroll stays in range and the note under the pointer.
    rig.edit(2);
    rig.slide_split();
    // The note starts on the step nearest the tap (up to 8 points right of it) and is a beat (64 points) long.
    let cell = pos2(60.0, 200.0);
    rig.click(cell);
    hides_and_comes_back(&mut rig, "piano roll", cell + vec2(10.0, 0.0), long_press);
    // The drum editor: a tap on a step adds a hit, a long press selects it.
    rig.edit(1);
    rig.slide_split();
    let step = rig.step(0, 2);
    rig.click(step);
    hides_and_comes_back(&mut rig, "drum editor", step, long_press);
}

#[test]
fn the_playlists_row_waits_at_the_grids_right_edge_while_the_selected_copy_is_past_it() {
    // Horizontally off-screen, the row is held at the viewport edge (x = … right edge − W). The
    // grid is x 48..1170; a beat is 38.4 points; the 5-button row is 256 wide (51.2 a button), its buttons 44.8 across.
    let mut rig = Rig::with_drums_and_synth();
    rig.app.song.channels[1].lanes[0].add_clip(24 * crate::song::PPQ, 4.0);
    rig.frame(vec![]);
    // Beat 24 is x 969.6: the clip spans x 969.6..1123.2.
    rig.click(pos2(1000.0, header(1).y));
    rig.frame(vec![]);
    let copy = egui::Id::new(("clip_button", 0_usize));
    let row_left = |rig: &Rig| (rig.ctx.read_response(copy)).map(|r| r.rect.center().x - 25.6);
    assert_eq!(
        row_left(&rig),
        Some(1000.0 - 128.0),
        "centred on the finger"
    );
    // Copy puts the copy right after the clip and selects it: beat 28 (still partly in view), then beat 32
    // (x 1276.8, past the grid). The row stays shown, the finger where it was.
    for _ in 0..2 {
        rig.click_id(copy);
        rig.frame(vec![]);
    }
    let starts: Vec<u32> = (rig.app.song.channels[1].lanes[0].clips.iter())
        .map(|c| c.start_tick / crate::song::PPQ)
        .collect();
    assert_eq!(starts, [24, 28, 32]);
    rig.frame(vec![]);
    assert_eq!(
        row_left(&rig),
        Some(1170.0 - 256.0),
        "held at the right edge"
    );
}

#[test]
fn folding_the_drum_mixer_out_hides_the_step_row_until_a_tap_on_the_selection() {
    // The fold animates the header column (3 U + c·12 U) and so the grid's left edge: a change of the grid rect,
    // which hides the row at once and keeps it hidden after the fold settles.
    let mut rig = Rig::new();
    rig.app.add_default_drums(0);
    rig.edit(1);
    rig.slide_split();
    let hit = rig.step(0, 2);
    rig.click(hit);
    long_press(&mut rig, hit);
    rig.frame(vec![]);
    assert!(rig.painted("Copy"), "shown over the selected hit");
    let grid = egui::Id::new("step_seq").with("grid");
    let grid_left = |rig: &Rig| rig.ctx.read_response(grid).expect("drawn").rect.left();
    let collapsed = grid_left(&rig);
    // The 3 U header column, then the t gap (ceil(U/16) px).
    assert_eq!(collapsed, 3.0 * U + 2.0);
    // A tap on the current row's header folds the mixer out; frames 1/60 s apart show it on its way (c in (0, 1)).
    let row = rig.app.slots.drum_editor.step.selected;
    rig.click_id(egui::Id::new("step_seq").with(("row", row)));
    let mut cs = Vec::new();
    for _ in 0..4 {
        rig.next_frame_at(rig.time + 1.0 / 60.0);
        rig.frame(vec![]);
        cs.push(rig.app.slots.drum_editor.step.fold.c);
        assert!(
            !rig.painted("Copy"),
            "hidden while the grid's edge moves: c {cs:?}"
        );
    }
    assert!(cs.windows(2).all(|w| w[0] < w[1]), "{cs:?}");
    assert!(cs.iter().all(|&c| 0.0 < c && c < 1.0), "{cs:?}");
    for _ in 0..10 {
        rig.frame(vec![]);
    }
    assert_eq!(grid_left(&rig), 15.0 * U + 2.0);
    assert!(!rig.painted("Copy"), "still hidden once the fold settles");
    // A long press on the selected hit, where the narrower grid now puts it, brings the row back; the hit stays
    // selected (a tap would delete it).
    let hit = rig.step(0, 2);
    long_press(&mut rig, hit);
    rig.frame(vec![]);
    assert!(
        rig.painted("Copy"),
        "back after a long press on the selection"
    );
}

#[test]
fn a_tap_on_an_overlay_over_the_selection_brings_no_row_back() {
    // The rack draws in the editors' layer. A selected clip spans the grid under it; a tap on the rack's background
    // over the clip is the rack's, not the clip's.
    let mut rig = Rig::with_drums_and_synth();
    rig.app.song.channels[1].lanes[0].add_clip(0, 64.0);
    rig.frame(vec![]);
    let at = pos2(300.0, header(1).y);
    rig.click(at);
    rig.frame(vec![]);
    assert!(rig.painted("Copy"), "shown");
    zoom(&mut rig, at);
    assert!(!rig.painted("Copy"), "hidden by the zoom");
    rig.app.navigate(Nav::Open(Overlay::Rack(2)));
    rig.slide_rack();
    rig.click(pos2(645.0, header(1).y));
    rig.frame(vec![]);
    assert!(!rig.painted("Copy"), "still hidden");
    // The same long press with the rack closed brings it back (a tap on the selected current clip deselects it).
    rig.app.navigate(Nav::Close);
    // Sliding out, the rack resizes the editor's grid, which hides the row: the long press comes once it is out.
    rig.slide_rack();
    long_press(&mut rig, pos2(645.0, header(1).y));
    rig.frame(vec![]);
    assert!(rig.painted("Copy"), "back");
}

/// With a selection at `at` and its row shown: a zoom there hides the row, and a tap at `at` with the rack open over it
/// is the rack's, which brings no row back. The rack is closed again after.
fn a_tap_on_the_rack_is_not_the_editors(rig: &mut Rig, editor: &str, at: Pos2) {
    // The rig's rack: x 530..1170, down to the panel.
    let rack = egui::Rect::from_min_max(pos2(530.0, 0.0), pos2(1170.0, PANEL_TOP));
    assert!(rack.contains(at), "{editor}: under the rack");
    rig.frame(vec![]);
    assert!(rig.painted("Copy"), "{editor}: shown");
    zoom(rig, at);
    assert!(!rig.painted("Copy"), "{editor}: hidden by the zoom");
    rig.app.navigate(Nav::Open(Overlay::Rack(2)));
    rig.slide_rack();
    rig.click(at);
    rig.frame(vec![]);
    assert!(!rig.painted("Copy"), "{editor}: still hidden");
    rig.app.navigate(Nav::Close);
    // Sliding out, the rack resizes the editor's grid, which hides the row: the tap comes once it is out.
    rig.slide_rack();
}

#[test]
fn a_tap_on_the_rack_over_a_selection_brings_no_row_back_in_the_piano_roll() {
    // The piano roll's floating row takes `covered` too. With the rack closed, the same tap brings the row back.
    let mut rig = Rig::with_drums_and_synth();
    rig.edit(2);
    rig.slide_split();
    let cell = pos2(700.0, 200.0);
    rig.click(cell);
    // On the note: it starts on the step nearest the tap (up to 8 points right of it) and is a beat long.
    let on = cell + vec2(10.0, 0.0);
    long_press(&mut rig, on);
    a_tap_on_the_rack_is_not_the_editors(&mut rig, "piano roll", on);
    long_press(&mut rig, on);
    rig.frame(vec![]);
    assert!(rig.painted("Copy"), "back");
}

#[test]
fn a_tap_on_the_rack_over_a_selection_brings_no_row_back_in_the_step_view() {
    // The step view's floating row takes `covered` too. With the rack closed, the same tap brings the row back.
    let mut rig = Rig::with_drums_and_synth();
    rig.edit(1);
    rig.slide_split();
    // Under the rack: step 20, past the first bar (the tap grows the loop to two bars).
    let step = rig.step(0, 20);
    rig.click(step);
    long_press(&mut rig, step);
    a_tap_on_the_rack_is_not_the_editors(&mut rig, "step view", step);
    long_press(&mut rig, step);
    rig.frame(vec![]);
    assert!(rig.painted("Copy"), "back");
}

#[test]
fn a_tap_on_the_rack_over_a_selection_brings_no_row_back_in_the_drum_note_grid() {
    // The free note-grid's floating row takes `covered` too. (There a tap on the selected hit deselects it, so no
    // tap with the rack closed follows.)
    use crate::song::{DrumGrid, LaneKind};
    let mut rig = Rig::with_drums_and_synth();
    let drums = &mut rig.app.song.channels[1];
    let lane = drums.add_lane(LaneKind::Drums);
    drums.lanes[lane].grid = DrumGrid::Notes;
    drums.lanes[lane].add_clip(0, 16.0);
    rig.app.song_edited(SongEdit::Final, 0);
    let key = ClipKey {
        channel: 1,
        lane,
        id: 1,
    };
    rig.app.navigate(Nav::EditClip(key));
    rig.slide_split();
    rig.frame(vec![]);
    let grid = (rig
        .ctx
        .read_response(egui::Id::new("drum_grid").with("grid")))
    .unwrap()
    .rect;
    // A tap adds a hit (at 1 point a tick: 608..640, x 704..736), a tap on it selects it.
    let hit = pos2(710.0, grid.top() + U);
    rig.click(hit);
    rig.click(hit);
    a_tap_on_the_rack_is_not_the_editors(&mut rig, "drum note grid", hit);
}

#[test]
fn a_tap_on_a_clip_of_another_track_shows_the_row_over_it() {
    // The selection moves to another track row, which is no scroll of the track list: the tap's release, a frame
    // after its press, still shows the row.
    let mut rig = Rig::with_drums_and_synth();
    for channel in [1, 2] {
        rig.app.song.channels[channel].lanes[0].add_clip(0, 8.0);
    }
    rig.frame(vec![]);
    for track in [1, 2, 1] {
        rig.click(pos2(100.0, header(track).y));
        rig.frame(vec![]);
        assert!(
            rig.painted("Copy"),
            "track {track}: shown over the tapped clip"
        );
    }
}

#[test]
fn a_scroll_of_the_track_list_hides_the_row_whether_by_a_few_points_or_by_whole_rows() {
    // The key is the list's scroll, not what is in view: a scroll by exactly a row (2 U = 64) puts the same tops in
    // view, with other rows.
    let mut rig = Rig::new();
    for _ in 0..6 {
        let kind = ChannelKind::Synth(SynthKind::MiniSynth);
        rig.app.add_bare_channel(kind, 0);
    }
    rig.app.song.channels[1].lanes[0].add_clip(0, 8.0);
    rig.frame(vec![]);
    let mut y = header(1).y;
    rig.click(pos2(100.0, y));
    rig.frame(vec![]);
    assert!(rig.painted("Copy"), "shown");
    for by in [10.0, 64.0] {
        rig.app.main_screen.scroll_y += by;
        y -= by;
        rig.frame(vec![]);
        assert!(!rig.painted("Copy"), "hidden by a scroll of {by}");
        rig.frame(vec![]);
        assert!(!rig.painted("Copy"), "{by}: it stays hidden");
        long_press(&mut rig, pos2(100.0, y.max(header(0).y)));
        rig.frame(vec![]);
        assert!(rig.painted("Copy"), "{by}: back on a long press");
    }
}
