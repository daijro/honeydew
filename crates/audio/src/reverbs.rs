use std::f32::consts::TAU;

use crate::biquad::flush;

struct Delay {
    samples: Vec<f32>,
    cursor: usize,
}

impl Delay {
    fn new(max_seconds: f32, sample_rate: f32) -> Self {
        Self {
            samples: vec![0.0; (max_seconds * sample_rate).ceil() as usize + 4],
            cursor: 0,
        }
    }

    fn read(&self, frames: f32) -> f32 {
        let len = self.samples.len() as f32;
        let position = (self.cursor as f32 - frames.clamp(1.0, len - 2.0)).rem_euclid(len);
        // A tiny negative wraps to exactly `len` in f32.
        let i = position as usize % self.samples.len();
        let next = (i + 1) % self.samples.len();
        self.samples[i] + (self.samples[next] - self.samples[i]) * position.fract()
    }

    fn push(&mut self, sample: f32) {
        self.samples[self.cursor] = flush(sample);
        self.cursor = (self.cursor + 1) % self.samples.len();
    }

    fn clear(&mut self) {
        self.samples.fill(0.0);
        self.cursor = 0;
    }
}

struct AllPass {
    delay: Delay,
}

impl AllPass {
    fn process(&mut self, input: f32, frames: f32, gain: f32) -> f32 {
        let delayed = self.delay.read(frames);
        let output = delayed - gain * input;
        self.delay.push(input + gain * output);
        output
    }
}

struct InputFilter {
    low_state: f32,
    high_state: f32,
}

impl InputFilter {
    fn process(&mut self, input: f32, low_cut: f32, high_cut: f32) -> f32 {
        self.low_state = flush(self.low_state + low_cut * (input - self.low_state));
        let high_passed = input - self.low_state;
        self.high_state = flush(self.high_state + high_cut * (high_passed - self.high_state));
        self.high_state
    }
}

pub struct ReverbState {
    id: u16,
    sample_rate: f32,
    params: Vec<f32>,
    predelay: [Delay; 2],
    diffuse: [AllPass; 4],
    early: [Delay; 2],
    late: [Delay; 4],
    filter: [InputFilter; 2],
    damp: [f32; 4],
    bass: [f32; 4],
    phase: f32,
    predelay_frames: f32,
    low_cut: f32,
    high_cut: f32,
    damping: f32,
    crossover: f32,
    decay_seconds: f32,
    room_scale: f32,
    diffusion: f32,
    modulation_rate: f32,
    modulation_depth: f32,
    wet_level: f32,
    early_level: f32,
    dry_level: f32,
    pan: f32,
}

impl ReverbState {
    pub fn new(id: u16, params: &[f32], sample_rate: f32) -> Self {
        assert!(matches!(id, 12 | 32 | 35));
        assert!(sample_rate.is_finite() && sample_rate > 0.0);
        let mut state = Self {
            id,
            sample_rate,
            params: params.to_vec(),
            predelay: std::array::from_fn(|_| Delay::new(1.1, sample_rate)),
            diffuse: std::array::from_fn(|_| AllPass {
                delay: Delay::new(0.02, sample_rate),
            }),
            early: std::array::from_fn(|_| Delay::new(0.1, sample_rate)),
            late: std::array::from_fn(|_| Delay::new(0.2, sample_rate)),
            filter: std::array::from_fn(|_| InputFilter {
                low_state: 0.0,
                high_state: 0.0,
            }),
            damp: [0.0; 4],
            bass: [0.0; 4],
            phase: 0.0,
            predelay_frames: 0.0,
            low_cut: 0.0,
            high_cut: 1.0,
            damping: 1.0,
            crossover: 0.0,
            decay_seconds: 1.0,
            room_scale: 1.0,
            diffusion: 0.5,
            modulation_rate: 0.0,
            modulation_depth: 0.0,
            wet_level: 1.0,
            early_level: 0.25,
            dry_level: 1.0,
            pan: 0.5,
        };
        state.refresh(params, sample_rate);
        state
    }

