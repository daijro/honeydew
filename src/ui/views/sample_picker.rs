//! The sample picker: the drum row menu's `Load Sample` and the drum view's `+` add-drum button, in either drum editor;
//! and the audio clip editor's sample row icon button.
//!
//! - The side browser panel of [`PresetPicker`], titled [`TITLE`]. Its root lists `My Files`, the user folder, which
//!   holds `My Samples` and `My Recordings`; each lists its folders, then its audio files by name
//!   ([`ProjectFiles::sample_library`]). None are shipped; with imported content in the content folder the root then
//!   lists its `Drum Samples` categories, read-only.
//! - `Load Sample`: the row's slot takes the file as its sample, its path (`PTH1`, a user-root or, for original
//!   content, an app-root shorthand), its folder and its sample name; a custom pad name stays.
//! - `+`: a new slot on the next free pad, with the default slot params and no custom pad name, takes the file. Its
//!   row becomes the current one.
//! - The sample row: the audio clip's slot takes the file; a file with a tempo turns SYNC on ([`load_clip_sample`]).
//! - A tap on an empty cell of the current audio track ([`show_track`]): the first pick makes an audio clip there.
//!   An audio file dropped on the playlist (`ui::file_drag`) decodes and makes its clip the same way ([`drop_decode`],
//!   [`drop_clip`]).
//! - A module's sample ([`show_module`]): `Custom` picked in a 3x Osc Shape menu, or the Slicer's `LOAD SAMPLE` or a
//!   tap on its waveform. The root lists `My Files`, then our own factory folder (3x Osc `Shapes`, Slicer `Loops`:
//!   listed from code, rendered only when played) and imported content's (`3xOsc Shapes`, `Loop Samples`) when it
//!   is there, its folders first at the root as every factory folder is. Ours list in index order (a factory folder
//!   keeps it). The pick sets the module's `CSTM` `PATH`.
//!
//! Every picker stays open after a pick, which applies once it is decoded and shows the sample strip, playing the file
//! once on the preview player (not the channel); the X, a click outside or Escape closes it and keeps the last pick. A
//! pad's or a clip's pick makes no undo step; the audio track's browse makes one, its clip. A module's sample
//! ([`show_module`]) closes on a pick, as the 3x Osc's custom-sample pick does, with no strip: a final edit; the rack
//! is not in its step, but a Slicer's AUTO clip is. Playing the sample in the song is the engine's: a slot with a path
//! plays its file, whatever its folder.
//!
//! A pick decodes on the preset picker's worker ([`decode_pick`]), which reads the file from disk on the desktop (the
//! web's copies are read at the pick), into a cache of its own that the app's takes when the pick applies: a pad's
//! `STR1` and a clip's SYNC stretch render there too, and a Slicer's loop is sliced there, so the engine build that
//! follows decodes, renders and slices nothing. Meanwhile the picked row pulses `LOADING...` (as the preset picker's)
//! and the strip shows the file's name only, its play button, waveform and progress label disabled or hidden; a new
//! pick replaces the one decoding, whose result is dropped; a close waits for it, drawing nothing and taking every
//! click (closing never reverts a pick), and so does a module's picker, which closes on its pick. On the web, which has
//! no threads, the decode runs at the pick and lands in its own frame, as before.
//!
//! [`ProjectFiles::sample_library`]: crate::project_files::ProjectFiles::sample_library

use std::path::{Path, PathBuf};
use std::sync::mpsc::{Receiver, TryRecvError};

use eframe::egui;
use honeydew_audio::device::AudioCommand;
use honeydew_audio::sample::decode;

use super::preset_picker::{WORKER_LOST, block, outcome, spawn};
use super::{AppCtx, Nav, drum_editor};
use crate::audio_engine::fit_audio_clip;
use crate::audio_engine::slicer::{sensitivity, slices, write_auto_clip};
use crate::audio_link::AudioLink;
use crate::content::{LOOPS, ROOT, SAMPLES, SHAPES, content_path, web};
use crate::flm::chunk::{ContentPath, ContentRoot};
use crate::panels::inst::{slicer, three_osc};
use crate::project_files::{DRUM_SAMPLES, ProjectFiles, SAMPLE_EXTS, SAMPLES_FOLDER, load_pick};
use crate::sample_cache::{self, SampleCache, SampleKey, Waveform, disk_reader};
use crate::song::{DrumKit, Lane, LaneKind, ModuleInstance, SamplerSlot, Song};
use crate::ui::drums::PAD_COUNT;
use crate::ui::picker_preview::{Play, SampleStrip};
use crate::ui::preset_picker::{PickerAction, PresetPicker};
use crate::ui::preview::ClipKey;
use crate::ui::sample_row;

/// The picker's root title (the side panel's titles name what they list, as `Drumsets` and `Instruments` do).
pub const TITLE: &str = "Samples";

/// The open picker. It starts afresh on each opening, however it was last closed.
#[derive(Default)]
pub struct State {
    picker: Option<PresetPicker>,
    /// The channel's kit when the picker opened, or as its own last pick left it: another one (an undo under the
    /// picker) closes it, as its pad may be another pad now.
    kit: Option<DrumKit>,
    /// The module's type when a module's picker opened: another one (an undo under the picker) closes it.
    module_type: Option<u16>,
    /// The egui pass it was last drawn in: a gap means it was closed in between.
    pass: u64,
    /// [`ProjectFiles::library_changes`] as it last listed: an `Import` since is listed at once.
    changes: u64,
    /// The playlist audio-track picker's clip, by id in its lane, once its first pick made it.
    made: Option<u32>,
    /// A pick decoding on the worker ([`decode_pick`]), by its path.
    loading: Option<(PathBuf, Receiver<Decoded>)>,
    /// The picker was closed while its pick decodes: it stays, drawing nothing and taking every click, until the pick
    /// lands.
    closing: bool,
}

impl State {
    /// The open picker's sample strip stops ([`super::Slots::stop_samples`]).
    pub(super) fn stop_sample(&mut self) {
        if let Some(picker) = &mut self.picker {
            picker.stop_sample();
        }
    }

    /// A pick is decoding on the worker.
    #[cfg(test)]
    pub(crate) fn loading(&self) -> bool {
        self.loading.is_some()
    }

    /// The sample strip's name, and whether it shows the waveform.
    #[cfg(test)]
    pub(crate) fn strip(&self) -> Option<(&str, bool)> {
        let picker = self.picker.as_ref()?;
        Some((picker.sample_name()?, picker.sample_data().is_some()))
    }
}

/// Starts the picker's frame: one not drawn last frame was closed in between and starts afresh. Returns whether it
/// does.
fn opening(state: &mut State, ui: &egui::Ui) -> bool {
    let pass = ui.ctx().cumulative_pass_nr();
    if state.pass + 1 != pass {
        state.picker = None;
        state.made = None;
        (state.loading, state.closing) = (None, false);
    }
    state.pass = pass;
    state.picker.is_none() && !state.closing
}

