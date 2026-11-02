use super::spec_rows::ROWS;
use super::*;

/// The effect overview: id, display name, unique name, params, legacy, role.
const OVERVIEW: [(u16, &str, &str, usize, bool, Role); 39] = [
    (3, "Equalizer", "Param EQ", 21, false, Role::Effect),
    (4, "MultiFX", "MultiFX", 7, false, Role::Effect),
    (5, "Limiter", "Limiter (Legacy)", 4, true, Role::Effect),
    (6, "Distortion", "Distortion", 7, false, Role::Effect),
    (7, "Graphic EQ", "Graphic EQ", 9, false, Role::Effect),
    (8, "Compressor", "Compressor", 10, false, Role::Effect),
    (9, "Filter", "Filter", 5, false, Role::Effect),
    (10, "Autoduck", "Trancer (legacy)", 4, true, Role::Effect),
    (11, "Chorus", "Chorus", 5, false, Role::Effect),
    (12, "Reverb", "Reverb (legacy)", 22, true, Role::Effect),
    (13, "Flanger", "Flanger", 12, false, Role::Effect),
    (14, "Phaser", "Phaser", 10, false, Role::Effect),
    (15, "Stereoizer", "Stereorizer", 7, false, Role::Effect),
    (16, "Trance Delay", "Trance Delay", 7, false, Role::Effect),
    (18, "Tape Delay", "Tape Delay", 5, false, Role::Effect),
    (20, "Leveller", "Leveller", 3, false, Role::Effect),
    (24, "Tuner", "Tuner", 1, false, Role::Effect),
    (25, "Pitcher", "Pitcher", 23, false, Role::Effect),
    (26, "Analyzer", "Analyzer", 1, false, Role::Effect),
    (27, "Gate", "Gate", 9, false, Role::Effect),
    (28, "Limiter", "Limiter", 5, false, Role::Effect),
    (29, "Autoduck", "Autoduck", 8, false, Role::Effect),
    (30, "Tuned EQ", "Tuned EQ", 11, false, Role::Effect),
    (32, "Reverb", "Reverb", 16, false, Role::Effect),
    (34, "Spacer", "Spacer", 8, false, Role::Effect),
    (35, "Reverb 2", "Reverb 2", 19, false, Role::Effect),
    (
        36,
        "Multiband Compressor",
        "Multiband Compressor",
        31,
        false,
        Role::Effect,
    ),
    (40, "Comb Filter", "Comb Filter", 12, false, Role::Effect),
    (41, "Waveshaper", "Waveshaper", 21, false, Role::Effect),
    (42, "Tape Stop", "Tape Stop", 8, false, Role::Effect),
    (
        43,
        "Wow & Flutter",
        "Wow & Flutter",
        11,
        false,
        Role::Effect,
    ),
    (44, "Spreader", "Spreader", 5, false, Role::Effect),
    (45, "Pitcher 2", "Pitcher 2", 27, false, Role::Effect),
    (46, "Tremolo", "Tremolo", 12, false, Role::Effect),
    (47, "Note Echo", "Note Echo", 16, false, Role::NoteEffect),
    (48, "Transpose", "Transpose", 3, false, Role::NoteEffect),
    (49, "Randomizer", "Randomizer", 5, false, Role::NoteEffect),
    (50, "Scale", "Scale", 16, false, Role::NoteEffect),
    (
        51,
        "Arpeggiator",
        "Arpeggiator",
        33,
        false,
        Role::NoteEffect,
    ),
];

fn spec(id: u16) -> &'static ModuleSpec {
    module_spec(id).unwrap_or_else(|| panic!("module {id} is not registered"))
}

fn text(id: u16, param: usize, v: f32) -> String {
    spec(id).params[param].display_text(v)
}

fn items(id: u16, param: usize) -> &'static [&'static str] {
    let kind = spec(id).params[param].kind;
    match kind.items() {
        [] => panic!("module {id} param {param} is {kind:?}, not a menu"),
        items => items,
    }
}

#[test]
fn overview_table_matches_the_registry() {
    for (id, display_name, unique_name, count, legacy, role) in OVERVIEW {
        let m = spec(id);
        assert_eq!(m.id, id);
        assert_eq!(m.display_name, display_name, "display name of {id}");
        assert_eq!(m.unique_name, unique_name, "unique name of {id}");
        assert_eq!(m.params.len(), count, "param count of {id}");
        assert_eq!(m.legacy, legacy, "legacy flag of {id}");
        assert_eq!(m.role, role, "role of {id}");
    }
}

#[test]
fn modules_lists_exactly_the_overview_ids_in_order() {
    let ids: Vec<u16> = modules().map(|m| m.id).collect();
    let want: Vec<u16> = OVERVIEW.iter().map(|o| o.0).collect();
    assert_eq!(ids, want);
    assert!(ids.windows(2).all(|w| w[0] < w[1]), "ids not ascending");
    for id in want {
        assert_eq!(module_spec(id).map(|m| m.id), Some(id), "lookup of {id}");
    }
}

#[test]
fn unregistered_ids_have_no_spec() {
    // The gaps of the id range and anything beyond the last module.
    for id in [17, 21, 22, 31, 37, 38, 9999] {
        assert!(module_spec(id).is_none(), "{id} must be unknown");
    }
}

#[test]
fn param_zero_is_enable_everywhere() {
    for (id, ..) in OVERVIEW {
        let p = &spec(id).params[0];
        assert_eq!(
            (p.name, p.default, p.kind),
            ("Enable", 1.0, ParamKind::Toggle)
        );
        assert_eq!(p.display_text(1.0), "On");
        assert_eq!(p.display_text(0.0), "Off");
    }
}

const KEYS: &[&str] = &[
    "C", "C#", "D", "D#", "E", "F", "F#", "G", "G#", "A", "A#", "B",
];

#[test]
fn tab_strips_follow_the_pages_column() {
    let expected: [(u16, &[&str]); 15] = [
        (3, &["BAND 1", "BAND 2", "BAND 3", "BAND 4"]),
        (
            4,
            &[
                "Delay",
                "Reverb",
                "LP Filter",
                "HP Filter",
                "Flanger",
                "Phaser",
                "Panner",
                "Vox",
            ],
        ),
        (12, &["MAIN", "TUNE", "MIXER"]),
        (16, &["1/4", "3/8", "1/2", "3/4", "1/1"]),
        (25, &["Basic", "Advanced"]),
        (
            26,
            &[
                "SPECTRUM", "SCOPE", "LOUDNESS", "STEREO", "PITCH", "SETTINGS",
            ],
        ),
        (27, &["LEVEL", "ENVELOPE"]),
        (30, KEYS),
        (32, &["MAIN", "TUNE", "MIXER"]),
        (35, &["MAIN", "TUNE"]),
        (36, &["Master", "Low", "Mid", "High"]),
        (41, &["SHAPE", "AMP", "EQ", "LFO"]),
        (45, &["TUNING", "EXPRESSION", "OPTIONS"]),
        (47, &["TIME", "PITCH", "VELOCITY", "PAN"]),
        (51, &["GLOBAL", "ARP 1", "ARP 2"]),
    ];
    for (id, ..) in OVERVIEW {
        let want: &[&str] = expected.iter().find(|e| e.0 == id).map_or(&[], |e| e.1);
        assert_eq!(spec(id).tabs, want, "tabs of {id}");
    }
}

#[test]
fn tab_strips_bound_to_a_parameter_equal_its_menu_items() {
    // MultiFX FX kind, Trance Delay time, Tuned EQ tuning (the 12-tab bar).
    for (id, param) in [(4, 1), (16, 1), (30, 9)] {
        assert_eq!(items(id, param), spec(id).tabs, "module {id} param {param}");
    }
}

#[test]
fn defaults_are_normalised_and_menus_have_items() {
    for m in modules() {
        for (i, p) in m.params.iter().enumerate() {
            assert!(
                (0.0..=1.0).contains(&p.default),
                "module {} param {i}: default {}",
                m.id,
                p.default
            );
            if let ParamKind::Menu { .. } | ParamKind::MenuBy { .. } = p.kind {
                assert!(p.kind.items().len() >= 2, "module {} param {i}: menu", m.id);
            }
        }
    }
}

// ---- Distortion and Compressor, written by hand ----

#[test]
fn distortion_by_hand() {
    let m = spec(6);
    let names: Vec<&str> = m.params.iter().map(|p| p.name).collect();
    assert_eq!(
        names,
        [
            "Enable", "Drive", "BitCrush", "Gain", "Stereo", "Kind", "Bias"
        ]
    );
    let defaults: Vec<f32> = m.params.iter().map(|p| p.default).collect();
    assert_eq!(defaults, [1.0, 0.25, 0.0, 1.0, 0.5, 0.0, 0.5]);
    assert!(matches!(m.params[5].kind, ParamKind::MenuBy { .. }));
    for p in [1, 2, 3, 4, 6] {
        assert!(
            matches!(m.params[p].kind, ParamKind::Continuous { .. }),
            "p{p}"
        );
    }
    assert_eq!(text(6, 3, 1.0), "0.0 dB");
    assert_eq!(text(6, 3, 0.0), "-24.0 dB");
    assert_eq!(text(6, 3, 0.5), "-12.0 dB");
    assert_eq!(text(6, 4, 0.5), "0");
    assert_eq!(text(6, 4, 0.0), "-100");
    assert_eq!(text(6, 4, 1.0), "100");
    assert_eq!(text(6, 1, 0.25), "25");
    assert_eq!(text(6, 2, 0.0), "0");
    assert_eq!(text(6, 6, 0.5), "0");
    assert_eq!(items(6, 5), ["Hard", "Tube", "Soft", "Clip"]);
    assert_eq!(text(6, 5, 0.0), "Hard");
    assert_eq!(text(6, 5, 1.0), "Clip");
    assert_eq!(text(6, 5, 0.34), "Tube");
}

