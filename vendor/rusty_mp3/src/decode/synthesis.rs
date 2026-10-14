//! Hybrid synthesis stage 2: the 32-band polyphase synthesis filterbank.
//!
//! Each of the 18 passes takes 32 subband samples and produces 32 PCM samples.
//! A 32→64 cosine matrixing feeds the 1024-sample FIFO `V[]`; a windowing step
//! gathers `U[]` from `V[]`, multiplies by the 512-tap `D[]` window, and sums to
//! 32 outputs. The matrixing is computed here; `D[]` is tabulated in [`tables`](crate::tables).

use std::f64::consts::PI;
use std::sync::OnceLock;

use crate::frame::{GRANULE_LINES, SUBBANDS, SUBBAND_LINES};

use super::synth_window::SYNTH_D;

/// Matrixing coefficients `N[i][k] = cos((16+i)·(2k+1)·π/64)`, 64×32. The dense
/// reference — still the source of truth that [`matrixing_fast`] is gated against.
fn matrix() -> &'static [[f32; SUBBANDS]; 64] {
    static T: OnceLock<[[f32; SUBBANDS]; 64]> = OnceLock::new();
    T.get_or_init(|| {
        let mut n = [[0f32; SUBBANDS]; 64];
        for (i, row) in n.iter_mut().enumerate() {
            for (k, c) in row.iter_mut().enumerate() {
                *c = (PI / 64.0 * (16 + i) as f64 * (2 * k + 1) as f64).cos() as f32;
            }
        }
        n
    })
}

/// Half-width DCT kernel `C[m][k] = cos(m·(2k+1)·π/64)`, `m=0..31`, `k=0..15` —
/// the 32 distinct matrixing values `G[m]` after both symmetries are folded out.
fn half_dct() -> &'static [[f32; 16]; 32] {
    static T: OnceLock<[[f32; 16]; 32]> = OnceLock::new();
    T.get_or_init(|| {
        let mut c = [[0f32; 16]; 32];
        for (m, row) in c.iter_mut().enumerate() {
            for (k, e) in row.iter_mut().enumerate() {
                *e = (PI / 64.0 * m as f64 * (2 * k + 1) as f64).cos() as f32;
            }
        }
        c
    })
}

/// **B3** — the 64 matrixing outputs `V[i] = Σ_k cos((16+i)(2k+1)π/64)·s[k]`,
/// computed via two exact cosine symmetries instead of a dense 64×32 product:
///
/// * within `k`: `cos(m·(63−2k)π/64) = (−1)^m cos(m·(2k+1)π/64)` folds the 32
///   inputs into 16 sum/difference terms, so each `G[m]` is 16 mults not 32;
/// * across `i`: every `V[i]` is a signed copy of one of just 32 values
///   `G[m] = Σ_k cos(m(2k+1)π/64)·s[k]` (`m=0..31`) — `V[16]` is identically 0.
///
/// 512 mults vs 2048. Equal to [`matrix`] up to float reassociation (the symmetry
/// is exact; only ULP rounding differs) — pinned by `fast_matrixing_matches_dense`.
fn matrixing_fast(s: &[f32; SUBBANDS]) -> [f32; 64] {
    let c = half_dct();
    // Fold k↔31−k: even-m terms use the sum, odd-m terms the difference.
    let mut plus = [0f32; 16];
    let mut minus = [0f32; 16];
    for k in 0..16 {
        plus[k] = s[k] + s[31 - k];
        minus[k] = s[k] - s[31 - k];
    }
    // The 32 distinct DCT outputs G[0..31].
    let mut g = [0f32; 32];
    for (m, gm) in g.iter_mut().enumerate() {
        let src = if m & 1 == 0 { &plus } else { &minus };
        let mut acc = 0f32;
        for k in 0..16 {
            acc += c[m][k] * src[k];
        }
        *gm = acc;
    }
    // Map G → the 64 V outputs by sign/index (V[16] = 0).
    expand_g(&g)
}

