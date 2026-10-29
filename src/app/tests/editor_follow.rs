//! The bottom editor follows the playlist's current clip (the editor area also collapses whenever the current track has
//! no current clip; a tap makes a clip current, a double tap or `Edit` opens it), and the playlist and the editor are
//! focus containers of their own (the X moves the focus back).

use super::*;
use crate::ui::clips::ClipRef;
use crate::ui::preview;

impl Rig {
    /// `Edit` on `channel`, the setup of the tests that open an editor on a channel: a main lane with no clip first
    /// gets one the app's way (`+Clip`'s one bar at song tick 0, [`crate::ui::clips::add_clip`]), as its first note or
    /// hit made it before the editor followed the playlist: left for the next edit to take into the history. The audio
    /// has it at once, as after `+Clip` (a new clip re-syncs): the next edit is that edit alone.
    pub(super) fn edit(&mut self, channel: usize) {
        let bar = self.app.song.time_sig.bar_beats();
        let lane = &mut self.app.song.channels[channel].lanes[0];
        if lane.clips.is_empty() {
            crate::ui::clips::add_clip(lane, 0.0, 1.0, bar);
            self.app.rebuild_as(0, crate::audio_link::Handover::Edit);
        }
        self.app.navigate(Nav::Edit(channel));
    }

    /// The current clip: the current track's own (its lane's DESc@24).
    pub(super) fn current_clip(&self) -> Option<ClipRef> {
        (self.app.main_screen.clips).clip_for(&self.app.song, self.app.selected_channel, false)
    }
}

/// The clip `id` of `channel`'s main lane.
fn key(channel: usize, id: u32) -> ClipKey {
    ClipKey {
        channel,
        lane: 0,
        id,
    }
}

/// The view, and the clip its editor edits.
fn shown(rig: &Rig) -> (View, Option<ClipKey>) {
    let view = rig.app.nav.view;
    (view, preview::current(view, &rig.app.slots, &rig.app.song))
}

/// The point `beat` beats into track row `row` of the playlist (a beat is 38.4 points from x 48).
fn at(row: usize, beat: f32) -> Pos2 {
    pos2(48.0 + beat * 38.4, header(row).y)
}

/// Drums (1) with a clip at beat 0, MiniSynth (2) with clips at beats 0 (a note in it) and 8, in the undo history.
fn rig() -> Rig {
    let mut rig = Rig::with_drums_and_synth();
    rig.app.song.channels[1].lanes[0].add_clip(0, 4.0);
    let synth = &mut rig.app.song.channels[2].lanes[0];
    synth.add_clip(0, 4.0).events = vec![crate::song::Event::Note {
        pos: 0,
        len_beats: 1.0,
        key: 60,
        velocity: crate::song::Event::DEFAULT_VELOCITY,
        pan: crate::song::Event::DEFAULT_PAN,
        slide: false,
    }];
    synth.add_clip(8 * 128, 4.0);
    rig.app.song_edited(SongEdit::Final, 0);
    rig.frame(vec![]);
    rig
}

/// [`rig`] with the piano roll open on the synth's first clip, the split moved to keep the playlist 0.6 of the view (it
/// opens fully expanded), so that its first three rows show over the editor.
fn open_rig() -> Rig {
    let mut rig = rig();
    rig.app.navigate(Nav::EditClip(key(2, 1)));
    rig.frame(vec![]);
    rig.app.main_screen.split.target = 0.6;
    rig.slide_split();
    assert_eq!(shown(&rig), (View::PianoRoll(2), Some(key(2, 1))));
    rig
}

#[test]
fn the_bottom_editor_follows_the_playlists_current_clip() {
    let mut rig = open_rig();
    // `Edit` makes its clip the current one, and its track current.
    let current = |rig: &Rig| rig.current_clip();
    assert_eq!(
        current(&rig),
        Some(ClipRef {
            channel: 2,
            lane: 0,
            id: 1
        })
    );
    // A tap on another clip makes it current: the open editor follows it, on the same channel and to another one.
    rig.click(at(2, 10.0));
    assert_eq!(shown(&rig), (View::PianoRoll(2), Some(key(2, 2))));
    rig.click(at(1, 2.0));
    assert_eq!(shown(&rig), (View::DrumEditor(1), Some(key(1, 1))));
    assert_eq!(current(&rig).map(|r| r.channel), Some(1));
    // A tap on a clip while the editor is collapsed opens nothing.
    rig.app.navigate(Nav::Collapse);
    rig.frame(vec![]);
    rig.click(at(2, 2.0));
    assert_eq!(shown(&rig), (View::Playlist, None));
}

