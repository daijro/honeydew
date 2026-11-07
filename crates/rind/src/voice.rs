// Copyright (C) 2026 The Honeydew contributors
// SPDX-License-Identifier: GPL-3.0-or-later

//! One voice: four oscillators in two parts (each with its own amplitude envelope), two filters fed by either part, two
//! modulation envelopes, two LFOs and the sixteen-slot modulation matrix. A voice renders in short control steps: the
//! modulation is computed once a step and the audio runs per sample in between.

use crate::env::{Env, EnvCoef, Lfo};
use crate::filter::{Filter, Setup};
use crate::ids::*;
use crate::osc::Osc;
use crate::params::Snapshot;
use crate::util::{Rng, note_hz};

/// The gain every voice plays at: room for chords before the output stage.
pub const VOICE_GAIN: f32 = 0.3;

const SOURCES: usize = 14;
const TARGETS: usize = 35;

/// What every voice shares in a control step.
pub struct Step<'a> {
    pub p: &'a Snapshot,
    pub sr: f32,
    pub amp: [EnvCoef; 2],
    pub menv: [EnvCoef; 2],
    pub bend: f32,
    pub wheel: f32,
    pub touch: f32,
    pub lfo_hz: [f32; 2],
}

fn balance(pan: f32) -> (f32, f32) {
    let pan = pan.clamp(-1.0, 1.0);
    (1.0 - pan.max(0.0), 1.0 + pan.min(0.0))
}

#[derive(Clone)]
pub struct Voice {
    pub active: bool,
    pub gate: bool,
    pub pedal: bool,
    pub note: u8,
    pub age: u64,
    vel: f32,
    step_vel: f32,
    target: f32,
    pos: f32,
    rand: f32,
    osc: [Osc; 4],
    amp: [Env; 2],
    menv: [Env; 2],
    lfo: [Lfo; 2],
    filt: [Filter; 2],
    rate_mod: [f32; 2],
    gain_mod: [f32; 2],
    rng: Rng,
    tail: u32,
}

impl Voice {
    pub fn new(seed: u32) -> Voice {
        Voice {
            active: false,
            gate: false,
            pedal: false,
            note: 0,
            age: 0,
            vel: 0.0,
            step_vel: 1.0,
            target: 60.0,
            pos: 60.0,
            rand: 0.0,
            osc: Default::default(),
            amp: Default::default(),
            menv: Default::default(),
            lfo: [Lfo::new(seed ^ 0x9e37), Lfo::new(seed ^ 0x7f4a)],
            filt: Default::default(),
            rate_mod: [0.0; 2],
            gain_mod: [0.0; 2],
            rng: Rng::new(seed.wrapping_mul(2_654_435_761)),
            tail: 0,
        }
    }

    pub fn is_released(&self) -> bool {
        !self.gate
    }

    /// How loud the voice still is, for picking one to take over.
    pub fn level(&self) -> f32 {
        self.amp[0].level.max(self.amp[1].level)
    }

    pub fn kill(&mut self) {
        self.active = false;
        self.gate = false;
        self.pedal = false;
        self.amp.iter_mut().for_each(Env::reset);
        self.menv.iter_mut().for_each(Env::reset);
        self.filt.iter_mut().for_each(Filter::reset);
    }

    /// Starts a note. `from` is the pitch to glide from; `keep` continues a voice that is sounding (taken over, or
    /// legato) without restarting its oscillators and filters.
    #[allow(clippy::too_many_arguments)]
    pub fn start(
        &mut self,
        p: &Snapshot,
        note: u8,
        vel: f32,
        step_vel: f32,
        from: Option<f32>,
        keep: bool,
        age: u64,
        lfo_phase: [f32; 2],
    ) {
        self.active = true;
        self.gate = true;
        self.pedal = false;
        self.note = note;
        self.age = age;
        self.vel = vel;
        self.step_vel = step_vel;
        self.target = note as f32;
        self.pos = from.unwrap_or(self.target);
        self.tail = 0;
        if !keep {
            self.rand = self.rng.unit();
            for o in 0..4 {
                self.osc[o].start(p[OSC_BASE + o * OSC_STRIDE + osc::RAND], &mut self.rng);
            }
            self.filt.iter_mut().for_each(Filter::reset);
            for l in 0..2 {
                let retrig = p[LFO_BASE + l * LFO_STRIDE + lfo::RETRIG] >= 0.5;
                self.lfo[l].start(if retrig { 0.0 } else { lfo_phase[l] });
            }
        }
        for part in 0..2 {
            if p[PART_BASE + part * PART_STRIDE + part::ON] >= 0.5 {
                self.amp[part].gate_on();
            } else {
                self.amp[part].reset();
            }
        }
        self.menv.iter_mut().for_each(Env::gate_on);
    }

