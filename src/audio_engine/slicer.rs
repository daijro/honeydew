//! The Slicer (ID 33) in the engine: its loop and Sensitivity from its `CSTM` (`SENS` and `PATH`), its slices and keys,
//! and AUTO's clip ([`auto_clip`], the app seam). Slicing runs here, and only when a loop or a Sensitivity is new: on a
//! loop pick's worker, else on the app thread at an engine build ([`slices`]).

use std::hash::{Hash, Hasher};
use std::sync::{Arc, Mutex, PoisonError, Weak};

use crate::flm::chunk::{ChunkIter, ContentPath, Cursor};
use crate::panels::cstm::{slicer_auto, slicer_sensitivity};
use crate::sample_cache::{Reader, SampleCache, SampleKey};
use crate::song::{Channel, Event, LaneKind, ModuleInstance, PPQ, Song};
use honeydew_audio::sample::SampleData;
use honeydew_audio::slicer::{self, SENSITIVITY_BEFORE_CSTM, Slicer};

const TYPE_ID: u16 = 33;

/// The first chunk `id` of `module`'s `CSTM`, when it is a Slicer.
fn chunk(module: &ModuleInstance, id: [u8; 4]) -> Option<Cursor<'_>> {
    if module.type_id != TYPE_ID {
        return None;
    }
    let blob = module.file.custom_data.as_deref()?;
    let chunk = ChunkIter::new(blob, "CSTM")
        .map_while(Result::ok)
        .find(|c| c.id == id)?;
    Some(chunk.reader())
}

/// The loop a Slicer plays: its `PATH`. `None` for another module, or a Slicer without one.
pub(crate) fn loop_path(module: &ModuleInstance) -> Option<ContentPath> {
    let path = chunk(module, *b"PATH")?.content_path32().ok()?;
    let (ContentPath::Shorthand { relative: text, .. } | ContentPath::Plain(text)) = &path;
    (!text.is_empty()).then_some(path)
}

/// A Slicer's Sensitivity: its `CSTM`'s ([`slicer_sensitivity`], which the panel shows), else the engine's value before
/// any `CSTM` is read (0.33).
pub(crate) fn sensitivity(module: &ModuleInstance) -> f32 {
    let blob = module.file.custom_data.as_deref();
    let blob = blob.filter(|_| module.type_id == TYPE_ID);
    blob.map_or(SENSITIVITY_BEFORE_CSTM, slicer_sensitivity)
}

/// A loop, a Sensitivity (its bits) and the slices detected there.
type Sliced = (Weak<SampleData>, u32, Arc<[usize]>);

/// Slices already detected, so that an engine rebuild slices again only when a loop or a Sensitivity is new, and a loop
/// a pick's worker sliced (`sample_picker::decode_pick`) is not sliced again. A loop no one holds is forgotten, and a
/// loop keeps its last [`PER_LOOP`] Sensitivities' (a Sensitivity drag rebuilds at each value).
// ponytail: linear scan; key it by hash if songs ever hold many Slicers.
static SLICES: Mutex<Vec<Sliced>> = Mutex::new(Vec::new());

/// The Sensitivities [`SLICES`] keeps per loop.
const PER_LOOP: usize = 4;

/// `sample`'s slices at `sensitivity` ([`slicer::detect`]), detected once per loop and Sensitivity, on any thread. The
/// memo is locked to look and to keep, never across a detect, so a worker slicing a long loop never stalls the UI.
pub(crate) fn slices(sample: &Arc<SampleData>, sensitivity: f32) -> Arc<[usize]> {
    let memo = || SLICES.lock().unwrap_or_else(PoisonError::into_inner);
    let key = (Arc::as_ptr(sample), sensitivity.to_bits());
    let found = {
        let mut memo = memo();
        memo.retain(|(sample, ..)| sample.strong_count() > 0);
        let found = memo.iter().find(|(s, bits, _)| (s.as_ptr(), *bits) == key);
        found.map(|(.., slices)| slices.clone())
    };
    if let Some(slices) = found {
        return slices;
    }
    #[cfg(test)]
    DETECTS.set(DETECTS.get() + 1);
    let slices: Arc<[usize]> = slicer::detect(sample, sensitivity).into();
    let mut memo = memo();
    let mine = |(s, ..): &Sliced| s.as_ptr() == key.0;
    if memo.iter().filter(|s| mine(s)).count() >= PER_LOOP {
        let oldest = memo.iter().position(mine).expect("one of its own");
        memo.remove(oldest);
    }
    memo.push((Arc::downgrade(sample), key.1, slices.clone()));
    slices
}

