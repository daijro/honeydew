use super::*;

fn names(song: &Song) -> Vec<&str> {
    song.channels.iter().map(|c| c.name.as_str()).collect()
}

#[test]
fn a_new_song_holds_only_the_master() {
    let song = Song::default();
    assert_eq!(names(&song), ["MASTER"]);
    assert_eq!(song.master().kind, ChannelKind::Master);
    assert_eq!(song.master().hue, 0.6);
    // HEAD@264, the new-song default: the startup song. The Song options dialog's own default is 140.
    assert_eq!(song.tempo_bpm, 120.0);
    assert_eq!(
        song.time_sig,
        TimeSig {
            subdivision: 4,
            beats_per_bar: 4
        }
    );
}

#[test]
fn time_sig_steps_and_bars_count_0_as_1() {
    let sig = |subdivision, beats_per_bar| TimeSig {
        subdivision,
        beats_per_bar,
    };
    assert_eq!((sig(3, 7).steps_per_beat(), sig(3, 7).bar_beats()), (3, 7));
    assert_eq!((sig(0, 0).steps_per_beat(), sig(0, 0).bar_beats()), (1, 1));
    // b = TDIV byte 1 whatever the subdivision, never num·4/den.
    for (d, b) in [(4, 4), (4, 3), (2, 7), (8, 6), (4, 0)] {
        assert_eq!(sig(d, b).bar_beats(), u32::from(b.max(1)), "{d}/{b}");
    }
    assert_eq!(sig(4, 4).step_ticks(), 32.0);
    assert!((sig(3, 4).step_ticks() - 128.0 / 3.0).abs() < 1e-12);
    // Steps start at round(i·128/d).
    let starts = |d| (0..7).map(|i| sig(d, 4).step_start(i)).collect::<Vec<_>>();
    assert_eq!(starts(4), [0, 32, 64, 96, 128, 160, 192]);
    assert_eq!(starts(3), [0, 43, 85, 128, 171, 213, 256]);
    assert_eq!(starts(6), [0, 21, 43, 64, 85, 107, 128]);
    // Every tick lies in the last step starting at or before it, for every d up to a step a tick.
    for d in 1..=128u8 {
        let s = sig(d, 4);
        for i in 0..600 {
            let (a, b) = (s.step_start(i), s.step_start(i + 1));
            assert_eq!(s.step_of(a), i, "d {d} step {i}");
            if b > a {
                assert_eq!(s.step_of(b - 1), i, "d {d} step {i}");
            }
        }
    }
}

#[test]
fn repeated_names_get_the_next_number() {
    let mut song = Song::default();
    for _ in 0..3 {
        song.add_channel(ChannelKind::Drums);
    }
    assert_eq!(names(&song), ["MASTER", "Drums", "Drums 2", "Drums 3"]);
}

#[test]
fn the_smallest_free_number_is_used() {
    let mut song = Song::default();
    song.channels
        .push(Channel::new("Drums 2".to_owned(), ChannelKind::Drums, 0.0));
    song.add_channel(ChannelKind::Drums);
    song.add_channel(ChannelKind::Drums);
    assert_eq!(names(&song), ["MASTER", "Drums 2", "Drums", "Drums 3"]);
}

#[test]
fn an_audio_record_channel_is_named_record_and_its_channel_id() {
    // `Record <ID>`, the ID being CHHD@1080, which is never reused.
    let mut song = Song::default();
    song.add_channel(ChannelKind::Drums);
    song.delete_channel(1);
    song.add_channel(ChannelKind::AudioRecord);
    song.add_channel(ChannelKind::AudioRecord);
    assert_eq!(names(&song), ["MASTER", "Record 2", "Record 3"]);
    assert_eq!(song.channels[1].id, 2);
}

/// Every kind is named by its menu label, except that `Audio Record` is `Record <ID>`
/// ([`an_audio_record_channel_is_named_record_and_its_channel_id`]).
#[test]
fn names_are_the_menu_labels() {
    let mut song = Song::default();
    for kind in ChannelKind::ADDABLE {
        song.add_channel(kind);
    }
    for synth in SynthKind::ALL {
        song.add_channel(ChannelKind::Synth(synth));
    }
    assert_eq!(
        names(&song)[1..],
        [
            "Drums",
            "Instrument",
            "Audio Clip",
            // A send is `Effect <channel ID>`, not its menu label.
            "Effect 4",
            "Record 5",
            "Rack",
            "MiniSynth",
            "DW Sampler",
            "GMSynth",
            "Transistor Bass",
            "SuperSaw",
            "Slicer",
            "3x Osc",
            "SoundFont Player",
        ]
    );
}

#[test]
fn hue_steps_by_the_golden_ratio_from_the_master_hue() {
    let mut song = Song::default();
    for _ in 0..3 {
        song.add_channel(ChannelKind::Instrument);
    }
    // fract(0.6 + n * 0.618034) for n = 1, 2, 3.
    for (channel, want) in song.channels[1..]
        .iter()
        .zip([0.218034, 0.836068, 0.454102])
    {
        assert!(
            (channel.hue - want).abs() < 1e-6,
            "{} vs {want}",
            channel.hue
        );
    }
}

#[test]
fn channel_params_default_to_rprm_and_name_level_pan_and_pitch_bend() {
    let mut p = ChannelParams::default();
    assert_eq!(p.0, [0.8531, 0.5, 0.0, 0.0, 0.5, 0.0]);
    assert_eq!((p.level(), p.pan(), p.pitch_bend()), (0.8531, 0.5, 0.5));
    p.0 = [0.1, 0.2, 0.3, 0.4, 0.6, 0.7];
    assert_eq!((p.level(), p.pan(), p.pitch_bend()), (0.1, 0.2, 0.6));
    assert_eq!(Channel::master().params, ChannelParams::default());
}

#[test]
fn instance_ids_start_at_2_and_are_never_reused() {
    let mut rack = Rack::default();
    assert_eq!(rack.last_instance_id, 1);
    assert_eq!(rack.add(6), 2);
    assert_eq!(rack.add(11), 3);
    assert_eq!(rack.remove(3).map(|m| m.type_id), Some(11));
    assert_eq!(rack.last_instance_id, 3);
    assert_eq!(rack.insert(0, 9), 4);
    assert_eq!(rack.add(13), 5);
    assert_eq!(rack.last_instance_id, 5);
    let ids: Vec<i32> = rack.modules.iter().map(|m| m.instance_id).collect();
    assert_eq!(ids, [4, 2, 5]);
    assert_eq!(rack.get(2).map(|m| m.type_id), Some(6));
    assert_eq!(rack.get(3), None);
    assert_eq!(rack.remove(3), None);
}

#[test]
fn modules_move_within_the_chain() {
    let mut rack = Rack::default();
    for type_id in [6, 9, 11] {
        rack.add(type_id);
    }
    let order = |rack: &Rack| {
        rack.modules
            .iter()
            .map(|m| m.instance_id)
            .collect::<Vec<_>>()
    };
    assert!(rack.move_up(3));
    assert_eq!(order(&rack), [3, 2, 4]);
    assert!(!rack.move_up(3), "already first");
    assert!(rack.move_down(2));
    assert_eq!(order(&rack), [3, 4, 2]);
    assert!(!rack.move_down(2), "already last");
    assert!(!rack.move_up(99));
}

#[test]
fn module_params_start_at_the_registry_defaults() {
    let m = ModuleInstance::new(6, 2); // Distortion
    assert_eq!(m.params.len(), 7);
    assert_eq!(m.params[1], 0.25);
    assert_eq!(m.common, CommonParams::default());
    assert_eq!(
        (m.common.post_gain(), m.common.mix(), m.common.wet_pan()),
        (0.5, 1.0, 0.5)
    );
    assert!(!m.minimized);
    // Instruments take their registry defaults too; unregistered ids start empty.
    assert_eq!(ModuleInstance::new(0, 2).params.len(), 44); // MiniSynth
    assert!(ModuleInstance::new(17, 2).params.is_empty());
}

#[test]
fn lane_0_follows_the_channel_kind() {
    let mut song = Song::default();
    for kind in [
        ChannelKind::EffectTrack,
        ChannelKind::Instrument,
        ChannelKind::Drums,
        ChannelKind::AudioClip,
        ChannelKind::AudioRecord,
        ChannelKind::Synth(SynthKind::SuperSaw),
    ] {
        song.add_channel(kind);
    }
    let level = Some(AutomationTarget {
        instance_id: 0,
        param: 0,
    });
    let lane0: Vec<(LaneKind, Option<AutomationTarget>)> = song
        .channels
        .iter()
        .map(|c| (c.lanes[0].kind, c.lanes[0].target))
        .collect();
    assert_eq!(
        lane0,
        [
            (LaneKind::Automation, level),
            (LaneKind::Automation, level),
            (LaneKind::Notes, None),
            (LaneKind::Drums, None),
            (LaneKind::Audio, None),
            (LaneKind::Audio, None),
            (LaneKind::Notes, None),
        ]
    );
    assert!(song.channels.iter().all(|c| c.lanes.len() == 1));
    // DESc @28: a channel param's lane is named by the param alone.
    assert_eq!(song.master().lanes[0].label, "Level");
}

#[test]
fn sends_get_unique_bus_ids_and_everything_routes_to_the_master() {
    let mut song = Song::default();
    song.add_channel(ChannelKind::EffectTrack);
    song.add_channel(ChannelKind::Drums);
    song.add_channel(ChannelKind::EffectTrack);
    let buses: Vec<(u32, i32, i32)> = song
        .channels
        .iter()
        .map(|c| (c.kind.file_kind(), c.bus_id, c.output_bus))
        .collect();
    assert_eq!(
        buses,
        [
            (0x000, -1, -1),
            (0x080, 1, -1),
            (0x100, -1, -1),
            (0x080, 2, -1)
        ]
    );
}

