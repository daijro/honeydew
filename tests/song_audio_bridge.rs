use honeydew::audio_bridge::SongAudioBridge;
use honeydew::song::{AutomationTarget, ChannelKind, Event, Lane, LaneKind, Song};
use honeydew_audio::ProcessContext;
use honeydew_audio::sequencer::{InputNoteEvent, NoteKind};

const CONTEXT: ProcessContext = ProcessContext {
    sample_rate: 128.0,
    tempo_bpm: 60.0,
    start_beat: 0.0,
    playing: true,
};

#[test]
fn rack_instances_and_looped_automation_reach_the_audio_host() {
    let mut song = Song::default();
    song.add_channel(ChannelKind::Instrument);
    let channel = &mut song.channels[1];
    let instance_id = channel.rack.add(20);
    assert_eq!(instance_id, 2);
    let lane = channel.add_automation_lane(AutomationTarget {
        instance_id,
        param: 0,
    });
    let clip = lane.add_clip(0, 1.0);
    clip.visible_len_beats = 2.0;
    clip.events.push(Event::Automation {
        pos: 64,
        value: 0.0,
        control: true,
    });

    let mut bridge = SongAudioBridge::new(&song, CONTEXT.sample_rate).unwrap();
    assert_eq!(
        bridge.audio.effect(1, instance_id).unwrap().parameter(0),
        Some(1.0)
    );
    assert_eq!(
        bridge.collect_changes(&song, CONTEXT, 128).unwrap().len(),
        1
    );
    assert_eq!(
        bridge.collect_changes(&song, CONTEXT, 128).unwrap()[0].frame,
        64
    );
    bridge.seek(&song, 96).unwrap();
    assert_eq!(
        bridge.audio.effect(1, instance_id).unwrap().parameter(0),
        Some(0.0)
    );
    bridge.seek(&song, 0).unwrap();
    assert_eq!(
        bridge.audio.effect(1, instance_id).unwrap().parameter(0),
        Some(1.0)
    );
    let next = bridge
        .collect_changes(
            &song,
            ProcessContext {
                start_beat: 1.0,
                ..CONTEXT
            },
            128,
        )
        .unwrap();
    assert_eq!(next.len(), 1);
    assert_eq!(next[0].frame, 64);
}

#[test]
fn removed_targets_and_nonautomation_lanes_are_ignored() {
    let mut song = Song::default();
    song.add_channel(ChannelKind::Instrument);
    let channel = &mut song.channels[1];
    let instance_id = channel.rack.add(20);
    let lane = channel.add_automation_lane(AutomationTarget {
        instance_id,
        param: 0,
    });
    lane.add_clip(0, 1.0).events.push(Event::Automation {
        pos: 0,
        value: 0.0,
        control: true,
    });
    channel.rack.remove(instance_id);
    let mut notes_lane = Lane::new(LaneKind::Notes);
    notes_lane.target = Some(AutomationTarget {
        instance_id: 0,
        param: 0,
    });
    notes_lane.add_clip(0, 1.0).events.push(Event::Automation {
        pos: 0,
        value: 0.0,
        control: true,
    });
    channel.lanes.push(notes_lane);

    let mut bridge = SongAudioBridge::new(&song, CONTEXT.sample_rate).unwrap();
    assert!(
        bridge
            .collect_changes(&song, CONTEXT, 128)
            .unwrap()
            .is_empty()
    );
    bridge.seek(&song, 64).unwrap();
    assert_eq!(bridge.audio.channel_parameter(1, 0), Some(0.8531));
}