#[test]
fn compressor_by_hand() {
    let m = spec(8);
    let names: Vec<&str> = m.params.iter().map(|p| p.name).collect();
    assert_eq!(
        names,
        [
            "Enable",
            "Thresh",
            "Ratio",
            "Attack",
            "Release",
            "Level",
            "Hold",
            "Auto gain",
            "Input gain",
            "RMS"
        ]
    );
    let defaults: Vec<f32> = m.params.iter().map(|p| p.default).collect();
    assert_eq!(
        defaults,
        [1.0, 0.499, 0.0, 0.6, 0.6, 0.5001, 0.0, 1.0, 0.5, 0.0]
    );
    assert_eq!(m.params[7].kind, ParamKind::Toggle);
    assert_eq!(m.params[9].kind, ParamKind::Toggle);
    assert_eq!(text(8, 1, 0.499), "-20.0 dB");
    assert_eq!(text(8, 1, 1.0), "0.0 dB");
    assert_eq!(text(8, 2, 0.0), "1.0:1");
    assert_eq!(text(8, 2, 0.5), "5.8:1");
    assert_eq!(text(8, 2, 1.0), "+inf");
    assert_eq!(text(8, 3, 0.6), "18.0 ms");
    assert_eq!(text(8, 3, 1.0), "50.0 ms");
    assert_eq!(text(8, 4, 0.0), "10.0 ms");
    assert_eq!(text(8, 4, 0.6), "186.4 ms");
    assert_eq!(text(8, 5, 1.0), "20.0 dB");
    assert_eq!(text(8, 6, 0.0), "0.0 ms");
    assert_eq!(text(8, 8, 0.0), "-20.0 dB");
    assert_eq!(text(8, 7, 1.0), "On");
    assert_eq!(text(8, 9, 0.0), "Off");
}

#[test]
fn leveller_gain_follows_the_captured_samples() {
    // v = 0, .05 ... 1.
    let samples = [
        -60.0, -48.0, -41.9, -36.9, -32.5, -28.6, -25.0, -21.6, -18.4, -15.4, -12.4, -9.7, -7.0,
        -4.3, -1.8, 0.5, 2.9, 5.3, 7.5, 9.8, 12.0f32,
    ];
    for (i, s) in samples.into_iter().enumerate() {
        let want = if s > 0.0 {
            format!("+{s:.1} dB")
        } else {
            format!("{s:.1} dB")
        };
        assert_eq!(
            text(20, 1, i as f32 * 0.05),
            want,
            "v = {}",
            i as f32 * 0.05
        );
    }
    assert_eq!(text(20, 1, 0.738), "0 dB");
    assert_eq!(text(20, 2, 0.0), "100.0% L");
    assert_eq!(text(20, 2, 0.5), "mid");
    assert_eq!(text(20, 2, 0.6), "20.0% R");
    assert_eq!(text(20, 2, 1.0), "100.0% R");
}

#[test]
fn menu_index_is_round_of_v_times_n_minus_1() {
    // Value and item index map both ways for any item count; the ends are 0 and 1.
    for len in 1..=40 {
        for i in 0..len {
            assert_eq!(menu_index(menu_value(i, len), len), i, "{i} of {len}");
        }
        assert_eq!(menu_value(0, len), 0.0);
        assert_eq!(menu_value(len - 1, len), if len == 1 { 0.0 } else { 1.0 });
    }
    // Off-grid values read as the nearest item, ties away from zero (the act-ID form).
    assert_eq!(menu_index(0.3, 5), 1);
    assert_eq!(menu_index(0.5, 2), 1);
    assert_eq!(menu_index(0.61, 10), 5);
    assert_eq!(menu_index(0.51, 3), 1);
    assert_eq!(menu_index(0.775, 5), 3);
    assert_eq!(menu_index(-1.0, 4), 0);
    assert_eq!(menu_index(7.0, 4), 3);
    assert_eq!(menu_index(f32::NAN, 4), 0);
    assert_eq!((menu_index(0.5, 1), menu_index(0.5, 0)), (0, 0));
    assert_eq!((menu_value(0, 1), menu_value(5, 3)), (0.0, 1.0));
}

// ---- the whole effect tables, row by row ----

pub(super) fn range_parts(range: &str) -> Option<[&str; 3]> {
    let law = range.split(';').next()?;
    let parts: Vec<&str> = law.split(" … ").collect();
    parts.try_into().ok()
}

pub(super) fn menu_items_of(range: &str) -> Option<Vec<&str>> {
    let list = range
        .strip_prefix("menu: ")
        .or_else(|| range.strip_prefix("options: "))?;
    let list = list
        .strip_suffix(" (binding I)")
        .or_else(|| list.strip_suffix(" (tab bar, binding I)"))
        .unwrap_or(list);
    // "options: -12 … 12 (step 1)" is a stepped range, not a list of labels.
    if list.contains(" … ") {
        return None;
    }
    Some(list.split(", ").collect())
}

/// Rows the tables mark as on/off switches.
pub(super) fn is_toggle_range(range: &str) -> bool {
    range == "options: Off, On"
        || range == "options: OFF, ON"
        || range.starts_with("Off/On")
        || range == "Off … Off … Off"
}

/// Where the registry deliberately differs from a table row, with the reason.
/// (id, param, what the registry holds: "toggle" or the menu items)
const KIND_NOTES: &[(u16, usize, &str)] = &[
    // Filter: the table shows the value text (LP / 48 dB/O); the registry keeps the menu labels.
    (9, 3, "Low Pass, Band Pass, High Pass"),
    (9, 4, "24 dB/Oct, 48 dB/Oct, 96 dB/Oct"),
    // Phaser: the LFO Rate list is cut off after "4 Bars" (the captured list and the preset grid have 13 items);
    // LFO Shape is a menu and Sync a toggle, which the table shows with the generic display.
    (
        14,
        1,
        "1/4, 3/8, 1/2, 3/4, 1/1, 2/1, 4/1, 8/1, 1 Bar, 2 Bars, 4 Bars, 8 Bars, 16 Bars",
    ),
    (14, 8, "Sine, Saw, Sqr"),
    (14, 6, "toggle"),
    // Pitcher (25): p6 is the input channel menu; the twelve "BYP" buttons and "Bypass snapping" are round on/off
    // buttons that the table shows with the generic display.
    (25, 6, "Left channel, Left + Right, Right channel"),
    (25, 10, "toggle"),
    (25, 11, "toggle"),
    (25, 12, "toggle"),
    (25, 13, "toggle"),
    (25, 14, "toggle"),
    (25, 15, "toggle"),
    (25, 16, "toggle"),
    (25, 17, "toggle"),
    (25, 18, "toggle"),
    (25, 19, "toggle"),
    (25, 20, "toggle"),
    (25, 21, "toggle"),
    (25, 22, "toggle"),
    // Spacer: the tables show the value texts (L/R Inversion's options, M/S Inversion's rule), the captured lists have
    // the labels; the value texts are the automation texts.
    (34, 4, "Nothing, Left, Right, Both"),
    (34, 6, "Nothing, Mid, Side, Both"),
    // Multiband Compressor p30 is the "Full / Safe" selector.
    (36, 30, "Full, Safe"),
    // Comb Filter: Mode and Polarity show their first label in all samples of the table (captured lists
    // here); Snap is a round on/off button.
    (40, 1, "Manual, MIDI Note, Audio Pitch"),
    (40, 7, "toggle"),
    (40, 8, "Both +, Left -, Right -, Both -"),
    // Tape Stop: the synced times are beat labels; the table has the 0 BPM capture of
    // the same ten steps as seconds.
    (
        42,
        3,
        "Off, 1/4, 3/8, 1/2, 3/4, 1 beat, 2 beats, 4 beats, 8 beats, 16 beats",
    ),
    (
        42,
        4,
        "Off, 1/4, 3/8, 1/2, 3/4, 1 beat, 2 beats, 4 beats, 8 beats, 16 beats",
    ),
    // Tremolo: the table's sample list is scrambled; the menu labels are the captured ones.
    (
        46,
        2,
        "1/8, 1/4, 3/8, 1/2, 3/4, beat, 2 beats, 4 beats, 8 beats, 16 beats",
    ),
    // Scale: the key menu is written "C … F# … B" in the table.
    (50, 1, "C, C#, D, D#, E, F, F#, G, G#, A, A#, B"),
];

fn note(id: u16, param: usize) -> Option<&'static str> {
    KIND_NOTES
        .iter()
        .find(|n| (n.0, n.1) == (id, param))
        .map(|n| n.2)
}

