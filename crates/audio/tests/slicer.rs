//! Slicer (ID 33): slice detection, keys, AUTO and playback, rendered offline.

use std::f64::consts::TAU;
use std::sync::Arc;

use honeydew_audio::ProcessContext;
use honeydew_audio::instrument::Instrument;
use honeydew_audio::sample::{SampleData, SampleMeta};
use honeydew_audio::slicer::{AutoNote, Slicer, auto_clip, auto_clip_beats, detect, first_note};

const RATE: f32 = 44_100.0;
const CONTEXT: ProcessContext = ProcessContext {
    sample_rate: RATE,
    tempo_bpm: 120.0,
    start_beat: 0.0,
    playing: true,
};

fn mono(left: Vec<f32>, rate: u32) -> SampleData {
    SampleData {
        sample_rate: rate,
        left: left.into(),
        right: None,
        meta: SampleMeta::default(),
    }
}

/// Silence of `frames`, with a 1 kHz burst decaying over about 10 ms (cut after 5000 frames) at each of `onsets`.
fn bursts(frames: usize, onsets: &[usize]) -> Vec<f32> {
    let mut x = vec![0.0_f32; frames];
    for &onset in onsets {
        for (t, s) in x[onset..].iter_mut().take(5000).enumerate() {
            let t = t as f64 / 44_100.0;
            *s += (0.8 * (TAU * 1000.0 * t).sin() * (-t / 0.01).exp()) as f32;
        }
    }
    x
}

// --- Detection ---

#[test]
fn each_burst_starts_a_slice_just_before_its_onset() {
    let onsets = [0, 6000, 15_000, 26_123, 37_000];
    let sample = mono(bursts(44_100, &onsets), 44_100);
    let slices = detect(&sample, 0.25);
    assert_eq!(slices.len(), onsets.len(), "{slices:?}");
    for (start, onset) in slices.iter().zip(onsets) {
        // The frame whose window first holds the onset, in silence: within one window before it.
        assert!(
            *start <= onset && onset - start < 128,
            "{start} for {onset}"
        );
    }
}

#[test]
fn a_slice_starts_on_a_zero_crossing_within_20_ms_before_it() {
    // A 50 Hz hum under the bursts: the start snaps back to the hum's latest zero crossing.
    let mut x = bursts(44_100, &[0, 20_000]);
    for (t, s) in x.iter_mut().enumerate() {
        *s += (0.1 * (TAU * 50.0 * t as f64 / 44_100.0).sin()) as f32;
    }
    let sample = mono(x.clone(), 44_100);
    let slices = detect(&sample, 0.25);
    assert_eq!(slices.len(), 2, "{slices:?}");
    let start = slices[1];
    assert!(start <= 20_000 && 20_000 - start <= 128 + 882, "{start}");
    assert!(x[start] == 0.0 || (x[start - 1] < 0.0) != (x[start] < 0.0));
}

#[test]
fn onsets_closer_than_60_ms_make_one_slice() {
    let sample = mono(bursts(44_100, &[0, 1323, 7000]), 44_100);
    let slices = detect(&sample, 1.0);
    assert_eq!(slices.len(), 2, "{slices:?}");
    assert!(slices[1] > 7000 - 128 && slices[1] <= 7000);
}

#[test]
fn at_most_128_slices() {
    let onsets: Vec<usize> = (0..140).map(|k| k * 2900).collect();
    let sample = mono(bursts(141 * 2900, &onsets), 44_100);
    let slices = detect(&sample, 0.25);
    assert_eq!(slices.len(), 128);
    assert!(slices.windows(2).all(|w| w[0] < w[1]));
}

#[test]
fn sensitivity_sets_the_threshold() {
    // A steady 1 kHz tone that steps up 3 dB at 22 050: a mild onset that only a high sensitivity hears.
    let x: Vec<f32> = (0..44_100)
        .map(|t| {
            let gain = if t < 22_050 { 0.5 } else { 0.5 * 2f64.sqrt() };
            (gain * (TAU * 1000.0 * t as f64 / 44_100.0).sin()) as f32
        })
        .collect();
    let sample = mono(x, 44_100);
    assert_eq!(detect(&sample, 1.0).len(), 2, "{:?}", detect(&sample, 1.0));
    assert_eq!(detect(&sample, 0.05), [0]);
}