#[test]
fn note_events_include_loop_plays_and_note_offs_across_blocks() {
    let mut song = Song::default();
    song.add_channel(ChannelKind::Instrument);
    let clip = song.channels[1].lanes[0].add_clip(0, 1.0);
    clip.visible_len_beats = 2.0;
    clip.events.push(Event::Note {
        pos: 32,
        len_beats: 0.5,
        key: 60,
        velocity: 204,
        pan: 127,
        slide: false,
    });

    let first = SongAudioBridge::note_events_in_block(&song, 1, 0.0, 1.0).unwrap();
    assert_eq!(
        first,
        [
            InputNoteEvent::on(0.25, 60, 204).with_pan(127.0 / 255.0),
            InputNoteEvent::off(0.75, 60)
        ]
    );
    let second = SongAudioBridge::note_events_in_block(&song, 1, 1.0, 2.0).unwrap();
    assert_eq!(
        second,
        [
            InputNoteEvent::on(1.25, 60, 204).with_pan(127.0 / 255.0),
            InputNoteEvent::off(1.75, 60)
        ]
    );
    assert_eq!(second[1].kind, NoteKind::Off);
}

#[test]
fn fractional_loop_uses_song_clip_positions_without_period_drift() {
    let mut song = Song::default();
    let clip = song.channels[0].lanes[0].add_clip(0, 1.0 / 3.0);
    clip.visible_len_beats = 2.0;
    clip.events.push(Event::Automation {
        pos: 0,
        value: 0.25,
        control: true,
    });
    let bridge = SongAudioBridge::new(&song, CONTEXT.sample_rate).unwrap();
    let frames: Vec<_> = bridge
        .collect_changes(&song, CONTEXT, 128)
        .unwrap()
        .into_iter()
        .map(|change| change.frame)
        .collect();
    assert_eq!(frames, [0, 43, 85]);
    let next = bridge
        .collect_changes(
            &song,
            ProcessContext {
                start_beat: 1.0,
                ..CONTEXT
            },
            128,
        )
        .unwrap();
    assert_eq!(next[0].frame, 0);
}

#[test]
fn note_off_at_the_next_loop_onset_is_sent_first() {
    let mut song = Song::default();
    song.add_channel(ChannelKind::Instrument);
    let clip = song.channels[1].lanes[0].add_clip(0, 1.0);
    clip.visible_len_beats = 2.0;
    clip.events.push(Event::Note {
        pos: 32,
        len_beats: 1.0,
        key: 60,
        velocity: 204,
        pan: 127,
        slide: false,
    });
    let next = SongAudioBridge::note_events_in_block(&song, 1, 1.0, 2.0).unwrap();
    assert_eq!(
        next,
        [
            InputNoteEvent::off(1.25, 60),
            InputNoteEvent::on(1.25, 60, 204).with_pan(127.0 / 255.0)
        ]
    );
}

#[test]
fn control_points_generate_automation_ramp_updates() {
    let mut song = Song::default();
    let clip = song.channels[0].lanes[0].add_clip(0, 8.0);
    clip.events.extend([
        Event::Automation {
            pos: 0,
            value: 0.0,
            control: true,
        },
        Event::Automation {
            pos: 512,
            value: 65534.0 / 65535.0,
            control: true,
        },
    ]);
    let mut bridge = SongAudioBridge::new(&song, CONTEXT.sample_rate).unwrap();
    let changes = bridge.collect_changes(&song, CONTEXT, 512).unwrap();
    assert_eq!(changes.len(), 128);
    assert_eq!(changes[0].frame, 0);
    assert_eq!(changes[1].frame, 4);
    bridge.seek(&song, 256).unwrap();
    let middle = bridge.audio.channel_parameter(0, 0).unwrap();
    assert!((middle - 0.5).abs() < 0.01);
}

#[test]
fn common_rack_automation_keeps_its_negative_target_index() {
    let mut song = Song::default();
    let id = song.channels[0].rack.add(20);
    song.channels[0].rack.get_mut(id).unwrap().params[1] = 0.0;
    let clip = song.channels[0]
        .add_automation_lane(AutomationTarget {
            instance_id: id,
            param: -2,
        })
        .add_clip(0, 1.0);
    clip.events.push(Event::Automation {
        pos: 32,
        value: 0.0,
        control: true,
    });
    let mut bridge = SongAudioBridge::new(&song, CONTEXT.sample_rate).unwrap();
    let changes = bridge.collect_changes(&song, CONTEXT, 128).unwrap();
    assert_eq!(changes.len(), 1);
    assert_eq!(changes[0].target.instance_id, id);
    assert_eq!(changes[0].target.parameter_index, -2);
    bridge.seek(&song, 64).unwrap();
    let mut left = [1.0; 2048];
    let mut right = [1.0; 2048];
    bridge
        .process_block(
            &mut [honeydew_audio::playback::StereoBlock {
                left: &mut left,
                right: &mut right,
            }],
            CONTEXT,
            &[],
        )
        .unwrap();
    assert!(left[2047] > 0.8);
}

