//! SuperSaw (type 23): a unison saw / hard-sync synth with a 4-pole ladder, three envelopes,
//! an LFO, a mod wheel, an arpeggiator and distortion + delay. Each law is named after its param or term; `vN` is
//! param N.
//!
//! Rendering runs in control ticks of [`TICK`] frames: the LFO, glide, unison setup, filter coefficients, filter and
//! sync EGs and the level/pan targets update once per tick; oscillators, the filter, the amp EG and gain smoothing
//! run per frame.

use std::f64::consts::TAU;
use std::sync::OnceLock;

use crate::ProcessContext;
use crate::instrument::Instrument;
use crate::voice::{
    ARP_RATE_BEATS, SUPERSAW_DELAY_BEATS, SUPERSAW_LFO_BEATS, lfo_rate_hz, tempo_step,
};

/// The module's param count.
pub const PARAM_COUNT: usize = 60;

/// Every param's default, in index order.
pub const DEFAULTS: [f32; PARAM_COUNT] = [
    1.0, 1.0, 0.0, 0.0, 0.0, 0.2, 0.0, 0.5, 0.0, 0.0, // 0–9
    0.0, 0.0, 1.0, 0.0, 1.0, 1.0, 1.0, 0.5, 0.0, 1.0, // 10–19
    0.0, 0.0, 0.0, 1.0, 1.0, 1.0, 0.5, 0.0, 0.0, 1.0, // 20–29
    1.0, 0.0, 1.0, 0.5, 0.0, 0.0, 0.5, 0.1, 1.0, 0.0, // 30–39
    0.0, 0.0, 0.0, 0.0, 0.5, 0.0, 0.0, 0.2, 0.5, 0.0, // 40–49
    0.0, 0.5, 0.0, 1.0, 0.775, 0.4, 0.0, 0.5, 0.5, 0.0, // 50–59
];

// Param indices. Hidden p43 is stored but never read.
const PULSE_WIDTH: usize = 1;
const PULSE_MODE: usize = 2;
const SUB_LEVEL: usize = 3;
const UNISON: usize = 4;
const DETUNE: usize = 5;
const UNISON_PAN: usize = 6;
const DETUNE_CURVE: usize = 7;
const RETRIG: usize = 8;
const UNISON_PHASE: usize = 9;
const SUPERSYNC: usize = 10;
const SYNC: usize = 11;
const SYNC_PULSE_WIDTH: usize = 12;
/// Sync EG Attack, Decay, Sustain, Release.
const SYNC_EG: usize = 13;
const SYNC_EG_AMOUNT: usize = 17;
const FLIP: usize = 18;
const CUTOFF: usize = 19;
const RESONANCE: usize = 20;
const KBD_TRK: usize = 21;
/// Filter EG Attack, Decay, Sustain, Release.
const FILTER_EG: usize = 22;
const FILTER_EG_AMOUNT: usize = 26;
const VEL_TRK: usize = 27;
/// Level EG Attack, Decay, Sustain, Release.
const LEVEL_EG: usize = 28;
const LEVEL: usize = 32;
const PAN: usize = 33;
const LEGATO: usize = 34;
const SLIDE: usize = 35;
const TRANSPOSE: usize = 36;
const BEND_RANGE: usize = 37;
const CONSTANT: usize = 38;
const LFO_SHAPE: usize = 39;
const LFO_RATE: usize = 40;
const LFO_MODE: usize = 41;
const LFO_BEAT: usize = 42;
const LFO_AMOUNT: usize = 44;
const LFO_DEST: usize = 45;
const LFO_FADE: usize = 46;
const ARP_RATE: usize = 47;
const ARP_RANGE: usize = 48;
const ARP_MODE: usize = 49;
const MONO: usize = 50;
const VOICES: usize = 51;
const DISTORTION: usize = 52;
const GAIN: usize = 53;
const DELAY_TIME: usize = 54;
const DELAY_FEEDBACK: usize = 55;
const DELAY_MIX: usize = 56;
const DELAY_FILTER: usize = 57;
const MW_AMOUNT: usize = 58;
const MW_DEST: usize = 59;

// Modulation targets: the LFO's eight destinations in its menu order (round(7·v45)), then the two only the mod wheel
// reaches.
const M_CUTOFF: usize = 0;
const M_RESONANCE: usize = 1;
const M_FREQUENCY: usize = 2;
const M_SYNC: usize = 3;
const M_PULSE_WIDTH: usize = 4;
const M_DETUNE: usize = 5;
const M_LEVEL: usize = 6;
const M_PAN: usize = 7;
const M_LFO_AMOUNT: usize = 8;
const M_LFO_RATE: usize = 9;
const MODS: usize = 10;
/// The mod wheel's destination menu (p59, int(8·v59)) as targets.
const WHEEL_TARGETS: [usize; 9] = [
    M_CUTOFF,
    M_RESONANCE,
    M_SYNC,
    M_PULSE_WIDTH,
    M_DETUNE,
    M_LEVEL,
    M_PAN,
    M_LFO_AMOUNT,
    M_LFO_RATE,
];

/// The channel params read: Pitch Bend and MIDI CC #1, the mod wheel.
const PITCH_BEND: usize = 4;
const MOD_WHEEL: usize = 7 + 1;

/// Frames per control tick.
const TICK: usize = 24;
const MAX_VOICES: usize = 8;
const MAX_UNISON: usize = 12;
/// SUPERSYNC: R = 1 + 5S ≤ 6, so at most six saw pairs.
const MAX_SYNC_PAIRS: usize = 6;

/// The global wavetable set: table m (1-based) is Σ_{h=1..m} sin(2πhn/2048)/h, a falling saw.
const TABLES: usize = 1024;
const TABLE_LEN: usize = 2048;
/// The lookup reads the top 11 bits of the 32-bit phase.
const INDEX_SHIFT: u32 = 32 - TABLE_LEN.trailing_zeros();
/// A fresh 22-bit dither per frame.
const DITHER_SHIFT: u32 = 32 - 22;
/// Increment = f/sr·0.99951·2³² (≈ 0.85 cent flat).
const INCREMENT_FACTOR: f64 = 0.99951;
const PHASE_SCALE: f64 = 4_294_967_296.0;

/// Unison detune curve exponent: e = 1 + c·4 (c ≥ 0) or 1 + c·0.9 (c < 0).
const CURVE_UP: f32 = 4.0;
const CURVE_DOWN: f32 = 0.9;
/// Unison pan and the sub are off below this.
const UNISON_PAN_MIN: f32 = 0.001;
const SUB_MIN: f32 = 0.001;
/// Retrig: phase_k = k·0.99·v9² cycles.
const PHASE_SPREAD: f32 = 0.99;
/// SUPERSYNC slave ratio R = 1 + 5S.
const SYNC_RANGE: f32 = 5.0;

/// The ladder runs 4096 = 2^12 forward-Euler sub-steps per frame.
const LADDER_SUBSTEPS_LOG2: u32 = 12;
const LADDER_SUBSTEPS: f64 = (1 << LADDER_SUBSTEPS_LOG2) as f64;
/// ω = 6.25·fc.
const LADDER_OMEGA: f64 = 6.25;
/// After each frame y1 ← y1/(1 + 10⁻⁵·y1²).
const Y1_SOFT_CLIP: f32 = 1e-5;
/// fc = min(2^(4 + 10.3·c′ + 8·E_f·(2v26 − 1) + (n/12 − 4)·v21), 20000).
const FC_BASE_OCT: f32 = 4.0;
const FC_RANGE_OCT: f32 = 10.3;
const FC_EG_OCT: f32 = 8.0;
const FC_KEY_PIVOT_OCT: f32 = 4.0;
const FC_MAX: f32 = 20_000.0;

