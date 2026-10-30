//! Real-time safety: once built and warmed up, the engine neither allocates nor frees on the
//! audio thread. A counting global allocator counts the test thread's calls inside `events_during`. Every lane
//! that adds a render path adds its scenario here.

use std::alloc::{GlobalAlloc, Layout, System};
use std::cell::Cell;
use std::sync::Arc;

use honeydew::audio_engine::{LiveInput, SongAudioEngine};
use honeydew::flm::chunk::{ContentPath, ContentRoot};
use honeydew::flm::sf2d::Sf2d;
use honeydew::sample_cache::SampleCache;
use honeydew::song::{AutomationTarget, ChannelKind, Event, Song, SynthKind};
use honeydew_audio::device::AudioRenderer;
use honeydew_audio::input::InputRing;

struct Counting;

thread_local! {
    /// While counting, this thread's allocator calls so far; per thread, so parallel tests do not add to it.
    static EVENTS: Cell<Option<usize>> = const { Cell::new(None) };
}

fn count() {
    let _ = EVENTS.try_with(|events| events.set(events.get().map(|n| n + 1)));
}

// SAFETY: every call forwards to `System` unchanged; counting touches only a const thread-local.
unsafe impl GlobalAlloc for Counting {
    unsafe fn alloc(&self, layout: Layout) -> *mut u8 {
        count();
        unsafe { System.alloc(layout) }
    }

    unsafe fn dealloc(&self, ptr: *mut u8, layout: Layout) {
        count();
        unsafe { System.dealloc(ptr, layout) }
    }

    unsafe fn alloc_zeroed(&self, layout: Layout) -> *mut u8 {
        count();
        unsafe { System.alloc_zeroed(layout) }
    }

    unsafe fn realloc(&self, ptr: *mut u8, layout: Layout, new_size: usize) -> *mut u8 {
        count();
        unsafe { System.realloc(ptr, layout, new_size) }
    }
}

#[global_allocator]
static ALLOCATOR: Counting = Counting;

/// The allocator events `f` causes on this thread.
fn events_during(f: impl FnOnce()) -> usize {
    EVENTS.with(|events| events.set(Some(0)));
    f();
    EVENTS.with(|events| events.take()).unwrap()
}

const RATE: f32 = 48_000.0;
/// Odd block sizes, cycled, including ones above the engine's 1024-frame block.
const BLOCKS: [usize; 7] = [1, 7, 64, 333, 1023, 1500, 4096];

/// A Distortion above a MiniSynth, three effects (Equalizer, Compressor, Filter) below it, notes and automation on an
/// effect param, a channel param and the generator's own param and common param.
fn song() -> Song {
    let mut song = Song {
        tempo_bpm: 120.0,
        ..Song::default()
    };
    song.add_channel(ChannelKind::Synth(SynthKind::MiniSynth));
    let channel = &mut song.channels[1];
    channel.rack.add(3);
    let compressor = channel.rack.add(8);
    channel.rack.add(9);
    // The rack runs interleaved: an effect, the instrument, the effects below.
    channel.rack.insert(0, 6);
    let clip = channel.lanes[0].add_clip(0, 8.0);
    for (i, key) in [60, 64, 67, 72, 69, 65].into_iter().enumerate() {
        clip.events.push(Event::Note {
            pos: i as u32 * 128,
            len_beats: 1.5,
            key,
            velocity: 200,
            pan: 127,
            slide: false,
        });
    }
    for target in [
        AutomationTarget {
            instance_id: compressor,
            param: 1,
        },
        AutomationTarget {
            instance_id: 0,
            param: 1,
        },
        AutomationTarget {
            instance_id: 2,
            param: 10,
        },
        AutomationTarget {
            instance_id: 2,
            param: AutomationTarget::POST_GAIN,
        },
    ] {
        let lane = channel.add_automation_lane(target);
        lane.add_clip(0, 8.0)
            .events
            .extend((0..8).map(|i| Event::Automation {
                pos: i * 128,
                value: if i % 2 == 0 { 0.2 } else { 0.8 },
                control: true,
            }));
    }
    song
}

#[test]
fn the_counter_sees_this_thread_allocate() {
    assert_eq!(events_during(|| drop(std::hint::black_box(Box::new(1)))), 2);
}

#[test]
fn engine_render_and_commands_do_not_allocate() {
    // The Compressor keyed from a second MiniSynth, holding a key: a sidechain.
    let mut song = song();
    song.add_channel(ChannelKind::Synth(SynthKind::MiniSynth));
    let key = song.channels[2].id;
    let modules = &mut song.channels[1].rack.modules;
    modules
        .iter_mut()
        .find(|m| m.type_id == 8)
        .unwrap()
        .sidechain = Some(key);
    // Channel 1's notes and CCs go on to channel 2 too: a MIDI send.
    (song.channels[1].file.midi_sends).push(honeydew::song::MidiSend {
        target_channel_id: key,
        enabled: true,
    });
    // Modulators: an Envelope keyed by channel 2 on the automated Compressor Threshold, an LFO on the automated Pan and
    // a Random on MIDI CC #1, which commands set too.
    let ch = &mut song.channels[1];
    let compressor = ch
        .rack
        .modules
        .iter()
        .find(|m| m.type_id == 8)
        .unwrap()
        .instance_id;
    for (source, target) in [(0.0, (compressor, 1)), (0.5, (0, 1)), (1.0, (0, 8))] {
        let id = ch.rack.add(honeydew_audio::modulator::MODULATOR);
        let module = ch.rack.get_mut(id).unwrap();
        (module.params[1], module.sidechain) = (source, Some(key));
        ch.file.modulations.push(honeydew::song::Modulation {
            modulator: id,
            channel: ch.id,
            target: AutomationTarget {
                instance_id: target.0,
                param: target.1,
            },
        });
    }
    let mut engine = SongAudioEngine::new(&song, RATE).unwrap();
    engine.note_on(2, 45, 1.0);
    let mut output = vec![0.0_f32; 4096 * 2];
    // Warm-up: one pass over every block size and command.
    engine.play();
    for frames in BLOCKS {
        engine.render(&mut output[..frames * 2], 2);
    }
    engine.seek(0);

    let mut energy = 0.0_f32;
    let events = events_during(|| {
        let mut rendered = 0;
        let mut i = 0;
        while rendered < 10 * RATE as usize {
            let frames = BLOCKS[i % BLOCKS.len()];
            engine.render(&mut output[..frames * 2], 2);
            energy += output[..frames * 2].iter().map(|s| s * s).sum::<f32>();
            rendered += frames;
            match i % 50 {
                5 => engine.set_param(1, 2, 13, (i % 7) as f32 / 7.0),
                10 => engine.set_param(1, 2, AutomationTarget::MIX, 0.7),
                15 => engine.set_param(1, 3, 2, 0.4),
                20 => engine.note_on(1, 50 + (i % 20) as u8, 0.8),
                25 => engine.note_off(1, 50 + ((i - 5) % 20) as u8),
                30 => engine.set_param(1, 0, 8, 0.6),
                // The sustain pedal down and up, and CC #7 (Level).
                32 => engine.set_param(1, 0, 71, ((i / 50) % 2) as f32),
                33 => engine.set_param(1, 0, 14, 0.8),
                35 => engine.seek(((i * 37) % 1024) as u64),
                40 => engine.stop(),
                45 => engine.play(),
                _ => {}
            }
            i += 1;
        }
    });
    assert_eq!(events, 0, "allocator calls on the audio thread");
    assert!(energy > 0.0, "the counted render was silent");
}

#[test]
fn loop_wraps_and_transport_commands_do_not_allocate() {
    // [100, 140) holds the note-on at 128 and the automation between its points; 40 ticks are 7500 frames.
    let mut song = song();
    song.play_loop = Some((100, 140));
    let mut engine = SongAudioEngine::new(&song, RATE).unwrap();
    let mut output = vec![0.0_f32; 4096 * 2];
    // Warm-up: one pass over every block size (7024 frames, about 37 ticks: short of the loop, so no wrap yet).
    engine.play();
    for frames in BLOCKS {
        engine.render(&mut output[..frames * 2], 2);
    }
    engine.seek(0);

    let mut wraps = 0;
    let mut energy = 0.0_f32;
    let events = events_during(|| {
        let mut i = 0;
        let mut tick = engine.transport_tick();
        while wraps < 100 {
            let frames = BLOCKS[i % BLOCKS.len()];
            engine.render(&mut output[..frames * 2], 2);
            energy += output[..frames * 2].iter().map(|s| s * s).sum::<f32>();
            let now = engine.transport_tick();
            wraps += usize::from(now < tick);
            tick = now;
            match i % 40 {
                10 => engine.all_notes_off(),
                20 => engine.set_setting(1, 2, 0x4E4F_5445, 0.5),
                30 => engine.set_loop(Some((100, 140 + (i % 3) as u64))),
                _ => {}
            }
            i += 1;
        }
    });
    assert_eq!(events, 0, "allocator calls on the audio thread");
    assert!(energy > 0.0, "the counted render was silent");
}

