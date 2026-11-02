// Modified from rusty_mp3 1.0.0 (see Cargo.toml).
//! The two-loop quantizer — rate control + noise shaping.
//!
//! * **Inner loop (rate):** raise the quantization step until the Huffman-coded
//!   spectrum fits the granule's bit budget.
//! * **Outer loop (distortion):** raise per-band scalefactors where quantization
//!   noise exceeds the psychoacoustic threshold, re-running the inner loop, until
//!   noise is masked everywhere or no scalefactor budget remains.
//!
//! Produces the quantized integer spectrum plus the side-info fields (global
//! gain, scalefactors, `scalefac_compress`, block flags) that describe it.

use std::sync::OnceLock;

use crate::frame::{BlockType, GranuleSideInfo, GRANULE_LINES};
use crate::header::FrameHeader;

use super::psychoacoustic::PsyResult;

// ── Brick N4: the nonuniform quantizer power law ──────────────────────────────
//
// The decoder requantizes `xr = sign(is)·|is|^(4/3)·scale` (see
// `decode/requantize.rs`). The encoder inverts the `|is|^(4/3)` core: a magnitude
// `xr` quantizes to the integer level `ix = nint(|xr|^(3/4) − BIAS)`. The two are
// exact inverses on the integer lattice — `quantize_level(requant_magnitude(ix))
// == ix` for every representable `ix` — which is N4's verification gate. The
// global-gain / scalefactor `scale` is applied by the rate loop (C2); N4 is the
// unit-step power law it builds on.

/// Largest Huffman magnitude reachable with `linbits` (matches the decoder).
pub const MAX_LEVEL: i32 = 8206;

/// ISO rounding bias subtracted before the round in the forward quantizer
/// (ISO/IEC 11172-3 2.4.2.7). It biases the decision boundary so the truncating
/// `nint` recovers the intended level.
pub const QUANT_BIAS: f64 = 0.0946;

/// `|level|^(4/3)`, table-backed for the `0..=MAX_LEVEL` magnitudes — the
/// requantization power law (the same curve the decoder applies, factored out so
/// the encoder's rate loop can predict what the decoder will reconstruct).
pub fn requant_magnitude(level: i32) -> f64 {
    static T: OnceLock<Vec<f64>> = OnceLock::new();
    let t = T.get_or_init(|| {
        (0..=MAX_LEVEL as usize)
            .map(|i| (i as f64).powf(4.0 / 3.0))
            .collect()
    });
    let a = level.unsigned_abs() as usize;
    t.get(a)
        .copied()
        .unwrap_or_else(|| (a as f64).powf(4.0 / 3.0))
}

/// Forward-quantize a (positive) frequency-line magnitude to its integer level
/// under unit step: `ix = nint(|xr|^(3/4) − BIAS)`, clamped to `[0, MAX_LEVEL]`.
/// The sign is carried separately, exactly as the bitstream does.
#[must_use]
pub fn quantize_level(xr: f64) -> i32 {
    level_from(xr.abs().powf(0.75))
}

/// Round a pre-powered magnitude `p = |xr|^(3/4)` to its quantized level:
/// `nint(p − BIAS)`, clamped to `[0, MAX_LEVEL]`. Factored out so the hot rate
/// loop can supply `p` from a precomputed `|xr|^(3/4)` (see [`xrpow`]).
///
/// **C (vectorization):** written branchlessly — clamp in `f64` before the cast
/// instead of an `if m <= 0` guard — so the per-line quantize loops auto-vectorize.
/// Byte-identical to the guarded form: for `m ≤ 0`, `round(m).clamp(0,·)` is `0`
/// (round of a non-positive is ≤ 0); for `m > 0` it is `round(m).min(MAX)`, the
/// same as the old `i32` clamp since `round(m) ≥ 0`. Pinned by `level_from_*` tests.
#[inline]
pub(crate) fn level_from(powered: f64) -> i32 {
    let m = powered - QUANT_BIAS;
    // Round-half-AWAY-from-zero, then clamp — with no call and no branch.
    //
    // `f64::round` is ties-away-from-zero, which **no x86 instruction
    // implements**, so it lowers to a libm CALL. One call per element is a hard
    // barrier that keeps the whole loop scalar however branchless the rest of it
    // is, and the emitted assembly confirmed it: `inner_gain` — ~59% of encode
    // time — held a `call round` per frequency line and **zero** packed
    // operations. (`floor`/`ceil`/`trunc` are no better: they need SSE4.1, above
    // the portable x86-64 baseline.)
    //
    // The result is clamped to `[0, MAX_LEVEL]` regardless, and over that
    // non-negative domain rounding half away from zero is exactly "add a half and
    // truncate toward zero" — and truncation IS baseline (`cvttsd2si`, which
    // vectorizes as `cvttpd2dq`). Clamping *before* the add keeps the value in
    // range so the cast cannot saturate, and makes the sequence pure arithmetic.
    //
    // Exact at the tie points, not merely close: `level_from_matches_guarded`
    // sweeps the working range and every half-integer midpoint against the
    // original guarded form. The magic-number ties-to-EVEN trick was tried first
    // and rejected here — it is bit-identical on all 24 corpus encodes but
    // differs at constructed ties, and weakening a standing exactness gate to buy
    // speed is the wrong trade when an exact form is available.
    (m.clamp(0.0, f64::from(MAX_LEVEL)) + 0.5) as i32
}

/// **A1** — precompute `|freq[i]|^(3/4)` for the whole granule, once. The forward
/// quantizer needs `(|freq|·scale)^(3/4)`; since that equals `|freq|^(3/4)·scale^(3/4)`,
/// hoisting the per-line `powf` out of the rate/distortion loops turns each later
/// quantize pass into a multiply-and-round. (The two factor orders differ only by a
/// last-ULP rounding, which the byte-identical-output gate verifies in practice.)
///
/// **Prometheus keeper `perf001`** (profiled: quantize = 62.7% of encode time;
/// this `powf` is its hot transcendental). Strength reduction: `x^(3/4) = √(x·√x)`
/// — two hardware `sqrt`s replace one libm `powf`, ~8× on this kernel at
/// **byte-identical** quantizer output (1 ULP absorbed by integer rounding).
///
/// **`perf003` (AVX2 xrpow) — PRUNED 2026-07-08, and its stated REASON was wrong
/// (corrected 2026-09-10).** The prune stands; the explanation did not.
///
/// The old note claimed this loop "already auto-vectorizes" because `sqrt` is
/// SSE2-baseline, so perf001 had captured the SIMD win implicitly. **It does not
/// vectorize.** The emitted assembly contains `sqrtsd` here and **not one
/// `sqrtpd` anywhere in the crate** — the loop is scalar, and a strength
/// reduction to a baseline-SIMD op does not by itself make a loop wide.
///
/// The real reason not to vectorize it is SIZE, measured rather than argued. A
/// doubling probe (compute `xrpow` twice, discard one — a stub probe is invalid
/// because these values steer the gain search) puts its marginal cost at ~0.8%
/// of encode on the min, against an arithmetic estimate of ~2.9%; the two
/// dependent `sqrt`s per element make it latency-bound, and a doubling probe
/// reads LOW on latency-bound code, so the truth is between. Halving that with
/// 2-wide SSE2 buys ~1%, which is inside this box's noise floor — and that, not
/// an imagined auto-vectorization, is why perf003 measured 0.97x.
///
/// Left as-is deliberately. Recorded so the next reader does not conclude from
/// the old comment that `sqrt` loops in this crate are already wide.
#[must_use]
pub fn xrpow(freq: &[f32; GRANULE_LINES]) -> [f64; GRANULE_LINES] {
    let mut p = [0f64; GRANULE_LINES];
    if xrpow_use_powf() {
        for (pi, &f) in p.iter_mut().zip(freq.iter()) {
            *pi = f64::from(f.abs()).powf(0.75); // the scalar `powf` oracle
        }
    } else {
        for (pi, &f) in p.iter_mut().zip(freq.iter()) {
            // x^0.75 = x^0.5 · x^0.25 = √(x·√x). 1 ULP vs powf, absorbed by the
            // quantizer's integer rounding. `sqrt` is SSE2-baseline so this loop
            // auto-vectorizes for free (see perf003 prune above).
            let a = f64::from(f.abs());
            *pi = (a * a.sqrt()).sqrt();
        }
    }
    p
}

/// Whether to use the original `powf` path (the oracle). Read once — an env
/// lookup per granule would perturb the very timing this optimizes. Default is
/// the fast `sqrt` identity; `RFF_MP3_XRPOW=powf` forces the oracle.
///
/// Vendored (studio): the `RFF_MP3_XRPOW` override is removed; the default.
fn xrpow_use_powf() -> bool {
    false
}

