//! Our demo song through the app's own paths. It reads back with no notices and its metadata lock (META@776)
//! on, places drums, bass, keys and a lead as an intro, a main part, a break and an outro over 16 bars, and plays
//! through with every part sounding where its clips are and a peak in [−30, −1] dBFS (the templates' range).
//! `Restore demo songs`' writer puts it into `Demo songs` below the library's root, making the folder: on a desktop
//! folder and in the web's session library. The listen table prints with `--nocapture`; the WAV for listening is the
//! ignored `render_the_demo`.

use std::path::{Path, PathBuf};

use honeydew::audio_engine::SongAudioEngine;
use honeydew::content::{DEMO_FOLDER, demo_songs, restore_demo_songs};
use honeydew::flm::chunk::ContentPath;
use honeydew::flm::{read_with_notices, write};
use honeydew::sample_cache::SampleCache;
use honeydew::song::{ChannelKind, Event, PPQ, Song};
use honeydew::ui::main_menu::{MemoryLibrary, NativeLibrary, SongLibrary};
use honeydew_audio::device::AudioRenderer;

const RATE: f32 = 48_000.0;
/// Ticks a bar (4/4).
const BAR: u32 = 4 * PPQ;

/// Our demo: its file name and song, read as the app reads a `.flm`; it must have no notices.
fn demo() -> (String, Song) {
    let mut demos = demo_songs();
    assert_eq!(demos.len(), 1, "one demo song");
    let (name, bytes) = demos.remove(0);
    let (song, notices) = read_with_notices(&bytes).expect("the demo reads");
    assert_eq!(notices, Vec::<String>::new(), "{name}");
    (name, song)
}

/// What each channel plays: `Drums`, `Bass` (single notes below C3), `Keys` (chords) or `Lead` (single notes from C4
/// up); `None` for the master and the send.
fn role(c: &honeydew::song::Channel) -> Option<&'static str> {
    let notes: Vec<(u32, u8)> = (c.lanes.iter().flat_map(|l| &l.clips))
        .flat_map(|clip| &clip.events)
        .filter_map(|e| match *e {
            Event::Note { pos, key, .. } => Some((pos, key)),
            _ => None,
        })
        .collect();
    let chords =
        (notes.iter()).any(|&(at, _)| notes.iter().filter(|&&(p, _)| p == at).count() >= 3);
    match c.kind {
        ChannelKind::Drums => Some("Drums"),
        ChannelKind::Synth(_) if chords => Some("Keys"),
        ChannelKind::Synth(_) if notes.iter().all(|&(_, k)| k < 48) => Some("Bass"),
        ChannelKind::Synth(_) if notes.iter().all(|&(_, k)| k >= 60) => Some("Lead"),
        _ => None,
    }
}

/// The arrangement: the sections as (name, first bar, bars), and where each part plays, (first bar, bars shown).
const SECTIONS: [(&str, u32, u32); 4] = [
    ("Intro", 0, 4),
    ("Main", 4, 8),
    ("Break", 12, 2),
    ("Outro", 14, 2),
];
const PARTS: [(&str, &[(u32, u32)]); 4] = [
    ("Drums", &[(2, 2), (4, 8), (14, 2)]),
    ("Bass", &[(4, 8), (14, 2)]),
    ("Keys", &[(0, 4), (4, 8), (12, 2), (14, 2)]),
    ("Lead", &[(4, 8), (12, 2)]),
];

fn samples(song: &Song) -> SampleCache {
    let mut cache = SampleCache::default();
    cache.load(song, &|_: &ContentPath| None);
    cache
}

fn db(x: f32) -> f32 {
    20.0 * x.max(1e-9).log10()
}

/// `bars` bars of `song` from its start: the stereo output, and each channel's strip peak (instance 0) per 512-frame
/// block.
fn play_bars(song: &Song, bars: u32) -> (Vec<f32>, Vec<Vec<f32>>) {
    let mut engine = SongAudioEngine::with_samples(song, RATE, &samples(song)).unwrap();
    engine.play();
    let frames = (f64::from(4 * bars) * 60.0 / song.tempo_bpm * f64::from(RATE)) as usize;
    let mut out = Vec::with_capacity(frames * 2);
    let mut strips = vec![Vec::new(); song.channels.len()];
    let mut block = vec![0.0; 512 * 2];
    while out.len() < frames * 2 {
        let n = 512.min(frames - out.len() / 2);
        engine.render(&mut block[..n * 2], 2);
        out.extend_from_slice(&block[..n * 2]);
        let mut peaks = vec![0.0_f32; song.channels.len()];
        for i in 0..engine.meter_count() {
            let key = engine.meter_key(i);
            if key.instance_id == 0 {
                let m = engine.meter(i);
                peaks[key.channel] = m.peak_left.max(m.peak_right);
            }
        }
        strips.iter_mut().zip(peaks).for_each(|(s, p)| s.push(p));
    }
    (out, strips)
}

