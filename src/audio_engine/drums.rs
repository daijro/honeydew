//! A channel's pad sampler: the kit built from the channel's `DrumKit`, and its drum lanes' hits as prepared actions.
//! The engine renders a kit before the channel's instruments, so the rack's modules run on the pads' audio.

use honeydew_audio::drums::{DEFAULTS, Pad, PadSampler, hit_pitch};
use honeydew_audio::mixer::OwnedStereoBuffer;

use super::EngineError;
use super::prepare::{Action, ScheduledAction, beat_frame};
use crate::audio_bridge::playing_clips;
use crate::sample_cache::SampleCache;
use crate::song::{DrumGrid, Event, LaneKind, PPQ, Song};

/// A channel's pads as the engine plays them.
pub(super) struct Kit {
    pub sampler: PadSampler,
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
    /// peak. A pad with a send target (`LINk`@10 ≥ 0) skips its own channel entirely: rack, strip and sends.
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
/// `end_beat` (a hostile beat offset) is dropped.
pub(super) fn hits(
    song: &Song,
    channel: usize,
    end_beat: f64,
    sample_rate: f32,
    tempo_bpm: f32,
    actions: &mut Vec<ScheduledAction>,
) -> Result<(), EngineError> {
    let Some(kit) = &song.channels[channel].drum_kit else {
        return Ok(());
    };
    let ppq = f64::from(PPQ);
    let mut prepared = Vec::new();
    let mut push = |beat: f64, action| -> Result<(), EngineError> {
        if beat > end_beat {
            return Ok(());
        }
        let frame = beat_frame(beat, sample_rate, tempo_bpm)?;
        prepared.push(ScheduledAction {
            frame,
            channel,
            action,
        });
        Ok(())
    };
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
                let hit = |held| Action::Hit {
                    pad,
                    velocity: f32::from(velocity) / 255.0,
                    pan: f32::from(pan) / 255.0,
                    pitch: hit_pitch(value),
                    held,
                };
                push(hit_tick / ppq, hit(held))?;
                if !notes_lane && !first.iter().any(|&(p, _)| p == pad) {
                    first.push((pad, hit_tick / ppq));
                }
                // @18 is signed: r ≥ 1 extra hits across the (swung) sixteenth, ignoring the beat offset.
                let r = f64::from((repeat as i8).max(0));
                let even = ((4.0 * pos) as u64).is_multiple_of(2);
                let sixteenth = 0.25 + if even { swing / 8.0 } else { -swing / 8.0 };
                for k in 1..=r as u32 {
                    push(pos + sixteenth * f64::from(k) / (r + 1.0), hit(false))?;
                }
                if held {
                    push((pos + len_beats).min(clip_end), Action::PadOff { pad })?;
                }
            }
            for (pad, beat) in first {
                push(beat, Action::StepCover { pad, on: true })?;
                push(clip_end, Action::StepCover { pad, on: false })?;
            }
        }
    }
    // A note-off before a hit at the same frame, so a hit right after a note-off still sounds.
    prepared.sort_by_key(|action| (action.frame, matches!(action.action, Action::Hit { .. })));
    actions.extend(prepared);
    Ok(())
}
