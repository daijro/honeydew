// Modified from rusty_mp3 1.0.0 (see Cargo.toml).
//! Short-block coding (brick **Q5**) — the forward reorder and the short-block
//! quantizer.
//!
//! Short blocks give the filterbank finer time resolution, which suppresses the
//! pre-echo a long block smears before a transient. The forward MDCT (`mdct.rs`)
//! already produces the three short windows interleaved in *subband* order; the
//! bitstream stores them in *scalefactor-band* order (sfb-major, then window, then
//! frequency). [`reorder_subband_to_bitstream`] is the exact inverse of the
//! decoder's requantize reorder, so a coded short block round-trips.
//!
//! This first version quantises uniformly (flat per-window scalefactors,
//! `subblock_gain = 0`) under global-gain rate control — valid short blocks that
//! fix pre-echo. Per-window perceptual shaping is a later refinement.

use crate::frame::{BlockType, GRANULE_LINES};
use crate::header::FrameHeader;
use crate::tables;

use super::quantize::QuantizedGranule;

const MAX_UNCLIPPED: i32 = 8191;

/// Reorder a short-block spectrum from the MDCT's subband-interleaved order into
/// the bitstream's `(sfb, window, freq)` order — the forward of the decoder's
/// requantize reorder (`out[dst] = coeffs[src]`).
#[must_use]
pub fn reorder_subband_to_bitstream(
    sample_rate: u32,
    subband: &[f32; GRANULE_LINES],
) -> [f32; GRANULE_LINES] {
    let off = tables::sfb_short_offsets(sample_rate);
    let mut out = [0f32; GRANULE_LINES];
    for sfb in 0..13 {
        let start = off[sfb] as usize;
        let width = (off[sfb + 1] - off[sfb]) as usize;
        for window in 0..3 {
            for f in 0..width {
                let src = start * 3 + window * width + f;
                let dst = start * 3 + window + f * 3;
                if src < GRANULE_LINES && dst < GRANULE_LINES {
                    out[src] = subband[dst];
                }
            }
        }
    }
    out
}

/// Uniformly quantize bitstream-order short-block lines at `global_gain`
/// (`subblock_gain = 0`, flat scalefactors), the forward of the decoder's short
/// requantization with those fields zero. Uses the precomputed `|freq|^(3/4)`
/// (A1): each gain probe is a multiply-and-round, no per-line `powf`.
fn quantize_uniform(
    freq: &[f32; GRANULE_LINES],
    xrp: &[f64; GRANULE_LINES],
    gain: i32,
) -> [i32; GRANULE_LINES] {
    // step = scale_inv^(3/4), scale_inv = 2^(-0.25·(gain−210)).
    let step = 2f64.powf(0.75 * -0.25 * f64::from(gain - 210));
    let mut coeffs = [0i32; GRANULE_LINES];
    for (i, &x) in freq.iter().enumerate() {
        let mag = super::quantize::level_from(xrp[i] * step);
        coeffs[i] = if x < 0.0 { -mag } else { mag };
    }
    coeffs
}