#[test]
fn silence_or_no_onset_is_one_whole_slice() {
    assert_eq!(detect(&mono(vec![0.0; 10_000], 44_100), 0.5), [0]);
    assert_eq!(detect(&mono(Vec::new(), 44_100), 0.5), [] as [usize; 0]);
}

#[test]
fn sensitivity_0_uses_the_cue_markers() {
    let mut sample = mono(bursts(44_100, &[0, 20_000]), 44_100);
    assert_eq!(detect(&sample, 0.0), [0], "no cues: one whole-file slice");
    sample.meta.cues = vec![30_000, 10_000, 10_000, 99_999];
    assert_eq!(detect(&sample, 0.0), [10_000, 30_000]);
}

// --- Keys and AUTO ---

#[test]
fn the_first_note_is_48_or_above_the_other_slicers_and_clamped() {
    assert_eq!(first_note(None, 16), 48);
    assert_eq!(first_note(Some(63), 16), 64);
    assert_eq!(first_note(Some(120), 16), 112);
    assert_eq!(first_note(None, 100), 28);
    assert_eq!(first_note(None, 128), 0);
    assert_eq!(first_note(Some(127), 0), 127);
}

#[test]
fn auto_clip_length_follows_the_loop_tempo_or_the_song_tempo() {
    // Tagged: frames × tempo / (60 · 44 100), whatever the file's rate.
    assert_eq!(auto_clip_beats(88_200, Some(120.0), 90.0), 4.0);
    assert_eq!(
        auto_clip_beats(88_200, Some(100.0), 90.0),
        88_200.0 * 100.0 / 2_646_000.0
    );
    // Untagged: max(4, next power of 2 ≥ trunc(0.75 b)), b = frames × BPM / (44 100 · 60).
    assert_eq!(auto_clip_beats(88_200, None, 120.0), 4.0); // b 4 → 3 → 4
    assert_eq!(auto_clip_beats(211_680, None, 120.0), 8.0); // b 9.6 → 7 → 8
    assert_eq!(auto_clip_beats(441_000, None, 120.0), 16.0); // b 20 → 15 → 16
    assert_eq!(auto_clip_beats(44_100, None, 120.0), 4.0); // b 2 → 1 → 1, at least 4
}

#[test]
fn auto_clip_writes_one_note_per_slice() {
    let notes = auto_clip(&[0, 22_050, 66_150], 88_200, 48, 4.0);
    let note = |key, start_beats, length_beats| AutoNote {
        key,
        start_beats,
        length_beats,
        velocity: 0.7,
    };
    assert_eq!(
        notes,
        [note(48, 0.0, 1.0), note(49, 1.0, 2.0), note(50, 3.0, 1.0)]
    );
}

// --- Playback ---

/// A ramp, so every frame is distinct: x[i] = (i mod 1000) / 1000 − 0.5.
fn ramp(frames: usize) -> Vec<f32> {
    (0..frames)
        .map(|i| (i % 1000) as f32 / 1000.0 - 0.5)
        .collect()
}

fn loaded(x: Vec<f32>, rate: u32, slices: &[usize]) -> (Slicer, Arc<SampleData>) {
    let sample = Arc::new(mono(x, rate));
    let slicer = Slicer::new(RATE, Some(sample.clone()), slices.into(), 48);
    (slicer, sample)
}

fn render(slicer: &mut Slicer, frames: usize) -> (Vec<f32>, Vec<f32>) {
    let (mut left, mut right) = (vec![0.0; frames], vec![0.0; frames]);
    slicer.process(&mut left, &mut right, CONTEXT);
    (left, right)
}

#[test]
fn key_first_plus_k_plays_slice_k_to_its_end_with_a_512_frame_fade() {
    let (mut slicer, sample) = loaded(ramp(5000), 44_100, &[0, 1000, 3000]);
    slicer.note_on(49, 1.0, 0.5);
    let (left, right) = render(&mut slicer, 4000);
    assert_eq!(left[..1488], sample.left[1000..2488]);
    assert_eq!(left, right);
    for (n, s) in left.iter().enumerate().take(2000).skip(1488) {
        let fade = (2000 - n) as f32 / 512.0;
        assert!((s - sample.left[1000 + n] * fade).abs() < 1e-6, "{n}");
    }
    assert!(left[2000..].iter().all(|&s| s == 0.0));
}

