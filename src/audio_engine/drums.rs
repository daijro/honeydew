//! A channel's pad sampler: the kit built from the channel's `DrumKit`, and its drum lanes' hits as prepared actions,
//! through the rack's note effects when it has any (issue #40). The engine renders a kit before the
//! channel's instruments, so the rack's modules run on the pads' audio.

use std::collections::VecDeque;

use honeydew_audio::drums::{DEFAULTS, Pad, PadSampler, hit_pitch};
use honeydew_audio::mixer::OwnedStereoBuffer;
use honeydew_audio::note_effects::{NoteEffectModule, NoteEffectTrace};
use honeydew_audio::sequencer::{InputNoteEvent, NoteKind, sort_note_events};

use super::EngineError;
use super::prepare::{Action, ScheduledAction, add_traces, beat_frame, note_effect, note_rack};
use crate::audio_bridge::playing_clips;
use crate::sample_cache::SampleCache;
use crate::song::{DrumGrid, DrumKit, Event, LaneKind, PPQ, Song};

/// A channel's pads as the engine plays them.
pub(super) struct Kit {
    pub sampler: PadSampler,
    /// Its channel is out of the mix (Mute, Solo): the pads that play into another channel are silent too.
    pub out: bool,
    /// Its channel.
    channel: usize,
    /// Per pad, its slot's `LINk` id, which a negative instance names.
    ids: Vec<u32>,
    /// Per pad, the channel whose buffer it plays into: its send target, else its own channel.
    outputs: Vec<usize>,
    /// Per pad, its output's peak (L, R) over the last block: the drum strips' meters.
    peaks: Vec<[f32; 2]>,
    /// One block of one pad, before it is added into its output.
    scratch: OwnedStereoBuffer,
}

impl Kit {
    /// Slot param `index` of the slot with id `id`; an unknown slot or index is ignored.
    pub fn set_slot_param(&mut self, id: u32, index: i32, value: f32) {
        let pad = self.ids.iter().position(|&slot| slot == id);
        if let (Some(pad), Ok(index)) = (pad, usize::try_from(index)) {
            self.sampler.set_param(pad, index, value);
        }
    }

    /// Setting `key` of the slot with id `id` ([`super::settings::SLOT_REVERSE`], [`super::settings::SLOT_OVERLAP`]:
    /// on above 0; [`super::settings::SLOT_CUT_GROUP`]: the group). An unknown slot or key is ignored.
    pub fn set_slot_setting(&mut self, id: u32, key: u32, value: f32) {
        let Some(pad) = self.ids.iter().position(|&slot| slot == id) else {
            return;
        };
        match key {
            super::settings::SLOT_REVERSE => self.sampler.set_reverse(pad, value > 0.0),
            super::settings::SLOT_OVERLAP => self.sampler.set_overlap(pad, value > 0.0),
            super::settings::SLOT_CUT_GROUP => self
                .sampler
                .set_cut_group(pad, value.clamp(0.0, 255.0) as u8),
            _ => {}
        }
    }

    /// Adds one block of every pad into its output channel's buffer, before that channel's rack runs, and keeps its
    /// peak. A pad with a send target (`LINk`@10 ≥ 0) skips its own channel entirely (rack, strip and sends) but for
    /// its channel being out of the mix ([`Self::out`]).
    pub fn render(&mut self, buffers: &mut [OwnedStereoBuffer]) {
        for (pad, &output) in self.outputs.iter().enumerate() {
            let buffer = &mut buffers[output];
            let frames = buffer.left.len().min(self.scratch.left.len());
            let (left, right) = (
                &mut self.scratch.left[..frames],
                &mut self.scratch.right[..frames],
            );
            left.fill(0.0);
            right.fill(0.0);
            self.sampler.process(pad, left, right);
            if self.out && output != self.channel {
                self.peaks[pad] = [0.0; 2];
                continue;
            }
            let mut peak = [0.0_f32; 2];
            let out = buffer.left.iter_mut().zip(buffer.right.iter_mut());
            for ((l, r), (pl, pr)) in out.zip(left.iter().zip(right.iter())) {
                (*l, *r) = (*l + pl, *r + pr);
                peak = [peak[0].max(pl.abs()), peak[1].max(pr.abs())];
            }
            self.peaks[pad] = peak;
        }
    }

