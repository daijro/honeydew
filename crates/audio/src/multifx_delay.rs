use std::f32::consts::TAU;

use crate::biquad::flush;

/// The MultiFX LFO at phase `phase` advancing by `increment`: Sine uses the phase
/// before the advance, Saw `1 − 2φ′` with φ′ the phase after one increment, Square +1 while φ ≤ 0.5.
pub(crate) fn lfo_wave(shape: usize, phase: f64, increment: f64) -> f32 {
    match shape {
        0 => (TAU * phase as f32).sin(),
        1 => 1.0 - 2.0 * (phase + increment).rem_euclid(1.0) as f32,
        _ => {
            if phase <= 0.5 {
                1.0
            } else {
                -1.0
            }
        }
    }
}

#[derive(Clone, Copy)]
pub struct MultiFxDelaySettings {
    pub x: f32,
    pub y: f32,
    pub rate_beats: f32,
    pub lfo_amount: f32,
    pub lfo_shape: usize,
    pub tempo_bpm: f32,
    pub start_beat: f64,
}

pub struct MultiFxDelay {
    sample_rate: f32,
    ring: [Vec<f32>; 2],
    cursor: usize,
    delay_samples: usize,
    x: f32,
    y: f32,
    /// The wet ramp `w`.
    pub(crate) wet: f32,
    first_stage: [f32; 2],
    second_stage: [f32; 2],
    lfo_amount: f32,
    lfo_shape: usize,
    lfo_phase: f64,
    lfo_increment: f64,
}

impl MultiFxDelay {
    pub fn new(sample_rate: f32) -> Self {
        let sample_rate = sample_rate.max(1.0);
        let capacity = (sample_rate * 1.5).trunc().max(4.0) as usize;
        Self {
            sample_rate,
            ring: std::array::from_fn(|_| vec![0.0; capacity]),
            cursor: 0,
            delay_samples: 1,
            x: 0.5,
            y: 0.5,
            wet: 0.0,
            first_stage: [0.0; 2],
            second_stage: [0.0; 2],
            lfo_amount: 0.0,
            lfo_shape: 0,
            lfo_phase: 0.0,
            lfo_increment: 0.0,
        }
    }

    pub fn capacity(&self) -> usize {
        self.ring[0].len()
    }

    #[cfg(test)]
    #[allow(dead_code)]
    pub fn delay_samples(&self) -> usize {
        self.delay_samples
    }

    #[cfg(test)]
    #[allow(dead_code)]
    pub fn wet_level(&self) -> f32 {
        self.wet
    }

    pub fn filter_coefficients(&self) -> (f32, f32) {
        if self.y <= 0.5 {
            (0.01 + 1.98 * self.y, 0.0)
        } else {
            (1.0, 0.9 * self.y - 0.45)
        }
    }

    pub fn configure(&mut self, settings: MultiFxDelaySettings) {
        self.x = settings.x.clamp(0.0, 1.0);
        self.y = settings.y.clamp(0.0, 1.0);
        self.lfo_amount = settings.lfo_amount.clamp(-1.0, 1.0);
        self.lfo_shape = settings.lfo_shape.min(2);
        if settings.rate_beats > 0.0 && settings.tempo_bpm > 0.0 {
            // The Delay kind runs on R_eff = 2R; the echo is R beats, halved until it is at most 1.5 s.
            let rate = 2.0 * settings.rate_beats;
            let mut normalized_time = rate * 60.0 / settings.tempo_bpm / 3.0;
            while normalized_time > 1.0 {
                normalized_time *= 0.5;
            }
            let delay_samples = ((self.capacity() - 1) as f32 * normalized_time).trunc() as usize;
            let delay_samples = delay_samples.clamp(1, self.capacity() - 1);
            if delay_samples != self.delay_samples {
                self.delay_samples = delay_samples;
                self.reset_audio();
            }
            // One LFO cycle per 2R beats, B/(60·Fs·2R) per sample.
            self.lfo_increment =
                settings.tempo_bpm as f64 / (60.0 * self.sample_rate as f64 * rate as f64);
            if settings.start_beat >= 0.0 {
                self.lfo_phase = (settings.start_beat / rate as f64).rem_euclid(1.0);
            }
        }
    }

    pub fn process_frame(&mut self, input: [f32; 2], target: f32) -> (f32, f32) {
        self.wet = flush(self.wet + 0.005 * (target.clamp(0.0, 1.0) - self.wet));
        let lfo = self.lfo_amount * lfo_wave(self.lfo_shape, self.lfo_phase, self.lfo_increment);
        self.lfo_phase = (self.lfo_phase + self.lfo_increment).rem_euclid(1.0);
        let read_index = (self.cursor + self.capacity() - self.delay_samples) % self.capacity();
        let delayed = [self.ring[0][read_index], self.ring[1][read_index]];
        // r = X² × the ring sample one tap back, a plain gain: X² is both the first echo's level and the
        // gain per repeat.
        let gain = self.x * self.x;
        let (first_coefficient, second_coefficient) = self.filter_coefficients();
        let mut wet = [0.0; 2];
        for (channel, sample) in delayed.iter().enumerate() {
            let previous_first = self.first_stage[channel];
            self.first_stage[channel] =
                flush(previous_first + first_coefficient * (gain * sample - previous_first));
            let second = &mut self.second_stage[channel];
            *second = flush(*second + second_coefficient * (previous_first - *second));
            wet[channel] = previous_first - *second;
            self.ring[channel][self.cursor] = flush(input[channel] * self.wet + wet[channel]);
        }
        self.cursor = (self.cursor + 1) % self.capacity();
        (
            input[0] + (1.0 + lfo) * wet[0],
            input[1] + (1.0 - lfo) * wet[1],
        )
    }

    /// Clears the echoes and the wet ramp. The LFO phase stays: resets never clear it.
    pub fn reset(&mut self) {
        self.reset_audio();
        self.wet = 0.0;
    }

    fn reset_audio(&mut self) {
        for channel in &mut self.ring {
            channel.fill(0.0);
        }
        self.cursor = 0;
        self.first_stage = [0.0; 2];
        self.second_stage = [0.0; 2];
    }
}
