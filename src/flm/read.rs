//! The project reader: `.flm` / `.songtmpl` bytes → [`Song`].
//!
//! The fatal list fails the whole song, everything else is skipped or repaired, and the post-load fixups run. What it
//! repairs so that every song it returns can be saved again:
//! - the first channel is the master (id 0), and only it;
//! - channel ids that are missing, out of 0..=4095 or repeated get fresh ones (legacy files: 0..N−1);
//! - send buses below 1 or repeated, module ids below 2, above [`MAX_ID`] or repeated, and slot ids 0, above
//!   [`MAX_ID`] or repeated get fresh ones; every counter is raised above the ids it covers;
//! - a non-master channel without a `SEND` gets one to the master, a Drums channel without `RSMP` an empty kit;
//! - unknown chunks whose ids are not printable are dropped, and sample paths that are not app- or user-root
//!   shorthands are left empty.
//!
//! Memory: what the song holds is charged against a budget of [`BUDGET_BASE`] plus twice the file size (each item
//! pushed onto a `Vec` at twice its size, for the growth), so no input costs more than a few times its size
//! (strings are never longer than the bytes they come from).

use std::cmp::Ordering;
use std::collections::HashSet;
use std::mem::size_of;

use super::chunk::{
    Chunk, ChunkIter, ContentPath, ContentRoot, Cursor, FlmError, FlmErrorKind, MAGIC_CH01,
    MAGIC_CH02, MAGIC_CL01, MAGIC_CL02, MAGIC_DW01, MAGIC_RK01, MAGIC_SL01, MAGIC_SL02, MAGIC_SM01,
    MAGIC_TMPL, MAX_FILE_SIZE, Prefix, PrefixKind, file_chunks, text,
};
use crate::params;
use crate::song::plugin::{self, PluginParam, PluginRef};
use crate::song::{
    AutomationTarget, CHANNEL_ID_LIMIT, Channel, ChannelKind, Clip, ClipFileState, DrumGrid,
    DrumKit, Event, KeyboardState, Lane, LaneKind, MetaState, ModuleInstance, PPQ, Raw, RawChunk,
    SamplerSlot, SendState, Song, SynthKind, TimeSig,
};
use honeydew_audio::dwp::Cstm;

/// The memory every file may use whatever its size.
pub const BUDGET_BASE: usize = 64 << 20;
/// Module, slot and clip ids and their counters stay at or below this, so a counter + 1 never overflows.
pub const MAX_ID: u32 = 1 << 24;
/// Channel ids run 0..=4095, so no song has more channels.
const MAX_CHANNELS: usize = 4096;

/// [`read_with_notices`] without the notices.
pub fn read(bytes: &[u8]) -> Result<Song, FlmError> {
    read_with_notices(bytes).map(|(song, _)| song)
}

/// Reads a project, and the notices to show once it is open ("Missing rack for Channel #N skipped", N counting the
/// `CHNL`s from 1). Fails on the fatal list, on a file over the size limits, and on a file that would cost more
/// memory than its budget.
pub fn read_with_notices(bytes: &[u8]) -> Result<(Song, Vec<String>), FlmError> {
    let mut reader = Reader {
        budget: BUDGET_BASE.saturating_add(bytes.len().saturating_mul(2)),
        notices: Vec::new(),
    };
    let song = reader.song(bytes)?;
    Ok((song, reader.notices))
}

/// Reads a module preset, `.flmpst`: an `RMOd` body without its prefix, whose `HEAD` names the module type. With
/// `into`, it reads as a module of that type, as a song's module does: a `HEAD` of another type fails, but a legacy
/// Reverb (12) reads into a Reverb v2 (32) by param name ([`preset_fits`]). The module has instance id 0: the rack it
/// goes into gives it one. Fails as a song's module does, without a `HEAD`, and over the size limits.
///
/// A legacy MiniSynth `.flms` or GMSynth `.flgsynth` (told by its magic, [`super::read_flms`],
/// [`super::read_flgsynth`]) reads as its module's preset, and fails `into` any other type.
pub fn read_preset(bytes: &[u8], into: Option<u16>) -> Result<ModuleInstance, FlmError> {
    const PATH: &str = "FLMPST";
    if let Some(module) = super::legacy::read(bytes) {
        let module = module?;
        if into.is_some_and(|into| into != module.type_id) {
            return Err(FlmError::new(PATH, FlmErrorKind::BadValue));
        }
        return Ok(module);
    }
    let mut reader = Reader::for_file(bytes, PATH)?;
    let head = head_type(bytes).ok_or_else(|| FlmError::new(PATH, FlmErrorKind::MissingChunk))?;
    let file_type = into.map_or(head, u32::from);
    // The prefix a rack's `RMOd` has: the type, and instance id 0.
    let payload = [&file_type.to_le_bytes()[..], &[0; 4], bytes].concat();
    let chunk = Chunk {
        id: *b"RMOd",
        payload: &payload,
        path: PATH.into(),
    };
    Ok(reader.module(&chunk)?.1)
}

/// Whether the module preset `bytes` reads into a module of type `type_id` ([`read_preset`]): its `HEAD` names that
/// type, or a legacy Reverb (12) for a Reverb v2 (32). Only the `HEAD` is looked at; a legacy `.flms` or `.flgsynth`
/// fits its own module by its magic, and so does a DW Sampler program (`.dwp`), which its own loader takes.
pub fn preset_fits(bytes: &[u8], type_id: u16) -> bool {
    if bytes.starts_with(b"DwPr") {
        return type_id == 1;
    }
    match super::legacy::legacy_type(bytes) {
        Some(legacy) => legacy == type_id,
        None => head_type(bytes).is_some_and(|head| fits(head, type_id.into())),
    }
}

/// The module type the preset `bytes` loads into ([`read_preset`] with no `into`): a DW Sampler program's (1), a legacy
/// `.flms` or `.flgsynth`'s by its magic, else the one its `HEAD` names. `None` for none of those.
pub fn preset_type(bytes: &[u8]) -> Option<u16> {
    if bytes.starts_with(b"DwPr") {
        return Some(1);
    }
    super::legacy::legacy_type(bytes).or_else(|| u16::try_from(head_type(bytes)?).ok())
}

/// The module type a module preset's `HEAD` names.
fn head_type(bytes: &[u8]) -> Option<u32> {
    let head = ChunkIter::new(bytes, "")
        .map_while(Result::ok)
        .find(|c| c.id == *b"HEAD")?;
    Some(u32::from_le_bytes(head.payload.get(..4)?.try_into().ok()?))
}

/// A module of type `head` (its `HEAD`) reads into a module of type `into` (its prefix): the same type, or a legacy
/// Reverb into a Reverb v2.
fn fits(head: u32, into: u32) -> bool {
    head == into || (into == 32 && head == 12)
}

/// Reads a drum kit template, `.tmpl`: `"LPMT" [PADS(22)] slot*`, the slots `SMPl` or legacy `SMPL` as in an
/// `RSMP`, each on the pad its `MAIN` pad index names. The slots keep the file's ids (fresh ones for missing or
/// repeated ids) and the counter covers them. Fails as a song's kit does, on another magic, and over the size
/// limits.
pub fn read_drumset(bytes: &[u8]) -> Result<DrumKit, FlmError> {
    const PATH: &str = "TMPL";
    let mut reader = Reader::for_file(bytes, PATH)?;
    Cursor::new(bytes, PATH).expect_magic(MAGIC_TMPL)?;
    // The `RSMP` of the same chunks, with lastSlotId 0: the slots' ids set the counter.
    let payload = [&MAGIC_SM01[..], &[0; 4], &bytes[4..]].concat();
    let chunk = Chunk {
        id: *b"RSMP",
        payload: &payload,
        path: PATH.into(),
    };
    Ok(reader.kit(&chunk)?.0)
}