fn clip(start_tick: u32, visible: f64, looped: f64, offset: f64) -> Clip {
    Clip {
        id: 1,
        start_tick,
        visible_len_beats: visible,
        loop_len_beats: looped,
        offset_beats: offset,
        events: vec![Event::Automation {
            pos: 0,
            value: 0.5,
            control: true,
        }],
        muted: false,
        file: ClipFileState::default(),
    }
}

#[test]
fn song_tick_wraps_the_content_inside_the_visible_length() {
    let c = clip(512, 8.0, 4.0, 0.0);
    assert_eq!(c.song_tick(0), Some(512));
    assert_eq!(c.song_tick(100), Some(612));
    let ticks: Vec<u64> = c.events_in_song_range(0, 10_000).map(|(t, _)| t).collect();
    assert_eq!(ticks, [512, 1024]);
    // Offset 1 beat: content pos 0 lands 3 beats into the clip.
    assert_eq!(clip(512, 8.0, 4.0, 1.0).song_tick(0), Some(512 + 384));
    // Visible 2 beats: content pos 0 at offset 1 beat lies beyond the visible end.
    assert_eq!(clip(512, 2.0, 4.0, 1.0).song_tick(0), None);
    // Visible 0 means the loop length.
    assert_eq!(clip(512, 0.0, 4.0, 0.0).song_tick(511), Some(1023));
    assert_eq!(
        clip(512, 4.0, 0.0, 0.0).song_tick(0),
        None,
        "no loop length"
    );
}

#[test]
fn events_in_song_range_iterate_repetitions_in_time_order() {
    let mut c = clip(512, 11.0, 4.0, 1.0);
    c.events = [0, 128, 384]
        .map(|pos| Event::Note {
            pos,
            len_beats: 0.5,
            key: 60,
            velocity: Event::DEFAULT_VELOCITY,
            pan: Event::DEFAULT_PAN,
            slide: false,
        })
        .to_vec();
    // Content pos p plays at 512 + (p - 128) mod 512, repeated every 512 ticks for 1408 ticks.
    let ticks = |start, end| -> Vec<(u64, u32)> {
        c.events_in_song_range(start, end)
            .map(|(t, e)| (t, e.pos()))
            .collect()
    };
    assert_eq!(
        ticks(0, u64::MAX),
        [
            (512, 128),
            (768, 384),
            (896, 0),
            (1024, 128),
            (1280, 384),
            (1408, 0),
            (1536, 128),
            (1792, 384) // 1920 (pos 0) is at the visible end
        ]
    );
    assert_eq!(ticks(1024, 1408), [(1024, 128), (1280, 384)]);
    assert_eq!(ticks(0, 512), []);
}

#[test]
fn a_play_rounded_onto_the_next_repetition_is_heard_at_block_boundaries() {
    // Content pos 38 at offset 38.4 ticks plays at 511.6 + k·512, rounded to the next repetition's first tick.
    let mut c = clip(0, 16.0, 4.0, 0.3);
    c.events[0] = Event::Automation {
        pos: 38,
        value: 0.5,
        control: true,
    };
    let ticks =
        |start, end| -> Vec<u64> { c.events_in_song_range(start, end).map(|(t, _)| t).collect() };
    assert_eq!(ticks(0, 4096), [512, 1024, 1536, 2048]);
    for block in [1, 7, 100, 333, 512, 1000] {
        let blocks: Vec<u64> = (0..4096)
            .step_by(block)
            .flat_map(|start| ticks(start, (start + block as u64).min(4096)))
            .collect();
        assert_eq!(blocks, [512, 1024, 1536, 2048], "blocks of {block}");
    }
}

#[test]
fn clip_windows_wrap_offsets_past_the_loop_and_stop_at_the_visible_end() {
    let ticks = |c: &Clip| -> Vec<u64> {
        c.events_in_song_range(0, u64::MAX)
            .map(|(t, _)| t)
            .collect()
    };
    // An offset of a loop and a beat plays like an offset of one beat: content pos 0 lands 3 beats in.
    assert_eq!(
        ticks(&clip(0, 8.0, 4.0, 5.0)),
        ticks(&clip(0, 8.0, 4.0, 1.0))
    );
    assert_eq!(ticks(&clip(0, 8.0, 4.0, 5.0)), [384, 896]);
    // Visible shorter than the loop: one play at most, and none past the end.
    assert_eq!(ticks(&clip(0, 2.0, 4.0, 0.0)), [0]);
    assert!(ticks(&clip(0, 2.0, 4.0, 1.0)).is_empty());
    // No loop length: nothing plays.
    assert!(ticks(&clip(0, 4.0, 0.0, 0.0)).is_empty());
}

#[test]
fn every_kind_but_master_and_send_is_an_instrument_channel_in_files() {
    use SynthKind::*;
    for kind in [
        ChannelKind::Drums,
        ChannelKind::Instrument,
        ChannelKind::AudioClip,
        ChannelKind::AudioRecord,
        ChannelKind::Rack,
        ChannelKind::Synth(MiniSynth),
        ChannelKind::Synth(SoundFontPlayer),
    ] {
        assert_eq!(kind.file_kind(), 0x100, "{kind:?}");
    }
    assert_eq!(ChannelKind::Master.file_kind(), 0x000);
    assert_eq!(ChannelKind::EffectTrack.file_kind(), 0x080);
}

#[test]
fn lanes_allocate_clip_ids_from_1() {
    let mut lane = Lane::new(LaneKind::Notes);
    assert_eq!(lane.add_clip(0, 4.0).id, 1);
    assert_eq!(lane.add_clip(512, 4.0).id, 2);
    assert_eq!(lane.last_clip_id, 2);
}

fn song_with_distortion() -> (Song, i32) {
    let mut song = Song::default();
    song.add_channel(ChannelKind::Instrument);
    let id = song.channel_mut(1).unwrap().rack.add(6);
    (song, id)
}

#[test]
fn automation_labels_read_param_colon_module() {
    let (mut song, id) = song_with_distortion();
    let channel = song.channel_mut(1).unwrap();
    let lane = channel.add_automation_lane(AutomationTarget {
        instance_id: id,
        param: 1,
    });
    assert_eq!(lane.kind, LaneKind::Automation);
    assert_eq!(lane.label, "Drive : Distortion");
    let pitch = AutomationTarget {
        instance_id: 0,
        param: 4,
    };
    assert_eq!(
        channel.automation_label(pitch).as_deref(),
        Some("Pitch Bend")
    );
    // So a renamed channel leaves its lanes' names right.
    channel.name = "Lead".to_owned();
    assert_eq!(
        channel.automation_label(pitch).as_deref(),
        Some("Pitch Bend")
    );
    let missing = AutomationTarget {
        instance_id: 99,
        param: 1,
    };
    assert_eq!(channel.automation_label(missing), None);
    assert_eq!(channel.add_automation_lane(missing).label, "");
    assert_eq!(channel.lanes.len(), 3);
}

#[test]
fn negative_params_target_the_module_common_params() {
    let (mut song, id) = song_with_distortion();
    let mix = AutomationTarget {
        instance_id: id,
        param: AutomationTarget::MIX,
    };
    let channel = song.channel_mut(1).unwrap();
    assert_eq!(
        channel.automation_label(mix).as_deref(),
        Some("Mix : Distortion")
    );
    assert_eq!(channel.param(mix), Some(1.0));
    *channel.param_mut(mix).unwrap() = 0.25;
    let module = song.find_module(1, id).unwrap();
    assert_eq!(module.common.mix(), 0.25);
    for (param, label, value) in [
        (AutomationTarget::POST_GAIN, "Post Gain : Distortion", 0.5),
        (AutomationTarget::WET_PAN, "Wet Pan : Distortion", 0.5),
    ] {
        let target = AutomationTarget {
            instance_id: id,
            param,
        };
        assert_eq!(
            song.channels[1].automation_label(target).as_deref(),
            Some(label)
        );
        assert_eq!(song.channels[1].param(target), Some(value));
    }
    let past = AutomationTarget {
        instance_id: id,
        param: -4,
    };
    assert_eq!(song.channels[1].param(past), None);
}

#[test]
fn targets_resolve_channel_and_module_params() {
    let (mut song, id) = song_with_distortion();
    let channel = song.channel_mut(1).unwrap();
    let level = AutomationTarget {
        instance_id: 0,
        param: 0,
    };
    *channel.param_mut(level).unwrap() = 0.3;
    assert_eq!(channel.params.level(), 0.3);
    let drive = AutomationTarget {
        instance_id: id,
        param: 1,
    };
    assert_eq!(channel.param(drive), Some(0.25));
    *channel.param_mut(drive).unwrap() = 0.75;
    assert_eq!(song.find_module(1, id).unwrap().params[1], 0.75);
    assert_eq!(
        song.channels[1].param(AutomationTarget {
            instance_id: 0,
            param: 6,
        }),
        None
    );
    assert_eq!(song.find_module(1, 99), None);
    assert_eq!(song.find_module(9, id), None);
}

#[test]
fn only_a_drums_channel_gets_a_kit_and_it_starts_with_the_default_kit() {
    let mut song = Song::default();
    for kind in ChannelKind::ADDABLE {
        song.add_channel(kind);
    }
    for channel in &song.channels {
        let expected = (channel.kind == ChannelKind::Drums).then(DrumKit::default_kit);
        assert_eq!(channel.drum_kit, expected, "{}", channel.name);
    }
    // `Default` stays the empty kit, the one a file reader fills.
    assert!(DrumKit::default().slots.is_empty());
}