#[test]
fn rows_names_defaults_and_kinds() {
    let mut seen = 0;
    for &(id, idx, name, default, _, range) in ROWS {
        let ctx = format!("module {id} param {idx} ({name})");
        let p = &spec(id).params[idx];
        let spec_name = if name == "(none)" { "" } else { name };
        assert_eq!(p.name, spec_name, "{ctx}: name");
        // The table's default is the reset value; a few params start elsewhere (CREATION_NOTES).
        match CREATION_NOTES.iter().find(|n| (n.0, n.1) == (id, idx)) {
            Some(n) => assert_eq!(
                (p.default, p.reset),
                (n.2, default),
                "{ctx}: creation value"
            ),
            None => assert!(
                p.default == default || rounded_step(p, default),
                "{ctx}: default {} != {default}",
                p.default
            ),
        }
        seen += 1;

        let is_toggle = is_toggle_range(range);
        let items = p.kind.items();
        match (note(id, idx), &p.kind) {
            (Some("toggle"), kind) => assert_eq!(*kind, ParamKind::Toggle, "{ctx}"),
            (Some(want), _) if !items.is_empty() => {
                assert_eq!(items.join(", "), want, "{ctx}: items")
            }
            (Some(_), kind) => panic!("{ctx}: expected a menu, got {kind:?}"),
            (None, ParamKind::Toggle) => {
                assert!(is_toggle, "{ctx}: toggle where the table has {range:?}")
            }
            (None, ParamKind::Menu { .. } | ParamKind::MenuBy { .. }) => {
                let want = menu_items_of(range)
                    .unwrap_or_else(|| panic!("{ctx}: menu where the table has {range:?}"));
                assert_eq!(items, want, "{ctx}: items");
            }
            (None, ParamKind::Continuous { .. }) => {
                assert!(
                    !is_toggle && menu_items_of(range).is_none(),
                    "{ctx}: continuous where the table has {range:?}"
                );
            }
        }
    }
    assert_eq!(seen, ROWS.len());
    assert_eq!(ROWS.len(), OVERVIEW.iter().map(|o| o.3).sum::<usize>());
    for &(id, idx, want) in KIND_NOTES {
        assert!(
            ROWS.iter().any(|r| (r.0, r.1) == (id, idx)),
            "kind note for a missing row {id}/{idx}"
        );
        // A note is stale once the table row and the registry agree.
        let row = ROWS.iter().find(|r| (r.0, r.1) == (id, idx)).unwrap();
        match want {
            "toggle" => assert!(!is_toggle_range(row.5), "stale kind note {id}/{idx}"),
            _ => assert_ne!(
                menu_items_of(row.5).map(|v| v.join(", ")).as_deref(),
                Some(want),
                "stale kind note {id}/{idx}"
            ),
        }
    }
}

/// Whether `p`'s default is an exact step k/(n − 1) that a table row prints rounded (0.3333 for 1/3, 0.167 for 1/6):
/// A four-digit literal sits below the step and truncates one item low.
pub(super) fn rounded_step(p: &ParamSpec, row: f32) -> bool {
    let n = match p.kind {
        ParamKind::Menu { items } | ParamKind::MenuBy { items, .. } => items.len(),
        ParamKind::Continuous { display } => match display.step_count() {
            Some(n) => n,
            None => return false,
        },
        _ => return false,
    };
    (p.default - row).abs() < 5e-4 && (0..n).any(|k| menu_value(k, n) == p.default)
}

/// The value a row's default stands for: the registry's exact step where the row prints it rounded, else the row's.
pub(super) fn row_value(p: &ParamSpec, row: f32) -> f32 {
    if rounded_step(p, row) { p.default } else { row }
}

/// Params a new module creates at another value than the table's default, which stays their reset value.
const CREATION_NOTES: &[(u16, usize, f32)] = &[
    // Waveshaper Post Gain: created at −12 dB although its reset default is 0 dB.
    (41, 6, 1.0 / 6.0),
];

#[test]
fn a_new_waveshaper_starts_its_post_gain_at_minus_12_db() {
    let p = &spec(41).params[6];
    assert_eq!(p.display_text(p.default), "-12.0 dB");
    assert_eq!(p.display_text(p.reset), "0.0 dB");
    assert!(p.bipolar());
}

/// Default texts where the registry prints something else than the table, with what it prints.
const DEFAULT_TEXT_NOTES: &[(u16, usize, &str)] = &[
    // No formatter in the table: the menu index shows as a number.
    (14, 8, "Sine"),
    // Menus and selectors whose table row shows the generic number.
    (25, 6, "Left channel"),
    (36, 30, "Full"),
    // Tape Stop synced times: the beat label of step 3 (table "1.67 s", the 0 BPM capture).
    (42, 3, "1/2"),
    (42, 4, "1/2"),
    // Autoduck Attack: the table's default is at 120 BPM; without a song the tempo is 0.
    (29, 2, "inf ms"),
    // Spacer inversions: the table's default is the spinner's label, the value texts are lowercase.
    (34, 4, "none"),
    (34, 6, "none"),
    // Equalizer Kind and Channel: the table's default is the panel's item, the param text the generic
    // v×100.
    (3, 4, "51"),
    (3, 8, "51"),
    (3, 12, "51"),
    (3, 16, "51"),
    (3, 17, "0"),
    (3, 18, "0"),
    (3, 19, "0"),
    (3, 20, "0"),
];

#[test]
fn rows_default_texts() {
    for &(id, idx, name, default, default_text, range) in ROWS {
        let ctx = format!("module {id} param {idx} ({name})");
        let p = &spec(id).params[idx];
        // Toggles without a formatter show 0/100 in the table, and the Sync toggles of
        // Tape Stop, Wow & Flutter and Tremolo their label; ours read Off/On. Labelled ones (OFF/ON, No/Yes) carry
        // their texts.
        if p.kind == ParamKind::Toggle && matches!(default_text, "0" | "100" | "Sync") {
            continue;
        }
        // The tables print the value texts: the automation lane's, where they differ from the control's.
        let ours = p.automation_text(row_value(p, default));
        match DEFAULT_TEXT_NOTES.iter().find(|n| (n.0, n.1) == (id, idx)) {
            Some(n) => {
                assert_eq!(ours, n.2, "{ctx}: noted text");
                assert_ne!(ours, default_text, "{ctx}: stale note");
            }
            None => assert_eq!(ours, default_text, "{ctx}: default text (range {range:?})"),
        }
    }
}

/// Rows whose "min … mid … max" samples the registry does not reproduce, with what it prints at 0, 0.5, 1.
const SAMPLE_NOTES: &[(u16, usize, [&str; 3])] = &[
    // Arpeggiator Rotate: the table's range is symbolic, ±(n − 1); without a module the Rhythm is Off.
    (51, 15, ["0", "0", "0"]),
];

#[test]
fn rows_range_samples() {
    let mut checked = 0;
    for &(id, idx, name, _, _, range) in ROWS {
        let ctx = format!("module {id} param {idx} ({name})");
        let p = &spec(id).params[idx];
        if p.kind == ParamKind::Toggle {
            continue;
        }
        let Some(want) = range_parts(range) else {
            continue;
        };
        let ours = [
            p.display_text(0.0),
            p.display_text(0.5),
            p.display_text(1.0),
        ];
        match SAMPLE_NOTES.iter().find(|n| (n.0, n.1) == (id, idx)) {
            Some(n) => {
                assert_eq!(ours, n.2, "{ctx}: noted samples");
                assert_ne!(ours, want, "{ctx}: stale note");
            }
            None => assert_eq!(ours, want, "{ctx}: samples at v = 0, 0.5, 1"),
        }
        checked += 1;
    }
    // The other continuous rows have the generic "0…100 (v×100)" range (covered by the default texts), a
    // "n steps" note or a list of options.
    assert_eq!(
        checked, CHECKED_SAMPLE_ROWS,
        "rows with min … mid … max samples"
    );
}

/// Rows with a "min … mid … max" range that is not a toggle: counted by hand from the tables.
const CHECKED_SAMPLE_ROWS: usize = 155;

// ---- formats the later modules use ----

fn show(display: Display, v: f32) -> String {
    display.text(v)
}

#[test]
fn signs_steps_and_special_texts() {
    // Note Echo pan offset and pitch offset: "+" on zero as well.
    let pan = Display::Linear {
        min: -1.0,
        max: 1.0,
        fmt: Fmt::new(2, "").sign(Sign::Always),
    };
    assert_eq!(
        [show(pan, 0.0), show(pan, 0.5), show(pan, 1.0)],
        ["-1.00", "+0.00", "+1.00"]
    );
    let pitch = Display::Linear {
        min: -24.0,
        max: 24.0,
        fmt: Fmt::new(0, "").sign(Sign::Always),
    };
    assert_eq!(
        [show(pitch, 0.0), show(pitch, 0.5), show(pitch, 1.0)],
        ["-24", "+0", "+24"]
    );
    // Reverb 2 separation: "+" only on positive numbers.
    let sep = Display::Linear {
        min: -100.0,
        max: 100.0,
        fmt: Fmt::new(0, "%").sign(Sign::Positive),
    };
    assert_eq!(
        [show(sep, 0.0), show(sep, 0.5), show(sep, 1.0)],
        ["-100%", "0%", "+100%"]
    );
    // Transpose octave and Pitcher pitch.
    let octave = Display::Steps {
        min: -4,
        count: 9,
        fmt: Fmt::new(0, "").sign(Sign::Always),
    };
    assert_eq!(
        [show(octave, 0.0), show(octave, 0.5), show(octave, 1.0)],
        ["-4", "+0", "+4"]
    );
    let semis = Display::Steps {
        min: -12,
        count: 25,
        fmt: Fmt::new(0, ""),
    };
    assert_eq!(
        [show(semis, 0.0), show(semis, 0.5), show(semis, 1.0)],
        ["-12", "0", "12"]
    );
    assert_eq!(show(semis, 0.52), "0");
    assert_eq!(show(semis, 0.55), "1");
    // Pitcher formant shift: 2^(2v-1).
    let formant = Display::exp(0.5, 2.0, 1, "");
    assert_eq!(
        [show(formant, 0.0), show(formant, 0.5), show(formant, 1.0)],
        ["0.5", "1.0", "2.0"]
    );
    // Reverb 2 high cut: the top of the range reads OFF.
    static LINEAR: Display = Display::Linear {
        min: 0.51,
        max: 22.09,
        fmt: Fmt::new(1, "kHz"),
    };
    let cut = Display::Special {
        inner: &LINEAR,
        at: At::Max,
        text: "OFF",
    };
    assert_eq!(
        [
            show(cut, 0.0),
            show(cut, 0.5),
            show(cut, 0.99),
            show(cut, 1.0)
        ],
        ["0.5kHz", "11.3kHz", "21.9kHz", "OFF"]
    );
    let centre = Display::Special {
        inner: &LINEAR,
        at: At::Mid,
        text: "off",
    };
    assert_eq!([show(centre, 0.5), show(centre, 0.51)], ["off", "11.5kHz"]);
    let min = Display::Special {
        inner: &LINEAR,
        at: At::Min,
        text: "OFF",
    };
    assert_eq!([show(min, 0.0), show(min, 0.001)], ["OFF", "0.5kHz"]);
}

