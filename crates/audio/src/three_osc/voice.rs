//! One 3x Osc voice: osc1 written, osc2 added, osc3 added or AM'd → filter →
//! amp. Pitch, mix, biquad and SVF values step per control tick; the amp gains and the Fast LP f ramp within it.

use std::f32::consts::FRAC_PI_2;

use super::filter::{self, Coefs, Filter, FilterType};
use super::osc::{self, Shape, Wave};
use super::{ModFrame, NoteSpec, Params, Target, VoiceMod, coarse, toggle};

/// The amp and Fast LP slope limit, per sample: 44.1/fs (full scale ≥ 22.7 ms).
const SLOPE_PER_SAMPLE_AT_1HZ: f32 = 44.1;

/// What every voice reads at a control tick.
pub(super) struct Ctl<'a> {
    pub p: &'a Params,
    pub fs: f32,
    /// Pitch Bend in semitones.
    pub wheel: f32,
    /// This tick's length in samples.
    pub n: f32,
    /// Samples since construction or reset (Free run).
    pub samples: u64,
}

/// A value that ramps linearly within a tick, its slope limited; the first tick jumps.
#[derive(Clone, Copy, Debug, Default)]
pub(super) struct Ramp {
    value: f32,
    step: f32,
    target: f32,
    /// The last ramp was not slope-limited, so it ended on its target.
    exact: bool,
}

impl Ramp {
    fn set(&mut self, target: f32, n: f32, slope: f32, jump: bool) {
        if self.exact || jump {
            self.value = if jump { target } else { self.target };
        }
        let d = (target - self.value) / n;
        self.step = d.clamp(-slope, slope);
        self.exact = self.step == d;
        self.target = target;
    }

    #[inline]
    fn next(&mut self) -> f32 {
        self.value += self.step;
        self.value
    }

    fn silent(&self) -> bool {
        self.value == 0.0 && self.target == 0.0
    }
}

/// Pitch (semitones), velocity and note pan, moving linearly from `from` to `to` over `len` ticks.
#[derive(Clone, Copy, Debug, Default)]
pub(super) struct Glide {
    from: [f32; 3],
    to: [f32; 3],
    k: f32,
    len: f32,
}

impl Glide {
    fn value(&self) -> [f32; 3] {
        if self.k >= self.len {
            return self.to;
        }
        let t = self.k / self.len;
        [0, 1, 2].map(|i| self.from[i] + (self.to[i] - self.from[i]) * t)
    }

    /// Glides from where it is now to `note` over `ticks`.
    pub(super) fn to(&mut self, note: &NoteSpec, ticks: f32) {
        self.from = self.value();
        self.to = target(note);
        self.k = 0.0;
        self.len = ticks;
    }
}

/// A note's glide values: pitch (key + note pitch), velocity and pan.
pub(super) fn target(note: &NoteSpec) -> [f32; 3] {
    [f32::from(note.key) + note.pitch, note.velocity, note.pan]
}

#[derive(Clone, Debug, Default)]
pub(super) struct Voice<M> {
    pub active: bool,
    /// Not released (note-off, Gate) or cut.
    pub held: bool,
    /// Cut by the polyphony limit: ramping down to be freed.
    pub cut: bool,
    pub tag: u32,
    /// Start order (the cut rule takes the most recent).
    pub order: u64,
    /// Gate: ticks left before the release; 0 = no gate.
    pub gate: u32,
    pub glide: Glide,
    /// The note's echo filter offsets.
    pub note: NoteSpec,
    /// Slide offsets: semitones, pan, velocity; a started voice has none.
    pub slide: [f32; 3],
    pub m: M,
    /// Started since the last control tick: Free run, the amp and Fast LP jump.
    fresh: bool,
    pub shape: [Shape; 3],
    /// Per osc, per channel (left, right).
    phase: [[f64; 2]; 3],
    step: [[f64; 2]; 3],
    level: [[Option<u8>; 2]; 3],
    gain: [[f32; 2]; 3],
    am: bool,
    coefs: Coefs,
    filter: Filter,
    fast_f: Ramp,
    amp: [Ramp; 2],
}

