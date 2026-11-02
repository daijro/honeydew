//! Typing-keyboard notes: a built-in map of 37 keys, by physical key position, plays MIDI 48–79 on the current channel,
//! through the touched keys' live-input path ([`HoneydewApp::key_on`]). Notes are not keymap actions.
//!
//! Routing: the screens go first, so a control's shortcut has consumed its press (`consume_key` takes the event) and
//! a focused text field has shown its IME. Then a press plays when no modifier is held, no editable text field has the
//! focus and the keymap binds no shortcut to the key alone. Releases always pass: a release ends the note its press
//! started (or, for a press that started none, its mapped note wherever it sounds).
//!
//! Details:
//! - **Key position.** egui's `physical_key` names the key; where the platform gives none (the web), its logical key
//!   stands in. egui reports the numpad's digits as the digit row's, so they play the same notes.
//! - **Modifiers** are egui's: Ctrl, Shift, Alt and Cmd. A change of them between key events or frames releases every
//!   typed note; the keys stay held.
//! - **Channel.** The master (0) plays nothing; the keys are still held. A note ends on the channel it began on (as a
//!   MIDI key does); a release that ends no sounding note sends nothing.
//! - **Key-lock** is the keyboard's (the song's scale, not Chromatic), on every channel but one whose pads replace the
//!   keyboard. No key-lock direction value is stored: the snap tries the downward side first. It applies at the press;
//!   The release ends what the press started, so a key-lock or channel change in between strands nothing.
//! - **Focus loss** ends every typed note and forgets the held keys: Wayland, macOS and the web send no key-up after
//!   it. The web page's hide is a focus loss too. A song change ends nothing.

use eframe::egui::{self, Event, Key, Modifiers};

use super::HoneydewApp;
use crate::keymap::{self, Keymap};
use crate::ui::piano_roll::notes::in_scale;
use crate::ui::views::{self, SongEdit, View};

/// The velocity of every typed note.
pub(super) const VELOCITY: f32 = 0.7;

/// The stored key-lock direction: none is stored, so the snap tries the downward side first.
pub(super) const UP: bool = false;

/// Physical key (named by its US-QWERTY legend) → MIDI note (60 is middle C).
const MAP: [(Key, u8); 37] = {
    use Key::*;
    [
        // Bottom row: white keys.
        (Z, 48),
        (X, 50),
        (C, 52),
        (V, 53),
        (B, 55),
        (N, 57),
        (M, 59),
        (Comma, 60),
        (Period, 62),
        (Slash, 64),
        // Home row: black keys.
        (S, 49),
        (D, 51),
        (G, 54),
        (H, 56),
        (J, 58),
        (L, 61),
        (Semicolon, 63),
        // Top row: white keys.
        (Q, 60),
        (W, 62),
        (E, 64),
        (R, 65),
        (T, 67),
        (Y, 69),
        (U, 71),
        (I, 72),
        (O, 74),
        (P, 76),
        (OpenBracket, 77),
        (CloseBracket, 79),
        // Digits: black keys.
        (Num2, 61),
        (Num3, 63),
        (Num5, 66),
        (Num6, 68),
        (Num7, 70),
        (Num9, 73),
        (Num0, 75),
        (Equals, 78),
    ]
};

/// The note the key at `key`'s position plays.
pub(super) fn note(key: Key) -> Option<u8> {
    MAP.iter().find(|(k, _)| *k == key).map(|&(_, n)| n)
}

/// `note` under key-lock (the scale `mask` on `root`): itself when in the scale (the root always is), else the first
/// scale tone ±1, ±2 … ±11 semitones away, the upward side first when `up`; unchanged when none is.
pub(super) fn snap(note: u8, lock: Option<(u8, u16)>, up: bool) -> u8 {
    let Some((root, mask)) = lock else {
        return note;
    };
    let tone = |n: i32| {
        let n = u8::try_from(n).ok().filter(|&n| n < 128);
        n.is_some_and(|n| n % 12 == root % 12 || in_scale(n, root, mask))
    };
    let n = i32::from(note);
    if tone(n) {
        return note;
    }
    let sides = |d: i32| if up { [n + d, n - d] } else { [n - d, n + d] };
    let found = (1..=11).flat_map(sides).find(|&c| tone(c));
    found.map_or(note, |c| c as u8)
}

