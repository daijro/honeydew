//! Preview: the editors' toggle that plays the current clip alone.

use eframe::egui::{Color32, Rect, Shape};

use super::*;
use crate::song::Event;
use crate::ui::preview::ClipKey;

/// Channel 2's first clip (bar 2, one bar) and its second (bar 3), and channel 1's drum clip (bar 1).
const SYNTH_1: ClipKey = ClipKey {
    channel: 2,
    lane: 0,
    id: 1,
};
const SYNTH_2: ClipKey = ClipKey { id: 2, ..SYNTH_1 };
const DRUMS: ClipKey = ClipKey {
    channel: 1,
    lane: 0,
    id: 1,
};

fn note(pos: u32, key: u8) -> Event {
    Event::Note {
        pos,
        len_beats: 1.0,
        key,
        velocity: Event::DEFAULT_VELOCITY,
        pan: Event::DEFAULT_PAN,
        slide: false,
    }
}

impl Rig {
    /// Drums (1) with a one-bar clip at bar 1, MiniSynth (2) with one-bar clips at bars 2 and 3, as loaded; the audio
    /// started, channel 2's piano roll shown (it edits clip 1) and an empty log.
    fn previewable() -> Self {
        let mut rig = Self::with_drums_and_synth();
        let song = &mut rig.app.song;
        song.channels[1].lanes[0].add_clip(0, 4.0);
        let synth = &mut song.channels[2].lanes[0];
        synth.add_clip(512, 4.0).events = vec![note(0, 60), note(256, 64)];
        synth.add_clip(1024, 4.0).events = vec![note(0, 67)];
        // As loaded: the undo history starts here.
        rig.app.song_edited(SongEdit::Replaced, 0);
        rig.pin_playlist();
        rig.click(OUTSIDE);
        rig.app.navigate(Nav::EditClip(SYNTH_1));
        rig.slide_split();
        rig.take();
        rig
    }

    fn preview(&mut self) {
        self.click_text_button("SOLO CLIP");
    }

    /// Where the engine the app sent last plays to from `tick`, 1024 frames on (a few ticks).
    fn engine_from(&self, tick: u64) -> u64 {
        use honeydew_audio::device::AudioRenderer;
        let mut rec = self.rec.0.borrow_mut();
        let engine = rec.engine.as_mut().expect("an engine");
        engine.seek(tick);
        engine.play();
        engine.render(&mut [0.0; 2048], 2);
        engine.transport_tick()
    }
}

#[test]
fn solo_clip_shows_while_the_current_track_has_a_current_clip_in_every_view_and_lights_while_on() {
    // A lane with no clip has no editor to open, and nothing to solo.
    let mut rig = Rig::with_synth();
    rig.app.navigate(Nav::Edit(1));
    rig.slide_split();
    assert!(!rig.painted("SOLO CLIP"));

    let mut rig = Rig::previewable();
    assert_eq!(rig.color_of("SOLO CLIP"), Some(ink()));
    rig.preview();
    assert_eq!(rig.app.preview, Some(SYNTH_1));
    rig.frame(vec![]);
    // Lit: a `text` disc behind the icon, and no caption.
    assert!(!rig.painted("SOLO CLIP"));
    let slot = rig
        .ctx
        .read_response(egui::Id::new(("transport", "SOLO CLIP")))
        .unwrap()
        .rect;
    let lit = |rig: &Rig| {
        rig.out.shapes.iter().any(|s| {
            matches!(&s.shape,
            Shape::Circle(c) if c.center == slot.center() && c.radius > slot.width() / 3.0)
        })
    };
    assert!(lit(&rig));
    rig.preview();
    assert_eq!(rig.app.preview, None);
    rig.frame(vec![]);
    assert!(rig.painted("SOLO CLIP") && !lit(&rig));

    // The drum editor, the playlist and the mixer have it too: the current track keeps its current clip.
    for nav in [
        Nav::EditClip(DRUMS),
        Nav::Show(View::Playlist),
        Nav::Show(View::Mixer),
    ] {
        rig.app.navigate(nav.clone());
        rig.frame(vec![]);
        rig.frame(vec![]);
        assert!(rig.painted("SOLO CLIP"), "{nav:?}");
    }
}

#[test]
fn undo_and_redo_always_show_and_ignore_taps_while_unavailable() {
    let mut rig = Rig::previewable();
    let rect = |rig: &Rig, label: &str| {
        let id = egui::Id::new(("transport", label));
        rig.ctx.read_response(id).map(|r| r.rect)
    };
    let (undo, redo) = (rect(&rig, "Undo").unwrap(), rect(&rig, "Redo").unwrap());
    // Nothing to undo: a tap does nothing (and is no tap on the bar's empty space).
    rig.click_text_button("Undo");
    assert!(!rig.app.history.can_redo());
    assert_eq!(rig.app.settings.preferred_kbd_size, 5.0);
    rig.app.song.tempo_bpm = 150.0;
    rig.app.song_edited(SongEdit::Final, 0);
    rig.frame(vec![]);
    let flags = |rig: &Rig| (rig.app.history.can_undo(), rig.app.history.can_redo());
    assert_eq!(flags(&rig), (true, false));
    rig.click_text_button("Undo");
    assert_eq!(flags(&rig), (false, true));
    rig.click_text_button("Redo");
    assert_eq!(flags(&rig), (true, false));
    // Nothing to redo now: a tap does nothing; neither button moved.
    rig.click_text_button("Redo");
    assert_eq!(flags(&rig), (true, false));
    assert_eq!(
        (rect(&rig, "Undo"), rect(&rig, "Redo")),
        (Some(undo), Some(redo))
    );
}

