//! Samples streamed from disk ([`honeydew_audio::stream`]): they play the frames a whole decode plays.

use std::path::PathBuf;
use std::sync::Arc;

use honeydew_audio::ProcessContext;
use honeydew_audio::dw_sampler::{DwSampler, DwZone};
use honeydew_audio::dwp::{Midi, Zone};
use honeydew_audio::flac_writer::flac;
use honeydew_audio::instrument::Instrument;
use honeydew_audio::sample::{SampleData, decode};
use honeydew_audio::sfz::parse;
use honeydew_audio::soundfont::{SoundFont, SoundFontPlayer, VOICES};
use honeydew_audio::stream::{Pcm, STREAM_VOICES, Source, Streamed, Streamer, head_frames};

const RATE: u32 = 48_000;

/// A stereo FLAC of `seconds`, written to a file of its own; every frame differs from its neighbours.
fn long_flac(name: &str, seconds: usize) -> PathBuf {
    let frames = seconds * RATE as usize;
    let tone = |hz: f64, i: usize| (0.4 * (hz * i as f64 / f64::from(RATE)).sin()) as f32;
    let left: Vec<f32> = (0..frames).map(|i| tone(2765.0, i)).collect();
    let right: Vec<f32> = (0..frames).map(|i| tone(1931.0, i)).collect();
    let dir = std::env::temp_dir().join(format!("honeydew-stream-{}", std::process::id()));
    std::fs::create_dir_all(&dir).unwrap();
    let path = dir.join(name);
    std::fs::write(&path, flac(RATE, 16, &[left, right])).unwrap();
    path
}

/// The file decoded whole, and streamed with heads at `starts`.
fn both(path: &PathBuf, starts: &[usize]) -> (Pcm, Pcm, Arc<Streamed>) {
    let whole = Arc::new(decode(&std::fs::read(path).unwrap()).unwrap());
    let source = Source::open(path).expect("a FLAC that counts its frames streams");
    assert_eq!(source.frames, whole.frames());
    let streamed = Arc::new(Streamed::new(source, starts).unwrap());
    (Pcm::Ram(whole), Pcm::Disk(streamed.clone()), streamed)
}

fn sfz_player(text: &str, pcm: Pcm) -> SoundFontPlayer {
    let regions = (parse(text, &|_| None).regions.into_iter())
        .map(|r| (r, pcm.clone()))
        .collect();
    SoundFontPlayer::new(
        RATE as f32,
        Some((Arc::new(SoundFont::sfz("Test", regions)), 0)),
    )
}

fn context() -> ProcessContext {
    ProcessContext {
        sample_rate: RATE as f32,
        tempo_bpm: 120.0,
        start_beat: 0.0,
        playing: true,
    }
}

/// `frames` of `player` from a note on `key`, in blocks of `block`, `between` run after each.
fn render(
    player: &mut dyn Instrument,
    key: u8,
    frames: usize,
    block: usize,
    mut between: impl FnMut(),
) -> Vec<f32> {
    player.note_on(key, 1.0, 0.5);
    let mut out = Vec::with_capacity(2 * frames);
    while out.len() < 2 * frames {
        let (mut left, mut right) = (vec![0.0; block], vec![0.0; block]);
        player.process(&mut left, &mut right, context());
        out.extend(left.into_iter().zip(right).flat_map(|(l, r)| [l, r]));
        between();
    }
    out
}

fn assert_same(streamed: &[f32], whole: &[f32]) {
    let off = streamed
        .iter()
        .zip(whole)
        .position(|(a, b)| (a - b).abs() > 1e-6);
    assert_eq!(
        off, None,
        "the streamed render left the whole one at sample {off:?}"
    );
    assert!(whole.iter().any(|s| s.abs() > 0.1), "silent");
}

const ONE_SHOT: &str = "<region> sample=x.flac pitch_keycenter=60 loop_mode=one_shot";

/// A three-second FLAC, all but its first half second streamed while it plays, renders the frames its whole decode
/// renders, and the streamer kept up: no underrun.
#[test]
fn a_streamed_flac_renders_the_frames_of_its_whole_decode() {
    let path = long_flac("long.flac", 3);
    let (ram, disk, streamed) = both(&path, &[0]);
    assert_eq!(
        streamed.byte_len(),
        8 * head_frames(RATE),
        "only the head is in memory"
    );
    let frames = 3 * RATE as usize - 1000;
    let whole = render(&mut sfz_player(ONE_SHOT, ram), 60, frames, 256, || {});
    let mut player = sfz_player(ONE_SHOT, disk);
    let rings = player.rings().expect("it streams").clone();
    let streamer = Streamer::spawn();
    streamer.register(&rings);
    // About 2.5× real time: the streamer polls every 2 ms.
    let pause = std::time::Duration::from_millis(2);
    let live = render(&mut player, 60, frames, 256, || std::thread::sleep(pause));
    assert_same(&live, &whole);
    assert_eq!(rings.underruns(), 0);
}