/// What the worker made of a picked sample file ([`decode_pick`]).
pub(crate) struct Decoded {
    /// A cache of its own holding the file's sample, or the 3x Osc Custom table built from it, for the app's to take
    /// once the pick applies ([`SampleCache::absorb`]).
    pub samples: SampleCache,
    /// The sample and its waveform, for the sample strip: `None` for a table, or a file that does not decode.
    pub wave: Option<Waveform>,
    /// Why the file does not read or decode; `None` when it does, or is ours.
    pub error: Option<String>,
}

#[cfg(test)]
thread_local! {
    /// The picks [`decode_pick`] decoded on this thread: the UI thread's (a test's own) stays 0.
    static DECODES: std::cell::Cell<usize> = const { std::cell::Cell::new(0) };
}

/// How many picks [`decode_pick`] decoded on this thread.
#[cfg(test)]
pub(crate) fn decodes_here() -> usize {
    DECODES.get()
}

/// What a pick goes into, as the pick will leave it: what the worker makes ready with the decode ([`decode_pick`]).
pub(super) enum PickFor {
    /// A pad's slot, the file given ([`load_sample`]): its `STR1` stretch and pitch's first frames render, and the rest
    /// streams in once the pick applies.
    Pad(SamplerSlot),
    /// A clip's or track's slot, the file given, with the song's BPM: SYNCed to the file's tempo
    /// ([`load_clip_sample`]), then its stretch renders.
    Clip(SamplerSlot, f64),
    /// A Slicer at this Sensitivity: the loop's slices, which the slice memo keeps by the loop's `Arc`, the one the
    /// app's cache takes ([`slices`]).
    Slicer(f32),
}

/// Decodes a picked file's `bytes` (empty for ours, which render from code) as the sample `key` ([`sample_key`]), or
/// with `shape` as a 3x Osc Custom table, into a cache of its own, and makes ready what the pick goes `into`. Runs on
/// the worker.
// ponytail: the cache's reader copies the file's bytes (2× a big file, briefly, on the worker); hand them over once if
// that ever matters.
pub(super) fn decode_pick(
    key: SampleKey,
    bytes: Result<Vec<u8>, String>,
    shape: bool,
    into: Option<PickFor>,
) -> Decoded {
    #[cfg(test)]
    DECODES.set(DECODES.get() + 1);
    let mut samples = SampleCache::default();
    let read = |_: &ContentPath| bytes.as_ref().ok().cloned();
    samples.load_sample(key.clone(), shape, &read);
    let wave = samples.get(&key).map(Waveform::new);
    match (into, &wave) {
        // The pad plays its first frames at once, and the rest streams in once the pick applies (its
        // job goes with `samples`), as a `Stretch` re-import's.
        (Some(PickFor::Pad(slot)), Some(_)) => {
            samples.load_slot(&slot, &read);
            samples.stream_slot(&slot);
        }
        (Some(PickFor::Clip(mut slot, bpm)), Some(wave)) => {
            sync(&mut slot, wave.data.meta.tempo, bpm);
            samples.render_slot(&slot, &read);
        }
        (Some(PickFor::Slicer(sens)), Some(wave)) => drop(slices(&wave.data, sens)),
        _ => {}
    }
    let error = match bytes {
        _ if wave.is_some() || samples.shape(&key).is_some() => None,
        Err(err) => Some(err),
        Ok(bytes) => decode(&bytes).err().map(|err| format!("{err:?}")),
    };
    Decoded {
        samples,
        wave,
        error,
    }
}

/// The sample a pick of `path` plays: its shorthand's ([`module_path`], [`SampleKey::of_path`]); for a file outside the
/// folders, which no pick stores, its path.
pub(super) fn sample_key(files: &ProjectFiles, path: &Path) -> SampleKey {
    let plain = || ContentPath::Plain(path.to_string_lossy().into_owned());
    SampleKey::of_path(&module_path(files, path).unwrap_or_else(plain))
}

/// The worker's read of the pick `path`. The desktop's reads it through [`disk_reader`], as the DW Sampler's worker
/// does its zones: a user or content file from disk, our bundled member from its pack. Without one, the web's copy is
/// read now, and so is our bundled member, downloaded ([`web`]: one not in memory failed to download); a code-made file
/// of ours has no bytes: it renders from code ([`honeydew_audio::builtin_drums::builtin`]), which wins over any bytes
/// anyway.
fn pick_bytes(
    files: &ProjectFiles,
    path: &Path,
) -> impl FnOnce() -> Result<Vec<u8>, String> + Send + 'static {
    let disk = disk_reader(files).zip(module_path(files, path));
    let now = match (content_path(path), &disk) {
        (_, Some(_)) => None,
        (Some(ours), None) => Some(match sample_cache::reader(files)(&ours) {
            Some(bytes) => Ok(bytes),
            None if web::lost(files.factory, path) => Err(web::DOWNLOAD_FAILED.to_owned()),
            None => Ok(Vec::new()),
        }),
        (None, None) => Some(files.read_user_file(path)),
    };
    let read = || disk.and_then(|(read, file)| read(&file));
    move || now.unwrap_or_else(|| read().ok_or_else(|| UNREAD.to_owned()))
}

/// What the picker did this frame.
enum Shown {
    /// It stays open, with no pick landed.
    Open,
    /// It closed (the X, a click outside, Escape).
    Closed,
    /// A pick's decode landed; the picker stays open, its sample strip playing the pick.
    Picked(PathBuf, Box<Decoded>),
}