#[test]
fn text_is_total_and_never_prints_negative_zero() {
    let bipolar = Display::linear(-1.0, 1.0, 1, " x");
    assert_eq!(show(bipolar, 0.49), "0.0 x");
    assert_eq!(show(bipolar, -3.0), show(bipolar, 0.0));
    assert_eq!(show(bipolar, 9.0), show(bipolar, 1.0));
    assert_eq!(show(bipolar, f32::NAN), show(bipolar, 0.0));
    assert_eq!(show(Display::Generic, 0.125), "12");
    assert_eq!(show(Display::Generic, f32::NAN), "0");
    let freq = Display::Freq {
        min: 32.0,
        max: 32.0 * 608.8,
        hz_decimals: 1,
        khz_decimals: 2,
    };
    assert_eq!(show(freq, 0.0), "32.0 Hz");
    assert_eq!(show(freq, 1.0), "19.48 kHz");
}

/// Percents with a unit round `v·100`, a tie away from zero (unlike `printf`'s ties-to-even), where the generic law
/// truncates: the 3x Osc captures 0.305 → 31% and 0.375 → 38% fit only rounding.
#[test]
fn effect_percents_round_ties_away_from_zero() {
    assert_eq!(text(11, 2, 0.125), "13 %"); // Chorus Depth
    assert_eq!(text(18, 2, 0.125), "13%"); // Tape Delay Fdbk
}

/// Tape Stop times are `round(v·5000)` ms, the same `round` as the generic law: an exact
/// half rounds away from zero.
#[test]
fn tape_stop_milliseconds_round_ties_away_from_zero() {
    assert_eq!(text(42, 1, 0.0625), "313 ms"); // 312.5
    assert_eq!(text(42, 1, 0.1875), "938 ms"); // 937.5
}

#[test]
fn constant_texts_ignore_the_value() {
    // Autoduck 2 Attack and Tape Stop Warp.
    let inf = Display::Const("inf ms");
    for v in [0.0, 0.1, 0.5, 1.0, f32::NAN] {
        assert_eq!(show(inf, v), "inf ms");
    }
}

#[test]
fn bipolar_ranges_may_have_different_ends() {
    // Note Echo Vel. Offset / Vel. Drift (-90% … 0% … 100%) and Length Drift (-50% … 0% … 100%).
    let vel = Display::Bipolar {
        neg: 90.0,
        pos: 100.0,
        fmt: Fmt::new(0, "%"),
    };
    assert_eq!(
        [0.0, 0.25, 0.5, 0.75, 1.0].map(|v| show(vel, v)),
        ["-90%", "-45%", "0%", "50%", "100%"]
    );
    let length = Display::Bipolar {
        neg: 50.0,
        pos: 100.0,
        fmt: Fmt::new(0, "%"),
    };
    assert_eq!(
        [0.0, 0.25, 0.5, 0.75, 1.0].map(|v| show(length, v)),
        ["-50%", "-25%", "0%", "50%", "100%"]
    );
    assert_eq!(show(vel, 0.499_999), "0%");
}

#[test]
fn mirrored_laws_read_the_distance_from_the_centre() {
    // Spacer L/R Delay (300·|2v−1|³ ms) and Pan: the side is the control's business.
    let delay = Display::Mirror {
        max: 300.0,
        exp: 3,
        fmt: Fmt::new(0, " ms"),
    };
    assert_eq!(
        [0.0, 0.25, 0.5, 0.75, 1.0].map(|v| show(delay, v)),
        ["300 ms", "38 ms", "0 ms", "38 ms", "300 ms"]
    );
    let pan = Display::Mirror {
        max: 100.0,
        exp: 1,
        fmt: Fmt::new(0, " %"),
    };
    assert_eq!(
        [0.0, 0.25, 0.5, 0.75, 1.0].map(|v| show(pan, v)),
        ["100 %", "50 %", "0 %", "50 %", "100 %"]
    );
}

#[test]
fn comb_frequencies_strip_trailing_zeros() {
    // Comb Filter Freq: 1 + 2499·v⁴ Hz, "1 Hz", "30.6 Hz", "157 Hz", "2.5 KHz".
    let comb = Display::HzSig {
        min: 1.0,
        max: 2500.0,
        exp: 4,
    };
    assert_eq!(
        [0.0, 0.33, 0.5, 0.75, 1.0].map(|v| show(comb, v)),
        ["1 Hz", "30.6 Hz", "157 Hz", "792 Hz", "2.5 KHz"]
    );
    // One decimal below 100, none from there; the unit switches on the raw value, before printing.
    let linear = |max| Display::HzSig {
        min: 0.0,
        max,
        exp: 1,
    };
    assert_eq!(
        [0.994, 0.9996].map(|v| show(linear(100.0), v)),
        ["99.4 Hz", "1 Hz"]
    );
    assert_eq!(
        [0.9994, 0.9996, 1.0].map(|v| show(linear(1000.0), v)),
        ["999 Hz", "1 Hz", "1 KHz"]
    );
    assert_eq!(show(linear(1000.0), 0.0), "0 Hz");
}

#[test]
fn the_khz_switch_follows_the_rounded_value() {
    // 999.96 Hz prints as "1000.0" in Hz, so it is written in kHz.
    let freq = |min| Display::Freq {
        min,
        max: 20_000.0,
        hz_decimals: 1,
        khz_decimals: 2,
    };
    assert_eq!(show(freq(999.96), 0.0), "1.00 kHz");
    assert_eq!(show(freq(999.94), 0.0), "999.9 Hz");
    assert_eq!(show(freq(1000.0), 0.0), "1.00 kHz");
}

#[test]
fn the_middle_special_text_tolerates_float_noise() {
    static LINEAR: Display = Display::Linear {
        min: -100.0,
        max: 100.0,
        fmt: Fmt::new(0, " %").sign(Sign::Positive),
    };
    let off = Display::Special {
        inner: &LINEAR,
        at: At::Mid,
        text: "off",
    };
    assert_eq!(
        [0.5, 0.500_05, 0.499_95].map(|v| show(off, v)),
        ["off", "off", "off"]
    );
    assert_eq!([show(off, 0.5002), show(off, 0.75)], ["0 %", "+50 %"]);
}

#[test]
fn half_steps_round_away_from_zero() {
    // The menu act-ID form: round half away from zero.
    assert_eq!(menu_index(0.5, 10), 5);
    assert_eq!(menu_index(0.5, 8), 4);
    assert_eq!([menu_index(0.25, 3), menu_index(0.75, 3)], [1, 2]);
    // Waveshaper LFO Rate shows its DSP's trunc(9v): 0.5 on 10 items reads "3/4" (item 4), as captured.
    assert_eq!(text(41, 17, 0.5), "3/4");
    let octave = Display::Steps {
        min: -1,
        count: 8,
        fmt: Fmt::new(0, ""),
    };
    assert_eq!([0.0, 0.5, 1.0].map(|v| show(octave, v)), ["-1", "3", "6"]);
}

#[test]
fn every_parameter_reads_something_at_any_value() {
    for m in modules().chain(instruments()) {
        for (i, p) in m.params.iter().enumerate() {
            for v in [0.0, 0.001, 0.25, 0.5, 0.75, 0.999, 1.0, -1.0, 2.0, f32::NAN] {
                let t = p.display_text(v);
                assert!(!t.is_empty(), "module {} param {i} at {v}", m.id);
                // No "-0", "-0.0 dB", "-0%": a minus sign needs a non-zero digit after it.
                let numeric = matches!(p.kind, ParamKind::Continuous { .. });
                // ("---", 3x Osc's unlimited polyphony, is a word, not a number.)
                if let Some(rest) = t
                    .strip_prefix('-')
                    .filter(|r| numeric && r.starts_with(|c: char| c.is_ascii_digit()))
                {
                    let number: String = rest
                        .chars()
                        .take_while(|c| *c != ' ' && *c != '%')
                        .collect();
                    assert!(number.bytes().any(|b| (b'1'..=b'9').contains(&b)), "{t:?}");
                }
            }
        }
    }
}

// ---- the later modules, by hand ----

