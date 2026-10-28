use std::collections::{BTreeMap, HashSet};

use crate::song::{AutomationTarget as SongTarget, Clip, Event, Lane, LaneKind, PPQ, Song};
use honeydew_audio::automation::{
    AutomationClip, AutomationPlayback, AutomationPoint, AutomationTarget,
};
use honeydew_audio::mixer::send_param;
use honeydew_audio::playback::{AudioPlayback, AutomationChange, PlaybackError, StereoBlock};
use honeydew_audio::sequencer::{InputNoteEvent, sort_note_events};
use honeydew_audio::{AudioEffect, EffectError, ProcessContext, effect_info};

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum SongAudioError {
    InvalidTiming,
    InvalidChannel,
    InvalidValue,
    Effect(EffectError),
    Playback(PlaybackError),
}

pub struct SongAudioBridge {
    pub audio: AudioPlayback,
}

/// A plugin effect module (desktop only), where its plugin runs: else it is left out and the sound passes.
fn add_plugin_effect(
    audio: &mut AudioPlayback,
    song: &Song,
    channel_index: usize,
    module: &crate::song::ModuleInstance,
    sample_rate: f32,
) -> Result<(), SongAudioError> {
    let channel = &song.channels[channel_index];
    let bar = song.time_sig.bar_beats();
    let Some(plugin) = crate::plugins::effect(channel, module, sample_rate, bar) else {
        return Ok(());
    };
    let (params, block) = (module.params.len(), PLUGIN_BLOCK);
    let mut effect = AudioEffect::external(module.type_id, sample_rate, plugin, params, block)
        .map_err(SongAudioError::Effect)?;
    for (index, value) in module.params.iter().copied().enumerate() {
        effect
            .set_parameter(index, value)
            .map_err(SongAudioError::Effect)?;
    }
    for (index, value) in module.common.0.into_iter().enumerate() {
        effect
            .set_common_parameter(0x7fc0 + index as u16, value)
            .map_err(SongAudioError::Effect)?;
    }
    audio
        .add_effect(channel_index, module.instance_id, effect)
        .map_err(SongAudioError::Playback)
}

/// The largest block a plugin effect takes at once (the engine's); longer ones run in parts.
const PLUGIN_BLOCK: usize = 1024;

impl SongAudioBridge {
    pub fn new(song: &Song, sample_rate: f32) -> Result<Self, SongAudioError> {
        if !sample_rate.is_finite() || sample_rate <= 0.0 {
            return Err(SongAudioError::InvalidTiming);
        }
        let mut audio = AudioPlayback::new(song.channels.len(), AutomationPlayback::default());
        for (channel_index, channel) in song.channels.iter().enumerate() {
            for (index, value) in channel.params.0.into_iter().enumerate() {
                audio
                    .set_target(
                        AutomationTarget {
                            channel: channel_index,
                            instance_id: 0,
                            parameter_index: index as i32,
                        },
                        value,
                    )
                    .map_err(SongAudioError::Playback)?;
            }
            for module in &channel.rack.modules {
                if module.type_id == crate::song::plugin::PLUGIN_EFFECT {
                    add_plugin_effect(&mut audio, song, channel_index, module, sample_rate)?;
                    continue;
                }
                let Some(info) = effect_info(module.type_id) else {
                    continue;
                };
                let mut effect = AudioEffect::new(module.type_id, sample_rate)
                    .map_err(SongAudioError::Effect)?;
                // Off the audio thread, for the song tempo: an edit's engine (a tempo drag's end) hands it to the running
                // effect it takes over, and a dragged tempo never resizes it (`AudioEffect::configure_tempo`).
                effect
                    .configure_tempo(song.tempo_bpm as f32)
                    .map_err(SongAudioError::Effect)?;
                for (index, value) in module
                    .params
                    .iter()
                    .copied()
                    .enumerate()
                    .take(info.parameter_count)
                {
                    effect
                        .set_parameter(index, value)
                        .map_err(SongAudioError::Effect)?;
                }
                for (index, value) in module.common.0.into_iter().enumerate() {
                    effect
                        .set_common_parameter(0x7fc0 + index as u16, value)
                        .map_err(SongAudioError::Effect)?;
                }
                audio
                    .add_effect(channel_index, module.instance_id, effect)
                    .map_err(SongAudioError::Playback)?;
            }
        }
        let mut bridge = Self { audio };
        bridge.seek(song, 0)?;
        Ok(bridge)
    }

