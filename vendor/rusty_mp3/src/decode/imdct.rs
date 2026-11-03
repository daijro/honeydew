//! Hybrid synthesis stage 1: the inverse MDCT with windowing and overlap-add.
//!
//! Per subband (32), the 18 frequency lines run through an IMDCT — one 36-point
//! transform for long/start/stop blocks, or three 12-point transforms for short
//! blocks — then the matching window (Long/Start/Short/Stop). The first 18
//! samples overlap-add the previous granule's stored tail; the second 18 become
//! the next overlap. Odd subbands then get frequency inversion (every odd time
//! sample negated) to align with the synthesis filterbank.

use std::f64::consts::PI;
use std::sync::OnceLock;

use crate::frame::{BlockType, GranuleSideInfo, GRANULE_LINES, SUBBANDS, SUBBAND_LINES};

struct Kernels {
    cos36: [[f32; 18]; 36],
    cos12: [[f32; 6]; 12],
    /// Long(0), Start(1), Stop(3) windows; index 2 unused (short handled apart).
    win: [[f32; 36]; 4],
    win_short: [f32; 12],
}

fn kernels() -> &'static Kernels {
    static T: OnceLock<Kernels> = OnceLock::new();
    T.get_or_init(|| {
        let mut cos36 = [[0f32; 18]; 36];
        for (n, row) in cos36.iter_mut().enumerate() {
            for (k, c) in row.iter_mut().enumerate() {
                *c = (PI / 72.0 * (2 * n + 1 + 18) as f64 * (2 * k + 1) as f64).cos() as f32;
            }
        }
        let mut cos12 = [[0f32; 6]; 12];
        for (n, row) in cos12.iter_mut().enumerate() {
            for (k, c) in row.iter_mut().enumerate() {
                *c = (PI / 24.0 * (2 * n + 1 + 6) as f64 * (2 * k + 1) as f64).cos() as f32;
            }
        }
        let sin = |x: f64| x.sin() as f32;
        let mut win = [[0f32; 36]; 4];
        for n in 0..36 {
            win[0][n] = sin(PI / 36.0 * (n as f64 + 0.5)); // Long
        }
        for n in 0..18 {
            win[1][n] = sin(PI / 36.0 * (n as f64 + 0.5)); // Start
        }
        for n in 18..24 {
            win[1][n] = 1.0;
        }
        for n in 24..30 {
            win[1][n] = sin(PI / 12.0 * ((n - 18) as f64 + 0.5));
        }
        for n in 6..12 {
            win[3][n] = sin(PI / 12.0 * ((n - 6) as f64 + 0.5)); // Stop
        }
        for n in 12..18 {
            win[3][n] = 1.0;
        }
        for n in 18..36 {
            win[3][n] = sin(PI / 36.0 * (n as f64 + 0.5));
        }
        let mut win_short = [0f32; 12];
        for (n, w) in win_short.iter_mut().enumerate() {
            *w = sin(PI / 12.0 * (n as f64 + 0.5));
        }
        Kernels {
            cos36,
            cos12,
            win,
            win_short,
        }
    })
}

/// The 18 of the 36 long-block outputs that are actually computed; the other 18
/// are exact mirrors of these (see the symmetries in `hybrid`).
const IMDCT_N: [usize; 18] = [
    0, 1, 2, 3, 4, 5, 6, 7, 8, 18, 19, 20, 21, 22, 23, 24, 25, 26,
];

/// `cos36` transposed and compacted to `[k][j]`, `j` indexing [`IMDCT_N`], padded
/// from 18 to 24 columns so it covers exactly three 8-wide lanes.
///
/// Transposing is what lets the transform vectorise WITHOUT reassociating
/// anything: the natural loop reduces over `k` (a horizontal sum), but with the
/// OUTPUT index as the lane each `out[j]` still accumulates over `k` in the
/// original order. The padding lanes compute garbage that is never read.
fn cos36_t() -> &'static [[f32; 24]; 18] {
    static T: OnceLock<[[f32; 24]; 18]> = OnceLock::new();
    T.get_or_init(|| {
        let c = &kernels().cos36;
        let mut t = [[0f32; 24]; 18];
        for (j, &n) in IMDCT_N.iter().enumerate() {
            for k in 0..18 {
                t[k][j] = c[n][k];
            }
        }
        t
    })
}