#[test]
fn keys_outside_the_slices_play_nothing() {
    let (mut slicer, _) = loaded(ramp(5000), 44_100, &[0, 1000, 3000]);
    for key in [0, 47, 51, 127] {
        slicer.note_on(key, 1.0, 0.5);
    }
    assert!(render(&mut slicer, 5000).0.iter().all(|&s| s == 0.0));
}

#[test]
fn to_end_plays_past_the_slice_end_and_dips_there() {
    let (mut slicer, sample) = loaded(ramp(5000), 44_100, &[0, 1000, 3000]);
    slicer.set_parameter(5, 1.0);
    slicer.note_on(49, 1.0, 0.5);
    slicer.note_off(49);
    let (left, _) = render(&mut slicer, 4500);
    assert_eq!(left[..1488], sample.left[1000..2488]);
    assert!(left[1999].abs() < 1.0 / 512.0);
    assert_eq!(left[2000..4000], sample.left[3000..5000]);
    assert!(left[4000..].iter().all(|&s| s == 0.0));
}

#[test]
fn hold_plays_while_held_then_fades_at_0_01_per_sample() {
    let (mut slicer, sample) = loaded(vec![0.5; 20_000], 44_100, &[0, 1000]);
    slicer.set_parameter(5, 0.5);
    slicer.note_on(48, 1.0, 0.5);
    let (held, _) = render(&mut slicer, 3000);
    assert_eq!(held[2000..], sample.left[2000..3000], "past the slice end");
    slicer.note_off(48);
    let (released, _) = render(&mut slicer, 1000);
    for (n, s) in released.iter().enumerate().take(900) {
        let want = 0.5 * 0.99_f32.powi(n as i32 + 1);
        assert!((s - want).abs() < 1e-4, "{n}: {s} vs {want}");
    }
    assert!(
        released[920..].iter().all(|&s| s == 0.0),
        "ended below 1e-4"
    );
}

#[test]
fn sustain_off_ignores_the_note_off() {
    let (mut slicer, sample) = loaded(ramp(5000), 44_100, &[0, 1000, 3000]);
    slicer.note_on(48, 1.0, 0.5);
    slicer.note_off(48);
    slicer.all_notes_off();
    assert_eq!(render(&mut slicer, 400).0, sample.left[..400]);
}

/// The SUST mode p5 = `v` plays: Off (0) stops at the slice end, Hold (1) plays past it until the note-off fades it,
/// To End (2) plays past it through the note-off.
fn sust_mode(v: f32) -> u8 {
    let (mut slicer, _) = loaded(vec![0.5; 4000], 44_100, &[0, 1000]);
    slicer.set_parameter(5, v);
    slicer.note_on(48, 1.0, 0.5);
    if render(&mut slicer, 2000).0[1500] == 0.0 {
        return 0;
    }
    slicer.note_off(48);
    if render(&mut slicer, 1000).0[999] == 0.0 {
        1
    } else {
        2
    }
}

#[test]
fn sust_truncates_2v_in_f64() {
    // trunc(2v) in f64, so Hold starts at exactly 0.5 and To End at 1.
    let (half, one) = (0.5_f32, 1.0_f32);
    let probes = [
        0.0,
        0.25,
        half.next_down(),
        half,
        0.75,
        one.next_down(),
        one,
    ];
    assert_eq!(probes.map(sust_mode), [0, 0, 0, 1, 1, 1, 2]);
}

#[test]
fn mono_fast_fades_the_other_slices() {
    let (mut both, _) = loaded(vec![0.5; 20_000], 44_100, &[0, 10_000]);
    both.set_parameter(6, 1.0);
    both.note_on(48, 1.0, 0.5);
    render(&mut both, 100);
    both.note_on(49, 1.0, 0.5);
    let (left, _) = render(&mut both, 2000);
    let (mut alone, _) = loaded(vec![0.5; 20_000], 44_100, &[0, 10_000]);
    alone.note_on(49, 1.0, 0.5);
    let (reference, _) = render(&mut alone, 2000);
    for n in [0, 10, 100] {
        let fading = 0.5 * 0.99_f32.powi(n as i32 + 1);
        assert!((left[n] - reference[n] - fading).abs() < 1e-4, "{n}");
    }
    assert_eq!(left[1000..], reference[1000..]);
}

