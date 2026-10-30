//! The user's samples, for the drum view's sample picker: `Load Sample` in the drum row menu and the `+` add-drum
//! button.
//!
//! - Folders: `My Samples` and `My Recordings`, in the user folder, the one that holds the presets.
//! - File types: `Audio Wave` (`.wav`), `Lossless FLAC Audio` (`.flac`) and `Compressed Audio` (`.mp3`, `.ogg`)
//!   ([`SAMPLE_EXTS`]). `.m4a` is left out.
//!
//! [`ProjectFiles::sample_library`] lists them as [`ProjectFiles::preset_library`] lists presets: on disk on the
//! desktop, the copies this session kept on the web.

use std::io;
use std::path::{Path, PathBuf};

use super::{
    CHORD_SETS_FOLDER, DRUMSET_EXT, DRUMSETS_FOLDER, DWP_EXT, FLGSYNTH_EXT, FLMS_EXT, PRESET_EXT,
    PRESETS_FOLDER, Pick, ProjectFiles, RACK_EXT, RACKS_FOLDER, SAVE_FILE_FAILED, SF2_EXT,
    SOUNDFONTS_FOLDER, UNKNOWN_TYPE,
};
use crate::ui::main_menu::{SONGS_FOLDER, SongEntry, SongLibrary};

/// The user folders samples come from.
pub const SAMPLES_FOLDER: &str = "My Samples";
pub const RECORDINGS_FOLDER: &str = "My Recordings";
/// The audio files a drum pad takes, without the dot, matched without case.
pub const SAMPLE_EXTS: [&str; 4] = ["wav", "flac", "mp3", "ogg"];
/// The user folder MIDI files go to.
pub const MIDI_FOLDER: &str = "My MIDI";
/// The user folders instruments (DirectWave programs, `.dwp`) and audio renders go to.
pub const INSTRUMENTS_FOLDER: &str = "My Instruments";
pub const TRACKS_FOLDER: &str = "My Tracks";
/// The user folder's own folders, made at every start where they are missing ([`make_user_folders`]).
pub const MY_FOLDERS: [&str; 11] = [
    CHORD_SETS_FOLDER,
    DRUMSETS_FOLDER,
    INSTRUMENTS_FOLDER,
    MIDI_FOLDER,
    PRESETS_FOLDER,
    RACKS_FOLDER,
    RECORDINGS_FOLDER,
    SAMPLES_FOLDER,
    SONGS_FOLDER,
    SOUNDFONTS_FOLDER,
    TRACKS_FOLDER,
];
/// The flash after an `Import`.
pub const IMPORTED: &str = "Successfully imported";
/// A failed `Import` of a file of another type.
pub const UNSUPPORTED: &str = "File type not supported";
/// Any other failed `Import`.
pub const IMPORT_FAILED: &str = "Import failed";
/// The marker file the recordings folder holds, which keeps media scanners out.
const NOMEDIA: &str = ".nomedia";

