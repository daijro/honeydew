// Copyright (C) 2026 The Honeydew contributors
// SPDX-License-Identifier: GPL-3.0-or-later

//! The multimode filter: a stereo pair of zero-delay-feedback state-variable filters, one stage (12 dB per octave) or two
//! (24), low, band or high pass, with an overdrive in front.

use std::f32::consts::PI;

use crate::util::soft;

#[derive(Clone, Copy, Debug, Default)]
struct Svf {
    ic1: f32,
    ic2: f32,
}

/// One stage's coefficients.
#[derive(Clone, Copy, Debug, Default)]
pub struct Coef {
    k: f32,
    a1: f32,
    a2: f32,
    a3: f32,
}

impl Coef {
    pub fn new(hz: f32, k: f32, sample_rate: f32) -> Coef {
        let g = (PI * (hz / sample_rate).clamp(1e-4, 0.49)).tan();
        let a1 = 1.0 / (1.0 + g * (g + k));
        let a2 = g * a1;
        Coef {
            k,
            a1,
            a2,
            a3: g * a2,
        }
    }
}

impl Svf {
    /// (low, band, high) for input `x`.
    #[inline]
    fn tick(&mut self, x: f32, c: &Coef) -> (f32, f32, f32) {
        let v3 = x - self.ic2;
        let v1 = c.a1 * self.ic1 + c.a2 * v3;
        let v2 = self.ic2 + c.a2 * self.ic1 + c.a3 * v3;
        self.ic1 = 2.0 * v1 - self.ic1;
        self.ic2 = 2.0 * v2 - self.ic2;
        (v2, v1, x - c.k * v1 - v2)
    }
}

/// The damping `k` (1 / Q) for a resonance in `[0, 1]`.
pub fn damping(res: f32) -> f32 {
    1.4 * (1.0 - res.clamp(0.0, 1.0)).powf(1.3) + 0.08
}

/// What a filter does with its input.
#[derive(Clone, Copy, Debug)]
pub struct Setup {
    /// 0 low, 1 band, 2 high.
    pub mode: u8,
    pub slope24: bool,
    pub first: Coef,
    pub second: Coef,
    pub drive: f32,
}

impl Setup {
    pub fn new(mode: u8, slope24: bool, hz: f32, res: f32, drive: f32, sample_rate: f32) -> Setup {
        let k = damping(res);
        Setup {
            mode,
            slope24,
            first: Coef::new(hz, k, sample_rate),
            // The second stage stays plain, so 24 dB does not square the resonance peak.
            second: Coef::new(hz, 1.0, sample_rate),
            drive,
        }
    }
}

/// One voice's filter state, both channels.
#[derive(Clone, Copy, Debug, Default)]
pub struct Filter {
    s: [[Svf; 2]; 2],
}

impl Filter {
    pub fn reset(&mut self) {
        *self = Filter::default();
    }

    /// One sample of channel `ch` (0 or 1).
    #[inline]
    pub fn tick(&mut self, ch: usize, x: f32, f: &Setup) -> f32 {
        let x = if f.drive > 0.0 {
            soft(x * (1.0 + f.drive * 6.0)) * (1.0 - 0.3 * f.drive)
        } else {
            x
        };
        let pick = |(lp, bp, hp): (f32, f32, f32), k: f32| match f.mode {
            0 => lp,
            1 => bp * k,
            _ => hp,
        };
        let y = pick(self.s[ch][0].tick(x, &f.first), f.first.k);
        if f.slope24 {
            pick(self.s[ch][1].tick(y, &f.second), f.second.k)
        } else {
            y
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn gain(mode: u8, slope24: bool, cutoff: f32, test_hz: f32) -> f32 {
        let sr = 48_000.0;
        let setup = Setup::new(mode, slope24, cutoff, 0.0, 0.0, sr);
        let mut f = Filter::default();
        let mut peak = 0.0f32;
        for i in 0..48_000 {
            let x = (2.0 * PI * test_hz * i as f32 / sr).sin();
            let y = f.tick(0, x, &setup);
            if i > 24_000 {
                peak = peak.max(y.abs());
            }
        }
        peak
    }

    #[test]
    fn low_pass_passes_lows_and_cuts_highs() {
        assert!(gain(0, false, 1000.0, 100.0) > 0.95);
        let g12 = gain(0, false, 1000.0, 8000.0);
        let g24 = gain(0, true, 1000.0, 8000.0);
        assert!(g12 < 0.05, "{g12}");
        assert!(g24 < g12 * 0.5, "24 dB cuts more: {g24} vs {g12}");
    }

    #[test]
    fn high_pass_cuts_lows() {
        assert!(gain(2, false, 4000.0, 100.0) < 0.05);
        assert!(gain(2, false, 1000.0, 10_000.0) > 0.9);
    }

    #[test]
    fn band_pass_peaks_at_the_cutoff() {
        let at = gain(1, false, 2000.0, 2000.0);
        assert!(at > gain(1, false, 2000.0, 200.0) * 2.0);
        assert!(at > gain(1, false, 2000.0, 15_000.0) * 2.0);
    }

    #[test]
    fn resonance_stays_stable() {
        let sr = 48_000.0;
        let setup = Setup::new(0, true, 5000.0, 1.0, 1.0, sr);
        let mut f = Filter::default();
        let mut peak = 0.0f32;
        for i in 0..96_000 {
            let x = if i % 480 == 0 { 1.0 } else { 0.0 };
            peak = peak.max(f.tick(0, x, &setup).abs());
        }
        assert!(peak.is_finite() && peak < 20.0, "{peak}");
    }
}
