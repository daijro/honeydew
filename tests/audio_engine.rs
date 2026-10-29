use honeydew::audio_engine::{EngineError, SongAudioEngine};
use honeydew::flm::chunk::{ContentPath, ContentRoot};
use honeydew::sample_cache::{Reader, SampleCache};
use honeydew::song::{AutomationTarget, ChannelKind, Event, Song, SynthKind};
use honeydew_audio::device::AudioRenderer;

fn song_with_note() -> Song {
    let mut song = Song {
        tempo_bpm: 120.0,
        ..Song::default()
    };
    song.add_channel(ChannelKind::Synth(SynthKind::MiniSynth));
    song.channels[1].lanes[0]
        .add_clip(0, 2.0)
        .events
        .push(Event::Note {
            pos: 0,
            len_beats: 1.0,
            key: 69,
            velocity: 204,
            pan: 127,
            slide: false,
        });
    song
}

fn energy(samples: &[f32]) -> f32 {
    samples.iter().map(|sample| sample * sample).sum()
}

#[test]
fn scheduled_note_reaches_master_and_advances_transport() {
    let song = song_with_note();
    let mut engine = SongAudioEngine::new(&song, 48_000.0).unwrap();
    engine.play();
    let mut output = vec![0.0; 4096 * 2];
    engine.render(&mut output, 2);
    assert!(energy(&output) > 0.01);
    assert!(engine.transport_tick() > 0);
    assert!(engine.meter_count() >= 3);
    assert!(engine.meter(0).peak_left > 0.0);
}

#[test]
fn bus_effect_processes_summed_synth_audio() {
    let mut song = Song {
        tempo_bpm: 120.0,
        ..Song::default()
    };
    song.add_channel(ChannelKind::EffectTrack);
    song.add_channel(ChannelKind::Synth(SynthKind::MiniSynth));
    song.channels[2].file.sends[0].target_channel_id = song.channels[1].id;
    song.channels[2].lanes[0]
        .add_clip(0, 2.0)
        .events
        .push(Event::Note {
            pos: 0,
            len_beats: 1.0,
            key: 69,
            velocity: 204,
            pan: 127,
            slide: false,
        });
    let mut dry = SongAudioEngine::new(&song, 48_000.0).unwrap();
    let effect_id = song.channels[1].rack.add(20);
    song.channels[1].rack.get_mut(effect_id).unwrap().params[1] = 0.0;
    let mut wet = SongAudioEngine::new(&song, 48_000.0).unwrap();
    dry.play();
    wet.play();
    let mut dry_output = vec![0.0; 8192 * 2];
    let mut wet_output = vec![0.0; 8192 * 2];
    dry.render(&mut dry_output, 2);
    wet.render(&mut wet_output, 2);
    assert!(energy(&dry_output) > 0.01);
    assert!(energy(&wet_output) < energy(&dry_output) * 0.5);
    assert_eq!(wet.meter_key(2).instance_id, effect_id);
}

#[test]
fn keyboard_preview_and_transport_commands_work() {
    let mut song = Song::default();
    song.add_channel(ChannelKind::Synth(SynthKind::MiniSynth));
    let mut engine = SongAudioEngine::new(&song, 48_000.0).unwrap();
    engine.note_on(1, 69, 1.0);
    let mut output = vec![0.0; 1024 * 2];
    engine.render(&mut output, 2);
    assert!(energy(&output) > 0.01);
    assert_eq!(engine.transport_tick(), 0);
    engine.set_param(1, 0, 2, 1.0);
    engine.render(&mut output, 2);
    assert_eq!(energy(&output), 0.0);
    engine.set_param(1, 0, 2, 0.0);
    engine.note_off(1, 69);
    engine.play();
    engine.render(&mut output, 2);
    assert!(engine.transport_tick() > 0);
    engine.stop();
    let tick = engine.transport_tick();
    engine.render(&mut output, 2);
    assert_eq!(engine.transport_tick(), tick);
    engine.seek(128);
    assert_eq!(engine.transport_tick(), 128);
}

#[test]
fn seek_applies_automation_at_the_seek_frame_and_restores_earlier_values() {
    let mut song = Song::default();
    song.add_channel(ChannelKind::Synth(SynthKind::MiniSynth));
    let lane = song.channels[1].add_automation_lane(AutomationTarget {
        instance_id: 0,
        param: 0,
    });
    lane.add_clip(0, 1.0).events.extend([
        Event::Automation {
            pos: 0,
            value: 0.0,
            control: true,
        },
        Event::Automation {
            pos: 64,
            value: 1.0,
            control: true,
        },
    ]);
    let mut engine = SongAudioEngine::new(&song, 48_000.0).unwrap();
    let mut output = vec![0.0; 1024 * 2];
    engine.note_on(1, 69, 1.0);
    engine.render(&mut output, 2);
    assert_eq!(energy(&output), 0.0);
    // A seek while stopped only moves the position; Play re-syncs the params there.
    engine.seek(64);
    engine.play();
    engine.render(&mut output, 2);
    let loud = energy(&output);
    assert!(loud > 0.01);
    // While playing, a seek re-syncs them at once: back to the start of the ramp up.
    engine.seek(0);
    engine.render(&mut output, 2);
    assert!(
        energy(&output) < loud * 0.01,
        "{} vs {loud}",
        energy(&output)
    );
}

#[test]
fn note_effect_preserves_note_pan_through_synth() {
    let mut song = song_with_note();
    if let Event::Note { pan, .. } = &mut song.channels[1].lanes[0].clips[0].events[0] {
        *pan = 255;
    }
    // Above the synth: a note effect acts only on the instruments below it.
    song.channels[1].rack.insert(0, 48);
    let mut engine = SongAudioEngine::new(&song, 48_000.0).unwrap();
    engine.play();
    let mut output = vec![0.0; 4096 * 2];
    engine.render(&mut output, 2);
    let left = energy(&output.iter().step_by(2).copied().collect::<Vec<_>>());
    let right = energy(
        &output
            .iter()
            .skip(1)
            .step_by(2)
            .copied()
            .collect::<Vec<_>>(),
    );
    assert!(right > left * 5.0);
}

#[test]
fn saved_mute_applies_before_any_command() {
    let mut song = song_with_note();
    song.channels[1].params.0[2] = 1.0;
    let mut engine = SongAudioEngine::new(&song, 48_000.0).unwrap();
    engine.play();
    let mut output = vec![0.0; 4096 * 2];
    engine.render(&mut output, 2);
    assert_eq!(energy(&output), 0.0);
}

/// A seek while stopped only moves the position, and one while
/// playing releases only the keys the song held: a keyboard voice sounds on through both.
#[test]
fn a_seek_leaves_a_keyboard_voice_sounding() {
    let mut song = Song::default();
    song.add_channel(ChannelKind::Synth(SynthKind::MiniSynth));
    let mut engine = SongAudioEngine::new(&song, 48_000.0).unwrap();
    engine.note_on(1, 69, 1.0);
    let mut output = vec![0.0; 1024 * 2];
    engine.render(&mut output, 2);
    assert!(energy(&output) > 0.01);
    engine.seek(128);
    engine.render(&mut output, 2);
    assert!(energy(&output) > 0.01);
    engine.play();
    engine.seek(512);
    engine.render(&mut output, 2);
    assert!(energy(&output) > 0.01);
}

#[test]
fn very_quiet_saved_note_remains_playable() {
    let mut song = song_with_note();
    if let Event::Note { velocity, .. } = &mut song.channels[1].lanes[0].clips[0].events[0] {
        *velocity = 1;
    }
    let mut engine = SongAudioEngine::new(&song, 48_000.0).unwrap();
    engine.play();
    let mut output = vec![0.0; 4096 * 2];
    engine.render(&mut output, 2);
    assert!(energy(&output) > 0.0);
}

#[test]
fn seek_keeps_live_channel_controls_without_automation() {
    let mut song = Song::default();
    song.add_channel(ChannelKind::Synth(SynthKind::MiniSynth));
    let mut engine = SongAudioEngine::new(&song, 48_000.0).unwrap();
    engine.set_param(1, 0, 2, 1.0);
    engine.seek(128);
    engine.note_on(1, 69, 1.0);
    let mut output = vec![0.0; 1024 * 2];
    engine.render(&mut output, 2);
    assert_eq!(energy(&output), 0.0);
}

/// A MiniSynth channel (channel 1) holding one key from the keyboard, rendered `frames` frames, stopped.
fn held_note(song: &Song, frames: usize) -> Vec<f32> {
    let mut engine = SongAudioEngine::new(song, 48_000.0).unwrap();
    engine.note_on(1, 69, 1.0);
    let mut output = vec![0.0; frames * 2];
    engine.render(&mut output, 2);
    output
}

fn minisynth_song() -> Song {
    let mut song = Song::default();
    song.add_channel(ChannelKind::Synth(SynthKind::MiniSynth));
    song
}

fn side(output: &[f32], right: bool) -> Vec<f32> {
    output
        .iter()
        .skip(usize::from(right))
        .step_by(2)
        .copied()
        .collect()
}

#[test]
fn two_instruments_in_one_rack_both_sound_and_sum() {
    let one = held_note(&minisynth_song(), 4096);
    let mut song = minisynth_song();
    song.channels[1].rack.add(0);
    let two = held_note(&song, 4096);
    assert!(energy(&one) > 0.01);
    for (a, b) in one.iter().zip(&two) {
        assert!((2.0 * a - b).abs() <= 1e-6, "{a} {b}");
    }
}

#[test]
fn a_synth_channel_without_its_module_is_silent() {
    let mut song = minisynth_song();
    song.channels[1].rack.modules.clear();
    assert_eq!(energy(&held_note(&song, 4096)), 0.0);
}