/// One granule's quantized output.
#[derive(Debug, Clone)]
pub struct QuantizedGranule {
    /// Quantized integer spectrum (`is`), 576 lines.
    pub coeffs: [i32; GRANULE_LINES],
    /// Side-info describing how to dequantize it (gain, tables, regions, flags).
    pub side: GranuleSideInfo,
    /// Scalefactors per band (long: 22; short: 3×13 packed).
    pub scalefactors: [u8; 39],
}

impl Default for QuantizedGranule {
    fn default() -> Self {
        // Arrays larger than 32 don't derive Default.
        Self {
            coeffs: [0; GRANULE_LINES],
            side: GranuleSideInfo::default(),
            scalefactors: [0; 39],
        }
    }
}

/// `2^(0.75 · SF_MULT · s)`, indexed by the raw `u8` scalefactor.
///
/// Sized 256 rather than `MAX_SF + 1` on purpose: the index is a `u8` from a
/// `[u8; 22]`, and nothing tells the compiler it is <= 15, so a 16-entry table
/// costs a `cmp` and a branch-to-panic on EVERY band of every gain probe. A table
/// covering the whole index type is provably in range, so the guard disappears.
/// Only the first sixteen entries are ever read, so the rest never load.
///
/// The per-band step is `2^(0.75·(base + SF_MULT·s))`, which factors exactly into
/// a per-GRANULE `2^(0.75·base)` times a per-BAND term that depends only on the
/// scalefactor — and `s` is an integer in `0..=MAX_SF`, so that term is a
/// 16-entry table. `exp2` has no SIMD instruction and cannot be vectorized, so
/// trading 22 of them per call for one plus 22 multiplies is the whole point.
///
/// Prometheus `perf002` pruned this factoring on 2026-07-08 as "byte-identical
/// but not measurably faster". That measurement was taken while `level_from`
/// still called libm `round` once per frequency line, which dominated the same
/// loop; with that call gone the baseline moved, and a refutation expires when
/// its baseline moves.
fn sf_step_lut() -> &'static [f64; 256] {
    static T: OnceLock<[f64; 256]> = OnceLock::new();
    T.get_or_init(|| {
        let mut t = [0f64; 256];
        for (s, v) in t.iter_mut().enumerate() {
            *v = 2f64.powf(0.75 * SF_MULT * s as f64);
        }
        t
    })
}

/// `2^(-SF_MULT · s)` — the requantization mirror of [`sf_step_lut`].
fn sf_inv_lut() -> &'static [f64; 256] {
    static T: OnceLock<[f64; 256]> = OnceLock::new();
    T.get_or_init(|| {
        let mut t = [0f64; 256];
        for (s, v) in t.iter_mut().enumerate() {
            *v = 2f64.powf(-SF_MULT * s as f64);
        }
        t
    })
}

/// Largest non-clipping quantized level. Above this the value would saturate at
/// `MAX_LEVEL`, losing precision — so a gain that produces it is *too fine*.
const MAX_UNCLIPPED: i32 = 8191;
/// Scalefactor multiplier when `scalefac_scale = 0` (the half-step we use).
const SF_MULT: f64 = 0.5;
/// Largest scalefactor value (a 4-bit `slen` field caps it).
const MAX_SF: u8 = 15;

/// Largest scalefactor the bitstream can actually carry for band `b`.
///
/// `scalefac_compress` selects one `(slen1, slen2)` pair for the whole granule:
/// `slen1` bits for bands 0..10, `slen2` for bands 11..20. The MPEG-1 table
/// ([`crate::tables::SCALEFAC_COMPRESS_V1`]) tops out at `slen1 = 4` but only
/// `slen2 = 3` — so the high group holds 0..7, not 0..15.
///
/// Amplifying past this does not fail loudly: `choose_compress` finds no
/// covering entry, falls back to `(4, 3)`, and the serializer writes the value
/// with 3 bits, silently truncating 8..15 down to 0..7. The decoder then
/// requantizes that band with a scalefactor up to 16x too small and it comes
/// back far too loud — one granule of near-full-scale garbage (measured: peak
/// 1.92 against a source peaking at 1.30, max sample error 1.24).
///
/// It stayed dormant only because nothing shaped: the CBR loop was inert, and
/// `loops_vbr` amplifies rarely enough that `prof::SF_OVERFLOW` counts 0 across
/// the whole `-q:a` ladder on the music corpus. The moment the slack refinement
/// started shaping, it corrupted a granule on the first clip.
#[inline]
fn max_sf(b: usize) -> u8 {
    if b < 11 {
        MAX_SF
    } else {
        7
    }
}
/// Outer distortion-loop iteration cap.
const MAX_OUTER: usize = 24;

/// Quantize one granule at `global_gain` with per-band scalefactors applied:
/// band `b` is amplified by `2^(SF_MULT·sf[b])` before quantizing (finer step →
/// less noise there), the forward of the decoder's per-band requantization.
fn quantize_with_sf(
    header: &FrameHeader,
    freq: &[f32; GRANULE_LINES],
    xrp: &[f64; GRANULE_LINES],
    gain: i32,
    sf: &[u8; 22],
) -> [i32; GRANULE_LINES] {
    let mut out = [0i32; GRANULE_LINES];
    quantize_into(header, freq, xrp, gain, sf, &mut out);
    out
}

/// [`quantize_with_sf`] writing through a caller-owned buffer.
///
/// The by-value form returns `[i32; 576]` — 2,304 bytes — and the rate loop calls
/// it once per gain probe, so the return alone moved tens of megabytes per clip.
/// Every line of `out` is written here (the band loop spans all 576), so the
/// caller's buffer needs no pre-clear.
fn quantize_into(
    header: &FrameHeader,
    freq: &[f32; GRANULE_LINES],
    xrp: &[f64; GRANULE_LINES],
    gain: i32,
    sf: &[u8; 22],
    out: &mut [i32; GRANULE_LINES],
) {
    let off = crate::tables::sfb_long_offsets(header.sample_rate);
    let base = -0.25 * f64::from(gain - 210);
    // One `exp2` for the granule; the per-band factor is a table lookup.
    let step_base = 2f64.powf(0.75 * base);
    let sf_step = sf_step_lut();
    // Band 21 is uncoded, i.e. always scalefactor 0. Expressing that as
    // `if b < 21 { sf[b] } else { 0 }` puts two compares and a branch INSIDE the
    // band loop for a condition true 21 times out of 22; zeroing the entry once
    // folds it out.
    let mut sfx = *sf;
    sfx[21] = 0;
    let mut steps = [0f64; 22];
    for b in 0..22 {
        let s = sfx[b];
        // step = scale_inv^(3/4): the per-band factor applied to the precomputed
        // |freq|^(3/4), instead of re-powering |freq|·scale_inv per line.
        //
        // Prometheus `perf002` (PRUNED 2026-07-08): factoring this into
        // `2^(0.75·base)·LUT[sf]` (per-band `powf`→LUT) was byte-identical but NOT
        // measurably faster (delta within run-to-run noise). Unlike xrpow's
        // `powf(0.75)` (arbitrary exponent → a real libm call → 8×), `2f64.powf(y)`
        // is base-2 and LLVM already lowers it to a fast `exp2` — so a LUT saves
        // nothing. Reverted; recorded in the Prometheus ledger.
        // VERIFIED 2026-07-08 at the asm level: rewriting these four base-2
        // `2f64.powf(x)` sites as explicit `x.exp2()` left the emitted call
        // counts identical (exp2=15/pow=10/powf=4 both ways) — the compiler
        // already does it, so `powf→exp2` is a genuine no-op. Don't re-try it.
        steps[b] = step_base * sf_step[s as usize];
    }
    let simd = crate::decode::isa::use_simd();
    if simd {
        // SAFETY: `use_simd()` checked the ISA; `off` spans exactly 0..576.
        unsafe { quantize_lines_simd(off, &steps, freq, xrp, out) };
    } else {
        quantize_lines_scalar(off, &steps, freq, xrp, out);
    }
    // Kernel-reach census, once per call (never per line).
    super::prof::QUANT_CALLS[usize::from(simd)].fetch_add(1, std::sync::atomic::Ordering::Relaxed);
}

