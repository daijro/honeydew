use super::*;
use crate::midi::MidiMsg::{self, *};
use crate::smf::codec::{Smf, SmfEvent};
use crate::smf::export::smf_of;
use crate::song::{Channel, ChannelKind, Event, Lane, LaneKind, Song, SynthKind, points};

// --- helpers ---

type Ev = (u64, SmfEvent);

fn v(t: u64, msg: MidiMsg) -> Ev {
    (t, SmfEvent::Voice(msg))
}

fn on(t: u64, ch: u8, key: u8, vel: u8) -> Ev {
    v(t, NoteOn { ch, key, vel })
}

fn off(t: u64, ch: u8, key: u8) -> Ev {
    v(t, NoteOff { ch, key, vel: 0 })
}

fn cc(t: u64, ch: u8, cc: u8, value: u8) -> Ev {
    v(t, Cc { ch, cc, value })
}

fn pb(t: u64, ch: u8, value: u16) -> Ev {
    v(t, PitchBend { ch, value })
}

fn prog(ch: u8, program: u8) -> Ev {
    v(0, Program { ch, program })
}

fn name(s: &str) -> Ev {
    (0, SmfEvent::Name(s.as_bytes().to_vec()))
}

/// A format-1 file at `division` ticks a beat.
fn smf(division: u16, tracks: Vec<Vec<Ev>>) -> Vec<u8> {
    codec::write(&Smf { division, tracks })
}

/// `bytes` opened as a song.
fn open(bytes: &[u8]) -> Song {
    let mut song = Song::default();
    import(
        &mut song,
        0,
        bytes,
        &ImportOpts::OPEN_AS_SONG,
        true,
        crate::content::factory(),
    )
    .expect("imports");
    song
}

/// A song with one instrument channel `Keys` (index 1; one note in a 4-beat clip on its lane), and `bytes`
/// imported into it with `opts` from the track menu (not as a song).
fn into_keys(bytes: &[u8], opts: ImportOpts) -> Song {
    let mut song = keys_song();
    import(&mut song, 1, bytes, &opts, false, crate::content::factory()).expect("imports");
    song
}

fn keys_song() -> Song {
    let mut song = Song::default();
    song.add_channel(ChannelKind::Instrument);
    song.channels[1].name = "Keys".into();
    song.channels[1].lanes[0].add_clip(0, 4.0).events = vec![note(0, 1.0, 72, 204)];
    song
}

fn note(pos: u32, len_beats: f64, key: u8, velocity: u8) -> Event {
    Event::Note {
        pos,
        len_beats,
        key,
        velocity,
        pan: Event::DEFAULT_PAN,
        slide: false,
    }
}

/// The notes of the lane's one clip: (song tick, key, velocity, length in beats). Every one has pan 127 and no
/// slide.
fn notes(lane: &Lane) -> Vec<(u32, u8, u8, f64)> {
    assert_eq!(lane.kind, LaneKind::Notes);
    assert_eq!(lane.clips.len(), 1, "{}", lane.label);
    let clip = &lane.clips[0];
    (clip.events.iter())
        .map(|e| match *e {
            Event::Note {
                pos,
                len_beats,
                key,
                velocity,
                pan,
                slide,
            } => {
                assert_eq!((pan, slide), (127, false));
                (clip.start_tick + pos, key, velocity, len_beats)
            }
            other => panic!("not a note: {other:?}"),
        })
        .collect()
}

/// The control points of the lane's one clip: (song tick, stored u16).
fn pts(lane: &Lane) -> Vec<(u32, u16)> {
    assert_eq!(lane.clips.len(), 1, "{}", lane.label);
    let clip = &lane.clips[0];
    (points::points(clip, points::PointKey::Automation).iter())
        .map(|p| (clip.start_tick + p.tick, p.value))
        .collect()
}

fn names(song: &Song) -> Vec<&str> {
    song.channels.iter().map(|c| c.name.as_str()).collect()
}

fn labels(c: &Channel) -> Vec<(LaneKind, &str)> {
    c.lanes.iter().map(|l| (l.kind, l.label.as_str())).collect()
}

/// The channel-scope automation lanes, (param, label), in lane order.
fn autos(c: &Channel) -> Vec<(i32, &str)> {
    (c.lanes.iter())
        .filter(|l| l.kind == LaneKind::Automation)
        .map(|l| {
            let t = l.target.expect("an automation target");
            assert_eq!(t.instance_id, 0);
            (t.param, l.label.as_str())
        })
        .collect()
}

fn lane<'a>(c: &'a Channel, label: &str) -> &'a Lane {
    (c.lanes.iter())
        .find(|l| l.label == label)
        .unwrap_or_else(|| panic!("no lane {label}: {:?}", labels(c)))
}

// --- parsing ---

#[test]
fn division_96_and_below_one() {
    let tracks = || vec![vec![name("A"), on(96, 0, 60, 100), off(144, 0, 60)]];
    // Beat = tick / division: tick 96 is beat 1 (song tick 128), 48 ticks are half a beat; velocity 100 → 200.
    let expect = vec![(128, 60, 200, 0.5)];
    let bytes = smf(96, tracks());
    assert_eq!(notes(&open(&bytes).channels[1].lanes[0]), expect);
    // Division 0, SMPTE (−25 fps) and other negatives read as 96.
    for division in [[0, 0], [0xE7, 0x28], [0xFF, 0xFF], [0x80, 0]] {
        let mut bytes = smf(480, tracks());
        bytes[12..14].copy_from_slice(&division);
        assert_eq!(
            notes(&open(&bytes).channels[1].lanes[0]),
            expect,
            "{division:?}"
        );
    }
    // Off the tick grid: 7/480 beat is 1.87 ticks, rounded to 2.
    let bytes = smf(480, vec![vec![on(7, 0, 60, 100)]]);
    assert_eq!(
        notes(&open(&bytes).channels[1].lanes[0]),
        [(2, 60, 200, 1.0)]
    );
    // 480 ticks a beat: the same beats.
    let bytes = smf(480, vec![vec![on(480, 0, 60, 100), off(720, 0, 60)]]);
    assert_eq!(notes(&open(&bytes).channels[1].lanes[0]), expect);
}

