//! Shared test songs.

use honeydew::song::{AutomationTarget, ChannelKind, Event, SamplerSlot, Song, SynthKind};

/// An automation value the file holds exactly: `EVN2` stores value / 65535, so other values read back as the
/// nearest of these.
pub fn grid(value: u16) -> f32 {
    f32::from(value) / 65535.0
}

/// One of every channel kind, a send with a user, automation of every target kind and clips of every lane kind.
pub fn every_feature_song() -> Song {
    let mut song = Song::default();
    for kind in ChannelKind::ADDABLE {
        song.add_channel(kind);
    }
    for synth in SynthKind::ALL {
        song.add_channel(ChannelKind::Synth(synth));
    }
    song.add_channel(ChannelKind::EffectTrack);
    song.play_loop = Some((128, 1024));
    song.tempo_bpm = 97.5;
    song.key_root = 3;
    song.key_scale = 2;
    song.file.meta.author = "Alex".into();
    song.file.meta.display_title = "Every feature".into();
    song.file.meta.info_url = "https://example.test/song".into();
    song.file.meta.editing_seconds = 42.0;
    let send_id = song.channels[4].id;
    let drums = &mut song.channels[1];
    drums.file.sends.push(honeydew::song::SendState {
        target_channel_id: send_id,
        level: 0.25,
        ..Default::default()
    });
    drums.output_bus = 1;
    drums.zipped = true;
    drums.name = "My Drums".into();
    let kit = drums.drum_kit.as_mut().unwrap();
    kit.slots[1].file.midi_key = 40;
    kit.slots[2].file.path = Some(honeydew::flm::chunk::ContentPath::Shorthand {
        root: honeydew::flm::chunk::ContentRoot::App,
        relative: "Drum Samples/Kick/Kick 01.wav".into(),
    });
    let hits = drums.lanes[0].add_clip(0, 4.0);
    hits.muted = true;
    hits.events.push(Event::Drum {
        pos: 32,
        len_beats: 0.25,
        pad: 1,
        velocity: 100,
        pan: 20,
        value: 0x1000,
        repeat: 3,
    });
    let module = drums.rack.add(9);
    for target in [
        (0, 0),
        (0, 4),
        (module, 1),
        (module, AutomationTarget::POST_GAIN),
        (-2, 0),
    ] {
        let lane = drums.add_automation_lane(AutomationTarget {
            instance_id: target.0,
            param: target.1,
        });
        lane.add_clip(64, 2.0).events.push(Event::Automation {
            pos: 0,
            value: grid(49151),
            control: true,
        });
    }
    let synth = song
        .channels
        .iter_mut()
        .find(|c| c.kind == ChannelKind::Synth(SynthKind::MiniSynth))
        .unwrap();
    let notes = synth.lanes[0].add_clip(512, 8.0);
    notes.offset_beats = 1.0;
    notes.visible_len_beats = 16.0;
    notes.events.push(Event::Note {
        pos: 0,
        len_beats: 1.5,
        key: 64,
        velocity: 90,
        pan: 10,
        slide: true,
    });
    let audio = song
        .channels
        .iter_mut()
        .find(|c| c.kind == ChannelKind::AudioClip)
        .unwrap();
    let clip = audio.lanes[0].add_clip(0, 4.0);
    clip.file.source_len_beats = 3.5;
    clip.file.sample = Some(Box::new(SamplerSlot {
        name: "Voice".into(),
        folder: "My Recordings".into(),
        file: honeydew::song::SlotFileState {
            id: 1,
            ..Default::default()
        },
        ..Default::default()
    }));
    clip.events.push(Event::Envelope {
        pos: 0,
        env_param: 0,
        value: 1.0,
        control: true,
    });
    song
}
