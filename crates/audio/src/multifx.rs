use std::f32::consts::TAU;

use crate::biquad::flush;
use crate::multifx_delay::{MultiFxDelay, MultiFxDelaySettings, lfo_wave};
use crate::multifx_reverb::MultiFxReverbState;

const RATE_BEATS: [f32; 10] = [0.125, 0.25, 0.375, 0.5, 0.75, 1.0, 2.0, 4.0, 8.0, 16.0];
const VOWEL_A: [f32; 8] = [
    250.0, 700.0, 2500.0, 3500.0, 4200.0, 8500.0, 10000.0, 12000.0,
];
const VOWEL_B: [f32; 8] = [
    700.0, 1250.0, 2500.0, 3500.0, 4200.0, 8500.0, 10000.0, 12000.0,
];
const VOWEL_C: [f32; 8] = [
    250.0, 2500.0, 2700.0, 3500.0, 4200.0, 8500.0, 10000.0, 12000.0,
];

/// The bank's kinds in visiting order. MultiFX shows eight of them;
/// DIST and RING run only in GMSynth's FX slots.
pub(crate) const DIST: usize = 0;
pub(crate) const FLANGER: usize = 1;
pub(crate) const PHASER: usize = 2;
pub(crate) const LPF: usize = 3;
pub(crate) const HPF: usize = 4;
pub(crate) const DELAY: usize = 5;
pub(crate) const REVERB: usize = 6;
pub(crate) const PANNER: usize = 7;
pub(crate) const RING: usize = 8;
pub(crate) const VOX: usize = 9;
const KINDS: usize = 10;
/// MultiFX's visible kind trunc(7v): Delay, Reverb, LP, HP, Flanger, Phaser, Panner, Vox.
const VISIBLE: [usize; 8] = [DELAY, REVERB, LPF, HPF, FLANGER, PHASER, PANNER, VOX];

/// A bank's controls: X, Y, the LFO rate R (beats), its amount and its shape (0 Sine, 1 Saw, 2 Square).
#[derive(Clone, Copy, Debug, PartialEq)]
pub(crate) struct Settings {
    pub(crate) x: f32,
    pub(crate) y: f32,
    pub(crate) rate_beats: f32,
    pub(crate) amount: f32,
    pub(crate) shape: usize,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum MultiFxError {
    UnsupportedKind(usize),
}

#[derive(Clone, Copy, Default)]
struct Biquad {
    x1: f32,
    x2: f32,
    y1: f32,
    y2: f32,
}

impl Biquad {
    fn process(&mut self, input: f32, c: &Coefficients) -> f32 {
        let output =
            flush(c.b0 * input + c.b1 * self.x1 + c.b0 * self.x2 - c.a1 * self.y1 - c.a2 * self.y2);
        self.x2 = self.x1;
        self.x1 = input;
        self.y2 = self.y1;
        self.y1 = output;
        output
    }
}

#[derive(Clone, Copy, Default)]
struct Coefficients {
    b0: f32,
    b1: f32,
    a1: f32,
    a2: f32,
}

#[derive(Default)]
struct Filter {
    stages: [[Biquad; 4]; 2],
    preceding: [[f32; 3]; 2],
    coefficients: Coefficients,
    wet: f32,
}

impl Filter {
    fn set(&mut self, frequency: f32, resonance: f32, sample_rate: f32, high_pass: bool) {
        // Clamped below π as the Equalizer's is: LP's 19.9 kHz top past Nyquist (a 16 or 22.05 kHz device rate) turned
        // the poles unstable. No cutoff at 44.1 kHz and above reaches 3.
        let angle = (TAU * frequency / sample_rate).min(3.0);
        let (sine, cosine) = angle.sin_cos();
        let b0 = (1.0 + if high_pass { cosine } else { -cosine }) / (2.0 + sine / resonance);
        self.coefficients = Coefficients {
            b0,
            b1: if high_pass { -2.0 * b0 } else { 2.0 * b0 },
            a1: -2.0 * resonance * cosine / (sine + resonance),
            a2: (resonance - sine) / (sine + resonance),
        };
    }

