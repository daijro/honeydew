//! The engine's note path: live keys through the note effects, slide
//! notes and Mute/Unmute re-syncing a playing song (`CLHd`@36). Rendered offline through
//! `AudioRenderer`; no device.

use honeydew::audio_engine::SongAudioEngine;
use honeydew::flm::chunk::ContentPath;
use honeydew::sample_cache::SampleCache;
use honeydew::song::{
    AutomationTarget, ChannelKind, DrumGrid, Event, LaneKind, SamplerSlot, Song, SynthKind,
};
use honeydew_audio::device::AudioRenderer;

/// One beat at 120 BPM and 48 kHz.
const BEAT: usize = 24_000;

fn energy(samples: &[f32]) -> f32 {
    samples.iter().map(|sample| sample * sample).sum()
}

fn left(output: &[f32]) -> Vec<f32> {
    output.iter().step_by(2).copied().collect()
}

/// 120 BPM, a MiniSynth channel (channel 1, its synth instance 2).
fn synth_song() -> Song {
    let mut song = Song {
        tempo_bpm: 120.0,
        ..Song::default()
    };
    song.add_channel(ChannelKind::Synth(SynthKind::MiniSynth));
    song
}

/// Module `type_id` inserted at rack position `at` of channel 1 with `params` set; its instance id.
fn insert(song: &mut Song, at: usize, type_id: u16, params: &[(usize, f32)]) -> i32 {
    let id = song.channels[1].rack.insert(at, type_id);
    let module = song.channels[1].rack.get_mut(id).unwrap();
    for &(index, value) in params {
        module.params[index] = value;
    }
    id
}

/// Transpose a whole octave up.
const OCTAVE_UP: [(usize, f32); 1] = [(1, 5.0 / 8.0)];

/// `song` stopped, `setup` run, then each of `keys` held on channel 1 from the keyboard at `velocity` for `frames`.
fn live(
    song: &Song,
    keys: &[u8],
    frames: usize,
    setup: impl FnOnce(&mut SongAudioEngine),
) -> Vec<f32> {
    let mut engine = SongAudioEngine::new(song, 48_000.0).unwrap();
    setup(&mut engine);
    for &key in keys {
        engine.note_on(1, key, 1.0);
    }
    let mut output = vec![0.0; frames * 2];
    engine.render(&mut output, 2);
    output
}

fn assert_close(actual: &[f32], expected: &[f32]) {
    assert!(energy(expected) > 0.01);
    for (frame, (a, e)) in actual.iter().zip(expected).enumerate() {
        assert!((a - e).abs() <= 1e-5, "frame {}: {a} vs {e}", frame / 2);
    }
}

/// On a notes lane a live key takes the sequenced path, so each
/// instrument gets it through the note effects above it.
#[test]
fn a_live_key_on_a_notes_lane_goes_through_the_note_effects_above_each_instrument() {
    let mut song = synth_song();
    // [MiniSynth, Transpose +12, MiniSynth]
    insert(&mut song, 1, 48, &OCTAVE_UP);
    song.channels[1].rack.add(0);
    let plain = synth_song();
    let expected: Vec<f32> = (live(&plain, &[60], 4096, |_| {}).iter())
        .zip(&live(&plain, &[72], 4096, |_| {}))
        .map(|(a, b)| a + b)
        .collect();
    assert_close(&live(&song, &[60], 4096, |_| {}), &expected);
}

/// Off a notes lane a live key goes once, after every note effect, to every instrument.
#[test]
fn off_a_notes_lane_a_live_key_goes_once_after_every_note_effect() {
    let mut song = synth_song();
    insert(&mut song, 1, 48, &OCTAVE_UP);
    song.channels[1].rack.add(0);
    let expected: Vec<f32> = (live(&synth_song(), &[72], 4096, |_| {}).iter())
        .map(|sample| 2.0 * sample)
        .collect();
    let off_lane = live(&song, &[60], 4096, |engine| engine.set_live_lane(1, false));
    assert_close(&off_lane, &expected);
    // Back on the notes lane, the next key takes the sequenced path again.
    let on_lane = live(&song, &[60], 4096, |engine| {
        engine.set_live_lane(1, false);
        engine.set_live_lane(1, true);
    });
    assert!(on_lane != off_lane);
}

