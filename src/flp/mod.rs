//! FL Studio desktop projects (`.flp`). The event container ([`container`], [`event`], [`check`], [`error`]) is
//! vendored from Lemon Studio's `os-flp` (GPL-3.0-or-later, credited in `THIRD_PARTY_NOTICES.md`); the mapping both
//! ways is Honeydew's own: [`import`] reads a project into a [`crate::song::Song`], [`export`] writes one.
//!
//! The event ids are the public FLP event list (as PyFLP and Lemon Studio document it): an id's size class is its
//! range (0–63 a byte, 64–127 a word, 128–191 a dword, 192–255 a varint length and data), so an unknown event is
//! always skipped whole.

pub mod check;
pub mod container;
pub mod error;
pub mod event;
pub mod export;
pub mod import;
pub mod mixer;
pub mod wrapper;

pub use check::check_alignment;
pub use container::{Chunk, ChunkBody, FlpFile, Header, Tag};
pub use event::{Event, Value};

/// The file type.
pub const FLP_EXT: &str = "flp";

/// The event ids Honeydew reads or writes.
pub(crate) mod id {
    pub const CH_ENABLED: u8 = 0;
    /// Before FL 4: the channel's volume and pan bytes.
    pub const CH_VOL_BYTE: u8 = 2;
    pub const CH_PAN_BYTE: u8 = 3;
    pub const CH_TYPE: u8 = 21;
    /// Before the word event 104: the channel's mixer insert, signed.
    pub const CH_TARGET_BYTE: u8 = 22;
    pub const CH_LOCKED: u8 = 32;
    pub const PL_COLOR_FLAG: u8 = 43;
    pub const CH_COLOR_FLAG: u8 = 41;
    pub const TIME_NUM: u8 = 17;
    pub const TIME_DEN: u8 = 18;
    pub const ARR_UNKNOWN: u8 = 36;
    pub const CH_END: u8 = 51;
    pub const PAT_META_END: u8 = 52;
    pub const CH_NEW: u8 = 64;
    pub const PAT_NEW: u8 = 65;
    /// Before FL 3.4: the tempo in whole BPM; 93 adds thousandths.
    pub const TEMPO_COARSE: u8 = 66;
    /// Before the Levels blob: the channel's volume and pan words.
    pub const CH_VOL_WORD: u8 = 72;
    pub const CH_PAN_WORD: u8 = 73;
    pub const TEMPO_FINE: u8 = 93;
    pub const MX_SLOT: u8 = 98;
    pub const ARR_NEW: u8 = 99;
    pub const ARR_CURRENT: u8 = 100;
    pub const CH_TARGET: u8 = 104;
    pub const CH_COLOR: u8 = 128;
    pub const CH_ROOT: u8 = 135;
    pub const CH_GROUP: u8 = 145;
    pub const MX_OUTPUT: u8 = 147;
    pub const MX_COLOR: u8 = 149;
    pub const PAT_COLOR: u8 = 150;
    pub const MX_INPUT: u8 = 154;
    pub const PLUGIN_ICON: u8 = 155;
    pub const TEMPO: u8 = 156;
    pub const BUILD: u8 = 159;
    pub const PAT_LENGTH: u8 = 164;
    pub const PLAYLIST_FIX: u8 = 169;
    pub const PAT_171: u8 = 171;
    /// Before 203: the channel's name.
    pub const CH_NAME_OLD: u8 = 192;
    pub const PAT_NAME: u8 = 193;
    pub const TITLE: u8 = 194;
    pub const SAMPLE_PATH: u8 = 196;
    pub const VERSION: u8 = 199;
    pub const PLUGIN_NAME: u8 = 201;
    pub const DISPLAY_NAME: u8 = 203;
    pub const MX_NAME: u8 = 204;
    pub const PLUGIN_REC: u8 = 212;
    pub const PLUGIN_STATE: u8 = 213;
    pub const INIT_CONTROLS: u8 = 216;
    pub const CH_LEVELS: u8 = 219;
    pub const PAT_NOTES: u8 = 224;
    pub const MX_PARAMS: u8 = 225;
    pub const AUTO_LINK: u8 = 227;
    pub const PLAYLIST: u8 = 233;
    pub const AUTO_DATA: u8 = 234;
    pub const MX_ROUTES: u8 = 235;
    pub const MX_FLAGS: u8 = 236;
    pub const WORK_TIME: u8 = 237;
    pub const PL_TRACK: u8 = 238;
    pub const PL_NAME: u8 = 239;
    pub const ARR_NAME: u8 = 241;
    pub const PAT_252: u8 = 252;
    pub const PAT_253: u8 = 253;
    /// A Layer channel's child, by its channel number; one event each.
    pub const CH_LAYER_CHILD: u8 = 94;
    /// A channel's sample and Misc settings; ints at 68 and 72 are its key zone (0 and 256: none).
    pub const CH_SAMPLE_SETTINGS: u8 = 215;
}

#[cfg(test)]
pub(crate) mod tests;
