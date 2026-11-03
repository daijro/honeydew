//! SuperSaw (type 23) rendered offline against its DSP laws.

use honeydew_audio::ProcessContext;
use honeydew_audio::instrument::{Instrument, InstrumentModule};
use honeydew_audio::supersaw::{DEFAULTS, PARAM_COUNT, SuperSaw};

const SR: f32 = 48_000.0;
/// The 0.99951 increment factor (≈ 0.85 cent flat).
const FLAT: f32 = 0.99951;
/// Stopped at 120 BPM.
const STOPPED: ProcessContext = ProcessContext {
    sample_rate: SR,
    tempo_bpm: 120.0,
    start_beat: 0.0,
    playing: false,
};

fn synth(params: &[(usize, f32)]) -> SuperSaw {
    let mut synth = SuperSaw::new(SR);
    synth.set_tempo_bpm(120.0);
    for &(index, value) in params {
        synth.set_parameter(index, value);
    }
    synth
}

fn render(synth: &mut SuperSaw, frames: usize) -> (Vec<f32>, Vec<f32>) {
    let (mut left, mut right) = (vec![0.0; frames], vec![0.0; frames]);
    synth.process(&mut left, &mut right, STOPPED);
    (left, right)
}

fn render_in(synth: &mut SuperSaw, frames: usize, context: ProcessContext) -> (Vec<f32>, Vec<f32>) {
    let (mut left, mut right) = (vec![0.0; frames], vec![0.0; frames]);
    synth.process(&mut left, &mut right, context);
    (left, right)
}

fn rms(x: &[f32]) -> f32 {
    (x.iter().map(|s| s * s).sum::<f32>() / x.len() as f32).sqrt()
}

fn peak(x: &[f32]) -> f32 {
    x.iter().fold(0.0, |p, s| p.max(s.abs()))
}

/// Hann-windowed DFT power at `hz`.
fn power_at(x: &[f32], hz: f32) -> f64 {
    let n = x.len() as f64;
    let (mut re, mut im) = (0.0, 0.0);
    for (i, &s) in x.iter().enumerate() {
        let w = 0.5 - 0.5 * (std::f64::consts::TAU * i as f64 / n).cos();
        let a = std::f64::consts::TAU * f64::from(hz) * i as f64 / f64::from(SR);
        re += w * f64::from(s) * a.cos();
        im += w * f64::from(s) * a.sin();
    }
    (re * re + im * im) / (n * n)
}

/// The frequency from the falling ramp's negative-going zero crossings (one per cycle), interpolated.
fn crossing_hz(x: &[f32]) -> f32 {
    let mut times = Vec::new();
    for i in 1..x.len() {
        if x[i - 1] > 0.0 && x[i] <= 0.0 {
            times.push((i - 1) as f64 + f64::from(x[i - 1] / (x[i - 1] - x[i])));
        }
    }
    let span = times.last().unwrap() - times.first().unwrap();
    ((times.len() - 1) as f64 / span * f64::from(SR)) as f32
}

fn hz(note: f32) -> f32 {
    440.0 * ((note - 69.0) / 12.0).exp2()
}

/// The note number the crossings give, the 0.99951 factor taken out.
fn note_of(x: &[f32]) -> f32 {
    69.0 + 12.0 * (crossing_hz(x) / (440.0 * FLAT)).log2()
}

#[test]
fn defaults_cover_every_param() {
    assert_eq!(DEFAULTS.len(), PARAM_COUNT);
    assert_eq!(PARAM_COUNT, 60);
    assert_eq!(
        (DEFAULTS[51], DEFAULTS[37], DEFAULTS[54]),
        (0.5, 0.1, 0.775)
    );
}

#[test]
fn pitch_is_a440_times_the_increment_factor() {
    let mut s = synth(&[]);
    s.note_on(69, 1.0, 0.5);
    let (l, r) = render(&mut s, 96_000);
    let f = crossing_hz(&l[4_800..]);
    assert!((f - 440.0 * FLAT).abs() < 0.02, "{f}");
    assert_eq!(l, r, "a single voice is a mono sum");
}

#[test]
fn transpose_and_pitch_bend_shift_the_pitch() {
    // Transpose trunc(24·v36) − 12: v = 1 → +12. Bend w·B: w = 2·1 − 1, B = int(11·0.1 + 1) = 2 st.
    let mut s = synth(&[(36, 1.0)]);
    s.set_controller(4, 1.0);
    s.note_on(57, 1.0, 0.5);
    let (l, _) = render(&mut s, 48_000);
    let f = crossing_hz(&l[4_800..]);
    assert!((f - hz(71.0) * FLAT).abs() < 0.05, "{f}");
}

#[test]
fn note_zero_is_silent() {
    let mut s = synth(&[]);
    s.note_on(0, 1.0, 0.5);
    assert_eq!(peak(&render(&mut s, 4_800).0), 0.0);
}

#[test]
fn detune_zero_equals_one_voice() {
    // Retrig on, Unison Phase 0: every unison voice starts at phase 0 and reads the same dither.
    let one = render(&mut played(&[(8, 1.0), (5, 0.0)], 60), 9_600).0;
    let twelve = render(&mut played(&[(8, 1.0), (5, 0.0), (4, 1.0)], 60), 9_600).0;
    let scale = 12.0 * 10f32.powf(-12.0 / 20.0) / 10f32.powf(-1.0 / 20.0);
    let error = one
        .iter()
        .zip(&twelve)
        .map(|(a, b)| (a * scale - b).powi(2))
        .sum::<f32>();
    assert!(rms(&one) > 0.1);
    // The y1 soft clip (1e-5·y1²) bends the 12× louder filter input by about 0.1 %; without it the two match to
    // rounding.
    let relative = error / one.iter().map(|a| (a * scale).powi(2)).sum::<f32>();
    assert!(relative < 1e-4 && relative > 1e-9, "{relative}");
}

fn played(params: &[(usize, f32)], note: u8) -> SuperSaw {
    let mut s = synth(params);
    s.note_on(note, 1.0, 0.5);
    s
}

#[test]
fn unison_count_spreads_the_spectrum_to_the_outer_detune() {
    // Detune v5 = 1 → d = 1 st; curve 0.5 → linear: three voices at −1, 0, +1 st.
    let outer = [hz(56.0) * FLAT, hz(58.0) * FLAT];
    let single = render(&mut played(&[(5, 1.0)], 57), 48_000).0;
    let three = render(&mut played(&[(5, 1.0), (4, 2.5 / 11.0)], 57), 48_000).0;
    for f in outer {
        let ratio = power_at(&three, f) / power_at(&single, f);
        assert!(ratio > 100.0, "{f} Hz: {ratio}");
    }
}

