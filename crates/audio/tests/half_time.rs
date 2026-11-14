use honeydew_audio::half_time::{HALF_TIME, LENGTH, SMOOTH, SPEED};
use honeydew_audio::{AudioEffect, ProcessContext};
use std::f64::consts::TAU;

const RATE: f32 = 48_000.0;
/// A beat at 120 BPM.
const BEAT: usize = 24_000;

/// The 1013 Hz sine the tests play, 0.5 high.
fn input(n: f64) -> f64 {
    0.5 * (TAU * 1013.0 * n / f64::from(RATE)).sin()
}

/// Half Time at Length 1/4 bar (a beat) and `params` on [`input`], played from beat 0 at 120 BPM in 512-frame blocks
/// for four beats.
fn play(params: &[(usize, f32)]) -> Vec<f32> {
    let mut effect = AudioEffect::new(HALF_TIME, RATE).unwrap();
    // 1/4 bar, item 2 of 1/16 bar … 8 bars.
    effect.set_parameter(LENGTH, 2.0 / 7.0).unwrap();
    for &(index, value) in params {
        effect.set_parameter(index, value).unwrap();
    }
    let block = 512;
    let mut output = Vec::new();
    for start in (0..4 * BEAT).step_by(block) {
        let mut left: Vec<f32> = (start..start + block)
            .map(|n| input(n as f64) as f32)
            .collect();
        let mut right = left.clone();
        let context = ProcessContext {
            sample_rate: RATE,
            tempo_bpm: 120.0,
            start_beat: start as f64 / BEAT as f64,
            playing: true,
        };
        effect.process(&mut left, &mut right, context).unwrap();
        assert_eq!(left, right);
        output.extend(left);
    }
    output
}

/// Past each start's fade, frame k of a beat is the input at `rate`·k frames into it.
fn assert_rate(output: &[f32], rate: f64) {
    for beat in 1..4 {
        for k in (3000..BEAT).step_by(97) {
            let n = beat * BEAT;
            let expected = input(n as f64 + rate * k as f64) as f32;
            assert!(
                (output[n + k] - expected).abs() < 0.01,
                "rate {rate} beat {beat} frame {k}: {} {expected}",
                output[n + k]
            );
        }
    }
}

/// The largest step between two frames.
fn steepest(output: &[f32]) -> f32 {
    (output.windows(2))
        .map(|w| (w[1] - w[0]).abs())
        .fold(0.0, f32::max)
}

/// The input's largest step.
fn input_steepest() -> f32 {
    0.5 * (TAU * 1013.0 / f64::from(RATE)) as f32
}

/// Each beat plays its first half at half speed (from the beat on), and the cycles' starts crossfade, never stepping
/// steeper than the input (#91).
#[test]
fn each_cycle_plays_its_first_half_at_half_speed_on_the_beat() {
    let output = play(&[]);
    assert_rate(&output, 0.5);
    let (steepest, input) = (steepest(&output), input_steepest());
    assert!(steepest <= 1.05 * input, "{steepest} {input}");
}

/// Speed 1/4 plays each beat's first quarter at a quarter of the rate, and 2/3 its first two thirds at two thirds
/// (#122).
#[test]
fn speed_a_quarter_plays_the_cycles_first_quarter_at_a_quarter_rate() {
    // 2/3 · 1/2 · 1/4.
    assert_rate(&play(&[(SPEED, 1.0)]), 0.25);
    assert_rate(&play(&[(SPEED, 0.0)]), 2.0 / 3.0);
}

/// Smooth 0 cuts from one cycle to the next, a click; Smooth crossfades it away (#122).
#[test]
fn smooth_crossfades_the_click_at_each_cycle_start_away() {
    let input = input_steepest();
    let cut = steepest(&play(&[(SMOOTH, 0.0)]));
    assert!(cut > 3.0 * input, "{cut} {input}");
    let smooth = steepest(&play(&[(SMOOTH, 1.0)]));
    assert!(smooth <= 1.05 * input, "{smooth} {input}");
}
