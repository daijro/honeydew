use crate::automation::{AutomationError, AutomationPlayback, AutomationTarget};
use crate::{AudioEffect, EffectError, ProcessContext};

#[derive(Clone, Copy, Debug, PartialEq)]
pub struct AutomationChange {
    pub frame: usize,
    pub target: AutomationTarget,
    pub value: f32,
}

pub struct StereoBlock<'a> {
    pub left: &'a mut [f32],
    pub right: &'a mut [f32],
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum PlaybackError {
    InvalidTarget,
    DuplicateInstance,
    MismatchedBuffers,
    InvalidChanges,
    InvalidContext,
    Automation(AutomationError),
    Effect(EffectError),
}

struct EffectInstance {
    id: i32,
    effect: AudioEffect,
}

pub struct PlaybackChannel {
    parameters: [f32; 134],
    effects: Vec<EffectInstance>,
}

pub struct AudioPlayback {
    pub automation: AutomationPlayback,
    channels: Vec<PlaybackChannel>,
}

impl AudioPlayback {
    pub fn new(channel_count: usize, automation: AutomationPlayback) -> Self {
        Self {
            automation,
            channels: (0..channel_count)
                .map(|_| PlaybackChannel {
                    parameters: {
                        let mut parameters = [0.0; 134];
                        parameters[..6].copy_from_slice(&[0.8531, 0.5, 0.0, 0.0, 0.5, 0.0]);
                        parameters
                    },
                    effects: Vec::new(),
                })
                .collect(),
        }
    }

    pub fn add_effect(
        &mut self,
        channel: usize,
        instance_id: i32,
        effect: AudioEffect,
    ) -> Result<(), PlaybackError> {
        let channel = self
            .channels
            .get_mut(channel)
            .ok_or(PlaybackError::InvalidTarget)?;
        if instance_id < 1 {
            return Err(PlaybackError::InvalidTarget);
        }
        if channel.effects.iter().any(|entry| entry.id == instance_id) {
            return Err(PlaybackError::DuplicateInstance);
        }
        channel.effects.push(EffectInstance {
            id: instance_id,
            effect,
        });
        Ok(())
    }

    pub fn effect(&self, channel: usize, instance_id: i32) -> Option<&AudioEffect> {
        self.channels
            .get(channel)?
            .effects
            .iter()
            .find(|entry| entry.id == instance_id)
            .map(|entry| &entry.effect)
    }

    pub fn effect_mut(&mut self, channel: usize, instance_id: i32) -> Option<&mut AudioEffect> {
        self.channels
            .get_mut(channel)?
            .effects
            .iter_mut()
            .find(|entry| entry.id == instance_id)
            .map(|entry| &mut entry.effect)
    }

    /// Takes over the running effects of `old`'s channel `from` (the playback this one replaces after an edit) for
    /// `channel`: each effect here of the same instance id and type as one there swaps with it, so the running one goes
    /// on here, its state and tail included, with the buffers built here for the new tempo
    /// ([`AudioEffect::take_buffers`]), and `old` keeps the one built here. `adopted` hears each instance id taken.
    /// Allocates nothing.
    pub fn adopt_effects(
        &mut self,
        channel: usize,
        old: &mut AudioPlayback,
        from: usize,
        mut adopted: impl FnMut(i32),
    ) {
        let (Some(ours), Some(theirs)) =
            (self.channels.get_mut(channel), old.channels.get_mut(from))
        else {
            return;
        };
        // An external effect is not swapped: what runs on is outside both engines, which wrap it alike.
        for entry in ours.effects.iter_mut().filter(|e| !e.effect.is_external()) {
            let same = |other: &&mut EffectInstance| {
                other.id == entry.id && other.effect.type_id() == entry.effect.type_id()
            };
            if let Some(other) = theirs.effects.iter_mut().find(same) {
                std::mem::swap(&mut entry.effect, &mut other.effect);
                entry.effect.take_buffers(&mut other.effect);
                adopted(entry.id);
            }
        }
    }

