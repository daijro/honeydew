//! The preset picker overlay: what it lists and where its pick loads.
//! - A card's preset button: the module's presets, or `Default`, into that module (and an instrument's name into its
//!   channel).
//! - The rack ≡ `Load Rack preset`: a rack preset replaces the rack's modules; the Modules browser's `Rack Preset` adds
//!   them where its `Paste` would; the add-channel `Rack` makes a new Rack channel of them, selected.
//! - `Load Drumset` (the rack ≡ menu's, the drums track menu's and the drum row menu's, and a new Drums channel's): the
//!   `Drumsets` picker over `My Drumsets`, `Default`, the built-in kit ([`DrumKit::default_kit`]), and our factory
//!   kits; a drum kit template replaces the channel's kit.
//! - `ChordSet`: the chord set picker, titled `Chord Sets`, over `My Chord Sets`, `Default` (the built-in `Major` set,
//!   [`KeyboardState::default`]), our eight sets and the content folder's `Chord Sets`. A pick is parsed
//!   ([`ChordSet::parse`]) and stored as its normalised JSON, under the file's name, in the song's `KEYB`: one undo
//!   step, no new engine. It belongs to the song, so the channel it is opened with does not matter. The keyboard
//!   settings' `ChordSet` ≡ `Load` opens it (`keyboard_settings::chord_set_item`).
//! - Our own factory bank ([`content`]) lists at each module's and the `Drumsets` picker's root, before the imported
//!   content's: `Presets/<module folder>` or `Drumsets` ([`built_in_folder`]). A pick of ours is named from the
//!   factory's root (`Presets/GMSynth/Bass/Round`).
//! - With imported content in the content folder ([`crate::project_files::CONTENT_FOLDER`]), each picker's root also
//!   lists its factory folder, read-only: a module's ([`ProjectFiles::factory_presets`]), `Drum Templates` or
//!   `Rack Presets`. A module takes its own file types ([`preset_exts`], [`load_pick`]): a legacy `.flms` or
//!   `.flgsynth`, a 3x Osc `.fst`, or a DW program, Slicer loop or 3x Osc shape, which names the file in the module's
//!   `CSTM`.
//!
//! A module pick goes to the module's own loader first; the DW Sampler's lists its programs (`.dwp`) beside its presets
//! and takes a `.dwp` or an audio file, and a file it rejects loads as a module preset. The files the worker has a
//! loader for ([`worker_loads`]) load on a worker thread and the result is applied on the UI thread a later frame, to
//! the channel the picker is on then: any DW Sampler pick, a SoundFont Player's `.sf2`, a Slicer's loop or a 3x Osc's
//! shape (an audio file its loader names in its `CSTM`), and a preset (`.flmpst`) for the Slicer, 3x Osc and SoundFont
//! Player. A DW program's zone samples decode on the worker too, and so does a loop or a shape, into a 3x Osc's Custom
//! table ([`SampleCache::absorb`]), so the engine build that follows finds them cached, a loop's slices too
//! ([`sample_picker::decode_pick`]). Meanwhile the picker stays, its row pulsing `LOADING...`, a Slicer's strip naming
//! its loop, a new pick replaces the one loading, and another picker opened in its place cancels it; a close while it
//! loads waits for it, drawing nothing and taking every click: closing never reverts a pick. A 3x Osc `.fst` loads on
//! the UI thread ([`load_module`]). Either way the file is stored by its shorthand (`@R@Built-in/…`, `@R@` in the
//! content folder, else `@L@`) and named, `PRST` included, relative to the root it is in
//! ([`ProjectFiles::name_module`]).
//!
//! The add-channel `Instrument` opens the Instruments picker ([`PresetTarget::Instruments`]): `My Files` over the whole
//! user folder, then the instrument categories of imported content and ours ([`categories`],
//! [`instrument_sources`]). A pick loads into the rack's top card when that is of the file's instrument, else in a new
//! module of it appended at the bottom in the top card's place ([`place_instrument`]), as a module picker's pick loads
//! (on a worker where that one would, its row pulsing); the channel is named by the picker's naming rule
//! ([`name_instrument_channel`]). An audio file or `.instr` in `My Files` loads nothing. Opened for a new channel, it
//! first picks `Close Grand` where the content has it, else our bundled grand piano ([`close_grand`]), as a tap would.
//!
//! A load is a final song edit with a new engine: a module's or a rack's preset makes no undo step (the rack and
//! presets are not covered by undo), a new Rack channel, a drum kit or a chord set one. A module's preset button and a
//! rack preset opened from the rack keep the picker open after a pick; the others close it ([`stays_open`]). A preset
//! that cannot be read or does not fit leaves the song as it was and shows `Failed to load project file`; skipped
//! modules show `Skipped N incompatible module(s)`. A rack preset with no modules, or a kit with no pads, loads
//! nothing.
//!
//! A module's picker lists its legacy presets too: MiniSynth's `.flms`, GMSynth's `.flgsynth`, 3x Osc's `.fst`
//! ([`preset_exts`]). The SoundFont Player's lists `.sf2` files, which its own loader takes: its `My SoundFonts` beside
//! `My Files` ([`PresetPicker::with_root`]) and the content folder's `SoundFonts` as a factory folder. A pick sets the
//! module's `SF2D` to the file at patch 0 ([`load_pick`]), names the module for it and tells the rack its patches, one
//! final edit; the patch is picked from the card's patch menu. A file that is no SoundFont shows
//! `Failed to load project file` with the SoundFont's own error and changes nothing. Every picker offers the file
//! browser's ≡ actions on the user's files ([`PresetPicker::show_mut`]), through
//! [`crate::project_files::ProjectFiles::with_preset_files`]; they change files, never imported content, and the
//! song only as SONGS' do: their moves are forwarded, so a renamed song file retitles it
//! ([`crate::project_files::ProjectFiles::path_moved`]). On the web the pickers that list audio files offer `Import`,
//! into the module's user folder.

use std::path::{Path, PathBuf};
use std::sync::mpsc::{self, Receiver, TryRecvError};

use eframe::egui;
use honeydew_audio::device::AudioCommand;

use super::{AppCtx, Nav, Overlay, rack, sample_picker};
use crate::audio_engine::slicer::write_auto_clip;
use crate::audio_link::AudioLink;
use crate::content;
use crate::flm::chunk::ContentPath;
use crate::panels::cstm::{clear_dw_program, load_dw_program, pick_dw_program};
use crate::params::{Role, module_spec};
use crate::project_files::{
    CHORD_SETS_FOLDER, CHORDS_EXT, DRUM_TEMPLATES, DRUMSET_EXT, DRUMSETS_FOLDER, FST_EXT,
    LOAD_FAILED, PRESET_EXT, PRESETS_FOLDER, ProjectFiles, RACK_EXT, RACK_PRESETS, RACKS_FOLDER,
    SAMPLE_EXTS, SF2_EXT, SOUNDFONTS_FOLDER, is_audio, load_pick, preset_exts, preset_folder,
};
use crate::sample_cache::{SampleCache, Waveform, WorkerReader, disk_reader, reader};
use crate::sf2::Preset;
use crate::song::Rack;
use crate::song::chords::ChordSet;
use crate::song::{ChannelKind, DrumKit, KeyboardState, ModuleInstance};
use crate::ui::picker_preview::Sound;
use crate::ui::preset_picker::categories;
use crate::ui::preset_picker::{
    CHORD_SETS_TITLE, DRUMSET_TITLE, INSTRUMENTS_TITLE, PickerAction, PresetPicker, RACK_TITLE,
};
use crate::ui::rack::{
    Held, Outcome, PresetTarget, hold_removal, load_rack_into, module_name,
    name_instrument_channel, name_song_channel,
};

/// The tag of the picker's messages, which only have `OK`.
pub const TAG: &str = "preset_picker";

/// The open picker. It starts afresh on each opening, however it was last closed: at its root, with the slide-in.
#[derive(Default)]
pub struct State {
    picker: Option<PresetPicker>,
    /// The egui pass it was last drawn in: a gap means it was closed in between.
    pass: u64,
    /// The channel a pick added, for the app to select once it has applied the frame's edit (which re-keys the
    /// selection).
    pub select: Option<usize>,
    /// A rack preset load removing modules automation lanes target, held back ([`hold_removal`]) with what it skipped:
    /// the app asks `Delete automation?` first, and shows the skipped modules once it is made.
    pub(crate) ask_remove: Option<Held>,
    /// A pick loading on a worker.
    loading: Option<Loading>,
    /// The picker was closed while its pick loads: it stays, drawing nothing and taking every click, until the pick
    /// lands (closing never reverts a pick).
    closing: bool,
    /// [`ProjectFiles::library_changes`] as it last listed: an `Import` since is listed at once.
    changes: u64,
    /// The Instruments picker next shown picks [`CLOSE_GRAND`] first, as a tap on it would, for the new `Instrument`
    /// channel it opens on (which is made with a SuperSaw).
    pub(crate) close_grand: bool,
    /// Every new DW Sampler loads Close Grand: the loads not landed yet ([`load_close_grand`]).
    close_grands: Vec<Loading>,
    /// The web: the [`Self::close_grand`] pick waiting for its download ([`content::web::picked`]).
    grand_download: Option<(PathBuf, content::web::Ticket)>,
    /// The web: the [`load_close_grand`]s waiting for theirs, by channel id and module id.
    grand_downloads: Vec<(i32, i32, (PathBuf, content::web::Ticket))>,
}

impl State {
    /// The open picker's sample strip stops ([`super::Slots::stop_samples`]).
    pub(super) fn stop_sample(&mut self) {
        if let Some(picker) = &mut self.picker {
            picker.stop_sample();
        }
    }

    /// A pick is loading on a worker.
    #[cfg(test)]
    pub(crate) fn loading(&self) -> bool {
        self.loading.is_some() || !self.close_grands.is_empty()
    }

    /// The open picker's audition strip is on.
    #[cfg(test)]
    pub(crate) fn auditioning(&self) -> bool {
        (self.picker.as_ref()).is_some_and(PresetPicker::auditioning)
    }

    /// The open picker shows the sample strip.
    #[cfg(test)]
    pub(crate) fn sample_shown(&self) -> bool {
        (self.picker.as_ref()).is_some_and(|p| p.sample_name().is_some())
    }
}

/// The DW Sampler's module type id; its picker lists its programs (`.dwp`) through `preset_exts`.
const DW: u16 = 1;

/// The module types some of whose picks load on a worker: DW Sampler, Slicer, 3x Osc, SoundFont Player.
/// [`worker_loads`] says which of their files do.
const ON_WORKER: [u16; 4] = [DW, 33, 39, 52];

/// `path` is a file the worker has a loader for, for a module of type `type_id` ([`ON_WORKER`]): any file for the DW
/// Sampler (its program loader, then a preset); a `.sf2` for the SoundFont Player; a loop or a shape for the Slicer or
/// 3x Osc ([`names_sample`]); a module preset (`.flmpst`) for any of them.
// ponytail: a 3x Osc `.fst` imports on the UI thread (`load_module`; a small file), and the shape it names decodes at
// the engine build; give it a `Picked` case of its own if an import ever shows.
fn worker_loads(type_id: u16, path: &Path) -> bool {
    let is = |ext: &str| (path.extension()).is_some_and(|e| e.eq_ignore_ascii_case(ext));
    type_id == DW
        || (type_id == SOUNDFONT_PLAYER && is(SF2_EXT))
        || is(PRESET_EXT)
        || names_sample(type_id, path)
}

/// `path` is an audio file the Slicer's or 3x Osc's loader names in its `CSTM`: a loop, a shape. Its sample decodes on
/// the worker ([`sample_picker::decode_pick`]).
fn names_sample(type_id: u16, path: &Path) -> bool {
    matches!(type_id, SLICER | THREE_OSC) && is_audio(path)
}

/// A pick of `path` for module `id` of type `type_id`, loading on a worker. Its channel is kept by id: the app re-keys
/// the open picker's channel index when channels move, and that index is the one it lands on. `program` is the file's
/// shorthand (what a path chunk holds: `@R@Built-in/…`, `@R@` in the content folder, else `@L@`; `None` for a file
/// outside them all), as [`load_module`] has it.
struct Loading {
    channel_id: i32,
    id: i32,
    type_id: u16,
    path: PathBuf,
    program: Option<ContentPath>,
    done: Receiver<Done>,
    /// An Instruments pick: it names the channel by that picker's rule.
    instruments: bool,
    /// The rack an automatic `Close Grand` replaced, back when the pick lands with none of its zones decoded (a silent
    /// channel).
    fallback: Option<Rack>,
}

/// What the worker made of a pick: the file loaded, and the samples a DW Sampler's program plays, decoded into a cache
/// of its own for the app's to absorb. Decoded there and not at the next engine build, on the UI thread.
struct Done {
    picked: Picked,
    samples: SampleCache,
}

impl Done {
    /// Zones referenced and none decoded, a DW Sampler that would be silent.
    fn silent(&self) -> bool {
        self.samples.missing().next().is_some() && !self.samples.decoded_any()
    }
}

