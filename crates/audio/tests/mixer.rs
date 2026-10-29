use honeydew_audio::mixer::{ChannelMix, MixError, Mixer, OwnedStereoBuffer, SendMix};
use honeydew_audio::playback::StereoBlock;

/// The send level of the helpers: +6 dB, so products stay exact powers of `full`.
const SEND: f32 = 1.0;

/// A channel at `level` and `pan` sending to each of `targets`, enabled, at [`SEND`].
fn channel(targets: &[usize], level: f32, pan: f32) -> ChannelMix {
    ChannelMix {
        level,
        pan,
        sends: targets
            .iter()
            .map(|&target| SendMix {
                target,
                enabled: 1.0,
                level: SEND,
            })
            .collect(),
        keys: Vec::new(),
    }
}

/// The master: no sends.
fn master(level: f32, pan: f32) -> ChannelMix {
    channel(&[], level, pan)
}

/// The channel Level law: dB = 66·v^0.6 − 60.
fn law(level: f32) -> f32 {
    10.0_f32.powf((66.0 * level.powf(0.6) - 60.0) / 20.0)
}

fn close(actual: f32, expected: f32) {
    assert!((actual - expected).abs() < 1e-4, "{actual} != {expected}");
}

#[test]
fn level_pan_and_master_gain_apply_to_stereo_mix() {
    let mixer = Mixer::new(&[
        master(0.5, 0.5),
        channel(&[0], 0.5, 0.0),
        channel(&[0], 0.25, 1.0),
    ])
    .unwrap();
    let mut master_left = [0.0];
    let mut master_right = [0.0];
    let mut first_left = [1.0];
    let mut first_right = [1.0];
    let mut second_left = [1.0];
    let mut second_right = [1.0];
    mixer
        .mix(
            &mut [
                StereoBlock {
                    left: &mut master_left,
                    right: &mut master_right,
                },
                StereoBlock {
                    left: &mut first_left,
                    right: &mut first_right,
                },
                StereoBlock {
                    left: &mut second_left,
                    right: &mut second_right,
                },
            ],
            |_, _, _| Ok::<_, ()>(()),
        )
        .unwrap();
    let half = law(0.5);
    let quarter = law(0.25);
    let send = law(SEND);
    close(master_left[0], half * half * send);
    close(master_right[0], half * quarter * send);
    close(first_left[0], half);
    close(first_right[0], 0.0);
    close(second_left[0], 0.0);
    close(second_right[0], quarter);
}

#[test]
fn nested_bus_effects_run_after_inputs_and_master_runs_last() {
    let mixer = Mixer::new(&[
        master(1.0, 0.5),
        channel(&[0], 1.0, 0.5),
        channel(&[1], 1.0, 0.5),
        channel(&[2], 1.0, 0.5),
    ])
    .unwrap();
    let mut left = [[0.0], [0.0], [0.0], [1.0]];
    let mut right = [[0.0], [0.0], [0.0], [1.0]];
    let mut calls = Vec::new();
    let [l0, l1, l2, l3] = &mut left;
    let [r0, r1, r2, r3] = &mut right;
    mixer
        .mix(
            &mut [
                StereoBlock {
                    left: l0,
                    right: r0,
                },
                StereoBlock {
                    left: l1,
                    right: r1,
                },
                StereoBlock {
                    left: l2,
                    right: r2,
                },
                StereoBlock {
                    left: l3,
                    right: r3,
                },
            ],
            |index, left, right| {
                calls.push(index);
                if index < 3 {
                    left[0] *= 2.0;
                    right[0] *= 2.0;
                }
                Ok::<_, ()>(())
            },
        )
        .unwrap();
    assert_eq!(calls, [3, 2, 1, 0]);
    // Four strips and three sends, each +6 dB.
    let full = 10.0_f32.powf(6.0 / 20.0);
    close(left[0][0], 8.0 * full.powi(7));
    close(right[0][0], 8.0 * full.powi(7));
}

#[test]
fn channels_on_both_sides_of_a_bus_sum_before_bus_processing() {
    let mixer = Mixer::new(&[
        master(1.0, 0.5),
        channel(&[2], 1.0, 0.5),
        channel(&[0], 1.0, 0.5),
        channel(&[2], 1.0, 0.5),
    ])
    .unwrap();
    let mut left = [[0.0], [1.0], [0.0], [2.0]];
    let mut right = [[0.0], [1.0], [0.0], [2.0]];
    let [l0, l1, l2, l3] = &mut left;
    let [r0, r1, r2, r3] = &mut right;
    mixer
        .mix(
            &mut [
                StereoBlock {
                    left: l0,
                    right: r0,
                },
                StereoBlock {
                    left: l1,
                    right: r1,
                },
                StereoBlock {
                    left: l2,
                    right: r2,
                },
                StereoBlock {
                    left: l3,
                    right: r3,
                },
            ],
            |index, left, right| {
                if index == 2 {
                    let full = 10.0_f32.powf(6.0 / 20.0);
                    close(left[0], 3.0 * full * full);
                    close(right[0], 3.0 * full * full);
                    left[0] *= 2.0;
                    right[0] *= 2.0;
                }
                Ok::<_, ()>(())
            },
        )
        .unwrap();
    let full = 10.0_f32.powf(6.0 / 20.0);
    close(left[0][0], 6.0 * full.powi(5));
    close(right[0][0], 6.0 * full.powi(5));
}

