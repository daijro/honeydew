//! Shared, atomic arrangement edits for MCP and in-app AI. No network or UI.
use crate::song::{ChannelKind, Event, LaneKind, PPQ, Song, SynthKind};
use serde::Deserialize;
use serde_json::{Value, json};

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct Arrangement {
    tempo_bpm: Option<f64>,
    tracks: Vec<TrackPattern>,
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct TrackPattern {
    target: Target,
    /// Zero-based bar in the song.
    start_bar: u32,
    bars: u32,
    notes: Vec<Note>,
}

#[derive(Deserialize)]
#[serde(tag = "kind", rename_all = "snake_case", deny_unknown_fields)]
enum Target {
    Existing {
        channel_id: i32,
        replace_clip_id: Option<u32>,
    },
    New {
        instrument: String,
        name: Option<String>,
    },
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct Note {
    /// Position from the start of this clip, measured in quarter-note beats.
    beat: f64,
    length_beats: f64,
    /// MIDI key for a pitched instrument; drum pad index for a drum kit.
    key: u8,
    velocity: u8,
}

fn synth(label: &str) -> Option<ChannelKind> {
    if label.eq_ignore_ascii_case("Drums") {
        return Some(ChannelKind::Drums);
    }
    SynthKind::ALL
        .iter()
        .copied()
        .find(|s| s.label().eq_ignore_ascii_case(label))
        .map(ChannelKind::Synth)
}

/// Mutates only a clone. If any track is invalid, the caller's song stays untouched.
pub(crate) fn arrange(song: &Song, request: Arrangement) -> Result<(Song, Vec<Value>), String> {
    if request.tracks.is_empty() || request.tracks.len() > 16 {
        return Err("provide 1–16 tracks".into());
    }
    let mut result = song.clone();
    if let Some(bpm) = request.tempo_bpm {
        if !bpm.is_finite() || !(1.0..=999.0).contains(&bpm) {
            return Err("tempo_bpm must be 1–999".into());
        }
        result.set_tempo(bpm);
    }
    let bar_beats = f64::from(song.time_sig.bar_beats());
    let mut made = Vec::new();
    let mut event_count = 0usize;
    for pattern in request.tracks {
        if pattern.bars == 0 || pattern.bars > 16 || pattern.start_bar > 4095 {
            return Err("bars must be 1–16 and start_bar at most 4095".into());
        }
        if pattern.notes.len() > 512 {
            return Err("at most 512 notes per track".into());
        }
        event_count += pattern.notes.len();
        if event_count > 2048 {
            return Err("at most 2048 notes per arrangement".into());
        }
        let (index, replace) = match pattern.target {
            Target::Existing {
                channel_id,
                replace_clip_id,
            } => {
                let index = result
                    .channels
                    .iter()
                    .position(|ch| ch.id == channel_id)
                    .ok_or_else(|| format!("channel {channel_id} is not in the open song"))?;
                (index, replace_clip_id)
            }
            Target::New { instrument, name } => {
                let kind = synth(&instrument)
                    .ok_or_else(|| format!("unknown built-in instrument: {instrument}"))?;
                let index = result
                    .insert_channel(result.channels.len(), kind)
                    .ok_or("channel limit reached")?;
                if let Some(name) = name {
                    if name.is_empty() || name.chars().count() > 80 {
                        return Err("track name must be 1–80 characters".into());
                    }
                    result.channels[index].name = name;
                }
                (index, None)
            }
        };
        let channel = &mut result.channels[index];
        let lane = channel.lanes.first_mut().ok_or("channel has no lane")?;
        if !matches!(lane.kind, LaneKind::Notes | LaneKind::Drums) {
            return Err(format!(
                "channel {} is not a note or drum track",
                channel.id
            ));
        }
        let start = f64::from(pattern.start_bar) * bar_beats * f64::from(PPQ);
        let start_tick = u32::try_from(start as u64).map_err(|_| "clip start out of range")?;
        let length = f64::from(pattern.bars) * bar_beats;
        let max_ticks = length * f64::from(PPQ);
        let mut events = Vec::with_capacity(pattern.notes.len());
        for note in pattern.notes {
            if !note.beat.is_finite()
                || !note.length_beats.is_finite()
                || note.beat < 0.0
                || note.length_beats <= 0.0
                || (note.beat + note.length_beats) * f64::from(PPQ) > max_ticks
                || note.velocity == 0
            {
                return Err(
                    "note beat/length must fit in the clip and velocity must be 1–255".into(),
                );
            }
            let pos = (note.beat * f64::from(PPQ)).round() as u32;
            let event = if lane.kind == LaneKind::Drums {
                if channel
                    .drum_kit
                    .as_ref()
                    .is_none_or(|kit| kit.pad_note(note.key as usize).is_none())
                {
                    return Err(format!(
                        "drum pad {} is not present on channel {}",
                        note.key, channel.id
                    ));
                }
                Event::Drum {
                    pos,
                    len_beats: note.length_beats,
                    pad: note.key,
                    velocity: note.velocity,
                    pan: Event::DEFAULT_PAN,
                    value: Event::DEFAULT_PITCH,
                    repeat: 0,
                }
            } else {
                if note.key > 127 {
                    return Err("MIDI keys must be 0–127".into());
                }
                Event::Note {
                    pos,
                    len_beats: note.length_beats,
                    key: note.key,
                    velocity: note.velocity,
                    pan: Event::DEFAULT_PAN,
                    slide: false,
                }
            };
            events.push(event);
        }
        events.sort_by_key(Event::pos);
        let clip = if let Some(clip_id) = replace {
            let clip = lane
                .clips
                .iter_mut()
                .find(|c| c.id == clip_id)
                .ok_or_else(|| format!("clip {clip_id} is not on channel {}", channel.id))?;
            if clip.start_tick != start_tick || (clip.loop_len_beats - length).abs() > 0.001 {
                return Err("replacement clip must have the given start_bar and bars".into());
            }
            clip
        } else {
            if lane.clips.iter().any(|c| c.start_tick == start_tick) {
                return Err(format!(
                    "channel {} already has a clip at bar {}: use replace_clip_id",
                    channel.id, pattern.start_bar
                ));
            }
            lane.add_clip(start_tick, length)
        };
        clip.events = events;
        let clip_id = clip.id;
        made.push(
            json!({"channel_id": channel.id, "channel_name": channel.name, "clip_id": clip_id}),
        );
    }
    Ok((result, made))
}
