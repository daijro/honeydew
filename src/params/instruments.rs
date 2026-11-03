//! Parameter tables of the instruments, IDs 0-52.
//!
//! Same conventions as the effect tables: `params[i]` is param number `i`, not the order of the UI, names are
//! copied as written ("" for none), and for modules with ten or more params every line ends in its index. Menus
//! hold the UI labels where the automation texts differ (MiniSynth `OSC Kind`, GMSynth `Filter Kind`, …), and
//! `with_automation` keeps the automation texts for the automation lane; the tests list every such difference
//! (`KIND_NOTES`). Where a param shows a unit on its default value but has a generic 0…100 (v×100) range (3x Osc
//! "50%"), the unit comes from the default (the generic `%` law, `PERCENT_TIGHT`). `with_reset` marks the controls
//! with a reset value other than the default.

use super::display::{count_index, half_up_index, trunc_index, trunc64_index};
use super::effects::{AMOUNT_TRUNC, BIPOLAR_PERCENT, DB_12, OFF_ON, PERCENT_TIGHT};
use super::{At, Display, Fmt, ModuleSpec, ParamKind, ParamSpec as P, Sign, TextCx};
use honeydew_audio::minisynth::transpose_steps;
use honeydew_audio::three_osc::arp::chord_value;
use honeydew_audio::voice::{lfo_attack_increment, lfo_rate_hz};
use honeydew_audio::{dw_sampler, gmsynth, slicer, supersaw, three_osc};

// ---- shared laws and lists ----

/// -100 … 0 … 100.
const AMOUNT: Display = Display::linear(-100.0, 100.0, 0, "");
/// -100.0 … 0.0 … 100.0: MiniSynth and SuperSaw amounts, 200·v − 100 with one decimal.
const AMOUNT_1: Display = Display::linear(-100.0, 100.0, 1, "");

/// The LFO rate of DW Sampler and SuperSaw, in Hz with two decimals: the engine's law
/// (`honeydew_audio::voice::lfo_rate_hz`).
const LFO_RATE: Display = Display::Custom(|v| Fmt::new(2, " Hz").apply(f64::from(lfo_rate_hz(v))));
/// The automation texts of a menu whose UI labels differ ([`P::with_automation`]).
const fn texts(items: &'static [&'static str]) -> ParamKind {
    ParamKind::Menu { items }
}

const LFO_MODES: &[&str] = &["Hertz", "One Shot", "Tmp Sync"];
const LFO_SHAPES_4: &[&str] = &["Sine", "Saw", "Square", "Random"];

// ---- 0 MiniSynth ----

const MS_DEST: &[&str] = &["FLT", "FRQ", "MOD"];
/// The tempo steps' labels: the text is item trunc(12v) (the DSP plays trunc(11v) of its own period table).
const MS_RATES: &[&str] = &[
    "1/4", "3/8", "1/2", "3/4", "1/1", "2/1", "4/1", "8/1", "1 bar", "2 bars", "4 bars", "8 bars",
    "16 bars",
];

/// Transpose, the audio's fused step as `%d`.
const MS_TRANSPOSE: Display = Display::Stepped {
    count: 25,
    text: |v| transpose_steps(v).to_string(),
};

// Reset values: MW 1.0; Modifier, Noise and LFO Rate 0.5; AMP DEC/SUS/REL 0; every FX-page knob 0.
const MINISYNTH: &[P] = &[
    P::ENABLE, // 0
    P::menu_by(
        "Voices Mode",
        0.0,
        &["POLY", "MONO", "LEAD"],
        trunc_index::<2>,
    ), // 1
    P::cont("Transpose", 0.5, MS_TRANSPOSE), // 2
    P::plain("Slide Time", 0.0), // 3
    P::menu_by("MW Dest", 0.0, MS_DEST, trunc_index::<2>)
        .with_automation(texts(&["FLT", "LFO", "OSCMOD"])), // 4
    P::plain("MW Amount", 0.0).with_reset(1.0), // 5
    P::menu_by(
        "OSC Kind",
        0.0,
        &[
            "SAW",
            "SAW+SAW",
            "PULSE",
            "SAW + SQUARE",
            "SQUARE+SQUARE",
            "SUPERSAW",
            "BELL",
            "DEAF SAW",
            "SPREAD OCT",
            "SPREAD 5th",
        ],
        trunc_index::<9>,
    )
    .with_automation(texts(&[
        "SAW",
        "SAW+SAW",
        "PULSE",
        "SQR+SAW",
        "SQR+SQR",
        "SUPERSAW",
        "BELL",
        "DEAF SAW",
        "SAW SPREAD",
        "SAW SPREAD 2",
    ])), // 6
    P::plain("OSC Mod", 0.0).with_reset(0.5), // 7
    P::plain("OSC Noise", 0.0).with_reset(0.5), // 8
    P::plain("FLT Freq", 1.0), // 9
    P::plain("FLT Peak", 0.0), // 10
    P::plain("FLT Kbd", 0.0), // 11
    P::plain("FLT Vel", 0.0), // 12
    P::plain("EGF Atk", 0.0), // 13
    P::plain("EGF Dec", 1.0), // 14
    P::plain("EGF Sus", 1.0), // 15
    P::plain("EGF Rel", 1.0), // 16
    P::cont("EGF Amnt", 0.5, AMOUNT_1), // 17
    P::menu_by("LFO Shape", 0.0, &["SIN", "SAW", "SQR"], trunc_index::<2>)
        .with_automation(texts(&["SINE", "SAW", "SQUARE"])), // 18
    P::menu_by("LFO Dest", 0.0, MS_DEST, trunc_index::<2>)
        .with_automation(texts(&["FLT", "FRQ", "OSCMOD"])), // 19
    P::menu_by("LFO Rate", 0.25, MS_RATES, trunc_index::<12>).with_reset(0.5), // 20
    P::cont("LFO Amnt", 0.5, AMOUNT_1), // 21
    P::plain("AMP Atk", 0.0), // 22
    P::plain("AMP Dec", 1.0).with_reset(0.0), // 23
    P::plain("AMP Sus", 1.0).with_reset(0.0), // 24
    P::plain("AMP Rel", 0.07).with_reset(0.0), // 25
    P::toggle("DIST", 0.0), // 26
    P::plain("Overdrive", 0.0).with_reset(0.0), // 27
    P::plain("Decimator", 0.0).with_reset(0.0), // 28
    P::toggle("DELAY", 0.0), // 29
    P::menu_by(
        "DELAY Time",
        0.7,
        &["1/4", "2/4", "3/4", "1/1"],
        trunc_index::<3>,
    )
    .with_reset(0.0), // 30
    P::plain("DELAY Fdbk", 0.5).with_reset(0.0), // 31
    P::plain("DELAY Filter", 1.0).with_reset(0.0), // 32
    P::plain("DELAY Mix", 0.5).with_reset(0.0), // 33
    P::toggle("CHORUS", 0.0), // 34
    P::plain("CHORUS ModRate", 0.25).with_reset(0.0), // 35
    P::plain("CHORUS ModDepth", 0.5).with_reset(0.0), // 36
    P::plain("CHORUS Mix", 0.5).with_reset(0.0), // 37
    P::toggle("PHASER", 0.0), // 38
    P::menu_by("PHASER ModSpeed", 0.1122, MS_RATES, trunc_index::<12>).with_reset(0.0), // 39
    P::plain("PHASER ModWidth", 0.4).with_reset(0.0), // 40
    P::plain("PHASER Fdbk", 0.1).with_reset(0.0), // 41
    P::plain("PHASER Mix", 1.0).with_reset(0.0), // 42
    P::plain("PHASER Freq", 0.5).with_reset(0.0), // 43
];

