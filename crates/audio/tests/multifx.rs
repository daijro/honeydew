use honeydew_audio::multifx::MultiFxState;
use honeydew_audio::{AudioEffect, ProcessContext};
use std::f32::consts::TAU;

fn params(kind: usize) -> [f32; 7] {
    [1.0, kind as f32 / 7.0, 0.5, 0.5, 5.0 / 9.0, 0.0, 0.0]
}

fn tone_level(kind: usize, x: f32, frequency: f32) -> f32 {
    let mut p = params(kind);
    p[2] = x;
    let mut state = MultiFxState::new(&p, 48_000.0);
    state.prepare_block(120.0, 0.0, 24_000);
    let mut energy = 0.0;
    for i in 0..24_000 {
        let input = (TAU * frequency * i as f32 / 48_000.0).sin();
        let (out, _) = state.process_frame(input, 0.0).unwrap();
        if i >= 12_000 {
            energy += out * out;
        }
    }
    (energy / 12_000.0).sqrt()
}

#[test]
fn delay_and_reverb_modes_produce_audio() {
    for kind in [0, 1] {
        let p = params(kind);
        let mut state = MultiFxState::new(&p, 48_000.0);
        state.prepare_block(120.0, 0.0, 48_000);
        let mut energy = 0.0;
        for i in 0..48_000 {
            // A 2000-frame step: the feed ramps in by 0.005 per frame. Only the wet path follows it.
            let (left, right) = state
                .process_frame(if i < 2_000 { 0.5 } else { 0.0 }, 0.0)
                .unwrap();
            if i > 2_100 {
                energy += left.abs() + right.abs();
            }
        }
        assert!(energy > 0.01, "kind {kind}: {energy}");
    }
}

#[test]
fn public_effect_runs_the_default_delay() {
    let mut effect = AudioEffect::new(4, 48_000.0).unwrap();
    assert_eq!(effect.parameter(1), Some(0.0));
    let mut left = vec![0.0; 48_000];
    let mut right = vec![0.0; 48_000];
    left[..2_000].fill(0.5);
    effect
        .process(
            &mut left,
            &mut right,
            ProcessContext {
                sample_rate: 48_000.0,
                tempo_bpm: 120.0,
                start_beat: 0.0,
                playing: true,
            },
        )
        .unwrap();
    assert!(
        left.iter()
            .skip(2_100)
            .map(|value| value.abs())
            .sum::<f32>()
            > 0.01
    );
}

#[test]
fn panner_distance_attenuates_the_far_channel() {
    let mut near_params = params(6);
    near_params[2] = 0.0;
    near_params[3] = 0.0;
    let mut far_params = near_params;
    far_params[3] = 1.0;
    let mut near = MultiFxState::new(&near_params, 48_000.0);
    let mut far = MultiFxState::new(&far_params, 48_000.0);
    near.prepare_block(120.0, 0.0, 3_000);
    far.prepare_block(120.0, 0.0, 3_000);
    for _ in 0..2_000 {
        near.process_frame(0.0, 0.0).unwrap();
        far.process_frame(0.0, 0.0).unwrap();
    }
    near.process_frame(0.0, 1.0).unwrap();
    far.process_frame(0.0, 1.0).unwrap();
    let mut near_right = 0.0;
    let mut far_right = 0.0;
    for _ in 0..200 {
        let (_, a) = near.process_frame(0.0, 0.0).unwrap();
        let (_, b) = far.process_frame(0.0, 0.0).unwrap();
        near_right += a.abs();
        far_right += b.abs();
    }
    assert!(near_right > 0.5, "near = {near_right}");
    assert!(far_right < near_right * 0.05, "far = {far_right}");
}

