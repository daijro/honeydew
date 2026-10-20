//! DW Sampler (ID 1): a key- and velocity-zoned sample player.
//! 128 voices, each with linear-interpolated playback, a linear amp envelope, a filter with its own envelope, a
//! frequency envelope and an LFO. The zones and their decoded samples arrive at construction on the app thread;
//! nothing here allocates after [`DwSampler::new`].

use std::f32::consts::{PI, TAU};
use std::sync::Arc;

use crate::ProcessContext;
use crate::biquad::flush;
use crate::dwp::{Midi, Zone};
use crate::instrument::Instrument;
use crate::sample::SampleData;
use crate::voice::{LFO_SYNC_BEATS, lfo_attack_increment, lfo_rate_hz};

/// The param count.
pub const PARAMS: usize = 34;
/// Param defaults in index order.
const DEFAULTS: [f32; PARAMS] = [
    1.0, 0.0, 0.1, 0.5, 0.5, 1.0, 1.0, 0.0, 0.0, 1.0, 0.0, 0.0, 1.0, 0.0, 0.0, 0.0, 0.0, 0.0, 1.0,
    1.0, 1.0, 0.5, 0.0, 1.0, 1.0, 1.0, 0.5, 0.0, 0.0, 0.0, 0.0, 0.5, 0.0, 0.0,
];
const ENABLE: usize = 0;
const ATTACK: usize = 1;
const RELEASE: usize = 2;
const PITCH: usize = 3;
const FINE: usize = 4;
const DECAY: usize = 5;
const SUSTAIN: usize = 6;
const SLIDE: usize = 7;
const REVERSE: usize = 8;
const LOOP: usize = 9;
const LEGATO: usize = 10;
const MONO: usize = 11;
const CUTOFF: usize = 12;
const RESONANCE: usize = 13;
const KBD_TRACK: usize = 14;
const VEL_TRACK: usize = 15;
const FILTER_TYPE: usize = 16;
/// Filter Attack, Decay, Sustain, Release, then Amount.
const FILTER_ENV: usize = 17;
/// Pitch Attack, Decay, Sustain, Release, then Amount.
const PITCH_ENV: usize = 22;
const LFO_SHAPE: usize = 27;
const LFO_RATE: usize = 28;
const LFO_MODE: usize = 29;
const LFO_SYNC: usize = 30;
const LFO_AMOUNT: usize = 31;
const LFO_DEST: usize = 32;
const LFO_ATTACK: usize = 33;
/// The channel param that carries Pitch Bend.
const PITCH_BEND: usize = 4;
/// The panel setting key of PB Range (SAMPLE `1.8`, `panels::cstm`): k/12 for k semitones, 1…12.
const PB_RANGE_SETTING: u32 = 11_800;

const VOICES: usize = 128;
/// LFO and filter coefficients are updated every 16 samples.
const TICK: u32 = 16;
/// LFO Level and Pan move this fraction of the way to their target per sample.
const SMOOTH: f32 = 0.01;
/// A forced kill (a mono retrigger) fades out in 1 ms.
const KILL_SECONDS: f32 = 0.001;
/// The cutoff stays below 0.49·Fs, where the bilinear prewarp is still finite.
const MAX_CUTOFF: f32 = 0.49;
/// A zone gain is linear and kept within 0..16, so a hostile file cannot blow up the mix.
const MAX_ZONE_GAIN: f32 = 16.0;

/// A zone with its decoded sample, as the engine hands it over.
#[derive(Clone, Debug)]
pub struct DwZone {
    pub zone: Zone,
    pub sample: Arc<SampleData>,
}

/// A zone ready to play: its effective loop, `[start, end)` in frames.
struct Playable {
    zone: Zone,
    sample: Arc<SampleData>,
    looped: Option<(f64, f64)>,
}

impl Playable {
    /// Start = end takes the file's `smpl` loop; the loop runs only with mode > 1 and start < end (end clipped to
    /// the sample).
    fn new(DwZone { zone, sample }: DwZone) -> Self {
        let (start, end) = if zone.loop_start == zone.loop_end {
            sample.meta.loop_frames.unwrap_or_default()
        } else {
            (zone.loop_start, zone.loop_end)
        };
        let end = f64::from(end).min(sample.frames() as f64);
        let looped =
            (zone.loop_mode > 1 && f64::from(start) < end).then_some((f64::from(start), end));
        Self {
            zone: Zone {
                gain: zone.gain.clamp(0.0, MAX_ZONE_GAIN),
                ..zone
            },
            sample,
            looped,
        }
    }
}

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
enum Stage {
    #[default]
    Off,
    Attack,
    Decay,
    Sustain,
    Release,
    Kill,
}

impl Stage {
    /// The key is still down.
    fn held(self) -> bool {
        matches!(self, Self::Attack | Self::Decay | Self::Sustain)
    }
}

/// The filter and frequency envelope: e += (target − e)·c per sample, flushed to 0 below 1e-30.
#[derive(Clone, Copy, Default)]
struct ExpEnv {
    e: f32,
    decaying: bool,
    released: bool,
}

/// One envelope's per-sample coefficients and sustain level.
struct ExpLaw {
    attack: f32,
    decay: f32,
    sustain: f32,
    release: f32,
}

impl ExpLaw {
    /// From the four params at `first` (Attack, Decay, Sustain, Release): attack c = 0.0002/(500x⁴), decay
    /// 0.0002/(50x⁴), capped at 1 (x = 0 is instant).
    // Release uses the decay law.
    fn new(p: &[f32; PARAMS], first: usize) -> Self {
        let c = |k: f32, x: f32| (0.0002 / (k * x.powi(4))).min(1.0);
        Self {
            attack: c(500.0, p[first]),
            decay: c(50.0, p[first + 1]),
            sustain: p[first + 2],
            release: c(50.0, p[first + 3]),
        }
    }
}

impl ExpEnv {
    /// Attack targets 4; past 1 it decays toward Sustain; release targets 0.
    // The attack ends at exactly 1, so an instant attack never overshoots to 4.
    fn advance(&mut self, law: &ExpLaw) -> f32 {
        let (target, c) = match (self.released, self.decaying) {
            (true, _) => (0.0, law.release),
            (false, true) => (law.sustain, law.decay),
            (false, false) => (4.0, law.attack),
        };
        self.e = flush(self.e + (target - self.e) * c);
        if !self.decaying && !self.released && self.e > 1.0 {
            self.e = 1.0;
            self.decaying = true;
        }
        self.e
    }
}