/// What the worker made of a picked file.
enum Picked {
    /// The DW Sampler's program loader took it, with the Attack and Release it sets.
    Program(Option<(f32, f32)>),
    /// A module preset.
    Preset(Box<ModuleInstance>),
    /// The SoundFont Player's loader took a `.sf2`, with its patches.
    SoundFont(Vec<Preset>),
    /// The Slicer's or 3x Osc's loader names the audio file in its `CSTM` ([`names_sample`]), its sample decoded: the
    /// loop's waveform for the sample strip (none for a shape's table, or a file that does not decode).
    Sample(Option<Waveform>),
    Failed(String),
}

/// The SoundFont Player's module type id, whose picker lists `.sf2` files.
const SOUNDFONT_PLAYER: u16 = 52;

/// The pick `bytes` for a module of type `type_id`: the module's own loader first (the DW Sampler's program loader; the
/// SoundFont Player's, for the `.sf2` file `sf2`, which reads the whole file the engine will parse; the others' are not
/// this one's), then a module preset. Runs on the worker.
fn picked(type_id: u16, sf2: bool, bytes: &[u8]) -> Picked {
    if type_id == DW
        && let Some(sets) = honeydew_audio::dwp::load_program(bytes)
    {
        return Picked::Program(sets);
    }
    if type_id == SOUNDFONT_PLAYER && sf2 {
        return match crate::sf2::presets(bytes) {
            Ok(patches) => Picked::SoundFont(patches),
            Err(err) => Picked::Failed(err.to_string()),
        };
    }
    match crate::flm::read_preset(bytes, Some(type_id)) {
        Ok(preset) => Picked::Preset(Box::new(preset)),
        Err(err) => Picked::Failed(err.to_string()),
    }
}

/// [`picked`] and, for a DW Sampler with a `read` that works off the UI thread, the samples its program will play (what
/// the engine build would decode on the UI thread): the program a pick took, or a preset's own. Runs on the worker.
// ponytail: where `read` is none (the web, which has no threads; copies kept in memory) the engine build decodes them
// as before; share the copies with the worker if a big program on the web matters.
fn load(
    type_id: u16,
    sf2: bool,
    bytes: &[u8],
    program: Option<&ContentPath>,
    read: Option<WorkerReader>,
) -> Done {
    let picked = picked(type_id, sf2, bytes);
    let mut samples = SampleCache::default();
    if let (Some(read), DW) = (read, type_id) {
        match &picked {
            Picked::Program(_) => {
                program.iter().for_each(|p| samples.load_program(p, &read));
            }
            Picked::Preset(preset) => samples.load_dw(preset, &read),
            Picked::SoundFont(_) | Picked::Sample(_) | Picked::Failed(_) => {}
        }
    }
    Done { picked, samples }
}

/// Tests hold the workers their own thread starts with this, to see a pick while it still loads: `lock` it as a
/// `Mutex`, and the hold lasts until the guard drops. Per test thread, so a test never waits on another's hold.
#[cfg(test)]
pub(crate) static WORKER_HOLD: Hold = Hold;

#[cfg(test)]
pub(crate) struct Hold;

#[cfg(test)]
thread_local! {
    /// The gate of this thread's last hold, which the workers it starts wait on.
    static GATE: std::cell::Cell<Option<&'static std::sync::Mutex<()>>> = const { std::cell::Cell::new(None) };
}

#[cfg(test)]
impl Hold {
    /// Holds the workers this thread starts until the guard drops. Each hold leaks its gate: a few bytes a test.
    pub(crate) fn lock(&self) -> std::sync::LockResult<std::sync::MutexGuard<'static, ()>> {
        let gate: &'static std::sync::Mutex<()> = Box::leak(Box::default());
        GATE.set(Some(gate));
        gate.lock()
    }
}

#[cfg(test)]
type Workers = std::cell::RefCell<Vec<std::thread::JoinHandle<()>>>;

#[cfg(test)]
thread_local! {
    /// The workers this thread started, for [`join_workers`].
    static WORKERS: Workers = const { std::cell::RefCell::new(Vec::new()) };
}

/// Waits until every worker this thread started has finished, its result sent: a test's wait for a pick to land, which
/// a loaded machine only makes longer. Never under this thread's [`WORKER_HOLD`], which its workers wait on.
#[cfg(test)]
pub(crate) fn join_workers() {
    for worker in WORKERS.take() {
        drop(worker.join());
    }
}

/// A pick's error when its worker ended without a result.
pub(super) const WORKER_LOST: &str = "The file could not be loaded.";

/// Runs `job` on a worker thread; on the web, which has none, at once. The sample picker's picks decode here too.
// ponytail: no cancellation. A pick replaced or closed while it loads still runs to its end and its result is dropped
// (`state.loading = None`); that is bounded by `MAX_FILE_BYTES` for the file and `MAX_FRAMES` of decoded samples.
// Add a shared `AtomicBool` the decoder polls if abandoned picks ever pile up.
pub(super) fn spawn<T: Send + 'static>(job: impl FnOnce() -> T + Send + 'static) -> Receiver<T> {
    let (tx, rx) = mpsc::channel();
    #[cfg(test)]
    let gate = GATE.get();
    let run = move || {
        #[cfg(test)]
        if let Some(gate) = gate {
            drop(gate.lock());
        }
        drop(tx.send(job()));
    };
    #[cfg(not(target_arch = "wasm32"))]
    {
        let worker = std::thread::spawn(run);
        // A test joins its own ([`join_workers`]); the app's run detached.
        #[cfg(test)]
        WORKERS.with_borrow_mut(|workers| workers.push(worker));
        #[cfg(not(test))]
        drop(worker);
    }
    #[cfg(target_arch = "wasm32")]
    run();
    rx
}

/// Whether a pick in `target`'s picker keeps it open: a module's presets (its preset button) and a rack preset opened
/// from the rack do; the drumsets, the chord sets and a new Rack channel's preset (the playlist's) close on a pick.
fn stays_open(target: PresetTarget) -> bool {
    matches!(
        target,
        PresetTarget::Module(_)
            | PresetTarget::RackReplace
            | PresetTarget::RackInsert(_)
            | PresetTarget::Instruments
    )
}

/// The Slicer's module type id: its picks show the sample strip instead of the audition strip.
const SLICER: u16 = 33;
/// 3x Osc's module type id, whose picker lists its Custom shapes.
const THREE_OSC: u16 = 39;

/// Draws the picker for `target` on `channel`, modal, over the left of the screen, and loads its pick at once, with no
/// undo step for a module's or a rack's preset. The X, a click outside, Escape, or the channel or module being gone or
/// changing type close it, and so does a pick where [`stays_open`] says; a message replaces it. A SoundFont picked for
/// a SoundFont Player is told to `rack`, which then shows its patches.
///
/// Once a pick loads into an instrument (not the Slicer) the audition strip turns on over its row, at once for a pick
/// loading on a worker, where the row pulses `LOADING...` until it lands; its notes go to `channel` as live notes. A
/// rack preset turns it on when the rack then holds an instrument. A Slicer pick of an audio file shows the sample
/// strip instead: its name at once, its waveform and preview once the loop is decoded.
pub fn show(
    state: &mut State,
    rack: &mut rack::State,
    ui: &mut egui::Ui,
    app: &mut AppCtx,
    channel: usize,
    target: PresetTarget,
    held: Option<(usize, u32)>,
) -> Option<Nav> {
    let module_type = match target {
        PresetTarget::Module(id) => app.song.find_module(channel, id).map(|m| Some(m.type_id)),
        // A new channel has no module yet; the chord set is the song's.
        PresetTarget::NewChannel(_) | PresetTarget::ChordSet => Some(None),
        _ => app.song.channels.get(channel).map(|_| None),
    };
    let Some(module_type) = module_type else {
        close(state, app, channel);
        return Some(Nav::Close);
    };
    let pass = ui.ctx().cumulative_pass_nr();
    if state.pass + 1 != pass {
        state.picker = None;
        state.loading = None;
        state.closing = false;
        state.grand_download = None;
    }
    // Its module is now another type (as a card's panel state keys by type): what it lists is not for that one.
    if state
        .picker
        .as_ref()
        .is_some_and(|p| p.module_type() != module_type)
    {
        close(state, app, channel);
        return Some(Nav::Close);
    }
    state.pass = pass;
    // Another picker in this one's place (an open replaces it) cancels a pick still loading.
    let channel_id = app.song.channels.get(channel).map(|c| c.id);
    let ours = |l: &Loading| {
        let mine = match target {
            PresetTarget::Instruments => l.instruments,
            _ => target == PresetTarget::Module(l.id),
        };
        mine && channel_id == Some(l.channel_id)
    };
    if state.loading.as_ref().is_some_and(|l| !ours(l)) {
        state.loading = None;
    }
    // A new Instrument channel's `Close Grand`, picked as a tap picks it, in the picker made first (the audition strip
    // turns on); landing silent, it gives the rack back ([`finish`]).
    let grand = std::mem::take(&mut state.close_grand) && target == PresetTarget::Instruments;
    let grand = grand.then(|| close_grand(app.files)).flatten();
    // On the web our grand may download first, the picker showing meanwhile; another picker in its place drops it.
    if target != PresetTarget::Instruments {
        state.grand_download = None;
    }
    if let Some(path) = content::web::picked(&mut state.grand_download, grand) {
        let files = &*app.files;
        (state.picker).get_or_insert_with(|| picker(files, target, module_type));
        let bytes = app.files.preset_library(&categories::EXTS).read(&path);
        let before = app.song.channels.get(channel).map(|c| c.rack.clone());
        let nav = pick_instrument(state, app, ui.ctx(), channel, path, bytes, held);
        if let Some(loading) = &mut state.loading {
            loading.fallback = before;
        }
        if nav.is_some() {
            return nav;
        }
    }
    if let Some(loading) = &state.loading {
        let done = match loading.done.try_recv() {
            Ok(done) => Some(done),
            Err(TryRecvError::Empty) => None,
            Err(TryRecvError::Disconnected) => Some(Done {
                picked: Picked::Failed(WORKER_LOST.into()),
                samples: SampleCache::default(),
            }),
        };
        if let Some(done) = done {
            let loading = state.loading.take().expect("loading");
            // A Slicer's loop, decoded: its waveform in the sample strip, which plays it.
            if let (Picked::Sample(wave @ Some(_)), SLICER, Some(picker)) =
                (&done.picked, loading.type_id, &mut state.picker)
            {
                sample_picker::show_strip(picker, &loading.path, wave.clone());
            }
            let landed = finish(app, rack, channel, loading, done, held);
            if state.closing || landed.is_err() {
                state.closing = false;
                close(state, app, channel);
                return Some(outcome(landed));
            }
        } else if state.closing {
            block(ui.ctx());
            return None;
        } else {
            ui.ctx().request_repaint();
        }
    }
    // The automatic Close Grand's download pulses too.
    let loading = state.loading.is_some() || state.grand_download.is_some();
    let (exts, top) = (exts(target, module_type), top(target, module_type));
    let files = &*app.files;
    let picker = (state.picker).get_or_insert_with(|| picker(files, target, module_type));
    if std::mem::replace(&mut state.changes, files.library_changes()) != files.library_changes() {
        picker.reload();
    }
    picker.set_loading(loading);
    let (ctx, cx) = (ui.ctx(), &app.cx);
    let shown = app.files.with_preset_files(top, exts, |library| {
        let action = picker.show_mut(ctx, cx, library)?;
        let bytes = match &action {
            PickerAction::Pick(path) => Some(library.read(path)),
            _ => None,
        };
        Some((action, bytes))
    });
    // The file actions may have moved or deleted the song's file or folder (the Instruments picker lists `My Songs`).
    for (from, to) in picker.take_moves() {
        app.files.path_moved(app.song, &from, to.as_deref());
    }
    sound(picker, app.audio, channel);
    let Some((action, bytes)) = shown else {
        sample_picker::preview(picker, app.audio);
        return None;
    };
    if action == PickerAction::Import {
        if let Some(dir) = module_type.and_then(|t| import_dir(app.files, t)) {
            app.files.import_audio(dir);
        }
        return None;
    }
    if action == PickerAction::Close {
        if state.loading.is_some() {
            // The pick still loading lands first.
            state.closing = true;
        }
        close(state, app, channel);
        if state.closing {
            block(ui.ctx());
            return None;
        }
        return Some(Nav::Close);
    }
    if target == PresetTarget::Instruments {
        let (PickerAction::Pick(path), Some(bytes)) = (action, bytes) else {
            return None;
        };
        return pick_instrument(state, app, ui.ctx(), channel, path, bytes, held);
    }
    // A pick on a new DW Sampler's card drops its Close Grand still loading.
    state
        .close_grands
        .retain(|l| target != PresetTarget::Module(l.id) || channel_id != Some(l.channel_id));
    (state.grand_downloads)
        .retain(|&(ch, id, _)| target != PresetTarget::Module(id) || channel_id != Some(ch));
    let on_worker = module_type.filter(|t| ON_WORKER.contains(t));
    // An instrument's pick (not the Slicer's) auditions once its load call returns.
    let instrument = module_type
        .filter(|&t| t != SLICER)
        .is_some_and(|t| module_spec(t).is_some_and(|s| s.role == Role::Instrument));
    let (action, bytes) = match (target, action, bytes, on_worker) {
        (PresetTarget::Module(id), PickerAction::Pick(path), Some(Ok(bytes)), Some(type_id))
            if worker_loads(type_id, &path) =>
        {
            let channel_id = channel_id.unwrap_or_default();
            // The Slicer's sample strip, for a pick of an audio file (a loop; a preset shows none): its name at once.
            let strip = (type_id == SLICER && is_audio(&path)).then(|| path.clone());
            let module = app.song.find_module(channel, id);
            let into = module.and_then(|m| sample_picker::slicer_pick(m, &path, app.files));
            state.loading = Some(start(app.files, channel_id, id, type_id, path, bytes, into));
            if let Some(picker) = &mut state.picker {
                if instrument {
                    picker.audition_on();
                }
                if let Some(path) = strip {
                    sample_picker::show_strip(picker, &path, None);
                }
                picker.set_loading(true);
                sample_picker::preview(picker, app.audio);
            }
            ui.ctx().request_repaint();
            return None;
        }
        (_, action, bytes, _) => {
            // A pick that applies now replaces one still loading, which would land over it.
            state.loading = None;
            (action, bytes)
        }
    };
    let loaded = match (target, action, bytes) {
        (_, _, Some(Err(err))) => Err(err.to_string()),
        (PresetTarget::Module(id), PickerAction::Pick(path), Some(Ok(bytes))) => {
            load_module(app, channel, id, Some((&path, &bytes)), held)
        }
        (PresetTarget::Module(id), ..) => load_module(app, channel, id, None, held),
        (PresetTarget::Drumset, _, Some(Ok(bytes))) => crate::flm::read_drumset(&bytes)
            .map_err(|e| e.to_string())
            .and_then(|kit| load_drumset(app, channel, kit)),
        (PresetTarget::Drumset, PickerAction::Default, _) => {
            load_drumset(app, channel, DrumKit::default_kit())
        }
        (PresetTarget::ChordSet, PickerAction::Pick(path), Some(Ok(bytes))) => {
            load_chord_set(app, Some((&path, &bytes)))
        }
        (PresetTarget::ChordSet, ..) => load_chord_set(app, None),
        (_, _, Some(Ok(bytes))) => load_rack_preset(app, state, channel, target, &bytes),
        // A rack has no `Default`.
        _ => Ok(0),
    };
    let stays = stays_open(target) && state.ask_remove.is_none();
    match (stays, &loaded, &mut state.picker) {
        (true, Ok(0), Some(picker)) => {
            // A rack preset auditions the rack's first instrument: the channel's live note reaches it.
            let rack_instrument = || {
                let ch = app.song.channels.get(channel);
                let modules = ch.map(|c| c.rack.modules.as_slice()).unwrap_or_default();
                (modules.iter())
                    .any(|m| module_spec(m.type_id).is_some_and(|s| s.role == Role::Instrument))
            };
            let audition = match target {
                PresetTarget::Module(_) => instrument,
                _ => rack_instrument(),
            };
            if audition {
                picker.audition_on();
            }
            sample_picker::preview(picker, app.audio);
            None
        }
        _ => {
            close(state, app, channel);
            Some(outcome(loaded))
        }
    }
}

