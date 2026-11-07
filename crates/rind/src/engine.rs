// Copyright (C) 2026 The Honeydew contributors
// SPDX-License-Identifier: GPL-3.0-or-later

//! The synthesis engine: sixteen preallocated voices, the arpeggiator, parameter smoothing and the master effects.
//! Everything here runs on the audio thread and allocates nothing.

use crate::arp::{Arp, ArpEv};
use crate::env::EnvCoef;
use crate::fx::Fx;
use crate::ids::*;
use crate::params::{RindParams, Snapshot};
use crate::util::{limit, smooth_coef};
use crate::voice::{Step, Voice};

pub const MAX_VOICES: usize = 16;
/// The control step, in samples.
pub const STEP: usize = 16;

pub struct Engine {
    sr: f32,
    cur: Snapshot,
    primed: bool,
    voices: Vec<Voice>,
    arp: Arp,
    arp_on: bool,
    fx: Fx,
    age: u64,
    /// Held keys, oldest first, for mono mode.
    stack: [u8; 32],
    stack_n: usize,
    last_note: Option<f32>,
    pedal: bool,
    bend: f32,
    wheel: f32,
    touch: f32,
    lfo_phase: [f32; 2],
    master: f32,
}

impl Engine {
    pub fn new(sr: f32) -> Engine {
        Engine {
            sr,
            cur: RindParams::defaults(),
            primed: false,
            voices: (0..MAX_VOICES as u32).map(|i| Voice::new(i + 1)).collect(),
            arp: Arp::default(),
            arp_on: false,
            fx: Fx::new(sr),
            age: 0,
            stack: [0; 32],
            stack_n: 0,
            last_note: None,
            pedal: false,
            bend: 0.0,
            wheel: 0.0,
            touch: 0.0,
            lfo_phase: [0.0; 2],
            master: 0.0,
        }
    }

    /// Silences everything.
    pub fn reset(&mut self) {
        for v in &mut self.voices {
            v.kill();
        }
        self.arp = Arp::default();
        self.arp_on = false;
        self.stack_n = 0;
        self.last_note = None;
        self.pedal = false;
        self.fx = Fx::new(self.sr);
        self.primed = false;
    }

    pub fn active_voices(&self) -> usize {
        self.voices.iter().filter(|v| v.active).count()
    }

    // ---- MIDI ----

    pub fn note_on(&mut self, note: u8, vel: f32) {
        if self.cur[ARP_ON] >= 0.5 {
            let hold = self.cur[ARP_HOLD] >= 0.5;
            self.arp.note_on(note, vel, hold);
        } else {
            self.voice_on(note, vel, 1.0);
        }
    }

    pub fn note_off(&mut self, note: u8) {
        if self.cur[ARP_ON] >= 0.5 {
            let hold = self.cur[ARP_HOLD] >= 0.5;
            self.arp.note_off(note, hold);
        } else {
            self.voice_off(note);
        }
    }

    pub fn cc(&mut self, cc: u8, value: f32) {
        match cc {
            1 => self.wheel = value,
            64 => {
                let down = value >= 0.5;
                if self.pedal && !down {
                    for v in &mut self.voices {
                        if v.pedal && v.active {
                            v.pedal = false;
                            v.release();
                        }
                    }
                }
                self.pedal = down;
            }
            120 | 123 => self.all_notes_off(cc == 120),
            _ => {}
        }
    }

    pub fn pitch_bend(&mut self, norm: f32) {
        self.bend = (norm * 2.0 - 1.0).clamp(-1.0, 1.0);
    }

    pub fn pressure(&mut self, v: f32) {
        self.touch = v.clamp(0.0, 1.0);
    }

    pub fn all_notes_off(&mut self, hard: bool) {
        self.stack_n = 0;
        self.arp.clear();
        for v in &mut self.voices {
            if hard {
                v.kill();
            } else if v.active && v.gate {
                v.release();
            }
        }
    }

    fn push_key(&mut self, note: u8) {
        if let Some(i) = self.stack[..self.stack_n].iter().position(|&n| n == note) {
            self.stack.copy_within(i + 1..self.stack_n, i);
            self.stack_n -= 1;
        }
        if self.stack_n < self.stack.len() {
            self.stack[self.stack_n] = note;
            self.stack_n += 1;
        }
    }

