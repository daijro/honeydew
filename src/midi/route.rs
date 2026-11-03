//! The MIDI input router: messages in, [`Out`]s for the app to execute. It holds no audio and no song.

use std::collections::BTreeMap;

use super::{CcLink, MidiMsg};

/// Set by the app while a learn prompt waits; a result switches it back to `Off`.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum Learn {
    #[default]
    Off,
    /// The next note-on → [`Out::LearnedNote`]; the note still plays.
    Note,
    /// The next CC → [`Out::LearnedCc`], not forwarded.
    Cc,
}

/// A note-on's velocity per MIDI step: v·0.007874, ≈ v/127.
const VELOCITY_STEP: f32 = 0.007874;

/// A channel a message reaches, and whether it sounds there (else REC only hears it: an armed channel that neither is
/// selected nor monitors).
pub type Target = (usize, bool);

#[derive(Clone, Copy, Debug, PartialEq)]
pub enum Out {
    /// `velocity` = MIDI velocity · [`VELOCITY_STEP`] at a note-on, / 127 at a note-off; `sound` as its [`Target`]'s.
    Note {
        channel: usize,
        key: u8,
        on: bool,
        velocity: f32,
        sound: bool,
    },
    /// A live channel param: 7 + CC, 4 = pitch bend, 5 = aftertouch.
    ChannelParam {
        channel: usize,
        index: i32,
        value: f32,
    },
    /// A CC-linked param of `channel` (the link's owner), set to the CC value.
    Linked {
        channel: usize,
        instance_id: i32,
        param: i32,
        value: f32,
    },
    LearnedNote(u8),
    LearnedCc(u8),
}

fn note((channel, sound): Target, key: u8, on: bool, velocity: f32) -> Out {
    Out::Note {
        channel,
        key,
        on,
        velocity,
        sound,
    }
}

#[derive(Debug, Default)]
pub struct Router {
    pub learn: Learn,
    /// Sounding keys, by (device, MIDI channel, key) → the targets their note-on went to.
    held: BTreeMap<(String, u8, u8), Vec<Target>>,
}

impl Router {
    /// Routes one message from `device` to `targets` (the channels whose MIDI input it passes: the selected one, the
    /// monitoring and the armed ones); `links` are every channel's ([`super::links::song_links`]). A key's note-off goes
    /// where its note-on from the same device and MIDI channel went.
    pub fn route(
        &mut self,
        device: &str,
        msg: MidiMsg,
        targets: &[Target],
        links: &[(usize, CcLink)],
    ) -> Vec<Out> {
        let mut out = Vec::new();
        // Channel params are live engine values: the channels that sound.
        let sounding = targets.iter().filter(|t| t.1).map(|t| t.0);
        match msg {
            MidiMsg::NoteOn { ch, key, vel } => {
                // The key sounds on one set of channels at a time, so no note is ever stranded.
                let old = self
                    .held
                    .insert((device.to_owned(), ch, key), targets.to_vec());
                let gone = old.into_iter().flatten().filter(|t| !targets.contains(t));
                out.extend(gone.map(|t| note(t, key, false, 0.0)));
                let velocity = f32::from(vel) * VELOCITY_STEP;
                out.extend(targets.iter().map(|&t| note(t, key, true, velocity)));
                if self.learn == Learn::Note {
                    self.learn = Learn::Off;
                    out.push(Out::LearnedNote(key));
                }
            }
            MidiMsg::NoteOff { ch, key, vel } => {
                let held = self.held.remove(&(device.to_owned(), ch, key));
                let velocity = f32::from(vel) / 127.0;
                let ends = held.unwrap_or_else(|| targets.to_vec());
                out.extend(ends.into_iter().map(|t| note(t, key, false, velocity)));
            }
            MidiMsg::Cc { cc, .. } if self.learn == Learn::Cc => {
                self.learn = Learn::Off;
                out.push(Out::LearnedCc(cc));
            }
            MidiMsg::Cc { cc, value, .. } => {
                let value = f32::from(value) / 127.0;
                out.extend(
                    links
                        .iter()
                        .filter(|(_, l)| l.cc == cc)
                        .map(|&(channel, l)| Out::Linked {
                            channel,
                            instance_id: l.instance_id,
                            param: l.param,
                            value,
                        }),
                );
                // The targets' own CC param only when no link fired, or for the mod wheel. There is no CC #127
                // channel param.
                if (out.is_empty() || cc == 1) && cc < 127 {
                    out.extend(sounding.map(|channel| Out::ChannelParam {
                        channel,
                        index: 7 + i32::from(cc),
                        value,
                    }));
                }
            }
            MidiMsg::PitchBend { value, .. } => {
                out.extend(sounding.map(|channel| Out::ChannelParam {
                    channel,
                    index: 4,
                    value: f32::from(value) / 16383.0,
                }))
            }
            MidiMsg::ChannelPressure { value, .. } => {
                out.extend(sounding.map(|channel| Out::ChannelParam {
                    channel,
                    index: 5,
                    value: f32::from(value) / 127.0,
                }))
            }
            // Ignored live.
            MidiMsg::Program { .. } => {}
        }
        out
    }

    /// The channels moved: each held key follows its channels by `channel`. A channel that went is dropped, so the
    /// key's note-off goes nowhere there.
    pub fn rekey(&mut self, channel: &dyn Fn(usize) -> Option<usize>) {
        for targets in self.held.values_mut() {
            targets.retain_mut(|t| channel(t.0).map(|c| t.0 = c).is_some());
        }
    }

    /// Note-offs for every held key, on each of its channels.
    pub fn release_all(&mut self) -> Vec<Out> {
        let held = std::mem::take(&mut self.held).into_iter();
        let ends =
            held.flat_map(|((_, _, key), targets)| targets.into_iter().map(move |t| (t, key)));
        ends.map(|(t, key)| note(t, key, false, 0.0)).collect()
    }
}