#[derive(Clone, Copy, Default, PartialEq, Eq)]
enum FilterKind {
    #[default]
    Off,
    Lowpass,
    Bandpass,
    Highpass,
}

/// Lowpass: a 4-pole ladder (zero-delay feedback, Zavalishin) with feedback k = 4·Res, the fed-back output
/// saturated by tanh. Bandpass and Highpass: a 2-pole state-variable filter (Simper), Q = 16·Res³ + 0.5, the band
/// output with constant skirt gain (peak Q). Standard topologies for the given slopes and laws.
/// States are flushed to 0 below 1e-30.
#[derive(Clone, Copy, Default)]
struct Filter {
    /// The kind the coefficients are for.
    kind: FilterKind,
    /// Lowpass: G = g/(1 + g), 1/(1 + g), k, 1/(1 + k·G⁴). State-variable: a1, a2, a3, k.
    c: [f32; 4],
    s: [[f32; 4]; 2],
}

impl Filter {
    /// New coefficients; a new kind also starts from silent states.
    // Switching the filter type mid-note restarts the filter rather than reusing another kind's states.
    fn tune(&mut self, kind: FilterKind, hz: f32, resonance: f32, sample_rate: f32) {
        if kind != self.kind {
            self.kind = kind;
            self.s = [[0.0; 4]; 2];
        }
        let g = (PI * hz.min(MAX_CUTOFF * sample_rate) / sample_rate).tan();
        self.c = if kind == FilterKind::Lowpass {
            let k = 4.0 * resonance;
            let big = g / (1.0 + g);
            [big, 1.0 / (1.0 + g), k, 1.0 / (1.0 + k * big.powi(4))]
        } else {
            let k = 1.0 / (16.0 * resonance.powi(3) + 0.5);
            let a1 = 1.0 / (1.0 + g * (g + k));
            [a1, g * a1, g * g * a1, k]
        };
    }

    fn process(&mut self, kind: FilterKind, channel: usize, x: f32) -> f32 {
        let [c0, c1, c2, c3] = self.c;
        let s = &mut self.s[channel];
        match kind {
            FilterKind::Off => x,
            FilterKind::Lowpass => {
                let (big, beta, k, norm) = (c0, c1, c2, c3);
                let tail = beta * (((big * s[0] + s[1]) * big + s[2]) * big + s[3]);
                let estimate = (big.powi(4) * x + tail) * norm;
                // At Res 0 the feedback is 0 either way: skip the tanh.
                let mut u = if k == 0.0 { x } else { x - k * estimate.tanh() };
                for stage in s.iter_mut() {
                    let v = (u - *stage) * big;
                    u = v + *stage;
                    *stage = flush(u + v);
                }
                u
            }
            FilterKind::Bandpass | FilterKind::Highpass => {
                let (a1, a2, a3, k) = (c0, c1, c2, c3);
                let v3 = x - s[1];
                let v1 = a1 * s[0] + a2 * v3;
                let v2 = s[1] + a2 * s[0] + a3 * v3;
                s[0] = flush(2.0 * v1 - s[0]);
                s[1] = flush(2.0 * v2 - s[1]);
                if kind == FilterKind::Bandpass {
                    v1
                } else {
                    x - k * v1 - v2
                }
            }
        }
    }
}

#[derive(Clone, Copy, Default)]
struct Lfo {
    phase: f32,
    fade: f32,
    /// The Random shape's held value.
    random: f32,
    /// The output l, updated every 16 samples.
    out: f32,
    /// The smoothed Level gain (1 + l) and Pan l.
    level: f32,
    pan: f32,
}

impl Lfo {
    /// One 16-sample tick: the output at the current phase, then the phase moves on at `hz` (One Shot holds at 1).
    /// `song` is Tmp Sync's phase from the song position, frac(beat/period), while the transport runs. Random draws a
    /// new value each cycle.
    fn tick(&mut self, block: &Block, hz: f32, song: Option<f32>, sample_rate: f32, rng: &mut u32) {
        self.fade = (self.fade + TICK as f32 * block.lfo_fade).min(1.0);
        if let Some(phase) = song {
            if phase < self.phase {
                self.random = unit(rng);
            }
            self.phase = phase;
        }
        let phase = self.phase;
        let shape = match block.lfo_shape {
            0 => (TAU * phase).sin(),
            1 => 1.0 - 2.0 * phase,
            2 if phase >= 0.5 => 1.0,
            2 => -1.0,
            _ => self.random,
        };
        self.out = shape * block.lfo_depth * self.fade;
        if song.is_none() {
            let next = phase + TICK as f32 * hz / sample_rate;
            self.phase = if block.lfo_mode == ONE_SHOT {
                next.min(1.0)
            } else if next >= 1.0 {
                self.random = unit(rng);
                next.fract()
            } else {
                next
            };
        }
    }
}

/// Xorshift32: deterministic, so renders repeat.
fn xorshift(state: &mut u32) -> u32 {
    *state ^= *state << 13;
    *state ^= *state >> 17;
    *state ^= *state << 5;
    *state
}

/// A value in [0, 1).
fn unit(state: &mut u32) -> f32 {
    (xorshift(state) >> 8) as f32 / (1 << 24) as f32
}

#[derive(Clone, Copy, Default)]
struct Voice {
    stage: Stage,
    level: f32,
    key: u8,
    zone: usize,
    pos: f64,
    reverse: bool,
    velocity: f32,
    /// The note pan p, and the left and right gains of clamp(p + slide pan, 0, 1).
    note_pan: f32,
    pan: (f32, f32),
    /// Slide offsets: semitones, pan, velocity; a launched voice starts at 0.
    slide: [f32; 3],
    /// Semitones still to glide, decaying to 0.
    glide: f32,
    filter_env: ExpEnv,
    pitch_env: ExpEnv,
    lfo: Lfo,
    tick: u32,
    filter: Filter,
}

impl Voice {
    fn release(&mut self) {
        if self.stage.held() {
            self.stage = Stage::Release;
            self.filter_env.released = true;
            self.pitch_env.released = true;
        }
    }
}

/// Everything a block reads from the params, worked out once per block.
struct Block {
    attack: f32,
    decay: f32,
    sustain: f32,
    release: f32,
    kill: f32,
    glide: f32,
    /// Pitch, Fine and bend as a ratio.
    pitch_ratio: f32,
    filter_env: ExpLaw,
    pitch_env: ExpLaw,
    /// Frequency envelope: octaves = this · e³.
    pitch_env_octaves: f32,
    filter: FilterKind,
    cutoff: f32,
    resonance: f32,
    kbd_track: f32,
    vel_track: f32,
    filter_env_octaves: f32,
    lfo_shape: usize,
    lfo_mode: usize,
    lfo_hz: f32,
    lfo_beats: f32,
    lfo_depth: f32,
    lfo_fade: f32,
    lfo_dest: usize,
    looping: bool,
}