#[test]
fn flanger_x_shortens_the_common_stereo_delay() {
    let mut long_params = params(4);
    long_params[2] = 0.0;
    long_params[3] = 0.0;
    let mut short_params = long_params;
    short_params[2] = 1.0;
    let mut long = MultiFxState::new(&long_params, 48_000.0);
    let mut short = MultiFxState::new(&short_params, 48_000.0);
    long.prepare_block(120.0, 0.0, 2_400);
    short.prepare_block(120.0, 0.0, 2_400);
    for _ in 0..2_000 {
        long.process_frame(0.0, 0.0).unwrap();
        short.process_frame(0.0, 0.0).unwrap();
    }
    long.process_frame(1.0, 0.0).unwrap();
    short.process_frame(1.0, 0.0).unwrap();
    let mut early_long = 0.0;
    let mut early_short = 0.0;
    for _ in 0..24 {
        let (ll, lr) = long.process_frame(0.0, 0.0).unwrap();
        let (sl, sr) = short.process_frame(0.0, 0.0).unwrap();
        assert!((ll - lr).abs() < 1e-6);
        assert!((sl - sr).abs() < 1e-6);
        early_long += ll.abs();
        early_short += sl.abs();
    }
    assert!(early_short > 0.1, "early short = {early_short}");
    assert!(early_long < 1e-6, "early long = {early_long}");
}

#[test]
fn vowel_axis_moves_the_low_formant() {
    let first = tone_level(7, 0.0, 250.0);
    let second = tone_level(7, 1.0 / 3.0, 250.0);
    let wrapped = tone_level(7, 1.0, 250.0);
    assert!(first > second * 1.2, "first = {first}, second = {second}");
    assert!(
        (first - wrapped).abs() < 0.001,
        "first = {first}, wrapped = {wrapped}"
    );
}

#[test]
fn vowel_formants_have_a_stereo_independent_tail() {
    let p = params(7);
    let mut state = MultiFxState::new(&p, 48_000.0);
    state.prepare_block(120.0, 0.0, 2_000);
    let mut tail = 0.0;
    for i in 0..2_000 {
        let (left, right) = state
            .process_frame(if i == 0 { 1.0 } else { 0.0 }, 0.0)
            .unwrap();
        assert_eq!(right, 0.0);
        if i > 10 {
            tail += left.abs();
        }
    }
    assert!(tail > 1e-5);
}

#[test]
fn phaser_has_a_stereo_independent_tail_and_feedback_controls_its_decay() {
    let mut no_feedback_params = params(5);
    no_feedback_params[2] = 0.2;
    no_feedback_params[3] = 0.0;
    let mut high_feedback_params = no_feedback_params;
    high_feedback_params[3] = 1.0;
    let mut no_feedback = MultiFxState::new(&no_feedback_params, 48_000.0);
    let mut high_feedback = MultiFxState::new(&high_feedback_params, 48_000.0);
    no_feedback.prepare_block(120.0, 0.0, 2_000);
    high_feedback.prepare_block(120.0, 0.0, 2_000);
    let mut low_tail = 0.0;
    let mut high_tail = 0.0;
    for i in 0..2_000 {
        let input = if i == 0 { 1.0 } else { 0.0 };
        let (low_l, low_r) = no_feedback.process_frame(input, 0.0).unwrap();
        let (high_l, high_r) = high_feedback.process_frame(input, 0.0).unwrap();
        assert_eq!(low_r, 0.0);
        assert_eq!(high_r, 0.0);
        if i >= 40 {
            low_tail += low_l.abs();
            high_tail += high_l.abs();
        }
    }
    assert!(low_tail > 1e-5);
    assert!(
        high_tail > low_tail * 1.2,
        "high = {high_tail}, low = {low_tail}"
    );
}

#[test]
fn rate_and_shape_select_transport_modulation() {
    let mut p = params(2);
    p[2] = 0.2;
    p[5] = 1.0;
    p[4] = 0.0;
    p[6] = 0.5;
    let mut saw_at_start = MultiFxState::new(&p, 48_000.0);
    let mut saw_at_end = MultiFxState::new(&p, 48_000.0);
    saw_at_start.prepare_block(120.0, 0.0, 12_000);
    saw_at_end.prepare_block(120.0, 0.12, 12_000);
    let mut start_energy = 0.0;
    let mut end_energy = 0.0;
    for i in 0..12_000 {
        let input = (TAU * 5_000.0 * i as f32 / 48_000.0).sin();
        let (a, _) = saw_at_start.process_frame(input, 0.0).unwrap();
        let (b, _) = saw_at_end.process_frame(input, 0.0).unwrap();
        if i > 6_000 {
            start_energy += a * a;
            end_energy += b * b;
        }
    }
    assert!(start_energy > end_energy * 2.0);
}

