//! SFZ (sfzformat.com), the text instrument format, read for the SoundFont Player: v1's mapping, tuning, level, loop,
//! envelope, filter, LFO, trigger and controller opcodes and the common v2 ones ([`SfzRegion`]), with v2's numbered LFOs
//! and envelopes and its CC modulation ([`Route`]).
//!
//! [`parse`] is pure: text in, and an `#include` reader; regions out. `<control>`, `<global>`, `<master>`, `<group>` and
//! `<region>` nest as the format says (a region starts from its group, which starts from its master, then its global;
//! a new header of a level drops the levels below it). `<control>` and `<curve>` give the whole instrument's
//! [`SfzControl`]; `<effect>`, `<midi>`, `<sample>` and unknown headers are read past, their opcodes dropped.
//! `#define $NAME value` substitutes `$NAME` in later lines; `#include "file"` reads the file through the reader (depth
//! [`MAX_INCLUDE_DEPTH`]; the reader keeps it inside the library). `//` and `/* */` are comments. A `sample=`,
//! `default_path=` or label value runs to the end of its line or the next `opcode=`, so it may hold spaces; other values
//! are one word. Opcodes this player does not play are counted in [`Sfz::skipped`], never an error. Input is hostile:
//! regions, include depth and the text read are bounded, and so are each region's LFOs, envelopes and routes.

use std::collections::BTreeMap;

use crate::sample::{SampleData, SampleMeta};

/// How deep `#include`s nest: a file that includes itself stops there.
pub const MAX_INCLUDE_DEPTH: usize = 8;
/// The most regions read.
pub const MAX_REGIONS: usize = crate::soundfont::MAX_REGIONS;
/// The most text read, the includes counted each time they are read: one that includes another many times stops here.
const MAX_TEXT: usize = 64 << 20;

/// `loop_mode`.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum LoopMode {
    /// Plays to its end, or until the note's release ends.
    #[default]
    NoLoop,
    /// Plays to its end whatever the note does.
    OneShot,
    /// Loops until the release ends.
    Continuous,
    /// Loops while the key is held, then plays on to its end.
    Sustain,
}

/// `trigger`.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum Trigger {
    /// On note-on.
    #[default]
    Attack,
    /// On note-off, at the note-on's velocity.
    Release,
    /// On note-on with no other key held.
    First,
    /// On note-on with another key held.
    Legato,
}

/// The most LFOs, envelopes (besides the amplifier's) and routes one region has: past them an opcode is skipped.
pub const MAX_LFOS: usize = 8;
pub const MAX_EGS: usize = 6;
pub const MAX_ROUTES: usize = 64;

/// What a [`Route`] moves, in the target's units.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Target {
    /// dB, added.
    Volume,
    /// Percent: a CC or an envelope scales the gain by it, an LFO swings it around full.
    Amplitude,
    /// −100 (left) … 100 (right), added.
    Pan,
    /// Cents, added.
    Pitch,
    /// Cents, added (`cutoff`'s Hz).
    Cutoff,
    /// dB, added.
    Resonance,
}

impl Target {
    /// The target an opcode's stem names (`volume_oncc7`, `lfo01_pitch`, `eg02_cutoff`).
    fn named(stem: &str) -> Option<Self> {
        Some(match stem {
            "volume" | "gain" => Self::Volume,
            "amplitude" => Self::Amplitude,
            "pan" => Self::Pan,
            "pitch" | "tune" => Self::Pitch,
            "cutoff" => Self::Cutoff,
            "resonance" => Self::Resonance,
            _ => return None,
        })
    }
}

/// What drives a [`Route`].
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Source {
    /// MIDI CC N (0…1), through the route's curve.
    Cc(u8),
    /// The region's LFO at this index of [`SfzRegion::lfos`] (−1…1).
    Lfo(usize),
    /// Its envelope at this index of [`SfzRegion::egs`] (its level).
    Eg(usize),
}

/// A modulation: `source` moves `target` by `depth` at its full value.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Route {
    pub source: Source,
    pub target: Target,
    pub depth: f32,
    /// A CC's `<curve>` (`*_curveccN`), 0 (linear) by default. [`crate::soundfont::SoundFont::sfz`] turns it into its
    /// table's index.
    pub curve: u8,
    /// A CC's smoothing (`*_smoothccN`), milliseconds.
    pub smooth: f32,
    /// A CC that adds to `depth`, its amount at CC 127 (`pitchlfo_depthccN`, `lfo01_pitch_onccN`).
    pub depth_cc: Option<(u8, f32)>,
}

/// An LFO: v1's `amplfo_*`, `fillfo_*`, `pitchlfo_*`, or v2's `lfoN_*`.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Lfo {
    /// Hz.
    pub freq: f32,
    /// Seconds silent, then seconds fading in.
    pub delay: f32,
    pub fade: f32,
    /// Its start in the cycle, 0…1.
    pub phase: f32,
    /// `lfoN_wave`: 0 triangle, 1 sine, 2 pulse 75 %, 3 square, 4 pulse 25 %, 5 pulse 12.5 %, 6 saw up, 7 saw down; any
    /// other plays the sine.
    pub wave: u8,
    /// A CC that adds to `freq`, its Hz at CC 127.
    pub freq_cc: Option<(u8, f32)>,
    /// Which one: v2's number, or v1's from [`V1`].
    id: u8,
}

/// An envelope: v1's `fileg_*` and `pitcheg_*` as their DAHDSR's points, or v2's `egN_*`.
#[derive(Clone, Debug, PartialEq)]
pub struct Eg {
    /// Per point, the seconds it takes from the point before and its level; the level starts at the first point's.
    pub points: Vec<(f32, f32)>,
    /// The point it holds at while the key is down, then a release moves on from there; `None` runs through.
    pub sustain: Option<usize>,
    id: u8,
}

