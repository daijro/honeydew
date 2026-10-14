use honeydew_audio::gm_waveshapes::{
    PARTIALS, SAW, SINE, SLOTS, SQUARE, Spectrum, TABLE_LEN, TABLES, TRIANGLE, Wavetable, read,
    spectrum, table_index_for,
};
use std::f32::consts::PI;
use std::f64::consts::TAU;

fn near(actual: f32, expected: f32, tolerance: f32) {
    assert!(
        (actual - expected).abs() <= tolerance,
        "{actual} != {expected}"
    );
}

fn shape(slot: usize) -> Spectrum {
    spectrum(slot).expect("a menu slot")
}

/// A uniform 22-bit dither source for the render tests (xorshift32).
fn dither(state: &mut u32) -> u32 {
    *state ^= *state << 13;
    *state ^= *state >> 17;
    *state ^= *state << 5;
    *state >> 10
}

#[test]
fn math_slots_follow_content_md() {
    let (sine, saw, square, triangle) = (shape(SINE), shape(SAW), shape(SQUARE), shape(TRIANGLE));
    assert_eq!(sine.mag[0], 0.5, "sine amplitude is 0.5 at partial 1");
    assert!(sine.mag[1..].iter().all(|&m| m == 0.0));
    for n in 1..=PARTIALS {
        let (nf, odd) = (n as f32, n % 2 == 1);
        near(saw.mag[n - 1], 1.0 / nf, 1e-7);
        near(square.mag[n - 1], if odd { 1.0 / nf } else { 0.0 }, 1e-7);
        near(
            triangle.mag[n - 1],
            if odd { 1.0 / (nf * nf) } else { 0.0 },
            1e-7,
        );
    }

    // In time, through the cos-phase tables at full band (no normalization): a 0.5 sine, a falling saw (π − θ)/2,
    // a square with a π/4 plateau and a triangle peaking at π²/8 a quarter cycle in.
    let full = TABLES - 1;
    let sine = Wavetable::build(&sine);
    for n in (0..TABLE_LEN).step_by(16) {
        let theta = TAU as f32 * n as f32 / TABLE_LEN as f32;
        near(sine.table(full)[n], 0.5 * theta.sin(), 1e-6);
    }
    near(Wavetable::build(&saw).table(full)[256], PI / 4.0, 0.01);
    let square = Wavetable::build(&square);
    near(square.table(full)[256], PI / 4.0, 0.01);
    near(square.table(full)[768], -PI / 4.0, 0.01);
    let triangle = Wavetable::build(&triangle);
    near(triangle.table(full)[0], 0.0, 1e-5);
    near(triangle.table(full)[256], PI * PI / 8.0, 0.002);
    near(triangle.table(full)[768], -PI * PI / 8.0, 0.002);
}

#[test]
fn every_slot_is_its_own_shape() {
    let all: Vec<_> = (0..SLOTS).map(shape).collect();
    for (slot, s) in all.iter().enumerate() {
        assert!(
            s.mag.iter().all(|m| m.is_finite() && *m >= 0.0),
            "slot {slot}"
        );
        assert!(s.mag.iter().any(|&m| m > 0.0), "slot {slot} is silent");
        assert!(
            s.phase.iter().all(|p| (-PI..=PI).contains(p)),
            "slot {slot}"
        );
        if ![SAW, SINE, SQUARE, TRIANGLE].contains(&slot) {
            let rms = (s.mag.iter().map(|m| m * m).sum::<f32>() / 2.0).sqrt();
            near(rms, PI / 4.0, 1e-4);
        }
        for (other, t) in all.iter().enumerate().skip(slot + 1) {
            assert_ne!(s, t, "slots {slot} and {other} are the same shape");
        }
    }
    assert_eq!(spectrum(SLOTS), None, "past the menu is a missing shape");
}

#[test]
fn tables_follow_the_spec_formula() {
    for slot in [SAW, 4, 13] {
        let s = shape(slot);
        let tables = Wavetable::build(&s);
        for h_count in [1, 2, 512] {
            let table = tables.table(h_count - 1);
            for (n, &v) in table.iter().enumerate() {
                let expected: f64 = (1..=h_count)
                    .map(|h| {
                        let angle =
                            f64::from(s.phase[h - 1]) + TAU * ((h * n) % TABLE_LEN) as f64 / 1024.0;
                        f64::from(s.mag[h - 1]) * angle.cos()
                    })
                    .sum();
                near(v, expected as f32, 1e-5);
            }
        }
    }
}

