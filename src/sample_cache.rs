//! Decoded samples for the engine. [`crate::audio_link::AudioLink`] owns a [`SampleCache`]. Before it builds an engine
//! it loads the song's samples on the app thread: it walks the references (`audio_engine/samples.rs`) and decodes only
//! what is new, after it frees what the song no longer plays. A DW Sampler pick decodes its program on the picker's
//! worker instead, into a cache of its own that [`SampleCache::absorb`] takes, and so does a picked loop, shape or
//! sample ([`SampleCache::load_sample`]). The engine takes `Arc` clones, so nothing decodes on the audio thread.
//!
//! Files are named by content paths and read through a [`Reader`]. The app's reader is [`reader`], built on
//! [`ProjectFiles`].

use std::collections::{HashMap, HashSet};
use std::path::Path;
use std::sync::Arc;

use honeydew_audio::builtin_drums::builtin;
use honeydew_audio::drums::{
    PadSample, STREAM_FRAMES, Stream, Stretching, stretch, stretch_frames,
};
use honeydew_audio::dwp::{self, Zone};
use honeydew_audio::sample::{MAX_FRAMES, SampleData, decode};
use honeydew_audio::sample_edit::summarize;
use honeydew_audio::soundfont::SoundFont;
use honeydew_audio::three_osc::osc::{CYCLE, MIP_LEVELS, MIP_POINTS, Wave};

use crate::flm::chunk::{ContentPath, ContentRoot};
use crate::project_files::content::built_in;
use crate::project_files::{
    BUILT_IN, MAX_FILE_BYTES, ProjectFiles, in_library, read_limited, resolve_in,
};
use crate::song::{ModuleInstance, SamplerSlot, Song};

/// Reads the file a content path names: `None` when it is missing.
pub type Reader<'a> = dyn Fn(&ContentPath) -> Option<Vec<u8>> + 'a;

/// One sample a song plays, as the cache keys it.
#[derive(Clone, Debug, PartialEq, Eq, Hash)]
pub enum SampleKey {
    /// A sample file, by its content path. Every reference has a key, including one that cannot resolve (`@R@`), so
    /// that [`SampleCache::missing`] can list it.
    File(ContentPath),
    /// A built-in one-shot, by its slot folder `Built-in/<Pad>` with `/`.
    Builtin(String),
    /// A DW program's (`.dwp`) zone `n` that embeds its audio (records 0x205/0x206), by the program's path.
    Embedded(ContentPath, u32),
}

/// A DW program's zones, each with the sample it plays.
pub type DwProgram = Arc<[(Zone, SampleKey)]>;

/// A sample and its waveform summary, each channel's [`summarize`] (left, then right): what the views that draw it read
/// ([`SampleCache::waveform`]), so that a column over many frames reads their pairs instead.
#[derive(Clone, Debug)]
pub struct Waveform {
    pub data: Arc<SampleData>,
    pub summary: Arc<[Vec<(f32, f32)>]>,
}

impl Waveform {
    /// `data` with its summary, made now: one pass over its frames.
    pub fn new(data: Arc<SampleData>) -> Self {
        let summary = std::iter::once(&*data.left)
            .chain(data.right.as_deref())
            .map(summarize)
            .collect();
        Self { data, summary }
    }

    /// Each channel's frames and summary, left then right.
    pub fn channels(&self) -> impl Iterator<Item = (&[f32], &[(f32, f32)])> {
        (std::iter::once(&*self.data.left).chain(self.data.right.as_deref()))
            .zip(self.summary.iter().map(Vec::as_slice))
    }
}

impl SampleKey {
    /// What the content path `path` plays: our own `@R@Built-in/<rel>` (either
    /// separator) is the built-in sample `Built-in/<rel>` without its extension, which code makes before any disk or
    /// Original Content lookup; any other path is its file.
    pub fn of_path(path: &ContentPath) -> Self {
        if let ContentPath::Shorthand {
            root: ContentRoot::App,
            relative,
        } = path
        {
            let relative = relative.replace('\\', "/");
            if (relative.strip_prefix(BUILT_IN)).is_some_and(|r| r.starts_with('/')) {
                // ponytail: the last `.` starts the extension; our names have no other.
                let stem = match relative.rsplit_once('.') {
                    Some((stem, ext)) if !ext.contains('/') => stem,
                    _ => &relative,
                };
                return Self::Builtin(stem.to_owned());
            }
        }
        Self::File(path.clone())
    }

    /// What a sampler slot plays. That is its `PTH1` file
    /// ([`Self::of_path`]). Without one, it is the built-in one-shot its `Built-in/<Pad>` folder names (either
    /// separator). Otherwise it is nothing.
    pub fn of_slot(slot: &SamplerSlot) -> Option<Self> {
        if let Some(path) = &slot.file.path {
            return Some(Self::of_path(path));
        }
        let folder = slot.folder.replace('\\', "/");
        folder
            .starts_with("Built-in/")
            .then_some(Self::Builtin(folder))
    }
}

/// The (ratio, semitones) a slot's `STR1` renders with: the ratio only with time-stretch on (mode ≠ 0).
fn str1(slot: &SamplerSlot) -> (f64, f64) {
    let file = &slot.file;
    let ratio = if file.stretch_mode != 0 {
        file.stretch_ratio
    } else {
        1.0
    };
    (ratio, file.pitch_semitones)
}

/// A slot's `STR1` render streaming in (the drum strip's `Stretch`, a pad's sample pick and every render a live load
/// misses), from the sample the cache already holds: [`SampleCache::stream_slot`] renders its first [`STREAM_FRAMES`]
/// into the cache, which pads play at once ([`SampleCache::pad_sample`]); [`Self::render`] renders the rest into that
/// buffer on a worker ([`SampleCache::take_jobs`]), and [`SampleCache::put_stretch`] takes the whole render in its
/// place.
pub struct StretchJob {
    key: StretchKey,
    sample: Arc<SampleData>,
    stream: Arc<Stream>,
    stretching: Option<Stretching>,
    render: Option<SampleData>,
}

/// The [`StretchJob`]s started and not yet taken ([`SampleCache::take_jobs`]). A clone of the cache takes none: they are
/// its own to run.
#[derive(Default)]
struct Jobs(Vec<StretchJob>);

impl Clone for Jobs {
    fn clone(&self) -> Self {
        Self::default()
    }
}

impl StretchJob {
    /// Renders the rest, [`STREAM_FRAMES`] at a time: a worker's work, which allocates. It stops, with no render to
    /// land, once neither the cache nor an engine holds the stream (only the job does, twice): no one would hear the
    /// rest (quick `Stretch` increments, undo steps).
    pub fn render(mut self) -> Self {
        let Some(mut stretching) = self.stretching.take() else {
            return self;
        };
        let stream = &self.stream;
        while stream.filled() < stream.frames() {
            if Arc::strong_count(stream) <= 2 {
                return self;
            }
            stretching.fill(stream.filled() + STREAM_FRAMES);
        }
        self.render = Some(stretching.finish());
        self
    }
}

/// A slot's sample with its `STR1` rendered: the sample, the mode, and the bits of the ratio and the semitones.
#[derive(Clone, Debug, PartialEq, Eq, Hash)]
struct StretchKey(SampleKey, u8, u64, u64);

impl StretchKey {
    /// The rendered sample `slot` plays, `None` when its `STR1` changes nothing: mode 0 (no time-stretch) and no
    /// semitones.
    fn of_slot(slot: &SamplerSlot, key: SampleKey) -> Option<Self> {
        let file = &slot.file;
        let (mode, ratio, semitones) =
            (file.stretch_mode, file.stretch_ratio, file.pitch_semitones);
        (mode != 0 || semitones != 0.0)
            .then(|| Self(key, mode, ratio.to_bits(), semitones.to_bits()))
    }
}

/// The most memory the cache holds: decoded PCM (`f32`, all channels), the files of the parsed SoundFonts and the
/// Custom tables of the 3x Oscs. A song names any number of files, so the limits of one file
/// ([`crate::project_files::MAX_FILE_BYTES`], `MAX_FRAMES`) do not bound the total. A sample, SoundFont or table over
/// what is left is missing, like one that does not decode. The web has less memory to give.
const MAX_DECODED_BYTES: usize = if cfg!(target_arch = "wasm32") {
    1 << 30
} else {
    2 << 30
};

/// The bytes of a Custom table ([`Wave::custom`]'s `byte_len`): the raw cycle and the mip levels, `f32`.
const TABLE_BYTES: usize = 4 * (CYCLE + MIP_LEVELS * MIP_POINTS);

/// The bytes `sample` holds as decoded PCM.
fn pcm_bytes(sample: &SampleData) -> usize {
    4 * (sample.left.len() + sample.right.as_ref().map_or(0, |right| right.len()))
}

/// The decoded samples, and the parsed SoundFonts, by key. One that failed (no file, or bytes that do not
/// decode or parse) is kept as missing. It is not tried again until [`Self::forget_missing`], or until a song load
/// starts a new cache. Everything is kept while the song plays it: [`Self::load`] frees the rest, so that an edit's
/// leftovers do not hold the byte budget. Slots' samples with their `STR1` stretch and pitch are rendered once each.
/// A clone shares the decoded data (a split export keeps one for its stems).
#[derive(Clone, Default)]
pub struct SampleCache {
    /// Tests lower the byte cap ([`MAX_DECODED_BYTES`]) to try it with tiny samples.
    max_bytes: Option<usize>,
    samples: HashMap<SampleKey, Option<Arc<SampleData>>>,
    /// DW programs by their file (a `File` key); `None` when the file is missing. A file that is no `.dwp` is a program
    /// of one user-sample zone playing that file, whose sample is missing when it does not decode.
    programs: HashMap<SampleKey, Option<DwProgram>>,
    /// Keyed by [`SampleKey::File`] of the `SF2D` path.
    soundfonts: HashMap<SampleKey, Option<Arc<SoundFont>>>,
    /// `None`: the render did not fit the byte budget ([`MAX_DECODED_BYTES`]).
    stretched: HashMap<StretchKey, Option<Arc<SampleData>>>,
    /// Renders streaming in ([`StretchJob`]), until the whole render goes in their place: no live walk renders them
    /// again.
    streams: HashMap<StretchKey, Arc<Stream>>,
    jobs: Jobs,
    /// 3x Osc Custom tables by their file ([`Refs::add_shape`]).
    /// `None`: the table did not fit the byte budget ([`MAX_DECODED_BYTES`]).
    shapes: HashMap<SampleKey, Option<Arc<Wave>>>,
    /// Decoded samples of `samples` with their summaries, made when a view first draws one ([`SampleCache::waveform`]).
    waveforms: HashMap<SampleKey, Waveform>,
    /// The stretches rendered so far.
    renders: usize,
    /// The Custom tables built so far.
    builds: usize,
}

impl SampleCache {
    #[cfg(test)]
    fn with_max_bytes(max_bytes: usize) -> Self {
        Self {
            max_bytes: Some(max_bytes),
            ..Self::default()
        }
    }

    /// The bytes the cache can still take: the decoded PCM of the samples and renders, the files the SoundFonts keep and
    /// the Custom tables. It sums what is kept: they number in the hundreds, and a counter would have to follow every insert,
    /// [`Self::forget`] and program zone.
    fn room(&self) -> usize {
        let pcm = (self.samples.values().chain(self.stretched.values()))
            .flatten()
            .map(|s| pcm_bytes(s));
        let streams = self.streams.values().map(|s| s.byte_len());
        let fonts = self.soundfonts.values().flatten().map(|f| f.byte_len());
        let tables = self.shapes.values().flatten().map(|w| w.byte_len());
        let held: usize = pcm.chain(streams).chain(fonts).chain(tables).sum();
        (self.max_bytes.unwrap_or(MAX_DECODED_BYTES)).saturating_sub(held)
    }

    /// Decodes each sample `song` plays that the cache has not met yet, reading files with `read`. App thread only.
    /// What `song` does not play is freed first (samples, waveform summaries, DW programs with their zones,
    /// SoundFonts, slot stretches, Custom tables), so that the new samples find its room: an undone take or a swapped
    /// sample does not hold the byte budget until the next song load. A holder of the `Arc` (the engine, an editor)
    /// keeps its own.
    pub fn load(&mut self, song: &Song, read: &Reader<'_>) {
        self.load_keeping(song, song, read);
    }

    /// [`Self::load`] for an engine that plays `song`, a part of the song the app holds (`keep`: the preview solo
    /// clears every clip but one): `song` is decoded, but what `keep` plays stays too, so that switching the preview
    /// off does not decode it all again.
    pub fn load_keeping(&mut self, song: &Song, keep: &Song, read: &Reader<'_>) {
        self.load_as(song, keep, read, false);
    }

    /// [`Self::load_keeping`] for the app's own engines: a slot's `STR1` render that the cache misses streams in
    /// ([`Self::stream_slot`]) instead of rendering whole here, so that no rebuild (an undo to another ratio, a drumset
    /// or song load) renders a whole buffer on the UI thread.
    pub fn load_live(&mut self, song: &Song, keep: &Song, read: &Reader<'_>) {
        self.load_as(song, keep, read, true);
    }

    fn load_as(&mut self, song: &Song, keep: &Song, read: &Reader<'_>, live: bool) {
        let mut plan = self.refs(read);
        plan.plan = true;
        crate::audio_engine::samples::walk(keep, &mut plan);
        if !std::ptr::eq(song, keep) {
            crate::audio_engine::samples::walk(song, &mut plan);
        }
        let met = plan.met;
        // A shape with no table yet keeps what the cache has of its file (a missing mark, or a sample too empty for a
        // table), or every load would read it again. With a table the sample is no one's but the pads'.
        let tableless =
            |key: &SampleKey| met.shapes.contains(key) && !self.shapes.contains_key(key);
        self.samples
            .retain(|key, _| met.samples.contains(key) || tableless(key));
        self.waveforms.retain(|key, _| met.samples.contains(key));
        self.programs.retain(|key, _| met.programs.contains(key));
        self.soundfonts
            .retain(|key, _| met.soundfonts.contains(key));
        self.stretched.retain(|key, _| met.stretched.contains(key));
        self.streams.retain(|key, _| met.stretched.contains(key));
        self.shapes.retain(|key, _| met.shapes.contains(key));
        // The decode walk knows the keys of the whole plan: an Osc that is met before a pad on the same file leaves the
        // file to the pad.
        let mut refs = self.refs(read);
        (refs.met, refs.live) = (met, live);
        crate::audio_engine::samples::walk(song, &mut refs);
    }

