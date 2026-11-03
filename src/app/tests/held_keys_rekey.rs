//! Live keys held while the channels move: a chord keyboard's, typed or MIDI note ends on the channel it sounds
//! on, wherever a delete, insert or reorder, or the undo of one, moved that channel meanwhile (the engine moves live
//! keys by channel id). The typed and MIDI keys light their channel's piano keys there too.

use super::*;

/// The edit that moves B, the last of the master, A and B.
#[derive(Clone, Copy, Debug)]
enum Move {
    /// A goes: B is 1.
    Delete,
    /// A channel goes in before A: B is 3.
    Insert,
    /// B goes before A: B is 1.
    Reorder,
}

/// What holds the key.
#[derive(Clone, Copy, Debug)]
enum Source {
    /// The chord keyboard's top cell of strip 0: C3 major, inversion +3 (48 52 55).
    Chord,
    /// Z: 48.
    Typed,
    /// MIDI key 60.
    Midi,
}

fn apply(rig: &mut Rig, m: Move) {
    match m {
        Move::Delete => rig.app.delete_channel(1, 0),
        Move::Insert => {
            (rig.app).insert_channel(1, ChannelKind::Synth(SynthKind::MiniSynth), 0);
        }
        Move::Reorder => rig.app.set_playlist(&[0, 2, 1], 0),
    }
}

/// The chord keyboard's top cell of strip 0 on the default 5 U panel.
const CELL: Pos2 = pos2(1170.0 / 16.0, KEYS_TOP + 18.0);

/// The keys the commands `log` turn on or off (`on`), on the channel each names.
fn notes(log: &[String], on: bool) -> Vec<(usize, u8)> {
    let what = if on { "NoteOn " } else { "NoteOff " };
    let mut notes: Vec<(usize, u8)> = (log.iter())
        .filter_map(|c| c.strip_prefix(what))
        .map(|c| {
            let mut words = c.split(' ').map(|w| w.parse::<usize>().unwrap());
            (words.next().unwrap(), words.next().unwrap() as u8)
        })
        .collect();
    notes.sort_unstable();
    notes
}

/// Holds a key from `source` on B, moves B by `m` (or, with `undo`, moves it by `m` before the hold and undoes that
/// during it), and lets the key go: every note-off lands on B where it is now.
fn held_through(source: Source, m: Move, undo: bool) {
    let mut rig = Rig::with_synth();
    rig.app
        .add_channel(ChannelKind::Synth(SynthKind::MiniSynth), 0);
    let device = matches!(source, Source::Midi).then(|| rig.plug_midi());
    if let Source::Chord = source {
        rig.app.settings.keyboard_kind = KeyboardMode::Chords as u8;
    }
    let b = rig.app.song.channels[2].id;
    let at = |rig: &Rig| (rig.app.song.channels.iter()).position(|c| c.id == b);
    if undo {
        apply(&mut rig, m);
    }
    rig.app.selected_channel = at(&rig).unwrap();
    rig.frame(vec![]);
    rig.take();
    let hold = |rig: &mut Rig, down: bool| match source {
        Source::Chord => rig.button(CELL, down),
        Source::Typed => rig.type_key(egui::Key::Z, down),
        Source::Midi => {
            let message: &[u8] = if down {
                &[0x90, 60, 100]
            } else {
                &[0x80, 60, 0]
            };
            rig.midi(device.as_ref().unwrap(), &[message]);
        }
    };
    if let Source::Chord = source {
        rig.frame(vec![egui::Event::PointerMoved(CELL)]);
    }
    hold(&mut rig, true);
    let on = notes(&rig.take(), true);
    let keys: Vec<u8> = on.iter().map(|n| n.1).collect();
    assert!(
        !on.is_empty() && on.iter().all(|n| Some(n.0) == at(&rig)),
        "{source:?} {m:?}: {on:?}"
    );
    if undo {
        let ctx = rig.ctx.clone();
        rig.app.step(&ctx, Step::Undo, 0);
    } else {
        apply(&mut rig, m);
    }
    rig.frame(vec![]);
    rig.take();
    let now = at(&rig).unwrap();
    // The typed and MIDI keys light B's keys where it is now.
    if !matches!(source, Source::Chord) {
        let bits = keys.iter().fold(0_u128, |bits, &k| bits | 1 << k);
        assert_eq!(rig.app.sounding(now), bits, "{source:?} {m:?} undo {undo}");
    }
    hold(&mut rig, false);
    let off = notes(&rig.take(), false);
    let want: Vec<(usize, u8)> = keys.iter().map(|&k| (now, k)).collect();
    assert_eq!(off, want, "{source:?} {m:?} undo {undo}");
}

#[test]
fn a_held_chord_ends_on_its_channel_after_a_delete_insert_or_reorder_and_their_undos() {
    for m in [Move::Delete, Move::Insert, Move::Reorder] {
        for undo in [false, true] {
            held_through(Source::Chord, m, undo);
        }
    }
}

