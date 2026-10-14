use honeydew_audio::{AudioEffect, EffectError, ProcessContext, effect_info};

const CTX: ProcessContext = ProcessContext {
    sample_rate: 48_000.0,
    tempo_bpm: 120.0,
    start_beat: 0.0,
    playing: true,
};

fn settle_common(effect: &mut AudioEffect) {
    let mut left = [0.0; 2048];
    let mut right = [0.0; 2048];
    effect.process(&mut left, &mut right, CTX).unwrap();
    effect.reset();
}

#[test]
fn registry_covers_and_constructs_audio_effects() {
    let ids = [
        3, 4, 5, 6, 7, 8, 9, 10, 11, 12, 13, 14, 15, 16, 18, 20, 24, 25, 26, 27, 28, 29, 30, 32,
        34, 35, 36, 40, 41, 42, 43, 44, 45, 46,
    ];
    for id in ids {
        assert!(effect_info(id).is_some(), "missing ID {id}");
        assert!(
            AudioEffect::new(id, CTX.sample_rate).is_ok(),
            "unavailable ID {id}"
        );
    }
    assert!(effect_info(99).is_none());
}

#[test]
fn reverb_variants_process_through_the_public_effect_api() {
    for id in [12, 32, 35] {
        let mut effect = AudioEffect::new(id, CTX.sample_rate).unwrap();
        let mut left = vec![0.0; 24_000];
        let mut right = vec![0.0; 24_000];
        left[0] = 1.0;
        effect.process(&mut left, &mut right, CTX).unwrap();
        assert!(
            left[2_000..].iter().any(|sample| sample.abs() > 1e-6),
            "ID {id}"
        );
        assert!(
            right[2_000..].iter().any(|sample| sample.abs() > 1e-6),
            "ID {id}"
        );
    }
}

#[test]
fn tape_stop_play_toggle_moves_the_public_audio_path() {
    let mut effect = AudioEffect::new(42, CTX.sample_rate).unwrap();
    let mut live_left = [0.8; 256];
    let mut live_right = [-0.4; 256];
    effect
        .process(&mut live_left, &mut live_right, CTX)
        .unwrap();
    assert_eq!(live_left, [0.8; 256]);
    assert_eq!(live_right, [-0.4; 256]);

    effect.set_parameter(5, 0.0).unwrap();
    let mut stopped_left = [0.8; 72_000];
    let mut stopped_right = [-0.4; 72_000];
    effect
        .process(&mut stopped_left, &mut stopped_right, CTX)
        .unwrap();
    assert!(stopped_left[71_999].abs() < 0.1);
    assert!(stopped_right[71_999].abs() < 0.1);
}

#[test]
fn spreader_width_routes_audio_through_the_public_effect_api() {
    let mut effect = AudioEffect::new(44, CTX.sample_rate).unwrap();
    effect.set_parameter(1, 1.0).unwrap();
    effect.set_parameter(2, 0.5).unwrap();
    let mut left = [0.0; 4096];
    let mut right = [0.0; 4096];
    left[0] = 1.0;
    effect.process(&mut left, &mut right, CTX).unwrap();
    assert!(right.iter().map(|sample| sample.abs()).sum::<f32>() > 0.001);
}

#[test]
fn multiband_solo_routes_only_the_selected_band() {
    let mut low = AudioEffect::new(36, CTX.sample_rate).unwrap();
    low.set_multiband_solo_band(Some(0)).unwrap();
    let mut high = AudioEffect::new(36, CTX.sample_rate).unwrap();
    high.set_multiband_solo_band(Some(2)).unwrap();
    let mut low_left = [0.0; 4096];
    let mut low_right = [0.0; 4096];
    let mut high_left = [0.0; 4096];
    let mut high_right = [0.0; 4096];
    for i in 0..4096 {
        let sample = (std::f32::consts::TAU * 80.0 * i as f32 / CTX.sample_rate).sin() * 0.5;
        low_left[i] = sample;
        low_right[i] = sample;
        high_left[i] = sample;
        high_right[i] = sample;
    }
    low.process(&mut low_left, &mut low_right, CTX).unwrap();
    high.process(&mut high_left, &mut high_right, CTX).unwrap();
    let low_energy: f32 = low_left.iter().map(|sample| sample * sample).sum();
    let high_energy: f32 = high_left.iter().map(|sample| sample * sample).sum();
    assert!(low_energy > 10.0 * high_energy);
}

#[test]
fn pitcher_reports_latency_and_delays_its_dry_path() {
    let mut effect = AudioEffect::new(25, 8_000.0).unwrap();
    effect
        .set_pitcher_notes(std::array::from_fn(|note| note == 0))
        .unwrap();
    effect.set_parameter(9, 0.0).unwrap();
    let latency = effect.latency_samples();
    let mut left = vec![0.0; latency + 1];
    let mut right = left.clone();
    left[0] = 1.0;
    effect
        .process(
            &mut left,
            &mut right,
            ProcessContext {
                sample_rate: 8_000.0,
                tempo_bpm: 120.0,
                start_beat: 0.0,
                playing: true,
            },
        )
        .unwrap();
    assert!(left[..latency].iter().all(|sample| sample.abs() < 1e-6));
    assert!((left[latency] - 1.0).abs() < 1e-6);
    assert!(AudioEffect::new(25, 1_000.0).is_err());
}

#[test]
fn pitcher2_reports_mode_latency_and_routes_delayed_dry_audio() {
    let mut effect = AudioEffect::new(45, 48_000.0).unwrap();
    assert_eq!(effect.latency_samples(), 2674);
    effect.set_parameter(25, 1.0).unwrap();
    assert_eq!(effect.latency_samples(), 1336);
    effect.set_parameter(22, 0.0).unwrap();
    let latency = effect.latency_samples();
    let mut left = vec![0.0; latency + 1];
    let mut right = left.clone();
    left[0] = 1.0;
    effect.process(&mut left, &mut right, CTX).unwrap();
    assert!(left[..latency].iter().all(|sample| sample.abs() < 1e-6));
    assert_eq!(left[latency], 1.0);
}

#[test]
fn waveshaper_shape_menu_changes_the_public_audio_path() {
    let mut analog = AudioEffect::new(41, CTX.sample_rate).unwrap();
    assert!((analog.parameter(6).unwrap() - 1.0 / 6.0).abs() < 1e-6);
    analog.set_parameter(6, 0.5).unwrap();
    analog.set_parameter(1, 1.0 / 6.0).unwrap();
    let mut fold = AudioEffect::new(41, CTX.sample_rate).unwrap();
    fold.set_parameter(6, 0.5).unwrap();
    fold.set_parameter(1, 0.5).unwrap();
    let mut analog_left = [0.2; 512];
    let mut analog_right = [0.2; 512];
    let mut fold_left = analog_left;
    let mut fold_right = analog_right;
    analog
        .process(&mut analog_left, &mut analog_right, CTX)
        .unwrap();
    fold.process(&mut fold_left, &mut fold_right, CTX).unwrap();
    assert!((analog_left[511] - fold_left[511]).abs() > 0.01);
}

