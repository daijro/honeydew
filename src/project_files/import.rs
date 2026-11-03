//! Importing the user's own copy of the source app into the Original Content folder. An `.obb` (the content
//! container, [`super::obb`]) or an `.apk` (a ZIP holding it at [`OBB_IN_APK`], streamed through
//! [`crate::zip::open_entry`]) dropped into the content folder unpacks there at the relative paths its entries name,
//! so everything that reads the folder picks them up.
//!
//! - **When.** At start and whenever SETTINGS, which shows the folder, comes up ([`ProjectFiles::import_archives`]),
//!   on a worker thread. [`ProjectFiles::update`] hands the result over once nothing else is shown or asked, on any
//!   screen: the flash [`IMPORTED`] with the count of files (`Imported %d file%s`), or the message [`IMPORT_FAILED`]
//!   with why and that count (no question whether to delete the archive: it is the user's own copy). GMSynth's
//!   waveshapes and the shortcut files need a restart, as the README says.
//! - **Sounds, presets and demo songs only.** The app's icon sheet (`wrap.png`) is skipped: the icons are always
//!   our own.
//! - **Once.** A hidden marker beside the archive, `.imported-<file name>-<size>`, says it was imported. A failure
//!   leaves none, so the next start tries again and goes on where it stopped; until then that archive is left alone
//!   unless it is replaced (one message a session, however often SETTINGS comes up).
//! - **Never over the user's files.** A file already there stays: each file is written to a hidden temp file and
//!   hard-linked into place, which never replaces anything (a link included), so quitting mid-import never leaves
//!   half a file that a later import would keep either. An APK's container is read whole first, so a damaged copy (its
//!   CRC) writes nothing. Nothing is written through a link that leads out of the content folder
//!   ([`ProjectFiles::resolve`]'s guard), nor where a file or a dangling link stands in for a folder: such an entry is
//!   logged and skipped. A folder that cannot be made for another reason (no permission, a full disk) fails the import.
//! - **The notice first.** Before the first import ever,
//!   while an archive waits, a message shows [`import_notice`] with [`IMPORT`] and [`NOT_NOW`] ([`NOTICE_TAG`]).
//!   `Import` goes ahead and sets [`ProjectFiles::import_accepted`], which the app keeps in its settings, so later
//!   imports do not ask; `Not now` leaves the archive for this session, and the notice comes back at the next start.
//! - **The command.** `honeydew-import` (`src/bin/honeydew-import.rs`) unpacks archives from anywhere into a folder
//!   through [`import_into`]: the same guards, markers and never-overwrite rules, and the notice printed once.
//! - Nothing is downloaded, decrypted or sent anywhere: only plain entries of a file the user already has are copied,
//!   and only into the folder. The web has no content folder, so no import (as `content`).

use std::fs::File;
use std::io::{self, BufReader, Read};
use std::path::{Path, PathBuf};
use std::sync::mpsc::{self, TryRecvError};
use std::time::{Duration, SystemTime};

use eframe::egui::Context;

use super::ProjectFiles;
use super::content::{CONTENT_FOLDER, FOLD_CASE, in_content_folder, inside, real};
use crate::ui::views::Answer;

/// Where an APK holds the content container.
pub const OBB_IN_APK: &str = "assets/obb/assets.obb";
/// The container's icon sheet, never imported.
const ICON_SHEET: &str = "wrap.png";
/// The flash after an import, and the message title after a failed one.
pub const IMPORTED: &str = "Successfully imported";
pub const IMPORT_FAILED: &str = "Import failed";
/// How often [`ProjectFiles::update`] looks for the worker's result while it runs.
const POLL: Duration = Duration::from_millis(250);
/// The first-import notice's message tag and choices.
pub const NOTICE_TAG: &str = "project_files_import_notice";
pub const IMPORT: &str = "Import";
pub const NOT_NOW: &str = "Not now";

/// The notice shown before the first import and printed by `honeydew-import`, a paragraph a line. It is the one
/// place the app names the app it imports from and its maker (with `README.md`'s "Original content"): a factual
/// compatibility line with the trademark disclaimer.
pub fn import_notice() -> Vec<String> {
    let app = crate::APP_NAME;
    vec![
        format!("{app} does not include any sounds, presets or other content from FL Studio Mobile."),
        format!(
            "The importer reads a copy of FL Studio Mobile that you legally own, on your own device, and writes \
             only to your {app} user folder. Do not share imported content."
        ),
        "Your FL Studio Mobile licence (EULA) still governs how you may use these sounds, including that they are \
         for your own songs."
            .into(),
        format!(
            "FL Studio and FL Studio Mobile are trademarks of Image-Line. {app} is not affiliated with or endorsed \
             by Image-Line."
        ),
    ]
}

