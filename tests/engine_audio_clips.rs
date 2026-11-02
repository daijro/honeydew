//! Audio clip lanes through the engine, driven
//! through `AudioRenderer` with samples the tests make. The `ramp` sample holds x[i] = i/2^20, so the output reads
//! back the play position.

use honeydew::audio_engine::{EngineError, SongAudioEngine, fit_audio_clip, source_beats};
use honeydew::flm::chunk::ContentPath;
use honeydew::sample_cache::SampleCache;
use honeydew::song::{ChannelKind, Clip, Event, LaneKind, SamplerSlot, Song, SynthKind};
use honeydew_audio::device::AudioRenderer;
use honeydew_audio::drums::level_gain;

const RATE: f32 = 44_100.0;
/// Frames per beat at 120 BPM.
const BEAT: usize = 22_050;
const SCALE: f32 = 1_048_576.0;

/// A 32-bit float mono WAV at 44.1 kHz.
fn wav(samples: &[f32]) -> Vec<u8> {
    let data: Vec<u8> = samples.iter().flat_map(|s| s.to_le_bytes()).collect();
    let mut bytes = b"RIFF".to_vec();
    bytes.extend((36 + data.len() as u32).to_le_bytes());
    bytes.extend(b"WAVEfmt ");
    bytes.extend(16u32.to_le_bytes());
    bytes.extend(3u16.to_le_bytes());
    bytes.extend(1u16.to_le_bytes());
    bytes.extend(44_100u32.to_le_bytes());
    bytes.extend((44_100u32 * 4).to_le_bytes());
    bytes.extend(4u16.to_le_bytes());
    bytes.extend(32u16.to_le_bytes());
    bytes.extend(b"data");
    bytes.extend((data.len() as u32).to_le_bytes());
    bytes.extend(data);
    bytes
}

/// The test samples by name: `ramp` (x[i] = i/2^20) and `dc` (0.5), 4 beats long, and a beat of 441 Hz `sine`.
fn read(path: &ContentPath) -> Option<Vec<u8>> {
    let ContentPath::Plain(name) = path else {
        return None;
    };
    let samples: Vec<f32> = match name.as_str() {
        "ramp" => (0..4 * BEAT).map(|i| i as f32 / SCALE).collect(),
        "dc" => vec![0.5; 4 * BEAT],
        "sine" => (0..BEAT)
            .map(|i| 0.5 * (std::f32::consts::TAU * 441.0 * i as f32 / RATE).sin())
            .collect(),
        _ => return None,
    };
    Some(wav(&samples))
}

/// A song at 120 BPM with an Audio Clip channel (channel 1).
fn song() -> Song {
    let mut song = Song {
        tempo_bpm: 120.0,
        ..Song::default()
    };
    song.add_channel(ChannelKind::AudioClip);
    song
}

/// A clip on channel 1's audio lane at `start_tick`, looping every `loop_beats`, playing `sample`.
fn clip<'a>(song: &'a mut Song, start_tick: u32, loop_beats: f64, sample: &str) -> &'a mut Clip {
    let mut slot = SamplerSlot::default();
    slot.file.path = Some(ContentPath::Plain(sample.into()));
    slot.file.id = 100 + song.channels[1].lanes[0].clips.len() as u32;
    let clip = song.channels[1].lanes[0].add_clip(start_tick, loop_beats);
    clip.file.sample = Some(Box::new(slot));
    clip
}

fn slot(clip: &mut Clip) -> &mut SamplerSlot {
    clip.file.sample.as_mut().unwrap()
}

fn engine(song: &Song) -> SongAudioEngine {
    let mut cache = SampleCache::default();
    cache.load(song, &read);
    SongAudioEngine::with_samples(song, RATE, &cache).unwrap()
}