/// A finger on E4 (52) going down or lifting: a touch, so the mouse stays free for the Preview button.
fn e4(phase: egui::TouchPhase) -> Vec<egui::Event> {
    vec![egui::Event::Touch {
        device_id: egui::TouchDeviceId(1),
        id: egui::TouchId(5),
        phase,
        pos: E4,
        force: None,
    }]
}

#[test]
fn turning_preview_on_or_off_silences_a_held_chord_typed_or_midi_key_too() {
    // As the piano's, each live key's notes end with the handover's all notes off, with no note-off of their own, go
    // dark, and the key's release later sends nothing: a chord cell's (and its finger's slide onto another cell), the
    // chord editor's button's, a typed key's and a MIDI key's.
    use egui::TouchPhase::{End, Move, Start};
    let cell = pos2(1170.0 / 16.0, KEYS_TOP + 18.0);
    // The strip's second cell, and the editor's `7` in the band above the strips (5 U panel: 0.75 U).
    let next_cell = cell + vec2(0.0, U);
    let seventh = pos2(1.5 * 1170.0 / 7.0, KEYS_TOP + 0.4 * U);
    let touch = |phase, pos| {
        vec![egui::Event::Touch {
            device_id: egui::TouchDeviceId(1),
            id: egui::TouchId(6),
            phase,
            pos,
            force: None,
        }]
    };
    for source in ["chord", "slide", "editor", "typed", "midi"] {
        for on in [true, false] {
            let mut rig = Rig::previewable();
            let device = (source == "midi").then(|| rig.plug_midi());
            if ["chord", "slide", "editor"].contains(&source) {
                rig.app.settings.keyboard_kind = KeyboardMode::Chords as u8;
                rig.app.slots.keyboard_settings.editor = source == "editor";
                rig.frame(vec![]);
            }
            if !on {
                rig.preview();
            }
            rig.take();
            let hold = |rig: &mut Rig, down: bool| match source {
                "chord" => rig.frame(touch(if down { Start } else { End }, cell)),
                "slide" if down => rig.frame(touch(Start, cell)),
                "slide" => {
                    rig.frame(touch(Move, next_cell));
                    assert_eq!(rig.take(), Vec::<String>::new(), "{on}: the slide");
                    rig.frame(touch(End, next_cell));
                }
                "editor" => rig.frame(touch(if down { Start } else { End }, seventh)),
                "typed" => rig.type_key(egui::Key::Z, down),
                _ => {
                    let message: &[u8] = if down {
                        &[0x90, 60, 100]
                    } else {
                        &[0x80, 60, 0]
                    };
                    rig.midi(device.as_ref().unwrap(), &[message]);
                }
            };
            hold(&mut rig, true);
            let held = rig.take();
            assert!(
                !held.is_empty() && held.iter().all(|c| c.starts_with("NoteOn 2 ")),
                "{source} {on}: {held:?}"
            );
            rig.preview();
            let mut sent = vec!["ReplaceSong adopting silencing 0,1,2"];
            if on {
                sent.push("Seek 512");
            }
            assert_eq!(rig.take(), sent, "{source} {on}");
            rig.frame(vec![]);
            assert_eq!(rig.take(), Vec::<String>::new(), "{source} {on}");
            assert_eq!(rig.app.key_input().notes, 0, "{source} {on}: dark");
            hold(&mut rig, false);
            rig.frame(vec![]);
            assert_eq!(
                rig.take(),
                Vec::<String>::new(),
                "{source} {on}: the release"
            );
        }
    }
    // REC starting while Preview is on turns it off the same way: a key held then sounds no more and goes dark.
    let mut rig = Rig::previewable();
    rig.app.settings.count_in = false;
    rig.preview();
    rig.app.main_screen.playing = true;
    rig.rec.0.borrow_mut().tick = 600;
    rig.frame(vec![]);
    rig.frame(e4(Start));
    assert!(rig.take().contains(&"NoteOn 2 52 108".to_string()));
    rig.click(pos2(585.0 - 2.0 * 1.75 * U, BAR_Y));
    assert!(rig.app.main_screen.armed);
    rig.frame(vec![]);
    assert_eq!(rig.app.preview, None);
    let log = rig.take();
    assert!(!log.iter().any(|c| c.starts_with("NoteO")), "{log:?}");
    assert_eq!(rig.app.key_input().notes, 0, "REC start: dark");
    rig.frame(e4(End));
    let log = rig.take();
    assert!(!log.iter().any(|c| c.starts_with("NoteO")), "{log:?}");
}

