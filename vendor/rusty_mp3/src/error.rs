//! Dependency-free error type for the MP3 codec.
//!
//! The [`Error::Again`] / [`Error::Eof`] pair mirrors FFmpeg's
//! `EAGAIN`/`EOF` codec-drain protocol: a pull call ([`crate::Mp3Decoder::next_frame`],
//! [`crate::Mp3Encoder::next_packet`]) returns `Err(Again)` when more input is
//! needed before output can be produced, and `Err(Eof)` once the stream has been
//! flushed ([`crate::Mp3Decoder::flush`] / [`crate::Mp3Encoder::finish`]) and
//! fully drained. Neither is a failure — they drive the push/pull loop.

use std::fmt;

/// Crate-wide result alias.
pub type Result<T> = std::result::Result<T, Error>;

/// All the ways MP3 encode/decode can fail (or signal drain-loop state).
#[non_exhaustive]
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Error {
    /// A code path that is scaffolded but not yet implemented. Carries a short
    /// static label so logs point straight at the missing piece.
    Unimplemented(&'static str),

    /// End of stream — the codec has been flushed and has no more output.
    Eof,

    /// More input is required before output can be produced (codec drain/fill).
    Again,

    /// The input bytes were malformed for the expected format/codec.
    InvalidData(String),

    /// A requested capability exists in concept but isn't supported here.
    Unsupported(String),
}

impl Error {
    /// Convenience constructor for `InvalidData`.
    pub fn invalid(msg: impl Into<String>) -> Self {
        Self::InvalidData(msg.into())
    }

    /// Convenience constructor for `Unsupported`.
    pub fn unsupported(msg: impl Into<String>) -> Self {
        Self::Unsupported(msg.into())
    }
}

impl fmt::Display for Error {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Unimplemented(what) => write!(f, "not yet implemented: {what}"),
            Self::Eof => write!(f, "end of stream"),
            Self::Again => write!(f, "more input required"),
            Self::InvalidData(msg) => write!(f, "invalid data: {msg}"),
            Self::Unsupported(msg) => write!(f, "unsupported: {msg}"),
        }
    }
}

impl std::error::Error for Error {}
