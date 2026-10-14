//! MIDI import: [`scan`] for the import dialog and the one-channel shortcut, and [`import`] for its three entry
//! points.

use std::collections::HashMap;

use super::codec::{self, SmfEvent};
use crate::midi::MidiMsg;
use crate::song::points::{self, Point, PointKey};
use crate::song::{
    AutomationTarget, ChannelKind, Clip, Event, Lane, LaneKind, PPQ, Song, SynthKind,
};

/// What every failure shows; the `Err` string is the detail.
pub const LOAD_FAILED: &str = "Could not load MIDI file. It may be corrupted.";

/// What the import dialog needs from a file.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct Scan {
    /// Bit c: MIDI channel c (0-based) has an event that makes a channel on import: a note-on, a CC other than 127
    /// or a pitch wheel. Program changes, pressure and note-offs alone do not count.
    pub used: u16,
    /// Per MIDI channel (used or not): the names of the programs its `Cn` events select, once each in first-seen
    /// order. A used channel without one lists its default program's name (program 0, the import's default too).
    pub programs: [Vec<String>; 16],
    /// `Bn` or `En` event exists: the dialog lays out `Import automation` and `Only supported`.
    pub automation: bool,
}

/// The dialog's choices.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct ImportOpts {
    /// `Split MIDI channels`: one new channel per MIDI channel; off, everything goes into the current channel.
    pub split: bool,
    /// `Import automation`: CC lanes. The pitch wheel is imported either way.
    pub automation: bool,
    /// `Only supported`: CC 1, 7, 8, 10 and 64 only.
    pub only_supported: bool,
    /// Bit c: MIDI channel c (0-based) is imported.
    pub channels: u16,
}

impl ImportOpts {
    /// Opening a `.mid` as a song: Split on, automation on, Only supported off, every channel.
    pub const OPEN_AS_SONG: Self = Self {
        split: true,
        automation: true,
        only_supported: false,
        channels: 0xFFFF,
    };
    /// The track menu's one-channel shortcut: Split off, automation on, Only supported off.
    pub const ONE_CHANNEL: Self = Self {
        split: false,
        ..Self::OPEN_AS_SONG
    };
}

/// The CCs `Only supported` keeps.
const SUPPORTED: [u8; 5] = [1, 7, 8, 10, 64];

/// Lane names with Split on, by MIDI 1.0 controller number.
const GM_CONTROLLERS: [(u8, &str); 36] = [
    (0, "Bank Select"),
    (1, "Modulation Wheel"),
    (2, "Breath Control"),
    (4, "Foot Pedal"),
    (5, "Portamento Time"),
    (7, "Volume"),
    (8, "Balance"),
    (10, "Pan Position"),
    (11, "Expression"),
    (12, "Effect Control 1"),
    (13, "Effect Control 2"),
    (64, "Sustain Pedal"),
    (65, "Portamento"),
    (66, "Sostenuto Pedal"),
    (67, "Soft Pedal"),
    (68, "Legato Pedal"),
    (69, "Hold Pedal 2"),
    (70, "Variation"),
    (71, "Timbre"),
    (72, "Sound Release Time"),
    (73, "Sound Attack Time"),
    (91, "Reverb Level"),
    (92, "Tremolo Level"),
    (93, "Chorus Level"),
    (94, "Detune Level"),
    (95, "Phaser Level"),
    (96, "Data Button Increment"),
    (97, "Data Button Decrement"),
    (120, "All Sound Off"),
    (121, "All Controllers Off"),
    (122, "Local Keyboard On"),
    (123, "All Notes Off"),
    (124, "Omni Mode Off"),
    (125, "Omni Mode On"),
    (126, "Mono Operation"),
    (127, "Poly Mode"),
];

