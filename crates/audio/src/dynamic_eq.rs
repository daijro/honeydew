//! Dynamic EQ (module type [`DYNAMIC_EQ`]), Honeydew's own effect: four EQ bands, each with a static gain and a gain
//! offset that follows the level of a key signal in the band.
//!
//! **Params**, normalized, in this order (the saved one): 0 Enable; then [`BANDS`] bands of [`PER_BAND`] params each,
//! band `b`'s param `p` at [`band`]`(b, p)`: [`ON`], [`SHAPE`], [`FREQUENCY`], [`Q`], [`GAIN`], [`THRESHOLD`],
//! [`RANGE`]; then [`ATTACK`] and [`RELEASE`], shared by the bands. Post Gain, Mix and Wet Pan are the rack's
//! (`CommonModule`). The laws from `v`:
//! - On: on from 0.5. Shape: `round(2v)`, 0 Bell, 1 Low shelf, 2 High shelf ([`Shape`]).
//! - Frequency `20·1000^v` Hz (20 Hz to 20 kHz), Q `0.1·100^v` (0.1 to 10, 1 at 0.5).
//! - Gain and Range `−18 + 36v` dB, Threshold `−60 + 60v` dB.
//! - Attack `0.1·1000^v` ms (0.1 to 100 ms), Release `10·100^v` ms (10 ms to 1 s).
//!
//! **Filters.** Each band is a trapezoidal state-variable filter (Zavalishin's TPT SVF, as Simper writes it) at the
//! band's frequency, prewarped (`g = tan(π·f/rate)`, the frequency held under 0.45·rate). Its coefficients change only
//! with the params: the band's gain is a scalar on the filter's outputs, so it may move every sample. With `A` the
//! band's gain (static Gain plus the dynamic offset) as an amplitude and `h = √A`:
//! - Bell: `y = x + (A − 1)·k·bp`, `k = 1/Q`; `k·bp` is the band-pass of 0 dB peak (the Regalia–Mitra form):
//!   `|H|² = ((1 − w²)² + A²k²w²)/((1 − w²)² + k²w²)` at `w` the frequency ratio (prewarped), `A` at the frequency and 1
//!   far from it, never past either, symmetric in log frequency; a cut is narrower than a boost of the same Q.
//! - Low shelf: two first-order shelves `x + (h − 1)·lp₁(x)` in series, each with half the gain in dB, which the SVF at
//!   `k = 2` gives at once (`lp₁ = lp + bp`, `lp₁² = lp`): `y = x + (h − 1)·(2·(lp + bp) + (h − 1)·lp)`.
//!   `|H| = (A + w²)/(1 + w²)`: `A` well under the frequency, 1 well over it, monotonic. Its poles stay at the frequency,
//!   so its half-gain point (in dB) is at `A^¼` times it: above it for a boost, as far below it for a cut.
//! - High shelf: the same with `hp₁ = hp + bp`: `y = x + (h − 1)·(2·(hp + bp) + (h − 1)·hp)`, `|H| = (1 + Aw²)/(1 + w²)`.
//!
//! Q shapes the bells only. [`response_db`] is the same response, for the panel's curve.
//!
//! **Detector.** Each band measures the key signal (the module's own input until a sidechain feeds it) through its own
//! copy of the band's filter, taking its band part: `k·bp` for a bell, `lp` for a low shelf, `hp` for a high shelf.
//! The level is the louder channel's `|part|`, followed by an envelope that rises with Attack and falls with Release
//! (one-pole, `c = e^(−1/(t·rate))`).
//!
//! **Gain law.** With `over` the envelope's level in dB above Threshold, the band's gain moves by
//! `sign(Range)·clamp(over, 0, |Range|)` dB: each dB over the threshold moves it a dB toward Range, until it reaches
//! Range. A negative Range cuts a loud band (held at the threshold until Range is spent, as a de-esser does); a
//! positive one lifts it. Under the threshold the band is its static Gain.
//!
//! Nothing allocates while it runs: the state is a fixed array, its filters reset in place.

use crate::biquad::flush;

/// The module type: Honeydew's own id, which a song file keeps apart from FL Studio Mobile's modules.
pub const DYNAMIC_EQ: u16 = 1004;