    fn process(&mut self, input: [f32; 2], target: f32) -> [f32; 2] {
        self.wet += (target - self.wet) * 0.005;
        let mut output = input;
        for channel in 0..2 {
            let old = self.preceding[channel];
            let mut next = [0.0; 4];
            for stage in 0..4 {
                let sample = if stage == 0 {
                    input[channel]
                } else {
                    old[stage - 1]
                };
                next[stage] = self.stages[channel][stage].process(sample, &self.coefficients);
            }
            self.preceding[channel].copy_from_slice(&next[..3]);
            output[channel] += self.wet * (next[3] - input[channel]);
        }
        output
    }
}

/// A mono feed −0.5·w·(L + R) plus the feedback −0.96Y·y enters a line of N = trunc(0.005·Fs) frames, read
/// with linear interpolation at the delay 1/speed. The read speed follows its target 1/D by 0.001 per sample, clamped
/// to [1/N, 0.9], so a delay change bends the pitch like tape. y is added to both channels; the dry stays at unity.
struct Flanger {
    samples: Vec<f32>,
    cursor: usize,
    /// N, the longest delay (frames).
    span: f32,
    /// The read speed 1/D and its target; 0 until a block sets it.
    speed: f32,
    target_speed: f32,
    feedback: f32,
    wet: f32,
}

impl Flanger {
    fn new(sample_rate: f32) -> Self {
        let span = (sample_rate * 0.005).trunc().max(4.0);
        Self {
            samples: vec![0.0; span as usize + 3],
            cursor: 0,
            span,
            speed: 0.0,
            target_speed: 0.0,
            feedback: 0.0,
            wet: 0.0,
        }
    }

    /// Clears the line in place (no allocation on the audio thread).
    fn reset(&mut self) {
        self.samples.fill(0.0);
        (self.cursor, self.speed, self.wet) = (0, 0.0, 0.0);
    }

    fn set(&mut self, x: f32, y: f32, lfo: f32) {
        let d = ((1.0 - x) - 0.5 * lfo).powi(4).min(1.0);
        self.target_speed = 1.0 / (2.0 + (self.span - 2.0) * d);
        self.feedback = -0.96 * y;
        // The read speed starts at its first target.
        if self.speed == 0.0 {
            self.speed = self.target_speed;
        }
    }

    fn process(&mut self, input: [f32; 2], target: f32) -> [f32; 2] {
        self.wet += (target - self.wet) * 0.005;
        self.speed =
            (self.speed + 0.001 * (self.target_speed - self.speed)).clamp(1.0 / self.span, 0.9);
        let length = self.samples.len();
        let position = (self.cursor as f32 - 1.0 / self.speed).rem_euclid(length as f32);
        // A tiny negative wraps to exactly `length` in f32.
        let index = position as usize % length;
        let y = self.samples[index]
            + (self.samples[(index + 1) % length] - self.samples[index]) * position.fract();
        self.samples[self.cursor] =
            flush(-0.5 * self.wet * (input[0] + input[1]) + self.feedback * y);
        self.cursor = (self.cursor + 1) % length;
        [input[0] + y, input[1] + y]
    }
}

struct Panner {
    samples: [Vec<f32>; 2],
    cursor: usize,
    wet: f32,
    pan: f32,
    distance: f32,
    target_pan: f32,
    target_distance: f32,
}

impl Panner {
    fn new(sample_rate: f32) -> Self {
        let capacity = (sample_rate * 0.002).trunc().max(4.0) as usize;
        Self {
            samples: [vec![0.0; capacity], vec![0.0; capacity]],
            cursor: 0,
            wet: 0.0,
            pan: -1.0,
            distance: -1.0,
            target_pan: 0.5,
            target_distance: 0.0,
        }
    }

    /// As `new`, in place (no allocation on the audio thread).
    fn reset(&mut self) {
        self.samples.iter_mut().for_each(|ring| ring.fill(0.0));
        (self.cursor, self.wet, self.pan, self.distance) = (0, 0.0, -1.0, -1.0);
        (self.target_pan, self.target_distance) = (0.5, 0.0);
    }

    fn set(&mut self, x: f32, y: f32, lfo: f32) {
        self.target_pan = (x + lfo).clamp(0.0, 1.0);
        self.target_distance = y.sqrt();
        if self.pan < 0.0 {
            self.pan = self.target_pan;
        }
        if self.distance < 0.0 {
            self.distance = self.target_distance;
        }
    }

