//! The file actions of SONGS and the preset pickers (the ≡ options menu; [`FileActions`]) on a [`SongLibrary`]: make a
//! folder, delete into [`RECENTLY_DELETED`] (a copy at the same place below the user folder), restore from it, delete
//! for good, paste what was copied or cut, rename, export, show in the system's file manager, and convert one WAV to
//! FLAC beside it.
//!
//! Nothing is ever replaced. A paste, a delete, a restore and an export take a free `Name (n)` where the name is
//! taken (as `New Song (1)`); a name the user typed that is taken is refused with a message. Links are
//! refused (the library's building blocks refuse them), and new names go through [`check_folder_name`] (a renamed
//! file keeps its extension). Errors come back as the message to show.

use std::borrow::Cow;
use std::ffi::OsStr;
use std::path::{Component, Path, PathBuf};

use super::library::{
    FORBIDDEN, RECENTLY_DELETED, SONGS_FOLDER, SongEntry, SongLibrary, check_folder_name,
};
use crate::project_files::PRESETS_FOLDER;
use crate::project_files::content::{FOLD_CASE, inside as fold_inside};
use crate::ui::popup::PopupItem;

/// The ≡ menu's items that apply to SONGS and the preset pickers: `Send to playlist` and `Insert at playhead` never do.
/// `Unzip` only does in SONGS, the one that lists archives; `Convert to FLAC` only in the pickers that list audio files.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum Opt {
    SelectAll,
    Restore,
    DeletePermanently,
    CreateFolder,
    Delete,
    /// SONGS only: [`FileActions`] leaves it to the tab, which needs the app's files to extract with.
    Unzip,
    ConvertFlac,
    Copy,
    Cut,
    Paste,
    Export,
    Rename,
    Reveal,
}

/// What the ≡ menu depends on.
#[derive(Clone, Copy, Debug, Default)]
pub(crate) struct MenuState {
    /// Selected rows.
    pub selected: usize,
    /// The selection is exactly one archive.
    pub archive: bool,
    /// The selection is exactly one `.wav` file.
    pub wav: bool,
    /// The open folder is `Recently Deleted`, or one inside it.
    pub in_trash: bool,
    /// The clipboard holds something.
    pub paste: bool,
    /// The selection can be exported (on the web: it holds no folder).
    pub export: bool,
    /// The system's file manager, for `Show in %s...`; `None` leaves it out.
    pub reveal: Option<&'static str>,
    /// The open folder is read-only (imported content): nothing goes into it.
    pub locked_folder: bool,
    /// A selected row is read-only (imported content): it is not moved, renamed or deleted.
    pub locked_picked: bool,
}

