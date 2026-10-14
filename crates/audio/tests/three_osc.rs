//! 3x Osc core: offline renders checked against its DSP laws.

use std::f32::consts::{FRAC_1_SQRT_2, PI};

use honeydew_audio::ProcessContext;
use honeydew_audio::instrument::{Instrument, InstrumentModule};
use honeydew_audio::sample::SampleData;
use honeydew_audio::three_osc::filter::{self, Coefs, Filter, FilterType};
use honeydew_audio::three_osc::osc::{self, Shape, Wave};
use honeydew_audio::three_osc::{
    TICK_D, ThreeOsc, coarse, gate_t, gate_ticks, max_voices, slide_t, slide_ticks, tick_samples,
};

const FS: f32 = 48_000.0;
/// 48 kHz, 120 BPM: one control tick.
const TICK: usize = 250;

fn synth(fs: f32, params: &[(usize, f32)]) -> ThreeOsc {
    let mut s = ThreeOsc::new(fs, 4);
    for &(i, v) in params {
        s.set_parameter(i, v);
    }
    s
}

/// Osc 1 alone (Mix 2 and 3 at 0), shape `shape`.
fn solo(fs: f32, shape: Shape, more: &[(usize, f32)]) -> ThreeOsc {
    let mut s = synth(
        fs,
        &[(1, shape as usize as f32 / 6.0), (22, 0.0), (23, 0.0)],
    );
    for &(i, v) in more {
        s.set_parameter(i, v);
    }
    s
}

/// The sample rate each test synth was built with is passed per call; 120 BPM, playing from beat 0.
fn context(fs: f32) -> ProcessContext {
    ProcessContext {
        sample_rate: fs,
        tempo_bpm: 120.0,
        start_beat: 0.0,
        playing: true,
    }
}

fn render(s: &mut ThreeOsc, frames: usize) -> (Vec<f32>, Vec<f32>) {
    let mut l = vec![0.0; frames];
    let mut r = vec![0.0; frames];
    s.process(&mut l, &mut r, context(FS));
    (l, r)
}

/// Frequency from rising zero crossings (linear interpolation between samples).
fn freq(x: &[f32], fs: f32) -> f32 {
    let mut crossings = Vec::new();
    for i in 1..x.len() {
        if x[i - 1] < 0.0 && x[i] >= 0.0 {
            crossings.push(i as f32 - 1.0 + x[i - 1] / (x[i - 1] - x[i]));
        }
    }
    let n = crossings.len();
    assert!(n > 2, "too few crossings");
    (n - 1) as f32 * fs / (crossings[n - 1] - crossings[0])
}

/// Amplitude of the component at `f` (Hann-windowed DFT).
fn amp(x: &[f32], fs: f32, f: f32) -> f32 {
    let n = x.len();
    let (mut re, mut im, mut wsum) = (0.0f64, 0.0f64, 0.0f64);
    for (i, &v) in x.iter().enumerate() {
        let w = 0.5 - 0.5 * (2.0 * std::f64::consts::PI * i as f64 / n as f64).cos();
        let a = 2.0 * std::f64::consts::PI * f as f64 * i as f64 / fs as f64;
        re += w * v as f64 * a.cos();
        im += w * v as f64 * a.sin();
        wsum += w;
    }
    (2.0 * (re * re + im * im).sqrt() / wsum) as f32
}

fn peak(x: &[f32]) -> f32 {
    x.iter().fold(0.0, |m, v| m.max(v.abs()))
}

fn hz(key: f32) -> f32 {
    440.0 * 2f32.powf((key - 69.0) / 12.0)
}

#[test]
fn tick_length_law() {
    assert_eq!(TICK_D, 4.0);
    assert_eq!(tick_samples(48_000.0, 120.0, TICK_D), 250);
    assert_eq!(tick_samples(44_100.0, 120.0, TICK_D), 230); // 229.6875
    assert_eq!(tick_samples(44_100.0, 140.0, TICK_D), 197); // 196.875
}

#[test]
fn note_on_waits_for_the_next_tick() {
    let mut s = solo(FS, Shape::Square, &[]);
    render(&mut s, 100); // the tick at sample 0 has passed
    s.note_on(69, 1.0, 0.5);
    let (l, _) = render(&mut s, 400);
    let first = l.iter().position(|v| *v != 0.0).unwrap();
    assert_eq!(first, TICK - 100);
}

#[test]
fn sine_plays_its_key_at_the_law_amplitude() {
    for fs in [44_100.0, 48_000.0] {
        let mut s = solo(fs, Shape::Sine, &[]);
        s.note_on(69, 1.0, 0.5);
        let (l, r) = render(&mut s, fs as usize / 2);
        assert!((freq(&l, fs) - 440.0).abs() < 0.2, "{fs}");
        // Osc pan centre × amp pan centre: cos(π/4)² = 0.5.
        assert!((peak(&l) - 0.5).abs() < 1e-3);
        assert!((peak(&r) - 0.5).abs() < 1e-3);
    }
}

#[test]
fn raw_cycles_follow_the_shape_formulas() {
    assert_eq!(osc::CYCLE, 16384);
    let tri = osc::cycle(Shape::Triangle);
    assert_eq!(
        (tri[0], tri[4096], tri[8192], tri[12288]),
        (0.0, 1.0, 0.0, -1.0)
    );
    let sq = osc::cycle(Shape::Square);
    assert_eq!(
        (sq[0], sq[8191], sq[8192], sq[16383]),
        (1.0, 1.0, -1.0, -1.0)
    );
    let saw = osc::cycle(Shape::Saw);
    assert_eq!((saw[0], saw[8192]), (-1.0, 0.0));
    let analog = osc::cycle(Shape::Analog);
    assert_eq!(analog[0], -1.0);
    let p = 0.1_f64;
    let want = 1.0 - 2.0 * 0.997_f64.powf(16384.0 * p);
    assert!((analog[1638] as f64 - (1.0 - 2.0 * 0.997_f64.powf(1638.0))).abs() < 1e-6);
    assert!(want > 0.0);
    assert_eq!(analog[(0.6 * 16384.0) as usize], 1.0);
    assert_eq!(analog[(0.9 * 16384.0) as usize], -1.0);
    // Breakpoints: the formula below 0.46387 (in f32 it reaches +1 from p ≈ 0.37 on, so the first breakpoint only
    // shows below that), +1 up to 0.83333, then −1.
    assert!(analog[5000] < 1.0); // p = 0.305: 1 − 2·0.997^5000 = 0.9999994
    assert_eq!((analog[13653], analog[13654]), (1.0, -1.0));
    let noise = osc::cycle(Shape::Noise);
    assert!(noise.iter().all(|v| (-1.0..=1.0).contains(v)));
    let mean = noise.iter().sum::<f32>() / 16384.0;
    assert!(mean.abs() < 0.03);
}

#[test]
fn shapes_have_their_harmonic_signatures() {
    let f = hz(45.0); // 110 Hz
    let spectrum = |shape| {
        let mut s = solo(FS, shape, &[]);
        s.note_on(45, 1.0, 0.5);
        let (l, _) = render(&mut s, 48_000);
        let h: Vec<f32> = (1..=4).map(|k| amp(&l, FS, k as f32 * f)).collect();
        (h[1] / h[0], h[2] / h[0], h[3] / h[0])
    };
    let (h2, h3, _) = spectrum(Shape::Sine);
    assert!(h2 < 1e-3 && h3 < 1e-3);
    let (h2, h3, h4) = spectrum(Shape::Saw);
    assert!((h2 - 0.5).abs() < 0.01 && (h3 - 1.0 / 3.0).abs() < 0.01 && (h4 - 0.25).abs() < 0.01);
    let (h2, h3, _) = spectrum(Shape::Square);
    assert!(h2 < 0.01 && (h3 - 1.0 / 3.0).abs() < 0.01);
    let (h2, h3, _) = spectrum(Shape::Triangle);
    assert!(h2 < 0.01 && (h3 - 1.0 / 9.0).abs() < 0.005);
}

#[test]
fn noise_ignores_pitch() {
    let play = |key| {
        let mut s = solo(FS, Shape::Noise, &[]);
        s.note_on(key, 1.0, 0.5);
        render(&mut s, 4096)
    };
    let a = play(30);
    assert_eq!(a, play(100));
    assert!(peak(&a.0) > 0.1);
    assert_eq!(osc::NOISE_STEP, 0.000_292_20);
}

