//! Channel sends (`SEND`, params 0x1000/0x2000 + channel id, Enable Send) and per-module key ranges.

use honeydew::audio_engine::SongAudioEngine;
use honeydew::song::{AutomationTarget, ChannelKind, Event, SendState, Song, SynthKind};
use honeydew_audio::device::AudioRenderer;
use honeydew_audio::instrument::KEY_LEARN;

const RATE: f32 = 48_000.0;
const ENABLE_SEND: i32 = 0x1000;
const SEND_LEVEL: i32 = 0x2000;

/// The channel Level law, which Send Level follows.
fn law(level: f32) -> f32 {
    10.0_f32.powf((66.0 * level.powf(0.6) - 60.0) / 20.0)
}

fn energy(samples: &[f32]) -> f32 {
    samples.iter().map(|s| s * s).sum()
}

/// Master, a send channel S (channel 1, id 1) and a MiniSynth A (channel 2, id 2) sending to the master and to S at
/// `level`. Both S and A start at the default sends to the master.
fn send_song(level: f32) -> Song {
    let mut song = Song::default();
    song.add_channel(ChannelKind::EffectTrack);
    song.add_channel(ChannelKind::Synth(SynthKind::MiniSynth));
    let s = song.channels[1].id;
    song.channels[2].file.sends.push(SendState {
        target_channel_id: s,
        level,
        ..SendState::default()
    });
    song
}

/// The peak of channel `channel`'s strip meter (instance 0) over the last render.
fn peak(engine: &SongAudioEngine, channel: usize) -> f32 {
    let index = (0..engine.meter_count())
        .find(|&i| engine.meter_key(i).channel == channel && engine.meter_key(i).instance_id == 0)
        .unwrap();
    engine.meter(index).peak_left
}

/// The peak of channel `channel`'s module `instance_id` over the last render.
fn module_peak(engine: &SongAudioEngine, channel: usize, instance_id: i32) -> f32 {
    let index = (0..engine.meter_count())
        .find(|&i| {
            engine.meter_key(i).channel == channel && engine.meter_key(i).instance_id == instance_id
        })
        .unwrap();
    engine.meter(index).peak_left
}

/// `song` built, `setup` run, then channel `channel` holding key `key` for 4096 frames; the engine and the output.
fn held(
    song: &Song,
    channel: usize,
    key: u8,
    setup: impl FnOnce(&mut SongAudioEngine),
) -> (SongAudioEngine, Vec<f32>) {
    let mut engine = SongAudioEngine::new(song, RATE).unwrap();
    setup(&mut engine);
    engine.note_on(channel, key, 1.0);
    let mut output = vec![0.0; 4096 * 2];
    engine.render(&mut output, 2);
    (engine, output)
}

fn close(a: f32, b: f32) -> bool {
    (a - b).abs() <= 1e-4 * b.abs().max(1e-3)
}

#[test]
fn a_send_adds_the_channel_into_its_target_at_the_send_level() {
    let (engine, _) = held(&send_song(0.8531), 2, 69, |_| {});
    let a = peak(&engine, 2);
    assert!(a > 0.01);
    // S's strip is the default 0.8531 too.
    assert!(
        close(peak(&engine, 1), a * law(0.8531) * law(0.8531)),
        "unity send"
    );
    let (engine, _) = held(&send_song(0.25), 2, 69, |_| {});
    assert!(
        close(peak(&engine, 1), a * law(0.25) * law(0.8531)),
        "quarter send"
    );
    // Without the send S hears nothing.
    let mut song = send_song(0.8531);
    song.channels[2].file.sends.pop();
    let (engine, _) = held(&song, 2, 69, |_| {});
    assert_eq!(peak(&engine, 1), 0.0);
}

#[test]
fn the_master_gets_both_paths() {
    let (_, both) = held(&send_song(0.8531), 2, 69, |_| {});
    let mut song = send_song(0.8531);
    song.channels[2].file.sends.pop();
    let (_, direct) = held(&song, 2, 69, |_| {});
    // Direct plus through S: about twice the amplitude.
    let ratio = (energy(&both) / energy(&direct)).sqrt();
    assert!((1.99..=2.01).contains(&ratio), "{ratio}");
}

