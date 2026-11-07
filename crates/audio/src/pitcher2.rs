use std::f32::consts::{PI, TAU};

use crate::pitcher::{Search, Tracker};

pub const DEFAULT_PARAMS: [f32; 27] = [
    1.0, 1.0, 0.0, 1.0, 1.0, 0.0, 1.0, 0.0, 1.0, 1.0, 0.0, 1.0, 0.0, 0.5, 0.0, 0.0, 0.0, 0.5, 1.0,
    0.5, 0.0, 0.0, 1.0, 0.5, 0.0, 0.0, 0.0,
];

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Pitcher2Error {
    InvalidValue,
    InvalidSampleRate,
    MismatchedBuffers,
}

pub struct Pitcher2State {
    params: [f32; 27],
    sample_rate: f32,
    window: usize,
    latency: usize,
    history: [Vec<f32>; 3],
    output: [Vec<f32>; 2],
    weights: Vec<f32>,
    write_index: usize,
    frames: usize,
    tracker: Tracker,
    tracked_frequency: f32,
    target_frequency: f32,
    stable_target: f32,
    /// The ratio the last analysis aims at, and the per-frame glide toward it (from Speed).
    desired_ratio: f32,
    coefficient: f32,
    ratio: f32,
    grain_countdown: f32,
    vibrato_phase: f32,
}

impl Pitcher2State {
    pub fn new(params: &[f32], sample_rate: f32) -> Result<Self, Pitcher2Error> {
        validate(params, sample_rate)?;
        let mut state = Self {
            params: DEFAULT_PARAMS,
            // No rate yet: `refresh` sizes the buffers.
            sample_rate: 0.0,
            window: 0,
            latency: 0,
            history: Default::default(),
            output: Default::default(),
            weights: Vec::new(),
            write_index: 0,
            frames: 0,
            tracker: Tracker::new(sample_rate, LOWEST),
            tracked_frequency: 0.0,
            target_frequency: 0.0,
            stable_target: 0.0,
            desired_ratio: 1.0,
            coefficient: 0.0,
            ratio: 1.0,
            grain_countdown: 0.0,
            vibrato_phase: 0.0,
        };
        state.refresh(params, sample_rate)?;
        Ok(state)
    }

    pub fn parameters(&self) -> &[f32; 27] {
        &self.params
    }

    pub fn latency_samples(&self) -> usize {
        self.latency
    }

    pub fn analysis_window_samples(&self) -> usize {
        self.window
    }

    pub fn tracked_frequency(&self) -> f32 {
        self.tracked_frequency
    }

    pub fn target_frequency(&self) -> f32 {
        self.target_frequency
    }

    pub fn reference_channel(&self) -> u8 {
        (2.0 * self.params[18]).trunc() as u8
    }

    pub fn formant_shift(&self) -> f32 {
        2.0 * self.params[17] - 1.0
    }

    /// How far Formants moves the formants: 2^shift (½ to 2 over F-Shift's −100 … +100 %) while on, else 1, the
    /// grains' own (PSOLA keeps them where the voice has them).
    pub fn formant_factor(&self) -> f32 {
        if self.params[16] >= 0.5 {
            2.0_f32.powf(self.formant_shift())
        } else {
            1.0
        }
    }

    pub fn speed_factor(&self) -> f32 {
        200.0_f32.powf(self.params[13]) - 0.9
    }

    pub fn sensitivity_factor(&self) -> f32 {
        200.0_f32.powf(self.params[23])
    }

    pub fn refresh(&mut self, params: &[f32], sample_rate: f32) -> Result<(), Pitcher2Error> {
        validate(params, sample_rate)?;
        let window = analysis_window(params[25], sample_rate);
        if self.sample_rate != sample_rate || self.window != window {
            self.window = window;
            self.latency = latency_for(window);
            // Room for the larger (normal) window at this rate, so a Low Latency switch on the audio thread never
            // allocates; only a rate change (off the audio thread) does.
            let largest = latency_for(analysis_window(0.0, sample_rate)) * 4 + 4096;
            let history_len = (self.latency * 4 + 4096).max(8192);
            let output_len = self.latency * 4 + 4096;
            for channel in &mut self.history {
                channel.resize(history_len, 0.0);
                channel.reserve(largest.max(8192) - history_len);
            }
            for channel in self.output.iter_mut().chain([&mut self.weights]) {
                channel.resize(output_len, 0.0);
                channel.reserve(largest - output_len);
            }
            if self.sample_rate != sample_rate {
                self.tracker = Tracker::new(sample_rate, LOWEST);
            }
            self.sample_rate = sample_rate;
            self.reset();
        }
        self.params.copy_from_slice(params);
        self.coefficient = (self.speed_factor() / sample_rate).min(1.0);
        Ok(())
    }