#[test]
fn lfo_keeps_phase_when_transport_position_is_unavailable() {
    let mut p = params(2);
    p[2] = 0.2;
    p[5] = 1.0;
    let mut continuing = MultiFxState::new(&p, 48_000.0);
    let mut located = MultiFxState::new(&p, 48_000.0);
    continuing.prepare_block(120.0, 0.0, 6_000);
    for _ in 0..6_000 {
        continuing.process_frame(0.0, 0.0).unwrap();
    }
    continuing.prepare_block(120.0, -1.0, 6_000);
    located.prepare_block(120.0, 0.25, 6_000);
    let mut difference = 0.0;
    for i in 0..6_000 {
        let input = (TAU * 5_000.0 * i as f32 / 48_000.0).sin();
        let (a, _) = continuing.process_frame(input, 0.0).unwrap();
        let (b, _) = located.process_frame(input, 0.0).unwrap();
        if i > 3_000 {
            difference += (a - b).abs();
        }
    }
    assert!(difference < 0.001, "difference = {difference}");
}

#[test]
fn low_pass_frequency_axis_opens_the_high_band() {
    let closed = tone_level(2, 0.0, 5_000.0);
    let open = tone_level(2, 1.0, 5_000.0);
    assert!(closed < 0.1, "closed = {closed}");
    assert!(open > 0.45, "open = {open}");
}

#[test]
fn high_pass_frequency_axis_removes_the_low_band() {
    let open = tone_level(3, 0.0, 100.0);
    let closed = tone_level(3, 1.0, 100.0);
    assert!(open > 0.45, "open = {open}");
    assert!(closed < 0.1, "closed = {closed}");
}

#[test]
fn lfo_changes_cutoff_at_the_selected_transport_phase() {
    let mut p = params(2);
    p[2] = 0.2;
    p[5] = 1.0;
    let mut rising = MultiFxState::new(&p, 48_000.0);
    let mut neutral = MultiFxState::new(&p, 48_000.0);
    rising.prepare_block(120.0, 0.25, 12_000);
    neutral.prepare_block(120.0, 0.0, 12_000);
    let mut rising_energy = 0.0;
    let mut neutral_energy = 0.0;
    for i in 0..12_000 {
        let input = (TAU * 5_000.0 * i as f32 / 48_000.0).sin();
        let (a, _) = rising.process_frame(input, 0.0).unwrap();
        let (b, _) = neutral.process_frame(input, 0.0).unwrap();
        if i > 6_000 {
            rising_energy += a * a;
            neutral_energy += b * b;
        }
    }
    assert!(rising_energy > neutral_energy * 2.0);
}

#[test]
fn changing_xy_does_not_flush_filter_history() {
    let mut p = params(2);
    p[2] = 0.5;
    let mut state = MultiFxState::new(&p, 48_000.0);
    state.prepare_block(120.0, 0.0, 4_000);
    for _ in 0..3_000 {
        state.process_frame(0.0, 0.0).unwrap();
    }
    state.process_frame(1.0, 0.0).unwrap();
    p[2] = 0.6;
    state.refresh(&p, 48_000.0);
    state.prepare_block(120.0, 0.0, 64);
    let mut tail = 0.0;
    for _ in 0..64 {
        let (out, _) = state.process_frame(0.0, 0.0).unwrap();
        tail += out.abs();
    }
    assert!(tail > 1e-5);
}

#[test]
fn changing_kind_keeps_the_previous_filter_tail() {
    let mut p = params(2);
    p[2] = 0.3;
    let mut state = MultiFxState::new(&p, 48_000.0);
    state.prepare_block(120.0, 0.0, 2_000);
    for _ in 0..1_000 {
        state.process_frame(0.0, 0.0).unwrap();
    }
    state.process_frame(1.0, 0.0).unwrap();
    p[1] = 3.0 / 7.0;
    state.refresh(&p, 48_000.0);
    state.prepare_block(120.0, 0.0, 1);
    state.process_frame(0.0, 0.0).unwrap();
    p[1] = 2.0 / 7.0;
    state.refresh(&p, 48_000.0);
    state.prepare_block(120.0, 0.0, 64);
    let mut tail = 0.0;
    for _ in 0..64 {
        let (left, _) = state.process_frame(0.0, 0.0).unwrap();
        tail += left.abs();
    }
    assert!(tail > 1e-5);
}