#[test]
fn leveller_gain_and_pan_change_the_audio() {
    let mut effect = AudioEffect::new(20, CTX.sample_rate).unwrap();
    settle_common(&mut effect);
    let mut left = [1.0; 2048];
    let mut right = [1.0; 2048];
    effect.process(&mut left, &mut right, CTX).unwrap();
    assert!((left[0] - 1.0).abs() < 0.02);
    assert!((right[0] - 1.0).abs() < 0.02);

    effect.set_parameter(2, 0.0).unwrap();
    effect.process(&mut left, &mut right, CTX).unwrap();
    assert!(left[2047] > 0.95);
    assert!(right[2047] < 0.05);

    effect.set_parameter(1, 0.0).unwrap();
    left.fill(1.0);
    right.fill(1.0);
    effect.process(&mut left, &mut right, CTX).unwrap();
    assert!((left[2047] - 0.001).abs() < 0.001);
}

#[test]
fn flat_equalizers_leave_audio_unchanged() {
    for id in [3, 7, 30] {
        let mut effect = AudioEffect::new(id, CTX.sample_rate).unwrap();
        settle_common(&mut effect);
        let mut left = [0.3, -0.8, 0.0, 1.0];
        let mut right = [-0.2, 0.7, -1.0, 0.1];
        let old_left = left;
        let old_right = right;
        effect.process(&mut left, &mut right, CTX).unwrap();
        assert_eq!(left, old_left, "effect {id}");
        assert_eq!(right, old_right, "effect {id}");
    }
}

#[test]
fn parametric_equalizer_can_target_only_the_left_channel() {
    let mut effect = AudioEffect::new(3, CTX.sample_rate).unwrap();
    settle_common(&mut effect);
    effect.set_parameter(1, 0.75).unwrap();
    effect.set_parameter(4, 0.5).unwrap();
    effect.set_parameter(17, 0.25).unwrap();
    let mut left = [0.0; 128];
    let mut right = [0.0; 128];
    left[0] = 1.0;
    right[0] = 1.0;
    effect.process(&mut left, &mut right, CTX).unwrap();
    assert_ne!(left[0], 1.0);
    assert_eq!(right[0], 1.0);
    assert!(right[1..].iter().all(|sample| *sample == 0.0));
}

#[test]
fn equalizer_channel_and_kind_truncate_between_steps() {
    // Channel trunc(4v), Kind trunc(2v); Channel 0.2 is Both, Kind 0.3
    // LowShelf (the panel shows Left and Peak there).
    let impulse = |channel: f32, kind: f32| {
        let mut effect = AudioEffect::new(3, CTX.sample_rate).unwrap();
        settle_common(&mut effect);
        effect.set_parameter(1, 0.75).unwrap();
        effect.set_parameter(4, kind).unwrap();
        effect.set_parameter(17, channel).unwrap();
        let mut left = [0.0; 128];
        let mut right = [0.0; 128];
        left[0] = 1.0;
        right[0] = 1.0;
        effect.process(&mut left, &mut right, CTX).unwrap();
        (left, right)
    };
    assert_eq!(impulse(0.2, 0.5), impulse(0.0, 0.5));
    assert_ne!(impulse(0.25, 0.5), impulse(0.0, 0.5));
    assert_eq!(impulse(0.0, 0.3), impulse(0.0, 0.0));
    assert_ne!(impulse(0.0, 0.5), impulse(0.0, 0.0));
}

#[test]
fn filter_kind_changes_which_frequencies_survive() {
    let mut low = AudioEffect::new(9, CTX.sample_rate).unwrap();
    settle_common(&mut low);
    low.set_parameter(1, 0.35).unwrap();
    let mut high = AudioEffect::new(9, CTX.sample_rate).unwrap();
    settle_common(&mut high);
    high.set_parameter(1, 0.35).unwrap();
    high.set_parameter(3, 1.0).unwrap();

    let mut low_dc = [1.0; 2048];
    let mut low_nyquist = [0.0; 2048];
    for (i, sample) in low_nyquist.iter_mut().enumerate() {
        *sample = if i % 2 == 0 { 1.0 } else { -1.0 };
    }
    let mut l = low_dc;
    low.process(&mut low_dc, &mut l, CTX).unwrap();
    let mut r = low_nyquist;
    low.reset();
    low.process(&mut low_nyquist, &mut r, CTX).unwrap();
    assert!((low_dc[2047] - 0.6813).abs() < 0.01);
    assert!(low_nyquist[2047].abs() < 0.1);

    let mut high_dc = [1.0; 2048];
    let mut high_r = high_dc;
    high.process(&mut high_dc, &mut high_r, CTX).unwrap();
    assert!(high_dc[2047].abs() < 0.1);
}

#[test]
fn filter_pairing_delays_one_sample_per_slope_pass() {
    for (slope, passes) in [(0.0, 1), (0.5, 2), (1.0, 4)] {
        let mut effect = AudioEffect::new(9, CTX.sample_rate).unwrap();
        effect.set_parameter(4, slope).unwrap();
        settle_common(&mut effect);
        let mut left = [0.0; 32];
        let mut right = [0.0; 32];
        left[0] = 1.0;
        effect.process(&mut left, &mut right, CTX).unwrap();
        assert!(left[..passes].iter().all(|sample| *sample == 0.0));
        assert!(left[passes] > 0.0);
        assert!(right.iter().all(|sample| *sample == 0.0));
    }
}

#[test]
fn legacy_autoduck_follows_beat_phase_and_depth() {
    let mut effect = AudioEffect::new(10, CTX.sample_rate).unwrap();
    effect.set_parameter(1, 1.0).unwrap();
    settle_common(&mut effect);
    let mut left = [1.0];
    let mut right = [0.5];
    effect
        .process(
            &mut left,
            &mut right,
            ProcessContext {
                start_beat: 0.25,
                ..CTX
            },
        )
        .unwrap();
    assert!((left[0] - 0.1619).abs() < 0.003);
    assert!((right[0] - 0.08095).abs() < 0.003);
}

#[test]
fn autoduck_shape_and_transport_phase_change_gain() {
    let mut accel = AudioEffect::new(29, CTX.sample_rate).unwrap();
    settle_common(&mut accel);
    let mut left = [1.0];
    let mut right = [1.0];
    accel
        .process(
            &mut left,
            &mut right,
            ProcessContext {
                start_beat: 0.25,
                ..CTX
            },
        )
        .unwrap();
    assert!((left[0] - 0.4068).abs() < 0.003);
    assert_eq!(left[0], right[0]);

    let mut s_curve = AudioEffect::new(29, CTX.sample_rate).unwrap();
    s_curve.set_parameter(7, 1.0).unwrap();
    settle_common(&mut s_curve);
    let mut curve_left = [1.0];
    let mut curve_right = [1.0];
    s_curve
        .process(
            &mut curve_left,
            &mut curve_right,
            ProcessContext {
                start_beat: 0.25,
                ..CTX
            },
        )
        .unwrap();
    assert!((curve_left[0] - left[0]).abs() > 0.02);
}