/// Shows the picker (a fresh one is `make`'s), modal, over the left of the screen, listing
/// [`ProjectFiles::sample_library`]: `Import` asks for a file, which it lists once it is in. A pick decodes on the
/// worker (as a 3x Osc table with `shape`), with what it goes `into` (of the song, the file and the files) made ready
/// there too, its row pulsing, and is [`Shown::Picked`] once it lands. With `strip`, it shows the sample strip, which
/// plays it on the preview player ([`preview`]) once decoded; without (a module's sample), the picker closes on the
/// pick, waiting for it.
fn pick(
    state: &mut State,
    ui: &egui::Ui,
    app: &mut AppCtx,
    make: impl FnOnce(&ProjectFiles) -> PresetPicker,
    (strip, shape): (bool, bool),
    into: impl FnOnce(&Song, &Path, &ProjectFiles) -> Option<PickFor>,
) -> Shown {
    let ctx = ui.ctx();
    let files = &mut *app.files;
    let action = match state.closing {
        true => None,
        false => {
            let picker = state.picker.get_or_insert_with(|| make(files));
            let changes = files.library_changes();
            if std::mem::replace(&mut state.changes, changes) != changes {
                picker.reload();
            }
            picker.set_loading(state.loading.is_some());
            picker.show(ctx, &app.cx, &*files.sample_library())
        }
    };
    match action {
        Some(PickerAction::Import) => {
            if let Some(user) = files.user_dir() {
                files.import_audio(user.join(SAMPLES_FOLDER));
            }
        }
        Some(PickerAction::Close) => {
            close_waiting(state, app.audio);
            if !state.closing {
                return Shown::Closed;
            }
        }
        Some(PickerAction::Pick(path)) => {
            let (key, read) = (sample_key(files, &path), pick_bytes(files, &path));
            let into = into(app.song, &path, files);
            // It replaces a pick still decoding, whose result is dropped.
            let done = spawn(move || decode_pick(key, read(), shape, into));
            if let (true, Some(picker)) = (strip, &mut state.picker) {
                show_strip(picker, &path, None);
            }
            state.loading = Some((path, done));
            if !strip {
                // A module's sample closes on the pick, as the 3x Osc's custom-sample pick does.
                close_waiting(state, app.audio);
            }
        }
        Some(PickerAction::Default | PickerAction::Back) | None => {}
    }
    let arrived = match state.loading.as_ref().map(|(_, done)| done.try_recv()) {
        None => None,
        Some(Err(TryRecvError::Empty)) => {
            match state.closing {
                true => block(ctx),
                false => ctx.request_repaint(),
            }
            None
        }
        Some(done) => {
            let lost = |_| Decoded {
                samples: SampleCache::default(),
                wave: None,
                error: Some(WORKER_LOST.into()),
            };
            let (path, _) = state.loading.take().expect("loading");
            Some((path, done.unwrap_or_else(lost)))
        }
    };
    if let Some(picker) = &mut state.picker {
        if let (true, Some((path, decoded))) = (strip, &arrived) {
            show_strip(picker, path, decoded.wave.clone());
        }
        preview(picker, app.audio);
    }
    arrived.map_or(Shown::Open, |(path, decoded)| {
        Shown::Picked(path, Box::new(decoded))
    })
}

/// A pick's error when the desktop's worker cannot read its file.
const UNREAD: &str = "The file could not be read.";

/// The picker closes, but a pick still decoding lands first: it is dropped, its sample stopped, and waits, drawing
/// nothing and taking every click (closing never reverts a pick).
fn close_waiting(state: &mut State, audio: &mut AudioLink) {
    let loading = state.loading.take();
    close(state, audio);
    (state.closing, state.loading) = (loading.is_some(), loading);
}

/// After a pick applied: a picker closed while it decoded closes now.
fn landed(state: &mut State, audio: &mut AudioLink) -> Option<Nav> {
    state.closing.then(|| {
        close(state, audio);
        Nav::Close
    })
}

/// Shows the sample strip for the pick `path` in `picker`: its name, and once it is decoded (`wave`) its waveform,
/// which plays from the start. Any sample playing stops.
pub(super) fn show_strip(picker: &mut PresetPicker, path: &Path, wave: Option<Waveform>) {
    let name = path.file_stem().unwrap_or_default().to_string_lossy();
    picker.set_sample(SampleStrip::new(name.into_owned(), wave));
}

/// Sends what `picker`'s sample strip asks of the preview player.
pub(super) fn preview(picker: &mut PresetPicker, audio: &mut AudioLink) {
    for play in picker.take_plays() {
        let sample = match play {
            Play::From(from) => match picker.sample_data() {
                Some(data) => Some((data, from)),
                None => continue,
            },
            Play::Stop => None,
        };
        audio.send(AudioCommand::Preview(sample));
    }
}

/// The picker closes: its sample stops, and it is dropped, with a pick still decoding.
fn close(state: &mut State, audio: &mut AudioLink) {
    if let Some(picker) = &mut state.picker {
        picker.closing();
        preview(picker, audio);
    }
    state.picker = None;
    (state.loading, state.closing) = (None, false);
}

/// A fresh picker over the user folder, our bundled `Samples` and imported content's `Drum Samples` in `files`,
/// with `Import` where `files` takes one ([`ProjectFiles::imports_audio`]): into `My Samples`.
pub(crate) fn picker(files: &ProjectFiles) -> PresetPicker {
    let user = files.user_dir().map(Path::to_owned);
    let ours = Path::new(ROOT).join(SAMPLES);
    PresetPicker::new(TITLE, user, SAMPLE_EXTS[0], false)
        .of_exts(&SAMPLE_EXTS)
        .with_factory(std::iter::once(ours).chain(files.content_folder(DRUM_SAMPLES)))
        .with_import(files.imports_audio())
}

/// Draws the picker for `channel`'s pad `pad` (`None`: a new pad, which the first pick adds and later picks reload),
/// modal, over the left of the screen, opening at the folder of the pad's sample with its row in view (at the root
/// without one), and gives the pad each pick once it is decoded, with no undo step; the pad becomes the drum editor's
/// (`drums`) current row. It stays open after a pick; the X, a click outside, Escape, or the channel's kit being gone
/// or changed by anything else close it. The app re-keys `channel` when channels move.
pub fn show(
    state: &mut State,
    drums: &mut drum_editor::State,
    ui: &mut egui::Ui,
    app: &mut AppCtx,
    channel: usize,
    pad: &mut Option<u8>,
) -> Option<Nav> {
    let kit = (app.song.channels.get(channel)).and_then(|c| c.drum_kit.as_ref());
    let mut start = None;
    if opening(state, ui) {
        state.kit = kit.cloned();
        // A pad with a sample opens at its folder, its row in view; an empty pad at the root, and so does a built-in
        // one-shot, which no folder lists.
        let path = pad.and_then(|p| kit?.slots.get(usize::from(p))?.file.path.as_ref());
        start = path.and_then(|p| app.files.resolve(p));
    }
    if kit.is_none() || kit != state.kit.as_ref() {
        close(state, app.audio);
        return Some(Nav::Close);
    }
    // The pad's slot as the pick will leave it.
    let at = pad.map(usize::from);
    let slot = |song: &Song, path: &Path, files: &ProjectFiles| {
        let mut kit = song.channels.get(channel)?.drum_kit.clone()?;
        let row = load_sample(&mut kit, at, path, files)?;
        Some(PickFor::Pad(kit.slots.swap_remove(row)))
    };
    let make = |files: &ProjectFiles| {
        let mut picker = picker(files);
        if let Some(file) = &start {
            picker.reveal(&*files.sample_library(), file);
        }
        picker
    };
    let (path, decoded) = match pick(state, ui, app, make, (true, false), slot) {
        Shown::Open => return None,
        Shown::Closed => return Some(Nav::Close),
        Shown::Picked(path, decoded) => (path, *decoded),
    };
    let kit = app.song.channels[channel].drum_kit.as_mut();
    if let Some(kit) = kit
        && let Some(row) = load_sample(kit, pad.map(usize::from), &path, app.files)
    {
        // Rack state: no undo step. A pick into a pad that was there (the pad icon's) sets no song-changed flag; Add
        // Drum's pick, which adds the pad, sets it, and its later picks reload that pad with the flag already set.
        if pad.is_none() {
            app.files.song_changed();
        }
        *pad = u8::try_from(row).ok();
        state.kit = Some(kit.clone());
        drums.select_row(app.song, row);
        // A new pick tries the samples missing before again, and has the one the worker decoded.
        app.audio.samples.forget_missing();
        app.audio.samples.absorb(decoded.samples);
        app.pick_loaded();
    }
    landed(state, app.audio)
}

