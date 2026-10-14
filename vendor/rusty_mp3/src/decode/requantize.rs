//! Requantization: integer coefficients → dequantized frequency lines.
//!
//! Per line: `xr = sign(is) · |is|^(4/3) · 2^(0.25·A) · 2^(-B)` where `A` folds in
//! `global_gain` (minus 210, and `8·subblock_gain` per window for short blocks)
//! and `B` folds in the band scalefactor scaled by `scalefac_scale` (and
//! `preflag·pretab` for long blocks). Short blocks are simultaneously reordered
//! from scalefactor-band order (Huffman output) into subband order (IMDCT input):
//! within a band, coded `window·width + freq` maps to interleaved `window +
//! freq·3`.

use std::sync::OnceLock;

use crate::frame::{BlockType, GranuleSideInfo, GRANULE_LINES};
use crate::header::{FrameHeader, MpegVersion};
use crate::tables;

use super::scalefactors::ScaleFactors;

/// `|n|^(4/3)` table for the `0..=8206` Huffman magnitudes (linbits reach 8206).
/// Returned by reference so the per-line loop indexes it directly instead of
/// re-checking the `OnceLock` and making a call 576× per granule.
fn pow43_table() -> &'static [f64] {
    static T: OnceLock<Vec<f64>> = OnceLock::new();
    T.get_or_init(|| (0..8207).map(|i| f64::from(i).powf(4.0 / 3.0)).collect())
}

/// Dequantize one line: `sign(c)·|c|^(4/3)·scale`. Branchless — for `c == 0`,
/// `sign(0)=0` and `pow[0]=0`, so the product is `0.0`, identical to a guard
/// (which kept the per-line loop from vectorizing). `pow` is the hoisted table.
#[inline]
fn dequant(c: i32, pow: &[f64], scale: f64) -> f32 {
    (f64::from(c.signum()) * pow[c.unsigned_abs() as usize] * scale) as f32
}