    /// Moves a sounding voice to another note without restarting it (mono legato).
    pub fn retarget(&mut self, note: u8) {
        self.note = note;
        self.target = note as f32;
    }

    pub fn release(&mut self) {
        self.gate = false;
        self.amp.iter_mut().for_each(Env::gate_off);
        self.menv.iter_mut().for_each(Env::gate_off);
    }

    /// Renders `out_l.len()` samples (one control step), adding to the outputs.
    pub fn render(&mut self, s: &Step, out_l: &mut [f32], out_r: &mut [f32]) {
        let n = out_l.len();
        let p = s.p;
        let sr = s.sr;

        // Portamento.
        let porta = p[PORTA];
        if porta > 0.0 {
            let a = 1.0 - (-(n as f32) / (sr * porta * 0.33)).exp();
            self.pos += (self.target - self.pos) * a;
        } else {
            self.pos = self.target;
        }

        // Modulation sources.
        let e3 = self.menv[0].tick(&s.menv[0]);
        let e4 = self.menv[1].tick(&s.menv[1]);
        let mut src = [0.0f32; SOURCES];
        for l in 0..2 {
            let b = LFO_BASE + l * LFO_STRIDE;
            let hz = s.lfo_hz[l] * (self.rate_mod[l] * 4.0).exp2();
            self.lfo[l].advance(hz * n as f32 / sr);
            let v = self.lfo[l].value(p[b + lfo::WAVE] as u8, p[b + lfo::PHASE]);
            src[1 + l] = (v * (p[b + lfo::GAIN] + self.gain_mod[l]).clamp(0.0, 1.0)
                + p[b + lfo::OFFSET])
                .clamp(-1.0, 1.0);
        }
        src[3] = e3;
        src[4] = e4;
        src[5] = self.vel;
        src[6] = ((self.note as f32 - 60.0) / 60.0).clamp(-1.0, 1.0);
        src[7] = s.wheel;
        src[8] = s.bend;
        src[9] = s.touch;
        src[10] = self.rand * 2.0 - 1.0;
        src[11] = self.step_vel;
        src[12] = self.amp[0].level;
        src[13] = self.amp[1].level;
        let mut md = [0.0f32; TARGETS];
        for m in 0..MAT_SLOTS {
            let b = MAT_BASE + m * MAT_STRIDE;
            let (so, de, am) = (
                p[b + mat::SRC] as usize,
                p[b + mat::DST] as usize,
                p[b + mat::AMT],
            );
            if so > 0 && de > 0 && am != 0.0 && so < SOURCES && de < TARGETS {
                md[de] += am * src[so];
            }
        }
        self.rate_mod = [md[mtgt::LFO_1_RATE], md[mtgt::LFO_2_RATE]];
        self.gain_mod = [md[mtgt::LFO_1_GAIN], md[mtgt::LFO_2_GAIN]];

        // Parts.
        let mut plev = [0.0f32; 2];
        let mut pbal = [(1.0f32, 1.0f32); 2];
        for part in 0..2 {
            let b = PART_BASE + part * PART_STRIDE;
            if p[b + part::ON] >= 0.5 {
                plev[part] = (p[b + part::LEVEL] + md[mtgt::LEVEL_A + part] + md[mtgt::LEVEL_ALL])
                    .clamp(0.0, 1.5);
                pbal[part] = balance(p[b + part::PAN] + md[mtgt::PAN_A + part] + md[mtgt::PAN_ALL]);
            }
        }

        // Oscillators.
        struct OscStep {
            on: bool,
            wave: u8,
            phase: f32,
            scale: f32,
            part: usize,
        }
        let mut os: [OscStep; 4] = std::array::from_fn(|_| OscStep {
            on: false,
            wave: 0,
            phase: 0.0,
            scale: 0.0,
            part: 0,
        });
        let detune_mod = md[mtgt::DETUNE_ALL];
        for o in 0..4 {
            let b = OSC_BASE + o * OSC_STRIDE;
            let part = o / 2;
            let voices = p[b + osc::VOICES] as usize;
            let vol = p[b + osc::VOL];
            if plev[part] <= 0.0 || voices == 0 || vol <= 0.0 {
                continue;
            }
            let semis = self.pos
                + p[MASTER_TUNE] * 0.01
                + s.bend * p[BEND]
                + p[b + osc::OCT] * 12.0
                + p[b + osc::SEMI]
                + p[b + osc::FINE] * 0.01
                + (md[mtgt::PITCH_ALL] + md[mtgt::PITCH_PART_A + part]) * 24.0
                + md[mtgt::PITCH_A1 + o] * 24.0;
            let base_dt = note_hz(semis) / sr;
            let d = (p[b + osc::DETUNE] + detune_mod).clamp(0.0, 1.0);
            let detune = 1.2 * d * d + 0.04 * d;
            let norm =
                self.osc[o].configure(voices, base_dt, detune, p[b + osc::STEREO], p[b + osc::PAN]);
            let inv = if p[b + osc::INV] >= 0.5 { -1.0 } else { 1.0 };
            os[o] = OscStep {
                on: true,
                wave: p[b + osc::WAVE] as u8,
                phase: (p[b + osc::PHASE]
                    + (md[mtgt::PHASE_A1 + o]
                        + md[mtgt::PHASE_PART_A + part]
                        + md[mtgt::PHASE_ALL])
                        * 0.5)
                    .rem_euclid(1.0),
                scale: norm * vol * inv,
                part,
            };
        }

        // Filters.
        let mut fset: [Option<Setup>; 2] = [None, None];
        let mut fin = [(0.0f32, 0.0f32); 2];
        let mut fvol = [(0.0f32, 0.0f32); 2];
        for f in 0..2 {
            let b = FILT_BASE + f * FILT_STRIDE;
            fin[f] = (p[b + filt::IN_A], p[b + filt::IN_B]);
            let bal = balance(p[b + filt::PAN]);
            let v = p[b + filt::VOL];
            fvol[f] = (v * bal.0, v * bal.1);
            if p[b + filt::ON] >= 0.5 {
                let semis = (md[mtgt::CUTOFF_1 + f] + md[mtgt::CUTOFF_ALL]) * 60.0
                    + p[b + filt::KEY] * (self.pos - 60.0);
                let hz = (p[b + filt::CUTOFF] * (semis * (1.0 / 12.0)).exp2())
                    .clamp(20.0, (0.45 * sr).min(20_000.0));
                let res = (p[b + filt::RES] + md[mtgt::RESONANCE_1 + f] + md[mtgt::RESONANCE_ALL])
                    .clamp(0.0, 1.0);
                let drive = (p[b + filt::DRIVE] + md[mtgt::DRIVE_1 + f]).clamp(0.0, 1.0);
                fset[f] = Some(Setup::new(
                    p[b + filt::TYPE] as u8,
                    p[b + filt::SLOPE] >= 0.5,
                    hz,
                    res,
                    drive,
                    sr,
                ));
            }
        }

        let vel_gain = 1.0 - p[VEL_AMP] + p[VEL_AMP] * self.vel;
        let gain = VOICE_GAIN * vel_gain * (1.0 + md[mtgt::AMP_ALL]).max(0.0);

        for i in 0..n {
            let mut part = [[0.0f32; 2]; 2];
            for o in 0..4 {
                let st = &os[o];
                if st.on {
                    let (l, r) = self.osc[o].tick(st.wave, st.phase, &mut self.rng);
                    part[st.part][0] += l * st.scale;
                    part[st.part][1] += r * st.scale;
                }
            }
            for k in 0..2 {
                let g = self.amp[k].tick(&s.amp[k]) * plev[k];
                part[k][0] *= g * pbal[k].0;
                part[k][1] *= g * pbal[k].1;
            }
            let (mut ol, mut or) = (0.0, 0.0);
            for f in 0..2 {
                let il = part[0][0] * fin[f].0 + part[1][0] * fin[f].1;
                let ir = part[0][1] * fin[f].0 + part[1][1] * fin[f].1;
                let (fl, fr) = match &fset[f] {
                    Some(setup) => (
                        self.filt[f].tick(0, il, setup),
                        self.filt[f].tick(1, ir, setup),
                    ),
                    None => (il, ir),
                };
                ol += fl * fvol[f].0;
                or += fr * fvol[f].1;
            }
            out_l[i] += ol * gain;
            out_r[i] += or * gain;
        }

        // A voice ends when both parts are silent and the filters have rung out.
        if self.amp[0].is_idle() && self.amp[1].is_idle() {
            self.tail += n as u32;
            if self.tail as f32 > 0.06 * sr {
                self.active = false;
            }
        } else {
            self.tail = 0;
        }
    }
}