#[test]
fn open_as_song_tempo() {
    let tempo = |tracks| open(&smf(96, tracks)).tempo_bpm;
    let t = |at, us| (at, SmfEvent::Tempo(us));
    let bad = (
        0,
        SmfEvent::Other {
            kind: 0x51,
            data: vec![7, 0xA1],
        },
    );
    assert_eq!(tempo(vec![vec![t(0, 500_000)]]), 120.0);
    assert_eq!(tempo(vec![vec![t(0, 600_000)]]), 100.0);
    // Clamped to [1, 999]; kept outside the tempo control's drag range.
    assert_eq!(tempo(vec![vec![t(0, 1)]]), 999.0);
    assert_eq!(tempo(vec![vec![t(0, 0)]]), 999.0);
    assert_eq!(tempo(vec![vec![t(0, 0xFF_FFFF)]]), 60e6 / 16_777_215.0);
    assert_eq!(tempo(vec![vec![t(0, 200_000)]]), 300.0);
    assert_eq!(tempo(vec![vec![t(0, 100_000)]]), 600.0);
    // The tempo at tick 0: the last one there in file order (the 60 at tick 960 starts a tempo map, below).
    let two = vec![vec![t(0, 500_000), t(960, 1_000_000)], vec![t(0, 400_000)]];
    assert_eq!(tempo(two), 150.0);
    // One tempo, said twice: no tempo lane.
    let one = open(&smf(96, vec![vec![t(0, 600_000), t(960, 600_000)]]));
    assert_eq!(one.tempo_bpm, 100.0);
    assert_eq!(one.channels[0].lanes.len(), 1);
    // No FF 51 → 120, the MIDI default; a malformed one is the parse error → 120 too.
    assert_eq!(tempo(vec![vec![on(0, 0, 60, 64)]]), 120.0);
    assert_eq!(tempo(vec![]), 120.0);
    assert_eq!(tempo(vec![vec![t(0, 1_000_000), bad.clone()]]), 120.0);
    assert_eq!(tempo(vec![vec![bad.clone(), t(0, 1_000_000)]]), 60.0);
    // The time signature is never read.
    let sig = (
        0,
        SmfEvent::Other {
            kind: 0x58,
            data: vec![3, 2, 24, 8],
        },
    );
    assert_eq!(
        open(&smf(96, vec![vec![sig, t(0, 500_000)]])).time_sig,
        Song::default().time_sig
    );
    // Track-menu and dialog imports never change the tempo: the file's 60 BPM stays out of the 120 BPM song.
    let bytes = smf(96, vec![vec![t(0, 1_000_000), on(0, 0, 60, 64)]]);
    assert_eq!(into_keys(&bytes, ImportOpts::ONE_CHANNEL).tempo_bpm, 120.0);
    assert_eq!(into_keys(&bytes, ImportOpts::OPEN_AS_SONG).tempo_bpm, 120.0);
    let mut song = keys_song();
    import(
        &mut song,
        1,
        &bytes,
        &ImportOpts::ONE_CHANNEL,
        false,
        crate::content::factory(),
    )
    .unwrap();
    assert_eq!(song.tempo_bpm, 120.0);
}

#[test]
fn a_tempo_map_imports_as_a_square_tempo_lane_that_plays_and_exports_back() {
    use crate::song::{TEMPO_TARGET, tempo_value};
    use honeydew_audio::device::AudioRenderer;
    use points::{PointKey, Shape, ShapeKind};
    // 120 BPM, 90 from bar 3, 140 from bar 5, at 96 ticks a beat: bar 3 is file tick 768, song tick 1024.
    let t = |at, bpm: f64| (at, SmfEvent::Tempo((60e6 / bpm).round() as u32));
    let bytes = smf(
        96,
        vec![vec![
            t(0, 120.0),
            on(0, 0, 60, 64),
            off(96, 0, 60),
            t(768, 90.0),
            t(1536, 140.0),
        ]],
    );
    let song = open(&bytes);
    assert_eq!(song.tempo_bpm, 120.0);
    let tempo = lane(&song.channels[0], "Tempo");
    assert_eq!(tempo.target, Some(TEMPO_TARGET));
    let value = |bpm| points::raw(tempo_value(bpm));
    let want = [(0, value(120.0)), (1024, value(90.0)), (2048, value(140.0))];
    assert_eq!(pts(tempo), want);
    // Square at 100 %: each tempo holds up to the next point and jumps there.
    let square = Shape {
        kind: ShapeKind::Square,
        tension: 100,
    };
    let shapes = points::points(&tempo.clips[0], PointKey::Automation);
    assert!(shapes.iter().all(|p| p.shape == square), "{shapes:?}");
    // Played: 4 s at 120 reach bar 3 (a jump halfway would have played 90 from bar 2), 5⅓ s at 90 reach bar 5.
    let mut engine = crate::audio_engine::SongAudioEngine::new(&song, 48_000.0).unwrap();
    engine.play();
    let mut out = vec![0.0; 512];
    let mut ticks = Vec::new();
    for frames in [192_000, 256_000] {
        for _ in 0..frames / 256 {
            engine.render(&mut out, 2);
        }
        ticks.push(engine.transport_tick());
    }
    assert!((1022..=1024).contains(&ticks[0]), "{ticks:?}");
    assert!((2046..=2048).contains(&ticks[1]), "{ticks:?}");
    // Exported and read back: the same tempo changes.
    let back = open(&smf_of(&song, None).unwrap());
    assert_eq!(back.tempo_bpm, 120.0);
    assert_eq!(pts(lane(&back.channels[0], "Tempo")), want);
    // Into the current song: its tempo and master stay.
    let into = into_keys(&bytes, ImportOpts::OPEN_AS_SONG);
    assert_eq!(into.channels[0].lanes.len(), 1);
}