impl ProjectFiles {
    /// The library the sample picker lists: the user folder holds [`SAMPLES_FOLDER`] and [`RECORDINGS_FOLDER`] (even
    /// before they exist), and each of those its folders and [`SAMPLE_EXTS`] files. Read-only.
    pub fn sample_library(&self) -> Box<dyn SongLibrary + '_> {
        Box::new(Samples(self))
    }

    /// The pickers that list audio files offer `Import`: where this session keeps the files (the web) or the app's
    /// private storage holds them (Android), as nothing else puts audio there. The desktop's pickers list its folders.
    pub fn imports_audio(&self) -> bool {
        self.copies || cfg!(target_os = "android")
    }

    /// `Import` in a picker that lists audio files: asks for one (the web's `<input type="file">`), which
    /// [`Self::update`] keeps in `dir`, the picker's own user folder ([`Self::audio_imported`]).
    pub fn import_audio(&mut self, dir: PathBuf) {
        self.import_dir = Some(dir);
        self.open_pick(Pick::Sample);
    }

    /// Keeps the picked audio file `path` (on the web its name alone) in the folder [`Self::import_audio`] named, under
    /// its name or a free `name (n)`, flashes [`IMPORTED`] and counts it in [`Self::library_changes`], so the open
    /// picker lists it. A file with no name to keep it under is [`IMPORT_FAILED`].
    ///
    /// ponytail: on the web each import stays in memory for the session, with no cap on their total; add one (or an
    /// IndexedDB store) if sessions import enough to strain the tab.
    pub(super) fn audio_imported(&mut self, path: &Path, bytes: Vec<u8>) {
        let Some(dir) = self.import_dir.take() else {
            return self.fail(IMPORT_FAILED, path.display().to_string());
        };
        if self.keep_audio(&dir, path, bytes).is_some() {
            self.flash = Some(IMPORTED.into());
        }
    }

    /// Keeps the audio file `path`'s `bytes` in `dir` under its name or a free `name (n)` and counts it in
    /// [`Self::library_changes`]; returns where it is kept. A failure shows its message and gives `None`.
    fn keep_audio(&mut self, dir: &Path, path: &Path, bytes: Vec<u8>) -> Option<PathBuf> {
        let Some(ext) = path.extension() else {
            self.fail(IMPORT_FAILED, path.display().to_string());
            return None;
        };
        let ext = ext.to_string_lossy();
        let name = self.free_name(Some(dir), &super::stem(path), &ext, None);
        let target = dir.join(format!("{name}.{ext}"));
        match self.write_user_file(&target, bytes) {
            Ok(()) => {
                self.library_changes += 1;
                Some(target)
            }
            Err(err) => {
                self.fail(SAVE_FILE_FAILED, format!("{}: {err}", target.display()));
                None
            }
        }
    }

    /// An audio file dropped on the window (the web hands its bytes, the desktop its path), kept in the user's
    /// [`SAMPLES_FOLDER`] as `Import` keeps a picked one ([`Self::keep_audio`]), so that songs refer to the copy. Returns
    /// where it is kept; a file that does not read shows [`IMPORT_FAILED`] and gives `None`.
    pub(crate) fn keep_dropped(&mut self, file: &eframe::egui::DroppedFile) -> Option<PathBuf> {
        let path = (file.path.clone()).unwrap_or_else(|| PathBuf::from(&file.name));
        let bytes = match (&file.bytes, &file.path) {
            (Some(bytes), _) => Ok(bytes.to_vec()),
            #[cfg(not(target_arch = "wasm32"))]
            (None, Some(at)) => std::fs::File::open(at)
                .map_err(|e| e.to_string())
                .and_then(|f| super::read_file_limited(f, super::MAX_FILE_BYTES)),
            _ => Err(String::new()),
        };
        let dir = self.user_dir()?.join(SAMPLES_FOLDER);
        match bytes {
            Ok(bytes) => self.keep_audio(&dir, &path, bytes),
            Err(err) => {
                self.fail(IMPORT_FAILED, format!("{}: {err}", path.display()));
                None
            }
        }
    }

    /// A failed `Import` (the pick or its read): [`UNSUPPORTED`] for a file of another type, else [`IMPORT_FAILED`].
    /// The pick is over.
    pub(super) fn import_failed(&mut self, title: &str, detail: String) {
        (self.pick, self.import_dir) = (None, None);
        let title = if title == UNKNOWN_TYPE {
            UNSUPPORTED
        } else {
            IMPORT_FAILED
        };
        self.fail(title, detail);
    }

    /// The folder recordings are written to, `<user>/My Recordings`, made with its [`NOMEDIA`] file where they are
    /// missing. `None` where the user folder is not on disk (the web, in-memory files) or there is
    /// none: nothing records there.
    pub fn recordings_dir(&self) -> Option<io::Result<PathBuf>> {
        let dir = self
            .user
            .as_ref()
            .filter(|_| !self.copies)?
            .join(RECORDINGS_FOLDER);
        let made = std::fs::create_dir_all(&dir).and_then(|()| {
            match std::fs::File::create_new(dir.join(NOMEDIA)) {
                Err(e) if e.kind() != io::ErrorKind::AlreadyExists => Err(e),
                _ => Ok(()),
            }
        });
        Some(made.map(|()| dir))
    }

    /// Whether the user file `path` exists: on disk on the desktop, among this session's copies on the web.
    pub fn user_file_exists(&self, path: &Path) -> bool {
        match self.copies {
            true => self.user_copies.exists(path),
            false => path.exists(),
        }
    }

    /// Writes `bytes` as the user file `path` (the audio editor's edited sample): this session's copy on
    /// the web; on the desktop the file, whole or not at all, its folder made where it is missing. Never into the
    /// original content's folder.
    pub fn write_user_file(&mut self, path: &Path, bytes: Vec<u8>) -> io::Result<()> {
        // The Original Content folder is read-only (`content`): no path into it writes there, through a link or in
        // another case either.
        if self.in_content(path) {
            return Err(io::Error::new(
                io::ErrorKind::PermissionDenied,
                "original content is read-only",
            ));
        }
        if self.copies {
            return self.user_copies.put(path.to_owned(), &bytes);
        }
        #[cfg(not(target_arch = "wasm32"))]
        {
            let dir = path.parent().unwrap_or(Path::new(""));
            std::fs::create_dir_all(dir).and_then(|()| super::write_atomic(path, &bytes))
        }
        // The web always keeps copies.
        #[cfg(target_arch = "wasm32")]
        Err(io::ErrorKind::Unsupported.into())
    }
}

