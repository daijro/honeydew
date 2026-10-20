//! Parameter tables of the audio and note effects, IDs 24-51.
//!
//! Same conventions as `effects.rs`: `params[i]` is param number `i`, not the order of the UI, and names are
//! copied as written. For modules with ten or more params every line ends in its index. Where a param shows a unit
//! on its default value but has a generic 0…100 (v×100) range, the unit comes from the default. Where a menu's text
//! reads the same at every value (a menu that reads "Manual" three times) the menu holds the UI's label list, and
//! round on/off buttons with the generic display are `Toggle`s; the tests list every such difference
//! (`KIND_NOTES`).

use super::display::{trunc_index, trunc64_index};
use super::effects::{
    BIPOLAR_PERCENT, COMP_ATTACK, DB_12, OFF_ON, PERCENT, PERCENT_TIGHT, on_above_0,
};
use super::{At, Display, Fmt, ModuleSpec, ParamKind, ParamSpec as P, Sign, TextCx, menu_index};

// ---- shared laws and lists ----

const KEYS: &[&str] = &[
    "C", "C#", "D", "D#", "E", "F", "F#", "G", "G#", "A", "A#", "B",
];
/// Synced rates of Wow & Flutter and Tremolo.
const BEAT_RATES: &[&str] = &[
    "1/8", "1/4", "3/8", "1/2", "3/4", "beat", "2 beats", "4 beats", "8 beats", "16 beats",
];
/// ±24 dB (Limiter gains, Multiband Compressor gains).
const DB_24: Display = Display::linear(-24.0, 24.0, 1, " dB");
/// ±18 dB (Waveshaper).
const DB_18: Display = Display::linear(-18.0, 18.0, 1, " dB");
/// -24 … 0 dB (Limiter threshold).
const DB_24_TO_0: Display = Display::linear(-24.0, 0.0, 1, " dB");
/// 0 … 1000 ms, whole milliseconds (Gate).
const MS_1000: Display = Display::linear(0.0, 1000.0, 0, " ms");
/// A linear range with a sign rule ("+0", "+25%").
const fn signed_linear(
    min: f64,
    max: f64,
    decimals: u8,
    unit: &'static str,
    sign: Sign,
) -> Display {
    Display::Linear {
        min,
        max,
        fmt: Fmt::new(decimals, unit).sign(sign),
    }
}
/// -100 … 0 … +100 with a spaced percent sign and a "+" on positive numbers.
const SEPARATION: Display = signed_linear(-100.0, 100.0, 0, " %", Sign::Positive);
/// The same with a tight percent sign (Reverb 2).
const SEPARATION_TIGHT: Display = signed_linear(-100.0, 100.0, 0, "%", Sign::Positive);
/// A note effect's whole step: round(f32(f32(n·v) + lo)), halves away from zero (the clamped range map), with a
/// "+" on zero and positives ("+0", "-24", "+24").
fn signed_step(x: f32) -> String {
    format!("{:+}", x.round() as i32)
}
/// The Note FX switch grid: `Note C` … `Note B`, a toggle each.
const fn note(name: &'static str, on: bool) -> P {
    P::toggle(name, if on { 1.0 } else { 0.0 })
}

// ---- 24 Tuner ----

const TUNER: &[P] = &[P::ENABLE];

// ---- 25 Pitcher ----

const PITCHER: &[P] = &[
    P::ENABLE,         // 0
    P::plain("", 0.0), // 1 Speed
    // v·24 − 12 and v·100 − 50 truncated in f64: the 25 Pitch steps read -12, -10, -9, -9 … 12.
    P::cont(
        "",
        0.5,
        Display::Trunc64 {
            min: -12.0,
            max: 12.0,
            unit: "",
        },
    ), // 2 Pitch
    P::cont(
        "",
        0.5,
        Display::Trunc64 {
            min: -50.0,
            max: 50.0,
            unit: "",
        },
    ), // 3 Fine Tune
    P::toggle("", 0.0), // 4 F-Mode
    // 2^(2v−1): 0.5 … 1.0 … 2.0.
    P::cont("", 0.5, Display::exp(0.5, 2.0, 1, "")), // 5 F-Shift
    P::menu_by(
        "",
        0.0,
        &["Left channel", "Left + Right", "Right channel"],
        trunc_index::<2>,
    ), // 6 input channel
    // 440·2^((2v−1)/12): 415.3 … 440.0 … 466.2 (≈ 415·1.123^v).
    P::cont(
        "",
        0.5,
        Display::exp(415.304_697_579_945, 466.163_761_518_09, 1, ""),
    ), // 7 Base Freq
    P::plain("", 0.0),  // 8 Stability
    P::plain("", 1.0),  // 9 Mix
    P::toggle("", 0.0), // 10 BYP C
    P::toggle("", 0.0), // 11 BYP C#
    P::toggle("", 0.0), // 12 BYP D
    P::toggle("", 0.0), // 13 BYP D#
    P::toggle("", 0.0), // 14 BYP E
    P::toggle("", 0.0), // 15 BYP F
    P::toggle("", 0.0), // 16 BYP F#
    P::toggle("", 0.0), // 17 BYP G
    P::toggle("", 0.0), // 18 BYP G#
    P::toggle("", 0.0), // 19 BYP A
    P::toggle("", 0.0), // 20 BYP A#
    P::toggle("", 0.0), // 21 BYP B
    P::toggle("", 0.0), // 22 Bypass snapping
];

// ---- 26 Analyzer ----

const ANALYZER: &[P] = &[P::ENABLE];

// ---- 27 Gate ----

const GATE_CURVE: Display = Display::linear(0.0, 1.0, 1, "");

const GATE: &[P] = &[
    P::ENABLE,
    P::cont("Threshold", 0.0, Display::linear(-60.0, 0.0, 0, " dB")),
    // 66·v² dB, `%.0f dB`. Continuous; its 66 steps only snap recorded automation to trunc(65v)/65.
    P::cont("Asymmetry", 0.0, Display::power(0.0, 66.0, 2, 0, " dB")).with_record_snap(65),
    P::cont("Attack", 0.01, MS_1000),
    P::cont("A. Curve", 0.0, GATE_CURVE),
    P::cont("Release", 0.25, MS_1000),
    P::cont("R. Curve", 1.0, GATE_CURVE),
    P::cont("Mix", 1.0, BIPOLAR_PERCENT),
    P::cont("Hold", 0.25, MS_1000),
];