/// Live keys play what the same notes play sequenced, through Note Echo and the Arpeggiator (they are routed exactly
/// like sequenced notes). The left side only: a live key is centred, a note's pan byte 127 is not.
#[test]
fn live_keys_play_as_the_same_notes_sequenced_through_echo_and_the_arpeggiator() {
    let mut song = synth_song();
    // Arpeggiator Up at 1/8, then one echo a quarter beat later (Sync on, no Delay Rand.).
    let echo = ((0.25 - 1.0 / 32.0) / (8.0 - 1.0 / 32.0_f64)).sqrt() as f32;
    insert(
        &mut song,
        0,
        47,
        &[(1, 1.0 / 32.0), (3, echo), (4, 1.0), (11, 0.0)],
    );
    insert(&mut song, 0, 51, &[(1, 2.0 / 7.0), (10, 2.0 / 23.0)]);
    let mut sequenced = song.clone();
    let clip = sequenced.channels[1].lanes[0].add_clip(0, 8.0);
    for key in [60, 64] {
        clip.events.push(Event::Note {
            pos: 0,
            len_beats: 2.0,
            key,
            velocity: 204,
            pan: 127,
            slide: false,
        });
    }
    let mut engine = SongAudioEngine::new(&sequenced, 48_000.0).unwrap();
    engine.play();
    let mut expected = vec![0.0; 4 * BEAT * 2];
    engine.render(&mut expected, 2);

    let mut engine = SongAudioEngine::new(&song, 48_000.0).unwrap();
    for key in [60, 64] {
        engine.note_on(1, key, 204.0 / 255.0);
    }
    let mut actual = vec![0.0; 4 * BEAT * 2];
    // Odd blocks, the keys lifting at beat 2 between two of them.
    let mut at = 0;
    for frames in [1, 333, 1024, 4096].into_iter().cycle() {
        let frames = frames.min(4 * BEAT - at);
        if at == 2 * BEAT {
            engine.note_off(1, 60);
            engine.note_off(1, 64);
        }
        let frames = if at < 2 * BEAT {
            frames.min(2 * BEAT - at)
        } else {
            frames
        };
        engine.render(&mut actual[at * 2..(at + frames) * 2], 2);
        at += frames;
        if at == 4 * BEAT {
            break;
        }
    }
    let (actual, expected) = (left(&actual), left(&expected));
    assert!(energy(&expected[3 * BEAT / 2..2 * BEAT]) > 0.01);
    for (frame, (a, e)) in actual.iter().zip(&expected).enumerate() {
        assert!((a - e).abs() <= 1e-5, "frame {frame}: {a} vs {e}");
    }
}

fn note(beat: f64, len_beats: f64, key: u8, slide: bool) -> Event {
    Event::Note {
        pos: (beat * 128.0) as u32,
        len_beats,
        key,
        velocity: 204,
        pan: 127,
        slide,
    }
}

/// `song` playing from tick 0 for `frames` frames.
fn played(song: &Song, frames: usize) -> Vec<f32> {
    let mut engine = SongAudioEngine::new(song, 48_000.0).unwrap();
    engine.play();
    let mut output = vec![0.0; frames * 2];
    engine.render(&mut output, 2);
    output
}

/// Sign changes per frame of the left side over beats [from, to).
fn crossings(output: &[f32], from: f64, to: f64) -> f32 {
    let left = left(output);
    let part = &left[(from * BEAT as f64) as usize..(to * BEAT as f64) as usize];
    let changes = part
        .windows(2)
        .filter(|w| (w[0] < 0.0) != (w[1] < 0.0))
        .count();
    changes as f32 / part.len() as f32
}

/// The sequencer sends no note-on or note-off for a slide note (EVN2 @19 = 1), and one with
/// no anchor does nothing.
#[test]
fn a_slide_note_sounds_no_note_of_its_own() {
    let mut song = synth_song();
    let clip = song.channels[1].lanes[0].add_clip(0, 8.0);
    clip.events.push(note(0.0, 2.0, 60, true));
    assert_eq!(energy(&played(&song, 3 * BEAT)), 0.0);
    let events = honeydew::audio_bridge::SongAudioBridge::note_events_in_block(&song, 1, 0.0, 8.0);
    assert_eq!(events.unwrap(), []);
}