// ---- 1 DW Sampler ----

const DW_SAMPLER: &[P] = &[
    P::ENABLE,                // 0
    P::plain("Attack", 0.0),  // 1
    P::plain("Release", 0.1), // 2
    // The audio's f32 fused step.
    P::cont(
        "Pitch",
        0.5,
        Display::Stepped {
            count: 49,
            text: |v| dw_sampler::pitch_shift(v).to_string(),
        },
    ), // 3
    P::cont("Fine Tune", 0.5, AMOUNT), // 4
    P::plain("Decay", 1.0),            // 5
    P::plain("Sustain", 1.0),          // 6
    P::plain("Slide", 0.0),            // 7
    P::toggle("Reverse", 0.0),         // 8
    P::toggle("Loop", 1.0),            // 9
    P::toggle("Legato", 0.0),          // 10
    P::toggle("Mono", 0.0),            // 11
    P::plain("Cutoff", 1.0),           // 12
    P::plain("Resonance", 0.0),        // 13
    P::plain("Kbd. Track", 0.0),       // 14
    P::plain("Vel. Track", 0.0),       // 15
    P::menu_by(
        "Filter type",
        0.0,
        &["Off", "Lowpass", "Bandpass", "Highpass"],
        trunc_index::<3>,
    ), // 16
    P::plain("Filter Attack", 0.0),    // 17
    P::plain("Filter Decay", 1.0),     // 18
    P::plain("Filter Sustain", 1.0),   // 19
    P::plain("Filter Release", 1.0),   // 20
    P::plain("Filter Env. Amount", 0.5), // 21
    P::plain("Pitch Attack", 0.0),     // 22
    P::plain("Pitch Decay", 1.0),      // 23
    P::plain("Pitch Sustain", 1.0),    // 24
    P::plain("Pitch Release", 1.0),    // 25
    P::plain("Pitch Env. Amount", 0.5), // 26
    P::menu_by("LFO Shape", 0.0, LFO_SHAPES_4, half_up_index::<3>), // 27
    P::cont("LFO Rate", 0.0, LFO_RATE), // 28
    P::menu_by("LFO Mode", 0.0, LFO_MODES, trunc_index::<2>), // 29
    P::menu_by(
        "LFO Sync Rate",
        0.0,
        &[
            "1/8", "1/4", "1/2", "3/4", "1/1", "2/1", "4/1", "8/1", "16/1", "32/1",
        ],
        trunc64_index::<9>,
    ), // 30
    // trunc(200·v − 100) in double precision, `%d`, as Distortion Bias.
    P::cont(
        "LFO Amount",
        0.5,
        Display::Trunc64 {
            min: -100.0,
            max: 100.0,
            unit: "",
        },
    ), // 31
    P::menu_by(
        "LFO Destination",
        0.0,
        &["Filter", "Frequency", "Level", "Pan"],
        trunc_index::<3>,
    ), // 32
    P::cont("LFO Attack", 0.0, Display::CustomCx(dw_lfo_attack)), // 33
];

/// LFO Attack, sample-rate dependent: x < 0.001 → c = 1, else c = 0.0002/(500·(0.95x+0.05)⁴) per sample;
/// t = 1/(c·Fs) s. t < 1 s → trunc(1000t) " ms", else two decimals + " s". Fs = 0 → "inf s". In f32; c is the
/// engine's own law (`honeydew_audio::voice::lfo_attack_increment`).
fn dw_lfo_attack(x: f32, cx: TextCx) -> String {
    let t = 1.0 / (lfo_attack_increment(x) * cx.sample_rate);
    if t < 1.0 {
        format!("{} ms", (1000.0 * t) as i32)
    } else {
        format!("{t:.2} s")
    }
}

// ---- 2 GMSynth ----

const GMS_RATES: &[&str] = &[
    "1/4", "3/8", "1/2", "3/4", "Beat", "2 Beats", "Bar", "2 Bars", "4 Bars", "8 Bars",
];
const GMS_SHAPES: &[&str] = &["Sine", "Saw", "Square"];
/// Automation texts of the tempo menus and LFO shapes.
const GMS_RATE_TEXTS: ParamKind = texts(&[
    "1/4", "3/8", "1/2", "3/4", "1/1", "2/1", "4/1", "8/1", "16/1", "32/1",
]);
const GMS_SHAPE_TEXTS: ParamKind = texts(&["Sine", "Saw", "Rect"]);
/// Automation texts of the FX kinds.
const GMS_FX_TEXTS: ParamKind = texts(&[
    "DIST", "FLNG", "PHAS", "F LP", "F HP", "DLY", "RVRB", "PAN", "RING", "VOX",
]);
const GMS_FX: &[&str] = &[
    "DIST", "FLANGER", "PHASER", "LPF", "HPF", "DELAY", "REVERB", "PANNER", "RING", "VOX",
];

