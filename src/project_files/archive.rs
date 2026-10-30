//! Opening a project archive (`.zip`). [`crate::zip::read`] takes the
//! hostile bytes: its caps ([`MAX_FILE_BYTES`] in all, the inflate ratio, 10 000 entries) hold, and every name that
//! could leave the folder it is extracted into (`..`, absolute, a drive, a link) is refused there. Then:
//! - An archive with exactly one song (`<name>.flm` and the user files it uses) is extracted into
//!   `<user>/Unzipped songs/<zip name>/` (on the web, into this session's copies), in place of what an earlier archive
//!   of that name left there, which goes into the bin unless it still holds the archive's files ([`bin_earlier`]),
//!   and the song opens with the `.zip` as its path and the zip name as its title. Its `@L@` paths whose files the
//!   archive holds now lead into that folder ([`relink`]), so the song plays them, and
//!   still does once saved and opened again.
//! - Any other archive's known files ([`import_folder`]) are counted: none is a message, some ask `Import %d files?`,
//!   and `Yes` copies them into their user folders by type, `name (2)` and so on on a clash, then sums up how many went
//!   where. No delete-archive question.

use std::io;
use std::path::{Path, PathBuf};

use super::picks::{DW_SAMPLER, SLICER, SOUNDFONT_PLAYER, THREE_OSC};
use super::samples::import_folder;
use super::{
    DRUMSETS_FOLDER, INSTRUMENTS_FOLDER, MAX_FILE_BYTES, MIDI_FOLDER, PRESETS_FOLDER, ProjectFiles,
    RACKS_FOLDER, READ_FAILED, SAMPLES_FOLDER, SOUNDFONTS_FOLDER, in_user_folder, stem,
};
use crate::audio_engine::samples::dw_path;
use crate::flm::chunk::{ChunkWriter, ContentPath};
use crate::song::Song;
use crate::ui::main_menu::library::RECENTLY_DELETED;
use crate::ui::main_menu::{SONGS_FOLDER, SongEntry, SongLibrary};
use crate::zip::{self, Entry, ZipError};

/// The archive messages.
pub const ARCHIVE_FAILED: &str = "Failed to open project archive";
pub const DECOMPRESS_FAILED: &str = "Failed to decompress project archive";
pub const NOTHING_KNOWN: &str = "The archive doesn't contain any files this app knows how to open";
pub const IMPORT_FAILED: &str = "Import failed";
/// The tag of the `Import %d files?` question.
pub const IMPORT_TAG: &str = "project_files_import";
/// The folder in the user folder archives are extracted into.
pub const UNZIPPED_FOLDER: &str = "Unzipped songs";
/// The extracted folder of an archive whose name makes no folder name (`..zip`).
const UNNAMED: &str = "Archive";

/// The archive's song, if it holds exactly one: a `.flm` that is no hidden file.
pub(super) fn the_song(entries: &[Entry]) -> Option<usize> {
    let is_song = |e: &Entry| import_folder(Path::new(&e.path)) == Some(SONGS_FOLDER);
    let mut songs = (0..entries.len()).filter(|&i| is_song(&entries[i]));
    match (songs.next(), songs.next()) {
        (Some(at), None) => Some(at),
        _ => None,
    }
}

/// Puts an earlier extraction at `dir` into `lib`'s bin as `Delete` does (at its place below the user folder, a taken
/// name numbered, kept 30 days). An archive extracts into the fixed folder `Unzipped songs/<zip name>/`,
/// where the load lookup finds the song's files, and SONGS lists that folder for the user to browse and edit, so the
/// folder in the way is binned, never removed for good. One that still [`holds`] the archive's files is not in the
/// way: [`ProjectFiles::extract`] keeps it.
fn bin_earlier(lib: &mut dyn SongLibrary, dir: &Path) -> Result<(), String> {
    if !lib.exists(dir) {
        return Ok(());
    }
    let earlier = SongEntry {
        name: dir.file_name().unwrap_or_default().to_string_lossy().into(),
        path: dir.to_owned(),
        is_folder: true,
        recency: 0,
    };
    crate::ui::main_menu::delete(lib, &[earlier], &mut Vec::new())
}

/// The flash line for an extraction into `dir` that put the earlier folder into the bin ([`bin_earlier`]): where it
/// went.
pub(super) fn earlier_in_bin(dir: &Path) -> String {
    let name = dir.file_name().unwrap_or_default().to_string_lossy();
    format!("The earlier \"{UNZIPPED_FOLDER}/{name}\" is in \"{RECENTLY_DELETED}\"")
}

/// `dir` holds exactly `entries`, what extracting them would make: every file listed in it is one of them (so a
/// smaller archive leaves nothing of a bigger one), and each of them is there with the same bytes, read by its path
/// (so a hidden entry, `.DS_Store` or `__MACOSX/._*`, which the listing leaves out, counts too). Never so through a
/// link (`dir`, or the folder it is in). The listing is compared first, so a file added reads nothing; the archive
/// has no CRC of a file cheaper than its bytes, as a CRC of the file on disk would read it whole too. A hidden file
/// or link the user put in `dir` stays, as `dir` is kept.
fn holds(lib: &dyn SongLibrary, dir: &Path, entries: &[Entry]) -> bool {
    if lib.is_link(dir) || dir.parent().is_some_and(|top| lib.is_link(top)) {
        return false;
    }
    let (mut files, mut todo) = (Vec::new(), vec![dir.to_owned()]);
    while let Some(at) = todo.pop() {
        let Ok(listed) = lib.list_all(&at) else {
            return false;
        };
        for e in listed {
            match e.is_folder {
                true => todo.push(e.path),
                false => files.push(e.path),
            }
        }
        if files.len() > entries.len() {
            return false;
        }
    }
    let mut want: Vec<PathBuf> = entries.iter().map(|e| dir.join(&e.path)).collect();
    want.sort();
    files.iter().all(|f| want.binary_search(f).is_ok())
        && (entries.iter()).all(|e| lib.read(&dir.join(&e.path)).is_ok_and(|b| b == e.data))
}

/// `path` for a message: a deep one as its first 60 characters, `…` and its last 60.
fn shown(path: &Path) -> String {
    let full = path.display().to_string();
    let n = full.chars().count();
    if n <= 121 {
        return full;
    }
    let head: String = full.chars().take(60).collect();
    let tail: String = full.chars().skip(n - 60).collect();
    format!("{head}…{tail}")
}

/// Every content path `song`'s samples come from that `map` maps (the sources `audio_engine::samples::walk` reads):
/// sampler slots (drum pads, clips), a DW Sampler's program and `ZONE` files, a Slicer's loop, 3x Osc's custom shape
/// (`PATH`) and a SoundFont Player's `SF2D`. The rest of each `CSTM` stays byte for byte.
pub(crate) fn relink(song: &mut Song, map: &dyn Fn(&ContentPath) -> Option<ContentPath>) {
    for channel in &mut song.channels {
        let pads = channel.drum_kit.iter_mut().flat_map(|kit| &mut kit.slots);
        let clips = (channel.lanes.iter_mut())
            .flat_map(|lane| &mut lane.clips)
            .filter_map(|clip| clip.file.sample.as_deref_mut());
        for slot in pads.chain(clips) {
            if let Some(new) = slot.file.path.as_ref().and_then(map) {
                slot.file.path = Some(new);
            }
        }
        for module in &mut channel.rack.modules {
            // The DW Sampler's blob starts with its magic; the others' are chunks.
            let (magic, ids): (&[u8], &[[u8; 4]]) = match module.type_id {
                DW_SAMPLER => (b"10WD", &[*b"PATh", *b"PATH", *b"ZONE"]),
                SLICER | THREE_OSC => (b"", &[*b"PATH"]),
                SOUNDFONT_PLAYER => (b"", &[*b"SF2D"]),
                _ => continue,
            };
            let blob = module.file.custom_data.as_deref().unwrap_or_default();
            if let Some(chunks) = blob.strip_prefix(magic)
                && let Some(new) = relink_chunks(chunks, ids, map)
            {
                module.file.custom_data = Some([magic, &new].concat());
            }
        }
    }
}

/// `chunks` (id, u32 size, body; a size past the end reads to the end) with the path at the start of each `ids` chunk
/// mapped: a `ZONE`'s `char[1024]`, else a length-prefixed one. `None` when nothing changed.
fn relink_chunks(
    chunks: &[u8],
    ids: &[[u8; 4]],
    map: &dyn Fn(&ContentPath) -> Option<ContentPath>,
) -> Option<Vec<u8>> {
    let (mut out, mut rest, mut changed) = (Vec::with_capacity(chunks.len()), chunks, false);
    while rest.len() >= 8 {
        let (head, after) = rest.split_at(8);
        let size = u32::from_le_bytes([head[4], head[5], head[6], head[7]]) as usize;
        let (body, next) = after.split_at(size.min(after.len()));
        rest = next;
        let relinked = (ids.iter().any(|id| head.starts_with(id)))
            .then(|| relink_body(head.starts_with(b"ZONE"), body, map))
            .flatten();
        match relinked {
            Some(body) => {
                changed = true;
                out.extend([&head[..4], &(body.len() as u32).to_le_bytes(), &body].concat());
            }
            None => out.extend([head, body].concat()),
        }
    }
    out.extend(rest);
    changed.then_some(out)
}

fn relink_body(
    zone: bool,
    body: &[u8],
    map: &dyn Fn(&ContentPath) -> Option<ContentPath>,
) -> Option<Vec<u8>> {
    let (raw, after) = match zone {
        true => body.split_at_checked(1024)?,
        false => {
            let len = u32::from_le_bytes(body.get(..4)?.try_into().ok()?) as usize;
            body.get(4..)?.split_at_checked(len)?
        }
    };
    let mut w = ChunkWriter::default();
    w.content_path32(&map(&dw_path(raw)?)?).ok()?;
    let mut path = w.into_bytes().split_off(4);
    match zone {
        // Shorter than its field, so that a NUL ends it.
        true if path.len() >= 1024 => None,
        true => {
            path.resize(1024, 0);
            Some([path, after.to_vec()].concat())
        }
        false => Some([&(path.len() as u32).to_le_bytes()[..], &path, after].concat()),
    }
}

impl ProjectFiles {
    /// The archive's files; `None` once the message says why not (not a ZIP file, or a damaged or hostile one).
    pub(super) fn unzip(&mut self, bytes: &[u8]) -> Option<Vec<Entry>> {
        let read = zip::read(bytes, MAX_FILE_BYTES);
        read.map_err(|err| {
            let title = match err {
                ZipError::NotZip => ARCHIVE_FAILED,
                ZipError::Corrupt => DECOMPRESS_FAILED,
            };
            self.fail(title, err.to_string());
        })
        .ok()
    }

