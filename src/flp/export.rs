//! A [`Song`] written as an FL Studio project (`.flp`), in the minimal layout FL Studio 2026 opens (Lemon Studio's
//! `os-flp` writer, checked against FL 26: version 26.1.7, 88-byte playlist records, 500 playlist tracks), at 384 PPQ
//! (3 of FL's ticks to each of ours, so every position is exact).
//!
//! - Each regular channel becomes an FL channel named, coloured and at the volume and pan it has. A 3x Osc becomes
//!   FL's 3x Osc with its oscillators ([`osc_state`], the inverse of the `.fst` import) and filter; a third-party plugin
//!   instrument FL's plugin wrapper with the plugin and its state ([`super::wrapper`]); a DW Sampler a
//!   Sampler with its sample (an audio file as it is, a program's zone nearest C5), its root key set so it plays at
//!   our pitch; a Drums channel one Sampler for each pad it plays, its hits at C5. Any other instrument becomes an
//!   empty Sampler named after its channel, with a notice.
//! - Each clip of a notes or drum lane is a pattern of its plays over its visible length (loops unrolled), placed in
//!   the playlist at its start, muted as it is, one playlist track per lane.
//! - An audio clip channel becomes an FL audio clip channel for each sample its clips play, each clip placed from its
//!   offset for its length.
//! - The samples it plays are WAV files beside the project ([`Exported::samples`]), named as the Samplers name them.
//!
//! Lossy: effects, sends, automation lanes, audio clips' stretch and pitch, the 3x Osc's envelopes, LFOs and arpeggiator, DW Sampler
//! envelopes and filters, every other instrument's sound.

use std::collections::BTreeSet;

use honeydew_audio::wav_writer::float32_wav;

use super::{Chunk, ChunkBody, Event, FlpFile, Header, Tag, Value, id};
use crate::audio_engine::samples::dw_path;
use crate::params::{Role, module_spec};
use crate::sample_cache::SampleCache;
use crate::smf::export::plays;
use crate::song::plugin::PLUGIN_INSTRUMENT;
use crate::song::{
    Channel, ChannelKind, Event as SongEvent, LaneKind, ModuleInstance, PPQ, SamplerSlot, Song,
};
use honeydew_audio::dwp::Cstm;

/// The version written (FL 26 opens it without a warning) and its build.
const VERSION: &str = "26.1.7.5653";
const BUILD: u32 = 5653;
/// FL ticks a beat: three of FL's to each of ours.
pub const FL_PPQ: u16 = 384;
const SCALE: u64 = FL_PPQ as u64 / PPQ as u64;
/// Playlist tracks an arrangement stores.
const TRACKS: u32 = 500;
/// The key a drum pad's Sampler plays its hits at: FL's C5, its Samplers' root.
const C5: u8 = 60;

/// A song written.
#[derive(Clone, Debug, PartialEq)]
pub struct Exported {
    pub flp: Vec<u8>,
    /// The sample files, by the names the project's Samplers give them.
    pub samples: Vec<(String, Vec<u8>)>,
    /// What had no FL counterpart.
    pub notices: Vec<String>,
}

/// One FL channel.
struct FlChannel {
    name: String,
    color: u32,
    volume: i32,
    pan: i32,
    /// 3x Osc's plugin state and its filter (219 #3–#5), or a Sampler's sample file.
    sound: Sound,
    root: u8,
}

enum Sound {
    Sampler(Option<String>),
    ThreeOsc(Vec<u8>, [i32; 3]),
    /// A third-party plugin: FL's plugin wrapper with its wrapper state ([`super::wrapper`]).
    Wrapper(Vec<u8>),
    /// An audio clip channel and its sample file.
    Audio(Option<String>),
}

/// Event 215 of a new audio clip channel as FL 26 writes it (declick mode 1, not stretched), from Lemon Studio's
/// `os-flp` (GPL-3.0-or-later).
const AUDIO_CLIP_215: &str = "8c0000000000000001000000ffffffff3c0000000000803f0000803f0000803f0000803f0000803f000000000300000000000000000400003000000000000100a7050000000000000001000000000000000000010000000000000000010000000000000000000000000000000000000002000000feffffffffffffff000000000000000000000000000000000000f03f0000000000000000ffffffff01010000000000000000e03f";