#[test]
fn hq_mip_levels_follow_the_law() {
    assert_eq!((osc::MIP_LEVELS, osc::MIP_POINTS), (44, 32768));
    let note = |n: f32| 8.175_799 * 2f32.powf(n / 12.0);
    assert_eq!(osc::mip_level(note(0.0)), 0);
    assert_eq!(osc::mip_level(note(60.5)), 15);
    assert_eq!(osc::mip_level(note(127.0)), 37);
    assert_eq!(osc::mip_level(note(200.0)), 42);
    assert_eq!(
        (
            osc::harmonics(0),
            osc::harmonics(4),
            osc::harmonics(31),
            osc::harmonics(42)
        ),
        (1024, 512, 4, 0)
    );
}

#[test]
fn hq_removes_the_aliases_of_a_high_saw() {
    let fs = 44_100.0;
    let f = hz(108.0); // 4186 Hz: harmonic 6 folds to 44100 − 6f
    let alias = fs - 6.0 * f;
    let play = |hq: f32| {
        let mut s = solo(fs, Shape::Saw, &[(25, hq)]);
        s.note_on(108, 1.0, 0.5);
        render(&mut s, 44_100).0
    };
    let off = play(0.0);
    let on = play(1.0);
    assert!(amp(&off, fs, alias) > 0.02, "raw table aliases");
    assert!(amp(&on, fs, alias) < 1e-3, "mip level keeps 4 harmonics");
    assert!((amp(&on, fs, f) - amp(&off, fs, f)).abs() < 0.02);
}

#[test]
fn coarse_fine_and_right_only_detune() {
    let play = |params: &[(usize, f32)]| {
        let mut s = solo(FS, Shape::Sine, params);
        s.note_on(69, 1.0, 0.5);
        let (l, r) = render(&mut s, 24_000);
        (freq(&l, FS), freq(&r, FS))
    };
    let (l, _) = play(&[(13, 0.75)]); // Coarse +12
    assert!((l - 880.0).abs() < 0.3);
    let (l, _) = play(&[(16, 1.0)]); // Fine +100% = +1 st
    assert!((l - hz(70.0)).abs() < 0.3);
    let (l, r) = play(&[(10, 1.0)]); // Detune +50%: right +0.5 st
    assert!((l - 440.0).abs() < 0.2);
    assert!((r - hz(69.5)).abs() < 0.3);
}

#[test]
fn pitch_below_one_silences_the_osc() {
    let mut s = solo(FS, Shape::Square, &[]);
    s.note_on(0, 1.0, 0.5);
    assert_eq!(peak(&render(&mut s, 4000).0), 0.0);
    let mut s = solo(FS, Shape::Square, &[]);
    s.note_on(1, 1.0, 0.5);
    assert!(peak(&render(&mut s, 4000).0) > 0.1);
}

#[test]
fn mix_law_with_am_off_uses_the_defaults() {
    // Defaults: three sines at 0, −12, −24 st; Mix 2 = 50 %, Mix 3 = 25 %.
    let mut s = synth(FS, &[]);
    s.note_on(69, 1.0, 0.5);
    let (l, _) = render(&mut s, 48_000);
    let (m2, m3) = (0.5, 0.25);
    for (f, gain) in [
        (440.0, (1.0 - m3) * (1.0 - m2)),
        (220.0, (1.0 - m3) * m2),
        (110.0, m3),
    ] {
        assert!((amp(&l, FS, f) - 0.5 * gain).abs() < 2e-3, "{f}");
    }
}

#[test]
fn am_on_scales_osc_1_and_2_and_multiplies_by_osc_3() {
    // osc1 440 Hz, osc2 880 Hz (Coarse +12), osc3 1760 Hz (Coarse +24); Mix 2 = 40 %, Mix 3 = 30 %, AM on.
    let (m2, m3) = (0.4, 0.3);
    let mut s = synth(FS, &[(14, 0.75), (22, m2), (23, m3), (24, 1.0), (15, 1.0)]);
    s.note_on(69, 1.0, 0.5);
    let (l, _) = render(&mut s, 200);
    let g = (PI / 4.0).cos();
    for (k, v) in l.iter().enumerate() {
        let t = k as f32 / FS;
        let sine = |f: f32| (2.0 * PI * f * t).sin();
        let sum = (1.0 - m2) * g * sine(440.0) + m2 * g * sine(880.0);
        let want = g * sum * (1.0 - m3 * g * sine(1760.0));
        assert!((v - want).abs() < 1e-4, "{k}: {v} vs {want}");
    }
}

#[test]
fn am_on_multiplies_the_sum_by_osc_3() {
    // osc1 440 Hz, osc3 1760 Hz (Coarse +24), Mix 3 = 50 %, AM on.
    let mut s = synth(FS, &[(22, 0.0), (23, 0.5), (24, 1.0), (15, 1.0)]);
    s.note_on(69, 1.0, 0.5);
    let (l, _) = render(&mut s, 48_000);
    let g3 = 0.5 * (PI / 4.0).cos();
    assert!((amp(&l, FS, 440.0) - 0.5).abs() < 2e-3, "osc1 at (1 − m2)");
    assert!(amp(&l, FS, 1760.0) < 1e-3, "osc3 is not heard");
    for f in [1320.0, 2200.0] {
        assert!(
            (amp(&l, FS, f) - 0.5 * g3 / 2.0).abs() < 2e-3,
            "sideband {f}"
        );
    }
    // Sample by sample: 0.5·osc1·(1 − g3·osc3), both from phase 0.
    for k in [3, 17, 40, 101] {
        let t = k as f32 / FS;
        let want = 0.5 * (2.0 * PI * 440.0 * t).sin() * (1.0 - g3 * (2.0 * PI * 1760.0 * t).sin());
        assert!((l[k] - want).abs() < 1e-4, "{k}: {} vs {want}", l[k]);
    }
}

#[test]
fn invert_negates_and_osc_pan_follows_the_law() {
    let play = |params: &[(usize, f32)]| {
        let mut s = solo(FS, Shape::Saw, params);
        s.note_on(57, 0.8, 0.5);
        render(&mut s, 3000)
    };
    let (l, r) = play(&[]);
    let (il, ir) = play(&[(4, 1.0)]);
    assert!(l.iter().zip(&il).all(|(a, b)| *a == -*b));
    assert!(r.iter().zip(&ir).all(|(a, b)| *a == -*b));
    let (pl, pr) = play(&[(19, 0.0)]); // Osc 1 pan hard left
    assert_eq!(peak(&pr), 0.0);
    assert!((peak(&pl) - peak(&l) / (PI / 4.0).cos()).abs() < 1e-3);
}

#[test]
fn phase_offsets_only_the_right_channel() {
    let mut s = solo(FS, Shape::Sine, &[(7, 0.75)]); // +50 % → 0.25 cycle
    s.note_on(69, 1.0, 0.5);
    let (l, r) = render(&mut s, 10);
    assert_eq!(l[0], 0.0);
    assert!((r[0] - 0.5).abs() < 1e-4);
    // Phase Rand 100 %: the left start is random too.
    let mut s = solo(FS, Shape::Sine, &[(26, 1.0)]);
    s.note_on(69, 1.0, 0.5);
    assert_ne!(render(&mut s, 1).0[0], 0.0);
}

#[test]
fn free_run_continues_across_notes() {
    let mut s = solo(FS, Shape::Sine, &[(146, 1.0)]);
    s.note_on(69, 1.0, 0.5);
    render(&mut s, 10 * TICK);
    s.note_off(69);
    render(&mut s, 10 * TICK);
    assert_eq!(s.active_voices(), 0);
    s.note_on(69, 1.0, 0.5);
    let start = 20 * TICK;
    let (l, _) = render(&mut s, 4);
    let step = 440.0_f64 / FS as f64;
    for (i, v) in l.iter().enumerate() {
        let phase = ((start + i) as f64 * step).fract();
        let want = 0.5 * (2.0 * std::f64::consts::PI * phase).sin();
        assert!((*v as f64 - want).abs() < 1e-3, "{i}: {v} vs {want}");
    }
}