#[test]
fn enable_off_silences_only_that_instrument() {
    let one = held_note(&minisynth_song(), 4096);
    let mut song = minisynth_song();
    let second = song.channels[1].rack.add(0);
    song.channels[1].rack.get_mut(second).unwrap().params[0] = 0.0;
    assert_eq!(held_note(&song, 4096), one);
    song.channels[1].rack.get_mut(2).unwrap().params[0] = 0.0;
    assert_eq!(energy(&held_note(&song, 4096)), 0.0);

    let mut engine = SongAudioEngine::new(&minisynth_song(), 48_000.0).unwrap();
    engine.set_param(1, 2, 0, 0.0);
    engine.note_on(1, 69, 1.0);
    let mut output = vec![0.0; 1024 * 2];
    engine.render(&mut output, 2);
    assert_eq!(energy(&output), 0.0);
}

#[test]
fn instrument_common_params_post_gain_mix_and_wet_pan() {
    let plain = held_note(&minisynth_song(), 8192);
    let mut song = minisynth_song();
    song.channels[1].rack.get_mut(2).unwrap().common.0[0] = 0.0;
    let quiet = held_note(&song, 8192);
    // Post Gain v = 0 is −20 dB once the gain smoother has settled.
    for (a, b) in plain[8192..].iter().zip(&quiet[8192..]) {
        assert!((a * 0.1 - b).abs() <= 1e-6, "{a} {b}");
    }

    let mut song = minisynth_song();
    song.channels[1].rack.get_mut(2).unwrap().common.0[1] = 0.0;
    assert_eq!(energy(&held_note(&song, 4096)), 0.0);
    let mut engine = SongAudioEngine::new(&minisynth_song(), 48_000.0).unwrap();
    engine.set_param(1, 2, AutomationTarget::MIX, 0.0);
    engine.note_on(1, 69, 1.0);
    let mut output = vec![0.0; 1024 * 2];
    engine.render(&mut output, 2);
    assert_eq!(energy(&output), 0.0);

    let mut song = minisynth_song();
    song.channels[1].rack.get_mut(2).unwrap().common.0[2] = 0.0;
    let panned = held_note(&song, 32_768);
    let tail = &panned[2 * 28_672..];
    assert!(energy(&side(tail, false)) > 0.01);
    assert!(energy(&side(tail, true)) < energy(&side(tail, false)) * 1e-6);
}

#[test]
fn instrument_common_param_automation_plays() {
    let mut song = minisynth_song();
    song.tempo_bpm = 120.0;
    let lane = song.channels[1].add_automation_lane(AutomationTarget {
        instance_id: 2,
        param: AutomationTarget::MIX,
    });
    lane.add_clip(0, 4.0).events.extend([
        Event::Automation {
            pos: 0,
            value: 0.0,
            control: true,
        },
        // Points ramp: hold 0 until tick 120, then rise to 1 at tick 128.
        Event::Automation {
            pos: 120,
            value: 0.0,
            control: true,
        },
        Event::Automation {
            pos: 128,
            value: 1.0,
            control: true,
        },
    ]);
    let mut engine = SongAudioEngine::new(&song, 48_000.0).unwrap();
    engine.note_on(1, 69, 1.0);
    engine.play();
    // One beat at 120 BPM is 24 000 frames.
    let mut before = vec![0.0; 20_000 * 2];
    engine.render(&mut before, 2);
    assert_eq!(energy(&before), 0.0);
    let mut after = vec![0.0; 16_000 * 2];
    engine.render(&mut after, 2);
    assert!(energy(&after[8000 * 2..]) > 0.01);
}

#[test]
fn compressor_gain_reduction_reads_zero_while_disabled() {
    let mut song = minisynth_song();
    let compressor = song.channels[1].rack.add(8);
    let params = &mut song.channels[1].rack.get_mut(compressor).unwrap().params;
    params[1] = 0.0;
    params[2] = 1.0;
    let mut engine = SongAudioEngine::new(&song, 48_000.0).unwrap();
    let meter = (0..engine.meter_count())
        .find(|&i| {
            engine.meter_key(i).channel == 1 && engine.meter_key(i).instance_id == compressor
        })
        .unwrap();
    engine.note_on(1, 69, 1.0);
    let mut output = vec![0.0; 4096 * 2];
    engine.render(&mut output, 2);
    assert!(engine.meter(meter).gain_reduction_db > 1.0);
    engine.set_param(1, compressor, 0, 0.0);
    engine.render(&mut output, 2);
    assert_eq!(engine.meter(meter).gain_reduction_db, 0.0);
}

#[test]
fn every_instrument_has_its_own_meter() {
    let mut song = minisynth_song();
    let second = song.channels[1].rack.add(0);
    let effect = song.channels[1].rack.add(20);
    song.channels[1].rack.get_mut(second).unwrap().params[0] = 0.0;
    let mut engine = SongAudioEngine::new(&song, 48_000.0).unwrap();
    let keys: Vec<_> = (0..engine.meter_count())
        .map(|i| engine.meter_key(i))
        .filter(|key| key.channel == 1)
        .map(|key| key.instance_id)
        .collect();
    assert_eq!(keys, [0, 2, second, effect]);
    engine.note_on(1, 69, 1.0);
    let mut output = vec![0.0; 4096 * 2];
    engine.render(&mut output, 2);
    let meter = |id| {
        let index = (0..engine.meter_count())
            .find(|&i| engine.meter_key(i).channel == 1 && engine.meter_key(i).instance_id == id)
            .unwrap();
        engine.meter(index)
    };
    assert!(meter(2).peak_left > 0.01);
    assert_eq!(meter(second).peak_left, 0.0);
}

#[test]
fn an_instrument_without_an_engine_is_silent() {
    let mut song = Song::default();
    // A Slicer with no loop loaded makes no sound.
    song.add_channel(ChannelKind::Synth(SynthKind::Slicer));
    let mut engine = SongAudioEngine::new(&song, 48_000.0).unwrap();
    engine.note_on(1, 69, 1.0);
    let mut output = vec![0.0; 1024 * 2];
    engine.render(&mut output, 2);
    assert_eq!(energy(&output), 0.0);
}

/// The FLAC fixture: 4410 frames at 44.1 kHz, L = 0.5·sin(441 Hz).
const FLAC_SINE: &[u8] = include_bytes!("../crates/audio/tests/fixtures/sine_flac.wav");

fn user(relative: &str) -> ContentPath {
    ContentPath::Shorthand {
        root: ContentRoot::User,
        relative: relative.into(),
    }
}

/// One second of a 441 Hz sine, a 16-bit mono WAV at 44.1 kHz.
fn sine_wav() -> Vec<u8> {
    let data: Vec<u8> = (0..44_100)
        .map(|n| {
            ((std::f64::consts::TAU * 441.0 * f64::from(n) / 44_100.0).sin() * 16_000.0) as i16
        })
        .flat_map(i16::to_le_bytes)
        .collect();
    let mut bytes = b"RIFF".to_vec();
    bytes.extend((36 + data.len() as u32).to_le_bytes());
    bytes.extend(b"WAVEfmt ");
    for word in [16u32, 0x0001_0001, 44_100, 88_200, 0x0010_0002] {
        bytes.extend(word.to_le_bytes());
    }
    bytes.extend(b"data");
    bytes.extend((data.len() as u32).to_le_bytes());
    bytes.extend(data);
    bytes
}

fn chunk(id: &[u8; 4], payload: &[u8]) -> Vec<u8> {
    [&id[..], &(payload.len() as u32).to_le_bytes(), payload].concat()
}

/// A DW Sampler channel (channel 1) whose module has the `CSTM` `blob`.
fn dw_song(blob: Vec<u8>) -> Song {
    let mut song = Song::default();
    song.add_channel(ChannelKind::Synth(SynthKind::DwSampler));
    song.channels[1].rack.modules[0].file.custom_data = Some(blob);
    song
}

/// The left channel of `frames` after channel 1 plays `key`.
fn dw_note(song: &Song, read: &Reader<'_>, key: u8, frames: usize) -> Vec<f32> {
    let mut cache = SampleCache::default();
    cache.load(song, read);
    let mut engine = SongAudioEngine::with_samples(song, 48_000.0, &cache).unwrap();
    engine.note_on(1, key, 1.0);
    let mut output = vec![0.0; frames * 2];
    engine.render(&mut output, 2);
    side(&output, false)
}

/// The frequency between the first and last rising zero crossings at 48 kHz.
fn pitch(x: &[f32]) -> f64 {
    let rises: Vec<usize> = (1..x.len())
        .filter(|&n| x[n - 1] <= 0.0 && x[n] > 0.0)
        .collect();
    assert!(rises.len() > 2, "silent");
    (rises.len() - 1) as f64 * 48_000.0 / (rises[rises.len() - 1] - rises[0]) as f64
}

#[test]
fn dw_sampler_plays_its_cstm_zones_from_the_sample_cache() {
    let mut zone = b"@L@\0My Samples/Sine.wav".to_vec();
    zone.resize(1024, 0);
    zone.extend([60, 0, 127, 0, 127]);
    for value in [1.0f32, 0.5] {
        zone.extend(value.to_le_bytes());
    }
    zone.extend([0; 12]);
    for value in [0.0f32, 1.0, 1.0, 0.1] {
        zone.extend(value.to_le_bytes());
    }
    let song = dw_song([&b"10WD"[..], &chunk(b"ZONE", &zone)].concat());
    let read = |path: &ContentPath| (*path == user("My Samples/Sine.wav")).then(sine_wav);
    // Root 60: key 72 plays an octave up.
    let out = dw_note(&song, &read, 72, 9600);
    assert!(
        (pitch(&out[480..]) - 882.0).abs() < 1.0,
        "{}",
        pitch(&out[480..])
    );
    // A missing sample drops its zone: silence.
    assert!(
        dw_note(&song, &|_| None, 72, 1024)
            .iter()
            .all(|s| *s == 0.0)
    );
    // No CSTM, no zones.
    let mut bare = dw_song(Vec::new());
    bare.channels[1].rack.modules[0].file.custom_data = None;
    assert!(dw_note(&bare, &read, 72, 1024).iter().all(|s| *s == 0.0));
}

