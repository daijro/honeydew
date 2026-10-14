//! Preparing a song for playback: every scheduled action from tick 0 to the song's end, with one function per lane
//! kind (Notes, Drums in `drums.rs`; Audio clips in `audio_clips.rs`), and the per-param timelines that seek
//! reads.

use std::collections::{BTreeMap, HashSet};

use super::{EngineError, strip_param};
use crate::audio_bridge::{SongAudioBridge, playing_clips};
use crate::panels::couple_scale;
use crate::params::snap_echo_delay;
use crate::song::{Clip, Event, Lane, LaneKind, ModuleInstance, PPQ, Song};
use honeydew_audio::ProcessContext;
use honeydew_audio::automation::{AutomationClip, AutomationPoint};
use honeydew_audio::instrument::InstrumentModule;
use honeydew_audio::note_effects::{NoteEffectModule, NoteEffectRack, NoteEffectTrace};
use honeydew_audio::sequencer::{InputNoteEvent, NoteKind};

/// The note actions' instance for the channel's pad sampler (instruments have ids ≥ 2).
pub(super) const KIT: i32 = 0;

#[derive(Clone, Copy)]
pub(super) enum Action {
    /// For the channel's instrument `instance_id` only, or its pad sampler for [`KIT`]: each has its own note list.
    NoteOn {
        instance_id: i32,
        key: u8,
        velocity: f32,
        pan: f32,
    },
    NoteOff {
        instance_id: i32,
        key: u8,
    },
    Param {
        instance_id: i32,
        index: i32,
        value: f32,
    },
    /// A drum hit on pad `pad` of the channel's kit: velocity and pan in 0..=1, pitch a ratio. `held`: a drum
    /// notes lane's hit, which a [`Action::PadOff`] releases.
    Hit {
        pad: usize,
        velocity: f32,
        pan: f32,
        pitch: f32,
        held: bool,
    },
    /// A drum notes lane's note-off on pad `pad`.
    PadOff {
        pad: usize,
    },
    /// A step lane's clip covers pad `pad` from its first hit on it (`on`) to the clip's end: what a position re-sync
    /// reads, since step hits have no length. It plays nothing.
    StepCover {
        pad: usize,
        on: bool,
    },
    /// A note-on (`on`) or note-off of `key` at the channel's input, before its note effects: what its keys light. It
    /// plays nothing.
    Key {
        key: u8,
        on: bool,
    },
    /// Audio clip slot `pad` of the channel plays from source time `offset` seconds, vel 1, centred.
    ClipHit {
        pad: usize,
        offset: f64,
    },
    /// Audio clip slot `pad`'s voices fast-fade.
    ClipFade {
        pad: usize,
    },
    /// A slide event for every instrument of the channel: the offsets of every voice. `key` is the slide note's, for
    /// The key range.
    Slide {
        key: u8,
        pitch: f32,
        pan: f32,
        velocity: f32,
    },
    /// An envelope step sets audio clip slot `pad`'s param `index` (Level, Pan, Speed). By pad, not by slot id: a
    /// `CLSm`'s `LINk` id is ignored and given anew on load, so no −id names a clip slot.
    ClipParam {
        pad: usize,
        index: usize,
        value: f32,
    },
}

#[derive(Clone, Copy)]
pub(super) struct ScheduledAction {
    pub frame: u64,
    pub channel: usize,
    pub action: Action,
}

pub(super) struct ParamTimeline {
    pub channel: usize,
    pub instance_id: i32,
    pub index: i32,
    pub changes: Vec<(u64, f32)>,
}

/// The most event plays a song's clips may expand to: the file reader's memory budget in events, the rule of
/// `ui::clips::can_unlink`.
pub(super) const MAX_PLAYS: usize = crate::flm::BUDGET_BASE / std::mem::size_of::<Event>();