#[test]
fn mono_is_on_at_any_nonzero_value() {
    // p6 is on at v ≠ 0, 0.5 and below included.
    for (mono, fades) in [(0.0, false), (0.001, true), (0.5, true)] {
        let (mut slicer, _) = loaded(vec![0.5; 20_000], 44_100, &[0, 10_000]);
        slicer.set_parameter(6, mono);
        slicer.note_on(48, 1.0, 0.5);
        slicer.note_on(49, 1.0, 0.5);
        let (left, _) = render(&mut slicer, 2000);
        assert_eq!(left[1500] == 0.5, fades, "MONO {mono}: {}", left[1500]);
    }
}

#[test]
fn polyphony_sums_the_voices() {
    let x = ramp(5000);
    let (mut both, _) = loaded(x.clone(), 44_100, &[0, 1000, 3000]);
    both.note_on(48, 1.0, 0.5);
    both.note_on(50, 1.0, 0.5);
    let (sum, _) = render(&mut both, 3000);
    let (mut one, _) = loaded(x.clone(), 44_100, &[0, 1000, 3000]);
    one.note_on(48, 1.0, 0.5);
    let (mut other, _) = loaded(x, 44_100, &[0, 1000, 3000]);
    other.note_on(50, 1.0, 0.5);
    let (a, b) = (render(&mut one, 3000).0, render(&mut other, 3000).0);
    for n in 0..3000 {
        assert_eq!(sum[n], a[n] + b[n], "{n}");
    }
    // Far more voices than any song needs: no panic, still sounding.
    for _ in 0..300 {
        both.note_on(48, 1.0, 0.5);
    }
    assert!(render(&mut both, 64).0.iter().any(|&s| s != 0.0));
}

#[test]
fn pitch_steps_whole_semitones_by_resampling() {
    let x = ramp(20_000);
    for (pitch, step) in [
        (0.75, 2.0),
        (0.5 + 1.0 / 96.0, 1.0),
        (0.0, 0.25),
        (1.0, 4.0),
    ] {
        let (mut slicer, sample) = loaded(x.clone(), 44_100, &[0]);
        slicer.set_parameter(1, pitch);
        slicer.note_on(48, 1.0, 0.5);
        let (left, _) = render(&mut slicer, 600);
        for (n, s) in left.iter().enumerate().take(400) {
            let pos = n as f64 * step;
            let (i, t) = (pos as usize, pos.fract() as f32);
            let want = sample.left[i] + (sample.left[i + 1] - sample.left[i]) * t;
            assert!(
                (s - want).abs() < 1e-6,
                "pitch {pitch} frame {n}: {s} vs {want}"
            );
        }
    }
}

#[test]
fn pitch_truncates_48v_minus_24_in_f64_at_the_stepped_values() {
    // s = trunc(f64(v)·48 − 24), ratio 2^(s/12). At v = f32(k/48) that is k − 24, except these k, where the
    // f32 value lies just off k/48.
    let semitones = |k: i32| match k {
        1 | 2 | 4 | 8 | 11 | 13 | 16 | 19 | 22 => k - 23,
        25 | 28 | 31 | 34 | 37 | 40 | 43 | 46 => k - 25,
        _ => k - 24,
    };
    // A ramp of x[i] = i/65536: the second output frame reads position `step`.
    let ramp: Vec<f32> = (0..1000).map(|i| i as f32 / 65_536.0).collect();
    for k in 0..=48 {
        let (mut slicer, _) = loaded(ramp.clone(), 44_100, &[0]);
        slicer.set_parameter(1, k as f32 / 48.0);
        slicer.note_on(48, 1.0, 0.5);
        let step = f64::from(render(&mut slicer, 2).0[1]) * 65_536.0;
        let measured = (12.0 * step.log2()).round() as i32;
        assert_eq!(measured, semitones(k), "k = {k}, step {step}");
    }
}

#[test]
fn the_file_rate_sets_the_step_with_linear_interpolation() {
    let (mut slicer, sample) = loaded(ramp(5000), 22_050, &[0]);
    slicer.note_on(48, 1.0, 0.5);
    let (left, _) = render(&mut slicer, 100);
    assert_eq!(left[2], sample.left[1]);
    assert!((left[3] - (sample.left[1] + sample.left[2]) * 0.5).abs() < 1e-7);
}

#[test]
fn velocity_is_linear_and_pan_follows_the_note_pan_law() {
    let x = vec![0.5; 5000];
    for (velocity, pan, want_l, want_r) in [
        (0.5, 0.5, 0.25, 0.25),
        (1.0, 0.0, 0.5, 0.0),
        (1.0, 1.0, 0.0, 0.5),
        (1.0, 0.75, 0.25, 0.5),
    ] {
        let (mut slicer, _) = loaded(x.clone(), 44_100, &[0]);
        slicer.note_on(48, velocity, pan);
        let (left, right) = render(&mut slicer, 10);
        assert_eq!((left[5], right[5]), (want_l, want_r), "{velocity} {pan}");
    }
}