#[test]
fn dw_sampler_plays_a_program_with_an_embedded_flac_zone() {
    let record = |tag: u32, data: &[u8]| {
        [
            &tag.to_le_bytes()[..],
            &(data.len() as u64).to_le_bytes(),
            data,
        ]
        .concat()
    };
    let program = record(1, &record(3, &record(0x206, FLAC_SINE)));
    let dwp = [&b"DwPr"[..], &38u32.to_le_bytes(), &program].concat();
    let path = b"@L@\0My Programs/Sine.dwp";
    let path_chunk = [
        &(path.len() as u32).to_le_bytes()[..],
        path,
        &0f32.to_le_bytes(),
        &0.1f32.to_le_bytes(),
    ]
    .concat();
    let song = dw_song([&b"10WD"[..], &chunk(b"PATh", &path_chunk)].concat());
    let read = |p: &ContentPath| (*p == user("My Programs/Sine.dwp")).then(|| dwp.clone());
    // Default root 48: key 60 plays the 441 Hz fixture an octave up, for 4410 frames at 2·44.1/48 per frame.
    let out = dw_note(&song, &read, 60, 4800);
    assert!(
        (pitch(&out[..2300]) - 882.0).abs() < 2.0,
        "{}",
        pitch(&out[..2300])
    );
    assert!(
        out[2500..].iter().all(|s| *s == 0.0),
        "a one-shot zone ends with its sample"
    );
}

/// A `PATh` may name a plain audio file, which plays as a user sample (root 48, all keys), and
/// a `ZONE` after a path is ignored.
#[test]
fn dw_sampler_plays_the_audio_file_its_path_names_and_no_zone_after_it() {
    let path = b"@L@\0My Samples/Sine.wav";
    let path_chunk = [&(path.len() as u32).to_le_bytes()[..], path].concat();
    let mut zone = b"@L@\0My Samples/Sine.wav".to_vec();
    zone.resize(1024, 0);
    zone.extend([60, 0, 127, 0, 127]);
    zone.extend([0; 36]);
    let blob = [
        &b"10WD"[..],
        &chunk(b"PATh", &path_chunk),
        &chunk(b"ZONE", &zone),
    ]
    .concat();
    let read = |p: &ContentPath| (*p == user("My Samples/Sine.wav")).then(sine_wav);
    // Root 48: key 60 plays the 441 Hz sine an octave up (the ZONE's root 60 would leave it at 441 Hz).
    let out = dw_note(&dw_song(blob), &read, 60, 9600);
    assert!(
        (pitch(&out[480..]) - 882.0).abs() < 1.0,
        "{}",
        pitch(&out[480..])
    );
}

/// `song` built, `setup` run on the engine, then channel 1 holding one key from the keyboard for `frames` frames.
fn held_after(song: &Song, frames: usize, setup: impl FnOnce(&mut SongAudioEngine)) -> Vec<f32> {
    let mut engine = SongAudioEngine::new(song, 48_000.0).unwrap();
    setup(&mut engine);
    engine.note_on(1, 69, 1.0);
    let mut output = vec![0.0; frames * 2];
    engine.render(&mut output, 2);
    output
}

/// `song` playing from tick 0 for `frames` frames.
fn played(song: &Song, frames: usize) -> Vec<f32> {
    let mut engine = SongAudioEngine::new(song, 48_000.0).unwrap();
    engine.play();
    let mut output = vec![0.0; frames * 2];
    engine.render(&mut output, 2);
    output
}

/// MiniSynth with its mod wheel on the filter cutoff (p4 = 0) at full depth (p5 = 1), the cutoff (p9) low enough
/// for the wheel to open it, at 120 BPM.
fn mod_wheel_song() -> Song {
    let mut song = minisynth_song();
    song.tempo_bpm = 120.0;
    let params = &mut song.channels[1].rack.get_mut(2).unwrap().params;
    params[5] = 1.0;
    params[9] = 0.3;
    song
}

#[test]
fn the_mod_wheel_is_cc_1_at_channel_param_8_live_and_automated() {
    let song = mod_wheel_song();
    let plain = held_note(&song, 4096);
    assert_eq!(
        held_after(&song, 4096, |e| e.set_param(1, 0, 7, 1.0)),
        plain
    );
    let wheel = held_after(&song, 4096, |e| e.set_param(1, 0, 8, 1.0));
    assert_ne!(wheel, plain);

    // A CC lane plays like the live param.
    let mut automated = song.clone();
    let lane = automated.channels[1].add_automation_lane(AutomationTarget {
        instance_id: 0,
        param: 8,
    });
    lane.add_clip(128, 4.0).events.push(Event::Automation {
        pos: 0,
        value: 1.0,
        control: true,
    });
    // Sought to beat 1 and played, the key struck: the wheel. Then stopped, back at 0 and struck again once the release
    // has died away: Stop keeps the MIDI CCs, so the wheel stays where the lane put it, as on the song without
    // the lane with the wheel moved live. (A seek while stopped only moves the position.)
    let run = |song: &Song, live: bool| {
        let mut engine = SongAudioEngine::new(song, 48_000.0).unwrap();
        engine.seek(128);
        engine.play();
        if live {
            engine.set_param(1, 0, 8, 1.0);
        }
        engine.note_on(1, 69, 1.0);
        let mut first = vec![0.0; 4096 * 2];
        engine.render(&mut first, 2);
        engine.stop();
        engine.seek(0);
        engine.render(&mut vec![0.0; 48_000 * 2], 2);
        engine.note_on(1, 69, 1.0);
        let mut second = vec![0.0; 4096 * 2];
        engine.render(&mut second, 2);
        (first, second)
    };
    let (first, second) = run(&automated, false);
    assert_eq!(first, wheel);
    assert!(energy(&second) > 0.01);
    assert_eq!(second, run(&song, true).1);
    assert_ne!(second, run(&song, false).1);
}

#[test]
fn muted_clips_make_no_events() {
    let mut song = song_with_note();
    song.channels[1].lanes[0].clips[0].muted = true;
    assert_eq!(energy(&played(&song, 4096)), 0.0);

    // A muted automation clip does not automate, on the strip and on a generator alike.
    for target in [
        AutomationTarget {
            instance_id: 0,
            param: 0,
        },
        AutomationTarget {
            instance_id: 2,
            param: AutomationTarget::MIX,
        },
    ] {
        let mut song = song_with_note();
        let lane = song.channels[1].add_automation_lane(target);
        lane.add_clip(0, 4.0).events.push(Event::Automation {
            pos: 0,
            value: 0.0,
            control: true,
        });
        assert_eq!(energy(&played(&song, 4096)), 0.0, "{target:?}");
        lane_clip_mut(&mut song, target).muted = true;
        assert!(energy(&played(&song, 4096)) > 0.01, "{target:?}");
    }
}

fn lane_clip_mut(song: &mut Song, target: AutomationTarget) -> &mut honeydew::song::Clip {
    let lane = song.channels[1].lanes.iter_mut();
    let mut lane = lane.filter(|lane| lane.target == Some(target));
    &mut lane.next().unwrap().clips[0]
}

#[test]
fn a_clip_shows_its_source_length_when_its_visible_length_is_not_positive() {
    // A 2-beat loop with one note at its start, shown for the 8-beat source length: the note plays at beat 6.
    let mut song = song_with_note();
    song.channels[1].lanes[0].clips[0].file.source_len_beats = 8.0;
    let beat = 24_000;
    let output = played(&song, 6 * beat + 4096);
    assert!(energy(&output[2 * 6 * beat..]) > 0.01);
    // A negative visible length means the same (`CLHd` @0).
    song.channels[1].lanes[0].clips[0].visible_len_beats = -1.0;
    assert_eq!(played(&song, 6 * beat + 4096), output);
}

/// Channel 1's held-note energy after `SetParam(1, 0, index, value)`.
fn strip_energy(index: i32, value: f32) -> f32 {
    energy(&held_after(&minisynth_song(), 4096, |e| {
        e.set_param(1, 0, index, value)
    }))
}

#[test]
fn the_strip_level_follows_the_channel_law() {
    // dB = 66·v^0.6 − 60, so v = 1 is +6 dB; v = 0 is silence.
    let full = strip_energy(0, 1.0);
    for v in [0.8531_f32, 0.6, 0.25, 0.1] {
        let db = 10.0 * (strip_energy(0, v) / full).log10() + 6.0;
        let law = 66.0 * v.powf(0.6) - 60.0;
        assert!((db - law).abs() < 0.02, "v {v}: {db} dB, law {law} dB");
    }
    let default = 10.0 * (strip_energy(0, 0.8531) / full).log10() + 6.0;
    assert!(default.abs() < 0.1, "{default} dB");
    assert_eq!(strip_energy(0, 0.0), 0.0);
}

#[test]
fn the_strip_pan_follows_the_channel_law() {
    let centre = held_note(&minisynth_song(), 4096);
    let pan = |v| held_after(&minisynth_song(), 4096, |e| e.set_param(1, 0, 1, v));
    let left = pan(0.0);
    assert_eq!(side(&left, false), side(&centre, false));
    assert_eq!(energy(&side(&left, true)), 0.0);
    let right = pan(1.0);
    assert_eq!(side(&right, true), side(&centre, true));
    assert_eq!(energy(&side(&right, false)), 0.0);
    // v = 0.75: p = 0.5, so L = 0.5 and R = 1.
    let quarter = pan(0.75);
    assert_eq!(side(&quarter, true), side(&centre, true));
    for (a, b) in side(&quarter, false).iter().zip(side(&centre, false)) {
        assert!((a - 0.5 * b).abs() <= 1e-6, "{a} {b}");
    }
}