#[test]
fn monitor_effects_do_not_change_the_audio() {
    for id in [24, 26] {
        let mut effect = AudioEffect::new(id, CTX.sample_rate).unwrap();
        settle_common(&mut effect);
        let mut left = [0.3, -0.7, 0.0, 0.9];
        let mut right = [-0.4, 0.5, -0.1, 0.0];
        let original_left = left;
        let original_right = right;
        effect.process(&mut left, &mut right, CTX).unwrap();
        assert_eq!(left, original_left);
        assert_eq!(right, original_right);
        assert_eq!(effect.meter().peak_left, 0.9);
        assert_eq!(effect.meter().peak_right, 0.5);
    }
}

#[test]
fn tremolo_uses_amount_limits_and_stereo_phase() {
    let mut effect = AudioEffect::new(46, CTX.sample_rate).unwrap();
    effect.set_parameter(6, 0.25).unwrap();
    let mut left = vec![1.0; 48_000];
    let mut right = vec![1.0; 48_000];
    effect.process(&mut left, &mut right, CTX).unwrap();
    assert!(left[12_000] > 0.35 && left[12_000] < 0.65);
    assert!(right[12_000] > 0.85);
    assert!(left[24_000] > 0.85);
    assert!(right[24_000] < 0.65);

    effect.set_parameter(3, 1.0).unwrap();
    effect.set_parameter(4, 1.0).unwrap();
    left.fill(1.0);
    right.fill(1.0);
    effect.process(&mut left, &mut right, CTX).unwrap();
    assert!((left[47_999] - 1.0).abs() < 0.002);
    assert!((right[47_999] - 1.0).abs() < 0.002);
}

#[test]
fn synced_tremolo_follows_transport_beat_phase() {
    let mut effect = AudioEffect::new(46, CTX.sample_rate).unwrap();
    effect.set_parameter(2, 5.0 / 9.0).unwrap();
    effect.set_parameter(7, 1.0).unwrap();
    let mut left = vec![1.0; 48_000];
    let mut right = vec![1.0; 48_000];
    effect.process(&mut left, &mut right, CTX).unwrap();
    assert!(left[12_000] > 0.85);
    assert!(left[24_000] < 0.2);
    assert!(left[36_000] > 0.85);
}

#[test]
fn phaser_uses_six_allpass_stages_and_adds_wet_output() {
    let mut effect = AudioEffect::new(14, CTX.sample_rate).unwrap();
    effect.set_parameter(2, 0.0).unwrap();
    effect.set_parameter(3, 0.0).unwrap();
    effect.set_parameter(4, 0.25).unwrap();
    settle_common(&mut effect);
    let low = 50.0 / CTX.sample_rate;
    let high = 16_000.0 / CTX.sample_rate;
    let frequency = low + 0.25_f32.powi(2) * (high - low);
    let coefficient = (1.0 - frequency) / (1.0 + frequency);
    let expected = 1.0 + coefficient.powi(6);
    let mut left = [1.0, 0.0];
    let mut right = [1.0, 0.0];
    effect.process(&mut left, &mut right, CTX).unwrap();
    assert!(
        (left[0] - expected).abs() < 1e-5,
        "{} != {}",
        left[0],
        expected
    );
    assert_eq!(left, right);
    assert!(left[1] < 0.0);
}

#[test]
fn phaser_stereo_offset_changes_the_right_sweep() {
    let mut effect = AudioEffect::new(14, CTX.sample_rate).unwrap();
    effect.set_parameter(2, 1.0).unwrap();
    effect.set_parameter(4, 0.5).unwrap();
    effect.set_parameter(8, 0.5).unwrap();
    effect.set_parameter(9, 1.0).unwrap();
    settle_common(&mut effect);
    let mut left = [1.0];
    let mut right = [1.0];
    effect.process(&mut left, &mut right, CTX).unwrap();
    assert!((left[0] - right[0]).abs() > 0.01);
}

#[test]
fn stereoizer_sends_a_delayed_mono_component_to_opposite_sides() {
    let mut effect = AudioEffect::new(15, CTX.sample_rate).unwrap();
    effect.set_parameter(1, 1.0).unwrap();
    effect.set_parameter(2, 0.0).unwrap();
    effect.set_parameter(3, 0.5).unwrap();
    settle_common(&mut effect);
    let mut left = [0.0; 64];
    let mut right = [0.0; 64];
    left[0] = 1.0;
    right[0] = 1.0;
    effect.process(&mut left, &mut right, CTX).unwrap();
    assert!(left[1..24].iter().all(|sample| sample.abs() < 1e-6));
    assert!(right[1..24].iter().all(|sample| sample.abs() < 1e-6));
    assert!((left[24] - 2.0).abs() < 1e-5);
    assert!((right[24] + 2.0).abs() < 1e-5);
}

#[test]
fn stereoizer_channel_delay_offsets_one_side() {
    let mut effect = AudioEffect::new(15, CTX.sample_rate).unwrap();
    effect.set_parameter(1, 0.0).unwrap();
    effect.set_parameter(4, 1.0).unwrap();
    settle_common(&mut effect);
    let mut left = vec![0.0; 1300];
    let mut right = vec![0.0; 1300];
    left[0] = 1.0;
    right[0] = 1.0;
    effect.process(&mut left, &mut right, CTX).unwrap();
    assert_eq!(left[0], 1.0);
    assert_eq!(right[0], 0.0);
    assert!(right.iter().position(|sample| *sample > 0.9).unwrap() > 500);
}

#[test]
fn spacer_default_preserves_stereo_and_output_ms_mode_converts_it() {
    let mut effect = AudioEffect::new(34, CTX.sample_rate).unwrap();
    settle_common(&mut effect);
    let mut left = [1.0, 0.25];
    let mut right = [-1.0, 0.75];
    effect.process(&mut left, &mut right, CTX).unwrap();
    assert!((left[0] - 1.0).abs() < 1e-6);
    assert!((right[0] + 1.0).abs() < 1e-6);
    effect.set_parameter(7, 0.5).unwrap();
    effect.reset();
    let mut left = [1.0];
    let mut right = [-1.0];
    effect.process(&mut left, &mut right, CTX).unwrap();
    assert!(left[0].abs() < 1e-6);
    assert!((right[0] - 1.0).abs() < 1e-6);
}

#[test]
fn spacer_separation_can_collapse_stereo_to_mid() {
    let mut effect = AudioEffect::new(34, CTX.sample_rate).unwrap();
    effect.set_parameter(3, 1.0).unwrap();
    let mut silence = vec![0.0; 16_000];
    let mut other = silence.clone();
    effect.process(&mut silence, &mut other, CTX).unwrap();
    effect.reset();
    let mut left = [1.0];
    let mut right = [0.0];
    effect.process(&mut left, &mut right, CTX).unwrap();
    assert!((left[0] - 0.5).abs() < 5e-5, "{}", left[0]);
    assert!((right[0] - 0.5).abs() < 5e-5, "{}", right[0]);
}