pub const BANDS: usize = 4;
/// A band's params, from its first ([`band`]).
pub const ON: usize = 0;
pub const SHAPE: usize = 1;
pub const FREQUENCY: usize = 2;
pub const Q: usize = 3;
pub const GAIN: usize = 4;
pub const THRESHOLD: usize = 5;
pub const RANGE: usize = 6;
pub const PER_BAND: usize = 7;
/// The envelope's times, after the bands.
pub const ATTACK: usize = 1 + BANDS * PER_BAND;
pub const RELEASE: usize = ATTACK + 1;
/// Enable, the bands' params, Attack and Release.
pub const PARAMS: usize = RELEASE + 1;

/// Band `b`'s param `p` (from 0 each).
pub const fn band(b: usize, p: usize) -> usize {
    1 + PER_BAND * b + p
}

/// A new module's params: every band on and flat (Gain and Range 0 dB, Threshold −20 dB, Q 1), a low shelf at 100 Hz,
/// bells at 500 Hz and 2 kHz and a high shelf at 8 kHz; Attack 3.2 ms, Release 100 ms.
pub const DEFAULTS: [f32; PARAMS] = {
    // Shape and frequency (normalized: 100 Hz, 500 Hz, 2 kHz, 8 kHz).
    const BAND: [(f32, f32); BANDS] = [
        (0.5, 0.232_990),
        (0.0, 0.465_980),
        (0.0, 0.666_667),
        (1.0, 0.867_353),
    ];
    let mut p = [0.5; PARAMS];
    p[0] = 1.0;
    let mut b = 0;
    while b < BANDS {
        p[band(b, ON)] = 1.0;
        (p[band(b, SHAPE)], p[band(b, FREQUENCY)]) = BAND[b];
        p[band(b, THRESHOLD)] = 2.0 / 3.0;
        b += 1;
    }
    p
};

/// A band's shape, `round(2v)` of its Shape param.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum Shape {
    #[default]
    Bell,
    LowShelf,
    HighShelf,
}

impl Shape {
    pub fn from_param(v: f32) -> Self {
        match (2.0 * v.clamp(0.0, 1.0)).round() as u8 {
            0 => Self::Bell,
            1 => Self::LowShelf,
            _ => Self::HighShelf,
        }
    }
}

/// Frequency, Hz.
pub fn hz(v: f32) -> f32 {
    20.0 * 1000f32.powf(v)
}

/// Gain and Range, dB.
pub fn gain_db(v: f32) -> f32 {
    36.0f32.mul_add(v, -18.0)
}

/// A band's params, decoded.
#[derive(Clone, Copy, Debug, Default)]
struct Setting {
    on: bool,
    shape: Shape,
    hz: f32,
    q: f32,
    gain_db: f32,
    threshold_db: f32,
    range_db: f32,
}

impl Setting {
    /// Band `b` of `params`; a param they lack reads as its default.
    fn of(params: &[f32], b: usize) -> Self {
        let v = |p: usize| {
            let i = band(b, p);
            params.get(i).copied().unwrap_or(DEFAULTS[i])
        };
        Self {
            on: v(ON) >= 0.5,
            shape: Shape::from_param(v(SHAPE)),
            hz: hz(v(FREQUENCY)),
            q: 0.1 * 100f32.powf(v(Q)),
            gain_db: gain_db(v(GAIN)),
            threshold_db: 60.0f32.mul_add(v(THRESHOLD), -60.0),
            range_db: gain_db(v(RANGE)),
        }
    }

    /// The SVF's damping: `1/Q` for a bell, 2 for a shelf (two first-order poles).
    fn k(&self) -> f32 {
        match self.shape {
            Shape::Bell => 1.0 / self.q,
            Shape::LowShelf | Shape::HighShelf => 2.0,
        }
    }

    /// The prewarped frequency `g = tan(π·f/rate)`, the frequency held under 0.45·rate.
    fn g(&self, sample_rate: f32) -> f32 {
        (std::f32::consts::PI * (self.hz / sample_rate).min(0.45)).tan()
    }
}

/// The response of `params`' bands at `hz`, dB: each band that is on at its Gain plus `dynamic_db[b]`, at `sample_rate`
/// (the module doc's `|H|`, at `w = tan(π·hz/rate)/g`).
pub fn response_db(params: &[f32], dynamic_db: [f32; BANDS], hz: f32, sample_rate: f32) -> f32 {
    let at = (std::f32::consts::PI * (hz / sample_rate).min(0.4999)).tan();
    (0..BANDS)
        .map(|b| (Setting::of(params, b), dynamic_db[b]))
        .filter(|(s, _)| s.on)
        .map(|(s, dynamic)| {
            let a = 10f32.powf((s.gain_db + dynamic) / 20.0);
            let w2 = (at / s.g(sample_rate)).powi(2);
            let magnitude = match s.shape {
                Shape::Bell => {
                    let (d, kw2) = ((1.0 - w2).powi(2), s.k().powi(2) * w2);
                    ((d + a * a * kw2) / (d + kw2)).sqrt()
                }
                Shape::LowShelf => (a + w2) / (1.0 + w2),
                Shape::HighShelf => (1.0 + a * w2) / (1.0 + w2),
            };
            20.0 * magnitude.log10()
        })
        .sum()
}