#[test]
fn the_keyboards_select_clip_retargets_the_editor() {
    let mut rig = open_rig();
    // A long press on the synth's other clip selects it without making it current, and puts the grid cursor and the
    // focus there.
    let other = at(2, 10.0);
    rig.press(other);
    for _ in 0..3 {
        rig.frame(vec![]);
    }
    rig.release(other);
    let current = |rig: &Rig| rig.current_clip().map(|r| r.id);
    assert_eq!(current(&rig), Some(1));
    assert_eq!(shown(&rig), (View::PianoRoll(2), Some(key(2, 1))));
    // `open` is `Select clip` there: it becomes current, and the editor follows.
    rig.frame(key_tap(egui::Key::Enter, egui::Modifiers::NONE));
    assert_eq!(current(&rig), Some(2));
    assert_eq!(shown(&rig), (View::PianoRoll(2), Some(key(2, 2))));
}

#[test]
fn edit_on_a_clip_that_is_not_current_makes_it_current() {
    let mut rig = open_rig();
    rig.app.navigate(Nav::EditClip(key(1, 1)));
    rig.frame(vec![]);
    assert_eq!(shown(&rig), (View::DrumEditor(1), Some(key(1, 1))));
    let current = rig.current_clip();
    assert_eq!(current.map(|r| (r.channel, r.id)), Some((1, 1)));
    assert_eq!(
        rig.app.selected_channel, 1,
        "its channel takes the keyboard"
    );
}

#[test]
fn with_no_current_clip_on_the_current_track_the_editor_collapses_and_the_rack_stays() {
    let mut rig = open_rig();
    rig.app.navigate(Nav::Open(Overlay::Rack(2)));
    rig.slide_rack();
    // A tap on the drums' empty lane space makes that track current: it has no current clip.
    rig.click(at(1, 8.0));
    assert_eq!(shown(&rig), (View::Playlist, None));
    // On the drums, now current (the open rack follows the current channel).
    assert_eq!(rig.overlays(), [Overlay::Rack(1)]);
    // The synth's track keeps its current clip; the split is the playlist's alone.
    assert_eq!(rig.app.song.channels[2].lanes[0].file.current_clip_id, 1);
    rig.frame(vec![]);
    assert_eq!(rig.app.main_screen.split.f, 1.0);
}

#[test]
fn edit_on_a_clip_less_lane_does_nothing() {
    let mut rig = Rig::with_drums_and_synth();
    for channel in [1, 2] {
        rig.app.navigate(Nav::Edit(channel));
        rig.frame(vec![]);
        assert_eq!(shown(&rig), (View::Playlist, None), "{channel}");
    }
    assert!((1..3).all(|c| rig.app.song.channels[c].lanes[0].clips.is_empty()));
    // With a clip, `Edit` opens it, as the clip's own `Edit` does.
    rig.app.song.channels[2].lanes[0].add_clip(0, 4.0);
    rig.app.navigate(Nav::Edit(2));
    rig.frame(vec![]);
    assert_eq!(shown(&rig), (View::PianoRoll(2), Some(key(2, 1))));
}

/// Drums (1) with clips 1 (beat 0), 2 (beat 8) and 3 (beat 16), each with one hit, in the undo history.
fn drum_rig() -> Rig {
    let mut rig = Rig::with_drums_and_synth();
    let lane = &mut rig.app.song.channels[1].lanes[0];
    for (start, pad) in [(0, 0), (8 * 128, 1), (16 * 128, 2)] {
        lane.add_clip(start, 4.0).events = vec![crate::song::Event::Drum {
            pos: 0,
            len_beats: 0.25,
            pad,
            velocity: 200,
            pan: 127,
            value: 0x7FFF,
            repeat: 0,
        }];
    }
    rig.app.song_edited(SongEdit::Final, 0);
    rig.frame(vec![]);
    rig
}

fn delete(rig: &mut Rig, id: u32) {
    (rig.app.song.channels[1].lanes[0].clips).retain(|c| c.id != id);
    rig.app.song_edited(SongEdit::Final, 0);
    rig.frame(vec![]);
}