#[test]
fn unison_pan_puts_the_outer_voices_hard_left_and_right() {
    // N = 2, Unison Pan 1: voice 0 (−d) L = 1, R = 0; voice 1 (+d) L = 0, R = 1.
    let (l, r) = render(
        &mut played(&[(4, 1.5 / 11.0), (5, 1.0), (6, 1.0)], 57),
        48_000,
    );
    let (low, high) = (hz(56.0) * FLAT, hz(58.0) * FLAT);
    assert!(power_at(&l, low) > 1000.0 * power_at(&l, high));
    assert!(power_at(&r, high) > 1000.0 * power_at(&r, low));
}

#[test]
fn pulse_mode_width_zero_is_silent_and_one_sounds() {
    let silent = render(&mut played(&[(2, 1.0), (1, 0.0)], 69), 4_800).0;
    assert_eq!(peak(&silent), 0.0);
    let square = render(&mut played(&[(2, 1.0), (1, 1.0)], 69), 4_800).0;
    assert!(rms(&square) > 0.3);
}

#[test]
fn sub_osc_is_a_saw_an_octave_down() {
    // Pulse width 0 silences the main saw, leaving the sub alone.
    let (l, _) = render(&mut played(&[(2, 1.0), (1, 0.0), (3, 1.0)], 69), 96_000);
    let f = crossing_hz(&l[4_800..]);
    assert!((f - 220.0 * FLAT).abs() < 0.02, "{f}");
    let half = render(&mut played(&[(2, 1.0), (1, 0.0), (3, 0.5)], 69), 48_000).0;
    let ratio = rms(&half[4_800..]) / rms(&l[4_800..48_000]);
    assert!((ratio - 0.25).abs() < 0.01, "sub level v3²: {ratio}");
}

#[test]
fn supersync_ratio_two_doubles_the_pulse_train() {
    // S = v11 = 0.2 → R = 2, T = 0.5, W = 0.5·v12 = 0.5: a square at 2f.
    let f = hz(57.0) * FLAT;
    let sync = render(&mut played(&[(10, 1.0), (11, 0.2)], 57), 48_000).0;
    assert!(power_at(&sync, 2.0 * f) > 1000.0 * power_at(&sync, f));
    let plain = render(&mut played(&[(10, 1.0), (11, 0.0)], 57), 48_000).0;
    assert!(power_at(&plain, f) > 1000.0 * power_at(&plain, 2.0 * f));
}

#[test]
fn flip_negates_the_sync_eg_amount_below_the_previous_note() {
    // EG Amount v17 = 1 → a = 1; the sync EG sustains at 1 → S = 1, R = 6. Flipped: S = 0, R = 1.
    let f = hz(57.0) * FLAT;
    for (flip, high) in [(0.0, true), (1.0, false)] {
        let mut s = synth(&[(10, 1.0), (17, 1.0), (18, flip)]);
        s.note_on(69, 1.0, 0.5);
        render(&mut s, 2_400);
        s.note_off(69);
        render(&mut s, 4_800);
        s.note_on(57, 1.0, 0.5);
        let out = render(&mut s, 48_000).0;
        let (at_f, at_6f) = (power_at(&out, f), power_at(&out, 6.0 * f));
        assert_eq!(at_6f > 100.0 * at_f, high, "flip {flip}: {at_f} {at_6f}");
    }
}

#[test]
fn low_cutoff_darkens_and_vel_trk_opens() {
    let bright = rms(&render(&mut played(&[], 45), 24_000).0[2_400..]);
    let dark = rms(&render(&mut played(&[(19, 0.2)], 45), 24_000).0[2_400..]);
    assert!(dark < 0.3 * bright, "{dark} {bright}");
    let opened = rms(&render(&mut played(&[(19, 0.2), (27, 1.0)], 45), 24_000).0[2_400..]);
    assert!(opened > 0.9 * bright, "{opened}");
}

/// The window RMS relative to the sustained level, at `at` seconds.
fn envelope_at(out: &[f32], at: f32, full: f32) -> f32 {
    let centre = (at * SR) as usize;
    rms(&out[centre - 240..centre + 240]) / full
}

#[test]
fn amp_attack_follows_t_of_point_nine_v_and_sounds_as_env_squared() {
    // A = T(0.9·0.6) = T(0.54) = 0.364 s. Half-way the env is 0.5, heard as 0.25.
    let attack = 0.364_1;
    let mut s = synth(&[(28, 0.6)]);
    s.note_on(57, 1.0, 0.5);
    let out = render(&mut s, 48_000).0;
    let full = rms(&out[38_400..]);
    let half = envelope_at(&out, attack / 2.0, full);
    assert!((half - 0.25).abs() < 0.02, "{half}");
}

#[test]
fn sustain_sounds_as_its_square_and_release_ends_the_voice() {
    // D = T(0.3), S = 0.5 → 0.25; R = T(0.85·0.5 + 0.05) = T(0.475) = 0.27 s.
    let reference = rms(&render(&mut played(&[], 57), 48_000).0[24_000..]);
    let mut s = played(&[(29, 0.3), (30, 0.5), (31, 0.5)], 57);
    let held = render(&mut s, 48_000).0;
    let level = rms(&held[24_000..]) / reference;
    assert!((level - 0.25).abs() < 0.01, "{level}");
    s.note_off(57);
    let released = render(&mut s, 24_000).0;
    assert!(rms(&released[..1_200]) > 0.1 * reference);
    assert_eq!(
        peak(&released[14_400..]),
        0.0,
        "idle after T(0.475) = 0.27 s"
    );
}

#[test]
fn the_voice_limit_steals_the_oldest() {
    // int(7·0.15) + 1 = 2 voices: the third note steals the first.
    let mut s = synth(&[(51, 0.15)]);
    for note in [57, 64, 69] {
        s.note_on(note, 1.0, 0.5);
    }
    let out = render(&mut s, 48_000).0;
    let (a, e, a2) = (hz(57.0) * FLAT, hz(64.0) * FLAT, hz(69.0) * FLAT);
    assert!(power_at(&out, e) > 1000.0 * power_at(&out, a));
    assert!(power_at(&out, a2) > 1000.0 * power_at(&out, a));
}

#[test]
fn mono_falls_back_to_the_highest_held_key() {
    let mut s = synth(&[(50, 1.0)]);
    s.note_on(57, 1.0, 0.5);
    s.note_on(64, 1.0, 0.5);
    s.note_on(60, 1.0, 0.5);
    let last = render(&mut s, 48_000).0;
    assert!((crossing_hz(&last[4_800..]) - hz(60.0) * FLAT).abs() < 0.05);
    s.note_off(60);
    let fallback = render(&mut s, 48_000).0;
    assert!((crossing_hz(&fallback[4_800..]) - hz(64.0) * FLAT).abs() < 0.05);
    assert!(
        rms(&fallback[..2_400]) > 0.9 * rms(&last[24_000..]),
        "no retrigger"
    );
}