/// `half_dct` transposed to `[k][m]`, so the 32 outputs `G[m]` are CONTIGUOUS
/// for a fixed `k`. Built once.
///
/// This is what makes the matrixing vectorisable without reassociating anything:
/// the natural loop reduces over `k` (a horizontal sum, which would need
/// reassociation and stop being bit-identical), but with `m` as the lane, each
/// `G[m]` accumulates over `k` in the ORIGINAL order, independently.
fn half_dct_t() -> &'static [[f32; 32]; 16] {
    static T: OnceLock<[[f32; 32]; 16]> = OnceLock::new();
    T.get_or_init(|| {
        let c = half_dct();
        let mut t = [[0f32; 32]; 16];
        for (m, row) in c.iter().enumerate() {
            for (k, v) in row.iter().enumerate() {
                t[k][m] = *v;
            }
        }
        t
    })
}

/// **D5** — AVX twin of [`matrixing_fast`], 8 of the 32 `G[m]` per lane.
///
/// Bit-identical for the same reason as [`window_avx`]: `m` is the lane, so each
/// output accumulates its 16 terms in the original `k` order, with separate
/// mul+add rather than FMA. The even/odd `m` split (even takes the sum-folded
/// input, odd the difference-folded one) becomes a single blend, because a
/// lane group always starts on an even `m`.
///
/// # Safety
/// Caller must have verified AVX is available.
#[cfg(target_arch = "x86_64")]
#[target_feature(enable = "avx")]
unsafe fn matrixing_avx(s: &[f32; SUBBANDS]) -> [f32; 64] {
    use std::arch::x86_64::*;
    let ct = half_dct_t();
    let mut plus = [0f32; 16];
    let mut minus = [0f32; 16];
    for k in 0..16 {
        plus[k] = s[k] + s[31 - k];
        minus[k] = s[k] - s[31 - k];
    }
    // SAFETY: a register constant -- no memory access; the ISA is this fn's contract. Redundant from Rust 1.86 (safe target-feature calls), required at the crate's MSRV (1.85).
    #[allow(unused_unsafe)]
    let mut acc = [unsafe { _mm256_setzero_ps() }; 4];
    for k in 0..16 {
        // SAFETY: `ct[k]` is `[f32; 32]` (`half_dct_t`); `v * 8 + 8 <= 32` for `v < 4`.
        unsafe {
            // Lane m takes plus[k] when m is even, minus[k] when odd. Each group
            // of 8 starts at a multiple of 8, hence always even, so one mask does.
            let src = _mm256_blend_ps(_mm256_set1_ps(plus[k]), _mm256_set1_ps(minus[k]), 0xAA);
            for (v, accv) in acc.iter_mut().enumerate() {
                let c = _mm256_loadu_ps(ct[k].as_ptr().add(v * 8));
                *accv = _mm256_add_ps(*accv, _mm256_mul_ps(c, src));
            }
        }
    }
    let mut g = [0f32; 32];
    for (v, accv) in acc.iter().enumerate() {
        // SAFETY: `g` is `[f32; 32]`; `v * 8 + 8 <= 32` for `v < 4`.
        unsafe { _mm256_storeu_ps(g.as_mut_ptr().add(v * 8), *accv) };
    }
    expand_g(&g)
}

/// NEON twin of [`matrixing_avx`], 4 of the 32 `G[m]` per lane. Each 4-lane
/// group starts on an even `m`, so the even/odd input split is the fixed lane
/// pattern `[plus, minus, plus, minus]`. Separate `vmulq` + `vaddq` keep it
/// bit-identical to `matrixing_fast` (pinned by `matrixing_simd_matches_scalar`,
/// run under qemu-aarch64).
///
/// # Safety
/// NEON is baseline on AArch64; all accesses are into fixed-size arrays.
#[cfg(target_arch = "aarch64")]
unsafe fn matrixing_neon(s: &[f32; SUBBANDS]) -> [f32; 64] {
    use std::arch::aarch64::*;
    let ct = half_dct_t();
    // SAFETY: a register constant -- no memory access; the ISA is this fn's contract. Redundant from Rust 1.86 (safe target-feature calls), required at the crate's MSRV (1.85).
    #[allow(unused_unsafe)]
    let mut acc = [unsafe { vdupq_n_f32(0.0) }; 8];
    for k in 0..16 {
        let (p, m) = (s[k] + s[31 - k], s[k] - s[31 - k]);
        let pm = [p, m, p, m];
        // SAFETY: `pm` is `[f32; 4]`; `ct[k]` is `[f32; 32]` and `v * 4 + 4 <= 32` for `v < 8`.
        unsafe {
            let src = vld1q_f32(pm.as_ptr());
            for (v, accv) in acc.iter_mut().enumerate() {
                let c = vld1q_f32(ct[k].as_ptr().add(v * 4));
                *accv = vaddq_f32(*accv, vmulq_f32(c, src));
            }
        }
    }
    let mut g = [0f32; 32];
    for (v, accv) in acc.iter().enumerate() {
        // SAFETY: `g` is `[f32; 32]`; `v * 4 + 4 <= 32` for `v < 8`.
        unsafe { vst1q_f32(g.as_mut_ptr().add(v * 4), *accv) };
    }
    expand_g(&g)
}