#[test]
fn deleting_the_open_drum_clip_collapses_instead_of_jumping_to_the_next() {
    let mut rig = drum_rig();
    rig.app.navigate(Nav::EditClip(key(1, 1)));
    rig.slide_split();
    assert_eq!(shown(&rig), (View::DrumEditor(1), Some(key(1, 1))));
    delete(&mut rig, 1);
    assert_eq!(shown(&rig), (View::Playlist, None));
}

#[test]
fn the_drum_editor_follows_its_clip_by_id() {
    let mut rig = drum_rig();
    rig.app.navigate(Nav::EditClip(key(1, 2)));
    rig.slide_split();
    assert_eq!(shown(&rig), (View::DrumEditor(1), Some(key(1, 2))));
    assert!(drawn_hit(&rig, 1, 0) && !drawn_hit(&rig, 0, 0));
    // The clip before it goes: it moves down the lane's list (another takes its place), and the editor stays on it.
    delete(&mut rig, 1);
    assert_eq!(shown(&rig), (View::DrumEditor(1), Some(key(1, 2))));
    assert_eq!(rig.app.slots.drum_editor.step.clip, Some(2));
    // Drawn too: clip 2's hit (pad 1, step 0), not clip 3's (pad 2).
    rig.frame(vec![]);
    assert!(drawn_hit(&rig, 1, 0) && !drawn_hit(&rig, 2, 0));
}

/// The step grid drew a hit in step `step` of row `row`: a bar inside the cell, taller than wide.
fn drawn_hit(rig: &Rig, row: usize, step: usize) -> bool {
    let cell = (rig.app.slots.drum_editor.step)
        .cell_rect(row, step)
        .expect("the step view drew");
    rig.out.shapes.iter().any(|s| {
        matches!(&s.shape, egui::Shape::Rect(r)
            if r.rect != cell && cell.contains_rect(r.rect) && r.rect.height() > r.rect.width())
    })
}

#[test]
fn a_header_tap_on_a_track_with_a_current_clip_retargets_the_editor_and_preview() {
    // Each track keeps its own current clip (DESc@24): the editor and Preview follow the current track's.
    let mut rig = open_rig();
    rig.click(at(1, 2.0));
    rig.click(at(2, 2.0));
    assert_eq!(shown(&rig), (View::PianoRoll(2), Some(key(2, 1))));
    rig.click_text_button("SOLO CLIP");
    assert_eq!(rig.app.preview, Some(key(2, 1)));
    // The drums' header makes their track current: its current clip shows, and Preview moves to it.
    rig.click(header(1));
    rig.frame(vec![]);
    assert_eq!(rig.app.selected_channel, 1);
    assert_eq!(shown(&rig), (View::DrumEditor(1), Some(key(1, 1))));
    assert_eq!(rig.app.preview, Some(key(1, 1)));
    // Back on the synth's: its own again.
    rig.click(header(2));
    rig.frame(vec![]);
    assert_eq!(shown(&rig), (View::PianoRoll(2), Some(key(2, 1))));
}

#[test]
fn preview_goes_off_when_the_editor_collapses_for_want_of_a_current_clip_but_not_on_the_x() {
    // The collapse takes Preview's button and leaves it nothing to play.
    let mut rig = open_rig();
    rig.click_text_button("SOLO CLIP");
    assert_eq!(rig.app.preview, Some(key(2, 1)));
    // The drums' clip was never made current: their header tap collapses the editor, and Preview goes off.
    rig.click(header(1));
    assert!(!rig.painted("SOLO CLIP"));
    rig.frame(vec![]);
    assert_eq!(shown(&rig), (View::Playlist, None));
    assert_eq!(rig.app.preview, None);
    // The X keeps the clip current, and Preview on it.
    rig.app.navigate(Nav::EditClip(key(2, 1)));
    rig.slide_split();
    rig.click_text_button("SOLO CLIP");
    rig.click_id(egui::Id::new("editor_close"));
    rig.frame(vec![]);
    assert_eq!(rig.app.nav.view, View::Playlist);
    assert_eq!(rig.app.preview, Some(key(2, 1)));
}