/// A Transistor Bass channel with distortion on: its render, legato notes, params and seek allocate nothing.
#[test]
fn transistor_bass_render_does_not_allocate() {
    let mut song = Song {
        tempo_bpm: 120.0,
        ..Song::default()
    };
    song.add_channel(ChannelKind::Synth(SynthKind::TransistorBass));
    let channel = &mut song.channels[1];
    let clip = channel.lanes[0].add_clip(0, 8.0);
    for (i, key) in [36, 48, 43, 36, 39, 41].into_iter().enumerate() {
        clip.events.push(Event::Note {
            pos: i as u32 * 128,
            len_beats: 1.5,
            key,
            velocity: if i % 2 == 0 { 120 } else { 80 },
            pan: 200,
            slide: false,
        });
    }
    let mut engine = SongAudioEngine::new(&song, RATE).unwrap();
    engine.set_param(1, 2, 7, 1.0);
    let mut output = vec![0.0_f32; 4096 * 2];
    engine.play();
    for frames in BLOCKS {
        engine.render(&mut output[..frames * 2], 2);
    }
    engine.seek(0);

    let mut energy = 0.0_f32;
    let events = events_during(|| {
        let (mut rendered, mut i) = (0, 0);
        while rendered < 5 * RATE as usize {
            let frames = BLOCKS[i % BLOCKS.len()];
            engine.render(&mut output[..frames * 2], 2);
            energy += output[..frames * 2].iter().map(|s| s * s).sum::<f32>();
            rendered += frames;
            match i % 40 {
                5 => engine.set_param(1, 2, 2, (i % 7) as f32 / 7.0),
                10 => engine.note_on(1, 30 + (i % 20) as u8, 0.9),
                15 => engine.note_off(1, 30 + ((i - 5) % 20) as u8),
                20 => engine.set_param(1, 2, 9, 0.6),
                25 => engine.seek(((i * 37) % 1024) as u64),
                30 => engine.stop(),
                35 => engine.play(),
                _ => {}
            }
            i += 1;
        }
    });
    assert_eq!(events, 0, "allocator calls on the audio thread");
    assert!(energy > 0.0, "the counted render was silent");
}

/// 3x Osc: HQ on, notes, Mono and filter-type changes, the polyphony limit, seek, stop and play. Echo
/// (10 echoes, Fat Mode on and off) and the Arp (Off, Up on a fixed chord, Bounce on a scale, Random on Auto (sustain);
/// Slide and Range changes) cycle every 60 blocks.
#[test]
fn three_osc_render_and_commands_do_not_allocate() {
    let mut song = Song {
        tempo_bpm: 120.0,
        ..Song::default()
    };
    song.add_channel(ChannelKind::Synth(SynthKind::ThreeXOsc));
    let channel = &mut song.channels[1];
    let id = channel
        .rack
        .modules
        .iter()
        .find(|m| m.type_id == 39)
        .unwrap()
        .instance_id;
    let clip = channel.lanes[0].add_clip(0, 8.0);
    for (i, key) in [60, 64, 67, 72, 69, 65].into_iter().enumerate() {
        clip.events.push(Event::Note {
            pos: i as u32 * 128,
            len_beats: 1.5,
            key,
            velocity: 200,
            pan: 127,
            slide: false,
        });
    }
    let mut engine = SongAudioEngine::new(&song, RATE).unwrap();
    let mut output = vec![0.0_f32; 4096 * 2];
    engine.set_param(1, id, 25, 1.0); // HQ: the shared mip tables were built with the engine
    engine.set_param(1, id, 1, 0.5); // Osc 1 Saw
    engine.set_param(1, id, 130, 1.0); // 10 echoes
    engine.set_param(1, id, 131, 0.4); // Echo Feedback 80 %: Echo on
    engine.set_param(1, id, 132, 0.1); // echoes 0.2 beat apart
    engine.set_param(1, id, 140, 0.2); // a fast arp step
    engine.play();
    for frames in BLOCKS {
        engine.render(&mut output[..frames * 2], 2);
    }
    engine.seek(0);

    let mut energy = 0.0_f32;
    let events = events_during(|| {
        let mut rendered = 0;
        let mut i = 0;
        while rendered < 10 * RATE as usize {
            let frames = BLOCKS[i % BLOCKS.len()];
            engine.render(&mut output[..frames * 2], 2);
            energy += output[..frames * 2].iter().map(|s| s * s).sum::<f32>();
            rendered += frames;
            match i % 60 {
                5 => engine.set_param(1, id, 110, (i % 8) as f32 / 7.0),
                10 => engine.set_param(1, id, 117, (i % 3) as f32 / 99.0),
                // Mono on every third cycle only, so Echo (off in Mono) runs in the others.
                15 => engine.set_param(1, id, 120, f32::from((i / 60) % 3 == 2)),
                20 => engine.note_on(1, 40 + (i % 40) as u8, 0.8),
                25 => engine.note_off(1, 40 + ((i - 5) % 40) as u8),
                30 => engine.set_param(1, id, 129, (i % 5) as f32 / 4.0),
                35 => engine.seek(((i * 37) % 1024) as u64),
                40 => engine.stop(),
                45 => engine.play(),
                50 => engine.set_param(1, 0, 4, 0.75),
                // Arp Off (Echo plain, then Fat Mode), Up on Major, Bounce on Major scale, Random on Auto (sustain).
                0 => engine.set_param(1, id, 139, [0.0, 0.0, 0.2, 0.6, 1.0][(i / 60) % 5]),
                2 => engine.set_param(1, id, 138, ((i / 60) % 2) as f32),
                4 => engine.set_param(1, id, 145, [3.0, 3.0, 3.0, 69.0, 1.0][(i / 60) % 5] / 90.0),
                7 => engine.set_param(1, id, 144, ((i / 120) % 2) as f32),
                9 => engine.set_param(1, id, 142, ((i / 60) % 7) as f32 / 6.0),
                _ => {}
            }
            i += 1;
        }
    });
    assert_eq!(events, 0, "allocator calls on the audio thread");
    assert!(energy > 0.0, "the counted render was silent");
}

/// SuperSaw: 12-voice unison with pulse, sub and unison pan, a Tmp Sync LFO, the arp, distortion and the delay;
/// clip notes and live notes, param changes (including the SUPERSYNC page), the mod wheel, seek and Play/Stop.
#[test]
fn supersaw_render_and_commands_do_not_allocate() {
    let mut song = Song {
        tempo_bpm: 120.0,
        ..Song::default()
    };
    song.add_channel(ChannelKind::Synth(SynthKind::SuperSaw));
    let channel = &mut song.channels[1];
    let id = channel.rack.modules[0].instance_id;
    let params = &mut channel.rack.get_mut(id).unwrap().params;
    for (index, value) in [
        (4, 1.0),
        (2, 1.0),
        (3, 0.5),
        (6, 1.0),
        (20, 0.8),
        (41, 1.0),
        (44, 0.9),
        (49, 0.6),
        (52, 0.5),
        (56, 0.5),
    ] {
        params[index] = value;
    }
    let clip = channel.lanes[0].add_clip(0, 8.0);
    for (i, key) in [48, 55, 60, 64].into_iter().enumerate() {
        clip.events.push(Event::Note {
            pos: i as u32 * 96,
            len_beats: 2.0,
            key,
            velocity: 200,
            pan: 127,
            slide: false,
        });
    }
    let mut engine = SongAudioEngine::new(&song, RATE).unwrap();
    let mut output = vec![0.0_f32; 4096 * 2];
    engine.play();
    for frames in BLOCKS {
        engine.render(&mut output[..frames * 2], 2);
    }
    engine.seek(0);

    let mut energy = 0.0_f32;
    let events = events_during(|| {
        let mut rendered = 0;
        let mut i = 0;
        while rendered < 10 * RATE as usize {
            let frames = BLOCKS[i % BLOCKS.len()];
            engine.render(&mut output[..frames * 2], 2);
            energy += output[..frames * 2].iter().map(|s| s * s).sum::<f32>();
            rendered += frames;
            match i % 60 {
                5 => engine.set_param(1, id, 10, (i % 2) as f32),
                10 => engine.set_param(1, id, 49, (i % 6) as f32 / 5.0),
                15 => engine.set_param(1, 0, 8, (i % 3) as f32 / 2.0),
                20 => engine.note_on(1, 50 + (i % 20) as u8, 0.8),
                25 => engine.note_off(1, 50 + ((i - 5) % 20) as u8),
                30 => engine.set_param(1, id, 50, (i % 2) as f32),
                35 => engine.seek(((i * 37) % 1024) as u64),
                40 => engine.stop(),
                45 => engine.play(),
                50 => engine.set_param(1, 0, 4, 0.8),
                _ => {}
            }
            i += 1;
        }
    });
    assert_eq!(events, 0, "allocator calls on the audio thread");
    assert!(energy > 0.0, "the counted render was silent");
}

#[test]
fn metronome_count_in_monitoring_recording_and_the_latency_ping_do_not_allocate() {
    // An Audio Record channel monitoring a stereo input through a Leveller, next to the synth song; the
    // input recorded through count-ins, stops, seeks and loop wraps, and the latency calibration's ping.
    let mut song = song();
    song.add_channel(ChannelKind::AudioRecord);
    let record = song.channels.last_mut().unwrap();
    record.file.input_monitoring = true;
    record.rack.add(20);
    let ring = Arc::new(InputRing::new(2, RATE as u32, RATE as usize));
    let live = LiveInput::new(ring.clone(), vec![(record.id, 1, Some(0))]);
    let mut engine =
        SongAudioEngine::with_input(&song, RATE, &SampleCache::default(), Some(&live)).unwrap();
    let mut output = vec![0.0_f32; 4096 * 2];
    let input = vec![0.25_f32; 3 * 4096 * 2];
    // Warm-up: one pass over every block size, counting in, recording.
    live.tap.arm(300);
    engine.set_loop(Some((0, 128)));
    engine.set_metronome(true, true, 0.7);
    engine.count_in(4);
    engine.play();
    for frames in BLOCKS {
        ring.push(input[..frames * 2].iter().copied());
        engine.render(&mut output[..frames * 2], 2);
    }
    engine.seek(0);

    let mut energy = 0.0_f32;
    let events = events_during(|| {
        let mut rendered = 0;
        let mut i = 0;
        while rendered < 10 * RATE as usize {
            let frames = BLOCKS[i % BLOCKS.len()];
            // Now and then the input runs three blocks ahead, so the monitor skips.
            let pushed = if i % 9 == 0 { 3 * frames } else { frames };
            ring.push(input[..pushed * 2].iter().copied());
            engine.render(&mut output[..frames * 2], 2);
            energy += output[..frames * 2].iter().map(|s| s * s).sum::<f32>();
            rendered += frames;
            // A cycle: stop, a beat of count-in from Play (55: disarmed and armed again during it), then the take
            // rolls through a seek and loop wraps.
            match i % 60 {
                10 if i < 60 => engine.ping(1),
                20 => engine.set_metronome(i % 120 == 20, true, 0.3),
                30 => engine.seek(96),
                40 => engine.stop(),
                45 => engine.count_in(1),
                50 => engine.play(),
                55 => live.tap.disarm(),
                58 => {
                    live.tap.arm(0);
                }
                _ => {}
            }
            i += 1;
        }
    });
    assert_eq!(events, 0, "allocator calls on the audio thread");
    assert!(energy > 0.0, "the counted render was silent");
    // It recorded several takes, and the ping heard the 0.25 input as noise.
    assert!(live.tap.started() > 3 && live.tap.frames().written() > RATE as u64);
    use honeydew_audio::device::PingFailure;
    assert_eq!(
        engine.ping_report(),
        Some((1, Some(Err(PingFailure::Noise))))
    );
}

