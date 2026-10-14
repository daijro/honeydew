//! Value-to-text laws: the text each parameter shows for its value.
//!
//! Laws are evaluated in f32: `v` is f32, a linear law is one fused multiply-add in f32, and the result is widened to
//! f64 only to be printed. The f64 exceptions are [`Display::Trunc64`] and Comb Filter Asym's formatter. Each module
//! formatter ([`Display::Custom`]) states its own precision, f32 unless it says otherwise. The [`Display::Table`] and
//! [`Display::GainCurve`] fits of sample points stay in f64.
//!
//! Rounding by kind of law:
//! - The generic law truncates `v·100`.
//! - `round(…)` law rounds halves away from zero, like C's `round`: the percents with a unit
//!   ([`Display::GenericUnit`]: the 3x Osc's 0.305 → 31% and 0.375 → 38% fit only rounding) and the Tape Stop
//!   times `round(v·5000)` ms.
//! - A number printed with a fixed count of decimals ([`Fmt`]) rounds exact binary ties to even, like `printf`:
//!   Flanger Delay `2.5·v²` prints "0.62 ms" at v = 0.5.
//! - A step or menu index (`round(v·(n − 1))`) rounds halves away from zero, like the menus' act-ID form. A selector
//!   whose audio picks its item another way shows the item the audio picks ([`trunc_index`] and the other index laws
//!   below, through [`crate::params::ParamKind::MenuBy`]), and so does a pitch step (the audio's own step function).

/// How a number is signed in the text.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Sign {
    /// Only negative numbers carry a sign ("-12.0 dB", "0.0 dB", "12.0 dB").
    Minus,
    /// Non-zero positive numbers carry a "+" ("-100%", "0%", "+100%").
    Positive,
    /// Zero and positive numbers carry a "+" ("-24", "+0", "+24").
    Always,
}

/// Number format: `decimals` fraction digits, then `unit` verbatim (any space is part of the unit).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Fmt {
    pub decimals: u8,
    pub unit: &'static str,
    pub sign: Sign,
}

impl Fmt {
    pub const fn new(decimals: u8, unit: &'static str) -> Self {
        Self {
            decimals,
            unit,
            sign: Sign::Minus,
        }
    }

    pub const fn sign(self, sign: Sign) -> Self {
        Self { sign, ..self }
    }

    /// Formats `x`. Ties round to even on exact binary ties, like `printf` ("0.62" for 0.625). A value that prints
    /// as zero never gets a minus sign.
    pub fn apply(self, x: f64) -> String {
        let digits = format!("{:.*}", usize::from(self.decimals), x.abs());
        let zero = prints_zero(&digits);
        let sign = match self.sign {
            _ if x < 0.0 && !zero => "-",
            Sign::Always => "+",
            Sign::Positive if !zero => "+",
            _ => "",
        };
        format!("{sign}{digits}{}", self.unit)
    }
}

/// How far from 0.5 a value may be and still be the middle of [`At::Mid`].
const MID_TOLERANCE: f64 = 1e-4;

/// What a value text may read besides the value: the module's other params, the song tempo and the audio sample rate
/// (Autoduck Attack and Hold, Spacer M/S Delay and M/S Inversion, Arpeggiator Rotate, DW Sampler LFO Attack). The
/// default is no module, 0 BPM and no audio, where those read "inf ms", "off" and "inf s".
#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub struct TextCx<'a> {
    /// The module's params by index; one it lacks reads as its default.
    pub params: &'a [f32],
    /// The song tempo, BPM.
    pub tempo_bpm: f64,
    /// The audio output's sample rate, Hz; 0 without audio.
    pub sample_rate: f32,
}

impl TextCx<'_> {
    /// Param `i` of the module, or `default` where the context has none.
    pub fn param(&self, i: usize, default: f32) -> f32 {
        self.params.get(i).copied().unwrap_or(default)
    }
}

/// Where a [`Display::Special`] text replaces the number.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum At {
    /// v = 0.
    Min,
    /// v = 0.5, give or take 1e-4 (float noise from knob maths).
    Mid,
    /// v = 1.
    Max,
}

