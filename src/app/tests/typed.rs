//! Typing-keyboard notes in the app: the press plays at 0.7 on the current channel through the touched keys' path, on
//! every screen and with the panel hidden; shortcuts and focused text fields keep their presses, never their releases;
//! the piano lights the notes.

use egui::{Event, Key, Modifiers};

use super::*;
use crate::app::typed::VELOCITY;

/// A key event at US position `at`, its logical key the same, with `modifiers`.
pub(super) fn typed(at: Key, pressed: bool, modifiers: Modifiers) -> Event {
    Event::Key {
        key: at,
        physical_key: Some(at),
        pressed,
        repeat: false,
        modifiers,
    }
}

impl Rig {
    /// Presses (`pressed`) or releases the key at US position `at`, with no modifier, in a frame of its own.
    pub(super) fn type_key(&mut self, at: Key, pressed: bool) {
        self.frame(vec![typed(at, pressed, Modifiers::NONE)]);
    }

    /// How far the last frame drew the white key at `at` (above its marker strip) from `whiteKey` toward the keyboard
    /// channel's colour: its press amount p, 0 … 1, on the sum of the three channels' distances. The last rect drawn
    /// there is the key's overpaint.
    pub(super) fn key_level(&self, at: Pos2) -> f32 {
        let fill = (self.out.shapes.iter().rev())
            .find_map(|s| match &s.shape {
                egui::Shape::Rect(r) if r.rect.contains(at) => Some(r.fill),
                _ => None,
            })
            .expect("a key there");
        let white = self.app.theme.colors.white_key.to_color32();
        let hue = (self.app.song.channels.get(self.app.selected_channel)).map_or(0.0, |c| c.hue);
        let channel = crate::theme::channel_color(hue, &self.app.theme).to_color32();
        let d = |a: egui::Color32, b: egui::Color32| {
            (0..3).map(|i| f32::from(a[i].abs_diff(b[i]))).sum::<f32>()
        };
        d(fill, white) / d(channel, white)
    }
}

/// C4 (MIDI 48), the leftmost white key, low on it.
const C4: Pos2 = pos2(U, LOW);

#[test]
fn a_typed_key_plays_on_the_current_channel_at_07_and_its_release_ends_it() {
    let mut rig = Rig::with_synth();
    rig.type_key(Key::Z, true);
    assert_eq!(rig.take(), ["NoteOn 1 48 89"]);
    rig.type_key(Key::Z, false);
    assert_eq!(rig.take(), ["NoteOff 1 48"]);
    // A shared pair: `,` and Q both play 60, and either release ends it.
    rig.type_key(Key::Comma, true);
    rig.type_key(Key::Q, true);
    assert_eq!(rig.take(), ["NoteOn 1 60 89", "NoteOn 1 60 89"]);
    rig.type_key(Key::Comma, false);
    rig.type_key(Key::Q, false);
    assert_eq!(rig.take(), ["NoteOff 1 60"]);
}

#[test]
fn typed_keys_play_over_the_rack_with_the_keyboard_panel_hidden() {
    let mut rig = Rig::with_synth();
    rig.app.main_screen.panel_hidden = true;
    rig.open_rack(1);
    rig.slide_rack();
    rig.take();
    rig.type_key(Key::Q, true);
    assert_eq!(rig.take(), ["NoteOn 1 60 89"]);
    // In the piano roll too.
    rig.type_key(Key::Q, false);
    rig.edit(1);
    rig.frame(vec![]);
    rig.take();
    rig.type_key(Key::Num2, true);
    assert_eq!(rig.take(), ["NoteOn 1 61 89"]);
}

/// A note started under key-lock on channel 1 ends there, though its release comes with channel 2
/// (Drums: no key-lock) current.
#[test]
fn a_release_ends_the_note_its_press_started_after_the_channel_and_key_lock_change() {
    let mut rig = Rig::with_synth();
    rig.app.add_default_drums(0);
    rig.app.song.key_scale = crate::song::scales::scale_to_stored(0);
    rig.app.selected_channel = 1;
    rig.frame(vec![]);
    rig.take();
    rig.type_key(Key::S, true);
    assert_eq!(rig.take(), ["NoteOn 1 48 89"]);
    rig.app.selected_channel = 2;
    rig.type_key(Key::S, false);
    assert_eq!(rig.take(), ["NoteOff 1 48"]);
}

/// Losing the window focus ends every typed note; the key-up may never come.
#[test]
fn a_focus_loss_ends_the_typed_notes_and_the_next_press_plays() {
    let mut rig = Rig::with_synth();
    rig.type_key(Key::Z, true);
    rig.type_key(Key::X, true);
    rig.take();
    rig.frame(vec![Event::WindowFocused(false)]);
    assert_eq!(rig.take(), ["NoteOff 1 48", "NoteOff 1 50"]);
    // Their keys fade out with them, back to `whiteKey`.
    for _ in 0..40 {
        rig.frame(vec![]);
    }
    assert_eq!(rig.key_level(C4), 0.0);
    rig.frame(vec![Event::WindowFocused(true)]);
    rig.type_key(Key::Z, true);
    assert_eq!(rig.take(), ["NoteOn 1 48 89"]);
}

