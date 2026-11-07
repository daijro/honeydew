//! Disk saves and, on desktop, system file dialogs (`rfd`).
//! Android saves into its private library and opens files and folders through the system document pickers
//! ([`crate::android::pick_document`], [`crate::android::pick_folder`]); external export awaits a document-provider
//! bridge. iOS saves into its user folder and opens through its document picker ([`crate::ios::pick`]). A folder
//! import's copy runs on a worker (Android's on a Java thread). `Send` shares through the phones' share sheets; the
//! desktop shows the file in its file manager.

use std::fs::File;
use std::path::{Path, PathBuf};
use std::sync::mpsc::{self, Receiver, TryRecvError};

use super::{
    FileEvent, FileProvider, MAX_FILE_BYTES, MAX_IMPORT_BYTES, OPEN_FAILED, READ_FAILED,
    SAVE_FAILED, UNKNOWN_TYPE, accepts, file_kind, read_file_limited, samples::import_folder,
};

#[derive(Default)]
pub struct NativeFiles {
    event: Option<FileEvent>,
    opening: Option<Receiver<Option<FileEvent>>>,
    /// A folder import's copy under way.
    copying: Option<Receiver<FileEvent>>,
    ctx: Option<eframe::egui::Context>,
}

impl NativeFiles {
    /// A channel for a result that another thread sends later: sending repaints, so it shows at once.
    fn later<T: Send + 'static>(&self) -> (impl FnOnce(T) + Send + 'static, Receiver<T>) {
        let (tx, rx) = mpsc::channel();
        let ctx = self.ctx.clone();
        let send = move |result| {
            let _ = tx.send(result);
            if let Some(ctx) = ctx {
                ctx.request_repaint();
            }
        };
        (send, rx)
    }
}

