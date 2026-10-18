use std::path::{Path, PathBuf};

use super::mem::MemFiles;
use super::*;
use crate::song::ChannelKind;

/// A synthetic project for the codec seam: `10LF`, a HEAD chunk holding only the version string (@358), then
/// `TEST`, the number of drum channels after the master, and the load notices, one per line.
pub(crate) fn synthetic_file(drums: u8, version: &str, notices: &str) -> Vec<u8> {
    let mut head = vec![0; 358];
    head.extend((version.len() as u32).to_le_bytes());
    head.extend(version.as_bytes());
    let mut bytes = b"10LFHEAD".to_vec();
    bytes.extend((head.len() as u32).to_le_bytes());
    bytes.extend(head);
    bytes.extend(b"TEST");
    bytes.push(drums);
    bytes.extend(notices.as_bytes());
    bytes
}

/// Reads and writes [`synthetic_file`]s.
pub(crate) fn synthetic_codec() -> Codec {
    Codec {
        read: |bytes| {
            let at = (bytes.starts_with(b"10LF"))
                .then(|| bytes.windows(4).position(|w| w == b"TEST"))
                .flatten()
                .ok_or("bad magic")?;
            let (&drums, notices) = bytes[at + 4..].split_first().ok_or("truncated")?;
            let mut song = Song {
                title: "from the file".into(),
                ..Song::default()
            };
            for _ in 0..drums {
                song.add_channel(ChannelKind::Drums);
            }
            let notices = String::from_utf8_lossy(notices);
            Ok((song, notices.lines().map(Into::into).collect()))
        },
        write: |song| {
            Ok(synthetic_file(
                (song.channels.len() - 1) as u8,
                FORMAT_VERSION,
                "",
            ))
        },
    }
}

fn plain(drums: u8) -> Vec<u8> {
    synthetic_file(drums, FORMAT_VERSION, "")
}

fn files(mem: &MemFiles) -> ProjectFiles {
    ProjectFiles {
        codec: synthetic_codec(),
        ..ProjectFiles::with_provider(Box::new(mem.clone()))
    }
}

fn song(title: &str) -> Song {
    Song {
        title: title.into(),
        ..Song::default()
    }
}

fn lines(lines: &[&str]) -> Option<Vec<String>> {
    Some(lines.iter().map(|l| l.to_string()).collect())
}

/// Opens the file the provider picks; returns the song.
fn open(files: &mut ProjectFiles) -> Option<Song> {
    files.open();
    files.poll(&mut Song::default())
}

#[test]
fn opening_a_project_titles_it_after_the_file_and_keeps_its_path() {
    let mem = MemFiles::with_file("/songs/Beat.flm", &plain(2));
    let mut files = files(&mem);
    let mut song = open(&mut files).expect("no song");
    assert_eq!(song.title, "Beat");
    assert_eq!(song.channels.len(), 3);
    assert_eq!(files.path, Some(PathBuf::from("/songs/Beat.flm")));
    assert_eq!(files.message, None);
    // Ctrl+S now writes there without asking.
    files.save(&song);
    assert_eq!(files.dialog, None);
    assert_eq!(files.poll(&mut song), None);
    let mem = mem.0.borrow();
    assert_eq!(mem.saves, [(PathBuf::from("/songs/Beat.flm"), false)]);
    assert_eq!(mem.files[&PathBuf::from("/songs/Beat.flm")], plain(2));
}

#[test]
fn quick_save_names_the_file_after_the_song_title() {
    let mem = MemFiles::with_file("/songs/Beat.flm", &plain(0));
    let mut files = files(&mem);
    let mut song = open(&mut files).unwrap();
    song.title = "Beat v2".into();
    files.save(&song);
    files.poll(&mut song);
    assert_eq!(files.path, Some(PathBuf::from("/songs/Beat v2.flm")));
    assert!(
        mem.0
            .borrow()
            .files
            .contains_key(&PathBuf::from("/songs/Beat v2.flm"))
    );
    // A title that is no file name keeps the file.
    song.title = "a/b".into();
    files.save(&song);
    let last = mem.0.borrow().saves.last().unwrap().0.clone();
    assert_eq!(last, PathBuf::from("/songs/Beat v2.flm"));
}

#[test]
fn a_template_opens_as_an_untitled_new_song_unique_in_the_song_folder() {
    let mem = MemFiles::with_file("/songs/Beat.flm", &plain(0));
    for name in ["New Song", "New Song (1)"] {
        mem.0
            .borrow_mut()
            .files
            .insert(format!("/songs/{name}.flm").into(), vec![]);
    }
    let mut files = files(&mem);
    open(&mut files).unwrap();
    mem.0
        .borrow_mut()
        .files
        .insert("/t/Trap.SongTmpl".into(), plain(1));
    mem.0.borrow_mut().pick = Some("/t/Trap.SongTmpl".into());
    let song = open(&mut files).expect("no song");
    assert_eq!(song.title, "New Song (2)");
    assert_eq!(song.channels.len(), 2);
    assert_eq!(files.path, None);
    // The first save asks, with that name, in the song folder.
    files.save(&song);
    assert_eq!(files.dialog.as_deref(), Some("New Song (2)"));
    assert!(files.dialog_save(&song, "New Song (2)"));
    assert_eq!(
        mem.0.borrow().saves,
        [(PathBuf::from("/songs/New Song (2).flm"), true)]
    );
    // The template is never written.
    assert_eq!(
        mem.0.borrow().files[&PathBuf::from("/t/Trap.SongTmpl")],
        plain(1)
    );
}

#[test]
fn an_untitled_song_without_a_folder_is_new_song() {
    let mut files = files(&MemFiles::default());
    files.save(&song(""));
    assert_eq!(files.dialog.take().as_deref(), Some("New Song"));
    files.save(&song("Sketch"));
    assert_eq!(files.dialog.as_deref(), Some("Sketch"));
}

#[test]
fn a_corrupt_file_shows_the_error_and_returns_no_song() {
    let mem = MemFiles::with_file("/songs/Bad.flm", b"JUNK");
    let mut files = files(&mem);
    assert_eq!(open(&mut files), None);
    assert_eq!(files.message, lines(&[LOAD_FAILED, "bad magic"]));
    assert_eq!(files.path, None);
}

#[test]
fn a_song_without_the_master_channel_is_a_load_error() {
    // The app indexes `channels.len() - 1`; the codec is not trusted to keep the master.
    let mem = MemFiles::with_file("/songs/Empty.flm", &plain(0));
    let mut files = files(&mem);
    files.codec.read = |_| {
        Ok((
            Song {
                channels: Vec::new(),
                ..Song::default()
            },
            Vec::new(),
        ))
    };
    assert_eq!(open(&mut files), None);
    assert_eq!(
        files.message,
        lines(&[LOAD_FAILED, "The song has no channels."])
    );
    assert_eq!(files.path, None);
}

#[test]
fn an_unknown_file_type_is_an_error_even_if_it_decodes() {
    // A provider that skipped the check still loads nothing.
    let mut files = files(&MemFiles::default());
    assert_eq!(files.opened("/songs/Beat.wav".into(), plain(0)), None);
    assert_eq!(files.message, lines(&["Unknown project file type"]));
}

#[test]
fn an_flm2_project_is_reported_as_an_unsupported_old_format() {
    let mem = MemFiles::with_file("/songs/Old.flm", b"\x99ELM\r\n\x1A\nHEAD");
    let mut files = files(&mem);
    assert_eq!(open(&mut files), None);
    assert_eq!(
        files.message,
        lines(&[LOAD_FAILED, "Unsupported old project format"])
    );
}

#[test]
fn a_failed_open_shows_the_providers_title() {
    let mem = MemFiles::default();
    mem.0.borrow_mut().pick = Some("/songs/Gone.flm".into());
    let mut files = files(&mem);
    assert_eq!(open(&mut files), None);
    assert_eq!(
        files.message,
        lines(&[OPEN_FAILED, "/songs/Gone.flm: not found"])
    );
}

#[test]
fn a_cancelled_open_does_nothing() {
    let mut files = files(&MemFiles::default());
    assert_eq!(open(&mut files), None);
    assert_eq!(files.message, None);
}

#[test]
fn load_notices_show_once_the_song_is_open() {
    let notices = "Missing rack for Channel #3 skipped\nMissing rack for Channel #4 skipped";
    let mem = MemFiles::with_file("/songs/Beat.flm", &synthetic_file(1, "4.7.14", notices));
    let mut files = files(&mem);
    assert!(open(&mut files).is_some());
    // The load warning, then the details.
    assert_eq!(
        files.message,
        lines(&[
            "Project file has corrupted data.",
            "The project was loaded successfully, but some elements may be missing.",
            "Details: Missing rack for Channel #3 skipped",
            "Missing rack for Channel #4 skipped"
        ])
    );
    assert!(matches!(
        files.take_overlay(),
        Some(Overlay::Message {
            tag: MESSAGE_TAG,
            confirm: None,
            ..
        })
    ));
}

#[test]
fn a_project_from_a_newer_version_waits_for_an_answer() {
    let mem = MemFiles::with_file("/songs/New.flm", &synthetic_file(0, "4.11", ""));
    let mut files = files(&mem);
    assert_eq!(open(&mut files), None);
    assert_eq!(
        files.message,
        lines(&[
            "This project was created with a newer version (4.11) than this app (4.10.19),",
            "and will probably not load correctly.",
            "Do you want to continue loading this project anyway?",
        ])
    );
    assert!(files.pending.is_some());
    assert_eq!(files.waiting(), None, "the question is not shown yet");
    let Some(Overlay::Message { tag, confirm, .. }) = files.take_overlay() else {
        panic!("no message");
    };
    assert_eq!(tag, NEWER_TAG);
    assert_eq!(confirm, Some(("Yes".into(), "Cancel".into())));
    assert_eq!(files.waiting(), Some(NEWER_TAG));
}

