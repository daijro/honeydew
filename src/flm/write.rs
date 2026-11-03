//! The project writer: [`Song`] → `.flm` bytes in the newest layout. Deterministic: the save-time stamp (dates,
//! version, GUIDs) is [`Song::stamp_save`]'s job.
//!
//! Derived on write: the playlist rows (channel and track order), the base and custom names, the output buses (from the
//! `SEND`s), the pad indexes (slot positions), and every counter raised to cover its ids. Bytes kept by the reader
//! (`EVN2`, `STRC`, `STR1`) go back while they still match the model.

use std::borrow::Cow;
use std::collections::HashSet;

use honeydew_audio::dwp::Cstm;

use super::chunk::{
    ChunkWriter, ContentPath, ContentRoot, FlmError, FlmErrorKind, MAGIC_CH02, MAGIC_CL02,
    MAGIC_RK01, MAGIC_SL02, MAGIC_SM01, MAGIC_TMPL,
};
use super::read::{check_cstm, decode_events};
use crate::song::{
    AutomationTarget, CHANNEL_ID_LIMIT, Channel, ChannelKind, Clip, DrumKit, Event, Lane, LaneKind,
    ModuleInstance, PPQ, RawChunk, SamplerSlot, SendState, Song, TEMPO_TARGET, output_bus,
};

/// Writes `song`. Fails only on a song no reader result can be: a first channel that is not the master with id 0,
/// channel ids outside 0..=4095 or repeated, a channel without lanes, a send without its own bus ≥ 1, module ids
/// below 2 or repeated, an unknown module type, a `CSTM` its module may not carry, events of another lane kind, or
/// sizes past the chunk limits.
pub fn write(song: &Song) -> Result<Vec<u8>, FlmError> {
    check(song)?;
    // Racks in kind order (master, sends, the rest), the mixer order.
    let channels: Vec<(usize, &Channel)> = (song.rack_order().into_iter())
        .map(|i| (i, &song.channels[i]))
        .collect();
    let buses = song.send_buses();
    let mut writer = ChunkWriter::new_file();
    writer.chunk(*b"HEAD", |writer| write_head(writer, song))?;
    writer.chunk(*b"KEYB", |writer| write_keyboard(writer, song))?;
    writer.chunk(*b"META", |writer| write_meta(writer, song))?;
    writer.chunk(*b"TDIV", |writer| {
        writer.u8(song.time_sig.subdivision)?;
        writer.u8(song.time_sig.beats_per_bar)?;
        writer.raw(&song.file.time_sig_extra)
    })?;
    for (_, channel) in &channels {
        writer.chunk(*b"RACK", |writer| write_rack_chunk(writer, channel, &buses))?;
    }
    // CHHD @1028: a channel's playlist row is its first track's. Every track is a row, and a zipped channel
    // shows one.
    let mut rows = vec![0; song.channels.len()];
    let mut row = 0;
    for c in song.playlist() {
        rows[c] = row;
        let channel = &song.channels[c];
        row += if channel.zipped {
            1
        } else {
            channel.lanes.len()
        };
    }
    for (index, channel) in &channels {
        writer.chunk(*b"CHNL", |writer| {
            write_channel(writer, channel, rows[*index])
        })?;
    }
    write_raw(&mut writer, &song.file.unknown_chunks)?;
    Ok(writer.into_bytes())
}

/// The id invariants every song [`write`] takes must hold.
fn check(song: &Song) -> Result<(), FlmError> {
    let bad = |path: &str| Err(FlmError::new(path, FlmErrorKind::BadValue));
    let master = song.channels.first();
    if master.is_none_or(|master| master.kind != ChannelKind::Master || master.id != 0) {
        return bad("RACK[0]");
    }
    let (mut ids, mut buses) = (HashSet::new(), HashSet::new());
    for (index, channel) in song.channels.iter().enumerate() {
        let send = channel.kind == ChannelKind::EffectTrack;
        if (index > 0 && channel.kind == ChannelKind::Master)
            || !(0..=CHANNEL_ID_LIMIT).contains(&channel.id)
            || !ids.insert(channel.id)
            || channel.lanes.is_empty()
            || (send && (channel.bus_id < 1 || !buses.insert(channel.bus_id)))
        {
            return bad(&format!("CHNL[{index}]"));
        }
        let mut modules = HashSet::new();
        let modules_ok = (channel.rack.modules.iter())
            .all(|module| module.instance_id >= 2 && modules.insert(module.instance_id));
        if !modules_ok {
            return bad(&format!("RACK[{index}]/RMOd"));
        }
    }
    Ok(())
}

