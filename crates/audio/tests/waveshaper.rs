use honeydew_audio::waveshaper::{
    DEFAULT_PARAMS, WaveshaperError, WaveshaperState, shape, shape_index,
};

fn near(actual: f32, expected: f32) {
    assert!((actual - expected).abs() < 1e-4, "{actual} != {expected}");
}

#[test]
fn known_curve_families_have_distinct_transfer_functions() {
    near(shape(1, 0.5, 0.0, 0.5), 0.5_f32.tanh());
    near(shape(3, 0.5, 0.0, 0.5), 0.5_f32.sin());
    let smooth = shape(0, 0.5, 0.5, 0.5);
    assert!(smooth > 0.0 && smooth < 1.0);
    let signatures: Vec<_> = (0..7).map(|index| shape(index, 0.67, 0.31, 0.37)).collect();
    assert!(signatures.iter().all(|value| value.is_finite()));
    assert!(
        signatures
            .windows(2)
            .all(|pair| (pair[0] - pair[1]).abs() > 1e-5)
    );
}

#[test]
fn dry_mix_is_exact_and_drive_changes_wet_signal() {
    let mut params = DEFAULT_PARAMS;
    params[8] = 0.0;
    params[5] = 1.0;
    let mut dry = WaveshaperState::new(&params, 48_000.0).unwrap();
    dry.prepare_block(120.0, Some(0.0)).unwrap();
    assert_eq!(dry.process_frame(0.2, -0.3), (0.2, -0.3));

    params[8] = 1.0;
    params[5] = 0.0;
    let mut clean = WaveshaperState::new(&params, 48_000.0).unwrap();
    params[5] = 0.5;
    let mut driven = WaveshaperState::new(&params, 48_000.0).unwrap();
    clean.prepare_block(120.0, Some(0.0)).unwrap();
    driven.prepare_block(120.0, Some(0.0)).unwrap();
    let a = clean.process_frame(0.2, 0.2).0;
    let b = driven.process_frame(0.2, 0.2).0;
    assert!((a - b).abs() > 0.02);
}

#[test]
fn prefilter_and_post_eq_change_only_the_wet_path() {
    let mut params = DEFAULT_PARAMS;
    params[1] = 1.0 / 6.0;
    let mut base = WaveshaperState::new(&params, 48_000.0).unwrap();
    params[10] = 0.7;
    params[11] = 1.0;
    let mut filtered = WaveshaperState::new(&params, 48_000.0).unwrap();
    base.prepare_block(120.0, None).unwrap();
    filtered.prepare_block(120.0, None).unwrap();
    let mut plain = 0.0;
    let mut changed = 0.0;
    for _ in 0..512 {
        plain = base.process_frame(0.2, 0.2).0;
        changed = filtered.process_frame(0.2, 0.2).0;
    }
    assert!(changed.abs() < plain.abs() * 0.5);
}

#[test]
fn lfo_phase_continues_across_blocks_and_sync_aligns_to_song_beat() {
    let mut params = DEFAULT_PARAMS;
    params[15] = 0.5;
    params[18] = 0.7;
    params[19] = 0.7;
    let mut continuous = WaveshaperState::new(&params, 100.0).unwrap();
    let mut split = WaveshaperState::new(&params, 100.0).unwrap();
    continuous.prepare_block(120.0, None).unwrap();
    split.prepare_block(120.0, None).unwrap();
    let a: Vec<_> = (0..20)
        .map(|_| continuous.process_frame(0.2, 0.2).0)
        .collect();
    let mut b: Vec<_> = (0..10).map(|_| split.process_frame(0.2, 0.2).0).collect();
    split.prepare_block(120.0, None).unwrap();
    b.extend((0..10).map(|_| split.process_frame(0.2, 0.2).0));
    assert_eq!(a, b);

    params[16] = 1.0;
    params[17] = 0.0;
    let mut synced = WaveshaperState::new(&params, 100.0).unwrap();
    synced.prepare_block(120.0, Some(0.0)).unwrap();
    let first = synced.process_frame(0.2, 0.2).0;
    synced.prepare_block(120.0, Some(0.0625)).unwrap();
    let shifted = synced.process_frame(0.2, 0.2).0;
    assert!((first - shifted).abs() > 1e-4);
}

#[test]
fn reset_clears_filter_and_lfo_history() {
    let mut params = DEFAULT_PARAMS;
    params[10] = 0.4;
    params[19] = 1.0;
    params[18] = 0.8;
    let mut state = WaveshaperState::new(&params, 48_000.0).unwrap();
    state.prepare_block(120.0, None).unwrap();
    let first = state.process_frame(0.3, 0.3);
    for _ in 0..100 {
        state.process_frame(0.3, 0.3);
    }
    state.reset();
    state.prepare_block(120.0, None).unwrap();
    assert_eq!(first, state.process_frame(0.3, 0.3));
}

