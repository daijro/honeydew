//! Audio clip lanes: each clip's `CLSm` slot is one pad of its channel's own [`PadSampler`], so Mute and Solo stay
//! among that channel's clip slots. The clips' starts, loop repetitions, ends and envelope steps are prepared actions;
//! The seek rule relaunches the clips under the playhead.

use honeydew_audio::drums::{DEFAULTS, Pad, PadSampler, SPEED};
use honeydew_audio::mixer::OwnedStereoBuffer;
use honeydew_audio::sample_edit::speed_ratio;

use super::EngineError;
use super::prepare::{Action, ScheduledAction, automation_clip, beat_frame};
use crate::audio_bridge::playing_clips;
use crate::sample_cache::SampleCache;
use crate::song::{Channel, Clip, Event, LaneKind, PPQ, SamplerSlot, Song};

/// The envelopes a clip plays: `env_param` 0, 1, 2 (file keys ±1, ±2, ±3) set the slot's Level, Pan and Speed, which
/// are slot params 0, 1, 2. Other envelopes are ignored.
const ENVELOPES: [i16; 3] = [0, 1, 2];

/// A channel's audio clips as the engine plays them.
pub(super) struct Clips {
    pub sampler: PadSampler,
    /// Per pad, its clip's timing and envelopes, for the seek rule.
    spans: Vec<Span>,
    /// The channel's first lane is audio: the seek rule cuts its clip voices first.
    cut: bool,
}

struct Span {
    /// The clip's start and end (start + visible length), as frames.
    start: u64,
    end: u64,
    /// Its start, loop length and content offset (`CLHd`@16), in beats.
    start_beat: f64,
    loop_beats: f64,
    offset: f64,
    /// Per envelope, its steps (content tick, value) in tick order.
    envelopes: [Vec<(u32, f32)>; 3],
}

impl Clips {
    /// Adds one block of every clip slot into the channel's buffer, before its rack runs.
    pub fn render(&mut self, buffer: &mut OwnedStereoBuffer) {
        for pad in 0..self.sampler.pads() {
            self.sampler
                .process(pad, &mut buffer.left, &mut buffer.right);
        }
    }

    /// The seek rule at `frame`, song beat `beat`, while playing: on a channel whose first lane is audio every clip
    /// voice is cut; then each clip with start < frame < end relaunches from content beat
    /// c = ((beat − start) mod L) + offset (less L once if ≥ L), source time c·60/BPM, after its envelopes take the
    /// value of their last step strictly before c (none: the slot keeps its value). A clip starting at `frame` starts
    /// through its own action.
    pub fn relaunch(&mut self, frame: u64, beat: f64, tempo_bpm: f32) {
        if self.cut {
            self.sampler.reset();
        }
        for (pad, span) in self.spans.iter().enumerate() {
            if !(span.start < frame && frame < span.end) {
                continue;
            }
            let mut c = (beat - span.start_beat).rem_euclid(span.loop_beats) + span.offset;
            if c >= span.loop_beats {
                c -= span.loop_beats;
            }
            for (index, steps) in span.envelopes.iter().enumerate() {
                let before = steps.partition_point(|&(pos, _)| f64::from(pos) < c * f64::from(PPQ));
                if let Some(&(_, value)) = before.checked_sub(1).and_then(|i| steps.get(i)) {
                    self.sampler.set_param(pad, index, value);
                }
            }
            self.sampler
                .hit_at(pad, 1.0, 0.5, 1.0, c * 60.0 / f64::from(tempo_bpm));
        }
    }

    /// Moves each clip's span to frame `at(frame)`: the frames of another tempo (an edit's engine taking these clips
    /// over at the song's new tempo).
    pub fn retime(&mut self, at: impl Fn(u64) -> u64) {
        for span in &mut self.spans {
            (span.start, span.end) = (at(span.start), at(span.end));
        }
    }

    /// These clips play the sample data `old` did, pad by pad, or none on both
    /// ([`honeydew_audio::drums::PadSample::same_data`]).
    pub fn same_samples(&self, old: &Clips) -> bool {
        let (a, b) = (&self.sampler, &old.sampler);
        a.pads() == b.pads()
            && (0..a.pads()).all(|pad| match (a.sample(pad), b.sample(pad)) {
                (Some(new), Some(old)) => new.same_data(old),
                (new, old) => new.is_none() && old.is_none(),
            })
    }