/// A plugin module's `PLUG` (`flm::write`), without its state.
fn plugin_ref(chunk: &Chunk<'_>) -> Result<PluginRef, FlmError> {
    let mut reader = chunk.reader();
    let mut texts = [(); 4].map(|_| String::new());
    for value in &mut texts {
        *value = text(reader.vstr16()?);
    }
    let [format, id, name, vendor] = texts;
    let path = text(reader.vstr32()?);
    let count = reader.u32()? as usize;
    // Each param takes at least 10 bytes: a count past what is left is broken.
    if count > reader.remaining() / 10 {
        return Err(FlmError::new(&chunk.path, FlmErrorKind::BadValue));
    }
    let mut params = Vec::with_capacity(count);
    for _ in 0..count {
        let id = reader.u32()?;
        let default = reader.f32()?;
        params.push(PluginParam {
            id,
            default,
            name: text(reader.vstr16()?),
        });
    }
    Ok(PluginRef {
        format,
        id,
        name,
        vendor,
        path,
        params,
        ..PluginRef::default()
    })
}

/// Reads a rack preset, `.flmrack`: its modules in chain order. Their ids are the saving rack's, so each takes a
/// fresh one after `last_instance_id`, the counter of the rack they go into, which then covers them. A file that fails
/// (as a song's module does, or over the size limits) leaves the counter as it was.
pub fn read_rack(
    bytes: &[u8],
    last_instance_id: &mut i32,
) -> Result<Vec<ModuleInstance>, FlmError> {
    const PATH: &str = "FLMRACK";
    let mut reader = Reader::for_file(bytes, PATH)?;
    let mut last = *last_instance_id;
    let mut modules = Vec::new();
    let mut plugins = Vec::new();
    for chunk in ChunkIter::new(bytes, PATH) {
        let chunk = chunk?;
        if chunk.id == *b"HDPm" {
            plugins.extend(reader.plugin_module(&chunk)?);
        } else if matches!(&chunk.id, b"RMOd" | b"RMOD") {
            modules.push(reader.module(&chunk)?.1);
        }
    }
    // The plugin modules back at their places in the chain.
    plugins.sort_by_key(|&(position, ..)| position);
    for (position, _, module) in plugins {
        modules.insert(position.min(modules.len()), module);
    }
    for module in &mut modules {
        // Ids are ≥ 2 and at most MAX_ID.
        last = last.clamp(1, MAX_ID as i32) + 1;
        if last > MAX_ID as i32 {
            return Err(FlmError::new(PATH, FlmErrorKind::TooLarge));
        }
        module.instance_id = last;
    }
    *last_instance_id = last;
    Ok(modules)
}

/// `Ok` when module `type_id` may carry the `CSTM` `payload` (the allowed modules, their minimum sizes, DirectWave's
/// magic and ZONE sizes). The writer checks the same.
pub(super) fn check_cstm(type_id: u16, payload: &[u8], path: &str) -> Result<(), FlmError> {
    if !matches!(type_id, 1 | 2 | 16 | 24 | 25 | 26 | 33 | 39 | 45 | 52) {
        return Err(FlmError::new(path, FlmErrorKind::BadValue));
    }
    let minimum = match type_id {
        1 => 4,
        2 => 8,
        24 => 16,
        26 => 128,
        _ => 0,
    };
    if payload.len() < minimum {
        return Err(FlmError::new(path, FlmErrorKind::TooShort));
    }
    if type_id == 1 {
        let mut reader = Cursor::new(payload, path);
        reader.expect_magic(MAGIC_DW01)?;
        for item in ChunkIter::new(reader.rest(), path) {
            let item = item?;
            if item.id == *b"ZONE" && item.payload.len() < 1065 {
                return Err(FlmError::new(&item.path, FlmErrorKind::TooShort));
            }
        }
    }
    Ok(())
}

/// Decodes an `EVN2` payload of a lane of `kind` whose `DESc` param is `file_param`, sorted by position.
/// Short or empty payloads give no events. Each event is charged to `budget`; `None` when it runs out.
pub(super) fn decode_events(
    payload: &[u8],
    kind: LaneKind,
    file_param: i32,
    budget: &mut usize,
) -> Option<Vec<Event>> {
    let mut reader = Cursor::new(payload, "");
    let Ok(size) = reader.u16().map(usize::from) else {
        return Some(Vec::new());
    };
    if size == 0 {
        return Some(Vec::new());
    }
    let count = reader.remaining() / size;
    *budget = budget.checked_sub(count.checked_mul(size_of::<Event>())?)?;
    let mut events = Vec::with_capacity(count);
    // The control-point sign: a control point's key is +(param + 1), which is negative for −2 and −3. With
    // param −1 the key is 0 and carries no sign: every point is a control point.
    let control_sign = i64::from(file_param) + 1 > 0;
    let unsigned = file_param == -1;
    for record in reader.rest().chunks_exact(size) {
        let mut item = Cursor::new(record, "");
        let pos = item.u32_or(0);
        let len_beats = item.f64_or(0.0);
        let key = item.i16_or(0);
        let a = item.u8_or(Event::DEFAULT_VELOCITY);
        let b = item.u8_or(Event::DEFAULT_PAN);
        let value = item.u16_or(Event::DEFAULT_PITCH);
        let repeat = item.u8_or(0);
        let slide = item.u8_or(0) != 0;
        events.push(match kind {
            LaneKind::Notes => Event::Note {
                pos,
                len_beats,
                key: key.clamp(0, 127) as u8,
                velocity: a,
                pan: b,
                slide,
            },
            LaneKind::Drums => Event::Drum {
                pos,
                len_beats,
                pad: key.clamp(0, 255) as u8,
                velocity: a,
                pan: b,
                value,
                repeat,
            },
            LaneKind::Automation => Event::Automation {
                pos,
                value: f32::from(value) / 65535.0,
                control: unsigned || (key > 0) == control_sign,
            },
            LaneKind::Audio => Event::Envelope {
                pos,
                env_param: (i32::from(key).abs() - 1) as i16,
                value: f32::from(value) / 65535.0,
                control: key > 0,
            },
        });
    }
    // Re-sorted by position, equal positions in file order. A stable sort takes scratch of half the events.
    if !events.is_sorted_by_key(Event::pos) {
        *budget = budget.checked_sub(count / 2 * size_of::<Event>())?;
        events.sort_by_key(Event::pos);
    }
    Some(events)
}

struct Reader {
    /// Bytes the song may still take.
    budget: usize,
    notices: Vec<String>,
}

/// A rack read, before its channel is paired and placed.
struct RackRead {
    kind: u32,
    channel: Channel,
    path: String,
    /// `CHHD` @1028.
    row: f64,
    /// `CHHD` base name: names the channel kind in our files ("<kind label> <id>").
    base_name: String,
    /// The kit's `MAIN@4` pad indexes in slot order, when they are not 0, 1, 2 …: drum hits are re-keyed by them.
    pads: Vec<f64>,
}

impl Reader {
    /// A reader for a preset file of `bytes`, with the budget a song of that size has.
    fn for_file(bytes: &[u8], path: &str) -> Result<Self, FlmError> {
        if bytes.len() > MAX_FILE_SIZE {
            return Err(FlmError::new(path, FlmErrorKind::TooLarge));
        }
        Ok(Self {
            budget: BUDGET_BASE.saturating_add(bytes.len().saturating_mul(2)),
            notices: Vec::new(),
        })
    }

