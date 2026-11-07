// Copyright (C) 2026 The Honeydew contributors
// SPDX-License-Identifier: GPL-3.0-or-later

//! The master effects, in series: distortion, phaser, chorus, equalizer, delay, reverb and compressor. All buffers are
//! allocated when the plugin is set up ([`Fx::new`]); processing never allocates.

use std::f32::consts::PI;

use crate::ids::*;
use crate::params::Snapshot;
use crate::util::{Biquad, OnePole, clean, db_to_gain, fast_sin, soft};

fn on(p: &Snapshot, i: usize) -> bool {
    p[i] >= 0.5
}

fn mix(dry: f32, wet: f32, m: f32) -> f32 {
    dry + (wet - dry) * m
}

/// A ring buffer with fractional reads.
#[derive(Clone, Debug)]
struct Ring {
    buf: Vec<f32>,
    pos: usize,
}

impl Ring {
    fn new(len: usize) -> Ring {
        Ring {
            buf: vec![0.0; len.max(4)],
            pos: 0,
        }
    }

    fn clear(&mut self) {
        self.buf.fill(0.0);
    }

    fn push(&mut self, x: f32) {
        self.pos += 1;
        if self.pos >= self.buf.len() {
            self.pos = 0;
        }
        self.buf[self.pos] = x;
    }

    /// The sample `d` samples back (`d >= 1`), linearly interpolated.
    fn read(&self, d: f32) -> f32 {
        let len = self.buf.len();
        let d = d.clamp(1.0, (len - 2) as f32);
        let i = d as usize;
        let frac = d - i as f32;
        let at = |k: usize| self.buf[(self.pos + len - k) % len];
        at(i) + (at(i + 1) - at(i)) * frac
    }
}

// ---- Distortion ----

#[derive(Clone, Debug, Default)]
struct Distortion {
    up: [[Biquad; 2]; 2],
    down: [[Biquad; 2]; 2],
    tone: [OnePole; 2],
    hold: [f32; 2],
    count: [f32; 2],
    was_on: bool,
}

fn shape(kind: i32, x: f32) -> f32 {
    match kind {
        0 => soft(x),
        1 => 4.0 * ((x + 1.0) * 0.25 - ((x + 1.0) * 0.25).round()).abs() - 1.0,
        _ => x.clamp(-1.0, 1.0),
    }
}

impl Distortion {
    fn new(sr: f32) -> Distortion {
        let mut d = Distortion::default();
        for ch in 0..2 {
            for (k, q) in [0.5412, 1.3066].into_iter().enumerate() {
                d.up[ch][k].low_pass(0.44 * sr, q, 2.0 * sr);
                d.down[ch][k].low_pass(0.44 * sr, q, 2.0 * sr);
            }
        }
        d
    }

    fn process(&mut self, p: &Snapshot, sr: f32, l: &mut [f32], r: &mut [f32]) {
        if !on(p, DIST_ON) {
            self.was_on = false;
            return;
        }
        if !self.was_on {
            self.was_on = true;
            *self = Distortion {
                was_on: true,
                ..Distortion::new(sr)
            };
        }
        let kind = p[DIST_TYPE] as i32;
        let drive = p[DIST_DRIVE];
        let gain = 1.0 + drive * 30.0;
        let comp = 1.0 / (1.0 + 0.6 * drive);
        let m = p[DIST_MIX];
        let tone_k = OnePole::coef(800.0 * 25f32.powf(p[DIST_TONE]), sr);
        let bits = 16.0 - drive * 14.0;
        let step = 2f32.powf(1.0 - bits);
        let factor = 1.0 + drive * 31.0;
        for (ch, buf) in [l, r].into_iter().enumerate() {
            for s in buf.iter_mut() {
                let dry = *s;
                let wet = match kind {
                    0..=2 => {
                        let x = dry * gain;
                        let a = shape(kind, pair(&mut self.up[ch], 2.0 * x));
                        let b = shape(kind, pair(&mut self.up[ch], 0.0));
                        pair(&mut self.down[ch], a);
                        pair(&mut self.down[ch], b) * comp
                    }
                    3 => {
                        self.count[ch] += 1.0;
                        if self.count[ch] >= factor {
                            self.count[ch] -= factor;
                            self.hold[ch] = dry;
                        }
                        self.hold[ch] * (1.0 + drive * 0.3)
                    }
                    _ => (dry / step).round() * step,
                };
                let wet = self.tone[ch].lp(wet, tone_k);
                *s = clean(mix(dry, wet, m));
            }
        }
    }
}