#[test]
fn constant_glide_takes_the_slide_time() {
    // s = (v35/2)² = 0.25 s, Constant on: 57 → 69 is half-way (63) at 0.125 s.
    let mut s = synth(&[(35, 1.0)]);
    s.note_on(57, 1.0, 0.5);
    render(&mut s, 2_400);
    s.note_off(57);
    s.note_on(69, 1.0, 0.5);
    let out = render(&mut s, 48_000).0;
    let mid = crossing_hz(&out[5_400..6_600]);
    assert!((mid - hz(63.0)).abs() < 0.03 * hz(63.0), "{mid}");
    assert!((crossing_hz(&out[14_400..]) - hz(69.0) * FLAT).abs() < 0.05);
}

#[test]
fn legato_glides_only_while_another_key_is_held() {
    let mut s = synth(&[(35, 1.0), (34, 1.0)]);
    s.note_on(57, 1.0, 0.5);
    render(&mut s, 2_400);
    s.note_off(57);
    s.note_on(69, 1.0, 0.5);
    let out = render(&mut s, 4_800).0;
    assert!(
        (crossing_hz(&out[1_200..]) - hz(69.0) * FLAT).abs() < 1.0,
        "no glide"
    );
}

#[test]
fn lfo_square_on_level_gates_at_thirty_hertz() {
    // Rate v40 = 1 → 1/0.033333 = 30 Hz; Square 1 for φ < 0.5; A = (2·0 − 1)·1 = −1 → Level 1 − 1 = 0.
    let mut s = played(
        &[(39, 2.0 / 3.0), (40, 1.0), (44, 0.0), (45, 6.0 / 7.0)],
        69,
    );
    let out = render(&mut s, 4_800).0;
    let full = rms(&render(&mut played(&[], 69), 4_800).0[1_000..]);
    // Cycle 33.3 ms: φ < 0.5 for 0…16.7 ms (silent, after the 2 ms gain smoothing), then loud.
    assert!(rms(&out[300..700]) < 0.05 * full);
    assert!(rms(&out[1_100..1_500]) > 0.9 * full);
}

#[test]
fn tmp_sync_lfo_locks_to_the_song_position() {
    // Beat int(9·0.45) = 4 → 1 beat. Playing from 0.25: φ 0.25…0.45 → square 1 → level 0; from 0.75: square 0.
    let params = [
        (39, 2.0 / 3.0),
        (41, 1.0),
        (42, 0.45),
        (44, 0.0),
        (45, 6.0 / 7.0),
    ];
    for (start_beat, loud) in [(0.25, false), (0.75, true)] {
        let mut s = synth(&params);
        s.note_on(69, 1.0, 0.5);
        let (mut l, mut r) = (vec![0.0; 4_800], vec![0.0; 4_800]);
        let context = ProcessContext {
            start_beat,
            playing: true,
            ..STOPPED
        };
        s.process(&mut l, &mut r, context);
        assert_eq!(rms(&l[2_400..]) > 0.1, loud, "beat {start_beat}");
    }
}

#[test]
fn mod_wheel_reaches_its_destination() {
    // Amount 2·0 − 1 = −1 at dest int(8·0.65) = 5 (Level): wheel 1 → level 0.
    let mut s = synth(&[(58, 0.0), (59, 0.65)]);
    s.note_on(69, 1.0, 0.5);
    assert!(rms(&render(&mut s, 4_800).0[2_400..]) > 0.1);
    s.set_controller(8, 1.0);
    assert!(rms(&render(&mut s, 4_800).0[2_400..]) < 1e-3);
}

#[test]
fn arp_steps_on_the_quarter_beat_grid_at_120_bpm() {
    // Rate int(5·0.2) = 1 → 1/4 beat = 0.125 s = 6000 frames; Up over 2 octaves: 57, 69, 57, 69.
    // Decay T(0.1) = 30 ms to sustain 0 makes each step a blip.
    let mut s = synth(&[(49, 0.2), (29, 0.1), (30, 0.0)]);
    s.note_on(57, 1.0, 0.5);
    let out = render(&mut s, 30_000).0;
    let mut onsets = Vec::new();
    let mut quiet = true;
    for (i, x) in out.iter().enumerate() {
        if quiet && x.abs() > 1e-3 {
            onsets.push(i);
        }
        quiet = if x.abs() > 1e-3 {
            false
        } else {
            quiet
                || out[i.saturating_sub(480)..=i]
                    .iter()
                    .all(|y| y.abs() <= 1e-3)
        };
    }
    assert_eq!(onsets.len(), 5, "{onsets:?}");
    for (k, onset) in onsets.iter().enumerate() {
        assert!(onset.abs_diff(k * 6_000) <= 48, "{onsets:?}");
    }
    let step = |k: usize| &out[k * 6_000 + 24..k * 6_000 + 1_200];
    assert!(power_at(step(1), hz(69.0)) > 10.0 * power_at(step(1), hz(57.0)));
    // A saw at 57 carries 69 as its second harmonic, so compare 57's fundamental across steps.
    assert!(power_at(step(2), hz(57.0)) > 10.0 * power_at(step(1), hz(57.0)));
}

#[test]
fn releasing_every_key_stops_the_arp() {
    let mut s = synth(&[(49, 0.2)]);
    s.note_on(57, 1.0, 0.5);
    render(&mut s, 3_000);
    s.note_off(57);
    let out = render(&mut s, 24_000).0;
    assert_eq!(peak(&out[2_400..]), 0.0);
}

#[test]
fn gain_is_minus_18_plus_18_v_db() {
    let full = render(&mut played(&[], 57), 9_600).0;
    let low = render(&mut played(&[(53, 0.0)], 57), 9_600).0;
    let ratio = 10f32.powf(-0.9);
    for (a, b) in full.iter().zip(&low) {
        assert!((a * ratio - b).abs() <= 1e-6, "{a} {b}");
    }
}

#[test]
fn distortion_is_bounded_by_tanh() {
    let out = render(&mut played(&[(4, 1.0), (5, 0.0), (52, 1.0)], 45), 9_600).0;
    assert!(peak(&out) <= 1.0 + 1e-6);
    assert!(rms(&out[4_800..]) > 0.5, "driven hard toward ±1");
}

#[test]
fn delay_first_echo_is_fb_times_mix_and_the_next_fb_squared() {
    // Time int(4·0) = 0 → 1/4 beat = 6000 frames at 120 BPM; fb = 0.99·0.5; m = 0.8²; filter centre (c = 1).
    let (fb, m) = (0.99 * 0.5, 0.8f32 * 0.8);
    let mut s = synth(&[(54, 0.0), (55, 0.5), (56, 0.8), (29, 0.05), (30, 0.0)]);
    s.note_on(57, 1.0, 0.5);
    let out = render(&mut s, 18_000).0;
    let dry = &out[..1_500];
    assert!(rms(dry) > 0.05);
    for i in 0..1_500 {
        assert!((out[6_000 + i] - fb * m * dry[i]).abs() < 1e-5, "{i}");
        assert!((out[12_000 + i] - fb * fb * m * dry[i]).abs() < 1e-5, "{i}");
    }
    // Delay Filter at either end mutes the echo (LP c = 0; HP c = 2 → α = 1, tap = d − s = 0).
    for filter in [0.0, 1.0] {
        let mut s = synth(&[(55, 0.5), (56, 0.8), (29, 0.05), (30, 0.0), (57, filter)]);
        s.note_on(57, 1.0, 0.5);
        let out = render(&mut s, 9_000).0;
        assert!(peak(&out[6_000..7_500]) < 1e-6);
    }
}