/// Draws the picker for the sampler slot of the audio clip `key` (the audio clip editor's sample row icon button), as
/// [`show`] does for a pad, and gives the slot each pick ([`load_clip_sample`]) with the file's tempo as the decode
/// reads it; a clip without a slot gets the default one first. No undo step. The X, a click outside, Escape, or the
/// clip being gone close it. The app re-keys `key` when channels or tracks move.
pub fn show_clip(
    state: &mut State,
    ui: &mut egui::Ui,
    app: &mut AppCtx,
    key: ClipKey,
) -> Option<Nav> {
    opening(state, ui);
    if key.clip(app.song).is_none() {
        close(state, app.audio);
        return Some(Nav::Close);
    }
    let slot = |song: &Song, path: &Path, files: &ProjectFiles| {
        let clip = key.clip(song)?;
        clip_pick(clip.file.sample.as_deref().cloned(), song, path, files)
    };
    let (path, decoded) = match pick(state, ui, app, picker, (true, false), slot) {
        Shown::Open => return None,
        Shown::Closed => return Some(Nav::Close),
        Shown::Picked(path, decoded) => (path, *decoded),
    };
    let tempo = decoded.wave.as_ref().and_then(|w| w.data.meta.tempo);
    let bpm = app.song.tempo_bpm;
    if let Some(clip) = sample_row::clip_mut(app.song, key) {
        let mut slot = clip.file.sample.as_deref().cloned().unwrap_or_default();
        if load_clip_sample(&mut slot, &path, app.files, tempo, bpm) {
            clip.file.sample = Some(Box::new(slot));
            app.audio.samples.forget_missing();
            app.audio.samples.absorb(decoded.samples);
            // A change.
            app.files.song_changed();
            app.pick_loaded();
        }
    }
    landed(state, app.audio)
}

/// Draws the picker a tap on an empty cell of the current audio track opens, for `channel`'s audio lane `lane` at song
/// tick `tick`. The first pick makes an audio clip there holding it ([`load_clip_sample`], fitted to the sample:
/// [`fit_audio_clip`]), one undo step; each later pick reloads that clip, with none, so the whole browse is one clip
/// and one step. It stays open; the X, a click outside, Escape, or the lane being gone or no audio lane close it. The
/// app re-keys `channel` and `lane` when they move.
pub fn show_track(
    state: &mut State,
    ui: &mut egui::Ui,
    app: &mut AppCtx,
    (channel, lane, tick): (usize, usize, u32),
) -> Option<Nav> {
    opening(state, ui);
    let audio = (app.song.channels.get(channel)).and_then(|c| c.lanes.get(lane));
    if audio.is_none_or(|l| l.kind != LaneKind::Audio) {
        close(state, app.audio);
        return Some(Nav::Close);
    }
    // The slot the pick goes into: the clip the browse made, or a new one's.
    let made = state.made;
    let slot = |song: &Song, path: &Path, files: &ProjectFiles| {
        let clips = &song.channels.get(channel)?.lanes.get(lane)?.clips;
        let clip = made.and_then(|id| clips.iter().find(|c| c.id == id));
        let sample = clip.and_then(|c| c.file.sample.as_deref().cloned());
        clip_pick(sample, song, path, files)
    };
    let (path, decoded) = match pick(state, ui, app, picker, (true, false), slot) {
        Shown::Open => return None,
        Shown::Closed => return Some(Nav::Close),
        Shown::Picked(path, decoded) => (path, *decoded),
    };
    let tempo = decoded.wave.as_ref().and_then(|w| w.data.meta.tempo);
    let (bpm, bar) = (app.song.tempo_bpm, app.song.time_sig.bar_beats());
    let l = &mut app.song.channels[channel].lanes[lane];
    let made = state
        .made
        .and_then(|id| l.clips.iter().position(|c| c.id == id));
    let old = made.and_then(|i| l.clips[i].file.sample.as_deref().cloned());
    let mut slot = old.unwrap_or_default();
    if !load_clip_sample(&mut slot, &path, app.files, tempo, bpm) {
        return landed(state, app.audio);
    }
    app.audio.samples.forget_missing();
    app.audio.samples.absorb(decoded.samples);
    let clip = match made {
        Some(i) => &mut l.clips[i],
        None => l.add_clip(tick, f64::from(bar)),
    };
    clip.file.sample = Some(Box::new(slot));
    fit_audio_clip(clip, &app.audio.samples, bpm, bar);
    state.made = Some(clip.id);
    match made {
        None => app.song_edited(true),
        // A change.
        Some(_) => {
            app.files.song_changed();
            app.pick_loaded();
        }
    }
    landed(state, app.audio)
}

/// Draws the picker for the module with instance id `module` on `channel`, modal, over the left of the screen
/// ([`State`] as [`show`]'s), and gives the module its pick once it is decoded: its `CSTM` `PATH` names the file
/// ([`load_pick`], as the preset picker's shape pick), one final song edit: a new engine, which plays it, and no undo
/// step for the module (the rack is not covered by undo); a Slicer with its AUTO flag on also gets AUTO's clip, an undo
/// step, into `held` (the channel's piano roll clip) first ([`write_auto_clip`]). `Import` keeps a file in `My Samples`
/// and stays open, as [`show`]'s. A pick (once it lands), the X, a click outside, Escape, or the module being gone or
/// now another type close it; only a pick changes the song (a cancelled `Custom` stays, silent without a file). A file
/// that does not decode shows `Failed to load project file` with its error in the picker's place and changes nothing (a
/// failure leaves the old table). The app re-keys `channel` when channels move.
pub fn show_module(
    state: &mut State,
    ui: &mut egui::Ui,
    app: &mut AppCtx,
    channel: usize,
    module: i32,
    held: Option<(usize, u32)>,
) -> Option<Nav> {
    let type_id = app.song.find_module(channel, module).map(|m| m.type_id);
    if opening(state, ui) {
        state.module_type = type_id;
    }
    let Some(type_id) = type_id.filter(|&t| Some(t) == state.module_type) else {
        close(state, app.audio);
        return Some(Nav::Close);
    };
    let make = |files: &ProjectFiles| module_picker(files, type_id);
    // It closes on a pick, as the 3x Osc's custom-sample pick does, so no sample strip: hidden, it waits for the pick
    // to land.
    let shape = type_id == three_osc::TYPE_ID;
    let into = |song: &Song, path: &Path, files: &ProjectFiles| {
        slicer_pick(song.find_module(channel, module)?, path, files)
    };
    let (path, decoded) = match pick(state, ui, app, make, (false, shape), into) {
        Shown::Open => return None,
        Shown::Closed => return Some(Nav::Close),
        Shown::Picked(path, decoded) => (path, *decoded),
    };
    close(state, app.audio);
    // Ours always render; any other file must decode.
    if let Some(err) = decoded.error {
        return Some(outcome(Err(err)));
    }
    let (shorthand, id) = (module_path(app.files, &path), module);
    let module = (app.song.channels.get_mut(channel)).and_then(|c| c.rack.get_mut(id));
    if let Some(module) = module
        && load_pick(module, &path, &[], shorthand).is_ok()
    {
        // A new pick tries the samples missing before again, and has the loop or table the worker made.
        app.audio.samples.forget_missing();
        app.audio.samples.absorb(decoded.samples);
        // A Slicer's new loop is a re-detect: AUTO's clip, the edit's undo step.
        let read = sample_cache::reader(app.files);
        let samples = &mut app.audio.samples;
        write_auto_clip(app.song, channel, id, samples, &read, false, held);
        drop(read);
        app.song_edited(true);
    }
    Some(Nav::Close)
}