/// The next `frames` frames, left and right.
fn render_lr(engine: &mut SongAudioEngine, frames: usize) -> (Vec<f32>, Vec<f32>) {
    let mut output = vec![0.0; frames * 2];
    engine.render(&mut output, 2);
    output.chunks(2).map(|frame| (frame[0], frame[1])).unzip()
}

fn render(engine: &mut SongAudioEngine, frames: usize) -> Vec<f32> {
    render_lr(engine, frames).0
}

/// `song` from its start for `frames` frames.
fn play(song: &Song, frames: usize) -> Vec<f32> {
    let mut engine = engine(song);
    engine.play();
    render(&mut engine, frames)
}

/// The output of a ramp sample at position 1, through the default slot and strips.
fn unit() -> f32 {
    static UNIT: std::sync::OnceLock<f32> = std::sync::OnceLock::new();
    *UNIT.get_or_init(|| {
        let mut song = song();
        clip(&mut song, 0, 4.0, "ramp");
        let out = play(&song, 2000);
        let unit = (out[1500] - out[500]) / 1000.0;
        assert!((unit * SCALE - 1.0).abs() < 0.01, "{}", unit * SCALE);
        unit
    })
}

/// The ramp position an output sample reads.
fn at(sample: f32) -> f32 {
    sample / unit()
}

fn close(a: f32, b: f32, tolerance: f32) -> bool {
    (a - b).abs() <= tolerance
}

fn assert_at(output: &[f32], frame: usize, position: f32) {
    let got = at(output[frame]);
    assert!(
        close(got, position, 0.05),
        "frame {frame}: at {got}, not {position}"
    );
}

#[test]
fn the_reference_walk_loads_every_clips_sample() {
    let mut song = song();
    clip(&mut song, 0, 4.0, "ramp").muted = true;
    clip(&mut song, 512, 4.0, "dc");
    let mut cache = SampleCache::default();
    cache.load(&song, &read);
    for clip in &song.channels[1].lanes[0].clips {
        let sample = cache.slot(clip.file.sample.as_ref().unwrap());
        assert_eq!(sample.map(|s| s.frames()), Some(4 * BEAT));
    }
}

#[test]
fn a_clip_starts_on_its_tick_from_the_sample_start() {
    let mut song = song();
    clip(&mut song, 128, 4.0, "ramp");
    let out = play(&song, 2 * BEAT);
    assert!(
        out[..BEAT].iter().all(|&s| s == 0.0),
        "silent before beat 1"
    );
    for k in [1, 2, 1000, 20_000] {
        assert_at(&out, BEAT + k, k as f32);
    }
}

#[test]
fn an_offset_clip_starts_offset_beats_into_its_sample_at_the_song_tempo() {
    // 0.5 beats at 120 BPM is 0.25 s, 11025 frames.
    let mut song = song();
    clip(&mut song, 128, 4.0, "ramp").offset_beats = 0.5;
    let out = play(&song, 2 * BEAT);
    assert_at(&out, BEAT, 11_025.0);
    assert_at(&out, BEAT + 100, 11_125.0);

    // Speed +12 steps twice as fast but does not scale the offset (a quirk); Sample Start adds to it.
    slot(&mut song.channels[1].lanes[0].clips[0]).params[2] = 0.75;
    let out = play(&song, 2 * BEAT);
    assert_at(&out, BEAT + 100, 11_225.0);
    let slot = slot(&mut song.channels[1].lanes[0].clips[0]);
    slot.params[2] = 0.5;
    // v = 0.6: u = 0.2, s = +0.02 s = 882 frames.
    slot.params[10] = 0.6;
    let out = play(&song, 2 * BEAT);
    assert_at(&out, BEAT + 100, 11_025.0 + 882.0 + 100.0);
}

#[test]
fn a_hostile_slot_param_keeps_its_default() {
    let mut song = song();
    let clip = clip(&mut song, 0, 4.0, "ramp");
    slot(clip).params[0] = f32::NAN;
    slot(clip).params[2] = 7.0;
    let out = play(&song, 1000);
    assert_at(&out, 500, 500.0);
}

