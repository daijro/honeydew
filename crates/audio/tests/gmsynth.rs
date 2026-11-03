//! GMSynth offline renders against its DSP laws.

use std::f64::consts::{PI, TAU};

use honeydew_audio::ProcessContext;
use honeydew_audio::gm_waveshapes::{SAW, SINE, SQUARE};
use honeydew_audio::gmsynth::{
    GmSynth, MOD_WHEEL, MW_AMOUNT, MW_DEST, PB_RANGE, PITCH_BEND, WAVE_A, WAVE_B, prepare_setting,
};
use honeydew_audio::instrument::{Instrument, InstrumentModule};

const RATE: f32 = 48_000.0;

fn context(rate: f32) -> ProcessContext {
    ProcessContext {
        sample_rate: rate,
        tempo_bpm: 120.0,
        start_beat: 0.0,
        playing: false,
    }
}

/// A GMSynth at `rate` with `settings` (built on this thread first, as the engine does) and `params`, reset so the
/// smoothers start at their targets.
fn synth(rate: f32, params: &[(usize, f32)], settings: &[(u32, f32)]) -> GmSynth {
    let mut synth = GmSynth::new(rate);
    for &(key, value) in settings {
        prepare_setting(key, value);
        synth.set_setting(key, value);
    }
    for &(index, value) in params {
        synth.set_parameter(index, value);
    }
    synth.reset();
    synth
}

/// Sine on osc A, the default elsewhere.
fn sine(rate: f32, params: &[(usize, f32)]) -> GmSynth {
    synth(rate, params, &[(WAVE_A, SINE as f32)])
}

fn render(synth: &mut GmSynth, frames: usize, rate: f32) -> (Vec<f32>, Vec<f32>) {
    let (mut left, mut right) = (vec![0.0; frames], vec![0.0; frames]);
    synth.process(&mut left, &mut right, context(rate));
    (left, right)
}

/// The frequency of a clean tone from its rising zero crossings, interpolated between samples.
fn frequency(signal: &[f32], rate: f32) -> f32 {
    let crossings: Vec<f64> = (signal.windows(2).enumerate())
        .filter(|(_, w)| w[0] < 0.0 && w[1] >= 0.0)
        .map(|(i, w)| i as f64 + f64::from(w[0] / (w[0] - w[1])))
        .collect();
    assert!(crossings.len() > 2, "no tone");
    let span = crossings[crossings.len() - 1] - crossings[0];
    ((crossings.len() - 1) as f64 * f64::from(rate) / span) as f32
}

/// The amplitude of the `freq` Hz component: Goertzel under a Hann window, so neighbouring partials barely leak.
fn amplitude(signal: &[f32], freq: f64, rate: f32) -> f64 {
    let n = signal.len() as f64;
    let coefficient = 2.0 * (TAU * freq / f64::from(rate)).cos();
    let (mut s1, mut s2, mut weight) = (0.0, 0.0, 0.0);
    for (i, &x) in signal.iter().enumerate() {
        let w = 0.5 - 0.5 * (TAU * i as f64 / n).cos();
        weight += w;
        let s0 = w * f64::from(x) + coefficient * s1 - s2;
        s2 = s1;
        s1 = s0;
    }
    let power = s1 * s1 + s2 * s2 - coefficient * s1 * s2;
    2.0 * power.max(0.0).sqrt() / weight
}

fn peak(signal: &[f32]) -> f32 {
    signal.iter().fold(0.0, |peak, x| peak.max(x.abs()))
}

fn rms(signal: &[f32]) -> f32 {
    (signal.iter().map(|x| x * x).sum::<f32>() / signal.len() as f32).sqrt()
}

/// f_A = 220·2^((key − 57)/12).
fn key_hz(key: f32) -> f32 {
    220.0 * 2.0_f32.powf((key - 57.0) / 12.0)
}

fn near(actual: f32, expected: f32, tolerance: f32) {
    assert!(
        (actual - expected).abs() <= tolerance,
        "{actual} != {expected} ± {tolerance}"
    );
}

#[test]
fn pitch_follows_key_transpose_bend_and_osc_b() {
    for rate in [44_100.0, RATE] {
        let mut a = sine(rate, &[]);
        a.note_on(69, 1.0, 0.5);
        near(frequency(&render(&mut a, 24_000, rate).0, rate), 440.0, 0.3);
    }
    // TRNSP trunc(48·v28 − 24) = +12.
    let mut up = sine(RATE, &[(28, 0.75)]);
    up.note_on(69, 1.0, 0.5);
    near(
        frequency(&render(&mut up, 24_000, RATE).0, RATE),
        880.0,
        0.5,
    );
    // Bend w = 2v − 1 = 1 times R: 2 st by default, 12 st with the `Oct` PB RANGE item.
    let mut bent = sine(RATE, &[]);
    bent.set_controller(PITCH_BEND, 1.0);
    bent.note_on(69, 1.0, 0.5);
    near(
        frequency(&render(&mut bent, 24_000, RATE).0, RATE),
        key_hz(71.0),
        0.5,
    );
    let mut octave = synth(RATE, &[], &[(WAVE_A, SINE as f32), (PB_RANGE, 3.0)]);
    octave.set_controller(PITCH_BEND, 0.0);
    octave.note_on(69, 1.0, 0.5);
    near(
        frequency(&render(&mut octave, 24_000, RATE).0, RATE),
        220.0,
        0.3,
    );
    // Osc B alone (BALANCE +100 %): PITCH B +12 st, FINE B +0.5 st.
    let b_only = |pitch: f32, fine: f32| {
        let mut b = synth(
            RATE,
            &[(13, 1.0), (11, pitch), (12, fine)],
            &[(WAVE_A, SAW as f32), (WAVE_B, SINE as f32)],
        );
        b.note_on(69, 1.0, 0.5);
        frequency(&render(&mut b, 24_000, RATE).0, RATE)
    };
    near(b_only(0.75, 0.5), 880.0, 0.5);
    near(b_only(0.75, 1.0), 880.0 * 2.0_f32.powf(0.5 / 12.0), 0.5);
    near(b_only(0.25, 0.0), 220.0 * 2.0_f32.powf(-0.5 / 12.0), 0.3);
    // Steps at v = f32(k/48): PITCH B plays trunc(f64(v)·48 − 24), so k = 1 plays −22 st (its text reads −23);
    // TRNSP takes trunc(fma_f32(v, 48, −24)), so k = 16 is −7 st.
    near(
        b_only((1.0_f64 / 48.0) as f32, 0.5),
        key_hz(69.0 - 22.0),
        0.2,
    );
    let mut down = sine(RATE, &[(28, (16.0_f64 / 48.0) as f32)]);
    down.note_on(69, 1.0, 0.5);
    near(
        frequency(&render(&mut down, 24_000, RATE).0, RATE),
        key_hz(62.0),
        0.3,
    );
}

#[test]
fn balance_endpoints_pick_a_b_and_minus_b() {
    let play = |balance: f32, a: usize, b: usize| {
        let mut s = synth(
            RATE,
            &[(13, balance)],
            &[(WAVE_A, a as f32), (WAVE_B, b as f32)],
        );
        s.note_on(69, 1.0, 0.5);
        render(&mut s, 4800, RATE).0
    };
    // 0 %: A only, at g = 0.5 (N = 1): the 0.5 sine is 0.25 into the filter's soft clip x/(1 + 0.05|x|).
    let a_only = play(0.5, SINE, SAW);
    assert_eq!(a_only, play(0.5, SINE, SQUARE), "B is not heard at 0 %");
    near(peak(&a_only), 0.25 / (1.0 + 0.05 * 0.25), 0.001);
    // +100 %: B only; −100 %: B inverted, sample for sample.
    let b_only = play(1.0, SINE, SAW);
    assert_eq!(b_only, play(1.0, SQUARE, SAW), "A is not heard at +100 %");
    let minus_b: Vec<f32> = play(0.0, SINE, SAW).iter().map(|x| -x).collect();
    assert_eq!(minus_b, b_only);
    // B plays at A's gain: a 0.5 sine at +100 % swings as far as A at 0 %, both ways.
    let b_sine = play(1.0, SAW, SINE);
    near(peak(&b_sine), peak(&a_only), 0.004);
    near(
        -b_sine.iter().fold(0.0_f32, |m, x| m.min(*x)),
        peak(&a_only),
        0.004,
    );
    // +50 %: A and B at g/2 each. PHASE B 0 locks B to A, so the two equal sines add up to the 0 % level.
    let half = play(0.75, SINE, SINE);
    near(peak(&half), peak(&a_only), 0.004);
}

