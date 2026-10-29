//! An FL Studio project (`.flp`) read into a [`Song`]: tempo, time signature and title; each generator channel with
//! its name, colour, volume and pan; the patterns' notes as clips where the playlist places them (pattern mode: one
//! after another); channel volume and pan automation clips; the mixer insert effects Honeydew has.
//!
//! **Versions.** Read event by event, so an unknown event is skipped and never fails the import, and a damaged file
//! keeps what came before the damage. The encodings that changed across versions are each read both ways: the tempo
//! (156, else 66 + 93 / 1000), the channel levels (219, else the words 72/73, else the bytes 2/3), the mixer insert
//! (104, else 22), the names (203, else 192; UTF-16 from FL 12, 8-bit before), the playlist records (32 bytes up to
//! FL 20, 60 from 21, 80 from 24.2, 88 from 26) and pattern clip offsets (f32 before 24.2, u32 ticks after), and the
//! timing at any PPQ. Built for FL 9 to FL 2025/2026; checked on real files of Lemon Studio's writer (its FL 26
//! layout) and on synthetic FL 12-era files ([`super::tests`]). A file older than FL 9 (or with no version) imports
//! what reads, with a notice.
//!
//! **Instruments.** 3x Osc goes through the `.fst` import ([`crate::project_files::import_fst`]); a Sampler's or audio
//! clip's sample is the caller's to find ([`Imported::samples`]); a Layer channel of Samplers each on keys of its own
//! (a key split) one channel whose samples are zones ([`SampleRef::zone`]); an empty Sampler with notes, our closest
//! sound; any other plugin plays the closest sound of ours, guessed from its channel's and plugin's names
//! ([`gm::stand_in`]), and is named in [`Imported::notices`]. A muted channel stays muted.
//!
//! Lossy: plugin states other than 3x Osc's, mixer routing, sends and insert levels, effect settings (a kept effect
//! starts at its defaults), audio clips' stretching and fades, other layer channels, automation of anything but
//! channel volume and pan, note fine pitch, release, mod X/Y and colour groups, playlist track names and markers.

use std::collections::{HashMap, HashSet};

use super::event::EventReader;
use super::{Event, Value, id};
use crate::content::Factory;
use crate::smf::gm;
use crate::song::plugin::{PLUGIN_EFFECT, PLUGIN_INSTRUMENT, PluginRef};
use crate::song::points::{self, Point, PointKey};
use crate::song::{
    AutomationTarget, Event as SongEvent, Lane, LaneKind, PPQ, Song, SynthKind, TimeSig,
};

/// A Sampler channel's sample, for the caller to find.
#[derive(Clone, Debug, PartialEq)]
pub struct SampleRef {
    /// The song channel.
    pub channel: usize,
    /// The path as the project stores it.
    pub path: String,
    /// The Sampler's root key (FL's C5, 60, by default).
    pub root: u8,
    /// An audio clip channel's: its clips play it.
    pub audio: bool,
    /// A key split's sample: it plays only the keys `lo..=hi`, beside the channel's other zones.
    pub zone: Option<(u8, u8)>,
}

/// A project read.
#[derive(Clone, Debug, PartialEq)]
pub struct Imported {
    pub song: Song,
    /// The Sampler channels with a sample to find. Each is a DW Sampler with nothing loaded yet.
    pub samples: Vec<SampleRef>,
    /// The summary's lines: plugins replaced, effects kept or dropped, what an older version skipped.
    pub notices: Vec<String>,
    /// The FL version that saved it (`""` when the file does not say).
    pub version: String,
}

/// Not an FL Studio project.
pub const NOT_FLP: &str = "not an .flp project";

/// Reads `bytes`. `Err` only when it is no FLP at all.
pub fn import(bytes: &[u8], factory: &Factory) -> Result<Imported, String> {
    let file = read_file(bytes)?;
    let mut notices = file.damaged.clone().into_iter().collect::<Vec<_>>();
    let project = Project::scan(&file);
    let version = project.version.clone();
    let major = major(&version);
    if major.is_none_or(|m| m < 9) {
        let shown = match version.is_empty() {
            true => "an unknown version".to_owned(),
            false => format!("version {version}"),
        };
        notices.push(format!(
            "Saved by {shown}, older than 9: imported its tempo, channels, notes and playlist; its mixer \
             effects and automation were skipped."
        ));
    }
    let mut song = Song::default();
    if let Some(title) = project.title.clone().filter(|t| !t.is_empty()) {
        song.title = title;
    }
    if let Some(tempo) = project.tempo {
        let (lo, hi) = crate::song::TEMPO_RANGE;
        song.tempo_bpm = tempo.clamp(lo, hi);
    }
    if let Some((num, steps)) = project.time_sig {
        song.time_sig = TimeSig {
            subdivision: steps,
            beats_per_bar: num,
        };
    }
    let mut built = Build {
        project: &project,
        song: &mut song,
        channels: HashMap::new(),
        ppq: f64::from(file.ppq),
    };
    built.channels();
    built.clips();
    built.audio_clips();
    if major.is_some_and(|m| m >= 9) {
        built.automation();
    }
    let (channels, samples) = built.instruments(factory, &mut notices);
    if major.is_some_and(|m| m >= 9) {
        built.effects(&channels, &mut notices);
    }
    Ok(Imported {
        song,
        samples,
        notices,
        version,
    })
}

/// The file's PPQ and events.
struct File {
    ppq: u16,
    events: Vec<Event>,
    /// Where the event stream broke off, when it did.
    damaged: Option<String>,
}