/// Every action of `song`, sorted by frame. `instruments` holds each channel's instruments by instance id. A song whose
/// loop passes would expand to more than [`MAX_PLAYS`] event plays is refused first (`TooManyEvents`). Each note effect
/// that ran leaves its trace in `traces`, with its channel.
pub(super) fn actions(
    song: &Song,
    bridge: &SongAudioBridge,
    instruments: &[Vec<(i32, InstrumentModule)>],
    sample_rate: f32,
    tempo_bpm: f32,
    traces: &mut Vec<(usize, NoteEffectTrace)>,
) -> Result<Vec<ScheduledAction>, EngineError> {
    if plays(song)? > MAX_PLAYS as f64 {
        return Err(EngineError::TooManyEvents);
    }
    let end_beat = song_end_beat(song)?;
    let end_frame = beat_frame(end_beat, sample_rate, tempo_bpm)?;
    let changes = bridge
        .collect_changes(
            song,
            ProcessContext {
                sample_rate,
                tempo_bpm,
                start_beat: 0.0,
                playing: false,
            },
            usize::try_from(end_frame).map_err(|_| EngineError::InvalidTiming)?,
        )
        .map_err(EngineError::Song)?;
    let mut actions: Vec<_> = changes
        .into_iter()
        .map(|change| ScheduledAction {
            frame: change.frame as u64,
            channel: change.target.channel,
            action: Action::Param {
                instance_id: change.target.instance_id,
                index: strip_param(change.target.instance_id, change.target.parameter_index),
                value: change.value,
            },
        })
        .collect();
    for (channel, instruments) in instruments.iter().enumerate() {
        let kit = (song.channels[channel].drum_kit.as_ref()).filter(|kit| !kit.slots.is_empty());
        if let Some(kit) = kit {
            // Slot automation (`inst < 0`) first, so a step at a note's or hit's frame applies before it.
            instrument_automation(
                song,
                channel,
                |id| id < 0 && kit.slot_by_id(id.unsigned_abs()).is_some(),
                end_beat,
                sample_rate,
                tempo_bpm,
                &mut actions,
            )?;
        }
        // A channel's pad sampler plays its notes too.
        if !instruments.is_empty() || kit.is_some() {
            notes(
                song,
                channel,
                instruments,
                end_beat,
                sample_rate,
                tempo_bpm,
                &mut actions,
                traces,
            )?;
        }
        if !instruments.is_empty() {
            instrument_automation(
                song,
                channel,
                |id| instruments.iter().any(|(instrument, _)| *instrument == id),
                end_beat,
                sample_rate,
                tempo_bpm,
                &mut actions,
            )?;
        }
        if kit.is_some() {
            super::drums::hits(
                song,
                channel,
                end_beat,
                sample_rate,
                tempo_bpm,
                &mut actions,
            )?;
        }
        super::audio_clips::actions(song, channel, sample_rate, tempo_bpm, &mut actions)?;
        if !instruments.is_empty() {
            super::slides::actions(
                song,
                channel,
                end_beat,
                sample_rate,
                tempo_bpm,
                &mut actions,
            )?;
        }
    }
    actions.sort_by_key(|action| action.frame);
    // A param changed several times at one frame gets only its last value (a Pitcher 2 Low Latency switch rebuilds).
    let mut later = HashSet::new();
    actions.reverse();
    actions.retain(|event| match event.action {
        Action::Param {
            instance_id, index, ..
        } => later.insert((event.frame, event.channel, instance_id, index)),
        _ => true,
    });
    actions.reverse();
    Ok(actions)
}

