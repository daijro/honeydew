//! Opening and saving projects.
//!
//! `Ctrl+O` opens and `Ctrl+S` saves (`Cmd` on macOS). Save is a quick save into the song's folder inside the songs
//! folder, named after the song title; an untitled song (new, or opened from a `.songtmpl`) shows the Save
//! dialog first, then asks where to put the file. The title is always the file name without extension.
//! Every failure ends in the inverted message popup with an `OK` item, never in a panic, and leaves the song as it
//! was.
//!
//! The Save dialog and the messages are overlays (`Overlay::SaveDialog`, `Overlay::Message`): this module asks
//! for them through [`ProjectFiles::take_overlay`], and the app opens them.
//!
//! Replacing a song that has unsaved changes (New, a SONGS row, Import, `Ctrl+O`) asks first, `Save` / `Discard
//! changes` / `Cancel`; its `Save` is a quick save that never asks.
//!
//! The main menu's SONGS tab lists the [`SongLibrary`] kept here and opens its songs through
//! [`ProjectFiles::replace`]. On the desktop the library is the songs folder, where saves start; on the web it
//! is in memory and keeps a copy of every song opened or saved.
//!
//! The `.flm` codec is [`Codec`], a pair of fn pointers: the [`crate::flm`] reader and writer (tests swap in their
//! own). A save writes the song stamped as [`Song::stamp_save`] says, and the song takes the stamp once it is written.
//! Files come from a [`FileProvider`]: native dialogs on desktop, a file input and a download on the web, an
//! in-memory one in tests.
//!
//! The Save dialog's other formats (ZIP, MIDI, the audio renders) and `Quick Render` are [`export`]'s.

use std::io::Read;
use std::path::{Path, PathBuf};

use eframe::egui::{Context, Key, KeyboardShortcut, Modifiers};

use crate::keymap;
use crate::panels::cstm::tuner_inputs_off;
use crate::song::{MetaState, Song};
use crate::ui::main_menu::library::{FORBIDDEN, check_folder_name, check_name};
use crate::ui::main_menu::{MemoryLibrary, SongLibrary};
use crate::ui::views::{Answer, Overlay};
use crate::zip::Entry;

#[cfg(not(target_arch = "wasm32"))]
mod native;
mod presets;
pub use presets::{
    CHORD_SETS_FOLDER, CHORDS_EXT, DRUMSET_EXT, DRUMSETS_FOLDER, FLGSYNTH_EXT, FLMS_EXT,
    PRESET_EXT, PRESETS_FOLDER, PresetKind, PresetSource, RACK_EXT, RACKS_FOLDER, preset_exts,
    preset_folder,
};
#[cfg(target_arch = "wasm32")]
mod web;
#[cfg(target_arch = "wasm32")]
pub(crate) use web::download;
mod soundfonts;
pub use soundfonts::{SF2_EXT, SOUNDFONTS_FOLDER};
pub(crate) use soundfonts::{in_library, in_user_folder};
mod samples;
pub use samples::{RECORDINGS_FOLDER, SAMPLE_EXTS, SAMPLES_FOLDER};
pub(crate) mod content;
#[cfg(not(target_arch = "wasm32"))]
pub(crate) use content::real_folder;
pub use content::{
    BUILT_IN, CONTENT_FOLDER, DRUM_SAMPLES, DRUM_TEMPLATES, RACK_PRESETS, README_FILE, SOUNDFONTS,
    TOP_FOLDERS, built_in,
};
pub(crate) use content::{in_content_folder, resolve_in};
mod picks;
#[cfg(all(test, not(target_arch = "wasm32")))]
mod verify;
pub use picks::{DWP_EXT, SPECTRA_EXT, WAVESHAPES_FOLDER, fits, is_audio, load_pick, read_spectra};
pub(crate) mod fst;
pub use fst::{FST_EXT, FstImport, import_fst};
mod flp;
pub use flp::{LOCATE, LOCATE_TAG};
mod archive;
pub mod export;
pub use archive::{
    ARCHIVE_FAILED, DECOMPRESS_FAILED, IMPORT_FAILED, IMPORT_TAG, NOTHING_KNOWN, UNZIPPED_FOLDER,
};
#[cfg(not(target_arch = "wasm32"))]
pub use samples::make_user_folders;
pub use samples::{INSTRUMENTS_FOLDER, MIDI_FOLDER, MY_FOLDERS, TRACKS_FOLDER};
#[cfg(not(target_arch = "wasm32"))]
mod import;
#[cfg(not(target_arch = "wasm32"))]
pub use import::{
    Counts, IMPORT, NOT_NOW, NOTICE_TAG, import_into, import_notice, import_target_allowed,
};
#[cfg(not(target_arch = "wasm32"))]
mod obb;

/// The format version this app reads and writes. A file saved by a newer version asks before loading.
pub use crate::song::FORMAT_VERSION;
/// The per-file limit: larger files fail with [`READ_FAILED`]. The web's is
/// 128 MiB: the file's `ArrayBuffer`, its copy and the reader's budget (64 MiB + 2× the file) must fit wasm32's memory,
/// or the tab aborts instead of showing the error.
pub const MAX_FILE_BYTES: u64 = if cfg!(target_arch = "wasm32") {
    128 << 20
} else {
    512 << 20
};
/// Longer HEAD version strings are ignored, and versions compare at most this many dotted parts.
const MAX_VERSION_LEN: usize = 64;
const MAX_VERSION_PARTS: usize = 8;

/// The message popup's titles.
pub const OPEN_FAILED: &str = "Failed to open project file";
pub const READ_FAILED: &str = "Failed to read project file";
pub const LOAD_FAILED: &str = "Failed to load project file";
pub const SAVE_FAILED: &str = "Failed to save project";
pub const UNKNOWN_TYPE: &str = "Unknown project file type";
/// The message a load with notices shows; the notices follow as its details.
pub const CORRUPTED: [&str; 2] = [
    "Project file has corrupted data.",
    "The project was loaded successfully, but some elements may be missing.",
];
/// At most this many load notices show; the rest are counted in one more line.
const MAX_NOTICES: usize = 8;
/// At most this many lines of an import's summary ([`STAND_INS`]) show; the rest are counted in one more line.
const MAX_STAND_INS: usize = 6;
/// The message of an import (MIDI, FLP) that loaded our sounds for what the file asked for; a line per channel follows.
pub const STAND_INS: &str = "Some instruments were replaced with the closest built-in sounds:";
/// FLM2 projects are detected and reported, not converted.
const FLM2_MAGIC: &[u8] = b"\x99ELM\r\n\x1A\n";
const FLM2_UNSUPPORTED: &str = "Unsupported old project format";

/// Open: the desktop convention.
const OPEN_KEY: KeyboardShortcut = KeyboardShortcut::new(Modifiers::COMMAND, Key::O);

/// An untitled song's name, made unique as `New Song (n)`.
const NEW_SONG: &str = "New Song";

/// The tags of this module's messages: a notice or error with `OK`, the newer-version question, and the
/// unsaved-changes question before a song is replaced.
pub const MESSAGE_TAG: &str = "project_files";
pub const NEWER_TAG: &str = "project_files_newer";
pub const DISCARD_TAG: &str = "project_files_discard";
/// The unsaved-changes question, and its choices.
pub const UNSAVED: &str = "You have unsaved changes. Please choose an option before proceeding.";
pub const SAVE: &str = "Save";
pub const DISCARD: &str = "Discard changes";
pub const CANCEL: &str = "Cancel";
/// The unsaved-changes question's `Save` failed, and the flash once it is done.
pub const SAVE_FILE_FAILED: &str = "Failed to save file";
pub const SAVED: &str = "Saved!";
/// A preset's typed name is taken: the question, its tag and its choices.
pub const OVERWRITE: &str = "The file already exists";
pub const OVERWRITE_TAG: &str = "project_files_overwrite";
pub const REPLACE_FILES: &str = "Replace files";
pub const KEEP_BOTH: &str = "Keep both";