#[test]
fn stereo_files_play_in_stereo() {
    let sample = Arc::new(SampleData {
        right: Some(vec![-0.25; 5000].into()),
        ..mono(vec![0.5; 5000], 44_100)
    });
    let mut slicer = Slicer::new(RATE, Some(sample), Arc::from([0]), 48);
    slicer.note_on(48, 1.0, 0.5);
    let (left, right) = render(&mut slicer, 10);
    assert_eq!((left[5], right[5]), (0.5, -0.25));
}

#[test]
fn attack_and_decay_follow_the_pad_envelope_laws() {
    // Attack 0.5: c = 0.0002/(500·x⁴), x = 0.95v + 0.05; full after ln 0.8 / ln(1 − c) samples, about 0.96 s.
    let (mut slicer, _) = loaded(vec![1.0; 60_000], 44_100, &[0]);
    slicer.set_parameter(3, 0.5);
    slicer.note_on(48, 1.0, 0.5);
    let (left, _) = render(&mut slicer, 50_000);
    let c = 0.0002 / (500.0 * 0.525_f64.powi(4));
    let full = (0.8_f64.ln() / (1.0 - c).ln()).ceil() as usize;
    let reached = left.iter().position(|&s| s >= 1.0).unwrap();
    assert!(reached.abs_diff(full) <= 2, "{reached} vs {full}");
    assert_eq!(left[reached + 100], 1.0, "Decay 1 = no decay");

    // Decay 0.25: τ = −1/ln(1 − c), c = 0.0002/(50·x⁴), about 38.7 ms.
    let (mut slicer, _) = loaded(vec![1.0; 60_000], 44_100, &[0]);
    slicer.set_parameter(4, 0.25);
    slicer.note_on(48, 1.0, 0.5);
    let (left, _) = render(&mut slicer, 10_000);
    let c = 0.0002 / (50.0 * 0.2875_f64.powi(4));
    assert_eq!(left[0], 1.0);
    let tau = (-1.0 / (1.0 - c).ln()).round() as usize;
    assert!((left[tau] - (-1.0_f32).exp()).abs() < 1e-3, "{}", left[tau]);
    assert!((tau as f32 / 44.1 - 38.7).abs() < 0.1, "{tau}");

    // Decay 0: instant.
    let (mut slicer, _) = loaded(vec![1.0; 60_000], 44_100, &[0]);
    slicer.set_parameter(4, 0.0);
    slicer.note_on(48, 1.0, 0.5);
    let (left, _) = render(&mut slicer, 100);
    assert_eq!(left[0], 1.0);
    assert!(left[1..].iter().all(|&s| s == 0.0));
}

#[test]
fn block_size_does_not_change_the_output() {
    let play = |sizes: &[usize]| {
        let (mut slicer, _) = loaded(ramp(30_000), 44_100, &[0, 7000, 15_000]);
        slicer.set_parameter(1, 0.6);
        slicer.set_parameter(3, 0.1);
        slicer.set_parameter(4, 0.4);
        slicer.note_on(48, 0.8, 0.3);
        slicer.note_on(50, 0.6, 0.7);
        let mut out = Vec::new();
        for &size in sizes {
            out.extend(render(&mut slicer, size).0);
        }
        out
    };
    assert_eq!(play(&[20_000]), play(&[1, 7, 333, 1023, 4096, 14_540]));
}

#[test]
fn reset_silences_and_a_missing_loop_is_silent() {
    let (mut slicer, _) = loaded(ramp(5000), 44_100, &[0]);
    slicer.note_on(48, 1.0, 0.5);
    slicer.reset();
    assert!(render(&mut slicer, 100).0.iter().all(|&s| s == 0.0));

    let mut missing = Slicer::new(RATE, None, Arc::from([]), 48);
    missing.note_on(48, 1.0, 0.5);
    let (mut left, mut right) = (vec![0.0; 64], vec![0.0; 64]);
    missing.process(&mut left, &mut right, CONTEXT);
    assert!(left.iter().all(|&s| s == 0.0));
}

