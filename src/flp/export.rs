//! A [`Song`] written as an FL Studio project (`.flp`), in the minimal layout FL Studio 2026 opens (Lemon Studio's
//! `os-flp` writer, checked against FL 26: version 26.1.7, 88-byte playlist records, 500 playlist tracks), at 384 PPQ
//! (3 of FL's ticks to each of ours, so every position is exact).
//!
//! - Each regular channel becomes an FL channel named, coloured and at the volume and pan it has, on a mixer insert of
//!   its own named after it. A 3x Osc becomes FL's 3x Osc with its oscillators ([`osc_state`], the inverse of the
//!   `.fst` import) and filter; a third-party plugin instrument FL's plugin wrapper with the plugin and its state
//!   ([`super::wrapper`]); a DW Sampler a Sampler of its sample at its root key, or with a program of several zones a
//!   Layer channel of one Sampler for each zone ([`dw_zones`]), each on its zone's keys; a Drums channel one Sampler
//!   for each pad it plays, its hits at C5. Any other instrument becomes a Sampler of one note of it, rendered.
//! - Each effect goes into its insert's slots ([`mixer`]): FL's own effect where it has one like it ([`EFFECTS`]), the
//!   Equalizer with its bands ([`eq_bands`]), Lemon Reverb and Lemon Limiter as Fruity Reeverb 2 and Fruity Limiter
//!   with their settings (`honeydew_audio::lemon`), the others at FL's default settings; a third-party plugin effect in
//!   FL's plugin wrapper. The master's effects go into the master's slots.
//! - Each clip of a notes or drum lane is a pattern of its plays over its visible length (loops unrolled), placed in
//!   the playlist at its start, muted as it is, one playlist track per lane.
//! - An audio clip channel becomes an FL audio clip channel for each sample its clips play, each clip placed from its
//!   offset for its length.
//! - With rendered audio, each channel's render (its effects in it) is an audio clip from the song's start on a
//!   playlist track of its own, routed to the master, and the channels above are muted.
//! - The samples it plays are WAV files beside the project ([`Exported::samples`]), named as the Samplers name them.
//!
//! Lossy: sends, automation lanes, audio clips' stretch and pitch, the 3x Osc's envelopes, LFOs and arpeggiator, DW
//! Sampler envelopes, filters and velocity layers, effect settings but the Equalizer's and Lemon Studio's effects', note
//! effects, every other instrument's sound but one note of it.

use std::collections::BTreeSet;
use std::sync::Arc;

use honeydew_audio::dw_sampler::pitch_shift;
use honeydew_audio::dwp::{Cstm, Zone};
use honeydew_audio::sample::SampleData;
use honeydew_audio::wav_writer::float32_wav;

use super::mixer::{self, Band, HIGH_SHELF, LOW_SHELF, PEAKING, Slot, Track};
use super::{Chunk, ChunkBody, Event, FlpFile, Header, Tag, Value, id};
use crate::audio_engine::samples::dw_path;
use crate::params::{Role, module_spec};
use crate::render::{Remainder, Render, RenderRange, RenderSpec};
use crate::sample_cache::{SampleCache, SampleKey};
use crate::smf::export::plays;
use crate::song::plugin::{PLUGIN_EFFECT, PLUGIN_INSTRUMENT};
use crate::song::{
    Channel, ChannelKind, ChannelParams, Event as SongEvent, Lane, LaneKind, ModuleInstance, PPQ,
    SamplerSlot, Song,
};
use honeydew_audio::lemon;

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
/// FL's centred pan.
const CENTRE: i32 = 6400;
/// FL's mixer inserts.
const INSERTS: usize = 125;
/// The rate a rendered note is sampled at.
const NOTE_RATE: u32 = 44_100;

/// A song written.
#[derive(Clone, Debug, PartialEq)]
pub struct Exported {
    pub flp: Vec<u8>,
    /// The sample files, by the names the project's Samplers give them.
    pub samples: Vec<(String, Vec<u8>)>,
    /// What does not carry over exactly.
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
    /// The mixer insert it plays into.
    insert: u16,
    /// Off: muted.
    enabled: bool,
    /// The only keys it plays (a Layer's child), `lo..=hi`.
    zone: Option<(u8, u8)>,
}

