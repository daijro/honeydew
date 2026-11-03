//! The engine's position re-sync: a seek while playing, a loop
//! wrap, play start and a clip Mute change release only the keys the sequencer holds that no unmuted note covers,
//! never start a note, put every param back to its initial value plus its automation at the beat, and restart the
//! audio clips there. A seek while stopped only moves the position. Rendered offline through `AudioRenderer`; no
//! device.

use honeydew::audio_engine::SongAudioEngine;
use honeydew::flm::chunk::ContentPath;
use honeydew::sample_cache::SampleCache;
use honeydew::song::{
    AutomationTarget, ChannelKind, DrumGrid, Event, LaneKind, SamplerSlot, Song, SynthKind,
};
use honeydew::ui::preview::{self, ClipKey};
use honeydew_audio::device::AudioRenderer;

/// One beat at 120 BPM and 48 kHz.
const BEAT: usize = 24_000;
const TICK: u64 = 128;

fn energy(samples: &[f32]) -> f32 {
    samples.iter().map(|sample| sample * sample).sum()
}

fn assert_close(actual: &[f32], expected: &[f32]) {
    assert!(energy(expected) > 0.01);
    assert_eq!(actual.len(), expected.len());
    for (frame, (a, e)) in actual.iter().zip(expected).enumerate() {
        assert!((a - e).abs() <= 1e-5, "frame {frame}: {a} vs {e}");
    }
}

/// The left side of `beats` beats rendered from where the engine is, in odd blocks.
fn render(engine: &mut SongAudioEngine, beats: f64) -> Vec<f32> {
    let frames = (beats * BEAT as f64) as usize;
    let mut left = Vec::with_capacity(frames);
    let mut output = vec![0.0; 777 * 2];
    while left.len() < frames {
        let n = 777.min(frames - left.len());
        engine.render(&mut output[..n * 2], 2);
        left.extend(output[..n * 2].iter().step_by(2));
    }
    left
}

fn note(beat: f64, len_beats: f64, key: u8) -> Event {
    Event::Note {
        pos: (beat * 128.0) as u32,
        len_beats,
        key,
        velocity: 204,
        pan: 127,
        slide: false,
    }
}

/// 120 BPM, a MiniSynth on channel 1 (instance 2) with a 16-beat clip holding `notes`.
fn synth(notes: &[Event]) -> Song {
    let mut song = Song {
        tempo_bpm: 120.0,
        ..Song::default()
    };
    song.add_channel(ChannelKind::Synth(SynthKind::MiniSynth));
    song.channels[1].lanes[0]
        .add_clip(0, 16.0)
        .events
        .extend_from_slice(notes);
    song
}

/// The channel's Level (channel param 0) automated by one clip from `start` beats for `len` beats with `points`
/// (beat in the clip, value).
fn level_lane(song: &mut Song, start: f64, len: f64, points: &[(f64, f32)]) {
    let lane = song.channels[1].add_automation_lane(AutomationTarget {
        instance_id: 0,
        param: 0,
    });
    let clip = lane.add_clip((start * 128.0) as u32, len);
    clip.events
        .extend(points.iter().map(|&(beat, value)| Event::Automation {
            pos: (beat * 128.0) as u32,
            value,
            control: true,
        }));
}

/// `song` played from beat 0 for `beats` beats.
fn played(song: &Song, beats: f64) -> Vec<f32> {
    let mut engine = engine(song);
    engine.play();
    render(&mut engine, beats)
}

/// `song` played for `before` beats, then sought to beat `to` while playing, then `after` beats: the output after the
/// seek.
fn sought(song: &Song, before: f64, to: f64, after: f64) -> Vec<f32> {
    let mut engine = engine(song);
    engine.play();
    render(&mut engine, before);
    engine.seek((to * TICK as f64) as u64);
    render(&mut engine, after)
}

/// A 32-bit float mono WAV at 48 kHz.
fn wav(samples: &[f32]) -> Vec<u8> {
    let data: Vec<u8> = samples.iter().flat_map(|s| s.to_le_bytes()).collect();
    let mut bytes = b"RIFF".to_vec();
    bytes.extend((36 + data.len() as u32).to_le_bytes());
    bytes.extend(b"WAVEfmt ");
    for field in [16u32, 0x0001_0003, 48_000, 192_000, 0x0020_0004] {
        bytes.extend(field.to_le_bytes());
    }
    bytes.extend(b"data");
    bytes.extend((data.len() as u32).to_le_bytes());
    bytes.extend(data);
    bytes
}