#[test]
fn reverse_counts_the_offset_from_the_end() {
    let mut song = song();
    let clip = clip(&mut song, 0, 4.0, "ramp");
    clip.offset_beats = 0.5;
    slot(clip).file.reverse = true;
    let out = play(&song, 1000);
    let last = (4 * BEAT - 1) as f32;
    assert_at(&out, 10, last - 11_025.0 - 10.0);
}

#[test]
fn each_loop_repetition_fades_the_voice_and_relaunches_from_source_time_0() {
    let mut song = song();
    clip(&mut song, 0, 1.0, "ramp").visible_len_beats = 3.0;
    let out = play(&song, 3 * BEAT);
    assert_at(&out, BEAT - 1, (BEAT - 1) as f32);
    // The old voice fades at c = 0.01 per frame as the new one starts from 0.
    assert!(
        close(at(out[BEAT]), BEAT as f32 * 0.99, 1.0),
        "{}",
        at(out[BEAT])
    );
    for k in [1500, 20_000] {
        assert_at(&out, BEAT + k, k as f32);
        assert_at(&out, 2 * BEAT + k, k as f32);
    }

    // With an offset of 0.25 beats the repetitions fall at start − 0.25 + n beats: 0.75, 1.75 …
    song.channels[1].lanes[0].clips[0].offset_beats = 0.25;
    let out = play(&song, 3 * BEAT);
    let quarter = BEAT as f32 / 4.0;
    assert_at(&out, 1000, quarter + 1000.0);
    let repeat = 3 * BEAT / 4 + 1;
    assert_at(&out, repeat + 1500, 1500.0);
    assert_at(&out, repeat + BEAT + 1500, 1500.0);
}

#[test]
fn the_clip_end_fades_its_voices() {
    let mut song = song();
    clip(&mut song, 0, 4.0, "ramp").visible_len_beats = 1.0;
    let out = play(&song, 2 * BEAT);
    assert_at(&out, BEAT - 1, (BEAT - 1) as f32);
    assert!(close(at(out[BEAT]), BEAT as f32 * 0.99, 1.0));
    assert!(out[BEAT + 1000..].iter().all(|&s| s == 0.0), "ended");
}

#[test]
fn a_muted_clip_is_silent_and_never_relaunched() {
    let mut song = song();
    clip(&mut song, 0, 4.0, "ramp").muted = true;
    let mut engine = engine(&song);
    engine.play();
    assert!(render(&mut engine, BEAT).iter().all(|&s| s == 0.0));
    engine.seek(192);
    assert!(render(&mut engine, BEAT).iter().all(|&s| s == 0.0));
    song.channels[1].lanes[0].clips[0].muted = false;
    assert!(
        play(&song, BEAT).iter().any(|&s| s != 0.0),
        "unmuted, it plays"
    );
}

#[test]
fn a_seek_while_playing_cuts_and_relaunches_mid_sample() {
    // The clip starts at beat 1 and loops every beat from 0.25 beats in. Tick 320 is 1.5 beats into the song,
    // 0.5 into the clip: content beat 0.75, source time 0.375 s.
    let mut song = song();
    let clip = clip(&mut song, 128, 1.0, "ramp");
    clip.visible_len_beats = 4.0;
    clip.offset_beats = 0.25;
    let mut engine = engine(&song);
    engine.play();
    render(&mut engine, 5000);
    engine.seek(320);
    let out = render(&mut engine, 1000);
    // Cut instantly: no fading voice adds to the first frame.
    assert_at(&out, 0, 0.75 * BEAT as f32);
    assert_at(&out, 999, 0.75 * BEAT as f32 + 999.0);
    // 0.75 into the clip, c = 1.0 − L = 0.
    engine.seek(128 + 96);
    assert_at(&render(&mut engine, 100), 50, 50.0);
    // Tick 237 is no repetition: c = 0.85 + 0.25 − L ≈ 0.1. Its frame, 40827, is 2239.5 source frames into the pass.
    engine.seek(237);
    assert_at(
        &render(&mut engine, 100),
        0,
        40_827.0 - BEAT as f32 - 0.75 * BEAT as f32,
    );
    // A seek outside the clip leaves it silent; one onto its start starts it through its start hit.
    engine.seek(64);
    assert!(render(&mut engine, 100).iter().all(|&s| s == 0.0));
    engine.seek(128);
    assert_at(&render(&mut engine, 100), 50, 0.25 * BEAT as f32 + 50.0);
}