/// Writes `song`; `cache` holds the samples it plays, decoded (what the engine plays).
pub fn export(song: &Song, cache: &SampleCache) -> Exported {
    let mut out = Out {
        tempo: song.tempo_bpm,
        ..Out::default()
    };
    for (i, channel) in song.channels.iter().enumerate() {
        if channel.kind.file_kind() != 0x100 {
            continue;
        }
        out.channel(i, channel, cache);
    }
    let flp = out.file(song);
    Exported {
        flp,
        samples: out.samples,
        notices: out.notices,
    }
}

#[derive(Default)]
struct Out {
    /// The song's tempo, which audio clip windows are timed at.
    tempo: f64,
    channels: Vec<FlChannel>,
    /// (song channel, drum pad or `None` for its notes) → FL channel.
    map: Vec<((usize, Option<u8>), u16)>,
    /// Each pattern: its name, notes, length in FL ticks; and where the playlist places it.
    patterns: Vec<(String, Vec<[u8; 24]>, u32)>,
    items: Vec<[u8; 88]>,
    /// The playlist tracks named, by number (1-based).
    tracks: Vec<(u32, String)>,
    samples: Vec<(String, Vec<u8>)>,
    notices: Vec<String>,
}

impl Out {
    fn channel(&mut self, i: usize, channel: &Channel, cache: &SampleCache) {
        let base = FlChannel {
            name: channel.name.clone(),
            color: color(channel.hue),
            volume: volume_of(channel.params.level()),
            pan: (channel.params.pan() * 12800.0).round() as i32,
            sound: Sound::Sampler(None),
            root: C5,
        };
        if matches!(
            channel.kind,
            ChannelKind::AudioClip | ChannelKind::AudioRecord
        ) {
            return self.audio(channel, cache, base);
        }
        if let (ChannelKind::Drums, Some(kit)) = (channel.kind, &channel.drum_kit) {
            let pads: BTreeSet<u8> = (channel.lanes.iter())
                .flat_map(|l| l.clips.iter().flat_map(|c| &c.events))
                .filter_map(|e| match *e {
                    SongEvent::Drum { pad, .. } => Some(pad),
                    _ => None,
                })
                .collect();
            for pad in pads {
                let slot = kit.slot_on_pad(usize::from(pad));
                let name = format!(
                    "{} {}",
                    channel.name,
                    slot.map_or("Pad", SamplerSlot::display_name)
                );
                let file = slot.and_then(|s| self.sample(&name, cache.slot(s).as_deref()));
                let n = self.add(FlChannel {
                    name,
                    sound: Sound::Sampler(file),
                    ..base.clone_levels()
                });
                self.map.push(((i, Some(pad)), n));
            }
        } else {
            let module = channel.rack.modules.iter().find(|m| is_instrument(m));
            let sound = match module {
                Some(m) if m.type_id == 39 => {
                    let p = |k: usize| m.params.get(k).copied().unwrap_or(0.0);
                    let filter =
                        [p(111) * 256.0, p(112) * 256.0, p(110) * 7.0].map(|v| v.round() as i32);
                    (Sound::ThreeOsc(osc_state(&m.params), filter), C5)
                }
                Some(m) if m.type_id == PLUGIN_INSTRUMENT && m.file.plugin.is_some() => {
                    let plugin = m.file.plugin.as_deref().expect("checked");
                    (Sound::Wrapper(super::wrapper::encode(None, plugin)), C5)
                }
                Some(m) if m.type_id == 1 => match self.dw_sample(&channel.name, m, cache) {
                    Some((file, root)) => (Sound::Sampler(Some(file)), root),
                    None => {
                        self.notices.push(format!(
                            "{}: its DW Sampler has no sample to export",
                            channel.name
                        ));
                        (Sound::Sampler(None), C5)
                    }
                },
                Some(m) => {
                    let what = module_spec(m.type_id).map_or("its instrument", |s| s.display_name);
                    self.notices.push(format!(
                        "{}: {what} has no .flp counterpart (an empty Sampler)",
                        channel.name
                    ));
                    (Sound::Sampler(None), C5)
                }
                None => (Sound::Sampler(None), C5),
            };
            let n = self.add(FlChannel {
                sound: sound.0,
                root: sound.1,
                ..base
            });
            self.map.push(((i, None), n));
        }
        if channel.rack.modules.iter().any(|m| !is_instrument(m)) {
            self.notices
                .push(format!("{}: effects are not exported", channel.name));
        }
        self.clips(i, channel);
    }