#[test]
fn gate_opens_for_sustained_audio_and_reports_detector_latency() {
    let mut effect = AudioEffect::new(27, CTX.sample_rate).unwrap();
    effect.set_parameter(1, 0.8).unwrap();
    effect.set_parameter(3, 0.0).unwrap();
    effect.set_parameter(8, 0.0).unwrap();
    settle_common(&mut effect);
    assert_eq!(effect.latency_samples(), 255);
    let mut left = vec![1.0; 2048];
    let mut right = left.clone();
    effect.process(&mut left, &mut right, CTX).unwrap();
    assert!(left[..255].iter().all(|sample| sample.abs() < 1e-6));
    assert!(left[1024] > 0.9);
    assert!(right[1024] > 0.9);
}

#[test]
fn gate_mix_polarity_uses_the_complementary_envelope() {
    let mut effect = AudioEffect::new(27, CTX.sample_rate).unwrap();
    effect.set_parameter(1, 0.8).unwrap();
    effect.set_parameter(3, 0.0).unwrap();
    effect.set_parameter(7, 0.0).unwrap();
    settle_common(&mut effect);
    let mut left = vec![1.0; 2048];
    let mut right = left.clone();
    effect.process(&mut left, &mut right, CTX).unwrap();
    assert!(left[1024] < 0.01);
    assert!(right[1024] < 0.01);
}

#[test]
fn tape_delay_repeats_at_its_squared_time_setting() {
    let mut effect = AudioEffect::new(18, CTX.sample_rate).unwrap();
    effect.set_parameter(1, 0.1).unwrap();
    effect.set_parameter(2, 0.5).unwrap();
    effect.set_parameter(4, 1.0).unwrap();
    settle_common(&mut effect);
    let mut left = vec![0.0; 1100];
    let mut right = left.clone();
    left[0] = 1.0;
    right[0] = 1.0;
    effect.process(&mut left, &mut right, CTX).unwrap();
    assert!((left[0] - 1.0).abs() < 1e-6);
    assert!(left[480..485].iter().copied().fold(0.0_f32, f32::max) > 0.2);
    assert!(left[960..970].iter().copied().fold(0.0_f32, f32::max) > 0.05);
    assert_eq!(left, right);
}

#[test]
fn flanger_inverted_mix_creates_a_short_negative_echo() {
    let mut effect = AudioEffect::new(13, CTX.sample_rate).unwrap();
    effect.set_parameter(5, 0.0).unwrap();
    effect.set_parameter(6, 0.4).unwrap();
    effect.set_parameter(7, 0.0).unwrap();
    settle_common(&mut effect);
    let mut left = [0.0; 64];
    let mut right = [0.0; 64];
    left[0] = 1.0;
    effect.process(&mut left, &mut right, CTX).unwrap();
    assert!((left[0] - 1.0).abs() < 1e-6);
    assert!(left[15..30].iter().copied().fold(0.0_f32, f32::min) < -0.5);
    assert!(right.iter().all(|sample| sample.abs() < 1e-6));
}

#[test]
fn wow_flutter_zero_depth_is_transparent() {
    let mut effect = AudioEffect::new(43, CTX.sample_rate).unwrap();
    effect.set_parameter(3, 0.0).unwrap();
    effect.set_parameter(6, 0.0).unwrap();
    settle_common(&mut effect);
    let mut left = [1.0, 0.0, -0.5, 0.25];
    let mut right = [-1.0, 0.5, 0.0, 0.25];
    let old_left = left;
    let old_right = right;
    effect.process(&mut left, &mut right, CTX).unwrap();
    assert_eq!(left, old_left);
    assert_eq!(right, old_right);
}

#[test]
fn wow_flutter_depth_moves_an_impulse_in_time() {
    let mut effect = AudioEffect::new(43, CTX.sample_rate).unwrap();
    effect.set_parameter(3, 1.0).unwrap();
    effect.set_parameter(6, 0.0).unwrap();
    let mut silence = vec![0.0; 48_000];
    let mut other = silence.clone();
    effect.process(&mut silence, &mut other, CTX).unwrap();
    effect.reset();
    let mut left = vec![0.0; 10_000];
    let mut right = left.clone();
    left[0] = 1.0;
    effect.process(&mut left, &mut right, CTX).unwrap();
    assert!(left[0].abs() < 1e-6);
    assert!(left.iter().skip(1).any(|sample| sample.abs() > 0.01));
    assert!(right.iter().all(|sample| sample.abs() < 1e-6));
}

#[test]
fn tremolo_amount_limits_stay_ordered() {
    let mut effect = AudioEffect::new(46, CTX.sample_rate).unwrap();
    effect.set_parameter(3, 0.2).unwrap();
    effect.set_parameter(4, 0.6).unwrap();
    assert_eq!(effect.parameter(3), Some(0.6));
    effect.set_parameter(3, 0.1).unwrap();
    assert_eq!(effect.parameter(4), Some(0.1));
}

#[test]
fn chorus_mix_and_stereo_taps_follow_the_shared_delay() {
    let mut effect = AudioEffect::new(11, CTX.sample_rate).unwrap();
    settle_common(&mut effect);
    let mut left = [1.0];
    let mut right = [1.0];
    effect.process(&mut left, &mut right, CTX).unwrap();
    assert!((left[0] - 1.0).abs() < 0.01);
    assert!(right[0] > 1.2 && right[0] < 1.3);

    let mut tail_left = vec![0.0; 960];
    let mut tail_right = vec![0.0; 960];
    effect
        .process(&mut tail_left, &mut tail_right, CTX)
        .unwrap();
    assert!(tail_left.iter().any(|sample| sample.abs() > 0.1));

    effect.set_parameter(4, 0.0).unwrap();
    let mut dry_left = [0.2, -0.5, 0.9];
    let mut dry_right = [-0.3, 0.4, 0.1];
    let expected_left = dry_left;
    let expected_right = dry_right;
    effect.process(&mut dry_left, &mut dry_right, CTX).unwrap();
    assert_eq!(dry_left, expected_left);
    assert_eq!(dry_right, expected_right);
}

#[test]
fn trance_delay_echoes_on_the_selected_beat_and_ping_pongs() {
    let mut stereo = AudioEffect::new(16, CTX.sample_rate).unwrap();
    stereo.set_parameter(1, 0.0).unwrap();
    settle_common(&mut stereo);
    let mut left = vec![0.0; 12_001];
    let mut right = vec![0.0; 12_001];
    left[0] = 1.0;
    stereo.process(&mut left, &mut right, CTX).unwrap();
    assert_eq!(left[0], 1.0);
    assert!((left[6_000] - 0.396).abs() < 0.002);
    assert!(right.iter().all(|sample| sample.abs() < 1e-6));

    let mut ping_pong = AudioEffect::new(16, CTX.sample_rate).unwrap();
    ping_pong.set_parameter(1, 0.0).unwrap();
    ping_pong.set_parameter(5, 0.5).unwrap();
    settle_common(&mut ping_pong);
    left.fill(0.0);
    right.fill(0.0);
    left[0] = 1.0;
    ping_pong.process(&mut left, &mut right, CTX).unwrap();
    assert!(left[6_000].abs() < 1e-6);
    assert!((right[6_000] - 0.396).abs() < 0.002);
}