    fn process(&mut self, input: [f32; 2], target: f32) -> [f32; 2] {
        self.wet += (target - self.wet) * 0.005;
        self.pan += (self.target_pan - self.pan) * 0.005;
        self.distance += (self.target_distance - self.distance) * 0.005;
        let far = [
            (2.0 * self.pan - 1.0).max(0.0),
            (1.0 - 2.0 * self.pan).max(0.0),
        ];
        let length = self.samples[0].len();
        let mut output = input;
        for channel in 0..2 {
            self.samples[channel][self.cursor] = input[channel];
            let delay = 1.0 + far[channel] * (length - 4) as f32;
            let position = (self.cursor as f32 - delay).rem_euclid(length as f32);
            // A tiny negative wraps to exactly `length` in f32.
            let index = position as usize % length;
            let delayed = self.samples[channel][index]
                + (self.samples[channel][(index + 1) % length] - self.samples[channel][index])
                    * position.fract();
            let gain = 1.0 - self.distance * far[channel];
            output[channel] = input[channel] * (1.0 - self.wet) + delayed * gain * self.wet;
        }
        self.cursor = (self.cursor + 1) % length;
        output
    }
}

#[derive(Default)]
struct Phaser {
    stages: [[f32; 6]; 2],
    feedback: [f32; 2],
    wet: f32,
    output_gain: f32,
    coefficient: f32,
    feedback_gain: f32,
}

impl Phaser {
    fn set(&mut self, x: f32, y: f32, lfo: f32, sample_rate: f32) {
        let normalized =
            (((x * x + x * x * lfo) * 19_950.0 + 50.0) / sample_rate).clamp(0.001, 0.499);
        self.coefficient = (1.0 - normalized) / (1.0 + normalized);
        self.feedback_gain = 0.98 * (0.6 * y).cbrt();
    }

    fn process(&mut self, input: [f32; 2], target: f32) -> [f32; 2] {
        self.wet += (target - self.wet) * 0.005;
        self.output_gain += (1.0 - self.output_gain) * 0.005;
        let mut output = input;
        for channel in 0..2 {
            let mut sample =
                input[channel] * self.wet + self.feedback[channel] * self.feedback_gain;
            for stage in &mut self.stages[channel] {
                let next = *stage - self.coefficient * sample;
                *stage = flush(sample + self.coefficient * next);
                sample = next;
            }
            self.feedback[channel] = flush(sample);
            output[channel] += sample * self.output_gain;
        }
        output
    }
}

#[derive(Clone, Copy, Default)]
struct BandPass {
    input1: f32,
    input2: f32,
    output1: f32,
    output2: f32,
    b0: f32,
    a1: f32,
    a2: f32,
}

impl BandPass {
    fn set(&mut self, frequency: f32, sample_rate: f32) {
        // Clamped below π: a formant past Nyquist (12 kHz × 1.8 at a 16 or 22.05 kHz device rate) turned the poles
        // unstable. 3.1, not the Equalizer's 3, which the top formant passes at 44.1 kHz (3.08 at Y = 1),
        // where the output stays as it was.
        let angle = (TAU * frequency / sample_rate).min(3.1);
        let (sine, cosine) = angle.sin_cos();
        let alpha = sine / 24.0;
        let denominator = 1.0 + alpha;
        self.b0 = 0.5 * sine / denominator;
        self.a1 = 2.0 * cosine / denominator;
        self.a2 = (1.0 - alpha) / denominator;
    }

    fn process(&mut self, input: f32) -> f32 {
        let output = flush(
            self.b0 * (input - self.input2) + self.a1 * self.output1 - self.a2 * self.output2,
        );
        self.input2 = self.input1;
        self.input1 = input;
        self.output2 = self.output1;
        self.output1 = output;
        output
    }
}

#[derive(Default)]
struct Vowel {
    formants: [[BandPass; 8]; 2],
    wet: f32,
    output_gain: f32,
    segment_gain: f32,
}

impl Vowel {
    fn set(&mut self, x: f32, y: f32, lfo: f32, sample_rate: f32) {
        let position = (x + lfo).clamp(0.0, 1.0) * 3.0;
        let segment = if position >= 3.0 {
            0
        } else {
            position as usize
        };
        let fraction = position.fract();
        let scale = 2.0_f32.powf(1.7 * y - 0.85);
        self.segment_gain = 1.0 - segment as f32 * 0.125;
        for band in 0..8 {
            let frequency = match segment {
                0 => VOWEL_A[band] * (1.0 - fraction) + VOWEL_B[band] * fraction,
                1 => VOWEL_B[band] * (1.0 - fraction) + VOWEL_C[band] * fraction,
                _ => VOWEL_C[band] * (1.0 - fraction) + VOWEL_A[band] * fraction,
            } * scale;
            for channel in 0..2 {
                self.formants[channel][band].set(frequency, sample_rate);
            }
        }
    }

