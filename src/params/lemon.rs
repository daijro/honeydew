// Copyright (C) 2026 ItsAlphaNeon
// SPDX-License-Identifier: GPL-3.0-or-later
// Parts of this file are from Lemon Studio: https://github.com/ItsAlphaNeon/Lemon-Studio
// Modified for Honeydew: the knob names and value texts of Lemon Studio's `plugins/lemon-reverb/src/params.rs` and
// `plugins/lemon-limiter/src/params.rs`, as Honeydew registry params, in Honeydew's units.

//! Lemon Studio's reverb and limiter ([`honeydew_audio::lemon`]): their params and value texts. A knob's value is
//! Lemon's stored value, normalized over the knob's range, and reads as Lemon's plugins show it.

use honeydew_audio::lemon::{self, dynamics, norm, reverb, stored};

use super::{Display, ModuleSpec, ParamSpec as P, TextCx};

/// Reverb knob `K` at its default, reading `reverb_text::<K>`.
const fn rv<const K: usize>(name: &'static str) -> P {
    let default = norm(reverb::RANGES[K], reverb::DEFAULTS[K]);
    P::cont(name, default, Display::Custom(reverb_text::<K>))
}

/// Limiter knob `K` at its default, reading `limiter_text::<K>`; its curve knobs step through their eight shapes.
const fn lm<const K: usize>(name: &'static str) -> P {
    let (min, max) = dynamics::RANGES[K];
    let display = match K {
        dynamics::LIM_ATT_CURVE | dynamics::LIM_REL_CURVE | dynamics::COMP_CURVE => {
            Display::Stepped {
                count: (max - min + 1) as u32,
                text: limiter_text::<K>,
            }
        }
        _ => Display::Custom(limiter_text::<K>),
    };
    P::cont(name, norm((min, max), dynamics::DEFAULTS[K]), display)
}

const REVERB: &[P] = &[
    P::ENABLE,
    rv::<{ reverb::LOW_CUT }>("Low Cut"),
    rv::<{ reverb::HIGH_CUT }>("High Cut"),
    P::cont(
        "Predelay",
        norm(
            reverb::RANGES[reverb::PREDELAY],
            reverb::DEFAULTS[reverb::PREDELAY],
        ),
        Display::CustomCx(predelay_text),
    ),
    rv::<{ reverb::ROOM }>("Room Size"),
    rv::<{ reverb::DIFFUSION }>("Diffusion"),
    rv::<{ reverb::DECAY }>("Decay"),
    rv::<{ reverb::DAMPING }>("High Damping"),
    rv::<{ reverb::BASS }>("Bass Multiplier"),
    rv::<{ reverb::CROSSOVER }>("Crossover"),
    rv::<{ reverb::SEPARATION }>("Separation"),
    rv::<{ reverb::DRY }>("Dry Level"),
    rv::<{ reverb::EARLY }>("Early Reflections"),
    rv::<{ reverb::WET }>("Wet Level"),
    rv::<{ reverb::MOD_SPEED }>("Mod Speed"),
    rv::<{ reverb::MOD_DEPTH }>("Mod Depth"),
    P::toggle("Tempo Predelay", 0.0),
    P::toggle("Mid/Side Input", 0.0),
];

const LIMITER: &[P] = &[
    P::ENABLE,
    lm::<{ dynamics::GAIN }>("Gain"),
    lm::<{ dynamics::SAT }>("Saturation"),
    lm::<{ dynamics::CEILING }>("Ceiling"),
    lm::<{ dynamics::LIM_ATT }>("Attack"),
    lm::<{ dynamics::LIM_ATT_CURVE }>("Attack Curve"),
    lm::<{ dynamics::LIM_REL }>("Release"),
    lm::<{ dynamics::LIM_REL_CURVE }>("Release Curve"),
    lm::<{ dynamics::PEAK_WINDOW }>("Peak Window"),
    lm::<{ dynamics::COMP_THRES }>("Comp Threshold"),
    lm::<{ dynamics::COMP_RATIO }>("Comp Ratio"),
    lm::<{ dynamics::COMP_KNEE }>("Comp Knee"),
    lm::<{ dynamics::COMP_ATT }>("Comp Attack"),
    lm::<{ dynamics::COMP_REL }>("Comp Release"),
    lm::<{ dynamics::COMP_CURVE }>("Comp Curve"),
    lm::<{ dynamics::RMS_WINDOW }>("RMS Window"),
    lm::<{ dynamics::GATE_GAIN }>("Gate Gain"),
    lm::<{ dynamics::GATE_THRES }>("Gate Threshold"),
    lm::<{ dynamics::GATE_REL }>("Gate Release"),
    P::toggle("Ceiling Safety", 1.0),
];