/// `FLhd` and the `FLdt` events. A stream that breaks off keeps the events before it; a cut chunk keeps what is there.
fn read_file(bytes: &[u8]) -> Result<File, String> {
    let header = bytes
        .get(..14)
        .filter(|h| &h[..4] == b"FLhd")
        .ok_or(NOT_FLP)?;
    let len = u32::from_le_bytes(header[4..8].try_into().expect("4 bytes")) as usize;
    let ppq = match u16::from_le_bytes([header[12], header[13]]) {
        0 => 96,
        ppq => ppq,
    };
    let mut at = 8usize.checked_add(len).ok_or(NOT_FLP)?;
    while bytes.len().saturating_sub(at) >= 8 && &bytes[at..at + 4] != b"FLdt" {
        let skip = u32::from_le_bytes(bytes[at + 4..at + 8].try_into().expect("4 bytes")) as usize;
        at = at.saturating_add(8).saturating_add(skip);
    }
    let data = bytes
        .get(at..at + 8)
        .filter(|d| &d[..4] == b"FLdt")
        .ok_or(NOT_FLP)?;
    let declared = u32::from_le_bytes(data[4..8].try_into().expect("4 bytes")) as usize;
    let body = &bytes[at + 8..];
    let body = &body[..declared.min(body.len())];
    let (mut events, mut damaged) = (Vec::new(), None);
    for read in EventReader::new(body, at + 8) {
        match read {
            Ok((_, event)) => events.push(event),
            Err(err) => damaged = Some(format!("The project is damaged and was read up to {err}.")),
        }
    }
    Ok(File {
        ppq,
        events,
        damaged,
    })
}

/// The FL major version of "a.b.c…", `None` without one.
fn major(version: &str) -> Option<u32> {
    version.split('.').next()?.trim().parse().ok()
}

/// A text event: UTF-16LE from FL 12 on, 8-bit before, whichever the bytes are when the version does not tell; one
/// trailing NUL dropped.
fn text(data: &[u8], utf16: Option<bool>) -> String {
    let even = data.len().is_multiple_of(2);
    let looks_utf16 = data.len() >= 2 && even && data[1] == 0;
    if utf16.unwrap_or(looks_utf16) && even {
        let units: Vec<u16> = data
            .as_chunks::<2>()
            .0
            .iter()
            .map(|&c| u16::from_le_bytes(c))
            .collect();
        let units = units.strip_suffix(&[0]).unwrap_or(&units);
        return String::from_utf16_lossy(units);
    }
    let data = data.strip_suffix(&[0]).unwrap_or(data);
    data.iter().map(|&b| char::from(b)).collect()
}

fn int(e: &Event) -> u32 {
    e.int_value().unwrap_or(0)
}

/// A channel of the channel rack.
#[derive(Default)]
struct FlChannel {
    /// Its event-64 number, which notes and links name.
    iid: u16,
    kind: Option<u8>,
    plugin: String,
    name: Option<String>,
    color: Option<u32>,
    /// 0..=12800, 10000 = FL's default.
    volume: Option<f64>,
    /// 0..=12800, 6400 = centre.
    pan: Option<f64>,
    insert: Option<i32>,
    sample: Option<String>,
    root: Option<u8>,
    state: Option<Vec<u8>>,
    automation: Option<Vec<u8>>,
    /// Off: muted.
    enabled: Option<bool>,
    /// A Layer's children, by channel number.
    children: Vec<u16>,
    /// The keys it plays (event 215's key zone), `None` for all.
    zone: Option<(u8, u8)>,
    /// Its block's events, for the 3x Osc import.
    events: Vec<Event>,
}

/// One note of a pattern, in pattern ticks.
#[derive(Clone, Copy)]
struct FlNote {
    pos: u32,
    channel: u16,
    len: u32,
    key: u8,
    velocity: u8,
    pan: u8,
    slide: bool,
}

#[derive(Default)]
struct Pattern {
    notes: Vec<FlNote>,
    /// The length box, ticks; `None` = Auto.
    length: Option<u32>,
}

/// One playlist clip.
#[derive(Clone, Copy)]
struct Item {
    pos: u32,
    len: u32,
    /// `Ok(pattern)` or `Err(channel)` (an audio or automation clip).
    source: Result<u16, u16>,
    /// The window into its source: pattern ticks, or an automation clip's beats.
    window: Option<(f64, f64)>,
    muted: bool,
}

/// An effect in a mixer insert's slot.
struct Effect {
    insert: i32,
    name: String,
    /// A wrapped third-party plugin's wrapper state ([`super::wrapper`]).
    wrapper: Option<Vec<u8>>,
    /// FL's own effect as Lemon Studio's ([`lemon_of`]).
    lemon: Option<(u16, Option<Vec<f32>>)>,
}

/// What the event stream holds.
#[derive(Default)]
struct Project {
    version: String,
    title: Option<String>,
    tempo: Option<f64>,
    time_sig: Option<(u8, u8)>,
    channels: Vec<FlChannel>,
    patterns: HashMap<u16, Pattern>,
    /// Each arrangement's clips by its number; the one before any arrangement is 0.
    playlists: Vec<(u16, Vec<Item>)>,
    current_arrangement: Option<u16>,
    /// Automation channel → the targets it drives.
    links: Vec<(u16, u32)>,
    effects: Vec<Effect>,
    /// What a record layout could not read.
    unread: Vec<String>,
    /// The Layers that are key splits ([`Project::splits`]), and their children that have no notes of their own.
    splits: HashSet<u16>,
    folded: HashSet<u16>,
}

/// Where the scan is.
#[derive(Clone, Copy, PartialEq)]
enum Region {
    Channels,
    /// The playlist and its tracks: nothing more for the channels.
    Rest,
    Mixer,
}

/// A mixer slot's plugin until its slot event.
#[derive(Default)]
struct Pending {
    name: String,
    display: Option<String>,
    state: Option<Vec<u8>>,
    insert: Option<i32>,
}