/// An Instruments pick of `path` (its `bytes` read) into `channel`: into the rack's top card or a new module of the
/// file's instrument ([`place_instrument`]), then loaded as a module picker's pick is ([`load_into`], or on a worker,
/// the row pulsing), with no undo step and the channel named by the picker's naming rule. The audition turns on (no
/// listed preset is the Slicer's) and the picker stays open. An audio file or `.instr` loads nothing. A load that fails
/// on the UI thread leaves the rack as it was and shows why; one on a worker fails when it lands, its new module left
/// at its defaults.
fn pick_instrument(
    state: &mut State,
    app: &mut AppCtx,
    ctx: &egui::Context,
    channel: usize,
    path: PathBuf,
    bytes: std::io::Result<Vec<u8>>,
    held: Option<(usize, u32)>,
) -> Option<Nav> {
    let bytes = match bytes {
        Ok(bytes) => bytes,
        Err(err) => {
            close(state, app, channel);
            return Some(outcome(Err(err.to_string())));
        }
    };
    let type_id = categories::instrument_of(&path, &bytes)?;
    let ch = app.song.channels.get_mut(channel)?;
    let (channel_id, before) = (ch.id, ch.rack.clone());
    let (id, swapped) = place_instrument(&mut ch.rack, type_id);
    // A pick that applies now replaces one still loading, which would land over it, and a Close Grand downloading.
    (state.loading, state.grand_download) = (None, None);
    let loaded = match ON_WORKER.contains(&type_id) && worker_loads(type_id, &path) {
        true => {
            let loading = start(app.files, channel_id, id, type_id, path, bytes, None);
            state.loading = Some(Loading {
                instruments: true,
                ..loading
            });
            if swapped {
                // A preset pick: the song is marked changed.
                app.files.song_changed();
                app.pick_loaded();
            }
            ctx.request_repaint();
            Ok(true)
        }
        false => load_into(app, channel, id, Some((&path, &bytes)), held, true).map(|_| false),
    };
    match (loaded, &mut state.picker) {
        (Ok(loading), Some(picker)) => {
            picker.audition_on();
            picker.set_loading(loading);
            sample_picker::preview(picker, app.audio);
            None
        }
        (Ok(_), None) => None,
        (Err(err), _) => {
            app.song.channels[channel].rack = before;
            close(state, app, channel);
            Some(outcome(Err(err)))
        }
    }
}

/// Where an Instruments pick of `type_id` loads in `rack`: the top card, when it is of that type; else the top card is
/// deleted and a new module of the type is appended at the bottom, the others kept. Its instance id, and whether the
/// rack changed.
fn place_instrument(rack: &mut Rack, type_id: u16) -> (i32, bool) {
    match rack.modules.first() {
        Some(top) if top.type_id == type_id => (top.instance_id, false),
        top => {
            if let Some(id) = top.map(|m| m.instance_id) {
                rack.remove(id);
            }
            (rack.add(type_id), true)
        }
    }
}

/// The Instruments picker's preset folders, its categories: the content folder's `DirectWave Samples` and
/// `Synth Presets2/<Synth>`, then ours, `Presets/<module folder>` (ours gives way to a duplicate); each group by the
/// bytes of its folders' paths. A folder that is not there is left out.
fn instrument_sources(files: &ProjectFiles) -> Vec<(u16, PathBuf)> {
    let theirs = |&t: &u16| {
        let folder = match t {
            DW => "DirectWave Samples".to_owned(),
            t => format!("Synth Presets2/{}", preset_folder(t)?),
        };
        Some((t, files.content_folder(&folder)?))
    };
    let ours = |&t: &u16| {
        let folder = Path::new(content::ROOT).join(content::PRESETS);
        Some((t, folder.join(preset_folder(t)?)))
    };
    let sorted = |mut group: Vec<(u16, PathBuf)>| {
        group.sort_by(|a, b| a.1.cmp(&b.1));
        group
    };
    let theirs = sorted(categories::SOURCES.iter().filter_map(theirs).collect());
    let ours = sorted(categories::SOURCES.iter().filter_map(ours).collect());
    theirs.into_iter().chain(ours).collect()
}

/// The preset a new `Instrument` channel starts with, in place of the SuperSaw it is made with.
const CLOSE_GRAND: &str = "Close Grand";

/// Our bundled grand: what a new DW Sampler loads where the content has no [`CLOSE_GRAND`].
const GRAND_PIANO: &str = "Presets/DW Sampler/Piano/Grand Piano.dwp";

/// Where the content has [`CLOSE_GRAND`], found by name as the Instruments picker indexes presets
/// ([`categories::index`]), over the DW Sampler's picker folders: its Instruments source, `DirectWave Samples`, and the
/// `DirectWave Samples EXP` tree, which the Instruments index leaves out and the content keeps it in (Essential Pianos
/// EXP). Without it, our bundled [`GRAND_PIANO`]; `None` without that either.
fn close_grand(files: &ProjectFiles) -> Option<PathBuf> {
    let sources: Vec<_> = (files.factory_presets(DW).into_iter())
        .map(|dir| (DW, dir))
        .collect();
    let library = files.preset_library(&categories::EXTS);
    let mut presets = categories::index(&*library, &sources)
        .into_iter()
        .flat_map(|c| c.items);
    let ours = Path::new(content::ROOT).join(GRAND_PIANO);
    (presets.find(|p| p.name == CLOSE_GRAND).map(|p| p.path))
        .or_else(|| library.exists(&ours).then_some(ours))
}

/// Every new DW Sampler loads Close Grand. Module `id` of type `type_id`, just made in the channel with id `channel_id`
/// (a Synth channel's, a Modules browser pick's), loads [`CLOSE_GRAND`] when it is a DW Sampler and the content has it,
/// else our bundled [`GRAND_PIANO`] ([`close_grand`]), on a worker as a pick of it in its card's picker loads; it lands
/// whatever is open ([`land_close_grands`]). Without either the module stays at `Default`.
pub(crate) fn load_close_grand(
    state: &mut State,
    files: &ProjectFiles,
    channel_id: i32,
    id: i32,
    type_id: u16,
) {
    let Some(path) = (type_id == DW).then(|| close_grand(files)).flatten() else {
        return;
    };
    // On the web our grand may download first ([`land_close_grands`]).
    let mut download = None;
    if let Some(path) = content::web::picked(&mut download, Some(path)) {
        start_close_grand(state, files, channel_id, id, path);
    }
    (state.grand_downloads).extend(download.map(|d| (channel_id, id, d)));
}

/// [`load_close_grand`] of `path`, read now, on a worker.
fn start_close_grand(
    state: &mut State,
    files: &ProjectFiles,
    channel_id: i32,
    id: i32,
    path: PathBuf,
) {
    if let Ok(bytes) = files.preset_library(&categories::EXTS).read(&path) {
        let loading = start(files, channel_id, id, DW, path, bytes, None);
        state.close_grands.push(loading);
    }
}

/// Each frame, the [`load_close_grand`]s that landed load as their pick would ([`finish`]), in the same edit's terms
/// (no undo step: the module's own creation holds it), into a DW Sampler still as it was made (no pick, param or edit
/// since; minimizing it aside). One that would be silent ([`Done::silent`]) leaves it at `Default`. A module that a
/// `Replace` holds for `Delete automation?` (its id not made yet) is waited for while that is `asking`, then dropped.
/// While a pointer is down nothing lands, so a drag's edits stay one undo step.
pub(crate) fn land_close_grands(
    state: &mut State,
    rack: &mut rack::State,
    ctx: &egui::Context,
    app: &mut AppCtx,
    asking: bool,
) {
    if ctx.input(|i| i.pointer.any_down()) {
        return;
    }
    for (channel_id, id, download) in std::mem::take(&mut state.grand_downloads) {
        let mut download = Some(download);
        match content::web::picked(&mut download, None) {
            Some(path) => start_close_grand(state, app.files, channel_id, id, path),
            None => (state.grand_downloads).extend(download.map(|d| (channel_id, id, d))),
        }
    }
    for loading in std::mem::take(&mut state.close_grands) {
        let channels = &app.song.channels;
        let channel = channels.iter().position(|c| c.id == loading.channel_id);
        let rack_of = |c: usize| &channels[c].rack;
        let module = channel.and_then(|c| rack_of(c).get(loading.id));
        let made = |m: &ModuleInstance| ModuleInstance {
            minimized: m.minimized,
            ..ModuleInstance::new(m.type_id, m.instance_id)
        };
        let fresh = module.is_some_and(|m| *m == made(m));
        let held =
            channel.is_some_and(|c| module.is_none() && rack_of(c).last_instance_id < loading.id);
        if held {
            if asking {
                state.close_grands.push(loading);
            }
            continue;
        }
        match loading.done.try_recv() {
            Err(TryRecvError::Empty) => state.close_grands.push(loading),
            Err(TryRecvError::Disconnected) => {}
            Ok(done) => {
                if let Some(channel) = channel.filter(|_| fresh && !done.silent()) {
                    drop(finish(app, rack, channel, loading, done, None));
                }
            }
        }
    }
    if !state.close_grands.is_empty() {
        ctx.request_repaint();
    }
}

/// Sends the audition strip's notes to `channel`: live notes, velocity 1, pan centre.
fn sound(picker: &mut PresetPicker, audio: &mut AudioLink, channel: usize) {
    for sound in picker.take_sounds() {
        audio.send(match sound {
            Sound::On(key) => AudioCommand::NoteOn {
                channel,
                key,
                velocity: 1.0,
            },
            Sound::Off(key) => AudioCommand::NoteOff { channel, key },
        });
    }
}

/// The web page was hidden (`HoneydewApp::on_hidden`): the note the audition strip holds stops now.
pub(crate) fn stop_notes(state: &mut State, audio: &mut AudioLink, channel: usize) {
    if let Some(picker) = &mut state.picker {
        picker.release();
        sound(picker, audio, channel);
    }
}