/// What replaces the song ([`ProjectFiles::replace`]).
#[derive(Clone, Debug, PartialEq)]
pub enum Replace {
    /// A new, empty song.
    New,
    /// A new, empty song with the Song options dialog's choices.
    NewWith(crate::ui::song_options::SongOptions),
    /// The library's song at this path (a SONGS row).
    Open(PathBuf),
    /// A new untitled `New Song (n)` from the song template at this path: one in [`crate::content::factory`]
    /// (SONGS `New`'s menu), else one of the library's.
    Template(PathBuf),
    /// A file the user picks (SONGS `Import`, `Ctrl+O`).
    Pick,
    /// A project file dropped on the window, with its bytes.
    Dropped(PathBuf, Vec<u8>),
}

/// What a file dialog was asked for ([`ProjectFiles`] remembers it until the file arrives).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Pick {
    /// A song, template, archive or MIDI file to open (`Ctrl+O`, SONGS `Import`).
    Song,
    /// An audio file for a picker's user folder (its `Import`, [`ProjectFiles::import_audio`]).
    Sample,
    /// The track menu's `Import MIDI Tracks` on the channel with this id: a MIDI file
    /// ([`ProjectFiles::pick_midi_tracks`]).
    MidiTracks(i32),
}

impl Pick {
    /// The extensions (no dot, any case) the dialog offers and the provider reads: every [`FileKind`] for a song, the
    /// sample ones for a sample, the MIDI ones for `Import MIDI Tracks`.
    pub const fn accept(self) -> &'static [&'static str] {
        match self {
            Pick::Song => &["flm", "songtmpl", "zip", "mid", "midi", "flp"],
            Pick::Sample => &SAMPLE_EXTS,
            Pick::MidiTracks(_) => FileKind::Midi.extensions(),
        }
    }
}

/// A MIDI file picked for `Import MIDI Tracks` ([`ProjectFiles::take_midi_tracks`]), read and scanned.
#[derive(Clone, Debug, PartialEq)]
pub struct MidiTracks {
    /// The id of the channel whose track menu asked.
    pub channel: i32,
    pub bytes: Vec<u8>,
    pub scan: crate::smf::import::Scan,
}

/// What a project file is, by its extension.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum FileKind {
    /// A song, `.flm`.
    Song,
    /// A song template, `.songtmpl`.
    Template,
    /// A project archive, `.zip`.
    Archive,
    /// A Standard MIDI file, `.mid` or `.midi`.
    Midi,
    /// An FL Studio project, `.flp`.
    Flp,
}

impl FileKind {
    /// The extensions of this kind, without the dot, in lower case.
    pub const fn extensions(self) -> &'static [&'static str] {
        match self {
            FileKind::Song => &["flm"],
            FileKind::Template => &["songtmpl"],
            FileKind::Archive => &["zip"],
            FileKind::Midi => &["mid", "midi"],
            FileKind::Flp => &["flp"],
        }
    }
}

/// A song read from a file, and the notices to show once it is open.
pub type Loaded = (Song, Vec<String>);

/// The `.flm` reader and writer. `read` returns the song and the notices to show after loading it ("Missing rack
/// for Channel #N skipped"), the details of [`CORRUPTED`]. `write` gets the song already stamped. The default
/// is [`crate::flm`].
#[derive(Clone, Copy)]
pub struct Codec {
    pub read: fn(&[u8]) -> Result<Loaded, String>,
    pub write: fn(&Song) -> Result<Vec<u8>, String>,
}

impl Default for Codec {
    fn default() -> Self {
        Self {
            read: |b| crate::flm::read_with_notices(b).map_err(|e| e.to_string()),
            write: |s| crate::flm::write(s).map_err(|e| e.to_string()),
        }
    }
}

/// A finished [`FileProvider`] request. A cancelled dialog finishes with no event.
#[derive(Clone, Debug, PartialEq)]
pub enum FileEvent {
    /// The user picked this file (on the web only its name) with these contents.
    Opened(PathBuf, Vec<u8>),
    /// Written to this path (on the web: downloaded under this name, a bare name that is taken to be in the folder
    /// the save asked for).
    Saved(PathBuf),
    /// A request failed: the popup's title ([`OPEN_FAILED`], [`READ_FAILED`] or [`SAVE_FAILED`]) and what went
    /// wrong where.
    Failed(&'static str, String),
    /// `Locate samples…` found these files (name, bytes) in the folder the user picked.
    Located(Vec<(String, Vec<u8>)>),
}

/// Where project files come from and go to. Requests may finish later (the web reads files asynchronously, and the
/// native open picker runs off the UI thread), so their outcome arrives through [`FileProvider::poll`]. A provider
/// checks a file's extension against what was asked for ([`Pick::accept`]) before it reads it, and reads at most
/// [`MAX_FILE_BYTES`].
pub trait FileProvider {
    /// Asks the user for a file to open, one whose extension is in `accept` (no dot, any case). Any other file
    /// finishes with [`UNKNOWN_TYPE`], unread.
    fn open(&mut self, accept: &'static [&'static str]);
    /// Writes `bytes` to `path`, making its folder if there is none. With `ask`, `path` is only a suggestion (a file
    /// name, maybe in a folder) and the user picks where; the dialog filters by the extension of `path`, and the
    /// file keeps it.
    fn save(&mut self, path: &Path, ask: bool, bytes: Vec<u8>);
    /// The next finished request.
    fn poll(&mut self) -> Option<FileEvent>;
    /// Hands over the context, to repaint when a request finishes between frames. Called every frame.
    fn attach(&mut self, _ctx: &Context) {}
    /// Whether an open-file picker is already waiting for the user's answer.
    fn opening(&self) -> bool {
        false
    }
    /// Whether `path` names an existing file or folder. Always false where that cannot be known (the web).
    fn exists(&self, path: &Path) -> bool;
    /// Whether a save keeps its folder. False where a save is a download (the web), which keeps only the file name.
    fn folders(&self) -> bool {
        true
    }
    /// `Locate samples…`: asks for a folder and finishes with [`FileEvent::Located`], the files in it or below named one
    /// of `names` (any case). A provider without folders does nothing.
    fn locate(&mut self, _names: Vec<String>) {}
}