    /// An audio channel's clips: an FL audio clip channel for each sample they play, each clip placed from its offset
    /// (FL's window, milliseconds at `tempo`) for its length, once.
    fn audio(&mut self, channel: &Channel, cache: &SampleCache, base: FlChannel) {
        let tempo = self.tempo;
        let ms = |beats: f64| (beats * 60_000.0 / tempo) as f32;
        let mut made: Vec<(Option<crate::flm::chunk::ContentPath>, u16)> = Vec::new();
        let track = self.tracks.len() as u32 + 1;
        let mut used = false;
        for clip in channel
            .lanes
            .iter()
            .filter(|l| l.kind == LaneKind::Audio)
            .flat_map(|l| &l.clips)
        {
            let Some(slot) = clip.file.sample.as_deref() else {
                continue;
            };
            let key = slot.file.path.clone();
            let n = match made.iter().find(|(k, _)| *k == key) {
                Some(&(_, n)) => n,
                None => {
                    let name = match made.len() {
                        0 => channel.name.clone(),
                        k => format!("{} {}", channel.name, k + 1),
                    };
                    let file = self.sample(&name, cache.slot(slot).as_deref());
                    let n = self.add(FlChannel {
                        name,
                        sound: Sound::Audio(file),
                        ..base.clone_levels()
                    });
                    made.push((key, n));
                    n
                }
            };
            used = true;
            let len = (clip.visible_ticks() * SCALE as f64).round().max(1.0) as u32;
            let mut r = item(clip.start_tick, len, 0, track - 1, clip.muted);
            r[6..8].copy_from_slice(&n.to_le_bytes());
            let start = ms(clip.offset_beats.max(0.0));
            r[24..28].copy_from_slice(&start.to_le_bytes());
            r[28..32].copy_from_slice(
                &(start + ms(clip.visible_ticks() / f64::from(PPQ))).to_le_bytes(),
            );
            self.items.push(r);
        }
        if used {
            self.tracks.push((track, channel.name.clone()));
        }
    }

    fn add(&mut self, channel: FlChannel) -> u16 {
        self.channels.push(channel);
        (self.channels.len() - 1) as u16
    }

    /// `data` as a WAV file named after `name` (unique), `None` without it.
    fn sample(
        &mut self,
        name: &str,
        data: Option<&honeydew_audio::sample::SampleData>,
    ) -> Option<String> {
        let data = data?;
        let mut file = format!(
            "{}.wav",
            name.replace(['/', '\\', ':', '*', '?', '"', '<', '>', '|'], "_")
        );
        let mut n = 1;
        while self
            .samples
            .iter()
            .any(|(f, _)| f.eq_ignore_ascii_case(&file))
        {
            n += 1;
            file = format!("{name} ({n}).wav");
        }
        let left: &[f32] = &data.left;
        let wav = match &data.right {
            Some(right) => float32_wav(data.sample_rate, &[left, right]),
            None => float32_wav(data.sample_rate, &[left]),
        };
        self.samples.push((file.clone(), wav));
        Some(file)
    }

    /// A DW Sampler's sample: its program's zone nearest C5 (the loudest layer), and the FL root key that plays it at
    /// our pitch (the zone's root less the module's Pitch).
    fn dw_sample(
        &mut self,
        name: &str,
        module: &ModuleInstance,
        cache: &SampleCache,
    ) -> Option<(String, u8)> {
        let blob = module.file.custom_data.as_deref()?;
        let path = dw_path(Cstm::read(blob).program?.path)?;
        let program = cache.program(&path)?;
        let (zone, key) = (program.iter()).min_by_key(|(z, _)| {
            (
                i32::from(z.root).abs_diff(i32::from(C5)),
                127 - i32::from(z.vel_high),
            )
        })?;
        let file = self.sample(name, cache.get(key).as_deref())?;
        let shift =
            honeydew_audio::dw_sampler::pitch_shift(module.params.get(3).copied().unwrap_or(0.5));
        Some((file, (i32::from(zone.root) - shift).clamp(0, 127) as u8))
    }