#[test]
fn malformed_clip_offset_is_rejected() {
    let mut song = Song::default();
    let clip = song.channels[0].lanes[0].add_clip(0, 1.0);
    clip.offset_beats = f64::NAN;
    clip.events.push(Event::Automation {
        pos: 0,
        value: 0.5,
        control: true,
    });
    assert!(SongAudioBridge::new(&song, CONTEXT.sample_rate).is_err());
    // A loop shorter than one tick is no loop (`Clip::loop_ticks`): the clip plays nothing, and the song builds.
    song.channels[0].lanes[0].clips[0].offset_beats = 0.0;
    song.channels[0].lanes[0].clips[0].loop_len_beats = 1.0 / 256.0;
    let bridge = SongAudioBridge::new(&song, CONTEXT.sample_rate).unwrap();
    assert!(
        bridge
            .collect_changes(&song, CONTEXT, 128)
            .unwrap()
            .is_empty()
    );
}

#[test]
fn event_between_samples_moves_to_the_next_block() {
    let mut song = Song::default();
    song.channels[0].lanes[0]
        .add_clip(0, 1.0)
        .events
        .push(Event::Automation {
            pos: 1,
            value: 0.25,
            control: true,
        });
    let bridge = SongAudioBridge::new(&song, 48_000.0).unwrap();
    let context = ProcessContext {
        sample_rate: 48_000.0,
        tempo_bpm: 120.0,
        start_beat: 0.0,
        playing: true,
    };
    assert!(
        bridge
            .collect_changes(&song, context, 188)
            .unwrap()
            .is_empty()
    );
    let next = bridge
        .collect_changes(
            &song,
            ProcessContext {
                start_beat: 188.0 / 24_000.0,
                ..context
            },
            1,
        )
        .unwrap();
    assert_eq!(next.len(), 1);
    assert_eq!(next[0].frame, 0);
}

#[test]
fn muted_clips_make_no_events() {
    let mut song = Song::default();
    song.add_channel(ChannelKind::Instrument);
    let channel = &mut song.channels[1];
    let notes = channel.lanes[0].add_clip(0, 1.0);
    notes.muted = true;
    notes.events.push(Event::Note {
        pos: 0,
        len_beats: 0.5,
        key: 60,
        velocity: 204,
        pan: 127,
        slide: false,
    });
    let lane = channel.add_automation_lane(AutomationTarget {
        instance_id: 0,
        param: 0,
    });
    let clip = lane.add_clip(0, 1.0);
    clip.muted = true;
    clip.events.push(Event::Automation {
        pos: 0,
        value: 0.0,
        control: true,
    });

    let mut bridge = SongAudioBridge::new(&song, CONTEXT.sample_rate).unwrap();
    assert!(
        bridge
            .collect_changes(&song, CONTEXT, 128)
            .unwrap()
            .is_empty()
    );
    bridge.seek(&song, 64).unwrap();
    assert_eq!(bridge.audio.channel_parameter(1, 0), Some(0.8531));
    assert!(
        SongAudioBridge::note_events_in_block(&song, 1, 0.0, 1.0)
            .unwrap()
            .is_empty()
    );
}

