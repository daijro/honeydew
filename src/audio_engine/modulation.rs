//! The Modulators' targets (Honeydew's own, `honeydew_audio::modulator`): each Modulator moves one param of the song,
//! its target ([`crate::song::Modulation`]), to its base plus Amount × its signal, held to 0…1. The base is the value the
//! automation or a control last set (`applied`), so they move the base and never the modulated value. The offsets of
//! several Modulators on one target add up.
//!
//! **Ruling (one block late).** A segment's offsets are set at the start of the next one, before any rack runs: the
//! racks may run in parallel, and none reads another's state while they do.
//!
//! The targets are modulated only while the transport rolls; stopped (and as an edit's engine takes over, which may
//! have removed the Modulator), each is set back to its base exactly. Nothing here allocates once built.
//!
//! The engine reports each target's value as the last segment set it, NaN while stopped ([`Modulations::shown`]), so
//! that its controls show it.
//!
//! **Controller links** (0.4.9). A plugin effect may be a source too: a
//! controller (Lemon Peak Controller, Lemon Formula Controller) whose output 0 ([`AudioEffect::control`]) its target
//! takes as its value, in place of its base, smoothed over [`SMOOTH_SECS`]; Modulators' offsets add to it. Saved as a
//! Modulator's target is (`HDMd`).

use honeydew_audio::AudioEffect;
use honeydew_audio::device::{Modulated, ModuleKey};
use honeydew_audio::modulator::MODULATOR;

use super::{SongAudioEngine, position, strip_param};
use crate::song::plugin::PLUGIN_EFFECT;
use crate::song::{AutomationTarget, Song};

/// A controller link's smoothing time: a one-pole that covers 63 % of a step in it.
const SMOOTH_SECS: f32 = 0.01;

/// A Modulator of the song wired to its target: its channel index and instance id, and its target's channel index and
/// target ([`wired`]).
pub(super) struct Wire {
    pub source: (usize, i32),
    pub channel: usize,
    pub target: AutomationTarget,
}

/// Each Modulator (or plugin effect: a controller) on its channel's rack whose target names a channel of the song.
/// Whether the param exists is the engine's `initial` business ([`Modulations::new`]).
pub(super) fn wired(song: &Song) -> Vec<Wire> {
    let mut wires = Vec::new();
    for (c, ch) in song.channels.iter().enumerate() {
        for m in &ch.file.modulations {
            let modulator = ch.rack.get(m.modulator);
            let channel = song.channels.iter().position(|t| t.id == m.channel);
            if let (Some(_), Some(channel)) = (
                modulator.filter(|x| matches!(x.type_id, MODULATOR | PLUGIN_EFFECT)),
                channel,
            ) {
                wires.push(Wire {
                    source: (c, m.modulator),
                    channel,
                    target: m.target,
                });
            }
        }
    }
    wires
}

#[derive(Clone, Copy)]
struct Target {
    /// The param, as `initial` keys it: channel, instance id, index.
    param: (usize, i32, i32),
    /// Its place in `initial` (and `applied`).
    at: usize,
    /// Its Modulators' offsets, summed.
    sum: f32,
    /// A controller's smoothed output, its value in place of the base.
    driven: Option<f32>,
    /// The value last set here; NaN once the base was set since.
    sent: f32,
}

/// The engine's modulated params and their Modulators.
#[derive(Default)]
pub(super) struct Modulations {
    /// Each Modulator: its channel, its instance id, its target's index in `targets` and, for a controller, its
    /// smoothed output (NaN until it has one).
    sources: Vec<(usize, i32, usize, f32)>,
    targets: Vec<Target>,
    /// By target, its value as the last segment set it; NaN while not modulated (the engine's
    /// `AudioRenderer::modulated`).
    pub(super) shown: Vec<Modulated>,
    /// The last segment modulated the targets: they may be off their bases.
    on: bool,
}

impl Modulations {
    /// `wires`' Modulators that run and targets that `initial` holds.
    pub(super) fn new(
        wires: &[Wire],
        initial: &[(usize, i32, i32, f32)],
        runs: impl Fn(usize, i32) -> bool,
    ) -> Self {
        let mut this = Self::default();
        for wire in wires.iter().filter(|w| runs(w.source.0, w.source.1)) {
            let (instance_id, param) = (wire.target.instance_id, wire.target.param);
            let param = (wire.channel, instance_id, strip_param(instance_id, param));
            let Some(at) = position(initial, param) else {
                continue;
            };
            let t = match this.targets.iter().position(|t| t.at == at) {
                Some(t) => t,
                None => {
                    this.targets.push(Target {
                        param,
                        at,
                        sum: 0.0,
                        driven: None,
                        sent: f32::NAN,
                    });
                    this.shown.push(Modulated {
                        key: ModuleKey {
                            channel: param.0,
                            instance_id: param.1,
                        },
                        index: param.2,
                        value: f32::NAN,
                    });
                    this.targets.len() - 1
                }
            };
            this.sources
                .push((wire.source.0, wire.source.1, t, f32::NAN));
        }
        this
    }

