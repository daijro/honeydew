//! Half Time (module type [`HALF_TIME`]), Honeydew's own effect (GitHub #91).
//! Tape-style slow down on the beat: each cycle of Length plays the start of its input at Speed, by default its first
//! half at half speed, an octave down.
//!
//! **Params**, normalized, in this order (the saved one): 0 Enable, [`OLD_LENGTH`], [`MIX`], and the Honeydew additions
//! of GitHub #122, appended so that older saves load as they were: [`LENGTH`], [`SPEED`], [`SMOOTH`]. Post Gain, Mix
//! and Wet Pan are the rack's too (`CommonModule`). The laws from `v`:
//! - Length `round(7v)` of [`LENGTHS`]: 1/16 bar to 8 bars, in bars of four beats. It was param 1 before (`round(3v)`
//!   of 1/4 bar to 2 bars), which [`upgrade`] reads into it from an older save; the effect ignores param 1 now.
//! - Mix `v`: `dry + v·(wet − dry)`.
//! - Speed `round(2v)` of [`SPEEDS`]: 2/3 (a 1.5× slow down), 1/2 or 1/4.
//! - Smooth `50v` ms: the crossfade at each cycle's start; 0 cuts.
//!
//! **Cycles.** While the transport plays they lie on the song's beat grid, one from each multiple of Length's beats;
//! stopped, they run on from the last. At frame `n` of a cycle that began at frame `s`, the wet signal is the input at
//! `s + r·(n − s)`, `r` the Speed, linearly interpolated. A cycle's start, a jump of the song position and a change of
//! Speed crossfade from the playback running on over Smooth (equal power), so none clicks.
//!
//! **Ring.** What a cycle plays lags its input by up to `1 − r` of the cycle: the ring holds that for 8 bars at 1/4 at
//! the tempo it is prepared for ([`HalfTimeState::prepare`], off the audio thread), at most [`MAX_SECONDS`]. At a
//! slower tempo a cycle ends where the ring runs out. Nothing allocates while it runs.

use std::f32::consts::FRAC_PI_2;

use crate::ProcessContext;

/// The module type: Honeydew's own id, which a song file keeps apart from FL Studio Mobile's modules.
pub const HALF_TIME: u16 = 1008;

/// Length before 0.5.4 (GitHub #122): `round(3v)` of 1/4 bar, 1/2 bar, 1 bar or 2 bars. Only [`upgrade`] reads it.
pub const OLD_LENGTH: usize = 1;
pub const MIX: usize = 2;
pub const LENGTH: usize = 3;
pub const SPEED: usize = 4;
pub const SMOOTH: usize = 5;
/// Enable and the five above.
pub const PARAMS: usize = 6;

/// The cycle lengths, in beats, with their names.
// ponytail: bars of four beats; the effect's context has no time signature, pass it in if 3/4 songs need true bars.
pub const LENGTHS: [(f64, &str); 8] = [
    (0.25, "1/16 bar"),
    (0.5, "1/8 bar"),
    (1.0, "1/4 bar"),
    (2.0, "1/2 bar"),
    (4.0, "1 bar"),
    (8.0, "2 bars"),
    (16.0, "4 bars"),
    (32.0, "8 bars"),
];

/// The playback rates, with their names.
pub const SPEEDS: [(f64, &str); 3] = [(2.0 / 3.0, "2/3"), (0.5, "1/2"), (0.25, "1/4")];

/// A new module's params: 1 bar, Mix 100 %, Speed 1/2, Smooth 10 ms (and the old Length at 1 bar).
pub const DEFAULTS: [f32; PARAMS] = [1.0, 2.0 / 3.0, 1.0, 4.0 / 7.0, 0.5, 0.2];

/// Brings `params` read from a save that held `saved` of them up to date: a save from before Length moved to
/// [`LENGTH`] (GitHub #122) has its length in [`OLD_LENGTH`].
pub fn upgrade(params: &mut [f32], saved: usize) {
    if saved <= LENGTH && params.len() > LENGTH {
        let old = ((params[OLD_LENGTH].clamp(0.0, 1.0) * 3.0).round() as usize).min(3);
        // 1/4 bar is item 2 of the eight.
        params[LENGTH] = (old + 2) as f32 / 7.0;
    }
}