#[test]
fn invalid_routes_and_controls_are_rejected() {
    let main = || master(1.0, 0.5);
    // To no channel, to itself, and from the master.
    assert!(matches!(
        Mixer::new(&[main(), channel(&[8], 1.0, 0.5)]),
        Err(MixError::InvalidRoute(1))
    ));
    assert!(matches!(
        Mixer::new(&[main(), channel(&[1], 1.0, 0.5)]),
        Err(MixError::InvalidRoute(1))
    ));
    assert!(matches!(
        Mixer::new(&[channel(&[1], 1.0, 0.5), channel(&[0], 1.0, 0.5)]),
        Err(MixError::InvalidRoute(0))
    ));
    assert!(matches!(
        Mixer::new(&[main(), channel(&[2], 1.0, 0.5), channel(&[1], 1.0, 0.5)]),
        Err(MixError::CyclicRoute)
    ));
    assert!(matches!(
        Mixer::new(&[main(), channel(&[0], f32::NAN, 0.5)]),
        Err(MixError::InvalidControl(1))
    ));
    let mut nan = channel(&[0], 1.0, 0.5);
    nan.sends[0].level = f32::NAN;
    assert!(matches!(
        Mixer::new(&[main(), nan]),
        Err(MixError::InvalidControl(1))
    ));
}

#[test]
fn invalid_buffers_do_not_run_processors_or_change_audio() {
    let mixer = Mixer::new(&[master(1.0, 0.5), channel(&[0], 1.0, 0.5)]).unwrap();
    let mut master_left = [0.0, 0.0];
    let mut master_right = [0.0, 0.0];
    let mut source_left = [1.0];
    let mut source_right = [1.0];
    let mut called = false;
    assert_eq!(
        mixer.mix(
            &mut [
                StereoBlock {
                    left: &mut master_left,
                    right: &mut master_right,
                },
                StereoBlock {
                    left: &mut source_left,
                    right: &mut source_right,
                },
            ],
            |_, _, _| {
                called = true;
                Ok::<_, ()>(())
            },
        ),
        Err(MixError::MismatchedBuffers)
    );
    assert!(!called);
    assert_eq!((source_left, source_right), ([1.0], [1.0]));
}

#[test]
fn control_updates_do_not_change_routes() {
    let mut mixer = Mixer::new(&[master(1.0, 0.5), channel(&[0], 1.0, 0.5)]).unwrap();
    assert_eq!(
        mixer.set_channel_controls(1, f32::NAN, 0.5),
        Err(MixError::InvalidControl(1))
    );
    mixer.set_channel_controls(1, 0.25, 0.0).unwrap();
    let mut master_left = [0.0];
    let mut master_right = [0.0];
    let mut bus_left = [1.0];
    let mut bus_right = [1.0];
    mixer
        .mix(
            &mut [
                StereoBlock {
                    left: &mut master_left,
                    right: &mut master_right,
                },
                StereoBlock {
                    left: &mut bus_left,
                    right: &mut bus_right,
                },
            ],
            |_, _, _| Ok::<_, ()>(()),
        )
        .unwrap();
    close(master_left[0], law(0.25) * law(SEND) * law(1.0));
    close(master_right[0], 0.0);
}

#[test]
fn owned_buffers_route_without_borrowed_blocks() {
    let mixer = Mixer::new(&[master(1.0, 0.5), channel(&[0], 1.0, 0.5)]).unwrap();
    let mut buffers = [
        OwnedStereoBuffer {
            left: vec![0.0],
            right: vec![0.0],
        },
        OwnedStereoBuffer {
            left: vec![1.0],
            right: vec![2.0],
        },
    ];
    mixer.mix(&mut buffers, |_, _, _| Ok::<_, ()>(())).unwrap();
    let full = 10.0_f32.powf(6.0 / 20.0);
    close(buffers[0].left[0], full.powi(3));
    close(buffers[0].right[0], 2.0 * full.powi(3));
}

