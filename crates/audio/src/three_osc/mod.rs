//! 3x Osc (ID 39), the core: three oscillators → filter → amp,
//! driven by a control tick. The envelopes, LFOs, Echo and Arp plug in through [`VoiceMod`] and [`Scheduler`];
//! until then [`FlatMod`] (Vol env = the gate, everything else 0) and [`PassThrough`] (keys straight to voices) stand
//! in. Everything after construction runs on the audio thread: no allocation, locks or I/O.

pub mod arp;
pub mod echo;
pub mod env;
pub mod filter;
pub mod lfo;
pub mod osc;
mod voice;

use std::sync::Arc;

use crate::ProcessContext;
use crate::instrument::Instrument;
use crate::sample::SampleData;
use osc::{Bank, Wave};
use voice::{Ctl, Voice, target};

// ---------------------------------------------------------------------------------------------------------------
// The seam between the core and its modulation and scheduling.

/// The instrument's param count and the normalized param array every component reads.
pub const PARAMS: usize = 149;
pub type Params = [f32; PARAMS];

/// Modulation targets in page order; they index `ModFrame::env` / `ModFrame::lfo`.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Target {
    Pan = 0,
    Vol = 1,
    Cut = 2,
    Res = 3,
    Pitch = 4,
}

/// What the instrument passes every control tick.
#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub struct Tick {
    /// Song tempo: `ProcessContext::tempo_bpm` of the block this tick falls in (120 when that is not positive).
    pub bpm: f32,
    /// D, the song's TDIV byte 0 (the subdivision, at least 1) in the tick and time laws.
    pub d: f32,
    /// Control ticks since construction or `reset` (the first tick is 0).
    pub count: u64,
    /// The song position of this tick in beats: `ProcessContext::start_beat` + the frames into the block (while
    /// playing).
    pub beat: f64,
    /// `ProcessContext::playing`. A false -> true edge is "playback start" for the Global LFO realign.
    pub playing: bool,
}

/// One voice's modulation for the current tick. Index with `Target as usize`.
#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub struct ModFrame {
    /// Envelope outputs, 0..=1. Vol: with Enable off, 1 while held and 0 once released. Other targets: 0 while their
    /// envelope is disabled. Pan has no envelope: always 0.
    pub env: [f32; 5],
    /// LFO outputs: shape · fade-in g, in −1..=1; 0 during the LFO Delay.
    pub lfo: [f32; 5],
    /// The Vol envelope has finished (released and below 2^−24, or cut). The instrument frees the voice once its amp
    /// ramp has also reached 0.
    pub done: bool,
}

/// Per-voice envelopes and LFOs. One value per voice slot plus one shared `Global`.
/// The voice applies the depths itself: the modulation returns raw env and LFO values only.
pub trait VoiceMod: Clone + Default + Send {
    /// Instrument-wide state, e.g. the Global LFO phases.
    type Global: Clone + Default + Send;
    /// Once per tick, before any voice's `tick`.
    fn tick_global(global: &mut Self::Global, params: &Params, tick: &Tick);
    /// A new voice starts (not a Mono or arp-Slide glide): envs restart at Delay, LFO phases at 0 (or Global).
    fn start(&mut self, params: &Params, global: &Self::Global);
    /// Note-off: every envelope enters Release from its current level.
    fn release(&mut self, params: &Params);
    /// Stolen by the polyphony limit: envelopes zeroed, `done` from the next tick on.
    fn cut(&mut self);
    /// Advances one tick (the voice's first tick included) and returns this tick's outputs.
    fn tick(&mut self, params: &Params, global: &Self::Global, tick: &Tick) -> ModFrame;
}

