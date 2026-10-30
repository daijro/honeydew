use super::codec::{self, MAX_DELTA, MAX_EVENTS, MAX_TRACKS, Smf, SmfEvent};
use super::export::{NoData, smf_of, smf_with};
use crate::midi::MidiMsg::{self, *};
use crate::song::{AutomationTarget, ChannelKind, Event, LaneKind, Song, points};

// --- helpers ---

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

fn hit(pos: u32, len_beats: f64, pad: u8) -> Event {
    Event::Drum {
        pos,
        len_beats,
        pad,
        velocity: Event::DEFAULT_VELOCITY,
        pan: Event::DEFAULT_PAN,
        value: Event::DEFAULT_PITCH,
        repeat: 0,
    }
}

/// Appends a channel of `kind` named `name`; a non-empty `events` goes into one 4-beat clip at tick 0 on its main
/// lane. Returns its index.
fn add(song: &mut Song, kind: ChannelKind, name: &str, events: Vec<Event>) -> usize {
    song.add_channel(kind);
    let i = song.channels.len() - 1;
    let c = &mut song.channels[i];
    c.name = name.to_owned();
    if !events.is_empty() {
        c.lanes[0].add_clip(0, 4.0).events = events;
    }
    i
}

fn parse(bytes: &[u8]) -> Smf {
    codec::read(bytes).expect("the export reads back")
}

fn voice(track: &[(u64, SmfEvent)]) -> Vec<(u64, MidiMsg)> {
    (track.iter())
        .filter_map(|(t, e)| match e {
            SmfEvent::Voice(m) => Some((*t, *m)),
            _ => None,
        })
        .collect()
}

fn channel_of(msg: MidiMsg) -> u8 {
    match msg {
        NoteOn { ch, .. }
        | NoteOff { ch, .. }
        | Cc { ch, .. }
        | PitchBend { ch, .. }
        | ChannelPressure { ch, .. }
        | Program { ch, .. } => ch,
    }
}

/// A format-1 file with these `MTrk` bodies.
fn file(division: [u8; 2], tracks: &[&[u8]]) -> Vec<u8> {
    let mut f = b"MThd\0\0\0\x06\0\x01".to_vec();
    f.extend((tracks.len() as u16).to_be_bytes());
    f.extend(division);
    for t in tracks {
        f.extend(b"MTrk");
        f.extend((t.len() as u32).to_be_bytes());
        f.extend(*t);
    }
    f
}

/// A song with notes, drums, a loop, and every kind of MIDI automation: the seed for the hostile-byte tests.
fn rich_song() -> Song {
    let mut song = Song::default();
    add(
        &mut song,
        ChannelKind::Instrument,
        "Keys",
        vec![note(0, 1.0, 60, 204), note(64, 0.5, 64, 255)],
    );
    add(
        &mut song,
        ChannelKind::Drums,
        "Kit",
        vec![hit(0, 0.0, 0), hit(32, 0.25, 3)],
    );
    let i = add(
        &mut song,
        ChannelKind::Instrument,
        "Lead",
        vec![note(0, 2.0, 72, 1)],
    );
    for param in [4, 5, 8] {
        let lane = song.channels[i].add_automation_lane(AutomationTarget {
            instance_id: 0,
            param,
        });
        lane.add_clip(0, 1.0).events = vec![
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
        ];
    }
    song
}

// --- codec ---

#[test]
fn codec_round_trip() {
    let smf = Smf {
        division: 96,
        tracks: vec![
            vec![
                (0, SmfEvent::Tempo(500_000)),
                (0, SmfEvent::Name(b"Piano".to_vec())),
                (
                    0,
                    SmfEvent::Voice(NoteOn {
                        ch: 3,
                        key: 60,
                        vel: 100,
                    }),
                ),
                (
                    96,
                    SmfEvent::Voice(NoteOff {
                        ch: 3,
                        key: 60,
                        vel: 0,
                    }),
                ),
                (
                    100,
                    SmfEvent::Voice(Cc {
                        ch: 15,
                        cc: 7,
                        value: 127,
                    }),
                ),
                (200, SmfEvent::Voice(PitchBend { ch: 1, value: 8192 })),
                (300, SmfEvent::Voice(ChannelPressure { ch: 2, value: 64 })),
                (300, SmfEvent::Voice(Program { ch: 2, program: 5 })),
                (
                    MAX_DELTA + 300,
                    SmfEvent::Other {
                        kind: 0x01,
                        data: b"text".to_vec(),
                    },
                ),
            ],
            vec![],
        ],
    };
    assert_eq!(codec::read(&codec::write(&smf)), Ok(smf));
}