    fn voice_on(&mut self, note: u8, vel: f32, step_vel: f32) {
        self.age += 1;
        let p = self.cur;
        let mono = p[MONO] >= 0.5;
        let porta_always = p[PORTA_MODE] < 0.5;
        let lfo_phase = self.lfo_phase;
        if mono {
            self.push_key(note);
            let v = &mut self.voices[0];
            if v.active && v.gate {
                // Legato: slide to the new note without restarting.
                v.retarget(note);
                v.age = self.age;
            } else {
                let from = if porta_always { self.last_note } else { None };
                v.start(&p, note, vel, step_vel, from, v.active, self.age, lfo_phase);
            }
            self.last_note = Some(note as f32);
            return;
        }
        let limit = (p[POLY] as usize).clamp(1, MAX_VOICES);
        let free = self.voices[..limit].iter().position(|v| !v.active);
        let (i, keep) = match free {
            Some(i) => (i, false),
            None => {
                // Take over the quietest released voice, else the oldest.
                let pick = self.voices[..limit]
                    .iter()
                    .enumerate()
                    .min_by(|(_, a), (_, b)| {
                        let ka = (
                            !a.is_released(),
                            if a.is_released() { a.level() } else { 0.0 },
                            a.age,
                        );
                        let kb = (
                            !b.is_released(),
                            if b.is_released() { b.level() } else { 0.0 },
                            b.age,
                        );
                        ka.partial_cmp(&kb).unwrap_or(std::cmp::Ordering::Equal)
                    })
                    .map(|(i, _)| i)
                    .unwrap_or(0);
                (pick, true)
            }
        };
        let from = if porta_always { self.last_note } else { None };
        self.voices[i].start(&p, note, vel, step_vel, from, keep, self.age, lfo_phase);
        self.last_note = Some(note as f32);
    }

    fn voice_off(&mut self, note: u8) {
        let pedal = self.pedal;
        if self.cur[MONO] >= 0.5 {
            if let Some(i) = self.stack[..self.stack_n].iter().position(|&n| n == note) {
                self.stack.copy_within(i + 1..self.stack_n, i);
                self.stack_n -= 1;
            }
            let v = &mut self.voices[0];
            if self.stack_n > 0 {
                if v.active && v.gate {
                    v.retarget(self.stack[self.stack_n - 1]);
                }
            } else if v.active && v.gate {
                if pedal {
                    v.pedal = true;
                } else {
                    v.release();
                }
            }
            return;
        }
        for v in &mut self.voices {
            if v.active && v.gate && v.note == note {
                if pedal {
                    v.pedal = true;
                } else {
                    v.release();
                }
            }
        }
    }

    // ---- Audio ----

    /// Renders `l.len()` samples from parameter snapshot `target` (what the sound should become), at `tempo` BPM.
    pub fn render(&mut self, target: &Snapshot, tempo: f32, l: &mut [f32], r: &mut [f32]) {
        if !self.primed {
            self.cur = *target;
            self.primed = true;
        }
        let n = l.len().min(r.len());
        l[..n].fill(0.0);
        r[..n].fill(0.0);
        let mut pos = 0;
        while pos < n {
            let end = (pos + STEP).min(n);
            self.step(target, tempo, &mut l[pos..end], &mut r[pos..end]);
            pos = end;
        }
        self.fx.process(&self.cur, tempo, &mut l[..n], &mut r[..n]);
        for s in l[..n].iter_mut().chain(r[..n].iter_mut()) {
            *s = limit(*s);
        }
    }