/// TRNSP and PITCH B as `%d`: the audio's steps, TRNSP f32 fused and PITCH B f64. PITCH B shows the step it plays,
/// which the f32 fused step would miss at some steps.
const GMS_TRANSPOSE: Display = Display::Stepped {
    count: 49,
    text: |v| (gmsynth::transpose_step(v) as i32).to_string(),
};
const GMS_PITCH_B: Display = Display::Stepped {
    count: 49,
    text: |v| (gmsynth::pitch_b_step(v) as i32).to_string(),
};

/// GMSynth's waveshapes, the OSC `A` and `B` menus in index order 0–28: the `CSTM` `waveA` /
/// `waveB` values, not params. The panel's spinners and the `.flgsynth` reader name them so.
pub(crate) const GMSYNTH_WAVESHAPES: &[&str] = &[
    "Beast",
    "Crunch",
    "Double Spike",
    "Digital",
    "Dirt",
    "Electro",
    "Filth",
    "Fox",
    "Funky",
    "Grubby",
    "Keys",
    "Liquid",
    "Ogre",
    "Rhodes",
    "Saw Mean",
    "Saw Retro",
    "Saw",
    "Silk",
    "Sine",
    "Smooth Sub",
    "Square Retro",
    "Square Smooth",
    "Square",
    "Strat",
    "String",
    "Throat",
    "Triangle",
    "Troll",
    "Vocal",
];

const GMSYNTH: &[P] = &[
    P::ENABLE,                                                        // 0
    P::cont("Level", 0.7273, Display::linear(-48.0, 18.0, 1, " dB")), // 1
    P::cont(
        "Pan",
        0.5,
        Display::Pan {
            decimals: 0,
            unit: " %",
            centre: "center",
            sides: ["L", "R"],
        },
    ), // 2
    P::plain("Level Attack", 0.0),                                    // 3
    P::plain("Level Decay", 1.0),                                     // 4
    P::plain("Level Sustain", 1.0),                                   // 5
    P::plain("Level Release", 0.2),                                   // 6
    // 7-10: hidden, no name (pitch and glide settings).
    P::cont("", 0.5, Display::linear(-12.0, 12.0, 2, "")), // 7
    P::cont(
        "",
        0.5,
        Display::Steps {
            min: -12,
            count: 25,
            fmt: Fmt::new(2, ""),
        },
    ), // 8
    P::cont("", 0.5, Display::linear(-100.0, 100.0, 1, " %")), // 9
    // 200·|2v−1|⁶ ms.
    P::cont(
        "",
        0.5,
        Display::Mirror {
            max: 200.0,
            exp: 6,
            fmt: Fmt::new(3, " ms"),
        },
    ), // 10
    P::cont("Osc B Pitch", 0.5, GMS_PITCH_B), // 11
    P::cont("Osc B Detune", 0.5, Display::linear(-50.0, 50.0, 1, "")), // 12
    P::cont("Osc B Mix", 0.5, Display::linear(-100.0, 100.0, 1, " %")), // 13
    P::plain("Osc B Phase", 0.0),             // 14
    P::plain("OSc Mod", 0.0),                 // 15
    P::menu_by(
        "Osc Mod Kind",
        0.0,
        &["Ring", "FM", "Sync", "Filter"],
        trunc_index::<3>,
    )
    .with_automation(texts(&["AM", "FM", "SNC", "FLT"])), // 16
    P::plain("Osc Mod Attack", 0.0),          // 17
    P::plain("Osc Mod Decay", 1.0),           // 18
    P::cont("Osc Mod Contour", 0.5, AMOUNT_TRUNC), // 19
    P::plain("Osc Noise", 0.0),               // 20
    P::plain("Cutoff", 1.0),                  // 21
    P::plain("Resonance", 0.0),               // 22
    P::menu_by(
        "Filter Kind",
        0.0,
        &["LP 12", "LP 24", "HP"],
        trunc_index::<2>,
    )
    .with_automation(texts(&["LP", "BP", "HP"])), // 23
    P::toggle("Filter Kbd. Scl.", 0.0),       // 24
    P::plain("Filter Attack", 0.0),           // 25
    P::plain("Filter Decay", 1.0),            // 26
    P::cont("Filter Contour", 0.5, AMOUNT_TRUNC), // 27
    P::cont("Transpose", 0.5, GMS_TRANSPOSE), // 28
    P::plain("Slide Time", 0.0),              // 29
    P::toggle("Single Voice", 0.0),           // 30
    P::toggle("Auto Portamento", 0.0),        // 31
    P::menu_by("LFO Rate", 1.0, GMS_RATES, trunc_index::<9>).with_automation(GMS_RATE_TEXTS), // 32
    P::menu_by("LFO Shape", 0.0, GMS_SHAPES, trunc_index::<2>).with_automation(GMS_SHAPE_TEXTS), // 33
    P::cont("LFO Ampl.", 0.5, AMOUNT_TRUNC), // 34
    P::menu_by(
        "LFO Dest",
        0.0,
        &["FLT", "MOD", "FREQ", "LVL", "PAN"],
        trunc_index::<4>,
    ), // 35
    P::plain("EG Attack", 0.0),              // 36
    P::plain("EG Decay", 1.0),               // 37
    P::plain("EG Sustain", 1.0),             // 38
    P::plain("EG Release", 1.0),             // 39
    P::cont("EG Contour", 0.5, AMOUNT_TRUNC), // 40
    P::menu_by(
        "EG Dest",
        0.0,
        &["Filter", "Osc. Mod.", "Pitch"],
        trunc_index::<2>,
    ), // 41
    P::menu_by(
        "Choir Num Voices",
        0.0,
        &["Solo", "Duet", "Trio", "Quartet"],
        trunc_index::<3>,
    )
    .with_automation(texts(&["1", "2", "3", "4"])), // 42
    P::menu_by(
        "Choir Scale",
        0.0,
        &["Flat", "Chord", "Octave"],
        trunc_index::<2>,
    )
    .with_automation(texts(&["MONO", "QNTA", "OCT"])), // 43
    P::plain("Choir Detune", 0.0),           // 44
    P::plain("Choir Stereo", 0.0),           // 45
    P::menu_by("FX 1 Kind", 1.0 / 9.0, GMS_FX, trunc_index::<9>).with_automation(GMS_FX_TEXTS), // 46
    P::toggle("FX 1 Enable", 0.0), // 47
    P::menu_by("FX 1 Sync", 0.0, GMS_RATES, trunc_index::<9>).with_automation(GMS_RATE_TEXTS), // 48
    P::menu_by("FX 1 LFO Shape", 0.0, GMS_SHAPES, trunc_index::<2>)
        .with_automation(GMS_SHAPE_TEXTS), // 49
    P::cont("FX 1 LFO Amount", 0.5, AMOUNT_TRUNC), // 50
    P::plain("FX 1 Param X", 0.5), // 51
    P::plain("FX 1 Param Y", 0.5), // 52
    P::menu_by("FX 2 Kind", 5.0 / 9.0, GMS_FX, trunc_index::<9>).with_automation(GMS_FX_TEXTS), // 53
    P::toggle("FX 2 Enable", 0.0), // 54
    P::menu_by("FX 2 Sync", 0.0, GMS_RATES, trunc_index::<9>).with_automation(GMS_RATE_TEXTS), // 55
    P::menu_by("FX 2 LFO Shape", 0.0, GMS_SHAPES, trunc_index::<2>)
        .with_automation(GMS_SHAPE_TEXTS), // 56
    P::cont("FX 2 LFO Amount", 0.5, AMOUNT_TRUNC), // 57
    P::plain("FX 2 Param X", 0.5), // 58
    P::plain("FX 2 Param Y", 0.5), // 59
    P::menu_by("FX 3 Kind", 3.0 / 9.0, GMS_FX, trunc_index::<9>).with_automation(GMS_FX_TEXTS), // 60
    P::toggle("FX 3 Enable", 0.0), // 61
    P::menu_by("FX 3 Sync", 0.0, GMS_RATES, trunc_index::<9>).with_automation(GMS_RATE_TEXTS), // 62
    P::menu_by("FX 3 LFO Shape", 0.0, GMS_SHAPES, trunc_index::<2>)
        .with_automation(GMS_SHAPE_TEXTS), // 63
    P::cont("FX 3 LFO Amount", 0.5, AMOUNT_TRUNC), // 64
    P::plain("FX 3 Param X", 0.5), // 65
    P::plain("FX 3 Param Y", 0.5), // 66
    // 67-73: hidden, no name (a tone EQ: four gains, a frequency and a gain).
    P::toggle("", 0.0),      // 67
    P::cont("", 0.0, DB_12), // 68
    P::cont("", 0.0, DB_12), // 69
    P::cont("", 0.0, DB_12), // 70
    P::cont("", 0.0, DB_12), // 71
    // 16·1024^v Hz, 512 Hz in the middle (≈ 15.9·1033^v would miss it).
    P::cont("", 0.0, Display::exp(16.0, 16384.0, 0, " Hz")), // 72
    P::cont("", 0.0, DB_12),                                 // 73
    // The EG page's VEL toggle has no name.
    P::toggle("", 0.0), // 74
];

