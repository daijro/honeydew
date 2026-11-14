//! Half Time ([`honeydew_audio::half_time`]), Honeydew's own effect (GitHub #91):
//! its params and value texts, in the engine's order and laws.

use honeydew_audio::half_time::{
    self as h, DEFAULTS, LENGTH, LENGTHS, MIX, OLD_LENGTH, PARAMS, SMOOTH, SPEED, SPEEDS,
};

use super::{Display, ModuleSpec, ParamSpec as P};

/// The names of a `(value, name)` table.
const fn names<const N: usize>(table: [(f64, &'static str); N]) -> [&'static str; N] {
    let mut names = [""; N];
    let mut i = 0;
    while i < N {
        names[i] = table[i].1;
        i += 1;
    }
    names
}

const LENGTH_NAMES: &[&str] = &names(LENGTHS);
const SPEED_NAMES: &[&str] = &names(SPEEDS);

static PARAM_SPECS: [P; PARAMS] = {
    let mut p = [P::ENABLE; PARAMS];
    // Length before 0.5.4, which only an older save's upgrade reads (`flm::read`): no name, so no control or automation.
    p[OLD_LENGTH] = P::menu(
        "",
        DEFAULTS[OLD_LENGTH],
        &["1/4 bar", "1/2 bar", "1 bar", "2 bars"],
    );
    p[MIX] = P::cont("Mix", DEFAULTS[MIX], Display::linear(0.0, 100.0, 0, " %"));
    // Honeydew additions (GitHub #122).
    p[LENGTH] = P::menu("Length", DEFAULTS[LENGTH], LENGTH_NAMES);
    p[SPEED] = P::menu("Speed", DEFAULTS[SPEED], SPEED_NAMES);
    p[SMOOTH] = P::cont(
        "Smooth",
        DEFAULTS[SMOOTH],
        Display::linear(0.0, 50.0, 0, " ms"),
    );
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
