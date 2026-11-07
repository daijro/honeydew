// Copyright (C) 2026 The Honeydew contributors
// SPDX-License-Identifier: GPL-3.0-or-later

//! Rind: Honeydew's own, Sylenth1-inspired subtractive synthesizer, built into the app as a CLAP instrument (on every
//! platform, like Lemon Studio's plugins: `honeydew::plugins::builtin`). It has no VST3 or standalone form; its editor
//! draws into the rack card (`os_plugkit::embed`).
//!
//! **Milestone 1 (editor spike).** A registered instrument that plays nothing yet, with a minimal editor to prove the
//! embedded path: painting, pointer and touch input, reset gestures and parking. The DSP and the permanent parameter
//! ids come in later milestones.
#![allow(semicolon_in_expressions_from_non_local_macros)]

mod editor;
pub mod params;
pub mod view;

use std::num::NonZeroU32;
use std::sync::Arc;

use nih_plug::prelude::*;

use params::RindParams;

/// The plugin's permanent CLAP id: songs know Rind by it.
pub const CLAP_ID: &str = "org.honeydew.rind";

pub struct Rind {
    params: Arc<RindParams>,
}

impl Default for Rind {
    fn default() -> Self {
        Rind {
            params: Arc::new(RindParams::default()),
        }
    }
}

impl Plugin for Rind {
    const NAME: &'static str = "Rind";
    const VENDOR: &'static str = "Honeydew";
    const URL: &'static str = "";
    const EMAIL: &'static str = "";
    const VERSION: &'static str = env!("CARGO_PKG_VERSION");

    const AUDIO_IO_LAYOUTS: &'static [AudioIOLayout] = &[AudioIOLayout {
        main_input_channels: None,
        main_output_channels: NonZeroU32::new(2),
        ..AudioIOLayout::const_default()
    }];
    const MIDI_INPUT: MidiConfig = MidiConfig::MidiCCs;
    const SAMPLE_ACCURATE_AUTOMATION: bool = false;

    type SysExMessage = ();
    type BackgroundTask = ();

    fn params(&self) -> Arc<dyn Params> {
        self.params.clone()
    }

    fn editor(&mut self, _async_executor: AsyncExecutor<Self>) -> Option<Box<dyn Editor>> {
        Some(Box::new(editor::RindEditor::new(self.params.clone())))
    }

    /// The spike plays nothing: silence, with no allocation.
    fn process(
        &mut self,
        buffer: &mut Buffer,
        _aux: &mut AuxiliaryBuffers,
        _ctx: &mut impl ProcessContext<Self>,
    ) -> ProcessStatus {
        for channel in buffer.as_slice().iter_mut() {
            channel.fill(0.0);
        }
        ProcessStatus::Normal
    }
}

impl ClapPlugin for Rind {
    const CLAP_ID: &'static str = CLAP_ID;
    const CLAP_DESCRIPTION: Option<&'static str> = Some("Honeydew's subtractive synthesizer");
    const CLAP_MANUAL_URL: Option<&'static str> = None;
    const CLAP_SUPPORT_URL: Option<&'static str> = None;
    const CLAP_FEATURES: &'static [ClapFeature] = &[
        ClapFeature::Instrument,
        ClapFeature::Synthesizer,
        ClapFeature::Stereo,
    ];
}

// Built as a plugin module: the C entry points hosts look for. Linked into Honeydew (`linked` feature): only the
// symbol-free table, so Rind fits in one program with the other built-in plugins.
#[cfg(not(feature = "linked"))]
nih_export_clap!(Rind);
os_plugkit::linked_clap_entry!(Rind);
