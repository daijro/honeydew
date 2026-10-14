// Modified from rusty_mp3 1.0.0 (see Cargo.toml).
//! Brick **L1** — the analysis polyphase filterbank, the inverse of decode's
//! synthesis stage (`decode/synthesis.rs`).
//!
//! Splits 32 PCM samples into 32 subband samples per pass (18 passes per
//! granule). Each pass shifts 32 new samples into a 512-tap FIFO, windows it with
//! the analysis window `C[]`, folds 512→64, then a 32×64 cosine matrix produces
//! the 32 subband outputs.
//!
//! Two ISO facts make this exact rather than guessed:
//! * The analysis window is the synthesis window scaled: `C[i] = D[i] / 32`
//!   (ISO/IEC 11172-3) — so we derive it from the already-sourced [`SYNTH_D`]
//!   rather than transcribing a second table.
//! * The analysis matrix `M[k][i] = cos((2k+1)(i−16)·π/64)` is the cosine-modulated
//!   partner of the decoder's synthesis matrix `N[i][k] = cos((16+i)(2k+1)·π/64)`.
//!   The two together are a (near-)perfect-reconstruction pseudo-QMF pair, which
//!   is exactly the round-trip the test exercises.

use std::f64::consts::PI;
use std::sync::OnceLock;

use crate::decode::synth_window::SYNTH_D;
use crate::frame::{SUBBANDS, SUBBAND_LINES};

/// Analysis window `C[i] = D[i] / 32` (ISO identity vs. the synthesis window).
fn window() -> &'static [f32; 512] {
    static C: OnceLock<[f32; 512]> = OnceLock::new();
    C.get_or_init(|| {
        let mut c = [0f32; 512];
        for (i, ci) in c.iter_mut().enumerate() {
            *ci = SYNTH_D[i] / 32.0;
        }
        c
    })
}

/// Analysis matrixing `M[k][i] = cos((2k+1)(i−16)·π/64)`, 32×64.
fn matrix() -> &'static [[f32; 64]; SUBBANDS] {
    static M: OnceLock<[[f32; 64]; SUBBANDS]> = OnceLock::new();
    M.get_or_init(|| {
        let mut m = [[0f32; 64]; SUBBANDS];
        for (k, row) in m.iter_mut().enumerate() {
            for (i, c) in row.iter_mut().enumerate() {
                *c = (PI / 64.0 * (2 * k + 1) as f64 * (i as f64 - 16.0)).cos() as f32;
            }
        }
        m
    })
}

/// `M` transposed, `[i][k]`: the lane-per-output layout the SIMD matrix twins
/// read, so each of the 32 outputs accumulates in its own lane with no
/// horizontal reduction -- written as 32 dot products the compiler leaves it
/// scalar (66 scalar / 2 packed float ops in the emitted asm).
fn matrix_t() -> &'static [[f32; SUBBANDS]; 64] {
    static T: OnceLock<[[f32; SUBBANDS]; 64]> = OnceLock::new();
    T.get_or_init(|| {
        let m = matrix();
        let mut t = [[0f32; SUBBANDS]; 64];
        for (k, row) in m.iter().enumerate() {
            for (i, &v) in row.iter().enumerate() {
                t[i][k] = v;
            }
        }
        t
    })
}

/// Window + fold 512 → 64: `Y[i] = Σ_{j=0..7} C[i+64j]·X[i+64j]`, each output
/// accumulating its eight taps in `j` order (`0.0 + p0 == p0`, so starting at
/// zero is exact). The scalar ORACLE for [`fold_simd`]. `head` must be a
/// multiple of 32 so every 32-run is contiguous inside the circular FIFO.
#[inline]
fn fold_scalar(c: &[f32; 512], fifo: &[f32; 512], head: usize) -> [f32; 64] {
    let mut y = [0f32; 64];
    for (h, yh) in y.chunks_exact_mut(32).enumerate() {
        for j in 0..8 {
            let at = (head + 32 * h + 64 * j) & 511;
            let (cs, xs) = (&c[32 * h + 64 * j..][..32], &fifo[at..at + 32]);
            for i in 0..32 {
                yh[i] += cs[i] * xs[i];
            }
        }
    }
    y
}