/// The Notes lanes of `channel` as note on/off actions: unchanged for its pad sampler, which renders before every
/// module, and for each of its `instruments` through the note effects above it. The note effects rewrite the event list
/// top to bottom, and each instrument gets the list as it stands at its own rack position, so a note effect acts only
/// on the instruments below it.
#[allow(
    clippy::too_many_arguments,
    reason = "the song's timing and both outputs"
)]
fn notes(
    song: &Song,
    channel: usize,
    instruments: &[(i32, InstrumentModule)],
    end_beat: f64,
    sample_rate: f32,
    tempo_bpm: f32,
    actions: &mut Vec<ScheduledAction>,
    traces: &mut Vec<(usize, NoteEffectTrace)>,
) -> Result<(), EngineError> {
    let input = SongAudioBridge::note_events_in_block(song, channel, 0.0, end_beat)
        .map_err(EngineError::Song)?;
    for note in &input {
        actions.push(ScheduledAction {
            frame: beat_frame(note.beat, sample_rate, tempo_bpm)?,
            channel,
            action: Action::Key {
                key: u8::try_from(note.pitch).map_err(|_| EngineError::InvalidTiming)?,
                on: note.kind == NoteKind::On,
            },
        });
    }
    let kit = song.channels[channel].drum_kit.as_ref();
    if kit.is_some_and(|kit| !kit.slots.is_empty()) {
        note_actions(&input, KIT, channel, sample_rate, tempo_bpm, actions)?;
    }
    let mut effects = Vec::new();
    // The list after the first `stage.0` note effects.
    let mut stage = (usize::MAX, Vec::new());
    for module in &song.channels[channel].rack.modules {
        if (47..=51).contains(&module.type_id) {
            effects.push(note_effect(song, channel, module, end_beat)?);
            continue;
        }
        let instance_id = module.instance_id;
        if !instruments.iter().any(|(id, _)| *id == instance_id) {
            continue;
        }
        if stage.0 != effects.len() {
            let mut rack = NoteEffectRack::new(effects.clone()).map_err(EngineError::Notes)?;
            rack.bar_beats = f64::from(song.time_sig.bar_beats());
            rack.sample_rate = f64::from(sample_rate);
            rack.block_frames = super::BLOCK_FRAMES as u64;
            let mut traced = Vec::new();
            let events = rack
                .process_events_traced(0.0, end_beat, f64::from(tempo_bpm), &input, &mut traced)
                .map_err(EngineError::Notes)?;
            stage = (effects.len(), events);
            // A later stage runs the earlier note effects again, alike: their first traces stand.
            for trace in traced {
                let id = trace_id(&trace);
                if !traces
                    .iter()
                    .any(|(c, t)| *c == channel && trace_id(t) == id)
                {
                    traces.push((channel, trace));
                }
            }
        }
        note_actions(
            &stage.1,
            instance_id,
            channel,
            sample_rate,
            tempo_bpm,
            actions,
        )?;
    }
    Ok(())
}

/// The note effect a trace is of.
pub(super) fn trace_id(trace: &NoteEffectTrace) -> i32 {
    match trace {
        NoteEffectTrace::Arp { instance_id, .. } | NoteEffectTrace::Echo { instance_id, .. } => {
            *instance_id
        }
    }
}

/// `notes` as note on/off actions for `instance_id` of `channel`: velocity a/255, pan b/255.
fn note_actions(
    notes: &[InputNoteEvent],
    instance_id: i32,
    channel: usize,
    sample_rate: f32,
    tempo_bpm: f32,
    actions: &mut Vec<ScheduledAction>,
) -> Result<(), EngineError> {
    for note in notes {
        let frame = beat_frame(note.beat, sample_rate, tempo_bpm)?;
        let key = u8::try_from(note.pitch).map_err(|_| EngineError::InvalidTiming)?;
        actions.push(ScheduledAction {
            frame,
            channel,
            action: match note.kind {
                NoteKind::On => Action::NoteOn {
                    instance_id,
                    key,
                    velocity: f32::from(note.velocity) / 255.0,
                    pan: note.pan,
                },
                NoteKind::Off => Action::NoteOff { instance_id, key },
            },
        });
    }
    Ok(())
}

/// Note effect `module` of `channel` with its automation lanes. Its params are coupled as its panel couples edits, for
/// The saved values (a preset) and every automated write alike: Note Echo stores `Delay (sync)` (p3) snapped while Sync
/// (p4) is On, and a write of Scale's Key or Scale rewrites its notes.
pub(super) fn note_effect(
    song: &Song,
    channel: usize,
    module: &ModuleInstance,
    end_beat: f64,
) -> Result<NoteEffectModule, EngineError> {
    let mut writes = Vec::new();
    for lane in &song.channels[channel].lanes {
        let target = lane.target.filter(|target| {
            lane.kind == LaneKind::Automation && target.instance_id == module.instance_id
        });
        let index = target.and_then(|target| usize::try_from(target.param).ok());
        let Some(index) = index.filter(|&index| index < module.params.len()) else {
            continue;
        };
        let points = lane_points(lane, end_beat)?.into_iter();
        writes.extend(points.map(|(tick, value)| (tick, index, value)));
    }
    writes.sort_by_key(|write| write.0);
    let mut params = module.params.clone();
    couple(module.type_id, &mut params, &[]);
    let initial = params.clone();
    let mut changes = Vec::new();
    for (tick, index, value) in writes {
        let before = params.clone();
        params[index] = value;
        couple(module.type_id, &mut params, &[index]);
        let beat = tick as f64 / f64::from(PPQ);
        let changed = (0..params.len()).filter(|&i| params[i] != before[i]);
        changes.extend(changed.map(|i| (beat, i, params[i])));
    }
    Ok(NoteEffectModule {
        instance_id: module.instance_id,
        type_id: module.type_id,
        params: initial,
        changes,
    })
}