/// A SoundFont Player playing the looped sine bank of `crates/audio/tests/soundfont.rs`, with notes, through every
/// command.
#[test]
fn soundfont_player_render_does_not_allocate() {
    let path = ContentPath::Shorthand {
        root: ContentRoot::User,
        relative: "My SoundFonts/sine.sf2".into(),
    };
    let mut song = Song {
        tempo_bpm: 120.0,
        ..Song::default()
    };
    song.add_channel(ChannelKind::Synth(SynthKind::SoundFontPlayer));
    let channel = &mut song.channels[1];
    let sf2d = Sf2d {
        path: path.clone(),
        patch: 0,
    };
    channel.rack.modules[0].file.custom_data = Some(sf2d.write(None).unwrap());
    let clip = channel.lanes[0].add_clip(0, 8.0);
    for (i, key) in [60, 64, 67, 72, 69, 65].into_iter().enumerate() {
        clip.events.push(Event::Note {
            pos: i as u32 * 128,
            len_beats: 1.5,
            key,
            velocity: 200,
            pan: 127,
            slide: false,
        });
    }
    let bytes = include_bytes!("../crates/audio/tests/fixtures/sine.sf2");
    let mut cache = SampleCache::default();
    cache.load(&song, &|p| (*p == path).then(|| bytes.to_vec()));
    let mut engine = SongAudioEngine::with_samples(&song, RATE, &cache).unwrap();
    let mut output = vec![0.0_f32; 4096 * 2];
    engine.play();
    for frames in BLOCKS {
        engine.render(&mut output[..frames * 2], 2);
    }
    engine.seek(0);

    let mut energy = 0.0_f32;
    let events = events_during(|| {
        let mut rendered = 0;
        let mut i = 0;
        while rendered < 10 * RATE as usize {
            let frames = BLOCKS[i % BLOCKS.len()];
            engine.render(&mut output[..frames * 2], 2);
            energy += output[..frames * 2].iter().map(|s| s * s).sum::<f32>();
            rendered += frames;
            match i % 50 {
                5 => engine.set_param(1, 2, 5, (i % 2) as f32),
                10 => engine.set_param(1, 2, 1 + (i % 4) as i32, 0.3),
                15 => engine.set_param(1, 0, 4, (i % 7) as f32 / 7.0),
                20 => (0..40).for_each(|k| engine.note_on(1, 40 + k, 0.8)),
                25 => engine.note_off(1, 50 + ((i - 5) % 20) as u8),
                35 => engine.seek(((i * 37) % 1024) as u64),
                40 => engine.stop(),
                45 => engine.play(),
                _ => {}
            }
            i += 1;
        }
    });
    assert_eq!(events, 0, "allocator calls on the audio thread");
    assert!(energy > 0.0, "the counted render was silent");
}

/// A 16-bit mono PCM WAV at 44.1 kHz.
fn wav(samples: &[i16]) -> Vec<u8> {
    let data: Vec<u8> = samples.iter().flat_map(|s| s.to_le_bytes()).collect();
    let mut bytes = b"RIFF".to_vec();
    bytes.extend((36 + data.len() as u32).to_le_bytes());
    bytes.extend(b"WAVEfmt \x10\0\0\0\x01\0\x01\0");
    bytes.extend(44_100u32.to_le_bytes());
    bytes.extend(88_200u32.to_le_bytes());
    bytes.extend(b"\x02\0\x10\0data");
    bytes.extend((data.len() as u32).to_le_bytes());
    bytes.extend(data);
    bytes
}

/// A Slicer playing its loop. Sequenced and live notes over its slices, MONO, every Sustain mode, Pitch, Enable,
/// seek and Play/Stop.
#[test]
fn slicer_render_and_commands_do_not_allocate() {
    let mut song = Song {
        tempo_bpm: 120.0,
        ..Song::default()
    };
    song.add_channel(ChannelKind::Synth(SynthKind::Slicer));
    let channel = &mut song.channels[1];
    let path = b"@L@\0loop.wav";
    let mut cstm = b"SENS\x05\0\0\0".to_vec();
    cstm.extend(0.25f32.to_le_bytes());
    cstm.push(1);
    cstm.extend(b"PATH");
    cstm.extend((4 + path.len() as u32).to_le_bytes());
    cstm.extend((path.len() as u32).to_le_bytes());
    cstm.extend(path);
    channel.rack.modules[0].file.custom_data = Some(cstm);
    let id = channel.rack.modules[0].instance_id;
    let clip = channel.lanes[0].add_clip(0, 8.0);
    for i in 0..16 {
        clip.events.push(Event::Note {
            pos: i * 64,
            len_beats: 0.4,
            key: 48 + (i % 8) as u8,
            velocity: 200,
            pan: 127,
            slide: false,
        });
    }
    // A burst every 100 ms: ten slices.
    let samples: Vec<i16> = (0..44_100)
        .map(|i| {
            let t = i % 4410;
            if t < 400 {
                ((t as f32 * 0.37).sin() * 20_000.0) as i16
            } else {
                0
            }
        })
        .collect();
    let bytes = wav(&samples);
    let mut cache = honeydew::sample_cache::SampleCache::default();
    cache.load(&song, &|_| Some(bytes.clone()));
    let mut engine = SongAudioEngine::with_samples(&song, RATE, &cache).unwrap();
    let mut output = vec![0.0_f32; 4096 * 2];
    engine.play();
    for frames in BLOCKS {
        engine.render(&mut output[..frames * 2], 2);
    }
    engine.seek(0);

    let mut energy = 0.0_f32;
    let events = events_during(|| {
        let mut rendered = 0;
        let mut i = 0;
        while rendered < 10 * RATE as usize {
            let frames = BLOCKS[i % BLOCKS.len()];
            engine.render(&mut output[..frames * 2], 2);
            energy += output[..frames * 2].iter().map(|s| s * s).sum::<f32>();
            rendered += frames;
            match i % 60 {
                5 => engine.set_param(1, id, 6, (i / 60 % 2) as f32),
                10 => engine.set_param(1, id, 5, (i / 60 % 3) as f32 / 2.0),
                15 => engine.set_param(1, id, 1, (i % 7) as f32 / 7.0),
                20 => engine.note_on(1, 46 + (i % 14) as u8, 0.8),
                25 => engine.note_off(1, 46 + ((i - 5) % 14) as u8),
                30 => engine.set_param(1, id, 0, 0.0),
                33 => engine.set_param(1, id, 0, 1.0),
                35 => engine.seek(((i * 37) % 1024) as u64),
                40 => engine.stop(),
                45 => engine.play(),
                _ => {}
            }
            i += 1;
        }
    });
    assert_eq!(events, 0, "allocator calls on the audio thread");
    assert!(energy > 0.0, "the counted render was silent");
}

#[test]
fn module_settings_do_not_allocate() {
    // Pitcher note enables (#5–#16) and the Multiband's soloed band (key 38) between renders.
    let mut song = song();
    let rack = &mut song.channels[1].rack;
    let (pitcher, multiband) = (rack.add(25), rack.add(36));
    let mut engine = SongAudioEngine::new(&song, RATE).unwrap();
    let mut output = vec![0.0_f32; 4096 * 2];
    engine.play();
    for frames in BLOCKS {
        engine.render(&mut output[..frames * 2], 2);
    }
    engine.seek(0);

    let mut energy = 0.0_f32;
    let events = events_during(|| {
        for i in 0..200 {
            let frames = BLOCKS[i % BLOCKS.len()];
            engine.render(&mut output[..frames * 2], 2);
            energy += output[..frames * 2].iter().map(|s| s * s).sum::<f32>();
            engine.set_setting(1, pitcher, 5 + (i % 12) as u32, (i % 2) as f32);
            engine.set_setting(1, multiband, 38, (i % 4) as f32);
        }
    });
    assert_eq!(events, 0, "allocator calls on the audio thread");
    assert!(energy > 0.0, "the counted render was silent");
}

/// GMSynth: its `CSTM` shapes, a full-width patch (Quartet choir, FM, noise, LFO, glide), notes and automation
/// on its cutoff.
fn gmsynth_song() -> (Song, i32) {
    let mut song = Song {
        tempo_bpm: 120.0,
        ..Song::default()
    };
    song.add_channel(ChannelKind::Synth(SynthKind::GmSynth));
    let channel = &mut song.channels[1];
    let synth = channel.rack.modules[0].instance_id;
    let module = channel.rack.get_mut(synth).unwrap();
    // waveA 3 (Digital), waveB 16 (Saw), PB RANGE, MW AMOUNT, MW DEST.
    let words = [
        3,
        16,
        0.5_f32.to_bits(),
        0.5_f32.to_bits(),
        0.0_f32.to_bits(),
    ];
    module.file.custom_data = Some(words.iter().flat_map(|w: &u32| w.to_le_bytes()).collect());
    for (index, value) in [
        (42, 1.0),
        (44, 0.5),
        (45, 0.7),
        (16, 1.0 / 3.0),
        (15, 0.3),
        (20, 0.2),
        (22, 0.6),
        (34, 0.8),
        (29, 0.2),
    ] {
        module.params[index] = value;
    }
    let clip = channel.lanes[0].add_clip(0, 8.0);
    for (i, key) in [60, 64, 67, 72, 69, 65].into_iter().enumerate() {
        clip.events.push(Event::Note {
            pos: i as u32 * 96,
            len_beats: 2.5,
            key,
            velocity: 200,
            pan: 127,
            slide: false,
        });
    }
    let target = AutomationTarget {
        instance_id: synth,
        param: 21,
    };
    let lane = channel.add_automation_lane(target);
    lane.add_clip(0, 8.0)
        .events
        .extend((0..8).map(|i| Event::Automation {
            pos: i * 128,
            value: if i % 2 == 0 { 0.3 } else { 0.9 },
            control: true,
        }));
    (song, synth)
}