#[test]
fn turning_preview_on_or_off_silences_a_held_drum_pad_too() {
    // Preview's silence lets go of the held pads exactly as of the held keys: no note-off of their own (it would choke
    // The pad's cut group), the finger still on the pad holds nothing more, and its release sends nothing; REC hears
    // The key-up then, and a tapped pad's gate ends with no note-off. A pad picked up and dragged meanwhile still
    // drops: the swap goes on.
    use crate::song::{DrumKit, SamplerSlot};
    use egui::TouchPhase::{End, Move, Start};
    let pad = super::editor_wiring::pad_in(crate::ui::keyboard_settings::PANEL_H, 0);
    let to = super::editor_wiring::pad_in(crate::ui::keyboard_settings::PANEL_H, 1);
    let at = |phase, pos| egui::Event::Touch {
        device_id: egui::TouchDeviceId(1),
        id: egui::TouchId(7),
        phase,
        pos,
        force: None,
    };
    // Pad 0 has a slot (key 48); the drum editor on channel 1's clip: the current channel, whose pads the keyboard
    // panel has.
    let pad_rig = || {
        let mut rig = Rig::previewable();
        rig.app.settings.lock_pads = false;
        let mut kit = DrumKit::default();
        kit.insert_slot(0, SamplerSlot::default());
        rig.app.song.channels[1].drum_kit = Some(kit);
        rig.app.song_edited(SongEdit::Final, 0);
        rig.app.navigate(Nav::EditClip(DRUMS));
        rig.slide_split();
        rig
    };
    let pads = |rig: &Rig| {
        rig.app.song.channels[1]
            .drum_kit
            .as_ref()
            .unwrap()
            .file
            .pads
    };
    for on in [true, false] {
        let mut rig = pad_rig();
        if !on {
            rig.preview();
        }
        rig.take();
        let before = pads(&rig);
        rig.frame(vec![at(Start, pad)]);
        assert_eq!(rig.take(), ["NoteOn 1 48 127"], "{on}");
        rig.preview();
        assert_eq!(rig.take(), ["ReplaceSong adopting silencing 0,1,2"], "{on}");
        rig.frame(vec![]);
        assert_eq!(rig.take(), Vec::<String>::new(), "{on}");
        // The finger still down holds nothing: dragged off and dropped on pad 1, it neither ends a note nor swaps.
        rig.frame(vec![at(Move, to)]);
        rig.frame(vec![at(End, to)]);
        rig.frame(vec![]);
        assert_eq!(rig.take(), Vec::<String>::new(), "{on}: the release");
        assert_eq!(pads(&rig), before, "{on}: no drop");
        // A pad picked up (its note ended at the pick-up), Preview toggled mid-drag, dropped on pad 1: they swap.
        rig.frame(vec![at(Start, pad)]);
        rig.frame(vec![at(Move, to)]);
        assert_eq!(rig.take(), ["NoteOn 1 48 127", "NoteOff 1 48"], "{on}");
        rig.preview();
        rig.frame(vec![at(End, to)]);
        let mut swapped = before;
        swapped.swap(0, 1);
        assert_eq!(pads(&rig), swapped, "{on}: the drop");
        // No pad is left held: a hide has nothing to stop.
        rig.take();
        rig.app.on_hidden();
        assert_eq!(rig.take(), Vec::<String>::new(), "{on}: nothing held");
    }

    // A tap and Preview's click in one frame: the tap's gate runs as the handover goes, and ends with no note-off.
    let mut rig = pad_rig();
    let button = (rig
        .ctx
        .read_response(egui::Id::new(("transport", "SOLO CLIP"))))
    .expect("drawn")
    .rect
    .center();
    rig.press(button);
    rig.take();
    rig.frame(vec![
        at(Start, pad),
        at(End, pad),
        button_event(button, false),
    ]);
    let log = rig.take();
    assert!(log.iter().any(|c| c.starts_with("ReplaceSong")), "{log:?}");
    assert!(!log.iter().any(|c| c.starts_with("NoteOff")), "{log:?}");
    for _ in 0..5 {
        rig.frame(vec![]);
    }
    assert_eq!(rig.take(), Vec::<String>::new(), "the tap's gate");

    // While recording: REC hears the held pad's key-up as Preview goes on, so its hit ends there (beats 2 to 5).
    let ppq = u64::from(crate::song::PPQ);
    let mut rig = pad_rig();
    rig.app.settings.count_in = false;
    rig.app.main_screen.armed = true;
    rig.rec.0.borrow_mut().tick = 2 * ppq;
    rig.frame(vec![at(Start, pad)]);
    rig.frame(vec![]);
    assert!(
        rig.app.main_screen.playing,
        "an armed press starts playback"
    );
    rig.rec.0.borrow_mut().tick = 5 * ppq;
    rig.preview();
    rig.frame(vec![]);
    rig.frame(vec![]);
    rig.rec.0.borrow_mut().tick = 7 * ppq;
    rig.frame(vec![at(End, pad)]);
    rig.frame(vec![]);
    rig.click(PLAY);
    let hits: Vec<(u32, f64)> = (rig.app.song.channels[1].lanes.iter())
        .flat_map(|l| l.clips.iter())
        .flat_map(|c| {
            c.events.iter().filter_map(move |e| match *e {
                Event::Drum { pos, len_beats, .. } => Some((c.start_tick + pos, len_beats)),
                _ => None,
            })
        })
        .collect();
    assert_eq!(hits, [(2 * crate::song::PPQ, 3.0)]);
}