#[test]
fn a_note_off_fades_only_its_own_key() {
    let (mut slicer, _) = loaded(vec![0.5; 20_000], 44_100, &[0, 10_000]);
    slicer.set_parameter(5, 0.5);
    slicer.note_on(48, 1.0, 0.5);
    slicer.note_on(49, 1.0, 0.5);
    slicer.note_off(48);
    let (left, _) = render(&mut slicer, 2000);
    assert_eq!(left[1500..], [0.5; 500], "49 still held");
}

#[test]
fn a_full_pool_takes_the_oldest_voice() {
    // 256 voices: the loud first note, then 255 silent ones; one more silent note takes the oldest, the loud one.
    let (mut slicer, _) = loaded(vec![0.5; 20_000], 44_100, &[0]);
    slicer.note_on(48, 1.0, 0.5);
    for _ in 0..255 {
        slicer.note_on(48, 0.0, 0.5);
    }
    assert!(render(&mut slicer, 10).0.iter().all(|&s| s == 0.5));
    slicer.note_on(48, 0.0, 0.5);
    assert!(render(&mut slicer, 10).0.iter().all(|&s| s == 0.0));
}

#[test]
fn velocity_above_1_plays_at_1() {
    let (mut slicer, _) = loaded(vec![0.5; 5000], 44_100, &[0]);
    slicer.note_on(48, 2.0, 0.5);
    assert_eq!(render(&mut slicer, 10).0, [0.5; 10]);
}

#[test]
fn decay_above_0_99_holds_and_below_0_018_is_instant() {
    let decayed = |decay: f32| {
        let (mut slicer, _) = loaded(vec![1.0; 5000], 44_100, &[0]);
        slicer.set_parameter(4, decay);
        slicer.note_on(48, 1.0, 0.5);
        render(&mut slicer, 1001).0
    };
    assert_eq!(decayed(0.991)[1000], 1.0);
    // 0.99 still decays: c = 0.0002/(50·0.9905⁴).
    let c = 0.0002 / (50.0 * 0.9905_f64.powi(4));
    assert!((f64::from(decayed(0.99)[1000]) - (1.0 - c).powi(1000)).abs() < 1e-6);
    // c = 1 when 50x⁴ < 0.001 (x = 0.95v + 0.05 below 0.0669, v below about 0.0178).
    assert_eq!(decayed(0.01)[1], 0.0);
    let c = 0.0002 / (50.0 * 0.069_f64.powi(4));
    assert!((f64::from(decayed(0.02)[1]) - (1.0 - c)).abs() < 1e-6);
}

#[test]
fn enable_off_freezes_the_voices_and_on_resumes_them() {
    // The Slicer freezes, it does not reset.
    use honeydew_audio::instrument::InstrumentModule;
    let play = |toggle: bool| {
        let (mut slicer, _) = loaded(ramp(5000), 44_100, &[0]);
        slicer.note_on(48, 1.0, 0.5);
        let mut module = InstrumentModule::new(Box::new(slicer), 256);
        let mut out = Vec::new();
        for part in 0..3 {
            if toggle {
                module.set_parameter(0, if part == 1 { 0.0 } else { 1.0 });
            }
            let (mut left, mut right) = (vec![0.0; 500], vec![0.0; 500]);
            module.process(&mut left, &mut right, CONTEXT);
            out.push(left);
        }
        out
    };
    let (toggled, straight) = (play(true), play(false));
    assert!(toggled[1].iter().all(|&s| s == 0.0), "off: not rendered");
    assert_eq!(toggled[2], straight[1], "on: resumes where it froze");
}

// --- Golden slice starts ---
//
// Each signal below is built here, deterministically (f64 math, xorshift32 noise, f32 samples). The expected starts
// come from an independent numpy model of the detector (D and the start moves), run on these exact samples; the
// targeted cases were chosen so that they tell the detector's details apart (noted per case).

/// xorshift32 noise, uniform in [−1, 1).
struct Noise(u32);

impl Noise {
    fn next(&mut self) -> f64 {
        let mut s = self.0;
        s ^= s << 13;
        s ^= s >> 17;
        s ^= s << 5;
        self.0 = s;
        f64::from(s >> 8) / f64::from(1u32 << 24) * 2.0 - 1.0
    }
}

const ONSETS: [usize; 10] = [
    0, 7001, 15_003, 22_050, 30_111, 41_000, 52_500, 61_234, 70_000, 80_000,
];