/// The per-line quantizer for all 22 bands: `sign(freq)·level_from(xrp·step[b])`.
/// The scalar ORACLE for [`quantize_lines_simd`].
#[inline]
fn quantize_lines_scalar(
    off: &[u16; 23],
    steps: &[f64; 22],
    freq: &[f32; GRANULE_LINES],
    xrp: &[f64; GRANULE_LINES],
    out: &mut [i32; GRANULE_LINES],
) {
    for b in 0..22 {
        let step = steps[b];
        let (lo, hi) = (off[b] as usize, (off[b + 1] as usize).min(GRANULE_LINES));
        for i in lo..hi {
            let mag = level_from(xrp[i] * step);
            out[i] = if freq[i] < 0.0 { -mag } else { mag };
        }
    }
}

/// AVX twin of [`quantize_lines_scalar`], four lines per step. BIT-identical:
/// the same f64 multiply, `level_from`'s exact sequence (subtract the bias, clamp
/// to `[0, MAX_LEVEL]`, add one half, truncate -- `max_pd`/`min_pd` equal
/// `f64::clamp` on these finite non-NaN inputs, and `cvttpd2dq` is the same
/// truncation as `as i32`), and the sign applied as `(m ^ mask) - mask` where
/// `mask` is `freq < 0.0` (so `-0.0` keeps its magnitude, as the scalar does).
/// The rate loop calls this ~8x per granule; the 22-band loop lives INSIDE the
/// `#[target_feature]` function, so each call crosses that boundary once.
///
/// # Safety
/// AVX must be available.
#[cfg(target_arch = "x86_64")]
#[target_feature(enable = "avx")]
unsafe fn quantize_lines_avx(
    off: &[u16; 23],
    steps: &[f64; 22],
    freq: &[f32; GRANULE_LINES],
    xrp: &[f64; GRANULE_LINES],
    out: &mut [i32; GRANULE_LINES],
) {
    use std::arch::x86_64::*;
    let (bias, zero, max, half) = (
        _mm256_set1_pd(QUANT_BIAS),
        _mm256_setzero_pd(),
        _mm256_set1_pd(f64::from(MAX_LEVEL)),
        _mm256_set1_pd(0.5),
    );
    let fzero = _mm_setzero_ps();
    for b in 0..22 {
        let (lo, hi) = (off[b] as usize, (off[b + 1] as usize).min(GRANULE_LINES));
        let vstep = _mm256_set1_pd(steps[b]);
        let mut i = lo;
        while i + 4 <= hi {
            // SAFETY: the loop guard `i + 4 <= hi` with `hi <= GRANULE_LINES` keeps all four lanes inside `xrp`, `freq` and `out` (each `[_; 576]`), whatever `off` holds.
            unsafe {
                let p = _mm256_mul_pd(_mm256_loadu_pd(xrp.as_ptr().add(i)), vstep);
                let m = _mm256_sub_pd(p, bias);
                let c = _mm256_min_pd(_mm256_max_pd(m, zero), max);
                let mag = _mm256_cvttpd_epi32(_mm256_add_pd(c, half));
                let neg = _mm_castps_si128(_mm_cmplt_ps(_mm_loadu_ps(freq.as_ptr().add(i)), fzero));
                let q = _mm_sub_epi32(_mm_xor_si128(mag, neg), neg);
                // `storeu` is the UNALIGNED store: the cast pointer is never
                // dereferenced as an aligned `__m128i`.
                #[allow(clippy::cast_ptr_alignment)]
                _mm_storeu_si128(out.as_mut_ptr().add(i).cast::<__m128i>(), q);
            }
            i += 4;
        }
        let step = steps[b];
        while i < hi {
            let mag = level_from(xrp[i] * step);
            out[i] = if freq[i] < 0.0 { -mag } else { mag };
            i += 1;
        }
    }
}

/// NEON twin of [`quantize_lines_scalar`], two lines per step (f64x2). Same
/// sequence: `vmaxq`/`vminq` clamp, `+0.5`, `vcvtq_s64_f64` truncation, narrowed to
/// i32 (in range after the clamp), sign via `(m ^ mask) - mask`. BIT-identical.
///
/// # Safety
/// NEON is baseline on AArch64; every access is bounded inside (`hi` is capped at
/// `GRANULE_LINES`), whatever `off` holds.
#[cfg(target_arch = "aarch64")]
unsafe fn quantize_lines_neon(
    off: &[u16; 23],
    steps: &[f64; 22],
    freq: &[f32; GRANULE_LINES],
    xrp: &[f64; GRANULE_LINES],
    out: &mut [i32; GRANULE_LINES],
) {
    use std::arch::aarch64::*;
    // SAFETY: NEON is baseline on AArch64; every load/store is guarded by `i + 2 <= hi` with `hi <= GRANULE_LINES`, inside `xrp`, `freq` and `out` (each `[_; 576]`), whatever `off` holds.
    unsafe {
        let (bias, zero, max, half) = (
            vdupq_n_f64(QUANT_BIAS),
            vdupq_n_f64(0.0),
            vdupq_n_f64(MAX_LEVEL as f64),
            vdupq_n_f64(0.5),
        );
        for b in 0..22 {
            let (lo, hi) = (off[b] as usize, (off[b + 1] as usize).min(GRANULE_LINES));
            let vstep = vdupq_n_f64(steps[b]);
            let mut i = lo;
            while i + 2 <= hi {
                let p = vmulq_f64(vld1q_f64(xrp.as_ptr().add(i)), vstep);
                let c = vminq_f64(vmaxq_f64(vsubq_f64(p, bias), zero), max);
                let mag = vmovn_s64(vcvtq_s64_f64(vaddq_f64(c, half)));
                let neg =
                    vreinterpret_s32_u32(vclt_f32(vld1_f32(freq.as_ptr().add(i)), vdup_n_f32(0.0)));
                let q = vsub_s32(veor_s32(mag, neg), neg);
                vst1_s32(out.as_mut_ptr().add(i), q);
                i += 2;
            }
            let step = steps[b];
            while i < hi {
                let mag = level_from(xrp[i] * step);
                out[i] = if freq[i] < 0.0 { -mag } else { mag };
                i += 1;
            }
        }
    }
}

/// This arch's quantizer twin (AVX / NEON / scalar).
///
/// # Safety
/// Caller checked [`crate::decode::isa::simd_available`].
#[inline]
unsafe fn quantize_lines_simd(
    off: &[u16; 23],
    steps: &[f64; 22],
    freq: &[f32; GRANULE_LINES],
    xrp: &[f64; GRANULE_LINES],
    out: &mut [i32; GRANULE_LINES],
) {
    #[cfg(target_arch = "x86_64")]
    // SAFETY: forwards this fn's contract (the caller checked `simd_available()`).
    unsafe {
        quantize_lines_avx(off, steps, freq, xrp, out);
    }
    #[cfg(target_arch = "aarch64")]
    // SAFETY: forwards this fn's contract (the caller checked `simd_available()`).
    unsafe {
        quantize_lines_neon(off, steps, freq, xrp, out)
    }
    #[cfg(not(any(target_arch = "x86_64", target_arch = "aarch64")))]
    quantize_lines_scalar(off, steps, freq, xrp, out)
}

/// Re-quantize ONE band in place, leaving every other line untouched.
///
/// Each band's levels depend only on `gain` and its own scalefactor, so when the
/// refinement bumps band `b` the other twenty-one are already correct. Walking
/// all 576 lines to recompute them is the "once per candidate" redundancy;
/// byte-identical to a full [`quantize_with_sf`] because it runs the same
/// arithmetic on the same inputs for the lines it does touch.
fn quantize_band_in_place(
    header: &FrameHeader,
    freq: &[f32; GRANULE_LINES],
    xrp: &[f64; GRANULE_LINES],
    gain: i32,
    sf_b: u8,
    b: usize,
    coeffs: &mut [i32; GRANULE_LINES],
) {
    let off = crate::tables::sfb_long_offsets(header.sample_rate);
    let base = -0.25 * f64::from(gain - 210);
    let s = f64::from(if b < 21 { sf_b } else { 0 });
    let step = 2f64.powf(0.75 * (base + SF_MULT * s));
    let (lo, hi) = (off[b] as usize, (off[b + 1] as usize).min(GRANULE_LINES));
    for i in lo..hi {
        let mag = level_from(xrp[i] * step);
        coeffs[i] = if freq[i] < 0.0 { -mag } else { mag };
    }
}