// ---- Phaser ----

#[derive(Clone, Debug, Default)]
struct Phaser {
    z: [[f32; 6]; 2],
    fb: [f32; 2],
    phase: f32,
    was_on: bool,
}

impl Phaser {
    fn process(&mut self, p: &Snapshot, sr: f32, l: &mut [f32], r: &mut [f32]) {
        if !on(p, PHS_ON) {
            self.was_on = false;
            return;
        }
        if !self.was_on {
            *self = Phaser {
                was_on: true,
                ..Phaser::default()
            };
        }
        let inc = p[PHS_RATE] / sr;
        let (depth, freq, fb, m) = (p[PHS_DEPTH] * 3.0, p[PHS_FREQ], p[PHS_FB], p[PHS_MIX]);
        let spread = p[PHS_SPREAD] * 0.5;
        for i in 0..l.len() {
            for (ch, buf) in [&mut *l, &mut *r].into_iter().enumerate() {
                let lfo = fast_sin(self.phase + if ch == 1 { spread } else { 0.0 });
                let fc = (freq * (depth * lfo).exp2()).clamp(20.0, 0.45 * sr);
                let t = (PI * fc / sr).tan();
                let a = (t - 1.0) / (t + 1.0);
                let dry = buf[i];
                let mut x = dry + fb * self.fb[ch];
                for z in &mut self.z[ch] {
                    let y = a * x + *z;
                    *z = x - a * y;
                    x = y;
                }
                self.fb[ch] = clean(x);
                buf[i] = clean(mix(dry, 0.5 * (dry + x), m * 2.0).clamp(-8.0, 8.0));
            }
            self.phase += inc;
            if self.phase >= 1.0 {
                self.phase -= 1.0;
            }
        }
    }
}

// ---- Chorus ----

#[derive(Clone, Debug)]
struct Chorus {
    line: [Ring; 2],
    phase: f32,
    was_on: bool,
}

impl Chorus {
    fn new(sr: f32) -> Chorus {
        let len = (sr * 0.09) as usize;
        Chorus {
            line: [Ring::new(len), Ring::new(len)],
            phase: 0.0,
            was_on: false,
        }
    }

    fn process(&mut self, p: &Snapshot, sr: f32, l: &mut [f32], r: &mut [f32]) {
        if !on(p, CHO_ON) {
            self.was_on = false;
            return;
        }
        if !self.was_on {
            self.was_on = true;
            self.line.iter_mut().for_each(Ring::clear);
        }
        let inc = p[CHO_RATE] / sr;
        let base = p[CHO_DELAY] * 0.001 * sr;
        let depth = p[CHO_DEPTH] * 0.9;
        let (fb, m) = (p[CHO_FB], p[CHO_MIX]);
        // Four voices: each channel two taps, a quarter turn from the other channel's.
        let offsets = [[0.0, 0.5], [0.25, 0.75]];
        for i in 0..l.len() {
            for (ch, buf) in [&mut *l, &mut *r].into_iter().enumerate() {
                let dry = buf[i];
                let mut wet = 0.0;
                for off in offsets[ch] {
                    let d = base * (1.0 + depth * fast_sin(self.phase + off));
                    wet += self.line[ch].read(d);
                }
                wet *= 0.5;
                self.line[ch].push(clean(dry + fb * wet));
                buf[i] = clean(mix(dry, wet, m));
            }
            self.phase += inc;
            if self.phase >= 1.0 {
                self.phase -= 1.0;
            }
        }
    }
}

// ---- Equalizer ----

#[derive(Clone, Debug, Default)]
struct Eq {
    bass: [Biquad; 2],
    treble: [Biquad; 2],
    key: [f32; 4],
}

impl Eq {
    fn process(&mut self, p: &Snapshot, sr: f32, l: &mut [f32], r: &mut [f32]) {
        if !on(p, EQ_ON) {
            return;
        }
        let key = [p[EQ_BASS_F], p[EQ_BASS_G], p[EQ_TREB_F], p[EQ_TREB_G]];
        if key != self.key {
            self.key = key;
            for ch in 0..2 {
                self.bass[ch].shelf(false, key[0], key[1], sr);
                self.treble[ch].shelf(true, key[2], key[3], sr);
            }
        }
        for (ch, buf) in [l, r].into_iter().enumerate() {
            for s in buf.iter_mut() {
                *s = clean(self.treble[ch].tick(self.bass[ch].tick(*s)));
            }
        }
    }
}

