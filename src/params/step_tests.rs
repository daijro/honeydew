//! Every stepped param's text shows the step its audio plays (semitone steps, stepped selectors, the menu act-ID
//! form). Each law is swept over every step v = k/m, every half step, and one f32 ulp either side of each, with the
//! steps computed as a menu writes them (the f32 quotient) and as the f64 quotient rounded to f32.

use super::*;
use honeydew_audio::three_osc::arp::chord_value;
use honeydew_audio::{dw_sampler, gmsynth, minisynth, sample_edit, slicer, supersaw, three_osc};

fn sweep(m: u32) -> Vec<f32> {
    let halves = 2 * m;
    (0..=halves)
        .flat_map(|h| {
            [
                h as f32 / halves as f32,
                (f64::from(h) / f64::from(halves)) as f32,
            ]
        })
        .flat_map(|v| [v.next_down(), v, v.next_up()])
        .filter(|v| (0.0..=1.0).contains(v))
        .collect()
}

fn param(id: u16, p: usize) -> &'static ParamSpec {
    &module_spec(id).unwrap().params[p]
}

// The audio's index laws, written out.
fn trunc<const N: u32>(v: f32) -> usize {
    (N as f32 * v) as usize
}
fn trunc64<const N: u32>(v: f32) -> usize {
    (f64::from(N) * f64::from(v)) as usize
}
fn half_up<const N: u32>(v: f32) -> usize {
    v.mul_add(N as f32, 0.5) as usize
}
fn count<const N: u32>(v: f32) -> usize {
    v.mul_add(N as f32, 1.0) as usize - 1
}
/// The act-ID form: round half away from zero of f32(n − 1)·v.
fn round<const N: u32>(v: f32) -> usize {
    (N as f32 * v).round() as usize
}

type Law = fn(f32) -> usize;

/// (module id, params, steps m = items − 1, the index the audio picks). Where the text and the audio differ the row
/// is the text's own law: the MiniSynth tempo steps, trunc(12v) over 13 labels (the DSP reads trunc(11v) of
/// another table), and the act-ID form of Flanger LFO Rate, SuperSaw ARP Range and Voices and
/// Equalizer Kind and Channel (the DSP truncates: `the_flanger_default_shows_2_1_and_runs_1_1`,
/// `the_supersaw_defaults_show_5_voices_and_play_4`, `equalizer_kind_and_channel_show_the_nearest_item_and_text_the_generic_number`).
const SELECTORS: &[(u16, &[usize], u32, Law)] = &[
    // MiniSynth.
    (0, &[1, 4, 18, 19], 2, trunc::<2>),
    (0, &[6], 9, trunc::<9>),
    (0, &[20, 39], 12, trunc::<12>),
    (0, &[30], 3, trunc::<3>),
    // DW Sampler.
    (1, &[16, 32], 3, trunc::<3>),
    (1, &[27], 3, half_up::<3>),
    (1, &[29], 2, trunc::<2>),
    (1, &[30], 9, trunc64::<9>),
    // GMSynth.
    (2, &[16, 42], 3, trunc::<3>),
    (2, &[23, 33, 41, 43, 49, 56, 63], 2, trunc::<2>),
    (2, &[32, 46, 48, 53, 55, 60, 62], 9, trunc::<9>),
    (2, &[35], 4, trunc::<4>),
    // SuperSaw.
    (23, &[4, 37], 11, count::<11>),
    (23, &[10], 1, |v| usize::from(v > 0.5)),
    (23, &[39], 3, half_up::<3>),
    (23, &[41], 2, trunc::<2>),
    (23, &[42], 9, trunc64::<9>),
    (23, &[45], 7, half_up::<7>),
    (23, &[47, 49], 5, trunc::<5>),
    (23, &[48], 3, round::<3>),
    (23, &[51], 7, round::<7>),
    (23, &[54], 4, trunc::<4>),
    (23, &[59], 8, trunc::<8>),
    // Slicer.
    (33, &[5], 2, trunc64::<2>),
    // 3x Osc.
    (39, &[1, 2, 3, 142], 6, round::<6>),
    (39, &[37, 38, 39, 40, 41], 2, round::<2>),
    (39, &[110], 7, round::<7>),
    (39, &[139, 143], 5, round::<5>),
    (39, &[145], 90, |v| (chord_value(v) + 2) as usize),
    // Equalizer (the panel's items; the DSP truncates), MultiFX, Distortion, Filter.
    (3, &[4, 8, 12, 16], 2, round::<2>),
    (3, &[17, 18, 19, 20], 4, round::<4>),
    (4, &[1], 7, trunc::<7>),
    (4, &[4], 9, trunc::<9>),
    (4, &[6], 2, trunc::<2>),
    (6, &[5], 3, trunc::<3>),
    (9, &[3, 4], 2, trunc::<2>),
    // Flanger, Phaser, Trance Delay.
    (13, &[1], 12, round::<12>),
    (13, &[4], 2, round::<2>),
    (14, &[1], 12, trunc::<12>),
    (14, &[8], 2, round::<2>),
    (16, &[1], 4, trunc::<4>),
    (16, &[5], 2, round::<2>),
    // Pitcher, Autoduck, Tuned EQ, Spacer, Multiband Compressor.
    (25, &[6], 2, trunc::<2>),
    (29, &[7], 5, trunc::<5>),
    (30, &[9], 11, |v| {
        f64::from(v).mul_add(11.0, f64::from(0.001_f32)) as usize
    }),
    (34, &[4, 6], 3, round::<3>),
    (34, &[7], 2, round::<2>),
    (36, &[12, 19, 26], 2, trunc::<2>),
    (36, &[30], 1, round::<1>),
    // Comb Filter, Waveshaper, Tape Stop, Wow & Flutter, Pitcher 2, Tremolo.
    (40, &[1], 2, trunc::<2>),
    (40, &[5], 4, |v| {
        (4f32.mul_add(v, -2.0).round() + 2.0) as usize
    }),
    (40, &[8], 3, trunc::<3>),
    (40, &[11], 11, round::<11>),
    (41, &[1], 6, round::<6>),
    (41, &[15], 2, round::<2>),
    (41, &[17], 9, trunc::<9>),
    (42, &[3, 4], 9, round::<9>),
    (43, &[2, 5, 9], 9, trunc::<9>),
    (45, &[18], 2, trunc::<2>),
    (46, &[2], 9, trunc64::<9>),
    // Scale, Arpeggiator: round(clamp(v)·(n − 1)) in f32.
    (50, &[1], 11, round::<11>),
    (50, &[2], 33, round::<33>),
    (51, &[1], 7, round::<7>),
    (51, &[5], 1, round::<1>),
    (51, &[10, 21], 23, round::<23>),
    (51, &[11, 22], 3, round::<3>),
    (51, &[12, 23], 6, round::<6>),
    (51, &[13, 17, 24, 28], 16, round::<16>),
    (51, &[14, 25], 21, round::<21>),
    (51, &[18, 29], 11, round::<11>),
    (51, &[31, 32], 14, round::<14>),
];