/// This arch's SIMD matrixing (AVX / NEON / scalar).
///
/// # Safety
/// Caller must have checked [`super::isa::simd_available`].
#[inline]
unsafe fn matrixing_simd(s: &[f32; SUBBANDS]) -> [f32; 64] {
    #[cfg(target_arch = "x86_64")]
    // SAFETY: forwards this fn's contract (the caller checked `simd_available()`).
    unsafe {
        matrixing_avx(s)
    }
    #[cfg(target_arch = "aarch64")]
    // SAFETY: forwards this fn's contract (the caller checked `simd_available()`).
    unsafe {
        matrixing_neon(s)
    }
    #[cfg(not(any(target_arch = "x86_64", target_arch = "aarch64")))]
    matrixing_fast(s)
}

/// Map the 32 distinct DCT outputs onto the 64 `V` values by sign and index
/// (`V[16]` is identically zero). Shared by the scalar and AVX matrixing.
#[inline]
fn expand_g(g: &[f32; 32]) -> [f32; 64] {
    let mut vv = [0f32; 64];
    for (i, vi) in vv.iter_mut().enumerate() {
        *vi = if i < 16 {
            g[16 + i]
        } else if i == 16 {
            0.0
        } else if i < 48 {
            -g[48 - i]
        } else {
            -g[i - 48]
        };
    }
    vv
}

/// **D4** — AVX twin of the 16-tap windowing sum, 8 outputs per lane.
///
/// BIT-IDENTICAL to the scalar loop, not merely close: each output `j` stays in
/// its own lane and accumulates its 16 taps in the original order (i ascending,
/// the `a` term before the `b` term), and separate `mul`+`add` are used rather
/// than FMA — an FMA would round once where the scalar rounds twice. Pinned by
/// `window_simd_matches_scalar`.
///
/// Worth doing because auto-vectorization demonstrably did NOT happen here:
/// `--emit asm` on the scalar version counts 0 packed ops against 1698 scalar
/// ones, even after the operands were made contiguous.
///
/// # Safety
/// Caller must have verified AVX is available. All memory bounds are established
/// inside (fixed-size arrays; `head` is masked to a multiple of 64).
#[cfg(target_arch = "x86_64")]
#[target_feature(enable = "avx")]
unsafe fn window_avx(fifo: &[f32; 1024], d: &[f32; 512], head: usize, out: &mut [f32; 32]) {
    use std::arch::x86_64::*;
    // `polyphase` only ever passes a multiple of 64; masking makes the bounds
    // below hold for ANY `head`, so the kernel's soundness is local (one `and`).
    let head = head & !63;
    // SAFETY: a register constant -- no memory access; the ISA is this fn's contract. Redundant from Rust 1.86 (safe target-feature calls), required at the crate's MSRV (1.85).
    #[allow(unused_unsafe)]
    let mut acc = [unsafe { _mm256_setzero_ps() }; 4];
    for i in 0..8 {
        // `a` is masked with 960, `b` with 1023, and the asymmetry is the point.
        // `head` starts at 0 and advances by `(head + 1024 - 64) & 1023`, so it is
        // always 64-aligned and `head + i * 128` has its low six bits clear -- the
        // narrower mask is the identity for `a` AND proves `a <= 960`, hence
        // `a + 32 <= 1024`, removing the span's bounds check. It is NOT valid for
        // `b`: `+ 96` is not 64-aligned (96 mod 64 = 32), so masking bit 5 away
        // there destroys signal. Tried; it moved the decode hash and failed two
        // reconstruction tests.
        let a = (head + i * 128) & 0x3C0; // 960
        let b = (head + i * 128 + 96) & 1023;
        for (v, accv) in acc.iter_mut().enumerate() {
            // SAFETY: `head` is masked to a multiple of 64 above, so `a <= 960` and `b` is 32 mod 64 with `b <= 992`: both 32-float spans (`+ v * 8 + 8 <= + 32`) end inside `fifo` (1024). `d` offsets reach `7 * 64 + 32 + 24 + 8 = 512`, its length.
            unsafe {
                let fa = _mm256_loadu_ps(fifo.as_ptr().add(a + v * 8));
                let da = _mm256_loadu_ps(d.as_ptr().add(i * 64 + v * 8));
                *accv = _mm256_add_ps(*accv, _mm256_mul_ps(fa, da));
                let fb = _mm256_loadu_ps(fifo.as_ptr().add(b + v * 8));
                let db = _mm256_loadu_ps(d.as_ptr().add(i * 64 + 32 + v * 8));
                *accv = _mm256_add_ps(*accv, _mm256_mul_ps(fb, db));
            }
        }
    }
    for (v, accv) in acc.iter().enumerate() {
        // SAFETY: `out` is `[f32; 32]`; `v * 8 + 8 <= 32` for `v < 4`.
        unsafe { _mm256_storeu_ps(out.as_mut_ptr().add(v * 8), *accv) };
    }
}