#[cfg(test)]
thread_local! {
    /// The loops [`slices`] detected on this thread.
    static DETECTS: std::cell::Cell<usize> = const { std::cell::Cell::new(0) };
}

/// How many loops [`slices`] detected on this thread: the UI thread's (a test's own) stays as it was across a pick.
#[cfg(test)]
pub(crate) fn detects_here() -> usize {
    DETECTS.get()
}

/// What one Slicer plays.
struct Loaded {
    instance_id: i32,
    sample: Option<Arc<SampleData>>,
    slices: Arc<[usize]>,
    first: u8,
}

/// `channel`'s Slicers in rack order, each with its loop from `samples`, its slices and its first key.
/// `Set First Note` stores its key as the `ADD1` low key, which the key range also applies to. The default 0
/// means none was set, and the first key is recomputed: 48, or one above the highest key a Slicer higher in the rack
/// plays (as if the loops were loaded top to bottom). Either is clamped so the last slice's key is ≤ 127.
fn slicers<'a>(
    channel: &'a Channel,
    samples: &'a SampleCache,
) -> impl Iterator<Item = Loaded> + 'a {
    let mut highest = None;
    let modules = channel.rack.modules.iter();
    modules.filter(|m| m.type_id == TYPE_ID).map(move |module| {
        let sample = loop_path(module).and_then(|path| samples.get(&SampleKey::of_path(&path)));
        let slices = (sample.as_ref()).map_or_else(
            || Arc::from([]),
            |sample| slices(sample, sensitivity(module)),
        );
        let set = u8::try_from(module.file.key_low)
            .ok()
            .filter(|&key| key > 0);
        let first = slicer::first_note(set.map(|key| key - 1).or(highest), slices.len());
        if let Some(last) = slices.len().checked_sub(1) {
            highest = highest.max(Some(first + last as u8));
        }
        Loaded {
            instance_id: module.instance_id,
            sample,
            slices,
            first,
        }
    })
}

/// `Set First Note` on a loop with too many slices for the key: each Slicer of `channel` whose first note is set
/// (`ADD1` low > 0) and plays lower, clamped so its last slice is ≤ 127, stores the key it plays, so the key range
/// starts where slice 0 sounds. Whether any changed. The app calls it after a rack edit, before the rebuild.
pub fn settle_first_notes(channel: &mut Channel, samples: &SampleCache) -> bool {
    let played: Vec<(i32, u8)> =
        (slicers(channel, samples).map(|s| (s.instance_id, s.first))).collect();
    let mut changed = false;
    for (id, first) in played {
        let Some(module) = channel.rack.get_mut(id) else {
            continue;
        };
        if module.file.key_low > 0 && module.file.key_low > first as i8 {
            module.file.key_low = first as i8;
            changed = true;
        }
    }
    changed
}

/// The Slicer `module` of `channel`, playing its loop from `samples`.
pub(super) fn instrument(
    channel: &Channel,
    module: &ModuleInstance,
    sample_rate: f32,
    samples: &SampleCache,
    origin: &mut impl Hasher,
) -> Option<Slicer> {
    let loaded = slicers(channel, samples).find(|s| s.instance_id == module.instance_id)?;
    let sample = loaded.sample.as_ref().map(Arc::as_ptr);
    (&module.file.custom_data, sample, loaded.first).hash(origin);
    Some(Slicer::new(
        sample_rate,
        loaded.sample,
        loaded.slices,
        loaded.first,
    ))
}

/// AUTO's clip for the Slicer `instance_id` on channel `channel`: the clip's loop length in beats and one note per
/// slice ([`slicer::auto_clip`]), velocity 0.7 (179/255), centred, each starting on the tick at or before its slice.
/// `None` without that Slicer or its loop. [`write_auto_clip`] writes it into the current clip.
pub fn auto_clip(
    song: &Song,
    channel: usize,
    instance_id: i32,
    samples: &SampleCache,
) -> Option<(f64, Vec<Event>)> {
    let loaded =
        slicers(song.channels.get(channel)?, samples).find(|s| s.instance_id == instance_id)?;
    let sample = loaded.sample?;
    let frames = sample.frames();
    let beats = slicer::auto_clip_beats(frames, sample.meta.tempo, song.tempo_bpm);
    let notes = slicer::auto_clip(&loaded.slices, frames, loaded.first, beats);
    let events = notes.iter().map(|note| Event::Note {
        pos: (note.start_beats * f64::from(PPQ)) as u32,
        len_beats: note.length_beats,
        key: note.key,
        velocity: (note.velocity * 255.0).round() as u8,
        pan: Event::DEFAULT_PAN,
        slide: false,
    });
    Some((beats, events.collect()))
}

