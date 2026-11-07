use std::f32::consts::TAU;

pub const DEFAULT_PARAMS: [f32; 23] = [
    1.0, 0.0, 0.5, 0.5, 0.0, 0.5, 0.0, 0.5, 0.0, 1.0, 0.0, 0.0, 0.0, 0.0, 0.0, 0.0, 0.0, 0.0, 0.0,
    0.0, 0.0, 0.0, 0.0,
];

/// The note enables C…B a new Pitcher snaps to: C minor (`CSTM` bytes 0–11). The panel's
/// note buttons own them; the Scale and Key spinners only rewrite them.
pub const DEFAULT_NOTES: [bool; 12] = [
    true, false, true, true, false, true, false, true, true, false, true, false,
];

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum PitcherError {
    InvalidValue,
    InvalidSampleRate,
    MismatchedBuffers,
}

pub struct PitcherState {
    params: [f32; 23],
    sample_rate: f32,
    latency: usize,
    history: [Vec<f32>; 2],
    formant_history: [Vec<f32>; 2],
    write_index: usize,
    tracker: Tracker,
    frames: usize,
    tracked_frequency: f32,
    /// The target over the tracked frequency, from the last analysis.
    target_ratio: f32,
    ratio: f32,
    phase: f32,
    /// The formant path's one-pole coefficient, from F-Shift.
    formant_coefficient: f32,
    formant_input_low: [f32; 2],
    formant_output_low: [f32; 2],
    /// The pitch classes (0 = C) a tone may snap to.
    notes: [bool; 12],
    stable_target: f32,
    /// The last analysis found a pitch (the tracked frequency holds through silence; this does not).
    voiced: bool,
}

impl PitcherState {
    pub fn new(params: &[f32], sample_rate: f32) -> Result<Self, PitcherError> {
        validate(params, sample_rate)?;
        let latency = latency_for(sample_rate);
        let mut state = Self {
            params: DEFAULT_PARAMS,
            sample_rate,
            latency,
            history: std::array::from_fn(|_| vec![0.0; latency * 3 + 4]),
            formant_history: std::array::from_fn(|_| vec![0.0; latency * 3 + 4]),
            write_index: 0,
            tracker: Tracker::new(sample_rate, LOWEST),
            frames: 0,
            tracked_frequency: 0.0,
            target_ratio: 1.0,
            ratio: 1.0,
            phase: 0.0,
            formant_coefficient: 0.0,
            formant_input_low: [0.0; 2],
            formant_output_low: [0.0; 2],
            notes: DEFAULT_NOTES,
            stable_target: 0.0,
            voiced: false,
        };
        state.refresh(params, sample_rate)?;
        Ok(state)
    }

    pub fn parameters(&self) -> &[f32; 23] {
        &self.params
    }
    pub fn latency_samples(&self) -> usize {
        self.latency
    }
    pub fn tracked_frequency(&self) -> f32 {
        self.tracked_frequency
    }
    /// The tracked frequency while the last analysis found a pitch, else 0.
    pub fn voiced_frequency(&self) -> f32 {
        if self.voiced {
            self.tracked_frequency
        } else {
            0.0
        }
    }
    /// The pitch class (C = 0) of the target the Pitcher holds (its Stability hysteresis keeps it), with Pitch and Fine
    /// Tune taken back out. `None` without a pitch or a held target, while snapping is bypassed for the tracked note,
    /// or with no note enabled.
    pub fn target_class(&self) -> Option<usize> {
        let frequency = self.voiced_frequency();
        if frequency <= 0.0 || self.stable_target <= 0.0 || !self.notes.contains(&true) {
            return None;
        }
        let base = self.base_frequency();
        let midi = 69.0 + 12.0 * (frequency / base).log2();
        let pitch_class = (midi.round() as i32).rem_euclid(12) as usize;
        if self.params[22] > 0.5 || self.params[10 + pitch_class] > 0.5 {
            return None;
        }
        let (semitones, cents) = self.transpose();
        let held = 69.0 + 12.0 * (self.stable_target / base).log2() - semitones - cents;
        Some((held.round() as i32).rem_euclid(12) as usize)
    }
    pub fn input_channel(&self) -> u8 {
        (self.params[6] * 2.0) as u8
    }
    pub fn base_frequency(&self) -> f32 {
        440.0 * 2.0_f32.powf((2.0 * self.params[7] - 1.0) / 12.0)
    }
    pub fn formant_ratio(&self) -> f32 {
        2.0_f32.powf(2.0 * self.params[5] - 1.0)
    }

