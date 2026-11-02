//! Modulator ([`honeydew_audio::modulator`]), Honeydew's own effect: its params and value texts, in the engine's order
//! and laws.

use honeydew_audio::modulator::{self as m, DEFAULTS, DIVISIONS, PARAMS};

use super::{Display, Fmt, ModuleSpec, ParamSpec as P, Sign};

/// The Source menu, in the engine's order ([`m::Source`]).
const SOURCES: &[&str] = &["Envelope", "LFO", "Random"];
const SHAPES: &[&str] = &["Sine", "Triangle", "Saw", "Square"];
const DIVISION_NAMES: &[&str] = &{
    let mut names = [""; DIVISIONS.len()];
    let mut i = 0;
    while i < names.len() {
        names[i] = DIVISIONS[i].1;
        i += 1;
    }
    names
};

/// `ms` as milliseconds below a second, else seconds.
fn time(ms: f32) -> String {
    if ms < 999.5 {
        format!("{ms:.0} ms")
    } else {
        format!("{:.2} s", ms / 1000.0)
    }
}

static PARAM_SPECS: [P; PARAMS] = {
    let mut p = [P::ENABLE; PARAMS];
    p[m::SOURCE] = P::menu("Source", DEFAULTS[m::SOURCE], SOURCES);
    p[m::AMOUNT] = P::cont(
        "Amount",
        DEFAULTS[m::AMOUNT],
        Display::Linear {
            min: -100.0,
            max: 100.0,
            fmt: Fmt::new(0, " %").sign(Sign::Positive),
        },
    )
    .with_reset(0.5);
    p[m::GAIN] = P::cont(
        "Gain",
        DEFAULTS[m::GAIN],
        Display::Linear {
            min: -24.0,
            max: 24.0,
            fmt: Fmt::new(1, " dB").sign(Sign::Positive),
        },
    );
    p[m::ATTACK] = P::cont(
        "Attack",
        DEFAULTS[m::ATTACK],
        Display::Custom(|v| time(1000f32.powf(v))),
    );
    p[m::RELEASE] = P::cont(
        "Release",
        DEFAULTS[m::RELEASE],
        Display::Custom(|v| time(10.0 * 1000f32.powf(v))),
    );
    p[m::SYNC] = P::toggle("Sync", DEFAULTS[m::SYNC]);
    p[m::RATE] = P::cont(
        "Rate",
        DEFAULTS[m::RATE],
        Display::exp(0.05, 20.0, 2, " Hz"),
    );
    p[m::DIVISION] = P::menu("Rate Sync", DEFAULTS[m::DIVISION], DIVISION_NAMES);
    p[m::SHAPE] = P::menu("Shape", DEFAULTS[m::SHAPE], SHAPES);
    p[m::SMOOTH] = P::cont(
        "Smooth",
        DEFAULTS[m::SMOOTH],
        Display::linear(0.0, 100.0, 0, " %"),
    );
    p
};

/// The module. Not in [`super::modules`]: that is FL Studio Mobile's effects.
pub static MODULE: ModuleSpec = ModuleSpec::effect(
    m::MODULATOR,
    "Modulator",
    "Modulator",
    false,
    &PARAM_SPECS,
    &[],
);
