use std::path::Path;

use eframe::egui::Context;
use honeydew_audio::sample::decode;
use honeydew_audio::wav_writer::float32_wav;

use super::*;
use crate::flm::chunk::ContentRoot;
use crate::project_files::mem::MemFiles;
use crate::project_files::{CANCEL, KEEP_BOTH, OVERWRITE_TAG, REPLACE_FILES};
use crate::render::{Remainder, single_pass};
use crate::smf::smf_of;
use crate::song::{ChannelKind, Event, SynthKind};
use crate::ui::views::Overlay;
use ExportFormat::*;

/// A low rate keeps the renders cheap.
const RATE: u32 = 8_000;

fn files() -> (ProjectFiles, MemFiles) {
    let mem = MemFiles::default();
    (ProjectFiles::with_provider(Box::new(mem.clone())), mem)
}

/// 120 BPM, a MiniSynth playing A4 for a beat in a 2-beat clip.
fn note_song() -> Song {
    let mut song = Song {
        tempo_bpm: 120.0,
        ..Song::default()
    };
    song.add_channel(ChannelKind::Synth(SynthKind::MiniSynth));
    let note = Event::Note {
        pos: 0,
        len_beats: 1.0,
        key: 69,
        velocity: 204,
        pan: 127,
        slide: false,
    };
    song.channels[1].lanes[0].add_clip(0, 2.0).events.push(note);
    song
}

/// `Cut remainder`, so a render is the song's 2 beats; `High quality` as given.
fn cut(high_quality: bool) -> RenderPage {
    RenderPage {
        remainder: Remainder::Cut,
        high_quality,
        ..RenderPage::default()
    }
}

/// Finishes the requests: the writes, and the overwrite question's `answer`.
fn update(files: &mut ProjectFiles, song: &mut Song, answer: Option<Answer>) {
    files.update(&Context::default(), song, false, answer);
}

fn written(mem: &MemFiles, path: &str) -> Option<Vec<u8>> {
    mem.0.borrow().files.get(Path::new(path)).cloned()
}

/// The lines of the message `files` asks for.
fn message(files: &mut ProjectFiles) -> Option<Vec<String>> {
    match files.take_overlay()? {
        Overlay::Message { lines, .. } => Some(lines),
        _ => None,
    }
}

fn export(format: ExportFormat, path: &str) -> Export {
    Export {
        format,
        path: path.into(),
        split: Split::Off,
    }
}

/// The render job of `song` as `My Tracks/Song.<ext>`.
fn job(song: &Song, format: ExportFormat, page: RenderPage) -> Job {
    let path = format!("My Tracks/Song.{}", format.ext());
    Job::new(
        song,
        &SampleCache::default(),
        export(format, &path),
        page,
        RATE,
    )
    .unwrap()
}

/// A stem a job handed over: its file name and bytes.
type Stem = (String, Vec<u8>);

/// `job` run to its end one block a slice: the titles it showed (in order, without repeats), the stems it handed over
/// and how it ended.
fn run(mut job: Job) -> (Vec<String>, Vec<Stem>, Done) {
    let (mut titles, mut stems): (Vec<String>, _) = (Vec::new(), Vec::new());
    loop {
        let title = job.title();
        if titles.last() != Some(&title) {
            titles.push(title);
        }
        match job.step(&mut || true) {
            None => {}
            Some(Done::Stem(name, bytes)) => stems.push((name, bytes)),
            Some(done) => return (titles, stems, done),
        }
    }
}

/// The file `job` ends with.
fn file(job: Job) -> Vec<u8> {
    match run(job).2 {
        Done::File(bytes) => bytes,
        done => panic!("{done:?}"),
    }
}

#[test]
fn midi_and_zip_are_written_into_their_folders_flash_saved_and_leave_the_song_alone() {
    let (mut files, mem) = files();
    let mut song = note_song();
    files.saved();
    files.export(&song, Midi, "Song", false);
    assert_eq!(files.take_export(), Some(export(Midi, "My MIDI/Song.mid")));
    assert_eq!(
        written(&mem, "My MIDI/Song.mid"),
        Some(smf_of(&song, None).unwrap())
    );
    update(&mut files, &mut song, None);
    assert_eq!(files.take_flash().as_deref(), Some(SAVED));
    // A typed `.zip` ending is the extension's.
    files.export(&song, Zip, "Song.ZIP", false);
    assert_eq!(files.take_export(), Some(export(Zip, "My Songs/Song.zip")));
    assert!(written(&mem, "My Songs/Song.zip").is_some());
    update(&mut files, &mut song, None);
    assert_eq!(files.take_flash().as_deref(), Some(SAVED));
    // An export is no save: the song keeps its (empty) title and stays untitled.
    assert_eq!(song.title, "");
    assert_eq!(files.path, None);
    assert!(!files.modified());
}

#[test]
fn audio_goes_to_my_tracks_through_the_app_and_flm_does_nothing() {
    let (mut files, mem) = files();
    let song = note_song();
    for format in [Wav, Flac, Mp3, Ogg] {
        files.export(&song, format, "Song", false);
        let path = format!("My Tracks/Song.{}", format.ext());
        assert_eq!(files.take_export(), Some(export(format, &path)));
    }
    assert!(mem.0.borrow().files.is_empty(), "the app writes renders");
    files.export(&song, Flm, "Song", false);
    assert_eq!(files.take_export(), None);
    assert_eq!(files.take_overlay(), None);
}