#[test]
fn choir_spreads_copies_over_detune_scale_and_stereo() {
    // N = 4 copies, D = 1 st: ratios 2^((i/3 − 1/2)/12); odd copies ×1.5 (Chord) or ×2 (Octave).
    let copies = |scale: f64| -> Vec<f64> {
        (0..4)
            .map(|i| {
                let ratio = 2.0_f64.powf((f64::from(i) / 3.0 - 0.5) / 12.0);
                440.0 * ratio * if i % 2 == 1 { scale } else { 1.0 }
            })
            .collect()
    };
    let play = |scale: f32, stereo: f32| {
        let mut s = sine(RATE, &[(42, 1.0), (44, 1.0), (43, scale), (45, stereo)]);
        s.note_on(69, 1.0, 0.5);
        render(&mut s, 48_000, RATE)
    };
    // g = 0.5/(1 + 0.15·3) per copy, on a 0.5 sine.
    let each = 0.5 * 0.5 / 1.45;
    for (scale, ratio) in [(0.0, 1.0), (0.5, 1.5), (1.0, 2.0)] {
        let (left, _) = play(scale, 0.0);
        let expected = copies(ratio);
        for &f in &expected {
            let found = amplitude(&left, f, RATE);
            assert!((found - each).abs() < 0.06 * each, "{f} Hz: {found}");
        }
        if scale == 0.0 {
            for pair in expected.windows(2) {
                let between = amplitude(&left, 0.5 * (pair[0] + pair[1]), RATE);
                assert!(between < 0.2 * each, "{between} between copies");
            }
        }
    }
    // CHOIR STR 1: x_i = 2i/3 − 1, so copy 0 is left only and copy 3 right only; copy 1 is 1 / (1 + x) = 2/3.
    let (left, right) = play(0.0, 1.0);
    let f = copies(1.0);
    assert!(amplitude(&right, f[0], RATE) < 0.02 * each);
    assert!(amplitude(&left, f[3], RATE) < 0.02 * each);
    near(
        (amplitude(&right, f[1], RATE) / amplitude(&left, f[1], RATE)) as f32,
        2.0 / 3.0,
        0.03,
    );
}

#[test]
fn filter_peaks_at_the_tables_resonance_frequency() {
    // RES 1: the 8-stage loop rings at f_r = 1.0159·f + 0.5 for the nominal f = 32·2^(9.27c), authored at 44.1 kHz
    // and scaling with sr/44100. The oscillators cancel (BALANCE +50 % with B = A shifted half a cycle by PHASE B), so
    // only a faint noise (NOISE 0.003) excites the loop and the soft clip stays linear.
    for (rate, nominal, kind) in [
        (44_100.0, 1000.0, 0.0),
        (44_100.0, 400.0, 0.5),
        (RATE, 2500.0, 0.0),
        (44_100.0, 1000.0, 1.0),
    ] {
        let c = (nominal / 32.0_f32).log2() / 9.27;
        let params = [
            (13, 0.75),
            (14, 0.5),
            (20, 0.003),
            (21, c),
            (22, 1.0),
            (23, kind),
        ];
        let mut s = synth(
            rate,
            &params,
            &[(WAVE_A, SINE as f32), (WAVE_B, SINE as f32)],
        );
        s.note_on(64, 1.0, 0.5);
        let (left, _) = render(&mut s, (1.5 * rate) as usize, rate);
        let tail = &left[(rate * 0.5) as usize..];
        let resonance = (1.0159 * f64::from(nominal) + 0.5) * f64::from(rate) / 44_100.0;
        let found = (0..=400)
            .map(|step| resonance * (0.9 + 0.0005 * f64::from(step)))
            .max_by(|a, b| amplitude(tail, *a, rate).total_cmp(&amplitude(tail, *b, rate)))
            .unwrap();
        assert!(
            (found - resonance).abs() < 0.001 * resonance,
            "{nominal} Hz at {rate}: peak {found}, expected {resonance}"
        );
    }
}

/// The amp envelope as the peak of |x| over one period around `at` seconds, relative to `full`.
fn envelope(signal: &[f32], at: f32, full: f32) -> f32 {
    let centre = (at * RATE) as usize;
    let half = (RATE / 880.0 / 2.0).ceil() as usize + 1;
    peak(&signal[centre - half..centre + half]) / full
}

#[test]
fn envelopes_follow_the_one_pole_laws() {
    // Key 81 = 880 Hz. Full level: attack 0 (one sample), sustain 1.
    let full = {
        let mut s = sine(RATE, &[]);
        s.note_on(81, 1.0, 0.5);
        peak(&render(&mut s, 4800, RATE).0[2400..])
    };
    // Attack v = 0.5: k = 8.82/(sr·100v⁴) toward 5.0; 0 → 1 in ≈ 2.53·v⁴ s, at T/4 5(1 − 1.25^−¼), at T/2 5(1 − 1.25^−½).
    let mut attack = sine(RATE, &[(3, 0.5)]);
    attack.note_on(81, 1.0, 0.5);
    let out = render(&mut attack, 24_000, RATE).0;
    let t = (1.25_f32.ln() / (8.82 / (RATE * 100.0 * 0.0625))) / RATE;
    near(t, 2.53 * 0.0625, 0.001);
    near(
        envelope(&out, t / 4.0, full),
        5.0 * (1.0 - 1.25_f32.powf(-0.25)),
        0.02,
    );
    near(
        envelope(&out, t / 2.0, full),
        5.0 * (1.0 - 1.25_f32.powf(-0.5)),
        0.02,
    );
    near(envelope(&out, t * 1.2, full), 1.0, 0.02);
    // Decay v = 0.5 to sustain 0: τ = (200v⁷ + 0.001)/8.82 s = 22.7·v⁷ s + 0.11 ms.
    let mut decay = sine(RATE, &[(4, 0.5), (5, 0.0)]);
    decay.note_on(81, 1.0, 0.5);
    let out = render(&mut decay, 24_000, RATE).0;
    let tau = (200.0 * 0.5_f32.powi(7) + 0.001) / 8.82;
    near(envelope(&out, tau, full), (-1.0_f32).exp(), 0.02);
    near(envelope(&out, 2.0 * tau, full), (-2.0_f32).exp(), 0.02);
    // Sustain v5² with a fast decay.
    let mut sustain = sine(RATE, &[(4, 0.0), (5, 0.5)]);
    sustain.note_on(81, 1.0, 0.5);
    let out = render(&mut sustain, 4800, RATE).0;
    near(envelope(&out, 0.05, full), 0.25, 0.01);
    // Release v = 0.5: τ = (v + 0.001)/8.82 s after the note-off; the voice frees below 10⁻⁴.
    let mut release = sine(RATE, &[(6, 0.5)]);
    release.note_on(81, 1.0, 0.5);
    render(&mut release, 4800, RATE);
    release.note_off(81);
    let out = render(&mut release, 48_000, RATE).0;
    let tau = 0.501 / 8.82;
    near(envelope(&out, tau, full), (-1.0_f32).exp(), 0.02);
    near(envelope(&out, 2.0 * tau, full), (-2.0_f32).exp(), 0.02);
    // Below 10⁻⁴ after ln(10⁴)·τ = 0.52 s.
    assert_eq!(peak(&out[30_000..]), 0.0, "the voice was freed");
}