/// A band's filter coefficients: the SVF's damping `k` and `a1 = 1/(1 + g(g + k))`, `a2 = g·a1`, `a3 = g·a2`.
#[derive(Clone, Copy, Default)]
struct Coefs {
    k: f32,
    a1: f32,
    a2: f32,
    a3: f32,
}

/// A trapezoidal SVF's two integrator states.
#[derive(Clone, Copy, Default)]
struct Svf {
    ic1: f32,
    ic2: f32,
}

impl Svf {
    /// One sample through the filter of `c`: its (low, band, high) outputs.
    fn tick(&mut self, c: &Coefs, x: f32) -> (f32, f32, f32) {
        let v3 = x - self.ic2;
        let bp = c.a1 * self.ic1 + c.a2 * v3;
        let lp = self.ic2 + c.a2 * self.ic1 + c.a3 * v3;
        self.ic1 = flush(2.0 * bp - self.ic1);
        self.ic2 = flush(2.0 * lp - self.ic2);
        (lp, bp, x - c.k * bp - lp)
    }
}

/// A band: its params, coefficients and running state.
#[derive(Clone, Copy, Default)]
struct Band {
    s: Setting,
    c: Coefs,
    /// The program's filters (left, right) and the key's.
    audio: [Svf; 2],
    key: [Svf; 2],
    envelope: f32,
    /// The gain offset the detector gives the band now, dB.
    dynamic_db: f32,
}

/// The Dynamic EQ's state.
#[derive(Default)]
pub struct DynamicEqState {
    bands: [Band; BANDS],
    /// The envelope's coefficients.
    attack: f32,
    release: f32,
}

impl DynamicEqState {
    /// Takes the params: coefficients only, no sample.
    pub fn refresh(&mut self, params: &[f32], sample_rate: f32) {
        let time = |i: usize, min_ms: f32, span: f32| {
            let ms = min_ms * span.powf(params.get(i).copied().unwrap_or(DEFAULTS[i]));
            (-1000.0 / (ms * sample_rate)).exp()
        };
        self.attack = time(ATTACK, 0.1, 1000.0);
        self.release = time(RELEASE, 10.0, 100.0);
        for (b, band) in self.bands.iter_mut().enumerate() {
            let s = Setting::of(params, b);
            if !s.on {
                // Off: it starts again from rest.
                *band = Band::default();
                continue;
            }
            let (g, k) = (s.g(sample_rate), s.k());
            let a1 = 1.0 / (1.0 + g * (g + k));
            (band.s, band.c) = (
                s,
                Coefs {
                    k,
                    a1,
                    a2: g * a1,
                    a3: g * g * a1,
                },
            );
        }
    }

    pub fn reset(&mut self) {
        for band in &mut self.bands {
            (band.audio, band.key) = Default::default();
            (band.envelope, band.dynamic_db) = (0.0, 0.0);
        }
    }

    /// Each band's gain offset from its detector now, dB (0 for a band that is off).
    pub fn dynamic_db(&self) -> [f32; BANDS] {
        self.bands.map(|b| b.dynamic_db)
    }