#[test]
fn output_pan_and_note_pan_follow_the_law() {
    let (l, r) = render(&mut played(&[(33, 0.0)], 57), 4_800);
    assert!(rms(&l[2_400..]) > 0.1 && peak(&r[2_400..]) < 1e-4);
    let mut s = synth(&[]);
    s.note_on(57, 1.0, 1.0);
    let (l, r) = render(&mut s, 4_800);
    assert!(
        rms(&r[2_400..]) > 0.1 && peak(&l[2_400..]) < 1e-4,
        "note pan 1 → P = 1"
    );
}

#[test]
fn level_is_squared_and_velocity_linear() {
    let full = rms(&render(&mut played(&[], 57), 9_600).0[4_800..]);
    let level = rms(&render(&mut played(&[(32, 0.5)], 57), 9_600).0[4_800..]);
    assert!((level / full - 0.25).abs() < 1e-3);
    let mut s = synth(&[]);
    s.note_on(57, 0.5, 0.5);
    let soft = rms(&render(&mut s, 9_600).0[4_800..]);
    assert!((soft / full - 0.5).abs() < 1e-3);
}

#[test]
fn block_size_does_not_change_the_output() {
    let params = [
        (4, 0.5),
        (6, 0.7),
        (40, 0.8),
        (44, 0.9),
        (45, 0.0),
        (56, 0.5),
        (49, 0.6),
    ];
    let mut whole = played(&params, 60);
    whole.note_on(64, 0.8, 0.3);
    let (wl, wr) = render(&mut whole, 20_000);
    let mut parts = played(&params, 60);
    parts.note_on(64, 0.8, 0.3);
    let (mut pl, mut pr) = (vec![0.0; 20_000], vec![0.0; 20_000]);
    let mut start = 0;
    for size in [1, 7, 23, 24, 25, 333, 1023, 4096].iter().cycle() {
        let end = (start + size).min(20_000);
        parts.process(&mut pl[start..end], &mut pr[start..end], STOPPED);
        start = end;
        if start == 20_000 {
            break;
        }
    }
    assert!(rms(&wl) > 0.01);
    assert_eq!((wl, wr), (pl, pr));
}

#[test]
fn reset_silences_and_all_notes_off_releases() {
    let mut s = played(&[(31, 0.5), (56, 1.0)], 57);
    render(&mut s, 4_800);
    s.all_notes_off();
    assert!(rms(&render(&mut s, 2_400).0) > 0.01, "release still sounds");
    s.reset();
    assert_eq!(peak(&render(&mut s, 48_000).0), 0.0);
}

/// Release-mode cost: `cargo test --release -p honeydew-audio --test supersaw -- --ignored --nocapture`.
#[test]
#[ignore]
fn render_cost_per_voice_second() {
    let start = std::time::Instant::now();
    drop(SuperSaw::new(SR));
    println!(
        "first construction (wavetable set): {:.1} ms",
        start.elapsed().as_secs_f64() * 1e3
    );
    for (name, params) in [
        ("1 osc", vec![]),
        (
            "12 unison + pulse + sub + LFO",
            vec![
                (4, 1.0),
                (2, 1.0),
                (1, 0.6),
                (3, 0.5),
                (6, 1.0),
                (40, 0.8),
                (44, 0.9),
            ],
        ),
        ("supersync", vec![(10, 1.0), (11, 1.0)]),
    ] {
        let mut s = synth(&params);
        s.set_parameter(51, 1.0);
        for note in [48, 52, 55, 59, 62, 65, 69, 72] {
            s.note_on(note, 1.0, 0.5);
        }
        let seconds = 10;
        let (mut l, mut r) = (vec![0.0; 512], vec![0.0; 512]);
        let start = std::time::Instant::now();
        for _ in 0..(seconds * SR as usize / 512) {
            s.process(&mut l, &mut r, STOPPED);
        }
        let per = start.elapsed().as_secs_f64() / (8 * seconds) as f64;
        println!(
            "{name}: {:.2} ms per voice-second ({:.2} % of one core per voice)",
            per * 1e3,
            per * 100.0
        );
    }
}

// ---- Regression cases ----

#[test]
fn arp_octaves_above_key_127_play_without_panicking() {
    // Up over 2 octaves from key 120: the second step plays 132.
    let mut s = synth(&[(49, 0.2)]);
    s.note_on(120, 1.0, 0.5);
    let out = render(&mut s, 24_000).0;
    assert!(rms(&out[6_000..12_000]) > 0.01);
}

#[test]
fn supersync_after_unison_pan_is_a_mono_sum() {
    let mut s = played(&[(4, 1.0), (6, 1.0), (5, 0.5)], 57);
    render(&mut s, 4_800);
    s.set_parameter(10, 1.0);
    let (l, r) = render(&mut s, 9_600);
    assert!(rms(&l) > 0.1);
    assert_eq!(l, r);
}

#[test]
fn enable_off_edge_resets_supersaw() {
    // SuperSaw cuts its voices at once.
    let mut module = InstrumentModule::new(Box::new(played(&[(56, 1.0)], 57)), 512);
    let (mut l, mut r) = (vec![0.0; 4_800], vec![0.0; 4_800]);
    module.process(&mut l, &mut r, STOPPED);
    assert!(rms(&l) > 0.1);
    module.set_parameter(0, 0.0);
    module.set_parameter(0, 1.0);
    let (mut l, mut r) = (vec![0.0; 4_800], vec![0.0; 4_800]);
    module.process(&mut l, &mut r, STOPPED);
    assert_eq!(peak(&l), 0.0);
}

// ---- Laws at interior values (I3) ----

#[test]
fn transpose_truncates() {
    // trunc(24·0.53) − 12 = 0.
    let mut s = played(&[(36, 0.53)], 69);
    let f = crossing_hz(&render(&mut s, 48_000).0[4_800..]);
    assert!((f - 440.0 * FLAT).abs() < 0.02, "{f}");
}