#[test]
fn a_seek_while_stopped_waits_for_play() {
    let mut song = song();
    clip(&mut song, 0, 4.0, "ramp");
    let mut engine = engine(&song);
    engine.seek(64);
    assert!(render(&mut engine, 1000).iter().all(|&s| s == 0.0));
    engine.play();
    let half = BEAT as f32 / 2.0;
    assert_at(&render(&mut engine, 100), 10, half + 10.0);
    // Stop fades the clip's voices; Play relaunches at the playhead.
    engine.stop();
    let out = render(&mut engine, 2000);
    assert!(
        close(at(out[0]), (half + 100.0) * 0.99, 1.0),
        "{}",
        at(out[0])
    );
    assert!(out[1500..].iter().all(|&s| s == 0.0));
    engine.play();
    assert_at(&render(&mut engine, 100), 10, half + 110.0);
}

#[test]
fn a_count_in_holds_the_clips_until_the_song_rolls() {
    let mut song = song();
    clip(&mut song, 0, 4.0, "ramp");
    let mut engine = engine(&song);
    engine.seek(64);
    engine.count_in(1);
    engine.play();
    let out = render(&mut engine, BEAT + 2000);
    // Only the count-in's click (its first 1000 frames) sounds during the held beat.
    assert!(
        out[1000..BEAT].iter().all(|&s| s == 0.0),
        "no clip during the count-in"
    );
    assert_at(&out, BEAT + 1500, BEAT as f32 / 2.0 + 1500.0);
}

#[test]
fn a_song_loop_wrap_relaunches_the_clip_at_the_loop_start() {
    // Loop [0.5, 1.5) beats over a 4-beat clip: after the wrap it plays from 0.5 beats in again.
    let mut song = song();
    song.play_loop = Some((64, 192));
    clip(&mut song, 0, 4.0, "ramp");
    let mut engine = engine(&song);
    engine.play();
    let out = render(&mut engine, BEAT * 3 / 2 + 1000);
    let end = BEAT * 3 / 2;
    assert_at(&out, end - 1, (end - 1) as f32);
    // The channel's first lane is audio: the old voice is cut, not faded.
    assert_at(&out, end, BEAT as f32 / 2.0);
    assert_at(&out, end + 999, BEAT as f32 / 2.0 + 999.0);
}

#[test]
fn a_channel_whose_first_lane_is_not_audio_gets_no_cut_at_a_wrap() {
    let mut song = Song {
        tempo_bpm: 120.0,
        play_loop: Some((64, 192)),
        ..Song::default()
    };
    song.add_channel(ChannelKind::Synth(SynthKind::MiniSynth));
    let lane = song.channels[1].add_lane(LaneKind::Audio);
    let mut slot = SamplerSlot::default();
    slot.file.path = Some(ContentPath::Plain("ramp".into()));
    let clip = song.channels[1].lanes[lane].add_clip(0, 4.0);
    clip.file.sample = Some(Box::new(slot));
    assert_ne!(song.channels[1].lanes[0].kind, LaneKind::Audio);
    let mut engine = engine(&song);
    engine.play();
    let out = render(&mut engine, BEAT * 3 / 2 + 10);
    // The old voice is released (the default EG R rings on) and the relaunched one adds to it.
    let end = BEAT * 3 / 2;
    let both = at(out[end + 5]);
    assert!(both > (end + BEAT / 2) as f32, "{both}");
}

