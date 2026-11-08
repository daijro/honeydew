// Copyright (C) 2026 The Honeydew contributors
// SPDX-License-Identifier: GPL-3.0-or-later

//! Rind: Honeydew's own subtractive synthesizer, built into the app as a CLAP instrument (on every platform, like Lemon
//! Studio's plugins: `honeydew::plugins::builtin`). It has no VST3 or standalone form; its editor draws into the rack
//! card (`os_plugkit::embed`).
//!
//! Four band-limited oscillators (two parts of two, up to eight unison voices each), two multimode filters, four
//! envelopes, two LFOs, a sixteen-slot modulation matrix, an arpeggiator with a sixteen-step pattern and seven master
//! effects, in sixteen preallocated voices.
#![allow(semicolon_in_expressions_from_non_local_macros)]
#![allow(
    clippy::needless_range_loop,
    clippy::too_many_arguments,
    clippy::type_complexity,
    clippy::manual_range_contains,
    clippy::excessive_precision,
    clippy::approx_constant,
    clippy::collapsible_else_if
)]

pub mod arp;
mod editor;
pub mod engine;
pub mod env;
pub mod filter;
pub mod fx;
pub mod ids;
pub mod osc;
pub mod params;
pub mod ranges;
pub mod util;
pub mod view;
pub mod voice;

use std::num::NonZeroU32;
use std::sync::Arc;

use nih_plug::prelude::*;

use engine::Engine;
use params::{RindParams, Snapshot};

/// The plugin's permanent CLAP id: songs know Rind by it.
pub const CLAP_ID: &str = "org.honeydew.rind";

pub struct Rind {
    params: Arc<RindParams>,
    engine: Box<Engine>,
    snap: Box<Snapshot>,
}

impl Default for Rind {
    fn default() -> Self {
        Rind {
            params: Arc::new(RindParams::default()),
            engine: Box::new(Engine::new(44_100.0)),
            snap: Box::new(RindParams::defaults()),
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

    fn initialize(
        &mut self,
        _layout: &AudioIOLayout,
        config: &BufferConfig,
        _ctx: &mut impl InitContext<Self>,
    ) -> bool {
        // Everything the sound needs is allocated here, never in `process`.
        self.engine = Box::new(Engine::new(config.sample_rate));
        true
    }

    fn reset(&mut self) {
        self.engine.reset();
    }

    fn process(
        &mut self,
        buffer: &mut Buffer,
        _aux: &mut AuxiliaryBuffers,
        ctx: &mut impl ProcessContext<Self>,
    ) -> ProcessStatus {
        let n = buffer.samples();
        let tempo = ctx.transport().tempo.unwrap_or(120.0) as f32;
        self.params.snapshot(&mut self.snap);
        let channels = buffer.as_slice();
        if channels.len() < 2 {
            return ProcessStatus::Normal;
        }
        let (a, b) = channels.split_at_mut(1);
        let (l, r) = (&mut *a[0], &mut *b[0]);

        let mut next = ctx.next_event();
        let mut pos = 0;
        while pos < n {
            while let Some(ev) = next {
                if ev.timing() as usize > pos {
                    break;
                }
                match ev {
                    NoteEvent::NoteOn { note, velocity, .. } => self.engine.note_on(note, velocity),
                    NoteEvent::NoteOff { note, .. } => self.engine.note_off(note),
                    NoteEvent::MidiPitchBend { value, .. } => self.engine.pitch_bend(value),
                    NoteEvent::MidiCC { cc, value, .. } => self.engine.cc(cc, value),
                    NoteEvent::MidiChannelPressure { pressure, .. } => {
                        self.engine.pressure(pressure)
                    }
                    _ => {}
                }
                next = ctx.next_event();
            }
            let end = next
                .map_or(n, |e| (e.timing() as usize).min(n))
                .max(pos + 1);
            self.engine
                .render(&self.snap, tempo, &mut l[pos..end], &mut r[pos..end]);
            pos = end;
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
