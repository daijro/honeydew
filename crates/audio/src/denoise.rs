//! Denoise (module type [`DENOISE`]), Honeydew's own effect: GitHub #43.
//! Spectral noise reduction.
//!
//! **Params**, normalized, in this order (the saved one): 0 Enable, [`REDUCTION`], [`SENSITIVITY`], [`LEARN`]. Post
//! Gain, Mix and Wet Pan are the rack's (`CommonModule`). The laws from `v`:
//! - Reduction `40v` dB, the most a bin is lowered.
//! - Sensitivity (0–100 %) `−6 + 18v` dB, the over-subtraction `θ`: how far over the noise floor a bin must be to pass.
//! - Learn: on from 0.5.
//!
//! **STFT.** Frames of [`FRAME`] samples every [`HOP`] (75 % overlap), a periodic Hann window before the FFT and again
//! after the inverse, overlap-added (the squared windows sum to 1.5 at this hop, which the output divides out). Both
//! channels share one complex FFT, the left real and the right imaginary: a real gain that is the same at bins `k` and
//! `FRAME − k` keeps them apart, and `(|X[k]|² + |X[FRAME − k]|²)/2` is the channels' mean power. One gain for both
//! keeps the stereo image. The output is [`FRAME`] samples late ([`DenoiseState::latency`]).
//!
//! **Noise floor.** Each bin's power is smoothed (`P ← 0.85·P + 0.15·|X|²`) and its minimum over the last 2 s tracked
//! in [`WINDOWS`] sub-windows (Martin's minimum statistics), times [`BIAS`], which brings the minimum of noise's smoothed
//! power back to its mean. A tone held longer than that reads as noise. While Learn is on, the floor is instead each
//! bin's mean power since Learn came on, a noise profile; once learned, it stays the floor (through a reset too) until
//! Learn comes on again.
//!
//! **Gain.** Ephraim and Malah's decision-directed rule: the a-priori SNR `ξ = 0.98·G'²·γ' + 0.02·max(γ − 1, 0)`,
//! with `γ = |X|²/(θ·N)`, `N` the floor and `G'`, `γ'` the bin's last frame; the gain `G = max(ξ/(1 + ξ), g)`, `g` the
//! Reduction as a gain. The recursion smooths each bin's gain over time, so lone noise peaks do not pop through
//! (musical noise), and the floor `g` masks what is left.
//!
//! Nothing allocates while it runs: [`DenoiseState::new`] makes every buffer, which is only cleared in place after.

use crate::slicer::fft;

/// The module type: Honeydew's own id, which a song file keeps apart from the song format's modules.
pub const DENOISE: u16 = 1007;

pub const REDUCTION: usize = 1;
pub const SENSITIVITY: usize = 2;
pub const LEARN: usize = 3;
/// Enable and the three above.
pub const PARAMS: usize = 4;

/// A new module's params: Reduction 12 dB, Sensitivity 50 % (+3 dB), not learning.
pub const DEFAULTS: [f32; PARAMS] = [1.0, 0.3, 0.5, 0.0];

/// The STFT's frame and hop, samples; the frame is also the latency.
pub const FRAME: usize = 1024;
pub const HOP: usize = FRAME / 4;
const BINS: usize = FRAME / 2 + 1;
/// The minimum's sub-windows, and their span, seconds.
const WINDOWS: usize = 8;
const SPAN_S: f32 = 2.0;
/// The power's smoothing, and the minimum's bias against the mean.
const SMOOTH: f64 = 0.85;
const BIAS: f64 = 2.8;
/// The decision-directed weight.
const DD: f64 = 0.98;

/// A bin's running state.
#[derive(Clone, Copy)]
struct Bin {
    /// The smoothed power, its minimum in the current sub-window and in each of the last [`WINDOWS`].
    smooth: f64,
    current: f64,
    minima: [f64; WINDOWS],
    /// The last frame's `G²·γ`.
    prior: f64,
    /// The learned noise power.
    profile: f64,
}

impl Bin {
    const REST: Self = Self {
        smooth: 0.0,
        current: f64::INFINITY,
        minima: [f64::INFINITY; WINDOWS],
        prior: 0.0,
        profile: 0.0,
    };
}

/// Denoise's state.
pub struct DenoiseState {
    /// The last [`FRAME`] inputs (left, right), a ring whose oldest is at `at`.
    input: Box<[[f32; 2]]>,
    at: usize,
    /// The overlap-add: its first [`HOP`] samples are done, played from `played`.
    output: Box<[[f32; 2]]>,
    played: usize,
    window: Box<[f64]>,
    re: Box<[f64]>,
    im: Box<[f64]>,
    bins: Box<[Bin]>,
    /// Frames run, the current sub-window's frames, its slot, and the frames a sub-window spans.
    frames: u64,
    in_window: usize,
    slot: usize,
    window_frames: usize,
    floor_gain: f64,
    theta: f64,
    learn: bool,
    /// Frames in the profile.
    learned: u64,
    /// The last frame's reduction over all bins, dB (positive).
    reduction_db: f32,
}