#[test]
fn mono_falls_back_to_held_keys() {
    let segment = |s: &mut GmSynth| {
        let out = render(s, 12_000, RATE).0;
        frequency(&out[1200..], RATE)
    };
    let mut mono = sine(RATE, &[(30, 1.0)]);
    for key in [60, 64, 67] {
        mono.note_on(key, 1.0, 0.5);
    }
    near(segment(&mut mono), key_hz(67.0), 0.5);
    // The sounding key's note-off falls back to the highest held key below it.
    mono.note_off(67);
    near(segment(&mut mono), key_hz(64.0), 0.5);
    mono.note_on(72, 1.0, 0.5);
    near(segment(&mut mono), key_hz(72.0), 0.5);
    mono.note_off(72);
    near(segment(&mut mono), key_hz(64.0), 0.5);
    // A held key that is not sounding only leaves the key table.
    mono.note_off(60);
    near(segment(&mut mono), key_hz(64.0), 0.5);
    // None held: release (0.2 → τ 23 ms).
    mono.note_off(64);
    let out = render(&mut mono, 24_000, RATE).0;
    assert_eq!(peak(&out[12_000..]), 0.0);

    // No key held below: the lowest held key above.
    let mut mono = sine(RATE, &[(30, 1.0)]);
    for key in [72, 76, 60] {
        mono.note_on(key, 1.0, 0.5);
    }
    near(segment(&mut mono), key_hz(60.0), 0.5);
    mono.note_off(60);
    near(segment(&mut mono), key_hz(72.0), 0.5);
}

/// The master pan P per sample, from the right/left ratio of a centred mono voice: P < 0.5 gives R/L = (2P)²,
/// P > 0.5 gives L/R = (2 − 2P)². Samples near a zero crossing are `None`.
fn pans(left: &[f32], right: &[f32]) -> Vec<Option<f32>> {
    (left.iter().zip(right))
        .map(|(&l, &r)| {
            (l.abs().max(r.abs()) > 0.05).then(|| {
                if r.abs() < l.abs() {
                    (r / l).abs().sqrt() / 2.0
                } else {
                    1.0 - (l / r).abs().sqrt() / 2.0
                }
            })
        })
        .collect()
}

#[test]
fn lfo_locks_to_the_song_while_playing_and_runs_free_when_stopped() {
    // LFO → PAN, Sine, amount v = 0.75 (A = 0.5, depth |A|A = 0.25), rate 1 beat at 120 BPM.
    let params = [(35, 1.0), (33, 0.0), (34, 0.75), (32, 4.0 / 9.0)];
    let beats_per_frame = 120.0 / (60.0 * f64::from(RATE));
    let expected = |beat: f64| 0.5 + 0.25 * (TAU * beat.fract()).sin() as f32;
    let play = |s: &mut GmSynth, from_beat: f64, frames: usize| {
        let (mut left, mut right) = (vec![0.0; frames], vec![0.0; frames]);
        for (n, (l, r)) in left.chunks_mut(500).zip(right.chunks_mut(500)).enumerate() {
            let context = ProcessContext {
                start_beat: from_beat + (n * 500) as f64 * beats_per_frame,
                playing: true,
                ..context(RATE)
            };
            s.process(l, r, context);
        }
        pans(&left, &right)
    };
    let check = |pans: &[Option<f32>], from_beat: f64, settle: usize| {
        let mut checked = 0;
        for (i, pan) in pans.iter().enumerate().skip(settle) {
            if let Some(pan) = pan {
                near(
                    *pan,
                    expected(from_beat + i as f64 * beats_per_frame),
                    0.015,
                );
                checked += 1;
            }
        }
        assert!(checked > pans.len() / 2);
    };
    let mut from_start = sine(RATE, &params);
    from_start.note_on(69, 1.0, 0.5);
    check(&play(&mut from_start, 0.0, 72_000), 0.0, 0);

    // Another history: it runs free while stopped (the pan moves with the song position held), then follows the
    // song from beat 1.37.
    let mut late = sine(RATE, &params);
    late.note_on(69, 1.0, 0.5);
    let (left, right) = render(&mut late, 12_000, RATE);
    let stopped: Vec<f32> = pans(&left, &right).into_iter().flatten().collect();
    let swing = stopped.iter().fold(0.0_f32, |m, p| m.max(*p))
        - stopped.iter().fold(1.0_f32, |m, p| m.min(*p));
    assert!(swing > 0.2, "free-running LFO swing {swing}");
    check(&play(&mut late, 1.37, 48_000), 1.37, 600);
}

#[test]
fn one_block_or_many_give_the_same_output() {
    let params = [
        (42, 0.67),
        (44, 0.4),
        (45, 0.8),
        (16, 1.0 / 3.0),
        (15, 0.4),
        (19, 0.8),
        (20, 0.3),
        (21, 0.5),
        (22, 0.6),
        (25, 0.2),
        (27, 0.9),
        (29, 0.3),
        (34, 0.9),
        (36, 0.3),
        (40, 0.2),
        (41, 1.0),
        // FX: Phaser (Saw LFO), Delay, Ring (Square LFO).
        (46, 2.0 / 9.0),
        (47, 1.0),
        (49, 0.5),
        (50, 0.9),
        (53, 5.0 / 9.0),
        (54, 1.0),
        (57, 0.2),
        (60, 8.0 / 9.0),
        (61, 1.0),
        (63, 1.0),
        (64, 1.0),
    ];
    // (frame, key, on): six notes on four voices (two steals), then a note-off.
    let events = [
        (0, 60, true),
        (0, 64, true),
        (777, 67, true),
        (5000, 71, true),
        (5000, 74, true),
        (9001, 48, true),
        (12_000, 64, false),
    ];
    let play = |blocks: &[usize]| {
        let mut s = synth(RATE, &params, &[(WAVE_A, 3.0), (WAVE_B, SAW as f32)]);
        let (mut left, mut right) = (vec![0.0; 20_000], vec![0.0; 20_000]);
        let mut at = 0;
        for &size in blocks.iter().cycle() {
            if at == 20_000 {
                break;
            }
            for &(_, key, on) in events.iter().filter(|event| event.0 == at) {
                if on {
                    s.note_on(key, 0.8, 0.3);
                } else {
                    s.note_off(key);
                }
            }
            let next = events.iter().map(|event| event.0).find(|&frame| frame > at);
            let end = (at + size).min(20_000).min(next.unwrap_or(20_000));
            s.process(&mut left[at..end], &mut right[at..end], context(RATE));
            at = end;
        }
        (left, right)
    };
    let whole = play(&[20_000]);
    assert!(peak(&whole.0) > 0.01);
    assert_eq!(whole, play(&[1, 7, 31, 32, 33, 64, 333, 1023, 4096]));
}