#[test]
fn filter_laws() {
    assert_eq!(filter::fast_lp_f(1.0, 44_100.0), 1.0);
    assert_eq!(filter::fast_lp_f(0.0, 44_100.0), 0.0);
    assert!((filter::fast_lp_f(0.5, 44_100.0) - (601f32.sqrt() - 1.0) / 600.0).abs() < 1e-6);
    assert!(
        (filter::fast_lp_f(0.5, 48_000.0) - (601f32.sqrt() - 1.0) / 600.0 * 0.91875).abs() < 1e-6
    );
    assert_eq!(filter::fast_lp_r(0.0), 0.0);
    assert!((filter::fast_lp_r(1.0) - (1.0 + (1.0 - 601f32.powf(0.375)) / 600.0)).abs() < 1e-6);
    assert!((filter::biquad_w0(1.0, 44_100.0) - 3.0).abs() < 1e-5);
    let w = 0.0596 * (51f32.sqrt() - 1.0) + 0.02;
    assert!((filter::biquad_w0(0.5, 44_100.0) - w).abs() < 1e-6);
    assert!((filter::biquad_w0(0.5, 48_000.0) - w * 0.91875).abs() < 1e-6);
    assert!(
        (filter::svf_f(0.5, 48_000.0) - 0.04 * (3.2581f32 * 0.5).exp_m1() * 0.91875).abs() < 1e-6
    );
    assert!((filter::biquad_q(0.5) - (0.1 + 0.4 * (2.45739f32 * 0.5).exp_m1())).abs() < 1e-6);
    assert!((filter::biquad_w0(-1.0, 44_100.0) - 0.02).abs() < 1e-6);
    assert_eq!(filter::biquad_w0(5.0, 22_050.0), 3.0);
    assert_eq!(filter::biquad_q(0.0), 0.1);
    assert!((filter::biquad_q(1.0) - 4.37).abs() < 0.01);
    assert_eq!(filter::svf_f(-0.1, 44_100.0), 0.0);
    assert!((filter::svf_f(0.5, 44_100.0) - 0.04 * (3.2581f32 * 0.5).exp_m1()).abs() < 1e-6);
    assert_eq!((filter::svf_q(0.0), filter::svf_q(1.0)), (1.0, 0.2));
    assert!((filter::svf_q(0.5) - 0.6).abs() < 1e-6);
    assert!((filter::lpx2_gain(4.37, 0.2) - (1.0 - 0.046875 * 4.37 * 3.0)).abs() < 1e-6);
}

/// Steady-state gain of `kind` at cutoff `c`, resonance `r` for a sine at `f` (44.1 kHz).
fn response(kind: FilterType, c: f32, r: f32, f: f32) -> f32 {
    let fs = 44_100.0;
    let coefs = Coefs::new(kind, c, r, fs);
    let mut flt = Filter::default();
    let fast = filter::fast_lp_f(c, fs);
    let n = 44_100;
    let out: Vec<f32> = (0..n)
        .map(|i| {
            let x = (2.0 * PI * f * i as f32 / fs).sin();
            flt.process([x, x], &coefs, fast)[0]
        })
        .collect();
    amp(&out[n / 2..], fs, f)
}

/// |H| of the RBJ cookbook biquad at ω (independent of the crate's code).
fn rbj(kind: FilterType, w0: f32, q: f32, f: f32) -> f32 {
    let w = 2.0 * std::f64::consts::PI * f as f64 / 44_100.0;
    let (s, c) = (w0 as f64).sin_cos();
    let alpha = s / (2.0 * q as f64);
    let b = match kind {
        FilterType::Hp => [(1.0 + c) / 2.0, -(1.0 + c), (1.0 + c) / 2.0],
        FilterType::Bp => [alpha, 0.0, -alpha],
        FilterType::Bs => [1.0, -2.0 * c, 1.0],
        _ => [(1.0 - c) / 2.0, 1.0 - c, (1.0 - c) / 2.0],
    };
    let a = [1.0 + alpha, -2.0 * c, 1.0 - alpha];
    let eval = |k: [f64; 3]| {
        let re = k[0] + k[1] * w.cos() + k[2] * (2.0 * w).cos();
        let im = -k[1] * w.sin() - k[2] * (2.0 * w).sin();
        (re * re + im * im).sqrt()
    };
    (eval(b) / eval(a)) as f32
}

#[test]
fn filter_types_respond_as_their_laws() {
    let fs = 44_100.0;
    for (c, r) in [(0.5, 0.0), (0.5, 0.5), (0.3, 1.0), (0.8, 0.2)] {
        let (w0, q) = (filter::biquad_w0(c, fs), filter::biquad_q(r));
        let fc = w0 * fs / (2.0 * PI);
        for kind in [
            FilterType::Lp,
            FilterType::Hp,
            FilterType::Bp,
            FilterType::Bs,
            FilterType::Lpx2,
        ] {
            for f in [fc / 8.0, fc / 2.0, fc, fc * 1.5, (fc * 4.0).min(20_000.0)] {
                let mut want = rbj(kind, w0, q, f);
                if kind == FilterType::Lpx2 {
                    want = want * want * filter::lpx2_gain(q, w0);
                }
                let got = response(kind, c, r, f);
                assert!(
                    (got - want).abs() < 0.01 * want.max(0.1),
                    "{kind:?} c {c} r {r} f {f}: {got} vs {want}"
                );
            }
        }
    }
    // BP peaks at 0 dB on ω0; BS notches it.
    let fc = filter::biquad_w0(0.5, fs) * fs / (2.0 * PI);
    assert!((response(FilterType::Bp, 0.5, 0.5, fc) - 1.0).abs() < 0.01);
    assert!(response(FilterType::Bs, 0.5, 0.5, fc) < 0.01);
    // Resonance peaks the LP near ω0.
    assert!(response(FilterType::Lp, 0.5, 1.0, fc) > 3.0);
    // SVF LP and LPx2 match a reference Chamberlin (low += f·band; band += f(x − low − q·band)), q₂ = 0.25 + 0.75q.
    for (c, r) in [(0.3, 0.0), (0.6, 0.4), (0.9, 1.0)] {
        let (f, q) = (filter::svf_f(c, fs), filter::svf_q(r));
        let stage = |s: &mut [f32; 2], q: f32, x: f32| {
            s[0] += f * s[1];
            s[1] += f * (x - s[0] - q * s[1]);
            s[0]
        };
        let (mut a, mut b1, mut b2) = ([0.0; 2], [0.0; 2], [0.0; 2]);
        let (mut one, mut two) = (Filter::default(), Filter::default());
        let (k1, k2) = (
            Coefs::new(FilterType::SvfLp, c, r, fs),
            Coefs::new(FilterType::SvfLpx2, c, r, fs),
        );
        for i in 0..2000 {
            let x = ((i * 7919) % 200) as f32 / 100.0 - 1.0;
            assert_eq!(one.process([x, x], &k1, 0.0)[0], stage(&mut a, q, x));
            let want = stage(&mut b2, 0.25 + 0.75 * q, stage(&mut b1, q, x));
            assert_eq!(two.process([x, x], &k2, 0.0)[1], want);
        }
    }
    // SVF LP: unity at low frequency, attenuates high; LPx2 more so.
    assert!((response(FilterType::SvfLp, 0.5, 0.0, 20.0) - 1.0).abs() < 0.02);
    let svf = response(FilterType::SvfLp, 0.3, 0.0, 8_000.0);
    assert!(svf < 0.2);
    assert!(response(FilterType::SvfLpx2, 0.3, 0.0, 8_000.0) < svf * 0.5);
    // Resonance raises the SVF near its corner.
    let corner = filter::svf_f(0.3, fs) * fs / (2.0 * PI);
    assert!(
        response(FilterType::SvfLp, 0.3, 1.0, corner)
            > 2.0 * response(FilterType::SvfLp, 0.3, 0.0, corner)
    );
    // Fast LP: open at c = 1, closing as c falls; resonance adds a peak.
    assert!((response(FilterType::FastLp, 1.0, 0.0, 5_000.0) - 1.0).abs() < 1e-3);
    assert!(response(FilterType::FastLp, 0.3, 0.0, 5_000.0) < 0.05);
    assert!(
        response(FilterType::FastLp, 0.5, 1.0, 200.0)
            > response(FilterType::FastLp, 0.5, 0.0, 200.0)
    );
}

#[test]
fn changing_the_filter_type_clears_its_state() {
    let fs = 44_100.0;
    let mut flt = Filter::default();
    let lp = Coefs::new(FilterType::SvfLp, 0.4, 0.5, fs);
    for _ in 0..100 {
        flt.process([1.0, 1.0], &lp, 0.0);
    }
    assert_ne!(flt.process([0.0, 0.0], &lp, 0.0), [0.0, 0.0]);
    let bp = Coefs::new(FilterType::Bp, 0.4, 0.5, fs);
    assert_eq!(flt.process([0.0, 0.0], &bp, 0.0), [0.0, 0.0]);
}

#[test]
fn the_voice_filter_follows_cutoff_and_offset() {
    // Cutoff 100 % + offset −100 % = c 0: Fast LP f = 0 silences.
    let mut s = solo(FS, Shape::Saw, &[(115, 0.0)]);
    s.note_on(57, 1.0, 0.5);
    assert!(peak(&render(&mut s, 4000).0) < 1e-6);
    // Same, but Key tracking to cutoff +100 %: (key − 60)/12 per semitone opens it again at key 72.
    let mut s = solo(FS, Shape::Saw, &[(115, 0.0), (123, 1.0)]);
    s.note_on(72, 1.0, 0.5);
    assert!(peak(&render(&mut s, 4000).0) > 0.3);
}

