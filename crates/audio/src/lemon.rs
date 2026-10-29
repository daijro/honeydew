//! Lemon Studio's hall reverb and its limiter (with compressor and noise gate), vendored from Lemon Studio
//! (`vendor/lemon-studio/os-engine`, GPL-3.0-or-later), as rack effects of Honeydew's own module types.
//!
//! A module's params are Enable, then Lemon's stored knob values in Lemon's order, each normalized over its knob's
//! range, then the switches. FL Studio's Fruity Reeverb 2 and Fruity Limiter saved states hold the same stored values
//! ([`reverb_from_fl`], [`limiter_fl_state`]).

pub use os_engine::effects::{dynamics, reverb};

use dynamics::Dynamics;
use reverb::{Reverb, Settings};

/// The module types: Honeydew's own ids, which a song file keeps apart from the format's own modules.
pub const REVERB: u16 = 1002;
pub const LIMITER: u16 = 1003;

/// The reverb's switches, after Enable and its 15 knobs: Tempo-based predelay and Mid/side input.
pub const REVERB_TEMPO: usize = 1 + reverb::PARAMS;
pub const REVERB_MID_SIDE: usize = REVERB_TEMPO + 1;
/// The limiter's switch, after Enable and its 18 knobs: on, the limiter aims 0.2 dB under the ceiling (Lemon's flags
/// bit 0 clear).
pub const LIMITER_SAFETY: usize = 1 + dynamics::PARAMS;

/// The slowest tempo a tempo-based pre-delay follows: the reverb's pre-delay line is sized for it when it is built.
const MIN_BPM: f64 = 30.0;

/// Whether an edit of param `param` of a module of type `type_id` is heard only from the engine built after it: Lemon
/// Reverb's Room Size ([`ReverbFx`]).
pub fn rebuilds(type_id: u16, param: i32) -> bool {
    type_id == REVERB && param == 1 + reverb::ROOM as i32
}

/// Stored value `x` of a knob with range `(min, max)` as a normalized param.
pub const fn norm((min, max): (i32, i32), x: i32) -> f32 {
    let x = if x < min {
        min
    } else if x > max {
        max
    } else {
        x
    };
    (x - min) as f32 / (max - min) as f32
}

/// The stored value of normalized param `v` of a knob with range `(min, max)`.
pub fn stored((min, max): (i32, i32), v: f32) -> i32 {
    min + ((max - min) as f32 * v.clamp(0.0, 1.0)).round() as i32
}

/// The params of a new module of type `id`: Lemon's fresh instance. Empty for another type.
pub fn defaults(id: u16) -> Vec<f32> {
    match id {
        REVERB => reverb_params(&Settings::default()),
        LIMITER => limiter_params(&dynamics::DEFAULTS, 0),
        _ => Vec::new(),
    }
}

fn reverb_params(s: &Settings) -> Vec<f32> {
    let knobs = (0..reverb::PARAMS).map(|k| norm(reverb::RANGES[k], s.knobs[k]));
    let switches = [s.tempo_sync, s.mid_side].map(f32::from);
    [1.0].into_iter().chain(knobs).chain(switches).collect()
}

fn limiter_params(values: &[i32; dynamics::PARAMS], flags: u8) -> Vec<f32> {
    let knobs = (0..dynamics::PARAMS).map(|k| norm(dynamics::RANGES[k], values[k]));
    let safety = f32::from(flags & dynamics::FLAG_EXACT_CEILING == 0);
    [1.0].into_iter().chain(knobs).chain([safety]).collect()
}

/// Lemon's reverb settings for the params (a missing one at its default).
pub fn reverb_settings(params: &[f32]) -> Settings {
    let mut s = Settings::default();
    for (k, knob) in s.knobs.iter_mut().enumerate() {
        if let Some(&v) = params.get(1 + k) {
            *knob = stored(reverb::RANGES[k], v);
        }
    }
    let on = |i: usize, default: bool| params.get(i).map_or(default, |&v| v >= 0.5);
    s.tempo_sync = on(REVERB_TEMPO, s.tempo_sync);
    s.mid_side = on(REVERB_MID_SIDE, s.mid_side);
    s
}

/// Lemon's limiter values and flags for the params (a missing one at its default).
pub fn limiter_values(params: &[f32]) -> ([i32; dynamics::PARAMS], u8) {
    let values = std::array::from_fn(|k| {
        params
            .get(1 + k)
            .map_or(dynamics::DEFAULTS[k], |&v| stored(dynamics::RANGES[k], v))
    });
    let safety = params.get(LIMITER_SAFETY).is_none_or(|&v| v >= 0.5);
    (
        values,
        if safety {
            0
        } else {
            dynamics::FLAG_EXACT_CEILING
        },
    )
}

// ---- FL Studio's saved states ----

/// The params for a Fruity Reeverb 2 saved state, any of its versions as Lemon reads them; `None` for one it cannot
/// read.
pub fn reverb_from_fl(state: &[u8]) -> Option<Vec<f32>> {
    Settings::from_state(state).map(|s| reverb_params(&s))
}