#[test]
fn osc_mod_kinds_make_their_sidebands() {
    // A sine 440 Hz, B sine 880 Hz (PITCH B +12), BALANCE 0 % (A only), MOD. AMNT 1: m = 1.
    let play = |kind: f32, amount: f32| {
        let mut s = synth(
            RATE,
            &[(11, 0.75), (16, kind), (15, amount)],
            &[(WAVE_A, SINE as f32), (WAVE_B, SINE as f32)],
        );
        s.note_on(69, 1.0, 0.5);
        render(&mut s, 48_000, RATE).0
    };
    let plain = play(0.0, 0.0);
    assert!(amplitude(&plain, 1320.0, RATE) < 0.002);
    // Ring: A·(1 + 3m·B) puts 880 ± 440 beside the carrier.
    let ring = play(0.0, 1.0);
    assert!(amplitude(&ring, 1320.0, RATE) > 0.05);
    // FM: A read at φ_A + (B + 1)·m cycles: sidebands at 440 ± k·880.
    let fm = play(1.0 / 3.0, 1.0);
    assert!(amplitude(&fm, 1320.0, RATE) > 0.02);
    // FM at m = 0.5 with B = A in phase (PITCH B 0) and BALANCE +50 %: A is read at θ + 2π·0.5·(B + 1), half a cycle
    // on, so out = g/2·(0.5·sin(θ + π(0.5·sin θ + 1)) + 0.5·sin θ), through the soft clip.
    let mut half = synth(
        RATE,
        &[(13, 0.75), (16, 1.0 / 3.0), (15, 0.5)],
        &[(WAVE_A, SINE as f32), (WAVE_B, SINE as f32)],
    );
    half.note_on(69, 1.0, 0.5);
    let expected = (0..4096)
        .map(|i| {
            let theta = TAU * f64::from(i) / 4096.0;
            let x =
                0.25 * (0.5 * (theta + PI * (0.5 * theta.sin() + 1.0)).sin() + 0.5 * theta.sin());
            (x / (1.0 + 0.05 * x.abs())).abs() as f32
        })
        .fold(0.0, f32::max);
    near(peak(&render(&mut half, 4800, RATE).0), expected, 0.01);
    // Sync with B an octave and a quarter tone down (PITCH B −12, FINE B +0.5): A runs at f_A·2^m′ = 880 Hz and
    // restarts at B's 226.4 Hz: its harmonics appear, 3 × 226.4 Hz among them.
    let b_hz = 220.0 * 2.0_f64.powf(0.5 / 12.0);
    let mut sync = synth(
        RATE,
        &[(11, 0.25), (12, 1.0), (16, 2.0 / 3.0), (15, 1.0)],
        &[(WAVE_A, SINE as f32), (WAVE_B, SINE as f32)],
    );
    sync.note_on(69, 1.0, 0.5);
    let sync = render(&mut sync, 48_000, RATE).0;
    assert!(amplitude(&sync, 3.0 * b_hz, RATE) > 0.01);
    assert!(amplitude(&plain, 3.0 * b_hz, RATE) < 0.002);
    // Filter: cutoff += 0.5·m·B moves the filter, so the output changes.
    let filter = |amount: f32| {
        let mut s = synth(
            RATE,
            &[(11, 0.75), (16, 1.0), (15, amount), (21, 0.4), (22, 0.5)],
            &[(WAVE_A, SAW as f32), (WAVE_B, SINE as f32)],
        );
        s.note_on(69, 1.0, 0.5);
        rms(&render(&mut s, 4800, RATE).0)
    };
    assert!((filter(1.0) - filter(0.0)).abs() > 0.1 * filter(0.0));
}

#[test]
fn mod_wheel_reaches_its_destination() {
    // MW AMOUNT 1, wheel full or off, by MW DEST item (0 Filter, 1 LFO, 2 Osc. Mod.).
    let play = |params: &[(usize, f32)], wheel: f32, dest: f32| {
        let settings = [
            (WAVE_A, SINE as f32),
            (WAVE_B, SINE as f32),
            (MW_DEST, dest),
            (MW_AMOUNT, 1.0),
        ];
        let mut s = synth(RATE, params, &settings);
        s.set_controller(MOD_WHEEL, wheel);
        s.reset();
        s.note_on(69, 1.0, 0.5);
        render(&mut s, 24_000, RATE).0
    };
    // Filter: the cutoff base, closed at 32 Hz, opens.
    let closed = [(21, 0.0)];
    assert!(rms(&play(&closed, 1.0, 0.0)[4800..]) > 20.0 * rms(&play(&closed, 0.0, 0.0)[4800..]));
    // LFO: |A| grows from 0 to 1. A Square LFO (−1 for its first 16 beats) on FREQ: −12 st.
    let lfo = [(33, 1.0), (35, 0.5), (32, 1.0)];
    near(frequency(&play(&lfo, 1.0, 1.0), RATE), 220.0, 0.3);
    near(frequency(&play(&lfo, 0.0, 1.0), RATE), 440.0, 0.3);
    // Osc. Mod.: the mod amount, here FM with B an octave up.
    let fm = [(11, 0.75), (16, 1.0 / 3.0)];
    assert!(amplitude(&play(&fm, 1.0, 2.0), 1320.0, RATE) > 0.02);
    assert!(amplitude(&play(&fm, 0.0, 2.0), 1320.0, RATE) < 0.002);
}

#[test]
fn master_level_and_pan_laws() {
    let play = |level: f32, pan: f32| {
        let mut s = sine(RATE, &[(1, level), (2, pan)]);
        s.note_on(69, 1.0, 0.5);
        let (l, r) = render(&mut s, 4800, RATE);
        (peak(&l[2400..]), peak(&r[2400..]))
    };
    let (unity, _) = play(48.0 / 66.0, 0.5);
    // dB = 66·v1 − 48.
    near(play(0.0, 0.5).0 / unity, 10.0_f32.powf(-48.0 / 20.0), 1e-4);
    near(play(1.0, 0.5).0 / unity, 10.0_f32.powf(18.0 / 20.0), 0.05);
    // L = min(1, 2 − 2P)², R = min(1, 2P)².
    let (l, r) = play(48.0 / 66.0, 0.25);
    near(l / unity, 1.0, 0.01);
    near(r / unity, 0.25, 0.01);
    let (l, r) = play(48.0 / 66.0, 0.75);
    near(l / unity, 0.25, 0.01);
    near(r / unity, 1.0, 0.01);
    let (l, r) = play(48.0 / 66.0, 1.0);
    near(l / unity, 0.0, 1e-6);
    near(r / unity, 1.0, 0.01);
}

#[test]
fn enable_off_ignores_note_ons_and_hidden_params_do_nothing() {
    // Enable is on at v ≥ 0.5: a note-on at 0.001 is ignored, one at 0.5 plays.
    for enable in [0.0, 0.001] {
        let mut off = sine(RATE, &[(0, enable)]);
        off.note_on(69, 1.0, 0.5);
        off.set_parameter(0, 1.0);
        assert_eq!(peak(&render(&mut off, 4800, RATE).0), 0.0);
    }
    let mut on = sine(RATE, &[(0, 0.5)]);
    on.note_on(69, 1.0, 0.5);
    assert!(peak(&render(&mut on, 4800, RATE).0) > 0.1);
    // Turned off while sounding: the rack module resets GMSynth at once, so nothing is left to come back.
    let mut module = InstrumentModule::new(Box::new(sine(RATE, &[])), 1024);
    module.instrument.note_on(69, 1.0, 0.5);
    let play = |module: &mut InstrumentModule| {
        let (mut left, mut right) = (vec![0.0; 4800], vec![0.0; 4800]);
        module.process(&mut left, &mut right, context(RATE));
        peak(&left)
    };
    assert!(play(&mut module) > 0.1);
    module.set_parameter(0, 0.0);
    assert_eq!(play(&mut module), 0.0);
    module.set_parameter(0, 1.0);
    assert_eq!(play(&mut module), 0.0, "the held voice is gone");

    let play = |hidden: f32| {
        let params: Vec<(usize, f32)> = (7..=10).chain(67..=73).map(|i| (i, hidden)).collect();
        let mut s = sine(RATE, &params);
        s.note_on(69, 1.0, 0.5);
        render(&mut s, 4800, RATE)
    };
    assert_eq!(play(0.0), play(1.0));
}

#[test]
fn a_missing_shape_silences_the_voice() {
    prepare_setting(WAVE_B, SAW as f32);
    let mut missing = synth(RATE, &[], &[(WAVE_B, 29.0)]);
    missing.note_on(69, 1.0, 0.5);
    assert_eq!(peak(&render(&mut missing, 4800, RATE).0), 0.0);
    // Setting a built shape back makes it sound.
    missing.set_setting(WAVE_B, SAW as f32);
    missing.note_on(69, 1.0, 0.5);
    assert!(peak(&render(&mut missing, 4800, RATE).0) > 0.05);
}

/// 0 dB master level (66·v1 − 48).
const UNITY: (usize, f32) = (1, 48.0 / 66.0);

/// FX slot `slot` (0–2, params 46 + 7·slot …) on, kind `kind` (0–9: DIST … VOX) at `x`, `y`; Sync 1/4, no LFO.
fn fx(slot: usize, kind: usize, x: f32, y: f32) -> [(usize, f32); 4] {
    let base = 46 + 7 * slot;
    [
        (base, kind as f32 / 9.0),
        (base + 1, 1.0),
        (base + 5, x),
        (base + 6, y),
    ]
}

