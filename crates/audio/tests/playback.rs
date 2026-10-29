use honeydew_audio::automation::{
    AutomationClip, AutomationLane, AutomationPlayback, AutomationPoint, AutomationTarget,
};
use honeydew_audio::playback::{AudioPlayback, AutomationChange, StereoBlock};
use honeydew_audio::{AudioEffect, ProcessContext};

const CONTEXT: ProcessContext = ProcessContext {
    sample_rate: 128.0,
    tempo_bpm: 60.0,
    start_beat: 0.0,
    playing: true,
};

fn lane(target: AutomationTarget, initial_value: f32, tick: u64, value: u16) -> AutomationLane {
    AutomationLane {
        target,
        initial_value,
        clips: vec![
            AutomationClip::new(
                0,
                128,
                128,
                0,
                (target.parameter_index + 1) as i16,
                vec![AutomationPoint {
                    tick,
                    value,
                    ..Default::default()
                }],
            )
            .unwrap(),
        ],
    }
}

#[test]
fn automation_changes_one_effect_at_its_sample_boundary() {
    let target = AutomationTarget {
        channel: 0,
        instance_id: 7,
        parameter_index: 0,
    };
    let automation = AutomationPlayback {
        lanes: vec![lane(target, 1.0, 64, 0)],
    };
    let mut host = AudioPlayback::new(2, automation);
    let mut first = AudioEffect::new(20, 128.0).unwrap();
    first.set_parameter(1, 0.0).unwrap();
    let mut second = AudioEffect::new(20, 128.0).unwrap();
    second.set_parameter(1, 0.0).unwrap();
    host.add_effect(0, 7, first).unwrap();
    host.add_effect(1, 7, second).unwrap();

    let changes = host.collect_changes(CONTEXT, 128).unwrap();
    assert_eq!(changes.len(), 1);
    assert_eq!(changes[0].frame, 64);
    let mut first_left = [1.0; 128];
    let mut first_right = [1.0; 128];
    let mut second_left = [1.0; 128];
    let mut second_right = [1.0; 128];
    host.process_block(
        &mut [
            StereoBlock {
                left: &mut first_left,
                right: &mut first_right,
            },
            StereoBlock {
                left: &mut second_left,
                right: &mut second_right,
            },
        ],
        CONTEXT,
        &changes,
    )
    .unwrap();

    assert!(first_left[63] < 0.9);
    assert_eq!(first_left[64], 1.0);
    assert!(second_left[64] < 0.9);
    assert_eq!(host.effect(0, 7).unwrap().parameter(0), Some(0.0));
    assert_eq!(host.effect(1, 7).unwrap().parameter(0), Some(1.0));
}

#[test]
fn seek_restores_effect_and_channel_values() {
    let effect_target = AutomationTarget {
        channel: 0,
        instance_id: 7,
        parameter_index: 0,
    };
    let channel_target = AutomationTarget {
        channel: 0,
        instance_id: 0,
        parameter_index: 0,
    };
    let automation = AutomationPlayback {
        lanes: vec![
            lane(effect_target, 1.0, 0, 0),
            lane(channel_target, 0.8531, 0, 32768),
        ],
    };
    let mut host = AudioPlayback::new(1, automation);
    host.add_effect(0, 7, AudioEffect::new(20, 128.0).unwrap())
        .unwrap();
    host.seek(64).unwrap();
    assert_eq!(host.effect(0, 7).unwrap().parameter(0), Some(0.0));
    assert_eq!(host.channel_parameter(0, 0), Some(32768.0 / 65535.0));
    host.seek(128).unwrap();
    assert_eq!(host.effect(0, 7).unwrap().parameter(0), Some(1.0));
    assert_eq!(host.channel_parameter(0, 0), Some(0.8531));
}

#[test]
fn seek_accepts_midi_cc_targets_and_rejects_bad_initial_values_atomically() {
    let cc_target = AutomationTarget {
        channel: 0,
        instance_id: 0,
        parameter_index: 133,
    };
    let invalid_target = AutomationTarget {
        channel: 0,
        instance_id: 0,
        parameter_index: 1,
    };
    let mut host = AudioPlayback::new(
        1,
        AutomationPlayback {
            lanes: vec![
                lane(cc_target, 0.0, 0, 32768),
                lane(invalid_target, f32::NAN, 0, 65534),
            ],
        },
    );
    assert!(host.seek(64).is_err());
    assert_eq!(host.channel_parameter(0, 133), Some(0.0));
    assert_eq!(host.channel_parameter(0, 1), Some(0.5));
    host.automation.lanes.pop();
    host.seek(64).unwrap();
    assert_eq!(host.channel_parameter(0, 133), Some(32768.0 / 65535.0));
}