#[test]
fn failures_leave_the_song() {
    let mut song = keys_song();
    let before = song.clone();
    for bytes in [
        &b""[..],
        b"MThd",
        b"RIFF\0\0\0\x06\0\x01\0\x01\0\x60",
        b"garbage bytes here",
    ] {
        assert!(scan(bytes).is_err());
        assert!(
            import(
                &mut song,
                1,
                bytes,
                &ImportOpts::ONE_CHANNEL,
                false,
                crate::content::factory()
            )
            .is_err()
        );
        assert!(
            import(
                &mut song,
                0,
                bytes,
                &ImportOpts::OPEN_AS_SONG,
                true,
                crate::content::factory()
            )
            .is_err()
        );
    }
    // A current channel that does not exist.
    let good = smf(96, vec![vec![on(0, 0, 60, 64)]]);
    assert!(
        import(
            &mut song,
            2,
            &good,
            &ImportOpts::ONE_CHANNEL,
            false,
            crate::content::factory()
        )
        .is_err()
    );
    assert_eq!(song, before);
    assert_eq!(
        LOAD_FAILED,
        "Could not load MIDI file. It may be corrupted."
    );
}

// --- channels ---

#[test]
fn split_channels_are_shared_across_tracks_and_renamed() {
    let bytes = smf(
        96,
        vec![
            vec![
                name("Piano"),
                prog(0, 5),
                prog(0, 0),
                on(0, 0, 60, 64),
                off(96, 0, 60),
            ],
            vec![name("Bass"), prog(1, 4), on(0, 1, 36, 64), prog(1, 33)],
            vec![name("Piano LH"), on(192, 0, 48, 64), off(288, 0, 48)],
            vec![name("Kit"), prog(9, 16), on(0, 9, 36, 127), off(24, 9, 36)],
            vec![name("Lead"), prog(2, 119), on(0, 2, 72, 64)],
            vec![name("Pad"), on(0, 3, 50, 64)],
            // A program change alone makes no channel.
            vec![name("Nothing"), prog(4, 7)],
        ],
    );
    let song = open(&bytes);
    // `<track name> (MIDI Channel c: <program name>)`: c 0-based; the last program wins; `Drums` on 9; else the GM
    // name (program 0 with no program change).
    assert_eq!(
        names(&song),
        [
            "MASTER",
            "Piano (MIDI Channel 0: Acoustic Grand Piano)",
            "Bass (MIDI Channel 1: Electric Bass (finger))",
            "Kit (MIDI Channel 9: Drums)",
            "Lead (MIDI Channel 2: Reverse Cymbal)",
            "Pad (MIDI Channel 3: Acoustic Grand Piano)",
        ]
    );
    // The SMF tracks on MIDI channel 0 share its channel; the later one adds its own lane.
    let piano = &song.channels[1];
    assert_eq!(
        labels(piano),
        [(LaneKind::Notes, "Piano"), (LaneKind::Notes, "Piano LH")]
    );
    assert_eq!(notes(&piano.lanes[0]), [(0, 60, 128, 1.0)]);
    assert_eq!(notes(&piano.lanes[1]), [(256, 48, 128, 1.0)]);
    assert!(
        song.channels[1..]
            .iter()
            .all(|c| c.kind.file_kind() == 0x100)
    );

    // Only the selected MIDI channels are imported (here 0 and 9).
    let mut song = keys_song();
    let keys = song.channels[1].clone();
    let opts = ImportOpts {
        channels: 1 | 1 << 9,
        ..ImportOpts::OPEN_AS_SONG
    };
    import(
        &mut song,
        1,
        &bytes,
        &opts,
        false,
        crate::content::factory(),
    )
    .unwrap();
    assert_eq!(
        names(&song),
        [
            "MASTER",
            "Keys",
            "Piano (MIDI Channel 0: Acoustic Grand Piano)",
            "Kit (MIDI Channel 9: Drums)",
        ]
    );
    // The current channel is left alone with Split on.
    assert_eq!(song.channels[1], keys);
    // Split off: the selected channels' notes, into the current channel.
    let mut song = keys_song();
    let opts = ImportOpts {
        channels: 1 << 1,
        ..ImportOpts::ONE_CHANNEL
    };
    import(
        &mut song,
        1,
        &bytes,
        &opts,
        false,
        crate::content::factory(),
    )
    .unwrap();
    assert_eq!(names(&song), ["MASTER", "Keys"]);
    assert_eq!(
        labels(&song.channels[1]),
        [(LaneKind::Notes, ""), (LaneKind::Notes, "Bass")]
    );
    assert_eq!(notes(&song.channels[1].lanes[1]), [(0, 36, 128, 1.0)]);
}