/// The picker closes: its held note is released and its sample stops; it is dropped.
fn close(state: &mut State, app: &mut AppCtx, channel: usize) {
    if let Some(picker) = &mut state.picker {
        picker.closing();
        sound(picker, app.audio, channel);
        sample_picker::preview(picker, app.audio);
    }
    (state.picker, state.grand_download) = (None, None);
}

/// Starts loading the pick `path` (its `bytes` already read) into module `id` of type `type_id` on the channel with id
/// `channel_id`, on a worker; a loop or shape decodes there with what it goes `into` ([`sample_picker::decode_pick`]: a
/// Slicer's slices).
fn start(
    files: &ProjectFiles,
    channel_id: i32,
    id: i32,
    type_id: u16,
    path: PathBuf,
    bytes: Vec<u8>,
    into: Option<sample_picker::PickFor>,
) -> Loading {
    let program = files.shorthand(&path);
    let sf2 = (path.extension()).is_some_and(|e| e.eq_ignore_ascii_case(SF2_EXT));
    let done = match names_sample(type_id, &path) {
        true => {
            let (key, shape) = (
                sample_picker::sample_key(files, &path),
                type_id == THREE_OSC,
            );
            spawn(move || {
                let decoded = sample_picker::decode_pick(key, Ok(bytes), shape, into);
                Done {
                    picked: Picked::Sample(decoded.wave),
                    samples: decoded.samples,
                }
            })
        }
        false => {
            let (read, job) = (disk_reader(files), program.clone());
            spawn(move || load(type_id, sf2, &bytes, job.as_ref(), read))
        }
    };
    Loading {
        channel_id,
        id,
        type_id,
        path,
        program,
        done,
        instruments: false,
        fallback: None,
    }
}

/// While a pick loads the picker is modal, as it is while it shows: a blocker over the screen takes every click, so no
/// other picker opens in its place. It draws nothing.
pub(super) fn block(ctx: &egui::Context) {
    let screen = ctx.content_rect();
    egui::Area::new(egui::Id::new(TAG).with("loading"))
        .order(egui::Order::Foreground)
        .fixed_pos(screen.min)
        .constrain(false)
        .fade_in(false)
        .sense(egui::Sense::click_and_drag())
        .show(ctx, |ui| {
            ui.allocate_exact_size(screen.size(), egui::Sense::hover())
        });
    ctx.request_repaint();
}

/// Where a load leaves the picker: closed, or a message in its place (skipped modules, or the load error).
pub(crate) fn outcome(loaded: Result<usize, String>) -> Nav {
    let lines = match loaded {
        Ok(n) => return skipped_message(n).map_or(Nav::Close, Nav::Swap),
        // No channel to load into: the limit's own message, not a load failure.
        Err(err) if err == CHANNEL_LIMIT => vec![err],
        Err(err) => vec![LOAD_FAILED.into(), err],
    };
    Nav::Swap(Overlay::Message {
        tag: TAG,
        lines,
        confirm: None,
        extra: None,
    })
}

/// Applies what the worker made of a pick, on the UI thread, as [`load_module`] does: the module's loader took it (a DW
/// Sampler program, now its `PATh`; a Slicer's loop or a 3x Osc's shape, now its `PATH`), or it loads as a module
/// preset (all its PRMS, then its `CSTM`, a DW Sampler's `PATh` with it, [`load_dw_program`]), or it failed. A DW
/// Sampler's loader has cleared its zones and path before it rejects a file, so a failure there is an edit too. A
/// module that is gone or now another type takes nothing. A Slicer also gets AUTO's clip in the same edit
/// ([`auto_clip`]).
fn finish(
    app: &mut AppCtx,
    rack: &mut rack::State,
    channel: usize,
    loading: Loading,
    done: Done,
    held: Option<(usize, u32)>,
) -> Result<usize, String> {
    let Loading {
        id,
        type_id,
        path,
        program: shorthand,
        instruments,
        fallback,
        ..
    } = loading;
    // An automatic `Close Grand` none of whose zones decode is silent: the rack it replaced comes back, the channel's
    // name as it was.
    if let (Some(rack), Some(ch)) = (fallback, app.song.channels.get_mut(channel))
        && done.silent()
    {
        ch.rack = rack;
        app.pick_loaded();
        return Ok(0);
    }
    let modules = app.song.channels.get_mut(channel).map(|c| &mut c.rack);
    let Some(module) = modules
        .and_then(|r| r.get_mut(id))
        .filter(|m| m.type_id == type_id)
    else {
        return Ok(0);
    };
    match done.picked {
        Picked::Program(sets) => {
            if !(shorthand.as_ref()).is_some_and(|program| pick_dw_program(module, program, sets)) {
                let path = path.display();
                return Err(format!("{path}: outside the user and content folders"));
            }
        }
        Picked::Preset(preset) => {
            module.load_preset(*preset);
            load_dw_program(module, &reader(app.files));
        }
        Picked::Sample(_) => load_pick(module, &path, &[], shorthand)?,
        Picked::SoundFont(patches) => {
            // The file's bytes are not read again: the worker checked them, and a named file's path is all that loads.
            load_pick(module, &path, &[], shorthand.clone())?;
            if let Some(shorthand) = shorthand {
                rack.soundfont_read(shorthand, Ok(patches));
            }
        }
        Picked::Failed(err) => {
            if clear_dw_program(module) {
                app.pick_loaded();
            }
            return Err(err);
        }
    }
    app.files.name_module(module, &path);
    let name = module.file.preset_name.clone();
    name_for(app.song, channel, name, type_id, instruments);
    // A new pick tries the samples missing before again, and has the ones the worker decoded for it.
    app.audio.samples.forget_missing();
    app.audio.samples.absorb(done.samples);
    // The AUTO rewrite rides with the pick: no undo step either. The song is marked changed.
    auto_clip(app, channel, id, held);
    app.files.song_changed();
    app.pick_loaded();
    Ok(0)
}

/// The file types the picker for `target` lists: a module's (`module_type`), drum kits or rack presets.
fn exts(target: PresetTarget, module_type: Option<u16>) -> &'static [&'static str] {
    match (target, module_type) {
        (PresetTarget::Instruments, _) => &categories::EXTS,
        (PresetTarget::Drumset, _) => &[DRUMSET_EXT],
        (PresetTarget::ChordSet, _) => &[CHORDS_EXT],
        (_, Some(t)) => preset_exts(t),
        _ => &[RACK_EXT],
    }
}

/// The user folder's folder that holds the files of the picker for `target` (a module of type `module_type`, drum kits
/// or rack presets); the Instruments picker's is the whole user folder (`""`).
fn top(target: PresetTarget, module_type: Option<u16>) -> &'static str {
    match (target, module_type) {
        (PresetTarget::Instruments, _) => "",
        (PresetTarget::Drumset, _) => DRUMSETS_FOLDER,
        (PresetTarget::ChordSet, _) => CHORD_SETS_FOLDER,
        (_, Some(_)) => PRESETS_FOLDER,
        _ => RACKS_FOLDER,
    }
}

/// Our own bank for `target`'s picker (a module of type `module_type`, drum kits, chord sets or rack presets), in the
/// factory ([`content`]): a module's `Presets/<module folder>` ([`preset_folder`]), `Drumsets`, `Chord Sets`; a rack has
/// none.
fn built_in_folder(target: PresetTarget, module_type: Option<u16>) -> Option<PathBuf> {
    let folder = match (target, module_type) {
        (_, Some(t)) => Path::new(content::PRESETS).join(preset_folder(t)?),
        (PresetTarget::Drumset, _) => content::DRUMSETS.into(),
        (PresetTarget::ChordSet, _) => content::CHORD_SETS.into(),
        _ => return None,
    };
    Some(Path::new(content::ROOT).join(folder))
}

/// A fresh picker for `target` (a module of type `module_type`, a drum kit or a rack), over the user's folder, our own
/// bank ([`built_in_folder`]) and the factory folders in `files`.
fn picker(files: &ProjectFiles, target: PresetTarget, module_type: Option<u16>) -> PresetPicker {
    let exts = exts(target, module_type);
    let ours = built_in_folder(target, module_type);
    match (target, module_type) {
        (PresetTarget::Instruments, _) => {
            let index = categories::index(&*files.preset_library(exts), &instrument_sources(files));
            PresetPicker::new(
                INSTRUMENTS_TITLE,
                files.user_dir().map(Path::to_owned),
                exts[0],
                false,
            )
            .of_exts(exts)
            .instruments(index)
        }
        (_, Some(t)) => {
            let picker = PresetPicker::new(module_name(t), files.presets_dir(t), exts[0], true)
                .of_exts(exts)
                .of_type(t)
                .with_factory(ours.into_iter().chain(files.factory_presets(t)))
                .with_import(import_dir(files, t).is_some());
            // The SoundFont Player's `.sf2` files are in `My SoundFonts`, beside `My Files`.
            match t == SOUNDFONT_PLAYER {
                true => picker.with_root(SOUNDFONTS_FOLDER, files.soundfonts_dir(), &[SF2_EXT]),
                false => picker,
            }
        }
        (PresetTarget::Drumset, _) => {
            PresetPicker::new(DRUMSET_TITLE, files.drumsets_dir(), exts[0], true)
                .with_factory(ours.into_iter().chain(files.content_folder(DRUM_TEMPLATES)))
        }
        // The content folder's `Chord Sets` is named as our factory folder is.
        (PresetTarget::ChordSet, _) => {
            PresetPicker::new(CHORD_SETS_TITLE, files.chord_sets_dir(), exts[0], true).with_factory(
                ours.into_iter()
                    .chain(files.content_folder(content::CHORD_SETS)),
            )
        }
        _ => PresetPicker::new(RACK_TITLE, files.racks_dir(), exts[0], false)
            .with_factory(files.content_folder(RACK_PRESETS)),
    }
}

/// The error when a new channel would pass the channel-id limit ([`crate::song::CHANNEL_ID_LIMIT`]): the add-channel
/// menu's, for every way a channel is added.
pub(crate) const CHANNEL_LIMIT: &str =
    "Failed to create new channel because channel id limit has been reached";

/// The [`CHANNEL_LIMIT`] message.
pub(crate) fn channel_limit_message() -> Overlay {
    Overlay::Message {
        tag: TAG,
        lines: vec![CHANNEL_LIMIT.into()],
        confirm: None,
        extra: None,
    }
}

/// Where `Import` keeps a file for module type `t`'s picker: the module's user folder, when the picker lists audio
/// files (the DW Sampler's, the Slicer's, 3x Osc's) and `files` takes an import ([`ProjectFiles::imports_audio`]).
fn import_dir(files: &ProjectFiles, t: u16) -> Option<PathBuf> {
    let audio = preset_exts(t).contains(&SAMPLE_EXTS[0]);
    files
        .presets_dir(t)
        .filter(|_| audio && files.imports_audio())
}

/// The `Skipped N incompatible module(s)` message of `n` modules skipped, `None` for none.
pub(crate) fn skipped_message(n: usize) -> Option<Overlay> {
    let line = match n {
        0 => return None,
        1 => "Skipped 1 incompatible module".to_owned(),
        n => format!("Skipped {n} incompatible modules"),
    };
    Some(Overlay::Message {
        tag: TAG,
        lines: vec![line],
        confirm: None,
        extra: None,
    })
}

/// Loads `Default` (no `pick`) or the file `pick` (its path and bytes) into module `id` of `channel` ([`load_pick`]);
/// an instrument's preset names its channel too, and a Slicer gets AUTO's clip ([`auto_clip`]). The preset's path is
/// relative to the folder it is in: the content folder, or the user folder.
fn load_module(
    app: &mut AppCtx,
    channel: usize,
    id: i32,
    pick: Option<(&Path, &[u8])>,
    held: Option<(usize, u32)>,
) -> Result<usize, String> {
    load_into(app, channel, id, pick, held, false)
}

/// [`load_module`]; an Instruments pick (`instruments`) names the channel by that picker's rule ([`name_for`]).
fn load_into(
    app: &mut AppCtx,
    channel: usize,
    id: i32,
    pick: Option<(&Path, &[u8])>,
    held: Option<(usize, u32)>,
    instruments: bool,
) -> Result<usize, String> {
    let files: &ProjectFiles = app.files;
    let Some(module) = (app.song.channels.get_mut(channel)).and_then(|c| c.rack.get_mut(id)) else {
        return Ok(0);
    };
    let name = match pick {
        None => {
            module.load_default();
            String::new()
        }
        Some((path, bytes)) => {
            load_pick(module, path, bytes, files.shorthand(path))?;
            // A `.fst`'s category is its folder's name, as its import set it.
            let fst = (path.extension()).is_some_and(|e| e.eq_ignore_ascii_case(FST_EXT));
            let category = module.file.category.clone();
            files.name_module(module, path);
            if fst {
                let file = &mut module.file;
                file.provenance = format!("{category}/{}", file.preset_name);
                file.category = category;
            }
            module.file.preset_name.clone()
        }
    };
    let type_id = module.type_id;
    name_for(app.song, channel, name, type_id, instruments);
    auto_clip(app, channel, id, held);
    app.files.song_changed();
    app.pick_loaded();
    Ok(0)
}