/// A stepped selector over `n + 1` items: trunc(n·v) in `f32`.
fn step(v: f32, n: usize) -> usize {
    (v.clamp(0.0, 1.0) * n as f32) as usize
}

/// Enable (p0) is on at v ≥ 0.5.
fn on(v: f32) -> bool {
    v >= 0.5
}

/// Reverse, Loop, Legato and Mono (p8–p11) are on at any v ≠ 0.
fn set(v: f32) -> bool {
    v != 0.0
}

/// 1/12 and 1/6 as `f32`, as the ratio law uses them.
const TWELFTH: f32 = 1.0 / 12.0;
const SIXTH: f32 = 1.0 / 6.0;

/// The pitch ratio of Pitch, Fine and bend: exp2f(fma(fma(bend, PBrange, s), 1/12, fma(Fine, 1/6,
/// −1/12))), each step fused in `f32`. `bend` is 2v − 1 of the wheel; Fine is in octaves (±1/12).
fn pitch_ratio(pitch: f32, fine: f32, bend: f32, pb_range: u8) -> f32 {
    let octaves = bend.mul_add(f32::from(pb_range), pitch_shift(pitch) as f32);
    octaves
        .mul_add(TWELFTH, fine.mul_add(SIXTH, -TWELFTH))
        .exp2()
}

/// The velocity step zones match on: trunc(fma_f32(vel, 127, 0.5)).
fn velocity_step(velocity: f32) -> i32 {
    velocity.mul_add(127.0, 0.5).trunc() as i32
}

/// Pitch (p3) in semitones: trunc(48·v − 24).
/// One fused multiply-add in `f32`, so 9 of the 49 steps land just below their integer and play one
/// semitone toward 0: k = 16, 19, 22 give −7, −4, −1 and k = 25 … 40 give 0, 3 … 15. The `%d` text uses it too.
pub fn pitch_shift(v: f32) -> i32 {
    v.mul_add(48.0, -24.0).trunc() as i32
}

/// The amp envelope's full-scale time T = 0.001 + 10·v⁴ s.
fn amp_seconds(v: f32) -> f32 {
    10.0f32.mul_add(v.powi(4), 0.001)
}

/// The filter cutoff in Hz: 2^(4 + 10.4·clamp(Cutoff + VelTrk·vel + LFO, 0, 1) + 8·e·(2·Amt − 1) +
/// KbdTrk·((key + glide)/12 − 4)), at most 22 kHz.
fn cutoff_hz(block: &Block, velocity: f32, lfo: f32, env: f32, pitch: f32) -> f32 {
    let x = (block.cutoff + block.vel_track * velocity + lfo).clamp(0.0, 1.0);
    let octaves =
        4.0 + 10.4 * x + env * block.filter_env_octaves + block.kbd_track * (pitch / 12.0 - 4.0);
    octaves.exp2().min(22_000.0)
}

/// The DW Sampler: zones, voices and an LFO.
pub struct DwSampler {
    zones: Vec<Playable>,
    params: [f32; PARAMS],
    voices: Box<[Voice]>,
    /// Per key, the zone it last picked.
    picks: [usize; 128],
    /// Keys held, oldest first.
    held: [u8; 128],
    held_count: usize,
    /// The last note's key, where the next glide starts.
    last: Option<f32>,
    rng: u32,
    bend: f32,
    midi: Midi,
    sample_rate: f32,
}

impl DwSampler {
    /// A sampler at `sample_rate` over `zones` (a zone with an empty sample is left out), with the `CSTM` `MIDI`
    /// settings. App thread: it allocates the 128 voices.
    pub fn new(sample_rate: f32, zones: Vec<DwZone>, midi: Midi) -> Self {
        Self {
            zones: zones
                .into_iter()
                .filter(|z| z.sample.frames() > 0)
                .map(Playable::new)
                .collect(),
            params: DEFAULTS,
            voices: vec![Voice::default(); VOICES].into_boxed_slice(),
            picks: [usize::MAX; 128],
            held: [0; 128],
            held_count: 0,
            last: None,
            rng: 0x9E37_79B9,
            bend: 0.0,
            midi,
            sample_rate,
        }
    }

    fn unhold(&mut self, key: u8) {
        if let Some(at) = self.held[..self.held_count].iter().position(|&k| k == key) {
            self.held.copy_within(at + 1..self.held_count, at);
            self.held_count -= 1;
        }
    }

    fn hold(&mut self, key: u8) {
        self.unhold(key);
        self.held[self.held_count] = key;
        self.held_count += 1;
        self.last = Some(f32::from(key));
    }

    /// Zone `index` answers `key` (shifted by Pitch) at velocity step `velocity`.
    fn answers(&self, index: usize, key: i32, velocity: i32) -> bool {
        let z = &self.zones[index].zone;
        (i32::from(z.key_low)..=i32::from(z.key_high)).contains(&key)
            && (i32::from(z.vel_low)..=i32::from(z.vel_high)).contains(&velocity)
    }

    /// The zone for `key`: one of the matches at random, never the previous pick for that key.
    fn pick(&mut self, key: u8, velocity: f32) -> Option<usize> {
        let shifted = i32::from(key) + pitch_shift(self.params[PITCH]);
        let step = velocity_step(velocity);
        let matches = (0..self.zones.len())
            .filter(|&i| self.answers(i, shifted, step))
            .count();
        let previous = self.picks[usize::from(key)];
        let avoid =
            matches > 1 && previous < self.zones.len() && self.answers(previous, shifted, step);
        let candidates = matches - usize::from(avoid);
        if candidates == 0 {
            return None;
        }
        let nth = if candidates > 1 {
            xorshift(&mut self.rng) as usize % candidates
        } else {
            0
        };
        let zone = (0..self.zones.len())
            .filter(|&i| self.answers(i, shifted, step) && !(avoid && i == previous))
            .nth(nth)?;
        self.picks[usize::from(key)] = zone;
        Some(zone)
    }