/// Open / save state: the song's file, the Save dialog and the message popup.
pub struct ProjectFiles {
    pub codec: Codec,
    provider: Box<dyn FileProvider>,
    /// The songs SONGS lists.
    pub library: Box<dyn SongLibrary>,
    /// Our own factory library: song templates under [`crate::content::ROOT`] are read from it, and the preset pickers
    /// list it. Tests set a fake.
    pub factory: &'static crate::content::Factory,
    /// Keep a copy of every song opened or saved in `library` (the web's, in memory).
    copies: bool,
    /// The save under way: the path it asked for, its bytes for the library's copy (where it keeps copies), and the
    /// song as written (stamped), whose stamp the song takes once it is done.
    saving: Option<(PathBuf, Option<Vec<u8>>, Song)>,
    /// Counts the saves and copies: SONGS reads its folder again when it changes.
    library_changes: u64,
    /// The song's file. `None`: untitled, and `Ctrl+S` asks.
    path: Option<PathBuf>,
    /// The folder of the last song file opened or saved inside the songs folder. New names are made unique in it,
    /// and the Save dialog starts there.
    dir: Option<PathBuf>,
    /// The Save dialog to open, with the file name it proposes, until [`Self::take_overlay`] hands it over.
    dialog: Option<String>,
    /// A message to show, until [`Self::take_overlay`] hands it over.
    message: Option<Vec<String>>,
    /// A project saved by a newer version, waiting for `Yes` / `Cancel` in the [`NEWER_TAG`] message.
    pending: Option<(PathBuf, Vec<u8>)>,
    /// What replaces the song once the [`DISCARD_TAG`] message is answered `Discard changes` or `Save`.
    discarding: Option<Replace>,
    /// The song-changed flag: set by [`Self::song_changed`],
    /// cleared by [`Self::saved`].
    changed: bool,
    /// What replaces the song once the unsaved-changes question's `Save` is written.
    after_save: Option<Replace>,
    /// The folder open in the SONGS tab (`None`: its root). The app keeps it current.
    pub songs_folder: Option<PathBuf>,
    /// SETTINGS asked for `Restore demo songs`: the app runs it behind its loading screen and clears this
    /// (`app/demo_restore.rs`).
    pub restore_demos: bool,
    /// The songs folder (`My Songs`). On the web, and in tests, the empty path: files have no folder there.
    /// `None`: there is none (the desktop without a home folder).
    root: Option<PathBuf>,
    /// A flash to show (it may have several lines), until [`Self::take_flash`] hands it over.
    flash: Option<String>,
    /// What the open dialog was asked for, until its file arrives.
    pick: Option<Pick>,
    /// The user folder, which holds `My Presets` and `My Racks`: the songs folder's on the
    /// desktop, the empty path on the web and in tests. `None`: there is none.
    user: Option<PathBuf>,
    /// What the preset dialog saves ([`Self::ask_preset`]), and the id of the channel it comes from.
    preset: Option<(PresetSource, i32)>,
    /// The preset dialog to open, with the file name it proposes, until [`Self::take_overlay`] hands it over.
    preset_dialog: Option<String>,
    /// The preset's typed name that is taken, while the [`OVERWRITE_TAG`] question waits.
    overwrite: Option<String>,
    /// The preset save under way: its path, and its bytes for the copy kept where `copies`.
    preset_saving: Option<(PathBuf, Option<Vec<u8>>)>,
    /// The module a module preset save under way names once written: its channel's id and its instance id.
    preset_card: Option<(i32, i32)>,
    /// A module took the name of the preset saved from it ([`Self::take_preset_named`]).
    preset_named: bool,
    /// The user files kept this session where `copies` (the web): the presets saved, at their paths below the user
    /// folder (the empty path), what [`Self::user_files`] lists there and the preset pickers' file actions change.
    user_copies: MemoryLibrary,
    /// The export waiting for the [`OVERWRITE_TAG`] question.
    export_asked: Option<export::Asked>,
    /// The export that went ahead, until [`Self::take_export`] hands it over.
    exported: Option<export::Export>,
    /// The export file being written.
    export_saving: Option<export::Export>,
    /// The original content files the ZIP export under way left out, for its flash.
    export_left_out: usize,
    /// A split export's stem files being written.
    stem_writes: export::StemWrites,
    /// An archive's files, while `Import %d files?` waits for its answer ([`IMPORT_TAG`]).
    importing: Option<Vec<Entry>>,
    /// Where the audio file an `Import` asked for goes ([`Self::import_audio`]).
    import_dir: Option<PathBuf>,
    /// The archive import running on a worker ([`Self::import_archives`]).
    #[cfg(not(target_arch = "wasm32"))]
    import: Option<std::sync::mpsc::Receiver<import::Imported>>,
    /// The archives whose import failed this session: tried again at the next start, or once replaced.
    #[cfg(not(target_arch = "wasm32"))]
    import_failed: std::collections::HashSet<import::Failed>,
    /// The first-import notice this session.
    #[cfg(not(target_arch = "wasm32"))]
    import_notice: Option<import::Notice>,
    /// The notice's `Import` was chosen, now or before: archives import without asking. The app keeps it in its
    /// settings (`importNoticeAccepted`) and hands it over each frame.
    pub import_accepted: bool,
    /// A MIDI export runs each channel's notes through its note effects at this engine sample rate (`Apply note
    /// effects`, [`crate::smf::smf_with`]); `None`: the raw notes. The Save dialog sets it before each export.
    pub midi_note_effects: Option<f32>,
    /// The MIDI file `Import MIDI Tracks` picked, until [`Self::take_midi_tracks`] hands it over.
    midi_tracks: Option<MidiTracks>,
    /// The last [`RECENT_STATE`] write, on its worker: the next waits for it, so the last load's lands last.
    #[cfg(not(target_arch = "wasm32"))]
    autosave: Option<std::thread::JoinHandle<()>>,
    /// The samples the last FLP open could not find, for `Locate samples…`.
    missing: Vec<flp::Missing>,
    /// The FLP summary with `Locate samples…` waits for its answer ([`LOCATE_TAG`]).
    locate_asked: bool,
    /// Samples were located into the song ([`Self::take_relinked`]).
    relinked: bool,
    /// What the FLP export under way could not write, shown once it is saved.
    export_notices: Vec<String>,
    /// A file dropped on the window, until no dialog or message is open.
    drop_waiting: Option<eframe::egui::DroppedFile>,
}

/// With the platform's provider and library. On the desktop the library is the songs folder, and saving starts
/// there.
impl Default for ProjectFiles {
    fn default() -> Self {
        #[cfg_attr(target_arch = "wasm32", allow(unused_mut))]
        let mut files = Self::with_provider(platform_provider());
        #[cfg(not(target_arch = "wasm32"))]
        {
            let library = crate::ui::main_menu::NativeLibrary::new();
            files.dir = library.root().map(Path::to_owned);
            files.root.clone_from(&files.dir);
            files.user = files
                .root
                .as_deref()
                .and_then(Path::parent)
                .map(Path::to_owned);
            files.library = Box::new(library);
            files.copies = false;
            // At start (not in tests, which must not touch the user's folder): the user folder's own folders, the
            // Original Content folder, and GMSynth's waveshapes from it before any GMSynth is built (`content`,
            // `picks`).
            #[cfg(not(test))]
            {
                if let Some(Err(err)) = files.user.as_deref().map(make_user_folders) {
                    eprintln!("user folders: {err}");
                }
                if let Some(Err(err)) = files.make_content_folder() {
                    eprintln!("{}: {err}", content::CONTENT_FOLDER);
                }
                files.install_original_waveshapes();
                files.install_original_keymap();
                files.import_archives();
            }
        }
        files
    }
}

impl ProjectFiles {
    /// With `provider`, and an in-memory library that keeps a copy of every song opened or saved (as on the web).
    pub fn with_provider(provider: Box<dyn FileProvider>) -> Self {
        Self {
            codec: Codec::default(),
            provider,
            library: Box::<MemoryLibrary>::default(),
            factory: crate::content::factory(),
            copies: true,
            saving: None,
            library_changes: 0,
            path: None,
            dir: None,
            dialog: None,
            message: None,
            pending: None,
            discarding: None,
            changed: false,
            after_save: None,
            songs_folder: None,
            restore_demos: false,
            root: Some(PathBuf::new()),
            flash: None,
            pick: None,
            user: Some(PathBuf::new()),
            preset: None,
            preset_dialog: None,
            overwrite: None,
            preset_saving: None,
            preset_card: None,
            preset_named: false,
            user_copies: MemoryLibrary::default(),
            export_asked: None,
            exported: None,
            export_saving: None,
            export_left_out: 0,
            stem_writes: export::StemWrites::default(),
            importing: None,
            import_dir: None,
            #[cfg(not(target_arch = "wasm32"))]
            import: None,
            #[cfg(not(target_arch = "wasm32"))]
            import_failed: std::collections::HashSet::new(),
            #[cfg(not(target_arch = "wasm32"))]
            import_notice: None,
            import_accepted: false,
            midi_note_effects: None,
            midi_tracks: None,
            #[cfg(not(target_arch = "wasm32"))]
            autosave: None,
            missing: Vec::new(),
            locate_asked: false,
            relinked: false,
            export_notices: Vec::new(),
            drop_waiting: None,
        }
    }

    /// Asks for a project to open; [`ProjectFiles::update`] returns it once it is read.
    pub fn open(&mut self) {
        self.open_pick(Pick::Song);
    }

    /// The track menu's `Import MIDI Tracks` on the channel with id `channel`:
    /// asks for a `.mid` or `.midi` file. Once it is read, [`Self::take_midi_tracks`] hands it over scanned; a file
    /// that does not scan shows `Could not load MIDI file. It may be corrupted.` with the detail.
    pub fn pick_midi_tracks(&mut self, channel: i32) {
        self.open_pick(Pick::MidiTracks(channel));
    }

    /// The MIDI file [`Self::pick_midi_tracks`] asked for, once, when it is here.
    pub fn take_midi_tracks(&mut self) -> Option<MidiTracks> {
        self.midi_tracks.take()
    }

    /// The file `Import MIDI Tracks` picked for the channel with id `channel`: scanned for [`Self::take_midi_tracks`],
    /// or the MIDI load error.
    fn midi_picked(&mut self, channel: i32, bytes: Vec<u8>) {
        use crate::smf::import::{LOAD_FAILED, scan};
        match scan(&bytes) {
            Ok(scan) => {
                self.midi_tracks = Some(MidiTracks {
                    channel,
                    bytes,
                    scan,
                })
            }
            Err(detail) => self.fail(LOAD_FAILED, detail),
        }
    }