#[test]
fn imported_channels_play_their_program_and_gm_drums_from_running_status() {
    // Format 0, 96 PPQ, running status throughout: program 33 (finger bass) on channel 0, GM drums on channel 10.
    let track: &[u8] = &[
        0x00, 0xC0, 33, // program 33
        0x00, 0x90, 40, 100, // note on
        0x60, 40, 0, // running status, v 0: off a beat later
        0x00, 43, 90, // running status: on
        0x84, 0x40, 43, 0, // off 6 beats later
        0x00, 0x99, 36, 127, // kick
        0x18, 36, 0, //
        0x00, 38, 100, // snare
        0x18, 38, 0, //
        0x00, 0xFF, 0x2F, 0x00,
    ];
    let mut bytes = b"MThd\0\0\0\x06\0\0\0\x01\0\x60MTrk".to_vec();
    bytes.extend((track.len() as u32).to_be_bytes());
    bytes.extend(track);
    // Without the bundle (a unit test's factory): our code-made preset for the program's family, and the Relay Kit.
    let song = open(&bytes);
    assert_eq!(
        names(&song),
        [
            "MASTER",
            "(MIDI Channel 0: Electric Bass (finger))",
            "(MIDI Channel 9: Drums)"
        ]
    );
    let bass = &song.channels[1];
    assert_eq!(bass.kind, ChannelKind::Synth(SynthKind::MiniSynth));
    assert_eq!(bass.rack.modules[0].file.preset_name, "Velvet Sub");
    assert_eq!(
        notes(&bass.lanes[0]),
        [(0, 40, 200, 1.0), (128, 43, 180, 6.0)]
    );
    // The clip covers the last note's end, not just its start.
    assert_eq!(bass.lanes[0].clips[0].visible_len_beats, 7.0);
    let drums = &song.channels[2];
    assert_eq!(drums.kind, ChannelKind::Drums);
    let kit = drums.drum_kit.as_ref().unwrap();
    assert_eq!(kit.slots[0].pad_name(), "Kick");
    assert_eq!(
        (kit.slots[0].midi_note(), kit.slots[1].midi_note()),
        (Some(36), Some(38))
    );
    let lane = &drums.lanes[0];
    assert_eq!((lane.kind, lane.grid), (LaneKind::Drums, DrumGrid::Notes));
    let hits: Vec<_> = (lane.clips[0].events.iter())
        .map(|e| match *e {
            Event::Drum {
                pos,
                pad,
                velocity,
                len_beats,
                ..
            } => (pos, pad, velocity, len_beats),
            other => panic!("not a hit: {other:?}"),
        })
        .collect();
    assert_eq!(
        (lane.clips[0].start_tick, hits),
        (512, vec![(384, 0, 254, 0.25), (416, 1, 200, 0.25)])
    );

    // With the bundle: our DW program for the program, at its own pitch.
    let entry = crate::content::Entry::new("Presets/DW Sampler/Bass/Finger Bass.dwp", Vec::new);
    let factory = crate::content::Factory::new(vec![entry]);
    let mut song = Song::default();
    let imported = import(
        &mut song,
        0,
        &bytes,
        &ImportOpts::OPEN_AS_SONG,
        true,
        &factory,
    )
    .unwrap();
    let dw = &song.channels[1].rack.modules[0];
    assert_eq!(
        song.channels[1].kind,
        ChannelKind::Synth(SynthKind::DwSampler)
    );
    assert_eq!(
        (dw.file.preset_name.as_str(), dw.params[3]),
        ("Finger Bass", 0.5)
    );
    assert_eq!(imported.channels, [1, 2]);
    assert_eq!(
        imported.notices,
        [
            "(MIDI Channel 0: Electric Bass (finger)): Finger Bass",
            "(MIDI Channel 9: Drums): the default kit"
        ]
    );
}

// --- lanes and clips ---

#[test]
fn each_track_starts_a_new_lane() {
    // Split off: each track's first note adds a lane named after it to the current channel; tracks without notes
    // add none, and every MIDI channel of a track shares its lane.
    let bytes = smf(
        96,
        vec![
            vec![name("T1"), on(0, 0, 60, 64), on(0, 1, 62, 64)],
            vec![name("T2"), cc(0, 0, 7, 100), on(96, 0, 64, 64)],
            vec![name("Ctl"), cc(0, 0, 7, 50)],
        ],
    );
    let song = into_keys(&bytes, ImportOpts::ONE_CHANNEL);
    let keys = &song.channels[1];
    assert_eq!(
        labels(keys),
        [
            (LaneKind::Notes, ""),
            (LaneKind::Notes, "T1"),
            (LaneKind::Notes, "T2"),
            (LaneKind::Automation, "CHN #0 : MIDI CC #7"),
        ]
    );
    assert_eq!(notes(&keys.lanes[0]), [(0, 72, 204, 1.0)]);
    assert_eq!(
        notes(&keys.lanes[1]),
        [(0, 60, 128, 1.0), (0, 62, 128, 1.0)]
    );
    assert_eq!(notes(&keys.lanes[2]), [(128, 64, 128, 1.0)]);
    assert_eq!(pts(&keys.lanes[3]).len(), 3);

    // Split on: a channel made in this track takes the track's notes on its own lane; a later track on it adds one.
    let bytes = smf(
        96,
        vec![
            vec![name("A"), cc(0, 0, 1, 10), on(96, 0, 60, 64)],
            vec![name("B"), on(0, 0, 61, 64)],
        ],
    );
    let a = &open(&bytes).channels[1];
    assert_eq!(
        labels(a),
        [
            (LaneKind::Notes, "A"),
            (LaneKind::Notes, "B"),
            (LaneKind::Automation, "Modulation Wheel")
        ]
    );
    assert_eq!(notes(&a.lanes[0]), [(128, 60, 128, 1.0)]);
    assert_eq!(notes(&a.lanes[1]), [(0, 61, 128, 1.0)]);

    // One flag a track: once a new channel's first note took it, a note for an older channel goes to that channel's
    // last notes lane.
    let bytes = smf(
        96,
        vec![
            vec![name("A"), on(0, 0, 60, 64)],
            vec![name("B"), on(0, 1, 40, 64), on(96, 0, 61, 64)],
        ],
    );
    let song = open(&bytes);
    assert_eq!(labels(&song.channels[1]), [(LaneKind::Notes, "A")]);
    assert_eq!(
        notes(&song.channels[1].lanes[0]),
        [(0, 60, 128, 1.0), (128, 61, 128, 1.0)]
    );
    assert_eq!(labels(&song.channels[2]), [(LaneKind::Notes, "B")]);
}