/// One typed note change: (channel, note, on).
pub(super) type Note = (usize, u8, bool);

/// How a frame's presses play.
pub(super) struct Route<'a> {
    /// The shortcuts: a key bound alone is theirs.
    pub keys: &'a Keymap,
    /// An editable text field has the focus: it takes printable presses.
    pub typing: bool,
    /// The current channel; the master (0) plays nothing.
    pub channel: usize,
    /// Key-lock: the scale's root and tone mask.
    pub lock: Option<(u8, u16)>,
}

/// The typing keyboard's state, app-wide.
#[derive(Debug, Default)]
pub(super) struct Typed {
    /// Every key held, by position, with the (channel, note) its press started.
    held: Vec<(Key, Option<(usize, u8)>)>,
    /// The typed notes sounding, as (channel, note).
    sounding: Vec<(usize, u8)>,
    /// The modifiers as last seen.
    mods: Modifiers,
}

impl Typed {
    /// What a frame's key `events` (those left after the screens) do, with `mods` held at its end.
    pub(super) fn frame(&mut self, events: &[Event], mods: Modifiers, route: &Route) -> Vec<Note> {
        let mut out = Vec::new();
        for event in events {
            let &Event::Key {
                key,
                physical_key,
                pressed,
                repeat,
                modifiers,
            } = event
            else {
                continue;
            };
            self.modifiers(modifiers, &mut out);
            let at = keymap::key_at(key, physical_key);
            let held = self.held.iter().position(|h| h.0 == at);
            if !pressed {
                let Some(i) = held else {
                    continue;
                };
                // The release ends what the press started, if it still sounds. A press that started nothing (a text
                // field or a modifier had it) ends its mapped note, key-locked now, wherever it sounds.
                let started = self.held.swap_remove(i).1;
                let mapped = note(at).map(|n| snap(n, route.lock, UP));
                let ends = |s: &(usize, u8)| started.map_or(mapped == Some(s.1), |on| on == *s);
                self.sounding.retain(|s| {
                    if ends(s) {
                        out.push((s.0, s.1, false));
                    }
                    !ends(s)
                });
                continue;
            }
            if repeat || held.is_some() {
                continue;
            }
            let free = !(modifiers.any() || route.typing || route.keys.binds_plain(at));
            let started = (note(at).filter(|_| free && route.channel > 0))
                .map(|n| (route.channel, snap(n, route.lock, UP)));
            self.held.push((at, started));
            if let Some(on) = started {
                out.push((on.0, on.1, true));
                if !self.sounding.contains(&on) {
                    self.sounding.push(on);
                }
            }
        }
        self.modifiers(mods, &mut out);
        out
    }

    /// A modifier went down or up since the last look: every typed note ends (its key stays held).
    fn modifiers(&mut self, mods: Modifiers, out: &mut Vec<Note>) {
        if mods != self.mods {
            self.mods = mods;
            self.end_all(out);
        }
    }

    fn end_all(&mut self, out: &mut Vec<Note>) {
        out.extend(self.sounding.drain(..).map(|(c, n)| (c, n, false)));
    }

    /// The window lost the focus: every typed note ends, and no key counts as held.
    pub(super) fn lose_focus(&mut self) -> Vec<Note> {
        self.held.clear();
        let mut out = Vec::new();
        self.end_all(&mut out);
        out
    }

    /// The channels moved: each note and held key follows its channel by `channel`. The notes of a channel that went
    /// are returned, to stop; its keys, still held, hold the master, so their release ends nothing.
    pub(super) fn rekey(&mut self, channel: &dyn Fn(usize) -> Option<usize>) -> Vec<(usize, u8)> {
        for (c, _) in self.held.iter_mut().filter_map(|h| h.1.as_mut()) {
            *c = channel(*c).unwrap_or(0);
        }
        let mut gone = Vec::new();
        self.sounding.retain_mut(|(c, note)| match channel(*c) {
            Some(to) => {
                *c = to;
                true
            }
            None => {
                gone.push((*c, *note));
                false
            }
        });
        gone
    }