fn write_raw(writer: &mut ChunkWriter, chunks: &[RawChunk]) -> Result<(), FlmError> {
    for chunk in chunks {
        writer.chunk(chunk.id, |writer| writer.raw(&chunk.payload))?;
    }
    Ok(())
}

fn write_head(writer: &mut ChunkWriter, song: &Song) -> Result<(), FlmError> {
    let file = &song.file;
    writer.i32(file.selected_channel)?;
    // @4 covers every send bus.
    let max_bus = song.channels.iter().map(|c| c.bus_id).max().unwrap_or(0);
    writer.i32(file.last_send_bus.max(max_bus))?;
    writer.char::<256>(&song.title)?;
    writer.f64(song.tempo_bpm)?;
    let (start, end) = song.play_loop.unwrap_or_default();
    writer.f64(f64::from(start) / f64::from(PPQ))?;
    writer.f64(f64::from(end) / f64::from(PPQ))?;
    writer.f64(file.playhead_beats)?;
    for value in file.playlist_view {
        writer.f64(value)?;
    }
    writer.raw(&file.guid)?;
    writer.i32(song.key_root)?;
    writer.i32(song.key_scale)?;
    writer.u8(file.show_sharps.into())?;
    writer.u8(file.play_truncated_notes.into())?;
    // @354: above every id, below 4096.
    let max_id = song.channels.iter().map(|c| c.id).max().unwrap_or(0);
    writer.i32(file.next_channel_id.max(max_id + 1).min(CHANNEL_ID_LIMIT))?;
    writer.vstr32(file.version.as_bytes())?;
    writer.raw(&file.head_extra)
}

fn write_keyboard(writer: &mut ChunkWriter, song: &Song) -> Result<(), FlmError> {
    let keyboard = &song.file.keyboard;
    writer.vstr32(keyboard.chord_json.as_bytes())?;
    writer.u32(keyboard.vertical_mode)?;
    writer.u32(keyboard.set_state)?;
    writer.vstr32(keyboard.chord_set_name.as_bytes())?;
    writer.i32(keyboard.octave_shift)?;
    writer.f32(keyboard.velocity)?;
    writer.f32(keyboard.timing)?;
    writer.u8(keyboard.hold.into())?;
    writer.raw(&keyboard.extra)
}

fn write_meta(writer: &mut ChunkWriter, song: &Song) -> Result<(), FlmError> {
    let meta = &song.file.meta;
    writer.char::<256>(&meta.created)?;
    writer.f64(meta.editing_seconds)?;
    writer.char::<256>(&meta.author)?;
    writer.char::<256>(&meta.display_title)?;
    writer.u8(meta.locked.into())?;
    writer.char::<2048>(&meta.info_url)?;
    writer.char::<256>(&meta.modified)?;
    writer.raw(&meta.extra)
}

/// `RACK`; `buses` are the send channels' (id, bus) for the output bus.
fn write_rack_chunk(
    writer: &mut ChunkWriter,
    channel: &Channel,
    buses: &[(i32, i32)],
) -> Result<(), FlmError> {
    writer.u32(channel.kind.file_kind())?;
    writer.raw(&MAGIC_RK01)?;
    writer.chunk(*b"RHED", |writer| {
        writer.i32(-1)?;
        let max_id = channel.rack.modules.iter().map(|m| m.instance_id).max();
        writer.i32(channel.rack.last_instance_id.max(max_id.unwrap_or(1)))?;
        // A send's bus, 0 on every other channel; the output bus from the SENDs.
        let send = channel.kind == ChannelKind::EffectTrack;
        writer.i32(if send { channel.bus_id } else { 0 })?;
        writer.i32(output_bus(&channel.file.sends, buses))?;
        writer.raw(&channel.file.rack_header_extra)
    })?;
    writer.chunk(*b"RPRM", |writer| {
        for value in channel.params.0 {
            writer.f32(value)?;
        }
        writer.raw(&channel.file.params_extra)
    })?;
    if !channel.file.cc_links.is_empty() {
        writer.chunk(*b"CCLK", |writer| writer.raw(&channel.file.cc_links))?;
    }
    // An RSMP on every channel whose lane 0 is not audio.
    let empty = DrumKit::default();
    let lane_0_audio = channel
        .lanes
        .first()
        .is_some_and(|l| l.kind == LaneKind::Audio);
    if let Some(kit) = channel
        .drum_kit
        .as_ref()
        .or((!lane_0_audio).then_some(&empty))
    {
        writer.chunk(*b"RSMP", |writer| {
            writer.raw(&MAGIC_SM01)?;
            let max_id = kit.slots.iter().map(|slot| slot.file.id).max();
            writer.u32(kit.file.last_slot_id.max(max_id.unwrap_or(0)))?;
            write_kit(writer, kit)
        })?;
    }
    write_modules(writer, &channel.rack.modules)?;
    write_raw(writer, &channel.file.rack_unknown_chunks)
}