#[test]
fn a_key_held_while_recording_ends_its_recorded_note_when_preview_goes_on() {
    // The handover silences the key, and REC hears its key-up then.
    let ppq = u64::from(crate::song::PPQ);
    let mut rig = Rig::previewable();
    rig.app.settings.count_in = false;
    rig.app.main_screen.armed = true;
    rig.rec.0.borrow_mut().tick = 2 * ppq;
    // Its Play goes out in the press's frame.
    rig.type_key(egui::Key::Z, true);
    rig.frame(vec![]);
    assert!(
        rig.app.main_screen.playing,
        "an armed press starts playback"
    );
    rig.rec.0.borrow_mut().tick = 5 * ppq;
    rig.preview();
    // REC hears the frame's keys at its end: the next frame's.
    rig.frame(vec![]);
    rig.rec.0.borrow_mut().tick = 7 * ppq;
    rig.type_key(egui::Key::Z, false);
    rig.click(PLAY);
    let notes: Vec<(u32, u8, f64)> = (rig.app.song.channels[2].lanes[0].clips.iter())
        .flat_map(|c| {
            c.events.iter().filter_map(move |e| match *e {
                Event::Note {
                    pos,
                    key,
                    len_beats,
                    ..
                } => Some((c.start_tick + pos, key, len_beats)),
                _ => None,
            })
        })
        .filter(|n| n.1 == 48)
        .collect();
    assert_eq!(notes, [(2 * crate::song::PPQ, 48, 3.0)]);
}

#[test]
fn turning_preview_on_silences_the_held_note_and_sends_the_clip_alone_from_its_span() {
    use egui::TouchPhase::{End, Start};
    let mut rig = Rig::previewable();
    // A note the keyboard holds.
    rig.frame(e4(Start));
    assert_eq!(rig.take(), ["NoteOn 2 52 108"]);
    // Stopped at tick 0, outside [512, 1024): the playhead jumps to 512. The handover's all notes off on every channel
    // silences the key: no note-off of its own, and the finger still on it plays nothing more.
    rig.preview();
    assert_eq!(
        rig.take(),
        ["ReplaceSong adopting silencing 0,1,2", "Seek 512"]
    );
    rig.frame(vec![]);
    assert_eq!(rig.take(), Vec::<String>::new());
    assert!(rig.app.notes.is_empty());
    assert_eq!(rig.app.key_input().notes, 0, "the key goes dark");
    rig.frame(e4(End));
    assert_eq!(rig.take(), Vec::<String>::new());
    // The engine plays the clip alone; the song keeps every clip.
    let solo = rig.app.solo().expect("previewing");
    let clips = |song: &Song| -> Vec<usize> {
        let lanes = song.channels.iter().map(|c| &c.lanes[0]);
        lanes.map(|l| l.clips.len()).collect()
    };
    assert_eq!(clips(&solo), [0, 0, 1]);
    assert_eq!(clips(&rig.app.song), [0, 1, 2]);
    // Off, with the key held again: the whole song again, the key silenced and dark the same way.
    rig.frame(e4(Start));
    assert_eq!(rig.take(), ["NoteOn 2 52 108"]);
    rig.preview();
    assert_eq!(rig.take(), ["ReplaceSong adopting silencing 0,1,2"]);
    assert_eq!(rig.app.solo(), None);
    rig.frame(vec![]);
    assert_eq!(rig.take(), Vec::<String>::new());
    assert_eq!(rig.app.key_input().notes, 0, "the key goes dark");
    rig.frame(e4(End));
    assert_eq!(rig.take(), Vec::<String>::new());

    // Playing inside the span: it carries on from there.
    rig.app.main_screen.playing = true;
    rig.rec.0.borrow_mut().tick = 700;
    rig.frame(vec![]);
    rig.take();
    rig.preview();
    let sent = ["ReplaceSong adopting silencing 0,1,2", "Play"];
    assert_eq!(rig.take(), sent);
}

#[test]
fn while_playing_the_engine_loops_the_span_and_a_playhead_before_it_jumps_to_its_start() {
    // The engine loops, so a playhead at or past the end needs no Seek from the app; one before the loop still jumps
    // to its start.
    let mut rig = Rig::previewable();
    rig.preview();
    rig.app.main_screen.playing = true;
    rig.frame(vec![]);
    rig.take();
    for (tick, sent) in [
        (600, &[][..]),
        (1024, &[]),
        (1500, &[]),
        (100, &["Seek 512"][..]),
        (512, &[]),
    ] {
        rig.rec.0.borrow_mut().tick = tick;
        rig.frame(vec![]);
        assert_eq!(rig.take(), sent, "{tick}");
    }
    // The solo engine loops [512, 1024): at or past its end it goes on from 512; before it, it plays into it.
    for (from, to) in [
        (1024, 512..520),
        (1500, 512..520),
        (600, 600..608),
        (100, 100..108),
    ] {
        assert!(to.contains(&rig.engine_from(from)), "{from}");
    }
    // Off: the song's engine, which no play loop loops.
    rig.preview();
    assert_eq!(rig.take(), ["ReplaceSong adopting silencing 0,1,2", "Play"]);
    assert!((1500..1508).contains(&rig.engine_from(1500)));
    rig.frame(vec![]);
    assert_eq!(rig.take(), Vec::<String>::new());
}