#[test]
fn clips_start_on_four_beats_and_grow_over_their_notes() {
    let clip = |tracks| open(&smf(128, tracks)).channels[1].lanes[0].clips[0].clone();
    // Start floor(5 / 4)·4 = 4; the last note starts 9.5 beats past it and lasts a beat, which the clip covers.
    let c = clip(vec![vec![
        on(5 * 128, 0, 60, 64),
        on(7 * 128, 0, 61, 64),
        on(13 * 128 + 64, 0, 62, 64),
    ]]);
    assert_eq!(c.start_tick, 512);
    assert_eq!(c.visible_len_beats, 10.5);
    assert_eq!(
        c.events.iter().map(Event::pos).collect::<Vec<_>>(),
        [128, 384, 1216]
    );
    // The loop holds the visible part, so nothing repeats inside it.
    assert!(c.loop_ticks().unwrap() >= c.visible_ticks());
    assert!(
        c.events
            .iter()
            .all(|e| f64::from(e.pos()) < c.loop_ticks().unwrap())
    );
    // Under 4 beats past the start the clip stays 4 beats long, but for the notes' ends.
    let c = clip(vec![vec![on(128, 0, 60, 64), on(2 * 128 + 64, 0, 61, 64)]]);
    assert_eq!((c.start_tick, c.visible_len_beats), (0, 4.0));
    let c = clip(vec![vec![on(0, 0, 60, 64), on(4 * 128, 0, 61, 64)]]);
    assert_eq!((c.start_tick, c.visible_len_beats), (0, 5.0));
    assert!(
        c.events
            .iter()
            .all(|e| f64::from(e.pos()) < c.loop_ticks().unwrap())
    );
    // A note-off past the last note start grows it to the note's end.
    let c = clip(vec![vec![on(9 * 128, 0, 60, 64), off(20 * 128, 0, 60)]]);
    assert_eq!((c.start_tick, c.visible_len_beats), (1024, 12.0));
    assert_eq!(c.events, [note(128, 11.0, 60, 128)]);
    // Automation clips grow alike.
    let song = open(&smf(
        128,
        vec![vec![cc(2 * 128, 0, 7, 1), cc(9 * 128, 0, 7, 2)]],
    ));
    let c = &lane(&song.channels[1], "Volume").clips[0];
    assert_eq!((c.start_tick, c.visible_len_beats), (0, 9.0 + 1.0 / 128.0));
    assert!(c.loop_ticks().unwrap() >= c.visible_ticks());
    assert!(
        c.events
            .iter()
            .all(|e| f64::from(e.pos()) < c.loop_ticks().unwrap())
    );
}

#[test]
fn a_grown_loop_stays_b_times_a_power_of_two() {
    // The last point sits exactly on 8 = b·2ᵏ beats: the visible part is a tick longer, so the loop doubles to 16,
    // never just covers it with 8 + 1/128.
    let song = open(&smf(128, vec![vec![cc(0, 0, 7, 1), cc(8 * 128, 0, 7, 2)]]));
    let c = &lane(&song.channels[1], "Volume").clips[0];
    assert_eq!(c.visible_len_beats, 8.0 + 1.0 / 128.0);
    assert_eq!(c.loop_len_beats, 16.0);
}

#[test]
fn the_last_events_of_a_grown_clip_play() {
    // A grown clip ends a tick after its last event start, so a sustain released, a bend reset
    // and a note at the end all play, and the pedal does not stick.
    let song = open(&smf(
        128,
        vec![vec![
            cc(0, 0, 64, 127),
            pb(0, 0, 12000),
            on(0, 0, 60, 64),
            off(128, 0, 60),
            on(8 * 128, 0, 62, 64),
            cc(8 * 128, 0, 64, 0),
            pb(8 * 128, 0, 8192),
        ]],
    ));
    let c = &song.channels[1];
    let last = |lane: &Lane| {
        let clip = &lane.clips[0];
        let plays: Vec<_> = clip.events_in_song_range(0, u64::MAX).collect();
        plays.last().map(|&(tick, e)| (tick, *e))
    };
    let point = |value| Event::Automation {
        pos: 1024,
        value: points::unit(value),
        control: true,
    };
    assert_eq!(last(lane(c, "Sustain Pedal")), Some((1024, point(0))));
    assert_eq!(last(lane(c, "Pitch Wheel")), Some((1024, point(32769))));
    assert_eq!(last(&c.lanes[0]), Some((1024, note(1024, 1.0, 62, 128))));
    // So a re-export keeps the last note and the pedal release.
    let again = codec::read(&smf_of(&song, None).unwrap()).unwrap();
    let voice: Vec<_> = (again.tracks.iter().flatten())
        .filter_map(|(t, e)| match e {
            SmfEvent::Voice(m) => Some((*t, *m)),
            _ => None,
        })
        .collect();
    assert!(voice.contains(&(
        1024,
        NoteOn {
            ch: 0,
            key: 62,
            vel: 64
        }
    )));
    assert!(voice.contains(&(
        1024,
        Cc {
            ch: 0,
            cc: 64,
            value: 0
        }
    )));
}

