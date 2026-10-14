//! Constant tables defined by ISO/IEC 11172-3 (MPEG-1) and 13818-3 (MPEG-2).
//!
//! The small, fully-specified ones live here as real data. The large DSP tables
//! (the 34 Huffman codebooks, the 512-tap synthesis window, the scalefactor-band
//! boundary tables, the requantization power table) are declared with their
//! shape + a `brick:` note and filled in as we implement each stage — keeping the
//! framework compiling while the data is ported.

// ---- header decode tables (complete) -----------------------------------------

/// Bitrate (kbps) by `bitrate_index`, MPEG-1 Layer III. Index 0 = free format,
/// index 15 = reserved/invalid.
pub const BITRATE_V1_L3: [u32; 16] = [
    0, 32, 40, 48, 56, 64, 80, 96, 112, 128, 160, 192, 224, 256, 320, 0,
];

/// Bitrate (kbps) by `bitrate_index`, MPEG-2 / 2.5 Layer III.
pub const BITRATE_V2_L3: [u32; 16] = [
    0, 8, 16, 24, 32, 40, 48, 56, 64, 80, 96, 112, 128, 144, 160, 0,
];

/// Base sample rates (Hz) by `samplerate_index`, for MPEG-1. MPEG-2 halves these
/// and MPEG-2.5 quarters them.
pub const SAMPLE_RATE: [u32; 4] = [44100, 48000, 32000, 0];

// ---- scalefactor decode tables (complete) ------------------------------------

/// `scalefac_compress` → (slen1, slen2): bit-lengths of the two scalefactor
/// groups for MPEG-1 long blocks.
pub const SCALEFAC_COMPRESS_V1: [(u8, u8); 16] = [
    (0, 0),
    (0, 1),
    (0, 2),
    (0, 3),
    (3, 0),
    (1, 1),
    (1, 2),
    (1, 3),
    (2, 1),
    (2, 2),
    (2, 3),
    (3, 1),
    (3, 2),
    (3, 3),
    (4, 2),
    (4, 3),
];

// ---- scalefactor-band boundary tables (to port) ------------------------------

/// Long-block scalefactor-band boundaries: cumulative line offsets (23 entries =
/// 22 bands), indexed by `samplerate_index` (0 = 44100, 1 = 48000, 2 = 32000),
/// MPEG-1. Drives requantization, region derivation, and stereo band maps.
pub const SFB_OFFSET_LONG_V1: [[u16; 23]; 3] = [
    // 44100 Hz
    [
        0, 4, 8, 12, 16, 20, 24, 30, 36, 44, 52, 62, 74, 90, 110, 134, 162, 196, 238, 288, 342,
        418, 576,
    ],
    // 48000 Hz
    [
        0, 4, 8, 12, 16, 20, 24, 30, 36, 42, 50, 60, 72, 88, 106, 128, 156, 190, 230, 276, 330,
        384, 576,
    ],
    // 32000 Hz
    [
        0, 4, 8, 12, 16, 20, 24, 30, 36, 44, 54, 66, 82, 102, 126, 156, 194, 240, 296, 364, 448,
        550, 576,
    ],
];

/// Short-block scalefactor-band boundaries: per-window line offsets (14 entries =
/// 13 bands), indexed by `samplerate_index`, MPEG-1. The max (192) is one window
/// of the 576/3 short-block lines; reorder interleaves the three windows.
pub const SFB_OFFSET_SHORT_V1: [[u16; 14]; 3] = [
    // 44100 Hz
    [0, 4, 8, 12, 16, 22, 30, 40, 52, 66, 84, 106, 136, 192],
    // 48000 Hz
    [0, 4, 8, 12, 16, 22, 28, 38, 50, 64, 80, 100, 126, 192],
    // 32000 Hz
    [0, 4, 8, 12, 16, 22, 30, 42, 58, 78, 104, 138, 180, 192],
];

/// Long-block scalefactor-band boundaries for MPEG-2 LSF (ISO/IEC 13818-3),
/// indexed `0 = 22050, 1 = 24000, 2 = 16000`. Validated against FFmpeg's decode.
pub const SFB_OFFSET_LONG_V2: [[u16; 23]; 3] = [
    // 22050 Hz
    [
        0, 6, 12, 18, 24, 30, 36, 44, 54, 66, 80, 96, 116, 140, 168, 200, 238, 284, 336, 396, 464,
        522, 576,
    ],
    // 24000 Hz
    [
        0, 6, 12, 18, 24, 30, 36, 44, 54, 66, 80, 96, 114, 136, 162, 194, 232, 278, 332, 394, 464,
        540, 576,
    ],
    // 16000 Hz
    [
        0, 6, 12, 18, 24, 30, 36, 44, 54, 66, 80, 96, 116, 140, 168, 200, 238, 284, 336, 396, 464,
        522, 576,
    ],
];

