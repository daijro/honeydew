// Copyright (C) 2026 The Honeydew contributors
// SPDX-License-Identifier: GPL-3.0-or-later

//! ADSR envelopes with exponential segments, and the low-frequency oscillators.

use crate::util::{Rng, fast_sin};

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Stage {
    Idle,
    Attack,
    Decay,
    Sustain,
    Release,
}

/// The per-tick smoothing amounts of an envelope's stages, for one tick (a sample, or a control step).
#[derive(Clone, Copy, Debug, Default)]
pub struct EnvCoef {
    att: f32,
    dec: f32,
    rel: f32,
    sus: f32,
}

/// The share of the way to the target covered in `seconds` of exponential approach, taking `ticks` ticks per second.
fn alpha(seconds: f32, rate: f32, reach: f32) -> f32 {
    1.0 - (-reach / (seconds.max(1e-4) * rate)).exp()
}

impl EnvCoef {
    /// For times in seconds and `rate` ticks a second (the attack aims past 1 so it arrives; decay and release cover 60 dB
    /// in their time).
    pub fn new(att: f32, dec: f32, sus: f32, rel: f32, rate: f32) -> EnvCoef {
        EnvCoef {
            att: alpha(att, rate, 1.466),
            dec: alpha(dec, rate, 6.9),
            rel: alpha(rel, rate, 6.9),
            sus,
        }
    }
}

#[derive(Clone, Copy, Debug)]
pub struct Env {
    pub level: f32,
    stage: Stage,
}

impl Default for Env {
    fn default() -> Self {
        Env {
            level: 0.0,
            stage: Stage::Idle,
        }
    }
}

impl Env {
    /// Starts the attack from where the level is (no click when a voice is taken over).
    pub fn gate_on(&mut self) {
        self.stage = Stage::Attack;
    }

    pub fn gate_off(&mut self) {
        if self.stage != Stage::Idle {
            self.stage = Stage::Release;
        }
    }

    pub fn is_idle(&self) -> bool {
        self.stage == Stage::Idle
    }

    pub fn is_released(&self) -> bool {
        matches!(self.stage, Stage::Release | Stage::Idle)
    }

    pub fn reset(&mut self) {
        *self = Env::default();
    }

    #[inline]
    pub fn tick(&mut self, c: &EnvCoef) -> f32 {
        match self.stage {
            Stage::Idle => {}
            Stage::Attack => {
                self.level += (1.3 - self.level) * c.att;
                if self.level >= 1.0 {
                    self.level = 1.0;
                    self.stage = Stage::Decay;
                }
            }
            Stage::Decay => {
                self.level += (c.sus - self.level) * c.dec;
                if (self.level - c.sus).abs() < 1e-4 {
                    self.level = c.sus;
                    self.stage = Stage::Sustain;
                }
            }
            Stage::Sustain => self.level += (c.sus - self.level) * c.dec,
            Stage::Release => {
                self.level -= self.level * c.rel;
                if self.level < 1e-4 {
                    self.level = 0.0;
                    self.stage = Stage::Idle;
                }
            }
        }
        self.level
    }
}

/// A low-frequency oscillator's state, advanced once per control step.
#[derive(Clone, Copy, Debug)]
pub struct Lfo {
    pub phase: f32,
    held: f32,
    prev: f32,
    next: f32,
    rng: Rng,
}

impl Lfo {
    pub fn new(seed: u32) -> Lfo {
        Lfo {
            phase: 0.0,
            held: 0.0,
            prev: 0.0,
            next: 0.0,
            rng: Rng::new(seed),
        }
    }

    /// Restarts at `phase` with fresh random values.
    pub fn start(&mut self, phase: f32) {
        self.phase = phase;
        self.held = self.rng.bipolar();
        self.prev = self.held;
        self.next = self.rng.bipolar();
    }

    /// Moves on by `inc` of a cycle.
    pub fn advance(&mut self, inc: f32) {
        self.phase += inc;
        if self.phase >= 1.0 {
            self.phase -= self.phase.floor();
            self.held = self.rng.bipolar();
            self.prev = self.next;
            self.next = self.rng.bipolar();
        }
    }

    /// The wave (see `LFO_WAVE_NAMES`) in `[-1, 1]`, `offset` of a cycle ahead.
    pub fn value(&self, wave: u8, offset: f32) -> f32 {
        let p = (self.phase + offset).rem_euclid(1.0);
        match wave {
            0 => fast_sin(p),
            1 => 1.0 - 4.0 * ((p + 0.25).rem_euclid(1.0) - 0.5).abs(),
            2 => 2.0 * p - 1.0,
            3 => 1.0 - 2.0 * p,
            4 => {
                if p < 0.5 {
                    1.0
                } else {
                    -1.0
                }
            }
            5 => {
                if p < 0.25 {
                    1.0
                } else {
                    -1.0
                }
            }
            6 => {
                if p < 0.125 {
                    1.0
                } else {
                    -1.0
                }
            }
            7 => self.held,
            8 => {
                let t = self.phase * self.phase * (3.0 - 2.0 * self.phase);
                self.prev + (self.next - self.prev) * t
            }
            9 => (p * 4.0).floor() / 3.0 * 2.0 - 1.0,
            _ => fast_sin(p).abs() * 2.0 - 1.0,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn adsr_rises_holds_and_releases() {
        let sr = 48_000.0;
        let c = EnvCoef::new(0.01, 0.05, 0.5, 0.1, sr);
        let mut e = Env::default();
        assert!(e.is_idle());
        e.gate_on();
        let mut peak = 0.0f32;
        for _ in 0..4800 {
            peak = peak.max(e.tick(&c));
        }
        assert!((peak - 1.0).abs() < 1e-3, "{peak}");
        for _ in 0..48_000 {
            e.tick(&c);
        }
        assert!((e.level - 0.5).abs() < 1e-3, "sustain {}", e.level);
        e.gate_off();
        for _ in 0..48_000 {
            e.tick(&c);
        }
        assert!(e.is_idle() && e.level == 0.0);
    }

    #[test]
    fn retrigger_keeps_the_level() {
        let c = EnvCoef::new(0.05, 0.05, 1.0, 0.5, 48_000.0);
        let mut e = Env::default();
        e.gate_on();
        for _ in 0..2000 {
            e.tick(&c);
        }
        e.gate_off();
        for _ in 0..500 {
            e.tick(&c);
        }
        let before = e.level;
        e.gate_on();
        assert!(e.tick(&c) >= before * 0.99);
    }

    #[test]
    fn lfo_waves_stay_in_range() {
        for wave in 0..11 {
            let mut l = Lfo::new(3);
            l.start(0.0);
            for _ in 0..5000 {
                l.advance(0.0137);
                let v = l.value(wave, 0.2);
                assert!((-1.001..=1.001).contains(&v), "wave {wave}: {v}");
            }
        }
    }

    #[test]
    fn sine_lfo_starts_at_zero_and_rises() {
        let mut l = Lfo::new(1);
        l.start(0.0);
        assert!(l.value(0, 0.0).abs() < 1e-3);
        l.advance(0.1);
        assert!(l.value(0, 0.0) > 0.5);
    }
}
