//! Transistor Bass (ID 19), rendered offline against its DSP laws.

use honeydew_audio::ProcessContext;
use honeydew_audio::instrument::Instrument;
use honeydew_audio::transistor_bass::TransistorBass;

const RATE: f32 = 48_000.0;

fn frames(seconds: f32) -> usize {
    (seconds * RATE) as usize
}

/// Renders `frames` frames and returns (left, right).
fn render_lr(synth: &mut TransistorBass, frames: usize) -> (Vec<f32>, Vec<f32>) {
    let mut left = vec![0.0; frames];
    let mut right = vec![0.0; frames];
    let context = ProcessContext {
        sample_rate: RATE,
        tempo_bpm: 120.0,
        start_beat: 0.0,
        playing: false,
    };
    synth.process(&mut left, &mut right, context);
    (left, right)
}

/// Renders `frames` frames of a centred note; both sides are equal.
fn render(synth: &mut TransistorBass, frames: usize) -> Vec<f32> {
    let (left, right) = render_lr(synth, frames);
    assert_eq!(left, right);
    left
}

fn rms(samples: &[f32]) -> f32 {
    (samples.iter().map(|s| s * s).sum::<f32>() / samples.len() as f32).sqrt()
}

/// The fundamental in Hz: the largest autocorrelation between 20 Hz and 2 kHz, refined by a parabola.
fn pitch(samples: &[f32]) -> f32 {
    let ac = |lag: usize| -> f64 {
        (samples[..samples.len() - lag].iter())
            .zip(&samples[lag..])
            .map(|(a, b)| f64::from(*a) * f64::from(*b))
            .sum()
    };
    let lags = (RATE / 2000.0) as usize..(RATE / 20.0) as usize;
    let best = lags.max_by(|&a, &b| ac(a).total_cmp(&ac(b))).unwrap();
    let (a, b, c) = (ac(best - 1), ac(best), ac(best + 1));
    let lag = best as f64 + 0.5 * (a - c) / (a - 2.0 * b + c);
    RATE / lag as f32
}

/// The amplitude of the `hz` component: a single-bin DFT under a Hann window.
fn amplitude(samples: &[f32], hz: f32) -> f32 {
    let w = std::f64::consts::TAU * f64::from(hz) / f64::from(RATE);
    let n = samples.len() as f64;
    let (re, im) = samples
        .iter()
        .enumerate()
        .fold((0.0, 0.0), |(re, im), (i, s)| {
            let i = i as f64;
            let s = f64::from(*s) * (1.0 - (std::f64::consts::TAU * i / n).cos());
            (re + s * (w * i).cos(), im + s * (w * i).sin())
        });
    (2.0 * (re * re + im * im).sqrt() / n) as f32
}

fn key_hz(key: f32) -> f32 {
    440.0 * 2f32.powf((key - 81.0) / 12.0)
}

/// Reso 0 and a saw, so the fundamental dominates for pitch tests.
fn plain(rate: f32) -> TransistorBass {
    let mut synth = TransistorBass::new(rate);
    synth.set_parameter(1, 1.0);
    synth.set_parameter(3, 0.0);
    synth
}

#[test]
fn silent_until_a_note_then_sounds() {
    let mut synth = TransistorBass::new(RATE);
    assert!(render(&mut synth, 1024).iter().all(|s| *s == 0.0));
    synth.note_on(48, 0.8, 0.5);
    let out = render(&mut synth, frames(0.5));
    assert!(out.iter().all(|s| s.is_finite()));
    let peak = out.iter().fold(0.0_f32, |m, s| m.max(s.abs()));
    assert!((0.02..2.0).contains(&peak), "peak {peak}");
}

#[test]
fn pitch_is_an_octave_below_3x_osc() {
    for (key, hz) in [(57, 110.0), (69, 220.0), (81, 440.0)] {
        let mut synth = plain(RATE);
        synth.note_on(key, 0.5, 0.5);
        let out = render(&mut synth, frames(0.4));
        let measured = pitch(&out[frames(0.1)..]);
        assert!(
            (measured / hz - 1.0).abs() < 0.003,
            "key {key}: {measured} Hz"
        );
    }
}

