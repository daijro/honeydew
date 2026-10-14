//! The SoundFont Player's files: the user's `My SoundFonts` folder, which the patch picker lists, and the SoundFont a
//! module's `SF2D` names, read for its patches. The app ships no SoundFonts: the user folder holds them, and the
//! Original Content folder's `SoundFonts` folder, once the user dropped it in ([`super::content`]).

use std::path::{Component, Path, PathBuf};

use super::ProjectFiles;
use crate::flm::chunk::{ContentPath, ContentRoot};
use crate::sf2::{Preset, Sf2Error};

/// The user folder SoundFonts are in.
pub const SOUNDFONTS_FOLDER: &str = "My SoundFonts";
/// The SoundFont file type.
pub const SF2_EXT: &str = "sf2";

impl ProjectFiles {
    /// `My SoundFonts`. `None` without a user folder.
    pub fn soundfonts_dir(&self) -> Option<PathBuf> {
        Some(self.user.as_ref()?.join(SOUNDFONTS_FOLDER))
    }

    /// Where the file `path` (an `SF2D` path) names is, as the SoundFont picker's library lists it: in the user folder
    /// (`@L@`) or the content folder (`@R@`, [`Self::resolve`]). `None` for any other path (see [`Self::soundfont`]),
    /// and for one that could leave its folder (a root, a `..`): a song file must not read any other file.
    pub fn library_path(&self, path: &ContentPath) -> Option<PathBuf> {
        in_library(path)?;
        self.resolve(path)
    }

    /// Whether the relative path a song file holds names something inside the root it is joined to: not empty, and
    /// plain names only, so no root, drive, `..`, or leading `./` (or `.`). `.` further in is dropped
    /// (`Path::components`): `My Samples/./a.wav` is `My Samples/a.wav`. The guard on every path a song file names.
    pub(crate) fn stays_inside(relative: &Path) -> bool {
        let mut parts = relative.components().peekable();
        parts.peek().is_some() && parts.all(|c| matches!(c, Component::Normal(_)))
    }

    /// The patches of the SoundFont `path` (an `SF2D` path) names. Only a user-folder or content-folder shorthand can
    /// name one: `SF2D` holds no other path.
    pub fn soundfont(&self, path: &ContentPath) -> Result<Vec<Preset>, Sf2Error> {
        let path = self.library_path(path).ok_or(Sf2Error::NotFound)?;
        // ponytail: reads the whole file for its headers, once per SoundFont (the rack keeps what it read); seek
        // past the `sdta` samples if large banks stall the frame.
        let read = self.read_user_file(&path);
        let bytes = read.map_err(|e| read_error(&e, !self.copies && path.exists()))?;
        crate::sf2::presets(&bytes)
    }
}

/// The path under the user folder that the `@L@` path `path` names, when it is a file inside it
/// ([`ProjectFiles::stays_inside`]); `None` for any other path (the audio editor's own-folder rule).
pub(crate) fn in_user_folder(path: &ContentPath) -> Option<&Path> {
    match path {
        ContentPath::Shorthand {
            root: ContentRoot::User,
            ..
        } => in_library(path),
        _ => None,
    }
}

/// The path under its folder that the `SF2D` path `path` names: only a user-folder or content-folder shorthand that
/// cannot leave it (no root, no `..`), as a song file must not read any other file. The engine's SoundFont load checks
/// the same (`sample_cache`).
pub(crate) fn in_library(path: &ContentPath) -> Option<&Path> {
    let ContentPath::Shorthand {
        root: ContentRoot::User | ContentRoot::App,
        relative,
    } = path
    else {
        return None;
    };
    let relative = Path::new(relative);
    ProjectFiles::stays_inside(relative).then_some(relative)
}