    fn refs<'a>(&'a mut self, read: &'a Reader<'a>) -> Refs<'a> {
        Refs::new(self, read)
    }

    /// Decodes the samples the DW Sampler `module` plays ([`crate::audio_engine::samples::dw_module`]) that this cache
    /// has not met. It is for a worker thread, on a cache of its own that [`Self::absorb`] hands to the app's.
    pub fn load_dw(&mut self, module: &ModuleInstance, read: &Reader<'_>) {
        crate::audio_engine::samples::dw_module(module, &mut self.refs(read));
    }

    /// Decodes the program at `path`, as a DW Sampler naming it would have it loaded ([`Refs::add_program`]). Also for a
    /// worker's cache.
    pub fn load_program(&mut self, path: &ContentPath, read: &Reader<'_>) {
        self.refs(read).add_program(path);
    }

    /// Decodes the sample file `key` names ([`Refs::add`]), or with `shape` builds the 3x Osc Custom table from it
    /// ([`Refs::add_shape`]): a picked loop, shape or sample, on a worker's cache too.
    pub fn load_sample(&mut self, key: SampleKey, shape: bool, read: &Reader<'_>) {
        let mut refs = self.refs(read);
        match shape {
            true => refs.add_shape(key),
            false => refs.add(key),
        }
    }

    /// Decodes `slot`'s sample and renders its `STR1` stretch and pitch ([`Refs::add_slot`]): a picked clip sample, on a
    /// worker's cache.
    pub fn render_slot(&mut self, slot: &SamplerSlot, read: &Reader<'_>) {
        self.refs(read).add_slot(slot);
    }

    /// Takes what `worker` decoded (its samples, stretch renders, streams and Custom tables, under this cache's byte
    /// budget) and its DW programs. A pick reads its files afresh, so they replace what this cache had of the same files, and what
    /// was made of those goes ([`Self::forget`]: waveform summaries, renders, tables); the others stay. One that does not
    /// fit what is left of the budget is missing.
    pub fn absorb(&mut self, worker: Self) {
        for key in worker.samples.keys().chain(worker.shapes.keys()) {
            self.forget(key);
        }
        for (key, sample) in worker.samples {
            let sample = sample.filter(|s| pcm_bytes(s) <= self.room());
            self.samples.insert(key, sample);
        }
        for (key, render) in worker.stretched {
            let render = render.filter(|s| pcm_bytes(s) <= self.room());
            self.stretched.insert(key, render);
        }
        // A stream over the budget is a missing render, which no walk renders while its rest streams in.
        for (key, stream) in worker.streams {
            if stream.byte_len() <= self.room() {
                self.streams.insert(key, stream);
            } else {
                self.stretched.insert(key, None);
            }
        }
        // Only the jobs of the streams it took: one over the budget is missing, with nothing left to render.
        let streams = &self.streams;
        let taken =
            |job: &StretchJob| (streams.get(&job.key)).is_some_and(|s| Arc::ptr_eq(s, &job.stream));
        self.jobs.0.extend(worker.jobs.0.into_iter().filter(taken));
        for (key, table) in worker.shapes {
            let table = table.filter(|w| w.byte_len() <= self.room());
            self.shapes.insert(key, table);
        }
        self.programs.extend(worker.programs);
    }

    /// Starts the render `slot`'s `STR1` needs streaming in, from the decoded sample this cache holds for it: its first
    /// [`STREAM_FRAMES`] now, into the cache, which pads play from now on ([`Self::pad_sample`]), and the rest as a job
    /// for a worker ([`Self::take_jobs`]). Nothing when it needs none (no stretch or pitch, or one the cache already
    /// has, whole or streaming: shared), or the cache has no sample for it (missing, or not loaded yet), which a load
    /// decodes and streams in; one over the byte budget is missing.
    pub fn stream_slot(&mut self, slot: &SamplerSlot) {
        let Some(sample_key) = SampleKey::of_slot(slot) else {
            return;
        };
        let Some(key) = StretchKey::of_slot(slot, sample_key.clone()) else {
            return;
        };
        if self.stretched.contains_key(&key) || self.streams.contains_key(&key) {
            return;
        }
        let Some(sample) = self.get(&sample_key) else {
            return;
        };
        let (ratio, semitones) = str1(slot);
        let frames = stretch_frames(sample.frames(), ratio, semitones);
        if Stream::bytes_for(frames, sample.right.is_some()) > self.room() {
            self.stretched.insert(key, None);
            return;
        }
        let mut stretching = Stretching::new(&sample, ratio, semitones);
        let stream = stretching.stream().clone();
        stretching.fill(STREAM_FRAMES);
        self.streams.insert(key.clone(), stream.clone());
        self.jobs.0.push(StretchJob {
            key,
            sample,
            stream,
            stretching: Some(stretching),
            render: None,
        });
    }

    /// The renders started streaming in since the last call, for a worker each ([`StretchJob::render`]).
    pub fn take_jobs(&mut self) -> Vec<StretchJob> {
        std::mem::take(&mut self.jobs.0)
    }

    /// Takes `job`'s whole render in place of its stream, under the byte budget (a render over it is missing), when the
    /// cache still holds that stream and the very sample it renders. Only that render goes in: the file's sample and
    /// its other renders stay. Returns whether it took it. A render whose stream a load has freed (no slot plays its
    /// ratio any more: the slot moved on) or [`Self::forget`] dropped (the file was read again) is dropped.
    pub fn put_stretch(&mut self, job: StretchJob) -> bool {
        let current = (self.streams.get(&job.key)).is_some_and(|s| Arc::ptr_eq(s, &job.stream));
        let same = (self.get(&job.key.0)).is_some_and(|sample| Arc::ptr_eq(&sample, &job.sample));
        let Some(render) = job.render.filter(|_| current && same) else {
            return false;
        };
        self.streams.remove(&job.key);
        let fits = pcm_bytes(&render) <= self.room();
        self.stretched
            .insert(job.key, fits.then(|| Arc::new(render)));
        true
    }

    /// The stretches rendered so far.
    #[cfg(test)]
    pub(crate) fn renders(&self) -> usize {
        self.renders
    }

    /// The sample `key` names, `None` when it is missing or was never loaded.
    pub fn get(&self, key: &SampleKey) -> Option<Arc<SampleData>> {
        self.samples.get(key)?.clone()
    }

    /// Whether a load met the sample or DW program `key` and found it missing.
    pub(crate) fn is_missing(&self, key: &SampleKey) -> bool {
        matches!(self.samples.get(key), Some(None)) || matches!(self.programs.get(key), Some(None))
    }

    /// Some sample decoded: of a worker's cache, whether a pick will sound.
    pub fn decoded_any(&self) -> bool {
        self.samples.values().any(Option::is_some)
    }

    /// The sample `slot` plays ([`SampleKey::of_slot`]), with its `STR1` stretch and pitch rendered ([`Refs::add_slot`]).
    pub fn slot(&self, slot: &SamplerSlot) -> Option<Arc<SampleData>> {
        let key = SampleKey::of_slot(slot)?;
        match StretchKey::of_slot(slot, key.clone()) {
            Some(stretched) => self.stretched.get(&stretched).cloned().flatten(),
            None => self.get(&key),
        }
    }

    /// What a pad playing `slot` plays: [`Self::slot`], or its render still streaming in ([`StretchJob`]).
    pub fn pad_sample(&self, slot: &SamplerSlot) -> Option<PadSample> {
        if let Some(sample) = self.slot(slot) {
            return Some(PadSample::Whole(sample));
        }
        let key = StretchKey::of_slot(slot, SampleKey::of_slot(slot)?)?;
        self.streams.get(&key).cloned().map(PadSample::Streaming)
    }

    /// The DW program at `path` that a load read, `None` when it is missing or was never loaded.
    pub fn program(&self, path: &ContentPath) -> Option<DwProgram> {
        self.programs.get(&SampleKey::File(path.clone()))?.clone()
    }

    /// [`Self::slot`], decoding the slot's sample first if the cache has not met it: for a view that shows a sample
    /// no engine build loaded yet (the audio clip editor's waveform). App thread only.
    pub fn load_slot(&mut self, slot: &SamplerSlot, read: &Reader<'_>) -> Option<Arc<SampleData>> {
        let key = SampleKey::of_slot(slot)?;
        self.refs(read).add(key.clone());
        self.get(&key)
    }

    /// [`Self::load_slot`]'s sample with its waveform summary, made the first time it is asked for (whoever decoded the
    /// sample, a DW program's plain-audio fallback included): what a view drawing the sample's waveform reads (the audio
    /// clip editor's, the playlist's audio clips).
    pub fn waveform(&mut self, slot: &SamplerSlot, read: &Reader<'_>) -> Option<Waveform> {
        let data = self.load_slot(slot, read)?;
        let key = SampleKey::of_slot(slot)?;
        Some(
            (self.waveforms.entry(key))
                .or_insert_with(|| Waveform::new(data))
                .clone(),
        )
    }

    /// The SoundFont at the `SF2D` path `path`, `None` when it is missing or was never loaded.
    pub fn soundfont(&self, path: &ContentPath) -> Option<Arc<SoundFont>> {
        self.soundfonts.get(&SampleKey::File(path.clone()))?.clone()
    }

    /// The 3x Osc Custom table built from the file `key` names ([`Refs::add_shape`]), `None` when it is missing, empty,
    /// over the byte budget or was never loaded.
    pub fn shape(&self, key: &SampleKey) -> Option<Arc<Wave>> {
        self.shapes.get(key).cloned().flatten()
    }

    /// The samples, DW programs, SoundFonts and Custom shapes that failed, for the load message "Some audio files were
    /// missing". The song's own: not our stand-in for a missing sampler ([`crate::audio_engine::samples::stand_in`]).
    pub fn missing(&self) -> impl Iterator<Item = &SampleKey> {
        let samples = self.samples.iter().filter(|(_, s)| s.is_none());
        let programs = self.programs.iter().filter(|(_, p)| p.is_none());
        let fonts = self.soundfonts.iter().filter(|(_, f)| f.is_none());
        let renders = self.stretched.iter().filter(|(_, r)| r.is_none());
        let tables = self.shapes.iter().filter(|(_, w)| w.is_none());
        let stand_in = SampleKey::File(crate::audio_engine::samples::stand_in());
        (samples.map(|(key, _)| key))
            .chain(renders.map(|(key, _)| &key.0))
            .chain(tables.map(|(key, _)| key))
            .chain(programs.map(|(key, _)| key))
            .chain(fonts.map(|(key, _)| key))
            .filter(move |&key| *key != stand_in)
    }

    /// How many samples, DW programs and SoundFonts the loads met, the missing ones too: the content check's count of
    /// what a song references. An audio file a DW Sampler plays is a sample and a program of one zone: once. Our
    /// stand-in for a missing sampler and its zones are not the song's.
    #[cfg(all(test, not(target_arch = "wasm32")))]
    pub(crate) fn met(&self) -> usize {
        let stand_in = SampleKey::File(crate::audio_engine::samples::stand_in());
        let zones = self.programs.get(&stand_in).cloned().flatten();
        let ours =
            |k: &SampleKey| *k == stand_in || zones.iter().any(|z| z.iter().any(|z| z.1 == *k));
        let samples = self.samples.keys().filter(|k| !ours(k));
        let programs = (self.programs.keys()).filter(|k| !self.samples.contains_key(k) && !ours(k));
        samples.count() + programs.count() + self.soundfonts.len()
    }

    /// Drops the sample `key` names and its stretch and pitch renders, and the DW program it names, so that the next
    /// load reads, decodes and renders it again: its file was rewritten (the audio editor's save), or a
    /// bundled lite program has its full one now.
    pub fn forget(&mut self, key: &SampleKey) {
        self.samples.remove(key);
        self.programs.remove(key);
        self.shapes.remove(key);
        self.waveforms.remove(key);
        self.stretched.retain(|k, _| k.0 != *key);
        self.streams.retain(|k, _| k.0 != *key);
    }

    /// The keys `song` plays, nothing read or decoded: [`Self::load`]'s plan walk. A program's zones are not among
    /// them (its file is not read); its own key is. For the web loader, which fetches what a song needs first.
    pub fn referenced(song: &Song) -> Vec<SampleKey> {
        let (mut cache, read) = (Self::default(), |_: &ContentPath| None);
        let mut plan = cache.refs(&read);
        plan.plan = true;
        crate::audio_engine::samples::walk(song, &mut plan);
        let met = plan.met;
        (met.samples.into_iter())
            .chain(met.programs)
            .chain(met.soundfonts)
            .chain(met.shapes)
            .collect()
    }

    /// Lets the next load try the missing samples, stretch renders, Custom tables, DW programs and SoundFonts again: a
    /// song load, or a new pick. A program with a missing embedded zone is read again.
    pub fn forget_missing(&mut self) {
        self.samples.retain(|_, sample| sample.is_some());
        let samples = &self.samples;
        let whole = |zones: &DwProgram| {
            let embedded = zones
                .iter()
                .filter(|(_, key)| matches!(key, SampleKey::Embedded(..)));
            embedded
                .into_iter()
                .all(|(_, key)| samples.contains_key(key))
        };
        self.programs
            .retain(|_, program| program.as_ref().is_some_and(whole));
        self.soundfonts.retain(|_, font| font.is_some());
        self.stretched.retain(|_, render| render.is_some());
        self.shapes.retain(|_, table| table.is_some());
    }
}

/// The keys a reference walk met: what the song plays, and so what [`SampleCache::load`] keeps.
#[derive(Default)]
struct Met {
    /// Every sample the song plays (the pads', a program's zones), not the files that only a 3x Osc's shape reads.
    samples: HashSet<SampleKey>,
    programs: HashSet<SampleKey>,
    soundfonts: HashSet<SampleKey>,
    stretched: HashSet<StretchKey>,
    shapes: HashSet<SampleKey>,
}

/// What the reference walk hands a song's samples to, during [`SampleCache::load`].
pub struct Refs<'a> {
    cache: &'a mut SampleCache,
    read: &'a Reader<'a>,
    /// The keys the walk met.
    met: Met,
    /// The walk only names the keys ([`SampleCache::load`]'s first): nothing is read or decoded.
    plan: bool,
    /// A live load ([`SampleCache::load_live`]): a missing `STR1` render streams in.
    live: bool,
}

impl<'a> Refs<'a> {
    fn new(cache: &'a mut SampleCache, read: &'a Reader<'a>) -> Self {
        Self {
            cache,
            read,
            met: Met::default(),
            plan: false,
            live: false,
        }
    }
}

impl Refs<'_> {
    /// The cache as loaded so far.
    pub(crate) fn cache(&self) -> &SampleCache {
        self.cache
    }

    /// The song plays `key`. It is decoded now unless the cache already has it, or knows it is missing. A sample
    /// that does not fit what is left of the cache's byte budget ([`MAX_DECODED_BYTES`]) is missing.
    pub fn add(&mut self, key: SampleKey) {
        self.met.samples.insert(key.clone());
        if self.plan || self.cache.samples.contains_key(&key) {
            return;
        }
        let decoded = |path: &ContentPath| self.read(path).and_then(|bytes| decode(&bytes).ok());
        let sample = match &key {
            // Our own `@R@Built-in/…` content first: code-made, else bundled (the reader); no content folder shadows it.
            SampleKey::File(path) => match SampleKey::of_path(path) {
                SampleKey::Builtin(folder) => builtin(&folder).or_else(|| decoded(path)),
                _ => decoded(path),
            },
            // ponytail: bundled audio is always `.flac`, as `of_path` drops the extension; a name table of the
            // code-made ones would let `of_path` keep it.
            SampleKey::Builtin(folder) => builtin(folder).or_else(|| {
                decoded(&ContentPath::Shorthand {
                    root: ContentRoot::App,
                    relative: format!("{folder}.flac"),
                })
            }),
            // Decoded with its program (`add_program`); met alone, it is missing.
            SampleKey::Embedded(..) => None,
        };
        let sample = sample.filter(|s| pcm_bytes(s) <= self.cache.room());
        self.cache.samples.insert(key, sample.map(Arc::new));
    }

    /// The song plays the DW program at `path`: a `.dwp`, or a plain audio file played as a user sample (the program
    /// loader). It is read and parsed once, and a `.dwp`'s embedded zones are decoded then, all of them together within
    /// `MAX_FRAMES` frames and the cache's byte budget: a zone over what is left is missing. Each zone file,
    /// `<folder of .dwp>/<.dwp base name>/<file name>`, is added every time.
    pub fn add_program(&mut self, path: &ContentPath) {
        let key = SampleKey::File(path.clone());
        self.met.programs.insert(key.clone());
        if !self.plan && !self.cache.programs.contains_key(&key) {
            // A frame is at most 8 bytes (stereo `f32`).
            let room = self.cache.room();
            let mut budget = MAX_FRAMES.min(room / 8);
            let program = self.read(path).map(|bytes| {
                let Some(zones) = dwp::read(&bytes) else {
                    // The loader's last try: a plain audio file, one user-sample zone. Bytes that do not decode
                    // leave that zone missing.
                    let decoded = || {
                        let sample = decode(&bytes).ok();
                        sample.filter(|s| pcm_bytes(s) <= room).map(Arc::new)
                    };
                    self.cache
                        .samples
                        .entry(key.clone())
                        .or_insert_with(decoded);
                    return Arc::from([(Zone::user_sample(), key.clone())]);
                };
                // Embedded zones decode across the keyboard: every key range's most played velocity layer, then
                // the next layer of each, and so on ([`decode_order`]). One whose size is over what is left is left
                // out, and plays the nearest decoded zone's sample on its own keys and velocities: a program bigger
                // than the budget plays every key, with fewer layers. One that does not decode is missing.
                let mut decoded = vec![false; zones.len()];
                let mut left_out = Vec::new();
                for index in decode_order(&zones) {
                    let zone = &zones[index];
                    let key = SampleKey::Embedded(path.clone(), index as u32);
                    if !self.cache.samples.contains_key(&key)
                        && zone.declared_frames() > budget as u64
                    {
                        left_out.push(key);
                        continue;
                    }
                    let decoded_zone = || zone.decode_with_limit(budget).map(Arc::new);
                    let sample = self.cache.samples.entry(key).or_insert_with(decoded_zone);
                    // A zone cached from an older version of the file may be larger than what is left.
                    budget =
                        budget.saturating_sub(sample.as_ref().map_or(0, |sample| sample.frames()));
                    decoded[index] = sample.is_some();
                }
                // With none decoded there is nothing to stand in: those left out are missing.
                if !decoded.contains(&true) {
                    for key in left_out.drain(..) {
                        self.cache.samples.insert(key, None);
                    }
                }
                let zones = zones.iter().enumerate().filter_map(|(index, zone)| {
                    if !zone.embedded() {
                        let file = SampleKey::File(zone_file(path, zone.file.as_deref()?));
                        return Some((zone.zone, file));
                    }
                    let key = |at: usize| SampleKey::Embedded(path.clone(), at as u32);
                    match left_out.contains(&key(index)) {
                        true => {
                            let at = nearest(&zones, &decoded, index)?;
                            let ranges = &zone.zone;
                            let stand_in = Zone {
                                key_low: ranges.key_low,
                                key_high: ranges.key_high,
                                vel_low: ranges.vel_low,
                                vel_high: ranges.vel_high,
                                ..zones[at].zone
                            };
                            Some((stand_in, key(at)))
                        }
                        false => Some((zone.zone, key(index))),
                    }
                });
                zones.collect::<DwProgram>()
            });
            self.cache.programs.insert(key.clone(), program);
        }
        let program = self.cache.programs.get(&key).cloned().flatten();
        for (_, sample) in program.iter().flat_map(|zones| zones.iter()) {
            match sample {
                SampleKey::File(_) => self.add(sample.clone()),
                // Decoded with the program.
                _ => {
                    self.met.samples.insert(sample.clone());
                }
            }
        }
    }

    /// The song plays sampler slot `slot`: its sample ([`Self::add`]), and that sample with the slot's `STR1` stretch
    /// and pitch, rendered unless the cache has it (once, at load). A live load streams it in
    /// ([`SampleCache::stream_slot`]) and leaves a stream to its worker; any other (an export, a worker's cache) needs
    /// it whole now, a stream not yet filled too.
    pub fn add_slot(&mut self, slot: &SamplerSlot) {
        let Some(key) = SampleKey::of_slot(slot) else {
            return;
        };
        self.add(key.clone());
        let Some(stretched) = StretchKey::of_slot(slot, key.clone()) else {
            return;
        };
        self.met.stretched.insert(stretched.clone());
        if self.plan || self.cache.stretched.contains_key(&stretched) {
            return;
        }
        let filling = (self.cache.streams.get(&stretched)).map(|s| s.filled() < s.frames());
        match (filling, self.live) {
            (Some(false), _) | (Some(true), true) => return,
            (None, true) => return self.cache.stream_slot(slot),
            (_, false) => {}
        }
        let Some(sample) = self.cache.get(&key) else {
            return;
        };
        let (ratio, semitones) = str1(slot);
        let rendered = stretch(&sample, ratio, semitones);
        self.cache.streams.remove(&stretched);
        let fits = pcm_bytes(&rendered) <= self.cache.room();
        (self.cache.stretched).insert(stretched, fits.then(|| Arc::new(rendered)));
        self.cache.renders += 1;
    }

    /// The song plays `key` as a 3x Osc's Custom shape: its table is built once ([`Wave::custom`]: a resample and FFTs)
    /// unless the cache has it. Only the table stays: the decoded sample is dropped then, unless another source met
    /// it first, even a pad walked later (the plan's keys). A file that is missing stays missing ([`Self::add`]), and so
    /// does a table that does not fit what is left of the cache's byte budget ([`MAX_DECODED_BYTES`], with the file's
    /// bytes free when the table replaces it): the Osc plays silence, and nothing is built for it.
    pub fn add_shape(&mut self, key: SampleKey) {
        self.met.shapes.insert(key.clone());
        if self.plan || self.cache.shapes.contains_key(&key) {
            return;
        }
        // Another source of the file (a pad, even one walked later) keeps its sample.
        let shared = self.cache.samples.contains_key(&key) || self.met.samples.contains(&key);
        self.add(key.clone());
        let Some(sample) = self.cache.get(&key) else {
            return;
        };
        // The table replaces the file, so its bytes are free for it. A table that cannot fit is not built.
        let freed = if shared { 0 } else { pcm_bytes(&sample) };
        let fits = TABLE_BYTES <= self.cache.room() + freed;
        let wave = if fits { Wave::custom(&sample) } else { None };
        if fits && wave.is_none() {
            // Too empty for a table: the sample stays, so that the file is not read again.
            return;
        }
        if !shared {
            self.cache.samples.remove(&key);
        }
        self.cache.builds += usize::from(fits);
        self.cache.shapes.insert(key, wave.map(Arc::new));
    }

    /// The song plays the SoundFont at the `SF2D` path `path`. It is read and parsed now unless the cache already has
    /// it, or knows it is missing. Only a user-folder or content-folder path that stays inside it is read
    /// ([`crate::project_files::ProjectFiles::library_path`]'s rule); any other is missing.
    pub fn add_soundfont(&mut self, path: ContentPath) {
        let key = SampleKey::File(path.clone());
        self.met.soundfonts.insert(key.clone());
        if self.plan || self.cache.soundfonts.contains_key(&key) {
            return;
        }
        let bytes = in_library(&path).and_then(|_| self.read(&path));
        // The file stays in memory as the SoundFont's samples: it counts against the budget like decoded PCM.
        let bytes = bytes.filter(|bytes| bytes.len() <= self.cache.room());
        let font = bytes.and_then(|bytes| SoundFont::parse(bytes).ok());
        self.cache.soundfonts.insert(key, font.map(Arc::new));
    }

    /// The bytes of the file `path` names, uncached. It is for a source that reads a file to find its samples, such
    /// as a `.dwp`'s zones.
    pub fn read(&self, path: &ContentPath) -> Option<Vec<u8>> {
        (self.read)(path)
    }
}