/// An instrument's preset `name` (of module type `type_id`) is its channel's ([`name_song_channel`]; an Instruments pick
/// by that picker's rule, [`name_instrument_channel`]). An effect's names nothing.
fn name_for(
    song: &mut crate::song::Song,
    channel: usize,
    name: String,
    type_id: u16,
    instruments: bool,
) {
    if !module_spec(type_id).is_some_and(|s| s.role == Role::Instrument) {
        return;
    }
    match instruments {
        true => name_instrument_channel(song, channel, &name, module_name(type_id)),
        false => name_song_channel(song, channel, name),
    }
}

/// A Slicer's new loop (or preset) is a re-detect: AUTO's clip, which rides with the pick, with no undo step, into
/// `held` first ([`write_auto_clip`]). Another module: nothing.
fn auto_clip(app: &mut AppCtx, channel: usize, id: i32, held: Option<(usize, u32)>) {
    let read = reader(app.files);
    write_auto_clip(
        app.song,
        channel,
        id,
        &mut app.audio.samples,
        &read,
        false,
        held,
    );
}

/// Loads the rack preset `bytes` for `target`: into `channel`'s rack, or into a new Rack channel. Returns how many
/// modules were skipped. A load into a rack that removes modules automation lanes target is held back for the app's
/// question ([`State::ask_remove`]).
fn load_rack_preset(
    app: &mut AppCtx,
    state: &mut State,
    channel: usize,
    target: PresetTarget,
    bytes: &[u8],
) -> Result<usize, String> {
    // The modules take the rack's own fresh ids as they go in. A DW Sampler's program loads with it.
    let mut modules = crate::flm::read_rack(bytes, &mut 1).map_err(|e| e.to_string())?;
    let read = reader(app.files);
    modules.iter_mut().for_each(|m| load_dw_program(m, &read));
    drop(read);
    if modules.is_empty() {
        return Ok(0);
    }
    let song = &mut *app.song;
    let (outcome, skipped) = match target {
        PresetTarget::NewChannel(at) => {
            // Past the channel-id limit nothing is added, and the message says why.
            let Some(index) =
                song.insert_channel(at.unwrap_or(song.channels.len()), ChannelKind::Rack)
            else {
                return Err(CHANNEL_LIMIT.to_owned());
            };
            state.select = Some(index);
            load_rack_into(song, index, &modules, None)
        }
        _ => {
            let Some(before) = song.channels.get(channel).cloned() else {
                return Ok(0);
            };
            let at = match target {
                PresetTarget::RackInsert(at) => Some(at),
                _ => None,
            };
            let loaded = load_rack_into(song, channel, &modules, at);
            if let Some(after) = hold_removal(&mut song.channels[channel], before) {
                // Its skipped modules show once it is made: a message now would give way to the question.
                let skipped = loaded.1;
                state.ask_remove = Some(Held {
                    channel,
                    after,
                    skipped,
                });
                return Ok(0);
            }
            loaded
        }
    };
    // A new Rack channel is one undo step; a rack preset into a rack is none.
    match (outcome, target) {
        (Outcome::ChainChanged, PresetTarget::NewChannel(_)) => app.song_edited(true),
        (Outcome::ChainChanged, _) => {
            app.files.song_changed();
            app.pick_loaded();
        }
        _ => {}
    }
    Ok(skipped)
}

/// Loads the drum kit `file` (a template, or `Default`) as `channel`'s kit. Its pads get new ids after the kit's last
/// (a new slot gets lastSlotId + 1), so automation of the old pads names none of them, and their routing goes to the
/// channel output (the file's send targets name another song's channels). The hits stay as they are, those on pads past
/// the new kit too: they play nothing, as after a `Delete`, and play again if the kit grows.
fn load_drumset(app: &mut AppCtx, channel: usize, file: DrumKit) -> Result<usize, String> {
    let Some(ch) = app.song.channels.get_mut(channel) else {
        return Ok(0);
    };
    if file.slots.is_empty() {
        return Ok(0);
    }
    let kit = ch.drum_kit.get_or_insert_with(DrumKit::default);
    kit.slots.clear();
    kit.file.pads = file.file.pads;
    kit.file.unknown_chunks = file.file.unknown_chunks;
    for mut slot in file.slots {
        (slot.file.output_bus, slot.file.send_target) = (-1, -1);
        kit.insert_slot(kit.slots.len(), slot);
    }
    app.song_edited(true);
    Ok(0)
}

/// What a chord set pick stores in `KEYB`: the set's JSON as [`ChordSet::to_json`] writes it, and its name. `Default`
/// (no `pick`) is the built-in `Major` set; the file `pick` (its path and bytes) is parsed, so a file that is not a
/// chord set is an error, and is named by its stem.
fn chord_set(pick: Option<(&Path, &[u8])>) -> Result<(String, String), String> {
    let Some((path, bytes)) = pick else {
        let default = KeyboardState::default();
        return Ok((default.chord_json, default.chord_set_name));
    };
    let text = std::str::from_utf8(bytes).map_err(|e| e.to_string())?;
    let set = ChordSet::parse(text)?;
    let name = path.file_stem().unwrap_or_default().to_string_lossy();
    Ok((set.to_json(), name.into_owned()))
}

/// Loads `Default` (no `pick`) or the chord set file `pick` into the song's `KEYB` ([`chord_set`]): one undo step. A
/// file that does not load leaves the song as it was.
fn load_chord_set(app: &mut AppCtx, pick: Option<(&Path, &[u8])>) -> Result<usize, String> {
    let (json, name) = chord_set(pick)?;
    let keyboard = &mut app.song.file.keyboard;
    (keyboard.chord_json, keyboard.chord_set_name) = (json, name);
    // A file is a loaded set (`ChordSet` shows its name); `Default` is the untouched default again.
    keyboard.set_state = if pick.is_some() {
        crate::song::SET_LOADED
    } else {
        crate::song::SET_DEFAULT
    };
    // The audio never reads `KEYB`: an undo step, with no new engine (as the key's change in the keyboard settings).
    app.param_edited();
    Ok(0)
}

#[cfg(all(test, not(target_arch = "wasm32")))]
mod tests {
    use super::*;
    use crate::audio_link::AudioLink;
    use crate::flm::chunk::{ContentPath, ContentRoot};
    use crate::project_files::CONTENT_FOLDER;
    use crate::project_files::content::tests::{Temp, fake_tree};
    use crate::settings::Settings;
    use crate::song::{DEFAULT_CHORD_JSON, Song, SynthKind};
    use crate::ui::UiCtx;
    use crate::ui::main_menu::SongLibrary;
    use crate::ui::main_screen::Regions;
    use crate::ui::preset_picker::{DEFAULT, MY_FILES};

    /// The rows of `target`'s picker (a module of type `module_type`) once `path` is opened.
    fn names(
        files: &ProjectFiles,
        target: PresetTarget,
        module_type: Option<u16>,
        path: &[&str],
    ) -> Vec<String> {
        let mut picker = picker(files, target, module_type);
        picker.names_in(&*files.preset_library(exts(target, module_type)), path)
    }

    #[test]
    fn each_preset_picker_lists_its_original_content_after_the_users_own() {
        let tmp = Temp::new("preset-pickers");
        fake_tree(&tmp.0.join(CONTENT_FOLDER));
        let files = ProjectFiles::on_disk(&tmp.0);
        let module = |t: u16, path: &[&str]| names(&files, PresetTarget::Module(2), Some(t), path);
        // Our four kits come before the content folder's.
        assert_eq!(
            names(&files, PresetTarget::Drumset, None, &[]),
            [
                MY_FILES,
                DEFAULT,
                "Dust Kit",
                "Foundry Kit",
                "Pocket Kit",
                "Relay Kit",
                "Gen Kit"
            ]
        );
        assert_eq!(
            names(&files, PresetTarget::RackReplace, None, &[]),
            [MY_FILES, "Gen Chain"]
        );
        // GMSynth: our categories, then its own; a module preset and a legacy one, not another module's. Our `Bass`
        // gives way to the content folder's of the same name.
        assert_eq!(
            module(2, &[]),
            [MY_FILES, DEFAULT, "Bells", "Keys", "Lead", "Pad", "Bass"]
        );
        assert_eq!(module(2, &["Bass"]), ["Gen Bass", "Gen Old"]);
        assert_eq!(module(0, &["Keys"]), ["Gen Keys"]);
        // Reverb 2 lists the `Reverb` folder's presets right at its root.
        assert_eq!(module(32, &[]), [MY_FILES, DEFAULT, "Gen Hall"]);
        // DW Sampler: both DirectWave folders, the `EXP` one's after; a program without its sample folder.
        assert_eq!(
            module(1, &[]),
            [MY_FILES, DEFAULT, "Guitar", "Synth Bass EXP"]
        );
        assert_eq!(module(1, &["Guitar"]), ["Gen Guitar"]);
        assert_eq!(module(33, &["Sliced Drums"]), ["Gen Loop"]);
        // 3x Osc: our categories, then its `.fst` presets' categories, then its custom shapes'.
        assert_eq!(
            module(39, &[]),
            [MY_FILES, DEFAULT, "Lead", "Pad", "Synth", "Bass", "HQ"]
        );
        assert_eq!(module(39, &["Bass"]), ["Gen"]);
        assert_eq!(module(39, &["HQ"]), ["Gen Shape"]);
        // No folder for the module: our bank's categories only.
        assert_eq!(
            module(23, &[]),
            [MY_FILES, DEFAULT, "Bass", "Lead", "Pad", "Pluck"]
        );
        // Without content, every picker is as before, but for our own banks (the drumsets' four kits).
        let bare = Temp::new("preset-pickers-bare");
        let files = ProjectFiles::on_disk(&bare.0);
        assert_eq!(
            names(&files, PresetTarget::Drumset, None, &[]),
            [
                MY_FILES,
                DEFAULT,
                "Dust Kit",
                "Foundry Kit",
                "Pocket Kit",
                "Relay Kit"
            ]
        );
    }

    /// The DW Sampler lists an Essential Pianos preset, not the folder of its zone samples beside it; a folder of
    /// samples with no program still lists. The user's own folder beside their preset lists.
    #[test]
    fn the_dw_samplers_presets_list_without_their_zone_sample_folders() {
        use crate::project_files::content::tests::put;
        let tmp = Temp::new("preset-pickers-zones");
        let exp = tmp.0.join(CONTENT_FOLDER).join("DirectWave Samples EXP");
        let dw = crate::flm::write_preset(&crate::song::ModuleInstance::new(1, 2)).unwrap();
        let pianos = exp.join("Essential Pianos EXP");
        put(&pianos.join("Close Grand.flmpst"), &dw);
        for n in [1, 10, 13, 2] {
            put(
                &pianos.join(format!("Close Grand/Close Grand {n}.wav")),
                b"RIFF",
            );
        }
        put(
            &exp.join("WA Production Selection EXP/Kicks/Kick 1.wav"),
            b"RIFF",
        );
        let files = ProjectFiles::on_disk(&tmp.0);
        let mine = files.presets_dir(1).unwrap();
        put(&mine.join("My Grand.flmpst"), &dw);
        put(&mine.join("my grand/My Grand 1.wav"), b"RIFF");
        let dw = |path: &[&str]| names(&files, PresetTarget::Module(2), Some(1), path);
        assert_eq!(dw(&["Essential Pianos EXP"]), ["Close Grand"]);
        assert_eq!(dw(&["WA Production Selection EXP", "Kicks"]), ["Kick 1"]);
        // The user's own folder beside their preset is theirs: it lists.
        assert_eq!(dw(&[MY_FILES]), ["my grand", "My Grand"]);
    }

    /// The imported content tree holds about 4,000 files. A picker reads one folder when it opens, so a frame never
    /// reads more than the largest folder: measured here on a fake tree of that size.
    #[test]
    fn opening_the_folders_of_a_tree_the_size_of_the_original_stays_within_a_frame_or_two() {
        use crate::project_files::content::tests::put;
        use std::time::{Duration, Instant};
        let tmp = Temp::new("preset-pickers-large");
        let c = tmp.0.join(CONTENT_FOLDER);
        let wav = b"RIFF";
        for (cat, n) in [
            ("Snare", 227),
            ("Hat", 200),
            ("Kick", 155),
            ("Percussion", 151),
            ("SFX", 79),
        ] {
            for i in 0..n {
                put(
                    &c.join(format!("Drum Samples/{cat}/Gen {cat} {i:03}.wav")),
                    wav,
                );
            }
        }
        let gm = crate::flm::write_preset(&crate::song::ModuleInstance::new(2, 2)).unwrap();
        for i in 0..112 {
            put(
                &c.join(format!("Synth Presets2/GMSynth/Synth/Gen {i:03}.flmpst")),
                &gm,
            );
            put(
                &c.join(format!(
                    "Synth Presets2/GMSynth/Synth/Gen Old {i:03}.flgsynth"
                )),
                b"2nhC",
            );
        }
        for p in 0..42 {
            put(
                &c.join(format!("DirectWave Samples/Sliced Drums/Gen {p:02}.dwp")),
                b"DwPr",
            );
            for z in 0..40 {
                put(
                    &c.join(format!(
                        "DirectWave Samples/Sliced Drums/Gen {p:02}/z{z:02}.wav"
                    )),
                    wav,
                );
            }
        }
        let files = ProjectFiles::on_disk(&tmp.0);
        let timed = |what: &str, target, module_type, path: &[&str]| {
            let start = Instant::now();
            let rows = names(&files, target, module_type, path).len();
            let took = start.elapsed();
            println!("{what}: {rows} rows in {took:?}");
            assert!(took < Duration::from_millis(500), "{what}: {took:?}");
            rows
        };
        assert_eq!(
            timed(
                "GMSynth Synth (each .flmpst read)",
                PresetTarget::Module(2),
                Some(2),
                &["Synth"]
            ),
            224
        );
        assert_eq!(
            timed(
                "DW Sliced Drums",
                PresetTarget::Module(2),
                Some(1),
                &["Sliced Drums"]
            ),
            42
        );
        let start = Instant::now();
        let mut sampler = crate::ui::views::sample_picker::picker(&files);
        let rows = sampler.names_in(&*files.sample_library(), &["Snare"]).len();
        println!("Drum Samples / Snare: {rows} rows in {:?}", start.elapsed());
        assert_eq!(rows, 227);
        assert!(start.elapsed() < Duration::from_millis(500));
    }