/// The text law of a continuous parameter: normalised `v` in [0, 1] to the string it shows.
// Only the tests compare laws; a `Custom` formatter is compared by address, which is good enough there.
#[allow(unpredictable_function_pointer_comparisons)]
#[derive(Clone, Copy, Debug, PartialEq)]
pub enum Display {
    /// `v × 100` in f32, truncated, no unit ("0…100 (v×100)"): MOD SPEED 0.499 reads "49".
    Generic,
    /// `round(v × 100)` followed by `unit`, halves away from zero: the percents with a unit ("60 %", 3x Osc "50%"),
    /// which round where the generic law truncates.
    GenericUnit(&'static str),
    /// `min + (max − min)·v`.
    Linear { min: f64, max: f64, fmt: Fmt },
    /// `min + (max − min)·v^exp` (time and rate laws such as `50·v²`).
    Power {
        min: f64,
        max: f64,
        exp: i32,
        fmt: Fmt,
    },
    /// `min·(max/min)^v`.
    Exp { min: f64, max: f64, fmt: Fmt },
    /// Like [`Display::Exp`] in Hz, written as Hz below 1 kHz and as kHz from 1 kHz up.
    Freq {
        min: f64,
        max: f64,
        hz_decimals: u8,
        khz_decimals: u8,
    },
    /// An integer from `min` in `count` steps: `min + round(v·(count − 1))`, the product in f32, halves away from zero.
    Steps { min: i32, count: u32, fmt: Fmt },
    /// A number in `count` steps whose text is the step its audio plays, from
    /// the clamped `v`; recording and stepped knobs snap to the steps k/(count − 1).
    Stepped { count: u32, text: fn(f32) -> String },
    /// `−100 + 200·v` percent as "x% L" / "x% R" (`sides`, after a space), `centre` where it rounds to zero.
    Pan {
        decimals: u8,
        unit: &'static str,
        centre: &'static str,
        sides: [&'static str; 2],
    },
    /// The Leveller gain: `72·v^0.6 − 60` dB, truncated to 0.1 dB, "+" above zero, "0 dB" at zero.
    GainCurve,
    /// The same text at every value ("inf ms").
    Const(&'static str),
    /// Asymmetric around the middle: `−neg·(1 − 2v)` below it and `pos·(2v − 1)` above it.
    Bipolar { neg: f64, pos: f64, fmt: Fmt },
    /// `max·|2v − 1|^exp`: the distance from the middle (the side is the control's business).
    Mirror { max: f64, exp: i32, fmt: Fmt },
    /// `min + (max − min)·v^exp` Hz, from 1000 Hz in KHz, printed with one decimal below 100 and none from there,
    /// then stripped of every trailing `0` and `.` (the Comb Filter Freq: "1 Hz", "30.6 Hz", "157 Hz", "2.5 KHz").
    HzSig { min: f64, max: f64, exp: i32 },
    /// `round(v·full)` milliseconds (halves away from zero) as "%.2f s" from one second up and "%.0f ms" below;
    /// `zero` instead of the number when that rounds to 0 (Tape Stop times: `full` = 5000, `zero` = "Off").
    Millis { full: f64, zero: &'static str },
    /// Straight lines between the `(v, x)` samples (ascending `v`); the end segments continue outside them.
    Table {
        points: &'static [(f64, f64)],
        fmt: Fmt,
    },
    /// The note name of step `round(v·(count − 1))` (halves away from zero), step 0 = "C0" ("C5" at step 60).
    Note { count: u32 },
    /// `inner`, except that `text` replaces the number at one point of the range ("+inf", "OFF").
    Special {
        inner: &'static Display,
        at: At,
        text: &'static str,
    },
    /// `min + (max − min)·v` evaluated in f64 and truncated toward zero, then `unit`: the f64 exceptions (Pitcher
    /// Pitch and Fine Tune, Limiter (legacy) Release Time, Reverb PAN).
    Trunc64 {
        min: f64,
        max: f64,
        unit: &'static str,
    },
    /// A module's own formatter, given the clamped `v`. Its doc states its precision: f32, or f64 where noted
    /// (Comb Filter Asym).
    Custom(fn(f32) -> String),
    /// A module's own formatter that also reads the [`TextCx`]: the tempo, the sample rate or another param.
    CustomCx(fn(f32, TextCx) -> String),
}

impl Display {
    pub const fn linear(min: f64, max: f64, decimals: u8, unit: &'static str) -> Self {
        Self::Linear {
            min,
            max,
            fmt: Fmt::new(decimals, unit),
        }
    }

    pub const fn power(min: f64, max: f64, exp: i32, decimals: u8, unit: &'static str) -> Self {
        Self::Power {
            min,
            max,
            exp,
            fmt: Fmt::new(decimals, unit),
        }
    }

    pub const fn exp(min: f64, max: f64, decimals: u8, unit: &'static str) -> Self {
        Self::Exp {
            min,
            max,
            fmt: Fmt::new(decimals, unit),
        }
    }

    /// The number of steps of a stepped law (behind a [`Display::Special`] text too); `None` for a continuous one.
    pub fn step_count(&self) -> Option<usize> {
        match *self {
            Self::Steps { count, .. } | Self::Note { count } | Self::Stepped { count, .. } => {
                Some(count as usize)
            }
            Self::Special { inner, .. } => inner.step_count(),
            _ => None,
        }
    }

    /// The text for normalised value `v` (clamped to [0, 1]) in the default [`TextCx`].
    pub fn text(&self, v: f32) -> String {
        self.text_in(v, TextCx::default())
    }

    /// The text for normalised value `v` (clamped to [0, 1]) in context `cx`.
    pub fn text_in(&self, v: f32, cx: TextCx) -> String {
        self.text_at(clamp01(v), cx)
    }

    fn text_at(&self, v: f32, cx: TextCx) -> String {
        match *self {
            Self::Generic => format!("{}", (v * 100.0) as i32),
            Self::GenericUnit(unit) => format!("{}{unit}", (v * 100.0).round() as i32),
            Self::Linear { min, max, fmt } => fmt.apply(linear(min, max, v).into()),
            Self::Power { min, max, exp, fmt } => fmt.apply(linear(min, max, v.powi(exp)).into()),
            Self::Exp { min, max, fmt } => fmt.apply(exponential(min, max, v).into()),
            Self::Freq {
                min,
                max,
                hz_decimals,
                khz_decimals,
            } => {
                let hz = f64::from(exponential(min, max, v));
                // The switch follows the printed value: 999.96 Hz reads "1.00 kHz", not "1000.0 Hz".
                if rounded(hz, hz_decimals) >= 1000.0 {
                    Fmt::new(khz_decimals, " kHz").apply(hz / 1000.0)
                } else {
                    Fmt::new(hz_decimals, " Hz").apply(hz)
                }
            }
            Self::Steps { min, count, fmt } => {
                let step = (v * count.saturating_sub(1) as f32).round();
                fmt.apply(f64::from(min) + f64::from(step))
            }
            Self::Stepped { text, .. } => text(v),
            Self::Pan {
                decimals,
                unit,
                centre,
                sides,
            } => {
                let percent = linear(-100.0, 100.0, v);
                let digits = format!("{:.*}", usize::from(decimals), percent.abs());
                if prints_zero(&digits) {
                    centre.to_owned()
                } else {
                    format!("{digits}{unit} {}", sides[usize::from(percent > 0.0)])
                }
            }
            Self::GainCurve => {
                let v = f64::from(v);
                let db = ((72.0 * v.powf(0.6) - 60.0) * 10.0).trunc() / 10.0;
                if db == 0.0 {
                    "0 dB".to_owned()
                } else if db > 0.0 {
                    format!("+{db:.1} dB")
                } else {
                    format!("{db:.1} dB")
                }
            }
            Self::Const(text) => text.to_owned(),
            Self::Bipolar { neg, pos, fmt } => {
                let x = if v < 0.5 {
                    -(neg as f32) * (1.0 - 2.0 * v)
                } else {
                    pos as f32 * (2.0 * v - 1.0)
                };
                fmt.apply(x.into())
            }
            Self::Mirror { max, exp, fmt } => {
                fmt.apply((max as f32 * (2.0 * v - 1.0).abs().powi(exp)).into())
            }
            Self::HzSig { min, max, exp } => {
                let hz = linear(min, max, v.powi(exp));
                let (x, unit) = if hz >= 1000.0 {
                    (hz / 1000.0, " KHz")
                } else {
                    (hz, " Hz")
                };
                let w = (x.log10() as i32).max(1);
                let digits = format!("{:.*}", (2 - w).max(0) as usize, x);
                // Every trailing '0' and '.' goes, so zeros before the point go too ("230" reads "23 Hz", 100 Hz
                // reads "1 Hz").
                let digits = digits.trim_end_matches(['0', '.']);
                format!("{}{unit}", if digits.is_empty() { "0" } else { digits })
            }
            Self::Millis { full, zero } => {
                let ms = f64::from((v * full as f32).round());
                if ms == 0.0 {
                    zero.to_owned()
                } else if ms >= 1000.0 {
                    format!("{:.2} s", ms / 1000.0)
                } else {
                    format!("{ms:.0} ms")
                }
            }
            Self::Table { points, fmt } => {
                let v = f64::from(v);
                // The segment that holds v, or the end segment on its side.
                let i = points
                    .partition_point(|p| p.0 <= v)
                    .clamp(1, points.len() - 1);
                let ((v0, x0), (v1, x1)) = (points[i - 1], points[i]);
                fmt.apply(x0 + (x1 - x0) * (v - v0) / (v1 - v0))
            }
            Self::Note { count } => {
                let step = (v * count.saturating_sub(1) as f32).round() as usize;
                format!("{}{}", NOTES[step % 12], step / 12)
            }
            Self::Special { inner, at, text } => {
                let hit = match at {
                    At::Min => v <= 0.0,
                    At::Mid => f64::from((v - 0.5).abs()) < MID_TOLERANCE,
                    At::Max => v >= 1.0,
                };
                if hit {
                    text.to_owned()
                } else {
                    inner.text_at(v, cx)
                }
            }
            Self::Trunc64 { min, max, unit } => {
                format!("{}{unit}", (min + (max - min) * f64::from(v)) as i64)
            }
            Self::Custom(law) => law(v),
            Self::CustomCx(law) => law(v, cx),
        }
    }
}

/// `min + (max − min)·v`, a linear law: one fused multiply-add in f32.
fn linear(min: f64, max: f64, v: f32) -> f32 {
    ((max - min) as f32).mul_add(v, min as f32)
}

/// `min·(max/min)^v` in f32.
fn exponential(min: f64, max: f64, v: f32) -> f32 {
    min as f32 * ((max / min) as f32).powf(v)
}

const NOTES: [&str; 12] = [
    "C", "C#", "D", "D#", "E", "F", "F#", "G", "G#", "A", "A#", "B",
];

/// `v` limited to [0, 1]; NaN reads as 0.
pub(crate) fn clamp01(v: f32) -> f32 {
    if v.is_nan() { 0.0 } else { v.clamp(0.0, 1.0) }
}

/// `x` as it reads when printed with `decimals` fraction digits.
fn rounded(x: f64, decimals: u8) -> f64 {
    format!("{:.*}", usize::from(decimals), x)
        .parse()
        .unwrap_or(x)
}

/// Whether formatted digits ("0.0", "000") are all zero.
fn prints_zero(digits: &str) -> bool {
    !digits.bytes().any(|b| (b'1'..=b'9').contains(&b))
}

/// Item index of a menu with `len` items: `round(f32(len − 1)·v)`, halves away from zero (the menu
/// act-ID form).
pub fn menu_index(v: f32, len: usize) -> usize {
    let last = len.saturating_sub(1);
    ((clamp01(v) * last as f32).round() as usize).min(last)
}

// The stepped selectors' index laws, for [`crate::params::ParamKind::MenuBy`]:
// each gives k at the menu's own values k/(N items − 1).

/// trunc(N·v) in f32.
pub(super) fn trunc_index<const N: u32>(v: f32) -> usize {
    (N as f32 * v) as usize
}

/// trunc(N·v) with v widened to f64.
pub(super) fn trunc64_index<const N: u32>(v: f32) -> usize {
    (f64::from(N) * f64::from(v)) as usize
}

/// trunc(fma_f32(v, N, 0.5)): N·v rounded half up.
pub(super) fn half_up_index<const N: u32>(v: f32) -> usize {
    v.mul_add(N as f32, 0.5) as usize
}

/// trunc(fma_f32(v, N, 1)) − 1: a count from 1 (voices, semitones), as an index.
pub(super) fn count_index<const N: u32>(v: f32) -> usize {
    v.mul_add(N as f32, 1.0) as usize - 1
}

/// Normalised value that selects item `index` of a menu with `len` items.
pub const fn menu_value(index: usize, len: usize) -> f32 {
    let last = len.saturating_sub(1);
    let index = if index < last { index } else { last };
    if last == 0 {
        0.0
    } else {
        index as f32 / last as f32
    }
}