#[test]
fn strip_mute_and_solo_follow_the_bus_routing() {
    // Channel 1 (A) feeds the bus on channel 3; channel 2 (B) goes to the master.
    let mut song = minisynth_song();
    song.add_channel(ChannelKind::Synth(SynthKind::MiniSynth));
    song.add_channel(ChannelKind::EffectTrack);
    song.channels[1].file.sends[0].target_channel_id = song.channels[3].id;
    let audible = |commands: &[(usize, i32, f32)]| {
        let mut engine = SongAudioEngine::new(&song, 48_000.0).unwrap();
        for &(channel, index, value) in commands {
            engine.set_param(channel, 0, index, value);
        }
        engine.note_on(1, 69, 1.0);
        engine.note_on(2, 69, 1.0);
        let mut output = vec![0.0; 4096 * 2];
        engine.render(&mut output, 2);
        let peak = |channel: usize| {
            let index = (0..engine.meter_count())
                .find(|&i| {
                    engine.meter_key(i).channel == channel && engine.meter_key(i).instance_id == 0
                })
                .unwrap();
            engine.meter(index).peak_left
        };
        [1, 2, 3].map(|channel| peak(channel) > 0.0)
    };
    assert_eq!(audible(&[]), [true, true, true]);
    assert_eq!(audible(&[(1, 2, 1.0)]), [false, true, false]);
    assert_eq!(audible(&[(1, 3, 1.0)]), [true, false, true]);
    assert_eq!(audible(&[(3, 3, 1.0)]), [true, false, true]);
    assert_eq!(audible(&[(2, 3, 1.0)]), [false, true, false]);
    assert_eq!(audible(&[(2, 3, 1.0), (2, 3, 0.0)]), [true, true, true]);
    // Under channel 2's solo, a soloed bus lifts channel 1, which feeds it; a bus Solo of 0.5 or 0.001 is off.
    assert_eq!(audible(&[(2, 3, 1.0), (3, 3, 1.0)]), [true, true, true]);
    for value in [0.5, 0.001] {
        let heard = audible(&[(2, 3, 1.0), (3, 3, value)]);
        assert_eq!(heard, [false, true, false], "bus Solo {value}");
    }
}

#[test]
fn cc_7_sets_the_level_and_cc_8_and_cc_10_set_the_pan() {
    assert_eq!(strip_energy(14, 0.0), 0.0);
    for (index, value, silent_right) in [(15, 0.0, true), (17, 1.0, false)] {
        let output = held_after(&minisynth_song(), 4096, |e| e.set_param(1, 0, index, value));
        assert_eq!(
            energy(&side(&output, silent_right)),
            0.0,
            "CC index {index}"
        );
        assert!(
            energy(&side(&output, !silent_right)) > 0.01,
            "CC index {index}"
        );
    }
    // The later write wins: Level after CC #7.
    let plain = held_note(&minisynth_song(), 4096);
    let restored = held_after(&minisynth_song(), 4096, |e| {
        e.set_param(1, 0, 14, 0.0);
        e.set_param(1, 0, 0, 0.8531);
    });
    assert_eq!(restored, plain);
    // A CC #7 lane drives the Level too.
    let mut song = song_with_note();
    let lane = song.channels[1].add_automation_lane(AutomationTarget {
        instance_id: 0,
        param: 14,
    });
    lane.add_clip(0, 4.0).events.push(Event::Automation {
        pos: 0,
        value: 0.0,
        control: true,
    });
    assert_eq!(energy(&played(&song, 4096)), 0.0);
}

/// Strikes key 69 on channel 1 `strikes` times (4096 frames each, then its note-off), renders `after` frames, and
/// returns the energy of the 4096 frames after those.
fn tail_energy(engine: &mut SongAudioEngine, strikes: usize, after: usize) -> f32 {
    let mut output = vec![0.0; 4096 * 2];
    for _ in 0..strikes {
        engine.note_on(1, 69, 1.0);
        engine.render(&mut output, 2);
        engine.note_off(1, 69);
    }
    let mut rest = vec![0.0; after * 2];
    engine.render(&mut rest, 2);
    engine.render(&mut output, 2);
    energy(&output)
}

#[test]
fn the_sustain_pedal_holds_note_offs_until_it_lifts() {
    let song = minisynth_song();
    let mut free = SongAudioEngine::new(&song, 48_000.0).unwrap();
    let released = tail_energy(&mut free, 1, 48_000);
    let mut pedalled = SongAudioEngine::new(&song, 48_000.0).unwrap();
    pedalled.set_param(1, 0, 71, 1.0);
    let held = tail_energy(&mut pedalled, 2, 48_000);
    assert!(held > 0.01);
    assert!(released < held * 1e-4, "{released} vs {held}");
    // Lifting releases both strikes of the key.
    pedalled.set_param(1, 0, 71, 0.0);
    let lifted = tail_energy(&mut pedalled, 0, 48_000);
    assert!(lifted < held * 1e-4, "{lifted} vs {held}");

    // A pedal lane holds a sequenced note past its end (beat 1) too.
    let mut song = song_with_note();
    let lane = song.channels[1].add_automation_lane(AutomationTarget {
        instance_id: 0,
        param: 71,
    });
    lane.add_clip(0, 4.0).events.push(Event::Automation {
        pos: 0,
        value: 1.0,
        control: true,
    });
    let beat = 24_000;
    let output = played(&song, 3 * beat);
    assert!(energy(&output[2 * (3 * beat - 4096)..]) > 0.01);
}

/// MiniSynth plus `clips` Level-automation clips, each a 1-tick loop with one point shown for `visible` beats.
fn one_tick_loops(clips: u32, visible: f64) -> Song {
    let mut song = minisynth_song();
    song.tempo_bpm = 120.0;
    let lane = song.channels[1].add_automation_lane(AutomationTarget {
        instance_id: 0,
        param: 0,
    });
    for i in 0..clips {
        let clip = lane.add_clip(i * 128, 1.0 / 128.0);
        clip.visible_len_beats = visible;
        clip.events.push(Event::Automation {
            pos: 0,
            value: 0.0,
            control: true,
        });
    }
    song
}

#[test]
fn a_song_whose_loop_passes_exceed_the_play_budget_is_refused_at_once() {
    // The budget is the file reader's, in events: 64 MiB / 24 B ≈ 2.8 M plays for the whole song.
    // One clip of 300 000 plays builds and plays; ten of them are over the budget together.
    let one = one_tick_loops(1, 300_000.0 / 128.0);
    let mut engine = SongAudioEngine::new(&one, 48_000.0).unwrap();
    engine.play();
    engine.note_on(1, 69, 1.0);
    let mut output = vec![0.0; 4096 * 2];
    engine.render(&mut output, 2);
    assert_eq!(energy(&output), 0.0);
    let ten = one_tick_loops(10, 300_000.0 / 128.0);
    assert!(matches!(
        SongAudioEngine::new(&ten, 48_000.0),
        Err(EngineError::TooManyEvents)
    ));

    // Ten 1-tick loops shown for 10⁹ beats each (about 1.3·10¹² plays), as automation and as notes: refused before
    // anything is expanded.
    assert!(matches!(
        SongAudioEngine::new(&one_tick_loops(10, 1e9), 48_000.0),
        Err(EngineError::TooManyEvents)
    ));
    let mut song = minisynth_song();
    for i in 0..10 {
        let clip = song.channels[1].lanes[0].add_clip(i * 128, 1.0 / 128.0);
        clip.visible_len_beats = 1e9;
        clip.events.push(Event::Note {
            pos: 0,
            len_beats: 1.0 / 256.0,
            key: 69,
            velocity: 204,
            pan: 127,
            slide: false,
        });
    }
    assert!(matches!(
        SongAudioEngine::new(&song, 48_000.0),
        Err(EngineError::TooManyEvents)
    ));
}

/// Pedal down, key 69 struck and released (held by the pedal), then `between` (a seek or a stop), then key 69
/// struck again and held, and the pedal lifted: the energy of the next 4096 frames.
fn restrike_after(between: impl FnOnce(&mut SongAudioEngine)) -> f32 {
    let mut engine = SongAudioEngine::new(&minisynth_song(), 48_000.0).unwrap();
    engine.set_param(1, 0, 71, 1.0);
    let mut output = vec![0.0; 4096 * 2];
    engine.note_on(1, 69, 1.0);
    engine.render(&mut output, 2);
    engine.note_off(1, 69);
    engine.render(&mut output, 2);
    between(&mut engine);
    engine.note_on(1, 69, 1.0);
    engine.render(&mut output, 2);
    engine.set_param(1, 0, 71, 0.0);
    let mut rest = vec![0.0; 48_000 * 2];
    engine.render(&mut rest, 2);
    engine.render(&mut output, 2);
    energy(&output)
}

#[test]
fn stop_drops_the_held_note_offs_and_a_seek_keeps_them() {
    // The note-off held before the stop must not release the key struck after it.
    assert!(restrike_after(|e| e.stop()) > 0.01);
    // A seek leaves the voices sounding, so the note-offs the pedal holds for them stay held and still
    // release the key when it lifts, as with no seek.
    assert_eq!(restrike_after(|e| e.seek(0)), restrike_after(|_| {}));
    assert!(restrike_after(|e| e.seek(0)) < 1e-6);
}

/// A MiniSynth channel at 120 BPM playing `notes`, each (key, start beat, length in beats).
fn notes_song(notes: &[(u8, f64, f64)]) -> Song {
    let mut song = minisynth_song();
    song.tempo_bpm = 120.0;
    let clip = song.channels[1].lanes[0].add_clip(0, 8.0);
    for &(key, start, len_beats) in notes {
        clip.events.push(Event::Note {
            pos: (start * 128.0) as u32,
            len_beats,
            key,
            velocity: 204,
            pan: 127,
            slide: false,
        });
    }
    song
}