    /// The typed notes sounding on `channel`.
    pub(super) fn notes(&self, channel: usize) -> impl Iterator<Item = u8> + '_ {
        (self.sounding.iter())
            .filter(move |s| s.0 == channel)
            .map(|s| s.1)
    }
}

impl HoneydewApp {
    /// The frame's typed notes, after the screens: each note-on answers `Set MIDI Note` in the drum editor first (one
    /// undo step, as a MIDI key does), then plays as a touched key does; each note-off goes to its channel.
    pub(super) fn typed_notes(&mut self, ctx: &egui::Context, tick: u64) {
        let pads = views::drum_editor::pads_channel(&self.song, self.selected_channel).is_some();
        let route = Route {
            keys: keymap::for_os(ctx.os()),
            typing: ctx.output(|o| o.ime.is_some()),
            channel: self.selected_channel,
            lock: self.key_input().lock.filter(|_| !pads),
        };
        let typed = &mut self.typed;
        let (mut notes, lost) = ctx.input(|i| {
            let notes = typed.frame(&i.events, i.modifiers, &route);
            (notes, crate::ui::focus_lost(i))
        });
        // After the frame's keys: a press that came before the loss has no key-up coming either.
        if lost {
            notes.extend(typed.lose_focus());
        }
        for (channel, key, on) in notes {
            if !on {
                self.key_off(channel, key);
                continue;
            }
            let editor = &mut self.slots.drum_editor;
            if matches!(self.nav.view, View::DrumEditor(_))
                && views::drum_editor::learned_note(editor, &mut self.song, key)
            {
                self.song_edited(SongEdit::Final, tick);
            }
            self.key_on(channel, key, VELOCITY);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::song::scales::SCALE_MASKS;

    const C_MAJOR: Option<(u8, u16)> = Some((0, SCALE_MASKS[0]));

    fn built_in() -> &'static Keymap {
        keymap::for_os(egui::os::OperatingSystem::Windows)
    }

    fn route(channel: usize) -> Route<'static> {
        Route {
            keys: built_in(),
            typing: false,
            channel,
            lock: None,
        }
    }

    /// A key event at position `at` (its logical key the same), with no modifier.
    fn key(at: Key, pressed: bool) -> Event {
        Event::Key {
            key: at,
            physical_key: Some(at),
            pressed,
            repeat: false,
            modifiers: Modifiers::NONE,
        }
    }

    fn press(at: Key) -> Event {
        key(at, true)
    }

    fn release(at: Key) -> Event {
        key(at, false)
    }

    /// One frame of `events` on channel 1, with no modifier held at its end.
    fn play(typed: &mut Typed, events: &[Event]) -> Vec<Note> {
        typed.frame(events, Modifiers::NONE, &route(1))
    }

    #[test]
    fn the_map_is_the_37_keys_over_midi_48_to_79_with_five_shared_pairs() {
        use Key::*;
        let rows: [&[(Key, u8)]; 4] = [
            &[
                (Z, 48),
                (X, 50),
                (C, 52),
                (V, 53),
                (B, 55),
                (N, 57),
                (M, 59),
                (Comma, 60),
                (Period, 62),
                (Slash, 64),
            ],
            &[
                (S, 49),
                (D, 51),
                (G, 54),
                (H, 56),
                (J, 58),
                (L, 61),
                (Semicolon, 63),
            ],
            &[
                (Q, 60),
                (W, 62),
                (E, 64),
                (R, 65),
                (T, 67),
                (Y, 69),
                (U, 71),
                (I, 72),
                (O, 74),
                (P, 76),
                (OpenBracket, 77),
                (CloseBracket, 79),
            ],
            &[
                (Num2, 61),
                (Num3, 63),
                (Num5, 66),
                (Num6, 68),
                (Num7, 70),
                (Num9, 73),
                (Num0, 75),
                (Equals, 78),
            ],
        ];
        let all: Vec<(Key, u8)> = rows.concat();
        assert_eq!(all.len(), 37);
        for (k, n) in &all {
            assert_eq!(note(*k), Some(*n), "{k:?}");
        }
        let mut notes: Vec<u8> = all.iter().map(|e| e.1).collect();
        notes.sort_unstable();
        notes.dedup();
        assert_eq!(notes, (48..=79).collect::<Vec<_>>(), "every note once");
        let pairs = [
            (Comma, Q, 60),
            (L, Num2, 61),
            (Period, W, 62),
            (Semicolon, Num3, 63),
            (Slash, E, 64),
        ];
        for (a, b, n) in pairs {
            assert_eq!((note(a), note(b)), (Some(n), Some(n)));
        }
        assert_eq!(all.len() - notes.len(), pairs.len(), "five pairs only");
    }

