//! The Original Content folder: where someone who owns the source app drops its content tree (sounds, drum kits,
//! instruments, presets) for personal use. The app reads it in place and never copies, uploads or bundles any of it.
//!
//! - **Where.** `<user folder>/Original Content` ([`CONTENT_FOLDER`]), beside `My Songs`: the user folder is the one
//!   `@L@` names. The app makes it on start, with a [`README_FILE`] saying what goes there
//!   ([`ProjectFiles::make_content_folder`]). The web has none: it would need an upload of its own.
//! - **`@R@`.** The shorthand for the app content root, the OBB tree; here that is this folder.
//!   [`ProjectFiles::resolve`] maps every shorthand to its file, with the same guard for `@L@` and `@R@`: only
//!   plain components (no root, drive, `..` or leading `.`), and for `@R@` no link that leads out of the folder. A
//!   file that is not there is missing, as before.
//! - **`@R@Built-in/…`** is our own generated content ([`built_in`]): the sample cache looks it up in code
//!   (`honeydew_audio::builtin_drums`) first, and it never reads the content folder, so a `Built-in` folder dropped in
//!   there never shadows it.
//! - **Untrusted.** The folder is the user's: the pickers list one folder at a time, skip links and hidden entries
//!   and stop at [`crate::ui::main_menu::library::MAX_LISTED`] entries; every file goes through the hardened readers
//!   and [`crate::project_files::MAX_FILE_BYTES`].
//!
//! **Archive import.** The user's own `.obb` or `.apk`, dropped into the folder, unpacks there once at its
//! entries' relative paths, and everything here picks them up: [`super::import`].

use std::io;
use std::path::{Component, Path, PathBuf};

use super::ProjectFiles;
use crate::flm::chunk::{ContentPath, ContentRoot};
use crate::song::ModuleInstance;

/// The folder's name, in the user folder.
pub const CONTENT_FOLDER: &str = "Original Content";
/// The note the app writes into the folder.
pub const README_FILE: &str = "README.txt";
/// Top folders a picker lists.
pub const DRUM_SAMPLES: &str = "Drum Samples";
pub const DRUM_TEMPLATES: &str = "Drum Templates";
pub const RACK_PRESETS: &str = "Rack Presets";
pub const SOUNDFONTS: &str = "SoundFonts";
/// The content tree's top folders, as the README lists them.
pub const TOP_FOLDERS: [&str; 15] = [
    DRUM_SAMPLES,
    DRUM_TEMPLATES,
    "DirectWave Samples",
    "DirectWave Samples EXP",
    "Loop Samples",
    SOUNDFONTS,
    "Synth Presets2",
    "Effect Presets",
    RACK_PRESETS,
    "3xOsc Shapes",
    "GMSynthWaveshapes",
    "Chord Sets",
    "New Song Templates",
    "Songs/Demo Songs",
    KEYBOARD_SHORTCUTS,
];

/// The folder of the shortcut files.
pub(super) const KEYBOARD_SHORTCUTS: &str = "Keyboard Shortcuts";

/// The first folder of our own built-in content under `@R@` (`Built-in/<Pad>`, the built-in drum kit's folders).
pub const BUILT_IN: &str = "Built-in";

/// The relative path of an `@R@Built-in/…` path (either separator): our own content, which code provides, never
/// a file. `None` for any other path.
pub fn built_in(path: &ContentPath) -> Option<&str> {
    match path {
        ContentPath::Shorthand {
            root: ContentRoot::App,
            relative,
        } if relative.split(['/', '\\']).next() == Some(BUILT_IN) => Some(relative),
        _ => None,
    }
}

/// The file system ignores case (macOS, Windows): the content folder is matched without it.
pub(crate) const FOLD_CASE: bool = cfg!(any(windows, target_os = "macos"));

/// `path` is `dir` or below it, component by component; with `fold`, ignoring case.
pub(crate) fn inside(path: &Path, dir: &Path, fold: bool) -> bool {
    if !fold {
        return path.starts_with(dir);
    }
    let lower = |p: &Path| -> Vec<String> {
        (p.components())
            .map(|c| c.as_os_str().to_string_lossy().to_lowercase())
            .collect()
    };
    lower(path).starts_with(&lower(dir))
}

/// Where `path` really is: its nearest existing folder (or itself) with every link followed, then the rest as written.
pub(super) fn real(path: &Path) -> Option<PathBuf> {
    path.ancestors()
        .find_map(|a| Some(a.canonicalize().ok()?.join(path.strip_prefix(a).ok()?)))
}

/// `path` is in the content folder `content`, or would be once made: as written, or where it really is ([`real`]),
/// without case where the file system ignores it ([`FOLD_CASE`]). The guard on every write near the user's files.
pub(crate) fn in_content_folder(content: &Path, path: &Path) -> bool {
    inside(path, content, FOLD_CASE)
        || real(path)
            .zip(real(content))
            .is_some_and(|(path, content)| inside(&path, &content, FOLD_CASE))
}

/// The folder `relative` below `root`, when every folder on the way is a folder and no link (`root` itself may be
/// one: the user may keep the tree elsewhere).
pub(crate) fn real_folder(root: &Path, relative: &str) -> Option<PathBuf> {
    let mut dir = root.to_owned();
    for part in Path::new(relative).components() {
        let Component::Normal(name) = part else {
            return None;
        };
        dir.push(name);
        if !std::fs::symlink_metadata(&dir).is_ok_and(|m| m.is_dir()) {
            return None;
        }
    }
    Some(dir)
}

/// The README's text. It names no third-party product (the `APP_NAME` rule).
pub fn readme() -> String {
    let folders: String = TOP_FOLDERS.iter().map(|f| format!("  {f}/\n")).collect();
    format!(
        "Original Content\n\
         ================\n\
         \n\
         If you own the original app, you may drop its content tree into this folder for your personal use.\n\
         {app} reads the files here in place. It never copies, uploads or redistributes them: keep them out of\n\
         anything you share.\n\
         \n\
         Or drop your own copy of the app's .obb or .apk file here: {app} unpacks it into this folder once, when it\n\
         starts or SETTINGS opens, and never over a file already here.\n\
         \n\
         Keep the original top folders and the relative paths of the files in them:\n\
         \n\
         {folders}\
         \n\
         The icon sheet, {sheet}, goes right in this folder; its trademark icons are never shown.\n\
         \n\
         Songs and templates that name original content find it here, and the pickers list it next to your own\n\
         files, read-only. Restart {app} after adding or changing GMSynth waveshapes or the icon sheet.\n",
        app = crate::APP_NAME,
        sheet = crate::icons::SHEET_FILE,
    )
}