// ---- Delay ----

#[derive(Clone, Debug, Default)]
struct Allpass {
    buf: Vec<f32>,
    pos: usize,
}

impl Allpass {
    fn new(len: usize) -> Allpass {
        Allpass {
            buf: vec![0.0; len.max(2)],
            pos: 0,
        }
    }

    fn tick(&mut self, x: f32, g: f32) -> f32 {
        let d = self.buf[self.pos];
        let y = d - g * x;
        self.buf[self.pos] = x + g * y;
        self.pos += 1;
        if self.pos >= self.buf.len() {
            self.pos = 0;
        }
        y
    }

    fn clear(&mut self) {
        self.buf.fill(0.0);
    }
}

#[derive(Clone, Debug)]
struct Delay {
    line: [Ring; 2],
    time: [f32; 2],
    lp: [OnePole; 2],
    hp: [OnePole; 2],
    smear: [[Allpass; 2]; 2],
    was_on: bool,
}

impl Delay {
    fn new(sr: f32) -> Delay {
        let len = (sr * 2.0) as usize + 8;
        let ap = |n: f32| Allpass::new((sr * n) as usize);
        Delay {
            line: [Ring::new(len), Ring::new(len)],
            time: [0.3 * sr; 2],
            lp: Default::default(),
            hp: Default::default(),
            smear: [[ap(0.0071), ap(0.0113)], [ap(0.0079), ap(0.0127)]],
            was_on: false,
        }
    }

    fn process(&mut self, p: &Snapshot, sr: f32, tempo: f32, l: &mut [f32], r: &mut [f32]) {
        if !on(p, DLY_ON) {
            self.was_on = false;
            return;
        }
        if !self.was_on {
            self.was_on = true;
            self.line.iter_mut().for_each(Ring::clear);
            self.smear.iter_mut().flatten().for_each(Allpass::clear);
            self.lp.iter_mut().for_each(OnePole::reset);
            self.hp.iter_mut().for_each(OnePole::reset);
        }
        let secs = |sync: bool, time: f32, div: f32| {
            let s = if sync {
                DIV_BEATS[(div as usize).min(DIV_BEATS.len() - 1)] * 60.0 / tempo.max(20.0)
            } else {
                time
            };
            s.clamp(0.005, 1.99) * sr
        };
        let sync = on(p, DLY_SYNC);
        let target = [
            secs(sync, p[DLY_L], p[DLY_DIV_L]),
            secs(sync, p[DLY_R], p[DLY_DIV_R]),
        ];
        let (fb, m, pp) = (p[DLY_FB], p[DLY_MIX], on(p, DLY_PP));
        let lp_k = OnePole::coef(p[DLY_LP], sr);
        let hp_k = OnePole::coef(p[DLY_HP], sr);
        let smear = p[DLY_SMEAR] * 0.6;
        for i in 0..l.len() {
            let mut out = [0.0; 2];
            for ch in 0..2 {
                self.time[ch] += (target[ch] - self.time[ch]) * 0.001;
                out[ch] = self.line[ch].read(self.time[ch]);
            }
            for (ch, buf) in [&mut *l, &mut *r].into_iter().enumerate() {
                let dry = buf[i];
                let src = if pp { out[1 - ch] } else { out[ch] };
                let mut f = self.lp[ch].lp(src, lp_k);
                f -= self.hp[ch].lp(f, hp_k);
                if smear > 0.0 {
                    f = self.smear[ch][0].tick(f, smear);
                    f = self.smear[ch][1].tick(f, smear);
                }
                self.line[ch].push(clean(dry + fb * f));
                buf[i] = clean(dry + m * out[ch]);
            }
        }
    }
}

// ---- Reverb ----

const COMBS: [usize; 8] = [1116, 1188, 1277, 1356, 1422, 1491, 1557, 1617];
const ALLPASSES: [usize; 4] = [556, 441, 341, 225];
const SPREAD: usize = 23;

#[derive(Clone, Debug)]
struct Comb {
    buf: Vec<f32>,
    pos: usize,
    filt: f32,
}

