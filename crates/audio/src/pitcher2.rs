use std::f32::consts::{PI, TAU};

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
    analysis: Vec<f32>,
    analysis_index: usize,
    analysis_count: usize,
    decimation: usize,
    decimation_count: usize,
    scores: Vec<f32>,
    tracked_frequency: f32,
    target_frequency: f32,
    stable_target: f32,
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
            analysis: vec![0.0; 2048],
            analysis_index: 0,
            analysis_count: 0,
            decimation: 1,
            decimation_count: 0,
            scores: Vec::new(),
            tracked_frequency: 0.0,
            target_frequency: 0.0,
            stable_target: 0.0,
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
            self.sample_rate = sample_rate;
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
            self.decimation = ((sample_rate / 12_000.0).floor() as usize).max(1);
            self.scores.resize(
                (sample_rate / self.decimation as f32 / 65.0).ceil() as usize + 2,
                0.0,
            );
            self.reset();
        }
        self.params.copy_from_slice(params);
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
        let desired_ratio = if self.tracked_frequency > 0.0 && self.target_frequency > 0.0 {
            let attraction = 1.0 - 0.5 * self.params[20];
            (self.target_frequency / self.tracked_frequency)
                .powf(attraction)
                .clamp(0.5, 2.0)
        } else {
            1.0
        };
        let coefficient = (self.speed_factor() / self.sample_rate).min(1.0);
        self.ratio += coefficient * (desired_ratio - self.ratio);
        self.vibrato_phase = (self.vibrato_phase + 5.0 / self.sample_rate).fract();
        let vibrato = 2.0_f32.powf(self.params[24] * 0.3 * (TAU * self.vibrato_phase).sin() / 12.0);
        if self.tracked_frequency > 0.0 && self.params[0] >= 0.5 {
            self.grain_countdown -= 1.0;
            if self.grain_countdown <= 0.0 {
                self.emit_grain();
                self.grain_countdown += (self.sample_rate
                    / (self.tracked_frequency * self.ratio * vibrato))
                    .clamp(8.0, self.sample_rate / 65.0);
            }
        } else {
            self.grain_countdown = 0.0;
        }
        let output_index = self.frames % self.output[0].len();
        let weight = self.weights[output_index];
        let dry_index = (index + self.history[0].len() - self.latency % self.history[0].len())
            % self.history[0].len();
        let dry = [self.history[0][dry_index], self.history[1][dry_index]];
        let mix = if self.params[0] >= 0.5 {
            self.params[22]
        } else {
            0.0
        };
        let wet = if weight > 1e-5 {
            [
                self.output[0][output_index] / weight,
                self.output[1][output_index] / weight,
            ]
        } else {
            dry
        };
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
        let search_start = period;
        let search_end = period * 2;
        let mut lag = search_start;
        let mut peak = f32::NEG_INFINITY;
        for candidate in search_start..=search_end {
            let value = self.read_history(2, candidate);
            if value > peak {
                peak = value;
                lag = candidate;
            }
        }
        let radius = period.min(self.latency.saturating_sub(lag + 1));
        if radius < 8 {
            return;
        }
        let output_len = self.output[0].len();
        for offset in -(radius as isize)..=(radius as isize) {
            let phase = (offset + radius as isize) as f32 / (2 * radius) as f32;
            let weight = (PI * phase).sin().powi(2);
            let source_lag = (lag as isize - offset) as usize;
            let output_time =
                (self.frames as isize + self.latency as isize - period as isize + offset) as usize;
            let output_index = output_time % output_len;
            for channel in 0..2 {
                self.output[channel][output_index] +=
                    self.read_history(channel, source_lag) * weight;
            }
            self.weights[output_index] += weight;
        }
    }

    fn read_history(&self, channel: usize, lag: usize) -> f32 {
        let history = &self.history[channel];
        history[(self.write_index + history.len() - lag % history.len()) % history.len()]
    }

    fn observe_pitch(&mut self, sample: f32) {
        self.decimation_count += 1;
        if self.decimation_count == self.decimation {
            self.decimation_count = 0;
            self.analysis[self.analysis_index] = sample;
            self.analysis_index = (self.analysis_index + 1) % self.analysis.len();
            self.analysis_count = (self.analysis_count + 1).min(self.analysis.len());
        }
        if !self.frames.is_multiple_of(256) {
            return;
        }
        let rate = self.sample_rate / self.decimation as f32;
        let min_lag = (rate / 1000.0).floor().max(2.0) as usize;
        let max_lag = (rate / 65.0).ceil() as usize;
        let window = max_lag * 2;
        if self.analysis_count < window + max_lag {
            return;
        }
        let mut best_lag = 0;
        let mut best_score = 0.15 + 0.3 / self.sensitivity_factor();
        for lag in min_lag..=max_lag {
            let mut cross = 0.0;
            let mut energy_a = 0.0;
            let mut energy_b = 0.0;
            for i in 0..window {
                let a = self.analysis
                    [(self.analysis_index + self.analysis.len() - 1 - i) % self.analysis.len()];
                let b = self.analysis[(self.analysis_index + self.analysis.len() - 1 - i - lag)
                    % self.analysis.len()];
                cross += a * b;
                energy_a += a * a;
                energy_b += b * b;
            }
            if energy_a < 0.001 || energy_b < 0.001 {
                continue;
            }
            let score = cross / (energy_a * energy_b).sqrt();
            self.scores[lag] = score;
            let preferred = score - 0.0005 * lag as f32;
            if preferred > best_score {
                best_score = preferred;
                best_lag = lag;
            }
        }
        if best_lag > min_lag && best_lag < max_lag {
            let previous = self.scores[best_lag - 1];
            let current = self.scores[best_lag];
            let next = self.scores[best_lag + 1];
            let denominator = previous - 2.0 * current + next;
            let offset = if denominator.abs() > 1e-6 {
                0.5 * (previous - next) / denominator
            } else {
                0.0
            };
            self.tracked_frequency = rate / (best_lag as f32 + offset.clamp(-0.5, 0.5));
        } else if best_lag == 0 {
            self.tracked_frequency = 0.0;
            self.stable_target = 0.0;
        }
    }

    pub fn reset(&mut self) {
        for channel in &mut self.history {
            channel.fill(0.0);
        }
        for channel in &mut self.output {
            channel.fill(0.0);
        }
        self.weights.fill(0.0);
        self.analysis.fill(0.0);
        self.write_index = 0;
        self.frames = 0;
        self.analysis_index = 0;
        self.analysis_count = 0;
        self.decimation_count = 0;
        self.tracked_frequency = 0.0;
        self.target_frequency = 0.0;
        self.stable_target = 0.0;
        self.ratio = 1.0;
        self.grain_countdown = 0.0;
        self.vibrato_phase = 0.0;
    }
}

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
