//! Joint-stereo reconstruction: MS (mid/side) and intensity stereo.
//!
//! Active only when the header mode is JointStereo; `mode_extension` says which
//! of MS / intensity is on. MS rotates (mid, side) → (L, R) by `1/√2`:
//! `L = (M+S)/√2`, `R = (M-S)/√2`. Intensity stereo, above the intensity bound,
//! reconstructs the right channel from the left using a per-band position.

use std::sync::OnceLock;

use crate::frame::{BlockType, ChannelMode, GranuleSideInfo, GranuleSpectrum, GRANULE_LINES};
use crate::header::{FrameHeader, MpegVersion};
use crate::tables;

use super::scalefactors::ScaleFactors;

const INV_SQRT2: f32 = std::f32::consts::FRAC_1_SQRT_2;

/// MPEG-1 intensity panning: for `is_pos` p in 0..=6, `L = x·f/(1+f)` and
/// `R = x/(1+f)` with `f = tan(p·π/12)` (ISO 11172-3 2.4.3.4.9.3). Computed in
/// f64 and stored as f32, as FFmpeg's float decoder does. `[0]` = left gain,
/// `[1]` = right gain.
fn is_table_v1() -> &'static [[f32; 7]; 2] {
    static T: OnceLock<[[f32; 7]; 2]> = OnceLock::new();
    T.get_or_init(|| {
        let mut t = [[0f32; 7]; 2];
        for i in 0..7 {
            let v = if i == 6 {
                1.0
            } else {
                let f = (i as f64 * std::f64::consts::PI / 12.0).tan();
                (f / (1.0 + f)) as f32
            };
            t[0][i] = v;
            t[1][6 - i] = v;
        }
        t
    })
}

/// MPEG-2 intensity panning (ISO 13818-3 2.4.3.2): `intensity_scale` (the low
/// bit of the right channel's `scalefac_compress`) picks a step of 2^-1/4 or
/// 2^-1/2; an odd `is_pos` attenuates the LEFT channel, an even one the RIGHT,
/// by `step^((is_pos+1)/2)`. `[scale][0|1][is_pos]`.
///
/// 32 positions, not 16: the right channel's position fields are up to FIVE bits
/// wide (`int_scalefac_compress < 180` gives slen 4/5/5), so positions reach 30
/// legally. FFmpeg's table stops at 16 and treats 16..=31 as "not intensity",
/// which is where it parts from minimp3 and the spec on ISO l3-test45/46.
fn is_table_lsf() -> &'static [[[f32; 32]; 2]; 2] {
    static T: OnceLock<[[[f32; 32]; 2]; 2]> = OnceLock::new();
    T.get_or_init(|| {
        let mut t = [[[0f32; 32]; 2]; 2];
        for i in 0..32 {
            for (j, tj) in t.iter_mut().enumerate() {
                let e = -((j as i32 + 1) * ((i as i32 + 1) >> 1));
                let f = 2f64.powf(f64::from(e) / 4.0) as f32;
                let k = i & 1;
                tj[k ^ 1][i] = f;
                tj[k][i] = 1.0;
            }
        }
        t
    })
}

/// Undo M/S on `idx` lines (no-op unless `ms`).
#[inline]
fn ms_lines(
    l: &mut [f32; GRANULE_LINES],
    r: &mut [f32; GRANULE_LINES],
    idx: impl Iterator<Item = usize>,
    ms: bool,
) {
    if !ms {
        return;
    }
    for i in idx {
        let (m, s) = (l[i], r[i]);
        l[i] = (m + s) * INV_SQRT2;
        r[i] = (m - s) * INV_SQRT2;
    }
}