#[test]
fn gmsynth_renders_and_takes_commands_and_settings_without_allocating() {
    use honeydew_audio::gmsynth::{MW_DEST, PB_RANGE, WAVE_A, WAVE_B, prepare_setting};
    let (song, synth) = gmsynth_song();
    let mut engine = SongAudioEngine::new(&song, RATE).unwrap();
    let mut output = vec![0.0_f32; 4096 * 2];
    // The app thread builds the shapes a live setting will pick before it sends the setting.
    let shapes = [5.0, 18.0, 27.0, 40.0];
    for slot in shapes {
        prepare_setting(WAVE_A, slot);
    }
    engine.play();
    for frames in BLOCKS {
        engine.render(&mut output[..frames * 2], 2);
    }
    engine.seek(0);

    let mut energy = 0.0_f32;
    let events = events_during(|| {
        let (mut rendered, mut i) = (0, 0);
        while rendered < 10 * RATE as usize {
            let frames = BLOCKS[i % BLOCKS.len()];
            engine.render(&mut output[..frames * 2], 2);
            energy += output[..frames * 2].iter().map(|s| s * s).sum::<f32>();
            rendered += frames;
            let shape = shapes[i / 60 % shapes.len()];
            // Slot 40 is past the menu: a missing shape, silent.
            match i % 60 {
                // MONO on (a note-off to all 128 keys) and off again.
                5 => engine.set_param(1, synth, 30, 1.0),
                10 => engine.set_param(1, synth, 30, 0.0),
                15 => engine.set_param(1, synth, 16, (i % 4) as f32 / 3.0),
                20 => engine.note_on(1, 50 + (i % 20) as u8, 0.8),
                25 => engine.note_off(1, 50 + ((i - 5) % 20) as u8),
                30 => engine.set_param(1, synth, 42, (i % 7) as f32 / 6.0),
                35 => engine.set_param(1, 0, 4, (i % 5) as f32 / 4.0),
                40 => engine.seek(((i * 37) % 1024) as u64),
                45 => engine.stop(),
                50 => engine.play(),
                52 => engine.set_setting(1, synth, WAVE_A, shape),
                54 => engine.set_setting(1, synth, WAVE_B, shape),
                55 => engine.set_param(1, synth, 41, (i % 3) as f32 / 2.0),
                56 => engine.set_setting(1, synth, PB_RANGE, (i % 4) as f32),
                58 => engine.set_setting(1, synth, MW_DEST, (i % 3) as f32),
                _ => {}
            }
            i += 1;
        }
    });
    assert_eq!(events, 0, "allocator calls on the audio thread");
    assert!(energy > 0.0, "the counted render was silent");
}

/// The default kit on built-in one-shots: step and notes-lane hits with lengths, repeats and per-hit pitch, the hats'
/// cut group, a pad past its 32 voices, a stored MIDI note, slot automation, a send target and a play loop.
fn drum_song() -> Song {
    let mut song = Song {
        tempo_bpm: 140.0,
        play_loop: Some((64, 448)),
        ..Song::default()
    };
    song.add_channel(ChannelKind::Drums);
    song.add_channel(ChannelKind::EffectTrack);
    let send = song.channels[2].id;
    let drums = &mut song.channels[1];
    drums.rack.add(3);
    let kit = drums.drum_kit.as_mut().unwrap();
    kit.slots[2].file.send_target = send;
    kit.slots[5].file.midi_key = 60;
    kit.slots[6].file.overlapping_voices = false;
    let crash = -(kit.slots[7].file.id as i32);
    let hit = |pos: u32, pad: u8, len_beats: f64, repeat: u8| Event::Drum {
        pos,
        len_beats,
        pad,
        velocity: 230,
        pan: (pos % 255) as u8,
        value: (pos * 997 % 65536) as u16,
        repeat,
    };
    drums.lanes[0]
        .add_clip(0, 4.0)
        .events
        .extend((0..64).map(|i| hit(i * 8, (i % 8) as u8, 0.0, (i % 9) as u8)));
    let notes = drums.add_lane(honeydew::song::LaneKind::Notes);
    drums.lanes[notes].grid = honeydew::song::DrumGrid::Notes;
    drums.lanes[notes]
        .add_clip(0, 2.0)
        .events
        .extend((0..16).map(|i| hit(i * 16, 7, 0.1, 8)));
    drums
        .add_automation_lane(AutomationTarget {
            instance_id: crash,
            param: 0,
        })
        .add_clip(0, 4.0)
        .events
        .extend((0..8).map(|i| Event::Automation {
            pos: i * 64,
            value: if i % 2 == 0 { 0.3 } else { 0.9 },
            control: true,
        }));
    song
}

#[test]
fn drum_kit_render_and_commands_do_not_allocate() {
    let song = drum_song();
    let mut cache = SampleCache::default();
    cache.load(&song, &|_| None);
    let mut engine = SongAudioEngine::with_samples(&song, RATE, &cache).unwrap();
    let crash = -(song.channels[1].drum_kit.as_ref().unwrap().slots[7].file.id as i32);
    let mut output = vec![0.0_f32; 4096 * 2];
    engine.play();
    for frames in BLOCKS {
        engine.render(&mut output[..frames * 2], 2);
    }
    engine.seek(0);

    let mut energy = 0.0_f32;
    let events = events_during(|| {
        let mut rendered = 0;
        let mut i = 0;
        while rendered < 10 * RATE as usize {
            let frames = BLOCKS[i % BLOCKS.len()];
            engine.render(&mut output[..frames * 2], 2);
            energy += output[..frames * 2].iter().map(|s| s * s).sum::<f32>();
            rendered += frames;
            // The device reads every meter after each render, the pads' too.
            for meter in 0..engine.meter_count() {
                std::hint::black_box(engine.meter(meter));
            }
            match i % 40 {
                3 => engine.note_on(1, 48 + (i % 9) as u8, 0.9),
                6 => engine.note_on(1, 60, 0.7),
                9 => engine.note_off(1, 48 + ((i - 6) % 9) as u8),
                12 => engine.set_param(1, crash, (i % 11) as i32, (i % 5) as f32 / 4.0),
                15 => engine.set_param(1, 0, 4, (i % 7) as f32 / 6.0),
                20 => engine.seek(((i * 53) % 512) as u64),
                25 => engine.all_notes_off(),
                30 => engine.stop(),
                33 => engine.play(),
                _ => {}
            }
            i += 1;
        }
    });
    assert_eq!(events, 0, "allocator calls on the audio thread");
    assert!(energy > 0.0, "the counted render was silent");
}

/// Two audio clips on an Audio Clip channel: offsets, loop repetitions, clip ends, all three envelopes, Reverse,
/// an `STR1` stretch, and a play loop whose wraps relaunch them mid-sample; plus a clip lane on a synth channel.
fn audio_clip_song() -> Song {
    let mut song = Song {
        tempo_bpm: 130.0,
        play_loop: Some((96, 900)),
        ..Song::default()
    };
    song.add_channel(ChannelKind::AudioClip);
    song.add_channel(ChannelKind::Synth(SynthKind::MiniSynth));
    let audio = song.channels[2].add_lane(honeydew::song::LaneKind::Audio);
    let lanes = [(1, 0), (1, 0), (2, audio)];
    for (i, (channel, lane)) in lanes.into_iter().enumerate() {
        let mut slot = honeydew::song::SamplerSlot::default();
        slot.file.path = Some(honeydew::flm::chunk::ContentPath::Plain(format!("{i}")));
        slot.file.reverse = i == 1;
        (slot.file.stretch_mode, slot.file.stretch_ratio) = (1, 1.5);
        let clip = song.channels[channel].lanes[lane].add_clip(i as u32 * 200, 1.5);
        clip.visible_len_beats = 5.0;
        clip.offset_beats = 0.4 * i as f64;
        clip.file.sample = Some(Box::new(slot));
        for env_param in 0..3 {
            clip.events.extend((0..6).map(|k| Event::Envelope {
                pos: k * 32,
                env_param,
                value: if k % 2 == 0 { 0.3 } else { 0.8 },
                control: true,
            }));
        }
    }
    song
}

/// A 16-bit mono WAV of `frames` frames of a 220 Hz sine at 44.1 kHz.
fn sine_wav(frames: usize) -> Vec<u8> {
    let data: Vec<u8> = (0..frames)
        .map(|i| (8000.0 * (std::f32::consts::TAU * 220.0 * i as f32 / 44_100.0).sin()) as i16)
        .flat_map(i16::to_le_bytes)
        .collect();
    let mut bytes = b"RIFF".to_vec();
    bytes.extend((36 + data.len() as u32).to_le_bytes());
    bytes.extend(b"WAVEfmt ");
    for field in [16u32, 0x0001_0001, 44_100, 88_200, 0x0010_0002] {
        bytes.extend(field.to_le_bytes());
    }
    bytes.extend(b"data");
    bytes.extend((data.len() as u32).to_le_bytes());
    bytes.extend(data);
    bytes
}