    pub fn target_for(&self, frequency: f32) -> f32 {
        if !frequency.is_finite() || frequency <= 0.0 {
            return 0.0;
        }
        let midi = 69.0 + 12.0 * (frequency / 440.0).log2();
        let center = midi.round() as i32;
        let best = (-12..=12)
            .map(|offset| center + offset)
            .filter(|note| self.params[1 + note.rem_euclid(12) as usize] > 0.5)
            .min_by(|a, b| {
                (midi - *a as f32)
                    .abs()
                    .total_cmp(&(midi - *b as f32).abs())
            });
        best.map_or(frequency, |note| {
            440.0 * 2.0_f32.powf((note as f32 - 69.0) / 12.0)
        })
    }

    pub fn process(&mut self, left: &mut [f32], right: &mut [f32]) -> Result<(), Pitcher2Error> {
        if left.len() != right.len() {
            return Err(Pitcher2Error::MismatchedBuffers);
        }
        for (l, r) in left.iter_mut().zip(right) {
            (*l, *r) = self.process_frame(*l, *r);
        }
        Ok(())
    }

    pub fn process_frame(&mut self, left: f32, right: f32) -> (f32, f32) {
        let reference = match self.reference_channel() {
            0 => left,
            1 => right,
            _ => 0.5 * (left + right),
        };
        let index = self.write_index;
        self.history[0][index] = left;
        self.history[1][index] = right;
        self.history[2][index] = reference;
        self.observe_pitch(reference);
        self.ratio += self.coefficient * (self.desired_ratio - self.ratio);
        self.vibrato_phase = (self.vibrato_phase + 5.0 / self.sample_rate).fract();
        if self.tracked_frequency > 0.0 && self.params[0] >= 0.5 {
            self.grain_countdown -= 1.0;
            if self.grain_countdown <= 0.0 {
                self.emit_grain();
                let vibrato =
                    2.0_f32.powf(self.params[24] * 0.3 * (TAU * self.vibrato_phase).sin() / 12.0);
                self.grain_countdown += (self.sample_rate
                    / (self.tracked_frequency * self.ratio * vibrato))
                    .clamp(8.0, self.sample_rate / 65.0);
            }
        } else {
            self.grain_countdown = 0.0;
        }
        let output_index = self.frames % self.output[0].len();
        let weight = self.weights[output_index];
        let dry_index = self.back(self.latency);
        let dry = [self.history[0][dry_index], self.history[1][dry_index]];
        let mix = if self.params[0] >= 0.5 {
            self.params[22]
        } else {
            0.0
        };
        // The grains, normalized by their weight; where they thin out (a voice's start and end) they fade to the dry
        // signal instead of switching to it, which clicked.
        let fade = (1.0 - weight / FULL).max(0.0);
        let wet = [0, 1].map(|c| self.output[c][output_index] / weight.max(FULL) + fade * dry[c]);
        let result = (
            dry[0] + mix * (wet[0] - dry[0]),
            dry[1] + mix * (wet[1] - dry[1]),
        );
        self.output[0][output_index] = 0.0;
        self.output[1][output_index] = 0.0;
        self.weights[output_index] = 0.0;
        self.write_index = (index + 1) % self.history[0].len();
        self.frames += 1;
        result
    }

    fn emit_grain(&mut self) {
        let period = (self.sample_rate / self.tracked_frequency).round() as usize;
        let period = period.clamp(8, self.window / 2);
        // The grain is read `formant` times as fast as it plays, which moves its formants by that factor; its pitch
        // mark lies far enough back for the faster read.
        let formant = self.formant_factor();
        let search_start = (period as f32 * formant.max(1.0)).ceil() as usize;
        let mut lag = search_start;
        let mut peak = f32::NEG_INFINITY;
        for candidate in search_start..=search_start + period {
            let value = self.history[2][self.back(candidate)];
            if value > peak {
                peak = value;
                lag = candidate;
            }
        }
        // It plays from `latency − period − radius` frames on, never before the next one.
        let radius = period.min(self.latency.saturating_sub(period + 1));
        if radius < 8 {
            return;
        }
        let output_len = self.output[0].len();
        let mut at = (self.frames + self.latency - period - radius) % output_len;
        for offset in -(radius as isize)..=(radius as isize) {
            let phase = (offset + radius as isize) as f32 / (2 * radius) as f32;
            let weight = (PI * phase).sin().powi(2);
            let source = lag as f32 - offset as f32 * formant;
            let (whole, fraction) = (source as usize, source.fract());
            let (newer, older) = (self.back(whole), self.back(whole + 1));
            for channel in 0..2 {
                let history = &self.history[channel];
                let sample = history[newer] + (history[older] - history[newer]) * fraction;
                self.output[channel][at] += sample * weight;
            }
            self.weights[at] += weight;
            at = if at + 1 == output_len { 0 } else { at + 1 };
        }
    }

