//! Transistor Bass (ID 19): a monophonic acid bass, built term by term. Every constant carries its term's name.

use std::f64::consts::PI;
use std::sync::OnceLock;

use crate::ProcessContext;
use crate::instrument::Instrument;

/// Params 0..=13.
pub const PARAMETERS: usize = 14;
const DEFAULTS: [f32; PARAMETERS] = [
    1.0, 0.0, 1.0, 1.0, 0.1, 1.0, 0.1, 0.0, 0.0, 0.3, 0.5, 0.0, 0.0, 0.0,
];
const WAVEFORM: usize = 1;
const CUTOFF: usize = 2;
const RESO: usize = 3;
const ENV_MOD: usize = 4;
const DECAY: usize = 5;
const ACCENT: usize = 6;
const DISTORTION: usize = 7;
const DRIVE_HP: usize = 8;
const DRIVE: usize = 9;
const DRIVE_VOL: usize = 10;
const PW: usize = 11;
const LFO_SPEED: usize = 12;
const LFO_TO_PW: usize = 13;

/// Voice: a stack of up to 16 held keys. A 17th key drops the oldest from the stack.
const MAX_KEYS: usize = 16;
/// Accent is set on each note-on when velocity ≥ 99/127.
const ACCENT_VELOCITY: f32 = 99.0 / 127.0;
/// Pitch T-smoother: T = 0.05 s while sliding, else 0.0001 s.
const SLIDE_T: f64 = 0.05;
const PITCH_T: f64 = 0.0001;
/// Oscillator: one falling-saw cycle over 1024 points, 48 mip levels a quarter octave apart, 64-tap windowed sinc,
/// level = clamp(floor(4·log2(1024·f/(0.9·fs))), 0, 47).
const SAW_POINTS: usize = 1024;
const MIP_LEVELS: usize = 48;
const SINC_TAPS: usize = 64;
const MIP_HEADROOM: f32 = 0.9;
/// Pulse width: pw = clamp(0.5 + 0.45·PW + 0.24·LFO→PW·sin 2πφ, 0.001, 0.999); LFO 0.1 + 9.9·LFO Speed Hz.
const PW_CENTRE: f32 = 0.5;
const PW_RANGE: f32 = 0.45;
const PW_LFO_DEPTH: f32 = 0.24;
const PW_MIN: f32 = 0.001;
const PW_MAX: f32 = 0.999;
const LFO_MIN_HZ: f32 = 0.1;
const LFO_RANGE_HZ: f32 = 9.9;
/// Envelopes decay ×0.1 every T: MEG T = 0.2775 + 1.8825·D² s (0.2775 s accented), VEG T = 1 s.
const MEG_T: f64 = 0.2775;
const MEG_T_DECAY: f64 = 1.8825;
const VEG_T: f64 = 1.0;
/// Accent sweep smoother: 0.05 s rising, 0.2 s falling.
const SWEEP_RISE_T: f64 = 0.05;
const SWEEP_FALL_T: f64 = 0.2;
/// Cutoff: f_c = 8.17579892·2^(128v/12) Hz, clamped to [11 Hz, 0.4·fs].
const KEY_0_HZ: f32 = 8.175_799;
const CUTOFF_MIN_HZ: f32 = 11.0;
const CUTOFF_MAX_FS: f32 = 0.4;
/// Filter feedback highpass.
const HP_FEEDBACK_HZ: f32 = 160.0;
/// VCA: accent level 0.74 + MEG·(0.74 + 1.86A²), through a 0.005 s T-smoother.
const VCA_ACCENT: f32 = 0.74;
const VCA_T: f64 = 0.005;
/// Post chain: one-pole HP 205 Hz, then ×0.25.
const HP_POST_HZ: f32 = 205.0;
const POST_GAIN: f32 = 0.25;
/// Distortion stages 2, 3, 4 and 5.
const DRIVE_HP_HZ: [f32; 2] = [7.2, 60.0];
const DRIVE_LP_HZ: [f32; 6] = [16000.0, 5000.0, 2500.0, 1500.0, 900.0, 600.0];
const SLEW_PER_SECOND: f32 = 300_000.0;
const SLEW_LIMIT: f32 = 4.5;
const DRIVE_OUT_LP_HZ: f32 = 19475.0;
/// One-pole cutoffs stop at 0.49·fs, so the 19475 Hz LP stays a lowpass at rates below 39.75 kHz.
const ONE_POLE_MAX_FS: f32 = 0.49;
/// Output: gains move 1 % toward target per audio block. A block is 256 frames, counted across `process`
/// calls, so the output does not depend on how the host splits blocks.
const BALANCE_STEP: f32 = 0.01;
const BALANCE_BLOCK: usize = 256;
/// Rendering stops 0.25 s after the last key-up.
const TAIL_S: f32 = 0.25;