#[test]
fn default_level_is_near_unity_and_zero_is_silent() {
    let mut buffers = [
        OwnedStereoBuffer {
            left: vec![0.0],
            right: vec![0.0],
        },
        OwnedStereoBuffer {
            left: vec![1.0],
            right: vec![1.0],
        },
    ];
    let mut unity = channel(&[0], 0.8531, 0.5);
    unity.sends[0].level = 0.8531;
    let default = Mixer::new(&[master(0.8531, 0.5), unity]).unwrap();
    default
        .mix(&mut buffers, |_, _, _| Ok::<_, ()>(()))
        .unwrap();
    assert!((0.98..=1.02).contains(&buffers[0].left[0]));
    buffers[0].left[0] = 0.0;
    buffers[0].right[0] = 0.0;
    buffers[1].left[0] = 1.0;
    buffers[1].right[0] = 1.0;
    let silent = Mixer::new(&[master(1.0, 0.5), channel(&[0], 0.0, 0.5)]).unwrap();
    silent.mix(&mut buffers, |_, _, _| Ok::<_, ()>(())).unwrap();
    assert_eq!((buffers[0].left[0], buffers[0].right[0]), (0.0, 0.0));
}

#[test]
fn level_follows_the_channel_law_between_its_anchors() {
    for level in [0.6, 0.8531, 0.95] {
        let mixer = Mixer::new(&[master(1.0, 0.5), channel(&[0], level, 0.5)]).unwrap();
        let mut buffers = [
            OwnedStereoBuffer {
                left: vec![0.0],
                right: vec![0.0],
            },
            OwnedStereoBuffer {
                left: vec![1.0],
                right: vec![1.0],
            },
        ];
        mixer.mix(&mut buffers, |_, _, _| Ok::<_, ()>(())).unwrap();
        close(buffers[1].left[0], law(level));
    }
}

/// Master, channel 1 and a send channel 2 in owned one-frame buffers, channels 1 and 2 holding 1.0.
fn buffers() -> [OwnedStereoBuffer; 3] {
    [0.0, 1.0, 1.0].map(|v| OwnedStereoBuffer {
        left: vec![v],
        right: vec![v],
    })
}

#[test]
fn a_channel_feeds_each_send_at_its_level_after_its_strip() {
    // Channel 1 at 0.6: to the master at 0.5 and to channel 2 at 0.25; channel 2 at unity strip, sending at SEND.
    let mut one = channel(&[0, 2], 0.6, 0.5);
    (one.sends[0].level, one.sends[1].level) = (0.5, 0.25);
    let mixer = Mixer::new(&[master(1.0, 0.5), one, channel(&[0], 0.8531, 0.5)]).unwrap();
    let mut calls = Vec::new();
    let mut b = buffers();
    mixer
        .mix(&mut b, |index, _, _| {
            calls.push(index);
            Ok::<_, ()>(())
        })
        .unwrap();
    assert_eq!(calls, [1, 2, 0], "the target after its source");
    let strip = law(0.6);
    // Channel 2 held 1.0 itself and took channel 1's strip output at 0.25.
    let two = (1.0 + strip * law(0.25)) * law(0.8531);
    close(b[2].left[0], two);
    close(
        b[0].left[0],
        (strip * law(0.5) + two * law(SEND)) * law(1.0),
    );
}

#[test]
fn a_send_mixes_only_while_enable_is_above_one_half() {
    for (enabled, expected) in [(0.0, 0.0), (0.5, 0.0), (0.5001, 1.0), (1.0, 1.0)] {
        let mut one = channel(&[0], 0.8531, 0.5);
        one.sends[0].enabled = enabled;
        let mixer = Mixer::new(&[master(0.8531, 0.5), one, master(0.8531, 0.5)]).unwrap();
        let mut b = buffers();
        mixer.mix(&mut b, |_, _, _| Ok::<_, ()>(())).unwrap();
        let gain = law(0.8531).powi(2) * law(SEND);
        close(b[0].left[0], expected * gain);
    }
}

#[test]
fn a_channel_without_sends_has_no_output() {
    let mixer = Mixer::new(&[master(1.0, 0.5), master(1.0, 0.5), master(1.0, 0.5)]).unwrap();
    let mut b = buffers();
    mixer.mix(&mut b, |_, _, _| Ok::<_, ()>(())).unwrap();
    assert_eq!(b[0].left[0], 0.0);
}

#[test]
fn send_enable_and_level_change_live() {
    let mut mixer =
        Mixer::new(&[master(1.0, 0.5), channel(&[0], 1.0, 0.5), master(1.0, 0.5)]).unwrap();
    let output = |mixer: &Mixer| {
        let mut b = buffers();
        mixer.mix(&mut b, |_, _, _| Ok::<_, ()>(())).unwrap();
        b[0].left[0]
    };
    mixer.set_send(1, 0, true, 0.25).unwrap();
    close(output(&mixer), law(1.0) * law(0.25) * law(1.0));
    mixer.set_send(1, 0, false, 0.0).unwrap();
    assert_eq!(output(&mixer), 0.0);
    // No such send, or a bad value: refused, nothing changes.
    assert_eq!(
        mixer.set_send(1, 2, true, 0.5),
        Err(MixError::InvalidRoute(1))
    );
    assert_eq!(
        mixer.set_send(1, 0, false, 2.0),
        Err(MixError::InvalidControl(1))
    );
    assert_eq!(output(&mixer), 0.0);
}
