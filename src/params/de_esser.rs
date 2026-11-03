//! De-esser ([`honeydew_audio::de_esser`]), Honeydew's own effect (GitHub #43):
//! its params and value texts, in the engine's order and laws.

use honeydew_audio::de_esser::{
    self as d, DEFAULTS, FREQUENCY, LISTEN, MODE, PARAMS, RANGE, THRESHOLD,
};

use super::{Display, ModuleSpec, ParamSpec as P};

/// The Mode menu, in the engine's order.
const MODES: &[&str] = &["Split", "Wide"];

static PARAM_SPECS: [P; PARAMS] = {
    let mut p = [P::ENABLE; PARAMS];
    p[FREQUENCY] = P::cont(
        "Frequency",
        DEFAULTS[FREQUENCY],
        Display::Freq {
            min: 2000.0,
            max: 12_000.0,
            hz_decimals: 0,
            khz_decimals: 1,
        },
    );
    p[THRESHOLD] = P::cont(
        "Threshold",
        DEFAULTS[THRESHOLD],
        Display::linear(-60.0, 0.0, 1, " dB"),
    );
    p[RANGE] = P::cont(
        "Range",
        DEFAULTS[RANGE],
        Display::linear(0.0, 24.0, 1, " dB"),
    );
    p[MODE] = P::menu("Mode", DEFAULTS[MODE], MODES);
    p[LISTEN] = P::toggle("Listen", DEFAULTS[LISTEN]);
    p
};

/// The module. Not in [`super::modules`]: that is the song format's effects.
pub static MODULE: ModuleSpec = ModuleSpec::effect(
    d::DE_ESSER,
    "De-esser",
    "De-esser",
    false,
    &PARAM_SPECS,
    &[],
);