/// Short-block scalefactor-band boundaries for MPEG-2 LSF, per window (×3).
pub const SFB_OFFSET_SHORT_V2: [[u16; 14]; 3] = [
    // 22050 Hz
    [0, 4, 8, 12, 18, 24, 32, 42, 56, 74, 100, 132, 174, 192],
    // 24000 Hz
    [0, 4, 8, 12, 18, 26, 36, 48, 62, 80, 104, 136, 180, 192],
    // 16000 Hz
    [0, 4, 8, 12, 18, 26, 36, 48, 62, 80, 104, 134, 174, 192],
];

/// MPEG-2.5 8000 Hz has its own bands (a very cramped top end), unlike 11025 and
/// 12000 which reuse the MPEG-2 22050/24000 grids. Long table validated vs FFmpeg.
pub const SFB_OFFSET_LONG_V25_8000: [u16; 23] = [
    0, 12, 24, 36, 48, 60, 72, 88, 108, 132, 160, 192, 232, 280, 336, 400, 476, 566, 568, 570, 572,
    574, 576,
];

/// MPEG-2.5 8000 Hz short bands (not exercised by the long-only V2/2.5 encoder).
pub const SFB_OFFSET_SHORT_V25_8000: [u16; 14] =
    [0, 8, 16, 24, 36, 52, 72, 96, 124, 160, 162, 164, 166, 192];

/// Long-block scalefactor-band offsets for a sample rate (MPEG-1 + MPEG-2 LSF).
#[must_use]
pub fn sfb_long_offsets(sample_rate: u32) -> &'static [u16; 23] {
    match sample_rate {
        48000 => &SFB_OFFSET_LONG_V1[1],
        32000 => &SFB_OFFSET_LONG_V1[2],
        // MPEG-2, plus MPEG-2.5 11025/12000 which reuse the 22050/24000 grids.
        // MPEG-2.5 11025/12000 share the MPEG-2 22050 LONG grid (ISO/minimp3 sr_idx 0);
        // note 12000 does NOT follow 24000 here (it's the low-rate family, not the mid).
        22050 | 11025 | 12000 => &SFB_OFFSET_LONG_V2[0],
        24000 => &SFB_OFFSET_LONG_V2[1],
        16000 => &SFB_OFFSET_LONG_V2[2],
        8000 => &SFB_OFFSET_LONG_V25_8000, // MPEG-2.5 8000 has its own bands
        _ => &SFB_OFFSET_LONG_V1[0],       // 44100 and fallback
    }
}

/// Short-block scalefactor-band offsets for a sample rate (MPEG-1 + MPEG-2 LSF).
#[must_use]
pub fn sfb_short_offsets(sample_rate: u32) -> &'static [u16; 14] {
    match sample_rate {
        48000 => &SFB_OFFSET_SHORT_V1[1],
        32000 => &SFB_OFFSET_SHORT_V1[2],
        22050 => &SFB_OFFSET_SHORT_V2[0],
        24000 => &SFB_OFFSET_SHORT_V2[1],
        // MPEG-2.5 11025/12000 share the 16000 SHORT grid (ISO/minimp3 sr_idx 0) — NOT
        // the 22050/24000 short grids their LONG bands use. (LONG 11025/12000 = 22050.)
        16000 | 11025 | 12000 => &SFB_OFFSET_SHORT_V2[2],
        8000 => &SFB_OFFSET_SHORT_V25_8000,
        _ => &SFB_OFFSET_SHORT_V1[0],
    }
}

/// Preflag additive table for long blocks (added to high-band scalefactors when
/// `preflag` is set). 22 entries (band 21 is uncoded → 0).
pub const PRETAB: [u8; 22] = [
    0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 1, 1, 1, 1, 2, 2, 3, 3, 3, 2, 0,
];

// ---- requantization (to port) ------------------------------------------------

/// `x^(4/3)` lookup for the 8207 possible Huffman magnitudes, the hot path of
/// requantization. Built once at init rather than `powf` per line.
///
/// brick: generate `i.powf(4.0/3.0)` for i in 0..8207 (lazy `OnceLock`).
pub const POW43_LEN: usize = 8207;

// ---- Huffman codebooks (to port) ---------------------------------------------

/// The 34 Layer III Huffman tables (big-value pairs) plus the 2 `count1` quad
/// tables. Each is a `(value, code, len)` set; the decoder builds a fast lookup.
///
/// brick: port the codeword tables from ISO Table B.7 and the linbits per table.
pub const HUFFMAN_TABLE_COUNT: usize = 34;

