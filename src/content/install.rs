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
//! - **Download** ([`download`], #66). The same, from a sound library release on GitHub
//!   ([`crate::update::sounds_url`]): Android's base APK and the desktop's `-no-sounds` downloads fetch only the parts
//!   they lack.

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

/// One sync or download at a time.
static ONE: Mutex<()> = Mutex::new(());

/// `<app data folder>/content` ([`crate::ui::main_menu::app_data_dir`]), never in the user folder. `None` without
/// one, and in tests, which never touch the user's folders.
pub fn dir() -> Option<PathBuf> {
    if cfg!(test) {
        return None;
    }
    let folder = crate::ui::main_menu::library::SOUND_LIBRARY_FOLDER;
    Some(crate::ui::main_menu::app_data_dir()?.join(folder))
}

/// Whether this build is to fetch its sound library ([`download`]), `bundle` being the one in use: none is in the APK
/// or beside the app, and the installed one is missing, cut short or older than [`SHIPPED_RELEASE`].
pub fn needed(bundle: Option<&super::Bundle>) -> bool {
    dir().is_some_and(|dir| needed_at(bundle, &dir))
}

/// [`needed`] with the installed library's folder `dir`.
fn needed_at(bundle: Option<&super::Bundle>, dir: &Path) -> bool {
    match bundle {
        Some(bundle) if bundle.dir.as_deref() != Some(dir) => false,
        Some(bundle) => !complete(dir) || older(&bundle.index.release, SHIPPED_RELEASE),
        None => true,
    }
}