#[test]
fn every_fx_kind_changes_the_output_and_its_tail_ends_once_disabled() {
    // A 440 Hz sine for 0.2 s, released (the voice is free by 0.45 s); at 0.5 s the slot is disabled.
    let run = |params: &[(usize, f32)], enable: Option<usize>| {
        let mut s = sine(RATE, params);
        s.note_on(69, 1.0, 0.5);
        let (mut left, mut right) = render(&mut s, 9_600, RATE);
        s.note_off(69);
        let mut more = |s: &mut GmSynth, frames| {
            let (l, r) = render(s, frames, RATE);
            left.extend(l);
            right.extend(r);
        };
        more(&mut s, 14_400);
        if let Some(enable) = enable {
            s.set_parameter(enable, 0.0);
        }
        more(&mut s, 144_000);
        (left, right)
    };
    let (dry_l, dry_r) = run(&[UNITY], None);
    assert!(
        dry_l[22_000..].iter().all(|&x| x == 0.0),
        "the voice rings on"
    );
    for kind in 0..10 {
        let slot = kind % 3;
        let mut params = fx(slot, kind, 0.4, 0.6).to_vec();
        params.push(UNITY);
        let (l, r) = run(&params, Some(47 + 7 * slot));
        let change = (dry_l.iter().zip(&l))
            .chain(dry_r.iter().zip(&r))
            .map(|(d, w)| (d - w).abs())
            .fold(0.0, f32::max);
        assert!(change > 1e-3, "kind {kind} changes by {change}");
        // Disabled over a silent bus: Delay and Reverb ring on; every kind has cleared within 2.5 s.
        let after = peak(&l[24_000..28_800]).max(peak(&r[24_000..28_800]));
        if kind == 5 || kind == 6 {
            assert!(
                after > 1e-4,
                "kind {kind}: no tail after disabling ({after})"
            );
        }
        let end = 24_000 + 120_000;
        assert!(
            l[end..].iter().chain(&r[end..]).all(|&x| x == 0.0),
            "kind {kind}: still sounding 2.5 s after it was disabled"
        );
    }
}

#[test]
fn fx_1_2_3_run_in_series_before_the_master() {
    // DIST at X = 1, Y = 0 keeps every sample below 0.1·(1 + 10⁻⁵). A Delay at X = 0.9 (0.81 a repeat, in phase with
    // the 440 Hz tone) builds well past that. DIST after the Delay holds the bound; DIST before it does not.
    let play = |dist: usize, delay: usize, level: f32| {
        let mut params = vec![(1, level)];
        params.extend(fx(dist, 0, 1.0, 0.0));
        params.extend(fx(delay, 5, 0.9, 0.5));
        let mut s = sine(RATE, &params);
        s.note_on(69, 1.0, 0.5);
        let (l, r) = render(&mut s, 48_000, RATE);
        peak(&l).max(peak(&r))
    };
    let bound = 0.1 * (1.0 + 1e-5) + 1e-6;
    for (first, second) in [(0, 1), (1, 2), (0, 2)] {
        let after = play(second, first, UNITY.1);
        assert!(
            after <= bound,
            "DIST in FX {} after the Delay in FX {}: {after}",
            second + 1,
            first + 1
        );
        let before = play(first, second, UNITY.1);
        assert!(
            before > 0.15,
            "DIST in FX {} before the Delay in FX {}: {before}",
            first + 1,
            second + 1
        );
    }
    // The master level (+18 dB) applies after the chain.
    near(play(2, 0, 1.0), 0.1 * 10.0_f32.powf(18.0 / 20.0), 0.01);
}

#[test]
fn fx_lfo_locks_to_the_song_while_playing_and_runs_free_when_stopped() {
    // DIST at X = 0.2, Y = 0 is memoryless (H = 1, g = 0.1), and the LFO moves its k = 10^(4(X + LFO) + 1) per
    // 32-frame block, so each output sample follows from the dry one. 0.2 s stopped (the song position is ignored;
    // the phase runs free from 0), then the song plays from beat 1.37.
    let beats_per_frame = 120.0 / (60.0 * f64::from(RATE));
    let play = |params: &[(usize, f32)]| {
        let mut s = sine(RATE, params);
        s.note_on(69, 1.0, 0.5);
        let (mut left, _) = {
            let stopped = ProcessContext {
                start_beat: 5.3,
                ..context(RATE)
            };
            let (mut l, mut r) = (vec![0.0; 9_600], vec![0.0; 9_600]);
            s.process(&mut l, &mut r, stopped);
            (l, r)
        };
        for n in 0..96 {
            let (mut l, mut r) = (vec![0.0; 500], vec![0.0; 500]);
            let playing = ProcessContext {
                start_beat: 1.37 + (n * 500) as f64 * beats_per_frame,
                playing: true,
                ..context(RATE)
            };
            s.process(&mut l, &mut r, playing);
            left.extend(l);
        }
        left
    };
    let dry = play(&[UNITY]);
    // (slot, Sync v, shape v, amount v; R, shape, A): R = [1/4 … 32][trunc(9v)], shape trunc(2v),
    // A = sign(2v − 1)·(2v − 1)².
    let cases = [
        (0, 0.0, 0.0, 0.8, 0.25, 0, 0.36),
        (1, 3.0 / 9.0, 0.5, 0.1, 0.75, 1, -0.64),
        (2, 4.0 / 9.0, 1.0, 1.0, 1.0, 2, 1.0),
        (1, 1.0, 0.0, 0.3, 32.0, 0, -0.16),
    ];
    for (slot, sync, shape_v, amount_v, rate, shape, a) in cases {
        let base = 46 + 7 * slot;
        let mut params = fx(slot, 0, 0.2, 0.0).to_vec();
        params.extend([
            UNITY,
            (base + 2, sync),
            (base + 3, shape_v),
            (base + 4, amount_v),
        ]);
        let wet = play(&params);
        let step = 32.0 * beats_per_frame / rate;
        for (i, (&x, &y)) in dry.iter().zip(&wet).enumerate() {
            let block = i / 32;
            let phase = if i < 9_600 {
                (block as f64 * step).fract()
            } else {
                ((1.37 + (32 * block - 9_600) as f64 * beats_per_frame) / rate).rem_euclid(1.0)
            };
            let lfo = a * match shape {
                0 => (TAU * phase).sin(),
                1 => 1.0 - 2.0 * (phase + step).fract(),
                _ => {
                    if phase <= 0.5 {
                        1.0
                    } else {
                        -1.0
                    }
                }
            };
            let k = 10.0_f64.powf(4.0 * (0.2 + lfo) + 1.0);
            let expected = f64::from(x) * 0.1 * (k + 1.0) / (1.0 + k * f64::from(x).abs());
            assert!(
                (f64::from(y) - expected).abs() < 2e-5,
                "slot {slot} rate {rate} frame {i}: {y} != {expected}"
            );
        }
    }

    // The Delay runs its LFO on 2R: a song started R later is half a cycle off, one started 2R later is not.
    let delay = |from_beat: f64| {
        let mut params = fx(1, 5, 0.6, 0.5).to_vec();
        params.extend([UNITY, (56, 1.0), (57, 1.0)]);
        let mut s = sine(RATE, &params);
        s.note_on(69, 1.0, 0.5);
        let (mut l, mut r) = (vec![0.0; 24_000], vec![0.0; 24_000]);
        let playing = ProcessContext {
            start_beat: from_beat,
            playing: true,
            ..context(RATE)
        };
        s.process(&mut l, &mut r, playing);
        l
    };
    let difference = |a: &[f32], b: &[f32]| {
        a.iter()
            .zip(b)
            .map(|(a, b)| (a - b).abs())
            .fold(0.0, f32::max)
    };
    assert!(difference(&delay(0.3), &delay(0.8)) < 1e-5);
    assert!(difference(&delay(0.3), &delay(0.55)) > 0.01);
}

