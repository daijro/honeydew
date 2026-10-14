//! MiniSynth's DSP laws, checked against closed forms and hand values.

use super::*;

const SR: f32 = 44_100.0;

fn fresh() -> MiniSynth {
    MiniSynth::new(SR).unwrap()
}

fn render(synth: &mut MiniSynth, frames: usize) -> (Vec<f32>, Vec<f32>) {
    let (mut left, mut right) = (vec![0.0; frames], vec![0.0; frames]);
    synth.process(&mut left, &mut right).unwrap();
    (left, right)
}

fn keys(synth: &MiniSynth) -> [Option<u8>; VOICES] {
    synth.voices.map(|voice| voice.busy.then_some(voice.key))
}

// ---- voices and notes ----

#[test]
fn poly_takes_the_highest_free_voice_and_steals_the_oldest_started() {
    let mut synth = fresh();
    for key in [60, 62, 64] {
        synth.note_on(key, 1.0);
    }
    assert_eq!(keys(&synth), [None, None, Some(64), Some(62), Some(60)]);
    synth.note_on(65, 1.0);
    synth.note_on(67, 1.0);
    // Releasing voices count as busy: the sixth note steals the oldest-started voice, the releasing 60.
    synth.note_off(60);
    assert_eq!(synth.voices[4].amp.stage, Stage::Release);
    synth.note_on(69, 1.0);
    assert_eq!(
        keys(&synth),
        [Some(67), Some(65), Some(64), Some(62), Some(69)]
    );
    synth.note_on(71, 1.0);
    assert_eq!(keys(&synth)[3], Some(71));
}

#[test]
fn mono_and_lead_always_use_voice_one() {
    for mode in [0.5, 1.0] {
        let mut synth = fresh();
        synth.set_parameter(1, mode).unwrap();
        synth.note_on(60, 1.0);
        synth.note_on(64, 1.0);
        assert_eq!(keys(&synth), [Some(64), None, None, None, None], "{mode}");
    }
}

#[test]
fn poly_and_mono_note_off_release_every_voice_on_the_key() {
    let mut synth = fresh();
    synth.note_on(60, 1.0);
    synth.note_on(60, 1.0);
    synth.note_off(60);
    assert!(
        synth.voices[3..]
            .iter()
            .all(|v| v.amp.stage == Stage::Release)
    );

    // MONO: releasing a non-sounding key does nothing; releasing the sounding key releases the voice although
    // another key is held.
    let mut synth = fresh();
    synth.set_parameter(1, 0.5).unwrap();
    synth.note_on(60, 1.0);
    synth.note_on(64, 1.0);
    synth.note_off(60);
    assert_ne!(synth.voices[0].amp.stage, Stage::Release);
    synth.note_on(60, 1.0);
    synth.note_on(67, 1.0);
    synth.note_off(67);
    assert_eq!(synth.voices[0].amp.stage, Stage::Release);
}

#[test]
fn lead_is_legato_while_another_key_is_held_and_falls_back_on_held_keys() {
    let mut synth = fresh();
    synth.set_parameter(1, 1.0).unwrap();
    synth.set_parameter(13, 0.5).unwrap();
    synth.note_on(60, 0.5);
    render(&mut synth, 1000);
    let filter_env = synth.voices[0].filter_env.level;
    assert!(filter_env > 0.0);
    // Legato: only key and velocity change; the filter env is not restarted.
    synth.note_on(64, 0.8);
    let voice = synth.voices[0];
    assert_eq!((voice.key, voice.velocity), (64, 0.8));
    assert_eq!(voice.filter_env.level, filter_env);
    // Releasing the sounding key moves the voice to the highest held key below, with that key's velocity.
    synth.note_on(67, 0.3);
    synth.note_off(67);
    assert_eq!((synth.voices[0].key, synth.voices[0].velocity), (64, 0.8));
    assert_ne!(synth.voices[0].amp.stage, Stage::Release);
    // None below: the lowest held key above.
    synth.note_on(55, 0.3);
    synth.note_off(55);
    assert_eq!((synth.voices[0].key, synth.voices[0].velocity), (60, 0.5));
    // Releasing a held key that is not sounding leaves the voice alone.
    synth.note_off(64);
    assert_eq!(synth.voices[0].key, 60);
    assert_ne!(synth.voices[0].amp.stage, Stage::Release);
    // No key held: it releases.
    synth.note_off(60);
    assert_eq!(synth.voices[0].amp.stage, Stage::Release);
    // A released voice retriggers, and with no other key held the glide starts at the note's own key.
    synth.note_on(72, 1.0);
    assert_eq!(synth.voices[0].filter_env.level, 0.0);
    assert_eq!(synth.voices[0].pitch, 72.0);
}

#[test]
fn retrigger_attacks_from_the_current_level_and_restarts_the_filter_env() {
    let mut synth = fresh();
    synth.set_parameter(1, 0.5).unwrap();
    synth.set_parameter(22, 0.5).unwrap();
    synth.note_on(60, 1.0);
    render(&mut synth, 1000);
    let level = synth.voices[0].amp.level;
    assert!(level > 0.0 && level < 1.0);
    synth.note_on(62, 1.0);
    let voice = synth.voices[0];
    assert_eq!((voice.amp.level, voice.amp.stage), (level, Stage::Attack));
    assert_eq!(voice.filter_env.level, 0.0);
}

#[test]
fn enable_off_resets_and_ignores_notes() {
    let mut synth = fresh();
    synth.note_on(60, 1.0);
    // 0.5 is on.
    synth.set_parameter(0, 0.5).unwrap();
    assert!(synth.voices[4].busy);
    synth.set_parameter(0, 0.49).unwrap();
    assert_eq!(keys(&synth), [None; VOICES]);
    synth.note_on(62, 1.0);
    assert_eq!(keys(&synth), [None; VOICES]);
    assert_eq!(render(&mut synth, 64).0, [0.0; 64]);
    synth.set_parameter(0, 1.0).unwrap();
    assert_eq!(render(&mut synth, 64).0, [0.0; 64]);
}

// ---- envelopes ----

/// Seconds at 44.1 kHz for `samples`.
fn seconds(samples: f32) -> f32 {
    samples / 44_100.0
}

/// The time constant of a one-pole step `k`, in samples (in f64: 1 − k is not exact in f32).
fn tau(k: f32) -> f32 {
    (-1.0 / (-f64::from(k)).ln_1p()) as f32
}

