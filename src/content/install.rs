//! The installed sound library, `<app data folder>/content` ([`dir`], apart from the user folder): on the desktop, the
//! library the app uses ([`super::Bundle::find`]), kept in step with the one that ships with the app.
//!
//! - **Sync.** [`sync`] makes the folder hold what a source library's index lists, and nothing else: `index.json`,
//!   `CREDITS.txt` and every part. The source is a content folder (beside a full download, or one picked) or a sounds
//!   ZIP (`honeydew-<tag>-sounds.zip`, `content/` at its top). Only what is new or changed is copied, by its size and
//!   CRC-32 (a ZIP's central directory's, a folder's computed) against the folder's [`MANIFEST`]; the folder is the
//!   app's, so every other file in it is removed. It never installs an older release over a newer one ([`older`]).
//! - **Safe to stop.** Each file is written to a temp file and renamed into place: the parts first, then the index,
//!   then the strays go, and the manifest is written last. A part is named by its hash, so the index in place always
//!   finds its parts: a sync cut short (a quit, a full disk, a damaged source) leaves a library that works, and the
//!   next one goes on from it.
//! - **Complete** ([`complete`]): the manifest names the index in place, as only a finished sync leaves it.
//! - **When.** A full download's first start, or a new version's, syncs from the library beside the app on a worker
//!   and uses that one meanwhile. SETTINGS `Install sound library…` syncs from a picked ZIP or folder ([`Job`]); the
//!   app is restarted to use it, as [`super::factory`] loads the content once.

use std::collections::{BTreeMap, BTreeSet};
use std::fs::{self, File};
use std::io::{self, Read, Write};
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicBool, AtomicU64, Ordering::Relaxed};
use std::sync::mpsc::{self, Receiver, TryRecvError};
use std::sync::{Arc, Mutex};

use super::bundle::{Index, MAX_INDEX_BYTES, regular};
use crate::project_files::{replace_file, write_atomic};

/// The manifest in the installed folder: a line per file, `<CRC-32 in hex> <size> <path>`.
pub const MANIFEST: &str = ".manifest";
const INDEX: &str = "index.json";
const CREDITS: &str = "CREDITS.txt";
/// A sounds ZIP holds the library in this folder.
const ZIP_TOP: &str = "content/";
/// The release of the sound library this app was built with, as its index records it (`tools/release-content.sh`
/// writes the file).
pub const SHIPPED_RELEASE: &str = include_str!("../../sounds/release.txt");

/// Files by path: their size and CRC-32.
type Manifest = BTreeMap<String, (u64, u32)>;

/// `<app data folder>/content` ([`crate::ui::main_menu::app_data_dir`]), never in the user folder. `None` without
/// one, and in tests, which never touch the user's folders.
pub fn dir() -> Option<PathBuf> {
    if cfg!(test) {
        return None;
    }
    let folder = crate::ui::main_menu::library::SOUND_LIBRARY_FOLDER;
    Some(crate::ui::main_menu::app_data_dir()?.join(folder))
}

/// The library at `dir` is complete: its manifest names the index in place.
pub fn complete(dir: &Path) -> bool {
    let index = index_at(dir);
    index.is_some_and(|i| manifest(dir).get(INDEX) == Some(&(i.len() as u64, crc32fast::hash(&i))))
}

/// The libraries at `a` and `b` have the same index.
pub fn same_index(a: &Path, b: &Path) -> bool {
    index_at(a).is_some_and(|index| index_at(b) == Some(index))
}

/// The release `a` (an index's, or [`SHIPPED_RELEASE`]) is older than `b`. Never when either is no `<n>.<n>…` version
/// (a dev build's `dev`).
pub fn older(a: &str, b: &str) -> bool {
    let version =
        |v: &str| (v.trim().split('.').map(str::parse::<u64>)).collect::<Result<Vec<_>, _>>();
    matches!((version(a), version(b)), (Ok(a), Ok(b)) if a < b)
}

/// The index file in `dir`, at most [`MAX_INDEX_BYTES`] of it.
fn index_at(dir: &Path) -> Option<Vec<u8>> {
    bounded(regular(&dir.join(INDEX))?).ok()
}

fn bounded(reader: impl Read) -> io::Result<Vec<u8>> {
    let mut bytes = Vec::new();
    reader
        .take(MAX_INDEX_BYTES as u64)
        .read_to_end(&mut bytes)?;
    Ok(bytes)
}