/// NEON twin of [`window_avx`]: 4 outputs per lane, 8 accumulators, the same tap
/// order (i ascending, `a` before `b`) with separate `vmulq` + `vaddq`. Pinned by
/// `window_simd_matches_scalar` under qemu-aarch64.
///
/// # Safety
/// NEON is baseline on AArch64; all memory bounds are established inside
/// (fixed-size arrays; `head` is masked to a multiple of 64).
#[cfg(target_arch = "aarch64")]
unsafe fn window_neon(fifo: &[f32; 1024], d: &[f32; 512], head: usize, out: &mut [f32; 32]) {
    use std::arch::aarch64::*;
    // See `window_avx`: masking makes the spans' bounds independent of the caller.
    let head = head & !63;
    // SAFETY: a register constant -- no memory access; the ISA is this fn's contract. Redundant from Rust 1.86 (safe target-feature calls), required at the crate's MSRV (1.85).
    #[allow(unused_unsafe)]
    let mut acc = [unsafe { vdupq_n_f32(0.0) }; 8];
    for i in 0..8 {
        let a = (head + i * 128) & 0x3C0; // 960
        let b = (head + i * 128 + 96) & 1023;
        for (v, accv) in acc.iter_mut().enumerate() {
            // SAFETY: `head` is masked to a multiple of 64 above, so `a <= 960` and `b <= 992`: both 32-float spans (`+ v * 4 + 4 <= + 32`) end inside `fifo` (1024). `d` offsets reach `7 * 64 + 32 + 28 + 4 = 512`, its length.
            unsafe {
                let fa = vld1q_f32(fifo.as_ptr().add(a + v * 4));
                let da = vld1q_f32(d.as_ptr().add(i * 64 + v * 4));
                *accv = vaddq_f32(*accv, vmulq_f32(fa, da));
                let fb = vld1q_f32(fifo.as_ptr().add(b + v * 4));
                let db = vld1q_f32(d.as_ptr().add(i * 64 + 32 + v * 4));
                *accv = vaddq_f32(*accv, vmulq_f32(fb, db));
            }
        }
    }
    for (v, accv) in acc.iter().enumerate() {
        // SAFETY: `out` is `[f32; 32]`; `v * 4 + 4 <= 32` for `v < 8`.
        unsafe { vst1q_f32(out.as_mut_ptr().add(v * 4), *accv) };
    }
}

/// This arch's SIMD windowing (AVX / NEON / scalar).
///
/// # Safety
/// Caller must have checked [`super::isa::simd_available`], and `head` must be a
/// multiple of 64.
#[inline]
unsafe fn window_simd(fifo: &[f32; 1024], d: &[f32; 512], head: usize, out: &mut [f32; 32]) {
    #[cfg(target_arch = "x86_64")]
    // SAFETY: forwards this fn's contract (the caller checked `simd_available()`).
    unsafe {
        window_avx(fifo, d, head, out);
    }
    #[cfg(target_arch = "aarch64")]
    // SAFETY: forwards this fn's contract (the caller checked `simd_available()`).
    unsafe {
        window_neon(fifo, d, head, out)
    }
    #[cfg(not(any(target_arch = "x86_64", target_arch = "aarch64")))]
    window_scalar(fifo, d, head, out)
}