#[test]
fn lookup_dither_spans_two_table_entries() {
    // Note 1, Transpose −12, bend −12 st: f = 440·2^(−92/12) ≈ 2.2 Hz, so harmonic 512 sits at 1.1 kHz in the
    // 1024-harmonic table. Averaged over time, harmonic h keeps sinc(h/2048) for the held entry and sinc(2h/2048) for
    // the U(0, 2 entries) dither: 512 against 64 reads 0.574 with the ladder's roll-off; no dither reads 0.896 and a
    // 23-bit dither 0.
    let mut s = synth(&[(36, 0.0), (37, 1.0)]);
    s.set_controller(4, 0.0);
    s.note_on(1, 1.0, 0.5);
    let out = render(&mut s, 192_000).0;
    let f = f64::from(440.0 * (-92.0f32 / 12.0).exp2() * FLAT);
    let amplitude = |h: f64| power_at(&out, (h * f) as f32).sqrt() * h;
    let sinc = |x: f64| (std::f64::consts::PI * x).sin() / (std::f64::consts::PI * x);
    let ladder = |hz: f64| 1.0 / (1.0 + (std::f64::consts::TAU * hz / 125_000.0).powi(2)).powi(2);
    let expect = |h: f64| sinc(h / 2048.0) * sinc(2.0 * h / 2048.0) * ladder(h * f);
    let ratio = amplitude(512.0) / amplitude(64.0);
    let law = expect(512.0) / expect(64.0);
    assert!((ratio / law - 1.0).abs() < 0.03, "{ratio} vs {law}");
}

#[test]
fn unison_count_follows_the_menu() {
    // N = int(11·v4 + 1); detune 0 and Retrig line the voices up, so the level is N·10^(−N/20).
    let single = render(&mut played(&[(8, 1.0), (5, 0.0)], 60), 9_600).0;
    // 1.6/11 sits between steps: int(2.6) = 2 voices.
    for (v4, n) in [
        (1.0 / 11.0, 2.0),
        (1.6 / 11.0, 2.0),
        (2.0 / 11.0, 3.0),
        (5.0 / 11.0, 6.0),
        (1.0, 12.0),
    ] {
        let params = [(8, 1.0), (5, 0.0), (4, v4)];
        let out = render(&mut played(&params, 60), 9_600).0;
        let ratio = rms(&out[4_800..]) / rms(&single[4_800..]);
        let law = n * 10f32.powf(-n / 20.0) / 10f32.powf(-1.0 / 20.0);
        assert!((ratio / law - 1.0).abs() < 2e-3, "N {n}: {ratio} vs {law}");
    }
}

#[test]
fn detune_curve_bends_the_inner_voices() {
    let at = |st: f32| 880.0 * FLAT * (st / 12.0).exp2();
    // c = +1 → e = 5: of four voices, the third sits at 2·(2/3)⁵ − 1 st (e = 4 would put it at 2·(2/3)⁴ − 1).
    let up = render(
        &mut played(&[(4, 3.0 / 11.0), (5, 1.0), (7, 1.0)], 81),
        48_000,
    )
    .0;
    let x: f32 = 2.0 / 3.0;
    let (right, wrong) = (at(2.0 * x.powi(5) - 1.0), at(2.0 * x.powi(4) - 1.0));
    assert!(power_at(&up, right) > 100.0 * power_at(&up, wrong));
    // c = −1 → e = 0.1: of three voices, the middle sits at 2·0.5^0.1 − 1 st.
    let down = render(
        &mut played(&[(4, 2.0 / 11.0), (5, 1.0), (7, 0.0)], 81),
        48_000,
    )
    .0;
    let (right, wrong) = (
        at(2.0 * 0.5f32.powf(0.1) - 1.0),
        at(2.0 * 0.5f32.powf(0.5) - 1.0),
    );
    assert!(power_at(&down, right) > 100.0 * power_at(&down, wrong));
}

#[test]
fn pulse_width_half_offsets_the_second_saw_by_a_quarter_cycle() {
    // saw(φ) − saw(φ + 0.5·0.5): a 25 % pulse, so harmonic 4 vanishes.
    let f = hz(45.0) * FLAT;
    let out = render(&mut played(&[(2, 1.0), (1, 0.5)], 45), 48_000).0;
    assert!(power_at(&out, 4.0 * f) < 1e-4 * power_at(&out, 2.0 * f));
}

#[test]
fn retrig_offsets_unison_phases_by_k_times_0_99_v9_squared() {
    // N = 2, Unison Phase 0.5: voice 1 starts 0.99·0.25 = 0.2475 cycles on, so the pair all but cancels harmonic 2.
    let f = hz(57.0) * FLAT;
    let params = [(4, 1.0 / 11.0), (5, 0.0), (8, 1.0), (9, 0.5)];
    let out = render(&mut played(&params, 57), 48_000).0;
    assert!(power_at(&out, 2.0 * f) < 1e-3 * power_at(&out, f));
}

#[test]
fn retrig_off_gives_an_idle_voice_fresh_random_phases() {
    let f = hz(57.0) * FLAT;
    let unison = [(4, 1.0 / 11.0), (5, 0.0)];
    let aligned = power_at(
        &render(&mut played(&[unison[0], unison[1], (8, 1.0)], 57), 24_000).0,
        f,
    );
    let mut s = played(&unison, 57);
    let first = power_at(&render(&mut s, 24_000).0, f);
    s.note_off(57);
    render(&mut s, 4_800);
    s.note_on(57, 1.0, 0.5);
    let second = power_at(&render(&mut s, 24_000).0, f);
    assert!(
        first < 0.99 * aligned && second < 0.99 * aligned,
        "{first} {second} {aligned}"
    );
    assert!((first / second - 1.0).abs() > 0.01, "{first} {second}");
}

#[test]
fn sync_eg_amount_is_cubed() {
    // a = (2·0.75 − 1)³ = 0.125 at sustain 1: S = 0.075 + 0.125 = 0.2 → R = 2, a square at 2f.
    let f = hz(57.0) * FLAT;
    let out = render(
        &mut played(&[(10, 1.0), (17, 0.75), (11, 0.075)], 57),
        48_000,
    )
    .0;
    assert!(power_at(&out[2_400..], 2.0 * f) > 100.0 * power_at(&out[2_400..], f));
}

#[test]
fn flip_also_applies_to_a_repeated_note() {
    // "Not above" the previous note includes the same key: a = −1 → S = 0 → R = 1.
    let f = hz(57.0) * FLAT;
    let mut s = synth(&[(10, 1.0), (17, 1.0), (18, 1.0)]);
    s.note_on(57, 1.0, 0.5);
    render(&mut s, 2_400);
    s.note_off(57);
    render(&mut s, 4_800);
    s.note_on(57, 1.0, 0.5);
    let out = render(&mut s, 48_000).0;
    assert!(power_at(&out, f) > 100.0 * power_at(&out, 6.0 * f));
}

#[test]
fn ladder_is_minus_12_db_at_6_25_fc_over_two_pi() {
    // ω = 6.25·fc: put 6.25·fc/2π on the note's fundamental; four stages at −3 dB give 1/16 of the power.
    let f0 = 440.0 * FLAT;
    let fc = std::f32::consts::TAU * f0 / 6.25;
    let cutoff = (fc.log2() - 4.0) / 10.3;
    let open = render(&mut played(&[], 69), 48_000).0;
    let at = render(&mut played(&[(19, cutoff)], 69), 48_000).0;
    let ratio = power_at(&at[4_800..], f0) / power_at(&open[4_800..], f0);
    let open_gain =
        1.0 / (1.0 + (f64::from(std::f32::consts::TAU * f0) / (6.25 * 20_000.0)).powi(2)).powi(2);
    let law = 0.0625 / (open_gain * open_gain);
    assert!((ratio / law - 1.0).abs() < 0.03, "{ratio} vs {law}");
}

