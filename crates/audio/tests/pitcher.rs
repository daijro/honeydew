#[allow(dead_code)]
#[path = "../src/pitcher.rs"]
mod pitcher;

use pitcher::{DEFAULT_NOTES, DEFAULT_PARAMS, PitcherError, PitcherState};
use std::f32::consts::TAU;

fn sine(sample: usize, frequency: f32, sample_rate: f32) -> f32 {
    (TAU * frequency * sample as f32 / sample_rate).sin() * 0.4
}

fn frequency(samples: &[f32], sample_rate: f32) -> f32 {
    let crossings = samples
        .windows(2)
        .filter(|pair| pair[0] <= 0.0 && pair[1] > 0.0)
        .count();
    crossings as f32 * sample_rate / samples.len() as f32
}

#[test]
fn parameter_map_and_latency_match_reference_controls() {
    let mut state = PitcherState::new(&DEFAULT_PARAMS, 48_000.0).unwrap();
    assert_eq!(state.latency_samples(), 872);
    assert_eq!(state.parameters(), &DEFAULT_PARAMS);
    assert_eq!(state.input_channel(), 0);
    let mut params = DEFAULT_PARAMS;
    params[6] = 0.5;
    params[7] = 1.0;
    params[5] = 1.0;
    state.refresh(&params, 48_000.0).unwrap();
    assert_eq!(state.input_channel(), 1);
    assert!((state.base_frequency() - 466.16).abs() < 0.5);
    assert!((state.formant_ratio() - 2.0).abs() < 1e-6);
}

#[test]
fn scale_snap_and_bypass_choose_distinct_targets() {
    let mut state = PitcherState::new(&DEFAULT_PARAMS, 8_000.0).unwrap();
    // Only C.
    state.set_notes(std::array::from_fn(|note| note == 0));
    let snapped = state.target_for(220.0);
    assert!((snapped - 261.63).abs() < 2.0);
    let mut params = DEFAULT_PARAMS;
    params[22] = 0.75;
    state.refresh(&params, 8_000.0).unwrap();
    assert!((state.target_for(220.0) - 220.0).abs() < 1.0);
    params[22] = 0.0;
    params[19] = 0.75;
    state.refresh(&params, 8_000.0).unwrap();
    assert!((state.target_for(220.0) - 220.0).abs() < 1.0);
    params[19] = 0.25;
    state.refresh(&params, 8_000.0).unwrap();
    assert!((state.target_for(220.0) - snapped).abs() < 1.0);
    params[19] = 0.0;
    params[2] = 1.0;
    params[3] = 1.0;
    state.refresh(&params, 8_000.0).unwrap();
    assert!((state.target_for(220.0) - 539.0).abs() < 5.0);
}

#[test]
fn octave_transposition_is_audible_after_declared_latency() {
    let mut params = DEFAULT_PARAMS;
    params[1] = 1.0;
    params[2] = 1.0;
    params[22] = 1.0;
    let mut state = PitcherState::new(&params, 8_000.0).unwrap();
    let output: Vec<_> = (0..8_000)
        .map(|i| {
            state
                .process_frame(sine(i, 220.0, 8_000.0), sine(i, 220.0, 8_000.0))
                .0
        })
        .collect();
    assert!(
        output[..state.latency_samples() / 2]
            .iter()
            .all(|sample| sample.abs() < 1e-4)
    );
    let tail = &output[5_000..];
    let rms = (tail.iter().map(|sample| sample * sample).sum::<f32>() / tail.len() as f32).sqrt();
    assert!(rms > 0.05);
    assert!(
        (frequency(tail, 8_000.0) - 440.0).abs() < 20.0,
        "{}",
        frequency(tail, 8_000.0)
    );
    assert!((state.tracked_frequency() - 220.0).abs() < 10.0);
}

#[test]
fn block_boundaries_do_not_restart_analysis_or_resynthesis() {
    let mut params = DEFAULT_PARAMS;
    params[1] = 1.0;
    params[2] = 0.75;
    params[22] = 1.0;
    let mut whole = PitcherState::new(&params, 8_000.0).unwrap();
    let mut split = PitcherState::new(&params, 8_000.0).unwrap();
    let input: Vec<_> = (0..4_000).map(|i| sine(i, 230.0, 8_000.0)).collect();
    let mut left_a = input.clone();
    let mut right_a = input.clone();
    whole.process(&mut left_a, &mut right_a).unwrap();
    let mut left_b = input.clone();
    let mut right_b = input;
    for (l, r) in left_b.chunks_mut(137).zip(right_b.chunks_mut(137)) {
        split.process(l, r).unwrap();
    }
    assert_eq!(left_a, left_b);
    assert_eq!(right_a, right_b);
}