/// The ids v1's LFOs and envelopes start from, past v2's numbers.
const V1: u8 = 0xF0;
/// v1's LFOs and envelopes, by opcode prefix (the v2 aliases `fil_` and `pitch_` too), with their target.
const V1_LFOS: [(&str, Target); 3] = [
    ("amplfo_", Target::Volume),
    ("fillfo_", Target::Cutoff),
    ("pitchlfo_", Target::Pitch),
];
const V1_EGS: [(&str, Target); 4] = [
    ("fileg_", Target::Cutoff),
    ("pitcheg_", Target::Pitch),
    ("fil_", Target::Cutoff),
    ("pitch_", Target::Pitch),
];
/// A v1 envelope's DAHDSR stages, by its points: the delay at the start level, the attack to full, the hold, the decay
/// to the sustain level (where it holds), the release to 0.
const DAHDSR: [&str; 5] = ["delay", "attack", "hold", "decay", "release"];

/// `fil_type`'s response.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum FilterShape {
    LowPass,
    HighPass,
    BandPass,
    BandReject,
    LowShelf,
    HighShelf,
    Peak,
}

/// `fil_type`: its response and poles (1, 2, 4 or 6), `None` for a type this player lacks (`apf_1p`, `pink`).
fn filter_type(value: &str) -> Option<(FilterShape, u8)> {
    use FilterShape::*;
    let mut parts = value.split('_');
    let shape = match parts.next()? {
        "lpf" => LowPass,
        "hpf" => HighPass,
        "bpf" => BandPass,
        "brf" => BandReject,
        "lsh" => return Some((LowShelf, 2)),
        "hsh" => return Some((HighShelf, 2)),
        "peq" | "pkf" => return Some((Peak, 2)),
        _ => return None,
    };
    // `_sv`, a state-variable filter, plays as the same response.
    let poles = match parts.next()? {
        "1p" => 1,
        "2p" => 2,
        "4p" => 4,
        "6p" => 6,
        _ => return None,
    };
    Some((shape, poles))
}

/// One `<region>`, with what its headers gave it. Values are as written, bounded to the format's ranges; keys are MIDI
/// notes (−1 is none), `default_path`, `note_offset` and `octave_offset` already applied.
#[derive(Clone, Debug, PartialEq)]
pub struct SfzRegion {
    /// The sample's path from the SFZ's folder, `/`-separated; a built-in one (`*sine`) as named.
    pub sample: String,
    /// `lokey`, `hikey` (`key` sets both and the key center).
    pub keys: [i32; 2],
    /// `lovel`, `hivel`.
    pub vels: [u8; 2],
    pub pitch_keycenter: i32,
    /// Cents per key.
    pub pitch_keytrack: f32,
    /// Semitones.
    pub transpose: i32,
    /// Cents.
    pub tune: f32,
    /// dB.
    pub volume: f32,
    /// Percent.
    pub amplitude: f32,
    /// −100 (left) … 100 (right).
    pub pan: f32,
    /// Percent, −100…100.
    pub amp_veltrack: f32,
    /// `amp_velcurve_N`: (velocity, gain 0…1) in the order written.
    pub amp_velcurve: Vec<(u8, f32)>,
    /// Frames.
    pub offset: u64,
    /// The last frame played; ≤ 0 plays nothing.
    pub end: Option<i64>,
    pub loop_mode: Option<LoopMode>,
    /// `loop_start` (`loopstart`), frames.
    pub loop_start: Option<u64>,
    /// `loop_end` (`loopend`): the loop's last frame.
    pub loop_end: Option<u64>,
    /// `ampeg_delay`, `_attack`, `_hold`, `_decay` (seconds), `_sustain` (percent), `_release` (seconds).
    pub ampeg: [f32; 6],
    pub trigger: Trigger,
    pub group: i64,
    /// A voice of this region stops when a region of this group starts.
    pub off_by: Option<i64>,
    /// `off_mode=normal`: a stopped voice releases; `fast` (the default) fades out at once.
    pub off_normal: bool,
    pub seq_length: u32,
    pub seq_position: u32,
    /// `lorand`, `hirand`.
    pub rand: [f32; 2],
    /// `sw_lokey`, `sw_hikey`: the keyswitch range.
    pub sw_keys: [i32; 2],
    pub sw_last: Option<i32>,
    pub sw_default: Option<i32>,
    /// `xfin_lovel`, `xfin_hivel`.
    pub xfin: [u8; 2],
    /// `xfout_lovel`, `xfout_hivel`.
    pub xfout: [u8; 2],
    /// `cutoff` (Hz), without which it has no filter; `resonance` (dB); `fil_type`; `fil_gain` (dB, the shelves' and
    /// the peak's).
    pub cutoff: Option<f32>,
    pub resonance: f32,
    pub fil_type: Option<(FilterShape, u8)>,
    pub fil_gain: f32,
    /// `fil_keytrack` (cents per key from `fil_keycenter`) and `fil_veltrack` (cents at velocity 127).
    pub fil_keytrack: f32,
    pub fil_keycenter: i32,
    pub fil_veltrack: f32,
    /// `pitch_veltrack`: cents at velocity 127.
    pub pitch_veltrack: f32,
    /// `amp_keytrack`: dB per key from `amp_keycenter`.
    pub amp_keytrack: f32,
    pub amp_keycenter: i32,
    /// `amp_random` (dB, 0 … x), `pitch_random` (cents, ±x), `fil_random` (cents, ±x), `pan_random` (±x).
    pub random: [f32; 4],
    /// `ampeg_start`: the level the attack starts from, percent.
    pub ampeg_start: f32,
    /// `ampeg_vel2delay` … `_vel2release`: added to [`Self::ampeg`] at velocity 127.
    pub ampeg_vel2: [f32; 6],
    /// `ampeg_*ccN`: the stage (as [`Self::ampeg`]), CC N, and what it adds at CC 127.
    pub ampeg_cc: Vec<(usize, u8, f32)>,
    /// `bend_up`, `bend_down`: cents at the bend's ends.
    pub bend: [f32; 2],
    /// `lobend`, `hibend`: the bend (−8192…8192) a note-on needs.
    pub bends: [i32; 2],
    /// `polyphony`, `note_polyphony`: the voices its `group` (of one key) keeps; a new one stops the oldest past it.
    pub polyphony: Option<u32>,
    pub note_polyphony: Option<u32>,
    /// `delay`: seconds before it plays.
    pub delay: f32,
    /// `rt_decay`: a release trigger's fall, dB per second since its note-on.
    pub rt_decay: f32,
    /// `sw_hilast`: with it, [`Self::sw_last`] (`sw_lolast`) is the low end of a range of keyswitches.
    pub sw_hilast: Option<i32>,
    /// `sw_down`, `sw_up`: a key that must be held, or up; `sw_previous`: the note-on before.
    pub sw_down: Option<i32>,
    pub sw_up: Option<i32>,
    pub sw_previous: Option<i32>,
    /// `loccN`, `hiccN` (`lohdccN`, `hihdccN`): the CCs a note-on needs, their ranges 0…1.
    pub cc_ranges: Vec<(u8, [f32; 2])>,
    /// `xfin_loccN` … `xfout_hiccN`: a crossfade by CC N, in over the first range, out over the second (0…1).
    pub xf_cc: Vec<(u8, [f32; 2], [f32; 2])>,
    pub routes: Vec<Route>,
    pub lfos: Vec<Lfo>,
    pub egs: Vec<Eg>,
}

