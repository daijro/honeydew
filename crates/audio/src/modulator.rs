//! Modulator (module type [`MODULATOR`]), Honeydew's own effect: its audio passes through unchanged, and it makes a
//! modulation signal that the host adds to another param of the song, its target ([`crate::AudioEffect::modulation`]).
//!
//! **Params**, normalized, in this order (the saved one): 0 Enable, [`SOURCE`], [`AMOUNT`], the Envelope's [`GAIN`],
//! [`ATTACK`] and [`RELEASE`], then [`SYNC`], [`RATE`] and [`DIVISION`] (the LFO's and Random's), the LFO's [`SHAPE`] and
//! Random's [`SMOOTH`]. Post Gain, Mix and Wet Pan are the rack's (`CommonModule`). The laws from `v`:
//! - Source `round(2v)`: 0 Envelope, 1 LFO, 2 Random ([`Source`]). Shape `round(3v)`: Sine, Triangle, Saw, Square.
//! - Amount `−1 + 2v` (−100 … +100 %): the offset is Amount × the signal, in normalized param units.
//! - Gain `−24 + 48v` dB. Attack `1000^v` ms (1 ms to 1 s), Release `10·1000^v` ms (10 ms to 10 s).
//! - Sync on from 0.5. Rate `0.05·400^v` Hz (0.05 to 20 Hz, 1 Hz at 0.5). Division `round(7v)` of [`DIVISIONS`], a
//!   cycle's length in beats. Smooth `v`.
//!
//! **Signals.**
//! - Envelope (unipolar, 0…1): the louder channel's `|x|` of the key (the module's own input until a sidechain feeds
//!   one) × Gain, followed by an envelope that rises with Attack and falls with Release (one-pole,
//!   `c = e^(−1/(t·rate))`), held under 1.
//! - LFO (bipolar, −1…1) at phase `φ` of its cycle: Sine `sin 2πφ`, Triangle and Saw rising through 0 at `φ = 0`, Square
//!   +1 then −1.
//! - Random (unipolar, 0…1): a new random value at each cycle's start, reached from the last one over Smooth × the cycle
//!   (0 steps).
//!
//! The cycle runs at Rate, or with Sync on at its Division of the tempo; synced while the transport plays, its phase is
//! the song beat's, so it lands on the beat wherever playback starts.
//!
//! Nothing allocates while it runs.

use crate::ProcessContext;
use crate::biquad::flush;

/// The module type: Honeydew's own id, which a song file keeps apart from FL Studio Mobile's modules.
pub const MODULATOR: u16 = 1005;

pub const SOURCE: usize = 1;
pub const AMOUNT: usize = 2;
pub const GAIN: usize = 3;
pub const ATTACK: usize = 4;
pub const RELEASE: usize = 5;
pub const SYNC: usize = 6;
pub const RATE: usize = 7;
pub const DIVISION: usize = 8;
pub const SHAPE: usize = 9;
pub const SMOOTH: usize = 10;
pub const PARAMS: usize = 11;

/// The synced cycles, in beats, with their names.
pub const DIVISIONS: [(f64, &str); 8] = [
    (0.125, "1/8"),
    (0.25, "1/4"),
    (0.5, "1/2"),
    (1.0, "beat"),
    (2.0, "2 beats"),
    (4.0, "4 beats"),
    (8.0, "8 beats"),
    (16.0, "16 beats"),
];

/// A new module's params: an Envelope at +50 %, 0 dB, 10 ms and 100 ms; a free 1 Hz sine; Smooth 50 %.
pub const DEFAULTS: [f32; PARAMS] = [
    1.0,
    0.0,
    0.75,
    0.5,
    1.0 / 3.0,
    1.0 / 3.0,
    0.0,
    0.5,
    3.0 / 7.0,
    0.0,
    0.5,
];

/// What makes the signal, `round(2v)` of Source.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum Source {
    #[default]
    Envelope,
    Lfo,
    Random,
}

impl Source {
    pub fn from_param(v: f32) -> Self {
        match (2.0 * v.clamp(0.0, 1.0)).round() as u8 {
            0 => Self::Envelope,
            1 => Self::Lfo,
            _ => Self::Random,
        }
    }

    /// The signal is −1…1 (an LFO), not 0…1.
    pub fn bipolar(self) -> bool {
        self == Self::Lfo
    }
}

/// Rate, Hz.
pub fn rate_hz(v: f32) -> f32 {
    0.05 * 400f32.powf(v)
}

/// The LFO's wave at phase `phase` (0…1), `shape` the Shape item (0 Sine, 1 Triangle, 2 Saw, 3 Square).
fn wave(shape: u8, phase: f32) -> f32 {
    match shape {
        0 => (std::f32::consts::TAU * phase).sin(),
        1 => 1.0 - 4.0 * ((phase + 0.25).fract() - 0.5).abs(),
        2 => 2.0 * (phase + 0.5).fract() - 1.0,
        _ if phase < 0.5 => 1.0,
        _ => -1.0,
    }
}