impl FileProvider for NativeFiles {
    #[cfg(not(mobile))]
    fn open(&mut self, accept: &'static [&'static str]) {
        let (done, rx) = self.later();
        self.opening = Some(rx);
        std::thread::spawn(move || {
            let dialog = rfd::AsyncFileDialog::new()
                .add_filter("Supported files", &dialog_extensions(accept))
                .pick_file();
            done(
                pollster::block_on(dialog).map(|file| open_path(file.path().to_path_buf(), accept)),
            );
        });
    }

    /// The picked document arrives as a cache copy (streamed, up to [`MAX_IMPORT_BYTES`]), read like a desktop pick and
    /// then removed; one too large to read stays for its import to move ([`FileEvent::Picked`]).
    #[cfg(target_os = "android")]
    fn open(&mut self, accept: &'static [&'static str]) {
        let (done, rx) = self.later();
        self.opening = Some(rx);
        crate::android::pick_document(MAX_IMPORT_BYTES, move |picked| {
            done(picked.map(|picked| match picked {
                Ok(path) => {
                    let event = open_path(path.clone(), accept);
                    if !matches!(event, FileEvent::Picked(_)) {
                        let _ = std::fs::remove_file(&path);
                    }
                    event
                }
                Err(err) => FileEvent::Failed(OPEN_FAILED, err),
            }));
        });
    }

    /// iOS copies the picked file into the app's `tmp`, read like a desktop pick and then removed; one too large to read
    /// stays for its import to copy ([`FileEvent::Picked`]).
    #[cfg(target_os = "ios")]
    fn open(&mut self, accept: &'static [&'static str]) {
        let (done, rx) = self.later();
        self.opening = Some(rx);
        crate::ios::pick(false, move |picked| {
            done(picked.map(|path| {
                let event = open_path(path.clone(), accept);
                if !matches!(event, FileEvent::Picked(_)) {
                    let _ = std::fs::remove_file(&path);
                }
                event
            }));
        });
    }

    /// The folder dialog, then its files listed, on a worker.
    #[cfg(not(mobile))]
    fn open_folder(&mut self) {
        let (done, rx) = self.later();
        self.opening = Some(rx);
        std::thread::spawn(move || {
            let dir = pollster::block_on(rfd::AsyncFileDialog::new().pick_folder());
            done(dir.map(|dir| FileEvent::Folder(folder_files(dir.path()))));
        });
    }

    /// iOS's folder picker, then its files listed in place, on a worker; the copy reads them there too.
    #[cfg(target_os = "ios")]
    fn open_folder(&mut self) {
        let (done, rx) = self.later();
        self.opening = Some(rx);
        crate::ios::pick(true, move |picked| match picked {
            Some(dir) => drop(std::thread::spawn(move || {
                done(Some(FileEvent::Folder(folder_files(&dir))))
            })),
            None => done(None),
        });
    }

    /// The system folder picker; Java lists the files of the folder picked.
    #[cfg(target_os = "android")]
    fn open_folder(&mut self) {
        let (done, rx) = self.later();
        self.opening = Some(rx);
        crate::android::pick_folder(move |picked| {
            done(picked.map(|picked| match picked {
                Ok(files) => FileEvent::Folder(files),
                Err(err) => FileEvent::Failed(OPEN_FAILED, err),
            }));
        });
    }

    /// Android's folder sources are documents, which Java copies; a path is a big pick's cache copy, which moves.
    fn copy(&mut self, files: Vec<(String, PathBuf)>) {
        let (done, rx) = self.later();
        self.copying = Some(rx);
        #[cfg(target_os = "android")]
        if !files.iter().all(|(from, _)| Path::new(from).is_absolute()) {
            return crate::android::copy_files(&files, move |error| done(FileEvent::Copied(error)));
        }
        let moves = cfg!(target_os = "android");
        std::thread::spawn(move || done(FileEvent::Copied(copy_files(&files, moves).err())));
    }

    fn save(&mut self, path: &Path, ask: bool, bytes: Vec<u8>) {
        #[cfg(mobile)]
        let (path, _) = (path.to_owned(), ask);
        #[cfg(not(mobile))]
        let path = {
            let filter = save_filter(path);
            if ask {
                let mut dialog = rfd::FileDialog::new();
                if let Some((label, ext)) = &filter {
                    dialog = dialog.add_filter(label, &[ext]);
                }
                if let Some(name) = path.file_name() {
                    dialog = dialog.set_file_name(name.to_string_lossy());
                }
                if let Some(dir) = path.parent().filter(|d| !d.as_os_str().is_empty()) {
                    // The songs folder, where the first save starts, may not exist yet.
                    let _ = std::fs::create_dir_all(dir);
                    if dir.is_dir() {
                        dialog = dialog.set_directory(dir);
                    }
                }
                let Some(chosen) = dialog.save_file() else {
                    self.event = Some(FileEvent::SaveCancelled(path.to_owned()));
                    return;
                };
                match &filter {
                    Some((_, ext)) => with_extension(chosen, ext),
                    None => chosen,
                }
            } else {
                path.to_owned()
            }
        };
        // A save may be the first into its folder (the songs folder of a fresh install, a render's stem folder),
        // which nothing makes before.
        let folder = match path.parent().filter(|d| !d.as_os_str().is_empty()) {
            Some(dir) => std::fs::create_dir_all(dir),
            None => Ok(()),
        };
        self.event = Some(
            match folder.and_then(|()| super::write_atomic(&path, &bytes)) {
                Ok(()) => FileEvent::Saved(path),
                Err(err) => FileEvent::Failed(SAVE_FAILED, format!("{}: {err}", path.display())),
            },
        );
    }

    fn poll(&mut self) -> Option<FileEvent> {
        if let Some(event) = self.event.take() {
            return Some(event);
        }
        match self.copying.as_ref().map(Receiver::try_recv) {
            None | Some(Err(TryRecvError::Empty)) => {}
            Some(done) => {
                self.copying = None;
                return Some(done.unwrap_or(FileEvent::Copied(Some("the copy stopped".into()))));
            }
        }
        match self.opening.as_ref()?.try_recv() {
            Ok(result) => {
                self.opening = None;
                result
            }
            Err(TryRecvError::Disconnected) => {
                self.opening = None;
                None
            }
            Err(TryRecvError::Empty) => None,
        }
    }

    fn attach(&mut self, ctx: &eframe::egui::Context) {
        self.ctx = Some(ctx.clone());
    }

    fn opening(&self) -> bool {
        self.opening.is_some()
    }

    fn exists(&self, path: &Path) -> bool {
        path.exists()
    }

    #[cfg(not(mobile))]
    fn locate(&mut self, names: Vec<String>) {
        self.event = super::flp::locate_native(&names);
    }

    fn share(&mut self, files: &[PathBuf], mime: &str) -> Result<(), String> {
        share(files, mime)
    }
}