#[test]
fn a_channel_without_a_send_has_no_output() {
    let mut song = send_song(0.8531);
    song.channels[2].file.sends.clear();
    let (engine, output) = held(&song, 2, 69, |_| {});
    assert!(peak(&engine, 2) > 0.01, "it still plays its strip");
    assert_eq!(energy(&output), 0.0);
}

#[test]
fn enable_send_and_send_level_are_live_channel_params() {
    let s = 1;
    let (engine, _) = held(&send_song(0.8531), 2, 69, |e| {
        e.set_param(2, 0, ENABLE_SEND + s, 0.5)
    });
    assert_eq!(peak(&engine, 1), 0.0, "Enable Send at 0.5 is off");
    let (on, _) = held(&send_song(0.8531), 2, 69, |e| {
        e.set_param(2, 0, ENABLE_SEND + s, 0.51)
    });
    let full = peak(&on, 1);
    assert!(full > 0.01, "on above 0.5");
    let (engine, _) = held(&send_song(0.8531), 2, 69, |e| {
        e.set_param(2, 0, SEND_LEVEL + s, 0.25)
    });
    assert!(close(peak(&engine, 1), full * law(0.25) / law(0.8531)));
    // The saved values apply, and a live value survives a seek.
    let mut song = send_song(0.8531);
    song.channels[2].file.sends[1].enabled = 0.0;
    let (engine, _) = held(&song, 2, 69, |_| {});
    assert_eq!(peak(&engine, 1), 0.0, "saved Enable Send off");
    let (engine, _) = held(&send_song(0.8531), 2, 69, |e| {
        e.set_param(2, 0, ENABLE_SEND + s, 0.0);
        e.seek(128);
    });
    assert_eq!(peak(&engine, 1), 0.0, "kept over a seek");
}

#[test]
fn send_automation_plays() {
    let mut song = send_song(0.8531);
    song.tempo_bpm = 120.0;
    let s = song.channels[1].id;
    let clip = song.channels[2].lanes[0].add_clip(0, 4.0);
    clip.events.push(Event::Note {
        pos: 0,
        len_beats: 4.0,
        key: 69,
        velocity: 255,
        pan: 127,
        slide: false,
    });
    let lane = song.channels[2].add_automation_lane(AutomationTarget {
        instance_id: 0,
        param: ENABLE_SEND + s,
    });
    // Off for the first beat, on from beat 1.
    lane.add_clip(0, 4.0).events.extend([
        Event::Automation {
            pos: 0,
            value: 0.0,
            control: true,
        },
        Event::Automation {
            pos: 128,
            value: 1.0,
            control: true,
        },
    ]);
    let mut engine = SongAudioEngine::new(&song, RATE).unwrap();
    engine.play();
    let mut output = vec![0.0; 12_000 * 2];
    engine.render(&mut output, 2);
    // The lane steps from 0 to 1 across the first beat: off up to its middle.
    assert_eq!(peak(&engine, 1), 0.0, "off in the first half beat");
    for _ in 0..4 {
        engine.render(&mut output, 2);
    }
    assert!(peak(&engine, 1) > 0.01, "on after beat 1");
    // A Send Level lane drives the level the same way: at most 0.1 (−43.4 dB) in the first tenth of a beat.
    let first_tenth = |song: &Song| {
        let mut engine = SongAudioEngine::new(song, RATE).unwrap();
        engine.play();
        engine.render(&mut vec![0.0; 2400 * 2], 2);
        peak(&engine, 1)
    };
    let mut lanes = song.channels[2].lanes.iter_mut();
    let lane = lanes.find(|lane| lane.target.is_some()).unwrap();
    lane.target = Some(AutomationTarget {
        instance_id: 0,
        param: SEND_LEVEL + s,
    });
    let quiet = first_tenth(&song);
    song.channels[2].lanes.retain(|lane| lane.target.is_none());
    let plain = first_tenth(&song);
    assert!(
        quiet < plain * law(0.1) / law(0.8531) * 1.01,
        "{quiet} {plain}"
    );
}