/// A rack's modules in chain order: an `RMOd` each, then each of Honeydew's own modules (a plugin module,
/// [`crate::song::plugin`], or one of Lemon Studio's effects) as an `HDPm`, which a reader that does not know it skips as
/// an unknown chunk, while an `RMOd` of a type it does not know would fail the whole song. An `HDPm` is `u32` its
/// position in the chain, `u32` 1 (the layout), then its module's `RMOd`, which carries a plugin's `PLUG` and `PSTA`.
fn write_modules(writer: &mut ChunkWriter, modules: &[ModuleInstance]) -> Result<(), FlmError> {
    let plugin = |module: &ModuleInstance| crate::song::plugin::is_own(module.type_id);
    for module in modules.iter().filter(|m| !plugin(m)) {
        writer.chunk(*b"RMOd", |writer| write_module(writer, module))?;
    }
    for (position, module) in modules.iter().enumerate().filter(|(_, m)| plugin(m)) {
        writer.chunk(*b"HDPm", |writer| {
            writer.u32(position as u32)?;
            writer.u32(1)?;
            writer.chunk(*b"RMOd", |writer| write_module(writer, module))
        })?;
    }
    Ok(())
}

/// A kit's `PADS`, its slots and its unknown chunks: what follows the `RSMP` prefix, or the `.tmpl` magic.
fn write_kit(writer: &mut ChunkWriter, kit: &DrumKit) -> Result<(), FlmError> {
    writer.chunk(*b"PADS", |writer| writer.raw(&kit.file.pads))?;
    // Each slot on its pad (its position), in slot-id order.
    let mut slots: Vec<(usize, &SamplerSlot)> = kit.slots.iter().enumerate().collect();
    slots.sort_by_key(|(_, slot)| slot.file.id);
    for (pad, slot) in slots {
        writer.chunk(*b"SMPl", |writer| write_slot(writer, slot, pad as f64))?;
    }
    write_raw(writer, &kit.file.unknown_chunks)
}

/// A drum kit template, `.tmpl`: `"LPMT"`, then the kit as an `RSMP` holds it, without its counter.
pub fn write_drumset(kit: &DrumKit) -> Result<Vec<u8>, FlmError> {
    let mut writer = ChunkWriter::default();
    writer.raw(&MAGIC_TMPL)?;
    write_kit(&mut writer, kit)?;
    Ok(writer.into_bytes())
}

/// A module preset, `.flmpst`: `module`'s `RMOd` body, a bare chunk list. Fails as [`write`] does on the module.
pub fn write_preset(module: &ModuleInstance) -> Result<Vec<u8>, FlmError> {
    let mut writer = ChunkWriter::default();
    write_module_body(&mut writer, module)?;
    Ok(writer.into_bytes())
}

/// A rack preset, `.flmrack`: an `RMOd` per module, in chain order, with its type and instance id (a plugin module an
/// `HDPm`, as in a song).
pub fn write_rack(modules: &[ModuleInstance]) -> Result<Vec<u8>, FlmError> {
    let mut writer = ChunkWriter::default();
    write_modules(&mut writer, modules)?;
    Ok(writer.into_bytes())
}

