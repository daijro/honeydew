//! SFZ (sfzformat.com), the text instrument format, read for the SoundFont Player: v1's mapping, tuning, level, loop,
//! envelope and trigger opcodes and the common v2 ones ([`SfzRegion`]).
//!
//! [`parse`] is pure: text in, and an `#include` reader; regions out. `<control>`, `<global>`, `<master>`, `<group>` and
//! `<region>` nest as the format says (a region starts from its group, which starts from its master, then its global;
//! a new header of a level drops the levels below it). `<curve>`, `<effect>`, `<midi>`, `<sample>` and unknown headers
//! are read past, their opcodes dropped. `#define $NAME value` substitutes `$NAME` in later lines; `#include "file"` reads
//! the file through the reader (depth [`MAX_INCLUDE_DEPTH`]; the reader keeps it inside the library). `//` and `/* */`
//! are comments. A `sample=` or `default_path=` value runs to the end of its line or the next `opcode=`, so paths may
//! hold spaces; other values are one word. Opcodes this player does not play are counted in [`Sfz::skipped`], never an
//! error. Input is hostile: regions, include depth and the text read are bounded.

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
            "pitch_keytrack" => self.pitch_keytrack = within(-1200.0, 1200.0),
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
            "ampeg_delay" => self.ampeg[0] = within(0.0, 100.0),
            "ampeg_attack" => self.ampeg[1] = within(0.0, 100.0),
            "ampeg_hold" => self.ampeg[2] = within(0.0, 100.0),
            "ampeg_decay" => self.ampeg[3] = within(0.0, 100.0),
            "ampeg_sustain" => self.ampeg[4] = within(0.0, 100.0),
            "ampeg_release" => self.ampeg[5] = within(0.0, 100.0),
            "group" => self.group = x as i64,
            "off_by" => self.off_by = Some(x as i64),
            "seq_length" => self.seq_length = x.round().clamp(1.0, 100.0) as u32,
            "seq_position" => self.seq_position = x.round().clamp(1.0, 100.0) as u32,
            "lorand" => self.rand[0] = within(0.0, 1.0),
            "hirand" => self.rand[1] = within(0.0, 1.0),
            "sw_lokey" => self.sw_keys[0] = key,
            "sw_hikey" => self.sw_keys[1] = key,
            "sw_last" => self.sw_last = Some(key),
            "sw_default" => self.sw_default = Some(key),
            "xfin_lovel" => self.xfin[0] = byte,
            "xfin_hivel" => self.xfin[1] = byte,
            "xfout_lovel" => self.xfout[0] = byte,
            "xfout_hivel" => self.xfout[1] = byte,
            _ => return false,
        }
        true
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
        self.sw_last = self.sw_last.map(moved);
        self.sw_default = self.sw_default.map(moved);
    }
}

/// A number, finite.
fn number(value: &str) -> Option<f64> {
    value.parse::<f64>().ok().filter(|x| x.is_finite())
}

/// A note name, `c4` (60), `c#4`, `db4`, `c-1` (0), as a MIDI note.
fn note(value: &str) -> Option<f64> {
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
        sfz: Sfz::default(),
    };
    parser.file(text, 0);
    parser.end_region();
    parser.sfz
}

#[derive(Clone, Copy, PartialEq, Eq)]
enum Scope {
    /// Before any header, and in a header whose opcodes are dropped.
    None,
    Control,
    /// `<global>`, `<master>`, `<group>`: an index into [`Parser::levels`].
    Level(usize),
    Region,
}

type Opcodes = Vec<(String, String)>;

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
            let value = match name {
                "sample" | "default_path" => value,
                _ => value.split_whitespace().next().unwrap_or_default(),
            };
            self.opcode(name, value);
            rest = &after[end..];
        }
    }

    fn header(&mut self, name: &str) {
        self.end_region();
        self.scope = match name {
            "control" => Scope::Control,
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
                _ => *self.sfz.skipped.entry(pair.0).or_default() += 1,
            },
            Scope::Level(level) => self.levels[level].push(pair),
            Scope::Region => self.region.get_or_insert_default().push(pair),
        }
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