    /// Extracts `entries` into `<user>/Unzipped songs/<name of the .zip>/` and returns that folder, and whether an
    /// earlier one went into the bin. A folder there that [`holds`] them already is kept as it is. Else they are
    /// written into a new folder beside it first, which then takes its place, the folder there going into the bin
    /// ([`bin_earlier`]): a failure leaves the folder as it was (or, if binning it failed part way, in the bin).
    pub(super) fn extract(
        &mut self,
        zip: &Path,
        entries: &[Entry],
    ) -> Result<(PathBuf, bool), String> {
        let user = self.user.clone().ok_or("There is no user folder.")?;
        let top = user.join(UNZIPPED_FOLDER);
        let name = stem(zip);
        // A stem with a leading dot is no folder name here: the staging folders have one.
        let named = Self::stays_inside(Path::new(&name))
            && !name.starts_with('.')
            && !name.trim_matches(['.', ' ']).is_empty();
        let name = if named { &name } else { UNNAMED };
        let (dir, staging) = (top.join(name), top.join(format!(".{name}.part")));
        // `zip::read` refuses every name that leaves the folder; checked again where the names meet the disk.
        if let Some(e) = entries
            .iter()
            .find(|e| !Self::stays_inside(Path::new(&e.path)))
        {
            return Err(format!(
                "{}: not inside the archive",
                shown(Path::new(&e.path))
            ));
        }
        if self.copies {
            let files = &mut self.user_copies;
            if holds(files, &dir, entries) {
                return Ok((dir, false));
            }
            let binned = files.exists(&dir);
            let at = |path: &Path, e: io::Error| format!("{}: {e}", shown(path));
            let _ = files.remove(&staging);
            files.make_folder(&staging).map_err(|e| at(&staging, e))?;
            let staged = entries.iter().try_for_each(|entry| {
                let path = staging.join(&entry.path);
                files
                    .put(path.clone(), &entry.data)
                    .map_err(|e| at(&path, e))
            });
            if let Err(err) = staged.and_then(|()| bin_earlier(files, &dir)) {
                let _ = files.remove(&staging);
                return Err(err);
            }
            files.move_to(&staging, &dir).map_err(|e| at(&dir, e))?;
            return Ok((dir, binned));
        }
        #[cfg(not(target_arch = "wasm32"))]
        {
            let mut lib = crate::ui::main_menu::NativeLibrary::user_folder(user);
            if holds(&lib, &dir, entries) {
                return Ok((dir, false));
            }
            // A link there is removed, not binned.
            let binned = lib.exists(&dir) && !lib.is_link(&dir);
            extract_on_disk(&top, &staging, &dir, entries, |dir| {
                bin_earlier(&mut lib, dir)
            })?;
            Ok((dir, binned))
        }
        // The web always keeps copies.
        #[cfg(target_arch = "wasm32")]
        Err("no files on disk".into())
    }

    /// SONGS `Unzip` on the library's archive at `path`: every file in it is extracted into
    /// `<user>/Unzipped songs/<name of the .zip>/` ([`Self::extract`], where opening it extracts to as well),
    /// and the song is not opened. The flash says where, and SONGS reads its folder
    /// again ([`Self::library_changes`]). A file that is gone, no archive or a damaged one, or a folder that cannot be
    /// written, shows its message as opening one does.
    pub fn unzip_archive(&mut self, path: &Path) {
        // ponytail: runs on the UI thread, like `Open`: reading, inflating and writing up to MAX_FILE_BYTES can stall
        // the frame. Upgrade: do it on a worker and report with a `FileEvent`.
        let bytes = match self.library.read(path) {
            Ok(bytes) => bytes,
            Err(err) => return self.fail(READ_FAILED, format!("{}: {err}", path.display())),
        };
        let Some(entries) = self.unzip(&bytes) else {
            return;
        };
        match self.extract(path, &entries) {
            Ok((dir, binned)) => {
                let name = dir.file_name().unwrap_or_default().to_string_lossy();
                let mut flash = format!("Unzipped to \"{UNZIPPED_FOLDER}/{name}\"");
                if binned {
                    flash = format!("{flash}\n{}", earlier_in_bin(&dir));
                }
                self.flash = Some(flash);
                self.library_changes += 1;
            }
            Err(err) => self.fail(DECOMPRESS_FAILED, err),
        }
    }

    /// The song an archive extracted into `dir` gave: its `@L@` paths whose file `dir` holds at the same relative path
    /// lead there; any other path stays as it is.
    pub(super) fn relink_to(&self, song: &mut Song, dir: &Path) {
        relink(song, &|path| {
            let file = dir.join(in_user_folder(path)?);
            (self.user_file_exists(&file))
                .then(|| self.user_shorthand(&file))
                .flatten()
        });
    }

    /// An archive that is not one song: its known files are counted, and imported once `Import %d files?` is
    /// answered `Yes` ([`Self::import_files`]).
    pub(super) fn ask_import(&mut self, entries: Vec<Entry>) {
        let known = entries
            .into_iter()
            .filter(|e| import_folder(Path::new(&e.path)).is_some());
        let known: Vec<_> = known.collect();
        if known.is_empty() {
            self.message = Some(vec![NOTHING_KNOWN.into()]);
            return;
        }
        self.message = Some(vec![format!("Import {} files?", known.len())]);
        self.importing = Some(known);
    }

    /// `Import %d files?` answered `Yes`: each file goes into its user folder ([`Self::import_file`]), an audio file
    /// that goes with a DirectWave program ([`program_of`]) into the program's sample folder, then `Imported %d
    /// file%s` shows, with a line per folder that took some ([`imported_lines`]). The first that cannot be written
    /// stops the import with `Import failed`.
    pub(super) fn import_files(&mut self, entries: Vec<Entry>) {
        let is_program = |e: &Entry| import_folder(Path::new(&e.path)) == Some(INSTRUMENTS_FOLDER);
        // The programs first: their samples go into the folder named after the name each one takes.
        let (programs, rest): (Vec<_>, Vec<_>) =
            (0..entries.len()).partition(|&i| is_program(&entries[i]));
        let mut sample_folder: Vec<Option<PathBuf>> = vec![None; entries.len()];
        for i in programs.into_iter().chain(rest) {
            let (entry, at) = (&entries[i], Path::new(&entries[i].path));
            let program =
                program_of(&entries, at).and_then(|p| Some((p, sample_folder[p].clone()?)));
            let written = match program {
                Some((p, folder)) => {
                    let into = folder.join(in_program_folder(&entries[p], at));
                    self.import_into(into.parent().unwrap_or(&folder), entry, false)
                }
                None => self.import_file(entry),
            };
            match written {
                Ok(written) if is_program(entry) => {
                    sample_folder[i] = Some(written.with_extension(""))
                }
                Ok(_) => {}
                Err(err) => {
                    let path = shown(at);
                    return self.fail(IMPORT_FAILED, format!("{path}: {err}"));
                }
            }
        }
        // SONGS reads its folder again: songs may have come in.
        self.library_changes += 1;
        self.message = Some(imported_lines(&entries));
    }

    /// Writes the archive's file `entry` into its user folder ([`import_folder`]) and returns where
    /// ([`Self::import_into`]): below the same folders when the archive has it in that user folder (an archive mirrors
    /// the user folder), else right in it. A song goes into the songs folder, through the library (the web keeps songs
    /// there), under a free name.
    fn import_file(&mut self, entry: &Entry) -> io::Result<PathBuf> {
        let path = Path::new(&entry.path);
        let folder = import_folder(path).ok_or(io::ErrorKind::Unsupported)?;
        if folder == SONGS_FOLDER {
            let name = self.free_name_from(2, self.root.as_deref(), &stem(path), "flm", None);
            let name = format!("{name}.flm");
            self.library.store(&name, &entry.data)?;
            return Ok(name.into());
        }
        let user = self.user.as_deref().ok_or(io::ErrorKind::NotFound)?;
        let mut dir = user.join(folder);
        if let Some(sub) = path.parent().and_then(|p| p.strip_prefix(folder).ok()) {
            dir.push(sub);
        }
        self.import_into(&dir, entry, folder == INSTRUMENTS_FOLDER)
    }

    /// Writes the archive's file `entry` into `dir` under its name or a free `name (2)`, `name (3)`, ...; a `program`'s
    /// name also leaves its sample folder (the name without the extension) to it alone. Returns where it is.
    fn import_into(&mut self, dir: &Path, entry: &Entry, program: bool) -> io::Result<PathBuf> {
        let path = Path::new(&entry.path);
        let (base, ext) = (
            stem(path),
            path.extension().unwrap_or_default().to_string_lossy(),
        );
        let taken = |name: &String| {
            self.taken(&dir.join(format!("{name}.{ext}"))) || program && self.taken(&dir.join(name))
        };
        let name = std::iter::once(base.clone())
            .chain((2..).map(|n| format!("{base} ({n})")))
            .find(|name| !taken(name))
            .expect("an unbounded range has a free number");
        let target = dir.join(format!("{name}.{ext}"));
        self.write_user_file(&target, entry.data.clone())?;
        Ok(target)
    }
}

/// The folders an archive's files are sorted into ([`import_folder`]), in the import summary's order, with what one
/// and more of their files are.
const IMPORTED_KINDS: [(&str, &str, &str); 8] = [
    (SAMPLES_FOLDER, "sample", "samples"),
    (MIDI_FOLDER, "MIDI file", "MIDI files"),
    (SONGS_FOLDER, "song", "songs"),
    (INSTRUMENTS_FOLDER, "instrument", "instruments"),
    (SOUNDFONTS_FOLDER, "SoundFont", "SoundFonts"),
    (PRESETS_FOLDER, "preset", "presets"),
    (RACKS_FOLDER, "rack", "racks"),
    (DRUMSETS_FOLDER, "drumset", "drumsets"),
];

/// The DirectWave program (`.dwp`) the archive's audio file `at` goes with, so that it finds its samples
/// (`sample_cache::zone_file`), as an index into `entries`: the program in the nearest folder of the archive that holds
/// the file at any depth (one at the archive's top takes every audio file); of several there, the one whose own sample
/// folder (its name without `.dwp`) holds the file, else the first. `None` for a plain sample and any other file.
// ponytail: each audio file looks at every program, O(files × programs) for an archive's 10 000 entries at most.
fn program_of(entries: &[Entry], at: &Path) -> Option<usize> {
    if import_folder(at) != Some(SAMPLES_FOLDER) {
        return None;
    }
    let folder = |p: &Path| p.parent().unwrap_or(Path::new("")).to_owned();
    (entries.iter().enumerate())
        .map(|(i, e)| (i, Path::new(&e.path)))
        .filter(|&(_, p)| import_folder(p) == Some(INSTRUMENTS_FOLDER) && at.starts_with(folder(p)))
        .max_by_key(|&(i, p)| {
            let own = folder(p).join(stem(p));
            (
                folder(p).components().count(),
                at.starts_with(own),
                std::cmp::Reverse(i),
            )
        })
        .map(|(i, _)| i)
}