/// Matrix 64 → 32: `S[k] = Σ_{i=0..63} M[k][i]·Y[i]`. The scalar ORACLE for
/// [`matrix_simd`].
#[inline]
fn matrix_scalar(m: &[[f32; 64]; SUBBANDS], y: &[f32; 64]) -> [f32; SUBBANDS] {
    let mut s = [0f32; SUBBANDS];
    for k in 0..SUBBANDS {
        let mut acc = 0f32;
        for i in 0..64 {
            acc += m[k][i] * y[i];
        }
        s[k] = acc;
    }
    s
}

/// AVX twin of [`fold_scalar`]: output index = lane, eight accumulators, taps in
/// the original `j` order, separate mul + add (no FMA) -- bit-identical.
///
/// # Safety
/// AVX must be available. All memory bounds are established inside (fixed-size
/// arrays; `head` is masked to a multiple of 32).
#[cfg(target_arch = "x86_64")]
#[target_feature(enable = "avx")]
unsafe fn fold_avx(c: &[f32; 512], fifo: &[f32; 512], head: usize) -> [f32; 64] {
    use std::arch::x86_64::*;
    // `analyze` only ever passes a multiple of 32; masking makes the bounds below
    // hold for ANY `head`, so the kernel's soundness is local (one `and`).
    let head = head & !31;
    // SAFETY: a register constant -- no memory access; the ISA is this fn's contract. Redundant from Rust 1.86 (safe target-feature calls), required at the crate's MSRV (1.85).
    #[allow(unused_unsafe)]
    let mut acc = [unsafe { _mm256_setzero_ps() }; 8];
    for j in 0..8 {
        for h in 0..2 {
            // `at` is a multiple of 32 below 512, so `at + 32 <= 512`.
            let at = (head + 32 * h + 64 * j) & 511;
            for q in 0..4 {
                // SAFETY: `c` offsets reach `32 + 448 + 24 + 8 = 512`, its length; `head` is masked to a multiple of 32 above, so `at` is a multiple of 32 below 512 and `at + 8 * q + 8 <= at + 32 <= 512`.
                unsafe {
                    let cv = _mm256_loadu_ps(c.as_ptr().add(32 * h + 64 * j + 8 * q));
                    let xv = _mm256_loadu_ps(fifo.as_ptr().add(at + 8 * q));
                    let a = &mut acc[4 * h + q];
                    *a = _mm256_add_ps(*a, _mm256_mul_ps(cv, xv));
                }
            }
        }
    }
    let mut y = [0f32; 64];
    for (v, a) in acc.iter().enumerate() {
        // SAFETY: `y` is `[f32; 64]`; `8 * v + 8 <= 64` for `v < 8`.
        unsafe { _mm256_storeu_ps(y.as_mut_ptr().add(8 * v), *a) };
    }
    y
}

/// AVX twin of [`matrix_scalar`] over the transposed matrix: `k` = lane, `i`
/// ascending, separate mul + add -- bit-identical.
///
/// # Safety
/// AVX must be available.
#[cfg(target_arch = "x86_64")]
#[target_feature(enable = "avx")]
unsafe fn matrix_avx(mt: &[[f32; SUBBANDS]; 64], y: &[f32; 64]) -> [f32; SUBBANDS] {
    use std::arch::x86_64::*;
    // SAFETY: a register constant -- no memory access; the ISA is this fn's contract. Redundant from Rust 1.86 (safe target-feature calls), required at the crate's MSRV (1.85).
    #[allow(unused_unsafe)]
    let mut acc = [unsafe { _mm256_setzero_ps() }; 4];
    for (i, row) in mt.iter().enumerate() {
        // SAFETY: `row` is `[f32; 32]`; `8 * v + 8 <= 32` for `v < 4`.
        unsafe {
            let x = _mm256_set1_ps(y[i]);
            for (v, a) in acc.iter_mut().enumerate() {
                *a = _mm256_add_ps(
                    *a,
                    _mm256_mul_ps(_mm256_loadu_ps(row.as_ptr().add(8 * v)), x),
                );
            }
        }
    }
    let mut s = [0f32; SUBBANDS];
    for (v, a) in acc.iter().enumerate() {
        // SAFETY: `s` is `[f32; 32]`; `8 * v + 8 <= 32` for `v < 4`.
        unsafe { _mm256_storeu_ps(s.as_mut_ptr().add(8 * v), *a) };
    }
    s
}