#[test]
fn writer_layout() {
    let smf = Smf {
        division: 128,
        tracks: vec![vec![(
            200,
            SmfEvent::Voice(NoteOn {
                ch: 0,
                key: 60,
                vel: 1,
            }),
        )]],
    };
    let mut want = b"MThd\0\0\0\x06\0\x01\0\x01\0\x80MTrk\0\0\0\x09".to_vec();
    // A two-byte delta, the event (no running status), and the end of track at delta 0.
    want.extend([0x81, 0x48, 0x90, 60, 1, 0x00, 0xFF, 0x2F, 0x00]);
    assert_eq!(codec::write(&smf), want);
}

#[test]
fn running_status_sysex_and_poly_pressure() {
    let track: &[u8] = &[
        0x00, 0x90, 0x3C, 0x64, // note-on
        0x10, 0x3E, 0x64, // running status
        0x00, 0xF0, 0x03, 0x01, 0x02, 0xF7, // SysEx: skipped by its length
        0x05, 0x40, 0x00, // running status survives it; 9n with velocity 0 is a note-off
        0x00, 0xF7, 0x01, 0x00, // an escape: skipped too
        0x00, 0xA0, 0x3C, 0x10, // poly pressure: skipped
        0x00, 0xFF, 0x2F, 0x00, // end of track
        0x00, 0x90, 0x3C, 0x64, // after the end: ignored
    ];
    let smf = codec::read(&file([0, 96], &[track])).unwrap();
    assert_eq!(smf.division, 96);
    assert_eq!(
        smf.tracks,
        vec![vec![
            (
                0,
                SmfEvent::Voice(NoteOn {
                    ch: 0,
                    key: 0x3C,
                    vel: 0x64
                })
            ),
            (
                16,
                SmfEvent::Voice(NoteOn {
                    ch: 0,
                    key: 0x3E,
                    vel: 0x64
                })
            ),
            (
                21,
                SmfEvent::Voice(NoteOff {
                    ch: 0,
                    key: 0x40,
                    vel: 0
                })
            ),
        ]]
    );
}

#[test]
fn metas_and_chunks() {
    let track: &[u8] = &[
        0x00, 0xFF, 0x03, 0x02, b'A', b'b', // track name
        0x00, 0xFF, 0x04, 0x01, b'C', // instrument name
        0x83, 0x00, 0xFF, 0x51, 0x03, 0x07, 0xA1, 0x20, // tempo at tick 384
        0x00, 0xFF, 0x51, 0x02, 0x07, 0xA1, // a malformed tempo stays `Other`
        0x00, 0xFF, 0x58, 0x04, 4, 2, 24,
        8, // time signature
           // no end of track: the chunk end ends it
    ];
    let mut bytes = file([0x01, 0x80], &[track, &[0x00, 0xC1, 0x05]]);
    // An unknown chunk between the tracks is skipped.
    let at = 14 + 8 + track.len();
    bytes.splice(at..at, *b"XFIH\0\0\0\x02ab");
    let smf = codec::read(&bytes).unwrap();
    assert_eq!(smf.division, 384);
    assert_eq!(
        smf.tracks,
        vec![
            vec![
                (0, SmfEvent::Name(b"Ab".to_vec())),
                (0, SmfEvent::Name(b"C".to_vec())),
                (384, SmfEvent::Tempo(500_000)),
                (
                    384,
                    SmfEvent::Other {
                        kind: 0x51,
                        data: vec![0x07, 0xA1]
                    }
                ),
                (
                    384,
                    SmfEvent::Other {
                        kind: 0x58,
                        data: vec![4, 2, 24, 8]
                    }
                ),
            ],
            vec![(0, SmfEvent::Voice(Program { ch: 1, program: 5 }))],
        ]
    );
}

#[test]
fn division_below_one_reads_96() {
    for (division, want) in [
        ([0, 0], 96),
        ([0xE7, 0x28], 96),
        ([0xFF, 0xFF], 96),
        ([0, 1], 1),
        ([0x7F, 0xFF], 32767),
    ] {
        assert_eq!(
            codec::read(&file(division, &[])).unwrap().division,
            want,
            "{division:02X?}"
        );
    }
}