/// Makes the user folder `user`'s own folders ([`MY_FOLDERS`]) where they are missing.
#[cfg(not(target_arch = "wasm32"))]
pub fn make_user_folders(user: &Path) -> io::Result<()> {
    MY_FOLDERS
        .iter()
        .try_for_each(|folder| std::fs::create_dir_all(user.join(folder)))
}

/// The user folder an archive's file of this name is imported into, by its type, in any case: samples, presets of
/// every kind, SoundFonts, songs ([`SONGS_FOLDER`], the songs folder), MIDI files and instruments. `None` for any other
/// type, and for a hidden file (`.DS_Store`, or the `__MACOSX/._x.wav` resource forks a macOS archive holds).
pub(crate) fn import_folder(path: &Path) -> Option<&'static str> {
    let name = path.file_name()?.to_str()?;
    let ext = path.extension()?.to_str()?.to_ascii_lowercase();
    if name.starts_with('.') {
        return None;
    }
    Some(match ext.as_str() {
        ext if SAMPLE_EXTS.contains(&ext) => SAMPLES_FOLDER,
        PRESET_EXT | FLMS_EXT | FLGSYNTH_EXT => PRESETS_FOLDER,
        RACK_EXT => RACKS_FOLDER,
        DRUMSET_EXT => DRUMSETS_FOLDER,
        SF2_EXT => SOUNDFONTS_FOLDER,
        "flm" => SONGS_FOLDER,
        "mid" | "midi" => MIDI_FOLDER,
        DWP_EXT => INSTRUMENTS_FOLDER,
        _ => return None,
    })
}

/// [`ProjectFiles::sample_library`].
struct Samples<'a>(&'a ProjectFiles);

impl SongLibrary for Samples<'_> {
    fn list(&self, folder: Option<&Path>) -> io::Result<Vec<SongEntry>> {
        let (Some(dir), Some(user)) = (folder, self.0.user_dir()) else {
            return Ok(Vec::new());
        };
        if dir == user {
            let folder = |name: &str| SongEntry {
                name: name.into(),
                path: user.join(name),
                is_folder: true,
                recency: 0,
            };
            return Ok(vec![folder(SAMPLES_FOLDER), folder(RECORDINGS_FOLDER)]);
        }
        let mut entries = self.0.preset_library(&SAMPLE_EXTS).list(Some(dir))?;
        entries.sort_by(|a, b| a.path.cmp(&b.path));
        Ok(entries)
    }

    fn read(&self, path: &Path) -> io::Result<Vec<u8>> {
        self.0.read_user_file(path).map_err(io::Error::other)
    }

    fn store(&mut self, _: &str, _: &[u8]) -> io::Result<()> {
        Err(io::ErrorKind::Unsupported.into())
    }
}

#[cfg(test)]
mod tests {
    use std::path::PathBuf;

    use super::*;
    use crate::project_files::mem::MemFiles;

