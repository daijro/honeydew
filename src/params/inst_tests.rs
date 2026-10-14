//! The instrument, drum slot and channel registries against the reference parameter table (`ROWS`) and the slot
//! and channel param layouts.

use super::inst_rows::ROWS;
use super::tests::{is_toggle_range, menu_items_of, range_parts, rounded_step, row_value};
use super::*;
use crate::song::ChannelParams;

/// Per instrument: id, display name, unique name, `PRMS` value count and pages.
const OVERVIEW: [(u16, &str, &str, usize, &[&str]); 8] = [
    (
        0,
        "MiniSynth",
        "MiniSynth",
        44,
        &["OSC", "FILTER", "FLT EG", "LFO", "MASTER", "FX"],
    ),
    (
        1,
        "DW Sampler",
        "DW Sampler",
        34,
        &["SAMPLE", "FILTER", "FLT ENV", "FRQ ENV", "LFO"],
    ),
    (
        2,
        "GMSynth",
        "GMS",
        75,
        &["FILTER", "OSC", "MOD", "LEVEL", "EG", "LFO", "VOICES", "FX"],
    ),
    (
        19,
        "Transistor Bass",
        "Transistor Bass",
        14,
        &["MAIN", "DISTORTION", "PULSE WIDTH"],
    ),
    (
        23,
        "SuperSaw",
        "SuperSaw",
        60,
        &["OSC", "FILTER", "LFO", "LEVEL", "MOD", "VOICES", "FX"],
    ),
    (33, "Slicer", "Slicer", 7, &[]),
    (
        39,
        "3x Osc",
        "3x Osc",
        149,
        &["OSC", "PAN", "VOL", "CUT", "RES", "PITCH", "FILT", "MISC"],
    ),
    (52, "SoundFont Player", "SoundFont Player", 6, &[]),
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
fn overview_matches_the_registry() {
    for (id, display_name, unique_name, count, tabs) in OVERVIEW {
        let m = spec(id);
        assert_eq!(m.id, id);
        assert_eq!(m.display_name, display_name, "display name of {id}");
        assert_eq!(m.unique_name, unique_name, "unique name of {id}");
        assert_eq!(m.params.len(), count, "PRMS n of {id}");
        assert_eq!((m.role, m.legacy), (Role::Instrument, false), "{id}");
        assert_eq!(m.tabs, tabs, "pages of {id}");
        let p = &m.params[0];
        assert_eq!(
            (p.name, p.default, p.kind),
            ("Enable", 1.0, ParamKind::Toggle)
        );
    }
    let ids: Vec<u16> = instruments().map(|m| m.id).collect();
    assert_eq!(ids, OVERVIEW.map(|o| o.0));
}

#[test]
fn module_spec_covers_every_effect_and_instrument_id() {
    // Ids 0-52 less the six unregistered ones.
    let gaps = [17, 21, 22, 31, 37, 38];
    for id in 0..=52u16 {
        let want = (!gaps.contains(&id)).then_some(id);
        assert_eq!(module_spec(id).map(|m| m.id), want, "lookup of {id}");
    }
    assert!(module_spec(53).is_none());
    // modules() keeps its meaning (the effects); the two lists split the registry.
    assert!(modules().all(|m| m.role != Role::Instrument));
    assert_eq!(modules().count() + instruments().count(), 53 - gaps.len());
}

#[test]
fn defaults_are_normalised_and_menus_have_items() {
    let all = instruments().flat_map(|m| m.params.iter().map(move |p| (m.id, p)));
    let fixed = DRUM_SLOT_PARAMS
        .iter()
        .chain(CHANNEL_PARAMS)
        .map(|p| (u16::MAX, p));
    for (id, p) in all.chain(fixed) {
        assert!((0.0..=1.0).contains(&p.default), "{id} {}", p.name);
        assert!((0.0..=1.0).contains(&p.reset), "{id} {}", p.name);
        if let ParamKind::Menu { items } = p.kind {
            assert!(items.len() >= 2, "{id} {}", p.name);
        }
    }
}

// ---- the reference table, row by row ----

/// Where the registry's kind differs from a table row, with what it holds: "continuous", "<n> items" or the menu
/// items. Menus hold the labels of the UI list, as in the effect registry; the table often prints the automation
/// texts instead.
const KIND_NOTES: &[(u16, usize, &str)] = &[
    // MiniSynth: the OSC Kind UI labels, the LFO shape menu and the destination menus.
    (0, 4, "FLT, FRQ, MOD"),
    (
        0,
        6,
        "SAW, SAW+SAW, PULSE, SAW + SQUARE, SQUARE+SQUARE, SUPERSAW, BELL, DEAF SAW, SPREAD OCT, SPREAD 5th",
    ),
    (0, 18, "SIN, SAW, SQR"),
    (0, 19, "FLT, FRQ, MOD"),
    // GMSynth: hidden p8 lists its 25 steps (-12.00 … 12.00); the registry has the stepped law with the same texts.
    (2, 8, "continuous"),
    (2, 16, "Ring, FM, Sync, Filter"),
    (2, 23, "LP 12, LP 24, HP"),
    (
        2,
        32,
        "1/4, 3/8, 1/2, 3/4, Beat, 2 Beats, Bar, 2 Bars, 4 Bars, 8 Bars",
    ),
    (2, 33, "Sine, Saw, Square"),
    (2, 42, "Solo, Duet, Trio, Quartet"),
    (2, 43, "Flat, Chord, Octave"),
    (
        2,
        46,
        "DIST, FLANGER, PHASER, LPF, HPF, DELAY, REVERB, PANNER, RING, VOX",
    ),
    (
        2,
        48,
        "1/4, 3/8, 1/2, 3/4, Beat, 2 Beats, Bar, 2 Bars, 4 Bars, 8 Bars",
    ),
    (2, 49, "Sine, Saw, Square"),
    (
        2,
        53,
        "DIST, FLANGER, PHASER, LPF, HPF, DELAY, REVERB, PANNER, RING, VOX",
    ),
    (
        2,
        55,
        "1/4, 3/8, 1/2, 3/4, Beat, 2 Beats, Bar, 2 Bars, 4 Bars, 8 Bars",
    ),
    (2, 56, "Sine, Saw, Square"),
    (
        2,
        60,
        "DIST, FLANGER, PHASER, LPF, HPF, DELAY, REVERB, PANNER, RING, VOX",
    ),
    (
        2,
        62,
        "1/4, 3/8, 1/2, 3/4, Beat, 2 Beats, Bar, 2 Bars, 4 Bars, 8 Bars",
    ),
    (2, 63, "Sine, Saw, Square"),
    // SuperSaw p10 is the SUPERSAW / SUPERSYNC mode menu; the table has the generic display.
    (23, 10, "SUPERSAW, SUPERSYNC"),
    // 3x Osc Arpeggio Chord: 91 steps, the chord list.
    (39, 145, "91 items"),
];

fn kind_note(id: u16, param: usize) -> Option<&'static str> {
    KIND_NOTES
        .iter()
        .find(|n| (n.0, n.1) == (id, param))
        .map(|n| n.2)
}