#[test]
fn each_audio_format_encodes_the_render_and_decodes_back() {
    let song = note_song();
    // 16-bit stores trunc(x·32767), 24-bit trunc(x·8388607); float is exact.
    let pcm = |bits: i32| 2.0 / ((1_i32 << (bits - 1)) - 1) as f32;
    for (format, hq, tolerance) in [
        (Wav, false, pcm(16)),
        (Wav, true, 0.0),
        (Flac, false, pcm(16)),
        (Flac, true, pcm(24)),
    ] {
        let page = cut(hq);
        let spec = RenderSpec {
            range: page.range,
            remainder: page.remainder,
            normalize: false,
            high_quality: hq,
            rate: RATE,
            solo: None,
            master_effects: true,
        };
        let rendered = single_pass(&song, &SampleCache::default(), spec).unwrap();
        let decoded = decode(&file(job(&song, format, page))).unwrap();
        assert_eq!(decoded.sample_rate, RATE, "{format:?} {hq}");
        assert_eq!(decoded.left.len(), rendered[0].len(), "{format:?} {hq}");
        let right = decoded.right.as_deref().expect("stereo");
        for (got, want) in [(&decoded.left[..], &rendered[0]), (right, &rendered[1])] {
            let err = (got.iter().zip(want)).fold(0f32, |m, (g, w)| m.max((g - w).abs()));
            assert!(err <= tolerance, "{format:?} {hq}: {err}");
        }
        assert!(rendered[0].iter().any(|x| x.abs() > 0.01), "it has sound");
    }
}

#[test]
fn mp3_encodes_the_render_at_192_or_320_kbps_and_decodes_back() {
    // At 44.1 kHz: the 8 kHz of `job` would go out at 11.025 kHz (`mp3_writer`).
    let song = note_song();
    for (hq, bitrate_index) in [(false, 0b1011), (true, 0b1110)] {
        let page = cut(hq);
        let spec = RenderSpec {
            range: page.range,
            remainder: page.remainder,
            normalize: false,
            high_quality: hq,
            rate: 44_100,
            solo: None,
            master_effects: true,
        };
        let rendered = single_pass(&song, &SampleCache::default(), spec).unwrap();
        let mp3 = export(Mp3, "My Tracks/Song.mp3");
        let job = Job::new(&song, &SampleCache::default(), mp3, page, 44_100).unwrap();
        let (titles, _, done) = run(job);
        assert_eq!(titles.last().unwrap(), "Encoding \"Song.mp3\"");
        let Done::File(file) = done else {
            panic!("{done:?}")
        };
        assert_eq!(file[2] >> 4, bitrate_index, "{hq}");
        let decoded = decode(&file).unwrap();
        assert_eq!(decoded.sample_rate, 44_100);
        assert!(decoded.right.is_some(), "stereo");
        // Gapless: exactly the render, in place.
        assert_eq!(decoded.left.len(), rendered[0].len(), "{hq}");
        let (signal, noise) = (decoded.left.iter().zip(&rendered[0]))
            .fold((0.0, 0.0), |(s, n), (&g, &w)| {
                (s + f64::from(w).powi(2), n + f64::from(g - w).powi(2))
            });
        let snr = 10.0 * (signal / noise).log10();
        assert!(snr >= 25.0, "{hq}: {snr:.1} dB");
        assert!(decoded.left.iter().any(|x| x.abs() > 0.01), "it has sound");
    }
}

#[test]
fn ogg_encodes_the_render_at_quality_0_4_or_1_and_decodes_back() {
    let song = note_song();
    for hq in [false, true] {
        let page = cut(hq);
        let spec = RenderSpec {
            range: page.range,
            remainder: page.remainder,
            normalize: false,
            high_quality: hq,
            rate: RATE,
            solo: None,
            master_effects: true,
        };
        let rendered = single_pass(&song, &SampleCache::default(), spec).unwrap();
        let (titles, _, done) = run(job(&song, Ogg, page));
        assert_eq!(titles.last().unwrap(), "Encoding \"Song.ogg\"");
        let Done::File(file) = done else {
            panic!("{done:?}")
        };
        // The render, pushed in its blocks into the encoder at its rate and quality.
        let mut ogg = OggEncoder::new(RATE, hq);
        for at in (0..rendered[0].len()).step_by(BLOCK) {
            let end = (at + BLOCK).min(rendered[0].len());
            ogg.push(&[&rendered[0][at..end], &rendered[1][at..end]]);
        }
        assert!(file == ogg.finish(), "{hq}");
        // Vorbis keeps the length exactly.
        let decoded = decode(&file).unwrap();
        assert_eq!(decoded.sample_rate, RATE);
        assert!(decoded.right.is_some(), "stereo");
        assert_eq!(decoded.frames(), rendered[0].len(), "{hq}");
        assert!(decoded.left.iter().any(|x| x.abs() > 0.01), "it has sound");
    }
}

#[test]
fn the_busy_titles_go_rendering_processing_then_encoding() {
    let (titles, _, done) = run(job(&note_song(), Wav, cut(false)));
    assert!(matches!(done, Done::File(_)), "{done:?}");
    assert_eq!(
        titles,
        [
            "Rendering \"Song.wav\"",
            "Processing \"Song.wav\"",
            "Encoding \"Song.wav\""
        ]
    );
}

#[test]
fn a_slice_runs_whole_blocks_until_it_is_spent_and_ends_with_its_stage() {
    // The 2-beat clip's song is one bar: 4 beats at 120 BPM and 8 kHz, 16 000 frames, so 16 blocks of 1024.
    let mut slices = 0;
    let mut job = job(&note_song(), Flac, cut(false));
    // Never spent: the render runs to its end, and the slice ends there.
    assert!(job.step(&mut || false).is_none());
    assert_eq!(job.title(), "Processing \"Song.flac\"");
    assert!(job.step(&mut || false).is_none());
    assert_eq!(job.title(), "Encoding \"Song.flac\"");
    // Spent at once: one block per slice.
    while job.step(&mut || true).is_none() {
        slices += 1;
    }
    assert_eq!(slices, 15);
}