/// Where the audio file `at` goes in the sample folder of `program` ([`program_of`]): its path from the program's
/// folder, less the program's own sample folder when it is in it (`Keys/Keys.dwp` with `Keys/C4.wav` or with
/// `Keys/Keys/C4.wav`: `C4.wav`).
fn in_program_folder(program: &Entry, at: &Path) -> PathBuf {
    let program = Path::new(&program.path);
    let rel = at
        .strip_prefix(program.parent().unwrap_or(Path::new("")))
        .unwrap_or(at);
    rel.strip_prefix(stem(program)).unwrap_or(rel).to_owned()
}

/// The summary of an import of `entries`: `Imported %d file%s`, then `%d samples in My Samples` and so on, a line
/// per folder that took some; the samples that went with a program count with the instruments
/// (`1 instrument (with 12 samples) in My Instruments`).
fn imported_lines(entries: &[Entry]) -> Vec<String> {
    let n = entries.len();
    let s = if n == 1 { "" } else { "s" };
    let with_program = |e: &Entry| program_of(entries, Path::new(&e.path)).is_some();
    let per_folder = IMPORTED_KINDS.iter().filter_map(|&(folder, one, more)| {
        let into =
            |e: &&Entry| import_folder(Path::new(&e.path)) == Some(folder) && !with_program(e);
        let n = entries.iter().filter(into).count();
        let samples = match folder == INSTRUMENTS_FOLDER {
            true => entries.iter().filter(|e| with_program(e)).count(),
            false => 0,
        };
        let with = match samples {
            0 => String::new(),
            1 => " (with 1 sample)".into(),
            n => format!(" (with {n} samples)"),
        };
        (n > 0).then(|| format!("{n} {}{with} in {folder}", if n == 1 { one } else { more }))
    });
    std::iter::once(format!("Imported {n} file{s}"))
        .chain(per_folder)
        .collect()
}

/// [`ProjectFiles::extract`] on disk. `top` must be no link, so that nothing is removed or written outside the user
/// folder. `staging` is made new (one a crash left is removed first) and each file in it new, plainly: the folder is
/// the extraction's own, so a file is no song to keep whole (no sync, no rename per file). Then `dir` goes into the
/// bin through `bin` (a link there is removed, not what it points at) and `staging` is renamed to it; on any error
/// `staging` goes and `dir` stays.
// ponytail: checked, then used by path: a process of the same user that swaps a folder on the way for a link between
// the check and the write is followed. `openat` with `O_NOFOLLOW` from the user folder down would close it.
#[cfg(not(target_arch = "wasm32"))]
fn extract_on_disk(
    top: &Path,
    staging: &Path,
    dir: &Path,
    entries: &[Entry],
    bin: impl FnOnce(&Path) -> Result<(), String>,
) -> Result<(), String> {
    use std::fs;
    use std::io::Write;
    fn at(path: &Path) -> impl Fn(io::Error) -> String + '_ {
        move |e| format!("{}: {e}", shown(path))
    }
    let gone = |path: &Path| match fs::remove_dir_all(path) {
        Err(e) if e.kind() == io::ErrorKind::NotFound => Ok(()),
        done => done,
    };
    fs::create_dir_all(top).map_err(at(top))?;
    if fs::symlink_metadata(top).map_err(at(top))?.is_symlink() {
        return Err(format!("{}: a link", shown(top)));
    }
    gone(staging).map_err(at(staging))?;
    fs::create_dir(staging).map_err(at(staging))?;
    let written = entries.iter().try_for_each(|entry| {
        let path = staging.join(&entry.path);
        if let Some(folder) = path.parent() {
            fs::create_dir_all(folder).map_err(at(&path))?;
        }
        let file = fs::OpenOptions::new()
            .write(true)
            .create_new(true)
            .open(&path);
        file.and_then(|mut f| f.write_all(&entry.data))
            .map_err(at(&path))
    });
    let moved = written.and_then(|()| {
        match fs::symlink_metadata(dir) {
            Ok(meta) if meta.is_symlink() => gone(dir).map_err(at(dir))?,
            _ => bin(dir)?,
        }
        fs::rename(staging, dir).map_err(at(dir))
    });
    if moved.is_err() {
        let _ = fs::remove_dir_all(staging);
    }
    moved
}

#[cfg(test)]
mod tests {
    use std::path::{Path, PathBuf};

    use super::*;
    use crate::flm::chunk::ContentRoot;
    use crate::flm::sf2d::Sf2d;
    use crate::project_files::mem::MemFiles;
    use crate::project_files::tests::{synthetic_codec, synthetic_file};
    use crate::project_files::{MESSAGE_TAG, NEWER_TAG, Replace};
    use crate::sample_cache::{SampleCache, reader};
    use crate::smf::codec::{Smf, SmfEvent};
    use crate::song::{ChannelKind, Song, SynthKind};
    use crate::ui::views::{Answer, Overlay};
    use honeydew_audio::wav_writer::float32_wav;

    fn user(relative: &str) -> ContentPath {
        ContentPath::Shorthand {
            root: ContentRoot::User,
            relative: relative.into(),
        }
    }

    /// A mono WAV of `frames` frames.
    fn wav(frames: usize) -> Vec<u8> {
        float32_wav(44_100, &[vec![0.25f32; frames]])
    }

    fn zip_of(files: &[(&str, &[u8])]) -> Vec<u8> {
        let entries: Vec<_> = (files.iter())
            .map(|(name, data)| (name.to_string(), data.to_vec()))
            .collect();
        zip::write(&entries).unwrap()
    }

    /// `zip` with every `from` (in the local header and the central directory) changed to `to`, of the same length:
    /// how names the writer refuses get into an archive.
    fn renamed(mut zip: Vec<u8>, from: &str, to: &str) -> Vec<u8> {
        assert_eq!(from.len(), to.len());
        let n = from.len();
        for at in 0..zip.len().saturating_sub(n - 1) {
            if &zip[at..at + n] == from.as_bytes() {
                zip[at..at + n].copy_from_slice(to.as_bytes());
            }
        }
        zip
    }

    /// A song whose drum kit's first two pads play `My Samples/kick.wav` and `My Samples/snare.wav`.
    fn song_file() -> Vec<u8> {
        let mut song = Song::default();
        song.add_channel(ChannelKind::Drums);
        let slots = &mut song.channels[1].drum_kit.as_mut().unwrap().slots;
        slots[0].file.path = Some(user("My Samples/kick.wav"));
        slots[1].file.path = Some(user("My Samples/snare.wav"));
        crate::flm::write(&song).unwrap()
    }

    /// The in-memory files of the web (the real `.flm` codec), the provider picking `name` with `bytes`.
    fn web(name: &str, bytes: &[u8]) -> ProjectFiles {
        ProjectFiles::with_provider(Box::new(MemFiles::with_file(name, bytes)))
    }

    fn open(files: &mut ProjectFiles) -> Option<Song> {
        files.open();
        files.poll(&mut Song::default())
    }

    fn lines(lines: &[&str]) -> Option<Vec<String>> {
        Some(lines.iter().map(|l| l.to_string()).collect())
    }

    /// A folder in the temp folder, removed when dropped (and before, if a run that died left it).
    struct Temp(PathBuf);

    impl Temp {
        fn new(name: &str) -> Self {
            Self::in_folder(&std::env::temp_dir(), name)
        }