    /// Each clip of the channel's notes and drum lanes: a pattern of its plays, placed on the lane's playlist track.
    fn clips(&mut self, i: usize, channel: &Channel) {
        for lane in channel
            .lanes
            .iter()
            .filter(|l| matches!(l.kind, LaneKind::Notes | LaneKind::Drums))
        {
            if lane.clips.iter().all(|c| c.events.is_empty()) {
                continue;
            }
            let track = self.tracks.len() as u32 + 1;
            let label = if lane.label.is_empty() {
                channel.name.clone()
            } else {
                lane.label.clone()
            };
            self.tracks.push((track, label));
            for clip in lane.clips.iter().filter(|c| !c.events.is_empty()) {
                let start = f64::from(clip.start_tick);
                let len = (clip.visible_ticks() * SCALE as f64).round().max(1.0) as u32;
                let mut notes = Vec::new();
                for (t, event) in plays(clip) {
                    let pos = ((t - start).max(0.0) * SCALE as f64) as u32;
                    let (fl, key, len_beats, velocity, pan, slide) = match *event {
                        SongEvent::Note {
                            len_beats,
                            key,
                            velocity,
                            pan,
                            slide,
                            ..
                        } => (self.fl(i, None), key, len_beats, velocity, pan, slide),
                        SongEvent::Drum {
                            len_beats,
                            pad,
                            velocity,
                            pan,
                            ..
                        } => (self.fl(i, Some(pad)), C5, len_beats, velocity, pan, false),
                        _ => continue,
                    };
                    let Some(fl) = fl else { continue };
                    let length = (len_beats * f64::from(FL_PPQ)).round().max(0.0) as u32;
                    notes.push(note(pos, fl, length, key, velocity, pan, slide));
                }
                if notes.is_empty() {
                    continue;
                }
                notes.sort_by_key(|n| u32::from_le_bytes([n[0], n[1], n[2], n[3]]));
                self.patterns.push((channel.name.clone(), notes, len));
                let number = self.patterns.len() as u16;
                self.items
                    .push(item(clip.start_tick, len, number, track - 1, clip.muted));
            }
        }
    }

    fn fl(&self, i: usize, pad: Option<u8>) -> Option<u16> {
        self.map
            .iter()
            .find(|(k, _)| *k == (i, pad))
            .map(|&(_, n)| n)
    }

    /// The whole file, in FL's order: project info, the patterns' notes, the channels (the patterns' names after the
    /// first), the arrangement and its tracks.
    fn file(&self, song: &Song) -> Vec<u8> {
        let mut ev = vec![
            Event::new(id::VERSION, Value::var(format!("{VERSION}\0").into_bytes())),
            Event::new(id::BUILD, Value::U32(BUILD)),
            // Without it FL 26 reads only the first playlist clip.
            Event::new(id::PLAYLIST_FIX, Value::U32(15)),
            Event::new(
                id::TEMPO,
                Value::U32((song.tempo_bpm * 1000.0).round() as u32),
            ),
            Event::new(
                id::TIME_NUM,
                Value::U8(song.time_sig.bar_beats().min(255) as u8),
            ),
            Event::new(
                id::TIME_DEN,
                Value::U8(song.time_sig.steps_per_beat().min(255) as u8),
            ),
        ];
        for (event, text) in [
            (id::TITLE, song.title.as_str()),
            (206, ""),
            (207, ""),
            (195, ""),
        ] {
            ev.push(Event::new(event, Value::var(utf16(text))));
        }
        ev.push(Event::new(id::WORK_TIME, Value::var(work_time())));
        ev.push(Event::new(id::INIT_CONTROLS, Value::var(Vec::new())));
        for (n, (_, notes, _)) in self.patterns.iter().enumerate() {
            ev.push(Event::new(id::PAT_NEW, Value::U16(n as u16 + 1)));
            ev.push(Event::new(id::PAT_NOTES, Value::var(notes.concat())));
        }
        for (n, channel) in self.channels.iter().enumerate() {
            ev.extend(channel_events(n as u16, channel));
            if n == 0 {
                for (k, (name, ..)) in self.patterns.iter().enumerate() {
                    ev.extend(pattern_meta(k as u16 + 1, name));
                }
            }
        }
        ev.extend([
            Event::new(id::ARR_NEW, Value::U16(0)),
            Event::new(id::ARR_NAME, Value::var(utf16("Arrangement"))),
            Event::new(id::ARR_UNKNOWN, Value::U8(0)),
            Event::new(id::PLAYLIST, Value::var(self.items_bytes())),
        ]);
        for number in 1..=TRACKS {
            ev.push(Event::new(id::PL_TRACK, Value::var(track(number))));
            ev.push(Event::new(id::PL_COLOR_FLAG, Value::U8(0)));
            if let Some((_, name)) = self.tracks.iter().find(|(t, _)| *t == number) {
                ev.push(Event::new(id::PL_NAME, Value::var(utf16(name))));
            }
        }
        ev.push(Event::new(id::ARR_CURRENT, Value::U16(0)));
        let mut header = Vec::with_capacity(6);
        header.extend(0u16.to_le_bytes());
        header.extend((self.channels.len() as u16).to_le_bytes());
        header.extend(FL_PPQ.to_le_bytes());
        let file = FlpFile {
            chunks: vec![
                Chunk {
                    tag: Tag::HEADER,
                    body: ChunkBody::Header(Header { raw: header }),
                },
                Chunk {
                    tag: Tag::DATA,
                    body: ChunkBody::Events(ev),
                },
            ],
            trailing: Vec::new(),
        };
        file.to_bytes().expect("every event is of its id's kind")
    }