    fn charge(&mut self, bytes: usize, path: &str) -> Result<(), FlmError> {
        self.budget = (self.budget.checked_sub(bytes))
            .ok_or_else(|| FlmError::new(path, FlmErrorKind::TooLarge))?;
        Ok(())
    }

    /// Keeps an unknown chunk, unless its id could not be written back.
    fn unknown(&mut self, chunk: &Chunk<'_>, into: &mut Vec<RawChunk>) -> Result<(), FlmError> {
        if !chunk.id.iter().all(|byte| (0x20..=0x7e).contains(byte)) {
            return Ok(());
        }
        self.charge(2 * size_of::<RawChunk>() + chunk.payload.len(), &chunk.path)?;
        into.push(RawChunk {
            id: chunk.id,
            payload: chunk.payload.to_vec(),
        });
        Ok(())
    }

    fn song(&mut self, bytes: &[u8]) -> Result<Song, FlmError> {
        let mut song = Song {
            tempo_bpm: 120.0,
            channels: Vec::new(),
            ..Song::default()
        };
        // An absent @353 is 0.
        song.file.play_truncated_notes = false;
        let mut next_channel_id = None;
        let mut racks = Vec::new();
        let mut chnls = Vec::new();
        for chunk in file_chunks(bytes)? {
            let chunk = chunk?;
            match &chunk.id {
                b"HEAD" => next_channel_id = head(&chunk, &mut song)?,
                b"TDIV" => {
                    let Some((&[subdivision, beats_per_bar], extra)) =
                        chunk.payload.split_first_chunk()
                    else {
                        return Err(FlmError::new(&chunk.path, FlmErrorKind::TooShort));
                    };
                    song.time_sig = TimeSig {
                        subdivision,
                        beats_per_bar,
                    };
                    song.file.time_sig_extra = extra.to_vec();
                }
                b"META" => song.file.meta = meta(chunk.payload),
                // A broken KEYB is only logged.
                b"KEYB" => song.file.keyboard = keyboard(&chunk).unwrap_or_default(),
                b"RACK" => {
                    if racks.len() >= MAX_CHANNELS {
                        return Err(FlmError::new(&chunk.path, FlmErrorKind::TooLarge));
                    }
                    racks.push(self.rack(&chunk)?);
                }
                b"CHNL" => {
                    self.charge(2 * size_of::<Chunk>() + chunk.path.len(), &chunk.path)?;
                    chnls.push(chunk);
                }
                _ => self.unknown(&chunk, &mut song.file.unknown_chunks)?,
            }
        }
        // Channels sort by rack kind, equal kinds in file order; CHNL i pairs with the i-th.
        racks.sort_by_key(|rack: &RackRead| rack.kind);
        for (order, rack) in racks.iter_mut().enumerate() {
            rack.channel.file.rack_order = Raw(Some(order));
        }
        for (index, chunk) in chnls.iter().enumerate() {
            match racks.get_mut(index) {
                Some(rack) => {
                    (rack.row, rack.base_name) = self.channel(chunk, &mut rack.channel)?
                }
                None => {
                    (self.notices).push(format!("Missing rack for Channel #{} skipped", index + 1))
                }
            }
        }
        if racks.is_empty() {
            return Err(FlmError::new("FLM", FlmErrorKind::MissingChunk));
        }
        if let Some(rack) = racks.iter().find(|rack| rack.channel.lanes.is_empty()) {
            return Err(FlmError::new(&rack.path, FlmErrorKind::MissingChunk));
        }
        // Channel 0 is the master, and only it.
        for (index, rack) in racks.iter_mut().enumerate() {
            rack.kind = match (index, rack.kind) {
                (0, _) => 0,
                (_, 0) => 0x100,
                (_, kind) => kind,
            };
        }
        racks.sort_by_key(|rack| rack.kind);
        for rack in &mut racks {
            settle_channel(rack);
        }
        let legacy = next_channel_id.is_none();
        assign_channel_ids(&mut racks, next_channel_id);
        let rows: Vec<f64> = racks.iter().map(|rack| rack.row).collect();
        let mut channels: Vec<Channel> = racks.into_iter().map(|rack| rack.channel).collect();
        if legacy {
            legacy_routing(&mut channels);
        }
        settle_routing(&mut channels, &mut song.file.last_send_bus);
        let max_id = channels.iter().map(|c| c.id).max().unwrap_or(0);
        song.file.next_channel_id =
            (next_channel_id.unwrap_or(0).max(max_id + 1)).min(CHANNEL_ID_LIMIT);
        // The channels take their playlist order (CHHD @1028), the master first. Duplicate rows are renumbered (every
        // other channel at or after one moves down a row, in file order); that keeps the order of distinct rows and
        // puts equal ones in file order, which is what a stable sort does.
        let mut order: Vec<usize> = (0..channels.len()).collect();
        let row = |a: usize, b: usize| rows[a].partial_cmp(&rows[b]).unwrap_or(Ordering::Equal);
        order.sort_by(|&a, &b| (a != 0).cmp(&(b != 0)).then(row(a, b)));
        // The master keeps channel index 0 and its own place among the rows (read first, it stays above an equal row).
        song.master_below = rows.first().and_then(|&master| {
            let above = order.iter().skip(1).rev().find(|&&c| rows[c] < master)?;
            Some(channels[*above].id)
        });
        let mut channels: Vec<Option<Channel>> = channels.into_iter().map(Some).collect();
        song.channels = order.iter().filter_map(|&i| channels[i].take()).collect();
        Ok(song)
    }

    fn rack(&mut self, chunk: &Chunk<'_>) -> Result<RackRead, FlmError> {
        let (prefix, children) = chunk.container_expect(PrefixKind::U32Magic, &[MAGIC_RK01])?;
        let Prefix::U32Magic(kind, _) = prefix else {
            return Err(FlmError::new(&chunk.path, FlmErrorKind::BadValue));
        };
        self.charge(2 * size_of::<Channel>(), &chunk.path)?;
        // The kind is set once the channel is read (a Drums `Channel::new` would bring the default kit).
        let mut channel = Channel::new(String::new(), ChannelKind::Instrument, 0.6);
        channel.lanes.clear();
        let mut module_ids = Vec::new();
        let mut plugins = Vec::new();
        let mut pads = Vec::new();
        for child in children {
            let child = child?;
            match &child.id {
                b"RHED" => {
                    if child.payload.len() < 16 {
                        return Err(FlmError::new(&child.path, FlmErrorKind::TooShort));
                    }
                    let mut reader = child.reader();
                    reader.i32()?;
                    channel.rack.last_instance_id = reader.i32()?;
                    channel.bus_id = reader.i32()?;
                    channel.output_bus = reader.i32()?;
                    channel.file.rack_header_extra = reader.rest().to_vec();
                }
                b"RPRM" => {
                    if child.payload.len() < 24 {
                        return Err(FlmError::new(&child.path, FlmErrorKind::TooShort));
                    }
                    let mut reader = child.reader();
                    for value in &mut channel.params.0 {
                        *value = reader.f32()?;
                    }
                    channel.file.params_extra = reader.rest().to_vec();
                }
                b"CCLK" => channel.file.cc_links = child.payload.to_vec(),
                b"RSMP" => {
                    let (kit, kit_pads) = self.kit(&child)?;
                    channel.drum_kit = Some(kit);
                    let identity = kit_pads.iter().enumerate().all(|(i, &pad)| pad == i as f64);
                    pads = if identity { Vec::new() } else { kit_pads };
                }
                b"RMOd" | b"RMOD" => {
                    let (id, module) = self.module(&child)?;
                    module_ids.push(id);
                    channel.rack.modules.push(module);
                }
                b"HDPm" => plugins.extend(self.plugin_module(&child)?),
                _ => self.unknown(&child, &mut channel.file.rack_unknown_chunks)?,
            }
        }
        // The plugin modules back at their places in the chain.
        plugins.sort_by_key(|&(position, ..)| position);
        for (position, id, module) in plugins {
            let at = position.min(module_ids.len());
            module_ids.insert(at, id);
            channel.rack.modules.insert(at, module);
        }
        // Module ids are ≥ 2 and unique; the counter covers them.
        let rack = &mut channel.rack;
        let valid = |id: u32| (2..=MAX_ID).contains(&id);
        let max = module_ids
            .iter()
            .copied()
            .filter(|&id| valid(id))
            .max()
            .unwrap_or(0);
        rack.last_instance_id = (rack.last_instance_id.clamp(1, MAX_ID as i32)).max(max as i32);
        let mut used = HashSet::new();
        for (module, id) in rack.modules.iter_mut().zip(module_ids) {
            if valid(id) && used.insert(id) {
                module.instance_id = id as i32;
            } else {
                rack.last_instance_id += 1;
                module.instance_id = rack.last_instance_id;
            }
        }
        Ok(RackRead {
            kind,
            channel,
            path: chunk.path.clone(),
            row: 0.0,
            base_name: String::new(),
            pads,
        })
    }