    pub fn refresh(&mut self, params: &[f32], sample_rate: f32) {
        assert!(sample_rate.is_finite() && sample_rate > 0.0);
        if sample_rate != self.sample_rate {
            *self = Self::new(self.id, params, sample_rate);
            return;
        }
        self.params.clear();
        self.params.extend_from_slice(params);
        let p = &self.params;
        let (low, high, damp, room, decay, diffusion, rate, depth, pan) = if self.id == 35 {
            (
                // LOW CUT ≈19 + 2981·v Hz, off at v = 0.
                if p[2] <= 0.0 {
                    0.0
                } else {
                    19.0 + 2_981.0 * p[2]
                },
                if p[1] >= 1.0 {
                    20_000.0
                } else {
                    510.0 + 21_580.0 * p[1]
                },
                if p[6] >= 1.0 {
                    20_000.0
                } else {
                    510.0 + 21_580.0 * p[6]
                },
                p[3],
                0.1 + 19.9 * p[4],
                p[5],
                p[16],
                p[13],
                p[18],
            )
        } else {
            let (room, diffusion, low, high, rate, depth, pan) = if self.id == 12 {
                (p[5], p[6], p[8], p[9], p[19], p[20], 0.5)
            } else {
                (p[7], p[8], p[10], p[11], p[12], p[13], p[4])
            };
            (
                low * 3_000.0,
                500.0 + high * 19_500.0,
                500.0 + (1.0 - p[2]) * 19_500.0,
                room,
                0.15 + 9.85 * p[1] * p[1],
                diffusion,
                rate,
                depth,
                pan,
            )
        };
        self.low_cut = one_pole(low, sample_rate);
        self.high_cut = one_pole(high, sample_rate);
        self.damping = one_pole(damp, sample_rate);
        self.crossover = one_pole(
            if self.id == 35 {
                25.0 + 1_975.0 * p[17]
            } else if self.id == 12 {
                25.0 + 1_975.0 * p[18]
            } else {
                500.0
            },
            sample_rate,
        );
        self.room_scale = 0.4 + 1.4 * room;
        self.decay_seconds = decay;
        self.diffusion = 0.15 + 0.55 * diffusion;
        self.modulation_rate = 0.05 + 2.0 * rate;
        self.modulation_depth = 12.0 * depth;
        self.pan = pan;
        if self.id == 35 {
            self.dry_level = 1.25 * p[7];
            self.early_level = 1.25 * p[8];
            self.wet_level = 1.25 * p[9];
        } else if self.id == 12 {
            self.dry_level = 1.0 - p[3] + p[12];
            self.early_level = p[3] * p[11];
            self.wet_level = p[3];
        } else {
            self.dry_level = (1.0 - p[3]) * p[15];
            self.early_level = p[3] * p[5] * p[14];
            self.wet_level = p[3] * p[14];
        }
        self.prepare_block(120.0);
    }

    pub fn prepare_block(&mut self, tempo_bpm: f32) {
        let delay = if self.id == 35 {
            if self.params[12] > 0.5 {
                self.params[11] * 60.0 / tempo_bpm.max(1.0)
            } else {
                self.params[11]
            }
        } else if self.id == 12 {
            self.params[4] * 0.25
        } else {
            self.params[6] * 0.25
        };
        self.predelay_frames =
            (delay * self.sample_rate).min(self.predelay[0].samples.len() as f32 - 2.0);
    }