#[test]
fn key_tracking_reads_the_note_without_transpose() {
    // Same pitch (57 + 12 vs 69), but key tracking at Kbd 1 puts fc an octave lower for the transposed note.
    let transposed = render(&mut played(&[(36, 1.0), (19, 0.2), (21, 1.0)], 57), 9_600).0;
    let plain = render(&mut played(&[(19, 0.2), (21, 1.0)], 69), 9_600).0;
    assert!(rms(&transposed[4_800..]) < 0.8 * rms(&plain[4_800..]));
}

/// Note 57 with the LFO on Frequency at A = 1 in Tmp Sync over 32 beats (φ = beat/32), played from `start_beat`.
fn tmp_sync_note(params: &[(usize, f32)], start_beat: f64) -> f32 {
    let mut s = synth(&[(45, 2.0 / 7.0), (41, 1.0), (42, 1.0), (44, 1.0)]);
    for &(index, value) in params {
        s.set_parameter(index, value);
    }
    s.note_on(57, 1.0, 0.5);
    let context = ProcessContext {
        start_beat,
        playing: true,
        ..STOPPED
    };
    note_of(&render_in(&mut s, 4_800, context).0[1_200..])
}

#[test]
fn lfo_shapes_at_known_phases() {
    let (sine, saw, square) = (0.0, 1.0 / 3.0, 2.0 / 3.0);
    // Sine sin 2πφ (bipolar); Saw 1 − φ; Square 1 below φ = 0.5, else 0. Frequency is ±12 st × out.
    for (shape, beat, note) in [
        (sine, 8.0, 69.0),
        (sine, 24.0, 45.0),
        (saw, 8.0, 66.0),
        (square, 8.0, 69.0),
        (square, 24.0, 57.0),
    ] {
        let got = tmp_sync_note(&[(39, shape)], beat);
        assert!(
            (got - note).abs() < 0.15,
            "shape {shape} beat {beat}: {got}"
        );
    }
    let random = tmp_sync_note(&[(39, 1.0)], 8.0);
    assert!(
        (56.9..69.0).contains(&random),
        "Random is unipolar: {random}"
    );
    // A = (2v − 1)·|2v − 1|: v = 0.75 → +0.25 (+3 st), v = 0.25 → −0.25.
    for (amount, note) in [(0.75, 60.0), (0.25, 54.0)] {
        let got = tmp_sync_note(&[(39, square), (44, amount)], 8.0);
        assert!((got - note).abs() < 0.1, "amount {amount}: {got}");
    }
}

#[test]
fn lfo_fade_in_rises_linearly_over_t_of_v46() {
    // T(0.5) = 0.305 s: at 0.152 s the square's +12 st is half faded in.
    let mut s = synth(&[(45, 2.0 / 7.0), (41, 1.0), (42, 1.0), (44, 1.0)]);
    s.set_parameter(39, 2.0 / 3.0);
    s.set_parameter(46, 0.5);
    s.note_on(57, 1.0, 0.5);
    let context = ProcessContext {
        start_beat: 8.0,
        playing: true,
        ..STOPPED
    };
    let out = render_in(&mut s, 9_600, context).0;
    let mid = note_of(&out[6_900..7_700]);
    assert!((mid - 63.0).abs() < 0.3, "{mid}");
}

#[test]
fn one_shot_lfo_holds_at_the_end_of_its_cycle() {
    // 30 Hz Saw: after 33 ms φ holds at 1, where Saw is 0.
    let params = [
        (45, 2.0 / 7.0),
        (41, 0.5),
        (40, 1.0),
        (44, 1.0),
        (39, 1.0 / 3.0),
    ];
    let out = render(&mut played(&params, 57), 24_000).0;
    assert!((note_of(&out[4_800..]) - 57.0).abs() < 0.02);
}

#[test]
fn a_new_voice_inherits_the_free_running_hertz_phase() {
    // Rate 0.5 → 1/(30·0.125 + 0.033333) Hz; 1892 ticks bring the global phase to 0.25, where Sine is 1.
    let params = [(45, 2.0 / 7.0), (40, 0.5), (44, 1.0)];
    let mut s = synth(&params);
    render(&mut s, 45_408);
    s.note_on(57, 1.0, 0.5);
    let out = render(&mut s, 4_800).0;
    assert!((note_of(&out[1_200..]) - 69.0).abs() < 0.2);
    // In arp mode the global phase stands still, so the arp's note starts at φ = 0.
    let mut s = synth(&params);
    s.set_parameter(49, 0.2);
    render(&mut s, 45_408);
    s.note_on(57, 1.0, 0.5);
    let out = render(&mut s, 4_800).0;
    assert!(note_of(&out[600..]) < 59.0);
}

#[test]
fn lfo_destination_rounds() {
    // round(7·0.93) = 7, Pan (int would give 6, Level): the square at A = −1 pans hard left.
    let (l, r) = render(
        &mut played(&[(39, 2.0 / 3.0), (44, 0.0), (45, 0.93)], 57),
        9_600,
    );
    assert!(rms(&l[4_800..]) > 0.1 && peak(&r[4_800..]) < 1e-4);
}

#[test]
fn mod_wheel_reaches_lfo_amount_and_rate() {
    // Dest 7 LFO Amount: v44 0.5 + 1 → A = 1, so the square lifts Frequency +12 st.
    let mut s = synth(&[(58, 1.0), (59, 7.0 / 8.0), (45, 2.0 / 7.0), (39, 2.0 / 3.0)]);
    s.set_controller(8, 1.0);
    s.note_on(57, 1.0, 0.5);
    assert!((note_of(&render(&mut s, 9_600).0[2_400..]) - 69.0).abs() < 0.05);
    // Dest 8 LFO Rate: v40 0 + 1 → 30 Hz; the square on Level at A = −1 gates.
    let full = rms(&render(&mut played(&[], 69), 4_800).0[1_000..]);
    let mut s = synth(&[
        (58, 1.0),
        (59, 1.0),
        (39, 2.0 / 3.0),
        (44, 0.0),
        (45, 6.0 / 7.0),
    ]);
    s.set_controller(8, 1.0);
    s.note_on(69, 1.0, 0.5);
    let out = render(&mut s, 4_800).0;
    assert!(rms(&out[300..700]) < 0.05 * full);
    assert!(rms(&out[1_100..1_500]) > 0.9 * full);
}