fn sine(hz: f64, t: usize) -> f64 {
    (TAU * hz * t as f64 / 44_100.0).sin()
}

/// Kicks (a 260 → 60 Hz sweep) or hats (noise) at `onsets`, at most 6000 frames each.
fn hits(n: usize, onsets: &[usize], kick: bool) -> Vec<f64> {
    let (mut noise, mut x) = (Noise(7), vec![0.0; n]);
    for &onset in onsets {
        for j in 0..6000.min(n - onset) {
            let t = j as f64 / 44_100.0;
            x[onset + j] += if kick {
                0.9 * (TAU * (60.0 + 200.0 * (-t / 0.01).exp()) * t).sin() * (-t / 0.08).exp()
            } else {
                0.5 * noise.next() * (-t / 0.005).exp()
            };
        }
    }
    x
}

fn to_f32(x: impl IntoIterator<Item = f64>) -> Vec<f32> {
    x.into_iter().map(|s| s as f32).collect()
}

/// Kicks and hats taking turns over noise at 0.05: 2 s.
fn kick_hat_noise() -> Vec<f32> {
    let every_other = |skip| {
        ONSETS
            .iter()
            .copied()
            .skip(skip)
            .step_by(2)
            .collect::<Vec<_>>()
    };
    let kicks = hits(88_200, &every_other(0), true);
    let hats = hits(88_200, &every_other(1), false);
    let mut noise = Noise(99);
    to_f32((0..88_200).map(|t| kicks[t] + hats[t] + 0.05 * noise.next()))
}

/// Hats over a hum at `hz` (0.1).
fn hats_over_hum(hz: f64) -> Vec<f32> {
    let hats = hits(88_200, &ONSETS, false);
    to_f32((0..88_200).map(|t| hats[t] + 0.1 * sine(hz, t)))
}

/// A 1 kHz tone at 0.02 whose power grows by 1.195 per hop for the 320 frames before 11 025, where it jumps to 1 and
/// decays; under it noise at 0.002 and `dc`.
fn crescendo(dc: f64) -> Vec<f32> {
    let (mut noise, rate) = (Noise(1), 1.195_f64.ln() / 32.0);
    to_f32((0..22_050).map(|t| {
        let a = if t < 10_705 {
            0.02
        } else if t < 11_025 {
            0.02 * (rate * (t - 10_705) as f64).exp()
        } else {
            (-((t - 11_025) as f64) / 4410.0).exp()
        };
        a * sine(1000.0, t) + 0.002 * noise.next() + dc
    }))
}

/// A 100 Hz hum at 0.01, a click of 0.05 at 6000 and one of 1 at 8559.
fn clicks() -> Vec<f32> {
    let mut x: Vec<f64> = (0..22_050).map(|t| 0.01 * sine(100.0, t)).collect();
    x[6000] += 0.05;
    x[8559] += 1.0;
    to_f32(x)
}

/// Silence, then from 5000 a 1 kHz tone (starting at sin 0) decaying over 50 ms.
fn from_silence() -> Vec<f32> {
    to_f32((0..22_050_usize).map(|t| match t.checked_sub(5000) {
        Some(j) => {
            let s = j as f64 / 44_100.0;
            0.8 * (TAU * 1000.0 * s).sin() * (-s / 0.05).exp()
        }
        None => 0.0,
    }))
}

/// Noise at 0.05 under a tone on FFT bin 6 (2067.19 Hz) that steps from 0.2 to 0.3 at 11 025.
fn tone_step() -> Vec<f32> {
    let mut noise = Noise(3);
    to_f32((0..22_050).map(|t| {
        let a = if t < 11_025 { 0.2 } else { 0.3 };
        a * sine(2067.1875, t) + 0.05 * noise.next()
    }))
}

fn stereo(left: Vec<f32>, right: Vec<f32>) -> SampleData {
    SampleData {
        right: Some(right.into()),
        ..mono(left, 44_100)
    }
}

