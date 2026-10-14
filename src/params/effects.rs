//! Parameter tables of the audio effects, IDs 3-20.
//!
//! `params[i]` is param number `i`, not the order of the UI page. Automation names are as written ("" for none).
//! Where a param shows a unit on its default value (`60 %`, `50%`) but has a generic 0…100 (v×100) range, the unit
//! comes from the default: those percents round, where the generic law truncates. For modules with ten or more
//! params every line ends in its index.

use super::display::trunc_index;
use super::{At, Display, ModuleSpec, ParamKind, ParamSpec as P};

// ---- shared laws and lists ----

pub(super) const DB_12: Display = Display::linear(-12.0, 12.0, 1, " dB");
/// 0..100 with a spaced percent sign ("60 %"): `round(v·100)`, ties away from zero.
pub(super) const PERCENT: Display = Display::GenericUnit(" %");
/// 0..100 with a tight percent sign ("40%"), rounded the same way.
pub(super) const PERCENT_TIGHT: Display = Display::GenericUnit("%");
pub(super) const BIPOLAR_PERCENT: Display = Display::linear(-100.0, 100.0, 0, "%");
/// trunc(200·v − 100) in f32, `%d`: Distortion Stereo, the GMSynth amounts.
pub(super) const AMOUNT_TRUNC: Display =
    Display::Custom(|v| (200f32.mul_add(v, -100.0) as i32).to_string());
/// The automation texts of a toggle with the options `OFF, ON` (the control itself reads Off / On).
pub(super) const OFF_ON: ParamKind = ParamKind::Menu {
    items: &["OFF", "ON"],
};

/// Item 1 at any value but 0: a two-state switch the engine applies there (Flanger and Phaser Sync, Comb Snap;
/// IDs 13, 14, 40), as the panels show it.
pub(super) fn on_above_0(v: f32) -> usize {
    usize::from(v > 0.0)
}

/// The lane texts of a switch on at any value but 0 ([`on_above_0`]).
pub(super) const OFF_ON_ABOVE_0: ParamKind = ParamKind::MenuBy {
    items: &["Off", "On"],
    index: on_above_0,
};
/// Free LFO speed of Flanger and Phaser: 0.099 + 9.9·v³ Hz.
const LFO_SPEED: Display = Display::power(0.099, 0.099 + 9.9, 3, 2, " Hz");

const LFO_SHAPE: &[&str] = &["Sine", "Saw", "Sqr"];
const SYNCED_RATES: &[&str] = &[
    "1/4", "3/8", "1/2", "3/4", "1/1", "2/1", "4/1", "8/1", "1 Bar", "2 Bars", "4 Bars", "8 Bars",
    "16 Bars",
];

// ---- 3 Equalizer ----

/// 32·608.8^v Hz, "115.4 Hz" below 1 kHz and "1.50 kHz" above.
const EQ_FREQ: Display = Display::Freq {
    min: 32.0,
    max: 32.0 * 608.8,
    hz_decimals: 1,
    khz_decimals: 2,
};
const EQ_KIND: &[&str] = &["LowShelf", "Peak", "HiShelf"];
const EQ_CHANNEL: &[&str] = &["Both", "Left", "Right", "Opp. L", "Opp. R"];