/// Retriggering a sounding voice attacks for at least T(0.01) ≈ 2.7 ms.
const RE_ATTACK_MIN_X: f32 = 0.01;
/// Glide time s = (v35/2)²; off at or below 1 ms.
const GLIDE_OFF_S: f32 = 0.001;
/// Output gain smoothing per frame.
const GAIN_SMOOTHING: f32 = 0.01;
/// Arp notes play at centre pan.
const ARP_PAN: f32 = 0.5;

/// Distortion is on above v52 = 0.005.
const DISTORTION_MIN: f32 = 0.005;
const DELAY_MAX_S: f32 = 2.0;
/// fb = 0.99·v55.
const DELAY_FEEDBACK_MAX: f32 = 0.99;

/// LFO modes, int(2·v41).
const HERTZ: usize = 0;
const ONE_SHOT: usize = 1;

/// The SuperSaw instrument. Build it on the app thread with [`SuperSaw::new`]; the [`Instrument`] methods never
/// allocate.
pub struct SuperSaw {
    sample_rate: f32,
    params: [f32; PARAM_COUNT],
    tables: &'static [f32],
    voices: [Voice; MAX_VOICES],
    control: Control,
    serial: u64,
    /// The velocity of each held key.
    held: [Option<f32>; 128],
    held_count: usize,
    /// The previously played note: the glide source and the Flip reference.
    last_note: Option<u8>,
    /// Frames into the current control tick.
    tick_pos: usize,
    /// Channel Pitch Bend and mod wheel (CC #1), normalized.
    bend: f32,
    wheel: f32,
    tempo_bpm: f32,
    /// The song position; it follows the transport while it plays and runs free otherwise.
    song_beat: f64,
    /// The free-running global Hertz-LFO phase a new voice inherits.
    lfo_phase: f64,
    arp: Arp,
    delay: [Vec<f32>; 2],
    delay_pos: usize,
    delay_filter: [f32; 2],
    rng: Rng,
}

impl SuperSaw {
    /// A SuperSaw at `sample_rate` with every param at its default. Builds the shared wavetable set on first use.
    pub fn new(sample_rate: f32) -> Self {
        let sample_rate = if sample_rate.is_finite() && sample_rate > 0.0 {
            sample_rate
        } else {
            44_100.0
        };
        let line = (DELAY_MAX_S * sample_rate) as usize + 1;
        let mut synth = Self {
            sample_rate,
            params: DEFAULTS,
            tables: wavetables(),
            voices: std::array::from_fn(Voice::new),
            control: Control::default(),
            serial: 0,
            held: [None; 128],
            held_count: 0,
            last_note: None,
            tick_pos: 0,
            bend: 0.5,
            wheel: 0.0,
            tempo_bpm: 120.0,
            song_beat: 0.0,
            lfo_phase: 0.0,
            arp: Arp::default(),
            delay: [vec![0.0; line], vec![0.0; line]],
            delay_pos: 0,
            delay_filter: [0.0; 2],
            rng: Rng::new(0),
        };
        synth.update_control();
        synth
    }

    /// Retrig, Legato and Mono Mode are on at any value ≠ 0.
    fn on(&self, index: usize) -> bool {
        self.params[index] != 0.0
    }

    fn arp_mode(&self) -> usize {
        ((5.0 * self.params[ARP_MODE]) as usize).min(5)
    }

    /// The tick's global state from the params, the controllers, the tempo and the song position.
    fn update_control(&mut self) {
        let p = &self.params;
        let rate = self.sample_rate;
        let tick_rate = rate / TICK as f32;
        let mut wheel = [0.0; MODS];
        wheel[WHEEL_TARGETS[((8.0 * p[MW_DEST]) as usize).min(8)]] =
            (2.0 * p[MW_AMOUNT] - 1.0) * self.wheel;
        let amount = 2.0 * (p[LFO_AMOUNT] + wheel[M_LFO_AMOUNT]).clamp(0.0, 1.0) - 1.0;
        let lfo_hz = lfo_rate_hz(p[LFO_RATE] + wheel[M_LFO_RATE]);
        // LFO sync trunc(9v) in f64, PB range trunc(fma_f32(v, 11, 1)).
        let lfo_beats = SUPERSAW_LFO_BEATS[((f64::from(p[LFO_BEAT]) * 9.0) as usize).min(9)];
        let w = 2.0 * self.bend - 1.0;
        let bend_range = p[BEND_RANGE].mul_add(11.0, 1.0).trunc();
        let drive = if p[DISTORTION] > DISTORTION_MIN {
            10f32.powf(2.0 * p[DISTORTION]) - 0.99
        } else {
            0.0
        };
        let delay_s = (tempo_step(&SUPERSAW_DELAY_BEATS, p[DELAY_TIME]) * 60.0 / self.tempo_bpm)
            .min(DELAY_MAX_S);
        let amp_attack = 0.9 * p[LEVEL_EG];
        self.control = Control {
            wheel,
            amp: Adsr::new(
                amp_attack,
                p[LEVEL_EG + 1],
                p[LEVEL_EG + 2],
                0.85 * p[LEVEL_EG + 3] + 0.05,
                rate,
            ),
            amp_re_attack: swing_step(envelope_time(amp_attack.max(RE_ATTACK_MIN_X)), rate),
            filter: Adsr::new(
                p[FILTER_EG],
                p[FILTER_EG + 1],
                p[FILTER_EG + 2],
                p[FILTER_EG + 3],
                tick_rate,
            ),
            sync: Adsr::new(
                p[SYNC_EG],
                p[SYNC_EG + 1],
                p[SYNC_EG + 2],
                p[SYNC_EG + 3],
                tick_rate,
            ),
            // Shape and Dest round as trunc(fma_f32(v, n, 0.5)).
            lfo_shape: (p[LFO_SHAPE].mul_add(3.0, 0.5) as usize).min(3),
            lfo_mode: ((2.0 * p[LFO_MODE]) as usize).min(2),
            lfo_dest: (p[LFO_DEST].mul_add(7.0, 0.5) as usize).min(7),
            lfo_amount: amount * amount.abs(),
            lfo_step: f64::from(lfo_hz) * TICK as f64 / f64::from(rate),
            lfo_sync_phase: (self.song_beat / f64::from(lfo_beats)).rem_euclid(1.0),
            lfo_fade_step: swing_step(envelope_time(p[LFO_FADE]), tick_rate),
            tune: transpose_steps(p[TRANSPOSE]) + w * bend_range,
            drive,
            drive_norm: if drive > 0.0 { 1.0 / drive.tanh() } else { 1.0 },
            gain: 10f32.powf(0.9 * p[GAIN] - 0.9),
            delay_frames: ((delay_s * rate) as usize).clamp(1, self.delay[0].len() - 1),
            feedback: DELAY_FEEDBACK_MAX * p[DELAY_FEEDBACK],
            mix: p[DELAY_MIX] * p[DELAY_MIX],
            delay_filter: 2.0 * p[DELAY_FILTER],
        };
    }