#[test]
fn note_offs_pair_with_the_latest_note() {
    let song = open(&smf(
        128,
        vec![vec![
            on(0, 0, 60, 64),
            off(64, 0, 60),
            // 1/128 beat is not more than 1/128: it keeps 1 beat.
            on(128, 0, 62, 64),
            off(129, 0, 62),
            on(256, 0, 63, 64),
            off(258, 0, 63),
            // Overlapped: each off finds the later note, and the earlier one keeps 1 beat.
            on(384, 0, 64, 64),
            on(512, 0, 64, 64),
            off(640, 0, 64),
            off(768, 0, 64),
            // An off with no note.
            off(900, 0, 70),
            // `9n` with velocity 0 is an off.
            on(1024, 0, 65, 64),
            on(1100, 0, 65, 0),
        ]],
    ));
    assert_eq!(
        notes(&song.channels[1].lanes[0]),
        [
            (0, 60, 128, 0.5),
            (128, 62, 128, 1.0),
            (256, 63, 128, 2.0 / 128.0),
            (384, 64, 128, 1.0),
            (512, 64, 128, 2.0),
            (1024, 65, 128, 76.0 / 128.0),
        ]
    );
    // Two notes of a key at one beat: the later one is the latest-starting and takes the off.
    let song = open(&smf(
        128,
        vec![vec![on(0, 0, 66, 64), on(0, 0, 66, 100), off(64, 0, 66)]],
    ));
    assert_eq!(
        notes(&song.channels[1].lanes[0]),
        [(0, 66, 128, 1.0), (0, 66, 200, 0.5)]
    );
    // Velocity byte trunc(v·0.007874·255): 2v.
    let song = open(&smf(
        96,
        vec![vec![on(0, 0, 1, 1), on(0, 0, 2, 64), on(0, 0, 3, 127)]],
    ));
    let vels: Vec<u8> = notes(&song.channels[1].lanes[0])
        .iter()
        .map(|n| n.2)
        .collect();
    assert_eq!(vels, [2, 128, 254]);
}

// --- automation ---

/// Every kind of controller on MIDI channel 2, and a pitch wheel.
fn controllers() -> Vec<u8> {
    let mut track = vec![
        name("Ctl"),
        on(0, 2, 60, 64),
        cc(0, 2, 7, 0),
        cc(128, 2, 7, 64),
        cc(256, 2, 7, 127),
        pb(0, 2, 8192),
    ];
    for n in [0, 1, 8, 10, 11, 32, 63, 64, 74, 121, 127] {
        track.push(cc(0, 2, n, 5));
    }
    track.sort_by_key(|e| e.0);
    smf(128, vec![track])
}

#[test]
fn controller_lanes() {
    // Split on: GM names, `Controller %d` for 32–63, `MIDI CC #%d` otherwise; CC 127 is dropped.
    let song = open(&controllers());
    let c = &song.channels[1];
    assert_eq!(
        autos(c),
        [
            (4, "Pitch Wheel"),
            (7, "Bank Select"),
            (8, "Modulation Wheel"),
            (14, "Volume"),
            (15, "Balance"),
            (17, "Pan Position"),
            (18, "Expression"),
            (39, "Controller 1"),
            (70, "Controller 32"),
            (71, "Sustain Pedal"),
            (81, "MIDI CC #74"),
            (128, "All Controllers Off"),
        ]
    );
    // Stepped: before each new point the previous value repeats at its tick. u16 = min(trunc(v·0.007874·65534),
    // 65534).
    assert_eq!(
        pts(lane(c, "Volume")),
        [(0, 0), (128, 0), (128, 33024), (256, 33024), (256, 65533)]
    );
    assert_eq!(pts(lane(c, "Pan Position")), [(0, 2580)]);
    // CC 10 never becomes note pan (`notes` checks pan 127).
    assert_eq!(notes(&c.lanes[0]).len(), 1);

    // Only supported: CC 1, 7, 8, 10 and 64.
    let mut song = keys_song();
    let opts = ImportOpts {
        only_supported: true,
        ..ImportOpts::OPEN_AS_SONG
    };
    import(
        &mut song,
        1,
        &controllers(),
        &opts,
        false,
        crate::content::factory(),
    )
    .unwrap();
    let params: Vec<i32> = autos(&song.channels[2]).iter().map(|a| a.0).collect();
    assert_eq!(params, [4, 8, 14, 15, 17, 71]);

    // Import automation off: no CC lane; the pitch wheel still comes.
    let mut song = keys_song();
    let opts = ImportOpts {
        automation: false,
        ..ImportOpts::OPEN_AS_SONG
    };
    import(
        &mut song,
        1,
        &controllers(),
        &opts,
        false,
        crate::content::factory(),
    )
    .unwrap();
    assert_eq!(autos(&song.channels[2]), [(4, "Pitch Wheel")]);

    // Split off: `CHN #c : MIDI CC #n` and `CHN #c : Pitch Wheel`, on the current channel.
    let song = into_keys(&controllers(), ImportOpts::ONE_CHANNEL);
    let a = autos(&song.channels[1]);
    assert_eq!(a.len(), 12);
    assert!(a.contains(&(4, "CHN #2 : Pitch Wheel")));
    assert!(a.contains(&(14, "CHN #2 : MIDI CC #7")));
    assert!(a.contains(&(39, "CHN #2 : MIDI CC #32")));
    assert!(a.contains(&(7, "CHN #2 : MIDI CC #0")));
}

#[test]
fn controller_lanes_are_reused_by_channel_and_midi_channel() {
    let bytes = smf(
        128,
        vec![
            vec![name("A"), cc(0, 0, 7, 10), cc(0, 1, 7, 20)],
            vec![name("B"), cc(128, 0, 7, 30), cc(128, 1, 7, 40)],
        ],
    );
    // Split off: one lane per MIDI channel on the current channel, shared by the tracks.
    let keys = into_keys(&bytes, ImportOpts::ONE_CHANNEL).channels[1].clone();
    assert_eq!(
        autos(&keys),
        [(14, "CHN #0 : MIDI CC #7"), (14, "CHN #1 : MIDI CC #7")]
    );
    let value = |v: u64| (v * 7874 * 65534 / 1_000_000) as u16;
    assert_eq!(
        pts(&keys.lanes[1]),
        [(0, value(10)), (128, value(10)), (128, value(30))]
    );
    // Split on: one lane on each MIDI channel's channel.
    let song = open(&bytes);
    assert_eq!(autos(&song.channels[1]), [(14, "Volume")]);
    assert_eq!(autos(&song.channels[2]), [(14, "Volume")]);
    assert_eq!(pts(&song.channels[2].lanes[1]).len(), 3);
}