#[test]
fn bad_sends_are_ignored_and_a_cycle_still_builds() {
    // A duplicate target, a target that does not exist and a send to itself: only the first send to S plays.
    let mut song = send_song(0.8531);
    let (s, a) = (song.channels[1].id, song.channels[2].id);
    let extra = [s, 999, a].map(|target_channel_id| SendState {
        target_channel_id,
        ..SendState::default()
    });
    song.channels[2].file.sends.extend(extra);
    let (engine, _) = held(&song, 2, 69, |_| {});
    let (plain, _) = held(&send_song(0.8531), 2, 69, |_| {});
    assert!(close(peak(&engine, 1), peak(&plain, 1)), "one send to S");
    // S back to A closes a cycle: scanning the channels in order, the send that would close it is dropped (A's to S,
    // as S's to A comes first), and the song still plays.
    song.channels[1].file.sends.push(SendState {
        target_channel_id: a,
        ..SendState::default()
    });
    let (engine, output) = held(&song, 2, 69, |_| {});
    assert_eq!(peak(&engine, 1), 0.0, "A's send to S is the one dropped");
    assert!(energy(&output) > 0.01);
}

#[test]
fn soloing_a_send_target_keeps_its_sources_audible() {
    // B (channel 3) only goes to the master; A feeds S.
    let mut song = send_song(0.8531);
    song.add_channel(ChannelKind::Synth(SynthKind::MiniSynth));
    let audible = |commands: &[(usize, i32, f32)]| {
        let mut engine = SongAudioEngine::new(&song, RATE).unwrap();
        for &(channel, index, value) in commands {
            engine.set_param(channel, 0, index, value);
        }
        engine.note_on(2, 69, 1.0);
        engine.note_on(3, 69, 1.0);
        let mut output = vec![0.0; 4096 * 2];
        engine.render(&mut output, 2);
        [1, 2, 3].map(|channel| peak(&engine, channel) > 0.0)
    };
    assert_eq!(audible(&[]), [true, true, true]);
    assert_eq!(audible(&[(1, 3, 1.0)]), [true, true, false]);
    assert_eq!(audible(&[(2, 3, 1.0)]), [true, true, false]);
    assert_eq!(audible(&[(3, 3, 1.0)]), [false, false, true]);
}

/// A MiniSynth channel with a second MiniSynth: (song, the first's id, the second's id).
fn two_synths() -> (Song, i32, i32) {
    let mut song = Song::default();
    song.add_channel(ChannelKind::Synth(SynthKind::MiniSynth));
    let first = song.channels[1].rack.modules[0].instance_id;
    let second = song.channels[1].rack.add(0);
    (song, first, second)
}

fn set_range(song: &mut Song, instance_id: i32, low: i8, high: i8) {
    let module = song.channels[1].rack.get_mut(instance_id).unwrap();
    (module.file.key_low, module.file.key_high) = (low, high);
}

#[test]
fn notes_outside_a_modules_key_range_never_reach_it() {
    let (mut song, first, second) = two_synths();
    set_range(&mut song, first, 0, 68);
    set_range(&mut song, second, 69, 69);
    let (engine, _) = held(&song, 1, 69, |_| {});
    assert_eq!(module_peak(&engine, 1, first), 0.0, "69 is above 0..=68");
    assert!(
        module_peak(&engine, 1, second) > 0.01,
        "both bounds inclusive"
    );
    let (engine, _) = held(&song, 1, 68, |_| {});
    assert!(module_peak(&engine, 1, first) > 0.01);
    assert_eq!(module_peak(&engine, 1, second), 0.0);
}

#[test]
fn sequenced_notes_follow_the_key_range_too() {
    let (mut song, first, second) = two_synths();
    song.tempo_bpm = 120.0;
    set_range(&mut song, first, 70, 127);
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
    let mut engine = SongAudioEngine::new(&song, RATE).unwrap();
    engine.play();
    let mut output = vec![0.0; 4096 * 2];
    engine.render(&mut output, 2);
    assert_eq!(module_peak(&engine, 1, first), 0.0);
    assert!(module_peak(&engine, 1, second) > 0.01);
}