// ---- 28 Limiter ----

const LIMITER: &[P] = &[
    P::ENABLE,
    P::cont("In Gain", 0.5, DB_24),
    P::cont("Out Gain", 0.5, DB_24),
    // 0.1 + 1.9·v⁴ s (0.10 s, 0.22 s and 2.00 s at v = 0, 0.5 and 1).
    P::cont("Release Time", 0.2, Display::power(0.1, 2.0, 4, 2, " s")),
    P::cont("Threshold", 1.0, DB_24_TO_0),
];

// ---- 29 Autoduck ----

const AUTODUCK: &[P] = &[
    P::ENABLE,
    P::cont("Ducking", 0.501, PERCENT),
    P::cont("Attack", 0.1, Display::CustomCx(autoduck_attack)),
    // 100 − 100·v %, "Linear" at v = 1.
    P::cont(
        "Curve",
        0.501,
        Display::Special {
            inner: &Display::linear(100.0, 0.0, 0, " %"),
            at: At::Max,
            text: "Linear",
        },
    ),
    P::cont("Time", 1.0, PERCENT),
    P::cont("Shift", 0.5, Display::linear(-50.0, 50.0, 0, " %")),
    P::cont("Hold", 0.0, Display::CustomCx(autoduck_hold)),
    P::menu_by(
        "Shape",
        0.0,
        &[
            "Accel",
            "Accel 2",
            "Decel",
            "Decel 2",
            "S-Curve",
            "S-Curve 2",
        ],
        trunc_index::<5>,
    ),
];

/// Beats b = 0.5·v + 0.001 (f32); text = round(b·60000/BPM) ` ms`, BPM = song tempo: "inf ms" at 0 BPM.
fn autoduck_attack(v: f32, cx: TextCx) -> String {
    let beats = 0.5f32.mul_add(v, 0.001);
    format!(
        "{:.0} ms",
        (f64::from(beats) * 60000.0 / cx.tempo_bpm).round()
    )
}

/// round(100·v·max(Time, 0.001)) ` %` (Hold is a fraction of Time), Time = p4.
fn autoduck_hold(v: f32, cx: TextCx) -> String {
    let time = cx.param(4, AUTODUCK[4].default);
    format!("{} %", (100.0 * v * time.max(0.001)).round())
}

// ---- 30 Tuned EQ ----

const TUNED_EQ: &[P] = &[
    P::ENABLE,                                     // 0
    P::cont("65 Hz", 0.5, DB_12),                  // 1
    P::cont("130 Hz", 0.5, DB_12),                 // 2
    P::cont("261 Hz", 0.5, DB_12),                 // 3
    P::cont("523 Hz", 0.5, DB_12),                 // 4
    P::cont("1.0 kHz", 0.5, DB_12),                // 5
    P::cont("2.1 kHz", 0.5, DB_12),                // 6
    P::cont("4.2 kHz", 0.5, DB_12),                // 7
    P::cont("8.4 kHz", 0.5, DB_12),                // 8
    P::menu_by("Tuning", 0.0, KEYS, tuned_eq_key), // 9 (the 12-tab bar)
    P::plain("Bandwidth", 0.6),                    // 10
];

/// Tuning: key = trunc(fma_f64(v, 11, f32(0.001))), the bias widened from f32.
fn tuned_eq_key(v: f32) -> usize {
    f64::from(v).mul_add(11.0, f64::from(0.001_f32)) as usize
}

// ---- 32 Reverb ----

const REVERB: &[P] = &[
    P::ENABLE,                  // 0
    P::plain("DECAY", 0.4),     // 1
    P::plain("HIGH DAMP", 0.8), // 2
    P::plain("MIX", 0.4),       // 3
    // 200v − 100 truncated in f64.
    P::cont(
        "PAN",
        0.5,
        Display::Trunc64 {
            min: -100.0,
            max: 100.0,
            unit: "",
        },
    ), // 4
    P::plain("EARLY REFL.", 0.25), // 5
    P::plain("PREDELAY", 0.0),     // 6
    P::plain("ROOM SIZE", 1.0),    // 7
    P::plain("DIFFUSION", 1.0),    // 8
    P::plain("WIDTH", 1.0),        // 9
    P::plain("LO-CUT", 0.0),       // 10
    P::plain("HI-CUT", 1.0),       // 11
    P::plain("MOD SPEED", 0.499),  // 12
    P::plain("MOD AMOUNT", 0.0),   // 13
    P::plain("", 1.0),             // 14 Wet Level
    P::plain("", 1.0),             // 15 Dry Level
];

// ---- 34 Spacer ----

/// L/R Delay: t = 300·|2v−1|³ ms: t < 0.01 "off"; < 1 two decimals; < 10 one decimal; else integer; + " ms". No
/// side shown.
fn spacer_delay(v: f32) -> String {
    let t = 300.0 * (2.0 * v - 1.0).abs().powi(3);
    if t < 0.01 {
        return "off".to_owned();
    }
    let decimals = if t < 1.0 {
        2
    } else if t < 10.0 {
        1
    } else {
        0
    };
    format!("{t:.decimals$} ms")
}

/// M/S Delay: d = |2v−1|⁵, t = 100·d ms. t under half a sample (0.5·1000/Fs ms) → "off"; t < 0.01 ms "x.xx us",
/// < 0.1 "x.x us", < 1 "x us" (µs = 100000·d, ASCII "us"); < 10 ms "x.x ms"; else "x ms". Fs = 0 (no audio) → "off"
/// everywhere.
fn spacer_ms_delay(v: f32, cx: TextCx) -> String {
    let d = (2.0 * v - 1.0).abs().powi(5);
    let t = 100.0 * d;
    let us = 100_000.0 * d;
    if t < 500.0 / cx.sample_rate {
        "off".to_owned()
    } else if t < 0.01 {
        format!("{us:.2} us")
    } else if t < 0.1 {
        format!("{us:.1} us")
    } else if t < 1.0 {
        format!("{us:.0} us")
    } else if t < 10.0 {
        format!("{t:.1} ms")
    } else {
        format!("{t:.0} ms")
    }
}

/// M/S Inversion's value text: it follows the L/R Inversion setting (index 0..3 → none/mid/side/both), a quirk,
/// kept.
fn spacer_ms_inversion(_: f32, cx: TextCx) -> String {
    ["none", "mid", "side", "both"][menu_index(cx.param(4, SPACER[4].default), 4)].to_owned()
}