impl DenoiseState {
    pub fn new(params: &[f32], sample_rate: f32) -> Self {
        let n = FRAME as f64;
        let mut state = Self {
            input: vec![[0.0; 2]; FRAME].into(),
            at: 0,
            output: vec![[0.0; 2]; FRAME].into(),
            played: 0,
            window: (0..FRAME)
                .map(|i| 0.5 - 0.5 * (std::f64::consts::TAU * i as f64 / n).cos())
                .collect(),
            re: vec![0.0; FRAME].into(),
            im: vec![0.0; FRAME].into(),
            bins: vec![Bin::REST; BINS].into(),
            frames: 0,
            in_window: 0,
            slot: 0,
            window_frames: 1,
            floor_gain: 1.0,
            theta: 1.0,
            learn: false,
            learned: 0,
            reduction_db: 0.0,
        };
        state.refresh(params, sample_rate);
        state
    }

    /// Takes the params. Learn coming on starts a new profile.
    pub fn refresh(&mut self, params: &[f32], sample_rate: f32) {
        let v = |i: usize| f64::from(params.get(i).copied().unwrap_or(DEFAULTS[i]));
        self.floor_gain = 10f64.powf(-40.0 * v(REDUCTION) / 20.0);
        self.theta = 10f64.powf(18.0f64.mul_add(v(SENSITIVITY), -6.0) / 10.0);
        let learn = v(LEARN) >= 0.5;
        if learn && !self.learn {
            self.learned = 0;
        }
        self.learn = learn;
        let frames = SPAN_S * sample_rate / (HOP * WINDOWS) as f32;
        self.window_frames = (frames.round() as usize).max(1);
    }

    /// Silence and a new noise floor; a learned profile stays.
    pub fn reset(&mut self) {
        self.input.fill([0.0; 2]);
        self.output.fill([0.0; 2]);
        (self.at, self.played) = (0, 0);
        for bin in self.bins.iter_mut() {
            *bin = Bin {
                profile: bin.profile,
                ..Bin::REST
            };
        }
        (self.frames, self.in_window, self.slot) = (0, 0, 0);
        self.reduction_db = 0.0;
    }

    /// The output's delay, samples.
    pub fn latency(&self) -> usize {
        FRAME
    }

    /// The last frame's reduction over all bins, dB (positive), for the meter.
    pub fn reduction_db(&self) -> f32 {
        self.reduction_db
    }

    /// One frame of `l` and `r` in; the frame [`FRAME`] samples ago out.
    pub fn process_frame(&mut self, l: f32, r: f32) -> (f32, f32) {
        self.input[self.at] = [l, r];
        self.at = (self.at + 1) % FRAME;
        let [out_l, out_r] = self.output[self.played];
        self.played += 1;
        if self.played == HOP {
            self.played = 0;
            self.stft_frame();
        }
        (out_l, out_r)
    }

