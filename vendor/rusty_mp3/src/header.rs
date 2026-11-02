//! MP3 frame header — the 32-bit sync header that prefixes every frame.
//!
//! ```text
//!  syncword(11) | version(2) | layer(2) | crc(1)
//!  bitrate_idx(4) | samplerate_idx(2) | padding(1) | private(1)
//!  channel_mode(2) | mode_ext(2) | copyright(1) | original(1) | emphasis(2)
//! ```
//!
//! From these fields we derive the frame size in bytes and the per-frame sample
//! count, which the demux/packetizer needs to walk the stream.

// Parses untrusted bytes: narrowing casts are lint-enforced here (see the crate
// lint policy in Cargo.toml) -- every one is masked, typed, or states its bound.
#![warn(
    clippy::cast_possible_truncation,
    clippy::cast_sign_loss,
    clippy::cast_possible_wrap
)]

use crate::{Error, Result};

use crate::frame::ChannelMode;
use crate::tables;

/// MPEG audio version (the sample-rate base differs per version).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum MpegVersion {
    /// MPEG-1 — 2 granules/frame, 1152 samples/frame.
    V1,
    /// MPEG-2 (LSF) — 1 granule/frame, 576 samples/frame.
    V2,
    /// MPEG-2.5 (unofficial low-sample-rate extension).
    V2_5,
}

impl MpegVersion {
    /// Granules per frame: 2 for MPEG-1, 1 for MPEG-2/2.5.
    #[must_use]
    pub fn granules(self) -> usize {
        match self {
            Self::V1 => 2,
            _ => 1,
        }
    }
    /// Decoded PCM samples per channel per frame.
    #[must_use]
    pub fn samples_per_frame(self) -> usize {
        self.granules() * crate::frame::GRANULE_LINES
    }
}

/// A parsed Layer III frame header.
#[allow(clippy::struct_excessive_bools)] // mirrors the bitstream's one-bit flags
#[derive(Debug, Clone)]
pub struct FrameHeader {
    pub version: MpegVersion,
    pub crc_protected: bool,
    pub bitrate_kbps: u32,
    pub sample_rate: u32,
    pub padding: bool,
    pub channel_mode: ChannelMode,
    pub copyright: bool,
    pub original: bool,
    pub emphasis: u8,
}

impl FrameHeader {
    /// Parse a 4-byte header. Returns `Error::invalid` on a bad sync word or a
    /// reserved/free field this decoder doesn't accept yet.
    ///
    /// # Errors
    /// [`Error::InvalidData`](crate::Error::InvalidData) on a bad sync word or a
    /// reserved version, bitrate or sample-rate index; [`Error::Unsupported`](crate::Error::Unsupported)
    /// for a layer other than III or the free-format bitrate.
    pub fn parse(bytes: [u8; 4]) -> Result<Self> {
        let h = u32::from_be_bytes(bytes);

        // 11-bit frame sync: all ones.
        if (h >> 21) & 0x7FF != 0x7FF {
            return Err(Error::invalid("mp3 header: bad frame sync"));
        }
        let version = match (h >> 19) & 0x3 {
            0b00 => MpegVersion::V2_5,
            0b10 => MpegVersion::V2,
            0b11 => MpegVersion::V1,
            _ => return Err(Error::invalid("mp3 header: reserved MPEG version")),
        };
        // Layer field: 0b01 == Layer III. This codec only does Layer III.
        if (h >> 17) & 0x3 != 0b01 {
            return Err(Error::unsupported(
                "mp3 header: only Layer III is supported",
            ));
        }
        let crc_protected = (h >> 16) & 1 == 0;

        let bitrate_index = ((h >> 12) & 0xF) as usize;
        if bitrate_index == 0 {
            return Err(Error::unsupported("mp3 header: free-format bitrate"));
        }
        if bitrate_index == 15 {
            return Err(Error::invalid("mp3 header: reserved bitrate index"));
        }
        let bitrate_kbps = match version {
            MpegVersion::V1 => tables::BITRATE_V1_L3[bitrate_index],
            _ => tables::BITRATE_V2_L3[bitrate_index],
        };

        let samplerate_index = ((h >> 10) & 0x3) as usize;
        if samplerate_index == 3 {
            return Err(Error::invalid("mp3 header: reserved sample-rate index"));
        }
        // Base rates are MPEG-1; MPEG-2 halves them and MPEG-2.5 quarters them.
        let base = tables::SAMPLE_RATE[samplerate_index];
        let sample_rate = match version {
            MpegVersion::V1 => base,
            MpegVersion::V2 => base / 2,
            MpegVersion::V2_5 => base / 4,
        };

        let padding = (h >> 9) & 1 == 1;
        let mode_ext = ((h >> 4) & 0x3) as u8;
        let channel_mode = match (h >> 6) & 0x3 {
            0b00 => ChannelMode::Stereo,
            // mode_extension bit 1 (value 2) = MS, bit 0 (value 1) = intensity.
            0b01 => ChannelMode::JointStereo {
                ms_stereo: mode_ext & 0x2 != 0,
                intensity_stereo: mode_ext & 0x1 != 0,
            },
            0b10 => ChannelMode::DualMono,
            _ => ChannelMode::Mono,
        };

        Ok(Self {
            version,
            crc_protected,
            bitrate_kbps,
            sample_rate,
            padding,
            channel_mode,
            copyright: (h >> 3) & 1 == 1,
            original: (h >> 2) & 1 == 1,
            emphasis: (h & 0x3) as u8,
        })
    }