#[test]
fn an_existing_file_asks_and_replace_keep_both_and_cancel_do_what_they_say() {
    let mut song = note_song();
    let midi = smf_of(&song, None).unwrap();
    let old = b"old".to_vec();
    for (answer, path, kept) in [
        (Answer::Extra, Some("My MIDI/Song.mid"), false),
        (Answer::Yes, Some("My MIDI/Song (1).mid"), true),
        (Answer::No, None, true),
    ] {
        let (mut files, mem) = files();
        mem.0
            .borrow_mut()
            .files
            .insert("My MIDI/Song.mid".into(), old.clone());
        files.export(&song, Midi, "Song", false);
        assert_eq!(
            files.take_export(),
            None,
            "{answer:?}: nothing before the answer"
        );
        assert_eq!(
            files.take_overlay(),
            Some(Overlay::Message {
                tag: OVERWRITE_TAG,
                lines: vec![OVERWRITE.into(), "Song.mid".into()],
                confirm: Some((KEEP_BOTH.into(), CANCEL.into())),
                extra: Some(REPLACE_FILES.into()),
            })
        );
        assert_eq!(files.waiting(), Some(OVERWRITE_TAG));
        update(&mut files, &mut song, Some(answer));
        assert_eq!(
            files.take_export(),
            path.map(|p| export(Midi, p)),
            "{answer:?}"
        );
        if let Some(path) = path {
            assert_eq!(written(&mem, path).as_ref(), Some(&midi), "{answer:?}");
        }
        assert_eq!(
            written(&mem, "My MIDI/Song.mid") == Some(old.clone()),
            kept,
            "{answer:?}"
        );
        assert_eq!(files.waiting(), None);
    }
    // An audio export asks the same, and keeps both under a free name for the app to render.
    let (mut files, mem) = files();
    for taken in ["My Tracks/Song.wav", "My Tracks/Song (1).wav"] {
        mem.0.borrow_mut().files.insert(taken.into(), old.clone());
    }
    files.export(&song, Wav, "Song", false);
    assert!(message(&mut files).is_some());
    update(&mut files, &mut song, Some(Answer::Yes));
    assert_eq!(
        files.take_export(),
        Some(export(Wav, "My Tracks/Song (2).wav"))
    );
}

#[test]
fn the_archive_holds_the_song_and_the_user_files_it_plays_only() {
    let (mut files, mem) = files();
    let kick = float32_wav(RATE, &[[0.5f32; 8]]);
    files.keep_user_file(Path::new("My Samples/Kicks/Kick.wav"), kick.clone());
    files.keep_user_file(Path::new("My Samples/Unused.wav"), kick.clone());
    let mut song = note_song();
    song.add_channel(ChannelKind::Drums);
    let slots = &mut song.channels[2].drum_kit.as_mut().unwrap().slots;
    slots[0].file.path = Some(ContentPath::Shorthand {
        root: ContentRoot::User,
        relative: "My Samples/Kicks/Kick.wav".into(),
    });
    // Bundled content, which no archive takes; the other pads play built-in one-shots.
    slots[1].file.path = Some(ContentPath::Shorthand {
        root: ContentRoot::App,
        relative: "Drums/Snare.wav".into(),
    });
    files.export(&song, Zip, "Beat", false);
    let zip = written(&mem, "My Songs/Beat.zip").expect("written");
    let entries = crate::zip::read(&zip, 1 << 30).unwrap();
    let names: Vec<_> = entries.iter().map(|e| e.path.as_str()).collect();
    assert_eq!(names, ["Beat.flm", "My Samples/Kicks/Kick.wav"]);
    assert_eq!(entries[1].data, kick);
    let (read, _) = (files.codec.read)(&entries[0].data).unwrap();
    assert_eq!(read.channels.len(), 3);
    assert_eq!(
        read.channels[2].drum_kit.as_ref().unwrap().slots[0]
            .file
            .path,
        song.channels[2].drum_kit.as_ref().unwrap().slots[0]
            .file
            .path
    );
}

/// Imported content never leaves the user folder. A file really in the Original Content
/// folder (`@R@`, an `@L@` path into it) stays out of the archive, and the flash says so; the user's own files are
/// still packed.
#[cfg(not(target_arch = "wasm32"))]
#[test]
fn the_archive_leaves_out_original_content_and_keeps_the_users_files() {
    use crate::project_files::content::tests::{Temp, put};
    let tmp = Temp::new("export-original");
    let mem = MemFiles::default();
    let mut files = ProjectFiles::on_disk(&tmp.0);
    files.provider = Box::new(mem.clone());
    let mut song = Song::default();
    song.add_channel(ChannelKind::Drums);
    let slots = &mut song.channels[1].drum_kit.as_mut().unwrap().slots;
    let pads = [
        (
            ContentRoot::App,
            "Drum Samples/Gen Kick.wav",
            "Original Content/",
        ),
        (
            ContentRoot::User,
            "Original Content/Drum Samples/Gen Snare.wav",
            "",
        ),
        (ContentRoot::User, "My Samples/Gen Hat.wav", ""),
    ];
    for (slot, (root, relative, folder)) in slots.iter_mut().zip(pads) {
        put(
            &tmp.0.join(format!("{folder}{relative}")),
            &float32_wav(RATE, &[[0.5f32; 8]]),
        );
        slot.file.path = Some(ContentPath::Shorthand {
            root,
            relative: relative.into(),
        });
    }
    files.export(&song, Zip, "Beat", false);
    update(&mut files, &mut song, None);
    let zip = written(&mem, tmp.0.join("My Songs/Beat.zip").to_str().unwrap()).expect("written");
    let entries = crate::zip::read(&zip, 1 << 30).unwrap();
    let names: Vec<_> = entries.iter().map(|e| e.path.as_str()).collect();
    assert_eq!(names, ["Beat.flm", "My Samples/Gen Hat.wav"]);
    assert_eq!(
        files.take_flash().as_deref(),
        Some("Saved!\n2 original content files not included")
    );
}

/// A tagged record of a `.dwp`.
fn record(tag: u32, data: &[u8]) -> Vec<u8> {
    let len = (data.len() as u64).to_le_bytes();
    [&tag.to_le_bytes()[..], &len, data].concat()
}

