//! A hosted plugin as the engine drives it: [`PluginInstrument`] (an engine instrument) and [`PluginEffect`] (an
//! external effect). The instance is shared with the app ([`super::host`]) and outlives engines; the audio thread only
//! `try_lock`s it, and a busy instance plays silence (an instrument, its notes kept for the next block) or passes the
//! sound through (an effect). Nothing here allocates or blocks.

use std::sync::Arc;
use std::sync::atomic::{AtomicBool, AtomicU32, Ordering};

use honeydew_audio::instrument::Instrument;
use honeydew_audio::{ExternalEffect, ProcessContext};
use os_engine::TransportInfo;
use os_host::SharedPlugin;

/// The plugin's params as the engine sets them: a param is sent only when its value differs from the last one sent
/// (or that the plugin reported), so an engine build's initial writes cost nothing for params that did not change.
/// Written by the app thread and the audio thread; a race costs at most one value sent twice.
pub(crate) struct ParamShared {
    sent: Box<[AtomicU32]>,
    pending: Box<[AtomicU32]>,
    dirty: Box<[AtomicBool]>,
    any: AtomicBool,
    /// The channel's controllers as last sent, channel params 4 (Pitch Bend), 5 (Aftertouch), 6 (none) and 7 + n
    /// (MIDI CC #n): a controller goes to the plugin only when it moves. They start at rest (bend centred, the rest 0),
    /// so a CC the song never moves is never sent: a plugin's own default stays (an expression CC #11 at 0 would mute
    /// it).
    controllers: [AtomicU32; CONTROLLERS],
}

/// Channel params 4..=134: Pitch Bend, Aftertouch, an unused one and the 128 MIDI CCs.
const CONTROLLERS: usize = 131;

impl ParamShared {
    /// For a plugin whose params are `values` now.
    pub(crate) fn new(values: &[f32]) -> Self {
        let atomics = |v: &[f32]| v.iter().map(|v| AtomicU32::new(v.to_bits())).collect();
        Self {
            sent: atomics(values),
            pending: atomics(values),
            dirty: values.iter().map(|_| AtomicBool::new(false)).collect(),
            any: AtomicBool::new(false),
            controllers: std::array::from_fn(|i| {
                AtomicU32::new(if i == 0 { 0.5f32 } else { 0.0 }.to_bits())
            }),
        }
    }

    /// Whether channel param `index` moved from what was last sent to `value` (and is now sent).
    fn controller_moved(&self, index: usize, value: f32) -> bool {
        let Some(sent) = index.checked_sub(4).and_then(|i| self.controllers.get(i)) else {
            return false;
        };
        sent.swap(value.to_bits(), Ordering::Relaxed) != value.to_bits()
    }

    /// Param `index` is to be `value`: queued for the next block when the plugin has another value.
    fn set(&self, index: usize, value: f32) {
        let (Some(sent), Some(pending), Some(dirty)) = (
            self.sent.get(index),
            self.pending.get(index),
            self.dirty.get(index),
        ) else {
            return;
        };
        if sent.load(Ordering::Relaxed) == value.to_bits() && !dirty.load(Ordering::Relaxed) {
            return;
        }
        pending.store(value.to_bits(), Ordering::Relaxed);
        dirty.store(true, Ordering::Release);
        self.any.store(true, Ordering::Release);
    }

    /// The plugin has `value` for param `index` (set on the app thread, or reported by its editor).
    pub(crate) fn known(&self, index: usize, value: f32) {
        if let Some(sent) = self.sent.get(index) {
            sent.store(value.to_bits(), Ordering::Relaxed);
        }
    }

    /// Hands each queued param to `send` (index, value).
    fn flush(&self, mut send: impl FnMut(u32, f64)) {
        if !self.any.swap(false, Ordering::Acquire) {
            return;
        }
        for (i, dirty) in self.dirty.iter().enumerate() {
            if dirty.swap(false, Ordering::Acquire) {
                let bits = self.pending[i].load(Ordering::Relaxed);
                self.sent[i].store(bits, Ordering::Relaxed);
                send(i as u32, f64::from(f32::from_bits(bits)));
            }
        }
    }
}

/// The song position a plugin sees for a block at `context` (a transport call while it plays).
fn transport(context: &ProcessContext, numerator: u16) -> TransportInfo {
    let tempo = f64::from(context.tempo_bpm.max(1.0));
    let beats = context.start_beat.max(0.0);
    let bar_len = f64::from(numerator.max(1));
    let bar = (beats / bar_len).floor();
    TransportInfo {
        playing: true,
        tempo,
        frame: (beats * 60.0 / tempo * f64::from(context.sample_rate)) as u64,
        beats,
        bar_start: bar * bar_len,
        bar: bar as i32,
        numerator: numerator.max(1),
        denominator: 4,
        looping: None,
    }
}

/// A note event waiting for the next block.
#[derive(Clone, Copy)]
enum Pending {
    On(u8, f32),
    Off(u8),
    Midi([u8; 3]),
}

/// Most note events a block keeps while the instance is busy; later ones are dropped.
const QUEUE: usize = 512;

