//! The engine's transport commands: the loop range, all notes off and the
//! peaks held over a render call. Rendered offline through `AudioRenderer`; no device.

use honeydew::audio_engine::SongAudioEngine;
use honeydew::song::{AutomationTarget, ChannelKind, Event, Song, SynthKind};
use honeydew_audio::device::AudioRenderer;

fn note(pos: u32, len_beats: f64) -> Event {
    Event::Note {
        pos,
        len_beats,
        key: 69,
        velocity: 204,
        pan: 127,
        slide: false,
    }
}

/// 120 BPM, MiniSynth on channel 1 (instance 2) with a 16-beat clip holding `notes`.
fn song(notes: &[Event]) -> Song {
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

/// The left channel of `frames` frames rendered in blocks of `block`, from where the engine is.
fn render(engine: &mut SongAudioEngine, frames: usize, block: usize) -> Vec<f32> {
    let mut left = Vec::with_capacity(frames);
    let mut output = vec![0.0; block * 2];
    while left.len() < frames {
        let n = block.min(frames - left.len());
        engine.render(&mut output[..n * 2], 2);
        left.extend(output[..n * 2].iter().step_by(2));
    }
    left
}

fn energy(samples: &[f32]) -> f32 {
    samples.iter().map(|sample| sample * sample).sum()
}

fn first_sound(samples: &[f32], from: usize) -> usize {
    from + samples[from..].iter().position(|s| *s != 0.0).unwrap()
}

/// Frame of song tick `tick` at 120 BPM (128 ticks a beat).
fn frame(tick: u64, rate: f32) -> usize {
    (tick as f64 * 60.0 * f64::from(rate) / (120.0 * 128.0)).ceil() as usize
}

#[test]
fn the_wrap_lands_on_the_loop_end_frame_for_any_block_size() {
    // Loop beats 1 to 3; a short note at the loop start that has died away well before the end.
    let mut song = song(&[note(128, 0.25)]);
    song.play_loop = Some((128, 384));
    for rate in [44_100.0, 48_000.0] {
        let (start, end) = (frame(128, rate), frame(384, rate));
        let mut reference: Option<Vec<f32>> = None;
        for block in [1, 64, 1024, 4096] {
            let mut engine = SongAudioEngine::new(&song, rate).unwrap();
            engine.play();
            let out = render(&mut engine, end + start, block);
            let onset = first_sound(&out, 0);
            assert!(
                (start..start + 8).contains(&onset),
                "{rate} {block}: {onset}"
            );
            assert_eq!(energy(&out[end - 4000..end]), 0.0, "{rate} {block}");
            // Pass 2 starts at the end frame exactly: the note sounds again as far after it as it did after the start.
            assert_eq!(
                first_sound(&out, end - 4000) - end,
                onset - start,
                "{rate} {block}"
            );
            assert!(energy(&out[end..end + 4000]) > 0.01);
            match &reference {
                Some(reference) => assert!(out == *reference, "{rate} {block}"),
                None => reference = Some(out),
            }
        }
    }
}

#[test]
fn the_automation_value_at_the_loop_start_is_restored_after_a_wrap() {
    // A note over the whole loop [128, 384). The instrument's saved Mix is 0; its automation rises from 0 at tick 0 to
    // 1 at 100 and falls to 0 at 256. So the value at the loop start (1) is neither the saved one nor a point at the
    // start: the wrap must restore it from the points before the start.
    let mut song = song(&[note(128, 2.0)]);
    song.play_loop = Some((128, 384));
    song.channels[1].rack.get_mut(2).unwrap().common.0[1] = 0.0;
    let lane = song.channels[1].add_automation_lane(AutomationTarget {
        instance_id: 2,
        param: AutomationTarget::MIX,
    });
    lane.add_clip(0, 16.0).events.extend([
        Event::Automation {
            pos: 0,
            value: 0.0,
            control: true,
        },
        Event::Automation {
            pos: 100,
            value: 1.0,
            control: true,
        },
        Event::Automation {
            pos: 250,
            value: 1.0,
            control: true,
        },
        Event::Automation {
            pos: 256,
            value: 0.0,
            control: true,
        },
    ]);
    let rate = 48_000.0;
    let mut engine = SongAudioEngine::new(&song, rate).unwrap();
    engine.play();
    let end = frame(384, rate);
    let out = render(&mut engine, end + 12_000, 1024);
    let mix_off = frame(256, rate);
    assert!(energy(&out[frame(128, rate)..mix_off]) > 0.01);
    assert_eq!(energy(&out[mix_off + 100..end]), 0.0);
    assert!(energy(&out[end..]) > 0.01, "Mix stayed 0 after the wrap");
}

#[test]
fn a_loop_end_on_a_callback_boundary_wraps_in_that_callback() {
    // 48 kHz, 120 BPM: a beat is 24 000 frames, 50 callbacks of 480. The loop [128, 256) ends after 100 callbacks.
    let mut song = song(&[]);
    song.play_loop = Some((128, 256));
    let mut engine = SongAudioEngine::new(&song, 48_000.0).unwrap();
    engine.play();
    let mut output = [0.0; 480 * 2];
    for _ in 0..100 {
        engine.render(&mut output, 2);
    }
    assert_eq!(engine.transport_tick(), 128);
}

#[test]
fn no_voice_is_stuck_across_a_wrap() {
    // A note that starts at tick 300 and ends after the loop end (384): the wrap releases it every pass, so the
    // stretch from the loop start to its next note-on is silent once the release has died away.
    let mut song = song(&[note(300, 2.0)]);
    song.play_loop = Some((128, 384));
    let rate = 48_000.0;
    let mut engine = SongAudioEngine::new(&song, rate).unwrap();
    engine.play();
    let (start, end, on) = (frame(128, rate), frame(384, rate), frame(300, rate));
    let pass = end - start;
    let out = render(&mut engine, start + 4 * pass, 777);
    for k in 0..4 {
        let at = start + k * pass;
        assert!(
            energy(&out[at + (on - start)..at + pass]) > 0.01,
            "pass {k}"
        );
        if k > 0 {
            // AMP Rel 0.07 releases within 0.14 s (6720 frames).
            assert_eq!(energy(&out[at + 8000..at + (on - start)]), 0.0, "pass {k}");
        }
    }
}

#[test]
fn all_notes_off_releases_a_held_keyboard_note_and_the_transport_keeps_going() {
    let mut engine = SongAudioEngine::new(&song(&[]), 48_000.0).unwrap();
    engine.note_on(1, 69, 1.0);
    engine.play();
    assert!(energy(&render(&mut engine, 4096, 4096)) > 0.01);
    let tick = engine.transport_tick();
    engine.all_notes_off();
    render(&mut engine, 8192, 1024);
    assert_eq!(energy(&render(&mut engine, 4096, 1024)), 0.0);
    assert!(engine.transport_tick() > tick + 50);
    // A setting for a module without one, or no module, is ignored.
    engine.set_setting(1, 2, 0, 0.5);
    engine.set_setting(9, 9, 0, f32::NAN);
}

#[test]
fn a_playhead_at_or_past_the_loop_end_jumps_to_its_start() {
    let rate = 48_000.0;
    let mut engine = SongAudioEngine::new(&song(&[]), rate).unwrap();
    // No loop: it plays on.
    engine.seek(500);
    engine.play();
    render(&mut engine, 64, 64);
    assert_eq!(engine.transport_tick(), 500);
    // Past the end, and at it.
    for at in [500, 384] {
        engine.set_loop(Some((128, 384)));
        engine.seek(at);
        render(&mut engine, 64, 64);
        assert_eq!(engine.transport_tick(), 128, "{at}");
    }
    // Before the start it plays into the loop.
    engine.seek(100);
    render(&mut engine, 64, 64);
    assert_eq!(engine.transport_tick(), 100);
    // Stopped, nothing moves.
    engine.stop();
    engine.seek(400);
    render(&mut engine, 64, 64);
    assert_eq!(engine.transport_tick(), 400);
    // Empty and inverted ranges are no loop.
    engine.play();
    for range in [(384, 128), (200, 200)] {
        engine.set_loop(Some(range));
        engine.seek(400);
        render(&mut engine, 64, 64);
        assert_eq!(engine.transport_tick(), 400, "{range:?}");
    }
    engine.set_loop(Some((128, 384)));
    engine.set_loop(None);
    render(&mut engine, 64, 64);
    assert_eq!(engine.transport_tick(), 400);
    // The song's own play loop, read at build.
    let mut looped = song(&[]);
    looped.play_loop = Some((128, 384));
    let mut engine = SongAudioEngine::new(&looped, rate).unwrap();
    engine.seek(400);
    engine.play();
    render(&mut engine, 64, 64);
    assert_eq!(engine.transport_tick(), 128);
}

#[test]
fn meters_hold_the_peak_of_every_segment_of_a_render_call() {
    // A keyboard note; the instrument's Mix falls to 0 at tick 16, so the call's last segments are silent.
    let mut song = song(&[]);
    let lane = song.channels[1].add_automation_lane(AutomationTarget {
        instance_id: 2,
        param: AutomationTarget::MIX,
    });
    lane.add_clip(0, 16.0).events.extend([
        Event::Automation {
            pos: 0,
            value: 1.0,
            control: true,
        },
        Event::Automation {
            pos: 15,
            value: 1.0,
            control: true,
        },
        Event::Automation {
            pos: 16,
            value: 0.0,
            control: true,
        },
    ]);
    let mut engine = SongAudioEngine::new(&song, 48_000.0).unwrap();
    let meter = (0..engine.meter_count())
        .find(|&i| engine.meter_key(i).channel == 1 && engine.meter_key(i).instance_id == 2)
        .unwrap();
    engine.note_on(1, 69, 1.0);
    engine.play();
    let out = render(&mut engine, 8192, 8192);
    assert_eq!(energy(&out[4000..]), 0.0);
    assert!(engine.meter(meter).peak_left > 0.01);
    assert!(engine.meter(0).peak_left > 0.01);
    // The next call holds its own peaks only.
    render(&mut engine, 1024, 1024);
    assert_eq!(engine.meter(meter).peak_left, 0.0);
}

#[test]
fn an_effects_analysis_and_input_peak_reach_its_meter() {
    // A held note through a Limiter: its history fills as the engine renders, keyed by the module's meter.
    let mut song = song(&[]);
    let id = song.channels[1].rack.add(28);
    let mut engine = SongAudioEngine::new(&song, 48_000.0).unwrap();
    let meter = (0..engine.meter_count())
        .find(|&i| engine.meter_key(i).channel == 1 && engine.meter_key(i).instance_id == id)
        .unwrap();
    let history = |engine: &SongAudioEngine| {
        honeydew_audio::analysis::limiter(engine.analysis(meter)).unwrap()
    };
    assert!(history(&engine).input.iter().all(|&f| f == 0.0));
    engine.note_on(1, 69, 1.0);
    engine.play();
    render(&mut engine, 48_000 / 4, 512);
    // Five 50 ms entries of the note, the newest last.
    let filled = history(&engine).input.iter().filter(|&&f| f > 0.0).count();
    assert_eq!(filled, 5);
    assert!(engine.meter(meter).input_peak > 0.01);
    // A channel and an instrument have none.
    assert!(engine.analysis(0).is_empty());
}

#[test]
fn an_arpeggiator_and_a_note_echo_show_their_live_state_through_their_meters() {
    // Their notes are worked out ahead; the engine looks their state up where the transport stands.
    let mut song = song(&[note(0, 4.0)]);
    let echo = song.channels[1].rack.insert(0, 47);
    let arp = song.channels[1].rack.insert(0, 51);
    let mut engine = SongAudioEngine::new(&song, 48_000.0).unwrap();
    let meter = |engine: &SongAudioEngine, id: i32| {
        (0..engine.meter_count())
            .find(|&i| engine.meter_key(i).channel == 1 && engine.meter_key(i).instance_id == id)
            .unwrap()
    };
    let (arp, echo) = (meter(&engine, arp), meter(&engine, echo));
    let arp_state = |engine: &SongAudioEngine| {
        honeydew_audio::analysis::arpeggiator(engine.analysis(arp)).unwrap()
    };
    let echo_state = |engine: &SongAudioEngine| {
        honeydew_audio::analysis::note_echo(engine.analysis(echo)).unwrap()
    };
    // Stopped: nothing runs.
    render(&mut engine, 4800, 512);
    assert_eq!(arp_state(&engine), [None, None]);
    assert!(!echo_state(&engine));
    engine.play();
    render(&mut engine, 24_000, 512);
    // Half a second in: the note is held, Arp 1 runs, and its echoes are to come.
    let first = arp_state(&engine)[0].expect("Arp 1 runs");
    assert!(echo_state(&engine));
    render(&mut engine, 12_000, 512);
    let later = arp_state(&engine)[0].unwrap();
    assert!(later > first, "{first} {later}");
    engine.stop();
    render(&mut engine, 512, 512);
    assert_eq!(arp_state(&engine)[0], None);
    assert!(!echo_state(&engine));
}

#[test]
fn a_param_set_several_times_at_one_frame_acts_once_at_its_last_value() {
    // Pitcher 2's Low Latency (p25) switch rebuilds its analysis, which resets it. Saved Off and
    // automated On from tick 0, each wrap restores Off and then On at one frame, and the lane holds On, Off, On at tick
    // 256. Only each frame's last value is applied, so it never switches and the song sounds as one saved On (whose
    // lane, On at 0 and 256, splits the blocks at the same frames).
    let song_with = |saved: f32, at_256: &[f32]| {
        let mut song = song(&[note(0, 4.0)]);
        song.play_loop = Some((0, 384));
        let id = song.channels[1].rack.add(45);
        song.channels[1].rack.get_mut(id).unwrap().params[25] = saved;
        let lane = song.channels[1].add_automation_lane(AutomationTarget {
            instance_id: id,
            param: 25,
        });
        let points = [(0, 1.0)]
            .into_iter()
            .chain(at_256.iter().map(|&value| (256, value)));
        lane.add_clip(0, 16.0)
            .events
            .extend(points.map(|(pos, value)| Event::Automation {
                pos,
                value,
                control: true,
            }));
        song
    };
    let rate = 48_000.0;
    let frames = 2 * frame(384, rate) + 12_000;
    let mut automated = SongAudioEngine::new(&song_with(0.0, &[1.0, 0.0, 1.0]), rate).unwrap();
    let mut reference = SongAudioEngine::new(&song_with(1.0, &[1.0]), rate).unwrap();
    automated.play();
    reference.play();
    let (automated, reference) = (
        render(&mut automated, frames, 1024),
        render(&mut reference, frames, 1024),
    );
    assert!(energy(&reference) > 1.0);
    let first = automated.iter().zip(&reference).position(|(a, b)| a != b);
    assert_eq!(first, None);
}

#[test]
fn stop_puts_every_param_back_to_its_value_before_automation() {
    // A stop returns every channel's params to their initial
    // (pre-automation) values. The instrument's Mix, saved 1, is automated to 0 from tick 0, so the song plays silent;
    // after Stop a key on the keyboard sounds at the saved Mix again.
    let mut song = song(&[]);
    song.channels[1].rack.get_mut(2).unwrap().common.0[1] = 1.0;
    let lane = song.channels[1].add_automation_lane(AutomationTarget {
        instance_id: 2,
        param: AutomationTarget::MIX,
    });
    lane.add_clip(0, 16.0).events.push(Event::Automation {
        pos: 0,
        value: 0.0,
        control: true,
    });
    let mut engine = SongAudioEngine::new(&song, 48_000.0).unwrap();
    engine.play();
    render(&mut engine, 4096, 1024);
    engine.note_on(1, 69, 1.0);
    assert_eq!(energy(&render(&mut engine, 4096, 1024)), 0.0);
    engine.stop();
    let tick = engine.transport_tick();
    engine.note_on(1, 69, 1.0);
    assert!(energy(&render(&mut engine, 4096, 1024)) > 0.01);
    // Stopped where it was.
    assert_eq!(engine.transport_tick(), tick);
}

/// The pickers' sample preview: once from its frame at gain 0.7, centred, rate 1, past the
/// master; `None` stops it, and an engine that replaces this one plays it on.
#[test]
fn the_sample_preview_plays_once_at_0_7_past_the_master_and_survives_a_rebuild() {
    use honeydew_audio::sample::SampleData;
    use std::sync::Arc;
    let samples: Vec<f32> = (0..100).map(|i| (i as f32 / 100.0) - 0.5).collect();
    let data = Arc::new(SampleData {
        sample_rate: 48_000,
        left: samples.clone().into(),
        right: None,
        meta: Default::default(),
    });
    let song = Song::default();
    let mut engine = SongAudioEngine::new(&song, 48_000.0).unwrap();
    engine.preview(Some((data.clone(), 0)));
    let out = render(&mut engine, 200, 64);
    for (i, s) in samples.iter().enumerate() {
        assert!((out[i] - 0.7 * s).abs() < 1e-6, "frame {i}: {}", out[i]);
    }
    assert!(out[100..].iter().all(|s| *s == 0.0), "it plays once");

    // From a frame; a rebuilt engine takes it over; `None` stops it.
    engine.preview(Some((data, 50)));
    let first = render(&mut engine, 10, 10);
    assert!((first[0] - 0.7 * samples[50]).abs() < 1e-6);
    let mut rebuilt = SongAudioEngine::new(&song, 48_000.0).unwrap();
    rebuilt.take_over(&mut engine);
    let next = render(&mut rebuilt, 10, 10);
    assert!((next[0] - 0.7 * samples[60]).abs() < 1e-6, "{}", next[0]);
    rebuilt.preview(None);
    assert!(render(&mut rebuilt, 10, 10).iter().all(|s| *s == 0.0));
}

/// Without a play loop the
/// transport wraps to the song start at the song end, the latest clip end rounded up to whole bars (a clip ending at
/// beat 4.5 makes two 4/4 bars), not at the clip end or a beat past it. The wrap is the loop's: the note on the song
/// start sounds again, as far after the end as it did after the start.
#[test]
fn without_a_loop_the_transport_wraps_at_the_song_end_rounded_up_to_whole_bars() {
    let rate = 48_000.0;
    let mut song = song(&[note(0, 0.25)]);
    song.channels[1].lanes[0].clips[0].loop_len_beats = 4.5;
    let mut engine = SongAudioEngine::new(&song, rate).unwrap();
    engine.play();
    render(&mut engine, frame(6 * 128, rate), 1000);
    assert_eq!(engine.transport_tick(), 6 * 128, "past 4.5 and 5.5");
    let mut engine = SongAudioEngine::new(&song, rate).unwrap();
    engine.play();
    let end = frame(8 * 128, rate);
    let out = render(&mut engine, end + 4000, 777);
    let onset = first_sound(&out, 0);
    assert_eq!(energy(&out[end - 4000..end]), 0.0);
    assert_eq!(first_sound(&out, end - 4000) - end, onset);
    assert!(engine.transport_tick() < 128, "{}", engine.transport_tick());
}

/// Takes: with no song loop a recording does not wrap at the song end. The
/// engine hears REC's arm as the metronome's (`App::auto_record`); disarmed past the end, it starts at the start.
#[test]
fn a_recording_without_a_loop_plays_on_past_the_song_end() {
    let rate = 48_000.0;
    let mut engine = SongAudioEngine::new(&song(&[]), rate).unwrap();
    engine.set_metronome(true, true, 0.0);
    engine.seek(15 * 128);
    engine.play();
    render(&mut engine, frame(2 * 128, rate), 1000);
    assert_eq!(engine.transport_tick(), 17 * 128);
    engine.set_metronome(false, true, 0.0);
    render(&mut engine, 64, 64);
    assert_eq!(engine.transport_tick(), 0);
}

/// An edit's engine re-estimates the length, and a transport at or past a
/// changed end wraps modulo the length; one the edit leaves the length of stays where it is, even at the end.
#[test]
fn an_edit_that_shortens_the_song_wraps_the_transport_modulo_its_new_length() {
    let rate = 48_000.0;
    let long = song(&[]);
    let mut engine = SongAudioEngine::new(&long, rate).unwrap();
    engine.play();
    render(&mut engine, frame(10 * 128, rate), 1000);
    let mut short = long.clone();
    short.channels[1].lanes[0].clips[0].loop_len_beats = 3.0;
    let mut edited = SongAudioEngine::new(&short, rate).unwrap().adopting();
    edited.take_over(&mut engine);
    assert_eq!(edited.transport_tick(), 2 * 128);
    let mut stopped = SongAudioEngine::new(&short, rate).unwrap();
    stopped.seek(4 * 128);
    let mut same = (SongAudioEngine::new(&short, rate).unwrap()).adopting_edit(false, Vec::new());
    same.take_over(&mut stopped);
    assert_eq!(same.transport_tick(), 4 * 128);
}

#[test]
fn the_masters_tempo_automation_plays_the_song_at_its_tempo() {
    // 120 BPM, and from beat 2 the master's Tempo lane says 240: the first second plays 2 beats, the next half second 2
    // more. Stop puts the song's tempo back, which plays again until the point.
    use honeydew::song::{TEMPO_TARGET, tempo_value};
    let rate = 48_000.0;
    let mut song = song(&[]);
    let lane = song.channels[0].add_automation_lane(TEMPO_TARGET);
    lane.add_clip(0, 16.0).events.push(Event::Automation {
        pos: 256,
        value: tempo_value(240.0),
        control: true,
    });
    let mut engine = SongAudioEngine::new(&song, rate).unwrap();
    engine.play();
    render(&mut engine, 72_000, 256);
    assert!(
        (510..=513).contains(&engine.transport_tick()),
        "{}",
        engine.transport_tick()
    );
    engine.stop();
    engine.seek(0);
    engine.play();
    render(&mut engine, 24_000, 256);
    assert_eq!(engine.transport_tick(), 128);
}
