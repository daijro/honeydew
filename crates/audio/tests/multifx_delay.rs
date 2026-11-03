// The module uses `crate::biquad::flush`.
#[allow(dead_code)]
#[path = "../src/biquad.rs"]
mod biquad;
#[path = "../src/multifx_delay.rs"]
mod multifx_delay;

use multifx_delay::{MultiFxDelay, MultiFxDelaySettings};

fn settings(
    x: f32,
    y: f32,
    rate_beats: f32,
    lfo_amount: f32,
    lfo_shape: usize,
    tempo_bpm: f32,
    start_beat: f64,
) -> MultiFxDelaySettings {
    MultiFxDelaySettings {
        x,
        y,
        rate_beats,
        lfo_amount,
        lfo_shape,
        tempo_bpm,
        start_beat,
    }
}

#[test]
fn the_echo_is_r_beats_halved_to_at_most_1_5_s() {
    // R_eff = 2R, u = (120R/B)/3 halved while above 1, tap trunc((ring − 1)·u).
    let mut delay = MultiFxDelay::new(48_000.0);
    delay.configure(settings(1.0, 0.0, 1.0, 0.0, 0, 120.0, 0.0));
    assert_eq!(delay.capacity(), 72_000);
    assert_eq!(delay.delay_samples(), 23_999);
    delay.configure(settings(1.0, 0.0, 0.25, 0.0, 0, 120.0, 0.0));
    assert_eq!(delay.delay_samples(), 5_999);
    delay.configure(settings(1.0, 0.0, 16.0, 0.0, 0, 120.0, 0.0));
    assert_eq!(delay.delay_samples(), 47_999);
}

#[test]
fn left_impulse_returns_at_the_selected_delay_without_right_leakage() {
    let mut delay = MultiFxDelay::new(2_000.0);
    delay.configure(settings(1.0, 0.5, 0.125, 0.0, 0, 120.0, 0.0));
    let tap = delay.delay_samples();
    let mut echoed = 0.0;
    for i in 0..tap + 3 {
        let (left, right) = delay.process_frame([if i == 0 { 1.0 } else { 0.0 }, 0.0], 1.0);
        if i > 0 && i <= tap {
            assert!(left.abs() < 1e-6, "early at {i}: {left}");
        }
        if i == tap + 1 {
            echoed = left;
        }
        assert!(right.abs() < 1e-6);
    }
    assert!(echoed > 0.001, "{echoed}");
}

#[test]
fn y_axis_changes_filter_attack() {
    let mut short = MultiFxDelay::new(2_000.0);
    let mut long = MultiFxDelay::new(2_000.0);
    short.configure(settings(1.0, 0.0, 0.125, 0.0, 0, 120.0, 0.0));
    long.configure(settings(1.0, 0.4, 0.125, 0.0, 0, 120.0, 0.0));
    let tap = short.delay_samples();
    let mut slow_first = 0.0;
    let mut fast_first = 0.0;
    for i in 0..tap + 2 {
        let input = [if i == 0 { 1.0 } else { 0.0 }, 0.0];
        let a = short.process_frame(input, 1.0).0;
        let b = long.process_frame(input, 1.0).0;
        if i == tap + 1 {
            slow_first = a;
            fast_first = b;
        }
    }
    assert!(fast_first > slow_first * 20.0);
}

#[test]
fn y_above_half_enables_the_second_filter_stage() {
    let mut delay = MultiFxDelay::new(2_000.0);
    delay.configure(settings(1.0, 0.5, 0.125, 0.0, 0, 120.0, 0.0));
    assert_eq!(delay.filter_coefficients(), (1.0, 0.0));
    delay.configure(settings(1.0, 1.0, 0.125, 0.0, 0, 120.0, 0.0));
    let (first, second) = delay.filter_coefficients();
    assert_eq!(first, 1.0);
    assert!((second - 0.45).abs() < 1e-6);
}