// ---- 19 Transistor Bass ----

// 7-13 have no automation name (UI: the DISTORTION toggle, Drive HP, Drive, Drive Vol., PW, LFO Speed, LFO->PW).
const TRANSISTOR_BASS: &[P] = &[
    P::ENABLE,                 // 0
    P::plain("Waveform", 0.0), // 1
    P::plain("Cutoff", 1.0),   // 2
    P::plain("Reso", 1.0),     // 3
    P::plain("Env Mod", 0.1),  // 4
    P::plain("Decay", 1.0),    // 5
    P::plain("Accent", 0.1),   // 6
    P::toggle("", 0.0),        // 7
    P::plain("", 0.0),         // 8
    P::plain("", 0.3),         // 9
    P::plain("", 0.5),         // 10
    P::plain("", 0.0),         // 11
    P::plain("", 0.0),         // 12
    P::plain("", 0.0),         // 13
];

// ---- 23 SuperSaw ----

const SS_DEST: &[&str] = &[
    "Cutoff",
    "Resonance",
    "Frequency",
    "Osc. Sync",
    "Pulse Width",
    "Unison Dtn.",
    "Level",
    "Pan",
];

const SUPERSAW: &[P] = &[
    P::ENABLE,                         // 0
    P::plain("Osc. Pulse Width", 1.0), // 1
    P::toggle("Osc. Pulse Mode", 0.0), // 2
    P::plain("Osc. Sub Level", 0.0),   // 3
    P::menu_by(
        "Osc. Unisono",
        0.0,
        &[
            "Single",
            "2 Voices",
            "3 Voices",
            "4 Voices",
            "5 Voices",
            "6 Voices",
            "7 Voices",
            "8 Voices",
            "9 Voices",
            "10 Voices",
            "11 Voices",
            "12 Voices",
        ],
        count_index::<11>,
    ), // 4
    P::plain("Osc. Unisono Detune", 0.2), // 5
    P::plain("Osc. Unisono Pan", 0.0), // 6
    P::plain("Osc. Unisono Detune Curve", 0.5), // 7
    P::toggle("Osc. Re-trig Phases", 0.0), // 8
    P::plain("Osc. Phases Offset", 0.0), // 9
    // The SUPERSAW / SUPERSYNC page selector.
    // SUPERSYNC above 0.5, as the engine switches.
    P::menu_by("SuperSync Mode", 0.0, &["SUPERSAW", "SUPERSYNC"], |v| {
        usize::from(v > 0.5)
    })
    .with_automation(ParamKind::Continuous {
        display: Display::Generic,
    }), // 10
    P::plain("Osc. Sync", 0.0),           // 11
    P::plain("Osc. Pulse Width", 1.0),    // 12
    P::plain("Osc. Sync Attack", 0.0),    // 13
    P::plain("Osc. Sync Decay", 1.0),     // 14
    P::plain("Osc. Sync Sustain", 1.0),   // 15
    P::plain("Osc. Sync Release", 1.0),   // 16
    P::plain("Osc. Sync EG Amount", 0.5), // 17
    P::toggle("Osc. Sync Kbd Dir.", 0.0), // 18
    P::plain("Filter Cutoff", 1.0),       // 19
    P::plain("Filter Resonance", 0.0),    // 20
    P::plain("Filter KBD", 0.0),          // 21
    P::plain("Filter EG Attack", 0.0),    // 22
    P::plain("Filter EG Decay", 1.0),     // 23
    P::plain("Filter EG Sustain", 1.0),   // 24
    P::plain("Filter EG Release", 1.0),   // 25
    P::plain("Filter EG Amount", 0.5),    // 26
    P::plain("Filter VEL", 0.0),          // 27
    P::plain("Level EG Attack", 0.0),     // 28
    P::plain("Level EG Decay", 1.0),      // 29
    P::plain("Level EG Sustain", 1.0),    // 30
    P::plain("Level EG Release", 0.0),    // 31
    P::plain("Level", 1.0),               // 32
    P::cont("Pan", 0.5, AMOUNT_1),        // 33
    P::toggle("Slide Held Mode", 0.0),    // 34
    P::plain("Slide Time", 0.0),          // 35
    // trunc(f32(24·v)) − 12: truncates between the steps, 0.53 reads "0".
    P::cont(
        "Transpose",
        0.5,
        Display::Stepped {
            count: 25,
            text: |v| (supersaw::transpose_steps(v) as i32).to_string(),
        },
    ), // 36
    P::menu_by(
        "Pitch Bend Range",
        0.1,
        &[
            "1 St", "2 St", "3 St", "4 St", "5 St", "6 St", "7 St", "8 St", "9 St", "10 St",
            "11 St", "12 St",
        ],
        count_index::<11>,
    ), // 37
    P::toggle("Slide Time Static", 1.0), // 38
    P::menu_by("LFO Shape", 0.0, LFO_SHAPES_4, half_up_index::<3>), // 39
    P::cont("LFO Rate", 0.0, LFO_RATE),  // 40
    P::menu_by("LFO Mode", 0.0, LFO_MODES, trunc_index::<2>), // 41
    // The order has 3/4 before 1/2.
    P::menu_by(
        "LFO Beat",
        0.0,
        &[
            "1/8", "1/4", "3/4", "1/2", "1/1", "2/1", "4/1", "8/1", "16/1", "32/1",
        ],
        trunc64_index::<9>,
    ), // 42
    // Hidden, no name.
    P::plain("", 0.0),                                        // 43
    P::cont("LFO Amount", 0.5, AMOUNT_1),                     // 44
    P::menu_by("LFO Dest", 0.0, SS_DEST, half_up_index::<7>), // 45
    P::plain("LFO Fade In", 0.0),                             // 46
    P::menu_by(
        "ARP Rate",
        0.2,
        &["1/8", "1/4", "3/8", "1/2", "3/4", "1/1"],
        trunc_index::<5>,
    ), // 47
    // The act-ID text: the default 0.5 shows 3 Oct while the DSP's int(3v) + 1 plays 2 (a quirk, kept).
    P::menu("ARP Range", 0.5, &["1 Oct", "2 Oct", "3 Oct", "4 Oct"]), // 48
    P::menu_by(
        "ARP Mode",
        0.0,
        &["Off", "Up", "Down", "Up/Down", "Up/Down 2", "Random"],
        trunc_index::<5>,
    ), // 49
    P::toggle("Monophonic Mode", 0.0),                                // 50
    // The act-ID text: the default 0.5 shows 5 Voices while the DSP's int(7v) + 1 plays 4 (a quirk, kept).
    P::menu(
        "Number Of Voices",
        0.5,
        &[
            "1 Voice", "2 Voices", "3 Voices", "4 Voices", "5 Voices", "6 Voices", "7 Voices",
            "8 Voices",
        ],
    ), // 51
    P::plain("Distortion", 0.0), // 52
    // Gain (no name): -18 + 18·v dB, "0 dB" at the top.
    P::cont(
        "",
        1.0,
        Display::Special {
            inner: &Display::linear(-18.0, 0.0, 1, " dB"),
            at: At::Max,
            text: "0 dB",
        },
    ), // 53
    P::menu_by(
        "Delay Time",
        0.775,
        &["1/4", "3/8", "1/2", "3/4", "1/1"],
        trunc_index::<4>,
    ), // 54
    P::plain("Delay Feedback", 0.4),        // 55
    P::plain("Delay Mix", 0.0),             // 56
    P::cont("Delay Filter", 0.5, AMOUNT_1), // 57
    // ModWheel Amount and destination have no name.
    P::cont("", 0.5, AMOUNT_1), // 58
    P::menu_by(
        "",
        0.0,
        &[
            "Cutoff",
            "Resonance",
            "Osc. Sync",
            "Pulse Width",
            "Unison Dtn.",
            "Level",
            "Pan",
            "LFO Amount",
            "LFO Rate",
        ],
        trunc_index::<8>,
    ), // 59
];