enum Sound {
    Sampler(Option<String>),
    ThreeOsc(Vec<u8>, [i32; 3]),
    /// A third-party plugin: FL's plugin wrapper with its wrapper state ([`super::wrapper`]).
    Wrapper(Vec<u8>),
    /// An audio clip channel and its sample file.
    Audio(Option<String>),
    /// A Layer channel and its children, which play its notes.
    Layer(Vec<u16>),
}

/// Event 215 of a new audio clip channel as FL 26 writes it (declick mode 1, not stretched), from Lemon Studio's
/// `os-flp` (GPL-3.0-or-later).
const AUDIO_CLIP_215: &str = "8c0000000000000001000000ffffffff3c0000000000803f0000803f0000803f0000803f0000803f000000000300000000000000000400003000000000000100a7050000000000000001000000000000000000010000000000000000010000000000000000000000000000000000000002000000feffffffffffffff000000000000000000000000000000000000f03f0000000000000000ffffffff01010000000000000000e03f";

/// Honeydew's effects and FL's own effect like each, by type id.
const EFFECTS: &[(&[u16], &str)] = &[
    (&[3, 7, 30], "Fruity Parametric EQ 2"),
    (&[12, 32, 35, lemon::REVERB], "Fruity Reeverb 2"),
    (&[16, 18], "Fruity Delay 3"),
    (&[8], "Fruity Compressor"),
    (&[5, 28, lemon::LIMITER], "Fruity Limiter"),
    (&[36], "Fruity Multiband Compressor"),
    (&[9], "Fruity Filter"),
    (&[11], "Fruity Chorus"),
    (&[13], "Fruity Flanger"),
    (&[14], "Fruity Phaser"),
    (&[6], "Fruity Fast Dist"),
    (&[15], "Fruity Stereo Enhancer"),
];
/// The Equalizer, whose bands are written ([`eq_bands`]).
const EQUALIZER: u16 = 3;