const SPACER: &[P] = &[
    P::ENABLE,
    P::cont("L/R Delay", 0.5, Display::Custom(spacer_delay)),
    P::cont(
        "Pan",
        0.5,
        Display::Special {
            inner: &Display::Mirror {
                max: 100.0,
                exp: 1,
                fmt: Fmt::new(0, " %"),
            },
            at: At::Mid,
            text: "Center",
        },
    ),
    P::cont(
        "Separation",
        0.5,
        Display::Special {
            inner: &SEPARATION,
            at: At::Mid,
            text: "off",
        },
    ),
    // The spinners show the labels; the value texts are "none, left, right, both".
    P::menu("L/R Inversion", 0.0, &["Nothing", "Left", "Right", "Both"]).with_automation(
        ParamKind::Menu {
            items: &["none", "left", "right", "both"],
        },
    ),
    P::cont("M/S Delay", 0.5, Display::CustomCx(spacer_ms_delay)),
    P::menu("M/S Inversion", 0.0, &["Nothing", "Mid", "Side", "Both"]).with_automation(
        ParamKind::Continuous {
            display: Display::CustomCx(spacer_ms_inversion),
        },
    ),
    P::menu("Stereo Mode", 0.0, &["Standard", "Output M/S", "Input M/S"]),
];

// ---- 35 Reverb 2 ----

/// High Cut and Damping: 510 + 21580·v Hz written in kHz ("0.5kHz", "11.3kHz"), OFF at v = 1.
const REVERB2_CUT: Display = Display::Special {
    inner: &Display::linear(0.51, 0.51 + 21.58, 1, "kHz"),
    at: At::Max,
    text: "OFF",
};
/// 125·v %: Dry, ER, Wet.
const REVERB2_LEVEL: Display = Display::linear(0.0, 125.0, 0, "%");

/// Decay in tenths of a second: floor(1.5 + 199v) in f32 ("1.5s" at the default 0.068).
fn reverb2_decay(v: f32) -> String {
    format!("{:.1}s", 199f32.mul_add(v, 1.5).floor() / 10.0)
}

/// Cross: floor(25.5 + 1975v) Hz in f32 ("500Hz" at the default, "1013Hz" in the middle).
fn reverb2_cross(v: f32) -> String {
    format!("{}Hz", 1975f32.mul_add(v, 25.5).floor())
}

const REVERB2: &[P] = &[
    P::ENABLE, // 0
    // 21580 (not 21600) reproduces the default 4.0kHz at 0.16; both read 11.3kHz at the middle.
    P::cont("HIGH CUT", 0.16, REVERB2_CUT), // 1
    // 19 + 2981·v Hz: 75Hz at the default 0.0187 (2980·v alone would read 56Hz).
    P::cont(
        "LOW CUT",
        0.0187,
        Display::Special {
            inner: &Display::linear(19.0, 3000.0, 0, "Hz"),
            at: At::Min,
            text: "OFF",
        },
    ), // 2
    P::cont("SIZE", 0.5, PERCENT_TIGHT),                     // 3
    P::cont("DECAY", 0.068, Display::Custom(reverb2_decay)), // 4
    P::cont("DIFFUSION", 1.0, PERCENT_TIGHT),                // 5
    P::cont("DAMPING", 0.16, REVERB2_CUT),                   // 6
    P::cont("DRY", 0.8, REVERB2_LEVEL),                      // 7
    P::cont("ER", 0.8, REVERB2_LEVEL),                       // 8
    P::cont("WET", 0.56, REVERB2_LEVEL),                     // 9
    P::toggle("SIDE INPUT", 0.0).with_automation(OFF_ON),    // 10
    P::cont("DELAY", 0.0, Display::linear(0.0, 1000.0, 0, "ms")), // 11
    P::toggle("TEMPO", 0.0).with_automation(OFF_ON),         // 12
    P::cont("MOD", 0.0, PERCENT_TIGHT),                      // 13
    P::cont("BASS", 0.28, Display::linear(0.2, 3.0, 1, "")), // 14
    P::cont("SEPARATION", 0.5, SEPARATION_TIGHT),            // 15
    P::cont("SPEED", 0.33, PERCENT_TIGHT),                   // 16
    P::cont("CROSS", 0.2405, Display::Custom(reverb2_cross)), // 17
    P::cont("PAN", 0.5, SEPARATION_TIGHT),                   // 18
];

// ---- 36 Multiband Compressor ----

/// Low Freq and High Freq: 20·512^v Hz. A base of 514.3 would miss the end point (10.2 kHz needs a base between
/// 507 and 513), while 512 gives 130.0 Hz, 452.5 Hz and 1.6 kHz.
const MBC_FREQ: Display = Display::Freq {
    min: 20.0,
    max: 20.0 * 512.0,
    hz_decimals: 1,
    khz_decimals: 1,
};
const MBC_THRES: Display = Display::linear(-48.0, 0.0, 1, " dB");
const MBC_RATIO: Display = Display::linear(1.0, 31.0, 1, "");
const MBC_RELEASE: Display = Display::linear(10.0, 1000.0, 1, " ms");
const BAND_STATE: &[&str] = &["On", "Bps", "Mute"];