/// NEON twin of [`fold_scalar`] (4 lanes, 16 accumulators; separate `vmulq` +
/// `vaddq`, never `vfmaq`) -- bit-identical.
///
/// # Safety
/// NEON is baseline on AArch64; all memory bounds are established inside
/// (fixed-size arrays; `head` is masked to a multiple of 32).
#[cfg(target_arch = "aarch64")]
unsafe fn fold_neon(c: &[f32; 512], fifo: &[f32; 512], head: usize) -> [f32; 64] {
    use std::arch::aarch64::*;
    // See `fold_avx`: masking makes the spans' bounds independent of the caller.
    let head = head & !31;
    // SAFETY: a register constant -- no memory access; the ISA is this fn's contract. Redundant from Rust 1.86 (safe target-feature calls), required at the crate's MSRV (1.85).
    #[allow(unused_unsafe)]
    let mut acc = [unsafe { vdupq_n_f32(0.0) }; 16];
    for j in 0..8 {
        for h in 0..2 {
            let at = (head + 32 * h + 64 * j) & 511;
            for q in 0..8 {
                // SAFETY: `c` offsets reach `32 + 448 + 28 + 4 = 512`, its length; `head` is masked to a multiple of 32 above, so `at` is a multiple of 32 below 512 and `at + 4 * q + 4 <= at + 32 <= 512`.
                unsafe {
                    let cv = vld1q_f32(c.as_ptr().add(32 * h + 64 * j + 4 * q));
                    let xv = vld1q_f32(fifo.as_ptr().add(at + 4 * q));
                    let a = &mut acc[8 * h + q];
                    *a = vaddq_f32(*a, vmulq_f32(cv, xv));
                }
            }
        }
    }
    let mut y = [0f32; 64];
    for (v, a) in acc.iter().enumerate() {
        // SAFETY: `y` is `[f32; 64]`; `4 * v + 4 <= 64` for `v < 16`.
        unsafe { vst1q_f32(y.as_mut_ptr().add(4 * v), *a) };
    }
    y
}

/// NEON twin of [`matrix_scalar`] (4 lanes, 8 accumulators) -- bit-identical.
///
/// # Safety
/// NEON is baseline on AArch64; fixed-size arrays only.
#[cfg(target_arch = "aarch64")]
unsafe fn matrix_neon(mt: &[[f32; SUBBANDS]; 64], y: &[f32; 64]) -> [f32; SUBBANDS] {
    use std::arch::aarch64::*;
    // SAFETY: a register constant -- no memory access; the ISA is this fn's contract. Redundant from Rust 1.86 (safe target-feature calls), required at the crate's MSRV (1.85).
    #[allow(unused_unsafe)]
    let mut acc = [unsafe { vdupq_n_f32(0.0) }; 8];
    for (i, row) in mt.iter().enumerate() {
        // SAFETY: `row` is `[f32; 32]`; `4 * v + 4 <= 32` for `v < 8`.
        unsafe {
            let x = vdupq_n_f32(y[i]);
            for (v, a) in acc.iter_mut().enumerate() {
                *a = vaddq_f32(*a, vmulq_f32(vld1q_f32(row.as_ptr().add(4 * v)), x));
            }
        }
    }
    let mut s = [0f32; SUBBANDS];
    for (v, a) in acc.iter().enumerate() {
        // SAFETY: `s` is `[f32; 32]`; `4 * v + 4 <= 32` for `v < 8`.
        unsafe { vst1q_f32(s.as_mut_ptr().add(4 * v), *a) };
    }
    s
}

/// This arch's fold twin (AVX / NEON / scalar).
///
/// # Safety
/// Caller checked [`crate::decode::isa::simd_available`]; `head % 32 == 0`.
#[inline]
unsafe fn fold_simd(c: &[f32; 512], fifo: &[f32; 512], head: usize) -> [f32; 64] {
    debug_assert_eq!(head % 32, 0);
    #[cfg(target_arch = "x86_64")]
    // SAFETY: forwards this fn's contract (the caller checked `simd_available()`).
    unsafe {
        fold_avx(c, fifo, head)
    }
    #[cfg(target_arch = "aarch64")]
    // SAFETY: forwards this fn's contract (the caller checked `simd_available()`).
    unsafe {
        fold_neon(c, fifo, head)
    }
    #[cfg(not(any(target_arch = "x86_64", target_arch = "aarch64")))]
    fold_scalar(c, fifo, head)
}

