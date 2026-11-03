use std::f32::consts::TAU;

use crate::biquad::flush;

const DIFFUSION_SECONDS: [f32; 4] = [0.0131, 0.0173, 0.0197, 0.0251];
const LATE_LEFT_SECONDS: [f32; 4] = [0.0441, 0.0633, 0.0781, 0.1039];
const LATE_RIGHT_SECONDS: [f32; 4] = [0.0473, 0.0659, 0.0829, 0.1091];

struct Ring {
    samples: Vec<f32>,
    cursor: usize,
}

impl Ring {
    fn new(max_frames: usize) -> Self {
        Self {
            samples: vec![0.0; max_frames.max(4) + 4],
            cursor: 0,
        }
    }

    fn read(&self, frames: f32) -> f32 {
        let length = self.samples.len() as f32;
        let position = (self.cursor as f32 - frames.clamp(1.0, length - 2.0)).rem_euclid(length);
        // A tiny negative wraps to exactly `length` in f32.
        let index = position as usize % self.samples.len();
        self.samples[index]
            + (self.samples[(index + 1) % self.samples.len()] - self.samples[index])
                * position.fract()
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

struct Diffuser {
    delay: Ring,
    frames: f32,
    gain: f32,
}

impl Diffuser {
    fn process(&mut self, input: f32) -> f32 {
        let delayed = self.delay.read(self.frames);
        let output = delayed - self.gain * input;
        self.delay.push(input + self.gain * output);
        output
    }

    fn clear(&mut self) {
        self.delay.clear();
    }
}

struct LateLine {
    delay: Ring,
    frames: f32,
    damping: f32,
}

impl LateLine {
    fn read(&self) -> f32 {
        self.delay.read(self.frames)
    }

    fn clear(&mut self) {
        self.delay.clear();
        self.damping = 0.0;
    }
}

/// The MultiFX Reverb kind: mono in, high-passed near 317 Hz, four diffusers, two
/// cross-coupled damped late branches. No modulated reads, and no early reflections or internal dry path: only the
/// late tail is added to the input.
pub struct MultiFxReverbState {
    sample_rate: f32,
    /// The wet ramp `w`.
    pub(crate) wet: f32,
    decay_seconds: f32,
    /// The late gain and its target, smoothed by 0.001 per sample.
    late_level: f32,
    late_target: f32,
    damping_alpha: f32,
    input_highpass_state: f32,
    input_previous: f32,
    diffusion: [Diffuser; 4],
    left: [LateLine; 4],
    right: [LateLine; 4],
}

impl MultiFxReverbState {
    pub fn new(sample_rate: f32) -> Self {
        let sample_rate = sample_rate.max(1.0);
        let diffusion = std::array::from_fn(|index| {
            let frames = (DIFFUSION_SECONDS[index] * sample_rate).round().max(1.0);
            Diffuser {
                delay: Ring::new(frames as usize + 2),
                frames,
                gain: if index % 2 == 0 { 0.7 } else { -0.7 },
            }
        });
        let build_late = |seconds: [f32; 4]| {
            std::array::from_fn(|index| {
                let frames = (seconds[index] * sample_rate).round().max(1.0);
                LateLine {
                    delay: Ring::new(frames as usize + 4),
                    frames,
                    damping: 0.0,
                }
            })
        };
        let mut state = Self {
            sample_rate,
            wet: 0.0,
            decay_seconds: 1.0,
            late_level: 0.0,
            late_target: 0.0,
            damping_alpha: 1.0,
            input_highpass_state: 0.0,
            input_previous: 0.0,
            diffusion,
            left: build_late(LATE_LEFT_SECONDS),
            right: build_late(LATE_RIGHT_SECONDS),
        };
        state.refresh(0.5, 0.5, sample_rate);
        // The late level starts at its target.
        state.late_level = state.late_target;
        state
    }

    pub fn refresh(&mut self, x: f32, y: f32, sample_rate: f32) {
        if (sample_rate - self.sample_rate).abs() > f32::EPSILON {
            *self = Self::new(sample_rate);
        }
        let x = x.clamp(0.0, 1.0);
        let y = y.clamp(0.0, 1.0);
        let decay_control = (1.0 + 199.0 * (0.75 * x.powi(3)) + 0.5).floor();
        self.decay_seconds = 0.1 * decay_control;
        let late_db = (-361.0 + 421.0 * (0.5 + 0.15 * x.powi(3)) + 0.5).floor();
        self.late_target = 0.3 * 10.0_f32.powf(late_db * 0.005);
        // The damping low-pass is at 100 × the control in Hz, unclamped; Y = 1 (control 221) turns it off.
        let filter_control = (5.0 + 216.0 * y * y + 0.5).floor();
        self.damping_alpha = if filter_control >= 221.0 {
            1.0
        } else {
            1.0 - (-TAU * 100.0 * filter_control / self.sample_rate).exp()
        };
    }

    pub fn process_frame(&mut self, left: f32, right: f32, active: bool) -> (f32, f32) {
        self.wet = flush(self.wet + (f32::from(active) - self.wet) * 0.005);
        self.late_level += 0.001 * (self.late_target - self.late_level);
        let input = 0.5 * (left + right) * self.wet;
        let highpass_decay = (-TAU * 317.0 / self.sample_rate).exp();
        self.input_highpass_state =
            flush(highpass_decay * (self.input_highpass_state + input - self.input_previous));
        self.input_previous = input;
        let mut diffused = self.input_highpass_state;
        for stage in &mut self.diffusion {
            diffused = stage.process(diffused);
        }
        let left_taps = std::array::from_fn::<_, 4, _>(|index| self.left[index].read());
        let right_taps = std::array::from_fn::<_, 4, _>(|index| self.right[index].read());
        let late_left = self.advance_branch(left_taps, right_taps, diffused, true);
        let late_right = self.advance_branch(right_taps, left_taps, diffused, false);
        (
            left + self.late_level * late_left,
            right + self.late_level * late_right,
        )
    }

    fn advance_branch(
        &mut self,
        taps: [f32; 4],
        opposite: [f32; 4],
        input: f32,
        is_left: bool,
    ) -> f32 {
        let transformed = [
            (taps[0] + taps[1] + taps[2] + taps[3]) * 0.5,
            (taps[0] - taps[1] + taps[2] - taps[3]) * 0.5,
            (taps[0] + taps[1] - taps[2] - taps[3]) * 0.5,
            (taps[0] - taps[1] - taps[2] + taps[3]) * 0.5,
        ];
        let lines = if is_left {
            &mut self.left
        } else {
            &mut self.right
        };
        for index in 0..4 {
            let seconds = lines[index].frames / self.sample_rate;
            let feedback = 10.0_f32.powf(-3.0 * seconds / self.decay_seconds);
            let damping = &mut lines[index].damping;
            *damping = flush(
                *damping
                    + self.damping_alpha * (transformed[index] + 0.08 * opposite[index] - *damping),
            );
            lines[index]
                .delay
                .push(input * 0.25 + feedback * lines[index].damping);
        }
        (taps[0] + taps[1] - taps[2] + taps[3]) * 0.5
    }

    pub fn reset(&mut self) {
        self.wet = 0.0;
        self.input_highpass_state = 0.0;
        self.input_previous = 0.0;
        for stage in &mut self.diffusion {
            stage.clear();
        }
        for line in &mut self.left {
            line.clear();
        }
        for line in &mut self.right {
            line.clear();
        }
    }
}