#[test]
fn the_archive_finds_the_user_files_of_stretched_slots_soundfonts_and_dw_programs() {
    let (mut files, mem) = files();
    let user = |relative: &str| ContentPath::Shorthand {
        root: ContentRoot::User,
        relative: relative.into(),
    };
    // A program whose one zone is a file beside it, in `Keys/`.
    let zone = record(0x1F6, b"%ILSharedData%\\Keys\\Keys C4.wav");
    let program = [
        &b"DwPr"[..],
        &38u32.to_le_bytes(),
        &record(1, &record(3, &zone)),
    ]
    .concat();
    // Not audio at all: the archive takes the files as they are, decoded or not.
    for (path, bytes) in [
        ("My Samples/Loop.wav", b"loop".to_vec()),
        ("My SoundFonts/Piano.sf2", b"sf2".to_vec()),
        ("My Programs/Keys.dwp", program.clone()),
        ("My Programs/Keys/Keys C4.wav", b"c4".to_vec()),
        ("My Samples/Unused.wav", b"no".to_vec()),
    ] {
        files.keep_user_file(Path::new(path), bytes);
    }
    let mut song = Song::default();
    song.add_channel(ChannelKind::Drums);
    let slot = &mut song.channels[1].drum_kit.as_mut().unwrap().slots[0];
    slot.file.path = Some(user("My Samples/Loop.wav"));
    // Its `STR1` stretch.
    slot.file.stretch_mode = 1;
    song.add_channel(ChannelKind::Synth(SynthKind::SoundFontPlayer));
    let sf2d = crate::flm::sf2d::Sf2d {
        path: user("My SoundFonts/Piano.sf2"),
        patch: 0,
    };
    song.channels[2].rack.modules[0].file.custom_data = Some(sf2d.write(None).unwrap());
    song.add_channel(ChannelKind::Synth(SynthKind::DwSampler));
    let path = b"@L@\0My Programs/Keys.dwp";
    let cstm = [&(path.len() as u32).to_le_bytes()[..], path, &[0; 8]].concat();
    let cstm = [&b"10WDPATh"[..], &(cstm.len() as u32).to_le_bytes(), &cstm].concat();
    song.channels[3].rack.modules[0].file.custom_data = Some(cstm);
    files.export(&song, Zip, "Kit", false);
    let zip = written(&mem, "My Songs/Kit.zip").expect("written");
    let entries = crate::zip::read(&zip, 1 << 30).unwrap();
    let names: Vec<_> = entries.iter().map(|e| e.path.as_str()).collect();
    assert_eq!(
        names,
        [
            "Kit.flm",
            "My Programs/Keys.dwp",
            "My Programs/Keys/Keys C4.wav",
            "My Samples/Loop.wav",
            "My SoundFonts/Piano.sf2"
        ]
    );
    assert_eq!(entries[1].data, program);
    assert_eq!(entries[2].data, b"c4");
}

#[test]
fn a_test_slice_is_spent_after_a_fixed_number_of_blocks() {
    let mut over = slice();
    let spent: Vec<bool> = (0..TEST_SLICE_BLOCKS).map(|_| over()).collect();
    assert_eq!(spent.iter().filter(|&&s| s).count(), 1);
    assert!(spent[TEST_SLICE_BLOCKS - 1]);
}

#[test]
fn a_written_render_flashes_render_complete_with_where_it_went() {
    let (mut files, mem) = files();
    let mut song = note_song();
    let flac = export(Flac, "My Tracks/Song.flac");
    files.write_export(&flac, b"fLaC".to_vec());
    update(&mut files, &mut song, None);
    assert_eq!(
        files.take_flash().as_deref(),
        Some("Render complete\nSaved to: My Tracks/Song.flac")
    );
    assert_eq!(written(&mem, "My Tracks/Song.flac"), Some(b"fLaC".to_vec()));
    // The web reports the name it downloaded under.
    mem.0.borrow_mut().names_only = true;
    files.write_export(&flac, b"fLaC".to_vec());
    update(&mut files, &mut song, None);
    assert_eq!(
        files.take_flash().as_deref(),
        Some("Render complete\nSaved to: Song.flac")
    );
    assert_eq!(song.title, "", "a render is no save");
}

#[test]
fn each_failure_shows_its_message() {
    let (mut files, mem) = files();
    let song = Song::default();
    // MIDI with no lane to write.
    files.export(&song, Midi, "Song", false);
    assert_eq!(message(&mut files), Some(vec![NO_DATA.into()]));
    assert_eq!(files.take_export(), None);
    // No audible frame; a render that cannot be built or is too long.
    assert!(matches!(
        run(job(&song, Wav, cut(false))).2,
        Done::Failed(RenderError::Silent)
    ));
    files.render_failed(&RenderError::Silent);
    assert_eq!(message(&mut files), Some(vec![NO_SOUND.into()]));
    files.render_failed(&RenderError::TooLong);
    assert_eq!(message(&mut files), Some(vec![RENDER_FAILED.into()]));
    assert!(mem.0.borrow().files.is_empty());
    // A write that fails: the format's message and what went wrong.
    mem.0.borrow_mut().write_error = Some("disk full".into());
    let song = note_song();
    for (format, title) in [(Midi, SAVE_FILE_FAILED), (Zip, ARCHIVE_FAILED)] {
        files.export(&song, format, "Song", false);
        assert!(files.take_export().is_some(), "{format:?} went ahead");
        update(&mut files, &mut song.clone(), None);
        assert_eq!(
            message(&mut files),
            Some(vec![title.into(), "disk full".into()]),
            "{format:?}"
        );
        assert_eq!(files.take_flash(), None);
    }
    files.write_export(&export(Wav, "My Tracks/Song.wav"), b"RIFF".to_vec());
    update(&mut files, &mut song.clone(), None);
    assert_eq!(
        message(&mut files),
        Some(vec![RENDER_FAILED.into(), "disk full".into()])
    );
    // A song that cannot be written into the archive.
    files.codec.write = |_| Err("no".into());
    files.export(&song, Zip, "Song", false);
    assert_eq!(
        message(&mut files),
        Some(vec![SAVE_FAILED.into(), "no".into()])
    );
    assert_eq!(files.take_export(), None);
}