/// A clip's or audio track's pick into its slot (`slot`, the default one when it has none): the slot takes the file
/// ([`load_clip_sample`]), to be SYNCed at the song's BPM once the worker reads the file's tempo. `None` when it
/// cannot.
fn clip_pick(
    slot: Option<SamplerSlot>,
    song: &Song,
    path: &Path,
    files: &ProjectFiles,
) -> Option<PickFor> {
    let (mut slot, bpm) = (slot.unwrap_or_default(), song.tempo_bpm);
    load_clip_sample(&mut slot, path, files, None, bpm).then_some(PickFor::Clip(slot, bpm))
}

/// Decodes the audio files `paths` dropped on the playlist (`ui::file_drag`) on the worker ([`spawn`]), each for a new
/// audio clip's slot as an audio track's pick decodes it ([`clip_pick`]); a file the slot does not take is left out.
pub(crate) fn drop_decode(
    song: &Song,
    paths: Vec<PathBuf>,
    files: &ProjectFiles,
) -> Receiver<Vec<(PathBuf, Decoded)>> {
    let jobs: Vec<_> = (paths.iter())
        .filter_map(|path| {
            let into = clip_pick(None, song, path, files)?;
            let (key, read) = (sample_key(files, path), pick_bytes(files, path));
            Some((path.clone(), key, read, into))
        })
        .collect();
    spawn(move || {
        (jobs.into_iter())
            .map(|(path, key, read, into)| (path, decode_pick(key, read(), false, Some(into))))
            .collect()
    })
}

/// An audio clip at `tick` on the audio lane `lane` holding the dropped file `path`, as its decode (`decoded`) left it,
/// as an audio track's first pick makes one ([`show_track`]): fitted to its sample, which `samples` takes. Returns its
/// end tick; `None`, and no clip, for a file the slot does not take or that did not decode.
pub(crate) fn drop_clip(
    lane: &mut Lane,
    tick: u32,
    path: &Path,
    decoded: Decoded,
    files: &ProjectFiles,
    samples: &mut SampleCache,
    (bpm, bar): (f64, u32),
) -> Option<u32> {
    let tempo = decoded.wave.as_ref()?.data.meta.tempo;
    let mut slot = SamplerSlot::default();
    load_clip_sample(&mut slot, path, files, tempo, bpm).then_some(())?;
    samples.forget_missing();
    samples.absorb(decoded.samples);
    let clip = lane.add_clip(tick, f64::from(bar));
    clip.file.sample = Some(Box::new(slot));
    fit_audio_clip(clip, samples, bpm, bar);
    Some(tick.saturating_add(clip.visible_ticks().round() as u32))
}

/// A Slicer `module`'s pick of the loop `path`: the Sensitivity the pick leaves it ([`load_pick`]: its own, or a new
/// module's), at which the worker slices the loop. `None` for another module, or a file it does not take.
pub(super) fn slicer_pick(
    module: &ModuleInstance,
    path: &Path,
    files: &ProjectFiles,
) -> Option<PickFor> {
    let mut module = Some(module)
        .filter(|m| m.type_id == slicer::TYPE_ID)?
        .clone();
    load_pick(&mut module, path, &[], module_path(files, path)).ok()?;
    Some(PickFor::Slicer(sensitivity(&module)))
}

/// A fresh picker for module type `type_id`'s sample: `My Files`, then its factory folders, ours first, then the imported
/// content's when it is there: the 3x Osc's shapes, the Slicer's loops (another type has none); with `Import` into `My
/// Samples` where `files` takes one, as [`picker`].
pub fn module_picker(files: &ProjectFiles, type_id: u16) -> PresetPicker {
    let folders = match type_id {
        three_osc::TYPE_ID => Some((SHAPES, "3xOsc Shapes")),
        slicer::TYPE_ID => Some((LOOPS, "Loop Samples")),
        _ => None,
    };
    let factory = folders.into_iter().flat_map(|(ours, original)| {
        std::iter::once(Path::new(ROOT).join(ours)).chain(files.content_folder(original))
    });
    let user = files.user_dir().map(Path::to_owned);
    PresetPicker::new(TITLE, user, SAMPLE_EXTS[0], false)
        .of_exts(&SAMPLE_EXTS)
        .with_factory(factory)
        .with_import(files.imports_audio())
}

/// The shorthand a module's `PATH` takes for the picked file `path`: our own `<built-in>/<rel>` is `@R@Built-in/<rel>`
/// ([`content_path`]), any other file its shorthand in `files` (`@L@`, or `@R@` for imported content). `None` for a
/// file outside them, which no `PATH` holds.
pub(crate) fn module_path(files: &ProjectFiles, path: &Path) -> Option<ContentPath> {
    content_path(path).or_else(|| files.shorthand(path))
}

/// Gives `slot` the sample file at `path`, in the user folder `user`: [`give_file`] with its user-root shorthand.
/// `None`, leaving `slot` as it was, for a file outside the user folder, which `PTH1` cannot hold.
pub(crate) fn give(slot: &mut SamplerSlot, path: &Path, user: &Path) -> Option<()> {
    let relative = path.strip_prefix(user).ok()?.to_str()?.replace('\\', "/");
    let shorthand = ContentPath::Shorthand {
        root: ContentRoot::User,
        relative,
    };
    give_file(slot, path, shorthand)
}

/// Gives `slot` the sample file at `path`, whose shorthand is `shorthand` (`@L@`, or `@R@` for imported content,
/// [`ProjectFiles::shorthand`]): its sample path, its folder and sample name (the file's, below that root) and its
/// `PRST` (the file's path and `<folder>/<name>`, as a module preset's is). The custom pad name (`MAIN`@1051) is
/// untouched. `None`, leaving `slot` as it was, for a path with no name.
fn give_file(slot: &mut SamplerSlot, path: &Path, shorthand: ContentPath) -> Option<()> {
    let ContentPath::Shorthand { relative, .. } = &shorthand else {
        return None;
    };
    let named = Path::new(relative).with_extension("");
    let name = named.file_name()?.to_str()?.to_owned();
    let folder = named.parent()?.to_str()?.to_owned();
    slot.file.provenance = named.to_str()?.to_owned();
    (slot.name, slot.folder) = (name, folder);
    slot.file.path = Some(shorthand);
    slot.file.preset_path = path.to_string_lossy().replace('\\', "/");
    Some(())
}

