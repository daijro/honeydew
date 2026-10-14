//! The metronome click: a square wave at 2500 Hz on a bar's
//! first beat and 2000 Hz on the others, amplitude ±0.631·level·env, env 1 when the click starts and 0.001 lower
//! each sample. The engine decides when a click starts (each beat, the count-in's beats).

/// The accent: a bar's first beat.
pub const ACCENT_HZ: f32 = 2500.0;
/// Every other beat.
pub const BEAT_HZ: f32 = 2000.0;
const AMPLITUDE: f32 = 0.631;
/// What the envelope loses each sample; it is silent from 1/DECAY samples on.
const DECAY: f32 = 0.001;

/// The click level for the normalized `Metronome level` setting v: 0.1 + 0.9·v².
pub fn level(v: f32) -> f32 {
    0.1 + 0.9 * v * v
}

/// One click voice: a new click cuts the sounding one off.
pub struct Metronome {
    sample_rate: f32,
    hz: f32,
    /// The click's level (see [`level`]).
    gain: f32,
    /// Samples since the click started.
    n: u32,
}

impl Metronome {
    /// A silent metronome at `sample_rate`.
    pub fn new(sample_rate: f32) -> Self {
        Self {
            sample_rate,
            hz: BEAT_HZ,
            gain: 0.0,
            n: 0,
        }
    }

    /// Starts a click at the next sample: 2500 Hz if `accent`, else 2000 Hz, at `level`.
    pub fn click(&mut self, accent: bool, level: f32) {
        self.hz = if accent { ACCENT_HZ } else { BEAT_HZ };
        self.gain = level;
        self.n = 0;
    }

    /// Adds the sounding click to both sides; equal lengths. Never allocates.
    pub fn add(&mut self, left: &mut [f32], right: &mut [f32]) {
        for (l, r) in left.iter_mut().zip(right) {
            let env = 1.0 - DECAY * self.n as f32;
            if env <= 0.0 || self.gain == 0.0 {
                return;
            }
            // The high half of each period first.
            let high = (self.n as f32 * self.hz / self.sample_rate).fract() < 0.5;
            let sample = AMPLITUDE * self.gain * env * if high { 1.0 } else { -1.0 };
            *l += sample;
            *r += sample;
            self.n += 1;
        }
    }
}