/// Quantization-noise energy for ONE band -- the per-band half of [`band_noise`],
/// so the refinement can update only the band it changed.
fn band_noise_one(
    header: &FrameHeader,
    freq: &[f32; GRANULE_LINES],
    coeffs: &[i32; GRANULE_LINES],
    gain: i32,
    sf_b: u8,
    b: usize,
) -> f32 {
    let off = crate::tables::sfb_long_offsets(header.sample_rate);
    let scale = 2f64.powf(0.25 * f64::from(gain - 210)) * sf_inv_lut()[sf_b as usize];
    let (lo, hi) = (off[b] as usize, (off[b + 1] as usize).min(GRANULE_LINES));
    let mut e = 0f64;
    for i in lo..hi {
        let xr = f64::from(coeffs[i].signum()) * requant_magnitude(coeffs[i]) * scale;
        let d = f64::from(freq[i]) - xr;
        e += d * d;
    }
    e as f32
}

/// Per-band quantization-noise energy: `Σ (freq − requantized)²` over each of the
/// 21 coded long bands, using the decoder's exact requantization.
fn band_noise(
    header: &FrameHeader,
    freq: &[f32; GRANULE_LINES],
    coeffs: &[i32; GRANULE_LINES],
    gain: i32,
    sf: &[u8; 22],
) -> [f32; 21] {
    let off = crate::tables::sfb_long_offsets(header.sample_rate);
    let mut noise = [0f32; 21];
    // Same factoring as `quantize_with_sf`: the granule term is one `exp2`, the
    // per-band term is a table lookup on the integer scalefactor.
    let scale_base = 2f64.powf(0.25 * f64::from(gain - 210));
    let sf_inv = sf_inv_lut();
    for (b, n) in noise.iter_mut().enumerate() {
        let scale = scale_base * sf_inv[sf[b] as usize];
        let (lo, hi) = (off[b] as usize, (off[b + 1] as usize).min(GRANULE_LINES));
        let mut e = 0f64;
        for i in lo..hi {
            let xr = f64::from(coeffs[i].signum()) * requant_magnitude(coeffs[i]) * scale;
            let d = f64::from(freq[i]) - xr;
            e += d * d;
        }
        *n = e as f32;
    }
    noise
}

/// Bits to represent values `0..=v`.
fn bits_for(v: u8) -> u8 {
    if v == 0 {
        0
    } else {
        (8 - v.leading_zeros()) as u8
    }
}

/// Pick the smallest `scalefac_compress` covering the current scalefactors, and
/// its scalefactor-bit cost (`11·slen1 + 10·slen2`).
fn choose_compress(sf: &[u8; 22]) -> (u16, usize) {
    let max1 = sf[0..11].iter().copied().max().unwrap_or(0);
    let max2 = sf[11..21].iter().copied().max().unwrap_or(0);
    let (need1, need2) = (bits_for(max1), bits_for(max2));
    for (idx, &(slen1, slen2)) in crate::tables::SCALEFAC_COMPRESS_V1.iter().enumerate() {
        if slen1 >= need1 && slen2 >= need2 {
            return (idx as u16, 11 * slen1 as usize + 10 * slen2 as usize);
        }
    }
    // No covering entry. The fallback below writes the high group with 3 bits,
    // which TRUNCATES any value above 7 and hands the decoder a scalefactor up to
    // 16x too small. Callers must clamp with `max_sf`, so reaching here is a bug
    // in the caller, not a representable encoding choice.
    debug_assert!(
        false,
        "scalefactors exceed the scalefac_compress table (need slen1>={need1}, slen2>={need2}); \
         the caller must clamp each band with max_sf()"
    );
    (15, 11 * 4 + 10 * 3)
}

/// Huffman bit cost of a coefficient set under the best table selection for
/// `block_type` (long vs window-switched regions — the emit must use the same).
fn huff_cost(
    header: &FrameHeader,
    coeffs: &[i32; GRANULE_LINES],
    block_type: BlockType,
) -> (GranuleSideInfo, usize) {
    // C (redundancy): select already computed the winning bit cost while choosing
    // the tables — take it directly instead of re-walking the spectrum.
    super::huffman::select(header, coeffs, block_type)
}

fn inner_gain(
    header: &FrameHeader,
    freq: &[f32; GRANULE_LINES],
    xrp: &[f64; GRANULE_LINES],
    sf: &[u8; 22],
    huff_budget: usize,
    block_type: BlockType,
    out: &mut [i32; GRANULE_LINES],
) -> (i32, Option<GranuleSideInfo>) {
    // Two buffers, and the SWAP IS ON THE REFERENCES. `mem::swap` on the arrays
    // themselves would move 2 x 2,304 bytes and make this worse than the copy it
    // replaces; swapping `&mut` bindings exchanges two pointers.
    //
    // The search previously cloned the whole coefficient array into an
    // `InnerResult` on every FITTING probe -- about half of the eight probes --
    // and the by-value return moved it again. Now the winner is simply whichever
    // buffer `best` points at, and exactly one copy happens, at the end.
    let mut buf_a = [0i32; GRANULE_LINES];
    let mut buf_b = [0i32; GRANULE_LINES];
    let mut probe: &mut [i32; GRANULE_LINES] = &mut buf_a;
    let mut best: &mut [i32; GRANULE_LINES] = &mut buf_b;

    let mut best_side: Option<GranuleSideInfo> = None;
    let mut best_gain = i32::MIN;
    let (mut lo, mut hi) = (0i32, 255i32);
    while lo < hi {
        let mid = (lo + hi) / 2;
        quantize_into(header, freq, xrp, mid, sf, probe);
        // Short-circuit: the cheap no-clip scan first; only then the costly
        // `huff_cost` (table selection). The last fitting probe is the
        // leftmost-fits winner.
        let fits = probe.iter().all(|&c| c.abs() <= MAX_UNCLIPPED) && {
            let (side, bits) = huff_cost(header, probe, block_type);
            let fits = bits <= huff_budget;
            if fits {
                std::mem::swap(&mut probe, &mut best);
                best_side = Some(side);
                best_gain = mid;
            }
            fits
        };
        if fits {
            hi = mid;
        } else {
            lo = mid + 1;
        }
    }
    // Keep the result only if it is the winning gain (the invariant above); a
    // mismatch (winner never evaluated as fitting) makes the caller quantize
    // fresh, byte-identically.
    if best_gain == lo {
        out.copy_from_slice(best);
        (lo, best_side)
    } else {
        (lo, None)
    }
}

