//! The Scale panel (ID 50), UIHeight 6.5 U.

use eframe::egui::{Color32, Rect, Ui};

#[cfg(test)]
use super::pitcher_2::{NOTE_W, NOTE_X, NOTES, drawn_bars};
use super::pitcher_2::{note_bars, note_buttons, scale_of, write_scale};
use crate::panels::build::*;
use crate::params::{menu_index, menu_value};
use crate::song::scales::SCALE_MASKS;
use crate::ui::UiCtx;

/// The `Scale` items' note masks C…B relative to the key (most significant bit C, 1 = in scale), in p2's order: the
/// project scales' [`SCALE_MASKS`] (the same 33 scales in the same order, checked below), then `Custom` with none.
const MASKS: [Option<u16>; 34] = {
    let mut masks = [None; 34];
    let mut i = 0;
    while i < SCALE_MASKS.len() {
        masks[i] = Some(SCALE_MASKS[i]);
        i += 1;
    }
    masks
};
/// `Custom` is shown but cannot be picked: a note edit that fits no scale selects it.
const CUSTOM: usize = 33;

/// Couples `Key` (p1), `Scale` (p2) and the notes p4–p15 after an edit. Only user edits drive it.
fn couple(module: &mut ModuleInstance, _: &mut PanelState, edited: &[u32]) -> Vec<usize> {
    let params = edited.iter().filter_map(|&n| match n {
        1..=12 => Some(3 + n as usize),
        13 => Some(1),
        14 => Some(2),
        _ => None,
    });
    couple_scale(&mut module.params, &params.collect::<Vec<_>>())
}

/// The coupling by param, the one implementation: a write of `Key` (p1) or `Scale` (p2) writes the scale's mask
/// transposed to the key to the notes p4–p15, and a write of a note re-selects the `Scale` that fits, or `Custom`.
/// Returns the params it changed. The panel runs it for edits; the engine for automation.
pub(crate) fn couple_scale(params: &mut [f32], edited: &[usize]) -> Vec<usize> {
    if params.len() < 16 {
        return Vec::new();
    }
    let (key, scale) = (
        menu_index(params[1], 12),
        menu_index(params[2], MASKS.len()),
    );
    if edited.iter().any(|p| [1, 2].contains(p)) {
        let changed = write_scale(&mut params[4..16], &MASKS, key, scale);
        return changed.into_iter().map(|i| 4 + i).collect();
    }
    if edited.iter().any(|p| (4..16).contains(p)) {
        let picked = scale_of(&params[4..16], &MASKS, key, CUSTOM);
        if picked != scale {
            params[2] = menu_value(picked, MASKS.len());
            return vec![2];
        }
    }
    Vec::new()
}

/// The note row: the `controlFill` strip and the note bars behind the note buttons #1–#12 (p4–p15).
fn bars(
    ui: &Ui,
    cx: &UiCtx,
    module: &ModuleInstance,
    _: &PanelState,
    rects: &[(u32, Rect)],
    _: Rect,
    channel: Color32,
) {
    let notes = module.params.get(4..16).unwrap_or_default();
    note_bars(ui, cx, rects, 1, notes, channel);
}

pub fn panel() -> Option<Panel> {
    let panel = Panel::new(vstack([
        hstack(note_buttons(1, 4)).h(Size::U(1.5)).mb(0.2),
        hstack([
            spinner(13, "Key").p(1),
            spinner(14, "Scale").p(2).disabled_items(&[CUSTOM]),
            toggle(15, "Snap").p(3).value_text(),
        ]),
    ]));
    Some(panel.underlay(bars).hook(couple))
}

/// Note button `i` (C = 0), `#1`… on p4…: no fill, the label `text`, at 25 % when off.
#[cfg(test)]
const fn note(i: usize) -> Row {
    let rect = [NOTE_X[i], 0.0, NOTE_W[i], 1.5];
    row(
        1 + i as u32,
        Widget::Button { round: false },
        NOTES[i],
        rect,
    )
    .p(4 + i)
    .colors(ButtonColors {
        fill_on: Ink::Clear,
        fill_off: Ink::Clear,
        text_on: Ink::Text,
        text_off: None,
    })
}