/// The Modulator's state.
#[derive(Default)]
pub struct ModulatorState {
    source: Source,
    shape: u8,
    amount: f32,
    gain: f32,
    /// The envelope's coefficients.
    attack: f32,
    release: f32,
    sync: bool,
    hz: f32,
    period_beats: f64,
    smooth: f32,
    sample_rate: f32,
    /// The cycle's phase (0…1) and its step per frame.
    phase: f64,
    step: f64,
    /// The song beat now while a synced cycle follows it, else negative; and its step per frame.
    beat: f64,
    beat_step: f64,
    envelope: f32,
    /// Random's last value and its next, and the generator's state.
    from: f32,
    to: f32,
    seed: u32,
    /// The signal now.
    signal: f32,
}

impl ModulatorState {
    /// Takes the params: coefficients only, no sample.
    pub fn refresh(&mut self, params: &[f32], sample_rate: f32) {
        let v = |i: usize| params.get(i).copied().unwrap_or(DEFAULTS[i]);
        let coef = |ms: f32| (-1000.0 / (ms * sample_rate)).exp();
        self.source = Source::from_param(v(SOURCE));
        self.shape = (3.0 * v(SHAPE).clamp(0.0, 1.0)).round() as u8;
        self.amount = 2.0f32.mul_add(v(AMOUNT), -1.0);
        self.gain = 10f32.powf(48.0f32.mul_add(v(GAIN), -24.0) / 20.0);
        self.attack = coef(1000f32.powf(v(ATTACK)));
        self.release = coef(10.0 * 1000f32.powf(v(RELEASE)));
        self.sync = v(SYNC) >= 0.5;
        self.hz = rate_hz(v(RATE));
        let division = (7.0 * v(DIVISION).clamp(0.0, 1.0)).round() as usize;
        self.period_beats = DIVISIONS[division.min(DIVISIONS.len() - 1)].0;
        self.smooth = v(SMOOTH).clamp(0.0, 1.0);
        self.sample_rate = sample_rate;
    }

    /// Back to rest: no envelope, the cycle at its start, Random on a fresh value (the same each time).
    pub fn reset(&mut self) {
        (self.phase, self.envelope, self.seed) = (0.0, 0.0, 0x9E37_79B9);
        let first = self.random();
        (self.from, self.to) = (first, first);
        self.signal = match self.source {
            Source::Random => first,
            _ => 0.0,
        };
    }

    /// The block's timing: the cycle's step per frame at `context`'s tempo, and the song beat a synced cycle follows
    /// while the transport plays (a negative start beat: it does not).
    pub fn prepare_block(&mut self, context: ProcessContext) {
        let rate = f64::from(context.sample_rate);
        let beats_per_frame = f64::from(context.tempo_bpm) / (60.0 * rate);
        self.step = match self.sync {
            true => beats_per_frame / self.period_beats,
            false => f64::from(self.hz) / rate,
        };
        self.beat = if self.sync { context.start_beat } else { -1.0 };
        self.beat_step = beats_per_frame;
    }

    /// The offset for the host: Amount × the signal.
    pub fn offset(&self) -> f32 {
        self.amount * self.signal
    }

    /// The signal now: 0…1, or −1…1 for an LFO.
    pub fn signal(&self) -> f32 {
        self.signal
    }

    /// One frame, its detector fed `key`: the audio passes unchanged.
    pub fn process_frame(&mut self, l: f32, r: f32, key: (f32, f32)) -> (f32, f32) {
        if self.source == Source::Envelope {
            let level = key.0.abs().max(key.1.abs()) * self.gain;
            let coef = if level > self.envelope {
                self.attack
            } else {
                self.release
            };
            self.envelope = flush(level + coef * (self.envelope - level));
            self.signal = self.envelope.min(1.0);
            return (l, r);
        }
        let before = self.phase;
        if self.beat >= 0.0 {
            self.phase = (self.beat / self.period_beats).fract();
            self.beat += self.beat_step;
        } else {
            self.phase = (self.phase + self.step).fract();
        }
        let phase = self.phase as f32;
        self.signal = match self.source {
            Source::Lfo => wave(self.shape, phase),
            _ => {
                if self.phase < before {
                    (self.from, self.to) = (self.signal, self.random());
                }
                let glide = if self.smooth > 0.0 {
                    (phase / self.smooth).min(1.0)
                } else {
                    1.0
                };
                self.from + (self.to - self.from) * glide
            }
        };
        (l, r)
    }

    /// The next random value, 0…1 (xorshift32).
    fn random(&mut self) -> f32 {
        let mut x = self.seed;
        x ^= x << 13;
        x ^= x >> 17;
        x ^= x << 5;
        self.seed = x;
        (x >> 8) as f32 / (1 << 24) as f32
    }
}