/// The first-import notice this session ([`import_notice`]).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(super) enum Notice {
    /// An archive waits: it imports at once if the notice was accepted by then (the app hands the settings over
    /// after the start's look), else the notice shows once nothing else is shown or asked.
    Due,
    /// Shown, until `Import` or `Not now`.
    Asked,
    /// `Not now`: no import this session.
    NotNow,
}

/// What an import wrote, and the files it left alone: already there, or where a link or a file stands in the way.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct Counts {
    pub written: usize,
    pub skipped: usize,
}

/// The worker's result: the files written, the first error, and the archives that failed ([`failed_key`]).
pub(super) type Imported = (usize, Option<String>, Vec<Failed>);
/// An archive whose import failed, as this session remembers it: its marker (name and size) and its modification
/// time, so a replaced copy of the same size is tried again.
pub(super) type Failed = (PathBuf, Option<SystemTime>);

fn failed_key(archive: &Path, marker: &Path) -> Failed {
    let modified = std::fs::metadata(archive).and_then(|m| m.modified());
    (marker.to_owned(), modified.ok())
}

/// The archives in `content` that wait: `.obb` and `.apk` files (any case; hidden ones and folders left out) without
/// their marker, each with its marker.
fn waiting(content: &Path) -> Vec<(PathBuf, PathBuf)> {
    let Ok(list) = std::fs::read_dir(content) else {
        return Vec::new();
    };
    let archive = |e: &std::fs::DirEntry| {
        let path = e.path();
        let name = e.file_name().into_string().ok()?;
        let ext = path.extension()?.to_str()?.to_ascii_lowercase();
        let size = std::fs::metadata(&path).ok().filter(|m| m.is_file())?.len();
        let marker = marker(content, &name, size);
        let new = !name.starts_with('.') && std::fs::symlink_metadata(&marker).is_err();
        (new && ["obb", "apk"].contains(&ext.as_str())).then_some((path, marker))
    };
    list.flatten().filter_map(|e| archive(&e)).collect()
}

/// The hidden marker in `content` that says the archive `name` of `size` bytes was imported.
fn marker(content: &Path, name: &str, size: u64) -> PathBuf {
    content.join(format!(".imported-{name}-{size}"))
}

/// Writes the marker `at`, new: anything already there (a file, or a link, never followed nor truncated) means the
/// archive was imported.
fn mark(at: &Path) -> io::Result<()> {
    match File::create_new(at) {
        Err(e) if e.kind() == io::ErrorKind::AlreadyExists => Ok(()),
        made => made.map(drop),
    }
}

/// Writes all of `data` at `path` unless something is there by then: a hidden temp file, synced, hard-linked into
/// place (which fails on anything already there) and removed. Returns whether it wrote.
fn write_new<R: Read>(path: &Path, data: &mut io::Take<R>) -> io::Result<bool> {
    let mut tmp = std::ffi::OsString::from(".");
    tmp.push(path.file_name().unwrap_or_default());
    tmp.push(".tmp");
    let tmp = path.with_file_name(tmp);
    let _ = std::fs::remove_file(&tmp);
    let made = File::create_new(&tmp)
        .and_then(|mut file| {
            io::copy(data, &mut file)?;
            if data.limit() > 0 {
                return Err(io::ErrorKind::UnexpectedEof.into());
            }
            // ponytail: a sync per file (seconds over a full container's 5571 files); drop it if it drags.
            file.sync_all()
        })
        // ponytail: a file system without hard links fails the import; a create_new copy if one turns up.
        .and_then(|()| std::fs::hard_link(&tmp, path));
    let _ = std::fs::remove_file(&tmp);
    match made {
        Ok(()) => Ok(true),
        Err(e) if e.kind() == io::ErrorKind::AlreadyExists => Ok(false),
        Err(e) => Err(e),
    }
}

/// Unpacks the container `obb` (`len` bytes) into `content`, counting what it writes and skips in `counts`.
fn unpack(obb: impl Read, len: u64, content: &Path, counts: &mut Counts) -> io::Result<()> {
    let root = content.canonicalize()?;
    super::obb::read(obb, len, |name, data| {
        if name == ICON_SHEET {
            return Ok(());
        }
        let path = content.join(name);
        let parent = path.parent().unwrap_or(content);
        if !real(parent).is_some_and(|p| inside(&p, &root, FOLD_CASE)) {
            eprintln!("{name}: a link leads out of the content folder, skipped");
            counts.skipped += 1;
            return Ok(());
        }
        // Already there: not even copied ([`write_new`] would keep it too).
        if std::fs::symlink_metadata(&path).is_ok() {
            counts.skipped += 1;
            return Ok(());
        }
        // A file or a dangling link where a folder would go is skipped; any other error (no permission, a full disk)
        // fails the import.
        match std::fs::create_dir_all(parent) {
            Err(e)
                if matches!(
                    e.kind(),
                    io::ErrorKind::AlreadyExists | io::ErrorKind::NotADirectory
                ) =>
            {
                eprintln!("{name}: {e}, skipped");
                counts.skipped += 1;
                return Ok(());
            }
            made => made?,
        }
        if write_new(&path, data)? {
            counts.written += 1;
        } else {
            counts.skipped += 1;
        }
        Ok(())
    })
}