#[test]
fn the_demo_reads_back_with_no_notices_locked_titled_and_with_zero_guids() {
    let (name, song) = demo();
    assert_eq!(
        DEMO_FOLDER, "Demo songs",
        "the screen capture's title (plan U10)"
    );
    assert_eq!(name, format!("{}.flm", song.file.meta.display_title));
    assert!(!song.file.meta.display_title.is_empty());
    // As demos ship: the metadata lock on, time counting off, never saved.
    assert!(song.file.meta.locked, "META@776 = 1");
    assert!(song.file.meta.editing_seconds < 0.0);
    assert_eq!(song.file.meta.modified, "Never / demo project");
    // The lock survives a save (it is written on every save).
    let (again, _) = read_with_notices(&write(&song).unwrap()).unwrap();
    assert!(again.file.meta.locked);
    // The same bytes each time: GUIDs zero, as the templates'.
    assert_eq!(demo_songs(), demo_songs());
    assert_eq!(song.file.guid, [0; 16]);
    assert!(song.channels.iter().all(|c| c.file.guid == [0; 16]));
    // Play loops the 16 bars.
    assert_eq!(song.play_loop, Some((0, 16 * BAR)));
}

#[test]
fn the_demo_places_drums_bass_keys_and_a_lead_as_intro_main_break_and_outro() {
    let (_, song) = demo();
    let mut found = Vec::new();
    for c in &song.channels {
        let Some(role) = role(c) else { continue };
        let placed: Vec<(u32, u32)> = (c.lanes[0].clips.iter())
            .map(|clip| (clip.start_tick / BAR, clip.visible_ticks() as u32 / BAR))
            .collect();
        found.push((role, placed));
        // Every clip sits inside one section and has something to play.
        for clip in &c.lanes[0].clips {
            let (start, end) = (
                clip.start_tick,
                clip.start_tick + clip.visible_ticks() as u32,
            );
            let inside = (SECTIONS.iter())
                .any(|&(_, bar, bars)| bar * BAR <= start && end <= (bar + bars) * BAR);
            assert!(inside, "{role}: {start}..{end}");
            assert!(!clip.events.is_empty(), "{role}");
        }
    }
    let expected: Vec<(&str, Vec<(u32, u32)>)> =
        PARTS.iter().map(|&(r, p)| (r, p.to_vec())).collect();
    found.sort();
    let mut expected = expected;
    expected.sort();
    assert_eq!(found, expected);
    // 16 bars: the sections tile them.
    assert_eq!(SECTIONS.iter().map(|s| s.2).sum::<u32>(), 16);
    assert!(SECTIONS.windows(2).all(|w| w[0].1 + w[0].2 == w[1].1));
}

#[test]
fn the_demo_plays_through_with_every_part_sounding_in_its_sections_and_a_peak_in_range() {
    let (name, song) = demo();
    assert_eq!(samples(&song).missing().count(), 0);
    let (out, strips) = play_bars(&song, 16);
    assert!(out.iter().all(|s| s.is_finite()), "NaN or inf");
    let peak = db(out.iter().fold(0.0, |p, s| p.max(s.abs())));
    let rms = db((out.iter().map(|s| s * s).sum::<f32>() / out.len() as f32).sqrt());
    let block_of = |tick: u32| {
        let secs = f64::from(tick) / f64::from(PPQ) * 60.0 / song.tempo_bpm;
        (secs * f64::from(RATE) / 512.0) as usize
    };
    let mut failed = Vec::new();
    let mut table = Vec::new();
    for (c, strip) in song.channels.iter().zip(&strips) {
        let Some(role) = role(c) else { continue };
        for clip in &c.lanes[0].clips {
            let start = clip.start_tick;
            let end = start + clip.visible_ticks() as u32;
            let p = db(strip[block_of(start)..block_of(end).min(strip.len())]
                .iter()
                .fold(0.0_f32, |a, &b| a.max(b)));
            table.push(format!(
                "{role} bars {}+{} {p:.1}",
                start / BAR,
                (end - start) / BAR
            ));
            if p < -40.0 {
                failed.push(format!(
                    "{role}: silent in bars {}..{} ({p:.1} dBFS)",
                    start / BAR,
                    end / BAR
                ));
            }
        }
    }
    println!(
        "{name} | peak {peak:6.1} dBFS | RMS {rms:6.1} dBFS\n{}",
        table.join("\n")
    );
    if !(-30.0..=-1.0).contains(&peak) {
        failed.push(format!("peak {peak:.1} dBFS"));
    }
    assert!(failed.is_empty(), "{}", failed.join("\n"));
}