/// The clip AUTO writes on `channel` (its lane and id): `held`, the clip the channel's piano roll edits, when it is
/// there; else the clip its main lane's `current_clip_id` names (a loaded file's); else its first notes lane's first
/// clip. `None` without a clip.
fn auto_target(channel: &Channel, held: Option<(usize, u32)>) -> Option<(usize, u32)> {
    let lanes = &channel.lanes;
    let exists = |&(lane, id): &(usize, u32)| {
        (lanes.get(lane)).is_some_and(|l| l.clips.iter().any(|c| c.id == id))
    };
    let current = lanes.first().map(|l| (0, l.file.current_clip_id));
    let first = || {
        let lane = lanes.iter().position(|l| l.kind == LaneKind::Notes)?;
        Some((lane, lanes[lane].clips.first()?.id))
    };
    held.filter(exists)
        .or(current.filter(exists))
        .or_else(first)
}

/// AUTO's clip ([`auto_clip`]) written into `channel`'s current clip ([`auto_target`] of `held`): its notes, and a loop
/// and visible length of the clip's beats from content beat 0. Only while the Slicer `instance_id`'s `SENS` AUTO flag
/// is on (on a slice re-detect: a new loop or Sensitivity), unless `always` (its momentary `AUTO` button: a one-shot
/// rewrite that leaves the flag as it is; so only a file switches AUTO off). The song's samples are loaded first
/// (`read`; a live load, [`SampleCache::load_live`]), so a loop just picked is sliced. Whether the clip changed: never
/// without a clip or the Slicer's loop.
pub fn write_auto_clip(
    song: &mut Song,
    channel: usize,
    instance_id: i32,
    samples: &mut SampleCache,
    read: &Reader<'_>,
    always: bool,
    held: Option<(usize, u32)>,
) -> bool {
    let auto = |m: &ModuleInstance| m.file.custom_data.as_deref().is_none_or(slicer_auto);
    let slicer = song.find_module(channel, instance_id);
    if !slicer.is_some_and(|m| m.type_id == TYPE_ID && (always || auto(m))) {
        return false;
    }
    samples.load_live(song, song, read);
    let Some((beats, events)) = auto_clip(song, channel, instance_id, samples) else {
        return false;
    };
    let Some((lane, id)) = auto_target(&song.channels[channel], held) else {
        return false;
    };
    let lane = &mut song.channels[channel].lanes[lane];
    let Some(clip) = lane.clips.iter_mut().find(|c| c.id == id) else {
        return false;
    };
    let lengths = (
        clip.loop_len_beats,
        clip.visible_len_beats,
        clip.offset_beats,
    );
    let changed = clip.events != events || lengths != (beats, beats, 0.0);
    clip.events = events;
    (
        clip.loop_len_beats,
        clip.visible_len_beats,
        clip.offset_beats,
    ) = (beats, beats, 0.0);
    changed
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::flm::chunk::ContentRoot;
    use honeydew_audio::sample::SampleMeta;

    /// The panel's knob shows the Sensitivity the engine slices at, from every blob: without a `SENS`, or with a short
    /// one, the engine's 0.33; a non-finite one reads as 0, as every file f32; the rest clamped.
    #[test]
    fn the_panel_and_the_engine_read_one_sensitivity() {
        use crate::flm::chunk::ChunkWriter;
        use crate::panels::cstm::settings_from_cstm;
        let chunk = |id: [u8; 4], payload: &[u8]| {
            let mut w = ChunkWriter::default();
            w.chunk(id, |w| w.raw(payload)).unwrap();
            w.into_bytes()
        };
        let sens = |v: f32| chunk(*b"SENS", &[&v.to_le_bytes()[..], &[1]].concat());
        for (blob, want) in [
            (chunk(*b"PATH", &[0; 4]), 0.33),
            (chunk(*b"SENS", &[0, 0]), 0.33),
            (sens(f32::NAN), 0.0),
            (sens(f32::INFINITY), 0.0),
            (sens(0.6), 0.6),
            (sens(2.0), 1.0),
        ] {
            let mut module = ModuleInstance::new(TYPE_ID, 2);
            module.file.custom_data = Some(blob.clone());
            assert_eq!(sensitivity(&module), want, "{blob:?}");
            let panel = settings_from_cstm(TYPE_ID, &blob).unwrap()[&3100];
            assert_eq!(panel, want, "{blob:?}");
        }
    }

    #[test]
    fn auto_writes_the_rolls_clip_else_the_files_current_one_else_the_first_notes_clip() {
        let mut song = Song::default();
        song.add_channel(crate::song::ChannelKind::Synth(
            crate::song::SynthKind::Slicer,
        ));
        let channel = song.channels.last_mut().unwrap();
        assert_eq!(auto_target(channel, None), None, "no clip");
        let lane = &mut channel.lanes[0];
        assert_eq!(lane.kind, LaneKind::Notes);
        let (a, b, c) = (
            lane.add_clip(0, 4.0).id,
            lane.add_clip(512, 4.0).id,
            lane.add_clip(1024, 4.0).id,
        );
        assert_eq!(auto_target(channel, None), Some((0, a)));
        // A loaded file's `DESc`@24.
        channel.lanes[0].file.current_clip_id = c;
        assert_eq!(auto_target(channel, None), Some((0, c)));
        assert_eq!(auto_target(channel, Some((0, b))), Some((0, b)));
        // One that is not there falls through.
        assert_eq!(auto_target(channel, Some((0, 99))), Some((0, c)));
        assert_eq!(auto_target(channel, Some((5, b))), Some((0, c)));
        channel.lanes[0].file.current_clip_id = 99;
        assert_eq!(auto_target(channel, None), Some((0, a)));
    }

    fn slicer_with(blob: &[u8]) -> ModuleInstance {
        let mut module = ModuleInstance::new(TYPE_ID, 1);
        module.file.custom_data = Some(blob.to_vec());
        module
    }

    #[test]
    fn the_cstm_names_the_loop_and_the_sensitivity() {
        let path = b"@L@\0My Samples/a.wav";
        let blob = [
            &b"SENS\x05\0\0\0"[..],
            &0.5f32.to_le_bytes(),
            &[1],
            b"PATH",
            &(4 + path.len() as u32).to_le_bytes(),
            &(path.len() as u32).to_le_bytes(),
            path,
        ]
        .concat();
        let module = slicer_with(&blob);
        assert_eq!(
            loop_path(&module),
            Some(ContentPath::Shorthand {
                root: ContentRoot::User,
                relative: "My Samples/a.wav".into()
            })
        );
        assert_eq!(sensitivity(&module), 0.5);
        // No SENS (or no CSTM): 0.33. A NaN reads as 0.
        assert_eq!(sensitivity(&slicer_with(&blob[13..])), 0.33);
        assert_eq!(sensitivity(&ModuleInstance::new(TYPE_ID, 1)), 0.33);
        let nan = [&b"SENS\x05\0\0\0"[..], &f32::NAN.to_le_bytes(), &[1]].concat();
        assert_eq!(sensitivity(&slicer_with(&nan)), 0.0);
        // Another module's CSTM is not a Slicer's; an empty path names nothing.
        let mut other = slicer_with(&blob);
        other.type_id = 39;
        assert_eq!(loop_path(&other), None);
        assert_eq!(loop_path(&slicer_with(b"PATH\x04\0\0\0\0\0\0\0")), None);
    }

    #[test]
    fn set_first_note_is_the_add1_low_key_and_0_the_computed_one() {
        let mut song = Song::default();
        song.add_channel(crate::song::ChannelKind::Synth(
            crate::song::SynthKind::Slicer,
        ));
        let channel = &mut song.channels[1];
        let samples = SampleCache::default();
        let firsts = |c: &Channel| slicers(c, &samples).map(|s| s.first).collect::<Vec<_>>();
        assert_eq!(firsts(channel), [48]);
        channel.rack.modules[0].file.key_low = 60;
        assert_eq!(firsts(channel), [60]);
    }

    /// A Slicer channel (channel 1) whose loop has a burst every 100 ms for `seconds`: 10 slices a second.
    fn burst_loop(seconds: usize) -> (Song, SampleCache) {
        let mut song = Song::default();
        song.add_channel(crate::song::ChannelKind::Synth(
            crate::song::SynthKind::Slicer,
        ));
        let path = b"@L@\0loop.wav";
        let mut cstm = b"SENS\x05\0\0\0".to_vec();
        cstm.extend(0.25f32.to_le_bytes());
        cstm.push(1);
        cstm.extend(b"PATH");
        cstm.extend((4 + path.len() as u32).to_le_bytes());
        cstm.extend((path.len() as u32).to_le_bytes());
        cstm.extend(path);
        song.channels[1].rack.modules[0].file.custom_data = Some(cstm);
        let data: Vec<u8> = (0..44_100 * seconds)
            .map(|i| match i % 4410 {
                t if t < 400 => ((t as f32 * 0.37).sin() * 20_000.0) as i16,
                _ => 0,
            })
            .flat_map(i16::to_le_bytes)
            .collect();
        let mut wav = b"RIFF".to_vec();
        wav.extend((36 + data.len() as u32).to_le_bytes());
        wav.extend(b"WAVEfmt \x10\0\0\0\x01\0\x01\0");
        wav.extend(44_100u32.to_le_bytes());
        wav.extend(88_200u32.to_le_bytes());
        wav.extend(b"\x02\0\x10\0data");
        wav.extend((data.len() as u32).to_le_bytes());
        wav.extend(data);
        let mut cache = SampleCache::default();
        cache.load(&song, &|_| Some(wav.clone()));
        (song, cache)
    }

    /// A first note set too high for its slices is stored as the clamped one, which is what plays.
    #[test]
    fn a_first_note_too_high_for_the_slices_is_stored_clamped() {
        use honeydew_audio::device::AudioRenderer;
        let (mut song, cache) = burst_loop(2);
        let count = slicers(&song.channels[1], &cache)
            .next()
            .unwrap()
            .slices
            .len();
        assert_eq!(count, 20);
        let module = &mut song.channels[1].rack.modules[0];
        module.file.key_low = 120;
        assert!(settle_first_notes(&mut song.channels[1], &cache));
        assert_eq!(song.channels[1].rack.modules[0].file.key_low, 108);
        assert!(
            !settle_first_notes(&mut song.channels[1], &cache),
            "settled"
        );
        // Slice 0 plays on 108, inside the stored range.
        let mut engine =
            super::super::SongAudioEngine::with_samples(&song, 44_100.0, &cache).unwrap();
        engine.note_on(1, 108, 1.0);
        let mut out = vec![0.0_f32; 2048 * 2];
        engine.render(&mut out, 2);
        assert!(out.iter().any(|s| s.abs() > 0.01));
        // An unset first note (0) and one that fits are left alone.
        song.channels[1].rack.modules[0].file.key_low = 0;
        assert!(!settle_first_notes(&mut song.channels[1], &cache));
        song.channels[1].rack.modules[0].file.key_low = 60;
        assert!(!settle_first_notes(&mut song.channels[1], &cache));
    }

    #[test]
    fn a_loop_is_sliced_once_per_sensitivity() {
        let sample = Arc::new(SampleData {
            sample_rate: 44_100,
            left: vec![0.5; 10_000].into(),
            right: None,
            meta: SampleMeta::default(),
        });
        let once = slices(&sample, 0.25);
        assert!(Arc::ptr_eq(&once, &slices(&sample, 0.25)));
        assert!(!Arc::ptr_eq(&once, &slices(&sample, 0.5)));
        // A new loop (even an equal one) is sliced again.
        let copy = Arc::new((*sample).clone());
        assert!(!Arc::ptr_eq(&once, &slices(&copy, 0.25)));
        let gone = Arc::downgrade(&sample);
        drop(sample);
        slices(&copy, 0.25);
        let memo = SLICES.lock().unwrap();
        assert!(!memo.iter().any(|(s, ..)| s.ptr_eq(&gone)), "forgotten");
    }

    #[test]
    fn a_loop_keeps_the_slices_of_its_last_four_sensitivities() {
        // A Sensitivity drag rebuilds at each value: the memo keeps the latest few per loop, not every one.
        let sample = Arc::new(SampleData {
            sample_rate: 44_100,
            left: vec![0.5; 10_000].into(),
            right: None,
            meta: SampleMeta::default(),
        });
        let mut last = Arc::from([]);
        for i in 0..10 {
            last = slices(&sample, i as f32 / 10.0);
        }
        let memo = SLICES.lock().unwrap();
        let ptr = Arc::as_ptr(&sample);
        assert_eq!(memo.iter().filter(|(s, ..)| s.as_ptr() == ptr).count(), 4);
        drop(memo);
        let detects = detects_here();
        assert!(Arc::ptr_eq(&last, &slices(&sample, 0.9)));
        assert_eq!(detects_here(), detects, "the latest is a hit");
    }
}