/// A note as the scheduler hands it to the voice manager.
#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub struct NoteSpec {
    pub key: u8,
    /// Linear velocity (0, 1] (echo: × (2·Feedback)^k).
    pub velocity: f32,
    /// Note pan 0..=1, 0.5 = centre (echo: + k(4·Pan − 2), Ping Pong).
    pub pan: f32,
    /// Extra semitones: the echo pitch k(24·Pitch − 12), Fat Mode steps.
    pub pitch: f32,
    /// Echo filter offsets x_k, added to the filter's c and r.
    pub cut: f32,
    pub res: f32,
}

/// What the scheduler emits. `tag` identifies a voice for `Off`/`Slide`; an `On` whose tag is still held releases
/// that voice first (the re-press rule). Pass-through uses tag = key.
#[derive(Clone, Copy, Debug, PartialEq)]
pub enum VoiceEvent {
    On {
        tag: u32,
        note: NoteSpec,
    },
    Off {
        tag: u32,
    },
    /// Arp Slide: voice `tag` glides pitch, velocity and pan to `note` over `ticks` ticks, no retrigger.
    Slide {
        tag: u32,
        note: NoteSpec,
        ticks: u32,
    },
}

/// Key-to-voice scheduling: Echo and Arp. Keys arrive at the tick after `Instrument::note_on/off`.
/// The instrument applies polyphony, Mono, Porta and Gate to what comes out. Read Mono (p120), Arp (p139) etc. from
/// `params` (Echo is off in Mono; arp notes never echo; Mono is ignored while Arp != Off — the instrument does that).
pub trait Scheduler: Default + Send {
    fn key_on(
        &mut self,
        key: u8,
        velocity: f32,
        pan: f32,
        params: &Params,
        tick: &Tick,
        emit: &mut dyn FnMut(VoiceEvent),
    );
    fn key_off(&mut self, key: u8, params: &Params, tick: &Tick, emit: &mut dyn FnMut(VoiceEvent));
    /// Once per tick, after that tick's keys: emits due echoes and arp steps.
    fn tick(&mut self, params: &Params, tick: &Tick, emit: &mut dyn FnMut(VoiceEvent));
    /// Seek: forget everything pending (no events).
    fn reset(&mut self);
}

/// The envelopes + LFOs.
pub type Mod = lfo::Modulation;
/// The Echo + Arp.
pub type Sched = arp::ArpEcho;

/// Placeholder modulation, the Enable-off law: the Vol env is 1 while held and 0 once released; everything else 0.
#[derive(Clone, Copy, Debug, Default)]
pub struct FlatMod {
    held: bool,
}

impl VoiceMod for FlatMod {
    type Global = ();

    fn tick_global(_: &mut (), _: &Params, _: &Tick) {}

    fn start(&mut self, _: &Params, _: &()) {
        self.held = true;
    }

    fn release(&mut self, _: &Params) {
        self.held = false;
    }

    fn cut(&mut self) {
        self.held = false;
    }

    fn tick(&mut self, _: &Params, _: &(), _: &Tick) -> ModFrame {
        let mut frame = ModFrame {
            done: !self.held,
            ..ModFrame::default()
        };
        frame.env[Target::Vol as usize] = if self.held { 1.0 } else { 0.0 };
        frame
    }
}

/// Placeholder scheduling: every key is one voice, tagged with its key.
#[derive(Clone, Copy, Debug, Default)]
pub struct PassThrough;

impl Scheduler for PassThrough {
    fn key_on(
        &mut self,
        key: u8,
        velocity: f32,
        pan: f32,
        _: &Params,
        _: &Tick,
        emit: &mut dyn FnMut(VoiceEvent),
    ) {
        let note = NoteSpec {
            key,
            velocity,
            pan,
            ..NoteSpec::default()
        };
        emit(VoiceEvent::On {
            tag: u32::from(key),
            note,
        });
    }

    fn key_off(&mut self, key: u8, _: &Params, _: &Tick, emit: &mut dyn FnMut(VoiceEvent)) {
        emit(VoiceEvent::Off {
            tag: u32::from(key),
        });
    }