impl Project {
    fn scan(file: &File) -> Self {
        let mut p = Project::default();
        let events = &file.events;
        // The version comes first in every file; its major sets the text encoding.
        let version = events.iter().find(|e| e.id == id::VERSION);
        p.version = version
            .and_then(Event::data)
            .map(|d| text(d, Some(false)))
            .unwrap_or_default();
        let utf16 = major(&p.version).map(|m| m >= 12);
        let txt = |e: &Event| e.data().map(|d| text(d, utf16)).unwrap_or_default();
        let (mut coarse, mut fine) = (None, None);
        let mut region = Region::Channels;
        let mut pattern: Option<u16> = None;
        let mut arrangement = 0u16;
        let (mut insert, mut pending) = (0i32, None::<Pending>);
        for e in events {
            match e.id {
                id::TEMPO => p.tempo = Some(f64::from(int(e)) / 1000.0),
                id::TEMPO_COARSE => coarse = Some(f64::from(int(e))),
                id::TEMPO_FINE => fine = Some(f64::from(int(e)) / 1000.0),
                id::TITLE => p.title = Some(txt(e)),
                id::TIME_NUM | id::TIME_DEN => {
                    let (num, den) = p.time_sig.get_or_insert((4, 4));
                    let v = (int(e) as u8).max(1);
                    *if e.id == id::TIME_NUM { num } else { den } = v;
                }
                id::PAT_NEW => pattern = Some(int(e) as u16),
                id::PAT_NOTES => {
                    let Some(number) = pattern else { continue };
                    let data = e.data().unwrap_or_default();
                    match notes(data) {
                        Some(notes) => p.patterns.entry(number).or_default().notes.extend(notes),
                        None => p.unread.push(format!("the notes of pattern {number}")),
                    }
                }
                id::PAT_LENGTH => {
                    if let Some(number) = pattern {
                        p.patterns.entry(number).or_default().length =
                            Some(int(e)).filter(|&l| l > 0);
                    }
                }
                id::ARR_NEW => {
                    region = Region::Rest;
                    arrangement = int(e) as u16;
                }
                id::ARR_CURRENT => p.current_arrangement = Some(int(e) as u16),
                id::PLAYLIST => {
                    region = region.max_rest();
                    match items(e.data().unwrap_or_default()) {
                        Some(items) => p.playlists.push((arrangement, items)),
                        None => p.unread.push("the playlist".into()),
                    }
                }
                id::PL_TRACK | id::PL_NAME => region = region.max_rest(),
                id::AUTO_LINK => {
                    let d = e.data().unwrap_or_default();
                    if d.len() >= 12 && d[0] == 0 && d[1] == 0 {
                        let ch = u16::from_le_bytes([d[2], d[3]]);
                        p.links.push((
                            ch,
                            u32::from_le_bytes(d[8..12].try_into().expect("4 bytes")),
                        ));
                    }
                }
                id::CH_NEW if region == Region::Channels => {
                    p.channels.push(FlChannel {
                        iid: int(e) as u16,
                        ..FlChannel::default()
                    });
                    pattern = None;
                }
                id::MX_FLAGS
                | id::MX_ROUTES
                | id::MX_OUTPUT
                | id::MX_COLOR
                | 42
                | id::MX_NAME
                | id::MX_SLOT
                | id::MX_PARAMS
                | id::MX_INPUT
                | 95 => {
                    region = Region::Mixer;
                    match e.id {
                        id::MX_SLOT => {
                            if let Some(plugin) = pending.take().filter(|p| !p.name.is_empty()) {
                                let name = match plugin.name.as_str() {
                                    "Fruity Wrapper" => {
                                        plugin.state.as_deref().and_then(wrapper_name)
                                    }
                                    _ => None,
                                };
                                let wrapper = (plugin.name == super::wrapper::WRAPPER)
                                    .then(|| plugin.state.clone())
                                    .flatten();
                                let lemon = lemon_of(&plugin.name, plugin.state.as_deref());
                                let name = name.or(plugin.display).unwrap_or(plugin.name);
                                p.effects.push(Effect {
                                    insert: plugin.insert.unwrap_or(insert),
                                    name,
                                    wrapper,
                                    lemon,
                                });
                            }
                        }
                        id::MX_OUTPUT => insert += 1,
                        _ => {}
                    }
                }
                _ if region == Region::Mixer => match e.id {
                    id::PLUGIN_NAME => {
                        pending = Some(Pending {
                            name: txt(e),
                            ..Pending::default()
                        })
                    }
                    id::DISPLAY_NAME => {
                        if let Some(p) = &mut pending {
                            p.display = Some(txt(e)).filter(|n| !n.is_empty());
                        }
                    }
                    id::PLUGIN_STATE => {
                        if let Some(p) = &mut pending {
                            p.state = e.data().map(<[u8]>::to_vec);
                        }
                    }
                    // The plugin's window record: a mixer slot's names its insert (flags 0x100).
                    id::PLUGIN_REC => {
                        let d = e.data().unwrap_or_default();
                        if let (Some(p), Some(r)) = (&mut pending, d.get(..20)) {
                            let flags = u32::from_le_bytes(r[16..20].try_into().expect("4 bytes"));
                            if flags & 0x100 != 0 {
                                p.insert =
                                    Some(i32::from_le_bytes(r[..4].try_into().expect("4 bytes")));
                            }
                        }
                    }
                    _ => {}
                },
                _ if region == Region::Channels => {
                    if let Some(ch) = p.channels.last_mut() {
                        ch.read(e, &txt);
                    }
                }
                _ => {}
            }
        }
        if p.tempo.is_none() {
            p.tempo = coarse.map(|c| c + fine.unwrap_or(0.0));
        }
        p.splits();
        p
    }

    /// The Layers that are key splits: each child a Sampler with a sample, no two on the same key. A child with no
    /// notes of its own is only the Layer's zone.
    fn splits(&mut self) {
        let noted: HashSet<u16> = (self.patterns.values())
            .flat_map(|p| p.notes.iter().map(|n| n.channel))
            .collect();
        for layer in self.channels.iter().filter(|c| c.kind == Some(3)) {
            let children: Option<Vec<&FlChannel>> = (layer.children.iter())
                .map(|&c| self.channels.iter().find(|ch| ch.iid == c))
                .collect();
            let Some(mut children) = children.filter(|c| !c.is_empty()) else {
                continue;
            };
            let sampler = |c: &&FlChannel| matches!(c.kind, Some(0) | None) && c.sample.is_some();
            children.sort_by_key(|c| c.keys());
            let split = children.iter().all(sampler)
                && children.windows(2).all(|w| w[0].keys().1 < w[1].keys().0);
            if split {
                self.splits.insert(layer.iid);
                (self.folded).extend(
                    children
                        .iter()
                        .map(|c| c.iid)
                        .filter(|c| !noted.contains(c)),
                );
            }
        }
    }