/// y += (x − y)·smoother(T): 10 % residual after T s.
fn smoother(fs: f32, t: f64) -> f32 {
    (1.0 - 0.1_f64.powf(1.0 / (f64::from(fs) * t))) as f32
}

/// ×0.1 every T s, per sample.
fn decay(fs: f32, t: f64) -> f32 {
    0.1_f64.powf(1.0 / (f64::from(fs) * t)) as f32
}

fn wrap(phase: f32) -> f32 {
    phase - phase.floor()
}

/// The cutoff law: f_c in Hz for the (smoothed) key `n`, MEG and the accent `sweep`.
fn cutoff_hz(params: &[f32; PARAMETERS], n: f32, meg: f32, sweep: f32, fs: f32) -> f32 {
    let (c, e) = (params[CUTOFF], params[ENV_MOD]);
    let v = 0.23 * sweep
        + meg * (0.16 + 0.54 * e * e)
        + (0.1 + 0.58 * c.powf(1.5)) * (1.0 - 0.31 * e * e)
        - 0.02
        - (n - 12.0) / 288.0;
    (KEY_0_HZ * (128.0 * v / 12.0).exp2()).clamp(CUTOFF_MIN_HZ, CUTOFF_MAX_FS * fs)
}

fn pulse_width(params: &[f32; PARAMETERS], lfo_phase: f32) -> f32 {
    let lfo = (std::f32::consts::TAU * lfo_phase).sin();
    (PW_CENTRE + PW_RANGE * params[PW] + PW_LFO_DEPTH * params[LFO_TO_PW] * lfo)
        .clamp(PW_MIN, PW_MAX)
}

/// The quarter-octave saw mip set, built once per process on the app thread.
struct SawMips {
    table: Vec<f32>,
    start: [usize; MIP_LEVELS],
    len: [usize; MIP_LEVELS],
}

static SAW_MIPS: OnceLock<SawMips> = OnceLock::new();

impl SawMips {
    fn get() -> &'static Self {
        SAW_MIPS.get_or_init(Self::build)
    }

    /// Level k = the cycle resampled to 1024·2^(−k/4) points (rounded, at least 1) through a 64-tap windowed sinc at
    /// that level's Nyquist. Details: the cycle is 1 − 2i/1023 (+1 → −1, zero mean); the window is Blackman; each
    /// point's taps are normalized to unit DC gain.
    fn build() -> Self {
        let cycle =
            |i: i64| 1.0 - 2.0 * i.rem_euclid(SAW_POINTS as i64) as f64 / (SAW_POINTS - 1) as f64;
        let half = (SINC_TAPS / 2) as i64;
        let mut mips = Self {
            table: Vec::new(),
            start: [0; MIP_LEVELS],
            len: [0; MIP_LEVELS],
        };
        for k in 0..MIP_LEVELS {
            let len = ((SAW_POINTS as f64 * (-(k as f64) / 4.0).exp2()).round() as usize).max(1);
            let c = len as f64 / SAW_POINTS as f64;
            mips.start[k] = mips.table.len();
            mips.len[k] = len;
            for j in 0..len {
                let t = j as f64 / c;
                let first = t.floor() as i64 - half + 1;
                let (mut sum, mut weight) = (0.0, 0.0);
                for i in first..first + SINC_TAPS as i64 {
                    let x = t - i as f64;
                    let sinc = if x == 0.0 {
                        1.0
                    } else {
                        (PI * c * x).sin() / (PI * c * x)
                    };
                    let u = x / half as f64;
                    let blackman = 0.42 + 0.5 * (PI * u).cos() + 0.08 * (2.0 * PI * u).cos();
                    let h = c * sinc * blackman;
                    sum += cycle(i) * h;
                    weight += h;
                }
                mips.table.push((sum / weight) as f32);
            }
        }
        mips
    }

    fn level(f: f32, fs: f32) -> usize {
        (4.0 * (SAW_POINTS as f32 * f / (MIP_HEADROOM * fs)).log2())
            .floor()
            .clamp(0.0, (MIP_LEVELS - 1) as f32) as usize
    }

    /// Level `level` at `phase` in 0..1, linearly interpolated.
    fn read(&self, level: usize, phase: f32) -> f32 {
        let (start, len) = (self.start[level], self.len[level]);
        let table = &self.table[start..start + len];
        let position = phase * len as f32;
        let i = (position as usize).min(len - 1);
        let next = table[(i + 1) % len];
        table[i] + (next - table[i]) * (position - i as f32)
    }
}

