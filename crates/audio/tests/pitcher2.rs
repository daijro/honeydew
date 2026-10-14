#[path = "../src/pitcher2.rs"]
mod pitcher2;

use pitcher2::{DEFAULT_PARAMS, Pitcher2Error, Pitcher2State};
use std::f32::consts::TAU;

#[test]
fn defaults_and_control_mappings() {
    let state = Pitcher2State::new(&DEFAULT_PARAMS, 48_000.0).unwrap();
    assert_eq!(state.parameters(), &DEFAULT_PARAMS);
    assert_eq!(state.reference_channel(), 2);
    assert!((state.formant_shift() - 0.0).abs() < 1e-6);
    assert!((state.speed_factor() - (200.0_f32.sqrt() - 0.9)).abs() < 1e-5);
    assert!((state.sensitivity_factor() - 200.0_f32.sqrt()).abs() < 1e-5);
    assert_eq!(state.latency_samples(), 2674);
    assert_eq!(state.analysis_window_samples(), 2229);
    assert_eq!(state.tracked_frequency(), 0.0);
    assert_eq!(state.target_frequency(), 0.0);
}

#[test]
fn low_latency_uses_psola_window_mapping() {
    let mut params = DEFAULT_PARAMS;
    params[25] = 1.0;
    let mut state = Pitcher2State::new(&params, 48_000.0).unwrap();
    assert_eq!(state.latency_samples(), 1336);
    params[25] = 0.0;
    state.refresh(&params, 44_100.0).unwrap();
    assert_eq!(state.latency_samples(), 2457);
}

#[test]
fn note_toggles_select_nearest_allowed_pitch_class() {
    let mut params = DEFAULT_PARAMS;
    params[1..13].fill(0.0);
    params[1] = 1.0;
    let state = Pitcher2State::new(&params, 48_000.0).unwrap();
    assert!((state.target_for(220.0) - 261.62555).abs() < 0.02);
    assert_eq!(state.target_for(0.0), 0.0);
}

#[test]
fn validates_parameters_and_channels() {
    assert!(matches!(
        Pitcher2State::new(&DEFAULT_PARAMS[..26], 48_000.0),
        Err(Pitcher2Error::InvalidValue)
    ));
    assert!(matches!(
        Pitcher2State::new(&DEFAULT_PARAMS, 1000.0),
        Err(Pitcher2Error::InvalidSampleRate)
    ));
    let mut state = Pitcher2State::new(&DEFAULT_PARAMS, 48_000.0).unwrap();
    assert_eq!(
        state.process(&mut [0.0; 4], &mut [0.0; 3]),
        Err(Pitcher2Error::MismatchedBuffers)
    );
    let mut params = DEFAULT_PARAMS;
    params[18] = 0.0;
    state.refresh(&params, 48_000.0).unwrap();
    assert_eq!(state.reference_channel(), 0);
    params[18] = 0.5;
    state.refresh(&params, 48_000.0).unwrap();
    assert_eq!(state.reference_channel(), 1);
}

#[test]
fn bypass_is_delayed_by_reported_latency() {
    let mut params = DEFAULT_PARAMS;
    params[0] = 0.0;
    let mut state = Pitcher2State::new(&params, 48_000.0).unwrap();
    let latency = state.latency_samples();
    for i in 0..latency + 3 {
        let (left, right) = state.process_frame(if i == 0 { 1.0 } else { 0.0 }, 0.0);
        assert_eq!(left, if i == latency { 1.0 } else { 0.0 });
        assert_eq!(right, 0.0);
    }
}

#[test]
fn sustained_sine_is_shifted_toward_selected_note() {
    let mut params = DEFAULT_PARAMS;
    params[1..13].fill(0.0);
    params[1] = 1.0;
    params[13] = 1.0;
    let mut state = Pitcher2State::new(&params, 48_000.0).unwrap();
    let len = 96_000;
    let mut left = (0..len)
        .map(|i| 0.5 * (TAU * 220.0 * i as f32 / 48_000.0).sin())
        .collect::<Vec<_>>();
    let mut right = left.clone();
    state.process(&mut left, &mut right).unwrap();
    let output = &left[72_000..];
    let crossings = output
        .windows(2)
        .filter(|pair| pair[0] <= 0.0 && pair[1] > 0.0)
        .count();
    let frequency = crossings as f32 * 48_000.0 / output.len() as f32;
    assert!(frequency > 245.0 && frequency < 280.0, "{frequency}");
    assert!(output.iter().any(|v| v.abs() > 0.1));
    assert!(left.iter().zip(right).all(|(a, b)| (*a - b).abs() < 1e-5));
}

#[test]
fn split_blocks_preserve_streaming_state() {
    let mut continuous = Pitcher2State::new(&DEFAULT_PARAMS, 48_000.0).unwrap();
    let mut split = Pitcher2State::new(&DEFAULT_PARAMS, 48_000.0).unwrap();
    let mut left = (0..8192)
        .map(|i| (TAU * 220.0 * i as f32 / 48_000.0).sin())
        .collect::<Vec<_>>();
    let mut right = left.clone();
    let mut left_split = left.clone();
    let mut right_split = right.clone();
    continuous.process(&mut left, &mut right).unwrap();
    for (l, r) in left_split.chunks_mut(127).zip(right_split.chunks_mut(127)) {
        split.process(l, r).unwrap();
    }
    assert_eq!(left, left_split);
    assert_eq!(right, right_split);
}

#[test]
fn low_latency_mode_corrects_and_reset_clears_analysis() {
    let mut params = DEFAULT_PARAMS;
    params[1..13].fill(0.0);
    params[1] = 1.0;
    params[13] = 1.0;
    params[25] = 1.0;
    let mut state = Pitcher2State::new(&params, 48_000.0).unwrap();
    let output = (0..48_000)
        .map(|i| {
            let sample = 0.5 * (TAU * 220.0 * i as f32 / 48_000.0).sin();
            state.process_frame(sample, sample).0
        })
        .collect::<Vec<_>>();
    let tail = &output[36_000..];
    let frequency = tail
        .windows(2)
        .filter(|pair| pair[0] <= 0.0 && pair[1] > 0.0)
        .count() as f32
        * 4.0;
    assert!(frequency > 245.0 && frequency < 280.0, "{frequency}");
    assert!((state.tracked_frequency() - 220.0).abs() < 3.0);
    assert!((state.target_frequency() - 261.62555).abs() < 0.02);
    state.reset();
    assert_eq!(state.tracked_frequency(), 0.0);
    assert_eq!(state.target_frequency(), 0.0);
}