#[test]
fn x_squared_scales_the_echo() {
    let mut dark = MultiFxDelay::new(2_000.0);
    let mut bright = MultiFxDelay::new(2_000.0);
    dark.configure(settings(0.2, 0.4, 0.125, 0.0, 0, 120.0, 0.0));
    bright.configure(settings(1.0, 0.4, 0.125, 0.0, 0, 120.0, 0.0));
    let tap = dark.delay_samples();
    let mut dark_first = 0.0;
    let mut bright_first = 0.0;
    for i in 0..tap + 2 {
        let input = [if i == 0 { 1.0 } else { 0.0 }, 0.0];
        let a = dark.process_frame(input, 1.0).0;
        let b = bright.process_frame(input, 1.0).0;
        if i == tap + 1 {
            dark_first = a;
            bright_first = b;
        }
    }
    assert!(bright_first > dark_first * 5.0);
}

#[test]
fn lfo_amount_and_shape_change_stereo_echo_balance() {
    let mut left = MultiFxDelay::new(2_000.0);
    let mut right = MultiFxDelay::new(2_000.0);
    // The LFO cycles every 2R = 0.25 beats; the echo comes half a cycle later (125 frames at 0.004).
    left.configure(settings(1.0, 0.5, 0.125, 1.0, 2, 120.0, 0.1875));
    right.configure(settings(1.0, 0.5, 0.125, 1.0, 2, 120.0, 0.0625));
    let tap = left.delay_samples();
    let mut first = (0.0, 0.0);
    let mut second = (0.0, 0.0);
    for i in 0..tap + 2 {
        let input = if i == 0 { [1.0, 1.0] } else { [0.0, 0.0] };
        first = left.process_frame(input, 1.0);
        second = right.process_frame(input, 1.0);
    }
    assert!(first.0 > first.1 * 2.0);
    assert!(second.1 > second.0 * 2.0);
}

#[test]
fn changing_tempo_retargets_and_clears_the_ring() {
    let mut delay = MultiFxDelay::new(2_000.0);
    delay.configure(settings(1.0, 0.4, 0.125, 0.0, 0, 120.0, 0.0));
    delay.process_frame([1.0, 0.0], 1.0);
    delay.configure(settings(1.0, 0.4, 0.125, 0.0, 0, 100.0, 0.0));
    assert_eq!(delay.delay_samples(), 149);
    for _ in 0..200 {
        let output = delay.process_frame([0.0, 0.0], 1.0);
        assert_eq!(output, (0.0, 0.0));
    }
    assert!(delay.wet_level() > 0.5);
    delay.reset();
    assert_eq!(delay.wet_level(), 0.0);
}

#[test]
fn wet_level_ramps_by_point_zero_zero_five_per_frame() {
    let mut delay = MultiFxDelay::new(2_000.0);
    delay.configure(settings(1.0, 0.2, 0.125, 0.0, 0, 120.0, 0.0));
    delay.process_frame([0.0, 0.0], 1.0);
    assert!((delay.wet_level() - 0.005).abs() < 1e-7);
    delay.process_frame([0.0, 0.0], 1.0);
    assert!((delay.wet_level() - 0.009975).abs() < 1e-7);
    delay.process_frame([0.0, 0.0], 0.0);
    assert!((delay.wet_level() - 0.009925125).abs() < 1e-7);
}

#[test]
fn changing_xy_keeps_pending_echo() {
    let mut delay = MultiFxDelay::new(2_000.0);
    delay.configure(settings(1.0, 0.1, 0.125, 0.0, 0, 120.0, 0.0));
    let tap = delay.delay_samples();
    delay.process_frame([1.0, 0.0], 1.0);
    delay.configure(settings(0.5, 0.4, 0.125, 0.0, 0, 120.0, -1.0));
    let mut echo = 0.0;
    for i in 1..=tap + 1 {
        let output = delay.process_frame([0.0, 0.0], 1.0);
        if i == tap + 1 {
            echo = output.0;
        }
    }
    assert!(echo > 0.0005);
    assert!((delay.filter_coefficients().0 - 0.802).abs() < 1e-6);
    assert_eq!(delay.filter_coefficients().1, 0.0);
}