// All automation names are empty ("<Band> <UI label>" is for the automation list to build).
const MULTIBAND_COMPRESSOR: &[P] = &[
    P::ENABLE,                                               // 0
    P::cont("", 0.5, DB_24),                                 // 1 Gain
    P::cont("", 0.3, MBC_FREQ),                              // 2 Low Freq
    P::cont("", 0.7, MBC_FREQ),                              // 3 High Freq
    P::toggle("", 0.0),                                      // 4 Solo
    P::cont("", 0.0, Display::power(0.0, 100.0, 2, 0, "%")), // 5 Dry Mix
    P::cont("", 0.5, DB_24),                                 // 6 Low Input Gain
    P::cont("", 1.0, MBC_THRES),                             // 7 Low Thres
    P::cont("", 0.0, MBC_RATIO),                             // 8 Low Ratio
    P::cont("", 0.3, COMP_ATTACK),                           // 9 Low Attack
    P::cont("", 0.3, MBC_RELEASE),                           // 10 Low Release
    P::cont("", 0.5, DB_24),                                 // 11 Low Gain
    P::menu_by("", 0.0, BAND_STATE, trunc_index::<2>),       // 12 Low state
    P::cont("", 0.5, DB_24),                                 // 13 Mid Input Gain
    P::cont("", 1.0, MBC_THRES),                             // 14 Mid Thres
    P::cont("", 0.0, MBC_RATIO),                             // 15 Mid Ratio
    P::cont("", 0.3, COMP_ATTACK),                           // 16 Mid Attack
    P::cont("", 0.3, MBC_RELEASE),                           // 17 Mid Release
    P::cont("", 0.5, DB_24),                                 // 18 Mid Gain
    P::menu_by("", 0.0, BAND_STATE, trunc_index::<2>),       // 19 Mid state
    P::cont("", 0.5, DB_24),                                 // 20 High Input Gain
    P::cont("", 1.0, MBC_THRES),                             // 21 High Thres
    P::cont("", 0.0, MBC_RATIO),                             // 22 High Ratio
    P::cont("", 0.3, COMP_ATTACK),                           // 23 High Attack
    P::cont("", 0.3, MBC_RELEASE),                           // 24 High Release
    P::cont("", 0.5, DB_24),                                 // 25 High Gain
    P::menu_by("", 0.0, BAND_STATE, trunc_index::<2>),       // 26 High state
    P::plain("", 0.5),                                       // 27 Low Width
    P::plain("", 0.5),                                       // 28 Mid Width
    P::plain("", 0.5),                                       // 29 High Width
    P::menu("", 0.0, &["Full", "Safe"]),                     // 30 (the "Full / Safe" selector)
];

// ---- 40 Comb Filter ----

/// Offset: round(4v − 2) octaves, halves away from zero (0.375 reads "-1 Oct").
fn comb_offset(v: f32) -> usize {
    (4f32.mul_add(v, -2.0).round() + 2.0) as usize
}

/// Pitch Oct: round(7v − 1) in f32.
fn comb_octave(v: f32) -> String {
    format!("{}", 7f32.mul_add(v, -1.0).round() as i32)
}

/// Pitch Note: round(11v).
fn comb_note(v: f32) -> usize {
    (11.0 * v).round() as usize
}

/// Asym: −5 + 10v in f64, one decimal.
fn comb_asym(v: f32) -> String {
    Fmt::new(1, " %").apply(-5.0 + 10.0 * f64::from(v))
}

const COMB_FILTER: &[P] = &[
    P::ENABLE, // 0
    // Mode and Polarity texts always read their first item (a quirk, kept); the menus show the labels.
    P::menu_by(
        "Mode",
        0.0,
        &["Manual", "MIDI Note", "Audio Pitch"],
        trunc_index::<2>,
    )
    .with_automation(ParamKind::Menu {
        items: &["Manual"; 3],
    }), // 1
    // 1 + 2499·v⁴ Hz: "1 Hz", "30.6 Hz", "157 Hz", "2.5 KHz".
    P::cont(
        "Pitch",
        0.33,
        Display::HzSig {
            min: 1.0,
            max: 2500.0,
            exp: 4,
        },
    ), // 2
    P::cont("Feedback", 0.8, PERCENT), // 3
    P::cont("Mix", 0.75, PERCENT),     // 4
    P::menu_by(
        "Offset",
        0.5,
        &["-2 Oct", "-1 Oct", "0 Oct", "+1 Oct", "+2 Oct"],
        comb_offset,
    ), // 5
    P::cont("Smoothing", 0.25, PERCENT), // 6
    P::toggle("Snap", 1.0).with_automation(ParamKind::MenuBy {
        items: &["No", "Yes"],
        index: on_above_0,
    }), // 7
    P::menu_by(
        "Polarity",
        0.0,
        &["Both +", "Left -", "Right -", "Both -"],
        trunc_index::<3>,
    )
    .with_automation(ParamKind::Menu {
        items: &["Both +"; 4],
    }), // 8
    P::cont("Asymmetry", 0.5, Display::Custom(comb_asym)), // 9
    P::cont("Pitch Oct", 0.5, Display::Custom(comb_octave)), // 10
    P::menu_by("Pitch Note", 0.4, KEYS, comb_note), // 11
];

// ---- 41 Waveshaper ----

/// 20·1000^v Hz (a base of 999.4 would print 19.99 kHz at the end).
const WS_CUT: Display = Display::Freq {
    min: 20.0,
    max: 20_000.0,
    hz_decimals: 1,
    khz_decimals: 2,
};

const WAVESHAPER: &[P] = &[
    P::ENABLE, // 0
    P::menu(
        "Shape",
        0.0,
        &[
            "Smooth",
            "Analog",
            "Moshpit",
            "Foldback",
            "Chainsaw",
            "Rectifuzz",
            "Cyclone",
        ],
    ), // 1
    P::toggle("Auto-Gain", 0.0), // 2
    P::plain("X", 0.5), // 3
    P::plain("Y", 0.5), // 4
    P::plain("Drive", 0.0), // 5
    // The value after creation is −12 dB although the reset default is 0 dB: a new module starts at v = 1/6,
    // and 0.5 is the reset value.
    P::cont("Post Gain", 1.0 / 6.0, DB_18).with_reset(0.5), // 6
    P::plain("Stereo Spread", 0.0),                         // 7
    P::plain("Mix", 1.0),                                   // 8
    P::cont("High Cut", 1.0, WS_CUT),                       // 9
    P::cont("Low Cut", 0.0, WS_CUT),                        // 10
    P::cont("Bass", 0.5, DB_18),                            // 11
    // 500·4^v Hz: 500.0 Hz … 1.00 kHz … 2.00 kHz.
    P::cont(
        "Mid Freq",
        0.5,
        Display::Freq {
            min: 500.0,
            max: 2000.0,
            hz_decimals: 1,
            khz_decimals: 2,
        },
    ), // 12
    P::cont("Mid Gain", 0.5, DB_18),                       // 13
    P::cont("Treble", 0.5, DB_18),                         // 14
    P::menu("LFO Shape", 0.0, &["Sine", "Saw", "Square"]), // 15
    P::toggle("LFO Sync", 0.0),                            // 16
    P::menu_by(
        "LFO Rate",
        0.5,
        &[
            "1/8", "1/4", "3/8", "1/2", "3/4", "beat", "2 beats", "bar", "2 bars", "4 bars",
        ],
        trunc_index::<9>,
    ), // 17
    P::plain("LFO Rate Free", 0.0),                        // 18
    P::cont("LFO to X", 0.0, Display::linear(0.0, 100.0, 1, " %")), // 19
    P::cont("LFO to Y", 0.0, Display::linear(0.0, 100.0, 1, " %")), // 20
];