    /// Takes over `old`, the kit of the same channel in the engine this one replaces (an edit while playing): each pad
    /// gets the voices of `old`'s pad with its slot id, and in `held` what the sequencer held of it (`old_held`), so a
    /// sounding voice plays on from its frame with the pad's sample and send target as they are now. A slot `old` has
    /// and this kit has not is destroyed: its voices stop at once. Allocates nothing.
    pub fn adopt(&mut self, old: &Kit, held: &mut [u8], old_held: &[u8]) {
        let from = |pad: usize| old.ids.iter().position(|&id| id == self.ids[pad]);
        self.sampler.adopt(&old.sampler, from);
        for (pad, held) in held.iter_mut().enumerate() {
            if let Some(count) = from(pad).and_then(|from| old_held.get(from)) {
                *held = *count;
            }
        }
    }

    /// The last block's peak (L, R) of the slot with id `id`, 0 for an unknown slot.
    pub fn peak(&self, id: u32) -> [f32; 2] {
        let pad = self.ids.iter().position(|&slot| slot == id);
        pad.map_or([0.0; 2], |pad| self.peaks[pad])
    }
}

/// The kit of channel `channel`, `None` without slots. Slot `i` is pad `i`. A slot plays what `samples` holds for it
/// (its file, a built-in one-shot, or nothing), with its `STR1` stretch and pitch already rendered by the cache, or
/// streaming in. The pads start at the default params: the engine's initial table sets the slot's own, through the
/// same check as every other write, so a hostile value never reaches a voice. A send target that names no channel
/// plays into the kit's own channel.
pub(super) fn build(
    song: &Song,
    channel: usize,
    sample_rate: f32,
    samples: &SampleCache,
) -> Option<Kit> {
    let kit = (song.channels[channel].drum_kit.as_ref()).filter(|kit| !kit.slots.is_empty())?;
    let pads = (kit.slots.iter().enumerate())
        .map(|(index, slot)| Pad {
            sample: samples.pad_sample(slot),
            params: DEFAULTS,
            note: kit.pad_note(index),
            reverse: slot.file.reverse,
            cut_group: slot.cut_group,
            overlap: slot.file.overlapping_voices,
        })
        .collect();
    let output = |target: i32| {
        let targets = song.channels.iter().position(|other| other.id == target);
        targets.filter(|_| target >= 0).unwrap_or(channel)
    };
    let count = kit.slots.len();
    Some(Kit {
        sampler: PadSampler::new(pads, sample_rate),
        out: false,
        channel,
        ids: kit.slots.iter().map(|slot| slot.file.id).collect(),
        outputs: (kit.slots.iter())
            .map(|slot| output(slot.file.send_target))
            .collect(),
        peaks: vec![[0.0; 2]; count],
        scratch: OwnedStereoBuffer {
            left: vec![0.0; super::BLOCK_FRAMES],
            right: vec![0.0; super::BLOCK_FRAMES],
        },
    })
}

/// The hits of `channel`'s drum lanes as actions for its kit: velocity a/255, pan b/255, pitch from the value, at the
/// hit's tick plus its slot's beat offset (clamped at 0, truncated to a tick); its Repeat hits; on a notes lane, the
/// note-off its length ends at; on a step lane, each pad's cover from its first hit in the clip to the clip's end, for
/// A re-sync. Playback adds no swing: it already moved the stored ticks; only Repeat spacing reads it. Anything past
/// `end_beat` (a hostile beat offset) is dropped. With note effects in the rack the hits pass them first
/// ([`through`]), each note effect that ran leaving its trace in `traces`; without, the actions are as they always were.
pub(super) fn hits(
    song: &Song,
    channel: usize,
    end_beat: f64,
    sample_rate: f32,
    tempo_bpm: f32,
    actions: &mut Vec<ScheduledAction>,
    traces: &mut Vec<(usize, NoteEffectTrace)>,
) -> Result<(), EngineError> {
    let played = played(song, channel, end_beat, sample_rate, tempo_bpm, traces)?;
    let mut prepared = Vec::with_capacity(played.len());
    for (beat, action) in played {
        if beat > end_beat {
            continue;
        }
        let frame = beat_frame(beat, sample_rate, tempo_bpm)?;
        prepared.push(ScheduledAction {
            frame,
            channel,
            action,
        });
    }
    // A note-off before a hit at the same frame, so a hit right after a note-off still sounds.
    prepared.sort_by_key(|action| (action.frame, matches!(action.action, Action::Hit { .. })));
    actions.extend(prepared);
    Ok(())
}