    /// Starts `key` on a voice: the voice law (poly: first free, else the oldest-started; Mono: voice 0, retriggered
    /// only once it is releasing), the glide from the previously played note, Flip and the phase rules.
    fn start_note(&mut self, key: u8, velocity: f32, pan: f32) {
        // Note < 1 → silence.
        if key < 1 {
            return;
        }
        self.update_control();
        let p = &self.params;
        let mono = self.on(MONO);
        let index = if mono {
            0
        } else {
            // trunc(fma_f32(v, 7, 1)).
            let count = (p[VOICES].mul_add(7.0, 1.0) as usize).min(MAX_VOICES);
            let voices = &self.voices[..count];
            voices
                .iter()
                .position(|voice| voice.amp.is_idle())
                .unwrap_or_else(|| (0..count).min_by_key(|&i| voices[i].age).unwrap_or(0))
        };
        // Arp notes reach key + 36, past the 128 held slots.
        let key_held = self.held.get(usize::from(key)).is_some_and(Option::is_some);
        let others_held = self.held_count > usize::from(key_held);
        let slide = (0.5 * p[SLIDE]).powi(2);
        let source = self
            .last_note
            .filter(|_| slide > GLIDE_OFF_S && (!self.on(LEGATO) || others_held));
        let flipped = p[FLIP] > 0.5 && self.last_note.is_some_and(|last| key <= last);
        let supersaw = p[SUPERSYNC] <= 0.5;
        let retrig = self.on(RETRIG);
        let spread = PHASE_SPREAD * p[UNISON_PHASE] * p[UNISON_PHASE];
        let c = self.control;
        self.serial += 1;
        let voice = &mut self.voices[index];
        let retrigger = !mono || !voice.amp.is_held();
        voice.note = key;
        voice.velocity = velocity;
        voice.pan = pan;
        voice.age = self.serial;
        voice.flip = if flipped { -1.0 } else { 1.0 };
        match source {
            Some(from) => {
                let distance = (f32::from(key) - f32::from(from)).abs();
                let ticks = slide * self.sample_rate / TICK as f32;
                voice.pitch = f32::from(from);
                voice.glide_step = if p[CONSTANT] > 0.5 {
                    distance / ticks
                } else {
                    1.0 / ticks
                };
            }
            None => {
                voice.pitch = f32::from(key);
                voice.glide_step = 0.0;
            }
        }
        if retrigger {
            voice.slide = [0.0; 3];
            if !retrig && voice.amp.is_idle() {
                for (phase, sub) in voice.phases.iter_mut().zip(&mut voice.sub_phases) {
                    *phase = self.rng.next();
                    *sub = self.rng.next();
                }
            } else if retrig && supersaw {
                // The sub restarts at its unison voice's phase.
                for (k, (phase, sub)) in voice
                    .phases
                    .iter_mut()
                    .zip(&mut voice.sub_phases)
                    .enumerate()
                {
                    *phase = cycles(f64::from(k as f32 * spread));
                    *sub = *phase;
                }
            }
            voice.re_attack = voice.amp.level > 0.0;
            voice.amp.attack();
            voice.filter_eg.restart();
            voice.sync_eg.restart();
            voice.lfo_fade = 0.0;
            voice.lfo_phase = match c.lfo_mode {
                HERTZ => self.lfo_phase,
                ONE_SHOT => 0.0,
                _ => c.lfo_sync_phase,
            };
            voice.lfo_random = voice.rng.unit();
        }
        self.last_note = Some(key);
        voice.compute(&self.params, &c, self.sample_rate);
    }

    fn release_all(&mut self) {
        for voice in &mut self.voices {
            voice.release();
        }
    }

    /// Runs the arp at this frame and returns the frames until its next step. The step timer runs free from the first
    /// held key and fires at once; each step releases every voice and plays the next note.
    fn arp_step(&mut self) -> usize {
        let mode = self.arp_mode();
        if mode == 0 || self.held_count == 0 {
            if self.arp.running {
                self.arp = Arp::default();
                self.release_all();
            }
            return usize::MAX;
        }
        let beats = tempo_step(&ARP_RATE_BEATS, self.params[ARP_RATE]);
        let length =
            f64::from(beats) * 60.0 / f64::from(self.tempo_bpm) * f64::from(self.sample_rate);
        if !self.arp.running {
            self.arp = Arp {
                running: true,
                step: 0,
                timer: length,
            };
        }
        if self.arp.timer >= length {
            self.arp.timer = (self.arp.timer - length) % length;
            let octaves = ((3.0 * self.params[ARP_RANGE]) as usize).min(3) + 1;
            let position = arp_position(
                mode,
                self.arp.step,
                self.held_count * octaves,
                self.rng.unit(),
            );
            self.arp.step = self.arp.step.wrapping_add(1);
            let next = (0u8..128)
                .filter_map(|key| Some((key, self.held[usize::from(key)]?)))
                .nth(position % self.held_count);
            if let Some((key, velocity)) = next {
                let octave = (position / self.held_count) as u8;
                self.release_all();
                self.start_note(key + 12 * octave, velocity, ARP_PAN);
            }
        }
        (length - self.arp.timer).ceil().max(1.0) as usize
    }

    /// One control tick: global state, the global LFO phase, then every sounding voice.
    fn tick(&mut self) {
        self.update_control();
        let c = self.control;
        // The global Hertz phase does not advance in arp mode (kept, though it looks like a bug).
        if self.arp_mode() == 0 {
            self.lfo_phase = (self.lfo_phase + c.lfo_step).fract();
        }
        for voice in &mut self.voices {
            if !voice.amp.is_idle() {
                voice.advance(&c);
                voice.compute(&self.params, &c, self.sample_rate);
            }
        }
    }

    /// Distortion → Gain → Delay on the voices' sum, added into the output.
    fn effects(&mut self, mix: &[[f32; TICK]; 2], left: &mut [f32], right: &mut [f32]) {
        let c = &self.control;
        let len = self.delay[0].len();
        let read = (self.delay_pos + len - c.delay_frames) % len;
        for (i, (out_l, out_r)) in left.iter_mut().zip(right.iter_mut()).enumerate() {
            let mut out = [0.0; 2];
            for (ch, out) in out.iter_mut().enumerate() {
                let mut x = mix[ch][i];
                if c.drive > 0.0 {
                    x = (c.drive * x).tanh() * c.drive_norm;
                }
                x *= c.gain;
                let line = &mut self.delay[ch];
                let d = line[(read + i) % len];
                let s = &mut self.delay_filter[ch];
                let tap = if c.delay_filter <= 1.0 {
                    *s += (d - *s) * c.delay_filter;
                    *s
                } else {
                    *s += (d - *s) * (c.delay_filter - 1.0).powi(4);
                    d - *s
                };
                line[(self.delay_pos + i) % len] = (tap + x) * c.feedback;
                *out = x + c.mix * tap;
            }
            *out_l += out[0];
            *out_r += out[1];
        }
        self.delay_pos = (self.delay_pos + left.len()) % len;
    }
}

impl Instrument for SuperSaw {
    fn set_parameter(&mut self, index: usize, value: f32) {
        if index >= PARAM_COUNT || !value.is_finite() {
            return;
        }
        let value = value.clamp(0.0, 1.0);
        let mono_on = index == MONO && !self.on(MONO) && value != 0.0;
        self.params[index] = value;
        if mono_on {
            // Turning Mono on silences the other voices.
            for voice in &mut self.voices[1..] {
                voice.amp = Env::default();
            }
        }
    }