/// The panel couplings that shape a note effect's sound, after a write of params `edited` (none: the saved values).
fn couple(type_id: u16, params: &mut [f32], edited: &[usize]) {
    match type_id {
        // Sync is On above 0.5.
        47 if params.get(4).is_some_and(|&sync| sync > 0.5) => {
            params[3] = snap_echo_delay(params[3]);
        }
        50 => {
            couple_scale(params, edited);
        }
        _ => {}
    }
}

/// The automation lanes of `channel` whose target instance `plays` (an instrument, or a drum slot): params and common
/// params alike.
fn instrument_automation(
    song: &Song,
    channel: usize,
    plays: impl Fn(i32) -> bool,
    end_beat: f64,
    sample_rate: f32,
    tempo_bpm: f32,
    actions: &mut Vec<ScheduledAction>,
) -> Result<(), EngineError> {
    for lane in &song.channels[channel].lanes {
        if lane.kind != LaneKind::Automation {
            continue;
        }
        let Some(target) = lane.target else { continue };
        if !plays(target.instance_id) {
            continue;
        }
        for (tick, value) in lane_points(lane, end_beat)? {
            actions.push(ScheduledAction {
                frame: beat_frame(tick as f64 / f64::from(PPQ), sample_rate, tempo_bpm)?,
                channel,
                action: Action::Param {
                    instance_id: target.instance_id,
                    index: target.param,
                    value,
                },
            });
        }
    }
    Ok(())
}

/// The values automation `lane` plays up to `end_beat`, as (song tick, value), clip by clip.
fn lane_points(lane: &Lane, end_beat: f64) -> Result<Vec<(u64, f32)>, EngineError> {
    let end_tick = (end_beat * f64::from(PPQ)).ceil() as u64;
    let mut points = Vec::new();
    for clip in playing_clips(lane) {
        let playback = automation_clip(clip)?;
        for (tick, event) in playback.events_in_song_range(0, end_tick) {
            if let Event::Automation { value, .. } = event {
                points.push((tick, *value));
            }
        }
    }
    Ok(points)
}

/// How many event plays the song's playing clips expand to, at most, over all their loop passes (a hostile file: a
/// 1-tick loop shown for 10⁹ beats), counted before any is expanded. A pass may be cut by the offset, hence the + 1.
/// An automation clip counts the events its control points generate.
fn plays(song: &Song) -> Result<f64, EngineError> {
    let mut plays = 0.0;
    for lane in song.channels.iter().flat_map(|channel| &channel.lanes) {
        for clip in playing_clips(lane) {
            let Some(loop_ticks) = clip.loop_ticks() else {
                continue;
            };
            let events = match lane.kind {
                LaneKind::Automation => automation_clip(clip)?.events.len(),
                // A drum hit plays once plus its Repeat hits.
                LaneKind::Drums => (clip.events.iter())
                    .map(|event| match *event {
                        Event::Drum { repeat, .. } => 1 + (repeat as i8).max(0) as usize,
                        _ => 1,
                    })
                    .sum(),
                LaneKind::Audio => super::audio_clips::plays_per_pass(clip)?,
                _ => clip.events.len(),
            };
            // It runs before `song_end_beat` validates the clips: a negative length adds nothing, a NaN one is over.
            let clip_plays = ((clip.visible_ticks() / loop_ticks).ceil() + 1.0) * events as f64;
            plays += if clip_plays.is_nan() {
                f64::INFINITY
            } else {
                clip_plays.max(0.0)
            };
        }
    }
    Ok(plays)
}

