//! The keyboard settings panel (icon 48): [`crate::ui::keyboard_settings`]'s panel over the keyboard, its key and
//! scale those of the song's key lock.

use eframe::egui;

use super::{AppCtx, Nav, Overlay};
use crate::settings::Settings;
use crate::song::chords::{ChordSet, minor_default};
use crate::song::{LaneKind, SET_DEFAULT, SET_EDITED, SET_LOADED, Song};
use crate::ui::keyboard_settings::{
    ChordControls, KEYS, KeyboardMode, KeyboardSettings, KeyboardSettingsAction as A, SetItem,
    TWO_ROWS_H, scale_from_stored, scale_to_stored,
};
use crate::ui::rack::PresetTarget;

/// What the panel keeps between frames: the values that are not the song's (keyboard mode, wheels, two rows).
pub type State = KeyboardSettings;

/// Draws the strip under the transport bar, above the keys; the transport's icon 48 closes it again. It shows
/// the song's key lock, and a key or scale the user picks (or `Reset`) is a param edit: an undo step, with no
/// rebuild, as the key lock changes no audio. A stored scale with no picker entry shows as `Chromatic` and stays
/// stored until the user picks another. Two rows are the app setting `showTwoRows` ([`apply`]). Escape closes it, once
/// an open spinner popup has taken its own.
pub fn show(state: &mut State, ui: &mut egui::Ui, app: &mut AppCtx) -> Option<Nav> {
    // Its gaps are the keyboard panel's resize grab and tap toggle, under it (`MainScreen::chrome`).
    let rect = app.regions.strip;
    let song = &mut *app.song;
    state.key = usize::try_from(song.key_root)
        .ok()
        .filter(|&k| k < KEYS.len())
        .unwrap_or(0);
    state.scale = scale_from_stored(song.key_scale);
    state.two_rows = app.settings.show_two_rows;
    state.wheels = app.settings.show_wheels;
    state.mode = app.settings.keyboard_kind();
    state.notes = notes_channel(song, *app.selected);
    state.chords = chord_controls(song);
    let action = state.show(ui, &app.cx, rect);
    if crate::ui::close_pressed(ui.ctx()) {
        return Some(Nav::Close);
    }
    match action? {
        A::ChordSet(item) => return chord_set_item(item, app),
        action => {
            if apply(action, state, &mut *app.song, app.settings) {
                app.param_edited();
            }
        }
    }
    None
}

/// The current channel's first lane is a notes lane, or there is none: the chosen keyboard kind plays.
pub fn notes_channel(song: &Song, selected: usize) -> bool {
    (song.channels.get(selected))
        .and_then(|c| c.lanes.first())
        .is_none_or(|l| l.kind == LaneKind::Notes)
}

/// The project scale's default chord set is `Minor`: by its stored id.
fn minor(song: &Song) -> bool {
    minor_default(song.key_scale)
}

/// The `ChordSet` button's text: `Default: <set>` while the set is the untouched default (its name by the project
/// scale), `<name>` once loaded or saved, `* <name>` once edited.
pub fn set_label(song: &Song) -> String {
    let k = &song.file.keyboard;
    match k.set_state {
        SET_DEFAULT => format!("Default: {}", if minor(song) { "Minor" } else { "Major" }),
        SET_EDITED => format!("* {}", k.chord_set_name),
        _ => k.chord_set_name.clone(),
    }
}

/// The set the chord keyboard plays: the default by the project scale while it is untouched (or the stored one does
/// not read), else the stored one; transposed to the project key.
pub fn chord_set(song: &Song) -> ChordSet {
    let k = &song.file.keyboard;
    let stored = (k.set_state != SET_DEFAULT)
        .then(|| ChordSet::parse(&k.chord_json).ok())
        .flatten();
    stored
        .unwrap_or_else(|| ChordSet::default_set(minor(song)))
        .transposed(song.key_root.rem_euclid(12))
}

/// The strip's chord values, from the song's `KEYB`.
fn chord_controls(song: &Song) -> ChordControls {
    let k = &song.file.keyboard;
    ChordControls {
        set_label: set_label(song),
        octave: k.octave_shift.clamp(-2, 2),
        velocity: k.velocity,
        timing_ms: k.timing,
        hold: k.hold,
    }
}