impl Default for SfzRegion {
    /// The format's defaults.
    fn default() -> Self {
        Self {
            sample: String::new(),
            keys: [0, 127],
            vels: [1, 127],
            pitch_keycenter: 60,
            pitch_keytrack: 100.0,
            transpose: 0,
            tune: 0.0,
            volume: 0.0,
            amplitude: 100.0,
            pan: 0.0,
            amp_veltrack: 100.0,
            amp_velcurve: Vec::new(),
            offset: 0,
            end: None,
            loop_mode: None,
            loop_start: None,
            loop_end: None,
            ampeg: [0.0, 0.0, 0.0, 0.0, 100.0, 0.001],
            trigger: Trigger::Attack,
            group: 0,
            off_by: None,
            off_normal: false,
            seq_length: 1,
            seq_position: 1,
            rand: [0.0, 1.0],
            sw_keys: [0, 127],
            sw_last: None,
            sw_default: None,
            xfin: [0, 0],
            xfout: [127, 127],
            cutoff: None,
            resonance: 0.0,
            fil_type: Some((FilterShape::LowPass, 2)),
            fil_gain: 0.0,
            fil_keytrack: 0.0,
            fil_keycenter: 60,
            fil_veltrack: 0.0,
            pitch_veltrack: 0.0,
            amp_keytrack: 0.0,
            amp_keycenter: 60,
            random: [0.0; 4],
            ampeg_start: 0.0,
            ampeg_vel2: [0.0; 6],
            ampeg_cc: Vec::new(),
            bend: [200.0, -200.0],
            bends: [-8192, 8192],
            polyphony: None,
            note_polyphony: None,
            delay: 0.0,
            rt_decay: 0.0,
            sw_hilast: None,
            sw_down: None,
            sw_up: None,
            sw_previous: None,
            cc_ranges: Vec::new(),
            xf_cc: Vec::new(),
            routes: Vec::new(),
            lfos: Vec::new(),
            egs: Vec::new(),
        }
    }
}

impl SfzRegion {
    /// This region playing `other`'s sample instead of its own: its own keys, velocities, level, envelope and rules,
    /// `other`'s sample, pitch and sample range, so that it sounds at the right pitch.
    pub fn playing(&self, other: &SfzRegion) -> Self {
        Self {
            sample: other.sample.clone(),
            pitch_keycenter: other.pitch_keycenter,
            pitch_keytrack: other.pitch_keytrack,
            transpose: other.transpose,
            tune: other.tune,
            offset: other.offset,
            end: other.end,
            loop_mode: other.loop_mode,
            loop_start: other.loop_start,
            loop_end: other.loop_end,
            ..self.clone()
        }
    }

    /// Sets opcode `name` to `value`; false for an opcode this player does not play (or a value it cannot read).
    fn set(&mut self, name: &str, value: &str) -> bool {
        match name {
            "sample" => self.sample = value.replace('\\', "/"),
            "loop_mode" | "loopmode" => {
                self.loop_mode = Some(match value {
                    "no_loop" => LoopMode::NoLoop,
                    "one_shot" => LoopMode::OneShot,
                    "loop_continuous" => LoopMode::Continuous,
                    "loop_sustain" => LoopMode::Sustain,
                    _ => return false,
                })
            }
            "trigger" => {
                self.trigger = match value {
                    "attack" => Trigger::Attack,
                    "release" => Trigger::Release,
                    "first" => Trigger::First,
                    "legato" => Trigger::Legato,
                    _ => return false,
                }
            }
            "off_mode" => {
                self.off_normal = match value {
                    "fast" => false,
                    "normal" => true,
                    _ => return false,
                }
            }
            "fil_type" => {
                self.fil_type = filter_type(value);
                return self.fil_type.is_some();
            }
            // Notes reach the player without a MIDI channel.
            "lochan" | "hichan" => {}
            _ => return self.set_number(name, value),
        }
        true
    }