    /// An `RSMP`, and its slots' `MAIN@4` pad indexes in slot order.
    fn kit(&mut self, chunk: &Chunk<'_>) -> Result<(DrumKit, Vec<f64>), FlmError> {
        let (prefix, children) = chunk.container_expect(PrefixKind::MagicU32, &[MAGIC_SM01])?;
        let Prefix::MagicU32(_, last_slot_id) = prefix else {
            return Err(FlmError::new(&chunk.path, FlmErrorKind::BadValue));
        };
        let mut kit = DrumKit::default();
        let mut slots = Vec::new();
        for child in children {
            let child = child?;
            match &child.id {
                b"PADS" => {
                    let Some((pads, _)) = child.payload.split_first_chunk() else {
                        return Err(FlmError::new(&child.path, FlmErrorKind::TooShort));
                    };
                    kit.file.pads = *pads;
                }
                b"SMPl" | b"SMPL" => slots.push(self.slot(&child)?),
                _ => self.unknown(&child, &mut kit.file.unknown_chunks)?,
            }
        }
        // Slot ids are ≥ 1, unique in the kit and ≤ lastSlotId; the first in file order keeps its id.
        let valid = |id: u32| (1..=MAX_ID).contains(&id);
        let max = (slots.iter())
            .map(|(_, s)| s.file.id)
            .filter(|&id| valid(id))
            .max()
            .unwrap_or(0);
        kit.file.last_slot_id = last_slot_id.min(MAX_ID).max(max);
        let mut used = HashSet::new();
        for (_, slot) in &mut slots {
            if !(valid(slot.file.id) && used.insert(slot.file.id)) {
                kit.file.last_slot_id += 1;
                slot.file.id = kit.file.last_slot_id;
            }
            // A pad's stretch mode is on exactly when its ratio is not 1, which is what the writer
            // saves, whatever mode the file held (`raw_stretch_state` keeps the file's, so a record that differs is
            // rewritten).
            slot.file.stretch_mode = u8::from(slot.file.stretch_ratio != 1.0);
        }
        // A slot sits on the pad its `MAIN` pad index names: slot i on pad i.
        slots.sort_by(|a, b| a.0.total_cmp(&b.0));
        let (pads, slots) = slots.into_iter().unzip();
        kit.slots = slots;
        Ok((kit, pads))
    }

    /// A slot and its `MAIN` pad index.
    fn slot(&mut self, chunk: &Chunk<'_>) -> Result<(f64, SamplerSlot), FlmError> {
        let (prefix, children) =
            chunk.container_expect(PrefixKind::Magic, &[MAGIC_SL02, MAGIC_SL01])?;
        self.charge(2 * size_of::<SamplerSlot>(), &chunk.path)?;
        let mut slot = SamplerSlot::default();
        let mut pad_index = -1.0;
        let (mut has_main, mut has_link) = (false, false);
        let (mut strc, mut str1) = (None, None);
        for child in children {
            let child = child?;
            match &child.id {
                b"MAIN" => {
                    if child.payload.len() < 1036 {
                        return Err(FlmError::new(&child.path, FlmErrorKind::TooShort));
                    }
                    has_main = true;
                    let mut reader = child.reader();
                    reader.i32()?;
                    pad_index = reader.f64()?;
                    slot.name = reader.char::<512>()?;
                    slot.folder = reader.char::<512>()?.replace('\\', "/");
                    slot.file.reverse = reader.u8_or(0) != 0;
                    slot.file.beat_offset = reader.f64_or(0.0);
                    slot.file.beat_offset_control = reader.f32_or(0.5);
                    slot.cut_group = reader.u8_or(0);
                    slot.file.overlapping_voices = reader.u8_or(1) != 0;
                    let mut name = reader.clone();
                    if let Ok(custom) = name.vstr16() {
                        slot.file.custom_pad_name = text(custom);
                        reader = name;
                    }
                    slot.file.main_extra = reader.rest().to_vec();
                }
                b"LINk" => {
                    if child.payload.len() < 4 {
                        return Err(FlmError::new(&child.path, FlmErrorKind::TooShort));
                    }
                    has_link = true;
                    let mut reader = child.reader();
                    slot.file.id = reader.u32()?;
                    slot.file.output_bus = reader.i32_or(-1);
                    slot.file.midi_key = reader.u16_or(u16::MAX);
                    slot.file.send_target = reader.i32_or(-1);
                    slot.file.link_extra = reader.rest().to_vec();
                }
                b"STRC" | b"STR1" => {
                    if child.payload.len() < 17 {
                        return Err(FlmError::new(&child.path, FlmErrorKind::TooShort));
                    }
                    let mut reader = child.reader();
                    let state = (reader.u8()?, reader.f64()?, reader.f64()?);
                    if child.id == *b"STR1" {
                        str1 = Some(state);
                        slot.file.raw_str1 = Raw(Some(child.payload.to_vec()));
                    } else {
                        strc = Some(state);
                        slot.file.raw_strc = Raw(Some(child.payload.to_vec()));
                    }
                }
                b"PTH1" => slot.file.path = child.reader().content_path16().ok().filter(writable),
                b"PATH" => {
                    slot.file.path = child.reader().content_path::<2048>().ok().filter(writable)
                }
                b"PRST" => {
                    let mut reader = child.reader();
                    if let (Ok(path), Ok(provenance)) = (reader.vstr32(), reader.vstr32()) {
                        slot.file.preset_path = text(path);
                        slot.file.provenance = text(provenance);
                    }
                }
                b"PRMS" => {
                    let mut reader = child.reader();
                    for value in slot.params.iter_mut().take(child.payload.len() / 4) {
                        *value = reader.f32()?;
                    }
                }
                _ => self.unknown(&child, &mut slot.file.unknown_chunks)?,
            }
        }
        // A slot needs its MAIN and (in the v2 layout) its LINk id.
        if !has_main || (!has_link && prefix == Prefix::Magic(MAGIC_SL02)) {
            return Err(FlmError::new(&chunk.path, FlmErrorKind::MissingChunk));
        }
        // STR1 wins; STRC alone forces mode 1 and rounds the pitch ratio to semitones.
        let state = str1.or_else(|| {
            strc.map(|(_, ratio, pitch)| {
                // A pitch of 0 or less has no finite log.
                let semitones = Some((12.0 * pitch.log2()).round()).filter(|s| s.is_finite());
                (1, ratio, semitones.unwrap_or(0.0))
            })
        });
        if let Some(state) = state {
            (
                slot.file.stretch_mode,
                slot.file.stretch_ratio,
                slot.file.pitch_semitones,
            ) = state;
            slot.file.raw_stretch_state = Raw(Some(state));
        }
        Ok((pad_index, slot))
    }

