// The module uses `crate::biquad::flush`.
#[allow(dead_code)]
#[path = "../src/biquad.rs"]
mod biquad;
#[path = "../src/spreader.rs"]
mod spreader;

use spreader::SpreaderState;

const DEFAULT: [f32; 5] = [1.0, 0.0, 0.0, 0.0, 0.5];

#[test]
fn param_0_never_reaches_the_dsp() {
    // The DSP's own Enable is forced to 1; the module's on/off is the rack's bypass.
    let render = |enable: f32| {
        let mut params = DEFAULT;
        params[0] = enable;
        params[1] = 1.0;
        let mut state = SpreaderState::new(&params, 48_000.0);
        (0..4096)
            .map(|i| state.process_frame((i as f32 * 0.05).sin(), (i as f32 * 0.03).cos()))
            .collect::<Vec<_>>()
    };
    let on = render(1.0);
    assert!(render(0.5) == on && render(0.0) == on);
}

#[test]
fn after_a_reset_the_dry_input_fades_out() {
    // The DSP Enable restarts at 0 at every reset and smooths to 1 with a = 1 − exp(−500/Fs): the first frame passes
    // (1 − a) of the input as dry, plus at most a few a of processed signal.
    let a = 1.0 - (-500.0_f32 / 48_000.0).exp();
    let mut state = SpreaderState::new(&DEFAULT, 48_000.0);
    for i in 0..4800 {
        state.process_frame((i as f32 * 0.05).sin(), (i as f32 * 0.03).cos());
    }
    state.reset();
    let (left, right) = state.process_frame(1.0, -0.5);
    assert!((left - (1.0 - a)).abs() < 5.0 * a, "{left}");
    assert!((right + 0.5 * (1.0 - a)).abs() < 5.0 * a, "{right}");
}

#[test]
fn enabled_spreader_routes_an_impulse_across_channels() {
    let mut params = DEFAULT;
    params[1] = 1.0;
    params[2] = 0.5;
    let mut state = SpreaderState::new(&params, 48_000.0);
    let mut right_energy = 0.0;
    for frame in 0..4096 {
        let (_, right) = state.process_frame(if frame == 0 { 1.0 } else { 0.0 }, 0.0);
        right_energy += right.abs();
    }
    assert!(right_energy > 0.001);
}

#[test]
fn control_mapping_changes_output_and_reset_clears_history() {
    let mut dry = SpreaderState::new(&DEFAULT, 44_100.0);
    let mut wide_params = DEFAULT;
    wide_params[1] = 1.0;
    wide_params[2] = 0.75;
    wide_params[3] = 1.0;
    wide_params[4] = 1.0;
    let mut wide = SpreaderState::new(&wide_params, 44_100.0);
    let mut changed = false;
    let mut first_run = Vec::new();
    for frame in 0..2048 {
        let input = if frame == 0 { (1.0, -0.5) } else { (0.0, 0.0) };
        let plain = dry.process_frame(input.0, input.1);
        let spread = wide.process_frame(input.0, input.1);
        changed |= (plain.0 - spread.0).abs() > 1e-6 || (plain.1 - spread.1).abs() > 1e-6;
        first_run.push(spread);
    }
    assert!(changed);
    wide.reset();
    for (frame, expected) in first_run.into_iter().enumerate() {
        let input = if frame == 0 { (1.0, -0.5) } else { (0.0, 0.0) };
        assert_eq!(wide.process_frame(input.0, input.1), expected);
    }
}

#[test]
fn refresh_recalculates_for_sample_rate_and_parameters() {
    let mut state = SpreaderState::new(&DEFAULT, 44_100.0);
    let mut params = DEFAULT;
    params[1] = 0.5;
    params[2] = 1.0;
    state.refresh(&params, 96_000.0);
    for frame in 0..4096 {
        let input = if frame == 0 { 1.0 } else { 0.0 };
        let output = state.process_frame(input, input);
        assert!(output.0.is_finite() && output.1.is_finite());
    }
}

#[test]
fn stereo_tone_matches_reference_routing_at_48khz() {
    for (params, expected) in [
        (
            [1.0, 0.0, 0.0, 0.0, 0.5],
            [
                (0, 0.002051009, 0.39794892),
                (256, 0.75117826, -0.35648116),
                (2048, -0.97848606, 0.19963083),
                (4095, -0.2954437, -0.15439019),
            ],
        ),
        (
            [1.0, 1.0, 0.5, 0.0, 0.5],
            [
                (0, 0.002017593, 0.3979148),
                (256, -1.7488229, 0.027997792),
                (2048, 1.9545735, -0.4369142),
                (4095, 0.38056713, 0.0680625),
            ],
        ),
        (
            [1.0, 1.0, 0.5, 1.0, 1.0],
            [
                (0, 0.0019964662, 0.39793593),
                (256, -2.5534859, 0.8326609),
                (2048, 2.9797418, -1.4620826),
                (4095, 0.64518976, -0.19656008),
            ],
        ),
    ] {
        let mut state = SpreaderState::new(&params, 48_000.0);
        for frame in 0..4096 {
            let left = (std::f64::consts::TAU * 440.0 * frame as f64 / 48_000.0).sin() as f32;
            let right =
                (0.4 * (std::f64::consts::TAU * 660.0 * frame as f64 / 48_000.0).cos()) as f32;
            let output = state.process_frame(left, right);
            if let Some(&(_, expected_left, expected_right)) =
                expected.iter().find(|v| v.0 == frame)
            {
                assert!((output.0 - expected_left).abs() < 0.0002, "frame {frame}");
                assert!((output.1 - expected_right).abs() < 0.0002, "frame {frame}");
            }
        }
    }
}
