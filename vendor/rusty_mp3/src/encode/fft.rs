//! Brick **Q2 (core)** — a radix-2 Cooley–Tukey FFT, the psychoacoustic model's
//! analysis transform. Pure in-house, no dependencies. Scalar Rust; this is the
//! single biggest cycle sink in a perceptual encoder and is flagged as a SIMD
//! hotspot (`lab::bricks::accel` → SIMD), but the scalar path is the reference the
//! SIMD twin will be checked against.

use std::cell::RefCell;
use std::f32::consts::PI;

/// Every stage's twiddle factors for one FFT size, laid out end to end: stage
/// `len` occupies `[len/2 - 1, len - 1)`, so the whole set is exactly `n - 1`
/// entries.
///
/// The values depend only on `n`, so recomputing them per call was pure waste:
/// the psymodel calls [`fft`] four times per frame at a fixed `N_FFT`, which had
/// it rebuilding the same table — including two `cos`/`sin` per stage, ~1.26 M
/// transcendental calls across a 7-minute track for **ten distinct values** —
/// and heap-allocating two zero-filled buffers each time, only to overwrite
/// every element before reading it.
///
/// Built by the *same recurrence* the inline version used, so the cached values
/// are bit-for-bit what the old code produced; `hoisted_matches_inline` pins
/// that against `fft_inline_ref`.
struct Twiddles {
    n: usize,
    re: Vec<f32>,
    im: Vec<f32>,
}

impl Twiddles {
    fn build(n: usize) -> Self {
        let mut re = vec![0f32; n.saturating_sub(1)];
        let mut im = vec![0f32; n.saturating_sub(1)];
        let mut len = 2usize;
        while len <= n {
            let ang = -2.0 * PI / len as f32;
            let (wr, wi) = (ang.cos(), ang.sin());
            let half = len / 2;
            let off = half - 1;
            re[off] = 1.0;
            im[off] = 0.0;
            for k in 1..half {
                // `re[off + k - 1]` is still the previous iteration's value when
                // the second line reads it — matching the original ordering.
                re[off + k] = re[off + k - 1] * wr - im[off + k - 1] * wi;
                im[off + k] = re[off + k - 1] * wi + im[off + k - 1] * wr;
            }
            len <<= 1;
        }
        Self { n, re, im }
    }
}

thread_local! {
    /// One table per thread, rebuilt only when the transform size changes — which
    /// in the encoder it never does after the first call. Thread-local rather than
    /// a `OnceLock` keyed by size so arbitrary `n` still works (the tests sweep
    /// several) with no lock on the hot path.
    static TWIDDLES: RefCell<Twiddles> = const {
        RefCell::new(Twiddles { n: 0, re: Vec::new(), im: Vec::new() })
    };
}

/// In-place complex FFT. `re`/`im` are the real/imaginary parts; `re.len()` must
/// be a power of two and equal to `im.len()`. Forward transform (`exp(-i2πkn/N)`).
///
/// # Panics
/// If `re.len()` is not a power of two, or differs from `im.len()`.
pub fn fft(re: &mut [f32], im: &mut [f32]) {
    let n = re.len();
    assert!(n.is_power_of_two(), "FFT length must be a power of two");
    assert_eq!(n, im.len());

    // Bit-reversal permutation.
    let mut j = 0usize;
    for i in 1..n {
        let mut bit = n >> 1;
        while j & bit != 0 {
            j ^= bit;
            bit >>= 1;
        }
        // Mask by `n - 1`. `n` is a power of two (asserted above) and `j` is built
        // from bits of `n >> 1` downward, so it is already below `n` and the mask is
        // the identity -- but `x & (n - 1)` is provably `< n` to the compiler, and
        // without it each swap bounds-checks both of its indices, twice over.
        j |= bit;
        let j = j & (n - 1);
        if i < j {
            re.swap(i, j);
            im.swap(i, j);
        }
    }

    // Butterfly stages.
    //
    // **SIMD Floor 1 / 1A:** the per-stage twiddle factors are precomputed once (via
    // the same recurrence that used to run inline) into `tw_r`/`tw_i`, instead of
    // being advanced by a serial recurrence *inside* the butterfly loop. That data
    // dependency (`cr,ci` carried k→k+1) blocked vectorization; hoisting it makes the
    // inner `k` loop independent and contiguous. Byte-identical (same recurrence
    // values, just computed once and reused across base-blocks) — pinned by
    // `hoisted_matches_inline`.
    TWIDDLES.with(|cache| {
        let mut cache = cache.borrow_mut();
        if cache.n != n {
            *cache = Twiddles::build(n);
        }
        let mut len = 2usize;
        while len <= n {
            let half = len / 2;
            // Slice this stage's span once, so the inner loop indexes from 0 and
            // stays as tight as when the table was per-stage.
            let off = half - 1;
            let tw_r = &cache.re[off..off + half];
            let tw_i = &cache.im[off..off + half];
            let mut base = 0;
            while base < n {
                for k in 0..half {
                    let a = base + k;
                    let b = a + half;
                    let (cr, ci) = (tw_r[k], tw_i[k]);
                    let tr = cr * re[b] - ci * im[b];
                    let ti = cr * im[b] + ci * re[b];
                    re[b] = re[a] - tr;
                    im[b] = im[a] - ti;
                    re[a] += tr;
                    im[a] += ti;
                }
                base += len;
            }
            len <<= 1;
        }
    });
}