/// Envelope `env_param`'s control points (content tick, value).
fn envelope(clip: &mut Clip, env_param: i16, points: &[(u32, f32)]) {
    clip.events
        .extend(points.iter().map(|&(pos, value)| Event::Envelope {
            pos,
            env_param,
            value,
            control: true,
        }));
}

#[test]
fn the_level_envelope_steps_like_automation() {
    let mut song = song();
    let clip = clip(&mut song, 0, 4.0, "dc");
    envelope(clip, 0, &[(0, 0.0), (256, 1.0)]);
    let out = play(&song, 3 * BEAT);
    let unity = 0.5 * unit() * SCALE / level_gain(0.8531);
    let level = |frame: usize| out[frame] / unity;
    // v = 0 plays at −60 dB, not silence; at beat 1 the step for level 64 (32766/65535) holds; at beat 2, +6 dB.
    assert!(close(level(10), level_gain(0.0), 1e-5), "{}", level(10));
    assert!(close(
        level(BEAT + 10),
        level_gain(32_766.0 / 65_535.0),
        1e-4
    ));
    assert!(close(level(2 * BEAT + 10), level_gain(1.0), 1e-3));
}

#[test]
fn pan_and_speed_envelopes_use_the_slot_laws_and_repeat_with_the_loop() {
    let mut song = song();
    let clip = clip(&mut song, 0, 1.0, "ramp");
    clip.visible_len_beats = 2.0;
    // Pan hard left from beat 0.5 of each pass; Speed +12 (0.76: 48v − 24 = 12.48, whole semitones).
    envelope(clip, 1, &[(0, 0.5), (63, 0.5), (64, 0.0)]);
    envelope(clip, 2, &[(0, 0.76)]);
    let mut engine = engine(&song);
    engine.play();
    let (left, right) = render_lr(&mut engine, 2 * BEAT);
    assert_at(&left, 100, 200.0);
    assert!(right[100] > 0.0, "centred");
    for pass in [0, BEAT] {
        let frame = pass + BEAT / 2 + 100;
        assert!(
            left[frame] > 0.0 && right[frame] == 0.0,
            "{pass}: hard left"
        );
        assert!(right[pass + 100] > 0.0, "{pass}: centred again");
    }
}

#[test]
fn a_seek_applies_each_envelopes_last_step_before_the_content_beat() {
    let mut song = song();
    let clip = clip(&mut song, 0, 1.0, "dc");
    clip.visible_len_beats = 4.0;
    envelope(clip, 0, &[(0, 0.2), (63, 0.2), (64, 0.9)]);
    let mut engine = engine(&song);
    engine.play();
    render(&mut engine, 100);
    // 2.75 beats in: content beat 0.75, after the step at 0.5.
    engine.seek(352);
    let out = render(&mut engine, 100);
    let unity = 0.5 * unit() * SCALE / level_gain(0.8531);
    assert!(
        close(out[10] / unity, level_gain(0.9), 1e-3),
        "{}",
        out[10] / unity
    );
}

/// The mean period in frames, from the rising zero crossings.
fn period(samples: &[f32]) -> f32 {
    let rising: Vec<usize> = (1..samples.len())
        .filter(|&i| samples[i - 1] < 0.0 && samples[i] >= 0.0)
        .collect();
    (rising[rising.len() - 1] - rising[0]) as f32 / (rising.len() - 1) as f32
}