/// The manifest in `dir`; empty without one.
fn manifest(dir: &Path) -> Manifest {
    let text = fs::read_to_string(dir.join(MANIFEST)).unwrap_or_default();
    let line = |line: &str| {
        let (crc, rest) = line.split_once(' ')?;
        let (size, name) = rest.split_once(' ')?;
        let crc = u32::from_str_radix(crc, 16).ok()?;
        Some((name.to_owned(), (size.parse().ok()?, crc)))
    };
    text.lines().filter_map(line).collect()
}

/// How far a sync is, in bytes, and whether it is to stop.
#[derive(Default)]
pub struct Progress {
    pub done: AtomicU64,
    pub total: AtomicU64,
    pub cancel: AtomicBool,
}

/// What a sync stopped with on [`Progress::cancel`].
const CANCELLED: &str = "cancelled";

/// Where a sync reads from.
enum Source {
    Folder(PathBuf),
    /// A sounds ZIP, and its files below [`ZIP_TOP`].
    Zip(PathBuf, Manifest),
}

impl Source {
    /// The library at `path`: a sounds ZIP, a content folder, or a folder with one in it (a full download's).
    fn at(path: &Path) -> io::Result<Self> {
        if path.is_dir() {
            let inner = path.join("content");
            let unpacked = !path.join(INDEX).exists() && inner.join(INDEX).exists();
            return Ok(Self::Folder(if unpacked { inner } else { path.into() }));
        }
        let listed = crate::zip::list(File::open(path)?).map_err(io::Error::other)?;
        let files = (listed.into_iter()).filter_map(|(name, size, crc)| {
            Some((name.strip_prefix(ZIP_TOP)?.to_owned(), (size, crc)))
        });
        Ok(Self::Zip(path.into(), files.collect()))
    }

    /// `name`'s size, `None` when there is no such file.
    fn size(&self, name: &str) -> Option<u64> {
        match self {
            Self::Folder(dir) => (fs::metadata(dir.join(name)).ok())
                .filter(|m| m.is_file())
                .map(|m| m.len()),
            Self::Zip(_, files) => files.get(name).map(|f| f.0),
        }
    }

    /// `name`'s CRC-32: a folder's file is read for it.
    fn crc(&self, name: &str, progress: &Progress) -> io::Result<u32> {
        match self {
            Self::Folder(_) => Ok(pump(self.open(name)?, io::sink(), progress)?.1),
            Self::Zip(_, files) => files
                .get(name)
                .map(|f| f.1)
                .ok_or(io::ErrorKind::NotFound.into()),
        }
    }

    /// `name`'s bytes. A ZIP's fail as they are read when they do not match its CRC.
    fn open(&self, name: &str) -> io::Result<Box<dyn Read>> {
        match self {
            Self::Folder(dir) => Ok(Box::new(
                regular(&dir.join(name)).ok_or(io::ErrorKind::NotFound)?,
            )),
            Self::Zip(path, _) => {
                let entry = crate::zip::open_entry(File::open(path)?, &format!("{ZIP_TOP}{name}"));
                Ok(entry
                    .map_err(io::Error::other)?
                    .ok_or(io::ErrorKind::NotFound)?
                    .0)
            }
        }
    }
}

/// Copies `from` into `to` to its end: its length and CRC-32. Stops on [`Progress::cancel`].
fn pump(mut from: impl Read, mut to: impl Write, progress: &Progress) -> io::Result<(u64, u32)> {
    let (mut hasher, mut len, mut buf) = (crc32fast::Hasher::new(), 0, vec![0; 1 << 16]);
    loop {
        if progress.cancel.load(Relaxed) {
            return Err(io::Error::other(CANCELLED));
        }
        let n = match from.read(&mut buf) {
            Ok(0) => return Ok((len, hasher.finalize())),
            Ok(n) => n,
            Err(e) if e.kind() == io::ErrorKind::Interrupted => continue,
            Err(e) => return Err(e),
        };
        to.write_all(&buf[..n])?;
        hasher.update(&buf[..n]);
        len += n as u64;
    }
}