/// An anchor A3 held four beats; a slide note to A4 from beat 1 for one beat glides it, in linear steps, an
/// octave up, and the pitch stays there after the slide.
#[test]
fn a_slide_glides_the_anchor_to_its_key_and_the_pitch_stays() {
    let mut song = synth_song();
    let clip = song.channels[1].lanes[0].add_clip(0, 8.0);
    clip.events
        .extend([note(0.0, 4.0, 57, false), note(1.0, 1.0, 69, true)]);
    let output = played(&song, 4 * BEAT);
    let before = crossings(&output, 0.25, 0.75);
    // Halfway through the slide: six semitones up, ±0.4.
    let halfway = crossings(&output, 1.45, 1.55) / before;
    assert!((halfway - 2f32.sqrt()).abs() < 0.05, "{halfway}");
    let after = crossings(&output, 2.5, 3.5) / before;
    assert!((after - 2.0).abs() < 0.05, "{after}");
}

/// The offsets go to every instrument of the rack, whatever note effects sit above it. With an
/// octave up above the second MiniSynth, each glides its own anchor by the same 12 semitones.
#[test]
fn a_slide_reaches_every_instrument_regardless_of_its_rack_position() {
    let song = |keys: [u8; 2], transpose: bool| {
        let mut song = synth_song();
        if transpose {
            insert(&mut song, 1, 48, &OCTAVE_UP);
            song.channels[1].rack.add(0);
        }
        let clip = song.channels[1].lanes[0].add_clip(0, 8.0);
        clip.events.extend([
            note(0.0, 4.0, keys[0], false),
            note(1.0, 1.0, keys[1], true),
        ]);
        song
    };
    let expected: Vec<f32> = (played(&song([57, 69], false), 3 * BEAT).iter())
        .zip(&played(&song([69, 81], false), 3 * BEAT))
        .map(|(a, b)| a + b)
        .collect();
    assert_close(&played(&song([57, 69], true), 3 * BEAT), &expected);
}

/// A 32-bit float mono WAV at 48 kHz.
fn wav(samples: &[f32]) -> Vec<u8> {
    let data: Vec<u8> = samples.iter().flat_map(|s| s.to_le_bytes()).collect();
    let mut bytes = b"RIFF".to_vec();
    bytes.extend((36 + data.len() as u32).to_le_bytes());
    bytes.extend(b"WAVEfmt ");
    bytes.extend(16u32.to_le_bytes());
    bytes.extend(3u16.to_le_bytes());
    bytes.extend(1u16.to_le_bytes());
    bytes.extend(48_000u32.to_le_bytes());
    bytes.extend((48_000u32 * 4).to_le_bytes());
    bytes.extend(4u16.to_le_bytes());
    bytes.extend(32u16.to_le_bytes());
    bytes.extend(b"data");
    bytes.extend((data.len() as u32).to_le_bytes());
    bytes.extend(data);
    bytes
}

/// Every test sample: 0.5 for eight beats.
fn read(path: &ContentPath) -> Option<Vec<u8>> {
    matches!(path, ContentPath::Plain(_)).then(|| wav(&vec![0.5; 8 * BEAT]))
}

/// The engine for `song` with its samples.
fn engine(song: &Song) -> SongAudioEngine {
    let mut cache = SampleCache::default();
    cache.load(song, &read);
    SongAudioEngine::with_samples(song, 48_000.0, &cache).unwrap()
}

/// `before` played to beat `at`; then an engine for `after` (the same song with other Mute flags) replaces it, as the
/// app does for Mute or Unmute (`take_over` runs as the device swaps them), and plays `beats` more beats.
fn remuted(before: &Song, after: &Song, at: usize, beats: usize) -> (SongAudioEngine, Vec<f32>) {
    let mut old = engine(before);
    old.play();
    let mut output = vec![0.0; at * BEAT * 2];
    old.render(&mut output, 2);
    let mut new = engine(after).adopting();
    new.take_over(&mut old);
    let mut output = vec![0.0; beats * BEAT * 2];
    new.render(&mut output, 2);
    (new, output)
}

/// `song` played for `beats` beats, from beat `from` on.
fn played_from(song: &Song, from: usize, beats: usize) -> Vec<f32> {
    let mut engine = engine(song);
    engine.play();
    let mut output = vec![0.0; (from + beats) * BEAT * 2];
    engine.render(&mut output, 2);
    output.split_off(from * BEAT * 2)
}