/// Gives `kit` the sample file at `path` ([`give_file`], its shorthand in `files`): the slot on pad `pad` takes it, or
/// (`None`) a new slot on the next free pad does. The custom pad name (`MAIN`@1051) is untouched: a new slot has none,
/// a loaded one keeps its own. Returns the pad, or `None` when nothing changed: no slot on `pad`, no free pad of the
/// [`PAD_COUNT`], or a file outside the user and content folders.
pub(crate) fn load_sample(
    kit: &mut DrumKit,
    pad: Option<usize>,
    path: &Path,
    files: &ProjectFiles,
) -> Option<usize> {
    match pad {
        Some(pad) => give_file(kit.slots.get_mut(pad)?, path, files.shorthand(path)?).map(|()| pad),
        None if kit.slots.len() < PAD_COUNT => {
            let mut slot = SamplerSlot::default();
            give_file(&mut slot, path, files.shorthand(path)?)?;
            kit.insert_slot(kit.slots.len(), slot);
            Some(kit.slots.len() - 1)
        }
        None => None,
    }
}

/// Gives an audio clip's `slot` the sample file at `path` ([`give_file`], its shorthand in `files`). A file with a
/// tempo (`tempo`, the decode's `meta.tempo`) turns SYNC on with ratio fileBPM/songBPM (`bpm`). Without one (or without
/// a song tempo) the ratio is 1.0, so the old sample's stretch does not carry over, and SYNC stays as it was. Returns
/// whether the slot took the file.
pub(crate) fn load_clip_sample(
    slot: &mut SamplerSlot,
    path: &Path,
    files: &ProjectFiles,
    tempo: Option<f32>,
    bpm: f64,
) -> bool {
    if files
        .shorthand(path)
        .and_then(|s| give_file(slot, path, s))
        .is_none()
    {
        return false;
    }
    sync(slot, tempo, bpm);
    true
}

/// [`load_clip_sample`]'s SYNC: a file with a tempo (`tempo`) turns it on with ratio fileBPM/songBPM (`bpm`); without
/// one (or without a song tempo) the ratio is 1.0 and SYNC stays as it was.
fn sync(slot: &mut SamplerSlot, tempo: Option<f32>, bpm: f64) {
    slot.file.stretch_ratio = 1.0;
    if let Some(tempo) = tempo
        && bpm > 0.0
    {
        (slot.file.stretch_mode, slot.file.stretch_ratio) = (1, f64::from(tempo) / bpm);
    }
}

#[cfg(test)]
mod tests {
    use std::path::PathBuf;

    use super::*;
    use crate::project_files::mem::MemFiles;
    use crate::ui::drums::row_lines;

    /// The default kit (pads Kick … Crash, ids 1–8), its Snare with its own params.
    fn kit() -> DrumKit {
        let mut kit = DrumKit::default_kit();
        kit.slots[1].params[0] = 0.3;
        kit
    }

    fn user_path(relative: &str) -> ContentPath {
        ContentPath::Shorthand {
            root: ContentRoot::User,
            relative: relative.into(),
        }
    }

    #[test]
    fn a_pick_decodes_into_a_cache_of_its_own_or_says_why_not() {
        use crate::sample_cache::tests::wav;
        let key = SampleKey::File(user_path("My Samples/Wave.wav"));
        let bytes = || Ok(wav(&[0, 9000, 0, -9000]));
        // A sample: the cache holds it, and the strip its waveform.
        let sample = decode_pick(key.clone(), bytes(), false, None);
        assert_eq!(sample.samples.get(&key).map(|s| s.frames()), Some(4));
        assert_eq!(sample.wave.map(|w| w.data.frames()), Some(4));
        assert_eq!(sample.error, None);
        // A 3x Osc shape: its table only.
        let shape = decode_pick(key.clone(), bytes(), true, None);
        assert!(shape.samples.shape(&key).is_some() && shape.samples.get(&key).is_none());
        assert!(shape.wave.is_none() && shape.error.is_none());
        // A file that does not decode, or does not read: why.
        let broken = decode_pick(key.clone(), Ok(b"RIFF".to_vec()), false, None);
        assert!(broken.wave.is_none() && broken.error.is_some());
        let gone = decode_pick(key, Err("gone".into()), true, None);
        assert_eq!(gone.error.as_deref(), Some("gone"));
        // Ours render from code, with no bytes.
        let dust = SampleKey::of_path(&ContentPath::Shorthand {
            root: ContentRoot::App,
            relative: "Built-in/Loops/Dust Break 84.wav".into(),
        });
        let ours = decode_pick(dust, Ok(Vec::new()), false, None);
        assert!(ours.wave.is_some() && ours.error.is_none());
    }

    #[test]
    fn load_sample_gives_the_rows_slot_the_files_path_folder_and_name_and_keeps_its_custom_name() {
        // On the desktop: an absolute path in the user folder.
        let user = PathBuf::from("/home/me/studio");
        let files = ProjectFiles::on_disk(&user);
        let path = user.join("My Samples/Kicks/Big Kick.wav");
        let mut kit = kit();
        let before = kit.clone();
        assert_eq!(load_sample(&mut kit, Some(1), &path, &files), Some(1));
        let slot = &kit.slots[1];
        assert_eq!(
            slot.file.path,
            Some(user_path("My Samples/Kicks/Big Kick.wav"))
        );
        // The pad name lines: the file's folder over its name.
        assert_eq!(row_lines(slot), ("Kicks", "Big Kick"));
        assert_eq!(slot.folder, "My Samples/Kicks");
        // PRST: the source path, and `<folder>/<name>`.
        assert_eq!(
            slot.file.preset_path,
            "/home/me/studio/My Samples/Kicks/Big Kick.wav"
        );
        assert_eq!(slot.file.provenance, "My Samples/Kicks/Big Kick");
        // Nothing else changes: the slot's id and params, and the other slots.
        let mut rest = slot.clone();
        rest.name.clone_from(&before.slots[1].name);
        rest.folder.clone_from(&before.slots[1].folder);
        rest.file.path = None;
        (rest.file.preset_path, rest.file.provenance) = (String::new(), String::new());
        assert_eq!(rest, before.slots[1]);
        assert_eq!(kit.slots.len(), before.slots.len());
        assert_eq!(kit.slots[0], before.slots[0]);
        assert_eq!(kit.file, before.file);
        // A custom name stays, over no second line.
        kit.slots[0].file.custom_pad_name = "Mine".into();
        load_sample(&mut kit, Some(0), &path, &files);
        assert_eq!(kit.slots[0].file.custom_pad_name, "Mine");
        assert_eq!(
            (kit.slots[0].name.as_str(), row_lines(&kit.slots[0])),
            ("Big Kick", ("Mine", ""))
        );
    }