#[test]
fn amp_law_velocity_volume_and_amount() {
    let level = |vel: f32, params: &[(usize, f32)]| {
        let mut s = solo(FS, Shape::Square, params);
        s.note_on(57, vel, 0.5);
        peak(&render(&mut s, 2000).0)
    };
    let full = level(1.0, &[]);
    assert!((full - 0.5).abs() < 1e-4);
    assert!((level(0.5, &[]) - 0.25).abs() < 1e-4);
    assert!(
        (level(1.0, &[(114, 1.0)]) - 1.0).abs() < 1e-4,
        "Volume 200 %"
    );
    assert!(
        (level(1.0, &[(106, 0.75)]) - 0.25).abs() < 1e-4,
        "Vol env amount 2(v − 0.5)"
    );
    assert_eq!(level(1.0, &[(106, 0.25)]), 0.0, "negative gains are 0");
}

#[test]
fn pan_sums_offset_note_pan_and_tracking() {
    let play = |key: u8, vel: f32, pan: f32, params: &[(usize, f32)]| {
        let mut s = solo(FS, Shape::Square, params);
        s.note_on(key, vel, pan);
        let (l, r) = render(&mut s, 2000);
        (peak(&l), peak(&r))
    };
    let (l, _) = play(60, 1.0, 0.5, &[(113, 1.0)]); // Pan offset +100 %
    assert_eq!(l, 0.0);
    let (_, r) = play(60, 1.0, 0.0, &[]); // note pan hard left
    assert_eq!(r, 0.0);
    let (l, _) = play(72, 1.0, 0.5, &[(122, 1.0)]); // key +12 × 1/12
    assert!(l < 1e-6);
    let (_, r) = play(48, 1.0, 0.5, &[(122, 1.0)]);
    assert!(r < 1e-6);
    // Velocity tracking to pan +100 %: P = 2·(vel − 0.1(e^(2.248·0.833) − 1)).
    let (l, r) = play(60, 1.0, 0.5, &[(126, 1.0)]);
    let p = 2.0 * (1.0 - 0.1 * ((2.248f32 * 0.833).exp() - 1.0));
    let want = ((p + 1.0) * PI / 4.0).sin() / ((p + 1.0) * PI / 4.0).cos();
    assert!((r / l - want).abs() < 1e-3 * want);
}

#[test]
fn release_ramps_at_the_slope_limit() {
    for fs in [44_100.0_f32, 48_000.0] {
        let tick = tick_samples(fs, 120.0, TICK_D) as usize;
        let mut s = solo(fs, Shape::Square, &[]);
        s.note_on(57, 1.0, 0.5);
        render(&mut s, 4 * tick);
        s.note_off(57);
        let (l, _) = render(&mut s, 6000);
        // Released at the next tick (4·tick): the amp gain 0.7071 falls by 44.1/fs per sample.
        let slope = 44.1 / fs;
        let gain = |k: usize| (FRAC_1_SQRT_2 - slope * (k as f32 + 1.0)).max(0.0);
        for k in [0_usize, 100, 300, 600] {
            let window = &l[k..k + 3];
            let want = gain(k) * FRAC_1_SQRT_2;
            assert!(
                (peak(window) - want).abs() < 2e-3,
                "{fs} {k}: {} vs {want}",
                peak(window)
            );
        }
        // The last tick's ramp lands exactly on 0 at its end (slower than the limit), then the voice is freed.
        let end = (FRAC_1_SQRT_2 / slope) as usize + tick + 2;
        assert!(l[end..].iter().all(|v| *v == 0.0));
        assert!(peak(&l[..end - 40]) > 0.0);
    }
}

#[test]
fn polyphony_cuts_the_most_recent_held_voice() {
    assert_eq!(
        (max_voices(0.0), max_voices(1.0), max_voices(2.0 / 99.0)),
        (256, 99, 2)
    );
    let mut s = solo(FS, Shape::Sine, &[(117, 2.0 / 99.0)]);
    for key in [60, 64, 67] {
        s.note_on(key, 1.0, 0.5);
        render(&mut s, TICK);
    }
    render(&mut s, 3000);
    assert_eq!(s.active_voices(), 2);
    let (l, _) = render(&mut s, 24_000);
    assert!(amp(&l, FS, hz(60.0)) > 0.4);
    assert!(amp(&l, FS, hz(64.0)) < 1e-3, "64 was cut");
    assert!(amp(&l, FS, hz(67.0)) > 0.4);
}

#[test]
fn polyphony_cuts_a_released_voice_first() {
    let mut s = solo(FS, Shape::Sine, &[(117, 2.0 / 99.0)]);
    s.note_on(60, 1.0, 0.5);
    s.note_on(64, 1.0, 0.5);
    render(&mut s, TICK);
    s.note_off(60);
    s.note_on(67, 1.0, 0.5);
    render(&mut s, 3000);
    let (l, _) = render(&mut s, 24_000);
    assert!(amp(&l, FS, hz(64.0)) > 0.4);
    assert!(amp(&l, FS, hz(67.0)) > 0.4);
    assert_eq!(s.active_voices(), 2);
}

#[test]
fn re_pressing_a_key_releases_its_old_voice() {
    let mut s = solo(FS, Shape::Sine, &[]);
    s.note_on(60, 1.0, 0.5);
    render(&mut s, TICK);
    s.note_on(60, 1.0, 0.5);
    render(&mut s, 3000);
    assert_eq!(s.active_voices(), 1);
}

#[test]
fn slide_and_gate_laws() {
    assert_eq!(slide_t(0.494), 24); // 1/8 beat
    assert_eq!(slide_t(1.0), 384); // 2 beats
    assert_eq!(slide_ticks(0.494, TICK_D), 12.0);
    assert_eq!(gate_t(0.0), 6);
    assert_eq!(gate_ticks(0.0, TICK_D), 3);
    assert_eq!(gate_ticks(0.999, TICK_D), 96); // t = 192: one beat
    // Odd t: 2t/D = 3.5 rounds up; D = 3 gives 14/3 → 5.
    assert_eq!(gate_t(0.0372), 7);
    assert_eq!(gate_ticks(0.0372, 4.0), 4);
    assert_eq!(gate_ticks(0.0372, 3.0), 5);
}

#[test]
fn gate_releases_the_note_after_its_time() {
    let mut s = solo(FS, Shape::Square, &[(129, 0.0)]); // 3 ticks
    s.note_on(57, 1.0, 0.5);
    let (l, _) = render(&mut s, 3 * TICK + 2000);
    assert!(
        (peak(&l[..3 * TICK]) - 0.5).abs() < 1e-4,
        "held for 3 ticks"
    );
    assert!(
        (peak(&l[3 * TICK - 10..3 * TICK]) - 0.5).abs() < 1e-4,
        "up to the third tick"
    );
    assert!(
        peak(&l[3 * TICK + 10..3 * TICK + 20]) < 0.5,
        "then released"
    );
    assert_eq!(s.active_voices(), 0);
}

#[test]
fn mono_glides_one_voice_without_retrigger() {
    // Slide 100 %: 384/192 beat = 2 beats = 192 ticks.
    let mut s = solo(FS, Shape::Sine, &[(120, 1.0), (118, 1.0)]);
    s.note_on(60, 1.0, 0.5);
    render(&mut s, 4 * TICK);
    s.note_on(72, 1.0, 0.5);
    let (l, _) = render(&mut s, 210 * TICK);
    assert_eq!(s.active_voices(), 1);
    let near = |ticks: usize, key: f32| {
        let f = freq(&l[(ticks - 6) * TICK..(ticks + 6) * TICK], FS);
        assert!(
            (f / hz(key) - 1.0).abs() < 0.01,
            "{ticks}: {f} vs {}",
            hz(key)
        );
    };
    near(96, 66.0);
    near(200, 72.0);
    // Amplitude never dips: no retrigger.
    assert!(l[TICK..].chunks(TICK).all(|c| peak(c) > 0.45));
    // Releasing 72 glides back to the held 60.
    s.note_off(72);
    let (l, _) = render(&mut s, 200 * TICK);
    let f = freq(&l[190 * TICK..199 * TICK], FS);
    assert!((f / hz(60.0) - 1.0).abs() < 0.01);
}

#[test]
fn porta_glides_each_new_note_unless_echo_feeds_back() {
    let start = |feedback: f32| {
        let mut s = solo(FS, Shape::Sine, &[(119, 1.0), (118, 1.0), (131, feedback)]);
        s.note_on(60, 1.0, 0.5);
        render(&mut s, TICK);
        s.note_off(60);
        render(&mut s, 8 * TICK);
        s.note_on(72, 1.0, 0.5);
        let (l, _) = render(&mut s, 4 * TICK);
        freq(&l, FS)
    };
    assert!(
        (start(0.0) / hz(60.0) - 1.0).abs() < 0.01,
        "starts from the previous note"
    );
    assert!(
        (start(0.5) / hz(72.0) - 1.0).abs() < 0.002,
        "Echo Feedback > 0 skips Porta"
    );
}