    fn note_on(&mut self, key: u8, velocity: f32, pan: f32) {
        let key = key.min(127);
        if self.held[usize::from(key)].replace(velocity).is_none() {
            self.held_count += 1;
        }
        if self.arp_mode() == 0 {
            self.start_note(key, velocity, pan);
        }
    }

    fn note_off(&mut self, key: u8) {
        let key = key.min(127);
        if self.held[usize::from(key)].take().is_some() {
            self.held_count -= 1;
        }
        if self.arp_mode() != 0 {
            return;
        }
        if self.on(MONO) {
            let voice = &self.voices[0];
            if voice.note != key || !voice.amp.is_held() {
                return;
            }
            // Fall back to the highest still-held key without retrigger.
            let (velocity, pan) = (voice.velocity, voice.pan);
            match (0..128u8)
                .rev()
                .find(|&k| self.held[usize::from(k)].is_some())
            {
                Some(next) => self.start_note(next, velocity, pan),
                None => self.voices[0].release(),
            }
        } else {
            for voice in &mut self.voices {
                if voice.note == key && voice.amp.is_held() {
                    voice.release();
                }
            }
        }
    }

    fn all_notes_off(&mut self) {
        self.held = [None; 128];
        self.held_count = 0;
        self.arp = Arp::default();
        self.release_all();
    }

    fn reset(&mut self) {
        self.voices = std::array::from_fn(Voice::new);
        self.held = [None; 128];
        self.held_count = 0;
        self.last_note = None;
        self.tick_pos = 0;
        self.lfo_phase = 0.0;
        self.arp = Arp::default();
        for line in &mut self.delay {
            line.fill(0.0);
        }
        self.delay_filter = [0.0; 2];
    }

    fn process(&mut self, left: &mut [f32], right: &mut [f32], context: ProcessContext) {
        if context.tempo_bpm.is_finite() && context.tempo_bpm > 0.0 {
            self.tempo_bpm = context.tempo_bpm;
        }
        if context.playing {
            self.song_beat = context.start_beat;
        }
        let beats_per_frame = f64::from(self.tempo_bpm) / (60.0 * f64::from(self.sample_rate));
        let frames = left.len().min(right.len());
        let mut done = 0;
        while done < frames {
            let arp_wait = self.arp_step();
            if self.tick_pos == 0 {
                self.tick();
            }
            let n = (TICK - self.tick_pos).min(frames - done).min(arp_wait);
            let mut mix = [[0.0; TICK]; 2];
            for voice in &mut self.voices {
                if !voice.amp.is_idle() {
                    voice.render(self.tables, &self.control, &mut mix, n);
                }
            }
            self.effects(&mix, &mut left[done..done + n], &mut right[done..done + n]);
            self.tick_pos = (self.tick_pos + n) % TICK;
            self.arp.timer += n as f64;
            self.song_beat += n as f64 * beats_per_frame;
            done += n;
        }
    }

    fn set_tempo_bpm(&mut self, bpm: f32) {
        if bpm.is_finite() && bpm > 0.0 {
            self.tempo_bpm = bpm;
        }
    }

    fn set_controller(&mut self, index: usize, value: f32) {
        match index {
            PITCH_BEND => self.bend = value.clamp(0.0, 1.0),
            MOD_WHEEL => self.wheel = value.clamp(0.0, 1.0),
            _ => {}
        }
    }

    /// Slides add to every voice's pitch, velocity and pan from the next control tick.
    fn set_slide(&mut self, pitch: f32, pan: f32, velocity: f32) {
        let finite = |value: f32| if value.is_finite() { value } else { 0.0 };
        for voice in &mut self.voices {
            voice.slide = [finite(pitch), finite(pan), finite(velocity)];
        }
    }
}

/// The global state of one control tick.
#[derive(Clone, Copy, Default)]
struct Control {
    /// The mod-wheel term at its destination: (2v58 − 1) × wheel.
    wheel: [f32; MODS],
    amp: Adsr,
    /// The amp attack step when re-attacking a sounding voice.
    amp_re_attack: f32,
    filter: Adsr,
    sync: Adsr,
    lfo_shape: usize,
    lfo_mode: usize,
    lfo_dest: usize,
    /// A = (2v44 − 1)·|2v44 − 1|.
    lfo_amount: f32,
    /// Hertz and One Shot: cycles per tick.
    lfo_step: f64,
    /// Tmp Sync: the phase locked to the song position.
    lfo_sync_phase: f64,
    lfo_fade_step: f32,
    /// trunc(24·v36) − 12 + w·B semitones.
    tune: f32,
    /// Distortion k (0 = off) and 1/tanh(k).
    drive: f32,
    drive_norm: f32,
    gain: f32,
    delay_frames: usize,
    feedback: f32,
    mix: f32,
    /// c = 2·v57: LP up to 1, HP above.
    delay_filter: f32,
}

/// Per-swing steps of a linear ADSR, and its sustain level.
#[derive(Clone, Copy, Default)]
struct Adsr {
    attack: f32,
    decay: f32,
    sustain: f32,
    release: f32,
}

impl Adsr {
    /// Times T(a), T(d), T(r) stepped at `rate`.
    fn new(a: f32, d: f32, sustain: f32, r: f32, rate: f32) -> Self {
        Self {
            attack: swing_step(envelope_time(a), rate),
            decay: swing_step(envelope_time(d), rate),
            sustain,
            release: swing_step(envelope_time(r), rate),
        }
    }
}

/// Transpose (p36) in semitones: trunc(f32(24·v)) − 12, the product rounded first, not fused: k − 12 at
/// every v = f32(k/24). The pitch and the text both use it.
pub fn transpose_steps(v: f32) -> f32 {
    (24.0 * v).trunc() - 12.0
}

/// The envelope time law: e = 2^x, then six times e ← 2^(e − 1); T(x) = 2^(5e − 5) − 1 seconds.
fn envelope_time(x: f32) -> f32 {
    let mut e = x.exp2();
    for _ in 0..6 {
        e = (e - 1.0).exp2();
    }
    (5.0 * e - 5.0).exp2() - 1.0
}

/// The step per full 0↔1 swing of a segment lasting `seconds` at `rate` steps per second: 1/(trunc(T·rate) + 1).
fn swing_step(seconds: f32, rate: f32) -> f32 {
    1.0 / ((seconds * rate).trunc() + 1.0)
}

#[derive(Clone, Copy, Default, PartialEq)]
enum Stage {
    #[default]
    Idle,
    Attack,
    /// Decay, stopping at sustain.
    Decay,
    Release,
}

/// A linear envelope.
#[derive(Clone, Copy, Default)]
struct Env {
    level: f32,
    stage: Stage,
}

impl Env {
    /// Attacks from the current level.
    fn attack(&mut self) {
        self.stage = Stage::Attack;
    }

    fn restart(&mut self) {
        self.level = 0.0;
        self.stage = Stage::Attack;
    }

    fn release(&mut self) {
        if self.stage != Stage::Idle {
            self.stage = Stage::Release;
        }
    }

    fn is_idle(&self) -> bool {
        self.stage == Stage::Idle
    }

    fn is_held(&self) -> bool {
        matches!(self.stage, Stage::Attack | Stage::Decay)
    }