#[test]
fn audio_clip_render_and_commands_do_not_allocate() {
    let song = audio_clip_song();
    let mut cache = honeydew::sample_cache::SampleCache::default();
    cache.load(&song, &|_| Some(sine_wav(100_000)));
    let mut engine = SongAudioEngine::with_samples(&song, RATE, &cache).unwrap();
    let mut output = vec![0.0_f32; 4096 * 2];
    engine.play();
    for frames in BLOCKS {
        engine.render(&mut output[..frames * 2], 2);
    }
    engine.seek(0);

    let mut energy = 0.0_f32;
    let events = events_during(|| {
        let mut rendered = 0;
        let mut i = 0;
        while rendered < 10 * RATE as usize {
            let frames = BLOCKS[i % BLOCKS.len()];
            engine.render(&mut output[..frames * 2], 2);
            energy += output[..frames * 2].iter().map(|s| s * s).sum::<f32>();
            rendered += frames;
            match i % 40 {
                10 => engine.seek(((i * 53) % 1000) as u64),
                20 => engine.all_notes_off(),
                25 => engine.stop(),
                27 => engine.count_in((i % 2) as u32),
                28 => engine.play(),
                33 => engine.set_loop(Some((96 + (i % 5) as u64, 900))),
                _ => {}
            }
            i += 1;
        }
    });
    assert_eq!(events, 0, "allocator calls on the audio thread");
    assert!(energy > 0.0, "the counted render was silent");
}

/// Live keys through the real-time note effects: a chord through the
/// Arpeggiator, a Note Echo, a Randomizer, Scale and Transpose, on and off the notes lane, playing and stopped, with
/// seeks and all notes off, allocates nothing.
#[test]
fn live_keys_through_the_note_effects_do_not_allocate() {
    let mut song = song();
    let rack = &mut song.channels[1].rack;
    let arp = rack.insert(0, 51);
    rack.get_mut(arp).unwrap().params[1] = 0.0;
    let echo = rack.insert(1, 47);
    rack.get_mut(echo).unwrap().params[1] = 8.0 / 32.0;
    for type_id in [49, 50, 48] {
        rack.insert(2, type_id);
    }
    let mut engine = SongAudioEngine::new(&song, RATE).unwrap();
    let mut output = vec![0.0_f32; 4096 * 2];
    let mut step = |engine: &mut SongAudioEngine, i: usize| {
        let frames = BLOCKS[i % BLOCKS.len()];
        engine.render(&mut output[..frames * 2], 2);
        let key = 48 + (i % 24) as u8;
        match i % 40 {
            0..=3 => engine.note_on(1, key, 0.8),
            12..=15 => engine.note_off(1, key - 12),
            20 => engine.set_live_lane(1, i % 80 == 20),
            25 => engine.seek(((i * 37) % 1024) as u64),
            30 => engine.all_notes_off(),
            33 => engine.stop(),
            37 => engine.play(),
            _ => {}
        }
        output[..frames * 2].iter().map(|s| s * s).sum::<f32>()
    };
    // Warm-up: every block size and command once.
    engine.play();
    for i in 0..80 {
        step(&mut engine, i);
    }
    let mut energy = 0.0_f32;
    let events = events_during(|| {
        for i in 0..2000 {
            energy += step(&mut engine, i);
        }
    });
    assert_eq!(events, 0, "allocator calls on the audio thread");
    assert!(energy > 0.0, "the counted render was silent");
}

/// Slide notes: slide events every tick to MiniSynth, GMSynth, SuperSaw and 3x Osc,
/// with seeks, stops and loop wraps, allocate nothing.
#[test]
fn slide_events_do_not_allocate() {
    let mut song = Song {
        tempo_bpm: 120.0,
        ..Song::default()
    };
    song.add_channel(ChannelKind::Synth(SynthKind::MiniSynth));
    for type_id in [2, 23, 39] {
        song.channels[1].rack.add(type_id);
    }
    let clip = song.channels[1].lanes[0].add_clip(0, 8.0);
    for (beat, len_beats, key, slide) in
        [(0, 8.0, 57, false), (1, 1.0, 69, true), (3, 2.0, 64, true)]
    {
        clip.events.push(Event::Note {
            pos: beat * 128,
            len_beats,
            key,
            velocity: 200,
            pan: 127,
            slide,
        });
    }
    song.play_loop = Some((0, 6 * 128));
    let mut engine = SongAudioEngine::new(&song, RATE).unwrap();
    let mut output = vec![0.0_f32; 4096 * 2];
    engine.play();
    for frames in BLOCKS {
        engine.render(&mut output[..frames * 2], 2);
    }
    engine.seek(0);
    let mut energy = 0.0_f32;
    let events = events_during(|| {
        for i in 0..400 {
            let frames = BLOCKS[i % BLOCKS.len()];
            engine.render(&mut output[..frames * 2], 2);
            energy += output[..frames * 2].iter().map(|s| s * s).sum::<f32>();
            match i % 100 {
                40 => engine.seek(200),
                70 => engine.stop(),
                75 => engine.play(),
                _ => {}
            }
        }
    });
    assert_eq!(events, 0, "allocator calls on the audio thread");
    assert!(energy > 0.0, "the counted render was silent");
}

/// Mute and Unmute while playing (`CLHd`@36): an adopting engine takes over the
/// playing one and re-syncs on the audio thread, notes, automation, drum hits and audio clips alike, allocating
/// nothing; the replaced engine leaves with the old events.
#[test]
fn a_mute_resync_does_not_allocate() {
    let mut song = audio_clip_song();
    let synth = song.channels.len();
    let drums = synth + 1;
    song.channels
        .extend(self::song().channels.into_iter().skip(1));
    song.channels
        .extend(drum_song().channels.into_iter().skip(1));
    let mut cache = honeydew::sample_cache::SampleCache::default();
    cache.load(&song, &|_| Some(sine_wav(100_000)));
    let flip = |song: &mut Song| {
        for channel in [1, synth, drums] {
            for lane in &mut song.channels[channel].lanes {
                for clip in &mut lane.clips {
                    clip.muted = !clip.muted;
                }
            }
        }
    };
    let mut flipped = song.clone();
    flip(&mut flipped);
    let mut output = vec![0.0_f32; 4096 * 2];
    let mut energy = 0.0_f32;
    for round in 0..4 {
        let (before, after) = if round % 2 == 0 {
            (&song, &flipped)
        } else {
            (&flipped, &song)
        };
        let mut old = SongAudioEngine::with_samples(before, RATE, &cache).unwrap();
        let mut new = SongAudioEngine::with_samples(after, RATE, &cache)
            .unwrap()
            .adopting();
        old.play();
        for frames in BLOCKS {
            old.render(&mut output[..frames * 2], 2);
        }
        let events = events_during(|| {
            new.take_over(&mut old);
            for frames in BLOCKS {
                new.render(&mut output[..frames * 2], 2);
                energy += output[..frames * 2].iter().map(|s| s * s).sum::<f32>();
            }
        });
        assert_eq!(
            events, 0,
            "allocator calls on the audio thread, round {round}"
        );
    }
    assert!(energy > 0.0, "the counted render was silent");
}

/// An edit's engine taking over without a re-sync at
/// another tempo, one silencing a channel then re-syncing, and a tempo drag's live tempo.
#[test]
fn edits_taken_over_and_a_live_tempo_do_not_allocate() {
    let mut song = audio_clip_song();
    let synth = song.channels.len();
    song.channels
        .extend(self::song().channels.into_iter().skip(1));
    song.channels
        .extend(drum_song().channels.into_iter().skip(1));
    let mut cache = honeydew::sample_cache::SampleCache::default();
    cache.load(&song, &|_| Some(sine_wav(100_000)));
    let mut output = vec![0.0_f32; 4096 * 2];
    let mut energy = 0.0_f32;
    for round in 0..2 {
        let mut after = song.clone();
        let new = if round == 0 {
            after.tempo_bpm = 100.0;
            SongAudioEngine::with_samples(&after, RATE, &cache)
                .unwrap()
                .adopting_edit(false, Vec::new())
        } else {
            SongAudioEngine::with_samples(&after, RATE, &cache)
                .unwrap()
                .adopting_edit(true, vec![synth, 1])
        };
        let mut new = new;
        let mut old = SongAudioEngine::with_samples(&song, RATE, &cache).unwrap();
        old.play();
        for frames in BLOCKS {
            old.render(&mut output[..frames * 2], 2);
        }
        let events = events_during(|| {
            new.take_over(&mut old);
            for (i, frames) in BLOCKS.into_iter().enumerate() {
                new.set_tempo(100.0 + 10.0 * i as f32);
                new.render(&mut output[..frames * 2], 2);
                energy += output[..frames * 2].iter().map(|s| s * s).sum::<f32>();
            }
        });
        assert_eq!(
            events, 0,
            "allocator calls on the audio thread, round {round}"
        );
    }
    assert!(energy > 0.0, "the counted render was silent");
}