#[cfg(test)]
pub const REFERENCE: Reference = Reference {
    size: [14.0, 4.49],
    settings: &[],
    cases: &[Case {
        rows: &[
            note(0),
            note(1),
            note(2),
            note(3),
            note(4),
            note(5),
            note(6),
            note(7),
            note(8),
            note(9),
            note(10),
            note(11),
            row(13, Widget::Spinner, "Key", [0.0, 1.7, 4.66, 2.79]).p(1),
            // `Custom` (item 33) shows but cannot be picked.
            row(14, Widget::Spinner, "Scale", [4.66, 1.7, 4.67, 2.79])
                .p(2)
                .disabled_items(&[33]),
            row(15, Widget::Toggle, "Snap", [9.33, 1.7, 4.67, 2.79])
                .p(3)
                .value_text(),
        ],
        ..Case::DEFAULT
    }],
};

#[cfg(test)]
mod tests {
    use super::*;

    /// The hook the panel runs, which the tests go through: a panel without it fails them.
    fn couple(m: &mut ModuleInstance, s: &mut PanelState, edited: &[u32]) -> Vec<usize> {
        (panel().unwrap().hook.expect("a hook"))(m, s, edited)
    }

    /// The notes p4–p15 that are on, as note names.
    fn on(m: &ModuleInstance) -> Vec<&str> {
        (0..12)
            .filter(|&i| m.params[4 + i] >= 0.5)
            .map(|i| NOTES[i])
            .collect()
    }

    #[test]
    fn the_note_bars_follow_p4_to_p15_under_the_note_buttons() {
        use crate::panels::tests::{Rig, card};
        use eframe::egui::Color32;
        let control_fill = crate::theme::theme_by_id(0)
            .colors
            .control_fill
            .to_color32();
        let (on, off) = (
            Color32::RED.gamma_multiply(0.2),
            control_fill.gamma_multiply(0.2),
        );
        let mut rig = Rig::new(50, panel().unwrap(), card(6.5).1);
        // C major by default: C D E F G A B.
        let major = [on, off, on, off, on, on, off, on, off, on, off, on];
        assert_eq!(drawn_bars(&rig, 1), major);
        // B off, A# on: the bars follow p4–p15.
        rig.module.params[4 + 11] = 0.0;
        rig.module.params[4 + 10] = 1.0;
        rig.frame(vec![]);
        let mut moved = major;
        moved.swap(10, 11);
        assert_eq!(drawn_bars(&rig, 1), moved);
    }

    #[test]
    fn key_and_scale_rewrite_the_notes_and_a_note_tap_picks_the_scale() {
        let mut m = ModuleInstance::new(50, 2);
        let mut state = PanelState::new(&panel().unwrap());
        assert_eq!(on(&m), ["C", "D", "E", "F", "G", "A", "B"], "C major");
        assert!(couple(&mut m, &mut state, &[]).is_empty());
        // A minor pentatonic.
        m.params[1] = menu_value(9, 12);
        m.params[2] = menu_value(14, 34);
        couple(&mut m, &mut state, &[14]);
        assert_eq!(on(&m), ["C", "D", "E", "G", "A"]);
        // Tapping F on: A minor's notes but for G and B, so no scale fits; tapping the rest in gives Minor Natural.
        m.params[4 + 5] = 1.0;
        assert_eq!(couple(&mut m, &mut state, &[6]), [2]);
        assert_eq!(menu_index(m.params[2], 34), CUSTOM);
        m.params[4 + 11] = 1.0;
        couple(&mut m, &mut state, &[12]);
        assert_eq!(menu_index(m.params[2], 34), 12, "Minor Natural in A");
        // A Key edit transposes it: D minor.
        m.params[1] = menu_value(2, 12);
        assert_eq!(couple(&mut m, &mut state, &[13]), [14, 15]);
        assert_eq!(on(&m), ["C", "D", "E", "F", "G", "A", "A#"]);
        // `Custom` has no mask: it leaves the notes.
        let before = m.params.clone();
        m.params[2] = menu_value(CUSTOM, 34);
        assert!(couple(&mut m, &mut state, &[14]).is_empty());
        assert_eq!(m.params[4..16], before[4..16]);
    }