    fn set_number(&mut self, name: &str, value: &str) -> bool {
        let Some(x) = number(value).or_else(|| note(value)) else {
            return false;
        };
        let key = x.round().clamp(-1.0, 127.0) as i32;
        let byte = x.round().clamp(0.0, 127.0) as u8;
        let frames = x.max(0.0) as u64;
        let within = |lo: f64, hi: f64| x.clamp(lo, hi) as f32;
        if let Some(velocity) = name.strip_prefix("amp_velcurve_") {
            let Some(velocity) = velocity.parse().ok().filter(|v| *v <= 127) else {
                return false;
            };
            self.amp_velcurve.push((velocity, within(0.0, 1.0)));
            return true;
        }
        match name {
            "lokey" => self.keys[0] = key,
            "hikey" => self.keys[1] = key,
            "key" => (self.keys, self.pitch_keycenter) = ([key, key], key),
            "pitch_keycenter" => self.pitch_keycenter = key,
            "pitch_keytrack" | "tune_keytrack" => self.pitch_keytrack = within(-1200.0, 1200.0),
            "lovel" => self.vels[0] = byte,
            "hivel" => self.vels[1] = byte,
            "transpose" => self.transpose = x.round().clamp(-127.0, 127.0) as i32,
            "tune" | "pitch" => self.tune = within(-9600.0, 9600.0),
            "volume" => self.volume = within(-144.0, 6.0),
            "amplitude" => self.amplitude = within(0.0, 100.0),
            "pan" => self.pan = within(-100.0, 100.0),
            "amp_veltrack" => self.amp_veltrack = within(-100.0, 100.0),
            "offset" => self.offset = frames,
            "end" => self.end = Some(x as i64),
            "loop_start" | "loopstart" => self.loop_start = Some(frames),
            "loop_end" | "loopend" => self.loop_end = Some(frames),
            "ampeg_delay" | "amp_delay" => self.ampeg[0] = within(0.0, 100.0),
            "ampeg_attack" | "amp_attack" => self.ampeg[1] = within(0.0, 100.0),
            "ampeg_hold" | "amp_hold" => self.ampeg[2] = within(0.0, 100.0),
            "ampeg_decay" | "amp_decay" => self.ampeg[3] = within(0.0, 100.0),
            "ampeg_sustain" | "amp_sustain" => self.ampeg[4] = within(0.0, 100.0),
            "ampeg_release" | "amp_release" => self.ampeg[5] = within(0.0, 100.0),
            "ampeg_start" => self.ampeg_start = within(0.0, 100.0),
            "group" | "polyphony_group" => self.group = x as i64,
            "off_by" => self.off_by = Some(x as i64),
            "seq_length" => self.seq_length = x.round().clamp(1.0, 100.0) as u32,
            "seq_position" => self.seq_position = x.round().clamp(1.0, 100.0) as u32,
            "lorand" => self.rand[0] = within(0.0, 1.0),
            "hirand" => self.rand[1] = within(0.0, 1.0),
            "sw_lokey" => self.sw_keys[0] = key,
            "sw_hikey" => self.sw_keys[1] = key,
            "sw_last" | "sw_lolast" => self.sw_last = Some(key),
            "sw_hilast" => self.sw_hilast = Some(key),
            "sw_down" => self.sw_down = Some(key),
            "sw_up" => self.sw_up = Some(key),
            "sw_previous" => self.sw_previous = Some(key),
            "sw_default" => self.sw_default = Some(key),
            "xfin_lovel" => self.xfin[0] = byte,
            "xfin_hivel" => self.xfin[1] = byte,
            "xfout_lovel" => self.xfout[0] = byte,
            "xfout_hivel" => self.xfout[1] = byte,
            "cutoff" => self.cutoff = Some(within(0.0, 100_000.0)),
            "resonance" => self.resonance = within(0.0, 40.0),
            "fil_gain" => self.fil_gain = within(-96.0, 24.0),
            "fil_keytrack" => self.fil_keytrack = within(-1200.0, 1200.0),
            "fil_keycenter" => self.fil_keycenter = key,
            "fil_veltrack" => self.fil_veltrack = within(-9600.0, 9600.0),
            "pitch_veltrack" | "tune_veltrack" => self.pitch_veltrack = within(-9600.0, 9600.0),
            "amp_keytrack" => self.amp_keytrack = within(-96.0, 12.0),
            "amp_keycenter" => self.amp_keycenter = key,
            "amp_random" | "gain_random" => self.random[0] = within(0.0, 24.0),
            "pitch_random" | "tune_random" => self.random[1] = within(0.0, 9600.0),
            "fil_random" | "cutoff_random" => self.random[2] = within(0.0, 9600.0),
            "pan_random" => self.random[3] = within(-100.0, 100.0).abs(),
            "bend_up" => self.bend[0] = within(-9600.0, 9600.0),
            "bend_down" => self.bend[1] = within(-9600.0, 9600.0),
            "lobend" => self.bends[0] = x.round().clamp(-8192.0, 8192.0) as i32,
            "hibend" => self.bends[1] = x.round().clamp(-8192.0, 8192.0) as i32,
            "polyphony" => self.polyphony = Some(x.round().clamp(1.0, 512.0) as u32),
            "note_polyphony" => self.note_polyphony = Some(x.round().clamp(1.0, 512.0) as u32),
            "delay" => self.delay = within(0.0, 100.0),
            "rt_decay" => self.rt_decay = within(0.0, 200.0),
            _ => {
                let vel2 = name
                    .strip_prefix("ampeg_vel2")
                    .or(name.strip_prefix("amp_vel2"));
                if let Some(stage) = vel2.and_then(|s| STAGES.iter().position(|&n| n == s)) {
                    self.ampeg_vel2[stage] = within(-100.0, 100.0);
                    return true;
                }
                return self.set_mod(name, x as f32);
            }
        }
        true
    }

    /// Sets a modulation opcode: an LFO's or envelope's, a route's, or one by a CC. False for any other.
    fn set_mod(&mut self, name: &str, v: f32) -> bool {
        if let Some((stem, kind, cc)) = cc_opcode(name) {
            return self.set_cc(stem, kind, cc, v);
        }
        let had = (self.lfos.len(), self.egs.len());
        let set = self.set_unit(name, v);
        if !set {
            // A unit this opcode made, which it then did not set, goes again.
            self.lfos.truncate(had.0);
            self.egs.truncate(had.1);
        }
        set
    }

    fn set_unit(&mut self, name: &str, v: f32) -> bool {
        let Some((source, v1, rest)) = self.unit(name) else {
            return false;
        };
        let v2 = Target::named(rest).filter(|_| v1.is_none());
        if let Some(target) = v1.filter(|_| rest == "depth").or(v2) {
            let Some(route) = self.route(source, target) else {
                return false;
            };
            route.depth = v.clamp(-12_000.0, 12_000.0);
            return true;
        }
        let time = v.clamp(0.0, 100.0);
        match source {
            Source::Lfo(i) => {
                let lfo = &mut self.lfos[i];
                match rest {
                    "freq" => lfo.freq = v.clamp(0.0, 100.0),
                    "delay" => lfo.delay = time,
                    "fade" => lfo.fade = time,
                    "phase" if v1.is_none() => lfo.phase = v.rem_euclid(1.0),
                    "wave" if v1.is_none() => lfo.wave = v.clamp(0.0, 255.0) as u8,
                    _ => return false,
                }
            }
            Source::Eg(i) => {
                let eg = &mut self.egs[i];
                if v1.is_some() {
                    let level = (v / 100.0).clamp(0.0, 1.0);
                    match rest {
                        "start" => eg.points[0].1 = level,
                        "sustain" => eg.points[3].1 = level,
                        _ => match DAHDSR.iter().position(|&s| s == rest) {
                            Some(point) => eg.points[point].0 = time,
                            None => return false,
                        },
                    }
                } else if rest == "sustain" {
                    eg.sustain = Some(v.clamp(0.0, 31.0) as usize);
                } else {
                    let (level, point) = match (numbered(rest, "time"), numbered(rest, "level")) {
                        (Some((point, "")), _) => (false, point),
                        (_, Some((point, ""))) => (true, point),
                        _ => return false,
                    };
                    let Some(point) = usize::try_from(point).ok().filter(|&p| p < 32) else {
                        return false;
                    };
                    if eg.points.len() <= point {
                        eg.points.resize(point + 1, (0.0, 0.0));
                    }
                    match level {
                        true => eg.points[point].1 = v.clamp(-1.0, 1.0),
                        false => eg.points[point].0 = time,
                    }
                }
            }
            Source::Cc(_) => return false,
        }
        true
    }