/// **D6** — AVX twin of the 18 long-block dot products, 8 outputs per lane.
///
/// Bit-identical to the scalar form: `out[j]` owns a lane and accumulates its 18
/// terms in the original `k` order, with separate mul+add rather than FMA.
/// Float multiply is commutative and exact, so `c*x` here equals the scalar's
/// `x*c`. Pinned by `imdct_simd_matches_scalar`.
///
/// # Safety
/// Caller must have verified AVX is available.
#[cfg(target_arch = "x86_64")]
#[target_feature(enable = "avx")]
unsafe fn imdct36_avx(lines: &[f32], out: &mut [f32; 24]) {
    use std::arch::x86_64::*;
    // Same fix as the scalar twin: an unsized slice cannot carry its length, so
    // each of the eighteen broadcasts paid a bounds check.
    let Some(lines) = lines.first_chunk::<18>() else {
        return;
    };
    let ct = cos36_t();
    // SAFETY: a register constant -- no memory access; the ISA is this fn's contract. Redundant from Rust 1.86 (safe target-feature calls), required at the crate's MSRV (1.85).
    #[allow(unused_unsafe)]
    let mut acc = [unsafe { _mm256_setzero_ps() }; 3];
    for (k, row) in ct.iter().enumerate() {
        // SAFETY: `row` is `[f32; 24]` (`cos36_t`); `v * 8 + 8 <= 24` for `v < 3`. AVX per this fn's contract.
        unsafe {
            let x = _mm256_set1_ps(lines[k]);
            for (v, accv) in acc.iter_mut().enumerate() {
                let c = _mm256_loadu_ps(row.as_ptr().add(v * 8));
                *accv = _mm256_add_ps(*accv, _mm256_mul_ps(c, x));
            }
        }
    }
    for (v, accv) in acc.iter().enumerate() {
        // SAFETY: `out` is `[f32; 24]`; `v * 8 + 8 <= 24` for `v < 3`.
        unsafe { _mm256_storeu_ps(out.as_mut_ptr().add(v * 8), *accv) };
    }
}

/// NEON twin of [`imdct36_avx`]: 4 outputs per lane, 6 accumulators. Bit-identical
/// for the same reason -- each output owns a lane and accumulates its 18 terms in
/// the original `k` order with SEPARATE `vmulq` + `vaddq` (never `vfmaq`, which
/// rounds once where the scalar rounds twice). Pinned by `imdct_simd_matches_scalar`,
/// which runs under qemu-aarch64 (`tools/bench/arm_qemu_test.sh`).
///
/// # Safety
/// NEON is baseline on AArch64; the only obligation is the caller's slice length,
/// checked here by `first_chunk`.
#[cfg(target_arch = "aarch64")]
unsafe fn imdct36_neon(lines: &[f32], out: &mut [f32; 24]) {
    use std::arch::aarch64::*;
    let Some(lines) = lines.first_chunk::<18>() else {
        return;
    };
    let ct = cos36_t();
    // SAFETY: a register constant -- no memory access; the ISA is this fn's contract. Redundant from Rust 1.86 (safe target-feature calls), required at the crate's MSRV (1.85).
    #[allow(unused_unsafe)]
    let mut acc = [unsafe { vdupq_n_f32(0.0) }; 6];
    for (k, row) in ct.iter().enumerate() {
        // SAFETY: `row` is `[f32; 24]` (`cos36_t`); `v * 4 + 4 <= 24` for `v < 6`.
        unsafe {
            let x = vdupq_n_f32(lines[k]);
            for (v, accv) in acc.iter_mut().enumerate() {
                let c = vld1q_f32(row.as_ptr().add(v * 4));
                *accv = vaddq_f32(*accv, vmulq_f32(c, x));
            }
        }
    }
    for (v, accv) in acc.iter().enumerate() {
        // SAFETY: `out` is `[f32; 24]`; `v * 4 + 4 <= 24` for `v < 6`.
        unsafe { vst1q_f32(out.as_mut_ptr().add(v * 4), *accv) };
    }
}