    /// The last [`FRAME`] inputs through the FFT, the gains and back, overlap-added (the module doc).
    fn stft_frame(&mut self) {
        self.output.copy_within(HOP.., 0);
        self.output[FRAME - HOP..].fill([0.0; 2]);
        for i in 0..FRAME {
            let [l, r] = self.input[(self.at + i) % FRAME];
            (self.re[i], self.im[i]) =
                (f64::from(l) * self.window[i], f64::from(r) * self.window[i]);
        }
        fft(&mut self.re, &mut self.im);
        // The minima move on a sub-window.
        self.in_window += 1;
        let rotate = self.in_window >= self.window_frames;
        if rotate {
            self.in_window = 0;
        }
        if self.learn {
            self.learned += 1;
        }
        // The first frames hold the silence before the input began: none of it is the floor.
        let filling = self.frames < (FRAME / HOP) as u64;
        self.frames += 1;
        let (mut before, mut after) = (0.0, 0.0);
        for k in 0..BINS {
            let mirror = (FRAME - k) % FRAME;
            let power = 0.5
                * (self.re[k].powi(2)
                    + self.im[k].powi(2)
                    + self.re[mirror].powi(2)
                    + self.im[mirror].powi(2));
            let bin = &mut self.bins[k];
            (bin.smooth, bin.current) = if filling {
                (power, power)
            } else {
                let smooth = SMOOTH.mul_add(bin.smooth, (1.0 - SMOOTH) * power);
                (smooth, bin.current.min(smooth))
            };
            if self.learn {
                bin.profile += (power - bin.profile) / self.learned as f64;
            }
            let floor = if self.learned > 0 {
                bin.profile
            } else {
                BIAS * bin.minima.iter().fold(bin.current, |a, &b| a.min(b))
            };
            if rotate {
                bin.minima[self.slot] = bin.current;
                bin.current = bin.smooth;
            }
            let gamma = power / (self.theta * floor).max(f64::MIN_POSITIVE);
            let xi = DD.mul_add(bin.prior, (1.0 - DD) * (gamma - 1.0).max(0.0));
            let gain = (xi / (1.0 + xi)).max(self.floor_gain);
            bin.prior = gain * gain * gamma;
            (before, after) = (before + power, gain.mul_add(gain * power, after));
            self.re[k] *= gain;
            self.im[k] *= gain;
            // Bins 0 and FRAME/2 are their own mirrors.
            if mirror != k {
                self.re[mirror] *= gain;
                self.im[mirror] *= gain;
            }
        }
        if rotate {
            self.slot = (self.slot + 1) % WINDOWS;
        }
        self.reduction_db = if before > 0.0 {
            (-10.0 * (after / before).log10()) as f32
        } else {
            0.0
        };
        // The inverse: conjugate, the forward FFT, conjugate (the imaginary part's sign) and 1/FRAME.
        for x in self.im.iter_mut() {
            *x = -*x;
        }
        fft(&mut self.re, &mut self.im);
        let scale = 1.0 / (1.5 * FRAME as f64);
        for (i, out) in self.output.iter_mut().enumerate() {
            let w = self.window[i] * scale;
            out[0] += (self.re[i] * w) as f32;
            out[1] -= (self.im[i] * w) as f32;
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{AudioEffect, ProcessContext};

    const RATE: f32 = 48_000.0;
    /// The measured span's FFT.
    const SPAN: usize = 16_384;

    /// `x`'s power between `lo` and `hi` Hz (a Hann-windowed FFT of its first [`SPAN`] samples).
    fn band_power(x: &[f32], lo: f32, hi: f32) -> f64 {
        let n = SPAN as f64;
        let mut re: Vec<f64> = (x[..SPAN].iter().enumerate())
            .map(|(i, &v)| {
                f64::from(v) * (0.5 - 0.5 * (std::f64::consts::TAU * i as f64 / n).cos())
            })
            .collect();
        let mut im = vec![0.0; SPAN];
        fft(&mut re, &mut im);
        let bin = |hz: f32| (hz / RATE * SPAN as f32).round() as usize;
        (bin(lo)..=bin(hi))
            .map(|k| re[k].powi(2) + im[k].powi(2))
            .sum()
    }

    /// White noise at about −46 dBFS for 5 s, a 1 kHz tone at −20 dBFS over its last second; with `learn`, Learn is on
    /// over the first second. Reduction 20 dB. The noise floor's change (2–16 kHz) and the tone's (980–1020 Hz) over
    /// the last 0.34 s, dB.
    fn run(learn: bool) -> (f64, f64) {
        let mut effect = AudioEffect::new(DENOISE, RATE).unwrap();
        effect.set_parameter(REDUCTION, 0.5).unwrap();
        let context = ProcessContext {
            sample_rate: RATE,
            tempo_bpm: 120.0,
            start_beat: 0.0,
            playing: true,
        };
        let mut seed = 0x2545_f491_u32;
        let input: Vec<f32> = (0..5 * 48_000)
            .map(|i| {
                seed ^= seed << 13;
                seed ^= seed >> 17;
                seed ^= seed << 5;
                let noise = 0.017 * (seed as f32 / u32::MAX as f32 - 0.5);
                let tone = (std::f32::consts::TAU * 1000.0 * i as f32 / RATE).sin();
                noise + if i >= 4 * 48_000 { 0.1 * tone } else { 0.0 }
            })
            .collect();
        let (mut left, mut right) = (input.clone(), input.clone());
        for (n, (l, r)) in left.chunks_mut(480).zip(right.chunks_mut(480)).enumerate() {
            effect
                .set_parameter(LEARN, if learn && n < 100 { 1.0 } else { 0.0 })
                .unwrap();
            effect.process(l, r, context).unwrap();
        }
        assert_eq!(effect.latency_samples(), FRAME);
        // The output is FRAME samples late.
        let at = 5 * 48_000 - SPAN;
        let (wet, dry) = (&left[at..], &input[at - FRAME..]);
        let db = |lo, hi| 10.0 * (band_power(wet, lo, hi) / band_power(dry, lo, hi)).log10();
        (db(2000.0, 16_000.0), db(980.0, 1020.0))
    }

    #[test]
    fn the_noise_floor_drops_by_the_reduction_and_a_tone_stays() {
        for learn in [false, true] {
            let (noise, tone) = run(learn);
            // The full 20 dB (to a tenth), the tone untouched.
            assert!(noise < -19.9, "learn {learn}: noise {noise} dB");
            assert!(tone.abs() < 0.1, "learn {learn}: tone {tone} dB");
        }
    }
}
