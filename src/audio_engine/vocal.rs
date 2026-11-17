//! Worker-prepared vocal expression events. Scalar updates keep the render thread allocation-free.
//! Note effects that transpose/generate notes are not yet supported for vocal metadata routing.
use super::EngineError;
use super::prepare::{Action, MAX_PLAYS, ScheduledAction, beat_frame, song_end_beat};
use crate::audio_bridge::playing_clips;
use crate::song::{Event, LaneKind, PPQ, Song, VocalPitchPoint};
use honeydew_audio::vocal_synth::VocalExpression;

fn vowel(lyric: &str) -> u8 {
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

fn pitch(points: &[VocalPitchPoint], at: f32) -> f32 {
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
                    let count = (stop - start).div_ceil(stride);
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