    /// Fast-fades every clip voice: Stop. Stop otherwise only releases notes, and a clip voice's release with
    /// The default EG R (τ 5.67 s) would play on for many seconds after Stop.
    pub fn fade_all(&mut self) {
        for pad in 0..self.sampler.pads() {
            self.sampler.fade(pad);
        }
    }
}

/// The clips of `channel`'s audio lanes that play a sample, in pad order: the unmuted ones with a loop
/// ([`playing_clips`]) and a `CLSm` slot.
fn clips(channel: &Channel) -> impl Iterator<Item = (&Clip, &SamplerSlot)> {
    let lanes = channel.lanes.iter().filter(|l| l.kind == LaneKind::Audio);
    lanes
        .flat_map(playing_clips)
        .filter_map(|clip| Some((clip, clip.file.sample.as_deref()?)))
}

/// Envelope `env_param` of `clip` as an automation clip of the same timing: its control points and the steps between
/// them.
fn envelope(clip: &Clip, env_param: i16) -> Result<Clip, EngineError> {
    let events = (clip.events.iter())
        .filter_map(|event| match *event {
            Event::Envelope {
                pos,
                env_param: k,
                value,
                control: true,
            } if k == env_param => Some(Event::Automation {
                pos,
                value,
                control: true,
            }),
            _ => None,
        })
        .collect();
    automation_clip(&Clip {
        events,
        ..clip.clone()
    })
}

/// The actions one loop pass of an audio clip adds, at most: its envelope steps, and a repetition's fade and start.
pub(super) fn plays_per_pass(clip: &Clip) -> Result<usize, EngineError> {
    let mut plays = 2;
    for env_param in ENVELOPES {
        plays += envelope(clip, env_param)?.events.len();
    }
    Ok(plays)
}

/// The clips of channel `channel`, `None` without any. A slot plays what `samples` holds for it, its `STR1` stretch
/// (the ratio while the mode, SYNC, is on) and pitch rendered once at load (`Refs::add_slot`, the pads' render). Its
/// params apply as on a pad, except the cut group and the beat offset; live notes never play it.
/// The channel's Pitch Bend does not reach it either (only the drum pads apply 2^w).
pub(super) fn build(
    song: &Song,
    channel: usize,
    sample_rate: f32,
    tempo_bpm: f32,
    samples: &SampleCache,
) -> Result<Option<Clips>, EngineError> {
    let channel = &song.channels[channel];
    let (mut pads, mut params, mut spans) = (Vec::new(), Vec::new(), Vec::new());
    for (clip, slot) in clips(channel) {
        let file = &slot.file;
        pads.push(Pad {
            sample: samples.pad_sample(slot),
            params: DEFAULTS,
            note: None,
            reverse: file.reverse,
            cut_group: 0,
            overlap: file.overlapping_voices,
        });
        params.push(slot.params);
        let start_beat = f64::from(clip.start_tick) / f64::from(PPQ);
        let end_beat = start_beat + clip.visible_ticks() / f64::from(PPQ);
        let mut envelopes: [Vec<(u32, f32)>; 3] = Default::default();
        for (steps, env_param) in envelopes.iter_mut().zip(ENVELOPES) {
            *steps = (envelope(clip, env_param)?.events.iter())
                .filter_map(|event| match *event {
                    Event::Automation { pos, value, .. } => Some((pos, value)),
                    _ => None,
                })
                .collect();
        }
        spans.push(Span {
            start: beat_frame(start_beat, sample_rate, tempo_bpm)?,
            end: beat_frame(end_beat, sample_rate, tempo_bpm)?,
            start_beat,
            loop_beats: clip.loop_len_beats,
            offset: clip.offset_beats,
            envelopes,
        });
    }
    if pads.is_empty() {
        return Ok(None);
    }
    // The slots' own params through the sampler's checked write, as the pads' (a hostile value never reaches a voice).
    let mut sampler = PadSampler::new(pads, sample_rate);
    for (pad, params) in params.into_iter().enumerate() {
        for (index, value) in params.into_iter().enumerate() {
            sampler.set_param(pad, index, value);
        }
    }
    Ok(Some(Clips {
        sampler,
        spans,
        cut: channel
            .lanes
            .first()
            .is_some_and(|l| l.kind == LaneKind::Audio),
    }))
}