#[test]
fn quick_render_takes_the_stored_audio_format_and_wav_for_any_other() {
    assert_eq!(quick_render_format(Flac.id()), Flac);
    assert_eq!(quick_render_format(Mp3.id()), Mp3);
    assert_eq!(quick_render_format(Ogg.id()), Ogg);
    for id in [Wav.id(), Flm.id(), Midi.id(), Zip.id(), 4, 200] {
        assert_eq!(quick_render_format(id), Wav, "{id}");
    }
}

#[test]
fn quick_render_names_the_file_after_the_song_or_new_song() {
    let (mut files, _) = files();
    let mut song = note_song();
    files.quick_render(&song, Flac);
    assert_eq!(
        files.take_export(),
        Some(export(Flac, "My Tracks/New Song.flac"))
    );
    song.title = "Beat".into();
    files.quick_render(&song, Wav);
    assert_eq!(files.take_export(), Some(export(Wav, "My Tracks/Beat.wav")));
}

// ---- Split mixer tracks and Enable master effects ----

/// A song of MiniSynth channels after the master, named as given, each playing `note_song`'s note in a 2-beat clip, or
/// with no clip (`false`).
fn stems_song(channels: &[(&str, bool)]) -> Song {
    let mut song = note_song();
    let clip = song.channels[1].lanes[0].clips.pop().unwrap();
    for (at, &(name, content)) in (1..).zip(channels) {
        if at > 1 {
            song.add_channel(ChannelKind::Synth(SynthKind::MiniSynth));
        }
        song.channels[at].name = name.into();
        if content {
            song.channels[at].lanes[0].clips.push(clip.clone());
        }
    }
    song
}

/// `Cut remainder` with `Split mixer tracks` on and `Enable master effects` as given.
fn split(master_effects: bool) -> RenderPage {
    RenderPage {
        split: true,
        master_effects,
        ..cut(false)
    }
}

fn folder(path: &str) -> Export {
    Export {
        split: Split::Folder,
        ..export(Wav, path)
    }
}

/// The split render job of `song` into `My Tracks/Song - Multitrack wav`.
fn stems_job(song: &Song, page: RenderPage) -> Job {
    let export = folder("My Tracks/Song - Multitrack wav");
    Job::new(song, &SampleCache::default(), export, page, RATE).unwrap()
}

/// A master Leveller at a low Gain.
fn master_gain(song: &mut Song) {
    let id = song.channels[0].rack.add(20);
    song.channels[0].rack.get_mut(id).unwrap().params[1] = 0.3;
}

fn peak(wav: &[u8]) -> f32 {
    let decoded = decode(wav).unwrap();
    let right = decoded.right.unwrap();
    (decoded.left.iter().chain(right.iter())).fold(0f32, |m, x| m.max(x.abs()))
}

#[test]
fn split_goes_into_a_multitrack_folder_with_one_stem_per_channel_and_numbered_duplicates() {
    let (mut files, _) = files();
    let song = stems_song(&[
        ("Lead", true),
        ("Lead", true),
        ("Pad", false),
        ("lead", true),
        ("Bass/Sub", true),
    ]);
    files.export(&song, Wav, "Song", true);
    assert_eq!(
        files.take_export(),
        Some(folder("My Tracks/Song - Multitrack wav"))
    );
    files.export(&song, Flac, "Song", true);
    assert_eq!(
        files.take_export(),
        Some(Export {
            split: Split::Folder,
            ..export(Flac, "My Tracks/Song - Multitrack flac")
        })
    );
    // Split is only for audio; a ZIP is the archive whatever the page says.
    files.export(&song, Zip, "Song", true);
    assert_eq!(files.take_export(), Some(export(Zip, "My Songs/Song.zip")));
    // The channel without a clip has no stem; names that differ only in case are the same file on some desktops; a
    // name's `/` cannot be in a file name.
    let (_, stems, done) = run(stems_job(&song, split(false)));
    assert!(matches!(done, Done::Stems), "{done:?}");
    let names: Vec<_> = stems.iter().map(|(name, _)| name.as_str()).collect();
    assert_eq!(
        names,
        ["Lead.wav", "Lead 1.wav", "lead 2.wav", "Bass_Sub.wav"]
    );
    for (name, bytes) in &stems {
        assert!(peak(bytes) > 0.01, "{name} has sound");
    }
}

#[test]
fn the_progress_names_each_stem_among_the_stems_with_its_stage() {
    let song = stems_song(&[("Lead", true), ("Pad", false), ("Lead", true)]);
    let (titles, _, _) = run(stems_job(&song, split(false)));
    assert_eq!(
        titles,
        [
            "Exporting channel 1: Lead\nRendering \"Lead.wav\"",
            "Exporting channel 1: Lead\nProcessing \"Lead.wav\"",
            "Exporting channel 1: Lead\nEncoding \"Lead.wav\"",
            "Exporting channel 2: Lead\nRendering \"Lead 1.wav\"",
            "Exporting channel 2: Lead\nProcessing \"Lead 1.wav\"",
            "Exporting channel 2: Lead\nEncoding \"Lead 1.wav\"",
        ]
    );
}