    /// The base at `initial` index `at` was just set: its target, if it is one, takes its offset again from it.
    pub(super) fn based(&mut self, at: usize) {
        for t in self.targets.iter_mut().filter(|t| t.at == at) {
            t.sent = f32::NAN;
        }
    }

    /// `initial` index `at` is a target.
    pub(super) fn holds(&self, at: usize) -> bool {
        self.targets.iter().any(|t| t.at == at)
    }
}

impl SongAudioEngine {
    /// The start of a segment of `frames`, before any rack runs: with `on`, each target at its base (or its
    /// controller's output, smoothed) plus its Modulators' offsets as their last segment left them; else, once, at its
    /// base. Sets only what changed.
    pub(super) fn modulate(&mut self, on: bool, frames: usize) {
        if !on && !self.modulation.on {
            return;
        }
        self.modulation.on = on;
        for t in &mut self.modulation.targets {
            t.sum = 0.0;
            t.driven = None;
        }
        let step = 1.0 - (-(frames as f32) / (SMOOTH_SECS * self.sample_rate)).exp();
        for source in &mut self.modulation.sources {
            let (channel, id, t, held) = source;
            let effect = self.audio.effect(*channel, *id);
            match effect.and_then(AudioEffect::control).filter(|_| on) {
                Some(v) => {
                    *held = if held.is_nan() {
                        v
                    } else {
                        *held + (v - *held) * step
                    };
                    self.modulation.targets[*t].driven = Some(*held);
                }
                None if on => {
                    self.modulation.targets[*t].sum += effect.map_or(0.0, AudioEffect::modulation)
                }
                // Stopped: the next start takes the output as it is.
                None => *held = f32::NAN,
            }
        }
        for k in 0..self.modulation.targets.len() {
            let t = self.modulation.targets[k];
            let value = (t.driven.unwrap_or(self.applied[t.at]) + t.sum).clamp(0.0, 1.0);
            self.modulation.shown[k].value = if on { value } else { f32::NAN };
            if value.is_finite() && value != t.sent {
                self.modulation.targets[k].sent = value;
                let (channel, instance_id, index) = t.param;
                self.send_param(channel, instance_id, index, value);
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use honeydew_audio::device::AudioRenderer;
    use honeydew_audio::modulator as m;

    use super::*;
    use crate::song::{ChannelKind, Modulation, SynthKind};

    const RATE: f32 = 48_000.0;
    /// B's Pan: its base is the centre.
    const PAN: AutomationTarget = AutomationTarget {
        instance_id: 0,
        param: 1,
    };
    /// The Cutoff of B's Filter (instance 3).
    const CUTOFF: AutomationTarget = AutomationTarget {
        instance_id: 3,
        param: 1,
    };

    /// Master, B (channel 1: a MiniSynth, a Filter and a Modulator set by `params` driving `target` on B) and A (channel
    /// 2, a MiniSynth); with `keyed`, A's output keys the Modulator.
    fn song(params: &[(usize, f32)], keyed: bool, target: AutomationTarget) -> Song {
        let mut song = Song::default();
        song.add_channel(ChannelKind::Synth(SynthKind::MiniSynth));
        song.add_channel(ChannelKind::Synth(SynthKind::MiniSynth));
        let (a, b) = (song.channels[2].id, song.channels[1].id);
        let ch = &mut song.channels[1];
        assert_eq!(ch.rack.add(9), CUTOFF.instance_id);
        let id = ch.rack.add(MODULATOR);
        let module = ch.rack.get_mut(id).unwrap();
        for &(p, v) in params {
            module.params[p] = v;
        }
        module.sidechain = keyed.then_some(a);
        ch.file.modulations.push(Modulation {
            modulator: id,
            channel: b,
            target,
        });
        song
    }

    /// B's `target` as the engine holds it.
    fn value(engine: &SongAudioEngine, target: AutomationTarget) -> f32 {
        let (id, p) = (target.instance_id, target.param as usize);
        match id {
            0 => engine.audio.channel_parameter(1, p),
            _ => engine.audio.effect(1, id).and_then(|e| e.parameter(p)),
        }
        .unwrap()
    }

    /// `frames` rendered in blocks of 256, B's `target` after each.
    fn values(engine: &mut SongAudioEngine, frames: usize, target: AutomationTarget) -> Vec<f32> {
        let mut output = vec![0.0; 256 * 2];
        (0..frames / 256)
            .map(|_| {
                engine.render(&mut output, 2);
                value(engine, target)
            })
            .collect()
    }

    #[test]
    fn an_envelope_keyed_by_a_burst_lifts_its_target_and_lets_it_back_to_its_base() {
        // An Envelope at +40 %, +12 dB, 1 ms attack and 10 ms release.
        let params = [
            (m::AMOUNT, 0.7),
            (m::GAIN, 0.75),
            (m::ATTACK, 0.0),
            (m::RELEASE, 0.0),
        ];
        let mut engine = SongAudioEngine::new(&song(&params, true, PAN), RATE).unwrap();
        let base = value(&engine, PAN);
        engine.play();
        let silent = values(&mut engine, 4096, PAN);
        assert!(silent.iter().all(|&p| p == base), "A is silent: {silent:?}");
        engine.note_on(2, 57, 1.0);
        let burst = values(&mut engine, 8192, PAN);
        assert!(burst.iter().any(|&p| p > base + 0.3), "{burst:?}");
        assert!(burst.iter().all(|&p| p <= base + 0.4 + 1e-6));
        engine.note_off(2, 57);
        // The MiniSynth's release, then the Envelope's.
        let after = values(&mut engine, 4 * 48_000, PAN);
        assert_eq!(after.last(), Some(&base));
    }

    #[test]
    fn an_lfo_moves_its_target_sinusoidally_at_its_rate() {
        // A free 1 Hz sine at −50 %: Pan = 0.5 − 0.5·sin(2π t).
        let params = [(m::SOURCE, 0.5), (m::AMOUNT, 0.25)];
        let mut engine = SongAudioEngine::new(&song(&params, false, PAN), RATE).unwrap();
        engine.play();
        for (k, p) in values(&mut engine, 48_000, PAN).into_iter().enumerate() {
            // Set at the start of block k, from the signal at the end of block k − 1 (frame 256·k).
            let t = (256 * k) as f32 / RATE;
            let expected = 0.5 - 0.5 * (std::f32::consts::TAU * t).sin();
            assert!((p - expected).abs() < 1e-3, "block {k}: {p} vs {expected}");
        }
    }

    /// A Lemon Peak Controller after B's MiniSynth, linked to B's Volume, sets it to its output
    /// (the peak it follows), smoothed: down near 0 while B is quiet, up while a note plays. The link is saved.
    #[test]
    fn a_peak_controller_fed_a_burst_drives_its_linked_volume_and_the_link_is_saved() {
        const VOLUME: AutomationTarget = AutomationTarget {
            instance_id: 0,
            param: 0,
        };
        let mut song = Song::default();
        song.add_channel(ChannelKind::Synth(SynthKind::MiniSynth));
        let b = song.channels[1].id;
        let ch = &mut song.channels[1];
        let uid = crate::plugins::builtin::named("Lemon Peak Controller").uid;
        let id = crate::plugins::add(&mut ch.rack, b, uid, 1).unwrap();
        ch.file.modulations.push(Modulation {
            modulator: id,
            channel: b,
            target: VOLUME,
        });
        let read = crate::flm::read(&crate::flm::write(&song).unwrap()).unwrap();
        assert_eq!(
            read.channels[1].file.modulations,
            song.channels[1].file.modulations
        );
        crate::plugins::sync(&read);
        let mut engine = SongAudioEngine::new(&read, RATE).unwrap();
        let base = value(&engine, VOLUME);
        engine.play();
        let quiet = values(&mut engine, 8192, VOLUME);
        engine.note_on(1, 57, 1.0);
        let loud = values(&mut engine, 8192, VOLUME);
        let (low, high) = (
            quiet[quiet.len() - 1],
            loud.iter().copied().fold(0.0, f32::max),
        );
        assert!(low < 0.05 && low < base, "quiet: {quiet:?}");
        assert!(high > low + 0.3, "loud: {loud:?}");
        // Smoothed: no block jumps all the way at once.
        assert!(
            loud.windows(2).all(|w| (w[1] - w[0]).abs() < 0.3),
            "{loud:?}"
        );
    }

    #[test]
    fn removing_the_modulator_or_stopping_puts_the_base_back_exactly() {
        // A 20 Hz sine at +100 % on the Filter's Cutoff, an effect param the edit's engine takes over with its effect.
        let params = [(m::SOURCE, 0.5), (m::AMOUNT, 1.0), (m::RATE, 1.0)];
        let modulated = song(&params, false, CUTOFF);
        let mut engine = SongAudioEngine::new(&modulated, RATE).unwrap();
        let base = value(&engine, CUTOFF);
        engine.play();
        assert!(values(&mut engine, 2048, CUTOFF).iter().any(|&v| v != base));
        let mut without = modulated.clone();
        without.channels[1]
            .rack
            .modules
            .retain(|m| m.type_id != MODULATOR);
        let mut edited = SongAudioEngine::new(&without, RATE).unwrap().adopting();
        edited.take_over(&mut engine);
        assert!(values(&mut edited, 2048, CUTOFF).iter().all(|&v| v == base));
        // Stop, with it still there.
        let mut engine = SongAudioEngine::new(&modulated, RATE).unwrap();
        engine.play();
        assert!(values(&mut engine, 2048, CUTOFF).iter().any(|&v| v != base));
        engine.stop();
        assert!(values(&mut engine, 2048, CUTOFF).iter().all(|&v| v == base));
    }
}