#[test]
fn processing_is_independent_of_block_size() {
    let mut one = AudioEffect::new(9, CTX.sample_rate).unwrap();
    settle_common(&mut one);
    let mut split = AudioEffect::new(9, CTX.sample_rate).unwrap();
    settle_common(&mut split);
    one.set_parameter(1, 0.4).unwrap();
    split.set_parameter(1, 0.4).unwrap();
    let mut whole = [0.0; 256];
    whole[0] = 1.0;
    let mut whole_r = whole;
    let mut chunks = whole;
    let mut chunks_r = whole;
    one.process(&mut whole, &mut whole_r, CTX).unwrap();
    for (l, r) in chunks.chunks_mut(17).zip(chunks_r.chunks_mut(17)) {
        split.process(l, r, CTX).unwrap();
    }
    for (a, b) in whole.iter().zip(chunks) {
        assert!((a - b).abs() < 1e-6);
    }
}

#[test]
fn bypass_and_zero_mix_keep_the_dry_signal() {
    let mut effect = AudioEffect::new(9, CTX.sample_rate).unwrap();
    settle_common(&mut effect);
    let source = [1.0, 0.0, -0.5, 0.25];
    let mut left = source;
    let mut right = source;
    effect.set_parameter(0, 0.0).unwrap();
    effect.process(&mut left, &mut right, CTX).unwrap();
    assert_eq!(left, source);
    effect.set_parameter(0, 1.0).unwrap();
    effect.set_common_parameter(0x7fc1, 0.0).unwrap();
    effect.process(&mut left, &mut right, CTX).unwrap();
    assert_eq!(left, source);
}

#[test]
fn invalid_controls_and_buffers_are_rejected() {
    let mut effect = AudioEffect::new(20, CTX.sample_rate).unwrap();
    settle_common(&mut effect);
    assert!(matches!(
        effect.set_parameter(1, f32::NAN),
        Err(EffectError::InvalidValue)
    ));
    assert!(matches!(
        effect.set_parameter(20, 0.5),
        Err(EffectError::InvalidParameter(20))
    ));
    assert!(matches!(
        effect.process(&mut [0.0; 2], &mut [0.0; 1], CTX),
        Err(EffectError::MismatchedBuffers)
    ));
}

#[test]
fn compressor_uses_linked_peak_and_limiting_ratio() {
    let mut effect = AudioEffect::new(8, CTX.sample_rate).unwrap();
    settle_common(&mut effect);
    effect.set_parameter(1, 0.5).unwrap();
    effect.set_parameter(2, 1.0).unwrap();
    effect.set_parameter(3, 0.0).unwrap();
    effect.set_parameter(7, 0.0).unwrap();
    effect.set_parameter(5, 0.0).unwrap();
    let mut left = [1.0];
    let mut right = [0.25];
    effect.process(&mut left, &mut right, CTX).unwrap();
    assert!((left[0] - 0.1).abs() < 0.002);
    assert!((right[0] - 0.025).abs() < 0.002);
    assert!(effect.meter().gain_reduction_db > 19.0);
}

#[test]
fn compressor_attack_reduces_gain_over_time() {
    let mut effect = AudioEffect::new(8, CTX.sample_rate).unwrap();
    settle_common(&mut effect);
    effect.set_parameter(1, 0.5).unwrap();
    effect.set_parameter(2, 1.0).unwrap();
    effect.set_parameter(3, 1.0).unwrap();
    effect.set_parameter(7, 0.0).unwrap();
    effect.set_parameter(5, 0.0).unwrap();
    let mut left = [1.0; 4096];
    let mut right = [1.0; 4096];
    effect.process(&mut left, &mut right, CTX).unwrap();
    assert!(left[0] > 0.9);
    assert!(left[4095] < 0.2);
}

#[test]
fn distortion_modes_produce_distinct_bounded_transfers() {
    let mut outputs = Vec::new();
    for mode in [0.0, 1.0 / 3.0, 2.0 / 3.0, 1.0] {
        let mut effect = AudioEffect::new(6, CTX.sample_rate).unwrap();
        settle_common(&mut effect);
        effect.set_parameter(1, 0.25).unwrap();
        effect.set_parameter(5, mode).unwrap();
        let mut left = [0.2];
        let mut right = [-0.2];
        effect.process(&mut left, &mut right, CTX).unwrap();
        assert!(left[0].is_finite() && left[0].abs() <= 1.0);
        assert!(right[0].is_finite() && right[0].abs() <= 1.0);
        outputs.push(left[0]);
    }
    assert!(
        outputs
            .windows(2)
            .any(|pair| (pair[0] - pair[1]).abs() > 0.05)
    );
}

#[test]
fn bitcrush_holds_samples_across_block_boundaries() {
    let mut effect = AudioEffect::new(6, CTX.sample_rate).unwrap();
    settle_common(&mut effect);
    effect.set_parameter(2, 1.0).unwrap();
    let mut left = [0.2, 0.8, -0.4, 0.1];
    let mut right = left;
    effect
        .process(&mut left[..2], &mut right[..2], CTX)
        .unwrap();
    effect
        .process(&mut left[2..], &mut right[2..], CTX)
        .unwrap();
    assert_eq!(left[0], left[1]);
    assert_eq!(left[1], left[2]);
}

#[test]
fn legacy_limiter_caps_output_at_unity_after_threshold_compensation() {
    let mut effect = AudioEffect::new(5, CTX.sample_rate).unwrap();
    settle_common(&mut effect);
    effect.set_parameter(1, 0.75).unwrap();
    let mut left = [4.0];
    let mut right = [-2.0];
    effect.process(&mut left, &mut right, CTX).unwrap();
    assert!((left[0] - 1.0).abs() < 1e-5);
    assert!((right[0] + 0.5).abs() < 1e-5);
    assert_eq!(effect.latency_samples(), 0);
}

#[test]
fn lookahead_limiter_reports_and_applies_two_milliseconds_of_latency() {
    let mut effect = AudioEffect::new(28, CTX.sample_rate).unwrap();
    settle_common(&mut effect);
    assert_eq!(effect.latency_samples(), 96);
    let mut left = [0.0; 128];
    let mut right = [0.0; 128];
    left[0] = 2.0;
    right[0] = -1.0;
    effect.process(&mut left, &mut right, CTX).unwrap();
    assert!(left[..96].iter().all(|sample| *sample == 0.0));
    assert!((left[96] - 1.0).abs() < 0.01);
    assert!((right[96] + 0.5).abs() < 0.01);
}

#[test]
fn sample_rate_reconfiguration_updates_latency_outside_processing() {
    let mut effect = AudioEffect::new(28, 48_000.0).unwrap();
    effect.configure_sample_rate(44_100.0).unwrap();
    assert_eq!(effect.latency_samples(), 88);
    let mut left = [0.0];
    let mut right = [0.0];
    assert!(matches!(
        effect.process(&mut left, &mut right, CTX),
        Err(EffectError::InvalidSampleRate)
    ));
}

