// Copyright (C) 2026 ItsAlphaNeon
// SPDX-License-Identifier: GPL-3.0-or-later
// This file is part of Lemon Studio: https://github.com/ItsAlphaNeon/Lemon-Studio

//! Honeydew's vendored subset of Lemon Studio's audio engine: only what the plugin hosts (`os-host`, `os-clap`,
//! `os-vst3`, `os-vst2`) use from it, so that they build against Honeydew's own engine.
//!
//! - [`instrument`]: the traits a hosted plugin implements for the engine (upstream `instrument.rs`).
//! - [`voices`]: a hosted instrument's voices as the host counts them (upstream `voices.rs`, without its tests).

pub mod instrument;
pub mod voices;

pub use instrument::{ChannelLevels, ChannelVoicing, HostedEffect, Instrument, NoteLevels, SharedEffect, SharedInstrument, TransportInfo};