#[test]
fn envelope_rates_give_the_spec_times() {
    // Attack aims at 4 and ends at 1: 4·(1 − (1 − k)ⁿ) = 1.
    let attack = |v| seconds((4.0f32 / 3.0).ln() * tau(attack_rate(v)));
    assert_eq!(attack_rate(0.0), 1.0);
    assert_eq!(attack_rate(0.0009), 1.0);
    // Each printed time holds to half a unit of its last digit.
    let close = |t: f32, (s, half): (f32, f32)| (t - s).abs() <= half;
    for (v, s) in [
        (0.25, (0.111, 0.0005)),
        (0.5, (1.24, 0.005)),
        (0.75, (5.51, 0.005)),
        (1.0, (16.3, 0.05)),
    ] {
        assert!(close(attack(v), s), "attack {v}: {}", attack(v));
    }
    assert_eq!(decay_rate(0.0176), 1.0);
    for (v, s) in [
        (0.25, (0.0387, 0.000_05)),
        (0.5, (0.431, 0.0005)),
        (0.75, (1.92, 0.005)),
        (1.0, (5.67, 0.005)),
    ] {
        let t = seconds(tau(decay_rate(v)));
        assert!(close(t, s), "decay {v}: {t}");
    }
    for (v, s) in [
        (0.0, (0.000_04, 0.000_005)),
        (0.07, (0.000_37, 0.000_005)),
        (0.25, (0.0062, 0.000_05)),
        (0.5, (0.051, 0.0005)),
        (0.75, (0.204, 0.0005)),
        (1.0, (0.567, 0.0005)),
    ] {
        let t = seconds(tau(amp_release_rate(v)));
        assert!(close(t, s), "release {v}: {t}");
    }
}

#[test]
fn envelope_attacks_toward_four_and_latches_decay_at_one() {
    let mut env = Env::default();
    env.attack(1.0);
    // k = 1 reaches 4 in one sample, which ends the attack at exactly 1.
    assert_eq!(env.tick(0.5, 0.25), 1.0);
    assert_eq!(env.stage, Stage::Decay);
    // The decay's rate and target were latched at its start.
    assert_eq!(env.tick(1.0, 0.9), 0.625);
    env.release(0.5);
    assert_eq!(env.tick(1.0, 0.9), 0.3125);
}

// ---- pitch ----

#[test]
fn glide_moves_once_per_control_block_from_the_previous_key() {
    // c = 0.9·(1 − v)⁴ + 0.01 per 32-frame block; τ at 44.1 kHz.
    for (v, ms, half) in [
        (0.0, 0.30, 0.005),
        (0.25, 2.1, 0.05),
        (0.5, 10.6, 0.05),
        (0.75, 53.0, 0.5),
        (1.0, 72.0, 0.5),
    ] {
        let t = 1000.0 * seconds(32.0 * tau(glide_rate(v)));
        assert!((t - ms).abs() <= half, "{v}: {t}");
    }
    let mut synth = fresh();
    synth.set_parameter(3, 1.0).unwrap();
    // One frame in, so the next control block starts 31 frames later.
    render(&mut synth, 1);
    synth.note_on(60, 1.0);
    assert_eq!(synth.voices[4].pitch, 60.0);
    synth.note_on(72, 1.0);
    // POLY too: the new voice starts at the previously pressed key.
    assert_eq!(synth.voices[3].pitch, 60.0);
    render(&mut synth, 31);
    assert_eq!(synth.voices[3].pitch, 60.0);
    render(&mut synth, 1);
    assert!((synth.voices[3].pitch - 60.12).abs() < 1e-5);
    // None after reset.
    synth.reset();
    synth.note_on(48, 1.0);
    assert_eq!(synth.voices[4].pitch, 48.0);
}

#[test]
fn transpose_is_the_fused_step() {
    // s = trunc(fma_f32(v, 24, −12)) at v = f32(k/24): k − 12, except
    // k = 8, 11, 14, 17, 20 → −3, 0, 1, 4, 7.
    for k in 0..=24 {
        let v = (f64::from(k) / 24.0) as f32;
        let expected = match k {
            8 => -3,
            11 => 0,
            14 => 1,
            17 => 4,
            20 => 7,
            _ => k - 12,
        };
        assert_eq!(transpose_steps(v), expected, "k = {k}");
    }
    assert_eq!(transpose_steps(-0.5), -12);
    assert_eq!(transpose_steps(1.5), 12);
}

#[test]
fn drift_walks_toward_a_fresh_random_target_each_block() {
    let mut synth = fresh();
    synth.note_on(60, 1.0);
    assert_eq!(synth.voices[4].drift, 0.0);
    render(&mut synth, 32);
    // d += 0.01·(r − d) with r in [−1, 1).
    let d = synth.voices[4].drift;
    assert!(d != 0.0 && d.abs() <= 0.01, "{d}");
    // It adds 0.08·d semitones to the pitch (and 0.103·d octaves to the cutoff: the filter env test).
    let voice = synth.voices[4];
    assert_eq!(
        voice.frequency,
        440.0 * ((voice.pitch + 0.08 * d - 69.0) / 12.0).exp2()
    );
    assert_ne!(
        voice.frequency,
        440.0 * ((voice.pitch - 69.0) / 12.0).exp2()
    );
}

// ---- voice output ----

#[test]
fn gain_and_pan_follow_the_linear_laws_and_a_new_voice_starts_at_its_target() {
    let mut synth = fresh();
    synth.note_on_with_pan(60, 0.6, 0.25);
    // L = min(1, 2 − 2p), R = min(1, 2p).
    assert_eq!(synth.voices[4].gain, [0.6, 0.3]);
    // Slides add velocity and pan before the clamps; the smoothing moves 0.005 of the gap per sample.
    synth.slide(0.0, 0.25, 0.6);
    render(&mut synth, 32);
    assert_eq!(synth.voices[4].target, [1.0, 1.0]);
    let gain = synth.voices[4].gain[1];
    render(&mut synth, 1);
    assert!((synth.voices[4].gain[1] - (gain + (1.0 - gain) * 0.005)).abs() < 1e-6);
}

#[test]
fn slides_add_pitch_and_clear_when_a_free_voice_starts() {
    let mut synth = fresh();
    synth.note_on(60, 1.0);
    synth.slide(2.0, 0.0, 0.0);
    assert_eq!(synth.voices[4].slide, [2.0, 0.0, 0.0]);
    synth.note_on(64, 1.0);
    assert_eq!(synth.voices[3].slide, [0.0; 3]);
}