    fn process(&mut self, input: [f32; 2], target: f32) -> [f32; 2] {
        self.wet += (target - self.wet) * 0.005;
        self.output_gain += (1.0 - self.output_gain) * 0.005;
        let mix = self.wet * self.output_gain;
        let mut output = input;
        for channel in 0..2 {
            let formants = self.formants[channel]
                .iter_mut()
                .map(|formant| formant.process(input[channel]))
                .sum::<f32>();
            output[channel] = input[channel] * (1.0 - mix) + formants * self.segment_gain * mix;
        }
        output
    }
}

/// DIST (Crusher, GMSynth only): every H = min(Y⁴, 1)·256 + 1 frames it latches y = x·g·(k + 1)/(1 + k|x|)
/// per channel and holds it; g = 0.1 − 0.09Y, k = 10^(4(X + LFO) + 1) unclamped. No ramp; untouched while off.
#[derive(Default)]
struct Crusher {
    held: [f32; 2],
    /// Frames since the last latch, and H.
    count: usize,
    hold: usize,
    gain: f32,
    knee: f32,
}

impl Crusher {
    fn set(&mut self, x: f32, y: f32, lfo: f32) {
        // H is whole frames, trunc(min(Y⁴, 1)·256) + 1, counted from the first frame; a shorter H
        // applies at once.
        self.hold = (y.powi(4).min(1.0) * 256.0) as usize + 1;
        self.gain = 0.1 - 0.09 * y;
        self.knee = 10.0_f32.powf(4.0 * (x + lfo) + 1.0);
    }

    fn process(&mut self, input: [f32; 2]) -> [f32; 2] {
        if self.count == 0 {
            let (g, k) = (self.gain, self.knee);
            self.held = input.map(|x| x * g * (k + 1.0) / (1.0 + k * x.abs()));
        }
        self.count += 1;
        if self.count >= self.hold {
            self.count = 0;
        }
        self.held
    }
}

/// RING (GMSynth only): a sine carrier at 2^(5 + 8.7·clamp(X + 0.5·LFO, 0, 1)) Hz; with m = w·g,
/// out = in·(1 − m) + m·in·sin 2πφ. Y is unused.
#[derive(Default)]
struct Ring {
    /// The carrier phase φ in cycles, and its step per frame.
    phase: f64,
    step: f64,
    wet: f32,
    output_gain: f32,
}

impl Ring {
    fn set(&mut self, x: f32, lfo: f32, sample_rate: f32) {
        self.step = f64::from((5.0 + 8.7 * (x + 0.5 * lfo).clamp(0.0, 1.0)).exp2() / sample_rate);
    }