    /// Sets an opcode by CC `cc` (`stem` and `kind` as [`cc_opcode`] reads them): a note-on's range, a crossfade, a
    /// route of the CC (its depth, curve or smoothing), a CC on an LFO's or envelope's depth or an LFO's frequency, or on
    /// an amplifier envelope stage.
    fn set_cc(&mut self, stem: &str, kind: &str, cc: u8, v: f32) -> bool {
        if let Some(target) = Target::named(stem) {
            let Some(route) = self.route(Source::Cc(cc), target) else {
                return false;
            };
            match kind {
                "_on" => route.depth = v.clamp(-12_000.0, 12_000.0),
                "_curve" => route.curve = v.clamp(0.0, 255.0) as u8,
                "_smooth" => route.smooth = v.clamp(0.0, 10_000.0),
                _ => return false,
            }
            return true;
        }
        if kind != "_on" {
            return false;
        }
        let midi = (v / 127.0).clamp(0.0, 1.0);
        match stem {
            "lo" | "hi" | "lohd" | "hihd" => {
                let value = if stem.ends_with("hd") {
                    v.clamp(0.0, 1.0)
                } else {
                    midi
                };
                let at = (self.cc_ranges.iter().position(|r| r.0 == cc)).unwrap_or_else(|| {
                    self.cc_ranges.push((cc, [0.0, 1.0]));
                    self.cc_ranges.len() - 1
                });
                self.cc_ranges[at].1[usize::from(stem.starts_with("hi"))] = value;
            }
            "xfin_lo" | "xfin_hi" | "xfout_lo" | "xfout_hi" => {
                let at = (self.xf_cc.iter().position(|r| r.0 == cc)).unwrap_or_else(|| {
                    self.xf_cc.push((cc, [0.0, 0.0], [1.0, 1.0]));
                    self.xf_cc.len() - 1
                });
                let (_, fade_in, fade_out) = &mut self.xf_cc[at];
                let range = if stem.starts_with("xfin") {
                    fade_in
                } else {
                    fade_out
                };
                range[usize::from(stem.ends_with("hi"))] = midi;
            }
            _ => {
                if let Some(stage) = stem.strip_prefix("ampeg_")
                    && let Some(stage) = STAGES.iter().position(|&s| s == stage)
                {
                    let amount = v.clamp(-100.0, 100.0);
                    let at = self.ampeg_cc.iter().position(|e| (e.0, e.1) == (stage, cc));
                    match at {
                        Some(at) => self.ampeg_cc[at].2 = amount,
                        None if self.ampeg_cc.len() < MAX_ROUTES => {
                            self.ampeg_cc.push((stage, cc, amount))
                        }
                        None => return false,
                    }
                    return true;
                }
                let had = (self.lfos.len(), self.egs.len());
                let set = self.set_unit_cc(stem, cc, v);
                if !set {
                    self.lfos.truncate(had.0);
                    self.egs.truncate(had.1);
                }
                return set;
            }
        }
        true
    }

    /// A CC on an LFO's or envelope's route depth (`pitchlfo_depthccN`, `lfo01_cutoff_onccN`) or an LFO's frequency.
    fn set_unit_cc(&mut self, stem: &str, cc: u8, v: f32) -> bool {
        let Some((source, v1, rest)) = self.unit(stem) else {
            return false;
        };
        let v2 = Target::named(rest).filter(|_| v1.is_none());
        if let Some(target) = v1.filter(|_| rest == "depth").or(v2) {
            let Some(route) = self.route(source, target) else {
                return false;
            };
            route.depth_cc = Some((cc, v.clamp(-12_000.0, 12_000.0)));
            return true;
        }
        match (source, rest) {
            (Source::Lfo(i), "freq") => self.lfos[i].freq_cc = Some((cc, v.clamp(-100.0, 100.0))),
            _ => return false,
        }
        true
    }