impl<M: VoiceMod> Voice<M> {
    /// Starts `note` in this slot. `porta` = (from [pitch, velocity, pan], ticks). `rand` draws uniformly from 0..1.
    #[allow(clippy::too_many_arguments)]
    pub(super) fn start(
        &mut self,
        tag: u32,
        note: NoteSpec,
        order: u64,
        porta: Option<([f32; 3], f32)>,
        gate: u32,
        p: &Params,
        global: &M::Global,
        rand: &mut impl FnMut() -> f64,
    ) {
        *self = Self {
            active: true,
            held: true,
            fresh: true,
            tag,
            order,
            gate,
            note,
            ..Self::default()
        };
        let to = target(&note);
        self.glide = Glide {
            from: to,
            to,
            k: 0.0,
            len: 0.0,
        };
        if let Some((from, ticks)) = porta {
            self.glide.from = from;
            self.glide.len = ticks;
        }
        // Left = U(0, PR), right = Phase + U(0, PR), drawn independently; Phase = (2v − 1)/2 cycle mod 1.
        let pr = f64::from(p[26]);
        for i in 0..3 {
            let phase = ((2.0 * f64::from(p[7 + i]) - 1.0) / 2.0).rem_euclid(1.0);
            self.phase[i] = [rand() * pr, (phase + rand() * pr).rem_euclid(1.0)];
        }
        self.m.start(p, global);
    }

    pub(super) fn release(&mut self, p: &Params) {
        if self.held {
            self.held = false;
            self.m.release(p);
        }
    }

    pub(super) fn cut(&mut self) {
        self.held = false;
        self.cut = true;
        self.m.cut();
    }