#[test]
fn common_post_gain_and_wet_pan_follow_the_shared_blend_law() {
    let mut effect = AudioEffect::new(3, CTX.sample_rate).unwrap();
    settle_common(&mut effect);
    effect.set_common_parameter(0x7fc0, 1.0).unwrap();
    let mut silent_left = [0.0; 2048];
    let mut silent_right = [0.0; 2048];
    effect
        .process(&mut silent_left, &mut silent_right, CTX)
        .unwrap();
    let mut left = [1.0];
    let mut right = [1.0];
    effect.process(&mut left, &mut right, CTX).unwrap();
    assert!((left[0] - 10.0).abs() < 0.01);
    assert!((right[0] - 10.0).abs() < 0.01);

    effect.set_common_parameter(0x7fc2, 0.0).unwrap();
    let mut silent_left = [0.0; 20_000];
    let mut silent_right = [0.0; 20_000];
    effect
        .process(&mut silent_left, &mut silent_right, CTX)
        .unwrap();
    let mut left = [1.0];
    let mut right = [1.0];
    effect.process(&mut left, &mut right, CTX).unwrap();
    assert!((left[0] - 10.0).abs() < 0.01);
    assert!((right[0] - 1.0).abs() < 0.01);
}

#[test]
fn returning_common_mix_to_full_wet_keeps_the_smoothing_ramp() {
    let mut effect = AudioEffect::new(20, CTX.sample_rate).unwrap();
    effect.set_parameter(1, 0.0).unwrap();
    effect.set_common_parameter(0x7fc1, 0.25).unwrap();
    let mut left = [1.0; 10_000];
    let mut right = [1.0; 10_000];
    effect.process(&mut left, &mut right, CTX).unwrap();
    effect.set_common_parameter(0x7fc1, 1.0).unwrap();
    let mut left = [1.0];
    let mut right = [1.0];
    effect.process(&mut left, &mut right, CTX).unwrap();
    assert!((0.7..0.8).contains(&left[0]));
    assert!((0.7..0.8).contains(&right[0]));
}

#[test]
fn the_pitcher_note_mask_goes_through_the_public_effect_api() {
    let mut pitcher = AudioEffect::new(25, CTX.sample_rate).unwrap();
    assert_eq!(
        pitcher.pitcher_notes(),
        Some(honeydew_audio::pitcher::DEFAULT_NOTES)
    );
    let only_e: [bool; 12] = std::array::from_fn(|note| note == 4);
    pitcher.set_pitcher_notes(only_e).unwrap();
    assert_eq!(pitcher.pitcher_notes(), Some(only_e));
    // A seek resets the DSP, not the mask.
    pitcher.reset();
    assert_eq!(pitcher.pitcher_notes(), Some(only_e));
    let mut other = AudioEffect::new(36, CTX.sample_rate).unwrap();
    assert_eq!(other.pitcher_notes(), None);
    assert_eq!(
        other.set_pitcher_notes(only_e),
        Err(EffectError::UnsupportedPitcherNotes(36))
    );
}

#[test]
fn song_locked_lfos_free_run_while_the_transport_is_stopped() {
    // Stopped, the host still passes a start beat (the playhead), but it doesn't advance: an LFO that follows it would
    // freeze. Stopped at beat 0 must sound like playing from beat 0.
    // MultiFX LP with LFO; Waveshaper, Tremolo, Flanger and Wow & Flutter synced; Phaser and Autoduck as they are.
    let setups: [(u16, &[(usize, f32)]); 8] = [
        (4, &[(1, 2.0 / 7.0), (2, 0.3), (4, 0.0), (5, 1.0)]),
        (41, &[(16, 1.0), (17, 0.0), (19, 1.0), (1, 3.0 / 6.0)]),
        (46, &[(7, 1.0)]),
        (14, &[]),
        (13, &[(2, 1.0)]),
        (43, &[(7, 1.0)]),
        (29, &[]),
        (10, &[]),
    ];
    for (id, settings) in setups {
        let render = |playing: bool| -> Vec<f32> {
            let mut effect = AudioEffect::new(id, 48_000.0).unwrap();
            for &(index, value) in settings {
                effect.set_parameter(index, value).unwrap();
            }
            let mut out = Vec::new();
            for block in 0..48 {
                let mut left: Vec<f32> = (0..500)
                    .map(|i| {
                        (std::f32::consts::TAU * 3_000.0 * (block * 500 + i) as f32 / 48_000.0)
                            .sin()
                    })
                    .collect();
                let mut right = left.clone();
                let start_beat = if playing {
                    block as f64 * 500.0 / 24_000.0
                } else {
                    0.0
                };
                let context = ProcessContext {
                    start_beat,
                    playing,
                    ..CTX
                };
                effect.process(&mut left, &mut right, context).unwrap();
                out.extend(left);
            }
            out
        };
        let (playing, stopped) = (render(true), render(false));
        let worst = playing
            .iter()
            .zip(&stopped)
            .fold(0.0f32, |m, (a, b)| m.max((a - b).abs()));
        assert!(
            worst < 1e-3,
            "effect {id}: stopped differs from playing by {worst}"
        );
    }
}

/// A fresh `id` through `steps`: each sets one param, then processes 4096 frames of a 220 Hz tone (right = −½ left).
/// Every output frame, left then right per block.
fn render_steps(id: u16, steps: &[(usize, f32)]) -> Vec<f32> {
    let mut effect = AudioEffect::new(id, CTX.sample_rate).unwrap();
    let mut output = Vec::new();
    for (block, &(index, value)) in steps.iter().enumerate() {
        effect.set_parameter(index, value).unwrap();
        let mut left: Vec<f32> = (0..4096)
            .map(|i| {
                (std::f32::consts::TAU * 220.0 * (block * 4096 + i) as f32 / CTX.sample_rate).sin()
            })
            .collect();
        let mut right: Vec<f32> = left.iter().map(|x| -0.5 * x).collect();
        effect.process(&mut left, &mut right, CTX).unwrap();
        output.extend(left);
        output.extend(right);
    }
    output
}

/// `steps(0.5)` renders exactly as `steps(1)` (on) when
/// `half_is_on`, else as `steps(0)` (off); `steps(0.001)` renders as off. Returns whether on and off differ.
fn two_state(id: u16, steps: impl Fn(f32) -> Vec<(usize, f32)>, half_is_on: bool) -> bool {
    let [off, half, low, on] = [0.0, 0.5, 0.001, 1.0].map(|v| render_steps(id, &steps(v)));
    let state = |on| if on { "on" } else { "off" };
    assert!(
        half == if half_is_on { &on } else { &off }[..],
        "ID {id}: 0.5 is not {}",
        state(half_is_on)
    );
    assert!(low == off, "ID {id}: 0.001 is not off");
    on != off
}

#[test]
fn tape_stop_toggles_are_on_above_one_half() {
    // Toggle Play: start at v, play, back to v. Sync: stop with the synced time (0.5 beat) or the free one (1 s).
    assert!(two_state(42, |v| vec![(5, v), (5, 1.0), (5, v)], false));
    assert!(two_state(42, |v| vec![(7, v), (5, 0.0)], false));
}

#[test]
fn reverb_two_toggles_are_on_above_one_half() {
    // Side Input inverts the right input. Tempo reads the Delay 0.05 as beats: 25 ms at 120 BPM instead of 50 ms.
    assert!(two_state(35, |v| vec![(10, v)], false));
    assert!(two_state(35, |v| vec![(11, 0.05), (12, v)], false));
}