    pub fn notes(&self) -> [bool; 12] {
        self.notes
    }

    /// The pitch classes (0 = C) a tone may snap to; none snaps nowhere.
    pub fn set_notes(&mut self, notes: [bool; 12]) {
        // A re-sent mask keeps the held target.
        if notes == self.notes {
            return;
        }
        self.notes = notes;
        self.stable_target = 0.0;
    }

    pub fn refresh(&mut self, params: &[f32], sample_rate: f32) -> Result<(), PitcherError> {
        validate(params, sample_rate)?;
        if self.sample_rate != sample_rate {
            self.sample_rate = sample_rate;
            self.latency = latency_for(sample_rate);
            let length = self.latency * 3 + 4;
            for channel in 0..2 {
                self.history[channel].resize(length, 0.0);
                self.formant_history[channel].resize(length, 0.0);
            }
            self.tracker = Tracker::new(sample_rate, LOWEST);
            self.reset();
        }
        self.params.copy_from_slice(params);
        let cutoff = (500.0 * self.formant_ratio()).min(0.45 * sample_rate);
        self.formant_coefficient = (1.0 - (-TAU * cutoff / sample_rate).exp()).clamp(0.0, 1.0);
        Ok(())
    }

    pub fn target_for(&self, frequency: f32) -> f32 {
        if !frequency.is_finite() || frequency <= 0.0 {
            return 0.0;
        }
        let midi = 69.0 + 12.0 * (frequency / self.base_frequency()).log2();
        let pitch_class = (midi.round() as i32).rem_euclid(12) as usize;
        let bypass = self.params[22] > 0.5 || self.params[10 + pitch_class] > 0.5;
        let target_note = if bypass {
            midi
        } else {
            self.nearest_allowed_note(midi)
        };
        let (semitones, cents) = self.transpose();
        self.base_frequency() * 2.0_f32.powf((target_note + semitones + cents - 69.0) / 12.0)
    }

    /// Pitch in whole semitones and Fine Tune in whole cents (as semitones): trunc(f64(v)·24 − 12) and
    /// trunc(f64(v)·100 − 50), exact in f64 like the text:
    /// f32(1/24) is −10 semitones, not −11.
    fn transpose(&self) -> (f32, f32) {
        let semitones = (24.0 * f64::from(self.params[2]) - 12.0).trunc() as f32;
        let cents = (100.0 * f64::from(self.params[3]) - 50.0).trunc() as f32 * 0.01;
        (semitones, cents)
    }

    fn nearest_allowed_note(&self, midi: f32) -> f32 {
        let center = midi.round() as i32;
        (-12..=12)
            .map(|offset| center + offset)
            .filter(|note| self.notes[note.rem_euclid(12) as usize])
            .min_by(|a, b| {
                (midi - *a as f32)
                    .abs()
                    .total_cmp(&(midi - *b as f32).abs())
            })
            .map_or(midi, |note| note as f32)
    }

    pub fn process(&mut self, left: &mut [f32], right: &mut [f32]) -> Result<(), PitcherError> {
        if left.len() != right.len() {
            return Err(PitcherError::MismatchedBuffers);
        }
        for (l, r) in left.iter_mut().zip(right) {
            (*l, *r) = self.process_frame(*l, *r);
        }
        Ok(())
    }

