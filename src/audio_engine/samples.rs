//! The song's sample references, which the sample cache decodes before an engine is built. Each sample-playing
//! lane kind appends its source to [`walk`]. A source is a `fn(&Channel, &mut Refs)` that calls `refs.add` for each
//! sample it plays (`refs.add_slot` for a sampler slot). It can call `refs.read` to open a file that names further
//! samples, such as a `.dwp`.

use std::sync::Arc;

use crate::flm::chunk::{ChunkIter, ContentPath, ContentRoot, Cursor};
use crate::flm::sf2d::Sf2d;
use crate::panels::inst::three_osc;
use crate::project_files::BUILT_IN;
use crate::sample_cache::{Refs, SampleCache, SampleKey};
use crate::song::{Channel, LaneKind, ModuleInstance, Song};
use honeydew_audio::dwp::{Cstm, Zone};
use honeydew_audio::sample::SampleData;

/// Hands every sample `song` plays to `refs`, channel by channel.
pub(crate) fn walk(song: &Song, refs: &mut Refs) {
    for channel in &song.channels {
        drum_slots(channel, refs);
        dw_zones(channel, refs);
        audio_clips(channel, refs);
        soundfonts(channel, refs);
        // Each lane kind appends its source here: audio clips, DW zones …
        slicer_loops(channel, refs);
        custom_shapes(channel, refs);
    }
}

/// A Drums channel's pads: each slot's file or built-in one-shot, with its `STR1` rendered.
fn drum_slots(channel: &Channel, refs: &mut Refs) {
    let slots = channel.drum_kit.iter().flat_map(|kit| &kit.slots);
    slots.for_each(|slot| refs.add_slot(slot));
}

/// Each audio lane clip's `CLSm` slot, muted or not, with its `STR1` rendered.
fn audio_clips(channel: &Channel, refs: &mut Refs) {
    let lanes = channel.lanes.iter().filter(|l| l.kind == LaneKind::Audio);
    let slots = lanes
        .flat_map(|l| &l.clips)
        .filter_map(|c| c.file.sample.as_deref());
    slots.for_each(|slot| refs.add_slot(slot));
}

/// Each DW Sampler's samples.
fn dw_zones(channel: &Channel, refs: &mut Refs) {
    (channel.rack.modules.iter()).for_each(|module| dw_module(module, refs));
}

/// A DW Sampler's samples: the files its `CSTM` `ZONE`s name, and its `PATh` program's zones. Another
/// module has none.
pub(crate) fn dw_module(module: &ModuleInstance, refs: &mut Refs) {
    let blob = module.file.custom_data.as_deref();
    let Some(blob) = blob.filter(|_| module.type_id == 1) else {
        return;
    };
    let cstm = Cstm::read(blob);
    for (path, _) in &cstm.zones {
        if let Some(path) = dw_path(path) {
            refs.add(SampleKey::File(path));
        }
    }
    if let Some(path) = program_path(&cstm) {
        refs.add_program(&path);
    }
    if silent(&cstm, refs.cache()) {
        refs.add_program(&stand_in());
    }
}

/// What a DW Sampler whose files are all missing plays instead (a song from another device, its samples left there):
/// our bundled grand piano. The song keeps naming its own files.
pub(crate) fn stand_in() -> ContentPath {
    ContentPath::Shorthand {
        root: ContentRoot::App,
        relative: format!("{BUILT_IN}/{}", crate::content::GRAND_PIANO),
    }
}

/// A DW Sampler of `song` names a file in the user folder: on the web, which has only this session's copies there, it
/// is likely missing and plays [`stand_in`], which downloads with the song (15 MB: not for every factory program).
pub(crate) fn may_stand_in(song: &Song) -> bool {
    let modules = song.channels.iter().flat_map(|c| &c.rack.modules);
    let blobs = modules
        .filter(|m| m.type_id == 1)
        .filter_map(|m| m.file.custom_data.as_deref());
    blobs.map(Cstm::read).any(|cstm| {
        let program = cstm.program.as_ref().map(|program| program.path);
        let paths = program
            .into_iter()
            .chain(cstm.zones.iter().map(|(path, _)| *path));
        paths.filter_map(dw_path).any(|path| {
            matches!(
                path,
                ContentPath::Shorthand {
                    root: ContentRoot::User,
                    ..
                }
            )
        })
    })
}

/// The zones DW Sampler `cstm` plays, each with its decoded sample: its program's (`PATh`/`PATH`) or its `ZONE`s, never
/// both ([`Cstm::read`]); a zone whose sample is missing is dropped. One that plays nothing of its own as its files are
/// missing ([`silent`]) plays [`stand_in`]'s.
pub(crate) fn sampler_zones(cstm: &Cstm, samples: &SampleCache) -> Vec<(Zone, Arc<SampleData>)> {
    match own_zones(cstm, samples) {
        zones if zones.is_empty() && missing(cstm, samples) => program_zones(&stand_in(), samples),
        zones => zones,
    }
}

