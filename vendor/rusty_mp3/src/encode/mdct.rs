//! Brick **L2** — the forward (analysis) MDCT, the TDAC inverse of decode's
//! hybrid IMDCT (`decode/imdct.rs`).
//!
//! Per subband, a 36-sample lapped frame (the previous granule's 18 samples plus
//! this granule's 18) is windowed and transformed to 18 frequency lines — one
//! 36-point MDCT for long/start/stop blocks, or three 12-point MDCTs for short
//! blocks. The same Princen-Bradley windows and cosine matrices as the decoder are
//! used, so forward∘inverse reconstructs the subband samples exactly (up to the
//! one-granule lap delay).
//!
//! Two details make it the precise inverse:
//! * **Frequency inversion** — the decoder negates odd-indexed output samples of
//!   odd subbands *after* the IMDCT. We apply the same involution to each
//!   subband's samples *before* the lap, so the two cancel on round-trip.
//! * **Scaling** — the decoder's IMDCT carries no normalisation, so the forward
//!   carries the whole MDCT constant: `1/9` for the 36-point transform (`N/4`,
//!   N=36) and `1/3` for the 12-point one.

use std::f64::consts::PI;
use std::sync::OnceLock;

use crate::frame::{BlockType, GRANULE_LINES, SUBBANDS, SUBBAND_LINES};

/// Forward MDCT reconstruction constants (`N/4`): the decoder's IMDCT is
/// unnormalised, so the analysis side carries the full scale.
const ALPHA_LONG: f32 = 1.0 / 9.0;
const ALPHA_SHORT: f32 = 1.0 / 3.0;

struct Kernels {
    cos36: [[f32; 18]; 36],
    cos12: [[f32; 6]; 12],
    /// Long(0), Start(1), Stop(3) windows; index 2 unused (short handled apart).
    win: [[f32; 36]; 4],
    win_short: [f32; 12],
}

/// Identical to the decoder's kernels — same matrices and PB windows, so the
/// transforms are exact inverses.
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

