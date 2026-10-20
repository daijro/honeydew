// Copyright (C) 2026 ItsAlphaNeon
// SPDX-License-Identifier: GPL-3.0-or-later
// This file is part of Lemon Studio: https://github.com/ItsAlphaNeon/Lemon-Studio

//! Honeydew's vendored subset of Lemon Studio's audio engine: what the plugin hosts (`os-host`, `os-clap`,
//! `os-vst3`, `os-vst2`) use from it, so that they build against Honeydew's own engine, and the mixer effects Honeydew
//! runs as rack modules.
//!
//! - [`instrument`]: the traits a hosted plugin implements for the engine (upstream `instrument.rs`).
//! - [`voices`]: a hosted instrument's voices as the host counts them (upstream `voices.rs`, without its tests).
//! - [`effects`]: the reverb, the limiter and their siblings (upstream `effects/`, unmodified).
//!
//! With the `plugins` feature, also what Lemon Studio's own plugins (Lemon Trio, Lemon Sampler, Lemon Parametric EQ)
//! use from it, upstream's files unmodified: [`clock`], [`envelope`], [`laws`] and [`sample`].

#[cfg(feature = "plugins")]
pub mod clock;
pub mod effects;
#[cfg(feature = "plugins")]
pub mod envelope;
pub mod instrument;
#[cfg(feature = "plugins")]
pub mod laws;
#[cfg(feature = "plugins")]
pub mod sample;
pub mod voices;

#[cfg(feature = "plugins")]
pub use clock::Clock;
pub use instrument::{ChannelLevels, ChannelVoicing, HostedEffect, Instrument, NoteLevels, SharedEffect, SharedInstrument, TransportInfo};
#[cfg(feature = "plugins")]
pub use sample::{Sample, SampleLibrary};
