// Copyright (C) 2026 The Honeydew contributors
// SPDX-License-Identifier: GPL-3.0-or-later

//! Small numeric helpers for the audio thread: none of them allocates.

use std::f32::consts::PI;

/// A xorshift noise source: cheap, deterministic, good enough for noise and random phases.
#[derive(Clone, Copy, Debug)]
pub struct Rng(u32);

impl Rng {
    pub fn new(seed: u32) -> Self {
        Rng(seed | 1)
    }

    pub fn next_u32(&mut self) -> u32 {
        let mut x = self.0;
        x ^= x << 13;
        x ^= x >> 17;
        x ^= x << 5;
        self.0 = x;
        x
    }

    /// Uniform in `[0, 1)`.
    pub fn unit(&mut self) -> f32 {
        (self.next_u32() >> 8) as f32 * (1.0 / 16_777_216.0)
    }

    /// Uniform in `[-1, 1)`.
    pub fn bipolar(&mut self) -> f32 {
        self.unit() * 2.0 - 1.0
    }
}

/// The frequency of MIDI note `note` (fractional), A4 = 440 Hz.
pub fn note_hz(note: f32) -> f32 {
    440.0 * ((note - 69.0) * (1.0 / 12.0)).exp2()
}

pub fn db_to_gain(db: f32) -> f32 {
    (db * 0.115_129_25).exp()
}

/// `sin(2 pi p)` for any `p`, within about 0.1 %.
pub fn fast_sin(p: f32) -> f32 {
    let x = p - (p + 0.5).floor();
    let z = 2.0 * x;
    let y = 4.0 * z * (1.0 - z.abs());
    y * (0.775 + 0.225 * y.abs())
}

/// A rational `tanh`, exact enough for saturation, saturating at +-1.
pub fn soft(x: f32) -> f32 {
    let x = x.clamp(-3.0, 3.0);
    x * (27.0 + x * x) / (27.0 + 9.0 * x * x)
}

/// The output stage: transparent up to 0.8, then a smooth knee towards 1.
pub fn limit(x: f32) -> f32 {
    let a = x.abs();
    if a <= 0.8 {
        x
    } else {
        x.signum() * (0.8 + 0.2 * soft((a - 0.8) * 5.0))
    }
}

/// A one-pole smoothing coefficient for a time constant of `seconds`, applied every `every` samples.
pub fn smooth_coef(seconds: f32, every: f32, sample_rate: f32) -> f32 {
    1.0 - (-every / (seconds * sample_rate).max(1.0)).exp()
}

/// Pan law: left and right gains for `pan` in `[-1, 1]` (constant power).
pub fn pan_gains(pan: f32) -> (f32, f32) {
    let a = (pan.clamp(-1.0, 1.0) + 1.0) * 0.25;
    ((a * PI).cos(), (a * PI).sin())
}

/// Silences denormals and NaNs.
pub fn clean(x: f32) -> f32 {
    if x.is_finite() && x.abs() > 1e-20 {
        x
    } else {
        0.0
    }
}

/// A biquad (transposed direct form II), one channel.
#[derive(Clone, Copy, Debug, Default)]
pub struct Biquad {
    b0: f32,
    b1: f32,
    b2: f32,
    a1: f32,
    a2: f32,
    z1: f32,
    z2: f32,
}

impl Biquad {
    pub fn set(&mut self, b0: f32, b1: f32, b2: f32, a0: f32, a1: f32, a2: f32) {
        let r = 1.0 / a0;
        self.b0 = b0 * r;
        self.b1 = b1 * r;
        self.b2 = b2 * r;
        self.a1 = a1 * r;
        self.a2 = a2 * r;
    }

    pub fn low_pass(&mut self, hz: f32, q: f32, sample_rate: f32) {
        let w = 2.0 * PI * (hz / sample_rate).clamp(1e-4, 0.49);
        let (s, c) = w.sin_cos();
        let al = s / (2.0 * q);
        self.set(
            (1.0 - c) * 0.5,
            1.0 - c,
            (1.0 - c) * 0.5,
            1.0 + al,
            -2.0 * c,
            1.0 - al,
        );
    }