#[test]
fn a_lane_shared_by_tracks_steps_in_time_order() {
    // Track B's point falls between track A's two: the curve is still stepped in time order, with no ramp steps.
    let bytes = smf(
        128,
        vec![
            vec![name("A"), cc(0, 0, 7, 10), cc(256, 0, 7, 30)],
            vec![name("B"), cc(128, 0, 7, 20)],
        ],
    );
    let song = open(&bytes);
    let volume = lane(&song.channels[1], "Volume");
    let value = |v: u64| (v * 7874 * 65534 / 1_000_000) as u16;
    assert_eq!(
        pts(volume),
        [
            (0, value(10)),
            (128, value(10)),
            (128, value(20)),
            (256, value(20)),
            (256, value(30)),
        ]
    );
    assert_eq!(volume.clips[0].events.len(), 5);
}

#[test]
fn pitch_wheel() {
    let bytes = smf(
        128,
        vec![vec![pb(0, 0, 8192), pb(128, 0, 0), pb(256, 0, 16383)]],
    );
    let opts = ImportOpts {
        automation: false,
        only_supported: true,
        ..ImportOpts::OPEN_AS_SONG
    };
    let mut song = Song::default();
    import(&mut song, 0, &bytes, &opts, true, crate::content::factory()).unwrap();
    let c = &song.channels[1];
    assert_eq!(autos(c), [(4, "Pitch Wheel")]);
    // clamp(trunc(v14·0.000061039·65534), 0, 65534): 8192 → 32769, 16383 → 65534.
    assert_eq!(
        pts(lane(c, "Pitch Wheel")),
        [(0, 32769), (128, 32769), (128, 0), (256, 0), (256, 65534)]
    );
    let song = open(&smf(128, vec![vec![pb(0, 0, 12000)]]));
    assert_eq!(pts(lane(&song.channels[1], "Pitch Wheel")), [(0, 48001)]);
    // Channel pressure and poly pressure are ignored, and make no channel.
    let pressure = smf(96, vec![vec![v(0, ChannelPressure { ch: 0, value: 9 })]]);
    assert_eq!(open(&pressure).channels.len(), 1);
}

// --- scan ---

#[test]
fn scan_lists_channels_programs_and_automation() {
    let bytes = smf(
        96,
        vec![
            vec![prog(0, 0), prog(0, 5), prog(0, 0), on(0, 0, 60, 64)],
            vec![on(0, 3, 60, 64), prog(9, 3), prog(9, 4)],
            vec![v(0, ChannelPressure { ch: 15, value: 1 }), off(0, 6, 60)],
            vec![cc(0, 7, 127, 0)],
        ],
    );
    // Used: what makes a channel on import (a note-on, a CC other than 127, a pitch wheel); not program changes,
    // pressure, note-offs or CC 127 alone.
    let s = scan(&bytes).unwrap();
    assert_eq!(s.used, 1 | 1 << 3);
    assert_eq!(s.programs[0], ["Acoustic Grand Piano", "Electric Piano 2"]);
    // No program change: the default program. Programs are collected on every channel; channel 10 is `Drums`.
    assert_eq!(s.programs[3], ["Acoustic Grand Piano"]);
    assert_eq!(s.programs[9], ["Drums"]);
    assert!(s.programs[1].is_empty() && s.programs[15].is_empty());
    assert!(!s.automation);
    let cc1 = scan(&smf(96, vec![vec![cc(0, 4, 1, 1)]])).unwrap();
    assert_eq!((cc1.used, cc1.automation), (1 << 4, true));
    let wheel = scan(&smf(96, vec![vec![pb(0, 4, 1)]])).unwrap();
    assert_eq!((wheel.used, wheel.automation), (1 << 4, true));
    // Notes on channel 0 and a stray `C5`: one channel, so the track menu takes the shortcut, and it imports them.
    let stray = smf(96, vec![vec![on(0, 0, 60, 64), prog(5, 2)]]);
    let s = scan(&stray).unwrap();
    assert_eq!(
        (s.used, s.programs[5].clone()),
        (1, vec!["Electric Grand Piano".to_owned()])
    );
    let keys = into_keys(&stray, ImportOpts::ONE_CHANNEL);
    assert_eq!(notes(&keys.channels[1].lanes[1]), [(0, 60, 128, 1.0)]);
    // Channel pressure alone: no channel, so the track menu does nothing.
    let pressure = smf(96, vec![vec![v(0, ChannelPressure { ch: 2, value: 9 })]]);
    assert_eq!(scan(&pressure).unwrap().used, 0);
    // No channel at all.
    assert_eq!(
        scan(&smf(96, vec![vec![name("x")]])).unwrap(),
        Scan::default()
    );
}

// --- round trip and hostile input ---

