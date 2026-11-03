//! A drag left running when its editor stops drawing (F9's mixer, `Nav::Show(Playlist)`) ends there
//! ([`HoneydewApp::left_view`]): the piano roll's dragged key stops and its move is one final edit, which reaches the
//! engine; the automation and audio editors' drags end with no edit, also once the editor shows again.

use super::*;
use crate::history::Step;
use crate::song::Event;
use crate::song::points::{self, PointKey};
use crate::ui::preview::ClipKey;

fn rect(rig: &Rig, id: egui::Id) -> egui::Rect {
    rig.ctx.read_response(id).expect("drawn").rect
}

fn notes(rig: &Rig) -> Vec<Event> {
    let clips = &rig.app.song.channels[1].lanes[0].clips;
    clips.iter().flat_map(|c| c.events.clone()).collect()
}

fn key_of(rig: &Rig) -> u8 {
    match notes(rig)[..] {
        [Event::Note { key, .. }] => key,
        ref other => panic!("{other:?}"),
    }
}

/// The synth's (1) roll with a note a tap added at its grid's left middle, pressed and dragged three rows up (its key
/// three up sounds at 0.7), still held there; the log empty. The note's first key, and where the finger is.
fn roll_drag() -> (Rig, Pos2, u8) {
    let mut rig = Rig::with_synth();
    rig.edit(1);
    rig.slide_split();
    let grid = rect(&rig, egui::Id::new("piano_roll").with("grid"));
    let at = pos2(grid.left() + 4.0, grid.center().y);
    rig.click(at);
    let key = key_of(&rig);
    rig.frame(vec![]);
    rig.frame(vec![]);
    rig.take();
    rig.press(at);
    let to = at + vec2(0.0, -48.0);
    rig.drag_to(at + vec2(0.0, -24.0));
    rig.drag_to(to);
    let log = rig.take();
    assert!(log.contains(&format!("NoteOn 1 {} 89", key + 3)), "{log:?}");
    assert!(!log.iter().any(|c| c.starts_with("ReplaceSong")), "{log:?}");
    (rig, to, key)
}

#[test]
fn a_roll_drag_the_mixer_or_the_playlist_takes_away_stops_its_key_and_closes_its_move() {
    for how in ["F9", "Show(Playlist)"] {
        let (mut rig, to, key) = roll_drag();
        if how == "F9" {
            rig.frame(vec![key_event(egui::Key::F9, egui::Modifiers::NONE)]);
        } else {
            rig.app.navigate(Nav::Show(View::Playlist));
            rig.frame(vec![]);
        }
        assert_ne!(rig.app.nav.view, View::PianoRoll(1), "{how}");
        assert_eq!(
            rig.take(),
            [
                format!("NoteOff 1 {}", key + 3),
                "ReplaceSong adopting".into()
            ],
            "{how}"
        );
        // The release while the roll is away ends nothing more.
        rig.release(to);
        rig.frame(vec![]);
        assert_eq!(rig.take(), Vec::<String>::new(), "{how}");
        assert_eq!(key_of(&rig), key + 3, "{how}");
        // The move was one undo step: one undo puts the note back on its row.
        let ctx = rig.ctx.clone();
        rig.app.step(&ctx, Step::Undo, 0);
        assert_eq!(key_of(&rig), key, "{how}");
    }
}