/// **Q5 short quantizer** — quantize a short-block granule (already reordered to
/// bitstream order) to fit `bit_budget`, with flat per-window scalefactors. Picks
/// the smallest non-clipping `global_gain` whose Huffman cost fits.
#[must_use]
pub fn quantize_short(
    header: &FrameHeader,
    freq_bitstream: &[f32; GRANULE_LINES],
    bit_budget: usize,
) -> QuantizedGranule {
    let xrp = super::quantize::xrpow(freq_bitstream); // A1: hoist |freq|^(3/4)
    let ok = |g: i32| {
        let coeffs = quantize_uniform(freq_bitstream, &xrp, g);
        coeffs.iter().all(|&c| c.abs() <= MAX_UNCLIPPED)
            && super::huffman::cost_short(header, &coeffs) <= bit_budget
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
    let coeffs = quantize_uniform(freq_bitstream, &xrp, lo);
    let (mut side, _) = super::huffman::select(header, &coeffs, BlockType::Short);
    side.global_gain = lo as u8;
    side.scalefac_compress = 0; // flat scalefactors → zero scalefactor bits
    QuantizedGranule {
        coeffs,
        side,
        scalefactors: [0; 39],
    }
}

/// Largest short scalefactor for band `b` under the MPEG-1 table: `slen1 <= 4`
/// covers bands 0..=5, but `slen2 <= 3` covers 6..=11 (the same ceiling the long
/// path learned the hard way -- see `quantize::max_sf`).
#[inline]
fn max_sf_short(b: usize) -> u8 {
    if b < 6 {
        15
    } else {
        7
    }
}

/// Smallest `scalefac_compress` covering the short scalefactors, and its cost:
/// each slen is paid for 6 bands x 3 windows.
fn choose_compress_short(sf: &[[u8; 12]; 3]) -> (u16, usize) {
    let max_in = |r: std::ops::Range<usize>| {
        sf.iter()
            .flat_map(|w| w[r.clone()].iter().copied())
            .max()
            .unwrap_or(0)
    };
    let bits = |v: u8| {
        if v == 0 {
            0
        } else {
            8 - v.leading_zeros() as u8
        }
    };
    let (need1, need2) = (bits(max_in(0..6)), bits(max_in(6..12)));
    for (idx, &(s1, s2)) in tables::SCALEFAC_COMPRESS_V1.iter().enumerate() {
        if s1 >= need1 && s2 >= need2 {
            return (idx as u16, 18 * s1 as usize + 18 * s2 as usize);
        }
    }
    debug_assert!(
        false,
        "short scalefactors exceed the table; clamp with max_sf_short"
    );
    (15, 18 * 4 + 18 * 3)
}

/// Re-quantize one (band, window) run in bitstream order at `gain` with
/// scalefactor `s`: the forward of the decoder's short requantization
/// (`2^(0.25·gain − 0.5·sf)`, `subblock_gain = 0`).
#[allow(clippy::too_many_arguments)]
fn quantize_run(
    freq: &[f32; GRANULE_LINES],
    xrp: &[f64; GRANULE_LINES],
    gain: i32,
    s: u8,
    lo: usize,
    hi: usize,
    coeffs: &mut [i32; GRANULE_LINES],
) {
    let step = 2f64.powf(0.75 * (-0.25 * f64::from(gain - 210) + 0.5 * f64::from(s)));
    for i in lo..hi {
        let mag = super::quantize::level_from(xrp[i] * step);
        coeffs[i] = if freq[i] < 0.0 { -mag } else { mag };
    }
}

/// Noise energy of one (band, window) run at `gain` with scalefactor `s`.
fn run_noise(
    freq: &[f32; GRANULE_LINES],
    coeffs: &[i32; GRANULE_LINES],
    gain: i32,
    s: u8,
    lo: usize,
    hi: usize,
) -> f32 {
    let scale = 2f64.powf(0.25 * f64::from(gain - 210) - 0.5 * f64::from(s));
    let mut e = 0f64;
    for i in lo..hi {
        let xr =
            f64::from(coeffs[i].signum()) * super::quantize::requant_magnitude(coeffs[i]) * scale;
        let d = f64::from(freq[i]) - xr;
        e += d * d;
    }
    e as f32
}

/// Whether short-block shaping is on (`MP3_SHORT_SHAPE=0` restores the flat,
/// byte-identical pre-shaping encoder). Read once.
///
/// Vendored (studio): the `MP3_SHORT_SHAPE` override is removed; the default.
fn short_shape_enabled() -> bool {
    true
}

/// **Short-block noise shaping: spend the bits the rate loop could not.**
///
/// The short path quantized every (band, window) at one global gain with flat
/// scalefactors -- exactly the shape the LONG path had before G1, and for the
/// same reason it leaves slack: a gain step is ~1.5 dB and cannot land on a bit
/// budget. This holds that gain FIXED and raises the scalefactor of the
/// worst-masked (band, window) while the granule still fits, so no run ever gets
/// coarser and every accepted step is a strict improvement paid for from bits that
/// were otherwise stuffing.
///
/// Thresholds are the long-band ones mapped onto the short grid by MINIMUM (see
/// [`short_thresholds`]) -- the conservative proxy for the short-block psymodel
/// this encoder does not have yet. Only the RANKING uses them, so their absolute
/// level cancels (the level/shape law in `docs/plans/mp3-gate-ledger.md`).
/// MPEG-1 only, like long-block shaping: the LSF scalefactor writer does not exist.
pub fn quantize_short_slack(
    header: &FrameHeader,
    freq_bitstream: &[f32; GRANULE_LINES],
    bit_budget: usize,
    long_thresholds: &[f32; crate::frame::SFB_LONG],
    signal_energy: f32,
) -> QuantizedGranule {
    let mut q = quantize_short(header, freq_bitstream, bit_budget);
    if !short_shape_enabled() || header.version != crate::header::MpegVersion::V1 {
        return q;
    }
    let gain = i32::from(q.side.global_gain);
    let xrp = super::quantize::xrpow(freq_bitstream);
    let off = tables::sfb_short_offsets(header.sample_rate);
    let mdct_energy: f32 = freq_bitstream.iter().map(|x| x * x).sum();
    let domain = if signal_energy > 1e-20 && mdct_energy > 1e-20 {
        mdct_energy / signal_energy
    } else {
        1.0
    };
    let thr = short_thresholds(header.sample_rate, long_thresholds);
    // Bitstream order: band b, window w occupies [3·off[b] + w·width, +width).
    let run = |b: usize, w: usize| {
        let width = (off[b + 1] - off[b]) as usize;
        let lo = (3 * off[b] as usize + w * width).min(GRANULE_LINES);
        (lo, (lo + width).min(GRANULE_LINES))
    };
    let mut sf = [[0u8; 12]; 3];
    let mut noise = [[0f32; 12]; 3];
    for (w, nw) in noise.iter_mut().enumerate() {
        for (b, n) in nw.iter_mut().enumerate() {
            let (lo, hi) = run(b, w);
            *n = run_noise(freq_bitstream, &q.coeffs, gain, 0, lo, hi);
        }
    }
    let mut coeffs = q.coeffs;
    let (mut side, mut compress) = (q.side.clone(), 0u16);
    for _ in 0..36 {
        let mut pick = None;
        let mut worst = f32::NEG_INFINITY;
        for w in 0..3 {
            for b in 0..12 {
                let score = noise[w][b] / (thr[b] * domain).max(1e-20);
                if sf[w][b] < max_sf_short(b) && score > worst {
                    worst = score;
                    pick = Some((w, b));
                }
            }
        }
        let Some((w, b)) = pick else { break };
        let mut trial = sf;
        trial[w][b] += 1;
        let (t_compress, t_sf_bits) = choose_compress_short(&trial);
        let (lo, hi) = run(b, w);
        quantize_run(freq_bitstream, &xrp, gain, trial[w][b], lo, hi, &mut coeffs);
        let clipped = coeffs[lo..hi].iter().any(|&c| c.abs() > MAX_UNCLIPPED);
        let fit = if clipped {
            None
        } else {
            let (t_side, t_bits) = super::huffman::select(header, &coeffs, BlockType::Short);
            (t_bits + t_sf_bits <= bit_budget).then_some(t_side)
        };
        let Some(t_side) = fit else {
            quantize_run(freq_bitstream, &xrp, gain, sf[w][b], lo, hi, &mut coeffs);
            break;
        };
        noise[w][b] = run_noise(freq_bitstream, &coeffs, gain, trial[w][b], lo, hi);
        sf = trial;
        side = t_side;
        compress = t_compress;
        super::prof::SHORT_REFINE_STEPS.fetch_add(1, std::sync::atomic::Ordering::Relaxed);
    }
    side.global_gain = gain as u8;
    side.scalefac_compress = compress;
    q.coeffs = coeffs;
    q.side = side;
    // Layout shared with the caller: scalefactors[3·band + window].
    for (w, sw) in sf.iter().enumerate() {
        for (b, &v) in sw.iter().enumerate() {
            q.scalefactors[3 * b + w] = v;
        }
    }
    q
}

/// **Q5 block-type FSM.** Given the previous granule's window type and this
/// frame's per-granule attack flags, choose valid window types that bracket every
/// attack with the required transition windows. Produces sequences like
/// `LONG…LONG | START, SHORT | (SHORT, SHORT)* | STOP, LONG` — a `START` always
/// precedes a `SHORT` and a `STOP` always follows it, the constraint the decoder's
/// overlapping windows require. MPEG-1 (2 granules per frame).
#[must_use]
pub fn decide_block_types(prev: BlockType, attacks: &[bool]) -> (Vec<BlockType>, BlockType) {
    if attacks.len() != 2 {
        // MPEG-2 (single granule) not yet block-switched.
        return (vec![BlockType::Long; attacks.len()], BlockType::Long);
    }
    let (types, new_prev) = decide_block_types_pair(prev, attacks);
    (types.to_vec(), new_prev)
}

/// [`decide_block_types`] without the `Vec`: the encoder calls this once per
/// frame, and a frame has at most two granules. For a single-granule (MPEG-2)
/// frame only `[0]` is meaningful, and it is `Long`, as above.
pub(crate) fn decide_block_types_pair(
    prev: BlockType,
    attacks: &[bool],
) -> ([BlockType; 2], BlockType) {
    if attacks.len() != 2 {
        return ([BlockType::Long; 2], BlockType::Long);
    }
    let any = attacks[0] || attacks[1];
    let types = if prev == BlockType::Short {
        // Inside a short run: continue or wind down with a STOP.
        if any {
            [BlockType::Short, BlockType::Short]
        } else {
            [BlockType::Stop, BlockType::Long]
        }
    } else if any {
        // Long/Stop and an attack this frame: transition in (START → SHORT).
        [BlockType::Start, BlockType::Short]
    } else {
        [BlockType::Long, BlockType::Long]
    };
    let new_prev = types[1];
    (types, new_prev)
}

/// Map the LONG-block masking thresholds onto the short-block band grid.
///
/// The psychoacoustic model only produces long-band thresholds (its
/// `block_type` is hard-wired to `Long`), so a short granule has no masking
/// curve of its own. Short blocks have 3x coarser frequency resolution — 192
/// lines per window against 576 — so short band `b`, spanning per-window lines
/// `[s0, s1)`, covers the same frequencies as long lines `[3·s0, 3·s1)`.
///
/// The overlapping long bands are combined by taking the MINIMUM. That is the
/// conservative direction on purpose: a short band straddling a quiet long band
/// must not inherit a loud neighbour's noise allowance, because the audible
/// failure mode here is pre-echo on exactly the transients short blocks exist
/// to code.
///
/// This is an approximation, and it is the reason short-block VBR is coarser
/// than long-block VBR. The principled fix is a short-block psymodel.
fn short_thresholds(sample_rate: u32, long_thr: &[f32; crate::frame::SFB_LONG]) -> [f32; 13] {
    let long_off = crate::tables::sfb_long_offsets(sample_rate);
    let short_off = crate::tables::sfb_short_offsets(sample_rate);
    let mut out = [f32::MAX; 13];
    for (b, o) in out.iter_mut().enumerate() {
        let (lo, hi) = (3 * short_off[b] as usize, 3 * short_off[b + 1] as usize);
        let mut m = f32::MAX;
        for lb in 0..21 {
            let (llo, lhi) = (long_off[lb] as usize, long_off[lb + 1] as usize);
            if llo < hi && lo < lhi {
                m = m.min(long_thr[lb]);
            }
        }
        *o = if m == f32::MAX { long_thr[20] } else { m };
    }
    out
}

/// Quantization noise per short band, summed across the three windows.
///
/// In bitstream order a short band's three windows are adjacent, so band `b`
/// occupies reordered indices `[3·off[b], 3·off[b+1])` — the same span the
/// threshold mapping above uses.
fn short_band_noise(
    sample_rate: u32,
    freq_bs: &[f32; GRANULE_LINES],
    coeffs: &[i32; GRANULE_LINES],
    gain: i32,
) -> [f32; 13] {
    let off = crate::tables::sfb_short_offsets(sample_rate);
    let scale = 2f64.powf(0.25 * f64::from(gain - 210));
    let mut noise = [0f32; 13];
    for (b, n) in noise.iter_mut().enumerate() {
        let (lo, hi) = (
            (3 * off[b] as usize).min(GRANULE_LINES),
            (3 * off[b + 1] as usize).min(GRANULE_LINES),
        );
        let mut e = 0f64;
        for i in lo..hi {
            let xr = f64::from(coeffs[i].signum())
                * super::quantize::requant_magnitude(coeffs[i])
                * scale;
            let d = f64::from(freq_bs[i]) - xr;
            e += d * d;
        }
        *n = e as f32;
    }
    noise
}

/// **VBR short blocks.** Coarsest gain (fewest bits) whose peak NMR still meets
/// `target_nmr`, bounded by the frame's physical capacity.
///
/// Short granules previously ignored `-q:a` entirely and always took the CBR bit
/// budget, so on transient-heavy content — where short blocks are 15-37% of
/// granules — the quality knob was partly dead even after the VBR rate control
/// was fixed.
///
/// Mirrors `quantize::loops_vbr`: same search shape, and the same domain
/// rescaling, because the thresholds live in the FFT power domain while the
/// noise is measured in the MDCT domain.
#[must_use]
pub fn quantize_short_vbr(
    header: &FrameHeader,
    freq_bitstream: &[f32; GRANULE_LINES],
    long_thresholds: &[f32; crate::frame::SFB_LONG],
    signal_energy: f32,
    target_nmr: f32,
    bit_ceiling: usize,
) -> QuantizedGranule {
    let xrp = super::quantize::xrpow(freq_bitstream);
    let thr = short_thresholds(header.sample_rate, long_thresholds);
    let mdct_energy: f32 = freq_bitstream.iter().map(|x| x * x).sum();
    let domain_scale = if signal_energy > 1e-20 && mdct_energy > 1e-20 {
        mdct_energy / signal_energy
    } else {
        1.0
    };
    let ok = |g: i32| {
        let coeffs = quantize_uniform(freq_bitstream, &xrp, g);
        if !coeffs.iter().all(|&c| c.abs() <= MAX_UNCLIPPED) {
            return false;
        }
        // A quality target is never a licence to overflow the frame.
        if super::huffman::cost_short(header, &coeffs) > bit_ceiling {
            return false;
        }
        let noise = short_band_noise(header.sample_rate, freq_bitstream, &coeffs, g);
        noise
            .iter()
            .zip(thr.iter())
            .map(|(&n, &t)| n / (t * domain_scale).max(1e-20))
            .fold(0f32, f32::max)
            <= target_nmr
    };
    // Largest gain (coarsest, fewest bits) that still satisfies everything.
    let (mut lo, mut hi) = (0i32, 255i32);
    while lo < hi {
        let mid = (lo + hi + 1) / 2;
        if ok(mid) {
            lo = mid;
        } else {
            hi = mid - 1;
        }
    }
    // If nothing met the target, fall back to the budget-driven path rather than
    // emitting an over-target or clipped granule.
    if !ok(lo) {
        return quantize_short(header, freq_bitstream, bit_ceiling);
    }
    let coeffs = quantize_uniform(freq_bitstream, &xrp, lo);
    let (mut side, _) = super::huffman::select(header, &coeffs, BlockType::Short);
    side.global_gain = lo as u8;
    side.scalefac_compress = 0;
    QuantizedGranule {
        coeffs,
        side,
        scalefactors: [0; 39],
    }
}

#[cfg(test)]
mod tests {
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

    #[test]
    fn short_coefficients_round_trip_through_decoder() {
        // A short-block spectrum (subband order) → reorder → short quantize →
        // Huffman → decoder Huffman + reorder-requantize must recover it.
        let header = hdr();
        let mut s = 0xABCD_1234u32;
        let mut freq = [0f32; GRANULE_LINES];
        for (i, v) in freq.iter_mut().enumerate() {
            s = s.wrapping_mul(1_664_525).wrapping_add(1_013_904_223);
            // taper high lines so there's an rzero tail (realistic short block)
            let taper = 1.0 - (i as f32 / GRANULE_LINES as f32);
            *v = ((s >> 8) as f32 / (1u32 << 24) as f32 - 0.5) * 8.0 * taper;
        }

        let freq_bs = reorder_subband_to_bitstream(header.sample_rate, &freq);
        let quant = quantize_short(&header, &freq_bs, 100_000);

        // Emit the Huffman spectrum, then decode it back.
        let mut w = crate::bitio::BitWriter::new();
        let hbits = super::super::huffman::encode(&quant, &header, &mut w);
        let bits = w.finish();

        let mut gi = quant.side.clone();
        gi.part2_3_length = hbits as u16;
        let mut pos = 0;
        let (coeffs, nz) = crate::decode::huffman::decode(&bits, &mut pos, hbits, &header, &gi);
        assert_eq!(
            coeffs, quant.coeffs,
            "short Huffman must round-trip exactly"
        );

        // Requantize (with the decoder's short reorder) → subband order.
        let mut out = [0f32; GRANULE_LINES];
        crate::decode::requantize::apply(
            &header,
            &gi,
            &ScaleFactors::default(),
            &coeffs,
            nz,
            &mut out,
        );

        // out (subband order) ≈ freq (subband order), to quantization error.
        let mut sig = 0f64;
        let mut err = 0f64;
        for i in 0..GRANULE_LINES {
            sig += f64::from(freq[i]).powi(2);
            err += f64::from(freq[i] - out[i]).powi(2);
        }
        let snr = 10.0 * (sig / err).log10();
        eprintln!("[Q5] short-block coefficient round-trip SNR {snr:.1} dB");
        assert!(snr > 40.0, "short-block coding SNR too low: {snr:.1} dB");
    }

    #[test]
    fn reorder_inverts_the_decoder_mapping() {
        // Forward reorder then the decoder's src/dst mapping must return the
        // original subband-order spectrum.
        let subband: [f32; GRANULE_LINES] = std::array::from_fn(|i| (i as f32 * 0.013).sin());
        let bitstream = reorder_subband_to_bitstream(44100, &subband);

        let off = tables::sfb_short_offsets(44100);
        let mut recovered = [0f32; GRANULE_LINES];
        for sfb in 0..13 {
            let start = off[sfb] as usize;
            let width = (off[sfb + 1] - off[sfb]) as usize;
            for window in 0..3 {
                for f in 0..width {
                    let src = start * 3 + window * width + f;
                    let dst = start * 3 + window + f * 3;
                    recovered[dst] = bitstream[src]; // decoder's placement
                }
            }
        }
        for i in 0..GRANULE_LINES {
            assert!((recovered[i] - subband[i]).abs() < 1e-9, "line {i}");
        }
    }
}