#[test]
fn a_held_typed_key_ends_on_its_channel_after_a_delete_insert_or_reorder_and_their_undos() {
    for m in [Move::Delete, Move::Insert, Move::Reorder] {
        for undo in [false, true] {
            held_through(Source::Typed, m, undo);
        }
    }
}

#[test]
fn a_held_midi_key_ends_on_its_channel_after_a_delete_insert_or_reorder_and_their_undos() {
    for m in [Move::Delete, Move::Insert, Move::Reorder] {
        for undo in [false, true] {
            held_through(Source::Midi, m, undo);
        }
    }
}

#[test]
fn a_held_key_whose_channel_is_deleted_stops_there_and_its_release_sends_nothing() {
    for source in [Source::Chord, Source::Typed, Source::Midi] {
        let mut rig = Rig::with_synth();
        rig.app
            .add_channel(ChannelKind::Synth(SynthKind::MiniSynth), 0);
        let device = matches!(source, Source::Midi).then(|| rig.plug_midi());
        if let Source::Chord = source {
            rig.app.settings.keyboard_kind = KeyboardMode::Chords as u8;
            rig.frame(vec![egui::Event::PointerMoved(CELL)]);
        }
        rig.take();
        let hold = |rig: &mut Rig, down: bool| match source {
            Source::Chord => rig.button(CELL, down),
            Source::Typed => rig.type_key(egui::Key::Z, down),
            Source::Midi => {
                let message: &[u8] = if down {
                    &[0x90, 60, 100]
                } else {
                    &[0x80, 60, 0]
                };
                rig.midi(device.as_ref().unwrap(), &[message]);
            }
        };
        hold(&mut rig, true);
        let on = notes(&rig.take(), true);
        // B itself goes: its notes stop in the engine they sound in, before the rebuild.
        rig.app.delete_channel(2, 0);
        let log = rig.take();
        let stop = log.iter().position(|c| c.starts_with("ReplaceSong"));
        let offs = notes(&log[..stop.unwrap_or(log.len())], false);
        assert_eq!(offs, on, "{source:?}: {log:?}");
        rig.frame(vec![]);
        rig.take();
        hold(&mut rig, false);
        assert_eq!(notes(&rig.take(), false), [], "{source:?}");
    }
}

#[test]
fn a_gated_note_off_lands_on_its_channel_after_a_delete() {
    // Z typed and let go within one frame on B; A goes before its tap gate ends.
    let mut rig = Rig::with_synth();
    rig.app
        .add_channel(ChannelKind::Synth(SynthKind::MiniSynth), 0);
    rig.frame(vec![]);
    rig.take();
    let both =
        [true, false].map(|down| super::typed::typed(egui::Key::Z, down, egui::Modifiers::NONE));
    rig.frame(both.to_vec());
    assert_eq!(rig.take(), ["NoteOn 2 48 89"]);
    rig.app.delete_channel(1, 0);
    rig.take();
    rig.next_frame_at(rig.time + 0.1 + TAP_GATE);
    rig.frame(vec![]);
    assert_eq!(notes(&rig.take(), false), [(1, 48)]);
}

#[test]
fn a_typed_key_held_on_a_deleted_channel_never_cuts_the_same_note_on_the_channel_now_in_its_place()
{
    // Q (60) held on B, `,` (60) on C; B goes, and C takes its index.
    let mut rig = Rig::with_synth();
    for _ in 0..2 {
        rig.app
            .add_channel(ChannelKind::Synth(SynthKind::MiniSynth), 0);
    }
    rig.app.selected_channel = 2;
    rig.frame(vec![]);
    rig.take();
    rig.type_key(egui::Key::Q, true);
    rig.app.selected_channel = 3;
    rig.type_key(egui::Key::Comma, true);
    assert_eq!(notes(&rig.take(), true), [(2, 60), (3, 60)]);
    rig.app.delete_channel(2, 0);
    let log = rig.take();
    assert_eq!(
        notes(&log, false),
        [(2, 60)],
        "B's note stops in the old engine: {log:?}"
    );
    rig.frame(vec![]);
    rig.take();
    rig.type_key(egui::Key::Q, false);
    assert_eq!(rig.take(), Vec::<String>::new(), "C's note sounds on");
    rig.type_key(egui::Key::Comma, false);
    assert_eq!(notes(&rig.take(), false), [(2, 60)]);
}

#[test]
fn a_note_on_and_off_around_a_channel_move_in_one_frame_still_waits_for_its_tap_gate() {
    // The frame's fresh notes follow their channels too.
    let mut rig = Rig::with_synth();
    rig.app
        .add_channel(ChannelKind::Synth(SynthKind::MiniSynth), 0);
    rig.frame(vec![]);
    rig.take();
    rig.app.key_on(2, 60, 0.5);
    rig.app.delete_channel(1, 0);
    rig.app.key_off(1, 60);
    assert_eq!(notes(&rig.take(), false), [], "gated");
    rig.next_frame_at(rig.time + 0.1 + TAP_GATE);
    rig.frame(vec![]);
    assert_eq!(notes(&rig.take(), false), [(1, 60)]);
}