#[test]
fn table_choice_follows_the_h_rule() {
    // (sample rate, frequency, H)
    let cases = [
        (44_100.0, 0.0, 512),
        (44_100.0, 20.0, 512),
        (44_100.0, 44_100.0 / 1024.0, 512),
        (44_100.0, 440.0, 51),
        (44_100.0, 4186.009, 6),
        (44_100.0, 22_050.0, 2),
        (44_100.0, 30_000.0, 1),
        (48_000.0, 440.0, 55),
        // ⌊48000/2000⌋ + 1 = 25: harmonic 25 (25 kHz) sits above Nyquist, on purpose.
        (48_000.0, 1000.0, 25),
        (96_000.0, 100.0, 481),
    ];
    for (rate, freq, h) in cases {
        assert_eq!(table_index_for(freq, rate), h - 1, "{freq} Hz at {rate} Hz");
    }
    assert_eq!(
        table_index_for(-440.0, 48_000.0),
        table_index_for(440.0, 48_000.0)
    );
    // sr ≤ 0 or a non-finite input gives table 0 (H = 1), whatever the frequency.
    for (freq, rate) in [
        (f32::NAN, 48_000.0),
        (440.0, f32::NAN),
        (f32::INFINITY, 48_000.0),
        (440.0, 0.0),
        (440.0, -48_000.0),
        (0.0, 0.0),
        (0.0, -48_000.0),
        (1.0, f32::INFINITY),
        (0.0, f32::NAN),
    ] {
        assert_eq!(table_index_for(freq, rate), 0, "{freq} Hz at {rate} Hz");
    }
    assert_eq!(table_index_for(1e-30, 48_000.0), TABLES - 1);
}

#[test]
fn double_spike_flips_negative_magnitudes_by_pi() {
    // Slot 2 is drawn as 2·cos(0.35πn)/(1 + (n/10)²) at sine phase π/2 − 0.35πn: a signed magnitude. Stored, a
    // negative one becomes |m| at its cos-phase + π, so mag·e^{iφ} = m·e^{i(π/2 − 0.35πn − π/2)}, up to the design
    // scale.
    let s = shape(2);
    let signed = |n: usize| {
        let nf = n as f64;
        let m = 2.0 * (std::f64::consts::PI * nf * 0.35).cos() / (1.0 + (nf / 10.0).powi(2));
        (m, -std::f64::consts::PI * nf * 0.35)
    };
    assert!(signed(2).0 < 0.0, "partial 2 is drawn negative");
    let scale = f64::from(s.mag[0]) / signed(1).0;
    for n in 1..=PARTIALS {
        let (m, angle) = signed(n);
        let (mag, phase) = (f64::from(s.mag[n - 1]), f64::from(s.phase[n - 1]));
        let tolerance = 1e-5 * scale.max(1.0);
        assert!(
            (mag * phase.cos() - scale * m * angle.cos()).abs() < tolerance,
            "partial {n}"
        );
        assert!(
            (mag * phase.sin() - scale * m * angle.sin()).abs() < tolerance,
            "partial {n}"
        );
    }
}

#[test]
fn lookup_takes_the_top_ten_bits_after_dither() {
    let tables = Wavetable::build(&shape(4));
    assert!(std::ptr::eq(tables.table(9999), tables.table(TABLES - 1)));
    let table = tables.table(TABLES - 1);
    for k in [0_u32, 1, 511, 1023] {
        assert_eq!(read(table, k << 22, 0), table[k as usize]);
    }
    assert_eq!(read(table, (5 << 22) | 0x3F_FFFF, 1), table[6]);
    // The index stays in 0..1023: it wraps instead of running off the table, and dither bits above 22 are ignored.
    assert_eq!(read(table, u32::MAX, u32::MAX), table[0]);
    for (phase, dither) in [
        (0, 0),
        (u32::MAX, 0),
        (0, u32::MAX),
        (0x8000_0000, 0x0012_3456),
    ] {
        assert_eq!(
            read(table, phase, dither),
            read(table, phase, dither & 0x3F_FFFF)
        );
    }
    // A uniform 22-bit dither averages to linear interpolation: a quarter step past sample 7.
    let phase = (7 << 22) + (1 << 20);
    let mean = (0..1024_u32)
        .map(|j| read(table, phase, j << 12))
        .sum::<f32>()
        / 1024.0;
    near(mean, 0.75 * table[7] + 0.25 * table[8], 1e-5);
}

#[test]
fn sine_plays_in_tune() {
    let (rate, freq) = (48_000.0_f32, 440.0_f32);
    let tables = Wavetable::build(&shape(SINE));
    let table = tables.table(table_index_for(freq, rate));
    let increment = (f64::from(freq / rate) * 4_294_967_296.0).round() as u32;
    let (mut phase, mut state) = (0_u32, 0x1234_5678_u32);
    let out: Vec<f32> = (0..48_000)
        .map(|_| {
            let v = read(table, phase, dither(&mut state));
            phase = phase.wrapping_add(increment);
            v
        })
        .collect();
    let rising = out.windows(2).filter(|w| w[0] < 0.0 && w[1] >= 0.0).count();
    assert!(
        (439..=441).contains(&rising),
        "{rising} cycles in one second"
    );
}