/// Forward-transform one channel's granule of subband samples into 576 frequency
/// lines (subband-major, matching the decoder's `lines` layout). `overlap` carries
/// the previous granule's (frequency-inverted) subband samples in, and this
/// granule's out — the lapping memory the MDCT needs.
pub fn forward(
    subbands: &[[f32; SUBBAND_LINES]; SUBBANDS],
    block_type: BlockType,
    overlap: &mut [f32; GRANULE_LINES],
) -> [f32; GRANULE_LINES] {
    let t = kernels();
    let mut lines = [0f32; GRANULE_LINES];
    let is_short = block_type == BlockType::Short;

    for sb in 0..SUBBANDS {
        let base = sb * SUBBAND_LINES;

        // This granule's samples, frequency-inverted for odd subbands (the
        // involution the decoder applies post-IMDCT, mirrored here pre-lap).
        let mut cur = subbands[sb];
        if sb & 1 == 1 {
            let mut i = 1;
            while i < SUBBAND_LINES {
                cur[i] = -cur[i];
                i += 2;
            }
        }

        // The 36-sample lapped frame: previous granule (older) then current.
        let mut u = [0f32; 36];
        u[..18].copy_from_slice(&overlap[base..base + 18]);
        u[18..].copy_from_slice(&cur[..18]);

        if is_short {
            // Three 12-point MDCTs at offsets 6, 12, 18 within the frame; line k of
            // window w lands at `base + w + 3k` (the decoder's short layout).
            for w in 0..3 {
                let off = 6 + w * 6;
                let mut seg = [0f32; 12];
                for n in 0..12 {
                    seg[n] = u[off + n] * t.win_short[n];
                }
                for k in 0..6 {
                    let mut acc = 0f32;
                    for (n, &s) in seg.iter().enumerate() {
                        acc += s * t.cos12[n][k];
                    }
                    lines[base + w + 3 * k] = ALPHA_SHORT * acc;
                }
            }
        } else {
            let wt = match block_type {
                BlockType::Start => 1,
                BlockType::Stop => 3,
                _ => 0,
            };
            let mut z = [0f32; 36];
            for n in 0..36 {
                z[n] = u[n] * t.win[wt][n];
            }
            for k in 0..18 {
                let mut acc = 0f32;
                for (n, &zn) in z.iter().enumerate() {
                    acc += zn * t.cos36[n][k];
                }
                lines[base + k] = ALPHA_LONG * acc;
            }
        }

        // Save this granule's (inverted) samples as the next lap's left half.
        overlap[base..base + 18].copy_from_slice(&cur);
    }
    lines
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::decode::imdct;
    use crate::frame::GranuleSideInfo;

    /// Deterministic pseudo-random subband samples for granule `g`.
    fn subbands(g: usize) -> [[f32; SUBBAND_LINES]; SUBBANDS] {
        let mut s = (g as u32).wrapping_mul(2_654_435_761).wrapping_add(1);
        let mut out = [[0f32; SUBBAND_LINES]; SUBBANDS];
        for row in &mut out {
            for v in row.iter_mut() {
                s = s.wrapping_mul(1_664_525).wrapping_add(1_013_904_223);
                *v = ((s >> 8) as f32 / (1u32 << 24) as f32) * 2.0 - 1.0;
            }
        }
        out
    }

    fn gi(block_type: BlockType) -> GranuleSideInfo {
        GranuleSideInfo {
            window_switching: block_type != BlockType::Long,
            block_type,
            ..Default::default()
        }
    }

    /// Run a sequence of block types through forward→IMDCT and return, per granule,
    /// the max reconstruction error of `out[g]` vs the input subbands of `g-1`.
    fn reconstruct(seq: &[BlockType]) -> Vec<f32> {
        let mut enc_overlap = [0f32; GRANULE_LINES];
        let mut dec_overlap = [0f32; GRANULE_LINES];
        let mut errs = Vec::new();
        for (g, &bt) in seq.iter().enumerate() {
            let sb = subbands(g);
            let lines = forward(&sb, bt, &mut enc_overlap);
            let out = imdct::hybrid(&gi(bt), &lines, &mut dec_overlap);
            if g >= 1 {
                let prev = subbands(g - 1);
                let mut max = 0f32;
                for k in 0..SUBBANDS {
                    for v in 0..SUBBAND_LINES {
                        let e = (out[k * SUBBAND_LINES + v] - prev[k][v]).abs();
                        if e > max {
                            max = e;
                        }
                    }
                }
                errs.push(max);
            }
        }
        errs
    }

    #[test]
    fn long_blocks_reconstruct_exactly() {
        // Steady-state long blocks: out[g] == subbands[g-1] (one-granule lap delay).
        let errs = reconstruct(&[BlockType::Long; 6]);
        let worst = errs.iter().skip(1).copied().fold(0.0, f32::max);
        assert!(worst < 1e-5, "long-block TDAC error {worst}");
    }

    #[test]
    fn short_blocks_reconstruct_exactly() {
        let errs = reconstruct(&[BlockType::Short; 6]);
        let worst = errs.iter().skip(1).copied().fold(0.0, f32::max);
        assert!(worst < 1e-5, "short-block TDAC error {worst}");
    }

    #[test]
    fn transition_sequence_reconstructs() {
        // The only valid way to exercise start/stop: long→start→short→stop→long.
        // The transition windows are PB against their designated neighbours, so the
        // settled granules in the middle must still reconstruct exactly.
        use BlockType::{Long, Short, Start, Stop};
        let seq = [Long, Long, Start, Short, Short, Stop, Long, Long];
        let errs = reconstruct(&seq);
        // errs[i] is the reconstruction of granule i (out[i+1] vs subbands[i]).
        // Every interior granule (1..=6) is bracketed by settled neighbours.
        let worst = errs.iter().skip(1).copied().fold(0.0, f32::max);
        assert!(worst < 1e-5, "transition TDAC error {worst}");
    }

    /// The whole analysis front-end (L1 ∘ L2) composed with the decoder back-end:
    /// PCM → analyze → forward MDCT → IMDCT → synthesis → PCM must reconstruct.
    #[test]
    #[cfg_attr(
        miri,
        ignore = "end-to-end / exhaustive: too slow to interpret under Miri (> 5 min)"
    )]
    fn full_analysis_chain_reconstructs_pcm() {
        use crate::decode::synthesis;
        use crate::encode::filterbank;
        use std::f32::consts::PI as PIf;

        let granules = 20;
        let n = granules * GRANULE_LINES;
        let input: Vec<f32> = (0..n)
            .map(|i| 0.45 * (2.0 * PIf * 900.0 * i as f32 / 44100.0).sin())
            .collect();

        let mut afifo = [0f32; 512];
        let mut enc_ov = [0f32; GRANULE_LINES];
        let mut dec_ov = [0f32; GRANULE_LINES];
        let mut sfifo = [0f32; 1024];
        let mut output = Vec::with_capacity(n);
        for g in 0..granules {
            let sb = filterbank::analyze(&input[g * GRANULE_LINES..], &mut afifo);
            let lines = forward(&sb, BlockType::Long, &mut enc_ov);
            let time = imdct::hybrid(&gi(BlockType::Long), &lines, &mut dec_ov);
            let pcm = synthesis::polyphase(&time, &mut sfifo);
            output.extend_from_slice(&pcm);
        }

        // Total delay = filterbank (481) + one MDCT-lap granule (576) = 1057.
        let (mut best_snr, mut best_delay) = (f64::NEG_INFINITY, 0usize);
        for delay in 1055..=1059 {
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
        eprintln!("[L1∘L2] full-chain SNR {best_snr:.1} dB at delay {best_delay}");
        assert!(
            best_snr > 70.0,
            "full analysis-chain SNR too low: {best_snr:.1} dB"
        );
    }
}
