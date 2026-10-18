// Copyright (C) 2026 ItsAlphaNeon
// SPDX-License-Identifier: GPL-3.0-or-later
// This file is part of Lemon Studio: https://github.com/ItsAlphaNeon/Lemon-Studio
// Modified for Honeydew: vendored from Lemon Studio's `os-flp` crate (container layer only); `thiserror` derives
// replaced by plain `Display` impls, paths moved into `crate::flp`.

// spec: flp/container.md (Conformance requirements, item 3)

use super::container::Tag;
use super::event::Kind;

/// A malformed file. Every variant carries the absolute byte offset of the offending
/// chunk or event (the position of its tag / ID byte).
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ReadError {
    ChunkOverrun {
        offset: usize,
        tag: Tag,
        declared: u32,
        available: usize,
    },

    EventOverrun {
        offset: usize,
        id: u8,
        needed: u64,
        available: usize,
    },

    PrefixTruncated {
        offset: usize,
        id: u8,
    },

    PrefixOverflow {
        offset: usize,
        id: u8,
    },
}

impl ReadError {
    /// Absolute byte offset of the malformed chunk or event.
    pub fn offset(&self) -> usize {
        match *self {
            ReadError::ChunkOverrun { offset, .. }
            | ReadError::EventOverrun { offset, .. }
            | ReadError::PrefixTruncated { offset, .. }
            | ReadError::PrefixOverflow { offset, .. } => offset,
        }
    }
}

/// A model that cannot be serialized.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum WriteError {
    KindMismatch {
        tag: Tag,
        index: usize,
        id: u8,
        expected: Kind,
        got: Kind,
    },

    ChunkTooLarge {
        tag: Tag,
        len: usize,
    },
}

impl std::fmt::Display for ReadError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match *self {
            ReadError::ChunkOverrun {
                offset,
                tag,
                declared,
                available,
            } => write!(
                f,
                "chunk {tag} at offset {offset:#x}: declared length {declared} overruns the file ({available} bytes left)"
            ),
            ReadError::EventOverrun {
                offset,
                id,
                needed,
                available,
            } => write!(
                f,
                "event {id} at offset {offset:#x}: value needs {needed} bytes, only {available} left in the chunk"
            ),
            ReadError::PrefixTruncated { offset, id } => {
                write!(
                    f,
                    "event {id} at offset {offset:#x}: length prefix runs past the end of the chunk"
                )
            }
            ReadError::PrefixOverflow { offset, id } => {
                write!(
                    f,
                    "event {id} at offset {offset:#x}: length prefix does not fit in 64 bits"
                )
            }
        }
    }
}

impl std::error::Error for ReadError {}

impl std::fmt::Display for WriteError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            WriteError::KindMismatch {
                tag,
                index,
                id,
                expected,
                got,
            } => {
                write!(
                    f,
                    "event #{index} (id {id}) in chunk {tag} holds a {got} value, but id {id} is {expected}"
                )
            }
            WriteError::ChunkTooLarge { tag, len } => {
                write!(
                    f,
                    "chunk {tag} payload is {len} bytes, more than a 32-bit length can describe"
                )
            }
        }
    }
}

impl std::error::Error for WriteError {}
