use honeydew_audio::automation::{
    AutomationClip, AutomationLane, AutomationPlayback, AutomationPoint, AutomationTarget,
};

fn target(channel: usize) -> AutomationTarget {
    AutomationTarget {
        channel,
        instance_id: 7,
        parameter_index: 2,
    }
}

fn clip(
    start: u64,
    length: u64,
    loop_ticks: u64,
    offset: u64,
    points: &[(u64, u16)],
) -> AutomationClip {
    AutomationClip::new(
        start,
        length,
        loop_ticks,
        offset,
        3,
        points
            .iter()
            .map(|&(tick, value)| AutomationPoint { tick, value })
            .collect(),
    )
    .unwrap()
}

#[test]
fn full_scale_ramps_generate_interior_levels_in_both_directions() {
    let rising = clip(0, 512, 512, 0, &[(0, 0), (512, 65534)]);
    let falling = clip(0, 512, 512, 0, &[(0, 65534), (512, 0)]);
    let rise_steps: Vec<_> = rising.events().iter().filter(|e| e.key < 0).collect();
    let fall_steps: Vec<_> = falling.events().iter().filter(|e| e.key < 0).collect();

    assert_eq!(rise_steps.len(), 127);
    assert_eq!(fall_steps.len(), 127);
    assert_eq!(
        (rise_steps[0].tick, rise_steps[0].key, rise_steps[0].value),
        (4, -3, 511)
    );
    assert_eq!((rise_steps[126].tick, rise_steps[126].value), (508, 65022));
    assert_eq!((fall_steps[0].tick, fall_steps[0].value), (4, 65022));
    assert_eq!((fall_steps[126].tick, fall_steps[126].value), (508, 511));
}

#[test]
fn small_or_same_tick_changes_have_no_steps_and_rebuild_is_idempotent() {
    let small = clip(0, 512, 512, 0, &[(0, 32256), (512, 32768)]);
    assert_eq!(small.events().len(), 2);

    let mut jump = clip(0, 128, 128, 0, &[(16, 0), (16, 65535)]);
    assert_eq!(jump.events().len(), 2);
    jump.rebuild_events();
    jump.rebuild_events();
    assert_eq!(jump.events().len(), 2);
    assert_eq!(jump.events()[1].normalized_value(), 1.0);
    jump.replace_points(vec![AutomationPoint {
        tick: 8,
        value: 100,
    }]);
    assert_eq!(
        jump.points(),
        &[AutomationPoint {
            tick: 8,
            value: 100
        }]
    );
    assert_eq!(jump.events().len(), 1);
}

#[test]
fn short_ramp_keeps_multiple_levels_on_one_tick() {
    let ramp = clip(0, 128, 128, 0, &[(10, 0), (11, 65534)]);
    assert_eq!(ramp.events().iter().filter(|e| e.key < 0).count(), 127);
    assert_eq!(ramp.events().iter().filter(|e| e.tick == 10).count(), 64);
}

#[test]
fn block_dispatch_respects_clip_offset_loop_and_channel_target() {
    let lane = AutomationLane {
        target: target(1),
        initial_value: 0.25,
        clips: vec![clip(128, 256, 128, 64, &[(0, 0), (64, 511)])],
    };
    let playback = AutomationPlayback { lanes: vec![lane] };
    let mut changes = Vec::new();
    playback
        .dispatch_block(1.0, 128, 128.0, 60.0, |sample, target, value| {
            changes.push((sample, target, value));
        })
        .unwrap();

    assert_eq!(
        changes,
        vec![(0, target(1), 511.0 / 65535.0), (64, target(1), 0.0)]
    );
    assert_eq!(playback.value_at(target(1), 192), Some(0.0));
    assert_eq!(playback.value_at(target(1), 256), Some(511.0 / 65535.0));
    assert_eq!(playback.value_at(target(1), 384), Some(0.25));
    assert_eq!(playback.value_at(target(0), 192), None);
}

#[test]
fn seek_resets_prior_transport_value_then_applies_middle_value() {
    let lane = AutomationLane {
        target: target(0),
        initial_value: 0.25,
        clips: vec![clip(0, 512, 512, 0, &[(0, 0), (512, 65534)])],
    };
    let playback = AutomationPlayback { lanes: vec![lane] };
    let mut values = Vec::new();

    playback.seek(400, |target, value| values.push((target, value)));
    values.clear();
    playback.seek(256, |target, value| values.push((target, value)));
    assert_eq!(values.len(), 2);
    assert_eq!(values[0], (target(0), 0.25));
    assert_eq!(values[1], (target(0), 32767.0 / 65535.0));

    values.clear();
    playback.seek(600, |target, value| values.push((target, value)));
    assert_eq!(values, vec![(target(0), 0.25)]);
}

#[test]
fn loop_endpoint_does_not_replace_the_first_point_at_song_start_or_next_loop() {
    let playback = AutomationPlayback {
        lanes: vec![AutomationLane {
            target: target(0),
            initial_value: 0.25,
            clips: vec![clip(0, 1024, 512, 0, &[(0, 0), (512, 65534)])],
        }],
    };

    assert_eq!(playback.value_at(target(0), 0), Some(0.0));
    assert_eq!(playback.value_at(target(0), 512), Some(0.0));
    let mut changes = Vec::new();
    playback
        .dispatch_block(4.0, 1, 128.0, 60.0, |_, _, value| changes.push(value))
        .unwrap();
    assert_eq!(changes, vec![65534.0 / 65535.0, 0.0]);
}

#[test]
fn instance_ids_and_common_parameters_are_independent_targets() {
    let a = AutomationTarget {
        channel: 2,
        instance_id: 11,
        parameter_index: -2,
    };
    let b = AutomationTarget {
        channel: 2,
        instance_id: 12,
        parameter_index: -2,
    };
    let playback = AutomationPlayback {
        lanes: vec![
            AutomationLane {
                target: a,
                initial_value: 0.1,
                clips: vec![clip(0, 128, 128, 0, &[(0, 16384)])],
            },
            AutomationLane {
                target: b,
                initial_value: 0.9,
                clips: vec![clip(0, 128, 128, 0, &[(0, 49152)])],
            },
        ],
    };
    assert_eq!(playback.value_at(a, 64), Some(16384.0 / 65535.0));
    assert_eq!(playback.value_at(b, 64), Some(49152.0 / 65535.0));
}

#[test]
fn source_points_past_loop_length_wrap_with_clip_offset() {
    let playback = AutomationPlayback {
        lanes: vec![AutomationLane {
            target: target(0),
            initial_value: 0.0,
            clips: vec![clip(256, 128, 128, 32, &[(160, 32768)])],
        }],
    };
    assert_eq!(playback.value_at(target(0), 256), Some(32768.0 / 65535.0));
}

#[test]
fn fractional_sample_event_dispatches_in_the_next_block() {
    let playback = AutomationPlayback {
        lanes: vec![AutomationLane {
            target: target(0),
            initial_value: 0.0,
            clips: vec![clip(0, 256, 256, 0, &[(1, 65534)])],
        }],
    };
    let mut changes = Vec::new();
    playback
        .dispatch_block(0.0, 188, 48_000.0, 120.0, |frame, _, _| changes.push(frame))
        .unwrap();
    assert!(changes.is_empty());
    playback
        .dispatch_block(188.0 / 24_000.0, 1, 48_000.0, 120.0, |frame, _, _| {
            changes.push(frame)
        })
        .unwrap();
    assert_eq!(changes, vec![0]);
}