#[test]
fn custom_shape_without_a_file_plays_silence() {
    // A fresh instance's custom table is all zeros, HQ mips too: no Saw fallback.
    let play = |s: &mut ThreeOsc| {
        s.note_on(57, 1.0, 0.5);
        render(s, 3000)
    };
    for hq in [0.0, 1.0] {
        let (l, r) = play(&mut solo(FS, Shape::Custom, &[(25, hq)]));
        assert!(l.iter().chain(&r).all(|&v| v == 0.0), "HQ {hq}");
    }
    assert!(osc::cycle(Shape::Custom).iter().all(|&v| v == 0.0));
    let (saw, _) = play(&mut solo(FS, Shape::Saw, &[]));
    assert!(saw.iter().any(|v| v.abs() > 0.1), "Saw still sounds");
    // A one-cycle sine file (1000 frames, stereo) becomes a sine cycle.
    let cycle: Vec<f32> = (0..1000)
        .map(|i| (2.0 * PI * i as f32 / 1000.0).sin())
        .collect();
    let sample = SampleData {
        sample_rate: 44_100,
        left: cycle.clone().into(),
        right: Some(cycle.into()),
        meta: Default::default(),
    };
    let mut s = ThreeOsc::with_custom_shape(FS, 4, Some(&sample));
    for (i, v) in [(1, 1.0), (22, 0.0), (23, 0.0)] {
        s.set_parameter(i, v);
    }
    s.note_on(45, 1.0, 0.5);
    let (l, _) = render(&mut s, 48_000);
    let f = hz(45.0);
    assert!((amp(&l, FS, f) - 0.5).abs() < 5e-3);
    assert!(amp(&l, FS, 2.0 * f) < 1e-3);
}

#[test]
fn block_size_does_not_change_the_output() {
    let run = |blocks: &[usize]| {
        let mut s = synth(
            FS,
            &[
                (110, 3.0 / 7.0),
                (111, 0.6),
                (112, 0.4),
                (26, 0.5),
                (25, 1.0),
            ],
        );
        let (mut l, mut r) = (Vec::new(), Vec::new());
        let mut i = 0;
        // Events land every 3000 samples; blocks are cut at those points so both runs see them at the same frame.
        for segment in 0..10 {
            match segment {
                0 => s.note_on(60, 0.9, 0.3),
                3 => s.note_on(67, 0.7, 0.8),
                5 => s.note_off(60),
                7 => s.note_off(67),
                _ => {}
            }
            let mut left = 3000;
            while left > 0 {
                let n = blocks[i % blocks.len()].min(left);
                let (bl, br) = render(&mut s, n);
                l.extend(bl);
                r.extend(br);
                left -= n;
                i += 1;
            }
        }
        (l, r)
    };
    assert_eq!(run(&[3000]), run(&[1, 7, 64, 333, 1023, 1572]));
}

#[test]
fn reset_silences_at_once() {
    let mut s = synth(FS, &[]);
    s.note_on(60, 1.0, 0.5);
    render(&mut s, 1000);
    s.reset();
    assert_eq!(s.active_voices(), 0);
    assert_eq!(peak(&render(&mut s, 1000).0), 0.0);
}

#[test]
fn d_scales_the_control_tick() {
    // D = 8: 500 samples per tick at 48 kHz, 120 BPM; D = 0 counts as 1.
    for (d, tick) in [(8, 500), (0, 63)] {
        let mut s = ThreeOsc::new(FS, d);
        s.set_parameter(1, Shape::Square as usize as f32 / 6.0);
        render(&mut s, 1);
        s.note_on(69, 1.0, 0.5);
        let (l, _) = render(&mut s, 1000);
        assert_eq!(l.iter().position(|v| *v != 0.0), Some(tick - 1), "D {d}");
    }
}

#[test]
fn a_frozen_instrument_cannot_drop_a_note_off() {
    // Enable off: notes queue without process(). The held key's off and 128 short notes must not overflow it.
    let mut s = ThreeOsc::new(FS, 4);
    s.note_on(50, 1.0, 0.5);
    render(&mut s, TICK);
    s.note_off(50);
    for k in 60..188u8 {
        s.note_on(k, 1.0, 0.5);
        s.note_off(k);
    }
    render(&mut s, 48_000);
    assert_eq!(s.active_voices(), 0, "stuck voice");
}

#[test]
fn pitch_bend_is_semitones() {
    let mut s = solo(FS, Shape::Sine, &[]);
    s.set_controller(4, 0.75); // w = 4v − 2 = +1 st
    s.note_on(69, 1.0, 0.5);
    let (l, _) = render(&mut s, 24_000);
    assert!((freq(&l, FS) - hz(70.0)).abs() < 0.3);
}

#[test]
fn hq_reads_the_raw_cycle_at_small_steps() {
    // Key 18 at 48 kHz: 23.1 Hz, a step of 0.00048 cycle/sample, under the 0.0005 threshold.
    let play = |hq: f32| {
        let mut s = solo(FS, Shape::Saw, &[(25, hq)]);
        s.note_on(18, 1.0, 0.5);
        render(&mut s, 4000)
    };
    assert_eq!(play(1.0), play(0.0));
    // Key 19 (24.5 Hz, step 0.00051) takes the mip level.
    let play = |hq: f32| {
        let mut s = solo(FS, Shape::Saw, &[(25, hq)]);
        s.note_on(19, 1.0, 0.5);
        render(&mut s, 4000)
    };
    assert_ne!(play(1.0), play(0.0));
}

#[test]
fn fast_lp_closes_at_the_slew_limit() {
    let mut s = solo(FS, Shape::Saw, &[]);
    s.note_on(57, 1.0, 0.5);
    render(&mut s, 4 * TICK);
    s.set_parameter(111, 0.0); // c 1 → 0: f falls by 44.1/fs per sample (≥ 1088 samples)
    let (l, _) = render(&mut s, 2000);
    let moving = |range: std::ops::Range<usize>| range.into_iter().any(|i| l[i + 1] != l[i]);
    assert!(moving(900..1000), "still open after 900 samples");
    assert!(!moving(1300..1999), "closed: f = 0 holds y");
}

#[test]
fn resonance_offset_and_tracking_raise_the_fast_lp_peak() {
    let level = |key: u8, params: &[(usize, f32)]| {
        let mut base = vec![(111, 0.45)];
        base.extend_from_slice(params);
        let mut s = solo(FS, Shape::Saw, &base);
        s.note_on(key, 1.0, 0.5);
        peak(&render(&mut s, 24_000).0[12_000..])
    };
    let flat = level(72, &[]);
    assert!(
        level(72, &[(116, 1.0)]) > 1.5 * flat,
        "Resonance offset +100 %"
    );
    assert!(
        level(72, &[(124, 1.0)]) > 1.5 * flat,
        "key tracking to resonance"
    );
    assert!(
        level(72, &[(128, 1.0)]) > 1.5 * flat,
        "velocity tracking to resonance"
    );
}

#[test]
fn all_notes_off_releases_every_voice() {
    let mut s = solo(FS, Shape::Sine, &[]);
    for key in [60, 64, 67] {
        s.note_on(key, 1.0, 0.5);
    }
    render(&mut s, 2 * TICK);
    s.all_notes_off();
    let (l, _) = render(&mut s, 3000);
    assert!(peak(&l[..TICK]) > 0.5, "the releases still sound");
    assert_eq!(s.active_voices(), 0);
    assert_eq!(peak(&l[2500..]), 0.0);
}

#[test]
fn mono_release_glides_back_to_the_most_recent_held_key() {
    let mut s = solo(FS, Shape::Sine, &[(120, 1.0), (118, 0.0)]); // Slide 0: glides jump
    for key in [60, 64, 67] {
        s.note_on(key, 1.0, 0.5);
        render(&mut s, TICK);
    }
    s.note_off(67);
    let (l, _) = render(&mut s, 24_000);
    assert!((freq(&l[TICK..], FS) / hz(64.0) - 1.0).abs() < 0.002);
}

#[test]
fn mono_gate_restarts_on_each_key() {
    let mut s = solo(FS, Shape::Square, &[(120, 1.0), (118, 0.0), (129, 0.0)]); // Gate: 3 ticks
    s.note_on(60, 1.0, 0.5);
    render(&mut s, 2 * TICK);
    s.note_on(64, 1.0, 0.5);
    let (l, _) = render(&mut s, 4 * TICK);
    assert!(
        (peak(&l[2 * TICK..3 * TICK]) - 0.5).abs() < 1e-4,
        "not released 3 ticks after the first key"
    );
    assert!(
        peak(&l[3 * TICK + 10..4 * TICK]) < 0.5,
        "released 3 ticks after the second"
    );
}