/// Output = pulse + (saw − pulse)·W, pulse = saw(φ) − saw(φ + pw).
fn oscillator(mips: &SawMips, level: usize, phase: f32, pw: f32, waveform: f32) -> f32 {
    let saw = mips.read(level, phase);
    let pulse = saw - mips.read(level, wrap(phase + pw));
    pulse + (saw - pulse) * waveform
}

/// A one-pole bilinear (TPT) filter, g = tan(π·fc/fs).
#[derive(Clone, Copy, Default)]
struct OnePole {
    k: f32,
    s: f32,
}

impl OnePole {
    fn new(hz: f32, fs: f32) -> Self {
        let mut filter = Self::default();
        filter.set(hz, fs);
        filter
    }

    fn set(&mut self, hz: f32, fs: f32) {
        let g = (std::f32::consts::PI * hz.min(ONE_POLE_MAX_FS * fs) / fs).tan();
        self.k = g / (1.0 + g);
    }

    fn lp(&mut self, x: f32) -> f32 {
        let v = (x - self.s) * self.k;
        let y = v + self.s;
        self.s = y + v;
        y
    }

    fn hp(&mut self, x: f32) -> f32 {
        x - self.lp(x)
    }
}

/// The 4-pole diode-ladder approximation with HP160 in the feedback.
#[derive(Clone, Copy)]
struct DiodeLadder {
    y: [f32; 4],
    hp: OnePole,
}

impl DiodeLadder {
    fn new(fs: f32) -> Self {
        Self {
            y: [0.0; 4],
            hp: OnePole::new(HP_FEEDBACK_HZ, fs),
        }
    }

    /// The coefficients b, K and g are evaluated in f64 from f_c.
    fn process(&mut self, input: f32, fc: f32, r: f32, fs: f32) -> f32 {
        let x = f64::from(fc) * std::f64::consts::FRAC_1_SQRT_2 / f64::from(fs);
        let b = (0.000_455_223_46 + 6.192_218_9 * x) / (1.0 + 12.358_354 * x + 4.415_634_5 * x * x);
        let k = (((((x + 7198.6997) * x - 5837.7917) * x - 476.47308) * x + 614.95611) * x
            + 213.87126)
            * x
            + 16.998792;
        let r64 = f64::from(r);
        let g = (((k / 17.0 - 1.0) * r64 + 1.0) * (1.0 + r64)) as f32;
        let (b, k) = (b as f32, k as f32);
        let [y1, y2, y3, y4] = &mut self.y;
        let y0 = input - self.hp.hp(k * r * *y4);
        *y1 += 2.0 * b * (y0 - *y1 + *y2);
        *y2 += b * (*y1 - 2.0 * *y2 + *y3);
        *y3 += b * (*y2 - 2.0 * *y3 + *y4);
        *y4 += b * (*y3 - 2.0 * *y4);
        2.0 * g * *y4
    }
}