const FS: usize = 48_000;
const CTX: ProcessContext = ProcessContext {
    sample_rate: 48_000.0,
    tempo_bpm: 120.0,
    start_beat: 0.0,
    playing: true,
};

/// The tail probe: a 21 ms 440 Hz burst at half scale on the left, then silence, `seconds` long in
/// all, through a MultiFX of `kind` at its defaults. Returns the left and right outputs.
fn burst_through(kind: usize, seconds: usize) -> (Vec<f32>, Vec<f32>) {
    let mut effect = AudioEffect::new(4, 48_000.0).unwrap();
    effect.set_parameter(1, kind as f32 / 7.0).unwrap();
    let mut left: Vec<f32> = (0..FS * seconds)
        .map(|i| match i {
            0..1008 => 0.5 * (TAU * 440.0 * i as f32 / 48_000.0).sin(),
            _ => 0.0,
        })
        .collect();
    let mut right = vec![0.0; left.len()];
    for (l, r) in left.chunks_mut(1000).zip(right.chunks_mut(1000)) {
        effect.process(l, r, CTX).unwrap();
    }
    (left, right)
}

fn peak(samples: &[f32]) -> f32 {
    samples.iter().fold(0.0, |m, v| m.max(v.abs()))
}

#[test]
fn the_default_delay_follows_the_round_8_law_and_its_echoes_die_out() {
    // Engine notes item 21. Defaults: Delay, X = Y = 0.5, a 1-beat rate, LFO amount 0, 120 BPM.
    let (left, right) = burst_through(0, 30);
    assert!(right.iter().all(|&v| v == 0.0));
    // The law written out independently: ring trunc(1.5 Fs); R_eff = 2R, so
    // u = (120R/B)/3 = 1/3 and the tap is clamp(trunc((ring − 1)·u), 1, ring − 1); r = X² × the ring sample one tap
    // back, a plain gain; (a, b) = (1, 0) at Y = 0.5.
    let ring_len = (1.5 * 48_000.0f32).trunc() as usize;
    let tap = (((ring_len - 1) as f32) * (1.0 / 3.0)).trunc() as usize;
    let (gain, a, b) = (0.25f32, 1.0f32, 0.0f32);
    let mut ring = vec![0.0f32; ring_len];
    let (mut cursor, mut ramp, mut first, mut second) = (0, 0.0f32, 0.0f32, 0.0f32);
    let mut worst = 0.0f32;
    for (i, &out) in left.iter().enumerate() {
        let input = if i < 1008 {
            0.5 * (TAU * 440.0 * i as f32 / 48_000.0).sin()
        } else {
            0.0
        };
        ramp += 0.005 * (1.0 - ramp);
        let r = gain * ring[(cursor + ring_len - tap) % ring_len];
        let old_first = first;
        first += a * (r - old_first);
        second += b * (old_first - second);
        let wet = old_first - second;
        ring[cursor] = input * ramp + wet;
        cursor = (cursor + 1) % ring_len;
        worst = worst.max((out - (input + wet)).abs());
    }
    assert!(
        worst < 1e-6,
        "the engine strays from the spec's equations by {worst}"
    );
    // One pass is the tap plus the first stage's one-frame delay: 24000 frames, 0.5 s, one beat. Each repeat is
    // X² = 0.25 (−12 dB) of the one before, at every frequency.
    assert_eq!(tap + 1, 24_000);
    let passes: Vec<f32> = left.chunks(24_000).map(peak).collect();
    for k in 1..5 {
        assert!(
            (passes[k + 1] / passes[k] - 0.25).abs() < 1e-3,
            "{passes:?}"
        );
    }
    // So the echoes are below −60 dB after 3 s and the tail ends: the tail no longer sustains itself.
    assert!(passes[6] < 1e-3 * passes[0], "{passes:?}");
    assert!(left[FS * 29..].iter().all(|&v| v == 0.0));
}