/// A `ChordSet` ≡ item:
/// - `Default` reloads the default set by the project scale: an undo step;
/// - `Load` opens the chord set picker;
/// - `Save` marks the set saved at once, an undo step, and asks for the file ([`crate::project_files`]);
/// - `Open Editor` / `Close Editor` toggled the editor (the strip's own state).
fn chord_set_item(item: SetItem, app: &mut AppCtx) -> Option<Nav> {
    let song = &mut *app.song;
    match item {
        SetItem::Default => {
            reload_default(song);
            app.param_edited();
        }
        SetItem::Load => {
            return Some(Nav::Open(Overlay::PresetPicker {
                channel: *app.selected,
                target: PresetTarget::ChordSet,
            }));
        }
        SetItem::Save => {
            let set = save_set(song);
            app.files.ask_chord_set(set);
            app.param_edited();
        }
        SetItem::Editor => {}
    }
    None
}

/// `Save` marks the set saved at once. Returns the set in use for the file.
fn save_set(song: &mut Song) -> ChordSet {
    let set = chord_set(song);
    // The set in use is what KEYB keeps from now on (an untouched default's stored JSON may be another scale's).
    song.file.keyboard.chord_json = set.to_json();
    song.file.keyboard.set_state = SET_LOADED;
    set
}

/// KEYB's set becomes the project scale's default, `Major` or `Minor`, untouched (`ChordSet` ≡ `Default`, and a scale
/// change while the set is the untouched default).
fn reload_default(song: &mut Song) {
    song.file.keyboard.set_state = SET_DEFAULT;
    song.set_key_scale(song.key_scale);
}