#[test]
fn a_play_loop_inside_the_span_is_previews_loop_and_markers_outside_it_are_the_spans() {
    // Inside [512, 1024): beats 1 to 2 of the clip. Turning Preview on at tick 0 jumps to the loop's start.
    let mut rig = Rig::previewable();
    rig.app.song.play_loop = Some((640, 768));
    rig.frame(vec![]);
    // Without Preview the song's play loop goes to the engine as it changes.
    assert_eq!(rig.take(), ["SetLoop 640 768"]);
    rig.preview();
    assert_eq!(
        rig.take(),
        ["ReplaceSong adopting silencing 0,1,2", "Seek 640"]
    );
    // The solo engine has the loop from its song: 768 goes on from 640.
    assert!((640..648).contains(&rig.engine_from(768)));
    rig.app.main_screen.playing = true;
    rig.frame(vec![]);
    rig.take();
    // At or past the end the engine wraps; before the start the app jumps.
    for (tick, sent) in [(700, &[][..]), (768, &[]), (600, &["Seek 640"][..])] {
        rig.rec.0.borrow_mut().tick = tick;
        rig.frame(vec![]);
        assert_eq!(rig.take(), sent, "{tick}");
    }
    // A marker the ruler moves outside the span [512, 1024] becomes the span's own; the other stays. The engine gets
    // each change once.
    for (play_loop, tick, sent) in [
        ((256, 768), 900, &["SetLoop 512 768"][..]),
        ((256, 768), 300, &["Seek 512"]),
        ((768, 1200), 1100, &["SetLoop 768 1024"]),
        ((768, 1200), 1000, &[]),
    ] {
        rig.app.song.play_loop = Some(play_loop);
        rig.rec.0.borrow_mut().tick = tick;
        rig.frame(vec![]);
        assert_eq!(rig.take(), sent, "{play_loop:?} {tick}");
    }
    assert!((768..776).contains(&rig.engine_from(1100)));
}

#[test]
fn turning_preview_on_clamps_the_play_loop_into_the_clip_and_saves_it_as_no_undo_step() {
    // Clip 1: s₀ = 512, L = 512, so the markers are clamped into [512, 1024].
    let mut rig = Rig::previewable();
    rig.app.song.play_loop = Some((256, 768));
    rig.app.files.saved();
    // The loop set before the click reaches the song's engine on the click's first frame.
    rig.preview();
    assert_eq!(rig.app.song.play_loop, Some((512, 768)));
    assert_eq!(
        rig.take(),
        [
            "SetLoop 256 768",
            "ReplaceSong adopting silencing 0,1,2",
            "Seek 512"
        ]
    );
    rig.frame(vec![]);
    assert!(!rig.app.history.can_undo());
    // The app's own clamp sets no song-changed flag.
    assert!(!rig.app.files.modified());
    // Off does not restore it.
    rig.preview();
    assert_eq!(rig.app.song.play_loop, Some((512, 768)));
    // Nothing of it inside: no range.
    let mut rig = Rig::previewable();
    rig.app.song.play_loop = Some((0, 512));
    rig.preview();
    assert_eq!(rig.app.song.play_loop, None);
    assert_eq!(
        rig.take(),
        [
            "SetLoop 0 512",
            "ReplaceSong adopting silencing 0,1,2",
            "Seek 512"
        ]
    );
}

#[test]
fn moving_preview_to_another_clip_or_channel_clamps_the_play_loop_into_that_clip() {
    let mut rig = Rig::previewable();
    rig.preview();
    // From inside clip 1 (to 1024) into clip 2 (1024 to 1536).
    rig.app.song.play_loop = Some((900, 1200));
    rig.app.song.channels[2].lanes[0].file.current_clip_id = 2;
    rig.frame(vec![]);
    assert_eq!(rig.app.preview, Some(SYNTH_2));
    assert_eq!(rig.app.song.play_loop, Some((1024, 1200)));
    // The drum clip's [0, 512] holds none of it.
    rig.app.navigate(Nav::EditClip(DRUMS));
    rig.slide_split();
    assert_eq!(rig.app.preview, Some(DRUMS));
    assert_eq!(rig.app.song.play_loop, None);
    assert!(!rig.app.history.can_undo());
}

#[test]
fn a_span_before_the_song_start_loops_from_0() {
    // The drum clip at 0, 5 beats into its 4-beat content: s₀ = −640 < −L, so [s, e) = [−128, 384).
    let mut rig = Rig::previewable();
    rig.app.song.channels[1].lanes[0].clips[0].offset_beats = 5.0;
    rig.app.song_edited(SongEdit::Final, 0);
    rig.app.navigate(Nav::EditClip(DRUMS));
    rig.slide_split();
    rig.frame(vec![]);
    rig.take();
    // At 0, inside: no jump.
    rig.preview();
    assert_eq!(rig.take(), ["ReplaceSong adopting silencing 0,1,2"]);
    rig.app.main_screen.playing = true;
    for tick in [200, 384, 1000] {
        rig.rec.0.borrow_mut().tick = tick;
        rig.frame(vec![]);
        assert_eq!(rig.take(), Vec::<String>::new(), "{tick}");
    }
    // The engine loops [0, 384).
    for (from, to) in [(200, 200..208), (384, 0..8), (1000, 0..8)] {
        assert!(to.contains(&rig.engine_from(from)), "{from}");
    }
}

#[test]
fn a_span_wholly_before_the_song_start_never_jumps() {
    // 8 beats into the 4-beat content, s₀ = −1024 ≤ −2L, so [s, e) = [−512, 0) holds no
    // tick ≥ 0: a jump to 0 would land outside it again, every frame. The transport plays on instead.
    let mut rig = Rig::previewable();
    rig.app.song.channels[1].lanes[0].clips[0].offset_beats = 8.0;
    rig.app.song_edited(SongEdit::Final, 0);
    rig.app.navigate(Nav::EditClip(DRUMS));
    rig.slide_split();
    rig.frame(vec![]);
    rig.preview();
    assert_eq!(rig.app.preview, Some(DRUMS));
    rig.take();
    rig.app.main_screen.playing = true;
    for tick in [0, 0, 200, 1000] {
        rig.rec.0.borrow_mut().tick = tick;
        rig.frame(vec![]);
        assert_eq!(rig.take(), Vec::<String>::new(), "{tick}");
    }
    // Nor does the engine loop it: with no loop it wraps at the previewed song's end, one bar, like any
    // transport, so it plays on from the start.
    assert!((0..8).contains(&rig.engine_from(1000)));
}