    #[test]
    fn the_other_keys_play_nothing() {
        use Key::*;
        let silent = [
            A, F, K, Quote, Num1, Num4, Num8, Minus, Backslash, Space, Enter, Tab, ArrowUp,
            ArrowDown, ArrowLeft, ArrowRight, F1, F2, F5, F10, F11, F12, Backtick, Escape,
            Backspace, Delete, Home, PageUp,
        ];
        for k in silent {
            assert_eq!(note(k), None, "{k:?}");
        }
        let mut typed = Typed::default();
        let events: Vec<Event> = silent.iter().map(|&k| press(k)).collect();
        assert_eq!(play(&mut typed, &events), []);
    }

    #[test]
    fn a_key_plays_by_its_position_not_its_layouts_legend() {
        let mut typed = Typed::default();
        // AZERTY: the key in the US "Z" position types "W".
        let azerty = Event::Key {
            key: Key::W,
            physical_key: Some(Key::Z),
            pressed: true,
            repeat: false,
            modifiers: Modifiers::NONE,
        };
        assert_eq!(play(&mut typed, &[azerty]), [(1, 48, true)]);
        // Its release names the same position.
        let up = Event::Key {
            key: Key::W,
            physical_key: Some(Key::Z),
            pressed: false,
            repeat: false,
            modifiers: Modifiers::NONE,
        };
        assert_eq!(play(&mut typed, &[up]), [(1, 48, false)]);
        // Without a position (the web), the logical key stands in.
        let web = |pressed| Event::Key {
            key: Key::Z,
            physical_key: None,
            pressed,
            repeat: false,
            modifiers: Modifiers::NONE,
        };
        assert_eq!(play(&mut typed, &[web(true)]), [(1, 48, true)]);
        assert_eq!(play(&mut typed, &[web(false)]), [(1, 48, false)]);
    }

    #[test]
    fn a_held_modifier_blocks_the_press_and_the_key_is_still_held() {
        for mods in [
            Modifiers::SHIFT,
            Modifiers::CTRL | Modifiers::COMMAND,
            Modifiers::ALT,
            Modifiers::MAC_CMD | Modifiers::COMMAND,
        ] {
            let mut typed = Typed::default();
            let shifted = Event::Key {
                key: Key::Z,
                physical_key: Some(Key::Z),
                pressed: true,
                repeat: false,
                modifiers: mods,
            };
            assert_eq!(typed.frame(&[shifted], mods, &route(1)), [], "{mods:?}");
            // The modifier goes; the held Z stays silent, and a press of it again is ignored.
            assert_eq!(play(&mut typed, &[press(Key::Z)]), [], "{mods:?}");
            assert_eq!(play(&mut typed, &[release(Key::Z)]), [], "{mods:?}");
            assert_eq!(play(&mut typed, &[press(Key::Z)]), [(1, 48, true)]);
        }
    }