/// An `RMOd` payload: the type and instance id, then the body, then Honeydew's own sidechain
/// ([`ModuleInstance::sidechain`]) if it has one: an `HDSc`, `i32` the source channel's id, which FL Studio Mobile skips as
/// an unknown chunk. A module preset (the body alone) keeps none.
fn write_module(writer: &mut ChunkWriter, module: &ModuleInstance) -> Result<(), FlmError> {
    writer.u32(u32::from(module.type_id))?;
    writer.u32(module.instance_id as u32)?;
    write_module_body(writer, module)?;
    if let Some(source) = module.sidechain {
        writer.chunk(*b"HDSc", |writer| writer.i32(source))?;
    }
    Ok(())
}

/// The module chunks: `HEAD DESC PRST ADD1 PRMS SMPR [CSTM]`, then the unknown ones kept.
fn write_module_body(writer: &mut ChunkWriter, module: &ModuleInstance) -> Result<(), FlmError> {
    if crate::params::module_spec(module.type_id).is_none() {
        return Err(FlmError::new("RMOd", FlmErrorKind::BadValue));
    }
    let custom_data = saved_cstm(module);
    if let Some(data) = &custom_data {
        check_cstm(module.type_id, data, "RMOd/CSTM")?;
    }
    writer.chunk(*b"HEAD", |writer| {
        writer.u32(u32::from(module.type_id))?;
        writer.u32(module.file.head_reserved)?;
        writer.u8(module.file.head_enabled)?;
        writer.raw(&module.file.head_extra)
    })?;
    writer.chunk(*b"DESC", |writer| {
        writer.char::<256>(&module.file.category.replace('\\', "/"))?;
        writer.char::<256>(&module.file.preset_name)?;
        if module.file.preset_path.is_empty() {
            writer.u32(0)?;
        } else {
            writer.vstr16(module.file.preset_path.as_bytes())?;
        }
        writer.raw(&module.file.desc_extra)
    })?;
    writer.chunk(*b"PRST", |writer| {
        writer.vstr32(module.file.source_path.as_bytes())?;
        writer.vstr32(module.file.provenance.as_bytes())
    })?;
    writer.chunk(*b"ADD1", |writer| {
        writer.u8(module.minimized.into())?;
        writer.u8(module.file.key_low as u8)?;
        writer.u8(module.file.key_high as u8)?;
        writer.raw(&module.file.add1_extra)
    })?;
    writer.chunk(*b"PRMS", |writer| {
        for value in &module.params {
            writer.f32(*value)?;
        }
        Ok(())
    })?;
    writer.chunk(*b"SMPR", |writer| {
        for value in module.common.0 {
            writer.f32(value)?;
        }
        Ok(())
    })?;
    if let Some(data) = &custom_data {
        writer.chunk(*b"CSTM", |writer| writer.raw(data))?;
    }
    if let Some(plugin) =
        (module.file.plugin.as_ref()).filter(|_| crate::song::plugin::is_plugin(module.type_id))
    {
        write_plugin(writer, plugin)?;
    }
    write_raw(writer, &module.file.unknown_chunks)
}

/// A plugin module's plugin, `PLUG`: `vstr16` format, id, name and vendor, `vstr32` path, `u32` n, then per param `u32`
/// id, `f32` default and `vstr16` name; and its state, `PSTA`, the plugin's own bytes.
fn write_plugin(
    writer: &mut ChunkWriter,
    plugin: &crate::song::plugin::PluginRef,
) -> Result<(), FlmError> {
    writer.chunk(*b"PLUG", |writer| {
        for text in [&plugin.format, &plugin.id, &plugin.name, &plugin.vendor] {
            writer.vstr16(text.as_bytes())?;
        }
        writer.vstr32(plugin.path.as_bytes())?;
        writer.u32(plugin.params.len() as u32)?;
        for param in &plugin.params {
            writer.u32(param.id)?;
            writer.f32(param.default)?;
            writer.vstr16(param.name.as_bytes())?;
        }
        Ok(())
    })?;
    writer.chunk(*b"PSTA", |writer| writer.raw(&plugin.state))
}