    fn advance(&mut self, law: &Adsr, attack: f32) -> f32 {
        match self.stage {
            Stage::Attack => {
                self.level += attack;
                if self.level >= 1.0 {
                    self.level = 1.0;
                    self.stage = Stage::Decay;
                }
            }
            Stage::Decay => self.level = (self.level - law.decay).max(law.sustain),
            Stage::Release => {
                self.level -= law.release;
                if self.level <= 0.0 {
                    self.level = 0.0;
                    self.stage = Stage::Idle;
                }
            }
            Stage::Idle => {}
        }
        self.level
    }
}

/// Xorshift32.
#[derive(Clone, Copy)]
struct Rng(u32);

impl Rng {
    fn new(seed: usize) -> Self {
        Self(0x2545_f491 ^ (seed as u32 + 1).wrapping_mul(0x9e37_79b9))
    }

    fn next(&mut self) -> u32 {
        self.0 ^= self.0 << 13;
        self.0 ^= self.0 >> 17;
        self.0 ^= self.0 << 5;
        self.0
    }

    /// Uniform in [0, 1).
    fn unit(&mut self) -> f32 {
        (self.next() >> 8) as f32 / (1 << 24) as f32
    }
}

/// The oscillator setup of one tick.
#[derive(Clone, Copy, Default)]
struct Osc {
    supersync: bool,
    unison: usize,
    increments: [u32; MAX_UNISON],
    tables: [usize; MAX_UNISON],
    sub_increments: [u32; MAX_UNISON],
    sub_tables: [usize; MAX_UNISON],
    /// Unison pan gains, left and right; `None` for a mono sum.
    pan: Option<[[f32; MAX_UNISON]; 2]>,
    /// Pulse mode: the second saw's offset, 0.5p cycles.
    pulse: Option<u32>,
    sub_level: f32,
    /// SUPERSYNC: ceil(R) pairs, each saw(φ + jT) − saw(φ + min(jT + WT, 1)).
    pairs: usize,
    starts: [u32; MAX_SYNC_PAIRS],
    ends: [u32; MAX_SYNC_PAIRS],
}

/// The 4-pole ladder, one state per channel, collapsed to y ← P·y + q·u per frame.
#[derive(Clone, Copy, Default)]
struct Ladder {
    p: [[f32; 4]; 4],
    q: [f32; 4],
    tuned: Option<(f64, f64)>,
    y: [[f32; 4]; 2],
}

impl Ladder {
    fn tune(&mut self, g: f64, r: f64) {
        if self.tuned != Some((g, r)) {
            let (p, q) = ladder_matrix(g, r);
            self.p = p.map(|row| row.map(|x| x as f32));
            self.q = q.map(|x| x as f32);
            self.tuned = Some((g, r));
        }
    }

    fn process(&mut self, channel: usize, u: f32) -> f32 {
        let (p, q, y) = (&self.p, &self.q, &mut self.y[channel]);
        let mut next = [0.0; 4];
        for (out, (row, qi)) in next.iter_mut().zip(p.iter().zip(q)) {
            *out = row[0] * y[0] + row[1] * y[1] + row[2] * y[2] + row[3] * y[3] + qi * u;
        }
        next[0] /= 1.0 + Y1_SOFT_CLIP * next[0] * next[0];
        *y = next;
        next[3]
    }
}

/// One frame of the ladder dy1/dt = ω(u − 4r·y4 − y1), dy_i/dt = ω(y_{i−1} − y_i) as 4096 forward-Euler sub-steps of
/// size g with u held: y ← A·y + g·u·e₁ per sub-step, so the frame is y ← A⁴⁰⁹⁶·y + (Σ_{k<4096} Aᵏ)·g·e₁·u. Twelve
/// squarings build both.
fn ladder_matrix(g: f64, r: f64) -> ([[f64; 4]; 4], [f64; 4]) {
    let k = 4.0 * r;
    let mut p = [
        [1.0 - g, 0.0, 0.0, -g * k],
        [g, 1.0 - g, 0.0, 0.0],
        [0.0, g, 1.0 - g, 0.0],
        [0.0, 0.0, g, 1.0 - g],
    ];
    let mut q = [g, 0.0, 0.0, 0.0];
    for _ in 0..LADDER_SUBSTEPS_LOG2 {
        let pq = p.map(|row| row.iter().zip(&q).map(|(a, b)| a * b).sum::<f64>());
        for (qi, add) in q.iter_mut().zip(pq) {
            *qi += add;
        }
        p = std::array::from_fn(|i| {
            std::array::from_fn(|j| (0..4).map(|m| p[i][m] * p[m][j]).sum())
        });
    }
    (p, q)
}

/// The cutoff in Hz: c′ already clamped, `eg` the filter EG level, `note` the voice's (gliding) note.
fn cutoff_hz(c_prime: f32, eg: f32, eg_amount: f32, note: f32, kbd: f32) -> f32 {
    (FC_BASE_OCT
        + FC_RANGE_OCT * c_prime
        + FC_EG_OCT * eg * (2.0 * eg_amount - 1.0)
        + (note / 12.0 - FC_KEY_PIVOT_OCT) * kbd)
        .exp2()
        .min(FC_MAX)
}

/// The LFO shape at `phase`: Sine is bipolar; Saw, Square and Random are unipolar.
fn lfo_shape(shape: usize, phase: f64, random: f32) -> f32 {
    match shape {
        0 => (TAU * phase).sin() as f32,
        1 => 1.0 - phase as f32,
        2 => {
            if phase < 0.5 {
                1.0
            } else {
                0.0
            }
        }
        _ => random,
    }
}

/// The arp's list position at `step` for mode 1 Up, 2 Down, 3 Up/Down, 4 Up/Down 2, 5 Random over a list of
/// `length` notes; `u` is a uniform draw for Random.
fn arp_position(mode: usize, step: usize, length: usize, u: f32) -> usize {
    match mode {
        1 => step % length,
        2 => length - 1 - step % length,
        3 => {
            let cycle = (2 * length).saturating_sub(2).max(1);
            let i = step % cycle;
            if i < length { i } else { cycle - i }
        }
        4 => {
            let i = step % (2 * length);
            if i < length { i } else { 2 * length - 1 - i }
        }
        _ if step == 0 => 0,
        _ => (u * (length - 1) as f32).round() as usize,
    }
}

#[derive(Default)]
struct Arp {
    running: bool,
    step: usize,
    /// Frames since the last step.
    timer: f64,
}

/// The phase of `cycles` (mod 1) as a 32-bit phase.
fn cycles(cycles: f64) -> u32 {
    (cycles.rem_euclid(1.0) * PHASE_SCALE) as u32
}

/// The 32-bit phase increment for `f` Hz: f/sr·0.99951·2³², saturating at very high pitch.
fn increment(f: f32, sample_rate: f32) -> u32 {
    (f64::from(f) / f64::from(sample_rate) * INCREMENT_FACTOR * PHASE_SCALE) as u32
}

/// The offset of the table for `f` Hz: it has clamp(floor((sr/2)/f), 1, 1023) + 1 harmonics.
fn table_offset(f: f32, sample_rate: f32) -> usize {
    let harmonics = (0.5 * sample_rate / f)
        .floor()
        .clamp(1.0, (TABLES - 1) as f32) as usize
        + 1;
    (harmonics - 1) * TABLE_LEN
}