/// The engine for `song`, every sample 0.5 for eight beats.
fn engine(song: &Song) -> SongAudioEngine {
    let mut cache = SampleCache::default();
    cache.load(song, &|path: &ContentPath| {
        matches!(path, ContentPath::Plain(_)).then(|| wav(&vec![0.5; 8 * BEAT]))
    });
    SongAudioEngine::with_samples(song, 48_000.0, &cache).unwrap()
}

/// A sounding key that a note at the new beat covers sounds on untouched,
/// no note-off and no new note-on, and ends at the next note-off the sequencer plays on it from there: the covering
/// note's end. A3 held from beat 0, sought at beat 1 into a second A3 (beats 8 to 10) at beat 9, sounds exactly as one
/// A3 held for two beats.
#[test]
fn a_seek_keeps_a_covered_sounding_key_until_the_covering_notes_end() {
    let song = synth(&[note(0.0, 4.0, 57), note(8.0, 2.0, 57)]);
    let mut output = played(&song, 1.0);
    output.extend(sought(&song, 1.0, 9.0, 2.0));
    assert_close(&output, &played(&synth(&[note(0.0, 2.0, 57)]), 3.0));
}

/// A key the sequencer holds that no note covers at the new beat gets its note-off there: it releases, as if its note
/// had ended at the seek.
#[test]
fn a_seek_releases_a_held_key_no_note_covers() {
    let song = synth(&[note(0.0, 4.0, 57)]);
    let mut output = played(&song, 1.0);
    output.extend(sought(&song, 1.0, 9.0, 2.0));
    assert_close(&output, &played(&synth(&[note(0.0, 1.0, 57)]), 3.0));
}

/// A key a note covers at the new beat that is not sounding stays silent: no note starts mid-span, from its start, at
/// the seek or with the remaining length; the next note starts when the playhead crosses its start.
#[test]
fn a_seek_into_a_note_that_is_not_sounding_starts_nothing() {
    let song = synth(&[note(2.0, 4.0, 57), note(7.0, 1.0, 57)]);
    let output = sought(&song, 0.5, 3.0, 4.5);
    assert_eq!(energy(&output[..4 * BEAT]), 0.0);
    assert!(energy(&output[4 * BEAT..]) > 0.01, "the next note plays");
}

/// Play start: all notes off, then a re-sync at the start beat. A note spanning it does not sound, whether the
/// transport was sought there while stopped or stopped there mid-note.
#[test]
fn play_start_onto_a_spanning_note_is_silent() {
    let song = synth(&[note(0.0, 8.0, 57)]);
    let mut engine = engine(&song);
    engine.seek(3 * TICK);
    engine.play();
    assert_eq!(energy(&render(&mut engine, 2.0)), 0.0);
    engine.stop();
    render(&mut engine, 1.0);
    engine.play();
    assert_eq!(energy(&render(&mut engine, 2.0)), 0.0);
}

/// A seek while stopped only moves the position: a live key sounds on, and the params stay as they are until Play
/// re-syncs them to the new beat. The Level is automated to 1 at beat 0 and 0 from beat 1.
#[test]
fn a_seek_while_stopped_only_moves_the_position() {
    let mut song = synth(&[]);
    level_lane(&mut song, 0.0, 2.0, &[(0.0, 1.0), (1.0, 0.0)]);
    let mut engine = engine(&song);
    engine.note_on(1, 69, 1.0);
    let before = render(&mut engine, 0.25);
    engine.seek(TICK * 3 / 2);
    assert_eq!(engine.transport_tick(), TICK * 3 / 2);
    let after = render(&mut engine, 0.25);
    assert!(
        energy(&after) > energy(&before) * 0.5,
        "the key sounds on at the Level of beat 0"
    );
    engine.play();
    render(&mut engine, 0.25);
    assert_eq!(
        energy(&render(&mut engine, 0.25)),
        0.0,
        "Play applied Level 0"
    );
}

/// Every param returns to its initial value, then takes its automation at the new beat: before the Level's clip (beats
/// 2 to 3, Level 0) the saved Level is back, inside it and past its end the clip's last point holds. A3 is held across
/// every seek (covered by its note, beats 0 to 8), so only the Level changes what sounds.
#[test]
fn a_seek_sets_each_param_to_its_initial_value_plus_its_automation_at_the_beat() {
    let mut song = synth(&[note(0.0, 8.0, 57)]);
    level_lane(&mut song, 2.0, 1.0, &[(0.0, 0.0)]);
    let mut engine = engine(&song);
    engine.play();
    assert!(energy(&render(&mut engine, 1.0)) > 0.01);
    render(&mut engine, 1.5);
    assert_eq!(energy(&render(&mut engine, 0.25)), 0.0, "the clip set 0");
    engine.seek(TICK);
    assert!(energy(&render(&mut engine, 0.25)) > 0.01, "the saved Level");
    engine.seek(TICK * 5 / 2);
    render(&mut engine, 0.05);
    assert_eq!(energy(&render(&mut engine, 0.2)), 0.0, "inside the clip");
    engine.seek(TICK * 4);
    render(&mut engine, 0.05);
    assert_eq!(energy(&render(&mut engine, 0.2)), 0.0, "past its end");
}