#[test]
fn malformed_files_fail() {
    let bad: &[&[u8]] = &[
        b"",
        b"RIFF\0\0\0\x06\0\x01\0\x01\0\x60",
        b"MThd\0\0\0\x05\0\x01\0\x01\0", // header shorter than 6
        b"MThd\0\0\0\x06\0\x01\0\x01\0", // truncated header
    ];
    for bytes in bad {
        assert!(codec::read(bytes).is_err(), "{bytes:02X?}");
    }
    let bad_tracks: &[&[u8]] = &[
        &[0x00, 0x3C, 0x64],                         // a data byte with no running status
        &[0x00, 0x90, 0x3C, 0x90],                   // a status byte as data
        &[0x00, 0xF8],                               // a realtime byte
        &[0x80, 0x80, 0x80, 0x80, 0x00, 0xC0, 0x01], // a five-byte delta
        &[0x00, 0x90, 0x3C],                         // an event past the chunk end
        &[0x00, 0xFF, 0x01, 0x05, b'a'],             // a meta past the chunk end
        &[0x00, 0xF0, 0x81],                         // a SysEx length past the chunk end
    ];
    for track in bad_tracks {
        assert!(
            codec::read(&file([0, 96], &[track])).is_err(),
            "{track:02X?}"
        );
    }
    // A chunk longer than the file.
    let mut cut = file([0, 96], &[&[0x00, 0xFF, 0x2F, 0x00]]);
    cut.pop();
    assert!(codec::read(&cut).is_err());
}

#[test]
fn track_and_event_caps() {
    let empty: &[u8] = &[];
    assert_eq!(
        codec::read(&file([0, 96], &[empty; MAX_TRACKS]))
            .unwrap()
            .tracks
            .len(),
        MAX_TRACKS
    );
    assert!(codec::read(&file([0, 96], &[empty; MAX_TRACKS + 1])).is_err());

    // Running-status notes: 3 bytes an event after the first. The cap counts over all tracks.
    let track = |events: usize| {
        let mut t = vec![0x00, 0x90, 0x3C, 0x64];
        t.extend([0x00, 0x3C, 0x64].repeat(events - 1));
        t
    };
    let half = track(MAX_EVENTS / 2);
    assert!(codec::read(&file([0, 96], &[&half, &half])).is_ok());
    assert!(codec::read(&file([0, 96], &[&half, &track(MAX_EVENTS / 2 + 1)])).is_err());
}

#[test]
fn truncation_at_every_byte() {
    let bytes = smf_of(&rich_song(), None).unwrap();
    let full = parse(&bytes);
    for n in 0..bytes.len() {
        // Only a cut between chunks reads, with fewer tracks.
        if let Ok(smf) = codec::read(&bytes[..n]) {
            assert!(n >= 14 && smf.tracks.len() < full.tracks.len(), "{n}");
        }
    }
}

#[test]
fn fuzz_never_panics_and_rewrites_stably() {
    let seed = smf_of(&rich_song(), None).unwrap();
    let mut x: u64 = 0x9E37_79B9_7F4A_7C15;
    let mut rand = move || {
        x ^= x << 13;
        x ^= x >> 7;
        x ^= x << 17;
        x
    };
    let mut read = 0;
    for i in 0..20_000 {
        let mut bytes = if i % 16 == 0 {
            // Random tracks under a valid header.
            let body: Vec<u8> = (0..rand() % 64).map(|_| rand() as u8).collect();
            file([0, 96], &[&body])
        } else {
            seed.clone()
        };
        for _ in 0..1 + rand() % 8 {
            let at = rand() as usize % bytes.len().max(1);
            match rand() % 4 {
                0 if !bytes.is_empty() => bytes[at] = rand() as u8,
                1 if !bytes.is_empty() => bytes[at] ^= 0x80,
                2 if !bytes.is_empty() => drop(bytes.remove(at)),
                _ => bytes.insert(at.min(bytes.len()), rand() as u8),
            }
        }
        if i % 4 == 0 {
            bytes.truncate(rand() as usize % (bytes.len() + 1));
        }
        let Ok(smf) = codec::read(&bytes) else {
            continue;
        };
        read += 1;
        assert!(smf.division >= 1 && smf.tracks.len() <= MAX_TRACKS);
        assert!(smf.tracks.iter().map(Vec::len).sum::<usize>() <= MAX_EVENTS);
        // The writer's file always reads, and back to the same thing unless a skipped event held a long gap.
        let again = codec::read(&codec::write(&smf)).expect("the writer's file reads");
        let gaps_fit = smf.tracks.iter().all(|t| {
            (t.iter().scan(0, |last, &(tick, _)| {
                Some(tick - std::mem::replace(last, tick))
            }))
            .all(|gap| gap <= MAX_DELTA)
        });
        if gaps_fit {
            assert_eq!(again, smf, "case {i}");
        }
    }
    assert!(
        read > 100,
        "the fuzz reaches the reader's success paths ({read})"
    );
}