    fn tick(&mut self, _: &Params, _: &Tick, _: &mut dyn FnMut(VoiceEvent)) {}

    fn reset(&mut self) {}
}

// ---------------------------------------------------------------------------------------------------------------
// Laws.

/// The default D (TDIV byte 0, the subdivision). The instrument uses the song's, passed to [`ThreeOsc::new`].
pub const TICK_D: f32 = 4.0;
/// The hard voice cap (Voices --- = unlimited).
pub const VOICE_CAP: usize = 256;
/// Notes waiting for the next tick, and Mono's held keys, are kept up to these counts; more are dropped.
const KEY_QUEUE: usize = 256;
const MONO_STACK: usize = 128;
/// The tempo used when the context's is not positive.
const FALLBACK_BPM: f32 = 120.0;

/// The control tick: round(fs·15·D/(96·BPM)) samples = D/384 beat (at least 1).
pub fn tick_samples(fs: f32, bpm: f32, d: f32) -> u32 {
    let bpm = effective_bpm(bpm);
    ((f64::from(fs) * 15.0 * f64::from(d) / (96.0 * f64::from(bpm))).round() as u32).max(1)
}

/// `bpm`, or 120 when it is not positive.
fn effective_bpm(bpm: f32) -> f32 {
    if bpm > 0.0 { bpm } else { FALLBACK_BPM }
}

/// round(1.8462(e^(0.0032186·x) − 1)), in 1/192 beat.
fn time_law(x: f32) -> u32 {
    (1.8462 * (0.003_218_6 * x).exp_m1()).round() as u32
}

/// Slide: t = round(1.8462(e^(0.0032186·trunc(1660v)) − 1)) (glide = t/192 beat).
pub fn slide_t(v: f32) -> u32 {
    time_law((1660.0 * v).trunc())
}

/// The Porta/Mono glide in ticks: t/192 beat = 2t/D ticks.
pub fn slide_ticks(v: f32, d: f32) -> f32 {
    2.0 * slide_t(v) as f32 / d
}

/// Gate: t = round(1.8462(e^(0.0032186·trunc(997v + 450)) − 1)).
pub fn gate_t(v: f32) -> u32 {
    time_law((997.0 * v + 450.0).trunc())
}

/// The Gate law in ticks: max(1 tick, t/192 beat). 2t/D ticks rounded half away from zero.
pub fn gate_ticks(v: f32, d: f32) -> u32 {
    ((2.0 * gate_t(v) as f32 / d).round() as u32).max(1)
}

/// A 3x Osc toggle's applied state: on at v > 0.5, so exactly 0.5 is off.
pub fn toggle(v: f32) -> bool {
    v > 0.5
}

/// Coarse (p13–p15) in semitones: round half away from zero of fma_f32(v, 48, −24), one rounding. The
/// knob's v = f32(k/48) gives k − 24 at every k.
pub fn coarse(v: f32) -> f32 {
    v.mul_add(48.0, -24.0).round()
}

/// Voices = round(99v); 0 = unlimited (the hard cap).
pub fn max_voices(v: f32) -> usize {
    match (99.0 * v.clamp(0.0, 1.0)).round() as usize {
        0 => VOICE_CAP,
        n => n,
    }
}