#[test]
fn every_kind_s_tail_ends_without_subnormals() {
    // Subnormal floats in a feedback state cost the audio thread up to 100× per operation and never reach 0.
    for kind in 0..8 {
        let (left, right) = burst_through(kind, 10);
        let last = FS * 9..FS * 10;
        let subnormal = left[last.clone()]
            .iter()
            .chain(&right[last.clone()])
            .filter(|v| v.is_subnormal())
            .count();
        assert_eq!(subnormal, 0, "kind {kind}");
        // Every kind dies out: below −180 dB after 9 s.
        let tail = peak(&left[last.clone()]).max(peak(&right[last]));
        assert!(tail < 1e-9, "kind {kind}: {tail}");
    }
}

#[test]
fn reset_is_a_fresh_state_for_every_kind() {
    // `reset` runs on the audio thread (Seek), so it clears in place; it must still equal a new state.
    let run = |state: &mut MultiFxState| -> Vec<(f32, f32)> {
        state.prepare_block(120.0, 0.0, 4_800);
        (0..4_800)
            .map(|i| {
                let input = if i < 480 { 0.5 } else { 0.0 };
                state.process_frame(input, 0.5 * input).unwrap()
            })
            .collect()
    };
    for kind in 0..8 {
        let mut state = MultiFxState::new(&params(kind), 48_000.0);
        let fresh = run(&mut state);
        run(&mut state);
        state.reset();
        assert_eq!(run(&mut state), fresh, "kind {kind}");
    }
}

/// A small deterministic generator for randomized runs (no dependency).
struct Lcg(u64);

impl Lcg {
    fn next(&mut self) -> f32 {
        self.0 = self
            .0
            .wrapping_mul(6_364_136_223_846_793_005)
            .wrapping_add(1_442_695_040_888_963_407);
        (self.0 >> 40) as f32 / (1u64 << 24) as f32
    }
}

#[test]
fn the_flanger_near_x_1_never_reads_past_its_ring() {
    // A fractional read at cursor − delay can round to exactly the ring length.
    let mut random = Lcg(0x5eed);
    for _ in 0..400 {
        let sample_rate = [44_100.0, 48_000.0, 96_000.0][(random.next() * 3.0) as usize % 3];
        let mut effect = AudioEffect::new(4, sample_rate).unwrap();
        let values = [
            4.0 / 7.0,
            0.99,
            random.next(),
            random.next(),
            random.next() * 2.0 - 1.0,
            random.next(),
        ];
        for (index, value) in [1, 2, 3, 4, 5, 6].into_iter().zip(values) {
            effect.set_parameter(index, value.clamp(0.0, 1.0)).unwrap();
        }
        let mut beat = f64::from(random.next()) * 16.0;
        let tempo = 30.0 + random.next() * 270.0;
        for _ in 0..8 {
            let frames = 1 + (random.next() * 700.0) as usize;
            let mut left: Vec<f32> = (0..frames).map(|_| random.next() - 0.5).collect();
            let mut right = left.clone();
            let context = ProcessContext {
                sample_rate,
                tempo_bpm: tempo,
                start_beat: beat,
                playing: true,
            };
            effect.process(&mut left, &mut right, context).unwrap();
            beat += f64::from(tempo) / 60.0 * frames as f64 / f64::from(sample_rate);
        }
    }
}

#[test]
fn a_new_sample_rate_gives_a_fresh_state_at_that_rate() {
    let run = |state: &mut MultiFxState, frames: usize| -> Vec<(f32, f32)> {
        state.prepare_block(120.0, 0.0, frames);
        (0..frames)
            .map(|i| {
                let input = (TAU * 300.0 * i as f32 / 9_600.0).sin();
                state.process_frame(input, 0.5 * input).unwrap()
            })
            .collect()
    };
    for kind in 0..8 {
        let p = params(kind);
        let mut moved = MultiFxState::new(&p, 44_100.0);
        run(&mut moved, 4_410);
        moved.refresh(&p, 96_000.0);
        let mut fresh = MultiFxState::new(&p, 96_000.0);
        assert_eq!(
            run(&mut moved, 9_600),
            run(&mut fresh, 9_600),
            "kind {kind}"
        );
    }
}