    #[cfg(not(target_arch = "wasm32"))]
    #[test]
    fn the_root_lists_the_original_drum_samples_after_my_files() {
        use crate::project_files::CONTENT_FOLDER;
        use crate::project_files::content::tests::{Temp, fake_tree};
        use crate::ui::preset_picker::MY_FILES;
        let tmp = Temp::new("sample-picker");
        let names = |files: &ProjectFiles, path: &[&str]| {
            picker(files).names_in(&*files.sample_library(), path)
        };
        let files = ProjectFiles::on_disk(&tmp.0);
        assert_eq!(names(&files, &[]), [MY_FILES]);
        fake_tree(&tmp.0.join(CONTENT_FOLDER));
        assert_eq!(names(&files, &[]), [MY_FILES, "Kick", "Snare"]);
        assert_eq!(names(&files, &["Kick"]), ["Gen Kick"]);
        assert_eq!(names(&files, &[MY_FILES]), ["My Recordings", "My Samples"]);
        let mut picked = picker(&files);
        picked.names_in(&*files.sample_library(), &["Snare"]);
        let path = picked.path_of("Gen Snare").unwrap();
        let mut kit = kit();
        assert_eq!(load_sample(&mut kit, Some(0), &path, &files), Some(0));
        assert_eq!(kit.slots[0].folder, "Drum Samples/Snare");
    }

    /// The rows of a 3x Osc's sample picker over `files`, once the folders `path` are opened.
    fn shape_rows(files: &ProjectFiles, path: &[&str]) -> Vec<String> {
        module_picker(files, three_osc::TYPE_ID).names_in(&*files.sample_library(), path)
    }

    #[test]
    fn a_three_osc_picker_lists_my_files_then_our_shapes_and_names_them_built_in() {
        use crate::ui::preset_picker::MY_FILES;
        // The web: no content folder.
        let files = ProjectFiles::with_provider(Box::new(MemFiles::default()));
        let shapes = ["Glass", "Grit", "Reed", "Thin Pulse", "Vowel A", "Vowel O"];
        assert_eq!(shape_rows(&files, &[]), [&[MY_FILES][..], &shapes].concat());
        let mut picked = module_picker(&files, three_osc::TYPE_ID);
        picked.names_in(&*files.sample_library(), &[]);
        let vowel = picked.path_of("Vowel A").unwrap();
        let app = |relative: &str| ContentPath::Shorthand {
            root: ContentRoot::App,
            relative: relative.into(),
        };
        assert_eq!(
            module_path(&files, &vowel),
            Some(app("Built-in/Shapes/Vowel A.wav"))
        );
        // Another module lists no shapes of ours.
        let other = module_picker(&files, 0).names_in(&*files.sample_library(), &[]);
        assert_eq!(other, [MY_FILES]);
        // A user file takes its user path; a file outside the folders none.
        let user = PathBuf::from("/home/me/studio");
        let disk = ProjectFiles::on_disk(&user);
        let mine = user.join("My Samples/Wave.wav");
        assert_eq!(
            module_path(&disk, &mine),
            Some(user_path("My Samples/Wave.wav"))
        );
        assert_eq!(module_path(&disk, Path::new("/tmp/Wave.wav")), None);
    }

    #[cfg(not(target_arch = "wasm32"))]
    #[test]
    fn the_original_3xosc_shapes_are_listed_with_ours() {
        use crate::project_files::CONTENT_FOLDER;
        use crate::project_files::content::tests::{Temp, fake_tree};
        use crate::ui::preset_picker::MY_FILES;
        let tmp = Temp::new("module-sample-picker");
        let files = ProjectFiles::on_disk(&tmp.0);
        fake_tree(&tmp.0.join(CONTENT_FOLDER));
        // The root's folders first (every factory folder before every file), then our shapes.
        let rows = shape_rows(&files, &[]);
        assert_eq!(rows[..2], [MY_FILES, "HQ"]);
        assert_eq!((rows.len(), rows.last()), (2 + 6, Some(&"Vowel O".into())));
        assert_eq!(shape_rows(&files, &["HQ"]), ["Gen Shape"]);
        let mut picked = module_picker(&files, three_osc::TYPE_ID);
        picked.names_in(&*files.sample_library(), &["HQ"]);
        let path = picked.path_of("Gen Shape").unwrap();
        let original = ContentPath::Shorthand {
            root: ContentRoot::App,
            relative: "3xOsc Shapes/HQ/Gen Shape.wav".into(),
        };
        assert_eq!(module_path(&files, &path), Some(original));
    }

    /// Our six loops, in index order.
    const LOOPS: [&str; 6] = [
        "Dust Break 84",
        "Foundry Drive 128",
        "Foundry Rush 172",
        "Pocket Groove 92",
        "Relay Bounce 100",
        "Relay Roll 140",
    ];

    /// The rows of a Slicer's sample picker over `files`, once the folders `path` are opened.
    fn loop_rows(files: &ProjectFiles, path: &[&str]) -> Vec<String> {
        module_picker(files, slicer::TYPE_ID).names_in(&*files.sample_library(), path)
    }

    #[cfg(not(target_arch = "wasm32"))]
    #[test]
    fn a_bundled_sample_pick_decodes_on_the_worker() {
        use crate::content::bundle::{Kind, tests::fixture};
        use crate::project_files::content::tests::Temp;
        let tmp = Temp::new("bundled-pick");
        let sine: Vec<f32> = (0..4410).map(|i| (i as f32 * 0.0627).sin() * 0.5).collect();
        let flac = honeydew_audio::flac_writer::flac(44_100, 16, &[sine]);
        let kick = vec![("K1.flac", flac)];
        let bundle = fixture(
            &tmp.0.join("content"),
            &[(Kind::Samples, "Samples/Kick/", kick)],
        );
        let mut files = ProjectFiles::on_disk(&tmp.0.join("user"));
        let factory = crate::content::Factory::new(Vec::new()).with_bundle(bundle);
        files.factory = Box::leak(Box::new(factory));
        let path = Path::new(ROOT).join("Samples/Kick/K1.flac");
        let picked = decode_pick(
            sample_key(&files, &path),
            pick_bytes(&files, &path)(),
            false,
            None,
        );
        assert!(picked.wave.is_some(), "{:?}", picked.error);
    }

    #[test]
    fn a_slicer_picker_lists_my_files_then_our_loops_and_names_them_built_in() {
        use crate::ui::preset_picker::MY_FILES;
        // The web: no content folder.
        let files = ProjectFiles::with_provider(Box::new(MemFiles::default()));
        assert_eq!(loop_rows(&files, &[]), [&[MY_FILES][..], &LOOPS].concat());
        let mut picked = module_picker(&files, slicer::TYPE_ID);
        picked.names_in(&*files.sample_library(), &[]);
        let dust = picked.path_of("Dust Break 84").unwrap();
        let app = ContentPath::Shorthand {
            root: ContentRoot::App,
            relative: "Built-in/Loops/Dust Break 84.wav".into(),
        };
        assert_eq!(module_path(&files, &dust), Some(app));
        // A 3x Osc lists no loops, a Slicer no shapes.
        let shapes = shape_rows(&files, &[]);
        assert!(
            !shapes.iter().any(|r| LOOPS.contains(&r.as_str())),
            "{shapes:?}"
        );
        assert!(!loop_rows(&files, &[]).contains(&"Glass".into()));
    }