    fn process(&mut self, input: [f32; 2], target: f32) -> [f32; 2] {
        self.wet += (target - self.wet) * 0.005;
        self.output_gain += (1.0 - self.output_gain) * 0.005;
        let mix = self.wet * self.output_gain;
        // The carrier reads φ before the advance; φ clears to 0 with the rest of the state.
        let carrier = (TAU * self.phase as f32).sin();
        self.phase = (self.phase + self.step).fract();
        input.map(|x| x * (1.0 - mix) + mix * x * carrier)
    }
}

/// Stop and tails: a wet level, an input or an output at or below this is silent.
const SILENT: f32 = 1e-4;

/// A processor of the bank, as its slot drives it.
trait Processor {
    fn run(&mut self, input: [f32; 2], target: f32) -> [f32; 2];
    /// The wet level `w`.
    fn wet(&mut self) -> &mut f32;
    /// Clears the audio state; a fade-in `g` drops to 0.
    fn clear(&mut self);
}

impl Processor for MultiFxDelay {
    fn run(&mut self, input: [f32; 2], target: f32) -> [f32; 2] {
        self.process_frame(input, target).into()
    }
    fn wet(&mut self) -> &mut f32 {
        &mut self.wet
    }
    fn clear(&mut self) {
        self.reset();
    }
}

impl Processor for MultiFxReverbState {
    fn run(&mut self, input: [f32; 2], target: f32) -> [f32; 2] {
        self.process_frame(input[0], input[1], target > 0.5).into()
    }
    fn wet(&mut self) -> &mut f32 {
        &mut self.wet
    }
    fn clear(&mut self) {
        self.reset();
    }
}

impl Processor for Flanger {
    fn run(&mut self, input: [f32; 2], target: f32) -> [f32; 2] {
        self.process(input, target)
    }
    fn wet(&mut self) -> &mut f32 {
        &mut self.wet
    }
    fn clear(&mut self) {
        self.reset();
    }
}

impl Processor for Panner {
    fn run(&mut self, input: [f32; 2], target: f32) -> [f32; 2] {
        self.process(input, target)
    }
    fn wet(&mut self) -> &mut f32 {
        &mut self.wet
    }
    fn clear(&mut self) {
        self.reset();
    }
}

/// Filter, Phaser, Ring and Vowel clear to their defaults; the next block sets their coefficients again.
macro_rules! cleared_by_default {
    ($($kind:ty),*) => {$(
        impl Processor for $kind {
            fn run(&mut self, input: [f32; 2], target: f32) -> [f32; 2] {
                self.process(input, target)
            }
            fn wet(&mut self) -> &mut f32 {
                &mut self.wet
            }
            fn clear(&mut self) {
                *self = Self::default();
            }
        }
    )*};
}
cleared_by_default!(Filter, Phaser, Ring, Vowel);

/// A processor's place in the bank, for stop and tails.
#[derive(Clone, Copy, Default)]
struct Slot {
    running: bool,
    /// Frames since the processor's input was last active, and since its output last differed from its input.
    idle: usize,
    quiet: usize,
}

impl Slot {
    /// One frame of `processor`. It runs while selected; once not, it keeps running until its wet level falls below
    /// [`SILENT`] and its output has been silent for `tail` frames, then clears and stops. After `skip` frames without
    /// input activity it is skipped, its wet level snapped to its target.
    // Activity is any input sample above SILENT, counted per frame rather than per block.
    fn visit(
        &mut self,
        processor: &mut impl Processor,
        selected: bool,
        input: [f32; 2],
        skip: usize,
        tail: usize,
    ) -> [f32; 2] {
        if !selected && !self.running {
            return input;
        }
        self.running = true;
        let target = f32::from(selected);
        let active = input.iter().any(|x| x.abs() > SILENT);
        self.idle = if active {
            0
        } else {
            self.idle.saturating_add(1)
        };
        let output = if self.idle > skip {
            *processor.wet() = target;
            input
        } else {
            processor.run(input, target)
        };
        let loud = output
            .iter()
            .zip(input)
            .any(|(o, i)| (o - i).abs() > SILENT);
        self.quiet = if loud {
            0
        } else {
            self.quiet.saturating_add(1)
        };
        if !selected && *processor.wet() < SILENT && self.quiet >= tail {
            processor.clear();
            *self = Self::default();
        }
        output
    }
}

/// One processor of every kind, each with its slot, visited in kind order.
/// MultiFX runs one bank; each GMSynth FX slot runs its own.
pub(crate) struct Bank {
    sample_rate: f32,
    tempo_bpm: f32,
    lfo_phase: f64,
    crusher: Crusher,
    flanger: Flanger,
    phaser: Phaser,
    low_pass: Filter,
    high_pass: Filter,
    delay: MultiFxDelay,
    reverb: MultiFxReverbState,
    panner: Panner,
    ring: Ring,
    vowel: Vowel,
    /// By kind; DIST's is unused (it has no ramp or tail).
    slots: [Slot; KINDS],
}

impl Bank {
    /// A cleared bank at `sample_rate`; builds the rate-sized lines, so not on the audio thread.
    pub(crate) fn new(sample_rate: f32) -> Self {
        let sample_rate = sample_rate.max(1.0);
        Self {
            sample_rate,
            tempo_bpm: 120.0,
            lfo_phase: 0.0,
            crusher: Crusher::default(),
            flanger: Flanger::new(sample_rate),
            phaser: Phaser::default(),
            low_pass: Filter::default(),
            high_pass: Filter::default(),
            delay: MultiFxDelay::new(sample_rate),
            reverb: MultiFxReverbState::new(sample_rate),
            panner: Panner::new(sample_rate),
            ring: Ring::default(),
            vowel: Vowel::default(),
            slots: [Slot::default(); KINDS],
        }
    }

