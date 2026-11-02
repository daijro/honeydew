use crate::VariableDelay;

pub struct CombState {
    lines: [VariableDelay; 2],
    sample_rate: f32,
    mode: u8,
    manual_frequency: f32,
    octave_offset: i32,
    smoothing: f32,
    asymmetry: f32,
    polarity: u8,
    feedback: f32,
    mix: f32,
    active_note: Option<u8>,
    last_note: Option<u8>,
    current_frequency: [f32; 2],
    analysis: [f32; 2048],
    analysis_index: usize,
    tracked_frequency: f32,
    lowpass: f32,
}

impl Default for CombState {
    fn default() -> Self {
        Self {
            lines: std::array::from_fn(|_| VariableDelay::default()),
            sample_rate: 0.0,
            mode: 0,
            manual_frequency: 1.0,
            octave_offset: 0,
            smoothing: 0.2,
            asymmetry: 0.0,
            polarity: 0,
            feedback: 0.8,
            mix: 0.75,
            active_note: None,
            last_note: None,
            current_frequency: [0.0; 2],
            analysis: [0.0; 2048],
            analysis_index: 0,
            tracked_frequency: 0.0,
            lowpass: 0.0,
        }
    }
}

impl CombState {
    pub fn refresh(&mut self, params: &[f32], sample_rate: f32) {
        if self.sample_rate != sample_rate {
            self.sample_rate = sample_rate;
            for line in &mut self.lines {
                line.resize(sample_rate.trunc() as usize);
            }
        }
        self.mode = (2.0 * params[1]).trunc() as u8;
        self.manual_frequency = if params[7] != 0.0 {
            let octave = (7.0 * params[10]).round() as i32;
            let note = (11.0 * params[11]).round() as i32;
            midi_frequency(octave * 12 + note)
        } else {
            1.0 + 2499.0 * params[2].powi(4)
        };
        self.octave_offset = (4.0 * params[5] - 2.0).round() as i32;
        self.smoothing = 0.2 - 0.1999 * params[6].powi(2);
        self.asymmetry = 0.1 * params[9] - 0.05;
        self.polarity = (3.0 * params[8]).trunc() as u8;
        self.feedback = params[3];
        self.mix = params[4];
    }

    pub fn set_note(&mut self, note: Option<u8>) {
        self.active_note = note;
        if note.is_some() {
            self.last_note = note;
        }
    }

    fn frequency(&self) -> f32 {
        let base = match self.mode {
            1 => self
                .active_note
                .or(self.last_note)
                .map(|note| midi_frequency(i32::from(note)))
                .unwrap_or(1.0),
            2 => self.tracked_frequency.max(1.0),
            _ => self.manual_frequency,
        };
        (base * 2.0_f32.powi(self.octave_offset)).clamp(1.0, 20_000.0)
    }

    pub fn process_frame(&mut self, left: f32, right: f32) -> (f32, f32) {
        if self.mode == 2 {
            let mono = 0.5 * (left + right);
            let cutoff = 1.0 - (-std::f32::consts::TAU * 1250.0 / self.sample_rate).exp();
            self.lowpass += cutoff * (mono - self.lowpass);
            self.analysis[self.analysis_index] = mono - self.lowpass;
            self.analysis_index += 1;
            if self.analysis_index == self.analysis.len() {
                self.analysis_index = 0;
                if let Some(frequency) = estimate_pitch(&self.analysis, self.sample_rate)
                    && (self.tracked_frequency == 0.0
                        || (frequency / self.tracked_frequency)
                            .max(self.tracked_frequency / frequency)
                            < 3.0)
                {
                    self.tracked_frequency = frequency;
                }
            }
        }
        let frequency = self.frequency();
        let targets = [
            frequency * (1.0 - self.asymmetry),
            frequency * (1.0 + self.asymmetry),
        ];
        let mut output = [0.0; 2];
        for channel in 0..2 {
            self.current_frequency[channel] +=
                (targets[channel] - self.current_frequency[channel]) * self.smoothing;
            let current = self.current_frequency[channel].clamp(1.0, 20_000.0);
            let delay = 2.0 + (self.sample_rate - 2.0) / current;
            self.lines[channel].set_target(delay);
            let polarity = if self.polarity & (1 << channel) != 0 {
                -1.0
            } else {
                1.0
            };
            let input = if channel == 0 { left } else { right };
            let wet = self.lines[channel].process(input, self.feedback * polarity);
            output[channel] = input + self.mix * wet;
        }
        (output[0], output[1])
    }

    pub fn reset(&mut self) {
        self.tracked_frequency = 0.0;
        let frequency = self.frequency();
        self.current_frequency = [
            frequency * (1.0 - self.asymmetry),
            frequency * (1.0 + self.asymmetry),
        ];
        for channel in 0..2 {
            let target = 2.0 + (self.sample_rate - 2.0) / self.current_frequency[channel];
            self.lines[channel].set_target(target);
            self.lines[channel].reset();
        }
        self.analysis.fill(0.0);
        self.analysis_index = 0;
        self.lowpass = 0.0;
    }
}

fn midi_frequency(note: i32) -> f32 {
    if note < 1 {
        0.0
    } else {
        440.0 * 2.0_f32.powf(note as f32 / 12.0 - 5.75)
    }
}

fn estimate_pitch(input: &[f32; 2048], sample_rate: f32) -> Option<f32> {
    let energy = input.iter().map(|sample| sample * sample).sum::<f32>() / input.len() as f32;
    if energy < 1e-5 {
        return None;
    }
    let mut cumulative = 0.0;
    let mut best = None;
    for lag in 2..1024 {
        let mut difference = 0.0;
        for i in 0..(input.len() - lag) {
            let delta = input[i] - input[i + lag];
            difference += delta * delta;
        }
        cumulative += difference;
        let normalized = difference * lag as f32 / cumulative.max(1e-20);
        if lag >= 24 && normalized < 0.12 {
            if best.is_none_or(|(_, previous)| normalized < previous) {
                best = Some((lag, normalized));
            } else {
                break;
            }
        }
    }
    best.map(|(lag, _)| sample_rate / lag as f32)
}

#[cfg(test)]
mod tests {
    use super::estimate_pitch;

    #[test]
    fn audio_pitch_tracker_finds_a_steady_a4() {
        let mut input = [0.0; 2048];
        for (i, sample) in input.iter_mut().enumerate() {
            *sample = (std::f32::consts::TAU * 440.0 * i as f32 / 48_000.0).sin();
        }
        let frequency = estimate_pitch(&input, 48_000.0).unwrap();
        assert!((frequency - 440.0).abs() < 8.0, "{frequency}");
    }
}
