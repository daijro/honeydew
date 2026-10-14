use std::convert::Infallible;

use crate::playback::StereoBlock;

pub struct OwnedStereoBuffer {
    pub left: Vec<f32>,
    pub right: Vec<f32>,
}

pub trait MixBuffer {
    fn stereo(&mut self) -> (&mut [f32], &mut [f32]);
}

impl MixBuffer for OwnedStereoBuffer {
    fn stereo(&mut self) -> (&mut [f32], &mut [f32]) {
        (&mut self.left, &mut self.right)
    }
}

impl MixBuffer for StereoBlock<'_> {
    fn stereo(&mut self) -> (&mut [f32], &mut [f32]) {
        (self.left, self.right)
    }
}

/// Channel params 0x1000 + id (Enable Send) and 0x2000 + id (Send Level) name the channel's send to the channel with
/// id `id` (ids are below 4096): (id, whether it is the level). `None` for any other param.
pub fn send_param(index: i32) -> Option<(i32, bool)> {
    match index {
        0x1000..=0x1fff => Some((index - 0x1000, false)),
        0x2000..=0x2fff => Some((index - 0x2000, true)),
        _ => None,
    }
}

/// One send of a channel (`SEND`): into channel `target`'s input, at `level` by the channel
/// Level law, mixing only while `enabled` > 0.5.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct SendMix {
    /// The target channel's index.
    pub target: usize,
    pub enabled: f32,
    pub level: f32,
}

/// A channel's strip: Level and Pan, then its sends. Channel 0 is the master, which has none and is the output; any
/// other channel without a send has no output.
#[derive(Clone, Debug, PartialEq)]
pub struct ChannelMix {
    pub level: f32,
    pub pan: f32,
    pub sends: Vec<SendMix>,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum MixError<E = Infallible> {
    MismatchedChannels,
    MismatchedBuffers,
    InvalidChannel(usize),
    InvalidControl(usize),
    /// A send from the master, to the channel itself or to no channel.
    InvalidRoute(usize),
    CyclicRoute,
    Process(E),
}

/// Mixes the channels into the master: each runs its rack, then its Level and Pan, then adds itself into each send's
/// target at that send's level. A target runs after all its sources. Sends are post-fader and post-pan, with
/// the default send to the master at unity this is the plain strip.
pub struct Mixer {
    channels: Vec<ChannelMix>,
    order: Vec<usize>,
}

impl Mixer {
    pub fn new(channels: &[ChannelMix]) -> Result<Self, MixError> {
        if channels.is_empty() {
            return Err(MixError::MismatchedChannels);
        }
        let mut incoming = vec![0; channels.len()];
        for (index, channel) in channels.iter().enumerate() {
            if !valid_controls(channel.level, channel.pan)
                || !(channel.sends.iter()).all(|s| valid_controls(s.level, s.enabled))
            {
                return Err(MixError::InvalidControl(index));
            }
            for send in &channel.sends {
                if index == 0 || send.target == index || send.target >= channels.len() {
                    return Err(MixError::InvalidRoute(index));
                }
                incoming[send.target] += 1;
            }
        }
        // Sources before targets (Kahn); the master, which sends nowhere, last.
        let mut queue: Vec<usize> = (1..channels.len())
            .filter(|&index| incoming[index] == 0)
            .collect();
        let mut cursor = 0;
        while cursor < queue.len() {
            for send in &channels[queue[cursor]].sends {
                incoming[send.target] -= 1;
                if send.target != 0 && incoming[send.target] == 0 {
                    queue.push(send.target);
                }
            }
            cursor += 1;
        }
        if queue.len() != channels.len() - 1 {
            return Err(MixError::CyclicRoute);
        }
        queue.push(0);
        Ok(Self {
            channels: channels.to_vec(),
            order: queue,
        })
    }

    pub fn set_channel_controls(
        &mut self,
        channel: usize,
        level: f32,
        pan: f32,
    ) -> Result<(), MixError> {
        if channel >= self.channels.len() {
            return Err(MixError::InvalidChannel(channel));
        }
        if !valid_controls(level, pan) {
            return Err(MixError::InvalidControl(channel));
        }
        self.channels[channel].level = level;
        self.channels[channel].pan = pan;
        Ok(())
    }

    /// Channel `channel`'s send to channel `target`: its Enable (`level` false) or its Level (`level` true). A value
    /// outside 0..=1 or a send that does not exist is refused.
    pub fn set_send(
        &mut self,
        channel: usize,
        target: usize,
        level: bool,
        value: f32,
    ) -> Result<(), MixError> {
        if !valid_controls(value, value) {
            return Err(MixError::InvalidControl(channel));
        }
        let sends = self.channels.get_mut(channel).map(|c| &mut c.sends);
        let send = sends.and_then(|sends| sends.iter_mut().find(|s| s.target == target));
        let send = send.ok_or(MixError::InvalidRoute(channel))?;
        *if level {
            &mut send.level
        } else {
            &mut send.enabled
        } = value;
        Ok(())
    }

    pub fn mix<B: MixBuffer, E>(
        &self,
        buffers: &mut [B],
        mut process: impl FnMut(usize, &mut [f32], &mut [f32]) -> Result<(), E>,
    ) -> Result<(), MixError<E>> {
        if buffers.len() != self.channels.len() {
            return Err(MixError::MismatchedChannels);
        }
        let frames = buffers[0].stereo().0.len();
        if buffers.iter_mut().any(|buffer| {
            let (left, right) = buffer.stereo();
            left.len() != frames || right.len() != frames
        }) {
            return Err(MixError::MismatchedBuffers);
        }
        for &index in &self.order {
            let (left, right) = buffers[index].stereo();
            process(index, left, right).map_err(MixError::Process)?;
            let channel = &self.channels[index];
            let gain = level_gain(channel.level);
            let left_gain = gain * (2.0 - 2.0 * channel.pan).min(1.0);
            let right_gain = gain * (2.0 * channel.pan).min(1.0);
            for (left, right) in left.iter_mut().zip(right.iter_mut()) {
                *left *= left_gain;
                *right *= right_gain;
            }
            for send in channel.sends.iter().filter(|send| send.enabled > 0.5) {
                let gain = level_gain(send.level);
                let (source, target) = if index < send.target {
                    let (before, after) = buffers.split_at_mut(send.target);
                    (&mut before[index], &mut after[0])
                } else {
                    let (before, after) = buffers.split_at_mut(index);
                    (&mut after[0], &mut before[send.target])
                };
                let (source_left, source_right) = source.stereo();
                let (target_left, target_right) = target.stereo();
                for ((target_left, target_right), (left, right)) in target_left
                    .iter_mut()
                    .zip(target_right.iter_mut())
                    .zip(source_left.iter().zip(source_right.iter()))
                {
                    *target_left += gain * left;
                    *target_right += gain * right;
                }
            }
        }
        Ok(())
    }
}

fn valid_controls(level: f32, pan: f32) -> bool {
    level.is_finite()
        && pan.is_finite()
        && (0.0..=1.0).contains(&level)
        && (0.0..=1.0).contains(&pan)
}

/// The channel Level law: dB = 66·v^0.6 − 60, so 0.8531 is unity and 1 is +6 dB;
/// v = 0 is exact silence. Send Level follows it too, `SEND`@20 stores it the same way (0.8531 = 0 dB) and
/// @8 is derived from it.
fn level_gain(level: f32) -> f32 {
    if level == 0.0 {
        return 0.0;
    }
    10.0_f32.powf((66.0 * level.powf(0.6) - 60.0) / 20.0)
}