/// The most audio the ring holds, s: the 24-beat lag of 8 bars at 1/4 down to 72 BPM.
// ponytail: a ring per instance of up to 8 MB at 48 kHz; keep only the segment a cycle plays if that is ever too much.
pub const MAX_SECONDS: f32 = 20.0;

/// Smooth's longest crossfade, s.
const SMOOTH_SECONDS: f32 = 0.05;
/// The slowest of [`SPEEDS`] and the longest of [`LENGTHS`], in beats: what the ring is sized for.
const SLOWEST: f64 = 0.25;
const LONGEST: f64 = 32.0;

/// Length's index for `v`.
pub fn length_index(v: f32) -> usize {
    ((v.clamp(0.0, 1.0) * 7.0).round() as usize).min(7)
}

/// Speed's index for `v`.
pub fn speed_index(v: f32) -> usize {
    ((v.clamp(0.0, 1.0) * 2.0).round() as usize).min(2)
}

pub struct HalfTimeState {
    sample_rate: f32,
    /// The tempo the ring is sized for.
    tempo_bpm: f32,
    beats: f64,
    mix: f32,
    /// Speed, and the one of the playback a fade leaves.
    rate: f64,
    old_rate: f64,
    /// The input, a power of two long, frame `n` at `n & (len − 1)`.
    left: Vec<f32>,
    right: Vec<f32>,
    /// Frames written.
    frame: u64,
    /// The cycle's start frame, and the start of the one it fades from.
    start: f64,
    old_start: f64,
    /// Fade frames left, of `fade_len`.
    fade: usize,
    fade_len: usize,
    /// The next frame's place in the cycle, beats, and the beats per frame.
    beat: f64,
    beat_step: f64,
}

impl HalfTimeState {
    /// The ring is sized for 120 BPM until [`Self::prepare`].
    pub fn new(params: &[f32], sample_rate: f32) -> Self {
        let size = capacity(sample_rate, 120.0);
        let mut state = Self {
            sample_rate,
            tempo_bpm: 120.0,
            beats: 4.0,
            mix: 1.0,
            rate: 0.5,
            old_rate: 0.5,
            left: vec![0.0; size],
            right: vec![0.0; size],
            frame: 0,
            start: 0.0,
            old_start: 0.0,
            fade: 0,
            fade_len: 0,
            beat: 0.0,
            beat_step: 2.0 / f64::from(sample_rate),
        };
        state.refresh(params, sample_rate);
        state
    }

    pub fn refresh(&mut self, params: &[f32], sample_rate: f32) {
        if sample_rate != self.sample_rate {
            let tempo = self.tempo_bpm;
            *self = Self::new(params, sample_rate);
            self.prepare(tempo);
            return;
        }
        self.beats = LENGTHS[length_index(params[LENGTH])].0;
        self.mix = params[MIX];
        self.fade_len = (SMOOTH_SECONDS * params[SMOOTH].clamp(0.0, 1.0) * sample_rate) as usize;
        self.fade = self.fade.min(self.fade_len);
        let rate = SPEEDS[speed_index(params[SPEED])].0;
        if rate != self.rate {
            // The same cycle at the new speed, faded in from the old.
            (self.old_start, self.old_rate) = (self.start, self.rate);
            self.fade = self.fade_len;
            self.rate = rate;
        }
    }

    /// Sizes the ring for `tempo_bpm`, keeping what it holds. It allocates, so the host calls it off the audio thread.
    pub fn prepare(&mut self, tempo_bpm: f32) {
        self.tempo_bpm = tempo_bpm;
        let needed = capacity(self.sample_rate, tempo_bpm);
        if needed > self.left.len() {
            self.regrow(&mut vec![0.0; needed], &mut vec![0.0; needed]);
        }
    }

    /// Takes the ring of `built`, this module built again for an edit's engine and sized off the audio thread, when that
    /// ring is longer; `built` keeps the shorter one. Allocates nothing.
    pub fn take_ring(&mut self, built: &mut Self) {
        if built.left.len() > self.left.len() && built.sample_rate == self.sample_rate {
            self.regrow(&mut built.left, &mut built.right);
        }
    }