    pub fn process_frame(&mut self, left: f32, right: f32) -> (f32, f32) {
        let mono = match self.input_channel() {
            0 => left,
            1 => 0.5 * (left + right),
            _ => right,
        };
        self.observe_pitch(mono);
        let speed = (1.0 - self.params[1]).powi(3) * 3.0 + 0.001;
        let coefficient = (8.82 / (self.sample_rate * speed)).min(1.0);
        self.ratio += coefficient * (self.target_ratio.clamp(0.5, 2.0) - self.ratio);
        let window = self.latency as f32;
        let phase = self.phase + (1.0 - self.ratio) / window;
        self.phase = phase - phase.floor();
        let formant_coefficient = self.formant_coefficient;
        let mut output = [0.0; 2];
        for (channel, input) in [left, right].into_iter().enumerate() {
            self.formant_input_low[channel] +=
                formant_coefficient * (input - self.formant_input_low[channel]);
            self.history[channel][self.write_index] = input;
            self.formant_history[channel][self.write_index] = self.formant_input_low[channel];
            let dry = read(&self.history[channel], self.write_index, window);
            let shifted = shifted(&self.history[channel], self.write_index, self.phase, window);
            let wet = if self.params[4] != 0.0 {
                self.formant_output_low[channel] +=
                    formant_coefficient * (shifted - self.formant_output_low[channel]);
                let original_low = read(&self.formant_history[channel], self.write_index, window);
                shifted + 0.4 * (original_low - self.formant_output_low[channel])
            } else {
                shifted
            };
            output[channel] = if self.params[0] < 0.5 {
                dry
            } else {
                dry + self.params[9] * (wet - dry)
            };
        }
        self.write_index = (self.write_index + 1) % self.history[0].len();
        // Only its phase within 64 matters (`observe_pitch`): kept inside, so wasm32's usize never overflows.
        self.frames = (self.frames + 1) % 64;
        (output[0], output[1])
    }

    fn observe_pitch(&mut self, sample: f32) {
        self.tracker.push(sample);
        if !self.frames.is_multiple_of(64) {
            return;
        }
        match self.tracker.search(1500.0, LOWEST, 0.3) {
            Search::Waiting => {}
            Search::Pitch(hz) => (self.tracked_frequency, self.voiced) = (hz, true),
            Search::Unvoiced | Search::Edge => self.voiced = false,
        }
        self.target_ratio = self.hold_target();
    }

    /// The target over the tracked frequency, holding the last target while Stability covers the new one.
    fn hold_target(&mut self) -> f32 {
        if self.tracked_frequency <= 0.0 {
            return 1.0;
        }
        let target = self.target_for(self.tracked_frequency);
        if self.params[8] > 0.0
            && self.stable_target > 0.0
            && (12.0 * (target / self.stable_target).log2()).abs() < self.params[8] * 0.5
        {
            self.stable_target / self.tracked_frequency
        } else {
            self.stable_target = target;
            target / self.tracked_frequency
        }
    }

    pub fn reset(&mut self) {
        for channel in 0..2 {
            self.history[channel].fill(0.0);
            self.formant_history[channel].fill(0.0);
        }
        self.tracker.reset();
        self.write_index = 0;
        self.frames = 0;
        self.tracked_frequency = 0.0;
        self.voiced = false;
        self.target_ratio = 1.0;
        self.ratio = 1.0;
        self.phase = 0.0;
        self.formant_input_low = [0.0; 2];
        self.formant_output_low = [0.0; 2];
        self.stable_target = 0.0;
    }
}

/// The lowest pitch the Pitcher tracks, Hz.
const LOWEST: f32 = 55.0;

/// The decimated input the tracker keeps, in samples.
const RING: usize = 2048;

/// What a [`Tracker::search`] found.
#[derive(Clone, Copy, Debug, PartialEq)]
pub(crate) enum Search {
    /// Not enough input yet.
    Waiting,
    /// No lag scored above the threshold.
    Unvoiced,
    /// The best lag is the shortest or the longest searched.
    Edge,
    /// A pitch, Hz.
    Pitch(f32),
}