    /// The playlist records, by position, ids from 1.
    fn items_bytes(&self) -> Vec<u8> {
        let mut items = self.items.clone();
        items.sort_by_key(|r| u32::from_le_bytes([r[0], r[1], r[2], r[3]]));
        for (n, r) in items.iter_mut().enumerate() {
            r[32..36].copy_from_slice(&(n as u32 + 1).to_le_bytes());
        }
        items.concat()
    }
}

impl FlChannel {
    /// Its name, colour, volume and pan, with no sound.
    fn clone_levels(&self) -> Self {
        FlChannel {
            name: self.name.clone(),
            color: self.color,
            volume: self.volume,
            pan: self.pan,
            sound: Sound::Sampler(None),
            root: C5,
        }
    }
}

fn is_instrument(m: &ModuleInstance) -> bool {
    module_spec(m.type_id).is_some_and(|s| s.role == Role::Instrument)
}

/// A 24-byte note record (FL ticks): flags 0x4000 (| 0x0008 slide), fine pitch 120, release 64, mod X/Y 128.
fn note(pos: u32, channel: u16, len: u32, key: u8, velocity: u8, pan: u8, slide: bool) -> [u8; 24] {
    let mut r = [0u8; 24];
    r[0..4].copy_from_slice(&pos.to_le_bytes());
    r[4..6].copy_from_slice(&(0x4000u16 | if slide { 0x0008 } else { 0 }).to_le_bytes());
    r[6..8].copy_from_slice(&channel.to_le_bytes());
    r[8..12].copy_from_slice(&len.to_le_bytes());
    r[12..14].copy_from_slice(&u16::from(key.min(127)).to_le_bytes());
    r[16] = 120;
    r[18] = 64;
    r[20] = ((u32::from(pan) * 128 + 127) / 255).min(128) as u8;
    r[21] = (u16::from(velocity).div_ceil(2)).min(128) as u8;
    r[22] = 128;
    r[23] = 128;
    r
}

/// An 88-byte playlist record of pattern `number` at our tick `start` for `len` FL ticks on track `track` (0-based),
/// as FL 26 writes a new one.
fn item(start: u32, len: u32, number: u16, track: u32, muted: bool) -> [u8; 88] {
    let mut r = [0u8; 88];
    r[0..4].copy_from_slice(
        &((u64::from(start) * SCALE).min(u64::from(u32::MAX)) as u32).to_le_bytes(),
    );
    r[4..6].copy_from_slice(&0x5000u16.to_le_bytes());
    r[6..8].copy_from_slice(&(0x5000u16 + number).to_le_bytes());
    r[8..12].copy_from_slice(&len.to_le_bytes());
    r[12..14].copy_from_slice(&(499u16.saturating_sub(track as u16)).to_le_bytes());
    r[16] = 0x78;
    r[18] = 0x40;
    r[19] = if muted { 0x20 } else { 0 };
    r[20] = 0x40;
    r[21] = 0x64;
    r[22] = 0x80;
    r[23] = 0x80;
    r[24..32].fill(0xff);
    r[70] = 0xf0;
    r[71] = 0x3f;
    r[76..80].fill(0xff);
    r
}