/// The channels, programs and controller data of `bytes`; `Err` with the reader's detail when it is no MIDI file.
pub fn scan(bytes: &[u8]) -> Result<Scan, String> {
    let smf = codec::read(bytes)?;
    let mut scan = Scan::default();
    for (_, event) in smf.tracks.iter().flatten() {
        let SmfEvent::Voice(msg) = *event else {
            continue;
        };
        let ch = channel_of(msg);
        // Used: what makes a channel on import. Programs are collected on every channel.
        match msg {
            MidiMsg::NoteOn { .. } => scan.used |= 1 << ch,
            MidiMsg::Cc { cc, .. } if cc != 127 => {
                scan.used |= 1 << ch;
                scan.automation = true;
            }
            MidiMsg::PitchBend { .. } => {
                scan.used |= 1 << ch;
                scan.automation = true;
            }
            MidiMsg::Program { program, .. } => {
                let name = program_name(ch, program);
                let names = &mut scan.programs[usize::from(ch)];
                if !names.contains(&name) {
                    names.push(name);
                }
            }
            _ => {}
        }
    }
    for ch in 0..16 {
        let names = &mut scan.programs[usize::from(ch)];
        if scan.used >> ch & 1 == 1 && names.is_empty() {
            names.push(program_name(ch, 0));
        }
    }
    Ok(scan)
}

/// Imports `bytes` into `song`: with Split on (always when `as_song`) into one new DW Sampler channel per selected MIDI
/// channel, else into channel `current`; notes on notes lanes, controllers and the pitch wheel on channel automation
/// lanes. `as_song` (opening a `.mid`) also sets the file tempo. `Err` (the detail for [`LOAD_FAILED`]) leaves the song
/// as it was: the file does not read, or `current` is no channel.
pub fn import(
    song: &mut Song,
    current: usize,
    bytes: &[u8],
    opts: &ImportOpts,
    as_song: bool,
) -> Result<(), String> {
    let smf = codec::read(bytes)?;
    if current >= song.channels.len() {
        return Err(format!("no channel {current}"));
    }
    let mut state = State {
        split: opts.split || as_song,
        current,
        made: [None; 16],
        last_notes: HashMap::new(),
        notes: Vec::new(),
        autos: Vec::new(),
        auto_index: HashMap::new(),
    };
    let division = f64::from(smf.division);
    let (mut tempo, mut program) = (None, [0u8; 16]);
    for track in &smf.tracks {
        let mut name = String::new();
        let mut new_lane = true;
        // The MIDI channels whose channel this track made.
        let mut made_here = 0u16;
        for &(tick, ref event) in track {
            let beat = tick as f64 / division;
            let msg = match event {
                SmfEvent::Name(bytes) => {
                    name = text(bytes);
                    continue;
                }
                SmfEvent::Tempo(us) => {
                    tempo = Some(60e6 / f64::from(*us));
                    continue;
                }
                // A malformed `FF 51` is a tempo parse error.
                SmfEvent::Other { kind: 0x51, .. } => {
                    tempo = Some(120.0);
                    continue;
                }
                SmfEvent::Other { .. } => continue,
                SmfEvent::Voice(msg) => *msg,
            };
            let ch = channel_of(msg);
            // ponytail: plays past the last u32 song tick are dropped; no song reaches them.
            if opts.channels >> ch & 1 == 0 || beat * f64::from(PPQ) >= 4_294_967_296.0 {
                continue;
            }
            let split = state.split;
            match msg {
                MidiMsg::Program { program: p, .. } => program[usize::from(ch)] = p,
                MidiMsg::NoteOn { key, vel, .. } => {
                    let Some(target) = state.target(song, ch, &name, &mut made_here) else {
                        continue;
                    };
                    if std::mem::take(&mut new_lane) && made_here >> ch & 1 == 0 {
                        let lane = song.channels[target].add_lane(LaneKind::Notes);
                        song.channels[target].lanes[lane].label.clone_from(&name);
                        state.start_notes(target, lane);
                    }
                    let velocity = (f64::from(vel) * 0.007874 * 255.0) as u8;
                    if let Some(&lane) = state.last_notes.get(&target) {
                        state.notes[lane].on(beat, key, velocity);
                    }
                }
                MidiMsg::NoteOff { key, .. } => {
                    let lane = state.existing(ch).and_then(|t| state.last_notes.get(&t));
                    if let Some(&lane) = lane {
                        state.notes[lane].off(beat, key);
                    }
                }
                MidiMsg::Cc { cc, value, .. } => {
                    // CC 127 would be param 134, past the channel's CC params.
                    let supported = !opts.only_supported || SUPPORTED.contains(&cc);
                    if !opts.automation || !supported || cc == 127 {
                        continue;
                    }
                    let label = if split {
                        cc_name(cc)
                    } else {
                        format!("CHN #{ch} : MIDI CC #{cc}")
                    };
                    let value = ((f64::from(value) * 0.007874 * 65534.0) as u16).min(65534);
                    if let Some(target) = state.target(song, ch, &name, &mut made_here) {
                        state.point(target, ch, 7 + i32::from(cc), label, beat, value);
                    }
                }
                MidiMsg::PitchBend { value, .. } => {
                    let label = if split {
                        "Pitch Wheel".to_owned()
                    } else {
                        format!("CHN #{ch} : Pitch Wheel")
                    };
                    let value = ((f64::from(value) * 0.000061039 * 65534.0) as u16).min(65534);
                    if let Some(target) = state.target(song, ch, &name, &mut made_here) {
                        state.point(target, ch, 4, label, beat, value);
                    }
                }
                MidiMsg::ChannelPressure { .. } => {}
            }
        }
    }
    if as_song {
        let (lo, hi) = crate::song::TEMPO_RANGE;
        song.tempo_bpm = tempo.unwrap_or(1.0).clamp(lo, hi);
    }
    let made = state.made;
    state.finish(song);
    for (ch, made) in (0..).zip(made) {
        if let Some(i) = made {
            let channel = &mut song.channels[i];
            let preset = program_name(ch, program[usize::from(ch)]);
            let renamed = format!("{} (MIDI Channel {ch}: {preset})", channel.name);
            channel.name = renamed.trim_start().to_owned();
        }
    }
    Ok(())
}