/// The DISTORTION page chain (Dr = Drive, H = Drive HP, Vo = Drive Vol.).
#[derive(Clone, Copy)]
struct Distortion {
    pre: f32,
    hp: [OnePole; 3],
    lp: OnePole,
    max_step: f32,
    slew: f32,
    out_lp: OnePole,
    post: f32,
}

impl Distortion {
    fn new(fs: f32) -> Self {
        Self {
            pre: 1.0,
            hp: [
                OnePole::new(DRIVE_HP_HZ[0], fs),
                OnePole::new(DRIVE_HP_HZ[1], fs),
                OnePole::default(),
            ],
            lp: OnePole::default(),
            max_step: SLEW_PER_SECOND / fs,
            slew: 0.0,
            out_lp: OnePole::new(DRIVE_OUT_LP_HZ, fs),
            post: 1.0,
        }
    }

    fn configure(&mut self, params: &[f32; PARAMETERS], fs: f32) {
        let (dr, h, vo) = (params[DRIVE], params[DRIVE_HP], params[DRIVE_VOL]);
        self.pre = 1.0 + 2305.0 * dr * dr * dr;
        self.hp[2].set(10.0 + 3000.0 * h * h, fs);
        let at = dr * 4.999;
        let i = (at as usize).min(DRIVE_LP_HZ.len() - 2);
        let lp_hz = DRIVE_LP_HZ[i] + (DRIVE_LP_HZ[i + 1] - DRIVE_LP_HZ[i]) * (at - i as f32);
        self.lp.set(lp_hz, fs);
        self.post =
            (1.0 + h) * (4.0 * vo - 2.0).exp2() * (0.9819 * (-11.834 * dr).exp() + 0.022199);
    }

    fn process(&mut self, x: f32) -> f32 {
        let mut x = x * self.pre;
        for hp in &mut self.hp {
            x = hp.hp(x);
        }
        x = self.lp.lp(x);
        self.slew = (self.slew + (x - self.slew).clamp(-self.max_step, self.max_step))
            .clamp(-SLEW_LIMIT, SLEW_LIMIT);
        self.out_lp.lp(self.slew.tanh()) * self.post
    }
}

/// The Transistor Bass. Everything it needs is allocated in [`Self::new`] (the mip set once per process).
pub struct TransistorBass {
    fs: f32,
    params: [f32; PARAMETERS],
    mips: &'static SawMips,
    // Voice.
    keys: [u8; MAX_KEYS],
    held: usize,
    accent: bool,
    sliding: bool,
    target: f32,
    pitch: f32,
    fresh: bool,
    pan: f32,
    rendering: bool,
    tail: usize,
    // Oscillator and modulation.
    phase: f32,
    lfo_phase: f32,
    meg: f32,
    veg: f32,
    sweep: f32,
    vca: f32,
    // Audio chain.
    ladder: DiodeLadder,
    hp_post: OnePole,
    distortion: Distortion,
    gains: [f32; 2],
    block_frame: usize,
    // Per-sample coefficients.
    slide_k: f32,
    pitch_k: f32,
    sweep_rise_k: f32,
    sweep_fall_k: f32,
    vca_k: f32,
    veg_decay: f32,
    meg_accent_decay: f32,
    meg_decay: f32,
    lfo_step: f32,
    tail_frames: usize,
}

impl TransistorBass {
    /// A Transistor Bass at `sample_rate` Hz with the default params.
    pub fn new(sample_rate: f32) -> Self {
        let fs = sample_rate.max(1.0);
        let mut synth = Self {
            fs,
            params: DEFAULTS,
            mips: SawMips::get(),
            keys: [0; MAX_KEYS],
            held: 0,
            accent: false,
            sliding: false,
            target: 0.0,
            pitch: 0.0,
            fresh: true,
            pan: 0.5,
            rendering: false,
            tail: 0,
            phase: 0.0,
            lfo_phase: 0.0,
            meg: 0.0,
            veg: 0.0,
            sweep: 0.0,
            vca: 0.0,
            ladder: DiodeLadder::new(fs),
            hp_post: OnePole::new(HP_POST_HZ, fs),
            distortion: Distortion::new(fs),
            // The balance starts centred.
            gains: [1.0; 2],
            block_frame: 0,
            slide_k: smoother(fs, SLIDE_T),
            pitch_k: smoother(fs, PITCH_T),
            sweep_rise_k: smoother(fs, SWEEP_RISE_T),
            sweep_fall_k: smoother(fs, SWEEP_FALL_T),
            vca_k: smoother(fs, VCA_T),
            veg_decay: decay(fs, VEG_T),
            meg_accent_decay: decay(fs, MEG_T),
            meg_decay: 1.0,
            lfo_step: 0.0,
            tail_frames: ((TAIL_S * fs).round() as usize).max(1),
        };
        synth.configure();
        synth
    }