/// The default kit's pads, in pad order.
const DEFAULT_PADS: [&str; 8] = [
    "Kick",
    "Snare",
    "Clap",
    "Closed Hat",
    "Open Hat",
    "Tom",
    "Perc",
    "Crash",
];

#[test]
fn the_default_kit_has_named_pads_on_built_in_one_shots() {
    // 4–14 pads, as a `.tmpl` kit has.
    let kit = DrumKit::default_kit();
    assert!((4..=14).contains(&kit.slots.len()));
    let pads: Vec<&str> = kit.slots.iter().map(SamplerSlot::pad_name).collect();
    assert_eq!(pads, DEFAULT_PADS);
    for (slot, pad) in kit.slots.iter().zip(DEFAULT_PADS) {
        assert_eq!(slot.folder, format!("Built-in/{pad}"));
        assert_eq!(slot.name, format!("Synth {pad}"));
    }
    // Open and closed hats choke each other (MAIN@1049); nothing else does.
    let cuts: Vec<u8> = kit.slots.iter().map(|s| s.cut_group).collect();
    assert_eq!(cuts, [0, 0, 0, 1, 1, 0, 0, 0]);
}

#[test]
fn every_default_kit_slot_starts_at_the_registry_defaults() {
    for slot in &DrumKit::default_kit().slots {
        assert_eq!(slot.params, SamplerSlot::default().params, "{}", slot.name);
        assert_eq!(slot.params[0], 0.8531, "Level is unity");
    }
}

#[test]
fn the_default_kits_pads_play_48_up() {
    let kit = DrumKit::default_kit();
    let n = kit.slots.len();
    let notes: Vec<Option<u8>> = (0..=n).map(|pad| kit.pad_note(pad)).collect();
    let mut expected: Vec<Option<u8>> = (48..48 + n as u8).map(Some).collect();
    expected.push(None);
    assert_eq!(notes, expected);
}

#[test]
fn a_slot_starts_at_the_registry_defaults() {
    let slot = SamplerSlot::default();
    assert_eq!(
        slot.params,
        // Level 0.8531 is unity (0 dB), the `PRMS` default.
        [0.8531, 0.5, 0.5, 0.0, 0.0, 0.0, 0.0, 1.0, 1.0, 1.0, 0.5]
    );
    assert_eq!(slot.cut_group, 0);
    assert_eq!((slot.name.as_str(), slot.folder.as_str()), ("", ""));
}

#[test]
fn the_pad_name_is_the_last_part_of_the_folder() {
    let slot = |folder: &str| SamplerSlot {
        folder: folder.to_owned(),
        ..SamplerSlot::default()
    };
    assert_eq!(slot("Drum Samples\\Kick").pad_name(), "Kick");
    assert_eq!(slot("My Samples/Hats").pad_name(), "Hats");
    assert_eq!(slot("Snare").pad_name(), "Snare");
    assert_eq!(slot("").pad_name(), "");
}

#[test]
fn slot_i_sits_on_pad_i() {
    let kit = DrumKit {
        slots: vec![
            SamplerSlot {
                name: "Kick 01".to_owned(),
                ..SamplerSlot::default()
            },
            SamplerSlot::default(),
        ],
        ..DrumKit::default()
    };
    assert_eq!(kit.slot_on_pad(0).map(|s| s.name.as_str()), Some("Kick 01"));
    assert_eq!(kit.slot_on_pad(1), Some(&SamplerSlot::default()));
    assert_eq!(kit.slot_on_pad(2), None);
    assert_eq!(DrumKit::default().slot_on_pad(0), None);
}

#[test]
fn a_pad_without_a_stored_note_sends_48_plus_its_index() {
    let kit = |n| DrumKit {
        slots: vec![SamplerSlot::default(); n],
        ..DrumKit::default()
    };
    assert_eq!(
        [0, 1, 2, 3].map(|pad| kit(3).pad_note(pad)),
        [Some(48), Some(49), Some(50), None]
    );
    // Past pad 79 the default is no MIDI key.
    assert_eq!(
        (kit(81).pad_note(79), kit(81).pad_note(80)),
        (Some(127), None)
    );
}

#[test]
fn a_new_lane_has_no_swing() {
    let mut song = Song::default();
    song.add_channel(ChannelKind::Drums);
    assert_eq!(song.channels[1].lanes[0].swing, 0.0);
    assert_eq!(Lane::new(LaneKind::Notes).swing, 0.0);
}

#[test]
fn a_synth_channel_starts_with_its_instrument_module() {
    let mut song = Song::default();
    song.add_channel(ChannelKind::Instrument);
    for synth in SynthKind::ALL {
        song.add_channel(ChannelKind::Synth(synth));
    }
    // The instrument type ids, in menu order.
    let ids = SynthKind::ALL.map(SynthKind::type_id);
    assert_eq!(ids, [0, 1, 2, 19, 23, 33, 39, 52]);
    for (channel, id) in song.channels[2..].iter().zip(ids) {
        assert_eq!(channel.rack.modules, [ModuleInstance::new(id, 2)]);
        assert_eq!(channel.rack.last_instance_id, 2);
    }
    // The other kinds start with an empty rack.
    assert!(song.channels[..2].iter().all(|c| c.rack.modules.is_empty()));
}

fn ids(song: &Song) -> Vec<i32> {
    song.channels.iter().map(|c| c.id).collect()
}

#[test]
fn channels_get_unique_ids_and_the_master_has_0() {
    let mut song = Song::default();
    for _ in 0..3 {
        song.add_channel(ChannelKind::Drums);
    }
    assert_eq!(ids(&song), [0, 1, 2, 3]);
    song.delete_channel(2);
    song.add_channel(ChannelKind::Drums);
    assert_eq!(ids(&song), [0, 1, 3, 4]);
    // New channels show their sub-tracks.
    assert!(song.channels.iter().all(|c| !c.zipped));
}

#[test]
fn an_inserted_channel_takes_the_index_and_follows_the_add_rules() {
    let mut song = Song::default();
    song.add_channel(ChannelKind::Drums);
    song.add_channel(ChannelKind::Drums);
    assert_eq!(song.insert_channel(1, ChannelKind::Drums), Some(1));
    assert_eq!(names(&song), ["MASTER", "Drums 3", "Drums", "Drums 2"]);
    assert_eq!(ids(&song), [0, 3, 1, 2]);
    assert!(song.channels[1].drum_kit.is_some());
    let send = song.insert_channel(1, ChannelKind::EffectTrack).unwrap();
    assert_eq!(song.channels[send].bus_id, 1);
}

#[test]
fn a_clone_goes_below_its_source_with_a_new_id_and_name() {
    let mut song = Song::default();
    song.add_channel(ChannelKind::Synth(SynthKind::MiniSynth));
    song.add_channel(ChannelKind::Drums);
    let synth = &mut song.channels[1];
    synth.lanes[0].add_clip(128, 4.0);
    synth.add_lane(LaneKind::Notes);
    synth.zipped = true;
    synth.params.0[1] = 0.25;
    synth.file.guid = [7; 16];
    assert_eq!(song.clone_channel(1), 2);
    assert_eq!(
        names(&song),
        ["MASTER", "MiniSynth", "MiniSynth 2", "Drums"]
    );
    assert_eq!(ids(&song), [0, 1, 3, 2]);
    // Everything else is the source's: hue, params, rack, lanes and their clips, the hidden sub-tracks.
    let (source, copy) = (&song.channels[1], &song.channels[2]);
    // But not its file identity: the copy's GUID is its own.
    assert_ne!(copy.file.guid, source.file.guid);
    assert_ne!(copy.file.guid, [0; 16]);
    assert_eq!(
        Channel {
            name: source.name.clone(),
            id: source.id,
            file: source.file.clone(),
            ..copy.clone()
        },
        *source
    );
}

#[test]
fn a_cloned_send_owns_a_bus_of_its_own() {
    let mut song = Song::default();
    song.add_channel(ChannelKind::EffectTrack);
    song.add_channel(ChannelKind::Drums);
    song.channels[2].output_bus = 1;
    let copy = song.clone_channel(1);
    assert_eq!(song.channels[copy].bus_id, 2);
    assert_eq!(
        song.channels[3].output_bus, 1,
        "the drums still use the first"
    );
}

#[test]
fn deleting_a_send_routes_its_users_to_the_master_and_the_master_stays() {
    let mut song = Song::default();
    song.add_channel(ChannelKind::EffectTrack);
    song.add_channel(ChannelKind::Drums);
    song.add_channel(ChannelKind::Instrument);
    song.channels[2].output_bus = 1;
    song.channels[3].output_bus = 1;
    song.delete_channel(1);
    assert_eq!(names(&song), ["MASTER", "Drums", "Instrument"]);
    assert!(song.channels.iter().all(|c| c.output_bus == -1));
    song.delete_channel(0);
    song.delete_channel(9);
    assert_eq!(names(&song), ["MASTER", "Drums", "Instrument"]);
}

