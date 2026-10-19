use super::import::{SampleRef, import, level_of};
use super::*;
use crate::content::factory;
use crate::song::{ChannelKind, DrumKit, Event as SongEvent, LaneKind, Song, SynthKind, TimeSig};

/// An FLP of `events` at `ppq`, with the header's channel count.
pub(crate) fn flp(ppq: u16, events: Vec<Event>) -> Vec<u8> {
    let channels = events.iter().filter(|e| e.id == id::CH_NEW).count() as u16;
    let header = [0u16, channels, ppq]
        .iter()
        .flat_map(|v| v.to_le_bytes())
        .collect();
    let file = FlpFile {
        chunks: vec![
            Chunk {
                tag: Tag::HEADER,
                body: ChunkBody::Header(Header { raw: header }),
            },
            Chunk {
                tag: Tag::DATA,
                body: ChunkBody::Events(events),
            },
        ],
        trailing: Vec::new(),
    };
    file.to_bytes().unwrap()
}

pub(crate) fn ev(id: u8, value: Value) -> Event {
    Event::new(id, value)
}

pub(crate) fn utf16(s: &str) -> Value {
    let mut v: Vec<u8> = s.encode_utf16().flat_map(u16::to_le_bytes).collect();
    v.extend([0, 0]);
    Value::var(v)
}

/// A 24-byte note record.
pub(crate) fn note(pos: u32, channel: u16, len: u32, key: u16, velocity: u8) -> Vec<u8> {
    let mut r = vec![0u8; 24];
    r[0..4].copy_from_slice(&pos.to_le_bytes());
    r[4..6].copy_from_slice(&0x4000u16.to_le_bytes());
    r[6..8].copy_from_slice(&channel.to_le_bytes());
    r[8..12].copy_from_slice(&len.to_le_bytes());
    r[12..14].copy_from_slice(&key.to_le_bytes());
    r[20] = 64;
    r[21] = velocity;
    r
}

/// A 32-byte playlist record (FL 20 and before): pattern `number` at `pos` for `len`, its window as f32 (-1 none).
fn old_item(pos: u32, number: u16, len: u32, window: (f32, f32)) -> Vec<u8> {
    let mut r = vec![0u8; 32];
    r[0..4].copy_from_slice(&pos.to_le_bytes());
    r[4..6].copy_from_slice(&0x5000u16.to_le_bytes());
    r[6..8].copy_from_slice(&(0x5000 + number).to_le_bytes());
    r[8..12].copy_from_slice(&len.to_le_bytes());
    r[12..14].copy_from_slice(&198u16.to_le_bytes());
    r[24..28].copy_from_slice(&window.0.to_le_bytes());
    r[28..32].copy_from_slice(&window.1.to_le_bytes());
    r
}

fn notes(song: &Song, channel: usize) -> Vec<Vec<(u32, u8, f64, u8)>> {
    let lane = &song.channels[channel].lanes[0];
    (lane.clips.iter())
        .map(|c| {
            (c.events.iter())
                .map(|e| match *e {
                    SongEvent::Note {
                        pos,
                        key,
                        len_beats,
                        velocity,
                        ..
                    } => (pos, key, len_beats, velocity),
                    SongEvent::Drum {
                        pos,
                        pad,
                        len_beats,
                        velocity,
                        ..
                    } => (pos, pad, len_beats, velocity),
                    ref other => panic!("{other:?}"),
                })
                .collect()
        })
        .collect()
}