    #[test]
    fn a_key_a_shortcut_binds_alone_never_plays() {
        let keys = Keymap::parse(
            r#"{"modifiers":["ctrl","shift","alt","win"],"actions":[
                {"key":"z","action":"play"},{"key":"ctrl+x","action":"cut"}]}"#,
        )
        .unwrap();
        let route = Route {
            keys: &keys,
            ..route(1)
        };
        let mut typed = Typed::default();
        let events = [press(Key::Z), press(Key::X)];
        assert_eq!(
            typed.frame(&events, Modifiers::NONE, &route),
            [(1, 50, true)],
            "X is bound with Ctrl only"
        );
        // The built-in map binds none of the 37 keys alone; it binds space (`play`), which plays no note anyway.
        let mut typed = Typed::default();
        let all: Vec<Event> = MAP.iter().map(|&(k, _)| press(k)).collect();
        assert_eq!(play(&mut typed, &all).len(), 37);
        assert!(built_in().binds_plain(Key::Space));
    }

    #[test]
    fn a_focused_text_field_takes_the_press_and_its_release_still_ends_the_note() {
        let mut typed = Typed::default();
        assert_eq!(play(&mut typed, &[press(Key::Comma)]), [(1, 60, true)]);
        let typing = Route {
            typing: true,
            ..route(1)
        };
        assert_eq!(typed.frame(&[press(Key::Q)], Modifiers::NONE, &typing), []);
        // Q's release passes through: it ends 60, which `,` holds.
        let released = typed.frame(&[release(Key::Q)], Modifiers::NONE, &typing);
        assert_eq!(released, [(1, 60, false)]);
        assert_eq!(play(&mut typed, &[release(Key::Comma)]), []);
    }

    #[test]
    fn a_release_ends_its_note_and_either_key_of_a_pair_ends_the_shared_one() {
        let mut typed = Typed::default();
        assert_eq!(
            play(
                &mut typed,
                &[press(Key::Z), press(Key::Comma), press(Key::Q)]
            ),
            [(1, 48, true), (1, 60, true), (1, 60, true)]
        );
        assert_eq!(typed.notes(1).collect::<Vec<_>>(), [48, 60]);
        assert_eq!(play(&mut typed, &[release(Key::Q)]), [(1, 60, false)]);
        assert_eq!(play(&mut typed, &[release(Key::Comma)]), []);
        assert_eq!(play(&mut typed, &[release(Key::Z)]), [(1, 48, false)]);
        assert_eq!(typed.notes(1).count(), 0);
    }

    #[test]
    fn os_repeats_and_presses_of_a_held_key_never_retrigger() {
        let mut typed = Typed::default();
        assert_eq!(play(&mut typed, &[press(Key::Z)]), [(1, 48, true)]);
        let repeat = Event::Key {
            key: Key::Z,
            physical_key: Some(Key::Z),
            pressed: true,
            repeat: true,
            modifiers: Modifiers::NONE,
        };
        assert_eq!(play(&mut typed, &[repeat.clone(), repeat]), []);
        assert_eq!(play(&mut typed, &[press(Key::Z)]), [], "already held");
        // A release of a key not held is ignored.
        assert_eq!(play(&mut typed, &[release(Key::X)]), []);
        assert_eq!(play(&mut typed, &[release(Key::Z)]), [(1, 48, false)]);
        assert_eq!(play(&mut typed, &[release(Key::Z)]), []);
    }

    #[test]
    fn a_modifier_change_releases_the_typed_notes_until_their_keys_are_pressed_again() {
        let mut typed = Typed::default();
        play(&mut typed, &[press(Key::Z), press(Key::X)]);
        // Shift goes down with no key event: the frame's modifiers tell.
        let shift = typed.frame(&[], Modifiers::SHIFT, &route(1));
        assert_eq!(shift, [(1, 48, false), (1, 50, false)]);
        // Its release releases again (nothing sounds now), and the held keys do not restart.
        assert_eq!(play(&mut typed, &[]), []);
        assert_eq!(play(&mut typed, &[press(Key::Z)]), [], "Z is still held");
        assert_eq!(play(&mut typed, &[release(Key::Z)]), []);
        assert_eq!(play(&mut typed, &[press(Key::Z)]), [(1, 48, true)]);
        // A modifier change inside a frame: the event's own modifiers tell, before its press is tested.
        let ctrl_c = Event::Key {
            key: Key::C,
            physical_key: Some(Key::C),
            pressed: true,
            repeat: false,
            modifiers: Modifiers::CTRL,
        };
        let frame = typed.frame(&[ctrl_c], Modifiers::CTRL, &route(1));
        assert_eq!(frame, [(1, 48, false)]);
    }

    #[test]
    fn an_os_repeat_of_a_key_not_held_plays_nothing() {
        let mut typed = Typed::default();
        let repeat = Event::Key {
            key: Key::Z,
            physical_key: Some(Key::Z),
            pressed: true,
            repeat: true,
            modifiers: Modifiers::NONE,
        };
        assert_eq!(play(&mut typed, &[repeat]), []);
        assert_eq!(play(&mut typed, &[press(Key::Z)]), [(1, 48, true)]);
    }

    /// Key-lock applies at the press, and the release ends what the press started, whatever the key-lock or the current
    /// channel is by then.
    #[test]
    fn the_release_ends_what_the_press_started_whatever_the_lock_and_channel_now() {
        let mut typed = Typed::default();
        let locked = Route {
            lock: C_MAJOR,
            ..route(1)
        };
        let on = typed.frame(&[press(Key::S)], Modifiers::NONE, &locked);
        assert_eq!(on, [(1, 48, true)]);
        let off = typed.frame(&[release(Key::S)], Modifiers::NONE, &route(2));
        assert_eq!(off, [(1, 48, false)]);
        // A pair: the second key's press started 48 as well; it ends nothing once the first release ended it.
        typed.frame(&[press(Key::Z), press(Key::S)], Modifiers::NONE, &locked);
        assert_eq!(play(&mut typed, &[release(Key::S)]), [(1, 48, false)]);
        assert_eq!(play(&mut typed, &[release(Key::Z)]), []);
    }

    /// Wayland, macOS and the web send no key-up after a focus loss, so the loss ends every typed note and forgets the
    /// held keys.
    #[test]
    fn a_focus_loss_ends_every_typed_note_and_forgets_the_held_keys() {
        let mut typed = Typed::default();
        play(&mut typed, &[press(Key::Z), press(Key::X), press(Key::A)]);
        assert_eq!(typed.lose_focus(), [(1, 48, false), (1, 50, false)]);
        assert_eq!(typed.notes(1).count(), 0);
        // The key-up never comes; the next press plays.
        assert_eq!(play(&mut typed, &[press(Key::Z)]), [(1, 48, true)]);
        assert_eq!(
            play(&mut typed, &[release(Key::A)]),
            [],
            "A is no longer held"
        );
    }

    #[test]
    fn the_master_plays_nothing_but_its_keys_are_held() {
        let mut typed = Typed::default();
        assert_eq!(
            typed.frame(&[press(Key::Z)], Modifiers::NONE, &route(0)),
            []
        );
        assert_eq!(play(&mut typed, &[press(Key::Z)]), [], "still held");
    }

    #[test]
    fn a_note_ends_on_the_channel_it_began_on() {
        let mut typed = Typed::default();
        play(&mut typed, &[press(Key::Z)]);
        let on_2 = typed.frame(&[release(Key::Z)], Modifiers::NONE, &route(2));
        assert_eq!(on_2, [(1, 48, false)]);
    }

    #[test]
    fn key_lock_snaps_within_eleven_semitones_downward_first() {
        // C major: C# goes to C (down first), or to D when the stored direction is up.
        assert_eq!(snap(49, C_MAJOR, false), 48);
        assert_eq!(snap(49, C_MAJOR, true), 50);
        // Scale tones stay; F# goes to F, A# to A.
        assert_eq!(snap(52, C_MAJOR, false), 52);
        assert_eq!(snap(54, C_MAJOR, false), 53);
        assert_eq!(snap(70, C_MAJOR, false), 69);
        // No lock, no snap.
        assert_eq!(snap(49, None, false), 49);
        // The root always counts: a mask without it still snaps to the root.
        assert_eq!(snap(50, Some((2, 0)), false), 50);
        assert_eq!(snap(51, Some((2, 0)), false), 50);
        assert_eq!(snap(61, Some((2, 0)), false), 62, "the nearest root, 1 up");
        assert_eq!(snap(56, Some((2, 0)), false), 50, "6 down before 6 up");
        // The press and the release both snap: C# plays C, and its release ends C.
        let mut typed = Typed::default();
        let locked = Route {
            lock: C_MAJOR,
            ..route(1)
        };
        let on = typed.frame(&[press(Key::S)], Modifiers::NONE, &locked);
        assert_eq!(on, [(1, 48, true)]);
        let off = typed.frame(&[release(Key::S)], Modifiers::NONE, &locked);
        assert_eq!(off, [(1, 48, false)]);
    }
}