#[test]
fn new_tracks_go_after_the_last_track_that_is_not_automation() {
    let mut channel = Channel::new("Drums".to_owned(), ChannelKind::Drums, 0.0);
    channel.add_automation_lane(AutomationTarget {
        instance_id: 0,
        param: 1,
    });
    assert_eq!(channel.new_lane_index(), 1);
    assert_eq!(channel.add_lane(LaneKind::Notes), 1);
    assert_eq!(channel.new_lane_index(), 2);
    assert_eq!(channel.add_lane(LaneKind::Drums), 2);
    let kinds: Vec<LaneKind> = channel.lanes.iter().map(|l| l.kind).collect();
    assert_eq!(
        kinds,
        [
            LaneKind::Drums,
            LaneKind::Notes,
            LaneKind::Drums,
            LaneKind::Automation
        ]
    );
}

#[test]
fn new_channel_ids_are_unique_whatever_the_ids_before() {
    // Channels as a reader might leave them: every id 0.
    let mut song = Song::default();
    for _ in 0..3 {
        song.channels
            .push(Channel::new("Drums".to_owned(), ChannelKind::Drums, 0.0));
    }
    song.add_channel(ChannelKind::Drums);
    song.clone_channel(1);
    song.insert_channel(1, ChannelKind::Instrument);
    let mut fresh: Vec<i32> = vec![
        song.channels[1].id,
        song.channels[3].id,
        song.channels[6].id,
    ];
    fresh.sort_unstable();
    assert_eq!(fresh, [1, 2, 3]);
}

// --- Project-file state and new-song defaults ---

#[test]
fn song_file_defaults_are_the_new_song_values() {
    let file = SongFileState::default();
    assert_eq!(file.version, "4.10.19");
    assert_eq!(file.selected_channel, 0);
    assert_eq!(file.last_send_bus, 0);
    assert_eq!(file.playhead_beats, 0.0);
    assert_eq!(file.playlist_view, [0.0, 0.0, 0.01, 0.02]);
    assert_eq!(file.guid, [0; 16]);
    assert!(file.show_sharps);
    assert!(file.play_truncated_notes);
    assert_eq!(file.next_channel_id, 1);
    assert!(file.head_extra.is_empty() && file.time_sig_extra.is_empty());
    assert!(file.unknown_chunks.is_empty());
    let song = Song::default();
    assert_eq!(song.play_loop, None);
    assert_eq!(song, Song::default());
}

#[test]
fn meta_and_keyboard_defaults_are_the_new_song_values() {
    let meta = MetaState::default();
    assert_eq!(meta.created, "?");
    assert_eq!(meta.editing_seconds, 0.0);
    assert_eq!(
        (&*meta.author, &*meta.display_title, &*meta.info_url),
        ("", "", "")
    );
    assert!(!meta.locked);
    assert_eq!(meta.modified, "Never / demo project");
    assert!(meta.extra.is_empty());
    let keyboard = KeyboardState::default();
    assert_eq!(keyboard.chord_json, DEFAULT_CHORD_JSON);
    assert_eq!(
        (
            keyboard.vertical_mode,
            keyboard.set_state,
            keyboard.octave_shift
        ),
        (0, 0, 0)
    );
    assert_eq!(keyboard.chord_set_name, "Major");
    assert_eq!((keyboard.velocity, keyboard.timing), (0.5, 0.0));
    assert!(!keyboard.hold && keyboard.extra.is_empty());
}

#[test]
fn the_default_chord_set_is_eight_chords_in_the_chords_schema() {
    let json: serde_json::Value = serde_json::from_str(DEFAULT_CHORD_JSON).unwrap();
    assert_eq!(json["rootNote"], 0);
    let chords = json["chords"].as_array().unwrap();
    assert_eq!(chords.len(), 8);
    for chord in chords {
        let keys: Vec<&str> = chord
            .as_object()
            .unwrap()
            .keys()
            .map(String::as_str)
            .collect();
        assert_eq!(keys, ["baseNote", "extensions", "octave", "quality"]);
    }
    // Pretty-printed with a 4-space indent.
    assert!(
        DEFAULT_CHORD_JSON.starts_with("{\n    \"chords\": [\n        {\n            \"baseNote\"")
    );
}

#[test]
fn send_and_channel_file_defaults() {
    let send = SendState::default();
    assert_eq!(
        (
            send.target_channel_id,
            send.enabled,
            send.linear_gain,
            send.enabled_init,
            send.level_init,
            send.level
        ),
        (0, 1.0, 1.0, 1.0, 0.8531, 0.8531)
    );
    assert!(send.extra.is_empty());
    let channel = ChannelFileState::default();
    assert_eq!(channel.view, [0.0; 2]);
    assert!(!channel.input_monitoring && !channel.record_armed);
    assert_eq!(channel.guid, [0; 16]);
    assert_eq!(channel.preset_name, "");
    assert!(channel.sends.is_empty() && channel.cc_links.is_empty());
}

#[test]
fn module_lane_and_clip_file_defaults() {
    let module = ModuleFileState::default();
    assert_eq!((module.head_reserved, module.head_enabled), (0, 1));
    assert_eq!(
        (
            &*module.category,
            &*module.preset_name,
            &*module.preset_path,
            &*module.source_path,
            &*module.provenance
        ),
        ("Default", "Default", "", "", "Default/Default")
    );
    assert_eq!((module.key_low, module.key_high), (0, 127));
    assert_eq!(module.custom_data, None);
    let lane = LaneFileState::default();
    assert_eq!((lane.flag, lane.current_clip_id), (0, 0));
    assert_eq!(lane.custom_label, "");
    let clip = ClipFileState::default();
    assert_eq!(clip.source_len_beats, 0.0);
    assert_eq!(clip.zoom, [0.0, 0.0, 1.0 / 192.0, 1.0 / 96.0]);
    assert!(clip.sample.is_none());
}

#[test]
fn kit_and_slot_file_defaults() {
    let kit = KitFileState::default();
    assert_eq!(kit.last_slot_id, 0);
    assert_eq!(kit.pads, std::array::from_fn(|i| i as u8));
    let slot = SlotFileState::default();
    assert_eq!(slot.id, 0);
    assert!(!slot.reverse);
    assert_eq!((slot.beat_offset, slot.beat_offset_control), (0.0, 0.5));
    assert!(slot.overlapping_voices);
    assert_eq!(slot.custom_pad_name, "");
    assert_eq!(
        (slot.output_bus, slot.midi_key, slot.send_target),
        (-1, 0xFFFF, -1)
    );
    assert_eq!(
        (slot.stretch_mode, slot.stretch_ratio, slot.pitch_semitones),
        (0, 1.0, 0.0)
    );
    assert_eq!(slot.path, None);
    assert_eq!((&*slot.preset_path, &*slot.provenance), ("", ""));
}

#[test]
fn raw_bytes_never_make_two_songs_differ() {
    let clip = ClipFileState {
        raw_events: Raw(Some(vec![1, 2, 3])),
        ..ClipFileState::default()
    };
    assert_eq!(clip, ClipFileState::default());
}

#[test]
fn a_new_channel_sends_to_the_master_and_has_a_guid_of_its_own() {
    let mut song = Song::default();
    song.add_channel(ChannelKind::Drums);
    song.add_channel(ChannelKind::Drums);
    assert_eq!(song.master().file.sends, []);
    for channel in &song.channels[1..] {
        assert_eq!(channel.file.sends, [SendState::default()]);
        assert_ne!(channel.file.guid, [0; 16]);
    }
    assert_ne!(song.channels[1].file.guid, song.channels[2].file.guid);
    // The new song's master and song GUIDs come with the first save: `Song::default()` stays one value.
    assert_eq!(song.master().file.guid, [0; 16]);
    assert_eq!(song.file.next_channel_id, 3);
}

#[test]
fn a_send_on_an_effect_track_targets_the_master_too() {
    let mut song = Song::default();
    song.add_channel(ChannelKind::EffectTrack);
    assert_eq!(song.channels[1].file.sends.len(), 1);
    assert_eq!(song.channels[1].file.sends[0].target_channel_id, 0);
}

#[test]
fn channel_ids_follow_the_song_counter_and_stop_below_4095() {
    let mut song = Song::default();
    song.file.next_channel_id = 9;
    song.add_channel(ChannelKind::Drums);
    assert_eq!(song.channels[1].id, 9);
    assert_eq!(song.file.next_channel_id, 10);
    // The counter never goes back: an id is never reused.
    song.delete_channel(1);
    song.add_channel(ChannelKind::Drums);
    assert_eq!(song.channels[1].id, 10);
    // HEAD@354 must stay below 4096, so the last id is 4094.
    song.file.next_channel_id = 4094;
    assert_eq!(song.insert_channel(1, ChannelKind::Drums), Some(1));
    assert_eq!(song.channels[1].id, 4094);
    let count = song.channels.len();
    assert_eq!(song.insert_channel(1, ChannelKind::Drums), None);
    song.add_channel(ChannelKind::Drums);
    assert_eq!(song.clone_channel(1), 1);
    assert_eq!(song.channels.len(), count);
    assert_eq!(song.file.next_channel_id, 4095);
}

#[test]
fn a_send_takes_the_bus_after_the_song_counter() {
    let mut song = Song::default();
    song.file.last_send_bus = 3;
    song.add_channel(ChannelKind::EffectTrack);
    assert_eq!(song.channels[1].bus_id, 4);
    assert_eq!(song.file.last_send_bus, 4);
    song.clone_channel(1);
    assert_eq!(song.channels[2].bus_id, 5);
    assert_eq!(song.file.last_send_bus, 5);
    // A bus already above the counter (an edited song) still gets a fresh number.
    song.channels[2].bus_id = 9;
    song.add_channel(ChannelKind::EffectTrack);
    assert_eq!(song.channels[3].bus_id, 10);
    assert_eq!(song.file.last_send_bus, 10);
}