/// Param defaults, by index.
pub const DEFAULTS: Params = [
    1.0, 0.0, 0.0, 0.0, 0.0, 0.0, 0.0, 0.5, 0.5, 0.5, // 0..
    0.5, 0.5, 0.5, 0.5, 0.25, 0.0, 0.5, 0.5, 0.5, 0.5, // 10..
    0.5, 0.5, 0.5, 0.25, 0.0, 0.0, 0.0, 0.0, 0.0, 0.0, // 20..
    0.0, 0.0, 0.0, 0.0, 0.0, 0.0, 0.0, 0.0, 0.0, 0.0, // 30..
    0.0, 0.0, 0.0, 0.0, 0.0, 0.0, 0.0, 0.305, 0.305, 0.305, // 40..
    0.305, 0.305, 0.5, 0.5, 0.5, 0.5, 0.5, 0.5, 0.5, 0.5, // 50..
    0.5, 0.5, 0.0, 0.0, 0.0, 0.0, 0.0, 0.0, 0.0, 0.0, // 60..
    0.0, 0.0, 0.0, 0.0, 0.01, 0.01, 0.01, 0.01, 0.5, 0.5, // 70..
    0.5, 0.5, 0.0, 0.0, 0.0, 0.0, 0.01, 0.01, 0.01, 0.01, // 80..
    0.5, 0.5, 0.5, 0.5, 1.0, 1.0, 1.0, 1.0, 0.01, 0.01, // 90..
    0.01, 0.01, 0.105, 0.5, 0.5, 0.5, 1.0, 0.5, 0.5, 0.5, // 100..
    0.0, 1.0, 0.0, 0.5, 0.5, 0.5, 0.5, 0.0, 0.494, 0.0, // 110..
    0.0, 0.5, 0.5, 0.5, 0.5, 0.833, 0.5, 0.5, 0.5, 1.0, // 120..
    0.333, 0.0, 0.375, 0.5, 0.5, 0.5, 0.5, 0.0, 0.0, 0.0, // 130..
    0.576, 1.0, 0.167, 0.0, 0.0, 0.0111, 0.0, 0.0, 0.0, // 140..
];

// Param indices the instrument reads (the voice reads its own).
const MAX_POLYPHONY: usize = 117;
const PORTAMENTO_TIME: usize = 118;
const PORTAMENTO_MODE: usize = 119;
const MONOPHONIC_MODE: usize = 120;
const GATE: usize = 129;
const DELAY_FEEDBACK: usize = 131;
const ARPEGGIO_DIRECTION: usize = 139;
/// Channel param 4.
const PITCH_BEND: usize = 4;

// ---------------------------------------------------------------------------------------------------------------
// The instrument.

#[derive(Clone, Copy, Debug)]
struct Key {
    key: u8,
    velocity: f32,
    pan: f32,
    on: bool,
}

/// The voices and their management: polyphony, the cut rule, Mono, Porta and Gate.
struct Pool {
    voices: Vec<Voice<Mod>>,
    global: <Mod as VoiceMod>::Global,
    /// Mono's held keys, oldest first.
    held: Vec<(u32, NoteSpec)>,
    /// D (TDIV byte 0).
    d: f32,
    order: u64,
    /// The pitch, velocity and pan of the last note started or glided to (Porta glides all three).
    last: Option<[f32; 3]>,
    rng: u32,
}

impl Pool {
    fn apply(&mut self, event: VoiceEvent, p: &Params) {
        let arp_off = (p[ARPEGGIO_DIRECTION] * 5.0).round() == 0.0;
        let mono = toggle(p[MONOPHONIC_MODE]) && arp_off;
        match event {
            VoiceEvent::On { tag, note } if mono => self.mono_on(tag, note, p),
            VoiceEvent::On { tag, note } => {
                self.release(tag, p);
                self.start(tag, note, p);
            }
            VoiceEvent::Off { tag } if mono => self.mono_off(tag, p),
            VoiceEvent::Off { tag } => self.release(tag, p),
            // An arp Slide is not a trigger; the voice keeps its filter state and slews.
            VoiceEvent::Slide { tag, note, ticks } => {
                if let Some(v) = self.held_voice(Some(tag)) {
                    v.note = note;
                    v.glide.to(&note, ticks as f32);
                    self.last = Some(target(&note));
                }
            }
        }
    }

    /// A held voice: with `tag`, or any.
    fn held_voice(&mut self, tag: Option<u32>) -> Option<&mut Voice<Mod>> {
        (self.voices.iter_mut()).find(|v| v.active && v.held && tag.is_none_or(|t| v.tag == t))
    }

