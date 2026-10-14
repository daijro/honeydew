use honeydew_audio::{AudioEffect, ProcessContext};

const CTX: ProcessContext = ProcessContext {
    sample_rate: 48_000.0,
    tempo_bpm: 120.0,
    start_beat: 0.0,
    playing: true,
};

fn impulse(effect: &mut AudioEffect, frames: usize) -> Vec<f32> {
    let mut left = vec![0.0; frames];
    let mut right = vec![0.0; frames];
    left[0] = 1.0;
    effect.process(&mut left, &mut right, CTX).unwrap();
    left
}

#[test]
fn manual_pitch_and_octave_set_the_comb_period() {
    let mut base = AudioEffect::new(40, CTX.sample_rate).unwrap();
    base.set_parameter(7, 0.0).unwrap();
    base.set_parameter(2, (999.0_f32 / 2499.0).powf(0.25))
        .unwrap();
    base.set_parameter(3, 0.0).unwrap();
    base.set_parameter(4, 1.0).unwrap();
    base.reset();
    let first = impulse(&mut base, 160);
    let peak = first[2..]
        .iter()
        .enumerate()
        .max_by(|a, b| a.1.abs().total_cmp(&b.1.abs()))
        .unwrap()
        .0
        + 2;

    let mut up = AudioEffect::new(40, CTX.sample_rate).unwrap();
    up.set_parameter(7, 0.0).unwrap();
    up.set_parameter(2, (999.0_f32 / 2499.0).powf(0.25))
        .unwrap();
    up.set_parameter(3, 0.0).unwrap();
    up.set_parameter(4, 1.0).unwrap();
    up.set_parameter(5, 0.75).unwrap();
    up.reset();
    let octave = impulse(&mut up, 160);
    let upper_peak = octave[2..]
        .iter()
        .enumerate()
        .max_by(|a, b| a.1.abs().total_cmp(&b.1.abs()))
        .unwrap()
        .0
        + 2;
    assert!((peak as isize - 50).abs() <= 2);
    assert!((upper_peak as isize - 26).abs() <= 2);
}

#[test]
fn note_mode_uses_current_note_and_keeps_last_when_released() {
    let mut comb = AudioEffect::new(40, CTX.sample_rate).unwrap();
    comb.set_parameter(1, 0.5).unwrap();
    comb.set_parameter(3, 0.0).unwrap();
    comb.set_parameter(4, 1.0).unwrap();
    comb.set_comb_note(Some(69)).unwrap();
    comb.reset();
    let a4 = impulse(&mut comb, 180);
    assert!(a4[110..115].iter().any(|sample| sample.abs() > 0.1));

    comb.set_comb_note(None).unwrap();
    comb.reset();
    let remembered = impulse(&mut comb, 180);
    let first_peak = a4[2..]
        .iter()
        .enumerate()
        .max_by(|a, b| a.1.abs().total_cmp(&b.1.abs()))
        .unwrap()
        .0;
    let second_peak = remembered[2..]
        .iter()
        .enumerate()
        .max_by(|a, b| a.1.abs().total_cmp(&b.1.abs()))
        .unwrap()
        .0;
    assert_eq!(first_peak, second_peak);
}

#[test]
fn feedback_polarity_changes_second_echo_not_first_echo() {
    let mut positive = AudioEffect::new(40, CTX.sample_rate).unwrap();
    positive.set_parameter(7, 0.0).unwrap();
    positive
        .set_parameter(2, (999.0_f32 / 2499.0).powf(0.25))
        .unwrap();
    positive.set_parameter(3, 0.75).unwrap();
    positive.set_parameter(4, 1.0).unwrap();
    positive.reset();
    let normal = impulse(&mut positive, 180);

    let mut negative = AudioEffect::new(40, CTX.sample_rate).unwrap();
    negative.set_parameter(7, 0.0).unwrap();
    negative
        .set_parameter(2, (999.0_f32 / 2499.0).powf(0.25))
        .unwrap();
    negative.set_parameter(3, 0.75).unwrap();
    negative.set_parameter(4, 1.0).unwrap();
    negative.set_parameter(8, 1.0 / 3.0).unwrap();
    negative.reset();
    let inverted = impulse(&mut negative, 180);
    assert!(
        normal[48..55]
            .iter()
            .zip(&inverted[48..55])
            .any(|(a, b)| (*a - *b).abs() < 1e-5 && a.abs() > 0.1)
    );
    assert!(
        normal[96..106]
            .iter()
            .zip(&inverted[96..106])
            .any(|(a, b)| *a > 0.1 && *b < -0.1)
    );
}
