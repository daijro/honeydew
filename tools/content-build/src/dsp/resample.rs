//! Band-limited sample-rate conversion, including 48 kHz sources to 44.1 kHz.

/// Taps a side at a 1:1 ratio, widened by the ratio when decimating so the transition band keeps its width at the
/// output rate.
const HALF: f64 = 64.0;
/// Kaiser β: about 100 dB of stopband.
const BETA: f64 = 10.0;
/// The cutoff, as a share of the lower rate's Nyquist. At 48 → 44.1 kHz the passband is flat past 19 kHz and the
/// stopband starts below 21.5 kHz.
const CUTOFF: f64 = 0.92;

/// `x` at `from` Hz resampled to `to` Hz through a Kaiser-windowed sinc, in f64, with a tap table for each phase the
/// rational ratio visits (each normalised to unity gain at DC). The output has ⌈len · to / from⌉ frames; the input is
/// zero outside its ends.
// ponytail: one table row per phase (to / gcd of the rates), so an odd rate like 44 101 Hz costs ~45 MB; add a
// sub-sampled table with interpolation if a source ever has one.
pub fn resample(x: &[f32], from: u32, to: u32) -> Vec<f32> {
    if from == to {
        return x.to_vec();
    }
    let gcd = gcd(from, to);
    // Output frame n sits at input position n · down / up.
    let (up, down) = ((to / gcd) as usize, (from / gcd) as usize);
    let stretch = (f64::from(from) / f64::from(to)).max(1.0);
    let half = (HALF * stretch).ceil() as usize;
    let fc = 0.5 * CUTOFF / stretch; // cycles per input frame
    let taps = 2 * half;
    // Row p, tap k: input frame i0 − half + 1 + k for an output at i0 + p / up.
    let mut table = vec![0.0; up * taps];
    for (p, row) in table.chunks_exact_mut(taps).enumerate() {
        for (k, h) in row.iter_mut().enumerate() {
            let t = k as f64 + 1.0 - half as f64 - p as f64 / up as f64;
            *h = 2.0 * fc * sinc(2.0 * fc * t) * kaiser(t / half as f64);
        }
        let sum: f64 = row.iter().sum();
        row.iter_mut().for_each(|h| *h /= sum);
    }
    let padded: Vec<f64> = (0..half)
        .map(|_| 0.0)
        .chain(x.iter().map(|&s| f64::from(s)))
        .chain((0..half).map(|_| 0.0))
        .collect();
    let len = (x.len() * up).div_ceil(down);
    (0..len)
        .map(|n| {
            let (i0, p) = ((n * down) / up, (n * down) % up);
            let row = &table[p * taps..][..taps];
            let window = &padded[i0 + 1..][..taps];
            row.iter().zip(window).map(|(h, s)| h * s).sum::<f64>() as f32
        })
        .collect()
}

fn gcd(a: u32, b: u32) -> u32 {
    if b == 0 { a } else { gcd(b, a % b) }
}

fn sinc(x: f64) -> f64 {
    if x == 0.0 {
        1.0
    } else {
        (std::f64::consts::PI * x).sin() / (std::f64::consts::PI * x)
    }
}

/// The Kaiser window at `r` in [−1, 1].
fn kaiser(r: f64) -> f64 {
    bessel_i0(BETA * (1.0 - r * r).max(0.0).sqrt()) / bessel_i0(BETA)
}

/// The modified Bessel function I₀, by its power series.
fn bessel_i0(x: f64) -> f64 {
    let (mut sum, mut term, mut k) = (1.0, 1.0, 1.0);
    while term > 1e-17 * sum {
        term *= (x / (2.0 * k)).powi(2);
        sum += term;
        k += 1.0;
    }
    sum
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::f64::consts::TAU;

    #[test]
    fn flat_to_18_khz_and_23_khz_rejected_by_80_db() {
        // A unit tone at 48 kHz, resampled to 44.1 kHz: its level in dB, measured away from the ends.
        let level = |f: f64| {
            let x: Vec<f32> = (0..48_000)
                .map(|i| (TAU * f * f64::from(i) / 48_000.0).sin() as f32)
                .collect();
            let y = resample(&x, 48_000, 44_100);
            assert_eq!(y.len(), 44_100);
            let mid = &y[4_410..y.len() - 4_410];
            let power = mid.iter().map(|&s| f64::from(s).powi(2)).sum::<f64>() / mid.len() as f64;
            10.0 * (2.0 * power).log10()
        };
        for f in [100.0, 1_000.0, 5_000.0, 10_000.0, 15_000.0, 18_000.0] {
            let db = level(f);
            assert!(db.abs() < 0.1, "{f} Hz: {db} dB");
        }
        let db = level(23_000.0);
        assert!(db < -80.0, "23 kHz: {db} dB");
    }
}