    fn release(&mut self, tag: u32, p: &Params) {
        for v in self.voices.iter_mut().filter(|v| v.active && v.tag == tag) {
            v.release(p);
        }
    }

    fn start(&mut self, tag: u32, note: NoteSpec, p: &Params) {
        let sounding = self.voices.iter().filter(|v| v.active && !v.cut).count();
        if sounding >= max_voices(p[MAX_POLYPHONY]) {
            self.cut_one();
        }
        let slot = (self.voices.iter().position(|v| !v.active))
            .or_else(|| self.voices.iter().position(|v| v.cut))
            .unwrap_or(0);
        // Porta: every new note glides from the previous one, unless Echo Feedback > 0.
        let porta = (toggle(p[PORTAMENTO_MODE]) && p[DELAY_FEEDBACK] == 0.0)
            .then_some(self.last)
            .flatten()
            .map(|from| (from, slide_ticks(p[PORTAMENTO_TIME], self.d)));
        let gate = if p[GATE] < 1.0 {
            gate_ticks(p[GATE], self.d)
        } else {
            0
        };
        self.order += 1;
        let rng = &mut self.rng;
        let mut rand = || {
            *rng ^= *rng << 13;
            *rng ^= *rng >> 17;
            *rng ^= *rng << 5;
            f64::from(*rng >> 8) / f64::from(1u32 << 24)
        };
        let order = self.order;
        self.voices[slot].start(tag, note, order, porta, gate, p, &self.global, &mut rand);
        self.last = Some(target(&note));
    }

    /// Over the limit: cut a released voice if any, else a held one; among candidates the most recently started.
    fn cut_one(&mut self) {
        let pick = |held: bool| {
            (self.voices.iter().enumerate())
                .filter(|(_, v)| v.active && !v.cut && v.held == held)
                .max_by_key(|(_, v)| v.order)
                .map(|(i, _)| i)
        };
        if let Some(i) = pick(false).or_else(|| pick(true)) {
            self.voices[i].cut();
        }
    }

    /// Mono: one voice; a new key glides pitch, velocity and pan over Slide time with no retrigger.
    fn mono_on(&mut self, tag: u32, note: NoteSpec, p: &Params) {
        self.held.retain(|(t, _)| *t != tag);
        if self.held.len() == MONO_STACK {
            self.held.remove(0);
        }
        self.held.push((tag, note));
        let gate = if p[GATE] < 1.0 {
            gate_ticks(p[GATE], self.d)
        } else {
            0
        };
        let glide = slide_ticks(p[PORTAMENTO_TIME], self.d);
        if let Some(v) = self.held_voice(None) {
            v.tag = tag;
            v.note = note;
            v.gate = gate;
            v.glide.to(&note, glide);
            self.last = Some(target(&note));
            return;
        }
        for v in self.voices.iter_mut().filter(|v| v.active && !v.cut) {
            v.cut();
        }
        self.start(tag, note, p);
    }

    /// Mono: releasing the sounding key glides back to the most recent held key.
    fn mono_off(&mut self, tag: u32, p: &Params) {
        self.held.retain(|(t, _)| *t != tag);
        let back = self.held.last().copied();
        let glide = slide_ticks(p[PORTAMENTO_TIME], self.d);
        let Some(v) = self.held_voice(Some(tag)) else {
            return;
        };
        match back {
            Some((tag, note)) => {
                v.tag = tag;
                v.note = note;
                v.glide.to(&note, glide);
                self.last = Some(target(&note));
            }
            None => v.release(p),
        }
    }
}