#[test]
fn last_note_priority_slides_and_returns_to_the_held_key() {
    let mut synth = plain(RATE);
    let window = |synth: &mut TransistorBass| pitch(&render(synth, frames(0.3))[frames(0.15)..]);
    let near = |measured: f32, key: f32| (measured / key_hz(key) - 1.0).abs() < 0.005;

    synth.note_on(57, 0.5, 0.5);
    assert!(near(window(&mut synth), 57.0));
    synth.note_on(64, 0.5, 0.5);
    assert!(near(window(&mut synth), 64.0), "the newest key sounds");
    synth.note_on(60, 0.5, 0.5);
    assert!(near(window(&mut synth), 60.0));
    synth.note_off(64);
    assert!(
        near(window(&mut synth), 60.0),
        "an older key's release changes nothing"
    );
    synth.note_off(60);
    assert!(
        near(window(&mut synth), 57.0),
        "slides back to the held key"
    );
    synth.note_off(57);
    render(&mut synth, frames(0.25));
    assert!(render(&mut synth, 1024).iter().all(|s| *s == 0.0));
}

#[test]
fn legato_keeps_the_envelopes_running_and_a_new_note_restarts_them() {
    // Unaccented and Env Mod 0, so the output follows VEG.
    let legato = |retrigger: bool| {
        let mut synth = plain(RATE);
        synth.set_parameter(4, 0.0);
        synth.note_on(48, 0.5, 0.5);
        let before = rms(&render(&mut synth, frames(0.6))[frames(0.5)..]);
        if retrigger {
            synth.note_off(48);
        }
        synth.note_on(50, 0.5, 0.5);
        let after = rms(&render(&mut synth, frames(0.2))[frames(0.1)..]);
        after / before
    };
    assert!(legato(false) < 1.0, "slide: VEG keeps decaying");
    assert!(legato(true) > 2.0, "new note: VEG restarts at 1");
}

#[test]
fn accent_is_louder() {
    let level = |velocity: f32| {
        let mut synth = TransistorBass::new(RATE);
        synth.note_on(48, velocity, 0.5);
        rms(&render(&mut synth, frames(0.05)))
    };
    let ratio = level(1.0) / level(0.5);
    assert!((1.3..1.8).contains(&ratio), "accent/plain {ratio}");
    assert_eq!(
        level(0.5),
        level(98.0 / 127.0),
        "velocity has no other effect"
    );
    assert_eq!(level(1.0), level(99.0 / 127.0));
}

#[test]
fn waveform_endpoints_are_pulse_and_saw() {
    // A 50 % pulse has no even harmonics; a saw has them at 1/h. Key 45 = 55 Hz, under the resting cutoff.
    let second_over_first = |waveform: f32| {
        let mut synth = TransistorBass::new(RATE);
        synth.set_parameter(1, waveform);
        synth.set_parameter(3, 0.0);
        synth.note_on(45, 0.5, 0.5);
        let out = render(&mut synth, frames(0.5));
        let steady = &out[frames(0.1)..frames(0.5)]; // 22 periods
        amplitude(steady, 110.0) / amplitude(steady, 55.0)
    };
    assert!(second_over_first(0.0) < 1e-3, "pulse");
    assert!(second_over_first(1.0) > 0.3, "saw");
}

#[test]
fn release_closes_the_gate_and_rendering_stops_after_a_quarter_second() {
    let mut synth = TransistorBass::new(RATE);
    synth.note_on(48, 0.5, 0.5);
    let held = rms(&render(&mut synth, frames(0.1))[frames(0.05)..]);
    synth.note_off(48);
    let tail = render(&mut synth, frames(0.25));
    assert!(
        rms(&tail[frames(0.03)..frames(0.05)]) < held * 0.01,
        "5 ms VCA fade"
    );
    assert!(render(&mut synth, 4096).iter().all(|s| *s == 0.0));
}