    fn block(&self) -> Block {
        let p = &self.params;
        let rate = self.sample_rate;
        let per_sample = |seconds: f32| 1.0 / (seconds * rate);
        let instant = p[ATTACK] <= 0.0 && self.midi.attack_flag == 0;
        let slide = (p[SLIDE] / 2.0).powi(3);
        let bipolar = |v: f32| 2.0 * v - 1.0;
        let amount = bipolar(p[LFO_AMOUNT]);
        Block {
            attack: if instant {
                1.0
            } else {
                per_sample(amp_seconds(p[ATTACK]))
            },
            decay: per_sample(amp_seconds(p[DECAY])),
            sustain: p[SUSTAIN],
            release: per_sample(amp_seconds(p[RELEASE])),
            kill: per_sample(KILL_SECONDS),
            glide: if slide > 0.0 {
                (-1.0 / (slide * rate)).exp()
            } else {
                0.0
            },
            pitch_ratio: pitch_ratio(p[PITCH], p[FINE], self.bend, self.midi.pb_range),
            filter_env: ExpLaw::new(p, FILTER_ENV),
            pitch_env: ExpLaw::new(p, PITCH_ENV),
            pitch_env_octaves: 4.0 * bipolar(p[PITCH_ENV + 4]).powi(3),
            filter: [
                FilterKind::Off,
                FilterKind::Lowpass,
                FilterKind::Bandpass,
                FilterKind::Highpass,
            ][step(p[FILTER_TYPE], 3)],
            cutoff: p[CUTOFF],
            resonance: p[RESONANCE],
            kbd_track: p[KBD_TRACK],
            vel_track: p[VEL_TRACK],
            filter_env_octaves: 8.0 * bipolar(p[FILTER_ENV + 4]),
            // trunc(fma_f32(v, 3, 0.5)).
            lfo_shape: p[LFO_SHAPE].clamp(0.0, 1.0).mul_add(3.0, 0.5) as usize,
            // trunc(2v), the rule for every menu.
            lfo_mode: step(p[LFO_MODE], 2),
            lfo_hz: lfo_rate_hz(p[LFO_RATE]),
            // trunc(9v) in f64.
            lfo_beats: LFO_SYNC_BEATS[(f64::from(p[LFO_SYNC].clamp(0.0, 1.0)) * 9.0) as usize],
            lfo_depth: amount * amount.abs(),
            lfo_fade: lfo_attack_increment(p[LFO_ATTACK]),
            lfo_dest: step(p[LFO_DEST], 3),
            looping: set(p[LOOP]),
        }
    }
}

const ONE_SHOT: usize = 1;
const TMP_SYNC: usize = 2;
const LFO_FILTER: usize = 0;
const LFO_FREQUENCY: usize = 1;
const LFO_LEVEL: usize = 2;
const LFO_PAN: usize = 3;

/// The note pan law: L = min(1, 2 − 2p), R = min(1, 2p).
fn pan_gains(p: f32) -> (f32, f32) {
    ((2.0 - 2.0 * p).min(1.0), (2.0 * p).min(1.0))
}

impl Instrument for DwSampler {
    fn set_parameter(&mut self, index: usize, value: f32) {
        if index >= PARAMS || !value.is_finite() {
            return;
        }
        // Enable On → Off cuts every voice at once.
        if index == ENABLE && on(self.params[ENABLE]) && !on(value) {
            self.reset();
        }
        self.params[index] = value;
    }

    // A note that arrives while Enable is off is ignored.
    fn note_on(&mut self, key: u8, velocity: f32, pan: f32) {
        if !on(self.params[ENABLE]) || key > 127 {
            return;
        }
        let velocity = velocity.clamp(0.0, 1.0);
        let others_held = self.held[..self.held_count].iter().any(|&k| k != key);
        let legato = set(self.params[LEGATO]);
        // Legato off: every note glides from the last. Legato on: only while another key is held.
        let from = self
            .last
            .filter(|_| others_held || !legato)
            .unwrap_or(f32::from(key));
        let note_pan = pan.clamp(0.0, 1.0);
        let pan = pan_gains(note_pan);
        if set(self.params[MONO]) {
            if others_held && let Some(voice) = self.voices.iter_mut().find(|v| v.stage.held()) {
                // A legato hand-over keeps the voice's zone and position and glides to the key.
                voice.glide += f32::from(voice.key) - f32::from(key);
                voice.key = key;
                voice.velocity = velocity;
                voice.note_pan = note_pan;
                voice.pan = pan_gains((note_pan + voice.slide[1]).clamp(0.0, 1.0));
                self.hold(key);
                return;
            }
            for voice in self.voices.iter_mut().filter(|v| v.stage != Stage::Off) {
                voice.stage = Stage::Kill;
            }
        }
        let picked = self.pick(key, velocity);
        let free = self.voices.iter().position(|v| v.stage == Stage::Off);
        self.hold(key);
        let (Some(zone), Some(index)) = (picked, free) else {
            return;
        };
        let instant = self.params[ATTACK] <= 0.0 && self.midi.attack_flag == 0;
        let reverse = set(self.params[REVERSE]);
        let random = unit(&mut self.rng);
        self.voices[index] = Voice {
            stage: if instant { Stage::Decay } else { Stage::Attack },
            level: if instant { 1.0 } else { 0.0 },
            key,
            zone,
            pos: if reverse {
                (self.zones[zone].sample.frames() - 1) as f64
            } else {
                0.0
            },
            reverse,
            velocity,
            note_pan,
            pan,
            glide: from - f32::from(key),
            lfo: Lfo {
                random,
                level: 1.0,
                ..Lfo::default()
            },
            ..Voice::default()
        };
    }

    fn note_off(&mut self, key: u8) {
        self.unhold(key);
        if set(self.params[MONO]) {
            let top = self.held[..self.held_count].last().copied();
            if let Some(voice) = self
                .voices
                .iter_mut()
                .find(|v| v.stage.held() && v.key == key)
            {
                // Last-note priority: back to the newest key still held, legato; else release.
                match top {
                    Some(top) => {
                        voice.glide += f32::from(voice.key) - f32::from(top);
                        voice.key = top;
                    }
                    None => voice.release(),
                }
            }
        } else {
            // A key's note-off releases every voice playing it.
            self.voices
                .iter_mut()
                .filter(|v| v.key == key)
                .for_each(Voice::release);
        }
    }

    fn all_notes_off(&mut self) {
        self.held_count = 0;
        self.voices.iter_mut().for_each(Voice::release);
    }

    fn reset(&mut self) {
        self.held_count = 0;
        self.last = None;
        self.voices.fill(Voice::default());
    }