// ---- 33 Slicer ----

const SLICER: &[P] = &[
    P::ENABLE,
    // The audio's f64 step.
    P::cont(
        "PITCH",
        0.5,
        Display::Stepped {
            count: 49,
            text: |v| (slicer::pitch_steps(v) as i32).to_string(),
        },
    ),
    // Hidden, no name.
    P::plain("", 0.0),
    P::plain("ATTACK", 0.0),
    P::plain("DECAY", 1.0),
    P::menu_by("SUST", 0.0, &["Off", "Hold", "To End"], trunc64_index::<2>),
    P::toggle("MONO", 0.0),
];

// ---- 39 3x Osc ----

/// "-100% … +0% … +100%".
const SIGNED_PERCENT: Display = Display::Linear {
    min: -100.0,
    max: 100.0,
    fmt: Fmt::new(0, "%").sign(Sign::Always),
};
/// "-50% … +0% … +50%".
const DETUNE: Display = Display::Linear {
    min: -50.0,
    max: 50.0,
    fmt: Fmt::new(0, "%").sign(Sign::Always),
};
/// The LFO amounts: 200·v − 100, rounded half away from zero, then `%`.
const LFO_AMOUNT: Display =
    Display::Custom(|v| format!("{}%", 200f32.mul_add(v, -100.0).round() as i32));