// --- export ---

#[test]
fn export_bytes() {
    let mut song = Song {
        tempo_bpm: 120.0,
        ..Song::default()
    };
    add(
        &mut song,
        ChannelKind::Instrument,
        "Piano",
        vec![note(0, 1.0, 60, 204)],
    );
    let track: &[u8] = &[
        0x00, 0xFF, 0x51, 0x03, 0x07, 0xA1, 0x20, // 60000000 / 120
        0x00, 0xFF, 0x04, 0x05, b'P', b'i', b'a', b'n', b'o', // instrument name
        0x00, 0x90, 60, 102, // velocity byte 204 >> 1
        0x81, 0x00, 0x80, 60, 0x00, // one beat later
        0x00, 0xFF, 0x2F, 0x00,
    ];
    let mut want = b"MThd\0\0\0\x06\0\x01\0\x01\0\x80MTrk".to_vec();
    want.extend((track.len() as u32).to_be_bytes());
    want.extend(track);
    assert_eq!(smf_of(&song, None).unwrap(), want);
}

/// `Apply note effects` (#31): a channel's notes are written as its rack's note effects play them. A Transpose an
/// octave up writes each note 12 higher; a Randomizer at Gate 0 % under it leaves nothing to write. Off: the raw notes.
#[test]
fn apply_note_effects_writes_the_notes_the_rack_plays() {
    let keys = |a, b| {
        let mut song = Song::default();
        let notes = vec![note(0, 1.0, a, 204), note(128, 0.5, b, 100)];
        add(&mut song, ChannelKind::Instrument, "Keys", notes);
        song
    };
    let voices =
        |song: &Song, effects| voice(&parse(&smf_with(song, None, effects).unwrap()).tracks[0]);
    let mut song = keys(60, 64);
    let rack = &mut song.channels.last_mut().unwrap().rack;
    let up = rack.add(48);
    rack.get_mut(up).unwrap().params[1] = 5.0 / 8.0;
    let octave_up = voices(&keys(72, 76), None);
    assert_eq!(octave_up.len(), 4);
    assert_eq!(voices(&song, Some(48_000.0)), octave_up);
    let rack = &mut song.channels.last_mut().unwrap().rack;
    let gate = rack.add(49);
    rack.get_mut(gate).unwrap().params[4] = 0.0;
    assert_eq!(smf_with(&song, None, Some(48_000.0)), Err(NoData));
    assert_eq!(voices(&song, None), voices(&keys(60, 64), None));
}

#[test]
fn midi_channel_counter() {
    let mut song = Song::default();
    for n in 0..17 {
        if n == 3 {
            // A send is not counted; an audio channel is, though it writes nothing.
            song.add_channel(ChannelKind::EffectTrack);
            add(&mut song, ChannelKind::AudioClip, "Audio", vec![]);
        } else {
            add(
                &mut song,
                ChannelKind::Instrument,
                &format!("C{n}"),
                vec![note(0, 1.0, 60, 204)],
            );
        }
    }
    let smf = parse(&smf_of(&song, None).unwrap());
    let channels: Vec<u8> = (smf.tracks.iter())
        .map(|t| channel_of(voice(t)[0].1))
        .collect();
    assert_eq!(
        channels,
        [0, 1, 2, 4, 5, 6, 7, 8, 9, 11, 12, 13, 14, 15, 15, 15]
    );
}