    pub fn seek(&mut self, song: &Song, song_tick: u64) -> Result<(), SongAudioError> {
        let end_tick = song_tick
            .checked_add(1)
            .ok_or(SongAudioError::InvalidTiming)?;
        let mut initial = Vec::new();
        let mut seen = HashSet::new();
        // Ordered: the targets are set in the same order on every build (a render runs twice, `render.rs`).
        let mut latest = BTreeMap::new();
        let mut order = 0usize;
        for (channel_index, channel) in song.channels.iter().enumerate() {
            for lane in &channel.lanes {
                if lane.kind != LaneKind::Automation {
                    continue;
                }
                let Some((song_target, target)) = lane.target.and_then(|target| {
                    self.audio_target(song, channel_index, target)
                        .map(|audio_target| (target, audio_target))
                }) else {
                    continue;
                };
                // A MIDI CC (channel param ≥ 6) has no value in the song; it rests at 0.
                let value = channel.param(song_target).unwrap_or(0.0);
                valid_value(value)?;
                if seen.insert(target) {
                    initial.push((target, value));
                }
                for clip in playing_clips(lane) {
                    let visible = validate_clip(clip)?;
                    let clip_start = u64::from(clip.start_tick);
                    if song_tick < clip_start
                        || song_tick as f64 >= clip_start as f64 + visible * f64::from(PPQ)
                    {
                        continue;
                    }
                    let lookback = (clip.loop_len_beats * f64::from(PPQ)).ceil() as u64;
                    let lookback = lookback.saturating_add(1);
                    let from = song_tick.saturating_sub(lookback).max(clip_start);
                    let playback = playback_clip(clip)?;
                    for (tick, event) in playback.events_in_song_range(from, end_tick) {
                        if let Event::Automation { value, .. } = event {
                            valid_value(*value)?;
                            order += 1;
                            let current = latest.entry(target).or_insert((tick, order, *value));
                            if (tick, order) >= (current.0, current.1) {
                                *current = (tick, order, *value);
                            }
                        }
                    }
                }
            }
        }
        // The sends are the mixer's, not the playback's.
        let playback = |target: &AutomationTarget| {
            target.instance_id != 0 || send_param(target.parameter_index).is_none()
        };
        initial.retain(|(target, _)| playback(target));
        latest.retain(|target, _| playback(target));
        for (target, value) in initial {
            self.audio
                .set_target(target, value)
                .map_err(SongAudioError::Playback)?;
        }
        for (target, (_, _, value)) in latest {
            self.audio
                .set_target(target, value)
                .map_err(SongAudioError::Playback)?;
        }
        Ok(())
    }

    pub fn collect_changes(
        &self,
        song: &Song,
        context: ProcessContext,
        frames: usize,
    ) -> Result<Vec<AutomationChange>, SongAudioError> {
        validate_context(context)?;
        if frames == 0 {
            return Ok(Vec::new());
        }
        let sample_rate = f64::from(context.sample_rate);
        let tempo = f64::from(context.tempo_bpm);
        let ticks_per_frame = tempo * f64::from(PPQ) / (60.0 * sample_rate);
        let start_tick = (context.start_beat * f64::from(PPQ) - ticks_per_frame)
            .floor()
            .max(0.0) as u64;
        let end_beat = context.start_beat + frames as f64 * tempo / (60.0 * sample_rate);
        let end_tick = (end_beat * f64::from(PPQ)).ceil();
        if !end_tick.is_finite() || end_tick >= u64::MAX as f64 {
            return Err(SongAudioError::InvalidTiming);
        }
        let mut scheduled = Vec::new();
        for (channel_index, channel) in song.channels.iter().enumerate() {
            for lane in &channel.lanes {
                if lane.kind != LaneKind::Automation {
                    continue;
                }
                let Some(target) = lane
                    .target
                    .and_then(|target| self.audio_target(song, channel_index, target))
                else {
                    continue;
                };
                for clip in playing_clips(lane) {
                    validate_clip(clip)?;
                    let playback = playback_clip(clip)?;
                    for (tick, event) in playback.events_in_song_range(start_tick, end_tick as u64)
                    {
                        if let Event::Automation { value, .. } = event {
                            valid_value(*value)?;
                            let frame = ((tick as f64 / f64::from(PPQ) - context.start_beat)
                                * 60.0
                                * sample_rate
                                / tempo)
                                .ceil();
                            if frame >= 0.0 && frame < frames as f64 {
                                scheduled.push((
                                    tick,
                                    AutomationChange {
                                        frame: frame as usize,
                                        target,
                                        value: *value,
                                    },
                                ));
                            }
                        }
                    }
                }
            }
        }
        // The sort is stable. A tie at a tick (a point on the loop end and the next pass's content 0) is already in
        // dispatch order: `Clip::events_in_song_range` yields the loop end first, so the lower content position lands
        // last and wins, as `honeydew_audio::automation` dispatches one. (Overlapping clips are undefined.)
        scheduled.sort_by(|a, b| a.1.frame.cmp(&b.1.frame).then(a.0.cmp(&b.0)));
        Ok(scheduled.into_iter().map(|(_, change)| change).collect())
    }

