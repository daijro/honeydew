//! Our preset banks through the engine: every preset of ours, read from the factory and loaded as a pick
//! loads it, holds C3 for 1 s through `SongAudioEngine` + `AudioRenderer`, then rings out for 0.5 s. No device.
//!
//! C3 is MIDI key 48 (the `Set middle C to C4` naming), a register every bank plays: bass, keys, lead, pad.
//!
//! Besides the basic checks (no NaN, a peak in [−30, 0) dBFS) each render gets listen proxies, printed as a table
//! (`--nocapture`): its RMS frequency (a spectral-centroid proxy, from the RMS of the first difference), how much of
//! its loudest level the held note keeps at 1 s, and what is left 0.3 s after the release. No two presets of a bank
//! share all three, a pad holds, and a pluck dies away.
//!
//! Levels: a peak is max(|L|, |R|). A single C3 peaks at −18 dBFS or more, so no preset is lost in a mix
//! (Transistor Bass's own chain is quiet: its presets make it up on the DISTORTION page), and a C minor triad (C3, Eb3,
//! G3) peaks at −3 dBFS or less, so a chord never clips. A C major triad, a C minor seventh and a C major
//! seventh peak at −2 dBFS or less too: their peaks fall elsewhere, and four notes stack higher than three.

use std::path::{Path, PathBuf};

use honeydew::audio_engine::SongAudioEngine;
use honeydew::content::{PRESETS, ROOT, factory};
use honeydew::flm::read_preset;
use honeydew::project_files::preset_folder;
use honeydew::song::{ChannelKind, Song, SynthKind};
use honeydew::ui::main_menu::SongLibrary;
use honeydew_audio::device::AudioRenderer;

const RATE: usize = 48_000;
const C3: u8 = 48;
const TRIAD: [u8; 3] = [C3, C3 + 3, C3 + 7];
/// C major, C minor seventh and C major seventh: each ≤ −2 dBFS.
const MAJOR: [u8; 3] = [C3, C3 + 4, C3 + 7];
const MINOR_SEVENTH: [u8; 4] = [C3, C3 + 3, C3 + 7, C3 + 10];
const MAJOR_SEVENTH: [u8; 4] = [C3, C3 + 4, C3 + 7, C3 + 11];
/// The keyboard's velocity, 100.
const VELOCITY: f32 = 100.0 / 127.0;

/// Every preset file below `folder` in the factory, in index order.
fn presets(folder: &Path) -> Vec<PathBuf> {
    let mut files = Vec::new();
    for entry in factory().list(Some(folder)).unwrap() {
        match entry.is_folder {
            true => files.extend(presets(&entry.path)),
            false => files.push(entry.path),
        }
    }
    files
}

/// `kind`'s bank: each preset's path, its category (`""` at the module folder's root) and its name.
fn bank(kind: SynthKind) -> Vec<(PathBuf, String, String)> {
    let folder = Path::new(ROOT)
        .join(PRESETS)
        .join(preset_folder(kind.type_id()).unwrap());
    let rows = presets(&folder).into_iter().map(|path| {
        let below = path.strip_prefix(&folder).unwrap();
        let category = below.parent().unwrap().to_string_lossy().into_owned();
        let name = below.file_stem().unwrap().to_string_lossy().into_owned();
        (path, category, name)
    });
    rows.collect()
}

/// The preset at `path` on a `kind` channel: `keys` held 1 s, then 0.5 s after their release; stereo, interleaved.
fn play(kind: SynthKind, path: &Path, keys: &[u8]) -> Vec<f32> {
    let bytes = factory().read(path).unwrap();
    let mut song = Song {
        tempo_bpm: 120.0,
        ..Song::default()
    };
    song.add_channel(ChannelKind::Synth(kind));
    let preset = read_preset(&bytes, Some(kind.type_id())).unwrap();
    song.channels[1].rack.modules[0].load_preset(preset);
    let mut engine = SongAudioEngine::new(&song, RATE as f32).unwrap();
    keys.iter()
        .for_each(|&key| engine.note_on(1, key, VELOCITY));
    let mut held = vec![0.0; RATE * 2];
    engine.render(&mut held, 2);
    keys.iter().for_each(|&key| engine.note_off(1, key));
    let mut tail = vec![0.0; RATE];
    engine.render(&mut tail, 2);
    held.extend(tail);
    assert!(held.iter().all(|s| s.is_finite()), "{path:?}: NaN or inf");
    held
}

/// The peak of `stereo`, max(|L|, |R|), dBFS.
fn peak_db(stereo: &[f32]) -> f32 {
    db(stereo.iter().fold(0.0, |p, s| p.max(s.abs())))
}

fn rms(x: &[f32]) -> f32 {
    (x.iter().map(|s| s * s).sum::<f32>() / x.len() as f32).sqrt()
}

fn db(x: f32) -> f32 {
    20.0 * x.max(1e-9).log10()
}

