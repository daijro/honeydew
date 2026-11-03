//! The six built-in 3x Osc shapes: one 2048-frame cycle each, checked by their own spectra.

use std::f64::consts::TAU;

use honeydew_audio::ProcessContext;
use honeydew_audio::builtin_drums::builtin;
use honeydew_audio::builtin_shapes::{
    CYCLE, PARTIALS, RATE, SHAPES, VOWEL_A_FORMANTS, VOWEL_O_FORMANTS, shape,
};
use honeydew_audio::instrument::Instrument;
use honeydew_audio::sample::SampleData;
use honeydew_audio::three_osc::ThreeOsc;
use honeydew_audio::three_osc::osc::Wave;

const MINUS_1_DBFS: f32 = 0.891_250_9;

fn cycle(name: &str) -> Vec<f32> {
    let data = shape(name).unwrap_or_else(|| panic!("no shape {name}"));
    assert_eq!(data.sample_rate, RATE);
    assert!(data.right.is_none(), "{name} is mono");
    data.left.to_vec()
}

/// Amplitudes of harmonics 0..CYCLE/2 of one cycle (index = harmonic number; DC at 0).
fn spectrum(x: &[f32]) -> Vec<f64> {
    let n = x.len();
    let (cos, sin): (Vec<f64>, Vec<f64>) = (0..n)
        .map(|m| (TAU * m as f64 / n as f64).sin_cos())
        .map(|(s, c)| (c, s))
        .unzip();
    (0..n / 2)
        .map(|k| {
            let (re, im) = x.iter().enumerate().fold((0.0, 0.0), |(re, im), (i, &v)| {
                let m = (k * i) % n;
                (re + v as f64 * cos[m], im + v as f64 * sin[m])
            });
            (if k == 0 { 1.0 } else { 2.0 }) * re.hypot(im) / n as f64
        })
        .collect()
}

/// The first 16 partials scaled to their own maximum.
fn profile(name: &str) -> Vec<f64> {
    let s = spectrum(&cycle(name));
    let top = s[1..=16].iter().fold(0.0_f64, |m, &v| m.max(v));
    s[1..=16].iter().map(|v| v / top).collect()
}

/// The harmonic with the largest magnitude among 1..=`upto`.
fn loudest(s: &[f64], upto: usize) -> usize {
    (1..=upto).max_by(|&a, &b| s[a].total_cmp(&s[b])).unwrap()
}

/// The cycle's wrap step (last frame to first) over its largest step inside the cycle: about 1 or less when it
/// loops cleanly, far above 1 for a cut waveform.
fn click(x: &[f32]) -> f32 {
    let inside = x
        .windows(2)
        .map(|w| (w[1] - w[0]).abs())
        .fold(0.0, f32::max);
    (x[0] - x[x.len() - 1]).abs() / inside
}

#[test]
fn six_named_shapes_of_one_cycle_each() {
    assert_eq!(
        SHAPES,
        ["Glass", "Reed", "Vowel A", "Vowel O", "Thin Pulse", "Grit"]
    );
    for name in SHAPES {
        assert_eq!(cycle(name).len(), CYCLE, "{name}");
    }
    assert_eq!(CYCLE, 2048);
    assert!(shape("Saw").is_none());
    assert!(shape("").is_none());
}

#[test]
fn deterministic_finite_dc_free_and_peaking_at_minus_1_dbfs() {
    for name in SHAPES {
        let x = cycle(name);
        assert_eq!(x, cycle(name), "{name}: two builds differ");
        assert!(x.iter().all(|v| v.is_finite()), "{name}: NaN or inf");
        let peak = x.iter().fold(0.0_f32, |m, v| m.max(v.abs()));
        assert!((peak - MINUS_1_DBFS).abs() < 1e-4, "{name}: peak {peak}");
        let mean = x.iter().sum::<f32>() / x.len() as f32;
        assert!(mean.abs() < 1e-3, "{name}: mean {mean}");
    }
}

#[test]
fn every_cycle_loops_without_a_click() {
    for name in SHAPES {
        let c = click(&cycle(name));
        assert!(
            c <= 1.05,
            "{name}: the wrap step is {c}x the largest step inside"
        );
    }
    // The measure does see a cut: a naive ramp's wrap is a full-scale jump.
    let ramp: Vec<f32> = (0..CYCLE).map(|i| i as f32 / CYCLE as f32).collect();
    assert!(click(&ramp) > 100.0);
}

