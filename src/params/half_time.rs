//! Half Time ([`honeydew_audio::half_time`]), Honeydew's own effect (GitHub #91):
//! its params and value texts, in the engine's order and laws.

use honeydew_audio::half_time::{self as h, DEFAULTS, LENGTH, LENGTHS, MIX, PARAMS};

use super::{Display, ModuleSpec, ParamSpec as P};

const LENGTH_NAMES: &[&str] = &{
    let mut names = [""; LENGTHS.len()];
    let mut i = 0;
    while i < names.len() {
        names[i] = LENGTHS[i].1;
        i += 1;
    }
    names
};

static PARAM_SPECS: [P; PARAMS] = {
    let mut p = [P::ENABLE; PARAMS];
    p[LENGTH] = P::menu("Length", DEFAULTS[LENGTH], LENGTH_NAMES);
    p[MIX] = P::cont("Mix", DEFAULTS[MIX], Display::linear(0.0, 100.0, 0, " %"));
    p
};

/// The module. Not in [`super::modules`]: that is FL Studio Mobile's effects.
pub static MODULE: ModuleSpec = ModuleSpec::effect(
    h::HALF_TIME,
    "Half Time",
    "Half Time",
    false,
    &PARAM_SPECS,
    &[],
);