/// Note effect `type_id` added at the top of channel 1's rack (above the synth), with `params` set; its instance id.
fn note_effect_on_top(song: &mut Song, type_id: u16, params: &[(usize, f32)]) -> i32 {
    let id = song.channels[1].rack.insert(0, type_id);
    let module = song.channels[1].rack.get_mut(id).unwrap();
    for &(index, value) in params {
        module.params[index] = value;
    }
    id
}

/// An automation lane on param `param` of channel 1's module `instance_id`, at `value` from song tick `tick` on.
fn automate(song: &mut Song, instance_id: i32, param: i32, tick: u32, value: f32) {
    let lane = song.channels[1].add_automation_lane(AutomationTarget { instance_id, param });
    lane.add_clip(tick, 8.0).events.push(Event::Automation {
        pos: 0,
        value,
        control: true,
    });
}

/// One beat at 120 BPM and 48 kHz.
const BEAT: usize = 24_000;

/// Each instrument gets the event list as it stands at its own
/// rack position, its note-offs included.
#[test]
fn a_note_effect_acts_only_on_the_instruments_below_it() {
    // An octave up between two MiniSynths; the second note outlasts the first.
    let notes = [(60, 0.0, 1.0), (72, 0.0, 3.0)];
    // Then the same with the sustain pedal (CC #64) down from the start until just before beat 2: each instrument
    // holds its own note-offs, so at the lift the first synth releases 60 (not its 72) and the second 72 (not its 84).
    for pedal in [false, true] {
        let song = |notes: &[(u8, f64, f64)]| {
            let mut song = notes_song(notes);
            if pedal {
                let lane = song.channels[1].add_automation_lane(AutomationTarget {
                    instance_id: 0,
                    param: 71,
                });
                let clip = lane.add_clip(0, 8.0);
                for (pos, value) in [(0, 1.0), (250, 1.0), (256, 0.0)] {
                    clip.events.push(Event::Automation {
                        pos,
                        value,
                        control: true,
                    });
                }
            }
            song
        };
        let mut both = song(&notes);
        both.channels[1].rack.insert(1, 48);
        let transpose = both.channels[1].rack.modules[1].instance_id;
        both.channels[1].rack.get_mut(transpose).unwrap().params[1] = 5.0 / 8.0;
        both.channels[1].rack.add(0);
        let both = played(&both, 4 * BEAT);
        let above = played(&song(&notes), 4 * BEAT);
        let below = played(&song(&[(72, 0.0, 1.0), (84, 0.0, 3.0)]), 4 * BEAT);
        assert!(energy(&above) > 0.01);
        for (i, (sum, (a, b))) in both.iter().zip(above.iter().zip(&below)).enumerate() {
            assert!(
                (sum - (a + b)).abs() <= 1e-5,
                "pedal {pedal}, frame {}: {sum} vs {}",
                i / 2,
                a + b
            );
        }
    }
}

#[test]
fn an_automated_transpose_changes_pitch_mid_song() {
    let mut song = notes_song(&[(60, 0.0, 1.0), (60, 2.0, 1.0)]);
    let transpose = note_effect_on_top(&mut song, 48, &[]);
    // A semitone up from beat 1.
    automate(&mut song, transpose, 2, 128, 13.0 / 24.0);
    let expected = played(&notes_song(&[(60, 0.0, 1.0), (61, 2.0, 1.0)]), 4 * BEAT);
    assert!(played(&song, 4 * BEAT) == expected);
}

/// Engine-notes item 12: automating Scale's Key rewrites its notes (p4–p15), as the panel's Key does.
#[test]
fn scale_automation_re_snaps_notes() {
    let mut song = notes_song(&[(61, 0.0, 1.0), (61, 2.0, 1.0)]);
    // C major by default, Snap on: C# snaps down to C.
    let scale = note_effect_on_top(&mut song, 50, &[]);
    // Key D from beat 1: D major has C#.
    automate(&mut song, scale, 1, 128, 2.0 / 11.0);
    let expected = played(&notes_song(&[(60, 0.0, 1.0), (61, 2.0, 1.0)]), 4 * BEAT);
    assert!(played(&song, 4 * BEAT) == expected);
}

/// Engine-notes item 14: with Sync On, `Delay (sync)` (p3) is snapped to its item before it reaches the rack, for a
/// stored value (a preset) and automation alike. p3 = 0.12 is nearest `1/8` (b = 0.146 unsnapped).
#[test]
fn an_unsnapped_echo_delay_echoes_at_the_nearest_item() {
    let first_difference = |song: &Song, reference: &Song| {
        let (a, b) = (played(song, BEAT), played(reference, BEAT));
        a.iter().zip(&b).position(|(x, y)| x != y).map(|i| i / 2)
    };
    let echo = |params: &[(usize, f32)]| {
        // A 1/16-beat note: an echo of a key still held is not emitted.
        let mut song = notes_song(&[(60, 0.0, 0.0625)]);
        let id = note_effect_on_top(&mut song, 47, params);
        (song, id)
    };
    // The same note without its one echo.
    let (dry, _) = echo(&[(1, 0.0)]);
    let (stored, _) = echo(&[(1, 1.0 / 32.0), (3, 0.12)]);
    let (mut automated, id) = echo(&[(1, 1.0 / 32.0)]);
    automate(&mut automated, id, 3, 0, 0.12);
    for song in [stored, automated] {
        // 1/8 beat is frame 3000; the echo's attack shows within a few frames.
        let onset = first_difference(&song, &dry).expect("an echo");
        assert!((3000..3008).contains(&onset), "{onset}");
    }
}

#[test]
fn a_transistor_bass_channel_plays_its_notes() {
    let mut song = Song {
        tempo_bpm: 120.0,
        ..Song::default()
    };
    song.add_channel(ChannelKind::Synth(SynthKind::TransistorBass));
    song.channels[1].lanes[0]
        .add_clip(0, 2.0)
        .events
        .push(Event::Note {
            pos: 0,
            len_beats: 1.0,
            key: 45,
            velocity: 100,
            pan: 127,
            slide: false,
        });
    let mut engine = SongAudioEngine::new(&song, 48_000.0).unwrap();
    engine.play();
    let mut output = vec![0.0; 4096 * 2];
    engine.render(&mut output, 2);
    assert!(energy(&output) > 0.01);

    let mut keyboard = SongAudioEngine::new(&song, 48_000.0).unwrap();
    keyboard.note_on(1, 45, 0.5);
    keyboard.render(&mut output, 2);
    assert!(energy(&output) > 0.01);
}

#[test]
fn supersaw_plays_and_its_defaults_match_the_registry() {
    let spec = honeydew::params::module_spec(23).unwrap();
    let defaults: Vec<f32> = spec.params.iter().map(|param| param.default).collect();
    assert_eq!(defaults, honeydew_audio::supersaw::DEFAULTS);
    let mut song = Song::default();
    song.add_channel(ChannelKind::Synth(SynthKind::SuperSaw));
    let mut engine = SongAudioEngine::new(&song, 48_000.0).unwrap();
    engine.note_on(1, 69, 1.0);
    let mut output = vec![0.0; 4096 * 2];
    engine.render(&mut output, 2);
    assert!(energy(&output) > 1.0);
}

/// The metronome, the count-in, monitoring and the test
/// click, rendered offline. The input is an `InputRing` the tests push frames into; no device.
mod recording {
    use std::sync::Arc;

    use honeydew::audio_engine::{LiveInput, MONITOR_MAX_LAG_FRAMES, SongAudioEngine};
    use honeydew::sample_cache::SampleCache;
    use honeydew::song::{ChannelKind, Song};
    use honeydew_audio::device::AudioRenderer;
    use honeydew_audio::input::InputRing;

    use super::{energy, song_with_note};

    /// The next `frames` frames from `engine`, left and right, rendered in blocks of 1000.
    fn render_stereo(engine: &mut SongAudioEngine, frames: usize) -> (Vec<f32>, Vec<f32>) {
        let (mut left, mut right) = (Vec::with_capacity(frames), Vec::with_capacity(frames));
        let mut output = vec![0.0; 2000];
        while left.len() < frames {
            let n = 1000.min(frames - left.len());
            engine.render(&mut output[..n * 2], 2);
            left.extend(output[..n * 2].iter().step_by(2));
            right.extend(output[1..n * 2].iter().step_by(2));
        }
        (left, right)
    }

    fn render(engine: &mut SongAudioEngine, frames: usize) -> Vec<f32> {
        render_stereo(engine, frames).0
    }

    /// The pitch of the square wave starting at `at`, from its sign changes over 20 ms.
    fn pitch(samples: &[f32], at: usize, rate: f32) -> f32 {
        let window = &samples[at..at + (rate * 0.02) as usize];
        let changes = window
            .windows(2)
            .filter(|pair| (pair[0] > 0.0) != (pair[1] > 0.0))
            .count();
        changes as f32 / 2.0 / 0.02
    }