/// The loop wrap: a note-off for each key the sequencer holds (a live key sounds on), then a re-sync at the loop start,
/// which starts no note spanning it. Loop beats 1 to 3; A3 from beat 0 for 4 beats spans the loop start.
#[test]
fn the_loop_wrap_releases_the_sequencers_keys_and_starts_no_spanning_note() {
    let mut song = synth(&[note(0.0, 4.0, 57)]);
    song.play_loop = Some((128, 3 * 128));
    let output = played(&song, 5.0);
    assert!(energy(&output[2 * BEAT..3 * BEAT]) > 0.01);
    // AMP Rel 0.07 releases within 0.14 s (6720 frames); the second pass is silent after that.
    assert_eq!(energy(&output[3 * BEAT + 8000..5 * BEAT]), 0.0);

    // A live key held across wraps sounds as it does with no loop.
    let live = |looped: bool| {
        let mut song = synth(&[]);
        song.play_loop = looped.then_some((0, 128));
        let mut engine = engine(&song);
        engine.play();
        engine.note_on(1, 69, 1.0);
        let output = render(&mut engine, 3.0);
        (output, engine.sounding(1))
    };
    let (looped, lit) = live(true);
    assert_close(&looped, &live(false).0);
    assert_eq!(lit, 1 << 69, "the wrap leaves a live key lit");
}

/// The 0.2 keys: a seek clears the keys the sequencer held that no note
/// covers; a covered key that is sounding stays lit, and a covered key that was not sounding stays dark.
#[test]
fn a_seek_clears_only_the_uncovered_keys_the_sequencer_held() {
    let song = synth(&[
        note(0.0, 4.0, 57),
        note(0.0, 4.0, 64),
        note(8.0, 2.0, 57),
        note(8.5, 2.0, 60),
    ]);
    let mut engine = engine(&song);
    engine.play();
    render(&mut engine, 1.0);
    assert_eq!(engine.sounding(1), 1 << 57 | 1 << 64);
    engine.seek(9 * TICK);
    assert_eq!(engine.sounding(1), 1 << 57);
    render(&mut engine, 0.5);
    assert_eq!(engine.sounding(1), 1 << 57);
    // Beat 10: the covering note ends, and so does the key.
    render(&mut engine, 0.75);
    assert_eq!(engine.sounding(1), 0);
}

/// Step lanes: every earlier hit in the clip covers its pad, since step hits have no length. A step hit sounds on
/// through a seek inside its clip and is released by a seek out of it, as a drum notes lane's hit is by its note-off.
#[test]
fn a_step_hit_sounds_on_through_a_seek_inside_its_clip_and_is_released_outside_it() {
    let drums = |grid: DrumGrid, len_beats: f64| {
        let mut song = Song {
            tempo_bpm: 120.0,
            ..Song::default()
        };
        song.add_channel(ChannelKind::Drums);
        let kit = song.channels[1].drum_kit.as_mut().unwrap();
        kit.slots[0].file.path = Some(ContentPath::Plain("dc".into()));
        let lane = &mut song.channels[1].lanes[0];
        lane.grid = grid;
        lane.add_clip(0, 4.0).events.push(Event::Drum {
            pos: 0,
            len_beats,
            pad: 0,
            velocity: 255,
            pan: 127,
            value: Event::DEFAULT_PITCH,
            repeat: 0,
        });
        // An empty clip at beat 64 keeps the song end, where playback wraps, out of the way.
        lane.add_clip(64 * 128, 1.0);
        song
    };
    let steps = drums(DrumGrid::Steps, 0.0);
    // Inside the clip: the voice goes on as if nothing happened.
    let inside = sought(&steps, 1.0, 2.0, 1.0);
    assert_close(&inside, &played(&steps, 2.0)[BEAT..]);
    // Out of it: released, as a notes lane's hit ending at the seek.
    let outside = sought(&steps, 1.0, 4.5, 1.0);
    let released = played(&drums(DrumGrid::Notes, 1.0), 2.0);
    assert_close(&outside, &released[BEAT..]);
}