#[test]
fn every_selector_text_shows_the_item_its_audio_picks() {
    for &(id, params, m, law) in SELECTORS {
        for &p in params {
            let spec = param(id, p);
            let items = spec.kind.items();
            assert_eq!(items.len(), m as usize + 1, "module {id} p{p}: items");
            for v in sweep(m) {
                let k = law(v);
                let ctx = format!("module {id} p{p} at v = {v:e}");
                assert_eq!(spec.kind.item(v), k, "{ctx}: item");
                assert_eq!(spec.display_text(v), items[k], "{ctx}: text");
                if let Some(ParamKind::Menu { items }) = spec.automation {
                    assert_eq!(spec.automation_text(v), items[k], "{ctx}: automation");
                }
            }
        }
    }
    // Every menu of the registry is in the table.
    for m in modules().chain(instruments()) {
        for (p, spec) in m.params.iter().enumerate() {
            let listed = SELECTORS.iter().any(|s| s.0 == m.id && s.1.contains(&p));
            assert_eq!(
                !spec.kind.items().is_empty(),
                listed,
                "module {} p{p}",
                m.id
            );
        }
    }
}

/// The number a step text shows for the audio's step `s`: `%d`, or with a sign on positives or on zero too.
fn plain(s: i32) -> String {
    s.to_string()
}
fn plus(s: i32) -> String {
    if s > 0 {
        format!("+{s}")
    } else {
        s.to_string()
    }
}
fn signed(s: i32) -> String {
    format!("{s:+}")
}

type Step = fn(f32) -> String;