/// The `CSTM` `module` saves: its blob, except that a DW Sampler's `PATh` floats (of the `PATh` that counts,
/// [`Cstm::read`]) are its Attack and Release init values, not its current ones ([`ModuleInstance::program_init`]).
/// A program chunk without both floats (a legacy `PATH`, an old `PATh`) is saved as it is: until its program loads,
/// its init values are the program's, unknown.
fn saved_cstm(module: &ModuleInstance) -> Option<Cow<'_, [u8]>> {
    let blob = module.file.custom_data.as_deref()?;
    let program = (module.type_id == 1)
        .then(|| Cstm::read(blob).program)
        .flatten();
    let Some(program) = program.filter(|p| p.init.is_some()) else {
        return Some(Cow::Borrowed(blob));
    };
    let at = program.chunk.start + 12 + program.path.len();
    let mut saved = blob.to_vec();
    let floats = module.program_init().map(f32::to_le_bytes).concat();
    saved[at..at + 8].copy_from_slice(&floats);
    Some(Cow::Owned(saved))
}

/// `CHNL`; `row` is the channel's playlist row.
fn write_channel(writer: &mut ChunkWriter, channel: &Channel, row: usize) -> Result<(), FlmError> {
    writer.u32(channel.kind.file_kind())?;
    writer.raw(&MAGIC_CH02)?;
    writer.chunk(*b"CHHD", |writer| {
        writer.char::<1024>(&channel.name)?;
        writer.u32(channel.zipped.into())?;
        writer.f64(row as f64)?;
        writer.f32(channel.hue)?;
        for value in channel.file.view {
            writer.f64(value)?;
        }
        writer.u32(channel.file.input_monitoring.into())?;
        writer.u32(channel.file.record_armed.into())?;
        writer.raw(&channel.file.guid)?;
        writer.i32(channel.id)?;
        // The base name is the one read, else "MASTER" or "<Kind> <id>". The custom name is set only when
        // the user renamed the channel: when its name is not the auto one (the preset name, else the base name);
        // the one read stays while it is still the name.
        let read = channel.file.names.0.as_ref();
        let base = match (read, channel.kind) {
            (Some((base, _)), _) => base.clone(),
            (None, ChannelKind::Master) => ChannelKind::Master.label().to_owned(),
            (None, kind) => format!("{} {}", kind.base_label(), channel.id),
        };
        let preset = &channel.file.preset_name;
        let auto = if preset.is_empty() { &base } else { preset };
        let custom = match read {
            Some((_, custom)) if *custom == channel.name => custom,
            _ if *auto == channel.name => "",
            _ => &channel.name,
        };
        writer.vstr32(base.as_bytes())?;
        writer.vstr32(preset.as_bytes())?;
        writer.vstr32(custom.as_bytes())?;
        writer.raw(&channel.file.header_extra)
    })?;
    // A SEND on every non-master channel, never on the master. A channel whose sends were all removed
    // (`Disconnected from MASTER`) writes the default one switched off, so it stays silent after a load.
    if channel.kind != ChannelKind::Master {
        let default = [SendState {
            enabled: 0.0,
            ..SendState::default()
        }];
        let sends = if channel.file.sends.is_empty() {
            &default[..]
        } else {
            &channel.file.sends
        };
        for send in sends {
            writer.chunk(*b"SEND", |writer| write_send(writer, send))?;
        }
    }
    // Automation lanes after the others, in file order. The master's tempo lane (Honeydew's own) is an `HDTk`, a
    // `TRKH` by another name, which a reader that does not know it skips as an unknown chunk.
    let mut lanes: Vec<&Lane> = channel.lanes.iter().collect();
    lanes.sort_by_key(|lane| lane.target.map_or(i32::MIN, AutomationTarget::file_order));
    for lane in lanes {
        let tempo = channel.kind == ChannelKind::Master && lane.target == Some(TEMPO_TARGET);
        let id = if tempo { *b"HDTk" } else { *b"TRKH" };
        writer.chunk(id, |writer| write_lane(writer, lane))?;
    }
    // Honeydew's own MIDI input, only when set (`flm::read` reads it back; FL Studio Mobile skips it): `vstr32` the
    // device ("" for All) and `u32` the MIDI channel 1…16 (0 for All).
    let input = &channel.file.midi_input;
    if input.device.is_some() || input.channel.is_some() {
        writer.chunk(*b"HDMi", |writer| {
            writer.vstr32(input.device.as_deref().unwrap_or_default().as_bytes())?;
            writer.u32(input.channel.map_or(0, |c| u32::from(c) + 1))
        })?;
    }
    // Honeydew's own MIDI sends, only when there are any: `i32` the target channel id and `u32` on (1) or off (0), each.
    let sends = &channel.file.midi_sends;
    if !sends.is_empty() {
        writer.chunk(*b"HDMs", |writer| {
            for send in sends {
                writer.i32(send.target_channel_id)?;
                writer.u32(send.enabled.into())?;
            }
            Ok(())
        })?;
    }
    // Honeydew's own Modulator targets, those of its rack's modules, only when there are any: `i32` the Modulator's
    // instance id, the target's channel id, instance id and param, each.
    let modulations = (channel.file.modulations.iter())
        .filter(|m| channel.rack.get(m.modulator).is_some())
        .collect::<Vec<_>>();
    if !modulations.is_empty() {
        writer.chunk(*b"HDMd", |writer| {
            for m in modulations {
                for v in [m.modulator, m.channel, m.target.instance_id, m.target.param] {
                    writer.i32(v)?;
                }
            }
            Ok(())
        })?;
    }
    write_raw(writer, &channel.file.channel_unknown_chunks)
}