impl ProjectFiles {
    /// The content folder, the `@R@` root: [`CONTENT_FOLDER`] in the user folder, on disk. `None` on the web and in
    /// in-memory files, and without a user folder.
    pub fn content_dir(&self) -> Option<PathBuf> {
        (!self.copies).then_some(())?;
        Some(self.user.as_ref()?.join(CONTENT_FOLDER))
    }

    /// `path` is in the content folder, or would be once made ([`in_content_folder`]): a link in the user folder
    /// into it, or another case where the file system ignores case, counts. Never so on the web.
    pub fn in_content(&self, path: &Path) -> bool {
        self.content_dir()
            .is_some_and(|dir| in_content_folder(&dir, path))
    }

    /// The content folder's sub-folder `relative` (`Drum Templates`, `Synth Presets2/GMSynth`), when it is a folder
    /// there and no link.
    pub fn content_folder(&self, relative: &str) -> Option<PathBuf> {
        real_folder(&self.content_dir()?, relative)
    }

    /// Makes the content folder and its [`README_FILE`] where they are missing; a README the user changed stays.
    /// `None` without a content folder (the web).
    pub fn make_content_folder(&self) -> Option<io::Result<PathBuf>> {
        let dir = self.content_dir()?;
        let made = std::fs::create_dir_all(&dir).and_then(|()| {
            match std::fs::File::create_new(dir.join(README_FILE)) {
                Ok(mut file) => io::Write::write_all(&mut file, readme().as_bytes()),
                Err(e) if e.kind() == io::ErrorKind::AlreadyExists => Ok(()),
                Err(e) => Err(e),
            }
        });
        Some(made.map(|()| dir))
    }

    /// The folders the shortcut files are in: the content folder's `Keyboard Shortcuts`, when
    /// it is a folder there and no link, and the user folder on disk (not the empty path of the web and of in-memory
    /// files, which would read `keymap.json` from wherever the app runs).
    pub(crate) fn keymap_dirs(&self) -> (Option<PathBuf>, Option<&Path>) {
        let user = self.user.as_deref().filter(|_| !self.copies);
        (self.content_folder(KEYBOARD_SHORTCUTS), user)
    }

    /// Merges the content folder's shortcut files and the user's `keymap.json` into the app's keymap
    /// ([`crate::keymap`]). Once, at start: the keymap is read by action ID from then on.
    pub fn install_original_keymap(&self) {
        let (shortcuts, user) = self.keymap_dirs();
        crate::keymap::install(shortcuts.as_deref(), user);
    }

    /// Makes the content folder's icon sheet (`<content folder>/wrap.png`) `ctx`'s icon atlas, when it is there and
    /// decodes as one ([`crate::icons::install_sheet`]). Returns whether it did; otherwise our drawn set stays.
    pub fn install_original_icons(&self, ctx: &eframe::egui::Context) -> bool {
        let Some(path) = self
            .content_dir()
            .map(|dir| dir.join(crate::icons::SHEET_FILE))
        else {
            return false;
        };
        let regular = std::fs::symlink_metadata(&path).is_ok_and(|m| m.is_file());
        let png = regular.then(|| self.read_user_file(&path).ok()).flatten();
        png.is_some_and(|png| crate::icons::install_sheet(ctx, &png))
    }

    /// The file `path` names: `@L@` in the user folder, `@R@` in the content folder, `@T@`
    /// in the system temp folder (desktop only). `None` for a plain path (`PTH1` and the other sample fields hold
    /// shorthands only, and a song must never read an arbitrary file), for a relative path that could leave its root
    /// (absolute, a drive, a `..`, a leading `./`, or empty; a `.` further in is dropped, as `Path::components`
    /// does), for an `@R@` file that is missing or that a link puts outside the content folder, and for our own
    /// `@R@Built-in/…` content ([`built_in`]), which is no file.
    pub fn resolve(&self, path: &ContentPath) -> Option<PathBuf> {
        resolve_in(self.user.as_deref(), self.content_dir().as_deref(), path)
    }

    /// The `@L@` shorthand of the file `path` in the user folder (`/`-separated), wherever it is below it. `None` for a
    /// file outside it.
    pub fn user_shorthand(&self, path: &Path) -> Option<ContentPath> {
        let relative = path.strip_prefix(self.user.as_ref()?).ok()?;
        let parts: Vec<_> = relative.iter().map(|p| p.to_string_lossy()).collect();
        Some(ContentPath::Shorthand {
            root: ContentRoot::User,
            relative: parts.join("/"),
        })
    }

    /// Names `module` for the file `path` it was loaded from ([`ModuleInstance::name_preset`]), from the root
    /// [`Self::shorthand`] names the file by (`Self::root_of`): the user folder, the content folder or our factory's
    /// root. The category and preset path are the folder and the path in that root (`Synth Presets2/GMSynth/Bass`,
    /// `DirectWave Samples/Guitar`, `Presets/GMSynth/Bass`, `My Presets/DW Sampler`), and `PRST`'s source path is the
    /// same relative path: a song file never carries where a folder is on this machine. (The format also allows an
    /// absolute path or one relative to the content root; as the web's copies, in the empty user folder,
    /// have always been relative.) A file outside every root has no shorthand and is named by its own path, which no
    /// picker lists. The one namer of a picked file, for every loader.
    pub fn name_module(&self, module: &mut ModuleInstance, path: &Path) {
        let base = self.root_of(path).map(|(_, base)| base);
        module.name_preset(path, base.as_deref().unwrap_or(Path::new("")));
        if base.is_some() {
            module.file.source_path = module.file.preset_path.clone();
        }
    }