/// Unmuting a notes clip while playing starts no note: the E4 it unmutes covers the playhead but stays silent, and A3
/// sounds on untouched.
#[test]
fn unmuting_a_clip_while_playing_starts_no_note() {
    let two_lanes = |muted: bool| {
        let mut song = synth(&[note(0.0, 4.0, 57)]);
        let lane = song.channels[1].add_lane(LaneKind::Notes);
        let clip = song.channels[1].lanes[lane].add_clip(0, 8.0);
        clip.events.push(note(1.0 / 128.0, 3.0, 64));
        clip.muted = muted;
        song
    };
    let mut old = engine(&two_lanes(true));
    old.play();
    render(&mut old, 1.0);
    let mut new = engine(&two_lanes(false)).adopting();
    new.take_over(&mut old);
    assert_eq!(new.sounding(1), 1 << 57);
    let output = render(&mut new, 2.0);
    assert_close(&output, &played(&two_lanes(true), 3.0)[BEAT..]);
}

/// Mute or Unmute: every audio lane on every channel is hard-stopped and its
/// covering clips restart at their offset, gain 1, centre pan, also on a channel whose first lane is not audio (its
/// clip slots are renumbered for the new song). The MiniSynth channel's audio lane plays on as it would have.
#[test]
fn a_mute_change_restarts_the_audio_clips_of_every_channel_in_place() {
    let song = |muted: bool| {
        let mut song = synth(&[note(0.0, 1.0, 57)]);
        song.channels[1].lanes[0].clips[0].muted = muted;
        let audio = song.channels[1].add_lane(LaneKind::Audio);
        let mut slot = SamplerSlot::default();
        slot.file.path = Some(ContentPath::Plain("dc".into()));
        let clip = song.channels[1].lanes[audio].add_clip(0, 8.0);
        clip.file.sample = Some(Box::new(slot));
        song
    };
    let mut old = engine(&song(false));
    old.play();
    render(&mut old, 2.0);
    let mut new = engine(&song(true)).adopting();
    new.take_over(&mut old);
    let output = render(&mut new, 1.0);
    let expected = &played(&song(false), 3.0)[2 * BEAT..];
    let half = BEAT / 2;
    assert_close(&output[half..], &expected[half..]);
}

/// Preview mode: only the current clip counts as covering: the engine plays the previewed
/// clip alone ([`preview::solo`]). A3 from the current clip (beats 0 to 2) is held at a seek to beat 5, where only
/// another clip's A3 (beats 4 to 8) covers it: the whole song keeps it, Preview releases it.
#[test]
fn in_preview_only_the_current_clip_covers_a_key() {
    let mut song = synth(&[note(0.0, 2.0, 57)]);
    let lane = song.channels[1].add_lane(LaneKind::Notes);
    song.channels[1].lanes[lane]
        .add_clip(0, 16.0)
        .events
        .push(note(4.0, 4.0, 57));
    let key = ClipKey {
        channel: 1,
        lane: 0,
        id: song.channels[1].lanes[0].clips[0].id,
    };
    let solo = preview::solo(&song, key).unwrap();
    let whole = sought(&song, 1.0, 5.0, 1.0);
    assert!(
        energy(&whole[BEAT / 2..]) > 0.01,
        "the other clip covers it"
    );
    let previewed = sought(&solo, 1.0, 5.0, 1.0);
    assert!(
        energy(&previewed[..BEAT / 8]) > 0.01,
        "a release, not a cut"
    );
    assert_eq!(energy(&previewed[BEAT / 2..]), 0.0, "released at the seek");
}

/// A count-in holds the transport; the re-sync runs when it ends, at the start beat. The Level is automated to 0 from
/// beat 1, so after a stopped seek to beat 1.5 and a one-beat count-in the note at beat 2 is silent.
#[test]
fn the_re_sync_runs_when_a_count_in_ends() {
    let mut song = synth(&[note(2.0, 1.0, 57)]);
    level_lane(&mut song, 1.0, 4.0, &[(0.0, 0.0)]);
    let mut engine = engine(&song);
    engine.seek(TICK * 3 / 2);
    engine.count_in(1);
    engine.play();
    render(&mut engine, 1.0);
    assert!(!engine.counting_in());
    assert_eq!(energy(&render(&mut engine, 1.5)), 0.0);
}

/// "start ≤ new beat": a note starting exactly at the new beat covers its key, so the sounding key gets no
/// note-off before it (it stays lit) and the sequencer then plays that note, whose note-off ends it.
#[test]
fn a_note_starting_at_the_new_beat_covers_its_key() {
    let song = synth(&[note(0.0, 4.0, 57), note(9.0, 1.5, 57)]);
    let mut engine = engine(&song);
    engine.play();
    render(&mut engine, 1.0);
    engine.seek(9 * TICK);
    assert_eq!(engine.sounding(1), 1 << 57, "no note-off");
    assert!(energy(&render(&mut engine, 1.0)) > 0.01);
    assert_eq!(engine.sounding(1), 1 << 57);
    // Beat 10.5: its note-off; AMP Rel 0.07 s has died away 8000 frames later.
    render(&mut engine, 0.55);
    assert_eq!(engine.sounding(1), 0);
    assert_eq!(energy(&render(&mut engine, 1.0)[8000..]), 0.0);
}