#[test]
fn rows_names_defaults_and_kinds() {
    for &(id, idx, name, default, _, range) in ROWS {
        let ctx = format!("module {id} param {idx} ({name})");
        let p = &spec(id).params[idx];
        let spec_name = if name == "(none)" { "" } else { name };
        assert_eq!(p.name, spec_name, "{ctx}: name");
        // A menu default is its exact step k/(n − 1), which the tables print rounded (0.1111 for
        // 1/9).
        assert!(
            p.default == default || rounded_step(p, default),
            "{ctx}: default {} != {default}",
            p.default
        );

        match (kind_note(id, idx), &p.kind) {
            (Some("continuous"), kind) => {
                assert!(matches!(kind, ParamKind::Continuous { .. }), "{ctx}")
            }
            (Some(want), kind) if !kind.items().is_empty() && want.ends_with(" items") => {
                assert_eq!(format!("{} items", kind.items().len()), want, "{ctx}")
            }
            (Some(want), kind) if !kind.items().is_empty() => {
                assert_eq!(kind.items().join(", "), want, "{ctx}: items")
            }
            (Some(_), kind) => panic!("{ctx}: expected a menu, got {kind:?}"),
            (None, ParamKind::Toggle) => {
                assert!(
                    is_toggle_range(range),
                    "{ctx}: toggle where the table has {range:?}"
                )
            }
            (None, kind @ (ParamKind::Menu { .. } | ParamKind::MenuBy { .. })) => {
                let want = menu_items_of(range)
                    .unwrap_or_else(|| panic!("{ctx}: menu where the table has {range:?}"));
                assert_eq!(kind.items(), want, "{ctx}: items");
            }
            (None, ParamKind::Continuous { .. }) => assert!(
                !is_toggle_range(range) && menu_items_of(range).is_none(),
                "{ctx}: continuous where the table has {range:?}"
            ),
        }
    }
    // Every instrument param but the SoundFont Player's has a row.
    let tabled: usize = OVERVIEW.iter().filter(|o| o.0 != 52).map(|o| o.3).sum();
    assert_eq!(ROWS.len(), tabled);
    for &(id, idx, want) in KIND_NOTES {
        let row = ROWS.iter().find(|r| (r.0, r.1) == (id, idx));
        let row = row.unwrap_or_else(|| panic!("kind note for a missing row {id}/{idx}"));
        let table = menu_items_of(row.5).map(|v| v.join(", "));
        assert_ne!(table.as_deref(), Some(want), "stale kind note {id}/{idx}");
    }
}