#[test]
fn an_fl_12_project_maps_its_tempo_channels_patterns_playlist_and_effects() {
    // FL 12's encodings: the coarse + fine tempo, the old name event, the level words and the insert byte, UTF-16
    // text, 32-byte playlist records with f32 windows, a mixer whose inserts end at 147.
    let bytes = flp(
        96,
        vec![
            ev(id::VERSION, Value::var(b"12.5.1.165\0".to_vec())),
            ev(id::TEMPO_COARSE, Value::U16(140)),
            ev(id::TEMPO_FINE, Value::U16(500)),
            ev(id::TIME_NUM, Value::U8(3)),
            ev(id::TIME_DEN, Value::U8(4)),
            ev(id::TITLE, utf16("Old Song")),
            ev(id::PAT_NEW, Value::U16(1)),
            ev(
                id::PAT_NOTES,
                Value::var([note(0, 0, 48, 60, 100), note(96, 1, 0, 64, 128)].concat()),
            ),
            ev(id::CH_NEW, Value::U16(0)),
            ev(id::CH_TYPE, Value::U8(0)),
            ev(id::CH_NAME_OLD, utf16("Kick")),
            ev(id::SAMPLE_PATH, utf16("kick.wav")),
            ev(id::CH_VOL_WORD, Value::U16(5000)),
            ev(id::CH_PAN_WORD, Value::U16(6400)),
            ev(id::CH_TARGET_BYTE, Value::U8(1)),
            ev(id::CH_NEW, Value::U16(1)),
            ev(id::CH_TYPE, Value::U8(2)),
            ev(id::PLUGIN_NAME, utf16("Sytrus")),
            ev(id::DISPLAY_NAME, utf16("Pad Strings")),
            ev(id::CH_COLOR, Value::U32(0x0000_00FF)),
            ev(
                id::CH_LEVELS,
                Value::var(
                    [0i32, 12800, 0]
                        .iter()
                        .flat_map(|v| v.to_le_bytes())
                        .collect::<Vec<_>>(),
                ),
            ),
            ev(
                id::PLAYLIST,
                Value::var(
                    [
                        old_item(0, 1, 288, (-1.0, -1.0)),
                        old_item(288, 1, 144, (48.0, 192.0)),
                    ]
                    .concat(),
                ),
            ),
            ev(id::MX_FLAGS, Value::var(vec![0; 12])),
            ev(id::MX_OUTPUT, Value::U32(0)),
            ev(id::MX_FLAGS, Value::var(vec![0; 12])),
            ev(id::PLUGIN_NAME, utf16("Fruity Reeverb 2")),
            ev(id::PLUGIN_STATE, Value::var(Vec::new())),
            ev(id::MX_SLOT, Value::U16(0)),
            ev(id::PLUGIN_NAME, utf16("Gross Beat")),
            ev(id::MX_SLOT, Value::U16(1)),
            ev(id::MX_OUTPUT, Value::U32(0)),
        ],
    );
    assert!(check_alignment(&FlpFile::parse(&bytes).unwrap()).is_empty());
    let imported = import(&bytes, factory()).unwrap();
    let song = &imported.song;
    assert_eq!(imported.version, "12.5.1.165");
    assert_eq!((song.title.as_str(), song.tempo_bpm), ("Old Song", 140.5));
    assert_eq!(
        song.time_sig,
        TimeSig {
            subdivision: 4,
            beats_per_bar: 3
        }
    );
    let names: Vec<_> = song.channels.iter().map(|c| c.name.as_str()).collect();
    assert_eq!(names, ["MASTER", "Kick", "Pad Strings"]);
    // Half FL's default volume is −6 dB; FL's 100% (12800) is +2.1 dB; pan centre.
    assert_eq!(song.channels[1].params.0[..2], [level_of(5000.0), 0.5]);
    assert!(song.channels[1].params.0[0] < 0.8531 && song.channels[2].params.0[0] > 0.8531);
    assert!(song.channels[2].hue.abs() < 1e-6, "red");
    // The pattern's notes on each channel, at the playlist's places: 96 PPQ to 128, a step note a sixteenth, FL
    // velocity ×2. The second clip shows from half a beat in (its f32 window), for 1.5 beats.
    assert_eq!(
        notes(song, 1),
        [vec![(0, 60, 0.5, 200)], vec![(0, 60, 0.5, 200)]]
    );
    assert_eq!(
        notes(song, 2),
        [vec![(128, 64, 0.25, 254)], vec![(128, 64, 0.25, 254)]]
    );
    let clips = &song.channels[1].lanes[0].clips;
    let shape = |c: &crate::song::Clip| {
        (
            c.start_tick,
            c.visible_len_beats,
            c.loop_len_beats,
            c.offset_beats,
        )
    };
    assert_eq!(
        clips.iter().map(shape).collect::<Vec<_>>(),
        [(0, 3.0, 3.0, 0.0), (384, 1.5, 3.0, 0.5)]
    );
    // The Sampler's sample is the caller's to find; Sytrus plays our closest sound by its channel's name.
    assert_eq!(
        imported.samples,
        [SampleRef {
            channel: 1,
            path: "kick.wav".into(),
            root: 60,
            audio: false,
            zone: None
        }]
    );
    assert_eq!(
        song.channels[2].kind,
        ChannelKind::Synth(SynthKind::SuperSaw)
    );
    // Insert 1's reverb comes along at its defaults; Gross Beat has no counterpart.
    assert!(
        song.channels[1]
            .rack
            .modules
            .iter()
            .any(|m| m.type_id == 35)
    );
    assert_eq!(
        imported.notices,
        [
            "Pad Strings: Sytrus replaced by Warm Blanket",
            "Kick: Fruity Reeverb 2 as Reverb 2",
            "Kick: Gross Beat dropped"
        ]
    );
}