/// The re-sync's steps in order: 1 every param to its initial value, 3 the note-offs, 4 the automation at
/// the beat. Step 1 keeps the MIDI CCs, so the note-offs meet the sustain pedal as it stands. A3 (beats 0 to
/// 2) is held at a seek to beat 3 or 5, past its end; the pedal lane writes from beat 4.
/// - Up initially, automated down: released at a seek to 5, before the automation puts the pedal down.
/// - Down initially, automated up: held at 3 (no automation yet), and at 5 held, then released as the automation lifts
///   the pedal.
#[test]
fn the_re_sync_note_offs_meet_the_pedal_as_it_stands() {
    let pedalled = |initial: f32, automated: f32, to: f64| {
        let mut song = synth(&[note(0.0, 2.0, 57)]);
        let lane = song.channels[1].add_automation_lane(AutomationTarget {
            instance_id: 0,
            param: 71,
        });
        lane.add_clip(4 * 128, 4.0).events.push(Event::Automation {
            pos: 0,
            value: automated,
            control: true,
        });
        let mut engine = engine(&song);
        engine.set_param(1, 0, 71, initial);
        engine.play();
        render(&mut engine, 1.0);
        engine.seek((to * TICK as f64) as u64);
        // AMP Rel 0.07 s: released, A3 is silent 8000 frames later; held, it sounds on.
        energy(&render(&mut engine, 1.0)[8000..])
    };
    assert_eq!(pedalled(0.0, 1.0, 5.0), 0.0, "up, then down");
    assert!(pedalled(1.0, 0.0, 3.0) > 0.01, "down");
    assert_eq!(pedalled(1.0, 0.0, 5.0), 0.0, "down, then up");
}

/// A note starting exactly at the new beat covers its sounding key (no note-off) and its own note-on and note-off count
/// it, so no hold is left once it ends: a live key on it then sounds on through a seek that finds nothing held (the
/// probe). The live key reaches the instrument at the next render.
#[test]
fn a_note_starting_at_the_new_beat_leaves_no_hold_behind() {
    let song = synth(&[note(0.0, 4.0, 57), note(9.0, 1.5, 57)]);
    let mut engine = engine(&song);
    engine.play();
    render(&mut engine, 1.0);
    engine.seek(9 * TICK);
    render(&mut engine, 3.0);
    engine.note_on(1, 57, 1.0);
    render(&mut engine, 0.1);
    engine.seek(30 * TICK);
    assert!(
        energy(&render(&mut engine, 0.5)[8000..]) > 0.01,
        "the live key sounds on"
    );
}

/// REV at the play-start point resets the sound: unlike Stop, whose
/// releases and tails ring on, everything goes silent at once: a synth note into a Reverb, a keyboard voice, a drum pad
/// and an audio clip. No key stays lit, and the position stays.
#[test]
fn a_sound_reset_silences_every_voice_at_once() {
    let mut song = synth(&[note(0.0, 4.0, 57)]);
    song.channels[1].rack.add(32);
    song.add_channel(ChannelKind::Drums);
    let kit = song.channels[2].drum_kit.as_mut().unwrap();
    kit.slots[0].file.path = Some(ContentPath::Plain("dc".into()));
    song.channels[2].lanes[0]
        .add_clip(0, 4.0)
        .events
        .push(Event::Drum {
            pos: 0,
            len_beats: 0.0,
            pad: 0,
            velocity: 255,
            pan: 127,
            value: Event::DEFAULT_PITCH,
            repeat: 0,
        });
    song.add_channel(ChannelKind::AudioClip);
    let mut slot = SamplerSlot::default();
    slot.file.path = Some(ContentPath::Plain("dc".into()));
    let clip = song.channels[3].lanes[0].add_clip(0, 8.0);
    clip.file.sample = Some(Box::new(slot));
    let after_stop = |reset: bool| {
        let mut engine = engine(&song);
        engine.play();
        render(&mut engine, 1.0);
        engine.stop();
        engine.note_on(1, 72, 1.0);
        let tick = engine.transport_tick();
        if reset {
            engine.reset_sound();
        }
        assert_eq!(engine.transport_tick(), tick);
        let output = render(&mut engine, 0.25);
        // Each part on its own after the first 0.01 beat: the clip's fade, the pad's release, the reverb's tail.
        (energy(&output), energy(&output[240..]), engine.sounding(1))
    };
    let (rings, tail, lit) = after_stop(false);
    assert!(rings > 0.01 && tail > 0.01);
    assert_eq!(lit, 1 << 72);
    assert_eq!(after_stop(true), (0.0, 0.0, 0));
}