#[test]
fn export_import_round_trip() {
    let mut song = Song {
        tempo_bpm: 120.0,
        ..Song::default()
    };
    let mut add = |kind, name: &str, start, events| {
        song.add_channel(kind);
        let c = song.channels.last_mut().unwrap();
        c.name = name.into();
        if events != vec![] {
            c.lanes[0].add_clip(start, 8.0).events = events;
        }
    };
    add(
        ChannelKind::Instrument,
        "Keys",
        512,
        vec![
            note(0, 1.0, 60, 204),
            note(64, 0.5, 64, 254),
            note(640, 2.25, 67, 2),
        ],
    );
    let hit = |pos, pad| Event::Drum {
        pos,
        len_beats: 0.0,
        pad,
        velocity: 100,
        pan: Event::DEFAULT_PAN,
        value: Event::DEFAULT_PITCH,
        repeat: 0,
    };
    add(ChannelKind::Drums, "Kit", 0, vec![hit(0, 0), hit(32, 3)]);
    add(ChannelKind::Instrument, "Empty", 0, vec![]);
    add(
        ChannelKind::Instrument,
        "Bass",
        1024,
        vec![note(0, 4.0, 36, 128)],
    );
    let back = open(&smf_of(&song, None).unwrap());
    assert_eq!(back.tempo_bpm, 120.0);
    assert_eq!(
        names(&back),
        [
            "MASTER",
            "Keys (MIDI Channel 0: Acoustic Grand Piano)",
            "Kit (MIDI Channel 9: Drums)",
            "Bass (MIDI Channel 3: Acoustic Grand Piano)",
        ]
    );
    // Song ticks, keys, velocities (v → 2v → v) and lengths survive; drums come back as hits: a pad without a MIDI note
    // goes out at pad + 36 and comes back on the GM kit's pad for that key.
    assert_eq!(
        notes(&back.channels[1].lanes[0]),
        [
            (512, 60, 204, 1.0),
            (576, 64, 254, 0.5),
            (1152, 67, 2, 2.25)
        ]
    );
    let hits = |c: &Channel| {
        (c.lanes[0].clips[0].events.iter())
            .map(|e| match *e {
                Event::Drum {
                    pos,
                    pad,
                    velocity,
                    len_beats,
                    ..
                } => (pos, pad, velocity, len_beats),
                other => panic!("not a hit: {other:?}"),
            })
            .collect::<Vec<_>>()
    };
    assert_eq!(
        hits(&back.channels[2]),
        [(0, 0, 100, 0.25), (32, 2, 100, 0.25)]
    );
    // The GM kit's pads play their MIDI notes, so its hits come back on the same pads.
    let again = open(&smf_of(&back, None).unwrap());
    assert_eq!(hits(&again.channels[2]), hits(&back.channels[2]));
    assert_eq!(notes(&back.channels[3].lanes[0]), [(1024, 36, 128, 4.0)]);
    // A one-channel file goes straight into the current channel.
    let one = smf_of(&song, Some(4)).unwrap();
    assert_eq!(scan(&one).unwrap().used, 1);
    let keys = into_keys(&one, ImportOpts::ONE_CHANNEL);
    assert_eq!(
        notes(lane(&keys.channels[1], "Bass")),
        [(1024, 36, 128, 4.0)]
    );
}

/// A file with every event kind the import reads.
fn every_kind() -> Vec<u8> {
    smf(
        96,
        vec![
            vec![
                (0, SmfEvent::Tempo(500_000)),
                name("Lead"),
                prog(0, 3),
                on(0, 0, 60, 100),
                cc(10, 0, 7, 90),
                pb(20, 0, 9000),
                off(96, 0, 60),
                on(400, 0, 60, 0),
            ],
            vec![
                name("Kit"),
                on(0, 9, 36, 127),
                cc(0, 9, 64, 127),
                off(48, 9, 36),
            ],
        ],
    )
}

/// Imports `bytes` the three ways; on success, every clip holds its events inside its loop.
fn import_all_ways(bytes: &[u8]) -> bool {
    let scanned = scan(bytes);
    let mut ok = scanned.is_ok();
    let dialog = ImportOpts {
        only_supported: true,
        ..ImportOpts::OPEN_AS_SONG
    };
    for (opts, as_song) in [
        (ImportOpts::OPEN_AS_SONG, true),
        (ImportOpts::ONE_CHANNEL, false),
        (dialog, false),
    ] {
        let mut song = keys_song();
        let before = song.clone();
        let result = import(
            &mut song,
            1,
            bytes,
            &opts,
            as_song,
            crate::content::factory(),
        );
        assert_eq!(result.is_ok(), ok, "scan and import read alike");
        ok &= result.is_ok();
        if result.is_err() {
            assert_eq!(song, before);
        }
        for clip in song
            .channels
            .iter()
            .flat_map(|c| &c.lanes)
            .flat_map(|l| &l.clips)
        {
            let lp = clip.loop_ticks().expect("a loop");
            assert!(clip.events.iter().all(|e| f64::from(e.pos()) < lp));
        }
    }
    ok
}

#[test]
fn truncation_at_every_byte() {
    for seed in [
        every_kind(),
        controllers(),
        smf_of(&keys_song(), None).unwrap(),
    ] {
        for n in 0..=seed.len() {
            import_all_ways(&seed[..n]);
        }
    }
}

#[test]
fn hostile_files_never_panic() {
    let seeds = [every_kind(), controllers()];
    let mut x: u64 = 0x2545_F491_4F6C_DD1D;
    let mut rand = move || {
        x ^= x << 13;
        x ^= x >> 7;
        x ^= x << 17;
        x
    };
    let mut read = 0;
    for i in 0..5_000 {
        let mut bytes = seeds[i % 2].clone();
        for _ in 0..1 + rand() % 3 {
            let at = rand() as usize % bytes.len().max(1);
            match rand() % 4 {
                0 if !bytes.is_empty() => bytes[at] = rand() as u8,
                1 if !bytes.is_empty() => bytes[at] ^= 1 << (rand() % 8),
                2 if !bytes.is_empty() => drop(bytes.remove(at)),
                _ => bytes.insert(at.min(bytes.len()), rand() as u8),
            }
        }
        if i % 8 == 0 {
            bytes.truncate(rand() as usize % (bytes.len() + 1));
        }
        read += usize::from(import_all_ways(&bytes));
    }
    assert!(read > 500, "the fuzz reaches the import ({read})");
    // Far ticks, huge deltas and a division of 1 stay in range.
    let far = smf(
        1,
        vec![vec![
            on(0, 0, 60, 64),
            on(0x0FFF_FFFF, 0, 61, 64),
            cc(0x0FFF_FFFF * 2, 0, 7, 1),
            on(0x0FFF_FFFF * 3, 0, 62, 64),
        ]],
    );
    assert!(import_all_ways(&far));
}