    /// A module and the instance id from its prefix (checked by the rack).
    fn module(&mut self, chunk: &Chunk<'_>) -> Result<(u32, ModuleInstance), FlmError> {
        let (prefix, mut children) = chunk.container(PrefixKind::TwoU32)?;
        let Prefix::TwoU32(file_type, instance_id) = prefix else {
            return Err(FlmError::new(&chunk.path, FlmErrorKind::BadValue));
        };
        let type_id = (u16::try_from(file_type).ok())
            .filter(|&id| params::module_spec(id).is_some())
            .ok_or_else(|| FlmError::new(&chunk.path, FlmErrorKind::BadValue))?;
        let mut module = ModuleInstance::new(type_id, 0);
        // The module, its params and its three default strings.
        let cost = 2 * size_of::<ModuleInstance>() + 4 * module.params.len() + 64;
        self.charge(cost, &chunk.path)?;
        let mut legacy_reverb = false;
        for child in children.by_ref() {
            let child = child?;
            match &child.id {
                b"HEAD" => {
                    if child.payload.len() < 9 {
                        return Err(FlmError::new(&child.path, FlmErrorKind::TooShort));
                    }
                    let mut reader = child.reader();
                    let head_type = reader.u32()?;
                    legacy_reverb = head_type != file_type;
                    if !fits(head_type, file_type) {
                        return Err(FlmError::new(&child.path, FlmErrorKind::BadValue));
                    }
                    module.file.head_reserved = reader.u32()?;
                    module.file.head_enabled = reader.u8()?;
                    module.file.head_extra = reader.rest().to_vec();
                }
                b"DESC" => {
                    if child.payload.len() < 512 {
                        return Err(FlmError::new(&child.path, FlmErrorKind::TooShort));
                    }
                    let mut reader = child.reader();
                    module.file.category = reader.char::<256>()?.replace('\\', "/");
                    module.file.preset_name = reader.char::<256>()?;
                    // Then a vstr16 preset path, or u32 0 for none.
                    let mut path = reader.clone();
                    if child.payload.get(512..516) == Some(&[0; 4]) {
                        reader.skip(4)?;
                    } else if let Ok(bytes) = path.vstr16() {
                        module.file.preset_path = text(bytes);
                        reader = path;
                    }
                    module.file.desc_extra = reader.rest().to_vec();
                }
                // An unreadable PRST fails the song.
                b"PRST" => {
                    let mut reader = child.reader();
                    module.file.source_path = text(reader.vstr32()?);
                    module.file.provenance = text(reader.vstr32()?);
                }
                b"ADD1" => {
                    let Some((&[minimized, low, high], extra)) = child.payload.split_first_chunk()
                    else {
                        return Err(FlmError::new(&child.path, FlmErrorKind::TooShort));
                    };
                    module.minimized = minimized != 0;
                    (module.file.key_low, module.file.key_high) = key_range(low as i8, high as i8);
                    module.file.add1_extra = extra.to_vec();
                }
                b"PRMS" => {
                    let mut reader = child.reader();
                    let values: Vec<f32> = (0..child.payload.len() / 4)
                        .map_while(|_| reader.f32().ok())
                        .collect();
                    if legacy_reverb {
                        legacy_reverb_params(&mut module.params, &values);
                    } else if plugin::is_plugin(type_id) && !values.is_empty() {
                        // A plugin module's params are Enable and then the plugin's, as many as it has.
                        self.charge(4 * values.len(), &child.path)?;
                        module.params = values;
                    } else {
                        for (param, value) in module.params.iter_mut().zip(values) {
                            *param = value;
                        }
                    }
                }
                b"SMPR" => {
                    let mut reader = child.reader();
                    for value in module.common.0.iter_mut().take(child.payload.len() / 4) {
                        *value = reader.f32()?;
                    }
                }
                b"CSTM" => {
                    check_cstm(type_id, child.payload, &child.path)?;
                    module.file.custom_data = Some(child.payload.to_vec());
                }
                b"PLUG" if plugin::is_plugin(type_id) => {
                    let state = module
                        .file
                        .plugin
                        .take()
                        .map(|p| p.state)
                        .unwrap_or_default();
                    let mut read = plugin_ref(&child)?;
                    self.charge(read.params.len() * size_of::<PluginParam>(), &child.path)?;
                    read.state = state;
                    module.file.plugin = Some(Box::new(read));
                }
                b"PSTA" if plugin::is_plugin(type_id) => {
                    self.charge(child.payload.len(), &child.path)?;
                    let plugin = module.file.plugin.get_or_insert_default();
                    plugin.state = child.payload.into();
                }
                _ => self.unknown(&child, &mut module.file.unknown_chunks)?,
            }
        }
        // A DW Sampler's `PATh` floats are its Attack and Release init values, applied (with the params) once
        // its program loads, which needs the file (`panels::cstm::load_dw_program`); until then they are kept.
        if type_id == 1 {
            let blob = module.file.custom_data.as_deref().unwrap_or_default();
            module.keep_program_init(Cstm::read(blob).program.and_then(|p| p.init));
        }
        // Legacy RMOD: u8 minimized, i8 keyLow, i8 keyHigh after the chunks.
        if chunk.id == *b"RMOD" {
            let tail = children.remainder();
            module.minimized |= tail.first().is_some_and(|&minimized| minimized != 0);
            if let [_, low, high, ..] = *tail {
                (module.file.key_low, module.file.key_high) = key_range(low as i8, high as i8);
            }
        }
        Ok((instance_id, module))
    }

    /// An `HDPm` (`flm::write`): its position in the chain and its plugin module with the instance id from its
    /// `RMOd`. `None` for a layout other than 1 or no `RMOd`, which is skipped as an unknown chunk would be.
    fn plugin_module(
        &mut self,
        chunk: &Chunk<'_>,
    ) -> Result<Option<(usize, u32, ModuleInstance)>, FlmError> {
        let (Prefix::TwoU32(position, 1), children) = chunk.container(PrefixKind::TwoU32)? else {
            return Ok(None);
        };
        for child in children {
            let child = child?;
            if child.id == *b"RMOd" {
                let (id, module) = self.module(&child)?;
                if plugin::is_plugin(module.type_id) {
                    return Ok(Some((position as usize, id, module)));
                }
            }
        }
        Ok(None)
    }