#[test]
fn glass_is_sparse_with_a_strong_fundamental() {
    let s = spectrum(&cycle("Glass"));
    let strong: Vec<usize> = (1..=PARTIALS)
        .filter(|&k| s[k] > 0.02 * s[loudest(&s, PARTIALS)])
        .collect();
    assert_eq!(&strong[..5], [1, 2, 3, 4, 6]);
    assert!(
        strong.len() <= 16,
        "{strong:?}: a few separated partials, not a ramp"
    );
    assert_eq!(loudest(&s, PARTIALS), 1);
    assert!(
        s[PARTIALS + 1..].iter().all(|&v| v < 1e-4),
        "no partials above {PARTIALS}"
    );
    assert!(s[5] < 1e-4 && s[7] < 1e-4, "gaps between the partials");
}

#[test]
fn reed_leads_with_the_odd_partials() {
    let s = spectrum(&cycle("Reed"));
    for n in (3..=15).step_by(2) {
        assert!(s[n] > 4.0 * s[n - 1], "partial {n} vs {}", n - 1);
        assert!(s[n] > 4.0 * s[n + 1], "partial {n} vs {}", n + 1);
    }
    assert!(s[2] > 0.0, "the even partials are weak, not absent");
    assert_eq!(loudest(&s, PARTIALS), 1);
    assert!(s[PARTIALS + 1..].iter().all(|&v| v < 1e-4));
}

#[test]
fn the_vowels_have_formant_bumps() {
    for (name, formants) in [("Vowel A", VOWEL_A_FORMANTS), ("Vowel O", VOWEL_O_FORMANTS)] {
        let s = spectrum(&cycle(name));
        let peak = loudest(&s, PARTIALS);
        assert!(
            (peak as f64 - formants[0]).abs() <= 1.0,
            "{name}: loudest partial {peak}, first formant at {}",
            formants[0]
        );
        assert!(
            s[peak] > 1.5 * s[1],
            "{name}: the bump stands above the fundamental"
        );
        // The third formant lifts the top above the quiet stretch before it.
        let f3 = formants[2].round() as usize;
        assert!(s[f3] > 2.0 * s[f3 - 8], "{name}: no third formant");
        assert!(s[PARTIALS + 1..].iter().all(|&v| v < 1e-4), "{name}");
    }
    // "Ah" keeps its second formant apart from the first; "oh" runs the two into one hump.
    let a = spectrum(&cycle("Vowel A"));
    let f2 = VOWEL_A_FORMANTS[1].round() as usize;
    assert!(
        a[f2] > a[f2 - 1] && a[f2] > a[f2 + 1],
        "Vowel A: no second bump at {f2}"
    );
    // And "ah" is the brighter of the two.
    let centroid = |name| {
        let s = spectrum(&cycle(name));
        (1..=PARTIALS).map(|k| k as f64 * s[k]).sum::<f64>()
            / (1..=PARTIALS).map(|k| s[k]).sum::<f64>()
    };
    assert!(centroid("Vowel A") > centroid("Vowel O"));
}

#[test]
fn thin_pulse_is_a_twelve_percent_pulse_from_partials() {
    let s = spectrum(&cycle("Thin Pulse"));
    // |sin(n pi d)| / n, d = 0.12: nulls at n = 25 (exact) and near 8.33 and 16.67.
    let want = |n: usize| (n as f64 * 0.12 * std::f64::consts::PI).sin().abs() / n as f64;
    for n in 1..=PARTIALS {
        assert!(
            (s[n] / s[1] - want(n) / want(1)).abs() < 2e-3,
            "partial {n}"
        );
    }
    assert!(s[25] < 1e-4 * s[1] + 1e-6);
    assert!(s[PARTIALS + 1..].iter().all(|&v| v < 1e-4));
    // The pulse is the short high stretch: about 12 % of the cycle is above half the peak.
    let x = cycle("Thin Pulse");
    let high = x.iter().filter(|&&v| v > 0.5 * MINUS_1_DBFS).count() as f64 / CYCLE as f64;
    assert!((high - 0.12).abs() < 0.03, "{high}");
}