/// What a render sounds like, roughly.
#[derive(Debug)]
struct Heard {
    peak_db: f32,
    /// The RMS frequency of the held note (0.1–0.6 s), Hz: a sine's own frequency, higher for brighter sounds.
    hz: f32,
    /// The last 50 ms of the held note against its loudest 50 ms, dB: 0 holds, far below 0 died away.
    held_db: f32,
    /// 0.3 s after the release against the loudest 50 ms, dB.
    tail_db: f32,
}

/// The proxies of a `stereo` render: its peak, and the rest of its mono sum, (L + R) / 2.
fn heard(stereo: &[f32]) -> Heard {
    let mono: Vec<f32> = stereo.chunks(2).map(|f| 0.5 * (f[0] + f[1])).collect();
    let window = RATE / 20;
    let level = |at: usize| rms(&mono[at..at + window]);
    let loudest = (0..RATE - window)
        .step_by(window / 4)
        .map(level)
        .fold(0.0, f32::max);
    let held = &mono[RATE / 10..RATE * 6 / 10];
    let diff: Vec<f32> = held.windows(2).map(|w| w[1] - w[0]).collect();
    let ratio = (rms(&diff) / rms(held).max(1e-9) / 2.0).min(1.0);
    Heard {
        peak_db: peak_db(stereo),
        hz: RATE as f32 / std::f32::consts::PI * ratio.asin(),
        held_db: db(level(RATE - window)) - db(loudest),
        tail_db: db(level(RATE * 13 / 10)) - db(loudest),
    }
}

/// Renders `kind`'s bank, prints what each preset sounds like, and checks it; every failure is listed at the end.
fn check_bank(kind: SynthKind, count: usize) {
    let bank = bank(kind);
    assert_eq!(bank.len(), count, "{kind:?}");
    let mut rows: Vec<(String, String, Heard)> = Vec::new();
    let mut failed: Vec<String> = Vec::new();
    let mut check = |ok: bool, why: String| {
        if !ok {
            failed.push(why);
        }
    };
    for (path, category, name) in bank {
        let h = heard(&play(kind, &path, &[C3]));
        let triad = peak_db(&play(kind, &path, &TRIAD));
        let major = peak_db(&play(kind, &path, &MAJOR));
        let seventh = peak_db(&play(kind, &path, &MINOR_SEVENTH));
        let major_seventh = peak_db(&play(kind, &path, &MAJOR_SEVENTH));
        println!(
            "{kind:?} | {category:5} | {name:16} | peak {:6.1} dBFS | triad {:6.1} | major {:6.1} | m7 {:6.1} | maj7 {:6.1} dBFS | {:6.0} Hz | held {:6.1} dB | tail {:6.1} dB",
            h.peak_db, triad, major, seventh, major_seventh, h.hz, h.held_db, h.tail_db
        );
        check(
            (-18.0..0.0).contains(&h.peak_db),
            format!("{name}: peak {h:?}"),
        );
        check(
            triad <= -3.0,
            format!("{name}: a triad peaks at {triad} dBFS"),
        );
        check(
            major <= -2.0 && seventh <= -2.0 && major_seventh <= -2.0,
            format!(
                "{name}: a major triad peaks at {major}, a minor seventh at {seventh}, a major seventh at \
                 {major_seventh} dBFS"
            ),
        );
        match category.as_str() {
            "Pad" => check(
                h.held_db > -6.0 && h.tail_db > -30.0,
                format!("{name}: a pad holds and rings, {h:?}"),
            ),
            "Pluck" => check(
                h.held_db < -12.0,
                format!("{name}: a pluck dies away, {h:?}"),
            ),
            _ => {}
        }
        rows.push((category, name, h));
    }
    // No two presets of a bank sound alike by brightness and envelope, whatever their levels.
    for (i, (_, a, ha)) in rows.iter().enumerate() {
        for (_, b, hb) in &rows[..i] {
            let alike = (ha.hz / hb.hz).ln().abs() < 0.05
                && (ha.held_db - hb.held_db).abs() < 3.0
                && (ha.tail_db - hb.tail_db).abs() < 3.0;
            check(!alike, format!("{a} and {b} sound alike: {ha:?} {hb:?}"));
        }
    }
    assert!(failed.is_empty(), "{kind:?}:\n{}", failed.join("\n"));
}

#[test]
fn every_minisynth_preset_plays_c3_with_a_sound_of_its_own() {
    check_bank(SynthKind::MiniSynth, 12);
}

#[test]
fn every_supersaw_preset_plays_c3_with_a_sound_of_its_own() {
    check_bank(SynthKind::SuperSaw, 11);
}

#[test]
fn every_transistor_bass_preset_plays_c3_with_a_sound_of_its_own() {
    check_bank(SynthKind::TransistorBass, 8);
}

#[test]
fn every_gmsynth_preset_plays_c3_with_a_sound_of_its_own() {
    check_bank(SynthKind::GmSynth, 14);
}

#[test]
fn every_three_osc_preset_plays_c3_with_a_sound_of_its_own() {
    check_bank(SynthKind::ThreeXOsc, 10);
}