#[test]
fn tremolo_and_wow_flutter_sync_is_on_above_one_half() {
    assert!(two_state(46, |v| vec![(7, v)], false));
    assert!(two_state(43, |v| vec![(7, v)], false));
}

#[test]
fn compressor_auto_gain_is_on_from_one_half() {
    assert!(two_state(8, |v| vec![(7, v)], true));
}

#[test]
fn enable_is_on_from_one_half_for_every_effect() {
    // The rack bypass: 0.5 runs the processor as 1 does, 0.001 bypasses it as 0 does. The Spreader too: its DSP never
    // sees param 0. Effects transparent at their defaults on a 220 Hz tone get a
    // setting that makes them audible first: an EQ band at +12 dB, MultiFX LP, the legacy limiter's +12 dB output, a
    // 10 ms tape delay, Tape Stop stopped. Tuner (24) and Analyzer (26) never change the audio or the meter, so their
    // Enable has nothing to observe here.
    let setup = |id: u16| -> &'static [(usize, f32)] {
        match id {
            3 | 7 | 30 => &[(1, 1.0)],
            4 => &[(1, 2.0 / 7.0), (2, 0.3)],
            5 => &[(3, 1.0)],
            18 => &[(1, 0.1)],
            42 => &[(5, 0.0)],
            _ => &[],
        }
    };
    let ids: Vec<u16> = (0..64)
        .filter(|&id| AudioEffect::new(id, CTX.sample_rate).is_ok())
        .collect();
    assert_eq!(ids.len(), 34);
    for id in ids {
        let steps = |v| [setup(id), &[(0, v)]].concat();
        let audible = two_state(id, steps, true);
        assert_eq!(audible, !matches!(id, 24 | 26), "ID {id}: on and off");
    }
}

#[test]
fn tape_stop_play_at_one_half_is_off_past_the_ramp_and_after_a_reset() {
    // Stop and Start Time 0.002 (10 ms): each ramp ends inside its 4096-frame block, so the stop crossfade, the
    // stopped hold and the start all see the 0.5. Then a reset at 0.5 starts stopped: playing ramps up from a halt.
    let short = |v| vec![(1, 0.002), (2, 0.002), (5, v), (5, 1.0), (5, v)];
    assert!(two_state(42, short, false));
    let after_reset = |v: f32| {
        let mut effect = AudioEffect::new(42, CTX.sample_rate).unwrap();
        effect.set_parameter(5, v).unwrap();
        effect.reset();
        let mut output = Vec::new();
        for (block, play) in [v, 1.0].into_iter().enumerate() {
            effect.set_parameter(5, play).unwrap();
            let mut left: Vec<f32> = (0..4096)
                .map(|i| {
                    let t = (block * 4096 + i) as f32 / CTX.sample_rate;
                    (std::f32::consts::TAU * 220.0 * t).sin()
                })
                .collect();
            let mut right = left.clone();
            effect.process(&mut left, &mut right, CTX).unwrap();
            output.extend(left);
        }
        output
    };
    let off = after_reset(0.0);
    assert!(after_reset(1.0) != off);
    for v in [0.5, 0.001] {
        assert!(after_reset(v) == off, "reset at {v} is not off");
    }
}

/// A run of `effect` over 0.25 s of xorshift noise in 256-frame blocks at `rate`: its output, both channels.
fn noise_through(effect: &mut AudioEffect, rate: f32, seed: &mut u32) -> Vec<f32> {
    let context = ProcessContext {
        sample_rate: rate,
        ..CTX
    };
    let mut out = Vec::new();
    for _ in 0..(rate as usize / 4).div_ceil(256) {
        let mut noise = || {
            *seed ^= *seed << 13;
            *seed ^= *seed >> 17;
            *seed ^= *seed << 5;
            *seed as f32 / u32::MAX as f32 - 0.5
        };
        let (mut left, mut right): (Vec<f32>, Vec<f32>) =
            (0..256).map(|_| (noise(), noise())).unzip();
        effect.process(&mut left, &mut right, context).unwrap();
        out.extend(left.into_iter().chain(right));
    }
    out
}

/// The device rate has no floor (a Bluetooth headset runs at 16 kHz), and a frequency past
/// Nyquist turned the Filter's cutoff (#1 = 1), the Spreader's (#1, #2 = 1) and MultiFX's (Vox, #1 = 1; LP at X = 1)
/// unstable: inf, then NaN until a reset. Every setting stays finite at 16 and 22.05 kHz: the edges, the test
/// cases and random settings.
#[test]
fn filter_spreader_and_multifx_stay_finite_below_44_1_khz() {
    let mut seed = 0x2545_f491_u32;
    for rate in [16_000.0, 22_050.0] {
        for id in [9, 44, 4] {
            let count =
                (0..).take_while(|&p| AudioEffect::new(id, rate).unwrap().parameter(p).is_some());
            let count = count.count();
            for case in 0..40 {
                let mut effect = AudioEffect::new(id, rate).unwrap();
                for p in 1..count {
                    seed ^= seed << 13;
                    seed ^= seed >> 17;
                    seed ^= seed << 5;
                    let v = match case {
                        0 => 1.0,
                        1 => 0.0,
                        // Only #1 at 1, the rest at the defaults.
                        2 if p == 1 => 1.0,
                        2 => continue,
                        // #1 and #2 at 1 (the Spreader's case).
                        3 if p <= 2 => 1.0,
                        3 => continue,
                        // Half the cases on the edges, half anywhere.
                        _ if seed.is_multiple_of(4) => (seed % 2) as f32,
                        _ => (seed >> 8) as f32 / (u32::MAX >> 8) as f32,
                    };
                    effect.set_parameter(p, v).unwrap();
                }
                let out = noise_through(&mut effect, rate, &mut seed);
                let settings: Vec<_> = (0..count).map(|p| effect.parameter(p).unwrap()).collect();
                assert!(
                    out.iter().all(|s| s.is_finite()),
                    "ID {id} at {rate} Hz, case {case}: {settings:?}"
                );
            }
        }
    }
}

/// A bypassed MultiFX stands still, its free LFO too. Stopped (the LFO free-runs from its
/// stored phase), the block after twenty bypassed ones is bit-identical to the block after none, Enable off or Mix 0.
#[test]
fn a_bypassed_multifx_resumes_its_free_lfo_where_it_stood() {
    let stopped = ProcessContext {
        playing: false,
        ..CTX
    };
    let block = |effect: &mut AudioEffect| {
        let mut left: Vec<f32> = (0..512).map(|i| (i as f32 * 0.37).sin()).collect();
        let mut right = left.clone();
        effect.process(&mut left, &mut right, stopped).unwrap();
        left.into_iter()
            .chain(right)
            .map(f32::to_bits)
            .collect::<Vec<_>>()
    };
    let resumed = |bypassed: usize, mix: bool| {
        let mut effect = AudioEffect::new(4, CTX.sample_rate).unwrap();
        // LP (trunc(7v) = 2) swept by the full LFO.
        effect.set_parameter(1, 0.3).unwrap();
        effect.set_parameter(5, 1.0).unwrap();
        block(&mut effect);
        let set = |effect: &mut AudioEffect, on: f32| {
            if mix {
                effect.set_common_parameter(0x7fc1, on).unwrap();
            } else {
                effect.set_parameter(0, on).unwrap();
            }
        };
        set(&mut effect, 0.0);
        for _ in 0..bypassed {
            block(&mut effect);
        }
        set(&mut effect, 1.0);
        block(&mut effect)
    };
    for mix in [false, true] {
        assert!(resumed(0, mix) == resumed(20, mix), "Mix {mix}");
    }
}