#[test]
fn deleting_a_send_moves_the_sends_to_it_to_the_master() {
    let mut song = Song::default();
    song.add_channel(ChannelKind::EffectTrack);
    song.add_channel(ChannelKind::Drums);
    let send_id = song.channels[1].id;
    let drums = &mut song.channels[2];
    drums.output_bus = 1;
    drums.file.sends = vec![
        SendState {
            target_channel_id: send_id,
            level: 0.25,
            ..SendState::default()
        },
        SendState::default(),
    ];
    song.delete_channel(1);
    let drums = &song.channels[1];
    assert_eq!(drums.output_bus, -1);
    // The master already had a send: the one to the deleted channel goes.
    assert_eq!(drums.file.sends, [SendState::default()]);
    // A channel whose only send went to it keeps one, now to the master.
    let mut song = Song::default();
    song.add_channel(ChannelKind::EffectTrack);
    song.add_channel(ChannelKind::Drums);
    song.channels[2].file.sends[0].target_channel_id = song.channels[1].id;
    song.delete_channel(1);
    assert_eq!(song.channels[1].file.sends[0].target_channel_id, 0);
}

#[test]
fn a_new_drum_kit_numbers_its_slots_from_1() {
    let kit = DrumKit::default_kit();
    let ids: Vec<u32> = kit.slots.iter().map(|s| s.file.id).collect();
    assert_eq!(ids, [1, 2, 3, 4, 5, 6, 7, 8]);
    assert_eq!(kit.file.last_slot_id, 8);
    assert_eq!(
        kit.slot_by_id(3).map(|s| s.name.as_str()),
        Some("Synth Clap")
    );
    assert_eq!(kit.slot_by_id(9), None);
}

#[test]
fn an_inserted_slot_gets_the_next_id_even_when_cloned() {
    let mut kit = DrumKit::default_kit();
    let copy = kit.slots[0].clone();
    assert_eq!(kit.insert_slot(1, copy), 9);
    assert_eq!(kit.slots[1].file.id, 9);
    assert_eq!(kit.slots[1].name, kit.slots[0].name);
    assert_eq!(kit.file.last_slot_id, 9);
    // Past the end: appended.
    assert_eq!(kit.insert_slot(99, SamplerSlot::default()), 10);
    assert_eq!(kit.slots.last().unwrap().file.id, 10);
    let mut ids: Vec<u32> = kit.slots.iter().map(|s| s.file.id).collect();
    ids.dedup();
    assert_eq!(ids.len(), kit.slots.len());
}

#[test]
fn a_negative_instance_targets_the_drum_slot_with_that_id() {
    let mut song = Song::default();
    song.add_channel(ChannelKind::Drums);
    let drums = &mut song.channels[1];
    let slot = AutomationTarget {
        instance_id: -3,
        param: 1,
    };
    assert_eq!(drums.param(slot), Some(0.5));
    *drums.param_mut(slot).unwrap() = 0.25;
    assert_eq!(drums.drum_kit.as_ref().unwrap().slots[2].params[1], 0.25);
    for missing in [(-9, 1), (-3, 11), (-3, -1), (i32::MIN, 0)] {
        let target = AutomationTarget {
            instance_id: missing.0,
            param: missing.1,
        };
        assert_eq!(drums.param(target), None);
        assert!(drums.param_mut(target).is_none());
    }
    assert_eq!(song.master().param(slot), None);
}

#[test]
fn a_slot_plays_its_stored_midi_note() {
    let mut slot = SamplerSlot::default();
    assert_eq!(slot.midi_note(), None);
    slot.file.midi_key = 36;
    assert_eq!(slot.midi_note(), Some(36));
    slot.file.midi_key = 128;
    assert_eq!(slot.midi_note(), None);
    let mut kit = DrumKit::default_kit();
    kit.slots[2].file.midi_key = 40;
    assert_eq!(kit.pad_note(2), Some(40));
    assert_eq!(kit.pad_note(3), Some(51));
}

#[test]
fn automation_lanes_are_kept_in_file_order() {
    let mut channel = Channel::new("Synth".to_owned(), ChannelKind::Instrument, 0.0);
    channel.rack.add(9);
    channel.rack.add(9);
    let lane = |instance_id, param| AutomationTarget { instance_id, param };
    for (instance, param) in [
        (3, 1),
        (2, AutomationTarget::MIX),
        (2, 4),
        (0, 1),
        (2, AutomationTarget::POST_GAIN),
        (2, AutomationTarget::WET_PAN),
    ] {
        channel.add_automation_lane(lane(instance, param));
    }
    let targets: Vec<_> = channel.lanes.iter().filter_map(|l| l.target).collect();
    // (inst & 0xFF) · 65536 + (file param & 0xFFFF), with the file codes Post Gain −3, Mix −2, Wet Pan 0x7FC2.
    assert_eq!(
        targets,
        [
            lane(0, 1),
            lane(2, 4),
            lane(2, AutomationTarget::WET_PAN),
            lane(2, AutomationTarget::POST_GAIN),
            lane(2, AutomationTarget::MIX),
            lane(3, 1),
        ]
    );
    assert_eq!(channel.lanes[0].kind, LaneKind::Notes);
}

#[test]
fn the_file_codes_of_common_params_round_trip() {
    for (model, file) in [
        (-1, -3),
        (-2, -2),
        (-3, 0x7FC2),
        (-4, -1),
        (-7, -4),
        (0, 0),
        (5, 5),
    ] {
        assert_eq!(AutomationTarget::file_param(2, model), file);
        assert_eq!(AutomationTarget::model_param(2, file), model);
    }
    assert_eq!(AutomationTarget::model_param(2, 0x7FC0), -1);
    assert_eq!(AutomationTarget::model_param(2, 0x7FC1), -2);
    // Channel and drum-slot params are not mapped.
    for instance in [0, -3] {
        assert_eq!(AutomationTarget::file_param(instance, -2), -2);
        assert_eq!(AutomationTarget::model_param(instance, -3), -3);
    }
}

#[test]
fn a_clip_without_a_visible_length_shows_its_source_then_its_loop() {
    let mut clip = clip(0, 0.0, 4.0, 0.0);
    assert_eq!(clip.visible_ticks(), 512.0);
    clip.file.source_len_beats = 2.5;
    assert_eq!(clip.visible_ticks(), 320.0);
    clip.visible_len_beats = 1.0;
    assert_eq!(clip.visible_ticks(), 128.0);
}

#[test]
fn a_new_lane_zooms_its_clips_by_kind() {
    let mut drums = Lane::new(LaneKind::Drums);
    assert_eq!(
        drums.add_clip(0, 4.0).file.zoom,
        ClipFileState::default().zoom
    );
    let mut audio = Lane::new(LaneKind::Audio);
    assert_eq!(
        audio.add_clip(0, 4.0).file.zoom,
        [0.0, 127.0, 1.0 / 96.0, 2.0]
    );
}

#[test]
fn a_save_stamp_dates_the_song_and_gives_missing_guids() {
    let mut song = Song::default();
    song.add_channel(ChannelKind::Drums);
    song.file.version = "4.7.14".into();
    song.stamp_save("2026-10-11 09:08:07");
    assert_eq!(song.file.meta.created, "2026-10-11 09-08-07");
    assert_eq!(song.file.meta.modified, "2026-10-11 09:08:07");
    assert_eq!(song.file.version, "4.10.19");
    let guids = [
        song.file.guid,
        song.channels[0].file.guid,
        song.channels[1].file.guid,
    ];
    assert!(guids.iter().all(|g| *g != [0; 16]));
    assert!(guids[0] != guids[1] && guids[1] != guids[2] && guids[0] != guids[2]);
    // A second save keeps the creation date and every GUID.
    song.stamp_save("2026-10-12 10:00:00");
    assert_eq!(song.file.meta.created, "2026-10-11 09-08-07");
    assert_eq!(song.file.meta.modified, "2026-10-12 10:00:00");
    assert_eq!(song.file.guid, guids[0]);
    assert_eq!(song.channels[1].file.guid, guids[2]);
}

#[test]
fn what_is_not_an_edit_is_kept_from_the_current_song() {
    let mut old = Song::default();
    old.add_channel(ChannelKind::Drums);
    old.add_channel(ChannelKind::Drums);
    let mut current = old.clone();
    current.delete_channel(1);
    current.tempo_bpm = 99.0;
    current.title = "Saved".into();
    current.play_loop = Some((0, 512));
    current.file.playhead_beats = 3.0;
    current.file.selected_channel = 1;
    current.file.playlist_view = [1.0; 4];
    current.file.meta.editing_seconds = 60.0;
    // The lock (META@776) is set by a confirmation, never an edit: no undo takes it back.
    current.file.meta.locked = true;
    // The PROJECT texts: META@520 display title, author, link.
    current.file.meta.display_title = "Shown".into();
    current.channels[1].file.view = [2.0, 3.0];
    current.stamp_save("2026-10-11 09:08:07");
    let mut restored = old.clone();
    restored.keep_unedited(&current);
    // The edits are the restored song's ...
    assert_eq!(restored.tempo_bpm, old.tempo_bpm);
    assert_eq!(restored.channels.len(), 3);
    // ... the rest is the current one's, per channel by id.
    assert_eq!(restored.title, "Saved");
    assert_eq!(restored.play_loop, Some((0, 512)));
    assert_eq!(restored.file.meta, current.file.meta);
    assert_eq!(
        (
            restored.file.version.as_str(),
            restored.file.guid,
            restored.file.playhead_beats
        ),
        (current.file.version.as_str(), current.file.guid, 3.0)
    );
    assert_eq!(
        (restored.file.selected_channel, restored.file.playlist_view),
        (1, [1.0; 4])
    );
    assert_eq!(
        restored.channels[0].file.guid,
        current.channels[0].file.guid
    );
    assert_eq!(
        restored.channels[1].file.guid, old.channels[1].file.guid,
        "gone from the current song"
    );
    assert_eq!(restored.channels[2].file.view, [2.0, 3.0]);
}