#[test]
fn a_released_voice_frees_once_both_smoothed_gains_fall_below_1e_5() {
    let mut synth = fresh();
    synth.set_parameter(25, 0.0).unwrap();
    synth.note_on(60, 1.0);
    render(&mut synth, 100);
    synth.note_off(60);
    // A < 1e-5 after about 20 samples; then the gains fall from 1 by 0.005 of themselves per sample:
    // ln(1e5)/−ln(0.995) = 2297 samples.
    render(&mut synth, 2250);
    assert!(synth.voices[4].busy);
    render(&mut synth, 150);
    assert!(!synth.voices[4].busy);
}

// ---- oscillators ----

/// Table `harmonics` (1…1024) of the base wave.
fn table(harmonics: usize) -> &'static [f32] {
    &tables()[(harmonics - 1) * TABLE..harmonics * TABLE]
}

#[test]
fn the_base_wave_has_the_spec_shape() {
    // A falling ramp with a rounded start; peak +0.332 at 14 % of the cycle, −0.463 at the
    // cycle end, rms 0.246 (the closed form is within 0.005 of the shipped spectrum).
    let wave = table(1024);
    let (peak_at, peak) =
        wave.iter().enumerate().fold(
            (0, f32::MIN),
            |best, (i, &x)| if x > best.1 { (i, x) } else { best },
        );
    assert!((peak - 0.332).abs() < 0.005, "{peak}");
    assert!(
        (peak_at as f32 / TABLE as f32 - 0.14).abs() < 0.01,
        "{peak_at}"
    );
    assert!(
        (wave[TABLE - 1] - -0.463).abs() < 0.005,
        "{}",
        wave[TABLE - 1]
    );
    let rms = (wave.iter().map(|x| x * x).sum::<f32>() / TABLE as f32).sqrt();
    assert!((rms - 0.246).abs() < 0.003, "{rms}");
    // T_1 is the first harmonic alone: M_1·cos(φ_1 + 2πn/2048).
    let (m, phase) = harmonic(1);
    assert!(
        (f64::from(table(1)[512]) - m * (phase + std::f64::consts::FRAC_PI_2).cos()).abs() < 1e-6
    );
}

#[test]
fn the_table_follows_the_first_oscillator() {
    // H = 1024 up to sr/2048, else clamp(floor(sr/(2·f₁)) + 1, 1, 1024).
    for (f, h) in [
        (20.0, 1024),
        (SR / 2048.0, 1024),
        (100.0, 221),
        (10_000.0, 3),
        (30_000.0, 1),
    ] {
        assert_eq!(harmonics(f, SR), h, "{f}");
    }
    // SAW SPREAD's first oscillator plays f·(0.5 − 0.005m): A4 reads table floor(44100/440) + 1 = 101.
    let mut synth = fresh();
    synth.set_parameter(6, 8.0 / 9.0).unwrap();
    synth.note_on(69, 1.0);
    assert_eq!(synth.voices[4].wave.as_ptr(), table(101).as_ptr());
}

#[test]
fn oscillator_increments_follow_each_kind() {
    // trunc(f·2³²/sr), at A4 = 440 Hz (a new voice has no drift yet).
    let step = |f: f32| (f64::from(f) * 4_294_967_296.0 / f64::from(SR)) as u32;
    let voice = |kind: f32, modifier: f32| {
        let mut synth = fresh();
        synth.set_parameter(6, kind / 9.0).unwrap();
        synth.set_parameter(7, modifier).unwrap();
        synth.note_on(69, 1.0);
        synth.voices[4]
    };
    assert_eq!(
        voice(0.0, 0.3).increments,
        [step(440.0), 0, 0, 0, 0, 0, 0, 0]
    );
    assert_eq!(step(440.0), 42_852_281);
    // SAW+SAW: r = 2^(s/12 + 0.00625), s = trunc(double(fma_f32(m, 2, −1))·12).
    let r = interval(0.75);
    assert_eq!(r, (6.0f32 / 12.0 + 0.00625).exp2());
    assert_eq!(
        voice(1.0, 0.75).increments[..2],
        [step(440.0), step(440.0 * r)]
    );
    // SUPERSAW: 8 at f·(1 + m·c).
    let spread = [-0.05, -0.0375, -0.025, -0.0125, 0.0, 0.0125, 0.025, 0.0375];
    assert_eq!(
        voice(5.0, 0.4).increments,
        spread.map(|c: f32| step(440.0 * (1.0 + 0.4 * c)))
    );
    // SAW SPREAD and SAW SPREAD 2.
    assert_eq!(
        voice(8.0, 0.5).increments[..3],
        [step(440.0 * 0.4975), step(440.0), step(440.0 * 2.01)]
    );
    assert_eq!(
        voice(9.0, 0.5).increments[..3],
        [step(440.0 * 1.4925), step(440.0), step(440.0 * 2.01)]
    );
}

#[test]
fn the_modifier_interval_is_the_round_12_law() {
    for (m, s) in [
        (0.0, -12.0),
        (0.5, 0.0),
        (0.75, 6.0),
        (1.0, 12.0),
        (-1.0, -36.0),
    ] {
        assert_eq!(interval(m), (s / 12.0 + 0.00625f32).exp2(), "{m}");
    }
    // Truncated toward zero: 2·0.52 − 1 = 0.04 → 0.48 → 0 semitones.
    assert_eq!(interval(0.52), 0.00625f32.exp2());
    // The f64 multiply by 12, at m = 0.083333343 and 0.95833331 (the shortest f32 literals below):
    // fma_f32(m, 2, −1) = −0.83333331 → −9.9999998 → −9, where an f32 multiply would round to −10; and
    // 0.91666663 → 10.9999996 → 10.
    let at = |s: f32| (s / 12.0 + 0.00625f32).exp2();
    assert_eq!(interval(0.083_333_34), at(-9.0));
    assert_eq!(interval(0.958_333_3), at(10.0));
}