    /// The controls for a block of `frames` frames at `tempo_bpm`, starting at song beat `start_beat` (negative:
    /// none, so the LFO runs on from its stored phase).
    pub(crate) fn prepare_block(
        &mut self,
        settings: &Settings,
        tempo_bpm: f32,
        start_beat: f64,
        frames: usize,
    ) {
        let Settings {
            x: x_raw,
            y,
            rate_beats,
            amount,
            shape,
        } = *settings;
        self.tempo_bpm = tempo_bpm;
        let phase = if start_beat >= 0.0 {
            (start_beat / rate_beats as f64).rem_euclid(1.0)
        } else {
            self.lfo_phase
        };
        // One LFO value for the block; the phase advances by the block for the next one.
        let step = if tempo_bpm >= 1.0 {
            frames as f64 * tempo_bpm as f64 / (self.sample_rate as f64 * 60.0 * rate_beats as f64)
        } else {
            0.0
        };
        self.lfo_phase = (phase + step).rem_euclid(1.0);
        // The block LFO's "one increment" (Saw's φ′) is the block's advance.
        let lfo = amount * lfo_wave(shape, phase, step);
        let x = (x_raw + lfo).clamp(0.0, 1.0);
        self.delay.configure(MultiFxDelaySettings {
            x: x_raw,
            y,
            rate_beats,
            lfo_amount: amount,
            lfo_shape: shape,
            tempo_bpm,
            start_beat,
        });
        // The reverb ignores the LFO.
        self.reverb.refresh(x_raw, y, self.sample_rate);
        let resonance = 1.0 + 2.0 * y * y;
        self.low_pass.set(
            2.0_f32.powf(6.0 + 8.28 * x),
            resonance,
            self.sample_rate,
            false,
        );
        self.high_pass.set(
            2.0_f32.powf(5.0 + 7.424 * x),
            resonance,
            self.sample_rate,
            true,
        );
        self.crusher.set(x_raw, y, lfo);
        self.phaser.set(x_raw, y, lfo, self.sample_rate);
        self.flanger.set(x_raw, y, lfo);
        self.panner.set(x_raw, y, lfo);
        self.ring.set(x_raw, lfo, self.sample_rate);
        self.vowel.set(x_raw, y, lfo, self.sample_rate);
    }

    /// One frame through the bank with `selected` enabled (none: every running processor winds down).
    pub(crate) fn process_frame(&mut self, input: [f32; 2], selected: Option<usize>) -> [f32; 2] {
        // BPM below 1 processes nothing.
        if self.tempo_bpm < 1.0 {
            return input;
        }
        let on = |kind| selected == Some(kind);
        let rate = self.sample_rate;
        let (quarter, second) = ((0.25 * rate) as usize, rate as usize);
        let slots = &mut self.slots;
        // DIST has no ramp or tail. Delay waits for 1.5 s of silent echoes, Reverb for 0.25 s; Flanger, Phaser, Ring
        // and Vox skip after 0.25 s without input, LP, HP and Panner after 1 s.
        let mut sample = if on(DIST) {
            self.crusher.process(input)
        } else {
            input
        };
        sample = slots[FLANGER].visit(&mut self.flanger, on(FLANGER), sample, quarter, 0);
        sample = slots[PHASER].visit(&mut self.phaser, on(PHASER), sample, quarter, 0);
        sample = slots[LPF].visit(&mut self.low_pass, on(LPF), sample, second, 0);
        sample = slots[HPF].visit(&mut self.high_pass, on(HPF), sample, second, 0);
        sample = slots[DELAY].visit(
            &mut self.delay,
            on(DELAY),
            sample,
            usize::MAX,
            (1.5 * rate) as usize,
        );
        sample = slots[REVERB].visit(&mut self.reverb, on(REVERB), sample, usize::MAX, quarter);
        sample = slots[PANNER].visit(&mut self.panner, on(PANNER), sample, second, 0);
        sample = slots[RING].visit(&mut self.ring, on(RING), sample, quarter, 0);
        slots[VOX].visit(&mut self.vowel, on(VOX), sample, quarter, 0)
    }