/// Channel 1's energy over 4096 frames once `release` has run on an engine holding key 69 for 4096 frames.
fn after_release(release: impl FnOnce(&mut SongAudioEngine)) -> f32 {
    let (mut engine, _) = held(&two_synths().0, 1, 69, |_| {});
    release(&mut engine);
    let mut output = vec![0.0; 48_000 * 2];
    engine.render(&mut output, 2);
    engine.render(&mut output[..4096 * 2], 2);
    energy(&output[..4096 * 2])
}

#[test]
fn a_learning_module_hears_no_notes_not_even_note_offs() {
    let (song, first, second) = two_synths();
    let (engine, _) = held(&song, 1, 69, |e| e.set_setting(1, first, KEY_LEARN, 1.0));
    assert_eq!(
        module_peak(&engine, 1, first),
        0.0,
        "no note-on while learning"
    );
    assert!(
        module_peak(&engine, 1, second) > 0.01,
        "the others still play"
    );
    let (engine, _) = held(&song, 1, 69, |e| {
        e.set_setting(1, first, KEY_LEARN, 1.0);
        e.set_setting(1, first, KEY_LEARN, 0.0);
    });
    assert!(module_peak(&engine, 1, first) > 0.01, "learning ended");
    // A note-off while learning never arrives, so the voice holds.
    let released = after_release(|e| e.note_off(1, 69));
    let held = after_release(|e| {
        e.set_setting(1, first, KEY_LEARN, 1.0);
        e.set_setting(1, second, KEY_LEARN, 1.0);
        e.note_off(1, 69);
    });
    assert!(held > 10.0 * released.max(1e-6), "{held} {released}");
}

/// A non-master channel without a `SEND` has no output, so every channel the app makes
/// without a file must come with its send to the master. The ways a channel comes to exist without a load: a new song
/// (the master alone), `Add channel` / `Insert Channel` of every kind (all through `Song::insert_channel`, a new Rack
/// channel's preset included), `Clone Channel`, and `Create FX Channel` (`ui::rack::create_send`,
/// `ui::drums::create_pad_send`, also `insert_channel`). A file's channels get one on read when they have none.
#[test]
fn every_channel_made_without_a_file_sends_to_the_master() {
    let mut song = Song::default();
    assert_eq!(song.channels.len(), 1, "a new song is the master alone");
    assert!(
        song.channels[0].file.sends.is_empty(),
        "the master is the output"
    );
    for kind in ChannelKind::ADDABLE {
        song.add_channel(kind);
    }
    for synth in SynthKind::ALL {
        song.add_channel(ChannelKind::Synth(synth));
    }
    song.insert_channel(1, ChannelKind::Synth(SynthKind::MiniSynth));
    let clone = song.clone_channel(1);
    assert_eq!(clone, 2);
    for (index, channel) in song.channels.iter().enumerate().skip(1) {
        let to_master = channel.file.sends.iter().any(|s| s.target_channel_id == 0);
        assert!(to_master, "channel {index} ({:?})", channel.kind);
    }
    // A save and load keeps them.
    let read = honeydew::flm::read(&honeydew::flm::write(&song).unwrap()).unwrap();
    assert!(read.channels[1..].iter().all(|c| !c.file.sends.is_empty()));
}

#[test]
fn a_new_songs_added_channels_are_heard_at_the_output() {
    // New song, then a MiniSynth, its clone, a Drums channel (the built-in kit) and an FX channel that a second
    // MiniSynth sends to instead of the master.
    let mut song = Song::default();
    song.add_channel(ChannelKind::Synth(SynthKind::MiniSynth));
    let clone = song.clone_channel(1);
    song.add_channel(ChannelKind::Drums);
    song.add_channel(ChannelKind::EffectTrack);
    song.add_channel(ChannelKind::Synth(SynthKind::MiniSynth));
    let fx = song.channels[4].id;
    song.channels[5].file.sends[0].target_channel_id = fx;
    let mut cache = honeydew::sample_cache::SampleCache::default();
    cache.load(&song, &|_| None);
    for (channel, key) in [(1, 69), (clone, 69), (3, 48), (5, 69)] {
        let mut engine = SongAudioEngine::with_samples(&song, RATE, &cache).unwrap();
        engine.note_on(channel, key, 1.0);
        let mut output = vec![0.0; 4096 * 2];
        engine.render(&mut output, 2);
        assert!(energy(&output) > 0.01, "channel {channel}");
    }
}