// ---- 42 Tape Stop ----

// Value texts. Free times: round(5000·v) ms, "Off" at 0, "%.0f ms" below one second and "%.2f s" from there.
// Synced times: ten steps with beat labels; a list of seconds ("556 ms" … "5.00 s") is the same ten steps at
// 0 BPM (see KIND_NOTES in the tests).
const TAPE_STOP_TIME: Display = Display::Millis {
    full: 5000.0,
    zero: "Off",
};
const TAPE_STOP_BEATS: &[&str] = &[
    "Off", "1/4", "3/8", "1/2", "3/4", "1 beat", "2 beats", "4 beats", "8 beats", "16 beats",
];

const TAPE_STOP: &[P] = &[
    P::ENABLE,
    P::cont("Stop Time", 0.2, TAPE_STOP_TIME),
    P::cont("Start Time", 0.2, TAPE_STOP_TIME),
    P::menu("Stop Time", 3.0 / 9.0, TAPE_STOP_BEATS),
    P::menu("Start Time", 3.0 / 9.0, TAPE_STOP_BEATS),
    P::toggle("Toggle Play", 1.0),
    P::cont("Warp", 0.5, Display::Const("Warp")),
    P::toggle("Sync", 0.0),
];

// ---- 43 Wow & Flutter ----

const WOW_AND_FLUTTER: &[P] = &[
    P::ENABLE,                                                        // 0
    P::cont("Wow Rate", 0.1846, Display::linear(0.1, 5.0, 2, " Hz")), // 1
    // The synced rate menus show item trunc(9v): 0.2 reads "1/4".
    P::menu_by("Wow Rate", 0.2, BEAT_RATES, trunc_index::<9>), // 2
    P::cont("Wow Amount", 0.25, PERCENT),                      // 3
    P::cont("Flutter Rate", 0.0668, Display::linear(5.0, 20.0, 2, " Hz")), // 4
    P::menu_by("Flutter Rate", 0.1, BEAT_RATES, trunc_index::<9>), // 5
    P::cont("Flutter Amount", 0.25, PERCENT),                  // 6
    P::toggle("Sync", 0.0),                                    // 7
    P::cont("Random Rate", 0.0895, Display::linear(0.1, 10.0, 1, " Hz")), // 8
    P::menu_by("Random Rate", 0.8, BEAT_RATES, trunc_index::<9>), // 9
    P::cont("Random Amount", 0.25, PERCENT),                   // 10
];

// ---- 44 Spreader ----

/// Low Bypass: f = 10·1000^v Hz in f32; whole Hz below 1000, "10 kHz" within 0.1 of 10000, else kHz with one decimal.
fn low_bypass(v: f32) -> String {
    let hz = 10.0 * 1000f32.powf(v);
    if hz < 1000.0 {
        format!("{hz:.0} Hz")
    } else if (hz - 10_000.0).abs() < 0.1 {
        "10 kHz".to_owned()
    } else {
        format!("{:.1} kHz", hz / 1000.0)
    }
}

/// Separation: s = −100 + 200v in f32; "Original" below 0.1 from zero, one decimal below 10, whole numbers from there.
fn spreader_separation(v: f32) -> String {
    let s = 200f32.mul_add(v, -100.0);
    match s.abs() {
        a if a < 0.1 => "Original".to_owned(),
        a if a < 10.0 => format!("{s:.1} %"),
        _ => format!("{s:.0} %"),
    }
}

const SPREADER: &[P] = &[
    P::ENABLE,
    P::cont("Width", 0.0, Display::linear(0.0, 2.0, 1, "")),
    P::cont("Low Bypass", 0.0, Display::Custom(low_bypass)),
    // The round "Mono" button (on from 0.5) reaches it, but the text is the generic one.
    P::plain("Mono Bass", 0.0),
    P::cont("Separation", 0.5, Display::Custom(spreader_separation)),
];

// ---- 45 Pitcher 2 ----

const PITCHER2: &[P] = &[
    P::ENABLE,                                      // 0
    note("Note C", true),                           // 1
    note("Note Db", false),                         // 2
    note("Note D", true),                           // 3
    note("Note Eb", true),                          // 4
    note("Note E", false),                          // 5
    note("Note F", true),                           // 6
    note("Note Gb", false),                         // 7
    note("Note G", true),                           // 8
    note("Note Ab", true),                          // 9
    note("Note A", false),                          // 10
    note("Note Bb", true),                          // 11
    note("Note B", false),                          // 12
    P::plain("Speed", 0.5),                         // 13
    P::plain("", 0.0),                              // 14
    P::plain("", 0.0),                              // 15
    P::toggle("Formant Mode", 0.0),                 // 16
    P::cont("Formant Ratio", 0.5, BIPOLAR_PERCENT), // 17
    P::menu_by(
        "Ref. Channel",
        1.0,
        &["Left channel", "Right channel", "Left + Right"],
        trunc_index::<2>,
    ), // 18
    P::cont("Stability", 0.5, PERCENT_TIGHT),       // 19
    P::cont("Humanization", 0.0, PERCENT_TIGHT),    // 20
    P::toggle("Snapping", 0.0),                     // 21
    P::cont("Mix", 1.0, PERCENT_TIGHT),             // 22
    P::cont("Sensitivity", 0.5, PERCENT_TIGHT),     // 23
    P::cont("Vibrato", 0.0, PERCENT_TIGHT),         // 24
    P::toggle("Latency", 0.0),                      // 25
    P::toggle("Process Sibilants", 0.0),            // 26
];

// ---- 46 Tremolo ----

/// 100v as a whole number: Tremolo's own formatter, which rounds where the generic law truncates.
const WHOLE_100: Display = Display::linear(0.0, 100.0, 0, "");

