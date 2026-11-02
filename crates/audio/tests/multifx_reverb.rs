// The module uses `crate::biquad::flush`.
#[allow(dead_code)]
#[path = "../src/biquad.rs"]
mod biquad;
#[path = "../src/multifx_reverb.rs"]
mod multifx_reverb;

use multifx_reverb::MultiFxReverbState;
use std::f32::consts::TAU;

fn state(x: f32, y: f32) -> MultiFxReverbState {
    let mut state = MultiFxReverbState::new(48_000.0);
    state.refresh(x, y, 48_000.0);
    for _ in 0..2_000 {
        state.process_frame(0.0, 0.0, true);
    }
    state
}

#[test]
fn impulse_creates_a_decorrelated_stereo_tail() {
    let mut reverb = state(0.75, 0.5);
    reverb.process_frame(1.0, 0.0, true);
    let mut tail_l = 0.0;
    let mut tail_r = 0.0;
    let mut difference = 0.0;
    for i in 0..48_000 {
        let (l, r) = reverb.process_frame(0.0, 0.0, true);
        if i > 2_000 {
            tail_l += l.abs();
            tail_r += r.abs();
            difference += (l - r).abs();
        }
    }
    assert!(tail_l > 0.01, "left tail = {tail_l}");
    assert!(tail_r > 0.01, "right tail = {tail_r}");
    assert!(difference > 0.01, "stereo difference = {difference}");
}

#[test]
fn x_extends_the_late_decay() {
    let mut short = state(0.0, 0.5);
    let mut long = state(1.0, 0.5);
    short.process_frame(1.0, 1.0, true);
    long.process_frame(1.0, 1.0, true);
    let mut short_late = 0.0;
    let mut long_late = 0.0;
    for i in 0..48_000 {
        let (a, _) = short.process_frame(0.0, 0.0, true);
        let (b, _) = long.process_frame(0.0, 0.0, true);
        if i >= 24_000 {
            short_late += a.abs();
            long_late += b.abs();
        }
    }
    assert!(
        long_late > short_late * 2.0,
        "long = {long_late}, short = {short_late}"
    );
}

#[test]
fn y_opens_the_high_frequency_damping_filter() {
    let mut dark = state(0.8, 0.0);
    let mut bright = state(0.8, 1.0);
    for i in 0..24_000 {
        let input = (TAU * 10_000.0 * i as f32 / 48_000.0).sin();
        dark.process_frame(input, input, true);
        bright.process_frame(input, input, true);
    }
    let mut dark_energy = 0.0;
    let mut bright_energy = 0.0;
    for i in 0..24_000 {
        let (a, _) = dark.process_frame(0.0, 0.0, true);
        let (b, _) = bright.process_frame(0.0, 0.0, true);
        if i > 6_000 {
            dark_energy += a * a;
            bright_energy += b * b;
        }
    }
    assert!(
        bright_energy > dark_energy * 1.2,
        "bright = {bright_energy}, dark = {dark_energy}"
    );
}

#[test]
fn reset_clears_pending_audio() {
    let mut reverb = state(1.0, 0.5);
    reverb.process_frame(1.0, 0.0, true);
    for _ in 0..5_000 {
        reverb.process_frame(0.0, 0.0, true);
    }
    reverb.reset();
    for _ in 0..1_000 {
        let (l, r) = reverb.process_frame(0.0, 0.0, true);
        assert_eq!((l, r), (0.0, 0.0));
    }
}

#[test]
fn control_refresh_preserves_the_pending_tail() {
    let mut reverb = state(0.8, 0.5);
    reverb.process_frame(1.0, 0.0, true);
    for _ in 0..4_000 {
        reverb.process_frame(0.0, 0.0, true);
    }
    reverb.refresh(0.9, 0.8, 48_000.0);
    let mut energy = 0.0;
    for _ in 0..4_000 {
        let (l, r) = reverb.process_frame(0.0, 0.0, true);
        energy += l.abs() + r.abs();
    }
    assert!(energy > 0.001, "tail = {energy}");
}

#[test]
fn long_decay_remains_bounded() {
    let mut reverb = state(1.0, 1.0);
    reverb.process_frame(1.0, 1.0, true);
    let mut peak = 0.0_f32;
    for _ in 0..240_000 {
        let (l, r) = reverb.process_frame(0.0, 0.0, true);
        peak = peak.max(l.abs()).max(r.abs());
    }
    assert!(peak.is_finite() && peak < 10.0, "peak = {peak}");
}
