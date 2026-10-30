//! Module and rack presets, and drum kits. The card ≡ menu's `Save preset`, the rack ≡ menu's `Save Rack
//! preset` and the drums track menu's `Save Drumset` open the preset dialog ([`ProjectFiles::ask_preset`]); its `SAVE`
//! ([`ProjectFiles::save_preset`]) writes the file through the [`super::FileProvider`], as a song is written, without
//! asking where:
//! - a module preset to `My Presets/<module folder>/<name>.flmpst` ([`preset_folder`]);
//! - a rack preset to `My Racks/<name>.flmrack`;
//! - a drum kit to `My Drumsets/<name>.tmpl`.
//!
//! The chord set picker lists `My Chord Sets` (`.chords`, [`ProjectFiles::chord_sets_dir`]). The keyboard settings'
//! `ChordSet` ≡ `Save` saves the set in use there ([`ProjectFiles::ask_chord_set`]):
//! the dialog titled `New Chord Set` proposes `Custom Chord Set`, and the file's name becomes the set's.
//!
//! The folders are in the user folder: the one holding `My Songs` on the desktop, the empty path on the web, where
//! the file is downloaded and this session keeps a copy. The dialog proposes a free name; a typed name that is taken
//! asks first (`Replace files` / `Keep both` / `Cancel`). Once written, a module
//! preset names the module it was saved from ([`ProjectFiles::take_preset_named`]). For the preset pickers,
//! [`ProjectFiles::preset_library`] lists the user's presets: [`ProjectFiles::user_files`] lists a folder and
//! [`ProjectFiles::read_user_file`] reads a file back.

use std::io;
use std::path::{Path, PathBuf};

use super::{
    FileEvent, MAX_FILE_BYTES, OVERWRITE, ProjectFiles, SAVE_FILE_FAILED, SAVED, read_limited,
};
use crate::content;
use crate::song::chords::ChordSet;
use crate::song::{DrumKit, ModuleInstance, Song};
use crate::ui::main_menu::{SongEntry, SongLibrary};
use crate::ui::rack::{instrument, name_song_channel};
use crate::ui::views::Answer;

/// The user folders presets go to.
pub const PRESETS_FOLDER: &str = "My Presets";
pub const RACKS_FOLDER: &str = "My Racks";
pub const DRUMSETS_FOLDER: &str = "My Drumsets";
/// The folder of the user's chord sets, and their file type.
pub const CHORD_SETS_FOLDER: &str = "My Chord Sets";
/// The file types.
pub const PRESET_EXT: &str = "flmpst";
pub const RACK_EXT: &str = "flmrack";
pub const DRUMSET_EXT: &str = "tmpl";
pub const CHORDS_EXT: &str = "chords";
/// The name the chord set dialog proposes.
pub const CUSTOM_CHORD_SET: &str = "Custom Chord Set";
/// The legacy preset types a module's picker also lists: MiniSynth's and GMSynth's.
pub const FLMS_EXT: &str = "flms";
pub const FLGSYNTH_EXT: &str = "flgsynth";

/// The files a module of type `type_id` loads from its picker: its presets ([`crate::flm::read_preset`]: `.flmpst`, and
/// MiniSynth's `.flms` or GMSynth's `.flgsynth`), and the files its loader names in its `CSTM` or `SF2D`
/// ([`super::load_pick`]): a DW program `.dwp` or an audio file for the DW Sampler, an audio file for the Slicer
/// and 3x Osc, a `.sf2` for the SoundFont Player; and 3x Osc's FL desktop
/// presets, `.fst` ([`super::import_fst`]).
pub fn preset_exts(type_id: u16) -> &'static [&'static str] {
    match type_id {
        0 => &[PRESET_EXT, FLMS_EXT],
        2 => &[PRESET_EXT, FLGSYNTH_EXT],
        1 => &[PRESET_EXT, "dwp", "wav", "flac", "mp3", "ogg"],
        33 => &[PRESET_EXT, "wav", "flac", "mp3", "ogg"],
        39 => &[PRESET_EXT, super::FST_EXT, "wav", "flac", "mp3", "ogg"],
        52 => &[PRESET_EXT, super::SF2_EXT, super::SFZ_EXT],
        _ => &[PRESET_EXT],
    }
}