    pub fn process_frame(&mut self, input_l: f32, input_r: f32) -> (f32, f32) {
        let p = &self.params;
        let input = if self.id == 35 && p[10] > 0.5 {
            [input_l, -input_r]
        } else {
            [input_l, input_r]
        };
        let mut feed = [0.0; 2];
        for ch in 0..2 {
            let delayed = if self.predelay_frames < 1.0 {
                input[ch]
            } else {
                self.predelay[ch].read(self.predelay_frames)
            };
            self.predelay[ch].push(input[ch]);
            feed[ch] = self.filter[ch].process(delayed, self.low_cut, self.high_cut);
        }
        if self.id == 12 {
            feed = [feed[0] * p[10], feed[1] * p[10]];
        }
        let ap_lengths = [0.0047, 0.0079, 0.0059, 0.0113];
        for (ch, feed_channel) in feed.iter_mut().enumerate() {
            for stage in 0..2 {
                let i = 2 * ch + stage;
                *feed_channel = self.diffuse[i].process(
                    *feed_channel,
                    ap_lengths[i] * self.sample_rate,
                    self.diffusion,
                );
            }
        }
        let early_l = 0.7 * self.early[0].read(0.0083 * self.sample_rate)
            + 0.3 * self.early[1].read(0.0221 * self.sample_rate);
        let early_r = 0.7 * self.early[1].read(0.0117 * self.sample_rate)
            + 0.3 * self.early[0].read(0.0193 * self.sample_rate);
        self.early[0].push(feed[0]);
        self.early[1].push(feed[1]);
        let lengths = [0.0297, 0.0371, 0.0411, 0.0437];
        let mut tail = [0.0; 4];
        let mut feedback = [0.0; 4];
        let bass_mult = if self.id == 35 {
            0.2 + 2.8 * p[14]
        } else if self.id == 12 {
            1.0 + 2.0 * p[17]
        } else {
            1.0
        };
        for i in 0..4 {
            let phase = self.phase + i as f32 * 0.25;
            let movement = self.modulation_depth * (TAU * phase).sin();
            let seconds = lengths[i] * self.room_scale;
            tail[i] = self.late[i].read(seconds * self.sample_rate + movement);
            self.damp[i] = flush(self.damp[i] + self.damping * (tail[i] - self.damp[i]));
            self.bass[i] = flush(self.bass[i] + self.crossover * (self.damp[i] - self.bass[i]));
            let gain = 10.0_f32.powf(-3.0 * seconds / self.decay_seconds);
            feedback[i] =
                gain * (self.damp[i] - self.bass[i]) + gain.powf(1.0 / bass_mult) * self.bass[i];
        }
        let sum = feedback.iter().sum::<f32>() * 0.5;
        for (i, feedback_channel) in feedback.iter().enumerate() {
            let injection = if i % 2 == 0 { feed[0] } else { feed[1] };
            self.late[i].push(0.25 * injection + feedback_channel - sum);
        }
        self.phase = (self.phase + self.modulation_rate / self.sample_rate).fract();
        let mut wet_l =
            self.early_level * early_l + self.wet_level * (tail[0] - tail[1] + tail[2] - tail[3]);
        let mut wet_r =
            self.early_level * early_r + self.wet_level * (tail[0] + tail[1] - tail[2] - tail[3]);
        if self.id == 35 {
            let separation = p[15];
            if separation <= 0.5 {
                let mid = 0.5 * (wet_l + wet_r);
                let side = (1.0 - separation) * (wet_l - wet_r);
                (wet_l, wet_r) = (mid + side, mid - side);
            } else {
                let side = 0.5 - separation;
                wet_l -= side * wet_r;
                wet_r = wet_l * side - wet_r;
            }
        } else {
            let width = if self.id == 12 { p[7] } else { p[9] };
            let mid = 0.5 * (wet_l + wet_r);
            let side = width * 0.5 * (wet_l - wet_r);
            (wet_l, wet_r) = (mid + side, mid - side);
        }
        let left_weight = (2.0 - 2.0 * self.pan).min(1.0);
        let right_weight = (2.0 * self.pan).min(1.0);
        (
            self.dry_level * input_l + wet_l * left_weight,
            self.dry_level * input_r + wet_r * right_weight,
        )
    }

    pub fn reset(&mut self) {
        for delay in &mut self.predelay {
            delay.clear();
        }
        for stage in &mut self.diffuse {
            stage.delay.clear();
        }
        for delay in &mut self.early {
            delay.clear();
        }
        for delay in &mut self.late {
            delay.clear();
        }
        self.filter = std::array::from_fn(|_| InputFilter {
            low_state: 0.0,
            high_state: 0.0,
        });
        self.damp = [0.0; 4];
        self.bass = [0.0; 4];
        self.phase = 0.0;
    }
}

fn one_pole(frequency: f32, sample_rate: f32) -> f32 {
    if frequency <= 0.0 {
        0.0
    } else {
        1.0 - (-TAU * frequency.min(0.45 * sample_rate) / sample_rate).exp()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Reverb 2's LOW CUT is ≈19 + 2981·v Hz (the panel's text), off at
    /// v = 0.
    #[test]
    fn reverb_2_low_cut_is_19_plus_2981_v_and_off_at_0() {
        let rate = 48_000.0;
        // Reverb 2's defaults (`AudioEffect::new`).
        let mut params = vec![
            1.0, 0.16, 0.0187, 0.5, 0.068, 1.0, 0.16, 0.8, 0.8, 0.56, 0.0, 0.0, 0.0, 0.0, 0.28,
            0.5, 0.33, 0.2405, 0.5,
        ];
        for (v, hz) in [(0.0, 0.0), (0.5, 1_509.5), (1.0, 3_000.0)] {
            params[2] = v;
            let state = ReverbState::new(35, &params, rate);
            assert_eq!(state.low_cut, one_pole(hz, rate), "v {v}");
        }
    }
}