#[test]
fn mono_starts_a_fresh_voice_after_a_release() {
    let mut s = solo(FS, Shape::Square, &[(120, 1.0), (118, 1.0)]);
    s.note_on(60, 1.0, 0.5);
    render(&mut s, TICK);
    s.note_off(60);
    render(&mut s, TICK);
    s.note_on(64, 1.0, 0.5);
    let (l, _) = render(&mut s, 12 * TICK);
    // The released 60 is cut and gone by tick 5; 64 sounds at full level and pitch, without a glide.
    assert_eq!(s.active_voices(), 1);
    assert!(
        (peak(&l[5 * TICK..]) - 0.5).abs() < 1e-4,
        "a fresh, held voice"
    );
    assert!(
        (freq(&l[5 * TICK..], FS) / hz(64.0) - 1.0).abs() < 0.005,
        "no glide"
    );
}

#[test]
fn cut_voices_do_not_count_against_the_limit() {
    // Limit 2. 64 is cut by 67 (a quiet note); 67 is then released and freed while 64 is still ramping down. 69
    // fits without cutting 60.
    let mut s = solo(FS, Shape::Sine, &[(117, 2.0 / 99.0)]);
    s.note_on(60, 1.0, 0.5);
    s.note_on(64, 1.0, 0.5);
    render(&mut s, TICK);
    s.note_on(67, 0.1, 0.5);
    render(&mut s, TICK);
    s.note_off(67);
    render(&mut s, 2 * TICK);
    s.note_on(69, 1.0, 0.5);
    render(&mut s, 3000);
    let (l, _) = render(&mut s, 24_000);
    assert!(amp(&l, FS, hz(60.0)) > 0.4);
    assert!(amp(&l, FS, hz(69.0)) > 0.4);
    assert_eq!(s.active_voices(), 2);
}

#[test]
fn a_cut_voice_is_not_cut_again() {
    // Limit 2: 67 cuts 64; then 69 must cut 67 (the most recent held), not the already-cut 64.
    let mut s = solo(FS, Shape::Sine, &[(117, 2.0 / 99.0)]);
    s.note_on(60, 1.0, 0.5);
    s.note_on(64, 1.0, 0.5);
    render(&mut s, TICK);
    s.note_on(67, 1.0, 0.5);
    render(&mut s, TICK);
    s.note_on(69, 1.0, 0.5);
    render(&mut s, 3000);
    assert_eq!(s.active_voices(), 2);
    let (l, _) = render(&mut s, 24_000);
    assert!(amp(&l, FS, hz(67.0)) < 1e-3);
}

#[test]
fn porta_glides_pitch_velocity_and_pan() {
    // Level and pan carry the Porta glide too. Slide 100 %: 192 ticks.
    let mut s = solo(FS, Shape::Square, &[(119, 1.0), (118, 1.0)]);
    s.note_on(60, 1.0, 0.0); // full velocity, hard left
    render(&mut s, TICK);
    s.note_off(60);
    render(&mut s, 8 * TICK);
    s.note_on(72, 0.5, 1.0); // half velocity, hard right
    let (l, r) = render(&mut s, 200 * TICK);
    let at = |k: usize| {
        (
            peak(&l[k * TICK..(k + 1) * TICK]),
            peak(&r[k * TICK..(k + 1) * TICK]),
        )
    };
    let (l0, r0) = at(0);
    assert!(
        (l0 - FRAC_1_SQRT_2).abs() < 1e-4 && r0 == 0.0,
        "starts at the previous note: {l0} {r0}"
    );
    let (l96, r96) = at(96);
    assert!(
        (l96 - 0.375).abs() < 0.01 && (r96 - 0.375).abs() < 0.01,
        "halfway: velocity 0.75, centred: {l96} {r96}"
    );
    let (l, r) = at(195);
    assert!(
        l < 1e-6 && (r - 0.5 * FRAC_1_SQRT_2).abs() < 1e-4,
        "then the new note's: {l} {r}"
    );
}

#[test]
fn phase_rand_draws_up_to_its_value() {
    // Saw: L[0] = 0.5·(2u − 1), so the left start phase u = L[0] + 0.5 must lie in [0, 0.25).
    let mut s = solo(FS, Shape::Saw, &[(26, 0.25)]);
    let mut most = 0.0_f32;
    for _ in 0..40 {
        s.reset();
        s.note_on(57, 1.0, 0.5);
        let u = render(&mut s, 1).0[0] + 0.5;
        assert!((0.0..0.25).contains(&u), "{u}");
        most = most.max(u);
    }
    assert!(most > 0.15);
}

#[test]
fn custom_shape_downsamples_without_aliasing() {
    // 40000 frames: one sine cycle, harmonic 5000 (kept: under the 16384-point cycle's Nyquist, 8192) and harmonic
    // 12000 (removed: above it).
    let n = 40_000;
    let wave = |p: f64, h: f64| (2.0 * std::f64::consts::PI * h * p).sin();
    let frames: Vec<f32> = (0..n)
        .map(|i| {
            let p = i as f64 / n as f64;
            (wave(p, 1.0) + 0.5 * wave(p, 5000.0) + 0.5 * wave(p, 12_000.0)) as f32
        })
        .collect();
    let sample = SampleData {
        sample_rate: 44_100,
        left: frames.into(),
        right: None,
        meta: Default::default(),
    };
    let custom = Wave::custom(&sample).unwrap();
    assert_eq!(custom.raw().len(), osc::CYCLE);
    for (i, v) in custom.raw().iter().enumerate() {
        let p = i as f64 / osc::CYCLE as f64;
        let want = (wave(p, 1.0) + 0.5 * wave(p, 5000.0)) as f32;
        assert!((v - want).abs() < 0.01, "{i}: {v} vs {want}");
    }
}

#[test]
fn custom_shape_reads_the_first_65536_frames_channels_averaged_into_16384_points() {
    // "the first 65 536 frames, channels averaged, resampled to 16 384 points".
    // One sine cycle over the first 65536 frames, split ±0.5 between the channels; past them, a constant that would
    // show if it were read.
    let n = osc::CUSTOM_MAX_FRAMES;
    let sine = |i: usize| (2.0 * std::f64::consts::PI * i as f64 / n as f64).sin() as f32;
    let channel = |offset: f32| -> std::sync::Arc<[f32]> {
        (0..2 * n)
            .map(|i| if i < n { sine(i) + offset } else { 1.0 })
            .collect()
    };
    let sample = SampleData {
        sample_rate: 44_100,
        left: channel(0.5),
        right: Some(channel(-0.5)),
        meta: Default::default(),
    };
    let custom = Wave::custom(&sample).unwrap();
    assert_eq!(custom.raw().len(), 16_384);
    for (j, v) in custom.raw().iter().enumerate() {
        let want = sine(j * n / 16_384);
        assert!((v - want).abs() < 0.01, "{j}: {v} vs {want}");
    }
}

#[test]
fn a_note_inside_one_tick_gap_plays_its_release() {
    // Vol env on, Attack 0 (starts at 1), Release 0.3: an on/off pair between two ticks queues both keys, and the
    // note sounds with its release tail.
    let mut s = synth(FS, &[(62, 1.0), (74, 0.0), (98, 0.3)]);
    render(&mut s, 10);
    s.note_on(69, 1.0, 0.5);
    s.note_off(69);
    let (l, _) = render(&mut s, 40 * TICK);
    let energy = |range: std::ops::Range<usize>| l[range].iter().map(|v| v * v).sum::<f32>();
    assert!(energy(0..2 * TICK) > 0.0, "the note sounds");
    assert!(energy(10 * TICK..12 * TICK) > 0.0, "with a release tail");
    assert!(energy(10 * TICK..12 * TICK) < energy(0..2 * TICK));
}

#[test]
fn enable_off_freezes_the_voices_and_on_resumes_them() {
    // 3x Osc is not reset; it picks up where it stopped.
    let module = || {
        let mut m = InstrumentModule::new(Box::new(synth(FS, &[])), 512);
        m.instrument.note_on(60, 1.0, 0.5);
        m
    };
    let run = |m: &mut InstrumentModule, frames: usize| {
        let (mut l, mut r) = (vec![0.0; frames], vec![0.0; frames]);
        m.process(&mut l, &mut r, context(FS));
        (l, r)
    };
    let mut steady = module();
    run(&mut steady, 2000);
    let expected = run(&mut steady, 2000);
    let mut toggled = module();
    run(&mut toggled, 2000);
    toggled.set_parameter(0, 0.0);
    assert_eq!(peak(&run(&mut toggled, 2000).0), 0.0, "skipped while off");
    toggled.set_parameter(0, 1.0);
    let resumed = run(&mut toggled, 2000);
    assert!(peak(&resumed.0) > 0.01, "silent after On");
    assert_eq!(resumed, expected);
}