/// **C2 + Q6 — the two-loop quantizer.** The inner loop (`inner_gain`) hits the
/// bit budget; the outer distortion loop raises the scalefactor of the worst
/// over-threshold band, re-runs the inner loop, and keeps the lowest-peak-NMR
/// result. With a flat threshold (C1) it degrades to pure rate control; with the
/// real psymodel (Q1–Q4) it shapes quantization noise under the masking curve.
///
/// # Panics
/// Never: the `expect` reads the best result of a search loop that always
/// runs at least one iteration.
pub fn loops(
    header: &FrameHeader,
    freq: &[f32; GRANULE_LINES],
    psy: &PsyResult,
    bit_budget: usize,
    block_type: BlockType,
) -> QuantizedGranule {
    let mut sf = [0u8; 22];
    let mut best: Option<(f32, QuantizedGranule)> = None;
    let mut best_iter = 0usize;
    let xrp = xrpow(freq);

    // Put the thresholds in the SAME domain as the noise before comparing them --
    // the correction `loops_vbr` has carried since 0.6.0, which this path never
    // got. `psy.thresholds` are unnormalized 1024-point FFT power; `band_noise`
    // is squared error on MDCT coefficients. Measured on real music the two
    // scales differ by 10^4.90 (~79,000x, 49 dB) and, being a units mismatch
    // rather than a property of the signal, that ratio reads the same on every
    // clip. Uncorrected, `n > psy.thresholds[b]` was false for EVERY band of
    // EVERY granule: 98.7% of bands scored more than six decades under their
    // mask, the outer loop broke on its first iteration in 100% of granules, and
    // we shipped one global gain per granule where LAME shapes 71-77% of them.
    let mdct_energy: f32 = freq.iter().map(|x| x * x).sum();
    let domain_scale = if domain_correction() && psy.signal_energy > 1e-20 && mdct_energy > 1e-20 {
        mdct_energy / psy.signal_energy
    } else {
        1.0
    };

    if shape_slack() {
        return loops_slack(
            header,
            freq,
            psy,
            bit_budget,
            block_type,
            domain_scale,
            &xrp,
        );
    }

    for outer in 0..MAX_OUTER {
        super::prof::OUTER_ITERS.fetch_add(1, std::sync::atomic::Ordering::Relaxed);
        let (compress, sf_bits) = choose_compress(&sf);
        let huff_budget = bit_budget.saturating_sub(sf_bits);

        let mut coeffs = [0i32; GRANULE_LINES];
        let (gain, inner) = inner_gain(
            header,
            freq,
            &xrp,
            &sf,
            huff_budget,
            block_type,
            &mut coeffs,
        );
        // Reuse the quantization + Huffman selection the rate loop already did at
        // the winning gain (perf004); recompute only in the rare uncached case.
        let mut side = if let Some(side) = inner {
            side
        } else {
            quantize_into(header, freq, &xrp, gain, &sf, &mut coeffs);
            huff_cost(header, &coeffs, block_type).0
        };
        side.global_gain = gain as u8;
        side.scalefac_compress = compress;
        let mut scalefactors = [0u8; 39];
        scalefactors[..22].copy_from_slice(&sf);
        let granule = QuantizedGranule {
            coeffs,
            side,
            scalefactors,
        };

        // Score: peak noise-to-mask ratio across the coded bands. (Floor 3 NOTE: the
        // calibration says ODG tracks TOTAL `% audible`, not peak — but changing this
        // to a total-audible objective made ZERO difference, because the loop kept
        // iteration 0 in 100% of granules: at a hard per-granule CBR budget, amplifying
        // any band forces the rate loop to coarsen everything, so no shaping step ever
        // beats iter 0. Effective bit allocation needs a reservoir-aware RD redesign,
        // not a scoring tweak — see the OUTER_KEPT0 diagnostic.)
        if outer == 0 {
            super::prof::note_domain(psy.signal_energy, mdct_energy);
        }
        let noise = band_noise(header, freq, &granule.coeffs, gain, &sf);
        let mut peak_nmr = f32::NEG_INFINITY;
        let mut worst: Option<usize> = None;
        let mut worst_nmr = f32::NEG_INFINITY;
        for (b, &n) in noise.iter().enumerate() {
            let thr = (psy.thresholds[b] * domain_scale).max(1e-20);
            let nmr = n / thr;
            if outer == 0 {
                super::prof::note_nmr(nmr);
            }
            peak_nmr = peak_nmr.max(nmr);
            if n > thr && sf[b] < max_sf(b) && nmr > worst_nmr && shaping_allowed(header) {
                worst_nmr = nmr;
                worst = Some(b);
            }
        }
        if best.as_ref().is_none_or(|(bn, _)| peak_nmr < *bn) {
            best = Some((peak_nmr, granule));
            best_iter = outer;
        }
        match worst {
            Some(b) => sf[b] += 1, // amplify the worst band, then re-quantize
            None => break,         // every band already masked, or all saturated
        }
    }

    super::prof::OUTER_TOTAL.fetch_add(1, std::sync::atomic::Ordering::Relaxed);
    if best_iter == 0 {
        super::prof::OUTER_KEPT0.fetch_add(1, std::sync::atomic::Ordering::Relaxed);
    }
    best.expect("at least one iteration runs").1
}

/// Whether this stream's scalefactors can carry per-band shaping at all.
///
/// `bitstream` serializes them with [`crate::tables::SCALEFAC_COMPRESS_V1`]
/// unconditionally, but MPEG-2/2.5 (LSF) read `scalefac_compress` as a different,
/// 9-bit four-group field — so a non-zero scalefactor written under the V1 layout
/// is decoded as something else entirely. V2/V2.5 have always emitted flat
/// scalefactors, which is why that never mattered; shaping only became reachable
/// when the distortion loop started firing, and it took MPEG-2 round-trip SNR to
/// 5.7 dB. Until the LSF scalefactor scheme is implemented, shape MPEG-1 only.
#[inline]
fn shaping_allowed(header: &FrameHeader) -> bool {
    header.version == crate::header::MpegVersion::V1
}

/// Cap on refinement steps. Each one costs a quantize + Huffman table selection,
/// and the budget test stops it long before this on real content (LAME's own
/// streams shape 2.5-3.8 bands by ~2.5 scalefactor units).
const MAX_REFINE: usize = 24;

/// **Shape the granule by spending the bits the rate loop could not.**
///
/// The classic outer loop amplifies a band and then re-runs the rate loop, which
/// raises `global_gain` — so one band gets 3 dB finer and the other twenty get
/// 1.5 dB coarser. That is a minimax trade under a hard budget, it almost never
/// improves the peak, and measured on the corpus it is worth about −0.01 ODG.
///
/// This does the opposite: it holds `global_gain` FIXED at what the rate loop
/// chose and amplifies bands only while the result still fits the budget. No
/// band ever gets coarser — each band's coefficients depend only on the gain and
/// its own scalefactor — so every accepted step is a strict improvement rather
/// than a trade, and it is paid for out of bits that were otherwise emitted as
/// stuffing.
///
/// The slack is real and was measured from the bitstream: at 192 kbps we left
/// **63-89 bits per granule** unspent where LAME left 5-7. A global gain is a
/// 1.5 dB knob and cannot land on a bit budget exactly; per-band scalefactors are
/// precisely the finer knob MPEG provides to spend the remainder, and we were
/// using them in 0.0% of granules.
#[allow(clippy::too_many_arguments)]
fn loops_slack(
    header: &FrameHeader,
    freq: &[f32; GRANULE_LINES],
    psy: &PsyResult,
    bit_budget: usize,
    block_type: BlockType,
    domain_scale: f32,
    xrp: &[f64; GRANULE_LINES],
) -> QuantizedGranule {
    let mut sf = [0u8; 22];

    // Rate loop once, flat -- this is the gain every step below keeps.
    let (mut compress, sf_bits) = choose_compress(&sf);
    let mut coeffs = [0i32; GRANULE_LINES];
    let (gain, inner) = inner_gain(
        header,
        freq,
        xrp,
        &sf,
        bit_budget.saturating_sub(sf_bits),
        block_type,
        &mut coeffs,
    );
    let mut side = if let Some(side) = inner {
        side
    } else {
        quantize_into(header, freq, xrp, gain, &sf, &mut coeffs);
        huff_cost(header, &coeffs, block_type).0
    };

    // Per-band noise, kept incrementally: a refinement step changes exactly one
    // band, so recomputing all 21 every iteration is the classic once-per-candidate
    // redundancy.
    let mut noise = band_noise(header, freq, &coeffs, gain, &sf);
    let off = crate::tables::sfb_long_offsets(header.sample_rate);

    for _ in 0..MAX_REFINE {
        if !shaping_allowed(header) {
            break; // LSF: scalefactors must stay flat, see `shaping_allowed`
        }
        // Worst-masked band that can still be amplified.
        //
        // `audible_only` decides whether a band must actually be OVER its
        // threshold to be worth refining. Spending the slack indiscriminately is
        // free in bits -- they would be stuffing otherwise -- but it is not free
        // perceptually: at 192 kbps this corpus is already transparent, so the
        // refinement is then just moving noise between inaudible bands on the
        // strength of a threshold ranking that has no headroom left to be right
        // about.
        let audible_only = shape_audible_only();
        let pick = pick_rule();
        let mut worst = None;
        let mut worst_score = f32::NEG_INFINITY;
        for (b, &n) in noise.iter().enumerate() {
            let thr = (psy.thresholds[b] * domain_scale).max(1e-20);
            // Candidate ranking signals for "which band gets the next bit". The
            // psymodel-driven one is the default; the others exist to answer
            // whether the model contributes anything to the decision at all -- if
            // ranking by raw noise ties ranking by noise-to-mask, the masking
            // curve is not informing the allocation.
            let score = match pick {
                Pick::Nmr => n / thr,
                Pick::Noise => n,
                Pick::Widest => n / (band_width(header, b) as f32),
                Pick::LowFirst => -(b as f32),
            };
            if sf[b] < max_sf(b) && score > worst_score && (!audible_only || n > thr) {
                worst_score = score;
                worst = Some(b);
            }
        }
        // Census: would the ranking have preferred a band that is already capped?
        {
            let mut any_best = None;
            let mut any_score = f32::NEG_INFINITY;
            for (bb, &n) in noise.iter().enumerate() {
                let thr = (psy.thresholds[bb] * domain_scale).max(1e-20);
                let sc = n / thr;
                if sc > any_score && (!audible_only || n > thr) {
                    any_score = sc;
                    any_best = Some(bb);
                }
            }
            if let Some(bb) = any_best {
                if sf[bb] >= max_sf(bb) {
                    use std::sync::atomic::Ordering::Relaxed;
                    if bb < 11 {
                        super::prof::CAP_BLOCK_LO.fetch_add(1, Relaxed);
                    } else {
                        super::prof::CAP_BLOCK_HI.fetch_add(1, Relaxed);
                    }
                }
            }
        }
        let Some(b) = worst else { break };

        let mut trial = sf;
        trial[b] += 1;
        let (t_compress, t_sf_bits) = choose_compress(&trial);
        // Only band `b` moves; `coeffs` holds the accepted state for the rest.
        quantize_band_in_place(header, freq, xrp, gain, trial[b], b, &mut coeffs);
        let (lo, hi) = (off[b] as usize, (off[b + 1] as usize).min(GRANULE_LINES));
        let clipped = coeffs[lo..hi].iter().any(|&c| c.abs() > MAX_UNCLIPPED);
        let (t_side, t_bits) = if clipped {
            (side.clone(), usize::MAX)
        } else {
            huff_cost(header, &coeffs, block_type)
        };
        // A finer step means larger integers. Either saturation or running out of
        // budget ends the refinement -- put band `b` back the way it was and stop.
        if clipped || t_bits + t_sf_bits > bit_budget {
            quantize_band_in_place(header, freq, xrp, gain, sf[b], b, &mut coeffs);
            break;
        }

        noise[b] = band_noise_one(header, freq, &coeffs, gain, trial[b], b);
        sf = trial;
        side = t_side;
        compress = t_compress;
        super::prof::REFINE_STEPS.fetch_add(1, std::sync::atomic::Ordering::Relaxed);
    }

    side.global_gain = gain as u8;
    side.scalefac_compress = compress;
    let mut scalefactors = [0u8; 39];
    scalefactors[..22].copy_from_slice(&sf);

    let relaxed = std::sync::atomic::Ordering::Relaxed;
    super::prof::OUTER_TOTAL.fetch_add(1, relaxed);
    if sf.iter().all(|&v| v == 0) {
        super::prof::OUTER_KEPT0.fetch_add(1, relaxed);
    } else if (11..21).all(|b| sf[b] >= crate::tables::PRETAB[b]) {
        super::prof::PREFLAG_OK.fetch_add(1, relaxed);
    }

    QuantizedGranule {
        coeffs,
        side,
        scalefactors,
    }
}