    fn open_pick(&mut self, pick: Pick) {
        if self.provider.opening() {
            return;
        }
        self.pick = Some(pick);
        self.provider.open(pick.accept());
    }

    /// The song has unsaved changes: the song-changed flag is set ([`Self::song_changed`]). The prompt checks only the
    /// flag, whatever the song holds.
    pub fn modified(&self) -> bool {
        self.changed
    }

    /// Sets the song-changed flag: an undo step saved, an undo or
    /// redo, a param control's value change, a preset pick, a drum strip's new `Stretch` ratio, a recorded take. A save,
    /// a new song and a load clear it.
    pub fn song_changed(&mut self) {
        self.changed = true;
    }

    /// Replaces the song with `what`; when it has unsaved changes ([`Self::modified`]), only once the [`DISCARD_TAG`]
    /// question is answered `Discard changes`, or `Save` and the song is saved (through [`Self::update`]). Returns the
    /// new song when it is ready now, for the caller to replace the current one with; a picked file, a question or an
    /// error comes through [`Self::update`] and [`Self::take_overlay`] after the frame.
    pub fn replace(&mut self, what: Replace) -> Option<Song> {
        if self.modified() {
            self.message = Some(vec![UNSAVED.into()]);
            self.discarding = Some(what);
            return None;
        }
        self.replace_now(what)
    }

    fn replace_now(&mut self, what: Replace) -> Option<Song> {
        match what {
            Replace::New => Some(self.new_song(None)),
            Replace::NewWith(options) => Some(self.new_song(Some(options))),
            Replace::Open(path) | Replace::Template(path) => self.open_song(path),
            Replace::Pick => {
                self.open();
                None
            }
            Replace::Dropped(path, bytes) => self.opened(path, bytes),
        }
    }

    /// Autosave: the whole project just loaded is written to [`RECENT_STATE`] in the user
    /// root, encoded and written whole ([`write_atomic`]) on a worker, as a big song takes a while. Only on disk: where
    /// The session keeps copies (the web) there is none. Nothing reads it back. A failure is logged.
    fn autosave(&mut self, song: &Song) {
        #[cfg(not(target_arch = "wasm32"))]
        {
            // Tests write into temp folders only, never the user's.
            let allowed = |u: &&Path| {
                u.is_absolute() && (cfg!(not(test)) || u.starts_with(std::env::temp_dir()))
            };
            let Some(user) = self
                .user
                .as_deref()
                .filter(allowed)
                .filter(|_| !self.copies)
            else {
                return;
            };
            let (path, song, write) = (user.join(RECENT_STATE), song.clone(), self.codec.write);
            let previous = self.autosave.take();
            self.autosave = Some(std::thread::spawn(move || {
                if let Some(previous) = previous {
                    let _ = previous.join();
                }
                let written =
                    write(&song).and_then(|b| write_atomic(&path, &b).map_err(|e| e.to_string()));
                if let Err(err) = written {
                    eprintln!("{} not written: {err}", path.display());
                }
            }));
        }
        #[cfg(target_arch = "wasm32")]
        let _ = song;
    }

    /// Waits for the [`RECENT_STATE`] write under way.
    #[cfg(all(test, not(target_arch = "wasm32")))]
    pub(crate) fn join_autosave(&mut self) {
        if let Some(worker) = self.autosave.take() {
            worker.join().unwrap();
        }
    }

    /// The song is saved, opened or new: the song-changed flag is clear. Also for a caller that knows it is (tests and
    /// render harnesses that set a song up).
    pub(crate) fn saved(&mut self) {
        self.changed = false;
    }

    /// Opens the library's song at `path` (a SONGS row) as a picked file opens, or a song template, which can be in
    /// our factory ([`Replace::Template`]); what it has to show (an error, the newer-version question) shows after
    /// the frame, and a newer project loads through [`Self::update`] once answered.
    fn open_song(&mut self, path: PathBuf) -> Option<Song> {
        let from: &dyn SongLibrary = match crate::content::is_factory(&path) {
            true => self.factory,
            false => &*self.library,
        };
        match from.read(&path) {
            Ok(bytes) => self.opened(path, bytes),
            Err(err) => {
                self.fail(READ_FAILED, format!("{}: {err}", path.display()));
                None
            }
        }
    }

    /// A new, empty song, created now, with `options` (TDIV and tempo) if given: untitled, so that the next
    /// save asks for its name.
    fn new_song(&mut self, options: Option<crate::ui::song_options::SongOptions>) -> Song {
        self.path = None;
        let mut song = Song::default();
        if let Some(options) = options {
            options.apply(&mut song);
        }
        song.file.meta.created = created_now();
        self.saved();
        song
    }

    /// A file action moved `from` (a file or a folder) to `to`, or deleted it (`None`): SONGS', or a preset picker's,
    /// which forwards its moves the same way. The song's file and folder follow, so a save never writes where another
    /// file may now be. A song whose file was renamed takes the new name as its title (unless a title was typed since),
    /// which is no unsaved change; one whose file was deleted has none, so its next save asks, and new names then go
    /// into the songs folder.
    pub fn path_moved(&mut self, song: &mut Song, from: &Path, to: Option<&Path>) {
        // `None`: not `from` or inside it. `Some(None)`: deleted.
        let follow = |path: &Path| {
            let rest = path.strip_prefix(from).ok()?;
            Some(to.map(|to| match rest.as_os_str().is_empty() {
                true => to.to_owned(),
                false => to.join(rest),
            }))
        };
        if let Some(old) = self.path.clone()
            && let Some(new) = follow(&old)
        {
            if let Some(new) = &new {
                let (was, now) = (stem(&old), stem(new));
                if song.title == was {
                    song.title = now;
                }
            }
            self.path = new;
        }
        if let Some(dir) = self.dir.as_deref()
            && let Some(new) = follow(dir)
        {
            self.dir = new.or_else(|| self.root.clone());
        }
    }

    /// The flash to show (`Saved!`), once.
    pub fn take_flash(&mut self) -> Option<String> {
        self.flash.take()
    }

    /// Asks for a flash from outside the files: the Modules browser's `Cannot paste module`. `\n` starts a new
    /// line (`Render complete\nSaved to: <path>`).
    pub fn flash(&mut self, text: impl Into<String>) {
        self.flash = Some(text.into());
    }

    /// Changes with every save and every copy kept in the library.
    pub fn library_changes(&self) -> u64 {
        self.library_changes
    }

    /// `Restore demo songs`' copy: our demo songs over the library's ([`crate::content::restore_demo_songs`]), a
    /// change SONGS reads its folder again for.
    pub fn restore_demo_songs(&mut self) -> std::io::Result<()> {
        crate::content::restore_demo_songs(&mut *self.library, true)?;
        self.library_changes += 1;
        Ok(())
    }

    /// Keeps a copy of a song opened or saved at `path` in the library, where it keeps copies: in its own folder
    /// when that is one of the library's (a song SONGS opened or moved), else in the root.
    fn keep(&mut self, path: &Path, bytes: &[u8]) {
        let name = path.file_name().and_then(|n| n.to_str());
        let folder = path.parent().filter(|f| self.library.exists(f));
        if self.copies
            && let Some(name) = name
            && match folder {
                Some(folder) => self.library.store_in(folder, name, bytes),
                None => self.library.store(name, bytes),
            }
            .is_ok()
        {
            self.library_changes += 1;
        }
    }

    /// Quick save (`Ctrl+S`, SONGS and PROJECT `Save`): [`Self::quick_save`], or the Save dialog when the song is
    /// untitled.
    pub fn save(&mut self, song: &Song) {
        match self.path {
            Some(_) => self.quick_save(song),
            None => self.save_as(song),
        }
    }

    /// `Save New`: the Save dialog, with the title (or `New Song (n)`) as the name, whether the song has a file or
    /// not.
    pub fn save_as(&mut self, song: &Song) {
        self.dialog = Some(self.proposed_name(song));
    }

    /// The file name a Save dialog proposes for `song`: its title, or `New Song (n)` for an untitled one.
    pub fn proposed_name(&self, song: &Song) -> String {
        match song.title.trim() {
            "" => self.new_song_name(),
            title => title.to_owned(),
        }
    }