/// An edit's engine taking over a playing one whose
/// structure differs (a channel deleted and one added; effects added, removed and moved; an instrument switched and one
/// given a preset; a note effect added under held live keys; a drum row removed, a pad's sample and send target changed;
/// a new tempo), re-syncing or not, then rendering, allocates nothing: the old engine keeps what it gives up. Each
/// adopted module is set only the params that changed.
#[test]
fn structural_edits_merged_into_the_playing_engine_do_not_allocate() {
    let mut song = song();
    // A Tape Stop, taken over with the ring built for the new tempo.
    song.channels[1].rack.add(42);
    song.add_channel(ChannelKind::Drums);
    song.add_channel(ChannelKind::EffectTrack);
    song.add_channel(ChannelKind::Synth(SynthKind::SuperSaw));
    song.add_channel(ChannelKind::Synth(SynthKind::MiniSynth));
    let (drums, send) = (2, song.channels[3].id);
    let kit = song.channels[drums].drum_kit.as_mut().unwrap();
    kit.slots[0].file.path = Some(ContentPath::Plain("a.wav".into()));
    kit.slots[1].file.send_target = send;
    let hits = (0..32).map(|i| Event::Drum {
        pos: i * 16,
        len_beats: 0.0,
        pad: (i % 4) as u8,
        velocity: 230,
        pan: 127,
        value: Event::DEFAULT_PITCH,
        repeat: 0,
    });
    (song.channels[drums].lanes[0].add_clip(0, 4.0).events).extend(hits);
    for channel in [4, 5] {
        let clip = song.channels[channel].lanes[0].add_clip(0, 8.0);
        clip.events.push(Event::Note {
            pos: 0,
            len_beats: 8.0,
            key: 57,
            velocity: 200,
            pan: 127,
            slide: false,
        });
    }
    let mut after = song.clone();
    after.tempo_bpm = 100.0;
    after.delete_channel(5);
    after.add_channel(ChannelKind::Synth(SynthKind::GmSynth));
    let rack = &mut after.channels[1].rack;
    let (first, last) = (rack.modules[0].instance_id, rack.modules[4].instance_id);
    rack.remove(first);
    rack.move_up(last);
    rack.add(32);
    // A note effect added under the live keys: they sound on, their note-offs routed where their note-ons went.
    rack.insert(0, 47);
    let mut preset = honeydew::song::ModuleInstance::new(0, 99);
    preset.params[9] = 0.3;
    rack.get_mut(2).unwrap().load_preset(preset);
    let rack = &mut after.channels[4].rack;
    rack.remove(2);
    rack.add(39);
    let kit = after.channels[drums].drum_kit.as_mut().unwrap();
    kit.slots.remove(3);
    kit.slots[0].file.path = Some(ContentPath::Plain("b.wav".into()));
    kit.slots[1].file.send_target = -1;
    kit.slots[2].file.send_target = send;
    (after.channels[drums].lanes[0].clips[0].events)
        .retain(|e| !matches!(e, Event::Drum { pad: 3, .. }));
    let mut before = SampleCache::default();
    before.load(&song, &|_| Some(sine_wav(100_000)));
    let mut edited = SampleCache::default();
    edited.load(&after, &|_| Some(sine_wav(100_000)));
    let mut output = vec![0.0_f32; 4096 * 2];
    let mut energy = 0.0_f32;
    for resync in [false, true] {
        let mut old = SongAudioEngine::with_samples(&song, RATE, &before).unwrap();
        old.play();
        for (i, frames) in BLOCKS.into_iter().enumerate() {
            old.note_on(1, 60 + i as u8, 0.8);
            old.render(&mut output[..frames * 2], 2);
        }
        let mut new = SongAudioEngine::with_samples(&after, RATE, &edited)
            .unwrap()
            .adopting_edit(resync, vec![2]);
        let events = events_during(|| {
            new.take_over(&mut old);
            for frames in BLOCKS {
                new.render(&mut output[..frames * 2], 2);
                energy += output[..frames * 2].iter().map(|s| s * s).sum::<f32>();
            }
        });
        assert_eq!(
            events, 0,
            "allocator calls on the audio thread, re-sync {resync}"
        );
    }
    assert!(energy > 0.0, "the counted render was silent");
}

/// A re-sync deep in a long song restores the cover from a checkpoint made at build, then replays
/// at most a span of actions; neither allocates.
#[test]
fn a_re_sync_from_a_checkpoint_does_not_allocate() {
    let mut song = Song {
        tempo_bpm: 120.0,
        ..Song::default()
    };
    song.add_channel(ChannelKind::Synth(SynthKind::MiniSynth));
    let note = |beat: u32, len_beats, key| Event::Note {
        pos: beat * 128,
        len_beats,
        key,
        velocity: Event::DEFAULT_VELOCITY,
        pan: Event::DEFAULT_PAN,
        slide: false,
    };
    let clip = song.channels[1].lanes[0].add_clip(0, 3000.0);
    clip.events.push(note(0, 2999.0, 40));
    clip.events
        .extend((0..3000).map(|beat| note(beat, 0.5, 60 + (beat % 12) as u8)));
    let mut engine = SongAudioEngine::new(&song, RATE).unwrap();
    engine.play();
    let mut output = vec![0.0_f32; 4096 * 2];
    for frames in BLOCKS {
        engine.render(&mut output[..frames * 2], 2);
    }
    let events = events_during(|| {
        for beat in [2500_u64, 1200, 2900] {
            engine.seek(beat * 128 + 32);
            for frames in BLOCKS {
                engine.render(&mut output[..frames * 2], 2);
            }
        }
    });
    assert_eq!(events, 0, "allocator calls on the audio thread");
}

/// A DW Sampler whose `CSTM` zone loops a user sample: a 16-bit mono WAV of a 220 Hz sine.
fn dw_song() -> (Song, Vec<u8>) {
    let mut song = Song {
        tempo_bpm: 120.0,
        ..Song::default()
    };
    song.add_channel(ChannelKind::Synth(SynthKind::DwSampler));
    let channel = &mut song.channels[1];
    let mut zone = b"@L@\0My Samples/Loop.wav".to_vec();
    zone.resize(1024, 0);
    zone.extend([48, 0, 127, 0, 127]);
    for word in [1.0f32.to_bits(), 0.5f32.to_bits(), 2, 100, 30_000] {
        zone.extend(word.to_le_bytes());
    }
    for value in [0.0f32, 1.0, 1.0, 0.1] {
        zone.extend(value.to_le_bytes());
    }
    let module = &mut channel.rack.modules[0];
    let mut blob = b"10WDZONE".to_vec();
    blob.extend((zone.len() as u32).to_le_bytes());
    blob.extend(zone);
    module.file.custom_data = Some(blob);
    // Lowpass with key and velocity tracking, a filter and pitch envelope, glide, and a Tmp Sync LFO on the cutoff.
    for (index, value) in [
        (7, 0.4),
        (12, 0.5),
        (13, 0.6),
        (14, 0.5),
        (15, 0.3),
        (16, 1.0 / 3.0),
        (21, 0.8),
        (26, 0.6),
        (29, 1.0),
        (31, 0.9),
        (33, 0.1),
    ] {
        module.params[index] = value;
    }
    let id = module.instance_id;
    let clip = channel.lanes[0].add_clip(0, 8.0);
    for (i, key) in [48, 55, 60, 67, 72, 36].into_iter().enumerate() {
        clip.events.push(Event::Note {
            pos: i as u32 * 128,
            len_beats: 1.5,
            key,
            velocity: 180,
            pan: 90,
            slide: false,
        });
    }
    let lane = channel.add_automation_lane(AutomationTarget {
        instance_id: id,
        param: 12,
    });
    lane.add_clip(0, 8.0)
        .events
        .extend((0..8).map(|i| Event::Automation {
            pos: i * 128,
            value: if i % 2 == 0 { 0.3 } else { 0.7 },
            control: true,
        }));
    let data: Vec<u8> = (0..44_100)
        .map(|n| {
            ((std::f64::consts::TAU * 220.0 * f64::from(n) / 44_100.0).sin() * 12_000.0) as i16
        })
        .flat_map(i16::to_le_bytes)
        .collect();
    let mut wav = b"RIFF".to_vec();
    wav.extend((36 + data.len() as u32).to_le_bytes());
    wav.extend(b"WAVEfmt ");
    for word in [16u32, 0x0001_0001, 44_100, 88_200, 0x0010_0002] {
        wav.extend(word.to_le_bytes());
    }
    wav.extend(b"data");
    wav.extend((data.len() as u32).to_le_bytes());
    wav.extend(data);
    (song, wav)
}

#[test]
fn dw_sampler_render_and_commands_do_not_allocate() {
    let (song, wav) = dw_song();
    let mut cache = SampleCache::default();
    cache.load(&song, &|_| Some(wav.clone()));
    let mut engine = SongAudioEngine::with_samples(&song, RATE, &cache).unwrap();
    let id = song.channels[1].rack.modules[0].instance_id;
    let mut output = vec![0.0_f32; 4096 * 2];
    engine.play();
    for frames in BLOCKS {
        engine.render(&mut output[..frames * 2], 2);
    }
    engine.seek(0);

    let mut energy = 0.0_f32;
    let events = events_during(|| {
        let mut rendered = 0;
        let mut i = 0;
        while rendered < 10 * RATE as usize {
            let frames = BLOCKS[i % BLOCKS.len()];
            engine.render(&mut output[..frames * 2], 2);
            energy += output[..frames * 2].iter().map(|s| s * s).sum::<f32>();
            rendered += frames;
            match i % 60 {
                5 => engine.set_param(1, id, 16, (i % 4) as f32 / 3.0),
                10 => engine.set_param(1, id, 11, (i % 2) as f32),
                15 => engine.set_param(1, id, 27, (i % 4) as f32 / 3.0),
                20 => engine.note_on(1, 40 + (i % 30) as u8, 0.8),
                25 => engine.note_off(1, 40 + ((i - 5) % 30) as u8),
                30 => engine.set_param(1, 0, 4, (i % 5) as f32 / 4.0),
                35 => engine.seek(((i * 37) % 1024) as u64),
                40 => engine.stop(),
                45 => engine.play(),
                50 => engine.set_param(1, id, 8, (i % 2) as f32),
                55 => engine.set_param(1, id, 0, ((i / 60) % 2) as f32),
                _ => {}
            }
            i += 1;
        }
    });
    assert_eq!(events, 0, "allocator calls on the audio thread");
    assert!(energy > 0.0, "the counted render was silent");
}

#[test]
fn note_effect_live_state_does_not_allocate() {
    // An Arpeggiator and a Note Echo before the MiniSynth; the engine looks up their state each render call.
    let mut song = song();
    song.channels[1].rack.insert(0, 47);
    song.channels[1].rack.insert(0, 51);
    let mut engine = SongAudioEngine::new(&song, RATE).unwrap();
    let mut output = vec![0.0_f32; 4096 * 2];
    engine.play();
    for frames in BLOCKS {
        engine.render(&mut output[..frames * 2], 2);
    }
    let events = events_during(|| {
        for i in 0..200 {
            let frames = BLOCKS[i % BLOCKS.len()];
            engine.render(&mut output[..frames * 2], 2);
            for meter in 0..engine.meter_count() {
                std::hint::black_box(engine.analysis(meter));
            }
            if i == 100 {
                engine.seek(0);
            }
        }
    });
    assert_eq!(events, 0, "allocator calls on the audio thread");
}

