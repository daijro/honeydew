use honeydew_audio::pitcher2::{DEFAULT_PARAMS, Pitcher2Error, Pitcher2State};
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

/// A 220 Hz vowel, harmonics shaped by one formant at 1 kHz; with Formants on, F-Shift +50 % (×√2) moves the formant
/// up and keeps the pitch; off, F-Shift does nothing (#94).
#[test]
fn formants_move_the_spectral_envelope_and_keep_the_pitch() {
    let rate = 48_000.0;
    let vowel = |i: usize| {
        (1..=30)
            .map(|k| {
                let f = 220.0 * k as f32;
                (TAU * f * i as f32 / rate).sin() / (1.0 + ((f - 1000.0) / 250.0).powi(2))
            })
            .sum::<f32>()
            * 0.3
    };
    // The centroid over the 220 Hz harmonics, and their share of the energy, over the last half second (Hann).
    let spectrum = |samples: &[f32]| {
        let n = samples.len();
        let window = |i: usize| 0.5 - 0.5 * (TAU * i as f32 / n as f32).cos();
        let (mut sum_w, mut sum_w2, mut energy) = (0.0, 0.0, 0.0);
        for (i, x) in samples.iter().enumerate() {
            let w = window(i);
            (sum_w, sum_w2, energy) = (sum_w + w, sum_w2 + w * w, energy + (x * w).powi(2));
        }
        let powers: Vec<(f32, f32)> = (1..=40)
            .map(|k| {
                let f = 220.0 * k as f32;
                let (mut re, mut im) = (0.0, 0.0);
                for (i, x) in samples.iter().enumerate() {
                    let phase = TAU * f * i as f32 / rate;
                    re += x * window(i) * phase.cos();
                    im += x * window(i) * phase.sin();
                }
                (f, re * re + im * im)
            })
            .collect();
        let total: f32 = powers.iter().map(|p| p.1).sum();
        let centroid = powers.iter().map(|(f, p)| f * p).sum::<f32>() / total;
        (centroid, 2.0 * sum_w2 / (sum_w * sum_w) * total / energy)
    };
    let render = |formants: f32| {
        let mut params = DEFAULT_PARAMS;
        params[1..13].fill(1.0);
        params[13] = 1.0;
        (params[16], params[17]) = (formants, 0.75);
        let mut state = Pitcher2State::new(&params, rate).unwrap();
        let output: Vec<f32> = (0..72_000)
            .map(|i| state.process_frame(vowel(i), vowel(i)).0)
            .collect();
        spectrum(&output[48_000..])
    };
    let input: Vec<f32> = (48_000..72_000).map(vowel).collect();
    let (dry, _) = spectrum(&input);
    let (off, _) = render(0.0);
    let (on, harmonic) = render(1.0);
    assert!((off / dry - 1.0).abs() < 0.1, "off {off} dry {dry}");
    assert!(on / dry > 1.25 && on / dry < 1.6, "on {on} dry {dry}");
    assert!(harmonic > 0.8, "{harmonic}");
}

/// Where a voice stops or starts (vowel, "s" noise, another vowel, silence), the grains fade into the dry signal: no
/// step steeper than the input's own (#94; the switch to dry clicked).
#[test]
fn voice_edges_fade_without_clicks() {
    let rate = 48_000.0;
    let mut seed = 1_u32;
    let input: Vec<f32> = (0..4 * 48_000)
        .map(|i| {
            seed ^= seed << 13;
            seed ^= seed >> 17;
            seed ^= seed << 5;
            let vowel = |f0: f32| {
                (1..=20)
                    .map(|k| {
                        let f = f0 * k as f32;
                        (TAU * f * i as f32 / rate).sin() / (1.0 + ((f - 900.0) / 300.0).powi(2))
                    })
                    .sum::<f32>()
                    * 0.25
            };
            match i % 48_000 {
                0..19_200 => vowel(233.0),
                19_200..26_400 => 0.08 * (seed as f32 / u32::MAX as f32 - 0.5),
                26_400..43_200 => vowel(185.0),
                _ => 0.0,
            }
        })
        .collect();
    let steepest = |v: &[f32]| {
        v.windows(2)
            .map(|w| (w[1] - w[0]).abs())
            .fold(0.0, f32::max)
    };
    let mut state = Pitcher2State::new(&DEFAULT_PARAMS, rate).unwrap();
    let output: Vec<f32> = input.iter().map(|&x| state.process_frame(x, x).0).collect();
    assert!(
        steepest(&output) < 1.1 * steepest(&input),
        "{} {}",
        steepest(&output),
        steepest(&input)
    );
}