/// Both Pitchers' pitch tracker: the input decimated to about 12 kHz, searched by normalized autocorrelation over two
/// periods of the lowest pitch. Sized at build; nothing allocates while it runs.
pub(crate) struct Tracker {
    /// The decimated input, each sample written at `i` and `i + RING`, so that the newest ones are one slice.
    ring: Vec<f32>,
    index: usize,
    count: usize,
    decimation: usize,
    decimation_count: usize,
    rate: f32,
    scores: Vec<f32>,
}

impl Tracker {
    /// A tracker at `sample_rate` for pitches down to `lowest` Hz.
    pub(crate) fn new(sample_rate: f32, lowest: f32) -> Self {
        let decimation = ((sample_rate / 12_000.0).floor() as usize).max(1);
        let rate = sample_rate / decimation as f32;
        Self {
            ring: vec![0.0; 2 * RING],
            index: 0,
            count: 0,
            decimation,
            decimation_count: 0,
            rate,
            scores: vec![0.0; (rate / lowest).ceil() as usize + 2],
        }
    }

    /// One input sample.
    pub(crate) fn push(&mut self, sample: f32) {
        self.decimation_count += 1;
        if self.decimation_count == self.decimation {
            self.decimation_count = 0;
            self.ring[self.index] = sample;
            self.ring[self.index + RING] = sample;
            self.index = (self.index + 1) % RING;
            self.count = (self.count + 1).min(RING);
        }
    }

    /// The pitch between `lowest` and `highest` Hz whose lag scores best above `threshold` (a lag's score less 0.0005
    /// per lag, favouring the fundamental), refined between its neighbours.
    pub(crate) fn search(&mut self, highest: f32, lowest: f32, threshold: f32) -> Search {
        let min_lag = (self.rate / highest).floor().max(2.0) as usize;
        let max_lag = ((self.rate / lowest).ceil() as usize).min(self.scores.len() - 2);
        let window = max_lag * 2;
        if self.count < window + max_lag {
            return Search::Waiting;
        }
        let end = self.index + RING;
        let x = &self.ring[end - window - max_lag..end];
        let a = &x[max_lag..];
        let energy_a = dot(a, a);
        // Lag `lag` compares the newest `window` samples with those `lag` before; its energy slides from lag to lag.
        let first = &x[max_lag - min_lag..][..window];
        let mut energy_b = f64::from(dot(first, first));
        let mut best = (0, threshold);
        self.scores.fill(0.0);
        for lag in min_lag..=max_lag {
            let start = max_lag - lag;
            if lag > min_lag {
                energy_b += f64::from(x[start]).powi(2) - f64::from(x[start + window]).powi(2);
            }
            let energy_b = energy_b as f32;
            if energy_a < 0.001 || energy_b < 0.001 {
                continue;
            }
            let score = dot(a, &x[start..start + window]) / (energy_a * energy_b).sqrt();
            self.scores[lag] = score;
            let preferred = score - 0.0005 * lag as f32;
            if preferred > best.1 {
                best = (lag, preferred);
            }
        }
        let mut lag = best.0;
        if lag == 0 {
            return Search::Unvoiced;
        }
        // A strong formant can score a period's whole lags under two periods' (an octave low): halve while the peak
        // near half the lag scores nearly as well.
        while lag / 2 > min_lag {
            let half = lag / 2;
            let at = (half - 1..=half + 1)
                .max_by(|a, b| self.scores[*a].total_cmp(&self.scores[*b]))
                .unwrap_or(half);
            if self.scores[at] < 0.9 * self.scores[lag] {
                break;
            }
            lag = at;
        }
        if lag == min_lag || lag == max_lag {
            return Search::Edge;
        }
        let (previous, current, next) =
            (self.scores[lag - 1], self.scores[lag], self.scores[lag + 1]);
        let denominator = previous - 2.0 * current + next;
        let offset = if denominator.abs() > 1e-6 {
            0.5 * (previous - next) / denominator
        } else {
            0.0
        };
        Search::Pitch(self.rate / (lag as f32 + offset.clamp(-0.5, 0.5)))
    }