/// This arch's matrix twin (AVX / NEON / scalar).
///
/// # Safety
/// Caller checked [`crate::decode::isa::simd_available`].
#[inline]
unsafe fn matrix_simd(mt: &[[f32; SUBBANDS]; 64], y: &[f32; 64]) -> [f32; SUBBANDS] {
    #[cfg(target_arch = "x86_64")]
    // SAFETY: forwards this fn's contract (the caller checked `simd_available()`).
    unsafe {
        matrix_avx(mt, y)
    }
    #[cfg(target_arch = "aarch64")]
    // SAFETY: forwards this fn's contract (the caller checked `simd_available()`).
    unsafe {
        matrix_neon(mt, y)
    }
    #[cfg(not(any(target_arch = "x86_64", target_arch = "aarch64")))]
    {
        let _ = mt; // Vendored (studio): `mt` is the SIMD twins' operand; unused (and warned about) elsewhere.
        matrix_scalar(matrix(), y)
    }
}

/// Analyze one granule of mono PCM (`pcm[0..576]`) into subband samples
/// `[subband][line]`, advancing the channel's filterbank FIFO `X[]`.
///
/// # Panics
/// Never: the one `expect` takes a 32-float run of the 512-float FIFO at a
/// `head` that is always a multiple of 32, so the run is always 32 long.
pub fn analyze(pcm: &[f32], fifo: &mut [f32; 512]) -> [[f32; SUBBAND_LINES]; SUBBANDS] {
    let c = window();
    let m = matrix();
    let mt = matrix_t();
    let mut out = [[0f32; SUBBAND_LINES]; SUBBANDS];
    // Resolved once per granule (codec-measurement: a dispatch read inside the
    // pass loop is overhead added to take a measurement).
    let simd = crate::decode::isa::use_simd();
    super::prof::FB_PASSES[usize::from(simd)]
        .fetch_add(SUBBAND_LINES as u64, std::sync::atomic::Ordering::Relaxed);
    // The granule is exactly `SUBBAND_LINES * 32` samples and callers pass an
    // open-ended slice, so `pcm[v * 32 + t]` could not be proven in range and every
    // one of the 576 FIFO pushes carried a bounds check.
    let Some(pcm) = pcm.first_chunk::<{ SUBBAND_LINES * 32 }>() else {
        return out;
    };

    // The FIFO is addressed CIRCULARLY rather than shifted -- the encoder twin of
    // the decoder's synthesis FIFO (decode/synthesis.rs, D2). Logical `X[L]`
    // (0 = newest) lives at physical `(head + L) & 511`, so "shift up by 32" is a
    // subtraction on `head` instead of a 480-float `copy_within` on every pass
    // (8,640 float moves per granule per channel). `head` stays a multiple of 32
    // and 512 is a multiple of 32, so every 32-long run below is contiguous and
    // never wraps mid-run. Byte-identical: same products, same order per output.
    let mut head = 0usize;
    for v in 0..SUBBAND_LINES {
        // Shift up by 32 and push the 32 new samples in, newest at X[0]
        // (ISO: X[i]=X[i-32]; then X[31]..X[0] take this pass's samples in order).
        head = (head + 512 - 32) & 511;
        let new: &mut [f32; 32] = (&mut fifo[head..head + 32])
            .try_into()
            .expect("32-aligned run");
        for t in 0..32 {
            new[31 - t] = pcm[v * 32 + t];
        }

        // Window + fold 512 → 64, then matrix 64 → 32.
        let s = if simd {
            // SAFETY: `simd` came from `use_simd()`; `head` is a multiple of 32.
            unsafe { matrix_simd(mt, &fold_simd(c, fifo, head)) }
        } else {
            matrix_scalar(m, &fold_scalar(c, fifo, head))
        };
        for k in 0..SUBBANDS {
            out[k][v] = s[k];
        }
    }
    // Restore the canonical layout (`X[L]` at physical `L`) the caller's state is
    // defined in: 18 passes moved `head` by 576 ≡ 64 (mod 512), to 448.
    fifo.rotate_left(head);
    out
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::decode::synthesis;
    use crate::frame::{GRANULE_LINES, SUBBANDS, SUBBAND_LINES};
    use std::f32::consts::PI as PIf;

    /// Flatten subband-major `[subband][line]` into the decoder's `time[k*18+v]`.
    fn flatten(sb: &[[f32; SUBBAND_LINES]; SUBBANDS]) -> [f32; GRANULE_LINES] {
        let mut t = [0f32; GRANULE_LINES];
        for k in 0..SUBBANDS {
            for v in 0..SUBBAND_LINES {
                t[k * SUBBAND_LINES + v] = sb[k][v];
            }
        }
        t
    }

    /// Drive a tone through analysis→synthesis and find the best-aligned
    /// reconstruction SNR (the filterbank has an inherent delay).
    #[test]
    fn analysis_synthesis_reconstructs_a_tone() {
        let granules = 16;
        let n = granules * GRANULE_LINES;
        let input: Vec<f32> = (0..n)
            .map(|i| 0.5 * (2.0 * PIf * 1000.0 * i as f32 / 44100.0).sin())
            .collect();

        let mut afifo = [0f32; 512];
        let mut sfifo = [0f32; 1024];
        let mut output = Vec::with_capacity(n);
        for g in 0..granules {
            let sb = analyze(&input[g * GRANULE_LINES..], &mut afifo);
            let pcm = synthesis::polyphase(&flatten(&sb), &mut sfifo);
            output.extend_from_slice(&pcm);
        }

        // Search the small delay range for the best reconstruction.
        let (mut best_snr, mut best_delay) = (f64::NEG_INFINITY, 0usize);
        for delay in 480..=482 {
            let mut sig = 0f64;
            let mut err = 0f64;
            for i in delay..n {
                let r = f64::from(input[i - delay]);
                let o = f64::from(output[i]);
                sig += r * r;
                err += (r - o) * (r - o);
            }
            let snr = 10.0 * (sig / err).log10();
            if snr > best_snr {
                best_snr = snr;
                best_delay = delay;
            }
        }
        eprintln!("[L1] best reconstruction SNR {best_snr:.1} dB at delay {best_delay}");
        // The MPEG pseudo-QMF reconstructs to better than ~80 dB; unity gain.
        assert!(
            best_snr > 70.0,
            "analysis/synthesis SNR too low: {best_snr:.1} dB"
        );
    }

    fn rng(seed: u32) -> impl FnMut() -> f32 {
        let mut st = seed;
        move || {
            st = st.wrapping_mul(1_664_525).wrapping_add(1_013_904_223);
            (st >> 8) as f32 / (1u32 << 24) as f32 - 0.5
        }
    }

    /// The SIMD fold must be BIT-identical to the scalar oracle at every legal
    /// `head` (all multiples of 32, so the circular wrap is covered).
    #[test]
    fn fold_simd_matches_scalar() {
        if !crate::decode::isa::simd_available() {
            eprintln!("no SIMD twin on this host - scalar path only, gate skipped");
            return;
        }
        let mut r = rng(0x2545_F491);
        let mut fifo = [0f32; 512];
        for x in &mut fifo {
            *x = r();
        }
        let c = window();
        for head in (0..512).step_by(32) {
            let a = fold_scalar(c, &fifo, head);
            // SAFETY: gated on simd_available(); head is a multiple of 32.
            let b = unsafe { fold_simd(c, &fifo, head) };
            assert_eq!(a, b, "fold SIMD/scalar mismatch at head={head}");
        }
    }

    /// The SIMD matrix (over the transposed table) must be BIT-identical too.
    #[test]
    fn matrix_simd_matches_scalar() {
        if !crate::decode::isa::simd_available() {
            eprintln!("no SIMD twin on this host - scalar path only, gate skipped");
            return;
        }
        let mut r = rng(0x9E37_79B9);
        for trial in 0..128 {
            let mut y = [0f32; 64];
            for x in &mut y {
                *x = r();
            }
            let a = matrix_scalar(matrix(), &y);
            // SAFETY: gated on simd_available().
            let b = unsafe { matrix_simd(matrix_t(), &y) };
            assert_eq!(a, b, "matrix SIMD/scalar mismatch on trial {trial}");
        }
    }

    #[test]
    fn dc_maps_to_subband_zero() {
        // A DC input lands entirely in subband 0; higher subbands stay ~silent
        // once the FIFO has primed.
        let input = [1.0f32; GRANULE_LINES * 4];
        let mut afifo = [0f32; 512];
        let mut sb = [[0f32; SUBBAND_LINES]; SUBBANDS];
        for g in 0..4 {
            sb = analyze(&input[g * GRANULE_LINES..], &mut afifo);
        }
        let sb0: f32 = sb[0].iter().map(|x| x.abs()).sum();
        let high: f32 = (1..SUBBANDS)
            .map(|k| sb[k].iter().map(|x| x.abs()).sum::<f32>())
            .sum();
        assert!(
            sb0 > high,
            "DC energy must concentrate in subband 0 (sb0={sb0}, high={high})"
        );
    }
}