/// The scalar windowing sum — the oracle the SIMD twin is gated against, and the
/// fallback on machines without AVX.
#[inline]
/// One 32-sample output block of the polyphase window.
///
/// `out` is a fixed-size block, not a slice: as `&mut [f32]` the compiler cannot
/// know it holds 32 elements, so each of the two accumulations per tap carried a
/// bounds check -- 512 of them per call, in the stage that is ~30% of decode.
///
/// REFUTED, kept as a warning: masking the FIFO offsets with 960 instead of 1023
/// looked equivalent -- `head` is 64-aligned, so `head + i * 128` has its low six
/// bits clear -- and would have proven `a + 32 <= 1024`, removing the span checks.
/// It is wrong for the SECOND offset: `+ 96` is not 64-aligned (96 mod 64 = 32), so
/// the narrower mask clears a bit that carries signal. Caught immediately by the
/// decode hash and two reconstruction tests.
fn window_scalar(fifo: &[f32; 1024], d: &[f32; 512], head: usize, out: &mut [f32; 32]) {
    out.fill(0.0);
    for i in 0..8 {
        // `a` is masked with 960, `b` with 1023, and the asymmetry is the point.
        // `head` starts at 0 and advances by `(head + 1024 - 64) & 1023`, so it is
        // always 64-aligned and `head + i * 128` has its low six bits clear -- the
        // narrower mask is the identity for `a` AND proves `a <= 960`, hence
        // `a + 32 <= 1024`, removing the span's bounds check. It is NOT valid for
        // `b`: `+ 96` is not 64-aligned (96 mod 64 = 32), so masking bit 5 away
        // there destroys signal. Tried; it moved the decode hash and failed two
        // reconstruction tests.
        let a = (head + i * 128) & 0x3C0; // 960
        let b = (head + i * 128 + 96) & 1023;
        let (fa, fb) = (&fifo[a..a + 32], &fifo[b..b + 32]);
        let (da, db) = (&d[i * 64..i * 64 + 32], &d[i * 64 + 32..i * 64 + 64]);
        for j in 0..32 {
            out[j] += fa[j] * da[j];
            out[j] += fb[j] * db[j];
        }
    }
}

/// Raw SIMD availability, for the oracle tests (they must run the kernel whatever
/// `MP3_ISA` says). The shipping path asks [`super::isa::use_simd`], resolved
/// once per call rather than per pass.
#[cfg(test)]
fn have_simd() -> bool {
    super::isa::simd_available()
}

/// Run the synthesis filterbank for one channel's granule (subband-major `time`),
/// returning 576 PCM samples. `fifo` is the persistent `V[]` state.
pub fn polyphase(time: &[f32; GRANULE_LINES], fifo: &mut [f32; 1024]) -> [f32; GRANULE_LINES] {
    // Resolved once per call, not per pass (codec-measurement: the A/B switch
    // itself is measurement overhead if it sits in the hot loop).
    let simd = super::isa::use_simd();
    super::prof::kernel_tally(
        if simd {
            &super::prof::K_SYNTH_SIMD
        } else {
            &super::prof::K_SYNTH_SCALAR
        },
        SUBBAND_LINES as u64,
    );
    // One `#[target_feature]` boundary per granule rather than two per pass: the
    // AVX entry compiles the whole body with AVX, so `matrixing_avx` and
    // `window_avx` inline instead of being called 36 times (see `imdct::hybrid`).
    #[cfg(target_arch = "x86_64")]
    if simd {
        // SAFETY: on x86_64 `use_simd()` is true only when AVX was detected.
        return unsafe { polyphase_avx(time, fifo) };
    }
    polyphase_impl(time, fifo, simd)
}

/// [`polyphase_impl`] compiled with AVX enabled, so the kernel calls inside inline.
///
/// # Safety
/// AVX must be available.
#[cfg(target_arch = "x86_64")]
#[target_feature(enable = "avx")]
unsafe fn polyphase_avx(
    time: &[f32; GRANULE_LINES],
    fifo: &mut [f32; 1024],
) -> [f32; GRANULE_LINES] {
    polyphase_impl(time, fifo, true)
}