// ---- MPEG-2 / 2.5 (LSF) scalefactor scheme (ISO 13818-3 §2.4.3.4) -------------

/// Band counts per scalefactor group, indexed `[blocknumber][blocktype][group]`,
/// where blocktype is 0=long, 1=short, 2=mixed. For long blocks a group's count is
/// scalefactor bands; for short it counts individual (sfb,window) scalefactors
/// (sum = 3·sfb). ISO 13818-3 Table (derived-slen scheme).
pub const NR_OF_SFB_BLOCK: [[[u8; 4]; 3]; 6] = [
    [[6, 5, 5, 5], [9, 9, 9, 9], [6, 9, 9, 9]],
    [[6, 5, 7, 3], [9, 9, 12, 6], [6, 9, 12, 6]],
    [[11, 10, 0, 0], [18, 18, 0, 0], [15, 18, 0, 0]],
    [[7, 7, 7, 0], [12, 12, 12, 0], [6, 15, 12, 0]],
    [[6, 6, 6, 3], [12, 9, 9, 6], [6, 12, 9, 6]],
    [[8, 8, 5, 0], [15, 12, 9, 0], [6, 18, 9, 0]],
];

/// LSF scalefactor bit-lengths `slen[4]` and per-group band counts `nr[4]`, derived
/// from `scalefac_compress` and the block type. Non-intensity (left channel / no
/// intensity stereo — the right channel of an `i_stereo` pair uses a different table,
/// not emitted by this encoder yet). `blocktype`: 0=long, 1=short, 2=mixed.
#[must_use]
pub fn lsf_scale_params(scalefac_compress: u16, blocktype: usize) -> ([u8; 4], [u8; 4]) {
    let sfc = u32::from(scalefac_compress);
    let (slen, blocknumber) = if sfc < 400 {
        (
            [(sfc >> 4) / 5, (sfc >> 4) % 5, (sfc % 16) >> 2, sfc % 4],
            0,
        )
    } else if sfc < 500 {
        let s = sfc - 400;
        ([(s >> 2) / 5, (s >> 2) % 5, s % 4, 0], 1)
    } else {
        let s = sfc - 500;
        ([s / 3, s % 3, 0, 0], 2)
    };
    let nr = NR_OF_SFB_BLOCK[blocknumber][blocktype];
    (
        [slen[0] as u8, slen[1] as u8, slen[2] as u8, slen[3] as u8],
        nr,
    )
}

/// LSF scalefactor bit-lengths and group counts for the INTENSITY-coded right
/// channel (ISO 13818-3 2.4.3.2): `int_scalefac_compress = scalefac_compress >> 1`
/// picks blocknumber 3..5; the low bit is `intensity_scale`, used by the panning,
/// not here. `blocktype`: 0=long, 1=short, 2=mixed.
#[must_use]
pub fn lsf_scale_params_intensity(scalefac_compress: u16, blocktype: usize) -> ([u8; 4], [u8; 4]) {
    let isc = u32::from(scalefac_compress >> 1);
    let (slen, blocknumber) = if isc < 180 {
        ([isc / 36, (isc % 36) / 6, (isc % 36) % 6, 0], 3)
    } else if isc < 244 {
        let s = isc - 180;
        ([(s % 64) >> 4, (s % 16) >> 2, s % 4, 0], 4)
    } else {
        let s = isc - 244;
        ([s / 3, s % 3, 0, 0], 5)
    };
    (
        [slen[0] as u8, slen[1] as u8, slen[2] as u8, slen[3] as u8],
        NR_OF_SFB_BLOCK[blocknumber][blocktype],
    )
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn sfb_offsets_monotonic_and_complete() {
        for row in &SFB_OFFSET_LONG_V1 {
            assert_eq!(row[0], 0);
            assert_eq!(
                *row.last().unwrap(),
                576,
                "long bands must cover all 576 lines"
            );
            assert!(
                row.windows(2).all(|w| w[0] < w[1]),
                "long sfb strictly increasing"
            );
        }
        for row in &SFB_OFFSET_SHORT_V1 {
            assert_eq!(row[0], 0);
            assert_eq!(*row.last().unwrap(), 192, "short window spans 576/3 lines");
            assert!(
                row.windows(2).all(|w| w[0] < w[1]),
                "short sfb strictly increasing"
            );
        }
    }
}

// ---- synthesis filterbank (to port) ------------------------------------------

// The 512-tap polyphase synthesis window `D[i]` (ISO 11172-3 Table 3-B.3) is the
// canonical ISO data generated into `decode/synth_window.rs`.