    #[cfg(not(target_arch = "wasm32"))]
    #[test]
    fn the_original_loop_samples_are_listed_with_ours() {
        use crate::project_files::CONTENT_FOLDER;
        use crate::project_files::content::tests::{Temp, fake_tree};
        use crate::ui::preset_picker::MY_FILES;
        let tmp = Temp::new("module-loop-picker");
        let files = ProjectFiles::on_disk(&tmp.0);
        fake_tree(&tmp.0.join(CONTENT_FOLDER));
        // The root's folders first (every factory folder before every file), then our loops.
        let rows = loop_rows(&files, &[]);
        assert_eq!(rows, [&[MY_FILES, "Sliced Drums"][..], &LOOPS].concat());
        assert_eq!(loop_rows(&files, &["Sliced Drums"]), ["Gen Loop"]);
        let mut picked = module_picker(&files, slicer::TYPE_ID);
        picked.names_in(&*files.sample_library(), &["Sliced Drums"]);
        let path = picked.path_of("Gen Loop").unwrap();
        let original = ContentPath::Shorthand {
            root: ContentRoot::App,
            relative: "Loop Samples/Sliced Drums/Gen Loop.flac".into(),
        };
        assert_eq!(module_path(&files, &path), Some(original));
    }

    #[test]
    fn a_sample_from_the_content_folder_takes_an_app_root_path() {
        let user = PathBuf::from("/home/me/studio");
        let files = ProjectFiles::on_disk(&user);
        let path = user.join("Original Content/Drum Samples/Kick/Gen Kick.wav");
        let mut kit = kit();
        assert_eq!(load_sample(&mut kit, Some(0), &path, &files), Some(0));
        let slot = &kit.slots[0];
        let app = ContentPath::Shorthand {
            root: ContentRoot::App,
            relative: "Drum Samples/Kick/Gen Kick.wav".into(),
        };
        assert_eq!(slot.file.path, Some(app));
        // The folder below the content root, as the content's own kits name it (`Drum Samples\…`).
        assert_eq!(slot.folder, "Drum Samples/Kick");
        assert_eq!(row_lines(slot), ("Kick", "Gen Kick"));
    }

    #[test]
    fn a_new_pad_takes_the_next_free_pad_the_default_params_and_no_custom_name() {
        // On the web: the user folder is the empty path, and paths are relative to it.
        let path = Path::new("My Recordings/Song Rec.ogg");
        let files = ProjectFiles::with_provider(Box::new(MemFiles::default()));
        let mut kit = kit();
        assert_eq!(load_sample(&mut kit, None, path, &files), Some(8));
        assert_eq!(kit.slots.len(), 9);
        let slot = &kit.slots[8];
        // A new id, the default slot params, no MIDI note, no cut group.
        assert_eq!((slot.file.id, kit.file.last_slot_id), (9, 9));
        assert_eq!(slot.params, SamplerSlot::default().params);
        assert_eq!((slot.midi_note(), slot.cut_group), (None, 0));
        // The pad name lines: no custom name (`MAIN`@1051), so the file's folder over its name.
        assert_eq!(row_lines(slot), ("My Recordings", "Song Rec"));
        assert_eq!(slot.file.custom_pad_name, "");
        assert_eq!(slot.folder, "My Recordings");
        assert_eq!(
            slot.file.path,
            Some(user_path("My Recordings/Song Rec.ogg"))
        );
        assert_eq!(slot.file.preset_path, "My Recordings/Song Rec.ogg");
        assert_eq!(slot.file.provenance, "My Recordings/Song Rec");
    }

    #[test]
    fn a_clips_slot_takes_the_file_and_its_folder_and_a_tempo_turns_sync_on() {
        // A file with a tempo turns SYNC on with ratio fileBPM/songBPM: mode 1, ratio = the output/source length
        // multiplier.
        let user = Path::new("/home/me/studio");
        let files = ProjectFiles::on_disk(user);
        let path = user.join("My Samples/Loops/Funk 96.wav");
        let mut slot = SamplerSlot::default();
        slot.params[0] = 0.3;
        slot.file.pitch_semitones = 5.0;
        let before = slot.clone();
        assert!(load_clip_sample(
            &mut slot,
            &path,
            &files,
            Some(96.0),
            120.0
        ));
        assert_eq!(
            slot.file.path,
            Some(user_path("My Samples/Loops/Funk 96.wav"))
        );
        assert_eq!(
            (slot.name.as_str(), slot.folder.as_str()),
            ("Funk 96", "My Samples/Loops")
        );
        assert_eq!(slot.file.provenance, "My Samples/Loops/Funk 96");
        assert_eq!((slot.file.stretch_mode, slot.file.stretch_ratio), (1, 0.8));
        // The row's other values stay.
        assert_eq!(
            (slot.params, slot.file.pitch_semitones),
            (before.params, 5.0)
        );
        // No tempo: the old sample's stretch does not carry over (ratio 1.0); the SYNC button stays as it was.
        let mut synced = before.clone();
        (synced.file.stretch_mode, synced.file.stretch_ratio) = (1, 1.25);
        assert!(load_clip_sample(&mut synced, &path, &files, None, 120.0));
        assert_eq!(
            (synced.file.stretch_mode, synced.file.stretch_ratio),
            (1, 1.0)
        );
        // No song tempo: no ratio to compute either.
        assert!(load_clip_sample(&mut slot, &path, &files, Some(96.0), 0.0));
        assert_eq!((slot.file.stretch_mode, slot.file.stretch_ratio), (1, 1.0));
        // A file outside the user folder changes nothing.
        let mut outside = before.clone();
        assert!(!load_clip_sample(
            &mut outside,
            Path::new("/tmp/a.wav"),
            &files,
            Some(96.0),
            120.0
        ));
        assert_eq!(outside, before);
    }

    #[test]
    fn nothing_changes_without_a_slot_a_free_pad_or_a_file_in_the_user_folder() {
        let user = Path::new("/home/me/studio");
        let files = ProjectFiles::on_disk(user);
        let path = user.join("My Samples/Hat.flac");
        let mut kit = kit();
        let before = kit.clone();
        // No slot on pad 8.
        assert_eq!(load_sample(&mut kit, Some(8), &path, &files), None);
        // A file outside the user folder: `PTH1` holds no plain path.
        assert_eq!(
            load_sample(&mut kit, Some(0), Path::new("/tmp/Hat.flac"), &files),
            None
        );
        assert_eq!(kit, before);
        // The 22 pads in use.
        while kit.slots.len() < PAD_COUNT {
            kit.insert_slot(kit.slots.len(), SamplerSlot::default());
        }
        let full = kit.clone();
        assert_eq!(load_sample(&mut kit, None, &path, &files), None);
        assert_eq!(kit, full);
    }
}
