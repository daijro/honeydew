//! Drum kits through the engine, driven through `AudioRenderer` with samples
//! the tests make.

use std::f32::consts::TAU;

use honeydew::audio_engine::{EngineError, SongAudioEngine};
use honeydew::flm::chunk::ContentPath;
use honeydew::sample_cache::SampleCache;
use honeydew::song::{AutomationTarget, ChannelKind, DrumGrid, Event, LaneKind, SamplerSlot, Song};
use honeydew_audio::device::AudioRenderer;

const RATE: f32 = 44_100.0;
/// Frames per beat at 120 BPM.
const BEAT: usize = 22_050;

/// A 16-bit mono PCM WAV at 44.1 kHz.
fn wav(samples: &[f32]) -> Vec<u8> {
    let data: Vec<u8> = (samples.iter())
        .flat_map(|s| ((s * 32768.0) as i16).to_le_bytes())
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

/// The test samples by name: `dc` (0.5 for 4 beats), `short` (0.5 for 100 frames), `sine` (441 Hz, 1 s) and `ramp`
/// (rising over 1000 frames).
fn read(path: &ContentPath) -> Option<Vec<u8>> {
    let ContentPath::Plain(name) = path else {
        return None;
    };
    let samples: Vec<f32> = match name.as_str() {
        "dc" => vec![0.5; 4 * BEAT],
        "short" => vec![0.5; 100],
        "sine" => (0..44_100)
            .map(|i| 0.5 * (TAU * 441.0 * i as f32 / RATE).sin())
            .collect(),
        "ramp" => (0..1000).map(|i| i as f32 / 2000.0).collect(),
        _ => return None,
    };
    Some(wav(&samples))
}

/// A song at 120 BPM with a Drums channel (channel 1, the default kit) whose first pads play `samples`.
fn kit(samples: &[&str]) -> Song {
    let mut song = Song {
        tempo_bpm: 120.0,
        ..Song::default()
    };
    song.add_channel(ChannelKind::Drums);
    for (slot, name) in slots(&mut song).iter_mut().zip(samples) {
        slot.file.path = Some(ContentPath::Plain(name.to_string()));
    }
    song
}

fn slots(song: &mut Song) -> &mut Vec<SamplerSlot> {
    &mut song.channels[1].drum_kit.as_mut().unwrap().slots
}

fn hit(pos: u32, pad: u8) -> Event {
    hit_of(pos, pad, 0.0, Event::DEFAULT_PITCH, 0)
}

/// A full-velocity, centred hit with a length, a per-hit Pitch value and a Repeat count.
fn hit_of(pos: u32, pad: u8, len_beats: f64, value: u16, repeat: u8) -> Event {
    Event::Drum {
        pos,
        len_beats,
        pad,
        velocity: 255,
        pan: 127,
        value,
        repeat,
    }
}

/// A 16-beat clip on the drum lane holding `events`.
fn hits(song: &mut Song, events: impl IntoIterator<Item = Event>) {
    let clip = song.channels[1].lanes[0].add_clip(0, 16.0);
    clip.events.extend(events);
}

fn engine(song: &Song) -> SongAudioEngine {
    let mut cache = SampleCache::default();
    cache.load(song, &read);
    SongAudioEngine::with_samples(song, RATE, &cache).unwrap()
}

/// The next `frames` frames of the left output.
fn render(engine: &mut SongAudioEngine, frames: usize) -> Vec<f32> {
    let mut output = vec![0.0; frames * 2];
    engine.render(&mut output, 2);
    output.chunks(2).map(|frame| frame[0]).collect()
}

/// `song` from its start for `frames` frames.
fn play(song: &Song, frames: usize) -> Vec<f32> {
    let mut engine = engine(song);
    engine.play();
    render(&mut engine, frames)
}

fn energy(samples: &[f32]) -> f32 {
    samples.iter().map(|s| s * s).sum()
}

/// The first frame that sounds.
fn onset(output: &[f32]) -> usize {
    output.iter().position(|s| s.abs() > 1e-6).unwrap()
}

/// The mean period in frames, from the rising zero crossings.
fn period(samples: &[f32]) -> f32 {
    let rising: Vec<usize> = (1..samples.len())
        .filter(|&i| samples[i - 1] < 0.0 && samples[i] >= 0.0)
        .collect();
    (rising[rising.len() - 1] - rising[0]) as f32 / (rising.len() - 1) as f32
}

/// One `dc` pad's level at the output, through the default strips.
fn unit() -> f32 {
    let mut song = kit(&["dc"]);
    hits(&mut song, [hit(0, 0)]);
    let unit = play(&song, 100)[50];
    assert!(unit > 0.4, "{unit}");
    unit
}

fn close(a: f32, b: f32) -> bool {
    (a - b).abs() <= 1e-3 * b.abs().max(1.0)
}

#[test]
fn the_default_kit_plays_eight_distinct_one_shots() {
    let mut song = kit(&[]);
    hits(&mut song, (0..8).map(|pad| hit(u32::from(pad) * 128, pad)));
    let output = play(&song, 8 * BEAT);
    let heads: Vec<&[f32]> = output.chunks(BEAT).map(|beat| &beat[..2000]).collect();
    for (pad, head) in heads.iter().enumerate() {
        assert!(energy(head) > 0.01, "pad {pad} is silent");
    }
    for a in 0..8 {
        for b in a + 1..8 {
            assert_ne!(heads[a], heads[b], "pads {a} and {b}");
        }
    }
    let mut silent = SongAudioEngine::new(&song, RATE).unwrap();
    silent.play();
    assert_eq!(
        energy(&render(&mut silent, BEAT)),
        0.0,
        "no samples, no sound"
    );
}

#[test]
fn pad_notes_and_stored_midi_notes_play_every_slot_on_that_key() {
    let unit = unit();
    let mut song = kit(&["dc", "dc"]);
    let mut one = engine(&song);
    one.note_on(1, 48, 1.0);
    assert!(
        close(render(&mut one, 100)[50], unit),
        "pad 0 plays on 48 + 0"
    );

    slots(&mut song)[1].file.midi_key = 48;
    let mut two = engine(&song);
    two.note_on(1, 48, 1.0);
    assert!(
        close(render(&mut two, 100)[50], 2.0 * unit),
        "the stored note 48 too"
    );

    let lane = song.channels[1].add_lane(LaneKind::Notes);
    song.channels[1].lanes[lane]
        .add_clip(0, 4.0)
        .events
        .push(Event::Note {
            pos: 0,
            len_beats: 1.0,
            key: 48,
            velocity: 255,
            pan: 127,
            slide: false,
        });
    assert!(
        close(play(&song, 100)[50], 2.0 * unit),
        "a stored MIDI note"
    );
}

#[test]
fn cut_group_one_stops_the_open_hat() {
    // Pad 3 (Closed Hat) and pad 4 (Open Hat) share cut group 1 in the default kit.
    let mut song = kit(&["dc", "dc", "dc", "short", "dc"]);
    hits(&mut song, [hit(0, 4), hit(128, 3)]);
    let choked = play(&song, 2 * BEAT);
    assert!(choked[BEAT - 10].abs() > 0.4);
    assert!(choked[BEAT + 2000..].iter().all(|&s| s == 0.0));

    slots(&mut song)[4].cut_group = 0;
    assert!(play(&song, 2 * BEAT)[BEAT + 2000] > 0.4);
}

#[test]
fn overlap_off_cuts_the_slots_own_voice() {
    let unit = unit();
    let mut song = kit(&["dc"]);
    hits(&mut song, [hit(0, 0), hit(64, 0)]);
    assert!(close(play(&song, BEAT)[BEAT - 1], 2.0 * unit));
    slots(&mut song)[0].file.overlapping_voices = false;
    assert!(close(play(&song, BEAT)[BEAT - 1], unit));
}

#[test]
fn speed_hit_pitch_and_pitch_bend_scale_the_period() {
    let mut song = kit(&["sine"]);
    hits(&mut song, [hit(0, 0)]);
    assert!((period(&play(&song, 4000)) - 100.0).abs() < 0.2);

    let mut speed = song.clone();
    slots(&mut speed)[0].params[2] = 36.0 / 48.0;
    assert!(
        (period(&play(&speed, 4000)) - 50.0).abs() < 0.2,
        "Speed +12"
    );

    let mut pitched = kit(&["sine"]);
    hits(&mut pitched, [hit_of(0, 0, 0.0, 65535, 0)]);
    assert!(
        (period(&play(&pitched, 4000)) - 50.0).abs() < 0.2,
        "hit Pitch"
    );

    let mut bent = engine(&song);
    bent.set_param(1, 0, 4, 0.75);
    bent.play();
    assert!(
        (period(&render(&mut bent, 4000)) - 50.0).abs() < 0.2,
        "Pitch Bend 2^w"
    );
}

#[test]
fn reverse_plays_the_slot_backwards() {
    let mut song = kit(&["ramp"]);
    hits(&mut song, [hit(0, 0)]);
    slots(&mut song)[0].file.reverse = true;
    let output = play(&song, 1000);
    assert!(output[0] > 0.49 && output.windows(2).all(|w| w[1] <= w[0]));
}

#[test]
fn a_negative_sample_start_delays_the_hit() {
    // v = 0.25: s = -0.125 s, 5512.5 frames of source silence first.
    let mut song = kit(&["dc"]);
    hits(&mut song, [hit(0, 0)]);
    slots(&mut song)[0].params[10] = 0.25;
    assert_eq!(onset(&play(&song, BEAT)), 5513);
}

#[test]
fn negative_instances_set_slot_params_live_and_from_automation() {
    let unit = unit();
    let mut song = kit(&["dc", "dc"]);
    hits(&mut song, [hit(0, 0), hit(0, 1)]);
    let second = -(slots(&mut song)[1].file.id as i32);
    let mut engine = engine(&song);
    engine.play();
    assert!(close(render(&mut engine, 100)[50], 2.0 * unit));
    engine.set_param(1, second, 4, 1.0); // Solo
    assert!(
        close(render(&mut engine, 100)[50], unit),
        "solo silences the other pad"
    );
    engine.set_param(1, second, 3, 1.0); // Mute
    assert_eq!(render(&mut engine, 100)[50], 0.0);
    engine.seek(0);
    engine.play();
    assert_eq!(
        render(&mut engine, 100)[50],
        0.0,
        "a live value survives a seek"
    );

    let first = -(slots(&mut song)[0].file.id as i32);
    let lane = song.channels[1].add_automation_lane(AutomationTarget {
        instance_id: first,
        param: 0,
    });
    lane.add_clip(0, 4.0).events.push(Event::Automation {
        pos: 0,
        value: 0.0,
        control: true,
    });
    // Pad 0 at Level 0 plays at -60 dB next to pad 1 at unity.
    let quiet = play(&song, 100)[50] - unit;
    assert!((quiet / (unit * 0.001) - 1.0).abs() < 0.05, "{quiet}");
}

#[test]
fn a_swing_lane_plays_its_stored_ticks() {
    // Swing moved the hits when it was set; playback must not move them again.
    let mut song = kit(&["dc"]);
    hits(&mut song, [hit(32, 0)]);
    song.channels[1].lanes[0].swing = 0.45;
    assert_eq!(onset(&play(&song, BEAT)), BEAT / 4 + 1);
}

#[test]
fn the_beat_offset_shifts_the_slots_hits() {
    let mut song = kit(&["dc"]);
    hits(&mut song, [hit(0, 0)]);
    slots(&mut song)[0].file.beat_offset = 0.25;
    assert_eq!(onset(&play(&song, BEAT)), BEAT / 4 + 1);
    slots(&mut song)[0].file.beat_offset = -0.25;
    assert_eq!(onset(&play(&song, BEAT)), 0, "clamped at 0");
}

#[test]
fn a_notes_lane_hit_ends_at_its_length_or_the_clip_end() {
    let mut song = kit(&["dc"]);
    slots(&mut song)[0].params[9] = 0.0; // EG R: instant
    hits(&mut song, [hit_of(0, 0, 0.5, Event::DEFAULT_PITCH, 0)]);
    assert!(
        play(&song, BEAT)[BEAT / 2 + 100] > 0.4,
        "a step lane ignores it"
    );
    song.channels[1].lanes[0].grid = DrumGrid::Notes;
    let output = play(&song, BEAT);
    assert!(output[BEAT / 2 - 10] > 0.4 && output[BEAT / 2 + 10..].iter().all(|&s| s == 0.0));

    let mut song = kit(&["dc"]);
    slots(&mut song)[0].params[9] = 0.0;
    song.channels[1].lanes[0].grid = DrumGrid::Notes;
    let clip = song.channels[1].lanes[0].add_clip(0, 1.0);
    clip.events.push(hit_of(0, 0, 3.0, Event::DEFAULT_PITCH, 0));
    let output = play(&song, 2 * BEAT);
    assert!(output[BEAT - 10] > 0.4 && output[BEAT + 10..].iter().all(|&s| s == 0.0));
}

#[test]
fn repeat_adds_hits_across_a_sixteenth() {
    let mut song = kit(&["short"]);
    hits(&mut song, [hit_of(0, 0, 0.0, Event::DEFAULT_PITCH, 3)]);
    let onsets = |output: Vec<f32>| -> Vec<usize> {
        (0..output.len())
            .filter(|&i| output[i] != 0.0 && (i == 0 || output[i - 1] == 0.0))
            .collect()
    };
    // pos + 0.25·k/4 beats: 1378.125 frames apart.
    assert_eq!(onsets(play(&song, BEAT)), [0, 1379, 2757, 4135]);
    // Swing 0.4 on an even sixteenth stretches it to 0.3 beat.
    song.channels[1].lanes[0].swing = 0.4;
    assert_eq!(onsets(play(&song, BEAT)), [0, 1654, 3308, 4962]);
}

#[test]
fn a_send_target_plays_only_into_the_target_channel() {
    let unit = unit();
    let mut song = kit(&["dc"]);
    song.add_channel(ChannelKind::EffectTrack);
    hits(&mut song, [hit(0, 0)]);
    slots(&mut song)[0].file.send_target = song.channels[2].id;
    song.channels[1].params.0[0] = 0.0;
    assert!(
        close(play(&song, 100)[50], unit),
        "the drum channel's Level is bypassed"
    );
    song.channels[2].params.0[0] = 0.0;
    assert_eq!(
        play(&song, 100)[50],
        0.0,
        "the target channel's strip applies"
    );
}

#[test]
fn a_pad_send_bypasses_the_drum_rack_and_the_drum_channels_sends() {
    // After its own Level, Pan, Mute/Solo and envelope the pad goes only
    // into the target's input, then through the target's rack.
    let unit = unit();
    let mut song = kit(&["dc"]);
    song.add_channel(ChannelKind::EffectTrack);
    hits(&mut song, [hit(0, 0)]);
    slots(&mut song)[0].file.send_target = song.channels[2].id;
    let effect = song.channels[1].rack.add(20);
    song.channels[1].rack.get_mut(effect).unwrap().params[1] = 0.0;
    song.channels[1].file.sends.clear();
    assert!(
        close(play(&song, 100)[50], unit),
        "neither the drum rack nor the drum channel's sends apply"
    );
    let effect = song.channels[2].rack.add(20);
    song.channels[2].rack.get_mut(effect).unwrap().params[1] = 0.0;
    let dry = energy(&play(&song, 8192));
    assert!(
        dry < 0.5 * unit * unit * 8192.0,
        "the target's rack applies"
    );
}

/// The peak of channel `channel`'s strip meter over the last render.
fn strip_peak(engine: &SongAudioEngine, channel: usize) -> f32 {
    let index = (0..engine.meter_count())
        .find(|&i| engine.meter_key(i).channel == channel && engine.meter_key(i).instance_id == 0)
        .unwrap();
    engine.meter(index).peak_left
}

/// A pad's send target makes its drum channel a source of the target, so soloing the target
/// keeps the drum channel audible; an unrelated channel goes silent.
#[test]
fn soloing_a_pads_send_target_keeps_the_drum_channel_audible() {
    let mut song = kit(&["dc", "dc"]);
    song.add_channel(ChannelKind::EffectTrack);
    song.add_channel(ChannelKind::Synth(honeydew::song::SynthKind::MiniSynth));
    slots(&mut song)[0].file.send_target = song.channels[2].id;
    hits(&mut song, [hit(0, 0), hit(0, 1)]);
    let mut engine = engine(&song);
    engine.set_param(2, 0, 3, 1.0);
    engine.note_on(3, 69, 1.0);
    engine.play();
    render(&mut engine, 1000);
    assert!(strip_peak(&engine, 2) > 0.1, "the soloed target");
    assert!(strip_peak(&engine, 1) > 0.1, "the drum channel's own pad");
    assert_eq!(strip_peak(&engine, 3), 0.0, "an unrelated channel");
}

#[test]
fn the_drum_rack_processes_the_pads() {
    let mut song = kit(&["dc"]);
    hits(&mut song, [hit(0, 0)]);
    let dry = energy(&play(&song, 8192));
    let effect = song.channels[1].rack.add(20);
    song.channels[1].rack.get_mut(effect).unwrap().params[1] = 0.0;
    let wet = energy(&play(&song, 8192));
    assert!(dry > 1.0 && wet < 0.5 * dry, "{dry} {wet}");
}

#[test]
fn a_muted_clip_plays_no_hits() {
    let mut song = kit(&["dc"]);
    hits(&mut song, [hit(0, 0)]);
    song.channels[1].lanes[0].clips[0].muted = true;
    assert_eq!(energy(&play(&song, BEAT)), 0.0);
}

#[test]
fn stop_and_all_notes_off_release_the_drum_voices() {
    let mut song = kit(&["dc"]);
    slots(&mut song)[0].params[9] = 0.1; // EG R: tau 2.5 ms
    hits(&mut song, [hit(0, 0)]);
    let mut engine = engine(&song);
    engine.play();
    render(&mut engine, 1000);
    engine.stop();
    let output = render(&mut engine, 2000);
    assert!(output[0] > 0.4 && output[1999] == 0.0);

    engine.note_on(1, 48, 1.0);
    render(&mut engine, 100);
    engine.all_notes_off();
    let output = render(&mut engine, 2000);
    assert!(output[0] > 0.4 && output[1999] == 0.0, "AllNotesOff too");
}

#[test]
fn repeat_hits_count_against_the_play_budget() {
    // 400 000 plays of a one-tick loop fit the budget (about 2.8 M); with Repeat 8 each plays 9 hits, which do not.
    let mut song = kit(&[]);
    let clip = song.channels[1].lanes[0].add_clip(0, 1.0 / 128.0);
    clip.visible_len_beats = 400_000.0 / 128.0;
    clip.events.push(hit_of(0, 0, 0.0, Event::DEFAULT_PITCH, 8));
    assert!(matches!(
        SongAudioEngine::new(&song, RATE),
        Err(EngineError::TooManyEvents)
    ));
}

#[test]
fn the_sustain_pedal_holds_a_pads_note_off() {
    let mut song = kit(&["dc"]);
    slots(&mut song)[0].params[9] = 0.0; // EG R: instant
    let mut engine = engine(&song);
    engine.set_param(1, 0, 71, 1.0); // CC #64 down
    engine.note_on(1, 48, 1.0);
    render(&mut engine, 100);
    engine.note_off(1, 48);
    assert!(render(&mut engine, 100)[50] > 0.4, "held");
    engine.set_param(1, 0, 71, 0.0);
    assert_eq!(
        render(&mut engine, 100)[50],
        0.0,
        "released when the pedal lifts"
    );
}

/// The next `frames` frames, left and right.
fn render_stereo(engine: &mut SongAudioEngine, frames: usize) -> (Vec<f32>, Vec<f32>) {
    let mut output = vec![0.0; frames * 2];
    engine.render(&mut output, 2);
    output.chunks(2).map(|frame| (frame[0], frame[1])).unzip()
}

#[test]
fn hostile_slot_params_play_as_the_defaults() {
    let unit = unit();
    for bad in [f32::NAN, -1.0, 2.0, f32::INFINITY] {
        let mut song = kit(&["dc"]);
        hits(&mut song, [hit(0, 0)]);
        slots(&mut song)[0].params = [bad; 11];
        let output = play(&song, 100);
        assert!(output.iter().all(|s| s.is_finite()), "{bad}");
        assert!(close(output[50], unit), "{bad}: {}", output[50]);
    }
}

#[test]
fn pads_get_the_notes_before_the_note_effects() {
    // Transpose +1 octave in the drum rack: key 48 still plays pad 0, not the pad on 60.
    let mut song = kit(&["dc", "short"]);
    slots(&mut song)[1].file.midi_key = 60;
    let transpose = song.channels[1].rack.add(48);
    song.channels[1].rack.get_mut(transpose).unwrap().params[1] = 5.0 / 8.0;
    let lane = song.channels[1].add_lane(LaneKind::Notes);
    song.channels[1].lanes[lane]
        .add_clip(0, 4.0)
        .events
        .push(Event::Note {
            pos: 0,
            len_beats: 1.0,
            key: 48,
            velocity: 255,
            pan: 127,
            slide: false,
        });
    assert!(play(&song, 1000)[500] > 0.4);
}

#[test]
fn hit_pan_is_b_over_255() {
    let pan = |byte: u8| {
        let mut song = kit(&["dc"]);
        hits(
            &mut song,
            [Event::Drum {
                pos: 0,
                len_beats: 0.0,
                pad: 0,
                velocity: 255,
                pan: byte,
                value: Event::DEFAULT_PITCH,
                repeat: 0,
            }],
        );
        let mut engine = engine(&song);
        engine.play();
        let (l, r) = render_stereo(&mut engine, 100);
        (l[50], r[50])
    };
    let (l, r) = pan(127);
    assert!(
        (r / l - 254.0 / 255.0).abs() < 1e-5,
        "127 is slightly left: {l} {r}"
    );
    assert_eq!(pan(0).1, 0.0);
    assert_eq!(pan(255).0, 0.0);
}

#[test]
fn the_hit_tick_is_truncated() {
    // An offset of 1.5 ticks moves a hit at tick 0 to tick 1: frame ceil(22050 / 128) = 173.
    let mut song = kit(&["dc"]);
    hits(&mut song, [hit(0, 0)]);
    slots(&mut song)[0].file.beat_offset = 1.5 / 128.0;
    assert_eq!(onset(&play(&song, BEAT)), 173);
}

/// Where each separate run of sound starts.
fn onsets(output: &[f32]) -> Vec<usize> {
    (0..output.len())
        .filter(|&i| output[i] != 0.0 && (i == 0 || output[i - 1] == 0.0))
        .collect()
}

#[test]
fn repeat_is_signed_ignores_the_offset_and_has_no_swing_on_notes_lanes() {
    let mut song = kit(&["short"]);
    hits(&mut song, [hit_of(0, 0, 0.0, Event::DEFAULT_PITCH, 255)]);
    let output = play(&song, BEAT);
    assert_eq!(onsets(&output), [0], "255 is -1: no repeat");
    assert!(output[100..].iter().all(|&s| s == 0.0), "not 255 repeats");

    // The hit moves by the offset (0.25 beat); its repeat stays at pos + 0.125.
    let mut song = kit(&["short"]);
    hits(&mut song, [hit_of(0, 0, 0.0, Event::DEFAULT_PITCH, 1)]);
    slots(&mut song)[0].file.beat_offset = 0.25;
    assert_eq!(onsets(&play(&song, BEAT)), [2757, 5513]);

    let mut song = kit(&["short"]);
    hits(&mut song, [hit_of(0, 0, 0.0, Event::DEFAULT_PITCH, 3)]);
    song.channels[1].lanes[0].swing = 0.4;
    song.channels[1].lanes[0].grid = DrumGrid::Notes;
    assert_eq!(onsets(&play(&song, BEAT)), [0, 1379, 2757, 4135]);

    // On an odd sixteenth (pos 0.25 beat) swing 0.4 shortens it to 0.2 beat: 0.05-beat steps from frame 5513.
    let mut song = kit(&["short"]);
    hits(&mut song, [hit_of(32, 0, 0.0, Event::DEFAULT_PITCH, 3)]);
    song.channels[1].lanes[0].swing = 0.4;
    assert_eq!(onsets(&play(&song, BEAT)), [5513, 6615, 7718, 8820]);
}

#[test]
fn a_note_off_counts_from_the_hits_position_before_its_offset() {
    // Offset 0.5 and length 0.25 put the note-off (0.25) before the hit (0.5), so the hit plays on.
    let mut song = kit(&["dc"]);
    slots(&mut song)[0].params[9] = 0.0;
    slots(&mut song)[0].file.beat_offset = 0.5;
    song.channels[1].lanes[0].grid = DrumGrid::Notes;
    hits(&mut song, [hit_of(0, 0, 0.25, Event::DEFAULT_PITCH, 0)]);
    let output = play(&song, 2 * BEAT);
    assert_eq!(onset(&output), BEAT / 2);
    assert!(output[BEAT + BEAT / 2] > 0.4);
}

#[test]
fn a_note_off_comes_before_a_hit_on_the_same_frame() {
    let mut song = kit(&["dc"]);
    slots(&mut song)[0].params[9] = 0.0;
    song.channels[1].lanes[0].grid = DrumGrid::Notes;
    hits(
        &mut song,
        [
            hit_of(0, 0, 0.5, Event::DEFAULT_PITCH, 0),
            hit_of(64, 0, 0.0, Event::DEFAULT_PITCH, 0),
        ],
    );
    let unit = unit();
    let output = play(&song, BEAT);
    assert!(
        close(output[BEAT / 2 + 100], unit),
        "{}",
        output[BEAT / 2 + 100]
    );
}

#[test]
fn a_huge_beat_offset_still_builds() {
    let mut song = kit(&["dc"]);
    hits(&mut song, [hit(0, 0)]);
    slots(&mut song)[0].file.beat_offset = 1e300;
    assert_eq!(energy(&play(&song, BEAT)), 0.0);
    slots(&mut song)[0].file.beat_offset = -1e300;
    assert_eq!(onset(&play(&song, BEAT)), 0);
}

#[test]
fn str1_stretch_mode_ratio_and_pitch_reach_the_pad() {
    let mut song = kit(&["dc"]);
    hits(&mut song, [hit(0, 0)]);
    slots(&mut song)[0].file.stretch_ratio = 2.0;
    // Mode 0: the ratio is ignored, the 4-beat sample ends at 4 beats.
    assert_eq!(play(&song, 5 * BEAT)[4 * BEAT + 100], 0.0);
    slots(&mut song)[0].file.stretch_mode = 1;
    assert!(play(&song, 5 * BEAT)[4 * BEAT + 100] > 0.4, "twice as long");

    let mut song = kit(&["sine"]);
    hits(&mut song, [hit(0, 0)]);
    slots(&mut song)[0].file.pitch_semitones = 12.0;
    let output = play(&song, 20_000);
    assert!((period(&output[5000..15_000]) - 50.0).abs() < 0.5);
}

/// On a step lane every earlier hit in the clip covers its pad, so a
/// seek inside the clip leaves the pad sounding (no cut, no restart).
#[test]
fn a_seek_inside_a_step_clip_leaves_its_hit_pads_sounding() {
    let mut song = kit(&["dc"]);
    hits(&mut song, [hit(0, 0)]);
    let mut engine = engine(&song);
    engine.play();
    render(&mut engine, 1000);
    engine.seek(2 * 128);
    let unit = unit();
    assert!(render(&mut engine, 1000).iter().all(|&s| close(s, unit)));
}

#[test]
fn seek_restores_the_saved_slot_params() {
    // Slot Level automation from beat 2 sets -60 dB; a seek back to beat 1 restores the saved Level, at which the pad
    // its step hit at beat 0 covers sounds on. A seek back to beat 0 restores it too: the hit there strikes again at
    // the saved Level.
    let unit = unit();
    let mut song = kit(&["dc"]);
    hits(&mut song, [hit(0, 0)]);
    let id = -(slots(&mut song)[0].file.id as i32);
    let lane = song.channels[1].add_automation_lane(AutomationTarget {
        instance_id: id,
        param: 0,
    });
    lane.add_clip(256, 2.0).events.push(Event::Automation {
        pos: 0,
        value: 0.0,
        control: true,
    });
    let sought = |to: u64| {
        let mut engine = engine(&song);
        engine.play();
        render(&mut engine, 3 * BEAT);
        engine.seek(to);
        render(&mut engine, 100)[50]
    };
    assert!(close(sought(128), unit));
    let restruck = sought(0);
    assert!(restruck >= 0.99 * unit, "{restruck} vs {unit}");
}

/// Each slot's own output peak, keyed (channel, −slot id) as its params are, for the drum strips' meters.
#[test]
fn each_slot_publishes_its_own_peak_meter_under_its_negative_instance() {
    let mut song = kit(&["dc", "short"]);
    hits(&mut song, [hit(0, 0)]);
    let ids: Vec<i32> = (slots(&mut song).iter())
        .map(|s| -(s.file.id as i32))
        .collect();
    let mut engine = engine(&song);
    engine.play();
    render(&mut engine, 100);
    let meter = |id: i32| {
        let key = honeydew_audio::device::ModuleKey {
            channel: 1,
            instance_id: id,
        };
        let index = (0..engine.meter_count()).find(|&i| engine.meter_key(i) == key);
        engine.meter(index.expect("a meter for every slot"))
    };
    let played = meter(ids[0]);
    assert!(played.peak_left > 0.4, "{played:?}");
    // Hit pan 127/255 sits a hair left of centre.
    assert!(
        (played.peak_left - played.peak_right).abs() < 0.01,
        "{played:?}"
    );
    for &id in &ids[1..] {
        assert_eq!(meter(id).peak_left, 0.0, "silent pads read 0");
    }
}

#[test]
fn rev_and_poly_reach_the_playing_kit_as_settings() {
    // `SetSetting` on instance −slot id, key 1036 (`MAIN`
    // reverse) or 1050 (overlap), with no new engine.
    let mut song = kit(&["ramp", "dc"]);
    let (ramp, dc) = (slots(&mut song)[0].file.id, slots(&mut song)[1].file.id);
    let mut engine = engine(&song);
    engine.set_setting(1, -(ramp as i32), 1036, 1.0);
    engine.note_on(1, 48, 1.0);
    let out = render(&mut engine, 1000);
    assert!(
        out[10] > out[500] && out[500] > out[900],
        "falls when reversed"
    );
    // Overlap off: a second live hit fades the first voice out.
    let single = {
        let mut engine = self::engine(&song);
        engine.note_on(1, 49, 1.0);
        render(&mut engine, 4000)[3999]
    };
    engine.set_setting(1, -(dc as i32), 1050, 0.0);
    engine.note_on(1, 49, 1.0);
    render(&mut engine, 100);
    engine.note_on(1, 49, 1.0);
    let out = render(&mut engine, 3900);
    assert!(
        (out[3899] - single).abs() < 1e-3,
        "{} vs {single}",
        out[3899]
    );
}

#[test]
fn a_live_pad_hit_ignores_the_beat_offset() {
    // `Shift` moves only the sequenced hits.
    let mut song = kit(&["dc"]);
    slots(&mut song)[0].file.beat_offset = 0.25;
    let mut engine = engine(&song);
    engine.note_on(1, 48, 1.0);
    assert_eq!(onset(&render(&mut engine, BEAT)), 0);
}

#[test]
fn a_cut_group_reaches_the_playing_kit_as_a_setting() {
    // The strip's `Cut...` (MAIN@1049) goes to the engine live, key 1049, with no new engine: pads 0 and 1 in group A
    // choke each other.
    let mut song = kit(&["dc", "dc"]);
    let ids: Vec<i32> = slots(&mut song)
        .iter()
        .map(|s| -(s.file.id as i32))
        .collect();
    let single = {
        let mut engine = engine(&song);
        engine.note_on(1, 49, 1.0);
        render(&mut engine, 4000)[3999]
    };
    let mut engine = engine(&song);
    for &id in &ids[..2] {
        engine.set_setting(1, id, 1049, 1.0);
    }
    engine.note_on(1, 48, 1.0);
    render(&mut engine, 100);
    engine.note_on(1, 49, 1.0);
    let out = render(&mut engine, 3900);
    assert!(
        (out[3899] - single).abs() < 1e-3,
        "{} vs {single}",
        out[3899]
    );
    // Back to no group: pad 0's new voice outlives pad 1's next hit (pad 1 then plays two voices, pad 0 one).
    engine.set_setting(1, ids[0], 1049, 0.0);
    engine.set_setting(1, ids[1], 1049, 0.0);
    engine.note_on(1, 48, 1.0);
    render(&mut engine, 100);
    engine.note_on(1, 49, 1.0);
    let out = render(&mut engine, 3900);
    assert!(
        (out[3899] - 3.0 * single).abs() < 1e-3,
        "{} vs {single}",
        out[3899]
    );
}