/// A [`Reader`] a worker thread can own.
pub type WorkerReader = Box<dyn Fn(&ContentPath) -> Option<Vec<u8>> + Send>;

/// [`reader`] for a worker thread, where the files are on disk ([`ProjectFiles::disk_user_dir`]); `None` where they are
/// not (the web, which has no threads either, or a session that keeps copies): the caller does without, and the engine
/// build reads them as before.
pub fn disk_reader(files: &ProjectFiles) -> Option<WorkerReader> {
    let user = files.disk_user_dir()?.to_owned();
    let content = files.content_dir();
    let factory = files.factory;
    Some(Box::new(move |path| {
        if built_in(path).is_some() {
            return bundled(factory, path);
        }
        let path = resolve_in(Some(&user), content.as_deref(), path)?;
        let file = (!special(&path)).then(|| std::fs::File::open(path).ok())??;
        // One that says it is longer fails at once, as the library's read does (`read_file_limited`), and one that
        // turns out longer at the cap.
        if file.metadata().is_ok_and(|m| m.len() > MAX_FILE_BYTES) {
            return None;
        }
        read_limited(file, MAX_FILE_BYTES).ok()
    }))
}

/// `.dwp` zone's sample file: `<folder of .dwp>/<.dwp base name>/<name>`.
/// The order a program's embedded zones decode in: by layer (each key range's zones nearest velocity 100 first), then
/// by key, then in file order. File zones are not in it.
fn decode_order(zones: &[dwp::DwpZone<'_>]) -> Vec<usize> {
    let distance = |z: &Zone| {
        (i32::from(z.vel_low) - 100)
            .max(100 - i32::from(z.vel_high))
            .max(0)
    };
    let range = |i: usize| (zones[i].zone.key_low, zones[i].zone.key_high);
    let mut by_range: Vec<usize> = (0..zones.len()).filter(|&i| zones[i].embedded()).collect();
    by_range.sort_by_key(|&i| (range(i), distance(&zones[i].zone), i));
    let mut layered = Vec::with_capacity(by_range.len());
    for (n, &i) in by_range.iter().enumerate() {
        let layer = match n.checked_sub(1).map(|p| by_range[p]) {
            Some(p) if range(p) == range(i) => layered.last().map_or(0, |&(l, _, _)| l + 1),
            _ => 0,
        };
        layered.push((layer, range(i), i));
    }
    layered.sort();
    layered.into_iter().map(|(_, _, i)| i).collect()
}

/// The decoded zone nearest zone `index`: overlapping keys or the smallest gap between them first, then the nearest
/// velocities, then the nearest root.
fn nearest(zones: &[dwp::DwpZone<'_>], decoded: &[bool], index: usize) -> Option<usize> {
    let z = &zones[index].zone;
    let gap = |a: u8, b: u8, c: u8, d: u8| {
        i32::from(c.saturating_sub(b)).max(i32::from(a.saturating_sub(d)))
    };
    (0..zones.len()).filter(|&i| decoded[i]).min_by_key(|&i| {
        let o = &zones[i].zone;
        (
            gap(z.key_low, z.key_high, o.key_low, o.key_high),
            gap(z.vel_low, z.vel_high, o.vel_low, o.vel_high),
            (i32::from(z.root) - i32::from(o.root)).abs(),
        )
    })
}

fn zone_file(program: &ContentPath, name: &str) -> ContentPath {
    let beside = |path: &str| {
        let base = path
            .rsplit_once('.')
            .filter(|(_, extension)| !extension.contains('/'));
        format!("{}/{name}", base.map_or(path, |(base, _)| base))
    };
    match program {
        ContentPath::Shorthand { root, relative } => ContentPath::Shorthand {
            root: *root,
            relative: beside(relative),
        },
        ContentPath::Plain(path) => ContentPath::Plain(beside(path)),
    }
}

/// The app's [`Reader`]: the file `path` names ([`ProjectFiles::resolve`]: `@L@`, `@R@`, `@T@`) through `files` (from
/// disk on the desktop, from this session's copies on the web), up to [`crate::project_files::MAX_FILE_BYTES`]. Only
/// a regular file is read.
pub fn reader(files: &ProjectFiles) -> impl Fn(&ContentPath) -> Option<Vec<u8>> + '_ {
    |path| {
        if built_in(path).is_some() {
            return bundled(files.factory, path);
        }
        let path = files.resolve(path)?;
        (!special(&path)).then(|| files.read_user_file(&path).ok())?
    }
}

/// The bundled member our own `@R@Built-in/<rel>` names (either separator), `None` without one: no bundle, or no such
/// member ([`crate::content::Bundle::read`]).
fn bundled(factory: &crate::content::Factory, path: &ContentPath) -> Option<Vec<u8>> {
    let rel = built_in(path)?.replace('\\', "/");
    factory
        .bundle
        .as_ref()?
        .read(rel.strip_prefix(BUILT_IN)?.strip_prefix('/')?)
}

/// `path` is on disk and is no regular file: a folder, a FIFO or a device, which would block or never end. A path
/// with no metadata is not special: the web's copies have none.
fn special(path: &Path) -> bool {
    path.metadata().is_ok_and(|m| !m.is_file())
}

#[cfg(test)]
pub(crate) mod tests {
    use std::cell::Cell;
    use std::path::Path;
    use std::sync::Arc;

    use super::*;
    use crate::audio_engine::SongAudioEngine;
    use crate::flm::chunk::{ContentPath, ContentRoot};
    use crate::project_files::ProjectFiles;
    use crate::project_files::mem::MemFiles;
    use crate::song::{ChannelKind, SamplerSlot, Song};
    use honeydew_audio::builtin_drums::builtin;

    fn user(relative: &str) -> ContentPath {
        ContentPath::Shorthand {
            root: ContentRoot::User,
            relative: relative.into(),
        }
    }

    fn app(relative: &str) -> ContentPath {
        ContentPath::Shorthand {
            root: ContentRoot::App,
            relative: relative.into(),
        }
    }

    /// A 16-bit mono PCM WAV at 44.1 kHz.
    pub(crate) fn wav(samples: &[i16]) -> Vec<u8> {
        let data: Vec<u8> = samples.iter().flat_map(|s| s.to_le_bytes()).collect();
        let mut bytes = b"RIFF".to_vec();
        bytes.extend((36 + data.len() as u32).to_le_bytes());
        bytes.extend(b"WAVEfmt ");
        bytes.extend(16u32.to_le_bytes());
        bytes.extend(1u16.to_le_bytes()); // PCM
        bytes.extend(1u16.to_le_bytes()); // mono
        bytes.extend(44_100u32.to_le_bytes());
        bytes.extend(88_200u32.to_le_bytes());
        bytes.extend(2u16.to_le_bytes());
        bytes.extend(16u16.to_le_bytes());
        bytes.extend(b"data");
        bytes.extend((data.len() as u32).to_le_bytes());
        bytes.extend(data);
        bytes
    }

    /// A song whose drum kit's first pad plays `path`; the other pads keep their built-in one-shots.
    fn song_with(path: ContentPath) -> Song {
        let mut song = Song::default();
        song.add_channel(ChannelKind::Drums);
        song.channels[1].drum_kit.as_mut().unwrap().slots[0]
            .file
            .path = Some(path);
        song
    }

    fn kit(song: &Song) -> &[SamplerSlot] {
        &song.channels[1].drum_kit.as_ref().unwrap().slots
    }

    /// `song` with a 3x Osc channel appended, its `CSTM` `PATH` on `path`.
    fn with_custom_shape(mut song: Song, path: &ContentPath) -> Song {
        song.add_channel(ChannelKind::Synth(crate::song::SynthKind::ThreeXOsc));
        let module = &mut song.channels.last_mut().unwrap().rack.modules[0];
        let shape = Path::new("shape.wav");
        crate::project_files::load_pick(module, shape, &[], Some(path.clone())).unwrap();
        song
    }

    #[test]
    fn a_file_a_pad_met_first_keeps_its_sample_beside_the_custom_table() {
        let path = user("My Samples/wave.wav");
        let key = SampleKey::File(path.clone());
        // The pad's channel (1) is walked before the 3x Osc's (2).
        let song = with_custom_shape(song_with(path.clone()), &path);
        let read = |p: &ContentPath| (*p == path).then(|| wav(&[0, 9000, 0, -9000]));
        let mut cache = SampleCache::default();
        cache.load(&song, &read);
        assert!(cache.get(&key).is_some(), "the pad plays it");
        assert!(cache.shape(&key).is_some());
    }

    #[test]
    fn forget_drops_a_custom_table_and_the_next_load_builds_it_again() {
        let path = user("My Samples/wave.wav");
        let key = SampleKey::File(path.clone());
        let song = with_custom_shape(Song::default(), &path);
        let read = |p: &ContentPath| (*p == path).then(|| wav(&[0, 9000, 0, -9000]));
        let mut cache = SampleCache::default();
        cache.load(&song, &read);
        let first = cache.shape(&key).expect("built at load");
        cache.forget(&key);
        assert!(cache.shape(&key).is_none());
        cache.load(&song, &read);
        assert!(!Arc::ptr_eq(&first, &cache.shape(&key).unwrap()));
    }

    #[test]
    fn two_loads_decode_once() {
        let song = song_with(user("My Samples/kick.wav"));
        let reads = Cell::new(0);
        let read = |path: &ContentPath| {
            reads.set(reads.get() + 1);
            (*path == user("My Samples/kick.wav")).then(|| wav(&[0, 8000, -8000, 0]))
        };
        let mut cache = SampleCache::default();
        cache.load(&song, &read);
        let kick = cache.slot(&kit(&song)[0]).expect("the kick decodes");
        assert_eq!(kick.frames(), 4);
        let snare = cache.slot(&kit(&song)[1]).expect("the built-in snare");
        cache.load(&song, &read);
        assert_eq!(reads.get(), 1, "the second load read the file again");
        assert!(Arc::ptr_eq(&kick, &cache.slot(&kit(&song)[0]).unwrap()));
        assert!(Arc::ptr_eq(&snare, &cache.slot(&kit(&song)[1]).unwrap()));
    }

    #[test]
    fn a_decoded_sample_comes_with_its_waveform_summary_made_once() {
        let song = song_with(user("My Samples/kick.wav"));
        let frames: Vec<i16> = (0..600).map(|i| (i % 7) * 1000 - 3000).collect();
        let reads = Cell::new(0);
        let read = |path: &ContentPath| {
            reads.set(reads.get() + 1);
            (*path == user("My Samples/kick.wav")).then(|| wav(&frames))
        };
        let mut cache = SampleCache::default();
        let slot = &kit(&song)[0];
        let w = cache.waveform(slot, &read).expect("the kick decodes");
        // Mono, 600 frames: three pairs of 256 frames, the last over 88.
        assert_eq!(w.summary.len(), 1);
        assert_eq!(w.summary[0].len(), 3);
        assert_eq!(w.summary[0], summarize(&w.data.left));
        let again = cache.waveform(slot, &read).unwrap();
        assert!(Arc::ptr_eq(&w.summary, &again.summary));
        assert_eq!(reads.get(), 1);
        // A forgotten sample (its file rewritten) is decoded and summarized again.
        cache.forget(&SampleKey::of_slot(slot).unwrap());
        let fresh = cache.waveform(slot, &read).unwrap();
        assert!(!Arc::ptr_eq(&w.summary, &fresh.summary));
    }

    #[test]
    fn a_slots_stretch_renders_once_per_setting() {
        let mut song = Song::default();
        song.add_channel(ChannelKind::Drums);
        let slot = &mut song.channels[1].drum_kit.as_mut().unwrap().slots[0];
        (slot.file.stretch_mode, slot.file.stretch_ratio) = (1, 2.0);
        let mut cache = SampleCache::default();
        cache.load(&song, &|_| None);
        assert_eq!(cache.renders, 1);
        let kick = builtin("Built-in/Kick").unwrap();
        let stretched = cache.slot(&kit(&song)[0]).unwrap();
        assert_eq!(stretched.frames(), 2 * kick.frames());
        // The next engine build loads the same song again: nothing renders.
        cache.load(&song, &|_| None);
        assert_eq!(cache.renders, 1);
        assert!(Arc::ptr_eq(
            &stretched,
            &cache.slot(&kit(&song)[0]).unwrap()
        ));
        // A new setting renders once more; mode 0 ignores the ratio, and with no semitones plays the plain sample.
        let slot = &mut song.channels[1].drum_kit.as_mut().unwrap().slots[0];
        slot.file.pitch_semitones = 12.0;
        cache.load(&song, &|_| None);
        assert_eq!(cache.renders, 2);
        let slot = &mut song.channels[1].drum_kit.as_mut().unwrap().slots[0];
        slot.file.stretch_mode = 0;
        cache.load(&song, &|_| None);
        let pitched = cache.slot(&kit(&song)[0]).unwrap();
        assert_eq!(
            pitched.frames(),
            kick.frames(),
            "mode 0: pitch only, the ratio unused"
        );
        let slot = &mut song.channels[1].drum_kit.as_mut().unwrap().slots[0];
        (slot.file.stretch_mode, slot.file.pitch_semitones) = (0, 0.0);
        cache.load(&song, &|_| None);
        assert_eq!(cache.renders, 3, "the plain sample needs no render");
        assert_eq!(cache.slot(&kit(&song)[0]).unwrap().frames(), kick.frames());
        assert!(
            cache.stretched.is_empty(),
            "renders the song no longer uses are freed"
        );
    }

    #[test]
    fn a_missing_file_is_a_silent_slot_and_waits_for_a_new_load() {
        let song = song_with(user("My Samples/gone.wav"));
        let (there, reads) = (Cell::new(false), Cell::new(0));
        let read = |_: &ContentPath| {
            reads.set(reads.get() + 1);
            there.get().then(|| wav(&[1000; 8]))
        };
        let mut cache = SampleCache::default();
        cache.load(&song, &read);
        assert_eq!(cache.slot(&kit(&song)[0]), None);
        let gone = SampleKey::File(user("My Samples/gone.wav"));
        assert_eq!(cache.missing().collect::<Vec<_>>(), [&gone]);
        // No error: the engine builds with the slot silent.
        assert!(SongAudioEngine::with_samples(&song, 48_000.0, &cache).is_ok());

        // Not retried on every rebuild, even once the file is there.
        there.set(true);
        cache.load(&song, &read);
        assert_eq!(reads.get(), 1);
        assert_eq!(cache.slot(&kit(&song)[0]), None);
        // A song load (or a new pick) forgets what was missing.
        cache.forget_missing();
        cache.load(&song, &read);
        assert!(cache.slot(&kit(&song)[0]).is_some());
        assert_eq!(cache.missing().count(), 0);
    }

    #[test]
    fn a_slot_plays_its_file_else_its_built_in_one_shot_else_nothing() {
        let mut song = song_with(user("My Samples/not audio.wav"));
        let slots = &mut song.channels[1].drum_kit.as_mut().unwrap().slots;
        slots[2].folder = "Built-in\\Clap".into();
        slots[3].folder = "Drum Samples/Hat".into();
        let mut cache = SampleCache::default();
        cache.load(&song, &|_| Some(b"RIFF\x04\0\0\0junk".to_vec()));
        let kit = kit(&song);
        // A path wins over a Built-in folder; bytes that do not decode are missing.
        let file = SampleKey::File(user("My Samples/not audio.wav"));
        assert_eq!(SampleKey::of_slot(&kit[0]), Some(file.clone()));
        assert_eq!(cache.slot(&kit[0]), None);
        assert_eq!(cache.missing().collect::<Vec<_>>(), [&file]);
        // Without a path, `Built-in/<Pad>` (either separator) is the built-in one-shot.
        assert_eq!(cache.slot(&kit[1]), builtin("Built-in/Snare").map(Arc::new));
        let clap = SampleKey::Builtin("Built-in/Clap".into());
        assert_eq!(SampleKey::of_slot(&kit[2]), Some(clap));
        assert!(cache.slot(&kit[2]).is_some());
        // Any other folder without a path has no sample, which is not missing.
        assert_eq!(SampleKey::of_slot(&kit[3]), None);
        assert_eq!(cache.slot(&kit[3]), None);
    }

    const SINE_SF2: &[u8] = include_bytes!("../crates/audio/tests/fixtures/sine.sf2");

    /// Two SoundFont Player channels whose `SF2D`s name `path`.
    fn soundfont_song(path: ContentPath) -> Song {
        soundfonts_song(&[path.clone(), path])
    }

    /// One SoundFont Player channel per path, whose `SF2D` names it.
    fn soundfonts_song(paths: &[ContentPath]) -> Song {
        let mut song = Song::default();
        for path in paths {
            song.add_channel(ChannelKind::Synth(crate::song::SynthKind::SoundFontPlayer));
            let sf2d = crate::flm::sf2d::Sf2d {
                path: path.clone(),
                patch: 0,
            };
            let module = &mut song.channels.last_mut().unwrap().rack.modules[0];
            module.file.custom_data = Some(sf2d.write(None).unwrap());
        }
        song
    }

    #[test]
    fn a_soundfont_is_read_and_parsed_once_and_shared() {
        let path = user("My SoundFonts/sine.sf2");
        let song = soundfont_song(path.clone());
        let reads = Cell::new(0);
        let read = |p: &ContentPath| {
            reads.set(reads.get() + 1);
            (*p == user("My SoundFonts/sine.sf2")).then(|| SINE_SF2.to_vec())
        };
        let mut cache = SampleCache::default();
        cache.load(&song, &read);
        cache.load(&song, &read);
        assert_eq!(reads.get(), 1);
        let font = cache.soundfont(&path).expect("the SoundFont parses");
        assert_eq!(font.presets().len(), 1);
        assert!(Arc::ptr_eq(&font, &cache.soundfont(&path).unwrap()));
        assert_eq!(cache.missing().count(), 0);
    }

    #[test]
    fn only_the_songs_soundfonts_stay_cached() {
        // Picking through several SoundFonts keeps only the one the song plays now.
        let (a, b) = (user("My SoundFonts/a.sf2"), user("My SoundFonts/b.sf2"));
        let read = |_: &ContentPath| Some(SINE_SF2.to_vec());
        let mut cache = SampleCache::default();
        cache.load(&soundfont_song(a.clone()), &read);
        let first = Arc::downgrade(&cache.soundfont(&a).unwrap());
        cache.load(&soundfont_song(b.clone()), &read);
        assert!(cache.soundfont(&a).is_none());
        assert!(first.upgrade().is_none(), "the first SoundFont was kept");
        assert!(cache.soundfont(&b).is_some());
    }

    #[test]
    fn a_soundfont_path_that_leaves_the_user_folder_is_never_read() {
        let escapes = [
            user("../Other/sine.sf2"),
            user("My SoundFonts/../../sine.sf2"),
            user("/home/u/sine.sf2"),
            app("../sine.sf2"),
            ContentPath::Plain("/tmp/sine.sf2".into()),
        ];
        for path in escapes {
            let mut cache = SampleCache::default();
            cache.load(&soundfont_song(path.clone()), &|_| {
                panic!("{path:?} was read")
            });
            assert!(cache.soundfont(&path).is_none());
            let missing = SampleKey::File(path.clone());
            assert_eq!(cache.missing().collect::<Vec<_>>(), [&missing]);
        }
    }

    #[test]
    fn a_soundfont_that_does_not_parse_is_missing_until_forgotten() {
        let path = user("My SoundFonts/sine.sf2");
        let song = soundfont_song(path.clone());
        let good = Cell::new(false);
        let read = |_: &ContentPath| {
            Some(if good.get() {
                SINE_SF2.to_vec()
            } else {
                b"RIFF".to_vec()
            })
        };
        let mut cache = SampleCache::default();
        cache.load(&song, &read);
        assert!(cache.soundfont(&path).is_none());
        assert_eq!(cache.missing().count(), 1);
        good.set(true);
        cache.load(&song, &read);
        assert!(cache.soundfont(&path).is_none(), "not retried on a rebuild");
        cache.forget_missing();
        cache.load(&song, &read);
        assert!(cache.soundfont(&path).is_some());
    }

    #[test]
    fn the_reader_reads_user_files_through_project_files() {
        // An in-memory provider keeps the user files in memory, as the web does.
        let mut files = ProjectFiles::with_provider(Box::new(MemFiles::default()));
        files.keep_user_file(Path::new("My Samples/kick.wav"), wav(&[1, 2]));
        let read = reader(&files);
        assert_eq!(read(&user("My Samples/kick.wav")), Some(wav(&[1, 2])));
        assert_eq!(read(&user("My Samples/snare.wav")), None);
        // No original content ships, and a plain path is no sample (PTH1 holds shorthands only).
        assert_eq!(read(&app("My Samples/kick.wav")), None);
        assert_eq!(
            read(&ContentPath::Plain("My Samples/kick.wav".into())),
            None
        );
    }

    #[cfg(unix)]
    #[test]
    fn only_regular_files_are_read() {
        // A FIFO or a device would block or never end. (No metadata is no reason to refuse: the web's copies, read
        // in `the_reader_reads_user_files_through_project_files`.)
        let dir = Removed(
            std::env::temp_dir().join(format!("honeydew-e7-special-{}", std::process::id())),
        );
        let root = &dir.0;
        // What a run that died before its guard ran left behind.
        let _ = std::fs::remove_dir_all(root);
        std::fs::create_dir_all(root.join("Kicks")).unwrap();
        std::fs::write(root.join("kick.wav"), wav(&[3])).unwrap();
        // A device that reads as empty: without the check it would read as a file of no bytes.
        std::os::unix::fs::symlink("/dev/null", root.join("null.wav")).unwrap();
        let files = ProjectFiles::on_disk(root);
        let read = reader(&files);
        assert_eq!(read(&user("kick.wav")), Some(wav(&[3])));
        assert_eq!(read(&user("gone.wav")), None);
        assert_eq!(read(&user("Kicks")), None);
        assert_eq!(read(&user("null.wav")), None);
    }

    #[test]
    fn content_paths_resolve_inside_their_root_only() {
        let root = Path::new("/home/u/Studio");
        let files = ProjectFiles::on_disk(root);
        let at = |path: ContentPath| files.resolve(&path);
        assert_eq!(
            at(user("My Samples/a.wav")),
            Some(root.join("My Samples/a.wav"))
        );
        // `@R@` is the content folder's: here there is no such file (`project_files::content` has the rest).
        assert_eq!(at(app("Drum Samples/Kick/a.wav")), None);
        let temp = |relative: &str| ContentPath::Shorthand {
            root: ContentRoot::Temp,
            relative: relative.into(),
        };
        assert_eq!(
            at(temp("rec.wav")),
            Some(std::env::temp_dir().join("rec.wav"))
        );
        // `Path::components` drops a `.` after the first part: no way out.
        assert_eq!(
            at(user("My Samples/./a.wav")),
            Some(root.join("My Samples/a.wav"))
        );
        // Nothing may leave its root: an absolute path, a `..`, an inner `..`, a leading `./`; and a plain path is no
        // sample.
        for escape in [
            "/etc/a.wav",
            "../a.wav",
            "My Samples/../../a.wav",
            "./a.wav",
            "",
        ] {
            assert_eq!(at(user(escape)), None, "@L@ {escape:?}");
            assert_eq!(at(temp(escape)), None, "@T@ {escape:?}");
        }
        assert_eq!(at(ContentPath::Plain("/data/a.wav".into())), None);
        assert_eq!(at(ContentPath::Plain("a.wav".into())), None);
    }

    /// A temp folder, removed with its guard.
    struct Removed(std::path::PathBuf);

    impl Drop for Removed {
        fn drop(&mut self) {
            let _ = std::fs::remove_dir_all(&self.0);
        }
    }

    pub(crate) fn record(tag: u32, data: &[u8]) -> Vec<u8> {
        [
            &tag.to_le_bytes()[..],
            &(data.len() as u64).to_le_bytes(),
            data,
        ]
        .concat()
    }

    /// `.dwp` whose zones are these records' lists.
    pub(crate) fn dwp(zones: &[Vec<u8>]) -> Vec<u8> {
        let zones: Vec<u8> = zones.iter().flat_map(|zone| record(3, zone)).collect();
        [&b"DwPr"[..], &38u32.to_le_bytes(), &record(1, &zones)].concat()
    }

    /// A DW Sampler's `CSTM` naming the program `path`.
    pub(crate) fn dw_cstm(path: &[u8]) -> Vec<u8> {
        let payload = [&(path.len() as u32).to_le_bytes()[..], path, &[0; 8]].concat();
        [
            &b"10WDPATh"[..],
            &(payload.len() as u32).to_le_bytes(),
            &payload,
        ]
        .concat()
    }

    /// `Keys.dwp`: a zone with a file, one that embeds its audio (4 frames) and one whose file is missing.
    fn keys_program() -> Vec<u8> {
        let file_zone = record(0x1F6, b"%ILSharedData%\\Keys\\Keys C4.wav");
        let pcm: Vec<u8> = [0i16, 9000, -9000, 0]
            .iter()
            .flat_map(|s| s.to_le_bytes())
            .collect();
        let mut format = 4u64.to_le_bytes().to_vec();
        format.extend(1u32.to_le_bytes());
        format.extend(16u32.to_le_bytes());
        format.extend(44_100f32.to_le_bytes());
        let embedded = [record(0x1F7, &format), record(0x205, &pcm)].concat();
        dwp(&[file_zone, embedded, record(0x1F6, b"Gone.wav")])
    }

    fn dw_song(cstm: Vec<u8>) -> Song {
        let mut song = Song::default();
        song.add_channel(ChannelKind::Synth(crate::song::SynthKind::DwSampler));
        song.channels[1].rack.modules[0].file.custom_data = Some(cstm);
        song
    }

    #[test]
    fn a_dw_program_is_read_once_and_its_zones_resolve_next_to_it() {
        let keys = user("My Programs/Keys.dwp");
        let program = keys_program();
        let reads = Cell::new(0);
        let read = |path: &ContentPath| {
            reads.set(reads.get() + 1);
            match path {
                p if *p == keys => Some(program.clone()),
                p if *p == user("My Programs/Keys/Keys C4.wav") => Some(wav(&[1, 2, 3])),
                _ => None,
            }
        };
        let song = dw_song(dw_cstm(b"@L@\0My Programs/Keys.dwp"));
        let mut cache = SampleCache::default();
        cache.load(&song, &read);
        let zones = cache.program(&keys).expect("the program loads");
        let expected = [
            SampleKey::File(user("My Programs/Keys/Keys C4.wav")),
            SampleKey::Embedded(keys.clone(), 1),
            SampleKey::File(user("My Programs/Keys/Gone.wav")),
        ];
        assert_eq!(
            zones.iter().map(|(_, key)| key.clone()).collect::<Vec<_>>(),
            expected
        );
        assert_eq!(cache.get(&expected[0]).unwrap().frames(), 3);
        assert_eq!(cache.get(&expected[1]).unwrap().frames(), 4);
        assert_eq!(cache.missing().collect::<Vec<_>>(), [&expected[2]]);
        assert_eq!(reads.get(), 3);
        // Again: nothing is read but the missing file once it is forgotten.
        cache.load(&song, &read);
        assert_eq!(reads.get(), 3);
        cache.forget_missing();
        cache.load(&song, &read);
        assert_eq!(reads.get(), 4);
        assert!(
            cache.get(&expected[1]).is_some(),
            "an embedded zone stays decoded"
        );
        // A forget drops the program too (the web's lite program upgraded to its full one): the next load reads it.
        cache.forget(&SampleKey::File(keys.clone()));
        assert!(cache.program(&keys).is_none());

        // A missing program is missing; a zone a CSTM names directly is a file, but not after a path.
        let mut zone = b"@L@\0My Samples/Lead.wav".to_vec();
        zone.resize(1065, 0);
        let zone = [&b"ZONE"[..], &1065u32.to_le_bytes(), &zone].concat();
        let missing = |blob: Vec<u8>| {
            let mut cache = SampleCache::default();
            cache.load(&dw_song(blob), &|_| None);
            assert!(cache.program(&user("Nowhere.dwp")).is_none());
            cache.missing().cloned().collect::<Vec<_>>()
        };
        let path_zone = [&dw_cstm(b"@L@\0Nowhere.dwp")[..], &zone].concat();
        assert_eq!(missing(path_zone), [SampleKey::File(user("Nowhere.dwp"))]);
        let zone_path = [&b"10WD"[..], &zone, &dw_cstm(b"@L@\0Nowhere.dwp")[4..]].concat();
        assert_eq!(
            missing(zone_path),
            [SampleKey::File(user("My Samples/Lead.wav"))]
        );
    }

    #[test]
    fn a_dw_sampler_whose_files_are_missing_plays_the_stand_in_grand() {
        use crate::audio_engine::samples::{sampler_zones, stand_in};
        // A song from another device: its sampler's sample stayed there.
        let grand = stand_in();
        let read = |path: &ContentPath| (*path == grand).then(|| wav(&[1, 2, 3]));
        let song = dw_song(dw_cstm(b"@L@\0My Samples/Gone.wav"));
        // The web's download plan: nothing is missing before a load, so no stand-in.
        assert!(!SampleCache::referenced(&song).contains(&SampleKey::File(grand.clone())));
        let mut cache = SampleCache::default();
        cache.load(&song, &read);
        let blob = song.channels[1].rack.modules[0].file.custom_data.clone();
        let cstm = honeydew_audio::dwp::Cstm::read(blob.as_deref().unwrap());
        let zones = sampler_zones(&cstm, &cache);
        assert_eq!(zones.len(), 1);
        assert_eq!(zones[0].1.frames(), 3);
        // The next load keeps it, read no more.
        cache.load(&song, &|_| None);
        assert_eq!(sampler_zones(&cstm, &cache).len(), 1);
    }

    #[test]
    fn a_program_path_naming_an_audio_file_plays_it_as_a_user_sample() {
        // The loader's third try is a plain audio file, one zone over all keys at root 48.
        let reads = Cell::new(0);
        let read = |path: &ContentPath| {
            reads.set(reads.get() + 1);
            (*path == user("My Samples/Pad.wav")).then(|| wav(&[1, 2, 3, 4, 5]))
        };
        let mut cache = SampleCache::default();
        cache.load(&dw_song(dw_cstm(b"@L@\0My Samples/Pad.wav")), &read);
        let pad = SampleKey::File(user("My Samples/Pad.wav"));
        let zones = cache.program(&user("My Samples/Pad.wav")).expect("taken");
        assert_eq!(&zones[..], [(Zone::user_sample(), pad.clone())]);
        assert_eq!(cache.get(&pad).unwrap().frames(), 5);
        assert_eq!(reads.get(), 1, "read once");
        assert_eq!(cache.missing().count(), 0);
        // An audio clip of the same file draws its waveform, though the program decoded it.
        let clip = song_with(user("My Samples/Pad.wav"));
        let w = (cache.waveform(&kit(&clip)[0], &read)).expect("the pad's waveform");
        assert_eq!(w.summary[0], summarize(&w.data.left));
        assert_eq!(reads.get(), 1, "read once");
        // A file no loader takes plays nothing and is listed missing.
        let junk = |path: &ContentPath| (*path == user("Junk.dwp")).then(|| b"junk".to_vec());
        let mut cache = SampleCache::default();
        cache.load(&dw_song(dw_cstm(b"@L@\0Junk.dwp")), &junk);
        assert_eq!(
            cache.missing().cloned().collect::<Vec<_>>(),
            [SampleKey::File(user("Junk.dwp"))]
        );
    }

    /// The files of a session (no content folder) whose factory is `bundle` alone.
    fn bundled_files(bundle: crate::content::Bundle) -> ProjectFiles {
        let mut files = ProjectFiles::with_provider(Box::new(MemFiles::default()));
        let factory = crate::content::Factory::new(Vec::new()).with_bundle(bundle);
        files.factory = Box::leak(Box::new(factory));
        files
    }

    /// A FLAC of a 440 Hz tone, 0.1 s.
    fn tone() -> Vec<u8> {
        let sine: Vec<f32> = (0..4410).map(|i| (i as f32 * 0.0627).sin() * 0.5).collect();
        honeydew_audio::flac_writer::flac(44_100, 16, &[sine])
    }

    /// Whether `channel`'s key `key` sounds in the engine `song` builds from `cache`.
    fn sounds(song: &Song, cache: &SampleCache, channel: usize, key: u8) -> bool {
        use honeydew_audio::device::AudioRenderer;
        let mut engine = SongAudioEngine::with_samples(song, 48_000.0, cache).unwrap();
        engine.note_on(channel, key, 1.0);
        let mut out = vec![0.0; 2 * 4800];
        engine.render(&mut out, 2);
        out.iter().any(|s| s.abs() > 1e-3)
    }

    #[test]
    fn a_bundled_program_plays_its_zones_and_a_pad_its_bundled_one_shot() {
        use crate::content::bundle::{Kind, tests::fixture};
        let tmp = crate::project_files::content::tests::Temp::new("bundle-play");
        let piano = dwp(&[record(0x1F6, b"C4.flac"), record(0x1F6, b"C5.flac")]);
        let mut bundle = fixture(
            &tmp.0,
            &[
                (
                    Kind::Program,
                    "Presets/DW Sampler/Keys/Test Piano.dwp",
                    vec![
                        ("Test Piano.dwp", piano),
                        ("Test Piano/C4.flac", tone()),
                        ("Test Piano/C5.flac", tone()),
                    ],
                ),
                (Kind::Samples, "Samples/Kick/", vec![("K1.flac", tone())]),
            ],
        );
        // An alias (the format's stability promise) reads its new path's member.
        let (old, new) = ("Samples/Kick/Old.flac", "Samples/Kick/K1.flac");
        bundle.index.aliases.insert(old.into(), new.into());
        assert_eq!(bundle.read(old), Some(tone()));
        let files = bundled_files(bundle);
        let mut picker = crate::ui::views::sample_picker::picker(&files);
        assert_eq!(picker.names_in(&*files.sample_library(), &["Kick"]), ["K1"]);

        let mut song = song_with(app("Built-in/Samples/Kick/K1.flac"));
        song.add_channel(ChannelKind::Synth(crate::song::SynthKind::DwSampler));
        let cstm = dw_cstm(b"@R@\0Built-in/Presets/DW Sampler/Keys/Test Piano.dwp");
        song.channels[2].rack.modules[0].file.custom_data = Some(cstm);
        let mut cache = SampleCache::default();
        cache.load(&song, &reader(&files));
        let piano = app("Built-in/Presets/DW Sampler/Keys/Test Piano.dwp");
        assert_eq!(cache.program(&piano).expect("the program loads").len(), 2);
        assert_eq!(cache.missing().count(), 0);
        assert!(sounds(&song, &cache, 2, 60), "the program");
        assert!(sounds(&song, &cache, 1, 48), "the pad");
    }

    #[test]
    fn a_code_made_built_in_wins_over_a_bundled_member_and_an_absent_member_is_missing() {
        use crate::content::bundle::{Kind, tests::fixture};
        let tmp = crate::project_files::content::tests::Temp::new("bundle-names");
        let loops = vec![("Dust Break 84.flac", tone())];
        let files = bundled_files(fixture(&tmp.0, &[(Kind::Samples, "Loops/", loops)]));
        let song = pads(&[
            app("Built-in/Loops/Dust Break 84.flac"),
            app("Built-in/Samples/Kick/Gone.flac"),
        ]);
        let mut cache = SampleCache::default();
        cache.load(&song, &reader(&files));
        let ours = builtin("Built-in/Loops/Dust Break 84").map(Arc::new);
        assert_eq!(cache.slot(&kit(&song)[0]), ours);
        let gone = SampleKey::Builtin("Built-in/Samples/Kick/Gone".into());
        assert_eq!(cache.missing().collect::<Vec<_>>(), [&gone]);
    }

    #[test]
    fn a_zone_file_is_in_the_folder_named_after_its_program() {
        let plain = ContentPath::Plain("/data/DW/Piano.dwp".into());
        assert_eq!(
            zone_file(&plain, "C4.wav"),
            ContentPath::Plain("/data/DW/Piano/C4.wav".into())
        );
        assert_eq!(
            zone_file(&user("A.b/Pad.dwp"), "x.wav"),
            user("A.b/Pad/x.wav")
        );
        assert_eq!(zone_file(&user("Pad"), "x.wav"), user("Pad/x.wav"));
    }

    /// A 55-byte FLAC whose one CONSTANT frame is numbered 8000 (4096-frame blocks): the decoder pads up to it, so it
    /// decodes to 8001 · 4096 = 32.77 M frames.
    fn far_flac() -> Vec<u8> {
        let crc8 = |bytes: &[u8]| {
            bytes.iter().fold(0u8, |crc, &b| {
                (0..8).fold(crc ^ b, |c, _| {
                    if c & 0x80 != 0 {
                        (c << 1) ^ 0x07
                    } else {
                        c << 1
                    }
                })
            })
        };
        let crc16 = |bytes: &[u8]| {
            bytes.iter().fold(0u16, |crc, &b| {
                (0..8).fold(crc ^ (u16::from(b) << 8), |c, _| {
                    if c & 0x8000 != 0 {
                        (c << 1) ^ 0x8005
                    } else {
                        c << 1
                    }
                })
            })
        };
        let mut bytes = b"fLaC".to_vec();
        bytes.extend([0x80, 0, 0, 34]);
        bytes.extend(4096u16.to_be_bytes());
        bytes.extend(4096u16.to_be_bytes());
        bytes.extend([0; 6]);
        bytes.extend(((44_100u64 << 44) | (15 << 36)).to_be_bytes());
        bytes.extend([0; 16]);
        // Block size code 12 (4096), mono, 16-bit from STREAMINFO; frame number 8000 in UTF-8 form.
        let mut frame = vec![0xFF, 0xF8, 0xC0, 0x00, 0xE1, 0xBD, 0x80];
        frame.push(crc8(&frame));
        frame.extend([0x00, 0x10, 0x00]); // CONSTANT subframe, value 4096
        frame.extend(crc16(&frame).to_be_bytes());
        bytes.extend(frame);
        assert_eq!(bytes.len(), 55);
        bytes
    }

    #[test]
    fn a_program_s_embedded_audio_stays_within_one_frame_budget() {
        let far = far_flac();
        let program = dwp(&vec![record(0x206, &far); 4]);
        assert_eq!(program.len(), 336);
        let path = user("Bomb.dwp");
        let song = dw_song(dw_cstm(b"@L@\0Bomb.dwp"));
        let mut cache = SampleCache::default();
        cache.load(&song, &|p| (*p == path).then(|| program.clone()));
        let kept: Vec<usize> = (0..4)
            .filter_map(|i| cache.get(&SampleKey::Embedded(path.clone(), i)))
            .map(|sample| sample.frames())
            .collect();
        assert_eq!(
            kept,
            [8001 * 4096],
            "the first fits; the rest are over the budget"
        );
        assert!(kept.iter().sum::<usize>() <= MAX_FRAMES);
        let missing = (1..4).map(|i| SampleKey::Embedded(path.clone(), i));
        for key in missing {
            assert!(cache.missing().any(|m| *m == key), "{key:?}");
        }
    }

    #[test]
    fn a_failed_embedded_zone_is_still_missing_after_a_retry() {
        let broken = record(0x206, b"fLaC and nothing else");
        let program = dwp(&[broken]);
        let path = user("Broken.dwp");
        let song = dw_song(dw_cstm(b"@L@\0Broken.dwp"));
        let read = |p: &ContentPath| (*p == path).then(|| program.clone());
        let mut cache = SampleCache::default();
        cache.load(&song, &read);
        let key = SampleKey::Embedded(path.clone(), 0);
        assert_eq!(cache.missing().collect::<Vec<_>>(), [&key]);
        cache.forget_missing();
        cache.load(&song, &read);
        assert_eq!(cache.missing().collect::<Vec<_>>(), [&key]);
    }

    #[test]
    fn a_program_over_the_budget_plays_every_key_with_fewer_layers() {
        // Four key ranges of two velocity layers, 4 frames each; the budget holds four zones. Each range decodes its
        // layer nearest velocity 100, and its other layer plays that sample on its own velocities: every key plays
        // and nothing is missing (decoded bottom-up, the top two ranges were silent).
        let zone = |lo: u8, hi: u8, vel: (u8, u8)| {
            let mut keys = vec![lo, lo, hi, vel.0, vel.1, 0, 0, 0, 0];
            keys.extend(1f32.to_le_bytes());
            keys.extend([0; 12]);
            let mut format = 4u64.to_le_bytes().to_vec();
            format.extend(1u32.to_le_bytes());
            format.extend(16u32.to_le_bytes());
            format.extend(44_100f32.to_le_bytes());
            let pcm: Vec<u8> = [1i16, 2, 3, 4]
                .iter()
                .flat_map(|s| s.to_le_bytes())
                .collect();
            [
                record(0x1F4, &keys),
                record(0x1F7, &format),
                record(0x205, &pcm),
            ]
            .concat()
        };
        let ranges = [(0, 31), (32, 63), (64, 95), (96, 127)];
        let program = dwp(&(ranges.iter())
            .flat_map(|&(lo, hi)| [zone(lo, hi, (1, 64)), zone(lo, hi, (65, 127))])
            .collect::<Vec<_>>());
        let path = user("Grand.dwp");
        let song = dw_song(dw_cstm(b"@L@\0Grand.dwp"));
        let mut cache = SampleCache::with_max_bytes(4 * 4 * 8);
        cache.load(&song, &|p| (*p == path).then(|| program.clone()));
        let zones = cache.program(&path).expect("the program loads");
        assert_eq!(zones.len(), 8);
        for (n, (zone, key)) in zones.iter().enumerate() {
            let loud = SampleKey::Embedded(path.clone(), (n | 1) as u32);
            assert_eq!(key, &loud, "zone {n}");
            assert!(cache.get(key).is_some(), "zone {n}");
            let (lo, hi) = ranges[n / 2];
            assert_eq!((zone.key_low, zone.key_high), (lo, hi), "zone {n}");
            let vel = if n % 2 == 0 { (1, 64) } else { (65, 127) };
            assert_eq!((zone.vel_low, zone.vel_high), vel, "zone {n}");
        }
        assert_eq!(cache.missing().count(), 0);
    }

    #[test]
    fn a_changed_program_never_overdraws_the_budget() {
        // v1: zone 0 broken, zone 1 decodes to 32.77 M frames. v2: zone 0 decodes to 32.77 M frames, and zone 1 is
        // still cached from v1 (its key is the index), which is more than the budget has left.
        let far = far_flac();
        let pcm: Vec<u8> = [1i16, 2, 3, 4]
            .iter()
            .flat_map(|s| s.to_le_bytes())
            .collect();
        let mut format = 4u64.to_le_bytes().to_vec();
        format.extend(1u32.to_le_bytes());
        format.extend(16u32.to_le_bytes());
        format.extend(44_100f32.to_le_bytes());
        let tiny = [record(0x1F7, &format), record(0x205, &pcm)].concat();
        let v1 = dwp(&[record(0x206, b"fLaC broken"), record(0x206, &far)]);
        let v2 = dwp(&[record(0x206, &far), tiny]);
        let path = user("Changed.dwp");
        let song = dw_song(dw_cstm(b"@L@\0Changed.dwp"));
        let second = Cell::new(false);
        let read = |p: &ContentPath| {
            (*p == path).then(|| if second.get() { v2.clone() } else { v1.clone() })
        };
        let mut cache = SampleCache::default();
        cache.load(&song, &read);
        assert!(cache.get(&SampleKey::Embedded(path.clone(), 1)).is_some());
        cache.forget_missing();
        second.set(true);
        cache.load(&song, &read);
        assert!(cache.get(&SampleKey::Embedded(path.clone(), 0)).is_some());
    }

    /// Adds `key` to `cache` as a song load would, reading every file as `bytes`.
    fn add(cache: &mut SampleCache, key: &SampleKey, bytes: &[u8]) {
        let read = |_: &ContentPath| Some(bytes.to_vec());
        let mut refs = Refs::new(cache, &read);
        refs.add(key.clone());
    }

    #[test]
    fn samples_past_the_total_decode_budget_are_missing() {
        // Each of these is 4 mono frames, 16 bytes of PCM; the cap holds two.
        let [a, b, c] = ["a.wav", "b.wav", "c.wav"].map(|name| SampleKey::File(user(name)));
        let mut cache = SampleCache::with_max_bytes(32);
        for key in [&a, &b, &c] {
            add(&mut cache, key, &wav(&[1, 2, 3, 4]));
        }
        assert!(cache.get(&a).is_some() && cache.get(&b).is_some());
        assert_eq!(cache.get(&c), None);
        assert_eq!(cache.missing().collect::<Vec<_>>(), [&c]);
        // The room a forgotten sample leaves is used by the next load, once the missing one is tried again.
        cache.forget(&a);
        cache.forget_missing();
        add(&mut cache, &c, &wav(&[1, 2, 3, 4]));
        assert!(cache.get(&c).is_some());
        assert_eq!(cache.missing().count(), 0);
        // One bigger than the whole cap is missing at once.
        add(&mut cache, &a, &wav(&[0; 9]));
        assert_eq!(cache.get(&a), None);
    }

    #[test]
    fn a_program_s_embedded_audio_counts_against_the_total_budget() {
        // Three zones of 4 mono frames each, and room for 64 bytes, which the program budget reads as 8 frames: two
        // fit, and the third plays one of them.
        let pcm: Vec<u8> = [1i16, 2, 3, 4]
            .iter()
            .flat_map(|s| s.to_le_bytes())
            .collect();
        let mut format = 4u64.to_le_bytes().to_vec();
        format.extend(1u32.to_le_bytes());
        format.extend(16u32.to_le_bytes());
        format.extend(44_100f32.to_le_bytes());
        let tiny = [record(0x1F7, &format), record(0x205, &pcm)].concat();
        let program = dwp(&[tiny.clone(), tiny.clone(), tiny]);
        let path = user("Three.dwp");
        let song = dw_song(dw_cstm(b"@L@\0Three.dwp"));
        let mut cache = SampleCache::with_max_bytes(64);
        cache.load(&song, &|p| (*p == path).then(|| program.clone()));
        let kept = (0..3).filter_map(|i| cache.get(&SampleKey::Embedded(path.clone(), i)));
        assert_eq!(kept.count(), 2);
        let zones = cache.program(&path).expect("the program loads");
        assert!(cache.get(&zones[2].1).is_some());
        assert_eq!(cache.missing().count(), 0);
    }

    #[test]
    fn a_workers_cache_hands_the_next_load_everything_it_decoded() {
        let keys = user("My Programs/Keys.dwp");
        let program = keys_program();
        let read = |path: &ContentPath| match path {
            p if *p == keys => Some(program.clone()),
            p if *p == user("My Programs/Keys/Keys C4.wav") => Some(wav(&[1, 2, 3])),
            _ => None,
        };
        let song = dw_song(dw_cstm(b"@L@\0My Programs/Keys.dwp"));
        let dw = &song.channels[1].rack.modules[0];
        // On the worker: by the preset's module, or by the program a pick took.
        let mut by_module = SampleCache::default();
        by_module.load_dw(dw, &read);
        let mut by_program = SampleCache::default();
        by_program.load_program(&keys, &read);
        for worker in [by_module, by_program] {
            let mut cache = SampleCache::default();
            cache.absorb(worker);
            // The engine build that follows reads and decodes nothing.
            cache.load(&song, &|_| panic!("the app thread read a file"));
            assert_eq!(cache.program(&keys).expect("the program").len(), 3);
            let c4 = SampleKey::File(user("My Programs/Keys/Keys C4.wav"));
            assert_eq!(cache.get(&c4).unwrap().frames(), 3);
            let embedded = SampleKey::Embedded(keys.clone(), 1);
            assert_eq!(cache.get(&embedded).unwrap().frames(), 4);
            // The zone whose file is missing is listed so, and not tried again.
            let gone = SampleKey::File(user("My Programs/Keys/Gone.wav"));
            assert_eq!(cache.missing().collect::<Vec<_>>(), [&gone]);
        }
        // Another module type, or a DW Sampler with no program, has nothing to decode.
        let mut other = dw.clone();
        other.type_id = 6;
        let mut none = SampleCache::default();
        none.load_dw(&other, &|_| panic!("read"));
        none.load_dw(&ModuleInstance::new(1, 2), &|_| panic!("read"));
        assert_eq!(none.missing().count(), 0);
    }

    #[test]
    fn what_a_worker_decoded_replaces_the_cached_samples_of_the_same_files() {
        let pad = user("My Samples/Pad.wav");
        let key = SampleKey::File(pad.clone());
        let other = SampleKey::File(user("My Samples/Other.wav"));
        let mut cache = SampleCache::default();
        cache.load_program(&pad, &|_| Some(wav(&[1, 2])));
        cache.samples.insert(other.clone(), None);
        assert_eq!(cache.get(&key).unwrap().frames(), 2);
        // The file was rewritten; the pick reads it afresh.
        let mut worker = SampleCache::default();
        worker.load_program(&pad, &|_| Some(wav(&[1, 2, 3])));
        cache.absorb(worker);
        assert_eq!(cache.get(&key).unwrap().frames(), 3);
        assert!(cache.samples.contains_key(&other), "the others stay");
    }

    #[test]
    fn the_disk_reader_reads_the_user_folder_as_the_reader_does_and_only_where_files_are_on_disk() {
        let dir = Removed(
            std::env::temp_dir().join(format!("honeydew-e14c-reader-{}", std::process::id())),
        );
        let root = &dir.0;
        let _ = std::fs::remove_dir_all(root);
        std::fs::create_dir_all(root.join("Kicks")).unwrap();
        std::fs::write(root.join("kick.wav"), wav(&[3])).unwrap();
        let files = ProjectFiles::on_disk(root);
        let read = disk_reader(&files).expect("files on disk");
        assert_eq!(read(&user("kick.wav")), Some(wav(&[3])));
        for none in [
            user("gone.wav"),
            user("Kicks"),
            user("../kick.wav"),
            app("kick.wav"),
        ] {
            assert_eq!(read(&none), None, "{none:?}");
        }
        // A session that keeps copies in memory (the web) has none to share with a thread.
        let mem = ProjectFiles::with_provider(Box::new(MemFiles::default()));
        assert!(disk_reader(&mem).is_none());
    }

    #[test]
    fn our_own_app_paths_are_built_in_samples_and_no_others() {
        let built_in = |name: &str| SampleKey::Builtin(name.into());
        for path in [
            "Built-in/Loops/X.wav",
            "Built-in\\Loops\\X.wav",
            "Built-in/Loops/X",
        ] {
            assert_eq!(
                SampleKey::of_path(&app(path)),
                built_in("Built-in/Loops/X"),
                "{path}"
            );
        }
        assert_eq!(
            SampleKey::of_path(&app("Built-in/Kick")),
            built_in("Built-in/Kick")
        );
        // Only `@R@` is built in, and only that first folder.
        for path in [
            user("Built-in/Loops/X.wav"),
            app("Built-in Loops/X.wav"),
            app("Loops/Built-in/X.wav"),
        ] {
            assert_eq!(SampleKey::of_path(&path), SampleKey::File(path.clone()));
        }
        let plain = ContentPath::Plain("Built-in/Kick".into());
        assert_eq!(SampleKey::of_path(&plain), SampleKey::File(plain.clone()));
        // A pad naming one plays it, never a file; `Built-in/<Pad>` with no path is unchanged.
        let mut song = song_with(app("Built-in\\Kick.wav"));
        song.channels[1].drum_kit.as_mut().unwrap().slots[1].folder = "Built-in/Snare".into();
        let mut cache = SampleCache::default();
        cache.load(&song, &|path| panic!("read {path:?}"));
        let kit = kit(&song);
        assert_eq!(SampleKey::of_slot(&kit[0]), Some(built_in("Built-in/Kick")));
        assert_eq!(cache.slot(&kit[0]), builtin("Built-in/Kick").map(Arc::new));
        assert_eq!(
            SampleKey::of_slot(&kit[1]),
            Some(built_in("Built-in/Snare"))
        );
        assert_eq!(cache.missing().count(), 0);
    }

    #[test]
    fn a_slicer_naming_a_built_in_loop_plays_it_without_reading_a_file() {
        let mut song = Song::default();
        song.add_channel(ChannelKind::Synth(crate::song::SynthKind::Slicer));
        let module = &mut song.channels[1].rack.modules[0];
        assert_eq!(module.type_id, 33);
        let id = module.instance_id;
        let name = honeydew_audio::builtin_loops::LOOPS[0].name;
        let path = app(&format!("Built-in/Loops/{name}.wav"));
        crate::project_files::load_pick(module, Path::new("x.wav"), &[], Some(path.clone()))
            .unwrap();
        let mut cache = SampleCache::default();
        cache.load(&song, &|path| panic!("read {path:?}"));
        assert_eq!(cache.missing().count(), 0);
        assert!(cache.get(&SampleKey::of_path(&path)).is_some());
        // The engine finds it under the same key: AUTO's clip has the loop's slices.
        let (beats, notes) = crate::audio_engine::slicer_auto_clip(&song, 1, id, &cache).unwrap();
        assert!(beats > 0.0 && !notes.is_empty(), "{beats} {}", notes.len());
    }

    /// A 16-bit stereo PCM WAV at 44.1 kHz: `frames` frames of silence.
    fn stereo_wav(frames: usize) -> Vec<u8> {
        let data = vec![0u8; frames * 4];
        let mut bytes = b"RIFF".to_vec();
        bytes.extend((36 + data.len() as u32).to_le_bytes());
        bytes.extend(b"WAVEfmt ");
        bytes.extend(16u32.to_le_bytes());
        bytes.extend(1u16.to_le_bytes()); // PCM
        bytes.extend(2u16.to_le_bytes()); // stereo
        bytes.extend(44_100u32.to_le_bytes());
        bytes.extend(176_400u32.to_le_bytes());
        bytes.extend(4u16.to_le_bytes());
        bytes.extend(16u16.to_le_bytes());
        bytes.extend(b"data");
        bytes.extend((data.len() as u32).to_le_bytes());
        bytes.extend(data);
        bytes
    }

    #[test]
    fn a_stereo_sample_counts_both_channels() {
        // 4 stereo frames are 32 bytes: one fills the cap.
        let [a, b] = ["a.wav", "b.wav"].map(|name| SampleKey::File(user(name)));
        let mut cache = SampleCache::with_max_bytes(32);
        add(&mut cache, &a, &stereo_wav(4));
        add(&mut cache, &b, &stereo_wav(4));
        assert!(cache.get(&a).is_some());
        assert_eq!(cache.get(&b), None);
    }

    #[test]
    fn a_plain_audio_program_over_the_budget_is_missing() {
        let path = user("Plain.wav");
        let song = dw_song(dw_cstm(b"@L@\0Plain.wav"));
        let read = |p: &ContentPath| (*p == path).then(|| wav(&[1, 2, 3, 4]));
        let key = SampleKey::File(path.clone());
        let mut cache = SampleCache::with_max_bytes(8);
        cache.load(&song, &read);
        assert_eq!(cache.get(&key), None);
        assert_eq!(cache.missing().collect::<Vec<_>>(), [&key]);
        let mut cache = SampleCache::with_max_bytes(16);
        cache.load(&song, &read);
        assert!(cache.get(&key).is_some());
    }

    #[test]
    fn a_stretch_render_past_the_budget_is_missing() {
        // 4 mono frames are 16 bytes, their x2 render 32: room for 40 holds the sample, not both.
        let mut slot = SamplerSlot::default();
        slot.file.path = Some(user("a.wav"));
        (slot.file.stretch_mode, slot.file.stretch_ratio) = (1, 2.0);
        let read = |_: &ContentPath| Some(wav(&[1, 2, 3, 4]));
        for (max, fits) in [(40, false), (48, true)] {
            let mut cache = SampleCache::with_max_bytes(max);
            let mut refs = Refs::new(&mut cache, &read);
            refs.add_slot(&slot);
            assert_eq!(cache.slot(&slot).is_some(), fits, "cap {max}");
            assert_eq!(cache.missing().count(), usize::from(!fits), "cap {max}");
            if !fits {
                // Like a missing sample, a missing render is tried again after `forget_missing`.
                cache.forget_missing();
                cache.max_bytes = Some(48);
                let mut refs = Refs::new(&mut cache, &read);
                refs.add_slot(&slot);
                assert!(cache.slot(&slot).is_some());
            }
        }
        // The renders share the budget with the samples: the sample (16) and the x2 render (32) leave 32 of 80, which
        // the x3 render (48) does not fit.
        let mut cache = SampleCache::with_max_bytes(80);
        let mut refs = Refs::new(&mut cache, &read);
        refs.add_slot(&slot);
        slot.file.stretch_ratio = 3.0;
        refs.add_slot(&slot);
        assert!(cache.slot(&slot).is_none());
        slot.file.stretch_ratio = 2.0;
        assert!(cache.slot(&slot).is_some());
    }

    /// A song whose drum pads play `paths`, in order, and nothing else: its pads are the only samples it names.
    fn pads(paths: &[ContentPath]) -> Song {
        let mut song = Song::default();
        song.add_channel(ChannelKind::Drums);
        let slots = &mut song.channels[1].drum_kit.as_mut().unwrap().slots;
        for (i, slot) in slots.iter_mut().enumerate() {
            slot.folder.clear();
            slot.file.path = paths.get(i).cloned();
        }
        song
    }

    fn file(path: &ContentPath) -> SampleKey {
        SampleKey::File(path.clone())
    }

    #[test]
    fn a_sample_the_song_stopped_playing_is_pruned_and_its_budget_freed() {
        // Each is 4 mono frames, 16 bytes of PCM; the cap holds two.
        let [a, b, c] = ["a.wav", "b.wav", "c.wav"].map(user);
        let reads = Cell::new(0);
        let read = |_: &ContentPath| {
            reads.set(reads.get() + 1);
            Some(wav(&[1, 2, 3, 4]))
        };
        let mut cache = SampleCache::with_max_bytes(32);
        cache.load(&pads(&[a.clone(), b.clone()]), &read);
        assert_eq!(cache.room(), 0);
        let held = cache.get(&file(&a)).unwrap();
        // An edit, or an undo, leaves `a` unplayed: its sample and its budget are freed by the next load.
        cache.load(&pads(std::slice::from_ref(&b)), &read);
        assert_eq!(cache.get(&file(&a)), None);
        assert!(cache.get(&file(&b)).is_some());
        assert_eq!(cache.room(), 16);
        assert_eq!(cache.missing().count(), 0);
        // Whoever holds the sample (the audio thread, an open editor) keeps it: the cache gave up only its own `Arc`.
        assert_eq!((held.frames(), Arc::strong_count(&held)), (4, 1));
        // The song comes back (a redo): `a` is read again.
        cache.load(&pads(&[a.clone(), b.clone()]), &read);
        assert!(cache.get(&file(&a)).is_some());
        assert_eq!(reads.get(), 3);
        // A swap at the cap, in one load: the sample it drops makes room for the one it adds.
        cache.load(&pads(&[b.clone(), c.clone()]), &read);
        assert!(cache.get(&file(&c)).is_some(), "the new sample was refused");
        assert_eq!(cache.missing().count(), 0);
    }

    #[test]
    fn a_sample_another_slot_still_plays_stays() {
        let [a, b] = ["a.wav", "b.wav"].map(user);
        let reads = Cell::new(0);
        let read = |_: &ContentPath| {
            reads.set(reads.get() + 1);
            Some(wav(&[1, 2, 3, 4]))
        };
        let mut cache = SampleCache::with_max_bytes(32);
        cache.load(&pads(&[a.clone(), a.clone()]), &read);
        let kept = cache.get(&file(&a)).unwrap();
        // The first pad moves to `b`; the second still plays `a`.
        cache.load(&pads(&[b.clone(), a.clone()]), &read);
        assert!(Arc::ptr_eq(&kept, &cache.get(&file(&a)).unwrap()));
        assert!(cache.get(&file(&b)).is_some());
        assert_eq!(reads.get(), 2, "each file was read once");
        assert_eq!(cache.room(), 0);
    }

    #[test]
    fn a_swap_at_the_cap_frees_the_old_stretch_render_first() {
        // 4 mono frames are 16 bytes and their x2 render 32: one sample with its render fills the cap.
        let [a, b] = ["a.wav", "b.wav"].map(user);
        let read = |_: &ContentPath| Some(wav(&[1, 2, 3, 4]));
        let stretched = |path: &ContentPath| {
            let mut song = pads(std::slice::from_ref(path));
            let slot = &mut song.channels[1].drum_kit.as_mut().unwrap().slots[0];
            (slot.file.stretch_mode, slot.file.stretch_ratio) = (1, 2.0);
            song
        };
        let mut cache = SampleCache::with_max_bytes(48);
        cache.load(&stretched(&a), &read);
        assert!(cache.slot(&kit(&stretched(&a))[0]).is_some());
        cache.load(&stretched(&b), &read);
        assert!(cache.slot(&kit(&stretched(&b))[0]).is_some());
        assert_eq!((cache.room(), cache.missing().count()), (0, 0));
        assert_eq!(cache.get(&file(&a)), None);
    }

    #[test]
    fn a_waveform_summary_goes_with_its_pruned_sample() {
        let a = user("a.wav");
        let read = |_: &ContentPath| Some(wav(&[1, 2, 3, 4]));
        let mut cache = SampleCache::default();
        let song = pads(std::slice::from_ref(&a));
        let w = cache.waveform(&kit(&song)[0], &read).unwrap();
        let data = Arc::downgrade(&w.data);
        // The song still plays it: the summary stays, the same one.
        cache.load(&song, &read);
        let again = cache.waveform(&kit(&song)[0], &read).unwrap();
        assert!(Arc::ptr_eq(&w.summary, &again.summary));
        drop((w, again));
        assert!(data.upgrade().is_some());
        cache.load(&pads(&[]), &read);
        assert!(data.upgrade().is_none(), "the summary kept the sample");
        assert!(cache.waveforms.is_empty());
    }

    #[test]
    fn a_plain_audio_program_swapped_at_the_cap_frees_the_old_one_first() {
        // Two programs that are plain audio files, 4 mono frames (16 bytes) each, and room for one.
        let [a, b] = ["a.wav", "b.wav"].map(user);
        let read = |_: &ContentPath| Some(wav(&[1, 2, 3, 4]));
        let song = |name: &[u8]| dw_song(dw_cstm(&[b"@L@\0", name].concat()));
        let mut cache = SampleCache::with_max_bytes(16);
        cache.load(&song(b"a.wav"), &read);
        assert!(cache.get(&file(&a)).is_some() && cache.program(&a).is_some());
        cache.load(&song(b"b.wav"), &read);
        assert!(cache.get(&file(&b)).is_some(), "b found no room");
        assert_eq!(cache.missing().count(), 0);
        assert!(cache.get(&file(&a)).is_none() && cache.program(&a).is_none());
    }

    #[test]
    fn a_stretch_render_changed_at_the_cap_frees_the_old_one_first() {
        // 4 mono frames are 16 bytes, their x2 render 32 and their x3 render 48: room for 64 holds the sample with the
        // x2 render, and the x3 render only once the x2 one is freed.
        let read = |_: &ContentPath| Some(wav(&[1, 2, 3, 4]));
        let stretched = |ratio: f64| {
            let mut song = pads(&[user("a.wav")]);
            let slot = &mut song.channels[1].drum_kit.as_mut().unwrap().slots[0];
            (slot.file.stretch_mode, slot.file.stretch_ratio) = (1, ratio);
            song
        };
        let mut cache = SampleCache::with_max_bytes(16 + 48);
        cache.load(&stretched(2.0), &read);
        assert_eq!(cache.room(), 16);
        let song = stretched(3.0);
        cache.load(&song, &read);
        assert_eq!(cache.slot(&kit(&song)[0]).map(|s| s.frames()), Some(12));
        assert_eq!(cache.missing().count(), 0);
    }

    /// A song with two audio clips, on `a.wav` and `b.wav`, and the key of the first.
    fn two_audio_clips() -> (Song, crate::ui::preview::ClipKey) {
        let mut song = Song::default();
        song.add_channel(ChannelKind::AudioClip);
        let lane = &mut song.channels[1].lanes[0];
        let ids: Vec<u32> = [("a.wav", 0), ("b.wav", 1024)]
            .into_iter()
            .map(|(name, start)| {
                let clip = lane.add_clip(start, 4.0);
                let mut slot = SamplerSlot::default();
                slot.file.path = Some(user(name));
                clip.file.sample = Some(Box::new(slot));
                clip.id
            })
            .collect();
        let key = crate::ui::preview::ClipKey {
            channel: 1,
            lane: 0,
            id: ids[0],
        };
        (song, key)
    }

    #[test]
    fn a_build_with_part_of_the_song_keeps_the_rest_decoded() {
        // Preview plays one clip alone (`preview::solo`: every other clip is cleared): its engine build must not free
        // the other clip's sample, or Preview off decodes it again.
        let (song, key) = two_audio_clips();
        let solo = crate::ui::preview::solo(&song, key).unwrap();
        assert_eq!(solo.channels[1].lanes[0].clips.len(), 1);
        let reads = Cell::new(0);
        let read = |_: &ContentPath| {
            reads.set(reads.get() + 1);
            Some(wav(&[1, 2, 3, 4]))
        };
        let mut cache = SampleCache::default();
        cache.load(&song, &read);
        let [a, b] = ["a.wav", "b.wav"].map(|name| cache.get(&file(&user(name))).unwrap());
        // Preview on, then off.
        cache.load_keeping(&solo, &song, &read);
        cache.load_keeping(&song, &song, &read);
        for (name, before) in [("a.wav", a), ("b.wav", b)] {
            let now = cache.get(&file(&user(name))).expect(name);
            assert!(Arc::ptr_eq(&before, &now), "{name} was decoded again");
        }
        assert_eq!(reads.get(), 2, "each file was read once");
        // The solo song alone is a song that plays one sample: the other goes.
        cache.load(&solo, &read);
        assert!(cache.get(&file(&user("b.wav"))).is_none());
    }

    #[test]
    fn a_custom_table_is_pruned_with_its_osc_and_a_missing_file_is_not_read_again() {
        let path = user("My Samples/wave.wav");
        let key = SampleKey::File(path.clone());
        let reads = Cell::new(0);
        let read = |p: &ContentPath| {
            reads.set(reads.get() + 1);
            (*p == path).then(|| wav(&[0, 9000, 0, -9000]))
        };
        // A pad plays the file too: the sample stays while it does, and goes with it; the table stays with the Osc.
        let with_pad = with_custom_shape(song_with(path.clone()), &path);
        let shape_only = with_custom_shape(Song::default(), &path);
        let mut cache = SampleCache::default();
        cache.load(&with_pad, &read);
        let table = cache.shape(&key).expect("built at load");
        assert!(cache.get(&key).is_some());
        cache.load(&shape_only, &read);
        assert!(cache.get(&key).is_none(), "the pad is gone");
        assert!(Arc::ptr_eq(&table, &cache.shape(&key).unwrap()));
        assert_eq!(reads.get(), 1, "the table is not built again");
        // The Osc gone: the table goes.
        cache.load(&Song::default(), &read);
        assert!(cache.shape(&key).is_none());
        // A file that is missing stays missing while the Osc plays it, without being read on each load.
        let gone = Cell::new(0);
        let read = |_: &ContentPath| {
            gone.set(gone.get() + 1);
            None
        };
        let mut cache = SampleCache::default();
        for _ in 0..3 {
            cache.load(&shape_only, &read);
        }
        assert_eq!(gone.get(), 1);
        assert_eq!(cache.missing().collect::<Vec<_>>(), [&key]);
        cache.load(&Song::default(), &read);
        assert_eq!(cache.missing().count(), 0);
    }

    #[test]
    fn a_custom_table_counts_against_the_total_budget() {
        // Room for one table and 40 bytes: the second Osc's file decodes (16 bytes) but its table does not fit, so its
        // shape is missing (the Osc plays silence), like a sample past the budget.
        let [pa, pb] = ["My Samples/a.wav", "My Samples/b.wav"].map(user);
        let (a, b) = (file(&pa), file(&pb));
        let reads = Cell::new(0);
        let read = |_: &ContentPath| {
            reads.set(reads.get() + 1);
            Some(wav(&[0, 9000, 0, -9000]))
        };
        let both = with_custom_shape(with_custom_shape(Song::default(), &pa), &pb);
        let mut cache = SampleCache::with_max_bytes(TABLE_BYTES + 40);
        cache.load(&both, &read);
        assert_eq!(cache.shape(&a).map(|w| w.byte_len()), Some(TABLE_BYTES));
        assert!(cache.shape(&b).is_none());
        assert_eq!(
            cache.room(),
            40,
            "the table counts; the decoded file is freed"
        );
        assert_eq!(cache.missing().collect::<Vec<_>>(), [&b]);
        // A rebuild keeps the miss without reading the file again; a new pick (`forget_missing`) with no room misses it
        // again.
        let before = reads.get();
        cache.load(&both, &read);
        assert_eq!(reads.get(), before);
        cache.forget_missing();
        cache.load(&both, &read);
        assert!(cache.shape(&b).is_none());
        assert_eq!(cache.missing().collect::<Vec<_>>(), [&b]);
        // The first Osc gone, its table's room is free: the next pick builds the second.
        let only_b = with_custom_shape(Song::default(), &pb);
        cache.load(&only_b, &read);
        cache.forget_missing();
        cache.load(&only_b, &read);
        assert!(cache.shape(&a).is_none() && cache.shape(&b).is_some());
        assert_eq!((cache.room(), cache.missing().count()), (40, 0));
        // Samples and tables share the budget: the pad's 16 bytes of PCM leave a byte too few for the table.
        let pad = user("My Samples/pad.wav");
        let song = with_custom_shape(pads(std::slice::from_ref(&pad)), &pa);
        let mut cache = SampleCache::with_max_bytes(TABLE_BYTES + 15);
        cache.load(&song, &read);
        assert!(cache.get(&file(&pad)).is_some());
        assert!(cache.shape(&a).is_none());
        assert_eq!(cache.missing().collect::<Vec<_>>(), [&a]);
    }

    /// A 3x Osc (channel 1) and a drum kit (channel 2) whose first pad (the only one with a sample) play `path`.
    fn osc_then_pad(path: &ContentPath) -> Song {
        let mut song = with_custom_shape(Song::default(), path);
        let pad = pads(std::slice::from_ref(path)).channels.remove(1);
        song.channels.push(pad);
        song
    }

    /// [`osc_then_pad`], the kit first.
    fn pad_then_osc(path: &ContentPath) -> Song {
        with_custom_shape(pads(std::slice::from_ref(path)), path)
    }

    #[test]
    fn a_lone_table_fits_once_its_file_is_dropped() {
        // One Osc, one 16-byte file: the table replaces the file, so the cap needs TABLE_BYTES, not TABLE_BYTES + 16.
        // A table that cannot fit is not built (a resample and FFTs for nothing).
        let path = user("My Samples/a.wav");
        let key = file(&path);
        let read = |_: &ContentPath| Some(wav(&[0, 9000, 0, -9000]));
        let song = with_custom_shape(Song::default(), &path);
        for (cap, fits) in [
            (TABLE_BYTES - 1, false),
            (TABLE_BYTES, true),
            (TABLE_BYTES + 8, true),
        ] {
            let mut cache = SampleCache::with_max_bytes(cap);
            cache.load(&song, &read);
            assert_eq!(cache.shape(&key).is_some(), fits, "cap {cap}");
            assert_eq!(cache.missing().count(), usize::from(!fits), "cap {cap}");
            assert_eq!(cache.builds, usize::from(fits), "cap {cap}");
            assert!(cache.get(&key).is_none(), "the file is dropped, cap {cap}");
        }
    }

    #[test]
    fn a_pad_that_shares_the_file_keeps_its_bytes_in_the_count() {
        // The pad keeps the 16-byte file, so the table needs TABLE_BYTES + 16, whichever of the two is walked first.
        let path = user("My Samples/a.wav");
        let key = file(&path);
        let read = |_: &ContentPath| Some(wav(&[0, 9000, 0, -9000]));
        let (pad_first, osc_first) = (pad_then_osc(&path), osc_then_pad(&path));
        for (name, song) in [("pad first", pad_first), ("osc first", osc_first)] {
            for (cap, fits) in [(TABLE_BYTES + 15, false), (TABLE_BYTES + 16, true)] {
                let mut cache = SampleCache::with_max_bytes(cap);
                cache.load(&song, &read);
                assert!(
                    cache.get(&key).is_some(),
                    "the pad's sample, {name}, cap {cap}"
                );
                assert_eq!(cache.shape(&key).is_some(), fits, "{name}, cap {cap}");
                assert_eq!(
                    cache.missing().count(),
                    usize::from(!fits),
                    "{name}, cap {cap}"
                );
            }
        }
    }

    #[test]
    fn a_file_an_osc_walks_before_a_pad_is_read_once() {
        let path = user("My Samples/wave.wav");
        let key = file(&path);
        let reads = Cell::new(0);
        let read = |_: &ContentPath| {
            reads.set(reads.get() + 1);
            Some(wav(&[0, 9000, 0, -9000]))
        };
        let song = osc_then_pad(&path);
        let mut cache = SampleCache::default();
        cache.load(&song, &read);
        assert_eq!(reads.get(), 1, "decoded twice");
        assert!(cache.get(&key).is_some() && cache.shape(&key).is_some());
        // A rebuild reads nothing; the pad gone, the sample goes and the table stays.
        cache.load(&song, &read);
        assert_eq!(reads.get(), 1);
        cache.load(&with_custom_shape(Song::default(), &path), &read);
        assert!(cache.get(&key).is_none() && cache.shape(&key).is_some());
        assert_eq!(reads.get(), 1);
    }

    #[test]
    fn a_program_and_its_embedded_zones_are_pruned_with_the_sampler_that_plays_it() {
        let pcm: Vec<u8> = [1i16, 2, 3, 4]
            .iter()
            .flat_map(|s| s.to_le_bytes())
            .collect();
        let mut format = 4u64.to_le_bytes().to_vec();
        format.extend(1u32.to_le_bytes());
        format.extend(16u32.to_le_bytes());
        format.extend(44_100f32.to_le_bytes());
        let tiny = [record(0x1F7, &format), record(0x205, &pcm)].concat();
        let program = dwp(&[tiny.clone(), tiny.clone(), tiny]);
        let path = user("Three.dwp");
        let song = dw_song(dw_cstm(b"@L@\0Three.dwp"));
        let read = |p: &ContentPath| (*p == path).then(|| program.clone());
        let zone = |cache: &SampleCache, i| cache.get(&SampleKey::Embedded(path.clone(), i));
        let mut cache = SampleCache::with_max_bytes(1000);
        cache.load(&song, &read);
        let first = zone(&cache, 0).unwrap();
        // While the song plays the program, its zones stay (a program loads once).
        cache.load(&song, &read);
        assert!(Arc::ptr_eq(&first, &zone(&cache, 0).unwrap()));
        assert_eq!(cache.room(), 1000 - 3 * 16);
        // A song without the sampler: the zones, and the program that names them, go.
        cache.load(&pads(&[]), &read);
        assert!((0..3).all(|i| zone(&cache, i).is_none()));
        assert!(cache.program(&path).is_none());
        assert_eq!((cache.room(), cache.missing().count()), (1000, 0));
        // It comes back whole: a program left without its zones would stay empty.
        cache.load(&song, &read);
        assert!((0..3).all(|i| zone(&cache, i).is_some()));
        assert!(cache.program(&path).is_some());
    }

    #[test]
    fn missing_lists_only_what_the_song_plays_now() {
        let gone = user("gone.wav");
        let font = user("My SoundFonts/Gone.sf2");
        let reads = Cell::new(0);
        let read = |_: &ContentPath| {
            reads.set(reads.get() + 1);
            None
        };
        let mut cache = SampleCache::default();
        let (song, fonts) = (
            pads(std::slice::from_ref(&gone)),
            soundfont_song(font.clone()),
        );
        cache.load(&song, &read);
        cache.load(&fonts, &read);
        assert_eq!(cache.missing().count(), 1, "the song with a font replaced");
        cache.load(&song, &read);
        assert_eq!(cache.missing().collect::<Vec<_>>(), [&file(&gone)]);
        // Still missing, not read again, while the song plays it.
        let before = reads.get();
        cache.load(&song, &read);
        assert_eq!(reads.get(), before);
        assert_eq!(cache.missing().collect::<Vec<_>>(), [&file(&gone)]);
        // A song that plays neither lists neither; the font was dropped by its own load already.
        cache.load(&pads(&[]), &read);
        assert_eq!(cache.missing().count(), 0);
        cache.load(&fonts, &read);
        cache.load(&pads(&[]), &read);
        assert_eq!(cache.missing().count(), 0);
    }

    #[test]
    fn soundfont_bytes_count_against_the_total_budget() {
        let len = SINE_SF2.len();
        let [a, b] = ["My SoundFonts/a.sf2", "My SoundFonts/b.sf2"].map(user);
        let read = |_: &ContentPath| Some(SINE_SF2.to_vec());
        // Room for one and a half: the second SoundFont is missing, like a sample past the budget.
        let mut cache = SampleCache::with_max_bytes(len + len / 2);
        cache.load(&soundfonts_song(&[a.clone(), b.clone()]), &read);
        assert_eq!(cache.soundfont(&a).map(|f| f.byte_len()), Some(len));
        assert!(cache.soundfont(&b).is_none());
        assert_eq!(cache.room(), len / 2);
        assert_eq!(cache.missing().collect::<Vec<_>>(), [&file(&b)]);
        // Not tried again on a rebuild; a new pick (`forget_missing`) with no room still misses it.
        cache.load(&soundfonts_song(&[a.clone(), b.clone()]), &read);
        assert!(cache.soundfont(&b).is_none());
        cache.forget_missing();
        cache.load(&soundfonts_song(&[a.clone(), b.clone()]), &read);
        assert!(cache.soundfont(&b).is_none());
        // The samples share what the SoundFont left: 652 bytes do not fit in 650; 16 do.
        let big = file(&user("big.wav"));
        add(&mut cache, &big, &wav(&vec![0; len / 8 + 1]));
        assert_eq!(cache.get(&big), None);
        add(&mut cache, &file(&user("small.wav")), &wav(&[1, 2, 3, 4]));
        assert_eq!(cache.room(), len / 2 - 16);
        // And a SoundFont past what the samples left is missing too: the song plays a 16-byte pad (walked first) with it.
        let mut song = soundfont_song(a.clone());
        let pad = pads(&[user("small.wav")]).channels.remove(1);
        song.channels.insert(1, pad);
        for (max, fits) in [(len + 16, true), (len + 15, false)] {
            let mut cache = SampleCache::with_max_bytes(max);
            cache.load(&song, &|p| {
                Some(if *p == user("small.wav") {
                    wav(&[1, 2, 3, 4])
                } else {
                    SINE_SF2.to_vec()
                })
            });
            assert_eq!(cache.soundfont(&a).is_some(), fits, "cap {max}");
        }
    }

    #[test]
    fn an_unused_soundfont_is_pruned_and_its_budget_freed() {
        let len = SINE_SF2.len();
        let [a, b] = ["My SoundFonts/a.sf2", "My SoundFonts/b.sf2"].map(user);
        let read = |_: &ContentPath| Some(SINE_SF2.to_vec());
        let mut cache = SampleCache::with_max_bytes(len);
        cache.load(&soundfont_song(a.clone()), &read);
        assert_eq!(cache.room(), 0);
        let held = cache.soundfont(&a).unwrap();
        // A song with no SoundFont Player: the cache lets go of the file's bytes.
        cache.load(&pads(&[]), &read);
        assert!(cache.soundfont(&a).is_none());
        assert_eq!((cache.room(), cache.missing().count()), (len, 0));
        // The audio thread's `Arc` keeps it playing.
        assert_eq!((held.presets().len(), Arc::strong_count(&held)), (1, 1));
        // A swap at the cap, in one load: `b` takes the room `a` leaves.
        cache.load(&soundfont_song(a.clone()), &read);
        cache.load(&soundfont_song(b.clone()), &read);
        assert!(cache.soundfont(&a).is_none() && cache.soundfont(&b).is_some());
        assert_eq!(cache.missing().count(), 0);
    }

    #[cfg(unix)]
    #[test]
    fn the_disk_reader_skips_a_fifo_zone_file_and_a_file_over_the_cap() {
        use std::time::Duration;
        // The worker reads what a song names, so the reader has the guards the app's has: a FIFO would block it for
        // good (no writer), and a file over the cap would fill memory.
        let dir = Removed(
            std::env::temp_dir().join(format!("honeydew-e14c-guards-{}", std::process::id())),
        );
        let root = &dir.0;
        let _ = std::fs::remove_dir_all(root);
        std::fs::create_dir_all(root.join("My Programs/Keys")).unwrap();
        std::fs::write(root.join("My Programs/Keys.dwp"), keys_program()).unwrap();
        let fifo = root.join("My Programs/Keys/Keys C4.wav");
        let made = std::process::Command::new("mkfifo").arg(&fifo).status();
        assert!(made.unwrap().success(), "mkfifo");
        // Sparse: it takes no disk space.
        let huge = std::fs::File::create(root.join("Huge.wav")).unwrap();
        huge.set_len(MAX_FILE_BYTES + 1).unwrap();
        let read = disk_reader(&ProjectFiles::on_disk(root)).expect("files on disk");
        // On a thread, so that a reader that opens the FIFO fails this test and does not hang it.
        let (tx, rx) = std::sync::mpsc::channel();
        let keys = user("My Programs/Keys.dwp");
        let program = keys.clone();
        std::thread::spawn(move || {
            let mut cache = SampleCache::default();
            cache.load_program(&program, &read);
            drop(tx.send((cache, read(&user("Huge.wav")))));
        });
        let (cache, huge) = rx
            .recv_timeout(Duration::from_secs(20))
            .expect("the disk reader blocked on a FIFO");
        assert!(
            huge.is_none(),
            "a file over the cap read {} bytes",
            huge.map_or(0, |b| b.len())
        );
        assert_eq!(cache.program(&keys).expect("the program").len(), 3);
        // The FIFO zone file and the absent one are missing; the embedded zone decoded.
        let at = |name: &str| SampleKey::File(user(&format!("My Programs/Keys/{name}")));
        let missing: Vec<_> = cache.missing().cloned().collect();
        assert_eq!(missing.len(), 2, "{missing:?}");
        assert!(missing.contains(&at("Keys C4.wav")) && missing.contains(&at("Gone.wav")));
        assert_eq!(
            cache.get(&SampleKey::Embedded(keys, 1)).map(|s| s.frames()),
            Some(4)
        );
    }

    // ---- What a worker decoded comes in under the byte budget, and never over what is held ----

    #[test]
    fn an_absorbed_sample_over_the_byte_cap_is_missing() {
        let key = SampleKey::File(user("My Samples/A.wav"));
        let bytes = wav(&[0, 9000, 0, -9000]); // 4 mono frames: 16 bytes of f32
        // The app's own load of it does not fit a cap of 8 bytes: missing.
        let mut direct = SampleCache::with_max_bytes(8);
        direct.load_sample(key.clone(), false, &|_| Some(bytes.clone()));
        assert!(
            direct.get(&key).is_none(),
            "own load: over the cap, missing"
        );
        // The same sample decoded on a worker's cache, then absorbed: missing too.
        let mut worker = SampleCache::default();
        worker.load_sample(key.clone(), false, &|_| Some(bytes.clone()));
        let mut app = SampleCache::with_max_bytes(8);
        app.absorb(worker);
        assert!(app.get(&key).is_none(), "absorbed past the byte cap");
        assert!(app.missing().any(|k| *k == key));
    }

    #[test]
    fn an_absorbed_table_over_the_byte_cap_is_missing() {
        let key = SampleKey::File(user("My Samples/A.wav"));
        let bytes = wav(&[0, 9000, 0, -9000]);
        let mut worker = SampleCache::default();
        worker.load_sample(key.clone(), true, &|_| Some(bytes.clone()));
        assert!(worker.shape(&key).is_some());
        let mut app = SampleCache::with_max_bytes(TABLE_BYTES - 1);
        app.absorb(worker);
        assert!(
            app.shape(&key).is_none(),
            "a table absorbed past the budget"
        );
        assert!(app.missing().any(|k| *k == key));
    }

    #[test]
    fn an_absorbed_render_over_the_byte_cap_is_missing_and_one_that_fits_is_kept() {
        let key = SampleKey::File(user("My Samples/A.wav"));
        let bytes = wav(&[0, 9000, 0, -9000]);
        let mut slot = SamplerSlot::default();
        slot.file.path = Some(user("My Samples/A.wav"));
        (slot.file.stretch_mode, slot.file.stretch_ratio) = (1, 2.0);
        let worker = || {
            let mut worker = SampleCache::default();
            worker.render_slot(&slot, &|_| Some(bytes.clone()));
            assert!(worker.slot(&slot).is_some());
            worker
        };
        // Room for the sample (16 bytes), not its render.
        let mut app = SampleCache::with_max_bytes(16);
        app.absorb(worker());
        assert!(app.get(&key).is_some() && app.slot(&slot).is_none());
        let mut roomy = SampleCache::default();
        roomy.absorb(worker());
        assert!(roomy.slot(&slot).is_some());
        // The build that follows has it: nothing renders on this thread.
        let mut song = Song::default();
        song.add_channel(ChannelKind::AudioClip);
        let lane = &mut song.channels[1].lanes[0];
        lane.add_clip(0, 4.0).file.sample = Some(Box::new(slot.clone()));
        roomy.load(&song, &|path| panic!("the build read {path:?}"));
        assert_eq!(roomy.renders(), 0);
    }

    #[test]
    fn absorbing_a_sample_drops_what_was_made_of_the_old_one() {
        // The pick read its file afresh, so the old sample's waveform summary and stretch render go with it.
        let key = SampleKey::File(user("My Samples/A.wav"));
        let mut slot = SamplerSlot::default();
        slot.file.path = Some(user("My Samples/A.wav"));
        (slot.file.stretch_mode, slot.file.stretch_ratio) = (1, 2.0);
        let mut app = SampleCache::default();
        let old = |_: &ContentPath| Some(wav(&[0, 9000]));
        app.render_slot(&slot, &old);
        let summary = app.waveform(&slot, &old).unwrap();
        assert_eq!(summary.data.frames(), 2);
        let mut worker = SampleCache::default();
        worker.load_sample(key.clone(), false, &|_| Some(wav(&[0, 9000, 0, -9000])));
        app.absorb(worker);
        assert_eq!(app.get(&key).map(|s| s.frames()), Some(4));
        assert!(app.slot(&slot).is_none(), "the old render went");
        let gone = |_: &ContentPath| -> Option<Vec<u8>> { panic!("decoded already") };
        assert_eq!(app.waveform(&slot, &gone).map(|w| w.data.frames()), Some(4));
    }

    /// The one job `stream_slot(slot)` starts in `cache`.
    fn stream_job(cache: &mut SampleCache, slot: &SamplerSlot) -> StretchJob {
        cache.stream_slot(slot);
        let mut jobs = cache.take_jobs();
        assert_eq!(jobs.len(), 1, "a render to make");
        jobs.pop().unwrap()
    }

    /// An audio clip slot on the built-in Kick stretched ×2, and a song playing it.
    fn kick_x2() -> (SamplerSlot, Song) {
        let mut slot = SamplerSlot {
            folder: "Built-in/Kick".into(),
            ..SamplerSlot::default()
        };
        (slot.file.stretch_mode, slot.file.stretch_ratio) = (1, 2.0);
        let mut song = Song::default();
        song.add_channel(ChannelKind::AudioClip);
        let lane = &mut song.channels[1].lanes[0];
        lane.add_clip(0, 4.0).file.sample = Some(Box::new(slot.clone()));
        (slot, song)
    }

    #[test]
    fn a_stretch_job_goes_in_only_while_the_cache_holds_its_stream_and_over_the_budget_as_missing()
    {
        // A pad on the built-in Kick, stretched ×2: its job renders from the cached sample.
        let key = SampleKey::Builtin("Built-in/Kick".into());
        let (slot, song) = kick_x2();
        let mut cache = SampleCache::default();
        cache.load_sample(key.clone(), false, &|_| None);
        let bytes = pcm_bytes(&cache.get(&key).expect("decoded"));
        // The file's sample changed since the job started (a re-pick of the same path): the render is not its.
        let job = stream_job(&mut cache, &slot).render();
        cache.forget(&key);
        cache.load_sample(key.clone(), false, &|_| None);
        assert!(!cache.put_stretch(job));
        assert!(cache.slot(&slot).is_none());
        let job = stream_job(&mut cache, &slot).render();
        assert!(cache.put_stretch(job));
        assert!(cache.slot(&slot).is_some());
        // A load where no slot plays the ratio any more (the slot moved on) lets the stream go, and its job with it.
        let mut moved = SampleCache::default();
        moved.load_sample(key.clone(), false, &|_| None);
        let job = stream_job(&mut moved, &slot).render();
        moved.load_live(&Song::default(), &Song::default(), &|_| None);
        moved.load_sample(key.clone(), false, &|_| None);
        assert!(!moved.put_stretch(job), "its stream is gone");
        assert!(moved.pad_sample(&slot).is_none());
        // The song still playing it keeps it.
        let job = stream_job(&mut moved, &slot).render();
        moved.load_live(&song, &song, &|_| None);
        assert!(moved.put_stretch(job));
        assert_eq!(
            cache.renders() + moved.renders(),
            0,
            "none on this thread's cache walk"
        );
        // Room for the sample but not its twice-as-long render: missing at once, with nothing to stream in.
        let mut cache = SampleCache::with_max_bytes(bytes + 64);
        cache.load_sample(key.clone(), false, &|_| None);
        assert!(cache.get(&key).is_some());
        cache.stream_slot(&slot);
        assert!(cache.take_jobs().is_empty());
        assert!(cache.pad_sample(&slot).is_none());
        assert!(cache.missing().any(|missing| *missing == key));
    }

    #[test]
    fn a_stretch_job_streams_its_first_8192_frames_into_the_cache_at_once_and_the_rest_on_the_worker()
     {
        // The pad plays the new, partly filled buffer at once; the rest streams into that same buffer on a worker,
        // whose whole render then replaces it.
        use honeydew_audio::drums::{PadSample, STREAM_FRAMES};
        let key = SampleKey::Builtin("Built-in/Kick".into());
        let (slot, song) = kick_x2();
        let mut cache = SampleCache::default();
        cache.load_sample(key.clone(), false, &|_| None);
        let whole = stretch(&cache.get(&key).expect("decoded"), 2.0, 0.0);
        assert!(whole.frames() > 2 * STREAM_FRAMES);
        let job = stream_job(&mut cache, &slot);
        let Some(PadSample::Streaming(stream)) = cache.pad_sample(&slot) else {
            panic!("the pad plays the stream");
        };
        assert_eq!(stream.frames(), whole.frames());
        assert_eq!(
            stream.filled(),
            STREAM_FRAMES,
            "only the first frames, at once"
        );
        assert!(cache.slot(&slot).is_none());
        // Shared, not rendered again: by a second job, or by a live load.
        cache.stream_slot(&slot);
        assert!(cache.take_jobs().is_empty());
        cache.load_live(&song, &song, &|_| None);
        assert_eq!(cache.renders(), 0);
        let still = cache.pad_sample(&slot);
        assert!(matches!(still, Some(PadSample::Streaming(s)) if Arc::ptr_eq(&s, &stream)));
        // The rest, on a worker: the same buffer fills, and the whole render goes in in its place.
        let job = std::thread::spawn(move || job.render()).join().unwrap();
        assert_eq!(stream.filled(), whole.frames());
        assert!(cache.put_stretch(job));
        let Some(PadSample::Whole(render)) = cache.pad_sample(&slot) else {
            panic!("the whole render");
        };
        assert_eq!(*render, whole);
    }

    #[test]
    fn a_live_load_streams_a_missing_render_and_any_other_load_needs_it_whole() {
        // An undo to another ratio, a drumset or song load renders no whole buffer on the UI thread; an export's load
        // (and a worker's) renders one still streaming in, in its place.
        use honeydew_audio::drums::PadSample;
        let (slot, song) = kick_x2();
        let mut cache = SampleCache::default();
        cache.load_live(&song, &song, &|_| None);
        assert_eq!(cache.renders(), 0);
        assert!(matches!(
            cache.pad_sample(&slot),
            Some(PadSample::Streaming(_))
        ));
        let mut jobs = cache.take_jobs();
        assert_eq!(jobs.len(), 1);
        let mut export = cache.clone();
        assert!(
            export.take_jobs().is_empty(),
            "the jobs are the live cache's"
        );
        export.load(&song, &|_| None);
        assert_eq!(export.renders(), 1);
        let Some(PadSample::Whole(whole)) = export.pad_sample(&slot) else {
            panic!("whole for the export");
        };
        let job = jobs.pop().unwrap().render();
        assert!(cache.put_stretch(job));
        assert_eq!(cache.slot(&slot), Some(whole));
    }

    #[test]
    fn a_stretch_job_whose_stream_nobody_holds_stops_rendering() {
        // Quick increments or undo steps each start a render; one the cache and every engine have let go of
        // stops at its next chunk, with no render to land.
        let (slot, song) = kick_x2();
        let mut cache = SampleCache::default();
        cache.load_live(&song, &song, &|_| None);
        let mut jobs = cache.take_jobs();
        let stream = match cache.pad_sample(&slot) {
            Some(PadSample::Streaming(stream)) => Arc::downgrade(&stream),
            _ => panic!("streaming"),
        };
        // No slot plays the ratio any more: the next load lets the stream go.
        cache.load_live(&Song::default(), &Song::default(), &|_| None);
        let job = jobs.pop().unwrap().render();
        let stream = stream.upgrade().expect("the job's own");
        assert_eq!(
            stream.filled(),
            STREAM_FRAMES,
            "stopped before its first chunk"
        );
        assert!(!cache.put_stretch(job));
        // One the cache still holds renders to its end.
        cache.load_live(&song, &song, &|_| None);
        let job = cache.take_jobs().pop().unwrap().render();
        assert!(cache.put_stretch(job));
    }

    #[test]
    fn a_stream_that_fills_the_byte_budget_exactly_fits_and_one_byte_more_is_missing() {
        // The budget check before anything renders counts the stream's bytes as it will hold them.
        let bytes = wav(&vec![9000; 20_000]);
        let mut slot = SamplerSlot::default();
        slot.file.path = Some(user("My Samples/A.wav"));
        (slot.file.stretch_mode, slot.file.stretch_ratio) = (1, 2.0);
        // The sample's 80 000 bytes and its x2 stream's 160 000.
        let streamed = |max_bytes: usize| {
            let mut cache = SampleCache::with_max_bytes(max_bytes);
            cache.load_slot(&slot, &|_| Some(bytes.clone()));
            cache.stream_slot(&slot);
            let Some(PadSample::Streaming(stream)) = cache.pad_sample(&slot) else {
                assert!(cache.take_jobs().is_empty());
                return None;
            };
            assert_eq!(cache.take_jobs().len(), 1);
            Some(stream.byte_len())
        };
        assert_eq!(streamed(240_000), Some(160_000));
        assert_eq!(streamed(239_999), None);
    }

    #[test]
    fn absorbing_a_stream_over_the_byte_cap_takes_no_job_for_it() {
        // The pick's stream that does not fit is missing, and its rest is not rendered.
        let key = SampleKey::File(user("My Samples/A.wav"));
        let bytes = wav(&vec![9000; 20_000]);
        let mut slot = SamplerSlot::default();
        slot.file.path = Some(user("My Samples/A.wav"));
        (slot.file.stretch_mode, slot.file.stretch_ratio) = (1, 2.0);
        let worker = || {
            let mut worker = SampleCache::default();
            worker.load_slot(&slot, &|_| Some(bytes.clone()));
            worker.stream_slot(&slot);
            worker
        };
        // Room for the sample (80 000 bytes), not its stream.
        let mut app = SampleCache::with_max_bytes(100_000);
        app.absorb(worker());
        assert!(app.get(&key).is_some());
        assert!(app.take_jobs().is_empty());
        assert!(app.pad_sample(&slot).is_none());
        assert!(app.missing().any(|k| *k == key));
        let mut roomy = SampleCache::default();
        roomy.absorb(worker());
        assert_eq!(roomy.take_jobs().len(), 1);
    }
}