    fn process(&mut self, left: &mut [f32], right: &mut [f32], context: ProcessContext) {
        let block = self.block();
        let Self {
            zones,
            voices,
            rng,
            sample_rate,
            ..
        } = self;
        let rate = *sample_rate;
        let beats_per_frame = f64::from(context.tempo_bpm) / (60.0 * f64::from(rate));
        for voice in voices.iter_mut().filter(|v| v.stage != Stage::Off) {
            let zone = &zones[voice.zone];
            let sample = &zone.sample;
            let frames = sample.frames() as f64;
            let rate_ratio = f64::from(sample.sample_rate) / f64::from(rate);
            let root = f32::from(zone.zone.root);
            let decay_ramp = zone.zone.floats[1] > 0.0;
            let zone_gain = if block.looping { zone.zone.gain } else { 1.0 };
            let looped = zone.looped.filter(|_| block.looping && !voice.reverse);
            for (frame, (out_l, out_r)) in left.iter_mut().zip(right.iter_mut()).enumerate() {
                let filter_env = voice.filter_env.advance(&block.filter_env);
                let pitch_env = voice.pitch_env.advance(&block.pitch_env);
                voice.glide = flush(voice.glide * block.glide);
                let pitch = f32::from(voice.key) + voice.glide + voice.slide[0];
                if voice.tick == 0 {
                    let song = (block.lfo_mode == TMP_SYNC && context.playing).then(|| {
                        let beat = context.start_beat + frame as f64 * beats_per_frame;
                        (beat / f64::from(block.lfo_beats)).rem_euclid(1.0) as f32
                    });
                    let hz = if block.lfo_mode == TMP_SYNC {
                        context.tempo_bpm / 60.0 / block.lfo_beats
                    } else {
                        block.lfo_hz
                    };
                    voice.lfo.tick(&block, hz, song, rate, rng);
                }
                // Every 16 samples, and at once when the filter type changes.
                if block.filter != FilterKind::Off
                    && (voice.tick == 0 || voice.filter.kind != block.filter)
                {
                    let lfo = if block.lfo_dest == LFO_FILTER {
                        voice.lfo.out
                    } else {
                        0.0
                    };
                    let hz = cutoff_hz(&block, voice.velocity, lfo, filter_env, pitch);
                    voice.filter.tune(block.filter, hz, block.resonance, rate);
                }
                voice.tick = (voice.tick + 1) % TICK;

                let l = voice.lfo.out;
                let (level_target, pan_target) = match block.lfo_dest {
                    LFO_LEVEL => (1.0 + l, 0.0),
                    LFO_PAN => (1.0, l),
                    _ => (1.0, 0.0),
                };
                voice.lfo.level += (level_target - voice.lfo.level) * SMOOTH;
                voice.lfo.pan = flush(voice.lfo.pan + (pan_target - voice.lfo.pan) * SMOOTH);

                let (dry_l, dry_r) = read(sample, voice.pos, looped);
                let wet_l = voice.filter.process(block.filter, 0, dry_l);
                let wet_r = if sample.right.is_some() {
                    voice.filter.process(block.filter, 1, dry_r)
                } else {
                    wet_l
                };
                // Velocity to gain is linear, slide velocity added before the clamp.
                let velocity = (voice.velocity + voice.slide[2]).clamp(0.0, 1.0);
                let gain = voice.level * voice.level * velocity * zone_gain * voice.lfo.level;
                *out_l += wet_l * gain * voice.pan.0 * (1.0 + voice.lfo.pan).min(1.0);
                *out_r += wet_r * gain * voice.pan.1 * (1.0 - voice.lfo.pan).min(1.0);

                match voice.stage {
                    Stage::Attack => {
                        voice.level += block.attack;
                        if voice.level >= 1.0 {
                            voice.level = 1.0;
                            voice.stage = Stage::Decay;
                        }
                    }
                    Stage::Decay if decay_ramp => {
                        voice.level -= block.decay;
                        if voice.level <= block.sustain {
                            voice.level = block.sustain;
                            voice.stage = Stage::Sustain;
                        }
                    }
                    Stage::Decay | Stage::Sustain => {
                        voice.level = block.sustain;
                        voice.stage = Stage::Sustain;
                    }
                    Stage::Release | Stage::Kill => {
                        let fall = if voice.stage == Stage::Kill {
                            block.kill
                        } else {
                            block.release
                        };
                        voice.level -= fall;
                        if voice.level <= 0.0 {
                            voice.stage = Stage::Off;
                        }
                    }
                    Stage::Off => {}
                }

                let lfo_octaves = if block.lfo_dest == LFO_FREQUENCY {
                    l * l.abs()
                } else {
                    0.0
                };
                let octaves = (pitch - root) / 12.0
                    + block.pitch_env_octaves * pitch_env.powi(3)
                    + lfo_octaves;
                let step = rate_ratio * f64::from(octaves.exp2()) * f64::from(block.pitch_ratio);
                if voice.reverse {
                    voice.pos -= step;
                } else {
                    voice.pos += step;
                    if let Some((start, end)) = looped
                        && voice.pos >= end
                    {
                        voice.pos = start + (voice.pos - start) % (end - start);
                    }
                }
                // A voice ends where its sample does.
                if !(0.0..frames).contains(&voice.pos) {
                    voice.stage = Stage::Off;
                }
                if voice.stage == Stage::Off {
                    break;
                }
            }
        }
    }

    fn set_controller(&mut self, index: usize, value: f32) {
        if index == PITCH_BEND && value.is_finite() {
            self.bend = 2.0 * value.clamp(0.0, 1.0) - 1.0;
        }
    }

    /// Slides add to every voice's pitch, velocity and note pan.
    fn set_slide(&mut self, pitch: f32, pan: f32, velocity: f32) {
        let finite = |value: f32| if value.is_finite() { value } else { 0.0 };
        for voice in &mut self.voices {
            voice.slide = [finite(pitch), finite(pan), finite(velocity)];
            voice.pan = pan_gains((voice.note_pan + voice.slide[1]).clamp(0.0, 1.0));
        }
    }

    /// PB Range, as the panel's codec reads it: round(12v) within 1…12.
    fn set_setting(&mut self, key: u32, value: f32) {
        if key == PB_RANGE_SETTING && value.is_finite() {
            self.midi.pb_range = (value * 12.0).round().clamp(1.0, 12.0) as u8;
        }
    }
}