    /// The LFO or envelope opcode `name` sets (made when new, while there is room), its v1 target, and the rest of the
    /// name: `pitchlfo_freq` (LFO, pitch, `freq`), `eg02_time1` (envelope, none, `time1`).
    fn unit<'a>(&mut self, name: &'a str) -> Option<(Source, Option<Target>, &'a str)> {
        for (i, (prefix, target)) in V1_LFOS.iter().enumerate() {
            if let Some(rest) = name.strip_prefix(prefix) {
                return Some((Source::Lfo(self.lfo(V1 + i as u8)?), Some(*target), rest));
            }
        }
        for (i, (prefix, target)) in V1_EGS.iter().enumerate() {
            if let Some(rest) = name.strip_prefix(prefix) {
                // `fil_` and `pitch_` are `fileg_`'s and `pitcheg_`'s.
                return Some((Source::Eg(self.eg(V1 + i as u8 % 2)?), Some(*target), rest));
            }
        }
        let numbered = |prefix| {
            let (n, rest) = numbered(name, prefix)?;
            let id = u8::try_from(n).ok().filter(|n| (1..V1).contains(n))?;
            Some((id, rest.strip_prefix('_')?))
        };
        if let Some((id, rest)) = numbered("lfo") {
            return Some((Source::Lfo(self.lfo(id)?), None, rest));
        }
        let (id, rest) = numbered("eg")?;
        Some((Source::Eg(self.eg(id)?), None, rest))
    }

    /// The index of LFO `id`, made when new; `None` when [`MAX_LFOS`] are there.
    fn lfo(&mut self, id: u8) -> Option<usize> {
        if let Some(at) = self.lfos.iter().position(|l| l.id == id) {
            return Some(at);
        }
        (self.lfos.len() < MAX_LFOS).then(|| {
            self.lfos.push(Lfo {
                freq: 0.0,
                delay: 0.0,
                fade: 0.0,
                phase: 0.0,
                wave: 1,
                freq_cc: None,
                id,
            });
            self.lfos.len() - 1
        })
    }

    /// The index of envelope `id`, made when new (v1's as its DAHDSR at the format's defaults); `None` when
    /// [`MAX_EGS`] are there.
    fn eg(&mut self, id: u8) -> Option<usize> {
        if let Some(at) = self.egs.iter().position(|e| e.id == id) {
            return Some(at);
        }
        let v1 = id >= V1;
        (self.egs.len() < MAX_EGS).then(|| {
            self.egs.push(Eg {
                points: match v1 {
                    true => vec![(0.0, 0.0), (0.0, 1.0), (0.0, 1.0), (0.0, 0.0), (0.0, 0.0)],
                    false => Vec::new(),
                },
                sustain: v1.then_some(3),
                id,
            });
            self.egs.len() - 1
        })
    }

    /// The route of `source` to `target`, made when new; `None` when [`MAX_ROUTES`] are there.
    fn route(&mut self, source: Source, target: Target) -> Option<&mut Route> {
        let at = self
            .routes
            .iter()
            .position(|r| r.source == source && r.target == target);
        let at = match at {
            Some(at) => at,
            None if self.routes.len() < MAX_ROUTES => {
                self.routes.push(Route {
                    source,
                    target,
                    depth: 0.0,
                    curve: 0,
                    smooth: 0.0,
                    depth_cc: None,
                });
                self.routes.len() - 1
            }
            None => return None,
        };
        Some(&mut self.routes[at])
    }

    /// Moves every key it names by `by` semitones (`note_offset`, `octave_offset`).
    fn shift(&mut self, by: i32) {
        let moved = |key: i32| {
            if key < 0 {
                key
            } else {
                (key + by).clamp(0, 127)
            }
        };
        self.keys = self.keys.map(moved);
        self.sw_keys = self.sw_keys.map(moved);
        self.pitch_keycenter = moved(self.pitch_keycenter);
        self.fil_keycenter = moved(self.fil_keycenter);
        self.amp_keycenter = moved(self.amp_keycenter);
        for key in [
            &mut self.sw_last,
            &mut self.sw_hilast,
            &mut self.sw_default,
            &mut self.sw_down,
            &mut self.sw_up,
            &mut self.sw_previous,
        ] {
            *key = key.map(moved);
        }
    }
}

/// The amplifier envelope's stages, as [`SfzRegion::ampeg`] holds them.
const STAGES: [&str; 6] = ["delay", "attack", "hold", "decay", "sustain", "release"];

/// `name` as `prefix`, a number and the rest: the number and the rest (`eg02_time1`, `eg` → 2, `_time1`).
fn numbered<'a>(name: &'a str, prefix: &str) -> Option<(u32, &'a str)> {
    let rest = name.strip_prefix(prefix)?;
    let digits = rest.bytes().take_while(u8::is_ascii_digit).count();
    Some((rest[..digits].parse().ok()?, &rest[digits..]))
}

/// `name` as a head and the number that ends it (`v064` → `v`, 64).
fn trailing(name: &str) -> Option<(&str, u32)> {
    let at = name.len() - name.bytes().rev().take_while(u8::is_ascii_digit).count();
    Some((&name[..at], name[at..].parse().ok()?))
}

/// An opcode by a CC as its stem, its kind and the CC: `cutoff_oncc74` (`cutoff`, `_on`, 74), `pan_curvecc10`
/// (`_curve`), `volume_smoothcc7` (`_smooth`), `_stepccN` (`_step`). v1's `XccN` and `X_ccN` are `_on`s, and so are
/// `loccN` (`lo`), `set_ccN` (`set`) and `label_ccN` (`label`).
fn cc_opcode(name: &str) -> Option<(&str, &str, u8)> {
    let (head, n) = trailing(name)?;
    let head = head.strip_suffix("cc")?;
    let cc = u8::try_from(n).ok().filter(|&cc| cc < 128)?;
    let kinds = ["_on", "_curve", "_smooth", "_step"].into_iter();
    let found = kinds
        .clone()
        .find_map(|kind| Some((head.strip_suffix(kind)?, kind)));
    let (stem, kind) = found.unwrap_or((head.strip_suffix('_').unwrap_or(head), "_on"));
    Some((stem, kind, cc))
}

/// A number, finite.
fn number(value: &str) -> Option<f64> {
    value.parse::<f64>().ok().filter(|x| x.is_finite())
}

/// A note name, `c4` (60), `c#4`, `db4`, `c-1` (0), as a MIDI note.
pub fn note(value: &str) -> Option<f64> {
    let mut chars = value.chars();
    let class = match chars.next()?.to_ascii_lowercase() {
        'c' => 0,
        'd' => 2,
        'e' => 4,
        'f' => 5,
        'g' => 7,
        'a' => 9,
        'b' => 11,
        _ => return None,
    };
    let rest = chars.as_str();
    let (accidental, octave) = match (rest.strip_prefix('#'), rest.strip_prefix('b')) {
        (Some(octave), _) => (1, octave),
        (_, Some(octave)) if !octave.is_empty() => (-1, octave),
        _ => (0, rest),
    };
    let octave: i32 = octave.parse().ok().filter(|o: &i32| o.abs() <= 20)?;
    Some(f64::from(12 * (octave + 1) + class + accidental))
}

/// A parsed SFZ.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct Sfz {
    /// In file order, at most [`MAX_REGIONS`]; a region without a sample is none.
    pub regions: Vec<SfzRegion>,
    /// The opcodes this player does not play, by name, with the number of regions they were given to (a `<control>`
    /// one: the times it was met).
    pub skipped: BTreeMap<String, usize>,
    pub control: SfzControl,
}