    /// Serialize back to 4 bytes (encoder side) — the exact inverse of [`parse`](Self::parse).
    #[must_use]
    pub fn to_bytes(&self) -> [u8; 4] {
        let mut h: u32 = 0x7FF << 21; // frame sync
        let version = match self.version {
            MpegVersion::V2_5 => 0b00,
            MpegVersion::V2 => 0b10,
            MpegVersion::V1 => 0b11,
        };
        h |= version << 19;
        h |= 0b01 << 17; // Layer III
        h |= u32::from(!self.crc_protected) << 16;

        let br_table: &[u32; 16] = match self.version {
            MpegVersion::V1 => &tables::BITRATE_V1_L3,
            _ => &tables::BITRATE_V2_L3,
        };
        let br_idx = br_table
            .iter()
            .position(|&b| b == self.bitrate_kbps)
            .and_then(|i| u32::try_from(i).ok())
            .unwrap_or(0);
        h |= br_idx << 12;

        let base = match self.version {
            MpegVersion::V1 => self.sample_rate,
            MpegVersion::V2 => self.sample_rate * 2,
            MpegVersion::V2_5 => self.sample_rate * 4,
        };
        let sr_idx = tables::SAMPLE_RATE
            .iter()
            .position(|&s| s == base)
            .and_then(|i| u32::try_from(i).ok())
            .unwrap_or(0);
        h |= sr_idx << 10;
        h |= u32::from(self.padding) << 9;

        let (chan, ext) = match self.channel_mode {
            ChannelMode::Stereo => (0b00, 0),
            ChannelMode::JointStereo {
                ms_stereo,
                intensity_stereo,
            } => (
                0b01,
                u32::from(ms_stereo) << 1 | u32::from(intensity_stereo),
            ),
            ChannelMode::DualMono => (0b10, 0),
            ChannelMode::Mono => (0b11, 0),
        };
        h |= chan << 6;
        h |= ext << 4;
        h |= u32::from(self.copyright) << 3;
        h |= u32::from(self.original) << 2;
        h |= u32::from(self.emphasis);
        h.to_be_bytes()
    }

    /// Total frame size in bytes, including the header and optional CRC.
    ///
    /// `floor(samples_per_frame / 8 * bitrate / sample_rate) + padding`.
    #[must_use]
    pub fn frame_size(&self) -> usize {
        let spf = self.version.samples_per_frame();
        // `.max(1)`: the rate is validated non-zero at parse and set from a fixed
        // table on encode, but the compiler cannot know that, so the division
        // carried a divide-by-zero panic path -- in a function called for every
        // frame from the muxer, the reservoir assembler and the rate loop. A `max`
        // is a cmov; the branch and its panic block go.
        let rate = (self.sample_rate as usize).max(1);
        let bytes = (spf / 8) * (self.bitrate_kbps as usize * 1000) / rate;
        bytes + usize::from(self.padding)
    }

    /// Bytes of side information following the header (+CRC): depends on version
    /// and channel count (MPEG-1 stereo: 32, mono: 17; MPEG-2 stereo: 17, mono: 9).
    #[must_use]
    pub fn side_info_len(&self) -> usize {
        let stereo = self.channel_mode.channels() == 2;
        match (self.version, stereo) {
            (MpegVersion::V1, true) => 32,
            (MpegVersion::V1, false) | (_, true) => 17,
            (_, false) => 9,
        }
    }
}
