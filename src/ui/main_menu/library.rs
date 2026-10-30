//! Where the SONGS tab finds songs. A [`SongLibrary`] lists a folder, reads a song's bytes and keeps a song the user
//! opened or saved. Its building blocks (make a folder, move, copy, remove) carry the SONGS file actions
//! ([`super::file_ops`]).
//!
//! - Native: [`NativeLibrary`], the `.flm` files and the sub-folders of the songs folder ([`songs_dir`]). It never
//!   follows a link, writes a song whole (a temp file renamed over it) and reads at most
//!   [`MAX_FILE_BYTES`](crate::project_files::MAX_FILE_BYTES).
//! - Web: [`MemoryLibrary`], the songs opened or saved in this session and the folders made, in memory (the browser
//!   has no folder to list). Tests use it as their fake library too.
//!
//! [`RECENTLY_DELETED`] sits beside the root, in the user folder, so nothing saved into the songs folder lands in it.
//! The root's listing adds it as a folder once it exists.
//!
//! **Recently Deleted.** A deleted file is kept 30 days. Once per app start, in the background, [`purge_trash`]
//! (started by the app through [`purge_trash_at_start`]) walks the whole bin, deletes every file last changed more than
//! 30 days ago and removes the empty subfolders. The web's session store has no bin ages, so it has no purge. Deleting
//! a file copies it into the bin at the same path relative to the user folder (the name made unique on a collision),
//! then removes the source, so its age counts from the deletion. Deleting inside the bin, or outside the user folder,
//! is permanent: [`SongLibrary::bin_path`] says where, and [`super::file_ops::delete`] does it. Restore is the mirror
//! ([`SongLibrary::restore_path`], [`super::file_ops::restore`]): an item goes back to its first place, the missing
//! folders made and a taken name numbered.

use std::collections::BTreeMap;
use std::io;
use std::path::{Component, Path, PathBuf};

/// The songs folder's name on disk, and the SONGS tab's root title.
pub const SONGS_FOLDER: &str = "My Songs";

/// The folder deleted files and folders go to.
pub const RECENTLY_DELETED: &str = "Recently Deleted";

/// How many days a deleted file stays in [`RECENTLY_DELETED`].
pub const BIN_DAYS: u64 = 30;

/// The Original Content folder's demo songs (`Songs/Demo Songs`), a read-only folder at the root of SONGS once the user
/// dropped them in, under this name.
pub const DEMO_SONGS: &str = "Demo Songs";
/// Where they are, in the content folder.
#[cfg(not(target_arch = "wasm32"))]
const DEMO_SONGS_PATH: &str = "Songs/Demo Songs";
/// The Original Content folder's song templates, which SONGS `New` lists.
#[cfg(not(target_arch = "wasm32"))]
const TEMPLATES_PATH: &str = "New Song Templates";
#[cfg(not(target_arch = "wasm32"))]
const TEMPLATE_EXT: &str = "songtmpl";
/// The most entries one folder listing holds: a folder is the user's, and may hold anything. The Original Content
/// folder's largest folder holds a few hundred.
pub const MAX_LISTED: usize = 10_000;
/// The most entries one folder listing reads, listed or not: a folder of a million other files is not read whole.
#[cfg(not(target_arch = "wasm32"))]
const MAX_SCANNED: usize = 10 * MAX_LISTED;

/// Characters a song name cannot hold.
pub(crate) const FORBIDDEN: &str = "/\\:*?\"<>|";

/// One row of a folder listing.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct SongEntry {
    /// The file or folder name, as shown.
    pub name: String,
    /// What [`SongLibrary::list`] (for a folder) and [`SongLibrary::read`] (for a song) take.
    pub path: PathBuf,
    pub is_folder: bool,
    /// Larger is more recent: seconds since 1970 on native, the save order on the web. The `Recent` sort uses it.
    pub recency: u64,
}

impl SongEntry {
    /// A project archive, a `.zip` file in any case: what `Unzip` is offered for.
    pub fn is_archive(&self) -> bool {
        !self.is_folder
            && Path::new(&self.name)
                .extension()
                .is_some_and(|e| e.eq_ignore_ascii_case("zip"))
    }
}

pub trait SongLibrary {
    /// The entries of `folder`, or of the root for `None`, in no particular order. A root that does not exist yet is
    /// empty.
    fn list(&self, folder: Option<&Path>) -> io::Result<Vec<SongEntry>>;
    /// The bytes of the song at `path`.
    fn read(&self, path: &Path) -> io::Result<Vec<u8>>;
    /// Keeps a song the user opened or saved in the root, as `name` (`My Song.flm`), replacing one of the same name.
    /// A name that is not a plain, visible `.flm` file name on every desktop is refused (see [`check_name`]).
    fn store(&mut self, name: &str, bytes: &[u8]) -> io::Result<()>;

    // The file actions' building blocks. Each refuses a link and never replaces anything: a `to` that exists fails
    // with `AlreadyExists`, and so does one inside `from`. The defaults are a library without them (a test fake).

    /// [`Self::store`] into `folder`, a folder of the library (not its root). The default keeps no copies there.
    fn store_in(&mut self, _folder: &Path, _name: &str, _bytes: &[u8]) -> io::Result<()> {
        Err(io::ErrorKind::Unsupported.into())
    }

    /// What `list(None)` lists. `None`: there is no library.
    fn root(&self) -> Option<&Path> {
        None
    }
    /// [`RECENTLY_DELETED`], which may not exist yet. `None`: there is none.
    fn trash(&self) -> Option<PathBuf> {
        None
    }
    /// The folders listed at the root that are places, not items: never selected, deleted, renamed or moved, as they
    /// hold what the user keeps in them. [`RECENTLY_DELETED`], and in the songs library `Unzipped songs`
    /// ([`crate::project_files::UNZIPPED_FOLDER`]); in the whole user folder also every fixed folder ([`SONGS_FOLDER`],
    /// the user folders, the Original Content folder). The default has the bin alone.
    fn places(&self) -> Vec<PathBuf> {
        self.trash().into_iter().collect()
    }
    /// Where deleting `path` puts it in [`Self::trash`]: its place relative to the user folder, the folder that holds
    /// the root and the bin. `None`: deleting it is for good, because it is in the bin or outside the user folder.
    fn bin_path(&self, _path: &Path) -> Option<PathBuf> {
        None
    }
    /// Where restoring `path`, which is in [`Self::trash`], puts it back: the inverse of [`Self::bin_path`], so
    /// below the user folder what `path` is below the bin (the `My Songs/` wrapper then leads to the root). A song or
    /// archive right in the bin (an earlier, flat bin's) goes into the root. `None`: it is not in the bin, or is the
    /// bin itself.
    fn restore_path(&self, _path: &Path) -> Option<PathBuf> {
        None
    }
    /// Where `path` really is: every link on the way to it, and `path` itself if it is one, resolved. Fails when that
    /// cannot be told (it is not there). The default is `path` as written (the web has no links).
    fn canonical(&self, path: &Path) -> io::Result<PathBuf> {
        Ok(path.to_owned())
    }
    /// `path` is a link.
    fn is_link(&self, _path: &Path) -> bool {
        false
    }
    /// `path` is a folder of the root named as a row the root lists from beside it ([`row_owns`]): the row owns the
    /// name there, so that [`Self::exists`] counts it as taken. The default has none.
    fn shadowed(&self, _path: &Path) -> bool {
        false
    }
    /// What `folder` holds, files of every type: what [`Self::list`] leaves out goes back with a restored place too.
    fn list_all(&self, folder: &Path) -> io::Result<Vec<SongEntry>> {
        self.list(Some(folder))
    }
    /// Every entry of `folder`, hidden ones and links too, which [`Self::list_all`] leaves out: what `Restore` moves
    /// when it merges the bin's copy of a place into the place, so that nothing is left in the bin. The default is
    /// [`Self::list_all`].
    fn list_raw(&self, folder: &Path) -> io::Result<Vec<SongEntry>> {
        self.list_all(folder)
    }
    /// Moves the link `from` itself, never followed, to `to`, which must be free: a link [`Self::list_raw`] listed.
    fn move_link(&mut self, _from: &Path, _to: &Path) -> io::Result<()> {
        Err(io::ErrorKind::Unsupported.into())
    }
    /// The folder `path` holds nothing at all: no entry, of any type, listed or not (a preset in the songs' bin).
    fn is_empty_folder(&self, path: &Path) -> bool {
        self.list(Some(path)).is_ok_and(|l| l.is_empty())
    }
    /// Something is at `path`, a link included.
    fn exists(&self, _path: &Path) -> bool {
        false
    }
    /// Makes the folder `path` (and its parents).
    fn make_folder(&mut self, _path: &Path) -> io::Result<()> {
        Err(io::ErrorKind::Unsupported.into())
    }
    /// Moves (renames) the file or folder `from` to `to`.
    fn move_to(&mut self, _from: &Path, _to: &Path) -> io::Result<()> {
        Err(io::ErrorKind::Unsupported.into())
    }
    /// Copies the file or the whole folder `from` to `to`.
    fn copy_to(&mut self, _from: &Path, _to: &Path) -> io::Result<()> {
        Err(io::ErrorKind::Unsupported.into())
    }
    /// Removes the file or the whole folder at `path`, for good.
    fn remove(&mut self, _path: &Path) -> io::Result<()> {
        Err(io::ErrorKind::Unsupported.into())
    }
    /// Writes the new file `path`, in a folder that is there, with `bytes` (`Convert to FLAC`).
    fn write_new(&mut self, _path: &Path, _bytes: &[u8]) -> io::Result<()> {
        Err(io::ErrorKind::Unsupported.into())
    }
    /// `path` is read-only: imported content, which the ≡ actions never change. The default has none.
    fn read_only(&self, _path: &Path) -> bool {
        false
    }
    /// The song templates SONGS `New` offers, each named by its stem, in byte order. The default has none.
    fn templates(&self) -> Vec<SongEntry> {
        Vec::new()
    }
}

/// `from` is there and `to` is free and not inside it.
fn check_move(lib: &dyn SongLibrary, from: &Path, to: &Path) -> io::Result<()> {
    if !lib.exists(from) {
        return Err(io::ErrorKind::NotFound.into());
    }
    if lib.exists(to) {
        return Err(io::Error::new(
            io::ErrorKind::AlreadyExists,
            "already exists",
        ));
    }
    if to.starts_with(from) {
        return Err(io::Error::new(
            io::ErrorKind::InvalidInput,
            "a folder cannot go into itself",
        ));
    }
    Ok(())
}

/// `path` is a folder of `root` named as one of `rows`, the folders the root lists from beside it (the bin, `Unzipped
/// songs`, `Demo Songs`), and is not that row. The root lists each such row once: the row owns its name there. A
/// library's `exists` counts the name as taken, so that no action makes such a folder (`Create folder` and `Rename`
/// find it taken, `Paste` numbers it, `Restore` puts nothing into one), and one made before is renamed at the start
/// ([`super::file_ops::rename_shadowed_folders`]); one that could not be is listed, never hidden. `rows` is asked for
/// only for a path right in the root.
fn row_owns(root: Option<&Path>, path: &Path, rows: impl FnOnce() -> Vec<PathBuf>) -> bool {
    path.parent() == root
        && rows()
            .iter()
            .any(|p| p != path && p.file_name() == path.file_name())
}

/// Device names Windows reserves, whatever the extension (`CON.flm` is the console).
const RESERVED: [&str; 22] = [
    "CON", "PRN", "AUX", "NUL", "COM1", "COM2", "COM3", "COM4", "COM5", "COM6", "COM7", "COM8",
    "COM9", "LPT1", "LPT2", "LPT3", "LPT4", "LPT5", "LPT6", "LPT7", "LPT8", "LPT9",
];

/// `name` is a plain `.flm` file name, valid on every desktop (see [`check_folder_name`]).
pub(crate) fn check_name(name: &str) -> io::Result<()> {
    let flm = Path::new(name)
        .extension()
        .is_some_and(|e| e.eq_ignore_ascii_case("flm"));
    check_folder_name(name)?;
    if !flm {
        return Err(not_a_name(name));
    }
    Ok(())
}

/// `name` is a plain folder (or file) name, valid on every desktop: not empty or hidden (a leading `.`), none of
/// `/\:*?"<>|` or control characters, and no reserved Windows device name before its first `.`.
pub(crate) fn check_folder_name(name: &str) -> io::Result<()> {
    let base = name.split('.').next().unwrap_or_default().trim_end();
    if name.is_empty()
        || name.starts_with('.')
        || name.contains(|c: char| FORBIDDEN.contains(c) || c.is_control())
        || RESERVED.iter().any(|r| base.eq_ignore_ascii_case(r))
    {
        return Err(not_a_name(name));
    }
    Ok(())
}