#[test]
fn reset_clears_the_fx() {
    // Every kind holding state from an earlier note; after reset the synth plays as a new one. (The Reverb sits at
    // X = 0.5, where a new bank's late level already is; DIST at Y = 1 holds each latch for 257 frames.)
    for slots in [
        [fx(0, 5, 0.9, 0.5), fx(1, 6, 0.5, 0.3), fx(2, 1, 0.5, 0.9)],
        [fx(0, 0, 0.6, 1.0), fx(1, 8, 0.5, 0.5), fx(2, 2, 0.5, 0.7)],
        [fx(0, 3, 0.3, 0.8), fx(1, 4, 0.6, 0.8), fx(2, 9, 0.5, 0.5)],
        [fx(0, 7, 0.2, 0.9), fx(1, 9, 0.8, 0.2), fx(2, 0, 0.5, 0.2)],
    ] {
        let mut params = slots.concat();
        params.push(UNITY);
        let mut fresh = sine(RATE, &params);
        fresh.note_on(69, 1.0, 0.5);
        let mut used = sine(RATE, &params);
        used.note_on(60, 1.0, 0.5);
        render(&mut used, 30_000, RATE);
        used.reset();
        used.note_on(69, 1.0, 0.5);
        assert_eq!(
            render(&mut used, 12_000, RATE),
            render(&mut fresh, 12_000, RATE),
            "{slots:?}"
        );
    }
}

/// The normalized cutoff c the filter sits at, read from its ringing: at RES 1 the loop rings at f_r = 1.0159·f + 0.5
/// for f = 32·2^(9.27c) (44.1 kHz). The oscillators cancel (BALANCE +50 %, B = A half a cycle on) and a faint NOISE
/// excites the loop, measured over its second half second.
fn ringing_cutoff(params: &[(usize, f32)], key: u8, velocity: f32) -> f32 {
    let rate = 44_100.0;
    let mut all = vec![(13, 0.75), (14, 0.5), (20, 0.003), (22, 1.0), (23, 0.5)];
    all.extend_from_slice(params);
    let mut s = synth(rate, &all, &[(WAVE_A, SINE as f32), (WAVE_B, SINE as f32)]);
    s.note_on(key, velocity, 0.5);
    let out = render(&mut s, 44_100, rate).0;
    let f_r = frequency(&out[22_050..], rate);
    ((f_r - 0.5) / 1.0159 / 32.0).log2() / 9.27
}

#[test]
fn the_cutoff_follows_kbd_contour_eg_lfo_and_vel() {
    let base = 0.4;
    near(ringing_cutoff(&[(21, base)], 64, 1.0), base, 0.002);
    // KBD: + (pitch − 64)·0.0089964 per key.
    let kbd = [(21, base), (24, 1.0)];
    near(ringing_cutoff(&kbd, 64, 1.0), base, 0.002);
    near(
        ringing_cutoff(&kbd, 76, 1.0),
        base + 12.0 * 0.008_996_4,
        0.002,
    );
    // Filter contour (2v27 − 1)³·E_flt: v27 0.75 is 0.125, with E_flt decaying at τ = 22.7 s (0.968 mid-window).
    let contour = [(21, base), (27, 0.75)];
    near(
        ringing_cutoff(&contour, 64, 1.0),
        base + 0.125 * (-0.75_f32 / 22.676).exp(),
        0.003,
    );
    // EG → Filter c³ at EG sustain v38²: c = 0.5 gives 0.125; c = 1 at sustain 0.5 gives 0.25.
    near(
        ringing_cutoff(&[(21, base), (40, 0.75)], 64, 1.0),
        base + 0.125,
        0.002,
    );
    let sustain = [(21, base), (40, 1.0), (37, 0.0), (38, 0.5)];
    near(ringing_cutoff(&sustain, 64, 1.0), base + 0.25, 0.002);
    // VEL, any nonzero v74: the Filter depth × velocity.
    // On at any nonzero v, 0.5 and 0.001 included.
    for vel in [0.001, 0.5] {
        let on = [(21, base), (40, 0.75), (74, vel)];
        near(ringing_cutoff(&on, 64, 0.5), base + 0.0625, 0.002);
    }
    near(
        ringing_cutoff(&[(21, base), (40, 0.75)], 64, 0.5),
        base + 0.125,
        0.002,
    );
    // LFO → FLT A³: a Square LFO sits at −1 for the first half of its 32 beats; A = 0.5.
    let lfo = [(21, base), (33, 1.0), (32, 1.0), (34, 0.75)];
    near(ringing_cutoff(&lfo, 64, 1.0), base - 0.125, 0.002);
}

#[test]
fn eg_to_pitch_depth_sustain_and_release() {
    let pitch = |params: &[(usize, f32)]| {
        let mut all = vec![(41, 1.0)];
        all.extend_from_slice(params);
        let mut s = sine(RATE, &all);
        s.note_on(69, 1.0, 0.5);
        frequency(&render(&mut s, 24_000, RATE).0[2400..], RATE)
    };
    // 12·c·|c| st: c = 0.5 → +3 st; c = 1 at sustain v38² = 0.25 → +3 st.
    near(pitch(&[(40, 0.75)]), key_hz(72.0), 0.5);
    near(pitch(&[(40, 1.0), (37, 0.0), (38, 0.5)]), key_hz(72.0), 0.5);
    // Release v39 = 0.5: τ = (200v⁴ + 0.001)/8.82 = 1.417 s; 35 ms into it the EG is at 0.976 (+11.71 st).
    let mut s = sine(RATE, &[(41, 1.0), (40, 1.0), (39, 0.5), (6, 1.0)]);
    s.note_on(69, 1.0, 0.5);
    render(&mut s, 4800, RATE);
    s.note_off(69);
    let out = render(&mut s, 3360, RATE).0;
    let eg = (-0.035_f32 / ((200.0 * 0.0625 + 0.001) / 8.82)).exp();
    near(
        frequency(&out[480..2880], RATE),
        key_hz(69.0 + 12.0 * eg),
        2.0,
    );
}

#[test]
fn vel_scales_only_the_filter_and_osc_mod_depths_never_level() {
    // With VEL on the output at velocity 1 is exactly twice the one at 0.5 unless the EG reaches Filter or
    // Osc. Mod.: EG → Pitch and the EG term in Sync's ratio ignore VEL, and VEL never changes level.
    let play = |params: &[(usize, f32)], velocity: f32| {
        let mut s = synth(RATE, params, &[(WAVE_A, SAW as f32), (WAVE_B, SINE as f32)]);
        s.note_on(69, velocity, 0.5);
        render(&mut s, 4800, RATE).0
    };
    let proportional = |params: &[(usize, f32)]| {
        let full = play(params, 1.0);
        let half: Vec<f32> = play(params, 0.5).iter().map(|x| 2.0 * x).collect();
        full == half
    };
    assert!(proportional(&[(74, 1.0)]), "level");
    assert!(
        proportional(&[(74, 1.0), (40, 1.0), (41, 1.0)]),
        "EG → Pitch"
    );
    let sync = [(74, 1.0), (40, 1.0), (41, 0.5), (16, 2.0 / 3.0), (11, 0.25)];
    assert!(proportional(&sync), "EG in Sync's m′");
    // On at any nonzero v, 0.5 and 0.001 included.
    for vel in [0.001, 0.5] {
        assert!(
            !proportional(&[(74, vel), (40, 0.75), (21, 0.4)]),
            "EG → Filter"
        );
    }
    assert!(proportional(&[(74, 0.0), (40, 0.75), (21, 0.4)]), "VEL off");
}

/// Ring sideband over carrier: A 440 Hz, B 19 st up, A alone in the mix (BALANCE 0 %). A·(1 + 3m·B) with two 0.5
/// sines puts 0.375m at f_B ± f_A beside the carrier's 0.5, so the ratio is 0.75m.
fn ring_ratio(params: &[(usize, f32)], velocity: f32) -> f32 {
    let mut all = vec![(11, 0.906_25)];
    all.extend_from_slice(params);
    let mut s = synth(RATE, &all, &[(WAVE_A, SINE as f32), (WAVE_B, SINE as f32)]);
    s.note_on(69, velocity, 0.5);
    let out = render(&mut s, 48_000, RATE).0;
    let b = 440.0 * 2.0_f64.powf(19.0 / 12.0);
    (amplitude(&out, b + 440.0, RATE) / amplitude(&out, 440.0, RATE)) as f32
}

