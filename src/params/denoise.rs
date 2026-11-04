//! Denoise ([`honeydew_audio::denoise`]), Honeydew's own effect (GitHub #43): its
//! params and value texts, in the engine's order and laws.

use honeydew_audio::denoise::{self as d, DEFAULTS, LEARN, PARAMS, REDUCTION, SENSITIVITY};

use super::{Display, ModuleSpec, ParamSpec as P};

static PARAM_SPECS: [P; PARAMS] = {
    let mut p = [P::ENABLE; PARAMS];
    p[REDUCTION] = P::cont(
        "Reduction",
        DEFAULTS[REDUCTION],
        Display::linear(0.0, 40.0, 1, " dB"),
    );
    p[SENSITIVITY] = P::cont(
        "Sensitivity",
        DEFAULTS[SENSITIVITY],
        Display::linear(0.0, 100.0, 0, " %"),
    );
    p[LEARN] = P::toggle("Learn", DEFAULTS[LEARN]);
    p
};

/// The module. Not in [`super::modules`]: that is the song format's effects.
pub static MODULE: ModuleSpec =
    ModuleSpec::effect(d::DENOISE, "Denoise", "Denoise", false, &PARAM_SPECS, &[]);