fn write_send(writer: &mut ChunkWriter, send: &SendState) -> Result<(), FlmError> {
    writer.i32(send.target_channel_id)?;
    writer.f32(send.enabled)?;
    writer.f32(send.linear_gain)?;
    writer.f32(send.enabled_init)?;
    writer.f32(send.level_init)?;
    writer.f32(send.level)?;
    writer.raw(&send.extra)
}

/// The `DESc` target of a lane: (instance, file param) for automation, else −1 and −1 (audio: 0).
fn desc_target(lane: &Lane) -> (i32, i32) {
    match lane.target {
        Some(target) if lane.kind == LaneKind::Automation => (
            target.instance_id,
            AutomationTarget::file_param(target.instance_id, target.param),
        ),
        _ => (-1, if lane.kind == LaneKind::Audio { 0 } else { -1 }),
    }
}

fn write_lane(writer: &mut ChunkWriter, lane: &Lane) -> Result<(), FlmError> {
    let (instance, param) = desc_target(lane);
    writer.chunk(*b"DESc", |writer| {
        writer.u8(lane.kind as u8)?;
        writer.u8(match lane.kind {
            LaneKind::Drums => lane.grid as u8,
            _ => lane.file.flag,
        })?;
        writer.u16(0)?;
        writer.i32(instance)?;
        writer.i32(param)?;
        writer.f64(lane.swing)?;
        let max_id = lane.clips.iter().map(|clip| clip.id).max();
        writer.u32(lane.last_clip_id.max(max_id.unwrap_or(0)))?;
        writer.u32(lane.file.current_clip_id)?;
        writer.char::<1024>(&lane.label)?;
        writer.vstr32(lane.file.custom_label.as_bytes())?;
        writer.raw(&lane.file.desc_extra)
    })?;
    let mut clips: Vec<&Clip> = lane.clips.iter().collect();
    clips.sort_by_key(|clip| clip.start_tick);
    for clip in clips {
        writer.chunk(*b"CLIP", |writer| {
            write_clip(writer, clip, lane.kind, param)
        })?;
    }
    // A hidden track (Honeydew's own) holds an empty `HDHd`, which a reader that does not know it skips.
    if lane.hidden {
        writer.chunk(*b"HDHd", |_| Ok(()))?;
    }
    write_raw(writer, &lane.file.unknown_chunks)
}