#[test]
fn a_key_cut_at_play_stays_dark_on_its_channel_after_it_moves_and_its_release_sounds_nothing() {
    // Play cuts the live keys held, dark until let go; the cut follows its channel's move.
    let mut rig = Rig::with_synth();
    rig.app
        .add_channel(ChannelKind::Synth(SynthKind::MiniSynth), 0);
    rig.app.settings.count_in = false;
    rig.frame(vec![]);
    rig.take();
    rig.type_key(egui::Key::Z, true);
    assert_eq!(notes(&rig.take(), true), [(2, 48)]);
    rig.click(PLAY);
    assert!(rig.app.main_screen.playing);
    assert_eq!(rig.app.sounding(2), 0, "cut at Play");
    rig.app.delete_channel(1, 0);
    rig.frame(vec![]);
    let log = rig.take();
    assert_eq!(notes(&log, true), [], "{log:?}");
    assert_eq!(rig.app.sounding(1), 0, "still dark where B is now");
    rig.type_key(egui::Key::Z, false);
    rig.frame(vec![]);
    let log = rig.take();
    assert_eq!(notes(&log, true), [], "{log:?}");
    assert_eq!(rig.app.sounding(1), 0);
}

#[test]
fn a_key_column_press_held_as_its_rolls_channel_moves_sounds_on_until_its_release() {
    // The re-key moves the view last drawn with the view to draw, so the moved roll is no view
    // that went (`left_view`): its key column's key sounds on, and its release ends it.
    let mut rig = Rig::with_synth();
    rig.app
        .add_channel(ChannelKind::Synth(SynthKind::MiniSynth), 0);
    rig.click(OUTSIDE);
    rig.edit(2);
    rig.slide_split();
    let keys = egui::Id::new("piano_roll").with("keys");
    let at = rig.ctx.read_response(keys).expect("drawn").rect.center();
    rig.take();
    rig.press(at);
    let on = rig.take();
    assert!(on.len() == 1 && on[0].starts_with("NoteOn 2 "), "{on:?}");
    let key = on[0].split(' ').nth(2).unwrap().to_owned();
    rig.app.delete_channel(1, 0);
    rig.frame(vec![]);
    rig.frame(vec![]);
    assert_eq!(rig.app.nav.view, View::PianoRoll(1));
    let log = rig.take();
    assert!(!log.iter().any(|c| c.starts_with("NoteO")), "{log:?}");
    rig.release(at);
    rig.frame(vec![]);
    let log: Vec<String> = (rig.take().into_iter())
        .filter(|c| c.starts_with("NoteO"))
        .collect();
    assert_eq!(log, [format!("NoteOff 1 {key}")]);
}

#[test]
fn a_song_load_under_a_held_piano_key_or_chord_editor_button_sounds_nothing_until_pressed_again() {
    // The load stops every live note; a finger still on the piano or on the chord editor's button
    // holds nothing on the new song until it lifts and presses again.
    let seventh = pos2(1.5 * 1170.0 / 7.0, KEYS_TOP + 0.4 * U);
    for editor in [false, true] {
        let mut rig = Rig::with_synth();
        rig.app.settings.keyboard_kind = match editor {
            true => KeyboardMode::Chords,
            false => KeyboardMode::Classic,
        } as u8;
        rig.app.slots.keyboard_settings.editor = editor;
        rig.frame(vec![]);
        let at = if editor { seventh } else { E4 };
        let touch = |phase| {
            vec![egui::Event::Touch {
                device_id: egui::TouchDeviceId(1),
                id: egui::TouchId(6),
                phase,
                pos: at,
                force: None,
            }]
        };
        rig.take();
        rig.frame(touch(egui::TouchPhase::Start));
        let on = rig.take();
        assert!(
            !on.is_empty() && on.iter().all(|c| c.starts_with("NoteOn 1 ")),
            "{editor}: {on:?}"
        );
        rig.app.test_edit = Some(SongEdit::Replaced);
        rig.frame(vec![]);
        rig.frame(vec![]);
        rig.frame(vec![]);
        let log = rig.take();
        assert!(
            !log.iter().any(|c| c.starts_with("NoteOn")),
            "{editor}: {log:?}"
        );
        assert_eq!(rig.app.sounding(1), 0, "{editor}: dark");
        rig.frame(touch(egui::TouchPhase::End));
        rig.frame(vec![]);
        let log = rig.take();
        assert!(
            !log.iter().any(|c| c.starts_with("NoteO")),
            "{editor}: {log:?}"
        );
    }
}