    /// The history index `lag` frames back; `lag` is under the history's length.
    fn back(&self, lag: usize) -> usize {
        let len = self.history[0].len();
        if lag > self.write_index {
            self.write_index + len - lag
        } else {
            self.write_index - lag
        }
    }

    fn observe_pitch(&mut self, sample: f32) {
        self.tracker.push(sample);
        if !self.frames.is_multiple_of(256) {
            return;
        }
        match self
            .tracker
            .search(1000.0, LOWEST, 0.15 + 0.3 / self.sensitivity_factor())
        {
            Search::Pitch(hz) => self.tracked_frequency = hz,
            Search::Unvoiced => {
                self.tracked_frequency = 0.0;
                self.stable_target = 0.0;
            }
            Search::Waiting | Search::Edge => {}
        }
        self.hold_target();
    }

    /// The target the analysis leads to, held while Stability covers a new one, and the ratio toward it.
    fn hold_target(&mut self) {
        let target = self.target_for(self.tracked_frequency);
        if target > 0.0 {
            let hysteresis = self.params[19] * 0.5;
            if self.stable_target == 0.0
                || (12.0 * (target / self.stable_target).log2()).abs() > hysteresis
            {
                self.stable_target = target;
            }
            self.target_frequency = self.stable_target;
        } else {
            self.target_frequency = 0.0;
        }
        self.desired_ratio = if self.tracked_frequency > 0.0 && self.target_frequency > 0.0 {
            let attraction = 1.0 - 0.5 * self.params[20];
            (self.target_frequency / self.tracked_frequency)
                .powf(attraction)
                .clamp(0.5, 2.0)
        } else {
            1.0
        };
    }

    pub fn reset(&mut self) {
        for channel in &mut self.history {
            channel.fill(0.0);
        }
        for channel in &mut self.output {
            channel.fill(0.0);
        }
        self.weights.fill(0.0);
        self.tracker.reset();
        self.write_index = 0;
        self.frames = 0;
        self.tracked_frequency = 0.0;
        self.target_frequency = 0.0;
        self.stable_target = 0.0;
        self.desired_ratio = 1.0;
        self.ratio = 1.0;
        self.grain_countdown = 0.0;
        self.vibrato_phase = 0.0;
    }
}

/// The grain weight from which the grains play alone; under it they fade to the dry signal. Overlapping grains weigh
/// about 1, and over 0.25 for corrections up to 6 semitones down.
const FULL: f32 = 0.25;

/// The lowest pitch Pitcher 2 tracks, Hz.
const LOWEST: f32 = 65.0;

fn validate(params: &[f32], sample_rate: f32) -> Result<(), Pitcher2Error> {
    if !sample_rate.is_finite() || sample_rate < 2000.0 {
        return Err(Pitcher2Error::InvalidSampleRate);
    }
    if params.len() != 27
        || params
            .iter()
            .any(|value| !value.is_finite() || !(0.0..=1.0).contains(value))
    {
        return Err(Pitcher2Error::InvalidValue);
    }
    Ok(())
}

fn analysis_window(low_latency: f32, sample_rate: f32) -> usize {
    let nominal = if low_latency >= 0.5 { 1024.0 } else { 2048.0 };
    (nominal * 0.000022676_f32 * sample_rate).trunc() as usize
}

fn latency_for(window: usize) -> usize {
    (1.2 * window as f32).trunc() as usize
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn latency_switch_uses_prepared_storage() {
        let mut low = DEFAULT_PARAMS;
        low[25] = 1.0;
        let mut state = Pitcher2State::new(&low, 48_000.0).unwrap();
        let history_capacity = state.history[0].capacity();
        let output_capacity = state.output[0].capacity();
        let weights_capacity = state.weights.capacity();
        low[25] = 0.0;
        state.refresh(&low, 48_000.0).unwrap();
        assert_eq!(state.history[0].capacity(), history_capacity);
        assert_eq!(state.output[0].capacity(), output_capacity);
        assert_eq!(state.weights.capacity(), weights_capacity);
    }
}
