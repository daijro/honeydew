//! MIDI export: the regular channels' notes, drum hits and channel MIDI automation as a format-1 file at 128 PPQ, with
//! the song's tempo map.

use super::codec::{self, MAX_DELTA, MAX_EVENTS, MAX_TRACKS, Smf, SmfEvent};
use crate::midi::MidiMsg;
use crate::song::points::{PointKey, shape_value};
use crate::song::{
    AutomationTarget, ChannelKind, Clip, DrumKit, Event, Lane, LaneKind, PPQ, Song, TEMPO_TARGET,
    automated_tempo, points, tempo_value,
};

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
/// lane order; the tempo map in the first track ([`tempo_map`]); each track named after its channel.
///
/// MIDI channels: a counter from 0 that steps once per regular channel (skipping 10, stopping at 15), 0 for a single
/// channel, and 9 for drum lanes. Plays at song ticks past [`MAX_DELTA`] are dropped (a note-off is cut there), and
/// the file stays under the reader's caps: at most [`MAX_TRACKS`] tracks and [`MAX_EVENTS`] events.
pub fn smf_of(song: &Song, only: Option<usize>) -> Result<Vec<u8>, NoData> {
    smf_with(song, only, None)
}

/// [`smf_of`], with `Apply note effects` (#31) when `note_effects` is the engine's sample rate: a channel with note
/// effects writes the notes its Notes lanes play through them, as playback prepares them at that rate (the
/// Arpeggiator's and Note Echo's timing back in ticks, [`crate::audio_engine::burned_notes`]), all on its first Notes
/// lane's track. A song playback refuses writes its raw notes.
pub fn smf_with(
    song: &Song,
    only: Option<usize>,
    note_effects: Option<f32>,
) -> Result<Vec<u8>, NoData> {
    // A drum hit with no length lasts one step.
    let step = 1.0 / f64::from(song.time_sig.steps_per_beat());
    let mut tracks = Vec::new();
    // ponytail: plays past the caps are dropped silently; a message if a real song ever gets there.
    let mut budget = MAX_EVENTS - 3 * MAX_TRACKS;
    let mut tempos = tempo_map(song);
    tempos.truncate(budget / 2);
    budget -= tempos.len();
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
        let effects = (channel.rack.modules.iter()).any(|m| (47..=51).contains(&m.type_id));
        let played = note_effects.filter(|_| effects).and_then(|rate| {
            crate::audio_engine::burned_notes(song, i, rate, 0, MAX_DELTA as u32).ok()
        });
        let mut played_written = false;
        for lane in &channel.lanes {
            let Some(write) = write_of(lane) else {
                continue;
            };
            if tracks.len() == MAX_TRACKS {
                continue;
            }
            let source: Box<dyn Iterator<Item = (f64, &Event)>> = match (write, &played) {
                (Write::Notes, Some(notes)) => {
                    if played_written || notes.is_empty() {
                        continue;
                    }
                    played_written = true;
                    Box::new(notes.iter().map(|e| (f64::from(e.pos()), e)))
                }
                _ if lane.clips.iter().all(|c| c.events.is_empty()) => continue,
                _ => Box::new(lane.clips.iter().flat_map(plays)),
            };
            // (tick, not a note-off, message): note-offs go first at a tick, then the clips' order.
            let mut events = Vec::new();
            for (t, event) in source {
                if budget < 2 {
                    break;
                }
                let msgs = messages(write, ch, step, t, event, channel.drum_kit.as_ref());
                budget -= msgs.len();
                events.extend(msgs);
            }
            events.sort_by_key(|&(tick, not_off, _)| (tick, not_off));
            let mut track = vec![(0, SmfEvent::Name(name.clone().into_bytes()))];
            track.extend((events.into_iter()).map(|(tick, _, msg)| (tick, SmfEvent::Voice(msg))));
            tracks.push(track);
        }
    }
    let first = tracks.first_mut().ok_or(NoData)?;
    first.splice(0..0, tempos);
    // Stable: a tempo goes first at its tick.
    first.sort_by_key(|e| e.0);
    Ok(codec::write(&Smf {
        division: PPQ as u16,
        tracks,
    }))
}

/// The song's tempo as `FF 51`s at song ticks: its tempo at tick 0, then the changes the master's tempo lanes play
/// ([`tempo_changes`]), each written only where the tempo changes at the lane's resolution; at one tick the last.
fn tempo_map(song: &Song) -> Vec<(u64, SmfEvent)> {
    let value = |bpm| points::raw(tempo_value(bpm));
    let mut map = vec![(0, song.tempo_bpm)];
    let master = song
        .channels
        .first()
        .filter(|c| c.kind == ChannelKind::Master);
    let lanes =
        (master.into_iter().flat_map(|c| &c.lanes)).filter(|l| l.target == Some(TEMPO_TARGET));
    for clip in lanes.flat_map(crate::audio_bridge::playing_clips) {
        let mut changes = clip.clone();
        changes.events = tempo_changes(clip);
        map.extend(plays(&changes).filter_map(|(t, e)| match *e {
            Event::Automation { value, .. } => Some((t as u64, automated_tempo(value))),
            _ => None,
        }));
    }
    map.sort_by_key(|c| c.0);
    // The song's own tempo stays at tick 0 when the lane's point there says the same.
    map.dedup_by(|later, kept| {
        let same = later.0 == kept.0;
        if same && value(later.1) != value(kept.1) {
            kept.1 = later.1;
        }
        same
    });
    map.dedup_by_key(|c| value(c.1));
    (map.into_iter())
        .map(|(t, bpm)| {
            let us = ((60_000_000.0 / bpm) as u32).clamp(1, 0xFF_FFFF);
            (t, SmfEvent::Tempo(us))
        })
        .collect()
}

/// A tempo clip's tempo changes as automation events at its content ticks: each point, each riser of a stepped segment
/// (Square, Stairs) as the engine plays it, and a curve every sixteenth note.
fn tempo_changes(clip: &Clip) -> Vec<Event> {
    const SIXTEENTH: usize = PPQ as usize / 4;
    let pts = points::points(clip, PointKey::Automation);
    let mut out = Vec::new();
    for (i, a) in pts.iter().enumerate() {
        out.push((a.tick, f64::from(a.value)));
        let Some(b) = pts.get(i + 1).filter(|b| b.tick > a.tick) else {
            continue;
        };
        let len = f64::from(b.tick - a.tick);
        let xs = a.shape.risers().unwrap_or_else(|| {
            let from = (u64::from(a.tick) + 1).next_multiple_of(SIXTEENTH as u64);
            (from..u64::from(b.tick))
                .step_by(SIXTEENTH)
                .map(|t| (t - u64::from(a.tick)) as f64 / len)
                .collect()
        });
        for x in xs {
            let (from, to) = (f64::from(a.value), f64::from(b.value));
            // Just after a riser, as the engine steps.
            let v = shape_value(a.shape, from, to, x.max(f64::MIN_POSITIVE));
            out.push(((f64::from(a.tick) + x * len).round() as u32, v));
        }
    }
    (out.into_iter())
        .map(|(pos, v)| Event::Automation {
            pos,
            value: points::unit(v.round() as u16),
            control: true,
        })
        .collect()
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
