//! Disk saves and, on desktop, system file dialogs (`rfd`).
//! Android saves into its private library; external import/export awaits a document-provider bridge.

#[cfg(not(target_os = "android"))]
use std::fs::File;
use std::path::Path;
#[cfg(not(target_os = "android"))]
use std::path::PathBuf;

use super::{FileEvent, FileProvider, SAVE_FAILED};
#[cfg(not(target_os = "android"))]
use super::{MAX_FILE_BYTES, OPEN_FAILED, READ_FAILED, UNKNOWN_TYPE, accepts, read_file_limited};

#[derive(Default)]
pub struct NativeFiles {
    event: Option<FileEvent>,
}

// ponytail: the dialogs block the UI thread while open (one frozen frame); rfd's AsyncFileDialog on a task if the
// app must keep drawing (audio runs on its own thread either way).
impl FileProvider for NativeFiles {
    #[cfg(not(target_os = "android"))]
    fn open(&mut self, accept: &'static [&'static str]) {
        let Some(path) = rfd::FileDialog::new()
            .add_filter("Supported files", &dialog_extensions(accept))
            .pick_file()
        else {
            return;
        };
        self.event = Some(open_path(path, accept));
    }

    #[cfg(target_os = "android")]
    fn open(&mut self, _: &'static [&'static str]) {
        self.event = Some(FileEvent::Failed(
            super::OPEN_FAILED,
            "External file import is not available in this Android preview.".into(),
        ));
    }

    fn save(&mut self, path: &Path, ask: bool, bytes: Vec<u8>) {
        #[cfg(target_os = "android")]
        let (path, _) = (path.to_owned(), ask);
        #[cfg(not(target_os = "android"))]
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
        self.event.take()
    }

    fn exists(&self, path: &Path) -> bool {
        path.exists()
    }

    #[cfg(not(target_os = "android"))]
    fn locate(&mut self, names: Vec<String>) {
        self.event = super::flp::locate_native(&names);
    }
}

/// `accept` for the open dialog: each extension and its upper-case form, because GTK's patterns are case-sensitive
/// (`Tune.MID` would not be offered by `mid`). [`accepts`] still checks the picked file case-insensitively.
#[cfg(not(target_os = "android"))]
fn dialog_extensions(accept: &[&str]) -> Vec<String> {
    accept
        .iter()
        .flat_map(|e| [e.to_string(), e.to_ascii_uppercase()])
        .collect()
}

/// Reads the picked file, if its type is one of `accept`.
#[cfg(not(target_os = "android"))]
fn open_path(path: PathBuf, accept: &[&str]) -> FileEvent {
    if !accepts(&path, accept) {
        return FileEvent::Failed(UNKNOWN_TYPE, path.display().to_string());
    }
    match read(&path) {
        Ok(bytes) => FileEvent::Opened(path, bytes),
        Err((title, err)) => FileEvent::Failed(title, format!("{}: {err}", path.display())),
    }
}

/// The save dialog's filter for a save suggested as `path`: its label and its extension (lower case). None for a
/// name with no extension.
#[cfg(not(target_os = "android"))]
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
#[cfg(not(target_os = "android"))]
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
#[cfg(not(target_os = "android"))]
fn read(path: &Path) -> Result<Vec<u8>, (&'static str, String)> {
    let file = File::open(path).map_err(|e| (OPEN_FAILED, e.to_string()))?;
    read_file_limited(file, MAX_FILE_BYTES).map_err(|e| (READ_FAILED, e))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::project_files::{Pick, UNKNOWN_TYPE};

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