#[test]
fn a_loop_the_engine_never_got_goes_out_once_the_audio_runs_again() {
    // With the stream failed no engine goes out, so the app does not count its loop as sent.
    let mut rig = Rig::previewable();
    rig.rec.0.borrow_mut().failed = true;
    rig.frame(vec![]);
    rig.app.song.play_loop = Some((640, 768));
    rig.app.test_edit = Some(SongEdit::Final);
    rig.frame(vec![]);
    assert_eq!(rig.take(), Vec::<String>::new());
    rig.click(OUTSIDE);
    assert!(rig.take().contains(&"SetLoop 640 768".to_string()));
}

#[test]
fn edits_while_previewing_reach_the_song_and_the_engine_gets_the_clip_alone_again() {
    let mut rig = Rig::previewable();
    rig.preview();
    rig.take();
    // A note added to the previewed clip, and one to another channel's: both are in the song.
    rig.app.song.channels[2].lanes[0].clips[0]
        .events
        .push(note(384, 72));
    rig.app.song.channels[1].lanes[0].clips[0]
        .events
        .push(Event::Drum {
            pos: 0,
            len_beats: 0.25,
            pad: 0,
            velocity: 200,
            pan: 127,
            value: 0x7FFF,
            repeat: 0,
        });
    rig.app.test_edit = Some(SongEdit::Final);
    rig.frame(vec![]);
    // Notes added by a final edit, as a Paste: a re-sync.
    assert_eq!(rig.take(), ["ReplaceSong adopting"]);
    assert_eq!(rig.app.preview, Some(SYNTH_1));
    assert_eq!(rig.app.song.channels[1].lanes[0].clips[0].events.len(), 1);
    let solo = rig.app.solo().unwrap();
    assert_eq!(solo.channels[2].lanes[0].clips[0].events.len(), 3);
    assert!(solo.channels[1].lanes[0].clips.is_empty());
    // Undo is an edit too: the engine still gets the clip alone.
    rig.frame(z(egui::Modifiers::NONE));
    assert_eq!(rig.take(), ["ReplaceSong adopting"]);
    assert_eq!(rig.app.song.channels[2].lanes[0].clips[0].events.len(), 2);
    assert_eq!(
        rig.app.solo().unwrap().channels[2].lanes[0].clips[0]
            .events
            .len(),
        2
    );
    // The previewed clip deleted: Preview goes off and the song plays.
    rig.app.song.channels[2].lanes[0].clips.remove(0);
    rig.app.test_edit = Some(SongEdit::Final);
    rig.frame(vec![]);
    assert_eq!(rig.take(), ["ReplaceSong adopting"]);
    assert_eq!(rig.app.preview, None);
}

#[test]
fn another_clip_or_channel_moves_preview_and_a_loaded_song_turns_it_off() {
    let mut rig = Rig::previewable();
    rig.preview();
    rig.take();
    // The playlist makes clip 2 current: the roll follows it, and Preview follows the roll.
    rig.app.song.channels[2].lanes[0].file.current_clip_id = 2;
    rig.frame(vec![]);
    assert_eq!(rig.app.preview, Some(SYNTH_2));
    assert_eq!(
        rig.take(),
        ["ReplaceSong adopting silencing 0,1,2", "Seek 1024"]
    );
    // Another channel's editor.
    rig.app.navigate(Nav::EditClip(DRUMS));
    rig.slide_split();
    assert_eq!(rig.app.preview, Some(DRUMS));
    // Its span holds the playhead (the snapshot says 0): no jump.
    assert_eq!(rig.take(), ["ReplaceSong adopting silencing 0,1,2"]);
    // The playlist edits no clip: Preview stays where it is.
    rig.app.navigate(Nav::Show(View::Playlist));
    rig.frame(vec![]);
    assert_eq!(rig.app.preview, Some(DRUMS));
    assert_eq!(rig.take(), Vec::<String>::new());
    // A loaded song.
    rig.app.song_edited(SongEdit::Replaced, 0);
    assert_eq!(rig.app.preview, None);
}

/// The fills of the small rects (a clip's miniature notes) the last frame painted inside `area`.
fn note_fills(rig: &Rig, area: Rect) -> Vec<Color32> {
    let mut fills: Vec<Color32> = (rig.out.shapes.iter())
        .filter_map(|s| match &s.shape {
            Shape::Rect(r) if area.contains_rect(r.rect) && r.rect.height() < 0.5 * U => {
                Some(r.fill)
            }
            _ => None,
        })
        .collect();
    fills.dedup();
    fills
}

#[test]
fn the_playlist_dims_every_clip_but_the_previewed_one() {
    let mut rig = Rig::previewable();
    // Clip 2 gets clip 1's velocities, so that only the dimming tells them apart.
    rig.app.song.channels[2].lanes[0].clips[1].events = vec![note(0, 60), note(256, 64)];
    rig.preview();
    rig.app.navigate(Nav::Show(View::Playlist));
    rig.frame(vec![]);
    rig.frame(vec![]);
    // Row 2 spans y 176..240; a beat is 38.4 points from x 48.
    let row = |from: f32, to: f32| Rect::from_x_y_ranges(from..=to, 176.0..=240.0);
    let (first, second) = (row(201.6, 355.0), row(355.2, 508.0));
    let (lit, dimmed) = (note_fills(&rig, first), note_fills(&rig, second));
    assert_eq!(lit.len(), 1, "{lit:?}");
    assert_eq!(dimmed.len(), 1, "{dimmed:?}");
    assert_ne!(lit, dimmed);
    // Dimmed as a muted clip is, once Preview is off.
    rig.app.song.channels[2].lanes[0].clips[1].muted = true;
    rig.app.preview = None;
    rig.frame(vec![]);
    assert_eq!(note_fills(&rig, second), dimmed);
    assert_eq!(note_fills(&rig, first), lit);
}