/// A channel the user disconnected (its only send removed) stays silent after a save and a load. The format
/// wants a `SEND` on every non-master channel, so the writer writes the default one to the master with Enable 0; it
/// reads back as a MASTER send row switched off.
#[test]
fn a_disconnected_channel_stays_silent_after_save_and_load() {
    let mut song = Song::default();
    song.add_channel(ChannelKind::Synth(SynthKind::MiniSynth));
    song.channels[1].file.sends.clear();
    let (_, output) = held(&song, 1, 69, |_| {});
    assert_eq!(energy(&output), 0.0, "disconnected");
    let read = honeydew::flm::read(&honeydew::flm::write(&song).unwrap()).unwrap();
    let sends = &read.channels[1].file.sends;
    assert_eq!(
        sends
            .iter()
            .map(|s| (s.target_channel_id, s.enabled))
            .collect::<Vec<_>>(),
        [(0, 0.0)]
    );
    let (engine, output) = held(&read, 1, 69, |_| {});
    assert!(peak(&engine, 1) > 0.01, "it still plays its strip");
    assert_eq!(energy(&output), 0.0, "silent after the load");
}

/// A seek restores a send's saved Enable before the automation that switched it off.
#[test]
fn a_seek_restores_the_send_enable_an_automation_lane_switched_off() {
    let mut song = send_song(0.8531);
    let s = song.channels[1].id;
    // Enable Send to S off from beat 1 on (the clip starts there, so nothing sets it before).
    let lane = song.channels[2].add_automation_lane(AutomationTarget {
        instance_id: 0,
        param: ENABLE_SEND + s,
    });
    lane.add_clip(128, 4.0).events.push(Event::Automation {
        pos: 0,
        value: 0.0,
        control: true,
    });
    let mut engine = SongAudioEngine::new(&song, RATE).unwrap();
    engine.play();
    // Past beat 1 (140 BPM: 20 571 frames).
    let mut output = vec![0.0; 24_000 * 2];
    engine.render(&mut output, 2);
    engine.note_on(2, 69, 1.0);
    let mut block = vec![0.0; 4096 * 2];
    engine.render(&mut block, 2);
    assert_eq!(peak(&engine, 1), 0.0, "off after beat 1");
    engine.seek(0);
    engine.note_on(2, 69, 1.0);
    engine.render(&mut block, 2);
    assert!(peak(&engine, 1) > 0.01, "on again at 0");
}

/// A live key off a notes lane (drums, automation or audio current) goes once, after the note effects, to
/// every instrument, and still only to those whose key range holds it, note-offs included.
#[test]
fn a_live_key_off_a_notes_lane_follows_the_key_range() {
    let (mut song, first, second) = two_synths();
    set_range(&mut song, first, 0, 68);
    let off_lane = |e: &mut SongAudioEngine| e.set_live_lane(1, false);
    let (engine, _) = held(&song, 1, 69, off_lane);
    assert_eq!(module_peak(&engine, 1, first), 0.0, "69 is above 0..=68");
    assert!(module_peak(&engine, 1, second) > 0.01);
    // A learning module gets no note-off either: its voice from before the learn holds.
    let tail = |learn: bool| {
        let (mut engine, _) = held(&song, 1, 60, off_lane);
        if learn {
            engine.set_setting(1, first, KEY_LEARN, 1.0);
        }
        engine.note_off(1, 60);
        let mut output = vec![0.0; 48_000 * 2];
        engine.render(&mut output, 2);
        engine.render(&mut output[..4096 * 2], 2);
        module_peak(&engine, 1, first)
    };
    assert_eq!(tail(false), 0.0, "released");
    assert!(tail(true) > 0.01, "held: the note-off never came");
}