/// Automation texts that are not the table row's own list: the FX 1 and FX 3 Kind rows repeat items ("DIST, DIST,
/// …", "…, PHAS, PHAS, …") and the FX 3 Sync row is written as a range.
const AUTOMATION_NOTES: &[(u16, usize, &str)] = &[
    (
        2,
        46,
        "DIST, FLNG, PHAS, F LP, F HP, DLY, RVRB, PAN, RING, VOX",
    ),
    (
        2,
        60,
        "DIST, FLNG, PHAS, F LP, F HP, DLY, RVRB, PAN, RING, VOX",
    ),
    (2, 62, "1/4, 3/8, 1/2, 3/4, 1/1, 2/1, 4/1, 8/1, 16/1, 32/1"),
];

#[test]
fn automation_texts_are_the_tables_where_the_ui_list_differs() {
    for &(id, idx, name, _, _, range) in ROWS {
        let ctx = format!("module {id} param {idx} ({name})");
        let p = &spec(id).params[idx];
        let noted = AUTOMATION_NOTES.iter().find(|n| (n.0, n.1) == (id, idx));
        match p.automation {
            None => {
                // No override: the control's own kind already matches the row (the UI list is the table's).
                let ui_differs =
                    kind_note(id, idx).is_some_and(|n| n != "continuous" && !n.ends_with(" items"));
                assert!(
                    !ui_differs,
                    "{ctx}: UI list differs from the table but no automation texts"
                );
            }
            Some(ParamKind::Menu { items }) => {
                let want = match noted {
                    Some(n) => n.2.split(", ").collect(),
                    None => menu_items_of(range).unwrap_or_else(|| panic!("{ctx}: {range:?}")),
                };
                assert_eq!(*items, want, "{ctx}: automation texts");
                // A labelled toggle (OFF/ON) keeps its texts here; any other override is a noted UI list.
                if noted.is_none() && p.kind != ParamKind::Toggle {
                    assert!(
                        kind_note(id, idx).is_some(),
                        "{ctx}: override without a kind note"
                    );
                }
            }
            Some(kind) => assert_eq!(
                (kind, range),
                (
                    ParamKind::Continuous {
                        display: Display::Generic
                    },
                    "0…100 (v×100)"
                ),
                "{ctx}"
            ),
        }
    }
    for &(id, idx, want) in AUTOMATION_NOTES {
        let row = ROWS.iter().find(|r| (r.0, r.1) == (id, idx)).unwrap();
        let table = menu_items_of(row.5).map(|v| v.join(", "));
        assert_ne!(
            table.as_deref(),
            Some(want),
            "stale automation note {id}/{idx}"
        );
    }
    // The panels show the UI labels, the automation lane the automation texts.
    let both = |id: u16, p: usize, v: f32| {
        let s = &spec(id).params[p];
        [s.display_text(v), s.automation_text(v)]
    };
    assert_eq!(both(0, 6, menu_value(3, 10)), ["SAW + SQUARE", "SQR+SAW"]);
    assert_eq!(both(0, 18, 0.0), ["SIN", "SINE"]);
    assert_eq!(both(0, 19, 1.0), ["MOD", "OSCMOD"]);
    assert_eq!(both(0, 4, 0.5), ["FRQ", "LFO"]);
    assert_eq!(both(23, 10, 1.0), ["SUPERSYNC", "100"]);
    assert_eq!(both(23, 10, 0.0), ["SUPERSAW", "0"]);
    assert_eq!(both(2, 23, 0.5), ["LP 24", "BP"]);
    assert_eq!(both(2, 32, 1.0), ["8 Bars", "32/1"]);
    assert_eq!(both(2, 46, menu_value(1, 10)), ["FLANGER", "FLNG"]);
    // Without an override both texts are the same.
    assert_eq!(both(0, 20, 0.25), ["3/4", "3/4"]);
    assert_eq!(both(39, 7, 1.0), ["+100%", "+100%"]);
}

