//! De-esser (module type [`DE_ESSER`]), Honeydew's own effect: GitHub #43. A
//! split-band compressor on the sibilance band.
//!
//! **Params**, normalized, in this order (the saved one): 0 Enable, [`FREQUENCY`], [`THRESHOLD`], [`RANGE`], [`MODE`],
//! [`LISTEN`]. Post Gain, Mix and Wet Pan are the rack's (`CommonModule`). The laws from `v`:
//! - Frequency `2000·6^v` Hz (2 to 12 kHz). Threshold `−60 + 60v` dB. Range `24v` dB, the most it reduces.
//! - Mode `round(v)`: 0 Split, 1 Wide. Listen: on from 0.5.
//!
//! **Bands.** The Multiband Compressor's Linkwitz–Riley crossover splits the input at Frequency; the high band is the
//! sibilance band. The two bands sum to an allpass, so the split is flat while nothing is reduced.
//!
//! **Detector.** The louder channel's `|high|`, followed by an envelope that rises in 0.5 ms and falls in 60 ms
//! (one-pole). With `over` its level in dB above Threshold, the reduction is `clamp(over, 0, Range)` dB: the band is
//! held at the threshold until Range is spent (the Dynamic EQ's law with a negative Range).
//!
//! **Mode.** Split reduces the high band alone (`low + g·high`), Wide the whole input (`g·x`), `g` the reduction as a
//! gain. Listen plays the detector's band (`high`) alone, unreduced; the meter still shows the reduction.
//!
//! Nothing allocates while it runs.

use crate::biquad::flush;
use crate::multiband::Splitter;

/// The module type: Honeydew's own id, which a song file keeps apart from the song format's modules.
pub const DE_ESSER: u16 = 1006;

pub const FREQUENCY: usize = 1;
pub const THRESHOLD: usize = 2;
pub const RANGE: usize = 3;
pub const MODE: usize = 4;
pub const LISTEN: usize = 5;
/// Enable and the five above.
pub const PARAMS: usize = 6;

/// A new module's params: 6 kHz, Threshold −30 dB, Range 12 dB, Split, not listening.
pub const DEFAULTS: [f32; PARAMS] = [1.0, 0.613_147, 0.5, 0.5, 0.0, 0.0];

/// Frequency, Hz.
pub fn hz(v: f32) -> f32 {
    2000.0 * 6f32.powf(v)
}

/// The De-esser's state.
pub struct DeEsserState {
    split: Splitter,
    threshold_db: f32,
    range_db: f32,
    wide: bool,
    listen: bool,
    /// The envelope's coefficients.
    attack: f32,
    release: f32,
    envelope: f32,
    /// The reduction now, dB (positive).
    reduction_db: f32,
}

impl DeEsserState {
    pub fn new(params: &[f32], sample_rate: f32) -> Self {
        let mut state = Self {
            split: Splitter::new(hz(DEFAULTS[FREQUENCY]), sample_rate),
            threshold_db: 0.0,
            range_db: 0.0,
            wide: false,
            listen: false,
            attack: 0.0,
            release: 0.0,
            envelope: 0.0,
            reduction_db: 0.0,
        };
        state.refresh(params, sample_rate);
        state
    }

    /// Takes the params: coefficients only, no sample.
    pub fn refresh(&mut self, params: &[f32], sample_rate: f32) {
        let v = |i: usize| params.get(i).copied().unwrap_or(DEFAULTS[i]);
        self.split.set_frequency(hz(v(FREQUENCY)), sample_rate);
        self.threshold_db = 60.0f32.mul_add(v(THRESHOLD), -60.0);
        self.range_db = 24.0 * v(RANGE);
        self.wide = v(MODE) >= 0.5;
        self.listen = v(LISTEN) >= 0.5;
        let coef = |ms: f32| (-1000.0 / (ms * sample_rate)).exp();
        (self.attack, self.release) = (coef(0.5), coef(60.0));
    }