    /// One frame of `l` and `r`, the bands' detectors fed `key` (the module doc's **Detector** and **Gain law**).
    pub fn process_frame(&mut self, l: f32, r: f32, key: (f32, f32)) -> (f32, f32) {
        let (attack, release) = (self.attack, self.release);
        let mut out = [l, r];
        for band in self.bands.iter_mut().filter(|b| b.s.on) {
            let (s, c) = (band.s, band.c);
            let mut level = 0f32;
            for (svf, x) in band.key.iter_mut().zip([key.0, key.1]) {
                let (lp, bp, hp) = svf.tick(&c, x);
                let part = match s.shape {
                    Shape::Bell => c.k * bp,
                    Shape::LowShelf => lp,
                    Shape::HighShelf => hp,
                };
                level = level.max(part.abs());
            }
            let coef = if level > band.envelope {
                attack
            } else {
                release
            };
            band.envelope = flush(level + coef * (band.envelope - level));
            let over = 20.0 * band.envelope.max(1e-6).log10() - s.threshold_db;
            band.dynamic_db = s.range_db.signum() * over.clamp(0.0, s.range_db.abs());
            // h = √A, A the band's gain as an amplitude.
            let h = 10f32.powf((s.gain_db + band.dynamic_db) / 40.0);
            for (x, svf) in out.iter_mut().zip(&mut band.audio) {
                let (lp, bp, hp) = svf.tick(&c, *x);
                *x += match s.shape {
                    Shape::Bell => (h * h - 1.0) * c.k * bp,
                    Shape::LowShelf => (h - 1.0) * (2.0 * (lp + bp) + (h - 1.0) * lp),
                    Shape::HighShelf => (h - 1.0) * (2.0 * (hp + bp) + (h - 1.0) * hp),
                };
            }
        }
        (out[0], out[1])
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{AudioEffect, ProcessContext};

    const RATE: f32 = 48_000.0;

    /// A sine of `hz` at `amplitude` through a Dynamic EQ whose band 1 is a 1 kHz bell (Q 1) with Threshold −30 dB and
    /// Range −12 dB, the other bands at their defaults (flat): the output's level against the input's over the last
    /// quarter of a second, dB, and band 1's gain offset as the panel reads it. With `key`, a 1 kHz sine of that amplitude
    /// is the sidechain key ([`AudioEffect::process_keyed`]).
    fn change_db(hz: f32, amplitude: f32, key: Option<f32>) -> (f32, f32) {
        let mut effect = AudioEffect::new(DYNAMIC_EQ, RATE).unwrap();
        for (p, v) in [
            (SHAPE, 0.0),
            (FREQUENCY, 0.566_323),
            (Q, 0.5),
            (THRESHOLD, 0.5),
            (RANGE, 1.0 / 6.0),
        ] {
            effect.set_parameter(band(0, p), v).unwrap();
        }
        let context = ProcessContext {
            sample_rate: RATE,
            tempo_bpm: 120.0,
            start_beat: 0.0,
            playing: true,
        };
        let input: Vec<f32> = (0..48_000)
            .map(|i| amplitude * (std::f32::consts::TAU * hz * i as f32 / RATE).sin())
            .collect();
        let key: Option<Vec<f32>> = key.map(|amplitude| {
            (0..48_000)
                .map(|i| amplitude * (std::f32::consts::TAU * 1000.0 * i as f32 / RATE).sin())
                .collect()
        });
        let (mut left, mut right) = (input.clone(), input.clone());
        for (n, (l, r)) in left.chunks_mut(512).zip(right.chunks_mut(512)).enumerate() {
            let k = key.as_deref().map(|k| &k[n * 512..n * 512 + l.len()]);
            effect
                .process_keyed(l, r, k.map(|k| (k, k)), context)
                .unwrap();
        }
        let rms = |x: &[f32]| (x[36_000..].iter().map(|v| v * v).sum::<f32>()).sqrt();
        assert_eq!(left, right);
        let offset = crate::analysis::dynamic_eq(effect.analysis()).unwrap()[0];
        (20.0 * (rms(&left) / rms(&input)).log10(), offset)
    }

    #[test]
    fn a_band_cuts_its_loud_frequency_by_its_range_and_leaves_the_rest() {
        // A loud 1 kHz sine (−6 dBFS, 24 dB over): cut by the Range, which the panel is told.
        let (loud, offset) = change_db(1000.0, 0.5, None);
        assert!((loud + 12.0).abs() < 0.5, "{loud}");
        assert!((offset + 12.0).abs() < 1e-3, "{offset}");
        // A quiet one, under the threshold, and a loud 100 Hz tone pass nearly unchanged.
        for (hz, amplitude) in [(1000.0, 0.01), (100.0, 0.5)] {
            let (change, _) = change_db(hz, amplitude, None);
            assert!(change.abs() < 0.1, "{hz} Hz at {amplitude}: {change}");
        }
        // Keyed: the quiet sine is cut when a loud key crosses the threshold, and the loud one passes under a silent key.
        let (keyed, _) = change_db(1000.0, 0.01, Some(0.5));
        assert!((keyed + 12.0).abs() < 0.5, "{keyed}");
        let (silent, _) = change_db(1000.0, 0.5, Some(0.0));
        assert!(silent.abs() < 0.1, "{silent}");
    }
}