/// Default texts where the registry prints something else than the table, with what it prints.
const DEFAULT_TEXT_NOTES: &[(u16, usize, &str)] = &[
    // The mode menu (KIND_NOTES); the table shows the generic number.
    (23, 10, "SUPERSAW"),
    // The default 1/90 is "Auto (sustain)"; the table cuts the label to "Auto".
    (39, 145, "Auto (sustain)"),
    // DW Sampler LFO Attack: the table's default is at 44.1 kHz; without audio the rate is 0.
    (1, 33, "inf s"),
];

#[test]
fn rows_default_texts() {
    for &(id, idx, name, default, default_text, range) in ROWS {
        let ctx = format!("module {id} param {idx} ({name})");
        let p = &spec(id).params[idx];
        // The table reads 0/100 for toggles without a formatter; ours read Off/On. 3x Osc's OFF/ON toggles carry
        // their texts.
        if p.kind == ParamKind::Toggle && matches!(default_text, "0" | "100") {
            continue;
        }
        // A toggle's table text is its value text (the automation lane's); menus are checked by their UI labels,
        // as the tables print those on the default (MiniSynth LFO Shape "SIN" of "SINE, SAW, SQUARE").
        let v = row_value(p, default);
        let ours = if p.kind == ParamKind::Toggle {
            p.automation_text(v)
        } else {
            p.display_text(v)
        };
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
    // GMSynth FX 3 Sync: the table samples the automation texts (1/4 … 32/1); the registry has the UI list.
    (2, 62, ["1/4", "Beat", "8 Bars"]),
];

/// Rows with a "min … mid … max" range that is not a toggle (counted by script from the tables).
const CHECKED_SAMPLE_ROWS: usize = 77;

#[test]
fn rows_range_samples() {
    let mut checked = 0;
    for &(id, idx, name, _, _, range) in ROWS {
        let ctx = format!("module {id} param {idx} ({name})");
        let p = &spec(id).params[idx];
        let Some(want) = range_parts(range) else {
            continue;
        };
        if p.kind == ParamKind::Toggle {
            continue;
        }
        let ours = [0.0, 0.5, 1.0].map(|v| p.display_text(v));
        match SAMPLE_NOTES.iter().find(|n| (n.0, n.1) == (id, idx)) {
            Some(n) => {
                assert_eq!(ours, n.2, "{ctx}: noted samples");
                assert_ne!(ours, want, "{ctx}: stale note");
            }
            None => assert_eq!(ours, want, "{ctx}: samples at v = 0, 0.5, 1"),
        }
        checked += 1;
    }
    assert_eq!(checked, CHECKED_SAMPLE_ROWS);
}

// ---- display laws by hand ----

#[test]
fn dw_sampler_and_supersaw_lfo_rate_follow_the_captured_curve() {
    // The LFO Rate curve in Hz at v = 0, .05 … 1; SuperSaw uses the same curve.
    let hz = [
        0.03, 0.04, 0.05, 0.05, 0.06, 0.08, 0.10, 0.12, 0.15, 0.20, 0.26, 0.36, 0.51, 0.76, 1.19,
        1.99, 3.66, 7.43, 15.79, 26.97, 30.00f64,
    ];
    for (i, hz) in hz.into_iter().enumerate() {
        let v = i as f32 / 20.0;
        for (id, p) in [(1, 28), (23, 40)] {
            assert_eq!(text(id, p, v), format!("{hz:.2} Hz"), "{id}/{p} at {v}");
        }
    }
    // Between the samples the text follows the same law as the engine, 1/(30·(1 − v)³ + 0.033333) Hz, not straight
    // lines between the sample points (which read 21.38).
    assert_eq!(text(1, 28, 0.925), "21.74 Hz");
    assert_eq!(text(23, 40, 0.925), "21.74 Hz");
}

#[test]
fn minisynth_transpose_text_is_the_fused_step() {
    // The semitone step law: the `%d` text uses the audio's s = trunc(fma_f32(v, 24, −12)), so at
    // v = f32(k/24) it reads k − 12 except k = 8, 11, 14, 17, 20 → −3, 0, 1, 4, 7.
    for k in 0..=24 {
        let expected = match k {
            8 => -3,
            11 => 0,
            14 => 1,
            17 => 4,
            20 => 7,
            _ => k - 12,
        };
        assert_eq!(
            text(0, 2, (f64::from(k) / 24.0) as f32),
            expected.to_string(),
            "k = {k}"
        );
    }
}

#[test]
fn instrument_display_samples() {
    // MiniSynth: Transpose in 25 steps, the UI lists.
    assert_eq!([text(0, 2, 0.0), text(0, 2, 1.0)], ["-12", "12"]);
    assert_eq!(text(0, 2, menu_value(13, 25)), "1");
    assert_eq!(text(0, 6, 1.0), "SPREAD 5th");
    assert_eq!(text(0, 20, 1.0), "16 bars");
    assert_eq!(text(0, 30, 1.0 / 3.0), "2/4");
    assert_eq!(text(0, 1, 1.0), "LEAD");
    // DW Sampler.
    assert_eq!(text(1, 3, 0.0), "-24");
    assert_eq!(text(1, 33, 0.7), "inf s");
    assert_eq!(items(1, 30).len(), 10);
    // GMSynth: level, pan, the hidden displays.
    assert_eq!(text(2, 1, 1.0), "18.0 dB");
    assert_eq!(text(2, 2, 0.25), "50 % L");
    assert_eq!(text(2, 2, 1.0), "100 % R");
    assert_eq!(text(2, 7, 0.25), "-6.00");
    assert_eq!([text(2, 8, 0.52), text(2, 8, 0.54)], ["0.00", "1.00"]);
    assert_eq!(text(2, 10, 0.25), "3.125 ms");
    assert_eq!(text(2, 72, 0.25), "91 Hz");
    assert_eq!(text(2, 46, 0.0), "DIST");
    // SuperSaw: Gain reads "0 dB" at the top, one decimal below it.
    assert_eq!([text(23, 53, 0.99), text(23, 53, 1.0)], ["-0.2 dB", "0 dB"]);
    assert_eq!(text(23, 42, menu_value(2, 10)), "3/4");
    assert_eq!(text(23, 10, 1.0), "SUPERSYNC");
    // Slicer.
    assert_eq!([text(33, 1, 0.0), text(33, 5, 1.0)], ["-24", "To End"]);
    // 3x Osc: key names, "---" and "OFF"/"Hold" ends, the echo count and the chord list.
    assert_eq!(text(39, 121, 0.0), "C0");
    assert_eq!(text(39, 121, menu_value(61, 121)), "C#5");
    assert_eq!(text(39, 121, 1.0), "C10");
    assert_eq!(text(39, 117, 0.0), "---");
    assert_eq!(text(39, 117, menu_value(1, 100)), "1");
    assert_eq!(text(39, 117, 1.0), "99");
    assert_eq!([text(39, 129, 0.5), text(39, 129, 1.0)], ["50%", "OFF"]);
    assert_eq!([text(39, 140, 0.5), text(39, 140, 1.0)], ["50%", "Hold"]);
    assert_eq!([text(39, 130, 0.0), text(39, 130, 1.0)], ["1", "10"]);
    assert_eq!([text(39, 142, 0.0), text(39, 143, 1.0)], ["---", "6"]);
    let chords = items(39, 145);
    assert_eq!(chords.len(), 91);
    assert_eq!(
        [chords[0], chords[2], chords[90]],
        ["Auto", "(none)", "Locrian scale"]
    );
    assert_eq!(text(39, 102, 0.105), "-79%");
    // The generic law with the default's "%" rounds halves away from zero (the plain generic law truncates).
    assert_eq!([text(39, 42, 0.125), text(39, 42, 0.375)], ["13%", "38%"]);
    assert_eq!([text(39, 129, 0.125), text(39, 140, 0.125)], ["13%", "13%"]);
    // A real linear law prints like printf: halves to even.
    assert_eq!(text(39, 114, 0.0625), "12%");
    assert_eq!(text(39, 7, 1.0), "+100%");
}

#[test]
fn soundfont_player_has_the_six_ui_bound_params() {
    // Enable, the sliders 1-4, the Envelope toggle 5.
    let m = spec(52);
    let names: Vec<&str> = m.params.iter().map(|p| p.name).collect();
    assert_eq!(
        names,
        [
            "Enable", "Attack", "Decay", "Sustain", "Release", "Envelope"
        ]
    );
    let defaults: Vec<f32> = m.params.iter().map(|p| p.default).collect();
    assert_eq!(defaults, [1.0, 0.0, 0.0, 1.0, 0.0, 0.0]);
    assert_eq!(m.params[5].kind, ParamKind::Toggle);
    // Times: t = max(0.005, 10·(0.95v + 0.05)³) s, "%.2f s" from 1 s, else round(1000t) ms.
    for p in [1, 2, 4] {
        let texts = [0.0, 0.3, 0.5, 1.0].map(|v| text(52, p, v));
        assert_eq!(texts, ["5 ms", "376 ms", "1.45 s", "10.00 s"], "param {p}");
    }
    assert_eq!(
        [0.0, 0.5, 1.0].map(|v| text(52, 3, v)),
        ["0 %", "50 %", "100 %"]
    );
    // Envelope: Original up to 0.5, then Custom.
    let envelope = |v| m.params[5].automation_text(v);
    assert_eq!(
        [0.0, 0.5, 0.51, 1.0].map(envelope),
        ["Original", "Original", "Custom", "Custom"]
    );
}

// ---- reset values and bipolar controls ----

/// The controls whose reset passes their own value instead of the default.
const RESETS: &[(u16, usize, f32)] = &[
    // Waveshaper Post Gain: created at −12 dB, reset to 0 dB.
    (41, 6, 0.5),
    // MiniSynth: MW 1.0; Modifier, Noise and LFO Rate 0.5; AMP DEC/SUS/REL 0; the FX knobs 0 (all of them).
    (0, 5, 1.0),
    (0, 7, 0.5),
    (0, 8, 0.5),
    (0, 20, 0.5),
    (0, 23, 0.0),
    (0, 24, 0.0),
    (0, 25, 0.0),
    (0, 27, 0.0),
    (0, 28, 0.0),
    (0, 30, 0.0),
    (0, 31, 0.0),
    (0, 32, 0.0),
    (0, 33, 0.0),
    (0, 35, 0.0),
    (0, 36, 0.0),
    (0, 37, 0.0),
    (0, 39, 0.0),
    (0, 40, 0.0),
    (0, 41, 0.0),
    (0, 42, 0.0),
    (0, 43, 0.0),
    // 3x Osc: osc 2/3 Coarse, osc 3 Mix, Volume envelope Amount.
    (39, 14, 0.5),
    (39, 15, 0.5),
    (39, 23, 0.5),
    (39, 106, 0.5),
];

#[test]
fn reset_values_are_the_default_but_for_the_listed_controls() {
    for m in modules().chain(instruments()) {
        for (i, p) in m.params.iter().enumerate() {
            let listed = RESETS.iter().find(|r| (r.0, r.1) == (m.id, i));
            assert_eq!(
                p.reset,
                listed.map_or(p.default, |r| r.2),
                "module {} param {i}",
                m.id
            );
        }
    }
    for p in DRUM_SLOT_PARAMS.iter().chain(CHANNEL_PARAMS) {
        assert_eq!(p.reset, p.default, "{}", p.name);
    }
}

#[test]
fn bipolar_is_a_reset_value_of_exactly_one_half() {
    // Bipolar when the origin/reset value is exactly 0.5, whatever the range (unsigned 0…100 knobs such as Equalizer
    // Bandwidth are bipolar).
    let bipolar = |id: u16, p: usize| spec(id).params[p].bipolar();
    for (id, p) in [
        // Signed ranges with default 0.5.
        (0, 2),
        (0, 17),
        (1, 4),
        (2, 2),
        (39, 7),
        (3, 1),
        // Unsigned ranges with default 0.5.
        (3, 3),
        (16, 4),
        (16, 6),
        (1, 21),
        (39, 22),
        (39, 57),
        // Listed reset values of 0.5 on another default.
        (0, 7),
        (0, 8),
        (0, 20),
        (39, 14),
        (39, 15),
        (39, 23),
        (39, 106),
    ] {
        assert!(bipolar(id, p), "{id}/{p}");
    }
    // MiniSynth FX knobs and AMP DEC/SUS/REL reset to 0, MW to 1; other defaults are not 0.5.
    for (id, p) in [(0, 31), (0, 33), (0, 43), (0, 23), (0, 5), (0, 0), (2, 72)] {
        assert!(!bipolar(id, p), "{id}/{p}");
    }
    for m in modules().chain(instruments()) {
        for (i, p) in m.params.iter().enumerate() {
            assert_eq!(p.bipolar(), p.reset == 0.5, "module {} param {i}", m.id);
        }
    }
    let names = |ps: &[ParamSpec]| -> Vec<&str> {
        ps.iter().filter(|p| p.bipolar()).map(|p| p.name).collect()
    };
    assert_eq!(names(DRUM_SLOT_PARAMS), ["Pan", "Speed", "Sample Start"]);
    assert_eq!(names(CHANNEL_PARAMS), ["Pan", "Pitch Bend"]);
}

// ---- drum slot and channel params ----

#[test]
fn drum_slot_params_follow_sections_2_11_and_3_2() {
    let names: Vec<&str> = DRUM_SLOT_PARAMS.iter().map(|p| p.name).collect();
    assert_eq!(
        names,
        [
            "Level",
            "Pan",
            "Speed",
            "Mute",
            "Solo",
            "Length",
            "EG A",
            "EG D",
            "EG S",
            "EG R",
            "Sample Start"
        ]
    );
    let defaults: Vec<f32> = DRUM_SLOT_PARAMS.iter().map(|p| p.default).collect();
    assert_eq!(
        defaults,
        [0.8531, 0.5, 0.5, 0.0, 0.0, 0.0, 0.0, 1.0, 1.0, 1.0, 0.5]
    );
    let slot = |p: usize, v: f32| DRUM_SLOT_PARAMS[p].display_text(v);
    for p in [3, 4] {
        assert_eq!(DRUM_SLOT_PARAMS[p].kind, ParamKind::Toggle);
    }
    // Level at v = 0, .1, .25, .5, .75, .9, 1: dB = 66·v^0.6 − 60 to 0.1, "-inf." at 0.
    let level = [
        (0.0, "-inf."),
        (0.1, "-43.4 dB"),
        (0.25, "-31.3 dB"),
        (0.5, "-16.5 dB"),
        (0.75, "-4.5 dB"),
        (0.9, "+2.0 dB"),
        (1.0, "+6.0 dB"),
    ];
    for (v, want) in level {
        assert_eq!(slot(0, v), want, "Level at {v}");
    }
    // 0 dB at the default (the fader's unity point); the law between the samples.
    assert_eq!(slot(0, 0.8531), "0.0 dB");
    assert_eq!(slot(0, 0.05), "-49.1 dB");
    assert_eq!(slot(0, 0.6), "-11.4 dB");
    assert_eq!(
        [slot(1, 0.0), slot(1, 0.5), slot(1, 0.75), slot(1, 1.0)],
        ["100% left", "center", "50% right", "100% right"]
    );
    assert_eq!(
        [slot(2, 0.0), slot(2, 0.5), slot(2, 1.0)],
        ["-24", "0", "+24"]
    );
    // The f32 fused step: k = 25 plays and reads 0, k = 26 +2.
    assert_eq!(
        [slot(2, menu_value(25, 49)), slot(2, menu_value(26, 49))],
        ["0", "+2"]
    );
    assert_eq!([slot(5, 0.25), slot(9, 1.0)], ["25", "100"]);
    // Sample Start: the signed square (`drum_sample_start_is_a_signed_square`).
    assert_eq!(
        [slot(10, 0.0), slot(10, 0.5), slot(10, 1.0)],
        ["-500", "0.00", "500"]
    );
}

#[test]
fn channel_params_follow_rprm() {
    let names: Vec<&str> = CHANNEL_PARAMS.iter().map(|p| p.name).collect();
    assert_eq!(
        names,
        ["Level", "Pan", "Mute", "Solo", "Pitch Bend", "Aftertouch"]
    );
    assert_eq!(
        (0..6).map(ChannelParams::name).collect::<Vec<_>>(),
        names
            .iter()
            .map(|&n| Some(n.to_owned()))
            .collect::<Vec<_>>()
    );
    assert_eq!(
        ChannelParams::default().0,
        [0.8531, 0.5, 0.0, 0.0, 0.5, 0.0]
    );
    // Level and Pan: the drum slot laws.
    for v in [0.0, 0.3, 0.8531, 1.0] {
        for p in [0, 1] {
            assert_eq!(
                CHANNEL_PARAMS[p].display_text(v),
                DRUM_SLOT_PARAMS[p].display_text(v)
            );
        }
    }
    assert_eq!(CHANNEL_PARAMS[0].display_text(0.8531), "0.0 dB");
    for p in [2, 3] {
        assert_eq!(CHANNEL_PARAMS[p].kind, ParamKind::Toggle);
    }
    assert_eq!(CHANNEL_PARAMS[4].display_text(0.5), "50");
}

// ---- exact laws, and the sample rate ----

#[test]
fn bipolar_amounts_are_exactly_200_v_minus_100_in_each_instruments_format() {
    // Bipolar amounts: 0.508 is 1.6, 0.5625 is 12.5 (a tie), 0.565 is 12.9999995 in f64.
    // MiniSynth and SuperSaw: one decimal.
    for (id, p) in [(0, 17), (0, 21), (23, 33), (23, 44), (23, 57), (23, 58)] {
        assert_eq!(
            [0.0, 0.5, 0.508, 1.0].map(|v| text(id, p, v)),
            ["-100.0", "0.0", "1.6", "100.0"],
            "{id}/{p}"
        );
    }
    // GMSynth LFO and FX amounts: integers truncated toward zero.
    for p in [19, 27, 34, 40, 50, 57, 64] {
        assert_eq!(
            [0.0, 0.508, 0.492, 1.0].map(|v| text(2, p, v)),
            ["-100", "1", "-1", "100"],
            "2/{p}"
        );
    }
    // DW Sampler LFO Amount: trunc in double precision, as Distortion Bias.
    assert_eq!(
        [0.25, 0.496, 0.508, 0.565].map(|v| text(1, 31, v)),
        ["-50", "0", "1", "12"]
    );
    // 3x Osc LFO amounts: rounded half away from zero, then %.
    for p in 52..=56 {
        assert_eq!(
            [0.0, 0.5, 0.5625, 0.4375, 1.0].map(|v| text(39, p, v)),
            ["-100%", "0%", "13%", "-13%", "100%"],
            "39/{p}"
        );
    }
}

#[test]
fn dw_sampler_lfo_attack_follows_the_sample_rate() {
    // #33: t = 1/(c·Fs) s; under 1 s trunc(1000t) ms, else two decimals; Fs = 0 → "inf s".
    let at = |fs: f32, v: f32| {
        let cx = TextCx {
            sample_rate: fs,
            ..TextCx::default()
        };
        spec(1).params[33].display_text_in(v, cx)
    };
    assert_eq!(
        [0.0, 0.25, 0.5, 1.0].map(|v| at(44_100.0, v)),
        ["0 ms", "387 ms", "4.31 s", "56.69 s"]
    );
    assert_eq!(
        [at(0.0, 0.0), at(0.0, 1.0), text(1, 33, 0.5)],
        ["inf s", "inf s", "inf s"]
    );
}

#[test]
fn drum_sample_start_is_a_signed_square() {
    // s = ±0.5·u² s, u = 2v − 1; m = 1000·s ms, no unit, decimals by |m|.
    let start = |v: f32| DRUM_SLOT_PARAMS[10].display_text(v);
    assert_eq!(
        [0.0, 0.25, 0.5, 0.75, 0.45, 0.4999, 0.6, 1.0].map(start),
        [
            "-500", "-125", "0.00", "125", "-5.00", "-0.00", "20.0", "500"
        ]
    );
}

#[test]
fn supersaw_transpose_truncates_the_f32_product() {
    // s = trunc(f32(24·v)) − 12, the product rounded first (not fused): k − 12 at
    // every v = f32(k/24); between the steps it truncates.
    for k in 0..=24 {
        let v = (f64::from(k) / 24.0) as f32;
        assert_eq!(text(23, 36, v), (k - 12).to_string(), "k = {k}");
    }
    assert_eq!([text(23, 36, 0.53), text(23, 36, 0.99)], ["0", "11"]);
}