/// Convert the two coded channels in place from the joint representation back to
/// independent left/right. `sf_right` is the right channel's scalefactors, which
/// under intensity stereo carry the per-band panning positions.
pub fn process(
    header: &FrameHeader,
    gi: &[GranuleSideInfo; 2],
    sf_right: &ScaleFactors,
    spectrum: &mut GranuleSpectrum,
) {
    let ChannelMode::JointStereo {
        ms_stereo: ms,
        intensity_stereo: intensity,
    } = header.channel_mode
    else {
        return; // plain stereo / mono: nothing to undo
    };
    let (a, b) = spectrum.lines.split_at_mut(1);
    let (l, r) = (&mut a[0], &mut b[0]);
    if !intensity {
        ms_lines(l, r, 0..GRANULE_LINES, ms);
        return;
    }

    // Intensity stereo. Walking DOWN from the top band, every band whose right
    // channel is all zero is intensity-coded: the left channel carries the sum and
    // the right channel's scalefactor is the panning position. The first band with
    // a non-zero right line is the intensity BOUND; it and everything below are
    // ordinary L/R (or M/S). Short blocks keep one bound PER WINDOW. The shape is
    // FFmpeg's `compute_stereo`, which the ISO vectors gate.
    let g1 = &gi[1];
    let lsf = header.version != MpegVersion::V1;
    let short = g1.window_switching && g1.block_type == BlockType::Short;
    let (long_end, short_start) = match (short, g1.mixed_block) {
        (false, _) => (22, 13),
        (true, false) => (0, 0),
        (true, true) => (if lsf { 6 } else { 8 }, 3),
    };
    // Two MPEG-2 rules, each measured load-bearing on ISO l3-test45 / l3-test46
    // (both PASS at 1 LSB only with BOTH; drop the illegal-position rule and they
    // read 25.4 / 14.1 dB, drop the top-band rule and 37.7 / 35.4 dB):
    //
    //  * ILLEGAL positions: the largest value a band's position field can hold,
    //    2^slen - 1, means "not intensity" (ISO 13818-3; minimp3). FFmpeg does
    //    not apply this.
    //  * the TOP band (no position of its own) takes the band below's position
    //    only if that band is itself intensity-coded; otherwise the default
    //    centre position (3 for MPEG-1, 0 for MPEG-2) -- minimp3's rule. FFmpeg
    //    always copies band 20 / 11.
    let lim = if lsf {
        super::scalefactors::lsf_intensity_illegal(g1)
    } else {
        ScaleFactors {
            long: [0xFF; 22],
            short: [[0xFF; 13]; 3],
        }
    };
    // `None` = an illegal position: the band is not intensity-coded.
    let pan = |sf: u8, illegal: u8| -> Option<(f32, f32)> {
        if lsf {
            let t = &is_table_lsf()[(g1.scalefac_compress & 1) as usize];
            (sf < 32 && sf != illegal).then(|| (t[0][sf as usize], t[1][sf as usize]))
        } else {
            let t = is_table_v1();
            (sf < 7).then(|| (t[0][sf as usize], t[1][sf as usize]))
        }
    };
    let default_pos: u8 = if lsf { 0 } else { 3 };

    // Short bands (reordered layout: band `i`, window `w`, line `f` lives at
    // `3·start + w + 3f`). The last band has no scalefactor and reuses band 11's.
    let so = tables::sfb_short_offsets(header.sample_rate);
    let mut nz_short = [false; 3];
    for i in (short_start..13).rev() {
        let (start, width) = (so[i] as usize, (so[i + 1] - so[i]) as usize);
        let sfi = if i == 12 { 11 } else { i };
        for w in (0..3).rev() {
            let lines = (0..width)
                .map(|f| 3 * start + w + 3 * f)
                .filter(|&j| j < GRANULE_LINES);
            if !nz_short[w] {
                if lines.clone().any(|j| r[j] != 0.0) {
                    nz_short[w] = true;
                } else if let Some((v1, v2)) = {
                    // The top band has no position of its own.
                    let (pos, ill) = if i == 12 {
                        let prev_nz = (0..(so[12] - so[11]) as usize)
                            .map(|f| 3 * so[11] as usize + w + 3 * f)
                            .any(|j| j < GRANULE_LINES && r[j] != 0.0);
                        if prev_nz {
                            (default_pos, 0xFF)
                        } else {
                            (sf_right.short[w][11], lim.short[w][11])
                        }
                    } else {
                        (sf_right.short[w][sfi], lim.short[w][sfi])
                    };
                    pan(pos, ill)
                } {
                    for j in lines {
                        let x = l[j];
                        l[j] = x * v1;
                        r[j] = x * v2;
                    }
                    continue;
                }
            }
            ms_lines(l, r, lines, ms);
        }
    }

    // Long bands. Band 21 has no scalefactor and reuses band 20's.
    let lo = tables::sfb_long_offsets(header.sample_rate);
    let mut nz = nz_short.iter().any(|&b| b);
    for i in (0..long_end).rev() {
        let lines = (lo[i] as usize).min(GRANULE_LINES)..(lo[i + 1] as usize).min(GRANULE_LINES);
        if !nz {
            if r[lines.clone()].iter().any(|&v| v != 0.0) {
                nz = true;
            } else if let Some((v1, v2)) = {
                let (pos, ill) = if i == 21 {
                    let a = (lo[20] as usize).min(GRANULE_LINES);
                    let b = (lo[21] as usize).min(GRANULE_LINES);
                    if r[a..b].iter().any(|&v| v != 0.0) {
                        (default_pos, 0xFF)
                    } else {
                        (sf_right.long[20], lim.long[20])
                    }
                } else {
                    let k = if i == 21 { 20 } else { i };
                    (sf_right.long[k], lim.long[k])
                };
                pan(pos, ill)
            } {
                for j in lines {
                    let x = l[j];
                    l[j] = x * v1;
                    r[j] = x * v2;
                }
                continue;
            }
        }
        ms_lines(l, r, lines, ms);
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::header::MpegVersion;

    fn joint(ms: bool, is: bool) -> FrameHeader {
        FrameHeader {
            version: MpegVersion::V1,
            crc_protected: false,
            bitrate_kbps: 128,
            sample_rate: 44100,
            padding: false,
            channel_mode: ChannelMode::JointStereo {
                ms_stereo: ms,
                intensity_stereo: is,
            },
            copyright: false,
            original: true,
            emphasis: 0,
        }
    }

    #[test]
    fn ms_stereo_rotation() {
        let mut spec = GranuleSpectrum::default();
        spec.lines[0][0] = 1.0; // M
        spec.lines[1][0] = 1.0; // S
        process(
            &joint(true, false),
            &[GranuleSideInfo::default(), GranuleSideInfo::default()],
            &ScaleFactors::default(),
            &mut spec,
        );
        // L = (1+1)/√2 = √2, R = (1-1)/√2 = 0.
        assert!((spec.lines[0][0] - 2f32.sqrt()).abs() < 1e-6);
        assert!(spec.lines[1][0].abs() < 1e-6);
    }

    /// MPEG-1 intensity: the bound is the highest band with a non-zero RIGHT line;
    /// bands above it are panned from the left channel by the right channel's
    /// scalefactor, and bands at/below it get M/S when it is on.
    #[test]
    fn intensity_pans_above_the_bound_and_ms_below() {
        let mut spec = GranuleSpectrum::default();
        // Band 0 (lines 0..4 at 44.1 kHz): right non-zero -> the bound, M/S applies.
        spec.lines[0][0] = 1.0;
        spec.lines[1][0] = 1.0;
        // Band 5 (lines 20..24): right all zero, is_pos 6 -> all to the left.
        spec.lines[0][20] = 2.0;
        // Band 10 (lines 52..62): is_pos 0 -> all to the right (tan 0 = 0).
        spec.lines[0][52] = 3.0;
        // Band 12 (lines 80..90): is_pos 7 is illegal -> not intensity, left as is.
        spec.lines[0][80] = 4.0;
        let mut sf = ScaleFactors::default();
        sf.long[5] = 6;
        sf.long[10] = 0;
        sf.long[12] = 7;
        for b in [1, 2, 3, 4, 6, 7, 8, 9, 11, 13, 14, 15, 16, 17, 18, 19, 20] {
            sf.long[b] = 3;
        }
        process(
            &joint(true, true),
            &[GranuleSideInfo::default(), GranuleSideInfo::default()],
            &sf,
            &mut spec,
        );
        assert!(
            (spec.lines[0][0] - 2f32.sqrt()).abs() < 1e-6,
            "M/S below the bound"
        );
        assert!(spec.lines[1][0].abs() < 1e-6);
        assert_eq!(
            (spec.lines[0][20], spec.lines[1][20]),
            (2.0, 0.0),
            "is_pos 6: left only"
        );
        assert!(
            spec.lines[0][52].abs() < 1e-6 && (spec.lines[1][52] - 3.0).abs() < 1e-6,
            "is_pos 0: right only"
        );
        // Illegal position: M/S of (4, 0) since M/S is on.
        assert!((spec.lines[0][80] - 4.0 * INV_SQRT2).abs() < 1e-6);
        assert!((spec.lines[1][80] - 4.0 * INV_SQRT2).abs() < 1e-6);
    }

    #[test]
    fn plain_stereo_is_untouched() {
        let mut spec = GranuleSpectrum::default();
        spec.lines[0][0] = 0.7;
        spec.lines[1][0] = 0.3;
        let mut h = joint(true, false);
        h.channel_mode = ChannelMode::Stereo;
        process(
            &h,
            &[GranuleSideInfo::default(), GranuleSideInfo::default()],
            &ScaleFactors::default(),
            &mut spec,
        );
        assert_eq!(spec.lines[0][0], 0.7);
        assert_eq!(spec.lines[1][0], 0.3);
    }
}
