use honeydew_audio::half_time::{HALF_TIME, LENGTH};
use honeydew_audio::{AudioEffect, ProcessContext};
use std::f64::consts::TAU;

/// Half Time at Length 1/4 bar (a beat) on a 1013 Hz sine, played from beat 0 at 120 BPM in 512-frame blocks: each beat
/// plays its first half at half speed (from the beat on), and the cycles' starts crossfade, never stepping
/// steeper than the input (#91).
#[test]
fn each_cycle_plays_its_first_half_at_half_speed_on_the_beat() {
    let (rate, tempo) = (48_000.0, 120.0);
    let mut effect = AudioEffect::new(HALF_TIME, rate).unwrap();
    effect.set_parameter(LENGTH, 0.0).unwrap();
    let input = |n: f64| 0.5 * (TAU * 1013.0 * n / f64::from(rate)).sin();
    let (beat_frames, block) = (24_000, 512);
    let mut output = Vec::new();
    for start in (0..4 * beat_frames).step_by(block) {
        let mut left: Vec<f32> = (start..start + block)
            .map(|n| input(n as f64) as f32)
            .collect();
        let mut right = left.clone();
        let context = ProcessContext {
            sample_rate: rate,
            tempo_bpm: tempo,
            start_beat: start as f64 / beat_frames as f64,
            playing: true,
        };
        effect.process(&mut left, &mut right, context).unwrap();
        assert_eq!(left, right);
        output.extend(left);
    }
    // Past each start's 10 ms fade, frame k of the beat is the input at k/2 frames into it.
    for beat in 1..4 {
        for k in (1000..beat_frames).step_by(97) {
            let n = beat * beat_frames;
            let expected = input(n as f64 + k as f64 / 2.0) as f32;
            assert!(
                (output[n + k] - expected).abs() < 0.01,
                "beat {beat} frame {k}: {} {expected}",
                output[n + k]
            );
        }
    }
    let steepest = output
        .windows(2)
        .map(|w| (w[1] - w[0]).abs())
        .fold(0.0, f32::max);
    let input_steepest = 0.5 * (TAU * 1013.0 / f64::from(rate)) as f32;
    assert!(
        steepest <= 1.05 * input_steepest,
        "{steepest} {input_steepest}"
    );
}
