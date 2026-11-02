//! Structural edits while playing: the engine for the edited song takes over the playing one channel by channel and
//! module by module, so an edit cuts only what it destroys. Each output after the edit is checked against an engine
//! that played the expected song from the start. Rendered offline through `AudioRenderer`; no device.

use honeydew::audio_engine::SongAudioEngine;
use honeydew::flm::chunk::ContentPath;
use honeydew::sample_cache::SampleCache;
use honeydew::song::{ChannelKind, Event, ModuleInstance, Song, SynthKind};
use honeydew_audio::device::{AudioRenderer, PingFailure};

/// One beat at 120 BPM and 48 kHz.
const BEAT: usize = 24_000;
/// Tape Delay, Reverb and SuperSaw.
const DELAY: u16 = 18;
const REVERB: u16 = 32;
const SUPERSAW: u16 = 23;

fn energy(samples: &[f32]) -> f32 {
    samples.iter().map(|sample| sample * sample).sum()
}

fn assert_close(actual: &[f32], expected: &[f32]) {
    assert!(energy(expected) > 0.01, "a silent reference");
    assert_eq!(actual.len(), expected.len());
    for (frame, (a, e)) in actual.iter().zip(expected).enumerate() {
        assert!((a - e).abs() <= 1e-5, "frame {frame}: {a} vs {e}");
    }
}