/// Which preset dialog is open, or the audio editor's one-type dialog.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum PresetKind {
    /// A card's `Save preset`.
    Module,
    /// The rack's `Save Rack preset`.
    Rack,
    /// A drums channel's `Save Drumset`.
    Drumset,
    /// The audio editor's `New sample name`: no preset, the edited sample's `.wav`.
    Sample,
    /// The keyboard settings' `ChordSet` ≡ `Save`.
    ChordSet,
}

impl PresetKind {
    /// The dialog's title.
    pub fn title(self) -> &'static str {
        match self {
            Self::Module => "Save preset",
            Self::Rack => "Save Rack preset",
            Self::Drumset => "Save Drumset",
            Self::Sample => "New sample name",
            Self::ChordSet => "New Chord Set",
        }
    }

    /// The line naming the file type, without a product name.
    pub fn format(self) -> &'static str {
        match self {
            Self::Module => "Sound Module Preset (.flmpst)",
            Self::Rack => "Rack Preset (.flmrack)",
            Self::Drumset => "Drumset (.tmpl)",
            Self::Sample => "Audio Wave (.wav)",
            Self::ChordSet => "Chord Set (.chords)",
        }
    }

    fn ext(self) -> &'static str {
        match self {
            Self::Module => PRESET_EXT,
            Self::Rack => RACK_EXT,
            Self::Drumset => DRUMSET_EXT,
            Self::Sample => "wav",
            Self::ChordSet => CHORDS_EXT,
        }
    }
}

/// What a preset dialog saves, as it was when the dialog opened.
// One at a time, never in a collection: the variants' sizes do not matter.
#[allow(clippy::large_enum_variant)]
#[derive(Clone, Debug, PartialEq)]
pub enum PresetSource {
    /// A card's module.
    Module(ModuleInstance),
    /// A rack's modules in chain order, and its channel's name, which the dialog proposes.
    Rack {
        name: String,
        modules: Vec<ModuleInstance>,
    },
    /// A drums channel's kit, and the channel's name, which the dialog proposes.
    Drumset { name: String, kit: DrumKit },
    /// The song's chord set in use.
    ChordSet(ChordSet),
}

impl PresetSource {
    pub fn kind(&self) -> PresetKind {
        match self {
            Self::Module(_) => PresetKind::Module,
            Self::Rack { .. } => PresetKind::Rack,
            Self::Drumset { .. } => PresetKind::Drumset,
            Self::ChordSet(_) => PresetKind::ChordSet,
        }
    }
}

/// The factory preset folders whose names are not the module's display name: the `Effect Presets` and
/// `Synth Presets2` folders.
const FACTORY_FOLDERS: [(u16, &str); 7] = [
    (4, "Multi FX"),
    (19, "TransistorBass"),
    (20, "Leveler"),
    (28, "Limiter 2"),
    (29, "Autoduck2"),
    (36, "MBand Comp"),
    (39, "3xOsc"),
];

/// The folder in `My Presets` of module type `type_id`: its factory folder's name (the category is
/// The folder), else its display name. Reverb (32) has no factory folder and loads the legacy Reverb's presets,
/// so it shares `Reverb` with it. `None` for an unknown type.
pub fn preset_folder(type_id: u16) -> Option<&'static str> {
    let spec = crate::params::module_spec(type_id)?;
    let factory = FACTORY_FOLDERS.iter().find(|&&(t, _)| t == type_id);
    Some(factory.map_or(spec.display_name, |&(_, folder)| folder))
}

impl ProjectFiles {
    /// `My Presets/<module folder>` of module type `type_id`. `None` without a user folder or for an unknown type.
    pub fn presets_dir(&self, type_id: u16) -> Option<PathBuf> {
        let folder = preset_folder(type_id)?;
        Some(self.user.as_ref()?.join(PRESETS_FOLDER).join(folder))
    }

    /// `My Racks`. `None` without a user folder.
    pub fn racks_dir(&self) -> Option<PathBuf> {
        Some(self.user.as_ref()?.join(RACKS_FOLDER))
    }

    /// `My Drumsets`. `None` without a user folder.
    pub fn drumsets_dir(&self) -> Option<PathBuf> {
        Some(self.user.as_ref()?.join(DRUMSETS_FOLDER))
    }

    /// `My Chord Sets`. `None` without a user folder.
    pub fn chord_sets_dir(&self) -> Option<PathBuf> {
        Some(self.user.as_ref()?.join(CHORD_SETS_FOLDER))
    }

    fn dir_of(&self, source: &PresetSource) -> Option<PathBuf> {
        match source {
            PresetSource::Module(module) => self.presets_dir(module.type_id),
            PresetSource::Rack { .. } => self.racks_dir(),
            PresetSource::Drumset { .. } => self.drumsets_dir(),
            PresetSource::ChordSet(_) => self.chord_sets_dir(),
        }
    }