/// Makes `target` hold the library at `source` (a sounds ZIP or a content folder), as the module docs say: the files
/// it copied, in order. Nothing is copied when the index is the one in place and the library is complete, and a source
/// of an older release than the one in place is refused. One sync runs at a time.
pub fn sync(source: &Path, target: &Path, progress: &Progress) -> io::Result<Vec<String>> {
    static ONE: Mutex<()> = Mutex::new(());
    let _one = ONE.lock().unwrap_or_else(|e| e.into_inner());
    let source = Source::at(source)?;
    let index = bounded(source.open(INDEX)?)?;
    let parsed = Index::parse(&index)
        .ok_or_else(|| io::Error::new(io::ErrorKind::InvalidData, "no sound library"))?;
    let in_place = index_at(target);
    if in_place.as_ref() == Some(&index) && complete(target) {
        return Ok(Vec::new());
    }
    // Never a downgrade.
    let installed = in_place.as_deref().and_then(Index::parse);
    if let Some(installed) = installed.filter(|i| older(&parsed.release, &i.release)) {
        let (from, to) = (&parsed.release, &installed.release);
        return Err(io::Error::other(format!(
            "It is older ({from}) than the installed one ({to})."
        )));
    }
    let parts = parsed.entries.iter().flat_map(|e| e.parts.iter());
    let mut names: BTreeSet<String> = parts.map(|p| p.file.clone()).collect();
    names.extend(source.size(CREDITS).map(|_| CREDITS.to_owned()));
    let missing =
        |name: &str| io::Error::new(io::ErrorKind::NotFound, format!("{name} is missing"));
    let sizes: Vec<u64> = (names.iter())
        .map(|name| source.size(name).ok_or_else(|| missing(name)))
        .collect::<io::Result<_>>()?;
    progress.total.store(sizes.iter().sum(), Relaxed);
    fs::create_dir_all(target)?;
    let (old, mut new, mut copied) = (manifest(target), Manifest::new(), Vec::new());
    for (name, size) in names.into_iter().zip(sizes) {
        let at = target.join(&name);
        let in_place = fs::metadata(&at).is_ok_and(|m| m.is_file() && m.len() == size);
        let kept = old.get(&name).filter(|&&(len, _)| len == size && in_place);
        let file = match kept {
            Some(&(_, crc)) if source.crc(&name, progress)? == crc => (size, crc),
            _ => {
                fs::create_dir_all(at.parent().unwrap_or(target))?;
                let mut file = None;
                replace_file(&at, |out| {
                    file = Some(pump(source.open(&name)?, out, progress)?);
                    Ok(())
                })?;
                copied.push(name.clone());
                file.unwrap_or_default()
            }
        };
        progress.done.fetch_add(size, Relaxed);
        new.insert(name, file);
    }
    // The index once its parts are in, as read: the one the parts were picked by.
    write_atomic(&target.join(INDEX), &index)?;
    copied.push(INDEX.into());
    new.insert(INDEX.into(), (index.len() as u64, crc32fast::hash(&index)));
    sweep(target, target, &new)?;
    let lines = new
        .iter()
        .map(|(name, (size, crc))| format!("{crc:08x} {size} {name}\n"));
    write_atomic(&target.join(MANIFEST), lines.collect::<String>().as_bytes())?;
    Ok(copied)
}

/// Removes each file in `dir`, at any depth below `target`, that `keep` does not list, but the manifest.
fn sweep(target: &Path, dir: &Path, keep: &Manifest) -> io::Result<()> {
    for entry in fs::read_dir(dir)? {
        let entry = entry?;
        let path = entry.path();
        let name = path.strip_prefix(target).map_err(io::Error::other)?;
        let name = name.to_string_lossy().replace('\\', "/");
        if entry.file_type()?.is_dir() {
            sweep(target, &path, keep)?;
        } else if name != MANIFEST && !keep.contains_key(&name) {
            fs::remove_file(&path)?;
        }
    }
    Ok(())
}

/// [`sync`] on a worker thread; a failure is logged, and the next start tries again.
pub fn sync_in_background(source: PathBuf, target: PathBuf) {
    std::thread::spawn(move || {
        if let Err(err) = sync(&source, &target, &Progress::default()) {
            eprintln!("{}: {err}", target.display());
        }
    });
}

/// `Install sound library…`'s sync, on a worker thread.
pub struct Job {
    progress: Arc<Progress>,
    result: Receiver<io::Result<Vec<String>>>,
}

impl Job {
    /// Syncs `target` from `source` ([`sync`]).
    pub fn start(source: PathBuf, target: PathBuf) -> Self {
        let (progress, (tx, result)) = (Arc::new(Progress::default()), mpsc::channel());
        let on = progress.clone();
        std::thread::spawn(move || tx.send(sync(&source, &target, &on)));
        Self { progress, result }
    }

    /// How far it is, in percent.
    pub fn percent(&self) -> u64 {
        let (done, total) = (
            self.progress.done.load(Relaxed),
            self.progress.total.load(Relaxed),
        );
        (done * 100).checked_div(total).unwrap_or(0)
    }

    pub fn cancel(&self) {
        self.progress.cancel.store(true, Relaxed);
    }

    pub fn cancelled(&self) -> bool {
        self.progress.cancel.load(Relaxed)
    }