/// Channel 1 holding A3 (lane 0) from beat 0 and E4 (a second notes lane) from tick 1 until beat `e4_end`; the E4
/// clip muted if `muted`. E4 starts a tick late so the two songs strike A3 and E4 in one order: the note path orders
/// notes that start together by their ends, and MiniSynth glides each note-on from the key struck before.
fn two_lanes(e4_end: f64, muted: bool) -> Song {
    let mut song = synth_song();
    song.channels[1].lanes[0]
        .add_clip(0, 8.0)
        .events
        .push(note(0.0, 4.0, 57, false));
    let lane = song.channels[1].add_lane(LaneKind::Notes);
    let clip = song.channels[1].lanes[lane].add_clip(0, 8.0);
    let tick = 1.0 / 128.0;
    clip.events.push(note(tick, e4_end - tick, 64, false));
    clip.muted = muted;
    song
}

/// `CLHd`@36: muting during playback re-syncs at once. A key the sequencer holds that
/// no unmuted note covers gets its note-off then; one still covered sounds on, untouched, and the transport goes on.
#[test]
fn muting_a_clip_while_playing_releases_the_keys_it_held_and_keeps_the_rest() {
    let (engine, output) = remuted(&two_lanes(4.0, false), &two_lanes(4.0, true), 1, 2);
    assert_eq!(engine.transport_tick(), 3 * 128);
    // As if E4 had ended at beat 1.
    assert_close(&output, &played_from(&two_lanes(1.0, false), 1, 2));
}

/// A held A3 under a Level (CC #7) lane whose clip writes 0 from beat 0.
fn level_lane(muted: bool) -> Song {
    let mut song = synth_song();
    song.channels[1].lanes[0]
        .add_clip(0, 8.0)
        .events
        .push(note(0.0, 8.0, 57, false));
    let lane = song.channels[1].add_automation_lane(AutomationTarget {
        instance_id: 0,
        param: 14,
    });
    let clip = lane.add_clip(0, 8.0);
    clip.events.push(Event::Automation {
        pos: 0,
        value: 0.0,
        control: true,
    });
    clip.muted = muted;
    song
}

/// "Automation lanes jump to their value at the playhead." Unmuting the Level lane silences the
/// channel at once. Muting it again re-syncs too: every param returns
/// to its initial value, then takes the automation still playing, so the Level is the saved one again and A3, held
/// all along, is heard as if the lane had always been muted.
#[test]
fn unmuting_an_automation_clip_while_playing_jumps_its_param_to_the_playhead() {
    let (engine, output) = remuted(&level_lane(true), &level_lane(false), 2, 1);
    assert_eq!(engine.transport_tick(), 3 * 128);
    let output = left(&output);
    assert!(
        energy(&output[..BEAT / 2]) < energy(&left(&played_from(&level_lane(true), 2, 1))) * 0.1
    );
    assert!(
        energy(&output[BEAT / 10..]) < 1e-9,
        "{}",
        energy(&output[BEAT / 10..])
    );
    let (_, output) = remuted(&level_lane(false), &level_lane(true), 2, 1);
    let half = output.len() / 2;
    assert_close(
        &output[half..],
        &played_from(&level_lane(true), 2, 1)[half..],
    );
}

/// An Audio Clip channel whose clip plays a constant 0.5 for eight beats from beat 0.
fn audio_clip(muted: bool) -> Song {
    let mut song = Song {
        tempo_bpm: 120.0,
        ..Song::default()
    };
    song.add_channel(ChannelKind::AudioClip);
    let mut slot = SamplerSlot::default();
    slot.file.path = Some(ContentPath::Plain("dc".into()));
    slot.file.id = 100;
    let clip = song.channels[1].lanes[0].add_clip(0, 8.0);
    clip.file.sample = Some(Box::new(slot));
    clip.muted = muted;
    song
}