    pub fn channel_parameter(&self, channel: usize, index: usize) -> Option<f32> {
        self.channels.get(channel)?.parameters.get(index).copied()
    }

    pub fn reset_effects(&mut self) {
        for channel in &mut self.channels {
            for entry in &mut channel.effects {
                entry.effect.reset();
            }
        }
    }

    pub fn process_channel(
        &mut self,
        channel: usize,
        left: &mut [f32],
        right: &mut [f32],
        context: ProcessContext,
    ) -> Result<(), PlaybackError> {
        self.process_effects(channel, 0..usize::MAX, left, right, context)
    }

    /// [`Self::process_channel`] for only the channel's effects at positions `effects` (0 is its first, in rack order):
    /// the run between two of its instruments. Positions past its last effect
    /// are ignored.
    pub fn process_effects(
        &mut self,
        channel: usize,
        effects: std::ops::Range<usize>,
        left: &mut [f32],
        right: &mut [f32],
        context: ProcessContext,
    ) -> Result<(), PlaybackError> {
        self.channels
            .get_mut(channel)
            .ok_or(PlaybackError::InvalidTarget)?
            .process_effects(effects, left, right, context)
    }

    /// Independently owned channel processors, for routing-level parallel playback.
    pub fn channels_mut(&mut self) -> &mut [PlaybackChannel] {
        &mut self.channels
    }

    pub fn set_target(
        &mut self,
        target: AutomationTarget,
        value: f32,
    ) -> Result<(), PlaybackError> {
        if !value.is_finite() || !(0.0..=1.0).contains(&value) {
            return Err(PlaybackError::InvalidTarget);
        }
        self.validate_target(target)?;
        let channel = &mut self.channels[target.channel];
        if target.instance_id == 0 {
            channel.parameters[target.parameter_index as usize] = value;
            return Ok(());
        }
        let effect = &mut channel
            .effects
            .iter_mut()
            .find(|entry| entry.id == target.instance_id)
            .unwrap()
            .effect;
        if target.parameter_index >= 0 {
            effect
                .set_parameter(target.parameter_index as usize, value)
                .map_err(PlaybackError::Effect)
        } else {
            effect
                .set_common_parameter((0x7fc0 - target.parameter_index - 1) as u16, value)
                .map_err(PlaybackError::Effect)
        }
    }

    pub fn seek(&mut self, song_tick: u64) -> Result<(), PlaybackError> {
        for lane in &self.automation.lanes {
            self.validate_target(lane.target)?;
            if !lane.initial_value.is_finite() || !(0.0..=1.0).contains(&lane.initial_value) {
                return Err(PlaybackError::InvalidTarget);
            }
        }
        let mut changes = Vec::new();
        self.automation
            .seek(song_tick, |target, value| changes.push((target, value)));
        for (target, value) in changes {
            self.set_target(target, value)?;
        }
        Ok(())
    }

    pub fn collect_changes(
        &self,
        context: ProcessContext,
        frame_count: usize,
    ) -> Result<Vec<AutomationChange>, PlaybackError> {
        let mut changes = Vec::new();
        self.automation
            .dispatch_block(
                context.start_beat,
                frame_count,
                f64::from(context.sample_rate),
                f64::from(context.tempo_bpm),
                |frame, target, value| {
                    changes.push(AutomationChange {
                        frame,
                        target,
                        value,
                    });
                },
            )
            .map_err(PlaybackError::Automation)?;
        Ok(changes)
    }