/// The centre of the one-bar playlist clip starting at song tick `start` on track `channel`'s row (a beat is 38.4
/// points from x 48).
fn clip_at(channel: usize, start: u32) -> Pos2 {
    pos2(
        48.0 + (start as f32 / 128.0 + 2.0) * 38.4,
        header(channel).y,
    )
}

#[test]
fn a_clips_edit_opens_that_clip_whose_edits_land_in_it_and_preview_follows() {
    // Preview on in channel 2's roll, which edits its first clip.
    let mut rig = Rig::previewable();
    rig.preview();
    rig.app.navigate(Nav::Show(View::Playlist));
    rig.frame(vec![]);
    // `Edit` on the second clip.
    rig.click(clip_at(2, 1024));
    rig.frame(vec![]);
    rig.click_id(egui::Id::new(("clip_button", 3_usize)));
    assert_eq!(rig.app.nav.view, View::PianoRoll(2));
    rig.slide_split();
    assert_eq!(rig.app.preview, Some(SYNTH_2));
    // A tap on an empty cell adds a note to that clip.
    rig.click_id(egui::Id::new("piano_roll").with("grid"));
    let clips = &rig.app.song.channels[2].lanes[0].clips;
    let notes: Vec<usize> = clips.iter().map(|c| c.events.len()).collect();
    assert_eq!(notes, [2, 2]);
}

/// While Preview plays a clip with a content offset, the editors map the transport through Preview's span
/// [s, e) (content = tick − s, a ruler tap seeks to s + c), as the engine plays it.
#[test]
fn previewing_an_offset_clip_the_roll_maps_the_transport_through_the_span() {
    let mut rig = Rig::previewable();
    // At 512 two beats into its content: the span is [256, 768).
    rig.app.song.channels[2].lanes[0].clips[0].offset_beats = 2.0;
    rig.app.song_edited(SongEdit::Final, 0);
    rig.take();
    rig.preview();
    assert_eq!(
        rig.take(),
        ["ReplaceSong adopting silencing 0,1,2", "Seek 256"]
    );
    // Playing at 300 the engine plays content 44: the handle and the line show it (a beat, 128 ticks, being 2 U).
    rig.app.main_screen.playing = true;
    rig.rec.0.borrow_mut().tick = 300;
    rig.frame(vec![]);
    let grid = egui::Id::new("piano_roll").with("grid");
    let left = rig.ctx.read_response(grid).unwrap().rect.left();
    assert_eq!(rig.playhead(grid), (left + 44.0 / 128.0 * 2.0 * U, true));
    // Stopped, a tap on content beat 2 (past the play box's touch area) seeks to s + 256, inside the loop.
    rig.app.main_screen.playing = false;
    rig.frame(vec![]);
    rig.take();
    rig.click(pos2(
        left + 4.0 * U,
        super::editor_wiring::EDITOR_TOP + 24.0,
    ));
    assert_eq!(rig.take(), ["Seek 512"]);
}

#[test]
fn previewing_an_offset_clip_the_drum_editor_maps_the_transport_through_the_span() {
    let mut rig = Rig::previewable();
    rig.app.song.channels[1].lanes[0].clips[0].start_tick = 512;
    rig.app.song.channels[1].lanes[0].clips[0].offset_beats = 2.0;
    rig.app.song_edited(SongEdit::Final, 0);
    rig.app.navigate(Nav::EditClip(DRUMS));
    rig.slide_split();
    rig.frame(vec![]);
    rig.take();
    rig.preview();
    assert_eq!(
        rig.take(),
        ["ReplaceSong adopting silencing 0,1,2", "Seek 256"]
    );
    // Playing at 300: content 44, 1.375 steps in.
    rig.app.main_screen.playing = true;
    rig.rec.0.borrow_mut().tick = 300;
    rig.frame(vec![]);
    // The step grid's playhead line (`text`, narrower than U, under the ruler) is there.
    let (x0, step) = rig.app.slots.drum_editor.step.time_axis().unwrap();
    let text = rig.app.theme.colors.text.to_color32();
    // The editor's, under the playlist's (`ui::split`).
    let line = (rig.rects(text).into_iter())
        .find(|r| r.width() < U && r.top() >= super::editor_wiring::EDITOR_TOP + RULER)
        .expect("the playhead line");
    // Centred on the device pixel nearest the playhead's x (MM's line, ppp 1).
    assert_eq!(
        crate::ui::timeline::testing::line_x(line, 1.0),
        (x0 + 1.375 * step).round(),
        "{line:?}"
    );
    // Stopped, a tap on step 8 (content 256, past the play box's touch area) seeks to s + 256 (the ruler maps song
    // ticks).
    rig.app.main_screen.playing = false;
    rig.frame(vec![]);
    rig.take();
    rig.click(pos2(
        x0 + 8.2 * step,
        super::editor_wiring::EDITOR_TOP + 24.0,
    ));
    assert_eq!(rig.take(), ["Seek 512"]);
}