/// (module id, params, steps m, the audio's step as its text). Pitcher's step is private to its DSP
/// (`pitcher_plays_the_pitch_and_fine_tune_its_text_shows` checks the sound); 3x Osc Echoes reads 1 + round(9v)
/// (`three_osc::echo`).
const STEPS: &[(u16, &[usize], u32, Step)] = &[
    (0, &[2], 24, |v| plain(minisynth::transpose_steps(v))),
    (1, &[3], 48, |v| plain(dw_sampler::pitch_shift(v))),
    (2, &[11], 48, |v| plain(gmsynth::pitch_b_step(v) as i32)),
    (2, &[28], 48, |v| plain(gmsynth::transpose_step(v) as i32)),
    (
        23,
        &[36],
        24,
        |v| plain(supersaw::transpose_steps(v) as i32),
    ),
    (33, &[1], 48, |v| plain(slicer::pitch_steps(v) as i32)),
    (
        39,
        &[13, 14, 15],
        48,
        |v| plain(three_osc::coarse(v) as i32),
    ),
    (39, &[117], 99, |v| match three_osc::max_voices(v) {
        three_osc::VOICE_CAP => "---".to_owned(),
        n => n.to_string(),
    }),
    (39, &[130], 9, |v| plain(1 + (9.0 * v).round() as i32)),
    (25, &[2], 24, |v| {
        plain((f64::from(v) * 24.0 - 12.0).trunc() as i32)
    }),
    // The note effects' clamped range map: round(f32(f32((hi − lo)·v) + lo)), halves away from zero.
    (47, &[1], 32, |v| plain((32.0 * v).round() as i32)),
    (47, &[5], 48, |v| signed((48.0 * v - 24.0).round() as i32)),
    (47, &[8], 24, |v| signed((24.0 * v - 12.0).round() as i32)),
    (48, &[1], 8, |v| signed((8.0 * v - 4.0).round() as i32)),
    (48, &[2], 24, |v| signed((24.0 * v - 12.0).round() as i32)),
    (49, &[1], 24, |v| {
        format!("{} st", (24.0 * v).round() as i32)
    }),
];

#[test]
fn every_step_text_shows_the_step_its_audio_plays() {
    for &(id, params, m, step) in STEPS {
        for &p in params {
            for v in sweep(m) {
                assert_eq!(
                    param(id, p).display_text(v),
                    step(v),
                    "module {id} p{p} at v = {v:e}"
                );
            }
        }
    }
    // The drum pad Speed: the f32 fused step, "+" on positives.
    for v in sweep(48) {
        let s = sample_edit::speed_semitones(v);
        assert_eq!(
            DRUM_SLOT_PARAMS[2].display_text(v),
            plus(s),
            "Speed at {v:e}"
        );
    }
}

#[test]
fn the_fused_and_f64_pitch_steps_read_their_deviating_steps() {
    // The semitone step at v = f32(k/48): the f32 fused step (pad Speed, DW Pitch, GMSynth TRNSP) reads k − 24
    // but k = 16, 19, 22 → −7, −4, −1 and k = 25 … 40 (every 3rd) → 0 … 15; the f64 one (Slicer, GMSynth PITCH B) k = 1,
    // 2, 4, 8, 11, 13, 16, 19, 22 → k − 23 and k = 25 … 46 (every 3rd) → k − 25.
    for k in 0..=48 {
        let v = (f64::from(k) / 48.0) as f32;
        let fused = match k {
            16 | 19 | 22 => k - 23,
            25..=40 if (k - 25) % 3 == 0 => k - 25,
            _ => k - 24,
        };
        let wide = match k {
            1 | 2 | 4 | 8 | 11 | 13 | 16 | 19 | 22 => k - 23,
            25..=46 if (k - 25) % 3 == 0 => k - 25,
            _ => k - 24,
        };
        assert_eq!(
            DRUM_SLOT_PARAMS[2].display_text(v),
            plus(fused),
            "Speed k {k}"
        );
        for (id, p, want) in [(1, 3, fused), (2, 28, fused), (2, 11, wide), (33, 1, wide)] {
            assert_eq!(
                param(id, p).display_text(v),
                want.to_string(),
                "{id}/{p} k {k}"
            );
        }
        // 3x Osc Coarse rounds: k − 24 everywhere.
        assert_eq!(
            param(39, 13).display_text(v),
            (k - 24).to_string(),
            "Coarse k {k}"
        );
    }
}