#[test]
fn mod_wheel_on_sync_and_on_supersync_pulse_width_after_the_half() {
    let f = hz(57.0) * FLAT;
    // Dest 2 Osc. Sync: S = 0.2 → R = 2.
    let mut s = synth(&[(10, 1.0), (58, 1.0), (59, 0.25)]);
    s.set_controller(8, 0.2);
    s.note_on(57, 1.0, 0.5);
    let out = render(&mut s, 48_000).0;
    assert!(power_at(&out, 2.0 * f) > 100.0 * power_at(&out, f));
    // Dest 3 Pulse Width: W = 0.5·0.5 + 0.25 = 0.5, a square without harmonic 2.
    let mut s = synth(&[(10, 1.0), (12, 0.5), (58, 1.0), (59, 0.375)]);
    s.set_controller(8, 0.25);
    s.note_on(57, 1.0, 0.5);
    let out = render(&mut s, 48_000).0;
    assert!(power_at(&out, 2.0 * f) < 1e-3 * power_at(&out, f));
}

#[test]
fn distortion_is_tanh_k_x_over_tanh_k() {
    // k = 10^(2v) − 0.99; on above v = 0.005. v = 0.03 and 0.1 show the normalisation, 0.5 the −0.99.
    let clean = render(&mut played(&[], 57), 9_600).0;
    for v in [0.03f32, 0.1, 0.5] {
        let k = 10f32.powf(2.0 * v) - 0.99;
        let driven = render(&mut played(&[(52, v)], 57), 9_600).0;
        for (x, y) in clean.iter().zip(&driven) {
            let law = (k * x).tanh() / k.tanh();
            assert!((law - y).abs() < 1e-5, "v {v}: {x} → {y}, law {law}");
        }
    }
}

#[test]
fn delay_tap_filter_is_a_one_pole_lp_below_centre_and_hp_above() {
    // c = 2·v57: 0.5 → α = 0.5, tap = s; 1.5 → α = 0.5⁴, tap = d − s. The first echo is m·tap of d = fb·x.
    let (fb, m) = (0.99 * 0.5, 0.8f32 * 0.8);
    for v57 in [0.25f32, 0.75] {
        let c = 2.0 * v57;
        let params = [
            (54, 0.0),
            (55, 0.5),
            (56, 0.8),
            (29, 0.05),
            (30, 0.0),
            (57, v57),
        ];
        let out = render(&mut played(&params, 57), 9_000).0;
        let mut s = 0.0f32;
        for i in 0..1_500 {
            let d = fb * out[i];
            let tap = if c <= 1.0 {
                s += (d - s) * c;
                s
            } else {
                s += (d - s) * (c - 1.0).powi(4);
                d - s
            };
            assert!((out[6_000 + i] - m * tap).abs() < 1e-5, "c {c} at {i}");
        }
    }
}

#[test]
fn delay_time_is_capped_at_two_seconds() {
    // 1 beat at 20 BPM is 3 s; the line holds 2 s = 96 000 frames.
    let (fb, m) = (0.99 * 0.5, 0.8f32 * 0.8);
    let slow = ProcessContext {
        tempo_bpm: 20.0,
        ..STOPPED
    };
    let mut s = synth(&[(54, 1.0), (55, 0.5), (56, 0.8), (29, 0.05), (30, 0.0)]);
    s.set_tempo_bpm(20.0);
    s.note_on(57, 1.0, 0.5);
    let out = render_in(&mut s, 98_000, slow).0;
    assert!(rms(&out[..1_500]) > 0.05);
    for i in 0..1_500 {
        assert!((out[96_000 + i] - fb * m * out[i]).abs() < 1e-5, "{i}");
    }
}

#[test]
fn release_from_full_takes_t_of_0_85_v_plus_0_05() {
    // R = T(0.475) = 0.2728 s from level 1: half-way (heard 0.25) at 0.136 s, silent after.
    let mut s = played(&[(31, 0.5)], 57);
    let full = rms(&render(&mut s, 24_000).0[12_000..]);
    s.note_off(57);
    let out = render(&mut s, 19_200).0;
    let half = envelope_at(&out, 0.1364, full);
    assert!((half - 0.25).abs() < 0.02, "{half}");
    assert_eq!(peak(&out[13_200..]), 0.0);
}

#[test]
fn a_stolen_voice_attacks_from_its_level() {
    // One voice, Attack T(0.45) = 0.23 s: the steal re-attacks from 1, so there is no dip.
    let mut s = played(&[(51, 0.0), (28, 0.5)], 57);
    render(&mut s, 24_000);
    s.note_on(64, 1.0, 0.5);
    let out = render(&mut s, 24_000).0;
    assert!(rms(&out[..480]) > 0.9 * rms(&out[12_000..]));
}

#[test]
fn the_filter_eg_runs_at_tick_rate() {
    // Decay T(0.3) = 0.122 s to sustain 0 at +8 oct: open at first, then the same as no EG.
    let with = render(
        &mut played(&[(19, 0.3), (26, 1.0), (23, 0.3), (24, 0.0)], 45),
        9_600,
    )
    .0;
    let without = render(&mut played(&[(19, 0.3)], 45), 9_600).0;
    assert!(rms(&with[240..1_200]) > 2.0 * rms(&without[240..1_200]));
    let difference: Vec<f32> = with.iter().zip(&without).map(|(a, b)| a - b).collect();
    assert!(rms(&difference[7_200..]) < 0.01 * rms(&without[7_200..]));
}

#[test]
fn the_sync_eg_runs_at_tick_rate() {
    // Decay T(0.3) = 0.122 s to sustain 0 at a = 1: S falls from 1 to 0, then matches a = 0.
    let with = render(
        &mut played(&[(10, 1.0), (17, 1.0), (14, 0.3), (15, 0.0)], 45),
        9_600,
    )
    .0;
    let without = render(&mut played(&[(10, 1.0)], 45), 9_600).0;
    let difference: Vec<f32> = with.iter().zip(&without).map(|(a, b)| a - b).collect();
    assert!(rms(&difference[240..1_200]) > 0.1 * rms(&without[240..1_200]));
    assert!(rms(&difference[7_200..]) < 1e-3 * rms(&without[7_200..]));
}

/// Mono, Constant off: 57, then 69 gliding from it.
fn mono_glide(slide: f32, frames: usize) -> Vec<f32> {
    let mut s = synth(&[(50, 1.0), (38, 0.0), (35, slide)]);
    s.note_on(57, 1.0, 0.5);
    render(&mut s, 2_400);
    s.note_off(57);
    s.note_on(69, 1.0, 0.5);
    render(&mut s, frames).0
}

#[test]
fn glide_without_constant_moves_one_over_s_semitones_per_second() {
    // s = (1/2)² = 0.25 → 4 st/s: 61 after 1 s.
    let out = mono_glide(1.0, 49_200);
    assert!((note_of(&out[46_800..]) - 61.0).abs() < 0.15);
    // s = (0.15/2)² = 5.6 ms (above the 1 ms floor) → 178 st/s: still low in the first 25 ms.
    let out = mono_glide(0.15, 4_800);
    assert!(note_of(&out[..1_200]) < 63.0);
}