fn not_a_name(name: &str) -> io::Error {
    io::Error::new(
        io::ErrorKind::InvalidInput,
        format!("not a file name: {name:?}"),
    )
}

/// The web's library: the songs opened or saved in this session and the folders made, in memory. Paths are relative
/// to the root, the empty path, so a song at the root is its name. [`RECENTLY_DELETED`] is `/Recently Deleted`,
/// outside the root.
#[derive(Debug, Default)]
pub struct MemoryLibrary {
    /// Every file (its bytes) and folder (`None`) by path, with when it was stored, copied or made: its `recency`.
    entries: BTreeMap<PathBuf, (Option<Vec<u8>>, u64)>,
    /// Counts the stores, copies and folders made.
    clock: u64,
}

impl MemoryLibrary {
    fn tick(&mut self) -> u64 {
        self.clock += 1;
        self.clock
    }

    /// `path` and everything in it, with where each goes when `path` goes to `to`.
    fn tree(&self, path: &Path, to: &Path) -> Vec<(PathBuf, PathBuf)> {
        self.entries
            .keys()
            .filter_map(|p| {
                let rest = p.strip_prefix(path).ok()?;
                let moved = match rest.as_os_str().is_empty() {
                    true => to.to_owned(),
                    false => to.join(rest),
                };
                Some((p.clone(), moved))
            })
            .collect()
    }

    /// Keeps the song `bytes` at `path`, replacing a song there but not a folder. Unlike [`SongLibrary::store`], any
    /// file name (the web's session copies of presets and samples).
    /// A file named `name` (any case) anywhere in it.
    pub(crate) fn find_name(&self, name: &str) -> Option<PathBuf> {
        let named = |p: &&PathBuf| {
            p.file_name()
                .is_some_and(|n| n.to_string_lossy().eq_ignore_ascii_case(name))
        };
        (self.entries.iter())
            .filter(|(_, (bytes, _))| bytes.is_some())
            .map(|(path, _)| path)
            .find(named)
            .cloned()
    }

    pub(crate) fn put(&mut self, path: PathBuf, bytes: &[u8]) -> io::Result<()> {
        if matches!(self.entries.get(&path), Some((None, _))) {
            return Err(io::ErrorKind::AlreadyExists.into());
        }
        let recency = self.tick();
        self.entries.insert(path, (Some(bytes.to_vec()), recency));
        Ok(())
    }

    fn entry(path: &Path, (bytes, recency): &(Option<Vec<u8>>, u64)) -> SongEntry {
        SongEntry {
            name: path
                .file_name()
                .map(|n| n.to_string_lossy().into_owned())
                .unwrap_or_default(),
            path: path.to_owned(),
            is_folder: bytes.is_none(),
            recency: *recency,
        }
    }
}

impl SongLibrary for MemoryLibrary {
    fn list(&self, folder: Option<&Path>) -> io::Result<Vec<SongEntry>> {
        let dir = folder.unwrap_or(Path::new(""));
        let mut list: Vec<_> = self
            .entries
            .iter()
            .filter(|(path, _)| path.parent() == Some(dir))
            .map(|(path, e)| Self::entry(path, e))
            .collect();
        // The folders a file kept deeper down implies (the web's copies are kept by path, `My Presets/MiniSynth/A.flmpst`,
        // with no folder made): the whole user folder lists them.
        for path in self.entries.keys() {
            let mut rest = path
                .strip_prefix(dir)
                .into_iter()
                .flat_map(Path::components);
            if let (Some(Component::Normal(name)), Some(_)) = (rest.next(), rest.next()) {
                let folder = dir.join(name);
                if !list.iter().any(|e| e.path == folder) {
                    list.push(SongEntry {
                        name: name.to_string_lossy().into_owned(),
                        path: folder,
                        is_folder: true,
                        recency: 0,
                    });
                }
            }
        }
        if folder.is_none()
            && let Some(trash) = self.trash()
            && let Some(e) = self.entries.get(&trash)
        {
            list.push(Self::entry(&trash, e));
        }
        Ok(list)
    }

    fn read(&self, path: &Path) -> io::Result<Vec<u8>> {
        match self.entries.get(path) {
            Some((Some(bytes), _)) => Ok(bytes.clone()),
            _ => Err(io::ErrorKind::NotFound.into()),
        }
    }

    fn store(&mut self, name: &str, bytes: &[u8]) -> io::Result<()> {
        check_name(name)?;
        self.put(PathBuf::from(name), bytes)
    }

    fn store_in(&mut self, folder: &Path, name: &str, bytes: &[u8]) -> io::Result<()> {
        check_name(name)?;
        if !matches!(self.entries.get(folder), Some((None, _))) {
            return Err(io::ErrorKind::NotFound.into());
        }
        self.put(folder.join(name), bytes)
    }

    fn root(&self) -> Option<&Path> {
        Some(Path::new(""))
    }

    fn trash(&self) -> Option<PathBuf> {
        Some(Path::new("/").join(RECENTLY_DELETED))
    }

    /// The session's songs are the whole user folder: a path is relative to it, and the bin is the one absolute path.
    fn bin_path(&self, path: &Path) -> Option<PathBuf> {
        let trash = self.trash()?;
        path.is_relative().then(|| trash.join(path))
    }

    fn restore_path(&self, path: &Path) -> Option<PathBuf> {
        let rel = path.strip_prefix(self.trash()?).ok()?;
        (!rel.as_os_str().is_empty()).then(|| rel.to_owned())
    }

    /// A file or folder kept, or a folder a deeper file implies (as [`Self::list`] lists it), or a name the root's bin
    /// owns there ([`SongLibrary::shadowed`]).
    fn exists(&self, path: &Path) -> bool {
        self.entries.contains_key(path)
            || (!path.as_os_str().is_empty() && self.entries.keys().any(|p| p.starts_with(path)))
            || self.shadowed(path)
    }

    /// The root's bin owns its name there ([`row_owns`]).
    fn shadowed(&self, path: &Path) -> bool {
        row_owns(self.root(), path, || self.places())
    }

    fn make_folder(&mut self, path: &Path) -> io::Result<()> {
        if self.exists(path) {
            return Err(io::ErrorKind::AlreadyExists.into());
        }
        let recency = self.tick();
        self.entries.insert(path.to_owned(), (None, recency));
        Ok(())
    }

    fn move_to(&mut self, from: &Path, to: &Path) -> io::Result<()> {
        check_move(self, from, to)?;
        for (old, new) in self.tree(from, to) {
            let entry = self.entries.remove(&old).expect("listed");
            self.entries.insert(new, entry);
        }
        Ok(())
    }

    fn copy_to(&mut self, from: &Path, to: &Path) -> io::Result<()> {
        check_move(self, from, to)?;
        for (old, new) in self.tree(from, to) {
            let bytes = self.entries[&old].0.clone();
            let recency = self.tick();
            self.entries.insert(new, (bytes, recency));
        }
        Ok(())
    }

    fn remove(&mut self, path: &Path) -> io::Result<()> {
        if !self.exists(path) {
            return Err(io::ErrorKind::NotFound.into());
        }
        for (old, _) in self.tree(path, path) {
            self.entries.remove(&old);
        }
        Ok(())
    }

    fn write_new(&mut self, path: &Path, bytes: &[u8]) -> io::Result<()> {
        if self.exists(path) {
            return Err(io::ErrorKind::AlreadyExists.into());
        }
        self.put(path.to_owned(), bytes)
    }
}

/// The songs folder: `My Songs` in the user folder ([`user_dir`]).
#[cfg(not(target_arch = "wasm32"))]
pub fn songs_dir() -> Option<PathBuf> {
    Some(user_dir()?.join(SONGS_FOLDER))
}

/// The user folder, which holds `My Songs` and the other `My …` folders ([`crate::project_files::MY_FOLDERS`]), the
/// Original Content folder, `Unzipped songs` and the bin, where the user finds it:
/// - Linux and other Unixes: `Honeydew` in the XDG documents folder, or `~/Documents/Honeydew`;
/// - macOS: `~/Documents/Honeydew`;
/// - Windows: `Honeydew` in the Documents known folder, or `%USERPROFILE%\Documents\Honeydew`;
/// - Android: the app's external files folder (`Android/data/<package>/files`).
///
/// The first time, what [`app_data_dir`], the user folder before, holds moves here ([`migrate_user_dir`]), unless it
/// cannot: then that folder stays the user folder. [`app_data_dir`] too where there is no documents folder (or no
/// external one on Android). Worked out once per process.
#[cfg(not(target_arch = "wasm32"))]
pub fn user_dir() -> Option<PathBuf> {
    static DIR: std::sync::OnceLock<PathBuf> = std::sync::OnceLock::new();
    // Outside the lock: on Android there is none before the activity starts, which a later call may find.
    let old = app_data_dir()?;
    let dir = DIR.get_or_init(|| match new_user_dir() {
        None => old,
        // Tests never touch the user's folders.
        Some(new) if cfg!(test) => new,
        Some(new) => migrate_user_dir(&old, &new, |from, to| std::fs::rename(from, to)),
    });
    Some(dir.clone())
}

#[cfg(target_os = "android")]
fn new_user_dir() -> Option<PathBuf> {
    crate::android::external_dir().filter(|dir| std::fs::create_dir_all(dir).is_ok())
}

#[cfg(all(not(target_arch = "wasm32"), not(target_os = "android")))]
fn new_user_dir() -> Option<PathBuf> {
    #[cfg(windows)]
    if let Some(documents) = known_documents() {
        return Some(documents.join(crate::APP_NAME));
    }
    // Tests never read the user's files (`user-dirs.dirs`).
    let read = |path: &Path| match cfg!(test) {
        true => None,
        false => std::fs::read_to_string(path).ok(),
    };
    let documents = documents_dir_from(|key| std::env::var_os(key), read)?;
    Some(documents.join(crate::APP_NAME))
}

/// The documents folder, from the environment variables `var` reads and the file `read` reads: `Documents` in the
/// home folder (`%USERPROFILE%` on Windows), but on Linux and other Unixes the XDG documents folder first
/// (`XDG_DOCUMENTS_DIR` in `user-dirs.dirs` in the config folder, unless it is the home folder itself). `None` without
/// an absolute home folder.
#[cfg(all(not(target_arch = "wasm32"), not(target_os = "android")))]
fn documents_dir_from(
    var: impl Fn(&str) -> Option<std::ffi::OsString>,
    read: impl Fn(&Path) -> Option<String>,
) -> Option<PathBuf> {
    let path = |key| var(key).map(PathBuf::from).filter(|p| p.is_absolute());
    let home = path(if cfg!(windows) { "USERPROFILE" } else { "HOME" })?;
    let xdg = || {
        let config = path("XDG_CONFIG_HOME").unwrap_or_else(|| home.join(".config"));
        let dirs = read(&config.join("user-dirs.dirs"))?;
        let line = dirs
            .lines()
            .find_map(|l| l.trim().strip_prefix("XDG_DOCUMENTS_DIR="))?;
        let value = line.trim().trim_matches('"');
        let dir = match value.strip_prefix("$HOME") {
            Some(rest) => home.join(rest.trim_start_matches('/')),
            None => PathBuf::from(value),
        };
        (dir.is_absolute() && dir != home).then_some(dir)
    };
    let xdg = if cfg!(any(windows, target_os = "macos")) {
        None
    } else {
        xdg()
    };
    Some(xdg.unwrap_or_else(|| home.join("Documents")))
}

/// The Documents known folder, wherever the user moved it (OneDrive, another drive).
#[cfg(windows)]
fn known_documents() -> Option<PathBuf> {
    use std::os::windows::ffi::OsStringExt;
    use windows_sys::Win32::System::Com::CoTaskMemFree;
    use windows_sys::Win32::UI::Shell::{FOLDERID_Documents, SHGetKnownFolderPath};
    let mut path = std::ptr::null_mut();
    // SAFETY: on success `path` is a NUL-terminated wide string the shell allocated; it is freed once, read or not.
    unsafe {
        let found =
            SHGetKnownFolderPath(&FOLDERID_Documents, 0, std::ptr::null_mut(), &mut path) == 0;
        let dir = found.then(|| {
            let len = (0..).take_while(|&i| *path.add(i) != 0).count();
            std::ffi::OsString::from_wide(std::slice::from_raw_parts(path, len))
        });
        CoTaskMemFree(path.cast());
        dir.map(PathBuf::from).filter(|dir| dir.is_absolute())
    }
}

/// The installed sound library's folder in [`app_data_dir`] (`content::install`): it stays there, as the app's.
pub const SOUND_LIBRARY_FOLDER: &str = "content";