/// Unpacks the archive `archive` into `content`, counting what it writes and skips in `counts`.
fn import(content: &Path, archive: &Path, counts: &mut Counts) -> io::Result<()> {
    if !archive
        .extension()
        .is_some_and(|e| e.eq_ignore_ascii_case("apk"))
    {
        let file = File::open(archive)?;
        let len = file.metadata()?.len();
        return unpack(BufReader::new(file), len, content, counts);
    }
    let invalid = |e: String| io::Error::new(io::ErrorKind::InvalidData, e);
    let obb = || {
        let opened = crate::zip::open_entry(File::open(archive)?, OBB_IN_APK)
            .map_err(|e| invalid(e.to_string()))?;
        opened.ok_or_else(|| invalid("the APK holds no content".into()))
    };
    // Read to its end first, so a damaged copy (its CRC) writes nothing; then again to unpack.
    io::copy(&mut obb()?.0, &mut io::sink())?;
    let (data, len) = obb()?;
    unpack(BufReader::new(data), len, content, counts)
}

/// Imports `archives` (from [`waiting`]) into `content`, marking each one done.
fn import_all(content: &Path, archives: Vec<(PathBuf, PathBuf)>) -> Imported {
    let (mut counts, mut error, mut failed) = (Counts::default(), None, Vec::new());
    for (archive, marker) in archives {
        let done = import(content, &archive, &mut counts).and_then(|()| mark(&marker));
        if let Err(e) = done {
            error = error.or(Some(e.to_string()));
            failed.push(failed_key(&archive, &marker));
        }
    }
    (counts.written, error, failed)
}

/// Imports the `.obb` or `.apk` `archive`, wherever it is, into the content folder `content` (made if missing), as
/// one dropped there imports, counting what it writes and skips in `counts`; once done, its marker in `content` says
/// so, and the app does not import a copy of it there again. Anything else is refused unread. The command
/// `honeydew-import` runs it.
pub fn import_into(content: &Path, archive: &Path, counts: &mut Counts) -> io::Result<()> {
    let ext = archive
        .extension()
        .and_then(|e| e.to_str())
        .map(str::to_ascii_lowercase);
    let name = (archive.file_name().and_then(|n| n.to_str()))
        .filter(|_| ext.is_some_and(|e| e == "obb" || e == "apk"))
        .ok_or_else(|| io::Error::new(io::ErrorKind::InvalidInput, "not an .obb or .apk"))?;
    let size = std::fs::metadata(archive)?.len();
    std::fs::create_dir_all(content)?;
    import(content, archive, counts)?;
    mark(&marker(content, name, size))
}

/// `to` may take imported content: it is the user folder `user`'s content folder or below it, or outside `user`, every
/// link followed. Anywhere else in the user folder, the project export's guard ([`ProjectFiles::in_content`]) would not
/// see it. `honeydew-import` refuses another `--to`.
pub fn import_target_allowed(user: &Path, to: &Path) -> bool {
    in_content_folder(&user.join(CONTENT_FOLDER), to) || !in_content_folder(user, to)
}

impl ProjectFiles {
    /// Starts importing the archives waiting in the content folder on a worker thread, unless an import runs or none
    /// waits (one listing of the folder; one that failed this session does not wait). The app calls it at start and
    /// when SETTINGS comes up; an Original Content tab would call it as it opens.
    pub fn import_archives(&mut self) {
        if self.import.is_some() || self.import_notice.is_some() {
            return;
        }
        let Some(content) = self.content_dir() else {
            return;
        };
        let mut archives = waiting(&content);
        archives
            .retain(|(archive, marker)| !self.import_failed.contains(&failed_key(archive, marker)));
        if archives.is_empty() {
            return;
        }
        // The first import ever waits for the notice ([`Notice`]).
        if !self.import_accepted {
            self.import_notice = Some(Notice::Due);
            return;
        }
        let (tx, rx) = mpsc::channel();
        std::thread::spawn(move || drop(tx.send(import_all(&content, archives))));
        self.import = Some(rx);
    }