#[test]
fn the_head_version_is_read_only_from_a_well_formed_first_chunk() {
    let file = synthetic_file(0, "4.10.19 G", "");
    assert_eq!(saved_version(&file).as_deref(), Some("4.10.19 G"));
    // An empty version still needs a 363-byte HEAD.
    assert_eq!(saved_version(&synthetic_file(0, "", "")), None);
    assert_eq!(
        saved_version(&synthetic_file(0, "1", "")).as_deref(),
        Some("1")
    );
    // Not first, wrong magic, or a version running past the chunk: none.
    let mut moved = b"10LFKEYB\0\0\0\0".to_vec();
    moved.extend(&file[4..]);
    assert_eq!(saved_version(&moved), None);
    assert_eq!(saved_version(&file[1..]), None);
    let mut long = file.clone();
    long[8 + 4 + 358] = 200;
    assert_eq!(saved_version(&long), None);
    // Every truncation and every single-byte corruption of the header: no panic.
    for n in 0..file.len() {
        let _ = saved_version(&file[..n]);
    }
    for i in 0..8 + 4 + 362 {
        for v in [0, 1, 0x7F, 0xFF] {
            let mut bad = file.clone();
            bad[i] = v;
            let _ = saved_version(&bad);
        }
    }
}

#[test]
fn versions_compare_as_dotted_numbers_with_missing_parts_zero() {
    assert!(newer("4.11", "4.10.19"));
    assert!(newer("4.10.20", "4.10.19"));
    assert!(newer("5", "4.10.19"));
    assert!(!newer("4.10.19", "4.10.19"));
    assert!(!newer("4.10.19 G", "4.10.19"));
    assert!(!newer("4.10", "4.10.0"));
    assert!(!newer("4.7.14", "4.10.19"));
    assert!(!newer("", "4.10.19"));
    assert!(newer("4.10.0.1", "4.10"));
}

#[test]
fn the_first_save_asks_then_the_song_is_named_after_its_file() {
    let mem = MemFiles::default();
    mem.0.borrow_mut().save_dir = Some(PathBuf::from("/home/me"));
    let mut files = files(&mem);
    let mut song = song("");
    files.save(&song);
    assert!(files.dialog.take().is_some(), "no dialog");
    assert!(files.dialog_save(&song, " Groove.flm "));
    assert_eq!(files.poll(&mut song), None);
    assert_eq!(song.title, "Groove");
    assert_eq!(files.path, Some(PathBuf::from("/home/me/Groove.flm")));
    files.save(&song);
    let mem = mem.0.borrow();
    assert_eq!(
        mem.saves,
        [
            (PathBuf::from("Groove.flm"), true),
            (PathBuf::from("/home/me/Groove.flm"), false)
        ]
    );
    assert_eq!(mem.files[&PathBuf::from("/home/me/Groove.flm")], plain(0));
}

#[test]
fn a_cancelled_save_dialog_leaves_the_song_untitled() {
    let mut files = files(&MemFiles::default());
    let mut song = song("");
    files.save(&song);
    let name = files.dialog.take().expect("no dialog");
    assert!(files.dialog_save(&song, &name));
    assert_eq!(files.poll(&mut song), None);
    assert_eq!(files.path, None);
    assert_eq!(files.message, None);
    assert_eq!(song.title, "");
    files.save(&song);
    assert!(files.dialog.is_some(), "the next save does not ask again");
}

#[test]
fn a_forbidden_symbol_keeps_the_dialog_and_shows_the_error() {
    let mem = MemFiles::default();
    let mut files = files(&mem);
    let song = song("");
    files.save(&song);
    assert!(!files.dialog_save(&song, "a:b"), "the dialog closed");
    assert_eq!(
        files.message,
        lines(&["Symbol ':' is not allowed in a file name."])
    );
    assert!(mem.0.borrow().saves.is_empty());
}

#[test]
fn an_empty_name_does_not_save() {
    let mem = MemFiles::default();
    let mut files = files(&mem);
    let song = song("");
    files.save(&song);
    assert!(!files.dialog_save(&song, "  "), "the dialog closed");
    assert_eq!(files.message, None);
    assert!(mem.0.borrow().saves.is_empty());
}

#[test]
fn a_failed_write_shows_the_error() {
    let mem = MemFiles::with_file("/songs/Beat.flm", &plain(0));
    mem.0.borrow_mut().write_error = Some("disk full".into());
    let mut files = files(&mem);
    let mut song = open(&mut files).unwrap();
    files.save(&song);
    assert_eq!(files.poll(&mut song), None);
    assert_eq!(
        files.message,
        lines(&["Failed to save project", "disk full"])
    );
}

#[test]
fn a_message_is_its_lines_with_ok_and_comes_after_the_dialog_to_open_on_top() {
    let mut files = ProjectFiles {
        message: lines(&["Failed", "why"]),
        dialog: Some("Song".into()),
        ..ProjectFiles::default()
    };
    assert_eq!(
        files.take_overlay(),
        Some(Overlay::SaveDialog {
            name: "Song".into()
        })
    );
    assert_eq!(
        files.take_overlay(),
        Some(Overlay::Message {
            tag: MESSAGE_TAG,
            lines: vec!["Failed".into(), "why".into()],
            confirm: None,
            extra: None,
        })
    );
    assert_eq!(files.take_overlay(), None);
}

#[test]
fn an_answer_to_the_newer_version_question_loads_on_yes_only() {
    let ctx = eframe::egui::Context::default();
    for (answer, loaded) in [(false, false), (true, true)] {
        let mem = MemFiles::with_file("/songs/New.flm", &synthetic_file(1, "9.0", ""));
        let mut files = files(&mem);
        let mut song = song("Old");
        files.open();
        assert_eq!(files.update(&ctx, &mut song, false, None), None);
        files.take_overlay();
        // Still waiting: nothing happens without an answer.
        assert_eq!(files.update(&ctx, &mut song, false, None), None);
        assert_eq!(files.waiting(), Some(NEWER_TAG));
        let opened = files.update(&ctx, &mut song, false, Some(answer.into()));
        assert_eq!(opened.is_some(), loaded, "{answer}");
        assert_eq!(files.waiting(), None);
        if let Some(opened) = opened {
            assert_eq!((opened.title.as_str(), opened.channels.len()), ("New", 2));
        }
    }
}

// ---- Reads, file kinds, names and saves ----

#[test]
fn reads_are_bounded_at_512_mib_whatever_the_source_claims() {
    assert_eq!(MAX_FILE_BYTES, 512 << 20);
    assert_eq!(too_large(), "larger than 512 MiB");
    assert_eq!(read_limited(&b"abcd"[..], 4), Ok(b"abcd".to_vec()));
    assert_eq!(read_limited(&b"abcde"[..], 4), Err(too_large()));
    // An endless source, like /dev/zero or a FIFO whose metadata says 0 bytes, stops one byte past the limit.
    assert_eq!(read_limited(std::io::repeat(0), 1000), Err(too_large()));
}

#[test]
fn a_long_version_string_is_ignored_and_version_parts_are_capped() {
    let at_cap = "1.".repeat(32);
    assert_eq!(at_cap.len(), 64);
    assert_eq!(
        saved_version(&synthetic_file(0, &at_cap, "")).as_deref(),
        Some(at_cap.as_str())
    );
    let long = format!("{at_cap}1");
    assert_eq!(saved_version(&synthetic_file(0, &long, "")), None);
    // Parts past the cap do not count, however many there are.
    let dotted = format!("4.10.19{}.1", ".0".repeat(100_000));
    assert!(!newer(&dotted, "4.10.19"));
    assert!(newer("4.10.19.0.0.1", "4.10.19"));
}

#[test]
fn the_provider_checks_the_type_before_reading() {
    let mem = MemFiles::with_file("/songs/Beat.wav", &plain(0));
    let mut files = files(&mem);
    assert_eq!(open(&mut files), None);
    assert_eq!(files.message, lines(&[UNKNOWN_TYPE, "/songs/Beat.wav"]));
    assert!(is_project(Path::new("/a/B.FlM")));
    assert!(is_project(Path::new("t.songTMPL")));
    assert!(!is_project(Path::new("/a/flm")));
    assert!(!is_project(Path::new("/a/b.flm2")));
}

#[test]
fn file_kinds_follow_the_extension_in_any_case() {
    let kind = |p: &str| file_kind(Path::new(p));
    for (name, expect) in [
        ("/a/B.flm", FileKind::Song),
        ("B.FLM", FileKind::Song),
        ("t.songtmpl", FileKind::Template),
        ("t.SongTmpl", FileKind::Template),
        ("/a/Pack.zip", FileKind::Archive),
        ("Pack.ZIP", FileKind::Archive),
        ("/a/Tune.mid", FileKind::Midi),
        ("Tune.MID", FileKind::Midi),
        ("Tune.midi", FileKind::Midi),
        ("/a.b/Tune.MiDi", FileKind::Midi),
    ] {
        assert_eq!(kind(name), Some(expect), "{name}");
        assert!(is_project(Path::new(name)), "{name}");
    }
    for name in [
        "/a/flm",
        "/a/b.flm2",
        "x.wav",
        "x.mid.txt",
        "x.",
        "x.zipx",
        "",
    ] {
        assert_eq!(kind(name), None, "{name}");
        assert!(!is_project(Path::new(name)), "{name}");
    }
}