fn defaults(id: u16) -> Vec<f32> {
    spec(id).params.iter().map(|p| p.default).collect()
}

#[test]
fn spacer_by_hand() {
    let m = spec(34);
    let names: Vec<&str> = m.params.iter().map(|p| p.name).collect();
    assert_eq!(
        names,
        [
            "Enable",
            "L/R Delay",
            "Pan",
            "Separation",
            "L/R Inversion",
            "M/S Delay",
            "M/S Inversion",
            "Stereo Mode"
        ]
    );
    assert_eq!(defaults(34), [1.0, 0.5, 0.5, 0.5, 0.0, 0.5, 0.0, 0.0]);
    // M/S Delay has a law of its own (`spacer_ms_delay_follows_the_sample_rate`).
    assert_eq!(
        [0.0, 0.25, 0.5, 0.75, 1.0].map(|v| text(34, 1, v)),
        ["300 ms", "38 ms", "off", "38 ms", "300 ms"]
    );
    assert_eq!(
        [0.0, 0.25, 0.5, 0.75, 1.0].map(|v| text(34, 2, v)),
        ["100 %", "50 %", "Center", "50 %", "100 %"]
    );
    assert_eq!(
        [0.0, 0.25, 0.5, 0.75, 1.0].map(|v| text(34, 3, v)),
        ["-100 %", "-50 %", "off", "+50 %", "+100 %"]
    );
    assert_eq!(items(34, 4), ["Nothing", "Left", "Right", "Both"]);
    assert_eq!(items(34, 6), ["Nothing", "Mid", "Side", "Both"]);
    assert_eq!(items(34, 7), ["Standard", "Output M/S", "Input M/S"]);
}

#[test]
fn comb_filter_by_hand() {
    assert_eq!(defaults(40)[..4], [1.0, 0.0, 0.33, 0.8]);
    assert_eq!(
        [0.0, 0.33, 0.5, 1.0].map(|v| text(40, 2, v)),
        ["1 Hz", "30.6 Hz", "157 Hz", "2.5 KHz"]
    );
    assert_eq!(
        [0.0, 0.5, 1.0].map(|v| text(40, 10, v)),
        ["-1", "3", "6"],
        "Pitch Oct"
    );
    assert_eq!(
        items(40, 5),
        ["-2 Oct", "-1 Oct", "0 Oct", "+1 Oct", "+2 Oct"]
    );
    assert_eq!(items(40, 11), KEYS);
    assert_eq!(
        [0.0, 0.5, 1.0].map(|v| text(40, 9, v)),
        ["-5.0 %", "0.0 %", "5.0 %"]
    );
    assert_eq!(spec(40).params[7].kind, ParamKind::Toggle, "Snap");
}

#[test]
fn reverb_2_by_hand() {
    assert_eq!(
        [0.0, 0.16, 0.5, 0.99, 1.0].map(|v| text(35, 1, v)),
        ["0.5kHz", "4.0kHz", "11.3kHz", "21.9kHz", "OFF"]
    );
    for v in [0.0, 0.16, 0.5, 0.99, 1.0] {
        assert_eq!(
            text(35, 6, v),
            text(35, 1, v),
            "Damping has the High Cut law"
        );
    }
    assert_eq!(
        [0.0, 0.0187, 0.5, 1.0].map(|v| text(35, 2, v)),
        ["OFF", "75Hz", "1510Hz", "3000Hz"]
    );
    assert_eq!(
        [0.0, 0.8, 1.0].map(|v| text(35, 7, v)),
        ["0%", "100%", "125%"]
    );
    assert_eq!(
        [0.0, 0.068, 1.0].map(|v| text(35, 4, v)),
        ["0.1s", "1.5s", "20.0s"]
    );
    assert_eq!(text(35, 14, 0.28), "1.0");
    assert_eq!(text(35, 17, 0.2405), "500Hz");
    assert_eq!(text(35, 15, 0.75), "+50%");
    assert_eq!(text(35, 10, 0.0), "Off");
}

#[test]
fn autoduck_2_and_gate_by_hand() {
    // Attack at 0 BPM (`autoduck_attack_follows_the_song_tempo`).
    for v in [0.0, 0.1, 1.0] {
        assert_eq!(text(29, 2, v), "inf ms");
    }
    assert_eq!(
        [0.0, 0.25, 0.501, 0.999, 1.0].map(|v| text(29, 3, v)),
        ["100 %", "75 %", "50 %", "0 %", "Linear"]
    );
    assert_eq!(
        [0.0, 0.5, 1.0].map(|v| text(29, 5, v)),
        ["-50 %", "0 %", "50 %"]
    );
    assert_eq!(items(29, 7)[..2], ["Accel", "Accel 2"]);
    // Gate: Asymmetry is continuous, 66·v² dB.
    assert_eq!(
        [0.0, 0.5, 1.0].map(|v| text(27, 2, v)),
        ["0 dB", "16 dB", "66 dB"]
    );
    assert_eq!(
        [0.0, 0.25, 0.5, 1.0].map(|v| text(27, 1, v)),
        ["-60 dB", "-45 dB", "-30 dB", "0 dB"]
    );
}

#[test]
fn multiband_compressor_by_hand() {
    let m = spec(36);
    assert!(
        m.params[1..].iter().all(|p| p.name.is_empty()),
        "all names are empty"
    );
    let mut want = vec![1.0f32; 31];
    for (i, d) in [
        (1, 0.5),
        (2, 0.3),
        (3, 0.7),
        (4, 0.0),
        (5, 0.0),
        (27, 0.5),
        (28, 0.5),
        (29, 0.5),
        (30, 0.0),
    ] {
        want[i] = d;
    }
    for band in [6, 13, 20] {
        for (offset, d) in [
            (0, 0.5),
            (1, 1.0),
            (2, 0.0),
            (3, 0.3),
            (4, 0.3),
            (5, 0.5),
            (6, 0.0),
        ] {
            want[band + offset] = d;
        }
    }
    assert_eq!(defaults(36), want);
    assert_eq!(
        [(2, 0.3), (3, 0.7), (2, 1.0), (2, 0.5)].map(|(p, v)| text(36, p, v)),
        ["130.0 Hz", "1.6 kHz", "10.2 kHz", "452.5 Hz"]
    );
    for band in [6, 13, 20] {
        assert_eq!(text(36, band, 0.5), "0.0 dB", "Input Gain");
        assert_eq!(text(36, band + 1, 1.0), "0.0 dB", "Thres");
        assert_eq!(text(36, band + 1, 0.0), "-48.0 dB", "Thres");
        assert_eq!(text(36, band + 2, 0.0), "1.0", "Ratio");
        assert_eq!(text(36, band + 3, 0.3), "4.5 ms", "Attack");
        assert_eq!(text(36, band + 4, 0.3), "307.0 ms", "Release");
        assert_eq!(text(36, band + 5, 1.0), "24.0 dB", "Gain");
        assert_eq!(items(36, band + 6), ["On", "Bps", "Mute"]);
    }
    assert_eq!(text(36, 5, 0.5), "25%");
    assert_eq!(items(36, 30), ["Full", "Safe"]);
}

#[test]
fn pitcher_and_pitcher_2_by_hand() {
    assert_eq!(
        [0.0, 0.5, 1.0].map(|v| text(25, 2, v)),
        ["-12", "0", "12"],
        "Pitch"
    );
    assert_eq!(
        [0.0, 0.5, 1.0].map(|v| text(25, 7, v)),
        ["415.3", "440.0", "466.2"],
        "Base Freq"
    );
    assert!(
        spec(25)
            .params
            .iter()
            .all(|p| p.name.is_empty() || p.name == "Enable")
    );
    // C natural minor: C D Eb F G Ab Bb.
    assert_eq!(
        defaults(45)[1..13],
        [1.0, 0.0, 1.0, 1.0, 0.0, 1.0, 0.0, 1.0, 1.0, 0.0, 1.0, 0.0]
    );
    assert_eq!(text(45, 18, 1.0), "Left + Right");
    assert_eq!(text(45, 17, 0.25), "-50%");
    // C major.
    assert_eq!(
        defaults(50)[4..16],
        [1.0, 0.0, 1.0, 0.0, 1.0, 1.0, 0.0, 1.0, 0.0, 1.0, 0.0, 1.0]
    );
    assert_eq!(items(50, 2).len(), 34);
    assert_eq!(text(50, 2, 1.0), "Custom");
    assert_eq!(items(50, 1), KEYS);
}

#[test]
fn note_effects_by_hand() {
    assert_eq!(
        [
            (1, 0.25),
            (5, 0.5),
            (5, 1.0),
            (8, 0.0),
            (6, 0.0),
            (7, 0.25),
            (10, 0.25)
        ]
        .map(|(p, v)| text(47, p, v)),
        ["8", "+0", "+24", "-12", "-90%", "-45%", "-25%"]
    );
    assert_eq!(
        [(12, 0.25), (13, 0.75), (9, 0.25), (3, 0.0)].map(|(p, v)| text(47, p, v)),
        ["-0.50", "+25%", "-15%", "1/8"]
    );
    assert_eq!([0.0, 0.5, 1.0].map(|v| text(48, 1, v)), ["-4", "+0", "+4"]);
    assert_eq!(
        [0.0, 0.5, 1.0].map(|v| text(48, 2, v)),
        ["-12", "+0", "+12"]
    );
    assert_eq!(text(49, 1, 0.5), "12 st");
    assert_eq!(text(49, 4, 1.0), "100%");
}