#[test]
fn single_channel_and_drums() {
    let mut song = Song {
        tempo_bpm: 140.0,
        ..Song::default()
    };
    add(
        &mut song,
        ChannelKind::Instrument,
        "Keys",
        vec![note(0, 1.0, 60, 204)],
    );
    let kit = add(
        &mut song,
        ChannelKind::Drums,
        "Kit",
        // No length: one step (1/4 beat). Pads past key 127 clamp there.
        vec![hit(0, 0.0, 2), hit(64, 0.5, 0), hit(256, 1.0, 100)],
    );
    let lead = add(
        &mut song,
        ChannelKind::Instrument,
        "Lead",
        vec![note(0, 1.0, 72, 204)],
    );
    let drums = vec![
        (
            0,
            NoteOn {
                ch: 9,
                key: 38,
                vel: 102,
            },
        ),
        (
            32,
            NoteOff {
                ch: 9,
                key: 38,
                vel: 0,
            },
        ),
        (
            64,
            NoteOn {
                ch: 9,
                key: 36,
                vel: 102,
            },
        ),
        (
            128,
            NoteOff {
                ch: 9,
                key: 36,
                vel: 0,
            },
        ),
        (
            256,
            NoteOn {
                ch: 9,
                key: 127,
                vel: 102,
            },
        ),
        (
            384,
            NoteOff {
                ch: 9,
                key: 127,
                vel: 0,
            },
        ),
    ];

    // The whole song: the kit's lane is on 9 though the counter gives the kit 1; the lead takes 2.
    let whole = parse(&smf_of(&song, None).unwrap());
    assert_eq!(whole.tracks.len(), 3);
    assert_eq!(voice(&whole.tracks[1]), drums);
    assert_eq!(channel_of(voice(&whole.tracks[2])[0].1), 2);

    let one = parse(&smf_of(&song, Some(lead)).unwrap());
    assert_eq!(
        one.tracks,
        vec![vec![
            (0, SmfEvent::Tempo(428_571)),
            (0, SmfEvent::Name(b"Lead".to_vec())),
            (
                0,
                SmfEvent::Voice(NoteOn {
                    ch: 0,
                    key: 72,
                    vel: 102
                })
            ),
            (
                128,
                SmfEvent::Voice(NoteOff {
                    ch: 0,
                    key: 72,
                    vel: 0
                })
            ),
        ]]
    );
    let one = parse(&smf_of(&song, Some(kit)).unwrap());
    assert_eq!(one.tracks.len(), 1);
    assert_eq!(voice(&one.tracks[0]), drums);
}

#[test]
fn velocities() {
    let mut song = Song::default();
    add(
        &mut song,
        ChannelKind::Instrument,
        "V",
        vec![
            note(0, 0.5, 60, 204),
            note(128, 0.5, 61, 255),
            note(256, 0.5, 62, 1),
        ],
    );
    let bytes = smf_of(&song, None).unwrap();
    // A velocity byte of 1 is written as a note-on with velocity 0.
    assert!(bytes.windows(3).any(|w| w == [0x90, 62, 0]));
    assert_eq!(
        voice(&parse(&bytes).tracks[0]),
        vec![
            (
                0,
                NoteOn {
                    ch: 0,
                    key: 60,
                    vel: 102
                }
            ),
            (
                64,
                NoteOff {
                    ch: 0,
                    key: 60,
                    vel: 0
                }
            ),
            (
                128,
                NoteOn {
                    ch: 0,
                    key: 61,
                    vel: 127
                }
            ),
            (
                192,
                NoteOff {
                    ch: 0,
                    key: 61,
                    vel: 0
                }
            ),
            (
                256,
                NoteOff {
                    ch: 0,
                    key: 62,
                    vel: 0
                }
            ),
            (
                320,
                NoteOff {
                    ch: 0,
                    key: 62,
                    vel: 0
                }
            ),
        ]
    );
}

#[test]
fn muted_clip_loop_passes_and_same_tick_order() {
    let mut song = Song::default();
    let i = add(&mut song, ChannelKind::Instrument, "Loop", vec![]);
    let clip = song.channels[i].lanes[0].add_clip(256, 1.0);
    clip.visible_len_beats = 2.5;
    clip.offset_beats = 0.5;
    clip.muted = true;
    clip.events = vec![note(0, 0.25, 60, 204), note(96, 1.0, 62, 204)];
    // Content 0 plays 64 ticks into each pass and content 96 at 32; the third pass is cut at 320.
    let on = |key| NoteOn {
        ch: 0,
        key,
        vel: 102,
    };
    let off = |key| NoteOff { ch: 0, key, vel: 0 };
    assert_eq!(
        voice(&parse(&smf_of(&song, None).unwrap()).tracks[0]),
        vec![
            (288, on(62)),
            (320, on(60)),
            (352, off(60)),
            (416, off(62)), // note-offs first at a tick
            (416, on(62)),
            (448, on(60)),
            (480, off(60)),
            (544, off(62)),
            (544, on(62)),
            (672, off(62)),
        ]
    );
}