/// The global wavetable set, built once: 1024 tables × 2048 samples, table m holding harmonics 1..=m of a falling saw,
/// all scaled by one factor so the global peak is 1.
fn wavetables() -> &'static [f32] {
    static SET: OnceLock<Box<[f32]>> = OnceLock::new();
    SET.get_or_init(|| {
        let sine: Vec<f64> = (0..TABLE_LEN)
            .map(|n| (TAU * n as f64 / TABLE_LEN as f64).sin())
            .collect();
        let mut sum = vec![0.0f64; TABLE_LEN];
        let mut set = vec![0.0f32; TABLES * TABLE_LEN];
        let mut peak = 0.0f64;
        for (m, table) in set.as_chunks_mut::<TABLE_LEN>().0.iter_mut().enumerate() {
            let h = m + 1;
            for (n, (acc, out)) in sum.iter_mut().zip(table).enumerate() {
                *acc += sine[h * n % TABLE_LEN] / h as f64;
                peak = peak.max(acc.abs());
                *out = *acc as f32;
            }
        }
        let scale = (1.0 / peak) as f32;
        set.iter_mut().for_each(|x| *x *= scale);
        set.into_boxed_slice()
    })
}

#[derive(Clone, Copy)]
struct Voice {
    note: u8,
    velocity: f32,
    pan: f32,
    /// Slide offsets: semitones, pan, velocity; a launched voice starts at 0.
    slide: [f32; 3],
    /// Start serial, for stealing the oldest-started.
    age: u64,
    /// The gliding note and its step per tick toward `note`.
    pitch: f32,
    glide_step: f32,
    /// −1 when Flip negates the sync EG amount.
    flip: f32,
    amp: Env,
    re_attack: bool,
    filter_eg: Env,
    sync_eg: Env,
    lfo_phase: f64,
    lfo_fade: f32,
    lfo_random: f32,
    phases: [u32; MAX_UNISON],
    sub_phases: [u32; MAX_UNISON],
    osc: Osc,
    ladder: Ladder,
    /// Smoothed output gains, left and right, and their tick targets.
    gain: [f32; 2],
    target: [f32; 2],
    /// The dither (and Random LFO) source.
    rng: Rng,
}

impl Voice {
    fn new(index: usize) -> Self {
        Self {
            note: 0,
            velocity: 0.0,
            pan: 0.5,
            slide: [0.0; 3],
            age: 0,
            pitch: 0.0,
            glide_step: 0.0,
            flip: 1.0,
            amp: Env::default(),
            re_attack: false,
            filter_eg: Env::default(),
            sync_eg: Env::default(),
            lfo_phase: 0.0,
            lfo_fade: 0.0,
            lfo_random: 0.0,
            phases: [0; MAX_UNISON],
            sub_phases: [0; MAX_UNISON],
            osc: Osc::default(),
            ladder: Ladder::default(),
            gain: [0.0; 2],
            target: [0.0; 2],
            rng: Rng::new(index + 1),
        }
    }

    fn release(&mut self) {
        self.amp.release();
        self.filter_eg.release();
        self.sync_eg.release();
    }

    /// The tick's state steps: glide, the filter and sync EGs, the LFO phase and fade-in.
    fn advance(&mut self, c: &Control) {
        let note = f32::from(self.note);
        self.pitch = if self.pitch < note {
            (self.pitch + self.glide_step).min(note)
        } else {
            (self.pitch - self.glide_step).max(note)
        };
        self.filter_eg.advance(&c.filter, c.filter.attack);
        self.sync_eg.advance(&c.sync, c.sync.attack);
        let old = self.lfo_phase;
        self.lfo_phase = match c.lfo_mode {
            HERTZ => (old + c.lfo_step).fract(),
            ONE_SHOT => (old + c.lfo_step).min(1.0),
            _ => c.lfo_sync_phase,
        };
        if self.lfo_phase < old {
            self.lfo_random = self.rng.unit();
        }
        self.lfo_fade = (self.lfo_fade + c.lfo_fade_step).min(1.0);
    }

    /// The tick's derived values: oscillator increments and tables, the ladder, the output targets.
    fn compute(&mut self, p: &[f32; PARAM_COUNT], c: &Control, sample_rate: f32) {
        let lfo =
            self.lfo_fade * lfo_shape(c.lfo_shape, self.lfo_phase, self.lfo_random) * c.lfo_amount;
        let mut m = c.wheel;
        m[c.lfo_dest] += lfo;
        let f = 440.0
            * ((self.pitch - 69.0 + c.tune + self.slide[0] + 12.0 * m[M_FREQUENCY]) / 12.0).exp2();
        let osc = &mut self.osc;
        let unison_gain = if p[SUPERSYNC] > 0.5 {
            let a = (2.0 * p[SYNC_EG_AMOUNT] - 1.0).powi(3) * self.flip;
            let s = (p[SYNC] + self.sync_eg.level * a + m[M_SYNC]).clamp(0.0, 1.0);
            let r = 1.0 + SYNC_RANGE * s;
            let t = 1.0 / r;
            let w = (0.5 * p[SYNC_PULSE_WIDTH] + m[M_PULSE_WIDTH]).clamp(0.0, 1.0);
            osc.supersync = true;
            osc.unison = 1;
            osc.pan = None;
            osc.pairs = (r.ceil() as usize).min(MAX_SYNC_PAIRS);
            for j in 0..osc.pairs {
                let start = j as f32 * t;
                osc.starts[j] = cycles(f64::from(start));
                osc.ends[j] = cycles(f64::from((start + w * t).min(1.0)));
            }
            osc.increments[0] = increment(f, sample_rate);
            osc.tables[0] = table_offset(f, sample_rate);
            1.0
        } else {
            // trunc(fma_f32(v, 11, 1)).
            let n = (p[UNISON].mul_add(11.0, 1.0) as usize).clamp(1, MAX_UNISON);
            let d = (p[DETUNE] * p[DETUNE] + m[M_DETUNE]).clamp(0.0, 1.0);
            let curve = 2.0 * p[DETUNE_CURVE] - 1.0;
            let e = 1.0 + curve * if curve >= 0.0 { CURVE_UP } else { CURVE_DOWN };
            let spread = n > 1 && p[UNISON_PAN] >= UNISON_PAN_MIN;
            let mut pan = [[1.0; MAX_UNISON]; 2];
            #[allow(clippy::needless_range_loop)] // k fills six parallel per-voice arrays
            for k in 0..n {
                let x = if n == 1 {
                    0.0
                } else {
                    k as f32 / (n - 1) as f32
                };
                let ratio = if n == 1 {
                    1.0
                } else {
                    (d / 12.0 * (2.0 * x.powf(e) - 1.0)).exp2()
                };
                let fk = f * ratio;
                osc.increments[k] = increment(fk, sample_rate);
                osc.tables[k] = table_offset(fk, sample_rate);
                osc.sub_increments[k] = increment(0.5 * fk, sample_rate);
                osc.sub_tables[k] = table_offset(0.5 * fk, sample_rate);
                if spread {
                    let q = (2.0 * x - 1.0) * p[UNISON_PAN];
                    if q >= 0.0 {
                        pan[0][k] = 1.0 - q;
                    } else {
                        pan[1][k] = 1.0 + q;
                    }
                }
            }
            osc.supersync = false;
            osc.unison = n;
            osc.pan = spread.then_some(pan);
            osc.pulse = (p[PULSE_MODE] > 0.5).then(|| {
                cycles(f64::from(
                    0.5 * (p[PULSE_WIDTH] + m[M_PULSE_WIDTH]).clamp(0.0, 1.0),
                ))
            });
            let sub = p[SUB_LEVEL] * p[SUB_LEVEL];
            osc.sub_level = if sub < SUB_MIN { 0.0 } else { sub };
            10f32.powf(-(n as f32) / 20.0)
        };
        // Key tracking reads the gliding note, without Transpose or bend.
        let c_prime = (p[CUTOFF] + self.velocity * p[VEL_TRK] + m[M_CUTOFF]).clamp(0.0, 1.0);
        let fc = cutoff_hz(
            c_prime,
            self.filter_eg.level,
            p[FILTER_EG_AMOUNT],
            self.pitch,
            p[KBD_TRK],
        );
        let g = (LADDER_OMEGA * f64::from(fc) / (LADDER_SUBSTEPS * f64::from(sample_rate)))
            .clamp(0.0, 1.0);
        let r = (p[RESONANCE] + m[M_RESONANCE]).clamp(0.0, 1.0);
        self.ladder.tune(g, f64::from(r));
        let level = (p[LEVEL] + m[M_LEVEL]).clamp(0.0, 1.0);
        let gain = level * level * unison_gain * (self.velocity + self.slide[2]).clamp(0.0, 1.0);
        let pan = (p[PAN] + (self.pan + self.slide[1] - 0.5) + m[M_PAN]).clamp(0.0, 1.0);
        self.target = [
            gain * (2.0 - 2.0 * pan).min(1.0),
            gain * (2.0 * pan).min(1.0),
        ];
    }