    /// Recomputes what depends on params.
    fn configure(&mut self) {
        let d = f64::from(self.params[DECAY]);
        self.meg_decay = decay(self.fs, MEG_T + MEG_T_DECAY * d * d);
        self.lfo_step = (LFO_MIN_HZ + LFO_RANGE_HZ * self.params[LFO_SPEED]) / self.fs;
        self.distortion.configure(&self.params, self.fs);
    }

    /// Moves the balance gains 1 % toward the note pan's targets.
    fn step_balance(&mut self) {
        let p = (2.0 * self.pan - 1.0).clamp(-1.0, 1.0);
        let targets = [1.0 - p.max(0.0), 1.0 + p.min(0.0)];
        for (gain, target) in self.gains.iter_mut().zip(targets) {
            *gain += (target - *gain) * BALANCE_STEP;
        }
    }

    /// One mono output sample.
    fn tick(&mut self) -> f32 {
        let (fs, p) = (self.fs, &self.params);
        let (r, a) = (p[RESO], p[ACCENT]);

        self.pitch += (self.target - self.pitch)
            * if self.sliding {
                self.slide_k
            } else {
                self.pitch_k
            };
        let f = 440.0 * ((self.pitch - 81.0) / 12.0).exp2();
        let level = SawMips::level(f, fs);
        let osc = oscillator(
            self.mips,
            level,
            self.phase,
            pulse_width(p, self.lfo_phase),
            p[WAVEFORM],
        );
        self.phase = wrap(self.phase + f / fs);
        self.lfo_phase = wrap(self.lfo_phase + self.lfo_step);

        let (meg, veg) = (self.meg, self.veg);
        self.meg *= if self.accent {
            self.meg_accent_decay
        } else {
            self.meg_decay
        };
        self.veg *= self.veg_decay;

        let accent = if self.accent {
            meg * (0.5 + 0.5 * r) * a * a
        } else {
            0.0
        };
        let k = if accent > self.sweep {
            self.sweep_rise_k
        } else {
            self.sweep_fall_k
        };
        self.sweep += (accent - self.sweep) * k;
        let sweep = self.sweep + (3.0 - 3.0 * r) * accent;

        let fc = cutoff_hz(p, self.pitch, meg, sweep, fs);
        let filtered = self.ladder.process(osc, fc, r, fs);

        let vca = match (self.held > 0, self.accent) {
            (false, _) => 0.0,
            (true, false) => veg,
            (true, true) => veg.max(VCA_ACCENT + meg * (VCA_ACCENT + 1.86 * a * a)),
        };
        self.vca += (vca - self.vca) * self.vca_k;
        let out = self.hp_post.hp(self.vca * filtered) * POST_GAIN;
        if p[DISTORTION] >= 0.5 {
            self.distortion.process(out)
        } else {
            out
        }
    }
}

impl Instrument for TransistorBass {
    fn set_parameter(&mut self, index: usize, value: f32) {
        if index < PARAMETERS {
            self.params[index] = value.clamp(0.0, 1.0);
            self.configure();
        }
    }