/// The [`Sf2Error`] of a failed read whose message is `error`, of a file that is `there` or not.
fn read_error(error: &str, there: bool) -> Sf2Error {
    match error {
        _ if error == super::too_large() => Sf2Error::TooLarge,
        _ if there => Sf2Error::Unreadable,
        _ => Sf2Error::NotFound,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::project_files::mem::MemFiles;
    use crate::sf2::tests::soundfont;

    fn user(relative: &str) -> ContentPath {
        ContentPath::Shorthand {
            root: ContentRoot::User,
            relative: relative.into(),
        }
    }

    fn names(read: Result<Vec<Preset>, Sf2Error>) -> Result<Vec<String>, Sf2Error> {
        read.map(|p| p.into_iter().map(|p| p.name).collect())
    }

    #[test]
    fn a_file_over_the_size_limit_is_too_large_not_missing() {
        let error = |e: String, there| read_error(&e, there);
        assert_eq!(
            error(crate::project_files::too_large(), true),
            Sf2Error::TooLarge
        );
        assert_eq!(error("denied".into(), true), Sf2Error::Unreadable);
        assert_eq!(error("not found".into(), false), Sf2Error::NotFound);
    }

    #[test]
    fn on_the_web_my_soundfonts_holds_this_sessions_copies() {
        let mut files = ProjectFiles::with_provider(Box::new(MemFiles::default()));
        let dir = files.soundfonts_dir().unwrap();
        assert_eq!(dir, Path::new("My SoundFonts"));
        let path = dir.join("Keys").join("Piano.sf2");
        files.keep_user_file(&path, soundfont(&[("Grand", 0, 0), ("Upright", 1, 0)]));
        let shorthand = files.shorthand(&path).unwrap();
        assert_eq!(shorthand, user("My SoundFonts/Keys/Piano.sf2"));
        assert_eq!(files.library_path(&shorthand), Some(path));
        // An empty path names the user folder itself, not a file in it: nothing, as for a sample's path.
        assert_eq!(files.library_path(&user("")), None);
        assert_eq!(
            names(files.soundfont(&shorthand)),
            Ok(vec!["Grand".into(), "Upright".into()])
        );
        assert_eq!(
            names(files.soundfont(&user("My SoundFonts/Gone.sf2"))),
            Err(Sf2Error::NotFound)
        );
        files.keep_user_file(Path::new("My SoundFonts/Bad.sf2"), b"RIFF".to_vec());
        assert_eq!(
            names(files.soundfont(&user("My SoundFonts/Bad.sf2"))),
            Err(Sf2Error::NotSoundFont)
        );
        // Whole patch headers over cut-off samples: the engine cannot play it, so the rack reads an error, not patches.
        let cut = crate::sf2::tests::cut_off_samples(&[("Grand", 0, 0)]);
        files.keep_user_file(Path::new("My SoundFonts/Cut.sf2"), cut);
        assert_eq!(
            names(files.soundfont(&user("My SoundFonts/Cut.sf2"))),
            Err(Sf2Error::Corrupted)
        );
    }

    #[cfg(not(target_arch = "wasm32"))]
    #[test]
    fn on_the_desktop_my_soundfonts_is_in_the_user_folder() {
        let root =
            std::env::temp_dir().join(format!("honeydew-k3-soundfonts-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&root);
        let files = ProjectFiles {
            user: Some(root.clone()),
            copies: false,
            ..ProjectFiles::with_provider(
                Box::<crate::project_files::native::NativeFiles>::default(),
            )
        };
        let dir = files.soundfonts_dir().unwrap();
        assert_eq!(dir, root.join("My SoundFonts"));
        std::fs::create_dir_all(&dir).unwrap();
        let path = dir.join("Pad.sf2");
        std::fs::write(&path, soundfont(&[("Warm", 0, 0)])).unwrap();
        let shorthand = files.shorthand(&path).unwrap();
        assert_eq!(shorthand, user("My SoundFonts/Pad.sf2"));
        assert_eq!(files.library_path(&shorthand).as_ref(), Some(&path));
        assert_eq!(names(files.soundfont(&shorthand)), Ok(vec!["Warm".into()]));
        // Outside the user folder: no shorthand. Other roots and plain paths name nothing that is there (the content
        // folder holds no `My SoundFonts`).
        assert_eq!(files.shorthand(&std::env::temp_dir().join("Pad.sf2")), None);
        let app = ContentPath::Shorthand {
            root: ContentRoot::App,
            relative: "My SoundFonts/Pad.sf2".into(),
        };
        let plain = ContentPath::Plain(path.to_string_lossy().into_owned());
        // A path that leaves the user folder names nothing, though the file is there.
        let up = format!(
            "../{}/My SoundFonts/Pad.sf2",
            root.file_name().unwrap().to_string_lossy()
        );
        let escapes = [
            path.to_string_lossy().into_owned(),
            up,
            "My SoundFonts/../My SoundFonts/Pad.sf2".to_owned(),
        ];
        let escapes = escapes.map(|p| user(&p));
        for path in [app, plain].into_iter().chain(escapes) {
            assert_eq!(files.library_path(&path), None, "{path:?}");
            assert_eq!(
                names(files.soundfont(&path)),
                Err(Sf2Error::NotFound),
                "{path:?}"
            );
        }
        // A file that is there but cannot be read is not missing.
        std::fs::create_dir(dir.join("Folder.sf2")).unwrap();
        let folder = files.soundfont(&user("My SoundFonts/Folder.sf2"));
        assert_eq!(folder, Err(Sf2Error::Unreadable));
        std::fs::remove_dir_all(&root).unwrap();
    }
}