#[test]
fn a_midi_cc_lane_reaches_the_channel_strip() {
    // Channel params 7 + n are MIDI CC #n; the song stores no value for them.
    let mut song = Song::default();
    song.add_channel(ChannelKind::Instrument);
    let lane = song.channels[1].add_automation_lane(AutomationTarget {
        instance_id: 0,
        param: 8,
    });
    lane.add_clip(0, 1.0).events.push(Event::Automation {
        pos: 32,
        value: 1.0,
        control: true,
    });
    let mut bridge = SongAudioBridge::new(&song, CONTEXT.sample_rate).unwrap();
    let changes = bridge.collect_changes(&song, CONTEXT, 128).unwrap();
    assert_eq!(changes.len(), 1);
    assert_eq!(changes[0].target.parameter_index, 8);
    bridge.seek(&song, 64).unwrap();
    assert_eq!(bridge.audio.channel_parameter(1, 8), Some(1.0));
    bridge.seek(&song, 0).unwrap();
    assert_eq!(bridge.audio.channel_parameter(1, 8), Some(0.0));
}

#[test]
fn the_bridge_sizes_tape_stop_for_the_song_tempo() {
    // The ring is sized from the song tempo when the effect is built, off the audio thread.
    // At 60 BPM a 16-beat synced Stop at the slowest warp reads further back than a ring sized for 120 BPM holds, so
    // the bridge's Tape Stop must sound as one sized for 10 BPM.
    const RATE: f32 = 8_000.0;
    let mut song = Song {
        tempo_bpm: 60.0,
        ..Song::default()
    };
    song.add_channel(ChannelKind::Instrument);
    let id = song.channels[1].rack.add(42);
    let module = song.channels[1].rack.get_mut(id).unwrap();
    for (index, value) in [(3, 1.0), (6, 0.0), (7, 1.0)] {
        module.params[index] = value;
    }
    let (params, common) = (module.params.clone(), module.common.0);
    let mut bridge = SongAudioBridge::new(&song, RATE).unwrap();
    let mut reference = honeydew_audio::AudioEffect::new(42, RATE).unwrap();
    reference.configure_tempo(10.0).unwrap();
    for (index, value) in params.into_iter().enumerate() {
        reference.set_parameter(index, value).unwrap();
    }
    for (index, value) in common.into_iter().enumerate() {
        reference
            .set_common_parameter(0x7fc0 + index as u16, value)
            .unwrap();
    }
    let context = ProcessContext {
        sample_rate: RATE,
        ..CONTEXT
    };
    let play = honeydew_audio::automation::AutomationTarget {
        channel: 1,
        instance_id: id,
        parameter_index: 5,
    };
    let mut seed = 1_u32;
    // 17 s of noise, then Stop and 20 s more, in 1000-frame blocks.
    for block in 0..37 * 8 {
        if block == 17 * 8 {
            bridge.audio.set_target(play, 0.0).unwrap();
            reference.set_parameter(5, 0.0).unwrap();
        }
        let left: Vec<f32> = (0..1000)
            .map(|_| {
                seed = seed.wrapping_mul(1_664_525).wrapping_add(1_013_904_223);
                (seed >> 8) as f32 / (1 << 23) as f32 - 1.0
            })
            .collect();
        let (mut bridged, mut alone) = ([left.clone(), left.clone()], [left.clone(), left]);
        let [l, r] = &mut bridged;
        bridge.audio.process_channel(1, l, r, context).unwrap();
        let [l, r] = &mut alone;
        reference.process(l, r, context).unwrap();
        assert!(bridged == alone, "block {block}");
    }
}

/// A song with an effect on channel 1 whose param 0 has one clip: a 2-beat loop shown twice, starting `offset` beats
/// in, ramping 0.2 → 0.9 with its last point on the loop end.
fn loop_end_song(offset: f64) -> (Song, i32) {
    let mut song = Song::default();
    song.add_channel(ChannelKind::Instrument);
    let channel = &mut song.channels[1];
    let instance_id = channel.rack.add(20);
    let clip = channel
        .add_automation_lane(AutomationTarget {
            instance_id,
            param: 0,
        })
        .add_clip(0, 2.0);
    clip.visible_len_beats = 4.0;
    clip.offset_beats = offset;
    clip.events = [(0, 0.2), (256, 0.9)]
        .map(|(pos, value)| Event::Automation {
            pos,
            value,
            control: true,
        })
        .to_vec();
    (song, instance_id)
}