/// The synthesis body; `simd` selects the kernel twins.
// Load-bearing: the body must inline INTO the `#[target_feature]` entry so the
// kernels inline with it (docs/plans/mp3-kernel-ledger.md, brick 4).
#[allow(clippy::inline_always)]
#[inline(always)]
fn polyphase_impl(
    time: &[f32; GRANULE_LINES],
    fifo: &mut [f32; 1024],
    simd: bool,
) -> [f32; GRANULE_LINES] {
    let d = &SYNTH_D;
    let mut pcm = [0f32; GRANULE_LINES];

    // **D2** — the V FIFO is addressed circularly rather than shifted. Logical
    // index `L` (0 = newest) lives at physical `(head + L) & 1023`, so advancing
    // the FIFO by 64 is a subtraction on `head` instead of a 960-float memmove.
    //
    // The old form moved 960 floats on every one of the 18 passes — 17,280 float
    // moves per granule per channel, ~17.5 GB across a 7-minute track. Here the
    // only movement is one 1024-float rotation at the end, 16.9x less.
    //
    // Every span stays contiguous, so nothing is given up in exchange: `head` is
    // always a multiple of 64, `i*128` a multiple of 128, and 1024 a multiple of
    // 32 — so each 32-long run sits inside one 32-aligned block and never wraps
    // mid-run.
    let mut head = 0usize;

    for v in 0..SUBBAND_LINES {
        // Gather this pass's 32 subband samples.
        let mut s = [0f32; SUBBANDS];
        for (k, sv) in s.iter_mut().enumerate() {
            *sv = time[k * SUBBAND_LINES + v];
        }
        // Advance V by 64 and matrix the new 64 values into the front.
        head = (head + 1024 - 64) & 1023;
        let vv = if simd {
            // SAFETY: `simd` came from `use_simd()` above.
            unsafe { matrixing_simd(&s) }
        } else {
            matrixing_fast(&s)
        };
        fifo[head..head + 64].copy_from_slice(&vv);
        // Build U from V, window with D, sum 16 taps → one PCM sample per j.
        //
        // `i` outer, `j` inner: each of the four operands is then a CONTIGUOUS
        // 32-float run, which auto-vectorizes. The old `j`-outer form walked the
        // FIFO with a stride of 128 floats in its inner loop, which cannot
        // vectorize, and it recomputed the two circular indices 32 times over.
        //
        // Each `out[j]` still accumulates its 16 taps in the original order
        // (i ascending, the `a` term before the `b` term), so this is
        // bit-identical, not merely close.
        let out: &mut [f32; 32] = pcm[v * 32..]
            .first_chunk_mut()
            .expect("granule is 18 blocks of 32");
        if simd {
            // SAFETY: `simd` was resolved from `use_simd()` before the loop;
            // `head` is a multiple of 64 so both spans leave 32 floats in bounds.
            unsafe { window_simd(fifo, d, head, out) };
        } else {
            window_scalar(fifo, d, head, out);
        }
    }
    // Restore the canonical `head == 0` layout the caller's state is defined in:
    // 18 passes advanced head by 18*64 = 1152 ≡ 128 (mod 1024), leaving it at 896.
    fifo.rotate_left(head);
    pcm
}

#[cfg(test)]
mod tests {
    use super::*;

    /// **D5 gate.** The AVX matrixing must be BIT-identical to the scalar
    /// `matrixing_fast`. It can be, because `m` is the lane: each of the 32
    /// outputs accumulates its 16 terms in the original `k` order, so nothing is
    /// reassociated. (Reducing over `k` instead — the natural SIMD shape — would
    /// be a horizontal sum and would NOT be bit-identical.)
    #[test]
    fn matrixing_simd_matches_scalar() {
        if !have_simd() {
            eprintln!("no SIMD twin on this host - scalar path only, gate skipped");
            return;
        }
        let mut st = 0x3C6E_F372u32;
        let mut rng = || {
            st = st.wrapping_mul(1_664_525).wrapping_add(1_013_904_223);
            (st >> 8) as f32 / (1u32 << 24) as f32 - 0.5
        };
        for trial in 0..64 {
            let mut s = [0f32; SUBBANDS];
            for v in &mut s {
                *v = rng();
            }
            let a = matrixing_fast(&s);
            // SAFETY: gated on `have_simd()` above.
            let b = unsafe { matrixing_simd(&s) };
            assert_eq!(a, b, "matrixing SIMD/scalar mismatch on trial {trial}");
        }
    }