/// With its streamer stalled, a voice plays its head, then silence, and counts underruns; a render's rings (no
/// streamer) read every frame.
#[test]
fn a_stalled_streamer_underruns_and_an_offline_render_never_does() {
    let path = long_flac("stalled.flac", 2);
    let (ram, disk, _) = both(&path, &[0]);
    let frames = RATE as usize;
    let whole = render(&mut sfz_player(ONE_SHOT, ram), 60, frames, 512, || {});
    let mut player = sfz_player(ONE_SHOT, disk.clone());
    let rings = player.rings().unwrap().clone();
    let streamer = Streamer::spawn();
    streamer.stall(true);
    streamer.register(&rings);
    let stalled = render(&mut player, 60, frames, 512, || {});
    let head = 2 * (head_frames(RATE) - 512);
    assert_same(&stalled[..head], &whole[..head]);
    let after = 2 * (head_frames(RATE) + 512);
    assert!(stalled[after..].iter().all(|&s| s == 0.0), "past the head");
    assert!(rings.underruns() > 0);

    let mut offline = sfz_player(ONE_SHOT, disk);
    let rendered = render(&mut offline, 60, frames, 512, || {});
    assert_same(&rendered, &whole);
    assert_eq!(offline.rings().unwrap().underruns(), 0);
}

/// A DW zone looping over 1.5 s of a streamed FLAC loops as the decoded one does, wrap after wrap; reversed, it plays
/// from its end.
#[test]
fn a_streamed_loop_loops_and_a_reversed_zone_plays_from_its_end() {
    let path = long_flac("loop.flac", 3);
    let (start, end) = (RATE, 5 * RATE / 2);
    let zone = Zone {
        root: 60,
        loop_mode: 2,
        loop_start: start,
        loop_end: end,
        ..Zone::default()
    };
    let frames = 3 * RATE as usize;
    // The DW zone's heads: its start, its loop start and its end (for Reverse).
    let starts = [0, start as usize, frames - head_frames(RATE)];
    let (ram, disk, _) = both(&path, &starts);
    let sampler =
        |sample: Pcm| DwSampler::new(RATE as f32, vec![DwZone { zone, sample }], Midi::default());
    // Pitch 0 (p3 = 0.5) and an instant attack: the loop of 1.5 s wraps three times in 6 s.
    let ready = |mut dw: DwSampler, reverse: bool| {
        dw.set_parameter(1, 0.0);
        dw.set_parameter(8, f32::from(u8::from(reverse)));
        dw
    };
    let long = 6 * RATE as usize;
    let whole = render(
        &mut ready(sampler(ram.clone()), false),
        60,
        long,
        1000,
        || {},
    );
    let streamed = render(
        &mut ready(sampler(disk.clone()), false),
        60,
        long,
        1000,
        || {},
    );
    assert_same(&streamed, &whole);
    let whole = render(&mut ready(sampler(ram), true), 60, frames, 1000, || {});
    let streamed = render(&mut ready(sampler(disk), true), 60, frames, 1000, || {});
    assert_same(&streamed, &whole);
}

/// A player of [`VOICES`] voices that streams keeps rings for its first [`STREAM_VOICES`] alone: a streamed note past
/// them steals the oldest streamed voice, never an older one playing from memory.
#[test]
fn only_the_stream_voices_have_rings_and_a_streamed_note_steals_among_them() {
    let path = long_flac("pool.flac", 1);
    let (ram, disk, _) = both(&path, &[0]);
    // Soft notes stream; loud ones play from memory, looping.
    let text = "<region> sample=x.flac hivel=63 pitch_keycenter=60
        <region> sample=x.flac lovel=64 pitch_keycenter=60 loop_mode=loop_continuous";
    let regions = (parse(text, &|_| None).regions.into_iter())
        .zip([disk, ram])
        .collect();
    let font = Arc::new(SoundFont::sfz("Test", regions));
    let mut player = SoundFontPlayer::new(RATE as f32, Some((font, 0)));
    let rings = player.rings().expect("it streams").voices();
    assert!((rings, VOICES) == (STREAM_VOICES, 512) && rings < VOICES);
    // Every voice past the rings plays from memory, then every ring streams, then one more streamed note.
    for n in 0..VOICES - STREAM_VOICES {
        player.note_on(100 + (n % 28) as u8, 1.0, 0.5);
    }
    for key in 0..=STREAM_VOICES as u8 {
        player.note_on(key, 0.3, 0.5);
    }
    assert_eq!(player.active_voices(), VOICES);
    let block = |player: &mut SoundFontPlayer| {
        let (mut left, mut right) = ([0.0; 4800], [0.0; 4800]);
        player.process(&mut left, &mut right, context());
    };
    // Key 0's voice, the oldest streamed one, was stolen: its note-off releases nothing. Key 1's does.
    player.note_off(0);
    block(&mut player);
    assert_eq!(player.active_voices(), VOICES);
    player.note_off(1);
    block(&mut player);
    assert_eq!(player.active_voices(), VOICES - 1);
}

#[test]
fn a_whole_decode_and_a_stream_agree_on_a_wav_too() {
    let frames: Vec<f32> = (0..RATE as usize)
        .map(|i| ((i % 97) as f32 - 48.0) / 64.0)
        .collect();
    let bytes = honeydew_audio::wav_writer::float32_wav(RATE, &[frames.clone(), frames]);
    let path = std::env::temp_dir().join(format!("honeydew-stream-{}.wav", std::process::id()));
    std::fs::write(&path, &bytes).unwrap();
    let whole: SampleData = decode(&bytes).unwrap();
    let streamed = Streamed::new(Source::open(&path).unwrap(), &[0]).unwrap();
    assert_eq!(streamed.whole().unwrap(), whole);
}