    /// `New Song`, or `New Song (n)` with the smallest n ≥ 1 that is free in the song folder.
    fn new_song_name(&self) -> String {
        self.free_name(self.dir.as_deref(), NEW_SONG, "flm", None)
    }

    /// `base`, or `base (n)` with the smallest n ≥ 1, whose `.ext` in `dir` is no file other than `own`.
    fn free_name(&self, dir: Option<&Path>, base: &str, ext: &str, own: Option<&Path>) -> String {
        self.free_name_from(1, dir, base, ext, own)
    }

    /// [`Self::free_name`] with the smallest n ≥ `first`.
    fn free_name_from(
        &self,
        first: usize,
        dir: Option<&Path>,
        base: &str,
        ext: &str,
        own: Option<&Path>,
    ) -> String {
        let taken = |name: &str| {
            dir.map(|dir| dir.join(format!("{name}.{ext}")))
                .is_some_and(|file| Some(file.as_path()) != own && self.taken(&file))
        };
        std::iter::once(base.to_owned())
            .chain((first..).map(|n| format!("{base} ({n})")))
            .find(|name| !taken(name))
            .expect("an unbounded range has a free number")
    }

    /// A file is at `file`: on disk, or (on the web) a song or preset this session keeps there. The web's provider
    /// cannot tell: the downloads folder cannot be listed.
    fn taken(&self, file: &Path) -> bool {
        self.provider.exists(file)
            || self.copies && (self.library.exists(file) || self.user_copies.exists(file))
    }

    fn write(&mut self, song: &Song, path: &Path, ask: bool) {
        let stamped = stamped(song, stem(path));
        match (self.codec.write)(&stamped) {
            // What could never be opened again is not saved (on the web, a download past its 128 MiB).
            Ok(bytes) if bytes.len() as u64 > MAX_FILE_BYTES => self.save_failed(too_large()),
            Ok(bytes) => {
                let copy = self.copies.then(|| bytes.clone());
                self.saving = Some((path.to_owned(), copy, stamped));
                self.provider.save(path, ask, bytes);
            }
            Err(err) => self.save_failed(err),
        }
    }

    /// A save failed: the message, with [`SAVE_FILE_FAILED`] and the action dropped when it was the
    /// unsaved-changes question's `Save`.
    fn save_failed(&mut self, detail: String) {
        let title = match self.after_save.take() {
            Some(_) => SAVE_FILE_FAILED,
            None => SAVE_FAILED,
        };
        self.fail(title, detail);
    }

    /// The unsaved-changes question's `Save`: a [`Self::quick_save`] that never asks, then
    /// `what` once it is written.
    fn save_then(&mut self, song: &Song, what: Replace) {
        self.after_save = Some(what);
        self.quick_save(song);
    }

    /// Every quick save writes here, never asking: into the song's
    /// folder if it is inside the songs folder, else the one open in SONGS (if inside), else the songs folder, so a
    /// file opened from elsewhere is never written over. The name is the title (or, when that is no song file name,
    /// the file's or `New Song`), made free: a quick save never writes over another song.
    fn quick_save(&mut self, song: &Song) {
        let own = self.path.clone();
        let folder = own
            .as_deref()
            .and_then(Path::parent)
            .filter(|f| self.inside(f))
            .or(self.songs_folder.as_deref().filter(|f| self.inside(f)))
            .or(self.root.as_deref());
        let Some(folder) = folder.map(Path::to_owned) else {
            return self.save_failed("There is no songs folder.".into());
        };
        let title = song.title.trim();
        let base = if !title.is_empty() && check_name(&format!("{title}.flm")).is_ok() {
            title.to_owned()
        } else {
            own.as_deref().map_or_else(|| NEW_SONG.to_owned(), stem)
        };
        let name = self.free_name(Some(&folder), &base, "flm", own.as_deref());
        self.write(song, &folder.join(format!("{name}.flm")), false);
    }

    /// A picked file's path. On the web, where it is kept in the library under its name ([`Self::keep`]), a name the
    /// session already keeps takes a free `name (n)`, so an import never replaces a song.
    fn imported(&self, path: PathBuf) -> PathBuf {
        let (Some(ext), Some(dir)) = (path.extension(), path.parent()) else {
            return path;
        };
        if !self.copies || !self.library.exists(&path) {
            return path;
        }
        let ext = ext.to_string_lossy();
        dir.join(format!(
            "{}.{ext}",
            self.free_name(Some(dir), &stem(&path), &ext, None)
        ))
    }

    /// `path` is inside the songs folder, and not in the bin (the web's root, the empty path, holds it); anything is
    /// when there is none.
    fn inside(&self, path: &Path) -> bool {
        self.root
            .as_deref()
            .is_none_or(|root| path.starts_with(root))
            && !(self.library.trash()).is_some_and(|trash| path.starts_with(trash))
    }

    /// The song's file is now `path`. Its folder is where new names go only when it is inside the songs folder.
    fn set_path(&mut self, path: PathBuf) {
        if let Some(dir) = path.parent().filter(|d| self.inside(d)) {
            self.dir = Some(dir.to_owned());
        }
        self.path = Some(path);
    }

    fn fail(&mut self, title: &str, detail: String) {
        self.message = Some(vec![title.into(), detail]);
    }

    /// `event` finishes the write of `path`: its `Saved` (on the web, of a file of its name), or a failed write that
    /// names it, or any failed write while no song save waits.
    fn finishes(&self, path: &Path, event: &FileEvent) -> bool {
        let name = path.file_name().map(Path::new);
        let ours = |p: &Path| p == path || (p.parent() == Some(Path::new("")) && Some(p) == name);
        match event {
            FileEvent::Saved(p) => ours(p),
            FileEvent::Failed(SAVE_FAILED, detail) => {
                self.saving.is_none()
                    || detail
                        .split_once(": ")
                        .is_some_and(|(p, _)| ours(Path::new(p)))
            }
            _ => false,
        }
    }

    /// Handles the finished requests. Returns the song opened, if any; a save renames `song` after its file.
    fn poll(&mut self, song: &mut Song) -> Option<Song> {
        let mut opened = None;
        while let Some(event) = self.provider.poll() {
            match event {
                event if self.is_preset_event(&event) => self.preset_done(event, song),
                event if self.is_export_event(&event) => self.export_done(event),
                FileEvent::Opened(path, bytes) => {
                    opened = match self.pick.take() {
                        Some(Pick::Sample) => {
                            self.audio_imported(&path, bytes);
                            None
                        }
                        Some(Pick::MidiTracks(channel)) => {
                            self.midi_picked(channel, bytes);
                            None
                        }
                        Some(Pick::Song) | None => {
                            let path = self.imported(path);
                            self.opened(path, bytes)
                        }
                    }
                    .or(opened);
                }
                FileEvent::Saved(mut path) => {
                    if let Some((asked, copy, stamped)) = self.saving.take() {
                        if let Some(bytes) = copy {
                            // The web reports only the name it downloaded under: in the library, the song is where
                            // the save asked (its own folder).
                            if path.parent() == Some(Path::new("")) {
                                path = asked.with_file_name(path);
                            }
                            self.keep(&path, &bytes);
                        }
                        // The stamp written (dates, version, GUIDs) and the Tuners as saved; the rest is the song as
                        // it was then.
                        song.keep_unedited(&stamped);
                        tuner_inputs_off(song);
                    }
                    // The save may have gone into the songs folder.
                    self.library_changes += 1;
                    song.title = stem(&path);
                    // ponytail: an edit made between the write and this event counts as saved; the native and
                    // web providers answer within the frame that writes.
                    self.saved();
                    self.set_path(path);
                    if let Some(what) = self.after_save.take() {
                        self.flash = Some(SAVED.into());
                        opened = self.replace_now(what).or(opened);
                    }
                }
                FileEvent::Failed(SAVE_FAILED, err) => {
                    self.saving = None;
                    self.save_failed(err);
                }
                FileEvent::Failed(title, err) if matches!(self.pick, Some(Pick::Sample)) => {
                    self.import_failed(title, err);
                }
                FileEvent::Failed(title, err) => self.fail(title, err),
                FileEvent::Located(files) => self.located(song, files),
            }
        }
        opened
    }