/// The ≡ menu, one column: `Select all`, then the file actions in their order. An item that is not present in this
/// state is left out; one that cannot apply to the selection is disabled, and so is one that would change original
/// content (`locked_folder`, `locked_picked`: copying out of it stays).
pub(crate) fn options_menu(s: MenuState) -> Vec<PopupItem<Opt>> {
    use Opt::*;
    let any = s.selected > 0;
    let reveal: Option<Cow<'static, str>> =
        s.reveal.map(|name| format!("Show in {name}...").into());
    // (item, label, present, enabled)
    let items: [(Opt, Cow<'static, str>, bool, bool); 13] = [
        (SelectAll, "Select all".into(), true, true),
        (Restore, "Restore".into(), s.in_trash, any),
        (
            DeletePermanently,
            "Delete permanently".into(),
            s.in_trash,
            any,
        ),
        (
            CreateFolder,
            "Create folder".into(),
            !s.in_trash,
            !s.locked_folder,
        ),
        (
            Delete,
            "Delete".into(),
            !s.in_trash && any,
            !s.locked_picked,
        ),
        (Unzip, "Unzip".into(), s.archive, true),
        // It writes beside the file, so not into original content.
        (
            ConvertFlac,
            "Convert to FLAC".into(),
            s.wav,
            !s.locked_picked,
        ),
        (Copy, "Copy".into(), any, true),
        (Cut, "Cut".into(), any, !s.locked_picked),
        (Paste, "Paste".into(), true, s.paste && !s.locked_folder),
        (Export, "Export".into(), any, s.export),
        (
            Rename,
            "Rename".into(),
            any,
            s.selected == 1 && !s.locked_picked,
        ),
        (
            Reveal,
            reveal.clone().unwrap_or_default(),
            any && reveal.is_some(),
            true,
        ),
    ];
    items
        .into_iter()
        .filter(|&(_, _, present, _)| present)
        .enumerate()
        .map(|(row, (opt, label, _, enabled))| PopupItem {
            disabled: !enabled,
            ..PopupItem::new(opt, 0, row, label)
        })
        .collect()
}

/// `Delete permanently`'s question.
fn purge_question(library: &dyn SongLibrary, picked: &[SongEntry]) -> String {
    match picked {
        [e] if e.is_folder && library.is_empty_folder(&e.path) => "Delete empty folder?".into(),
        [_] => "Delete 1 file?".into(),
        _ => format!("Delete {} files?", picked.len()),
    }
}

/// The ≡ menu's actions on a browser's selection, shared by SONGS and the preset pickers (the preset browsers show the
/// same actions): what `Copy` or `Cut` took, what `Delete permanently` waits to remove, and what the actions moved or
/// deleted.
#[derive(Default)]
pub(crate) struct FileActions {
    clipboard: Option<Clipboard>,
    purge: Option<Vec<SongEntry>>,
    moved: Vec<Moved>,
    convert: Option<Convert>,
}

/// What [`FileActions::apply`] leaves to the browser.
#[derive(Debug, PartialEq)]
pub(crate) enum Step {
    /// `Select all`: select every row but the bin.
    SelectAll,
    /// Nothing more: `Copy`, `Cut`, a `Paste` with nothing copied, or a `Rename` of other than one item.
    Nothing,
    /// `Create folder` (`None`) or `Rename` (what it renames): ask for the name in a field (`Enter folder name`,
    /// `Enter name`) starting with this text, and hand it to [`FileActions::name`].
    Ask(Option<SongEntry>, String),
    /// `Delete permanently`: ask this question, `Yes` / `No`, and hand the answer to [`FileActions::answered`].
    Confirm(String),
    /// The folder may have changed: read it again, select nothing, and show a failure's message.
    Changed(Done),
    /// Nothing moved (`Export`, `Show in ...`): the selection stays; show a failure's message.
    Kept(Done),
}

impl FileActions {
    /// `Paste` has something to paste.
    pub(crate) fn can_paste(&self) -> bool {
        self.clipboard.is_some()
    }

    /// Does `opt` on `picked`, the selected items in the folder's order, with `dir` the open folder (`None`: there is
    /// no library). An item below a picked folder (a search's results can hold both) goes with that folder only.
    pub(crate) fn apply(
        &mut self,
        opt: Opt,
        mut picked: Vec<SongEntry>,
        dir: Option<PathBuf>,
        lib: &mut dyn SongLibrary,
    ) -> Step {
        let all = picked.clone();
        let below = |e: &SongEntry, f: &SongEntry| {
            f.is_folder && f.path != e.path && e.path.starts_with(&f.path)
        };
        picked.retain(|e| !all.iter().any(|f| below(e, f)));
        Step::Changed(match opt {
            Opt::SelectAll => return Step::SelectAll,
            Opt::CreateFolder => return Step::Ask(None, String::new()),
            Opt::Rename => {
                let [entry] = &picked[..] else {
                    return Step::Nothing;
                };
                let text = match entry.is_folder {
                    true => entry.name.clone(),
                    false => Path::new(&entry.name)
                        .file_stem()
                        .map_or_else(String::new, |s| s.to_string_lossy().into_owned()),
                };
                return Step::Ask(Some(entry.clone()), text);
            }
            Opt::Copy | Opt::Cut => {
                let cut = opt == Opt::Cut;
                self.clipboard = Some(Clipboard {
                    entries: picked,
                    cut,
                });
                return Step::Nothing;
            }
            Opt::DeletePermanently => {
                let question = purge_question(lib, &picked);
                self.purge = Some(picked);
                return Step::Confirm(question);
            }
            Opt::Unzip => return Step::Nothing,
            Opt::ConvertFlac => {
                if let [entry] = &picked[..] {
                    self.convert = Some(Convert::Shown(entry.path.clone()));
                }
                return Step::Nothing;
            }
            Opt::Delete => delete(lib, &picked, &mut self.moved),
            Opt::Restore => restore(lib, &picked, &mut self.moved),
            Opt::Paste => {
                let Some(clip) = self.clipboard.take() else {
                    return Step::Nothing;
                };
                let done = match dir {
                    Some(dir) => paste(lib, &clip, &dir, &mut self.moved),
                    None => Err(NO_LIBRARY.into()),
                };
                // A cut is pasted once; a copy as often as wanted.
                if !(clip.cut && done.is_ok()) {
                    self.clipboard = Some(clip);
                }
                done
            }
            Opt::Export => return Step::Kept(export(lib, &picked)),
            Opt::Reveal => {
                #[cfg(not(target_arch = "wasm32"))]
                if let Some(entry) = picked.first() {
                    return Step::Kept(reveal(&entry.path));
                }
                return Step::Kept(Ok(()));
            }
        })
    }

    /// The name typed for [`Step::Ask`]: renames `renaming`, or makes the folder in `dir` (`None`: there is no
    /// library).
    pub(crate) fn name(
        &mut self,
        renaming: Option<&SongEntry>,
        text: &str,
        dir: Option<&Path>,
        lib: &mut dyn SongLibrary,
    ) -> Done {
        match (renaming, dir) {
            (Some(entry), _) => rename(lib, entry, text, &mut self.moved).map(drop),
            (None, Some(dir)) => create_folder(lib, dir, text).map(drop),
            (None, None) => Err(NO_LIBRARY.into()),
        }
    }

    /// The answer to [`Step::Confirm`]'s question: `yes` removes what it asked about, for good. `None`: nothing was
    /// done (the answer was no, or nothing was asked).
    pub(crate) fn answered(&mut self, yes: bool, lib: &mut dyn SongLibrary) -> Option<Done> {
        let picked = self.purge.take()?;
        yes.then(|| delete_permanently(lib, &picked, &mut self.moved))
    }

    /// What the actions moved (`Some` where to) or deleted since the last call, oldest first.
    pub(crate) fn take_moves(&mut self) -> Vec<Moved> {
        std::mem::take(&mut self.moved)
    }

    /// `Convert to FLAC` is under way: the browser shows [`CONVERTING`] with an abort button over everything, and
    /// calls [`Self::convert_step`] each frame (or [`Self::abort_convert`]).
    pub(crate) fn converting(&self) -> bool {
        self.convert.is_some()
    }

    /// Stops `Convert to FLAC`: nothing is written.
    pub(crate) fn abort_convert(&mut self) {
        self.convert = None;
    }

    /// One slice of `Convert to FLAC`, 4096 frames at a time until `over` says the slice is spent: the frame it was
    /// picked in shows the loading screen, the next reads and decodes the WAV, the rest encode it. The FLAC goes beside
    /// the WAV as `<name>.flac`, or the free `<name> (n).flac`, once whole. `Some` when it is over: a failure is
    /// [`CONVERT_FAILED`].
    pub(crate) fn convert_step(
        &mut self,
        lib: &mut dyn SongLibrary,
        over: &mut dyn FnMut() -> bool,
    ) -> Option<Done> {
        let done = match self.convert.take()? {
            Convert::Shown(wav) => {
                self.convert = Some(Convert::Read(wav));
                return None;
            }
            Convert::Read(wav) => match Encode::new(lib, wav) {
                Some(encode) => {
                    self.convert = Some(Convert::Encode(Box::new(encode)));
                    return None;
                }
                None => Err(()),
            },
            Convert::Encode(mut encode) => {
                if !encode.step(over) {
                    self.convert = Some(Convert::Encode(encode));
                    return None;
                }
                encode.write(lib)
            }
        };
        Some(done.map_err(|()| CONVERT_FAILED.into()))
    }
}

/// `Convert to FLAC`'s loading screen and failure message.
pub(crate) const CONVERTING: &str = "Converting to FLAC...";
pub(crate) const CONVERT_FAILED: &str = "Failed to convert to FLAC.";

/// Where `Convert to FLAC` is, with the WAV's path.
enum Convert {
    /// Picked: the loading screen shows first.
    Shown(PathBuf),
    /// Read and decoded next.
    Read(PathBuf),
    Encode(Box<Encode>),
}

/// A WAV decoded, and encoded so far.
struct Encode {
    wav: PathBuf,
    data: honeydew_audio::sample::SampleData,
    encoder: honeydew_audio::flac_writer::FlacEncoder,
    /// 2^(bits − 1): a sample's full scale at the FLAC's depth.
    scale: f32,
    /// Frames encoded.
    at: usize,
}

impl Encode {
    /// The WAV at `wav` read and decoded, to a FLAC of its depth: 16-bit for an 8- or 16-bit file, 24-bit for the rest
    /// (24-bit, 32-bit and float, the encoder's deepest), mono or stereo as the file is.
    fn new(lib: &dyn SongLibrary, wav: PathBuf) -> Option<Self> {
        let bytes = lib.read(&wav).ok()?;
        let data = honeydew_audio::sample::decode(&bytes).ok()?;
        let bits = match honeydew_audio::sample::wav_bits(&bytes) {
            Some(8 | 16) => 16,
            _ => 24,
        };
        let channels = 1 + usize::from(data.right.is_some());
        let encoder =
            honeydew_audio::flac_writer::FlacEncoder::new(data.sample_rate, bits, channels);
        let scale = (1u32 << (bits - 1)) as f32;
        Some(Self {
            wav,
            data,
            encoder,
            scale,
            at: 0,
        })
    }

    /// Encodes until `over` or the end; returns whether it is the end. A sample the decoder read from an integer
    /// file comes back exactly.
    fn step(&mut self, over: &mut dyn FnMut() -> bool) -> bool {
        const BLOCK: usize = 4096;
        let scale = self.scale;
        let quantize = |x: &f32| (x * scale).round().clamp(-scale, scale - 1.0) as i32;
        let channels = std::iter::once(&self.data.left).chain(&self.data.right);
        let channels: Vec<&[f32]> = channels.map(|c| &c[..]).collect();
        while self.at < self.data.frames() {
            let end = (self.at + BLOCK).min(self.data.frames());
            let block: Vec<Vec<i32>> = (channels.iter())
                .map(|c| c[self.at..end].iter().map(quantize).collect())
                .collect();
            self.encoder.push_i32(&block);
            self.at = end;
            if over() {
                break;
            }
        }
        self.at == self.data.frames()
    }

    /// The finished FLAC (the WAV's loop, tempo and markers kept) written beside the WAV under a free name.
    fn write(self, lib: &mut dyn SongLibrary) -> Result<(), ()> {
        let bytes = self.encoder.finish_with_meta(&self.data.meta);
        let dir = self.wav.parent().ok_or(())?;
        let stem = self.wav.file_stem().ok_or(())?;
        let mut name = stem.to_owned();
        name.push(".flac");
        let to = free_path(lib, dir, &name, false);
        lib.write_new(&to, &bytes).map_err(drop)
    }
}

/// What `Copy` or `Cut` took, for `Paste`.
#[derive(Clone, Debug, PartialEq)]
pub struct Clipboard {
    pub entries: Vec<SongEntry>,
    /// `Cut`: paste moves them.
    pub cut: bool,
}

type Done<T = ()> = Result<T, String>;

/// A file or folder an action moved (`Some` where to) or deleted (`None`, also into [`RECENTLY_DELETED`]), for the
/// open song's file to follow ([`ProjectFiles::path_moved`](crate::project_files::ProjectFiles::path_moved)).
pub type Moved = (PathBuf, Option<PathBuf>);

/// Without a songs folder there is nowhere to act (the desktop without a home folder).
pub(super) const NO_LIBRARY: &str = "There is no songs folder.";

/// `dir/name`, or `dir/<stem> (n).<ext>` (a folder: `dir/<name> (n)`) with the smallest free n ≥ 1. `name` is the real
/// one (an entry's [`raw_name`]: one that is no UTF-8 is kept); the numbered ones are made from it lossily. A place's
/// name is never free ([`is_place`]: the whole user folder's bin, while it is missing).
pub fn free_path(lib: &dyn SongLibrary, dir: &Path, name: &OsStr, is_folder: bool) -> PathBuf {
    let lossy = name.to_string_lossy();
    let (stem, ext) = match lossy.rsplit_once('.') {
        Some((stem, ext)) if !is_folder && !stem.is_empty() => (stem, format!(".{ext}")),
        _ => (&*lossy, String::new()),
    };
    std::iter::once(dir.join(name))
        .chain((1..).map(|n| dir.join(format!("{stem} ({n}){ext}"))))
        .find(|path| !lib.exists(path) && !is_place(lib, path))
        .expect("an unbounded range has a free number")
}

/// `e`'s real name, which its shown one is lossily when it is no UTF-8.
fn raw_name(e: &SongEntry) -> &OsStr {
    e.path.file_name().unwrap_or(e.name.as_ref())
}

/// `Create folder` (`Enter folder name`): the folder `name` in `dir`.
pub fn create_folder(lib: &mut dyn SongLibrary, dir: &Path, name: &str) -> Done<PathBuf> {
    let name = name.trim();
    check_new_name(name)?;
    let path = dir.join(name);
    if lib.exists(&path) || is_place(lib, &path) {
        return Err(taken(true));
    }
    lib.make_folder(&path).map_err(|e| failed(name, e))?;
    Ok(path)
}

/// Once at the start, before the bin's purge: each folder of the root named as a row the root lists from beside it
/// ([`SongLibrary::shadowed`]: `My Songs/Demo Songs` an earlier version made by `Paste`, or a file manager did) is
/// renamed to a free numbered name (`Demo Songs (1)`), so that it is listed as itself beside the row, nothing hidden.
/// Each is tried; the first failure is returned, and what failed stays listed under its own name.
pub fn rename_shadowed_folders(lib: &mut dyn SongLibrary) -> Done {
    let Some(root) = lib.root().map(Path::to_owned) else {
        return Ok(());
    };
    let listed = lib.list(None).map_err(|err| failed(SONGS_FOLDER, err))?;
    let shadowed: Vec<_> = (listed.into_iter())
        .filter(|e| e.is_folder && lib.shadowed(&e.path))
        .collect();
    let mut done = Ok(());
    for e in &shadowed {
        let to = free_path(lib, &root, raw_name(e), true);
        let moved = lib
            .move_to(&e.path, &to)
            .map_err(|err| failed(&e.name, err));
        done = done.and(moved);
    }
    done
}

/// `Delete`: copies each of `entries` into [`RECENTLY_DELETED`] at its place below the user folder
/// (`Recently Deleted/My Songs/F/A.flm` for `My Songs/F/A.flm`; the folders on the way are made, the name made free
/// where it is taken), then removes the source. A copy is as new as the deletion, so the bin's 30 days count from it; a
/// move would keep the file's old time and the next start's purge would take it at once. What is in the bin, or outside
/// the user folder, is removed for good, going by where it really is ([`SongLibrary::canonical`]) and refused when that
/// cannot be told. A link in the bin's own path refuses the delete ([`make_folders`]).
pub fn delete(lib: &mut dyn SongLibrary, entries: &[SongEntry], moved: &mut Vec<Moved>) -> Done {
    let trash = lib.trash().ok_or(NO_LIBRARY)?;
    // Where each really is, every link resolved: `alias/My Songs`, with `alias` a link to the user folder, is the songs
    // folder. When that cannot be told the entry is refused, never deleted for good on the strength of its text.
    let here = entries
        .iter()
        .map(|e| lib.canonical(&e.path).map_err(|err| failed(&e.name, err)))
        .collect::<Done<Vec<_>>>()?;
    // The places, as they are (as written where they are not made yet).
    let real = |p: &Path| lib.canonical(p).unwrap_or_else(|_| p.to_owned());
    let (real_trash, real_root) = (real(&trash), lib.root().map(real));
    // In the user folder (the bin's) or the root, as they really are, in no case where the file system ignores it
    // (`FOLD_CASE`). What goes into the bin must be: one a link leads elsewhere (`Unzipped songs` a link to a folder
    // outside, a forged entry below it) is not the user's to bin. What is removed for good as outside must be in
    // neither, written or resolved: `USER/My Songs/A.flm` on a disk that ignores case is the user's song.
    let real_data = trash.parent().map(real);
    let inside = |path: &Path| {
        let under = |d: &Option<PathBuf>| {
            d.as_deref()
                .is_some_and(|d| fold_inside(path, d, FOLD_CASE))
        };
        under(&real_data) || under(&real_root)
    };
    // The bin and what holds the songs folder are places, not deletable (a stale or forged entry would take all), and
    // so are the root's other places (`Unzipped songs`). Nor is a path with `..` in it: `Recently Deleted/../My Songs`
    // is in the bin to `bin_path` and is the root.
    let places = lib.places();
    let place = |(e, here): &(&SongEntry, &PathBuf)| {
        e.path == trash
            || places.contains(&e.path)
            || *here == &real_trash
            || lib.root().is_some_and(|root| root.starts_with(&e.path))
            || real_root
                .as_deref()
                .is_some_and(|root| root.starts_with(here))
            || up(&e.path)
            || (lib.bin_path(&e.path).is_some() && !inside(here))
    };
    if let Some((e, _)) = entries.iter().zip(&here).find(place) {
        return Err(format!("'{}' cannot be deleted.", e.name));
    }
    for (e, here) in entries.iter().zip(&here) {
        match lib.bin_path(&e.path) {
            Some(at) => into_bin(lib, e, &trash, &at)?,
            None => {
                // For good only what really is in the bin, or provably outside the user folder and the root.
                let outside = !inside(here) && !inside(&e.path);
                if !(here.starts_with(&real_trash) || outside) {
                    return Err(format!("'{}' cannot be deleted.", e.name));
                }
                lib.remove(&e.path).map_err(|err| failed(&e.name, err))?;
            }
        }
        moved.push((e.path.clone(), None));
    }
    Ok(())
}

/// Copies `e` to `at` in the bin `trash` under a free name, then removes it. A copy that fails is taken back out of
/// the bin. One that succeeds stays whatever happens to the source: removing a folder is not atomic, so when it
/// fails part way (a read-only folder, a file open on Windows) what it did remove is in the bin and nowhere else.
/// The error is returned, and the duplicate the bin then holds is harmless.
fn into_bin(lib: &mut dyn SongLibrary, e: &SongEntry, trash: &Path, at: &Path) -> Done {
    let dir = at.parent().unwrap_or(trash);
    make_folders(lib, dir, trash.parent().unwrap_or(Path::new("")))?;
    let to = free_path(lib, dir, raw_name(e), e.is_folder);
    copy_then_remove(lib, e, &to, &mut Vec::new())
}

/// `Restore`: moves `entries`, which are in [`RECENTLY_DELETED`], back to where they were deleted from (the bin
/// mirrors the user folder: [`SongLibrary::restore_path`]), making the folders missing on the way and numbering a
/// taken name as `Delete` does. The wrapper folder the bin holds for the songs root (`My Songs`, on disk) goes back
/// as its contents, as it is the root itself, which is there.
pub fn restore(lib: &mut dyn SongLibrary, entries: &[SongEntry], moved: &mut Vec<Moved>) -> Done {
    let root = lib.root().ok_or(NO_LIBRARY)?.to_owned();
    deleted(lib, entries)?;
    let mut renamed = Vec::new();
    entries
        .iter()
        .try_for_each(|e| restore_one(lib, e, &root, moved, &mut renamed))
}

fn restore_one(
    lib: &mut dyn SongLibrary,
    e: &SongEntry,
    root: &Path,
    moved: &mut Vec<Moved>,
    renamed: &mut Vec<(PathBuf, PathBuf)>,
) -> Done {
    let at = lib
        .restore_path(&e.path)
        .ok_or_else(|| format!("'{}' is not in {RECENTLY_DELETED}.", e.name))?;
    let at = unshadowed(lib, root, at, renamed);
    // A place that is there takes back every entry the bin holds of it, hidden ones and links too; one that is missing
    // comes back whole, empty too. Nothing is left in the bin for the purge.
    let place = e.is_folder && place(lib, root, &at);
    if place && lib.exists(&at) {
        let inside = lib.list_raw(&e.path).map_err(|err| failed(&e.name, err))?;
        return inside
            .iter()
            .try_for_each(|c| restore_one(lib, c, root, moved, renamed));
    }
    let dir = at.parent().unwrap_or(root);
    // The folders on the way are made below the root, or below the user folder for one beside the root.
    let above = match dir.starts_with(root) {
        true => root,
        false => root.parent().unwrap_or(root),
    };
    make_folders(lib, dir, above)?;
    let to = match place {
        true => at,
        false => free_path(lib, dir, raw_name(e), e.is_folder),
    };
    if lib.is_link(&e.path) {
        lib.move_link(&e.path, &to)
            .map_err(|err| failed(&e.name, err))?;
        moved.push((e.path.clone(), Some(to)));
        return Ok(());
    }
    move_entry(lib, e, &to, moved)
}

/// `at` below a folder of `root` whose name a row beside the root owns ([`SongLibrary::shadowed`]: what was deleted from
/// `My Songs/Demo Songs`) is put below a free numbered folder instead (`Demo Songs (1)`, made by the restore), the same
/// one for everything one `Restore` puts back there (`renamed`): never into a folder of that name. A folder of that name
/// itself is numbered as any taken name is ([`free_path`]).
fn unshadowed(
    lib: &dyn SongLibrary,
    root: &Path,
    at: PathBuf,
    renamed: &mut Vec<(PathBuf, PathBuf)>,
) -> PathBuf {
    let mut parts = at
        .strip_prefix(root)
        .map(Path::components)
        .into_iter()
        .flatten();
    let (Some(first), Some(_)) = (parts.next(), parts.next()) else {
        return at;
    };
    let top = root.join(first);
    if !lib.shadowed(&top) {
        return at;
    }
    let free = match renamed.iter().find(|(from, _)| *from == top) {
        Some((_, to)) => to.clone(),
        None => {
            let to = free_path(lib, root, first.as_os_str(), true);
            renamed.push((top.clone(), to.clone()));
            to
        }
    };
    free.join(at.strip_prefix(&top).unwrap_or(&at))
}

/// `at` is a place, not an item, so the bin's copy of it goes back as what it holds: the root or a folder holding it, a
/// folder of the user folder (`My Presets`), or a module's folder in `My Presets` (a preset picker's `My Files`, which
/// the pickers delete from into the same bin). The user folder holds the bin: the root's parent, or the root itself
/// (the Instruments picker's `My Files`). The bin itself is none: a folder of its name in it comes back numbered.
fn place(lib: &dyn SongLibrary, root: &Path, at: &Path) -> bool {
    if lib.trash().as_deref() == Some(at) {
        return false;
    }
    let bins = lib.trash().and_then(|t| t.parent().map(Path::to_owned));
    let user = bins.filter(|u| root.starts_with(u));
    let user = (user.as_deref().or(root.parent())).filter(|u| !u.as_os_str().is_empty());
    let parent = at.parent();
    let presets = parent.filter(|p| p.file_name() == Some(PRESETS_FOLDER.as_ref()));
    root.starts_with(at)
        || user.is_some_and(|user| {
            parent == Some(user) || presets.and_then(Path::parent) == Some(user)
        })
}

/// Moves `e` to `to`, which is free, and reports it in `moved`. A rename across file systems (the songs folder on
/// another disk) fails with `CrossesDevices`, every time: then it copies, then removes the source. A copy that fails
/// is taken back out; one that succeeds stays whatever happens to the source, as in [`delete`] (a duplicate is
/// harmless, a lost file is not).
fn move_entry(lib: &mut dyn SongLibrary, e: &SongEntry, to: &Path, moved: &mut Vec<Moved>) -> Done {
    match lib.move_to(&e.path, to) {
        Ok(()) => moved.push((e.path.clone(), Some(to.to_owned()))),
        Err(err) if err.kind() == std::io::ErrorKind::CrossesDevices => {
            copy_then_remove(lib, e, to, moved)?;
        }
        Err(err) => return Err(failed(&e.name, err)),
    }
    Ok(())
}

/// Copies `e` to `to`, which is free, then removes it, reporting it in `moved` once copied. The copy is new (the time
/// of the copy). A copy that fails is taken back out; one that succeeds stays whatever happens to the source.
fn copy_then_remove(
    lib: &mut dyn SongLibrary,
    e: &SongEntry,
    to: &Path,
    moved: &mut Vec<Moved>,
) -> Done {
    if let Err(err) = lib.copy_to(&e.path, to) {
        // Not what is already there, if something took the name meanwhile.
        if err.kind() != std::io::ErrorKind::AlreadyExists {
            let _ = lib.remove(to);
        }
        return Err(failed(&e.name, err));
    }
    moved.push((e.path.clone(), Some(to.to_owned())));
    lib.remove(&e.path).map_err(|err| failed(&e.name, err))
}

/// Makes the folders of `dir` that are missing, outermost first, below `above` (which is not made, nor are its
/// parents). One that is there must not be a link: a file put in one lands where nothing lists it (in the bin, where
/// the purge never looks), or where it was not meant to go.
fn make_folders(lib: &mut dyn SongLibrary, dir: &Path, above: &Path) -> Done {
    let folders: Vec<_> = dir.ancestors().take_while(|d| *d != above).collect();
    for folder in folders.into_iter().rev() {
        let name = folder.file_name().unwrap_or(folder.as_os_str());
        let name = name.to_string_lossy();
        if lib.is_link(folder) {
            let link = std::io::Error::new(std::io::ErrorKind::InvalidInput, "a link");
            return Err(failed(&name, link));
        }
        if !lib.exists(folder) {
            lib.make_folder(folder).map_err(|err| failed(&name, err))?;
        }
    }
    Ok(())
}

/// `Delete permanently`: removes `entries`, which are in [`RECENTLY_DELETED`], for good. Each must be in it written
/// ([`deleted`]) and really, every link resolved, as [`delete`] judges what it removes for good from the bin: a forged
/// `Recently Deleted/lnk/x`, `lnk` a link to a folder outside, is refused, and so is `lnk` (never followed; the bin
/// lists no link). All are checked before any is removed.
pub fn delete_permanently(
    lib: &mut dyn SongLibrary,
    entries: &[SongEntry],
    moved: &mut Vec<Moved>,
) -> Done {
    deleted(lib, entries)?;
    let trash = lib.trash().ok_or(NO_LIBRARY)?;
    let real_trash = lib.canonical(&trash).unwrap_or(trash);
    for e in entries {
        let here = lib.canonical(&e.path).map_err(|err| failed(&e.name, err))?;
        if here == real_trash || !here.starts_with(&real_trash) {
            return Err(format!("'{}' cannot be deleted.", e.name));
        }
    }
    for e in entries {
        lib.remove(&e.path).map_err(|err| failed(&e.name, err))?;
        moved.push((e.path.clone(), None));
    }
    Ok(())
}

/// `Paste`: copies (or, after `Cut`, moves: [`move_entry`]) what `clip` holds into `dir`. Cut entries already in `dir`
/// stay. A cut into [`RECENTLY_DELETED`] is copied then removed, as [`delete`] does: the bin's 30 days count from the
/// copy's time, and a move would keep the old one, for the next start's purge to take the only copy.
pub fn paste(
    lib: &mut dyn SongLibrary,
    clip: &Clipboard,
    dir: &Path,
    moved: &mut Vec<Moved>,
) -> Done {
    if let Some(e) = clip.entries.iter().find(|e| dir.starts_with(&e.path)) {
        return Err(format!("'{}' cannot go into itself.", e.name));
    }
    // A place moves no more than it is deleted ([`delete`]); a copy of one is the user's.
    if clip.cut
        && let Some(e) = clip.entries.iter().find(|e| is_place(lib, &e.path))
    {
        return Err(format!("'{}' cannot be moved.", e.name));
    }
    let into_bin = lib.trash().is_some_and(|trash| {
        let real = |p: &Path| lib.canonical(p).unwrap_or_else(|_| p.to_owned());
        dir.starts_with(&trash) || real(dir).starts_with(real(&trash))
    });
    for e in &clip.entries {
        if clip.cut && e.path.parent() == Some(dir) {
            continue;
        }
        let to = free_path(lib, dir, raw_name(e), e.is_folder);
        if clip.cut && into_bin {
            copy_then_remove(lib, e, &to, moved)?;
        } else if clip.cut {
            move_entry(lib, e, &to, moved)?;
        } else {
            lib.copy_to(&e.path, &to)
                .map_err(|err| failed(&e.name, err))?;
        }
    }
    Ok(())
}

/// `Rename` (`Enter name`): renames `entry` to `name` in its folder. A file keeps its extension, typed or not.
pub fn rename(
    lib: &mut dyn SongLibrary,
    entry: &SongEntry,
    name: &str,
    moved: &mut Vec<Moved>,
) -> Done<PathBuf> {
    let name = name.trim();
    let name = match Path::new(&entry.name).extension() {
        Some(ext) if !entry.is_folder => {
            let ext = ext.to_string_lossy();
            format!("{}.{ext}", strip_ext(name, &ext))
        }
        _ => name.to_owned(),
    };
    check_new_name(&name)?;
    if is_place(lib, &entry.path) {
        return Err(format!("'{}' cannot be renamed.", entry.name));
    }
    let to = entry.path.with_file_name(&name);
    if to == entry.path {
        return Ok(to);
    }
    if lib.exists(&to) || is_place(lib, &to) {
        return Err(taken(entry.is_folder));
    }
    lib.move_to(&entry.path, &to)
        .map_err(|e| failed(&entry.name, e))?;
    moved.push((entry.path.clone(), Some(to.clone())));
    Ok(to)
}

/// `Export` on the desktop. One song: the system's save dialog, which asks before it replaces a file. Anything
/// else: a folder to copy into, under free names.
#[cfg(all(not(target_arch = "wasm32"), not(target_os = "android")))]
pub fn export(lib: &mut dyn SongLibrary, entries: &[SongEntry]) -> Done {
    if let [e] = entries
        && !e.is_folder
    {
        let Some(to) = rfd::FileDialog::new().set_file_name(&e.name).save_file() else {
            return Ok(());
        };
        let bytes = lib.read(&e.path).map_err(|err| failed(&e.name, err))?;
        return crate::project_files::write_atomic(&to, &bytes).map_err(|err| failed(&e.name, err));
    }
    match rfd::FileDialog::new().pick_folder() {
        Some(dir) => export_into(lib, entries, &dir),
        None => Ok(()),
    }
}

/// `Export` on the web: a download for each song. A folder cannot be downloaded: SONGS disables `Export` for one.
#[cfg(target_arch = "wasm32")]
pub fn export(lib: &mut dyn SongLibrary, entries: &[SongEntry]) -> Done {
    for e in entries {
        let bytes = lib.read(&e.path).map_err(|err| failed(&e.name, err))?;
        crate::project_files::download(&e.name, &bytes)
            .map_err(|err| format!("{}: {err}", e.name))?;
    }
    Ok(())
}

#[cfg(all(not(target_arch = "wasm32"), not(target_os = "android")))]
fn export_into(lib: &mut dyn SongLibrary, entries: &[SongEntry], dir: &Path) -> Done {
    for e in entries {
        let to = free_path(lib, dir, raw_name(e), e.is_folder);
        lib.copy_to(&e.path, &to)
            .map_err(|err| failed(&e.name, err))?;
    }
    Ok(())
}

/// `Show in file browser...`: opens the system's file manager on `path` (the folder that holds it on Linux, where
/// `xdg-open` cannot point at a file).
#[cfg(not(target_arch = "wasm32"))]
pub fn reveal(path: &Path) -> Done {
    let mut child = reveal_command(path)
        .spawn()
        .map_err(|e| format!("{}: {e}", FILE_MANAGER.unwrap_or_default()))?;
    // Waited for on its own thread, so that it does not linger as a zombie.
    std::thread::spawn(move || child.wait());
    Ok(())
}

/// `path` is one of the library's places ([`SongLibrary::places`]: the bin, `Unzipped songs`, and in the whole user
/// folder every fixed folder), which hold what the user keeps, in any case where the disk ignores it ([`FOLD_CASE`]).
/// One that is missing is no taken name there (`exists`), yet nothing may become it: a folder renamed or pasted as the
/// bin would be purged by its old times at the next start.
fn is_place(lib: &dyn SongLibrary, path: &Path) -> bool {
    let same = |p: &PathBuf| fold_inside(path, p, FOLD_CASE) && fold_inside(p, path, FOLD_CASE);
    lib.trash().iter().chain(&lib.places()).any(same)
}

/// `path` has a `..` in it, so that where it is cannot be read from its text.
fn up(path: &Path) -> bool {
    path.components().any(|c| c == Component::ParentDir)
}

/// Every one of `entries` is in [`RECENTLY_DELETED`] (not the folder itself, and not by way of a `..`).
fn deleted(lib: &dyn SongLibrary, entries: &[SongEntry]) -> Done {
    let trash = lib.trash().ok_or(NO_LIBRARY)?;
    match entries
        .iter()
        .find(|e| e.path == trash || !e.path.starts_with(&trash) || up(&e.path))
    {
        Some(e) => Err(format!("'{}' is not in {RECENTLY_DELETED}.", e.name)),
        None => Ok(()),
    }
}

/// A name the user typed is a plain folder or file name (a renamed file keeps its own extension, a song's `.flm` or a
/// preset's): the message for one that is not.
fn check_new_name(name: &str) -> Done {
    if let Some(c) = name.chars().find(|&c| FORBIDDEN.contains(c)) {
        return Err(format!("Symbol '{c}' is not allowed in a file name."));
    }
    check_folder_name(name).map_err(|_| format!("'{name}' is not allowed as a file name."))
}

/// `The %s already exists`.
fn taken(is_folder: bool) -> String {
    let what = if is_folder { "folder" } else { "file" };
    format!("The {what} already exists")
}

fn failed(name: &str, err: std::io::Error) -> String {
    format!("{name}: {err}")
}

/// `name` without a `.<ext>` ending, in any case.
fn strip_ext<'a>(name: &'a str, ext: &str) -> &'a str {
    name.len()
        .checked_sub(ext.len() + 1)
        .filter(|&i| {
            name.is_char_boundary(i)
                && name[i..].starts_with('.')
                && name[i + 1..].eq_ignore_ascii_case(ext)
        })
        .map_or(name, |i| &name[..i])
}