    /// The Instruments picker indexes every instrument preset when it opens, reading each `.flmpst`'s `HEAD` (about
    /// 1,100 presets, 420 of them `.flmpst`): measured on a fake tree of that size.
    #[test]
    fn indexing_a_tree_the_size_of_the_original_when_the_instruments_picker_opens_stays_within_a_few_frames()
     {
        use crate::project_files::content::tests::put;
        use std::time::{Duration, Instant};
        let tmp = Temp::new("instruments-large");
        let c = tmp.0.join(CONTENT_FOLDER);
        let preset = |t: u16| crate::flm::write_preset(&ModuleInstance::new(t, 2)).unwrap();
        let (gm, ss, tb) = (preset(2), preset(23), preset(19));
        let mut files = 0;
        let mut add = |rel: String, bytes: &[u8]| {
            put(&c.join(rel), bytes);
            files += 1;
        };
        for (folder, n) in [
            ("Arps", 28),
            ("Bass", 75),
            ("Keys", 20),
            ("Lead", 81),
            ("Synth", 112),
        ] {
            for i in 0..n {
                let ext = if i % 2 == 0 { "flmpst" } else { "flgsynth" };
                let bytes: &[u8] = if i % 2 == 0 { &gm } else { b"2nhC" };
                add(
                    format!("Synth Presets2/GMSynth/{folder}/Gen {i:03}.{ext}"),
                    bytes,
                );
            }
        }
        for (folder, n) in [
            ("Arps", 17),
            ("Basses", 31),
            ("Leads", 28),
            ("Pads", 18),
            ("Synths", 34),
            ("Bells", 27),
        ] {
            for i in 0..n {
                add(
                    format!("Synth Presets2/SuperSaw/{folder}/Gen {i:03}.flmpst"),
                    &ss,
                );
            }
        }
        for i in 0..10 {
            add(
                format!("Synth Presets2/TransistorBass/Gen {i:02}.flmpst"),
                &tb,
            );
        }
        for (folder, n) in [
            ("Bass", 23),
            ("Keys", 9),
            ("Lead", 7),
            ("Pads", 9),
            ("Synth EXP", 100),
        ] {
            for i in 0..n {
                add(
                    format!("Synth Presets2/MiniSynth/{folder}/Gen {i:03}.flms"),
                    b"x",
                );
            }
        }
        for (folder, n) in [
            ("Bass", 41),
            ("Lead", 33),
            ("Orchestral", 42),
            ("Sequence", 53),
        ] {
            for i in 0..n {
                add(
                    format!("Synth Presets2/3xOsc/{folder}/Gen {i:03}.fst"),
                    b"x",
                );
            }
        }
        for (folder, n) in [
            ("Guitar", 9),
            ("Sliced Drums", 42),
            ("Synth Key", 15),
            ("Orchestral", 17),
        ] {
            for i in 0..n {
                add(
                    format!("DirectWave Samples/{folder}/Gen {i:02}.dwp"),
                    b"DwPr",
                );
                add(
                    format!("DirectWave Samples/{folder}/Gen {i:02}/z.wav"),
                    b"RIFF",
                );
            }
        }
        let files_made = files;
        let files = ProjectFiles::on_disk(&tmp.0);
        let start = Instant::now();
        let mut picker = picker(&files, PresetTarget::Instruments, None);
        let took = start.elapsed();
        println!("{files_made} files indexed in {took:?}");
        assert!(took < Duration::from_millis(500), "{took:?}");
        let library = files.preset_library(exts(PresetTarget::Instruments, None));
        let synth_exp = picker.names_in(&*library, &["Synth EXP"]);
        assert_eq!(synth_exp.len(), 100);
    }

    #[test]
    fn a_loop_or_a_shape_loads_on_the_worker_and_a_fst_on_the_ui_thread() {
        let file = Path::new;
        assert!(
            worker_loads(SLICER, file("Loop.wav")) && worker_loads(THREE_OSC, file("Shape.FLAC"))
        );
        assert!(!worker_loads(THREE_OSC, file("Bass.fst")));
        assert!(
            !worker_loads(2, file("Loop.wav")),
            "GMSynth's loader takes no audio file"
        );
    }

    #[test]
    fn a_factory_pick_loads_and_is_named_from_the_content_root() {
        let tmp = Temp::new("preset-pick");
        fake_tree(&tmp.0.join(CONTENT_FOLDER));
        let mut files = ProjectFiles::on_disk(&tmp.0);
        let mut song = Song::default();
        song.add_channel(crate::song::ChannelKind::Synth(SynthKind::GmSynth));
        song.add_channel(crate::song::ChannelKind::Drums);
        let id = song.channels[1].rack.modules[0].instance_id;
        let (theme, m) = (
            crate::theme::theme_by_id(0),
            crate::metrics::Metrics::from_pixels_per_point(1.0, 1),
        );
        let (mut audio, mut selected, mut settings) =
            (AudioLink::default(), 1, Settings::default());
        let mut picker = picker(&files, PresetTarget::Module(id), Some(2));
        picker.names_in(
            &*files.preset_library(exts(PresetTarget::Module(id), Some(2))),
            &["Bass"],
        );
        let path = picker.path_of("Gen Old").unwrap();
        let bytes = std::fs::read(&path).unwrap();
        let mut app = AppCtx {
            cx: UiCtx {
                theme: &theme,
                m: &m,
            },
            regions: Regions::new(
                egui::Rect::from_min_size(egui::Pos2::ZERO, egui::vec2(1170.0, 540.0)),
                &m,
            ),
            song: &mut song,
            audio: &mut audio,
            selected: &mut selected,
            snapshot: None,
            playing: false,
            armed: false,
            previewing: false,
            middle_c_is_c4: false,
            settings: &mut settings,
            ghost_notes: true,
            files: &mut files,
            edit: None,
            answer: None,
            covered: false,
            playhead: None,
            seek: None,
            midi_devices: &[],
            midi_rescan: false,
        };
        assert_eq!(
            load_module(&mut app, 1, id, Some((&path, &bytes)), None),
            Ok(0)
        );
        let module = &app.song.channels[1].rack.modules[0];
        assert_eq!(module.file.preset_name, "Gen Old");
        assert_eq!(module.file.category, "Synth Presets2/GMSynth/Bass");
        assert_eq!(
            module.file.preset_path,
            "Synth Presets2/GMSynth/Bass/Gen Old.flgsynth"
        );
        assert_eq!(app.song.channels[1].name, "Gen Old");
        // A `.fst` into 3x Osc: its params and its custom shape, named like any pick, one song edit.
        app.song
            .add_channel(crate::song::ChannelKind::Synth(SynthKind::ThreeXOsc));
        let osc = app.song.channels[3].rack.modules[0].instance_id;
        let fst = tmp
            .0
            .join(CONTENT_FOLDER)
            .join("Synth Presets2/3xOsc/Bass/Gen.fst");
        let bytes = std::fs::read(&fst).unwrap();
        assert_eq!(
            load_module(&mut app, 3, osc, Some((&fst, &bytes)), None),
            Ok(0)
        );
        let module = &app.song.channels[3].rack.modules[0];
        assert_eq!(
            module.params,
            crate::project_files::import_fst(&bytes, &fst).params
        );
        assert!(module.file.custom_data.is_some());
        // Its category is its folder's name.
        let file = &module.file;
        assert_eq!(
            (&*file.preset_name, &*file.category, &*file.provenance),
            ("Gen", "Bass", "Bass/Gen")
        );
        assert_eq!(file.preset_path, "Synth Presets2/3xOsc/Bass/Gen.fst");
        assert_eq!(app.song.channels[3].name, "Gen");
        // A kit from `Drum Templates`: its pads name imported content by `@R@`.
        let kit = tmp
            .0
            .join(CONTENT_FOLDER)
            .join("Drum Templates/Gen Kit.tmpl");
        let kit = crate::flm::read_drumset(&std::fs::read(kit).unwrap()).unwrap();
        assert_eq!(load_drumset(&mut app, 2, kit), Ok(0));
        let pad = app.song.channels[2].drum_kit.as_ref().unwrap().slots[0]
            .file
            .path
            .clone();
        let kick = ContentPath::Shorthand {
            root: ContentRoot::App,
            relative: "Drum Samples/Kick/Gen Kick.wav".into(),
        };
        assert_eq!(pad, Some(kick));
        let mut cache = crate::sample_cache::SampleCache::default();
        cache.load(app.song, &crate::sample_cache::reader(app.files));
        assert_eq!(cache.missing().count(), 0);
    }

    #[test]
    fn an_instruments_pick_loads_into_the_top_card_of_its_type_else_in_its_place_at_the_bottom() {
        let mut rack = Rack::default();
        let supersaw = rack.add(23);
        let fx = rack.add(6);
        assert_eq!(place_instrument(&mut rack, 23), (supersaw, false));
        assert_eq!(rack.modules.len(), 2);
        let (gm, changed) = place_instrument(&mut rack, 2);
        assert!(changed && gm > fx);
        let types: Vec<(u16, i32)> = rack
            .modules
            .iter()
            .map(|m| (m.type_id, m.instance_id))
            .collect();
        assert_eq!(types, [(6, fx), (2, gm)]);
        // An empty rack takes the instrument.
        let mut empty = Rack::default();
        let (id, changed) = place_instrument(&mut empty, 1);
        assert!(changed);
        assert_eq!(empty.modules[0].instance_id, id);
    }

    #[test]
    fn the_instruments_picker_indexes_the_content_folders_presets_before_ours() {
        let tmp = Temp::new("instruments-picker");
        fake_tree(&tmp.0.join(CONTENT_FOLDER));
        let files = ProjectFiles::on_disk(&tmp.0);
        let content = files.content_dir().unwrap();
        let sources = instrument_sources(&files);
        let ours = |f: &str| Path::new(content::ROOT).join("Presets").join(f);
        assert_eq!(
            sources,
            [
                (1, content.join("DirectWave Samples")),
                (39, content.join("Synth Presets2/3xOsc")),
                (2, content.join("Synth Presets2/GMSynth")),
                (0, content.join("Synth Presets2/MiniSynth")),
                (39, ours("3xOsc")),
                (1, ours("DW Sampler")),
                (2, ours("GMSynth")),
                (0, ours("MiniSynth")),
                (23, ours("SuperSaw")),
                (19, ours("TransistorBass")),
            ]
        );
        // The fake tree's: DW `Guitar` (its program, not its sample folder), GMSynth's and 3x Osc's `Bass`, MiniSynth's
        // `Keys` as `Keyboard`; GMSynth's `Gen Wrong`, a Reverb's preset, is not listed. Ours after, by category.
        let target = PresetTarget::Instruments;
        let mut picker = picker(&files, target, None);
        let library = files.preset_library(exts(target, None));
        let root = picker.names_in(&*library, &[]);
        assert_eq!(
            root,
            [
                MY_FILES, "Bass", "Bells", "Guitar", "Keyboard", "Lead", "Pad", "Pluck", "Synth"
            ]
        );
        let bass = picker.names_in(&*library, &["Bass"]);
        let mut sorted = bass.clone();
        sorted.sort();
        assert_eq!(
            bass, sorted,
            "by `<Category>/<Name>`, the instruments mixed"
        );
        for theirs in ["Gen", "Gen Bass", "Gen Old"] {
            assert!(bass.iter().any(|n| n == theirs), "{theirs}: {bass:?}");
        }
        assert!(!bass.iter().any(|n| n == "Gen Wrong"));
        let mut picker = super::picker(&files, target, None);
        assert_eq!(picker.names_in(&*library, &["Guitar"]), ["Gen Guitar"]);
        let mut picker = super::picker(&files, target, None);
        let keyboard = picker.names_in(&*library, &["Keyboard"]);
        assert!(keyboard.iter().any(|n| n == "Gen Keys"), "{keyboard:?}");
    }