/// The two modules. Not in [`super::modules`]: those are the song format's own effects.
pub static MODULES: [ModuleSpec; 2] = [
    ModuleSpec::effect(
        lemon::REVERB,
        "Lemon Reverb",
        "Lemon Reverb",
        false,
        REVERB,
        &["ROOM", "TONE", "MIX"],
    ),
    ModuleSpec::effect(
        lemon::LIMITER,
        "Lemon Limiter",
        "Lemon Limiter",
        false,
        LIMITER,
        &["LIMIT", "COMP", "GATE"],
    ),
];

fn reverb_text<const K: usize>(v: f32) -> String {
    reverb_value(K, stored(reverb::RANGES[K], v), false)
}

/// The pre-delay: milliseconds, or 16th-note steps and their 48 ticks while Tempo Predelay is on.
fn predelay_text(v: f32, cx: TextCx) -> String {
    let tempo = cx.param(lemon::REVERB_TEMPO, 0.0) >= 0.5;
    let k = reverb::PREDELAY;
    reverb_value(k, stored(reverb::RANGES[k], v), tempo)
}

fn reverb_value(k: usize, x: i32, tempo: bool) -> String {
    use reverb::*;
    // A level or modulation percentage: halves round up.
    let percent = |full: i32| (f64::from(x) * 100.0 / f64::from(full)).round();
    match k {
        LOW_CUT if x <= LOW_CUT_OFF => "Off".into(),
        HIGH_CUT | DAMPING if x >= LOWPASS_OFF => "Off".into(),
        HIGH_CUT | DAMPING => format!("{:.1} kHz", f64::from(x) / 10.0),
        LOW_CUT | CROSSOVER => format!("{x} Hz"),
        PREDELAY if tempo => format!("{}:{:02}", x.div_euclid(48), x.rem_euclid(48)),
        PREDELAY => format!("{} ms", (i64::from(x) * 1000).div_euclid(384)),
        DECAY => format!("{:.1} s", f64::from(x) / 10.0),
        BASS => format!("{x}%"),
        SEPARATION if x == 0 => "Original".into(),
        // Halves to even here.
        SEPARATION => format!(
            "{}% {}",
            (f64::from(x.abs()) * 100.0 / 64.0).round_ties_even(),
            if x < 0 { "sep." } else { "merged" }
        ),
        DRY | EARLY | WET => format!("{}%", percent(128)),
        MOD_SPEED | MOD_DEPTH => format!("{}%", percent(2048)),
        _ => x.to_string(),
    }
}

fn limiter_text<const K: usize>(v: f32) -> String {
    use dynamics::*;
    let x = stored(RANGES[K], v);
    let db = |a: f32| match a {
        a if a <= 0.0 => "-inf dB".to_owned(),
        a => format!("{:.1} dB", 20.0 * f64::from(a).log10()),
    };
    let ms = |t: f32| match t {
        t if t >= 100.0 => format!("{t:.0} ms"),
        t if t >= 10.0 => format!("{t:.1} ms"),
        t => format!("{t:.2} ms"),
    };
    match K {
        GAIN | COMP_THRES | GATE_GAIN | GATE_THRES => db(level(x, 7.0)),
        SAT | CEILING => db(level(x, 3.0)),
        LIM_ATT | PEAK_WINDOW | COMP_ATT | RMS_WINDOW => ms(time_ms(x)),
        // A release shows twice its time.
        LIM_REL | COMP_REL | GATE_REL => ms(2.0 * time_ms(x)),
        COMP_RATIO => {
            let c = 51f64.powf(f64::from(x.unsigned_abs()) / 1000.0) - 1.0;
            match x {
                1.. => format!("{:.1}:1", 0.38 * c + 1.0),
                _ => format!("1:{:.1}", 0.06 * c + 1.0),
            }
        }
        COMP_KNEE => format!("{}%", (f64::from(x) / 10.0).round()),
        _ => x.to_string(),
    }
}