#[test]
fn module_common_mix_uses_the_rack_instance_target() {
    let mut host = AudioPlayback::new(1, AutomationPlayback::default());
    let mut effect = AudioEffect::new(20, 128.0).unwrap();
    effect.set_parameter(1, 0.0).unwrap();
    host.add_effect(0, 7, effect).unwrap();
    host.set_target(
        AutomationTarget {
            channel: 0,
            instance_id: 7,
            parameter_index: -2,
        },
        0.0,
    )
    .unwrap();
    let mut left = [1.0; 2048];
    let mut right = [1.0; 2048];
    host.process_block(
        &mut [StereoBlock {
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
fn invalid_change_rejects_the_block_before_touching_audio() {
    let mut host = AudioPlayback::new(1, AutomationPlayback::default());
    let mut left = [0.5; 4];
    let mut right = [0.5; 4];
    let change = honeydew_audio::playback::AutomationChange {
        frame: 2,
        target: AutomationTarget {
            channel: 0,
            instance_id: 9,
            parameter_index: 0,
        },
        value: 0.0,
    };
    assert!(
        host.process_block(
            &mut [StereoBlock {
                left: &mut left,
                right: &mut right,
            }],
            CONTEXT,
            &[change],
        )
        .is_err()
    );
    assert_eq!(left, [0.5; 4]);
}

#[test]
fn sample_rate_mismatch_rejects_before_an_earlier_effect_changes_audio() {
    let mut host = AudioPlayback::new(1, AutomationPlayback::default());
    let mut first = AudioEffect::new(20, 128.0).unwrap();
    first.set_parameter(1, 0.0).unwrap();
    host.add_effect(0, 7, first).unwrap();
    host.add_effect(0, 8, AudioEffect::new(20, 48_000.0).unwrap())
        .unwrap();
    let mut left = [1.0; 64];
    let mut right = [1.0; 64];
    assert!(
        host.process_block(
            &mut [StereoBlock {
                left: &mut left,
                right: &mut right,
            }],
            CONTEXT,
            &[],
        )
        .is_err()
    );
    assert_eq!(left, [1.0; 64]);
}

#[test]
fn several_changes_of_one_param_at_one_frame_apply_only_the_last() {
    // Pitcher 2's Low Latency (p25) switch rebuilds its analysis, which resets it.
    const RATE: f32 = 48_000.0;
    let target = AutomationTarget {
        channel: 0,
        instance_id: 1,
        parameter_index: 25,
    };
    let run = |low_latency: f32, values: &[f32]| -> (Vec<f32>, f32) {
        let mut host = AudioPlayback::new(1, AutomationPlayback::default());
        let mut pitcher = AudioEffect::new(45, RATE).unwrap();
        pitcher.set_parameter(25, low_latency).unwrap();
        host.add_effect(0, 1, pitcher).unwrap();
        let changes: Vec<_> = values
            .iter()
            .map(|&value| AutomationChange {
                frame: 4_800,
                target,
                value,
            })
            .collect();
        let mut left: Vec<f32> = (0..9_600).map(|i| (i as f32 * 0.05).sin()).collect();
        let mut right = left.clone();
        let context = ProcessContext {
            sample_rate: RATE,
            tempo_bpm: 120.0,
            ..CONTEXT
        };
        host.process_block(
            &mut [StereoBlock {
                left: &mut left,
                right: &mut right,
            }],
            context,
            &changes,
        )
        .unwrap();
        (left, host.effect(0, 1).unwrap().parameter(25).unwrap())
    };
    // On, then Off and On at one frame: no switch at all, as with no change.
    assert!(run(1.0, &[0.0, 1.0]) == run(1.0, &[]));
    // Off, then On, Off and On at one frame: one switch to On.
    assert!(run(0.0, &[1.0, 0.0, 1.0]) == run(0.0, &[1.0]));
}

/// After a tempo drag's release down to 60 BPM, the Tape Stop taken over plays the next synced 16-beat
/// Stop whole in the ring built for 60 BPM, as one sized for it from the start does.
#[test]
fn a_tape_stop_taken_over_at_a_slower_tempo_plays_its_next_stop_whole() {
    const RATE: f32 = 8_000.0;
    let built = |tempo: f32| {
        let mut playback = AudioPlayback::new(1, AutomationPlayback::default());
        let mut effect = AudioEffect::new(42, RATE).unwrap();
        effect.configure_tempo(tempo).unwrap();
        for (index, value) in [(3, 1.0), (6, 0.0), (7, 1.0)] {
            effect.set_parameter(index, value).unwrap();
        }
        playback.add_effect(0, 2, effect).unwrap();
        playback
    };
    let (mut running, mut roomy) = (built(120.0), built(60.0));
    let mut seed = 5_u32;
    let mut noise = move |frames: usize| -> Vec<f32> {
        (0..frames)
            .map(|_| {
                seed = seed.wrapping_mul(1_664_525).wrapping_add(1_013_904_223);
                (seed >> 8) as f32 / (1 << 23) as f32 - 1.0
            })
            .collect()
    };
    let run = |playback: &mut AudioPlayback, input: &[f32], tempo_bpm: f32| {
        let context = ProcessContext {
            sample_rate: RATE,
            tempo_bpm,
            ..CONTEXT
        };
        let (mut left, mut right) = (input.to_vec(), input.to_vec());
        for (l, r) in left.chunks_mut(1024).zip(right.chunks_mut(1024)) {
            playback.process_channel(0, l, r, context).unwrap();
        }
        left
    };
    let input = noise(40_000);
    run(&mut running, &input, 120.0);
    run(&mut roomy, &input, 60.0);
    let mut edited = built(60.0);
    edited.adopt_effects(0, &mut running, 0, |_| {});
    // A block at 60 BPM, then the Stop.
    let input = noise(1_000);
    for playback in [&mut edited, &mut roomy] {
        run(playback, &input, 60.0);
        playback
            .effect_mut(0, 2)
            .unwrap()
            .set_parameter(5, 0.0)
            .unwrap();
    }
    // 16 beats at 60 BPM: 128 000 frames, more than the 120 BPM ring's 65 536.
    let input = noise(140_000);
    assert!(run(&mut edited, &input, 60.0) == run(&mut roomy, &input, 60.0));
}

/// An edit's playback takes over a running effect of the same instance and type, its state with it, and the effect
/// it built goes back. A Tape Stop is taken over at a new, slower tempo too (no edit resets effect tails), and takes
/// the ring built for that tempo.
#[test]
fn adopt_effects_takes_the_running_effect_a_tape_stop_at_a_slower_tempo_too() {
    const RATE: f32 = 48_000.0;
    let context = ProcessContext {
        sample_rate: RATE,
        tempo_bpm: 120.0,
        ..CONTEXT
    };
    let built = |tempo: f32| {
        let mut playback = AudioPlayback::new(1, AutomationPlayback::default());
        for (id, type_id) in [(2, 42), (3, 32)] {
            let mut effect = AudioEffect::new(type_id, RATE).unwrap();
            effect.configure_tempo(tempo).unwrap();
            playback.add_effect(0, id, effect).unwrap();
        }
        playback
    };
    let mut running = built(120.0);
    let (mut left, mut right) = (vec![0.5; 64], vec![0.5; 64]);
    running
        .process_channel(0, &mut left, &mut right, context)
        .unwrap();
    let heard =
        |playback: &AudioPlayback, id| playback.effect(0, id).unwrap().meter().input_peak > 0.0;
    for tempo in [120.0, 60.0] {
        let mut old = built(120.0);
        old.process_channel(0, &mut left, &mut right, context)
            .unwrap();
        let mut new = built(tempo);
        let mut taken = Vec::new();
        new.adopt_effects(0, &mut old, 0, |id| taken.push(id));
        assert!(heard(&new, 2), "the Tape Stop, tempo {tempo}");
        assert!(heard(&new, 3), "the Reverb, tempo {tempo}");
        assert_eq!(taken, [2, 3], "tempo {tempo}");
    }
    assert!(heard(&running, 2));
}