#[test]
fn block_size_does_not_change_the_output() {
    let run = |blocks: &[usize]| {
        let mut synth = TransistorBass::new(RATE);
        synth.set_parameter(7, 1.0);
        synth.set_parameter(13, 0.5);
        synth.set_parameter(12, 0.3);
        let mut out = Vec::new();
        for (i, event) in [(48, true), (55, true), (55, false), (48, false), (36, true)]
            .into_iter()
            .enumerate()
        {
            match event {
                (key, true) => synth.note_on(key, if i % 2 == 0 { 1.0 } else { 0.5 }, 0.8),
                (key, false) => synth.note_off(key),
            }
            let mut left_to_render = frames(0.3);
            let mut b = 0;
            while left_to_render > 0 {
                let n = blocks[b % blocks.len()].min(left_to_render);
                let (l, r) = render_lr(&mut synth, n);
                out.extend(l.into_iter().zip(r));
                left_to_render -= n;
                b += 1;
            }
        }
        out
    };
    assert_eq!(run(&[frames(0.3)]), run(&[1, 7, 64, 333, 1023, 1500]));
}

#[test]
fn distortion_is_bounded_by_its_output_law() {
    // Stage 6 scales a tanh (≤ 1) after the 19475 Hz bilinear one-pole. At 48 kHz its pole is negative, so its
    // impulse response alternates and its L1 gain is 2g/(1 + g), g = tan(π·19475/48000) (≈ 1.53).
    let g = (std::f32::consts::PI * 19475.0 / RATE).tan();
    let lp_gain = 2.0 * g / (1.0 + g);
    for (drive, hp, vol) in [(0.3, 0.0, 0.5), (1.0, 0.0, 0.5), (0.1, 0.3, 0.8)] {
        let mut synth = TransistorBass::new(RATE);
        for (index, value) in [(7, 1.0), (8, hp), (9, drive), (10, vol)] {
            synth.set_parameter(index, value);
        }
        synth.note_on(48, 1.0, 0.5);
        let out = render(&mut synth, frames(0.3));
        let bound =
            (1.0 + hp) * 2f32.powf(4.0 * vol - 2.0) * (0.9819 * (-11.834 * drive).exp() + 0.022199);
        let peak = out.iter().fold(0.0_f32, |m, s| m.max(s.abs()));
        assert!(
            peak <= bound * lp_gain * 1.0001,
            "drive {drive}: {peak} > {bound}"
        );
        if drive == 1.0 {
            assert!(
                peak > bound * 0.5,
                "full drive saturates: {peak} vs {bound}"
            );
        }
    }
}

#[test]
fn resonance_peaks_at_the_resting_cutoff() {
    // Saw, Decay 0 and no accent: after 1.5 s MEG is ~1e-5, so the cutoff rests at 8.17579892·2^(128v/12),
    // v = 0.68·(1 − 0.31·0.01) − 0.02 − (36 − 12)/288 ≈ 572 Hz. Reso 1 over Reso 0 peaks just above it (the
    // ladder's resonance sits ~6 % above f_c; harmonics are 32.7 Hz apart).
    let spectrum = |reso: f32| {
        let mut synth = TransistorBass::new(RATE);
        synth.set_parameter(1, 1.0);
        synth.set_parameter(3, reso);
        synth.set_parameter(5, 0.0);
        synth.note_on(36, 0.5, 0.5);
        let out = render(&mut synth, frames(2.0));
        let periods = (15.0 * RATE / key_hz(36.0)).round() as usize;
        let steady = &out[frames(1.5)..frames(1.5) + periods];
        (2..40)
            .map(|h| amplitude(steady, h as f32 * key_hz(36.0)))
            .collect::<Vec<_>>()
    };
    let (resonant, flat) = (spectrum(1.0), spectrum(0.0));
    let h = (0..resonant.len())
        .max_by(|&a, &b| (resonant[a] / flat[a]).total_cmp(&(resonant[b] / flat[b])))
        .unwrap();
    let peak = (h + 2) as f32 * key_hz(36.0);
    let v = 0.68 * (1.0 - 0.31 * 0.01) - 0.02 - 24.0 / 288.0;
    let cutoff = 8.175_799 * 2f32.powf(128.0 * v / 12.0);
    assert!(
        (1.0..1.12).contains(&(peak / cutoff)),
        "resonance at {peak} Hz, cutoff {cutoff} Hz"
    );
}