#[test]
fn without_preview_the_engine_gets_each_play_loop_change_once() {
    // The engine loops sample-accurately; the app only tells it the loop.
    let mut rig = Rig::previewable();
    // An engine for the song, which has no loop.
    rig.app.test_edit = Some(SongEdit::Final);
    rig.frame(vec![]);
    assert_eq!(rig.take(), ["ReplaceSong adopting no-resync"]);
    rig.app.song.play_loop = Some((640, 768));
    rig.app.main_screen.playing = true;
    let mut sent = vec!["SetLoop 640 768"];
    for tick in [100, 700, 768, 2000] {
        rig.rec.0.borrow_mut().tick = tick;
        rig.frame(vec![]);
        assert_eq!(rig.take(), sent, "{tick}");
        sent.clear();
    }
    assert!((640..648).contains(&rig.engine_from(1000)));
    // No loop: on to the song end (three bars).
    rig.app.song.play_loop = None;
    rig.frame(vec![]);
    assert_eq!(rig.take(), ["SetLoop none"]);
    assert!((1000..1008).contains(&rig.engine_from(1000)));
    // A rebuilt engine reads the loop from its song: nothing more to send.
    rig.app.song.play_loop = Some((128, 256));
    rig.app.test_edit = Some(SongEdit::Final);
    rig.frame(vec![]);
    assert_eq!(rig.take(), ["ReplaceSong adopting no-resync", "Play"]);
    assert!((128..136).contains(&rig.engine_from(300)));
}

#[test]
fn starting_a_recording_by_playing_with_preview_on_keeps_the_starting_key() {
    // REC armed and stopped, Preview on, a press starts the recording, which turns Preview off before the press's note
    // goes out (the starting key keeps sounding): it sounds, stays lit and records its whole length. Preview's
    // tap in the press's frame changes nothing, the recording that starts there turning it off.
    let ppq = u64::from(crate::song::PPQ);
    let z = |pressed| super::typed::typed(egui::Key::Z, pressed, egui::Modifiers::NONE);
    for same_frame in [false, true] {
        let mut rig = Rig::previewable();
        rig.app.settings.count_in = false;
        rig.app.main_screen.armed = true;
        rig.rec.0.borrow_mut().tick = 2 * ppq;
        if !same_frame {
            rig.preview();
            assert_eq!(rig.app.preview, Some(SYNTH_1));
        }
        rig.frame(vec![]);
        let button = (rig.ctx)
            .read_response(egui::Id::new(("transport", "SOLO CLIP")))
            .expect("drawn")
            .rect
            .center();
        if same_frame {
            rig.press(button);
        }
        rig.take();
        match same_frame {
            true => rig.frame(vec![button_event(button, false), z(true)]),
            false => rig.frame(vec![z(true)]),
        }
        assert!(rig.app.main_screen.playing, "{same_frame}");
        assert_eq!(rig.app.preview, None, "{same_frame}");
        let log = rig.take();
        let on = log.iter().position(|c| c.starts_with("NoteOn 2 48 "));
        let off = log.iter().position(|c| c.starts_with("ReplaceSong"));
        assert!(on.is_some() && off < on, "{same_frame}: {log:?}");
        assert!(
            !log.contains(&"NoteOff 2 48".into()),
            "{same_frame}: {log:?}"
        );
        assert_eq!(rig.app.key_input().notes, 1 << 48, "{same_frame}: lit");
        rig.rec.0.borrow_mut().tick = 5 * ppq;
        rig.frame(vec![]);
        assert_eq!(rig.app.key_input().notes, 1 << 48, "{same_frame}: lit");
        rig.frame(vec![z(false)]);
        rig.click(PLAY);
        let notes: Vec<(u32, u8, f64)> = (rig.app.song.channels[2].lanes.iter())
            .flat_map(|l| l.clips.iter())
            .flat_map(|c| {
                c.events.iter().filter_map(move |e| match *e {
                    Event::Note {
                        pos,
                        key,
                        len_beats,
                        ..
                    } => Some((c.start_tick + pos, key, len_beats)),
                    _ => None,
                })
            })
            .filter(|n| n.1 == 48)
            .collect();
        assert_eq!(notes, [(2 * crate::song::PPQ, 48, 3.0)], "{same_frame}");
    }
}

/// While Preview is on, the song loop holds Preview's markers, which may lie past the song end (a
/// clip whose loop is longer than it shows). An edit that shortens the song leaves them alone, and Preview loops them.
#[test]
fn a_shortening_edit_leaves_previews_loop_alone() {
    let mut rig = Rig::previewable();
    let synth = &mut rig.app.song.channels[2].lanes[0].clips[0];
    (synth.loop_len_beats, synth.visible_len_beats) = (16.0, 4.0);
    rig.app.song.play_loop = Some((2048, 2304));
    rig.app.song_edited(SongEdit::Final, 0);
    rig.preview();
    assert_eq!(rig.app.preview, Some(SYNTH_1));
    assert_eq!(rig.app.song.play_loop, Some((2048, 2304)));
    // The clip at 1024 goes: the song ends at 1024, not 1536.
    rig.app.song.channels[2].lanes[0].clips.pop();
    rig.app.song_edited(SongEdit::Final, 0);
    rig.frame(vec![]);
    assert_eq!(rig.app.song.play_loop, Some((2048, 2304)));
    assert!((2048..2056).contains(&rig.engine_from(2400)));
}