    /// (name, path, folder) of `dir`'s entries, by path.
    fn listed(files: &ProjectFiles, dir: &Path) -> Vec<(String, PathBuf, bool)> {
        let library = files.sample_library();
        let mut entries: Vec<_> = (library.list(Some(dir)).unwrap().into_iter())
            .map(|e| (e.name, e.path, e.is_folder))
            .collect();
        entries.sort_by(|a, b| a.1.cmp(&b.1));
        entries
    }

    fn folder(user: &Path, name: &str) -> (String, PathBuf, bool) {
        (name.into(), user.join(name), true)
    }

    fn file(path: &Path) -> (String, PathBuf, bool) {
        let name = path.file_name().unwrap().to_string_lossy().into_owned();
        (name, path.to_owned(), false)
    }

    /// At start the user folder gets its eleven own folders where they are missing; what is there stays.
    #[cfg(not(target_arch = "wasm32"))]
    #[test]
    fn the_user_folders_own_folders_are_made_where_missing() {
        let user = std::env::temp_dir().join(format!("honeydew-my-folders-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&user);
        std::fs::create_dir_all(user.join(SAMPLES_FOLDER)).unwrap();
        std::fs::write(user.join(SAMPLES_FOLDER).join("kick.wav"), b"kick").unwrap();
        make_user_folders(&user).unwrap();
        let mut made: Vec<_> = (std::fs::read_dir(&user).unwrap())
            .map(|e| e.unwrap().file_name().into_string().unwrap())
            .collect();
        made.sort();
        let all = [
            "My Chord Sets",
            "My Drumsets",
            "My Instruments",
            "My MIDI",
            "My Presets",
            "My Racks",
            "My Recordings",
            "My Samples",
            "My Songs",
            "My SoundFonts",
            "My Tracks",
        ];
        assert_eq!(made, all);
        assert!(all.iter().all(|f| user.join(f).is_dir()));
        assert_eq!(
            std::fs::read(user.join("My Samples/kick.wav")).unwrap(),
            b"kick"
        );
        let _ = std::fs::remove_dir_all(&user);
    }

    #[test]
    fn on_the_web_the_sample_folders_list_this_sessions_audio_copies() {
        let mut files = ProjectFiles::with_provider(Box::new(MemFiles::default()));
        let user = files.user_dir().unwrap().to_owned();
        let samples = user.join(SAMPLES_FOLDER);
        let audio = ["Kick 01.wav", "Loop.FLAC", "Vox.mp3", "Pad.ogg"].map(|n| samples.join(n));
        let take = user.join(RECORDINGS_FOLDER).join("Take.wav");
        let other = [
            samples.join("Song.flm"),
            samples.join("Voice.m4a"),
            user.join("Kick.wav"),
            user.join("My Presets/Reverb/Hall.flmpst"),
        ];
        for path in audio.iter().chain([&take]).chain(&other) {
            files.keep_user_file(path, b"RIFF".to_vec());
        }
        // The user folder holds the two sample folders, whatever else is there.
        assert_eq!(
            listed(&files, &user),
            [
                folder(&user, RECORDINGS_FOLDER),
                folder(&user, SAMPLES_FOLDER)
            ]
        );
        let mut want: Vec<_> = audio.iter().map(|p| file(p)).collect();
        want.sort_by(|a, b| a.1.cmp(&b.1));
        assert_eq!(listed(&files, &samples), want);
        assert_eq!(listed(&files, &user.join(RECORDINGS_FOLDER)), [file(&take)]);
        let library = files.sample_library();
        assert_eq!(library.read(&take).unwrap(), b"RIFF");
    }

    /// A failed `Import` says so: `File type not supported` for a file of another type, else
    /// `Import failed`. It forgets the pick, so the next file opened is a song again.
    #[test]
    fn a_failed_audio_import_says_so_and_forgets_the_pick() {
        let ctx = eframe::egui::Context::default();
        let mut song = crate::song::Song::default();
        for (picked, title) in [("Notes.txt", UNSUPPORTED), ("Gone.wav", IMPORT_FAILED)] {
            let mem = MemFiles::with_file("Notes.txt", b"text");
            mem.0.borrow_mut().pick = Some(PathBuf::from(picked));
            let mut files = ProjectFiles::with_provider(Box::new(mem));
            let dir = files.user_dir().unwrap().join(SAMPLES_FOLDER);
            files.import_audio(dir);
            assert_eq!(files.update(&ctx, &mut song, false, None), None);
            let message = files.message.take().unwrap();
            assert_eq!(message[0], title, "{picked}");
            assert!(message[1].contains(picked), "{message:?}");
            assert!(
                files.pick.is_none() && files.import_dir.is_none(),
                "{picked}"
            );
            assert_eq!(files.library_changes(), 0);
        }
    }

    /// A picked file with no name to keep it under (no stem and extension) is no silent no-op: `Import failed`.
    #[test]
    fn an_imported_file_with_no_name_says_import_failed() {
        let mut files = ProjectFiles::with_provider(Box::new(MemFiles::default()));
        let dir = files.user_dir().unwrap().join(SAMPLES_FOLDER);
        files.import_audio(dir);
        files.audio_imported(Path::new(".wav"), b"RIFF".to_vec());
        assert_eq!(files.message.take().unwrap(), [IMPORT_FAILED, ".wav"]);
        assert_eq!((files.library_changes(), files.take_flash()), (0, None));
    }

    #[test]
    fn in_memory_files_have_no_recordings_folder() {
        let files = ProjectFiles::with_provider(Box::new(MemFiles::default()));
        assert!(files.recordings_dir().is_none());
    }

    #[cfg(not(target_arch = "wasm32"))]
    #[test]
    fn the_recordings_folder_is_made_once_with_its_nomedia_file() {
        let user =
            std::env::temp_dir().join(format!("honeydew-h11-nomedia-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&user);
        let files = ProjectFiles {
            user: Some(user.clone()),
            copies: false,
            ..ProjectFiles::with_provider(
                Box::<crate::project_files::native::NativeFiles>::default(),
            )
        };
        let dir = user.join(RECORDINGS_FOLDER);
        assert_eq!(files.recordings_dir().unwrap().unwrap(), dir);
        assert_eq!(std::fs::read(dir.join(NOMEDIA)).unwrap(), b"");
        // Made once: a second call keeps the folder's files, its marker included.
        std::fs::write(dir.join(NOMEDIA), b"kept").unwrap();
        std::fs::write(dir.join("Take.wav"), b"RIFF").unwrap();
        assert_eq!(files.recordings_dir().unwrap().unwrap(), dir);
        assert_eq!(std::fs::read(dir.join(NOMEDIA)).unwrap(), b"kept");
        assert!(dir.join("Take.wav").exists());
        // The sample picker does not list the marker.
        assert_eq!(listed(&files, &dir), [file(&dir.join("Take.wav"))]);
        std::fs::remove_dir_all(&user).unwrap();
    }

    #[cfg(not(target_arch = "wasm32"))]
    #[test]
    fn on_the_desktop_the_sample_folders_list_their_folders_and_audio_files() {
        let user = std::env::temp_dir().join(format!("honeydew-k2-samples-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&user);
        let samples = user.join(SAMPLES_FOLDER);
        let kicks = samples.join("Kicks");
        std::fs::create_dir_all(&kicks).unwrap();
        let big = kicks.join("Big.WAV");
        let pad = samples.join("Pad.ogg");
        for path in [
            &big,
            &pad,
            &samples.join(".hidden.wav"),
            &samples.join("notes.txt"),
        ] {
            std::fs::write(path, b"RIFF").unwrap();
        }
        let files = ProjectFiles {
            user: Some(user.clone()),
            copies: false,
            ..ProjectFiles::with_provider(
                Box::<crate::project_files::native::NativeFiles>::default(),
            )
        };
        assert_eq!(
            listed(&files, &user),
            [
                folder(&user, RECORDINGS_FOLDER),
                folder(&user, SAMPLES_FOLDER)
            ]
        );
        assert_eq!(
            listed(&files, &samples),
            [folder(&samples, "Kicks"), file(&pad)]
        );
        assert_eq!(listed(&files, &kicks), [file(&big)]);
        // My Recordings does not exist yet: nothing in it.
        let recordings = files
            .sample_library()
            .list(Some(&user.join(RECORDINGS_FOLDER)));
        assert!(recordings.unwrap_or_default().is_empty());
        std::fs::remove_dir_all(&user).unwrap();
    }
}