#[test]
fn fixed_offsets_follow_the_modifier() {
    let offsets = |kind: f32, modifier: f32| {
        let mut synth = fresh();
        synth.set_parameter(6, kind / 9.0).unwrap();
        synth.set_parameter(7, modifier).unwrap();
        synth.note_on(69, 1.0);
        synth.voices[4].offsets
    };
    let cycles = |x: f64| (x * 4_294_967_296.0) as u64 as u32;
    // d = 1 − |m| cycles; SQR+SAW ½; SQR+SQR 0.5625; BELL k·|m|/2.
    assert_eq!(offsets(0.0, 0.25)[0], cycles(0.75));
    assert_eq!(offsets(2.0, 0.0)[0], 0);
    assert_eq!(offsets(3.0, 0.3)[0], 1 << 31);
    assert_eq!(offsets(4.0, 0.3)[0], 0x9000_0000);
    assert_eq!(offsets(6.0, 0.5), [0.25, 0.5, 0.75, 1.0, 1.25].map(cycles));
}

#[test]
fn kinds_combine_table_reads_as_the_spec_lists() {
    let wave = table(1024);
    let s = |phase: u32| wave[(phase >> 21) as usize];
    let mut voice = Voice {
        wave,
        phases: [0x1000_0000, 0x5000_0000, 0x9000_0000, 0, 0, 0, 0, 0],
        offsets: [0x2000_0000, 0, 0, 0, 0],
        ..Voice::default()
    };
    let (a, b, c) = (0x1000_0000u32, 0x5000_0000u32, 0x9000_0000u32);
    let d = a + 0x2000_0000;
    // Dither 0 and increments 0, so every call reads the same phases.
    assert_eq!(voice.oscillators(0, 0), 2.0 * s(a) - s(d));
    assert_eq!(voice.oscillators(1, 0), s(a) + s(b));
    assert_eq!(voice.oscillators(2, 0), s(a) - s(d));
    assert_eq!(voice.oscillators(3, 0), s(a) - s(d) + s(b));
    assert_eq!(
        voice.oscillators(4, 0),
        s(a) - s(d) + s(b) - s(b + 0x2000_0000)
    );
    assert_eq!(voice.oscillators(7, 0), s(a) + s(d));
    assert_eq!(voice.oscillators(8, 0), s(a) + s(b) + s(c));
    // SUPERSAW: 8 × 0.5·S, one per oscillator; BELL: 6 × 0.5·S(φ + offset k), k = 0…5, all from oscillator 1.
    let phases: [u32; 8] = std::array::from_fn(|i| 0x0F00_0000 * (i as u32 + 1));
    let offsets: [u32; 5] = std::array::from_fn(|k| 0x1300_0000 * (k as u32 + 1));
    let mut many = Voice {
        wave,
        phases,
        offsets,
        ..Voice::default()
    };
    let supersaw = 0.5 * phases.iter().map(|&p| s(p)).sum::<f32>();
    assert!((many.oscillators(5, 0) - supersaw).abs() < 1e-6);
    let bell = 0.5 * (s(phases[0]) + offsets.iter().map(|&k| s(phases[0] + k)).sum::<f32>());
    assert!((many.oscillators(6, 0) - bell).abs() < 1e-6);
    // The dither is added to the phase before the top 11 bits are taken.
    assert_eq!(
        voice.oscillators(1, 0x20_0000),
        s(a + 0x20_0000) + s(b + 0x20_0000)
    );
}

#[test]
fn a_stolen_voice_keeps_its_phases_and_a_freed_one_gets_new_ones() {
    let mut synth = fresh();
    let phases = synth.voices.map(|voice| voice.phases);
    assert!(phases.iter().all(|p| p[0] != 0) && phases[0] != phases[1]);
    for key in [60, 62, 64, 65, 67] {
        synth.note_on(key, 1.0);
    }
    render(&mut synth, 100);
    let before = synth.voices[4].phases;
    synth.note_on(69, 1.0);
    assert_eq!(synth.voices[4].phases, before);
    synth.set_parameter(25, 0.0).unwrap();
    synth.all_notes_off();
    let increments = synth.voices[4].increments;
    for _ in 0..4000 {
        let running = synth.voices[4].phases;
        render(&mut synth, 1);
        if !synth.voices[4].busy {
            // Not where the oscillators would have gone: re-randomized as the voice freed.
            let advanced: [u32; 8] =
                std::array::from_fn(|i| running[i].wrapping_add(increments[i]));
            assert_ne!(synth.voices[4].phases, advanced);
            return;
        }
    }
    panic!("the voice never freed");
}

// ---- filter ----

/// The filter loop, literally: 4096 Euler substeps with u held, then s1 := s1/(1 + s1²), in f64.
fn euler(state: &mut [f64; 4], u: f64, g: f64, k: f64) -> f64 {
    for _ in 0..4096 {
        state[0] += g * (u - k * state[3] - state[0]);
        state[1] += g * (state[0] - state[1]);
        state[2] += g * (state[1] - state[2]);
        state[3] += g * (state[2] - state[3]);
    }
    state[0] /= 1.0 + state[0] * state[0];
    state[3]
}

#[test]
fn the_ladder_matrix_is_4096_euler_substeps() {
    for (fc, k) in [
        (1_000.0, 0.0),
        (5_000.0, 2.5),
        (300.0, 4.0095),
        (20_000.0, 1.0),
    ] {
        let g = fc * 6.25 / (4096.0 * f64::from(SR));
        let mut ladder = Ladder::default();
        ladder.set(g, k);
        let mut reference = [0.0f64; 4];
        for n in 0..200 {
            let u = (0.3 * (n as f32 * 0.37).sin() + 0.1) * if n < 100 { 1.0 } else { 0.0 };
            let want = euler(&mut reference, f64::from(u), g, k);
            let got = ladder.tick(u);
            assert!(
                (f64::from(got) - want).abs() < 1e-5,
                "fc {fc} k {k} n {n}: {got} vs {want}"
            );
        }
    }
}

#[test]
fn each_ladder_stage_cuts_near_fc() {
    // Four poles at fc·6.25/2π: −12 dB at fc, 1/(1 + (1/0.99472)²)² = 0.2474; DC passes at unity.
    let (fc, f) = (1_000.0f64, 1_000.0f32);
    let mut ladder = Ladder::default();
    ladder.set(fc * 6.25 / (4096.0 * f64::from(SR)), 0.0);
    let mut peak = 0.0f32;
    for n in 0..44_100 {
        let y = ladder.tick(0.01 * (TAU * f * n as f32 / SR).sin());
        if n > 22_050 {
            peak = peak.max(y.abs());
        }
    }
    assert!((peak / 0.01 - 0.2474).abs() < 0.003, "{}", peak / 0.01);
    let mut ladder = Ladder::default();
    ladder.set(fc * 6.25 / (4096.0 * f64::from(SR)), 0.0);
    // (The s1 saturation takes s1³/a off a constant input: 8·10⁻⁶ here.)
    let dc = (0..20_000).map(|_| ladder.tick(0.01)).last().unwrap();
    assert!((dc / 0.01 - 1.0).abs() < 1e-3, "{dc}");
}