#[test]
fn rejects_invalid_controls_before_mutating_state() {
    let mut params = DEFAULT_PARAMS;
    params[5] = f32::NAN;
    assert!(matches!(
        WaveshaperState::new(&params, 48_000.0),
        Err(WaveshaperError::InvalidValue)
    ));
    params = DEFAULT_PARAMS;
    let mut state = WaveshaperState::new(&params, 48_000.0).unwrap();
    params[9] = 1.5;
    assert_eq!(
        state.refresh(&params, 48_000.0),
        Err(WaveshaperError::InvalidValue)
    );
    assert_eq!(state.parameters(), &DEFAULT_PARAMS);
}

/// `shape(index, x, y, ·)` at each `(input, want)`.
fn check(index: usize, x: f32, y: f32, points: &[(f32, f32)]) {
    for &(input, want) in points {
        let got = shape(index, x, y, input);
        assert!(
            (got - want).abs() < 1e-4,
            "shape {index} at x {x}, y {y}, in {input}: {got}, want {want}"
        );
    }
}

/// The shape functions, by shape index.
#[test]
fn the_seven_shapes_are_the_panel_spec_s_functions() {
    // Smooth: a = 2x − 1, k = 1 + 9y, g(u) = u/√((u − a)² + 1), f = g(k·in)/|g(sgn a·(1 + 1/|a|))|.
    check(0, 0.5, 0.5, &[(-1.0, -0.983853), (0.5, 0.939824)]);
    check(0, 0.75, 0.0, &[(1.0, 0.802773), (-1.0, -0.497859)]);
    check(0, 0.25, 0.0, &[(1.0, 0.497859), (-1.0, -0.802773)]);
    // Analog: (tanh a + tanh(k·in − a))/(1 + tanh|a|).
    check(1, 0.75, 0.0, &[(1.0, 0.632121), (-1.0, -0.303007)]);
    // Moshpit: the 9-point curve; at x = y = 0.5 straight through (±0.1, ±0.02) and (±0.19, ±0.1).
    check(
        2,
        0.5,
        0.5,
        &[(-0.1, -0.02), (0.1, 0.02), (0.19, 0.1), (0.75, 0.75)],
    );
    check(
        2,
        0.25,
        0.2,
        &[(-1.0, -0.778), (-0.055, -0.548), (-0.485, -0.35)],
    );
    // Foldback: sin(k·in + 2 − 4x), k = 1 + 7y.
    check(3, 0.0, 1.0, &[(0.25, -0.756802)]);
    // Chainsaw: a = 0.1 + 0.9x, p = 0.1 + 0.6y, q = frac((in + 1)/(2p)).
    check(
        4,
        0.5,
        0.5,
        &[(-0.6, 0.818182), (-0.3, -0.444444), (-0.16, -0.818182)],
    );
    // Rectifuzz: a full-wave rectifier at x = y = 0.5; the negative half at Yf = 0 for x = 0.25.
    check(5, 0.5, 0.5, &[(-0.5, 0.5005), (0.3, 0.3003)]);
    check(5, 0.25, 1.0, &[(-0.5, 0.0), (0.4995, 0.129449)]);
    // Cyclone: in + exp(−e^(20^(1−a))·in²)·sin(2πk·in), a = 0.6 + 0.4x, k = ±(0.03 + 7.94·|y − 0.5|).
    check(6, 0.5, 0.5, &[(0.5, 0.479902)]);
    check(6, 0.0, 0.0, &[(0.1, -0.346432)]);
    assert_eq!(shape_index(0.58), 3);
    assert_eq!(shape_index(0.59), 4);
    assert_eq!(shape_index(1.0), 6);
}

/// The effective X/Y are p3/p4 plus LFO × X Mod·0.5 (Y Mod·0.5), clamped to 0…1, each smoothed per sample
/// with coefficient 1 − 44.1/sr.
#[test]
fn effective_xy_follow_the_lfo_clamped_and_smoothed() {
    for sample_rate in [44_100.0f32, 48_000.0] {
        let pole = 1.0 - 44.1 / sample_rate;
        let mut params = DEFAULT_PARAMS;
        params[3] = 0.2;
        let mut state = WaveshaperState::new(&params, sample_rate).unwrap();
        assert_eq!(state.effective_xy(), (0.2, 0.5));
        // Square LFO (−1 for the first half of a 50 s period), X Mod 1, Y Mod 0.4: X → clamp(0.2 − 0.5) = 0,
        // Y → 0.5 − 0.2 = 0.3.
        params[15] = 1.0;
        params[19] = 1.0;
        params[20] = 0.4;
        state.refresh(&params, sample_rate).unwrap();
        state.prepare_block(120.0, None).unwrap();
        for _ in 0..1000 {
            state.process_frame(0.1, 0.1);
        }
        let decay = pole.powi(1000);
        let (x, y) = state.effective_xy();
        assert!((x - 0.2 * decay).abs() < 1e-4, "{sample_rate}: x {x}");
        assert!(
            (y - (0.3 + 0.2 * decay)).abs() < 1e-4,
            "{sample_rate}: y {y}"
        );
        // A transport reset starts again from p3/p4.
        state.reset();
        assert_eq!(state.effective_xy(), (0.2, 0.5));
    }
}