impl Comb {
    fn new(len: usize) -> Comb {
        Comb {
            buf: vec![0.0; len.max(2)],
            pos: 0,
            filt: 0.0,
        }
    }

    fn tick(&mut self, x: f32, fb: f32, damp: f32) -> f32 {
        let out = self.buf[self.pos];
        self.filt = out * (1.0 - damp) + self.filt * damp;
        self.buf[self.pos] = clean(x + self.filt * fb);
        self.pos += 1;
        if self.pos >= self.buf.len() {
            self.pos = 0;
        }
        out
    }

    fn clear(&mut self) {
        self.buf.fill(0.0);
        self.filt = 0.0;
    }
}

#[derive(Clone, Debug)]
struct Reverb {
    combs: [Vec<Comb>; 2],
    aps: [Vec<Allpass>; 2],
    pre: Ring,
    was_on: bool,
}

impl Reverb {
    fn new(sr: f32) -> Reverb {
        let k = sr / 44_100.0;
        let sized = |n: usize, extra: usize| ((n + extra) as f32 * k) as usize;
        let side = |extra: usize| {
            (
                COMBS
                    .iter()
                    .map(|&n| Comb::new(sized(n, extra)))
                    .collect::<Vec<_>>(),
                ALLPASSES
                    .iter()
                    .map(|&n| Allpass::new(sized(n, extra)))
                    .collect::<Vec<_>>(),
            )
        };
        let (cl, al) = side(0);
        let (cr, ar) = side(SPREAD);
        Reverb {
            combs: [cl, cr],
            aps: [al, ar],
            pre: Ring::new((sr * 0.26) as usize),
            was_on: false,
        }
    }

    fn process(&mut self, p: &Snapshot, sr: f32, l: &mut [f32], r: &mut [f32]) {
        if !on(p, REV_ON) {
            self.was_on = false;
            return;
        }
        if !self.was_on {
            self.was_on = true;
            self.pre.clear();
            self.combs.iter_mut().flatten().for_each(Comb::clear);
            self.aps.iter_mut().flatten().for_each(Allpass::clear);
        }
        let fb = 0.7 + p[REV_SIZE] * 0.28;
        let damp = p[REV_DAMP] * 0.4;
        let m = p[REV_MIX];
        let pre = (p[REV_PRE] * sr).max(1.0);
        let w1 = 0.5 * (1.0 + p[REV_WIDTH]);
        let w2 = 0.5 * (1.0 - p[REV_WIDTH]);
        for i in 0..l.len() {
            let (dl, dr) = (l[i], r[i]);
            self.pre.push((dl + dr) * 0.015);
            let input = self.pre.read(pre);
            let mut wet = [0.0f32; 2];
            for ch in 0..2 {
                let mut sum = 0.0;
                for c in &mut self.combs[ch] {
                    sum += c.tick(input, fb, damp);
                }
                for a in &mut self.aps[ch] {
                    sum = a.tick(sum, -0.5);
                }
                wet[ch] = sum;
            }
            l[i] = clean(dl + m * 3.0 * (wet[0] * w1 + wet[1] * w2));
            r[i] = clean(dr + m * 3.0 * (wet[1] * w1 + wet[0] * w2));
        }
    }
}

// ---- Compressor ----

#[derive(Clone, Debug, Default)]
struct Compressor {
    env: f32,
    gain_db: f32,
}

impl Compressor {
    fn process(&mut self, p: &Snapshot, sr: f32, l: &mut [f32], r: &mut [f32]) {
        if !on(p, CMP_ON) {
            self.env = 0.0;
            self.gain_db = 0.0;
            return;
        }
        let (thr, ratio) = (p[CMP_THR], p[CMP_RATIO].max(1.0));
        let att = (-1.0 / (p[CMP_ATT] * 0.001 * sr).max(1.0)).exp();
        let rel = (-1.0 / (p[CMP_REL] * 0.001 * sr).max(1.0)).exp();
        let makeup = db_to_gain(p[CMP_GAIN]);
        let knee = 6.0;
        for i in 0..l.len() {
            let level = l[i].abs().max(r[i].abs());
            let c = if level > self.env { att } else { rel };
            self.env = level + (self.env - level) * c;
            let db = 20.0 * (self.env + 1e-9).log10();
            let over = db - thr;
            let target = if over <= -knee * 0.5 {
                0.0
            } else if over >= knee * 0.5 {
                -over * (1.0 - 1.0 / ratio)
            } else {
                let x = over + knee * 0.5;
                -(1.0 - 1.0 / ratio) * x * x / (2.0 * knee)
            };
            self.gain_db += (target - self.gain_db) * 0.01_f32.max(1.0 - rel);
            let g = db_to_gain(self.gain_db) * makeup;
            l[i] = clean(l[i] * g);
            r[i] = clean(r[i] * g);
        }
    }
}