#[test]
fn an_automation_or_audio_drag_whose_editor_goes_lands_nowhere_when_it_comes_back() {
    for audio in [false, true] {
        let mut rig = Rig::new();
        let (key, name, kind) = if audio {
            rig.app.add_channel(ChannelKind::AudioClip, 0);
            rig.app.nav.overlays.clear();
            let key = ClipKey {
                channel: 1,
                lane: 0,
                id: 1,
            };
            (key, "audio_editor", PointKey::Envelope(0))
        } else {
            let key = ClipKey {
                channel: 0,
                lane: 0,
                id: 1,
            };
            (key, "automation_editor", PointKey::Automation)
        };
        rig.app.song.channels[key.channel].lanes[0].add_clip(0, 4.0);
        rig.app.song_edited(SongEdit::Final, 0);
        rig.click(OUTSIDE);
        rig.app.navigate(Nav::EditClip(key));
        rig.slide_split();
        // A point a tap adds, then a drag of it the editor stops seeing: shown again, nothing has moved.
        let grid = rect(&rig, egui::Id::new(name).with("grid"));
        let at = pos2(grid.left() + 64.0, grid.center().y);
        rig.click(at);
        rig.frame(vec![]);
        let pts = |rig: &Rig| points::points(key.clip(&rig.app.song).expect("the clip"), kind);
        let before = pts(&rig);
        assert_eq!(before.len(), 1, "{name}");
        rig.take();
        rig.press(at);
        rig.drag_to(at + vec2(64.0, 0.0));
        rig.drag_to(at + vec2(128.0, 20.0));
        rig.app.navigate(Nav::Show(View::Playlist));
        rig.frame(vec![]);
        rig.drag_to(at + vec2(300.0, 40.0));
        rig.app.navigate(Nav::EditClip(key));
        rig.slide_split();
        rig.release(at + vec2(300.0, 40.0));
        rig.frame(vec![]);
        assert_eq!(pts(&rig), before, "{name}");
        let log = rig.take();
        assert!(
            !log.iter().any(|c| c.starts_with("ReplaceSong")),
            "{name}: {log:?}"
        );
    }
}

/// The synth's (1) roll on a one-bar clip with a note (key 60, velocity 204) at tick 0, its graph lane shown (`Vel`),
/// in the undo history; the log empty. The lane's value area.
fn lane_rig() -> (Rig, egui::Rect) {
    let mut rig = Rig::with_synth();
    rig.app.song.channels[1].lanes[0].add_clip(0, 4.0).events = vec![Event::Note {
        pos: 0,
        len_beats: 1.0,
        key: 60,
        velocity: Event::DEFAULT_VELOCITY,
        pan: Event::DEFAULT_PAN,
        slide: false,
    }];
    rig.app.song_edited(SongEdit::Final, 0);
    rig.edit(1);
    rig.slide_split();
    // The lane's knob on the editor's bottom edge, at 0.75 U.
    rig.click(pos2(0.75 * U, PANEL_TOP - 4.0));
    rig.frame(vec![]);
    rig.take();
    let lane = rect(&rig, egui::Id::new("piano_roll").with("graph"));
    (rig, lane)
}

fn velocity(rig: &Rig) -> u8 {
    match notes(rig)[..] {
        [Event::Note { velocity, .. }] => velocity,
        ref other => panic!("{other:?}"),
    }
}

/// A Vel paint the playlist takes away mid-drag is one final edit: an undo step, heard at once, and one undo puts the
/// velocity back.
#[test]
fn a_lane_paint_the_playlist_takes_away_is_one_undo_step_and_reaches_the_engine() {
    let (mut rig, lane) = lane_rig();
    let steps = rig.app.history.steps();
    let x = lane.left() + 10.0;
    rig.press(pos2(x, lane.center().y));
    rig.drag_to(pos2(x, lane.top()));
    assert_eq!(velocity(&rig), 255);
    assert!(!rig.take().iter().any(|c| c.starts_with("ReplaceSong")));
    rig.app.navigate(Nav::Show(View::Playlist));
    rig.frame(vec![]);
    // As its release: a velocity-only edit takes no re-sync.
    assert_eq!(rig.take(), ["ReplaceSong adopting no-resync"]);
    assert_eq!(rig.app.history.steps(), steps + 1);
    rig.release(pos2(x, lane.top()));
    rig.frame(vec![]);
    assert_eq!(rig.take(), Vec::<String>::new());
    rig.frame(z(egui::Modifiers::NONE));
    assert_eq!(velocity(&rig), Event::DEFAULT_VELOCITY);
    assert_eq!(rig.app.history.steps(), steps);
}