// ---------------------------------------------------------------------------------------------------------------
// Filter state on trigger, the key-tracking pitch, level against velocity.

/// The largest sample-to-sample step.
fn max_step(x: &[f32]) -> f32 {
    x.windows(2).fold(0.0, |m, w| m.max((w[1] - w[0]).abs()))
}

#[test]
fn a_new_voice_starts_with_clear_filter_state_for_every_type() {
    for kind in 0..8 {
        let params = [(110, kind as f32 / 7.0), (111, 0.4), (112, 0.6)];
        let mut fresh = solo(FS, Shape::Saw, &params);
        fresh.note_on(64, 1.0, 0.5);
        let want = render(&mut fresh, 2000).0;
        // The same note in the slot a released note has just left, its filter still holding state.
        let mut used = solo(FS, Shape::Saw, &params);
        used.note_on(57, 1.0, 0.5);
        render(&mut used, 20 * TICK);
        used.note_off(57);
        while used.active_voices() > 0 {
            render(&mut used, TICK);
        }
        used.note_on(64, 1.0, 0.5);
        assert!(render(&mut used, 2000).0 == want, "type {kind}");
    }
}

#[test]
fn the_fast_lp_f_starts_at_its_target_and_slews_from_the_second_tick() {
    // c 1: f = 1 from the first sample; a slewed start would stay near silent for ~1000 samples.
    let mut s = solo(FS, Shape::Square, &[]);
    s.note_on(57, 1.0, 0.5);
    let (l, _) = render(&mut s, TICK);
    assert!((peak(&l[..20]) - 0.5).abs() < 1e-4);
    // c 1 → 0 on the second tick: f falls by 44.1/fs per sample (≥ 1088 samples).
    s.set_parameter(111, 0.0);
    let (l, _) = render(&mut s, 2000);
    let moving = |range: std::ops::Range<usize>| range.into_iter().any(|i| l[i + 1] != l[i]);
    assert!(moving(900..1000), "still open after 900 samples");
    assert!(!moving(1300..1999), "closed");
}

/// A slow square (key 25, 34.6 Hz) through a slow Fast LP (c 0.5: f ≈ 0.036): y settles at ±0.707 between edges, so
/// clearing it steps the output by ≈ 0.5, while an edge moves it by at most ≈ 0.04 per sample.
const SETTLED: [(usize, f32); 1] = [(111, 0.5)];

#[test]
fn mono_legato_keeps_the_filter_state_and_slews() {
    let mut s = solo(FS, Shape::Square, &[(120, 1.0), (118, 0.0), SETTLED[0]]);
    s.note_on(25, 1.0, 0.5);
    let (mut l, _) = render(&mut s, 10 * TICK);
    s.note_on(37, 1.0, 0.5);
    l.extend(render(&mut s, 10 * TICK).0);
    assert_eq!(s.active_voices(), 1);
    assert!(max_step(&l) < 0.1, "{}", max_step(&l));
    // Key tracking to cutoff, c = (key − 60)/12: legato 72 → 60 slews f from 1 to 0 instead of jumping.
    let mut s = solo(
        FS,
        Shape::Saw,
        &[(120, 1.0), (118, 0.0), (115, 0.0), (123, 1.0)],
    );
    s.note_on(72, 1.0, 0.5);
    render(&mut s, 4 * TICK);
    s.note_on(60, 1.0, 0.5);
    let (l, _) = render(&mut s, 2000);
    let moving = |range: std::ops::Range<usize>| range.into_iter().any(|i| l[i + 1] != l[i]);
    assert!(moving(900..1000), "still open after 900 samples");
    assert!(!moving(1300..1999), "closed");
}

#[test]
fn an_arp_slide_step_keeps_the_filter_state() {
    // An arp Slide is not a trigger. Up, Gate 50 %, Range 2, root only, Slide on.
    let arp = [
        SETTLED[0],
        (139, 0.2),
        (141, 0.5),
        (142, 2.0 / 6.0),
        (144, 1.0),
        (145, 2.0 / 90.0),
    ];
    let mut s = solo(FS, Shape::Square, &arp);
    s.note_on(25, 1.0, 0.5);
    render(&mut s, TICK);
    let (l, _) = render(&mut s, 100 * TICK);
    assert_eq!(s.active_voices(), 1);
    assert!(max_step(&l) < 0.1, "{}", max_step(&l));
    // Gate 100 %: the Slide to step 2 (tick 24) jumps the pitch 60 → 72. Key tracking to cutoff −100 % takes c from
    // 1 to 0, and f slews down (≥ 1088 samples) rather than jumping as on a trigger.
    let mut s = solo(
        FS,
        Shape::Saw,
        &[
            (123, 0.0),
            (139, 0.2),
            (142, 2.0 / 6.0),
            (144, 1.0),
            (145, 2.0 / 90.0),
        ],
    );
    s.note_on(60, 1.0, 0.5);
    let (l, _) = render(&mut s, 40 * TICK);
    let moving = |range: std::ops::Range<usize>| range.into_iter().any(|i| l[i + 1] != l[i]);
    let step = 24 * TICK;
    assert!(
        moving(step + 900..step + 1000),
        "still open 900 samples after the Slide"
    );
    assert!(!moving(step + 1300..step + 2000), "closed");
}

#[test]
fn an_arp_slide_sets_portas_previous_note() {
    // Porta glides from the last note started or glided to. Arp Up, Range 2, root only, Slide on,
    // Gate 100 %: 60, then a Slide to 72 at tick 24. Porta 100 % (192 ticks).
    let mut s = solo(
        FS,
        Shape::Sine,
        &[
            (119, 1.0),
            (118, 1.0),
            (139, 0.2),
            (142, 2.0 / 6.0),
            (144, 1.0),
            (145, 2.0 / 90.0),
        ],
    );
    s.note_on(60, 1.0, 0.5);
    render(&mut s, 30 * TICK);
    s.note_off(60);
    render(&mut s, 8 * TICK);
    s.note_on(60, 1.0, 0.5);
    let (l, _) = render(&mut s, 4 * TICK);
    let f = freq(&l, FS);
    assert!((f / hz(72.0) - 1.0).abs() < 0.02, "glides from 72: {f}");
}

#[test]
fn the_key_tracking_pitch_leaves_out_osc_offsets_and_pitch_mods() {
    // Key 60 at Key Middle 50 % (60): the key term stays 0 whatever the oscs' offsets or the Pitch env/LFO do.
    let cases: [&[(usize, f32)]; 5] = [
        &[(13, 0.75)],                                  // osc 1 Coarse +12
        &[(16, 1.0)],                                   // Fine +1
        &[(10, 1.0)],                                   // Detune +0.5 (right)
        &[(65, 1.0), (77, 0.0), (109, 0.75)],           // Pitch env on, Attack 0, Amount +12
        &[(41, 1.0), (51, 0.0), (61, 1.0), (56, 0.75)], // Pitch LFO Square (+1 for 32 s), Attack 0, +12
    ];
    let play = |case: &[(usize, f32)], tracking: &[(usize, f32)], wheel: f32| {
        let mut s = solo(FS, Shape::Saw, case);
        for &(i, v) in tracking {
            s.set_parameter(i, v);
        }
        s.set_controller(4, wheel);
        s.note_on(60, 1.0, 0.5);
        let (l, r) = render(&mut s, 4 * TICK);
        (peak(&l[2 * TICK..]), peak(&r[2 * TICK..]))
    };
    for case in cases {
        let (l, r) = play(case, &[(122, 1.0)], 0.5); // key tracking to pan +100 %
        assert!(
            (r / l - 1.0).abs() < 0.02,
            "{case:?}: pan stays centred: {l} {r}"
        );
        let shut = play(case, &[(115, 0.0), (123, 1.0)], 0.5); // to cutoff +100 %, c 0 otherwise
        assert_eq!(shut, (0.0, 0.0), "{case:?}: the filter stays shut");
        // To resonance +100 %, r 0 otherwise: no resonance, the same as without the tracking.
        assert_eq!(
            play(case, &[(111, 0.45), (124, 1.0)], 0.5),
            play(case, &[(111, 0.45)], 0.5),
            "{case:?}: the resonance stays 0"
        );
    }
    // The wheel is in: +2 st, P = 2/12.
    let (l, r) = play(&[], &[(122, 1.0)], 1.0);
    let want = ((2.0 / 12.0 + 1.0) * PI / 4.0).tan();
    assert!((r / l - want).abs() < 0.02 * want, "{} vs {want}", r / l);
    // The Porta glide is in: 48 → 72 over 192 ticks passes 60 (centred) at tick 96 and ends hard right.
    let mut s = solo(FS, Shape::Saw, &[(119, 1.0), (118, 1.0), (122, 1.0)]);
    s.note_on(48, 1.0, 0.5);
    render(&mut s, TICK);
    s.note_off(48);
    render(&mut s, 8 * TICK);
    s.note_on(72, 1.0, 0.5);
    let (l, r) = render(&mut s, 200 * TICK);
    let at = |k: usize| {
        (
            peak(&l[k * TICK..(k + 1) * TICK]),
            peak(&r[k * TICK..(k + 1) * TICK]),
        )
    };
    let (l96, r96) = at(96);
    assert!((r96 / l96 - 1.0).abs() < 0.02, "tick 96: {l96} {r96}");
    let (l195, r195) = at(195);
    assert!(l195 == 0.0 && r195 > 0.3, "tick 195: {l195} {r195}");
    // The echo pitch is in: Pitch +12 per echo (Feedback 100 %, one echo 24 ticks on) pans the echo hard right.
    let mut s = solo(
        FS,
        Shape::Saw,
        &[(122, 1.0), (130, 0.0), (131, 0.5), (132, 0.125), (134, 1.0)],
    );
    s.note_on(60, 1.0, 0.5);
    let (l, r) = render(&mut s, 10 * TICK);
    s.note_off(60);
    let (el, er) = render(&mut s, 30 * TICK);
    let (l, r) = (peak(&l[2 * TICK..]), peak(&r[2 * TICK..]));
    assert!((r / l - 1.0).abs() < 0.02, "output is centred: {l} {r}");
    let (el, er) = (
        peak(&el[16 * TICK..22 * TICK]),
        peak(&er[16 * TICK..22 * TICK]),
    );
    assert!(el == 0.0 && er > 0.3, "the echo is hard right: {el} {er}");
}