/// What the sequencer holds is counted exactly through a re-sync: a held key no note covers is released and no longer
/// held, and one a note covers is held once more, until that note's note-off. Then a live key on it sounds on through a
/// later seek, which finds nothing held and sends no note-off. Overlapping notes on the key come first.
#[test]
fn a_re_sync_leaves_no_hold_behind() {
    let song = synth(&[note(0.0, 4.0, 57), note(0.5, 3.5, 57), note(8.0, 2.0, 57)]);
    for to in [9.0, 20.0] {
        let mut engine = engine(&song);
        engine.play();
        render(&mut engine, 1.0);
        engine.seek((to * TICK as f64) as u64);
        render(&mut engine, 2.0);
        engine.note_on(1, 57, 1.0);
        render(&mut engine, 0.1);
        engine.seek(30 * TICK);
        assert!(
            energy(&render(&mut engine, 0.5)[8000..]) > 0.01,
            "{to}: the live key sounds on"
        );
    }
}

/// Step 1 covers what a note-off reads: an instrument's params return to their initial value before the
/// note-offs. MiniSynth latches its AMP Release at the note-off; automated to 1.0 over beats 4 to 8, it is back at its
/// saved 0.07 when a seek to beat 1 releases the note (beats 4 to 8), which dies away within 8000 frames (the
/// probe).
#[test]
fn a_re_sync_note_off_meets_the_instruments_initial_params() {
    let mut song = synth(&[note(4.0, 4.0, 57)]);
    let lane = song.channels[1].add_automation_lane(AutomationTarget {
        instance_id: 2,
        param: 25,
    });
    lane.add_clip(4 * 128, 4.0).events.push(Event::Automation {
        pos: 0,
        value: 1.0,
        control: true,
    });
    let mut engine = engine(&song);
    engine.seek(4 * TICK);
    engine.play();
    assert!(energy(&render(&mut engine, 1.0)) > 0.01);
    engine.seek(TICK);
    assert_eq!(energy(&render(&mut engine, 1.0)[8000..]), 0.0);
}

/// The sustain pedal is not one of them: a MIDI CC, which step 1 keeps. Up
/// initially and automated down over beats 4 to 8, it is still down for the note-off of a seek from inside that span to
/// beat 6, inside it too, past the note (beats 4.5 to 5.5): the pedal holds the note-off, and the note sounds on.
#[test]
fn a_re_sync_note_off_meets_the_pedal_down_between_two_automated_downs() {
    let mut song = synth(&[note(4.5, 1.0, 57)]);
    let lane = song.channels[1].add_automation_lane(AutomationTarget {
        instance_id: 0,
        param: 71,
    });
    lane.add_clip(4 * 128, 4.0).events.push(Event::Automation {
        pos: 0,
        value: 1.0,
        control: true,
    });
    let mut engine = engine(&song);
    engine.seek(TICK * 9 / 2);
    engine.play();
    assert!(energy(&render(&mut engine, 0.25)) > 0.01);
    engine.seek(6 * TICK);
    assert!(energy(&render(&mut engine, 1.0)[8000..]) > 0.01);
}

/// The pad version of the stale-hold probe: a notes-lane hit starting exactly at the new beat on a pad already sounding
/// takes the pad over (no release) and its own hit and note-off count it, so no hold is left once it ends: a live hit
/// on the pad then sounds on, undecayed, through a seek that finds nothing held.
#[test]
fn a_hit_starting_at_the_new_beat_leaves_no_pad_hold_behind() {
    let mut song = Song {
        tempo_bpm: 120.0,
        ..Song::default()
    };
    song.add_channel(ChannelKind::Drums);
    let kit = song.channels[1].drum_kit.as_mut().unwrap();
    kit.slots[0].file.path = Some(ContentPath::Plain("dc".into()));
    let key = kit.pad_note(0).unwrap();
    let lane = &mut song.channels[1].lanes[0];
    lane.grid = DrumGrid::Notes;
    let hit = |beat: f64, len_beats: f64| Event::Drum {
        pos: (beat * 128.0) as u32,
        len_beats,
        pad: 0,
        velocity: 255,
        pan: 127,
        value: Event::DEFAULT_PITCH,
        repeat: 0,
    };
    (lane.add_clip(0, 16.0).events).extend([hit(0.0, 4.0), hit(9.0, 1.5)]);
    // An empty clip at beat 64 keeps the song end, where playback wraps, out of the way.
    lane.add_clip(64 * 128, 1.0);
    let mut engine = engine(&song);
    engine.play();
    render(&mut engine, 1.0);
    engine.seek(9 * TICK);
    // Past the hit's note-off (beat 10.5) and the end of every eight-beat sample struck so far.
    render(&mut engine, 9.0);
    engine.note_on(1, key, 1.0);
    render(&mut engine, 0.1);
    engine.seek(30 * TICK);
    let output = render(&mut engine, 0.5);
    assert!(output[100] > 0.1, "{}", output[100]);
    assert!(
        (output[100] - output[10_000]).abs() < 1e-4,
        "{} then {}: released",
        output[100],
        output[10_000]
    );
}