/// Each param's changes in frame order, for seek.
pub(super) fn timelines(actions: &[ScheduledAction]) -> Vec<ParamTimeline> {
    let mut timelines = BTreeMap::<(usize, i32, i32), Vec<(u64, f32)>>::new();
    for event in actions {
        if let Action::Param {
            instance_id,
            index,
            value,
        } = event.action
        {
            timelines
                .entry((event.channel, instance_id, index))
                .or_default()
                .push((event.frame, value));
        }
    }
    timelines
        .into_iter()
        .map(|((channel, instance_id, index), changes)| ParamTimeline {
            channel,
            instance_id,
            index,
            changes,
        })
        .collect()
}

/// The beat after which nothing sounds, the horizon of what is prepared: the last clip's end ([`Clip::visible_ticks`],
/// so a length ≤ 0 shows the source), plus one beat; playback wraps at the song length (`SongAudioEngine::wrap_range`).
/// No note or drum hit outlasts its clip, however long: each note-off is clamped to the clip's length (`drums.rs`
/// clamps a hit's). The beat keeps what lands on or just past a clip end inside this exclusive bound: a note-off
/// exactly on the clip end, and a pad-Shifted hit (Shift moves a hit by at most +0.25 beat). Muted clips count: they
/// still show. Clips without a loop do not.
pub(super) fn song_end_beat(song: &Song) -> Result<f64, EngineError> {
    let mut end = 1.0_f64;
    for channel in &song.channels {
        for lane in &channel.lanes {
            // A clip without a loop plays nothing (`Clip::loop_ticks`).
            for clip in lane.clips.iter().filter(|clip| clip.loop_ticks().is_some()) {
                let visible = clip.visible_ticks() / f64::from(PPQ);
                let finite = [visible, clip.loop_len_beats, clip.offset_beats];
                if !finite.iter().all(|value| value.is_finite()) || visible <= 0.0 {
                    return Err(EngineError::InvalidTiming);
                }
                end = end.max(f64::from(clip.start_tick) / f64::from(PPQ) + visible);
            }
        }
    }
    let end = end + 1.0;
    if end.is_finite() {
        Ok(end)
    } else {
        Err(EngineError::InvalidTiming)
    }
}