    /// Its result once it ended: why it failed.
    pub fn result(&self) -> Option<Result<(), String>> {
        match self.result.try_recv() {
            Ok(result) => Some(result.map(drop).map_err(|e| e.to_string())),
            Err(TryRecvError::Empty) => None,
            Err(TryRecvError::Disconnected) => Some(Err("the install stopped".into())),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::content::bundle::tests::fixture;
    use crate::content::bundle::{Bundle, Kind, Part};

    /// A part's name.
    fn part(n: u8) -> String {
        format!("p/{n:016x}.tar")
    }

    #[test]
    fn a_sync_copies_what_changed_removes_what_went_never_goes_back_and_one_cut_short_leaves_the_library_working()
     {
        let tmp = crate::project_files::content::tests::Temp::new("sounds-sync");
        let (folder, target, zip) = (
            tmp.0.join("a"),
            tmp.0.join("installed"),
            tmp.0.join("s.zip"),
        );
        let sync_from = |source: &Path| sync(source, &target, &Progress::default());
        let read = |rel: &str| Bundle::open(&target)?.read(rel);
        let entries = [
            ("Loops/", "A.flac", b"aaaa"),
            ("Kick/", "K.flac", b"kkkk"),
            ("Hat/", "H.flac", b"hhhh"),
        ];
        let entries =
            entries.map(|(path, name, bytes)| (Kind::Samples, path, vec![(name, bytes.to_vec())]));
        let mut index = fixture(&folder, &entries).index;
        let copied = sync_from(&folder).unwrap();
        assert_eq!(copied, [part(0), part(1), part(2), INDEX.into()]);
        assert_eq!(read("Loops/A.flac").as_deref(), Some(&b"aaaa"[..]));
        assert!(complete(&target));
        assert_eq!(
            sync_from(&folder).unwrap(),
            Vec::<String>::new(),
            "nothing new"
        );

        // A sounds ZIP: part 0 changed (its size kept), part 1 the same, part 2 gone for a new one.
        index.entries[2].parts[0].file = part(0xff);
        index.release = "0.2.0".into();
        let zipped = |index: &Index, parts: &[(String, &[u8])]| {
            let mut files = vec![(
                format!("{ZIP_TOP}{INDEX}"),
                serde_json::to_vec(index).unwrap(),
            )];
            files.extend(
                parts
                    .iter()
                    .map(|(name, bytes)| (format!("{ZIP_TOP}{name}"), bytes.to_vec())),
            );
            crate::zip::write(&files).unwrap()
        };
        let parts: Vec<(String, &[u8])> = vec![
            (part(0), b"AAAA"),
            (part(1), b"kkkk"),
            (part(0xff), b"hhhh"),
        ];
        fs::write(&zip, zipped(&index, &parts)).unwrap();
        assert_eq!(
            sync_from(&zip).unwrap(),
            [part(0), part(0xff), INDEX.into()]
        );
        assert!(!target.join(part(2)).exists());
        assert_eq!(read("Loops/A.flac").as_deref(), Some(&b"AAAA"[..]));
        assert_eq!(read("Hat/H.flac").as_deref(), Some(&b"hhhh"[..]));
        assert!(complete(&target));

        // An older release: refused, the library as it was.
        let mut old = index.clone();
        old.release = "0.1.9".into();
        fs::write(&zip, zipped(&old, &parts)).unwrap();
        let refused = sync_from(&zip).unwrap_err().to_string();
        assert!(refused.contains("older (0.1.9)"), "{refused}");
        assert_eq!(Bundle::open(&target).unwrap().index, index);

        // One more part, damaged in the ZIP (its CRC): the sync fails and the library is the one before.
        let mut more = index.clone();
        more.entries[1].parts.push(Part {
            file: part(0xcc),
            bytes: 4,
            sha256: String::new(),
        });
        let mut parts = parts;
        parts.push((part(0xcc), b"cccc"));
        let mut damaged = zipped(&more, &parts);
        let at = damaged.windows(4).position(|w| w == b"cccc").unwrap();
        damaged[at] = b'x';
        fs::write(&zip, damaged).unwrap();
        assert!(sync_from(&zip).is_err());
        assert!(!target.join(part(0xcc)).exists());
        assert_eq!(Bundle::open(&target).unwrap().index, index);
        assert_eq!(read("Kick/K.flac").as_deref(), Some(&b"kkkk"[..]));
        assert!(complete(&target));
        let left: Vec<_> = fs::read_dir(target.join("p"))
            .unwrap()
            .flatten()
            .map(|e| e.file_name())
            .collect();
        assert_eq!(left.len(), 3, "no temp file left: {left:?}");
    }
}