/// Where an import puts what it reads, until [`State::finish`] writes the clips.
struct State {
    split: bool,
    current: usize,
    /// With Split on: the channel made for each MIDI channel.
    made: [Option<usize>; 16],
    /// Each target channel's last notes lane, an index into `notes`.
    last_notes: HashMap<usize, usize>,
    notes: Vec<NotesLane>,
    autos: Vec<AutoLane>,
    /// (target channel, MIDI channel, param) → index into `autos` (the reuse key).
    auto_index: HashMap<(usize, u8, i32), usize>,
}

/// A notes lane the import made, and its notes.
struct NotesLane {
    channel: usize,
    lane: usize,
    notes: Vec<Note>,
    /// Per key, the latest-starting note (the last added of equal starts).
    latest: [Option<usize>; 128],
}

struct Note {
    beat: f64,
    key: u8,
    velocity: u8,
    len: f64,
}

/// A channel automation lane to add, and its points as (beat, u16).
struct AutoLane {
    channel: usize,
    param: i32,
    label: String,
    points: Vec<(f64, u16)>,
}

impl NotesLane {
    fn on(&mut self, beat: f64, key: u8, velocity: u8) {
        let key = key & 0x7F;
        let latest = &mut self.latest[usize::from(key)];
        if latest.is_none_or(|i| self.notes[i].beat <= beat) {
            *latest = Some(self.notes.len());
        }
        self.notes.push(Note {
            beat,
            key,
            velocity,
            len: 1.0,
        });
    }

    /// The latest-starting note of `key` ends here, if that leaves it more than a tick; else it keeps its length.
    fn off(&mut self, beat: f64, key: u8) {
        if let Some(i) = self.latest[usize::from(key & 0x7F)] {
            let note = &mut self.notes[i];
            if beat - note.beat > 1.0 / f64::from(PPQ) {
                note.len = beat - note.beat;
            }
        }
    }
}