    pub fn process_block(
        &mut self,
        buffers: &mut [StereoBlock<'_>],
        context: ProcessContext,
        changes: &[AutomationChange],
    ) -> Result<(), SongAudioError> {
        self.audio
            .process_block(buffers, context, changes)
            .map_err(SongAudioError::Playback)
    }

    pub fn note_events_in_block(
        song: &Song,
        channel_index: usize,
        start_beat: f64,
        end_beat: f64,
    ) -> Result<Vec<InputNoteEvent>, SongAudioError> {
        if !start_beat.is_finite()
            || !end_beat.is_finite()
            || start_beat < 0.0
            || end_beat <= start_beat
            || end_beat * f64::from(PPQ) >= u64::MAX as f64
        {
            return Err(SongAudioError::InvalidTiming);
        }
        let channel = song
            .channels
            .get(channel_index)
            .ok_or(SongAudioError::InvalidChannel)?;
        let mut events = Vec::new();
        for lane in &channel.lanes {
            if lane.kind != LaneKind::Notes {
                continue;
            }
            for clip in playing_clips(lane) {
                validate_clip(clip)?;
                let mut max_length = 0.0_f64;
                for event in &clip.events {
                    if let Event::Note { len_beats, .. } = event {
                        max_length = max_length.max(*len_beats);
                    }
                }
                // Each note-off is clamped to the clip's length, so a note longer than its clip (however long) ends
                // with it, and no note reaches back further than the clip.
                let visible = clip.visible_ticks() / f64::from(PPQ);
                let clip_end = f64::from(clip.start_tick) / f64::from(PPQ) + visible;
                let back = max_length.min(visible);
                let from = ((start_beat - back).max(0.0) * f64::from(PPQ)).floor() as u64;
                let to = (end_beat * f64::from(PPQ)).ceil() as u64;
                for (tick, event) in clip.events_in_song_range(from, to) {
                    // A slide note sends no note-on or note-off: slide events instead. Nor does a note of length ≤ 0 (a
                    // key held at Stop while recording; its off would sort first and leave it on) or one whose length
                    // is not finite (in memory only: a file's reads as 0): it is skipped, and the song plays on.
                    if let Event::Note {
                        len_beats,
                        key,
                        velocity,
                        pan,
                        slide: false,
                        ..
                    } = event
                        && *len_beats > 0.0
                        && len_beats.is_finite()
                    {
                        let on = tick as f64 / f64::from(PPQ);
                        let off = (on + len_beats).min(clip_end);
                        // A fractional offset or loop can round a play kept inside the clip onto its end: its clamped
                        // off then falls on its on, and the note is skipped.
                        if off <= on {
                            continue;
                        }
                        if (start_beat..end_beat).contains(&on) {
                            // The byte itself: instruments get a/255. A 0 still plays.
                            events.push(
                                InputNoteEvent::on(on, i16::from(*key), (*velocity).max(1))
                                    .with_pan(f32::from(*pan) / 255.0),
                            );
                        }
                        if (start_beat..end_beat).contains(&off) {
                            events.push(InputNoteEvent::off(off, i16::from(*key)));
                        }
                    }
                }
            }
        }
        sort_note_events(&mut events);
        Ok(events)
    }

    fn audio_target(
        &self,
        song: &Song,
        channel_index: usize,
        target: SongTarget,
    ) -> Option<AutomationTarget> {
        if target.instance_id == 0 {
            // Every strip param plays, the MIDI CCs (7 + n) included, though the song stores only 0..=5; and so do
            // the sends' Enable and Level, which the engine's mixer holds.
            if send_param(target.param).is_none() {
                self.audio
                    .channel_parameter(channel_index, usize::try_from(target.param).ok()?)?;
            }
        } else {
            song.channels.get(channel_index)?.param(target)?;
            let effect = self.audio.effect(channel_index, target.instance_id)?;
            if target.param >= 0 {
                effect.parameter(usize::try_from(target.param).ok()?)?;
            } else if !(-3..=-1).contains(&target.param) {
                return None;
            }
        }
        Some(AutomationTarget {
            channel: channel_index,
            instance_id: target.instance_id,
            parameter_index: target.param,
        })
    }
}

/// The clips of `lane` that play. A muted clip (`CLHd` @36) makes no events on any lane kind: notes, automation
/// (a muted automation clip does not automate), drum hits and audio. Nor does a clip without a loop
/// ([`Clip::loop_ticks`]: shorter than one tick). Every lane reader goes through here.
pub(crate) fn playing_clips(lane: &Lane) -> impl Iterator<Item = &Clip> {
    (lane.clips.iter()).filter(|clip| !clip.muted && clip.loop_ticks().is_some())
}

fn valid_value(value: f32) -> Result<(), SongAudioError> {
    if value.is_finite() && (0.0..=1.0).contains(&value) {
        Ok(())
    } else {
        Err(SongAudioError::InvalidValue)
    }
}

fn playback_clip(clip: &Clip) -> Result<Clip, SongAudioError> {
    validate_clip(clip)?;
    let mut points = Vec::new();
    for event in &clip.events {
        if let Event::Automation {
            pos,
            value,
            control: true,
        } = event
        {
            valid_value(*value)?;
            points.push(AutomationPoint {
                tick: u64::from(*pos),
                value: (f64::from(*value) * 65535.0).round() as u16,
            });
        }
    }
    if points.is_empty() {
        return Ok(clip.clone());
    }
    let generated =
        AutomationClip::new(0, 1, 1, 0, 1, points).map_err(|_| SongAudioError::InvalidValue)?;
    let mut playback = clip.clone();
    playback.events = generated
        .events()
        .iter()
        .map(|event| {
            Ok(Event::Automation {
                pos: u32::try_from(event.tick).map_err(|_| SongAudioError::InvalidValue)?,
                value: event.normalized_value(),
                control: event.key > 0,
            })
        })
        .collect::<Result<_, _>>()?;
    Ok(playback)
}

/// The clip's visible length in beats ([`Clip::visible_ticks`]: a length ≤ 0 shows the source, else the loop).
fn validate_clip(clip: &Clip) -> Result<f64, SongAudioError> {
    let visible = clip.visible_ticks() / f64::from(PPQ);
    let end = f64::from(clip.start_tick) + visible * f64::from(PPQ);
    let finite = [visible, clip.loop_len_beats, clip.offset_beats];
    if !finite.iter().all(|value| value.is_finite())
        || visible <= 0.0
        || end >= u64::MAX as f64
        || clip.loop_ticks().is_none()
        || clip.loop_len_beats * f64::from(PPQ) >= u64::MAX as f64
    {
        return Err(SongAudioError::InvalidTiming);
    }
    Ok(visible)
}

fn validate_context(context: ProcessContext) -> Result<(), SongAudioError> {
    if context.sample_rate.is_finite()
        && context.sample_rate > 0.0
        && context.tempo_bpm.is_finite()
        && context.tempo_bpm > 0.0
        && context.start_beat.is_finite()
        && context.start_beat >= 0.0
    {
        Ok(())
    } else {
        Err(SongAudioError::InvalidTiming)
    }
}