/// The 3x Osc instrument.
pub struct ThreeOsc {
    fs: f32,
    params: Params,
    /// Pitch Bend, semitones.
    wheel: f32,
    /// Samples left before the next control tick.
    until_tick: u32,
    ticks: u64,
    samples: u64,
    keys: Vec<Key>,
    /// The last slide set while keys were queued, and how many were: the voices those keys start take it, as their
    /// note-ons came before it (a key queued after it starts without it).
    slide: Option<(usize, [f32; 3])>,
    all_off: bool,
    pool: Pool,
    sched: Sched,
    bank: &'static Bank,
    custom: Option<Arc<Wave>>,
}

impl ThreeOsc {
    /// Builds the instrument (app thread) for a song whose TDIV byte 0 (the subdivision, D) is `subdivision`
    /// (0 counts as 1). The first instance in a process builds the shared shape tables.
    pub fn new(sample_rate: f32, subdivision: u8) -> Self {
        Self::with_custom_shape(sample_rate, subdivision, None)
    }

    /// With the Custom shape's WAV (CSTM `PATH`), built into its table now ([`Wave::custom`]); with `None` or an empty
    /// file Custom is silent (the fresh table is all zeros).
    pub fn with_custom_shape(
        sample_rate: f32,
        subdivision: u8,
        custom: Option<&SampleData>,
    ) -> Self {
        let wave = custom.and_then(Wave::custom).map(Arc::new);
        Self::with_custom_wave(sample_rate, subdivision, wave)
    }

    /// With the Custom shape's table, built once ([`Wave::custom`]) and shared (the sample cache keeps it per file);
    /// with `None` Custom is silent.
    pub fn with_custom_wave(sample_rate: f32, subdivision: u8, custom: Option<Arc<Wave>>) -> Self {
        Self {
            fs: sample_rate,
            params: DEFAULTS,
            wheel: 0.0,
            until_tick: 0,
            ticks: 0,
            samples: 0,
            keys: Vec::with_capacity(KEY_QUEUE),
            slide: None,
            all_off: false,
            pool: Pool {
                voices: vec![Voice::default(); VOICE_CAP],
                global: Default::default(),
                held: Vec::with_capacity(MONO_STACK),
                d: f32::from(subdivision.max(1)),
                order: 0,
                last: None,
                rng: 0x9e37_79b9,
            },
            sched: Sched::default(),
            bank: Bank::get(),
            custom,
        }
    }

    /// Voices sounding or ramping down.
    pub fn active_voices(&self) -> usize {
        self.pool.voices.iter().filter(|v| v.active).count()
    }

    /// Queues a key for the next tick. Both keys of a note shorter than a tick gap are queued: it sounds (Attack 0
    /// starts at 1) and plays its release. Only with the queue full does an off cancel its key's queued on instead,
    /// so a long freeze (Enable off) cannot drop an off and leave a stuck note.
    fn queue(&mut self, key: Key) {
        if self.keys.len() < KEY_QUEUE {
            self.keys.push(key);
        } else if !key.on
            && let Some(i) = self.keys.iter().rposition(|k| k.on && k.key == key.key)
        {
            self.keys.remove(i);
            if let Some((count, _)) = &mut self.slide
                && i < *count
            {
                *count -= 1;
            }
        }
    }

    fn control_tick(&mut self, tick: &Tick, n: u32) {
        let Self {
            params,
            pool,
            sched,
            keys,
            slide,
            ..
        } = self;
        // Gate: the note releases after its ticks.
        for v in pool.voices.iter_mut().filter(|v| v.active && v.gate > 0) {
            v.gate -= 1;
            if v.gate == 0 {
                v.release(params);
            }
        }
        if std::mem::take(&mut self.all_off) {
            sched.reset();
            pool.held.clear();
            for v in pool.voices.iter_mut().filter(|v| v.active) {
                v.release(params);
            }
        }
        let slide = slide.take();
        for (i, key) in keys.drain(..).enumerate() {
            let emit = &mut |event| pool.apply(event, params);
            if key.on {
                sched.key_on(key.key, key.velocity, key.pan, params, tick, emit);
            } else {
                sched.key_off(key.key, params, tick, emit);
            }
            if let Some((count, value)) = slide
                && i + 1 == count
            {
                for v in &mut pool.voices {
                    v.slide = value;
                }
            }
        }
        sched.tick(params, tick, &mut |event| pool.apply(event, params));
        Mod::tick_global(&mut pool.global, params, tick);
        let ctl = Ctl {
            p: params,
            fs: self.fs,
            wheel: self.wheel,
            n: n as f32,
            samples: self.samples,
        };
        for v in pool.voices.iter_mut().filter(|v| v.active) {
            let frame = v.m.tick(params, &pool.global, tick);
            if v.control(&ctl, &frame) {
                v.active = false;
            }
        }
        self.ticks += 1;
    }
}

