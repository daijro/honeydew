//! Song edits while playing through the app's funnel: an edit's engine takes over the playing one's sound; undo and
//! redo always re-sync.

use super::*;
use crate::song::Event;
use honeydew_audio::device::AudioRenderer;

fn note(beat: u32, len_beats: f64, key: u8) -> Event {
    Event::Note {
        pos: beat * 128,
        len_beats,
        key,
        velocity: Event::DEFAULT_VELOCITY,
        pan: Event::DEFAULT_PAN,
        slide: false,
    }
}

/// The MiniSynth rig playing A3 (beats 0 to 4) and E4 (beat 3) from beat 0, one beat in, its log empty.
fn playing() -> Rig {
    let mut rig = Rig::with_synth();
    let clip = rig.app.song.channels[1].lanes[0].add_clip(0, 8.0);
    clip.events.extend([note(0, 4.0, 57), note(3, 1.0, 64)]);
    rig.app.song_edited(SongEdit::Final, 0);
    rig.frame(vec![]);
    rig.click(PLAY);
    rig.frame(vec![]);
    {
        let mut rec = rig.rec.0.borrow_mut();
        let engine = rec.engine.as_mut().expect("an engine");
        engine.render(&mut vec![0.0; 24_000 * 2], 2);
    }
    rig.frame(vec![]);
    rig.take();
    rig
}

/// Deleting E4 re-syncs (a removal); its undo brings it back, an addition, which re-syncs too as an undo.
#[test]
fn an_undo_while_playing_re_syncs_and_keeps_the_engine() {
    let mut rig = playing();
    rig.app.song.channels[1].lanes[0].clips[0].events.pop();
    rig.app.song_edited(SongEdit::Final, 128);
    rig.frame(vec![]);
    assert_eq!(rig.take(), ["ReplaceSong adopting", "Play"]);
    rig.frame(z(egui::Modifiers::NONE));
    assert_eq!(rig.app.song.channels[1].lanes[0].clips[0].events.len(), 2);
    assert_eq!(rig.take(), ["ReplaceSong adopting", "Play"]);
    let rec = rig.rec.0.borrow();
    assert_eq!(rec.engine.as_ref().unwrap().sounding(1), 1 << 57);
}

/// In the piano roll, a tap on empty grid creates a note with no re-sync; Paste adds notes too, and re-syncs.
#[test]
fn a_tap_created_note_does_not_re_sync_but_a_paste_does() {
    let mut rig = playing();
    rig.edit(1);
    rig.slide_split();
    rig.take();
    let notes = |rig: &Rig| rig.app.song.channels[1].lanes[0].clips[0].events.len();
    // Far right of the grid, past the notes, left of the rack tab's 1 U radius.
    let grid = rig
        .ctx
        .read_response(egui::Id::new("piano_roll").with("grid"));
    let grid = grid.expect("the roll drew").rect;
    rig.click(pos2(grid.right() - 2.0 * U, grid.center().y));
    assert_eq!(notes(&rig), 3);
    let log = rig.take();
    assert!(
        log.contains(&"ReplaceSong adopting no-resync".into()),
        "{log:?}"
    );
    for key in [egui::Key::A, egui::Key::C, egui::Key::V] {
        rig.frame(key_tap(key, egui::Modifiers::COMMAND));
    }
    assert_eq!(notes(&rig), 6);
    let log = rig.take();
    assert!(log.contains(&"ReplaceSong adopting".into()), "{log:?}");
    assert!(!log.iter().any(|c| c.ends_with("no-resync")), "{log:?}");
}