#[test]
fn str1_stretch_and_pitch_are_rendered_once_as_on_pads() {
    // A beat of 441 Hz (period 100): (frames it sounds, period).
    let render = |mode: u8, ratio: f64, semitones: f64| {
        let mut song = song();
        let slot = slot(clip(&mut song, 0, 4.0, "sine"));
        (slot.file.stretch_mode, slot.file.stretch_ratio) = (mode, ratio);
        slot.file.pitch_semitones = semitones;
        let out = play(&song, 3 * BEAT);
        let sounding = out.iter().rposition(|s| s.abs() > 1e-3).unwrap();
        (sounding, period(&out[2000..BEAT - 2000]))
    };
    let (long, pitch) = render(1, 2.0, 0.0);
    assert!(
        long.abs_diff(2 * BEAT) < 1100 && close(pitch, 100.0, 0.5),
        "twice as long: {long} {pitch}"
    );
    let (native, pitch) = render(0, 2.0, 0.0);
    assert!(
        native.abs_diff(BEAT) < 10 && close(pitch, 100.0, 0.5),
        "SYNC off: {native} {pitch}"
    );
    let (same, pitch) = render(0, 1.0, 12.0);
    assert!(
        same.abs_diff(BEAT) < 1100 && close(pitch, 50.0, 0.5),
        "an octave up: {same} {pitch}"
    );
}

#[test]
fn block_size_does_not_change_the_output() {
    let mut song = song();
    let clip = clip(&mut song, 64, 1.0, "ramp");
    clip.visible_len_beats = 3.0;
    clip.offset_beats = 0.3;
    envelope(clip, 0, &[(0, 0.1), (100, 0.9)]);
    song.play_loop = Some((100, 400));
    let frames = 4 * BEAT;
    let whole = play(&song, frames);
    let mut engine = engine(&song);
    engine.play();
    let mut pieces = Vec::new();
    for &size in [1, 7, 333, 1023, 4096].iter().cycle() {
        if pieces.len() >= frames {
            break;
        }
        pieces.extend(render(&mut engine, size));
    }
    pieces.truncate(frames);
    assert!(whole.iter().filter(|&&s| s != 0.0).count() > 3 * BEAT);
    assert_eq!(whole, pieces);
}

#[test]
fn clhd_28_and_8_follow_the_rendered_sample_speed_and_tempo() {
    // S = BPM × frames / (rate × 60 × Speed ratio): 4 beats of ramp at 120 BPM.
    assert_eq!(source_beats(4 * BEAT, 44_100, 0.5, 120.0), 4.0);
    assert_eq!(
        source_beats(4 * BEAT, 44_100, 0.75, 120.0),
        2.0,
        "Speed +12"
    );
    assert_eq!(source_beats(4 * BEAT, 44_100, 0.5, 60.0), 2.0);

    // The app's order: edit the clip, load the cache (it renders a new STR1 once), then fit.
    let mut fixed = song();
    clip(&mut fixed, 0, 1.0, "ramp");
    let mut cache = SampleCache::default();
    let mut fit = |song: &mut Song, b: u32, edit: &dyn Fn(&mut Clip)| {
        edit(&mut song.channels[1].lanes[0].clips[0]);
        cache.load(song, &read);
        let c = &mut song.channels[1].lanes[0].clips[0];
        let changed = fit_audio_clip(c, &cache, 120.0, b);
        (changed, c.file.source_len_beats, c.loop_len_beats)
    };
    assert_eq!(fit(&mut fixed, 4, &|_| {}), (true, 4.0, 4.0));
    assert_eq!(fit(&mut fixed, 4, &|_| {}), (false, 4.0, 4.0), "unchanged");
    // SYNC on with ratio 2 renders twice the frames; the STR1 pitch keeps the length; SYNC off leaves the ratio unused.
    let sync = |c: &mut Clip| (slot(c).file.stretch_mode, slot(c).file.stretch_ratio) = (1, 2.0);
    assert_eq!(fit(&mut fixed, 4, &sync), (true, 8.0, 8.0));
    let pitch = |c: &mut Clip| slot(c).file.pitch_semitones = 12.0;
    assert_eq!(fit(&mut fixed, 4, &pitch), (false, 8.0, 8.0));
    assert_eq!(
        fit(&mut fixed, 4, &|c| slot(c).file.stretch_mode = 0),
        (true, 4.0, 4.0)
    );
    // L also holds the last envelope point (b·2^k), and never drops below 4 or b.
    let point = |c: &mut Clip| envelope(c, 0, &[(9 * 128, 0.5)]);
    assert_eq!(fit(&mut fixed, 3, &point), (true, 4.0, 12.0));
    let speed = |c: &mut Clip| {
        c.events.clear();
        slot(c).params[2] = 1.0;
    };
    assert_eq!(
        fit(&mut fixed, 4, &speed),
        (true, 1.0, 4.0),
        "Speed +24: S = 1, L stays 4"
    );

    // No sample loaded: nothing changes.
    let mut missing = song();
    clip(&mut missing, 0, 1.0, "missing");
    cache.load(&missing, &read);
    assert!(!fit_audio_clip(
        &mut missing.channels[1].lanes[0].clips[0],
        &cache,
        120.0,
        4
    ));
}