    /// Reads a `CHNL` into its rack's channel; returns its playlist row and base name.
    fn channel(
        &mut self,
        chunk: &Chunk<'_>,
        channel: &mut Channel,
    ) -> Result<(f64, String), FlmError> {
        let (_, children) =
            chunk.container_expect(PrefixKind::U32Magic, &[MAGIC_CH02, MAGIC_CH01])?;
        let (mut row, mut base_name) = (0.0, String::new());
        let mut targets = HashSet::new();
        // Audio clips' samples get slot ids after the kit's, in file order (CLSm).
        let mut next_slot = channel
            .drum_kit
            .as_ref()
            .map_or(0, |kit| kit.file.last_slot_id);
        for child in children {
            let child = child?;
            match &child.id {
                b"CHHD" => (row, base_name) = header(&child, channel)?,
                b"SEND" => {
                    let send = send(&child);
                    // A repeated target is ignored.
                    if targets.insert(send.target_channel_id) {
                        self.charge(2 * size_of::<SendState>(), &child.path)?;
                        channel.file.sends.push(send);
                    }
                }
                b"TRKH" => {
                    if let Some(lane) = self.lane(&child, &mut next_slot)? {
                        channel.lanes.push(lane);
                    }
                }
                _ => self.unknown(&child, &mut channel.file.channel_unknown_chunks)?,
            }
        }
        if channel.lanes.is_empty() {
            return Err(FlmError::new(&chunk.path, FlmErrorKind::MissingChunk));
        }
        Ok((row, base_name))
    }

    /// `TRKH`; `None` (skipped) without a `DESc` or with an unknown lane kind. The first `DESc` is read first,
    /// then the clips as they come (none is held back unread).
    fn lane(&mut self, chunk: &Chunk<'_>, next_slot: &mut u32) -> Result<Option<Lane>, FlmError> {
        let Some(desc) = (chunk.children()).find_map(|c| c.ok().filter(|c| c.id == *b"DESc"))
        else {
            return Ok(None);
        };
        let Some((mut lane, file_param)) = self.desc(&desc)? else {
            return Ok(None);
        };
        let mut seen_desc = false;
        for child in chunk.children() {
            let child = child?;
            match &child.id {
                b"CLIP" => {
                    let clip = self.clip(&child, lane.kind, file_param, next_slot)?;
                    lane.clips.push(clip);
                }
                b"DESc" if !seen_desc => seen_desc = true,
                _ => self.unknown(&child, &mut lane.file.unknown_chunks)?,
            }
        }
        // Clips are sorted by start tick; the counter covers their ids.
        lane.clips.sort_by_key(|clip| clip.start_tick);
        let max = lane.clips.iter().map(|clip| clip.id).max().unwrap_or(0);
        lane.last_clip_id = lane.last_clip_id.max(max).min(MAX_ID);
        Ok(Some(lane))
    }

    /// `DESc`: the lane it describes and its file param; `None` for an unknown lane kind.
    fn desc(&mut self, chunk: &Chunk<'_>) -> Result<Option<(Lane, i32)>, FlmError> {
        if chunk.payload.len() < 1052 {
            return Err(FlmError::new(&chunk.path, FlmErrorKind::TooShort));
        }
        let kind = match chunk.payload[0] {
            0 => LaneKind::Notes,
            1 => LaneKind::Automation,
            2 => LaneKind::Audio,
            3 => LaneKind::Drums,
            _ => return Ok(None),
        };
        self.charge(2 * size_of::<Lane>(), &chunk.path)?;
        let mut reader = chunk.reader();
        reader.skip(1)?;
        let mut lane = Lane::new(kind);
        // @1: a drum lane's grid (any byte but 0 is a notes lane); other lanes keep it as read.
        match (kind, reader.u8()?) {
            (LaneKind::Drums, 0) => {}
            (LaneKind::Drums, _) => lane.grid = DrumGrid::Notes,
            (_, flag) => lane.file.flag = flag,
        }
        reader.u16()?;
        let instance_id = reader.i32()?;
        let file_param = reader.i32()?;
        lane.swing = reader.f64()?;
        lane.last_clip_id = reader.u32()?;
        lane.file.current_clip_id = reader.u32()?;
        lane.label = reader.char::<1024>()?;
        // Without the vstr32 (older files) the char name is the custom label.
        let mut custom = reader.clone();
        lane.file.custom_label = match custom.vstr32() {
            Ok(label) => {
                reader = custom;
                text(label)
            }
            Err(_) => lane.label.clone(),
        };
        lane.file.desc_extra = reader.rest().to_vec();
        if kind == LaneKind::Automation {
            let param = AutomationTarget::model_param(instance_id, file_param);
            lane.target = Some(AutomationTarget { instance_id, param });
        }
        Ok(Some((lane, file_param)))
    }

    fn clip(
        &mut self,
        chunk: &Chunk<'_>,
        kind: LaneKind,
        file_param: i32,
        next_slot: &mut u32,
    ) -> Result<Clip, FlmError> {
        let (prefix, children) =
            chunk.container_expect(PrefixKind::U32Magic, &[MAGIC_CL02, MAGIC_CL01])?;
        let Prefix::U32Magic(start_tick, _) = prefix else {
            return Err(FlmError::new(&chunk.path, FlmErrorKind::BadValue));
        };
        self.charge(2 * size_of::<Clip>(), &chunk.path)?;
        let mut clip = Clip {
            id: 0,
            start_tick,
            visible_len_beats: 0.0,
            loop_len_beats: 0.0,
            offset_beats: 0.0,
            events: Vec::new(),
            muted: false,
            file: ClipFileState::default(),
        };
        for child in children {
            let child = child?;
            match &child.id {
                // A visible length ≤ 0 stays: it means the source or loop length (`Clip::visible_ticks`).
                b"CLHd" => {
                    let mut reader = child.reader();
                    clip.visible_len_beats = reader.f64_or(0.0);
                    clip.loop_len_beats = reader.f64_or(0.0);
                    clip.offset_beats = reader.f64_or(0.0);
                    clip.id = reader.u32_or(0);
                    clip.file.source_len_beats = reader.f64_or(0.0);
                    clip.muted = reader.u8_or(0) != 0;
                    clip.file.header_extra = reader.rest().to_vec();
                }
                b"ZOOM" => {
                    let mut reader = child.reader();
                    for value in &mut clip.file.zoom {
                        *value = reader.f64_or(*value);
                    }
                }
                b"EVN2" => {
                    let path = &child.path;
                    clip.events = decode_events(child.payload, kind, file_param, &mut self.budget)
                        .ok_or_else(|| FlmError::new(path, FlmErrorKind::TooLarge))?;
                    // Kept only when encoding the events would not give the bytes back. The writer's records are
                    // 20 bytes, so other sizes never do: no trial encode for them (it would cost more than the
                    // payload for small records). For 20-byte ones the trial is transient and costs about 1.6–1.8×
                    // the payload (the encoded bytes, a vector of event references and its sort scratch).
                    let canonical = child.payload.starts_with(&20_u16.to_le_bytes())
                        && super::write::encode_events(&clip.events, kind, file_param).as_deref()
                            == Some(child.payload);
                    if !canonical {
                        self.charge(child.payload.len(), path)?;
                        clip.file.raw_events = Raw(Some(child.payload.to_vec()));
                    }
                }
                // Legacy: no record size, 20-byte records. Written back as EVN2.
                b"EVNT" => {
                    let mut payload = 20_u16.to_le_bytes().to_vec();
                    payload.extend_from_slice(child.payload);
                    clip.events = decode_events(&payload, kind, file_param, &mut self.budget)
                        .ok_or_else(|| FlmError::new(&child.path, FlmErrorKind::TooLarge))?;
                }
                b"CLSm" => {
                    let (_, mut sample) = self.slot(&child)?;
                    *next_slot = (*next_slot + 1).min(MAX_ID);
                    sample.file.id = *next_slot;
                    clip.file.sample = Some(Box::new(sample));
                }
                _ => self.unknown(&child, &mut clip.file.unknown_chunks)?,
            }
        }
        Ok(clip)
    }
}