/// `Send` (#72: the Save dialog's, SONGS') on a phone: the system's share sheet ([`crate::android::share`], [`crate::ios::share`]).
#[cfg(target_os = "android")]
pub(crate) use crate::android::share;
#[cfg(target_os = "ios")]
pub(crate) use crate::ios::share;

/// `Send` on the desktop, which has no share sheet: the file manager shows the first file.
#[cfg(not(mobile))]
pub(crate) fn share(files: &[PathBuf], _mime: &str) -> Result<(), String> {
    crate::ui::main_menu::reveal(files.first().ok_or("Nothing to send")?)
}

/// `accept` for the open dialog: each extension and its upper-case form, because GTK's patterns are case-sensitive
/// (`Tune.MID` would not be offered by `mid`). [`accepts`] still checks the picked file case-insensitively.
#[cfg(not(mobile))]
fn dialog_extensions(accept: &[&str]) -> Vec<String> {
    accept
        .iter()
        .flat_map(|e| [e.to_string(), e.to_ascii_uppercase()])
        .collect()
}

/// The files in `dir` and below, each as its path from `dir`'s parent (`Drums/Kit/Kick.wav`) and on disk. Hidden files
/// and folders are left out, as the import skips them, and so are links (never followed, so no loop) and names that
/// are not UTF-8.
#[cfg(not(target_os = "android"))]
pub(super) fn folder_files(dir: &Path) -> Vec<(String, String)> {
    let base = dir.parent().unwrap_or(dir);
    let (mut files, mut todo) = (Vec::new(), vec![dir.to_owned()]);
    while let Some(at) = todo.pop() {
        for entry in std::fs::read_dir(&at).into_iter().flatten().flatten() {
            let (path, kind) = (entry.path(), entry.file_type());
            if entry
                .file_name()
                .to_str()
                .is_none_or(|n| n.starts_with('.'))
            {
                continue;
            }
            match kind {
                Ok(kind) if kind.is_dir() => todo.push(path),
                Ok(kind) if kind.is_file() => {
                    let rel = path.strip_prefix(base).unwrap_or(&path).to_str();
                    if let (Some(rel), Some(from)) = (rel, path.to_str()) {
                        files.push((rel.to_owned(), from.to_owned()));
                    }
                }
                _ => {}
            }
        }
    }
    files
}

/// Copies each file `(from, to)`, making its folder: whole or not at all (beside it first, then renamed into place); with
/// `moves`, moved where it can be. The first that fails stops the copy, and says why.
fn copy_files(files: &[(String, PathBuf)], moves: bool) -> Result<(), String> {
    for (from, to) in files {
        let name = to.file_name().unwrap_or_default().to_string_lossy();
        let part = to.with_file_name(format!(".{name}.part"));
        let folder = to.parent().map_or(Ok(()), std::fs::create_dir_all);
        // A move of the provider's own copy: no second copy on a phone's storage, where one file system holds both.
        let moved = moves && folder.is_ok() && std::fs::rename(from, &part).is_ok();
        let copied = folder
            .and_then(|()| match moved {
                true => Ok(0),
                false => std::fs::copy(from, &part),
            })
            .and_then(|_| std::fs::rename(&part, to));
        if let Err(err) = copied {
            let _ = std::fs::remove_file(&part);
            return Err(format!("{from}: {err}"));
        }
    }
    Ok(())
}

/// Reads the picked file, if its type is one of `accept`. One past [`MAX_FILE_BYTES`] that a user folder takes (not a
/// song or an archive, which load whole) is left unread up to [`MAX_IMPORT_BYTES`], for its import to copy.
pub(super) fn open_path(path: PathBuf, accept: &[&str]) -> FileEvent {
    if !accepts(&path, accept) {
        return FileEvent::Failed(UNKNOWN_TYPE, path.display().to_string());
    }
    let len = std::fs::metadata(&path).map_or(0, |m| m.len());
    // By its name: the folders the pick came from are no archive's, so a hidden one (`~/.local`) is no junk.
    let named = Path::new(path.file_name().unwrap_or_default());
    let copied = file_kind(&path).is_none() && import_folder(named).is_some();
    if copied && len > MAX_FILE_BYTES {
        return match len <= MAX_IMPORT_BYTES {
            true => FileEvent::Picked(path),
            false => {
                let detail = format!(
                    "{}: larger than {} MiB",
                    path.display(),
                    MAX_IMPORT_BYTES >> 20
                );
                FileEvent::Failed(READ_FAILED, detail)
            }
        };
    }
    match read(&path) {
        Ok(bytes) => FileEvent::Opened(path, bytes),
        Err((title, err)) => FileEvent::Failed(title, format!("{}: {err}", path.display())),
    }
}