/// What the whole instrument shares: its `<control>` and `<curve>`s.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct SfzControl {
    /// `set_ccN` (`set_hdccN`): CC N's value when the instrument loads, 0…1.
    pub set_cc: BTreeMap<u8, f32>,
    /// `label_ccN`: the controls it names, CC N and its name, in the order written.
    pub labels: Vec<(u8, String)>,
    /// The `<curve>`s by `curve_index`: their `vNNN` points (N 0…127, the value −1…1), in the order written.
    pub curves: BTreeMap<u8, Vec<(u8, f32)>>,
}

/// Parses the SFZ `text`, reading each `#include` through `include` (its path as written, `/`-separated: the caller
/// resolves it from the SFZ's folder and keeps it inside the library; `None` skips it).
pub fn parse(text: &str, include: &dyn Fn(&str) -> Option<String>) -> Sfz {
    let mut parser = Parser {
        include,
        text_left: MAX_TEXT,
        defines: Vec::new(),
        scope: Scope::None,
        levels: Default::default(),
        region: None,
        default_path: String::new(),
        offset: [0, 0],
        curve: None,
        sfz: Sfz::default(),
    };
    parser.file(text, 0);
    parser.end_header();
    parser.sfz
}

#[derive(Clone, Copy, PartialEq, Eq)]
enum Scope {
    /// Before any header, and in a header whose opcodes are dropped.
    None,
    Control,
    Curve,
    /// `<global>`, `<master>`, `<group>`: an index into [`Parser::levels`].
    Level(usize),
    Region,
}

type Opcodes = Vec<(String, String)>;
/// A curve's `vNNN` points.
type Points = Vec<(u8, f32)>;

struct Parser<'a> {
    include: &'a dyn Fn(&str) -> Option<String>,
    text_left: usize,
    /// `$NAME` and value, the longest names first so that none replaces part of a longer one.
    defines: Vec<(String, String)>,
    scope: Scope,
    /// The opcodes of the open `<global>`, `<master>` and `<group>`.
    levels: [Opcodes; 3],
    /// The opcodes of the open `<region>`.
    region: Option<Opcodes>,
    default_path: String,
    /// `note_offset`, `octave_offset`.
    offset: [i32; 2],
    /// The open `<curve>`: its `curve_index` and points.
    curve: Option<(Option<u8>, Points)>,
    sfz: Sfz,
}

impl Parser<'_> {
    fn file(&mut self, text: &str, depth: usize) {
        let Some(left) = self.text_left.checked_sub(text.len()) else {
            return;
        };
        self.text_left = left;
        for line in without_comments(text).lines() {
            let line = line.trim();
            if let Some(define) = line.strip_prefix("#define") {
                let (name, value) = define
                    .trim()
                    .split_once(char::is_whitespace)
                    .unwrap_or_default();
                if name.len() > 1 && name.starts_with('$') {
                    let value = self.substitute(value.trim());
                    self.defines.retain(|(n, _)| n != name);
                    self.defines.push((name.to_owned(), value));
                    self.defines
                        .sort_by_key(|(n, _)| std::cmp::Reverse(n.len()));
                }
            } else if let Some(name) = line.strip_prefix("#include") {
                let name = self.substitute(name.trim());
                let name = name.trim_matches('"').replace('\\', "/");
                if depth < MAX_INCLUDE_DEPTH
                    && let Some(text) = (self.include)(&name)
                {
                    self.file(&text, depth + 1);
                }
            } else {
                let line = self.substitute(line);
                self.line(&line);
            }
        }
    }

    fn substitute(&self, line: &str) -> String {
        let mut line = line.to_owned();
        if line.contains('$') {
            for (name, value) in &self.defines {
                line = line.replace(name, value);
            }
        }
        line
    }

    /// The headers and opcodes of one line.
    fn line(&mut self, mut rest: &str) {
        loop {
            rest = rest.trim_start();
            if let Some(header) = rest.strip_prefix('<') {
                let Some((name, after)) = header.split_once('>') else {
                    return;
                };
                self.header(name.trim());
                rest = after;
                continue;
            }
            let Some((name, after)) = rest.split_once('=') else {
                return;
            };
            if name.is_empty() || name.contains(|c: char| c.is_whitespace() || c == '<') {
                // Not an opcode: on to the next word or header.
                let at = rest
                    .find(|c: char| c.is_whitespace() || c == '<')
                    .unwrap_or(rest.len());
                rest = &rest[at.max(1).min(rest.len())..];
                continue;
            }
            let end = value_end(after);
            let value = after[..end].trim();
            let label = name.starts_with("label_cc") || name.ends_with("_label");
            let value = match name {
                "sample" | "default_path" => value,
                _ if label => value,
                _ => value.split_whitespace().next().unwrap_or_default(),
            };
            self.opcode(name, value);
            rest = &after[end..];
        }
    }

    fn header(&mut self, name: &str) {
        self.end_header();
        self.scope = match name {
            "control" => Scope::Control,
            "curve" => {
                self.curve = Some((None, Vec::new()));
                Scope::Curve
            }
            "global" => Scope::Level(0),
            "master" => Scope::Level(1),
            "group" => Scope::Level(2),
            "region" => {
                self.region = Some(Vec::new());
                Scope::Region
            }
            _ => Scope::None,
        };
        if let Scope::Level(level) = self.scope {
            self.levels[level..].iter_mut().for_each(Vec::clear);
        }
    }

    fn opcode(&mut self, name: &str, value: &str) {
        let pair = (name.to_owned(), value.to_owned());
        match self.scope {
            Scope::None => {}
            Scope::Control => match name {
                "default_path" => self.default_path = value.replace('\\', "/"),
                "note_offset" => self.offset[0] = clamped(value, 127),
                "octave_offset" => self.offset[1] = clamped(value, 10),
                _ => {
                    if !self.control(name, value) {
                        *self.sfz.skipped.entry(pair.0).or_default() += 1;
                    }
                }
            },
            Scope::Curve => {
                if !self.curve_point(name, value) {
                    *self.sfz.skipped.entry(pair.0).or_default() += 1;
                }
            }
            Scope::Level(level) => self.levels[level].push(pair),
            Scope::Region => self.region.get_or_insert_default().push(pair),
        }
    }

    /// A `<control>` CC opcode: `set_ccN`, `set_hdccN`, `label_ccN`.
    fn control(&mut self, name: &str, value: &str) -> bool {
        let Some((stem, "_on", cc)) = cc_opcode(name) else {
            return false;
        };
        let control = &mut self.sfz.control;
        match (stem, number(value)) {
            ("set" | "set_hd", Some(x)) => {
                let x = if stem == "set" { x / 127.0 } else { x };
                control.set_cc.insert(cc, x.clamp(0.0, 1.0) as f32);
            }
            ("label", _) => {
                control.labels.retain(|(c, _)| *c != cc);
                control.labels.push((cc, value.to_owned()));
            }
            _ => return false,
        }
        true
    }

    /// A `<curve>` opcode: `curve_index`, or a point `vNNN`.
    fn curve_point(&mut self, name: &str, value: &str) -> bool {
        let (Some((index, points)), Some(x)) = (self.curve.as_mut(), number(value)) else {
            return false;
        };
        match trailing(name) {
            _ if name == "curve_index" => *index = Some(x.clamp(0.0, 255.0) as u8),
            Some(("v", n @ 0..=127)) => points.push((n as u8, x.clamp(-1.0, 1.0) as f32)),
            _ => return false,
        }
        true
    }

    /// Ends the open region or curve.
    fn end_header(&mut self) {
        if let Some((Some(index), points)) = self.curve.take() {
            self.sfz.control.curves.insert(index, points);
        }
        self.end_region();
    }

    /// Ends the open region, if any: its opcodes over its levels', in order.
    fn end_region(&mut self) {
        let Some(own) = self.region.take() else {
            return;
        };
        let mut region = SfzRegion::default();
        let mut skipped = Vec::new();
        for (name, value) in self.levels.iter().flatten().chain(&own) {
            if !region.set(name, value) {
                skipped.push(name);
            }
        }
        if region.sample.is_empty() || self.sfz.regions.len() == MAX_REGIONS {
            return;
        }
        for name in skipped {
            *self.sfz.skipped.entry(name.clone()).or_default() += 1;
        }
        if !region.sample.starts_with('*') {
            region.sample = format!("{}{}", self.default_path, region.sample);
        }
        region.shift(self.offset[0] + 12 * self.offset[1]);
        self.sfz.regions.push(region);
    }
}