/// `seconds` of silence after a 0.1 s burst through `effect`, in 512-frame blocks: the output's subnormal samples in
/// the last second (each costs the audio thread up to 100× an operation, and none ever reaches 0).
fn subnormals_after_silence(effect: &mut AudioEffect, seconds: usize) -> usize {
    let rate = CTX.sample_rate as usize;
    let mut count = 0;
    for block in 0..(seconds * rate).div_ceil(512) {
        let start = block * 512;
        let mut left: Vec<f32> = (start..start + 512)
            .map(|i| {
                if i < rate / 10 {
                    (i as f32 * 0.05).sin() * 0.8
                } else {
                    0.0
                }
            })
            .collect();
        let mut right = left.clone();
        effect.process(&mut left, &mut right, CTX).unwrap();
        if start >= (seconds - 1) * rate {
            count += left
                .iter()
                .chain(&right)
                .filter(|s| s.is_subnormal())
                .count();
        }
    }
    count
}

/// A Distortion bias just off centre makes a DC its filter all but follows: the leftover, left
/// minus filter, sank to subnormals and stayed there. Trance Delay's tone filter runs on its 1e-30 floor. Neither
/// outputs subnormals once the input is silent.
#[test]
fn distortion_and_trance_delay_output_no_subnormals_after_silence() {
    for bias in [0.500_000_06, 0.499_999_97, 0.500_001, 0.51] {
        let mut effect = AudioEffect::new(6, CTX.sample_rate).unwrap();
        effect.set_parameter(6, bias).unwrap();
        let count = subnormals_after_silence(&mut effect, 3);
        assert_eq!(count, 0, "Distortion bias {bias}");
    }
    for mode in [0.0, 0.5, 1.0] {
        for filter in [0.0, 0.1, 0.25, 0.5, 0.6, 0.75, 0.9, 1.0] {
            let mut effect = AudioEffect::new(16, CTX.sample_rate).unwrap();
            effect.set_parameter(5, mode).unwrap();
            effect.set_parameter(4, filter).unwrap();
            let count = subnormals_after_silence(&mut effect, 6);
            assert_eq!(count, 0, "Trance Delay mode {mode} filter {filter}");
        }
    }
    // A low Mix scales the high-passed echo (p4 0.77) of a long feedback into subnormals, in mono (p5 0)
    // and in ping-pong (p5 1).
    for (feedback, mode) in [(0.92, 0.0), (0.4, 1.0)] {
        let mut effect = AudioEffect::new(16, CTX.sample_rate).unwrap();
        for (p, v) in [(2, feedback), (3, 0.05), (4, 0.77), (5, mode)] {
            effect.set_parameter(p, v).unwrap();
        }
        let count = subnormals_after_silence(&mut effect, 12);
        assert_eq!(count, 0, "Trance Delay feedback {feedback} mode {mode}");
    }
}

/// A bypassed Tape Stop still takes the tempo. A synced 1-beat stop set at 120 BPM, bypassed while the
/// tempo goes to 60, then pressed as it comes back: a 0.5 DC slows for 48000 frames (a beat at 60 BPM, not the 24000
/// of a stale 120), then its held value decays below 1e-3 in 6761 more.
#[test]
fn a_bypassed_tape_stop_keeps_the_tempo_for_a_synced_stop() {
    let at = |tempo_bpm| ProcessContext { tempo_bpm, ..CTX };
    let mut effect = AudioEffect::new(42, CTX.sample_rate).unwrap();
    effect.set_parameter(7, 1.0).unwrap();
    effect.set_parameter(3, 5.0 / 9.0).unwrap();
    let block = |effect: &mut AudioEffect, tempo_bpm| {
        let (mut left, mut right) = (vec![0.5; 512], vec![0.5; 512]);
        effect
            .process(&mut left, &mut right, at(tempo_bpm))
            .unwrap();
        left
    };
    for _ in 0..10 {
        block(&mut effect, 120.0);
    }
    effect.set_parameter(0, 0.0).unwrap();
    for _ in 0..10 {
        block(&mut effect, 60.0);
    }
    effect.set_parameter(0, 1.0).unwrap();
    effect.set_parameter(5, 0.0).unwrap();
    let out: Vec<f32> = (0..300).flat_map(|_| block(&mut effect, 60.0)).collect();
    let silent_from = out
        .iter()
        .rposition(|s| s.abs() > 1e-3)
        .map_or(0, |p| p + 1);
    assert_eq!(silent_from, 54_761);
}

/// Filter, Spreader and MultiFX sound at 44.1, 48 and 96 kHz as they did before their angles were clamped
/// short of π for low device rates (`3f9124edf42526bd`, taken before the clamps): 60 settings each, the edges and
/// random ones, over noise. A deliberate change to these effects' sound updates the hash.
#[test]
fn filter_spreader_and_multifx_are_unchanged_at_44_1_khz_and_above() {
    let mut seed = 0x1234_5678_u32;
    let mut next = move || {
        seed ^= seed << 13;
        seed ^= seed >> 17;
        seed ^= seed << 5;
        seed
    };
    let mut hash = 0u64;
    for rate in [44_100.0, 48_000.0, 96_000.0] {
        let context = ProcessContext {
            sample_rate: rate,
            ..CTX
        };
        for id in [9, 44, 4] {
            let count =
                (0..).take_while(|&p| AudioEffect::new(id, rate).unwrap().parameter(p).is_some());
            let count = count.count();
            for case in 0..60 {
                let mut effect = AudioEffect::new(id, rate).unwrap();
                for p in 1..count {
                    let r = next();
                    let v = match case {
                        0 => 1.0,
                        1 => 0.0,
                        _ if r.is_multiple_of(3) => (r % 2) as f32,
                        _ => (r >> 8) as f32 / (u32::MAX >> 8) as f32,
                    };
                    effect.set_parameter(p, v).unwrap();
                }
                for _ in 0..40 {
                    let mut noise = || next() as f32 / u32::MAX as f32 - 0.5;
                    let (mut left, mut right): (Vec<f32>, Vec<f32>) =
                        (0..256).map(|_| (noise(), noise())).unzip();
                    effect.process(&mut left, &mut right, context).unwrap();
                    for s in left.iter().chain(&right) {
                        hash = hash
                            .wrapping_mul(0x100_0000_01b3)
                            .wrapping_add(u64::from(s.to_bits()));
                    }
                }
            }
        }
    }
    // Spreader's tanf differs slightly between glibc 2.39 and 2.44; both complete renders are verified.
    assert!(
        [0x3f91_24ed_f425_26bd, 0xe6a9_4b03_7ed8_d400].contains(&hash),
        "unexpected render hash: {hash:016x}"
    );
}