/// This arch's SIMD twin (AVX on x86_64, NEON on aarch64, scalar elsewhere).
///
/// # Safety
/// The caller must have checked [`super::isa::simd_available`] (the shipping path
/// asks `use_simd`, which implies it): on x86_64 this executes AVX instructions.
#[inline]
unsafe fn imdct36_simd(lines: &[f32], out: &mut [f32; 24]) {
    #[cfg(target_arch = "x86_64")]
    // SAFETY: forwards this fn's contract (the caller checked `simd_available()`).
    unsafe {
        imdct36_avx(lines, out);
    }
    #[cfg(target_arch = "aarch64")]
    // SAFETY: forwards this fn's contract (the caller checked `simd_available()`).
    unsafe {
        imdct36_neon(lines, out)
    }
    #[cfg(not(any(target_arch = "x86_64", target_arch = "aarch64")))]
    imdct36_scalar(lines, out)
}

/// The scalar twin — oracle and non-SIMD fallback.
#[inline]
fn imdct36_scalar(lines: &[f32], out: &mut [f32; 24]) {
    // Convert to a fixed-size reference ONCE. As a `&[f32]` the compiler cannot
    // know the length, so each of the eighteen reads in the inner loop carried its
    // own bounds check; every caller passes exactly eighteen lines.
    let Some(lines) = lines.first_chunk::<18>() else {
        return;
    };
    let cos36 = &kernels().cos36;
    for (j, &n) in IMDCT_N.iter().enumerate() {
        let row = &cos36[n];
        let mut acc = 0f32;
        for k in 0..18 {
            acc += lines[k] * row[k];
        }
        out[j] = acc;
    }
}

/// Raw SIMD availability, for the oracle tests (they must run the kernel whatever
/// `MP3_ISA` says). The shipping path asks [`super::isa::use_simd`].
#[cfg(test)]
fn have_simd() -> bool {
    super::isa::simd_available()
}

/// Run the hybrid IMDCT for one channel's granule. `overlap` holds the previous
/// granule's tail on entry and is updated with this granule's tail on exit.
/// Returns 576 time-domain values (subband-major) for the synthesis stage.
pub fn hybrid(
    gi: &GranuleSideInfo,
    lines: &[f32; GRANULE_LINES],
    overlap: &mut [f32; GRANULE_LINES],
) -> [f32; GRANULE_LINES] {
    // Resolved once per granule, not per subband.
    let simd = super::isa::use_simd();
    let is_short = gi.window_switching && gi.block_type == BlockType::Short;
    // Census: how many subbands of this granule take the 36-point (kernel) path.
    let long_subbands = match (is_short, gi.mixed_block) {
        (false, _) => SUBBANDS as u64,
        (true, true) => 2,
        (true, false) => 0,
    };
    super::prof::kernel_tally(
        if simd {
            &super::prof::K_IMDCT_SIMD
        } else {
            &super::prof::K_IMDCT_SCALAR
        },
        long_subbands,
    );
    // The `#[target_feature]` boundary sits HERE, once per granule, not inside the
    // subband loop: a target-feature function cannot inline into a caller that
    // lacks the feature, so calling `imdct36_avx` from the plain body paid a real
    // call per long subband (32 per granule). Entering an AVX-compiled copy of the
    // whole body lets the kernel inline (codec-vectorize-kernel: put the LOOP
    // inside the `#[target_feature]` function). NEON is baseline, so aarch64 never
    // had the boundary.
    #[cfg(target_arch = "x86_64")]
    if simd {
        // SAFETY: on x86_64 `use_simd()` is true only when AVX was detected.
        return unsafe { hybrid_avx(gi, lines, overlap) };
    }
    hybrid_impl(gi, lines, overlap, simd)
}

/// [`hybrid_impl`] compiled with AVX enabled, so the kernel calls inside inline.
///
/// # Safety
/// AVX must be available.
#[cfg(target_arch = "x86_64")]
#[target_feature(enable = "avx")]
unsafe fn hybrid_avx(
    gi: &GranuleSideInfo,
    lines: &[f32; GRANULE_LINES],
    overlap: &mut [f32; GRANULE_LINES],
) -> [f32; GRANULE_LINES] {
    hybrid_impl(gi, lines, overlap, true)
}