#[test]
fn arpeggiator_by_hand() {
    let m = spec(51);
    assert_eq!(m.params[19].name, "Enable (Arp 1)");
    assert_eq!(m.params[30].name, "Enable (Arp 2)");
    assert_eq!((m.params[19].default, m.params[30].default), (1.0, 0.0));
    assert_eq!(m.params[31].name, "Rate (Arp 1)");
    assert_eq!(m.params[32].name, "Rate (Arp 2)");
    assert_eq!(
        [items(51, 1).len(), items(51, 31).len(), items(51, 10).len()],
        [8, 15, 24]
    );
    assert_eq!(
        [
            items(51, 14).len(),
            items(51, 17).len(),
            items(51, 18).len()
        ],
        [22, 17, 12]
    );
    // Arp 2 repeats Arp 1's controls with indices shifted.
    for (a, b) in [
        (10, 21),
        (11, 22),
        (12, 23),
        (13, 24),
        (14, 25),
        (17, 28),
        (18, 29),
    ] {
        assert_eq!(
            items(51, a),
            items(51, b),
            "Arp 1 param {a} and Arp 2 param {b}"
        );
        assert_eq!(m.params[a].default, m.params[b].default);
    }
    assert_eq!(
        [(9, 0.4444), (9, 0.0), (9, 1.0)].map(|(p, v)| text(51, p, v)),
        ["50%", "10%", "100%"]
    );
    assert_eq!(text(51, 13, 0.5), "Orig");
    assert_eq!(text(51, 12, 0.5), "None");
    assert_eq!(text(51, 31, 0.4286), "x 1");
    assert_eq!(text(51, 2, 0.2105), "250 ms");
}

#[test]
fn milliseconds_switch_to_seconds_at_one_second() {
    // Tape Stop Stop / Start Time: t = round(5000·v) ms.
    let time = Display::Millis {
        full: 5000.0,
        zero: "Off",
    };
    assert_eq!(
        [0.0, 0.00005, 0.0002, 0.01, 0.1, 0.1998, 0.2, 0.5, 1.0].map(|v| show(time, v)),
        [
            "Off", "Off", "1 ms", "50 ms", "500 ms", "999 ms", "1.00 s", "2.50 s", "5.00 s"
        ]
    );
    assert_eq!(show(time, f32::NAN), "Off");
}

#[test]
fn tape_stop_by_hand() {
    let m = spec(42);
    let names: Vec<&str> = m.params.iter().map(|p| p.name).collect();
    assert_eq!(
        names,
        [
            "Enable",
            "Stop Time",
            "Start Time",
            "Stop Time",
            "Start Time",
            "Toggle Play",
            "Warp",
            "Sync"
        ]
    );
    assert_eq!(
        defaults(42),
        [1.0, 0.2, 0.2, 3.0 / 9.0, 3.0 / 9.0, 1.0, 0.5, 0.0]
    );
    // Free times (p1, p2): Off at 0, milliseconds below one second, seconds from there.
    for p in [1, 2] {
        assert_eq!(
            [0.0, 0.01, 0.1, 0.2, 0.5, 1.0].map(|v| text(42, p, v)),
            ["Off", "50 ms", "500 ms", "1.00 s", "2.50 s", "5.00 s"],
            "param {p}"
        );
    }
    // Synced times (p3, p4): ten steps, the beat labels.
    let beats = [
        "Off", "1/4", "3/8", "1/2", "3/4", "1 beat", "2 beats", "4 beats", "8 beats", "16 beats",
    ];
    for p in [3, 4] {
        assert_eq!(items(42, p), beats, "param {p}");
        assert_eq!(text(42, p, 3.0 / 9.0), "1/2", "default of param {p}");
        assert_eq!(text(42, p, 1.0), "16 beats");
    }
    assert_eq!(text(42, 6, 0.1), "Warp");
}

// ---- display laws ----

/// Display arithmetic: the examples where f32 (or, for the exceptions, f64) changes the text.
#[test]
fn display_arithmetic_examples() {
    // Limiter (legacy) Release Time is an f64 exception: 0.7 as f32 times 100 is 69.99999… in f64 (70 in f32).
    assert_eq!(text(5, 2, 0.7), "69%");
    // Linear laws are one f32 fused multiply-add; f64 would print 0.35, 5.13 and 0.1.
    assert_eq!(text(43, 1, 0.05), "0.34 Hz", "Wow Rate");
    assert_eq!(text(43, 4, 0.009), "5.14 Hz", "Flutter Rate");
    assert_eq!(text(43, 8, (0.05f64 / 9.9) as f32), "0.2 Hz", "Random Rate");
    // Reverb 2 Decay: floor(1.5 + 199·v) in f32 is 2 here, 1 in f64.
    assert_eq!(text(35, 4, (0.5f64 / 199.0) as f32), "0.2s");
}

/// The generic law: `v × 100` in f32, truncated.
#[test]
fn the_generic_display_truncates() {
    assert_eq!(text(12, 19, 0.499), "49", "Reverb legacy MOD SPEED");
    assert_eq!(text(32, 12, 0.499), "49", "Reverb MOD SPEED");
    assert_eq!(
        [0.0, 0.125, 0.5, 0.999, 1.0].map(|v| show(Display::Generic, v)),
        ["0", "12", "50", "99", "100"]
    );
}

#[test]
fn pitcher_pitch_and_fine_tune_truncate_in_f64() {
    // The 25 Pitch steps: trunc(24·v − 12) at v = f32(k/24).
    let steps: Vec<String> = (0..25).map(|k| text(25, 2, k as f32 / 24.0)).collect();
    assert_eq!(
        steps,
        [
            "-12", "-10", "-9", "-9", "-7", "-7", "-6", "-5", "-3", "-3", "-2", "0", "0", "1", "1",
            "3", "4", "4", "6", "7", "7", "9", "10", "10", "12"
        ]
    );
    // Fine Tune at v = f32(k/200): the groups that share a text.
    let fine = |k: u32| text(25, 3, k as f32 / 200.0);
    for (ks, want) in [
        (0..=0, "-50"),
        (1..=2, "-49"),
        (3..=4, "-48"),
        (96..=96, "-2"),
        (97..=97, "-1"),
        (98..=102, "0"),
        (103..=104, "1"),
        (200..=200, "50"),
    ] {
        for k in ks {
            assert_eq!(fine(k), want, "k = {k}");
        }
    }
    assert_eq!(
        [0.0, 0.5, 1.0].map(|v| text(25, 5, v)),
        ["0.5", "1.0", "2.0"],
        "F-Shift"
    );
}

#[test]
fn reverb_pan_truncates_in_f64() {
    assert_eq!(
        [0.0, 0.25, 0.5, 0.7, 1.0].map(|v| text(32, 4, v)),
        ["-100", "-50", "0", "39", "100"]
    );
}

#[test]
fn reverb_2_decay_and_cross_are_floored_in_f32() {
    assert_eq!(
        [0.0, 0.068, 0.5, 1.0].map(|v| text(35, 4, v)),
        ["0.1s", "1.5s", "10.1s", "20.0s"]
    );
    assert_eq!(
        [0.0, 0.2405, 0.5, 1.0].map(|v| text(35, 17, v)),
        ["25Hz", "500Hz", "1013Hz", "2000Hz"]
    );
}

#[test]
fn comb_filter_freq_strips_trailing_zeros() {
    // The samples, then the strip that also eats zeros before the point.
    assert_eq!(
        [0.0, 0.2, 0.25, 0.33, 0.4, 0.5, 0.8, 0.85, 0.95, 1.0].map(|v| text(40, 2, v)),
        [
            "1 Hz", "5 Hz", "10.8 Hz", "30.6 Hz", "65 Hz", "157 Hz", "1 KHz", "1.3 KHz", "2 KHz",
            "2.5 KHz"
        ]
    );
    assert_eq!(text(40, 2, 0.55), "23 Hz", "230 loses its zero");
    let hundred = (99.0f64 / 2499.0).powf(0.25) as f32;
    assert_eq!(text(40, 2, hundred), "1 Hz", "100 Hz");
}

#[test]
fn comb_filter_other_texts() {
    // Mode and Polarity always read their first item (a quirk, kept); the panel's menus keep the labels.
    for v in [0.0, 0.5, 1.0] {
        assert_eq!(spec(40).params[1].automation_text(v), "Manual");
        assert_eq!(spec(40).params[8].automation_text(v), "Both +");
    }
    // Offset round(4v − 2), halves away from zero; Pitch Oct round(7v − 1); Pitch Note round(11v).
    assert_eq!(
        [0.0, 0.375, 0.5, 0.625, 1.0].map(|v| text(40, 5, v)),
        ["-2 Oct", "-1 Oct", "0 Oct", "+1 Oct", "+2 Oct"]
    );
    assert_eq!([0.0, 0.5, 1.0].map(|v| text(40, 10, v)), ["-1", "3", "6"]);
    assert_eq!([0.0, 0.4, 1.0].map(|v| text(40, 11, v)), ["C", "E", "B"]);
    assert_eq!(
        [0.0, 0.5, 0.75, 1.0].map(|v| text(40, 9, v)),
        ["-5.0 %", "0.0 %", "2.5 %", "5.0 %"]
    );
}