#[test]
fn stays_finite_at_the_extremes() {
    for rate in [44_100.0, 48_000.0] {
        for value in [0.0, 1.0] {
            let mut synth = TransistorBass::new(rate);
            for index in 1..14 {
                synth.set_parameter(index, value);
            }
            synth.set_parameter(7, 1.0);
            for key in [0, 127, 64] {
                synth.note_on(key, 1.0, 0.5);
                let (left, _) = render_lr(&mut synth, (rate * 0.5) as usize);
                assert!(left.iter().all(|s| s.is_finite() && s.abs() < 10.0));
            }
        }
    }
}

#[test]
fn note_pan_is_a_linear_balance_moving_one_percent_per_block() {
    let mut synth = TransistorBass::new(RATE);
    synth.note_on(48, 0.5, 1.0);
    let (left, right) = render_lr(&mut synth, 256 * 101);
    // Block k (from 0) renders with the left gain 0.99^(k+1); the right stays 1.
    let sum = |side: &[f32], k: usize| {
        side[k * 256..(k + 1) * 256]
            .iter()
            .map(|s| s.abs())
            .sum::<f32>()
    };
    for k in [0, 10, 100] {
        let ratio = sum(&left, k) / sum(&right, k);
        let expected = 0.99_f32.powi(k as i32 + 1);
        assert!(
            (ratio - expected).abs() < 1e-4,
            "block {k}: {ratio} vs {expected}"
        );
    }
}

#[test]
fn reset_silences_at_once() {
    let mut synth = TransistorBass::new(RATE);
    synth.note_on(48, 1.0, 0.5);
    render(&mut synth, 4096);
    synth.reset();
    assert!(render(&mut synth, 1024).iter().all(|s| *s == 0.0));
    synth.note_on(48, 1.0, 0.5);
    synth.all_notes_off();
    render(&mut synth, frames(0.25));
    assert!(render(&mut synth, 1024).iter().all(|s| *s == 0.0));
}

#[test]
fn with_distortion_on_the_tail_renders_for_exactly_a_quarter_second() {
    // The distortion's 7.2 Hz highpass rings on after the 5 ms VCA fade, so every tail frame is non-zero.
    let mut synth = TransistorBass::new(RATE);
    synth.set_parameter(7, 1.0);
    synth.note_on(48, 1.0, 0.5);
    render(&mut synth, frames(0.2));
    synth.note_off(48);
    let tail = render(&mut synth, frames(0.25));
    assert!(tail[frames(0.2)..].iter().all(|s| *s != 0.0));
    assert!(render(&mut synth, 1024).iter().all(|s| *s == 0.0));
}

/// A golden-scenario step: note-on with a velocity, or note-off, then render for some seconds.
enum Step {
    On(u8, f32, f32),
    Off(u8, f32),
}

/// RMS and peak (f64) of a centred render of `steps` after setting `params`.
fn golden(params: &[(usize, f32)], steps: &[Step]) -> (f64, f64) {
    let mut synth = TransistorBass::new(RATE);
    for &(index, value) in params {
        synth.set_parameter(index, value);
    }
    let mut out = Vec::new();
    for step in steps {
        let seconds = match *step {
            Step::On(key, velocity, seconds) => {
                synth.note_on(key, velocity, 0.5);
                seconds
            }
            Step::Off(key, seconds) => {
                synth.note_off(key);
                seconds
            }
        };
        out.extend(render(&mut synth, frames(seconds)));
    }
    let energy = out.iter().map(|s| f64::from(*s).powi(2)).sum::<f64>();
    let peak = out.iter().fold(0.0_f64, |m, s| m.max(f64::from(s.abs())));
    ((energy / out.len() as f64).sqrt(), peak)
}