/// Power spectrum `|X[k]|²` of a real signal windowed into `re` (with `im`
/// zeroed), for the first `n/2 + 1` bins. Convenience for the psymodel.
pub fn power_spectrum(re: &mut [f32], im: &mut [f32], out: &mut [f32]) {
    fft(re, im);
    for (k, o) in out.iter_mut().enumerate() {
        *o = re[k] * re[k] + im[k] * im[k];
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Naive DFT reference.
    fn dft(re: &[f32], im: &[f32]) -> (Vec<f32>, Vec<f32>) {
        let n = re.len();
        let mut or = vec![0f32; n];
        let mut oi = vec![0f32; n];
        for k in 0..n {
            for t in 0..n {
                let ang = -2.0 * PI * (k * t) as f32 / n as f32;
                or[k] += re[t] * ang.cos() - im[t] * ang.sin();
                oi[k] += re[t] * ang.sin() + im[t] * ang.cos();
            }
        }
        (or, oi)
    }

    /// The pre-1A FFT: twiddle advanced by a serial recurrence inside the loop.
    /// Kept as the bit-identity oracle for the hoisted version.
    fn fft_inline_ref(re: &mut [f32], im: &mut [f32]) {
        let n = re.len();
        let mut j = 0usize;
        for i in 1..n {
            let mut bit = n >> 1;
            while j & bit != 0 {
                j ^= bit;
                bit >>= 1;
            }
            j |= bit;
            if i < j {
                re.swap(i, j);
                im.swap(i, j);
            }
        }
        let mut len = 2usize;
        while len <= n {
            let ang = -2.0 * PI / len as f32;
            let (wr, wi) = (ang.cos(), ang.sin());
            let half = len / 2;
            let mut base = 0;
            while base < n {
                let (mut cr, mut ci) = (1.0f32, 0.0f32);
                for k in 0..half {
                    let a = base + k;
                    let b = a + half;
                    let tr = cr * re[b] - ci * im[b];
                    let ti = cr * im[b] + ci * re[b];
                    re[b] = re[a] - tr;
                    im[b] = im[a] - ti;
                    re[a] += tr;
                    im[a] += ti;
                    let ncr = cr * wr - ci * wi;
                    ci = cr * wi + ci * wr;
                    cr = ncr;
                }
                base += len;
            }
            len <<= 1;
        }
    }

    /// **1A gate.** Hoisting the twiddle recurrence must be BIT-identical to the old
    /// inline recurrence — the same float values, just computed once and reused.
    #[test]
    fn hoisted_matches_inline() {
        let mut s = 0xC0FF_EE11u32;
        let mut rng = || {
            s = s.wrapping_mul(1_664_525).wrapping_add(1_013_904_223);
            (s >> 8) as f32 / (1u32 << 24) as f32 - 0.5
        };
        for &n in &[2usize, 4, 8, 16, 64, 256, 1024] {
            let re0: Vec<f32> = (0..n).map(|_| rng()).collect();
            let im0: Vec<f32> = (0..n).map(|_| rng()).collect();
            let (mut ra, mut ia) = (re0.clone(), im0.clone());
            let (mut rb, mut ib) = (re0.clone(), im0.clone());
            fft(&mut ra, &mut ia);
            fft_inline_ref(&mut rb, &mut ib);
            assert_eq!(ra, rb, "re mismatch at n={n}");
            assert_eq!(ia, ib, "im mismatch at n={n}");
        }
    }

    #[test]
    fn matches_naive_dft() {
        let n = 64;
        let mut s = 0x1234_5678u32;
        let mut re: Vec<f32> = (0..n)
            .map(|_| {
                s = s.wrapping_mul(1_664_525).wrapping_add(1_013_904_223);
                (s >> 8) as f32 / (1u32 << 24) as f32 - 0.5
            })
            .collect();
        let mut im = vec![0f32; n];
        let (rr, ri) = dft(&re, &im);
        fft(&mut re, &mut im);
        for k in 0..n {
            assert!(
                (re[k] - rr[k]).abs() < 1e-3,
                "bin {k} re: {} vs {}",
                re[k],
                rr[k]
            );
            assert!((im[k] - ri[k]).abs() < 1e-3, "bin {k} im");
        }
    }

    #[test]
    fn pure_tone_concentrates_in_one_bin() {
        let n = 256;
        let bin = 8;
        let mut re: Vec<f32> = (0..n)
            .map(|t| (2.0 * PI * bin as f32 * t as f32 / n as f32).cos())
            .collect();
        let mut im = vec![0f32; n];
        fft(&mut re, &mut im);
        let mag: Vec<f32> = (0..n)
            .map(|k| (re[k] * re[k] + im[k] * im[k]).sqrt())
            .collect();
        // Energy at bin and its mirror; ~zero elsewhere.
        assert!(mag[bin] > 100.0, "tone bin weak: {}", mag[bin]);
        for k in 0..n {
            if k != bin && k != n - bin {
                assert!(mag[k] < 1.0, "leak at bin {k}: {}", mag[k]);
            }
        }
    }

    #[test]
    fn parseval_energy_conserved() {
        let n = 128;
        let mut s = 99u32;
        let orig: Vec<f32> = (0..n)
            .map(|_| {
                s = s.wrapping_mul(1_664_525).wrapping_add(1_013_904_223);
                (s >> 8) as f32 / (1u32 << 24) as f32 - 0.5
            })
            .collect();
        let mut re = orig.clone();
        let mut im = vec![0f32; n];
        fft(&mut re, &mut im);
        let time_energy: f32 = orig.iter().map(|x| x * x).sum();
        let freq_energy: f32 =
            (0..n).map(|k| re[k] * re[k] + im[k] * im[k]).sum::<f32>() / n as f32;
        assert!(
            (time_energy - freq_energy).abs() < 1e-2,
            "{time_energy} vs {freq_energy}"
        );
    }
}