#[test]
fn mono_legato_keeps_the_filter_eg() {
    // Filter attack T(0.5) = 0.3 s to +8 oct: the second key while the first is held does not restart it.
    let mut s = played(&[(50, 1.0), (19, 0.2), (26, 1.0), (22, 0.5)], 45);
    let before = render(&mut s, 24_000).0;
    s.note_on(52, 1.0, 0.5);
    let after = render(&mut s, 2_400).0;
    assert!(rms(&after) > 0.9 * rms(&before[21_600..]));
}

#[test]
fn turning_mono_on_silences_the_other_voices() {
    let mut s = played(&[], 57);
    s.note_on(64, 1.0, 0.5);
    render(&mut s, 4_800);
    s.set_parameter(50, 1.0);
    let out = render(&mut s, 24_000).0;
    let (kept, cut) = (hz(57.0) * FLAT, hz(64.0) * FLAT);
    assert!(power_at(&out, cut) < 1e-6 * power_at(&out, kept));
}

#[test]
fn arp_notes_keep_their_key_velocity() {
    let level = |velocity| {
        let mut s = synth(&[(49, 0.2)]);
        s.note_on(57, velocity, 0.5);
        rms(&render(&mut s, 4_800).0[2_400..])
    };
    assert!((level(0.5) / level(1.0) - 0.5).abs() < 1e-3);
}

#[test]
fn each_arp_step_releases_every_voice() {
    // Eight voices, sustain 1, release T(0.05) = 14 ms: in step 1 (69) the first step's 57 is gone.
    let mut s = synth(&[(49, 0.2), (51, 1.0), (31, 0.0)]);
    s.note_on(57, 1.0, 0.5);
    let out = render(&mut s, 12_000).0;
    let step = &out[7_200..];
    assert!(power_at(step, hz(57.0) * FLAT) < 1e-4 * power_at(step, hz(69.0) * FLAT));
}

#[test]
fn output_gain_glides_one_percent_per_frame_from_the_next_tick() {
    // 4824 frames = 201 ticks: the next tick takes Level 0 as its target, then g += 0.01·(0 − g) per frame.
    let mut reference = played(&[], 57);
    let mut s = played(&[], 57);
    render(&mut reference, 4_824);
    render(&mut s, 4_824);
    s.set_parameter(32, 0.0);
    let (r, o) = (render(&mut reference, 480).0, render(&mut s, 480).0);
    for n in 0..480 {
        if r[n].abs() > 0.05 {
            let law = 0.99f32.powi(n as i32 + 1);
            assert!(
                (o[n] / r[n] - law).abs() < 1e-3,
                "{n}: {} vs {law}",
                o[n] / r[n]
            );
        }
    }
}

#[test]
fn sub_and_unison_pan_switch_on_at_0_001() {
    // Sub level 0.05² = 0.0025 sounds (power 6.25e-6 of the main saw); 0.03² = 0.0009 is off.
    let f = hz(69.0) * FLAT;
    let sub = |v3: f32| {
        let out = render(&mut played(&[(3, v3)], 69), 48_000).0;
        power_at(&out, f / 2.0) / power_at(&out, f)
    };
    let on = sub(0.05);
    assert!((on / 6.25e-6 - 1.0).abs() < 0.3, "{on}");
    assert!(sub(0.03) < 1e-8);
    // Unison Pan 0.05 spreads the pair; 0.0009 leaves a mono sum.
    let (l, r) = render(
        &mut played(&[(4, 1.0 / 11.0), (5, 0.5), (6, 0.05)], 57),
        4_800,
    );
    assert_ne!(l, r);
    let (l, r) = render(
        &mut played(&[(4, 1.0 / 11.0), (5, 0.5), (6, 0.0009)], 57),
        4_800,
    );
    assert_eq!(l, r);
}

#[test]
fn retrig_legato_and_mono_are_on_at_any_nonzero_value() {
    // p8 Retrig, p34 Legato and p50 Mono Mode are on at v ≠ 0.
    let retrig = |v: f32| {
        let params = [(4, 1.0 / 11.0), (5, 0.0), (9, 0.5), (8, v)];
        render(&mut played(&params, 57), 24_000).0
    };
    let legato = |v: f32| {
        let mut s = synth(&[(35, 1.0), (34, v)]);
        s.note_on(57, 1.0, 0.5);
        render(&mut s, 2_400);
        s.note_off(57);
        s.note_on(69, 1.0, 0.5);
        render(&mut s, 4_800).0
    };
    let mono = |v: f32| {
        let mut s = synth(&[(50, v)]);
        for note in [57, 64, 60] {
            s.note_on(note, 1.0, 0.5);
        }
        render(&mut s, 9_600).0
    };
    let check = |name: &str, out: &dyn Fn(f32) -> Vec<f32>| {
        let on = out(1.0);
        assert!(out(0.0) != on, "{name} is audible");
        for v in [0.5, 0.001] {
            assert!(out(v) == on, "{name} {v} is on");
        }
    };
    check("Retrig", &retrig);
    check("Legato", &legato);
    check("Mono", &mono);
}

#[test]
fn transpose_steps_are_k_minus_12() {
    // s = trunc(f32(24·v)) − 12 is k − 12 at every v = f32(k/24); 0.99 is 11.
    let steps = (0..=24).map(|k| ((f64::from(k) / 24.0) as f32, k - 12));
    for (v, s) in steps.chain([(0.99, 11)]) {
        let f = crossing_hz(&render(&mut played(&[(36, v)], 69), 48_000).0[4_800..]);
        let want = 440.0 * FLAT * 2f32.powf(s as f32 / 12.0);
        assert!((f / want - 1.0).abs() < 0.01, "v {v}: {f} Hz, want {want}");
    }
}

#[test]
fn mono_switched_on_while_sounding_and_its_key_release_are_on_at_any_nonzero_value() {
    // Turning Mono on at 0.5 or 0.001 cuts the other voices as 1
    // does, and a Mono key release falls back to the highest held key as at 1.
    let switched_on = |v: f32| {
        let mut s = synth(&[]);
        for note in [57, 64, 60] {
            s.note_on(note, 1.0, 0.5);
        }
        render(&mut s, 4_800);
        s.set_parameter(50, v);
        render(&mut s, 9_600).0
    };
    let released = |v: f32| {
        let mut s = synth(&[(50, v)]);
        for note in [57, 64, 60] {
            s.note_on(note, 1.0, 0.5);
        }
        render(&mut s, 4_800);
        s.note_off(60);
        render(&mut s, 9_600).0
    };
    let check = |name: &str, out: &dyn Fn(f32) -> Vec<f32>| {
        let on = out(1.0);
        assert!(out(0.0) != on, "{name} is audible");
        for v in [0.5, 0.001] {
            assert!(out(v) == on, "{name} {v} is on");
        }
    };
    check("Mono switched on", &switched_on);
    check("Mono release", &released);
}