#[test]
fn silent_stems_are_skipped_and_with_no_stem_left_the_song_has_no_sound() {
    let mut song = stems_song(&[("Lead", true), ("Muted", true), ("Bass", true)]);
    // Muted: its stem renders, and has no sound.
    song.channels[2].params.0[2] = 1.0;
    let (titles, stems, done) = run(stems_job(&song, split(false)));
    let names: Vec<_> = stems.iter().map(|(name, _)| name.as_str()).collect();
    assert_eq!(names, ["Lead.wav", "Bass.wav"]);
    assert!(titles.contains(&"Exporting channel 2: Muted\nProcessing \"Muted.wav\"".to_owned()));
    assert!(matches!(done, Done::Stems), "{done:?}");
    for channel in &mut song.channels[1..] {
        channel.params.0[2] = 1.0;
    }
    let (_, stems, done) = run(stems_job(&song, split(false)));
    assert!(stems.is_empty());
    assert!(
        matches!(done, Done::Failed(RenderError::Silent)),
        "{done:?}"
    );
    // No channel with content: no stem to render at all.
    let song = stems_song(&[("Pad", false)]);
    let export = folder("My Tracks/Song - Multitrack wav");
    let job = Job::new(&song, &SampleCache::default(), export, split(false), RATE);
    assert!(matches!(job, Err(RenderError::Silent)));
}

#[test]
fn a_master_gain_changes_the_stems_only_with_master_effects_on_and_the_mix_always() {
    let plain = stems_song(&[("Lead", true)]);
    let mut song = plain.clone();
    master_gain(&mut song);
    let stem = |song: &Song, on: bool| run(stems_job(song, split(on))).1.remove(0).1;
    let bare = stem(&plain, false);
    assert_eq!(stem(&plain, true), bare, "no master effect, no change");
    assert_eq!(
        stem(&song, false),
        bare,
        "off: the master's rack is left out"
    );
    assert!(
        peak(&stem(&song, true)) < 0.9 * peak(&bare),
        "on: through it"
    );
    // The mix (Split off) always goes through the master, whatever the hidden toggle says.
    let mix = |master_effects| {
        let page = RenderPage {
            master_effects,
            ..cut(false)
        };
        file(job(&song, Wav, page))
    };
    assert_eq!(mix(false), mix(true));
    assert!(peak(&mix(false)) < 0.9 * peak(&file(job(&plain, Wav, cut(false)))));
}

#[test]
fn an_existing_stem_folder_asks_and_keep_both_numbers_the_folder() {
    let mut song = stems_song(&[("Lead", true)]);
    for (answer, path) in [
        (Answer::Extra, Some("My Tracks/Song - Multitrack wav")),
        (Answer::Yes, Some("My Tracks/Song - Multitrack wav (2)")),
        (Answer::No, None),
    ] {
        let (mut files, mem) = files();
        for old in [
            "My Tracks/Song - Multitrack wav/Old.wav",
            "My Tracks/Song - Multitrack wav (1)/Old.wav",
        ] {
            mem.0.borrow_mut().files.insert(old.into(), b"old".to_vec());
        }
        files.export(&song, Wav, "Song", true);
        assert_eq!(files.take_export(), None);
        assert_eq!(
            message(&mut files),
            Some(vec![OVERWRITE.into(), "Song - Multitrack wav".into()])
        );
        update(&mut files, &mut song, Some(answer));
        assert_eq!(files.take_export(), path.map(folder), "{answer:?}");
    }
}

#[test]
fn the_stems_are_written_into_their_folder_and_flash_it_once_all_are_written() {
    let (mut files, mem) = files();
    let mut song = stems_song(&[("Lead", true)]);
    files.export(&song, Wav, "Song", true);
    let export = files.take_export().unwrap();
    files.write_stem(&export, "Lead.wav", b"lead".to_vec());
    update(&mut files, &mut song, None);
    assert_eq!(files.take_flash(), None, "more stems may come");
    files.write_stem(&export, "Bass.wav", b"bass".to_vec());
    files.stems_rendered();
    assert_eq!(files.take_flash(), None, "a write is under way");
    update(&mut files, &mut song, None);
    assert_eq!(
        files.take_flash().as_deref(),
        Some("Render complete\nSaved to: My Tracks/Song - Multitrack wav")
    );
    for (path, bytes) in [("Lead.wav", &b"lead"[..]), ("Bass.wav", b"bass")] {
        let path = format!("My Tracks/Song - Multitrack wav/{path}");
        assert_eq!(written(&mem, &path).as_deref(), Some(bytes));
    }
    // The writes are no song save.
    assert_eq!(song.title, "");
    assert_eq!(files.path, None);
    // A write that fails: its message, and no flash.
    mem.0.borrow_mut().write_error = Some("disk full".into());
    files.export(&song, Wav, "Take", true);
    let export = files.take_export().unwrap();
    files.write_stem(&export, "Lead.wav", b"lead".to_vec());
    files.stems_rendered();
    update(&mut files, &mut song, None);
    assert_eq!(
        message(&mut files),
        Some(vec![RENDER_FAILED.into(), "disk full".into()])
    );
    assert_eq!(files.take_flash(), None);
}

#[test]
fn the_web_downloads_one_archive_of_the_stems() {
    let (mut files, mem) = files();
    mem.0.borrow_mut().names_only = true;
    let mut song = stems_song(&[("Lead", true), ("Lead", true)]);
    files.export(&song, Wav, "Song", true);
    let zip_export = files.take_export().unwrap();
    assert_eq!(
        zip_export,
        Export {
            split: Split::Zip,
            ..export(Wav, "My Tracks/Song - Multitrack wav.zip")
        }
    );
    let job = Job::new(
        &song,
        &SampleCache::default(),
        zip_export.clone(),
        split(false),
        RATE,
    );
    let (titles, stems, done) = run(job.unwrap());
    assert!(stems.is_empty(), "nothing is written before the archive");
    assert_eq!(
        titles[0],
        "Exporting channel 1: Lead\nRendering \"Lead.wav\""
    );
    let Done::File(zip) = done else {
        panic!("{done:?}")
    };
    let entries = crate::zip::read(&zip, 1 << 30).unwrap();
    let names: Vec<_> = entries.iter().map(|e| e.path.as_str()).collect();
    assert_eq!(names, ["Lead.wav", "Lead 1.wav"]);
    for entry in &entries {
        assert!(peak(&entry.data) > 0.01, "{}", entry.path);
    }
    files.write_export(&zip_export, zip);
    update(&mut files, &mut song, None);
    assert_eq!(
        files.take_flash().as_deref(),
        Some("Render complete\nSaved to: Song - Multitrack wav.zip")
    );
    assert!(written(&mem, "Song - Multitrack wav.zip").is_some());
}