    fn step(&mut self, target: &Snapshot, tempo: f32, l: &mut [f32], r: &mut [f32]) {
        let n = l.len();
        let sr = self.sr;
        // Smooth the parameters towards the target (discrete ones follow at once).
        let k = smooth_coef(0.005, STEP as f32, sr);
        for (i, (c, t)) in self.cur.iter_mut().zip(target.iter()).enumerate() {
            if matches!(SPECS[i].kind, Kind::Int(..)) {
                *c = *t;
            } else {
                *c += (*t - *c) * k;
            }
        }
        let p = self.cur;

        // The arpeggiator turns held keys into note events.
        let arp_on = p[ARP_ON] >= 0.5;
        if arp_on != self.arp_on {
            self.arp_on = arp_on;
            let mut evs = [None; 16];
            let mut c = 0;
            self.arp.stop(&mut |e| {
                if c < 16 {
                    evs[c] = Some(e);
                    c += 1;
                }
            });
            for e in evs.into_iter().flatten() {
                if let ArpEv::Off { note } = e {
                    self.voice_off(note);
                }
            }
            if !arp_on {
                for v in &mut self.voices {
                    if v.active && v.gate {
                        v.release();
                    }
                }
                self.stack_n = 0;
            }
        }
        if arp_on {
            let mut evs = [None; 32];
            let mut c = 0;
            self.arp.process(n, &p, tempo, sr, &mut |e| {
                if c < 32 {
                    evs[c] = Some(e);
                    c += 1;
                }
            });
            for e in evs.into_iter().flatten() {
                match e {
                    ArpEv::On {
                        note,
                        vel,
                        step_vel,
                    } => self.voice_on(note, vel, step_vel),
                    ArpEv::Off { note } => self.voice_off(note),
                }
            }
        }

        // Shared per-step values.
        let env = |b: usize, rate: f32| {
            EnvCoef::new(
                p[b + env::ATT],
                p[b + env::DEC],
                p[b + env::SUS],
                p[b + env::REL],
                rate,
            )
        };
        let part_env = |part: usize| {
            let b = PART_BASE + part * PART_STRIDE;
            EnvCoef::new(
                p[b + part::ATT],
                p[b + part::DEC],
                p[b + part::SUS],
                p[b + part::REL],
                sr,
            )
        };
        let step_rate = sr / STEP as f32;
        let mut lfo_hz = [0.0f32; 2];
        for (i, hz) in lfo_hz.iter_mut().enumerate() {
            let b = LFO_BASE + i * LFO_STRIDE;
            *hz = if p[b + lfo::SYNC] >= 0.5 {
                tempo.max(20.0)
                    / 60.0
                    / DIV_BEATS[(p[b + lfo::DIV] as usize).min(DIV_BEATS.len() - 1)]
            } else {
                p[b + lfo::RATE]
            };
            self.lfo_phase[i] = (self.lfo_phase[i] + *hz * n as f32 / sr).fract();
        }
        let s = Step {
            p: &p,
            sr,
            amp: [part_env(0), part_env(1)],
            menv: [
                env(ENV_BASE, step_rate),
                env(ENV_BASE + ENV_STRIDE, step_rate),
            ],
            bend: self.bend,
            wheel: self.wheel,
            touch: self.touch,
            lfo_hz,
        };
        for v in &mut self.voices {
            if v.active {
                v.render(&s, l, r);
            }
        }
        // The master volume, smoothly.
        let target_gain = p[MASTER_VOL] * p[MASTER_VOL];
        let from = self.master;
        let inc = (target_gain - from) / n as f32;
        for i in 0..n {
            let g = from + inc * (i + 1) as f32;
            l[i] *= g;
            r[i] *= g;
        }
        self.master = target_gain;
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn defaults() -> Snapshot {
        RindParams::defaults()
    }

    fn play(e: &mut Engine, p: &Snapshot, blocks: usize) -> (Vec<f32>, Vec<f32>) {
        let (mut ol, mut or) = (Vec::new(), Vec::new());
        for _ in 0..blocks {
            let mut l = vec![0.0; 128];
            let mut r = vec![0.0; 128];
            e.render(p, 120.0, &mut l, &mut r);
            ol.extend(l);
            or.extend(r);
        }
        (ol, or)
    }

    fn rms(x: &[f32]) -> f32 {
        (x.iter().map(|s| s * s).sum::<f32>() / x.len().max(1) as f32).sqrt()
    }

    #[test]
    fn silent_until_a_note_plays() {
        let mut e = Engine::new(48_000.0);
        let (l, r) = play(&mut e, &defaults(), 20);
        assert!(l.iter().chain(&r).all(|s| *s == 0.0));
    }

    #[test]
    fn a_note_sounds_and_dies_out() {
        let p = defaults();
        let mut e = Engine::new(48_000.0);
        e.render(&p, 120.0, &mut [0.0; 16], &mut [0.0; 16]);
        e.note_on(60, 1.0);
        let (l, r) = play(&mut e, &p, 50);
        assert!(rms(&l) > 0.01 && rms(&r) > 0.01, "{} {}", rms(&l), rms(&r));
        assert!(l.iter().chain(&r).all(|s| s.is_finite() && s.abs() <= 1.0));
        e.note_off(60);
        let (l, _) = play(&mut e, &p, 400);
        assert!(rms(&l[l.len() - 2000..]) < 1e-4);
        assert_eq!(e.active_voices(), 0);
    }

    #[test]
    fn polyphony_limit_takes_over_voices() {
        let mut p = defaults();
        p[POLY] = 3.0;
        let mut e = Engine::new(48_000.0);
        play(&mut e, &p, 1);
        for n in 60..70 {
            e.note_on(n, 0.8);
            play(&mut e, &p, 2);
        }
        assert!(e.active_voices() <= 3);
    }

    #[test]
    fn mono_legato_keeps_one_voice() {
        let mut p = defaults();
        p[MONO] = 1.0;
        let mut e = Engine::new(48_000.0);
        play(&mut e, &p, 1);
        e.note_on(60, 1.0);
        e.note_on(64, 1.0);
        play(&mut e, &p, 4);
        assert_eq!(e.active_voices(), 1);
        e.note_off(64);
        play(&mut e, &p, 4);
        assert_eq!(e.active_voices(), 1, "back to the first held key");
    }

    #[test]
    fn unison_and_both_parts_are_louder_than_one_voice() {
        let p = defaults();
        let mut e = Engine::new(48_000.0);
        play(&mut e, &p, 1);
        e.note_on(48, 1.0);
        let one = rms(&play(&mut e, &p, 40).0);
        let mut q = defaults();
        q[OSC_BASE + osc::VOICES] = 8.0;
        q[OSC_BASE + OSC_STRIDE + osc::VOICES] = 4.0;
        let mut e2 = Engine::new(48_000.0);
        play(&mut e2, &q, 1);
        e2.note_on(48, 1.0);
        let many = rms(&play(&mut e2, &q, 40).0);
        assert!(many > one, "{many} vs {one}");
    }

    #[test]
    fn modulation_matrix_moves_the_filter() {
        let mut p = defaults();
        p[FILT_BASE + filt::CUTOFF] = 80.0;
        p[OSC_BASE + osc::WAVE] = 1.0;
        let mut e = Engine::new(48_000.0);
        play(&mut e, &p, 1);
        e.note_on(60, 1.0);
        let closed = rms(&play(&mut e, &p, 40).0);
        // Velocity opens the cutoff by up to five octaves, far above the closed filter.
        let mut q = p;
        q[MAT_BASE + mat::SRC] = 5.0;
        q[MAT_BASE + mat::DST] = 1.0;
        q[MAT_BASE + mat::AMT] = 1.0;
        let mut e2 = Engine::new(48_000.0);
        play(&mut e2, &q, 1);
        e2.note_on(60, 1.0);
        let open = rms(&play(&mut e2, &q, 40).0);
        assert!(open > closed * 1.5, "{open} vs {closed}");
    }

    #[test]
    fn the_arpeggiator_plays_notes_in_time() {
        let mut p = defaults();
        p[ARP_ON] = 1.0;
        let mut e = Engine::new(48_000.0);
        play(&mut e, &p, 1);
        e.note_on(60, 1.0);
        e.note_on(64, 1.0);
        let (l, _) = play(&mut e, &p, 200);
        assert!(rms(&l) > 0.005);
        e.note_off(60);
        e.note_off(64);
        play(&mut e, &p, 800);
        assert_eq!(e.active_voices(), 0);
    }

    #[test]
    fn every_effect_on_stays_finite_with_a_chord() {
        let mut p = defaults();
        for on in [DIST_ON, PHS_ON, CHO_ON, EQ_ON, DLY_ON, REV_ON, CMP_ON] {
            p[on] = 1.0;
        }
        let mut e = Engine::new(44_100.0);
        play(&mut e, &p, 1);
        for n in [48, 55, 60, 64] {
            e.note_on(n, 1.0);
        }
        let (l, r) = play(&mut e, &p, 200);
        assert!(l.iter().chain(&r).all(|s| s.is_finite() && s.abs() <= 1.0));
    }
}
