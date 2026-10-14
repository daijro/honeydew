//! MIDI input in the app. Each frame the device link's messages go through the [`Router`], and its outputs are executed
//! here:
//! - notes play on the selected channel the way the on-screen keyboard's do ([`HoneydewApp::key_on`]); each key of a
//!   chord sounds, and a key ends on the channel it began on;
//! - the selected channel's own CC, pitch bend and aftertouch params are live engine values, no song edit;
//! - a CC link sets its param on the channel that owns it, as a hand move does: the song-changed flag and no undo step,
//!   and REC records it when it is the recent control. The undo history takes a burst's values [`BURST_SECS`] after its
//!   last move with no pointer down ([`crate::history::History::sync`]);
//! - while `Set MIDI Note` waits, the next note-on is the pad's note (one undo step), and it also plays;
//! - while `Link to MIDI CC` waits ([`Overlay::LearnCc`] on top), the next CC of any number is linked to the recent
//!   control: a record in its channel's `CCLK`, one undo step. `Link to Mod Wheel` links CC 1 at once and
//!   `Unlink from MIDI CC` removes the param's links, one undo step each ([`HoneydewApp::link_cc`]).
//!
//! With the master selected, notes and its CC params go nowhere, as the keyboard's; links still fire.

use std::any::Any;

use eframe::egui;
use honeydew_audio::device::AudioCommand;

use super::{HoneydewApp, START_ON_RELEASE, audio_gesture, typed};
use crate::midi::device::{MidiChoice, MidiLink};
use crate::midi::links::{self, song_links};
use crate::midi::{Learn, MidiMsg, Out, Router};
use crate::song::AutomationTarget;
use crate::ui::views::{self, Nav, Overlay, SongEdit, View};

/// Linked CC moves closer together than this make one undo step.
pub(super) const BURST_SECS: f64 = 0.5;

pub(super) struct Midi {
    /// The input device. `HoneydewApp::new` makes it the platform's; the default lists and opens nothing.
    pub(super) link: MidiLink,
    pub(super) router: Router,
    /// The link first opens in a frame with a click: on the web that asks for MIDI access. Natively at once.
    pub(super) on_gesture: bool,
    /// The link was opened; from then on it reopens when the chosen device changes.
    opened: bool,
    /// [`MidiLink::reopened`] as last seen.
    reopened: u32,
    /// The egui time of the last linked CC move.
    linked_at: Option<f64>,
    /// Each MIDI key sounding, as (channel, controller key, the key it plays under key-lock): its note-off ends that
    /// key, whatever the lock or the current channel has become since (as a typed key's release).
    pub(super) snapped: Vec<(usize, u8, u8)>,
}

impl Default for Midi {
    fn default() -> Self {
        Self {
            link: MidiLink::new(|_, _, _| Ok(Box::new(()) as Box<dyn Any>), Vec::new, None),
            router: Router::default(),
            on_gesture: cfg!(target_arch = "wasm32"),
            opened: false,
            reopened: 0,
            linked_at: None,
            snapped: Vec::new(),
        }
    }
}

impl HoneydewApp {
    /// The SETTINGS `Input device:` pick.
    fn midi_device(&self) -> MidiChoice {
        MidiChoice::from_label(&self.settings.midi_device)
    }

    /// The `Input device:` dropdown opened: the ports are listed again, once. Its tap is a user gesture, so a link
    /// never opened (the web waits for one) opens instead.
    pub(super) fn midi_rescan(&mut self) {
        let device = self.midi_device();
        let midi = &mut self.midi;
        if midi.opened {
            midi.link.rescan();
        } else {
            midi.opened = true;
            // As in [`Self::midi_input`].
            let _ = midi.link.open(device);
        }
    }