    /// Clears every processor. The LFO phases stay: resets never clear them.
    pub(crate) fn reset(&mut self) {
        self.crusher = Crusher::default();
        self.delay.reset();
        self.reverb.reset();
        self.flanger.reset();
        self.panner.reset();
        self.low_pass = Filter::default();
        self.high_pass = Filter::default();
        self.phaser = Phaser::default();
        self.ring = Ring::default();
        self.vowel = Vowel::default();
        self.slots = [Slot::default(); KINDS];
    }
}

/// The MultiFX effect (ID 4): one bank with the panel's eight kinds.
pub struct MultiFxState {
    enabled: bool,
    /// The bank kind the panel selects.
    kind: usize,
    settings: Settings,
    bank: Bank,
}

impl MultiFxState {
    pub fn new(params: &[f32; 7], sample_rate: f32) -> Self {
        let mut state = Self {
            enabled: false,
            kind: DELAY,
            settings: Settings {
                x: 0.5,
                y: 0.5,
                rate_beats: 1.0,
                amount: 0.0,
                shape: 0,
            },
            bank: Bank::new(sample_rate),
        };
        state.refresh(params, sample_rate);
        state
    }

    pub fn refresh(&mut self, params: &[f32; 7], sample_rate: f32) {
        if (sample_rate - self.bank.sample_rate).abs() > f32::EPSILON {
            // Off the audio thread: the rings are sized by the rate. The LFO phase and the tempo stay.
            self.bank = Bank {
                lfo_phase: self.bank.lfo_phase,
                tempo_bpm: self.bank.tempo_bpm,
                ..Bank::new(sample_rate)
            };
        }
        self.enabled = params[0] >= 0.5;
        self.kind = VISIBLE[((params[1].clamp(0.0, 1.0) * 7.0) as usize).min(7)];
        self.settings = Settings {
            x: params[2].clamp(0.0, 1.0),
            y: params[3].clamp(0.0, 1.0),
            rate_beats: RATE_BEATS[((params[4].clamp(0.0, 1.0) * 9.0) as usize).min(9)],
            amount: params[5] * params[5].abs(),
            shape: ((params[6].clamp(0.0, 1.0) * 2.0) as usize).min(2),
        };
    }

    pub fn prepare_block(&mut self, tempo_bpm: f32, start_beat: f64, frames: usize) {
        self.bank
            .prepare_block(&self.settings, tempo_bpm, start_beat, frames);
    }

    pub fn process_frame(&mut self, left: f32, right: f32) -> Result<(f32, f32), MultiFxError> {
        if !self.enabled {
            return Ok((left, right));
        }
        let [left, right] = self.bank.process_frame([left, right], Some(self.kind));
        Ok((left, right))
    }