/// A fresh voice's cutoff for one note: (params, key, velocity).
fn cutoff(params: &[(usize, f32)], key: u8, velocity: f32) -> f32 {
    let mut synth = fresh();
    for &(index, value) in params {
        synth.set_parameter(index, value).unwrap();
    }
    synth.note_on(key, velocity);
    synth.voices[4].cutoff
}

#[test]
fn the_cutoff_follows_the_spec_law() {
    // log2 f₀ = 4 + 10.3·v9 at key 48, velocity 0.5: 16, 95, 568, 3385 Hz; 20171 is clamped to 20 kHz.
    for (v, hz) in [
        (0.0, 16.0),
        (0.25, 95.0),
        (0.5, 568.0),
        (0.75, 3385.0),
        (1.0, 20_000.0),
    ] {
        let fc = cutoff(&[(9, v)], 48, 0.5);
        assert!((fc - hz).abs() < 0.5, "{v}: {fc}");
    }
    // KBD TRK 100 % = 1 oct/oct about key 48; VEL TRK 100 % = ±7 oct.
    let base = cutoff(&[(9, 0.25), (11, 1.0)], 48, 0.5);
    assert!((cutoff(&[(9, 0.25), (11, 1.0)], 60, 0.5) / base - 2.0).abs() < 1e-4);
    let fc = cutoff(&[(9, 0.0), (12, 1.0)], 48, 1.0);
    assert!((fc / 16.0 - 128.0).abs() < 0.01, "{fc}");
    // RES: k = 4.05·0.99·v10.
    let mut synth = fresh();
    synth.set_parameter(10, 1.0).unwrap();
    render(&mut synth, 1);
    assert_eq!(synth.control.resonance, 4.05 * 0.99);
}

#[test]
fn the_filter_envelope_moves_the_cutoff_linearly_in_hz() {
    // fc = f₀ + (f₀·2^(10·(2v17 − 1)) − f₀)·F, F = the filter env at the block start.
    let mut synth = fresh();
    synth.set_parameter(9, 0.0).unwrap();
    synth.set_parameter(17, 1.0).unwrap();
    synth.note_on(48, 0.5);
    assert_eq!(synth.voices[4].cutoff, 16.0);
    // EGF ATK 0: F = 1 after one sample, read at the next block (where the drift adds 0.103·d octave).
    render(&mut synth, 33);
    let f0 = 16.0 * (0.103 * synth.voices[4].drift).exp2();
    let fc = synth.voices[4].cutoff;
    assert!((fc / (f0 * 1024.0) - 1.0).abs() < 1e-5, "{fc}");
    synth.voices[4].filter_env.level = 0.5;
    synth.voice_control(4);
    let want = f0 + (f0 * 1024.0 - f0) * 0.5;
    assert!((synth.voices[4].cutoff / want - 1.0).abs() < 1e-5);
}

#[test]
fn the_amp_envelope_scales_the_input_of_the_filter() {
    // PULSE at Modifier 0 cancels exactly, so u = 0.07·A while A climbs (ATK 0.2): the first block's output is
    // ((0.07·A) → ladder → ×5 → DC blocker) × the gain, sample for sample. Scaling after the filter would differ.
    let params = [(6, 2.0 / 9.0), (9, 0.3), (10, 0.6), (22, 0.2)];
    let mut synth = with(&params);
    synth.note_on(60, 1.0);
    let (left, _) = render(&mut synth, 32);
    // A twin at the same block start: the first control block has set the ladder (drift included).
    let mut twin = with(&params);
    twin.note_on(60, 1.0);
    twin.control_block();
    let (mut voice, c) = (twin.voices[4], twin.control);
    let mut after = voice.ladder;
    let mut dc_after = 0.0;
    for (n, &out) in left.iter().enumerate() {
        let a = voice.amp.tick(c.decay[0], c.sustain[0]);
        assert!(a < 1.0);
        let y = voice.ladder.tick(0.07 * a) * 5.0 - voice.dc;
        voice.dc += 0.005 * y;
        assert_eq!(out, y * voice.gain[0], "{n}");
        let late = after.tick(0.07) * 5.0 * a - dc_after;
        dc_after += 0.005 * late;
        if n == 31 {
            assert!((late / out - 1.0).abs() > 0.5, "{out} vs {late}");
        }
    }
}

#[test]
fn the_signal_path_offsets_then_filters_then_blocks_dc() {
    // u = (osc·0.2 + 0.07)·A → ladder → ×5 → DC blocker, at CUTOFF 0.5 (568 Hz): the offline chain from the voice's
    // own table and increment, without the dither, gives the synth's level. (The s1 saturation compresses the 0.07
    // offset and the wave, so a linear closed form would read about 14 % high here.)
    let mut synth = fresh();
    synth.set_parameter(9, 0.5).unwrap();
    synth.note_on(48, 1.0);
    let voice = synth.voices[4];
    let (left, _) = render(&mut synth, 44_100);
    let level = (left[22_050..].iter().map(|x| x * x).sum::<f32>() / 22_050.0).sqrt();
    let mut ladder = Ladder::default();
    ladder.set(
        f64::from(voice.cutoff) * 6.25 / (4096.0 * f64::from(SR)),
        0.0,
    );
    let (mut phase, mut dc, mut power) = (voice.phases[0], 0.0f32, 0.0f32);
    for n in 0..44_100 {
        let u = voice.wave[(phase >> 21) as usize] * 0.2 + 0.07;
        phase = phase.wrapping_add(voice.increments[0]);
        let y = ladder.tick(u) * 5.0 - dc;
        dc += 0.005 * y;
        if n >= 22_050 {
            power += y * y;
        }
    }
    let want = (power / 22_050.0).sqrt();
    assert!((level / want - 1.0).abs() < 0.02, "{level} vs {want}");
}

// ---- LFO and mod wheel ----