/// The save dialog's filter for a save suggested as `path`: its label and its extension (lower case). None for a
/// name with no extension.
#[cfg(not(mobile))]
fn save_filter(path: &Path) -> Option<(String, String)> {
    let ext = path.extension()?.to_str()?.to_ascii_lowercase();
    let label = match ext.as_str() {
        "flm" => "Songs".to_owned(),
        _ => format!("{} files", ext.to_ascii_uppercase()),
    };
    Some((label, ext))
}

/// The path the save dialog returned, with `.ext` added unless it ends in `.ext` (any case): a name edited to
/// `Beat v1.3` is `Beat v1.3.flm`, not a file of type "3".
#[cfg(not(mobile))]
fn with_extension(path: PathBuf, ext: &str) -> PathBuf {
    if path
        .extension()
        .is_some_and(|e| e.eq_ignore_ascii_case(ext))
    {
        return path;
    }
    let mut path = path.into_os_string();
    path.push(".");
    path.push(ext);
    path.into()
}

/// The whole file, up to [`MAX_FILE_BYTES`]. Errors carry the popup title.
fn read(path: &Path) -> Result<Vec<u8>, (&'static str, String)> {
    let file = File::open(path).map_err(|e| (OPEN_FAILED, e.to_string()))?;
    read_file_limited(file, MAX_FILE_BYTES).map_err(|e| (READ_FAILED, e))
}

#[cfg(all(test, not(target_os = "android")))]
mod tests {
    use super::*;
    use crate::project_files::{Pick, UNKNOWN_TYPE};

    #[test]
    fn pending_open_is_polled_without_waiting_for_the_picker() {
        let (tx, rx) = mpsc::channel();
        let mut files = NativeFiles {
            opening: Some(rx),
            ..Default::default()
        };
        assert!(files.opening());
        assert_eq!(files.poll(), None);
        let event = FileEvent::Opened(PathBuf::from("Song.flm"), b"10LF".to_vec());
        tx.send(Some(event.clone())).unwrap();
        assert_eq!(files.poll(), Some(event));
        assert!(!files.opening());
        assert_eq!(files.poll(), None);
    }

    #[test]
    fn cancelling_an_open_clears_the_pending_picker() {
        let (tx, rx) = mpsc::channel();
        let mut files = NativeFiles {
            opening: Some(rx),
            ..Default::default()
        };
        tx.send(None).unwrap();
        assert_eq!(files.poll(), None);
        assert!(!files.opening());
    }

    #[test]
    fn the_type_is_checked_before_the_file_is_opened() {
        let event = open_path(PathBuf::from("/no/such/dir/notes.txt"), Pick::Song.accept());
        assert_eq!(
            event,
            FileEvent::Failed(UNKNOWN_TYPE, "/no/such/dir/notes.txt".into())
        );
    }

    #[test]
    fn a_picked_project_is_read() {
        let path = std::env::temp_dir().join(format!("honeydew-m3t5-{}.FLM", std::process::id()));
        std::fs::write(&path, b"10LF").unwrap();
        let event = open_path(path.clone(), Pick::Song.accept());
        std::fs::remove_file(&path).unwrap();
        assert_eq!(event, FileEvent::Opened(path, b"10LF".to_vec()));
    }

    #[test]
    fn a_save_path_without_the_requested_extension_gets_it() {
        let flm = |p: &str| with_extension(PathBuf::from(p), "flm");
        assert_eq!(flm("/a/Groove"), PathBuf::from("/a/Groove.flm"));
        assert_eq!(flm("/a/Groove.flm"), PathBuf::from("/a/Groove.flm"));
        assert_eq!(flm("/a/Groove.FLM"), PathBuf::from("/a/Groove.FLM"));
        // A dot in the name is no extension.
        assert_eq!(flm("/a/Groove.v2"), PathBuf::from("/a/Groove.v2.flm"));
        assert_eq!(flm("/a/Beat v1.3"), PathBuf::from("/a/Beat v1.3.flm"));
        // The extension is the save's own, not always `.flm`.
        let wav = |p: &str| with_extension(PathBuf::from(p), "wav");
        assert_eq!(wav("/a/Mix"), PathBuf::from("/a/Mix.wav"));
        assert_eq!(wav("/a/Mix.WAV"), PathBuf::from("/a/Mix.WAV"));
        assert_eq!(wav("/a/Mix.flm"), PathBuf::from("/a/Mix.flm.wav"));
        assert_eq!(wav("/a/Mix v1.3"), PathBuf::from("/a/Mix v1.3.wav"));
    }