#[test]
fn an_exported_song_passes_the_alignment_checks_and_reads_back() {
    let mut song = Song {
        tempo_bpm: 128.0,
        ..Song::default()
    };
    song.add_channel(ChannelKind::Synth(SynthKind::ThreeXOsc));
    song.channels[1].name = "Lead".into();
    let osc = &mut song.channels[1].rack.modules[0].params;
    for (k, v) in [(1, 0.5), (13, 0.75), (19, 0.25), (22, 0.5), (111, 0.5)] {
        osc[k] = v;
    }
    let lead = song.channels[1].lanes[0].add_clip(512, 4.0);
    lead.visible_len_beats = 8.0;
    lead.events = vec![
        SongEvent::Note {
            pos: 0,
            len_beats: 1.0,
            key: 60,
            velocity: 204,
            pan: 127,
            slide: false,
        },
        SongEvent::Note {
            pos: 256,
            len_beats: 0.5,
            key: 67,
            velocity: 100,
            pan: 127,
            slide: true,
        },
    ];
    song.add_channel(ChannelKind::Drums);
    song.channels[2].name = "Beat".into();
    song.channels[2].drum_kit = Some(DrumKit::default_kit());
    let hit = |pos, pad| SongEvent::Drum {
        pos,
        len_beats: 0.25,
        pad,
        velocity: 200,
        pan: 127,
        value: 0x7FFF,
        repeat: 0,
    };
    song.channels[2].lanes[0].add_clip(0, 4.0).events = vec![hit(0, 0), hit(128, 1)];
    song.add_channel(ChannelKind::Synth(SynthKind::MiniSynth));
    song.channels[3].name = "Silent".into();
    song.channels[3].lanes[0].add_clip(0, 4.0).events = vec![SongEvent::Note {
        pos: 0,
        len_beats: 1.0,
        key: 48,
        velocity: 204,
        pan: 127,
        slide: false,
    }];
    let mut cache = crate::sample_cache::SampleCache::default();
    cache.load(&song, &|_| None);
    let out = export::export(&song, &cache, &[]);
    assert!(check_alignment(&FlpFile::parse(&out.flp).unwrap()).is_empty());
    assert_eq!(
        out.samples
            .iter()
            .map(|(n, _)| n.as_str())
            .collect::<Vec<_>>(),
        ["Beat Kick.wav", "Beat Snare.wav", "Silent.wav"]
    );
    // MiniSynth has none: a Sampler of one note of it.
    assert_eq!(
        out.notices,
        ["Silent: MiniSynth has no .flp counterpart, exported as a sample of one note"]
    );

    let back = import(&out.flp, factory()).unwrap();
    let names: Vec<_> = back.song.channels.iter().map(|c| c.name.as_str()).collect();
    assert_eq!(
        names,
        ["MASTER", "Lead", "Beat Kick", "Beat Snare", "Silent"]
    );
    assert_eq!(back.song.tempo_bpm, 128.0);
    // 3x Osc's oscillators and filter come back through the `.fst` import.
    let lead = &back.song.channels[1];
    assert_eq!(lead.kind, ChannelKind::Synth(SynthKind::ThreeXOsc));
    for k in [1, 13, 19, 22, 111] {
        assert!(
            (lead.rack.modules[0].params[k] - song.channels[1].rack.modules[0].params[k]).abs()
                < 0.01,
            "{k}"
        );
    }
    // The loop is unrolled over the visible length; keys, lengths, velocities and the slide survive.
    let clip = &lead.lanes[0].clips[0];
    assert_eq!((clip.start_tick, clip.visible_len_beats), (512, 8.0));
    assert_eq!(
        notes(&back.song, 1),
        [vec![
            (0, 60, 1.0, 204),
            (256, 67, 0.5, 100),
            (512, 60, 1.0, 204),
            (768, 67, 0.5, 100)
        ]]
    );
    assert!(matches!(
        clip.events[1],
        SongEvent::Note { slide: true, .. }
    ));
    // Each pad plays from its own Sampler at C5.
    assert_eq!(notes(&back.song, 2), [vec![(0, 60, 0.25, 200)]]);
    assert_eq!(notes(&back.song, 3), [vec![(128, 60, 0.25, 200)]]);
    let paths: Vec<_> = back.samples.iter().map(|s| s.path.as_str()).collect();
    assert_eq!(paths, ["Beat Kick.wav", "Beat Snare.wav", "Silent.wav"]);
    assert_eq!(back.song.channels[4].lanes[0].kind, LaneKind::Notes);
}