/// A lane touch the playlist takes away that changed nothing (none painted, or painted back to where it was)
/// re-syncs as its release would (`SongEdit::Resync`), with no undo step.
#[test]
fn a_lane_touch_that_changed_nothing_and_is_taken_away_re_syncs() {
    for back in [false, true] {
        let (mut rig, lane) = lane_rig();
        let steps = rig.app.history.steps();
        let x = lane.left() + 10.0;
        // Past the note's beat (nothing to paint), or up to the top over the note and back to its 204.
        let at = if back {
            let v204 = lane.bottom() - lane.height() * 204.5 / 255.0;
            rig.press(pos2(x, v204));
            rig.drag_to(pos2(x, lane.top()));
            rig.drag_to(pos2(x, v204));
            pos2(x, v204)
        } else {
            let at = pos2(lane.right() - 10.0, lane.center().y);
            rig.press(at);
            at
        };
        assert_eq!(velocity(&rig), Event::DEFAULT_VELOCITY, "back {back}");
        rig.take();
        rig.app.navigate(Nav::Show(View::Playlist));
        rig.frame(vec![]);
        assert_eq!(rig.take(), ["ReplaceSong adopting"], "back {back}");
        assert_eq!(rig.app.history.steps(), steps, "back {back}");
        rig.release(at);
        rig.frame(vec![]);
        assert_eq!(rig.take(), Vec::<String>::new(), "back {back}");
    }
}

#[test]
fn a_key_column_press_the_roll_hid_sounds_nothing_when_a_roll_shows_again_until_pressed_anew() {
    // The hide latches the key column's press as stale, so a roll shown under that finger (the same one again, or
    // another channel's: every roll's key column has one id) plays nothing until it lifts and presses again.
    for to in [1, 2] {
        let mut rig = Rig::with_synth();
        rig.app
            .add_channel(ChannelKind::Synth(SynthKind::MiniSynth), 0);
        rig.click(OUTSIDE);
        rig.edit(1);
        rig.slide_split();
        let at = rect(&rig, egui::Id::new("piano_roll").with("keys")).center();
        rig.take();
        rig.press(at);
        let on = rig.take();
        assert!(
            on.len() == 1 && on[0].starts_with("NoteOn 1 "),
            "{to}: {on:?}"
        );
        let off = on[0]
            .rsplit_once(' ')
            .unwrap()
            .0
            .replace("NoteOn", "NoteOff");
        if to == 1 {
            rig.app.navigate(Nav::Show(View::Playlist));
            rig.frame(vec![]);
            assert_eq!(rig.take(), std::slice::from_ref(&off));
        }
        rig.edit(to);
        rig.slide_split();
        // Channel 2's new clip goes to the engine.
        let log: Vec<String> = (rig.take().into_iter())
            .filter(|c| c.starts_with("NoteO"))
            .collect();
        let expected: &[String] = if to == 1 {
            &[]
        } else {
            std::slice::from_ref(&off)
        };
        assert_eq!(log, expected, "{to}: shown under the held press");
        rig.release(at);
        rig.frame(vec![]);
        assert_eq!(rig.take(), Vec::<String>::new(), "{to}: the release");
        rig.press(at);
        let on = rig.take();
        assert!(
            on.len() == 1 && on[0].starts_with(&format!("NoteOn {to} ")),
            "{to}: a new press {on:?}"
        );
        rig.release(at);
        assert_eq!(rig.take().len(), 1, "{to}");
    }
}