impl State {
    /// The channel MIDI channel `ch` goes to, if it has one yet.
    fn existing(&self, ch: u8) -> Option<usize> {
        if self.split {
            self.made[usize::from(ch)]
        } else {
            Some(self.current)
        }
    }

    /// The channel MIDI channel `ch` goes to. With Split on, the first time makes it: a DW Sampler channel named after
    /// the track, its notes lane too, with Pitch at 0.75 (+12 semitones) and nothing loaded (no program or kit ships).
    /// `None` once the channel ids are used up.
    fn target(
        &mut self,
        song: &mut Song,
        ch: u8,
        name: &str,
        made_here: &mut u16,
    ) -> Option<usize> {
        if let Some(i) = self.existing(ch) {
            return Some(i);
        }
        // ponytail: a MIDI channel past the channel id limit is dropped; no message for it yet.
        let i = song.insert_channel(
            song.channels.len(),
            ChannelKind::Synth(SynthKind::DwSampler),
        )?;
        let channel = &mut song.channels[i];
        channel.name = name.to_owned();
        channel.lanes[0].label = name.to_owned();
        if let Some(pitch) = channel.rack.modules[0].params.get_mut(3) {
            *pitch = 0.75;
        }
        self.made[usize::from(ch)] = Some(i);
        *made_here |= 1 << ch;
        self.start_notes(i, 0);
        Some(i)
    }

    /// Lane `lane` of channel `channel` becomes its last notes lane.
    fn start_notes(&mut self, channel: usize, lane: usize) {
        self.last_notes.insert(channel, self.notes.len());
        self.notes.push(NotesLane {
            channel,
            lane,
            notes: Vec::new(),
            latest: [None; 128],
        });
    }

    /// Adds a point to channel `channel`'s lane of `param` for MIDI channel `ch`, made (labelled `label`) the first
    /// time.
    fn point(&mut self, channel: usize, ch: u8, param: i32, label: String, beat: f64, value: u16) {
        let next = self.autos.len();
        let i = *self.auto_index.entry((channel, ch, param)).or_insert(next);
        if i == next {
            self.autos.push(AutoLane {
                channel,
                param,
                label,
                points: Vec::new(),
            });
        }
        self.autos[i].points.push((beat, value));
    }

    /// Writes one clip on each lane that got events, and adds the automation lanes (after the notes lanes, so the
    /// notes lanes' indices hold).
    fn finish(self, song: &mut Song) {
        let bar = song.time_sig.bar_beats();
        for lane in self.notes.into_iter().filter(|l| !l.notes.is_empty()) {
            let target = &mut song.channels[lane.channel].lanes[lane.lane];
            let clip = new_clip(target, lane.notes.iter().map(|n| n.beat));
            let start = clip.start_tick;
            clip.events = (lane.notes.iter())
                .map(|n| Event::Note {
                    pos: tick(n.beat).saturating_sub(start),
                    len_beats: n.len,
                    key: n.key,
                    velocity: n.velocity,
                    pan: Event::DEFAULT_PAN,
                    slide: false,
                })
                .collect();
            clip.events.sort_by_key(Event::pos);
            fit(clip, bar);
        }
        for mut auto in self.autos {
            let target = AutomationTarget {
                instance_id: 0,
                param: auto.param,
            };
            let lane = song.channels[auto.channel].add_automation_lane(target);
            lane.label = auto.label;
            // in time order (stable), so a lane shared by tracks stays a stepped curve.
            auto.points.sort_by(|a, b| a.0.total_cmp(&b.0));
            let clip = new_clip(lane, auto.points.iter().map(|p| p.0));
            let start = clip.start_tick;
            let mut pts = Vec::with_capacity(2 * auto.points.len());
            let mut last = None;
            for &(beat, value) in &auto.points {
                let tick = tick(beat).saturating_sub(start);
                // Stepped: the previous value first, at the new point's tick.
                if let Some(value) = last {
                    pts.push(Point { tick, value });
                }
                pts.push(Point { tick, value });
                last = Some(value);
            }
            points::set_points(clip, PointKey::Automation, pts);
            fit(clip, bar);
        }
    }
}