/// On a channel whose first lane is audio every sampler voice is cut at once, then each unmuted clip
/// over the playhead restarts at its place in the clip.
#[test]
fn muting_an_audio_clip_while_playing_stops_it_at_once_and_unmuting_restarts_it() {
    let (engine, output) = remuted(&audio_clip(false), &audio_clip(true), 2, 1);
    assert_eq!(engine.transport_tick(), 3 * 128);
    assert_eq!(energy(&output), 0.0);
    let (_, output) = remuted(&audio_clip(true), &audio_clip(false), 2, 1);
    let expected = played_from(&audio_clip(false), 2, 1);
    assert!(energy(&expected) > 1.0);
    let half = output.len() / 2;
    assert_close(&output[half..], &expected[half..]);
}

/// A Drums channel whose first pad plays 0.5 for eight beats, struck at beat 0 on a notes lane for `beats`, its clip
/// muted if `muted`.
fn held_pad(beats: f64, muted: bool) -> Song {
    let mut song = Song {
        tempo_bpm: 120.0,
        ..Song::default()
    };
    song.add_channel(ChannelKind::Drums);
    let kit = song.channels[1].drum_kit.as_mut().unwrap();
    kit.slots[0].file.path = Some(ContentPath::Plain("dc".into()));
    let lane = &mut song.channels[1].lanes[0];
    lane.grid = DrumGrid::Notes;
    let clip = lane.add_clip(0, 8.0);
    clip.events.push(Event::Drum {
        pos: 0,
        len_beats: beats,
        pad: 0,
        velocity: 255,
        pan: 127,
        value: Event::DEFAULT_PITCH,
        repeat: 0,
    });
    clip.muted = muted;
    song
}

/// A pad a drum notes lane holds is a key the sequencer holds: muting its clip releases it then.
#[test]
fn muting_a_held_drum_hit_while_playing_releases_its_pad() {
    let (_, output) = remuted(&held_pad(4.0, false), &held_pad(4.0, true), 1, 2);
    assert_close(&output, &played_from(&held_pad(1.0, false), 1, 2));
}

/// A Transpose above the synth (instance `id`) and an automation lane setting it an octave up from beat 0, its clip
/// muted if `muted`.
fn automated_transpose(muted: bool) -> Song {
    let mut song = synth_song();
    let id = insert(&mut song, 0, 48, &[]);
    let lane = song.channels[1].add_automation_lane(AutomationTarget {
        instance_id: id,
        param: 1,
    });
    let clip = lane.add_clip(0, 8.0);
    clip.events.push(Event::Automation {
        pos: 0,
        value: 5.0 / 8.0,
        control: true,
    });
    clip.muted = muted;
    song
}

/// After an Unmute while playing, live keys read the note
/// effects' automation of the unmuted song.
#[test]
fn after_an_unmute_live_keys_read_the_unmuted_note_effect_automation() {
    let (mut engine, _) = remuted(
        &automated_transpose(true),
        &automated_transpose(false),
        1,
        0,
    );
    engine.note_on(1, 60, 1.0);
    let mut output = vec![0.0; 4096 * 2];
    engine.render(&mut output, 2);
    let mut expected = self::engine(&automated_transpose(false));
    expected.play();
    let mut skip = vec![0.0; BEAT * 2];
    expected.render(&mut skip, 2);
    expected.note_on(1, 60, 1.0);
    let mut reference = vec![0.0; 4096 * 2];
    expected.render(&mut reference, 2);
    assert_close(&output, &reference);
}

/// After an Unmute while playing, the Arpeggiator's panel feed follows the unmuted song: its arp runs over the chord.
#[test]
fn after_an_unmute_the_arpeggiator_feed_follows_the_unmuted_song() {
    let song = |muted: bool| {
        let mut song = synth_song();
        insert(&mut song, 0, 51, &[(1, 2.0 / 7.0), (10, 2.0 / 23.0)]);
        let clip = song.channels[1].lanes[0].add_clip(0, 8.0);
        clip.events
            .extend([note(0.0, 4.0, 60, false), note(0.0, 4.0, 64, false)]);
        clip.muted = muted;
        song
    };
    let (engine, _) = remuted(&song(true), &song(false), 1, 1);
    let arp = (0..engine.meter_count())
        .find(|&i| {
            engine.meter_key(i).instance_id == song(true).channels[1].rack.modules[0].instance_id
        })
        .unwrap();
    assert_eq!(
        engine.analysis(arp).first(),
        Some(&1.0),
        "{:?}",
        engine.analysis(arp)
    );
}