    /// Each scale's mask C…B relative to the key (1 = in scale), in option order.
    const SPEC: [(&str, &str); 33] = [
        ("Major", "101011010101"),
        ("Major Bebop", "101011011101"),
        ("Major Bulgarian", "101001101101"),
        ("Major Pentatonic", "101010010100"),
        ("Major Persian", "110011101001"),
        ("Major Polymode", "101101101011"),
        ("Lydian", "101010110101"),
        ("Mixolydian", "101011010110"),
        ("Phrygian Dominant", "110011011010"),
        ("Minor Harmonic", "101101011001"),
        ("Minor Hungarian", "111100111001"),
        ("Minor Melodic", "101101010101"),
        ("Minor Natural", "101101011010"),
        ("Minor Neapolitan", "110101011001"),
        ("Minor Pentatonic", "100101010010"),
        ("Minor Polymode", "111011011010"),
        ("Minor Romanian", "101100110110"),
        ("Dorian", "101101010110"),
        ("Phrygian", "110101011010"),
        ("Locrian", "110101101010"),
        ("Chromatic", "111111111111"),
        ("Arabic", "110011011001"),
        ("Blues", "100101110010"),
        ("Diminished", "101101101101"),
        ("Dominant Bebop", "101011010111"),
        ("Egyptian", "101001010010"),
        ("Enigmatic", "110010101011"),
        ("Hirajoshi", "101100011000"),
        ("Iwato", "110001100010"),
        ("Japanese Insen", "110001010010"),
        ("Locrian Super", "110110101010"),
        ("Neapolitan", "110101010101"),
        ("Whole Tone", "101010101010"),
    ];

    /// A new Scale module with `scale` picked on `key`.
    fn pick(key: usize, scale: usize) -> ModuleInstance {
        let mut m = ModuleInstance::new(50, 2);
        let mut state = PanelState::new(&panel().unwrap());
        m.params[1] = menu_value(key, 12);
        m.params[2] = menu_value(scale, 34);
        couple(&mut m, &mut state, &[14]);
        m
    }

    #[test]
    fn the_masks_and_names_are_the_spec_table() {
        use crate::params::{ParamKind, module_spec};
        let ParamKind::Menu { items } = module_spec(50).unwrap().params[2].kind else {
            panic!("p2 is a menu");
        };
        assert_eq!(items.len(), MASKS.len());
        for (i, (name, bits)) in SPEC.iter().enumerate() {
            assert_eq!(items[i], *name, "option {i}");
            assert_eq!(MASKS[i], u16::from_str_radix(bits, 2).ok(), "{name}");
            // The shared table is the keyboard settings' scale list: the same order as these options.
            assert_eq!(crate::song::scales::SCALES[i], *name, "project scale {i}");
        }
        assert_eq!((items[CUSTOM], MASKS[CUSTOM]), ("Custom", None));
    }

    #[test]
    fn all_33_masks_differ() {
        let masks: std::collections::HashSet<u16> = MASKS.iter().flatten().copied().collect();
        assert_eq!(masks.len(), 33);
    }

    #[test]
    fn the_scales_that_had_no_mask_write_their_notes() {
        assert_eq!(
            on(&pick(0, 2)),
            ["C", "D", "F", "F#", "G#", "A", "B"],
            "Major Bulgarian"
        );
        assert_eq!(
            on(&pick(0, 5)),
            ["C", "D", "D#", "F", "F#", "G#", "A#", "B"],
            "Major Polymode"
        );
        assert_eq!(
            on(&pick(0, 15)),
            ["C", "C#", "D", "E", "F", "G", "G#", "A#"],
            "Minor Polymode"
        );
    }

    #[test]
    fn a_pick_sets_note_c_plus_n_to_the_mask_bit_at_n_minus_key() {
        assert_eq!(
            on(&pick(4, 15)),
            ["C", "D", "E", "F", "F#", "G#", "A", "B"],
            "Minor Polymode on E"
        );
        for key in 0..12 {
            for (s, (name, bits)) in SPEC.iter().enumerate() {
                let m = pick(key, s);
                for n in 0..12 {
                    let bit = bits.as_bytes()[(n + 12 - key) % 12] == b'1';
                    assert_eq!(
                        m.params[4 + n] >= 0.5,
                        bit,
                        "{name}, key {key}, note C + {n}"
                    );
                }
            }
        }
    }

    #[test]
    fn a_note_edit_shows_the_scale_whose_mask_fits_or_custom() {
        let mut state = PanelState::new(&panel().unwrap());
        for (s, (name, bits)) in SPEC.iter().enumerate() {
            let mut m = ModuleInstance::new(50, 2);
            m.params[1] = menu_value(7, 12);
            for n in 0..12 {
                m.params[4 + n] = f32::from(bits.as_bytes()[(n + 5) % 12] - b'0');
            }
            couple(&mut m, &mut state, &[1]);
            assert_eq!(menu_index(m.params[2], 34), s, "{name} on G");
        }
        // C and C# fit no scale.
        let mut m = ModuleInstance::new(50, 2);
        m.params[4..16].copy_from_slice(&[1., 1., 0., 0., 0., 0., 0., 0., 0., 0., 0., 0.]);
        assert_eq!(couple(&mut m, &mut state, &[2]), [2]);
        assert_eq!(menu_index(m.params[2], 34), CUSTOM);
    }
}