#[test]
fn space_and_the_silent_keys_play_nothing_and_a_modifier_blocks_and_releases() {
    let mut rig = Rig::with_synth();
    for at in [
        Key::Space,
        Key::A,
        Key::F,
        Key::Num1,
        Key::Minus,
        Key::Quote,
    ] {
        rig.type_key(at, true);
        rig.type_key(at, false);
    }
    let log = rig.take();
    assert!(!log.iter().any(|c| c.starts_with("Note")), "{log:?}");
    // Shift+X plays nothing, and Shift going down releases the held Z.
    rig.type_key(Key::Z, true);
    rig.frame(vec![typed(Key::X, true, Modifiers::SHIFT)]);
    assert_eq!(rig.take(), ["NoteOn 1 48 89", "NoteOff 1 48"]);
    rig.type_key(Key::Z, false);
    rig.type_key(Key::X, false);
    assert_eq!(rig.take(), Vec::<String>::new());
}

#[test]
fn a_focused_text_field_takes_printable_presses_and_a_release_still_ends_the_note() {
    let mut rig = Rig::with_synth();
    rig.type_key(Key::Comma, true);
    assert_eq!(rig.take(), ["NoteOn 1 60 89"]);
    rig.app.navigate(Nav::Open(Overlay::ChannelSettings {
        channel: 1,
        name: "Lead".into(),
    }));
    rig.frame(vec![]);
    rig.frame(vec![]);
    // The name field has the focus: Q types into it and plays nothing.
    rig.frame(vec![
        typed(Key::Q, true, Modifiers::NONE),
        Event::Text("q".into()),
    ]);
    assert_eq!(rig.take(), Vec::<String>::new());
    assert_eq!(
        rig.overlays(),
        [Overlay::ChannelSettings {
            channel: 1,
            name: "Leadq".into()
        }]
    );
    // Its release passes through: it ends 60, which `,` holds.
    rig.type_key(Key::Q, false);
    assert_eq!(rig.take(), ["NoteOff 1 60"]);
}

/// Live note recording is not built yet; typed notes take [`HoneydewApp::key_on`], the touched keys' one way to the
/// engine (and MIDI's), so they will record wherever touched keys do.
#[test]
fn typed_notes_while_rec_is_on_take_the_touched_keys_path() {
    let mut rig = Rig::with_synth();
    rig.app.main_screen.armed = true;
    rig.app.main_screen.playing = true;
    rig.frame(vec![]);
    assert!(rig.app.recording());
    rig.take();
    rig.press(E4);
    rig.release(E4);
    let touched = rig.take();
    rig.type_key(Key::C, true);
    rig.type_key(Key::C, false);
    let typed = rig.take();
    assert_eq!(touched, ["NoteOn 1 52 108", "NoteOff 1 52"]);
    assert_eq!(typed, ["NoteOn 1 52 89", "NoteOff 1 52"]);
    assert!(rig.app.recording(), "a note neither stops nor disarms");
}

#[test]
fn key_lock_snaps_a_typed_note_downward_first_and_its_release_ends_the_snapped_note() {
    let mut rig = Rig::with_synth();
    // C major: S (C#) plays C, H (G#) plays G.
    rig.app.song.key_scale = crate::song::scales::scale_to_stored(0);
    for (at, key) in [(Key::S, 48), (Key::H, 55), (Key::C, 52)] {
        rig.type_key(at, true);
        rig.type_key(at, false);
        assert_eq!(
            rig.take(),
            [format!("NoteOn 1 {key} 89"), format!("NoteOff 1 {key}")]
        );
    }
}

/// `Set MIDI Note` takes a typed key as it takes a MIDI one: one undo step, then the key plays.
#[test]
fn set_midi_note_learns_a_typed_key() {
    let mut rig = Rig::with_synth();
    rig.app.add_default_drums(0);
    rig.edit(2);
    rig.slide_split();
    rig.frame(vec![]);
    let row = egui::Id::new("step_seq").with(("row", 1_usize));
    let header = rig.ctx.read_response(row).expect("drawn").rect;
    rig.secondary_click(header.center());
    rig.frame(vec![]);
    rig.click_id(
        egui::Id::new("step_seq")
            .with("row menu")
            .with((0_usize, 5_usize)),
    );
    rig.take();
    let note = |rig: &Rig| {
        let kit = rig.app.song.channels[2].drum_kit.as_ref().unwrap();
        kit.slots[1].midi_note()
    };
    assert_eq!(note(&rig), None);
    rig.type_key(Key::M, true);
    assert_eq!(note(&rig), Some(59));
    assert_eq!(
        rig.take(),
        ["ReplaceSong adopting no-resync", "NoteOn 2 59 89"]
    );
}