    #[test]
    fn the_metronome_clicks_each_beat_while_playing_and_accents_beat_0_of_each_bar() {
        // b = TDIV byte 1, 3 here: beats 0 and 3 take the 2500 Hz accent. At 130 BPM a beat is 22 153.8
        // frames, so beat k starts at ceil(k·beat), inside a render call (1000 frames each).
        let song = Song {
            tempo_bpm: 130.0,
            time_sig: honeydew::song::TimeSig {
                subdivision: 4,
                beats_per_bar: 3,
            },
            ..Song::default()
        };
        let beat = 60.0 * 48_000.0 / 130.0;
        let starts: Vec<usize> = (0..6).map(|k| (k as f64 * beat).ceil() as usize).collect();
        assert_eq!(starts[1..4], [22_154, 44_308, 66_462]);
        let mut engine = SongAudioEngine::new(&song, 48_000.0).unwrap();
        engine.set_metronome(true, true, 1.0);
        assert_eq!(
            energy(&render(&mut engine, 4096)),
            0.0,
            "clicked while stopped"
        );
        engine.play();
        let out = render(&mut engine, starts[5]);
        for k in 0..5 {
            let at = starts[k];
            assert!((out[at] - 0.631).abs() < 1e-6, "beat {k}: {}", out[at]);
            let hz = if k % 3 == 0 { 2500.0 } else { 2000.0 };
            assert!((pitch(&out, at, 48_000.0) - hz).abs() <= 50.0, "beat {k}");
            assert!(
                out[at + 1000..starts[k + 1]].iter().all(|s| *s == 0.0),
                "beat {k} rang on past its envelope, or the next one started early"
            );
        }
        // The level law: v 0.5 is 0.325. Off stops the clicks, whatever the level.
        engine.set_metronome(true, true, 0.5);
        let out = render(&mut engine, starts[1]);
        assert!((out[0] - 0.631 * 0.325).abs() < 1e-6, "{}", out[0]);
        engine.set_metronome(false, true, f32::NAN);
        assert_eq!(
            energy(&render(&mut engine, starts[2])),
            0.0,
            "clicked while off"
        );
    }

    #[test]
    fn a_count_in_holds_the_transport_for_four_clicked_beats_then_the_song_plays() {
        // 130 BPM: a beat is 20 353.8 frames at 44.1 kHz and 22 153.8 at 48 kHz; beat k starts at ceil(k·beat).
        let mut song = song_with_note();
        song.tempo_bpm = 130.0;
        for (rate, length) in [(44_100.0, 81_416), (48_000.0, 88_616)] {
            let beat = 60.0 * f64::from(rate) / 130.0;
            let mut engine = SongAudioEngine::new(&song, rate).unwrap();
            engine.count_in(4);
            assert!(!engine.counting_in(), "counting before Play");
            engine.play();
            assert!(engine.counting_in());
            let out = render(&mut engine, length - 1);
            assert!(engine.counting_in(), "{rate}: the count-in ended early");
            assert_eq!(engine.transport_tick(), 0, "the transport moved");
            // Four clicks at the setting's default level (v 0.5: 0.325), the first accented; nothing else sounds.
            let starts: Vec<usize> = (0..4).map(|k| (k as f64 * beat).ceil() as usize).collect();
            for (k, &at) in starts.iter().enumerate() {
                assert!((out[at] - 0.631 * 0.325).abs() < 1e-6, "{rate}: beat {k}");
                let hz = if k == 0 { 2500.0 } else { 2000.0 };
                assert!(
                    (pitch(&out, at, rate) - hz).abs() <= 50.0,
                    "{rate}: beat {k}"
                );
            }
            let stray = (out.iter().enumerate()).find(|&(i, s)| {
                *s != 0.0 && !starts.iter().any(|&at| (at..at + 1000).contains(&i))
            });
            assert_eq!(
                stray, None,
                "{rate}: a song event sounded during the count-in"
            );
            render(&mut engine, 1);
            assert!(
                !engine.counting_in(),
                "{rate}: the count-in is {length} frames"
            );
            assert_eq!(engine.transport_tick(), 0);
            // Then the song plays from its position: the note at tick 0, and no metronome (it is off).
            let after = render(&mut engine, 4096);
            assert!(energy(&after) > 0.01, "{rate}: the song did not start");
            assert!(engine.transport_tick() > 0);
        }
    }

    #[test]
    fn stop_ends_a_count_in_and_a_count_in_waits_for_the_next_play() {
        let mut engine = SongAudioEngine::new(&song_with_note(), 48_000.0).unwrap();
        engine.count_in(4);
        engine.play();
        render(&mut engine, 100);
        engine.stop();
        assert!(!engine.counting_in());
        // The first click's tail rings out (900 more samples); no further beat clicks.
        let out = render(&mut engine, 30_000);
        assert!(out[899] != 0.0);
        assert_eq!(energy(&out[900..]), 0.0, "the count-in clicked on");
        // A plain Play has no count-in, and CountIn(0) takes one back.
        engine.play();
        assert!(!engine.counting_in());
        engine.stop();
        engine.count_in(4);
        engine.count_in(0);
        engine.play();
        assert!(!engine.counting_in());
    }

    /// An Audio Record channel (1) with monitoring on (CHHD @1056) and a Leveller at Gain `gain`.
    fn monitored(gain: f32) -> Song {
        let mut song = Song::default();
        song.add_channel(ChannelKind::AudioRecord);
        let channel = &mut song.channels[1];
        channel.file.input_monitoring = true;
        let leveller = channel.rack.add(20);
        channel.rack.get_mut(leveller).unwrap().params[1] = gain;
        song
    }

    fn engine_on(
        song: &Song,
        ring: &Arc<InputRing>,
        ports: Vec<(i32, usize, Option<usize>)>,
    ) -> SongAudioEngine {
        let input = LiveInput::new(ring.clone(), ports);
        SongAudioEngine::with_input(song, 48_000.0, &SampleCache::default(), Some(&input)).unwrap()
    }

    /// Port 0 and port 1 of input frame `i`: two sines.
    fn input(i: usize) -> [f32; 2] {
        let t = i as f32 / 48_000.0;
        [
            0.4 * (std::f32::consts::TAU * 440.0 * t).sin(),
            -0.3 * (std::f32::consts::TAU * 150.0 * t).sin(),
        ]
    }

    /// Pushes `frames` frames of [`input`] into `ring`.
    fn push(ring: &InputRing, frames: std::ops::Range<usize>) {
        let samples: Vec<f32> = frames.flat_map(input).collect();
        ring.push(samples.into_iter());
    }

    /// Pushes 9600 frames of [`input`] in blocks of 480, each block rendered by every engine as it arrives; each
    /// engine's (left, right) for the last 4800 frames, when the Leveller has settled.
    fn monitor(ring: &InputRing, engines: &mut [SongAudioEngine]) -> Vec<(Vec<f32>, Vec<f32>)> {
        let mut outputs = vec![(Vec::new(), Vec::new()); engines.len()];
        for block in 0..20 {
            push(ring, block * 480..(block + 1) * 480);
            for (engine, (left, right)) in engines.iter_mut().zip(&mut outputs) {
                let (l, r) = render_stereo(engine, 480);
                if block >= 10 {
                    left.extend(l);
                    right.extend(r);
                }
            }
        }
        outputs
    }

    /// The gain k that best fits `output` = k·`input`, and the largest error left.
    fn fit(output: &[f32], input: &[f32]) -> (f32, f32) {
        let k = output.iter().zip(input).map(|(o, i)| o * i).sum::<f32>()
            / input.iter().map(|i| i * i).sum::<f32>();
        let error = (output.iter().zip(input)).fold(0.0_f32, |e, (o, i)| e.max((o - k * i).abs()));
        (k, error)
    }

    #[test]
    fn a_monitored_channel_with_a_gain_effect_outputs_the_pushed_input_scaled() {
        let ring = Arc::new(InputRing::new(2, 48_000, 48_000));
        let swapped = vec![(1, 1, Some(0))];
        let mut engines = [
            // The Leveller at 0 dB (its default), and at Gain 0.5.
            engine_on(&monitored(0.738), &ring, vec![]),
            engine_on(&monitored(0.5), &ring, vec![]),
            // Channel id 1 takes port 1 on the left and port 0 on the right.
            engine_on(&monitored(0.738), &ring, swapped),
        ];
        let outputs = monitor(&ring, &mut engines);
        let port = |p: usize| -> Vec<f32> { (4800..9600).map(|i| input(i)[p]).collect() };
        let (port0, port1) = (port(0), port(1));
        // Left = the first port, right = the second or the first; the strip is at unity (Level 0.8531, centre).
        for (side, expected) in [(&outputs[0].0, &port0), (&outputs[0].1, &port0)] {
            let (k, error) = fit(side, expected);
            assert!(
                (k - 1.0).abs() < 1e-3 && error < 1e-3,
                "unity: k {k}, error {error}"
            );
        }
        let (k, error) = fit(&outputs[1].0, &port0);
        assert!(
            error < 1e-4 && (0.2..0.3).contains(&k),
            "Gain 0.5 (about −12.5 dB): k {k}, error {error}"
        );
        assert_eq!(fit(&outputs[1].1, &port0).0, k);
        for (side, expected) in [(&outputs[2].0, &port1), (&outputs[2].1, &port0)] {
            let (k, error) = fit(side, expected);
            assert!(
                (k - 1.0).abs() < 1e-3 && error < 1e-3,
                "ports: k {k}, error {error}"
            );
        }
    }

    #[test]
    fn only_an_audio_record_channel_with_monitoring_on_and_a_ring_at_the_rate_monitors() {
        let ring = Arc::new(InputRing::new(2, 48_000, 48_000));
        let mut off = monitored(0.738);
        off.channels[1].file.input_monitoring = false;
        let mut synth = Song::default();
        synth.add_channel(ChannelKind::Synth(honeydew::song::SynthKind::MiniSynth));
        synth.channels[1].file.input_monitoring = true;
        let other_rate = Arc::new(InputRing::new(2, 44_100, 48_000));
        let mut engines = [
            engine_on(&off, &ring, vec![]),
            engine_on(&synth, &ring, vec![]),
            engine_on(&monitored(0.738), &other_rate, vec![]),
        ];
        push(&other_rate, 0..9600);
        for (n, (left, right)) in monitor(&ring, &mut engines).into_iter().enumerate() {
            assert_eq!(energy(&left) + energy(&right), 0.0, "engine {n} monitored");
        }
    }