    /// Opens the preset dialog for `source`, from the channel with id `channel`, after the frame, proposing a free
    /// name: the module's preset name (its folder's while it is `Default`), or the rack's or kit's channel name.
    pub fn ask_preset(&mut self, source: PresetSource, channel: i32) {
        let base = match &source {
            PresetSource::Module(module) => match module.file.preset_name.trim() {
                "" | "Default" => preset_folder(module.type_id).unwrap_or_default(),
                name => name,
            },
            PresetSource::Rack { name, .. } | PresetSource::Drumset { name, .. } => name.trim(),
            PresetSource::ChordSet(_) => CUSTOM_CHORD_SET,
        };
        let dir = self.dir_of(&source);
        let name = self.free_name(dir.as_deref(), base, source.kind().ext(), None);
        self.preset_dialog = Some(name);
        self.preset = Some((source, channel));
    }

    /// Opens the chord set dialog (`New Chord Set`) for `set`, proposing a free `Custom Chord Set` name in `My Chord
    /// Sets`.
    pub fn ask_chord_set(&mut self, set: ChordSet) {
        self.ask_preset(PresetSource::ChordSet(set), 0);
    }

    /// `SAVE` in the preset dialog with the file name `name`: checks it as the Save dialog does, then writes the
    /// preset into its folder under that name. A name that is taken asks first ([`OVERWRITE_TAG`]): the dialog stays
    /// open until the answer, which [`Self::update`] takes. Returns whether the dialog closes: a bad name keeps it
    /// open (behind the message), an empty one does nothing.
    ///
    /// [`OVERWRITE_TAG`]: super::OVERWRITE_TAG
    pub fn save_preset(&mut self, name: &str) -> bool {
        let Some(kind) = self.preset.as_ref().map(|(s, _)| s.kind()) else {
            return true;
        };
        let Some(name) = self.typed_name(name, kind.ext()) else {
            return false;
        };
        let Some(dir) = self.preset.as_ref().and_then(|(s, _)| self.dir_of(s)) else {
            self.preset = None;
            self.fail(SAVE_FILE_FAILED, "There is no user folder.".into());
            return true;
        };
        let file = format!("{name}.{}", kind.ext());
        if self.taken(&dir.join(&file)) {
            self.message = Some(vec![OVERWRITE.into(), file]);
            self.overwrite = Some(name);
            return false;
        }
        self.write_preset(&dir, name);
        true
    }

    /// The overwrite question for the typed name `name` was answered: `Replace files` writes over the file, `Keep
    /// both` writes under the first free `name (n)`, `Cancel` leaves the dialog open.
    pub(super) fn overwrite_answered(&mut self, name: String, answer: Answer) {
        let Some((kind, dir)) =
            (self.preset.as_ref()).and_then(|(s, _)| Some((s.kind(), self.dir_of(s)?)))
        else {
            return;
        };
        match answer {
            Answer::Extra => self.write_preset(&dir, name),
            Answer::Yes => {
                let free = self.free_name(Some(&dir), &name, kind.ext(), None);
                self.write_preset(&dir, free);
            }
            Answer::No => {}
        }
    }

    /// Writes the dialog's preset as `name` in `dir`, its folder, and lets go of it. A module preset's `DESC` and
    /// `PRST` name the file ([`ModuleInstance::name_preset`]), and so will the module's once it is written.
    fn write_preset(&mut self, dir: &Path, name: String) {
        let Some((source, channel)) = self.preset.take() else {
            return;
        };
        let path = dir.join(format!("{name}.{}", source.kind().ext()));
        let (written, card) = match source {
            PresetSource::Module(mut module) => {
                module.name_preset(&path, self.user.as_deref().unwrap_or(Path::new("")));
                let card = (channel, module.instance_id);
                (crate::flm::write_preset(&module), Some(card))
            }
            PresetSource::Rack { modules, .. } => (crate::flm::write_rack(&modules), None),
            PresetSource::Drumset { kit, .. } => (crate::flm::write_drumset(&kit), None),
            PresetSource::ChordSet(set) => (Ok(set.to_json().into_bytes()), None),
        };
        match written {
            Ok(bytes) => {
                let copy = self.copies.then(|| bytes.clone());
                self.preset_saving = Some((path.clone(), copy));
                self.preset_card = card;
                self.provider.save(&path, false, bytes);
            }
            Err(err) => self.fail(SAVE_FILE_FAILED, err.to_string()),
        }
    }