/// Notes that start on one tick strike in clip order, as live keys strike in press order, not in the order they end
/// (a tie of our own choosing). In MONO the last struck wins: a long A4 then a short A3 on
/// one tick sound the A3, and the other way round the A4.
#[test]
fn notes_on_one_tick_strike_in_clip_order() {
    let song = |notes: &[(u8, f64)]| {
        let mut song = synth_song();
        song.channels[1].rack.get_mut(2).unwrap().params[1] = 0.5;
        let clip = song.channels[1].lanes[0].add_clip(0, 8.0);
        (clip.events).extend(notes.iter().map(|&(key, len)| note(0.0, len, key, false)));
        song
    };
    let a3 = crossings(&played(&song(&[(57, 1.0)]), BEAT), 0.25, 0.75);
    let chord = crossings(&played(&song(&[(69, 2.0), (57, 1.0)]), BEAT), 0.25, 0.75);
    assert!((chord / a3 - 1.0).abs() < 0.05, "{chord} / {a3}");
    // Swapped: the A3 first, so the A4 wins.
    let a4 = crossings(&played(&song(&[(69, 2.0)]), BEAT), 0.25, 0.75);
    let chord = crossings(&played(&song(&[(57, 1.0), (69, 2.0)]), BEAT), 0.25, 0.75);
    assert!((chord / a4 - 1.0).abs() < 0.05, "{chord} / {a4}");
}

/// A 3x Osc starts a note at its next control tick, yet a 1-tick slide on the note's own tick (its only
/// slide event, on the note-on's frame) still glides it, and the pitch stays.
#[test]
fn a_one_tick_slide_on_a_three_osc_note_s_own_tick_is_not_lost() {
    let song = |slide: bool| {
        let mut song = Song {
            tempo_bpm: 120.0,
            ..Song::default()
        };
        song.add_channel(ChannelKind::Synth(SynthKind::ThreeXOsc));
        let clip = song.channels[1].lanes[0].add_clip(0, 8.0);
        clip.events.push(note(0.0, 2.0, 57, false));
        if slide {
            clip.events.push(note(0.0, 1.0 / 128.0, 69, true));
        }
        song
    };
    let plain = crossings(&played(&song(false), BEAT), 0.25, 0.75);
    let slid = crossings(&played(&song(true), BEAT), 0.25, 0.75);
    assert!((slid / plain - 2.0).abs() < 0.1, "{slid} / {plain}");
}

// The keys sounding at a channel's input, which the keyboard lights.

/// `engine` renders `beats` beats in one call.
fn render_beats(engine: &mut SongAudioEngine, beats: f64) {
    let frames = (beats * BEAT as f64) as usize;
    engine.render(&mut vec![0.0; frames * 2], 2);
}

/// Song playback sets a key at the channel's input on its note-on and clears it on its note-off; each channel has
/// its own keys.
#[test]
fn song_playback_lights_each_channels_input_keys_until_their_note_off() {
    let mut song = synth_song();
    song.add_channel(ChannelKind::Synth(SynthKind::MiniSynth));
    let clip = song.channels[1].lanes[0].add_clip(0, 8.0);
    (clip.events).extend([note(0.0, 1.0, 60, false), note(0.5, 2.0, 64, false)]);
    let clip = song.channels[2].lanes[0].add_clip(0, 8.0);
    clip.events.push(note(0.0, 4.0, 67, false));
    let mut engine = SongAudioEngine::new(&song, 48_000.0).unwrap();
    engine.play();
    assert_eq!(engine.sounding(1), 0);
    render_beats(&mut engine, 0.25);
    assert_eq!((engine.sounding(1), engine.sounding(2)), (1 << 60, 1 << 67));
    render_beats(&mut engine, 0.5);
    assert_eq!(engine.sounding(1), 1 << 60 | 1 << 64);
    // Beat 1.25: 60 has ended; beat 2.75: 64 too.
    render_beats(&mut engine, 0.5);
    assert_eq!(engine.sounding(1), 1 << 64);
    render_beats(&mut engine, 1.5);
    assert_eq!((engine.sounding(1), engine.sounding(2)), (0, 1 << 67));
    assert_eq!((engine.sounding(0), engine.sounding(9)), (0, 0));
}