#[test]
fn the_lfo_depth_is_a_signed_square() {
    // L = s·a²·wave, a = 2v21 − 1 (+ v5·mw for MW Dest LFO), s = sign(2v21 − 1).
    for (v, depth) in [
        (0.0, -1.0),
        (0.25, -0.25),
        (0.5, 0.0),
        (0.75, 0.25),
        (1.0, 1.0),
    ] {
        assert_eq!(lfo_depth(v, 0.0), depth, "{v}");
    }
    // The wheel adds before the square; sign(0) = +1, so the wheel still opens a centred amount.
    assert_eq!(lfo_depth(0.5, 0.5), 0.25);
    assert_eq!(lfo_depth(0.25, 0.5), 0.0);
    assert_eq!(lfo_depth(1.0, 1.0), 4.0);
}

#[test]
fn the_lfo_period_comes_from_the_dsp_beats_table() {
    // trunc(11·v20) over 0.25 … 256 beats; the default 0.25 plays index 2 = 1/2 beat; "1 bar" plays 16 beats.
    let beats = |v: f32| RATE_BEATS[rate_index(v)];
    assert_eq!(beats(0.25), 0.5);
    assert_eq!(beats(0.0), 0.25);
    assert_eq!(beats(8.0 / 11.0 + 1e-6), 16.0);
    assert_eq!(beats(1.0), 256.0);
}

/// L at full depth in the control block at song position `beat`, read through LFO Dest FRQ (12·L semitones).
fn lfo_at(shape: f32, beat: f64) -> f32 {
    let mut synth = fresh();
    synth.set_parameter(18, shape).unwrap();
    synth.set_parameter(19, 0.5).unwrap();
    synth.set_parameter(21, 1.0).unwrap();
    synth.set_song_beat(beat);
    render(&mut synth, 1);
    synth.control.lfo_pitch / 12.0
}

#[test]
fn the_lfo_shapes_lock_to_the_song_position() {
    // φ = frac(beat / 0.5): beat 1.1 → 0.2, beat 7.4 → 0.8.
    assert!((lfo_at(0.0, 1.1) - (TAU * 0.2).sin()).abs() < 1e-5);
    assert!((lfo_at(0.5, 1.1) - 0.6).abs() < 1e-5);
    assert!((lfo_at(0.5, 7.4) - -0.6).abs() < 1e-5);
    assert_eq!(lfo_at(1.0, 1.1), 1.0);
    assert_eq!(lfo_at(1.0, 7.4), -1.0);
}

#[test]
fn stopped_the_lfo_runs_freely_at_the_tempo() {
    let mut synth = fresh();
    synth.set_tempo_bpm(150.0).unwrap();
    render(&mut synth, 32 * 100);
    // 3200 frames at 150 BPM, 44.1 kHz.
    assert!((synth.beat - 3200.0 * 150.0 / (60.0 * 44_100.0)).abs() < 1e-9);
}

#[test]
fn lfo_and_wheel_reach_their_destinations() {
    let control = |params: &[(usize, f32)], wheel: f32| {
        let mut synth = fresh();
        for &(index, value) in params {
            synth.set_parameter(index, value).unwrap();
        }
        synth.set_mod_wheel(wheel).unwrap();
        // Square at φ = 0: wave +1, so L = depth.
        synth.set_parameter(18, 1.0).unwrap();
        render(&mut synth, 1);
        synth.control
    };
    let base = control(&[(9, 0.5)], 0.0);
    // LFO Dest FLT: +4·L octaves; FRQ: +12·L semitones; OSCMOD: +L to the modifier.
    assert_eq!(
        control(&[(9, 0.5), (21, 0.75)], 0.0).log2_cutoff,
        base.log2_cutoff + 1.0
    );
    assert_eq!(control(&[(19, 0.5), (21, 0.75)], 0.0).lfo_pitch, 3.0);
    assert_eq!(
        control(&[(7, 0.25), (19, 1.0), (21, 0.75)], 0.0).modifier,
        0.5
    );
    // MW Dest FLT: +4·v5·mw octaves; LFO: v5·mw into the amount; OSCMOD: +v5·mw to the modifier (clamped ±1).
    assert_eq!(
        control(&[(9, 0.5), (5, 0.5)], 0.5).log2_cutoff,
        base.log2_cutoff + 1.0
    );
    assert_eq!(
        control(&[(4, 0.5), (5, 1.0), (19, 0.5)], 0.5).lfo_pitch,
        3.0
    );
    assert_eq!(control(&[(4, 1.0), (5, 1.0), (7, 0.75)], 0.5).modifier, 1.0);
}

// ---- FX ----

/// A synth with `params` set and its block control up to date.
fn with(params: &[(usize, f32)]) -> MiniSynth {
    let mut synth = fresh();
    for &(index, value) in params {
        synth.set_parameter(index, value).unwrap();
    }
    synth.update_control();
    synth
}

#[test]
fn overdrive_follows_the_curve_and_the_gain_law() {
    // Gain 1, .67, .33, .30, .30 at v = 0 … 1.
    for (v, gain) in [
        (0.0, 1.0),
        (0.25, 0.67),
        (0.5, 0.33),
        (0.75, 0.30),
        (1.0, 0.30),
    ] {
        let c = with(&[(27, v)]).control;
        assert_eq!(c.drive, 20.0 * v * v);
        assert!((c.drive_gain - gain).abs() < 0.005, "{v}: {}", c.drive_gain);
    }
    // D = 0 passes the signal; D = 20 bends ±0.1 to ±(1 − 0.9·10⁻²).
    assert!((overdrive(0.3, 0.0) - 0.3).abs() < 1e-7);
    assert!((overdrive(0.1, 20.0) - 0.991).abs() < 1e-6);
    assert!((overdrive(-0.1, 20.0) + 0.991).abs() < 1e-6);
    let mut synth = with(&[(26, 1.0), (27, 1.0)]);
    let y = synth.distortion([0.1, -0.1]);
    let gain = synth.control.drive_gain;
    assert_eq!(
        y,
        [overdrive(0.1, 20.0) * gain, overdrive(-0.1, 20.0) * gain]
    );
}

#[test]
fn the_decimator_holds_every_h_th_frame() {
    // h = trunc(64·v28²): 0.5 → 16; 0.125 → 1, which is off.
    assert_eq!(with(&[(28, 0.125)]).control.hold, 1);
    let mut synth = with(&[(26, 1.0), (28, 0.5)]);
    assert_eq!(synth.control.hold, 16);
    let out: Vec<f32> = (0..48)
        .map(|n| synth.distortion([n as f32 * 0.01, 0.0])[0])
        .collect();
    for (n, y) in out.iter().enumerate() {
        assert_eq!(*y, overdrive((n / 16 * 16) as f32 * 0.01, 0.0), "{n}");
    }
}