/// The hybrid IMDCT body; `simd` selects the kernel twin (callers pass a value
/// from `use_simd()`, and on x86_64 only `hybrid_avx` passes `true`).
// Load-bearing: the body must inline INTO the `#[target_feature]` entry so the
// kernels inline with it (docs/plans/mp3-kernel-ledger.md, brick 4).
#[allow(clippy::inline_always)]
#[inline(always)]
fn hybrid_impl(
    gi: &GranuleSideInfo,
    lines: &[f32; GRANULE_LINES],
    overlap: &mut [f32; GRANULE_LINES],
    simd: bool,
) -> [f32; GRANULE_LINES] {
    let t = kernels();
    let mut out = [0f32; GRANULE_LINES];
    let is_short = gi.window_switching && gi.block_type == BlockType::Short;

    for sb in 0..SUBBANDS {
        let base = sb * SUBBAND_LINES;
        let mut samp = [0f32; 36];
        // Mixed blocks keep the lowest two subbands long.
        let short_here = is_short && !(gi.mixed_block && sb < 2);

        if short_here {
            for w in 0..3 {
                let mut y = [0f32; 12];
                // The 12-point kernel carries the same two exact symmetries:
                // cos12[5−n][k] == −cos12[n][k] and cos12[17−n][k] == +cos12[n][k].
                for n in 0..3 {
                    let mut acc = 0f32;
                    for k in 0..6 {
                        acc += lines[base + w + 3 * k] * t.cos12[n][k];
                    }
                    y[n] = acc * t.win_short[n];
                    y[5 - n] = -acc * t.win_short[5 - n];
                }
                for n in 6..9 {
                    let mut acc = 0f32;
                    for k in 0..6 {
                        acc += lines[base + w + 3 * k] * t.cos12[n][k];
                    }
                    y[n] = acc * t.win_short[n];
                    y[17 - n] = acc * t.win_short[17 - n];
                }
                for n in 0..12 {
                    samp[6 + w * 6 + n] += y[n];
                }
            }
        } else {
            // The mixed flag is not exclusive to short blocks: on ANY
            // window-switching block its two lowest subbands use the NORMAL window
            // (ISO 11172-3 2.4.2.7; FFmpeg `win_idx = switch_point && j < 2 ? 0 :
            // block_type`). A Start/Stop block with the flag set put its start/stop
            // window on subbands 0..1 -- ISO l3-si_block, all error in subband 0-1.
            let wt = if gi.mixed_block && sb < 2 {
                0
            } else {
                match gi.block_type {
                    BlockType::Start => 1,
                    BlockType::Stop => 3,
                    _ => 0,
                }
            };
            // **D3** — half the dot products are free. The kernel has two exact
            // symmetries, and they hold BIT-EXACTLY in the stored f32 tables
            // (pinned by `imdct_kernel_symmetries_are_bit_exact`):
            //
            //   cos36[17−n][k] == −cos36[n][k]   (first half, antisymmetric)
            //   cos36[53−n][k] == +cos36[n][k]   (second half, symmetric)
            //
            // Because IEEE multiplication and round-to-nearest-even are both
            // sign-symmetric, `Σ lines[k]·(−c[k])` is EXACTLY `−Σ lines[k]·c[k]`.
            // So the mirrored output is bit-identical to computing its own dot
            // product, not an approximation of it — 36×18 MACs become 18×18.
            //
            // **D6** — those 18 dot products now run 8-wide with the OUTPUT index
            // as the lane, so each still accumulates over `k` in the original
            // order and the result is bit-identical (not merely close).
            let mut dp = [0f32; 24];
            if simd {
                // SAFETY: `simd` came from `use_simd()` before the loop.
                unsafe { imdct36_simd(&lines[base..base + 18], &mut dp) };
            } else {
                imdct36_scalar(&lines[base..base + 18], &mut dp);
            }
            for (j, &n) in IMDCT_N.iter().enumerate() {
                let acc = dp[j];
                if n < 9 {
                    samp[n] = acc * t.win[wt][n];
                    samp[17 - n] = -acc * t.win[wt][17 - n];
                } else {
                    samp[n] = acc * t.win[wt][n];
                    samp[53 - n] = acc * t.win[wt][53 - n];
                }
            }
        }

        // Overlap-add: first half with the saved tail, second half becomes tail.
        for n in 0..18 {
            out[base + n] = samp[n] + overlap[base + n];
            overlap[base + n] = samp[n + 18];
        }
        // Frequency inversion: negate odd samples of odd subbands.
        if sb & 1 == 1 {
            let mut i = 1;
            while i < 18 {
                out[base + i] = -out[base + i];
                i += 2;
            }
        }
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    /// The pre-D3 dense IMDCT, verbatim: every one of the 36 (or 3x12) outputs
    /// computed as its own full dot product. Kept as the oracle the half-work
    /// form is gated against.
    fn hybrid_dense(
        gi: &GranuleSideInfo,
        lines: &[f32; GRANULE_LINES],
        overlap: &mut [f32; GRANULE_LINES],
    ) -> [f32; GRANULE_LINES] {
        let t = kernels();
        let mut out = [0f32; GRANULE_LINES];
        let is_short = gi.window_switching && gi.block_type == BlockType::Short;
        for sb in 0..SUBBANDS {
            let base = sb * SUBBAND_LINES;
            let mut samp = [0f32; 36];
            let short_here = is_short && !(gi.mixed_block && sb < 2);
            if short_here {
                for w in 0..3 {
                    let mut y = [0f32; 12];
                    for n in 0..12 {
                        let mut acc = 0f32;
                        for k in 0..6 {
                            acc += lines[base + w + 3 * k] * t.cos12[n][k];
                        }
                        y[n] = acc * t.win_short[n];
                    }
                    for n in 0..12 {
                        samp[6 + w * 6 + n] += y[n];
                    }
                }
            } else {
                let wt = if gi.mixed_block && sb < 2 {
                    0
                } else {
                    match gi.block_type {
                        BlockType::Start => 1,
                        BlockType::Stop => 3,
                        _ => 0,
                    }
                };
                for n in 0..36 {
                    let mut acc = 0f32;
                    for k in 0..18 {
                        acc += lines[base + k] * t.cos36[n][k];
                    }
                    samp[n] = acc * t.win[wt][n];
                }
            }
            for n in 0..18 {
                out[base + n] = samp[n] + overlap[base + n];
                overlap[base + n] = samp[n + 18];
            }
            if sb & 1 == 1 {
                let mut i = 1;
                while i < 18 {
                    out[base + i] = -out[base + i];
                    i += 2;
                }
            }
        }
        out
    }

    /// **D3 corpus-gap closer.** The 15-stream decode corpus covers long and
    /// short blocks (15-37% short) but shows MIXED blocks at 0.0% on every
    /// stream — LAME simply never emits them, so no LAME-sourced corpus can
    /// reach that path. Rather than claim coverage we do not have, the mixed
    /// case (and every other block type) is gated directly against the dense
    /// oracle here, asserting BIT-identity, not a tolerance.
    #[test]
    fn half_work_imdct_matches_dense_for_every_block_type() {
        let mut s = 0x9E37_79B9u32;
        let mut rng = || {
            s = s.wrapping_mul(1_664_525).wrapping_add(1_013_904_223);
            (s >> 8) as f32 / (1u32 << 24) as f32 - 0.5
        };
        for &(bt, mixed) in &[
            (BlockType::Long, false),
            (BlockType::Start, false),
            (BlockType::Stop, false),
            (BlockType::Short, false),
            (BlockType::Short, true), // the population the corpus cannot reach
            // The mixed flag on a Start/Stop block (ISO l3-si_block carries both).
            (BlockType::Start, true),
            (BlockType::Stop, true),
        ] {
            let gi = GranuleSideInfo {
                window_switching: bt != BlockType::Long,
                block_type: bt,
                mixed_block: mixed,
                ..Default::default()
            };
            // Several granules in sequence, so the overlap state carries across
            // exactly as it does in a real stream.
            let (mut ov_a, mut ov_b) = ([0f32; GRANULE_LINES], [0f32; GRANULE_LINES]);
            for g in 0..4 {
                let mut lines = [0f32; GRANULE_LINES];
                for l in &mut lines {
                    *l = rng();
                }
                let fast = hybrid(&gi, &lines, &mut ov_a);
                let dense = hybrid_dense(&gi, &lines, &mut ov_b);
                assert_eq!(
                    fast, dense,
                    "block_type={bt:?} mixed={mixed} granule={g}: half-work IMDCT diverged from dense"
                );
                assert_eq!(
                    ov_a, ov_b,
                    "overlap diverged: block_type={bt:?} mixed={mixed}"
                );
            }
        }
    }

    /// **D6 gate.** The AVX IMDCT must be BIT-identical to the scalar twin — the
    /// whole point of transposing the kernel is that the reduction over `k` is
    /// never reassociated, so this is `assert_eq!`, not a tolerance.
    #[test]
    fn imdct_simd_matches_scalar() {
        if !have_simd() {
            eprintln!("no SIMD twin on this host - scalar path only, gate skipped");
            return;
        }
        let mut st = 0x1F35_3D9Bu32;
        let mut rng = || {
            st = st.wrapping_mul(1_664_525).wrapping_add(1_013_904_223);
            (st >> 8) as f32 / (1u32 << 24) as f32 - 0.5
        };
        for trial in 0..128 {
            let lines: Vec<f32> = (0..18).map(|_| rng()).collect();
            let (mut a, mut b) = ([0f32; 24], [0f32; 24]);
            imdct36_scalar(&lines, &mut a);
            // SAFETY: gated on `have_simd()` above.
            unsafe { imdct36_simd(&lines, &mut b) };
            // Only the 18 real outputs; columns 18..24 are padding.
            assert_eq!(
                a[..18],
                b[..18],
                "IMDCT SIMD/scalar mismatch on trial {trial}"
            );
        }
    }

    /// **D3 gate.** The half-work IMDCT is bit-identical ONLY because these four
    /// symmetries hold exactly in the stored f32 tables — not merely to within a
    /// tolerance. If the table generation is ever changed (different argument
    /// form, f32 math instead of f64, a fast-cos), this fires here rather than
    /// letting the decoder drift off FFmpeg conformance silently.
    #[test]
    fn imdct_kernel_symmetries_are_bit_exact() {
        let t = kernels();
        for n in 0..9 {
            for k in 0..18 {
                assert_eq!(
                    t.cos36[17 - n][k],
                    -t.cos36[n][k],
                    "cos36 antisym n={n} k={k}"
                );
            }
        }
        for n in 18..27 {
            for k in 0..18 {
                assert_eq!(t.cos36[53 - n][k], t.cos36[n][k], "cos36 sym n={n} k={k}");
            }
        }
        for n in 0..3 {
            for k in 0..6 {
                assert_eq!(
                    t.cos12[5 - n][k],
                    -t.cos12[n][k],
                    "cos12 antisym n={n} k={k}"
                );
            }
        }
        for n in 6..9 {
            for k in 0..6 {
                assert_eq!(t.cos12[17 - n][k], t.cos12[n][k], "cos12 sym n={n} k={k}");
            }
        }
    }

    #[test]
    fn long_imdct_impulse_matches_formula() {
        // A single coefficient in subband 0 → out[n] = cos36[n][0]·win_long[n]
        // (overlap starts at zero).
        let mut lines = [0f32; GRANULE_LINES];
        lines[0] = 1.0;
        let mut overlap = [0f32; GRANULE_LINES];
        let out = hybrid(&GranuleSideInfo::default(), &lines, &mut overlap);
        let expected = (PI / 72.0 * 19.0).cos() as f32 * (PI / 36.0 * 0.5).sin() as f32;
        assert!(
            (out[0] - expected).abs() < 1e-5,
            "out[0]={} expected={}",
            out[0],
            expected
        );
        // The second half of the 36-sample frame is saved as the next overlap.
        let expect_ov = (PI / 72.0 * 55.0).cos() as f32 * (PI / 36.0 * 18.5).sin() as f32;
        assert!((overlap[0] - expect_ov).abs() < 1e-5);
    }

    #[test]
    fn odd_subband_frequency_inversion() {
        // Subband 1 (odd): odd-indexed output samples are negated vs the raw IMDCT.
        let mut lines = [0f32; GRANULE_LINES];
        lines[SUBBAND_LINES] = 1.0; // subband 1, coefficient 0
        let mut overlap = [0f32; GRANULE_LINES];
        let out = hybrid(&GranuleSideInfo::default(), &lines, &mut overlap);
        let raw1 = (PI / 72.0 * (2.0 + 1.0 + 18.0)).cos() as f32 * (PI / 36.0 * 1.5).sin() as f32;
        assert!(
            (out[SUBBAND_LINES + 1] + raw1).abs() < 1e-5,
            "odd sample must be negated"
        );
    }
}