#[test]
fn wow_and_flutter_synced_menus_truncate() {
    // The item is trunc(9v): the defaults 0.2, 0.1 and 0.8 read 1/4, 1/8 and 4 beats.
    assert_eq!(
        [(2, 0.2), (5, 0.1), (9, 0.8)].map(|(p, v)| text(43, p, v)),
        ["1/4", "1/8", "4 beats"]
    );
    assert_eq!(
        [0.0, 0.11, 0.111_2, 0.5, 0.99, 1.0].map(|v| text(43, 2, v)),
        ["1/8", "1/8", "1/4", "3/4", "8 beats", "16 beats"]
    );
    // A picked item writes k/9, which reads back as item k.
    let kind = spec(43).params[2].kind;
    for k in 0..10 {
        assert_eq!(kind.item(menu_value(k, 10)), k, "item {k}");
    }
}

#[test]
fn spreader_texts_follow_the_value() {
    assert_eq!(
        [0.0, 0.5, 1.0].map(|v| text(44, 1, v)),
        ["0.0", "1.0", "2.0"]
    );
    assert_eq!(
        [0.0, 1.0 / 3.0, 0.5, 0.7, 0.99, 1.0].map(|v| text(44, 2, v)),
        ["10 Hz", "100 Hz", "316 Hz", "1.3 kHz", "9.3 kHz", "10 kHz"]
    );
    assert_eq!(
        [0.0, 0.25, 0.45, 0.475, 0.4875, 0.5, 0.5002, 0.75, 1.0].map(|v| text(44, 4, v)),
        [
            "-100 %", "-50 %", "-10 %", "-5.0 %", "-2.5 %", "Original", "Original", "50 %", "100 %"
        ]
    );
}

#[test]
fn equalizer_kind_and_channel_show_the_nearest_item_and_text_the_generic_number() {
    // Equalizer: the panel shows round-half-away(f32(2v)) and
    // (f32(4v)), the DSP truncates (`honeydew_audio`), and the param text is the generic v×100.
    let eq = spec(3);
    for p in [4, 8, 12, 16] {
        let kind = &eq.params[p];
        assert_eq!(kind.kind.items()[kind.kind.item(0.3)], "Peak", "p{p}");
        assert_eq!(kind.kind.item(0.75), 2, "p{p}");
        assert_eq!(kind.automation_text(0.3), "30", "p{p}");
        assert_eq!(kind.automation_text(kind.default), "51", "p{p}");
    }
    for p in 17..=20 {
        let channel = &eq.params[p];
        assert_eq!(channel.kind.items()[channel.kind.item(0.2)], "Left", "p{p}");
        assert_eq!(channel.automation_text(0.2), "20", "p{p}");
        assert_eq!(channel.automation_text(1.0), "100", "p{p}");
    }
}

#[test]
fn tremolo_texts() {
    let rate = &spec(46).params[1];
    assert_eq!(rate.display_text(rate.default), "1.00 Hz");
    assert_eq!(text(46, 1, 0.0455), "1.01 Hz", "the old tabled default");
    // Rate Sync: the menu shows 1/8, the scrambled automation list 1/4.
    let sync = &spec(46).params[2];
    assert_eq!(
        [sync.display_text(0.0), sync.automation_text(0.0)],
        ["1/8", "1/4"]
    );
    // Shape, Smooth, Width: 100v whole (printf), not the generic truncation.
    for p in [8, 9, 11] {
        assert_eq!(text(46, p, 0.999), "100", "param {p}");
    }
    assert_eq!(text(46, 3, 0.5), "50 %");
    assert_eq!(text(46, 5, 0.25), "90.00");
}

#[test]
fn note_echo_delay_items_and_boundaries() {
    let items = ["1/8", "1/4", "3/8", "1/2", "3/4", "1", "2", "3", "4", "8"];
    let centres = [
        0.1085, 0.1657, 0.2077, 0.2425, 0.3003, 0.3487, 0.4971, 0.6104, 0.7057, 1.0,
    ];
    for (c, want) in centres.into_iter().zip(items) {
        assert_eq!(text(47, 3, c), want, "v = {c}");
    }
    // Each boundary separates two neighbours.
    let bounds = [
        0.1400, 0.1879, 0.2258, 0.2730, 0.3254, 0.4293, 0.5566, 0.6598, 0.8655,
    ];
    for (i, b) in bounds.into_iter().enumerate() {
        assert_eq!(text(47, 3, b - 0.0002), items[i], "below {b}");
        assert_eq!(text(47, 3, b + 0.0002), items[i + 1], "above {b}");
    }
    assert_eq!(text(47, 3, 0.0), "1/8");
    // Sync Off: b with two decimals.
    assert_eq!(
        [0.0, 0.5, 1.0].map(|v| text(47, 2, v)),
        ["0.03", "2.02", "8.00"]
    );
}

#[test]
fn note_echo_snaps_to_the_item_value() {
    // v_i = √((b_i − 1/32)/(8 − 1/32)).
    let centres: [f32; 10] = [
        0.1085, 0.1657, 0.2077, 0.2425, 0.3003, 0.3487, 0.4971, 0.6104, 0.7057, 1.0,
    ];
    for v in [
        0.0, 0.1, 0.15, 0.2, 0.24, 0.3, 0.4, 0.5, 0.6, 0.75, 0.9, 1.0,
    ] {
        let snapped = snap_echo_delay(v);
        assert_eq!(text(47, 3, snapped), text(47, 3, v), "v = {v}");
        assert!(
            centres.iter().any(|c| (c - snapped).abs() < 1e-4),
            "{v} snaps to {snapped}"
        );
        assert_eq!(snap_echo_delay(snapped), snapped, "idempotent at {v}");
    }
    assert_eq!(text(47, 2, snap_echo_delay(0.25)), "0.50");
}

#[test]
fn rack_common_params() {
    let [post_gain, mix, wet_pan] = &COMMON_PARAMS;
    assert_eq!(
        [post_gain.name, mix.name, wet_pan.name],
        ["Post Gain", "Mix", "Wet Pan"]
    );
    assert_eq!(
        [post_gain.default, mix.default, wet_pan.default],
        [0.5, 1.0, 0.5]
    );
    assert_eq!(
        [0.0, 0.5, 0.5625, 1.0].map(|v| post_gain.display_text(v)),
        ["-20 dB", "0 dB", "3 dB", "20 dB"]
    );
    assert_eq!(
        [0.0, 0.125, 1.0].map(|v| mix.display_text(v)),
        ["0 %", "13 %", "100 %"]
    );
    assert_eq!(
        [0.0, 0.25, 0.49, 0.5, 0.51, 0.53, 0.75, 1.0].map(|v| wet_pan.display_text(v)),
        [
            "100 % Left",
            "50 % Left",
            "Centered",
            "Centered",
            "Centered",
            "5 % Right",
            "50 % Right",
            "100 % Right"
        ]
    );
}

/// Every default text this change moved, as the registry now prints it (control text, then automation text).
#[test]
fn changed_default_texts_are_pinned() {
    for (id, p, want) in [
        (5, 2, "69%"),
        (12, 19, "49"),
        (32, 12, "49"),
        (43, 2, "1/4"),
        (43, 5, "1/8"),
        (44, 2, "10 Hz"),
        (44, 4, "Original"),
        (46, 1, "1.00 Hz"),
        (47, 2, "0.50"),
        (47, 3, "1/2"),
    ] {
        let param = &spec(id).params[p];
        assert_eq!(param.display_text(param.default), want, "{id}/{p}");
    }
    assert_eq!(spec(46).params[2].automation_text(0.0), "1/4");
    assert_eq!(
        COMMON_PARAMS.map(|p| p.display_text(p.default)),
        ["0 dB", "100 %", "Centered"]
    );
}

// ---- exact laws, and texts that read a context ----

/// `id`'s param `param` at `v` in context `cx`.
fn text_in(id: u16, param: usize, v: f32, cx: TextCx) -> String {
    spec(id).params[param].display_text_in(v, cx)
}

#[test]
fn distortion_bias_truncates_in_f64_and_stereo_in_f32() {
    // Bias: trunc(200·v − 100) in double precision, `%d` ("0" for 0.495 < v < 0.505).
    assert_eq!(
        [0.25, 0.496, 0.504].map(|v| text(6, 6, v)),
        ["-50", "0", "0"]
    );
    // Truncated, not rounded: 0.508 is 1.6, 0.492 is -1.6.
    assert_eq!([0.508, 0.492].map(|v| text(6, 6, v)), ["1", "-1"]);
    // Stereo: the same law in single precision. f32 0.565 is 12.9999995 in f64 and 13 once the f32 sum rounds.
    assert_eq!([text(6, 6, 0.565), text(6, 4, 0.565)], ["12", "13"]);
    assert_eq!([0.508, 0.492].map(|v| text(6, 4, v)), ["1", "-1"]);
}

#[test]
fn gate_asymmetry_is_66_v_squared() {
    // 66·v² dB, `%.0f dB`; continuous (0.5 is 16.5, a tie printed to even).
    assert_eq!(
        [0.0, 0.5, 0.7, 1.0].map(|v| text(27, 2, v)),
        ["0 dB", "16 dB", "32 dB", "66 dB"]
    );
}

#[test]
fn autoduck_attack_follows_the_song_tempo() {
    // b = 0.5·v + 0.001 in f32, round(b·60000/BPM) ms.
    let at = |bpm: f64, v: f32| {
        let cx = TextCx {
            tempo_bpm: bpm,
            ..TextCx::default()
        };
        text_in(29, 2, v, cx)
    };
    assert_eq!(
        [0.0, 0.1, 0.5, 1.0].map(|v| at(120.0, v)),
        ["1 ms", "25 ms", "125 ms", "250 ms"]
    );
    assert_eq!(at(0.0, 0.5), "inf ms");
    // Without a context: 0 BPM, the table's capture.
    assert_eq!(text(29, 2, 0.1), "inf ms");
}