#[test]
fn after_the_x_preview_follows_the_current_tracks_current_clip_and_goes_off_without_one() {
    // Preview plays the current track's current clip whatever the view shows.
    let mut rig = open_rig();
    rig.click(at(1, 2.0));
    rig.click(at(2, 2.0));
    rig.click_text_button("SOLO CLIP");
    rig.click_id(egui::Id::new("editor_close"));
    rig.frame(vec![]);
    let state = |rig: &Rig| (rig.app.nav.view, rig.app.preview);
    assert_eq!(state(&rig), (View::Playlist, Some(key(2, 1))));
    // The drums' header: their current clip, where Preview moves though no editor shows it.
    rig.click(header(1));
    rig.frame(vec![]);
    assert_eq!(state(&rig), (View::Playlist, Some(key(1, 1))));
    // The master's track, with no current clip: Preview goes off (a first tap may only deselect).
    for _ in 0..2 {
        rig.click(at(0, 20.0));
    }
    rig.frame(vec![]);
    assert_eq!(state(&rig), (View::Playlist, None));
}

#[test]
fn a_loaded_songs_stored_current_clip_is_its_tracks_current_clip() {
    let mut rig = Rig::with_drums_and_synth();
    let lane = &mut rig.app.song.channels[1].lanes[0];
    lane.add_clip(0, 4.0);
    lane.add_clip(8 * 128, 4.0);
    lane.file.current_clip_id = 2;
    // Through the file, as a song opens: the drums (1) selected.
    let bytes = crate::flm::write(&rig.app.song).unwrap();
    rig.app.song = crate::flm::read(&bytes).unwrap();
    rig.app.song_edited(SongEdit::Replaced, 0);
    rig.frame(vec![]);
    let current = ClipRef {
        channel: 1,
        lane: 0,
        id: 2,
    };
    assert_eq!(rig.current_clip(), Some(current));
    rig.app.navigate(Nav::Edit(1));
    rig.slide_split();
    assert_eq!(shown(&rig), (View::DrumEditor(1), Some(key(1, 2))));
}

#[test]
fn a_tracks_current_clip_is_no_undo_step_nor_unsaved_change_and_undo_keeps_it() {
    let mut rig = rig();
    rig.app.files.saved();
    let steps = rig.app.history.steps();
    rig.click(at(2, 10.0));
    rig.frame(vec![]);
    assert_eq!(rig.current_clip().map(|r| r.id), Some(2));
    assert_eq!(rig.app.history.steps(), steps);
    assert!(!rig.app.files.modified());
    // An edit, its undo and its redo: the current clip stays where the tap put it.
    rig.app.song.tempo_bpm = 150.0;
    rig.app.song_edited(SongEdit::Final, 0);
    for keys in [z(egui::Modifiers::NONE), z(egui::Modifiers::SHIFT)] {
        rig.frame(keys);
        assert_eq!(rig.current_clip().map(|r| r.id), Some(2));
    }
    rig.frame(z(egui::Modifiers::NONE));
    // The edit's undo step set the song-changed flag; its undo leaves it set.
    assert!(rig.app.files.modified());
}

#[test]
fn select_clip_on_another_tracks_clip_opens_its_editor_on_its_channel() {
    let mut rig = open_rig();
    // A long press on the drums' clip selects it (the current track stays the synth's) and puts the cursor there.
    let drums = at(1, 2.0);
    rig.press(drums);
    for _ in 0..3 {
        rig.frame(vec![]);
    }
    rig.release(drums);
    assert_eq!(shown(&rig), (View::PianoRoll(2), Some(key(2, 1))));
    rig.frame(key_tap(egui::Key::Enter, egui::Modifiers::NONE));
    assert_eq!(shown(&rig), (View::DrumEditor(1), Some(key(1, 1))));
    assert_eq!(rig.app.selected_channel, 1);
}

#[test]
fn with_no_editor_and_nothing_focused_delete_removes_the_selected_playlist_clip() {
    let mut rig = rig();
    rig.click(at(2, 10.0));
    assert!(rig.app.main_screen.clips.selected.is_some());
    focus::clear(&rig.ctx);
    rig.frame(vec![]);
    assert_eq!(focus::focused(&rig.ctx), None);
    rig.frame(key_tap(egui::Key::Delete, egui::Modifiers::NONE));
    assert_eq!(rig.app.song.channels[2].lanes[0].clips.len(), 1);
}

/// The ids of the focus containers.
fn playlist() -> egui::Id {
    egui::Id::new(focus::PLAYLIST)
}

fn editor() -> egui::Id {
    egui::Id::new(focus::EDITOR)
}