#[test]
fn zero_length_notes_last_a_tick() {
    let mut song = Song::default();
    add(
        &mut song,
        ChannelKind::Instrument,
        "Z",
        vec![note(10, 0.0, 60, 204), note(20, f64::NAN, 61, 204)],
    );
    assert_eq!(
        voice(&parse(&smf_of(&song, None).unwrap()).tracks[0]),
        vec![
            (
                10,
                NoteOn {
                    ch: 0,
                    key: 60,
                    vel: 102
                }
            ),
            (
                11,
                NoteOff {
                    ch: 0,
                    key: 60,
                    vel: 0
                }
            ),
            (
                20,
                NoteOn {
                    ch: 0,
                    key: 61,
                    vel: 102
                }
            ),
            (
                21,
                NoteOff {
                    ch: 0,
                    key: 61,
                    vel: 0
                }
            ),
        ]
    );
}

#[test]
fn tempo_meta_and_names() {
    // 140 BPM: 60e6/140 µs a beat rounds (428 571.4 → 428 571).
    let mut song = Song {
        tempo_bpm: 140.0,
        ..Song::default()
    };
    let long = "é".repeat(200);
    add(
        &mut song,
        ChannelKind::Instrument,
        "Bass",
        vec![note(0, 1.0, 40, 204)],
    );
    add(
        &mut song,
        ChannelKind::Instrument,
        &long,
        vec![note(0, 1.0, 40, 204)],
    );
    let smf = parse(&smf_of(&song, None).unwrap());
    assert_eq!(smf.division, 128);
    assert_eq!(
        smf.tracks[0][..2],
        [
            (0, SmfEvent::Tempo(428_571)),
            (0, SmfEvent::Name(b"Bass".to_vec()))
        ]
    );
    // Cut to 255 bytes at a character boundary; the length is a 2-byte VLQ.
    assert_eq!(
        smf.tracks[1][0],
        (0, SmfEvent::Name(long.as_bytes()[..254].to_vec()))
    );
    for track in &smf.tracks {
        let metas = track
            .iter()
            .filter(|(_, e)| !matches!(e, SmfEvent::Voice(_)));
        // No other meta: no FF 03, no FF 58, no tempo but the first track's.
        assert_eq!(
            metas.count(),
            if std::ptr::eq(track, &smf.tracks[0]) {
                2
            } else {
                1
            }
        );
    }
}

#[test]
fn automation_values_clone_ruling() {
    let mut song = Song::default();
    let i = add(&mut song, ChannelKind::Instrument, "Synth", vec![]);
    let events: Vec<Event> = ([0, 32767, 65534].into_iter().enumerate())
        .map(|(n, raw)| Event::Automation {
            pos: n as u32 * 32,
            value: points::unit(raw),
            control: n != 1,
        })
        .collect();
    // Level, Pan-like params, CC #127's 134 and module automation are not written.
    for (instance_id, param) in [
        (0, 0),
        (0, 4),
        (0, 5),
        (0, 6),
        (0, 8),
        (0, 133),
        (0, 134),
        (1, 4),
    ] {
        let lane = song.channels[i].add_automation_lane(AutomationTarget { instance_id, param });
        lane.add_clip(0, 1.0).events = events.clone();
    }
    let smf = parse(&smf_of(&song, None).unwrap());
    let tracks: Vec<_> = smf.tracks.iter().map(|t| voice(t)).collect();
    let at = |msgs: [MidiMsg; 3]| vec![(0, msgs[0]), (32, msgs[1]), (64, msgs[2])];
    assert_eq!(
        tracks,
        vec![
            at([0, 8192, 16383].map(|value| PitchBend { ch: 0, value })),
            at([0, 64, 127].map(|value| ChannelPressure { ch: 0, value })),
            at([0, 64, 127].map(|value| Cc {
                ch: 0,
                cc: 1,
                value
            })),
            at([0, 64, 127].map(|value| Cc {
                ch: 0,
                cc: 126,
                value
            })),
        ]
    );
}