/// The Fruity Reeverb 2 saved state of the params (its current 66-byte layout).
pub fn reverb_fl_state(params: &[f32]) -> Vec<u8> {
    reverb_settings(params).to_state()
}

/// Fruity Limiter's saved state, as Lemon Limiter reads and writes it: int32 version 7, then the 18 values, a spare
/// bank of 18 from byte 76, the editor's bytes, and the flags in the last byte.
const LIMITER_STATE: usize = 169;
const LIMITER_VERSION: i32 = 7;
const SPARE_AT: usize = 76;
const FLAGS_AT: usize = LIMITER_STATE - 1;

/// The params for a Fruity Limiter saved state; values a short state lacks are the defaults.
pub fn limiter_from_fl(state: &[u8]) -> Vec<f32> {
    let int = |at: usize| {
        let bytes = state.get(at..at + 4)?;
        Some(i32::from_le_bytes(bytes.try_into().ok()?))
    };
    let values = std::array::from_fn(|k| int(4 * (k + 1)).unwrap_or(dynamics::DEFAULTS[k]));
    limiter_params(&values, state.get(FLAGS_AT).copied().unwrap_or(0))
}

/// The Fruity Limiter saved state of the params: a fresh instance's (the spare bank at the defaults, no sidechain)
/// with these values and flags.
pub fn limiter_fl_state(params: &[f32]) -> Vec<u8> {
    let (values, flags) = limiter_values(params);
    let mut b = vec![0; LIMITER_STATE];
    b[..4].copy_from_slice(&LIMITER_VERSION.to_le_bytes());
    for (k, v) in values.iter().enumerate() {
        b[4 * (k + 1)..][..4].copy_from_slice(&v.to_le_bytes());
        b[SPARE_AT + 4 * k..][..4].copy_from_slice(&dynamics::DEFAULTS[k].to_le_bytes());
    }
    b[FLAGS_AT] = flags;
    b
}

// ---- The processors ----

/// Lemon's reverb in a module. A new room size resizes its lines, which the audio thread never does: once a block has
/// run, [`Self::live`], it keeps the room it has, and an edit's engine, built off the audio thread at the new size,
/// hands its reverb over ([`Self::take`]).
pub(crate) struct ReverbFx {
    dsp: Reverb,
    rate: u32,
    pub(crate) live: bool,
}

impl ReverbFx {
    pub(crate) fn new(sample_rate: f32, params: &[f32]) -> Self {
        let rate = sample_rate.round().max(1.0) as u32;
        let settings = reverb_settings(params);
        // The pre-delay line at its longest now: tempo-based, at its top, at the slowest tempo followed.
        let mut longest = settings;
        longest.tempo_sync = true;
        longest.knobs[reverb::PREDELAY] = reverb::RANGES[reverb::PREDELAY].1;
        let mut dsp = Reverb::new(rate, longest, MIN_BPM);
        dsp.process(&mut [], &mut []);
        dsp.set(settings);
        Self {
            dsp,
            rate,
            live: false,
        }
    }

    /// The params at `sample_rate` (another rate builds it again).
    pub(crate) fn set(&mut self, params: &[f32], sample_rate: f32) {
        if sample_rate.round() as u32 != self.rate {
            *self = Self::new(sample_rate, params);
            return;
        }
        let mut s = reverb_settings(params);
        if self.live {
            s.knobs[reverb::ROOM] = self.dsp.settings().knobs[reverb::ROOM];
        }
        self.dsp.set(s);
    }

    pub(crate) fn set_tempo(&mut self, bpm: f32) {
        self.dsp.set_tempo(f64::from(bpm).max(MIN_BPM));
    }

    pub(crate) fn frame(&mut self, l: f32, r: f32) -> (f32, f32) {
        self.dsp.frame(l, r)
    }

    pub(crate) fn clear(&mut self) {
        self.dsp.clear();
    }

    /// Takes the reverb of `built` (this module built again for an edit's engine) where its room size is another: the
    /// room changes and the tail is cleared, as Lemon's reverb clears it. Allocates nothing.
    pub(crate) fn take(&mut self, built: &mut ReverbFx) {
        let room = |fx: &ReverbFx| fx.dsp.settings().knobs[reverb::ROOM];
        if room(built) != room(self) {
            std::mem::swap(&mut self.dsp, &mut built.dsp);
        }
    }
}

/// Lemon's limiter in a module, its buffers sized for every setting when it is built.
pub(crate) struct LimiterFx {
    dsp: Dynamics,
    rate: f32,
}