#[test]
fn dry_mix_uses_reported_latency_and_input_channel_selects_tracker() {
    let mut params = DEFAULT_PARAMS;
    params[9] = 0.0;
    params[6] = 1.0;
    let mut state = PitcherState::new(&params, 8_000.0).unwrap();
    let latency = state.latency_samples();
    let mut output = Vec::new();
    for i in 0..4_000 {
        let left = if i == 0 { 1.0 } else { sine(i, 220.0, 8_000.0) };
        let right = sine(i, 330.0, 8_000.0);
        output.push(state.process_frame(left, right).0);
    }
    assert!(output[..latency].iter().all(|sample| sample.abs() < 1e-6));
    assert!((output[latency] - 1.0).abs() < 1e-6);
    assert!((state.tracked_frequency() - 330.0).abs() < 15.0);
}

#[test]
fn formant_mode_uses_a_separate_signal_path() {
    let mut params = DEFAULT_PARAMS;
    params[1] = 1.0;
    params[2] = 0.75;
    params[22] = 1.0;
    let mut basic = PitcherState::new(&params, 8_000.0).unwrap();
    params[4] = 1.0;
    params[5] = 1.0;
    let mut formant = PitcherState::new(&params, 8_000.0).unwrap();
    let mut difference = 0.0;
    for i in 0..4_000 {
        let input = sine(i, 220.0, 8_000.0) + 0.15 * sine(i, 660.0, 8_000.0);
        difference +=
            (basic.process_frame(input, input).0 - formant.process_frame(input, input).0).abs();
    }
    assert!(difference > 5.0);
}

#[test]
fn invalid_parameters_fail_without_destroying_running_state() {
    let mut state = PitcherState::new(&DEFAULT_PARAMS, 8_000.0).unwrap();
    let mut params = DEFAULT_PARAMS;
    params[8] = f32::NAN;
    assert_eq!(
        state.refresh(&params, 8_000.0),
        Err(PitcherError::InvalidValue)
    );
    assert_eq!(state.parameters(), &DEFAULT_PARAMS);
    assert_eq!(
        state.process(&mut [0.0; 2], &mut [0.0; 3]),
        Err(PitcherError::MismatchedBuffers)
    );
}

/// The note enables of `notes` (pitch classes, 0 = C).
fn mask(notes: &[usize]) -> [bool; 12] {
    std::array::from_fn(|note| notes.contains(&note))
}

#[test]
fn a_new_pitcher_snaps_to_c_minor() {
    // C, D, D#, F, G, G#, A# (1,0,1,1,0,1,0,1,1,0,1,0).
    let state = PitcherState::new(&DEFAULT_PARAMS, 8_000.0).unwrap();
    assert_eq!(DEFAULT_NOTES, mask(&[0, 2, 3, 5, 7, 8, 10]));
    assert_eq!(state.notes(), DEFAULT_NOTES);
    // 30 cents over A4 and over E4: A and E are not in C minor, so the nearer neighbours A#4 and F4.
    let cents_up = |hz: f32| hz * 2.0_f32.powf(0.3 / 12.0);
    assert!((state.target_for(cents_up(440.0)) - 466.16).abs() < 0.5);
    assert!((state.target_for(cents_up(329.63)) - 349.23).abs() < 0.5);
}

#[test]
fn the_note_mask_is_what_a_tone_is_corrected_to() {
    // Only D and G: a 220 Hz A3 goes to G3 (2 semitones down; D4 is 5 up), audibly once the glide settles.
    let mut params = DEFAULT_PARAMS;
    params[1] = 1.0;
    let mut state = PitcherState::new(&params, 8_000.0).unwrap();
    state.set_notes(mask(&[2, 7]));
    assert_eq!(state.notes(), mask(&[2, 7]));
    assert!((state.target_for(220.0) - 196.0).abs() < 0.5);
    let output: Vec<_> = (0..8_000)
        .map(|i| {
            state
                .process_frame(sine(i, 220.0, 8_000.0), sine(i, 220.0, 8_000.0))
                .0
        })
        .collect();
    let heard = frequency(&output[5_000..], 8_000.0);
    assert!((heard - 196.0).abs() < 10.0, "{heard}");
    // An empty mask snaps nowhere: a tone between semitones passes at its own pitch, not at the nearest semitone.
    state.set_notes([false; 12]);
    assert!((state.target_for(225.0) - 225.0).abs() < 0.5);
}

#[test]
fn pitch_steps_truncate_in_f64() {
    // s = trunc(f64(v)·24 − 12), exact. At v = f32(k/24): k − 12, except k = 1, 2, 4, 8,
    // 11 → k − 11 and k = 14, 17, 20, 23 → k − 13. Bypass on and Fine Tune centred: the target is 440·2^(s/12).
    let mut params = DEFAULT_PARAMS;
    params[22] = 1.0;
    let mut state = PitcherState::new(&params, 48_000.0).unwrap();
    for k in 0..=24 {
        params[2] = (f64::from(k) / 24.0) as f32;
        state.refresh(&params, 48_000.0).unwrap();
        let s = (12.0 * (state.target_for(440.0) / 440.0).log2()).round() as i32;
        let want = match k {
            1 | 2 | 4 | 8 | 11 => k - 11,
            14 | 17 | 20 | 23 => k - 13,
            _ => k - 12,
        };
        assert_eq!(s, want, "k = {k}");
    }
}