/// The song tick of `beat`, rounded.
fn tick(beat: f64) -> u32 {
    (beat * f64::from(PPQ)).round() as u32
}

/// The lane's clip for events starting at `beats`: from floor(first / 4)·4, 4 beats long, grown to the last event start
/// when that is 4 beats or more past the start ([`fit`] adds a tick). The events are the caller's.
fn new_clip(lane: &mut Lane, beats: impl Iterator<Item = f64> + Clone) -> &mut Clip {
    let first = beats.clone().fold(f64::INFINITY, f64::min);
    let last = beats.fold(0.0, f64::max);
    let start = (first / 4.0).floor() * 4.0;
    let clip = lane.add_clip(tick(start), (last - start).max(4.0));
    clip.visible_len_beats = clip.loop_len_beats;
    clip
}

/// Sets the clip's visible length, and its loop to `fit_loop`'s b·2ᵏ, doubled until it holds the visible part: nothing
/// repeats inside the clip, and no event sits on the loop end.
fn fit(clip: &mut Clip, bar: u32) {
    let last = clip.events.iter().map(Event::pos).max().unwrap_or(0);
    // A grown clip ends exactly 1 tick after its last event start, so the final note, CC or pitch-wheel value
    // starts (a sustain released at the end, a bend reset), as `fit_loop` lets a zero-length note last a tick. The
    // notes keep their lengths but sound only to the clip end (the last one a tick): a note-off is clamped to the
    // clip's length.
    let visible = (clip.visible_len_beats).max((f64::from(last) + 1.0) / f64::from(PPQ));
    clip.fit_loop(bar);
    while clip.loop_len_beats < visible {
        clip.loop_len_beats *= 2.0;
    }
    clip.visible_len_beats = visible;
}

fn channel_of(msg: MidiMsg) -> u8 {
    match msg {
        MidiMsg::NoteOn { ch, .. }
        | MidiMsg::NoteOff { ch, .. }
        | MidiMsg::Cc { ch, .. }
        | MidiMsg::PitchBend { ch, .. }
        | MidiMsg::ChannelPressure { ch, .. }
        | MidiMsg::Program { ch, .. } => ch,
    }
}

/// The program name of `program` on MIDI channel `ch`: `Drums` on channel 9; else the preset name the GM
/// program maps to. Only 0 → `Close Grand` is known (no preset index ships); the rest are `Program %u`, 0-based.
fn program_name(ch: u8, program: u8) -> String {
    match (ch, program) {
        (9, _) => "Drums".to_owned(),
        (_, 0) => "Close Grand".to_owned(),
        _ => format!("Program {program}"),
    }
}

/// A CC lane's name with Split on.
fn cc_name(cc: u8) -> String {
    match GM_CONTROLLERS.iter().find(|(n, _)| *n == cc) {
        Some((_, name)) => (*name).to_owned(),
        None if (32..=63).contains(&cc) => format!("Controller {}", cc - 31),
        None => format!("MIDI CC #{cc}"),
    }
}

/// A track name: UTF-8, else Latin-1; trailing NULs dropped, cut to 255 bytes as the export cuts it.
fn text(bytes: &[u8]) -> String {
    let mut s = match std::str::from_utf8(bytes) {
        Ok(s) => s.to_owned(),
        Err(_) => bytes.iter().map(|&b| char::from(b)).collect(),
    };
    s.truncate(s.trim_end_matches('\0').len());
    s.truncate(s.floor_char_boundary(255));
    s
}

#[cfg(test)]
mod tests;