#[test]
fn deleting_a_channel_drops_the_send_automation_aimed_at_it() {
    let mut song = Song::default();
    song.add_channel(ChannelKind::EffectTrack);
    song.add_channel(ChannelKind::Drums);
    let id = song.channels[1].id;
    let drums = &mut song.channels[2];
    for param in [0x1000 + id, 0x2000 + id, 0x1000 + id + 1, 1] {
        drums.add_automation_lane(AutomationTarget {
            instance_id: 0,
            param,
        });
    }
    song.delete_channel(1);
    let params: Vec<i32> = (song.channels[1].lanes.iter())
        .filter_map(|l| l.target.map(|t| t.param))
        .collect();
    assert_eq!(params, [1, 0x1000 + id + 1]);
}

#[test]
fn the_master_is_never_moved_or_cloned() {
    let mut song = Song::default();
    song.add_channel(ChannelKind::Drums);
    assert_eq!(song.insert_channel(0, ChannelKind::Drums), Some(1));
    assert_eq!(song.master().kind, ChannelKind::Master);
    let count = song.channels.len();
    assert_eq!(song.clone_channel(0), 0);
    assert_eq!(song.channels.len(), count);
}

#[test]
fn channel_and_common_param_names_come_from_the_registry() {
    assert_eq!(ChannelParams::name(4).as_deref(), Some("Pitch Bend"));
    assert_eq!(ChannelParams::name(6), None);
    assert_eq!(CommonParams::default().0, [0.5, 1.0, 0.5]);
}

#[test]
fn channel_params_7_to_133_are_midi_ccs_and_sends_name_their_target() {
    // 0-5 are RPRM, 6 has no name, 7-133 are MIDI CC #0-126 (no CC #127).
    let name = ChannelParams::name;
    let names = [
        (0, "Level"),
        (5, "Aftertouch"),
        (7, "MIDI CC #0"),
        (8, "MIDI CC #1"),
    ];
    for (i, want) in names
        .into_iter()
        .chain([(71, "MIDI CC #64"), (133, "MIDI CC #126")])
    {
        assert_eq!(name(i).as_deref(), Some(want), "{i}");
    }
    for i in [6, 134, 0x1000, 0x2000] {
        assert_eq!(name(i), None, "{i}");
    }
    // The lane's auto name is the param's name.
    let channel = Channel::master();
    let label = |param| {
        channel.automation_label(AutomationTarget {
            instance_id: 0,
            param,
        })
    };
    assert_eq!(label(71).as_deref(), Some("MIDI CC #64"));
    assert_eq!(label(134), None);
    // The send labels "Enable Send (to <name>)" and "Send Level (to <name>)", with the target's id below 4096 in the
    // low bits.
    let send = |i| ChannelParams::send_name(i, "Effect 3");
    assert_eq!(send(0x1003).as_deref(), Some("Enable Send (to Effect 3)"));
    assert_eq!(send(0x2003).as_deref(), Some("Send Level (to Effect 3)"));
    assert_eq!(send(0x1FFF).as_deref(), Some("Enable Send (to Effect 3)"));
    // Each range's first and last index, and the neighbours just outside them.
    assert_eq!(send(0x1000).as_deref(), Some("Enable Send (to Effect 3)"));
    assert_eq!(send(0x2000).as_deref(), Some("Send Level (to Effect 3)"));
    assert_eq!(send(0x2FFF).as_deref(), Some("Send Level (to Effect 3)"));
    for i in [0, 71, 0xFFF, 0x3000] {
        assert_eq!(send(i), None, "{i}");
    }
}

#[test]
fn a_channel_that_differs_only_in_guid_or_view_is_the_same_edit() {
    let mut song = Song::default();
    song.add_channel(ChannelKind::Drums);
    let a = song.channels[1].clone();
    let mut b = a.clone();
    b.file.guid = [9; 16];
    b.file.view = [1.0, 2.0];
    assert!(a.same_edit(&b) && b.same_edit(&a));
    b.hue = 0.1;
    assert!(!a.same_edit(&b));
    let mut c = a.clone();
    c.id = 7;
    c.file.guid = [9; 16];
    assert!(!a.same_edit(&c));
}

#[test]
fn deleting_a_send_routes_its_users_by_their_next_send() {
    // An instrument sends to A, then B; with A gone it outputs to B, as a save and load would give (RHED[12] is the
    // bus of the first send to a send channel).
    let mut song = Song::default();
    song.add_channel(ChannelKind::EffectTrack);
    song.add_channel(ChannelKind::EffectTrack);
    song.add_channel(ChannelKind::Drums);
    let (a, b) = (song.channels[1].id, song.channels[2].id);
    let drums = &mut song.channels[3];
    drums.output_bus = 1;
    drums.file.sends = [a, b]
        .map(|target_channel_id| SendState {
            target_channel_id,
            ..SendState::default()
        })
        .to_vec();
    song.delete_channel(1);
    assert_eq!(song.channels[2].output_bus, 2);
    assert_eq!(
        song.channels[2].file.sends.len(),
        2,
        "the one to A now goes to the master"
    );
    assert_eq!(
        output_bus(&song.channels[2].file.sends, &song.send_buses()),
        2
    );
    let saved = crate::flm::read(&crate::flm::write(&song).unwrap()).unwrap();
    assert_eq!(saved, song);
}

#[test]
fn a_preset_loads_into_a_module_which_keeps_its_id_and_placement_and_default_resets_it() {
    let mut module = ModuleInstance::new(6, 7);
    module.minimized = true;
    (module.file.key_low, module.file.key_high) = (10, 90);
    module.file.add1_extra = vec![9];
    let mut preset = ModuleInstance::new(6, 0);
    preset.params[1] = 0.9;
    preset.common.0[1] = 0.3;
    preset.file.custom_data = Some(vec![1, 2]);
    preset.file.category = "Old".into();
    module.load_preset(preset.clone());
    let placed = |file: ModuleFileState| ModuleFileState {
        key_low: 10,
        key_high: 90,
        add1_extra: vec![9],
        ..file
    };
    let loaded = ModuleInstance {
        instance_id: 7,
        minimized: true,
        file: placed(preset.file.clone()),
        ..preset
    };
    assert_eq!(module, loaded);

    // Its DESC and PRST name the file it came from, from the user folder.
    let user = std::path::Path::new("/home/u/studio");
    module.name_preset(&user.join("My Presets/Distortion/Crunch.flmpst"), user);
    let f = &module.file;
    assert_eq!(
        [
            &f.category,
            &f.preset_name,
            &f.preset_path,
            &f.source_path,
            &f.provenance
        ],
        [
            "My Presets/Distortion",
            "Crunch",
            "My Presets/Distortion/Crunch.flmpst",
            "/home/u/studio/My Presets/Distortion/Crunch.flmpst",
            "My Presets/Distortion/Crunch"
        ]
    );

    // `Default`: the registry defaults, no CSTM, named Default, still in its place.
    module.load_default();
    let default = ModuleInstance {
        minimized: true,
        file: placed(ModuleFileState::default()),
        ..ModuleInstance::new(6, 7)
    };
    assert_eq!(module, default);
}

#[test]
fn a_new_drum_channels_first_lane_is_a_step_lane() {
    // A new drum channel's first lane has flag 0.
    let mut song = Song::default();
    song.add_channel(ChannelKind::Drums);
    let lane = &song.channels[1].lanes[0];
    assert_eq!((lane.kind, lane.grid), (LaneKind::Drums, DrumGrid::Steps));
    assert_eq!(Lane::new(LaneKind::Drums).grid, DrumGrid::Steps);
    assert_eq!((DrumGrid::Steps as u8, DrumGrid::Notes as u8), (0, 1));
}

#[test]
fn a_loop_shorter_than_a_tick_is_no_loop() {
    // A file may hold any loop length; one under a tick would repeat millions of times on a screen (or in Unlink
    // loop). Like a loop of 0, nothing plays, as the audio bridge refuses the clip.
    for looped in [1e-6, 1e-9, 0.5 / f64::from(PPQ)] {
        let c = clip(0, 4.0, looped, 0.0);
        assert_eq!(c.loop_ticks(), None, "{looped}");
        assert_eq!(c.song_tick(0), None, "{looped}");
        assert_eq!(c.events_in_song_range(0, 512).count(), 0, "{looped}");
    }
    // One tick is a loop.
    let c = clip(0, 4.0 / f64::from(PPQ), 1.0 / f64::from(PPQ), 0.0);
    assert_eq!(c.loop_ticks(), Some(1.0));
    assert_eq!(c.events_in_song_range(0, 512).count(), 4);
}

#[test]
fn deleting_a_channel_keeps_the_files_selection_and_pad_sends_in_range() {
    // HEAD @0 indexes the channels, and a slot's LINk @10 names a channel by id.
    let mut song = Song::default();
    song.add_channel(ChannelKind::EffectTrack);
    song.add_channel(ChannelKind::Drums);
    let send_id = song.channels[1].id;
    song.file.selected_channel = 2;
    let kit = song.channels[2].drum_kit.as_mut().unwrap();
    kit.slots[0].file.send_target = send_id;
    kit.slots[1].file.send_target = 0;
    song.delete_channel(1);
    assert_eq!(song.file.selected_channel, 1);
    let kit = song.channels[1].drum_kit.as_ref().unwrap();
    assert_eq!(kit.slots[0].file.send_target, -1);
    assert_eq!(kit.slots[1].file.send_target, 0);
}

