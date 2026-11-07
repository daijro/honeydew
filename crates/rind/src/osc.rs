// Copyright (C) 2026 The Honeydew contributors
// SPDX-License-Identifier: GPL-3.0-or-later

//! Band-limited oscillators (PolyBLEP for saw and pulse, PolyBLAMP for the triangle) with up to eight unison voices, each
//! spread in pitch and across the stereo field.

use crate::util::{Rng, fast_sin, pan_gains};

pub const MAX_UNISON: usize = 8;

/// The residual of a unit step at phase 0, for a sample at phase `t` and phase increment `dt`.
fn blep(t: f32, dt: f32) -> f32 {
    if t < dt {
        let x = t / dt;
        x + x - x * x - 1.0
    } else if t > 1.0 - dt {
        let x = (t - 1.0) / dt;
        x * x + x + x + 1.0
    } else {
        0.0
    }
}

/// The residual of a unit slope change at phase 0 (in units of `dt`).
fn blamp(t: f32, dt: f32) -> f32 {
    if t < dt {
        let x = t / dt - 1.0;
        -(1.0 / 3.0) * x * x * x
    } else if t > 1.0 - dt {
        let x = (t - 1.0) / dt + 1.0;
        (1.0 / 3.0) * x * x * x
    } else {
        0.0
    }
}

fn wrap(x: f32) -> f32 {
    if x >= 1.0 {
        x - 1.0
    } else if x < 0.0 {
        x + 1.0
    } else {
        x
    }
}

fn saw(p: f32, dt: f32) -> f32 {
    2.0 * p - 1.0 - blep(p, dt)
}

fn tri(p: f32, dt: f32) -> f32 {
    let naive = 4.0 * (p - 0.5).abs() - 1.0;
    naive - 8.0 * dt * blamp(p, dt) + 8.0 * dt * blamp(wrap(p - 0.5), dt)
}

fn pulse(p: f32, dt: f32, w: f32) -> f32 {
    let naive = if p < w { 1.0 } else { -1.0 };
    let y = naive + blep(p, dt) - blep(wrap(p - w), dt);
    (y - (2.0 * w - 1.0)) / (2.0 * w.max(1.0 - w))
}

/// One sample of waveform `wave` (see `WAVE_NAMES`) at phase `p` in `[0, 1)`.
#[inline]
fn sample(wave: u8, p: f32, dt: f32, rng: &mut Rng) -> f32 {
    match wave {
        0 => fast_sin(p),
        1 => saw(p, dt),
        2 => tri(p, dt),
        3 => pulse(p, dt, 0.5),
        4 => pulse(p, dt, 0.25),
        5 => pulse(p, dt, 0.125),
        6 => 0.5 * (saw(p, dt) + tri(p, dt)),
        _ => rng.bipolar() * 0.7,
    }
}

/// One oscillator's unison voices.
#[derive(Clone, Debug)]
pub struct Osc {
    phase: [f32; MAX_UNISON],
    dt: [f32; MAX_UNISON],
    ratio: [f32; MAX_UNISON],
    pgl: [f32; MAX_UNISON],
    pgr: [f32; MAX_UNISON],
    n: usize,
    /// What `ratio` and the pan gains were computed for, to compute them again only when it moves.
    ratio_key: (usize, f32),
    pan_key: (usize, f32, f32),
}

impl Default for Osc {
    fn default() -> Self {
        Osc {
            phase: [0.0; MAX_UNISON],
            dt: [0.0; MAX_UNISON],
            ratio: [1.0; MAX_UNISON],
            pgl: [1.0; MAX_UNISON],
            pgr: [1.0; MAX_UNISON],
            n: 0,
            ratio_key: (0, f32::NAN),
            pan_key: (0, f32::NAN, f32::NAN),
        }
    }
}

/// A unison voice's place in `[-1, 1]`.
fn spread(i: usize, n: usize) -> f32 {
    if n <= 1 {
        0.0
    } else {
        -1.0 + 2.0 * i as f32 / (n - 1) as f32
    }
}

impl Osc {
    /// Starts the voices' phases: `rand` of a random turn each.
    pub fn start(&mut self, rand: f32, rng: &mut Rng) {
        for p in &mut self.phase {
            *p = rand * rng.unit();
        }
    }