#[test]
fn golden_starts_of_kicks_hats_hums_and_a_louder_right_channel() {
    let kicks = hits(88_200, &ONSETS, true);
    let stereo = stereo(
        to_f32(kicks.iter().copied()),
        to_f32(kicks.iter().map(|s| 2.0 * s)),
    );
    let mix = || mono(kick_hat_noise(), 44_100);
    let cases: [(&str, SampleData, f32, &[usize]); 6] = [
        (
            "kick, hat and noise at 0.1",
            mix(),
            0.1,
            &[
                0, 6911, 14_894, 21_952, 30_000, 40_893, 52_400, 61_136, 69_904, 79_896,
            ],
        ),
        (
            "kick, hat and noise at 0.5",
            mix(),
            0.5,
            &[
                0, 6894, 14_894, 21_952, 30_000, 40_893, 52_400, 61_136, 69_886, 79_896,
            ],
        ),
        (
            "kick, hat and noise at 1",
            mix(),
            1.0,
            &[
                0, 6894, 14_894, 21_952, 30_000, 36_008, 40_893, 52_400, 58_395, 61_119, 69_886,
                75_910, 79_896,
            ],
        ),
        // Each start snaps to the hum's last zero crossing (at most 441 frames back at 50 Hz).
        (
            "hats over a 50 Hz hum",
            mono(hats_over_hum(50.0), 44_100),
            0.33,
            &[
                0, 6615, 14_553, 21_609, 29_548, 40_572, 52_039, 60_859, 69_678, 79_821,
            ],
        ),
        // At 30 Hz the crossings are 735 frames apart: some snaps reach further back than 441 frames.
        (
            "hats over a 30 Hz hum",
            mono(hats_over_hum(30.0), 44_100),
            0.33,
            &[
                0, 6616, 14_701, 21_316, 29_401, 40_426, 52_186, 61_005, 69_826, 79_381,
            ],
        ),
        (
            "kicks, the right channel twice as loud",
            stereo,
            0.33,
            &[
                0, 5880, 12_881, 20_883, 27_930, 35_991, 40_880, 46_880, 52_384, 58_380, 61_120,
                67_114, 69_888, 75_880, 79_888,
            ],
        ),
    ];
    for (name, sample, sensitivity, want) in cases {
        assert_eq!(detect(&sample, sensitivity), want, "{name}");
    }
}

#[test]
fn the_start_moves_back_to_sens_d_above_0_75_then_forward_to_5_percent_rms() {
    // The crescendo's frames reach sens·D ≈ 0.88 for 14 frames before the jump fires; the start moves back to the
    // first of them, then forward to the first frame whose normalised RMS is above 0.05 (the RMS crosses it inside
    // the crescendo), then to the tone's zero crossing.
    assert_eq!(detect(&mono(crescendo(0.0), 44_100), 1.0), [0, 10_849]);
    // With a DC offset of 0.1 every frame is above 5 % RMS: the RMS is taken on the normalised file, not on the
    // high-passed copy, so the start stays at the back-moved frame.
    assert_eq!(detect(&mono(crescendo(0.1), 44_100), 1.0), [0, 10_672]);
}

#[test]
fn the_rms_floor_is_relative_to_the_peak_over_both_channels() {
    // The right channel is twice as loud: the left's normalised RMS is halved, so the forward move goes further.
    let left = crescendo(0.0);
    let right = left.iter().map(|s| 2.0 * s).collect();
    assert_eq!(detect(&stereo(left, right), 1.0), [0, 10_893]);
}

#[test]
fn the_next_search_resumes_at_the_rounded_frame() {
    // The slice at 5733 (the hum's zero crossing) resumes the search at round((5733 + 2646 + 64)/16) = 528. The
    // second click first shows in frame 527 (floor) and no longer fires at 528: no slice.
    assert_eq!(detect(&mono(clicks(), 44_100), 0.01), [0, 5733]);
}

#[test]
fn an_onset_from_silence_is_measured_against_the_band_floor() {
    // From silence, D = k·(E/ε − 1): about 1.804e6 on the tone's first frame (305). Sensitivity 5 % either side of
    // 1/D pins the band power law (window gain, power, bins and ε).
    let sample = mono(from_silence(), 44_100);
    assert_eq!(
        detect(&sample, 5.266_744e-7),
        [0],
        "below: no onset, one whole slice"
    );
    assert_eq!(detect(&sample, 5.821_138_4e-7), [4880]);
}

#[test]
fn an_onset_needs_sens_d_of_at_least_1() {
    // A tone on FFT bin 6 steps up 3.5 dB over noise: D peaks at 2.187 (frame 686). 5 % either side of 1/D.
    let sample = mono(tone_step(), 44_100);
    assert_eq!(detect(&sample, 0.434_358_06), [0]);
    assert_eq!(detect(&sample, 0.480_079_98), [0, 10_976]);
}