#[test]
fn fit_loop_takes_the_smallest_power_of_two_bars_and_shrinks() {
    let point = |pos| Event::Automation {
        pos,
        value: 0.5,
        control: true,
    };
    let note = |pos, len_beats| Event::Note {
        pos,
        len_beats,
        key: 60,
        velocity: Event::DEFAULT_VELOCITY,
        pan: Event::DEFAULT_PAN,
        slide: false,
    };
    let fit = |events: Vec<Event>, b, loop_beats| {
        let mut c = Lane::new(LaneKind::Automation)
            .add_clip(0, loop_beats)
            .clone();
        c.events = events;
        c.fit_loop(b);
        c.loop_len_beats
    };
    // Empty: one bar, however long the loop was.
    assert_eq!(fit(vec![], 4, 16.0), 4.0);
    assert_eq!(fit(vec![], 3, 1.0), 3.0);
    assert_eq!(fit(vec![], 0, 1.0), 1.0);
    // A point has no length (b·2^k ≥ the last point): 512 fits 4 beats, 513 needs 8, 1025 needs 16 (not 12).
    assert_eq!(fit(vec![point(511)], 4, 64.0), 4.0);
    assert_eq!(fit(vec![point(512)], 4, 4.0), 4.0);
    assert_eq!(fit(vec![point(513)], 4, 4.0), 8.0);
    assert_eq!(fit(vec![point(1024)], 4, 4.0), 8.0);
    assert_eq!(fit(vec![point(1025)], 4, 4.0), 16.0);
    // b = 3: 3, 6, 12.
    assert_eq!(fit(vec![point(384)], 3, 3.0), 3.0);
    assert_eq!(fit(vec![point(385)], 3, 3.0), 6.0);
    assert_eq!(fit(vec![point(800)], 3, 48.0), 12.0);
    // A note counts to its end.
    assert_eq!(fit(vec![note(0, 4.0)], 4, 1.0), 4.0);
    assert_eq!(fit(vec![note(0, 7.0)], 4, 1.0), 8.0);
    assert_eq!(fit(vec![note(0, 7.0)], 3, 1.0), 12.0);
    // The refit never writes the visible length: a set one stays, even when it equals the loop; a length of 0 stays 0
    // and follows the loop through `visible_ticks`.
    let mut c = Lane::new(LaneKind::Automation).add_clip(0, 8.0).clone();
    c.visible_len_beats = 8.0;
    c.fit_loop(4);
    assert_eq!((c.loop_len_beats, c.visible_len_beats), (4.0, 8.0));
    c.visible_len_beats = 12.0;
    c.fit_loop(4);
    assert_eq!((c.loop_len_beats, c.visible_len_beats), (4.0, 12.0));
    c.visible_len_beats = 0.0;
    c.events = vec![point(600)];
    c.fit_loop(4);
    assert_eq!((c.loop_len_beats, c.visible_len_beats), (8.0, 0.0));
}

#[test]
fn a_clip_with_a_set_visible_length_keeps_it_when_the_loop_changes_and_a_new_one_follows_the_loop()
{
    // The length is the visible length (CLHd@0) if it is > 0, else the loop.
    // A new clip has 0, so its length grows and shrinks with the loop; once the clip has been resized, its length
    // stays fixed and the content loops inside it. A resize to exactly the loop length is still a set length.
    let note = |pos| Event::Note {
        pos,
        len_beats: 0.25,
        key: 60,
        velocity: Event::DEFAULT_VELOCITY,
        pan: Event::DEFAULT_PAN,
        slide: false,
    };
    let beats = |c: &Clip| c.visible_ticks() / f64::from(PPQ);
    let plays =
        |c: &Clip, end| -> Vec<u64> { c.events_in_song_range(0, end).map(|(t, _)| t).collect() };

    // New: 0 = the loop, two bars after a note at beat 5, one again once it goes.
    let mut new = Lane::new(LaneKind::Notes).add_clip(0, 4.0).clone();
    assert_eq!((new.visible_len_beats, beats(&new)), (0.0, 4.0));
    new.events = vec![note(5 * 128)];
    new.fit_loop(4);
    assert_eq!(
        (new.loop_len_beats, new.visible_len_beats, beats(&new)),
        (8.0, 0.0, 8.0)
    );
    assert_eq!(plays(&new, 4096), [640]);
    new.events.clear();
    new.fit_loop(4);
    assert_eq!(
        (new.loop_len_beats, new.visible_len_beats, beats(&new)),
        (4.0, 0.0, 4.0)
    );

    // Resized to the loop length: the loop grows past it, the length stays and the note beyond it is not heard.
    let mut sized = Lane::new(LaneKind::Notes).add_clip(0, 4.0).clone();
    sized.visible_len_beats = 4.0;
    sized.events = vec![note(5 * 128)];
    sized.fit_loop(4);
    assert_eq!(
        (sized.loop_len_beats, sized.visible_len_beats, beats(&sized)),
        (8.0, 4.0, 4.0)
    );
    assert_eq!(plays(&sized, 4096), Vec::<u64>::new());
    // Two bars shown of an 8-beat loop, then the loop shrinks to 4: the length stays 8 and the content repeats.
    let mut sized = Lane::new(LaneKind::Notes).add_clip(0, 8.0).clone();
    sized.visible_len_beats = 8.0;
    sized.events = vec![note(0)];
    sized.fit_loop(4);
    assert_eq!(
        (sized.loop_len_beats, sized.visible_len_beats, beats(&sized)),
        (4.0, 8.0, 8.0)
    );
    assert_eq!(plays(&sized, 4096), [0, 512]);

    // An audio clip of 0 shows its source length (CLHd@0 → @28), whatever the loop does.
    let mut audio = Lane::new(LaneKind::Audio).add_clip(0, 4.0).clone();
    audio.file.source_len_beats = 5.0;
    audio.fit_loop(4);
    assert_eq!(
        (audio.loop_len_beats, audio.visible_len_beats, beats(&audio)),
        (8.0, 0.0, 5.0)
    );
}

#[test]
fn an_audio_clips_loop_also_holds_its_source_in_4_2_m_beats() {
    // CLHd@8: L = max(b·2^k ≥ the last envelope point, 4·2^m ≥ trunc(4·S)/4), ≥ 4 and b.
    let envelope = |pos| Event::Envelope {
        pos,
        env_param: 0,
        value: 0.5,
        control: true,
    };
    let fit = |source: f64, events: Vec<Event>, b| {
        let mut c = Lane::new(LaneKind::Audio).add_clip(0, 1.0).clone();
        c.file.source_len_beats = source;
        c.events = events;
        c.fit_loop(b);
        c.loop_len_beats
    };
    // A 16-beat source and no points keeps 16.
    assert_eq!(fit(16.0, vec![], 4), 16.0);
    // trunc(4·5.3)/4 = 5.25 → 8; trunc(4·4.2)/4 = 4.0 → 4 (whole quarter beats).
    assert_eq!(fit(5.3, vec![], 4), 8.0);
    assert_eq!(fit(4.2, vec![], 4), 4.0);
    // The envelope side wins when it is longer: 9 beats → 16. A last point on 8 beats keeps 8; a tick past needs 16.
    assert_eq!(fit(5.0, vec![envelope(9 * 128)], 4), 16.0);
    assert_eq!(fit(5.0, vec![envelope(8 * 128)], 4), 8.0);
    assert_eq!(fit(5.0, vec![envelope(8 * 128 + 1)], 4), 16.0);
    // Never below 4 or b: a 1-beat source at b = 3 is 4, at b = 6 is 6.
    assert_eq!(fit(1.0, vec![], 3), 4.0);
    assert_eq!(fit(1.0, vec![], 6), 6.0);
    // The source side counts in 4·2^m, not b·2^k: 5 beats at b = 3 is 8, not 6.
    assert_eq!(fit(5.0, vec![], 3), 8.0);
}

#[test]
fn deleting_a_channel_before_the_selected_one_keeps_it_selected() {
    // [M, A, B, C] with B selected; A goes, and HEAD @0 follows B to index 2.
    let mut song = Song::default();
    for _ in 0..3 {
        song.add_channel(ChannelKind::Instrument);
    }
    let b = song.channels[2].id;
    song.file.selected_channel = 2;
    song.delete_channel(1);
    assert_eq!(song.file.selected_channel, 1);
    assert_eq!(song.channels[1].id, b);
    // The selected one itself: the next takes its index, or the last when it was the last.
    song.delete_channel(1);
    assert_eq!(song.file.selected_channel, 1);
    song.delete_channel(1);
    assert_eq!(song.file.selected_channel, 0);
}

#[test]
fn plays_to_draw_bounds_the_plays_a_pixel_not_the_passes() {
    // A 4-tick loop of 100 000 notes over a million beats: a pass a pixel, but 100 000 plays in each.
    let mut c = clip(0, 1e6, 4.0 / f64::from(PPQ), 0.0);
    c.events = (0..100_000)
        .map(|i| Event::Automation {
            pos: i % 4,
            value: 0.5,
            control: true,
        })
        .collect();
    assert_eq!(c.plays_to_draw(0, 64, 4.0).count(), 0);
    // At most 128 rows' worth a pixel: 128 events in a pass of a pixel draw, 129 do not.
    c.events.truncate(128);
    assert_eq!(c.plays_to_draw(0, 64, 4.0).count(), 128 * 16);
    c.events.clear();
    c.events.extend((0..129).map(|i| Event::Automation {
        pos: i % 4,
        value: 0.5,
        control: true,
    }));
    assert_eq!(c.plays_to_draw(0, 64, 4.0).count(), 0);
    assert_eq!(c.plays_to_draw(0, 64, 3.9).count(), 129 * 16);
}