    /// The playlist shown: the current arrangement's, else the first.
    fn playlist(&self) -> &[Item] {
        let current = self.current_arrangement;
        let shown = current.and_then(|n| self.playlists.iter().find(|(a, _)| *a == n));
        shown
            .or(self.playlists.first())
            .map_or(&[], |(_, items)| items)
    }
}

impl Region {
    /// The playlist or a later part: never back to the channels.
    fn max_rest(self) -> Self {
        match self {
            Region::Channels => Region::Rest,
            other => other,
        }
    }
}

impl FlChannel {
    fn read(&mut self, e: &Event, txt: &impl Fn(&Event) -> String) {
        self.events.push(e.clone());
        match e.id {
            id::CH_TYPE => self.kind = Some(int(e) as u8),
            id::PLUGIN_NAME => self.plugin = txt(e),
            id::DISPLAY_NAME => self.name = Some(txt(e)).filter(|n| !n.is_empty()),
            id::CH_NAME_OLD if self.name.is_none() => {
                self.name = Some(txt(e)).filter(|n| !n.is_empty())
            }
            id::CH_COLOR => self.color = Some(int(e)),
            id::CH_LEVELS => {
                let d = e.data().unwrap_or_default();
                let at = |o: usize| {
                    d.get(o..o + 4)
                        .map(|b| i32::from_le_bytes(b.try_into().expect("4 bytes")))
                };
                self.pan = at(0).map(f64::from);
                self.volume = at(4).map(f64::from);
            }
            id::CH_VOL_WORD => self.volume = self.volume.or(Some(f64::from(int(e)))),
            id::CH_PAN_WORD => self.pan = self.pan.or(Some(f64::from(int(e)))),
            id::CH_VOL_BYTE => self.volume = self.volume.or(Some(f64::from(int(e)) * 100.0)),
            id::CH_PAN_BYTE => self.pan = self.pan.or(Some(f64::from(int(e)) * 100.0)),
            id::CH_TARGET => self.insert = Some(i32::from(int(e) as u16 as i16)),
            id::CH_TARGET_BYTE => self.insert = self.insert.or(Some(i32::from(int(e) as u8 as i8))),
            id::SAMPLE_PATH => self.sample = Some(txt(e)).filter(|s| !s.is_empty()),
            id::CH_ROOT => self.root = Some(int(e).min(127) as u8),
            id::PLUGIN_STATE => self.state = e.data().map(<[u8]>::to_vec),
            id::AUTO_DATA => self.automation = e.data().map(<[u8]>::to_vec),
            id::CH_ENABLED => self.enabled = Some(int(e) != 0),
            id::CH_LAYER_CHILD => self.children.push(int(e) as u16),
            id::CH_SAMPLE_SETTINGS => {
                let d = e.data().unwrap_or_default();
                let at = |o: usize| {
                    d.get(o..o + 4)
                        .map(|b| i32::from_le_bytes(b.try_into().expect("4 bytes")))
                };
                if let (Some(lo), Some(hi)) = (at(68), at(72)) {
                    let key = |k: i32| k.clamp(0, 127) as u8;
                    self.zone = ((lo, hi) != (0, 256)).then_some((key(lo), key(hi)));
                }
            }
            _ => {}
        }
    }

    /// The keys it plays, `lo..=hi`.
    fn keys(&self) -> (u8, u8) {
        self.zone.unwrap_or((0, 127))
    }

    /// The name the channel rack shows: its own, else its plugin's (a VST's from the wrapper), else its sample's.
    fn shown_name(&self) -> String {
        let sample = self.sample.as_deref().map(file_stem);
        (self.name.clone())
            .or_else(|| self.plugin_name())
            .or(sample)
            .unwrap_or_else(|| "Sampler".into())
    }

    /// The third-party plugin a wrapper channel hosts, with its state ([`super::wrapper`]).
    fn wrapped(&self) -> Option<PluginRef> {
        (self.plugin == super::wrapper::WRAPPER).then_some(())?;
        super::wrapper::decode(self.state.as_deref()?)
    }

    /// The plugin's name: a VST's from its wrapper state, else the internal one; `None` for none.
    fn plugin_name(&self) -> Option<String> {
        let wrapped =
            (self.plugin == "Fruity Wrapper").then(|| self.state.as_deref().and_then(wrapper_name));
        wrapped
            .flatten()
            .or_else(|| Some(self.plugin.clone()).filter(|p| !p.is_empty()))
    }

    /// An audio clip channel.
    fn audio(&self) -> bool {
        self.kind == Some(4)
    }

    /// A generator: a Sampler or a plugin; not a layer, an audio clip or an automation clip.
    fn generator(&self) -> bool {
        match self.kind {
            Some(0) | None => true,
            Some(2) => !self.plugin.is_empty(),
            _ => false,
        }
    }
}

/// Adds a module of `type_id` hosting `plugin` to channel `index`'s rack, after what it holds: its params come from the
/// plugin once it runs (`plugins::adopt_params`).
fn plugin_module(song: &mut Song, index: usize, type_id: u16, plugin: PluginRef) {
    let rack = &mut song.channels[index].rack;
    let id = rack.add(type_id);
    if let Some(module) = rack.get_mut(id) {
        module.file.plugin = Some(Box::new(plugin));
    }
}