    pub fn process_block(
        &mut self,
        buffers: &mut [StereoBlock<'_>],
        context: ProcessContext,
        changes: &[AutomationChange],
    ) -> Result<(), PlaybackError> {
        if buffers.len() != self.channels.len()
            || !context.sample_rate.is_finite()
            || context.sample_rate <= 0.0
            || !context.tempo_bpm.is_finite()
            || context.tempo_bpm <= 0.0
            || !context.start_beat.is_finite()
        {
            return Err(PlaybackError::InvalidContext);
        }
        let frames = buffers.first().map_or(0, |buffer| buffer.left.len());
        if buffers
            .iter()
            .any(|buffer| buffer.left.len() != frames || buffer.right.len() != frames)
        {
            return Err(PlaybackError::MismatchedBuffers);
        }
        if self
            .channels
            .iter()
            .flat_map(|channel| &channel.effects)
            .any(|entry| entry.effect.sample_rate != context.sample_rate)
        {
            return Err(PlaybackError::Effect(EffectError::InvalidSampleRate));
        }
        let mut previous = 0;
        for change in changes {
            if change.frame < previous || change.frame >= frames {
                return Err(PlaybackError::InvalidChanges);
            }
            self.validate_target(change.target)?;
            if !change.value.is_finite() || !(0.0..=1.0).contains(&change.value) {
                return Err(PlaybackError::InvalidChanges);
            }
            previous = change.frame;
        }
        let mut cursor = 0;
        for (position, change) in changes.iter().enumerate() {
            self.process_segment(buffers, context, cursor, change.frame)?;
            cursor = change.frame;
            // Only the last of several changes of one target at one frame acts (a Pitcher 2 Low Latency switch
            // rebuilds).
            let superseded = changes[position + 1..]
                .iter()
                .take_while(|next| next.frame == cursor)
                .any(|next| next.target == change.target);
            if !superseded {
                self.set_target(change.target, change.value)?;
            }
        }
        self.process_segment(buffers, context, cursor, frames)
    }

    fn validate_target(&self, target: AutomationTarget) -> Result<(), PlaybackError> {
        let channel = self
            .channels
            .get(target.channel)
            .ok_or(PlaybackError::InvalidTarget)?;
        if target.instance_id == 0 {
            if (0..channel.parameters.len() as i32).contains(&target.parameter_index) {
                return Ok(());
            }
        } else if target.instance_id > 0
            && let Some(effect) = channel
                .effects
                .iter()
                .find(|entry| entry.id == target.instance_id)
                .map(|entry| &entry.effect)
            && ((target.parameter_index >= 0
                && effect.parameter(target.parameter_index as usize).is_some())
                || (-3..=-1).contains(&target.parameter_index))
        {
            return Ok(());
        }
        Err(PlaybackError::InvalidTarget)
    }

    fn process_segment(
        &mut self,
        buffers: &mut [StereoBlock<'_>],
        context: ProcessContext,
        start: usize,
        end: usize,
    ) -> Result<(), PlaybackError> {
        if start == end {
            return Ok(());
        }
        let segment_context = ProcessContext {
            start_beat: context.start_beat
                + start as f64 * f64::from(context.tempo_bpm)
                    / (60.0 * f64::from(context.sample_rate)),
            ..context
        };
        for (index, buffer) in buffers.iter_mut().enumerate() {
            self.process_channel(
                index,
                &mut buffer.left[start..end],
                &mut buffer.right[start..end],
                segment_context,
            )?;
        }
        Ok(())
    }
}

impl PlaybackChannel {
    /// The effects at positions `effects` in this channel's rack.
    pub fn process_effects(
        &mut self,
        effects: std::ops::Range<usize>,
        left: &mut [f32],
        right: &mut [f32],
        context: ProcessContext,
    ) -> Result<(), PlaybackError> {
        if left.len() != right.len() {
            return Err(PlaybackError::MismatchedBuffers);
        }
        if !context.sample_rate.is_finite()
            || context.sample_rate <= 0.0
            || !context.tempo_bpm.is_finite()
            || context.tempo_bpm <= 0.0
            || !context.start_beat.is_finite()
        {
            return Err(PlaybackError::InvalidContext);
        }
        for entry in self
            .effects
            .iter_mut()
            .take(effects.end)
            .skip(effects.start)
        {
            if entry.effect.sample_rate != context.sample_rate {
                return Err(PlaybackError::Effect(EffectError::InvalidSampleRate));
            }
            entry
                .effect
                .process(left, right, context)
                .map_err(PlaybackError::Effect)?;
        }
        Ok(())
    }
}