/// `HEAD`; returns @354, the next channel id, `None` when absent (legacy routing).
fn head(chunk: &Chunk<'_>, song: &mut Song) -> Result<Option<i32>, FlmError> {
    if chunk.payload.len() < 328 {
        return Err(FlmError::new(&chunk.path, FlmErrorKind::TooShort));
    }
    let mut reader = chunk.reader();
    song.file.selected_channel = reader.i32()?;
    song.file.last_send_bus = reader.i32()?;
    song.title = reader.char::<256>()?;
    // Any f64 on file (no number reads as 0): the engine gets one in the app's range.
    let (lo, hi) = crate::song::TEMPO_RANGE;
    song.tempo_bpm = reader.f64()?.clamp(lo, hi);
    // A reversed loop is swapped; start = end is none.
    let (start, end) = (reader.f64()?, reader.f64()?);
    let ticks = |beats: f64| {
        (beats * f64::from(PPQ))
            .round()
            .clamp(0.0, f64::from(u32::MAX)) as u32
    };
    let (start, end) = (ticks(start.min(end)), ticks(start.max(end)));
    song.play_loop = (start < end).then_some((start, end));
    song.file.playhead_beats = reader.f64()?;
    for view in &mut song.file.playlist_view {
        *view = reader.f64()?;
    }
    if let Ok(guid) = reader.take(16) {
        song.file.guid.copy_from_slice(guid);
    }
    song.key_root = reader.i32_or(0);
    song.key_scale = reader.i32_or(0);
    song.file.show_sharps = reader.u8_or(1) != 0;
    song.file.play_truncated_notes = reader.u8_or(0) != 0;
    if reader.remaining() < 4 {
        return Ok(None);
    }
    let next = reader.u32()?;
    if next >= 4096 {
        return Err(FlmError::new(&chunk.path, FlmErrorKind::BadValue));
    }
    let mut version = reader.clone();
    if let Ok(bytes) = version.vstr32() {
        song.file.version = text(bytes);
        reader = version;
    }
    song.file.head_extra = reader.rest().to_vec();
    Ok(Some(next as i32))
}

/// `META`: fields that are cut short take their loader defaults.
fn meta(bytes: &[u8]) -> MetaState {
    let field = |offset: usize, size: usize| {
        let field = bytes.get(offset..)?;
        let field = &field[..field.len().min(size)];
        Some(text(
            &field[..field.iter().position(|&b| b == 0).unwrap_or(field.len())],
        ))
    };
    let mut meta = MetaState::default();
    meta.created = field(0, 256).unwrap_or(meta.created);
    meta.editing_seconds = Cursor::new(bytes.get(256..).unwrap_or_default(), "").f64_or(0.0);
    meta.author = field(264, 256).unwrap_or_default();
    meta.display_title = field(520, 256).unwrap_or_default();
    meta.locked = bytes.get(776).is_some_and(|&locked| locked != 0);
    meta.info_url = field(777, 2048).unwrap_or_default();
    meta.modified = field(2825, 256).unwrap_or(meta.modified);
    meta.extra = bytes.get(3081..).unwrap_or_default().to_vec();
    meta
}

/// `KEYB`.
fn keyboard(chunk: &Chunk<'_>) -> Result<KeyboardState, FlmError> {
    let mut reader = chunk.reader();
    let mut keyboard = KeyboardState {
        chord_json: text(reader.vstr32()?),
        vertical_mode: reader.u32_or(0),
        set_state: reader.u32_or(0),
        ..KeyboardState::default()
    };
    if reader.remaining() >= 4 {
        keyboard.chord_set_name = text(reader.vstr32()?);
    }
    keyboard.octave_shift = reader.i32_or(0);
    keyboard.velocity = reader.f32_or(0.5);
    keyboard.timing = reader.f32_or(0.0);
    keyboard.hold = reader.u8_or(0) != 0;
    keyboard.extra = reader.rest().to_vec();
    Ok(keyboard)
}

/// `CHHD` into `channel`; returns the playlist row and the base name.
fn header(chunk: &Chunk<'_>, channel: &mut Channel) -> Result<(f64, String), FlmError> {
    if chunk.payload.len() < 1040 {
        return Err(FlmError::new(&chunk.path, FlmErrorKind::TooShort));
    }
    let mut reader = chunk.reader();
    let display = reader.char::<1024>()?;
    channel.zipped = reader.u32()? != 0;
    let row = reader.f64()?;
    channel.hue = reader.f32()?.clamp(0.0, 1.0);
    channel.file.view = [reader.f64_or(0.0), reader.f64_or(0.0)];
    channel.file.input_monitoring = reader.u32_or(0) != 0;
    channel.file.record_armed = reader.u32_or(0) != 0;
    if let Ok(guid) = reader.take(16) {
        channel.file.guid.copy_from_slice(guid);
    }
    channel.id = reader.i32_or(-1);
    let mut names = [const { String::new() }; 3];
    for name in &mut names {
        let mut next = reader.clone();
        let Ok(bytes) = next.vstr32() else {
            break;
        };
        *name = text(bytes);
        reader = next;
    }
    let [base_name, preset_name, custom_name] = names;
    channel.file.names = Raw(Some((base_name.clone(), custom_name.clone())));
    // The display name counts only when all three are empty: it is then the custom name.
    channel.name = [&custom_name, &preset_name, &base_name, &display]
        .into_iter()
        .find(|name| !name.is_empty())
        .cloned()
        .unwrap_or_default();
    channel.file.preset_name = preset_name;
    channel.file.header_extra = reader.rest().to_vec();
    Ok((row, base_name))
}

/// `SEND`; missing fields take the defaults.
fn send(chunk: &Chunk<'_>) -> SendState {
    let mut reader = chunk.reader();
    let default = SendState::default();
    SendState {
        target_channel_id: reader.i32_or(default.target_channel_id),
        enabled: reader.f32_or(default.enabled),
        linear_gain: reader.f32_or(default.linear_gain),
        enabled_init: reader.f32_or(default.enabled_init),
        level_init: reader.f32_or(default.level_init),
        level: reader.f32_or(default.level),
        extra: reader.rest().to_vec(),
    }
}

/// ADD1: an inverted range is swapped, then a negative low clamped to 0 (and a high below it raised to it, so a
/// range read once reads the same again).
fn key_range(low: i8, high: i8) -> (i8, i8) {
    let (low, high) = if low > high { (high, low) } else { (low, high) };
    let low = low.max(0);
    (low, high.max(low))
}

/// Reverb v2 (32) with a legacy Reverb (12) HEAD: the legacy params by name.
fn legacy_reverb_params(params: &mut [f32], legacy: &[f32]) {
    let (Some(new), Some(old)) = (params::module_spec(32), params::module_spec(12)) else {
        return;
    };
    for (spec, value) in old.params.iter().zip(legacy) {
        let to = (new.params.iter()).position(|p| !p.name.is_empty() && p.name == spec.name);
        if let Some(param) = to.and_then(|i| params.get_mut(i)) {
            *param = *value;
        }
    }
}

/// A sample path a file can hold: a shorthand into the content or user root.
fn writable(path: &ContentPath) -> bool {
    matches!(
        path,
        ContentPath::Shorthand {
            root: ContentRoot::App | ContentRoot::User,
            ..
        }
    )
}