    /// An RBJ shelf: `high` picks the high shelf, else the low one, of `gain_db` at `hz`.
    pub fn shelf(&mut self, high: bool, hz: f32, gain_db: f32, sample_rate: f32) {
        let a = 10f32.powf(gain_db / 40.0);
        let w = 2.0 * PI * (hz / sample_rate).clamp(1e-4, 0.49);
        let (s, c) = w.sin_cos();
        let al = s * 0.5 * 2f32.sqrt();
        let t = 2.0 * a.sqrt() * al;
        if high {
            self.set(
                a * ((a + 1.0) + (a - 1.0) * c + t),
                -2.0 * a * ((a - 1.0) + (a + 1.0) * c),
                a * ((a + 1.0) + (a - 1.0) * c - t),
                (a + 1.0) - (a - 1.0) * c + t,
                2.0 * ((a - 1.0) - (a + 1.0) * c),
                (a + 1.0) - (a - 1.0) * c - t,
            );
        } else {
            self.set(
                a * ((a + 1.0) - (a - 1.0) * c + t),
                2.0 * a * ((a - 1.0) - (a + 1.0) * c),
                a * ((a + 1.0) - (a - 1.0) * c - t),
                (a + 1.0) + (a - 1.0) * c + t,
                -2.0 * ((a - 1.0) + (a + 1.0) * c),
                (a + 1.0) + (a - 1.0) * c - t,
            );
        }
    }

    pub fn tick(&mut self, x: f32) -> f32 {
        let y = self.b0 * x + self.z1;
        self.z1 = self.b1 * x - self.a1 * y + self.z2;
        self.z2 = self.b2 * x - self.a2 * y;
        y
    }

    pub fn reset(&mut self) {
        self.z1 = 0.0;
        self.z2 = 0.0;
    }
}

/// A one-pole filter: low-pass `lp`, and the high-pass as the rest.
#[derive(Clone, Copy, Debug, Default)]
pub struct OnePole {
    z: f32,
}

impl OnePole {
    pub fn coef(hz: f32, sample_rate: f32) -> f32 {
        1.0 - (-2.0 * PI * (hz / sample_rate).clamp(1e-5, 0.45)).exp()
    }

    pub fn lp(&mut self, x: f32, k: f32) -> f32 {
        self.z += (x - self.z) * k;
        self.z
    }

    pub fn reset(&mut self) {
        self.z = 0.0;
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn fast_sin_is_close() {
        for i in 0..1000 {
            let p = i as f32 / 100.0 - 5.0;
            assert!((fast_sin(p) - (p * 2.0 * PI).sin()).abs() < 2e-3, "{p}");
        }
    }

    #[test]
    fn limiter_is_transparent_then_bounded() {
        assert_eq!(limit(0.5), 0.5);
        assert!(limit(10.0) <= 1.0 && limit(10.0) > 0.95);
        assert!(limit(-10.0) >= -1.0);
    }

    #[test]
    fn pan_is_constant_power() {
        for p in [-1.0, -0.3, 0.0, 0.5, 1.0] {
            let (l, r) = pan_gains(p);
            assert!((l * l + r * r - 1.0).abs() < 1e-5);
        }
    }

    #[test]
    fn notes_are_in_tune() {
        assert!((note_hz(69.0) - 440.0).abs() < 1e-3);
        assert!((note_hz(81.0) - 880.0).abs() < 1e-2);
    }

    #[test]
    fn shelf_gain_is_right_at_the_far_end() {
        let sr = 48_000.0;
        let mut b = Biquad::default();
        b.shelf(false, 200.0, 12.0, sr);
        // DC through a low shelf of +12 dB: about 4x.
        let mut y = 0.0;
        for _ in 0..20_000 {
            y = b.tick(1.0);
        }
        assert!((y - db_to_gain(12.0)).abs() < 0.05, "{y}");
    }
}