/// Lemon Studio's effect for FL's own `plugin` with saved state `state`: Fruity Reeverb 2 as Lemon Reverb, Fruity
/// Limiter as Lemon Limiter, with the params the state holds (`None` inside for a state that cannot be read).
fn lemon_of(plugin: &str, state: Option<&[u8]>) -> Option<(u16, Option<Vec<f32>>)> {
    use honeydew_audio::lemon;
    let state = state.unwrap_or_default();
    match plugin {
        "Fruity Reeverb 2" => Some((lemon::REVERB, lemon::reverb_from_fl(state))),
        "Fruity Limiter" => Some((
            lemon::LIMITER,
            (!state.is_empty()).then(|| lemon::limiter_from_fl(state)),
        )),
        _ => None,
    }
}

/// The plugin name in a Fruity Wrapper state: a u32, then `(u32 id, u64 size, bytes)` records; record 54 is the name.
fn wrapper_name(state: &[u8]) -> Option<String> {
    let mut at = 4;
    while let Some(head) = state.get(at..at + 12) {
        let rec = u32::from_le_bytes(head[..4].try_into().expect("4 bytes"));
        let size =
            usize::try_from(u64::from_le_bytes(head[4..12].try_into().expect("8 bytes"))).ok()?;
        let body = state.get(at + 12..(at + 12).checked_add(size)?)?;
        if rec == 54 {
            let body = body.strip_suffix(&[0]).unwrap_or(body);
            return Some(String::from_utf8_lossy(body).into_owned()).filter(|n| !n.is_empty());
        }
        at += 12 + size;
    }
    None
}

/// The file name of `path` (either separator, FL's `%…%` folders included) without its extension.
pub fn file_stem(path: &str) -> String {
    let name = file_name(path);
    match name.rsplit_once('.') {
        Some((stem, _)) if !stem.is_empty() => stem.to_owned(),
        _ => name.to_owned(),
    }
}

/// The last part of `path`, by either separator.
pub fn file_name(path: &str) -> &str {
    path.rsplit(['/', '\\']).next().unwrap_or(path)
}

/// Event 224's 24-byte notes; `None` for another layout.
fn notes(data: &[u8]) -> Option<Vec<FlNote>> {
    if !data.len().is_multiple_of(24) {
        return None;
    }
    let notes = data.as_chunks::<24>().0.iter().map(|r| {
        let u16_at = |o: usize| u16::from_le_bytes([r[o], r[o + 1]]);
        let u32_at = |o: usize| u32::from_le_bytes(r[o..o + 4].try_into().expect("4 bytes"));
        FlNote {
            pos: u32_at(0),
            slide: u16_at(4) & 0x0008 != 0,
            channel: u16_at(6),
            len: u32_at(8),
            key: u16_at(12).min(127) as u8,
            pan: r[20],
            velocity: r[21],
        }
    });
    Some(notes.collect())
}

/// Event 233's records: 88, 80, 60 or 32 bytes, whichever the list is a whole number of with `00 50` at byte 4 of
/// each; `None` for none.
fn items(data: &[u8]) -> Option<Vec<Item>> {
    if data.is_empty() {
        return Some(Vec::new());
    }
    let len = [88, 80, 60, 32].into_iter().find(|&n| {
        data.len().is_multiple_of(n) && data.chunks_exact(n).all(|r| r[4..6] == [0x00, 0x50])
    })?;
    let items = data.chunks_exact(len).map(|r| {
        let u32_at = |o: usize| u32::from_le_bytes(r[o..o + 4].try_into().expect("4 bytes"));
        let item = u16::from_le_bytes([r[6], r[7]]);
        let source = if item >= 0x5000 {
            Ok(item - 0x5000)
        } else {
            Err(item)
        };
        // Offsets: u32 pattern ticks from 24.2 (`ffffffff` none), f32 before and for other clips (-1 none).
        let offset = |o: usize| {
            let raw = u32_at(o);
            let float = f32::from_bits(raw);
            match raw {
                u32::MAX => None,
                _ if float == -1.0 => None,
                _ if source.is_err() || (float.is_finite() && float >= 0.5) => {
                    Some(f64::from(float))
                }
                _ => Some(f64::from(raw)),
            }
        };
        Item {
            pos: u32_at(0),
            len: u32_at(8),
            source,
            window: offset(24).zip(offset(28)),
            muted: r[19] & 0x20 != 0,
        }
    });
    Some(items.collect())
}

/// The song under construction.
struct Build<'a> {
    project: &'a Project,
    song: &'a mut Song,
    /// FL channel number → song channel.
    channels: HashMap<u16, usize>,
    ppq: f64,
}