#[test]
fn drum_slot_automation_labels_read_param_colon_pad_name() {
    // The pad name is the row header's first line: the custom pad name, else the sample's folder.
    let mut song = Song::default();
    song.add_channel(ChannelKind::Drums);
    let channel = song.channel_mut(1).unwrap();
    let id = channel.drum_kit.as_ref().unwrap().slots[1].file.id;
    let speed = AutomationTarget {
        instance_id: -(id as i32),
        param: 2,
    };
    assert_eq!(
        channel.automation_label(speed).as_deref(),
        Some("Speed : Snare")
    );
    let slot = &mut channel.drum_kit.as_mut().unwrap().slots[1];
    slot.file.custom_pad_name = "Rim".into();
    assert_eq!(channel.add_automation_lane(speed).label, "Speed : Rim");
    // A param past the slot's, a slot the kit lacks.
    let past = AutomationTarget { param: 11, ..speed };
    assert_eq!(channel.automation_label(past), None);
    let gone = AutomationTarget {
        instance_id: -99,
        ..speed
    };
    assert_eq!(channel.automation_label(gone), None);
}

#[test]
fn a_slots_display_name_is_its_custom_pad_name_else_its_folder() {
    let mut slot = SamplerSlot {
        name: "Big Kick".into(),
        folder: "Drum Samples/Kick".into(),
        ..SamplerSlot::default()
    };
    assert_eq!(slot.display_name(), "Kick");
    slot.file.custom_pad_name = "Mine".into();
    assert_eq!(slot.display_name(), "Mine");
}

#[test]
fn a_drum_slot_lane_without_a_pad_name_reads_the_sample_name_else_the_param_alone() {
    let mut song = Song::default();
    song.add_channel(ChannelKind::Drums);
    let channel = song.channel_mut(1).unwrap();
    let slot = &mut channel.drum_kit.as_mut().unwrap().slots[1];
    slot.folder.clear();
    let speed = AutomationTarget {
        instance_id: -(slot.file.id as i32),
        param: 2,
    };
    assert_eq!(
        channel.automation_label(speed).as_deref(),
        Some("Speed : Synth Snare")
    );
    channel.drum_kit.as_mut().unwrap().slots[1].name.clear();
    assert_eq!(channel.automation_label(speed).as_deref(), Some("Speed"));
}

/// A song: master, `Effect 1` (a send), and a Drums channel (id 2) sending to the master and to it at 0.4.
fn send_song() -> Song {
    let mut song = Song::default();
    song.add_channel(ChannelKind::EffectTrack);
    song.add_channel(ChannelKind::Drums);
    song.channels[2].file.sends.push(SendState {
        target_channel_id: 1,
        level: 0.4,
        ..SendState::default()
    });
    song
}

#[test]
fn send_params_are_the_channels_sends() {
    // 0x1000 + id is Enable Send, 0x2000 + id Send Level, of the send to channel id.
    let mut song = send_song();
    let ch = &mut song.channels[2];
    let target = |param| AutomationTarget {
        instance_id: 0,
        param,
    };
    assert_eq!(ch.param(target(0x2001)), Some(0.4));
    assert_eq!(ch.param(target(0x1000)), Some(1.0));
    // No send to channel 5: no param.
    assert_eq!(ch.param(target(0x1005)), None);
    *ch.param_mut(target(0x1001)).unwrap() = 0.0;
    assert_eq!(ch.file.sends[1].enabled, 0.0);
}

#[test]
fn send_lanes_are_named_after_their_target_channel() {
    let mut song = send_song();
    let target = |param| AutomationTarget {
        instance_id: 0,
        param,
    };
    let label = |song: &Song, param| song.automation_label(2, target(param));
    assert_eq!(
        label(&song, 0x2001).as_deref(),
        Some("Send Level (to Effect 1)")
    );
    assert_eq!(
        label(&song, 0x1000).as_deref(),
        Some("Enable Send (to MASTER)")
    );
    // Other params read as the channel names them; a send to no channel has no name.
    assert_eq!(label(&song, 0).as_deref(), Some("Level"));
    assert_eq!(label(&song, 0x2009), None);
    song.channels[1].name = "Verb".into();
    assert_eq!(
        label(&song, 0x2001).as_deref(),
        Some("Send Level (to Verb)")
    );
}

#[test]
fn a_point_on_the_loop_end_plays_just_before_the_next_passs_content_0() {
    // fit_loop keeps a point on the loop end; it plays with the next pass's content 0, before it, so the lower
    // position lands last at the tie (as `honeydew_audio::automation` dispatches one), at any offset, in time order.
    let point = |pos, value| Event::Automation {
        pos,
        value,
        control: true,
    };
    for (offset, want) in [
        (
            0.0,
            [(0, 0.9), (0, 0.2), (128, 0.5), (256, 0.9), (256, 0.2)],
        ),
        (
            1.0,
            [(0, 0.5), (128, 0.9), (128, 0.2), (256, 0.5), (384, 0.9)],
        ),
    ] {
        let mut c = Lane::new(LaneKind::Automation).add_clip(0, 2.0).clone();
        (c.visible_len_beats, c.offset_beats) = (4.0, offset);
        c.events = vec![point(0, 0.2), point(128, 0.5), point(256, 0.9)];
        let plays: Vec<(u64, f32)> = (c.events_in_song_range(0, 512))
            .map(|(tick, e)| match *e {
                Event::Automation { value, .. } => (tick, value),
                _ => unreachable!(),
            })
            .take(5)
            .collect();
        assert_eq!(plays, want, "offset {offset}");
    }
}

#[test]
fn a_new_playlist_order_moves_the_channels_and_the_masters_place_and_a_save_keeps_both() {
    // Master, a send, a Drums channel with a second track, a MiniSynth: ids 0..=3.
    let mut song = Song::default();
    song.add_channel(ChannelKind::EffectTrack);
    song.add_channel(ChannelKind::Drums);
    song.add_channel(ChannelKind::Synth(SynthKind::MiniSynth));
    song.channels[2].add_lane(LaneKind::Notes);
    song.file.selected_channel = 2;
    assert_eq!(song.playlist(), [0, 1, 2, 3]);
    // The send to the bottom; the file's selected channel (an index) stays on its channel.
    assert!(song.set_playlist(&[0, 2, 3, 1]));
    assert_eq!(ids(&song), [0, 2, 3, 1]);
    assert_eq!(song.playlist(), [0, 1, 2, 3]);
    assert_eq!(song.file.selected_channel, 1);
    // The master moves too, and stays channel 0: below the MiniSynth.
    assert!(song.set_playlist(&[1, 2, 0, 3]));
    assert_eq!(ids(&song), [0, 2, 3, 1]);
    assert_eq!(song.master_below, Some(3));
    assert_eq!(song.playlist(), [1, 2, 0, 3]);
    // No order, or the same one, changes nothing.
    for order in [&[1, 2, 0, 3][..], &[0, 1, 2], &[0, 1, 2, 2], &[0, 1, 2, 4]] {
        assert!(!song.set_playlist(order), "{order:?}");
    }
    // The rows a save writes (@1028) bring both back; the racks stay in kind order.
    let saved = crate::flm::read(&crate::flm::write(&song).unwrap()).unwrap();
    assert_eq!(ids(&saved), [0, 2, 3, 1]);
    assert_eq!(saved.master_below, Some(3));
    assert_eq!(saved.channels[1].lanes.len(), 2);
    // A clone of the channel above the master goes between them; a delete of it leaves the master below the next one
    // up, and of the top one, first.
    let copy = song.clone_channel(2);
    assert_eq!(song.master_below, Some(song.channels[copy].id));
    assert_eq!(song.playlist(), [1, 2, 3, 0, 4]);
    song.delete_channel(3);
    assert_eq!(song.playlist(), [1, 2, 0, 3]);
    song.delete_channel(2);
    song.delete_channel(1);
    assert_eq!((song.playlist(), song.master_below), (vec![0, 1], None));
}

#[test]
fn a_scale_change_reloads_an_untouched_default_chord_set_and_leaves_a_loaded_one() {
    use crate::song::chords::ChordSet;
    let mut song = Song::default();
    // Minor Natural (id 10): the Minor default, in C, root 0.
    song.set_key_scale(10);
    let k = &song.file.keyboard;
    assert_eq!(song.key_scale, 10);
    assert_eq!(
        k.chord_json,
        ChordSet::default_set(true).transposed(0).to_json()
    );
    assert_eq!(
        (k.chord_set_name.as_str(), k.set_state),
        ("Minor", SET_DEFAULT)
    );
    // Back to a major scale: the built-in set itself.
    song.set_key_scale(1);
    assert_eq!(song.file.keyboard.chord_json, DEFAULT_CHORD_JSON);
    assert_eq!(song.file.keyboard.chord_set_name, "Major");
    // A loaded set stays.
    song.file.keyboard.set_state = SET_LOADED;
    song.file.keyboard.chord_set_name = "Mine".into();
    song.set_key_scale(10);
    let k = &song.file.keyboard;
    assert_eq!(
        (k.chord_json.as_str(), k.chord_set_name.as_str()),
        (DEFAULT_CHORD_JSON, "Mine")
    );
    assert_eq!(song.key_scale, 10);
}