/// Coarse: the audio's step, round half away from zero of fma_f32(v, 48, −24).
const COARSE: Display = Display::Stepped {
    count: 49,
    text: |v| (three_osc::coarse(v) as i32).to_string(),
};
/// "0% … 100% … 200%".
const PERCENT_200: Display = Display::linear(0.0, 200.0, 0, "%");
const OSC_SHAPES: &[&str] = &[
    "Sine", "Triangle", "Square", "Saw", "Analog", "Noise", "Custom",
];
const OSC_LFO_SHAPES: &[&str] = &["Sine", "Triangle", "Square"];
/// The Arpeggio Chord list (index order).
const CHORDS: &[&str] = &[
    "Auto",
    "Auto (sustain)",
    "(none)",
    "Major",
    "sus2",
    "sus4",
    "Majb5",
    "minor",
    "mb5",
    "aug",
    "augsus4",
    "tri",
    "6",
    "6sus4",
    "6add9",
    "m6",
    "m6add9",
    "7",
    "7sus4",
    "7#5",
    "7b5",
    "7#9",
    "7b9",
    "7#5#9",
    "7#5b9",
    "7b5b9",
    "7add11",
    "7add13",
    "7#11",
    "Maj7",
    "Maj7b5",
    "Maj7#5",
    "Maj7#11",
    "Maj7add13",
    "m7",
    "m7b5",
    "m7b9",
    "m7add11",
    "m7add13",
    "m-Maj7",
    "m-Maj7add11",
    "m-Maj7add13",
    "9",
    "9sus4",
    "add9",
    "9#5",
    "9b5",
    "9#11",
    "9b13",
    "Maj9",
    "Maj9sus4",
    "Maj9#5",
    "Maj9#11",
    "m9",
    "madd9",
    "m9b5",
    "m9-Maj7",
    "11",
    "11b9",
    "Maj11",
    "m11",
    "m-Maj11",
    "13",
    "13#9",
    "13b9",
    "13b5b9",
    "Maj13",
    "m13",
    "m-Maj13",
    "Major scale",
    "Major pentatonic scale",
    "Major bebop scale",
    "Minor Harmonic scale",
    "Minor Melodic scale",
    "Minor pentatonic scale",
    "Minor Natural scale",
    "Minor Neapolitan scale",
    "Minor Hungarian scale",
    "Whole tone scale",
    "Diminished scale",
    "Dominant bebop scale",
    "Jap in sen scale",
    "Blues scale",
    "Arabic scale",
    "Enigmatic scale",
    "Neapolitan scale",
    "Dorian scale",
    "Phrygian scale",
    "Lydian scale",
    "Mixolydian scale",
    "Locrian scale",
];

const fn pct(name: &'static str, default: f32) -> P {
    P::cont(name, default, PERCENT_TIGHT)
}

/// A toggle with the options `OFF, ON`: those are its value texts.
const fn off_on(name: &'static str, default: f32) -> P {
    P::toggle(name, default).with_automation(OFF_ON)
}