#[test]
fn a_flanger_without_input_is_skipped_after_a_quarter_second() {
    // Stop and tails: X = 0, Y = 1 is a 240-frame line fed back at −0.96, still ringing at 0.25 s.
    let mut p = params(4);
    p[2] = 0.0;
    p[3] = 1.0;
    let mut state = MultiFxState::new(&p, 48_000.0);
    state.prepare_block(120.0, 0.0, 24_000);
    let output: Vec<f32> = (0..24_000)
        .map(|i| {
            state
                .process_frame(if i < 480 { 0.5 } else { 0.0 }, 0.0)
                .unwrap()
                .0
        })
        .collect();
    // The last input above −80 dB is frame 479; 0.25 s later the processor is skipped and its output is its input.
    assert!(peak(&output[11_000..12_000]) > 1e-3);
    assert!(output[12_480 + 1..].iter().all(|&v| v == 0.0));
}

#[test]
fn below_1_bpm_nothing_is_processed() {
    // A flanger, whose wet line is audible within a few frames.
    let mut state = MultiFxState::new(&params(4), 48_000.0);
    state.prepare_block(120.0, 0.0, 2_000);
    for i in 0..2_000 {
        state.process_frame((i as f32 * 0.01).sin(), 0.0).unwrap();
    }
    state.prepare_block(0.5, 0.0, 100);
    for i in 0..100 {
        let input = i as f32 / 100.0;
        assert_eq!(state.process_frame(input, -input).unwrap(), (input, -input));
    }
}

#[test]
fn the_reverb_ignores_the_lfo() {
    let mut p = params(1);
    p[5] = 1.0;
    p[6] = 0.5;
    let mut early = MultiFxState::new(&p, 48_000.0);
    let mut late = MultiFxState::new(&p, 48_000.0);
    early.prepare_block(120.0, 0.0, 4_800);
    late.prepare_block(120.0, 0.6, 4_800);
    for i in 0..4_800 {
        let input = if i < 480 { 0.5 } else { 0.0 };
        assert_eq!(
            early.process_frame(input, input).unwrap(),
            late.process_frame(input, input).unwrap()
        );
    }
}

#[test]
fn a_ringing_delay_echo_does_not_pass_through_the_low_pass() {
    // The bank runs Flanger, Phaser, LP, HP, Delay, Reverb, Panner, Vox (kind order), so the
    // Delay's tail comes after the filters.
    // X = 0.5 is shared: the echo gain X² = 0.25, and the LP's cutoff 2^(6 + 4.14) ≈ 1.1 kHz.
    let p = params(0);
    let mut state = MultiFxState::new(&p, 48_000.0);
    state.prepare_block(120.0, 0.0, 24_000);
    for i in 0..24_000 {
        // A 5 kHz burst; the first echo comes back one beat (24000 frames) later.
        let input = if i < 2_400 {
            0.5 * (TAU * 5_000.0 * i as f32 / 48_000.0).sin()
        } else {
            0.0
        };
        state.process_frame(input, 0.0).unwrap();
    }
    state.refresh(&params(2), 48_000.0);
    state.prepare_block(120.0, 0.5, 2_400);
    let echo: Vec<f32> = (0..2_400)
        .map(|_| state.process_frame(0.0, 0.0).unwrap().0)
        .collect();
    // The echo is a quarter of the burst; four LP stages at 1.1 kHz would take 5 kHz down by about 100 dB.
    assert!(peak(&echo[1_200..]) > 0.1, "{}", peak(&echo[1_200..]));
}

#[test]
fn a_new_sample_rate_keeps_the_tempo_until_the_next_block() {
    // Below 1 BPM nothing is processed; a rate change before the next `prepare_block` keeps that tempo. (An LP that
    // ran would move the output at once: its rebuilt filter starts cleared.)
    let mut state = MultiFxState::new(&params(2), 48_000.0);
    state.prepare_block(0.5, 0.0, 100);
    state.refresh(&params(2), 96_000.0);
    for i in 0..100 {
        let input = (i as f32 * 0.01).sin();
        assert_eq!(
            state.process_frame(input, 0.5 * input).unwrap(),
            (input, 0.5 * input)
        );
    }
}