/// The two differ somewhere by more than the comparison's tolerance.
fn differ(a: &[f32], b: &[f32]) -> bool {
    a.iter().zip(b).any(|(a, b)| (a - b).abs() > 1e-3)
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

fn hit(pad: u8, velocity: u8) -> Event {
    Event::Drum {
        pos: 0,
        len_beats: 0.0,
        pad,
        velocity,
        pan: 127,
        value: Event::DEFAULT_PITCH,
        repeat: 0,
    }
}

/// 120 BPM and a MiniSynth channel for each of `channels`, playing its notes from one 16-beat clip.
fn synths(channels: &[&[Event]]) -> Song {
    let mut song = Song {
        tempo_bpm: 120.0,
        ..Song::default()
    };
    for (index, notes) in channels.iter().enumerate() {
        song.add_channel(ChannelKind::Synth(SynthKind::MiniSynth));
        (song.channels[index + 1].lanes[0].add_clip(0, 16.0).events).extend_from_slice(notes);
    }
    song
}

/// 120 BPM and Drums on channel 1 whose first three pads play sample "dc" (0.5, eight beats), with `hits` at beat 0
/// in one 4-beat step clip.
fn drums(hits: &[Event]) -> Song {
    let mut song = Song {
        tempo_bpm: 120.0,
        ..Song::default()
    };
    song.add_channel(ChannelKind::Drums);
    let kit = song.channels[1].drum_kit.as_mut().unwrap();
    for slot in &mut kit.slots[..3] {
        slot.file.path = Some(ContentPath::Plain("dc".into()));
    }
    song.channels[1].lanes[0]
        .add_clip(0, 4.0)
        .events
        .extend_from_slice(hits);
    song
}

/// Every sample file: "quarter" 0.25, any other 0.5, for eight beats.
fn read(path: &ContentPath) -> Option<Vec<u8>> {
    match path {
        ContentPath::Plain(name) => {
            let level = if name.as_str() == "quarter" {
                0.25
            } else {
                0.5
            };
            Some(wav(&vec![level; 8 * BEAT]))
        }
        _ => None,
    }
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

fn engine(song: &Song) -> SongAudioEngine {
    let mut cache = SampleCache::default();
    cache.load(song, &read);
    SongAudioEngine::with_samples(song, 48_000.0, &cache).unwrap()
}

/// `song` played from beat 0: `beats` beats from beat `from`.
fn played(song: &Song, from: f64, beats: f64) -> Vec<f32> {
    let mut engine = engine(song);
    engine.play();
    render(&mut engine, from);
    render(&mut engine, beats)
}

/// `before` played for `beats` beats, then an engine built for `after` takes it over (re-syncing if `resync`): the
/// engine that plays on.
fn edited(before: &Song, after: &Song, resync: bool, beats: f64) -> SongAudioEngine {
    let mut old = engine(before);
    old.play();
    render(&mut old, beats);
    let mut new = engine(after).adopting_edit(resync, Vec::new());
    new.take_over(&mut old);
    new
}

/// Item 7: adding an effect changes nothing else: the delay tail of a note that ended sounds on untouched.
#[test]
fn a_delay_tail_survives_an_effect_added_on_another_channel() {
    let mut song = synths(&[&[note(0.0, 0.25, 60)], &[]]);
    song.channels[1].rack.add(DELAY);
    let mut added = song.clone();
    added.channels[2].rack.add(REVERB);
    let mut engine = edited(&song, &added, false, 1.0);
    assert_close(&render(&mut engine, 1.0), &played(&song, 1.0, 1.0));
}

/// Item 7: a removed effect is destroyed, its tail ends at once; the other channel's tail and the instrument's own
/// release go on.
#[test]
fn removing_an_effect_ends_only_its_tail() {
    let mut song = synths(&[&[note(0.0, 0.25, 60)], &[note(0.0, 0.25, 64)]]);
    let id = song.channels[1].rack.add(DELAY);
    song.channels[2].rack.add(DELAY);
    let mut removed = song.clone();
    removed.channels[1].rack.remove(id);
    assert!(
        differ(&played(&song, 1.0, 1.0), &played(&removed, 1.0, 1.0)),
        "no tail to end"
    );
    let mut engine = edited(&song, &removed, false, 1.0);
    assert_close(&render(&mut engine, 1.0), &played(&removed, 1.0, 1.0));
}

/// Item 7: a preset of the same type is applied to the running instrument: its sounding voice goes on, with the
/// preset's params (here a lower filter cutoff).
#[test]
fn a_same_type_preset_load_keeps_a_sounding_voice() {
    let song = synths(&[&[note(0.0, 8.0, 57)]]);
    let mut loaded = song.clone();
    let mut preset = ModuleInstance::new(0, 99);
    preset.params[9] = 0.3;
    loaded.channels[1].rack.modules[0].load_preset(preset);
    assert_eq!(loaded.channels[1].rack.modules[0].instance_id, 2);
    let mut engine = edited(&song, &loaded, false, 1.0);
    let after = render(&mut engine, 0.5);
    assert!(energy(&after) > 0.01, "the voice was cut");
    assert!(
        differ(&after, &played(&song, 1.0, 0.5)),
        "the preset's params"
    );
    assert_eq!(engine.sounding(1), 1 << 57);
}

/// Item 7: another instrument type destroys the old one, whose voices stop at once, and starts silent: the held key
/// is not played again. The other channel plays on.
#[test]
fn a_type_switch_cuts_only_that_channel() {
    let song = synths(&[&[note(0.0, 8.0, 57)], &[note(0.0, 8.0, 64)]]);
    let mut switched = song.clone();
    let rack = &mut switched.channels[1].rack;
    let id = rack.modules[0].instance_id;
    rack.remove(id);
    rack.add(SUPERSAW);
    let mut engine = edited(&song, &switched, false, 1.0);
    let only_second = synths(&[&[], &[note(0.0, 8.0, 64)]]);
    assert_close(&render(&mut engine, 1.0), &played(&only_second, 1.0, 1.0));
}

/// Item 7: a deleted channel ends at once; the routing is rebuilt and the others re-sync, their covered voices
/// sounding on.
#[test]
fn deleting_a_channel_leaves_the_other_channels_voices() {
    let three = |second: &[Event]| synths(&[&[note(0.0, 8.0, 57)], second, &[note(0.0, 8.0, 67)]]);
    let song = three(&[note(0.0, 8.0, 64)]);
    let mut deleted = song.clone();
    deleted.delete_channel(2);
    let mut engine = edited(&song, &deleted, true, 1.0);
    assert_eq!((engine.sounding(1), engine.sounding(2)), (1 << 57, 1 << 67));
    assert_close(&render(&mut engine, 1.0), &played(&three(&[]), 1.0, 1.0));
}

/// A pad's `AUX` send target: from the next block the pad's whole output goes to the new
/// target, its sounding voice included.
#[test]
fn aux_switches_a_sounding_voices_output() {
    let mut song = drums(&[hit(0, 255)]);
    song.add_channel(ChannelKind::EffectTrack);
    song.channels[2].params.0[0] = 0.3;
    let mut sent = song.clone();
    sent.channels[1].drum_kit.as_mut().unwrap().slots[0]
        .file
        .send_target = sent.channels[2].id;
    assert!(differ(&played(&song, 1.0, 1.0), &played(&sent, 1.0, 1.0)));
    let mut engine = edited(&song, &sent, false, 1.0);
    assert_close(&render(&mut engine, 1.0), &played(&sent, 1.0, 1.0));
}

/// Item 6 (quirk): a pad sample pick replaces the pad's sample under its sounding voice, which reads the new one at
/// the frame it is at. Stretch takes the same path: its render is the pad's new sample.
#[test]
fn a_pad_pick_keeps_the_voice_and_swaps_the_sample() {
    let song = drums(&[hit(0, 255)]);
    let mut picked = song.clone();
    picked.channels[1].drum_kit.as_mut().unwrap().slots[0]
        .file
        .path = Some(ContentPath::Plain("quarter".into()));
    assert!(differ(&played(&song, 1.0, 1.0), &played(&picked, 1.0, 1.0)));
    let mut engine = edited(&song, &picked, false, 1.0);
    assert_close(&render(&mut engine, 1.0), &played(&picked, 1.0, 1.0));
}

/// Item 6: removing a row deletes its hits, moves the higher rows down and destroys its slot, whose voices stop at
/// once with no release; the other pads ring on.
#[test]
fn removing_a_row_cuts_only_that_slot() {
    let song = drums(&[hit(0, 255), hit(1, 128), hit(2, 64)]);
    let mut removed = song.clone();
    removed.channels[1]
        .drum_kit
        .as_mut()
        .unwrap()
        .slots
        .remove(1);
    removed.channels[1].lanes[0].clips[0].events = vec![hit(0, 255), hit(1, 64)];
    let without = drums(&[hit(0, 255), hit(2, 64)]);
    assert!(differ(
        &played(&song, 1.0, 1.0),
        &played(&without, 1.0, 1.0)
    ));
    let mut engine = edited(&song, &removed, false, 1.0);
    assert_close(&render(&mut engine, 1.0), &played(&without, 1.0, 1.0));
}

/// An edit puts a sounding pad in one cut group with another; the other's hit after the edit
/// chokes the voice taken over, as in the edited song played from the start.
#[test]
fn a_cut_group_an_edit_sets_chokes_the_voice_taken_over() {
    let mut at_beat_2 = hit(1, 255);
    if let Event::Drum { pos, .. } = &mut at_beat_2 {
        *pos = 256;
    }
    let grouped = |group: u8| {
        let mut song = drums(&[hit(0, 255), at_beat_2]);
        let slots = &mut song.channels[1].drum_kit.as_mut().unwrap().slots;
        (slots[0].cut_group, slots[1].cut_group) = (group, group);
        song
    };
    let (before, after) = (grouped(0), grouped(2));
    assert!(differ(
        &played(&before, 1.0, 2.0),
        &played(&after, 1.0, 2.0)
    ));
    let mut engine = edited(&before, &after, false, 1.0);
    assert_close(&render(&mut engine, 2.0), &played(&after, 1.0, 2.0));
}

/// Item 7 and the runtime contract on bypass: Enable off skips the module and freezes its state, an edit meanwhile
/// keeps it frozen, and Enable on resumes the tail where it stood.
#[test]
fn bypass_freezes_a_tail_and_re_enable_resumes_it() {
    let mut song = synths(&[&[note(0.0, 0.25, 60)], &[]]);
    let id = song.channels[1].rack.add(DELAY);
    let mut dry = song.clone();
    dry.channels[1].rack.remove(id);
    assert_eq!(
        energy(&played(&dry, 2.0, 1.0)),
        0.0,
        "the note still sounds"
    );
    let tail = played(&song, 2.0, 1.0);
    let mut old = engine(&song);
    old.play();
    render(&mut old, 2.0);
    old.set_param(1, id, 0, 0.0);
    assert_eq!(energy(&render(&mut old, 0.5)), 0.0, "bypassed");
    let mut off = song.clone();
    off.channels[1].rack.get_mut(id).unwrap().params[0] = 0.0;
    off.channels[2].rack.add(REVERB);
    let mut engine = engine(&off).adopting_edit(false, Vec::new());
    engine.take_over(&mut old);
    assert_eq!(energy(&render(&mut engine, 0.25)), 0.0, "still bypassed");
    engine.set_param(1, id, 0, 1.0);
    assert_close(&render(&mut engine, 1.0), &tail);
}

/// A live key held through an edit sounds on and its key-up releases it.
#[test]
fn a_live_key_held_through_an_edit_is_released_by_its_key_up() {
    let song = synths(&[&[], &[]]);
    let held = |edit: fn(&mut Song)| {
        let mut old = engine(&song);
        old.note_on(1, 60, 0.8);
        render(&mut old, 0.25);
        let mut edited = song.clone();
        edit(&mut edited);
        let mut engine = engine(&edited).adopting_edit(false, Vec::new());
        engine.take_over(&mut old);
        engine
    };
    let mut engine = held(|song| _ = song.channels[2].rack.add(REVERB));
    assert_eq!(engine.sounding(1), 1 << 60);
    assert!(energy(&render(&mut engine, 0.25)) > 0.01, "cut");
    engine.note_off(1, 60);
    render(&mut engine, 2.0);
    assert_eq!(engine.sounding(1), 0);
    assert_eq!(energy(&render(&mut engine, 0.25)), 0.0, "never released");
}

/// Moved modules keep their state, and notes continue through a tempo drag: a live key held
/// through an Arpeggiator and a Transpose sounds on through an edit that moves the Transpose above the Arpeggiator, and
/// through the edit at the end of a tempo drag; its key-up then releases everything made from it.
#[test]
fn a_live_key_sounds_on_through_a_moved_note_effect_and_a_tempo_drag() {
    let mut song = synths(&[&[]]);
    let rack = &mut song.channels[1].rack;
    rack.insert(0, 51);
    let transpose = rack.insert(1, 48);
    rack.get_mut(transpose).unwrap().params[1] = 5.0 / 8.0;
    for drag in [false, true] {
        let mut old = engine(&song);
        old.note_on(1, 60, 0.8);
        render(&mut old, 0.6);
        let mut edited = song.clone();
        if drag {
            old.set_tempo(90.0);
            render(&mut old, 0.6);
            edited.set_tempo(90.0);
        } else {
            edited.channels[1].rack.move_up(transpose);
        }
        let mut engine = engine(&edited).adopting_edit(false, Vec::new());
        engine.take_over(&mut old);
        assert_eq!(engine.sounding(1), 1 << 60, "drag {drag}");
        assert!(energy(&render(&mut engine, 1.0)) > 0.01, "drag {drag}: cut");
        engine.note_off(1, 60);
        render(&mut engine, 2.0);
        assert_eq!(engine.sounding(1), 0);
        assert_eq!(
            energy(&render(&mut engine, 0.25)),
            0.0,
            "drag {drag}: never released"
        );
    }
}

/// `song` with key `key` held on channel 1 from frame 0 for a quarter beat, then the engine for the song `edit` leaves
/// takes over: that engine, and one that played on unedited.
fn held_through(
    mut song: Song,
    key: u8,
    edit: impl Fn(&mut Song),
) -> (SongAudioEngine, SongAudioEngine) {
    let (mut old, mut unedited) = (engine(&song), engine(&song));
    for engine in [&mut old, &mut unedited] {
        engine.note_on(1, key, 0.8);
        render(engine, 0.25);
    }
    edit(&mut song);
    let mut edited = engine(&song).adopting_edit(false, Vec::new());
    edited.take_over(&mut old);
    (edited, unedited)
}

/// The key `key` held through `held_through` sounds on as on the engine never edited, and its key-up then releases it
/// as there (nothing left stuck).
fn sounds_on_then_ends(
    case: &str,
    key: u8,
    (mut engine, mut unedited): (SongAudioEngine, SongAudioEngine),
) {
    assert_eq!(engine.sounding(1), 1 << key, "{case}");
    assert_close(&render(&mut engine, 1.0), &render(&mut unedited, 1.0));
    engine.note_off(1, key);
    unedited.note_off(1, key);
    let released = render(&mut engine, 2.25);
    assert!(
        !differ(&released, &render(&mut unedited, 2.25)),
        "{case}: stuck"
    );
    assert_eq!(engine.sounding(1), 0, "{case}");
}

/// A Transpose an octave up at the top of channel 1's rack, above its MiniSynth.
fn octave_up(song: &mut Song) {
    let id = song.channels[1].rack.insert(0, 48);
    song.channels[1].rack.get_mut(id).unwrap().params[1] = 5.0 / 8.0;
}

/// The instrument's voices and the other modules are untouched: a note effect added above, removed from above, or
/// moved below the instrument while a live key is held releases nothing (the voice plays on as on an engine never
/// edited), and the key-up then releases the voice the key started, whatever the key maps to now (no 72 left stuck by
/// a Transpose that moved or went).
#[test]
fn a_note_effect_added_removed_or_moved_under_a_held_key_releases_nothing_and_leaves_nothing_stuck()
{
    let remove = |song: &mut Song| {
        let rack = &mut song.channels[1].rack;
        rack.remove(rack.modules[0].instance_id);
    };
    let move_below = |song: &mut Song| {
        let rack = &mut song.channels[1].rack;
        rack.move_down(rack.modules[0].instance_id);
    };
    let synth = synths(&[&[]]);
    let mut transposed = synth.clone();
    octave_up(&mut transposed);
    sounds_on_then_ends("added", 60, held_through(synth, 60, octave_up));
    sounds_on_then_ends("removed", 60, held_through(transposed.clone(), 60, remove));
    sounds_on_then_ends("moved below", 60, held_through(transposed, 60, move_below));
}

/// A held key's note-off follows its instrument by instance, not by rack row: a second instrument
/// inserted above the one playing it (now row 1) leaves nothing stuck.
#[test]
fn a_held_key_ends_on_its_instrument_after_another_is_inserted_above_it() {
    let insert = |song: &mut Song| {
        let rack = &mut song.channels[1].rack;
        rack.insert(0, rack.modules[0].type_id);
    };
    sounds_on_then_ends("inserted", 60, held_through(synths(&[&[]]), 60, insert));
}

/// A drum channel's live key (off a notes lane: the pads hear it) held through an edit elsewhere ends
/// on the pad it started.
#[test]
fn a_drum_channels_held_key_ends_on_its_pad_after_an_edit() {
    let add = |song: &mut Song| song.add_channel(ChannelKind::Synth(SynthKind::MiniSynth));
    sounds_on_then_ends("drums", 48, held_through(drums(&[]), 48, add));
}

/// 120 BPM and a DW Sampler on channel 1 whose one `CSTM` zone plays "My Samples/Loop.wav" over every key, holding A3
/// from beat 0 for eight beats.
fn dw_song() -> Song {
    let mut song = synths(&[&[note(0.0, 8.0, 57)]]);
    let module = &mut song.channels[1].rack.modules[0];
    *module = ModuleInstance::new(1, module.instance_id);
    let mut zone = b"@L@\0My Samples/Loop.wav".to_vec();
    zone.resize(1024, 0);
    zone.extend([48, 0, 127, 0, 127]);
    for word in [1.0f32.to_bits(), 0.5f32.to_bits(), 2, 100, 30_000] {
        zone.extend(word.to_le_bytes());
    }
    for value in [0.0f32, 1.0, 1.0, 0.1] {
        zone.extend(value.to_le_bytes());
    }
    let mut blob = b"10WDZONE".to_vec();
    blob.extend((zone.len() as u32).to_le_bytes());
    blob.extend(zone);
    module.file.custom_data = Some(blob);
    song
}

/// An instrument's samples are part of what it is built from. A sample read again under the same path
/// (an audio editor's rewrite) builds the DW Sampler again, as its new data would (a worker-loaded instrument's
/// new data starts it afresh, silent); the same sample keeps the playing one.
#[test]
fn a_sample_rewritten_under_its_path_rebuilds_the_instrument_playing_it() {
    let song = dw_song();
    let cache = |level: f32| {
        let mut cache = SampleCache::default();
        cache.load(&song, &|_| Some(wav(&vec![level; 8 * BEAT])));
        cache
    };
    let (first, rewritten) = (cache(0.5), cache(0.25));
    for (samples, sounds) in [(&first, true), (&rewritten, false)] {
        let mut old = SongAudioEngine::with_samples(&song, 48_000.0, &first).unwrap();
        old.play();
        assert!(energy(&render(&mut old, 1.0)) > 0.01, "a silent DW Sampler");
        let new = SongAudioEngine::with_samples(&song, 48_000.0, samples).unwrap();
        let mut new = new.adopting_edit(false, Vec::new());
        new.take_over(&mut old);
        assert_eq!(energy(&render(&mut new, 0.5)) > 0.0, sounds);
    }
}

/// The edit at the end of a tempo drag (its song at the dragged tempo, an effect
/// added elsewhere) takes over a fractional playhead counted at the old build tempo; the beat is kept, the voice goes
/// on, and the next note starts on its beat at the new tempo.
#[test]
fn an_edit_at_the_end_of_a_tempo_drag_keeps_the_beat() {
    let song = synths(&[&[note(0.0, 8.0, 57), note(3.0, 1.0, 64)], &[]]);
    let mut old = engine(&song);
    old.play();
    render(&mut old, 1.0);
    old.set_tempo(160.0);
    // 0.75 of a beat's output frames at 120 BPM is a whole beat at 160.
    render(&mut old, 0.75);
    assert_eq!(old.transport_tick(), 2 * 128);
    let mut dragged = song.clone();
    dragged.set_tempo(160.0);
    dragged.channels[2].rack.add(REVERB);
    let mut engine = engine(&dragged).adopting_edit(false, Vec::new());
    engine.take_over(&mut old);
    assert_eq!(
        (engine.transport_tick(), engine.sounding(1)),
        (2 * 128, 1 << 57)
    );
    // A beat at 160 BPM is 0.75 of `render`'s beats: E4 at beat 3.
    render(&mut engine, 0.7);
    assert_eq!(engine.sounding(1), 1 << 57);
    render(&mut engine, 0.1);
    assert_eq!(engine.sounding(1), 1 << 57 | 1 << 64);
}

/// `song` played for `beats` beats (after `prepare`), then taken over by an engine for the same song at `tempo`
/// BPM: the engine that plays on.
fn retimed(
    song: &Song,
    tempo: f64,
    prepare: impl FnOnce(&mut SongAudioEngine),
    beats: f64,
) -> SongAudioEngine {
    let mut old = engine(song);
    prepare(&mut old);
    old.play();
    render(&mut old, beats);
    let mut faster = song.clone();
    faster.set_tempo(tempo);
    let mut new = engine(&faster).adopting_edit(false, Vec::new());
    new.take_over(&mut old);
    new
}

/// A count-in running through an edit at another tempo keeps its beats: 3 of its 4 beats
/// left at 120 BPM are 3 at 60.
#[test]
fn a_count_in_through_an_edit_at_another_tempo_keeps_its_beats() {
    let mut engine = retimed(&synths(&[&[]]), 60.0, |e| e.count_in(4), 1.0);
    assert!(engine.counting_in());
    // 3 beats at 60 BPM are 6 of `render`'s 120 BPM beats.
    render(&mut engine, 5.99);
    assert!(engine.counting_in(), "ended early");
    render(&mut engine, 0.02);
    assert!(!engine.counting_in());
}

/// The live clock (frames output since the build) goes on through an edit. A live key is timed
/// on it: pressed 2 beats in and taken over before it sounds, it sounds in the new engine's first block, not 2 beats on.
#[test]
fn the_live_clock_goes_on_through_an_edit() {
    let song = synths(&[&[]]);
    let mut old = engine(&song);
    old.play();
    render(&mut old, 2.0);
    old.note_on(1, 60, 0.8);
    let mut new = engine(&song).adopting_edit(false, Vec::new());
    new.take_over(&mut old);
    assert!(energy(&render(&mut new, 0.05)) > 0.0);
}

/// A latency measurement goes on through an edit, and its result stays
/// to be read in the engine after.
#[test]
fn a_latency_measurement_goes_on_through_an_edit() {
    let song = synths(&[&[]]);
    let mut old = engine(&song);
    old.ping(3);
    let mut new = engine(&song).adopting_edit(false, Vec::new());
    new.take_over(&mut old);
    assert_eq!(new.ping_report(), Some((3, None)));
    // Without an input it fails at the next render; the next engine reports that.
    render(&mut new, 0.01);
    let unknown = Some((3, Some(Err(PingFailure::Unknown))));
    assert_eq!(new.ping_report(), unknown);
    let mut newer = engine(&song).adopting_edit(false, Vec::new());
    newer.take_over(&mut new);
    assert_eq!(newer.ping_report(), unknown);
}

/// A metronome click sounding at an edit rings on.
#[test]
fn a_metronome_click_rings_on_through_an_edit() {
    let song = synths(&[&[]]);
    let on = |e: &mut SongAudioEngine| e.set_metronome(true, true, 1.0);
    // 100 frames into beat 1's click.
    let mut engine = retimed(&song, 120.0, on, 1.0 + 100.0 / BEAT as f64);
    assert!(
        energy(&render(&mut engine, 0.01)) > 0.01,
        "the click was cut"
    );
}

/// A tempo drag during a running count-in keeps its beats: 3 of its 4 beats left at 120 BPM
/// are 3 at 60.
#[test]
fn a_tempo_drag_during_a_count_in_keeps_its_beats() {
    let mut engine = engine(&synths(&[&[]]));
    engine.count_in(4);
    engine.play();
    render(&mut engine, 1.0);
    engine.set_tempo(60.0);
    // 3 beats at 60 BPM are 6 of `render`'s 120 BPM beats.
    render(&mut engine, 5.99);
    assert!(engine.counting_in(), "ended early");
    render(&mut engine, 0.02);
    assert!(!engine.counting_in());
}

/// A track reorder (`Song::set_playlist`) moves channels to other indices. The
/// edit's engine matches channels by id as it takes over, so every voice sounds on, now at its channel's new index, and
/// the output is what the reordered song plays.
#[test]
fn a_track_reorder_keeps_every_voice() {
    let song = synths(&[
        &[note(0.0, 8.0, 57)],
        &[note(0.0, 8.0, 64)],
        &[note(0.0, 8.0, 67)],
    ]);
    let mut moved = song.clone();
    assert!(moved.set_playlist(&[0, 3, 1, 2]));
    let mut engine = edited(&song, &moved, false, 1.0);
    let sounding = [1, 2, 3].map(|channel| engine.sounding(channel));
    assert_eq!(sounding, [1 << 67, 1 << 57, 1 << 64]);
    assert_close(&render(&mut engine, 1.0), &played(&moved, 1.0, 1.0));
}

/// `engine` renders `frames` frames on a thread: a render that never returns fails the test instead of hanging it.
fn render_or_hang(mut engine: SongAudioEngine, frames: usize) -> SongAudioEngine {
    let (send, receive) = std::sync::mpsc::channel();
    std::thread::spawn(move || {
        let mut output = vec![0.0; frames * 2];
        engine.render(&mut output, 2);
        let _ = send.send(engine);
    });
    let timeout = std::time::Duration::from_secs(10);
    receive
        .recv_timeout(timeout)
        .expect("render never returned")
}

/// A 1-beat count-in played to its last frame at 120 BPM (hotfix HF).
fn on_a_count_ins_last_frame(song: &Song) -> SongAudioEngine {
    let mut engine = engine(song);
    engine.count_in(1);
    engine.play();
    engine.render(&mut vec![0.0; (BEAT - 1) * 2], 2);
    engine
}

/// Hotfix HF: a tempo drag on a count-in's last frame keeps a frame to go (its frame left at 120 BPM is 0.4 at 300),
/// so the count-in ends on its beat as that frame renders; the render loop never stands at its end (it hung).
#[test]
fn a_tempo_drag_on_a_count_ins_last_frame_ends_it_on_its_beat() {
    let mut engine = on_a_count_ins_last_frame(&synths(&[&[]]));
    engine.set_tempo(300.0);
    assert!(engine.counting_in(), "ended early");
    let engine = render_or_hang(engine, 1);
    assert!(!engine.counting_in(), "ended late");
    render_or_hang(engine, 512);
}

/// Hotfix HF: the same through an edit's engine built at 300 BPM taking over.
#[test]
fn a_count_in_on_its_last_frame_through_an_edit_at_a_faster_tempo_ends_on_its_beat() {
    let song = synths(&[&[]]);
    let mut old = on_a_count_ins_last_frame(&song);
    let mut faster = song.clone();
    faster.set_tempo(300.0);
    let mut new = engine(&faster).adopting_edit(false, Vec::new());
    new.take_over(&mut old);
    assert!(new.counting_in(), "ended early");
    let new = render_or_hang(new, 1);
    assert!(!new.counting_in(), "ended late");
    render_or_hang(new, 512);
}

/// A count-in sent before Play (armed) when an edit's engine takes over is still armed: the next
/// Play counts in.
#[test]
fn a_count_in_armed_through_an_edit_counts_in_at_play() {
    let song = synths(&[&[]]);
    let mut old = engine(&song);
    old.count_in(4);
    let mut new = engine(&song).adopting_edit(false, Vec::new());
    new.take_over(&mut old);
    new.play();
    assert!(new.counting_in());
}
