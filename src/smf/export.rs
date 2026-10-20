//! MIDI export: the regular channels' notes, drum hits and channel MIDI automation as a format-1 file at 128 PPQ.

use super::codec::{self, MAX_DELTA, MAX_EVENTS, MAX_TRACKS, Smf, SmfEvent};
use crate::midi::MidiMsg;
use crate::song::{AutomationTarget, Clip, DrumKit, Event, Lane, LaneKind, PPQ, Song, points};

/// No lane to write. The Save dialog shows `Error: no data to save`.
#[derive(Debug, PartialEq, Eq)]
pub struct NoData;

/// What a lane writes; every other lane is left out.
#[derive(Clone, Copy)]
enum Write {
    Notes,
    Drums,
    PitchBend,
    Aftertouch,
    Cc(u8),
}

fn write_of(lane: &Lane) -> Option<Write> {
    match (lane.kind, lane.target) {
        (LaneKind::Notes, _) => Some(Write::Notes),
        (LaneKind::Drums, _) => Some(Write::Drums),
        (
            LaneKind::Automation,
            Some(AutomationTarget {
                instance_id: 0,
                param,
            }),
        ) => match param {
            4 => Some(Write::PitchBend),
            5 => Some(Write::Aftertouch),
            7..=133 => Some(Write::Cc((param - 7) as u8)),
            _ => None,
        },
        _ => None,
    }
}

/// The song (`only: None`) or channel `only` as a Standard MIDI file: one track per written lane, in channel and then
/// lane order; the tempo in the first track; each track named after its channel.
///
/// MIDI channels: a counter from 0 that steps once per regular channel (skipping 10, stopping at 15), 0 for a single
/// channel, and 9 for drum lanes. Plays at song ticks past [`MAX_DELTA`] are dropped (a note-off is cut there), and
/// the file stays under the reader's caps: at most [`MAX_TRACKS`] tracks and [`MAX_EVENTS`] events.
pub fn smf_of(song: &Song, only: Option<usize>) -> Result<Vec<u8>, NoData> {
    // A drum hit with no length lasts one step.
    let step = 1.0 / f64::from(song.time_sig.steps_per_beat());
    let mut tracks = Vec::new();
    // ponytail: plays past the caps are dropped silently; a message if a real song ever gets there.
    let mut budget = MAX_EVENTS - 3 * MAX_TRACKS;
    let mut next = 0u8;
    for (i, channel) in song.channels.iter().enumerate() {
        // Regular channels only: not the master, not a send.
        if channel.kind.file_kind() != 0x100 {
            continue;
        }
        let counted = next;
        next = if next + 1 == 10 { next + 2 } else { next + 1 }.min(15);
        if only.is_some_and(|o| o != i) {
            continue;
        }
        let ch = if only.is_some() { 0 } else { counted };
        let mut name = channel.name.clone();
        name.truncate(name.floor_char_boundary(255));
        for lane in &channel.lanes {
            let Some(write) = write_of(lane) else {
                continue;
            };
            if tracks.len() == MAX_TRACKS || lane.clips.iter().all(|c| c.events.is_empty()) {
                continue;
            }
            // (tick, not a note-off, message): note-offs go first at a tick, then the clips' order.
            let mut events = Vec::new();
            'clips: for clip in &lane.clips {
                for (t, event) in plays(clip) {
                    if budget < 2 {
                        break 'clips;
                    }
                    let msgs = messages(write, ch, step, t, event, channel.drum_kit.as_ref());
                    budget -= msgs.len();
                    events.extend(msgs);
                }
            }
            events.sort_by_key(|&(tick, not_off, _)| (tick, not_off));
            let mut track = vec![(0, SmfEvent::Name(name.clone().into_bytes()))];
            track.extend((events.into_iter()).map(|(tick, _, msg)| (tick, SmfEvent::Voice(msg))));
            tracks.push(track);
        }
    }
    let first = tracks.first_mut().ok_or(NoData)?;
    let tempo = ((60_000_000.0 / song.tempo_bpm) as u32).clamp(1, 0xFF_FFFF);
    first.insert(0, (0, SmfEvent::Tempo(tempo)));
    Ok(codec::write(&Smf {
        division: PPQ as u16,
        tracks,
    }))
}