/// Kind and Channel: the panel shows the nearest item,
/// round-half-away(f32(2v)) and (f32(4v)), while the DSP truncates; the param text is the generic v×100.
const fn eq_menu(name: &'static str, default: f32, items: &'static [&'static str]) -> P {
    P::menu(name, default, items).with_automation(ParamKind::Continuous {
        display: Display::Generic,
    })
}

const EQUALIZER: &[P] = &[
    P::ENABLE,                             // 0
    P::cont("Gain 1", 0.5, DB_12),         // 1
    P::cont("Frequency 1", 0.2, EQ_FREQ),  // 2
    P::plain("Bandwidth 1", 0.5),          // 3
    eq_menu("Kind 1", 0.51, EQ_KIND),      // 4
    P::cont("Gain 2", 0.5, DB_12),         // 5
    P::cont("Frequency 2", 0.4, EQ_FREQ),  // 6
    P::plain("Bandwidth 2", 0.5),          // 7
    eq_menu("Kind 2", 0.51, EQ_KIND),      // 8
    P::cont("Gain 3", 0.5, DB_12),         // 9
    P::cont("Frequency 3", 0.6, EQ_FREQ),  // 10
    P::plain("Bandwidth 3", 0.5),          // 11
    eq_menu("Kind 3", 0.51, EQ_KIND),      // 12
    P::cont("Gain 4", 0.5, DB_12),         // 13
    P::cont("Frequency 4", 0.8, EQ_FREQ),  // 14
    P::plain("Bandwidth 4", 0.5),          // 15
    eq_menu("Kind 4", 0.51, EQ_KIND),      // 16
    eq_menu("Channel 1", 0.0, EQ_CHANNEL), // 17
    eq_menu("Channel 2", 0.0, EQ_CHANNEL), // 18
    eq_menu("Channel 3", 0.0, EQ_CHANNEL), // 19
    eq_menu("Channel 4", 0.0, EQ_CHANNEL), // 20
];

// ---- 4 MultiFX ----

const FX_KINDS: &[&str] = &[
    "Delay",
    "Reverb",
    "LP Filter",
    "HP Filter",
    "Flanger",
    "Phaser",
    "Panner",
    "Vox",
];

const MULTI_FX: &[P] = &[
    P::ENABLE,
    P::menu_by("FX Kind", 0.0, FX_KINDS, trunc_index::<7>),
    P::plain("Axis X", 0.5),
    P::plain("Axis Y", 0.5),
    P::menu_by(
        "LFO Rate",
        0.61,
        &[
            "1/8", "1/4", "3/8", "1/2", "3/4", "beat", "2 beats", "bar", "2 bars", "4 bars",
        ],
        trunc_index::<9>,
    ),
    P::plain("LFO Amount", 0.0),
    P::menu_by(
        "LFO Shape",
        0.0,
        &["Sine", "Saw", "Square"],
        trunc_index::<2>,
    ),
];

// ---- 5 Limiter (legacy) ----

const LIMITER_LEGACY: &[P] = &[
    P::ENABLE,
    P::cont("Threshold", 1.0, Display::linear(-24.0, 0.0, 1, " dB")),
    // Truncated in f64, so the default 0.7 reads 69% (70 in f32).
    P::cont(
        "Release Time",
        0.7,
        Display::Trunc64 {
            min: 0.0,
            max: 100.0,
            unit: "%",
        },
    ),
    P::cont(
        "Output Gain",
        0.6667,
        Display::linear(-24.0, 12.0, 1, " dB"),
    ),
];

// ---- 6 Distortion ----

const DISTORTION: &[P] = &[
    P::ENABLE,
    P::plain("Drive", 0.25),
    P::plain("BitCrush", 0.0),
    P::cont("Gain", 1.0, Display::linear(-24.0, 0.0, 1, " dB")),
    // Stereo (p4) uses the same law in single precision.
    P::cont("Stereo", 0.5, AMOUNT_TRUNC),
    P::menu_by(
        "Kind",
        0.0,
        &["Hard", "Tube", "Soft", "Clip"],
        trunc_index::<3>,
    ),
    // trunc(200·v − 100) in double precision, `%d` ("0" for 0.495 < v < 0.505).
    P::cont(
        "Bias",
        0.5,
        Display::Trunc64 {
            min: -100.0,
            max: 100.0,
            unit: "",
        },
    ),
];

// ---- 7 Graphic EQ ----

const GRAPHIC_EQ: &[P] = &[
    P::ENABLE,
    P::cont("80 Hz", 0.5, DB_12),
    P::cont("160 Hz", 0.5, DB_12),
    P::cont("375 Hz", 0.5, DB_12),
    P::cont("750 Hz", 0.5, DB_12),
    P::cont("1.5 kHz", 0.5, DB_12),
    P::cont("3 kHz", 0.5, DB_12),
    P::cont("6 kHz", 0.5, DB_12),
    P::cont("12 kHz", 0.5, DB_12),
];

// ---- 8 Compressor ----

/// Attack and Hold: 50·v² ms (the Multiband Compressor's attacks too).
pub(super) const COMP_ATTACK: Display = Display::power(0.0, 50.0, 2, 1, " ms");
/// 1 + 19·v², "+inf" (limiting) at v = 1.
const COMP_RATIO: Display = Display::Special {
    inner: &Display::power(1.0, 20.0, 2, 1, ":1"),
    at: At::Max,
    text: "+inf",
};

const COMPRESSOR: &[P] = &[
    P::ENABLE,                                                          // 0
    P::cont("Thresh", 0.499, Display::linear(-40.0, 0.0, 1, " dB")),    // 1
    P::cont("Ratio", 0.0, COMP_RATIO),                                  // 2
    P::cont("Attack", 0.6, COMP_ATTACK),                                // 3
    P::cont("Release", 0.6, Display::power(10.0, 500.0, 2, 1, " ms")),  // 4
    P::cont("Level", 0.5001, Display::linear(-20.0, 20.0, 1, " dB")),   // 5
    P::cont("Hold", 0.0, COMP_ATTACK),                                  // 6
    P::toggle("Auto gain", 1.0),                                        // 7
    P::cont("Input gain", 0.5, Display::linear(-20.0, 20.0, 1, " dB")), // 8
    // No UI; its text is "Off".
    P::toggle("RMS", 0.0), // 9
];

// ---- 9 Filter ----

// The menus hold the labels of the UI lists; the value texts are the short ones (LP/BP/HP, "48 dB/O").
const FILTER: &[P] = &[
    P::ENABLE,
    // 16·1260.7^v Hz, always in Hz ("20171.1 Hz").
    P::cont("Frequency", 0.7, Display::exp(16.0, 20171.1, 1, " Hz")),
    P::plain("Resonance", 0.0),
    P::menu_by(
        "Kind",
        0.0,
        &["Low Pass", "Band Pass", "High Pass"],
        trunc_index::<2>,
    )
    .with_automation(ParamKind::Menu {
        items: &["LP", "BP", "HP"],
    }),
    P::menu_by(
        "Slope",
        0.5,
        &["24 dB/Oct", "48 dB/Oct", "96 dB/Oct"],
        trunc_index::<2>,
    )
    .with_automation(ParamKind::Menu {
        items: &["24 dB/O", "48 dB/O", "96 dB/O"],
    }),
];

// ---- 10 Autoduck (legacy) ----

const AUTODUCK_LEGACY: &[P] = &[
    P::ENABLE,
    P::plain("Ducking", 0.501),
    P::plain("Attack", 0.1),
    P::plain("Curve", 0.501),
];

// ---- 11 Chorus ----

const CHORUS: &[P] = &[
    P::ENABLE,
    P::cont("Rate", 0.4, Display::power(0.2, 4.2, 2, 2, " Hz")),
    P::cont("Depth", 0.6, PERCENT),
    P::cont("Stereo", 1.0, PERCENT),
    P::cont("Mix", 0.6, PERCENT),
];

// ---- 12 Reverb (legacy) ----

const REVERB_LEGACY: &[P] = &[
    P::ENABLE,                     // 0
    P::plain("DECAY", 0.4),        // 1
    P::plain("HIGH DAMP", 0.8),    // 2
    P::plain("MIX", 0.4),          // 3
    P::plain("PREDELAY", 0.0),     // 4
    P::plain("ROOM SIZE", 1.0),    // 5
    P::plain("DIFFUSION", 1.0),    // 6
    P::plain("WIDTH", 1.0),        // 7
    P::plain("LO-CUT", 0.0),       // 8
    P::plain("HI-CUT", 1.0),       // 9
    P::plain("INPUT", 1.0),        // 10
    P::plain("EARLY REFL.", 0.25), // 11
    P::plain("DRY LEVEL", 0.0),    // 12
    // 13-16: hidden.
    P::plain("", 0.0),            // 13
    P::plain("", 0.0),            // 14
    P::plain("", 0.0),            // 15
    P::plain("", 0.0),            // 16
    P::plain("BASS MULT.", 0.0),  // 17
    P::plain("CRSV", 0.0),        // 18
    P::plain("MOD SPEED", 0.499), // 19 ("49": the generic law truncates)
    P::plain("MOD AMOUNT", 0.0),  // 20
    // 21: hidden.
    P::plain("", 0.0), // 21
];

// ---- 13 Flanger ----

const FLANGER: &[P] = &[
    P::ENABLE, // 0
    // The act-ID text: the default 0.4 shows 2/1 while the DSP's trunc(12v) runs 1/1 (a quirk, kept).
    P::menu("LFO Rate", 0.4, SYNCED_RATES), // 1
    P::toggle("Sync", 0.0).with_automation(OFF_ON_ABOVE_0), // 2
    P::cont("LFO Speed", 0.55, LFO_SPEED),  // 3
    P::menu("LFO Shape", 0.0, LFO_SHAPE),   // 4
    P::cont("LFO Amount", 1.0, PERCENT),    // 5
    // 2.5·v² ms.
    P::cont("Delay", 0.25, Display::power(0.0, 2.5, 2, 2, " ms")), // 6
    P::cont("Feedback", 0.75, PERCENT),                            // 7
    P::toggle("Inv. FDBK", 1.0),                                   // 8
    P::cont("Stereo", 0.0, PERCENT),                               // 9
    P::cont("Mix", 1.0, PERCENT),                                  // 10
    P::toggle("Inv. Mix", 1.0),                                    // 11
];

// ---- 14 Phaser ----

// The LFO Rate list has 13 items, as the preset values need.
const PHASER: &[P] = &[
    P::ENABLE,                                                       // 0
    P::menu_by("LFO Rate", 0.1122, SYNCED_RATES, trunc_index::<12>), // 1
    P::cont("LFO Amount", 0.25, PERCENT),                            // 2
    P::cont("Feedback", 0.1, PERCENT),                               // 3
    // 50 + 15950·v² Hz (≈ 16000·v² would miss the default of 1047 Hz).
    P::cont(
        "Frequency",
        0.25,
        Display::power(50.0, 16000.0, 2, 0, " Hz"),
    ), // 4
    P::cont("Mix", 1.0, PERCENT), // 5
    // The Sync toggle has no automation name.
    P::toggle("", 1.0).with_automation(OFF_ON_ABOVE_0), // 6
    P::cont("LFO Speed", 0.0, LFO_SPEED),               // 7
    P::menu("LFO Shape", 0.0, LFO_SHAPE),               // 8
    P::cont("Stereo", 1.0, PERCENT),                    // 9
];

// ---- 15 Stereoizer ----

const STEREOIZER: &[P] = &[
    P::ENABLE,
    P::plain("Strength", 0.7),
    P::plain("Timbre", 0.52),
    P::cont("Focus", 0.75, BIPOLAR_PERCENT),
    P::cont("L/R Delay", 0.5, BIPOLAR_PERCENT),
    P::cont("Pan", 0.5, BIPOLAR_PERCENT),
    // Hidden.
    P::plain("", 0.0),
];

// ---- 16 Trance Delay ----

const TRANCE_TIMES: &[&str] = &["1/4", "3/8", "1/2", "3/4", "1/1"];

const TRANCE_DELAY: &[P] = &[
    P::ENABLE,
    P::menu_by("Time", 0.775, TRANCE_TIMES, trunc_index::<4>),
    P::plain("Fdbk", 0.4),
    P::plain("Mix", 1.0),
    P::plain("Filter", 0.5),
    // Mode and Input Pan have no automation name.
    P::menu("", 0.0, &["Stereo", "Ping-Pong", "Mirror"]),
    P::plain("", 0.5),
];

// ---- 18 Tape Delay ----

const TAPE_DELAY: &[P] = &[
    P::ENABLE,
    // 1000·v² ms.
    P::cont("Time", 0.6, Display::power(0.0, 1000.0, 2, 0, " ms")),
    P::cont("Fdbk", 0.6, PERCENT_TIGHT),
    P::cont("Async.", 0.5, BIPOLAR_PERCENT),
    P::cont("Mix", 0.4, PERCENT_TIGHT),
];

// ---- 20 Leveller ----

const LEVELLER: &[P] = &[
    P::ENABLE,
    P::cont("Gain", 0.738, Display::GainCurve),
    P::cont(
        "Pan",
        0.5,
        Display::Pan {
            decimals: 1,
            unit: "%",
            centre: "mid",
            sides: ["L", "R"],
        },
    ),
];

pub static MODULES: &[ModuleSpec] = &[
    ModuleSpec::effect(
        3,
        "Equalizer",
        "Param EQ",
        false,
        EQUALIZER,
        &["BAND 1", "BAND 2", "BAND 3", "BAND 4"],
    ),
    ModuleSpec::effect(4, "MultiFX", "MultiFX", false, MULTI_FX, FX_KINDS),
    ModuleSpec::effect(5, "Limiter", "Limiter (Legacy)", true, LIMITER_LEGACY, &[]),
    ModuleSpec::effect(6, "Distortion", "Distortion", false, DISTORTION, &[]),
    ModuleSpec::effect(7, "Graphic EQ", "Graphic EQ", false, GRAPHIC_EQ, &[]),
    ModuleSpec::effect(8, "Compressor", "Compressor", false, COMPRESSOR, &[]),
    ModuleSpec::effect(9, "Filter", "Filter", false, FILTER, &[]),
    ModuleSpec::effect(
        10,
        "Autoduck",
        "Trancer (legacy)",
        true,
        AUTODUCK_LEGACY,
        &[],
    ),
    ModuleSpec::effect(11, "Chorus", "Chorus", false, CHORUS, &[]),
    ModuleSpec::effect(
        12,
        "Reverb",
        "Reverb (legacy)",
        true,
        REVERB_LEGACY,
        &["MAIN", "TUNE", "MIXER"],
    ),
    ModuleSpec::effect(13, "Flanger", "Flanger", false, FLANGER, &[]),
    ModuleSpec::effect(14, "Phaser", "Phaser", false, PHASER, &[]),
    ModuleSpec::effect(15, "Stereoizer", "Stereorizer", false, STEREOIZER, &[]),
    ModuleSpec::effect(
        16,
        "Trance Delay",
        "Trance Delay",
        false,
        TRANCE_DELAY,
        TRANCE_TIMES,
    ),
    ModuleSpec::effect(18, "Tape Delay", "Tape Delay", false, TAPE_DELAY, &[]),
    ModuleSpec::effect(20, "Leveller", "Leveller", false, LEVELLER, &[]),
];