/// A Drums channel plays the pad whose effective note is the typed one (its stored note, else 48 + its pad index), as
/// a MIDI key does; key-lock leaves pads alone.
#[test]
fn a_typed_note_plays_the_drum_pad_whose_note_matches() {
    use crate::audio_engine::SongAudioEngine;
    use honeydew_audio::device::AudioRenderer;
    let render = |engine: &mut SongAudioEngine| {
        let mut output = vec![0.0; 4096 * 2];
        engine.render(&mut output, 2);
        output
    };
    // Typed `at` on a fresh default kit (channel 1), its pad 1 stored on `stored`: what the engine played, and the
    // same song's engine given `key` at 0.7.
    let play = |at: Key, stored: Option<u8>, key: u8| {
        let mut rig = Rig::new();
        rig.app.add_default_drums(0);
        rig.app.song.key_scale = crate::song::scales::scale_to_stored(0);
        let kit = rig.app.song.channels[1].drum_kit.as_mut().unwrap();
        kit.slots[1].file.midi_key = stored.map_or(u16::MAX, u16::from);
        rig.click(OUTSIDE);
        // The engine the recorder renders comes with a rebuild.
        rig.app.song_edited(SongEdit::Final, 0);
        rig.frame(vec![]);
        rig.take();
        rig.type_key(at, true);
        assert_eq!(rig.take(), [format!("NoteOn 1 {key} 89")], "unsnapped");
        let played = render(rig.rec.0.borrow_mut().engine.as_mut().unwrap());
        let samples = &rig.app.audio.samples;
        let mut engine = SongAudioEngine::with_samples(&rig.app.song, 48_000.0, samples).unwrap();
        engine.note_on(1, key, VELOCITY);
        (played, render(&mut engine))
    };
    let sounds = |out: &[f32]| out.iter().any(|s| s.abs() > 1e-4);
    // S (49) is pad 1's default note; with pad 1 on 57 it plays nothing, and N (57) plays pad 1.
    let (played, reference) = play(Key::S, None, 49);
    assert!(sounds(&played) && played == reference);
    let (played, _) = play(Key::S, Some(57), 49);
    assert!(!sounds(&played));
    let (played, reference) = play(Key::N, Some(57), 57);
    assert!(sounds(&played) && played == reference);
}

#[test]
fn the_piano_lights_a_typed_note_and_fades_it_after_its_release() {
    let mut rig = Rig::with_synth();
    rig.type_key(Key::Z, true);
    // The note went on after the screens: the next frame, which the app asks for, lights it.
    let asked = rig.out.viewport_output[&egui::ViewportId::ROOT].repaint_delay;
    assert_eq!(asked, std::time::Duration::ZERO);
    rig.frame(vec![]);
    assert!(
        (rig.key_level(C4) - 0.9).abs() < 0.01,
        "{}",
        rig.key_level(C4)
    );
    rig.frame(vec![]);
    assert!((rig.key_level(C4) - 0.99).abs() < 0.01);
    rig.type_key(Key::Z, false);
    rig.frame(vec![]);
    let fading = rig.key_level(C4);
    assert!(fading < 0.99 && fading > 0.5, "{fading}");
    // D (50) stays dark.
    assert!(rig.key_level(pos2(3.0 * U, LOW)).abs() < 0.01);
}

#[test]
fn a_key_typed_and_let_go_within_one_frame_sounds_for_the_tap_gate() {
    // Its note-off came with its note-on, which the engine drains together: a silent note. It waits
    // for the tap gate, as a tapped piano key's does.
    let mut rig = Rig::with_synth();
    let tapped = rig.time + 0.1;
    let both = [true, false].map(|down| typed(Key::Z, down, Modifiers::NONE));
    rig.frame(both.to_vec());
    assert_eq!(rig.take(), ["NoteOn 1 48 89"]);
    rig.assert_gate(tapped, "NoteOff 1 48");
    // Pressed again within its gate: the gated note-off goes first, so it never cuts the new note.
    let tapped = rig.time + 0.1;
    rig.frame(both.to_vec());
    rig.next_frame_at(tapped + TAP_GATE / 2.0);
    rig.type_key(Key::Z, true);
    assert_eq!(
        rig.take(),
        ["NoteOn 1 48 89", "NoteOff 1 48", "NoteOn 1 48 89"]
    );
    rig.type_key(Key::Z, false);
    assert_eq!(rig.take(), ["NoteOff 1 48"]);
}

#[test]
fn a_gated_typed_note_off_goes_at_once_on_the_web_pages_hide_or_a_focus_loss() {
    // A hidden page runs no frame to end the gate, and a focus loss stops every note.
    for hide in [true, false] {
        let mut rig = Rig::with_synth();
        let both = [true, false].map(|down| typed(Key::Z, down, Modifiers::NONE));
        let tapped = rig.time + 0.1;
        rig.frame(both.to_vec());
        assert_eq!(rig.take(), ["NoteOn 1 48 89"]);
        if hide {
            rig.app.on_hidden();
        } else {
            // Within the gate: only the focus loss sends it.
            rig.next_frame_at(tapped + TAP_GATE / 4.0);
            rig.frame(vec![Event::WindowFocused(false)]);
        }
        assert_eq!(rig.take(), ["NoteOff 1 48"], "hide {hide}");
        rig.frame(vec![]);
        assert_eq!(rig.take(), Vec::<String>::new(), "hide {hide}");
    }
}