#[test]
fn the_fx_run_distortion_delay_phaser_chorus() {
    // Every toggle on (any value ≠ 0 is on: 0.3 and 0.5 included): the chain is the four stages in order.
    let params = [
        (26, 0.3),
        (27, 0.4),
        (28, 0.3),
        (29, 0.5),
        (34, 0.3),
        (36, 0.8),
        (38, 0.5),
        (40, 0.7),
    ];
    let (mut chain, mut stages) = (with(&params), with(&params));
    for n in 0..2_000 {
        let x = [
            (n as f32 * 0.05).sin() * 0.5,
            (n as f32 * 0.031).cos() * 0.4,
        ];
        let by_stage = {
            let x = stages.distortion(x);
            let x = stages.delay(x);
            let x = stages.phaser(x);
            stages.chorus(x)
        };
        assert_eq!(chain.effects(x), by_stage, "{n}");
    }
}

#[test]
fn the_delay_time_is_a_quarter_beat_step_halved_to_1_5_s() {
    // 0.25·(trunc(3·v30) + 1) beats → T·60/BPM s, halved until ≤ 1.5 s; trunc(s·sr) frames.
    for (v, bpm, frames) in [
        (0.7, 120.0, 16_537),
        (1.0, 30.0, 44_100),
        (0.0, 20.0, 33_075),
        (1.0, 20.0, 66_150),
        (1.0, 10.0, 66_150),
    ] {
        let mut synth = with(&[(30, v)]);
        synth.set_tempo_bpm(bpm).unwrap();
        synth.update_control();
        assert_eq!(synth.control.delay_frames, frames, "{v} at {bpm}");
    }
}

#[test]
fn echoes_are_mono_and_fall_by_the_feedback() {
    // Mono in (L + R); echo n after n·T at fbⁿ (Flt 100 opens the loop low-pass); y·v33² on both sides; dry unchanged.
    let echoes = |flt: f32| {
        let mut synth = with(&[(29, 1.0), (30, 0.0), (31, 0.5), (32, flt), (33, 0.5)]);
        synth.set_tempo_bpm(600.0).unwrap();
        synth.update_control();
        let d = synth.control.delay_frames;
        let out: Vec<[f32; 2]> = (0..3 * d + 1)
            .map(|n| synth.delay(if n == 0 { [1.0, 0.5] } else { [0.0; 2] }))
            .collect();
        (d, out)
    };
    let (d, out) = echoes(1.0);
    assert_eq!(d, 1102);
    assert_eq!(out[0], [1.0, 0.5]);
    for k in 1..=3 {
        let y = 1.5 * 0.5f32.powi(k as i32) * 0.25;
        assert!(
            (out[k * d][0] - y).abs() < 1e-7 && out[k * d][1] == out[k * d][0],
            "{k}"
        );
    }
    assert!(
        out.iter()
            .enumerate()
            .all(|(n, x)| n % d == 0 || *x == [0.0; 2])
    );
    // Flt 0: the loop low-pass moves 0.01 of the way per sample.
    let (_, out) = echoes(0.0);
    assert!((out[d][0] - 1.5 * 0.5 * 0.01 * 0.25).abs() < 1e-8);
}

#[test]
fn toggling_the_delay_clears_its_buffer() {
    let mut synth = with(&[(29, 1.0), (31, 0.5)]);
    let d = synth.control.delay_frames;
    synth.delay([1.0, 1.0]);
    synth.set_parameter(29, 0.0).unwrap();
    synth.set_parameter(29, 0.3).unwrap();
    assert!((1..=d).all(|_| synth.delay([0.0; 2]) == [0.0; 2]));
}

/// The phaser's steady peak over 0.1 s for a sine at `w` rad/sample fed to both channels.
fn phaser_level(synth: &mut MiniSynth, w: f64) -> f32 {
    (0..8_820)
        .map(|n| synth.phaser([(w * n as f64).sin() as f32; 2])[0].abs())
        .skip(4_410)
        .fold(0.0, f32::max)
}

#[test]
fn the_phaser_cancels_where_each_allpass_turns_a_quarter_cycle() {
    // FREQ 50 %: s = 0.25, x = (50 + 19950·0.25)/sr, a = (1 − x)/(1 + x); Width 0, Fdbk 0, Mix 1: out = in + six
    // allpasses (−a + z⁻¹)/(1 − a·z⁻¹) (the form), so −90° per stage cancels and −60° per stage doubles.
    let x = 5_037.5 / f64::from(SR);
    let a = (1.0 - x) / (1.0 + x);
    let phase = |w: f64| -w - 2.0 * (a * w.sin()).atan2(1.0 - a * w.cos());
    let at = |target: f64| {
        let (mut lo, mut hi) = (0.0, std::f64::consts::PI);
        for _ in 0..60 {
            let mid = (lo + hi) / 2.0;
            if phase(mid) > target {
                lo = mid
            } else {
                hi = mid
            }
        }
        lo
    };
    let params = [(38, 1.0), (40, 0.0), (41, 0.0), (42, 1.0), (43, 0.5)];
    let notch = phaser_level(&mut with(&params), at(-std::f64::consts::FRAC_PI_2));
    assert!(notch < 0.01, "{notch}");
    let peak = phaser_level(&mut with(&params), at(-std::f64::consts::FRAC_PI_3));
    assert!((peak - 2.0).abs() < 0.01, "{peak}");
}

#[test]
fn phaser_controls_follow_the_spec_laws() {
    let c = with(&[(40, 0.5), (41, 0.1), (42, 0.0), (43, 0.0)]).control;
    assert_eq!(c.phaser_width, 0.125);
    assert!((c.phaser_feedback - 0.98 * 0.1f32.cbrt()).abs() < 1e-7);
    assert_eq!((c.phaser_mix, c.phaser_target), (1e-5, 1e-5));
    // s moves toward max(v43², 1e-5) at 0.001 per sample.
    let mut synth = with(&[(38, 1.0), (43, 1.0)]);
    synth.phaser([0.0; 2]);
    assert!((synth.phaser_sweep - (0.25 + 0.75 * 0.001)).abs() < 1e-7);
    // φ from Speed (default 0.1122 → 3/8 beat), locked to the song position like the LFO.
    let cos_at = |beat: f64| {
        let mut synth = fresh();
        synth.set_song_beat(beat);
        synth.update_control();
        synth.control.phaser_cos
    };
    assert!((cos_at(1.5) - 1.0).abs() < 1e-6);
    assert!((cos_at(0.1875) + 1.0).abs() < 1e-6);
}

