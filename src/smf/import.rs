//! MIDI import: [`scan`] for the import dialog and the one-channel shortcut, and [`import`] for its three entry
//! points.
//!
//! Where MIDI files and readers differ, Honeydew's choices:
//! - Running status is kept across metas and SysEx, which are skipped ([`codec::read`]); a data byte with no status
//!   before it fails the file.
//! - A channel's program is its last `Cn`; a channel without one plays GM program 0.
//! - A CC lane is shared by the SMF tracks of one target channel, MIDI channel and CC.
//! - A file whose scan finds no channel opens as a song with only the master (the tempo set); the track menu imports
//!   nothing from it.
//! - Opened as a song, a file of more than one tempo plays as a tempo map ([`set_tempo`]); the track menu keeps the
//!   song's tempo and master.

use std::collections::HashMap;

use super::codec::{self, SmfEvent};
use super::gm;
use crate::content::Factory;
use crate::midi::MidiMsg;
use crate::song::points::{self, Point, PointKey, Shape, ShapeKind};
use crate::song::{
    AutomationTarget, ChannelKind, Clip, DrumGrid, Event, Lane, LaneKind, PPQ, Song, SynthKind,
    TEMPO_RANGE, TEMPO_TARGET, tempo_value,
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

/// What an import made: the new channels' indices, and a line per channel naming the sound it plays
/// (`<channel>: <sound>`).
#[derive(Clone, Debug, Default, PartialEq)]
pub struct Imported {
    pub channels: Vec<usize>,
    pub notices: Vec<String>,
}

/// Imports `bytes` into `song`: with Split on (always when `as_song`) into one new channel per selected MIDI channel,
/// else into channel `current`; notes on notes lanes, controllers and the pitch wheel on channel automation lanes. A
/// new channel plays the sound of its GM program from `factory` ([`gm::load_program`]); MIDI channel 10 a Drums
/// channel with our kit, its notes as hits on a notes-grid drum lane ([`gm::drum_kit`]). `as_song` (opening a `.mid`)
/// also sets the file tempo and its tempo map (`set_tempo`). `Err` (the detail for [`LOAD_FAILED`]) leaves the song as
/// it was: the file does not read, or `current` is no channel.
pub fn import(
    song: &mut Song,
    current: usize,
    bytes: &[u8],
    opts: &ImportOpts,
    as_song: bool,
    factory: &Factory,
) -> Result<Imported, String> {
    let smf = codec::read(bytes)?;
    if current >= song.channels.len() {
        return Err(format!("no channel {current}"));
    }
    let mut state = State {
        split: opts.split || as_song,
        current,
        factory,
        kits: Vec::new(),
        made: [None; 16],
        last_notes: HashMap::new(),
        notes: Vec::new(),
        autos: Vec::new(),
        auto_index: HashMap::new(),
    };
    let division = f64::from(smf.division);
    // The `FF 51`s as (beat, BPM), in file order.
    let (mut tempos, mut program) = (Vec::new(), [0u8; 16]);
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
                    tempos.push((beat, 60e6 / f64::from(*us)));
                    continue;
                }
                // A malformed `FF 51` is a tempo parse error.
                SmfEvent::Other { kind: 0x51, .. } => {
                    tempos.push((beat, 120.0));
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
                        let channel = &mut song.channels[target];
                        let drums = state.is_drums(target);
                        let lane = match drums {
                            true => channel.add_lane(LaneKind::Drums),
                            false => channel.add_lane(LaneKind::Notes),
                        };
                        channel.lanes[lane].label.clone_from(&name);
                        if drums {
                            channel.lanes[lane].grid = DrumGrid::Notes;
                        }
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
        set_tempo(song, tempos);
    }
    let (made, kits) = (state.made, state.kits.clone());
    state.finish(song);
    let mut imported = Imported::default();
    for (ch, made) in (0..).zip(made) {
        let Some(i) = made else { continue };
        let p = program[usize::from(ch)];
        let sound = match kits.iter().find(|(c, _)| *c == i) {
            Some(&(_, kit)) => kit.to_owned(),
            None => gm::load_program(song, i, p, factory),
        };
        let channel = &mut song.channels[i];
        let preset = program_name(ch, p);
        let renamed = format!("{} (MIDI Channel {ch}: {preset})", channel.name);
        channel.name = renamed.trim_start().to_owned();
        imported.channels.push(i);
        imported.notices.push(format!("{}: {sound}", channel.name));
    }
    Ok(imported)
}