    #[test]
    fn each_picker_lists_its_own_bank_of_ours() {
        let ours = |rel: &str| Some(Path::new(content::ROOT).join(rel));
        let module = |t: u16| built_in_folder(PresetTarget::Module(2), Some(t));
        assert_eq!(module(2), ours("Presets/GMSynth"));
        assert_eq!(module(19), ours("Presets/TransistorBass"));
        assert_eq!(module(35), ours("Presets/Reverb 2"));
        assert_eq!(module(999), None);
        assert_eq!(
            built_in_folder(PresetTarget::Drumset, None),
            ours("Drumsets")
        );
        assert_eq!(
            built_in_folder(PresetTarget::ChordSet, None),
            ours("Chord Sets")
        );
        for rack in [PresetTarget::RackReplace, PresetTarget::NewChannel(None)] {
            assert_eq!(built_in_folder(rack, None), None);
        }
    }

    /// Our eight chord sets, in the order the picker lists them.
    const OUR_SETS: [&str; 8] = [
        "Dark Minor",
        "Dorian Walk",
        "Epic Rise",
        "Gospel Nines",
        "Lo-Fi Haze",
        "Night Minor",
        "Pop Circle",
        "Soul Sevenths",
    ];

    #[test]
    fn the_chord_set_picker_lists_my_files_default_and_our_eight_then_the_content_folders() {
        let bare = Temp::new("chord-picker-bare");
        let files = ProjectFiles::on_disk(&bare.0);
        let rows = names(&files, PresetTarget::ChordSet, None, &[]);
        let expected: Vec<String> = [MY_FILES, DEFAULT]
            .into_iter()
            .chain(OUR_SETS)
            .map(String::from)
            .collect();
        assert_eq!(rows, expected);
        // Synthetic files only: a set in the user's `My Chord Sets`, and one in the content folder's `Chord Sets`,
        // which comes after ours. Another type in either is not listed.
        let tmp = Temp::new("chord-picker");
        let json = ChordSet::parse(DEFAULT_CHORD_JSON).unwrap().to_json();
        let at = |dir: &str, file: &str| tmp.0.join(dir).join(file);
        use crate::project_files::content::tests::put;
        put(&at("My Chord Sets", "Mine.chords"), json.as_bytes());
        put(&at("My Chord Sets", "Not a set.txt"), b"{}");
        put(
            &at("Original Content/Chord Sets", "Theirs.chords"),
            json.as_bytes(),
        );
        put(&at("Original Content/Chord Sets", "Sets.zip"), b"PK");
        let files = ProjectFiles::on_disk(&tmp.0);
        let rows = names(&files, PresetTarget::ChordSet, None, &[]);
        let expected: Vec<String> = [MY_FILES, DEFAULT]
            .into_iter()
            .chain(OUR_SETS)
            .chain(["Theirs"])
            .map(String::from)
            .collect();
        assert_eq!(rows, expected);
        assert_eq!(
            names(&files, PresetTarget::ChordSet, None, &[MY_FILES]),
            ["Mine"]
        );
        // The search keeps the rows whose name holds the query, in the root too.
        let mut picker = picker(&files, PresetTarget::ChordSet, None);
        let library = files.preset_library(exts(PresetTarget::ChordSet, None));
        picker.names_in(&*library, &[]);
        assert_eq!(
            picker
                .path_of("Theirs")
                .map(|p| p.ends_with("Chord Sets/Theirs.chords")),
            Some(true)
        );
        assert!(
            picker
                .path_of("Soul Sevenths")
                .is_some_and(|p| content::is_factory(&p))
        );
    }

    #[test]
    fn a_modules_or_a_racks_picker_stays_open_on_a_pick_and_the_others_close() {
        // The module preset button and a rack preset opened from the rack stay open; the drumsets, the chord sets and a
        // new Rack channel's preset (the playlist's) close.
        use crate::ui::modules_browser::InsertAt;
        for target in [
            PresetTarget::Module(2),
            PresetTarget::RackReplace,
            PresetTarget::RackInsert(InsertAt::End),
        ] {
            assert!(stays_open(target), "{target:?}");
        }
        for target in [
            PresetTarget::Drumset,
            PresetTarget::ChordSet,
            PresetTarget::NewChannel(None),
        ] {
            assert!(!stays_open(target), "{target:?}");
        }
    }

    /// The Instruments picker's `My Files` is the whole user folder, in which every fixed folder is a place (the songs
    /// folder, the bin, `Unzipped songs`, imported content, and each user folder: deleting `My Presets` could not
    /// be restored, renaming `My Samples` would break every song's `@L@` paths): its ≡ `Delete`, `Rename`, and `Cut`
    /// then `Paste` refuse each, moving nothing. A folder in one is the user's, and renames.
    #[cfg(not(target_arch = "wasm32"))]
    #[test]
    fn the_instruments_pickers_file_actions_never_delete_rename_or_move_a_place() {
        use crate::project_files::{
            MIDI_FOLDER, RECORDINGS_FOLDER, SAMPLES_FOLDER, SOUNDFONTS_FOLDER, UNZIPPED_FOLDER,
        };
        use crate::ui::main_menu::library::RECENTLY_DELETED;
        use crate::ui::main_menu::{FileActions, Opt, SONGS_FOLDER, Step};
        let tmp = Temp::new("fd4-instr-places");
        let places = [
            SONGS_FOLDER,
            RECENTLY_DELETED,
            UNZIPPED_FOLDER,
            CONTENT_FOLDER,
            PRESETS_FOLDER,
            RACKS_FOLDER,
            DRUMSETS_FOLDER,
            CHORD_SETS_FOLDER,
            SAMPLES_FOLDER,
            RECORDINGS_FOLDER,
            SOUNDFONTS_FOLDER,
            MIDI_FOLDER,
            "My Tracks",
        ];
        for folder in places.into_iter().chain(["Mine"]) {
            std::fs::create_dir_all(tmp.0.join(folder).join("F")).unwrap();
        }
        let mut files = ProjectFiles::on_disk(&tmp.0);
        let target = PresetTarget::Instruments;
        let (user, mine) = (Some(tmp.0.clone()), Some(tmp.0.join("Mine")));
        files.with_preset_files(top(target, None), exts(target, None), |lib| {
            let listed = lib.list(Some(&tmp.0)).unwrap();
            let refused = |step| matches!(step, Step::Changed(Err(_)));
            for place in places {
                let e = listed.iter().find(|e| e.name == place).expect(place);
                let mut actions = FileActions::default();
                let delete = actions.apply(Opt::Delete, vec![e.clone()], user.clone(), lib);
                assert!(refused(delete), "{place}");
                assert!(
                    actions.name(Some(e), "Old", user.as_deref(), lib).is_err(),
                    "{place}"
                );
                actions.apply(Opt::Cut, vec![e.clone()], user.clone(), lib);
                assert!(
                    refused(actions.apply(Opt::Paste, vec![], mine.clone(), lib)),
                    "{place}"
                );
                assert!(tmp.0.join(place).join("F").is_dir(), "{place}");
                assert_eq!(actions.take_moves(), [], "{place}");
            }
            let songs = tmp.0.join(SONGS_FOLDER);
            let f = &lib.list(Some(&songs)).unwrap()[0];
            let mut actions = FileActions::default();
            actions.name(Some(f), "Old", Some(&songs), lib).unwrap();
            assert_eq!(
                actions.take_moves(),
                [(f.path.clone(), Some(songs.join("Old")))]
            );
        });
        assert_eq!(std::fs::read_dir(tmp.0.join("Mine")).unwrap().count(), 1);
    }

    /// `Restore` in the Instruments picker's bin (its `My Files` is the whole user folder, the root itself) puts a user
    /// folder's wrapper back as what it holds, into the folder that is there, never beside it as `My Songs (1)` or
    /// `GMSynth (1)`.
    #[cfg(not(target_arch = "wasm32"))]
    #[test]
    fn restore_in_the_instruments_pickers_bin_puts_files_back_into_their_folders() {
        use crate::project_files::content::tests::put;
        use crate::ui::main_menu::library::RECENTLY_DELETED;
        use crate::ui::main_menu::{FileActions, Opt, SONGS_FOLDER, Step};
        let target = PresetTarget::Instruments;
        // What is restored, below the bin; the file it holds, below the user folder, once back.
        let cases = [
            ("My Songs", "My Songs/Old.flm"),
            ("My Presets", "My Presets/GMSynth/B.flmpst"),
            ("My Presets/GMSynth", "My Presets/GMSynth/B.flmpst"),
        ];
        for (i, (picked, file)) in cases.into_iter().enumerate() {
            let tmp = Temp::new(&format!("fd4-instr-restore-{i}"));
            let bin = tmp.0.join(RECENTLY_DELETED);
            put(&bin.join(file), b"x");
            put(&tmp.0.join(SONGS_FOLDER).join("A.flm"), b"a");
            put(&tmp.0.join("My Presets/GMSynth/A.flmpst"), b"a");
            let mut files = ProjectFiles::on_disk(&tmp.0);
            files.with_preset_files(top(target, None), exts(target, None), |lib| {
                let path = bin.join(picked);
                let parent = path.parent().unwrap().to_owned();
                let listed = lib.list(Some(&parent)).unwrap();
                let e = listed.iter().find(|e| e.path == path).expect(picked);
                let mut actions = FileActions::default();
                let step = actions.apply(Opt::Restore, vec![e.clone()], Some(parent), lib);
                assert_eq!(step, Step::Changed(Ok(())), "{picked}");
            });
            assert_eq!(std::fs::read(tmp.0.join(file)).unwrap(), b"x", "{picked}");
            let mut top_level: Vec<_> = (std::fs::read_dir(&tmp.0).unwrap())
                .map(|e| e.unwrap().file_name().into_string().unwrap())
                .collect();
            top_level.sort();
            assert_eq!(
                top_level,
                ["My Presets", "My Songs", RECENTLY_DELETED],
                "{picked}"
            );
            let presets = tmp.0.join("My Presets");
            assert_eq!(std::fs::read_dir(presets).unwrap().count(), 1, "{picked}");
        }
    }

    #[test]
    fn the_chord_set_picker_is_titled_and_lists_my_chord_sets_in_the_user_folder_on_disk() {
        let tmp = Temp::new("chord-top");
        let mut files = ProjectFiles::on_disk(&tmp.0);
        let dir = tmp.0.join("My Chord Sets");
        assert_eq!(files.chord_sets_dir(), Some(dir.clone()));
        use crate::project_files::content::tests::put;
        put(&dir.join("Mine.chords"), b"{}");
        put(&dir.join("Other.txt"), b"x");
        let target = PresetTarget::ChordSet;
        assert_eq!(top(target, None), "My Chord Sets");
        assert_eq!(picker(&files, target, None).root_title(), "Chord Sets");
        let wanted = exts(target, None);
        let listed = files.with_preset_files(top(target, None), wanted, |library| {
            let rows = library.list(Some(&dir)).unwrap();
            (
                library.root().map(Path::to_owned),
                rows.into_iter().map(|e| e.name).collect::<Vec<_>>(),
            )
        });
        assert_eq!(listed, (Some(dir), vec!["Mine.chords".to_owned()]));
    }