/// MiniSynth on channel 1 with range `range`, holding an anchor note 60 for 4 beats at 120 BPM and, with `slide`,
/// a slide note to 72 at beat 1 for one beat. Its output over 3 beats.
fn slid(range: (i8, i8), slide: bool) -> Vec<f32> {
    let mut song = Song {
        tempo_bpm: 120.0,
        ..Song::default()
    };
    song.add_channel(ChannelKind::Synth(SynthKind::MiniSynth));
    let id = song.channels[1].rack.modules[0].instance_id;
    set_range(&mut song, id, range.0, range.1);
    let note = |pos, key, len_beats, slide| Event::Note {
        pos,
        len_beats,
        key,
        velocity: 200,
        pan: 127,
        slide,
    };
    let clip = song.channels[1].lanes[0].add_clip(0, 4.0);
    clip.events.push(note(0, 60, 4.0, false));
    if slide {
        clip.events.push(note(128, 72, 1.0, true));
    }
    let mut engine = SongAudioEngine::new(&song, RATE).unwrap();
    engine.play();
    let mut output = vec![0.0; 72_000 * 2];
    engine.render(&mut output, 2);
    output
}

#[test]
fn a_slide_outside_a_modules_key_range_never_reaches_it() {
    // A slide event is the slide note's, so its key (72) is the one the range checks.
    assert_ne!(
        slid((0, 127), true),
        slid((0, 127), false),
        "the slide glides"
    );
    assert_eq!(
        slid((0, 64), true),
        slid((0, 64), false),
        "72 is out of 0..=64"
    );
}

/// Master, B (channel 1) and A (channel 2), MiniSynths. B's Compressor (−16 dB, 20:1, no make-up gain) is keyed from A
/// when `keyed`; A is muted with its fader down (a ghost key), so the output is B's alone.
fn keyed_song(keyed: bool) -> Song {
    let mut song = Song::default();
    song.add_channel(ChannelKind::Synth(SynthKind::MiniSynth));
    song.add_channel(ChannelKind::Synth(SynthKind::MiniSynth));
    let a = &mut song.channels[2];
    (a.params.0[0], a.params.0[2]) = (0.0, 1.0);
    let a = a.id;
    let b = &mut song.channels[1].rack;
    let id = b.add(8);
    let compressor = b.get_mut(id).unwrap();
    for (index, value) in [(1, 0.6), (2, 1.0), (5, 0.0), (7, 0.0)] {
        compressor.params[index] = value;
    }
    compressor.sidechain = keyed.then_some(a);
    song
}

/// `song` with B holding a quiet key and, if `a_plays`, A a loud one: the output.
fn keyed_output(song: &Song, a_plays: bool) -> Vec<f32> {
    let mut engine = SongAudioEngine::new(song, RATE).unwrap();
    engine.note_on(1, 69, 0.1);
    if a_plays {
        engine.note_on(2, 45, 1.0);
    }
    let mut output = vec![0.0; 8192 * 2];
    engine.render(&mut output, 2);
    output
}

#[test]
fn a_keyed_compressor_ducks_its_channel_while_the_source_plays() {
    // B comes first in channel order, yet runs after A, its key. A is silent in the output.
    let off = keyed_output(&keyed_song(false), true);
    assert!(energy(&off) > 0.01);
    assert_eq!(keyed_output(&keyed_song(false), false), off, "A muted");
    assert_eq!(keyed_output(&keyed_song(true), false), off, "a silent key");
    // The key is A's rack output before its fader and mute: B ducks under the muted A's burst.
    let ducked = keyed_output(&keyed_song(true), true);
    assert!(
        energy(&ducked) < 0.5 * energy(&off),
        "{} {}",
        energy(&ducked),
        energy(&off)
    );
    // Nor does a solo change the key: A unmuted at unity but soloed out (B soloed) keys B alike, and is not heard.
    let mut soloed = keyed_song(true);
    let a = &mut soloed.channels[2].params.0;
    (a[0], a[2]) = (0.8531, 0.0);
    soloed.channels[1].params.0[3] = 1.0;
    assert_eq!(keyed_output(&soloed, true), ducked);
    // A keyed back from B would close a cycle: that key is dropped, and B still ducks.
    let mut cycle = keyed_song(true);
    let b = cycle.channels[1].id;
    let id = cycle.channels[2].rack.add(8);
    cycle.channels[2].rack.get_mut(id).unwrap().sidechain = Some(b);
    assert!(energy(&keyed_output(&cycle, true)) < 0.5 * energy(&off));
}