impl Build<'_> {
    /// Song ticks of `t` FL ticks.
    fn ticks(&self, t: f64) -> u32 {
        (t * f64::from(PPQ) / self.ppq)
            .round()
            .clamp(0.0, f64::from(u32::MAX)) as u32
    }

    fn beats(&self, t: f64) -> f64 {
        t / self.ppq
    }

    /// A song channel for each generator: a DW Sampler with nothing loaded, named, coloured, at its volume and pan.
    fn channels(&mut self) {
        let project = self.project;
        for ch in (project.channels.iter())
            .filter(|c| c.generator() || c.audio() || project.splits.contains(&c.iid))
            .filter(|c| !project.folded.contains(&c.iid))
        {
            let kind = match ch.audio() {
                true => crate::song::ChannelKind::AudioClip,
                false => crate::song::ChannelKind::Synth(SynthKind::DwSampler),
            };
            let Some(i) = self.song.insert_channel(self.song.channels.len(), kind) else {
                break;
            };
            let channel = &mut self.song.channels[i];
            channel.name = ch.shown_name();
            if let Some(hue) = ch.color.and_then(hue) {
                channel.hue = hue;
            }
            if let Some(volume) = ch.volume {
                channel.params.0[0] = level_of(volume);
            }
            if let Some(pan) = ch.pan {
                channel.params.0[1] = (pan / 12800.0).clamp(0.0, 1.0) as f32;
            }
            if ch.enabled == Some(false) {
                channel.params.0[2] = 1.0;
            }
            self.channels.insert(ch.iid, i);
        }
    }

    /// Each pattern clip's notes as a clip on each channel it plays, on the first notes lane it fits in, from the
    /// pattern's start (its window) for the clip's length, looping the pattern. Without pattern clips (pattern mode),
    /// the patterns one after another.
    fn clips(&mut self) {
        let project = self.project;
        let bar = f64::from(self.song.time_sig.bar_beats()) * self.ppq;
        let mut items: Vec<Item> = (project.playlist().iter())
            .filter(|i| i.source.is_ok())
            .copied()
            .collect();
        if items.is_empty() {
            let mut numbers: Vec<_> = project
                .patterns
                .iter()
                .filter(|(_, p)| !p.notes.is_empty())
                .collect();
            numbers.sort_by_key(|(n, _)| **n);
            let mut at = 0.0;
            for (&number, pattern) in numbers {
                let len = pattern_len(pattern, bar);
                items.push(Item {
                    pos: at as u32,
                    len: len as u32,
                    source: Ok(number),
                    window: None,
                    muted: false,
                });
                at += len;
            }
        }
        items.sort_by_key(|i| i.pos);
        for item in items {
            let Ok(number) = item.source else { continue };
            let Some(pattern) = project.patterns.get(&number) else {
                continue;
            };
            let loop_len = pattern_len(pattern, bar);
            let mut by_channel: Vec<(usize, Vec<&FlNote>)> = Vec::new();
            for note in &pattern.notes {
                let Some(&c) = self.channels.get(&note.channel) else {
                    continue;
                };
                match by_channel.iter_mut().find(|(i, _)| *i == c) {
                    Some((_, notes)) => notes.push(note),
                    None => by_channel.push((c, vec![note])),
                }
            }
            let start = self.ticks(f64::from(item.pos));
            let visible = self.beats(f64::from(item.len.max(1)));
            let offset = item.window.map_or(0.0, |(a, _)| self.beats(a));
            for (c, notes) in by_channel {
                let mut events: Vec<SongEvent> = (notes.iter())
                    .map(|n| SongEvent::Note {
                        pos: self.ticks(f64::from(n.pos)),
                        len_beats: match n.len {
                            0 => 0.25,
                            len => self.beats(f64::from(len)),
                        },
                        key: n.key,
                        velocity: (u16::from(n.velocity.min(127)) * 2) as u8,
                        pan: (u32::from(n.pan.min(128)) * 255 / 128).min(255) as u8,
                        slide: n.slide,
                    })
                    .collect();
                events.sort_by_key(SongEvent::pos);
                let loop_beats = self.beats(loop_len);
                let lane = lane_for(&mut self.song.channels[c].lanes, start, visible);
                let clip = lane.add_clip(start, loop_beats);
                clip.visible_len_beats = visible;
                clip.offset_beats = offset;
                clip.muted = item.muted;
                clip.events = events;
            }
        }
    }

    /// Each audio clip on its channel's audio lane, from its window's start (milliseconds) for its length, playing once.
    /// The sample is the caller's to find.
    fn audio_clips(&mut self) {
        let project = self.project;
        let beats_a_ms = self.song.tempo_bpm / 60_000.0;
        for item in project.playlist() {
            let Err(ch) = item.source else { continue };
            let audio = project.channels.iter().any(|c| c.iid == ch && c.audio());
            let Some(&c) = self.channels.get(&ch).filter(|_| audio) else {
                continue;
            };
            let start = self.ticks(f64::from(item.pos));
            let visible = self.beats(f64::from(item.len.max(1)));
            let offset = item.window.map_or(0.0, |(a, _)| (a * beats_a_ms).max(0.0));
            let Some(lane) = self.song.channels[c]
                .lanes
                .iter_mut()
                .find(|l| l.kind == LaneKind::Audio)
            else {
                continue;
            };
            let clip = lane.add_clip(start, visible + offset);
            clip.visible_len_beats = visible;
            clip.offset_beats = offset;
            clip.muted = item.muted;
        }
    }

    /// Automation clips of a channel's volume or pan: a Level or Pan lane on that channel, its points stepped as FL
    /// draws them only at its points (no curve shapes).
    fn automation(&mut self) {
        let project = self.project;
        for item in project.playlist() {
            let Err(auto) = item.source else { continue };
            let Some(data) = project
                .channels
                .iter()
                .find(|c| c.iid == auto)
                .and_then(|c| c.automation.as_deref())
            else {
                continue;
            };
            let Some(points) = auto_points(data) else {
                continue;
            };
            for &(_, target) in project.links.iter().filter(|(c, _)| *c == auto) {
                let (fl_channel, param) = ((target >> 16) as u16, (target & 0xFFFF) as i32);
                let Some(&c) = self.channels.get(&fl_channel).filter(|_| param <= 1) else {
                    continue;
                };
                let start = self.ticks(f64::from(item.pos));
                let visible = self.beats(f64::from(item.len.max(1)));
                let offset = item.window.map_or(0.0, |(a, _)| a);
                let pts: Vec<Point> = (points.iter())
                    .filter(|(beat, _)| *beat + 1e-9 >= offset && *beat - offset <= visible + 1e-9)
                    .map(|&(beat, value)| {
                        let value = match param {
                            0 => level_of(value * 12800.0),
                            _ => value as f32,
                        };
                        Point {
                            tick: ((beat - offset) * f64::from(PPQ)).round() as u32,
                            value: points::raw(value.clamp(0.0, 1.0)),
                            ..Default::default()
                        }
                    })
                    .collect();
                if pts.is_empty() {
                    continue;
                }
                let target = AutomationTarget {
                    instance_id: 0,
                    param,
                };
                let channel = &mut self.song.channels[c];
                if !channel.lanes.iter().any(|l| l.target == Some(target)) {
                    channel.add_automation_lane(target);
                }
                let lane = channel
                    .lanes
                    .iter_mut()
                    .find(|l| l.target == Some(target))
                    .expect("just made");
                let clip = lane.add_clip(start, visible);
                clip.visible_len_beats = visible;
                points::set_points(clip, PointKey::Automation, pts);
            }
        }
    }

    /// Each channel's instrument: 3x Osc through the `.fst` import, a Sampler left for its sample, any other plugin
    /// our closest sound. Returns each song channel's FL channel, and the samples to find.
    fn instruments(
        &mut self,
        factory: &Factory,
        notices: &mut Vec<String>,
    ) -> (Vec<(usize, usize)>, Vec<SampleRef>) {
        let project = self.project;
        let mut pairs = Vec::new();
        let mut samples = Vec::new();
        for (n, ch) in project.channels.iter().enumerate() {
            let Some(&i) = self.channels.get(&ch.iid) else {
                continue;
            };
            pairs.push((i, n));
            let name = self.song.channels[i].name.clone();
            let sample = |audio| {
                (ch.sample.clone()).map(|path| SampleRef {
                    channel: i,
                    path,
                    root: ch.root.unwrap_or(60),
                    audio,
                    zone: None,
                })
            };
            let notes = (self.song.channels[i].lanes.iter())
                .any(|l| l.clips.iter().any(|c| !c.events.is_empty()));
            match ch.kind {
                Some(4) => samples.extend(sample(true)),
                // A key split: each child's sample on its keys.
                Some(3) => {
                    let children = (ch.children.iter())
                        .filter_map(|&c| project.channels.iter().find(|x| x.iid == c));
                    samples.extend(children.filter_map(|child| {
                        Some(SampleRef {
                            channel: i,
                            path: child.sample.clone()?,
                            root: child.root.unwrap_or(60),
                            audio: false,
                            zone: Some(child.keys()),
                        })
                    }));
                }
                Some(0) | None => match sample(false) {
                    Some(sample) => samples.push(sample),
                    // An empty Sampler with notes (a placeholder, or another app's synth) plays our closest sound.
                    None if notes => {
                        let loaded = gm::stand_in(self.song, i, &[&name], factory);
                        notices.push(format!("{name}: empty Sampler, plays {loaded}"));
                    }
                    None => {}
                },
                _ if ch.plugin.eq_ignore_ascii_case("3x Osc") => {
                    three_osc(self.song, i, ch, &project.version)
                }
                // A third-party plugin that is installed here plays itself, from its saved state.
                _ if (ch.wrapped()).is_some_and(|p| crate::plugins::installed(&p)) => {
                    let plugin = ch.wrapped().expect("checked");
                    plugin_module(self.song, i, PLUGIN_INSTRUMENT, plugin);
                }
                _ => {
                    let plugin = ch.plugin_name().unwrap_or_default();
                    let loaded = gm::stand_in(self.song, i, &[&name, &plugin], factory);
                    notices.push(format!("{name}: {plugin} replaced by {loaded}"));
                }
            }
        }
        for what in &project.unread {
            notices.push(format!("Could not read {what} (an unknown layout)."));
        }
        let skipped = (project.channels.iter())
            .filter(|c| c.kind == Some(3) && !project.splits.contains(&c.iid))
            .count();
        if skipped > 0 {
            notices.push(format!("{skipped} layer channel(s) were skipped."));
        }
        (pairs, samples)
    }

    /// The effects of each channel's mixer insert, appended to its rack where Honeydew has the effect (Fruity Reeverb 2
    /// and Fruity Limiter as Lemon Studio's, with their settings; the others at their default settings), and the master
    /// insert's to the master's. `pairs`: each song channel's FL channel.
    fn effects(&mut self, pairs: &[(usize, usize)], notices: &mut Vec<String>) {
        let project = self.project;
        let mut inserts: Vec<(i32, Vec<usize>)> = vec![(0, vec![0])];
        for &(i, n) in pairs {
            let insert = project.channels[n].insert.unwrap_or(0);
            if insert <= 0 {
                continue;
            }
            match inserts.iter_mut().find(|(k, _)| *k == insert) {
                Some((_, channels)) => channels.push(i),
                None => inserts.push((insert, vec![i])),
            }
        }
        for (insert, channels) in inserts {
            let names: Vec<_> = channels
                .iter()
                .map(|&c| self.song.channels[c].name.clone())
                .collect();
            let where_ = names.join(", ");
            for effect in project.effects.iter().filter(|e| e.insert == insert) {
                // A third-party plugin that is installed here runs itself, from its saved state.
                let wrapped = effect.wrapper.as_deref().and_then(super::wrapper::decode);
                if let Some(plugin) = wrapped.filter(crate::plugins::installed) {
                    for &c in &channels {
                        plugin_module(self.song, c, PLUGIN_EFFECT, plugin.clone());
                    }
                    continue;
                }
                if let Some((type_id, params)) = &effect.lemon {
                    for &c in &channels {
                        let rack = &mut self.song.channels[c].rack;
                        let id = rack.add(*type_id);
                        if let (Some(module), Some(params)) = (rack.get_mut(id), params) {
                            module.params.clone_from(params);
                        }
                    }
                    if params.is_none() {
                        let ours =
                            crate::params::module_spec(*type_id).map_or("", |s| s.display_name);
                        notices.push(format!(
                            "{where_}: {} as {ours} at its default settings",
                            effect.name
                        ));
                    }
                    continue;
                }
                match effect_of(&effect.name) {
                    Some((type_id, ours)) => {
                        for &c in &channels {
                            self.song.channels[c].rack.add(type_id);
                        }
                        notices.push(format!("{where_}: {} as {ours}", effect.name));
                    }
                    None => notices.push(format!("{where_}: {} dropped", effect.name)),
                }
            }
        }
    }
}