#[test]
fn quick_render_renders_the_mix_whatever_split_says() {
    let (mut files, _) = files();
    files.quick_render(&note_song(), Wav);
    assert_eq!(
        files.take_export(),
        Some(export(Wav, "My Tracks/New Song.wav"))
    );
}

#[test]
fn review_probe_a_stem_is_its_channel_through_its_sends_and_fx_channels_get_none() {
    let mut song = stems_song(&[("Lead", true), ("Bass", true)]);
    song.add_channel(ChannelKind::EffectTrack);
    let fx = song.channels.len() - 1;
    let fx_id = song.channels[fx].id;
    // Both synths output only into the FX channel, which runs a Leveller at a low gain.
    for c in [1, 2] {
        song.channels[c].file.sends[0].target_channel_id = fx_id;
    }
    let id = song.channels[fx].rack.add(20);
    song.channels[fx].rack.get_mut(id).unwrap().params[1] = 0.3;
    let (_, stems, done) = run(stems_job(&song, split(false)));
    assert!(matches!(done, Done::Stems), "{done:?}");
    let names: Vec<_> = stems.iter().map(|(name, _)| name.as_str()).collect();
    assert_eq!(
        names,
        ["Lead.wav", "Bass.wav"],
        "no stem for the FX channel"
    );
    // Lead's stem is the mix with Bass muted: Lead through the FX channel, nothing of Bass.
    let mut lead_only = song.clone();
    lead_only.channels[2].params.0[2] = 1.0;
    assert_eq!(stems[0].1, file(job(&lead_only, Wav, cut(false))));
    let mut bass_only = song.clone();
    bass_only.channels[1].params.0[2] = 1.0;
    assert_eq!(stems[1].1, file(job(&bass_only, Wav, cut(false))));
}

#[test]
fn review_probe_the_first_stem_is_soloed_too() {
    // Two channels: the first stem must not be the mix.
    let song = stems_song(&[("Lead", true), ("Bass", true)]);
    let (_, stems, _) = run(stems_job(&song, split(false)));
    let mix = file(job(&song, Wav, cut(false)));
    assert_ne!(stems[0].1, mix);
    assert_eq!(stems[0].1, stems[1].1, "same note, same synth: equal stems");
}

#[test]
fn review_probe_names_no_file_can_have_take_an_underscore() {
    let song = stems_song(&[("CON", true), ("", true), (".hidden", true), ("..", true)]);
    let (_, stems, _) = run(stems_job(&song, split(false)));
    let names: Vec<_> = stems.iter().map(|(name, _)| name.as_str()).collect();
    assert_eq!(names, ["_CON.wav", "_.wav", "_.hidden.wav", "_...wav"]);
}

#[test]
fn review_probe_an_audio_clip_is_content_and_an_fx_channel_is_not() {
    let mut song = stems_song(&[("Lead", false)]);
    song.add_channel(ChannelKind::AudioRecord);
    let rec = song.channels.len() - 1;
    song.channels[rec].lanes[0].add_clip(0, 1.0);
    song.add_channel(ChannelKind::EffectTrack);
    let fx = song.channels.len() - 1;
    let note = Event::Note {
        pos: 0,
        len_beats: 1.0,
        key: 60,
        velocity: 100,
        pan: 127,
        slide: false,
    };
    song.channels[fx].lanes[0]
        .add_clip(0, 1.0)
        .events
        .push(note);
    let name = format!("{}.wav", song.channels[rec].name);
    assert_eq!(stems_of(&song, "wav"), [(rec, name)]);
}

#[test]
fn a_stem_is_handed_over_before_the_next_one_is_built() {
    let song = stems_song(&[("Lead", true), ("Bass", true)]);
    let mut job = stems_job(&song, split(false));
    let stem = loop {
        if let Some(done) = job.step(&mut || false) {
            break done;
        }
    };
    assert!(
        matches!(&stem, Done::Stem(name, _) if name == "Lead.wav"),
        "{stem:?}"
    );
    // Nothing of the next stem is held yet; its title is already up.
    assert!(matches!(job.stage, Stage::Done));
    assert_eq!(
        job.title(),
        "Exporting channel 2: Bass\nRendering \"Bass.wav\""
    );
    assert!(job.step(&mut || true).is_none());
    assert!(matches!(job.stage, Stage::Rendering(_)));
}

/// Today's export of `song` with `page` (and `solo`): the single pass held whole, pushed 1024 frames at a time into
/// `format`'s encoder.
fn single_pass_file(
    song: &Song,
    format: ExportFormat,
    page: RenderPage,
    solo: Option<usize>,
) -> Vec<u8> {
    let spec = RenderSpec {
        range: page.range,
        remainder: page.remainder,
        normalize: page.normalize,
        high_quality: page.high_quality,
        rate: RATE,
        solo,
        master_effects: !page.split || page.master_effects,
    };
    let audio = single_pass(song, &SampleCache::default(), spec).unwrap();
    let mut encoder = Encoder::new(format, RATE, page.high_quality, audio[0].len());
    for at in (0..audio[0].len()).step_by(BLOCK) {
        let end = (at + BLOCK).min(audio[0].len());
        encoder.push(&audio[0][at..end], &audio[1][at..end]);
    }
    encoder.finish()
}