    /// Checks the file type and version, then loads it; a newer version asks first. `.mid` opens as a new song
    /// ([`Self::open_midi`]). Of a `.zip`, its one song is checked and loaded; an archive without exactly one asks to
    /// import its files instead (`archive`).
    fn opened(&mut self, path: PathBuf, bytes: Vec<u8>) -> Option<Song> {
        // The providers check first; this keeps a provider that does not from loading anything.
        let mut entries = match file_kind(&path) {
            Some(FileKind::Song | FileKind::Template) => None,
            Some(FileKind::Midi) => return self.open_midi(&path, &bytes),
            Some(FileKind::Flp) => return self.open_flp(&path, &bytes, None),
            Some(FileKind::Archive) => Some(self.unzip(&bytes)?),
            None => {
                self.message = Some(vec![UNKNOWN_TYPE.into()]);
                return None;
            }
        };
        // An archive of an FL Studio project and its samples (FL's zipped loop package, our FLP export).
        if let Some(all) = entries
            .as_deref()
            .filter(|all| archive::the_song(all).is_none())
        {
            let flp = |e: &&Entry| file_kind(Path::new(&e.path)) == Some(FileKind::Flp);
            let mut projects = all.iter().filter(flp);
            if let (Some(project), None) = (projects.next(), projects.next()) {
                return self.open_flp(&path, &project.data, Some(all));
            }
        }
        if let Some(entries) = entries.take_if(|all| archive::the_song(all).is_none()) {
            self.ask_import(entries);
            return None;
        }
        let song = match &entries {
            Some(all) => &all[archive::the_song(all)?].data[..],
            None => &bytes[..],
        };
        if song.starts_with(FLM2_MAGIC) {
            self.fail(LOAD_FAILED, FLM2_UNSUPPORTED.into());
            return None;
        }
        if let Some(version) = saved_version(song).filter(|v| newer(v, FORMAT_VERSION)) {
            self.message = Some(vec![
                // The file's version is its format's; the app names its own version.
                format!(
                    "This project was saved in a newer format ({version}) than {} {} can read,",
                    crate::APP_NAME,
                    crate::ui::main_menu::version()
                ),
                "and will probably not load correctly.".into(),
                "Do you want to continue loading this project anyway?".into(),
            ]);
            self.pending = Some((path, bytes));
            return None;
        }
        self.load_from(path, song, entries.as_deref())
    }

    /// Loads the file `bytes` of `path` once [`Self::opened`] checked it: a song or template, or an archive whose one
    /// song is read again (the newer-version question waits with the archive's bytes).
    fn load(&mut self, path: PathBuf, bytes: &[u8]) -> Option<Song> {
        if file_kind(&path) != Some(FileKind::Archive) {
            return self.load_from(path, bytes, None);
        }
        let entries = self.unzip(bytes)?;
        let song = &entries[archive::the_song(&entries)?].data;
        self.load_from(path, song, Some(&entries))
    }

    /// `.flm` keeps its path (a quick save of one outside the songs folder goes into it, [`Self::quick_save`]); a
    /// `.songtmpl` opens as an untitled `New Song (n)`. An archive's song (`bytes`, one of `entries`,
    /// The archive's files) opens once they are extracted, its paths leading to them, with the `.zip` as its path.
    /// The title is the file name.
    fn load_from(
        &mut self,
        path: PathBuf,
        bytes: &[u8],
        entries: Option<&[Entry]>,
    ) -> Option<Song> {
        let template = file_kind(&path) == Some(FileKind::Template);
        // The app needs the master channel (a song without channels fails to load); this is the trust boundary.
        let read =
            (self.codec.read)(bytes).and_then(|(song, notices)| match song.channels.is_empty() {
                true => Err("The song has no channels.".to_owned()),
                false => Ok((song, notices)),
            });
        match read {
            Ok((mut song, mut notices)) => {
                // An archive's files are extracted, and its song's paths lead there: nothing opens without
                // them. Before the DW programs load, which read them.
                if let Some(entries) = entries {
                    match self.extract(&path, entries) {
                        Ok((dir, binned)) => {
                            if binned {
                                self.flash = Some(archive::earlier_in_bin(&dir));
                            }
                            self.relink_to(&mut song, &dir);
                        }
                        Err(err) => {
                            self.fail(DECOMPRESS_FAILED, err);
                            return None;
                        }
                    }
                }
                // Each DW Sampler's program loads now, with its files.
                crate::panels::cstm::load_dw_programs(
                    &mut song,
                    &crate::sample_cache::reader(self),
                );
                if template {
                    // A new song, created now and never saved, with GUIDs no other song from the
                    // template has.
                    song.title = self.new_song_name();
                    song.new_guids();
                    song.file.meta.created = created_now();
                    song.file.meta.modified = MetaState::default().modified;
                    self.path = None;
                } else {
                    // The library keeps songs; an archive's song is no file of its own.
                    if entries.is_none() {
                        self.keep(&path, bytes);
                    }
                    song.title = stem(&path);
                    self.set_path(path);
                }
                cap_notices(&mut notices);
                if let Some(first) = notices.first_mut() {
                    *first = format!("Details: {first}");
                }
                self.message = (!notices.is_empty()).then(|| {
                    CORRUPTED
                        .map(String::from)
                        .into_iter()
                        .chain(notices)
                        .collect()
                });
                self.saved();
                self.autosave(&song);
                Some(song)
            }
            Err(err) => {
                self.fail(LOAD_FAILED, err);
                None
            }
        }
    }

    /// A file dropped on the window: a project file ([`file_kind`]) is read (the web hands its bytes, the desktop its
    /// path) and replaces the song as a picked one; any other file is left alone.
    fn dropped(&mut self, file: eframe::egui::DroppedFile) -> Option<Song> {
        let path = file
            .path
            .clone()
            .unwrap_or_else(|| PathBuf::from(&file.name));
        file_kind(&path)?;
        let bytes = match (file.bytes, &file.path) {
            (Some(bytes), _) => bytes.to_vec(),
            #[cfg(not(target_arch = "wasm32"))]
            (None, Some(at)) => match std::fs::File::open(at).map_err(|e| e.to_string()) {
                Ok(f) => match read_file_limited(f, MAX_FILE_BYTES) {
                    Ok(bytes) => bytes,
                    Err(err) => {
                        self.fail(READ_FAILED, format!("{}: {err}", at.display()));
                        return None;
                    }
                },
                Err(err) => {
                    self.fail(READ_FAILED, format!("{}: {err}", at.display()));
                    return None;
                }
            },
            _ => return None,
        };
        let path = self.imported(path);
        self.replace(Replace::Dropped(path, bytes))
    }

    /// `.mid` opens as a new song: the master channel, then every MIDI channel
    /// imported with Split on, Import automation on and Only supported off, at the file's tempo. It is untitled, so its
    /// first save asks, and titled after the file. A file that does not read changes nothing.
    fn open_midi(&mut self, path: &Path, bytes: &[u8]) -> Option<Song> {
        use crate::smf::import::{ImportOpts, LOAD_FAILED, import};
        let mut song = Song::default();
        let as_song = &ImportOpts::OPEN_AS_SONG;
        let imported = match import(&mut song, 0, bytes, as_song, true, self.factory) {
            Ok(imported) => imported,
            Err(err) => {
                self.fail(LOAD_FAILED, err);
                return None;
            }
        };
        crate::panels::cstm::load_dw_programs(&mut song, &crate::sample_cache::reader(self));
        self.message = stand_ins(imported.notices);
        song.title = stem(path);
        song.file.meta.created = created_now();
        self.path = None;
        self.saved();
        self.autosave(&song);
        Some(song)
    }

    /// `SAVE` in the Save dialog with the file name `name`: checks it, then asks where to save `<name>.flm`,
    /// starting in the song folder. Returns whether the dialog closes: a bad name keeps it open (behind the
    /// message), an empty one does nothing.
    pub fn dialog_save(&mut self, song: &Song, name: &str) -> bool {
        let Some(name) = self.typed_name(name, "flm") else {
            return false;
        };
        // Without a system save dialog (web and Android), keep other songs under the name and take a free name.
        let name = match self.copies || cfg!(target_os = "android") {
            true => {
                let dir = self.dir.as_deref().unwrap_or(Path::new(""));
                self.free_name(Some(dir), &name, "flm", self.path.as_deref())
            }
            false => name,
        };
        let file = format!("{name}.flm");
        let path = self
            .dir
            .as_ref()
            .map_or_else(|| file.clone().into(), |d| d.join(&file));
        self.write(song, &path, true);
        true
    }

