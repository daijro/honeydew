//! The metronome click: a square wave at 2500 Hz
//! (accent) or 2000 Hz, amplitude ±0.631·level·env, env 1 at the click's start and 0.001 less each sample, and
//! level = 0.1 + 0.9·v². Rendered offline.

use honeydew_audio::metronome::{Metronome, level};

/// `frames` frames of `metronome` added onto silence, left and right.
fn render(metronome: &mut Metronome, frames: usize) -> (Vec<f32>, Vec<f32>) {
    let (mut left, mut right) = (vec![0.0; frames], vec![0.0; frames]);
    metronome.add(&mut left, &mut right);
    (left, right)
}

/// Sign changes in `samples`: two per period of a square wave.
fn sign_changes(samples: &[f32]) -> usize {
    samples
        .windows(2)
        .filter(|pair| (pair[0] > 0.0) != (pair[1] > 0.0))
        .count()
}

#[test]
fn the_level_law_is_a_tenth_plus_nine_tenths_v_squared() {
    assert_eq!(level(0.0), 0.1);
    assert_eq!(level(1.0), 1.0);
    assert!((level(0.5) - 0.325).abs() < 1e-6);
}

#[test]
fn a_click_is_a_decaying_square_wave_at_its_pitch() {
    for (rate, accent, hz) in [
        (48_000.0, true, 2500.0_f32),
        (48_000.0, false, 2000.0),
        (44_100.0, true, 2500.0),
        (44_100.0, false, 2000.0),
    ] {
        let mut metronome = Metronome::new(rate);
        metronome.click(accent, 0.5);
        let (left, right) = render(&mut metronome, 1200);
        assert_eq!(left, right, "the click is the same on both sides");
        // Amplitude ±0.631·level·env, env = 1 − 0.001·n; it ends where env reaches 0.
        for (n, sample) in left.iter().enumerate().take(1000) {
            let env = 1.0 - 0.001 * n as f32;
            assert!(
                (sample.abs() - 0.631 * 0.5 * env).abs() < 1e-5,
                "{rate} Hz, sample {n}: {sample}"
            );
        }
        assert!(left[0] > 0.0, "a click starts on the wave's high half");
        assert!(
            left[1000..].iter().all(|s| *s == 0.0),
            "the click outlived its envelope"
        );
        // 20 ms holds 50 periods at 2500 Hz, 40 at 2000 Hz.
        let window = (rate * 0.02) as usize;
        let changes = sign_changes(&left[..window]) as f32;
        assert!(
            (changes / 2.0 - hz * 0.02).abs() <= 1.0,
            "{rate} Hz: {changes} sign changes for {hz} Hz"
        );
    }
}

#[test]
fn a_click_adds_and_a_new_one_starts_over() {
    let mut metronome = Metronome::new(48_000.0);
    // Silent until clicked.
    assert_eq!(render(&mut metronome, 64).0, vec![0.0; 64]);
    metronome.click(false, 1.0);
    let (mut left, mut right) = (vec![0.25; 300], vec![-0.25; 300]);
    metronome.add(&mut left, &mut right);
    assert!((left[0] - (0.25 + 0.631)).abs() < 1e-6);
    assert!((right[0] - (-0.25 + 0.631)).abs() < 1e-6);
    // Mid-click, a new click restarts the envelope and the wave, at its own pitch.
    metronome.click(true, 1.0);
    let (left, _) = render(&mut metronome, 1000);
    assert!((left[0] - 0.631).abs() < 1e-6);
    assert!((left[1] - 0.631 * 0.999).abs() < 1e-6);
    assert_eq!(sign_changes(&left[..960]), 99);
}