    /// The shorthand of the file `path` (`/`-separated): `@R@Built-in/…` for one of our own factory files
    /// ([`crate::content::content_path`]), `@R@` in the content folder, else `@L@` in the user folder. `None` for a
    /// file outside them all, which a shorthand field cannot hold.
    pub fn shorthand(&self, path: &Path) -> Option<ContentPath> {
        self.root_of(path).map(|(shorthand, _)| shorthand)
    }

    /// [`Self::shorthand`] with the folder it strips from `path`: the factory's root, the content folder or the user
    /// folder. What a pick is named from ([`Self::name_module`]).
    fn root_of(&self, path: &Path) -> Option<(ContentPath, PathBuf)> {
        if crate::content::is_factory(path) {
            let shorthand = crate::content::content_path(path)?;
            return Some((shorthand, PathBuf::from(crate::content::ROOT)));
        }
        let roots = [
            (ContentRoot::App, self.content_dir()),
            (ContentRoot::User, self.user.clone()),
        ];
        roots.into_iter().find_map(|(root, dir)| {
            let dir = dir?;
            let relative = path.strip_prefix(&dir).ok()?;
            let parts: Vec<_> = relative.iter().map(|p| p.to_string_lossy()).collect();
            let shorthand = ContentPath::Shorthand {
                root,
                relative: parts.join("/"),
            };
            (!parts.is_empty()).then_some((shorthand, dir))
        })
    }
}

/// [`ProjectFiles::resolve`] for the roots `user` and `content` (the content folder; none on the web): all it needs of
/// the files, so that a worker thread can own them ([`crate::sample_cache::disk_reader`]).
pub(crate) fn resolve_in(
    user: Option<&Path>,
    content: Option<&Path>,
    path: &ContentPath,
) -> Option<PathBuf> {
    let ContentPath::Shorthand { root, relative } = path else {
        return None;
    };
    if built_in(path).is_some() {
        return None;
    }
    let dir = match root {
        ContentRoot::User => user?.to_owned(),
        ContentRoot::App => content?.to_owned(),
        ContentRoot::Temp if !cfg!(target_arch = "wasm32") => std::env::temp_dir(),
        ContentRoot::Temp => return None,
    };
    let online = relative.strip_prefix("Online Content/");
    let relative = Path::new(relative);
    if !ProjectFiles::stays_inside(relative) {
        return None;
    }
    let path = dir.join(relative);
    // An EXP preset's `@L@Online Content/<rest>` missing from the user folder is the content folder's
    // `DirectWave Samples EXP/<rest>`, where the content keeps the EXP products (Essential Pianos EXP's zones).
    if let (ContentRoot::User, Some(rest)) = (root, online)
        && !path.exists()
    {
        let exp = ContentPath::Shorthand {
            root: ContentRoot::App,
            relative: format!("DirectWave Samples EXP/{rest}"),
        };
        return resolve_in(user, content, &exp).or(Some(path));
    }
    // `@R@`, or an `@L@` path into the content folder: where the file really is must be in the content folder,
    // every link followed. The path stays as written, as the pickers list it.
    let content = match root {
        ContentRoot::App => dir,
        _ if content.is_some_and(|dir| in_content_folder(dir, &path)) => content?.to_owned(),
        _ => return Some(path),
    };
    // ponytail: checked, then read (a link swapped in between is followed); open the canonical path if that matters.
    let real = path.canonicalize().ok()?;
    inside(&real, &content.canonicalize().ok()?, FOLD_CASE).then_some(path)
}

#[cfg(all(test, not(target_arch = "wasm32")))]
pub(crate) mod tests {
    use super::*;
    use crate::project_files::mem::MemFiles;

    /// A temp folder, removed when dropped (and before use, from a run that died).
    pub(crate) struct Temp(pub PathBuf);

    impl Temp {
        pub(crate) fn new(name: &str) -> Self {
            let dir =
                std::env::temp_dir().join(format!("honeydew-byo-{name}-{}", std::process::id()));
            let _ = std::fs::remove_dir_all(&dir);
            std::fs::create_dir_all(&dir).unwrap();
            Self(dir)
        }
    }

    impl Drop for Temp {
        fn drop(&mut self) {
            let _ = std::fs::remove_dir_all(&self.0);
        }
    }

    /// Writes `bytes` at `path`, making its folders.
    pub(crate) fn put(path: &Path, bytes: &[u8]) {
        std::fs::create_dir_all(path.parent().unwrap()).unwrap();
        std::fs::write(path, bytes).unwrap();
    }

    pub(crate) fn app(relative: &str) -> ContentPath {
        ContentPath::Shorthand {
            root: ContentRoot::App,
            relative: relative.into(),
        }
    }

    pub(crate) fn user(relative: &str) -> ContentPath {
        ContentPath::Shorthand {
            root: ContentRoot::User,
            relative: relative.into(),
        }
    }

    #[test]
    fn the_content_folder_is_in_the_user_folder_and_there_is_none_in_memory() {
        let files = ProjectFiles::on_disk(Path::new("/home/u/.local/share/honeydew"));
        assert_eq!(
            files.content_dir(),
            Some(PathBuf::from(
                "/home/u/.local/share/honeydew/Original Content"
            ))
        );
        let web = ProjectFiles::with_provider(Box::new(MemFiles::default()));
        assert_eq!(web.content_dir(), None);
        assert_eq!(web.make_content_folder().map(|r| r.is_ok()), None);
        assert_eq!(web.resolve(&app("Drum Samples/Kick/a.wav")), None);
    }