    /// The name typed in a save dialog, without a `.ext` ending (any case), if it makes a file name. `None` for an empty
    /// one, and for a bad one, whose message then shows.
    pub(crate) fn typed_name(&mut self, name: &str, ext: &str) -> Option<String> {
        let name = strip_ext(name.trim(), ext).to_owned();
        if name.is_empty() {
            return None;
        }
        if let Some(c) = name.chars().find(|&c| FORBIDDEN.contains(c)) {
            self.message = Some(vec![format!("Symbol '{c}' is not allowed in a file name.")]);
            return None;
        }
        if check_folder_name(&format!("{name}.{ext}")).is_err() {
            self.message = Some(vec![format!("'{name}' is not allowed as a file name.")]);
            return None;
        }
        Some(name)
    }

    /// The overlay to open, once: the Save dialog with the name it proposes, then the preset dialog, then a message ([`MESSAGE_TAG`] with
    /// `OK`; [`NEWER_TAG`] with `Yes` / `Cancel` while a newer project waits; [`DISCARD_TAG`] with `Save` /
    /// `Discard changes` / `Cancel` while a replacement waits; [`IMPORT_TAG`] with `Yes` / `No` while an archive's files
    /// wait; the first-import notice with `Import` / `Not now`), which so opens on top of the dialog.
    pub fn take_overlay(&mut self) -> Option<Overlay> {
        if let Some(name) = self.dialog.take() {
            return Some(Overlay::SaveDialog { name });
        }
        if let Some(name) = self.preset_dialog.take()
            && let Some((source, _)) = &self.preset
        {
            let kind = source.kind();
            return Some(Overlay::PresetDialog { name, kind });
        }
        let lines = self.message.take()?;
        let (tag, confirm, extra) = match self.waiting_for() {
            Some(NEWER_TAG) => (NEWER_TAG, Some(("Yes", CANCEL)), None),
            Some(IMPORT_TAG) => (IMPORT_TAG, Some(("Yes", "No")), None),
            Some(LOCATE_TAG) => (LOCATE_TAG, Some((LOCATE, "OK")), None),
            #[cfg(not(target_arch = "wasm32"))]
            Some(NOTICE_TAG) => (NOTICE_TAG, Some((IMPORT, NOT_NOW)), None),
            // Top to bottom: `Replace files`, `Keep both`, `Cancel`.
            Some(OVERWRITE_TAG) => (
                OVERWRITE_TAG,
                Some((KEEP_BOTH, CANCEL)),
                Some(REPLACE_FILES),
            ),
            Some(tag) => (tag, Some((DISCARD, CANCEL)), Some(SAVE)),
            None => (MESSAGE_TAG, None, None),
        };
        Some(Overlay::Message {
            tag,
            lines,
            confirm: confirm.map(|(yes, no)| (yes.into(), no.into())),
            extra: extra.map(Into::into),
        })
    }

    /// The tag of the shown question whose answer [`Self::update`] waits for: a project from a newer version
    /// ([`NEWER_TAG`]), a song to replace ([`DISCARD_TAG`]), a preset to write over ([`OVERWRITE_TAG`]) or an
    /// archive's files to import ([`IMPORT_TAG`]).
    pub fn waiting(&self) -> Option<&'static str> {
        self.waiting_for().filter(|_| self.message.is_none())
    }

    fn waiting_for(&self) -> Option<&'static str> {
        #[cfg(not(target_arch = "wasm32"))]
        if self.import_notice == Some(import::Notice::Asked) {
            return Some(NOTICE_TAG);
        }
        if self.pending.is_some() {
            Some(NEWER_TAG)
        } else if self.discarding.is_some() {
            Some(DISCARD_TAG)
        } else if self.overwrite.is_some() {
            Some(OVERWRITE_TAG)
        } else if self.importing.is_some() {
            Some(IMPORT_TAG)
        } else {
            self.locate_asked.then_some(LOCATE_TAG)
        }
    }

    /// Call once per frame, after the screens. Reads `Ctrl+O` / `Ctrl+S` when `shortcuts` (no Save dialog or
    /// message is open; `Ctrl+O` asks first when the song has unsaved changes). Once `answer` comes (to the
    /// question [`Self::waiting`] names), loads the waiting newer project, replaces the song or imports an archive's
    /// files on yes, saves the song first on the extra choice (`Save`, see [`Self::save_then`]), and drops them on no.
    /// Finishes requests. A save renames `song` after its file. Returns the song just opened, for the caller to replace `song` with;
    /// [`Self::take_overlay`] then has what to show.
    pub fn update(
        &mut self,
        ctx: &Context,
        song: &mut Song,
        shortcuts: bool,
        answer: Option<Answer>,
    ) -> Option<Song> {
        self.provider.attach(ctx);
        if shortcuts {
            if ctx.input_mut(|i| i.consume_shortcut(&OPEN_KEY)) {
                self.replace(Replace::Pick);
            }
            // `saveProject` is whatever the keymap binds: Ctrl+S, or Cmd+S on macOS.
            let keys = keymap::for_os(ctx.os());
            if ctx.input_mut(|i| keys.consume_press(i, "saveProject")) {
                self.save(song);
            }
        }
        #[cfg(not(target_arch = "wasm32"))]
        let answer = self.notice_answered(answer);
        let mut opened = None;
        if let Some(answer) = answer {
            if let Some((path, bytes)) = self.pending.take() {
                opened = (answer == Answer::Yes)
                    .then(|| self.load(path, &bytes))
                    .flatten();
            } else if let Some(what) = self.discarding.take() {
                match answer {
                    Answer::Yes => opened = self.replace_now(what),
                    Answer::Extra => self.save_then(song, what),
                    Answer::No => {}
                }
            } else if let Some(name) = self.overwrite.take() {
                match self.export_asked.take() {
                    Some(asked) => self.export_answered(asked, answer),
                    None => self.overwrite_answered(name, answer),
                }
            } else if let Some(entries) = self.importing.take()
                && answer == Answer::Yes
            {
                self.import_files(entries);
            } else if self.locate_asked {
                self.locate_answered(answer == Answer::Yes);
            }
        }
        // A project file dropped on the window opens as a picked one does (asking first with unsaved changes), once
        // no dialog or message is open (Playground drops a file it opens a moment after the page loads). Audio files
        // dropped are the playlist's (`ui::file_drag`).
        let project = |f: &&eframe::egui::DroppedFile| {
            is_project(f.path.as_deref().unwrap_or(Path::new(&f.name)))
        };
        if let Some(file) = ctx.input(|i| i.raw.dropped_files.iter().find(project).cloned()) {
            self.drop_waiting = Some(file);
        }
        if shortcuts && let Some(file) = self.drop_waiting.take() {
            opened = self.dropped(file).or(opened);
        }
        // Last, so completed requests land in this frame (the native open picker may still be pending).
        let changes = self.library_changes;
        opened = self.poll(song).or(opened);
        // After all that may ask something this frame: the import's result waits for it.
        #[cfg(not(target_arch = "wasm32"))]
        self.import_done(ctx, shortcuts);
        // The overlays it asks for show from the next frame on, and the pickers list what came in.
        if opened.is_some()
            || self.library_changes != changes
            || self.message.is_some()
            || self.dialog.is_some()
            || self.preset_dialog.is_some()
        {
            ctx.request_repaint();
        }
        opened
    }
}

/// Reads `reader` to its end, failing with [`too_large`] once it passes `max` bytes. The length a file claims is
/// not trusted: it is 0 for `/dev/zero`, FIFOs and procfs. The web checks `Blob.size` instead.
#[cfg_attr(target_arch = "wasm32", allow(dead_code))]
pub(crate) fn read_limited(reader: impl Read, max: u64) -> Result<Vec<u8>, String> {
    let mut bytes = Vec::new();
    reader
        .take(max.saturating_add(1))
        .read_to_end(&mut bytes)
        .map_err(|e| e.to_string())?;
    if bytes.len() as u64 > max {
        return Err(too_large());
    }
    Ok(bytes)
}

/// The file in the user root every load writes the whole project to.
#[cfg(not(target_arch = "wasm32"))]
pub(crate) const RECENT_STATE: &str = "recent.state";