/// The sample at `pos` by linear interpolation; inside a loop the frame after the last is the loop start. Mono
/// returns the same value twice.
fn read(sample: &SampleData, pos: f64, looped: Option<(f64, f64)>) -> (f32, f32) {
    let i = pos as usize;
    let t = (pos - i as f64) as f32;
    let next = match looped {
        Some((start, end)) if (i + 1) as f64 >= end => start as usize,
        _ => i + 1,
    };
    let lerp = |x: &[f32]| {
        let a = x.get(i).copied().unwrap_or(0.0);
        a + t * (x.get(next).copied().unwrap_or(0.0) - a)
    };
    let left = lerp(&sample.left);
    (left, sample.right.as_deref().map_or(left, lerp))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn block(set: &[(usize, f32)]) -> Block {
        let mut dw = DwSampler::new(48_000.0, Vec::new(), Midi::default());
        for &(index, value) in set {
            dw.set_parameter(index, value);
        }
        dw.block()
    }

    #[test]
    fn the_cutoff_law_spans_16_hz_to_21_6_khz_and_tracks_key_48() {
        let open = block(&[(CUTOFF, 1.0)]);
        assert!((cutoff_hz(&open, 0.0, 0.0, 0.0, 48.0) - 2f32.powf(14.4)).abs() < 1.0);
        let shut = block(&[(CUTOFF, 0.0)]);
        assert_eq!(cutoff_hz(&shut, 0.0, 0.0, 0.0, 48.0), 16.0);
        // Velocity and LFO add on the 0..1 scale, clamped.
        assert_eq!(cutoff_hz(&shut, 1.0, -0.5, 0.0, 48.0), 16.0);
        let vel = block(&[(CUTOFF, 0.0), (VEL_TRACK, 0.5)]);
        assert!((cutoff_hz(&vel, 1.0, 0.0, 0.0, 48.0) - 2f32.powf(9.2)).abs() < 0.01);
        // Kbd Track 1: an octave per octave above key 48; Filter Env Amount 1: 8 octaves at e = 1.
        let kbd = block(&[(CUTOFF, 0.5), (KBD_TRACK, 1.0), (FILTER_ENV + 4, 1.0)]);
        assert!(
            (cutoff_hz(&kbd, 0.0, 0.0, 0.0, 60.0) / cutoff_hz(&kbd, 0.0, 0.0, 0.0, 48.0) - 2.0)
                .abs()
                < 1e-4
        );
        assert!(
            (cutoff_hz(&kbd, 0.0, 0.0, 0.5, 36.0) - 2f32.powf(4.0 + 5.2 + 4.0 - 1.0)).abs() < 0.1
        );
        assert_eq!(cutoff_hz(&kbd, 0.0, 0.0, 1.0, 127.0), 22_000.0);
    }

    #[test]
    fn amp_times_glide_and_the_envelope_coefficients() {
        assert_eq!(amp_seconds(0.0), 0.001);
        assert!((amp_seconds(1.0) - 10.001).abs() < 1e-6);
        // Slide v: t = (v/2)³ s.
        let glide = block(&[(SLIDE, 1.0)]).glide;
        assert!((glide - (-1.0f32 / (0.125 * 48_000.0)).exp()).abs() < 1e-7);
        assert_eq!(block(&[]).glide, 0.0);
        let law = ExpLaw::new(&DEFAULTS, FILTER_ENV);
        assert_eq!(
            (law.attack, law.decay, law.sustain),
            (1.0, 0.0002 / 50.0, 1.0)
        );
        let mut env = ExpEnv::default();
        assert_eq!(env.advance(&law), 1.0, "an instant attack stops at 1");
        assert!(env.decaying);
        // Pitch: trunc(48v − 24); Fine ±1 semitone.
        assert_eq!(
            (pitch_shift(0.0), pitch_shift(0.5), pitch_shift(1.0)),
            (-24, 0, 24)
        );
    }

    #[test]
    fn pitch_is_trunc_of_a_fused_48v_minus_24() {
        // trunc(fma_f32(v, 48, −24)); at v = f32(k/48) nine steps land just below their integer.
        let below = [
            (16, -7),
            (19, -4),
            (22, -1),
            (25, 0),
            (28, 3),
            (31, 6),
            (34, 9),
            (37, 12),
            (40, 15),
        ];
        for k in 0..=48 {
            let expected = below
                .iter()
                .find(|(at, _)| *at == k)
                .map_or(k - 24, |(_, s)| *s);
            assert_eq!(pitch_shift(k as f32 / 48.0), expected, "step {k}");
        }
        // Between steps it truncates toward zero.
        assert_eq!(pitch_shift(0.5 + 1.7 / 48.0), 1);
        assert_eq!(pitch_shift(0.5 - 1.7 / 48.0), -1);
    }

    #[test]
    fn the_pitch_ratio_and_the_velocity_step_follow_round_11() {
        // exp2f(fma(fma(bend, PBrange, s), 1/12, fma(Fine, 1/6, −1/12))), bend = 2v − 1 as stored.
        let mut dw = DwSampler::new(
            48_000.0,
            Vec::new(),
            Midi {
                pb_range: 7,
                attack_flag: 0,
            },
        );
        dw.set_parameter(PITCH, 30.0 / 48.0);
        dw.set_parameter(FINE, 0.3);
        dw.set_controller(4, 0.8);
        let twelfth = 1.0f32 / 12.0;
        assert_eq!(twelfth, 1.0 / 12.0);
        let s = (30.0f32 / 48.0).mul_add(48.0, -24.0).trunc();
        let bend = 2.0 * 0.8f32 - 1.0;
        let octaves = bend
            .mul_add(7.0, s)
            .mul_add(twelfth, 0.3f32.mul_add(1.0 / 6.0, -twelfth));
        assert_eq!(dw.block().pitch_ratio, octaves.exp2());
        // Fine 0.094, where an unfused Fine·1/6 − 1/12 gives other bits than the fused one.
        dw.set_parameter(FINE, 0.094);
        let octaves = bend
            .mul_add(7.0, s)
            .mul_add(twelfth, 0.094f32.mul_add(1.0 / 6.0, -twelfth));
        assert_eq!(dw.block().pitch_ratio, octaves.exp2());
        assert_eq!(block(&[(FINE, 1.0)]).pitch_ratio, (1.0f32 / 12.0).exp2());
        // trunc(fma_f32(vel, 127, 0.5)).
        assert_eq!(velocity_step(0.5), 64);
        let below = f32::from_bits(0.5f32.to_bits() - 1);
        assert_eq!(velocity_step(below), 63);
        assert_eq!((velocity_step(0.0), velocity_step(1.0)), (0, 127));
    }

    #[test]
    fn envelopes_glides_and_filters_settle_to_exactly_zero() {
        let mut p = DEFAULTS;
        p[FILTER_ENV + 1] = 0.1;
        p[FILTER_ENV + 2] = 0.0;
        p[FILTER_ENV + 3] = 0.1;
        let law = ExpLaw::new(&p, FILTER_ENV);
        for released in [true, false] {
            // Release targets 0; so does Decay to Sustain 0.
            let mut env = ExpEnv {
                e: 1.0,
                decaying: true,
                released,
            };
            for _ in 0..20_000 {
                assert!(!env.advance(&law).is_subnormal());
            }
            assert_eq!(env.e, 0.0);
        }
        for kind in [
            FilterKind::Lowpass,
            FilterKind::Bandpass,
            FilterKind::Highpass,
        ] {
            let mut filter = Filter::default();
            filter.tune(kind, 1000.0, 0.9, 48_000.0);
            filter.process(kind, 0, 1.0);
            for _ in 0..200_000 {
                assert!(!filter.process(kind, 0, 0.0).is_subnormal());
            }
            assert_eq!(filter.s, [[0.0; 4]; 2]);
        }
        // A glide from an octave below, and an LFO Pan that swung and then stopped.
        let held = Zone {
            loop_mode: 2,
            loop_start: 0,
            loop_end: 100,
            ..Zone::default()
        };
        let dc = SampleData {
            sample_rate: 48_000,
            left: vec![0.5; 100].into(),
            right: None,
            meta: Default::default(),
        };
        let zones = vec![DwZone {
            zone: held,
            sample: Arc::new(dc),
        }];
        let mut dw = DwSampler::new(48_000.0, zones, Midi::default());
        dw.set_parameter(SLIDE, 0.1);
        dw.note_on(36, 1.0, 0.5);
        dw.note_on(48, 1.0, 0.5);
        dw.voices[1].lfo.pan = 1.0;
        let (mut left, mut right) = (vec![0.0; 10_000], vec![0.0; 10_000]);
        dw.process(&mut left, &mut right, CONTEXT);
        assert_eq!((dw.voices[1].glide, dw.voices[1].lfo.pan), (0.0, 0.0));
    }

    const CONTEXT: ProcessContext = ProcessContext {
        sample_rate: 48_000.0,
        tempo_bpm: 120.0,
        start_beat: 0.0,
        playing: false,
    };

    #[test]
    fn switching_the_filter_on_mid_note_tunes_it_at_once() {
        let dc = SampleData {
            sample_rate: 48_000,
            left: vec![0.5; 4800].into(),
            right: None,
            meta: Default::default(),
        };
        let zones = vec![DwZone {
            zone: Zone::default(),
            sample: Arc::new(dc),
        }];
        let mut dw = DwSampler::new(48_000.0, zones, Midi::default());
        dw.note_on(48, 1.0, 0.5);
        let (mut left, mut right) = ([0.0; 5], [0.0; 5]);
        dw.process(&mut left, &mut right, CONTEXT);
        dw.set_parameter(FILTER_TYPE, 1.0 / 3.0);
        let (mut left, mut right) = ([0.0; 1], [0.0; 1]);
        dw.process(&mut left, &mut right, CONTEXT);
        assert!(left[0] > 0.1, "{}", left[0]);
    }

    /// The LFO output on its first tick at `phase`, with shape `shape` (0..=3) and amount `amount`.
    fn lfo_at(shape: usize, amount: f32, phase: f32) -> f32 {
        let block = block(&[(LFO_SHAPE, shape as f32 / 3.0), (LFO_AMOUNT, amount)]);
        let mut lfo = Lfo {
            phase,
            ..Lfo::default()
        };
        lfo.tick(&block, 0.0, None, 48_000.0, &mut 1);
        lfo.out
    }

    #[test]
    fn lfo_shapes_and_the_depth_law() {
        // Sine sin 2πφ, Saw 1 − 2φ, Square ±1 (+1 from φ = 0.5).
        assert!((lfo_at(0, 1.0, 0.25) - 1.0).abs() < 1e-6);
        assert!((lfo_at(0, 1.0, 0.75) + 1.0).abs() < 1e-6);
        assert_eq!(lfo_at(1, 1.0, 0.25), 0.5);
        assert_eq!(lfo_at(1, 1.0, 0.75), -0.5);
        assert_eq!((lfo_at(2, 1.0, 0.25), lfo_at(2, 1.0, 0.5)), (-1.0, 1.0));
        // Depth u|u|, u = 2·Amount − 1: ±0.25 at 0.75 and 0.25.
        assert_eq!(lfo_at(1, 0.75, 0.25), 0.125);
        assert_eq!(lfo_at(1, 0.25, 0.25), -0.125);
        assert_eq!(lfo_at(1, 0.5, 0.25), 0.0);
        // Random is a held value in [0, 1): unipolar, redrawn each cycle.
        let random = block(&[(LFO_SHAPE, 1.0), (LFO_AMOUNT, 1.0), (LFO_RATE, 1.0)]);
        let (mut lfo, mut rng) = (Lfo::default(), 0x1234_5678);
        let mut draws = Vec::new();
        for _ in 0..20_000 {
            lfo.tick(&random, 30.0, None, 48_000.0, &mut rng);
            draws.push(lfo.out);
        }
        assert!(draws.iter().all(|v| (0.0..1.0).contains(v)));
        let mean = draws.iter().sum::<f32>() / draws.len() as f32;
        assert!((mean - 0.5).abs() < 0.05, "{mean}");
        let changes = draws.windows(2).filter(|w| w[0] != w[1]).count();
        // 20 000 ticks of 16 samples at 30 Hz is 200 cycles.
        assert!((195..=200).contains(&changes), "{changes}");
    }

    #[test]
    fn lfo_fade_and_one_shot() {
        // The fade-in rises by 16·c per 16-sample tick.
        let c = lfo_attack_increment(0.5);
        let fading = block(&[(LFO_SHAPE, 2.0 / 3.0), (LFO_AMOUNT, 1.0), (LFO_ATTACK, 0.5)]);
        let mut lfo = Lfo::default();
        for _ in 0..10 {
            lfo.tick(&fading, 0.0, None, 48_000.0, &mut 1);
        }
        assert!(
            (lfo.fade - 160.0 * c).abs() < 1e-6 * lfo.fade,
            "{}",
            lfo.fade
        );
        assert!((lfo.out + lfo.fade).abs() < 1e-9);
        // One Shot runs once at its rate and holds φ = 1: saw −1.
        let one_shot = block(&[(LFO_SHAPE, 1.0 / 3.0), (LFO_AMOUNT, 1.0), (LFO_MODE, 0.5)]);
        let mut lfo = Lfo::default();
        for _ in 0..1000 {
            lfo.tick(&one_shot, 30.0, None, 48_000.0, &mut 1);
        }
        assert_eq!((lfo.phase, lfo.out), (1.0, -1.0));
    }

    #[test]
    fn the_lfo_and_the_filter_update_every_16_samples() {
        let dc = SampleData {
            sample_rate: 48_000,
            left: vec![0.5; 4800].into(),
            right: None,
            meta: Default::default(),
        };
        let zones = vec![DwZone {
            zone: Zone::default(),
            sample: Arc::new(dc),
        }];
        let mut dw = DwSampler::new(48_000.0, zones, Midi::default());
        for (index, value) in [
            (LFO_SHAPE, 1.0 / 3.0),
            (LFO_RATE, 1.0),
            (LFO_AMOUNT, 1.0),
            (FILTER_TYPE, 1.0 / 3.0),
        ] {
            dw.set_parameter(index, value);
        }
        dw.note_on(48, 1.0, 0.5);
        let (mut lfo, mut coefficients, mut changes) = (f32::NAN, [0.0; 4], Vec::new());
        for frame in 0..100 {
            dw.process(&mut [0.0], &mut [0.0], CONTEXT);
            let voice = &dw.voices[0];
            if voice.lfo.out != lfo || voice.filter.c != coefficients {
                changes.push(frame);
                (lfo, coefficients) = (voice.lfo.out, voice.filter.c);
            }
        }
        assert_eq!(changes, [0, 16, 32, 48, 64, 80, 96]);
    }

    #[test]
    fn the_envelope_laws() {
        let mut p = DEFAULTS;
        p[PITCH_ENV] = 0.5;
        p[PITCH_ENV + 3] = 0.5;
        let law = ExpLaw::new(&p, PITCH_ENV);
        assert_eq!(law.attack, 0.0002 / (500.0 * 0.0625));
        assert_eq!(law.release, 0.0002 / (50.0 * 0.0625));
        // Attack heads to 4, release to 0.
        let mut env = ExpEnv::default();
        assert_eq!(env.advance(&law), 4.0 * law.attack);
        let mut env = ExpEnv {
            e: 0.5,
            decaying: true,
            released: true,
        };
        assert_eq!(env.advance(&law), 0.5 - 0.5 * law.release);
        // Vel Track scales by velocity.
        let vel = block(&[(CUTOFF, 0.0), (VEL_TRACK, 0.5)]);
        assert!((cutoff_hz(&vel, 0.5, 0.0, 0.0, 48.0) - 2f32.powf(4.0 + 2.6)).abs() < 0.01);
    }

    #[test]
    fn the_attack_runs_to_1_and_a_new_filter_kind_starts_silent() {
        let law = ExpLaw {
            attack: 0.03,
            decay: 0.0,
            sustain: 0.5,
            release: 0.0,
        };
        let mut env = ExpEnv::default();
        for _ in 0..9 {
            env.advance(&law);
        }
        // 4·(1 − 0.97⁹) ≈ 0.959: still attacking.
        assert!((env.e - 0.959).abs() < 0.001 && !env.decaying, "{}", env.e);
        assert_eq!(env.advance(&law), 1.0);
        assert!(env.decaying);

        let mut filter = Filter::default();
        filter.tune(FilterKind::Lowpass, 1000.0, 0.5, 48_000.0);
        for _ in 0..10 {
            filter.process(FilterKind::Lowpass, 0, 1.0);
        }
        filter.tune(FilterKind::Lowpass, 2000.0, 0.5, 48_000.0);
        assert_ne!(filter.s, [[0.0; 4]; 2], "a retune keeps the states");
        filter.tune(FilterKind::Highpass, 2000.0, 0.5, 48_000.0);
        assert_eq!(filter.s, [[0.0; 4]; 2], "a new kind starts silent");
    }

    #[test]
    fn tmp_sync_random_redraws_once_per_cycle() {
        let random = block(&[(LFO_SHAPE, 1.0), (LFO_AMOUNT, 1.0), (LFO_MODE, 1.0)]);
        let (mut lfo, mut rng) = (Lfo::default(), 0x1234_5678);
        let mut outs = Vec::new();
        for phase in [0.1, 0.5, 0.9, 0.2, 0.6, 0.95, 0.3] {
            lfo.tick(&random, 0.0, Some(phase), 48_000.0, &mut rng);
            outs.push(lfo.out);
        }
        assert_eq!(outs[0], outs[1]);
        assert_eq!(outs[1], outs[2]);
        assert_ne!(outs[2], outs[3], "the wrap draws a new value");
        assert_eq!((outs[3], outs[4]), (outs[5], outs[5]));
        assert_ne!(outs[5], outs[6]);
    }

    #[test]
    fn stepped_selectors_follow_round_13() {
        // Filter type and LFO destination trunc(3v), LFO mode trunc(2v), shape trunc(fma(v, 3, 0.5)), sync trunc(9v) in
        // f64: just below each step, and at it.
        let at = |index: usize, v: f32| block(&[(index, v)]);
        let filter = |v: f32| at(FILTER_TYPE, v).filter;
        assert!(filter(0.33) == FilterKind::Off && filter(1.0 / 3.0) == FilterKind::Lowpass);
        assert!(filter(0.66) == FilterKind::Lowpass && filter(2.0 / 3.0) == FilterKind::Bandpass);
        assert!(filter(0.99) == FilterKind::Bandpass && filter(1.0) == FilterKind::Highpass);
        let dest = |v: f32| at(LFO_DEST, v).lfo_dest;
        assert_eq!(
            [0.33, 1.0 / 3.0, 0.66, 2.0 / 3.0, 0.99, 1.0].map(dest),
            [0, 1, 1, 2, 2, 3]
        );
        let mode = |v: f32| at(LFO_MODE, v).lfo_mode;
        assert_eq!([0.49, 0.5, 0.99, 1.0].map(mode), [0, 1, 1, 2]);
        let shape = |v: f32| at(LFO_SHAPE, v).lfo_shape;
        assert_eq!(
            [0.16, 0.17, 0.49, 0.5, 0.83, 0.84, 1.0].map(shape),
            [0, 1, 1, 2, 2, 3, 3]
        );
        let sync = |v: f32| at(LFO_SYNC, v).lfo_beats;
        assert_eq!(
            [0.11, 1.0 / 9.0, 0.99, 1.0].map(sync),
            [0.125, 0.25, 16.0, 32.0]
        );
        for (k, beats) in LFO_SYNC_BEATS.iter().enumerate() {
            assert_eq!(sync(k as f32 / 9.0), *beats, "sync step {k}");
        }
    }
}