/// An instrument plugin in the engine.
pub(crate) struct PluginInstrument {
    plugin: SharedPlugin,
    params: Arc<ParamShared>,
    rate: u32,
    /// The song's beats per bar (TDIV byte 1), for the plugin's transport.
    numerator: u16,
    queue: [Pending; QUEUE],
    queued: usize,
    held: [bool; 128],
    reset: bool,
    left: Vec<f32>,
    right: Vec<f32>,
}

impl PluginInstrument {
    pub(crate) fn new(
        plugin: SharedPlugin,
        params: Arc<ParamShared>,
        rate: u32,
        numerator: u16,
        max_block: usize,
    ) -> Self {
        Self {
            plugin,
            params,
            rate,
            numerator,
            queue: [Pending::Off(0); QUEUE],
            queued: 0,
            held: [false; 128],
            reset: false,
            left: vec![0.0; max_block.max(1)],
            right: vec![0.0; max_block.max(1)],
        }
    }

    fn push(&mut self, event: Pending) {
        if self.queued < QUEUE {
            self.queue[self.queued] = event;
            self.queued += 1;
        }
    }
}

impl Instrument for PluginInstrument {
    /// Param 0 is the module's Enable; `index` ≥ 1 is the plugin's param `index − 1`.
    fn set_parameter(&mut self, index: usize, value: f32) {
        if let Some(i) = index.checked_sub(1) {
            self.params.set(i, value);
        }
    }

    fn note_on(&mut self, key: u8, velocity: f32, _pan: f32) {
        let key = key.min(127);
        self.held[usize::from(key)] = true;
        self.push(Pending::On(key, velocity.clamp(0.0, 1.0)));
    }

    fn note_off(&mut self, key: u8) {
        let key = key.min(127);
        self.held[usize::from(key)] = false;
        self.push(Pending::Off(key));
    }

    fn all_notes_off(&mut self) {
        for key in 0..128u8 {
            if std::mem::take(&mut self.held[usize::from(key)]) {
                self.push(Pending::Off(key));
            }
        }
    }

    fn reset(&mut self) {
        self.queued = 0;
        self.held = [false; 128];
        self.reset = true;
    }

    fn process(&mut self, left: &mut [f32], right: &mut [f32], context: ProcessContext) {
        let Ok(mut plugin) = self.plugin.try_lock() else {
            return;
        };
        if std::mem::take(&mut self.reset) {
            plugin.reset();
        }
        self.params.flush(|i, v| plugin.param(0, i, v));
        for event in &self.queue[..self.queued] {
            match *event {
                Pending::On(key, velocity) => plugin.note_on(0, key, velocity),
                Pending::Off(key) => plugin.note_off(0, key),
                Pending::Midi(message) => plugin.midi(0, message),
            }
        }
        self.queued = 0;
        for (out_l, out_r) in left
            .chunks_mut(self.left.len())
            .zip(right.chunks_mut(self.right.len()))
        {
            if context.playing {
                plugin.transport(&transport(&context, self.numerator));
            }
            let n = out_l.len();
            let (l, r) = (&mut self.left[..n], &mut self.right[..n]);
            l.fill(0.0);
            r.fill(0.0);
            plugin.process(self.rate, l, r);
            for (o, s) in out_l.iter_mut().zip(l.iter()) {
                *o += s;
            }
            for (o, s) in out_r.iter_mut().zip(r.iter()) {
                *o += s;
            }
        }
    }

    /// Pitch Bend (4), Aftertouch (5) and MIDI CC #n (7 + n) as MIDI messages, each when it moves.
    fn set_controller(&mut self, index: usize, value: f32) {
        let value = value.clamp(0.0, 1.0);
        if !self.params.controller_moved(index, value) {
            return;
        }
        let byte = (value * 127.0).round() as u8;
        let message = match index {
            4 => {
                let bend = (value * 16383.0).round() as u16;
                [0xE0, (bend & 0x7F) as u8, (bend >> 7) as u8]
            }
            5 => [0xD0, byte, 0],
            7..=134 => [0xB0, (index - 7) as u8, byte],
            _ => return,
        };
        self.push(Pending::Midi(message));
    }
}

/// An effect plugin in the engine.
pub(crate) struct PluginEffect {
    plugin: SharedPlugin,
    params: Arc<ParamShared>,
    rate: u32,
    numerator: u16,
    reset: bool,
}

impl PluginEffect {
    pub(crate) fn new(
        plugin: SharedPlugin,
        params: Arc<ParamShared>,
        rate: u32,
        numerator: u16,
    ) -> Self {
        Self {
            plugin,
            params,
            rate,
            numerator,
            reset: false,
        }
    }
}

impl ExternalEffect for PluginEffect {
    fn set_parameter(&mut self, index: usize, value: f32) {
        if let Some(i) = index.checked_sub(1) {
            self.params.set(i, value);
        }
    }

    fn process(&mut self, left: &mut [f32], right: &mut [f32], context: ProcessContext) {
        let Ok(mut plugin) = self.plugin.try_lock() else {
            return;
        };
        if std::mem::take(&mut self.reset) {
            plugin.reset_fx();
        }
        self.params.flush(|i, v| plugin.param_fx(0, i, v));
        if context.playing {
            plugin.transport_fx(&transport(&context, self.numerator));
        }
        plugin.process_fx(self.rate, left, right);
    }

    fn reset(&mut self) {
        self.reset = true;
    }
}