/// Writes `bytes` to `path` whole or not at all (see [`replace_file`]): a crash or a full disk never leaves half
/// a song.
#[cfg(not(target_arch = "wasm32"))]
pub(crate) fn write_atomic(path: &Path, bytes: &[u8]) -> std::io::Result<()> {
    use std::io::Write;
    replace_file(path, |file| file.write_all(bytes))
}

/// Replaces `path` with what `write` puts in a new hidden temp file beside it, `.<name>.tmp`. The temp file is made
/// new, so it is never opened through a link (one a crash left behind is removed first; removing a link removes
/// only the link). It is synced, then renamed over `path`. On any error it is removed and `path` is left as it was.
#[cfg(not(target_arch = "wasm32"))]
pub(crate) fn replace_file(
    path: &Path,
    write: impl FnOnce(&mut std::fs::File) -> std::io::Result<()>,
) -> std::io::Result<()> {
    use std::fs;
    let name = path.file_name().ok_or(std::io::ErrorKind::InvalidInput)?;
    let mut tmp_name = std::ffi::OsString::from(".");
    tmp_name.push(name);
    tmp_name.push(".tmp");
    let tmp = path.with_file_name(tmp_name);
    let _ = fs::remove_file(&tmp);
    let written = fs::OpenOptions::new()
        .write(true)
        .create_new(true)
        .open(&tmp)
        .and_then(|mut file| {
            write(&mut file)?;
            file.sync_all()
        })
        .and_then(|()| fs::rename(&tmp, path));
    if written.is_err() {
        let _ = fs::remove_file(&tmp);
    }
    written
}

/// `file` read whole, up to `max` bytes: one that says it is longer fails at once, and one that turns out longer
/// fails at `max` ([`read_limited`]).
#[cfg(not(target_arch = "wasm32"))]
pub(crate) fn read_file_limited(file: std::fs::File, max: u64) -> Result<Vec<u8>, String> {
    if file.metadata().is_ok_and(|m| m.len() > max) {
        return Err(too_large());
    }
    read_limited(file, max)
}

/// `song` as a save writes it, titled `title` (HEAD @8 is the file name): stamped now, the Tuners' inputs off.
fn stamped(song: &Song, title: String) -> Song {
    let mut stamped = song.clone();
    // The plugins' states as they are now (the song holds them as added or read).
    crate::plugins::store_states(&mut stamped);
    stamped.stamp_save(&now());
    stamped.title = title;
    tuner_inputs_off(&mut stamped);
    stamped
}

/// Now, as `META` writes it (the format names no time zone).
fn now() -> String {
    #[cfg(target_arch = "wasm32")]
    let secs = {
        let date = web_sys::js_sys::Date::new_0();
        (date.get_time() / 1000.0 - date.get_timezone_offset() * 60.0) as i64
    };
    // UTC on the desktop (std has no time zones), local time on the web.
    #[cfg(not(target_arch = "wasm32"))]
    let secs = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map_or(0, |d| d.as_secs() as i64);
    crate::ui::main_menu::meta_time(secs)
}

/// Now, as `META` `created` has it: dashes in the time.
fn created_now() -> String {
    now().replace(':', "-")
}

/// The detail for a file over [`MAX_FILE_BYTES`].
pub(crate) fn too_large() -> String {
    format!("larger than {} MiB", MAX_FILE_BYTES >> 20)
}

/// Keeps the first [`MAX_NOTICES`] notices and counts the rest in one more line.
pub(super) fn cap_notices(notices: &mut Vec<String>) {
    if notices.len() > MAX_NOTICES {
        let more = notices.len() - MAX_NOTICES;
        notices.truncate(MAX_NOTICES);
        notices.push(format!("and {more} more"));
    }
}

/// The [`STAND_INS`] message over `notices` (capped as load notices are), `None` without any.
pub fn stand_ins(mut notices: Vec<String>) -> Option<Vec<String>> {
    // Fewer than a load's notices: the summary's header and choices fit a small screen with them.
    if notices.len() > MAX_STAND_INS {
        let more = notices.len() - MAX_STAND_INS;
        notices.truncate(MAX_STAND_INS);
        notices.push(format!("and {more} more"));
    }
    (!notices.is_empty()).then(|| {
        std::iter::once(STAND_INS.to_owned())
            .chain(notices)
            .collect()
    })
}

/// The kind of project file `path` names: `.flm`, `.songtmpl`, `.zip`, `.mid`, `.midi` or `.flp`, in any case.
pub fn file_kind(path: &Path) -> Option<FileKind> {
    [
        FileKind::Song,
        FileKind::Template,
        FileKind::Archive,
        FileKind::Midi,
        FileKind::Flp,
    ]
    .into_iter()
    .find(|kind| accepts(path, kind.extensions()))
}

/// Whether `path` names a file [`file_kind`] knows.
pub fn is_project(path: &Path) -> bool {
    file_kind(path).is_some()
}

/// Whether the extension of `path` is one of `accept` (no dot), in any case.
fn accepts(path: &Path, accept: &[&str]) -> bool {
    path.extension()
        .and_then(|e| e.to_str())
        .is_some_and(|ext| accept.iter().any(|a| ext.eq_ignore_ascii_case(a)))
}

/// `accept` as an `<input type="file">` `accept` attribute: `.flm,.zip`.
#[cfg(any(target_arch = "wasm32", test))]
fn accept_attr(accept: &[&str]) -> String {
    let dotted: Vec<_> = accept.iter().map(|ext| format!(".{ext}")).collect();
    dotted.join(",")
}

/// `name` without a `.ext` ending, in any case.
fn strip_ext<'a>(name: &'a str, ext: &str) -> &'a str {
    let split = name
        .len()
        .checked_sub(ext.len() + 1)
        .and_then(|i| Some((name.get(..i)?, name.get(i..)?)));
    match split {
        Some((stem, end)) if end.starts_with('.') && end[1..].eq_ignore_ascii_case(ext) => stem,
        _ => name,
    }
}

/// The file name without extension.
fn stem(path: &Path) -> String {
    path.file_stem()
        .map(|s| s.to_string_lossy().into_owned())
        .unwrap_or_default()
}

/// The HEAD version string, read as the song browser does: `10LF`, HEAD as the first chunk, at least 363
/// bytes long, and the `vstr32` at 358 inside it. `None` for anything else.
fn saved_version(bytes: &[u8]) -> Option<String> {
    let head = bytes.strip_prefix(b"10LF")?.strip_prefix(b"HEAD")?;
    let size = u32::from_le_bytes(head.get(..4)?.try_into().ok()?) as usize;
    // A chunk's size is clamped to the bytes left.
    let payload = &head[4..];
    let payload = &payload[..size.min(payload.len())];
    if payload.len() < 363 {
        return None;
    }
    let len = u32::from_le_bytes(payload[358..362].try_into().ok()?) as usize;
    if len > MAX_VERSION_LEN {
        return None;
    }
    let version = payload.get(362..362usize.checked_add(len)?)?;
    Some(String::from_utf8_lossy(version).into_owned())
}

/// Whether `version` is newer than `than`: dotted numeric compare, missing parts = 0, over the first
/// [`MAX_VERSION_PARTS`] parts. A part counts its leading digits only (`19 G` is 19).
fn newer(version: &str, than: &str) -> bool {
    let parts = |v: &str| -> Vec<u64> {
        v.split('.')
            .take(MAX_VERSION_PARTS)
            .map(|p| {
                let digits: String = p.trim().chars().take_while(char::is_ascii_digit).collect();
                digits.parse().unwrap_or(0)
            })
            .collect()
    };
    let (a, b) = (parts(version), parts(than));
    let n = a.len().max(b.len());
    let at = |v: &[u64], i: usize| v.get(i).copied().unwrap_or(0);
    (0..n).map(|i| at(&a, i)).gt((0..n).map(|i| at(&b, i)))
}

#[cfg(not(target_arch = "wasm32"))]
fn platform_provider() -> Box<dyn FileProvider> {
    Box::new(native::NativeFiles::default())
}

#[cfg(target_arch = "wasm32")]
fn platform_provider() -> Box<dyn FileProvider> {
    Box::new(web::WebFiles::default())
}

#[cfg(test)]
pub(crate) mod mem;
#[cfg(test)]
pub(crate) mod tests;