    #[test]
    fn the_open_filter_lists_each_extension_in_both_cases() {
        // GTK's patterns are case-sensitive: `Tune.MID` and `Song.FLM` need their own entries.
        let exts = dialog_extensions(&["flm", "mid"]);
        assert_eq!(exts, ["flm", "FLM", "mid", "MID"]);
        // The shared list is untouched.
        assert_eq!(Pick::Song.accept()[0], "flm");
    }

    #[test]
    fn the_save_filter_is_named_after_the_extension_of_the_path() {
        let filter = |p: &str| save_filter(Path::new(p));
        let some = |label: &str, ext: &str| Some((label.to_owned(), ext.to_owned()));
        assert_eq!(filter("/a/Groove.flm"), some("Songs", "flm"));
        assert_eq!(filter("/a/Mix.wav"), some("WAV files", "wav"));
        assert_eq!(filter("/a/Mix.MP3"), some("MP3 files", "mp3"));
        assert_eq!(filter("/a/Mix"), None);
    }
}

#[cfg(test)]
mod save_tests {
    use super::*;

    /// A save whose temp file cannot be made (a folder is in its place) fails and leaves the song as it was: the
    /// song file is never written in place.
    #[test]
    fn a_quick_save_writes_a_temp_file_and_never_the_song_in_place() {
        let dir = std::env::temp_dir().join(format!("honeydew-native-save-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(dir.join(".Beat.flm.tmp")).unwrap();
        let path = dir.join("Beat.flm");
        std::fs::write(&path, b"old song").unwrap();
        let mut files = NativeFiles::default();
        files.save(&path, false, b"new song".to_vec());
        assert!(matches!(
            files.poll(),
            Some(FileEvent::Failed(SAVE_FAILED, _))
        ));
        assert_eq!(std::fs::read(&path).unwrap(), b"old song");
        std::fs::remove_dir(dir.join(".Beat.flm.tmp")).unwrap();
        files.save(&path, false, b"new song".to_vec());
        assert_eq!(files.poll(), Some(FileEvent::Saved(path.clone())));
        assert_eq!(std::fs::read(&path).unwrap(), b"new song");
        std::fs::remove_dir_all(&dir).unwrap();
    }

    /// A render's file keeps its own extension, and a stem folder is made as the file is written.
    #[test]
    fn a_save_keeps_its_wav_extension_and_makes_its_folder() {
        let dir = std::env::temp_dir().join(format!("honeydew-native-wav-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        let folder = dir.join("My Tracks").join("Song - Multitrack wav");
        let path = folder.join("Bass.wav");
        let mut files = NativeFiles::default();
        files.save(&path, false, b"RIFF".to_vec());
        assert_eq!(files.poll(), Some(FileEvent::Saved(path.clone())));
        assert_eq!(std::fs::read(&path).unwrap(), b"RIFF");
        let names: Vec<_> = std::fs::read_dir(&folder)
            .unwrap()
            .map(|e| e.unwrap().file_name())
            .collect();
        assert_eq!(names, ["Bass.wav"]);
        std::fs::remove_dir_all(&dir).unwrap();
    }

    /// The first quick save on a fresh install goes into a songs folder nothing has made yet.
    #[test]
    fn a_quick_save_makes_the_folder_it_saves_into() {
        let dir =
            std::env::temp_dir().join(format!("honeydew-native-mkdir-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        let path = dir.join("honeydew").join("My Songs").join("New Song.flm");
        let mut files = NativeFiles::default();
        files.save(&path, false, b"new song".to_vec());
        assert_eq!(files.poll(), Some(FileEvent::Saved(path.clone())));
        assert_eq!(std::fs::read(&path).unwrap(), b"new song");
        std::fs::remove_dir_all(&dir).unwrap();
    }
}