/// A pattern's loop: its length box, else whole bars over its notes (at least one).
fn pattern_len(pattern: &Pattern, bar: f64) -> f64 {
    if let Some(len) = pattern.length {
        return f64::from(len);
    }
    let end = (pattern.notes.iter())
        .map(|n| f64::from(n.pos) + f64::from(n.len.max(1)))
        .fold(0.0, f64::max);
    ((end / bar).ceil() * bar).max(bar)
}

/// The first notes lane of `lanes` free over `[start, start + visible)`, a new one when none is.
fn lane_for(lanes: &mut Vec<Lane>, start: u32, visible: f64) -> &mut Lane {
    let end = f64::from(start) + visible * f64::from(PPQ);
    let free = |lane: &Lane| {
        (lane.clips.iter()).all(|c| {
            let c_end = f64::from(c.start_tick) + c.visible_ticks();
            c_end <= f64::from(start) || f64::from(c.start_tick) >= end
        })
    };
    match lanes
        .iter()
        .position(|l| l.kind == LaneKind::Notes && free(l))
    {
        Some(i) => &mut lanes[i],
        None => {
            let at = lanes
                .iter()
                .rposition(|l| l.kind != LaneKind::Automation)
                .map_or(0, |i| i + 1);
            lanes.insert(at, Lane::new(LaneKind::Notes));
            &mut lanes[at]
        }
    }
}