fn assert_golden(name: &str, (rms, peak): (f64, f64), golden: (f64, f64)) {
    assert!(
        (rms / golden.0 - 1.0).abs() < 1e-4 && (peak / golden.1 - 1.0).abs() < 1e-4,
        "{name}: ({rms:e}, {peak:e}) vs {golden:?}"
    );
}

#[test]
fn golden_plain_note_with_slide_and_slide_back() {
    // Defaults (Reso 1): HP160 feedback, g compensation, HP 205 Hz, ×0.25, the smoothed n in the cutoff.
    use Step::*;
    let steps = [
        On(36, 0.5, 0.25),
        On(48, 0.5, 0.25),
        Off(48, 0.25),
        Off(36, 0.3),
    ];
    assert_golden("plain", golden(&[], &steps), (2.129_957e-2, 1.597_266e-1));
}

#[test]
fn golden_accent_sweep_at_accent_1_reso_0() {
    // a = MEG·0.5, sweep = S + 3a; the second phrase slides to an unaccented key, so S falls.
    use Step::*;
    let params = [(6, 1.0), (3, 0.0), (1, 1.0), (4, 0.5)];
    let steps = [
        On(48, 1.0, 0.4),
        Off(48, 0.1),
        On(43, 1.0, 0.3),
        On(46, 0.5, 0.3),
        Off(46, 0.0),
        Off(43, 0.3),
    ];
    assert_golden("accent", golden(&params, &steps), (1.278_776e-1, 1.672_243));
}

#[test]
fn golden_distortion_at_mid_settings() {
    use Step::*;
    let params = [(7, 1.0), (8, 0.5), (9, 0.5), (10, 0.5), (1, 0.5)];
    let steps = [On(40, 1.0, 0.5), Off(40, 0.3)];
    assert_golden(
        "distortion",
        golden(&params, &steps),
        (1.969_889e-2, 4.469_147e-2),
    );
}

#[test]
fn golden_distortion_at_full_drive() {
    // Drive 1: the gain of 2306 drives the slew limiter rail to rail (±4.5), and the stage-3 LP sits at the 600 Hz
    // end of its table.
    use Step::*;
    let params = [(7, 1.0), (8, 0.5), (9, 1.0), (10, 0.5), (1, 0.5)];
    let steps = [On(40, 1.0, 0.5), Off(40, 0.3)];
    assert_golden(
        "full drive",
        golden(&params, &steps),
        (2.355_784e-2, 4.152_768e-2),
    );
}

#[test]
fn golden_pulse_width_lfo_at_full_speed_and_depth() {
    use Step::*;
    let params = [(13, 1.0), (12, 1.0), (11, 0.3)];
    let steps = [On(45, 0.5, 0.6)];
    assert_golden(
        "pw lfo",
        golden(&params, &steps),
        (2.691_225e-2, 1.629_212e-1),
    );
}

#[test]
fn golden_bright_resonance_near_the_cutoff_ceiling() {
    // Env Mod 1 at key 24 keeps f_c in the kHz range, where the K polynomial's high terms count.
    use Step::*;
    let params = [(4, 1.0), (1, 1.0)];
    let steps = [On(24, 0.5, 0.4)];
    assert_golden(
        "bright",
        golden(&params, &steps),
        (1.826_829e-2, 7.995_865e-2),
    );
}

#[test]
fn distortion_is_on_from_one_half() {
    // p7 at 0.5 distorts as 1 does; 0.001 is off as 0 is.
    let out = |v: f32| {
        let mut synth = TransistorBass::new(RATE);
        synth.set_parameter(7, v);
        synth.note_on(48, 1.0, 0.5);
        render(&mut synth, frames(0.2))
    };
    let (off, on) = (out(0.0), out(1.0));
    assert!(off != on);
    assert!(out(0.5) == on, "0.5 is on");
    assert!(out(0.001) == off, "0.001 is off");
}