    /// Clears every processor. The LFO phases stay: resets never clear them.
    pub fn reset(&mut self) {
        self.bank.reset();
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const RATE: f32 = 48_000.0;

    /// Prepares `bank` at 120 BPM from song beat 0 (LFO phase 0, rate 1 beat).
    fn prepare(bank: &mut Bank, x: f32, y: f32, amount: f32, shape: usize) {
        let settings = Settings {
            x,
            y,
            rate_beats: 1.0,
            amount,
            shape,
        };
        bank.prepare_block(&settings, 120.0, 0.0, 64);
    }

    fn bank(x: f32, y: f32, amount: f32, shape: usize) -> Bank {
        let mut bank = Bank::new(RATE);
        prepare(&mut bank, x, y, amount, shape);
        bank
    }

    fn input(i: usize) -> [f32; 2] {
        let t = i as f32;
        [(t * 0.37).sin(), -0.5 * (t * 0.11).cos()]
    }

    fn near(actual: [f32; 2], expected: [f32; 2], frame: usize) {
        for (a, e) in actual.into_iter().zip(expected) {
            assert!(
                (a - e).abs() <= 1e-6 * (1.0 + e.abs()),
                "frame {frame}: {actual:?} != {expected:?}"
            );
        }
    }

    #[test]
    fn dist_latches_its_saturator_every_h_frames_and_stands_still_while_off() {
        // X = 0.3: H = trunc(min(Y⁴, 1)·256) + 1 (62 at Y = 0.7, 257 at Y = 1), g = 0.1 − 0.09Y; k = 10^(4(X + LFO) + 1),
        // unclamped: Square at phase 0 is +A.
        for (y, hold, amount, exponent) in [
            (0.7, 62, 0.0, 2.2),
            (0.7, 62, 0.6, 4.6),
            (0.7, 62, -0.9, -1.4),
            (1.0, 257, 0.0, 2.2),
        ] {
            let mut bank = bank(0.3, y, amount, 2);
            let (g, k) = (0.1 - 0.09 * y, 10.0_f32.powf(exponent));
            let law = |x: [f32; 2]| x.map(|x| x * g * (k + 1.0) / (1.0 + k * x.abs()));
            let first = 3 * hold + 14;
            for i in 0..first {
                near(
                    bank.process_frame(input(i), Some(DIST)),
                    law(input(i - i % hold)),
                    i,
                );
            }
            // Off: the input passes untouched and so does the hold count (14 frames into frame 3H's hold).
            for i in 0..10 {
                assert_eq!(bank.process_frame(input(i), None), input(i));
            }
            for i in first..first + 2 * hold {
                let latched = 3 * hold + (i - 3 * hold) / hold * hold;
                near(
                    bank.process_frame(input(i), Some(DIST)),
                    law(input(latched)),
                    i,
                );
            }
        }
    }

    #[test]
    fn ring_multiplies_by_its_carrier_with_w_and_g_fading_in() {
        // A Square LFO at +0.4: 2^(5 + 8.7·clamp(X + 0.5·0.4, 0, 1)) Hz. Y is unused. w and g both start at 0 and
        // approach 1 by 0.005 a frame, so m = w·g = (1 − 0.995ⁿ⁺¹)².
        for (x, y, octaves) in [(0.4, 0.0, 0.6), (0.4, 1.0, 0.6), (0.9, 0.5, 1.0)] {
            let step = f64::from((5.0 + 8.7 * octaves as f32).exp2() / RATE);
            let expected = |n: usize| {
                let m = (1.0 - 0.995_f64.powi(n as i32 + 1)).powi(2) as f32;
                let carrier = (std::f64::consts::TAU * (n as f64 * step).fract()).sin() as f32;
                input(n).map(|x| x * (1.0 - m) + m * x * carrier)
            };
            let fresh: Vec<[f32; 2]> = (0..3_000).map(expected).collect();
            let mut bank = bank(x, y, 0.4, 2);
            for (n, &e) in fresh.iter().enumerate() {
                let out = bank.process_frame(input(n), Some(RING));
                for (a, e) in out.into_iter().zip(e) {
                    assert!(
                        (a - e).abs() < 2e-4,
                        "x {x} y {y} frame {n}: {out:?} != {e}"
                    );
                }
            }
            // Off, it winds down until w < 1e-4, then clears: the input passes exactly.
            for n in 0..1_838 {
                bank.process_frame(input(n), None);
            }
            for n in 1_838..2_000 {
                assert_eq!(bank.process_frame(input(n), None), input(n), "frame {n}");
            }
            // Selected again at the next block, it starts over: phase 0, w and g from 0.
            prepare(&mut bank, x, y, 0.4, 2);
            for (n, &e) in fresh.iter().enumerate().take(500) {
                let out = bank.process_frame(input(n), Some(RING));
                assert!((out[0] - e[0]).abs() < 2e-4, "again, frame {n}");
            }
            // 0.5 s without input: it runs 0.25 s, then is skipped, so the carrier stops after 500 + 12 000 frames.
            for _ in 0..24_000 {
                bank.process_frame([0.0; 2], Some(RING));
            }
            for n in 0..200 {
                let carrier =
                    (std::f64::consts::TAU * ((12_500 + n) as f64 * step).fract()).sin() as f32;
                let out = bank.process_frame(input(n), Some(RING));
                for (a, i) in out.into_iter().zip(input(n)) {
                    assert!(
                        (a - i * carrier).abs() < 1e-3,
                        "after the skip, frame {n}: {a} != {}",
                        i * carrier
                    );
                }
            }
        }
    }
}