    /// No key held: a new note, envelopes reset. Keys held: a slide. Accent and pan are set on every note-on.
    /// The note pan is the newest note-on's, kept on slide-back.
    fn note_on(&mut self, key: u8, velocity: f32, pan: f32) {
        if self.keys[..self.held].contains(&key) {
            return;
        }
        if self.held == MAX_KEYS {
            self.keys.copy_within(1.., 0);
            self.held -= 1;
        }
        self.sliding = self.held > 0;
        self.keys[self.held] = key;
        self.held += 1;
        self.accent = velocity >= ACCENT_VELOCITY;
        self.target = f32::from(key);
        self.pan = pan;
        if !self.sliding {
            self.meg = 1.0;
            self.veg = 1.0;
        }
        // The first note after construction or reset starts at its pitch.
        if self.fresh {
            self.pitch = self.target;
            self.fresh = false;
        }
        self.rendering = true;
    }

    /// Releasing the newest key slides back to the previous held one, keeping the accent; releasing the last
    /// closes the gate.
    fn note_off(&mut self, key: u8) {
        let Some(i) = self.keys[..self.held].iter().position(|&k| k == key) else {
            return;
        };
        let newest = i + 1 == self.held;
        self.keys.copy_within(i + 1..self.held, i);
        self.held -= 1;
        if self.held == 0 {
            self.tail = self.tail_frames;
        } else if newest {
            self.target = f32::from(self.keys[self.held - 1]);
            self.sliding = true;
        }
    }

    fn all_notes_off(&mut self) {
        if self.held > 0 {
            self.held = 0;
            self.tail = self.tail_frames;
        }
    }

    fn reset(&mut self) {
        let params = self.params;
        *self = Self::new(self.fs);
        self.params = params;
        self.configure();
    }