    /// **D4 gate.** The AVX windowing twin must be BIT-identical to the scalar
    /// oracle — not within a tolerance. It can be, because each output stays in
    /// its own lane and accumulates in the original order, and the kernel uses
    /// separate mul+add rather than FMA (an FMA rounds once where the scalar
    /// rounds twice, which would change the result).
    ///
    /// Swept over every legal `head` (multiples of 64) so the circular wrap is
    /// covered, with random FIFO contents.
    #[test]
    fn window_simd_matches_scalar() {
        if !have_simd() {
            eprintln!("no SIMD twin on this host - scalar path only, gate skipped");
            return;
        }
        let mut st = 0x7F4A_7C15u32;
        let mut rng = || {
            st = st.wrapping_mul(1_664_525).wrapping_add(1_013_904_223);
            (st >> 8) as f32 / (1u32 << 24) as f32 - 0.5
        };
        let mut fifo = [0f32; 1024];
        for f in &mut fifo {
            *f = rng();
        }
        let d = &SYNTH_D;
        for head in (0..1024).step_by(64) {
            let mut a = [0f32; 32];
            let mut b = [0f32; 32];
            window_scalar(&fifo, d, head, &mut a);
            // SAFETY: gated on `have_simd()`; `head` steps by 64.
            unsafe { window_simd(&fifo, d, head, &mut b) };
            assert_eq!(a, b, "SIMD/scalar mismatch at head={head}");
        }
    }

    #[test]
    fn matrix_matches_cosine_formula() {
        let n = matrix();
        // Spot-check a couple of entries against N[i][k] = cos((16+i)(2k+1)π/64).
        let e =
            |i: usize, k: usize| (PI / 64.0 * (16 + i) as f64 * (2 * k + 1) as f64).cos() as f32;
        assert!((n[0][0] - e(0, 0)).abs() < 1e-6);
        assert!((n[33][7] - e(33, 7)).abs() < 1e-6);
        assert!((n[63][31] - e(63, 31)).abs() < 1e-6);
    }

    /// The dense 64×32 matrixing — the reference `matrixing_fast` must reproduce.
    fn matrixing_dense(s: &[f32; SUBBANDS]) -> [f32; 64] {
        let n = matrix();
        let mut v = [0f32; 64];
        for (i, vi) in v.iter_mut().enumerate() {
            let mut acc = 0f32;
            for k in 0..SUBBANDS {
                acc += n[i][k] * s[k];
            }
            *vi = acc;
        }
        v
    }

    /// **B3 gate.** The fast matrixing must equal the dense reference to float
    /// precision (the symmetries are exact; only ULP reassociation differs) — for
    /// many random subband inputs. A bug in the index/sign map shows up as a large
    /// error here, long before it could reach the FFmpeg-conformance check.
    #[test]
    #[cfg_attr(
        miri,
        ignore = "end-to-end / exhaustive: too slow to interpret under Miri (> 5 min)"
    )]
    fn fast_matrixing_matches_dense() {
        let mut st = 0x1234_9ABCu32;
        let mut rng = || {
            st = st.wrapping_mul(1_664_525).wrapping_add(1_013_904_223);
            (st >> 8) as f32 / (1u32 << 24) as f32 * 2.0 - 1.0
        };
        let mut worst = 0f32;
        for _ in 0..2000 {
            let mut s = [0f32; SUBBANDS];
            for sv in &mut s {
                *sv = rng() * 100.0;
            }
            let dense = matrixing_dense(&s);
            let fast = matrixing_fast(&s);
            for i in 0..64 {
                // relative error vs the row's scale (sum of |s|).
                let scale = s.iter().map(|x| x.abs()).sum::<f32>().max(1.0);
                worst = worst.max((dense[i] - fast[i]).abs() / scale);
            }
        }
        eprintln!("[B3] worst relative matrixing error fast vs dense: {worst:.2e}");
        assert!(
            worst < 1e-5,
            "fast matrixing diverges from dense: {worst:.2e}"
        );
    }

    #[test]
    fn fifo_advances_and_output_is_finite() {
        // Feed every pass (all-ones) so the FIFO stays populated across the 18
        // shifts. With the placeholder D (zeros) the PCM is zero but finite; the
        // matrixing/FIFO must run cleanly and leave the FIFO non-empty.
        let time = [1f32; GRANULE_LINES];
        let mut fifo = [0f32; 1024];
        let pcm = polyphase(&time, &mut fifo);
        assert!(pcm.iter().all(|v| v.is_finite()));
        assert!(
            fifo.iter().any(|&v| v != 0.0),
            "matrixing must fill the FIFO"
        );
    }
}