/// The master effects chain.
/// `x` through two biquads in series.
fn pair(f: &mut [Biquad; 2], x: f32) -> f32 {
    let t = f[0].tick(x);
    f[1].tick(t)
}

pub struct Fx {
    sr: f32,
    dist: Distortion,
    phaser: Phaser,
    chorus: Chorus,
    eq: Eq,
    delay: Delay,
    reverb: Reverb,
    comp: Compressor,
}

impl Fx {
    pub fn new(sr: f32) -> Fx {
        Fx {
            sr,
            dist: Distortion::new(sr),
            phaser: Phaser::default(),
            chorus: Chorus::new(sr),
            eq: Eq::default(),
            delay: Delay::new(sr),
            reverb: Reverb::new(sr),
            comp: Compressor::default(),
        }
    }

    pub fn process(&mut self, p: &Snapshot, tempo: f32, l: &mut [f32], r: &mut [f32]) {
        let sr = self.sr;
        self.dist.process(p, sr, l, r);
        self.phaser.process(p, sr, l, r);
        self.chorus.process(p, sr, l, r);
        self.eq.process(p, sr, l, r);
        self.delay.process(p, sr, tempo, l, r);
        self.reverb.process(p, sr, l, r);
        self.comp.process(p, sr, l, r);
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::params::RindParams;

    fn burst(p: &Snapshot, blocks: usize) -> (Vec<f32>, Vec<f32>) {
        let mut fx = Fx::new(48_000.0);
        let (mut ol, mut or) = (Vec::new(), Vec::new());
        for b in 0..blocks {
            let mut l: Vec<f32> = (0..256)
                .map(|i| {
                    if b == 0 && i < 64 {
                        ((i as f32) * 0.3).sin() * 0.5
                    } else {
                        0.0
                    }
                })
                .collect();
            let mut r = l.clone();
            fx.process(p, 120.0, &mut l, &mut r);
            ol.extend(l);
            or.extend(r);
        }
        (ol, or)
    }

    fn with(on: usize) -> Snapshot {
        let mut p = RindParams::defaults();
        p[on] = 1.0;
        p
    }

    #[test]
    fn every_effect_is_finite_and_bounded() {
        for on in [DIST_ON, PHS_ON, CHO_ON, EQ_ON, DLY_ON, REV_ON, CMP_ON] {
            let mut p = with(on);
            p[DLY_FB] = 0.9;
            p[PHS_FB] = 0.9;
            for kind in 0..5 {
                p[DIST_TYPE] = kind as f32;
                let (l, r) = burst(&p, 200);
                assert!(
                    l.iter().chain(&r).all(|s| s.is_finite() && s.abs() < 20.0),
                    "effect {on} kind {kind}"
                );
            }
        }
    }

    #[test]
    fn off_effects_do_nothing() {
        let p = RindParams::defaults();
        let (l, _) = burst(&p, 2);
        assert!(l[..64].iter().any(|s| s.abs() > 0.1));
        assert!(l[64..256].iter().all(|s| *s == 0.0));
    }

    #[test]
    fn delay_and_reverb_ring_on() {
        for on in [DLY_ON, REV_ON] {
            let (l, _) = burst(&with(on), 100);
            let tail: f32 = l[256 * 20..].iter().map(|s| s.abs()).sum();
            assert!(tail > 0.01, "effect {on} tail {tail}");
        }
    }

    #[test]
    fn compressor_lowers_a_loud_signal() {
        let mut p = with(CMP_ON);
        p[CMP_THR] = -30.0;
        p[CMP_RATIO] = 10.0;
        let mut fx = Fx::new(48_000.0);
        let mut l = vec![0.9; 48_000];
        let mut r = l.clone();
        fx.process(&p, 120.0, &mut l, &mut r);
        assert!(l[47_999] < 0.5, "{}", l[47_999]);
    }
}