#[test]
fn a_key_column_press_held_as_the_editor_follows_the_current_clip_to_another_roll_plays_nothing_there()
 {
    // The editor follows the current clip within the frame (app.rs `Some(Some(view))`), and the new roll
    // draws before `left_view`: as the playlist's keyboard `open` on channel 2's clip would, the mouse still on
    // channel 1's key column. The press is stale there at once.
    let mut rig = Rig::with_synth();
    rig.app
        .add_channel(ChannelKind::Synth(SynthKind::MiniSynth), 0);
    rig.click(OUTSIDE);
    rig.edit(2);
    rig.slide_split();
    rig.edit(1);
    rig.slide_split();
    let at = rect(&rig, egui::Id::new("piano_roll").with("keys")).center();
    rig.take();
    rig.press(at);
    let on = rig.take();
    assert!(on.len() == 1 && on[0].starts_with("NoteOn 1 "), "{on:?}");
    let off = on[0]
        .rsplit_once(' ')
        .unwrap()
        .0
        .replace("NoteOn", "NoteOff");
    let id = rig.app.song.channels[2].lanes[0].clips[0].id;
    let clip = crate::ui::clips::ClipRef {
        channel: 2,
        lane: 0,
        id,
    };
    let app = &mut rig.app;
    (app.main_screen.clips).make_current(&mut app.song, &mut app.selected_channel, clip);
    rig.frame(vec![]);
    rig.frame(vec![]);
    assert_eq!(rig.app.nav.view, View::PianoRoll(2));
    assert_eq!(rig.take(), std::slice::from_ref(&off));
    rig.release(at);
    rig.frame(vec![]);
    assert_eq!(rig.take(), Vec::<String>::new(), "the release");
}

#[test]
fn a_playlist_clip_drag_the_mixer_takes_away_ends_where_it_is_as_one_final_edit() {
    // The drag's ongoing moves set the song-changed flag, and F9 `showMainMixer` mid-drag ends it there, at the
    // place it moved the clip to, as its release would (`left_view`): one undo step, which reaches the engine.
    // The release under the mixer and the playlist shown again move nothing more.
    let mut rig = Rig::with_synth();
    rig.app.song.channels[1].lanes[0].add_clip(0, 4.0);
    rig.app.song_edited(SongEdit::Final, 0);
    rig.frame(vec![]);
    rig.app.files.saved();
    let steps = rig.app.history.steps();
    let start = |rig: &Rig| rig.app.song.channels[1].lanes[0].clips[0].start_tick;
    // The clip's middle, a beat being 38.4 points from x 48.
    let at = pos2(48.0 + 2.0 * 38.4, header(1).y);
    // A tap makes it current, which a press then drags at once.
    rig.click(at);
    for _ in 0..5 {
        rig.frame(vec![]);
    }
    rig.app.files.saved();
    rig.press(at);
    for beats in 1..=4 {
        rig.drag_to(at + vec2(beats as f32 * 0.5 * 38.4, 0.0));
    }
    let moved = start(&rig);
    assert_ne!(moved, 0, "dragged");
    assert!(rig.app.files.modified(), "the ongoing moves");
    rig.take();
    rig.frame(vec![key_event(egui::Key::F9, egui::Modifiers::NONE)]);
    assert_eq!(rig.app.nav.view, View::Mixer);
    let log = rig.take();
    assert!(log.iter().any(|c| c.starts_with("ReplaceSong")), "{log:?}");
    assert_eq!(rig.app.history.steps(), steps + 1);
    rig.drag_to(at + vec2(12.0 * 38.4, 0.0));
    rig.release(at + vec2(12.0 * 38.4, 0.0));
    rig.app.navigate(Nav::Show(View::Playlist));
    rig.frame(vec![]);
    assert_eq!(start(&rig), moved);
    assert_eq!(rig.take(), Vec::<String>::new());
    assert_eq!(rig.app.history.steps(), steps + 1);
}