/// The messages of one play at song tick `t` (fractional) as `(tick, not a note-off, message)`. `step`: a drum hit's
/// length when it has none; `kit`: the channel's, whose pads' MIDI notes drum hits play.
fn messages(
    write: Write,
    ch: u8,
    step: f64,
    t: f64,
    event: &Event,
    kit: Option<&DrumKit>,
) -> Vec<(u64, bool, MidiMsg)> {
    let on = t as u64;
    // A note lasts a tick at least, so its off never sorts before its on.
    let note = |ch, key: u8, vel: u8, len: f64| {
        let key = key.min(127);
        let off = ((t + len * f64::from(PPQ)) as u64).clamp(on + 1, MAX_DELTA);
        let vel = vel >> 1;
        vec![
            (on, true, MidiMsg::NoteOn { ch, key, vel }),
            (off, false, MidiMsg::NoteOff { ch, key, vel: 0 }),
        ]
    };
    // u16/65534 scaled to the full MIDI range, and Aftertouch as a real `Dn vv`.
    let scaled = |value: f32, max: u16| {
        let raw = u64::from(points::raw(value));
        ((raw * u64::from(max) + 32767) / 65534).min(u64::from(max)) as u16
    };
    let msg = match (write, *event) {
        (
            Write::Notes,
            Event::Note {
                len_beats,
                key,
                velocity,
                ..
            },
        ) => {
            return note(ch, key, velocity, len_beats);
        }
        (
            Write::Drums,
            Event::Drum {
                len_beats,
                pad,
                velocity,
                ..
            },
        ) => {
            let len = if len_beats > 0.0 { len_beats } else { step };
            // A pad with a MIDI note set plays it, so an imported GM kit's keys come back; any other pad plays
            // pad + 36.
            let stored = kit.and_then(|k| k.slot_on_pad(usize::from(pad))?.midi_note());
            return note(9, stored.unwrap_or(pad.saturating_add(36)), velocity, len);
        }
        (Write::PitchBend, Event::Automation { value, .. }) => {
            let value = scaled(value, 16383);
            MidiMsg::PitchBend { ch, value }
        }
        (Write::Aftertouch, Event::Automation { value, .. }) => {
            let value = scaled(value, 127) as u8;
            MidiMsg::ChannelPressure { ch, value }
        }
        (Write::Cc(cc), Event::Automation { value, .. }) => {
            let value = scaled(value, 127) as u8;
            MidiMsg::Cc { ch, cc, value }
        }
        _ => return Vec::new(),
    };
    vec![(on, true, msg)]
}

/// Every play of `clip`'s events, `(song tick, event)` with the tick fractional, by the playback loop rule: the
/// content repeats every [`Clip::loop_ticks`] from the offset, shown for [`Clip::visible_ticks`]. Pass by pass, in
/// stored order within a pass; plays at or past [`MAX_DELTA`] are dropped.
pub(crate) fn plays(clip: &Clip) -> impl Iterator<Item = (f64, &Event)> {
    let lp = clip.loop_ticks().unwrap_or(f64::NAN);
    let visible = clip.visible_ticks();
    let off = clip.offset_beats * f64::from(PPQ);
    let start = f64::from(clip.start_tick);
    let last = MAX_DELTA as f64;
    // Without a finite offset nothing plays; without events no pass would end the walk.
    let walk = off.is_finite() && !clip.events.is_empty();
    (0u64..)
        .map(move |k| k as f64 * lp)
        .take_while(move |&base| walk && base < visible && start + base < last)
        .flat_map(move |base| {
            clip.events.iter().filter_map(move |e| {
                let rel = base + (f64::from(e.pos()) - off).rem_euclid(lp);
                (rel < visible && start + rel < last).then_some((start + rel, e))
            })
        })
}