/// `Restore demo songs` into `lib`, whose root holds `folder` once it is done: the folder is made, the demo written,
/// a deleted copy written back, a song of the user's in the folder left alone, and an edited copy put into Recently
/// Deleted before the demo replaces it. The first run, which replaces nothing, leaves an edited copy as it is.
fn restores_into(lib: &mut dyn SongLibrary, folder: &Path) {
    let (name, bytes) = demo_songs().remove(0);
    assert!(!lib.exists(folder));
    restore_demo_songs(lib, true).unwrap();
    let root = lib.list(None).unwrap();
    assert!(
        root.iter()
            .any(|e| e.is_folder && e.name == DEMO_FOLDER && e.path == folder),
        "{root:?}"
    );
    let file = folder.join(&name);
    assert_eq!(lib.read(&file).unwrap(), bytes);
    // Again, over a deleted copy, beside a song of the user's.
    lib.remove(&file).unwrap();
    lib.store_in(folder, "Mine.flm", b"mine").unwrap();
    restore_demo_songs(lib, true).unwrap();
    assert_eq!(lib.read(&file).unwrap(), bytes);
    assert_eq!(lib.read(&folder.join("Mine.flm")).unwrap(), b"mine");
    let listed = lib.list(Some(folder)).unwrap();
    assert_eq!(listed.len(), 2, "{listed:?}");
    // The same bytes are no change: nothing goes to the bin.
    let binned = lib.bin_path(&file).expect("in the user folder");
    restore_demo_songs(lib, true).unwrap();
    assert!(!lib.exists(&binned));
    // An edited copy: the first run keeps it; a restore bins it, then writes the demo.
    lib.store_in(folder, &name, b"my edit").unwrap();
    restore_demo_songs(lib, false).unwrap();
    assert_eq!(lib.read(&file).unwrap(), b"my edit");
    assert!(!lib.exists(&binned));
    restore_demo_songs(lib, true).unwrap();
    assert_eq!(lib.read(&file).unwrap(), bytes);
    assert_eq!(lib.read(&binned).unwrap(), b"my edit");
}

#[test]
fn restore_makes_demo_songs_in_the_web_session_library() {
    restores_into(&mut MemoryLibrary::default(), Path::new(DEMO_FOLDER));
}

#[test]
fn restore_makes_demo_songs_in_a_desktop_songs_folder() {
    let user =
        Path::new(env!("CARGO_TARGET_TMPDIR")).join(format!("demo-songs-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&user);
    let root = user.join("My Songs");
    restores_into(
        &mut NativeLibrary::at(root.clone()),
        &root.join(DEMO_FOLDER),
    );
    assert!(root.join(DEMO_FOLDER).is_dir());
    std::fs::remove_dir_all(&user).unwrap();
}

/// The render harness, not a check: one pass of the demo as a 48 kHz float WAV, `$DEMO_RENDER_DIR/<name>.wav`. Run
/// with `DEMO_RENDER_DIR=<dir> cargo test --test content_demo render_the_demo -- --ignored`.
#[test]
#[ignore]
fn render_the_demo() {
    let dir = PathBuf::from(std::env::var("DEMO_RENDER_DIR").expect("set DEMO_RENDER_DIR"));
    let (name, song) = demo();
    let (out, _) = play_bars(&song, 16);
    let (left, right): (Vec<f32>, Vec<f32>) = out.chunks(2).map(|f| (f[0], f[1])).unzip();
    let wav = honeydew_audio::wav_writer::float32_wav(RATE as u32, &[left, right]);
    std::fs::create_dir_all(&dir).unwrap();
    std::fs::write(dir.join(Path::new(&name).with_extension("wav")), wav).unwrap();
}