/// A playlist track record (event 238) with FL's defaults.
fn track(number: u32) -> Vec<u8> {
    let mut r = vec![0u8; 70];
    r[0..4].copy_from_slice(&number.to_le_bytes());
    r[4..8].copy_from_slice(&0x002D_2C29u32.to_le_bytes());
    r[12] = 1;
    r[13..17].copy_from_slice(&1.0f32.to_le_bytes());
    r[17] = if number <= 33 { 0xf0 } else { 0xff };
    r[18..21].fill(0xff);
    r[30] = 5;
    r[38] = 1;
    r[53..61].fill(0xff);
    r[61] = 1;
    r
}

/// A pattern's metadata block: its name and FL's defaults for the rest (an Auto length).
fn pattern_meta(number: u16, name: &str) -> Vec<Event> {
    vec![
        Event::new(id::PAT_NEW, Value::U16(number)),
        Event::new(id::PAT_NAME, Value::var(utf16(name))),
        Event::new(id::PAT_COLOR, Value::U32(0x002D_2C29)),
        Event::new(157, Value::U32(u32::MAX)),
        Event::new(158, Value::U32(u32::MAX)),
        Event::new(id::PAT_LENGTH, Value::U32(0)),
        Event::new(id::PAT_META_END, Value::U8(0)),
        Event::new(id::PAT_171, Value::U32(0)),
        Event::new(id::PAT_252, Value::var(vec![0, 0])),
        Event::new(id::PAT_253, Value::var(utf16("0,0,0,0,0,0,0,0,0,0,0,0"))),
    ]
}

/// A channel block in the minimal form FL 26 loads.
fn channel_events(n: u16, ch: &FlChannel) -> Vec<Event> {
    let plugin = matches!(ch.sound, Sound::ThreeOsc(..) | Sound::Wrapper(_));
    let kind = match ch.sound {
        Sound::ThreeOsc(..) | Sound::Wrapper(_) => 2,
        Sound::Audio(_) => 4,
        Sound::Sampler(_) => 0,
    };
    let internal = match ch.sound {
        Sound::ThreeOsc(..) => "3x Osc",
        Sound::Wrapper(_) => super::wrapper::WRAPPER,
        _ => "",
    };
    let mut b = vec![
        Event::new(id::CH_NEW, Value::U16(n)),
        Event::new(id::CH_TYPE, Value::U8(kind)),
        Event::new(id::PLUGIN_NAME, Value::var(utf16(internal))),
    ];
    if plugin {
        let mut rec = vec![0u8; 52];
        rec[8] = 2;
        rec[16..20].copy_from_slice(&0x50u32.to_le_bytes());
        b.push(Event::new(id::PLUGIN_REC, Value::var(rec)));
    }
    b.push(Event::new(id::DISPLAY_NAME, Value::var(utf16(&ch.name))));
    if plugin {
        b.push(Event::new(id::PLUGIN_ICON, Value::U32(0)));
    }
    b.push(Event::new(id::CH_COLOR, Value::U32(ch.color)));
    b.push(Event::new(id::CH_COLOR_FLAG, Value::U8(1)));
    let mut levels = [0u8; 24];
    levels[0..4].copy_from_slice(&ch.pan.to_le_bytes());
    levels[4..8].copy_from_slice(&ch.volume.to_le_bytes());
    // The template's `00 01 00 00` (#3, a full 256) after the pitch; 3x Osc's filter there.
    levels[13] = 1;
    if let Sound::ThreeOsc(state, filter) = &ch.sound {
        b.push(Event::new(id::PLUGIN_STATE, Value::var(state.clone())));
        for (k, v) in filter.iter().enumerate() {
            levels[12 + 4 * k..16 + 4 * k].copy_from_slice(&v.to_le_bytes());
        }
    }
    if let Sound::Wrapper(state) = &ch.sound {
        b.push(Event::new(id::PLUGIN_STATE, Value::var(state.clone())));
    }
    b.push(Event::new(id::CH_ENABLED, Value::U8(1)));
    b.push(Event::new(id::CH_TARGET, Value::U16(0)));
    b.push(Event::new(id::CH_LEVELS, Value::var(levels.to_vec())));
    if let Sound::Audio(_) = ch.sound {
        // FL writes the sample settings right after the levels.
        let settings = (0..AUDIO_CLIP_215.len()).step_by(2);
        let settings =
            settings.map(|i| u8::from_str_radix(&AUDIO_CLIP_215[i..i + 2], 16).unwrap_or(0));
        b.push(Event::new(215, Value::var(settings.collect::<Vec<_>>())));
    }
    b.push(Event::new(id::CH_GROUP, Value::U32(0)));
    b.push(Event::new(id::CH_LOCKED, Value::U8(0)));
    if let Sound::Sampler(Some(file)) | Sound::Audio(Some(file)) = &ch.sound {
        b.push(Event::new(id::SAMPLE_PATH, Value::var(utf16(file))));
    }
    if ch.root != C5 {
        b.push(Event::new(id::CH_ROOT, Value::U32(u32::from(ch.root))));
    }
    b.push(Event::new(id::CH_END, Value::U8(0)));
    b
}