#[test]
fn the_chorus_taps_n_minus_m2dn_on_the_left_and_crossfades_on_the_right() {
    // N = trunc(0.01·sr) = 441; d = 0.36·v36 + 0.04; m = 0.5 + d·sin 2πφ; φ starts at 0.62 (L) and 0.1 (R) and
    // runs at 4·v35² + 0.2 Hz (L), + 0.25 Hz (R). An impulse into the mono buffer (L + R = 1).
    let mut synth = with(&[(34, 1.0), (35, 0.0), (36, 0.0), (37, 1.0)]);
    assert_eq!(
        (synth.control.chorus_frames, synth.control.chorus_depth),
        (441, 0.04)
    );
    let out: Vec<[f32; 2]> = (0..500)
        .map(|n| synth.chorus(if n == 0 { [0.75, 0.25] } else { [0.0; 2] }))
        .collect();
    let m = |phase: f32, hz: f32, n: usize| 0.5 + 0.04 * (TAU * (phase + hz * n as f32 / SR)).sin();
    // R wet = (1 − m)·x[n] + m·x[n − N + 1]: the impulse at n = 0 and again at n = 440, nowhere else.
    assert!((out[0][1] - (0.25 + 1.0 - m(0.1, 0.25, 0))).abs() < 1e-6);
    assert!((out[440][1] - m(0.1, 0.25, 440)).abs() < 1e-5);
    assert!(
        out.iter()
            .enumerate()
            .all(|(n, x)| n == 0 || n == 440 || x[1] == 0.0)
    );
    // L wet: the impulse at a delay of N·(1 − m²·d), split between two frames by linear interpolation.
    let delay = 441.0 * (1.0 - m(0.62, 0.2, 437).powi(2) * 0.04);
    let centroid = (1..500).map(|n| n as f32 * out[n][0]).sum::<f32>();
    let total = (1..500).map(|n| out[n][0]).sum::<f32>();
    // (The two frames read slightly different delays: the LFO moves 3·10⁻⁵ of a frame between them.)
    assert!((total - 1.0).abs() < 1e-4, "{total}");
    assert!(
        (centroid / total - delay).abs() < 0.01,
        "{centroid} vs {delay}"
    );
    // At Depth 100 the left tap spans N·(1 − 0.9²·0.4) … N·(1 − 0.1²·0.4): 6.8 … 10 ms.
    assert_eq!(with(&[(36, 1.0)]).control.chorus_depth, 0.4);
}

#[test]
fn switching_the_chorus_on_restarts_its_phases() {
    let mut synth = with(&[(34, 1.0)]);
    for _ in 0..1000 {
        synth.chorus([0.0; 2]);
    }
    assert_ne!(synth.chorus_phase, [0.62, 0.1]);
    synth.set_parameter(34, 0.0).unwrap();
    synth.set_parameter(34, 0.5).unwrap();
    assert_eq!(synth.chorus_phase, [0.62, 0.1]);
}

// ---- tail ----

/// Plays a note for 1000 frames, releases it, and renders one frame at a time until the voice frees.
fn play_and_free(synth: &mut MiniSynth) {
    synth.note_on(60, 1.0);
    render(synth, 1000);
    synth.note_off(60);
    while synth.voices[4].busy {
        render(synth, 1);
    }
}

/// Frames rendered one at a time until the FX stop, up to `limit`.
fn frames_until_silent(synth: &mut MiniSynth, limit: usize) -> usize {
    (0..limit)
        .find(|_| {
            render(synth, 1);
            !synth.running
        })
        .unwrap_or(limit)
}

#[test]
fn the_fx_run_half_a_second_after_the_last_voice_frees_then_freeze() {
    let mut synth = with(&[(25, 0.0), (38, 1.0), (34, 1.0)]);
    play_and_free(&mut synth);
    let frames = frames_until_silent(&mut synth, 30_000);
    assert!(frames.abs_diff(22_050) <= 32, "{frames}");
    // Stopped: no output, and the FX states stay where they were.
    let (phaser, chorus, beat) = (synth.phaser_out, synth.chorus_pos, synth.beat);
    let (left, right) = render(&mut synth, 5_000);
    assert!(left.iter().chain(&right).all(|x| *x == 0.0));
    assert_eq!((synth.phaser_out, synth.chorus_pos), (phaser, chorus));
    // The song position still runs on.
    assert!(synth.beat > beat);
    // The next note runs them again.
    synth.note_on(60, 1.0);
    assert!(synth.running);
}

#[test]
fn a_ringing_delay_keeps_the_fx_running_until_it_falls_below_1e_5() {
    // 1/4 beat at 600 BPM, Fdbk 0.8: about 46 echoes of 25 ms to fall below 1e-5, beyond the 0.5 s tail.
    let mut synth = with(&[(25, 0.0), (29, 1.0), (30, 0.0), (31, 0.8)]);
    synth.set_tempo_bpm(600.0).unwrap();
    play_and_free(&mut synth);
    let frames = frames_until_silent(&mut synth, 200_000);
    assert!(frames > 22_050 + 4_410 && frames < 200_000, "{frames}");
    assert!(synth.delay_peak <= 1e-5 || !synth.running);
}

// ---- subnormals ----

/// Zero or a normal float: never a subnormal.
fn not_subnormal(x: f32) -> bool {
    x == 0.0 || x.abs() >= f32::MIN_POSITIVE
}

#[test]
fn a_chord_decaying_to_sustain_zero_leaves_no_subnormal_state() {
    // AMP DEC 0.05 (τ ≈ 22 samples) toward SUS 0: A falls through the subnormals within 0.1 s and, unflushed, sticks
    // at about 10⁻⁴⁴, where (0 − e)·k rounds to 0. States below 10⁻²⁰ are zeroed once per block.
    let mut synth = with(&[(23, 0.05), (24, 0.0), (14, 0.05), (15, 0.0)]);
    for key in [60, 64, 67] {
        synth.note_on(key, 1.0);
    }
    render(&mut synth, 32 * 4_134);
    let mut checked = 0;
    for voice in synth.voices.iter().filter(|voice| voice.busy) {
        let states = [voice.amp.level, voice.filter_env.level, voice.dc];
        for x in states.into_iter().chain(voice.ladder.state) {
            assert!(not_subnormal(x), "{x:e}");
            checked += 1;
        }
    }
    assert_eq!(checked, 3 * 7);
}