        fn in_folder(base: &Path, name: &str) -> Self {
            let dir = base.join(format!("honeydew-i1-{name}-{}", std::process::id()));
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

    /// Every file below `dir`, relative to it, sorted.
    #[cfg(not(target_arch = "wasm32"))]
    fn tree(dir: &Path) -> Vec<String> {
        let mut files = Vec::new();
        let mut todo = vec![dir.to_owned()];
        while let Some(at) = todo.pop() {
            for entry in std::fs::read_dir(&at).into_iter().flatten().flatten() {
                let path = entry.path();
                match entry.file_type().unwrap().is_dir() {
                    true => todo.push(path),
                    false => files.push(path.strip_prefix(dir).unwrap().to_string_lossy().into()),
                }
            }
        }
        files.sort();
        files
    }

    const SINE_SF2: &[u8] = include_bytes!("../../crates/audio/tests/fixtures/sine.sf2");

    /// `Unzipped songs/Pack/<relative>` as a song path.
    fn unzipped(relative: &str) -> ContentPath {
        user(&format!("{UNZIPPED_FOLDER}/Pack/{relative}"))
    }

    /// The drum pads' paths.
    fn pads(song: &Song) -> Vec<Option<ContentPath>> {
        let slots = &song.channels[1].drum_kit.as_ref().unwrap().slots;
        slots[..2].iter().map(|s| s.file.path.clone()).collect()
    }

    #[test]
    fn a_single_song_archive_plays_its_own_samples() {
        let beat = song_file();
        let archive = zip_of(&[
            ("Beat.flm", &beat),
            ("My Samples/kick.wav", &wav(3)),
            // A macOS archive's resource fork is no second song.
            ("__MACOSX/._Beat.flm", b"\0\x05\x16\x07"),
        ]);
        let mut files = web("Pack.zip", &archive);
        // The user's own files: one the archive also has, whose archive copy the song plays, and one it lacks.
        files.keep_user_file(Path::new("My Samples/kick.wav"), wav(7));
        files.keep_user_file(Path::new("My Samples/snare.wav"), wav(5));
        let song = open(&mut files).expect("the song opens");
        assert_eq!(files.message, None);
        assert_eq!(song.title, "Pack");
        assert_eq!(files.path, Some(PathBuf::from("Pack.zip")));
        assert!(!files.modified());
        // Extracted whole into `Unzipped songs/<zip name>/`, the folder made for it gone.
        let dir = Path::new(UNZIPPED_FOLDER).join("Pack");
        assert_eq!(files.read_user_file(&dir.join("Beat.flm")).unwrap(), beat);
        assert!(!files.user_file_exists(Path::new("Unzipped songs/.Pack.part")));
        // A path whose file the archive holds leads there; any other stays.
        let pads = pads(&song);
        assert_eq!(
            pads,
            [
                Some(unzipped("My Samples/kick.wav")),
                Some(user("My Samples/snare.wav"))
            ]
        );
        let mut cache = SampleCache::default();
        cache.load(&song, &reader(&files));
        let kit = &song.channels[1].drum_kit.as_ref().unwrap().slots;
        assert_eq!(cache.slot(&kit[0]).map(|s| s.frames()), Some(3));
        assert_eq!(cache.slot(&kit[1]).map(|s| s.frames()), Some(5));
        // No copy of the archive in the library: SONGS lists songs, and its song is no `.zip`.
        assert!(!files.library.exists(Path::new("Pack.zip")));
    }

    #[test]
    fn a_saved_archive_song_still_finds_its_samples() {
        let archive = zip_of(&[("Beat.flm", &song_file()), ("My Samples/kick.wav", &wav(3))]);
        let mut files = web("Pack.zip", &archive);
        let mut song = open(&mut files).unwrap();
        // A quick save writes a song file beside where the archive was, never the archive.
        files.save(&song);
        files.poll(&mut song);
        assert_eq!(files.path, Some(PathBuf::from("Pack.flm")));
        files.replace(Replace::New).unwrap();
        let again = files.replace(Replace::Open("Pack.flm".into()));
        let again = again.expect("the saved song opens");
        assert_eq!(pads(&again)[0], Some(unzipped("My Samples/kick.wav")));
        let mut cache = SampleCache::default();
        cache.load(&again, &reader(&files));
        let kick = &again.channels[1].drum_kit.as_ref().unwrap().slots[0];
        assert_eq!(cache.slot(kick).map(|s| s.frames()), Some(3));
    }

    /// A one-zone `.dwp` whose zone plays `C4.wav` with Attack and Release 0.25 and 0.6.
    fn one_zone_dwp() -> Vec<u8> {
        let record = |tag: u32, data: &[u8]| {
            [
                &tag.to_le_bytes()[..],
                &(data.len() as u64).to_le_bytes(),
                data,
            ]
            .concat()
        };
        let floats = [0.25f32, 1.0, 1.0, 0.6].map(f32::to_le_bytes).concat();
        let zone = record(
            3,
            &[record(0x1F6, b"C4.wav"), record(0x1FD, &floats)].concat(),
        );
        [&b"DwPr"[..], &38u32.to_le_bytes(), &record(1, &zone)].concat()
    }

    /// `id`, its size and `body`.
    fn chunk(id: &[u8; 4], body: &[u8]) -> Vec<u8> {
        [&id[..], &(body.len() as u32).to_le_bytes(), body].concat()
    }

    /// `path` with its length before it.
    fn vstr(path: &[u8]) -> Vec<u8> {
        [&(path.len() as u32).to_le_bytes()[..], path].concat()
    }

    #[test]
    fn the_programs_and_soundfonts_of_an_archive_song_load_from_it() {
        let mut song = Song::default();
        song.add_channel(ChannelKind::Synth(SynthKind::DwSampler));
        let program = chunk(b"PATh", &vstr(b"@L@\0My Presets/DW Sampler/Keys.dwp"));
        song.channels[1].rack.modules[0].file.custom_data = Some([&b"10WD"[..], &program].concat());
        song.add_channel(ChannelKind::Synth(SynthKind::SoundFontPlayer));
        let sf2d = Sf2d {
            path: user("My SoundFonts/Sine.sf2"),
            patch: 0,
        };
        song.channels[2].rack.modules[0].file.custom_data = Some(sf2d.write(None).unwrap());
        let archive = zip_of(&[
            ("Synths.flm", &crate::flm::write(&song).unwrap()),
            ("My Presets/DW Sampler/Keys.dwp", &one_zone_dwp()),
            ("My Presets/DW Sampler/Keys/C4.wav", &wav(6)),
            ("My SoundFonts/Sine.sf2", SINE_SF2),
        ]);
        let mut files = web("Pack.zip", &archive);
        let song = open(&mut files).expect("opens");
        // The program loaded on open from the archive: its zone's Attack and Release.
        let dw = &song.channels[1].rack.modules[0];
        assert_eq!((dw.params[1], dw.params[2]), (0.25, 0.6));
        let keys = unzipped("My Presets/DW Sampler/Keys.dwp");
        let mut cache = SampleCache::default();
        cache.load(&song, &reader(&files));
        let zones = cache
            .program(&keys)
            .expect("the program loads from the archive");
        assert_eq!(cache.get(&zones[0].1).map(|s| s.frames()), Some(6));
        // The SoundFont Player plays the archive's SoundFont, and its patch list reads it too.
        let sf2d = Sf2d::read(
            song.channels[2].rack.modules[0]
                .file
                .custom_data
                .as_deref()
                .unwrap(),
        );
        let sf2d = sf2d.unwrap();
        assert_eq!(
            (sf2d.path.clone(), sf2d.patch),
            (unzipped("My SoundFonts/Sine.sf2"), 0)
        );
        assert!(cache.soundfont(&sf2d.path).is_some());
        assert_eq!(files.soundfont(&sf2d.path).map(|p| p.len()), Ok(1));
    }

    #[test]
    fn relinking_maps_every_sample_source_and_keeps_the_rest_of_each_blob() {
        let map = |p: &ContentPath| match in_user_folder(p)?.to_str()? {
            "b.wav" => None,
            relative => Some(user(&format!("X/{relative}"))),
        };
        let mut song = Song::default();
        song.add_channel(ChannelKind::Drums);
        let channel = &mut song.channels[1];
        let slots = &mut channel.drum_kit.as_mut().unwrap().slots;
        slots[0].file.path = Some(user("a.wav"));
        slots[1].file.path = Some(user("b.wav"));
        let mut clip_sample = crate::song::SamplerSlot::default();
        clip_sample.file.path = Some(user("clip.wav"));
        channel.lanes[0].add_clip(0, 4.0).file.sample = Some(Box::new(clip_sample));
        let zone = |path: &[u8]| {
            let mut body = path.to_vec();
            body.resize(1024, 0);
            chunk(b"ZONE", &[body, vec![7; 41]].concat())
        };
        let floats = [0.5f32, 0.75].map(f32::to_le_bytes).concat();
        let midi = chunk(b"MIDI", &[2, 0]);
        // `PATh` with its floats, a legacy `PATH` without, a `ZONE` to map and one to keep, and the `MIDI`.
        let dw = |program: &[u8], legacy: &[u8], zone_a: &[u8]| {
            let program = chunk(b"PATh", &[vstr(program), floats.clone()].concat());
            [
                &b"10WD"[..],
                &program,
                &chunk(b"PATH", &vstr(legacy)),
                &zone(zone_a),
                &zone(b"@L@\0b.wav"),
                &midi,
            ]
            .concat()
        };
        let sens = chunk(b"SENS", &0.5f32.to_le_bytes());
        let slicer = |path: &[u8]| [sens.clone(), chunk(b"PATH", &vstr(path))].concat();
        let sf2 = |path: &[u8]| chunk(b"SF2D", &[vstr(path), 3i32.to_le_bytes().to_vec()].concat());
        let shape = |path: &[u8]| chunk(b"PATH", &vstr(path));
        // A module that holds no sample keeps a `PATH` it might have.
        let other = slicer(b"@L@\0a.wav");
        let blobs = [
            (
                DW_SAMPLER,
                dw(b"@L@\0Keys.dwp", b"@L@\0Old.dwp", b"@L@\0a.wav"),
            ),
            (SLICER, slicer(b"@L@\0loop.wav")),
            (THREE_OSC, shape(b"@L@\0shape.wav")),
            (SOUNDFONT_PLAYER, sf2(b"@L@\0Piano.sf2")),
            (2, other.clone()),
        ];
        for (type_id, blob) in blobs {
            let id = channel.rack.add(type_id);
            channel.rack.get_mut(id).unwrap().file.custom_data = Some(blob);
        }
        relink(&mut song, &map);
        let channel = &song.channels[1];
        assert_eq!(pads(&song), [Some(user("X/a.wav")), Some(user("b.wav"))]);
        let clip = channel.lanes[0].clips[0].file.sample.as_deref().unwrap();
        assert_eq!(clip.file.path, Some(user("X/clip.wav")));
        let blobs: Vec<_> = (channel.rack.modules.iter().rev().take(5).rev())
            .map(|m| m.file.custom_data.clone().unwrap())
            .collect();
        assert_eq!(
            blobs,
            [
                dw(b"@L@\0X/Keys.dwp", b"@L@\0X/Old.dwp", b"@L@\0X/a.wav"),
                slicer(b"@L@\0X/loop.wav"),
                shape(b"@L@\0X/shape.wav"),
                sf2(b"@L@\0X/Piano.sf2"),
                other,
            ]
        );
        // `ZONE` path must leave a NUL in its 1024 bytes: one that would fill them stays.
        let zone_of = |z: usize| zone(format!("@L@\0{}.wav", "z".repeat(z)).as_bytes());
        // `@L@\0` + `X/` + z + `.wav`: 1024 bytes with 1014, 1023 with 1013.
        assert_eq!(relink_chunks(&zone_of(1014), &[*b"ZONE"], &map), None);
        let relinked = relink_chunks(&zone_of(1013), &[*b"ZONE"], &map).unwrap();
        assert_eq!(
            (relinked.len(), relinked[8 + 1023]),
            (zone_of(1013).len(), 0)
        );
    }

    /// The detail of the message shown, which must name a deep path shortened (a whole one runs to 4 KiB).
    #[cfg(unix)]
    fn short_detail(files: &ProjectFiles, title: &str) {
        let message = files.message.clone().unwrap();
        assert_eq!(message[0], title);
        let detail = &message[1];
        assert!(
            detail.contains('…') && detail.chars().count() < 200,
            "{detail}"
        );
    }

    #[cfg(unix)]
    #[test]
    fn error_messages_name_deep_paths_shortened() {
        // An entry 4 KiB deep: the zip reader takes it, the file system does not (its path is too long).
        let deep = "d/".repeat(2040) + "x.wav";
        let archive = zip_of(&[("Beat.flm", &song_file()), (&deep, b"x")]);
        let home = Temp::new("deep");
        let mut files = ProjectFiles::on_disk(&home.0.join("user"));
        assert_eq!(files.opened(home.0.join("Pack.zip"), archive), None);
        short_detail(&files, DECOMPRESS_FAILED);
        // The import's message: a file that cannot be written (no user folder).
        let mut files = web(
            "Pack.zip",
            &zip_of(&[(&("d/".repeat(100) + "kick.wav"), b"k")]),
        );
        files.user = None;
        open(&mut files);
        let entries = files.importing.take().unwrap();
        files.import_files(entries);
        short_detail(&files, IMPORT_FAILED);
    }

    #[test]
    fn a_deep_path_is_shortened_in_messages() {
        let deep = "a/".repeat(500) + "kick.wav";
        let shown = shown(Path::new(&deep));
        assert_eq!(shown.chars().count(), 121);
        assert!(shown.starts_with("a/a/") && shown.ends_with("a/kick.wav") && shown.contains('…'));
        assert_eq!(
            super::shown(Path::new("My Samples/kick.wav")),
            "My Samples/kick.wav"
        );
    }

    #[test]
    fn a_newer_song_in_an_archive_asks_first_then_opens_extracted() {
        let ctx = eframe::egui::Context::default();
        let archive = zip_of(&[
            ("Beat.flm", &synthetic_file(1, "9.0", "")),
            ("My Samples/kick.wav", &wav(3)),
        ]);
        let mut files = ProjectFiles {
            codec: synthetic_codec(),
            ..web("Pack.zip", &archive)
        };
        let mut song = Song::default();
        files.open();
        assert_eq!(files.update(&ctx, &mut song, false, None), None);
        assert!(
            matches!(
                files.take_overlay(),
                Some(Overlay::Message { tag: NEWER_TAG, .. })
            ),
            "the newer-version question"
        );
        // Nothing is extracted before the answer.
        let kick = Path::new(UNZIPPED_FOLDER).join("Pack/My Samples/kick.wav");
        assert!(!files.user_file_exists(&kick));
        let opened = files.update(&ctx, &mut song, false, Some(Answer::Yes));
        let opened = opened.expect("loads on Yes");
        assert_eq!((opened.title.as_str(), opened.channels.len()), ("Pack", 2));
        assert!(files.user_file_exists(&kick));
        // An FLM2 song is refused as a picked one is.
        let old = zip_of(&[("Old.flm", b"\x99ELM\r\n\x1A\nHEAD")]);
        assert_eq!(files.opened("Old.zip".into(), old), None);
        assert_eq!(
            files.message,
            lines(&[
                crate::project_files::LOAD_FAILED,
                "Unsupported old project format"
            ])
        );
    }

    #[test]
    fn archive_failures_show_their_messages_and_change_nothing() {
        let good = zip_of(&[("Beat.flm", &song_file()), ("My Samples/kick.wav", b"QQQQ")]);
        // A damaged byte: the CRC no longer matches.
        let crc = renamed(good.clone(), "QQQQ", "QQQR");
        let cases: [(&str, Vec<u8>, &str); 4] = [
            ("Junk.zip", b"not an archive".to_vec(), ARCHIVE_FAILED),
            ("Cut.zip", good[..good.len() - 3].to_vec(), ARCHIVE_FAILED),
            ("Crc.zip", crc, DECOMPRESS_FAILED),
            (
                "Bad.zip",
                zip_of(&[("Beat.flm", b"10LF junk")]),
                crate::project_files::LOAD_FAILED,
            ),
        ];
        for (name, bytes, title) in cases {
            let mut files = web(name, &bytes);
            assert_eq!(open(&mut files), None, "{name}");
            let message = files.message.clone().unwrap();
            assert_eq!((message[0].as_str(), message.len()), (title, 2), "{name}");
            assert_eq!(files.path, None, "{name}");
            assert!(files.importing.is_none(), "{name}");
        }
        // Nothing this app opens: a message, no question.
        let mut files = web(
            "Docs.zip",
            &zip_of(&[("readme.txt", b"hi"), (".x.wav", b"")]),
        );
        assert_eq!(open(&mut files), None);
        assert_eq!(files.message, lines(&[NOTHING_KNOWN]));
        assert_eq!(files.waiting(), None);
    }

    #[test]
    fn other_archives_import_their_known_files_on_yes() {
        let ctx = eframe::egui::Context::default();
        let archive = zip_of(&[
            ("My Samples/Kicks/kick.wav", b"kick"),
            ("loops/Loop.WAV", b"loop"),
            ("Hall.flmpst", b"hall"),
            ("My Racks/Bus.flmrack", b"rack"),
            ("Kit.tmpl", b"kit"),
            ("Piano.sf2", b"sf2"),
            ("A.flm", b"song a"),
            ("B.flm", b"song b"),
            ("Tune.mid", b"tune"),
            ("readme.txt", b"no"),
            ("__MACOSX/loops/._Loop.WAV", b"no"),
        ]);
        for yes in [false, true] {
            let mut files = web("Pack.zip", &archive);
            files.keep_user_file(Path::new("My Samples/Loop.WAV"), b"mine".to_vec());
            files.library.store("A.flm", b"my a").unwrap();
            let mut song = Song::default();
            files.open();
            assert_eq!(files.update(&ctx, &mut song, false, None), None);
            assert_eq!(
                files.take_overlay(),
                Some(Overlay::Message {
                    tag: IMPORT_TAG,
                    lines: vec!["Import 9 files?".into()],
                    confirm: Some(("Yes".into(), "No".into())),
                    extra: None,
                })
            );
            assert_eq!(files.waiting(), Some(IMPORT_TAG));
            let changes = files.library_changes();
            assert_eq!(files.update(&ctx, &mut song, false, Some(yes.into())), None);
            assert_eq!(files.waiting(), None);
            let shown = files.take_overlay();
            let user = |p: &str| files.read_user_file(Path::new(p)).ok();
            let song_at = |p: &str| files.library.read(Path::new(p)).ok();
            if !yes {
                assert_eq!(shown, None);
                assert_eq!(user("My Samples/Kicks/kick.wav"), None);
                assert_eq!(song_at("B.flm"), None);
                continue;
            }
            assert_eq!(
                shown,
                Some(Overlay::Message {
                    tag: MESSAGE_TAG,
                    lines: [
                        "Imported 9 files",
                        "2 samples in My Samples",
                        "1 MIDI file in My MIDI",
                        "2 songs in My Songs",
                        "1 SoundFont in My SoundFonts",
                        "1 preset in My Presets",
                        "1 rack in My Racks",
                        "1 drumset in My Drumsets",
                    ]
                    .map(String::from)
                    .to_vec(),
                    confirm: None,
                    extra: None,
                })
            );
            assert!(files.library_changes() > changes);
            let mine: [(&str, &[u8]); 9] = [
                ("My Samples/Kicks/kick.wav", b"kick"),
                ("My Samples/Loop.WAV", b"mine"),
                ("My Samples/Loop (2).WAV", b"loop"),
                ("My Presets/Hall.flmpst", b"hall"),
                ("My Racks/Bus.flmrack", b"rack"),
                ("My Drumsets/Kit.tmpl", b"kit"),
                ("My SoundFonts/Piano.sf2", b"sf2"),
                ("My MIDI/Tune.mid", b"tune"),
                ("My Samples/._Loop.WAV", b""),
            ];
            for (path, bytes) in &mine[..8] {
                assert_eq!(user(path).as_deref(), Some(*bytes), "{path}");
            }
            assert_eq!(user(mine[8].0), None, "a resource fork is no sample");
            // Songs go to the songs, a clash taking a free name.
            assert_eq!(song_at("A.flm").as_deref(), Some(&b"my a"[..]));
            assert_eq!(song_at("A (2).flm").as_deref(), Some(&b"song a"[..]));
            assert_eq!(song_at("B.flm").as_deref(), Some(&b"song b"[..]));
            assert_eq!(song.channels.len(), 1, "the song stays");
        }
        // One file: the plural only where it belongs.
        let mut files = web("One.zip", &zip_of(&[("kick.wav", b"k")]));
        open(&mut files);
        assert_eq!(files.message, lines(&["Import 1 files?"]));
        let entries = files.importing.take().unwrap();
        files.import_files(entries);
        assert_eq!(
            files.message,
            lines(&["Imported 1 file", "1 sample in My Samples"])
        );
    }

    /// On disk, an archive that is no single song sorts its files into the user folder's own folders by type, a taken
    /// name numbered from 2, and sums up where they went; one with a name that leaves the folder imports nothing. A
    /// DirectWave program takes the audio files of its folder along into `My Instruments`, into the folder named after
    /// the name it takes, where it plays them (`sample_cache::zone_file`).
    #[cfg(not(target_arch = "wasm32"))]
    #[test]
    fn an_archive_sorts_its_files_into_the_user_folders_by_type() {
        let home = Temp::new("import-sort");
        let (user, songs, mut files) = unzip_on_disk(&home.0);
        files.root = Some(songs.clone());
        std::fs::write(songs.join("A.flm"), b"mine").unwrap();
        std::fs::create_dir_all(user.join("My Samples")).unwrap();
        std::fs::write(user.join("My Samples/kick.wav"), b"my kick").unwrap();
        // The user's own folder of that name: the program takes another name, and the folder of it.
        std::fs::create_dir_all(user.join("My Instruments/Keys")).unwrap();
        std::fs::write(user.join("My Instruments/Keys/C4.wav"), b"mine").unwrap();
        let mine = tree(&user);
        // A name that leaves the folder (`../`; `renamed` keeps the length) refuses the archive whole.
        let slip = zip_of(&[("A.flm", b"a"), ("B.flm", b"b"), ("xx/evil.wav", b"evil")]);
        assert!(
            files
                .unzip(&renamed(slip, "xx/evil.wav", "../evil.wav"))
                .is_none()
        );
        assert_eq!(files.message.take().unwrap()[0], DECOMPRESS_FAILED);
        assert_eq!(tree(&user), mine);

        let archive = zip_of(&[
            ("Pack/kick.wav", b"kick"),
            ("Pack/Tune.mid", b"tune"),
            ("Pack/A.flm", b"song a"),
            ("Pack/B.flm", b"song b"),
            ("Keys/Keys.dwp", &one_zone_dwp()),
            ("Keys/C4.wav", &wav(6)),
            ("Pack/Piano.sf2", b"sf2"),
            ("Pack/Hall.flmpst", b"hall"),
        ]);
        let entries = files.unzip(&archive).unwrap();
        files.ask_import(entries);
        assert_eq!(files.message.take(), lines(&["Import 8 files?"]));
        let entries = files.importing.take().unwrap();
        files.import_files(entries);
        assert_eq!(
            files.message,
            lines(&[
                "Imported 8 files",
                "1 sample in My Samples",
                "1 MIDI file in My MIDI",
                "2 songs in My Songs",
                "1 instrument (with 1 sample) in My Instruments",
                "1 SoundFont in My SoundFonts",
                "1 preset in My Presets",
            ])
        );
        let read = |path: &str| std::fs::read(user.join(path)).unwrap();
        let imported: [(&str, &[u8]); 11] = [
            ("My Samples/kick.wav", b"my kick"),
            ("My Samples/kick (2).wav", b"kick"),
            ("My MIDI/Tune.mid", b"tune"),
            ("My Songs/A.flm", b"mine"),
            ("My Songs/A (2).flm", b"song a"),
            ("My Songs/B.flm", b"song b"),
            ("My Instruments/Keys/C4.wav", b"mine"),
            ("My Instruments/Keys (2).dwp", &one_zone_dwp()),
            ("My Instruments/Keys (2)/C4.wav", &wav(6)),
            ("My SoundFonts/Piano.sf2", b"sf2"),
            ("My Presets/Hall.flmpst", b"hall"),
        ];
        for (path, bytes) in imported {
            assert_eq!(read(path), bytes, "{path}");
        }
        assert_eq!(
            tree(&user).len(),
            imported.len(),
            "{:?} (mine: {mine:?})",
            tree(&user)
        );
        // A DW Sampler on the imported program plays its sample.
        let mut song = Song::default();
        song.add_channel(ChannelKind::Synth(SynthKind::DwSampler));
        let program = chunk(b"PATh", &vstr(b"@L@\0My Instruments/Keys (2).dwp"));
        song.channels[1].rack.modules[0].file.custom_data = Some([&b"10WD"[..], &program].concat());
        let mut cache = SampleCache::default();
        cache.load(&song, &reader(&files));
        let keys = ContentPath::Shorthand {
            root: ContentRoot::User,
            relative: "My Instruments/Keys (2).dwp".into(),
        };
        let zones = cache.program(&keys).expect("the program loads");
        assert_eq!(cache.get(&zones[0].1).map(|s| s.frames()), Some(6));
    }

    #[test]
    fn a_failed_import_says_so() {
        // Without a user folder nothing can be written.
        let mut files = web("Pack.zip", &zip_of(&[("kick.wav", b"k")]));
        files.user = None;
        open(&mut files);
        let entries = files.importing.take().unwrap();
        files.import_files(entries);
        let message = files.message.clone().unwrap();
        assert_eq!((message[0].as_str(), message.len()), (IMPORT_FAILED, 2));
    }

    /// A web-style files holder whose library (the one SONGS lists) holds the archive `Pack.zip`.
    fn with_archive(bytes: &[u8]) -> ProjectFiles {
        let mut files = ProjectFiles::with_provider(Box::new(MemFiles::default()));
        let mut library = crate::ui::main_menu::MemoryLibrary::default();
        library.put(PathBuf::from("Pack.zip"), bytes).unwrap();
        files.library = Box::new(library);
        files
    }

    #[test]
    fn unzip_extracts_every_file_into_unzipped_songs_without_opening_the_song() {
        let beat = song_file();
        let archive = zip_of(&[
            ("Beat.flm", &beat),
            ("My Samples/kick.wav", &wav(3)),
            ("readme.txt", b"hi"),
        ]);
        let mut files = with_archive(&archive);
        files.keep_user_file(Path::new("My Samples/mine.wav"), wav(9));
        let changes = files.library_changes();
        files.unzip_archive(Path::new("Pack.zip"));
        // Everything the archive holds, the unknown file too, below `Unzipped songs/<zip name>/`.
        let dir = Path::new(UNZIPPED_FOLDER).join("Pack");
        assert_eq!(files.read_user_file(&dir.join("Beat.flm")).unwrap(), beat);
        assert_eq!(
            files.read_user_file(&dir.join("readme.txt")).unwrap(),
            b"hi"
        );
        assert_eq!(
            files
                .read_user_file(&dir.join("My Samples/kick.wav"))
                .unwrap(),
            wav(3)
        );
        assert!(!files.user_file_exists(Path::new("Unzipped songs/.Pack.part")));
        // The user's own files and the library stay as they were, the song is not opened, and SONGS reads again.
        assert!(files.user_file_exists(Path::new("My Samples/mine.wav")));
        assert!(!files.user_file_exists(Path::new("My Samples/kick.wav")));
        assert_eq!(files.library.list(None).unwrap().len(), 1);
        assert_eq!((files.path.clone(), files.message.clone()), (None, None));
        assert!(!files.modified());
        assert!(files.library_changes() > changes);
        assert_eq!(
            files.take_flash().as_deref(),
            Some("Unzipped to \"Unzipped songs/Pack\"")
        );
        // Again: the same folder is made new, nothing of the first left.
        let second = zip_of(&[("Beat.flm", &beat)]);
        files.library = {
            let mut library = crate::ui::main_menu::MemoryLibrary::default();
            library.put(PathBuf::from("Pack.zip"), &second).unwrap();
            Box::new(library)
        };
        files.unzip_archive(Path::new("Pack.zip"));
        assert!(files.user_file_exists(&dir.join("Beat.flm")));
        assert!(!files.user_file_exists(&dir.join("readme.txt")));
        // The first is in the session's bin, at its place: nothing the user could change there is lost. It went in
        // once the new files were all written (copied after them), so a failure on the way would have left it.
        let bin = Path::new("/Recently Deleted").join(&dir);
        assert_eq!(
            files.read_user_file(&bin.join("readme.txt")).unwrap(),
            b"hi"
        );
        assert_eq!(files.message, None);
        assert_eq!(
            files.take_flash().as_deref(),
            Some(
                "Unzipped to \"Unzipped songs/Pack\"\n\
                 The earlier \"Unzipped songs/Pack\" is in \"Recently Deleted\""
            )
        );
        let recency = |dir: &Path, name: &str| {
            let listed = files.user_copies.list(Some(dir)).unwrap();
            listed.into_iter().find(|e| e.name == name).unwrap().recency
        };
        assert!(recency(&bin, "readme.txt") > recency(&dir, "Beat.flm"));
        // The same archive again: the folder holds its files already, so it stays and nothing more goes into the bin.
        files.unzip_archive(Path::new("Pack.zip"));
        assert!(files.user_file_exists(&dir.join("Beat.flm")));
        assert!(!files.user_file_exists(&bin.with_file_name("Pack (1)")));
        assert_eq!(
            files.take_flash().as_deref(),
            Some("Unzipped to \"Unzipped songs/Pack\"")
        );
    }

    /// Extracting over an earlier extraction (`Unzip` again, or the archive opened again) never loses what the user
    /// changed there (SONGS lists `Unzipped songs`, so its folders are the user's to browse and edit): the
    /// earlier folder goes into the bin at its place, as `Delete` puts it, a taken name
    /// numbered, and the archive's files take its place. A file added, changed (to
    /// other bytes of the same size, too) or gone is a change.
    #[cfg(not(target_arch = "wasm32"))]
    #[test]
    fn extracting_over_a_changed_folder_puts_it_in_the_bin() {
        let home = Temp::new("rebin");
        let (user, songs, mut files) = unzip_on_disk(&home.0);
        let beat = song_file();
        // A hidden entry too, which the listing leaves out: a file added makes as many listed files as entries.
        let archive = zip_of(&[
            ("Beat.flm", &beat),
            ("My Samples/kick.wav", &wav(3)),
            (".DS_Store", b"ds"),
        ]);
        std::fs::write(songs.join("Pack.zip"), &archive).unwrap();
        files.unzip_archive(&songs.join("Pack.zip"));
        let dir = user.join(UNZIPPED_FOLDER).join("Pack");
        let bin = user.join("Recently Deleted").join(UNZIPPED_FOLDER);
        assert_eq!(
            files.take_flash().as_deref(),
            Some("Unzipped to \"Unzipped songs/Pack\"")
        );
        // A file added, nothing else changed: the flash says where the earlier folder went.
        std::fs::write(dir.join("Mine.flm"), b"mine").unwrap();
        files.unzip_archive(&songs.join("Pack.zip"));
        assert_eq!(files.message, None);
        assert_eq!(tree(&dir), [".DS_Store", "Beat.flm", "My Samples/kick.wav"]);
        assert_eq!(std::fs::read(dir.join("Beat.flm")).unwrap(), beat);
        assert_eq!(std::fs::read(bin.join("Pack/Mine.flm")).unwrap(), b"mine");
        assert_eq!(std::fs::read(bin.join("Pack/Beat.flm")).unwrap(), beat);
        assert_eq!(
            files.take_flash().as_deref(),
            Some(
                "Unzipped to \"Unzipped songs/Pack\"\n\
                 The earlier \"Unzipped songs/Pack\" is in \"Recently Deleted\""
            )
        );
        // Other bytes of the same size, then opened again: in once more, under a free name, and the flash says so.
        let mut kick = wav(3);
        *kick.last_mut().unwrap() ^= 1;
        std::fs::write(dir.join("My Samples/kick.wav"), &kick).unwrap();
        files
            .opened(songs.join("Pack.zip"), archive.clone())
            .expect("opens");
        assert_eq!(
            files.take_flash().as_deref(),
            Some("The earlier \"Unzipped songs/Pack\" is in \"Recently Deleted\"")
        );
        assert_eq!(
            std::fs::read(dir.join("My Samples/kick.wav")).unwrap(),
            wav(3)
        );
        assert_eq!(
            std::fs::read(bin.join("Pack (1)/My Samples/kick.wav")).unwrap(),
            kick
        );
        // A file gone.
        std::fs::remove_file(dir.join("My Samples/kick.wav")).unwrap();
        files.unzip_archive(&songs.join("Pack.zip"));
        assert_eq!(tree(&dir), [".DS_Store", "Beat.flm", "My Samples/kick.wav"]);
        assert_eq!(tree(&bin.join("Pack (2)")), [".DS_Store", "Beat.flm"]);
    }

    /// A folder that still holds exactly the archive's files is what extracting it again would make: it is kept as it
    /// is, and nothing goes into the bin (an archive song opened every day leaves no copy a day there).
    #[cfg(not(target_arch = "wasm32"))]
    #[test]
    fn extracting_over_an_unchanged_folder_keeps_it_and_bins_nothing() {
        let home = Temp::new("rebin-same");
        let (user, songs, mut files) = unzip_on_disk(&home.0);
        // Hidden entries too, which SONGS never lists (a Mac's `.DS_Store`, `__MACOSX/._*`): read, not listed.
        let archive = zip_of(&[
            ("Beat.flm", &song_file()),
            ("My Samples/kick.wav", &wav(3)),
            (".DS_Store", b"ds"),
            ("__MACOSX/._Beat.flm", b"fork"),
        ]);
        std::fs::write(songs.join("Pack.zip"), &archive).unwrap();
        files.unzip_archive(&songs.join("Pack.zip"));
        let dir = user.join(UNZIPPED_FOLDER).join("Pack");
        let before = tree(&dir);
        let kick = dir.join("My Samples/kick.wav");
        let written = std::fs::metadata(&kick).unwrap().modified().unwrap();
        files.unzip_archive(&songs.join("Pack.zip"));
        files
            .opened(songs.join("Pack.zip"), archive)
            .expect("opens");
        assert_eq!((files.message.clone(), tree(&dir)), (None, before));
        assert_eq!(
            std::fs::metadata(&kick).unwrap().modified().unwrap(),
            written
        );
        assert!(!user.join("Recently Deleted").exists());
        assert_eq!(
            files.take_flash().as_deref(),
            Some("Unzipped to \"Unzipped songs/Pack\"")
        );
        // A hidden entry changed is a change too: its bytes are read.
        std::fs::write(dir.join(".DS_Store"), b"dz").unwrap();
        files.unzip_archive(&songs.join("Pack.zip"));
        assert_eq!(std::fs::read(dir.join(".DS_Store")).unwrap(), b"ds");
        assert!(user.join("Recently Deleted/Unzipped songs/Pack").is_dir());
    }

    /// A folder that holds the archive's files, reached through a link, is never kept: one at `Unzipped songs/Pack` is
    /// replaced by a folder of the extraction's own (what it points at untouched), and with `Unzipped songs` a link
    /// the archive does not open.
    #[cfg(unix)]
    #[test]
    fn a_matching_folder_behind_a_link_is_never_kept() {
        let home = Temp::new("rebin-link");
        let (user, outside) = (home.0.join("user"), home.0.join("outside"));
        let archive = zip_of(&[("Beat.flm", &song_file()), ("My Samples/kick.wav", &wav(3))]);
        // The archive's files, outside the user folder.
        let mut files = ProjectFiles::on_disk(&outside);
        files
            .opened(home.0.join("Pack.zip"), archive.clone())
            .expect("opens");
        let theirs = outside.join(UNZIPPED_FOLDER);
        let before = tree(&theirs);
        std::fs::create_dir_all(user.join(UNZIPPED_FOLDER)).unwrap();
        let pack = user.join(UNZIPPED_FOLDER).join("Pack");
        std::os::unix::fs::symlink(theirs.join("Pack"), &pack).unwrap();
        let mut files = ProjectFiles::on_disk(&user);
        files
            .opened(home.0.join("Pack.zip"), archive.clone())
            .expect("opens");
        assert!(!std::fs::symlink_metadata(&pack).unwrap().is_symlink());
        assert_eq!(tree(&pack), ["Beat.flm", "My Samples/kick.wav"]);
        assert_eq!(tree(&theirs), before);
        // The link was removed, not binned: the flash says nothing of the bin.
        assert_eq!(files.take_flash(), None);
        assert!(!user.join("Recently Deleted").exists());
        // `Unzipped songs` itself a link to a folder holding a matching `Pack`: refused.
        std::fs::remove_dir_all(user.join(UNZIPPED_FOLDER)).unwrap();
        std::os::unix::fs::symlink(&theirs, user.join(UNZIPPED_FOLDER)).unwrap();
        let mut files = ProjectFiles::on_disk(&user);
        assert_eq!(files.opened(home.0.join("Pack.zip"), archive), None);
        assert_eq!(files.message.clone().unwrap()[0], DECOMPRESS_FAILED);
        assert_eq!(tree(&theirs), before);
    }

    #[test]
    fn unzip_that_fails_says_why_and_changes_nothing() {
        let good = zip_of(&[("Beat.flm", &song_file()), ("My Samples/kick.wav", b"QQQQ")]);
        let crc = renamed(good.clone(), "QQQQ", "QQQR");
        let cases: [(&[u8], &str); 3] = [
            (b"not an archive", ARCHIVE_FAILED),
            (&good[..good.len() - 3], ARCHIVE_FAILED),
            (&crc, DECOMPRESS_FAILED),
        ];
        for (bytes, title) in cases {
            let mut files = with_archive(bytes);
            let changes = files.library_changes();
            files.unzip_archive(Path::new("Pack.zip"));
            let message = files.message.clone().unwrap();
            assert_eq!((message[0].as_str(), message.len()), (title, 2), "{title}");
            assert_eq!(
                (files.library_changes(), files.take_flash()),
                (changes, None)
            );
            assert!(!files.user_file_exists(Path::new("Unzipped songs/Pack/Beat.flm")));
        }
        // An archive that is gone (removed meanwhile) is the read failure, naming it.
        let mut files = with_archive(&good);
        files.unzip_archive(Path::new("Gone.zip"));
        let message = files.message.clone().unwrap();
        assert_eq!(message[0], crate::project_files::READ_FAILED);
        assert!(message[1].starts_with("Gone.zip: "), "{message:?}");
        // Without a user folder nothing can be written: the decompress failure, with the reason.
        let mut files = with_archive(&good);
        files.user = None;
        files.unzip_archive(Path::new("Pack.zip"));
        let message = files.message.clone().unwrap();
        assert_eq!(message[0], DECOMPRESS_FAILED);
        assert_eq!((message.len(), files.take_flash()), (2, None));
    }

    /// `Unzip` on disk: the songs folder `<home>/a/user/My Songs`, as the library of a files holder whose user folder is
    /// `<home>/a/user`. Returns the user folder, the songs folder and the holder.
    #[cfg(not(target_arch = "wasm32"))]
    fn unzip_on_disk(home: &Path) -> (PathBuf, PathBuf, ProjectFiles) {
        use crate::ui::main_menu::{NativeLibrary, SONGS_FOLDER};
        let user = home.join("a").join("user");
        let songs = user.join(SONGS_FOLDER);
        std::fs::create_dir_all(&songs).unwrap();
        let mut files = ProjectFiles::on_disk(&user);
        files.library = Box::new(NativeLibrary::at(songs.clone()));
        (user, songs, files)
    }

    /// A hostile entry name makes the archive unreadable, so that `Unzip` writes nothing, not even beside the user
    /// folder, where `../` would lead.
    #[cfg(not(target_arch = "wasm32"))]
    #[test]
    fn unzip_refuses_zip_slip_names_and_writes_nothing() {
        let home = Temp::new("unzip-slip");
        let (_, songs, mut files) = unzip_on_disk(&home.0);
        let hostile = [
            "../evil.wav",
            "/x/evil.wav",
            "..\\evil.wav",
            "C:/evil.wav",
            "x/../../e.w",
            "\\\\x\\evil.wa",
            "a/C:evil.wa",
            "x/./../evil",
            "..\\..\\e.wav",
        ];
        for name in hostile {
            // `renamed` keeps the length of the name it replaces.
            assert_eq!(name.len(), "xx/evil.wav".len(), "{name}");
            let archive = zip_of(&[("Beat.flm", &song_file()), ("xx/evil.wav", b"evil")]);
            std::fs::write(
                songs.join("Pack.zip"),
                renamed(archive, "xx/evil.wav", name),
            )
            .unwrap();
            files.message = None;
            files.unzip_archive(&songs.join("Pack.zip"));
            let message = files.message.clone().unwrap();
            assert_eq!(message[0], DECOMPRESS_FAILED, "{name}");
            assert_eq!(tree(&home.0), ["a/user/My Songs/Pack.zip"], "{name}");
            assert_eq!(files.take_flash(), None, "{name}");
        }
    }

    #[cfg(not(target_arch = "wasm32"))]
    #[test]
    fn on_disk_unzip_reads_the_archive_from_the_songs_folder() {
        use crate::ui::main_menu::NativeLibrary;
        let home = Temp::new("unzip");
        let user = home.0.join("user");
        let songs = user.join(crate::ui::main_menu::SONGS_FOLDER);
        std::fs::create_dir_all(&songs).unwrap();
        std::fs::write(
            songs.join("Pack.zip"),
            zip_of(&[("Beat.flm", &song_file()), ("My Samples/kick.wav", &wav(3))]),
        )
        .unwrap();
        let mut files = ProjectFiles::on_disk(&user);
        files.library = Box::new(NativeLibrary::at(songs.clone()));
        files.unzip_archive(&songs.join("Pack.zip"));
        assert_eq!(files.message, None);
        assert_eq!(
            tree(&user.join(UNZIPPED_FOLDER).join("Pack")),
            ["Beat.flm", "My Samples/kick.wav"]
        );
        // The archive itself is untouched, and nothing else was made in the songs folder.
        assert_eq!(tree(&songs), ["Pack.zip"]);
        assert!(files.take_flash().is_some());
    }

    // ---- .mid ----

    fn note_on(ch: u8, key: u8) -> crate::midi::MidiMsg {
        crate::midi::MidiMsg::NoteOn { ch, key, vel: 100 }
    }

    fn smf(track: &[(u64, SmfEvent)]) -> Vec<u8> {
        crate::smf::codec::write(&Smf {
            division: 96,
            tracks: vec![track.to_vec()],
        })
    }

    #[test]
    fn a_mid_file_opens_as_a_new_untitled_song_with_a_channel_per_midi_channel() {
        let tune = smf(&[
            (0, SmfEvent::Name(b"Lead".to_vec())),
            // 500 000 µs a beat: 120 BPM.
            (0, SmfEvent::Tempo(500_000)),
            (0, SmfEvent::Voice(note_on(0, 60))),
            (96, SmfEvent::Voice(note_on(9, 36))),
            (192, SmfEvent::Voice(note_on(3, 64))),
        ]);
        let mut files = web("Tune.mid", &tune);
        let mut song = open(&mut files).expect("the file opens");
        // One summary of the sound each channel got.
        assert_eq!(
            files.message.as_ref().map(|m| (m[0].as_str(), m.len())),
            Some((crate::project_files::STAND_INS, 4))
        );
        assert_eq!(song.title, "Tune");
        assert_eq!(song.tempo_bpm, 120.0);
        let names: Vec<_> = song.channels.iter().map(|c| c.name.as_str()).collect();
        assert_eq!(
            names,
            [
                song.channels[0].name.as_str(),
                "Lead (MIDI Channel 0: Acoustic Grand Piano)",
                "Lead (MIDI Channel 9: Drums)",
                "Lead (MIDI Channel 3: Acoustic Grand Piano)",
            ]
        );
        assert_eq!(song.channels[0].kind, ChannelKind::Master);
        assert!(!song.file.meta.created.is_empty());
        // Untitled: no copy kept, no path, so the first save asks; the import is no unsaved change.
        assert_eq!(files.path, None);
        assert!(!files.library.exists(Path::new("Tune.mid")));
        assert!(!files.modified());
        files.save(&song);
        assert_eq!(
            files.take_overlay(),
            Some(Overlay::SaveDialog {
                name: "Tune".into()
            })
        );
        files.poll(&mut song);

        // No tempo in the file: the MIDI default of 120 BPM.
        let mut files = web("Slow.MIDI", &smf(&[(0, SmfEvent::Voice(note_on(0, 60)))]));
        assert_eq!(open(&mut files).unwrap().tempo_bpm, 120.0);
    }

    #[test]
    fn a_mid_file_that_does_not_read_shows_why() {
        let tune = smf(&[(0, SmfEvent::Voice(note_on(0, 60)))]);
        // The MIDI reader: a last track cut short is "truncated".
        let mut files = web("Cut.mid", &tune[..tune.len() - 2]);
        let mut song = Song {
            title: "Kept".into(),
            ..Song::default()
        };
        assert_eq!(open(&mut files), None);
        assert_eq!(
            files.message,
            lines(&[crate::smf::import::LOAD_FAILED, "truncated"])
        );
        assert_eq!(files.path, None);
        files.poll(&mut song);
        assert_eq!(song.title, "Kept");
    }

    // ---- on disk ----

    #[cfg(not(target_arch = "wasm32"))]
    #[test]
    fn on_disk_the_archive_is_extracted_into_unzipped_songs_and_replaces_the_last() {
        let home = Temp::new("extract");
        let root = home.0.join("user");
        let mut files = ProjectFiles::on_disk(&root);
        let first = zip_of(&[
            ("Beat.flm", &song_file()),
            ("My Samples/kick.wav", &wav(3)),
            ("My Samples/old.wav", &wav(1)),
        ]);
        let song = files.opened(home.0.join("Pack.zip"), first).expect("opens");
        let dir = root.join(UNZIPPED_FOLDER).join("Pack");
        assert_eq!(
            tree(&dir),
            ["Beat.flm", "My Samples/kick.wav", "My Samples/old.wav"]
        );
        let mut cache = SampleCache::default();
        cache.load(&song, &reader(&files));
        let kick = &song.channels[1].drum_kit.as_ref().unwrap().slots[0];
        assert_eq!(cache.slot(kick).map(|s| s.frames()), Some(3));
        assert_eq!(files.path, Some(home.0.join("Pack.zip")));
        // An archive of the same name: its files only, nothing left of the first.
        let second = zip_of(&[("Beat.flm", &song_file()), ("My Samples/kick.wav", &wav(4))]);
        files
            .opened(home.0.join("other").join("Pack.zip"), second)
            .unwrap();
        assert_eq!(tree(&dir), ["Beat.flm", "My Samples/kick.wav"]);
        assert_eq!(
            reader(&files)(&unzipped("My Samples/kick.wav")),
            Some(wav(4))
        );
        // A name that makes no folder, or one a staging folder could have: `Unzipped songs/Archive`.
        let dots = zip_of(&[("Beat.flm", &song_file())]);
        files
            .opened(home.0.join(".Pack.part.zip"), dots.clone())
            .unwrap();
        assert_eq!(
            tree(&root.join(UNZIPPED_FOLDER).join(UNNAMED)),
            ["Beat.flm"]
        );
        files.opened(home.0.join("..zip"), dots).unwrap();
        assert_eq!(
            tree(&root.join(UNZIPPED_FOLDER).join(UNNAMED)),
            ["Beat.flm"]
        );
        assert_eq!(tree(&dir), ["Beat.flm", "My Samples/kick.wav"]);
    }

    #[cfg(not(target_arch = "wasm32"))]
    #[test]
    fn a_failed_extraction_leaves_the_open_songs_folder_as_it_was() {
        let home = Temp::new("staging");
        let root = home.0.join("user");
        let mut files = ProjectFiles::on_disk(&root);
        let good = zip_of(&[("Beat.flm", &song_file()), ("My Samples/kick.wav", &wav(3))]);
        files.opened(home.0.join("Pack.zip"), good).expect("opens");
        let top = root.join(UNZIPPED_FOLDER);
        let before = tree(&top);
        assert_eq!(before, ["Pack/Beat.flm", "Pack/My Samples/kick.wav"]);
        // An archive of the same name whose files cannot all be written: a file and a folder named alike.
        let clash = zip_of(&[("Beat.flm", &song_file()), ("a", b"file"), ("a/b", b"in a")]);
        assert_eq!(files.opened(home.0.join("x").join("Pack.zip"), clash), None);
        assert_eq!(files.message.clone().unwrap()[0], DECOMPRESS_FAILED);
        assert_eq!(tree(&top), before, "the open song's files are all there");
        assert!(
            !top.join(".Pack.part").exists(),
            "the half-written folder is gone"
        );
        // A folder a crash left half-written is no obstacle.
        std::fs::create_dir_all(top.join(".Pack.part/My Samples")).unwrap();
        std::fs::write(top.join(".Pack.part/stale.wav"), b"stale").unwrap();
        let again = zip_of(&[("Beat.flm", &song_file()), ("My Samples/kick.wav", &wav(4))]);
        files.opened(home.0.join("Pack.zip"), again).expect("opens");
        assert_eq!(tree(&top), before);
    }

    #[cfg(not(target_arch = "wasm32"))]
    #[test]
    fn extracting_costs_about_what_writing_the_files_plainly_does() {
        // On disk beside the test binary, where syncing each file would show (a temp folder may be in memory).
        let exe = std::env::current_exe().unwrap();
        let home = Temp::in_folder(exe.parent().unwrap(), "speed");
        // Enough files that a sync each (1–20 ms on the disks seen) is seconds past the bound, even while the plain writes
        // are slowed by load.
        let entries: Vec<Entry> = (0..5000)
            .map(|i| Entry {
                path: format!("My Samples/{}/{i}.wav", i % 50),
                data: vec![i as u8; 16],
            })
            .collect();
        // The fastest of three runs a side, taken in turns and into new folders, so that load on the machine weighs
        // little.
        let timed = |run: &mut dyn FnMut()| {
            let start = std::time::Instant::now();
            run();
            start.elapsed()
        };
        let mut files = ProjectFiles::on_disk(&home.0.join("user"));
        let (mut plain, mut extract) = (std::time::Duration::MAX, std::time::Duration::MAX);
        for k in 0..3 {
            // The yardstick: the same files, each written as `std::fs::write` does.
            let written = timed(&mut || {
                for entry in &entries {
                    let path = home.0.join(format!("plain {k}")).join(&entry.path);
                    std::fs::create_dir_all(path.parent().unwrap()).unwrap();
                    std::fs::write(path, &entry.data).unwrap();
                }
            });
            let zip = format!("Pack {k}.zip");
            let extracted = timed(&mut || {
                files.extract(Path::new(&zip), &entries).unwrap();
            });
            (plain, extract) = (plain.min(written), extract.min(extracted));
        }
        let dir = home.0.join("user").join(UNZIPPED_FOLDER).join("Pack 2");
        assert_eq!(tree(&dir).len(), 5000);
        let bound = plain * 10 + std::time::Duration::from_secs(1);
        assert!(
            extract < bound,
            "{extract:?} to extract, {plain:?} to write"
        );
    }

    #[cfg(not(target_arch = "wasm32"))]
    #[test]
    fn zip_slip_never_writes_outside_the_extracted_folder() {
        let home = Temp::new("slip");
        let user = home.0.join("a").join("user");
        std::fs::create_dir_all(&user).unwrap();
        // Each hostile name has the length of `xx/evil.wav`, which the writer takes.
        let hostile = [
            "../evil.wav",
            "/x/evil.wav",
            "..\\evil.wav",
            "C:/evil.wav",
            "x/../../e.w",
            "\\\\x\\evil.wa",
            "a/C:evil.wa",
            "x/./../evil",
        ];
        for name in hostile {
            let archive = zip_of(&[("Beat.flm", &song_file()), ("xx/evil.wav", b"evil")]);
            let archive = renamed(archive, "xx/evil.wav", name);
            let mut files = ProjectFiles::on_disk(&user);
            assert_eq!(
                files.opened(home.0.join("Pack.zip"), archive),
                None,
                "{name}"
            );
            let message = files.message.clone().unwrap();
            assert_eq!(message[0], DECOMPRESS_FAILED, "{name}");
            assert_eq!(tree(&home.0), Vec::<String>::new(), "{name}");
        }
        // A link in the archive is refused, and so is a file and a folder of one name.
        let link = zip_of(&[("Beat.flm", &song_file()), ("link.wav", b"/etc/passwd")]);
        let mode = |m: u32| (m << 16).to_le_bytes();
        let at = (link.windows(4).rposition(|w| w == mode(0o100644)))
            .expect("the central directory's file mode");
        let mut link = link;
        link[at..at + 4].copy_from_slice(&mode(0o120777));
        let clash = zip_of(&[("Beat.flm", &song_file()), ("a", b"file"), ("a/b", b"in a")]);
        for (what, archive) in [("link", link), ("clash", clash)] {
            let mut files = ProjectFiles::on_disk(&user);
            assert_eq!(
                files.opened(home.0.join("Pack.zip"), archive),
                None,
                "{what}"
            );
            assert_eq!(
                files.message.clone().unwrap()[0],
                DECOMPRESS_FAILED,
                "{what}"
            );
        }
        assert_eq!(tree(&home.0), Vec::<String>::new());
    }

    #[cfg(unix)]
    #[test]
    fn links_on_the_way_never_take_the_extraction_or_its_cleanup_outside() {
        let home = Temp::new("links");
        let (user, outside) = (home.0.join("user"), home.0.join("outside"));
        std::fs::create_dir_all(user.join(UNZIPPED_FOLDER)).unwrap();
        std::fs::create_dir_all(outside.join("Pack")).unwrap();
        std::fs::write(outside.join("Pack/keep.wav"), b"keep").unwrap();
        // `Unzipped songs/Pack` is a link to a folder outside: the link goes, never what it points at.
        let pack = user.join(UNZIPPED_FOLDER).join("Pack");
        std::os::unix::fs::symlink(outside.join("Pack"), &pack).unwrap();
        let archive = zip_of(&[("Beat.flm", &song_file()), ("My Samples/kick.wav", &wav(3))]);
        let mut files = ProjectFiles::on_disk(&user);
        assert!(
            files
                .opened(home.0.join("Pack.zip"), archive.clone())
                .is_some()
        );
        assert!(!std::fs::symlink_metadata(&pack).unwrap().is_symlink());
        assert_eq!(tree(&outside), ["Pack/keep.wav"]);
        // `Unzipped songs` itself a link: refused, nothing removed or written through it.
        std::fs::remove_dir_all(user.join(UNZIPPED_FOLDER)).unwrap();
        std::os::unix::fs::symlink(&outside, user.join(UNZIPPED_FOLDER)).unwrap();
        let mut files = ProjectFiles::on_disk(&user);
        assert_eq!(files.opened(home.0.join("Pack.zip"), archive), None);
        assert_eq!(files.message.clone().unwrap()[0], DECOMPRESS_FAILED);
        assert_eq!(tree(&outside), ["Pack/keep.wav"]);
    }

    #[test]
    fn hostile_archives_never_panic() {
        let archive = zip_of(&[("Beat.flm", &song_file()), ("My Samples/kick.wav", &wav(3))]);
        for cut in 0..archive.len() {
            let mut files = web("Pack.zip", &archive[..cut]);
            assert_eq!(open(&mut files), None, "{cut}");
        }
    }
}