/// The system's file manager, as `Show in %s...` names it. `None` on the web, which has none.
pub const FILE_MANAGER: Option<&str> = if cfg!(any(target_arch = "wasm32", target_os = "android")) {
    None
} else if cfg!(windows) {
    Some("Explorer")
} else if cfg!(target_os = "macos") {
    Some("Finder")
} else {
    Some("file browser")
};

#[cfg(not(target_arch = "wasm32"))]
fn reveal_command(path: &Path) -> std::process::Command {
    use std::process::Command;
    let mut command;
    if cfg!(windows) {
        command = Command::new("explorer");
        command.arg("/select,").arg(path);
    } else if cfg!(target_os = "macos") {
        command = Command::new("open");
        command.arg("-R").arg(path);
    } else {
        command = Command::new("xdg-open");
        command.arg(path.parent().unwrap_or(path));
    }
    command
}

#[cfg(test)]
mod tests {
    use super::*;
    #[cfg(not(target_arch = "wasm32"))]
    use crate::ui::main_menu::NativeLibrary;
    use crate::ui::main_menu::SONGS_FOLDER;
    use crate::ui::main_menu::library::MemoryLibrary;

    /// Runs `check` on an in-memory library, then (off the web) on one in a new temp folder.
    fn both(test: &str, check: impl Fn(&mut dyn SongLibrary)) {
        check(&mut MemoryLibrary::default());
        #[cfg(not(target_arch = "wasm32"))]
        {
            let base =
                std::env::temp_dir().join(format!("honeydew-ops-{test}-{}", std::process::id()));
            let _ = std::fs::remove_dir_all(&base);
            check(&mut NativeLibrary::at(base.join(SONGS_FOLDER)));
            std::fs::remove_dir_all(&base).unwrap();
        }
        #[cfg(target_arch = "wasm32")]
        let _ = test;
    }

    fn root(lib: &dyn SongLibrary) -> PathBuf {
        lib.root().unwrap().to_owned()
    }

    fn trash(lib: &dyn SongLibrary) -> PathBuf {
        lib.trash().unwrap()
    }

    /// Where the root's files go in the bin: `Recently Deleted/My Songs` on disk, the bin itself on the web.
    fn bin_root(lib: &dyn SongLibrary) -> PathBuf {
        lib.bin_path(&root(lib))
            .expect("the root is in the user folder")
    }

    /// A song `name` in the folder `dir` (`None`: the root).
    fn put(lib: &mut dyn SongLibrary, dir: Option<&Path>, name: &str, bytes: &[u8]) {
        lib.store(name, bytes).unwrap();
        if let Some(dir) = dir {
            let from = root(lib).join(name);
            lib.move_to(&from, &dir.join(name)).unwrap();
        }
    }

    /// The names in `dir`, sorted, folders ending in `/`.
    fn names(lib: &dyn SongLibrary, dir: Option<&Path>) -> Vec<String> {
        let mut names: Vec<_> = lib
            .list(dir)
            .unwrap()
            .into_iter()
            .map(|e| e.name + if e.is_folder { "/" } else { "" })
            .collect();
        names.sort();
        names
    }

    fn entry(lib: &dyn SongLibrary, dir: Option<&Path>, name: &str) -> SongEntry {
        let list = lib.list(dir).unwrap();
        list.into_iter()
            .find(|e| e.name == name)
            .unwrap_or_else(|| panic!("no {name} in {dir:?}"))
    }

    /// The ≡ menu's (label, enabled) items, checking they fill one column top down.
    fn menu(state: MenuState) -> Vec<(String, bool)> {
        let menu = options_menu(state);
        for (row, item) in menu.iter().enumerate() {
            assert_eq!((item.col, item.row), (0, row), "{}", item.label);
        }
        menu.iter()
            .map(|i| (i.label.to_string(), !i.disabled))
            .collect()
    }

    fn on(label: &str, enabled: bool) -> (String, bool) {
        (label.to_owned(), enabled)
    }

    /// `Unzip` is added for exactly one archive, after `Delete` and before `Copy`, and is enabled then; the picker's
    /// selection never has one.
    #[test]
    fn unzip_follows_delete_for_one_archive_only() {
        let one = MenuState {
            selected: 1,
            archive: true,
            export: true,
            ..MenuState::default()
        };
        assert_eq!(
            menu(one),
            [
                on("Select all", true),
                on("Create folder", true),
                on("Delete", true),
                on("Unzip", true),
                on("Copy", true),
                on("Cut", true),
                on("Paste", false),
                on("Export", true),
                on("Rename", true),
            ]
        );
        let unzip = |s| options_menu(s).iter().any(|i| i.id == Some(Opt::Unzip));
        assert!(unzip(one));
        // Not an archive, or more than the one (the caller says which).
        assert!(!unzip(MenuState {
            archive: false,
            ..one
        }));
        assert!(!unzip(MenuState::default()));
        // The condition has no folder in it: in `Recently Deleted` too, after `Delete permanently`.
        let trash = menu(MenuState {
            in_trash: true,
            ..one
        });
        let at = |label: &str| trash.iter().position(|(l, _)| l == label);
        assert!(at("Delete permanently") < at("Unzip") && at("Unzip") < at("Copy"));
        // The preset pickers' state never says so, and `FileActions` takes nothing for it.
        let mut lib = MemoryLibrary::default();
        let mut actions = FileActions::default();
        assert_eq!(
            actions.apply(Opt::Unzip, Vec::new(), None, &mut lib),
            Step::Nothing
        );
    }

    /// A PCM WAV at 44.1 kHz of `bits` and `channels`, from interleaved `samples`.
    fn pcm_wav(bits: u16, channels: u16, samples: &[i32]) -> Vec<u8> {
        let width = usize::from(bits / 8);
        let data: Vec<u8> = (samples.iter())
            .flat_map(|s| s.to_le_bytes()[..width].to_vec())
            .collect();
        let (rate, align) = (44_100u32, channels * bits / 8);
        let mut out = [b"RIFF".as_slice(), &(36 + data.len() as u32).to_le_bytes()].concat();
        out.extend(b"WAVEfmt \x10\0\0\0\x01\0");
        out.extend(channels.to_le_bytes());
        out.extend(rate.to_le_bytes());
        out.extend((rate * u32::from(align)).to_le_bytes());
        out.extend([align.to_le_bytes(), bits.to_le_bytes()].concat());
        out.extend(b"data");
        out.extend((data.len() as u32).to_le_bytes());
        out.extend(data);
        out
    }

    /// `Convert to FLAC` is offered for exactly one WAV, after `Unzip` and before `Copy`. It writes a FLAC beside the
    /// WAV, under a free name, that decodes to the same samples at the WAV's depth (16-bit stereo, 24-bit mono), and
    /// keeps the WAV; aborted, it writes nothing.
    #[test]
    fn convert_to_flac_writes_the_same_samples_beside_one_wav() {
        let one = MenuState {
            selected: 1,
            wav: true,
            ..MenuState::default()
        };
        let both_kinds = menu(MenuState {
            archive: true,
            ..one
        });
        let at = |label: &str| both_kinds.iter().position(|(l, _)| l == label);
        assert!(at("Unzip") < at("Convert to FLAC") && at("Convert to FLAC") < at("Copy"));
        assert!(menu(one).contains(&on("Convert to FLAC", true)));
        let convert = |s| {
            options_menu(s)
                .iter()
                .any(|i| i.id == Some(Opt::ConvertFlac))
        };
        assert!(!convert(MenuState { wav: false, ..one }));

        both("flac", |lib| {
            let dir = root(lib).join("Samples");
            lib.make_folder(&dir).unwrap();
            // A ramp over the full 16-bit range, and 24-bit values 16 bits could not hold.
            let stereo: Vec<i32> = (0..20_000).map(|i| i * 13 % 65_536 - 32_768).collect();
            let mono: Vec<i32> = (0..5000)
                .map(|i| i * 3355 % 16_777_216 - 8_388_608)
                .collect();
            let files = [
                ("Kick", pcm_wav(16, 2, &stereo)),
                ("Pad", pcm_wav(24, 1, &mono)),
            ];
            for (name, wav) in &files {
                lib.write_new(&dir.join(format!("{name}.wav")), wav)
                    .unwrap();
            }
            lib.write_new(&dir.join("Kick.flac"), b"taken").unwrap();
            let decoded = |lib: &dyn SongLibrary, name: &str| {
                honeydew_audio::sample::decode(&lib.read(&dir.join(name)).unwrap()).unwrap()
            };
            // The songs library on disk lists no audio files.
            let wav = |name: &str| SongEntry {
                name: format!("{name}.wav"),
                path: dir.join(format!("{name}.wav")),
                is_folder: false,
                recency: 0,
            };
            let mut actions = FileActions::default();
            for ((name, _), flac) in files.iter().zip(["Kick (1).flac", "Pad.flac"]) {
                let wav = wav(name);
                let step = actions.apply(Opt::ConvertFlac, vec![wav], Some(dir.clone()), lib);
                assert_eq!(step, Step::Nothing);
                let done = (0..10).find_map(|_| actions.convert_step(lib, &mut || true));
                assert_eq!(done, Some(Ok(())), "{name}");
                assert!(!actions.converting());
                let (wav, flac) = (decoded(lib, &format!("{name}.wav")), decoded(lib, flac));
                assert_eq!(flac.sample_rate, 44_100);
                assert_eq!((flac.left, flac.right), (wav.left, wav.right), "{name}");
            }
            assert_eq!(lib.read(&dir.join("Kick.flac")).unwrap(), b"taken");
            // Aborted while it encodes: nothing is written.
            actions.apply(Opt::ConvertFlac, vec![wav("Kick")], Some(dir.clone()), lib);
            for _ in 0..3 {
                assert_eq!(actions.convert_step(lib, &mut || true), None);
            }
            actions.abort_convert();
            assert!(!actions.converting());
            assert!(!lib.exists(&dir.join("Kick (2).flac")));
            assert!(lib.exists(&dir.join("Kick.wav")) && lib.exists(&dir.join("Pad.wav")));
        });
    }

    #[test]
    fn the_options_menu_follows_the_selection_the_clipboard_and_the_folder() {
        // Nothing selected, nothing copied.
        assert_eq!(
            menu(MenuState::default()),
            [
                on("Select all", true),
                on("Create folder", true),
                on("Paste", false)
            ]
        );
        // Two selected, something copied, a file manager: Rename needs exactly one.
        let two = MenuState {
            selected: 2,
            paste: true,
            export: true,
            reveal: Some("Files"),
            ..MenuState::default()
        };
        assert_eq!(
            menu(two),
            [
                on("Select all", true),
                on("Create folder", true),
                on("Delete", true),
                on("Copy", true),
                on("Cut", true),
                on("Paste", true),
                on("Export", true),
                on("Rename", false),
                on("Show in Files...", true),
            ]
        );
        let one = menu(MenuState { selected: 1, ..two });
        assert!(one.contains(&on("Rename", true)));
        // Without a file manager (the web) it is left out; a folder in the selection there disables Export.
        let web = MenuState {
            selected: 1,
            ..MenuState::default()
        };
        let web = menu(web);
        assert!(web.contains(&on("Export", false)));
        assert!(!web.iter().any(|(l, _)| l.starts_with("Show in")));
        // In Recently Deleted: Restore and Delete permanently, no Create folder or Delete.
        let trash = MenuState {
            in_trash: true,
            ..MenuState::default()
        };
        assert_eq!(
            menu(trash),
            [
                on("Select all", true),
                on("Restore", false),
                on("Delete permanently", false),
                on("Paste", false),
            ]
        );
        assert_eq!(
            menu(MenuState {
                selected: 1,
                export: true,
                ..trash
            }),
            [
                on("Select all", true),
                on("Restore", true),
                on("Delete permanently", true),
                on("Copy", true),
                on("Cut", true),
                on("Paste", false),
                on("Export", true),
                on("Rename", true),
            ]
        );
        assert_eq!(options_menu(two)[0].id, Some(Opt::SelectAll));
        // In the Original Content folder (`Demo Songs`): what would change it is dimmed, what copies out of it is not.
        let locked = MenuState {
            selected: 1,
            paste: true,
            export: true,
            locked_folder: true,
            locked_picked: true,
            ..MenuState::default()
        };
        assert_eq!(
            menu(locked),
            [
                on("Select all", true),
                on("Create folder", false),
                on("Delete", false),
                on("Copy", true),
                on("Cut", false),
                on("Paste", false),
                on("Export", true),
                on("Rename", false),
            ]
        );
        // At the songs' root with `Demo Songs` selected: the folder itself takes new files.
        let root = menu(MenuState {
            locked_folder: false,
            ..locked
        });
        assert!(root.contains(&on("Create folder", true)) && root.contains(&on("Paste", true)));
        assert!(root.contains(&on("Delete", false)) && root.contains(&on("Rename", false)));
    }

    #[test]
    fn delete_permanently_asks_about_files_or_an_empty_folder() {
        let mut lib = crate::ui::main_menu::MemoryLibrary::default();
        lib.store("A.flm", b"").unwrap();
        lib.make_folder(Path::new("Empty")).unwrap();
        let list = lib.list(None).unwrap();
        let by = |n: &str| list.iter().find(|e| e.name == n).unwrap().clone();
        assert_eq!(purge_question(&lib, &[by("A.flm")]), "Delete 1 file?");
        assert_eq!(purge_question(&lib, &[by("Empty")]), "Delete empty folder?");
        assert_eq!(
            purge_question(&lib, &[by("A.flm"), by("Empty")]),
            "Delete 2 files?"
        );
    }

    #[test]
    fn create_folder_makes_a_new_folder_with_a_good_name() {
        both("mkdir", |lib| {
            let root = root(lib);
            let beats = create_folder(lib, &root, " Beats ").unwrap();
            assert_eq!(beats, root.join("Beats"));
            assert_eq!(names(lib, None), ["Beats/"]);
            assert_eq!(
                create_folder(lib, &root, "Beats").unwrap_err(),
                "The folder already exists"
            );
            assert_eq!(
                create_folder(lib, &root, "a?b").unwrap_err(),
                "Symbol '?' is not allowed in a file name."
            );
            for bad in ["", ".hidden", "a/b", "a\\b", "CON", "x\ny"] {
                assert!(create_folder(lib, &root, bad).is_err(), "{bad:?}");
            }
            create_folder(lib, &beats, "Old").unwrap();
            assert_eq!(names(lib, Some(&beats)), ["Old/"]);
            assert_eq!(names(lib, None), ["Beats/"]);
        });
    }

    #[test]
    fn delete_copies_into_recently_deleted_at_its_place_without_replacing_anything() {
        both("delete", |lib| {
            let root = root(lib);
            put(lib, None, "A.flm", b"1");
            let folder = create_folder(lib, &root, "F").unwrap();
            put(lib, Some(&folder), "B.flm", b"b");
            let picked = [entry(lib, None, "A.flm"), entry(lib, None, "F")];
            delete(lib, &picked, &mut Vec::new()).unwrap();
            let (trash, bin) = (trash(lib), bin_root(lib));
            // The root lists Recently Deleted once it exists.
            assert_eq!(names(lib, None), ["Recently Deleted/"]);
            assert_eq!(entry(lib, None, RECENTLY_DELETED).path, trash);
            assert_eq!(names(lib, Some(&bin)), ["A.flm", "F/"]);
            assert_eq!(names(lib, Some(&bin.join("F"))), ["B.flm"]);
            // Another A.flm goes in beside the first.
            put(lib, None, "A.flm", b"2");
            {
                let picked = [entry(lib, None, "A.flm")];
                delete(lib, &picked, &mut Vec::new())
            }
            .unwrap();
            assert_eq!(names(lib, Some(&bin)), ["A (1).flm", "A.flm", "F/"]);
            assert_eq!(lib.read(&bin.join("A.flm")).unwrap(), b"1");
            assert_eq!(lib.read(&bin.join("A (1).flm")).unwrap(), b"2");
            // A folder of a taken name is numbered too, and keeps what was in it.
            let f = create_folder(lib, &root, "F").unwrap();
            put(lib, Some(&f), "C.flm", b"c");
            {
                let picked = [entry(lib, None, "F")];
                delete(lib, &picked, &mut Vec::new())
            }
            .unwrap();
            assert_eq!(
                names(lib, Some(&bin)),
                ["A (1).flm", "A.flm", "F (1)/", "F/"]
            );
            assert_eq!(names(lib, Some(&bin.join("F (1)"))), ["C.flm"]);
            assert_eq!(names(lib, Some(&bin.join("F"))), ["B.flm"]);
            // Nothing was left behind.
            assert_eq!(names(lib, None), ["Recently Deleted/"]);
        });
    }

