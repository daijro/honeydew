//! The drum pad slot and channel strip params: the slot `PRMS` and the `RPRM` values with their names, defaults
//! and display laws; and every module's common params.

use super::{At, Display, Fmt, ParamSpec as P, Sign};
use honeydew_audio::sample_edit::speed_semitones;

/// The channel and slot Level law: dB = 66·v^0.6 − 60 in f32, to 0.1 dB with a
/// sign on gains; `-inf.` at v = 0. The default 0.8531 is 0 dB.
const LEVEL: Display = Display::Special {
    inner: &Display::Custom(level_db),
    at: At::Min,
    text: "-inf.",
};

fn level_db(v: f32) -> String {
    let db = 66.0f32.mul_add(v.powf(0.6), -60.0);
    Fmt::new(1, " dB").sign(Sign::Positive).apply(db.into())
}

/// "100% left … center … 100% right", linear.
const PAN: Display = Display::Pan {
    decimals: 0,
    unit: "%",
    centre: "center",
    sides: ["left", "right"],
};

/// Slot `PRMS` (11 values), indexed like drum-slot automation (`inst < 0`).
pub static DRUM_SLOT_PARAMS: &[P] = &[
    P::cont("Level", 0.8531, LEVEL),
    P::cont("Pan", 0.5, PAN),
    // −24 … +24 semitones in 49 steps: the audio's f32 fused step.
    P::cont(
        "Speed",
        0.5,
        Display::Stepped {
            count: 49,
            text: |v| {
                Fmt::new(0, "")
                    .sign(Sign::Positive)
                    .apply(speed_semitones(v).into())
            },
        },
    ),
    P::toggle("Mute", 0.0),
    P::toggle("Solo", 0.0),
    P::plain("Length", 0.0),
    P::plain("EG A", 0.0),
    P::plain("EG D", 1.0),
    P::plain("EG S", 1.0),
    P::plain("EG R", 1.0),
    P::cont("Sample Start", 0.5, Display::Custom(sample_start)),
];

/// The signed-square law: u = 2v − 1, offset s = ±0.5·u² seconds with the sign of u (u = 0 counts as +). Text
/// m = 1000·s ms, no unit: |m| < 10 two decimals, < 100 one decimal, else integer truncated toward zero (just below
/// 0.5 "-0.00"). In f32.
fn sample_start(v: f32) -> String {
    let u = 2.0 * v - 1.0;
    let m = 1000.0 * (0.5 * u * u).copysign(u);
    if m.abs() < 10.0 {
        format!("{m:.2}")
    } else if m.abs() < 100.0 {
        format!("{m:.1}")
    } else {
        (m as i32).to_string()
    }
}

/// `RPRM` (6 values), indexed like channel automation (`inst == 0`). Level and Pan take the slot laws, the likely
/// same curves.
pub static CHANNEL_PARAMS: &[P] = &[
    P::cont("Level", 0.8531, LEVEL),
    P::cont("Pan", 0.5, PAN),
    P::toggle("Mute", 0.0),
    P::toggle("Solo", 0.0),
    // TODO: no known law; the generic display.
    P::plain("Pitch Bend", 0.5),
    P::plain("Aftertouch", 0.0),
];

/// A module's common params in `SMPR` order, `[Post Gain, Mix, Wet Pan]`.
pub const COMMON_PARAMS: [P; 3] = [
    P::cont("Post Gain", 0.5, Display::Custom(post_gain)),
    // round(100v) as "N %".
    P::cont("Mix", 1.0, Display::GenericUnit(" %")),
    P::cont("Wet Pan", 0.5, Display::Custom(wet_pan)),
];

/// round(40v − 20) in f32 as "N dB", halves away from zero (0.5625 reads "3 dB").
fn post_gain(v: f32) -> String {
    format!("{} dB", 40f32.mul_add(v, -20.0).round() as i32)
}

/// "Centered" within 0.025 of the middle, else trunc(200·|v − 0.5|) in f32 as "N % Left" below it, "N % Right" above.
fn wet_pan(v: f32) -> String {
    let off = v - 0.5;
    if off.abs() < 0.025 {
        return "Centered".to_owned();
    }
    let side = if off < 0.0 { "Left" } else { "Right" };
    format!("{} % {side}", (200.0 * off.abs()) as i32)
}