impl LimiterFx {
    pub(crate) fn new(sample_rate: f32, params: &[f32]) -> Self {
        use dynamics::{LIM_ATT, PEAK_WINDOW, RANGES, RMS_WINDOW};
        let (values, flags) = limiter_values(params);
        let rate = f64::from(sample_rate);
        // The look-ahead and both windows at their longest (1 s), and the peak window's queue grown to hold 1 s of
        // falling input: no later setting or input outgrows them. Then every buffer is emptied, each by a change of
        // its length.
        let sized = [LIM_ATT, PEAK_WINDOW, RMS_WINDOW];
        let (mut longest, mut shortest) = (values, values);
        for k in sized {
            (longest[k], shortest[k]) = (RANGES[k].1, RANGES[k].0);
        }
        let mut dsp = Dynamics::new(rate, longest, flags);
        let n = dynamics::time_samples(RANGES[PEAK_WINDOW].1, rate).round() as usize + 1;
        let mut left: Vec<f32> = (0..n).map(|i| 1.0 - 0.5 * i as f32 / n as f32).collect();
        let mut right = left.clone();
        dsp.process(&mut left, &mut right, None);
        dsp.set_params(&shortest);
        dsp.set_params(&values);
        dsp.reset();
        Self {
            dsp,
            rate: sample_rate,
        }
    }

    /// The params at `sample_rate` (another rate builds it again).
    pub(crate) fn set(&mut self, params: &[f32], sample_rate: f32) {
        if sample_rate != self.rate {
            *self = Self::new(sample_rate, params);
            return;
        }
        let (values, flags) = limiter_values(params);
        self.dsp.set_params(&values);
        self.dsp.set_flags(flags);
    }

    // ponytail: Lemon's block process called one frame at a time, as the engine runs its built-in effects; its silence
    // sleep and state flush then act per frame. A block path for it if the per-call cost ever shows.
    pub(crate) fn frame(&mut self, l: f32, r: f32) -> (f32, f32) {
        self.keyed(l, r, None)
    }

    /// [`Self::frame`] with its compressor reading `key` (left, right) instead of the frame: Lemon's own sidechain input.
    pub(crate) fn keyed(&mut self, l: f32, r: f32, key: Option<(f32, f32)>) -> (f32, f32) {
        let (mut left, mut right) = ([l], [r]);
        let key = key.map(|(key_l, key_r)| ([key_l], [key_r]));
        let key = key.as_ref().map(|(key_l, key_r)| (&key_l[..], &key_r[..]));
        self.dsp.process(&mut left, &mut right, key);
        (left[0], right[0])
    }

    pub(crate) fn reset(&mut self) {
        self.dsp.reset();
    }

    pub(crate) fn latency(&self) -> usize {
        self.dsp.latency()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{AudioEffect, ProcessContext};

    const RATE: f32 = 48_000.0;
    const CONTEXT: ProcessContext = ProcessContext {
        sample_rate: RATE,
        tempo_bpm: 120.0,
        start_beat: 0.0,
        playing: true,
    };

    fn run(effect: &mut AudioEffect, left: &mut [f32], right: &mut [f32]) {
        for (l, r) in left.chunks_mut(512).zip(right.chunks_mut(512)) {
            effect.process(l, r, CONTEXT).unwrap();
        }
    }

    #[test]
    fn an_impulse_rings_through_both_and_the_limiter_holds_its_ceiling() {
        // An impulse: the reverb's tail and the limiter's look-ahead-delayed copy, finite.
        for (id, after) in [(REVERB, 4_800), (LIMITER, 1)] {
            let mut effect = AudioEffect::new(id, RATE).unwrap();
            let (mut left, mut right) = (vec![0.0; 24_000], vec![0.0; 24_000]);
            (left[0], right[0]) = (0.8, 0.8);
            run(&mut effect, &mut left, &mut right);
            assert!(left.iter().chain(&right).all(|v| v.is_finite()), "{id}");
            assert!(left[after..].iter().any(|v| v.abs() > 1e-4), "{id}: silent");
        }
        // A tone driven 18 dB over the ceiling (0 dB, aimed 0.2 dB under) stays under it once settled.
        let mut limiter = AudioEffect::new(LIMITER, RATE).unwrap();
        limiter.set_parameter(1 + dynamics::GAIN, 1.0).unwrap();
        let tone: Vec<f32> = (0..24_000).map(|i| 0.9 * (i as f32 * 0.05).sin()).collect();
        let (mut left, mut right) = (tone.clone(), tone);
        run(&mut limiter, &mut left, &mut right);
        let peak = left[4_800..].iter().fold(0f32, |m, v| m.max(v.abs()));
        assert!(peak <= 1.0 && peak > 0.9, "{peak}");
        // A room size set while it runs waits for the reverb an edit's engine builds at that size.
        let mut running = AudioEffect::new(REVERB, RATE).unwrap();
        run(&mut running, &mut [0.0; 64], &mut [0.0; 64]);
        let room = 1 + reverb::ROOM;
        running.set_parameter(room, 1.0).unwrap();
        let size = |e: &AudioEffect| match &e.processor {
            crate::Processor::LemonReverb(fx) => fx.dsp.settings().knobs[reverb::ROOM],
            _ => unreachable!(),
        };
        assert_eq!(size(&running), reverb::DEFAULTS[reverb::ROOM]);
        let mut built = AudioEffect::new(REVERB, RATE).unwrap();
        built.set_parameter(room, 1.0).unwrap();
        running.take_buffers(&mut built);
        assert_eq!(size(&running), reverb::RANGES[reverb::ROOM].1);
    }
}