/// The bytes of `index`'s parts that the library at `dir` lacks: the ones not in place at their size (a part is named
/// by its hash).
pub fn lacking(index: &Index, dir: &Path) -> u64 {
    let parts = index.entries.iter().flat_map(|e| e.parts.iter());
    let lacks = |p: &&super::bundle::Part| {
        fs::metadata(dir.join(&p.file)).map_or(0, |m| m.len()) != p.bytes
    };
    parts.filter(lacks).map(|p| p.bytes).sum()
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

/// How far a sync is, in bytes, whether it is to stop, and whether it is paused (a paused download waits before its
/// next part and goes on from the `.part` it left, so continuing downloads only what is missing).
#[derive(Default)]
pub struct Progress {
    pub done: AtomicU64,
    pub total: AtomicU64,
    pub cancel: AtomicBool,
    pub paused: AtomicBool,
}

impl Progress {
    /// Waits while the download is paused; false once it is to stop.
    pub fn wait_out_pause(&self) -> bool {
        while self.paused.load(Relaxed) {
            if self.cancel.load(Relaxed) {
                return false;
            }
            std::thread::sleep(std::time::Duration::from_millis(500));
        }
        true
    }
}

/// What a sync stopped with on [`Progress::cancel`].
const CANCELLED: &str = "cancelled";

/// Where a sync reads from.
enum Source {
    Folder(PathBuf),
    /// A sounds ZIP, and its files below [`ZIP_TOP`].
    Zip(PathBuf, Manifest),
    /// The library in this APK's assets (`content/`): Android's full APK.
    #[cfg(target_os = "android")]
    Apk,
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
            #[cfg(target_os = "android")]
            Self::Apk => {
                crate::android::asset(&format!("content/{name}")).map(|a| a.length() as u64)
            }
        }
    }

    /// `name`'s CRC-32: a folder's file is read for it.
    fn crc(&self, name: &str, progress: &Progress) -> io::Result<u32> {
        match self {
            #[cfg(target_os = "android")]
            Self::Apk => Ok(pump(self.open(name)?, io::sink(), progress)?.1),
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
            #[cfg(target_os = "android")]
            Self::Apk => {
                let asset = crate::android::asset(&format!("content/{name}"));
                Ok(Box::new(asset.ok_or(io::ErrorKind::NotFound)?))
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
    sync_from(Source::at(source)?, target, progress)
}

/// Before the base APK replaces this full one (#66): the library in this APK's assets copied into `target`, the
/// installed library's folder, which the base APK reads, so that it fetches only the parts that changed ([`sync`],
/// off the UI thread). `Ok(false)` when this APK has none, or `target` holds it already; [`io::ErrorKind::StorageFull`]
/// when `free` bytes leave no room for it and `spare` more (the update's own).
#[cfg(target_os = "android")]
pub fn keep_apk_library(
    target: &Path,
    free: u64,
    spare: u64,
    progress: &Progress,
) -> io::Result<bool> {
    let Some(index) = crate::android::asset_bytes("content/index.json", MAX_INDEX_BYTES) else {
        return Ok(false);
    };
    if index_at(target).as_deref() == Some(&index[..]) && complete(target) {
        return Ok(false);
    }
    let parsed = Index::parse(&index).ok_or(io::ErrorKind::InvalidData)?;
    if lacking(&parsed, target) + spare > free {
        return Err(io::Error::new(
            io::ErrorKind::StorageFull,
            "There is not enough free space.",
        ));
    }
    sync_from(Source::Apk, target, progress).map(|_| true)
}

/// [`sync`] from `source`.
fn sync_from(source: Source, target: &Path, progress: &Progress) -> io::Result<Vec<String>> {
    let _one = ONE.lock().unwrap_or_else(|e| e.into_inner());
    let index = bounded(source.open(INDEX)?)?;
    let parsed = Index::parse(&index)
        .ok_or_else(|| io::Error::new(io::ErrorKind::InvalidData, "no sound library"))?;
    if !news(target, &index, &parsed)? {
        return Ok(Vec::new());
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
    finish(target, &index, new, &mut copied)?;
    Ok(copied)
}

/// Whether `index` (`parsed`) is news for the library at `target`: not when it is the index in place and the library
/// is complete. An older release than the one in place is refused: never a downgrade.
fn news(target: &Path, index: &[u8], parsed: &Index) -> io::Result<bool> {
    let in_place = index_at(target);
    if in_place.as_deref() == Some(index) && complete(target) {
        return Ok(false);
    }
    let installed = in_place.as_deref().and_then(Index::parse);
    if let Some(installed) = installed.filter(|i| older(&parsed.release, &i.release)) {
        let (from, to) = (&parsed.release, &installed.release);
        return Err(io::Error::other(format!(
            "It is older ({from}) than the installed one ({to})."
        )));
    }
    Ok(true)
}

/// A sync's or download's end, once the parts in `files` are in: the index, as read (the one the parts were picked
/// by), then the strays go, then the manifest. `done` gets the index.
fn finish(
    target: &Path,
    index: &[u8],
    mut files: Manifest,
    done: &mut Vec<String>,
) -> io::Result<()> {
    write_atomic(&target.join(INDEX), index)?;
    done.push(INDEX.into());
    files.insert(INDEX.into(), (index.len() as u64, crc32fast::hash(index)));
    sweep(target, target, &files)?;
    let lines = files
        .iter()
        .map(|(name, (size, crc))| format!("{crc:08x} {size} {name}\n"));
    write_atomic(&target.join(MANIFEST), lines.collect::<String>().as_bytes())
}

/// Fetches a URL into a file, going on from the bytes the file holds, and tells the closure the file's size as it
/// grows ([`crate::update::get`]; a test's fake).
pub type Get<'a> = &'a dyn Fn(&str, &Path, &dyn Fn(u64)) -> io::Result<()>;

/// A file's name in a published library: flat, a release's downloads having no folders (`p/<hash>.tar` is
/// `<hash>.tar`).
fn published(name: &str) -> &str {
    name.strip_prefix("p/").unwrap_or(name)
}

/// The size of the file at `path`, 0 without one.
fn size_of(path: &Path) -> u64 {
    fs::metadata(path).map_or(0, |m| m.len())
}

/// The file at `at`'s CRC-32 and SHA-256 (base64, as an index's [`super::bundle::Part::sha256`]).
fn digest(at: &Path) -> io::Result<(u32, String)> {
    use sha2::Digest;
    let (mut file, mut crc, mut sha) = (
        File::open(at)?,
        crc32fast::Hasher::new(),
        sha2::Sha256::new(),
    );
    let mut buf = vec![0; 1 << 16];
    loop {
        let n = file.read(&mut buf)?;
        if n == 0 {
            return Ok((crc.finalize(), base64(&sha.finalize())));
        }
        crc.update(&buf[..n]);
        sha.update(&buf[..n]);
    }
}

/// `bytes` in standard base64, padded.
fn base64(bytes: &[u8]) -> String {
    const ABC: &[u8; 64] = b"ABCDEFGHIJKLMNOPQRSTUVWXYZabcdefghijklmnopqrstuvwxyz0123456789+/";
    let mut out = String::new();
    for chunk in bytes.chunks(3) {
        let n = chunk
            .iter()
            .enumerate()
            .fold(0u32, |n, (i, &b)| n | u32::from(b) << (16 - 8 * i));
        for i in 0..4 {
            let sextet = (n >> (18 - 6 * i) & 63) as usize;
            out.push(if i <= chunk.len() {
                ABC[sextet] as char
            } else {
                '='
            });
        }
    }
    out
}

/// Makes `target` hold the library published at `base` (`<base>/index.json`, `CREDITS.txt` and each part by its
/// [`published`] name), as [`sync`] does from a folder: the files it fetched, in order. A part is named by its hash,
/// so one in place at its size is kept and only the others are fetched, each into `<part>.part` first and renamed
/// into place once whole and of the SHA-256 its index gives: a download cut short goes on from there the next time,
/// and leaves a library that works. A part that comes damaged is fetched again from its start once, then the download
/// fails. `progress` counts the bytes of the parts it fetches.
pub fn download(
    base: &str,
    target: &Path,
    progress: &Progress,
    get: Get,
) -> io::Result<Vec<String>> {
    let _one = ONE.lock().unwrap_or_else(|e| e.into_inner());
    fs::create_dir_all(target)?;
    // A small file, whole each time: the index, then the credits.
    let whole = |name: &str| -> io::Result<Vec<u8>> {
        let at = target.join(format!(".{name}.part"));
        let _ = fs::remove_file(&at);
        get(&format!("{base}/{name}"), &at, &|_| ())?;
        let bytes = bounded(regular(&at).ok_or(io::ErrorKind::NotFound)?);
        let _ = fs::remove_file(&at);
        bytes
    };
    let index = whole(INDEX)?;
    let parsed = Index::parse(&index)
        .ok_or_else(|| io::Error::new(io::ErrorKind::InvalidData, "no sound library"))?;
    if !news(target, &index, &parsed)? {
        return Ok(Vec::new());
    }
    let parts: BTreeMap<&str, (u64, &str)> = (parsed.entries.iter())
        .flat_map(|e| {
            e.parts
                .iter()
                .map(|p| (p.file.as_str(), (p.bytes, p.sha256.as_str())))
        })
        .collect();
    let kept = |name: &str, size: u64| size_of(&target.join(name)) == size;
    progress.total.store(lacking(&parsed, target), Relaxed);
    let (old, mut new, mut fetched) = (manifest(target), Manifest::new(), Vec::new());
    for (&name, &(size, sha256)) in &parts {
        // A paused download waits here, then goes on; the part it left resumes from its `.part`.
        if !progress.wait_out_pause() {
            return Err(io::Error::other(CANCELLED));
        }
        let at = target.join(name);
        if kept(name, size) {
            let crc = old.get(name).filter(|f| f.0 == size).map(|f| f.1);
            new.insert(
                name.into(),
                (size, crc.map_or_else(|| digest(&at).map(|d| d.0), Ok)?),
            );
            continue;
        }
        fs::create_dir_all(at.parent().unwrap_or(target))?;
        let part = target.join(format!("{name}.part"));
        let done = progress.done.load(Relaxed);
        let mut tries = 0;
        let crc = loop {
            if size_of(&part) > size {
                fs::remove_file(&part)?;
            }
            if size_of(&part) < size {
                let url = format!("{base}/{}", published(name));
                get(&url, &part, &|n| {
                    progress.done.store(done + n.min(size), Relaxed)
                })?;
            }
            let (crc, sha) = digest(&part)?;
            if size_of(&part) == size && sha == sha256 {
                break crc;
            }
            let _ = fs::remove_file(&part);
            progress.done.store(done, Relaxed);
            tries += 1;
            if tries == 2 {
                return Err(io::Error::other(format!("{name} came damaged twice")));
            }
        };
        fs::rename(&part, &at)?;
        progress.done.store(done + size, Relaxed);
        new.insert(name.into(), (size, crc));
        fetched.push(name.into());
    }
    let credits = whole(CREDITS)?;
    write_atomic(&target.join(CREDITS), &credits)?;
    new.insert(
        CREDITS.into(),
        (credits.len() as u64, crc32fast::hash(&credits)),
    );
    fetched.push(CREDITS.into());
    finish(target, &index, new, &mut fetched)?;
    Ok(fetched)
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

    #[test]
    fn a_download_fetches_only_the_parts_it_lacks_checks_them_goes_on_from_one_cut_short_and_never_goes_back()
     {
        use sha2::Digest;
        use std::cell::{Cell, RefCell};
        let tmp = crate::project_files::content::tests::Temp::new("sounds-download");
        let (lib, site, target, apk) = (
            tmp.0.join("lib"),
            tmp.0.join("site"),
            tmp.0.join("installed"),
            tmp.0.join("from-apk"),
        );
        let sha = |bytes: &[u8]| base64(&sha2::Sha256::digest(bytes));
        assert_eq!(sha(b"abc"), "ungWv48Bz+pBQUDeXa4iI7ADYaOWF3qctBD/YfIAFa0=");
        let entries = [("Loops/", b"aaaa"), ("Kick/", b"kkkk"), ("Hat/", b"hhhh")];
        let entries =
            entries.map(|(path, bytes)| (Kind::Samples, path, vec![("A.flac", bytes.to_vec())]));
        let mut index = fixture(&lib, &entries).index;
        // `index` with each part's SHA-256, as the build tool writes it.
        let signed = |index: &Index, parts: &[(String, &[u8])]| {
            let mut index = index.clone();
            for p in index.entries.iter_mut().flat_map(|e| e.parts.iter_mut()) {
                if let Some((_, bytes)) = parts.iter().find(|(name, _)| *name == p.file) {
                    p.sha256 = sha(bytes);
                }
            }
            index
        };
        // A library published as a sound library release's downloads: flat.
        let publish = |index: &Index, parts: &[(String, &[u8])]| {
            let _ = fs::remove_dir_all(&site);
            fs::create_dir_all(&site).unwrap();
            let index = serde_json::to_vec(&signed(index, parts)).unwrap();
            fs::write(site.join(INDEX), index).unwrap();
            fs::write(site.join(CREDITS), b"credits").unwrap();
            for (name, bytes) in parts {
                fs::write(site.join(published(name)), bytes).unwrap();
            }
        };
        // The fake network: each fetch's file and the bytes it went on from; a part cut short after `cut` bytes, and
        // the next `damaged` parts with their first byte changed.
        let (asked, cut, damaged) = (RefCell::new(Vec::new()), Cell::new(None), Cell::new(0));
        let get = |url: &str, to: &Path, grown: &dyn Fn(u64)| -> io::Result<()> {
            let name = url.strip_prefix("https://host/").unwrap();
            let mut bytes = fs::read(site.join(name))?;
            let have = size_of(to) as usize;
            asked.borrow_mut().push((name.to_owned(), have));
            let part = name.ends_with(".tar");
            if part && damaged.get() > 0 {
                damaged.set(damaged.get() - 1);
                bytes[0] ^= 1;
            }
            let end = part.then(|| cut.take()).flatten().unwrap_or(bytes.len());
            let mut file = fs::OpenOptions::new().append(true).create(true).open(to)?;
            file.write_all(&bytes[have..end])?;
            grown(end as u64);
            (end == bytes.len())
                .then_some(())
                .ok_or(io::Error::other("cut"))
        };
        let fetch_into = |dir: &Path| {
            asked.borrow_mut().clear();
            download("https://host", dir, &Progress::default(), &get)
        };
        let fetch = || fetch_into(&target);
        let read = |rel: &str| Bundle::open(&target)?.read(rel);
        let flat = |n: u8| (published(&part(n)).to_owned(), 0);
        let first: [(String, &[u8]); 3] =
            [(part(0), b"aaaa"), (part(1), b"kkkk"), (part(2), b"hhhh")];

        // The full APK's library copied out before the base APK replaces it (`keep_apk_library`, a sync): the base APK
        // has this build's release, needs no download, and one fetches nothing but the index.
        let mut shipped = signed(&index, &first);
        shipped.release = SHIPPED_RELEASE.trim().into();
        fs::write(lib.join(INDEX), serde_json::to_vec(&shipped).unwrap()).unwrap();
        sync(&lib, &apk, &Progress::default()).unwrap();
        assert!(!needed_at(Bundle::open(&apk).as_ref(), &apk));
        assert!(needed_at(None, &target));
        publish(&shipped, &first);
        assert_eq!(fetch_into(&apk).unwrap(), Vec::<String>::new());
        assert_eq!(*asked.borrow(), [(INDEX.to_owned(), 0)]);

        index.release = "0.1.0".into();
        publish(&index, &first);
        let fetched = fetch().unwrap();
        assert_eq!(
            fetched,
            [part(0), part(1), part(2), CREDITS.into(), INDEX.into()]
        );
        assert_eq!(read("Kick/A.flac").as_deref(), Some(&b"kkkk"[..]));
        assert!(complete(&target));
        assert_eq!(fetch().unwrap(), Vec::<String>::new(), "nothing new");
        assert_eq!(*asked.borrow(), [(INDEX.to_owned(), 0)]);

        // A new release: part 0 changed (a new name), part 1 the same, part 2 gone for a new one. The first part
        // fetched is cut short: the library stays the one before.
        index.release = "0.2.0".into();
        index.entries[0].parts[0].file = part(0xa0);
        index.entries[2].parts[0].file = part(0xff);
        let second: [(String, &[u8]); 3] = [
            (part(0xa0), b"AAAA"),
            (part(1), b"kkkk"),
            (part(0xff), b"HHHH"),
        ];
        publish(&index, &second);
        cut.set(Some(2));
        assert!(fetch().is_err());
        assert_eq!(read("Loops/A.flac").as_deref(), Some(&b"aaaa"[..]));
        assert!(complete(&target));
        // The next one goes on from those 2 bytes and fetches nothing else it has.
        let fetched = fetch().unwrap();
        assert_eq!(
            fetched,
            [part(0xa0), part(0xff), CREDITS.into(), INDEX.into()]
        );
        let resumed = (published(&part(0xa0)).to_owned(), 2);
        assert_eq!(asked.borrow()[1..3], [resumed, flat(0xff)]);
        assert_eq!(read("Loops/A.flac").as_deref(), Some(&b"AAAA"[..]));
        assert_eq!(read("Hat/A.flac").as_deref(), Some(&b"HHHH"[..]));
        assert!(!target.join(part(2)).exists() && complete(&target));

        // A part that comes damaged (not its SHA-256) is fetched again once; damaged twice, the download fails and
        // the library stays the one before.
        index.release = "0.3.0".into();
        index.entries[1].parts[0].file = part(0xbb);
        let third: [(String, &[u8]); 3] = [
            (part(0xa0), b"AAAA"),
            (part(0xbb), b"KKKK"),
            (part(0xff), b"HHHH"),
        ];
        publish(&index, &third);
        damaged.set(2);
        let failed = fetch().unwrap_err().to_string();
        assert!(failed.contains("damaged"), "{failed}");
        assert_eq!(read("Kick/A.flac").as_deref(), Some(&b"kkkk"[..]));
        assert!(complete(&target));
        damaged.set(1);
        let fetched = fetch().unwrap();
        assert_eq!(fetched, [part(0xbb), CREDITS.into(), INDEX.into()]);
        let again = (asked.borrow().iter())
            .filter(|a| *a == &flat(0xbb))
            .count();
        assert_eq!(again, 2, "fetched again from its start");
        assert_eq!(read("Kick/A.flac").as_deref(), Some(&b"KKKK"[..]));

        // An older release: refused, the library as it was.
        index.release = "0.1.9".into();
        publish(&index, &[]);
        let refused = fetch().unwrap_err().to_string();
        assert!(refused.contains("older (0.1.9)"), "{refused}");
        assert_eq!(Bundle::open(&target).unwrap().index.release, "0.3.0");
    }
}