/// Lines in long-block scalefactor band `b`.
fn band_width(header: &FrameHeader, b: usize) -> usize {
    let off = crate::tables::sfb_long_offsets(header.sample_rate);
    (off[b + 1] as usize).min(GRANULE_LINES) - off[b] as usize
}

/// Candidate band-ranking signals for the refinement (great-gate P1 signal audit).
#[derive(Clone, Copy, PartialEq)]
enum Pick {
    /// Noise-to-mask ratio -- the psychoacoustic ranking (default).
    Nmr,
    /// Raw quantization-noise energy; ignores the masking model entirely.
    Noise,
    /// Noise per line, so wide high bands do not win on width alone.
    Widest,
    /// Lowest frequency first -- a model-free ordering, the null hypothesis.
    LowFirst,
}

/// Vendored (studio): the `MP3_PICK` override is removed; the default.
fn pick_rule() -> Pick {
    Pick::Nmr
}

/// Whether refinement only targets bands whose noise is actually above the
/// masking threshold (`MP3_SHAPE=audible`), rather than the worst-ranked band
/// regardless. Read once.
///
/// Vendored (studio): the `MP3_SHAPE` override is removed; the default.
fn shape_audible_only() -> bool {
    false
}

/// Which shaping strategy the CBR path uses. `slack` (the default) refines bands
/// inside the rate loop's leftover bits at a fixed gain; `MP3_SHAPE=outer`
/// restores the classic amplify-and-re-rate loop.
///
/// Vendored (studio): the `MP3_SHAPE` override is removed; the default.
fn shape_slack() -> bool {
    true
}

/// Whether the CBR distortion loop rescales the psychoacoustic thresholds into
/// the noise's domain before comparing. On (the default) is correct; `MP3_PSY_DOMAIN=0`
/// restores the pre-fix behaviour, where the comparison was off by ~10^4.9 and the
/// loop therefore never shaped a band. Kept as an A/B toggle, read once.
///
/// Vendored (studio): the `MP3_PSY_DOMAIN` override is removed; the default.
fn domain_correction() -> bool {
    true
}

/// Smallest gain whose flat-scalefactor quantization doesn't clip — the finest
/// representable step for this spectrum.
fn nonclip_floor(
    header: &FrameHeader,
    freq: &[f32; GRANULE_LINES],
    xrp: &[f64; GRANULE_LINES],
) -> i32 {
    let flat = [0u8; 22];
    let ok = |g: i32| {
        quantize_with_sf(header, freq, xrp, g, &flat)
            .iter()
            .all(|&c| c.abs() <= MAX_UNCLIPPED)
    };
    let (mut lo, mut hi) = (0i32, 255i32);
    while lo < hi {
        let mid = (lo + hi) / 2;
        if ok(mid) {
            hi = mid;
        } else {
            lo = mid + 1;
        }
    }
    lo
}

/// **R2 (VBR)** — quantize to a *quality* target instead of a bit budget. Picks the
/// coarsest gain whose peak noise-to-mask ratio stays under `target_nmr` (fewest
/// bits that still meet quality), never finer than the no-clip floor, then shapes
/// scalefactors under the threshold. The resulting bit count — and hence the
/// frame's bitrate — varies with content.
pub fn loops_vbr(
    header: &FrameHeader,
    freq: &[f32; GRANULE_LINES],
    psy: &PsyResult,
    target_nmr: f32,
    block_type: BlockType,
) -> QuantizedGranule {
    let flat = [0u8; 22];
    let xrp = xrpow(freq);
    // Put the thresholds in the SAME domain as the noise before comparing them.
    //
    // `psy.thresholds` are FFT power-spectrum energies (windowed, 1024-point);
    // `band_noise` measures in the MDCT domain (576 lines, different
    // normalisation). On real content the two scales differ by ~10^4, which made
    // every absolute test pass: `peak(255)` — the COARSEST possible quantization,
    // where nearly everything rounds to zero — read 2.5e-4 against a target of
    // 1.0, so the search saturated at gain 255 for 97.5% of granules and VBR
    // emitted ~39 kbps at every `-q:a`. Rescaling by the ratio of total energies
    // is self-calibrating: no magic constant, and it tracks the content.
    let mdct_energy: f32 = freq.iter().map(|x| x * x).sum();
    let domain_scale = if psy.signal_energy > 1e-20 && mdct_energy > 1e-20 {
        mdct_energy / psy.signal_energy
    } else {
        1.0
    };
    let peak = |g: i32| {
        let coeffs = quantize_with_sf(header, freq, &xrp, g, &flat);
        let noise = band_noise(header, freq, &coeffs, g, &flat);
        noise
            .iter()
            .enumerate()
            .map(|(b, &n)| n / (psy.thresholds[b] * domain_scale).max(1e-20))
            .fold(0f32, f32::max)
    };
    // Largest gain (coarsest → fewest bits) whose peak NMR meets the target.
    let (mut lo, mut hi) = (0i32, 255i32);
    while lo < hi {
        let mid = (lo + hi + 1) / 2;
        if peak(mid) <= target_nmr {
            lo = mid;
        } else {
            hi = mid - 1;
        }
    }
    let floor = nonclip_floor(header, freq, &xrp);
    let gain = lo.max(floor);
    // TEMPORARY VBR DIAGNOSTIC (removed after the fix is characterised).
    {
        use std::sync::atomic::{AtomicU64, Ordering::Relaxed};
        pub static N: AtomicU64 = AtomicU64::new(0);
        pub static SUM_LO: AtomicU64 = AtomicU64::new(0);
        pub static SUM_FLOOR: AtomicU64 = AtomicU64::new(0);
        pub static CLAMPED: AtomicU64 = AtomicU64::new(0);
        pub static SATURATED: AtomicU64 = AtomicU64::new(0);
        N.fetch_add(1, Relaxed);
        SUM_LO.fetch_add(lo.max(0) as u64, Relaxed);
        SUM_FLOOR.fetch_add(floor.max(0) as u64, Relaxed);
        if floor > lo {
            CLAMPED.fetch_add(1, Relaxed);
        }
        if lo >= 255 {
            SATURATED.fetch_add(1, Relaxed);
        }
        let n = N.load(Relaxed);
        if n % 2000 == 0 {
            // What does the NMR actually read at the extremes?
            let p_floor = peak(floor);
            let p_max = peak(255);
            let thr_min = psy.thresholds.iter().copied().fold(f32::MAX, f32::min);
            let thr_max = psy.thresholds.iter().copied().fold(0f32, f32::max);
            let sig: f32 = freq.iter().map(|x| x * x).sum();
            eprintln!(
                "[nmr] peak(floor={floor})={p_floor:.6e} peak(255)={p_max:.6e} thr[min={thr_min:.3e} max={thr_max:.3e}] sig_energy={sig:.3e}"
            );
            eprintln!(
                "[vbr] n={n} target_nmr={target_nmr:.4} mean_lo={:.1} mean_floor={:.1} clamped={:.1}% saturated={:.1}%",
                SUM_LO.load(Relaxed) as f64 / n as f64,
                SUM_FLOOR.load(Relaxed) as f64 / n as f64,
                100.0 * CLAMPED.load(Relaxed) as f64 / n as f64,
                100.0 * SATURATED.load(Relaxed) as f64 / n as f64,
            );
        }
    }

    // Distortion loop at the fixed gain: raise the worst over-threshold band.
    let mut sf = [0u8; 22];
    for _ in 0..MAX_OUTER {
        let coeffs = quantize_with_sf(header, freq, &xrp, gain, &sf);
        let noise = band_noise(header, freq, &coeffs, gain, &sf);
        let mut worst = None;
        let mut worst_nmr = f32::NEG_INFINITY;
        for (b, &n) in noise.iter().enumerate() {
            // Same domain correction as the gain search above — an absolute
            // "is this band over threshold?" test needs the scaled threshold.
            let thr = (psy.thresholds[b] * domain_scale).max(1e-20);
            let nmr = n / thr;
            if n > thr && nmr > worst_nmr && sf[b] >= max_sf(b) && sf[b] < MAX_SF {
                // The pre-fix guard would have amplified here and the serializer
                // would have truncated the value.
                super::prof::SF_OVERFLOW.fetch_add(1, std::sync::atomic::Ordering::Relaxed);
            }
            if n > thr && sf[b] < max_sf(b) && nmr > worst_nmr {
                worst_nmr = nmr;
                worst = Some(b);
            }
        }
        match worst {
            Some(b) => sf[b] += 1,
            None => break,
        }
    }

    let coeffs = quantize_with_sf(header, freq, &xrp, gain, &sf);
    let (compress, _) = choose_compress(&sf);
    let (mut side, _) = super::huffman::select(header, &coeffs, block_type);
    side.global_gain = gain as u8;
    side.scalefac_compress = compress;
    let mut scalefactors = [0u8; 39];
    scalefactors[..22].copy_from_slice(&sf);
    QuantizedGranule {
        coeffs,
        side,
        scalefactors,
    }
}