    #[test]
    fn a_chord_set_pick_is_stored_as_normalised_json_named_by_its_file() {
        let default = KeyboardState::default();
        assert_eq!(chord_set(None), Ok((default.chord_json, "Major".into())));
        // One of ours, read from the factory as the picker does.
        let ours = Path::new(content::ROOT).join("Chord Sets/Epic Rise.chords");
        let bytes = content::factory().read(&ours).unwrap();
        let (json, name) = chord_set(Some((&ours, &bytes))).unwrap();
        assert_eq!((json.as_bytes(), name.as_str()), (&bytes[..], "Epic Rise"));
        // A user file written another way (nine chords, keys in another order) comes back in the chord set file layout
        // with eight, named by its stem, and that text reads again.
        let chord = r#"{"quality":1,"octave":3,"baseNote":9,"extensions":[10]}"#;
        let loose = format!(r#"{{"rootNote":0,"chords":[{}]}}"#, [chord; 9].join(","));
        let mine = Path::new("My Chord Sets/Some.Thing.chords");
        let (json, name) = chord_set(Some((mine, loose.as_bytes()))).unwrap();
        assert_eq!(name, "Some.Thing");
        let set = ChordSet::parse(&json).unwrap();
        assert_eq!((set.chords.len(), json == set.to_json()), (8, true));
        assert!(json.starts_with("{\n    \"chords\": [\n        {\n"));
        // Bad files: the error is the parser's, or the text's.
        let seven = format!(r#"{{"rootNote":0,"chords":[{}]}}"#, [chord; 7].join(","));
        let err = chord_set(Some((mine, seven.as_bytes()))).unwrap_err();
        assert_eq!(err, "A chord set needs 8 chords");
        assert!(chord_set(Some((mine, b"\xff\xfe"))).is_err());
        assert!(chord_set(Some((mine, b"not json"))).is_err());
    }

    /// Runs `f` on an app context over `song` and `files`.
    fn with_app<R>(
        song: &mut Song,
        files: &mut ProjectFiles,
        f: impl FnOnce(&mut AppCtx) -> R,
    ) -> R {
        let (theme, m) = (
            crate::theme::theme_by_id(0),
            crate::metrics::Metrics::from_pixels_per_point(1.0, 1),
        );
        let (mut audio, mut selected, mut settings) =
            (AudioLink::default(), 1, Settings::default());
        let mut app = AppCtx {
            cx: UiCtx {
                theme: &theme,
                m: &m,
            },
            regions: Regions::new(
                egui::Rect::from_min_size(egui::Pos2::ZERO, egui::vec2(1170.0, 540.0)),
                &m,
            ),
            song,
            audio: &mut audio,
            selected: &mut selected,
            snapshot: None,
            playing: false,
            armed: false,
            previewing: false,
            middle_c_is_c4: false,
            settings: &mut settings,
            ghost_notes: true,
            files,
            edit: None,
            answer: None,
            covered: false,
            playhead: None,
            seek: None,
            midi_devices: &[],
            midi_rescan: false,
        };
        f(&mut app)
    }

    /// A pick of `path` for module `id` of type `type_id` on channel `channel`, as `start` makes one, but not loading.
    fn loading(app: &AppCtx, channel: usize, id: i32, type_id: u16, path: PathBuf) -> Loading {
        Loading {
            channel_id: app.song.channels[channel].id,
            id,
            type_id,
            program: app.files.shorthand(&path),
            path,
            done: mpsc::channel().1,
            instruments: false,
            fallback: None,
        }
    }

    /// What a worker made of a pick, with nothing decoded.
    fn done(picked: Picked) -> Done {
        Done {
            picked,
            samples: SampleCache::default(),
        }
    }

    /// A DW program from the content folder, picked on the worker, is stored (its `PATh`) and named exactly as the
    /// non-worker loader does it: `@R@`, and a category relative to the content folder, not `@L@Original Content/…`.
    #[test]
    fn a_factory_dwp_picked_on_the_worker_is_stored_and_named_as_the_module_loader_does() {
        use crate::flm::chunk::{ContentPath, ContentRoot};
        use honeydew_audio::dwp::Cstm;
        let tmp = Temp::new("preset-pick-dw");
        fake_tree(&tmp.0.join(CONTENT_FOLDER));
        let mut files = ProjectFiles::on_disk(&tmp.0);
        let mut song = Song::default();
        for _ in 0..2 {
            song.add_channel(crate::song::ChannelKind::Synth(SynthKind::DwSampler));
        }
        song.add_channel(crate::song::ChannelKind::Synth(SynthKind::Slicer));
        song.add_channel(crate::song::ChannelKind::Synth(SynthKind::SoundFontPlayer));
        song.add_channel(crate::song::ChannelKind::Synth(SynthKind::DwSampler));
        let ids = [1, 2, 3, 4, 5].map(|c| song.channels[c].rack.modules[0].instance_id);
        let content = files.content_dir().unwrap();
        let path = content.join("DirectWave Samples/Guitar/Gen Guitar.dwp");
        let bytes = std::fs::read(&path).unwrap();
        let program_of = |m: &ModuleInstance| {
            let blob = m.file.custom_data.as_deref().unwrap();
            Cstm::read(blob).program.map(|p| p.path.to_vec())
        };
        with_app(&mut song, &mut files, |app| {
            // Channel 1: the non-worker path (`load_pick`, which the other module types still take).
            assert_eq!(
                load_module(app, 1, ids[0], Some((&path, &bytes)), None),
                Ok(0)
            );
            // Channel 2: the worker's.
            let channel_id = app.song.channels[2].id;
            let loading = start(
                app.files,
                channel_id,
                ids[1],
                1,
                path.clone(),
                bytes.clone(),
                None,
            );
            let done = loading.done.recv().unwrap();
            let mut rack = rack::State::default();
            assert_eq!(finish(app, &mut rack, 2, loading, done, None), Ok(0));
            let [module, worker] = [1, 2].map(|c| app.song.channels[c].rack.modules[0].clone());
            let want = b"@R@\0DirectWave Samples/Guitar/Gen Guitar.dwp".to_vec();
            assert_eq!(program_of(&module), Some(want.clone()));
            assert_eq!(program_of(&worker), Some(want));
            let named = |m: &ModuleInstance| {
                let f = &m.file;
                [&f.category, &f.preset_name, &f.preset_path, &f.provenance].map(String::clone)
            };
            assert_eq!(named(&worker), named(&module));
            assert_eq!(
                named(&worker),
                [
                    "DirectWave Samples/Guitar",
                    "Gen Guitar",
                    "DirectWave Samples/Guitar/Gen Guitar.dwp",
                    "DirectWave Samples/Guitar/Gen Guitar",
                ]
            );
            // `PRST`'s source path is relative to the content root for a content file: a song never carries where the
            // folder is on this machine.
            let relative = "DirectWave Samples/Guitar/Gen Guitar.dwp";
            assert_eq!(worker.file.source_path, relative);
            assert_eq!(module.file.source_path, relative);
            let names = [1, 2].map(|c| app.song.channels[c].name.clone());
            assert_eq!(names, ["Gen Guitar", "Gen Guitar"]);
            // The worker decoded the program and its zone file under the same `@R@` keys the engine build asks for.
            let app_path = |relative: &str| ContentPath::Shorthand {
                root: ContentRoot::App,
                relative: relative.into(),
            };
            let samples = &mut app.audio.samples;
            assert!(
                samples
                    .program(&app_path("DirectWave Samples/Guitar/Gen Guitar.dwp"))
                    .is_some()
            );
            let zone = crate::sample_cache::SampleKey::File(app_path(
                "DirectWave Samples/Guitar/Gen Guitar/Gen Zone.wav",
            ));
            assert_eq!(samples.get(&zone).map(|s| s.frames()), Some(500));
            samples.load(app.song, &|path| panic!("the build read {path:?}"));
            assert_eq!(samples.missing().count(), 0);
            // A Slicer's loop from the content folder, which the non-worker loader names the same way.
            let looped = content.join("Loop Samples/Sliced Drums/Gen Loop.flac");
            let flac = std::fs::read(&looped).unwrap();
            assert_eq!(
                load_module(app, 3, ids[2], Some((&looped, &flac)), None),
                Ok(0)
            );
            let slicer = &app.song.channels[3].rack.modules[0];
            assert_eq!(
                slicer.file.source_path,
                "Loop Samples/Sliced Drums/Gen Loop.flac"
            );
            assert_eq!(slicer.file.category, "Loop Samples/Sliced Drums");
            // A SoundFont from the content folder, picked on the worker (its `.sf2` arm): its `SF2D` is `@R@`, and it
            // is named the same way.
            let font = content.join("SoundFonts/BAS Sine.sf2");
            let bytes = std::fs::read(&font).unwrap();
            let channel_id = app.song.channels[4].id;
            let loading = start(app.files, channel_id, ids[3], 52, font.clone(), bytes, None);
            let done = loading.done.recv().unwrap();
            let mut rack = rack::State::default();
            assert_eq!(finish(app, &mut rack, 4, loading, done, None), Ok(0));
            let player = &app.song.channels[4].rack.modules[0];
            let sf2d = crate::flm::sf2d::Sf2d::read(player.file.custom_data.as_deref().unwrap());
            assert_eq!(
                sf2d.map(|s| s.path),
                Some(app_path("SoundFonts/BAS Sine.sf2"))
            );
            assert_eq!(player.file.category, "SoundFonts");
            assert_eq!(player.file.source_path, "SoundFonts/BAS Sine.sf2");
            // The user's own program, through the worker too: named from the user folder, its source path relative.
            let mine = tmp.0.join("My Presets/DW Sampler/Mine.dwp");
            std::fs::create_dir_all(mine.parent().unwrap()).unwrap();
            let program = std::fs::read(&path).unwrap();
            std::fs::write(&mine, &program).unwrap();
            let channel_id = app.song.channels[5].id;
            let loading = start(app.files, channel_id, ids[4], 1, mine, program, None);
            let done = loading.done.recv().unwrap();
            assert_eq!(
                finish(app, &mut rack::State::default(), 5, loading, done, None),
                Ok(0)
            );
            let user = &app.song.channels[5].rack.modules[0].file;
            assert_eq!(user.category, "My Presets/DW Sampler");
            assert_eq!(user.source_path, "My Presets/DW Sampler/Mine.dwp");
            // Saved, nothing of the song names the folder it was picked from.
            let saved = crate::flm::write(app.song).unwrap();
            let here = tmp.0.to_str().unwrap().as_bytes();
            assert!(
                !saved.windows(here.len()).any(|w| w == here),
                "an absolute path was saved"
            );
            let reread = crate::flm::read(&saved).unwrap();
            let again = &reread.channels[2].rack.modules[0].file;
            assert_eq!(
                (again.source_path.as_str(), again.preset_path.as_str()),
                (relative, relative)
            );
        });
    }

    #[test]
    fn a_pick_of_ours_is_named_from_the_factory_root() {
        let tmp = Temp::new("our-pick");
        let mut files = ProjectFiles::on_disk(&tmp.0);
        let mut song = Song::default();
        song.add_channel(crate::song::ChannelKind::Synth(SynthKind::GmSynth));
        song.add_channel(crate::song::ChannelKind::Synth(SynthKind::ThreeXOsc));
        let gm = song.channels[1].rack.modules[0].instance_id;
        let osc = song.channels[2].rack.modules[0].instance_id;
        with_app(&mut song, &mut files, |app| {
            let ours = Path::new(content::ROOT).join("Presets/GMSynth/Bass/Round.flmpst");
            let bytes = crate::flm::write_preset(&ModuleInstance::new(2, 2)).unwrap();
            assert_eq!(load_module(app, 1, gm, Some((&ours, &bytes)), None), Ok(0));
            let module = &app.song.channels[1].rack.modules[0];
            assert_eq!(module.file.preset_name, "Round");
            assert_eq!(module.file.category, "Presets/GMSynth/Bass");
            assert_eq!(module.file.preset_path, "Presets/GMSynth/Bass/Round.flmpst");
            assert_eq!(app.song.channels[1].name, "Round");
            // A module whose picks load on a worker is named the same way.
            let ours = Path::new(content::ROOT).join("Presets/3xOsc/Lead/Edge.flmpst");
            let preset = crate::flm::read_preset(
                &crate::flm::write_preset(&ModuleInstance::new(39, 2)).unwrap(),
                Some(39),
            )
            .unwrap();
            let loading = loading(app, 2, osc, 39, ours);
            let done = done(Picked::Preset(Box::new(preset)));
            assert_eq!(
                finish(app, &mut rack::State::default(), 2, loading, done, None),
                Ok(0)
            );
            let module = &app.song.channels[2].rack.modules[0];
            assert_eq!(module.file.category, "Presets/3xOsc/Lead");
            assert_eq!(module.file.preset_path, "Presets/3xOsc/Lead/Edge.flmpst");
        });
    }

    /// A program or loop of ours names itself `@R@Built-in/…`, never `@L@`: on the desktop, and on the web, whose user
    /// folder is the empty path.
    #[test]
    fn a_program_or_loop_pick_of_ours_is_stored_as_its_app_shorthand() {
        use crate::project_files::mem::MemFiles;
        let tmp = Temp::new("our-files");
        let ours = |rel: &str| Path::new(content::ROOT).join(rel);
        let holds = |blob: &Option<Vec<u8>>, rel: &str| {
            let want = [&b"@R@\0Built-in/"[..], rel.as_bytes()].concat();
            blob.as_deref()
                .is_some_and(|b| b.windows(want.len()).any(|w| w == want))
        };
        for mut files in [
            ProjectFiles::on_disk(&tmp.0),
            ProjectFiles::with_provider(Box::new(MemFiles::default())),
        ] {
            let app_path = ContentPath::Shorthand {
                root: ContentRoot::App,
                relative: "Built-in/Loops/Beat.wav".into(),
            };
            assert_eq!(files.shorthand(&ours("Loops/Beat.wav")), Some(app_path));
            let mut song = Song::default();
            song.add_channel(crate::song::ChannelKind::Synth(SynthKind::DwSampler));
            song.add_channel(crate::song::ChannelKind::Synth(SynthKind::Slicer));
            let dw = song.channels[1].rack.modules[0].instance_id;
            let slicer = song.channels[2].rack.modules[0].instance_id;
            with_app(&mut song, &mut files, |app| {
                let loading = loading(app, 1, dw, 1, ours("Presets/DW Sampler/Gen.dwp"));
                let done = done(Picked::Program(None));
                assert_eq!(
                    finish(app, &mut rack::State::default(), 1, loading, done, None),
                    Ok(0)
                );
                let beat = ours("Loops/Beat.wav");
                assert_eq!(
                    load_module(app, 2, slicer, Some((&beat, b"RIFF")), None),
                    Ok(0)
                );
            });
            let blob = |c: usize| &song.channels[c].rack.modules[0].file.custom_data;
            assert!(holds(blob(1), "Presets/DW Sampler/Gen.dwp"));
            assert!(holds(blob(2), "Loops/Beat.wav"));
        }
    }
}