    pub(crate) fn reset(&mut self) {
        self.ring.fill(0.0);
        self.index = 0;
        self.count = 0;
        self.decimation_count = 0;
    }
}

/// Σ a·b, in eight lanes so that it vectorizes.
fn dot(a: &[f32], b: &[f32]) -> f32 {
    let ((a, a_tail), (b, b_tail)) = (a.as_chunks::<8>(), b.as_chunks::<8>());
    let tail: f32 = a_tail.iter().zip(b_tail).map(|(x, y)| x * y).sum();
    let mut lanes = [0.0_f32; 8];
    for (a, b) in a.iter().zip(b) {
        for (lane, (x, y)) in lanes.iter_mut().zip(a.iter().zip(b)) {
            *lane += x * y;
        }
    }
    lanes.iter().sum::<f32>() + tail
}

fn validate(params: &[f32], sample_rate: f32) -> Result<(), PitcherError> {
    if !sample_rate.is_finite() || sample_rate < 2000.0 {
        return Err(PitcherError::InvalidSampleRate);
    }
    if params.len() != 23
        || params
            .iter()
            .any(|value| !value.is_finite() || !(0.0..=1.0).contains(value))
    {
        return Err(PitcherError::InvalidValue);
    }
    Ok(())
}

fn latency_for(sample_rate: f32) -> usize {
    ((sample_rate / 55.0) as usize & !3).max(4)
}

/// `buffer` `delay` samples before `write_index`, interpolated; `delay` is under the buffer's length.
fn read(buffer: &[f32], write_index: usize, delay: f32) -> f32 {
    let len = buffer.len();
    let mut position = write_index as f32 - delay;
    if position < 0.0 {
        position += len as f32;
    }
    // A tiny negative wraps to exactly the length in f32.
    let index = if position as usize >= len {
        0
    } else {
        position as usize
    };
    let next = if index + 1 == len { 0 } else { index + 1 };
    buffer[index] + (buffer[next] - buffer[index]) * position.fract()
}

fn shifted(buffer: &[f32], write_index: usize, phase: f32, window: f32) -> f32 {
    let other = (phase + 0.5).fract();
    let weight = 1.0 - (2.0 * phase - 1.0).abs();
    let other_weight = 1.0 - (2.0 * other - 1.0).abs();
    let base = 0.5 * window;
    weight * read(buffer, write_index, base + phase * window)
        + other_weight * read(buffer, write_index, base + other * window)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_target_class_is_the_held_target_before_pitch_and_fine_tune() {
        // Pitch +5 semitones (p2 17.5/24), Fine Tune +30 cents (p3 0.8), every note enabled.
        let mut params = DEFAULT_PARAMS;
        (params[2], params[3]) = (17.5 / 24.0, 0.8);
        let mut state = PitcherState::new(&params, 48_000.0).unwrap();
        state.set_notes([true; 12]);
        // The engine holds A3 (its Stability hysteresis) while it tracks A#3: the disc shows the held A (9), not the
        // A# a fresh snap would pick, and not the D the offsets lead to.
        state.stable_target = state.target_for(220.0);
        (state.tracked_frequency, state.voiced) = (233.08, true);
        assert_eq!(state.target_class(), Some(9));
        // Snapping bypassed for the tracked note (A#, p20), or for all (p22): none.
        state.params[20] = 1.0;
        assert_eq!(state.target_class(), None);
        state.params[20] = 0.0;
        state.params[22] = 1.0;
        assert_eq!(state.target_class(), None);
        state.params[22] = 0.0;
        // No pitch, or nothing held yet: none.
        state.voiced = false;
        assert_eq!(state.target_class(), None);
        state.voiced = true;
        state.stable_target = 0.0;
        assert_eq!(state.target_class(), None);
    }
}