/// A tempo drag's live tempo keeps the beat, and every prepared event
/// keeps its own: A3 (beats 0 to 2) at 120 BPM for one beat, then 240 BPM, sounds until its note-off half a second
/// (one beat at 240) later, not a whole one.
#[test]
fn a_live_tempo_keeps_the_beat_of_the_transport_and_of_the_events() {
    let song = synth(&[note(0.0, 2.0, 57)]);
    let mut engine = engine(&song);
    engine.play();
    render(&mut engine, 1.0);
    engine.set_tempo(240.0);
    assert_eq!(engine.transport_tick(), TICK);
    // 0.45 beat at 120 BPM: 0.9 beat at 240.
    render(&mut engine, 0.45);
    assert_eq!(engine.sounding(1), 1 << 57);
    render(&mut engine, 0.1);
    assert_eq!(engine.sounding(1), 0, "released at beat 2");
    assert_eq!(engine.transport_tick(), 2 * TICK + TICK / 10);
}

/// Item 2 and the engine side of an edit taken over without a re-sync: past its step clip's end the pad of the last
/// hit still rings, uncovered, and goes on as if nothing happened; with a re-sync it is released.
#[test]
fn an_edit_taken_over_without_a_re_sync_releases_nothing() {
    let mut song = Song {
        tempo_bpm: 120.0,
        ..Song::default()
    };
    song.add_channel(ChannelKind::Drums);
    let kit = song.channels[1].drum_kit.as_mut().unwrap();
    kit.slots[0].file.path = Some(ContentPath::Plain("dc".into()));
    song.channels[1].lanes[0]
        .add_clip(0, 4.0)
        .events
        .push(Event::Drum {
            pos: 3 * 128,
            len_beats: 0.0,
            pad: 0,
            velocity: 255,
            pan: 127,
            value: Event::DEFAULT_PITCH,
            repeat: 0,
        });
    // An empty clip at beat 64 keeps the song end, where playback wraps, out of the way.
    song.channels[1].lanes[0].add_clip(64 * 128, 1.0);
    let after = |resync: bool| {
        let mut old = engine(&song);
        old.play();
        render(&mut old, 4.5);
        let mut new = engine(&song).adopting_edit(resync, Vec::new());
        new.take_over(&mut old);
        render(&mut new, 1.0)
    };
    let expected = &played(&song, 5.5)[(4.5 * BEAT as f64) as usize..];
    assert_close(&after(false), expected);
    assert!(
        energy(&after(true)) < energy(expected) * 0.99,
        "a re-sync releases it"
    );
}

/// A live tempo moves the transport over the events as they were prepared, each at its beat. At 240 BPM
/// from the start, the Level automation's drop at beat 2, the audio clip's start at beat 3 and the loop's wrap at beat 4
/// all come twice as soon; the wrap starts the note and the automation over.
#[test]
fn a_live_tempo_keeps_the_loop_the_automation_and_an_audio_clip_in_beat() {
    let mut song = synth(&[note(0.0, 4.0, 57)]);
    song.play_loop = Some((0, 4 * 128));
    level_lane(&mut song, 0.0, 4.0, &[(0.0, 0.8), (1.99, 0.8), (2.0, 0.0)]);
    let audio = song.channels.len();
    song.add_channel(ChannelKind::AudioClip);
    let mut slot = SamplerSlot::default();
    slot.file.path = Some(ContentPath::Plain("dc".into()));
    song.channels[audio].lanes[0]
        .add_clip(3 * 128, 1.0)
        .file
        .sample = Some(Box::new(slot));
    let mut engine = engine(&song);
    engine.set_tempo(240.0);
    engine.play();
    // 4.5 beats at 240 BPM, 12 000 frames each.
    let output = render(&mut engine, 2.25);
    let beats = |from: f64, to: f64| {
        let at = |beat: f64| (beat * BEAT as f64 / 2.0) as usize;
        energy(&output[at(from)..at(to)])
    };
    assert!(beats(0.1, 1.9) > 0.01, "the note at its level");
    assert!(beats(2.1, 2.9) < 1e-6, "the Level automated to 0 at beat 2");
    assert!(beats(3.1, 3.9) > 0.01, "the audio clip from beat 3");
    assert!(
        beats(4.1, 4.4) > 0.01,
        "wrapped at beat 4: the note at its level again"
    );
    assert_eq!(engine.transport_tick(), 64);
}