#[test]
fn every_format_writes_the_bytes_of_the_single_pass() {
    // A beat of silence first: trimmed, the stream's blocks start in the middle of the render's.
    let mut song = note_song();
    let Event::Note { pos, .. } = &mut song.channels[1].lanes[0].clips[0].events[0] else {
        unreachable!()
    };
    *pos = crate::song::PPQ / 2;
    let trim = RenderPage {
        range: crate::render::RenderRange::TrimSilence,
        normalize: true,
        ..RenderPage::default()
    };
    let wrap = RenderPage {
        remainder: Remainder::Wrap,
        ..RenderPage::default()
    };
    for page in [trim, wrap, cut(false)] {
        for high_quality in [false, true] {
            let page = RenderPage {
                high_quality,
                ..page
            };
            for format in [Wav, Flac, Mp3, Ogg] {
                let want = single_pass_file(&song, format, page, None);
                assert!(
                    file(job(&song, format, page)) == want,
                    "{format:?} {page:?}"
                );
            }
        }
    }
}

#[test]
fn each_stem_writes_the_bytes_of_its_single_pass() {
    let mut song = stems_song(&[("Lead", true), ("Bass", true)]);
    master_gain(&mut song);
    let page = RenderPage {
        remainder: Remainder::Leave,
        normalize: true,
        ..split(false)
    };
    let (_, stems, done) = run(stems_job(&song, page));
    assert!(matches!(done, Done::Stems), "{done:?}");
    assert_eq!(stems.len(), 2);
    for (channel, (name, bytes)) in (1..).zip(stems) {
        assert!(
            bytes == single_pass_file(&song, Wav, page, Some(channel)),
            "{name}"
        );
    }
}

#[test]
fn an_flp_puts_the_effects_on_the_channel_s_insert_and_with_rendered_audio_holds_its_render_too() {
    use crate::flp::{ChunkBody, FlpFile, id};
    let (mut files, mem) = files();
    let mut song = note_song();
    song.channels[1].name = "Lead".into();
    let rack = &mut song.channels[1].rack;
    let eq = rack.add(3);
    // Band 1: +6 dB.
    rack.get_mut(eq).unwrap().params[1] = 0.75;
    rack.add(35);
    files.export(&song, Flp, "Song", true);
    // `Include rendered audio`: the app renders the stems first.
    let export = files.take_export().expect("rendered first");
    assert_eq!(export.split, Split::Flp);
    assert!(written(&mem, "My Songs/Song.zip").is_none());
    let job = Job::new(
        &song,
        &SampleCache::default(),
        export.clone(),
        cut(false),
        RATE,
    )
    .unwrap();
    let Done::Rendered(rendered, stems) = run(job).2 else {
        panic!("the stems");
    };
    assert_eq!(stems.iter().map(|s| s.0).collect::<Vec<_>>(), [1]);
    files.flp_rendered(&rendered, &export, &stems);
    let zip = written(&mem, "My Songs/Song.zip").unwrap();
    let entries = crate::zip::read(&zip, u64::MAX).unwrap();
    let names: Vec<_> = entries.iter().map(|e| e.path.as_str()).collect();
    assert_eq!(names, ["Song.flp", "Lead.wav", "Lead (rendered).wav"]);
    assert_eq!(entries[2].data, stems[0].1);
    assert_eq!(
        files.export_notices,
        [
            "Lead: Reverb 2 is Fruity Reeverb 2 at its default settings",
            "Lead: MiniSynth has no .flp counterpart, exported as a sample of one note"
        ]
    );
    // The mixer's insert 1, Lead's, holds Fruity Parametric EQ 2 with its band and Fruity Reeverb 2 with none.
    let file = FlpFile::parse(&entries[0].data).unwrap();
    let ChunkBody::Events(events) = &file.chunks[1].body else {
        panic!("events");
    };
    let text = |e: &crate::flp::Event| {
        String::from_utf16_lossy(
            &(e.data().unwrap().chunks(2))
                .map(|c| u16::from_le_bytes([c[0], c[1]]))
                .take_while(|&u| u != 0)
                .collect::<Vec<_>>(),
        )
    };
    let named = |name: &str| {
        events
            .iter()
            .position(|e| e.id == 201 && text(e) == name)
            .unwrap()
    };
    let eq = named("Fruity Parametric EQ 2");
    let state = events[eq..]
        .iter()
        .find(|e| e.id == 213)
        .unwrap()
        .data()
        .unwrap();
    assert_eq!(
        (
            state.len(),
            i32::from_le_bytes(state[4..8].try_into().unwrap())
        ),
        (354, 600)
    );
    let reverb = named("Fruity Reeverb 2");
    assert!(
        events[reverb..]
            .iter()
            .take_while(|e| e.id != 98)
            .all(|e| e.id != 213)
    );
    let lead = events
        .iter()
        .position(|e| e.id == id::MX_NAME && text(e) == "Lead")
        .unwrap();
    assert!(lead < eq && eq < reverb);

    // Read back: Lead muted on insert 1 with its effects (Fruity Reeverb 2 as Lemon Reverb, at its defaults), and its
    // render from the start for its length.
    let back = crate::flp::import::import(&entries[0].data, files.factory)
        .unwrap()
        .song;
    let names: Vec<_> = back.channels.iter().map(|c| c.name.as_str()).collect();
    assert_eq!(names, ["MASTER", "Lead", "Lead (rendered)"]);
    let lead = &back.channels[1];
    let effects: Vec<_> = lead
        .rack
        .modules
        .iter()
        .skip(1)
        .map(|m| m.type_id)
        .collect();
    assert_eq!(
        (lead.params.0[2], effects),
        (1.0, vec![3, honeydew_audio::lemon::REVERB])
    );
    let audio = &back.channels[2];
    assert_eq!(
        (audio.kind, audio.params.0[2]),
        (ChannelKind::AudioClip, 0.0)
    );
    let clip = &audio.lanes[0].clips[0];
    let secs = decode(&stems[0].1).unwrap().frames() as f64 / f64::from(RATE);
    assert_eq!(clip.start_tick, 0);
    assert!(
        (clip.visible_len_beats - secs * 2.0).abs() < 0.01,
        "{}",
        clip.visible_len_beats
    );
}