fn write_clip(
    writer: &mut ChunkWriter,
    clip: &Clip,
    kind: LaneKind,
    param: i32,
) -> Result<(), FlmError> {
    writer.u32(clip.start_tick)?;
    writer.raw(&MAGIC_CL02)?;
    writer.chunk(*b"CLHd", |writer| {
        writer.f64(clip.visible_len_beats)?;
        writer.f64(clip.loop_len_beats)?;
        writer.f64(clip.offset_beats)?;
        writer.u32(clip.id)?;
        writer.f64(clip.file.source_len_beats)?;
        writer.u8(clip.muted.into())?;
        writer.raw(&clip.file.header_extra)
    })?;
    writer.chunk(*b"ZOOM", |writer| {
        for value in clip.file.zoom {
            writer.f64(value)?;
        }
        Ok(())
    })?;
    if let Some(slot) = &clip.file.sample {
        writer.chunk(*b"CLSm", |writer| write_slot(writer, slot, -1.0))?;
    }
    writer.chunk(*b"EVN2", |writer| {
        // Kept bytes go back while they still decode to the events (and, on automation, still name the param).
        if let Some(raw) = &clip.file.raw_events.0
            && decode_events(raw, kind, param, &mut { usize::MAX }).as_deref()
                == Some(&clip.events[..])
            && (kind != LaneKind::Automation || keys_name(raw, param))
        {
            return writer.raw(raw);
        }
        let bytes = encode_events(&clip.events, kind, param)
            .ok_or_else(|| FlmError::new("EVN2", FlmErrorKind::BadValue))?;
        writer.raw(&bytes)
    })?;
    // Honeydew's segment shapes, none for the straight line: an `HDSh`, which a reader that does not know it skips as an
    // unknown chunk (and plays the segments as lines). Records of u32 tick, u8 shape (Linear, Square, Smooth, Stairs),
    // u8 tension (%).
    if kind == LaneKind::Automation && !clip.file.shapes.is_empty() {
        writer.chunk(*b"HDSh", |writer| {
            for &(tick, shape) in &clip.file.shapes {
                writer.u32(tick)?;
                writer.u8(shape.kind as u8)?;
                writer.u8(shape.tension)?;
            }
            Ok(())
        })?;
    }
    write_raw(writer, &clip.file.unknown_chunks)
}

/// Every record's key is ±(param + 1), so kept automation bytes still name the lane's param.
fn keys_name(raw: &[u8], param: i32) -> bool {
    let Some((size, records)) = raw.split_first_chunk::<2>() else {
        return true;
    };
    let size = usize::from(u16::from_le_bytes(*size));
    let key = automation_key(param).unsigned_abs();
    size == 0
        || (size >= 14
            && (records.chunks_exact(size))
                .all(|r| i16::from_le_bytes([r[12], r[13]]).unsigned_abs() == key))
}

/// An automation control point's key: param + 1, saturated to an `i16`.
fn automation_key(param: i32) -> i16 {
    let max = i32::from(i16::MAX);
    param.saturating_add(1).clamp(-max, max) as i16
}

/// The `EVN2` payload of `events` on a lane of `kind` with `DESc` param `param`: 20-byte records sorted by
/// position. `None` when an event is not of the lane's kind.
pub(super) fn encode_events(events: &[Event], kind: LaneKind, param: i32) -> Option<Vec<u8>> {
    let mut bytes = Vec::with_capacity(2 + 20 * events.len());
    bytes.extend_from_slice(&20_u16.to_le_bytes());
    let control_key = automation_key(param);
    let unit = |value: f32| (value.clamp(0.0, 1.0) * 65535.0).round() as u16;
    let signed = |key: i16, control: bool| if control { key } else { -key };
    let mut events: Vec<&Event> = events.iter().collect();
    events.sort_by_key(|event| event.pos());
    for event in events {
        // (pos, length, key, a, b, value, @18, @19); automation and envelopes write 0xB2 / 0x7F in a and b.
        let record = match (kind, *event) {
            (
                LaneKind::Notes,
                Event::Note {
                    pos,
                    len_beats,
                    key,
                    velocity,
                    pan,
                    slide,
                },
            ) => (
                pos,
                len_beats,
                i16::from(key),
                velocity,
                pan,
                Event::DEFAULT_PITCH,
                0,
                slide,
            ),
            (
                LaneKind::Drums,
                Event::Drum {
                    pos,
                    len_beats,
                    pad,
                    velocity,
                    pan,
                    value,
                    repeat,
                },
            ) => (
                pos,
                len_beats,
                i16::from(pad),
                velocity,
                pan,
                value,
                repeat,
                false,
            ),
            (
                LaneKind::Automation,
                Event::Automation {
                    pos,
                    value,
                    control,
                },
            ) => (
                pos,
                0.0,
                signed(control_key, control),
                0xb2,
                0x7f,
                unit(value),
                0,
                false,
            ),
            (
                LaneKind::Audio,
                Event::Envelope {
                    pos,
                    env_param,
                    value,
                    control,
                },
            ) => {
                let key = env_param.saturating_add(1);
                (
                    pos,
                    0.0,
                    signed(key, control),
                    0xb2,
                    0x7f,
                    unit(value),
                    0,
                    false,
                )
            }
            _ => return None,
        };
        let (pos, length, key, a, b, value, repeat, slide) = record;
        bytes.extend_from_slice(&pos.to_le_bytes());
        bytes.extend_from_slice(&length.to_le_bytes());
        bytes.extend_from_slice(&key.to_le_bytes());
        bytes.extend_from_slice(&[a, b]);
        bytes.extend_from_slice(&value.to_le_bytes());
        bytes.extend_from_slice(&[repeat, slide.into()]);
    }
    Some(bytes)
}