const TREMOLO: &[P] = &[
    P::ENABLE,                                                            // 0
    P::cont("Rate Free", 0.045467, Display::linear(0.1, 20.0, 2, " Hz")), // 1
    // Item trunc(9v), in f64. The automation texts are in a scrambled order (kept); the menu shows the labels.
    P::menu_by("Rate Sync", 0.0, BEAT_RATES, trunc64_index::<9>).with_automation(ParamKind::Menu {
        items: &[
            "1/4", "3/8", "1/2", "3/4", "2 beats", "16 beats", "1/8", "1/8", "1/8", "1/8",
        ],
    }), // 2
    P::cont("Amount Max", 1.0, Display::linear(0.0, 100.0, 0, " %")), // 3
    P::cont("Amount Min", 0.0, Display::linear(0.0, 100.0, 0, " %")), // 4
    P::cont("Phase Offset", 0.0, Display::linear(0.0, 360.0, 2, "")), // 5
    P::cont("Stereo Offset", 0.0, Display::linear(0.0, 360.0, 2, "")), // 6
    P::toggle("Sync", 0.0),                                           // 7
    P::cont("Shape", 0.5, WHOLE_100),                                 // 8
    P::cont("Smooth", 1.0, WHOLE_100),                                // 9
    P::cont("Skew", 0.5, Display::linear(-100.0, 100.0, 0, "")),      // 10
    P::cont("Width", 0.5, WHOLE_100),                                 // 11
];

// ---- 47 Note Echo ----

/// The delay in beats, both modes: b = 1/32 + (8 − 1/32)·v² in f32.
const ECHO_BEATS: Display = Display::power(1.0 / 32.0, 8.0, 2, 2, "");
/// `Delay (sync)`: the items and their beats.
const ECHO_ITEMS: [(f32, &str); 10] = [
    (0.125, "1/8"),
    (0.25, "1/4"),
    (0.375, "3/8"),
    (0.5, "1/2"),
    (0.75, "3/4"),
    (1.0, "1"),
    (2.0, "2"),
    (3.0, "3"),
    (4.0, "4"),
    (8.0, "8"),
];

/// The item nearest to b (in f32), ties to the earlier one.
fn echo_item(v: f32) -> usize {
    let b = (8.0 - 1.0 / 32.0f32).mul_add(v * v, 1.0 / 32.0);
    let off = |i: usize| (b - ECHO_ITEMS[i].0).abs();
    (0..ECHO_ITEMS.len())
        .min_by(|&i, &j| off(i).total_cmp(&off(j)))
        .unwrap_or(0)
}

/// The name of [`echo_item`], in f32.
fn echo_sync_text(v: f32) -> String {
    ECHO_ITEMS[echo_item(v)].1.to_owned()
}

/// Note Echo `Delay (sync)` (p3) snapped to its item: v_i = √((b_i − 1/32)/(8 − 1/32)). Writing p3 while Sync is On
/// stores this.
pub fn snap_echo_delay(v: f32) -> f32 {
    let b = ECHO_ITEMS[echo_item(super::display::clamp01(v))].0;
    ((b - 1.0 / 32.0) / (8.0 - 1.0 / 32.0)).sqrt()
}

/// -90 … 0 … 100 %: Vel. Offset and Vel. Drift.
const VELOCITY_RANGE: Display = Display::Bipolar {
    neg: 90.0,
    pos: 100.0,
    fmt: Fmt::new(0, "%"),
};

const NOTE_ECHO: &[P] = &[
    P::ENABLE, // 0
    P::cont(
        "Repetitions",
        0.25,
        Display::Steps {
            min: 0,
            count: 33,
            fmt: Fmt::new(0, ""),
        },
    ), // 1
    // The delay param Sync does not select reads the generic text instead (`24` for p2 at its default while Sync is
    // On); its control is hidden then, and the registry keeps the visible law.
    P::cont("Delay", 0.242536, ECHO_BEATS), // 2
    P::cont("Delay (sync)", 0.242536, Display::Custom(echo_sync_text)), // 3
    P::toggle("Sync", 1.0),                 // 4
    P::cont(
        "Pitch Offset",
        0.5,
        Display::Stepped {
            count: 49,
            text: |v| signed_step(48.0 * v - 24.0),
        },
    ), // 5
    P::cont("Vel. Offset", 0.5, VELOCITY_RANGE), // 6
    P::cont("Velocity", 0.5, VELOCITY_RANGE), // 7
    P::cont(
        "Pitch",
        0.5,
        Display::Stepped {
            count: 25,
            text: |v| signed_step(24.0 * v - 12.0),
        },
    ), // 8
    P::cont("Time Drift", 0.5, Display::linear(-30.0, 30.0, 0, "%")), // 9
    P::cont(
        "Length Drift",
        0.5,
        Display::Bipolar {
            neg: 50.0,
            pos: 100.0,
            fmt: Fmt::new(0, "%"),
        },
    ), // 10
    P::cont("Delay Rand.", 0.0, PERCENT_TIGHT), // 11
    P::cont(
        "Pan Offset",
        0.5,
        signed_linear(-1.0, 1.0, 2, "", Sign::Always),
    ), // 12
    P::cont(
        "Pan Drift",
        0.5,
        signed_linear(-50.0, 50.0, 0, "%", Sign::Always),
    ), // 13
    P::toggle("Ping-Pong", 0.0),            // 14
    // Hidden; it always reads "0".
    P::cont("Stop", 0.0, Display::Const("0")), // 15
];

// ---- 48 Transpose ----

const TRANSPOSE: &[P] = &[
    P::ENABLE,
    P::cont(
        "Octave",
        0.5,
        Display::Stepped {
            count: 9,
            text: |v| signed_step(8.0 * v - 4.0),
        },
    ),
    P::cont(
        "Semitone",
        0.5,
        Display::Stepped {
            count: 25,
            text: |v| signed_step(24.0 * v - 12.0),
        },
    ),
];

// ---- 49 Randomizer ----

const RANDOMIZER: &[P] = &[
    P::ENABLE,
    P::cont(
        "Pitch",
        0.0,
        Display::Steps {
            min: 0,
            count: 25,
            fmt: Fmt::new(0, " st"),
        },
    ),
    P::cont("Velocity", 0.0, PERCENT_TIGHT),
    P::cont("Pan", 0.0, PERCENT_TIGHT),
    P::cont("Gate", 1.0, PERCENT_TIGHT),
];

// ---- 50 Scale ----