/// 3x Osc's plugin state (event 213, version 12): per oscillator its pan, shape, coarse, fine, phase and detune, and
/// the next one's Mix; the stereo phase randomness; the invert and AM switches. The inverse of the `.fst` import's
/// laws (`crate::project_files::fst`).
pub fn osc_state(params: &[f32]) -> Vec<u8> {
    let p = |k: usize| params.get(k).copied().unwrap_or(0.0);
    let mut r: Vec<i32> = vec![12];
    for o in 0..3 {
        r.extend(
            [
                (p(19 + o) - 0.5) * 128.0,
                p(1 + o) * 6.0,
                (p(13 + o) - 0.5) * 48.0,
                (p(16 + o) - 0.5) * 200.0,
                (p(7 + o) - 0.5) * 128.0,
                (p(10 + o) - 0.5) * 100.0,
            ]
            .map(|v| v.round() as i32),
        );
        if o < 2 {
            r.push((p(22 + o) * 128.0).round() as i32);
        }
    }
    r.push((p(26) * 64.0).round() as i32);
    let mut out: Vec<u8> = r.iter().flat_map(|v| v.to_le_bytes()).collect();
    out.extend([4, 5, 6, 24].map(|k| u8::from(p(k) >= 0.5)));
    out.push(u8::from(p(25) >= 0.5));
    out
}

/// FL's volume for our Level `level`: the same gain, 10000 at our 0 dB ([`super::import::level_of`]'s inverse).
fn volume_of(level: f32) -> i32 {
    if level <= 0.0 {
        return 0;
    }
    let db = 66.0 * f64::from(level).powf(0.6) - 60.0;
    (10000.0 * 10f64.powf(db / 20.0))
        .round()
        .clamp(0.0, 12800.0) as i32
}

/// FL's colour (`0x00BBGGRR`, each part at least 0x14) for our hue.
fn color(hue: f32) -> u32 {
    let h = f64::from(hue.rem_euclid(1.0)) * 6.0;
    let (s, v) = (0.55, 0.85);
    let c = v * s;
    let x = c * (1.0 - ((h % 2.0) - 1.0).abs());
    let (r, g, b) = match h as u32 {
        0 => (c, x, 0.0),
        1 => (x, c, 0.0),
        2 => (0.0, c, x),
        3 => (0.0, x, c),
        4 => (x, 0.0, c),
        _ => (c, 0.0, x),
    };
    let m = v - c;
    let byte = |f: f64| (((f + m) * 255.0).round() as u32).clamp(0x14, 0xFF);
    byte(r) | byte(g) << 8 | byte(b) << 16
}

fn utf16(s: &str) -> Vec<u8> {
    let mut v: Vec<u8> = s.encode_utf16().flat_map(u16::to_le_bytes).collect();
    v.extend([0, 0]);
    v
}

/// Event 237: now in days since 1899-12-30 (UTC), nothing spent.
fn work_time() -> Vec<u8> {
    #[cfg(target_arch = "wasm32")]
    let secs = web_sys::js_sys::Date::now() / 1000.0;
    #[cfg(not(target_arch = "wasm32"))]
    let secs = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map_or(0.0, |d| d.as_secs_f64());
    // 1970-01-01 is day 25569.
    let days = 25569.0 + secs / 86_400.0;
    [days.to_le_bytes(), 0f64.to_le_bytes()].concat()
}