// The five modulation targets come in blocks of five (LFO: Pan, Volume, ModX, ModY, Pitch) or four (envelope:
// Volume, ModX, ModY, Pitch). Reset values: osc 2/3 Coarse, osc 3 Mix and Volume env Amount pass 0.5.
const THREE_OSC: &[P] = &[
    P::ENABLE,                                                          // 0
    P::menu("Osc 1 Shape", 0.0, OSC_SHAPES),                            // 1
    P::menu("Osc 2 Shape", 0.0, OSC_SHAPES),                            // 2
    P::menu("Osc 3 Shape", 0.0, OSC_SHAPES),                            // 3
    off_on("Osc 1 Invert", 0.0),                                        // 4
    off_on("Osc 2 Invert", 0.0),                                        // 5
    off_on("Osc 3 Invert", 0.0),                                        // 6
    P::cont("Osc 1 Phase", 0.5, SIGNED_PERCENT),                        // 7
    P::cont("Osc 2 Phase", 0.5, SIGNED_PERCENT),                        // 8
    P::cont("Osc 3 Phase", 0.5, SIGNED_PERCENT),                        // 9
    P::cont("Osc 1 Detune", 0.5, DETUNE),                               // 10
    P::cont("Osc 2 Detune", 0.5, DETUNE),                               // 11
    P::cont("Osc 3 Detune", 0.5, DETUNE),                               // 12
    P::cont("Osc 1 Coarse Pitch", 0.5, COARSE),                         // 13
    P::cont("Osc 2 Coarse Pitch", 0.25, COARSE).with_reset(0.5),        // 14
    P::cont("Osc 3 Coarse Pitch", 0.0, COARSE).with_reset(0.5),         // 15
    P::cont("Osc 1 Fine Pitch", 0.5, SIGNED_PERCENT),                   // 16
    P::cont("Osc 2 Fine Pitch", 0.5, SIGNED_PERCENT),                   // 17
    P::cont("Osc 3 Fine Pitch", 0.5, SIGNED_PERCENT),                   // 18
    P::cont("Osc 1 Pan", 0.5, SIGNED_PERCENT),                          // 19
    P::cont("Osc 2 Pan", 0.5, SIGNED_PERCENT),                          // 20
    P::cont("Osc 3 Pan", 0.5, SIGNED_PERCENT),                          // 21
    pct("Osc 2 Mix", 0.5),                                              // 22
    pct("Osc 3 Mix", 0.25).with_reset(0.5),                             // 23
    off_on("Osc 3 = amplitude modulation", 0.0),                        // 24
    off_on("HQ Mode", 0.0),                                             // 25
    pct("Stereo Phase Randomness", 0.0),                                // 26
    off_on("Pan LFO Global Phase", 0.0),                                // 27
    off_on("Volume LFO Global Phase", 0.0),                             // 28
    off_on("ModX LFO Global Phase", 0.0),                               // 29
    off_on("ModY LFO Global Phase", 0.0),                               // 30
    off_on("Pitch LFO Global Phase", 0.0),                              // 31
    off_on("Pan LFO Tempo Sync", 0.0),                                  // 32
    off_on("Volume LFO Tempo Sync", 0.0),                               // 33
    off_on("ModX LFO Tempo Sync", 0.0),                                 // 34
    off_on("ModY LFO Tempo Sync", 0.0),                                 // 35
    off_on("Pitch LFO Tempo Sync", 0.0),                                // 36
    P::menu("Pan LFO Shape", 0.0, OSC_LFO_SHAPES),                      // 37
    P::menu("Volume LFO Shape", 0.0, OSC_LFO_SHAPES),                   // 38
    P::menu("ModX LFO Shape", 0.0, OSC_LFO_SHAPES),                     // 39
    P::menu("ModY LFO Shape", 0.0, OSC_LFO_SHAPES),                     // 40
    P::menu("Pitch LFO Shape", 0.0, OSC_LFO_SHAPES),                    // 41
    pct("Pan LFO Predelay", 0.0),                                       // 42
    pct("Volume LFO Predelay", 0.0),                                    // 43
    pct("ModX LFO Predelay", 0.0),                                      // 44
    pct("ModY LFO Predelay", 0.0),                                      // 45
    pct("Pitch LFO Predelay", 0.0),                                     // 46
    pct("Pan LFO Attack", 0.305),                                       // 47
    pct("Volume LFO Attack", 0.305),                                    // 48
    pct("ModX LFO Attack", 0.305),                                      // 49
    pct("ModY LFO Attack", 0.305),                                      // 50
    pct("Pitch LFO Attack", 0.305),                                     // 51
    P::cont("Pan LFO Amount", 0.5, LFO_AMOUNT),                         // 52
    P::cont("Volume LFO Amount", 0.5, LFO_AMOUNT),                      // 53
    P::cont("ModX LFO Amount", 0.5, LFO_AMOUNT),                        // 54
    P::cont("ModY LFO Amount", 0.5, LFO_AMOUNT),                        // 55
    P::cont("Pitch LFO Amount", 0.5, LFO_AMOUNT),                       // 56
    pct("Pan LFO Speed", 0.5),                                          // 57
    pct("Volume LFO Speed", 0.5),                                       // 58
    pct("ModX LFO Speed", 0.5),                                         // 59
    pct("ModY LFO Speed", 0.5),                                         // 60
    pct("Pitch LFO Speed", 0.5),                                        // 61
    off_on("Volume Envelope Enable", 0.0),                              // 62
    off_on("ModX Envelope Enable", 0.0),                                // 63
    off_on("ModY Envelope Enable", 0.0),                                // 64
    off_on("Pitch Envelope Enable", 0.0),                               // 65
    off_on("Volume Envelope Tempo Sync", 0.0),                          // 66
    off_on("ModX Envelope Tempo Sync", 0.0),                            // 67
    off_on("ModY Envelope Tempo Sync", 0.0),                            // 68
    off_on("Pitch Envelope Tempo Sync", 0.0),                           // 69
    pct("Volume Envelope Predelay", 0.0),                               // 70
    pct("ModX Envelope Predelay", 0.0),                                 // 71
    pct("ModY Envelope Predelay", 0.0),                                 // 72
    pct("Pitch Envelope Predelay", 0.0),                                // 73
    pct("Volume Envelope Attack", 0.01),                                // 74
    pct("ModX Envelope Attack", 0.01),                                  // 75
    pct("ModY Envelope Attack", 0.01),                                  // 76
    pct("Pitch Envelope Attack", 0.01),                                 // 77
    P::cont("Volume Envelope Attack Tension", 0.5, BIPOLAR_PERCENT),    // 78
    P::cont("ModX Envelope Attack Tension", 0.5, BIPOLAR_PERCENT),      // 79
    P::cont("ModY Envelope Attack Tension", 0.5, BIPOLAR_PERCENT),      // 80
    P::cont("Pitch Envelope Attack Tension", 0.5, BIPOLAR_PERCENT),     // 81
    pct("Volume Envelope Hold", 0.0),                                   // 82
    pct("ModX Envelope Hold", 0.0),                                     // 83
    pct("ModY Envelope Hold", 0.0),                                     // 84
    pct("Pitch Envelope Hold", 0.0),                                    // 85
    pct("Volume Envelope Decay", 0.01),                                 // 86
    pct("ModX Envelope Decay", 0.01),                                   // 87
    pct("ModY Envelope Decay", 0.01),                                   // 88
    pct("Pitch Envelope Decay", 0.01),                                  // 89
    P::cont("Volume Envelope Decay Tension", 0.5, BIPOLAR_PERCENT),     // 90
    P::cont("ModX Envelope Decay Tension", 0.5, BIPOLAR_PERCENT),       // 91
    P::cont("ModY Envelope Decay Tension", 0.5, BIPOLAR_PERCENT),       // 92
    P::cont("Pitch Envelope Decay Tension", 0.5, BIPOLAR_PERCENT),      // 93
    pct("Volume Envelope Sustain", 1.0),                                // 94
    pct("ModX Envelope Sustain", 1.0),                                  // 95
    pct("ModY Envelope Sustain", 1.0),                                  // 96
    pct("Pitch Envelope Sustain", 1.0),                                 // 97
    pct("Volume Envelope Release", 0.01),                               // 98
    pct("ModX Envelope Release", 0.01),                                 // 99
    pct("ModY Envelope Release", 0.01),                                 // 100
    pct("Pitch Envelope Release", 0.01),                                // 101
    P::cont("Volume Envelope Release Tension", 0.105, BIPOLAR_PERCENT), // 102
    P::cont("ModX Envelope Release Tension", 0.5, BIPOLAR_PERCENT),     // 103
    P::cont("ModY Envelope Release Tension", 0.5, BIPOLAR_PERCENT),     // 104
    P::cont("Pitch Envelope Release Tension", 0.5, BIPOLAR_PERCENT),    // 105
    pct("Volume Envelope Amount", 1.0).with_reset(0.5),                 // 106
    pct("ModX Envelope Amount", 0.5),                                   // 107
    pct("ModY Envelope Amount", 0.5),                                   // 108
    pct("Pitch Envelope Amount", 0.5),                                  // 109
    P::menu(
        "Filter Type",
        0.0,
        &[
            "Fast LP", "LP", "BP", "HP", "BS", "LPx2", "SVF LP", "SVF LPx2",
        ],
    ), // 110
    pct("Filter Cutoff", 1.0),                                          // 111
    pct("Filter Resonance", 0.0),                                       // 112
    P::cont("Pan Offset", 0.5, SIGNED_PERCENT),                         // 113
    P::cont("Volume Multiplier", 0.5, PERCENT_200),                     // 114
    P::cont("Filter Cutoff Offset", 0.5, SIGNED_PERCENT),               // 115
    P::cont("Filter Resonance Offset", 0.5, SIGNED_PERCENT),            // 116
    // 100 steps: round(99v), "---" (unlimited) at 0, then 1-99, as the engine counts voices.
    P::cont(
        "Maximum Polyphony",
        0.0,
        Display::Stepped {
            count: 100,
            text: |v| match three_osc::max_voices(v) {
                three_osc::VOICE_CAP => "---".to_owned(),
                n => n.to_string(),
            },
        },
    ), // 117
    pct("Portamento Time", 0.494),  // 118
    off_on("Portamento Mode", 0.0), // 119
    off_on("Monophonic Mode", 0.0), // 120
    // C0 … C10 in semitones (121 steps, C0 and C10 at the ends, C5 in the middle).
    P::cont(
        "Key Tracking Middle Value",
        0.5,
        Display::Note { count: 121 },
    ), // 121
    P::cont("Key Tracking To Pan", 0.5, BIPOLAR_PERCENT), // 122
    P::cont("Key Tracking To Cutoff", 0.5, BIPOLAR_PERCENT), // 123
    P::cont("Key Tracking To Resonance", 0.5, BIPOLAR_PERCENT), // 124
    pct("Velocity Tracking Middle Value", 0.833),         // 125
    P::cont("Velocity Tracking To Pan", 0.5, BIPOLAR_PERCENT), // 126
    P::cont("Velocity Tracking To Cutoff", 0.5, BIPOLAR_PERCENT), // 127
    P::cont("Velocity Tracking To Resonance", 0.5, BIPOLAR_PERCENT), // 128
    // A percent, "OFF" at 100%.
    P::cont(
        "Gate",
        1.0,
        Display::Special {
            inner: &PERCENT_TIGHT,
            at: At::Max,
            text: "OFF",
        },
    ), // 129
    P::cont(
        "Delay Echoes",
        3.0 / 9.0,
        Display::Steps {
            min: 1,
            count: 10,
            fmt: Fmt::new(0, ""),
        },
    ), // 130
    P::cont("Delay Feedback", 0.0, PERCENT_200), // 131
    pct("Delay Time", 0.375),                    // 132
    P::cont("Delay Pan", 0.5, BIPOLAR_PERCENT),  // 133
    P::cont(
        "Delay Pitch",
        0.5,
        Display::Linear {
            min: -12.0,
            max: 12.0,
            fmt: Fmt::new(2, "").sign(Sign::Always),
        },
    ), // 134
    P::cont("Delay Cutoff", 0.5, BIPOLAR_PERCENT), // 135
    P::cont("Delay Resonance", 0.5, BIPOLAR_PERCENT), // 136
    off_on("Panning will bounce", 0.0),          // 137
    off_on("Fat (Unison) Mode", 0.0),            // 138
    P::menu(
        "Arpeggio Direction",
        0.0,
        &["Off", "Up", "Down", "Bounce", "Sticky", "Random"],
    ), // 139
    // A percent, "Hold" at 100%.
    P::cont(
        "Arpeggio Time",
        0.576,
        Display::Special {
            inner: &PERCENT_TIGHT,
            at: At::Max,
            text: "Hold",
        },
    ), // 140
    pct("Arpeggio Gate", 1.0), // 141
    P::menu(
        "Arpeggio Range",
        1.0 / 6.0,
        &["---", "1", "2", "3", "4", "5", "6"],
    ), // 142
    P::menu("Arpeggio Repeat", 0.0, &["---", "2", "3", "4", "5", "6"]), // 143
    off_on("Arpeggio Slide", 0.0), // 144
    // Menu index = the arp's Chord value + 2; the default 1/90 is Auto (sustain).
    P::menu_by("Arpeggio Chord", 1.0 / 90.0, CHORDS, |v| {
        (chord_value(v) + 2) as usize
    }), // 145
    off_on("Osc 1 Free Run", 0.0), // 146
    off_on("Osc 2 Free Run", 0.0), // 147
    off_on("Osc 3 Free Run", 0.0), // 148
];