#[test]
fn the_editing_keys_go_to_the_focused_container_and_at_the_root_to_the_open_editor() {
    let mut rig = open_rig();
    let notes = |rig: &Rig| rig.app.song.channels[2].lanes[0].clips[0].events.len();
    // Nothing focused: the open editor holds them. Ctrl+A, Delete: the clip's note goes, the clip stays.
    assert_eq!(focus::focused(&rig.ctx), None);
    rig.frame(key_tap(egui::Key::A, egui::Modifiers::COMMAND));
    rig.frame(key_tap(egui::Key::Delete, egui::Modifiers::NONE));
    assert_eq!(notes(&rig), 0);
    rig.frame(z(egui::Modifiers::NONE));
    assert_eq!(notes(&rig), 1);
    // A tap on the clip in the playlist selects it and puts the focus there: Delete deletes the clip.
    rig.click(at(2, 2.0));
    rig.frame(vec![]);
    assert!(focus::within(&rig.ctx, playlist()));
    assert!(rig.app.main_screen.clips.selected.is_some());
    rig.frame(key_tap(egui::Key::Delete, egui::Modifiers::NONE));
    let clips = |rig: &Rig| rig.app.song.channels[2].lanes[0].clips.len();
    assert_eq!(clips(&rig), 1);
    // The current clip went with it: the editor collapsed.
    assert_eq!(shown(&rig), (View::Playlist, None));
}

#[test]
fn a_tap_in_the_editor_gives_it_the_keys_over_the_playlists_selection() {
    let mut rig = open_rig();
    // The clip selected in the playlist, then a tap on the roll's grid: the focus is the editor's.
    rig.click(at(2, 2.0));
    assert!(rig.app.main_screen.clips.selected.is_some());
    rig.click(pos2(700.0, 330.0));
    rig.frame(vec![]);
    assert!(focus::within(&rig.ctx, editor()));
    let clips = |rig: &Rig| rig.app.song.channels[2].lanes[0].clips.len();
    rig.frame(key_tap(egui::Key::A, egui::Modifiers::COMMAND));
    rig.frame(key_tap(egui::Key::Delete, egui::Modifiers::NONE));
    assert_eq!(clips(&rig), 2, "the playlist's selection stays");
    let notes = rig.app.song.channels[2].lanes[0].clips[0].events.len();
    assert_eq!(notes, 0);
}

#[test]
fn the_close_x_moves_the_focus_from_the_editor_to_the_playlist() {
    let mut rig = open_rig();
    rig.click(pos2(700.0, 330.0));
    rig.frame(vec![]);
    assert!(focus::within(&rig.ctx, editor()));
    rig.click_id(egui::Id::new("editor_close"));
    rig.frame(vec![]);
    assert_eq!(rig.app.nav.view, View::Playlist);
    assert!(focus::within(&rig.ctx, playlist()));
    assert!(
        !focus::is_keyboard_mode(&rig.ctx),
        "a tap's move: no cursor"
    );
    // With no editor and the focus there, the playlist holds the keys: `focusEditor` focuses it too.
    let mut navs = Vec::new();
    let ctx = rig.ctx.clone();
    focus::clear(&ctx);
    rig.app.global(
        &ctx,
        crate::app::keys::Global::FocusEditor,
        &mut false,
        &mut navs,
    );
    rig.frame(vec![]);
    assert!(focus::within(&ctx, playlist()));
}

/// Editors open fully expanded. One opened while the playlist holds the focus (a clip tapped, then `Edit`) leaves it
/// nothing focused: the playlist, 0 high, is not drawn, so the focus goes to the root and the open editor takes the
/// editing keys.
#[test]
fn an_editor_opened_from_the_focused_playlist_takes_the_editing_keys() {
    let mut rig = rig();
    rig.click(at(2, 2.0));
    rig.frame(vec![]);
    assert!(focus::within(&rig.ctx, playlist()));
    rig.app.navigate(Nav::EditClip(key(2, 1)));
    rig.slide_split();
    assert_eq!(rig.app.main_screen.split.f, 0.0);
    assert!(!focus::within(&rig.ctx, playlist()));
    rig.frame(key_tap(egui::Key::A, egui::Modifiers::COMMAND));
    rig.frame(key_tap(egui::Key::Delete, egui::Modifiers::NONE));
    let lane = &rig.app.song.channels[2].lanes[0];
    assert_eq!((lane.clips.len(), lane.clips[0].events.len()), (2, 0));
}