/// A drum hit on its way through the note effects: a note of its pad's key, with the hit's pan byte, Pitch value and,
/// on a notes lane, the beat of its note-off.
#[derive(Clone, Copy)]
struct Strike {
    beat: f64,
    key: u8,
    velocity: u8,
    pan: u8,
    value: u16,
    off: Option<f64>,
}

/// [`hits`] by beat, in the order they were made, before `end_beat` drops what is past it.
pub(super) fn played(
    song: &Song,
    channel: usize,
    end_beat: f64,
    sample_rate: f32,
    tempo_bpm: f32,
    traces: &mut Vec<(usize, NoteEffectTrace)>,
) -> Result<Vec<(f64, Action)>, EngineError> {
    let Some(kit) = &song.channels[channel].drum_kit else {
        return Ok(Vec::new());
    };
    let effects = (song.channels[channel].rack.modules.iter())
        .filter(|module| (47..=51).contains(&module.type_id))
        .map(|module| note_effect(song, channel, module, end_beat))
        .collect::<Result<Vec<_>, _>>()?;
    let ppq = f64::from(PPQ);
    let mut prepared = Vec::new();
    let mut struck = Vec::new();
    let lanes = song.channels[channel].lanes.iter();
    for lane in lanes.filter(|lane| lane.kind == LaneKind::Drums) {
        let notes_lane = lane.grid == DrumGrid::Notes;
        let swing = if notes_lane { 0.0 } else { lane.swing };
        for clip in playing_clips(lane) {
            let clip_end = (f64::from(clip.start_tick) + clip.visible_ticks()) / ppq;
            // A step lane's pads with their first hit in the clip, which covers them to its end.
            let mut first: Vec<(usize, f64)> = Vec::new();
            for (tick, event) in clip.events_in_song_range(0, (end_beat * ppq).ceil() as u64) {
                let Event::Drum {
                    len_beats,
                    pad,
                    velocity,
                    pan,
                    value,
                    repeat,
                    ..
                } = *event
                else {
                    continue;
                };
                let Some(slot) = kit.slot_on_pad(usize::from(pad)) else {
                    continue;
                };
                let pad = usize::from(pad);
                let pos = tick as f64 / ppq;
                let offset = Some(slot.file.beat_offset).filter(|offset| offset.is_finite());
                let hit_tick = (tick as f64 + offset.unwrap_or(0.0) * ppq).max(0.0).trunc();
                let held = notes_lane && len_beats.is_finite() && len_beats > 0.0;
                let off = held.then(|| (pos + len_beats).min(clip_end));
                // @18 is signed: r ≥ 1 extra hits across the (swung) sixteenth, ignoring the beat offset.
                let r = f64::from((repeat as i8).max(0));
                let even = ((4.0 * pos) as u64).is_multiple_of(2);
                let sixteenth = 0.25 + if even { swing / 8.0 } else { -swing / 8.0 };
                let repeats = (1..=r as u32).map(|k| pos + sixteenth * f64::from(k) / (r + 1.0));
                // A silent hit, or one on a pad with no key, is no note: it plays as it is.
                match kit
                    .pad_note(pad)
                    .filter(|_| !effects.is_empty() && velocity > 0)
                {
                    Some(key) => {
                        let strike = |beat, off| Strike {
                            beat,
                            key,
                            velocity,
                            pan,
                            value,
                            off,
                        };
                        struck.push(strike(hit_tick / ppq, off));
                        struck.extend(repeats.map(|beat| strike(beat, None)));
                    }
                    None => {
                        let hit = |held| Action::Hit {
                            pad,
                            velocity: f32::from(velocity) / 255.0,
                            pan: f32::from(pan) / 255.0,
                            pitch: hit_pitch(value),
                            held,
                        };
                        prepared.push((hit_tick / ppq, hit(held)));
                        prepared.extend(repeats.map(|beat| (beat, hit(false))));
                        prepared.extend(off.map(|off| (off, Action::PadOff { pad })));
                    }
                }
                if !notes_lane && !first.iter().any(|&(p, _)| p == pad) {
                    first.push((pad, hit_tick / ppq));
                }
            }
            for (pad, beat) in first {
                prepared.push((beat, Action::StepCover { pad, on: true }));
                prepared.push((clip_end, Action::StepCover { pad, on: false }));
            }
        }
    }
    if !struck.is_empty() {
        let args = (song, channel, end_beat, sample_rate, tempo_bpm);
        prepared.extend(through(args, kit, effects, struck, traces)?);
    }
    Ok(prepared)
}