    /// The preset dialog has something to save: it is open, or about to open.
    pub fn preset_open(&self) -> bool {
        self.preset.is_some()
    }

    /// The preset dialog closed without saving: lets go of what it would have saved.
    pub fn close_preset(&mut self) {
        self.preset = None;
        self.overwrite = None;
    }

    /// `event` finishes the preset save under way: the write of its file (on the web, of a file of its name), or a
    /// failed write that names it, or any failed write while no song save waits.
    pub(super) fn is_preset_event(&self, event: &FileEvent) -> bool {
        (self.preset_saving.as_ref()).is_some_and(|(path, _)| self.finishes(path, event))
    }

    /// The preset save under way finished with `event` (written, or failed): the web keeps its copy, the module
    /// saved takes the preset's name in `song` if it is still there (an instrument's channel too), and `Saved!`
    /// flashes; or the message says why it failed (the write, or the web's copy).
    pub(super) fn preset_done(&mut self, event: FileEvent, song: &mut Song) {
        let Some((path, copy)) = self.preset_saving.take() else {
            return;
        };
        let card = self.preset_card.take();
        if let FileEvent::Failed(_, err) = event {
            return self.fail(SAVE_FILE_FAILED, err);
        }
        // The web's session copy, which its picker lists: a copy that cannot be kept fails the save, as a write does.
        if let Some(bytes) = copy
            && let Err(err) = self.user_copies.put(path.clone(), &bytes)
        {
            return self.fail(SAVE_FILE_FAILED, format!("{}: {err}", path.display()));
        }
        // A chord set takes its file's name.
        if path.extension().is_some_and(|e| e == CHORDS_EXT) {
            let name = path.file_stem().unwrap_or_default().to_string_lossy();
            song.file.keyboard.chord_set_name = name.into_owned();
            self.preset_named = true;
        }
        if let Some((channel, id)) = card
            && let Some(index) = song.channels.iter().position(|c| c.id == channel)
            && let Some(module) = song.channels[index].rack.get_mut(id)
        {
            module.name_preset(&path, self.user.as_deref().unwrap_or(Path::new("")));
            // An instrument's preset is its channel's too, as on a load.
            if instrument(module.type_id).is_some() {
                let preset = module.file.preset_name.clone();
                name_song_channel(song, index, preset);
            }
            self.preset_named = true;
        }
        self.flash = Some(SAVED.into());
    }

    /// Whether a module took the name of the preset saved from it since the last call: rack state, with no undo step
    /// and nothing for the audio.
    pub fn take_preset_named(&mut self) -> bool {
        std::mem::take(&mut self.preset_named)
    }

    /// Keeps `bytes` as the user file `path`, as a preset saved this session on the web is kept.
    #[cfg(test)]
    pub(crate) fn keep_user_file(&mut self, path: &Path, bytes: Vec<u8>) {
        self.user_copies.put(path.to_owned(), &bytes).unwrap();
    }

    /// The desktop's files, from disk, with `user` as the user folder.
    #[cfg(all(test, not(target_arch = "wasm32")))]
    pub(crate) fn on_disk(user: &Path) -> Self {
        Self {
            user: Some(user.to_owned()),
            copies: false,
            ..Self::with_provider(Box::<super::native::NativeFiles>::default())
        }
    }

    /// The user folder, which holds `My Presets`, `My Racks`, `My Drumsets` and `My Chord Sets`. `None`: there is none.
    pub fn user_dir(&self) -> Option<&Path> {
        self.user.as_deref()
    }

    /// The user folder where its files are on disk, which a worker thread can read as the app does
    /// ([`crate::sample_cache::disk_reader`]); `None` on the web and where the session keeps copies in memory.
    pub fn disk_user_dir(&self) -> Option<&Path> {
        self.user_dir().filter(|_| !self.copies)
    }