    #[test]
    fn a_monitor_that_falls_behind_skips_ahead_to_a_bounded_lag() {
        // A ramp: input frame i holds i / 100 000, so an output sample names the frame it came from.
        let ring = Arc::new(InputRing::new(1, 48_000, 48_000));
        let mut song = Song::default();
        song.add_channel(ChannelKind::AudioRecord);
        song.channels[1].file.input_monitoring = true;
        let mut engine = engine_on(&song, &ring, vec![]);
        // A backlog within the bound is played in order, oldest first. The strip's gain (about 0.9997) is read off
        // the ramp, so that a sample names its frame exactly.
        ring.push((0..1000).map(|i| i as f32 / 100_000.0));
        let out = render(&mut engine, 256);
        let gain = out[255] / (255.0 / 100_000.0);
        let frame_of = |sample: f32| (sample / gain * 100_000.0).round() as i64;
        let frames: Vec<i64> = out[..256].iter().map(|s| frame_of(*s)).collect();
        assert_eq!(frames, (0..256).collect::<Vec<_>>());
        // 19 000 more frames arrive at once (the clocks drifted, or the engine started late): the next block takes
        // the newest frames, at most MONITOR_MAX_LAG_FRAMES behind, and in order.
        ring.push((1000..20_000).map(|i| i as f32 / 100_000.0));
        let out = render(&mut engine, 256);
        // It skips to half the bound: 1024 frames are left behind this block.
        let first = frame_of(out[0]);
        assert_eq!(20_000 - 256 - first, MONITOR_MAX_LAG_FRAMES as i64 / 2);
        assert_eq!(
            frame_of(out[255]) - first,
            255,
            "the block is not contiguous"
        );
    }
}

/// The looped 440 Hz sine bank (root key 69) that `crates/audio/tests/soundfont.rs` writes.
const SINE_SF2: &[u8] = include_bytes!("../crates/audio/tests/fixtures/sine.sf2");

/// A SoundFont Player channel whose `SF2D` names `My SoundFonts/sine.sf2`, patch 0, and the cache that loaded it.
fn soundfont_song() -> (Song, honeydew::sample_cache::SampleCache) {
    use honeydew::flm::chunk::{ContentPath, ContentRoot};
    let path = ContentPath::Shorthand {
        root: ContentRoot::User,
        relative: "My SoundFonts/sine.sf2".into(),
    };
    let mut song = Song {
        tempo_bpm: 120.0,
        ..Song::default()
    };
    song.add_channel(ChannelKind::Synth(SynthKind::SoundFontPlayer));
    let sf2d = honeydew::flm::sf2d::Sf2d {
        path: path.clone(),
        patch: 0,
    };
    song.channels[1].rack.modules[0].file.custom_data = Some(sf2d.write(None).unwrap());
    let mut cache = honeydew::sample_cache::SampleCache::default();
    cache.load(&song, &|p| (*p == path).then(|| SINE_SF2.to_vec()));
    (song, cache)
}

#[test]
fn a_soundfont_player_plays_its_patch_from_the_cache() {
    let (song, cache) = soundfont_song();
    let mut engine = SongAudioEngine::with_samples(&song, 48_000.0, &cache).unwrap();
    engine.note_on(1, 69, 1.0);
    let mut output = vec![0.0; 9600 * 2];
    engine.render(&mut output, 2);
    let left = side(&output, false);
    let rising = (4800..9600)
        .filter(|&i| left[i - 1] <= 0.0 && left[i] > 0.0)
        .count();
    assert!(rising.abs_diff(44) <= 1, "{rising} cycles in 0.1 s");
    // Without the SoundFont the channel builds and is silent.
    let mut engine = SongAudioEngine::new(&song, 48_000.0).unwrap();
    engine.note_on(1, 69, 1.0);
    engine.render(&mut output, 2);
    assert_eq!(energy(&output), 0.0);
}

// --- Slicer ---

mod slicer {
    use std::f64::consts::TAU;

    use honeydew::audio_engine::{SongAudioEngine, slicer_auto_clip};
    use honeydew::flm::chunk::ContentPath;
    use honeydew::sample_cache::{SampleCache, SampleKey};
    use honeydew::song::{ChannelKind, Event, Song, SynthKind};
    use honeydew_audio::device::AudioRenderer;

    const LOOP: &str = "My Samples/Loops/loop.wav";
    const RATE: f32 = 44_100.0;

    /// A 16-bit mono PCM WAV at 44.1 kHz.
    fn wav(samples: &[f32]) -> Vec<u8> {
        let data: Vec<u8> = (samples.iter())
            .flat_map(|s| ((s * 32767.0).round() as i16).to_le_bytes())
            .collect();
        let mut bytes = b"RIFF".to_vec();
        bytes.extend((36 + data.len() as u32).to_le_bytes());
        bytes.extend(b"WAVEfmt ");
        bytes.extend(16u32.to_le_bytes());
        bytes.extend(1u16.to_le_bytes());
        bytes.extend(1u16.to_le_bytes());
        bytes.extend(44_100u32.to_le_bytes());
        bytes.extend(88_200u32.to_le_bytes());
        bytes.extend(2u16.to_le_bytes());
        bytes.extend(16u16.to_le_bytes());
        bytes.extend(b"data");
        bytes.extend((data.len() as u32).to_le_bytes());
        bytes.extend(data);
        bytes
    }

    /// One second with a 1 kHz burst on each quarter: four slices at the default sensitivity.
    fn four_bursts() -> Vec<f32> {
        let mut x = vec![0.0_f32; 44_100];
        for onset in [0, 11_025, 22_050, 33_075] {
            for (t, s) in x[onset..].iter_mut().take(5000).enumerate() {
                let t = t as f64 / 44_100.0;
                *s = (0.8 * (TAU * 1000.0 * t).sin() * (-t / 0.01).exp()) as f32;
            }
        }
        x
    }

    /// A Slicer's `CSTM`: `SENS` (sensitivity, AUTO 1) when given, and `PATH` naming `@L@` + `relative`.
    fn cstm(sensitivity: Option<f32>, relative: &str) -> Vec<u8> {
        let mut blob = Vec::new();
        if let Some(sensitivity) = sensitivity {
            blob.extend(b"SENS\x05\0\0\0");
            blob.extend(sensitivity.to_le_bytes());
            blob.push(1);
        }
        let path = [&b"@L@\0"[..], relative.as_bytes()].concat();
        blob.extend(b"PATH");
        blob.extend((4 + path.len() as u32).to_le_bytes());
        blob.extend((path.len() as u32).to_le_bytes());
        blob.extend(path);
        blob
    }

    /// Channel 1 is a Slicer playing `LOOP` at `sensitivity`; its instance id.
    fn song(sensitivity: Option<f32>) -> (Song, i32) {
        let mut song = Song {
            tempo_bpm: 120.0,
            ..Song::default()
        };
        song.add_channel(ChannelKind::Synth(SynthKind::Slicer));
        let module = &mut song.channels[1].rack.modules[0];
        module.file.custom_data = Some(cstm(sensitivity, LOOP));
        let id = module.instance_id;
        (song, id)
    }

    fn cache(song: &Song, samples: &[f32]) -> SampleCache {
        let bytes = wav(samples);
        let mut cache = SampleCache::default();
        cache.load(song, &|path| {
            matches!(path, ContentPath::Shorthand { relative, .. } if relative == LOOP)
                .then(|| bytes.clone())
        });
        cache
    }

    fn left(engine: &mut SongAudioEngine, frames: usize) -> Vec<f32> {
        let mut output = vec![0.0; frames * 2];
        engine.render(&mut output, 2);
        output.iter().step_by(2).copied().collect()
    }

    fn energy(samples: &[f32]) -> f32 {
        samples.iter().map(|s| s * s).sum()
    }

    #[test]
    fn the_loop_named_by_its_cstm_plays_slice_k_on_key_48_plus_k() {
        let (song, _) = song(Some(0.25));
        let cache = cache(&song, &four_bursts());
        let mut engine = SongAudioEngine::with_samples(&song, RATE, &cache).unwrap();
        for (key, sounds) in [(47, false), (48, true), (49, true), (51, true), (52, false)] {
            engine.seek(0);
            engine.note_on(1, key, 1.0);
            let out = left(&mut engine, 11_025);
            assert_eq!(energy(&out[..1000]) > 1.0, sounds, "key {key}");
            // A slice stops at its end: the next burst does not follow.
            assert!(out[10_000..].iter().all(|&s| s == 0.0), "key {key}");
        }
    }

    #[test]
    fn a_missing_loop_is_silent() {
        let (song, _) = song(Some(0.25));
        let mut cache = SampleCache::default();
        cache.load(&song, &|_| None);
        let loop_key = SampleKey::File(ContentPath::Shorthand {
            root: honeydew::flm::chunk::ContentRoot::User,
            relative: LOOP.into(),
        });
        assert_eq!(cache.missing().collect::<Vec<_>>(), [&loop_key]);
        let mut engine = SongAudioEngine::with_samples(&song, RATE, &cache).unwrap();
        engine.note_on(1, 48, 1.0);
        assert_eq!(energy(&left(&mut engine, 4096)), 0.0);
    }

    #[test]
    fn enable_off_freezes_the_voices_and_on_resumes_them() {
        // Sensitivity 0 without cue markers: one whole-file slice.
        let (song, id) = song(Some(0.0));
        let ramp: Vec<f32> = (0..44_100).map(|i| (i % 500) as f32 / 1000.0).collect();
        let cache = cache(&song, &ramp);
        let mut reference = SongAudioEngine::with_samples(&song, RATE, &cache).unwrap();
        reference.note_on(1, 48, 1.0);
        let reference = left(&mut reference, 3000);
        let mut engine = SongAudioEngine::with_samples(&song, RATE, &cache).unwrap();
        engine.note_on(1, 48, 1.0);
        assert_eq!(left(&mut engine, 1000), reference[..1000]);
        engine.set_param(1, id, 0, 0.0);
        assert_eq!(energy(&left(&mut engine, 1000)), 0.0);
        engine.set_param(1, id, 0, 1.0);
        let resumed = left(&mut engine, 1000);
        for (a, b) in resumed.iter().zip(&reference[1000..2000]) {
            assert!((a - b).abs() < 1e-6);
        }
    }