    pub fn reset(&mut self) {
        self.split.clear();
        (self.envelope, self.reduction_db) = (0.0, 0.0);
    }

    /// The reduction now, dB (positive), for the meter.
    pub fn reduction_db(&self) -> f32 {
        self.reduction_db
    }

    /// One frame (the module doc's **Detector** and **Mode**).
    pub fn process_frame(&mut self, l: f32, r: f32) -> (f32, f32) {
        let ((low_l, low_r), (high_l, high_r)) = self.split.process(l, r);
        let level = high_l.abs().max(high_r.abs());
        let coef = if level > self.envelope {
            self.attack
        } else {
            self.release
        };
        self.envelope = flush(level + coef * (self.envelope - level));
        let over = 20.0 * self.envelope.max(1e-6).log10() - self.threshold_db;
        self.reduction_db = over.clamp(0.0, self.range_db);
        if self.listen {
            return (high_l, high_r);
        }
        let g = 10f32.powf(-self.reduction_db / 20.0);
        if self.wide {
            (g * l, g * r)
        } else {
            (g.mul_add(high_l, low_l), g.mul_add(high_r, low_r))
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{AudioEffect, ProcessContext};

    const RATE: f32 = 48_000.0;

    /// `x`'s amplitude at `hz`, over a whole number of its periods.
    fn amplitude(x: &[f32], hz: f32) -> f32 {
        let (mut c, mut s) = (0.0f64, 0.0f64);
        for (i, &v) in x.iter().enumerate() {
            let phase = f64::from(std::f32::consts::TAU * hz / RATE) * i as f64;
            c += f64::from(v) * phase.cos();
            s += f64::from(v) * phase.sin();
        }
        (2.0 * c.hypot(s) / x.len() as f64) as f32
    }

    #[test]
    fn a_burst_in_the_band_is_reduced_by_the_range_and_the_lows_pass() {
        // Split at 2 kHz, Threshold −30 dB, Range 12 dB. A 200 Hz tone at −6 dBFS throughout, a 10 kHz burst at −12 dBFS
        // (18 dB over) in the second half.
        let mut effect = AudioEffect::new(DE_ESSER, RATE).unwrap();
        for (p, v) in [(FREQUENCY, 0.0), (THRESHOLD, 0.5), (RANGE, 0.5)] {
            effect.set_parameter(p, v).unwrap();
        }
        let context = ProcessContext {
            sample_rate: RATE,
            tempo_bpm: 120.0,
            start_beat: 0.0,
            playing: true,
        };
        let tone = |hz: f32, i: usize| (std::f32::consts::TAU * hz * i as f32 / RATE).sin();
        let input: Vec<f32> = (0..48_000)
            .map(|i| {
                0.5 * tone(200.0, i)
                    + if i >= 24_000 {
                        0.25 * tone(10_000.0, i)
                    } else {
                        0.0
                    }
            })
            .collect();
        let (mut left, mut right) = (input.clone(), input.clone());
        let mut reductions = Vec::new();
        for (l, r) in left.chunks_mut(480).zip(right.chunks_mut(480)) {
            effect.process(l, r, context).unwrap();
            reductions.push(effect.meter().gain_reduction_db);
        }
        // The lows alone reduce nothing; the burst is held 12 dB down, which the meter shows.
        assert!(
            reductions[..50].iter().all(|&db| db == 0.0),
            "{reductions:?}"
        );
        assert!((reductions[99] - 12.0).abs() < 1e-3, "{}", reductions[99]);
        // Over the burst's last quarter second (whole periods of both tones).
        let (tail, dry) = (&left[36_000..], &input[36_000..]);
        let db = |hz| 20.0 * (amplitude(tail, hz) / amplitude(dry, hz)).log10();
        assert!((db(10_000.0) + 12.0).abs() < 0.5, "10 kHz {}", db(10_000.0));
        assert!(db(200.0).abs() < 0.05, "200 Hz {}", db(200.0));
    }
}