    #[test]
    fn the_shortcut_files_are_the_content_folders_and_the_user_folders_on_disk() {
        use crate::keymap::{self, tests::fires};
        use eframe::egui::{Key, Modifiers, os::OperatingSystem::Windows};
        let tmp = Temp::new("keymap-dirs");
        let files = ProjectFiles::on_disk(&tmp.0);
        assert_eq!(files.keymap_dirs(), (None, Some(tmp.0.as_path())));
        let bind = |action: &str| {
            format!(r#"{{"modifiers":[],"actions":[{{"key":"ctrl+q","action":"{action}"}}]}}"#)
        };
        let shortcuts = tmp.0.join(CONTENT_FOLDER).join("Keyboard Shortcuts");
        put(
            &shortcuts.join("windows.json"),
            bind("fromContent").as_bytes(),
        );
        put(&tmp.0.join("keymap.json"), bind("fromUser").as_bytes());
        let (dir, user) = files.keymap_dirs();
        assert_eq!(dir.as_deref(), Some(shortcuts.as_path()));
        let (map, log) = keymap::load(Windows, dir.as_deref(), user);
        assert!(log.is_empty(), "{log:?}");
        let ctrl_q = Modifiers::CTRL | Modifiers::COMMAND;
        for action in ["fromContent", "fromUser"] {
            assert!(fires(&map, Windows, action, Key::Q, ctrl_q), "{action}");
        }
        // The web and in-memory files read neither: no content folder, and no user folder on disk.
        let web = ProjectFiles::with_provider(Box::new(MemFiles::default()));
        assert_eq!(web.keymap_dirs(), (None, None));
        // A link in the folder's place is not followed.
        #[cfg(unix)]
        {
            let other = Temp::new("keymap-dirs-link");
            std::fs::rename(&shortcuts, other.0.join("real")).unwrap();
            std::os::unix::fs::symlink(other.0.join("real"), &shortcuts).unwrap();
            assert_eq!(files.keymap_dirs().0, None);
        }
    }

    #[test]
    fn the_folder_is_made_once_with_a_readme_that_names_no_product() {
        let tmp = Temp::new("readme");
        let files = ProjectFiles::on_disk(&tmp.0);
        let dir = files.make_content_folder().unwrap().unwrap();
        assert_eq!(dir, tmp.0.join(CONTENT_FOLDER));
        let readme = std::fs::read_to_string(dir.join(README_FILE)).unwrap();
        for folder in TOP_FOLDERS {
            assert!(readme.contains(&format!("  {folder}/\n")), "{folder}");
        }
        for name in ["FL Studio", "Image-Line", "Image Line", "FLM"] {
            assert!(!readme.contains(name), "{name}");
        }
        assert!(readme.contains("personal use"));
        // A second start keeps what the user did to it.
        std::fs::write(dir.join(README_FILE), b"mine").unwrap();
        std::fs::write(dir.join("Drum Samples"), b"").unwrap();
        files.make_content_folder().unwrap().unwrap();
        assert_eq!(std::fs::read(dir.join(README_FILE)).unwrap(), b"mine");
        // A file in the folder's place is an error, not a panic.
        let blocked = Temp::new("readme-blocked");
        std::fs::write(blocked.0.join(CONTENT_FOLDER), b"").unwrap();
        let files = ProjectFiles::on_disk(&blocked.0);
        assert!(files.make_content_folder().unwrap().is_err());
    }

    /// An EXP preset's zones are `@L@Online Content/<rest>` (Essential
    /// Pianos EXP); the content folder holds them as `DirectWave Samples EXP/<rest>`. The user folder's file comes first.
    #[test]
    fn online_content_paths_fall_back_to_the_content_folders_directwave_samples_exp() {
        let tmp = Temp::new("online-content");
        let files = ProjectFiles::on_disk(&tmp.0);
        let rest = "Essential Pianos EXP/Close Grand/Close Grand 1.wav";
        let exp = tmp.0.join(CONTENT_FOLDER).join("DirectWave Samples EXP");
        put(&exp.join(rest), b"RIFF");
        let online = user(&format!("Online Content/{rest}"));
        assert_eq!(files.resolve(&online), Some(exp.join(rest)));
        let own = tmp.0.join("Online Content").join(rest);
        put(&own, b"RIFF");
        assert_eq!(files.resolve(&online), Some(own.clone()));
        // Missing in both: the path as written, missing.
        let gone = user("Online Content/Essential Pianos EXP/Gone.wav");
        assert_eq!(
            files.resolve(&gone),
            Some(tmp.0.join("Online Content/Essential Pianos EXP/Gone.wav"))
        );
        // Never out of the content folder.
        let up = user("Online Content/../../secret.wav");
        put(&tmp.0.join("secret.wav"), b"RIFF");
        assert_eq!(files.resolve(&up), None);
    }

    #[test]
    fn app_paths_resolve_in_the_content_folder_and_never_out_of_it() {
        let tmp = Temp::new("resolve");
        let files = ProjectFiles::on_disk(&tmp.0);
        let content = tmp.0.join(CONTENT_FOLDER);
        let kick = content.join("Drum Samples/Kick/Big Kick.wav");
        put(&kick, b"RIFF");
        put(&tmp.0.join("My Samples/a.wav"), b"RIFF");
        put(&tmp.0.join("secret.wav"), b"RIFF");
        let resolved = files.resolve(&app("Drum Samples/Kick/Big Kick.wav"));
        // As written, as the pickers list it.
        assert_eq!(resolved, Some(kick.clone()));
        // `\` was made `/` by the reader; a `.` further in is dropped.
        assert!(
            files
                .resolve(&app("Drum Samples/./Kick/Big Kick.wav"))
                .is_some()
        );
        // Missing is missing.
        assert_eq!(files.resolve(&app("Drum Samples/Kick/Gone.wav")), None);
        // `@L@` keeps its root: the user folder.
        assert_eq!(
            files.resolve(&user("My Samples/a.wav")),
            Some(tmp.0.join("My Samples/a.wav"))
        );
        let up = format!("../{CONTENT_FOLDER}/Drum Samples/Kick/Big Kick.wav");
        let absolute = kick.to_string_lossy().into_owned();
        for escape in [
            "../secret.wav",
            "Drum Samples/../../secret.wav",
            "./a.wav",
            "",
            &absolute,
            &up,
        ] {
            assert_eq!(files.resolve(&app(escape)), None, "@R@ {escape:?}");
            assert_eq!(files.resolve(&user(escape)), None, "@L@ {escape:?}");
        }
        assert_eq!(files.resolve(&ContentPath::Plain(absolute.clone())), None);
    }

    #[cfg(unix)]
    #[test]
    fn a_link_in_the_content_folder_never_leads_out_of_it() {
        let tmp = Temp::new("links");
        let files = ProjectFiles::on_disk(&tmp.0);
        let content = tmp.0.join(CONTENT_FOLDER);
        put(&content.join("Drum Samples/Kick/a.wav"), b"RIFF");
        put(&tmp.0.join("outside/secret.wav"), b"RIFF");
        let link = |to: &Path, at: &str| std::os::unix::fs::symlink(to, content.join(at)).unwrap();
        link(&tmp.0.join("outside"), "Out");
        link(&tmp.0.join("outside/secret.wav"), "Drum Samples/secret.wav");
        link(&content.join("Drum Samples/Kick"), "Kicks");
        assert_eq!(files.resolve(&app("Out/secret.wav")), None);
        assert_eq!(files.resolve(&app("Drum Samples/secret.wav")), None);
        // An `@L@` path into the content folder has its links checked the same way.
        let through = |p: &str| files.resolve(&user(&format!("{CONTENT_FOLDER}/{p}")));
        assert_eq!(through("Out/secret.wav"), None);
        assert_eq!(through("Drum Samples/secret.wav"), None);
        assert!(through("Kicks/a.wav").is_some());
        // Every folder below the content folder must be a folder, not a link: the first one too.
        put(&tmp.0.join("outside/GMSynth/a.flmpst"), b"x");
        link(&tmp.0.join("outside"), "Synth Presets2");
        assert_eq!(files.content_folder("Synth Presets2/GMSynth"), None);
        // A link that stays inside is followed.
        assert!(files.resolve(&app("Kicks/a.wav")).is_some());
        // The folder itself may be a link: the user keeps the tree elsewhere.
        let elsewhere = Temp::new("links-elsewhere");
        put(&elsewhere.0.join("tree/Drum Samples/b.wav"), b"RIFF");
        let user = Temp::new("links-user");
        std::os::unix::fs::symlink(elsewhere.0.join("tree"), user.0.join(CONTENT_FOLDER)).unwrap();
        let files = ProjectFiles::on_disk(&user.0);
        assert!(files.resolve(&app("Drum Samples/b.wav")).is_some());
        // But a sub-folder that is a link is no content folder for the pickers.
        let files = ProjectFiles::on_disk(&tmp.0);
        assert_eq!(files.content_folder("Out"), None);
        assert_eq!(files.content_folder("Kicks"), None);
        assert_eq!(
            files.content_folder("Drum Samples"),
            Some(content.join("Drum Samples"))
        );
        assert_eq!(files.content_folder("Gone"), None);
    }

    /// A sine burst as a FLAC stream in a `.wav` name, as the content's samples are.
    pub(crate) fn flac_named_wav() -> Vec<u8> {
        let tone: Vec<f32> = (0..4410).map(|i| 0.5 * (i as f32 * 0.1).sin()).collect();
        honeydew_audio::flac_writer::flac(44_100, 16, &[tone])
    }

    #[test]
    fn a_demo_style_song_that_names_app_content_plays_it_and_misses_it_without() {
        use crate::audio_engine::SongAudioEngine;
        use crate::sample_cache::{SampleCache, SampleKey, reader};
        use crate::song::{ChannelKind, Event, Song};
        use honeydew_audio::device::AudioRenderer;

        let kick = app("Drum Samples/Kick/Gen Kick.wav");
        let mut song = Song {
            tempo_bpm: 120.0,
            ..Song::default()
        };
        song.add_channel(ChannelKind::Drums);
        song.channels[1].drum_kit.as_mut().unwrap().slots[0]
            .file
            .path = Some(kick.clone());
        let hit = Event::Drum {
            pos: 0,
            len_beats: 0.0,
            pad: 0,
            velocity: 255,
            pan: 127,
            value: Event::DEFAULT_PITCH,
            repeat: 0,
        };
        song.channels[1].lanes[0].add_clip(0, 4.0).events.push(hit);
        // Through the file format, as a demo song arrives.
        let song = crate::flm::read(&crate::flm::write(&song).unwrap()).unwrap();
        let path = song.channels[1].drum_kit.as_ref().unwrap().slots[0]
            .file
            .path
            .clone();
        assert_eq!(path, Some(kick.clone()));
        let play = |files: &ProjectFiles| {
            let mut cache = SampleCache::default();
            cache.load(&song, &reader(files));
            let missing: Vec<_> = cache.missing().cloned().collect();
            let mut engine = SongAudioEngine::with_samples(&song, 44_100.0, &cache).unwrap();
            engine.play();
            let mut out = vec![0.0; 4410 * 2];
            engine.render(&mut out, 2);
            (missing, out.iter().map(|s| s * s).sum::<f32>())
        };
        let tmp = Temp::new("demo");
        let files = ProjectFiles::on_disk(&tmp.0);
        let (missing, energy) = play(&files);
        assert_eq!(missing, [SampleKey::File(kick.clone())]);
        assert_eq!(energy, 0.0, "a missing sample is a silent pad");
        put(
            &tmp.0
                .join(CONTENT_FOLDER)
                .join("Drum Samples/Kick/Gen Kick.wav"),
            &flac_named_wav(),
        );
        let (missing, energy) = play(&files);
        assert_eq!(missing, []);
        assert!(energy > 1.0, "{energy}");
    }

    /// A small fake content tree in `content`, every file made here by our own writers: no real content file ever
    /// enters a test. Names start with `Gen` where real ones would be content names.
    pub(crate) fn fake_tree(content: &Path) {
        use crate::song::{ChannelKind, DrumKit, Event, ModuleInstance, Song};
        let at = |p: &str| content.join(p);
        let pcm = |n: usize| {
            let tone: Vec<f32> = (0..n).map(|i| 0.4 * (i as f32 * 0.05).sin()).collect();
            honeydew_audio::wav_writer::float32_wav(44_100, &[tone])
        };
        put(&at("Drum Samples/Kick/Gen Kick.wav"), &flac_named_wav());
        put(&at("Drum Samples/Snare/Gen Snare.wav"), &pcm(2000));
        let mut kit = DrumKit::default_kit();
        kit.slots.truncate(2);
        kit.slots[0].file.path = Some(app("Drum Samples/Kick/Gen Kick.wav"));
        kit.slots[1].file.path = Some(app("Drum Samples/Snare/Gen Snare.wav"));
        put(
            &at("Drum Templates/Gen Kit.tmpl"),
            &crate::flm::write_drumset(&kit).unwrap(),
        );
        // DW programs: one zone naming a sample beside the program, one embedding its PCM.
        let record = |tag: u32, data: &[u8]| {
            [
                &tag.to_le_bytes()[..],
                &(data.len() as u64).to_le_bytes(),
                data,
            ]
            .concat()
        };
        let dwp = |zone: Vec<u8>| {
            let program = record(1, &record(3, &zone));
            [&b"DwPr"[..], &38u32.to_le_bytes(), &program].concat()
        };
        let file_zone = record(0x1F6, b"%ILSharedData%\\Gen Guitar\\Gen Zone.wav");
        put(
            &at("DirectWave Samples/Guitar/Gen Guitar.dwp"),
            &dwp(file_zone),
        );
        let mut format = 4u64.to_le_bytes().to_vec();
        format.extend(
            [
                &1u32.to_le_bytes()[..],
                &16u32.to_le_bytes(),
                &44_100f32.to_le_bytes(),
            ]
            .concat(),
        );
        let pcm16: Vec<u8> = [0i16, 9000, -9000, 0]
            .iter()
            .flat_map(|s| s.to_le_bytes())
            .collect();
        let embedded = dwp([record(0x1F7, &format), record(0x205, &pcm16)].concat());
        put(
            &at("DirectWave Samples/Guitar/Gen Guitar/Gen Zone.wav"),
            &pcm(500),
        );
        put(
            &at("DirectWave Samples EXP/Synth Bass EXP/Gen Bass.dwp"),
            &embedded,
        );
        put(
            &at("Loop Samples/Sliced Drums/Gen Loop.flac"),
            &flac_named_wav(),
        );
        put(&at("SoundFonts/BAS Sine.sf2"), SINE_SF2);
        let preset = |t: u16| crate::flm::write_preset(&ModuleInstance::new(t, 2)).unwrap();
        put(
            &at("Synth Presets2/GMSynth/Bass/Gen Bass.flmpst"),
            &preset(2),
        );
        put(
            &at("Synth Presets2/GMSynth/Bass/Gen Wrong.flmpst"),
            &preset(32),
        );
        let floats = vec![0.5; 74];
        put(
            &at("Synth Presets2/GMSynth/Bass/Gen Old.flgsynth"),
            &crate::project_files::picks::tests::flgsynth("Saw", "Sine", &floats),
        );
        put(
            &at("Synth Presets2/MiniSynth/Keys/Gen Keys.flms"),
            &crate::project_files::picks::tests::flms("Gen Keys", &floats[..43]),
        );
        {
            use crate::project_files::fst::tests::{blob, fst, ints, shape, version};
            let osc = blob(213, &ints(&[3, -12, 50, 32, 6, 24, -100, 96, 0, 0, 0]));
            let custom = shape(r"\Instruments\Shapes\HQ\Gen Shape.wav");
            put(
                &at("Synth Presets2/3xOsc/Bass/Gen.fst"),
                &fst(&[version("3.1.1"), osc, custom]),
            );
        }
        put(&at("Effect Presets/Reverb/Gen Hall.flmpst"), &preset(32));
        let rack = [ModuleInstance::new(2, 2), ModuleInstance::new(32, 3)];
        put(
            &at("Rack Presets/Gen Chain.flmrack"),
            &crate::flm::write_rack(&rack).unwrap(),
        );
        put(&at("3xOsc Shapes/HQ/Gen Shape.wav"), &pcm(1024));
        put(
            &at("GMSynthWaveshapes/Beast.spectra"),
            &crate::project_files::picks::tests::spectra(8),
        );
        let mut song = Song {
            tempo_bpm: 120.0,
            ..Song::default()
        };
        song.add_channel(ChannelKind::Drums);
        song.channels[1].drum_kit = Some(kit);
        let hit = Event::Drum {
            pos: 0,
            len_beats: 0.0,
            pad: 0,
            velocity: 255,
            pan: 127,
            value: Event::DEFAULT_PITCH,
            repeat: 0,
        };
        song.channels[1].lanes[0].add_clip(0, 4.0).events.push(hit);
        let song = crate::flm::write(&song).unwrap();
        put(&at("New Song Templates/Gen Beat.songtmpl"), &song);
        put(&at("New Song Templates/Gen Ambient.songtmpl"), &song);
        put(&at("Songs/Demo Songs/Gen Demo.flm"), &song);
        // A whole set (eight chords): the built-in one's.
        let chords = crate::song::DEFAULT_CHORD_JSON.as_bytes();
        put(&at("Chord Sets/Gen Minor.chords"), chords);
        put(
            &at("Keyboard Shortcuts/windows.json"),
            br#"{ "modifiers": ["ctrl"], "actions": [] }"#,
        );
        put(
            &at(crate::icons::SHEET_FILE),
            &crate::icons::sheet_tests::fake_sheet(),
        );
    }

    /// Our own test SoundFont (one sine patch).
    pub(crate) const SINE_SF2: &[u8] = include_bytes!("../../crates/audio/tests/fixtures/sine.sf2");

    #[test]
    fn a_soundfont_in_the_content_folder_lists_its_patches_and_plays() {
        use crate::flm::sf2d::Sf2d;
        use crate::sample_cache::{SampleCache, reader};
        use crate::song::{ChannelKind, Song, SynthKind};

        let tmp = Temp::new("sf2");
        let files = ProjectFiles::on_disk(&tmp.0);
        let file = tmp.0.join(CONTENT_FOLDER).join("SoundFonts/BAS Sine.sf2");
        put(&file, SINE_SF2);
        let path = app("SoundFonts/BAS Sine.sf2");
        assert_eq!(files.shorthand(&file), Some(path.clone()));
        assert_eq!(
            files.library_path(&path).map(|p| p.canonicalize().unwrap()),
            Some(file.canonicalize().unwrap())
        );
        assert_eq!(files.soundfont(&path).map(|p| p.len()), Ok(1));
        let mut song = Song::default();
        song.add_channel(ChannelKind::Synth(SynthKind::SoundFontPlayer));
        let sf2d = Sf2d {
            path: path.clone(),
            patch: 0,
        };
        song.channels[1].rack.modules[0].file.custom_data = Some(sf2d.write(None).unwrap());
        let mut cache = SampleCache::default();
        cache.load(&song, &reader(&files));
        assert!(cache.soundfont(&path).is_some());
        assert_eq!(cache.missing().count(), 0);
        // `@T@` names no SoundFont (SF2D holds `@L@` / `@R@`).
        let temp = ContentPath::Shorthand {
            root: ContentRoot::Temp,
            relative: "a.sf2".into(),
        };
        assert_eq!(files.library_path(&temp), None);
    }

    #[test]
    fn built_in_content_comes_first_and_a_built_in_folder_never_shadows_it() {
        use crate::sample_cache::{SampleCache, reader};
        use crate::song::{ChannelKind, Song};
        use honeydew_audio::builtin_drums::builtin;

        let tmp = Temp::new("built-in");
        let files = ProjectFiles::on_disk(&tmp.0);
        // A user folder named `Built-in` in the content folder, with a file where the built-in kick's name is.
        let shadow = tmp.0.join(CONTENT_FOLDER).join("Built-in");
        put(&shadow.join("Kick"), &flac_named_wav());
        put(&shadow.join("Mine.wav"), &flac_named_wav());
        for relative in ["Built-in/Kick", "Built-in/Mine.wav", "Built-in"] {
            assert_eq!(built_in(&app(relative)), Some(relative));
            assert_eq!(files.resolve(&app(relative)), None, "{relative}");
            assert_eq!(reader(&files)(&app(relative)), None, "{relative}");
        }
        // Only `@R@`, and only that first folder.
        assert_eq!(built_in(&user("Built-in/Kick")), None);
        assert_eq!(built_in(&app("Built-in Kicks/Kick")), None);
        assert_eq!(built_in(&app("Drum Samples/Built-in/Kick")), None);
        // A pad naming `@R@Built-in/Kick` plays the built-in kick, not the file.
        let mut song = Song::default();
        song.add_channel(ChannelKind::Drums);
        let slots = &mut song.channels[1].drum_kit.as_mut().unwrap().slots;
        slots[0].file.path = Some(app("Built-in/Kick"));
        slots[1].file.path = Some(app("Built-in/Mine.wav"));
        let mut cache = SampleCache::default();
        cache.load(&song, &reader(&files));
        let slots = &song.channels[1].drum_kit.as_ref().unwrap().slots;
        assert_eq!(
            cache.slot(&slots[0]).as_deref(),
            builtin("Built-in/Kick").as_ref()
        );
        // No such built-in one-shot: missing, though the folder holds the file.
        assert_eq!(cache.slot(&slots[1]), None);
    }

    #[test]
    fn a_built_in_path_with_backslashes_is_built_in_too() {
        use crate::sample_cache::reader;
        let tmp = Temp::new("built-in-backslash");
        let files = ProjectFiles::on_disk(&tmp.0);
        let content = tmp.0.join(CONTENT_FOLDER);
        put(&content.join("Built-in/X.dwp"), b"DwPr");
        // Where `\` is no separator, a file can bear the whole name.
        #[cfg(unix)]
        put(&content.join("Built-in\\X.dwp"), b"DwPr");
        for relative in [
            "Built-in\\X.dwp",
            "Built-in\\Loops\\Y.wav",
            "Built-in/X.dwp",
        ] {
            assert_eq!(built_in(&app(relative)), Some(relative));
            assert_eq!(files.resolve(&app(relative)), None, "{relative}");
            assert_eq!(reader(&files)(&app(relative)), None, "{relative}");
        }
        assert_eq!(built_in(&app("Built-in Kicks\\Kick")), None);
    }

    #[test]
    fn the_original_icon_sheet_in_the_content_folder_becomes_the_atlas() {
        use eframe::egui::{Color32, Context, Pos2, Rect, Shape, Vec2};
        let ctx = Context::default();
        let tmp = Temp::new("icons");
        let files = ProjectFiles::on_disk(&tmp.0);
        assert!(!files.install_original_icons(&ctx), "no sheet: our set");
        let sheet = tmp.0.join(CONTENT_FOLDER).join(crate::icons::SHEET_FILE);
        put(&sheet, b"\x89PNG junk");
        assert!(!files.install_original_icons(&ctx), "a bad sheet: our set");
        fake_tree(&tmp.0.join(CONTENT_FOLDER));
        assert!(files.install_original_icons(&ctx));
        let square = Rect::from_min_size(Pos2::ZERO, Vec2::splat(48.0));
        let play = crate::icons::IconId::Play;
        let shapes = crate::icons::painted_shapes(&ctx, play, square, Color32::WHITE);
        assert!(matches!(&shapes[..], [Shape::Mesh(_)]), "{shapes:?}");
        let web = ProjectFiles::with_provider(Box::new(MemFiles::default()));
        assert!(!web.install_original_icons(&Context::default()));
    }

    #[cfg(unix)]
    #[test]
    fn a_link_in_the_user_folder_never_writes_into_the_content_folder() {
        let tmp = Temp::new("write-link");
        let mut files = ProjectFiles::on_disk(&tmp.0);
        let kicks = tmp.0.join(CONTENT_FOLDER).join("Drum Samples/Kick");
        std::fs::create_dir_all(&kicks).unwrap();
        std::fs::create_dir_all(tmp.0.join("My Samples")).unwrap();
        std::os::unix::fs::symlink(&kicks, tmp.0.join("My Samples/Linked")).unwrap();
        for target in ["My Samples/Linked/Gen.wav", "My Samples/Linked/New/Gen.wav"] {
            let err = (files.write_user_file(&tmp.0.join(target), b"RIFF".to_vec())).unwrap_err();
            assert_eq!(err.kind(), std::io::ErrorKind::PermissionDenied, "{target}");
        }
        assert!(
            std::fs::read_dir(&kicks).unwrap().next().is_none(),
            "nothing written"
        );
        assert!(files.in_content(&tmp.0.join("My Samples/Linked/Gen.wav")));
        assert!(!files.in_content(&tmp.0.join("My Samples/Gen.wav")));
    }

    #[test]
    fn the_content_folder_is_matched_without_case_where_the_file_system_ignores_it() {
        let dir = Path::new("/u/Original Content");
        let other_case = Path::new("/u/original content/Drum Samples/a.wav");
        assert!(inside(other_case, dir, true));
        assert!(!inside(other_case, dir, false));
        assert!(!inside(Path::new("/u/Original Contents/a.wav"), dir, true));
        assert!(inside(dir, dir, false));
        assert_eq!(FOLD_CASE, cfg!(any(windows, target_os = "macos")));
    }

    #[cfg(any(windows, target_os = "macos"))]
    #[test]
    fn a_write_into_the_content_folder_in_another_case_is_refused() {
        let tmp = Temp::new("write-case");
        let mut files = ProjectFiles::on_disk(&tmp.0);
        std::fs::create_dir_all(tmp.0.join(CONTENT_FOLDER)).unwrap();
        let path = tmp.0.join("original content").join("Gen.wav");
        let err = files.write_user_file(&path, b"RIFF".to_vec()).unwrap_err();
        assert_eq!(err.kind(), std::io::ErrorKind::PermissionDenied);
        assert!(!tmp.0.join(CONTENT_FOLDER).join("Gen.wav").exists());
    }

    #[test]
    fn an_edited_sample_is_never_written_into_the_content_folder() {
        let tmp = Temp::new("write");
        let mut files = ProjectFiles::on_disk(&tmp.0);
        let inside = tmp
            .0
            .join(CONTENT_FOLDER)
            .join("Drum Samples/Kick/Gen Kick.wav");
        let err = files
            .write_user_file(&inside, b"RIFF".to_vec())
            .unwrap_err();
        assert_eq!(err.kind(), std::io::ErrorKind::PermissionDenied);
        assert!(!inside.exists());
        let mine = tmp.0.join("My Samples/Gen Kick.wav");
        files.write_user_file(&mine, b"RIFF".to_vec()).unwrap();
        assert_eq!(std::fs::read(&mine).unwrap(), b"RIFF");
    }

    #[test]
    fn a_file_in_the_content_folder_has_an_app_shorthand() {
        let files = ProjectFiles::on_disk(Path::new("/u"));
        let at = |p: &str| files.shorthand(Path::new(p));
        assert_eq!(
            at("/u/Original Content/Drum Samples/Kick/a.wav"),
            Some(app("Drum Samples/Kick/a.wav"))
        );
        assert_eq!(at("/u/My Samples/a.wav"), Some(user("My Samples/a.wav")));
        assert_eq!(at("/elsewhere/a.wav"), None);
        assert_eq!(at("/u"), None);
        // On the web the session's copies are in the user folder, the empty path.
        let web = ProjectFiles::with_provider(Box::new(MemFiles::default()));
        assert_eq!(
            web.shorthand(Path::new("My SoundFonts/a.sf2")),
            Some(user("My SoundFonts/a.sf2"))
        );
    }

    #[test]
    fn a_picked_file_is_named_from_the_root_its_shorthand_names() {
        use crate::song::ModuleInstance;
        let files = ProjectFiles::on_disk(Path::new("/u"));
        let named = |p: &str| {
            let mut module = ModuleInstance::new(1, 2);
            files.name_module(&mut module, Path::new(p));
            let f = module.file;
            [
                f.category,
                f.preset_name,
                f.preset_path,
                f.source_path,
                f.provenance,
            ]
        };
        // Content: relative to the content folder throughout, so a song never says where that folder is.
        assert_eq!(
            named("/u/Original Content/DirectWave Samples/Guitar/Gen.dwp"),
            [
                "DirectWave Samples/Guitar",
                "Gen",
                "DirectWave Samples/Guitar/Gen.dwp",
                "DirectWave Samples/Guitar/Gen.dwp",
                "DirectWave Samples/Guitar/Gen"
            ]
        );
        // The user's: relative to the user folder throughout, as the web's copies always are (no absolute path).
        assert_eq!(
            named("/u/My Presets/DW Sampler/Keys.dwp"),
            [
                "My Presets/DW Sampler",
                "Keys",
                "My Presets/DW Sampler/Keys.dwp",
                "My Presets/DW Sampler/Keys.dwp",
                "My Presets/DW Sampler/Keys"
            ]
        );
        // Our own bank: relative to its root (`Presets/…`, not `<built-in>/Presets/…` and not `Built-in/…`).
        assert_eq!(
            named("<built-in>/Presets/GMSynth/Bass/Round.flmpst"),
            [
                "Presets/GMSynth/Bass",
                "Round",
                "Presets/GMSynth/Bass/Round.flmpst",
                "Presets/GMSynth/Bass/Round.flmpst",
                "Presets/GMSynth/Bass/Round"
            ]
        );
        // A file in neither has no shorthand: its own path stands.
        assert_eq!(
            named("/elsewhere/Gen.dwp"),
            [
                "/elsewhere",
                "Gen",
                "/elsewhere/Gen.dwp",
                "/elsewhere/Gen.dwp",
                "/elsewhere/Gen"
            ]
        );
        // The web's copies are in the user folder, the empty path.
        let web = ProjectFiles::with_provider(Box::new(MemFiles::default()));
        let mut module = ModuleInstance::new(1, 2);
        web.name_module(&mut module, Path::new("My Presets/DW Sampler/Keys.dwp"));
        assert_eq!(module.file.category, "My Presets/DW Sampler");
        assert_eq!(module.file.preset_path, "My Presets/DW Sampler/Keys.dwp");
    }
}