/// Where an import puts what it reads, until [`State::finish`] writes the clips.
struct State<'a> {
    split: bool,
    current: usize,
    factory: &'a Factory,
    /// The Drums channels made for MIDI channel 10, with their kit's name.
    kits: Vec<(usize, &'static str)>,
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

impl State<'_> {
    /// Channel `channel` is a Drums channel this import made: its notes are hits.
    fn is_drums(&self, channel: usize) -> bool {
        self.kits.iter().any(|(c, _)| *c == channel)
    }

    /// The channel MIDI channel `ch` goes to, if it has one yet.
    fn existing(&self, ch: u8) -> Option<usize> {
        if self.split {
            self.made[usize::from(ch)]
        } else {
            Some(self.current)
        }
    }

    /// The channel MIDI channel `ch` goes to. With Split on, the first time makes it, named after the track, its notes
    /// lane too: a DW Sampler channel, whose sound [`import`] loads at the end; for MIDI channel 10 a Drums channel with
    /// our kit and a notes-grid drum lane. `None` once the channel ids are used up.
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
        let drums = ch == 9;
        let kind = match drums {
            true => ChannelKind::Drums,
            false => ChannelKind::Synth(SynthKind::DwSampler),
        };
        // ponytail: a MIDI channel past the channel id limit is dropped; no message for it yet.
        let i = song.insert_channel(song.channels.len(), kind)?;
        let channel = &mut song.channels[i];
        channel.name = name.to_owned();
        channel.lanes[0].label = name.to_owned();
        if drums {
            channel.lanes[0].grid = DrumGrid::Notes;
            let default = || {
                (
                    channel.drum_kit.clone().unwrap_or_default(),
                    "the default kit",
                )
            };
            let (kit, kit_name) = gm::drum_kit(self.factory).unwrap_or_else(default);
            channel.drum_kit = Some(kit);
            self.kits.push((i, kit_name));
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
        for lane in self.notes.iter().filter(|l| !l.notes.is_empty()) {
            let channel = &mut song.channels[lane.channel];
            // A Drums channel's notes are hits, each GM key on its pad.
            let kit =
                (self.is_drums(lane.channel)).then(|| channel.drum_kit.clone().unwrap_or_default());
            let target = &mut channel.lanes[lane.lane];
            let clip = new_clip(target, lane.notes.iter().map(|n| n.beat));
            let start = clip.start_tick;
            clip.events = (lane.notes.iter())
                .map(|n| {
                    let pos = tick(n.beat).saturating_sub(start);
                    match &kit {
                        Some(kit) => Event::Drum {
                            pos,
                            len_beats: n.len,
                            pad: gm::drum_pad(kit, n.key),
                            velocity: n.velocity,
                            pan: Event::DEFAULT_PAN,
                            value: Event::DEFAULT_PITCH,
                            repeat: 0,
                        },
                        None => Event::Note {
                            pos,
                            len_beats: n.len,
                            key: n.key,
                            velocity: n.velocity,
                            pan: Event::DEFAULT_PAN,
                            slide: false,
                        },
                    }
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
                    pts.push(Point {
                        tick,
                        value,
                        ..Default::default()
                    });
                }
                pts.push(Point {
                    tick,
                    value,
                    ..Default::default()
                });
                last = Some(value);
            }
            points::set_points(clip, PointKey::Automation, pts);
            fit(clip, bar);
        }
    }
}

/// Sets the song's tempo from the file's `tempos`, (beat, BPM) in file order, clamped to [`TEMPO_RANGE`], and replaces
/// the master's tempo lane. One tempo (at the lane's resolution; none at all is the MIDI
/// default of 120 BPM) is the song's tempo, wherever it falls. More are a tempo map: the song's tempo is the one in
/// force at tick 0 (the last there in file order, else 120), and a master `Tempo` lane gets a point at each change,
/// Square at 100 % so it holds its tempo up to the next point and jumps there (its values clamped to the lane's range,
/// [`crate::song::AUTOMATION_TEMPO_RANGE`]).
fn set_tempo(song: &mut Song, mut tempos: Vec<(f64, f64)>) {
    let value = |bpm| points::raw(tempo_value(bpm));
    let (lo, hi) = TEMPO_RANGE;
    let bar = song.time_sig.bar_beats();
    let master = &mut song.channels[0];
    master.lanes.retain(|l| l.target != Some(TEMPO_TARGET));
    if tempos.iter().all(|t| value(t.1) == value(tempos[0].1)) {
        song.tempo_bpm = tempos.last().map_or(120.0, |t| t.1).clamp(lo, hi);
        return;
    }
    tempos.sort_by(|a, b| a.0.total_cmp(&b.0));
    let mut map = vec![(0, 120.0)];
    for (beat, bpm) in tempos {
        let at = tick(beat);
        match map.last_mut() {
            Some(last) if last.0 == at => last.1 = bpm,
            _ => map.push((at, bpm)),
        }
    }
    map.dedup_by_key(|p| value(p.1));
    song.tempo_bpm = map[0].1.clamp(lo, hi);
    if map.len() == 1 {
        return;
    }
    let lane = master.add_automation_lane(TEMPO_TARGET);
    let clip = new_clip(lane, map.iter().map(|p| f64::from(p.0) / f64::from(PPQ)));
    let shape = Shape {
        kind: ShapeKind::Square,
        tension: 100,
    };
    let pts = (map.iter())
        .map(|&(tick, bpm)| Point {
            tick,
            value: value(bpm),
            shape,
        })
        .collect();
    points::set_points(clip, PointKey::Automation, pts);
    fit(clip, bar);
}

/// The song tick of `beat`, rounded.
fn tick(beat: f64) -> u32 {
    (beat * f64::from(PPQ)).round() as u32
}

/// The lane's clip for events starting at `beats`: from floor(first / 4)·4, 4 beats long, grown to the last event start
/// when that is 4 beats or more past the start ([`fit`] grows it over the notes). The events are the caller's.
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
    // A grown clip ends at least 1 tick after its last event start, so the final CC or pitch-wheel value starts (a
    // sustain released at the end, a bend reset), as `fit_loop` lets a zero-length note last a tick; and it covers its
    // notes' ends, so the last notes sound in full.
    let ppq = f64::from(PPQ);
    let end = (clip.events.iter())
        .map(|e| match *e {
            Event::Note { pos, len_beats, .. } | Event::Drum { pos, len_beats, .. } => {
                f64::from(pos) + (len_beats * ppq).max(1.0)
            }
            ref e => f64::from(e.pos()) + 1.0,
        })
        .fold(0.0, f64::max);
    let visible = (clip.visible_len_beats).max(end / ppq);
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

/// The program name of `program` on MIDI channel `ch`: `Drums` on channel 9; else its GM name.
fn program_name(ch: u8, program: u8) -> String {
    match ch {
        9 => "Drums".to_owned(),
        _ => gm::program_name(program).to_owned(),
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