/// An integer within ±`max`, 0 when it is none.
fn clamped(value: &str, max: i32) -> i32 {
    number(value).map_or(0, |x| {
        x.round().clamp(-f64::from(max), f64::from(max)) as i32
    })
}

/// `text` with its comments as spaces: `//` to the end of the line, `/* */` keeping its line breaks.
fn without_comments(text: &str) -> String {
    let mut out = String::with_capacity(text.len());
    let mut rest = text;
    while let Some(at) = rest.find('/') {
        let (before, from) = rest.split_at(at);
        out.push_str(before);
        if from.starts_with("//") {
            rest = from.find('\n').map_or("", |n| &from[n..]);
        } else if let Some(body) = from.strip_prefix("/*") {
            let (comment, after) = body.split_once("*/").unwrap_or((body, ""));
            out.push(' ');
            out.extend(comment.matches('\n'));
            rest = after;
        } else {
            out.push('/');
            rest = &from[1..];
        }
    }
    out.push_str(rest);
    out
}

/// Where the value that starts `after` ends: at a header (`<`), or at the space before the next `name=`.
fn value_end(after: &str) -> usize {
    let bytes = after.as_bytes();
    let word = |b: u8| b.is_ascii_alphanumeric() || b == b'_' || b == b'$';
    let mut i = 0;
    while i < bytes.len() {
        if bytes[i] == b'<' {
            return i;
        }
        if !bytes[i].is_ascii_whitespace() {
            i += 1;
            continue;
        }
        let start = i + bytes[i..]
            .iter()
            .take_while(|b| b.is_ascii_whitespace())
            .count();
        let name = bytes[start..].iter().take_while(|&&b| word(b)).count();
        if name > 0 && bytes.get(start + name) == Some(&b'=') {
            return i;
        }
        i = start;
    }
    bytes.len()
}

/// Frames of one cycle of a built-in wave.
pub const CYCLE: usize = 2048;

/// The built-in sample `name` (`*sine`, `*saw`, `*square`, `*triangle`, `*noise`, `*silence`), looped whole. A wave is
/// one cycle of [`CYCLE`] frames, which a region plays at its key center's pitch; the noise is a second of it at
/// 48 kHz. `None` for any other name.
// ponytail: naive waves, which alias at high keys; band-limit them (mip tables) if `*saw` up high ever matters.
pub fn builtin(name: &str) -> Option<SampleData> {
    let phase = |i: usize| i as f32 / CYCLE as f32;
    let wave: Vec<f32> = match name {
        "*sine" => (0..CYCLE)
            .map(|i| (std::f32::consts::TAU * phase(i)).sin())
            .collect(),
        "*saw" => (0..CYCLE).map(|i| 2.0 * phase(i) - 1.0).collect(),
        "*square" => (0..CYCLE)
            .map(|i| if phase(i) < 0.5 { 1.0 } else { -1.0 })
            .collect(),
        "*triangle" | "*tri" => (0..CYCLE)
            .map(|i| 1.0 - 4.0 * (phase(i) - 0.5).abs())
            .collect(),
        "*noise" => {
            let mut x = 0x2545_F491_u32;
            let mut next = move || {
                x ^= x << 13;
                x ^= x >> 17;
                x ^= x << 5;
                x as f32 / u32::MAX as f32 * 2.0 - 1.0
            };
            (0..48_000).map(|_| next()).collect()
        }
        "*silence" => vec![0.0; CYCLE],
        _ => return None,
    };
    let last = wave.len() as u32 - 1;
    Some(SampleData {
        sample_rate: 48_000,
        left: wave.into(),
        right: None,
        meta: SampleMeta {
            loop_frames: Some((0, last)),
            ..SampleMeta::default()
        },
    })
}

/// Whether the built-in sample `name` is a wave, played at its region's pitch: all but the noise and the silence.
pub fn pitched(name: &str) -> bool {
    name.starts_with('*') && !matches!(name, "*noise" | "*silence")
}
