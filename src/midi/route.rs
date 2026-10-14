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

#[derive(Clone, Copy, Debug, PartialEq)]
pub enum Out {
    /// `velocity` = MIDI velocity · [`VELOCITY_STEP`] at a note-on, / 127 at a note-off.
    Note {
        channel: usize,
        key: u8,
        on: bool,
        velocity: f32,
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

#[derive(Debug, Default)]
pub struct Router {
    pub learn: Learn,
    /// Sounding keys → the channel their note-on went to.
    held: BTreeMap<u8, usize>,
}

impl Router {
    /// Routes one message to `channel` (the selected one); `links` are every channel's ([`super::links::song_links`]).
    /// All 16 MIDI channels are merged (omni).
    pub fn route(&mut self, msg: MidiMsg, channel: usize, links: &[(usize, CcLink)]) -> Vec<Out> {
        let mut out = Vec::new();
        match msg {
            MidiMsg::NoteOn { key, vel, .. } => {
                // The key sounds on one channel at a time, so no note is ever stranded.
                if let Some(old) = self.held.insert(key, channel)
                    && old != channel
                {
                    out.push(Out::Note {
                        channel: old,
                        key,
                        on: false,
                        velocity: 0.0,
                    });
                }
                out.push(Out::Note {
                    channel,
                    key,
                    on: true,
                    velocity: f32::from(vel) * VELOCITY_STEP,
                });
                if self.learn == Learn::Note {
                    self.learn = Learn::Off;
                    out.push(Out::LearnedNote(key));
                }
            }
            MidiMsg::NoteOff { key, vel, .. } => {
                let channel = self.held.remove(&key).unwrap_or(channel);
                out.push(Out::Note {
                    channel,
                    key,
                    on: false,
                    velocity: f32::from(vel) / 127.0,
                });
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
                // The selected channel's own CC param only when no link fired, or for the mod wheel. There
                // is no CC #127 channel param.
                if (out.is_empty() || cc == 1) && cc < 127 {
                    out.push(Out::ChannelParam {
                        channel,
                        index: 7 + i32::from(cc),
                        value,
                    });
                }
            }
            MidiMsg::PitchBend { value, .. } => out.push(Out::ChannelParam {
                channel,
                index: 4,
                value: f32::from(value) / 16383.0,
            }),
            MidiMsg::ChannelPressure { value, .. } => out.push(Out::ChannelParam {
                channel,
                index: 5,
                value: f32::from(value) / 127.0,
            }),
            // Ignored live.
            MidiMsg::Program { .. } => {}
        }
        out
    }

    /// The keys held, each with the channel it sounds on.
    pub fn held(&self) -> impl Iterator<Item = (u8, usize)> + '_ {
        self.held.iter().map(|(&key, &channel)| (key, channel))
    }

    /// The channels moved: each held key follows its channel by `channel`. A key whose channel went holds the master,
    /// which plays nothing, so its note-off goes nowhere.
    pub fn rekey(&mut self, channel: &dyn Fn(usize) -> Option<usize>) {
        for c in self.held.values_mut() {
            *c = channel(*c).unwrap_or(0);
        }
    }

    /// Note-offs for every held key, each on its own channel.
    pub fn release_all(&mut self) -> Vec<Out> {
        std::mem::take(&mut self.held)
            .into_iter()
            .map(|(key, channel)| Out::Note {
                channel,
                key,
                on: false,
                velocity: 0.0,
            })
            .collect()
    }
}