/// MSP: a heavy MiniSynth patch (SUPERSAW, noise, resonant filter, LFO, all four FX on) with chords, voice stealing,
/// mode and kind changes, the FX toggles (the delay clears its buffer), Enable off and on (a reset), pitch bend and
/// the mod wheel: its render and commands allocate nothing.
#[test]
fn minisynth_heavy_patch_does_not_allocate() {
    let mut song = Song {
        tempo_bpm: 128.0,
        ..Song::default()
    };
    song.add_channel(ChannelKind::Synth(SynthKind::MiniSynth));
    let channel = &mut song.channels[1];
    let id = channel.rack.modules[0].instance_id;
    let params = &mut channel.rack.get_mut(id).unwrap().params;
    for (index, value) in [
        (6, 5.0 / 9.0),
        (7, 0.6),
        (8, 0.5),
        (9, 0.5),
        (10, 1.0),
        (17, 0.8),
        (21, 0.9),
        (26, 1.0),
        (27, 0.5),
        (28, 0.2),
        (29, 1.0),
        (31, 0.6),
        (34, 1.0),
        (36, 1.0),
        (38, 1.0),
        (40, 0.8),
        (41, 0.6),
    ] {
        params[index] = value;
    }
    let clip = channel.lanes[0].add_clip(0, 8.0);
    for (i, key) in [48, 55, 60, 64, 67, 72, 76].into_iter().enumerate() {
        clip.events.push(Event::Note {
            pos: i as u32 * 64,
            len_beats: 3.0,
            key,
            velocity: 200,
            pan: 127,
            slide: false,
        });
    }
    let mut engine = SongAudioEngine::new(&song, RATE).unwrap();
    let mut output = vec![0.0_f32; 4096 * 2];
    engine.play();
    for frames in BLOCKS {
        engine.render(&mut output[..frames * 2], 2);
    }
    engine.seek(0);

    let mut energy = 0.0_f32;
    let events = events_during(|| {
        let mut rendered = 0;
        let mut i = 0;
        while rendered < 10 * RATE as usize {
            let frames = BLOCKS[i % BLOCKS.len()];
            engine.render(&mut output[..frames * 2], 2);
            energy += output[..frames * 2].iter().map(|s| s * s).sum::<f32>();
            rendered += frames;
            match i % 60 {
                5 => engine.set_param(1, id, 1, (i % 3) as f32 / 2.0),
                10 => engine.set_param(1, id, 6, (i % 10) as f32 / 9.0),
                15 => engine.set_param(1, id, 29, (i % 2) as f32),
                20 => (0..8).for_each(|k| engine.note_on(1, 50 + k * 3, 0.8)),
                25 => (0..8).for_each(|k| engine.note_off(1, 50 + k * 3)),
                30 => engine.set_param(1, id, 0, (i % 2) as f32),
                33 => engine.set_param(1, id, 0, 1.0),
                35 => engine.seek(((i * 37) % 1024) as u64),
                40 => engine.stop(),
                45 => engine.play(),
                50 => engine.set_param(1, 0, 4, (i % 5) as f32 / 4.0),
                55 => engine.set_param(1, 0, 8, (i % 3) as f32 / 2.0),
                _ => {}
            }
            i += 1;
        }
    });
    assert_eq!(events, 0, "allocator calls on the audio thread");
    assert!(energy > 0.0, "the counted render was silent");
}

/// The drum song (one pad sends to channel 2, a send channel) plus a MiniSynth on channel 3 that sends to the
/// master and to channel 2, with Enable Send and Send Level lanes, and a key range on the synth.
fn send_song() -> Song {
    let mut song = drum_song();
    song.add_channel(ChannelKind::Synth(SynthKind::MiniSynth));
    let send = song.channels[2].id;
    let synth = &mut song.channels[3];
    synth.file.sends.push(honeydew::song::SendState {
        target_channel_id: send,
        level: 0.6,
        ..honeydew::song::SendState::default()
    });
    (
        synth.rack.modules[0].file.key_low,
        synth.rack.modules[0].file.key_high,
    ) = (40, 90);
    let clip = synth.lanes[0].add_clip(0, 4.0);
    clip.events.extend((0..16).map(|i| Event::Note {
        pos: i * 32,
        len_beats: 0.2,
        key: 30 + (i * 5) as u8,
        velocity: 200,
        pan: 127,
        slide: false,
    }));
    for param in [0x1000 + send, 0x2000 + send] {
        synth
            .add_automation_lane(AutomationTarget {
                instance_id: 0,
                param,
            })
            .add_clip(0, 4.0)
            .events
            .extend((0..8).map(|i| Event::Automation {
                pos: i * 64,
                value: if i % 2 == 0 { 0.3 } else { 0.9 },
                control: true,
            }));
    }
    song
}

#[test]
fn sends_pad_sends_and_key_ranges_do_not_allocate() {
    let song = send_song();
    let send = song.channels[2].id;
    let mut cache = SampleCache::default();
    cache.load(&song, &|_| None);
    let mut engine = SongAudioEngine::with_samples(&song, RATE, &cache).unwrap();
    let mut output = vec![0.0_f32; 4096 * 2];
    engine.play();
    for frames in BLOCKS {
        engine.render(&mut output[..frames * 2], 2);
    }
    engine.seek(0);

    let mut energy = 0.0_f32;
    let events = events_during(|| {
        let mut rendered = 0;
        let mut i = 0;
        while rendered < 10 * RATE as usize {
            let frames = BLOCKS[i % BLOCKS.len()];
            engine.render(&mut output[..frames * 2], 2);
            energy += output[..frames * 2].iter().map(|s| s * s).sum::<f32>();
            rendered += frames;
            match i % 40 {
                3 => engine.note_on(3, 30 + (i % 70) as u8, 0.9),
                6 => engine.note_on(1, 48 + (i % 9) as u8, 0.7),
                9 => engine.note_off(3, 30 + ((i - 6) % 70) as u8),
                12 => engine.set_param(3, 0, 0x1000 + send, (i % 3) as f32 / 2.0),
                15 => engine.set_param(3, 0, 0x2000 + send, (i % 5) as f32 / 4.0),
                18 => engine.set_param(2, 0, 3, ((i / 40) % 2) as f32),
                21 => engine.set_setting(
                    3,
                    2,
                    honeydew_audio::instrument::KEY_LEARN,
                    ((i / 40) % 2) as f32,
                ),
                24 => engine.seek(((i * 53) % 512) as u64),
                27 => engine.set_param(1, 0, 2, ((i / 40) % 2) as f32),
                30 => engine.stop(),
                33 => engine.play(),
                _ => {}
            }
            i += 1;
        }
    });
    assert_eq!(events, 0, "allocator calls on the audio thread");
    assert!(energy > 0.0, "the counted render was silent");
}

/// The pickers' preview never frees a sample on the audio thread. The link keeps each sample it
/// sent ([`honeydew::audio_link::AudioLink`]) until the engine has let go, so the engine never drops the last one: not on
/// a start, a stop, a replacing pick, or the sample's end.
#[test]
fn the_preview_never_frees_on_the_audio_thread() {
    use honeydew_audio::sample::SampleData;
    let mut engine = SongAudioEngine::new(&Song::default(), RATE).unwrap();
    let mut output = vec![0.0_f32; 64 * 2];
    let sample = || {
        Arc::new(SampleData {
            sample_rate: 48_000,
            left: vec![0.1; 4800].into(),
            right: None,
            meta: Default::default(),
        })
    };
    engine.render(&mut output, 2);
    // A local stand-in for `AudioLink::previews` (the real one is tested in `audio_link`): what the link keeps of each
    // preview it sent; the strip has let go of them all.
    let mut link = Vec::new();
    let mut send = |engine: &mut SongAudioEngine, data: Option<Arc<SampleData>>| {
        link.extend(data.clone());
        engine.preview(data.map(|d| (d, 0)));
    };
    send(&mut engine, Some(sample()));
    let started = events_during(|| engine.render(&mut output, 2));
    // A new pick replaces it, then the picker closes.
    let (a, b) = (sample(), None);
    let replaced = events_during(|| send(&mut engine, Some(a)));
    let stopped = events_during(|| send(&mut engine, b));
    // One that plays to its end.
    send(&mut engine, Some(sample()));
    let ended = events_during(|| {
        for _ in 0..100 {
            engine.render(&mut output, 2);
        }
    });
    assert_eq!((started, replaced, stopped, ended), (0, 0, 0, 0));
    // The link lets go on its own thread once the engine has.
    link.retain(|d| Arc::strong_count(d) > 1);
    assert!(link.is_empty());
}

/// The keys sounding at each channel's input are set and cleared on the audio thread without allocating: song
/// notes into an Arpeggiator, live keys, loop wraps, a seek and Stop.
#[test]
fn the_sounding_keys_update_without_allocating() {
    let mut song = song();
    song.channels[1].rack.insert(0, 51);
    song.play_loop = Some((0, 512));
    let mut engine = SongAudioEngine::new(&song, RATE).unwrap();
    let mut output = vec![0.0_f32; 4096 * 2];
    engine.play();
    for frames in BLOCKS {
        engine.render(&mut output[..frames * 2], 2);
    }
    engine.seek(0);
    let mut lit = 0;
    let events = events_during(|| {
        for i in 0..200 {
            match i % 10 {
                0 => engine.note_on(1, 50, 1.0),
                5 => engine.note_off(1, 50),
                _ => {}
            }
            let frames = BLOCKS[i % BLOCKS.len()];
            engine.render(&mut output[..frames * 2], 2);
            lit |= engine.sounding(1);
        }
        engine.seek(0);
        engine.stop();
        engine.play();
    });
    assert_eq!(events, 0);
    assert_eq!(lit & (1 << 50 | 1 << 60), 1 << 50 | 1 << 60, "{lit:b}");
}

