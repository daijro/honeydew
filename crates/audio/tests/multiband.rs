// The module uses `crate::biquad::flush`.
#[allow(dead_code)]
#[path = "../src/biquad.rs"]
mod biquad;
#[path = "../src/multiband.rs"]
mod multiband;

use multiband::MultibandState;

const DEFAULT: [f32; 31] = [
    1.0, 0.5, 0.3, 0.7, 0.0, 0.0, 0.5, 1.0, 0.0, 0.3, 0.3, 0.5, 0.0, 0.5, 1.0, 0.0, 0.3, 0.3, 0.5,
    0.0, 0.5, 1.0, 0.0, 0.3, 0.3, 0.5, 0.0, 0.5, 0.5, 0.5, 0.0,
];

fn tone(state: &mut MultibandState, frequency: f32, frames: usize) -> f32 {
    let mut sum = 0.0;
    for i in 0..frames {
        let sample = (std::f32::consts::TAU * frequency * i as f32 / 48_000.0).sin() * 0.5;
        let (left, right) = state.process_frame(sample, sample);
        if i > frames / 2 {
            sum += left * left + right * right;
        }
    }
    (sum / frames as f32).sqrt()
}

#[test]
fn split_and_solo_select_the_expected_frequency_band() {
    let mut low = MultibandState::new(&DEFAULT, 48_000.0);
    low.set_solo_band(Some(0));
    let mut high = MultibandState::new(&DEFAULT, 48_000.0);
    high.set_solo_band(Some(2));
    assert!(tone(&mut low, 50.0, 48_000) > 5.0 * tone(&mut high, 50.0, 48_000));

    let mut low = MultibandState::new(&DEFAULT, 48_000.0);
    low.set_solo_band(Some(0));
    let mut high = MultibandState::new(&DEFAULT, 48_000.0);
    high.set_solo_band(Some(2));
    assert!(tone(&mut high, 5_000.0, 24_000) > 5.0 * tone(&mut low, 5_000.0, 24_000));
}

#[test]
fn band_compression_and_gain_change_low_band_level() {
    let mut plain = MultibandState::new(&DEFAULT, 48_000.0);
    plain.set_solo_band(Some(0));
    let mut params = DEFAULT;
    params[7] = 0.35;
    params[8] = 1.0;
    params[11] = 0.4;
    let mut compressed = MultibandState::new(&params, 48_000.0);
    compressed.set_solo_band(Some(0));
    assert!(tone(&mut compressed, 80.0, 48_000) < 0.5 * tone(&mut plain, 80.0, 48_000));
}

#[test]
fn band_bypass_ignores_its_compressor_and_mute_removes_it() {
    let mut params = DEFAULT;
    params[6] = 1.0;
    params[7] = 0.0;
    params[8] = 1.0;
    params[11] = 0.0;
    params[12] = 0.5;
    let mut bypass = MultibandState::new(&params, 48_000.0);
    bypass.set_solo_band(Some(0));
    let bypass_level = tone(&mut bypass, 80.0, 24_000);
    params[12] = 1.0;
    let mut muted = MultibandState::new(&params, 48_000.0);
    muted.set_solo_band(Some(0));
    assert!(bypass_level > 0.05);
    assert!(tone(&mut muted, 80.0, 24_000) < 0.001);
}

#[test]
fn master_gain_dry_mix_and_safe_width_affect_output() {
    let mut normal = MultibandState::new(&DEFAULT, 48_000.0);
    let normal_level = tone(&mut normal, 1_000.0, 24_000);
    let mut params = DEFAULT;
    params[1] = 1.0;
    params[5] = 1.0;
    let mut boosted = MultibandState::new(&params, 48_000.0);
    assert!(tone(&mut boosted, 1_000.0, 24_000) > 5.0 * normal_level);

    params[30] = 1.0;
    params[28] = 1.0;
    let mut safe = MultibandState::new(&params, 48_000.0);
    let mut full_params = params;
    full_params[30] = 0.0;
    let mut full = MultibandState::new(&full_params, 48_000.0);
    let mut difference = 0.0;
    for i in 0..8192 {
        let left = (std::f32::consts::TAU * 997.0 * i as f32 / 48_000.0).sin() * 0.3;
        let right = (std::f32::consts::TAU * 1_031.0 * i as f32 / 48_000.0).sin() * 0.2;
        let a = safe.process_frame(left, right);
        let b = full.process_frame(left, right);
        difference += (a.0 - b.0).abs() + (a.1 - b.1).abs();
    }
    assert!(difference > 1.0);
}

#[test]
fn refreshing_crossover_and_resetting_keep_output_finite() {
    let mut state = MultibandState::new(&DEFAULT, 48_000.0);
    tone(&mut state, 500.0, 4096);
    let mut params = DEFAULT;
    params[2] = 0.4;
    params[3] = 0.8;
    state.refresh(&params, 48_000.0);
    for i in 0..8192 {
        let input = if i == 0 { 1.0 } else { 0.0 };
        let output = state.process_frame(input, -input);
        assert!(output.0.is_finite() && output.1.is_finite());
    }
    state.reset();
    assert!(state.process_frame(0.5, -0.5).0.is_finite());
}

#[test]
fn each_band_reports_its_levels_for_the_band_graph() {
    // A 100 Hz tone (0.5) is in the low band: over a period its input and output levels peak above 0.3, the high
    // band's stay low. The default low band is bypassed (p12 0.5, `Bps`), so its gain reads 1; the mid band
    // compresses nothing below its threshold.
    let mut state = MultibandState::new(&DEFAULT, 48_000.0);
    tone(&mut state, 100.0, 4_800);
    let mut peaks = [[0.0f32; 3]; 3];
    for i in 0..480 {
        let sample = (std::f32::consts::TAU * 100.0 * i as f32 / 48_000.0).sin() * 0.5;
        state.process_frame(sample, sample);
        for (peak, band) in peaks.iter_mut().zip(state.band_levels()) {
            for (p, v) in peak.iter_mut().zip(band) {
                *p = p.max(v);
            }
        }
    }
    let [low, mid, high] = peaks;
    assert!(low[0] > 0.3 && low[1] > 0.3, "{low:?}");
    assert!(high[0] < 0.05, "{high:?}");
    assert_eq!((low[2], mid[2]), (1.0, 1.0));
}