/// Issue #40: `struck`, the hits of `channel` on pads with a key, pass the rack's
/// note effects (`effects`, every one in rack order, wherever it sits: the pads play before any module) as notes of
/// their pads' keys, as a live key does. A hit lasts to its note-off on a notes lane, else a sixteenth; notes on one key
/// that overlap pair their note-offs oldest first, as any notes through note effects do. What comes out plays every pad
/// on its key (a note on none is dropped), at its velocity and pan, taking the Pitch value and the note-off of the hit
/// it passed as (one on its key and beat, in turn), else of the latest hit on its key before it (none: centred, no
/// note-off). Transpose, Scale and the Randomizer's Pitch move a hit to another pad.
fn through(
    (song, channel, end_beat, sample_rate, tempo_bpm): (&Song, usize, f64, f32, f32),
    kit: &DrumKit,
    effects: Vec<NoteEffectModule>,
    mut struck: Vec<Strike>,
    traces: &mut Vec<(usize, NoteEffectTrace)>,
) -> Result<Vec<(f64, Action)>, EngineError> {
    let tick = 1.0 / f64::from(PPQ);
    // What is past the end is dropped anyway; the rest, note-offs included, ends before end + 1.
    struck.retain(|strike| strike.beat <= end_beat);
    let mut input = Vec::with_capacity(2 * struck.len());
    for s in &struck {
        let off = s.off.unwrap_or(s.beat + 0.25).max(s.beat + tick);
        let pan = f32::from(s.pan) / 255.0;
        input.push(InputNoteEvent::on(s.beat, i16::from(s.key), s.velocity).with_pan(pan));
        input.push(InputNoteEvent::off(off, i16::from(s.key)));
    }
    sort_note_events(&mut input);
    let mut traced = Vec::new();
    let output = (note_rack(song, effects, sample_rate)?)
        .process_events_traced(
            0.0,
            end_beat + 1.0,
            f64::from(tempo_bpm),
            &input,
            &mut traced,
        )
        .map_err(EngineError::Notes)?;
    add_traces(traces, channel, traced);
    struck.sort_by(|a, b| a.key.cmp(&b.key).then(a.beat.total_cmp(&b.beat)));
    let pads = |key| (0..kit.slots.len()).filter(move |&pad| kit.pad_note(pad) == Some(key));
    // Per key, whether each note sounding there is released by its note-off, oldest first.
    let mut open = vec![VecDeque::new(); 128];
    let mut used = vec![false; struck.len()];
    let mut played = Vec::new();
    for event in output {
        let key = event.pitch as u8;
        if event.kind == NoteKind::Off {
            if open[usize::from(key)].pop_front() == Some(true) {
                played.extend(pads(key).map(|pad| (event.beat, Action::PadOff { pad })));
            }
            continue;
        }
        // The hit it passed as, in turn among those on its key and beat, else the latest one before it there.
        let (at, latest) = (
            struck.partition_point(|s| (s.key, s.beat) < (key, event.beat)),
            struck.partition_point(|s| (s.key, s.beat) <= (key, event.beat)),
        );
        let turn = (at..latest).find(|&i| !used[i]);
        let from = turn.or(latest.checked_sub(1)).map(|i| struck[i]);
        let from = from.filter(|s| s.key == key);
        if let Some(i) = turn {
            used[i] = true;
        }
        let held = from.is_some_and(|s| s.off.is_some());
        open[usize::from(key)].push_back(held);
        let pitch = hit_pitch(from.map_or(Event::DEFAULT_PITCH, |s| s.value));
        let hit = |pad| Action::Hit {
            pad,
            velocity: f32::from(event.velocity) / 255.0,
            pan: event.pan,
            pitch,
            held,
        };
        played.extend(pads(key).map(|pad| (event.beat, hit(pad))));
    }
    Ok(played)
}