#[test]
fn automation_texts_read_the_tempo_of_their_context() {
    // An automation lane's text reads the song tempo as the panel's does (Autoduck Attack).
    let at = |bpm: f64| {
        let cx = TextCx {
            tempo_bpm: bpm,
            ..TextCx::default()
        };
        spec(29).params[2].automation_text_in(0.5, cx)
    };
    assert_eq!([at(120.0), at(60.0)], ["125 ms", "251 ms"]);
    // The automation table still wins where it differs from the UI's (MiniSynth OSC Kind).
    let osc = spec(0).params[6].automation_text_in(menu_value(3, 10), TextCx::default());
    assert_eq!(osc, "SQR+SAW");
}

#[test]
fn autoduck_hold_is_a_fraction_of_time() {
    // round(100·v·max(Time, 0.001)) %, Time = p4.
    let at = |time: f32, v: f32| {
        let mut params = defaults(29);
        params[4] = time;
        text_in(
            29,
            6,
            v,
            TextCx {
                params: &params,
                ..TextCx::default()
            },
        )
    };
    assert_eq!(
        [at(0.5, 0.5), at(1.0, 0.37), at(0.0, 1.0)],
        ["25 %", "37 %", "0 %"]
    );
    // No module: Time at its default 1.
    assert_eq!(text(29, 6, 0.5), "50 %");
}

#[test]
fn spacer_lr_delay_decimals_follow_the_time() {
    // t = 300·|2v−1|³ ms: < 0.01 "off", < 1 two decimals, < 10 one, else none; no side.
    assert_eq!(
        [0.0, 0.25, 0.4, 0.45, 0.49, 0.5, 0.6, 1.0].map(|v| text(34, 1, v)),
        [
            "300 ms", "38 ms", "2.4 ms", "0.30 ms", "off", "off", "2.4 ms", "300 ms"
        ]
    );
}

#[test]
fn spacer_ms_delay_follows_the_sample_rate() {
    // d = |2v−1|⁵, t = 100·d ms; under half a sample "off"; µs below 1 ms.
    let at = |fs: f32, v: f32| {
        let cx = TextCx {
            sample_rate: fs,
            ..TextCx::default()
        };
        text_in(34, 5, v, cx)
    };
    assert_eq!(
        [0.0, 0.25, 0.35, 0.4, 0.45, 0.5, 0.55, 1.0].map(|v| at(44_100.0, v)),
        [
            "100 ms", "3.1 ms", "243 us", "32.0 us", "off", "off", "off", "100 ms"
        ]
    );
    // Two decimals of µs once half a sample is short enough: 96 kHz.
    assert_eq!(
        [at(96_000.0, 0.575), at(44_100.0, 0.575)],
        ["7.59 us", "off"]
    );
    // No audio: off everywhere.
    assert_eq!([0.0, 0.4, 1.0].map(|v| at(0.0, v)), ["off", "off", "off"]);
    assert_eq!(text(34, 5, 0.0), "off");
}

#[test]
fn spacer_ms_inversion_text_follows_lr_inversion() {
    // The value texts are none/left/right/both, and M/S Inversion's reads L/R Inversion's index (a quirk, kept).
    // The controls keep their labels.
    let lr = spec(34).params[4];
    let lr_texts = [0, 1, 2, 3].map(|k| lr.automation_text(menu_value(k, 4)));
    assert_eq!(lr_texts, ["none", "left", "right", "both"]);
    let ms = spec(34).params[6];
    let at = |lr: usize, v: f32| {
        let mut params = defaults(34);
        params[4] = menu_value(lr, 4);
        let cx = TextCx {
            params: &params,
            ..TextCx::default()
        };
        ms.automation.unwrap().text_in(v, cx)
    };
    assert_eq!(
        [at(2, 0.0), at(2, 1.0), at(0, 1.0), at(3, 0.34)],
        ["side", "side", "none", "both"]
    );
    assert_eq!(ms.display_text(1.0), "Both");
}

#[test]
fn arpeggiator_rotate_reads_its_rhythm_step_count() {
    // `%d` of llroundf((2v−1)(n−1)), n = the Rhythm's step count; Rhythm Off → 0.
    let at = |rhythm_param: usize, rhythm: usize, rotate_param: usize, v: f32| {
        let mut params = defaults(51);
        params[rhythm_param] = menu_value(rhythm, 22);
        let cx = TextCx {
            params: &params,
            ..TextCx::default()
        };
        text_in(51, rotate_param, v, cx)
    };
    // R7 is x..xx., 6 steps.
    assert_eq!(
        [0.0, 0.5, 0.7, 1.0].map(|v| at(14, 7, 15, v)),
        ["-5", "0", "2", "5"]
    );
    assert_eq!([at(14, 0, 15, 1.0), at(14, 0, 15, 0.0)], ["0", "0"]);
    // R1 is 2 steps: 0.5 rounds away from zero.
    assert_eq!([at(14, 1, 15, 0.75), at(14, 1, 15, 0.25)], ["1", "-1"]);
    // R14 … R21 are 8 steps.
    assert_eq!(at(14, 21, 15, 1.0), "7");
    // Arp 2 reads its own Rhythm, p25.
    assert_eq!([at(25, 7, 26, 1.0), at(14, 7, 26, 1.0)], ["5", "0"]);
    // No module: Rhythm Off.
    assert_eq!(text(51, 15, 1.0), "0");
}

#[test]
fn the_arpeggiator_rhythm_patterns_are_the_spec_table() {
    // The patterns (index -> steps, x = on); a step count is a pattern's length, and the menu
    // is Off then R1 ... R21.
    assert_eq!(
        ARP_RHYTHM_PATTERNS,
        [
            "x.", "x.x", "x..x", "x.xx", "x.x.x", "x..x.x", "x..xx.", "x.x.xx", "x..x.x.",
            "x..x.xx", "x..xx.x", "x.x.x.x", "x.xx.xx", "x...x.xx", "x...xx.x", "x..x..x.",
            "x..x.xx.", "x..x.xxx", "x..xx.x.", "x.x.x.xx", "x.x.xx.x",
        ]
    );
    assert_eq!(items(51, 14).len(), 1 + ARP_RHYTHM_PATTERNS.len());
}

#[test]
fn the_rhythm_maps_to_one_pattern_and_step_count_for_the_text_the_snap_and_the_ring() {
    // Off, then R1 ... R21 in the table's order, on the Rhythm menu's own items.
    let rhythm = &module_spec(51).unwrap().params[14].kind;
    assert_eq!(arp_steps(menu_value(0, 22)), None);
    for k in 0..22 {
        let v = menu_value(k, 22);
        assert_eq!(rhythm.item(v), k);
        assert_eq!(arp_pattern(v), k.checked_sub(1), "item {k}");
        assert_eq!(
            arp_steps(v),
            k.checked_sub(1).map(|p| ARP_RHYTHM_PATTERNS[p].len()),
            "item {k}"
        );
    }
    // The ends, and what is not a value, read as the menu reads them.
    assert_eq!(arp_steps(1.5), Some(8));
    assert_eq!([arp_steps(-1.0), arp_steps(f32::NAN)], [None, None]);
    // The Rotate text spans the same count: the last item's -(n-1) ... n-1.
    for k in 1..22 {
        let mut params = defaults(51);
        params[14] = menu_value(k, 22);
        let cx = TextCx {
            params: &params,
            ..TextCx::default()
        };
        let n = arp_steps(params[14]).unwrap();
        assert_eq!(text_in(51, 15, 1.0, cx), (n - 1).to_string(), "R{k}");
    }
}

#[test]
fn pitcher_plays_the_pitch_and_fine_tune_its_text_shows() {
    // Pitch and Fine Tune are truncated in f64, in the audio too. Bypass on, so the
    // target is 440·2^((s + c/100)/12): its steps must read as the registry's text at every v = f32(k/n).
    use honeydew_audio::pitcher::{DEFAULT_PARAMS, PitcherState};
    let mut params = DEFAULT_PARAMS;
    params[22] = 1.0;
    let mut state = PitcherState::new(&params, 48_000.0).unwrap();
    for (index, steps, per_semitone) in [(2, 24, 1.0), (3, 200, 100.0)] {
        for k in 0..=steps {
            let v = (f64::from(k) / f64::from(steps)) as f32;
            let mut p = params;
            p[index] = v;
            state.refresh(&p, 48_000.0).unwrap();
            let played = (12.0 * per_semitone * (state.target_for(440.0) / 440.0).log2()).round();
            assert_eq!(played.to_string(), text(25, index, v), "p{index} k = {k}");
        }
    }
}

/// Flanger and Phaser Sync and Comb Snap apply at any value but 0, and their automation lanes read so, as their
/// panels do.
#[test]
fn sync_and_snap_lanes_read_on_at_any_value_but_0() {
    for (id, p, off, on) in [
        (13, 2, "Off", "On"),
        (14, 6, "Off", "On"),
        (40, 7, "No", "Yes"),
    ] {
        let spec = &spec(id).params[p];
        let texts = [0.0, 0.25, 0.5, 1.0].map(|v| spec.automation_text(v));
        assert_eq!(texts, [off, on, on, on], "module {id} p{p}");
    }
}
