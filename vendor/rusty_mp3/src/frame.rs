//! Shared MP3 types: channel layout, block types, side-information, and the
//! fixed structural constants. These are read by the decoder and written by the
//! encoder, so they live in one place.
//!
//! Layer III packs **two granules** per frame (MPEG-1) or **one** (MPEG-2/2.5),
//! each carrying up to **576** frequency lines per channel; a frame therefore
//! yields 1152 (MPEG-1) or 576 (MPEG-2) PCM samples per channel.

/// Frequency lines per granule per channel.
pub const GRANULE_LINES: usize = 576;
/// Subbands in the polyphase filterbank.
pub const SUBBANDS: usize = 32;
/// Frequency lines per subband (`GRANULE_LINES / SUBBANDS`).
pub const SUBBAND_LINES: usize = 18;
/// Number of scalefactor bands for long blocks.
pub const SFB_LONG: usize = 22;
/// Number of scalefactor bands for short blocks (per window: 13; ×3 windows).
pub const SFB_SHORT: usize = 13;

/// Stereo/mono layout from the header `mode` field.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum ChannelMode {
    Stereo,
    /// Joint stereo; `mode_extension` selects MS / intensity on/off.
    JointStereo {
        ms_stereo: bool,
        intensity_stereo: bool,
    },
    DualMono,
    #[default]
    Mono,
}

impl ChannelMode {
    /// Number of coded channels (mono → 1, everything else → 2).
    #[must_use]
    pub fn channels(self) -> usize {
        match self {
            Self::Mono => 1,
            _ => 2,
        }
    }
}

/// The four window sequences. Long blocks use one 18-point IMDCT per subband;
/// short blocks use three 6-point IMDCTs (better time resolution for transients).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum BlockType {
    #[default]
    Long,
    Start,
    Short,
    Stop,
}

/// Per-granule, per-channel side information — the recipe for reconstructing one
/// granule's spectrum from the Huffman-coded main data.
#[allow(clippy::struct_excessive_bools)] // mirrors the bitstream's one-bit flags
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct GranuleSideInfo {
    /// Bits of main data for this granule/channel (scalefactors + Huffman).
    pub part2_3_length: u16,
    /// Half the number of big-value pairs in the Huffman region.
    pub big_values: u16,
    /// Global quantizer gain (the `2^(gain/4)` requantization step).
    pub global_gain: u8,
    /// Selects how the scalefactor bit-lengths split (slen1/slen2).
    pub scalefac_compress: u16,
    /// True when `block_type != Long` (window switching active).
    pub window_switching: bool,
    pub block_type: BlockType,
    /// For START/STOP/SHORT: whether the lowest two subbands stay long-block mixed.
    pub mixed_block: bool,
    /// Huffman table select per big-value region (2 regions for short blocks,
    /// 3 for long blocks).
    pub table_select: [u8; 3],
    /// Per-window gain offset for short blocks.
    pub subblock_gain: [u8; 3],
    /// Region boundaries (long blocks): big-values split into 3 regions.
    pub region0_count: u8,
    pub region1_count: u8,
    /// Adds 1 to high-band scalefactors (long blocks).
    pub preflag: bool,
    /// Scalefactor multiplier: `0 => 0.5`, `1 => 1.0` step in the requantizer.
    pub scalefac_scale: bool,
    /// Selects the Huffman table for the `count1` (quad) region.
    pub count1table_select: bool,
}

/// All side information for one frame (both granules, both channels).
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct SideInfo {
    /// Back-pointer into the bit reservoir: where this frame's main data begins,
    /// counted in bytes *before* the current frame's main-data boundary.
    pub main_data_begin: u16,
    /// Private bits (unused payload).
    pub private_bits: u8,
    /// Scalefactor selection info: per channel, per scalefactor-band group,
    /// whether granule 1 reuses granule 0's scalefactors.
    pub scfsi: [[bool; 4]; 2],
    /// `[granule][channel]`.
    pub granules: [[GranuleSideInfo; 2]; 2],
}

/// One granule's decoded, requantized spectrum (`576` lines per channel) plus the
/// overlap state needed by the synthesis stage. The reusable working buffer the
/// decode pipeline threads through requantize → stereo → antialias → imdct.
#[derive(Debug, Clone)]
pub struct GranuleSpectrum {
    /// `[channel][line]` dequantized frequency-domain samples.
    pub lines: [[f32; GRANULE_LINES]; 2],
    /// Non-zero line count per channel (the rzero boundary).
    pub nonzero: [usize; 2],
}

impl Default for GranuleSpectrum {
    fn default() -> Self {
        Self {
            lines: [[0.0; GRANULE_LINES]; 2],
            nonzero: [0; 2],
        }
    }
}