impl Instrument for ThreeOsc {
    fn set_parameter(&mut self, index: usize, value: f32) {
        if let Some(p) = self.params.get_mut(index) {
            *p = value;
        }
    }

    fn note_on(&mut self, key: u8, velocity: f32, pan: f32) {
        self.queue(Key {
            key,
            velocity,
            pan,
            on: true,
        });
    }

    fn note_off(&mut self, key: u8) {
        self.queue(Key {
            key,
            velocity: 0.0,
            pan: 0.5,
            on: false,
        });
    }

    fn all_notes_off(&mut self) {
        self.keys.clear();
        self.slide = None;
        self.all_off = true;
    }

    fn reset(&mut self) {
        for v in &mut self.pool.voices {
            v.active = false;
        }
        self.pool.held.clear();
        self.pool.global = Default::default();
        self.pool.last = None;
        self.sched.reset();
        self.keys.clear();
        self.slide = None;
        self.all_off = false;
        self.until_tick = 0;
        self.ticks = 0;
        self.samples = 0;
    }

    fn process(&mut self, left: &mut [f32], right: &mut [f32], context: ProcessContext) {
        let n = tick_samples(self.fs, context.tempo_bpm, self.pool.d);
        let beats_per_frame = f64::from(context.tempo_bpm) / (60.0 * f64::from(self.fs));
        let mut pos = 0;
        while pos < left.len() {
            if self.until_tick == 0 {
                let into = if context.playing {
                    pos as f64 * beats_per_frame
                } else {
                    0.0
                };
                let tick = Tick {
                    bpm: effective_bpm(context.tempo_bpm),
                    d: self.pool.d,
                    count: self.ticks,
                    beat: context.start_beat + into,
                    playing: context.playing,
                };
                self.control_tick(&tick, n);
                self.until_tick = n;
            }
            let len = (self.until_tick as usize).min(left.len() - pos);
            let custom = self.custom.as_deref();
            for v in self.pool.voices.iter_mut().filter(|v| v.active) {
                let waves = v.shape.map(|s| self.bank.wave(s, custom));
                v.render(waves, &mut left[pos..pos + len], &mut right[pos..pos + len]);
            }
            pos += len;
            self.until_tick -= len as u32;
            self.samples += len as u64;
        }
    }

    fn set_controller(&mut self, index: usize, value: f32) {
        if index == PITCH_BEND {
            // w = 4v − 2 semitones.
            self.wheel = 4.0 * value - 2.0;
        }
    }

    /// Slides add to every voice's pitch, velocity and note pan from its next control tick, also to the voices
    /// of the keys queued for that tick.
    fn set_slide(&mut self, pitch: f32, pan: f32, velocity: f32) {
        let finite = |value: f32| if value.is_finite() { value } else { 0.0 };
        let value = [finite(pitch), finite(pan), finite(velocity)];
        for voice in &mut self.pool.voices {
            voice.slide = value;
        }
        self.slide = (!self.keys.is_empty()).then_some((self.keys.len(), value));
    }

    /// Enable off freezes the voices; On resumes them.
    fn freezes_when_disabled(&self) -> bool {
        true
    }
}