/// Applies the strip's `action` (`state` already holds it) to the song and the app settings; whether the song changed,
/// an undo step.
/// - The key and the scale are the song's key lock; `Reset` sets both back and turns two rows off (with the wheels and
///   key-lock), and resets `KEYB`'s chords: the default set by the project scale, `Octave` 0, `Timing` 0, `Velocity`
///   0.5, `Hold` off.
/// - Two rows on raises the panel's preferred height to [`TWO_ROWS_H`] if lower; off leaves it.
/// - Picking `Chords` or `Strum` turns the chord keyboard's Hold off, an edit when it was on.
fn apply(action: A, state: &State, song: &mut Song, settings: &mut Settings) -> bool {
    match action {
        A::Key(key) => song.key_root = key as i32,
        A::Scale(scale) => song.set_key_scale(scale_to_stored(scale)),
        A::Reset => {
            song.key_root = state.key as i32;
            let k = &mut song.file.keyboard;
            (k.octave_shift, k.timing, k.velocity, k.hold) = (0, 0.0, 0.5, false);
            // The untouched default, which the scale below reloads.
            k.set_state = SET_DEFAULT;
            song.set_key_scale(scale_to_stored(state.scale));
            settings.show_two_rows = false;
            settings.show_wheels = false;
            settings.keyboard_kind = KeyboardMode::Classic as u8;
        }
        A::TwoRows(on) => {
            settings.show_two_rows = on;
            if on {
                settings.preferred_kbd_size = settings.preferred_kbd_size.max(TWO_ROWS_H);
            }
            return false;
        }
        A::Mode(mode @ (KeyboardMode::Chords | KeyboardMode::Strum)) => {
            settings.keyboard_kind = mode as u8;
            // KEYB's vertical mode follows the kind (0 Chords, 1 Strum).
            let k = &mut song.file.keyboard;
            let vertical = u32::from(mode == KeyboardMode::Strum);
            let edited = k.hold || k.vertical_mode != vertical;
            (k.hold, k.vertical_mode) = (false, vertical);
            return edited;
        }
        A::Octave(shift) => song.file.keyboard.octave_shift = shift,
        A::Velocity(v) => song.file.keyboard.velocity = v,
        A::Timing(ms) => song.file.keyboard.timing = ms,
        A::Hold(on) => song.file.keyboard.hold = on,
        A::ChordSet(_) => return false,
        A::Wheels(on) => {
            settings.show_wheels = on;
            return false;
        }
        A::Mode(mode) => {
            settings.keyboard_kind = mode as u8;
            return false;
        }
    }
    true
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::ui::keyboard_settings::CHROMATIC;
    use crate::ui::keyboard_settings::SCALES;

    #[test]
    fn two_rows_is_the_setting_and_on_raises_the_preferred_height_to_6_u() {
        let (mut song, mut settings) = (Song::default(), Settings::default());
        let state = State::default();
        assert!(!apply(A::TwoRows(true), &state, &mut song, &mut settings));
        assert!(settings.show_two_rows);
        assert_eq!(settings.preferred_kbd_size, 6.0);
        // Off never shrinks it; a taller preference stays.
        assert!(!apply(A::TwoRows(false), &state, &mut song, &mut settings));
        assert!(!settings.show_two_rows);
        assert_eq!(settings.preferred_kbd_size, 6.0);
        settings.preferred_kbd_size = 8.0;
        apply(A::TwoRows(true), &state, &mut song, &mut settings);
        assert_eq!(settings.preferred_kbd_size, 8.0);
        assert_eq!(song, Song::default(), "no song edit");
    }

    #[test]
    fn reset_turns_two_rows_and_the_wheels_off_with_key_lock() {
        let mut song = Song {
            key_root: 2,
            key_scale: scale_to_stored(17),
            ..Song::default()
        };
        let mut settings = Settings {
            show_two_rows: true,
            show_wheels: true,
            preferred_kbd_size: 6.0,
            ..Settings::default()
        };
        // The panel's state after `Reset`: every default.
        assert!(apply(A::Reset, &State::default(), &mut song, &mut settings));
        assert_eq!(
            (song.key_root, song.key_scale),
            (0, scale_to_stored(CHROMATIC))
        );
        assert!(!settings.show_two_rows && !settings.show_wheels);
        // The panel keeps its height.
        assert_eq!(settings.preferred_kbd_size, 6.0);
    }

    #[test]
    fn the_wheels_toggle_is_the_show_wheels_setting_and_no_song_edit() {
        let (mut song, mut settings) = (Song::default(), Settings::default());
        let state = State::default();
        assert!(!apply(A::Wheels(true), &state, &mut song, &mut settings));
        assert!(settings.show_wheels);
        assert!(!apply(A::Wheels(false), &state, &mut song, &mut settings));
        assert!(!settings.show_wheels);
        assert_eq!(song, Song::default());
    }

    #[test]
    fn picking_chords_or_strum_turns_hold_off() {
        let state = State::default();
        let mut settings = Settings::default();
        for mode in [KeyboardMode::Chords, KeyboardMode::Strum] {
            let mut song = Song::default();
            song.file.keyboard.hold = true;
            assert!(
                apply(A::Mode(mode), &state, &mut song, &mut settings),
                "{mode:?}"
            );
            assert!(!song.file.keyboard.hold);
            // Already off: no edit.
            assert!(!apply(A::Mode(mode), &state, &mut song, &mut settings));
        }
        // Classic and Scale leave it.
        for mode in [KeyboardMode::Classic, KeyboardMode::Scale] {
            let mut song = Song::default();
            song.file.keyboard.hold = true;
            assert!(!apply(A::Mode(mode), &state, &mut song, &mut settings));
            assert!(song.file.keyboard.hold, "{mode:?}");
        }
        // The kind is the `keyboardKind` setting: the last one picked, Scale.
        let kind = KeyboardMode::Scale as u8;
        assert_eq!(
            settings,
            Settings {
                keyboard_kind: kind,
                ..Settings::default()
            }
        );
    }

    #[test]
    fn the_chord_controls_edit_the_songs_keyb_and_strum_sets_its_vertical_mode() {
        let (mut song, mut settings) = (Song::default(), Settings::default());
        let state = State::default();
        for action in [
            A::Octave(-2),
            A::Velocity(0.75),
            A::Timing(-20.0),
            A::Hold(true),
        ] {
            assert!(
                apply(action, &state, &mut song, &mut settings),
                "{action:?}"
            );
        }
        let k = &song.file.keyboard;
        assert_eq!(
            (k.octave_shift, k.velocity, k.timing, k.hold),
            (-2, 0.75, -20.0, true)
        );
        // Strum: Hold off and KEYB's vertical mode 1; Chords: 0.
        assert!(apply(
            A::Mode(KeyboardMode::Strum),
            &state,
            &mut song,
            &mut settings
        ));
        assert_eq!(
            (song.file.keyboard.hold, song.file.keyboard.vertical_mode),
            (false, 1)
        );
        assert!(apply(
            A::Mode(KeyboardMode::Chords),
            &state,
            &mut song,
            &mut settings
        ));
        assert_eq!(song.file.keyboard.vertical_mode, 0);
        assert!(!apply(
            A::Mode(KeyboardMode::Chords),
            &state,
            &mut song,
            &mut settings
        ));
    }

    #[test]
    fn the_set_label_and_the_set_in_use_follow_the_keyb_state_the_scale_and_the_key() {
        let mut song = Song::default();
        assert_eq!(set_label(&song), "Default: Major");
        // A minor scale: the Minor default.
        song.key_scale = scale_to_stored(SCALES.iter().position(|&s| s == "Dorian").unwrap());
        assert_eq!(set_label(&song), "Default: Minor");
        assert_eq!(chord_set(&song), ChordSet::default_set(true).transposed(0));
        // Untouched, the stored JSON is not read; loaded, it is, by its name; edited, with `* `.
        song.file.keyboard.chord_json = ChordSet::default_set(false).transposed(5).to_json();
        song.file.keyboard.chord_set_name = "Mine".into();
        assert_eq!(chord_set(&song).chords[0].quality, 1);
        song.file.keyboard.set_state = crate::song::SET_LOADED;
        assert_eq!(set_label(&song), "Mine");
        song.file.keyboard.set_state = crate::song::SET_EDITED;
        assert_eq!(set_label(&song), "* Mine");
        // Played in the project key: a set written in F transposes to D (−3 semitones).
        song.key_root = 2;
        let set = chord_set(&song);
        assert_eq!(set.root_note, 2);
        assert_eq!((set.chords[0].base_note, set.chords[0].octave), (2, 3));
    }

    #[test]
    fn a_notes_channel_is_one_whose_first_lane_is_notes_or_none() {
        let mut song = Song::default();
        assert!(!notes_channel(&song, 0), "the master's lane is automation");
        assert!(notes_channel(&song, 5), "no channel");
        song.add_channel(crate::song::ChannelKind::Synth(
            crate::song::SynthKind::MiniSynth,
        ));
        assert!(notes_channel(&song, 1));
        song.add_channel(crate::song::ChannelKind::Drums);
        assert!(!notes_channel(&song, 2));
    }

    #[test]
    fn save_keeps_the_set_in_use_and_marks_it_saved() {
        // An untouched default in a minor scale, D: the stored JSON is still the built-in Major one.
        let mut song = Song {
            key_root: 2,
            key_scale: 10,
            ..Song::default()
        };
        let before = chord_set(&song);
        assert_eq!(before, ChordSet::default_set(true).transposed(2));
        let saved = save_set(&mut song);
        assert_eq!(saved, before);
        assert_eq!(song.file.keyboard.set_state, SET_LOADED);
        assert_eq!(
            chord_set(&song),
            before,
            "the keyboard plays on what it played"
        );
    }

    #[test]
    fn while_the_set_is_the_untouched_default_a_scale_pick_or_reset_reloads_the_default() {
        let (mut song, mut settings) = (Song::default(), Settings::default());
        let state = State::default();
        let minor = SCALES.iter().position(|&s| s == "Minor Natural").unwrap();
        assert!(apply(A::Scale(minor), &state, &mut song, &mut settings));
        let k = &song.file.keyboard;
        assert_eq!(
            k.chord_json,
            ChordSet::default_set(true).transposed(0).to_json()
        );
        assert_eq!(
            (k.chord_set_name.as_str(), k.set_state),
            ("Minor", SET_DEFAULT)
        );
        // Reset: Chromatic, so Major again.
        assert!(apply(A::Reset, &state, &mut song, &mut settings));
        let k = &song.file.keyboard;
        assert_eq!(
            k.chord_json,
            ChordSet::default_set(false).transposed(0).to_json()
        );
        assert_eq!(k.chord_set_name, "Major");
        // A loaded set stays as it is.
        song.file.keyboard.set_state = SET_LOADED;
        song.file.keyboard.chord_set_name = "Mine".into();
        let json = song.file.keyboard.chord_json.clone();
        apply(A::Scale(minor), &state, &mut song, &mut settings);
        assert_eq!(
            (
                song.file.keyboard.chord_json.as_str(),
                song.file.keyboard.chord_set_name.as_str()
            ),
            (json.as_str(), "Mine")
        );
    }
}