/// Stop clears every key, live and sequenced. A seek while playing clears only the keys the sequencer held that no note
/// covers at the new beat and lights none: 60 (beats 0.5 to 4.5) stays
/// lit at beat 1, goes dark at beat 0, and stays dark at a seek back into its span. The loop wrap's note-offs reach
/// every key the sequencer held.
#[test]
fn stop_clears_every_key_a_seek_the_uncovered_held_ones_and_the_wrap_the_held_ones() {
    let mut song = synth_song();
    let clip = song.channels[1].lanes[0].add_clip(0, 8.0);
    clip.events.push(note(0.5, 4.0, 60, false));
    let mut engine = SongAudioEngine::new(&song, 48_000.0).unwrap();
    engine.play();
    engine.note_on(1, 72, 1.0);
    render_beats(&mut engine, 1.0);
    assert_eq!(engine.sounding(1), 1 << 60 | 1 << 72);
    engine.stop();
    assert_eq!(engine.sounding(1), 0);
    engine.play();
    engine.seek(0);
    render_beats(&mut engine, 1.0);
    assert_eq!(engine.sounding(1), 1 << 60);
    engine.seek(128);
    assert_eq!(engine.sounding(1), 1 << 60);
    engine.seek(0);
    assert_eq!(engine.sounding(1), 0);
    engine.seek(256);
    assert_eq!(engine.sounding(1), 0, "covered, not sounding");
    engine.seek(0);
    // Looping beats 0 to 2: lit at beat 1.5; wrapped at 2, back at beat 0.25, before the note; on again at 0.75.
    engine.set_loop(Some((0, 256)));
    render_beats(&mut engine, 1.5);
    assert_eq!(engine.sounding(1), 1 << 60);
    render_beats(&mut engine, 0.75);
    assert_eq!(engine.sounding(1), 0);
    render_beats(&mut engine, 0.5);
    assert_eq!(engine.sounding(1), 1 << 60);
}

/// The keys are the channel's input, before its note effects: an Arpeggiator's input lights its key the whole time
/// it is held, and the arpeggio, an octave up through the Transpose below it, lights none.
#[test]
fn note_effect_output_lights_no_key_and_the_arpeggiators_input_does() {
    let mut song = synth_song();
    insert(&mut song, 0, 48, &OCTAVE_UP);
    insert(&mut song, 0, 51, &[(1, 2.0 / 7.0), (10, 2.0 / 23.0)]);
    let clip = song.channels[1].lanes[0].add_clip(0, 8.0);
    clip.events.push(note(0.0, 2.0, 60, false));
    let mut engine = SongAudioEngine::new(&song, 48_000.0).unwrap();
    engine.play();
    let mut output = vec![0.0; BEAT / 16 * 2];
    let mut played = 0.0;
    for step in 0..32 {
        engine.render(&mut output, 2);
        played += energy(&output);
        assert_eq!(engine.sounding(1), 1 << 60, "sixteenth {step}");
    }
    assert!(played > 0.0, "the arpeggio sounds");
    // Its note-off, at beat 2, clears it.
    engine.render(&mut output, 2);
    assert_eq!(engine.sounding(1), 0);
}

/// A live key (the keyboard, a preview) lights until its note-off, and so does a song note: the sustain pedal holding
/// either note-off back does not keep the key lit.
#[test]
fn a_note_off_the_sustain_pedal_holds_back_still_clears_its_key() {
    let mut song = synth_song();
    let clip = song.channels[1].lanes[0].add_clip(0, 8.0);
    clip.events.push(note(0.0, 0.5, 64, false));
    let mut engine = SongAudioEngine::new(&song, 48_000.0).unwrap();
    engine.set_param(1, 0, 71, 1.0);
    engine.play();
    engine.note_on(1, 62, 1.0);
    render_beats(&mut engine, 0.25);
    assert_eq!(engine.sounding(1), 1 << 62 | 1 << 64);
    engine.note_off(1, 62);
    render_beats(&mut engine, 0.5);
    assert_eq!(engine.sounding(1), 0);
}