/// The marker an earlier user folder holds once it stays the user folder ([`migrate_user_dir`]): why it does.
#[cfg(not(target_arch = "wasm32"))]
const STAYS: &str = ".user-folder-stays-here";

/// Moves what the earlier user folder `old` holds into the user folder `new` and returns `new`, or returns `old` where
/// it stays the user folder. Each entry of `old` but the installed sound library ([`SOUND_LIBRARY_FOLDER`]) is renamed
/// into `new` with `rename` (one rename each, nothing copied), and nothing is deleted or written over:
/// - `old` missing or holding nothing to move: `new`;
/// - `old` marked ([`STAYS`]) by an earlier start: `old`;
/// - a name in both: nothing moves, both keep their files, and `old` stays (marked);
/// - a rename fails (another file system, no permission): what moved goes back, and `old` stays (marked, so the next
///   start does not try again).
// ponytail: checked, then renamed: a file made at one of the names in `new` between the check and the rename would be
// replaced (rename's Unix semantics). `renameat2` with `RENAME_NOREPLACE` would close it.
#[cfg(not(target_arch = "wasm32"))]
fn migrate_user_dir(
    old: &Path,
    new: &Path,
    rename: impl Fn(&Path, &Path) -> io::Result<()>,
) -> PathBuf {
    let stays = |why: String| {
        eprintln!(
            "kept the user folder {}, not moved to {}: {why}",
            old.display(),
            new.display()
        );
        if let Err(err) = std::fs::write(old.join(STAYS), &why) {
            eprintln!("{}: {err}", old.join(STAYS).display());
        }
        old.to_owned()
    };
    if old == new {
        return new.to_owned();
    }
    if old.join(STAYS).exists() {
        return old.to_owned();
    }
    let Ok(listed) = std::fs::read_dir(old) else {
        return new.to_owned();
    };
    let names: Vec<_> = (listed.flatten().map(|e| e.file_name()))
        .filter(|name| name != SOUND_LIBRARY_FOLDER)
        .collect();
    if names.is_empty() {
        return new.to_owned();
    }
    if let Some(name) =
        (names.iter()).find(|name| std::fs::symlink_metadata(new.join(name)).is_ok())
    {
        return stays(format!("{} is in both", name.display()));
    }
    let made = !new.exists();
    if let Err(err) = std::fs::create_dir_all(new) {
        return stays(err.to_string());
    }
    for (i, name) in names.iter().enumerate() {
        if let Err(err) = rename(&old.join(name), &new.join(name)) {
            for back in names[..i].iter().rev() {
                if let Err(err) = rename(&new.join(back), &old.join(back)) {
                    eprintln!("{} stays in {}: {err}", back.display(), new.display());
                }
            }
            if made {
                // Only an empty folder goes.
                let _ = std::fs::remove_dir(new);
            }
            return stays(format!("{}: {err}", name.display()));
        }
    }
    eprintln!(
        "moved the user folder {} to {}",
        old.display(),
        new.display()
    );
    new.to_owned()
}

/// The folder eframe keeps app data in (`eframe::storage_dir`'s layout, which needs eframe's `persistence` feature,
/// so it is repeated here): the installed sound library's ([`SOUND_LIBRARY_FOLDER`]), and the user folder before it
/// moved to the documents folder ([`user_dir`]):
/// - Linux and other Unixes: `$XDG_DATA_HOME/honeydew`, or `~/.local/share/honeydew`;
/// - macOS: `~/Library/Application Support/Honeydew`;
/// - Windows: `%APPDATA%\Honeydew\data`;
/// - Android: `Honeydew` in the app's internal data folder.
///
/// The app's folder named after its former name ([`crate::LEGACY_APP_NAME`]) becomes this one the first time
/// ([`crate::migrate_legacy_dir_once`]), before [`user_dir`] moves what it holds. `None` when the home or app-data
/// folder is unknown or not an absolute path.
#[cfg(target_os = "android")]
pub fn app_data_dir() -> Option<PathBuf> {
    Some(crate::android::data_dir()?.join(crate::APP_NAME))
}

#[cfg(all(not(target_arch = "wasm32"), not(target_os = "android")))]
pub fn app_data_dir() -> Option<PathBuf> {
    #[cfg(not(test))]
    let migrate = crate::migrate_legacy_dir_once;
    // Tests never touch the user's folders.
    #[cfg(test)]
    let migrate = |_: &Path, new: &Path| new.to_owned();
    app_data_dir_from(|key| std::env::var_os(key), migrate)
}

/// [`app_data_dir`] with the environment variables `var` reads, the app's folder of its former name becoming the new
/// one through `migrate` (old, new).
#[cfg(all(not(target_arch = "wasm32"), not(target_os = "android")))]
fn app_data_dir_from(
    var: impl Fn(&str) -> Option<std::ffi::OsString>,
    migrate: fn(&Path, &Path) -> PathBuf,
) -> Option<PathBuf> {
    // No relative fallback: an unset, empty or relative folder means there is no app data folder.
    let path = |key| var(key).map(PathBuf::from).filter(|p| p.is_absolute());
    let home = || path("HOME");
    let data = if cfg!(windows) {
        path("APPDATA")
    } else if cfg!(target_os = "macos") {
        home().map(|h| h.join("Library").join("Application Support"))
    } else {
        path("XDG_DATA_HOME").or_else(|| home().map(|h| h.join(".local").join("share")))
    }?;
    // The app's own folder, in lower case on Linux and other Unixes.
    let named = |app: &str| {
        if cfg!(any(windows, target_os = "macos")) {
            data.join(app)
        } else {
            data.join(app.to_lowercase())
        }
    };
    let app = migrate(&named(crate::LEGACY_APP_NAME), &named(crate::APP_NAME));
    Some(if cfg!(windows) { app.join("data") } else { app })
}

/// The extensions the songs library lists: songs, and project archives, which SONGS opens like a picked one and
/// offers `Unzip` for.
#[cfg(not(target_arch = "wasm32"))]
const SONG_EXTS: [&str; 5] = ["flm", "zip", "mid", "midi", "flp"];

/// The desktop's library: the files of some extensions (any case; songs and archives, [`SONG_EXTS`], unless
/// [`NativeLibrary::with_ext`] or [`NativeLibrary::with_exts`] says otherwise) and the folders in a folder on disk, at
/// most [`MAX_LISTED`] of them.
/// Hidden entries (a leading `.`) and links are left out. The Original Content folder beside the root
/// ([`crate::project_files::CONTENT_FOLDER`]) is read-only: its demo songs list at the songs' root as [`DEMO_SONGS`],
/// and nothing in it is written, moved or removed.
#[cfg(not(target_arch = "wasm32"))]
#[derive(Debug)]
pub struct NativeLibrary {
    root: Option<PathBuf>,
    /// The extensions of the files it lists, without the dot.
    exts: Vec<&'static str>,
    /// The root is the user folder itself ([`Self::user_folder`]), not one of its folders: the bin and the content
    /// folder are in it.
    whole: bool,
}

#[cfg(not(target_arch = "wasm32"))]
impl NativeLibrary {
    /// The library in [`songs_dir`]. Without one it is disabled: it lists nothing and stores nothing.
    pub fn new() -> Self {
        Self::songs(songs_dir())
    }

    /// The library in `root`, an absolute path, with [`RECENTLY_DELETED`] beside it.
    pub fn at(root: PathBuf) -> Self {
        Self::songs(Some(root))
    }

    /// The songs library in `root`: [`SONG_EXTS`], one place for [`Self::new`] and [`Self::at`].
    fn songs(root: Option<PathBuf>) -> Self {
        Self {
            root,
            exts: SONG_EXTS.to_vec(),
            whole: false,
        }
    }

    /// The library of the whole user folder `user`, an absolute path (the Instruments picker's `My Files`): its
    /// [`RECENTLY_DELETED`] and imported content folder are in it, as two of its folders.
    pub fn user_folder(user: PathBuf) -> Self {
        Self {
            whole: true,
            ..Self::songs(Some(user))
        }
    }

    /// The user folder: the root's parent, or the root itself ([`Self::user_folder`]).
    fn user(&self) -> Option<&Path> {
        let root = self.root.as_deref()?;
        match self.whole {
            true => Some(root),
            false => root.parent(),
        }
    }

    /// This is the songs library, which lists songs and archives, not the files of other types
    /// ([`Self::with_exts`]): the root lists `Unzipped songs` too.
    fn is_songs(&self) -> bool {
        self.exts == SONG_EXTS
    }

    /// `<user>/Unzipped songs`, beside the root: where archives are extracted ([`crate::project_files`]). Only the
    /// songs library has it.
    fn unzipped(&self) -> Option<PathBuf> {
        let user = self.user()?;
        self.is_songs()
            .then(|| user.join(crate::project_files::UNZIPPED_FOLDER))
    }

    /// Lists the `ext` files (without the dot: `flmpst`) instead of the songs and archives. Storing still takes songs
    /// only.
    pub fn with_ext(self, ext: &'static str) -> Self {
        self.with_exts(&[ext])
    }

    /// [`Self::with_ext`] for the files of any of `exts` (MiniSynth's presets: `flmpst` and `flms`).
    pub fn with_exts(self, exts: &[&'static str]) -> Self {
        Self {
            exts: exts.to_vec(),
            ..self
        }
    }

    /// The folders the root lists from beside it ([`row_owns`]): its places, and the Original Content folder's
    /// [`DEMO_SONGS`] once it is there.
    fn root_rows(&self) -> Vec<PathBuf> {
        let demos = self.content_folder(DEMO_SONGS_PATH);
        self.places().into_iter().chain(demos).collect()
    }

    /// The Original Content folder, beside the root as the bin is.
    fn content(&self) -> Option<PathBuf> {
        let content = crate::project_files::CONTENT_FOLDER;
        Some(self.user()?.join(content))
    }

    /// `path` may be written: it is not in the Original Content folder, nor reaches it through a link or in another
    /// case ([`crate::project_files::in_content_folder`]).
    fn writable(&self, path: &Path) -> io::Result<()> {
        match self.content() {
            Some(content) if crate::project_files::in_content_folder(&content, path) => {
                Err(io::Error::new(
                    io::ErrorKind::PermissionDenied,
                    "original content is read-only",
                ))
            }
            _ => Ok(()),
        }
    }

    /// The content folder's `relative` folder, when every folder on the way is one and no link.
    fn content_folder(&self, relative: &str) -> Option<PathBuf> {
        crate::project_files::real_folder(&self.content()?, relative)
    }
}

#[cfg(not(target_arch = "wasm32"))]
impl Default for NativeLibrary {
    fn default() -> Self {
        Self::new()
    }
}

#[cfg(not(target_arch = "wasm32"))]
impl SongLibrary for NativeLibrary {
    fn list(&self, folder: Option<&Path>) -> io::Result<Vec<SongEntry>> {
        let Some(dir) = folder.or(self.root.as_deref()) else {
            return Ok(Vec::new());
        };
        let mut songs = Vec::new();
        let entries = match std::fs::read_dir(dir) {
            Err(e) if folder.is_none() && e.kind() == io::ErrorKind::NotFound => None,
            result => Some(result?),
        };
        // An entry that cannot be read (removed meanwhile) is skipped, and so is a link. A name that is no UTF-8 shows
        // lossily; its path is the real one. The cap counts what is listed, not what the folder holds; the scan stops
        // after [`MAX_SCANNED`] entries. Only a row kept is asked for its time.
        let scanned = entries.into_iter().flatten().take(MAX_SCANNED).flatten();
        let listed = scanned.filter_map(|entry| {
            let name = entry.file_name().to_string_lossy().into_owned();
            // `DirEntry::file_type` does not follow links, and most systems give it with the entry.
            let kind = entry.file_type().ok()?;
            let is_folder = kind.is_dir();
            let is_song = Path::new(&name)
                .extension()
                .is_some_and(|e| self.exts.iter().any(|x| e.eq_ignore_ascii_case(x)))
                || self.exts.is_empty();
            if name.starts_with('.') || !(is_folder || is_song && kind.is_file()) {
                return None;
            }
            let meta = entry.metadata().ok()?;
            Some(SongEntry {
                name,
                path: entry.path(),
                is_folder,
                recency: recency(&meta),
            })
        });
        songs.extend(listed.take(MAX_LISTED));
        // The whole user folder lists its bin as one of its folders.
        if folder.is_none()
            && !self.whole
            && let Some(trash) = self.trash()
            && let Ok(meta) = std::fs::symlink_metadata(&trash)
            && meta.is_dir()
        {
            songs.push(SongEntry {
                name: RECENTLY_DELETED.into(),
                path: trash,
                is_folder: true,
                recency: recency(&meta),
            });
        }
        if folder.is_none()
            && let Some(unzipped) = self.unzipped()
            && let Ok(meta) = std::fs::symlink_metadata(&unzipped)
            && meta.is_dir()
        {
            songs.push(SongEntry {
                name: crate::project_files::UNZIPPED_FOLDER.into(),
                path: unzipped,
                is_folder: true,
                recency: recency(&meta),
            });
        }
        if folder.is_none()
            && let Some(demos) = self.content_folder(DEMO_SONGS_PATH)
        {
            songs.push(SongEntry {
                name: DEMO_SONGS.into(),
                path: demos,
                is_folder: true,
                recency: 0,
            });
        }
        Ok(songs)
    }