    #[test]
    fn delete_keeps_the_folders_a_file_was_in_and_numbers_a_taken_name() {
        both("delete-nested", |lib| {
            let root = root(lib);
            let f = create_folder(lib, &root, "F").unwrap();
            let g = create_folder(lib, &f, "G").unwrap();
            put(lib, Some(&g), "B.flm", b"1");
            let picked = [entry(lib, Some(&g), "B.flm")];
            delete(lib, &picked, &mut Vec::new()).unwrap();
            let bin = bin_root(lib);
            // Only the file went; its folders stay, and the bin got the same ones.
            assert_eq!(names(lib, Some(&g)), Vec::<String>::new());
            assert_eq!(names(lib, Some(&bin)), ["F/"]);
            assert_eq!(names(lib, Some(&bin.join("F"))), ["G/"]);
            assert_eq!(lib.read(&bin.join("F/G/B.flm")).unwrap(), b"1");
            // The same path again: unique, beside the first, with the folders reused.
            put(lib, Some(&g), "B.flm", b"2");
            let picked = [entry(lib, Some(&g), "B.flm")];
            delete(lib, &picked, &mut Vec::new()).unwrap();
            assert_eq!(names(lib, Some(&bin.join("F/G"))), ["B (1).flm", "B.flm"]);
            assert_eq!(lib.read(&bin.join("F/G/B (1).flm")).unwrap(), b"2");
            assert_eq!(lib.read(&bin.join("F/G/B.flm")).unwrap(), b"1");
        });
    }

    #[test]
    fn delete_inside_the_bin_is_for_good_and_the_bin_itself_stays() {
        both("delete-bin", |lib| {
            let root = root(lib);
            put(lib, None, "A.flm", b"a");
            put(lib, None, "B.flm", b"b");
            let picked = [entry(lib, None, "A.flm"), entry(lib, None, "B.flm")];
            delete(lib, &picked, &mut Vec::new()).unwrap();
            let (trash, bin) = (trash(lib), bin_root(lib));
            let mut moved = Vec::new();
            let picked = [entry(lib, Some(&bin), "A.flm")];
            delete(lib, &picked, &mut moved).unwrap();
            // Gone for good: not copied anywhere, and reported as deleted.
            assert_eq!(names(lib, Some(&bin)), ["B.flm"]);
            assert_eq!(moved, [(bin.join("A.flm"), None)]);
            assert_eq!(names(lib, None), ["Recently Deleted/"]);
            // The bin, and a folder that holds the root, are not deletable.
            let place = |path: &Path| SongEntry {
                name: "place".into(),
                path: path.to_owned(),
                is_folder: true,
                recency: 0,
            };
            assert!(delete(lib, &[place(&trash)], &mut Vec::new()).is_err());
            assert!(delete(lib, &[place(&root)], &mut Vec::new()).is_err());
            assert!(
                delete(
                    lib,
                    &[place(root.parent().unwrap_or(&root))],
                    &mut Vec::new()
                )
                .is_err()
            );
            assert!(lib.exists(&trash) && lib.exists(&bin.join("B.flm")));
        });
    }