#[cfg(test)]
mod c2_tests {
    use super::*;
    use crate::decode::scalefactors::ScaleFactors;
    use crate::frame::ChannelMode;
    use crate::header::MpegVersion;

    fn hdr() -> FrameHeader {
        FrameHeader {
            version: MpegVersion::V1,
            crc_protected: false,
            bitrate_kbps: 128,
            sample_rate: 44100,
            padding: false,
            channel_mode: ChannelMode::Mono,
            copyright: false,
            original: true,
            emphasis: 0,
        }
    }

    /// A1 invariant: quantizing via the precomputed `|freq|^(3/4)` table must
    /// reproduce the per-line `powf` reference. The two differ only by last-ULP
    /// float rounding, so we allow an off-by-one at a quantization boundary but
    /// require it to be vanishingly rare (the byte-identical-output gate is the
    /// real proof; this catches any systematic error).
    #[test]
    fn xrpow_path_matches_powf_reference() {
        let header = hdr();
        let mut s = 0xABCD_1234u32;
        let mut total = 0usize;
        let mut diffs = 0usize;
        for trial in 0..40 {
            let mut freq = [0f32; GRANULE_LINES];
            for f in &mut freq {
                s = s.wrapping_mul(1_664_525).wrapping_add(1_013_904_223);
                // magnitudes spanning the quantizer's working range
                *f = ((s >> 8) as f32 / (1u32 << 24) as f32 - 0.5) * 2.0 * 1000.0;
            }
            let xrp = xrpow(&freq);
            let off = crate::tables::sfb_long_offsets(header.sample_rate);
            for &gain in &[120i32, 180, 210, 230] {
                let mut sf = [0u8; 22];
                for (b, sfb) in sf.iter_mut().enumerate() {
                    *sfb = ((trial + b) % 4) as u8;
                }
                let got = quantize_with_sf(&header, &freq, &xrp, gain, &sf);
                // Reference: the original per-line powf path.
                let base = -0.25 * f64::from(gain - 210);
                for b in 0..22 {
                    let sv = f64::from(if b < 21 { sf[b] } else { 0 });
                    let scale_inv = 2f64.powf(base + SF_MULT * sv);
                    let (lo, hi) = (off[b] as usize, (off[b + 1] as usize).min(GRANULE_LINES));
                    for i in lo..hi {
                        let mag = quantize_level(f64::from(freq[i].abs()) * scale_inv);
                        let want = if freq[i] < 0.0 { -mag } else { mag };
                        total += 1;
                        if got[i] != want {
                            assert!(
                                (got[i] - want).abs() <= 1,
                                "line {i}: table {} vs powf {want} (gain {gain})",
                                got[i]
                            );
                            diffs += 1;
                        }
                    }
                }
            }
        }
        // Expect near-perfect agreement: well under 0.1% of lines may ULP-flip.
        assert!(
            diffs * 1000 < total,
            "too many xrpow/powf mismatches: {diffs}/{total}"
        );
        eprintln!("[A1] xrpow vs powf: {diffs}/{total} off-by-one (ULP) lines");
    }

    #[test]
    fn spectrum_roundtrip_through_quantizer() {
        // A small synthetic spectrum: quantize then requantize should recover it.
        let header = hdr();
        let mut freq = [0f32; GRANULE_LINES];
        freq[40] = 5.0;
        freq[41] = -3.0;
        freq[100] = 1.2;
        freq[200] = 0.6;

        let psy = PsyResult::default();
        let q = loops(&header, &freq, &psy, 100_000, BlockType::Long); // generous → fine gain
        eprintln!(
            "[C2dbg] gain={} part2_3={} nz_coeffs={}",
            q.side.global_gain,
            q.side.part2_3_length,
            q.coeffs.iter().filter(|&&c| c != 0).count()
        );

        // Requantize the way the decoder does, with the granule's scalefactors.
        let mut sf = ScaleFactors::default();
        sf.long.copy_from_slice(&q.scalefactors[..22]);
        let mut out = [0f32; GRANULE_LINES];
        let nz = q.coeffs.iter().rposition(|&c| c != 0).map_or(0, |i| i + 1);
        crate::decode::requantize::apply(&header, &q.side, &sf, &q.coeffs, nz, &mut out);

        let mut maxerr = 0f32;
        for i in 0..GRANULE_LINES {
            maxerr = maxerr.max((out[i] - freq[i]).abs());
        }
        eprintln!(
            "[C2dbg] requant maxerr={maxerr} out[40]={} out[41]={}",
            out[40], out[41]
        );
        assert!(maxerr < 0.2, "spectrum round-trip error {maxerr}");
    }
}

#[cfg(test)]
mod q6_tests {
    use super::*;
    use crate::frame::{BlockType, ChannelMode, GranuleSideInfo};
    use crate::header::MpegVersion;

    fn hdr() -> FrameHeader {
        FrameHeader {
            version: MpegVersion::V1,
            crc_protected: false,
            bitrate_kbps: 128,
            sample_rate: 44100,
            padding: false,
            channel_mode: ChannelMode::Mono,
            copyright: false,
            original: true,
            emphasis: 0,
        }
    }

    /// Peak noise-to-mask ratio (dB) of a granule under `thresholds`.
    fn peak_nmr_db(
        header: &FrameHeader,
        freq: &[f32; GRANULE_LINES],
        g: &QuantizedGranule,
        thresholds: &[f32; 22],
    ) -> f32 {
        let mut sf = [0u8; 22];
        sf.copy_from_slice(&g.scalefactors[..22]);
        let noise = band_noise(header, freq, &g.coeffs, i32::from(g.side.global_gain), &sf);
        let mut peak = f32::NEG_INFINITY;
        for (b, &n) in noise.iter().enumerate() {
            peak = peak.max(10.0 * (n / thresholds[b].max(1e-20)).log10());
        }
        peak
    }

