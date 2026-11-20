//! Stable parameter layouts for original Banana rack effects.
use super::{Display, ModuleSpec, ParamSpec as P};
use banaforge_audio::banana_power as b;
const PERCENT: Display = Display::linear(0.0, 100.0, 0, " %");
const TRIM: P = P::cont("Trim", 0.5, Display::linear(-12.0, 12.0, 1, " dB"));
const FRESH: [P; 4] = [
    P::ENABLE,
    P::cont("Mid Air", 0.0, PERCENT),
    P::cont("High Air", 0.0, PERCENT),
    TRIM,
];
const GOOD: [P; 3] = [
    P::ENABLE,
    P::cont("Amount", 0.0, PERCENT),
    P::menu("Profile", 0.0, &["A", "B", "C", "D"]),
];
const BLOOD: [P; 6] = [
    P::ENABLE,
    P::cont("Drive", 0.35, PERCENT),
    P::cont("Color", 0.5, PERCENT),
    P::cont("Post Filter", 0.7, PERCENT),
    TRIM,
    P::cont("Mix", 1.0, PERCENT),
];
const GROSS: [P; b::PARAMS] = {
    let mut p = [P::cont("Time Point", 0.0, PERCENT); b::PARAMS];
    p[0] = P::ENABLE;
    p[1] = P::menu(
        "Time",
        0.0,
        &["Through", "Half", "Tape Stop", "Stutter", "Custom"],
    );
    p[2] = P::menu("Volume", 0.0, &["Through", "Gate", "Pump", "Custom"]);
    p[3] = P::menu(
        "Length",
        0.5,
        &["1 beat", "2 beats", "1 bar", "2 bars", "4 bars"],
    );
    p[4] = P::cont("Mix", 1.0, PERCENT);
    let mut i = b::VOLUME_POINTS;
    while i < b::PARAMS {
        p[i] = P::cont("Volume Point", 1.0, PERCENT);
        i += 1;
    }
    p
};
pub static MODULES: [ModuleSpec; 4] = [
    ModuleSpec::effect(
        b::FRESH_AIR,
        "Banana Fresh Air",
        "Banana Fresh Air",
        false,
        &FRESH,
        &[],
    ),
    ModuleSpec::effect(
        b::SOUNDGOODIZER,
        "Banana Soundgoodizer",
        "Banana Soundgoodizer",
        false,
        &GOOD,
        &[],
    ),
    ModuleSpec::effect(
        b::BLOOD_OVERDRIVE,
        "Banana Blood Overdrive",
        "Banana Blood Overdrive",
        false,
        &BLOOD,
        &[],
    ),
    ModuleSpec::effect(
        b::GROSS_BEAT,
        "Banana Gross Beat",
        "Banana Gross Beat",
        false,
        &GROSS,
        &["TIME", "VOLUME"],
    ),
];
