// Copyright (C) 2026 The Honeydew contributors
// SPDX-License-Identifier: GPL-3.0-or-later

//! Rind's parameters.
//!
//! **Milestone 1 (editor spike).** These three parameters only exist to drive the editor through every path: a
//! `FloatParam` knob, a skewed `FloatParam` fader and an `IntParam` choice. They are throwaway: the permanent,
//! append-only id table replaces them (`ids.rs`, Milestone 2) before Rind is released, so none of these ids may ever be
//! saved in a user's project.

use std::sync::Arc;

use nih_plug::prelude::*;
use os_plugkit::live::LiveEdits;

/// The spike knob's id.
pub const KNOB: &str = "spike_knob";
/// The spike fader's id.
pub const FADER: &str = "spike_fader";
/// The spike choice's id.
pub const CHOICE: &str = "spike_choice";

/// The choice's last value: it picks one of `0..=CHOICE_MAX`.
pub const CHOICE_MAX: i32 = 3;

#[derive(Params)]
pub struct RindParams {
    #[id = "spike_knob"]
    pub knob: FloatParam,
    #[id = "spike_fader"]
    pub fader: FloatParam,
    #[id = "spike_choice"]
    pub choice: IntParam,
    /// The editor's edits the host has not handed back yet. The sound reads every parameter through it
    /// ([`LiveEdits::value`]), so an edit sounds at once.
    pub live: Arc<LiveEdits>,
}

impl Default for RindParams {
    fn default() -> Self {
        RindParams {
            knob: FloatParam::new("Spike Knob", 0.5, FloatRange::Linear { min: 0.0, max: 1.0 }),
            fader: FloatParam::new(
                "Spike Fader",
                1.0,
                FloatRange::Skewed {
                    min: 0.0,
                    max: 1.0,
                    factor: FloatRange::skew_factor(-1.0),
                },
            ),
            choice: IntParam::new(
                "Spike Choice",
                0,
                IntRange::Linear {
                    min: 0,
                    max: CHOICE_MAX,
                },
            ),
            live: Arc::new(LiveEdits::default()),
        }
    }
}