/// The sampler plays nothing of its own, as a load found its files missing. Nothing is missing before a load has met
/// them (the web's download plan), so only a song that needs it loads [`stand_in`].
fn silent(cstm: &Cstm, samples: &SampleCache) -> bool {
    own_zones(cstm, samples).is_empty() && missing(cstm, samples)
}

fn own_zones(cstm: &Cstm, samples: &SampleCache) -> Vec<(Zone, Arc<SampleData>)> {
    match program_path(cstm) {
        Some(path) => program_zones(&path, samples),
        None => (cstm.zones.iter())
            .filter_map(|(path, zone)| {
                Some((*zone, samples.get(&SampleKey::File(dw_path(path)?))?))
            })
            .collect(),
    }
}

fn program_zones(path: &ContentPath, samples: &SampleCache) -> Vec<(Zone, Arc<SampleData>)> {
    let program = samples.program(path);
    let zones = program.iter().flat_map(|zones| zones.iter());
    zones
        .filter_map(|(zone, key)| Some((*zone, samples.get(key)?)))
        .collect()
}

/// A file the sampler names (its program, the program's zones, or its own `ZONE`s) is missing.
fn missing(cstm: &Cstm, samples: &SampleCache) -> bool {
    match program_path(cstm) {
        Some(path) => {
            let zones = samples.program(&path);
            let mut keys = zones
                .iter()
                .flat_map(|zones| zones.iter())
                .map(|zone| &zone.1);
            samples.is_missing(&SampleKey::File(path.clone()))
                || keys.any(|key| samples.is_missing(key))
        }
        None => (cstm.zones.iter())
            .filter_map(|(path, _)| dw_path(path))
            .any(|path| samples.is_missing(&SampleKey::File(path))),
    }
}

/// The program a DW `CSTM` names (`PATh`/`PATH`).
fn program_path(cstm: &Cstm) -> Option<ContentPath> {
    cstm.program
        .as_ref()
        .and_then(|program| dw_path(program.path))
}

/// The content path a DW `CSTM` stores as shorthand bytes, `None` when it is empty.
pub(crate) fn dw_path(raw: &[u8]) -> Option<ContentPath> {
    let vstr = [&(raw.len() as u32).to_le_bytes()[..], raw].concat();
    let path = Cursor::new(&vstr, "CSTM").content_path32().ok()?;
    let (ContentPath::Shorthand { relative: text, .. } | ContentPath::Plain(text)) = &path;
    (!text.is_empty()).then_some(path)
}

/// Each Slicer's loop: its `CSTM` `PATH`, our own `@R@Built-in/Loops/…` ones from code.
fn slicer_loops(channel: &Channel, refs: &mut Refs) {
    (channel.rack.modules.iter())
        .filter_map(super::slicer::loop_path)
        .for_each(|path| refs.add(SampleKey::of_path(&path)));
}

/// Each 3x Osc's custom waveform: its `CSTM` `PATH`, kept as its built table.
fn custom_shapes(channel: &Channel, refs: &mut Refs) {
    (channel.rack.modules.iter())
        .filter_map(shape_key)
        .for_each(|key| refs.add_shape(key));
}

/// A 3x Osc's custom waveform: the path its `CSTM` `PATH` chunk holds. `None` for another module,
/// or a 3x Osc without one.
pub(crate) fn shape_path(module: &ModuleInstance) -> Option<ContentPath> {
    if module.type_id != three_osc::TYPE_ID {
        return None;
    }
    let blob = module.file.custom_data.as_deref()?;
    let mut chunks = ChunkIter::new(blob, "CSTM").map_while(Result::ok);
    let path = chunks
        .find(|c| c.id == *b"PATH")?
        .reader()
        .content_path32()
        .ok()?;
    let (ContentPath::Shorthand { relative: text, .. } | ContentPath::Plain(text)) = &path;
    (!text.is_empty()).then_some(path)
}

/// The sample a 3x Osc's custom waveform plays ([`shape_path`], [`SampleKey::of_path`]): our own
/// `@R@Built-in/Shapes/…` ones from code.
pub(crate) fn shape_key(module: &ModuleInstance) -> Option<SampleKey> {
    shape_path(module).map(|path| SampleKey::of_path(&path))
}

/// The SoundFont each SoundFont Player of the rack names in its `SF2D`.
fn soundfonts(channel: &Channel, refs: &mut Refs) {
    let players = channel
        .rack
        .modules
        .iter()
        .filter(|m| m.type_id == SOUNDFONT_PLAYER);
    players
        .filter_map(|module| Sf2d::read(module.file.custom_data.as_deref()?))
        .for_each(|sf2d| refs.add_soundfont(sf2d.path));
}

/// The SoundFont Player's module type id.
const SOUNDFONT_PLAYER: u16 = 52;