#[test]
fn the_open_filter_lists_all_four_kinds_and_the_provider_enforces_it() {
    let mut accept: Vec<_> = Pick::Song.accept().to_vec();
    accept.sort_unstable();
    assert_eq!(accept, ["flm", "flp", "mid", "midi", "songtmpl", "zip"]);
    assert_eq!(
        accept_attr(Pick::Song.accept()),
        ".flm,.songtmpl,.zip,.mid,.midi,.flp"
    );
    // Every extension of every kind is in it, and nothing else is.
    for kind in [
        FileKind::Song,
        FileKind::Template,
        FileKind::Archive,
        FileKind::Midi,
        FileKind::Flp,
    ] {
        for ext in kind.extensions() {
            assert!(Pick::Song.accept().contains(ext), "{ext}");
            assert_eq!(file_kind(Path::new(&format!("x.{ext}"))), Some(kind));
        }
    }

    let mem = MemFiles::with_file("/songs/Beat.flm", &plain(0));
    let mut project = files(&mem);
    assert_eq!(project.pick, None);
    project.open();
    // The provider was asked with the list, and ProjectFiles remembers what asked until the answer.
    assert_eq!(mem.0.borrow().accepts, [Pick::Song.accept()]);
    assert_eq!(project.pick, Some(Pick::Song));
    assert!(project.poll(&mut Song::default()).is_some());
    assert_eq!(project.pick, None);

    // A provider only reads what the list names, in any case: `.WAV` is no project, `.MID` is a pick.
    for (path, picked) in [("/m/Beat.WAV", false), ("/m/Tune.MID", true)] {
        let mem = MemFiles::with_file(path, &plain(0));
        let mut project = files(&mem);
        project.open();
        let failed = matches!(mem.clone().poll(), Some(FileEvent::Failed(UNKNOWN_TYPE, _)));
        assert_eq!(failed, !picked, "{path}");
    }
}

#[test]
fn zip_and_midi_files_are_read_as_their_kind_in_any_case() {
    // Bytes that are neither: each kind's own failure, never the `.flm` reader's (`archive` has the rest).
    for (name, title) in [
        ("Pack.zip", ARCHIVE_FAILED),
        ("PACK.ZIP", ARCHIVE_FAILED),
        ("Tune.mid", crate::smf::import::LOAD_FAILED),
        ("Tune.MIDI", crate::smf::import::LOAD_FAILED),
    ] {
        let path = format!("/songs/{name}");
        let mem = MemFiles::with_file(&path, &plain(0));
        let mut files = files(&mem);
        assert_eq!(open(&mut files), None, "{name}");
        assert_eq!(files.message.as_ref().unwrap()[0], title, "{name}");
        // Nothing was kept: not a path, not a copy in the library.
        assert_eq!(files.path, None, "{name}");
        assert!(!files.library.exists(Path::new(&path)), "{name}");
        assert_eq!(files.pending, None, "{name}");
    }
}

#[test]
fn a_flash_holds_text_of_several_lines() {
    let mem = MemFiles::default();
    let mut files = files(&mem);
    files.flash("Cannot paste module");
    assert_eq!(files.take_flash().as_deref(), Some("Cannot paste module"));
    files.flash(format!("Render complete\nSaved to: {}", "/x/y.wav"));
    assert_eq!(
        files.take_flash().as_deref(),
        Some("Render complete\nSaved to: /x/y.wav")
    );
    assert_eq!(files.take_flash(), None);
}

#[test]
fn the_flm_extension_is_stripped_in_any_case() {
    let mem = MemFiles::default();
    let mut files = files(&mem);
    let song = song("");
    files.save(&song);
    files.dialog_save(&song, "Groove.FLM");
    assert_eq!(mem.0.borrow().saves, [(PathBuf::from("Groove.flm"), true)]);
}

#[test]
fn many_notices_end_in_a_count_of_the_rest() {
    let notices: Vec<String> = (1..=20).map(|i| format!("notice {i}")).collect();
    let mem = MemFiles::with_file(
        "/songs/Beat.flm",
        &synthetic_file(0, "", &notices.join("\n")),
    );
    let mut files = files(&mem);
    assert!(open(&mut files).is_some());
    let mut expected = CORRUPTED.map(String::from).to_vec();
    expected.push("Details: notice 1".into());
    expected.extend_from_slice(&notices[1..MAX_NOTICES]);
    expected.push("and 12 more".into());
    assert_eq!(files.message, Some(expected));
}

#[test]
fn the_default_provider_is_ready_before_the_first_update() {
    // It was made on the first frame, so `open` and `save` did nothing before it. Now it is always there.
    let files = ProjectFiles::default();
    assert!(
        !files
            .provider
            .exists(Path::new("/no/such/dir/New Song.flm"))
    );
}

#[cfg(not(target_arch = "wasm32"))]
#[test]
fn on_the_desktop_the_library_is_the_songs_folder_where_saving_starts() {
    let files = ProjectFiles::default();
    assert_eq!(files.dir, crate::ui::main_menu::songs_dir());
    assert!(!files.copies, "copies into the folder it saves to");
}

#[test]
fn quick_save_never_takes_a_name_that_is_no_song_file_or_another_song() {
    let mem = MemFiles::with_file("/songs/Beat.flm", &plain(0));
    mem.0
        .borrow_mut()
        .files
        .insert("/songs/Groove.flm".into(), b"another song".to_vec());
    let mut files = files(&mem);
    let mut song = open(&mut files).unwrap();
    // Reserved, hidden or with control characters: the file is kept.
    for title in ["CON", "nul", ".hidden", "a\tb"] {
        song.title = title.into();
        files.save(&song);
        let last = mem.0.borrow().saves.last().unwrap().0.clone();
        assert_eq!(last, PathBuf::from("/songs/Beat.flm"), "{title:?}");
    }
    // Another song's title: a free name next to it, never over it.
    song.title = "Groove".into();
    files.save(&song);
    files.poll(&mut song);
    assert_eq!(
        mem.0.borrow().files[&PathBuf::from("/songs/Groove.flm")],
        b"another song"
    );
    assert_eq!(files.path, Some(PathBuf::from("/songs/Groove (1).flm")));
    assert_eq!(song.title, "Groove (1)");
}

#[test]
fn the_save_dialog_refuses_names_that_are_no_song_file() {
    let mem = MemFiles::default();
    let mut files = files(&mem);
    let song = song("");
    for name in ["CON", "lpt1", ".hidden", "a\nb"] {
        files.save(&song);
        assert!(
            !files.dialog_save(&song, name),
            "{name:?}: the dialog closed"
        );
        assert_eq!(
            files.message.take(),
            lines(&[&format!("'{name}' is not allowed as a file name.")])
        );
    }
    assert!(mem.0.borrow().saves.is_empty());
}