/// A slot on pad `pad_index` (−1 for an audio clip's).
fn write_slot(
    writer: &mut ChunkWriter,
    slot: &SamplerSlot,
    pad_index: f64,
) -> Result<(), FlmError> {
    let file = &slot.file;
    writer.raw(&MAGIC_SL02)?;
    writer.chunk(*b"MAIN", |writer| {
        writer.i32(-1)?;
        writer.f64(pad_index)?;
        writer.char::<512>(&slot.name)?;
        writer.char::<512>(&slot.folder.replace('\\', "/"))?;
        writer.u8(file.reverse.into())?;
        writer.f64(file.beat_offset)?;
        writer.f32(file.beat_offset_control)?;
        writer.u8(slot.cut_group)?;
        writer.u8(file.overlapping_voices.into())?;
        // The custom pad name only when renamed, unless bytes follow (they would read as the name).
        if !file.custom_pad_name.is_empty() || !file.main_extra.is_empty() {
            writer.vstr16(file.custom_pad_name.as_bytes())?;
        }
        writer.raw(&file.main_extra)
    })?;
    writer.chunk(*b"LINk", |writer| {
        writer.u32(file.id)?;
        writer.i32(file.output_bus)?;
        writer.u16(file.midi_key)?;
        writer.i32(file.send_target)?;
        writer.raw(&file.link_extra)
    })?;
    // A pad's mode is written as 1 exactly when its ratio is not 1; an audio clip's is its SYNC
    // button's. Kept bytes go back only while they hold what would be written.
    let mode = if pad_index >= 0.0 {
        u8::from(file.stretch_ratio != 1.0)
    } else {
        file.stretch_mode
    };
    let state = (mode, file.stretch_ratio, file.pitch_semitones);
    let unchanged = file.raw_stretch_state.0 == Some(state);
    writer.chunk(*b"STRC", |writer| {
        if let Some(raw) = file.raw_strc.0.as_ref().filter(|_| unchanged) {
            return writer.raw(raw);
        }
        writer.u8(mode)?;
        writer.f64(file.stretch_ratio)?;
        writer.f64(2.0_f64.powf(file.pitch_semitones / 12.0))?;
        writer.f32((file.pitch_semitones / 48.0 + 0.5) as f32)?;
        writer.f32((file.stretch_ratio.log2() / 2.0 + 0.5) as f32)
    })?;
    writer.chunk(*b"STR1", |writer| {
        if let Some(raw) = file.raw_str1.0.as_ref().filter(|_| unchanged) {
            return writer.raw(raw);
        }
        writer.u8(mode)?;
        writer.f64(file.stretch_ratio)?;
        writer.f64(file.pitch_semitones)
    })?;
    // PTH1 holds only a shorthand into the content or user root; any other sample is not saved.
    if let Some(
        path @ ContentPath::Shorthand {
            root: ContentRoot::App | ContentRoot::User,
            ..
        },
    ) = &file.path
    {
        writer.chunk(*b"PTH1", |writer| writer.content_path16(path))?;
    }
    writer.chunk(*b"PRST", |writer| {
        writer.vstr32(file.preset_path.as_bytes())?;
        writer.vstr32(file.provenance.as_bytes())
    })?;
    writer.chunk(*b"PRMS", |writer| {
        for value in slot.params {
            writer.f32(value)?;
        }
        Ok(())
    })?;
    write_raw(writer, &file.unknown_chunks)
}
