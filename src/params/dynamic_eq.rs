//! Dynamic EQ ([`honeydew_audio::dynamic_eq`]), Honeydew's own effect: its params and value texts, in the engine's
//! order and laws.

use honeydew_audio::dynamic_eq::{
    self as d, ATTACK, BANDS, DEFAULTS, FREQUENCY, GAIN, ON, PARAMS, PER_BAND, Q, RANGE, RELEASE,
    SHAPE, THRESHOLD, band,
};

use super::{Display, Fmt, ModuleSpec, ParamSpec as P, Sign};

/// The Shape menu, in the engine's order ([`d::Shape`]).
const SHAPES: &[&str] = &["Bell", "Low Shelf", "High Shelf"];

const DB: Fmt = Fmt::new(1, " dB");

/// Band `$n`'s param names, in param order.
macro_rules! names {
    ($n:literal) => {
        [
            concat!("Band ", $n, " On"),
            concat!("Band ", $n, " Shape"),
            concat!("Band ", $n, " Frequency"),
            concat!("Band ", $n, " Q"),
            concat!("Band ", $n, " Gain"),
            concat!("Band ", $n, " Threshold"),
            concat!("Band ", $n, " Range"),
        ]
    };
}
const NAMES: [[&str; PER_BAND]; BANDS] = [names!(1), names!(2), names!(3), names!(4)];

static PARAM_SPECS: [P; PARAMS] = {
    let signed = Display::Linear {
        min: -18.0,
        max: 18.0,
        fmt: DB.sign(Sign::Positive),
    };
    let mut p = [P::ENABLE; PARAMS];
    let mut b = 0;
    while b < BANDS {
        let (n, i) = (NAMES[b], band(b, 0));
        p[i + ON] = P::toggle(n[ON], DEFAULTS[i + ON]);
        p[i + SHAPE] = P::menu(n[SHAPE], DEFAULTS[i + SHAPE], SHAPES);
        p[i + FREQUENCY] = P::cont(
            n[FREQUENCY],
            DEFAULTS[i + FREQUENCY],
            Display::Freq {
                min: 20.0,
                max: 20_000.0,
                hz_decimals: 0,
                khz_decimals: 1,
            },
        );
        p[i + Q] = P::cont(n[Q], DEFAULTS[i + Q], Display::exp(0.1, 10.0, 2, ""));
        p[i + GAIN] = P::cont(n[GAIN], DEFAULTS[i + GAIN], signed);
        p[i + THRESHOLD] = P::cont(
            n[THRESHOLD],
            DEFAULTS[i + THRESHOLD],
            Display::linear(-60.0, 0.0, 1, " dB"),
        );
        p[i + RANGE] = P::cont(n[RANGE], DEFAULTS[i + RANGE], signed);
        b += 1;
    }
    p[ATTACK] = P::cont(
        "Attack",
        DEFAULTS[ATTACK],
        Display::exp(0.1, 100.0, 1, " ms"),
    );
    p[RELEASE] = P::cont(
        "Release",
        DEFAULTS[RELEASE],
        Display::exp(10.0, 1000.0, 0, " ms"),
    );
    p
};

/// The module. Not in [`super::modules`]: that is FL Studio Mobile's effects.
pub static MODULE: ModuleSpec = ModuleSpec::effect(
    d::DYNAMIC_EQ,
    "Dynamic EQ",
    "Dynamic EQ",
    false,
    &PARAM_SPECS,
    &["BAND 1", "BAND 2", "BAND 3", "BAND 4", "TIME"],
);