#[cfg(not(target_arch = "wasm32"))]
fn temp_dir(test: &str) -> PathBuf {
    let dir = std::env::temp_dir().join(format!("honeydew-pf-{test}-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(&dir).unwrap();
    dir
}

#[cfg(not(target_arch = "wasm32"))]
fn names_in(dir: &Path) -> Vec<String> {
    let mut names: Vec<_> = std::fs::read_dir(dir)
        .unwrap()
        .map(|e| e.unwrap().file_name().into_string().unwrap())
        .collect();
    names.sort();
    names
}

#[cfg(not(target_arch = "wasm32"))]
#[test]
fn a_write_that_fails_halfway_leaves_the_old_file_whole() {
    use std::io::Write;
    let dir = temp_dir("halfway");
    let path = dir.join("Beat.flm");
    std::fs::write(&path, b"old song").unwrap();
    let err = replace_file(&path, |file| {
        file.write_all(b"half")?;
        Err(std::io::Error::other("disk full"))
    })
    .unwrap_err();
    assert_eq!(err.to_string(), "disk full");
    assert_eq!(std::fs::read(&path).unwrap(), b"old song");
    assert_eq!(names_in(&dir), ["Beat.flm"], "the temp file is left");
    write_atomic(&path, b"new song").unwrap();
    assert_eq!(std::fs::read(&path).unwrap(), b"new song");
    assert_eq!(names_in(&dir), ["Beat.flm"]);
    std::fs::remove_dir_all(&dir).unwrap();
}

// ---- the unsaved-changes question's `Save` ----

/// Asks to replace `song` (with unsaved changes: the song-changed flag is set here) with a new song and answers the
/// unsaved-changes question with `answer`; returns the new song, if any.
fn answer_unsaved(files: &mut ProjectFiles, song: &mut Song, answer: Answer) -> Option<Song> {
    files.song_changed();
    assert_eq!(files.replace(Replace::New), None, "it asks");
    assert!(matches!(
        files.take_overlay(),
        Some(Overlay::Message {
            tag: DISCARD_TAG,
            extra: Some(_),
            ..
        })
    ));
    let ctx = eframe::egui::Context::default();
    files.update(&ctx, song, false, Some(answer))
}

#[test]
fn the_unsaved_questions_save_is_a_quick_save_into_the_songs_folder_that_never_asks() {
    // (the song's own file, the folder open in SONGS, its title) → where it is written.
    let cases = [
        // Untitled, never saved: My Songs, as `New Song`.
        (None, None, "", "/My Songs/New Song.flm"),
        // The folder open in SONGS.
        (
            None,
            Some("/My Songs/Demo"),
            "Groove",
            "/My Songs/Demo/Groove.flm",
        ),
        // Outside My Songs: My Songs.
        (None, Some("/elsewhere"), "Groove", "/My Songs/Groove.flm"),
        // The song's own folder first.
        (
            Some("/My Songs/Demo/Beat.flm"),
            Some("/My Songs/Old"),
            "Beat",
            "/My Songs/Demo/Beat.flm",
        ),
        // Its own folder outside My Songs: the folder open in SONGS, else My Songs.
        (
            Some("/tmp/Beat.flm"),
            Some("/My Songs/Demo"),
            "Beat",
            "/My Songs/Demo/Beat.flm",
        ),
        (Some("/tmp/Beat.flm"), None, "Beat", "/My Songs/Beat.flm"),
        // A title that is no song file name keeps the file's name.
        (
            Some("/My Songs/Beat.flm"),
            None,
            "CON",
            "/My Songs/Beat.flm",
        ),
        // Another song's name: a free one next to it, never over it.
        (None, None, "Taken", "/My Songs/Taken (1).flm"),
    ];
    for (own, folder, title, written) in cases {
        let case = format!("{own:?} {folder:?} {title:?}");
        let mem = MemFiles::default();
        mem.0
            .borrow_mut()
            .files
            .insert("/My Songs/Taken.flm".into(), b"another song".to_vec());
        let mut files = files(&mem);
        files.root = Some("/My Songs".into());
        files.songs_folder = folder.map(PathBuf::from);
        let mut song = Song::default();
        if let Some(own) = own {
            mem.0.borrow_mut().files.insert(own.into(), plain(0));
            mem.0.borrow_mut().pick = Some(own.into());
            song = open(&mut files).unwrap();
        }
        song.title = title.into();
        song.add_channel(ChannelKind::Drums);
        let changes = files.library_changes();
        let new = answer_unsaved(&mut files, &mut song, Answer::Extra);
        let saves = mem.0.borrow().saves.clone();
        assert_eq!(saves, [(PathBuf::from(written), false)], "{case}");
        assert_eq!(mem.0.borrow().files[Path::new(written)], plain(1), "{case}");
        // Then the action: the new song, clean; SONGS reads its folder again; `Saved!` flashes once.
        let new = new.unwrap();
        assert!(new_now(&new), "{case}");
        assert!(!files.modified(), "{case}");
        assert!(files.library_changes() > changes, "{case}");
        assert_eq!(files.take_flash().as_deref(), Some(SAVED), "{case}");
        assert_eq!(files.take_flash(), None, "{case}");
        assert_eq!(files.take_overlay(), None, "{case}");
    }
}

#[test]
fn a_failed_save_from_the_unsaved_question_shows_its_message_and_drops_the_action() {
    let ctx = eframe::egui::Context::default();
    // `/songs/Beat.flm` opened, then edited.
    let opened = |files: &mut ProjectFiles| {
        let mut song = open(files).unwrap();
        song.add_channel(ChannelKind::Drums);
        song
    };
    // The write fails; the codec cannot write; there is no songs folder (the desktop without a home folder) and
    // the song has no file of its own.
    let write_fails = MemFiles::with_file("/songs/Beat.flm", &plain(0));
    write_fails.0.borrow_mut().write_error = Some("disk full".into());
    let mut no_codec = files(&MemFiles::with_file("/songs/Beat.flm", &plain(0)));
    no_codec.codec.write = |_| Err("no codec".into());
    let mut no_folder = files(&MemFiles::default());
    no_folder.root = None;
    // Too large to open again (`vec![0; n]` takes zeroed pages, untouched until written).
    let mut too_large_codec = files(&MemFiles::with_file("/songs/Beat.flm", &plain(0)));
    too_large_codec.codec.write = |_| Ok(vec![0; MAX_FILE_BYTES as usize + 1]);
    let too_large = too_large();
    for (mut files, detail) in [
        (files(&write_fails), "disk full"),
        (no_codec, "no codec"),
        (no_folder, "There is no songs folder."),
        (too_large_codec, too_large.as_str()),
    ] {
        let mut song = if files.root.is_some() {
            opened(&mut files)
        } else {
            let mut song = song("Beat");
            song.add_channel(ChannelKind::Drums);
            song
        };
        let edited = song.clone();
        assert_eq!(answer_unsaved(&mut files, &mut song, Answer::Extra), None);
        assert_eq!(
            files.take_overlay(),
            Some(Overlay::Message {
                tag: MESSAGE_TAG,
                lines: vec![SAVE_FILE_FAILED.into(), detail.into()],
                confirm: None,
                extra: None,
            }),
            "{detail}"
        );
        // Dropped: no question waits, nothing replaces the song later, nothing flashes.
        assert_eq!(files.waiting(), None, "{detail}");
        assert_eq!(
            files.update(&ctx, &mut song, false, Some(Answer::Yes)),
            None
        );
        assert_eq!(song, edited, "{detail}");
        assert_eq!(files.take_flash(), None, "{detail}");
        assert!(files.after_save.is_none(), "{detail}");
    }
    // Not even by the next save: once the write works again, Ctrl+S only saves.
    let mut files = files(&write_fails);
    let mut song = opened(&mut files);
    answer_unsaved(&mut files, &mut song, Answer::Extra);
    files.take_overlay();
    write_fails.0.borrow_mut().write_error = None;
    let edited = song.clone();
    files.save(&song);
    assert_eq!(files.update(&ctx, &mut song, false, None), None);
    assert_eq!(song, with_stamp(&edited, &song));
    let written = write_fails.0.borrow().files[Path::new("/songs/Beat.flm")].clone();
    assert_eq!(written, plain(1));
    assert_eq!(files.take_flash(), None);
    assert_eq!(files.take_overlay(), None);
}

#[test]
fn a_song_file_moved_in_songs_is_where_the_song_saves_next_named_after_it() {
    let mem = MemFiles::with_file("/songs/Beat.flm", &plain(0));
    let mut files = files(&mem);
    let mut song = open(&mut files).unwrap();
    // Other paths, and paths that only begin with the same letters, change nothing.
    for (from, to) in [
        ("/songs/Beat.fl", Some(Path::new("/x"))),
        ("/song", Some(Path::new("/x"))),
        ("/songs/Other.flm", None),
    ] {
        files.path_moved(&mut song, Path::new(from), to);
    }
    assert_eq!(files.path, Some(PathBuf::from("/songs/Beat.flm")));
    // Renamed: the title follows the file, and that is no unsaved change.
    let renamed = Path::new("/songs/Old.flm");
    files.path_moved(&mut song, Path::new("/songs/Beat.flm"), Some(renamed));
    assert_eq!(files.path.as_deref(), Some(renamed));
    assert_eq!(song.title, "Old");
    assert!(!files.modified());
    // Its folder moved: the file and the folder follow; the name, and so the title, stays.
    files.path_moved(
        &mut song,
        Path::new("/songs"),
        Some(Path::new("/new/songs")),
    );
    assert_eq!(files.path, Some(PathBuf::from("/new/songs/Old.flm")));
    assert_eq!(files.dir, Some(PathBuf::from("/new/songs")));
    assert_eq!(song.title, "Old");
    files.save(&song);
    files.poll(&mut song);
    let last = mem.0.borrow().saves.last().unwrap().clone();
    assert_eq!(last, (PathBuf::from("/new/songs/Old.flm"), false));
    // A title typed but not saved yet is kept.
    song.title = "Typed".into();
    files.path_moved(
        &mut song,
        Path::new("/new/songs/Old.flm"),
        Some(Path::new("/new/songs/X.flm")),
    );
    assert_eq!(song.title, "Typed");
    assert_eq!(files.path, Some(PathBuf::from("/new/songs/X.flm")));
}

#[test]
fn a_deleted_song_file_makes_the_next_save_ask() {
    let mem = MemFiles::with_file("/songs/A/Beat.flm", &plain(0));
    let mut files = files(&mem);
    let mut song = open(&mut files).unwrap();
    files.root = Some(PathBuf::from("/songs"));
    // Its folder deleted: the song has no file, and new names go into the songs folder.
    files.path_moved(&mut song, Path::new("/songs/A"), None);
    assert_eq!(files.path, None);
    assert_eq!(files.dir, Some(PathBuf::from("/songs")));
    assert_eq!(song.title, "Beat");
    files.save(&song);
    assert_eq!(files.dialog, Some("Beat".into()));
    assert!(mem.0.borrow().saves.is_empty());
}

#[test]
fn on_the_web_a_song_in_a_folder_saves_into_that_folder() {
    // The web downloads a save under its file name and reports only that name.
    let mem = MemFiles::default();
    mem.0.borrow_mut().names_only = true;
    let mut files = files(&mem);
    let (folder, beat) = (Path::new("F"), Path::new("F/Beat.flm"));
    files.library.make_folder(folder).unwrap();
    files
        .library
        .store_in(folder, "Beat.flm", &plain(0))
        .unwrap();
    let open = Replace::Open(beat.to_owned());
    let mut song = files.replace(open).unwrap();
    song.add_channel(ChannelKind::Drums);
    // Quick save, twice: both times into F, never a copy at the root.
    for _ in 0..2 {
        files.save(&song);
        files.poll(&mut song);
        assert_eq!(files.path.as_deref(), Some(beat));
        assert_eq!(files.dir.as_deref(), Some(folder));
        assert_eq!(files.library.read(beat).unwrap(), plain(1));
        assert!(!files.library.exists(Path::new("Beat.flm")));
    }
    assert_eq!(mem.0.borrow().saves, vec![(beat.to_owned(), false); 2]);
    // An untitled song saved from the unsaved-changes question while SONGS shows F: into F too.
    let mut song = files.replace(Replace::New).unwrap();
    files.songs_folder = Some(folder.to_owned());
    song.add_channel(ChannelKind::Drums);
    answer_unsaved(&mut files, &mut song, Answer::Extra);
    assert_eq!(
        files.library.read(Path::new("F/New Song.flm")).unwrap(),
        plain(1)
    );
    assert!(!files.library.exists(Path::new("New Song.flm")));
}

#[test]
fn every_quick_save_of_a_song_from_outside_the_songs_folder_writes_into_it() {
    // Ctrl+S and menu Save used to write back into the outside file, the unsaved question's Save into My Songs.
    let ctrl_s = |files: &mut ProjectFiles, song: &mut Song| {
        let ctx = Context::default();
        let event = eframe::egui::Event::Key {
            key: Key::S,
            physical_key: None,
            pressed: true,
            repeat: false,
            modifiers: Modifiers::COMMAND,
        };
        let input = eframe::egui::RawInput {
            events: vec![event],
            ..Default::default()
        };
        let _ = ctx.run(input, |ctx| {
            files.update(ctx, song, true, None);
        });
    };
    for (folder, written) in [
        (None, "/My Songs/Beat.flm"),
        (Some("/My Songs/Demo"), "/My Songs/Demo/Beat.flm"),
    ] {
        for how in ["Ctrl+S", "menu Save", "the unsaved question's Save"] {
            let case = format!("{how} {folder:?}");
            let mem = MemFiles::with_file("/Downloads/Beat.flm", &plain(0));
            let mut files = files(&mem);
            files.root = Some("/My Songs".into());
            files.dir = Some("/My Songs".into());
            files.songs_folder = folder.map(PathBuf::from);
            let mut song = open(&mut files).unwrap();
            assert_eq!(
                files.dir,
                Some("/My Songs".into()),
                "{case}: new names stay in My Songs"
            );
            song.add_channel(ChannelKind::Drums);
            match how {
                "Ctrl+S" => ctrl_s(&mut files, &mut song),
                "menu Save" => files.save(&song),
                _ => drop(answer_unsaved(&mut files, &mut song, Answer::Extra)),
            }
            files.poll(&mut song);
            let mem = mem.0.borrow();
            assert_eq!(mem.saves, [(PathBuf::from(written), false)], "{case}");
            assert_eq!(
                mem.files[Path::new("/Downloads/Beat.flm")],
                plain(0),
                "{case}"
            );
            // The unsaved question's Save then makes a new, untitled song.
            let kept = !how.contains("unsaved");
            assert_eq!(files.path, kept.then(|| written.into()), "{case}");
        }
    }
}

// ---- The `.flm` codec ----

/// A fresh 4.10.19 song.
pub(crate) const FRESH: &[u8] = include_bytes!("../../tests/fixtures/owner-fresh-song.flm");

/// A chunk: id, little-endian payload length, payload.
fn chunk(id: &[u8; 4], payload: &[u8]) -> Vec<u8> {
    [&id[..], &(payload.len() as u32).to_le_bytes(), payload].concat()
}

#[test]
fn the_default_codec_opens_saves_and_reopens_an_flm_project() {
    let mem = MemFiles::with_file("/songs/Fresh.flm", FRESH);
    let mut files = ProjectFiles::with_provider(Box::new(mem.clone()));
    let mut song = open(&mut files).expect("the fixture opens");
    assert_eq!(files.message, None);
    let read = crate::flm::read(FRESH).unwrap();
    assert_eq!(song.channels, read.channels);
    assert_eq!(song.title, "Fresh");
    files.save(&song);
    assert_eq!(files.poll(&mut song), None);
    assert_eq!(files.message, None);
    assert!(!files.modified());
    assert_eq!(open(&mut files), Some(song));
}

#[test]
fn a_save_stamps_the_song_once_it_is_written() {
    let mem = MemFiles::default();
    let mut files = ProjectFiles::with_provider(Box::new(mem.clone()));
    let mut song = Song::default();
    song.add_channel(ChannelKind::Drums);
    files.saved();
    // The OS dialog is cancelled: nothing changes.
    files.save(&song);
    assert!(files.dialog_save(&song, "Sketch"));
    let before = song.clone();
    files.poll(&mut song);
    assert_eq!(song, before);
    assert!(!files.modified());
    // Written: the song has the file's stamp (dates, GUIDs) and counts as saved.
    mem.0.borrow_mut().save_dir = Some("/songs".into());
    assert!(files.dialog_save(&song, "Sketch"));
    files.poll(&mut song);
    assert!(!files.modified());
    let meta = song.file.meta.clone();
    assert!(
        crate::ui::main_menu::meta_secs(&meta.modified).is_some(),
        "{meta:?}"
    );
    assert!(
        crate::ui::main_menu::meta_secs(&meta.created).is_some(),
        "{meta:?}"
    );
    assert!(
        !meta.created.split_once(' ').unwrap().1.contains(':'),
        "{meta:?}"
    );
    assert_ne!(song.file.guid, [0; 16]);
    assert!(song.channels.iter().all(|c| c.file.guid != [0; 16]));
    let written = mem.0.borrow().files[Path::new("/songs/Sketch.flm")].clone();
    let file = crate::flm::read(&written).unwrap();
    assert_eq!((&file.file.meta, file.file.guid), (&meta, song.file.guid));
    // HEAD @8 is the file name, though the song was untitled when it was written.
    assert_eq!(file.title, "Sketch");
    // The next save keeps the created date and the GUIDs.
    let guids: Vec<_> = song.channels.iter().map(|c| c.file.guid).collect();
    let guid = song.file.guid;
    song.title = "Sketch 2".into();
    files.save(&song);
    files.poll(&mut song);
    assert_eq!(
        (song.file.meta.created.as_str(), song.file.guid),
        (meta.created.as_str(), guid)
    );
    assert!(song.channels.iter().map(|c| c.file.guid).eq(guids));
}

#[test]
fn the_readers_notices_show_once_the_song_is_open() {
    // A minimal file, plus a second channel without its RACK.
    let rack = chunk(b"RACK", &[&0u32.to_le_bytes()[..], b"10KR"].concat());
    let lane = chunk(b"TRKH", &chunk(b"DESc", &[0; 1052]));
    let chnl = chunk(b"CHNL", &[&0u32.to_le_bytes()[..], b"20HC", &lane].concat());
    let bytes = [&b"10LF"[..], &rack, &chnl, &chnl].concat();
    let mem = MemFiles::with_file("/songs/Old.flm", &bytes);
    let mut files = ProjectFiles::with_provider(Box::new(mem));
    let song = open(&mut files).expect("it opens");
    assert_eq!(song.channels.len(), 1);
    let details = "Details: Missing rack for Channel #2 skipped";
    assert_eq!(files.message, lines(&[CORRUPTED[0], CORRUPTED[1], details]));
}

#[test]
fn on_the_web_a_song_saved_and_opened_again_from_songs_is_the_same() {
    // The web's provider: a picked file is only its name, a save a download under its name.
    let mem = MemFiles::with_file("Fresh.flm", FRESH);
    mem.0.borrow_mut().names_only = true;
    let mut files = ProjectFiles::with_provider(Box::new(mem.clone()));
    let mut song = open(&mut files).expect("the fixture opens");
    song.tempo_bpm = 97.0;
    files.save(&song);
    files.poll(&mut song);
    assert_eq!(files.message, None);
    let downloaded = mem.0.borrow().files[Path::new("Fresh.flm")].clone();
    assert_eq!(
        files.library.read(Path::new("Fresh.flm")).unwrap(),
        downloaded
    );
    let open = Replace::Open("Fresh.flm".into());
    assert_eq!(files.replace(open), Some(song));
}

/// `expected` with the save stamp of `stamped` (`Song::stamp_save`: version, META dates, the song and channel
/// GUIDs), and nothing else of it.
pub(crate) fn with_stamp(expected: &Song, stamped: &Song) -> Song {
    let mut song = expected.clone();
    song.file.version.clone_from(&stamped.file.version);
    song.file.guid = stamped.file.guid;
    let (meta, from) = (&mut song.file.meta, &stamped.file.meta);
    (meta.created, meta.modified) = (from.created.clone(), from.modified.clone());
    for (c, from) in song.channels.iter_mut().zip(&stamped.channels) {
        c.file.guid = from.file.guid;
    }
    song
}

/// `created` as `META` has it (dashes in the time) and within a minute of now (UTC, as `now()` stamps on the
/// desktop).
fn created_now(created: &str) -> bool {
    let now = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .unwrap()
        .as_secs() as i64;
    let time = created.split_once(' ').map_or("", |(_, t)| t);
    !time.contains(':')
        && crate::ui::main_menu::meta_secs(created).is_some_and(|s| (s - now).abs() < 60)
}

/// `song` is a new, empty song created now.
pub(crate) fn new_now(song: &Song) -> bool {
    let mut new = Song::default();
    new.file.meta.created.clone_from(&song.file.meta.created);
    created_now(&song.file.meta.created) && *song == new
}

#[test]
fn a_new_song_is_created_now() {
    // `created` is set when the song is created, not at its first save.
    let mut files = files(&MemFiles::default());
    let song = files.replace(Replace::New).unwrap();
    assert!(created_now(&song.file.meta.created), "{:?}", song.file.meta);
    assert!(!files.modified());
}

#[test]
fn a_new_song_takes_the_song_options_even_after_the_unsaved_question() {
    use crate::song::TimeSig;
    use crate::ui::song_options::{Mode, SongOptions};
    // `Time division:` 3 / 8 at 120 BPM writes TDIV (3, 4) as (beats per bar, subdivision), and 120 × 8/4 BPM.
    let options = SongOptions {
        tempo: 120,
        mode: Mode::Division,
        beats: 3,
        division: 8,
        ..SongOptions::default()
    };
    let made = TimeSig {
        subdivision: 4,
        beats_per_bar: 3,
    };
    let mut files = files(&MemFiles::default());
    let song = (files.replace(Replace::NewWith(options))).unwrap();
    assert_eq!((song.time_sig, song.tempo_bpm), (made, 240.0));
    assert!(created_now(&song.file.meta.created));
    assert!(!files.modified(), "a new song has nothing to save");
    // A song with unsaved changes asks first; `Discard changes` then makes the same song.
    let mut edited = song.clone();
    edited.add_channel(ChannelKind::Drums);
    files.song_changed();
    assert_eq!(files.replace(Replace::NewWith(options)), None);
    assert!(files.take_overlay().is_some());
    let ctx = eframe::egui::Context::default();
    let song = (files.update(&ctx, &mut edited, false, Some(Answer::Yes))).unwrap();
    assert_eq!(
        (song.time_sig, song.tempo_bpm, song.channels.len()),
        (made, 240.0, 1)
    );
}

#[test]
fn a_song_from_a_template_is_created_now_and_never_saved() {
    // Opening a template creates a new song. The fixture's META says 2026-10-11 07-13-44 / 07:17:45.
    let mem = MemFiles::with_file("/t/Fresh.songtmpl", FRESH);
    let mut files = ProjectFiles::with_provider(Box::new(mem));
    let song = open(&mut files).expect("it opens");
    let meta = &song.file.meta;
    assert!(created_now(&meta.created), "{meta:?}");
    assert_eq!(meta.modified, crate::song::MetaState::default().modified);
    assert!(!files.modified());
}

#[test]
fn the_metadata_lock_survives_templates_and_copies_and_a_new_song_has_none() {
    // META@776: written on every save, so copies (`Save New`) and templates keep it; a new empty song = 0.
    let mut locked = Song::default();
    locked.file.meta.locked = true;
    // The locked Title is META@520, which no save or rename changes.
    locked.file.meta.display_title = "Shown".into();
    let mem = MemFiles::with_file("/t/Locked.songtmpl", &crate::flm::write(&locked).unwrap());
    mem.0.borrow_mut().save_dir = Some("/songs".into());
    let mut files = ProjectFiles::with_provider(Box::new(mem.clone()));
    let mut song = open(&mut files).expect("it opens");
    assert!(song.file.meta.locked);
    files.save_as(&song);
    assert!(files.dialog_save(&song, "Copy"));
    files.poll(&mut song);
    let written = mem.0.borrow().files[Path::new("/songs/Copy.flm")].clone();
    let meta = crate::flm::read(&written).unwrap().file.meta;
    assert!(meta.locked);
    assert_eq!(meta.display_title, "Shown");
    assert_eq!(song.title, "Copy");
    let renamed = Path::new("/songs/Renamed.flm");
    files.path_moved(&mut song, Path::new("/songs/Copy.flm"), Some(renamed));
    assert_eq!(song.title, "Renamed");
    assert_eq!(song.file.meta.display_title, "Shown");
    let new = files.replace(Replace::New).expect("saved, so new now");
    assert!(!new.file.meta.locked);
}

#[test]
fn a_failed_save_lets_go_of_the_song_it_wrote() {
    let mem = MemFiles::with_file("/songs/Beat.flm", &plain(0));
    mem.0.borrow_mut().write_error = Some("disk full".into());
    let mut files = files(&mem);
    let mut song = open(&mut files).unwrap();
    files.save(&song);
    assert!(files.saving.is_some());
    files.poll(&mut song);
    assert!(files.saving.is_none());
}

#[test]
fn a_tuner_off_an_audio_input_saves_its_input_off() {
    // The panel forces Input to 0 there, so such modules save 0, whether or not a setting was edited.
    let mem = MemFiles::with_file("/songs/Fresh.flm", FRESH);
    let mut files = ProjectFiles::with_provider(Box::new(mem.clone()));
    let mut song = open(&mut files).unwrap();
    song.channels[1].rack.add(24);
    song.add_channel(ChannelKind::AudioRecord);
    let input = song.channels.len() - 1;
    song.channels[input].rack.add(24);
    files.save(&song);
    files.poll(&mut song);
    let written = mem.0.borrow().files[Path::new("/songs/Fresh.flm")].clone();
    let file = crate::flm::read(&written).unwrap();
    let input_of = |song: &Song, channel: usize| {
        let module = song.channels[channel].rack.modules.last().unwrap();
        crate::panels::cstm::settings_from_cstm(24, module.file.custom_data.as_deref()?)
            .map(|s| s[&6])
    };
    assert_eq!(input_of(&file, 1), Some(0.0));
    assert_eq!(
        input_of(&file, input),
        None,
        "an audio input's has no blob yet"
    );
    // The song as saved, so it reopens the same and has no unsaved change.
    assert_eq!(input_of(&song, 1), Some(0.0));
    assert!(!files.modified());
    assert_eq!(open(&mut files), Some(song));
}

/// The web's provider (a pick is a name, a save a download under its name) over a session library holding `A.flm`.
fn web_with_a(mem: &MemFiles) -> ProjectFiles {
    let mut s = mem.0.borrow_mut();
    (s.names_only, s.save_dir) = (true, Some(PathBuf::new()));
    drop(s);
    let mut files = files(mem);
    files.library.store("A.flm", &plain(0)).unwrap();
    files
}

#[test]
fn on_the_web_a_save_or_an_import_never_replaces_a_session_song() {
    // The downloads folder cannot be listed, so only the session library knows `A.flm` is taken.
    let mem = MemFiles::default();
    let mut files = web_with_a(&mem);
    // A new song saved as A, from the Save dialog.
    let mut song = song("");
    files.save(&song);
    assert!(files.dialog_save(&song, "A"));
    files.poll(&mut song);
    assert_eq!(files.library.read(Path::new("A.flm")).unwrap(), plain(0));
    assert_eq!(files.path.as_deref(), Some(Path::new("A (1).flm")));
    assert_eq!(song.title, "A (1)");
    // Retitled A and quick saved: its own file is the first free name.
    song.title = "A".into();
    song.add_channel(ChannelKind::Drums);
    files.save(&song);
    files.poll(&mut song);
    assert_eq!(files.library.read(Path::new("A.flm")).unwrap(), plain(0));
    assert_eq!(files.path.as_deref(), Some(Path::new("A (1).flm")));
    assert_eq!(
        files.library.read(Path::new("A (1).flm")).unwrap(),
        plain(1)
    );
    // A picked file named A.
    mem.0.borrow_mut().files.insert("A.flm".into(), plain(2));
    mem.0.borrow_mut().pick = Some("A.flm".into());
    let song = open(&mut files).unwrap();
    assert_eq!(files.library.read(Path::new("A.flm")).unwrap(), plain(0));
    assert_eq!(
        files.library.read(Path::new("A (2).flm")).unwrap(),
        plain(2)
    );
    assert_eq!(files.path.as_deref(), Some(Path::new("A (2).flm")));
    assert_eq!(song.title, "A (2)");
}

#[test]
fn on_the_web_a_song_opened_from_the_bin_saves_into_the_songs_folder() {
    // The web's root is the empty path, which every path starts with, the bin's too.
    let mem = MemFiles::default();
    let mut files = web_with_a(&mem);
    let trash = files.library.trash().unwrap();
    files.library.make_folder(&trash).unwrap();
    let a = Path::new("A.flm");
    files.library.move_to(a, &trash.join(a)).unwrap();
    let open = Replace::Open(trash.join("A.flm"));
    let mut song = files.replace(open).unwrap();
    // The bin open in SONGS, too.
    files.songs_folder = Some(trash.clone());
    song.add_channel(ChannelKind::Drums);
    files.save(&song);
    files.poll(&mut song);
    assert_eq!(files.library.read(Path::new("A.flm")).unwrap(), plain(1));
    assert_eq!(files.library.read(&trash.join("A.flm")).unwrap(), plain(0));
    assert_eq!(files.path.as_deref(), Some(Path::new("A.flm")));
}

#[test]
fn a_song_too_large_to_open_again_is_not_saved() {
    // The web saves through a download, and opens at most MAX_FILE_BYTES (128 MiB there): a larger save could never
    // be opened again. `vec![0; n]` takes zeroed pages, untouched until written.
    let mem = MemFiles::default();
    let mut files = ProjectFiles {
        codec: Codec {
            write: |_| Ok(vec![0; MAX_FILE_BYTES as usize + 1]),
            ..synthetic_codec()
        },
        ..ProjectFiles::with_provider(Box::new(mem.clone()))
    };
    let mut song = song("Big");
    files.path = Some("Big.flm".into());
    files.save(&song);
    files.poll(&mut song);
    assert!(mem.0.borrow().saves.is_empty());
    assert_eq!(files.message, lines(&[SAVE_FAILED, &too_large()]));
    // Exactly the limit still saves (without the web's library copy, which would fill the pages).
    let mem = MemFiles::default();
    let mut files = ProjectFiles {
        codec: Codec {
            write: |_| Ok(vec![0; MAX_FILE_BYTES as usize]),
            ..synthetic_codec()
        },
        copies: false,
        ..ProjectFiles::with_provider(Box::new(mem.clone()))
    };
    files.path = Some("Big.flm".into());
    files.save(&song);
    files.poll(&mut song);
    assert_eq!(mem.0.borrow().saves, [(PathBuf::from("Big.flm"), false)]);
    assert_eq!(files.message, None);
}

/// Opening a song loads each DW Sampler's `PATh` program: a `.dwp` with zones sets Attack and Release from its last
/// zone. The song opened is the clean one.
#[test]
fn opening_a_song_loads_its_dw_programs_attack_and_release() {
    let record = |tag: u32, data: &[u8]| {
        [
            &tag.to_le_bytes()[..],
            &(data.len() as u64).to_le_bytes(),
            data,
        ]
        .concat()
    };
    let floats: Vec<u8> = [0.25f32, 1.0, 1.0, 0.6]
        .iter()
        .flat_map(|f| f.to_le_bytes())
        .collect();
    let zone = record(
        3,
        &[record(0x1F6, b"C4.wav"), record(0x1FD, &floats)].concat(),
    );
    let dwp = [&b"DwPr"[..], &38u32.to_le_bytes(), &record(1, &zone)].concat();
    let path = b"@L@\0My Presets/DW Sampler/Keys.dwp";
    let mut cstm = b"10WDPATh".to_vec();
    cstm.extend((4 + path.len() as u32).to_le_bytes());
    cstm.extend((path.len() as u32).to_le_bytes());
    cstm.extend(path);
    let mut song = Song::default();
    song.add_channel(ChannelKind::Synth(crate::song::SynthKind::DwSampler));
    song.channels[1].rack.modules[0].file.custom_data = Some(cstm);
    let mem = MemFiles::with_file("/songs/Keys.flm", &crate::flm::write(&song).unwrap());
    let mut files = ProjectFiles::with_provider(Box::new(mem));
    files.keep_user_file(Path::new("My Presets/DW Sampler/Keys.dwp"), dwp);
    let song = open(&mut files).expect("the song opens");
    let module = &song.channels[1].rack.modules[0];
    assert_eq!((module.params[1], module.params[2]), (0.25, 0.6));
    assert!(!files.modified());
}

/// A song whose DW Sampler program is missing saves back byte for byte (but the save stamp): its PRMS and its `PATh`
/// floats, which differ, both stay.
#[test]
fn a_song_with_a_missing_dw_program_saves_back_byte_for_byte() {
    let path = b"@L@\0My Presets/DW Sampler/Gone.dwp";
    let mut cstm = b"10WDPATh".to_vec();
    cstm.extend((12 + path.len() as u32).to_le_bytes());
    cstm.extend((path.len() as u32).to_le_bytes());
    cstm.extend(path);
    cstm.extend([0.7f32, 0.8].iter().flat_map(|f| f.to_le_bytes()));
    cstm.extend(b"MIDI\x02\0\0\0\x05\x00");
    // Titled as its file, which a save writes into HEAD.
    let mut song = song("Gone");
    song.add_channel(ChannelKind::Synth(crate::song::SynthKind::DwSampler));
    let module = &mut song.channels[1].rack.modules[0];
    (module.params[1], module.params[2]) = (0.3, 0.4);
    module.file.custom_data = Some(cstm.clone());
    module.file.program_init = Some([0.7, 0.8]);
    let bytes = crate::flm::write(&song).unwrap();
    let mem = MemFiles::with_file("/songs/Gone.flm", &bytes);
    let mut files = ProjectFiles::with_provider(Box::new(mem.clone()));
    let mut opened = open(&mut files).expect("the song opens");
    let module = &opened.channels[1].rack.modules[0];
    assert_eq!((module.params[1], module.params[2]), (0.3, 0.4));
    assert_eq!(module.file.program_init, Some([0.7, 0.8]));
    files.save(&opened);
    assert_eq!(files.poll(&mut opened), None);
    let saved = mem.0.borrow().files[Path::new("/songs/Gone.flm")].clone();
    let back = crate::flm::read(&saved).unwrap();
    assert_eq!(
        back.channels[1].rack.modules[0].file.custom_data,
        Some(cstm)
    );
    let read = crate::flm::read(&bytes).unwrap();
    assert_eq!(crate::flm::write(&with_stamp(&read, &back)).unwrap(), saved);
}

/// A fake factory whose `Templates/<name>.songtmpl` files all hold `song` (written by the real writer).
fn template_factory(names: &[&str], song: &Song) -> &'static crate::content::Factory {
    let bytes = crate::flm::write(song).unwrap();
    let entries = names.iter().map(|name| {
        let bytes = bytes.clone();
        crate::content::Entry::new(format!("Templates/{name}.songtmpl"), move || bytes.clone())
    });
    Box::leak(Box::new(crate::content::Factory::new(entries.collect())))
}

/// Where the fake factory's template `name` is.
fn template_at(name: &str) -> PathBuf {
    Path::new(crate::content::ROOT).join(format!("Templates/{name}.songtmpl"))
}

/// A template song: two drum channels after the master, 87 BPM, 3/4.
fn beat() -> Song {
    let mut song = Song::default();
    song.add_channel(ChannelKind::Drums);
    song.add_channel(ChannelKind::Drums);
    song.tempo_bpm = 87.0;
    song.time_sig = crate::song::TimeSig {
        subdivision: 4,
        beats_per_bar: 3,
    };
    song
}

#[test]
fn a_factory_template_opens_as_an_untitled_new_song_created_now() {
    // A template is loaded, then the song gets the path `New Song (n)`; its channels and tempo stay.
    let mem = MemFiles::with_file("/songs/New Song.flm", &plain(0));
    let mut files = ProjectFiles::with_provider(Box::new(mem));
    files.factory = template_factory(&["D&B", "Lo-Fi"], &beat());
    files.dir = Some("/songs".into());
    let song = (files.replace(Replace::Template(template_at("D&B"))))
        .expect("a factory template opens at once");
    assert_eq!((song.channels.len(), song.tempo_bpm), (3, 87.0));
    assert_eq!(song.time_sig, beat().time_sig);
    assert_eq!(song.title, "New Song (1)");
    assert_eq!(files.path, None);
    assert!(created_now(&song.file.meta.created));
    assert!(!files.modified(), "nothing to save yet");
    // The first save asks, with that name.
    files.save(&song);
    assert_eq!(files.dialog.as_deref(), Some("New Song (1)"));
}

#[test]
fn a_library_template_opens_the_same_way() {
    // SONGS `New` also lists the Original Content's templates, which are in the library, not the factory.
    let mut files = files(&MemFiles::default());
    files.factory = template_factory(&[], &Song::default());
    let mut library = MemoryLibrary::default();
    library.put("Mine.songtmpl".into(), &plain(2)).unwrap();
    files.library = Box::new(library);
    let what = Replace::Template("Mine.songtmpl".into());
    let song = files.replace(what).unwrap();
    assert_eq!((song.channels.len(), song.title.as_str()), (3, "New Song"));
    assert_eq!(files.path, None);
}

#[test]
fn a_factory_template_asks_about_unsaved_changes_first() {
    let mut files = ProjectFiles::with_provider(Box::new(MemFiles::default()));
    files.factory = template_factory(&["D&B"], &beat());
    let mut edited = Song::default();
    edited.add_channel(ChannelKind::Drums);
    files.song_changed();
    let what = Replace::Template(template_at("D&B"));
    assert_eq!(files.replace(what.clone()), None);
    assert!(matches!(
        files.take_overlay(),
        Some(Overlay::Message {
            tag: DISCARD_TAG,
            ..
        })
    ));
    let ctx = eframe::egui::Context::default();
    // `Cancel` keeps the song and drops the template.
    assert_eq!(
        files.update(&ctx, &mut edited, false, Some(Answer::No)),
        None
    );
    assert_eq!(files.waiting(), None);
    // Asked again: `Discard changes` lets it in.
    assert_eq!(files.replace(what), None);
    files.take_overlay();
    let song = (files.update(&ctx, &mut edited, false, Some(Answer::Yes))).unwrap();
    assert_eq!((song.channels.len(), song.tempo_bpm), (3, 87.0));
    assert!(!files.modified());
}

#[test]
fn a_template_that_is_not_there_fails_with_a_message_and_keeps_the_song() {
    let mut files = ProjectFiles::with_provider(Box::new(MemFiles::default()));
    files.factory = template_factory(&["D&B"], &beat());
    let what = Replace::Template(template_at("Missing"));
    assert_eq!(files.replace(what), None);
    assert!(matches!(
        files.take_overlay(),
        Some(Overlay::Message {
            tag: MESSAGE_TAG,
            ..
        })
    ));
}

#[test]
fn songs_made_from_one_factory_template_share_no_guid_with_it_or_each_other() {
    // A song made from a template is a song of its own.
    let mut template = beat();
    template.file.guid = [7; 16];
    for channel in &mut template.channels {
        channel.file.guid = [8; 16];
    }
    let mut files = ProjectFiles::with_provider(Box::new(MemFiles::default()));
    files.factory = template_factory(&["D&B"], &template);
    let mut guids = vec![];
    for _ in 0..2 {
        let what = Replace::Template(template_at("D&B"));
        let current = files.replace(what).unwrap();
        guids.push(current.file.guid);
        guids.extend(current.channels.iter().map(|c| c.file.guid));
    }
    assert!(
        guids.iter().all(|g| *g != [7; 16] && *g != [8; 16]),
        "{guids:?}"
    );
    let (first, second) = guids.split_at(guids.len() / 2);
    assert!(first.iter().all(|g| !second.contains(g)), "{guids:?}");
}

/// Songs from one template share none of its GUIDs (the template is loaded, then it is a new song): each gets a fresh
/// song GUID and fresh channel GUIDs. Nothing in a song refers to a GUID (sends and the inputs' routing go by channel
/// id), so the ids and sends stay the template's. `.flm` keeps its own across saves.
#[test]
fn a_song_from_a_template_gets_fresh_guids_and_a_song_keeps_its_own() {
    let mut template = Song::default();
    template.add_channel(ChannelKind::Drums);
    template.add_channel(ChannelKind::EffectTrack);
    template.channels[1].file.sends[0].target_channel_id = template.channels[2].id;
    template.stamp_save("2026-10-12 10:00:00");
    let bytes = crate::flm::write(&template).unwrap();
    let guids = |s: &Song| {
        let channels = s.channels.iter().map(|c| c.file.guid);
        std::iter::once(s.file.guid)
            .chain(channels)
            .collect::<Vec<_>>()
    };
    let routing = |s: &Song| {
        let sends =
            |c: &crate::song::Channel| c.file.sends.iter().map(|s| s.target_channel_id).collect();
        s.channels
            .iter()
            .map(|c| (c.id, sends(c)))
            .collect::<Vec<(i32, Vec<i32>)>>()
    };
    let mem = MemFiles::with_file("/t/Beat.songtmpl", &bytes);
    let mut files = ProjectFiles::with_provider(Box::new(mem.clone()));
    let a = open(&mut files).expect("it opens");
    let b = open(&mut files).expect("it opens again");
    let all: Vec<_> = [&template, &a, &b].into_iter().flat_map(guids).collect();
    let distinct: std::collections::HashSet<_> = all.iter().collect();
    assert_eq!(distinct.len(), all.len(), "{all:02x?}");
    assert!(!all.contains(&[0; 16]));
    assert_eq!(
        (routing(&a), routing(&b)),
        (routing(&template), routing(&template))
    );
    assert!(!files.modified());
    // The template's song, opened as a song, is the same song.
    mem.0
        .borrow_mut()
        .files
        .insert("/songs/Beat.flm".into(), bytes);
    mem.0.borrow_mut().pick = Some("/songs/Beat.flm".into());
    assert_eq!(
        guids(&open(&mut files).expect("it opens")),
        guids(&template)
    );
}

// ---- The track menu's `Import MIDI Tracks` ----

/// A one-note format-1 file on MIDI channel `ch`, a CC on it too when `cc`.
fn midi_file(ch: u8, cc: bool) -> Vec<u8> {
    use crate::midi::MidiMsg::{Cc, NoteOff, NoteOn};
    use crate::smf::SmfEvent::Voice;
    let mut track = vec![
        (
            0,
            Voice(NoteOn {
                ch,
                key: 60,
                vel: 100,
            }),
        ),
        (
            96,
            Voice(NoteOff {
                ch,
                key: 60,
                vel: 0,
            }),
        ),
    ];
    if cc {
        track.push((
            0,
            Voice(Cc {
                ch,
                cc: 1,
                value: 64,
            }),
        ));
        track.sort_by_key(|e| e.0);
    }
    crate::smf::codec::write(&crate::smf::Smf {
        division: 96,
        tracks: vec![track],
    })
}

#[test]
fn import_midi_tracks_asks_for_a_midi_file_and_hands_it_over_scanned_with_its_channel() {
    let bytes = midi_file(3, true);
    let mem = MemFiles::with_file("/in/Tune.MID", &bytes);
    let mut files = files(&mem);
    files.pick_midi_tracks(42);
    assert_eq!(mem.0.borrow().accepts, [&["mid", "midi"][..]]);
    // No song is opened; nothing shows.
    assert_eq!(files.poll(&mut song("Mine")), None);
    assert_eq!(files.take_overlay(), None);
    let picked = files.take_midi_tracks().expect("handed over");
    assert_eq!(picked.channel, 42);
    assert_eq!(picked.bytes, bytes);
    assert_eq!(picked.scan, crate::smf::import::scan(&bytes).unwrap());
    assert_eq!(picked.scan.used, 1 << 3);
    assert_eq!(files.take_midi_tracks(), None, "once");
    // A song open afterwards is a song again.
    let mem = MemFiles::with_file("/songs/Song.flm", &plain(1));
    let mut files = super::tests::files(&mem);
    files.pick_midi_tracks(1);
    assert_eq!(
        files.poll(&mut song("Mine")),
        None,
        "not a MIDI file: unread"
    );
    assert_eq!(files.take_midi_tracks(), None);
    assert_eq!(open(&mut files).map(|s| s.title), Some("Song".into()));
}

#[test]
fn a_picked_file_that_is_no_midi_file_shows_the_midi_load_error() {
    let mem = MemFiles::with_file("/in/Broken.mid", b"MThd\0\0\0\x06\0\x01");
    let mut files = files(&mem);
    files.pick_midi_tracks(1);
    assert_eq!(files.poll(&mut song("Mine")), None);
    assert_eq!(files.take_midi_tracks(), None);
    let Some(Overlay::Message { lines, .. }) = files.take_overlay() else {
        panic!("no message");
    };
    assert_eq!(lines[0], crate::smf::import::LOAD_FAILED);
    assert_eq!(lines.len(), 2, "with the detail: {lines:?}");
}

// ---- The song-changed flag ----

#[test]
fn the_song_changed_flag_holds_until_a_save_a_new_song_or_a_load() {
    let mem = MemFiles::with_file("/songs/Beat.flm", &plain(1));
    let mut files = files(&mem);
    let mut song = Song::default();
    files.saved();
    // Set with the song as it was saved: the question asks all the same.
    files.song_changed();
    assert!(files.modified());
    // A successful save clears it.
    mem.0.borrow_mut().save_dir = Some("/songs".into());
    assert!(files.dialog_save(&song, "Sketch"));
    files.poll(&mut song);
    assert!(!files.modified());
    // So do a new song, once the question is answered, and a load.
    answer_unsaved(&mut files, &mut song, Answer::Yes).expect("a new song");
    assert!(!files.modified());
    files.song_changed();
    mem.0.borrow_mut().pick = Some("/songs/Beat.flm".into());
    open(&mut files).expect("opened");
    assert!(!files.modified());
}

#[test]
fn opening_a_midi_file_as_a_song_clears_the_song_changed_flag() {
    let mem = MemFiles::with_file("/in/Tune.mid", &midi_file(0, false));
    let mut files = files(&mem);
    files.song_changed();
    let song = open(&mut files).expect("the .mid opens as a song");
    assert_eq!(song.title, "Tune");
    assert!(!files.modified());
}

#[test]
fn only_the_flag_counts_whatever_the_song_holds() {
    // The prompt checks only the song-changed flag.
    let mut files = files(&MemFiles::default());
    let mut song = Song::default();
    files.saved();
    song.add_channel(ChannelKind::Drums);
    assert!(!files.modified());
    assert_eq!(
        files.replace(Replace::New).map(|s| s.channels.len()),
        Some(1)
    );
}

/// Every load writes the whole project to `recent.state` in the user root, as the codec writes it, off the UI thread;
/// The next load replaces it. Nothing reads it back. Where the session keeps copies (the web, tests in memory) there
/// is no disk and nothing is written.
#[cfg(not(target_arch = "wasm32"))]
#[test]
fn every_load_writes_the_project_to_recent_state_in_the_user_root() {
    use crate::project_files::content::tests::{Temp, put};
    use crate::ui::main_menu::{NativeLibrary, SONGS_FOLDER};
    let tmp = Temp::new("fd4-autosave");
    let songs = tmp.0.join(SONGS_FOLDER);
    put(&songs.join("A.flm"), &plain(1));
    put(&songs.join("B.flm"), &plain(2));
    let mut disk = ProjectFiles {
        codec: synthetic_codec(),
        library: Box::new(NativeLibrary::at(songs.clone())),
        ..ProjectFiles::on_disk(&tmp.0)
    };
    let state = tmp.0.join(RECENT_STATE);
    disk.replace(Replace::Open(songs.join("A.flm")))
        .expect("A opens");
    disk.join_autosave();
    assert_eq!(std::fs::read(&state).unwrap(), plain(1));
    disk.replace(Replace::Open(songs.join("B.flm")))
        .expect("B opens");
    disk.join_autosave();
    assert_eq!(std::fs::read(&state).unwrap(), plain(2));
    assert_eq!(std::fs::read(songs.join("A.flm")).unwrap(), plain(1));
    // In memory: nothing to write to.
    let mem = MemFiles::with_file("/songs/C.flm", &plain(1));
    let mut copies = files(&mem);
    assert!(open(&mut copies).is_some());
    assert!(copies.autosave.is_none());
}

/// The [`RECENT_STATE`] writes land in load order, the last load's last, however long an earlier one takes to encode:
/// here the first song's encode takes 400 ms.
#[cfg(not(target_arch = "wasm32"))]
#[test]
fn the_last_loads_recent_state_lands_last_after_a_slow_earlier_one() {
    use crate::project_files::content::tests::{Temp, put};
    use crate::ui::main_menu::{NativeLibrary, SONGS_FOLDER};
    let tmp = Temp::new("fd4-autosave-order");
    let songs = tmp.0.join(SONGS_FOLDER);
    put(&songs.join("Slow.flm"), &plain(1));
    put(&songs.join("Fast.flm"), &plain(2));
    let codec = Codec {
        write: |song| {
            let drums = song.channels.len() - 1;
            if drums == 1 {
                std::thread::sleep(std::time::Duration::from_millis(400));
            }
            Ok(synthetic_file(drums as u8, FORMAT_VERSION, ""))
        },
        ..synthetic_codec()
    };
    let mut disk = ProjectFiles {
        codec,
        library: Box::new(NativeLibrary::at(songs.clone())),
        ..ProjectFiles::on_disk(&tmp.0)
    };
    disk.replace(Replace::Open(songs.join("Slow.flm")))
        .expect("Slow opens");
    disk.replace(Replace::Open(songs.join("Fast.flm")))
        .expect("Fast opens");
    disk.join_autosave();
    // Past the slow encode's end, had it run on after the fast one.
    std::thread::sleep(std::time::Duration::from_millis(600));
    assert_eq!(std::fs::read(tmp.0.join(RECENT_STATE)).unwrap(), plain(2));
}

/// `.mid` opened as a song is a load too: [`RECENT_STATE`] holds the song it made.
#[cfg(not(target_arch = "wasm32"))]
#[test]
fn a_midi_file_opened_as_a_song_writes_recent_state() {
    use crate::project_files::content::tests::{Temp, put};
    use crate::ui::main_menu::{NativeLibrary, SONGS_FOLDER};
    let tmp = Temp::new("fd4-autosave-midi");
    let songs = tmp.0.join(SONGS_FOLDER);
    put(&songs.join("Tune.mid"), &midi_file(0, false));
    let mut disk = ProjectFiles {
        codec: synthetic_codec(),
        library: Box::new(NativeLibrary::at(songs.clone())),
        ..ProjectFiles::on_disk(&tmp.0)
    };
    let song = disk.replace(Replace::Open(songs.join("Tune.mid")));
    let song = song.expect("the MIDI file opens");
    disk.join_autosave();
    let state = std::fs::read(tmp.0.join(RECENT_STATE)).unwrap();
    assert_eq!(state, (disk.codec.write)(&song).unwrap());
}