    /// One frame of MIDI input, before the screens, which show what it moved. Opens the link (at once natively, at the
    /// first click on the web) and again when the chosen device changes, plays the messages that came, and releases
    /// every held key when the link reconnected: an unplugged device sends no note-offs. `tick` is the transport
    /// position.
    ///
    /// `hidden`: the web page was hidden since the last frame. What the device sent meanwhile is dropped, not played
    /// late in a burst: the hide released every key, and a linked move would be an edit nobody made.
    pub(super) fn midi_input(&mut self, ctx: &egui::Context, hidden: bool, tick: u64) {
        let device = self.midi_device();
        let midi = &mut self.midi;
        let open = if midi.opened {
            device != *midi.link.choice()
        } else {
            !midi.on_gesture || ctx.input(|i| audio_gesture(i, START_ON_RELEASE))
        };
        if open {
            midi.opened = true;
            // A failed open leaves nothing open, and the link's rescans retry it.
            let _ = midi.link.open(device);
        }
        midi.link.tick(self.now);
        let mut messages = midi.link.drain();
        if hidden {
            messages.clear();
        }
        // `Link to MIDI CC` waits while its prompt is shown, the topmost overlay; `Set MIDI Note` in the drum editor
        // only ([`views::drum_editor::learned_note`] knows whether it does).
        midi.router.learn = if self.nav.overlays.last() == Some(&Overlay::LearnCc) {
            Learn::Cc
        } else if matches!(self.nav.view, View::DrumEditor(_)) {
            Learn::Note
        } else {
            Learn::Off
        };
        let mut links = if messages.is_empty() {
            Vec::new()
        } else {
            song_links(&self.song)
        };
        let mut linked = false;
        for msg in messages.iter().filter_map(|m| MidiMsg::parse(m)) {
            let mut outs = self.midi.router.route(msg, self.selected_channel, &links);
            // The learned key plays after its edit rebuilds the engine, so it sounds with its pad.
            outs.sort_by_key(|out| !matches!(out, Out::LearnedNote(_)));
            for out in outs {
                let learned = matches!(out, Out::LearnedCc(_));
                linked |= self.midi_out(out, tick);
                // The rest of the twist that was learned moves its param.
                if learned {
                    links = song_links(&self.song);
                }
            }
        }
        if linked {
            self.midi.linked_at = Some(self.now);
            self.song_edited(SongEdit::ChannelParam, tick);
        }
        // After the drain: a note-on the old connection queued is held by now, so this releases it too.
        if self.midi.link.reopened() != self.midi.reopened {
            self.midi.reopened = self.midi.link.reopened();
            self.release_midi();
        }
    }

    /// Note-offs for every key MIDI holds, each on the channel it plays on.
    pub(super) fn release_midi(&mut self) {
        for out in self.midi.router.release_all() {
            self.midi_out(out, 0);
        }
    }

    /// A burst of linked CC moves goes into the undo history once [`BURST_SECS`] pass without one; meanwhile it asks
    /// for the frame that does it.
    pub(super) fn midi_quiet(&self, ctx: &egui::Context) -> bool {
        let left = (self.midi.linked_at).map_or(0.0, |at| at + BURST_SECS - self.now);
        if left > 0.0 {
            ctx.request_repaint_after_secs(left as f32);
        }
        left <= 0.0
    }

