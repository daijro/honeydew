//! The factory 3x Osc custom shapes: six single cycles of our own design, written as sums of partials (plus one
//! smoothed noise cycle) and
//! rendered at run time. No sample goes in; every number below is ours, and the names only say what each shape was
//! drawn to evoke. A slot or CSTM `PATH` folder `Built-in/Shapes/<Name>` plays `builtin(folder)` (`builtin_drums`),
//! which calls [`shape`].
//!
//! Each shape is one cycle of [`CYCLE`] frames, mono at [`RATE`], with the DC removed and a −1 dBFS peak. The engine
//! resamples the whole file to its own cycle (`Wave::custom`), so the length is free; a power of two keeps the
//! partials on whole FFT bins. Every shape except `Grit` has at most [`PARTIALS`] partials, and each cycle is exactly
//! periodic (integer partials; `Grit` is smoothed circularly), so its last frame meets its first without a click.
//! Rendering is cheap (about 130k sines), deterministic, and done on the app thread; the sample cache keeps the result.

use std::f64::consts::{PI, TAU};

use crate::builtin_drums::RATE as DRUM_RATE;
use crate::sample::SampleData;

/// The shapes' sample rate.
pub const RATE: u32 = DRUM_RATE;
/// Frames in one cycle.
pub const CYCLE: usize = 2048;
/// The most partials a shape but `Grit` has.
pub const PARTIALS: usize = 64;
/// The shapes, in definition order; the picker lists index order.
pub const SHAPES: [&str; 6] = ["Glass", "Reed", "Vowel A", "Vowel O", "Thin Pulse", "Grit"];

/// The pitch (Hz) the vowels are drawn for. A single cycle has no fixed formants: they sit at fixed harmonics, so a
/// vowel sounds right near this pitch (A2) and brightens or darkens away from it.
const VOWEL_F0: f64 = 110.0;
/// Harmonic numbers of the first three formants of `Vowel A` and `Vowel O`: textbook formant frequencies (Hz, an open
/// "ah" and a round "oh") over [`VOWEL_F0`]. The largest partial is within one harmonic of the first.
pub const VOWEL_A_FORMANTS: [f64; 3] = [730.0 / VOWEL_F0, 1090.0 / VOWEL_F0, 2440.0 / VOWEL_F0];
pub const VOWEL_O_FORMANTS: [f64; 3] = [570.0 / VOWEL_F0, 840.0 / VOWEL_F0, 2410.0 / VOWEL_F0];
/// Formant bump gain and Gaussian half-width (harmonics), first to third.
const FORMANT_GAIN: [f64; 3] = [1.0, 0.6, 0.25];
const FORMANT_WIDTH: [f64; 3] = [1.2, 1.4, 4.0];
/// What the source keeps between the formants (keeps the fundamental audible).
const FORMANT_FLOOR: f64 = 0.08;

/// The partials a bell is drawn from: the first four fill the bottom, then they spread out.
const GLASS: [usize; 12] = [1, 2, 3, 4, 6, 8, 11, 15, 20, 27, 36, 48];
/// `Thin Pulse`'s duty: the fraction of the cycle it is high.
const PULSE_DUTY: f64 = 0.12;
/// `Grit`'s noise seed, and how many times the circular [1 2 1] / 4 smoother runs over it.
const GRIT_SEED: u32 = 0x2f6b_71d3;
const GRIT_SMOOTHING: usize = 2;
/// −1 dBFS.
const PEAK: f32 = 0.891_250_9;

/// The shape `name` (one of [`SHAPES`]) as a one-cycle sample, or `None` for any other name.
pub fn shape(name: &str) -> Option<SampleData> {
    let mut x = match name {
        "Glass" => additive(|n| {
            let a = if GLASS.contains(&n) {
                (n as f64).powf(-0.8)
            } else {
                0.0
            };
            (a, 0.0)
        }),
        // Odd partials lead, the even ones sit 18 dB down, and everything softens from about partial 20.
        "Reed" => additive(|n| {
            let odd = if n % 2 == 1 { 1.0 } else { 0.12 };
            (odd / n as f64 / (1.0 + (n as f64 / 20.0).powi(2)), 0.0)
        }),
        "Vowel A" => additive(|n| (vowel(n, VOWEL_A_FORMANTS), 0.0)),
        "Vowel O" => additive(|n| (vowel(n, VOWEL_O_FORMANTS), 0.0)),
        // A pulse of width d centred at 0.5: partial n is sin(n pi d) / n at a cosine phase of n / 2 cycles, so
        // the cycle starts on the low stretch.
        "Thin Pulse" => additive(|n| {
            let a = (n as f64 * PI * PULSE_DUTY).sin() / n as f64;
            (a, 0.25 - 0.5 * n as f64)
        }),
        "Grit" => grit(),
        _ => return None,
    };
    let mean = x.iter().sum::<f64>() / CYCLE as f64;
    let peak = x.iter().fold(0.0, |m, v| f64::max(m, (v - mean).abs()));
    let gain = f64::from(PEAK) / peak;
    Some(SampleData {
        sample_rate: RATE,
        left: x.iter_mut().map(|v| ((*v - mean) * gain) as f32).collect(),
        right: None,
        ..Default::default()
    })
}

/// One cycle of the partials 1..=[`PARTIALS`], `partial(n)` = (amplitude, phase in cycles) of partial `n` in a sine
/// basis.
fn additive(partial: impl Fn(usize) -> (f64, f64)) -> Vec<f64> {
    let partials: Vec<(f64, f64)> = (1..=PARTIALS).map(partial).collect();
    (0..CYCLE)
        .map(|i| {
            partials
                .iter()
                .zip(1..)
                .map(|(&(a, phase), n)| {
                    // `n * i mod CYCLE` keeps the angle exact however large the partial.
                    a * (TAU * ((n * i % CYCLE) as f64 / CYCLE as f64 + phase)).sin()
                })
                .sum()
        })
        .collect()
}

/// A glottal-like 1/n source shaped by three Gaussian formant bumps over a low floor.
fn vowel(n: usize, formants: [f64; 3]) -> f64 {
    let n = n as f64;
    let bumps: f64 = (0..3)
        .map(|i| FORMANT_GAIN[i] * (-((n - formants[i]) / FORMANT_WIDTH[i]).powi(2)).exp())
        .sum();
    (FORMANT_FLOOR + bumps) / n
}

/// Seeded white noise (xorshift32), smoothed circularly so the cycle's end meets its start like any other pair.
fn grit() -> Vec<f64> {
    let mut state = GRIT_SEED;
    let mut x: Vec<f64> = (0..CYCLE)
        .map(|_| {
            state ^= state << 13;
            state ^= state >> 17;
            state ^= state << 5;
            f64::from(state) / f64::from(u32::MAX) * 2.0 - 1.0
        })
        .collect();
    for _ in 0..GRIT_SMOOTHING {
        let before = x.clone();
        for (i, v) in x.iter_mut().enumerate() {
            *v =
                (before[(i + CYCLE - 1) % CYCLE] + 2.0 * before[i] + before[(i + 1) % CYCLE]) / 4.0;
        }
    }
    x
}