    fn read(&self, path: &Path) -> io::Result<Vec<u8>> {
        use crate::project_files::{MAX_FILE_BYTES, read_file_limited};
        // ponytail: checked, then opened (a link swapped in between is followed); O_NOFOLLOW if that matters.
        if std::fs::symlink_metadata(path)?.is_symlink() {
            return Err(io::Error::new(io::ErrorKind::InvalidInput, "a link"));
        }
        read_file_limited(std::fs::File::open(path)?, MAX_FILE_BYTES).map_err(io::Error::other)
    }

    /// Writes the song whole or not at all ([`write_atomic`](crate::project_files::write_atomic)). A song that is a
    /// link is refused.
    fn store(&mut self, name: &str, bytes: &[u8]) -> io::Result<()> {
        check_name(name)?;
        let root = self.root.as_ref().ok_or(io::ErrorKind::NotFound)?;
        self.writable(root)?;
        std::fs::create_dir_all(root)?;
        let path = root.join(name);
        if std::fs::symlink_metadata(&path).is_ok_and(|m| m.is_symlink()) {
            return Err(io::Error::new(io::ErrorKind::InvalidInput, "a link"));
        }
        crate::project_files::write_atomic(&path, bytes)
    }

    /// [`Self::store`] into the folder `folder`, which must be there and no link (`Restore demo songs`).
    /// As with [`SongLibrary::move_to`] and [`SongLibrary::copy_to`], `folder` is not confined to the root: it must
    /// come from [`SongLibrary::list`] (or be made below the root), never from outside input.
    fn store_in(&mut self, folder: &Path, name: &str, bytes: &[u8]) -> io::Result<()> {
        check_name(name)?;
        self.writable(folder)?;
        if !no_link(folder)?.is_dir() {
            return Err(io::ErrorKind::NotADirectory.into());
        }
        let path = folder.join(name);
        if std::fs::symlink_metadata(&path).is_ok_and(|m| m.is_symlink()) {
            return Err(io::Error::new(io::ErrorKind::InvalidInput, "a link"));
        }
        crate::project_files::write_atomic(&path, bytes)
    }

    fn root(&self) -> Option<&Path> {
        self.root.as_deref()
    }

    fn trash(&self) -> Option<PathBuf> {
        Some(self.user()?.join(RECENTLY_DELETED))
    }

    fn places(&self) -> Vec<PathBuf> {
        let mut places: Vec<_> = self.trash().into_iter().chain(self.unzipped()).collect();
        // The whole user folder holds every fixed folder as its own: deleting one (`My Presets`) could not be restored,
        // and renaming one (`My Samples`) would break the songs' paths into it. What they hold stays the user's.
        if self.whole
            && let Some(user) = self.user()
        {
            use crate::project_files as pf;
            let fixed =
                (pf::MY_FOLDERS.into_iter()).chain([pf::UNZIPPED_FOLDER, pf::CONTENT_FOLDER]);
            places.extend(fixed.map(|f| user.join(f)));
        }
        places
    }

    fn bin_path(&self, path: &Path) -> Option<PathBuf> {
        // The Original Content folder is never deleted, into the bin or for good ([`Self::writable`]).
        self.writable(path).ok()?;
        let user = self.user()?;
        let rel = path.strip_prefix(user).ok()?;
        let outside = rel.as_os_str().is_empty() || rel.starts_with(RECENTLY_DELETED);
        (!outside).then(|| user.join(RECENTLY_DELETED).join(rel))
    }

    fn restore_path(&self, path: &Path) -> Option<PathBuf> {
        let rel = path.strip_prefix(self.trash()?).ok()?;
        let root = self.root.as_deref()?;
        let mut parts = rel.components();
        let first = Path::new(parts.next()?.as_os_str());
        // The bin mirrors the user folder, which every browser deletes from (`My Songs/…`, a preset picker's `My
        // Presets/…`, the whole user folder's `Mine`). Only a song or archive right at the top is what the flat bin of
        // before held, from the songs folder: no browser lists one beside it.
        let song = first
            .extension()
            .is_some_and(|e| SONG_EXTS.iter().any(|x| e.eq_ignore_ascii_case(x)));
        let flat = !self.whole && parts.next().is_none() && song;
        Some(if flat { root } else { self.user()? }.join(rel))
    }

    /// Every file, whatever its type (hidden ones and links aside).
    fn list_all(&self, folder: &Path) -> io::Result<Vec<SongEntry>> {
        let all = Self {
            root: None,
            exts: Vec::new(),
            whole: false,
        };
        all.list(Some(folder))
    }

    /// Every entry, uncapped: a merge leaves none behind. A `folder` that is a link is refused, never listed through.
    fn list_raw(&self, folder: &Path) -> io::Result<Vec<SongEntry>> {
        no_link(folder)?;
        let mut all = Vec::new();
        for entry in std::fs::read_dir(folder)? {
            let entry = entry?;
            // Not following a link: one to a folder is no folder here.
            let meta = entry.metadata()?;
            all.push(SongEntry {
                name: entry.file_name().to_string_lossy().into_owned(),
                path: entry.path(),
                is_folder: meta.is_dir(),
                recency: recency(&meta),
            });
        }
        Ok(all)
    }

    fn move_link(&mut self, from: &Path, to: &Path) -> io::Result<()> {
        wait_for_purge();
        self.writable(to)?;
        if !self.is_link(from) {
            return Err(io::Error::new(io::ErrorKind::InvalidInput, "not a link"));
        }
        check_move(self, from, to)?;
        std::fs::rename(from, to)
    }

    fn canonical(&self, path: &Path) -> io::Result<PathBuf> {
        std::fs::canonicalize(path)
    }

    fn is_link(&self, path: &Path) -> bool {
        std::fs::symlink_metadata(path).is_ok_and(|m| m.is_symlink())
    }

    /// On disk, what [`Self::list`] leaves out (other file types, hidden files, links) counts too.
    fn is_empty_folder(&self, path: &Path) -> bool {
        std::fs::read_dir(path).is_ok_and(|mut entries| entries.next().is_none())
    }

    fn exists(&self, path: &Path) -> bool {
        std::fs::symlink_metadata(path).is_ok() || self.shadowed(path)
    }

    fn shadowed(&self, path: &Path) -> bool {
        row_owns(self.root.as_deref(), path, || self.root_rows())
    }

    fn make_folder(&mut self, path: &Path) -> io::Result<()> {
        wait_for_purge();
        self.writable(path)?;
        if let Some(parent) = path.parent() {
            std::fs::create_dir_all(parent)?;
        }
        std::fs::create_dir(path)
    }

    fn move_to(&mut self, from: &Path, to: &Path) -> io::Result<()> {
        wait_for_purge();
        self.writable(from)?;
        self.writable(to)?;
        no_link(from)?;
        check_move(self, from, to)?;
        not_inside(from, to)?;
        // ponytail: checked, then renamed (a file made at `to` in between is replaced); renameat2(RENAME_NOREPLACE)
        // if that matters.
        std::fs::rename(from, to)
    }

    fn copy_to(&mut self, from: &Path, to: &Path) -> io::Result<()> {
        wait_for_purge();
        self.writable(to)?;
        no_link(from)?;
        check_move(self, from, to)?;
        not_inside(from, to)?;
        copy_tree(&*self, from, to, 0)
    }

    fn read_only(&self, path: &Path) -> bool {
        self.writable(path).is_err()
    }

    fn remove(&mut self, path: &Path) -> io::Result<()> {
        wait_for_purge();
        self.writable(path)?;
        if no_link(path)?.is_dir() {
            std::fs::remove_dir_all(path)
        } else {
            std::fs::remove_file(path)
        }
    }

    /// Whole or not at all ([`write_atomic`](crate::project_files::write_atomic)).
    fn write_new(&mut self, path: &Path, bytes: &[u8]) -> io::Result<()> {
        wait_for_purge();
        self.writable(path)?;
        if self.exists(path) {
            return Err(io::ErrorKind::AlreadyExists.into());
        }
        crate::project_files::write_atomic(path, bytes)
    }