/// A Mute while playing (`CLHd`@36): a key only the muted clip held goes dark with its note-off; one
/// still covered stays lit.
#[test]
fn muting_a_clip_while_playing_darkens_the_keys_it_held_and_keeps_the_rest() {
    let (kept, _) = remuted(&two_lanes(4.0, false), &two_lanes(4.0, false), 1, 0);
    assert_eq!(kept.sounding(1), 1 << 57 | 1 << 64);
    let (muted, _) = remuted(&two_lanes(4.0, false), &two_lanes(4.0, true), 1, 0);
    assert_eq!(muted.sounding(1), 1 << 57);
}

/// `synth_song` with one 8-beat clip holding `notes`.
fn clip_song(notes: &[Event]) -> Song {
    let mut song = synth_song();
    let clip = song.channels[1].lanes[0].add_clip(0, 8.0);
    clip.events.extend_from_slice(notes);
    song
}

/// A note of length ≤ 0 is skipped at playback, and so is
/// one whose length is not finite (in memory only: a file's reads as 0): the song still builds and plays the rest as if
/// it were not there. A slide note of length ≤ 0 still slides one tick; one whose length is not finite is
/// skipped too.
#[test]
fn a_note_with_a_broken_length_is_skipped_and_the_rest_plays() {
    let anchor = note(0.0, 3.0, 69, false);
    let expected = played(&clip_song(&[anchor]), 3 * BEAT);
    let broken = [-0.25, f64::NAN, f64::INFINITY, f64::NEG_INFINITY];
    let notes = (broken.map(|len| note(1.0, len, 64, false)).into_iter())
        .chain([f64::NAN, f64::INFINITY, f64::NEG_INFINITY].map(|len| note(1.0, len, 81, true)));
    for bad in notes {
        assert_close(&played(&clip_song(&[anchor, bad]), 3 * BEAT), &expected);
    }
}

/// A file holding one note of length −0.25 loads with that note as it was, so a save writes it back unchanged,
/// and it plays: the bad note is skipped, the others sound.
#[test]
fn a_loaded_song_with_a_negative_note_keeps_it_and_plays_the_rest() {
    let good = note(0.0, 2.0, 69, false);
    let song = clip_song(&[good, note(1.0, -0.25, 64, false)]);
    let bytes = honeydew::flm::write(&song).unwrap();
    let read = honeydew::flm::read(&bytes).unwrap();
    assert_eq!(
        read.channels[1].lanes[0].clips[0].events,
        song.channels[1].lanes[0].clips[0].events
    );
    assert_eq!(honeydew::flm::write(&read).unwrap(), bytes);
    assert_close(
        &played(&read, 3 * BEAT),
        &played(&clip_song(&[good]), 3 * BEAT),
    );
}

/// A note's note-off is clamped to its clip's length. A note longer than its clip ends
/// with the clip, however long (1e300 beats, past any frame): the song builds and plays it as a note to the clip end.
/// A slide that long is cut at the clip end at its own rate (the glide time is its length): it barely moves.
#[test]
fn a_note_longer_than_its_clip_ends_with_the_clip() {
    let two_beats = |notes: &[Event]| {
        let mut song = synth_song();
        song.channels[1].lanes[0]
            .add_clip(0, 2.0)
            .events
            .extend_from_slice(notes);
        song
    };
    let expected = played(&two_beats(&[note(0.0, 2.0, 69, false)]), 4 * BEAT);
    for len in [3.0, 1e300] {
        let song = two_beats(&[note(0.0, len, 69, false)]);
        assert_close(&played(&song, 4 * BEAT), &expected);
    }
    let song = two_beats(&[note(0.0, 1e300, 69, false), note(1.0, 1e300, 81, true)]);
    assert_close(&played(&song, 4 * BEAT), &expected);
}

/// A fractional clip offset can round a play kept inside the clip (t < its visible length) onto the clip
/// end. Its note-off, clamped to the clip end, then falls on its note-on: the note is skipped, and the song builds.
#[test]
fn a_note_rounded_onto_its_clip_end_is_skipped_and_the_song_builds() {
    let mut song = synth_song();
    let clip = song.channels[1].lanes[0].add_clip(0, 1.0);
    clip.offset_beats = 0.3 / 128.0;
    clip.events.push(note(0.0, 1.0, 69, false));
    let events = honeydew::audio_bridge::SongAudioBridge::note_events_in_block(&song, 1, 0.0, 4.0);
    assert_eq!(events.unwrap(), []);
    assert_eq!(energy(&played(&song, 2 * BEAT)), 0.0);
}