    /// The library a preset picker lists the files of `exts` from: the user folder on disk on the desktop (the
    /// content folder in it too), this session's copies on the web ([`Self::user_files`]: no folders); and our own
    /// factory beside it ([`content::Union`]).
    pub fn preset_library(&self, exts: &'static [&'static str]) -> Box<dyn SongLibrary + '_> {
        let factory = self.factory;
        #[cfg(not(target_arch = "wasm32"))]
        if !self.copies
            && let Some(user) = &self.user
        {
            let library = crate::ui::main_menu::NativeLibrary::at(user.clone());
            let user = Box::new(library.with_exts(exts));
            return Box::new(content::Union { user, factory });
        }
        let user = Box::new(Copies { files: self, exts });
        Box::new(content::Union { user, factory })
    }

    /// Runs `f` on the library the module, rack, drum kit and chord set pickers list, read and change with their ≡ file
    /// actions: on the desktop the user folder's `top` ([`PRESETS_FOLDER`], [`RACKS_FOLDER`], [`DRUMSETS_FOLDER`] or
    /// [`CHORD_SETS_FOLDER`]; `""` the whole user folder, the Instruments picker's) on disk, listing the `exts` files,
    /// with `Recently Deleted` beside it in the user folder; on the web, and without a user folder, this session's
    /// copies, every file listed. Our own factory is beside it, read-only ([`content::Union`]).
    pub fn with_preset_files<R>(
        &mut self,
        top: &str,
        exts: &[&'static str],
        f: impl FnOnce(&mut dyn SongLibrary) -> R,
    ) -> R {
        let factory = self.factory;
        #[cfg(not(target_arch = "wasm32"))]
        if !self.copies
            && let Some(user) = &self.user
        {
            use crate::ui::main_menu::NativeLibrary;
            let library = match top {
                "" => NativeLibrary::user_folder(user.clone()),
                top => NativeLibrary::at(user.join(top)),
            };
            let user = &mut library.with_exts(exts);
            return f(&mut content::Union { user, factory });
        }
        #[cfg(target_arch = "wasm32")]
        let _ = (top, exts);
        let user = &mut self.user_copies;
        f(&mut content::Union { user, factory })
    }

    /// The files of type `ext` (any case) right in `dir` (see [`Self::presets_dir`], [`Self::racks_dir`]), in path
    /// order: on disk on the desktop, the copies this session kept on the web. Hidden files and links are left out.
    pub fn user_files(&self, dir: &Path, ext: &str) -> Vec<PathBuf> {
        let wanted = |path: &Path| {
            let name = path.file_name().map(|n| n.to_string_lossy());
            !name.is_some_and(|n| n.starts_with('.'))
                && path
                    .extension()
                    .is_some_and(|e| e.eq_ignore_ascii_case(ext))
        };
        let mut files: Vec<PathBuf> = if self.copies {
            let listed = self.user_copies.list(Some(dir)).unwrap_or_default();
            (listed.into_iter().filter(|e| !e.is_folder))
                .map(|e| e.path)
                .collect()
        } else {
            (std::fs::read_dir(dir).into_iter().flatten().flatten())
                .filter(|e| e.file_type().is_ok_and(|t| t.is_file()))
                .map(|e| e.path())
                .collect()
        };
        files.retain(|p| wanted(p));
        files.sort();
        files
    }

    /// The bytes of the file at `path`, one [`Self::user_files`] listed, up to [`MAX_FILE_BYTES`].
    pub fn read_user_file(&self, path: &Path) -> Result<Vec<u8>, String> {
        if self.copies {
            return (self.user_copies.read(path)).map_err(|e| format!("{}: {e}", path.display()));
        }
        let file = std::fs::File::open(path).map_err(|e| e.to_string())?;
        read_limited(file, MAX_FILE_BYTES)
    }
}

/// [`ProjectFiles::preset_library`] on the web: the presets this session saved, read-only.
struct Copies<'a> {
    files: &'a ProjectFiles,
    exts: &'static [&'static str],
}

impl SongLibrary for Copies<'_> {
    fn list(&self, folder: Option<&Path>) -> io::Result<Vec<SongEntry>> {
        let files = folder.map(|dir| {
            let mut files: Vec<PathBuf> = (self.exts.iter())
                .flat_map(|ext| self.files.user_files(dir, ext))
                .collect();
            files.sort();
            files
        });
        let entry = |path: PathBuf| SongEntry {
            name: path
                .file_name()
                .unwrap_or_default()
                .to_string_lossy()
                .into(),
            path,
            is_folder: false,
            recency: 0,
        };
        Ok(files.unwrap_or_default().into_iter().map(entry).collect())
    }

    fn read(&self, path: &Path) -> io::Result<Vec<u8>> {
        self.files.read_user_file(path).map_err(io::Error::other)
    }

    fn store(&mut self, _: &str, _: &[u8]) -> io::Result<()> {
        Err(io::ErrorKind::Unsupported.into())
    }
}

#[cfg(test)]
mod tests;