    /// Once the import is done and nothing else is shown or asked (`shortcuts` is [`Self::update`]'s: no other
    /// screen's dialog or message is open): its flash or message.
    pub(super) fn import_done(&mut self, ctx: &Context, shortcuts: bool) {
        // Held while something else shows, so it never takes another question's place or buttons.
        let busy = !shortcuts
            || self.message.is_some()
            || self.flash.is_some()
            || self.waiting_for().is_some();
        match self.import_notice {
            Some(Notice::Due) if self.import_accepted => {
                self.import_notice = None;
                self.import_archives();
            }
            Some(Notice::Due) if !busy => {
                self.import_notice = Some(Notice::Asked);
                self.message = Some(import_notice());
            }
            Some(Notice::Due) => ctx.request_repaint_after(POLL),
            _ => {}
        }
        let Some(running) = &self.import else {
            return;
        };
        let done = match (!busy).then(|| running.try_recv()) {
            Some(Ok(done)) => Some(done),
            Some(Err(TryRecvError::Disconnected)) => {
                Some((0, Some("the import stopped".into()), Vec::new()))
            }
            _ => None,
        };
        let Some((written, error, failed)) = done else {
            ctx.request_repaint_after(POLL);
            return;
        };
        self.import = None;
        self.import_failed.extend(failed);
        let count = format!(
            "Imported {written} file{}",
            if written == 1 { "" } else { "s" }
        );
        match error {
            None => self.flash = Some(format!("{IMPORTED}\n{count}")),
            Some(why) => self.message = Some(vec![IMPORT_FAILED.into(), why, count]),
        }
        ctx.request_repaint();
    }