#[test]
fn ring_depth_is_3m_from_amount_eg_vel_and_lfo() {
    near(ring_ratio(&[(15, 0.5)], 1.0), 0.75 * 0.5, 0.01);
    // EG → Osc. Mod |c|c: c = 0.5 → m = 0.25.
    near(ring_ratio(&[(41, 0.5), (40, 0.75)], 1.0), 0.75 * 0.25, 0.01);
    // VEL on: c = 1 at velocity 0.5 → m = 0.5; off → m = 1.
    near(
        ring_ratio(&[(41, 0.5), (40, 1.0), (74, 1.0)], 0.5),
        0.75 * 0.5,
        0.01,
    );
    near(ring_ratio(&[(41, 0.5), (40, 1.0)], 0.5), 0.75, 0.015);
    // LFO → MOD |A|A: a Square LFO at −1 with A = 0.5 takes 0.25 off MOD. AMNT 0.75.
    let lfo = [(15, 0.75), (35, 0.25), (33, 1.0), (32, 1.0), (34, 0.75)];
    near(ring_ratio(&lfo, 1.0), 0.75 * 0.5, 0.01);
}

/// A's own frequency under Sync, from the median rising-zero-crossing interval: B sits 24.5 st down, so A runs
/// several cycles between B's wraps.
fn sync_frequency(params: &[(usize, f32)], velocity: f32) -> f32 {
    let mut all = vec![(16, 2.0 / 3.0), (11, 0.0), (12, 0.0)];
    all.extend_from_slice(params);
    let mut s = synth(RATE, &all, &[(WAVE_A, SINE as f32), (WAVE_B, SINE as f32)]);
    s.note_on(69, velocity, 0.5);
    let out = render(&mut s, 24_000, RATE).0;
    let crossings: Vec<f32> = (out.windows(2).enumerate())
        .filter(|(_, w)| w[0] < 0.0 && w[1] >= 0.0)
        .map(|(i, w)| i as f32 + w[0] / (w[0] - w[1]))
        .collect();
    let mut gaps: Vec<f32> = crossings.windows(2).map(|w| w[1] - w[0]).collect();
    gaps.sort_by(f32::total_cmp);
    RATE / gaps[gaps.len() / 2]
}

#[test]
fn sync_runs_a_at_two_to_the_m_prime() {
    // m′ = clamp(v15 + MW, 0, 1) + (2v19 − 1)·E_mod + mods.
    near(
        sync_frequency(&[(15, 0.5)], 1.0),
        440.0 * 2.0_f32.sqrt(),
        2.0,
    );
    near(sync_frequency(&[(15, 1.0)], 1.0), 880.0, 3.0);
    // The EG term ignores VEL: c = 1 at velocity 0.5 still adds 1.
    near(
        sync_frequency(&[(41, 0.5), (40, 1.0), (74, 1.0)], 0.5),
        880.0,
        3.0,
    );
}

#[test]
fn lfo_level_saw_sign_and_the_master_floor() {
    let peak_after = |params: &[(usize, f32)]| {
        let mut s = sine(RATE, params);
        s.note_on(69, 1.0, 0.5);
        peak(&render(&mut s, 4800, RATE).0[2400..])
    };
    let unity = peak_after(&[(1, 48.0 / 66.0)]);
    // LVL ±12·A dB: a Square LFO at −1, A = 0.5 → −6 dB.
    let lvl = [
        (1, 48.0 / 66.0),
        (35, 0.75),
        (33, 1.0),
        (32, 1.0),
        (34, 0.75),
    ];
    near(peak_after(&lvl) / unity, 10.0_f32.powf(-6.0 / 20.0), 0.002);
    // The floor is "below −60 dB": v1 = 0 with LVL at −12 dB sits exactly on it and still sounds.
    let floor = [(1, 0.0), (35, 0.75), (33, 1.0), (32, 1.0), (34, 1.0)];
    near(peak_after(&floor) / unity, 1e-3, 2e-5);
    // Saw 2φ − 1 on PAN (A = 0.5, depth 0.25): a quarter beat in, P = 0.5 + 0.25·(−0.5).
    let mut saw = sine(RATE, &[(35, 1.0), (33, 0.5), (34, 0.75), (32, 4.0 / 9.0)]);
    saw.note_on(69, 1.0, 0.5);
    let (left, right) = render(&mut saw, 7000, RATE);
    let around: Vec<f32> = pans(&left[5900..6100], &right[5900..6100])
        .into_iter()
        .flatten()
        .collect();
    near(
        around.iter().sum::<f32>() / around.len() as f32,
        0.375,
        0.01,
    );
}

#[test]
fn noise_level_and_the_mix_scale_with_v20_squared() {
    // The oscillators get g·(1 − v20²): 0.75 of the sine at v20 = 0.5.
    let carrier = |noise: f32| {
        let mut s = sine(RATE, &[(20, noise)]);
        s.note_on(69, 1.0, 0.5);
        amplitude(&render(&mut s, 48_000, RATE).0, 440.0, RATE)
    };
    near((carrier(0.5) / carrier(0.0)) as f32, 0.75, 0.02);
    // NOISE alone (the oscillators cancel): v20² doubles from v20 = 0.5 to √½.
    let noise = |v20: f32| {
        let params = [(13, 0.75), (14, 0.5), (20, v20)];
        let mut s = synth(
            RATE,
            &params,
            &[(WAVE_A, SINE as f32), (WAVE_B, SINE as f32)],
        );
        s.note_on(69, 1.0, 0.5);
        rms(&render(&mut s, 24_000, RATE).0)
    };
    near(noise(0.5_f32.sqrt()) / noise(0.5), 2.0, 0.05);
}

#[test]
fn filter_kinds_lp12_lp24_slopes_and_hp() {
    // RES 0, nominal f = 500 Hz at 44.1 kHz, a 1760 Hz tone. LP 12 taps stage 4 and LP 24 stage 8 of one linear
    // cascade H: out12 = H⁴·in and out24 = H⁸·in at any cutoff, so out24(c)·out12(1)² / (out24(1)·out12(c)²) = 1.
    let rate = 44_100.0;
    let c = (500.0_f32 / 32.0).log2() / 9.27;
    let level = |cutoff: f32, kind: f32, key: u8| {
        let mut s = sine(rate, &[(21, cutoff), (23, kind)]);
        s.note_on(key, 1.0, 0.5);
        let out = render(&mut s, 44_100, rate).0;
        amplitude(&out[4410..], f64::from(key_hz(f32::from(key))), rate)
    };
    let (lp12, lp24) = (level(c, 0.0, 93), level(c, 0.5, 93));
    let (open12, open24) = (level(1.0, 0.0, 93), level(1.0, 0.5, 93));
    assert!(lp12 < 0.5 * open12, "LP 12 passes {lp12}");
    near(
        (lp24 * open12 * open12 / (open24 * lp12 * lp12)) as f32,
        1.0,
        0.01,
    );
    let octave = (20.0 * (level(c, 0.0, 105) / level(c, 0.0, 93)).log10()) as f32;
    assert!((-25.0..-19.0).contains(&octave), "LP 12: {octave} dB/oct");
    // HP = clip input − stage 1: an 8 Hz sine (key 0) under a 20 kHz cutoff is gone (the clip's own 8 Hz part
    // included); 440 Hz over 32 Hz passes.
    let mut dc = sine(rate, &[(23, 1.0)]);
    dc.note_on(0, 1.0, 0.5);
    let out = render(&mut dc, 44_100, rate).0;
    assert!(amplitude(&out[4410..], f64::from(key_hz(0.0)), rate) < 3e-4);
    let mut high = sine(rate, &[(23, 1.0), (21, 0.0)]);
    high.note_on(69, 1.0, 0.5);
    assert!(peak(&render(&mut high, 4410, rate).0[441..]) > 0.2);
}