#[test]
fn the_play_budget_counts_each_pass_of_a_clips_envelopes() {
    // 30 000 one-beat passes of a 0 → 1 Level ramp: 129 steps, a fade and a relaunch each, past the budget (≈ 2.8 M).
    let mut song = song();
    let c = clip(&mut song, 0, 1.0, "ramp");
    c.visible_len_beats = 30_000.0;
    envelope(c, 0, &[(0, 0.0), (64, 1.0)]);
    let mut cache = SampleCache::default();
    cache.load(&song, &read);
    let built = SongAudioEngine::with_samples(&song, RATE, &cache);
    assert!(matches!(built, Err(EngineError::TooManyEvents)));
    // A clip shown for 10⁹ beats is refused before anything expands.
    let c = &mut song.channels[1].lanes[0].clips[0];
    (c.visible_len_beats, c.events) = (1e9, Vec::new());
    let built = SongAudioEngine::with_samples(&song, RATE, &cache);
    assert!(matches!(built, Err(EngineError::TooManyEvents)));
}

#[test]
fn channel_pitch_bend_does_not_reach_clip_slots() {
    // The channel bend reaches only drum pads, not audio-clip slots.
    let mut song = song();
    clip(&mut song, 0, 4.0, "ramp");
    let mut engine = engine(&song);
    engine.set_param(1, 0, 4, 1.0);
    engine.play();
    let out = render(&mut engine, 1000);
    assert_at(&out, 500, 500.0);
}

/// Clips an edit's engine takes over at another tempo without a re-sync (a Quiet handover at the
/// end of a tempo drag) are timed at its tempo: a seek relaunches the clip that covers the beat there.
#[test]
fn clips_taken_over_at_another_tempo_are_timed_at_it() {
    let mut song = song();
    // Beats 4 to 8.
    clip(&mut song, 4 * 128, 4.0, "dc").visible_len_beats = 4.0;
    let mut slower = song.clone();
    slower.tempo_bpm = 60.0;
    // One cache: the clips keep their very samples.
    let mut cache = SampleCache::default();
    cache.load(&song, &read);
    let build = |song: &Song| SongAudioEngine::with_samples(song, RATE, &cache).unwrap();
    let mut old = build(&song);
    old.play();
    render(&mut old, BEAT);
    let mut new = build(&slower).adopting_edit(false, Vec::new());
    new.take_over(&mut old);
    let mut heard = build(&slower);
    heard.play();
    // Beat 5, a beat into the clip: at 60 BPM past the clip's end at the old tempo's frames.
    for engine in [&mut new, &mut heard] {
        engine.seek(5 * 128);
    }
    let (actual, expected) = (render(&mut new, 1000), render(&mut heard, 1000));
    assert!(expected.iter().all(|&s| s > 0.1), "a silent reference");
    for (frame, (a, e)) in actual.iter().zip(&expected).enumerate() {
        assert!(close(*a, *e, 1e-6), "frame {frame}: {a} vs {e}");
    }
}