    #[test]
    fn distortion_loop_beats_flat_on_a_complex_signal() {
        // Two tones in different critical bands → low-masking bands sit next to
        // high-masking ones, so shaping noise to the threshold helps.
        let header = hdr();
        let sr = 44100.0;
        let pcm: Vec<f32> = (0..1152)
            .map(|i| {
                let t = i as f32 / sr;
                0.35 * (2.0 * std::f32::consts::PI * 600.0 * t).sin()
                    + 0.2 * (2.0 * std::f32::consts::PI * 5200.0 * t).sin()
            })
            .collect();

        let psy = super::super::psychoacoustic::analyze(&pcm, 44100);

        // Forward path to the MDCT spectrum.
        let mut fifo = [0f32; 512];
        let sub = super::super::filterbank::analyze(&pcm, &mut fifo);
        let mut overlap = [0f32; GRANULE_LINES];
        let mut freq = super::super::mdct::forward(&sub, BlockType::Long, &mut overlap);
        super::super::antialias::expand(&GranuleSideInfo::default(), &mut freq);

        let budget = 1600;
        let flat = PsyResult {
            thresholds: [f32::MAX; 22], // never over threshold → no shaping (pure rate)
            ..psy.clone()
        };
        let shaped = loops(&header, &freq, &psy, budget, BlockType::Long);
        let plain = loops(&header, &freq, &flat, budget, BlockType::Long);

        let nmr_shaped = peak_nmr_db(&header, &freq, &shaped, &psy.thresholds);
        let nmr_plain = peak_nmr_db(&header, &freq, &plain, &psy.thresholds);
        eprintln!("[Q6] peak NMR: shaped {nmr_shaped:.1} dB vs flat {nmr_plain:.1} dB");
        assert!(
            nmr_shaped <= nmr_plain + 0.01,
            "psymodel shaping must not worsen peak NMR: {nmr_shaped} vs {nmr_plain}"
        );
    }
}

#[cfg(test)]
mod n4_tests {
    use super::*;

    #[test]
    fn requant_magnitude_matches_power_law() {
        for level in [0, 1, 2, 3, 17, 255, 1024, MAX_LEVEL] {
            let expect = f64::from(level).powf(4.0 / 3.0);
            assert!((requant_magnitude(level) - expect).abs() < 1e-9);
        }
        // Sign is carried separately, so the magnitude ignores it.
        assert_eq!(requant_magnitude(-3), requant_magnitude(3));
    }

    #[test]
    fn forward_inverse_round_trip_on_the_lattice() {
        // The verification gate: every representable level survives
        // requantize → quantize unchanged. If the BIAS or the power law were
        // wrong, some level would round to a neighbour.
        for level in 0..=MAX_LEVEL {
            let xr = requant_magnitude(level);
            assert_eq!(
                quantize_level(xr),
                level,
                "round-trip failed at level {level} (xr={xr})"
            );
        }
    }

    #[test]
    fn quantizer_clamps_and_zeroes() {
        assert_eq!(quantize_level(0.0), 0);
        // A value just below 1^(4/3) still rounds to 0 (below the first lattice
        // point, the bias pulls it under 0.5).
        assert_eq!(quantize_level(0.3), 0);
        // Saturates at MAX_LEVEL rather than overflowing.
        assert_eq!(quantize_level(1.0e9), MAX_LEVEL);
    }

    /// C gate: the branchless `level_from` must equal the original guarded form
    /// for every input, including the boundaries (≤0, the rounding seam, and the
    /// saturation knee) — a wide dense sweep plus the exact lattice midpoints.
    /// The SIMD quantizer must be BIT-identical to the scalar oracle -- including
    /// at the rounding seams (exact half-integer + bias), past saturation, at zero
    /// and below the bias, and for `-0.0` frequencies (which keep a positive sign).
    #[test]
    fn quantize_lines_simd_matches_scalar() {
        if !crate::decode::isa::simd_available() {
            eprintln!("no SIMD twin on this host - scalar path only, gate skipped");
            return;
        }
        let mut st = 0x6C07_8965u32;
        let mut r = move || {
            st = st.wrapping_mul(1_664_525).wrapping_add(1_013_904_223);
            st
        };
        for &rate in &[44100u32, 32000, 22050, 8000] {
            let off = crate::tables::sfb_long_offsets(rate);
            for trial in 0..64 {
                let mut freq = [0f32; GRANULE_LINES];
                let mut xrp = [0f64; GRANULE_LINES];
                for i in 0..GRANULE_LINES {
                    let k = r();
                    freq[i] = match k % 7 {
                        0 => -0.0,
                        1 => 0.0,
                        _ => ((k >> 3) as f32 / 2.0e8) - 10.0,
                    };
                    xrp[i] = match (k >> 8) % 5 {
                        0 => f64::from(k % 9000) + 0.5 + QUANT_BIAS, // a rounding seam
                        1 => 1.0e9,                                  // past saturation
                        2 => 0.0,
                        _ => f64::from(k >> 4) / 1.0e5,
                    };
                }
                let mut steps = [0f64; 22];
                for s in &mut steps {
                    *s = [1.0, 0.5, 2.0, 0.123_456_7][(r() % 4) as usize];
                }
                let (mut a, mut b) = ([0i32; GRANULE_LINES], [0i32; GRANULE_LINES]);
                quantize_lines_scalar(off, &steps, &freq, &xrp, &mut a);
                // SAFETY: gated on simd_available(); off spans 0..576.
                unsafe { quantize_lines_simd(off, &steps, &freq, &xrp, &mut b) };
                assert_eq!(
                    a, b,
                    "quantizer SIMD/scalar mismatch at rate {rate} trial {trial}"
                );
            }
        }
    }

    #[test]
    fn level_from_matches_guarded() {
        // The reference: the pre-optimisation guarded implementation.
        let guarded = |powered: f64| -> i32 {
            let m = powered - QUANT_BIAS;
            if m <= 0.0 {
                0
            } else {
                (m.round() as i32).clamp(0, MAX_LEVEL)
            }
        };
        // Dense sweep across the working range and a bit past saturation.
        let mut p = -1.0f64;
        // A float-stepped sweep is the point here (it lands near many rounding seams).
        #[allow(clippy::while_float)]
        while p < 9000.0 {
            assert_eq!(level_from(p), guarded(p), "level_from mismatch at {p}");
            p += 0.013; // irrational-ish step to land near many rounding seams
        }
        // Exact half-integer + bias midpoints (the rounding boundary itself).
        for n in 0..50 {
            let mid = f64::from(n) + 0.5 + QUANT_BIAS;
            assert_eq!(level_from(mid), guarded(mid), "midpoint {mid}");
        }
        // Extremes.
        for &p in &[f64::from(0), 1e9, f64::from(MAX_LEVEL) + 5.0] {
            assert_eq!(level_from(p), guarded(p));
        }
    }
}

#[cfg(test)]
mod sf_range_tests {
    use super::*;

    /// Every scalefactor the loops can produce must be representable.
    ///
    /// `scalefac_compress` gives bands 11..20 only `slen2` bits, and the MPEG-1
    /// table stops at `slen2 = 3` — so 7 is the ceiling there, not `MAX_SF`. The
    /// old guard used `MAX_SF` for every band; `choose_compress` then found no
    /// covering entry and the serializer truncated the value, which the decoder
    /// read as a scalefactor up to 16x too small (one granule of near-full-scale
    /// garbage). This pins the ceiling per band group.
    #[test]
    fn max_sf_is_representable_for_every_band() {
        for b in 0..21 {
            let mut sf = [0u8; 22];
            sf[b] = max_sf(b);
            let (idx, _bits) = choose_compress(&sf);
            let (slen1, slen2) = crate::tables::SCALEFAC_COMPRESS_V1[idx as usize];
            let slen = if b < 11 { slen1 } else { slen2 };
            let representable = if slen == 0 { 0 } else { (1u16 << slen) - 1 };
            assert!(
                u16::from(sf[b]) <= representable,
                "band {b}: sf {} needs more than slen {slen} ({representable} max)",
                sf[b]
            );
        }
    }

    /// The high group really is the tighter one — a regression guard on the table
    /// itself, so a future table edit cannot quietly widen the ceiling.
    #[test]
    fn high_band_ceiling_is_seven() {
        assert_eq!(max_sf(0), 15);
        assert_eq!(max_sf(10), 15);
        assert_eq!(max_sf(11), 7);
        assert_eq!(max_sf(20), 7);
        let max_slen2 = crate::tables::SCALEFAC_COMPRESS_V1
            .iter()
            .map(|&(_, s2)| s2)
            .max()
            .unwrap();
        assert_eq!(max_slen2, 3, "slen2 ceiling moved; revisit max_sf()");
    }
}