const SCALES: &[&str] = &[
    "Major",
    "Major Bebop",
    "Major Bulgarian",
    "Major Pentatonic",
    "Major Persian",
    "Major Polymode",
    "Lydian",
    "Mixolydian",
    "Phrygian Dominant",
    "Minor Harmonic",
    "Minor Hungarian",
    "Minor Melodic",
    "Minor Natural",
    "Minor Neapolitan",
    "Minor Pentatonic",
    "Minor Polymode",
    "Minor Romanian",
    "Dorian",
    "Phrygian",
    "Locrian",
    "Chromatic",
    "Arabic",
    "Blues",
    "Diminished",
    "Dominant Bebop",
    "Egyptian",
    "Enigmatic",
    "Hirajoshi",
    "Iwato",
    "Japanese Insen",
    "Locrian Super",
    "Neapolitan",
    "Whole Tone",
    "Custom",
];

const SCALE: &[P] = &[
    P::ENABLE,                     // 0
    P::menu("Key", 0.0, KEYS),     // 1
    P::menu("Scale", 0.0, SCALES), // 2
    P::toggle("Snap", 1.0),        // 3
    // C major.
    note("Note C", true),   // 4
    note("Note C#", false), // 5
    note("Note D", true),   // 6
    note("Note D#", false), // 7
    note("Note E", true),   // 8
    note("Note F", true),   // 9
    note("Note F#", false), // 10
    note("Note G", true),   // 11
    note("Note G#", false), // 12
    note("Note A", true),   // 13
    note("Note A#", false), // 14
    note("Note B", true),   // 15
];

// ---- 51 Arpeggiator ----

const ARP_PATTERNS: &[&str] = &[
    "As Played",
    "Chord",
    "Up",
    "Down",
    "Up-Down",
    "Down-Up",
    "Outward",
    "Inward",
    "Anchor",
    "Bass Up",
    "Bass Up-Down",
    "Treble Down",
    "Treble Down-Up",
    "Jump Up",
    "Jump Down",
    "Jump Up-Down",
    "SuperSaw",
    "Cascade Up",
    "Cascade Down",
    "Cascade Up-Down",
    "Random",
    "Rand No Repeat",
    "Random Once",
    "Random Walk",
];
const ARP_RATE_MULTIPLIERS: &[&str] = &[
    "x 1/8", "x 1/6", "x 1/4", "x 1/3", "x 1/2", "x 2/3", "x 1", "x 3/2", "x 2", "x 3", "x 4",
    "x 5", "x 6", "x 7", "x 8",
];
const ARP_START_NOTES: &[&str] = &[
    "- 8", "- 7", "- 6", "- 5", "- 4", "- 3", "- 2", "- 1", "Orig", "+ 1", "+ 2", "+ 3", "+ 4",
    "+ 5", "+ 6", "+ 7", "+ 8",
];
const ARP_RHYTHMS: &[&str] = &[
    "Off", "R1", "R2", "R3", "R4", "R5", "R6", "R7", "R8", "R9", "R10", "R11", "R12", "R13", "R14",
    "R15", "R16", "R17", "R18", "R19", "R20", "R21",
];
const ARP_REPEATS: &[&str] = &[
    "inf", "1", "2", "3", "4", "5", "6", "7", "8", "9", "10", "11", "12", "13", "14", "15", "16",
];
const ARP_RETRIG: &[&str] = &[
    "Off", "Notes", "1/8", "1/4", "3/8", "1/2", "3/4", "1 bar", "2 bars", "3 bars", "4 bars",
    "8 bars",
];
const ARP_RANGE: &[&str] = &["1 oct", "2 oct", "3 oct", "4 oct"];
const ARP_OCTAVE_SHIFT: &[&str] = &[
    "-3 oct", "-2 oct", "-1 oct", "None", "+1 oct", "+2 oct", "+3 oct",
];
/// 10.2 + 90·v %: 10% … 55% … 100%.
const ARP_GATE: Display = Display::linear(10.2, 10.2 + 90.0, 0, "%");
/// The Rhythm patterns R1 … R21 (`x` on): the params layer owns them, the Arpeggiator panel's ring and write snap
/// import them, and a pattern's step count is its `.len()`. Two roundings read the count and stay apart: the Rotate
/// text rounds in f32 (`llroundf`, [`arp_rotate`]), the panel's write snap in f64 as the engine does.
pub(crate) const ARP_RHYTHM_PATTERNS: [&str; 21] = [
    "x.", "x.x", "x..x", "x.xx", "x.x.x", "x..x.x", "x..xx.", "x.x.xx", "x..x.x.", "x..x.xx",
    "x..xx.x", "x.x.x.x", "x.xx.xx", "x...x.xx", "x...xx.x", "x..x..x.", "x..x.xx.", "x..x.xxx",
    "x..xx.x.", "x.x.x.xx", "x.x.xx.x",
];

// The Rhythm menu is Off, then the patterns.
const _: () = assert!(ARP_RHYTHMS.len() == ARP_RHYTHM_PATTERNS.len() + 1);

/// The pattern (index into [`ARP_RHYTHM_PATTERNS`]) the normalised `Rhythm` `v` shows, `None` for Off: the one map
/// from the param to a pattern, for the Rotate text ([`arp_rotate`]) and the Arpeggiator panel's write snap and ring.
pub(crate) fn arp_pattern(v: f32) -> Option<usize> {
    menu_index(v, ARP_RHYTHMS.len()).checked_sub(1)
}

/// The step count of the pattern the normalised `Rhythm` `v` shows, `None` for Off.
pub(crate) fn arp_steps(v: f32) -> Option<usize> {
    arp_pattern(v).map(|p| ARP_RHYTHM_PATTERNS[p].len())
}

/// Rotate of the arp whose Rhythm is p`RHYTHM`: −(n−1) … 0 … +(n−1), n = step count of Rhythm; `%d` of
/// llroundf((2v−1)(n−1)), no `+`; Rhythm Off → `0`.
fn arp_rotate<const RHYTHM: usize>(v: f32, cx: TextCx) -> String {
    let Some(n) = arp_steps(cx.param(RHYTHM, ARPEGGIATOR[RHYTHM].default)) else {
        return "0".to_owned();
    };
    (((2.0 * v - 1.0) * (n - 1) as f32).round() as i32).to_string()
}

