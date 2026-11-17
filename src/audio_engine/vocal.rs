//! Worker-prepared vocal expression events. Scalar updates keep the render thread allocation-free.
//! Note effects that transpose/generate notes are not yet supported for vocal metadata routing.
use super::EngineError;
use super::prepare::{Action, MAX_PLAYS, ScheduledAction, beat_frame, song_end_beat};
use crate::audio_bridge::playing_clips;
use crate::song::{Event, LaneKind, PPQ, Song, VocalPitchPoint};
use honeydew_audio::vocal_synth::VocalExpression;

pub(crate) fn vowel(lyric: &str) -> u8 {
    // Procedural fallback only, not a phonemizer. Last explicit Latin vowel wins.
    lyric
        .chars()
        .rev()
        .find_map(|ch| match ch.to_ascii_lowercase() {
            'a' | 'あ' | 'ア' => Some(0),
            'i' | 'い' | 'イ' => Some(1),
            'u' | 'う' | 'ウ' => Some(2),
            'e' | 'え' | 'エ' => Some(3),
            'o' | 'お' | 'オ' => Some(4),
            _ => None,
        })
        .unwrap_or(255)
}

pub(crate) fn pitch(points: &[VocalPitchPoint], at: f32) -> f32 {
    let after = points.partition_point(|p| p.offset < at);
    let left = after.checked_sub(1).and_then(|i| points.get(i));
    let right = points.get(after);
    match (left, right) {
        (Some(a), Some(b)) if b.offset > a.offset => {
            a.cents
                + (b.cents - a.cents) * ((at - a.offset) / (b.offset - a.offset)).clamp(0.0, 1.0)
        }
        (Some(p), _) | (_, Some(p)) => p.cents,
        _ => 0.0,
    }
}

pub(super) fn actions(
    song: &Song,
    rate: f32,
    bpm: f32,
    out: &mut Vec<ScheduledAction>,
) -> Result<(), EngineError> {
    let end = (song_end_beat(song)? * f64::from(PPQ)).ceil() as u64;
    // 100 Hz is independent of PPQ and sample rate; curves are smoothed in the voice.
    let stride = (rate / 100.0).round().max(1.0) as u64;
    let mut budget = MAX_PLAYS;
    for (channel, ch) in song.channels.iter().enumerate() {
        let modules: Vec<_> = ch.rack.modules.iter().filter(|m| m.type_id == 53).collect();
        if modules.is_empty() {
            continue;
        }
        for lane in ch.lanes.iter().filter(|lane| lane.kind == LaneKind::Notes) {
            for clip in playing_clips(lane) {
                if clip.file.vocal_notes.is_empty() {
                    continue;
                }
                let cut = f64::from(clip.start_tick) + clip.visible_ticks();
                for (tick, event) in clip.events_in_song_range(0, end) {
                    let Event::Note {
                        pos,
                        key,
                        len_beats,
                        slide: false,
                        ..
                    } = event
                    else {
                        continue;
                    };
                    let Some(data) = clip
                        .file
                        .vocal_notes
                        .iter()
                        .find(|n| n.pos == *pos && n.key == *key)
                    else {
                        continue;
                    };
                    if !len_beats.is_finite() || *len_beats <= 0.0 {
                        continue;
                    }
                    let start = beat_frame(tick as f64 / f64::from(PPQ), rate, bpm)?;
                    let whole_end =
                        beat_frame(tick as f64 / f64::from(PPQ) + len_beats, rate, bpm)?;
                    let stop = beat_frame(
                        (tick as f64 + len_beats * f64::from(PPQ)).min(cut) / f64::from(PPQ),
                        rate,
                        bpm,
                    )?;
                    if stop <= start {
                        continue;
                    }
                    let count = if data.pitch.is_empty() {
                        1
                    } else {
                        (stop - start).div_ceil(stride)
                    };
                    let needed = count.saturating_mul(modules.len() as u64);
                    if needed > budget as u64 {
                        return Err(EngineError::TooManyEvents);
                    }
                    budget -= needed as usize;
                    let mut points: Vec<_> = data
                        .pitch
                        .iter()
                        .copied()
                        .filter(|p| p.offset.is_finite() && p.cents.is_finite())
                        .collect();
                    points.sort_by(|a, b| a.offset.total_cmp(&b.offset));
                    for step in 0..count {
                        let frame = start + step * stride;
                        let at = (frame - start) as f32 / (whole_end - start).max(1) as f32;
                        let expression = VocalExpression {
                            cents: pitch(&points, at),
                            dynamics: data.dynamics,
                            breathiness: data.breathiness,
                            formant: data.formant,
                            vowel: vowel(&data.lyric),
                        };
                        for module in &modules {
                            out.push(ScheduledAction {
                                frame,
                                channel,
                                action: Action::Vocal {
                                    instance_id: module.instance_id,
                                    key: *key,
                                    expression,
                                },
                            });
                        }
                    }
                }
            }
        }
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn vocal_pitch_interpolates_and_extends_endpoints() {
        let points = [
            VocalPitchPoint {
                offset: 0.25,
                cents: -100.0,
            },
            VocalPitchPoint {
                offset: 0.75,
                cents: 100.0,
            },
        ];
        assert_eq!(pitch(&points, 0.0), -100.0);
        assert_eq!(pitch(&points, 0.5), 0.0);
        assert_eq!(pitch(&points, 1.0), 100.0);
        assert_eq!(vowel("hello"), 4);
        assert_eq!(vowel("息"), 255);
    }
}

#[cfg(test)]
mod schedule_tests {
    use super::*;
    use crate::song::{ChannelKind, SynthKind, VocalNoteState};

    #[test]
    fn vocal_schedule_repeats_clips_and_cuts_at_visible_end() {
        let mut song = Song::default();
        song.add_channel(ChannelKind::Synth(SynthKind::VocalSynth));
        let clip = song.channels[1].lanes[0].add_clip(128, 1.0);
        clip.visible_len_beats = 2.5;
        clip.events.push(Event::Note {
            pos: 0,
            key: 60,
            len_beats: 1.0,
            velocity: 255,
            pan: 127,
            slide: false,
        });
        clip.file.vocal_notes.push(VocalNoteState {
            pos: 0,
            key: 60,
            lyric: "la".into(),
            dynamics: 0.75,
            breathiness: 0.2,
            formant: 0.6,
            pitch: vec![
                VocalPitchPoint {
                    offset: 0.0,
                    cents: -100.0,
                },
                VocalPitchPoint {
                    offset: 1.0,
                    cents: 100.0,
                },
            ],
        });
        let mut out = Vec::new();
        actions(&song, 48000.0, 120.0, &mut out).unwrap();
        assert_eq!(out.first().unwrap().frame, 24000);
        assert!(out.iter().all(|event| event.frame < 84000));
        assert_eq!(out.len(), 125);
        for start in [24000, 48000, 72000] {
            assert!(out.iter().any(|event| event.frame == start && matches!(event.action,
                Action::Vocal { expression, key: 60, .. } if expression.cents == -100.0 && expression.vowel == 0)));
        }
        song.channels[1].lanes[0].clips[0].muted = true;
        out.clear();
        actions(&song, 48000.0, 120.0, &mut out).unwrap();
        assert!(out.is_empty());
    }
}