#[test]
fn cutoff_base_snaps_at_note_start_then_glides_at_0_001() {
    // Key 93 (1760 Hz). CUTOFF 1 at note start: open from the first block.
    let mut open = sine(RATE, &[]);
    open.note_on(93, 1.0, 0.5);
    assert!(peak(&render(&mut open, 64, RATE).0[8..]) > 0.2);
    // CUTOFF 0 → 1 at a block start: S = 1 − 0.999ⁿ is 0.18 (102 Hz) 200 samples on and 0.98 after 4000.
    let mut glide = sine(RATE, &[(21, 0.0)]);
    glide.note_on(93, 1.0, 0.5);
    render(&mut glide, 32, RATE);
    glide.set_parameter(21, 1.0);
    let out = render(&mut glide, 4400, RATE).0;
    assert!(peak(&out[180..220]) < 0.01);
    assert!(peak(&out[4000..]) > 0.2);
}

#[test]
fn voice_pan_gain_master_and_mod_amount_smoothing() {
    // Voice pan p = 0.25: L = min(1, 2 − 2p) = 1, R = min(1, 2p) = 0.5.
    let mut panned = sine(RATE, &[]);
    panned.note_on(69, 1.0, 0.25);
    let (l, r) = render(&mut panned, 4800, RATE);
    near(peak(&r) / peak(&l), 0.5, 0.005);

    // Against a reference that changes nothing, sample by sample after a change at a block start.
    let ratio_after = |change: &dyn Fn(&mut GmSynth), params: &[(usize, f32)]| {
        let (mut a, mut b) = (sine(RATE, params), sine(RATE, params));
        a.note_on(69, 1.0, 0.5);
        b.note_on(69, 1.0, 0.5);
        render(&mut a, 320, RATE);
        render(&mut b, 320, RATE);
        change(&mut a);
        let (a, b) = (render(&mut a, 600, RATE).0, render(&mut b, 600, RATE).0);
        (a, b)
    };
    // Gain: a MONO legato note on the same key at velocity 0.5 → 0.5 + 0.5·0.99ⁿ.
    let (a, b) = ratio_after(&|s| s.note_on(69, 0.5, 0.5), &[(30, 1.0)]);
    for n in (0..600).filter(|&n| b[n].abs() > 0.05) {
        near(a[n] / b[n], 0.5 + 0.5 * 0.99_f32.powi(n as i32 + 1), 1e-3);
    }
    // Master level: v1 to 0 (−48 dB) → c + (1 − c)·0.99ⁿ.
    let floor = 10.0_f32.powf(-48.0 / 20.0) / 10.0_f32.powf((66.0 * 0.7273 - 48.0) / 20.0);
    let (a, b) = ratio_after(&|s| s.set_parameter(1, 0.0), &[]);
    for n in (0..600).filter(|&n| b[n].abs() > 0.05) {
        near(
            a[n] / b[n],
            floor + (1.0 - floor) * 0.99_f32.powi(n as i32 + 1),
            1e-3,
        );
    }
    // MOD. AMNT 0 → 1 on Ring: m climbs at 0.01 per sample, so the first samples barely move.
    let ring = [(11, 0.75)];
    let (a, b) = ratio_after(&|s| s.set_parameter(15, 1.0), &ring);
    let moved =
        |range: std::ops::Range<usize>| range.map(|n| (a[n] - b[n]).abs()).fold(0.0_f32, f32::max);
    assert!(moved(0..8) < 0.05, "{}", moved(0..8));
    assert!(moved(400..600) > 0.1);
}

#[test]
fn band_limit_dither_free_level_and_random_phases() {
    // B alone, a Saw at 7040 Hz (key 93, PITCH B +24) at 48 kHz: its table holds 4 harmonics (the 4th past Nyquist
    // by the +1), chosen from f_B; the 5th would alias to 12.8 kHz.
    let mut saw = synth(
        RATE,
        &[(13, 1.0), (11, 1.0)],
        &[(WAVE_A, SINE as f32), (WAVE_B, SAW as f32)],
    );
    saw.note_on(93, 1.0, 0.5);
    let out = render(&mut saw, 48_000, RATE).0;
    assert!(amplitude(&out, 7040.0, RATE) > 0.1);
    assert!(amplitude(&out, 12_800.0, RATE) < 0.001);

    // The 22-bit dither turns the 1024-step lookup's error into noise: no spur where truncation would put one
    // (1024·440 Hz folds to 18.56 kHz, ± 440 Hz).
    let mut sine_440 = sine(RATE, &[]);
    sine_440.note_on(69, 1.0, 0.5);
    let out = render(&mut sine_440, 48_000, RATE).0;
    for spur in [18_120.0, 19_000.0] {
        assert!(amplitude(&out, spur, RATE) < 2e-5, "{spur} Hz");
    }

    // A releasing voice sounds until its amp env falls below 10⁻⁴: REL 0.5 (τ 56.8 ms) still sounds 0.3–0.45 s in.
    let mut release = sine(RATE, &[(6, 0.5)]);
    release.note_on(69, 1.0, 0.5);
    render(&mut release, 4800, RATE);
    release.note_off(69);
    let out = render(&mut release, 28_800, RATE).0;
    assert!(peak(&out[14_400..21_600]) > 0.0);
    assert_eq!(peak(&out[26_000..]), 0.0);

    // An idle voice gets random A phases: four undetuned copies don't add up in phase (that would peak at 0.67).
    let mut choir = sine(RATE, &[(42, 1.0)]);
    choir.note_on(69, 1.0, 0.5);
    assert!(peak(&render(&mut choir, 4800, RATE).0) < 0.6);
}

#[test]
fn filter_mod_moves_the_cutoff_by_half_m_times_b() {
    // Filter kind, m = 1: c = S + 0.5·m·B per sample. B is a 0.5 sine at 2 Hz (key 0, PITCH B −24, FINE B −0.5), so
    // c sweeps 0.5 ± 0.25 and the ringing (RES 1, driven by NOISE 1, which also mutes the oscillators) sweeps with it:
    // nominal f from 160 Hz to 3.96 kHz.
    let rate = 44_100.0;
    let params = [
        (16, 1.0),
        (15, 1.0),
        (11, 0.0),
        (12, 0.0),
        (20, 1.0),
        (21, 0.5),
        (22, 1.0),
    ];
    let mut s = synth(
        rate,
        &params,
        &[(WAVE_A, SINE as f32), (WAVE_B, SINE as f32)],
    );
    s.note_on(0, 1.0, 0.5);
    let out = render(&mut s, 2 * 44_100, rate).0;
    let highest = out[4410..]
        .chunks(1024)
        .map(|window| frequency(window, rate))
        .fold(0.0_f32, f32::max);
    let top = 1.0159 * 32.0 * (9.27_f32 * 0.75).exp2() + 0.5;
    assert!(
        (0.95 * top..1.05 * top).contains(&highest),
        "{highest} Hz, expected about {top}"
    );
}

#[test]
fn mono_is_on_at_any_nonzero_value() {
    // MONO at v ≠ 0. Keys 60 then 64: MONO sounds only 64 (legato on one voice), poly sounds both.
    let play = |mono: f32| {
        let mut s = sine(RATE, &[(30, mono)]);
        s.note_on(60, 1.0, 0.5);
        s.note_on(64, 1.0, 0.5);
        let out = render(&mut s, 48_000, RATE).0;
        (
            amplitude(&out, f64::from(key_hz(60.0)), RATE),
            amplitude(&out, f64::from(key_hz(64.0)), RATE),
        )
    };
    for mono in [0.001, 0.5] {
        let (low, high) = play(mono);
        assert!(low < 0.01 && high > 0.2, "MONO {mono}: {low} {high}");
    }
    let (low, high) = play(0.0);
    assert!(low > 0.2 && high > 0.2);
}

#[test]
fn a_solo_choir_is_one_centred_copy_at_ratio_one() {
    // N = 1 skips the detune, the Chord/Octave multiply and the stereo spread: one 440 Hz copy, L = R, at
    // g = 0.5·(1 − v20²).
    let mut s = sine(RATE, &[(44, 1.0), (43, 1.0), (45, 1.0)]);
    s.note_on(69, 1.0, 0.5);
    let (left, right) = render(&mut s, 48_000, RATE);
    assert_eq!(left, right);
    near(frequency(&left, RATE), 440.0, 0.3);
    near(amplitude(&left, 440.0, RATE) as f32, 0.25, 0.005);
}