    /// Outside the user folder: for good, with nothing in the bin.
    #[cfg(not(target_arch = "wasm32"))]
    #[test]
    fn delete_outside_the_user_folder_is_for_good() {
        let base =
            std::env::temp_dir().join(format!("honeydew-ops-outside-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&base);
        let user = base.join("user");
        let mut lib = NativeLibrary::at(user.join(SONGS_FOLDER));
        std::fs::create_dir_all(base.join("elsewhere")).unwrap();
        let path = base.join("elsewhere").join("X.flm");
        std::fs::write(&path, b"x").unwrap();
        let x = SongEntry {
            name: "X.flm".into(),
            path: path.clone(),
            is_folder: false,
            recency: 0,
        };
        let mut moved = Vec::new();
        delete(&mut lib, &[x], &mut moved).unwrap();
        assert!(!path.exists());
        assert_eq!(moved, [(path, None)]);
        assert!(!user.join(RECENTLY_DELETED).exists());
        std::fs::remove_dir_all(&base).unwrap();
    }

    /// A deleted file is a new file in the bin: its age counts from the deletion, not from its last change.
    #[cfg(not(target_arch = "wasm32"))]
    #[test]
    fn a_deleted_file_is_as_new_as_its_deletion() {
        use std::time::{Duration, SystemTime};
        let base = std::env::temp_dir().join(format!("honeydew-ops-age-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&base);
        let root = base.join(SONGS_FOLDER);
        let mut lib = NativeLibrary::at(root.clone());
        let f = create_folder(&mut lib, &root, "F").unwrap();
        for path in [root.join("Old.flm"), f.join("Older.flm")] {
            std::fs::write(&path, b"x").unwrap();
            let file = std::fs::File::options().write(true).open(&path).unwrap();
            file.set_modified(SystemTime::now() - Duration::from_secs(86_400 * 90))
                .unwrap();
        }
        let picked = [entry(&lib, None, "Old.flm"), entry(&lib, None, "F")];
        delete(&mut lib, &picked, &mut Vec::new()).unwrap();
        let bin = base.join(RECENTLY_DELETED).join(SONGS_FOLDER);
        for copy in [bin.join("Old.flm"), bin.join("F/Older.flm")] {
            let age = SystemTime::now()
                .duration_since(std::fs::metadata(&copy).unwrap().modified().unwrap())
                .unwrap_or_default();
            assert!(age < Duration::from_secs(60), "{copy:?} is {age:?} old");
        }
        // So the purge of the next start keeps them.
        super::super::library::purge_trash(&base.join(RECENTLY_DELETED), SystemTime::now());
        assert!(bin.join("Old.flm").is_file() && bin.join("F/Older.flm").is_file());
        std::fs::remove_dir_all(&base).unwrap();
    }

    /// A cut pasted into the bin is a deletion too: copied then removed, as new as the paste, so the next start's purge
    /// keeps it. A rename would keep its old time, and the purge would take the only copy.
    #[cfg(not(target_arch = "wasm32"))]
    #[test]
    fn a_cut_pasted_into_the_bin_is_as_new_as_the_paste() {
        use std::time::{Duration, SystemTime};
        let base =
            std::env::temp_dir().join(format!("honeydew-ops-cut-bin-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&base);
        let root = base.join(SONGS_FOLDER);
        let mut lib = NativeLibrary::at(root.clone());
        let f = create_folder(&mut lib, &root, "F").unwrap();
        let trash = trash(&lib);
        let sub = create_folder(&mut lib, &trash, "Sub").unwrap();
        let old = SystemTime::now() - Duration::from_secs(86_400 * 90);
        for path in [
            root.join("Old.flm"),
            f.join("Older.flm"),
            sub.join("In.flm"),
        ] {
            std::fs::write(&path, b"x").unwrap();
            let file = std::fs::File::options().write(true).open(&path).unwrap();
            file.set_modified(old).unwrap();
        }
        let clip = Clipboard {
            entries: vec![entry(&lib, None, "Old.flm"), entry(&lib, None, "F")],
            cut: true,
        };
        paste(&mut lib, &clip, &trash, &mut Vec::new()).unwrap();
        // Into a folder below the bin too.
        let clip = Clipboard {
            entries: vec![entry(&lib, Some(&trash), "Old.flm")],
            cut: true,
        };
        paste(&mut lib, &clip, &sub, &mut Vec::new()).unwrap();
        assert!(!root.join("Old.flm").exists() && !f.exists());
        super::super::library::purge_trash(&trash, SystemTime::now());
        assert!(sub.join("Old.flm").is_file() && trash.join("F/Older.flm").is_file());
        // What was in the bin before keeps its age.
        assert!(!sub.join("In.flm").exists());
        std::fs::remove_dir_all(&base).unwrap();
    }

    #[test]
    fn restore_puts_back_into_the_root_under_a_free_name() {
        both("restore", |lib| {
            let root = root(lib);
            put(lib, None, "A.flm", b"old");
            create_folder(lib, &root, "F").unwrap();
            {
                let picked = [entry(lib, None, "A.flm"), entry(lib, None, "F")];
                delete(lib, &picked, &mut Vec::new())
            }
            .unwrap();
            // A new song took the name meanwhile.
            put(lib, None, "A.flm", b"new");
            let (trash, bin) = (trash(lib), bin_root(lib));
            let picked = [entry(lib, Some(&bin), "A.flm"), entry(lib, Some(&bin), "F")];
            restore(lib, &picked, &mut Vec::new()).unwrap();
            assert_eq!(
                names(lib, None),
                ["A (1).flm", "A.flm", "F/", "Recently Deleted/"]
            );
            assert_eq!(lib.read(&root.join("A (1).flm")).unwrap(), b"old");
            assert_eq!(lib.read(&root.join("A.flm")).unwrap(), b"new");
            assert!(names(lib, Some(&bin)).is_empty());
            assert!(lib.exists(&trash));
            // Only what is in Recently Deleted is restored.
            assert!(
                {
                    let picked = [entry(lib, None, "A.flm")];
                    restore(lib, &picked, &mut Vec::new())
                }
                .is_err()
            );
            assert_eq!(lib.read(&root.join("A.flm")).unwrap(), b"new");
        });
    }

    /// The bin mirrors the user folder, so Restore puts an item back where it was deleted from: below the bin's
    /// prefix, and below `My Songs/` where the layout has that wrapper.
    #[test]
    fn restore_puts_a_nested_file_back_in_its_old_folder_and_makes_the_folders_missing() {
        both("restore-nested", |lib| {
            let root = root(lib);
            let f = create_folder(lib, &root, "F").unwrap();
            let g = create_folder(lib, &f, "G").unwrap();
            put(lib, Some(&g), "B.flm", b"b");
            put(lib, Some(&f), "C.flm", b"c");
            let picked = [entry(lib, Some(&g), "B.flm"), entry(lib, Some(&f), "C.flm")];
            delete(lib, &picked, &mut Vec::new()).unwrap();
            let bin = bin_root(lib);
            // The folders stayed: the files go back into them, not into the root.
            let mut moved = Vec::new();
            let picked = [entry(lib, Some(&bin.join("F/G")), "B.flm")];
            restore(lib, &picked, &mut moved).unwrap();
            assert_eq!(moved, [(bin.join("F/G/B.flm"), Some(g.join("B.flm")))]);
            assert_eq!(lib.read(&g.join("B.flm")).unwrap(), b"b");
            assert_eq!(names(lib, None), ["F/", "Recently Deleted/"]);
            // The folders are gone meanwhile: made again, on the way.
            lib.remove(&f).unwrap();
            let picked = [entry(lib, Some(&bin.join("F")), "C.flm")];
            restore(lib, &picked, &mut Vec::new()).unwrap();
            assert_eq!(lib.read(&f.join("C.flm")).unwrap(), b"c");
            assert_eq!(names(lib, Some(&f)), ["C.flm"]);
            assert_eq!(names(lib, None), ["F/", "Recently Deleted/"]);
        });
    }

    #[test]
    fn restore_numbers_a_name_taken_at_the_old_place() {
        both("restore-taken", |lib| {
            let root = root(lib);
            let f = create_folder(lib, &root, "F").unwrap();
            put(lib, Some(&f), "B.flm", b"old");
            let picked = [entry(lib, Some(&f), "B.flm")];
            delete(lib, &picked, &mut Vec::new()).unwrap();
            // A new B.flm took the name in F meanwhile.
            put(lib, Some(&f), "B.flm", b"new");
            let bin = bin_root(lib);
            let picked = [entry(lib, Some(&bin.join("F")), "B.flm")];
            let mut moved = Vec::new();
            restore(lib, &picked, &mut moved).unwrap();
            assert_eq!(names(lib, Some(&f)), ["B (1).flm", "B.flm"]);
            assert_eq!(lib.read(&f.join("B (1).flm")).unwrap(), b"old");
            assert_eq!(lib.read(&f.join("B.flm")).unwrap(), b"new");
            assert_eq!(moved, [(bin.join("F/B.flm"), Some(f.join("B (1).flm")))]);
        });
    }

    #[test]
    fn restore_of_a_folder_puts_it_back_whole_and_numbers_a_taken_name() {
        both("restore-folder", |lib| {
            let root = root(lib);
            let f = create_folder(lib, &root, "F").unwrap();
            let g = create_folder(lib, &f, "G").unwrap();
            put(lib, Some(&g), "B.flm", b"b");
            put(lib, Some(&f), "C.flm", b"c");
            let picked = [entry(lib, None, "F")];
            delete(lib, &picked, &mut Vec::new()).unwrap();
            let bin = bin_root(lib);
            let picked = [entry(lib, Some(&bin), "F")];
            restore(lib, &picked, &mut Vec::new()).unwrap();
            assert_eq!(names(lib, None), ["F/", "Recently Deleted/"]);
            assert_eq!(names(lib, Some(&f)), ["C.flm", "G/"]);
            assert_eq!(lib.read(&g.join("B.flm")).unwrap(), b"b");
            assert!(names(lib, Some(&bin)).is_empty());
            // Deleted again with an F back at its place: the folder returns as `F (1)`.
            let picked = [entry(lib, None, "F")];
            delete(lib, &picked, &mut Vec::new()).unwrap();
            create_folder(lib, &root, "F").unwrap();
            let picked = [entry(lib, Some(&bin), "F")];
            restore(lib, &picked, &mut Vec::new()).unwrap();
            assert_eq!(names(lib, None), ["F (1)/", "F/", "Recently Deleted/"]);
            assert_eq!(names(lib, Some(&root.join("F (1)"))), ["C.flm", "G/"]);
            assert_eq!(names(lib, Some(&root.join("F"))), Vec::<String>::new());
            // A nested folder goes back into its parent folder, not into the root.
            let (f1, g) = (root.join("F (1)"), root.join("F (1)/G"));
            let picked = [entry(lib, Some(&f1), "G")];
            delete(lib, &picked, &mut Vec::new()).unwrap();
            assert_eq!(names(lib, Some(&bin.join("F (1)"))), ["G/"]);
            let picked = [entry(lib, Some(&bin.join("F (1)")), "G")];
            restore(lib, &picked, &mut Vec::new()).unwrap();
            assert_eq!(names(lib, Some(&f1)), ["C.flm", "G/"]);
            assert_eq!(lib.read(&g.join("B.flm")).unwrap(), b"b");
        });
    }

    /// On disk the bin holds a `My Songs/` wrapper: restoring it puts its contents back in the songs folder, not a
    /// second songs folder (`My Songs (1)`) beside it, where nothing lists it.
    #[test]
    fn restore_of_the_wrapper_folder_puts_its_contents_in_the_root() {
        both("restore-wrapper", |lib| {
            let root = root(lib);
            let (trash, bin) = (trash(lib), bin_root(lib));
            put(lib, None, "A.flm", b"a");
            let f = create_folder(lib, &root, "F").unwrap();
            put(lib, Some(&f), "B.flm", b"b");
            let picked = [entry(lib, None, "A.flm"), entry(lib, None, "F")];
            delete(lib, &picked, &mut Vec::new()).unwrap();
            // The web has no wrapper: the bin's root holds them.
            if bin == trash {
                return;
            }
            put(lib, None, "A.flm", b"new");
            let wrapper = entry(lib, Some(&trash), SONGS_FOLDER);
            restore(lib, &[wrapper], &mut Vec::new()).unwrap();
            assert_eq!(
                names(lib, None),
                ["A (1).flm", "A.flm", "F/", "Recently Deleted/"]
            );
            assert_eq!(lib.read(&root.join("A (1).flm")).unwrap(), b"a");
            assert_eq!(lib.read(&f.join("B.flm")).unwrap(), b"b");
            assert!(!lib.exists(&root.parent().unwrap().join("My Songs (1)")));
        });
    }

    /// `Unzipped songs` is a place of the songs library, beside `My Songs`: it is never deleted whole, what is deleted
    /// from it goes into the bin at its own place (`Recently Deleted/Unzipped songs/…`), and `Restore` puts it back
    /// there, not below `My Songs`.
    #[cfg(not(target_arch = "wasm32"))]
    #[test]
    fn unzipped_songs_is_a_place_and_what_is_deleted_from_it_restores_into_it() {
        let tmp = crate::project_files::content::tests::Temp::new("ops-unzipped");
        let user = tmp.0.clone();
        let unzipped = user.join("Unzipped songs");
        std::fs::create_dir_all(unzipped.join("Pack/My Samples")).unwrap();
        std::fs::write(unzipped.join("Pack/Beat.flm"), b"b").unwrap();
        let mut lib = NativeLibrary::at(user.join(SONGS_FOLDER));
        assert_eq!(lib.places(), [trash(&lib), unzipped.clone()]);
        // The folder itself: a root of SONGS, which cannot be deleted whole.
        let root_entry = entry(&lib, None, "Unzipped songs");
        assert_eq!(root_entry.path, unzipped);
        assert!(delete(&mut lib, &[root_entry], &mut Vec::new()).is_err());
        assert!(unzipped.join("Pack/Beat.flm").exists());
        // What is in it goes into the bin at its own place, and comes back there.
        let pack = entry(&lib, Some(&unzipped), "Pack");
        delete(&mut lib, &[pack], &mut Vec::new()).unwrap();
        assert!(!unzipped.join("Pack").exists());
        let bin = trash(&lib).join("Unzipped songs");
        assert_eq!(lib.read(&bin.join("Pack/Beat.flm")).unwrap(), b"b");
        let wrapper = entry(&lib, Some(&trash(&lib)), "Unzipped songs");
        restore(&mut lib, &[wrapper], &mut Vec::new()).unwrap();
        assert_eq!(lib.read(&unzipped.join("Pack/Beat.flm")).unwrap(), b"b");
        assert!(unzipped.join("Pack/My Samples").is_dir());
        assert!(!user.join(SONGS_FOLDER).join("Unzipped songs").exists());
    }

    /// The root lists each of its rows from beside it once: a folder of the songs folder named as one
    /// (`My Songs/Unzipped songs`, `My Songs/Recently Deleted`, `My Songs/Demo Songs` beside the Original Content
    /// folder's), which an earlier version or a file manager made, is renamed at the start to a free numbered name and
    /// listed as that, with what it holds; nothing is hidden. No action makes one again, as the name is taken:
    /// `Create folder` and `Rename` refuse it, `Paste` numbers it.
    #[cfg(not(target_arch = "wasm32"))]
    #[test]
    fn a_songs_folder_named_as_a_root_row_is_renamed_at_the_start_and_none_is_made() {
        use crate::project_files::CONTENT_FOLDER;
        use crate::project_files::content::tests::Temp;
        use crate::ui::main_menu::library::DEMO_SONGS;
        let base = Temp::new("ops-place-names");
        let user = base.0.join("user");
        let root = user.join(SONGS_FOLDER);
        let unzipped = user.join("Unzipped songs");
        let demos = user.join(CONTENT_FOLDER).join("Songs/Demo Songs");
        std::fs::create_dir_all(unzipped.join("Pack")).unwrap();
        std::fs::create_dir_all(user.join(RECENTLY_DELETED)).unwrap();
        std::fs::create_dir_all(&demos).unwrap();
        let names = ["Unzipped songs", RECENTLY_DELETED, DEMO_SONGS];
        for name in names {
            std::fs::create_dir_all(root.join(name)).unwrap();
            std::fs::write(root.join(name).join("Mine.flm"), name).unwrap();
        }
        // Below the root the names are free.
        std::fs::create_dir_all(root.join("F").join(DEMO_SONGS)).unwrap();
        let mut lib = NativeLibrary::at(root.clone());
        rename_shadowed_folders(&mut lib).unwrap();
        let rows = lib.list(None).unwrap();
        for (name, row) in names.into_iter().zip([&unzipped, &trash(&lib), &demos]) {
            let named: Vec<_> = rows.iter().filter(|e| e.name == name).collect();
            assert_eq!(named.len(), 1, "{name}: {rows:?}");
            assert_eq!(&named[0].path, row);
            let renamed = entry(&lib, None, &format!("{name} (1)"));
            assert_eq!(names_in(&lib, &renamed.path), ["Mine.flm"]);
            assert_eq!(
                std::fs::read(renamed.path.join("Mine.flm")).unwrap(),
                name.as_bytes()
            );
        }
        assert!(root.join("F").join(DEMO_SONGS).is_dir());
        // None is made again.
        let f = root.join("F");
        let folder = entry(&lib, None, "F");
        for name in names {
            assert_eq!(create_folder(&mut lib, &root, name), Err(taken(true)));
            assert_eq!(
                rename(&mut lib, &folder, name, &mut Vec::new()),
                Err(taken(true))
            );
            std::fs::create_dir_all(f.join(name)).unwrap();
            let clip = Clipboard {
                entries: vec![entry(&lib, Some(&f), name)],
                cut: false,
            };
            paste(&mut lib, &clip, &root, &mut Vec::new()).unwrap();
            assert!(root.join(format!("{name} (2)")).is_dir(), "{name}");
            assert!(!root.join(name).exists(), "{name}");
        }
        // Without the Original Content folder there is no `Demo Songs` row, and the name is free.
        std::fs::remove_dir_all(user.join(CONTENT_FOLDER)).unwrap();
        create_folder(&mut lib, &root, DEMO_SONGS).unwrap();
        rename_shadowed_folders(&mut lib).unwrap();
        assert!(root.join(DEMO_SONGS).is_dir());
    }

    /// The names in the folder `dir`, sorted, folders ending in `/`.
    fn names_in(lib: &dyn SongLibrary, dir: &Path) -> Vec<String> {
        names(lib, Some(dir))
    }

    /// `Restore` never puts anything into a folder of the root named as a row beside it: what was deleted from one
    /// (`Recently Deleted/My Songs/Demo Songs/Old.flm`) goes back into a free numbered folder, made once for all of it,
    /// listed at the root.
    #[cfg(not(target_arch = "wasm32"))]
    #[test]
    fn restore_never_puts_anything_into_a_folder_named_as_a_root_row() {
        use crate::project_files::CONTENT_FOLDER;
        use crate::project_files::content::tests::Temp;
        use crate::ui::main_menu::library::DEMO_SONGS;
        let base = Temp::new("ops-restore-row");
        let user = base.0.join("user");
        let root = user.join(SONGS_FOLDER);
        std::fs::create_dir_all(user.join(CONTENT_FOLDER).join("Songs/Demo Songs")).unwrap();
        std::fs::create_dir_all(&root).unwrap();
        let mut lib = NativeLibrary::at(root.clone());
        let deleted = trash(&lib).join(SONGS_FOLDER).join(DEMO_SONGS);
        std::fs::create_dir_all(&deleted).unwrap();
        std::fs::write(deleted.join("Old.flm"), b"old").unwrap();
        std::fs::write(deleted.join("Older.flm"), b"older").unwrap();
        // The user's own folder from before, renamed at the start.
        std::fs::create_dir_all(root.join("Demo Songs (1)")).unwrap();
        let picked = [
            entry(&lib, Some(&deleted), "Old.flm"),
            entry(&lib, Some(&deleted), "Older.flm"),
        ];
        restore(&mut lib, &picked, &mut Vec::new()).unwrap();
        assert!(!root.join(DEMO_SONGS).exists());
        let back = entry(&lib, None, "Demo Songs (2)");
        assert_eq!(names_in(&lib, &back.path), ["Old.flm", "Older.flm"]);
        assert_eq!(std::fs::read(back.path.join("Old.flm")).unwrap(), b"old");
    }

    /// The web's root lists its bin once too: a folder `Recently Deleted` of the session's songs is renamed at the
    /// start, `Restore` puts nothing into one, and no action makes one.
    #[test]
    fn the_webs_folder_named_as_its_bin_is_renamed_and_none_is_made() {
        let mut lib = MemoryLibrary::default();
        put(&mut lib, None, "A.flm", b"a");
        let picked = [entry(&lib, None, "A.flm")];
        delete(&mut lib, &picked, &mut Vec::new()).unwrap();
        // One kept below such a folder (the web has no other way to make it).
        lib.put(PathBuf::from("Recently Deleted/B.flm"), b"b")
            .unwrap();
        rename_shadowed_folders(&mut lib).unwrap();
        assert_eq!(
            names(&lib, None),
            ["Recently Deleted (1)/", "Recently Deleted/"]
        );
        assert_eq!(entry(&lib, None, RECENTLY_DELETED).path, trash(&lib));
        assert_eq!(
            lib.read(Path::new("Recently Deleted (1)/B.flm")).unwrap(),
            b"b"
        );
        let root = root(&lib);
        assert_eq!(
            create_folder(&mut lib, &root, RECENTLY_DELETED),
            Err(taken(true))
        );
        let f = create_folder(&mut lib, &root, "F").unwrap();
        let folder = entry(&lib, None, "F");
        assert_eq!(
            rename(&mut lib, &folder, RECENTLY_DELETED, &mut Vec::new()),
            Err(taken(true))
        );
        lib.make_folder(&f.join(RECENTLY_DELETED)).unwrap();
        let clip = Clipboard {
            entries: vec![entry(&lib, Some(&f), RECENTLY_DELETED)],
            cut: true,
        };
        paste(&mut lib, &clip, &root, &mut Vec::new()).unwrap();
        assert!(lib.exists(Path::new("Recently Deleted (2)")));
        // What the bin holds from such a folder goes back into a free numbered one.
        let deleted = trash(&lib).join(RECENTLY_DELETED);
        lib.put(deleted.join("C.flm"), b"c").unwrap();
        let picked = [entry(&lib, Some(&deleted), "C.flm")];
        restore(&mut lib, &picked, &mut Vec::new()).unwrap();
        assert_eq!(
            lib.read(Path::new("Recently Deleted (3)/C.flm")).unwrap(),
            b"c"
        );
        assert_eq!(
            names(&lib, None),
            [
                "F/",
                "Recently Deleted (1)/",
                "Recently Deleted (2)/",
                "Recently Deleted (3)/",
                "Recently Deleted/"
            ]
        );
    }

    /// In the whole user folder (the Instruments picker's `My Files`) the bin and `Unzipped songs` are its own folders,
    /// not names a row owns: a delete makes the bin, and lists it.
    #[cfg(not(target_arch = "wasm32"))]
    #[test]
    fn the_whole_user_folders_own_bin_is_no_taken_name() {
        use crate::project_files::content::tests::Temp;
        let base = Temp::new("ops-whole-bin");
        let user = base.0.join("user");
        std::fs::create_dir_all(user.join("My Presets")).unwrap();
        std::fs::write(user.join("My Presets/P.flm"), b"p").unwrap();
        let mut lib = NativeLibrary::user_folder(user.clone());
        let bin = user.join(RECENTLY_DELETED);
        assert!(!lib.shadowed(&bin) && !lib.exists(&bin));
        assert!(!lib.shadowed(&user.join("Unzipped songs")));
        let picked = [entry(&lib, Some(&user.join("My Presets")), "P.flm")];
        delete(&mut lib, &picked, &mut Vec::new()).unwrap();
        assert_eq!(std::fs::read(bin.join("My Presets/P.flm")).unwrap(), b"p");
        assert!(names(&lib, None).contains(&format!("{RECENTLY_DELETED}/")));
    }

    /// In the whole user folder a missing bin is no taken name, yet a folder renamed, made or pasted by its
    /// name (in any case where the disk ignores it) would be the bin, and the next start's purge would take what it
    /// holds by its old times. Each is refused or numbered, and so is every other place's name (`My Presets`).
    #[cfg(not(target_arch = "wasm32"))]
    #[test]
    fn the_whole_user_folder_never_makes_its_missing_bin_by_name() {
        use crate::project_files::content::tests::Temp;
        let base = Temp::new("ops-whole-no-bin");
        let user = base.0.join("user");
        let old = user.join("Old Projects");
        std::fs::create_dir_all(old.join(RECENTLY_DELETED)).unwrap();
        let now = std::time::SystemTime::now();
        let song = std::fs::File::create(old.join("Lead.flms")).unwrap();
        (song.set_modified(now - std::time::Duration::from_secs(90 * 86_400))).unwrap();
        let mut lib = NativeLibrary::user_folder(user.clone());
        let bin = trash(&lib);
        let folder = entry(&lib, None, "Old Projects");
        let mut taken = vec![RECENTLY_DELETED.to_owned(), "My Presets".into()];
        if FOLD_CASE {
            taken.push(RECENTLY_DELETED.to_lowercase());
        }
        for name in &taken {
            assert!(
                rename(&mut lib, &folder, name, &mut Vec::new()).is_err(),
                "{name}"
            );
            assert!(create_folder(&mut lib, &user, name).is_err(), "{name}");
        }
        // A folder of that name pasted there, copied or cut, is numbered.
        let inner = entry(&lib, Some(&old), RECENTLY_DELETED);
        for cut in [false, true] {
            let clip = Clipboard {
                entries: vec![inner.clone()],
                cut,
            };
            paste(&mut lib, &clip, &user, &mut Vec::new()).unwrap();
        }
        assert!(!bin.exists() && !user.join("My Presets").exists());
        assert!(user.join("Recently Deleted (1)").is_dir());
        assert!(user.join("Recently Deleted (2)").is_dir());
        crate::ui::main_menu::library::purge_trash(&bin, now);
        assert!(old.join("Lead.flms").exists());
    }

    /// The preset pickers delete into the same bin (`<user>/Recently Deleted/My Presets/…`), which SONGS lists whole.
    /// A user folder in it (`My Presets`), and a module's folder in that (a picker's `My Files`), are places: SONGS'
    /// `Restore` puts back what they hold, beside `My Songs`, and its question counts every file, not only songs.
    #[test]
    fn songs_restores_deleted_presets_into_my_presets_and_never_calls_them_empty() {
        let user =
            std::env::temp_dir().join(format!("honeydew-ops-presets-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&user);
        let mini = user.join("My Presets").join("MiniSynth");
        std::fs::create_dir_all(&mini).unwrap();
        for name in ["Fat.flms", "Lead.flmpst"] {
            std::fs::write(mini.join(name), name).unwrap();
        }
        let mut presets = NativeLibrary::at(user.join("My Presets")).with_exts(&["flmpst", "flms"]);
        let fat = entry(&presets, Some(&mini), "Fat.flms");
        delete(&mut presets, &[fat], &mut Vec::new()).unwrap();
        let mut songs = NativeLibrary::at(user.join(SONGS_FOLDER));
        let trash = trash(&songs);
        let wrapper = entry(&songs, Some(&trash), "My Presets");
        // Only a preset in it: not an empty folder.
        assert_eq!(
            purge_question(&songs, std::slice::from_ref(&wrapper)),
            "Delete 1 file?"
        );
        let module = entry(&songs, Some(&wrapper.path), "MiniSynth");
        assert_eq!(purge_question(&songs, &[module]), "Delete 1 file?");
        restore(&mut songs, &[wrapper], &mut Vec::new()).unwrap();
        assert_eq!(std::fs::read(mini.join("Fat.flms")).unwrap(), b"Fat.flms");
        assert!(!user.join(SONGS_FOLDER).join("My Presets").exists());
        assert!(!user.join("My Presets").join("MiniSynth (1)").exists());
        assert!(!user.join("My Presets (1)").exists());
        // An empty folder is still one.
        std::fs::create_dir_all(trash.join("Empty")).unwrap();
        let empty = entry(&songs, Some(&trash), "Empty");
        assert_eq!(purge_question(&songs, &[empty]), "Delete empty folder?");
        std::fs::remove_dir_all(&user).unwrap();
    }

    /// The chord set picker deletes into the same bin (`<user>/Recently Deleted/My Chord Sets/…`): SONGS' `Restore`
    /// puts the file back into `My Chord Sets`, beside `My Songs`, and not into the songs folder.
    #[test]
    fn songs_restores_deleted_chord_sets_into_my_chord_sets() {
        let user = std::env::temp_dir().join(format!("honeydew-ops-chords-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&user);
        let mine = user.join("My Chord Sets");
        std::fs::create_dir_all(&mine).unwrap();
        std::fs::write(mine.join("Mine.chords"), "set").unwrap();
        let mut sets = NativeLibrary::at(mine.clone()).with_exts(&["chords"]);
        let set = entry(&sets, Some(&mine), "Mine.chords");
        delete(&mut sets, &[set], &mut Vec::new()).unwrap();
        assert!(!mine.join("Mine.chords").exists());
        let mut songs = NativeLibrary::at(user.join(SONGS_FOLDER));
        let wrapper = entry(&songs, Some(&trash(&songs)), "My Chord Sets");
        restore(&mut songs, &[wrapper], &mut Vec::new()).unwrap();
        assert_eq!(std::fs::read(mine.join("Mine.chords")).unwrap(), b"set");
        assert!(!user.join(SONGS_FOLDER).join("My Chord Sets").exists());
        assert!(!user.join("My Chord Sets (1)").exists());
        std::fs::remove_dir_all(&user).unwrap();
    }

    /// The Instruments picker's `My Files` deletes from the whole user folder into the same bin, at its place
    /// (`Recently Deleted/Mine`): SONGS' `Restore` puts it back there, beside `My Songs`, not into it.
    #[cfg(not(target_arch = "wasm32"))]
    #[test]
    fn songs_restores_what_the_whole_user_folder_deleted_to_where_it_was() {
        use crate::project_files::content::tests::Temp;
        let base = Temp::new("ops-whole-restore");
        let user = base.0.clone();
        std::fs::create_dir_all(user.join("Mine")).unwrap();
        std::fs::write(user.join("Mine/Lead.flms"), b"l").unwrap();
        let mut whole = NativeLibrary::user_folder(user.clone());
        let mine = entry(&whole, None, "Mine");
        delete(&mut whole, &[mine], &mut Vec::new()).unwrap();
        let mut songs = NativeLibrary::at(user.join(SONGS_FOLDER));
        let deleted = entry(&songs, Some(&trash(&songs)), "Mine");
        restore(&mut songs, &[deleted], &mut Vec::new()).unwrap();
        assert_eq!(std::fs::read(user.join("Mine/Lead.flms")).unwrap(), b"l");
        assert!(!user.join(SONGS_FOLDER).join("Mine").exists());
    }

    /// A folder of the user folder deleted whole comes back whole where nothing has its name, so what `list_all` leaves
    /// out (a hidden file) goes back with it, and an empty folder comes back too: neither is left in the bin for the
    /// purge.
    #[cfg(not(target_arch = "wasm32"))]
    #[test]
    fn restore_puts_a_missing_user_folder_back_whole() {
        use crate::project_files::content::tests::Temp;
        let base = Temp::new("ops-whole-back");
        let user = base.0.clone();
        std::fs::create_dir_all(user.join("Mine")).unwrap();
        std::fs::create_dir_all(user.join("Empty")).unwrap();
        std::fs::write(user.join("Mine/.hidden"), b"h").unwrap();
        let mut lib = NativeLibrary::user_folder(user.clone());
        let picked = [entry(&lib, None, "Mine"), entry(&lib, None, "Empty")];
        delete(&mut lib, &picked, &mut Vec::new()).unwrap();
        let bin = trash(&lib);
        assert_eq!(std::fs::read(bin.join("Mine/.hidden")).unwrap(), b"h");
        let deleted = [
            entry(&lib, Some(&bin), "Mine"),
            entry(&lib, Some(&bin), "Empty"),
        ];
        restore(&mut lib, &deleted, &mut Vec::new()).unwrap();
        assert_eq!(std::fs::read(user.join("Mine/.hidden")).unwrap(), b"h");
        assert!(user.join("Empty").is_dir());
        assert!(!bin.join("Mine").exists() && !bin.join("Empty").exists());
    }

    /// `Restore` into a user folder that is there (`Mine`) moves every entry the bin's copy holds,
    /// hidden files and links too (a link itself, never followed), numbering a taken name: nothing is left in the bin.
    #[cfg(unix)]
    #[test]
    fn restore_into_a_user_folder_that_is_there_moves_hidden_files_and_links_too() {
        use crate::project_files::content::tests::Temp;
        let base = Temp::new("ops-whole-merge");
        let user = base.0.join("user");
        let bin = user.join(RECENTLY_DELETED);
        std::fs::create_dir_all(user.join("Mine")).unwrap();
        std::fs::write(user.join("Mine/.hidden"), b"there").unwrap();
        std::fs::create_dir_all(bin.join("Mine")).unwrap();
        std::fs::write(bin.join("Mine/.hidden"), b"h").unwrap();
        let target = base.0.join("Target.flms");
        std::fs::write(&target, b"t").unwrap();
        std::os::unix::fs::symlink(&target, bin.join("Mine/Link.flms")).unwrap();
        let mut lib = NativeLibrary::user_folder(user.clone());
        let deleted = entry(&lib, Some(&bin), "Mine");
        restore(&mut lib, &[deleted], &mut Vec::new()).unwrap();
        assert_eq!(std::fs::read(user.join("Mine/.hidden")).unwrap(), b"there");
        assert_eq!(std::fs::read(user.join("Mine/.hidden (1)")).unwrap(), b"h");
        let link = std::fs::symlink_metadata(user.join("Mine/Link.flms")).unwrap();
        assert!(link.is_symlink());
        assert_eq!(std::fs::read_dir(bin.join("Mine")).unwrap().count(), 0);
        assert_eq!(std::fs::read(&target).unwrap(), b"t");
    }

    /// A stale bin entry swapped for a link to a folder outside is never merged through: the restore fails on the link,
    /// and nothing outside moves.
    #[cfg(unix)]
    #[test]
    fn restore_never_merges_through_a_bin_folder_swapped_for_a_link() {
        use crate::project_files::content::tests::Temp;
        let base = Temp::new("ops-merge-link");
        let user = base.0.join("user");
        let bin = user.join(RECENTLY_DELETED);
        let outside = base.0.join("outside");
        std::fs::create_dir_all(user.join("Mine")).unwrap();
        std::fs::create_dir_all(bin.join("Mine")).unwrap();
        std::fs::create_dir_all(&outside).unwrap();
        std::fs::write(outside.join("X.flms"), b"x").unwrap();
        let mut lib = NativeLibrary::user_folder(user.clone());
        let stale = entry(&lib, Some(&bin), "Mine");
        std::fs::remove_dir(bin.join("Mine")).unwrap();
        std::os::unix::fs::symlink(&outside, bin.join("Mine")).unwrap();
        let err = restore(&mut lib, &[stale], &mut Vec::new()).unwrap_err();
        assert_eq!(err, "Mine: a link");
        assert_eq!(std::fs::read(outside.join("X.flms")).unwrap(), b"x");
        assert!(!user.join("Mine/X.flms").exists());
    }

    /// A folder named as the bin, in the bin (pasted there), is no place: it restores beside the bin under a free
    /// numbered name, not into the bin itself.
    #[cfg(not(target_arch = "wasm32"))]
    #[test]
    fn a_folder_named_as_the_bin_restores_numbered_beside_it() {
        use crate::project_files::content::tests::Temp;
        let base = Temp::new("ops-bin-in-bin");
        let user = base.0.clone();
        let mut lib = NativeLibrary::at(user.join(SONGS_FOLDER));
        let bin = trash(&lib);
        let inner = bin.join(RECENTLY_DELETED);
        std::fs::create_dir_all(&inner).unwrap();
        std::fs::write(inner.join("A.flm"), b"a").unwrap();
        let deleted = entry(&lib, Some(&bin), RECENTLY_DELETED);
        restore(&mut lib, &[deleted], &mut Vec::new()).unwrap();
        let back = user.join("Recently Deleted (1)").join("A.flm");
        assert_eq!(std::fs::read(back).unwrap(), b"a");
        assert!(!inner.exists() && !bin.join("A.flm").exists());
    }

    /// A name that is no UTF-8 lists lossily (`A\u{FFFD}.flm`), but `Delete`, `Restore` and `Paste` keep its real name:
    /// the lossy one is a name nobody gave it.
    #[cfg(unix)]
    #[test]
    fn delete_restore_and_paste_keep_a_name_that_is_no_utf8() {
        use crate::project_files::content::tests::Temp;
        use std::os::unix::ffi::OsStrExt as _;
        let base = Temp::new("ops-non-utf8");
        let root = base.0.join(SONGS_FOLDER);
        let raw = std::ffi::OsStr::from_bytes(b"A\xff.flm");
        std::fs::create_dir_all(root.join("F")).unwrap();
        std::fs::write(root.join(raw), b"a").unwrap();
        let mut lib = NativeLibrary::at(root.clone());
        let lossy = "A\u{FFFD}.flm";
        let song = entry(&lib, None, lossy);
        delete(&mut lib, &[song], &mut Vec::new()).unwrap();
        let bin = bin_root(&lib);
        assert_eq!(std::fs::read(bin.join(raw)).unwrap(), b"a");
        let deleted = entry(&lib, Some(&bin), lossy);
        restore(&mut lib, &[deleted], &mut Vec::new()).unwrap();
        assert_eq!(std::fs::read(root.join(raw)).unwrap(), b"a");
        let clip = Clipboard {
            entries: vec![entry(&lib, None, lossy)],
            cut: false,
        };
        paste(&mut lib, &clip, &root.join("F"), &mut Vec::new()).unwrap();
        assert_eq!(std::fs::read(root.join("F").join(raw)).unwrap(), b"a");
    }

    /// The source that cannot be removed (a read-only folder here, an open file on Windows) is no reason to take the
    /// copy back: `remove_dir_all` is not atomic, so what it removed first is in the bin and nowhere else.
    #[cfg(unix)]
    #[test]
    fn a_delete_whose_original_cannot_be_removed_keeps_every_file_in_the_bin() {
        use std::os::unix::fs::PermissionsExt;
        let base =
            std::env::temp_dir().join(format!("honeydew-ops-partial-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&base);
        let root = base.join(SONGS_FOLDER);
        let mut lib = NativeLibrary::at(root.clone());
        let f = create_folder(&mut lib, &root, "F").unwrap();
        let mut files: Vec<_> = (0..40).map(|i| format!("{i:02}.flm")).collect();
        files.push("Locked/X.flm".into());
        std::fs::create_dir(f.join("Locked")).unwrap();
        for name in &files {
            std::fs::write(f.join(name), name.as_bytes()).unwrap();
        }
        let locked = f.join("Locked");
        std::fs::set_permissions(&locked, std::fs::Permissions::from_mode(0o555)).unwrap();
        let allow = || std::fs::set_permissions(&locked, std::fs::Permissions::from_mode(0o755));
        // Root ignores the mode: nothing to reproduce there.
        if std::fs::write(locked.join("probe"), b"").is_ok() {
            allow().unwrap();
            std::fs::remove_dir_all(&base).unwrap();
            return;
        }
        let picked = [entry(&lib, None, "F")];
        let mut moved = Vec::new();
        assert!(delete(&mut lib, &picked, &mut moved).is_err());
        // Some of the source may be gone; all of it is in the bin, whole.
        let bin = base.join(RECENTLY_DELETED).join(SONGS_FOLDER).join("F");
        for name in &files {
            assert_eq!(std::fs::read(bin.join(name)).unwrap(), name.as_bytes());
            assert!(
                f.join(name).exists() || bin.join(name).exists(),
                "{name} is lost"
            );
        }
        allow().unwrap();
        std::fs::remove_dir_all(&base).unwrap();
    }

    /// A path with `..` in it is lexically inside the bin and in the root's way both: no guard may read it so.
    #[test]
    fn a_path_with_a_parent_component_is_never_deleted_or_restored() {
        both("dotdot", |lib| {
            let root = root(lib);
            put(lib, None, "A.flm", b"a");
            let trash = trash(lib);
            let picked = [entry(lib, None, "A.flm")];
            delete(lib, &picked, &mut Vec::new()).unwrap();
            put(lib, None, "A.flm", b"a");
            // `Recently Deleted/../My Songs` is the songs folder itself on disk; the web's one is a file.
            let name = root.file_name().map_or("A.flm".as_ref(), |n| n);
            let forged = SongEntry {
                name: "forged".into(),
                path: trash.join("..").join(name),
                is_folder: true,
                recency: 0,
            };
            let forged = [forged];
            assert!(delete(lib, &forged, &mut Vec::new()).is_err());
            assert!(delete_permanently(lib, &forged, &mut Vec::new()).is_err());
            assert!(restore(lib, &forged, &mut Vec::new()).is_err());
            assert_eq!(lib.read(&root.join("A.flm")).unwrap(), b"a");
        });
    }

    /// Places are told apart by where they really are: `alias/My Songs`, with `alias` a link to the user folder, is the
    /// songs folder, not something outside the user folder to delete for good.
    #[cfg(unix)]
    #[test]
    fn a_path_through_a_link_to_the_user_folder_is_still_in_it() {
        let base =
            std::env::temp_dir().join(format!("honeydew-ops-alias-up-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&base);
        let (user, alias) = (base.join("user"), base.join("alias"));
        let root = user.join(SONGS_FOLDER);
        std::fs::create_dir_all(root.join("F")).unwrap();
        std::fs::write(root.join("A.flm"), b"a").unwrap();
        std::fs::write(root.join("F/B.flm"), b"b").unwrap();
        std::os::unix::fs::symlink(&user, &alias).unwrap();
        let mut lib = NativeLibrary::at(root.clone());
        let at = |path: PathBuf, is_folder| SongEntry {
            name: "forged".into(),
            path,
            is_folder,
            recency: 0,
        };
        let forged = [
            at(alias.join("My Songs"), true),
            at(alias.join("My Songs/F"), true),
            at(alias.join("My Songs/A.flm"), false),
            at(alias.clone(), true),
        ];
        for e in &forged {
            let done = delete(&mut lib, std::slice::from_ref(e), &mut Vec::new());
            assert!(done.is_err(), "{:?}", e.path);
        }
        // Not told apart at all (a missing path): refused too, never deleted for good.
        let missing = at(alias.join("My Songs/Gone.flm"), false);
        assert!(delete(&mut lib, &[missing], &mut Vec::new()).is_err());
        assert_eq!(std::fs::read(root.join("A.flm")).unwrap(), b"a");
        assert_eq!(std::fs::read(root.join("F/B.flm")).unwrap(), b"b");
        assert!(!user.join(RECENTLY_DELETED).exists());
        std::fs::remove_dir_all(&base).unwrap();
    }

    /// The same places behind a link in the data folder's own path: no false refusals.
    #[cfg(unix)]
    #[test]
    fn a_data_folder_behind_a_link_deletes_into_the_bin_and_for_good() {
        let base = std::env::temp_dir().join(format!("honeydew-ops-behind-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&base);
        let (real, link) = (base.join("real"), base.join("link"));
        std::fs::create_dir_all(real.join("user/My Songs")).unwrap();
        std::fs::create_dir_all(base.join("elsewhere")).unwrap();
        std::fs::write(real.join("user/My Songs/A.flm"), b"a").unwrap();
        std::fs::write(base.join("elsewhere/X.flm"), b"x").unwrap();
        std::os::unix::fs::symlink(&real, &link).unwrap();
        let mut lib = NativeLibrary::at(link.join("user/My Songs"));
        let picked = [entry(&lib, None, "A.flm")];
        delete(&mut lib, &picked, &mut Vec::new()).unwrap();
        let bin = real.join("user/Recently Deleted/My Songs");
        assert_eq!(std::fs::read(bin.join("A.flm")).unwrap(), b"a");
        // In the bin, reached through the link: for good.
        let in_bin = lib.trash().unwrap().join("My Songs");
        let picked = [entry(&lib, Some(&in_bin), "A.flm")];
        delete(&mut lib, &picked, &mut Vec::new()).unwrap();
        assert!(!bin.join("A.flm").exists());
        // Outside the user folder: for good.
        let x = SongEntry {
            name: "X.flm".into(),
            path: base.join("elsewhere/X.flm"),
            is_folder: false,
            recency: 0,
        };
        delete(&mut lib, &[x], &mut Vec::new()).unwrap();
        assert!(!base.join("elsewhere/X.flm").exists());
        std::fs::remove_dir_all(&base).unwrap();
    }

    /// A forged entry below the user folder in writing that a link (`Unzipped songs`, one to a folder outside) leads
    /// elsewhere is refused: nothing outside the user folder goes into the bin, or is removed.
    #[cfg(unix)]
    #[test]
    fn delete_refuses_what_a_link_leads_outside_the_user_folder() {
        use crate::project_files::content::tests::Temp;
        let base = Temp::new("ops-unzipped-link");
        let (user, elsewhere) = (base.0.join("user"), base.0.join("elsewhere"));
        let root = user.join(SONGS_FOLDER);
        std::fs::create_dir_all(&root).unwrap();
        std::fs::create_dir_all(elsewhere.join("F")).unwrap();
        std::fs::write(elsewhere.join("X.wav"), b"x").unwrap();
        std::fs::write(elsewhere.join("F/Y.wav"), b"y").unwrap();
        std::os::unix::fs::symlink(&elsewhere, user.join("Unzipped songs")).unwrap();
        let mut lib = NativeLibrary::at(root.clone());
        let forged = |name: &str, is_folder| SongEntry {
            name: name.into(),
            path: user.join("Unzipped songs").join(name),
            is_folder,
            recency: 0,
        };
        for e in [forged("X.wav", false), forged("F", true)] {
            let err = delete(&mut lib, std::slice::from_ref(&e), &mut Vec::new()).unwrap_err();
            assert!(err.contains("cannot be deleted"), "{err}");
        }
        assert_eq!(std::fs::read(elsewhere.join("X.wav")).unwrap(), b"x");
        assert_eq!(std::fs::read(elsewhere.join("F/Y.wav")).unwrap(), b"y");
        assert!(!user.join(RECENTLY_DELETED).exists());
        // What really is in the songs folder still goes into the bin.
        std::fs::write(root.join("A.flm"), b"a").unwrap();
        let picked = [entry(&lib, None, "A.flm")];
        delete(&mut lib, &picked, &mut Vec::new()).unwrap();
        let bin = user.join(RECENTLY_DELETED).join(SONGS_FOLDER);
        assert_eq!(std::fs::read(bin.join("A.flm")).unwrap(), b"a");
        // So does what is in a songs folder that is itself a link (the songs kept on another disk): it is the root.
        let (other, disk) = (base.0.join("other"), base.0.join("disk"));
        std::fs::create_dir_all(&other).unwrap();
        std::fs::create_dir_all(&disk).unwrap();
        std::fs::write(disk.join("B.flm"), b"b").unwrap();
        std::os::unix::fs::symlink(&disk, other.join(SONGS_FOLDER)).unwrap();
        let mut lib = NativeLibrary::at(other.join(SONGS_FOLDER));
        let picked = [entry(&lib, None, "B.flm")];
        delete(&mut lib, &picked, &mut Vec::new()).unwrap();
        let bin = other.join(RECENTLY_DELETED).join(SONGS_FOLDER);
        assert_eq!(std::fs::read(bin.join("B.flm")).unwrap(), b"b");
        assert!(!disk.join("B.flm").exists());
    }

    /// On a disk that ignores case, a forged entry naming the user's song in another case (`USER/My Songs/A.flm`) is
    /// not provably outside the user folder: refused, never removed for good, whatever the resolved path's case.
    #[cfg(any(windows, target_os = "macos"))]
    #[test]
    fn delete_never_removes_a_case_variant_of_the_users_song_for_good() {
        use crate::project_files::content::tests::Temp;
        let base = Temp::new("ops-case-forged");
        let root = base.0.join("user").join(SONGS_FOLDER);
        std::fs::create_dir_all(&root).unwrap();
        std::fs::write(root.join("A.flm"), b"a").unwrap();
        let mut lib = NativeLibrary::at(root.clone());
        let forged = SongEntry {
            name: "A.flm".into(),
            path: base.0.join("USER").join(SONGS_FOLDER).join("A.flm"),
            is_folder: false,
            recency: 0,
        };
        assert!(delete(&mut lib, &[forged], &mut Vec::new()).is_err());
        assert_eq!(std::fs::read(root.join("A.flm")).unwrap(), b"a");
    }

    /// What a link in the bin leads to is no bin item: deleting `Recently Deleted/lnk/x`, with `lnk` a link to a folder
    /// outside, is refused, never done for good, and what it points at stays.
    #[cfg(unix)]
    #[test]
    fn delete_in_the_bin_refuses_what_a_link_there_leads_outside() {
        use crate::project_files::content::tests::Temp;
        let base = Temp::new("ops-bin-lnk");
        let (user, elsewhere) = (base.0.join("user"), base.0.join("elsewhere"));
        let root = user.join(SONGS_FOLDER);
        let bin = user.join(RECENTLY_DELETED);
        std::fs::create_dir_all(&root).unwrap();
        std::fs::create_dir_all(&bin).unwrap();
        std::fs::create_dir_all(&elsewhere).unwrap();
        std::fs::write(elsewhere.join("x"), b"x").unwrap();
        std::os::unix::fs::symlink(&elsewhere, bin.join("lnk")).unwrap();
        let mut lib = NativeLibrary::at(root);
        let forged = SongEntry {
            name: "x".into(),
            path: bin.join("lnk/x"),
            is_folder: false,
            recency: 0,
        };
        let err = delete(&mut lib, &[forged], &mut Vec::new()).unwrap_err();
        assert!(err.contains("cannot be deleted"), "{err}");
        assert_eq!(std::fs::read(elsewhere.join("x")).unwrap(), b"x");
    }

    /// `Delete permanently` removes only what really is in the bin: `Recently Deleted/lnk/x`, with `lnk` a link to a
    /// folder outside, is refused and what it points at stays; `lnk` itself is never followed.
    #[cfg(unix)]
    #[test]
    fn delete_permanently_never_follows_a_link_out_of_the_bin() {
        use crate::project_files::content::tests::Temp;
        let base = Temp::new("ops-purge-lnk");
        let (user, elsewhere) = (base.0.join("user"), base.0.join("elsewhere"));
        let bin = user.join(RECENTLY_DELETED);
        std::fs::create_dir_all(user.join(SONGS_FOLDER)).unwrap();
        std::fs::create_dir_all(&bin).unwrap();
        std::fs::create_dir_all(&elsewhere).unwrap();
        std::fs::write(elsewhere.join("x"), b"x").unwrap();
        std::fs::write(bin.join("A.flm"), b"a").unwrap();
        std::os::unix::fs::symlink(&elsewhere, bin.join("lnk")).unwrap();
        let mut lib = NativeLibrary::at(user.join(SONGS_FOLDER));
        let at = |path: PathBuf, is_folder| SongEntry {
            name: "forged".into(),
            path,
            is_folder,
            recency: 0,
        };
        let a = at(bin.join("A.flm"), false);
        for forged in [at(bin.join("lnk/x"), false), at(bin.join("lnk"), true)] {
            let picked = [a.clone(), forged];
            let err = delete_permanently(&mut lib, &picked, &mut Vec::new()).unwrap_err();
            assert!(err.contains("cannot be deleted"), "{err}");
        }
        assert_eq!(std::fs::read(elsewhere.join("x")).unwrap(), b"x");
        // Nothing went before the refusal, and what is in the bin still goes.
        assert!(bin.join("A.flm").exists());
        delete_permanently(&mut lib, &[a], &mut Vec::new()).unwrap();
        assert!(!bin.join("A.flm").exists());
    }

    /// A link in the bin's own path would put what is deleted out of sight, where the purge never looks: refused.
    #[cfg(unix)]
    #[test]
    fn delete_refuses_a_link_on_the_way_into_the_bin() {
        use std::os::unix::fs::symlink;
        let base =
            std::env::temp_dir().join(format!("honeydew-ops-bin-link-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&base);
        let (user, elsewhere) = (base.join("user"), base.join("elsewhere"));
        let root = user.join(SONGS_FOLDER);
        std::fs::create_dir_all(&root).unwrap();
        std::fs::create_dir_all(&elsewhere).unwrap();
        std::fs::write(root.join("A.flm"), b"a").unwrap();
        let mut lib = NativeLibrary::at(root.clone());
        let bin = user.join(RECENTLY_DELETED);
        let refused = |lib: &mut NativeLibrary| {
            let picked = [entry(lib, None, "A.flm")];
            let err = delete(lib, &picked, &mut Vec::new()).unwrap_err();
            assert!(err.ends_with(": a link"), "{err}");
            assert_eq!(std::fs::read(root.join("A.flm")).unwrap(), b"a");
            assert_eq!(std::fs::read_dir(&elsewhere).unwrap().count(), 0);
        };
        // `Recently Deleted` a link.
        symlink(&elsewhere, &bin).unwrap();
        refused(&mut lib);
        // Its `My Songs` a link.
        std::fs::remove_file(&bin).unwrap();
        std::fs::create_dir(&bin).unwrap();
        symlink(&elsewhere, bin.join(SONGS_FOLDER)).unwrap();
        refused(&mut lib);
        // Without them it goes in.
        std::fs::remove_file(bin.join(SONGS_FOLDER)).unwrap();
        let picked = [entry(&lib, None, "A.flm")];
        delete(&mut lib, &picked, &mut Vec::new()).unwrap();
        assert_eq!(std::fs::read(bin.join("My Songs/A.flm")).unwrap(), b"a");
        std::fs::remove_dir_all(&base).unwrap();
    }

    /// A memory library whose `move_to` fails as a rename across file systems does, and whose `remove` fails for one
    /// path.
    struct Flaky {
        inner: MemoryLibrary,
        cross: bool,
        stuck: Option<PathBuf>,
        /// `copy_to` makes the copy, then fails, as a full disk does part way.
        broken_copy: bool,
    }

    impl SongLibrary for Flaky {
        fn list(&self, folder: Option<&Path>) -> std::io::Result<Vec<SongEntry>> {
            self.inner.list(folder)
        }
        fn read(&self, path: &Path) -> std::io::Result<Vec<u8>> {
            self.inner.read(path)
        }
        fn store(&mut self, name: &str, bytes: &[u8]) -> std::io::Result<()> {
            self.inner.store(name, bytes)
        }
        fn store_in(&mut self, folder: &Path, name: &str, bytes: &[u8]) -> std::io::Result<()> {
            self.inner.store_in(folder, name, bytes)
        }
        fn root(&self) -> Option<&Path> {
            self.inner.root()
        }
        fn trash(&self) -> Option<PathBuf> {
            self.inner.trash()
        }
        fn bin_path(&self, path: &Path) -> Option<PathBuf> {
            self.inner.bin_path(path)
        }
        fn restore_path(&self, path: &Path) -> Option<PathBuf> {
            self.inner.restore_path(path)
        }
        fn exists(&self, path: &Path) -> bool {
            self.inner.exists(path)
        }
        fn make_folder(&mut self, path: &Path) -> std::io::Result<()> {
            self.inner.make_folder(path)
        }
        fn move_to(&mut self, from: &Path, to: &Path) -> std::io::Result<()> {
            if self.cross {
                return Err(std::io::ErrorKind::CrossesDevices.into());
            }
            self.inner.move_to(from, to)
        }
        fn copy_to(&mut self, from: &Path, to: &Path) -> std::io::Result<()> {
            self.inner.copy_to(from, to)?;
            match self.broken_copy {
                true => Err(std::io::Error::other("disk full")),
                false => Ok(()),
            }
        }
        fn remove(&mut self, path: &Path) -> std::io::Result<()> {
            if self.stuck.as_deref() == Some(path) {
                return Err(std::io::Error::other("busy"));
            }
            self.inner.remove(path)
        }
    }

    /// The songs folder on another file system: a rename out of the bin fails with `CrossesDevices`, every time, so
    /// Restore copies, then removes, and keeps the copy if the removal fails.
    #[test]
    fn restore_across_file_systems_copies_then_removes_and_keeps_the_copy_on_a_failed_removal() {
        let mut lib = Flaky {
            inner: MemoryLibrary::default(),
            cross: false,
            stuck: None,
            broken_copy: false,
        };
        let root = root(&lib);
        let f = create_folder(&mut lib, &root, "F").unwrap();
        put(&mut lib, Some(&f), "B.flm", b"b");
        put(&mut lib, Some(&f), "C.flm", b"c");
        let picked = [
            entry(&lib, Some(&f), "B.flm"),
            entry(&lib, Some(&f), "C.flm"),
        ];
        delete(&mut lib, &picked, &mut Vec::new()).unwrap();
        let bin = bin_root(&lib).join("F");
        lib.cross = true;
        // A plain rename fails, so the file is copied back and the bin's one removed.
        let mut moved = Vec::new();
        let picked = [entry(&lib, Some(&bin), "B.flm")];
        restore(&mut lib, &picked, &mut moved).unwrap();
        assert_eq!(lib.read(&f.join("B.flm")).unwrap(), b"b");
        assert!(!lib.exists(&bin.join("B.flm")));
        assert_eq!(moved, [(bin.join("B.flm"), Some(f.join("B.flm")))]);
        // The bin's copy cannot be removed: the restored one stays, and so does the bin's, never neither.
        lib.stuck = Some(bin.join("C.flm"));
        let picked = [entry(&lib, Some(&bin), "C.flm")];
        let mut moved = Vec::new();
        assert!(restore(&mut lib, &picked, &mut moved).is_err());
        assert_eq!(lib.read(&f.join("C.flm")).unwrap(), b"c");
        assert_eq!(lib.read(&bin.join("C.flm")).unwrap(), b"c");
        assert_eq!(moved, [(bin.join("C.flm"), Some(f.join("C.flm")))]);
    }

    /// Paste after Cut renames, and a rename across file systems fails with `CrossesDevices` every time: it copies,
    /// then removes, as Restore does.
    #[test]
    fn paste_after_cut_across_file_systems_copies_then_removes_and_keeps_the_copy_on_a_failed_removal()
     {
        let mut lib = Flaky {
            inner: MemoryLibrary::default(),
            cross: false,
            stuck: None,
            broken_copy: false,
        };
        let root = root(&lib);
        let g = create_folder(&mut lib, &root, "G").unwrap();
        put(&mut lib, None, "A.flm", b"a");
        let f = create_folder(&mut lib, &root, "F").unwrap();
        put(&mut lib, Some(&f), "B.flm", b"b");
        lib.cross = true;
        let cut = |lib: &Flaky, names: &[&str]| Clipboard {
            entries: names.iter().map(|n| entry(lib, None, n)).collect(),
            cut: true,
        };
        let mut moved = Vec::new();
        let clip = cut(&lib, &["A.flm", "F"]);
        paste(&mut lib, &clip, &g, &mut moved).unwrap();
        assert_eq!(names(&lib, None), ["G/"]);
        assert_eq!(names(&lib, Some(&g)), ["A.flm", "F/"]);
        assert_eq!(lib.read(&g.join("A.flm")).unwrap(), b"a");
        assert_eq!(lib.read(&g.join("F/B.flm")).unwrap(), b"b");
        let to = |n: &str| (root.join(n), Some(g.join(n)));
        assert_eq!(moved, [to("A.flm"), to("F")]);
        // A source that cannot be removed: the copy stays, the source too, never neither.
        put(&mut lib, None, "A.flm", b"new");
        lib.stuck = Some(root.join("A.flm"));
        let mut moved = Vec::new();
        let clip = cut(&lib, &["A.flm"]);
        assert!(paste(&mut lib, &clip, &g, &mut moved).is_err());
        assert_eq!(lib.read(&root.join("A.flm")).unwrap(), b"new");
        assert_eq!(lib.read(&g.join("A (1).flm")).unwrap(), b"new");
        assert_eq!(moved, [(root.join("A.flm"), Some(g.join("A (1).flm")))]);
    }

    #[test]
    fn a_failed_copy_across_file_systems_leaves_no_partial_copy_and_the_source_stays() {
        let mut lib = Flaky {
            inner: MemoryLibrary::default(),
            cross: false,
            stuck: None,
            broken_copy: false,
        };
        let root = root(&lib);
        let g = create_folder(&mut lib, &root, "G").unwrap();
        put(&mut lib, None, "A.flm", b"a");
        let f = create_folder(&mut lib, &root, "F").unwrap();
        put(&mut lib, Some(&f), "B.flm", b"b");
        let picked = [entry(&lib, None, "F")];
        delete(&mut lib, &picked, &mut Vec::new()).unwrap();
        lib.cross = true;
        lib.broken_copy = true;
        // Paste after Cut.
        let clip = Clipboard {
            entries: vec![entry(&lib, None, "A.flm")],
            cut: true,
        };
        let mut moved = Vec::new();
        assert!(paste(&mut lib, &clip, &g, &mut moved).is_err());
        assert_eq!(names(&lib, Some(&g)), Vec::<String>::new());
        assert_eq!(lib.read(&root.join("A.flm")).unwrap(), b"a");
        // Restore.
        let bin = bin_root(&lib);
        let picked = [entry(&lib, Some(&bin), "F")];
        assert!(restore(&mut lib, &picked, &mut moved).is_err());
        assert_eq!(names(&lib, None), ["A.flm", "G/", "Recently Deleted/"]);
        assert_eq!(lib.read(&bin.join("F/B.flm")).unwrap(), b"b");
        assert_eq!(moved, []);
    }

    /// A native library on a disk that ignores case (FAT and exFAT on a USB stick, macOS by default): `exists` finds a
    /// name in any case, `move_to` fails as a rename across disks does, and `copy_to` is the real tree copy.
    #[cfg(not(target_arch = "wasm32"))]
    struct Folded {
        inner: NativeLibrary,
    }

    #[cfg(not(target_arch = "wasm32"))]
    impl SongLibrary for Folded {
        fn list(&self, folder: Option<&Path>) -> std::io::Result<Vec<SongEntry>> {
            self.inner.list(folder)
        }
        fn read(&self, path: &Path) -> std::io::Result<Vec<u8>> {
            self.inner.read(path)
        }
        fn store(&mut self, name: &str, bytes: &[u8]) -> std::io::Result<()> {
            self.inner.store(name, bytes)
        }
        fn root(&self) -> Option<&Path> {
            self.inner.root()
        }
        fn trash(&self) -> Option<PathBuf> {
            self.inner.trash()
        }
        fn bin_path(&self, path: &Path) -> Option<PathBuf> {
            self.inner.bin_path(path)
        }
        fn restore_path(&self, path: &Path) -> Option<PathBuf> {
            self.inner.restore_path(path)
        }
        fn canonical(&self, path: &Path) -> std::io::Result<PathBuf> {
            self.inner.canonical(path)
        }
        fn is_link(&self, path: &Path) -> bool {
            self.inner.is_link(path)
        }
        fn exists(&self, path: &Path) -> bool {
            let (Some(dir), Some(name)) = (path.parent(), path.file_name()) else {
                return self.inner.exists(path);
            };
            let name = name.to_string_lossy().to_lowercase();
            let same = |e: std::io::Result<std::fs::DirEntry>| {
                e.is_ok_and(|e| e.file_name().to_string_lossy().to_lowercase() == name)
            };
            self.inner.exists(path) || std::fs::read_dir(dir).is_ok_and(|mut d| d.any(same))
        }
        fn make_folder(&mut self, path: &Path) -> std::io::Result<()> {
            self.inner.make_folder(path)
        }
        fn move_to(&mut self, _from: &Path, _to: &Path) -> std::io::Result<()> {
            Err(std::io::ErrorKind::CrossesDevices.into())
        }
        fn copy_to(&mut self, from: &Path, to: &Path) -> std::io::Result<()> {
            crate::ui::main_menu::library::copy_tree(self, from, to, 0)
        }
        fn remove(&mut self, path: &Path) -> std::io::Result<()> {
            self.inner.remove(path)
        }
    }

    /// `Song.flm` and `song.flm` are two files on a disk that tells case apart, one on a disk that does not: the
    /// second copied would replace the first, and then the source is removed. The copy fails instead, the source
    /// stays whole and no partial copy is left. (A real case-insensitive disk is not needed: the library says
    /// which names are taken.)
    #[cfg(not(target_arch = "wasm32"))]
    #[test]
    fn a_folder_whose_names_differ_only_in_case_is_not_copied_onto_a_disk_that_ignores_case() {
        let base = std::env::temp_dir().join(format!("honeydew-ops-case-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&base);
        let (root, bin) = (
            base.join(SONGS_FOLDER),
            base.join("Recently Deleted/My Songs"),
        );
        for dir in [root.join("F"), root.join("G"), bin.join("F")] {
            std::fs::create_dir_all(&dir).unwrap();
            std::fs::write(dir.join("Song.flm"), b"upper").unwrap();
            std::fs::write(dir.join("song.flm"), b"lower").unwrap();
        }
        // On a host that ignores case itself the two names cannot be set up.
        if std::fs::read_dir(root.join("F")).unwrap().count() != 2 {
            std::fs::remove_dir_all(&base).unwrap();
            return;
        }
        std::fs::remove_dir_all(root.join("G")).unwrap();
        std::fs::create_dir(root.join("G")).unwrap();
        let mut lib = Folded {
            inner: NativeLibrary::at(root.clone()),
        };
        let whole = |dir: &Path| {
            assert_eq!(std::fs::read(dir.join("Song.flm")).unwrap(), b"upper");
            assert_eq!(std::fs::read(dir.join("song.flm")).unwrap(), b"lower");
        };
        // Paste after Cut.
        let clip = Clipboard {
            entries: vec![entry(&lib, None, "F")],
            cut: true,
        };
        assert!(paste(&mut lib, &clip, &root.join("G"), &mut Vec::new()).is_err());
        whole(&root.join("F"));
        assert!(!root.join("G/F").exists(), "a partial copy is left");
        // Delete.
        let picked = [entry(&lib, None, "F")];
        assert!(delete(&mut lib, &picked, &mut Vec::new()).is_err());
        whole(&root.join("F"));
        assert!(
            !bin.join("F (1)").exists(),
            "a partial copy is left in the bin"
        );
        // Restore.
        let picked = [entry(&lib, Some(&bin), "F")];
        assert!(restore(&mut lib, &picked, &mut Vec::new()).is_err());
        whole(&bin.join("F"));
        assert!(!root.join("F (1)").exists(), "a partial copy is left");
        // The error is not "already exists", which the cleanup takes for a name taken before the copy.
        let kind = |r: std::io::Result<()>| r.unwrap_err().kind();
        let (f, g) = (root.join("F"), root.join("G/Copy"));
        let copied = crate::ui::main_menu::library::copy_tree(&lib, &f, &g, 0);
        assert_ne!(kind(copied), std::io::ErrorKind::AlreadyExists);
        assert!(
            g.exists(),
            "the copy itself leaves a partial one for the caller to take back"
        );
        std::fs::remove_dir_all(&base).unwrap();
    }

    /// An earlier, flat bin held its songs at the top: they go back into the songs root, not beside `My Songs`. (A folder
    /// at the top is the whole user folder's: it goes back beside `My Songs`.)
    #[test]
    fn restore_puts_what_a_flat_bin_held_into_the_root() {
        both("restore-flat", |lib| {
            let (root, trash) = (root(lib), trash(lib));
            lib.make_folder(&trash).unwrap();
            put(lib, Some(&trash), "Old.flm", b"o");
            let picked = [entry(lib, Some(&trash), "Old.flm")];
            restore(lib, &picked, &mut Vec::new()).unwrap();
            assert_eq!(names(lib, None), ["Old.flm", "Recently Deleted/"]);
            assert_eq!(lib.read(&root.join("Old.flm")).unwrap(), b"o");
            // Nothing was made beside the songs folder.
            if let Some(user) = root.parent().filter(|p| !p.as_os_str().is_empty()) {
                let mut beside: Vec<_> = std::fs::read_dir(user)
                    .unwrap()
                    .map(|e| e.unwrap().file_name().into_string().unwrap())
                    .collect();
                beside.sort();
                assert_eq!(beside, ["My Songs", "Recently Deleted"]);
            }
        });
    }

    #[test]
    fn delete_permanently_removes_only_what_is_in_recently_deleted() {
        both("purge", |lib| {
            let root = root(lib);
            put(lib, None, "A.flm", b"a");
            let folder = create_folder(lib, &root, "F").unwrap();
            put(lib, Some(&folder), "B.flm", b"b");
            {
                let picked = [entry(lib, None, "A.flm"), entry(lib, None, "F")];
                delete(lib, &picked, &mut Vec::new())
            }
            .unwrap();
            let (trash, bin) = (trash(lib), bin_root(lib));
            let picked = [entry(lib, Some(&bin), "A.flm"), entry(lib, Some(&bin), "F")];
            delete_permanently(lib, &picked, &mut Vec::new()).unwrap();
            assert!(names(lib, Some(&bin)).is_empty());
            assert!(!lib.exists(&bin.join("F").join("B.flm")));
            put(lib, None, "C.flm", b"c");
            assert!(
                {
                    let picked = [entry(lib, None, "C.flm")];
                    delete_permanently(lib, &picked, &mut Vec::new())
                }
                .is_err()
            );
            assert!(
                {
                    let picked = [entry(lib, None, RECENTLY_DELETED)];
                    delete_permanently(lib, &picked, &mut Vec::new())
                }
                .is_err()
            );
            assert_eq!(lib.read(&root.join("C.flm")).unwrap(), b"c");
            assert!(lib.exists(&trash));
        });
    }

    #[test]
    fn paste_after_copy_copies_under_free_names() {
        both("copy", |lib| {
            let root = root(lib);
            put(lib, None, "A.flm", b"a");
            let folder = create_folder(lib, &root, "F").unwrap();
            put(lib, Some(&folder), "B.flm", b"b");
            let clip = Clipboard {
                entries: vec![entry(lib, None, "A.flm"), entry(lib, None, "F")],
                cut: false,
            };
            paste(lib, &clip, &root, &mut Vec::new()).unwrap();
            assert_eq!(names(lib, None), ["A (1).flm", "A.flm", "F (1)/", "F/"]);
            assert_eq!(lib.read(&root.join("A (1).flm")).unwrap(), b"a");
            assert_eq!(names(lib, Some(&root.join("F (1)"))), ["B.flm"]);
            paste(lib, &clip, &root, &mut Vec::new()).unwrap();
            assert_eq!(
                names(lib, None),
                ["A (1).flm", "A (2).flm", "A.flm", "F (1)/", "F (2)/", "F/"]
            );
            // Into another folder, under the same name.
            let one = Clipboard {
                entries: vec![entry(lib, None, "A.flm")],
                cut: false,
            };
            paste(lib, &one, &folder, &mut Vec::new()).unwrap();
            assert_eq!(names(lib, Some(&folder)), ["A.flm", "B.flm"]);
            // A folder never goes into itself.
            let f = Clipboard {
                entries: vec![entry(lib, None, "F")],
                cut: false,
            };
            assert!(paste(lib, &f, &folder, &mut Vec::new()).is_err());
            assert_eq!(names(lib, Some(&folder)), ["A.flm", "B.flm"]);
        });
    }

    #[test]
    fn paste_after_cut_moves_under_free_names() {
        both("cut", |lib| {
            let root = root(lib);
            let f = create_folder(lib, &root, "F").unwrap();
            create_folder(lib, &root, "G").unwrap();
            // Stored at the root, then moved: first.
            put(lib, Some(&f), "A.flm", b"fa");
            put(lib, None, "A.flm", b"a");
            let clip = Clipboard {
                entries: vec![entry(lib, None, "A.flm"), entry(lib, None, "G")],
                cut: true,
            };
            // Into the folder they are in: nothing moves.
            paste(lib, &clip, &root, &mut Vec::new()).unwrap();
            assert_eq!(names(lib, None), ["A.flm", "F/", "G/"]);
            paste(lib, &clip, &f, &mut Vec::new()).unwrap();
            assert_eq!(names(lib, None), ["F/"]);
            assert_eq!(names(lib, Some(&f)), ["A (1).flm", "A.flm", "G/"]);
            assert_eq!(lib.read(&f.join("A (1).flm")).unwrap(), b"a");
            assert_eq!(lib.read(&f.join("A.flm")).unwrap(), b"fa");
            let g = Clipboard {
                entries: vec![entry(lib, Some(&f), "G")],
                cut: true,
            };
            assert!(paste(lib, &g, &f.join("G"), &mut Vec::new()).is_err());
            assert_eq!(names(lib, Some(&f)), ["A (1).flm", "A.flm", "G/"]);
        });
    }

    /// A preset picker's files: a preset keeps its own type.
    #[test]
    fn rename_keeps_any_files_own_extension() {
        let mut lib = MemoryLibrary::default();
        lib.put(PathBuf::from("Hall.flmpst"), b"p").unwrap();
        let hall = entry(&lib, None, "Hall.flmpst");
        let room = rename(&mut lib, &hall, "Room", &mut Vec::new());
        assert_eq!(room.unwrap(), PathBuf::from("Room.flmpst"));
        assert_eq!(lib.read(Path::new("Room.flmpst")).unwrap(), b"p");
    }

    #[test]
    fn rename_takes_a_good_free_name_and_keeps_the_extension() {
        both("rename", |lib| {
            let root = root(lib);
            put(lib, None, "A.flm", b"a");
            put(lib, None, "B.flm", b"b");
            put(lib, None, "Up.FLM", b"u");
            create_folder(lib, &root, "F").unwrap();
            let a = entry(lib, None, "A.flm");
            assert_eq!(
                rename(lib, &a, " Song ", &mut Vec::new()).unwrap(),
                root.join("Song.flm")
            );
            assert_eq!(lib.read(&root.join("Song.flm")).unwrap(), b"a");
            // A typed extension is not doubled; the file keeps its own.
            let song = entry(lib, None, "Song.flm");
            assert_eq!(
                rename(lib, &song, "Tune.flm", &mut Vec::new()).unwrap(),
                root.join("Tune.flm")
            );
            let up = entry(lib, None, "Up.FLM");
            assert_eq!(
                rename(lib, &up, "Down", &mut Vec::new()).unwrap(),
                root.join("Down.FLM")
            );
            // A taken name is refused, and nothing moves.
            let tune = entry(lib, None, "Tune.flm");
            assert_eq!(
                rename(lib, &tune, "B", &mut Vec::new()).unwrap_err(),
                "The file already exists"
            );
            assert_eq!(lib.read(&root.join("B.flm")).unwrap(), b"b");
            assert_eq!(
                rename(lib, &tune, "a*", &mut Vec::new()).unwrap_err(),
                "Symbol '*' is not allowed in a file name."
            );
            for bad in ["", "a/b", "CON", ".x"] {
                assert!(rename(lib, &tune, bad, &mut Vec::new()).is_err(), "{bad:?}");
            }
            // The same name: nothing to do.
            assert_eq!(
                rename(lib, &tune, "Tune", &mut Vec::new()).unwrap(),
                tune.path
            );
            // A folder keeps its whole name.
            let f = entry(lib, None, "F");
            assert_eq!(
                rename(lib, &f, "v1.2", &mut Vec::new()).unwrap(),
                root.join("v1.2")
            );
            let v = entry(lib, None, "v1.2");
            assert_eq!(
                rename(lib, &v, "B.flm", &mut Vec::new()).unwrap_err(),
                "The folder already exists"
            );
            assert_eq!(names(lib, None), ["B.flm", "Down.FLM", "Tune.flm", "v1.2/"]);
        });
    }

    #[test]
    fn move_and_copy_never_replace_a_target_or_go_into_their_source() {
        use std::io::ErrorKind::{AlreadyExists, InvalidInput};
        both("contract", |lib| {
            let root = root(lib);
            put(lib, None, "A.flm", b"a");
            put(lib, None, "B.flm", b"b");
            let f = create_folder(lib, &root, "F").unwrap();
            put(lib, Some(&f), "C.flm", b"c");
            let (a, b) = (root.join("A.flm"), root.join("B.flm"));
            for (from, to) in [(&a, &b), (&f, &b), (&a, &f)] {
                assert_eq!(lib.move_to(from, to).unwrap_err().kind(), AlreadyExists);
                assert_eq!(lib.copy_to(from, to).unwrap_err().kind(), AlreadyExists);
            }
            let inside = f.join("Sub");
            assert_eq!(lib.move_to(&f, &inside).unwrap_err().kind(), InvalidInput);
            assert_eq!(lib.copy_to(&f, &inside).unwrap_err().kind(), InvalidInput);
            assert_eq!(lib.read(&b).unwrap(), b"b");
            assert_eq!(names(lib, None), ["A.flm", "B.flm", "F/"]);
            assert_eq!(names(lib, Some(&f)), ["C.flm"]);
        });
    }

    #[cfg(not(target_arch = "wasm32"))]
    #[test]
    fn export_refuses_a_folder_inside_what_it_exports() {
        let base =
            std::env::temp_dir().join(format!("honeydew-ops-export-in-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&base);
        let root = base.join(SONGS_FOLDER);
        let mut lib = NativeLibrary::at(root.clone());
        let f = create_folder(&mut lib, &root, "F").unwrap();
        std::fs::write(f.join("B.flm"), b"b").unwrap();
        let picked = [entry(&lib, None, "F")];
        assert!(export_into(&mut lib, &picked, &f).is_err());
        assert_eq!(names(&lib, Some(&f)), ["B.flm"]);
        std::fs::remove_dir_all(&base).unwrap();
    }

    /// A folder reached through a link to the source is still inside it: no copy of a folder into itself.
    #[cfg(unix)]
    #[test]
    fn a_link_to_the_source_does_not_get_a_folder_copied_into_itself() {
        let base = std::env::temp_dir().join(format!("honeydew-ops-alias-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&base);
        let root = base.join(SONGS_FOLDER);
        let mut lib = NativeLibrary::at(root.clone());
        let f = create_folder(&mut lib, &root, "F").unwrap();
        std::fs::write(f.join("B.flm"), b"b").unwrap();
        let alias = base.join("alias");
        std::os::unix::fs::symlink(&f, &alias).unwrap();
        let to = alias.join("F");
        assert_eq!(
            lib.copy_to(&f, &to).unwrap_err().kind(),
            std::io::ErrorKind::InvalidInput
        );
        assert_eq!(
            lib.move_to(&f, &to).unwrap_err().kind(),
            std::io::ErrorKind::InvalidInput
        );
        let picked = [entry(&lib, None, "F")];
        assert!(export_into(&mut lib, &picked, &alias).is_err());
        assert_eq!(names(&lib, Some(&f)), ["B.flm"]);
        std::fs::remove_dir_all(&base).unwrap();
    }

    #[test]
    fn every_move_and_delete_is_reported_for_the_open_song() {
        both("moved", |lib| {
            let root = root(lib);
            put(lib, None, "A.flm", b"a");
            put(lib, None, "B.flm", b"b");
            let f = create_folder(lib, &root, "F").unwrap();
            let mut moved = Vec::new();
            let a = entry(lib, None, "A.flm");
            rename(lib, &a, "C", &mut moved).unwrap();
            assert_eq!(moved, [(root.join("A.flm"), Some(root.join("C.flm")))]);
            // A copy moves nothing, and nor does a cut pasted where it is.
            moved.clear();
            let picked = vec![entry(lib, None, "C.flm"), entry(lib, None, "B.flm")];
            let copy = Clipboard {
                entries: picked.clone(),
                cut: false,
            };
            paste(lib, &copy, &root, &mut moved).unwrap();
            let cut = Clipboard {
                entries: picked[..1].to_vec(),
                cut: true,
            };
            paste(lib, &cut, &root, &mut moved).unwrap();
            assert_eq!(moved, []);
            paste(lib, &cut, &f, &mut moved).unwrap();
            assert_eq!(moved, [(root.join("C.flm"), Some(f.join("C.flm")))]);
            // Deleted, even into Recently Deleted, is gone; restored is moved.
            moved.clear();
            let picked = [entry(lib, None, "F"), entry(lib, None, "B.flm")];
            delete(lib, &picked, &mut moved).unwrap();
            assert_eq!(moved, [(f.clone(), None), (root.join("B.flm"), None)]);
            moved.clear();
            let bin = bin_root(lib);
            let picked = [entry(lib, Some(&bin), "F")];
            restore(lib, &picked, &mut moved).unwrap();
            assert_eq!(moved, [(bin.join("F"), Some(f.clone()))]);
            moved.clear();
            let picked = [entry(lib, Some(&bin), "B.flm")];
            delete_permanently(lib, &picked, &mut moved).unwrap();
            assert_eq!(moved, [(bin.join("B.flm"), None)]);
        });
    }

    #[test]
    fn free_names_count_up_before_the_extension() {
        let mut lib = MemoryLibrary::default();
        lib.store("A.flm", b"").unwrap();
        lib.store("A (1).flm", b"").unwrap();
        let dir = Path::new("");
        assert_eq!(
            free_path(&lib, dir, "A.flm".as_ref(), false),
            Path::new("A (2).flm")
        );
        assert_eq!(
            free_path(&lib, dir, "B.flm".as_ref(), false),
            Path::new("B.flm")
        );
        lib.make_folder(Path::new("v1.2")).unwrap();
        assert_eq!(
            free_path(&lib, dir, "v1.2".as_ref(), true),
            Path::new("v1.2 (1)")
        );
    }

    #[cfg(unix)]
    #[test]
    fn links_are_never_moved_copied_removed_or_followed() {
        use std::os::unix::fs::symlink;
        let base = std::env::temp_dir().join(format!("honeydew-ops-links-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&base);
        let (root, outside) = (base.join(SONGS_FOLDER), base.join("outside"));
        std::fs::create_dir_all(root.join("F")).unwrap();
        std::fs::create_dir_all(outside.join("Folder")).unwrap();
        std::fs::write(outside.join("Folder").join("X.flm"), b"x").unwrap();
        std::fs::write(outside.join("Out.flm"), b"out").unwrap();
        std::fs::write(root.join("F").join("Song.flm"), b"s").unwrap();
        symlink(outside.join("Out.flm"), root.join("Link.flm")).unwrap();
        symlink(outside.join("Folder"), root.join("F").join("Inner")).unwrap();
        let mut lib = NativeLibrary::at(root.clone());
        // The list leaves links out; a stale or forged entry for one is refused.
        let link = SongEntry {
            name: "Link.flm".into(),
            path: root.join("Link.flm"),
            is_folder: false,
            recency: 0,
        };
        let copy = |entries| Clipboard {
            entries,
            cut: false,
        };
        assert!(delete(&mut lib, std::slice::from_ref(&link), &mut Vec::new()).is_err());
        assert!(rename(&mut lib, &link, "Other", &mut Vec::new()).is_err());
        assert!(
            paste(
                &mut lib,
                &copy(vec![link.clone()]),
                &root.join("F"),
                &mut Vec::new()
            )
            .is_err()
        );
        assert!(lib.remove(&link.path).is_err());
        assert!(std::fs::symlink_metadata(&link.path).unwrap().is_symlink());
        // A folder holding a link is copied without it, and deleted without following it.
        let f = entry(&lib, None, "F");
        paste(&mut lib, &copy(vec![f.clone()]), &root, &mut Vec::new()).unwrap();
        let copied: Vec<_> = std::fs::read_dir(root.join("F (1)"))
            .unwrap()
            .map(|e| e.unwrap().file_name())
            .collect();
        assert_eq!(copied, ["Song.flm"]);
        delete(&mut lib, &[f], &mut Vec::new()).unwrap();
        let bin = bin_root(&lib);
        {
            let picked = [entry(&lib, Some(&bin), "F")];
            delete_permanently(&mut lib, &picked, &mut Vec::new())
        }
        .unwrap();
        assert_eq!(
            std::fs::read(outside.join("Folder").join("X.flm")).unwrap(),
            b"x"
        );
        assert_eq!(std::fs::read(outside.join("Out.flm")).unwrap(), b"out");
        std::fs::remove_dir_all(&base).unwrap();
    }

    #[cfg(not(target_arch = "wasm32"))]
    #[test]
    fn export_copies_into_the_picked_folder_under_free_names() {
        let base = std::env::temp_dir().join(format!("honeydew-ops-export-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&base);
        let (root, dest) = (base.join(SONGS_FOLDER), base.join("dest"));
        let mut lib = NativeLibrary::at(root.clone());
        lib.store("A.flm", b"a").unwrap();
        let f = create_folder(&mut lib, &root, "F").unwrap();
        std::fs::write(f.join("B.flm"), b"b").unwrap();
        std::fs::create_dir_all(&dest).unwrap();
        std::fs::write(dest.join("A.flm"), b"keep").unwrap();
        let picked = [entry(&lib, None, "A.flm"), entry(&lib, None, "F")];
        export_into(&mut lib, &picked, &dest).unwrap();
        assert_eq!(std::fs::read(dest.join("A.flm")).unwrap(), b"keep");
        assert_eq!(std::fs::read(dest.join("A (1).flm")).unwrap(), b"a");
        assert_eq!(std::fs::read(dest.join("F").join("B.flm")).unwrap(), b"b");
        assert_eq!(names(&lib, None), ["A.flm", "F/"]);
        std::fs::remove_dir_all(&base).unwrap();
    }

    #[cfg(all(unix, not(target_os = "macos")))]
    #[test]
    fn show_in_file_browser_opens_the_folder_that_holds_the_item() {
        let cmd = reveal_command(Path::new("/s/My Songs/A.flm"));
        assert_eq!(cmd.get_program(), "xdg-open");
        assert_eq!(cmd.get_args().collect::<Vec<_>>(), ["/s/My Songs"]);
        assert_eq!(FILE_MANAGER, Some("file browser"));
    }
}

#[cfg(target_os = "android")]
pub fn export(_: &mut dyn SongLibrary, _: &[SongEntry]) -> Done {
    Err("External file export is not available in this Android preview.".into())
}