/// Writes `song`; `cache` holds the samples it plays, decoded (what the engine plays). `rendered`: each channel's render
/// as a WAV file, by song channel, for the rendered audio clips (none without).
pub fn export(song: &Song, cache: &SampleCache, rendered: &[(usize, Vec<u8>)]) -> Exported {
    let mut out = Out {
        tempo: song.tempo_bpm,
        mixer: vec![Track::default()],
        ..Out::default()
    };
    for (i, channel) in song.channels.iter().enumerate() {
        match channel.kind.file_kind() {
            0x000 => out.mixer[0].slots = out.effects(channel),
            0x100 => out.channel(i, channel, cache),
            _ => {}
        }
    }
    if !rendered.is_empty() {
        out.rendered(song, rendered);
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
    /// The master, then the inserts.
    mixer: Vec<Track>,
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
            insert: self.insert(channel),
            enabled: true,
            zone: None,
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
            let zones = module.map_or_else(Vec::new, |m| dw_zones(m, cache));
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
                Some(m) if zones.len() > 1 => {
                    let n = self.layer(&base, m, zones);
                    self.map.push(((i, None), n));
                    return self.clips(i, channel);
                }
                Some(m) if !zones.is_empty() => {
                    let (zone, data) = &zones[0];
                    let file = self.sample(&channel.name, Some(data));
                    (Sound::Sampler(file), fl_key(zone.root, m))
                }
                // No counterpart: one note of it, sampled.
                Some(m) => {
                    let what = match m.type_id {
                        1 => "its DW Sampler",
                        id => module_spec(id).map_or("its instrument", |s| s.display_name),
                    };
                    let note = note_of(channel, cache);
                    let file = self.sample(&channel.name, note.as_ref());
                    self.notices.push(match (&file, m.type_id) {
                        (Some(_), _) => format!(
                            "{}: {what} has no .flp counterpart, exported as a sample of one note",
                            channel.name
                        ),
                        (None, 1) => format!("{}: {what} has no sample to export", channel.name),
                        (None, _) => format!("{}: {what} has no .flp counterpart", channel.name),
                    });
                    (Sound::Sampler(file), C5)
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
        self.clips(i, channel);
    }

    /// A DW Sampler `module` of several `zones` as a Layer channel (`base`'s name, at FL's default level) and its
    /// children, a Sampler of each zone on its keys at its root, at the channel's volume and pan. Returns the Layer.
    fn layer(
        &mut self,
        base: &FlChannel,
        module: &ModuleInstance,
        zones: Vec<(Zone, Arc<SampleData>)>,
    ) -> u16 {
        let layer = self.add(FlChannel {
            sound: Sound::Layer(Vec::new()),
            volume: 10000,
            pan: CENTRE,
            ..base.clone_levels()
        });
        let mut children = Vec::new();
        for (zone, data) in zones {
            let root = fl_key(zone.root, module);
            let name = format!(
                "{} {}",
                base.name,
                crate::ui::keyboard::key_name(root, true, false)
            );
            let file = self.sample(&name, Some(&data));
            children.push(self.add(FlChannel {
                name,
                sound: Sound::Sampler(file),
                root,
                zone: Some((fl_key(zone.key_low, module), fl_key(zone.key_high, module))),
                ..base.clone_levels()
            }));
        }
        self.channels[usize::from(layer)].sound = Sound::Layer(children);
        layer
    }

    /// A mixer insert of the channel's own, named after it, with its effects; the master when every insert is taken.
    // ponytail: past FL's 125 inserts a channel plays into the master without its effects.
    fn insert(&mut self, channel: &Channel) -> u16 {
        if self.mixer.len() > INSERTS {
            if channel.rack.modules.iter().any(|m| !is_instrument(m)) {
                self.notices.push(format!(
                    "{}: its effects have no free mixer insert",
                    channel.name
                ));
            }
            return 0;
        }
        let slots = self.effects(channel);
        self.mixer.push(Track {
            name: Some(channel.name.clone()),
            slots,
        });
        (self.mixer.len() - 1) as u16
    }

    /// The channel's effects as an insert's slot plugins, in order ([`EFFECTS`]): the Equalizer with its bands, Lemon
    /// Studio's effects with their settings, the others at FL's defaults (each a notice), a third-party plugin in FL's plugin wrapper. One with no FL counterpart,
    /// or past the insert's ten slots, is left out with a notice.
    fn effects(&mut self, channel: &Channel) -> Vec<Slot> {
        let mut slots = Vec::new();
        for m in channel.rack.modules.iter().filter(|m| !is_instrument(m)) {
            let what = match m.file.plugin.as_deref() {
                Some(plugin) => plugin.name.as_str(),
                None => module_spec(m.type_id).map_or("an effect", |s| s.display_name),
            };
            let on = m.params.first().is_none_or(|&v| v >= 0.5);
            let fl = EFFECTS.iter().find(|(ids, _)| ids.contains(&m.type_id));
            let slot = match (m.file.plugin.as_deref(), fl) {
                (Some(plugin), _) if m.type_id == PLUGIN_EFFECT => Slot {
                    plugin: super::wrapper::WRAPPER.into(),
                    state: super::wrapper::encode(None, plugin),
                    on,
                },
                (_, Some(&(_, fl))) => {
                    let state = match m.type_id {
                        EQUALIZER => mixer::eq2_state(&eq_bands(&m.params)),
                        lemon::REVERB => lemon::reverb_fl_state(&m.params),
                        lemon::LIMITER => lemon::limiter_fl_state(&m.params),
                        _ => Vec::new(),
                    };
                    if state.is_empty() {
                        self.notices.push(format!(
                            "{}: {what} is {fl} at its default settings",
                            channel.name
                        ));
                    }
                    Slot {
                        plugin: fl.into(),
                        state,
                        on,
                    }
                }
                _ => {
                    self.notices
                        .push(format!("{}: {what} has no .flp counterpart", channel.name));
                    continue;
                }
            };
            if slots.len() == mixer::SLOTS {
                self.notices
                    .push(format!("{}: {what} has no free mixer slot", channel.name));
                continue;
            }
            slots.push(slot);
        }
        slots
    }

    /// The rendered audio: each channel's render (`rendered`, by song channel) as an FL audio clip channel playing it
    /// from the song's start on a playlist track of its own, into the master (its effects are in it); every channel
    /// written before is muted.
    fn rendered(&mut self, song: &Song, rendered: &[(usize, Vec<u8>)]) {
        self.channels.iter_mut().for_each(|c| c.enabled = false);
        for (i, wav) in rendered {
            let Some((rate, frames)) = wav_len(wav) else {
                continue;
            };
            let name = format!("{} (rendered)", song.channels[*i].name);
            let file = self.file_name(&name);
            self.samples.push((file.clone(), wav.clone()));
            let n = self.add(FlChannel {
                name: name.clone(),
                color: color(song.channels[*i].hue),
                volume: 10000,
                pan: CENTRE,
                sound: Sound::Audio(Some(file)),
                root: C5,
                insert: 0,
                enabled: true,
                zone: None,
            });
            let track = self.tracks.len() as u32 + 1;
            self.tracks.push((track, name));
            let secs = frames as f64 / f64::from(rate);
            let len = (secs * self.tempo / 60.0 * f64::from(FL_PPQ))
                .round()
                .max(1.0) as u32;
            let mut r = item(0, len, 0, track - 1, false);
            r[6..8].copy_from_slice(&n.to_le_bytes());
            r[24..28].copy_from_slice(&0f32.to_le_bytes());
            r[28..32].copy_from_slice(&((secs * 1000.0) as f32).to_le_bytes());
            self.items.push(r);
        }
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

    /// A sample file's name after `name`, unique in the archive (in any case).
    fn file_name(&self, name: &str) -> String {
        let base = name.replace(['/', '\\', ':', '*', '?', '"', '<', '>', '|'], "_");
        let taken = |file: &str| {
            self.samples
                .iter()
                .any(|(f, _)| f.eq_ignore_ascii_case(file))
        };
        let mut file = format!("{base}.wav");
        let mut n = 1;
        while taken(&file) {
            n += 1;
            file = format!("{base} ({n}).wav");
        }
        file
    }

    /// `data` as a WAV file named after `name` (unique), `None` without it.
    fn sample(&mut self, name: &str, data: Option<&SampleData>) -> Option<String> {
        let data = data?;
        let file = self.file_name(name);
        let left: &[f32] = &data.left;
        let wav = match &data.right {
            Some(right) => float32_wav(data.sample_rate, &[left, right]),
            None => float32_wav(data.sample_rate, &[left]),
        };
        self.samples.push((file.clone(), wav));
        Some(file)
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
    /// first), the arrangement and its tracks, the mixer.
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
        ev.extend(mixer::events(&self.mixer));
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
    /// Its name, colour, volume, pan and insert, with no sound.
    fn clone_levels(&self) -> Self {
        FlChannel {
            name: self.name.clone(),
            color: self.color,
            volume: self.volume,
            pan: self.pan,
            sound: Sound::Sampler(None),
            root: C5,
            insert: self.insert,
            enabled: self.enabled,
            zone: None,
        }
    }
}

fn is_instrument(m: &ModuleInstance) -> bool {
    module_spec(m.type_id).is_some_and(|s| s.role == Role::Instrument)
}

/// The zones a DW Sampler plays at a forte velocity (100 of 127; all of them when none does), each with its sample:
/// its program's (`PATh`) or its own `ZONE`s. One per key range (the first of any that share one), by key, each
/// starting past the one before, so that no key plays two; none without a sample.
fn dw_zones(module: &ModuleInstance, cache: &SampleCache) -> Vec<(Zone, Arc<SampleData>)> {
    let Some(blob) = module
        .file
        .custom_data
        .as_deref()
        .filter(|_| module.type_id == 1)
    else {
        return Vec::new();
    };
    let cstm = Cstm::read(blob);
    let zones: Vec<(Zone, SampleKey)> = match cstm.program.as_ref().and_then(|p| dw_path(p.path)) {
        Some(path) => cache.program(&path).map_or_else(Vec::new, |p| p.to_vec()),
        None => (cstm.zones.iter())
            .filter_map(|(raw, zone)| Some((*zone, SampleKey::File(dw_path(raw)?))))
            .collect(),
    };
    let mut zones: Vec<(Zone, Arc<SampleData>)> = (zones.into_iter())
        .filter_map(|(zone, key)| Some((zone, cache.get(&key)?)))
        .collect();
    let forte = |z: &Zone| (z.vel_low..=z.vel_high).contains(&100);
    if zones.iter().any(|(z, _)| forte(z)) {
        zones.retain(|(z, _)| forte(z));
    }
    zones.sort_by_key(|(z, _)| (z.key_low, z.key_high));
    zones.dedup_by_key(|(z, _)| (z.key_low, z.key_high));
    let mut free = 0;
    zones.retain_mut(|(z, _)| {
        let (lo, hi) = (z.key_low.max(free), z.key_high.min(127));
        (z.key_low, z.key_high) = (lo, hi);
        free = hi.saturating_add(1);
        lo <= hi
    });
    zones
}

/// FL's key for a DW Sampler `module`'s key `key`: less its Pitch, so that FL plays it at our pitch.
fn fl_key(key: u8, module: &ModuleInstance) -> u8 {
    let shift = pitch_shift(module.params.get(3).copied().unwrap_or(0.5));
    (i32::from(key) - shift).clamp(0, 127) as u8
}

/// One note of `channel`'s instrument (its effects left out) at C5: velocity 200 for a bar at 120 BPM, then a bar to
/// ring out, at the channel's default level and pan, rendered offline. `None` when it is silent.
fn note_of(channel: &Channel, cache: &SampleCache) -> Option<SampleData> {
    let mut solo = channel.clone();
    solo.rack.modules.retain(is_instrument);
    solo.params = ChannelParams::default();
    let mut lane = Lane::new(LaneKind::Notes);
    let clip = lane.add_clip(0, 8.0);
    clip.visible_len_beats = 8.0;
    clip.events = vec![SongEvent::Note {
        pos: 0,
        len_beats: 4.0,
        key: C5,
        velocity: 200,
        pan: 127,
        slide: false,
    }];
    solo.lanes = vec![lane];
    let mut song = Song::default();
    song.channels.push(solo);
    let spec = RenderSpec {
        range: RenderRange::Full,
        remainder: Remainder::Cut,
        normalize: false,
        high_quality: true,
        rate: NOTE_RATE,
        solo: None,
        master_effects: false,
    };
    let mut stream = Render::new(&song, cache, spec).ok()?.finish().ok()?;
    let (mut left, mut right) = (Vec::new(), Vec::new());
    while !stream.done() {
        if let Some([l, r]) = stream.step() {
            left.extend_from_slice(l);
            right.extend_from_slice(r);
        }
    }
    Some(SampleData {
        sample_rate: NOTE_RATE,
        left: left.into(),
        right: Some(right.into()),
        meta: Default::default(),
    })
}

/// The Equalizer's four bands (params 1 + 4k: gain, frequency, bandwidth, kind) as Fruity Parametric EQ 2's: the same
/// type, frequency and level, and its bandwidth (octaves) as the width of the same Q by Lemon Studio's fit of FL's
/// (Q = 0.36 / width peaking, 0.575 / width shelving). Its Channel setting is not kept: every band acts on both sides.
fn eq_bands(params: &[f32]) -> Vec<Band> {
    (0..4)
        .map(|k| {
            let p = |i: usize| f64::from(params.get(1 + 4 * k + i).copied().unwrap_or(0.5));
            let hz = 2f64.powf(5.0 + 9.25 * p(1));
            let octaves = 0.25 + 7.0 * p(2);
            let q = 1.0 / (2.0 * (std::f64::consts::LN_2 / 2.0 * octaves).sinh());
            let kind = match (p(3) * 2.0).trunc() as u8 {
                0 => LOW_SHELF,
                2 => HIGH_SHELF,
                _ => PEAKING,
            };
            let fit = if kind == PEAKING { 0.36 } else { 0.575 };
            Band {
                db: -12.0 + 24.0 * p(0),
                freq: (hz / 20.0).ln() / 1000f64.ln(),
                width: fit / q,
                kind,
            }
        })
        .collect()
}

/// A WAV file's rate and frames, from its header (the export's own: 46 bytes, the data last).
fn wav_len(wav: &[u8]) -> Option<(u32, usize)> {
    let rate = u32::from_le_bytes(wav.get(24..28)?.try_into().ok()?);
    let align = u16::from_le_bytes(wav.get(32..34)?.try_into().ok()?);
    let data = wav
        .len()
        .checked_sub(honeydew_audio::wav_writer::HEADER_LEN)?;
    (rate > 0 && align > 0).then(|| (rate, data / usize::from(align)))
}

/// Event 215 of a new Sampler channel that plays only the keys `lo..=hi`: the audio clip channel's record with a
/// Sampler's own defaults where they differ, as Lemon Studio's writer makes it for a new channel (its
/// `new_sample_settings`): main pitch on (byte 11), arpeggio range 1 and no chord (ints at 44, 48), declick mode 0
/// (byte 83), the loop not following a reversal (byte 156), and the key zone (ints at 68 and 72).
fn zone_215(lo: u8, hi: u8) -> Vec<u8> {
    let mut d = audio_clip_215();
    d[11] = 1;
    d[44..48].copy_from_slice(&1i32.to_le_bytes());
    d[48..52].copy_from_slice(&(-1i32).to_le_bytes());
    d[83] = 0;
    d[156] = 0;
    d[68..72].copy_from_slice(&i32::from(lo).to_le_bytes());
    d[72..76].copy_from_slice(&i32::from(hi).to_le_bytes());
    d
}

fn audio_clip_215() -> Vec<u8> {
    (0..AUDIO_CLIP_215.len())
        .step_by(2)
        .map(|i| u8::from_str_radix(&AUDIO_CLIP_215[i..i + 2], 16).unwrap_or(0))
        .collect()
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
        Sound::Layer(_) => 3,
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
    b.push(Event::new(id::CH_ENABLED, Value::U8(u8::from(ch.enabled))));
    b.push(Event::new(id::CH_TARGET, Value::U16(ch.insert)));
    if let Sound::Layer(children) = &ch.sound {
        b.extend((children.iter()).map(|&c| Event::new(id::CH_LAYER_CHILD, Value::U16(c))));
    }
    b.push(Event::new(id::CH_LEVELS, Value::var(levels.to_vec())));
    if let Sound::Audio(_) = ch.sound {
        // FL writes the sample settings right after the levels.
        b.push(Event::new(
            id::CH_SAMPLE_SETTINGS,
            Value::var(audio_clip_215()),
        ));
    }
    b.push(Event::new(id::CH_GROUP, Value::U32(0)));
    b.push(Event::new(id::CH_LOCKED, Value::U8(0)));
    if let Sound::Sampler(Some(file)) | Sound::Audio(Some(file)) = &ch.sound {
        b.push(Event::new(id::SAMPLE_PATH, Value::var(utf16(file))));
    }
    if ch.root != C5 {
        b.push(Event::new(id::CH_ROOT, Value::U32(u32::from(ch.root))));
    }
    if let Some((lo, hi)) = ch.zone {
        b.push(Event::new(
            id::CH_SAMPLE_SETTINGS,
            Value::var(zone_215(lo, hi)),
        ));
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