#[test]
fn the_flanger_default_shows_2_1_and_runs_1_1() {
    // The default rate quirk: the stored 0.4 shows round(12·0.4) = 5 (`2/1`) while the DSP plays
    // trunc(12·0.4) = 4 (`1/1`). Kept deliberately.
    let rate = param(13, 1);
    assert_eq!(rate.default, 0.4);
    assert_eq!(rate.display_text(rate.default), "2/1");
    assert_eq!(rate.kind.items()[trunc::<12>(rate.default)], "1/1");
    // Picking 2/1 writes 5/12, which both read as 5.
    let picked = menu_value(5, 13);
    assert_eq!((rate.kind.item(picked), trunc::<12>(picked)), (5, 5));
}

#[test]
fn the_supersaw_defaults_show_5_voices_and_play_4() {
    // The default 0.5 gives 4 voices, although its menu text rounds to 5 Voices, and 2 octaves under a `3 Oct`
    // text. Kept deliberately: the DSP counts int(7v) + 1 and int(3v) + 1.
    let voices = param(23, 51);
    assert_eq!(voices.display_text(voices.default), "5 Voices");
    assert_eq!(voices.kind.items()[count::<7>(voices.default)], "4 Voices");
    let range = param(23, 48);
    assert_eq!(range.display_text(range.default), "3 Oct");
    assert_eq!(range.kind.items()[trunc::<3>(range.default)], "2 Oct");
    // A picked item writes k/(n − 1), which both read as k.
    for k in 0..8 {
        assert_eq!(
            (
                voices.kind.item(menu_value(k, 8)),
                count::<7>(menu_value(k, 8))
            ),
            (k, k)
        );
    }
    for k in 0..4 {
        assert_eq!(
            (
                range.kind.item(menu_value(k, 4)),
                trunc::<3>(menu_value(k, 4))
            ),
            (k, k)
        );
    }
}

#[test]
fn minisynth_tempo_texts_truncate_12v_over_13_labels() {
    // Tempo steps: the LFO Rate and PHASER ModSpeed texts are trunc(12v) over 1/4 … 8 bars, 16 bars.
    for p in [20, 39] {
        let rate = param(0, p);
        assert_eq!(rate.kind.items().len(), 13);
        assert_eq!(
            [0.0, 0.0833, 0.25, 0.95, 0.99, 1.0].map(|v| rate.display_text(v)),
            ["1/4", "1/4", "3/4", "8 bars", "8 bars", "16 bars"],
            "p{p}"
        );
    }
}

/// Menu and step defaults that are not a step k/(n − 1), with the item each shows.
const OFF_STEP_DEFAULTS: &[(u16, usize, &str)] = &[
    (0, 30, "3/4"),
    (0, 39, "3/8"),
    (3, 4, "Peak"),
    (3, 8, "Peak"),
    (3, 12, "Peak"),
    (3, 16, "Peak"),
    (4, 4, "beat"),
    (13, 1, "2/1"),
    (14, 1, "3/8"),
    (16, 1, "3/4"),
    (23, 37, "2 St"),
    (23, 48, "3 Oct"),
    (23, 51, "5 Voices"),
    (23, 54, "3/4"),
    (40, 11, "E"),
    (41, 17, "3/4"),
    (43, 2, "1/4"),
    (43, 5, "1/8"),
    (43, 9, "4 beats"),
];

#[test]
fn menu_and_step_defaults_sit_on_their_steps() {
    // A menu or stepped knob writes k/(n − 1) exactly; a four-digit literal such as 0.3333 sits
    // below 1/3 and truncates one item low.
    for m in modules().chain(instruments()) {
        for (p, spec) in m.params.iter().enumerate() {
            let n = match spec.kind {
                ParamKind::Menu { items } | ParamKind::MenuBy { items, .. } => items.len(),
                ParamKind::Continuous { display } => match display.step_count() {
                    Some(n) => n,
                    None => continue,
                },
                _ => continue,
            };
            let ctx = format!("module {} p{p} ({})", m.id, spec.name);
            match OFF_STEP_DEFAULTS.iter().find(|d| (d.0, d.1) == (m.id, p)) {
                Some(d) => {
                    assert_eq!(spec.display_text(spec.default), d.2, "{ctx}: text");
                    assert!(
                        (0..n).all(|k| menu_value(k, n) != spec.default),
                        "{ctx}: on a step, drop the note"
                    );
                }
                None => {
                    let k = (0..n).find(|&k| menu_value(k, n) == spec.default);
                    let k = k.unwrap_or_else(|| panic!("{ctx}: {} is no step", spec.default));
                    if !spec.kind.items().is_empty() {
                        assert_eq!(spec.kind.item(spec.default), k, "{ctx}: item");
                    }
                }
            }
        }
    }
}