#[test]
fn no_data() {
    assert_eq!(smf_of(&Song::default(), None), Err(NoData));
    let mut song = Song::default();
    add(&mut song, ChannelKind::Instrument, "Empty", vec![]);
    // Master Level automation.
    song.channels[0].lanes[0].add_clip(0, 1.0).events = vec![Event::Automation {
        pos: 0,
        value: 0.5,
        control: true,
    }];
    // A send's notes.
    let send = add(&mut song, ChannelKind::EffectTrack, "Send", vec![]);
    let lane = song.channels[send].add_lane(LaneKind::Notes);
    song.channels[send].lanes[lane].add_clip(0, 1.0).events = vec![note(0, 1.0, 60, 204)];
    // An audio lane's envelope.
    add(
        &mut song,
        ChannelKind::AudioClip,
        "Audio",
        vec![Event::Envelope {
            pos: 0,
            env_param: 0,
            value: 0.5,
            control: true,
        }],
    );
    assert_eq!(smf_of(&song, None), Err(NoData));
    assert_eq!(smf_of(&song, Some(0)), Err(NoData));
    assert_eq!(smf_of(&song, Some(send)), Err(NoData));
    assert_eq!(smf_of(&song, Some(99)), Err(NoData));

    // A clip with an event but no loop still writes its lane, with no notes.
    let i = add(
        &mut song,
        ChannelKind::Instrument,
        "NoLoop",
        vec![note(0, 1.0, 60, 204)],
    );
    song.channels[i].lanes[0].clips[0].loop_len_beats = 0.0;
    let smf = parse(&smf_of(&song, None).unwrap());
    assert_eq!(smf.tracks.len(), 1);
    assert!(voice(&smf.tracks[0]).is_empty());
}

#[test]
fn hostile_clips_stay_bounded() {
    let mut song = Song::default();
    let i = add(&mut song, ChannelKind::Instrument, "Huge", vec![]);
    let lane = &mut song.channels[i].lanes[0];
    // A clip at the last tick, and one with a NaN offset: nothing plays.
    let clip = lane.add_clip(u32::MAX, 1.0);
    clip.events = vec![note(0, 1.0, 61, 204)];
    let clip = lane.add_clip(0, 1.0);
    clip.offset_beats = f64::NAN;
    clip.events = vec![note(0, 1.0, 62, 204)];
    // A one-tick loop shown for a trillion beats, its notes as long.
    let clip = lane.add_clip(0, 1.0 / 128.0);
    clip.visible_len_beats = 1e12;
    clip.events = vec![note(0, 1e30, 60, 204)];
    let smf = parse(&smf_of(&song, None).unwrap());
    let events = voice(&smf.tracks[0]);
    assert!(events.len() <= MAX_EVENTS);
    assert!(
        events.iter().all(|&(t, m)| t <= MAX_DELTA
            && matches!(m, NoteOn { key: 60, .. } | NoteOff { key: 60, .. }))
    );
}

#[test]
fn fractional_play_ticks_truncate() {
    // tick = trunc(event tick + clip start beat × 128): an offset of 0.3 beats (38.4 ticks) plays content 0 at 89.6
    // ticks into each 1-beat pass, so the note-on is at 89 and its off at trunc(89.6 + 32) = 121.
    let mut song = Song::default();
    let i = add(&mut song, ChannelKind::Instrument, "Offset", vec![]);
    let clip = song.channels[i].lanes[0].add_clip(0, 1.0);
    clip.offset_beats = 0.3;
    clip.events = vec![note(0, 0.25, 60, 204)];
    assert_eq!(
        voice(&parse(&smf_of(&song, None).unwrap()).tracks[0]),
        vec![
            (
                89,
                NoteOn {
                    ch: 0,
                    key: 60,
                    vel: 102
                }
            ),
            (
                121,
                NoteOff {
                    ch: 0,
                    key: 60,
                    vel: 0
                }
            ),
        ]
    );
}

#[test]
fn lanes_past_max_tracks_are_not_written() {
    let mut song = Song::default();
    let i = add(&mut song, ChannelKind::Instrument, "Many", vec![]);
    let c = &mut song.channels[i];
    for _ in 1..=MAX_TRACKS {
        c.add_lane(LaneKind::Notes);
    }
    for lane in &mut c.lanes {
        lane.add_clip(0, 1.0).events = vec![note(0, 1.0, 60, 204)];
    }
    assert_eq!(c.lanes.len(), MAX_TRACKS + 1);
    let bytes = smf_of(&song, None).unwrap();
    assert_eq!(bytes[10..12], (MAX_TRACKS as u16).to_be_bytes());
    assert_eq!(parse(&bytes).tracks.len(), MAX_TRACKS);
}