    /// The Original Content folder's `New Song Templates`, by stem, sorted by byte order.
    fn templates(&self) -> Vec<SongEntry> {
        let Some(dir) = self.content_folder(TEMPLATES_PATH) else {
            return Vec::new();
        };
        let lib = Self::at(dir.clone()).with_ext(TEMPLATE_EXT);
        let mut templates: Vec<SongEntry> = (lib.list(Some(&dir)).unwrap_or_default())
            .into_iter()
            .filter(|e| !e.is_folder)
            .filter_map(|mut e| {
                e.name = Path::new(&e.name).file_stem()?.to_str()?.to_owned();
                Some(e)
            })
            .collect();
        templates.sort_by(|a, b| a.name.as_bytes().cmp(b.name.as_bytes()));
        templates
    }
}

/// The once-per-start purge of the bin `trash`: walks it, deletes every file last changed more than [`BIN_DAYS`] days
/// before `now`, and removes the folders left empty, those that were empty before and those this pass emptied (the
/// next start would take them anyway). The bin itself stays. A link is neither followed nor deleted, and nor is a bin
/// that is one.
///
/// Best effort: what cannot be read or removed stays for the next start. The web has no bin to age (its session
/// store keeps no times), so it has no purge.
#[cfg(not(target_arch = "wasm32"))]
pub fn purge_trash(trash: &Path, now: std::time::SystemTime) {
    if std::fs::symlink_metadata(trash).is_ok_and(|m| m.is_dir()) {
        purge_dir(trash, now, 0);
    }
}

/// The start-up purge, while it may run.
#[cfg(not(target_arch = "wasm32"))]
static PURGE: std::sync::Mutex<Option<std::thread::JoinHandle<()>>> = std::sync::Mutex::new(None);

/// [`purge_trash`] as of now, on its own thread (a big bin takes a while): what the app does once at its start. The
/// library's building blocks wait for it ([`wait_for_purge`]).
#[cfg(not(target_arch = "wasm32"))]
pub fn purge_trash_at_start(trash: PathBuf) {
    let purge = std::thread::spawn(move || purge_trash(&trash, std::time::SystemTime::now()));
    *PURGE
        .lock()
        .unwrap_or_else(std::sync::PoisonError::into_inner) = Some(purge);
}

/// Waits for the start-up purge to finish. It reads an entry, then removes it by path, so it would remove what an
/// action put at that path meanwhile (an item restored, and another of its name deleted): the bin's only copy.
// ponytail: an action in the first moments after a start with a big bin waits on the UI thread; disable the SONGS
// actions while the purge runs if that shows.
#[cfg(not(target_arch = "wasm32"))]
fn wait_for_purge() {
    // Held while joining, so that every other caller waits too.
    let mut purge = PURGE
        .lock()
        .unwrap_or_else(std::sync::PoisonError::into_inner);
    if let Some(running) = purge.take() {
        let _ = running.join();
    }
}

/// [`purge_trash`] of `dir`, `depth` folders below the bin. `true`: nothing is left in it.
#[cfg(not(target_arch = "wasm32"))]
fn purge_dir(dir: &Path, now: std::time::SystemTime, depth: usize) -> bool {
    let Ok(entries) = std::fs::read_dir(dir) else {
        return false;
    };
    let mut empty = true;
    for entry in entries.flatten() {
        let path = entry.path();
        // `DirEntry::metadata` does not follow links, so a link is neither a file nor a folder here.
        let Ok(meta) = entry.metadata() else {
            empty = false;
            continue;
        };
        let gone = if meta.is_dir() {
            // ponytail: folders nested deeper than the copy limit are left alone, a bound on the recursion.
            depth < MAX_COPY_DEPTH
                && purge_dir(&path, now, depth + 1)
                && std::fs::remove_dir(&path).is_ok()
        } else {
            meta.is_file() && expired(&meta, now) && std::fs::remove_file(&path).is_ok()
        };
        empty &= gone;
    }
    empty
}

/// `meta`'s file was last changed more than [`BIN_DAYS`] days before `now`: (now − time) / 86 400 s > 30. A time that is
/// unknown or in the future is not old.
#[cfg(not(target_arch = "wasm32"))]
fn expired(meta: &std::fs::Metadata, now: std::time::SystemTime) -> bool {
    let kept = std::time::Duration::from_secs(BIN_DAYS * 86_400);
    meta.modified()
        .ok()
        .and_then(|t| now.duration_since(t).ok())
        .is_some_and(|age| age > kept)
}

/// Seconds since 1970 of the last change, 0 when unknown.
#[cfg(not(target_arch = "wasm32"))]
fn recency(meta: &std::fs::Metadata) -> u64 {
    meta.modified()
        .ok()
        .and_then(|t| t.duration_since(std::time::UNIX_EPOCH).ok())
        .map_or(0, |d| d.as_secs())
}

/// `to` is not inside `from` with every link in their paths resolved: [`check_move`] compares them as written, and a
/// folder reached through a link to `from` (a picked export folder, say) is still inside it. `to`'s folder must exist.
#[cfg(not(target_arch = "wasm32"))]
fn not_inside(from: &Path, to: &Path) -> io::Result<()> {
    let name = to.file_name().ok_or(io::ErrorKind::InvalidInput)?;
    let parent = to.parent().ok_or(io::ErrorKind::InvalidInput)?;
    if parent
        .canonicalize()?
        .join(name)
        .starts_with(from.canonicalize()?)
    {
        return Err(io::Error::new(
            io::ErrorKind::InvalidInput,
            "a folder cannot go into itself",
        ));
    }
    Ok(())
}

/// What is at `path`, refusing a link.
#[cfg(not(target_arch = "wasm32"))]
fn no_link(path: &Path) -> io::Result<std::fs::Metadata> {
    let meta = std::fs::symlink_metadata(path)?;
    if meta.is_symlink() {
        return Err(io::Error::new(io::ErrorKind::InvalidInput, "a link"));
    }
    Ok(meta)
}

/// How deep [`copy_tree`] goes: folders nested up to this many levels below the one copied. Far deeper than any
/// songs folder, and a bound on the recursion: a copy that finds itself inside what it copies (through a gap in
/// [`not_inside`]) fails instead of overflowing the stack.
#[cfg(not(target_arch = "wasm32"))]
const MAX_COPY_DEPTH: usize = 64;

/// Copies `from`, `depth` folders below the one copied, to `to`, which must be free: a file is streamed into a new
/// file, whole or not at all ([`replace_file`]), so it needs no room in memory and has no size limit; a folder is
/// made and filled. The copy is new: it has the time of the copy, which the bin's age counts from (a `fs::copy`
/// would keep the source's on macOS and Windows). Only files and folders are copied: a link is left out and never
/// followed, and so is a FIFO, a socket or a device, which would block the reading.
///
/// Nothing is replaced. [`replace_file`] renames over what is there, so a file whose `to` `lib` finds taken is
/// refused first: on a disk that ignores case (FAT, exFAT, macOS by default) `song.flm` finds `Song.flm` taken, and
/// would otherwise replace it, the caller then removing the source. `AlreadyExists` is the top one's alone, a `to`
/// taken before the copy started, which the caller must not take back; a name taken inside is reported as another
/// error, and the caller cleans up the partial copy.
///
/// A folder past [`MAX_COPY_DEPTH`] fails the copy, leaving what was copied so far.
///
/// [`replace_file`]: crate::project_files::replace_file
#[cfg(not(target_arch = "wasm32"))]
pub(super) fn copy_tree(
    lib: &dyn SongLibrary,
    from: &Path,
    to: &Path,
    depth: usize,
) -> io::Result<()> {
    use crate::project_files::replace_file;
    let meta = no_link(from)?;
    if meta.is_file() {
        if lib.exists(to) {
            return Err(io::ErrorKind::AlreadyExists.into());
        }
        return replace_file(to, |out| {
            io::copy(&mut std::fs::File::open(from)?, out).map(drop)
        });
    }
    if !meta.is_dir() {
        return Err(io::Error::new(
            io::ErrorKind::InvalidInput,
            "not a file or a folder",
        ));
    }
    if depth > MAX_COPY_DEPTH {
        let deep = format!("folders nested more than {MAX_COPY_DEPTH} deep");
        return Err(io::Error::other(deep));
    }
    std::fs::create_dir(to)?;
    for entry in std::fs::read_dir(from)? {
        let entry = entry?;
        // `DirEntry::file_type` does not follow links: a link is neither.
        let kind = entry.file_type()?;
        if kind.is_file() || kind.is_dir() {
            let inner = to.join(entry.file_name());
            copy_tree(lib, &entry.path(), &inner, depth + 1).map_err(|err| match err.kind() {
                io::ErrorKind::AlreadyExists => {
                    io::Error::other(format!("{}: already exists", inner.display()))
                }
                _ => err,
            })?;
        }
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    fn names(entries: &[SongEntry]) -> Vec<(&str, bool)> {
        let mut v: Vec<_> = entries
            .iter()
            .map(|e| (e.name.as_str(), e.is_folder))
            .collect();
        v.sort();
        v
    }

    #[test]
    fn the_memory_library_keeps_stored_songs_most_recent_last() {
        let mut lib = MemoryLibrary::default();
        assert_eq!(lib.list(None).unwrap(), []);
        lib.store("B.flm", b"b").unwrap();
        lib.store("A.flm", b"a").unwrap();
        // Storing again replaces the bytes and makes it the most recent.
        lib.store("B.flm", b"b2").unwrap();
        let list = lib.list(None).unwrap();
        assert_eq!(names(&list), [("A.flm", false), ("B.flm", false)]);
        let recency = |n: &str| list.iter().find(|e| e.name == n).unwrap().recency;
        assert!(recency("B.flm") > recency("A.flm"));
        let b = list.iter().find(|e| e.name == "B.flm").unwrap();
        assert_eq!(lib.read(&b.path).unwrap(), b"b2");
        assert_eq!(
            lib.read(Path::new("C.flm")).unwrap_err().kind(),
            io::ErrorKind::NotFound
        );
        // Flat: a folder has nothing in it.
        assert_eq!(lib.list(Some(Path::new("A.flm"))).unwrap(), []);
    }

    #[test]
    fn names_with_a_path_in_them_are_refused() {
        let mut lib = MemoryLibrary::default();
        for name in [
            "", ".", "..", "../x.flm", "a/b.flm", "a\\b.flm", "a:b.flm", "a?.flm",
        ] {
            let err = lib.store(name, b"x").unwrap_err();
            assert_eq!(err.kind(), io::ErrorKind::InvalidInput, "{name:?}");
        }
        assert_eq!(lib.list(None).unwrap(), []);
    }

    #[cfg(not(target_arch = "wasm32"))]
    #[test]
    fn the_native_library_lists_songs_and_folders_and_stores_into_its_root() {
        let root = std::env::temp_dir().join(format!("honeydew-songs-test-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&root);
        let mut lib = NativeLibrary::at(root.clone());
        // No folder yet: empty, not an error.
        assert_eq!(lib.list(None).unwrap(), []);
        lib.store("One.flm", b"1").unwrap();
        assert!(lib.store("../Out.flm", b"x").is_err());
        std::fs::create_dir(root.join("Demo songs")).unwrap();
        std::fs::create_dir(root.join(".hidden")).unwrap();
        for (name, bytes) in [
            ("Two.FLM", &b"2"[..]),
            ("notes.txt", b"t"),
            (".Hidden.flm", b"h"),
            ("Demo songs/Three.flm", b"3"),
        ] {
            std::fs::write(root.join(name), bytes).unwrap();
        }
        let list = lib.list(None).unwrap();
        assert_eq!(
            names(&list),
            [("Demo songs", true), ("One.flm", false), ("Two.FLM", false)]
        );
        let one = list.iter().find(|e| e.name == "One.flm").unwrap();
        assert_eq!(lib.read(&one.path).unwrap(), b"1");
        assert!(one.recency > 0);
        let demo = list.iter().find(|e| e.is_folder).unwrap();
        let inside = lib.list(Some(&demo.path)).unwrap();
        assert_eq!(names(&inside), [("Three.flm", false)]);
        assert_eq!(lib.read(&inside[0].path).unwrap(), b"3");
        assert!(!root.parent().unwrap().join("Out.flm").exists());
        std::fs::remove_dir_all(&root).unwrap();
    }

    #[cfg(not(target_arch = "wasm32"))]
    #[test]
    fn the_native_library_lists_the_files_of_its_extension_only() {
        let root = temp_root("ext");
        std::fs::create_dir_all(root.join("Sub")).unwrap();
        for name in ["Lead.flmpst", "Pad.FLMPST", "Song.flm", "Chain.flmrack"] {
            std::fs::write(root.join(name), b"x").unwrap();
        }
        let presets = NativeLibrary::at(root.clone()).with_exts(&["flmpst"]);
        assert_eq!(
            names(&presets.list(Some(&root)).unwrap()),
            [("Lead.flmpst", false), ("Pad.FLMPST", false), ("Sub", true)]
        );
        // The songs library lists no preset or rack.
        let songs = NativeLibrary::at(root.clone());
        assert_eq!(
            names(&songs.list(None).unwrap()),
            [("Song.flm", false), ("Sub", true)]
        );
        std::fs::remove_dir_all(&root).unwrap();
    }

    /// SONGS lists project archives beside the songs (and offers `Unzip` for one); no other library does.
    #[cfg(not(target_arch = "wasm32"))]
    #[test]
    fn the_native_songs_library_lists_archives_beside_songs() {
        let tmp = crate::project_files::content::tests::Temp::new("archives");
        let root = tmp.0.clone();
        std::fs::create_dir_all(root.join("Folder.zip")).unwrap();
        for name in [
            "Pack.zip",
            "Big.ZIP",
            "Song.flm",
            ".hidden.zip",
            "zip",
            "notes.txt",
        ] {
            std::fs::write(root.join(name), b"x").unwrap();
        }
        let songs = NativeLibrary::at(root.clone());
        let list = songs.list(None).unwrap();
        assert_eq!(
            names(&list),
            [
                ("Big.ZIP", false),
                ("Folder.zip", true),
                ("Pack.zip", false),
                ("Song.flm", false)
            ]
        );
        // Only a file with the extension is an archive, in any case; a folder named so is a folder.
        let archives: Vec<_> = list.iter().filter(|e| e.is_archive()).collect();
        let mut archives: Vec<_> = archives.iter().map(|e| e.name.as_str()).collect();
        archives.sort();
        assert_eq!(archives, ["Big.ZIP", "Pack.zip"]);
        // The bytes are the archive's own (what `Open` and `Unzip` read).
        let pack = list.iter().find(|e| e.name == "Pack.zip").unwrap();
        assert_eq!(songs.read(&pack.path).unwrap(), b"x");
        // The preset pickers list their own types only.
        let presets = NativeLibrary::at(root.clone()).with_exts(&["flmpst"]);
        assert_eq!(names(&presets.list(None).unwrap()), [("Folder.zip", true)]);
    }

    #[test]
    fn names_a_song_file_cannot_have_are_refused() {
        let mut lib = MemoryLibrary::default();
        for name in [
            // Reserved device names on Windows, in any case and with any extension.
            "CON.flm",
            "nul.flm",
            "Com1.flm",
            "LPT9.FLM",
            "aux .flm", // Control characters.
            "a\nb.flm",
            "a\tb.flm",
            "a\u{7f}.flm", // Not a song file, or hidden.
            "song.txt",
            "song",
            ".flm",
            ".hidden.flm",
        ] {
            let err = lib.store(name, b"x").unwrap_err();
            assert_eq!(err.kind(), io::ErrorKind::InvalidInput, "{name:?}");
        }
        assert_eq!(lib.list(None).unwrap(), []);
        for name in ["Console.flm", "COM10.flm", "Beat.FLM", "a.b.flm"] {
            lib.store(name, b"x").unwrap();
        }
    }

    /// [`MAX_LISTED`] counts the rows listed, not the folder's entries: 24,000 audio files beside five
    /// songs leave the five listed. A name that is no UTF-8 lists too, shown lossily, its path the real one.
    #[cfg(not(target_arch = "wasm32"))]
    #[test]
    fn the_listing_cap_counts_listed_rows_and_a_non_utf8_name_lists() {
        let tmp = crate::project_files::content::tests::Temp::new("fd4-cap");
        let root = tmp.0.join(SONGS_FOLDER);
        std::fs::create_dir_all(&root).unwrap();
        // Made between two lots of other files: past the cap in a listing in either order of making.
        let wavs = |from: usize| {
            for i in from..from + 12_000 {
                std::fs::File::create(root.join(format!("{i}.wav"))).unwrap();
            }
        };
        wavs(0);
        for i in 0..5 {
            std::fs::write(root.join(format!("S{i}.flm")), b"s").unwrap();
        }
        wavs(12_000);
        let songs = NativeLibrary::at(root.clone());
        assert_eq!(songs.list(None).unwrap().len(), 5);
        #[cfg(unix)]
        {
            use std::os::unix::ffi::OsStrExt as _;
            let raw = root.join(std::ffi::OsStr::from_bytes(b"Bad\xff.flm"));
            std::fs::write(&raw, b"b").unwrap();
            let listed = songs.list(None).unwrap();
            let bad = listed.iter().find(|e| e.path == raw).expect("listed");
            assert_eq!(bad.name, "Bad\u{FFFD}.flm");
            assert_eq!(songs.read(&bad.path).unwrap(), b"b");
        }
    }

    #[cfg(not(target_arch = "wasm32"))]
    fn temp_root(test: &str) -> PathBuf {
        let root = std::env::temp_dir().join(format!("honeydew-{test}-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&root);
        root
    }

    /// `Unzipped songs` beside the songs folder is listed at the songs library's root once it exists, as a folder to go
    /// into; a preset picker's library, and a link, never list it.
    #[cfg(not(target_arch = "wasm32"))]
    #[test]
    fn the_songs_library_lists_unzipped_songs_as_a_root_once_it_exists() {
        let tmp = crate::project_files::content::tests::Temp::new("unzipped-root");
        let user = tmp.0.clone();
        let root = user.join(SONGS_FOLDER);
        std::fs::create_dir_all(&root).unwrap();
        std::fs::write(root.join("Song.flm"), b"s").unwrap();
        let songs = NativeLibrary::at(root.clone());
        assert_eq!(names(&songs.list(None).unwrap()), [("Song.flm", false)]);
        let unzipped = user.join("Unzipped songs");
        std::fs::create_dir_all(unzipped.join("Pack/My Samples")).unwrap();
        std::fs::write(unzipped.join("Pack/Beat.flm"), b"b").unwrap();
        std::fs::write(unzipped.join("Pack/My Samples/kick.wav"), b"k").unwrap();
        std::fs::create_dir_all(unzipped.join(".Pack.part")).unwrap();
        let listed = songs.list(None).unwrap();
        assert_eq!(
            names(&listed),
            [("Song.flm", false), ("Unzipped songs", true)]
        );
        assert_eq!(songs.places(), [songs.trash().unwrap(), unzipped.clone()]);
        // Browsable: the extraction's folder, then its songs and the folders its samples are in.
        let pack = songs.list(Some(&unzipped)).unwrap();
        assert_eq!(names(&pack), [("Pack", true)]);
        let inside = songs.list(Some(&pack[0].path)).unwrap();
        assert_eq!(names(&inside), [("Beat.flm", false), ("My Samples", true)]);
        // Not a preset picker's root (it has its own bin and no unzipped songs), and the web's library has none.
        let presets = NativeLibrary::at(root.clone()).with_exts(&["flmpst"]);
        assert_eq!(presets.list(None).unwrap(), []);
        assert_eq!(presets.places(), [presets.trash().unwrap()]);
        assert_eq!(MemoryLibrary::default().places().len(), 1);
        // A link is no folder of ours.
        std::fs::remove_dir_all(&unzipped).unwrap();
        #[cfg(unix)]
        {
            std::os::unix::fs::symlink(&root, &unzipped).unwrap();
            assert_eq!(names(&songs.list(None).unwrap()), [("Song.flm", false)]);
        }
    }

    /// `new()` is the songs library as `at()` is: songs and archives.
    #[cfg(not(target_arch = "wasm32"))]
    #[test]
    fn the_default_songs_library_lists_what_the_one_at_a_folder_does() {
        assert_eq!(
            NativeLibrary::new().exts,
            NativeLibrary::at("/x".into()).exts
        );
        assert_eq!(NativeLibrary::new().exts, SONG_EXTS);
    }

    #[cfg(not(target_arch = "wasm32"))]
    #[test]
    fn the_original_content_is_read_only_and_its_templates_list_in_byte_order() {
        use crate::project_files::CONTENT_FOLDER;
        use crate::ui::main_menu::file_ops;
        let base = temp_root("byo-songs");
        let songs = base.join(SONGS_FOLDER);
        let demos = base.join(CONTENT_FOLDER).join("Songs/Demo Songs");
        let templates = base.join(CONTENT_FOLDER).join(TEMPLATES_PATH);
        std::fs::create_dir_all(&demos).unwrap();
        std::fs::create_dir_all(&songs).unwrap();
        std::fs::create_dir_all(&templates).unwrap();
        let demo = demos.join("Gen Demo.flm");
        std::fs::write(&demo, b"demo").unwrap();
        std::fs::write(songs.join("Mine.flm"), b"mine").unwrap();
        for name in ["b Beat", "B Beat", "a Ambient", ".hidden"] {
            std::fs::write(templates.join(format!("{name}.songtmpl")), b"t").unwrap();
        }
        std::fs::write(templates.join("notes.txt"), b"t").unwrap();
        let mut lib = NativeLibrary::at(songs.clone());
        assert_eq!(
            names(&lib.list(None).unwrap()),
            [(DEMO_SONGS, true), ("Mine.flm", false)]
        );
        assert_eq!(
            names(&lib.list(Some(&demos)).unwrap()),
            [("Gen Demo.flm", false)]
        );
        assert_eq!(lib.read(&demo).unwrap(), b"demo");
        let names_of = |l: &NativeLibrary| {
            l.templates()
                .into_iter()
                .map(|t| t.name)
                .collect::<Vec<_>>()
        };
        assert_eq!(names_of(&lib), ["B Beat", "a Ambient", "b Beat"]);
        // Nothing in it is written, moved or removed; a copy out of it is the user's own.
        let denied =
            |r: io::Result<()>| r.is_err_and(|e| e.kind() == io::ErrorKind::PermissionDenied);
        assert!(denied(lib.move_to(&demo, &songs.join("Moved.flm"))));
        assert!(denied(
            lib.move_to(&songs.join("Mine.flm"), &demos.join("Mine.flm"))
        ));
        assert!(denied(lib.remove(&demo)));
        assert!(denied(lib.remove(&demos)));
        assert!(denied(lib.make_folder(&demos.join("New"))));
        assert!(denied(
            lib.copy_to(&songs.join("Mine.flm"), &demos.join("Mine.flm"))
        ));
        lib.copy_to(&demo, &songs.join("Gen Demo.flm")).unwrap();
        assert_eq!(std::fs::read(songs.join("Gen Demo.flm")).unwrap(), b"demo");
        // `Delete` refuses it, and puts nothing in the bin.
        let entry = SongEntry {
            name: "Gen Demo.flm".into(),
            path: demo.clone(),
            is_folder: false,
            recency: 0,
        };
        assert_eq!(lib.bin_path(&demo), None);
        assert!(file_ops::delete(&mut lib, &[entry], &mut Vec::new()).is_err());
        assert!(demo.exists());
        assert!(!base.join(RECENTLY_DELETED).exists());
        // Read-only, for the ≡ menu (`DEMO_SONGS` and what is in it): never the songs' own.
        assert!(lib.read_only(&demos) && lib.read_only(&demo));
        assert!(!lib.read_only(&songs.join("Mine.flm")));
        // A link anywhere on the way into the content folder hides the demos: `Songs` itself here.
        #[cfg(unix)]
        {
            let elsewhere = base.join("elsewhere");
            std::fs::create_dir_all(elsewhere.join("Demo Songs")).unwrap();
            let songs_dir = base.join(CONTENT_FOLDER).join("Songs");
            std::fs::rename(&songs_dir, base.join("Songs.moved")).unwrap();
            std::os::unix::fs::symlink(&elsewhere, &songs_dir).unwrap();
            assert_eq!(
                names(&lib.list(None).unwrap()),
                [("Gen Demo.flm", false), ("Mine.flm", false)]
            );
        }
        // Without the content folder: no demo folder and no templates.
        let bare = NativeLibrary::at(temp_root("byo-songs-bare").join(SONGS_FOLDER));
        assert!(bare.list(None).unwrap().is_empty());
        assert!(bare.templates().is_empty());
        std::fs::remove_dir_all(&base).unwrap();
    }

    #[cfg(not(target_arch = "wasm32"))]
    #[test]
    fn the_native_store_replaces_a_song_whole_and_leaves_no_temp_file() {
        let root = temp_root("store");
        let mut lib = NativeLibrary::at(root.clone());
        lib.store("Beat.flm", b"old").unwrap();
        // A temp file left by a crash does not stop the next store.
        std::fs::write(root.join(".Beat.flm.tmp"), b"stale").unwrap();
        lib.store("Beat.flm", b"new").unwrap();
        assert_eq!(std::fs::read(root.join("Beat.flm")).unwrap(), b"new");
        let names: Vec<_> = std::fs::read_dir(&root)
            .unwrap()
            .map(|e| e.unwrap().file_name())
            .collect();
        assert_eq!(names, ["Beat.flm"]);
        std::fs::remove_dir_all(&root).unwrap();
    }

    #[cfg(unix)]
    #[test]
    fn the_native_library_never_follows_a_link() {
        use std::os::unix::fs::symlink;
        let root = temp_root("links");
        let outside = temp_root("links-outside");
        std::fs::create_dir_all(&root).unwrap();
        std::fs::create_dir_all(outside.join("Folder")).unwrap();
        std::fs::write(outside.join("Out.flm"), b"out").unwrap();
        std::fs::write(outside.join("Tmp.flm"), b"tmp").unwrap();
        symlink(outside.join("Out.flm"), root.join("Link.flm")).unwrap();
        symlink(outside.join("Folder"), root.join("Linked folder")).unwrap();
        symlink(outside.join("Tmp.flm"), root.join(".New.flm.tmp")).unwrap();
        let mut lib = NativeLibrary::at(root.clone());

        assert_eq!(lib.list(None).unwrap(), []);
        assert!(lib.read(&root.join("Link.flm")).is_err());
        assert!(lib.store("Link.flm", b"x").is_err());
        // The temp file is made new, never opened through a link.
        lib.store("New.flm", b"new").unwrap();
        assert_eq!(std::fs::read(root.join("New.flm")).unwrap(), b"new");
        assert_eq!(std::fs::read(outside.join("Out.flm")).unwrap(), b"out");
        assert_eq!(std::fs::read(outside.join("Tmp.flm")).unwrap(), b"tmp");
        std::fs::remove_dir_all(&root).unwrap();
        std::fs::remove_dir_all(&outside).unwrap();
    }

    /// `store_in`'s ground: a user folder holding `My Songs` with a folder `F`, and a folder `outside` it.
    /// Returns the user folder, the root, `outside`, the library and `F`.
    #[cfg(unix)]
    fn store_in_ground(test: &str) -> (PathBuf, PathBuf, PathBuf, NativeLibrary, PathBuf) {
        let user = temp_root(test);
        let (root, outside) = (user.join(SONGS_FOLDER), user.join("outside"));
        let folder = root.join("F");
        std::fs::create_dir_all(&outside).unwrap();
        std::fs::create_dir_all(&folder).unwrap();
        (user, root.clone(), outside, NativeLibrary::at(root), folder)
    }

    #[cfg(unix)]
    #[test]
    fn store_in_refuses_a_name_that_is_no_plain_song_name() {
        let (user, root, _, mut lib, folder) = store_in_ground("si-names");
        for bad in [
            "../x.flm", "a/b.flm", ".h.flm", "x.txt", "", "..", "a\\b.flm", "CON.flm",
        ] {
            assert!(
                lib.store_in(&folder, bad, b"x").is_err(),
                "{bad:?} accepted"
            );
        }
        assert!(!root.join("x.flm").exists() && !user.join("x.flm").exists());
        std::fs::remove_dir_all(&user).unwrap();
    }

    #[cfg(unix)]
    #[test]
    fn store_in_needs_a_real_folder_there() {
        let (user, root, _, mut lib, _) = store_in_ground("si-folder");
        let missing = lib.store_in(&root.join("none"), "a.flm", b"x");
        assert_eq!(missing.unwrap_err().kind(), io::ErrorKind::NotFound);
        std::fs::write(root.join("file"), b"x").unwrap();
        assert!(lib.store_in(&root.join("file"), "a.flm", b"x").is_err());
        std::fs::remove_dir_all(&user).unwrap();
    }

    #[cfg(unix)]
    #[test]
    fn store_in_never_writes_through_a_link() {
        use std::os::unix::fs::symlink;
        let (user, root, outside, mut lib, folder) = store_in_ground("si-links");
        // A linked folder.
        symlink(&outside, root.join("L")).unwrap();
        assert!(lib.store_in(&root.join("L"), "a.flm", b"x").is_err());
        assert!(
            std::fs::read_dir(&outside).unwrap().next().is_none(),
            "wrote through a linked folder"
        );
        // A linked file.
        std::fs::write(outside.join("t.flm"), b"orig").unwrap();
        symlink(outside.join("t.flm"), folder.join("a.flm")).unwrap();
        assert!(lib.store_in(&folder, "a.flm", b"new").is_err());
        assert_eq!(std::fs::read(outside.join("t.flm")).unwrap(), b"orig");
        std::fs::remove_dir_all(&user).unwrap();
    }

    #[cfg(unix)]
    #[test]
    fn store_in_leaves_the_original_content_alone() {
        let (user, _, _, mut lib, _) = store_in_ground("si-content");
        let content = user
            .join(crate::project_files::CONTENT_FOLDER)
            .join("Songs/Demo Songs");
        std::fs::create_dir_all(&content).unwrap();
        let refused = lib.store_in(&content, "a.flm", b"x").unwrap_err();
        assert_eq!(refused.kind(), io::ErrorKind::PermissionDenied);
        assert!(!content.join("a.flm").exists());
        std::fs::remove_dir_all(&user).unwrap();
    }

    #[cfg(unix)]
    #[test]
    fn store_in_replaces_a_song_whole_and_leaves_no_temp_file() {
        let (user, _, _, mut lib, folder) = store_in_ground("si-whole");
        std::fs::write(folder.join("b.flm"), b"old").unwrap();
        lib.store_in(&folder, "b.flm", b"new").unwrap();
        assert_eq!(std::fs::read(folder.join("b.flm")).unwrap(), b"new");
        let left: Vec<_> = (std::fs::read_dir(&folder).unwrap())
            .map(|e| e.unwrap().file_name())
            .collect();
        assert_eq!(left, ["b.flm"]);
        std::fs::remove_dir_all(&user).unwrap();
    }

    #[cfg(not(target_arch = "wasm32"))]
    #[test]
    fn a_copy_takes_folders_nested_up_to_the_limit_and_fails_past_it() {
        let root = temp_root("deep");
        let mut lib = NativeLibrary::at(root.clone());
        let nested = |from: &Path, levels| (0..levels).fold(from.to_owned(), |p, _| p.join("d"));
        let (f, g) = (root.join("F"), root.join("G"));
        let deep = nested(&f, MAX_COPY_DEPTH);
        std::fs::create_dir_all(&deep).unwrap();
        std::fs::write(deep.join("B.flm"), b"b").unwrap();
        lib.copy_to(&f, &g).unwrap();
        let copied = nested(&g, MAX_COPY_DEPTH).join("B.flm");
        assert_eq!(std::fs::read(copied).unwrap(), b"b");
        // One level more: an error, not a stack overflow.
        std::fs::create_dir(deep.join("d")).unwrap();
        let err = lib.copy_to(&f, &root.join("H")).unwrap_err();
        assert_eq!(err.kind(), io::ErrorKind::Other);
        let deep = format!("folders nested more than {MAX_COPY_DEPTH} deep");
        assert_eq!(err.to_string(), deep);
        std::fs::remove_dir_all(&root).unwrap();
    }

    #[cfg(not(target_arch = "wasm32"))]
    #[test]
    fn a_song_larger_than_the_read_cap_is_refused() {
        let root = temp_root("cap");
        std::fs::create_dir_all(&root).unwrap();
        let path = root.join("Huge.flm");
        // Sparse: it takes no disk space.
        std::fs::File::create(&path)
            .unwrap()
            .set_len(crate::project_files::MAX_FILE_BYTES + 1)
            .unwrap();
        let err = NativeLibrary::at(root.clone()).read(&path).unwrap_err();
        assert!(err.to_string().contains("larger than 512 MiB"), "{err}");
        std::fs::remove_dir_all(&root).unwrap();
    }

    #[cfg(not(target_arch = "wasm32"))]
    #[test]
    fn without_a_songs_folder_the_library_is_disabled() {
        let mut lib = NativeLibrary {
            root: None,
            exts: vec!["flm"],
            whole: false,
        };
        assert_eq!(lib.root(), None);
        assert_eq!(lib.list(None).unwrap(), []);
        assert!(lib.store("Beat.flm", b"x").is_err());
        assert!(
            !Path::new(SONGS_FOLDER).exists(),
            "fell back to the working folder"
        );
    }

    /// A file `name` in `dir` whose last change was `days` days (and `extra` seconds) before `now`.
    #[cfg(not(target_arch = "wasm32"))]
    fn aged(dir: &Path, name: &str, now: std::time::SystemTime, days: u64, extra: u64) {
        std::fs::create_dir_all(dir).unwrap();
        let file = std::fs::File::create(dir.join(name)).unwrap();
        let age = std::time::Duration::from_secs(days * 86_400 + extra);
        file.set_modified(now - age).unwrap();
    }

    #[cfg(not(target_arch = "wasm32"))]
    #[test]
    fn the_purge_deletes_files_older_than_the_limit_and_empty_folders() {
        let base = temp_root("purge");
        let bin = base.join(RECENTLY_DELETED);
        let now = std::time::SystemTime::now();
        // Exactly 30 days is not more than 30 days; a second more is.
        aged(&bin, "edge.flm", now, BIN_DAYS, 0);
        aged(&bin, "over.flm", now, BIN_DAYS, 1);
        aged(&bin, "new.flm", now, 1, 0);
        // A hidden leftover is a file like any other.
        aged(&bin, ".Old.flm.tmp", now, 90, 0);
        // A folder whose files all expire goes with them; one with a fresh file stays, so does its parent.
        aged(&bin.join("My Songs/Gone"), "a.flm", now, 40, 0);
        aged(&bin.join("My Songs/Gone/Deeper"), "b.flm", now, 40, 0);
        aged(&bin.join("My Songs/Kept"), "old.flm", now, 40, 0);
        aged(&bin.join("My Songs/Kept"), "new.flm", now, 2, 0);
        // A folder that was empty before the pass goes too.
        std::fs::create_dir_all(bin.join("Empty/Inner")).unwrap();
        // A file stamped in the future (a clock set back) is not old.
        aged(
            &bin,
            "future.flm",
            now + std::time::Duration::from_secs(86_400 * 90),
            0,
            0,
        );

        purge_trash(&bin, now);

        let left = |dir: &Path| {
            let mut v: Vec<_> = std::fs::read_dir(dir)
                .unwrap()
                .map(|e| e.unwrap().file_name().into_string().unwrap())
                .collect();
            v.sort();
            v
        };
        assert_eq!(
            left(&bin),
            ["My Songs", "edge.flm", "future.flm", "new.flm"]
        );
        assert_eq!(left(&bin.join("My Songs")), ["Kept"]);
        assert_eq!(left(&bin.join("My Songs/Kept")), ["new.flm"]);
        // Much later it is all expired, folders included, but the bin itself stays.
        purge_trash(&bin, now + std::time::Duration::from_secs(86_400 * 400));
        assert!(bin.is_dir() && left(&bin).is_empty());
        // A missing bin is nothing to do.
        purge_trash(&base.join("nope"), now);
        std::fs::remove_dir_all(&base).unwrap();
    }

    /// The bin's age counts from the copy: a copy of an old file has the time of the copy, not the source's.
    #[cfg(not(target_arch = "wasm32"))]
    #[test]
    fn a_streamed_copy_has_the_time_of_the_copy_and_the_bytes_of_the_source() {
        let root = temp_root("copy-time");
        let now = std::time::SystemTime::now();
        // Bigger than one copy buffer, so that it takes more than one read.
        let bytes: Vec<u8> = (0..300_000_u32).map(|i| (i % 251) as u8).collect();
        std::fs::create_dir_all(root.join("F")).unwrap();
        std::fs::write(root.join("F/Old.flm"), &bytes).unwrap();
        let old = std::fs::File::options()
            .write(true)
            .open(root.join("F/Old.flm"))
            .unwrap();
        old.set_modified(now - std::time::Duration::from_secs(86_400 * 90))
            .unwrap();
        let mut lib = NativeLibrary::at(root.clone());
        lib.copy_to(&root.join("F"), &root.join("G")).unwrap();
        let copy = std::fs::metadata(root.join("G/Old.flm")).unwrap();
        let age = std::time::SystemTime::now()
            .duration_since(copy.modified().unwrap())
            .unwrap_or_default();
        assert!(age < std::time::Duration::from_secs(60), "{age:?}");
        assert_eq!(std::fs::read(root.join("G/Old.flm")).unwrap(), bytes);
        // No temp file is left beside it.
        let left: Vec<_> = std::fs::read_dir(root.join("G"))
            .unwrap()
            .map(|e| e.unwrap().file_name())
            .collect();
        assert_eq!(left, ["Old.flm"]);
        std::fs::remove_dir_all(&root).unwrap();
    }

    /// `replace_file` renames over what is there: a copy onto a name that is taken must fail, not replace it (on a
    /// disk that ignores case, the name taken may be the same name in another case).
    #[cfg(not(target_arch = "wasm32"))]
    #[test]
    fn a_copy_onto_a_taken_file_name_fails_and_leaves_what_is_there() {
        let root = temp_root("copy-taken");
        std::fs::create_dir_all(&root).unwrap();
        std::fs::write(root.join("A.flm"), b"new").unwrap();
        std::fs::write(root.join("B.flm"), b"old").unwrap();
        let lib = NativeLibrary::at(root.clone());
        let err = copy_tree(&lib, &root.join("A.flm"), &root.join("B.flm"), 0).unwrap_err();
        assert_eq!(err.kind(), io::ErrorKind::AlreadyExists);
        assert_eq!(std::fs::read(root.join("B.flm")).unwrap(), b"old");
        std::fs::remove_dir_all(&root).unwrap();
    }

    /// A FIFO (or a device) in a folder is not a file to copy: opening it would hang the UI thread.
    #[cfg(unix)]
    #[test]
    fn a_copy_leaves_out_what_is_neither_a_file_nor_a_folder() {
        let root = temp_root("fifo");
        std::fs::create_dir_all(root.join("F")).unwrap();
        std::fs::write(root.join("F/A.flm"), b"a").unwrap();
        let made = std::process::Command::new("mkfifo")
            .arg(root.join("F/pipe"))
            .status();
        if !made.is_ok_and(|s| s.success()) {
            std::fs::remove_dir_all(&root).unwrap();
            return;
        }
        let (from, to) = (root.join("F"), root.join("G"));
        let lib = NativeLibrary::at(root.clone());
        let (tx, rx) = std::sync::mpsc::channel();
        std::thread::spawn(move || tx.send(copy_tree(&lib, &from, &to, 0)));
        let done = rx.recv_timeout(std::time::Duration::from_secs(10));
        assert!(done.expect("the copy hung on the FIFO").is_ok());
        assert_eq!(std::fs::read(root.join("G/A.flm")).unwrap(), b"a");
        assert!(!root.join("G/pipe").exists());
        std::fs::remove_dir_all(&root).unwrap();
    }

    #[cfg(not(target_arch = "wasm32"))]
    #[test]
    fn the_start_up_purge_runs_as_of_now_off_the_calling_thread() {
        let bin = temp_root("purge-start").join(RECENTLY_DELETED);
        let now = std::time::SystemTime::now();
        aged(&bin, "old.flm", now, BIN_DAYS + 1, 0);
        aged(&bin, "new.flm", now, BIN_DAYS - 1, 0);
        for i in 0..500 {
            aged(&bin, &format!("old {i}.flm"), now, BIN_DAYS + 1, 0);
        }
        purge_trash_at_start(bin.clone());
        // The library's building blocks wait for it: it reads an entry, then removes it by path, and would take what
        // an action put there meanwhile.
        NativeLibrary::at(bin.with_file_name(SONGS_FOLDER))
            .make_folder(&bin.join("F"))
            .unwrap();
        assert!(!bin.join("old.flm").exists() && bin.join("new.flm").is_file());
        assert_eq!(std::fs::read_dir(&bin).unwrap().count(), 2);
        std::fs::remove_dir_all(bin.parent().unwrap()).unwrap();
    }

    /// The purge never follows a link: not out of the bin, and not when the bin itself is one.
    #[cfg(unix)]
    #[test]
    fn the_purge_never_follows_a_link() {
        use std::os::unix::fs::symlink;
        let base = temp_root("purge-links");
        let (bin, outside) = (base.join(RECENTLY_DELETED), base.join("outside"));
        let now = std::time::SystemTime::now();
        aged(&outside, "Old.flm", now, 90, 0);
        aged(&outside.join("Sub"), "Old.flm", now, 90, 0);
        std::fs::create_dir_all(&bin).unwrap();
        symlink(outside.join("Old.flm"), bin.join("link.flm")).unwrap();
        symlink(outside.join("Sub"), bin.join("linked folder")).unwrap();
        purge_trash(&bin, now);
        assert!(std::fs::symlink_metadata(bin.join("link.flm")).is_ok());
        assert!(std::fs::symlink_metadata(bin.join("linked folder")).is_ok());
        assert!(outside.join("Old.flm").is_file() && outside.join("Sub/Old.flm").is_file());
        // The bin itself a link to a folder: left alone.
        let linked = base.join("Linked bin");
        symlink(&outside, &linked).unwrap();
        purge_trash(&linked, now);
        assert!(outside.join("Old.flm").is_file() && outside.join("Sub/Old.flm").is_file());
        std::fs::remove_dir_all(&base).unwrap();
    }

    #[cfg(not(target_arch = "wasm32"))]
    #[test]
    fn deleting_puts_a_file_in_the_bin_at_its_place_below_the_user_folder() {
        let user = temp_root("bin-path");
        let lib = NativeLibrary::at(user.join(SONGS_FOLDER));
        let bin = user.join(RECENTLY_DELETED);
        let at = |p: &str| lib.bin_path(&user.join(p));
        assert_eq!(at("My Songs/A.flm"), Some(bin.join("My Songs/A.flm")));
        assert_eq!(at("My Songs/F/A.flm"), Some(bin.join("My Songs/F/A.flm")));
        // Beside the songs folder is still the user folder.
        assert_eq!(at("My Presets/P.flm"), Some(bin.join("My Presets/P.flm")));
        // In the bin, in the user folder's place, and outside it: for good.
        assert_eq!(at("Recently Deleted"), None);
        assert_eq!(at("Recently Deleted/My Songs/A.flm"), None);
        assert_eq!(lib.bin_path(&user), None);
        assert_eq!(
            lib.bin_path(&user.with_file_name("elsewhere").join("A.flm")),
            None
        );
        // A folder of that name below the songs folder is not the bin.
        assert_eq!(
            at("My Songs/Recently Deleted/A.flm"),
            Some(bin.join("My Songs/Recently Deleted/A.flm"))
        );
        // The web's session store is the whole user folder.
        let web = MemoryLibrary::default();
        assert_eq!(
            web.bin_path(Path::new("F/A.flm")),
            Some("/Recently Deleted/F/A.flm".into())
        );
        assert_eq!(web.bin_path(Path::new("/Recently Deleted/A.flm")), None);
    }

    #[cfg(all(unix, not(target_os = "macos")))]
    #[test]
    fn the_app_data_and_documents_folders_need_an_absolute_home() {
        let var = |vars: &'static [(&str, &str)]| {
            move |key: &str| vars.iter().find(|(k, _)| *k == key).map(|(_, v)| v.into())
        };
        let dir = |vars| app_data_dir_from(var(vars), |_, new| new.to_owned());
        assert_eq!(dir(&[]), None);
        assert_eq!(dir(&[("HOME", "relative")]), None);
        assert_eq!(dir(&[("HOME", "")]), None);
        assert_eq!(
            dir(&[("HOME", "/h")]),
            Some(PathBuf::from("/h/.local/share/honeydew"))
        );
        assert_eq!(
            dir(&[("XDG_DATA_HOME", "/d"), ("HOME", "/h")]),
            Some(PathBuf::from("/d/honeydew"))
        );
        assert_eq!(
            dir(&[("XDG_DATA_HOME", "rel"), ("HOME", "/h")]),
            Some(PathBuf::from("/h/.local/share/honeydew"))
        );
        // The documents folder: `user-dirs.dirs`' one (not the home folder itself), else `~/Documents`.
        let docs = |vars, dirs: &'static str| {
            documents_dir_from(var(vars), |p: &Path| {
                (p == Path::new("/h/.config/user-dirs.dirs")).then(|| dirs.to_owned())
            })
        };
        let home = &[("HOME", "/h")];
        assert_eq!(docs(&[], ""), None);
        assert_eq!(docs(home, ""), Some(PathBuf::from("/h/Documents")));
        let dirs = "# comment\nXDG_DOCUMENTS_DIR=\"$HOME/Dokumente\"\n";
        assert_eq!(docs(home, dirs), Some(PathBuf::from("/h/Dokumente")));
        let dirs = "XDG_DOCUMENTS_DIR=\"/data/docs\"";
        assert_eq!(docs(home, dirs), Some(PathBuf::from("/data/docs")));
        let dirs = "XDG_DOCUMENTS_DIR=\"$HOME/\"";
        assert_eq!(docs(home, dirs), Some(PathBuf::from("/h/Documents")));
    }

    #[cfg(all(unix, not(target_os = "macos")))]
    #[test]
    fn the_app_folder_of_the_former_name_becomes_the_app_data_folder() {
        let data = std::env::temp_dir().join(format!("br-songs-migrate-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&data);
        let old = data.join("studio").join(SONGS_FOLDER);
        std::fs::create_dir_all(&old).unwrap();
        std::fs::write(old.join("A.flm"), b"song").unwrap();
        let var = |key: &str| (key == "XDG_DATA_HOME").then(|| data.clone().into_os_string());
        let dir = app_data_dir_from(var, crate::migrate_legacy_dir).unwrap();
        assert_eq!(dir, data.join("honeydew"));
        let song = dir.join(SONGS_FOLDER).join("A.flm");
        assert_eq!(std::fs::read(song).unwrap(), b"song");
        assert!(!data.join("studio").exists());
        let _ = std::fs::remove_dir_all(&data);
    }

    #[cfg(not(target_arch = "wasm32"))]
    #[test]
    fn the_songs_folder_is_my_songs_under_the_user_folder() {
        let dir = songs_dir().expect("HOME is set in tests");
        assert_eq!(dir.parent(), user_dir().as_deref(), "{dir:?}");
        assert!(dir.ends_with(SONGS_FOLDER), "{dir:?}");
        let app = dir.parent().unwrap().to_string_lossy().to_lowercase();
        assert!(app.contains("honeydew"), "{dir:?}");
    }

    /// Every file below `dir`, relative to it, sorted; a folder holding nothing as `<folder>/`.
    #[cfg(not(target_arch = "wasm32"))]
    fn tree(dir: &Path) -> Vec<String> {
        let mut found = Vec::new();
        let mut todo = vec![dir.to_owned()];
        while let Some(at) = todo.pop() {
            let entries: Vec<_> = std::fs::read_dir(&at)
                .into_iter()
                .flatten()
                .flatten()
                .collect();
            if entries.is_empty() && at != dir {
                found.push(format!("{}/", at.strip_prefix(dir).unwrap().display()));
            }
            for entry in entries {
                match entry.file_type().unwrap().is_dir() {
                    true => todo.push(entry.path()),
                    false => found.push(
                        entry
                            .path()
                            .strip_prefix(dir)
                            .unwrap()
                            .display()
                            .to_string(),
                    ),
                }
            }
        }
        found.sort();
        found
    }

    /// The earlier user folder `<base>/old` of the app data folder, holding the installed sound library and the user's
    /// files, and where the user folder goes, `<base>/Documents/Honeydew`.
    #[cfg(not(target_arch = "wasm32"))]
    fn earlier_user_folder(base: &Path) -> (PathBuf, PathBuf) {
        let _ = std::fs::remove_dir_all(base);
        let (old, new) = (
            base.join("old"),
            base.join("Documents").join(crate::APP_NAME),
        );
        for (path, bytes) in [
            ("My Songs/A.flm", &b"song"[..]),
            ("My Samples/Kicks/kick.wav", b"kick"),
            ("Original Content/README.txt", b"readme"),
            ("Unzipped songs/Pack/Pack.flm", b"pack"),
            ("Recently Deleted/My Songs/B.flm", b"gone"),
            ("recent.state", b"state"),
            ("content/index.json", b"library"),
        ] {
            let path = old.join(path);
            std::fs::create_dir_all(path.parent().unwrap()).unwrap();
            std::fs::write(path, bytes).unwrap();
        }
        std::fs::create_dir_all(old.join("My Racks")).unwrap();
        (old, new)
    }

    const USER_FILES: [&str; 7] = [
        "My Racks/",
        "My Samples/Kicks/kick.wav",
        "My Songs/A.flm",
        "Original Content/README.txt",
        "Recently Deleted/My Songs/B.flm",
        "Unzipped songs/Pack/Pack.flm",
        "recent.state",
    ];

    /// The user folder moves out of the app data folder by renames, all of it but the installed sound library, or not
    /// at all: a name in both places, or a rename that fails, keeps the earlier folder in use with every file in it.
    #[cfg(not(target_arch = "wasm32"))]
    #[test]
    fn the_user_folder_moves_whole_to_the_documents_folder_or_stays() {
        let base = std::env::temp_dir().join(format!("honeydew-userdir-{}", std::process::id()));
        let rename = |from: &Path, to: &Path| std::fs::rename(from, to);
        let library = ["content/index.json".to_owned()];

        // Moved: all of it but the sound library, which stays in the app data folder.
        let (old, new) = earlier_user_folder(&base);
        assert_eq!(migrate_user_dir(&old, &new, rename), new);
        assert_eq!(tree(&new), USER_FILES);
        assert_eq!(tree(&old), library);
        assert_eq!(std::fs::read(new.join("My Songs/A.flm")).unwrap(), b"song");
        // Nothing left to move: the next start uses the new folder.
        assert_eq!(
            migrate_user_dir(&old, &new, |_, _| panic!("moved again")),
            new
        );

        // A rename that fails (the third: another file system) puts back what moved, and the earlier folder stays,
        // marked, so later starts keep it without trying again.
        let (old, new) = earlier_user_folder(&base);
        let tries = std::cell::Cell::new(0);
        let failing = |from: &Path, to: &Path| {
            tries.set(tries.get() + 1);
            match tries.get() == 3 {
                true => Err(io::Error::new(
                    io::ErrorKind::CrossesDevices,
                    "another disk",
                )),
                false => std::fs::rename(from, to),
            }
        };
        assert_eq!(migrate_user_dir(&old, &new, failing), old);
        let mut all: Vec<_> = USER_FILES
            .map(String::from)
            .into_iter()
            .chain(library.clone())
            .collect();
        all.push(STAYS.into());
        all.sort();
        assert_eq!(tree(&old), all);
        assert!(!new.exists(), "the folder it made goes, empty");
        assert_eq!(
            migrate_user_dir(&old, &new, |_, _| panic!("tried again")),
            old
        );

        // A name in both places: nothing moves, both keep their files.
        let (old, new) = earlier_user_folder(&base);
        std::fs::create_dir_all(new.join("My Songs")).unwrap();
        std::fs::write(new.join("My Songs/A.flm"), b"other").unwrap();
        assert_eq!(migrate_user_dir(&old, &new, |_, _| panic!("moved")), old);
        assert_eq!(std::fs::read(old.join("My Songs/A.flm")).unwrap(), b"song");
        assert_eq!(tree(&new), ["My Songs/A.flm"]);
        assert_eq!(std::fs::read(new.join("My Songs/A.flm")).unwrap(), b"other");

        let _ = std::fs::remove_dir_all(&base);
    }

    /// The folders a deeper file implies list (the web's `My Files`) and exist, so the file actions take them.
    #[test]
    fn a_folder_a_deeper_file_implies_lists_exists_and_moves_or_goes_whole() {
        let mut lib = MemoryLibrary::default();
        lib.put("My Presets/GMSynth/A.flmpst".into(), b"a").unwrap();
        lib.put("My Presets/GMSynth/B.flmpst".into(), b"b").unwrap();
        let names = |lib: &MemoryLibrary, dir: &str| {
            let list = lib.list(Some(Path::new(dir))).unwrap();
            list.into_iter()
                .map(|e| (e.name, e.is_folder))
                .collect::<Vec<_>>()
        };
        assert_eq!(names(&lib, ""), [("My Presets".to_owned(), true)]);
        assert_eq!(names(&lib, "My Presets"), [("GMSynth".to_owned(), true)]);
        for folder in ["My Presets", "My Presets/GMSynth"] {
            assert!(lib.exists(Path::new(folder)), "{folder}");
        }
        assert!(!lib.exists(Path::new("My")) && !lib.exists(Path::new("")));
        lib.move_to(Path::new("My Presets"), Path::new("Kept"))
            .unwrap();
        assert_eq!(lib.read(Path::new("Kept/GMSynth/B.flmpst")).unwrap(), b"b");
        assert!(!lib.exists(Path::new("My Presets")));
        lib.remove(Path::new("Kept/GMSynth")).unwrap();
        assert_eq!(names(&lib, ""), []);
    }
}