    /// Executes one router output. Whether it was a linked param's move.
    fn midi_out(&mut self, out: Out, tick: u64) -> bool {
        match out {
            Out::Note { channel: 0, .. } | Out::ChannelParam { channel: 0, .. } => {}
            Out::Note {
                channel,
                key,
                on: true,
                velocity,
            } => {
                let played = self.key_locked(key);
                // A retrigger on its channel: a note the lock moved since ends first.
                if let Some(old) = self.unsnap(channel, key).filter(|&old| old != played) {
                    self.key_off(channel, old);
                }
                self.midi.snapped.push((channel, key, played));
                self.key_on(channel, played, velocity);
            }
            Out::Note { channel, key, .. } => {
                let played = self.unsnap(channel, key);
                self.key_off(channel, played.unwrap_or_else(|| self.key_locked(key)));
            }
            Out::ChannelParam {
                channel,
                index,
                value,
            } => {
                // CC 1 is the MOD wheel's value: the wheel shows it.
                if index == crate::ui::wheels::MOD
                    && let Some(ch) = self.song.channels.get(channel)
                {
                    self.mod_wheel.insert(ch.id, value);
                }
                // A live Pitch Bend or channel pressure is no song value: a re-sync and Stop put the song's back.
                self.audio.send(match index {
                    4 | 5 => AudioCommand::LiveParam {
                        channel,
                        index,
                        value,
                    },
                    _ => AudioCommand::SetParam {
                        channel,
                        instance_id: 0,
                        index,
                        value,
                    },
                });
            }
            Out::Linked {
                channel,
                instance_id,
                param,
                value,
            } => {
                let target = AutomationTarget { instance_id, param };
                let ch = self.song.channels.get_mut(channel);
                let Some(v) = ch.and_then(|c| c.param_mut(target)) else {
                    return false;
                };
                *v = value;
                if let Some(module) = self.song.channels[channel].rack.get_mut(instance_id) {
                    crate::panels::couple_param(module, param);
                }
                // A drum pad's address is its −slot id, as the drum strips send it.
                self.audio.send(AudioCommand::SetParam {
                    channel,
                    instance_id,
                    index: param,
                    value,
                });
                return true;
            }
            Out::LearnedNote(key) => {
                let editor = &mut self.slots.drum_editor;
                if views::drum_editor::learned_note(editor, &mut self.song, key) {
                    self.song_edited(SongEdit::Final, tick);
                }
            }
            Out::LearnedCc(cc) => {
                self.nav.overlays.retain(|o| *o != Overlay::LearnCc);
                self.link_cc(Some(cc), tick);
            }
        }
        false
    }

    /// The MOD wheel at `value` through the CC path: every param linked to CC 1 moves to it, unquantised, as a linked
    /// controller's would. Whether one moved (a param edit).
    pub(super) fn mod_wheel_cc(&mut self, value: f32) -> bool {
        let links = song_links(&self.song);
        let mut linked = false;
        for &(channel, link) in links.iter().filter(|(_, l)| l.cc == 1) {
            let out = Out::Linked {
                channel,
                instance_id: link.instance_id,
                param: link.param,
                value,
            };
            linked |= self.midi_out(out, 0);
        }
        linked
    }

    /// The key controller key `key` plays on `channel`, forgotten: what its press snapped it to ([`Midi::snapped`]).
    fn unsnap(&mut self, channel: usize, key: u8) -> Option<u8> {
        let snapped = &mut self.midi.snapped;
        let at = snapped.iter().position(|s| (s.0, s.1) == (channel, key))?;
        Some(snapped.remove(at).2)
    }

    /// A MIDI `key` under key-lock, as a typed key is (the snap comes before recording): the keyboard's lock, but none
    /// on a channel whose pads replace the keyboard. Its note-off ends the key the press played ([`Midi::snapped`]),
    /// else the one it snaps to now.
    fn key_locked(&self, key: u8) -> u8 {
        let pads = views::drum_editor::pads_channel(&self.song, self.selected_channel).is_some();
        typed::snap(key, self.key_input().lock.filter(|_| !pads), typed::UP)
    }

    /// `Link to MIDI CC`: its prompt opens, and the router waits for a CC while it is open ([`Self::midi_input`]).
    pub(super) fn learn_cc(&mut self) {
        if self.recent_index().is_some() {
            self.navigate(Nav::Open(Overlay::LearnCc));
        }
    }

    /// Links `cc` to the recent control (`Link to MIDI CC`'s learned CC, `Link to Mod Wheel`'s CC 1), next to any link
    /// it has, or with `None` removes its links (`Unlink from MIDI CC`): in the `CCLK` of its channel, the one its
    /// target names (for "the current channel"), with its target as the record's. A change is one undo step.
    pub(super) fn link_cc(&mut self, cc: Option<u8>, tick: u64) {
        let Some((channel, target)) = self.recent_index() else {
            return;
        };
        let raw = &mut self.song.channels[channel].file.cc_links;
        let before = raw.clone();
        match cc {
            Some(cc) => links::link(raw, cc, target.instance_id, target.param),
            None => links::unlink(raw, target.instance_id, target.param),
        }
        if *raw != before {
            self.song_edited(SongEdit::Final, tick);
        }
    }
}