    #[test]
    fn the_slicer_ignores_pitch_bend() {
        let (song, _) = song(Some(0.25));
        let cache = cache(&song, &four_bursts());
        let play = |bend: Option<f32>| {
            let mut engine = SongAudioEngine::with_samples(&song, RATE, &cache).unwrap();
            if let Some(bend) = bend {
                engine.set_param(1, 0, 4, bend);
            }
            engine.note_on(1, 48, 1.0);
            left(&mut engine, 4000)
        };
        let straight = play(None);
        assert!(energy(&straight) > 1.0);
        assert_eq!(play(Some(1.0)), straight);
        assert_eq!(play(Some(0.0)), straight);
    }

    #[test]
    fn auto_writes_one_note_per_slice_over_the_clip_length() {
        let (song, id) = song(Some(0.25));
        let cache = cache(&song, &four_bursts());
        // One second at 120 BPM: b = 2 beats, so the clip is max(4, 1) = 4 beats; slices start on the beats.
        let (beats, events) = slicer_auto_clip(&song, 1, id, &cache).unwrap();
        assert_eq!(beats, 4.0);
        assert_eq!(events.len(), 4);
        for (k, event) in events.iter().enumerate() {
            let &Event::Note {
                pos,
                key,
                velocity,
                pan,
                slide,
                ..
            } = event
            else {
                panic!("{event:?}");
            };
            assert_eq!((key, velocity, pan, slide), (48 + k as u8, 179, 127, false));
            // A slice starts within one 128-frame window before its burst: 1.5 ticks here.
            assert!(
                pos <= 128 * k as u32 && 128 * k as u32 - pos <= 2,
                "{k}: {pos}"
            );
        }
        assert_eq!(slicer_auto_clip(&song, 1, id + 1, &cache), None);
        assert_eq!(
            slicer_auto_clip(&song, 1, id, &SampleCache::default()),
            None
        );
    }

    #[test]
    fn the_sensitivity_comes_from_the_cstm() {
        let notes = |sensitivity| {
            let (song, id) = song(sensitivity);
            let cache = cache(&song, &four_bursts());
            slicer_auto_clip(&song, 1, id, &cache).unwrap().1.len()
        };
        assert_eq!(notes(Some(0.25)), 4);
        assert_eq!(notes(Some(0.0)), 1, "M: no cue markers, one slice");
        assert_eq!(notes(None), 4, "no SENS: 0.33");
    }

    #[test]
    fn a_second_slicer_starts_one_key_above_the_first() {
        let (mut song, first) = song(Some(0.25));
        let second = song.channels[1].rack.add(33);
        song.channels[1]
            .rack
            .get_mut(second)
            .unwrap()
            .file
            .custom_data = Some(cstm(Some(0.25), LOOP));
        let cache = cache(&song, &four_bursts());
        let keys = |id| -> Vec<u8> {
            let (_, events) = slicer_auto_clip(&song, 1, id, &cache).unwrap();
            (events.iter())
                .map(|event| match event {
                    Event::Note { key, .. } => *key,
                    _ => unreachable!(),
                })
                .collect()
        };
        assert_eq!(keys(first), [48, 49, 50, 51]);
        assert_eq!(keys(second), [52, 53, 54, 55]);
        // Both play: key 52 sounds only from the second.
        let mut engine = SongAudioEngine::with_samples(&song, RATE, &cache).unwrap();
        engine.note_on(1, 52, 1.0);
        assert!(energy(&left(&mut engine, 1000)) > 1.0);
    }
}

#[test]
fn the_sustain_pedal_is_down_only_above_one_half() {
    // CC #64 at 0.5 or 0.001 holds nothing, and lowering a
    // held pedal to either releases the held note-offs.
    let song = minisynth_song();
    let mut down = SongAudioEngine::new(&song, 48_000.0).unwrap();
    down.set_param(1, 0, 71, 1.0);
    let held = tail_energy(&mut down, 1, 48_000);
    assert!(held > 0.01);
    for value in [0.5, 0.001] {
        let mut up = SongAudioEngine::new(&song, 48_000.0).unwrap();
        up.set_param(1, 0, 71, value);
        let released = tail_energy(&mut up, 1, 48_000);
        assert!(
            released < held * 1e-4,
            "pedal {value}: {released} vs {held}"
        );
        let mut lifted = SongAudioEngine::new(&song, 48_000.0).unwrap();
        lifted.set_param(1, 0, 71, 1.0);
        tail_energy(&mut lifted, 1, 48_000);
        lifted.set_param(1, 0, 71, value);
        let tail = tail_energy(&mut lifted, 0, 48_000);
        assert!(tail < held * 1e-4, "lifted to {value}: {tail} vs {held}");
    }
}

#[test]
fn strip_mute_and_solo_are_on_only_above_one_half() {
    // Out of the mix if Mute > 0.5, or if any channel has Solo > 0.5 and its own
    // Solo is ≤ 0.5. Channel 1 holds a key; channel 2 is a second MiniSynth.
    let mut song = minisynth_song();
    song.add_channel(ChannelKind::Synth(SynthKind::MiniSynth));
    let heard = |commands: &[(usize, i32, f32)]| {
        energy(&held_after(&song, 4096, |e| {
            for &(channel, index, value) in commands {
                e.set_param(channel, 0, index, value);
            }
        })) > 0.0
    };
    assert!(heard(&[]));
    assert!(!heard(&[(1, 2, 1.0)]));
    assert!(!heard(&[(2, 3, 1.0)]));
    for value in [0.5, 0.001] {
        assert!(heard(&[(1, 2, value)]), "Mute {value} is off");
        assert!(heard(&[(2, 3, value)]), "another Solo {value} is off");
        assert!(
            !heard(&[(2, 3, 1.0), (1, 3, value)]),
            "own Solo {value} is off"
        );
    }
}

#[test]
fn saved_common_params_hold_from_the_first_sample() {
    // A saved Post Gain of 0 (−20 dB), Mix 0.5 and Wet Pan 0 (all left) on the instrument, from sample 0: no
    // glide from the defaults.
    let plain = held_note(&minisynth_song(), 4096);
    let mut song = minisynth_song();
    song.channels[1].rack.get_mut(2).unwrap().common.0 = [0.0, 0.5, 0.0];
    let saved = held_note(&song, 4096);
    let (plain_l, saved_l) = (side(&plain, false), side(&saved, false));
    assert!(energy(&plain_l) > 0.01);
    for (i, (a, b)) in plain_l.iter().zip(&saved_l).enumerate() {
        assert!((a * 0.05 - b).abs() <= 1e-6, "{i}: {a} {b}");
    }
    assert_eq!(energy(&side(&saved, true)), 0.0, "the right is silent");
}

/// The Distortion effect's type id.
const DISTORTION: u16 = 6;

/// A song whose channel 1 rack is `order`, top to bottom, of: `a` a MiniSynth hearing only key 60, `b` one hearing
/// only key 72, `d` a Distortion at its defaults.
fn rack(order: &str) -> Song {
    let mut song = minisynth_song();
    let rack = &mut song.channels[1].rack;
    rack.modules.clear();
    for name in order.chars() {
        let id = rack.add(if name == 'd' { DISTORTION } else { 0 });
        if name != 'd' {
            let key = if name == 'a' { 60 } else { 72 };
            let module = rack.get_mut(id).unwrap();
            (module.file.key_low, module.file.key_high) = (key, key);
        }
    }
    song
}

/// `song`'s output over 4096 frames while channel 1 holds `keys` live.
fn holding(song: &Song, keys: &[u8]) -> Vec<f32> {
    let mut engine = SongAudioEngine::new(song, 48_000.0).unwrap();
    for &key in keys {
        engine.note_on(1, key, 1.0);
    }
    let mut output = vec![0.0; 4096 * 2];
    engine.render(&mut output, 2);
    output
}

#[test]
fn an_effect_above_an_instrument_does_not_process_it() {
    // An instrument adds at its own rack position.
    let clean = holding(&rack("a"), &[60]);
    assert!(energy(&clean) > 0.01);
    assert!(
        holding(&rack("da"), &[60]) == clean,
        "Distortion above: clean"
    );
    assert!(
        holding(&rack("ad"), &[60]) != clean,
        "Distortion below: processed"
    );
}

#[test]
fn an_effect_below_processes_every_instrument_above_it() {
    // Each one alone comes out as it does through the Distortion on its own...
    assert!(holding(&rack("abd"), &[60]) == holding(&rack("ad"), &[60]));
    assert!(holding(&rack("abd"), &[72]) == holding(&rack("bd"), &[72]));
    // ...and together it distorts their sum: not one distorted plus the other clean.
    let both = |order| holding(&rack(order), &[60, 72]);
    let sum = both("abd");
    assert!(sum != both("adb"), "not D(a) + b");
    assert!(sum != both("bda"), "not D(b) + a");
    // In [a, D, b] the Distortion processes only a (as in [MiniSynth, Reverb, GMSynth, Delay]).
    let mut alone = holding(&rack("ad"), &[60]);
    let clean = holding(&rack("b"), &[72]);
    let mixed = both("adb");
    alone.iter_mut().zip(&clean).for_each(|(a, b)| *a += b);
    let error = mixed
        .iter()
        .zip(&alone)
        .map(|(m, a)| (m - a).abs())
        .fold(0.0, f32::max);
    assert!(error < 1e-5, "[a, D, b] = D(a) + b: {error}");
}