/// Dequantize `coeffs` into `out` (576 lines), applying gains, scalefactors, and
/// — for short blocks — the reorder.
pub fn apply(
    header: &FrameHeader,
    gi: &GranuleSideInfo,
    sf: &ScaleFactors,
    coeffs: &[i32; GRANULE_LINES],
    nz: usize,
    out: &mut [f32; GRANULE_LINES],
) {
    out.fill(0.0);
    // D8 rests on this: the Huffman stage writes only [0, nz) into a zeroed
    // array, so everything above the rzero boundary is zero and can be skipped.
    // Cheap to assert, and a silent corruption if it ever stops holding.
    debug_assert!(
        coeffs[nz.min(GRANULE_LINES)..].iter().all(|&c| c == 0),
        "rzero invariant broken: non-zero coefficient at or above nz={nz}"
    );
    let pow = pow43_table(); // hoisted once per granule, not per line
    let gain = i32::from(gi.global_gain) - 210;
    let sf_mult = if gi.scalefac_scale { 1.0f64 } else { 0.5 };
    let is_short = gi.window_switching && gi.block_type == BlockType::Short;

    // **D8** — stop at the rzero boundary. `nz` is the index past the last
    // coefficient the Huffman stage actually decoded; everything above it is
    // zero, `out` was just filled with 0.0, and `dequant(0, ..) == 0.0` exactly
    // (`0.signum() == 0`), so the tail is already correct. This is byte-identical,
    // not an approximation.
    //
    // It was already being computed and handed to us as `_nz`, then ignored: the
    // stage dequantized all 576 lines on every granule. On real music the rzero
    // boundary typically sits well below 576, so this also skips those bands'
    // per-band `2f64.powf(exp)` entirely.
    if is_short {
        // Short block: dequant per (band, window) and reorder into subband order.
        //
        // A MIXED block codes its lowest two subbands LONG: the first `long_end`
        // long scalefactor bands (8 for MPEG-1, 6 for MPEG-2/2.5 -- FFmpeg's
        // `switch_point` rule) take the long gain, long scalefactors and pretab,
        // with no subblock gain and no reorder; the short bands then start at 3.
        // Treating those lines as short applied a subblock gain and a reorder to
        // long data: ISO l3-si_block decoded 19/126 granules wrong.
        let first_short = if gi.mixed_block {
            let long_end = if header.version == MpegVersion::V1 {
                8
            } else {
                6
            };
            let off = tables::sfb_long_offsets(header.sample_rate);
            for sfb in 0..long_end {
                let start = off[sfb] as usize;
                if start >= nz {
                    break;
                }
                let end = (off[sfb + 1] as usize).min(nz);
                let pre = f64::from(if gi.preflag { tables::PRETAB[sfb] } else { 0 });
                let exp = 0.25 * f64::from(gain) - sf_mult * (f64::from(sf.long[sfb]) + pre);
                let scale = 2f64.powf(exp);
                for i in start..end {
                    out[i] = dequant(coeffs[i], pow, scale);
                }
            }
            3
        } else {
            0
        };
        let off = tables::sfb_short_offsets(header.sample_rate);
        for sfb in first_short..13 {
            let start = off[sfb] as usize;
            let width = (off[sfb + 1] - off[sfb]) as usize;
            if start * 3 >= nz {
                break; // rzero, in bitstream order
            }
            for window in 0..3 {
                let gain_w = gain - 8 * i32::from(gi.subblock_gain[window]);
                let exp = 0.25 * f64::from(gain_w) - sf_mult * f64::from(sf.short[window][sfb]);
                let scale = 2f64.powf(exp);
                for f in 0..width {
                    let src = start * 3 + window * width + f;
                    if src >= nz {
                        break; // this window's run is past rzero
                    }
                    let dst = start * 3 + window + f * 3;
                    if src < GRANULE_LINES && dst < GRANULE_LINES {
                        out[dst] = dequant(coeffs[src], pow, scale);
                    }
                }
            }
        }
    } else {
        // Long block: each band scales a contiguous run of lines.
        let off = tables::sfb_long_offsets(header.sample_rate);
        for sfb in 0..22 {
            let start = off[sfb] as usize;
            if start >= nz {
                break; // rzero: this band and every band above it is all zeros
            }
            let end = (off[sfb + 1] as usize).min(GRANULE_LINES).min(nz);
            let pre = f64::from(if gi.preflag { tables::PRETAB[sfb] } else { 0 });
            let exp = 0.25 * f64::from(gain) - sf_mult * (f64::from(sf.long[sfb]) + pre);
            let scale = 2f64.powf(exp);
            for i in start..end {
                out[i] = dequant(coeffs[i], pow, scale);
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
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
    fn long_block_power_law_and_sign() {
        // global_gain 210 → gain 0, scalefac 0, no preflag → scale = 2^0 = 1, so
        // xr = sign(is)·|is|^(4/3).
        let gi = GranuleSideInfo {
            global_gain: 210,
            ..Default::default()
        };
        let sf = ScaleFactors::default();
        let mut coeffs = [0i32; GRANULE_LINES];
        coeffs[0] = 3;
        coeffs[1] = -2;
        coeffs[2] = 1;
        let mut out = [0f32; GRANULE_LINES];
        apply(&hdr(), &gi, &sf, &coeffs, 3, &mut out);
        assert!((out[0] - 3f32.powf(4.0 / 3.0)).abs() < 1e-4);
        assert!((out[1] + 2f32.powf(4.0 / 3.0)).abs() < 1e-4);
        assert!((out[2] - 1.0).abs() < 1e-4);
    }

    #[test]
    fn scalefactor_halves_the_step() {
        // scalefac 2 on band 0, scalefac_scale 0 (×0.5) → B = 0.5·2 = 1 → ÷2.
        let gi = GranuleSideInfo {
            global_gain: 210,
            ..Default::default()
        };
        let mut sf = ScaleFactors::default();
        sf.long[0] = 2;
        let mut coeffs = [0i32; GRANULE_LINES];
        coeffs[0] = 1;
        let mut out = [0f32; GRANULE_LINES];
        apply(&hdr(), &gi, &sf, &coeffs, 1, &mut out);
        assert!((out[0] - 0.5).abs() < 1e-4, "1^(4/3)·2^-1 = 0.5");
    }
}