    /// Tempo and song position are unused: the Transistor Bass has no tempo-synced params.
    fn process(&mut self, left: &mut [f32], right: &mut [f32], _context: ProcessContext) {
        for (l, r) in left.iter_mut().zip(right.iter_mut()) {
            if !self.rendering {
                return;
            }
            if self.block_frame == 0 {
                self.step_balance();
            }
            self.block_frame = (self.block_frame + 1) % BALANCE_BLOCK;
            let out = self.tick();
            *l += out * self.gains[0];
            *r += out * self.gains[1];
            if self.held == 0 {
                self.tail -= 1;
                self.rendering = self.tail > 0;
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const FS: f32 = 48_000.0;

    /// Renders `frames` frames and drops them.
    fn run(synth: &mut TransistorBass, frames: usize) {
        let (mut left, mut right) = (vec![0.0; frames], vec![0.0; frames]);
        let context = ProcessContext {
            sample_rate: FS,
            tempo_bpm: 120.0,
            start_beat: 0.0,
            playing: false,
        };
        synth.process(&mut left, &mut right, context);
    }

    #[test]
    fn cutoff_law_gives_the_spec_examples_at_n_60() {
        // ≈ 309 Hz at rest (v = 0.491225) and ≈ 1049 Hz at the envelope peak (v = 0.656625).
        let rest = cutoff_hz(&DEFAULTS, 60.0, 0.0, 0.0, FS);
        let peak = cutoff_hz(&DEFAULTS, 60.0, 1.0, 0.0, FS);
        assert!((rest - 308.92).abs() < 0.05, "{rest}");
        assert!((peak - 1049.41).abs() < 0.2, "{peak}");
        assert_eq!(cutoff_hz(&DEFAULTS, 400.0, 0.0, 0.0, FS), CUTOFF_MIN_HZ);
        assert_eq!(cutoff_hz(&DEFAULTS, 0.0, 1.0, 9.0, FS), 0.4 * FS);
    }

    #[test]
    fn smoothers_leave_ten_percent_after_t() {
        let k = smoother(FS, 0.05);
        let mut y = 0.0;
        for _ in 0..(0.05 * FS) as usize {
            y += (1.0 - y) * k;
        }
        assert!((1.0 - y - 0.1).abs() < 1e-4, "{y}");
        assert!((decay(FS, 1.0).powi(FS as i32) - 0.1).abs() < 1e-4);
    }

    #[test]
    fn mip_set_has_48_quarter_octave_levels() {
        let mips = SawMips::get();
        assert_eq!(mips.len[0], 1024);
        assert_eq!(mips.len[1], 861);
        assert_eq!(mips.len[4], 512);
        assert_eq!(mips.len[40], 1);
        assert_eq!(mips.len[47], 1);
        // Level 0 is the raw cycle: +1 → −1 over 1024 points.
        assert_eq!(mips.read(0, 0.0), 1.0);
        assert!((mips.read(0, 1023.0 / 1024.0) + 1.0).abs() < 1e-6);
        assert!((mips.read(0, 0.25) - (1.0 - 2.0 * 256.0 / 1023.0)).abs() < 1e-6);
        for k in 0..=20 {
            let (start, len) = (mips.start[k], mips.len[k]);
            let level = &mips.table[start..start + len];
            let mean = level.iter().sum::<f32>() / len as f32;
            let peak = level.iter().fold(0.0_f32, |m, v| m.max(v.abs()));
            assert!(
                mean.abs() < 5e-3 && (0.8..1.2).contains(&peak),
                "level {k}: {mean} {peak}"
            );
        }
        // floor(4·log2(1024·f/(0.9·fs))): 1024·f = 0.9·fs·2^(k/4) selects level k.
        assert_eq!(SawMips::level(20.0, FS), 0);
        assert_eq!(SawMips::level(0.9 * FS * 2.0 / 1024.0 * 1.001, FS), 4);
        assert_eq!(SawMips::level(0.9 * FS * 2.0 / 1024.0 * 0.999, FS), 3);
        assert_eq!(SawMips::level(1e9, FS), 47);
    }

    #[test]
    fn level_16_droops_as_its_kernel_response_says() {
        // Level 16 (64 points) takes every 16th point of the cycle through one fixed 64-tap kernel h(x), x = −32..32,
        // c = 1/16. So its DFT bin m is (1/16)·Σr H(m + 64r)·S(m + 64r): S is the cycle's DFT and H the kernel's,
        // aliases included. At m = len/4 and 3·len/8 the kernel droops to |H| ≈ 0.877 and 0.717.
        let mips = SawMips::get();
        let (len, step, c) = (64, 16, 1.0 / 16.0);
        let table = &mips.table[mips.start[16]..mips.start[16] + len];
        let dft = |values: &mut dyn Iterator<Item = (f64, f64)>, k: usize, n: usize| {
            values.fold((0.0, 0.0), |(re, im), (j, v)| {
                let w = -2.0 * PI * (k as f64) * j / n as f64;
                (re + v * w.cos(), im + v * w.sin())
            })
        };
        let kernel = |x: f64| {
            let sinc = if x == 0.0 {
                1.0
            } else {
                (PI * c * x).sin() / (PI * c * x)
            };
            let u = x / 32.0;
            c * sinc * (0.42 + 0.5 * (PI * u).cos() + 0.08 * (2.0 * PI * u).cos())
        };
        let gain: f64 = (-32..32).map(|x| kernel(f64::from(x))).sum();
        let h = |k| {
            dft(
                &mut (-32..32).map(|x| (f64::from(x), kernel(f64::from(x)) / gain)),
                k,
                1024,
            )
        };
        let s = |k| {
            dft(
                &mut (0..1024).map(|i| (i as f64, 1.0 - 2.0 * i as f64 / 1023.0)),
                k,
                1024,
            )
        };
        let magnitude = |(re, im): (f64, f64)| (re * re + im * im).sqrt();
        for (m, droop) in [(len / 4, 0.877), (len * 3 / 8, 0.717)] {
            assert!(
                (magnitude(h(m)) - droop).abs() < 1e-3,
                "|H({m})| = {}",
                magnitude(h(m))
            );
            let expected = (0..step).fold((0.0, 0.0), |(re, im), r| {
                let ((hr, hi), (sr, si)) = (h(m + len * r), s(m + len * r));
                (
                    re + (hr * sr - hi * si) / step as f64,
                    im + (hr * si + hi * sr) / step as f64,
                )
            });
            let actual = dft(
                &mut table
                    .iter()
                    .enumerate()
                    .map(|(j, v)| (j as f64, f64::from(*v))),
                m,
                len,
            );
            let error = magnitude((actual.0 - expected.0, actual.1 - expected.1));
            assert!(
                error < 1e-4 * magnitude(expected),
                "bin {m}: {actual:?} vs {expected:?}"
            );
        }
    }

    #[test]
    fn waveform_endpoints_are_pulse_and_saw() {
        let mips = SawMips::get();
        for phase in [0.1, 0.3, 0.6, 0.9] {
            // Saw: +1 → −1.
            let saw = oscillator(mips, 0, phase, 0.5, 1.0);
            assert!((saw - (1.0 - 2.0 * phase)).abs() < 2e-3, "{saw}");
            // Pulse levels +2pw and 2pw − 2.
            for pw in [0.5, 0.25] {
                let pulse = oscillator(mips, 0, phase, pw, 0.0);
                let expected = if phase < 1.0 - pw {
                    2.0 * pw
                } else {
                    2.0 * pw - 2.0
                };
                assert!(
                    (pulse - expected).abs() < 2e-3,
                    "pw {pw} φ {phase}: {pulse}"
                );
            }
        }
    }

    #[test]
    fn pulse_width_law() {
        let mut p = DEFAULTS;
        assert_eq!(pulse_width(&p, 0.0), 0.5);
        p[PW] = 1.0;
        assert!((pulse_width(&p, 0.0) - 0.95).abs() < 1e-6);
        p[LFO_TO_PW] = 1.0;
        assert_eq!(pulse_width(&p, 0.25), PW_MAX);
        p[PW] = 0.0;
        assert!((pulse_width(&p, 0.75) - 0.26).abs() < 1e-6);
    }

    #[test]
    fn accent_threshold_and_slide_back_keeps_the_accent() {
        let mut synth = TransistorBass::new(FS);
        synth.note_on(40, 98.0 / 127.0, 0.5);
        assert!(!synth.accent && !synth.sliding);
        synth.note_on(43, 99.0 / 127.0, 0.5);
        assert!(synth.accent && synth.sliding && synth.target == 43.0);
        synth.note_on(43, 0.1, 0.5);
        assert!(synth.accent, "a held key's note-on is ignored");
        synth.note_off(43);
        assert!(
            synth.accent && synth.sliding && synth.target == 40.0,
            "slide-back keeps the accent"
        );
        synth.note_on(45, 0.5, 0.5);
        synth.note_off(45);
        assert!(!synth.accent && synth.target == 40.0);
    }

    #[test]
    fn the_stack_holds_16_keys_and_drops_the_oldest() {
        let mut synth = TransistorBass::new(FS);
        for key in 0..17 {
            synth.note_on(key, 0.5, 0.5);
        }
        assert_eq!(synth.held, 16);
        assert_eq!(synth.keys[0], 1);
        synth.note_off(0);
        assert_eq!(synth.held, 16);
    }

    #[test]
    fn envelopes_decay_tenfold_per_t() {
        let mut synth = TransistorBass::new(FS);
        synth.set_parameter(DECAY, 0.5);
        synth.note_on(48, 0.5, 0.5);
        let meg_t = 0.2775 + 1.8825 * 0.25;
        run(&mut synth, (meg_t * FS) as usize);
        assert!((synth.meg - 0.1).abs() < 1e-3, "MEG {}", synth.meg);

        let mut synth = TransistorBass::new(FS);
        synth.note_on(48, 1.0, 0.5);
        run(&mut synth, (0.2775 * FS) as usize);
        assert!((synth.meg - 0.1).abs() < 1e-3, "accented MEG {}", synth.meg);
        run(&mut synth, FS as usize - (0.2775 * FS) as usize);
        assert!((synth.veg - 0.1).abs() < 1e-3, "VEG {}", synth.veg);
    }

    #[test]
    fn a_slide_glides_with_t_50_ms() {
        let mut synth = TransistorBass::new(FS);
        synth.note_on(48, 0.5, 0.5);
        synth.note_on(60, 0.5, 0.5);
        run(&mut synth, (0.05 * FS) as usize);
        assert!((synth.pitch - (60.0 - 1.2)).abs() < 0.01, "{}", synth.pitch);
    }
}