/// `CLHd`@28, an audio clip's source length in beats: S = BPM × frames / (rate × 60 × the Speed ratio), with `frames`
/// and `rate` those of the sample after the load-time stretch and pitch, and the Speed ratio
/// 2^(trunc(48·Speed − 24)/12).
pub fn source_beats(frames: usize, rate: u32, speed: f32, bpm: f64) -> f64 {
    bpm * frames as f64 / (f64::from(rate) * 60.0 * speed_ratio(speed))
}

/// Recomputes `clip`'s `CLHd`@28 ([`source_beats`]) and @8 ([`Clip::fit_loop`] with `b` beats a bar) from its slot's
/// sample as the engine renders it. Call it when the clip updates (a new sample, Speed, `STR1` pitch or SYNC, a tempo
/// change), once `samples` has loaded the song; a stored value is not kept. True when either changed. Without a
/// `CLSm` slot, a loaded sample or a finite positive length, nothing changes.
pub fn fit_audio_clip(clip: &mut Clip, samples: &SampleCache, bpm: f64, b: u32) -> bool {
    let Some(slot) = clip.file.sample.as_deref() else {
        return false;
    };
    let Some(sample) = samples.pad_sample(slot) else {
        return false;
    };
    let beats = source_beats(
        sample.frames(),
        sample.sample_rate(),
        slot.params[SPEED],
        bpm,
    );
    if !(beats.is_finite() && beats > 0.0) {
        return false;
    }
    let before = (clip.file.source_len_beats, clip.loop_len_beats);
    clip.file.source_len_beats = beats;
    clip.fit_loop(b);
    before != (clip.file.source_len_beats, clip.loop_len_beats)
}

/// The actions of `channel`'s audio clips, pad by pad as [`build`] numbers them:
/// - at the start, a hit from source time offset·60/BPM (vel 1, centred; Speed does not scale it);
/// - at start − offset + n·L (n ≥ 1) inside [start, end), a fade then a hit from source time 0;
/// - at the end (start + visible length), a fade;
/// - each envelope step, repeated with the content loop inside the visible length.
pub(super) fn actions(
    song: &Song,
    channel: usize,
    sample_rate: f32,
    tempo_bpm: f32,
    actions: &mut Vec<ScheduledAction>,
) -> Result<(), EngineError> {
    let ppq = f64::from(PPQ);
    for (pad, (clip, _)) in clips(&song.channels[channel]).enumerate() {
        let mut push = |beat: f64, action| -> Result<(), EngineError> {
            let frame = beat_frame(beat, sample_rate, tempo_bpm)?;
            actions.push(ScheduledAction {
                frame,
                channel,
                action,
            });
            Ok(())
        };
        let (start, lp, offset) = (
            f64::from(clip.start_tick) / ppq,
            clip.loop_len_beats,
            clip.offset_beats,
        );
        let end = start + clip.visible_ticks() / ppq;
        push(
            start,
            Action::ClipHit {
                pad,
                offset: offset * 60.0 / f64::from(tempo_bpm),
            },
        )?;
        // At most one repetition a loop pass; `plays` bounded the passes.
        let (first, passes) = ((offset / lp).ceil().max(1.0), (end - start) / lp + 1.0);
        for n in 0..passes as u64 {
            let at = start - offset + (first + n as f64) * lp;
            if at >= end {
                break;
            }
            if at >= start {
                push(at, Action::ClipFade { pad })?;
                push(at, Action::ClipHit { pad, offset: 0.0 })?;
            }
        }
        push(end, Action::ClipFade { pad })?;
        for (index, env_param) in ENVELOPES.into_iter().enumerate() {
            let steps = envelope(clip, env_param)?;
            for (tick, event) in steps.events_in_song_range(0, u64::MAX) {
                if let Event::Automation { value, .. } = *event {
                    let action = Action::ClipParam { pad, index, value };
                    push(tick as f64 / ppq, action)?;
                }
            }
        }
    }
    Ok(())
}