/// An automation clip's points (event 234): a 21-byte header with the count at 17, then 24-byte points, each a
/// delta in beats and a value 0..1, as `(beat, value)`.
fn auto_points(d: &[u8]) -> Option<Vec<(f64, f64)>> {
    let n = u32::from_le_bytes(d.get(17..21)?.try_into().ok()?) as usize;
    let body = d.get(21..21usize.checked_add(n.checked_mul(24)?)?)?;
    let mut beat = 0.0;
    let points = body.as_chunks::<24>().0.iter().map(|r| {
        beat += f64::from_le_bytes(r[0..8].try_into().expect("8 bytes"));
        (
            beat,
            f64::from_le_bytes(r[8..16].try_into().expect("8 bytes")).clamp(0.0, 1.0),
        )
    });
    Some(points.collect())
}

/// Our Level for FL channel volume `volume` (0..=12800, 10000 the default, our 0 dB): the same gain in dB, 0 for none.
pub fn level_of(volume: f64) -> f32 {
    if volume <= 0.0 {
        return 0.0;
    }
    let db = (20.0 * (volume / 10000.0).log10()).clamp(-60.0, 6.0);
    crate::content::kits::level(db as f32).clamp(0.0, 1.0)
}

/// The hue of FL colour `bgr` (`0x00BBGGRR`), 0..1; `None` for a grey.
fn hue(bgr: u32) -> Option<f32> {
    let [r, g, b, _] = bgr.to_le_bytes().map(|c| f32::from(c) / 255.0);
    let (max, min) = (r.max(g).max(b), r.min(g).min(b));
    let d = max - min;
    if d < 0.02 {
        return None;
    }
    let h = if max == r {
        ((g - b) / d).rem_euclid(6.0)
    } else if max == g {
        (b - r) / d + 2.0
    } else {
        (r - g) / d + 4.0
    };
    Some(h / 6.0)
}

/// FL effects by name word, and ours: what a mixer insert's effect becomes.
const EFFECTS: &[(&[&str], u16, &str)] = &[
    (&["multiband"], 36, "Multiband Compressor"),
    (
        &["reverb", "reeverb", "verb", "room", "hall", "plate"],
        35,
        "Reverb 2",
    ),
    (&["delay", "echo"], 18, "Tape Delay"),
    (&["chorus"], 11, "Chorus"),
    (&["flanger", "flangus"], 13, "Flanger"),
    (&["phaser"], 14, "Phaser"),
    (
        &["limiter", "maximus", "clipper", "soundgoodizer"],
        28,
        "Limiter",
    ),
    (&["compressor", "compress", "comp"], 8, "Compressor"),
    (&["graphic"], 7, "Graphic EQ"),
    (&["eq", "equo", "equalizer", "parametric"], 3, "Equalizer"),
    (&["filter", "philter"], 9, "Filter"),
    (&["waveshaper"], 41, "Waveshaper"),
    (
        &[
            "distortion",
            "distort",
            "overdrive",
            "dist",
            "distructor",
            "saturator",
            "bitcrusher",
            "crusher",
        ],
        6,
        "Distortion",
    ),
    (
        &["stereo", "enhancer", "shaper", "widener"],
        15,
        "Stereoizer",
    ),
    (&["gate"], 27, "Gate"),
    (&["tremolo", "panomatic"], 46, "Tremolo"),
];

/// Our effect for FL effect `name`, `None` when Honeydew has none like it.
fn effect_of(name: &str) -> Option<(u16, &'static str)> {
    let lower = name.to_lowercase();
    let words: Vec<&str> = lower
        .split(|c: char| !c.is_alphanumeric())
        .filter(|w| !w.is_empty())
        .collect();
    let found = EFFECTS
        .iter()
        .find(|(list, ..)| words.iter().any(|w| list.contains(w)));
    found.map(|&(_, id, ours)| (id, ours))
}

/// Channel `index` becomes a 3x Osc whose params the `.fst` import reads from the FL channel's own events (an `.fst`
/// is one channel's events): its plugin state and its envelope, filter, voice and arpeggio blobs.
fn three_osc(song: &mut Song, index: usize, ch: &FlChannel, version: &str) {
    let mut events = Vec::new();
    super::event::Event::new(id::VERSION, Value::var(format!("{version}\0").into_bytes()))
        .encode_into(&mut events);
    for e in &ch.events {
        e.encode_into(&mut events);
    }
    let mut fst = b"FLhd\x06\0\0\0\x20\0\x01\0\x60\0FLdt".to_vec();
    fst.extend((events.len() as u32).to_le_bytes());
    fst.extend(events);
    let import = crate::project_files::import_fst(&fst, std::path::Path::new("3x Osc.fst"));
    let channel = &mut song.channels[index];
    channel.kind = crate::song::ChannelKind::Synth(SynthKind::ThreeXOsc);
    let module = gm::instrument(&mut channel.rack, SynthKind::ThreeXOsc);
    if import.ok {
        module.params = import.params;
    }
}