const ARPEGGIATOR: &[P] = &[
    P::ENABLE, // 0
    P::menu(
        "Rate",
        2.0 / 7.0,
        &["1/16", "1/12", "1/8", "1/6", "1/4", "1/3", "1/2", "1"],
    ), // 1
    P::cont("Rate (ms)", 0.2105, Display::linear(50.0, 1000.0, 0, " ms")), // 2
    P::toggle("Sync", 1.0), // 3
    P::cont("Swing", 0.0, PERCENT), // 4
    P::menu("Swing rate", 0.0, &["1/8", "1/16"]), // 5
    P::toggle("Latch", 0.0), // 6
    P::cont("Velocity 1", 0.5, Display::linear(-100.0, 100.0, 0, " %")), // 7
    P::cont("Velocity 2", 0.5, Display::linear(-100.0, 100.0, 0, " %")), // 8
    P::cont("Gate (Arp 1)", 0.4444, ARP_GATE), // 9
    P::menu("Arp (Arp 1)", 0.0, ARP_PATTERNS), // 10
    P::menu("Range (Arp 1)", 0.0, ARP_RANGE), // 11
    P::menu("Octave shift (Arp 1)", 0.5, ARP_OCTAVE_SHIFT), // 12
    P::menu("Start note (Arp 1)", 0.5, ARP_START_NOTES), // 13
    P::menu("Rhythm (Arp 1)", 0.0, ARP_RHYTHMS), // 14
    P::cont("Rotate (Arp 1)", 0.5, Display::CustomCx(arp_rotate::<14>)), // 15
    P::plain("", 0.0), // 16 hidden
    P::menu("Repeats (Arp 1)", 0.0, ARP_REPEATS), // 17
    P::menu("Retrig (Arp 1)", 0.0, ARP_RETRIG), // 18
    P::toggle("Enable (Arp 1)", 1.0), // 19
    P::cont("Gate (Arp 2)", 0.4444, ARP_GATE), // 20
    P::menu("Arp (Arp 2)", 0.0, ARP_PATTERNS), // 21
    P::menu("Range (Arp 2)", 0.0, ARP_RANGE), // 22
    P::menu("Octave shift (Arp 2)", 0.5, ARP_OCTAVE_SHIFT), // 23
    P::menu("Start note (Arp 2)", 0.5, ARP_START_NOTES), // 24
    P::menu("Rhythm (Arp 2)", 0.0, ARP_RHYTHMS), // 25
    P::cont("Rotate (Arp 2)", 0.5, Display::CustomCx(arp_rotate::<25>)), // 26
    P::plain("", 0.0), // 27 hidden
    P::menu("Repeats (Arp 2)", 0.0, ARP_REPEATS), // 28
    P::menu("Retrig (Arp 2)", 0.0, ARP_RETRIG), // 29
    P::toggle("Enable (Arp 2)", 0.0), // 30
    P::menu("Rate (Arp 1)", 6.0 / 14.0, ARP_RATE_MULTIPLIERS), // 31
    P::menu("Rate (Arp 2)", 6.0 / 14.0, ARP_RATE_MULTIPLIERS), // 32
];

pub static MODULES: &[ModuleSpec] = &[
    ModuleSpec::effect(24, "Tuner", "Tuner", false, TUNER, &[]),
    ModuleSpec::effect(
        25,
        "Pitcher",
        "Pitcher",
        false,
        PITCHER,
        &["Basic", "Advanced"],
    ),
    ModuleSpec::effect(
        26,
        "Analyzer",
        "Analyzer",
        false,
        ANALYZER,
        &[
            "SPECTRUM", "SCOPE", "LOUDNESS", "STEREO", "PITCH", "SETTINGS",
        ],
    ),
    ModuleSpec::effect(27, "Gate", "Gate", false, GATE, &["LEVEL", "ENVELOPE"]),
    ModuleSpec::effect(28, "Limiter", "Limiter", false, LIMITER, &[]),
    ModuleSpec::effect(29, "Autoduck", "Autoduck", false, AUTODUCK, &[]),
    ModuleSpec::effect(30, "Tuned EQ", "Tuned EQ", false, TUNED_EQ, KEYS),
    ModuleSpec::effect(
        32,
        "Reverb",
        "Reverb",
        false,
        REVERB,
        &["MAIN", "TUNE", "MIXER"],
    ),
    ModuleSpec::effect(34, "Spacer", "Spacer", false, SPACER, &[]),
    ModuleSpec::effect(
        35,
        "Reverb 2",
        "Reverb 2",
        false,
        REVERB2,
        &["MAIN", "TUNE"],
    ),
    ModuleSpec::effect(
        36,
        "Multiband Compressor",
        "Multiband Compressor",
        false,
        MULTIBAND_COMPRESSOR,
        &["Master", "Low", "Mid", "High"],
    ),
    ModuleSpec::effect(40, "Comb Filter", "Comb Filter", false, COMB_FILTER, &[]),
    ModuleSpec::effect(
        41,
        "Waveshaper",
        "Waveshaper",
        false,
        WAVESHAPER,
        &["SHAPE", "AMP", "EQ", "LFO"],
    ),
    ModuleSpec::effect(42, "Tape Stop", "Tape Stop", false, TAPE_STOP, &[]),
    ModuleSpec::effect(
        43,
        "Wow & Flutter",
        "Wow & Flutter",
        false,
        WOW_AND_FLUTTER,
        &[],
    ),
    ModuleSpec::effect(44, "Spreader", "Spreader", false, SPREADER, &[]),
    ModuleSpec::effect(
        45,
        "Pitcher 2",
        "Pitcher 2",
        false,
        PITCHER2,
        &["TUNING", "EXPRESSION", "OPTIONS"],
    ),
    ModuleSpec::effect(46, "Tremolo", "Tremolo", false, TREMOLO, &[]),
    ModuleSpec::note_effect(
        47,
        "Note Echo",
        "Note Echo",
        NOTE_ECHO,
        &["TIME", "PITCH", "VELOCITY", "PAN"],
    ),
    ModuleSpec::note_effect(48, "Transpose", "Transpose", TRANSPOSE, &[]),
    ModuleSpec::note_effect(49, "Randomizer", "Randomizer", RANDOMIZER, &[]),
    ModuleSpec::note_effect(50, "Scale", "Scale", SCALE, &[]),
    ModuleSpec::note_effect(
        51,
        "Arpeggiator",
        "Arpeggiator",
        ARPEGGIATOR,
        &["GLOBAL", "ARP 1", "ARP 2"],
    ),
];