    /// `answer`, unless it answers the notice ([`NOTICE_TAG`]): `Import` goes ahead and sets
    /// [`Self::import_accepted`]; `Not now` leaves the archives until the next start.
    pub(super) fn notice_answered(&mut self, answer: Option<Answer>) -> Option<Answer> {
        if self.import_notice != Some(Notice::Asked) {
            return answer;
        }
        let yes = answer? == Answer::Yes;
        self.import_notice = (!yes).then_some(Notice::NotNow);
        if yes {
            self.import_accepted = true;
            self.import_archives();
        }
        None
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::project_files::CONTENT_FOLDER;
    use crate::project_files::content::tests::{Temp, put};
    use crate::project_files::obb::tests::container;

    /// The files on disk in `user`, the notice accepted (as the settings say after its `Import`).
    fn accepted(user: &Path) -> ProjectFiles {
        let mut files = ProjectFiles::on_disk(user);
        files.import_accepted = true;
        files
    }

    /// The content folder in `tmp`, made.
    fn content(tmp: &Temp) -> PathBuf {
        let dir = tmp.0.join(CONTENT_FOLDER);
        std::fs::create_dir_all(&dir).unwrap();
        dir
    }

    /// Imports what waits in `content`, as the worker does; `None` when nothing waits.
    fn run(content: &Path) -> Option<(usize, Option<String>)> {
        let archives = waiting(content);
        (!archives.is_empty())
            .then(|| import_all(content, archives))
            .map(|(written, error, _)| (written, error))
    }

    /// Deterministic bytes deflate cannot shrink.
    fn noise(len: usize) -> Vec<u8> {
        let mut x = 0x9E37_79B9u32;
        (0..len)
            .map(|_| {
                x ^= x << 13;
                x ^= x >> 17;
                x ^= x << 5;
                (x >> 11) as u8
            })
            .collect()
    }

    const TREE: [(&str, &[u8]); 3] = [
        ("Drum Samples/Kick/Gen Kick.wav", b"fLaC gen kick"),
        ("Chord Sets/Gen Minor.chords", b"{ \"chords\": [] }"),
        ("Gen Root.data", b""),
    ];

    /// The hidden files `content` holds (markers and temp files).
    fn hidden(content: &Path) -> Vec<String> {
        let names = std::fs::read_dir(content).unwrap().flatten();
        let names = names.map(|e| e.file_name().to_string_lossy().into_owned());
        names.filter(|n| n.starts_with('.')).collect()
    }

    #[test]
    fn an_obb_in_the_content_folder_unpacks_at_its_paths_once() {
        let tmp = Temp::new("import-obb");
        let content = content(&tmp);
        let obb = container(&TREE);
        put(&content.join("Gen.obb"), &obb);
        assert_eq!(run(&content), Some((3, None)));
        for (name, data) in TREE {
            assert_eq!(std::fs::read(content.join(name)).unwrap(), data, "{name}");
        }
        assert_eq!(
            hidden(&content),
            [format!(".imported-Gen.obb-{}", obb.len())]
        );
        // The next start finds nothing waiting, and a file the user removed stays removed.
        std::fs::remove_file(content.join(TREE[0].0)).unwrap();
        assert_eq!(run(&content), None);
        assert!(!content.join(TREE[0].0).exists());
        // A different file of the same name (a new size) imports again, keeping what is there.
        let more = container(&[TREE[1], ("Gen New.wav", b"fLaC")]);
        put(&content.join("Gen.obb"), &more);
        assert_eq!(run(&content), Some((1, None)));
        assert!(content.join("Gen New.wav").is_file());
    }

    #[test]
    fn files_already_there_are_never_written_over() {
        let tmp = Temp::new("import-keep");
        let content = content(&tmp);
        put(&content.join(TREE[1].0), b"mine");
        // A folder where a file would go is kept too.
        std::fs::create_dir_all(content.join(TREE[2].0)).unwrap();
        put(&content.join("Gen.obb"), &container(&TREE));
        assert_eq!(run(&content), Some((1, None)));
        assert_eq!(std::fs::read(content.join(TREE[1].0)).unwrap(), b"mine");
        assert!(content.join(TREE[2].0).is_dir());
        assert_eq!(std::fs::read(content.join(TREE[0].0)).unwrap(), TREE[0].1);
        // A file where a folder would go, at any depth: those entries only are skipped.
        let entries = [
            ("Gen File/Sub/x.wav", &b"x"[..]),
            ("Gen File/y.wav", b"y"),
            ("Gen z.wav", b"z"),
        ];
        put(&content.join("Gen File"), b"mine");
        put(&content.join("More.obb"), &container(&entries));
        assert_eq!(run(&content), Some((1, None)));
        assert_eq!(std::fs::read(content.join("Gen File")).unwrap(), b"mine");
        assert!(content.join("Gen z.wav").is_file());
    }

    #[test]
    fn an_apk_imports_the_obb_it_holds_deflated_or_stored() {
        let (silence, loud) = (vec![0; 50_000], noise(3000));
        let deflated = [TREE[0], TREE[1], ("Gen Silence.wav", &silence[..])];
        let stored = [("Drum Samples/Gen Noise.wav", &loud[..])];
        for (case, entries) in [&deflated[..], &stored[..]].into_iter().enumerate() {
            let tmp = Temp::new(&format!("import-apk-{case}"));
            let content = content(&tmp);
            let obb = container(entries);
            let apk = crate::zip::write(&[
                ("AndroidManifest.xml".into(), b"<manifest/>".to_vec()),
                (OBB_IN_APK.into(), obb.clone()),
                ("classes.dex".into(), noise(100)),
            ])
            .unwrap();
            // Deflated when that is smaller, as an APK builder does: silence yes, noise no.
            assert_eq!(apk.len() < obb.len(), case == 0, "case {case}");
            put(&content.join("Gen Build.APK"), &apk);
            assert_eq!(run(&content), Some((entries.len(), None)), "case {case}");
            for (name, data) in entries {
                assert_eq!(std::fs::read(content.join(name)).unwrap(), *data, "{name}");
            }
            assert_eq!(run(&content), None);
        }
    }

    #[test]
    fn a_damaged_archive_fails_leaves_no_marker_and_is_tried_again() {
        let tmp = Temp::new("import-bad");
        let content = content(&tmp);
        // A container cut short, and one naming a file outside: refused before anything is written.
        let obb = container(&TREE);
        put(&content.join("Cut.obb"), &obb[..obb.len() - 1]);
        let escape = container(&[("Gen ok.wav", b"x"), ("../Gen escaped.wav", b"y")]);
        put(&content.join("Escape.obb"), &escape);
        // An APK without the container, and one whose container fails its CRC once read.
        let empty = crate::zip::write(&[("classes.dex".into(), noise(10))]).unwrap();
        put(&content.join("Other.apk"), &empty);
        // Its CRC is the last thing read; behind the container's data, bytes nothing unpacks.
        let trailing = [container(&TREE), b"gen trailing".to_vec()].concat();
        for (name, obb) in [("Crc.apk", container(&TREE)), ("Trailing.apk", trailing)] {
            let apk = crate::zip::write(&[(OBB_IN_APK.into(), obb)]).unwrap();
            let crc_at = apk.windows(4).position(|w| w == b"PK\x01\x02").unwrap() + 16;
            let mut bad_crc = apk;
            bad_crc[crc_at] ^= 1;
            put(&content.join(name), &bad_crc);
        }
        let (written, error) = run(&content).unwrap();
        assert!(error.is_some_and(|e| !e.is_empty()));
        assert_eq!(written, 0);
        assert!(!tmp.0.join("Gen escaped.wav").exists());
        assert!(!content.join("Gen ok.wav").exists());
        assert!(hidden(&content).is_empty(), "{:?}", hidden(&content));
        assert_eq!(waiting(&content).len(), 5);
        // The same again: each is tried, nothing is written twice.
        assert_eq!(
            run(&content).map(|(n, e)| (n, e.is_some())),
            Some((0, true))
        );
    }

    #[test]
    fn a_damaged_apk_writes_nothing_and_its_good_copy_imports_whole() {
        let tmp = Temp::new("import-apk-crc");
        let content = content(&tmp);
        let loud = noise(200_000);
        let entries = [
            ("Gen One.wav", &loud[..100_000]),
            ("Gen Two.wav", &loud[100_000..]),
        ];
        let apk = crate::zip::write(&[(OBB_IN_APK.into(), container(&entries))]).unwrap();
        assert!(apk.len() > 200_000, "stored");
        // A byte inside the first file's data: only the CRC at the end of the second can tell.
        let mut bad = apk.clone();
        bad[50_000] ^= 1;
        put(&content.join("Gen.apk"), &bad);
        let (written, error) = run(&content).unwrap();
        assert_eq!((written, error.is_some()), (0, true));
        for (name, _) in entries {
            assert!(!content.join(name).exists(), "{name}");
        }
        // The good copy in its place (the same size) imports whole: no damaged file was kept.
        put(&content.join("Gen.apk"), &apk);
        assert_eq!(run(&content), Some((2, None)));
        for (name, data) in entries {
            assert_eq!(std::fs::read(content.join(name)).unwrap(), data, "{name}");
        }
    }

    #[cfg(unix)]
    #[test]
    fn nothing_is_written_through_a_link_out_of_the_content_folder() {
        let tmp = Temp::new("import-link");
        let content = content(&tmp);
        let outside = tmp.0.join("Elsewhere");
        std::fs::create_dir_all(&outside).unwrap();
        std::os::unix::fs::symlink(&outside, content.join("Drum Samples")).unwrap();
        // A dangling link where a folder would go skips its files only.
        std::os::unix::fs::symlink(tmp.0.join("Gone"), content.join("Chord Sets")).unwrap();
        put(&content.join("Gen.obb"), &container(&TREE));
        assert_eq!(run(&content), Some((1, None)));
        assert_eq!(std::fs::read_dir(&outside).unwrap().count(), 0);
        assert!(!tmp.0.join("Gone").exists());
        assert!(content.join(TREE[2].0).is_file());
    }

    #[test]
    fn a_file_that_turns_up_while_one_is_written_is_kept() {
        let tmp = Temp::new("import-race");
        let path = tmp.0.join("Gen.wav");
        put(&path, b"mine");
        assert!(!write_new(&path, &mut (&b"theirs"[..]).take(6)).unwrap());
        #[cfg(unix)]
        {
            let link = tmp.0.join("Gen Link.wav");
            std::os::unix::fs::symlink(tmp.0.join("Gone.wav"), &link).unwrap();
            assert!(!write_new(&link, &mut (&b"theirs"[..]).take(6)).unwrap());
            assert!(!tmp.0.join("Gone.wav").exists());
        }
        assert_eq!(std::fs::read(&path).unwrap(), b"mine");
        // A stream that ends early writes nothing.
        let cut = write_new(&tmp.0.join("Gen Cut.wav"), &mut (&b"abc"[..]).take(5));
        assert_eq!(cut.unwrap_err().kind(), io::ErrorKind::UnexpectedEof);
        // No temp file is left behind, and a new one is written whole.
        assert!(write_new(&tmp.0.join("Gen New.wav"), &mut (&b"new"[..]).take(3)).unwrap());
        let mut names: Vec<_> = std::fs::read_dir(&tmp.0)
            .unwrap()
            .flatten()
            .map(|e| e.file_name())
            .collect();
        names.sort();
        let expected: &[&str] = if cfg!(unix) {
            &["Gen Link.wav", "Gen New.wav", "Gen.wav"]
        } else {
            &["Gen New.wav", "Gen.wav"]
        };
        assert_eq!(names, expected);
    }

    /// Runs `files`' frames until the import hands its result over, at most half a minute.
    fn until_done(files: &mut ProjectFiles, ctx: &Context) {
        let mut song = crate::song::Song::default();
        for _ in 0..3000 {
            files.update(ctx, &mut song, true, None);
            if files.import.is_none() {
                return;
            }
            std::thread::sleep(Duration::from_millis(10));
        }
        panic!("the import never finished");
    }

    #[test]
    fn an_archives_icon_sheet_is_skipped_and_a_flash_counts_the_files() {
        let tmp = Temp::new("import-icons");
        let mut files = accepted(&tmp.0);
        let ctx = Context::default();
        let obb = container(&[(ICON_SHEET, b"\x89PNG gen"), TREE[0], TREE[1]]);
        put(&tmp.0.join(CONTENT_FOLDER).join("Gen.obb"), &obb);
        files.import_archives();
        assert!(files.import.is_some());
        // Held while something else shows: a flash, then a message.
        files.flash("Saved!");
        until_done_or_held(&mut files, &ctx);
        assert_eq!(files.take_flash().as_deref(), Some("Saved!"));
        files.message = Some(vec!["Gen".into()]);
        until_done_or_held(&mut files, &ctx);
        assert!(files.take_overlay().is_some());
        until_done(&mut files, &ctx);
        assert_eq!(
            files.take_flash().as_deref(),
            Some("Successfully imported\nImported 2 files")
        );
        assert!(!tmp.0.join(CONTENT_FOLDER).join(ICON_SHEET).exists());
        // Nothing waits any more.
        files.import_archives();
        assert!(files.import.is_none());
    }

    /// A few frames while the worker surely finished: the result stays held.
    fn until_done_or_held(files: &mut ProjectFiles, ctx: &Context) {
        let mut song = crate::song::Song::default();
        std::thread::sleep(Duration::from_millis(300));
        for _ in 0..3 {
            files.update(ctx, &mut song, true, None);
        }
        assert!(files.import.is_some(), "held");
    }

    #[test]
    fn a_broken_archive_fails_once_a_session() {
        let tmp = Temp::new("import-once");
        let ctx = Context::default();
        let obb = tmp.0.join(CONTENT_FOLDER).join("Gen.obb");
        let good = container(&TREE);
        put(&obb, &patch_count(&good));
        let mut files = accepted(&tmp.0);
        for start in 0..2 {
            files = accepted(&tmp.0);
            // Three visits to SETTINGS: one message, then nothing until the next start.
            let mut messages = 0;
            for _ in 0..3 {
                files.import_archives();
                until_done(&mut files, &ctx);
                messages += usize::from(files.take_overlay().is_some());
            }
            assert_eq!(messages, 1, "start {start}");
        }
        // A good copy of the same size in its place (a new modification time) is tried at once.
        put(&obb, &good);
        let later = std::time::SystemTime::now() + Duration::from_secs(10);
        File::options()
            .write(true)
            .open(&obb)
            .unwrap()
            .set_modified(later)
            .unwrap();
        files.import_archives();
        until_done(&mut files, &ctx);
        assert_eq!(
            files.take_flash().as_deref(),
            Some("Successfully imported\nImported 3 files")
        );
    }

    /// `obb` with an entry count no file can hold: the same size, refused.
    fn patch_count(obb: &[u8]) -> Vec<u8> {
        let mut bad = obb.to_vec();
        bad[24..28].copy_from_slice(&u32::MAX.to_be_bytes());
        bad
    }

    #[cfg(unix)]
    #[test]
    fn a_folder_that_cannot_be_written_fails_the_import_and_it_is_tried_again() {
        use std::os::unix::fs::PermissionsExt;
        let tmp = Temp::new("import-readonly");
        let content = content(&tmp);
        let drums = content.join("Drum Samples");
        std::fs::create_dir_all(&drums).unwrap();
        let mode =
            |m| std::fs::set_permissions(&drums, std::fs::Permissions::from_mode(m)).unwrap();
        mode(0o555);
        // Root writes anyway: nothing to check then.
        if std::fs::create_dir(drums.join("probe")).is_ok() {
            return mode(0o755);
        }
        put(&content.join("Gen.obb"), &container(&TREE));
        let mut files = accepted(&tmp.0);
        files.import_archives();
        until_done(&mut files, &Context::default());
        let message = files.take_overlay();
        mode(0o755);
        let Some(crate::ui::views::Overlay::Message { lines, .. }) = message else {
            panic!("no message: {:?}", files.take_flash());
        };
        assert_eq!(lines[0], IMPORT_FAILED);
        assert!(hidden(&content).is_empty(), "{:?}", hidden(&content));
        // The next start, with the folder writable: the rest is imported.
        let mut files = accepted(&tmp.0);
        files.import_archives();
        until_done(&mut files, &Context::default());
        assert_eq!(
            files.take_flash().as_deref(),
            Some("Successfully imported\nImported 2 files")
        );
        assert_eq!(std::fs::read(content.join(TREE[0].0)).unwrap(), TREE[0].1);
    }

    #[test]
    fn a_failed_import_shows_a_message_with_the_count() {
        let tmp = Temp::new("import-message");
        let mut files = accepted(&tmp.0);
        put(
            &tmp.0.join(CONTENT_FOLDER).join("Gen.obb"),
            b"not a container",
        );
        files.import_archives();
        until_done(&mut files, &Context::default());
        let Some(crate::ui::views::Overlay::Message { tag, lines, .. }) = files.take_overlay()
        else {
            panic!("no message");
        };
        assert_eq!(tag, crate::project_files::MESSAGE_TAG);
        assert_eq!(lines[0], IMPORT_FAILED);
        assert_eq!(lines[2], "Imported 0 files");
        assert!(files.take_flash().is_none());
    }

    /// The first import ever waits for the notice's `Import`. `Not now` leaves the
    /// archive until the next start; once accepted, later imports do not ask.
    #[test]
    fn the_notice_gates_the_first_import_and_its_import_is_remembered() {
        use crate::ui::views::{Answer, Overlay};
        let tmp = Temp::new("import-notice");
        let ctx = Context::default();
        let mut song = crate::song::Song::default();
        let content = content(&tmp);
        put(&content.join("Gen.obb"), &container(&TREE));
        let asked = |files: &mut ProjectFiles, song: &mut crate::song::Song| {
            files.import_archives();
            assert!(files.import.is_none(), "nothing before the notice");
            files.update(&ctx, song, true, None);
            let Some(Overlay::Message {
                tag,
                lines,
                confirm,
                extra: None,
            }) = files.take_overlay()
            else {
                panic!("no notice");
            };
            assert_eq!(tag, NOTICE_TAG);
            assert_eq!(lines, import_notice());
            assert_eq!(confirm, Some((IMPORT.into(), NOT_NOW.into())));
            assert_eq!(files.waiting(), Some(NOTICE_TAG));
        };
        for start in 0..2 {
            let mut files = ProjectFiles::on_disk(&tmp.0);
            asked(&mut files, &mut song);
            files.update(&ctx, &mut song, true, Some(Answer::No));
            // SETTINGS again this session: neither the notice nor an import.
            files.import_archives();
            files.update(&ctx, &mut song, true, None);
            assert!(files.import.is_none(), "start {start}");
            assert!(files.take_overlay().is_none(), "start {start}");
            assert!(!files.import_accepted);
            assert!(hidden(&content).is_empty(), "the archive waits");
        }
        // `Import`: it imports, and says so for the settings to keep.
        let mut files = ProjectFiles::on_disk(&tmp.0);
        asked(&mut files, &mut song);
        files.update(&ctx, &mut song, true, Some(Answer::Yes));
        assert!(files.import_accepted);
        until_done(&mut files, &ctx);
        assert_eq!(
            files.take_flash().as_deref(),
            Some("Successfully imported\nImported 3 files")
        );
        // A later archive this session imports without asking.
        put(
            &content.join("More.obb"),
            &container(&[("Gen More.wav", b"x")]),
        );
        files.import_archives();
        assert!(files.import.is_some());
        until_done(&mut files, &ctx);
        assert!(files.take_flash().is_some());
        // And at a later start, once the app hands over the kept acceptance (after the start's own look).
        put(
            &content.join("Last.obb"),
            &container(&[("Gen Last.wav", b"y")]),
        );
        let mut files = ProjectFiles::on_disk(&tmp.0);
        files.import_archives();
        files.import_accepted = true;
        until_done(&mut files, &ctx);
        assert_eq!(
            files.take_flash().as_deref(),
            Some("Successfully imported\nImported 1 file")
        );
        assert!(files.take_overlay().is_none());
    }

    /// The command's import (`honeydew-import`): an archive from anywhere into a folder it makes, with its marker
    /// there, counting the files written and those skipped.
    #[test]
    fn an_archive_from_anywhere_imports_into_the_folder_counting_what_it_skips() {
        let tmp = Temp::new("import-into");
        let to = tmp.0.join("Elsewhere").join(CONTENT_FOLDER);
        let obb = container(&TREE);
        let archive = tmp.0.join("Gen.obb");
        put(&archive, &obb);
        let mut counts = Counts::default();
        import_into(&to, &archive, &mut counts).unwrap();
        assert_eq!((counts.written, counts.skipped), (3, 0));
        for (name, data) in TREE {
            assert_eq!(std::fs::read(to.join(name)).unwrap(), data, "{name}");
        }
        // Its marker is the app's: the same archive dropped into the folder does not import again.
        put(&to.join("Gen.obb"), &obb);
        assert_eq!(run(&to), None);
        // Again: what is there stays, skipped.
        put(&to.join(TREE[0].0), b"mine");
        let mut counts = Counts::default();
        import_into(&to, &archive, &mut counts).unwrap();
        assert_eq!((counts.written, counts.skipped), (0, 3));
        assert_eq!(std::fs::read(to.join(TREE[0].0)).unwrap(), b"mine");
        // Anything but an `.obb` or `.apk` is refused, unread.
        let zip = tmp.0.join("Gen.zip");
        put(&zip, &obb);
        let refused = import_into(&to, &zip, &mut Counts::default()).unwrap_err();
        assert_eq!(refused.kind(), io::ErrorKind::InvalidInput);
        // A link in the marker's place is never followed: the file it names is neither emptied nor made.
        #[cfg(unix)]
        {
            let (outside, gone) = (tmp.0.join("Gen Outside.txt"), tmp.0.join("Gen Gone.txt"));
            put(&outside, b"mine");
            for (name, target) in [("Gen Linked.obb", &outside), ("Gen Dangling.obb", &gone)] {
                let at = marker(&to, name, obb.len() as u64);
                std::os::unix::fs::symlink(target, at).unwrap();
                put(&tmp.0.join(name), &obb);
                import_into(&to, &tmp.0.join(name), &mut Counts::default()).unwrap();
            }
            assert_eq!(std::fs::read(&outside).unwrap(), b"mine");
            assert!(!gone.exists());
        }
    }
}