#[test]
fn a_clip_dragged_onto_its_channels_sub_track_moves_there_as_one_undo_step() {
    // Clips drag between tracks of any channel by lane kind. The synth's current clip,
    // dragged a row down onto the synth's second notes track: one undo step that sets the song-changed flag and
    // re-syncs the engine, and the clip is still the current clip, on that track.
    let mut rig = Rig::with_synth();
    let synth = &mut rig.app.song.channels[1];
    let sub = synth.add_lane(crate::song::LaneKind::Notes);
    synth.lanes[0].add_clip(0, 4.0);
    rig.app.song_edited(SongEdit::Final, 0);
    rig.frame(vec![]);
    let at = pos2(48.0 + 2.0 * 38.4, header(1).y);
    rig.click(at);
    for _ in 0..5 {
        rig.frame(vec![]);
    }
    let steps = rig.app.history.steps();
    let before = rig.app.song.clone();
    // Onto the sub-track and back where it started: nothing changed, no undo step.
    let to = pos2(at.x, header(2).y);
    rig.press(at);
    rig.drag_to(to);
    rig.drag_to(at);
    rig.release(at);
    assert_eq!(rig.app.history.steps(), steps);
    assert!(rig.app.song.channels[1].same_edit(&before.channels[1]));
    rig.app.files.saved();
    rig.take();
    rig.press(at);
    rig.drag_to(at.lerp(to, 0.5));
    rig.drag_to(to);
    rig.release(to);
    let lanes = &rig.app.song.channels[1].lanes;
    assert!(lanes[0].clips.is_empty());
    assert_eq!(lanes[sub].clips.len(), 1);
    assert_eq!(rig.app.history.steps(), steps + 1);
    assert!(rig.app.files.modified());
    let log = rig.take();
    assert!(log.iter().any(|c| c.starts_with("ReplaceSong")), "{log:?}");
    let current =
        (rig.app.main_screen.clips).clip_for(&rig.app.song, rig.app.selected_channel, false);
    assert_eq!(current.map(|r| (r.channel, r.lane)), Some((1, sub)));
}

#[test]
fn hiding_the_page_stops_the_rolls_dragged_key_its_key_columns_key_and_a_new_notes_sound() {
    // The web's hide runs no frame (`on_hidden`): what the shown roll sounds stops at once, as the roll's hide does,
    // and nothing of it again once the page is back.
    let back = || {
        vec![
            egui::Event::WindowFocused(false),
            egui::Event::WindowFocused(true),
        ]
    };
    let quiet = |rig: &mut Rig, what: &str| {
        for _ in 0..5 {
            rig.frame(vec![]);
        }
        let log = rig.take();
        assert!(
            !log.iter().any(|c| c.starts_with("NoteO")),
            "{what}: {log:?}"
        );
    };
    // A note's drag.
    let (mut rig, to, key) = roll_drag();
    rig.app.on_hidden();
    assert_eq!(rig.take(), [format!("NoteOff 1 {}", key + 3)]);
    rig.frame(back());
    rig.release(to);
    quiet(&mut rig, "the drag");
    // The key column's key.
    let keys = rect(&rig, egui::Id::new("piano_roll").with("keys")).center();
    rig.press(keys);
    let on = rig.take();
    assert!(on.len() == 1 && on[0].starts_with("NoteOn 1 "), "{on:?}");
    let off = on[0]
        .rsplit_once(' ')
        .unwrap()
        .0
        .replace("NoteOn", "NoteOff");
    rig.app.on_hidden();
    assert_eq!(rig.take(), [off]);
    rig.frame(back());
    rig.release(keys);
    quiet(&mut rig, "the key column");
    // A new note's sound, within its gate.
    let mut rig = Rig::with_synth();
    rig.edit(1);
    rig.slide_split();
    let grid = rect(&rig, egui::Id::new("piano_roll").with("grid"));
    rig.take();
    rig.click(pos2(grid.left() + 4.0, grid.center().y));
    let on: Vec<String> = (rig.take().into_iter())
        .filter(|c| c.starts_with("NoteO"))
        .collect();
    assert!(on.len() == 1 && on[0].starts_with("NoteOn 1 "), "{on:?}");
    let off = on[0]
        .rsplit_once(' ')
        .unwrap()
        .0
        .replace("NoteOn", "NoteOff");
    rig.app.on_hidden();
    assert_eq!(rig.take(), [off]);
    rig.frame(back());
    quiet(&mut rig, "the new note");
}