pub(super) fn automation_clip(clip: &Clip) -> Result<Clip, EngineError> {
    let mut points = Vec::new();
    for event in &clip.events {
        if let Event::Automation {
            pos,
            value,
            control: true,
        } = event
        {
            if !value.is_finite() || !(0.0..=1.0).contains(value) {
                return Err(EngineError::InvalidTiming);
            }
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
        AutomationClip::new(0, 1, 1, 0, 1, points).map_err(|_| EngineError::InvalidTiming)?;
    let mut playback = clip.clone();
    playback.events = generated
        .events()
        .iter()
        .map(|event| {
            Ok(Event::Automation {
                pos: u32::try_from(event.tick).map_err(|_| EngineError::InvalidTiming)?,
                value: event.normalized_value(),
                control: event.key > 0,
            })
        })
        .collect::<Result<_, _>>()?;
    Ok(playback)
}

pub(super) fn beat_frame(beat: f64, sample_rate: f32, tempo_bpm: f32) -> Result<u64, EngineError> {
    let frame = (beat * 60.0 * f64::from(sample_rate) / f64::from(tempo_bpm)).ceil();
    if frame.is_finite() && frame >= 0.0 && frame < u64::MAX as f64 {
        Ok(frame as u64)
    } else {
        Err(EngineError::InvalidTiming)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::song::Lane;

    /// Each note's note-off is clamped to its clip's length, and so is a drum hit's (`drums.rs`). So no note or hit
    /// adds to the song's end, whatever its length: one longer than its clip (however long), of length 0 or negative,
    /// or not finite. The song ends one beat after the clip.
    #[test]
    fn no_note_or_hit_length_moves_the_song_end_past_its_clip() {
        let lengths = [
            3.0,
            1e300,
            0.0,
            -1.0,
            f64::NAN,
            f64::INFINITY,
            f64::NEG_INFINITY,
        ];
        for len_beats in lengths {
            let mut song = Song::default();
            let mut drums = Lane::new(LaneKind::Drums);
            drums.add_clip(0, 1.0).events.push(Event::Drum {
                pos: 0,
                len_beats,
                pad: 0,
                velocity: 204,
                pan: 127,
                value: Event::DEFAULT_PITCH,
                repeat: 0,
            });
            let mut notes = Lane::new(LaneKind::Notes);
            notes.add_clip(0, 1.0).events.push(Event::Note {
                pos: 0,
                len_beats,
                key: 60,
                velocity: 204,
                pan: 127,
                slide: false,
            });
            song.channels[0].lanes.extend([drums, notes]);
            assert_eq!(song_end_beat(&song).unwrap(), 2.0, "{len_beats}");
        }
    }

    /// The pad sampler renders before every module, so a note effect above an instrument changes what that instrument
    /// gets and not what the kit gets.
    #[test]
    fn the_kit_gets_the_notes_before_every_note_effect() {
        use crate::sample_cache::SampleCache;
        use crate::song::ChannelKind;
        let mut song = Song::default();
        song.add_channel(ChannelKind::Drums);
        let rack = &mut song.channels[1].rack;
        let transpose = rack.add(48);
        rack.get_mut(transpose).unwrap().params[1] = 5.0 / 8.0; // +1 octave
        let synth = rack.add(0);
        let lane = song.channels[1].add_lane(LaneKind::Notes);
        song.channels[1].lanes[lane]
            .add_clip(0, 4.0)
            .events
            .push(Event::Note {
                pos: 0,
                len_beats: 1.0,
                key: 48,
                velocity: 255,
                pan: 127,
                slide: false,
            });
        let channel = &song.channels[1];
        let instruments =
            super::super::generators::build(channel, 48e3, 120.0, 64, &SampleCache::default(), 4);
        let mut actions = Vec::new();
        notes(
            &song,
            1,
            &instruments,
            4.0,
            48e3,
            120.0,
            &mut actions,
            &mut Vec::new(),
        )
        .unwrap();
        let notes: Vec<_> = (actions.iter())
            .filter_map(|action| match action.action {
                Action::NoteOn {
                    instance_id, key, ..
                } => Some((instance_id, key, true)),
                Action::NoteOff { instance_id, key } => Some((instance_id, key, false)),
                _ => None,
            })
            .collect();
        assert_eq!(
            notes,
            [
                (KIT, 48, true),
                (KIT, 48, false),
                (synth, 60, true),
                (synth, 60, false)
            ]
        );
    }

    /// Instruments get velocity a/255 and pan b/255, whatever the note effects above.
    #[test]
    fn note_velocity_and_pan_reach_the_instruments_over_255() {
        use crate::sample_cache::SampleCache;
        use crate::song::{ChannelKind, SynthKind};
        let mut song = Song::default();
        song.add_channel(ChannelKind::Synth(SynthKind::MiniSynth));
        // Transpose at +0 above the synth: the note list passes through the note effects.
        song.channels[1].rack.insert(0, 48);
        let clip = song.channels[1].lanes[0].add_clip(0, 4.0);
        for (i, byte) in [255, 127, 1].into_iter().enumerate() {
            clip.events.push(Event::Note {
                pos: i as u32 * PPQ,
                len_beats: 0.5,
                key: 60,
                velocity: byte,
                pan: byte,
                slide: false,
            });
        }
        let channel = &song.channels[1];
        let instruments =
            super::super::generators::build(channel, 48e3, 120.0, 64, &SampleCache::default(), 4);
        let mut actions = Vec::new();
        notes(
            &song,
            1,
            &instruments,
            4.0,
            48e3,
            120.0,
            &mut actions,
            &mut Vec::new(),
        )
        .unwrap();
        let ons: Vec<_> = (actions.iter())
            .filter_map(|action| match action.action {
                Action::NoteOn { velocity, pan, .. } => Some((velocity, pan)),
                _ => None,
            })
            .collect();
        let byte = |b: f32| (b / 255.0, b / 255.0);
        assert_eq!(ons, [byte(255.0), byte(127.0), byte(1.0)]);
        assert!((ons[1].0 - 0.498).abs() < 1e-3);
    }

    /// The Arpeggiator's bar retrigger restarts its sequence every bar of the song's beats per bar (TDIV byte 1).
    #[test]
    fn the_arpeggiator_bar_is_the_songs_beats_per_bar() {
        use crate::sample_cache::SampleCache;
        use crate::song::{ChannelKind, SynthKind};
        let mut song = Song::default();
        song.add_channel(ChannelKind::Synth(SynthKind::MiniSynth));
        // Arpeggiator 1 Up once a beat (Rate 1), Retrig `1 bar`, over C and E held for six beats.
        let arp = song.channels[1].rack.insert(0, 51);
        let params = &mut song.channels[1].rack.get_mut(arp).unwrap().params;
        (params[1], params[10], params[18]) = (4.0 / 7.0, 2.0 / 23.0, 7.0 / 11.0);
        let clip = song.channels[1].lanes[0].add_clip(0, 8.0);
        for key in [60, 64] {
            clip.events.push(Event::Note {
                pos: 0,
                len_beats: 6.0,
                key,
                velocity: 204,
                pan: 127,
                slide: false,
            });
        }
        let keys = |song: &Song| {
            let channel = &song.channels[1];
            let instruments = super::super::generators::build(
                channel,
                48e3,
                120.0,
                64,
                &SampleCache::default(),
                4,
            );
            let mut actions = Vec::new();
            notes(
                song,
                1,
                &instruments,
                8.0,
                48e3,
                120.0,
                &mut actions,
                &mut Vec::new(),
            )
            .unwrap();
            (actions.iter())
                .filter_map(|action| match action.action {
                    Action::NoteOn { key, .. } => Some(key),
                    _ => None,
                })
                .collect::<Vec<_>>()
        };
        assert_eq!(keys(&song), [60, 64, 60, 64, 60, 64]);
        song.time_sig = crate::song::TimeSig {
            subdivision: 4,
            beats_per_bar: 3,
        };
        assert_eq!(keys(&song), [60, 64, 60, 60, 64, 60]);
    }

    /// `plays` runs before `song_end_beat` validates the clips: a broken source length must not slip under the budget.
    #[test]
    fn a_broken_source_length_neither_hides_nor_cancels_plays() {
        let note = Event::Note {
            pos: 0,
            len_beats: 1.0,
            key: 60,
            velocity: 204,
            pan: 127,
            slide: false,
        };
        let mut song = Song::default();
        let mut lane = Lane::new(LaneKind::Notes);
        lane.add_clip(0, 1.0).events.push(note);
        song.channels[0].lanes.push(lane);
        let fair = plays(&song).unwrap();
        assert!(fair > 0.0);
        // A visible length ≤ 0 shows the source length: a negative one adds nothing rather than subtracting.
        let mut broken = song.clone();
        let lane = broken.channels[0].lanes.last_mut().unwrap();
        let clip = lane.add_clip(0, 1.0);
        clip.events.push(note);
        clip.visible_len_beats = 0.0;
        clip.file.source_len_beats = -1e12;
        assert_eq!(plays(&broken).unwrap(), fair);
        // A NaN one is over the budget, not a NaN total that compares false.
        let lane = broken.channels[0].lanes.last_mut().unwrap();
        lane.clips[1].file.source_len_beats = f64::NAN;
        assert!(plays(&broken).unwrap() > MAX_PLAYS as f64);
    }

    /// Note Echo's Sync (p4) is on above 0.5, so 0.5 is off and p3 keeps its unsnapped value. Probes: 0, 0.001, exactly
    /// 0.5, the next f32 above it, and 1.
    #[test]
    fn the_echo_delay_snaps_only_while_sync_is_above_half() {
        // p3 = 0.12 is nearest `1/8`.
        let snapped = snap_echo_delay(0.12);
        assert_ne!(snapped, 0.12);
        for (sync, on) in [
            (0.0, false),
            (0.001, false),
            (0.5, false),
            (f32::from_bits(0.5f32.to_bits() + 1), true),
            (1.0, true),
        ] {
            let mut params = vec![0.5; 16];
            params[3] = 0.12;
            params[4] = sync;
            couple(47, &mut params, &[]);
            assert_eq!(params[3], if on { snapped } else { 0.12 }, "Sync {sync:?}");
        }
    }
}