    /// Sets `n` voices (`0..=8`) of base phase increment `base_dt`, `detune_semis` apart at the outer voices, spread over
    /// `stereo` around `pan`. Returns the normalising gain for the sum.
    pub fn configure(
        &mut self,
        n: usize,
        base_dt: f32,
        detune_semis: f32,
        stereo: f32,
        pan: f32,
    ) -> f32 {
        let n = n.min(MAX_UNISON);
        self.n = n;
        if self.ratio_key != (n, detune_semis) {
            self.ratio_key = (n, detune_semis);
            for i in 0..n {
                self.ratio[i] = (detune_semis * spread(i, n) * (1.0 / 12.0)).exp2();
            }
        }
        if self.pan_key != (n, stereo, pan) {
            self.pan_key = (n, stereo, pan);
            for i in 0..n {
                let (l, r) = pan_gains(pan + spread(i, n) * stereo);
                self.pgl[i] = l;
                self.pgr[i] = r;
            }
        }
        for i in 0..n {
            self.dt[i] = (base_dt * self.ratio[i]).min(0.45);
        }
        if n == 0 { 0.0 } else { 1.0 / (n as f32).sqrt() }
    }

    pub fn voices(&self) -> usize {
        self.n
    }

    /// One stereo sample, the voices summed. `phase_off` is in `[0, 1)`.
    #[inline]
    pub fn tick(&mut self, wave: u8, phase_off: f32, rng: &mut Rng) -> (f32, f32) {
        let (mut l, mut r) = (0.0, 0.0);
        for i in 0..self.n {
            let p = self.phase[i];
            let dt = self.dt[i];
            let mut rp = p + phase_off;
            if rp >= 1.0 {
                rp -= 1.0;
            }
            let s = sample(wave, rp, dt, rng);
            l += s * self.pgl[i];
            r += s * self.pgr[i];
            let np = p + dt;
            self.phase[i] = if np >= 1.0 { np - 1.0 } else { np };
        }
        (l, r)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn run(wave: u8, hz: f32, sr: f32, n: usize) -> Vec<f32> {
        let mut o = Osc::default();
        let mut rng = Rng::new(1);
        o.configure(1, hz / sr, 0.0, 0.0, 0.0);
        (0..n).map(|_| o.tick(wave, 0.0, &mut rng).0).collect()
    }

    #[test]
    fn every_wave_is_finite_and_bounded() {
        for wave in 0..8 {
            for hz in [55.0, 440.0, 3000.0, 12_000.0] {
                for s in run(wave, hz, 48_000.0, 4800) {
                    assert!(s.is_finite() && s.abs() < 1.6, "wave {wave} at {hz}: {s}");
                }
            }
        }
    }

    #[test]
    fn waves_have_the_right_pitch() {
        for wave in 1..7 {
            let out = run(wave, 480.0, 48_000.0, 48_000);
            let crossings = out.windows(2).filter(|w| w[0] < 0.0 && w[1] >= 0.0).count();
            assert!(
                (crossings as i32 - 480).abs() <= 2,
                "wave {wave}: {crossings}"
            );
        }
    }

    #[test]
    fn waves_have_no_dc() {
        for wave in 0..7 {
            let out = run(wave, 500.0, 48_000.0, 48_000);
            let mean = out.iter().sum::<f32>() / out.len() as f32;
            assert!(mean.abs() < 0.02, "wave {wave}: {mean}");
        }
    }

    #[test]
    fn unison_spreads_and_normalises() {
        let mut o = Osc::default();
        let mut rng = Rng::new(7);
        let g = o.configure(8, 440.0 / 48_000.0, 0.3, 1.0, 0.0);
        assert!((g - 1.0 / 8f32.sqrt()).abs() < 1e-6);
        o.start(1.0, &mut rng);
        let (mut l, mut r) = (0.0, 0.0);
        for _ in 0..4800 {
            let (a, b) = o.tick(1, 0.0, &mut rng);
            l += a * a;
            r += b * b;
        }
        assert!(l > 1.0 && r > 1.0);
    }

    #[test]
    fn no_voices_make_silence() {
        let mut o = Osc::default();
        let mut rng = Rng::new(1);
        assert_eq!(o.configure(0, 0.01, 0.0, 0.0, 0.0), 0.0);
        assert_eq!(o.tick(1, 0.0, &mut rng), (0.0, 0.0));
    }
}