/// The position re-sync allocates nothing on the audio thread: seeks
/// while playing over held and covered notes, step and notes-lane drum hits, automation and audio clips; loop wraps;
/// play start; a count-in's end; and seeks while stopped, which only move the position.
#[test]
fn the_position_resync_does_not_allocate() {
    let mut song = audio_clip_song();
    song.channels
        .extend(self::song().channels.into_iter().skip(1));
    song.channels
        .extend(drum_song().channels.into_iter().skip(1));
    let mut cache = honeydew::sample_cache::SampleCache::default();
    cache.load(&song, &|_| Some(sine_wav(100_000)));
    let mut engine = SongAudioEngine::with_samples(&song, RATE, &cache).unwrap();
    let mut output = vec![0.0_f32; 4096 * 2];
    // Warm-up: every block size, a count-in, a seek while playing and one while stopped.
    engine.count_in(1);
    engine.play();
    for _ in 0..30 {
        for frames in BLOCKS {
            engine.render(&mut output[..frames * 2], 2);
        }
    }
    engine.seek(500);
    engine.stop();
    engine.seek(0);
    engine.play();
    let mut energy = 0.0_f32;
    let events = events_during(|| {
        for i in 0..420 {
            let frames = BLOCKS[i % BLOCKS.len()];
            engine.render(&mut output[..frames * 2], 2);
            energy += output[..frames * 2].iter().map(|s| s * s).sum::<f32>();
            match i % 70 {
                10 | 30 => engine.seek(((i * 37) % 1024) as u64),
                40 => engine.stop(),
                41 => engine.seek(((i * 53) % 900) as u64),
                42 => {
                    engine.count_in(1);
                    engine.play();
                }
                _ => {}
            }
        }
    });
    assert_eq!(events, 0, "allocator calls on the audio thread");
    assert!(energy > 0.0, "the counted render was silent");
}

/// A sound reset (REV at the play-start point) allocates nothing on the audio thread: synth voices and their effects,
/// drum pads, audio clips and live keys, called while playing and while stopped.
#[test]
fn a_sound_reset_does_not_allocate() {
    let mut song = audio_clip_song();
    song.channels
        .extend(self::song().channels.into_iter().skip(1));
    song.channels
        .extend(drum_song().channels.into_iter().skip(1));
    let mut cache = honeydew::sample_cache::SampleCache::default();
    cache.load(&song, &|_| Some(sine_wav(100_000)));
    let mut engine = SongAudioEngine::with_samples(&song, RATE, &cache).unwrap();
    let mut output = vec![0.0_f32; 4096 * 2];
    // Warm-up: every block size, a sound reset, a live key.
    engine.play();
    for frames in BLOCKS {
        engine.render(&mut output[..frames * 2], 2);
    }
    engine.note_on(2, 60, 1.0);
    engine.reset_sound();
    engine.render(&mut output, 2);
    let mut energy = 0.0_f32;
    let events = events_during(|| {
        for i in 0..280 {
            let frames = BLOCKS[i % BLOCKS.len()];
            engine.render(&mut output[..frames * 2], 2);
            energy += output[..frames * 2].iter().map(|s| s * s).sum::<f32>();
            match i % 70 {
                10 => engine.note_on(2, 64, 1.0),
                20 => engine.reset_sound(),
                40 => engine.stop(),
                41 => engine.reset_sound(),
                45 => engine.play(),
                _ => {}
            }
        }
    });
    assert_eq!(events, 0, "allocator calls on the audio thread");
    assert!(energy > 0.0, "the counted render was silent");
}

/// Live keys held through an Arpeggiator, a Note Echo and a Transpose while a tempo drag
/// down to 60 BPM retimes them and the loop wraps at the dragged tempos; a synced 16-beat Tape Stop started at 60 BPM,
/// cut to its 120 BPM ring; then the edit's engine at 60 BPM, its Transpose moved up, takes over mid-stop and hands the
/// Tape Stop its longer ring. The keys sound on, and nothing allocates.
#[test]
fn a_tempo_drag_and_moved_note_effects_under_held_keys_do_not_allocate() {
    let mut song = song();
    song.play_loop = Some((0, 128 + 37));
    let rack = &mut song.channels[1].rack;
    rack.insert(0, 51);
    let echo = rack.insert(1, 47);
    rack.get_mut(echo).unwrap().params[1] = 4.0 / 32.0;
    let transpose = rack.insert(2, 48);
    let tape = rack.add(42);
    (rack.get_mut(tape).unwrap().params[3..8]).copy_from_slice(&[1.0, 1.0 / 3.0, 1.0, 0.0, 1.0]);
    let mut after = song.clone();
    after.set_tempo(60.0);
    after.channels[1].rack.move_up(transpose);
    after.channels[1].rack.get_mut(tape).unwrap().params[5] = 0.0;
    let mut new = SongAudioEngine::new(&after, RATE)
        .unwrap()
        .adopting_edit(false, Vec::new());
    let mut old = SongAudioEngine::new(&song, RATE).unwrap();
    let mut output = vec![0.0_f32; 4096 * 2];
    // Warm-up: every block size, the keys down.
    old.play();
    for (i, frames) in BLOCKS.into_iter().enumerate() {
        old.note_on(1, 62 + 4 * i as u8, 0.8);
        old.render(&mut output[..frames * 2], 2);
    }
    let mut energy = 0.0_f32;
    let events = events_during(|| {
        for i in 0..120 {
            old.set_tempo(120.0 - i as f32 / 2.0);
            let frames = BLOCKS[i % BLOCKS.len()];
            old.render(&mut output[..frames * 2], 2);
        }
        old.set_param(1, tape, 5, 0.0);
        for frames in BLOCKS {
            old.render(&mut output[..frames * 2], 2);
        }
        new.take_over(&mut old);
        for i in 0..70 {
            let frames = BLOCKS[i % BLOCKS.len()];
            new.render(&mut output[..frames * 2], 2);
            energy += output[..frames * 2].iter().map(|s| s * s).sum::<f32>();
        }
    });
    assert_eq!(events, 0, "allocator calls on the audio thread");
    assert!(energy > 0.0, "the counted render was silent");
    assert_eq!(new.sounding(1) & 1 << 62, 1 << 62, "a held key released");
}

/// Play start (the preview stopped, every live, sequenced and pedal-held
/// key force-released, a press queued with it kept, a count-in or not), Stop's param scope, a re-sync's and the loop
/// wrap's forced note-offs under a pedal down allocate nothing on the audio thread.
#[test]
fn play_start_stop_and_the_wraps_forced_note_offs_do_not_allocate() {
    use honeydew_audio::sample::SampleData;
    let mut song = song();
    song.channels[1].rack.insert(0, 51);
    // Half a beat, 12 000 frames: a wrap or two while playing, over the held note from beat 0.
    song.play_loop = Some((0, 64));
    let mut engine = SongAudioEngine::new(&song, RATE).unwrap();
    let mut output = vec![0.0_f32; 4096 * 2];
    // What the link keeps of the previews it sent: the engine never drops the last one.
    let preview = Arc::new(SampleData {
        sample_rate: 48_000,
        left: vec![0.1; 48_000].into(),
        right: None,
        meta: Default::default(),
    });
    let mut step = |engine: &mut SongAudioEngine, i: usize| {
        let frames = BLOCKS[i % BLOCKS.len()];
        engine.render(&mut output[..frames * 2], 2);
        match i % 30 {
            0 => engine.set_param(1, 0, 71, 1.0),
            1 => engine.note_on(1, 50 + (i % 7) as u8, 1.0),
            3 => engine.note_off(1, 50 + (i % 7) as u8),
            4 => engine.preview(Some((Arc::clone(&preview), 0))),
            5 => engine.note_on(1, 62, 0.8),
            6 => {
                engine.count_in((i % 60 == 6).into());
                engine.note_on(1, 70, 0.8);
                engine.play();
            }
            12 => engine.set_param(1, 0, 4, 0.9),
            16 => engine.seek(((i * 37) % 600) as u64),
            20 => engine.note_off(1, 62),
            22 => engine.note_off(1, 70),
            25 => engine.stop(),
            _ => {}
        }
        output[..frames * 2].iter().map(|s| s * s).sum::<f32>()
    };
    // Warm-up: every block size and command once.
    for i in 0..120 {
        step(&mut engine, i);
    }
    let mut energy = 0.0_f32;
    let events = events_during(|| {
        for i in 0..1500 {
            energy += step(&mut engine, i);
        }
    });
    assert_eq!(events, 0, "allocator calls on the audio thread");
    assert!(energy > 0.0, "the counted render was silent");
}

/// An edit's engine takes the playing engine's MIDI CCs (`keep_ccs`), each set where the new
/// engine holds another value: a live sustain pedal down holding note-offs, a live CC #2 and an automated CC #1,
/// re-syncing or not, rolling or after Stop. Neither the take-over nor the render after it allocates.
#[test]
fn the_ccs_an_edit_keeps_do_not_allocate() {
    let mut song = song();
    let target = AutomationTarget {
        instance_id: 0,
        param: 8,
    };
    let clip = song.channels[1]
        .add_automation_lane(target)
        .add_clip(0, 8.0);
    clip.events.extend((0..8).map(|i| Event::Automation {
        pos: i * 128,
        value: if i % 2 == 0 { 0.2 } else { 0.8 },
        control: true,
    }));
    let mut after = song.clone();
    after.channels[1].rack.add(32);
    let mut output = vec![0.0_f32; 4096 * 2];
    for resync in [false, true] {
        for stop in [false, true] {
            let mut old = SongAudioEngine::new(&song, RATE).unwrap();
            old.play();
            // The pedal (CC #64, param 71) down and CC #2 (param 9) live; past beat 1, so CC #1 is at 0.8.
            old.set_param(1, 0, 71, 1.0);
            old.set_param(1, 0, 9, 0.4);
            for (i, frames) in BLOCKS.into_iter().cycle().take(28).enumerate() {
                let key = 48 + (i % 12) as u8;
                old.note_on(1, key, 0.8);
                old.render(&mut output[..frames * 2], 2);
                old.note_off(1, key);
            }
            if stop {
                old.stop();
            }
            let mut new = SongAudioEngine::new(&after, RATE)
                .unwrap()
                .adopting_edit(resync, Vec::new());
            let events = events_during(|| {
                new.take_over(&mut old);
                for frames in BLOCKS {
                    new.render(&mut output[..frames * 2], 2);
                }
            });
            assert_eq!(events, 0, "re-sync {resync}, Stop {stop}");
        }
    }
}