#[test]
fn level_is_unclamped_in_velocity_and_ignores_velocity_tracking() {
    // Echo Feedback 200 %: the one echo (0.25 beat = 24 ticks on) plays at velocity 1.5, twice the played note's level.
    let mut s = solo(FS, Shape::Square, &[(130, 0.0), (131, 1.0), (132, 0.125)]);
    s.note_on(57, 0.75, 0.5);
    let (l, _) = render(&mut s, 10 * TICK);
    s.note_off(57);
    let (echo, _) = render(&mut s, 30 * TICK);
    assert!((peak(&l[2 * TICK..]) - 0.375).abs() < 1e-4);
    assert!((peak(&echo[16 * TICK..22 * TICK]) - 0.75).abs() < 1e-4);
    // Velocity tracking (Middle 100 %: term 0.153) to pan and cutoff moves the image, not the level.
    let mut s = solo(FS, Shape::Square, &[(125, 1.0), (126, 1.0), (127, 1.0)]);
    s.note_on(57, 1.0, 0.5);
    let (l, r) = render(&mut s, 2 * TICK);
    let (l, r) = (peak(&l), peak(&r));
    assert!(r > 1.2 * l, "panned right: {l} {r}");
    assert!((l.hypot(r) - FRAC_1_SQRT_2).abs() < 1e-4, "{}", l.hypot(r));
}

// ---------------------------------------------------------------------------------------------------------------
// Toggles on at v > 0.5. The Coarse step.

/// Saw oscs through Cutoff 50 % at 100 BPM (Tempo sync changes the time laws; at 120 BPM K is 49.152 either way; one
/// tick = 300 samples): 3 ticks, then 60, 20 ticks later 67 (Mono and Porta glide), both up 20 ticks later, 30 ticks of
/// tails. Returns L then R.
fn toggle_run(set: &[(usize, f32)]) -> Vec<f32> {
    let mut s = synth(FS, &[(1, 0.5), (2, 0.5), (3, 0.5), (111, 0.5)]);
    for &(i, v) in set {
        s.set_parameter(i, v);
    }
    let context = ProcessContext {
        tempo_bpm: 100.0,
        ..context(FS)
    };
    let mut out = Vec::new();
    let mut run = |s: &mut ThreeOsc, ticks: usize| {
        let (mut l, mut r) = (vec![0.0; 300 * ticks], vec![0.0; 300 * ticks]);
        s.process(&mut l, &mut r, context);
        out.extend(l.into_iter().chain(r));
    };
    run(&mut s, 3); // Free run and the Global LFO phases move on before the first note.
    s.note_on(60, 0.8, 0.5);
    run(&mut s, 20);
    s.note_on(67, 0.8, 0.5);
    run(&mut s, 20);
    s.note_off(60);
    s.note_off(67);
    run(&mut s, 30);
    out
}

#[test]
fn every_toggle_is_off_at_one_half() {
    // The 3x Osc toggles are on at v > 0.5.
    let mut toggles: Vec<(usize, Vec<(usize, f32)>)> = vec![
        (4, vec![]),   // osc 1 Invert
        (5, vec![]),   // osc 2 Invert
        (6, vec![]),   // osc 3 Invert
        (24, vec![]),  // AM
        (25, vec![]),  // HQ
        (146, vec![]), // osc 1 Free run
        (147, vec![]), // osc 2 Free run
        (148, vec![]), // osc 3 Free run
        (62, vec![]),  // Vol env Enable
        (63, vec![(107, 1.0)]),
        (64, vec![(108, 1.0)]),
        (65, vec![(109, 1.0)]),
        (119, vec![]),                        // Porta
        (120, vec![]),                        // Mono
        (120, vec![(131, 0.4), (132, 0.05)]), // Mono turns Echo off (echoes every 0.1 beat)
        (
            137,
            vec![(130, 3.0 / 9.0), (131, 0.4), (132, 0.05), (133, 1.0)],
        ), // Ping Pong
        (138, vec![(131, 0.4)]),              // Fat Mode
        (
            144,
            vec![(139, 0.2), (141, 0.5), (142, 2.0 / 6.0), (145, 2.0 / 90.0)],
        ), // arp Slide
    ];
    for t in 0..5 {
        // LFO Global and Tempo, with that LFO's Amount at +100 %.
        toggles.push((27 + t, vec![(52 + t, 1.0)]));
        toggles.push((32 + t, vec![(52 + t, 1.0)]));
    }
    for i in 0..4 {
        // Env Tempo, with that env on, Attack 30 % and (but for Vol) its Amount at +100 %.
        let mut extra = vec![(62 + i, 1.0), (74 + i, 0.3)];
        if i > 0 {
            extra.push((106 + i, 1.0));
        }
        toggles.push((66 + i, extra));
    }
    assert_eq!(toggles.len(), 32);
    for (toggle, extra) in toggles {
        let at = |v: f32| {
            let mut set = extra.clone();
            set.push((toggle, v));
            toggle_run(&set)
        };
        let off = at(0.0);
        assert!(at(1.0) != off, "p{toggle}: on changes the output");
        assert!(at(0.5) == off, "p{toggle}: 0.5 is off");
    }
}

#[test]
fn coarse_is_the_fused_step_rounded_half_away_from_zero() {
    // Round half away from zero of fma_f32(v, 48, −24); the knob's v = f32(k/48) gives k − 24 at every k.
    for k in 0..=48 {
        assert_eq!(
            coarse((f64::from(k) / 48.0) as f32),
            k as f32 - 24.0,
            "k {k}"
        );
    }
    assert_eq!(coarse(0.59375), 5.0, "4.5 rounds away from zero");
    assert_eq!(coarse(0.40625), -5.0, "−4.5 too");
    // One rounding: the fused value is 2.499999 (−0.4999995); a multiply then a subtract gives 2.5 (−0.5) instead.
    let v = f32::from_bits(1_057_838_421);
    assert_eq!(coarse(v), 2.0);
    assert_eq!(coarse(f32::from_bits(1_056_615_083)), 0.0);
    // The voice uses it: osc 1 at key 57 plays 59.
    let mut s = solo(FS, Shape::Sine, &[(13, v)]);
    s.note_on(57, 1.0, 0.5);
    let f = freq(&render(&mut s, 24_000).0, FS);
    assert!((f / hz(59.0) - 1.0).abs() < 0.002, "{f}");
}

#[test]
fn a_wave_reports_the_bytes_of_its_tables() {
    // The app counts a Custom table against its memory budget (`sample_cache`): the raw cycle and 44 mip levels of
    // 32768 points, `f32`. A tiny file makes the same table.
    let sample = SampleData {
        sample_rate: 44_100,
        left: vec![0.5; 8].into(),
        right: None,
        meta: Default::default(),
    };
    let custom = Wave::custom(&sample).unwrap();
    assert_eq!(
        custom.byte_len(),
        4 * (osc::CYCLE + osc::MIP_LEVELS * osc::MIP_POINTS)
    );
}