    /// Takes `left` and `right` (longer) as its ring, with the frames the old one holds, and leaves the old one in them.
    fn regrow(&mut self, left: &mut Vec<f32>, right: &mut Vec<f32>) {
        std::mem::swap(&mut self.left, left);
        std::mem::swap(&mut self.right, right);
        let (old, new) = (left.len() as u64 - 1, self.left.len() as u64 - 1);
        for n in self.frame.saturating_sub(old + 1)..self.frame {
            self.left[(n & new) as usize] = left[(n & old) as usize];
            self.right[(n & new) as usize] = right[(n & old) as usize];
        }
    }

    /// The block's tempo, and while the transport plays (`start_beat` ≥ 0) its place on the beat grid: a jump restarts
    /// the cycle there.
    pub fn prepare_block(&mut self, context: ProcessContext) {
        self.beat_step = f64::from(context.tempo_bpm) / (60.0 * f64::from(context.sample_rate));
        if context.start_beat < 0.0 {
            return;
        }
        let at = context.start_beat.rem_euclid(self.beats);
        let off = (at - self.beat).rem_euclid(self.beats);
        if off.min(self.beats - off) > 1.5 * self.beat_step {
            self.restart(at / self.beat_step);
        }
        self.beat = at;
    }

    /// A cycle that began `into` frames before the next one, faded in from the playback running on.
    fn restart(&mut self, into: f64) {
        (self.old_start, self.old_rate) = (self.start, self.rate);
        self.fade = self.fade_len;
        self.start = self.frame as f64 - into.min(self.span());
    }

    /// The most frames a cycle runs: its playback, the fade's too, stays inside the ring at any Speed.
    fn span(&self) -> f64 {
        (self.left.len() - 2) as f64 / (1.0 - SLOWEST) - self.fade_len as f64
    }

    pub fn process_frame(&mut self, l: f32, r: f32) -> (f32, f32) {
        let mask = self.left.len() as u64 - 1;
        let n = self.frame;
        self.left[(n & mask) as usize] = l;
        self.right[(n & mask) as usize] = r;
        if self.beat >= self.beats {
            // Past more than one cycle only after Length shrank.
            self.beat %= self.beats;
            self.restart(self.beat / self.beat_step);
        } else if n as f64 - self.start >= self.span() {
            self.beat = 0.0;
            self.restart(0.0);
        }
        let head = self.start + self.rate * (n as f64 - self.start);
        let mut wet = (read(&self.left, head), read(&self.right, head));
        if self.fade > 0 {
            let x = FRAC_PI_2 * (1.0 - self.fade as f32 / self.fade_len as f32);
            let old = self.old_start + self.old_rate * (n as f64 - self.old_start);
            let (new_gain, old_gain) = x.sin_cos();
            wet.0 = new_gain * wet.0 + old_gain * read(&self.left, old);
            wet.1 = new_gain * wet.1 + old_gain * read(&self.right, old);
            self.fade -= 1;
        }
        self.beat += self.beat_step;
        self.frame += 1;
        (l + self.mix * (wet.0 - l), r + self.mix * (wet.1 - r))
    }

    /// Frames before 0 read as silence ([`read`]), so the ring is not cleared: a reset costs nothing on the audio thread.
    pub fn reset(&mut self) {
        self.frame = 0;
        self.start = 0.0;
        self.old_start = 0.0;
        self.fade = 0;
        self.beat = 0.0;
    }
}

/// The lag of the longest cycle at the slowest Speed, at `tempo_bpm`, with the longest fade, at most [`MAX_SECONDS`],
/// as a power of two.
fn capacity(sample_rate: f32, tempo_bpm: f32) -> usize {
    let cycle = LONGEST as f32 * 60.0 / tempo_bpm + SMOOTH_SECONDS;
    let seconds = ((1.0 - SLOWEST as f32) * cycle).min(MAX_SECONDS);
    ((seconds * sample_rate).ceil() as usize + 2).next_power_of_two()
}

/// `ring` at frame `head`, linearly interpolated; before frame 0 it reads silence.
// ponytail: linear interpolation leaves faint images above half the rate; a short half-band filter if they show.
fn read(ring: &[f32], head: f64) -> f32 {
    let mask = ring.len() - 1;
    let whole = head.floor();
    let fraction = (head - whole) as f32;
    let at = |i: i64| if i < 0 { 0.0 } else { ring[i as usize & mask] };
    let (a, b) = (at(whole as i64), at(whole as i64 + 1));
    a + (b - a) * fraction
}