#[test]
fn a_point_on_the_loop_end_dispatches_before_the_next_passs_first_point() {
    // A point on the loop end plays with the next pass's content 0; the lower position lands last and wins, as
    // `honeydew_audio::automation` dispatches a tie, whatever the clip's offset. One tick a frame.
    for offset in [0.0, 1.0] {
        let (song, id) = loop_end_song(offset);
        let mut bridge = SongAudioBridge::new(&song, CONTEXT.sample_rate).unwrap();
        let changes = bridge.collect_changes(&song, CONTEXT, 512).unwrap();
        // Each pass start (song ticks 0, 256 at offset 0; 128, 384 at offset 1): the end value, then the first.
        let starts: &[usize] = if offset == 0.0 {
            &[0, 256]
        } else {
            &[128, 384]
        };
        for &frame in starts {
            let at: Vec<f32> = (changes.iter())
                .filter(|c| c.frame == frame)
                .map(|c| c.value)
                .collect();
            assert_eq!(
                at.last(),
                Some(&0.2),
                "offset {offset}, frame {frame}: {at:?}"
            );
            assert!(at.len() >= 2, "offset {offset}, frame {frame}: {at:?}");
        }
        // A seek onto a pass start takes the first point's value too.
        bridge.seek(&song, starts[1] as u64).unwrap();
        let value = bridge.audio.effect(1, id).unwrap().parameter(0);
        assert_eq!(value, Some(0.2), "offset {offset}");
    }
}

#[test]
fn a_note_of_length_0_negative_or_not_finite_makes_no_events() {
    // A key held at Stop while recording keeps length 0. It plays
    // nothing; a note-on with its note-off on the same beat would sort the off
    // first. A negative or non-finite one (in memory only: a file's reads as 0) is skipped too, not an error.
    let mut song = Song::default();
    song.add_channel(ChannelKind::Instrument);
    let note = |pos, len_beats| Event::Note {
        pos,
        len_beats,
        key: 60,
        velocity: 204,
        pan: 127,
        slide: false,
    };
    let clip = song.channels[1].lanes[0].add_clip(0, 1.0);
    clip.events = vec![note(0, 0.0), note(64, 0.25)];
    let events = SongAudioBridge::note_events_in_block(&song, 1, 0.0, 1.0).unwrap();
    let beats: Vec<_> = events
        .iter()
        .map(|e| (e.beat, e.kind == NoteKind::Off))
        .collect();
    assert_eq!(beats, [(0.5, false), (0.75, true)]);
    for len_beats in [-1.0, f64::NAN, f64::INFINITY, f64::NEG_INFINITY] {
        song.channels[1].lanes[0].clips[0].events[0] = note(0, len_beats);
        let events = SongAudioBridge::note_events_in_block(&song, 1, 0.0, 1.0).unwrap();
        assert_eq!(events.len(), 2, "{len_beats}");
    }
}

/// A note's note-off is clamped to its clip's length, so one longer than its clip
/// (however long) ends at the clip end.
#[test]
fn a_notes_off_is_clamped_to_its_clip_end() {
    let mut song = Song::default();
    song.add_channel(ChannelKind::Instrument);
    for len_beats in [3.0, 1e300] {
        let clip = &mut song.channels[1].lanes[0];
        clip.clips.clear();
        clip.add_clip(128, 1.0).events = vec![Event::Note {
            pos: 0,
            len_beats,
            key: 60,
            velocity: 204,
            pan: 127,
            slide: false,
        }];
        let events = SongAudioBridge::note_events_in_block(&song, 1, 0.0, 4.0).unwrap();
        let beats: Vec<_> = events
            .iter()
            .map(|e| (e.beat, e.kind == NoteKind::Off))
            .collect();
        assert_eq!(beats, [(1.0, false), (2.0, true)], "{len_beats}");
        // A block after the start still finds the note and its clamped off.
        let events = SongAudioBridge::note_events_in_block(&song, 1, 1.5, 2.5).unwrap();
        assert_eq!(events.len(), 1, "{len_beats}");
    }
}
