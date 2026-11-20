// Copyright (C) 2026 ItsAlphaNeon
// SPDX-License-Identifier: GPL-3.0-or-later
// This file is part of Lemon Studio: https://github.com/ItsAlphaNeon/Lemon-Studio

//! BanaForge's vendored subset of Banana Studio's audio engine: what the plugin hosts (`os-host`, `os-clap`,
//! `os-vst3`, `os-vst2`) use from it, so that they build against BanaForge's own engine, and the mixer effects BanaForge
//! runs as rack modules.
//!
//! - [`instrument`]: the traits a hosted plugin implements for the engine (upstream `instrument.rs`).
//! - [`voices`]: a hosted instrument's voices as the host counts them (upstream `voices.rs`, without its tests).
//! - [`effects`]: the reverb, the limiter and their siblings (upstream `effects/`, unmodified), and [`formula`], the
//!   formula language Banana Formula Controller's effect reads (upstream `formula.rs`, unmodified).
//!
//! With the `plugins` feature, also what Banana Studio's own plugins (Banana Trio, Banana Sampler, Banana Parametric EQ)
//! use from it, upstream's files unmodified: [`clock`], [`envelope`], [`laws`] and [`sample`].

#[cfg(feature = "plugins")]
pub mod clock;
pub mod effects;
#[cfg(feature = "plugins")]
pub mod envelope;
pub mod formula;
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