/// The channel's lane order, kind, name and kit, once its rack and CHNL are read.
fn settle_channel(rack: &mut RackRead) {
    let channel = &mut rack.channel;
    // Lanes that are not automation first, in file order; then automation in file order.
    channel
        .lanes
        .sort_by_key(|lane| lane.target.map_or(i32::MIN, AutomationTarget::file_order));
    channel.kind = channel_kind(rack.kind, &rack.base_name, channel);
    if channel.name.is_empty() {
        channel.name = channel.kind.label().to_owned();
    }
    match channel.kind {
        ChannelKind::Drums => {
            channel.drum_kit.get_or_insert_with(DrumKit::default);
        }
        // Every channel whose lane 0 is not audio has an RSMP: only a non-default one is kept.
        _ if channel.drum_kit == Some(DrumKit::default()) => channel.drum_kit = None,
        _ => {}
    }
    if channel.kind != ChannelKind::EffectTrack {
        channel.bus_id = -1;
    }
    rekey_hits(channel, &rack.pads);
}

/// Drum hits are keyed by `MAIN@4` pad index and the model's by slot position: with the kit's pads not
/// 0, 1, 2 … (`pads`, in slot order; empty when they are), each hit takes its slot's position. A hit on a pad without
/// a slot plays nothing in the app: it stays past the slots, or goes to pad 255 (past the slots of any kit of under
/// 256) when its key is now a slot's position.
fn rekey_hits(channel: &mut Channel, pads: &[f64]) {
    if pads.is_empty() {
        return;
    }
    let position = |key: u8| pads.iter().position(|&pad| pad == f64::from(key));
    let lanes = channel
        .lanes
        .iter_mut()
        .filter(|l| l.kind == LaneKind::Drums);
    for event in lanes.flat_map(|l| &mut l.clips).flat_map(|c| &mut c.events) {
        if let Event::Drum { pad, .. } = event {
            *pad = match position(*pad) {
                Some(slot) => u8::try_from(slot).unwrap_or(u8::MAX),
                None if usize::from(*pad) < pads.len() => u8::MAX,
                None => *pad,
            };
        }
    }
}

/// The kind of a channel of file kind `file_kind`: our base names ("<kind label> <id>") name it; else the main lane
/// and the generator module tell.
fn channel_kind(file_kind: u32, base_name: &str, channel: &Channel) -> ChannelKind {
    match file_kind {
        0 => return ChannelKind::Master,
        0x80 => return ChannelKind::EffectTrack,
        _ => {}
    }
    let main = channel.lanes[0].kind;
    let fits = |kind: &ChannelKind| match kind {
        ChannelKind::Drums => main == LaneKind::Drums,
        ChannelKind::AudioClip | ChannelKind::AudioRecord => main == LaneKind::Audio,
        ChannelKind::Instrument | ChannelKind::Rack | ChannelKind::Synth(_) => {
            main == LaneKind::Notes
        }
        ChannelKind::Master | ChannelKind::EffectTrack => false,
    };
    let label = match base_name.rsplit_once(' ') {
        Some((label, id)) if !id.is_empty() && id.bytes().all(|b| b.is_ascii_digit()) => label,
        _ => base_name,
    };
    let mut kinds =
        (ChannelKind::ADDABLE.into_iter()).chain(SynthKind::ALL.map(ChannelKind::Synth));
    if let Some(kind) = kinds.find(|kind| fits(kind) && kind.label() == label) {
        return kind;
    }
    match main {
        LaneKind::Drums => ChannelKind::Drums,
        LaneKind::Audio => ChannelKind::AudioClip,
        _ => {
            let generator = channel.rack.modules.first().map(|module| module.type_id);
            (SynthKind::ALL.into_iter())
                .find(|synth| Some(synth.type_id()) == generator)
                .map_or(ChannelKind::Instrument, ChannelKind::Synth)
        }
    }
}

/// Channel ids: the master 0; legacy files 0..N−1 in channel order; otherwise the file's, with
/// missing, out-of-range and repeated ones given the next free ids from the song's counter (@354).
fn assign_channel_ids(racks: &mut [RackRead], next_channel_id: Option<i32>) {
    let Some(next) = next_channel_id else {
        for (id, rack) in racks.iter_mut().enumerate() {
            rack.channel.id = id as i32;
        }
        return;
    };
    let mut used = HashSet::from([0]);
    let mut fresh = Vec::new();
    for (index, rack) in racks.iter_mut().enumerate() {
        let id = rack.channel.id;
        if index == 0 {
            rack.channel.id = 0;
        } else if !((1..=CHANNEL_ID_LIMIT).contains(&id) && used.insert(id)) {
            fresh.push(index);
        }
    }
    let start = next.clamp(1, CHANNEL_ID_LIMIT);
    let mut free = (start..=CHANNEL_ID_LIMIT)
        .chain(1..start)
        .filter(|id| !used.contains(id));
    for index in fresh {
        racks[index].channel.id = free.next().expect("at most 4096 channels: an id is free");
    }
}

/// Legacy routing (HEAD without @354): each non-master channel sends to the send channel whose bus is its
/// `RHED[12]`, else to the master; each drum slot to the send channel whose bus is its LINk@4, else its channel.
fn legacy_routing(channels: &mut [Channel]) {
    let buses: Vec<(i32, i32)> = (channels.iter())
        .filter(|c| c.kind == ChannelKind::EffectTrack && c.bus_id >= 1)
        .map(|c| (c.bus_id, c.id))
        .collect();
    let send_to = |bus: i32| buses.iter().find(|(b, _)| *b == bus).map(|&(_, id)| id);
    for channel in channels.iter_mut().skip(1) {
        let target = send_to(channel.output_bus).unwrap_or(0);
        channel.file.sends = vec![SendState {
            target_channel_id: target,
            ..SendState::default()
        }];
        for slot in channel.drum_kit.iter_mut().flat_map(|kit| &mut kit.slots) {
            slot.file.send_target = send_to(slot.file.output_bus).unwrap_or(-1);
        }
    }
}

/// Send buses unique and ≥ 1 with HEAD@4 above them; a SEND on every non-master channel (none on the master);
/// each output bus derived from the SENDs (`RHED[12]`).
fn settle_routing(channels: &mut [Channel], last_send_bus: &mut i32) {
    let valid = |bus: i32| (1..=MAX_ID as i32).contains(&bus);
    let sends = channels
        .iter()
        .filter(|c| c.kind == ChannelKind::EffectTrack);
    let max = sends
        .map(|c| c.bus_id)
        .filter(|&bus| valid(bus))
        .max()
        .unwrap_or(0);
    *last_send_bus = (*last_send_bus).clamp(0, MAX_ID as i32).max(max);
    let mut used = HashSet::new();
    for channel in channels
        .iter_mut()
        .filter(|c| c.kind == ChannelKind::EffectTrack)
    {
        if !(valid(channel.bus_id) && used.insert(channel.bus_id)) {
            *last_send_bus += 1;
            channel.bus_id = *last_send_bus;
        }
    }
    channels[0].file.sends.clear();
    for channel in channels.iter_mut().skip(1) {
        if channel.file.sends.is_empty() {
            channel.file.sends.push(SendState::default());
        }
    }
    let buses: Vec<(i32, i32)> = (channels.iter())
        .filter(|c| c.kind == ChannelKind::EffectTrack)
        .map(|c| (c.id, c.bus_id))
        .collect();
    for channel in channels.iter_mut() {
        channel.output_bus = crate::song::output_bus(&channel.file.sends, &buses);
    }
}