// ---- 52 SoundFont Player ----

/// An envelope time: t = max(0.005, 10·(0.95v + 0.05)³) s, "%.2f s" from one second up, else round(1000t) "ms".
/// In f32.
fn soundfont_time(v: f32) -> String {
    let t = (10.0 * 0.95f32.mul_add(v, 0.05).powi(3)).max(0.005);
    if t >= 1.0 {
        format!("{t:.2} s")
    } else {
        format!("{} ms", (1000.0 * t).round())
    }
}

const SOUNDFONT_TIME: Display = Display::Custom(soundfont_time);

// The UI binds indices 1-5. The Envelope toggle is off (Original: the SoundFont's own envelopes), and reads
// Original up to 0.5, Custom above.
const SOUNDFONT_PLAYER: &[P] = &[
    P::ENABLE,
    P::cont("Attack", 0.0, SOUNDFONT_TIME),
    P::cont("Decay", 0.0, SOUNDFONT_TIME),
    P::cont(
        "Sustain",
        1.0,
        Display::Custom(|v| format!("{} %", (100.0 * v).round())),
    ),
    P::cont("Release", 0.0, SOUNDFONT_TIME),
    P::toggle("Envelope", 0.0).with_automation(ParamKind::MenuBy {
        items: &["Original", "Custom"],
        index: |v| usize::from(v > 0.5),
    }),
];

pub static MODULES: &[ModuleSpec] = &[
    ModuleSpec::instrument(
        0,
        "MiniSynth",
        "MiniSynth",
        MINISYNTH,
        &["OSC", "FILTER", "FLT EG", "LFO", "MASTER", "FX"],
    ),
    ModuleSpec::instrument(
        1,
        "DW Sampler",
        "DW Sampler",
        DW_SAMPLER,
        &["SAMPLE", "FILTER", "FLT ENV", "FRQ ENV", "LFO"],
    ),
    ModuleSpec::instrument(
        2,
        "GMSynth",
        "GMS",
        GMSYNTH,
        &["FILTER", "OSC", "MOD", "LEVEL", "EG", "LFO", "VOICES", "FX"],
    ),
    ModuleSpec::instrument(
        19,
        "Transistor Bass",
        "Transistor Bass",
        TRANSISTOR_BASS,
        &["MAIN", "DISTORTION", "PULSE WIDTH"],
    ),
    ModuleSpec::instrument(
        23,
        "SuperSaw",
        "SuperSaw",
        SUPERSAW,
        &["OSC", "FILTER", "LFO", "LEVEL", "MOD", "VOICES", "FX"],
    ),
    ModuleSpec::instrument(33, "Slicer", "Slicer", SLICER, &[]),
    ModuleSpec::instrument(
        39,
        "3x Osc",
        "3x Osc",
        THREE_OSC,
        &["OSC", "PAN", "VOL", "CUT", "RES", "PITCH", "FILT", "MISC"],
    ),
    ModuleSpec::instrument(
        52,
        "SoundFont Player",
        "SoundFont Player",
        SOUNDFONT_PLAYER,
        &[],
    ),
];