/// A live key held when Play is tapped: its sound is released at once, though the finger is still down, and
/// it goes dark. Its later release is a second note-off with no effect: it does not end the song's own A4, which
/// started on that key meanwhile (beats 0.5 to 2.5).
#[test]
fn a_live_key_held_at_play_is_cut_and_its_release_does_nothing() {
    let song = synth(&[note(0.5, 2.0, 69)]);
    let mut engine = engine(&song);
    engine.note_on(1, 69, 1.0);
    assert!(energy(&render(&mut engine, 0.25)) > 0.01);
    engine.play();
    assert_eq!(engine.sounding(1), 0, "dark at Play");
    // AMP Rel 0.07 s: released at Play, silent from 8000 frames on until the song's A4 at beat 0.5.
    assert_eq!(energy(&render(&mut engine, 1.0)[8000..BEAT / 2]), 0.0);
    engine.note_off(1, 69);
    assert_eq!(engine.sounding(1), 1 << 69);
    assert!(
        energy(&render(&mut engine, 1.0)[8000..]) > 0.01,
        "the song's A4 sounds on"
    );
}

/// The key that starts playback keeps sounding: its press is queued, and reaches the instruments at the
/// start of the next block, after the Play that went out with it and its all-notes-off. It sounds and stays lit.
#[test]
fn a_press_queued_with_play_sounds_after_its_all_notes_off() {
    let mut engine = engine(&synth(&[]));
    engine.note_on(1, 72, 1.0);
    engine.play();
    assert_eq!(engine.sounding(1), 1 << 72);
    assert!(energy(&render(&mut engine, 0.5)[8000..]) > 0.01);
}

/// Stop has Play start's all-notes-off: a press queued with it plays after it too, so the app's lit key is
/// heard.
#[test]
fn a_press_queued_with_stop_sounds_after_its_all_notes_off() {
    let mut engine = engine(&synth(&[]));
    engine.play();
    render(&mut engine, 0.25);
    engine.note_on(1, 72, 1.0);
    engine.stop();
    assert_eq!(engine.sounding(1), 1 << 72);
    assert!(energy(&render(&mut engine, 0.5)[8000..]) > 0.01);
}

/// Sustain pedal: with the pedal down, a struck and released key is held by the pedal alone. Play releases
/// it (a forced note-off), but leaves the pedal down: a key released after Play is held by it again.
#[test]
fn play_releases_the_keys_the_pedal_alone_holds_and_keeps_the_pedal() {
    let mut engine = engine(&synth(&[]));
    engine.set_param(1, 0, 71, 1.0);
    let struck = |engine: &mut SongAudioEngine, key: u8| {
        engine.note_on(1, key, 1.0);
        render(engine, 0.25);
        engine.note_off(1, key);
    };
    struck(&mut engine, 72);
    assert!(energy(&render(&mut engine, 0.5)[8000..]) > 0.01, "held");
    engine.play();
    assert_eq!(energy(&render(&mut engine, 0.5)[8000..]), 0.0, "released");
    struck(&mut engine, 76);
    assert!(
        energy(&render(&mut engine, 0.5)[8000..]) > 0.01,
        "held again"
    );
}

/// Loop wrap: only the keys the sequencer holds get a note-off, a forced one that a pedal down
/// does not hold; a live key sounds on. Loop beats 0 to 2; channel 1's A3 (beats 0.5 to 4) spans the loop end with its
/// pedal down; channel 2 holds a live key.
#[test]
fn the_loop_wrap_force_releases_only_the_sequencers_keys() {
    let mut song = synth(&[note(0.5, 3.5, 57)]);
    song.play_loop = Some((0, 2 * 128));
    song.add_channel(ChannelKind::Synth(SynthKind::MiniSynth));
    let mut engine = engine(&song);
    engine.set_param(1, 0, 71, 1.0);
    engine.play();
    engine.note_on(2, 69, 1.0);
    render(&mut engine, 2.2);
    // Past the wrap by the release time, before A3 starts again at beat 0.5.
    let peak = |engine: &SongAudioEngine, channel: usize| {
        let at = (0..engine.meter_count())
            .find(|&i| {
                engine.meter_key(i).channel == channel && engine.meter_key(i).instance_id == 0
            })
            .unwrap();
        engine.meter(at).peak_left
    };
    render(&mut engine, 0.2);
    assert_eq!(peak(&engine, 1), 0.0, "the song's A3 released");
    assert!(peak(&engine, 2) > 0.01, "the live key sounds on");
    assert_eq!((engine.sounding(1), engine.sounding(2)), (0, 1 << 69));
}