#[test]
fn grit_is_broadband_and_tilts_down() {
    let s = spectrum(&cycle("Grit"));
    let top = s[1..].iter().fold(0.0_f64, |m, &v| m.max(v));
    // Noise-like: nearly every low partial is present, none dominates.
    let present = (1..=256).filter(|&k| s[k] > 0.05 * top).count();
    assert!(present > 230, "{present} of 256 partials present");
    assert!(top < 0.12, "no tone: the largest partial is {top}");
    // Smoothed: the top half of the partials carries far less per partial than the lowest quarter.
    let power = |a: usize, b: usize| (a..b).map(|k| s[k] * s[k]).sum::<f64>() / (b - a) as f64;
    assert!(
        power(512, 1024) < 0.05 * power(1, 256),
        "smoothed, not white"
    );
}

#[test]
fn no_two_shapes_share_a_spectrum() {
    for (i, a) in SHAPES.iter().enumerate() {
        for b in &SHAPES[i + 1..] {
            let (pa, pb) = (profile(a), profile(b));
            let apart = pa
                .iter()
                .zip(&pb)
                .map(|(x, y)| (x - y).abs())
                .fold(0.0, f64::max);
            assert!(apart > 0.1, "{a} and {b}: first 16 partials within {apart}");
        }
    }
}

#[test]
fn the_builtin_path_serves_the_same_cycles_untouched() {
    for name in SHAPES {
        for sep in ['/', '\\'] {
            let path = format!("Built-in{sep}Shapes{sep}{name}");
            let data = builtin(&path).unwrap_or_else(|| panic!("{path}"));
            // No pad fade: the cycle has to loop, so its last frame is not forced to silence.
            assert_eq!(data.left.to_vec(), cycle(name), "{path}");
            assert_eq!(data.sample_rate, RATE);
        }
    }
    for bad in [
        "Built-in/Shapes",
        "Built-in/Shapes/",
        "Built-in/Shapes/Nope",
        "Built-in/Glass",
        "Shapes/Glass",
    ] {
        assert!(builtin(bad).is_none(), "{bad}");
    }
    assert!(
        builtin("Built-in/Kick").is_some(),
        "the default kit still plays"
    );
}

const FS: f32 = 48_000.0;

/// Osc 1 alone on Custom, playing A4 for one second.
fn play_a4(sample: &SampleData) -> Vec<f32> {
    let mut s = ThreeOsc::with_custom_shape(FS, 4, Some(sample));
    for (i, v) in [(1, 1.0), (22, 0.0), (23, 0.0)] {
        s.set_parameter(i, v);
    }
    s.note_on(69, 1.0, 0.5);
    let (mut l, mut r) = (vec![0.0; FS as usize], vec![0.0; FS as usize]);
    s.process(
        &mut l,
        &mut r,
        ProcessContext {
            sample_rate: FS,
            tempo_bpm: 120.0,
            start_beat: 0.0,
            playing: true,
        },
    );
    l
}

/// Amplitude of the component at `f` (Hann-windowed DFT).
fn amp(x: &[f32], f: f64) -> f64 {
    let n = x.len();
    let (mut re, mut im, mut w_sum) = (0.0, 0.0, 0.0);
    for (i, &v) in x.iter().enumerate() {
        let w = 0.5 - 0.5 * (TAU * i as f64 / n as f64).cos();
        let a = TAU * f * i as f64 / FS as f64;
        re += w * v as f64 * a.cos();
        im += w * v as f64 * a.sin();
        w_sum += w;
    }
    2.0 * f64::hypot(re, im) / w_sum
}

#[test]
fn three_osc_plays_every_shape_at_its_pitch() {
    for name in SHAPES {
        let sample = shape(name).unwrap();
        assert!(Wave::custom(&sample).is_some(), "{name}");
        let out = play_a4(&sample);
        assert!(out.iter().all(|v| v.is_finite()), "{name}");
        let tail = &out[FS as usize / 2..];
        assert!(tail.iter().any(|v| v.abs() > 0.05), "{name} is silent");
        // The cycle repeats at 440 Hz: its own partials come out at 440k, with nothing at the octave below.
        let own = spectrum(&sample.left);
        for k in 1..=4 {
            let played = amp(tail, 440.0 * k as f64);
            // The instrument plays a unit sine at 0.5 (tests/three_osc.rs).
            let want = 0.5 * own[k];
            assert!(
                (played - want).abs() < 0.01 * own[loudest(&own, 64)],
                "{name} partial {k}: played {played}, stored {want}"
            );
        }
        assert!(
            amp(tail, 220.0) < 5e-3,
            "{name} has energy below its fundamental"
        );
    }
}