    /// One control tick: pitch, mix, filter and amp from the params and this tick's modulation. Returns whether
    /// the voice is finished (its modulation is done and its amp has reached 0).
    pub(super) fn control(&mut self, c: &Ctl, f: &ModFrame) -> bool {
        let p = c.p;
        let (env, lfo) = (|t: Target| f.env[t as usize], |t: Target| f.lfo[t as usize]);
        let [base, vel, pan] = self.glide.value();
        self.glide.k += 1.0;
        // Slides add to the pitch (tracking included), the unclamped velocity and the note pan.
        let [base, pan, vel] = [
            base + self.slide[0],
            pan + self.slide[1],
            vel + self.slide[2],
        ];
        // Key, note pitch (echo, arp, Fat Mode) and glide, plus the wheel: the tracking pitch, which leaves
        // out the oscs' Coarse/Fine/Detune and the Pitch env and LFO.
        let key_pitch = base + c.wheel;
        let pitch = key_pitch
            + 48.0 * ((p[109] - 0.5) * env(Target::Pitch) + (p[56] - 0.5) * lfo(Target::Pitch));
        // Tracking: key term = key pitch − 120·Middle, amounts (2v − 1)/12 per semitone; velocity term =
        // vel − 0.1(e^(2.248·Middle) − 1), amounts 4v − 2.
        let key_term = key_pitch - 120.0 * p[121];
        let vel_term = vel - 0.1 * (2.248 * p[125]).exp_m1();
        let track = |key: usize, velocity: usize| {
            (2.0 * p[key] - 1.0) / 12.0 * key_term + (4.0 * p[velocity] - 2.0) * vel_term
        };

        // Oscillators and the mix.
        let (m2, m3) = (p[22], p[23]);
        self.am = toggle(p[24]);
        let mix = if self.am {
            [1.0 - m2, m2, m3]
        } else {
            [(1.0 - m3) * (1.0 - m2), (1.0 - m3) * m2, m3]
        };
        let hq = toggle(p[25]);
        for i in 0..3 {
            let shape = Shape::from_param(p[1 + i]);
            let coarse = coarse(p[13 + i]);
            let fine = 2.0 * p[16 + i] - 1.0;
            let osc_pitch = pitch + coarse + fine;
            for ch in 0..2 {
                // The right channel adds Detune (v − 0.5 st); the left has none.
                let detune = if ch == 1 { p[10 + i] - 0.5 } else { 0.0 };
                let hz = 440.0 * ((osc_pitch + detune - 69.0) / 12.0).exp2();
                let step = if shape == Shape::Noise {
                    osc::NOISE_STEP
                } else {
                    f64::from(hz) / f64::from(c.fs)
                };
                self.step[i][ch] = step;
                self.level[i][ch] = (hq && shape.has_mips() && step > osc::MIP_MIN_STEP)
                    .then(|| osc::mip_level(hz) as u8);
                if self.fresh && toggle(p[146 + i]) {
                    // Free run: + (samples since reset)·step.
                    self.phase[i][ch] = (self.phase[i][ch] + c.samples as f64 * step).fract();
                }
            }
            // p < 1 silences the osc (Noise ignores pitch). Invert negates the gain. Osc pan q = 2v − 1:
            // L × cos((q + 1)π/4), R × sin((q + 1)π/4).
            let silent = osc_pitch < 1.0 && shape != Shape::Noise;
            let invert = if toggle(p[4 + i]) { -1.0 } else { 1.0 };
            let g = if silent { 0.0 } else { mix[i] * invert };
            let angle = p[19 + i] * FRAC_PI_2;
            self.gain[i] = [g * angle.cos(), g * angle.sin()];
            self.shape[i] = shape;
        }

        // Filter: c = Cutoff + (2·CutOff − 1) + echo offset + mods + tracking; r the same.
        let cut = p[111]
            + (2.0 * p[115] - 1.0)
            + self.note.cut
            + 2.0 * (p[107] - 0.5) * env(Target::Cut)
            + 2.0 * (p[54] - 0.5) * lfo(Target::Cut)
            + track(123, 127);
        let res = p[112]
            + (2.0 * p[116] - 1.0)
            + self.note.res
            + 2.0 * (p[108] - 0.5) * env(Target::Res)
            + 2.0 * (p[55] - 0.5) * lfo(Target::Res)
            + track(124, 128);
        self.coefs = Coefs::new(FilterType::from_param(p[110]), cut, res, c.fs);
        let slope = SLOPE_PER_SAMPLE_AT_1HZ / c.fs;
        self.fast_f
            .set(filter::fast_lp_f(cut, c.fs), c.n, slope, self.fresh);

        // Amp: A = volEnv·(1 + 2(VolLfoAmt − 0.5)·lfo)·2(VolEnvAmt − 0.5)·velocity·2·Volume. The velocity is the glided
        // one, unclamped; velocity tracking never reaches the level.
        let a = env(Target::Vol)
            * (1.0 + 2.0 * (p[53] - 0.5) * lfo(Target::Vol))
            * 2.0
            * (p[106] - 0.5)
            * vel
            * 2.0
            * p[114];
        // P = (2·PanOffset − 1) + 2(notePan − 0.5) + Pan LFO + tracking, clamped ±1; negative gains → 0.
        let pan = ((2.0 * p[113] - 1.0)
            + 2.0 * (pan - 0.5)
            + 2.0 * (p[52] - 0.5) * lfo(Target::Pan)
            + track(122, 126))
        .clamp(-1.0, 1.0);
        let angle = (pan + 1.0) * FRAC_PI_2 / 2.0;
        let gains = [(a * angle.cos()).max(0.0), (a * angle.sin()).max(0.0)];
        for (ramp, gain) in self.amp.iter_mut().zip(gains) {
            ramp.set(gain, c.n, slope, self.fresh);
        }
        self.fresh = false;
        f.done && self.amp.iter().all(Ramp::silent)
    }

    /// Adds this voice into `out_l` / `out_r` (at most one tick).
    pub(super) fn render(&mut self, waves: [&Wave; 3], out_l: &mut [f32], out_r: &mut [f32]) {
        let gain = self.gain;
        for (l, r) in out_l.iter_mut().zip(out_r.iter_mut()) {
            let mut x = [0.0; 2];
            for (ch, x) in x.iter_mut().enumerate() {
                let mut o = [0.0; 3];
                for (i, o) in o.iter_mut().enumerate() {
                    *o = waves[i].read(self.phase[i][ch], self.level[i][ch]);
                    let next = self.phase[i][ch] + self.step[i][ch];
                    self.phase[i][ch] = next - next.floor();
                }
                let sum = gain[0][ch] * o[0] + gain[1][ch] * o[1];
                *x = if self.am {
                    sum * (1.0 - gain[2][ch] * o[2])
                } else {
                    sum + gain[2][ch] * o[2]
                };
            }
            let y = self.filter.process(x, &self.coefs, self.fast_f.next());
            *l += self.amp[0].next() * y[0];
            *r += self.amp[1].next() * y[1];
        }
    }
}