    /// Adds `frames` frames into `mix`: oscillators, the ladder per channel, the amp EG² and the smoothed gains.
    fn render(&mut self, tables: &[f32], c: &Control, mix: &mut [[f32; TICK]; 2], frames: usize) {
        let o = &self.osc;
        let attack = if self.re_attack {
            c.amp_re_attack
        } else {
            c.amp.attack
        };
        let [mix_l, mix_r] = mix;
        for (out_l, out_r) in mix_l.iter_mut().zip(mix_r.iter_mut()).take(frames) {
            // One dither per frame, shared by every oscillator of the voice; no interpolation.
            let dither = self.rng.next() >> DITHER_SHIFT;
            let read = |table: usize, phase: u32| {
                tables[table + (phase.wrapping_add(dither) >> INDEX_SHIFT) as usize]
            };
            let (mut l, mut r) = (0.0, 0.0);
            if o.supersync {
                let (phase, table) = (self.phases[0], o.tables[0]);
                for (start, end) in o.starts[..o.pairs].iter().zip(&o.ends) {
                    l += read(table, phase.wrapping_add(*start))
                        - read(table, phase.wrapping_add(*end));
                }
                self.phases[0] = phase.wrapping_add(o.increments[0]);
            } else {
                for k in 0..o.unison {
                    let phase = self.phases[k];
                    let mut s = read(o.tables[k], phase);
                    if let Some(width) = o.pulse {
                        s -= read(o.tables[k], phase.wrapping_add(width));
                    }
                    if o.sub_level > 0.0 {
                        s += o.sub_level * read(o.sub_tables[k], self.sub_phases[k]);
                    }
                    match &o.pan {
                        Some(pan) => {
                            l += s * pan[0][k];
                            r += s * pan[1][k];
                        }
                        None => l += s,
                    }
                    self.phases[k] = phase.wrapping_add(o.increments[k]);
                    self.sub_phases[k] = self.sub_phases[k].wrapping_add(o.sub_increments[k]);
                }
            }
            let (yl, yr) = if o.pan.is_some() {
                (self.ladder.process(0, l), self.ladder.process(1, r))
            } else {
                // A mono sum: one channel's ladder, its state copied to the other.
                let y = self.ladder.process(0, l);
                self.ladder.y[1] = self.ladder.y[0];
                (y, y)
            };
            let env = self.amp.advance(&c.amp, attack);
            let env2 = env * env;
            for (gain, target) in self.gain.iter_mut().zip(self.target) {
                *gain += GAIN_SMOOTHING * (target - *gain);
            }
            *out_l += yl * env2 * self.gain[0];
            *out_r += yr * env2 * self.gain[1];
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn envelope_time_matches_the_spec_points() {
        for (x, t) in [
            (0.0, 0.0),
            (0.1, 0.030),
            (0.25, 0.094),
            (0.5, 0.305),
            (0.75, 1.07),
            (0.9, 3.93),
            (1.0, 31.0),
        ] {
            let got = envelope_time(x);
            assert!((got - t).abs() <= 0.006 * t.max(0.1), "T({x}) = {got}");
        }
        // Amp release T(0.85·v + 0.05): 14 ms … 3.93 s.
        assert!((envelope_time(0.05) - 0.014).abs() < 0.0005);
        assert!((envelope_time(0.9) - 3.93).abs() < 0.005);
        // The re-attack floor T(0.01) ≈ 2.7 ms.
        assert!((envelope_time(RE_ATTACK_MIN_X) - 0.0027).abs() < 0.0001);
        assert_eq!(swing_step(0.0, 48_000.0), 1.0);
        assert_eq!(swing_step(1.0, 48_000.0), 1.0 / 48_001.0);
    }

    #[test]
    fn collapsed_ladder_equals_4096_euler_sub_steps() {
        for (g, r) in [(3.2e-5, 0.0), (6.9e-4, 0.7), (1e-6, 1.0)] {
            let (p, q) = ladder_matrix(g, r);
            let mut y = [0.3, -0.2, 0.1, 0.4];
            let u = 0.8;
            let expect: [f64; 4] =
                std::array::from_fn(|i| (0..4).map(|j| p[i][j] * y[j]).sum::<f64>() + q[i] * u);
            for _ in 0..4096 {
                y = [
                    y[0] + g * (u - 4.0 * r * y[3] - y[0]),
                    y[1] + g * (y[0] - y[1]),
                    y[2] + g * (y[1] - y[2]),
                    y[3] + g * (y[2] - y[3]),
                ];
            }
            for (a, b) in y.iter().zip(expect) {
                assert!((a - b).abs() < 1e-9, "g {g} r {r}: {a} {b}");
            }
        }
    }

    /// The steady-state output amplitude for a sine at `hz`.
    fn ladder_gain(fc: f32, r: f64, hz: f64) -> f32 {
        let sr = 48_000.0;
        let mut ladder = Ladder::default();
        ladder.tune(LADDER_OMEGA * f64::from(fc) / (LADDER_SUBSTEPS * sr), r);
        let mut peak = 0.0f32;
        for n in 0..96_000 {
            let y = ladder.process(0, (TAU * hz * f64::from(n) / sr).sin() as f32);
            if n > 72_000 {
                peak = peak.max(y.abs());
            }
        }
        peak
    }

    #[test]
    fn ladder_is_minus_12_db_at_omega_and_peaks_there_with_resonance() {
        // H(jω) = −1/(4 − 4r) at ω = 6.25·fc rad/s: 1/4 at r = 0, 2.5 at r = 0.9.
        let at = LADDER_OMEGA * 1000.0 / TAU;
        assert!((ladder_gain(1000.0, 0.0, at) - 0.25).abs() < 0.005);
        let resonant = ladder_gain(1000.0, 0.9, at);
        assert!((resonant - 2.5).abs() < 0.05, "{resonant}");
        assert!(ladder_gain(1000.0, 0.9, at / 2.0) < 0.5 * resonant);
        assert!(ladder_gain(1000.0, 0.9, at * 2.0) < 0.5 * resonant);
    }

    #[test]
    fn cutoff_law() {
        assert_eq!(cutoff_hz(1.0, 0.0, 0.5, 60.0, 0.0), FC_MAX);
        assert_eq!(cutoff_hz(0.0, 0.0, 0.5, 60.0, 0.0), 16.0);
        // EG ±8 octaves; key tracking pivots on note 48.
        assert_eq!(cutoff_hz(0.0, 1.0, 1.0, 60.0, 0.0), 16.0 * 256.0);
        assert_eq!(cutoff_hz(0.0, 1.0, 0.0, 60.0, 0.0), 16.0 / 256.0);
        assert_eq!(cutoff_hz(0.0, 0.0, 0.5, 48.0, 1.0), 16.0);
        assert_eq!(cutoff_hz(0.0, 0.0, 0.5, 72.0, 1.0), 64.0);
    }

    #[test]
    fn wavetables_peak_at_one_and_table_m_has_m_harmonics() {
        let set = wavetables();
        let peak = set.iter().fold(0.0f32, |p, x| p.max(x.abs()));
        assert!((peak - 1.0).abs() < 1e-6);
        // The first harmonic's amplitude is the scale factor ≈ 0.5404.
        let harmonic = |table: &[f32], h: usize| {
            let (re, im) = table
                .iter()
                .enumerate()
                .fold((0.0, 0.0), |(re, im), (n, x)| {
                    let a = TAU * (h * n) as f64 / TABLE_LEN as f64;
                    (re + f64::from(*x) * a.cos(), im + f64::from(*x) * a.sin())
                });
            2.0 * (re * re + im * im).sqrt() / TABLE_LEN as f64
        };
        let table_10 = &set[9 * TABLE_LEN..10 * TABLE_LEN];
        let scale = harmonic(table_10, 1);
        assert!((scale - 0.5404).abs() < 0.0005, "{scale}");
        assert!((harmonic(table_10, 10) - scale / 10.0).abs() < 1e-5);
        assert!(harmonic(table_10, 11) < 1e-5);
        // Falling: Σ sin(hπ/2)/h = 0.834 at a quarter cycle, its negative at three quarters.
        assert!((table_10[512] - 0.834 * scale as f32).abs() < 1e-3);
        assert!((table_10[1536] + 0.834 * scale as f32).abs() < 1e-3);
    }

    #[test]
    fn table_choice_and_increment() {
        let sr = 48_000.0;
        // floor(24000/440) = 54 → 55 harmonics.
        assert_eq!(table_offset(440.0, sr), 54 * TABLE_LEN);
        assert_eq!(table_offset(10.0, sr), (TABLES - 1) * TABLE_LEN);
        assert_eq!(table_offset(20_000.0, sr), TABLE_LEN);
        assert_eq!(
            increment(sr / 4.0, sr),
            (0.25 * INCREMENT_FACTOR * PHASE_SCALE) as u32
        );
        assert_eq!(increment(2.0 * sr, sr), u32::MAX);
    }

    #[test]
    fn arp_orders() {
        let run = |mode, length| {
            (0..10)
                .map(|s| arp_position(mode, s, length, 0.0))
                .collect::<Vec<_>>()
        };
        assert_eq!(run(1, 3), [0, 1, 2, 0, 1, 2, 0, 1, 2, 0]);
        assert_eq!(run(2, 3), [2, 1, 0, 2, 1, 0, 2, 1, 0, 2]);
        assert_eq!(run(3, 3), [0, 1, 2, 1, 0, 1, 2, 1, 0, 1]);
        assert_eq!(run(4, 3), [0, 1, 2, 2, 1, 0, 0, 1, 2, 2]);
        assert_eq!(run(3, 1), [0; 10]);
        assert_eq!(run(4, 1), [0; 10]);
        assert_eq!(run(3, 2), [0, 1, 0, 1, 0, 1, 0, 1, 0, 1]);
        // Random: the first step is the lowest, then round(u·(L − 1)).
        assert_eq!(arp_position(5, 0, 8, 0.99), 0);
        assert_eq!(arp_position(5, 3, 8, 0.99), 7);
        assert_eq!(arp_position(5, 3, 8, 0.5), 4);
    }

    #[test]
    fn stepped_selectors_follow_round_13() {
        // Voices trunc(fma_f32(v, 7, 1)); Unison and PB range trunc(fma_f32(v, 11, 1)); LFO
        // sync trunc(9v) in f64; LFO Dest and Shape trunc(fma_f32(v, 7, 0.5)) and trunc(fma_f32(v, 3, 0.5)). The fused
        // sum rounds once, so one ulp below f32(k/7) can already be k + 1 voices.
        use crate::tests::step_edges as edges;
        let fresh = |index: usize, v: f32| {
            let mut s = SuperSaw::new(48_000.0);
            s.set_parameter(index, v);
            s
        };
        let voices = |v: f32| {
            let mut s = fresh(VOICES, v);
            for key in 60..60 + MAX_VOICES as u8 {
                s.note_on(key, 1.0, 0.5);
            }
            s.voices.iter().filter(|voice| !voice.amp.is_idle()).count()
        };
        assert_eq!(
            edges(7, voices),
            [
                vec![2, 3, 3, 5, 5, 6, 7],
                (1..=8).collect(),
                (1..=7).collect()
            ]
        );
        let unison = |v: f32| {
            let mut s = fresh(UNISON, v);
            s.note_on(60, 1.0, 0.5);
            s.voices[0].osc.unison
        };
        let fused_11 = [
            vec![2, 3, 3, 5, 6, 6, 7, 9, 9, 11, 11],
            (1..=12).collect(),
            (1..=11).collect(),
        ];
        assert_eq!(edges(11, unison), fused_11);
        let control = |index: usize, v: f32| {
            let mut s = fresh(index, v);
            // Bend fully up and transpose 0: tune = the PB range.
            s.set_controller(PITCH_BEND, 1.0);
            s.song_beat = 0.0625;
            s.update_control();
            s.control
        };
        assert_eq!(
            edges(11, |v| control(BEND_RANGE, v).tune as usize),
            fused_11
        );
        let sync = |v: f32| {
            let phase = control(LFO_BEAT, v).lfo_sync_phase;
            (SUPERSAW_LFO_BEATS.iter())
                .position(|&beats| (0.0625 / f64::from(beats)).rem_euclid(1.0) == phase)
                .unwrap()
        };
        assert_eq!(
            edges(9, sync),
            [
                (0..9).collect::<Vec<_>>(),
                (0..10).collect(),
                (0..9).collect()
            ]
        );
        // Dest and Shape step at the half steps (k + ½)/n: probe every f32(j/2n).
        assert_eq!(
            edges(14, |v| control(LFO_DEST, v).lfo_dest),
            [
                vec![1, 1, 1, 2, 2, 3, 3, 4, 4, 5, 5, 6, 6, 7],
                vec![0, 1, 1, 2, 2, 3, 3, 4, 4, 5, 5, 6, 6, 7, 7],
                vec![0, 1, 1, 2, 2, 3, 3, 4, 4, 5, 5, 6, 6, 7],
            ]
        );
        assert_eq!(
            edges(6, |v| control(LFO_SHAPE, v).lfo_shape),
            [
                vec![1, 1, 1, 2, 2, 3],
                vec![0, 1, 1, 2, 2, 3, 3],
                vec![0, 1, 1, 2, 2, 3]
            ]
        );
    }
}
