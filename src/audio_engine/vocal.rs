//! Worker-prepared vocal expression events. Scalar updates keep the render thread allocation-free.
//! Note effects that transpose/generate notes are not yet supported for vocal metadata routing.
use super::EngineError;
use super::prepare::{Action, MAX_PLAYS, ScheduledAction, beat_frame, song_end_beat};
use crate::audio_bridge::playing_clips;
use crate::song::{Event, LaneKind, PPQ, Song, VocalPitchPoint};
use banaforge_audio::vocal_synth::VocalExpression;

/// Until a bank renderer is active, lyrics are annotations, never DSP controls.
/// 255 preserves the procedural instrument's own Vowel parameter.
pub(crate) fn vowel(_lyric: &str) -> u8 {
    255
}

pub(crate) fn pitch(points: &[VocalPitchPoint], at: f32) -> f32 {
    let after = points.partition_point(|p| p.offset < at);
    let left = after.checked_sub(1).and_then(|i| points.get(i));
    let right = points.get(after);
    match (left, right) {
        (Some(a), Some(b)) if b.offset > a.offset => {
            let t = ((at - a.offset) / (b.offset - a.offset)).clamp(0.0, 1.0);
            a.cents + (b.cents - a.cents) * t * t * (3.0 - 2.0 * t)
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
        assert_eq!(vowel("hello"), 255);
        for text in ["", "hello", "あ", "a", "bonjour", "你好", "مرحبا"] {
            assert_eq!(vowel(text), 255);
        }
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
                Action::Vocal { expression, key: 60, .. } if expression.cents == -100.0 && expression.vowel == 255)));
        }
        song.channels[1].lanes[0].clips[0].muted = true;
        out.clear();
        actions(&song, 48000.0, 120.0, &mut out).unwrap();
        assert!(out.is_empty());
    }
}

pub(super) struct Timeline {
    pub channel: usize,
    pub instance_id: i32,
    pub key: u8,
    pub changes: Vec<(u64, VocalExpression)>,
}

/// Built before playback. Note-ons reset expression, so removing a curve in an edit cannot
/// leave the old voice bent. Binary search restores the most recent value without allocation.
pub(super) fn timelines(song: &Song, actions: &[ScheduledAction]) -> Vec<Timeline> {
    let mut timelines =
        std::collections::BTreeMap::<(usize, i32, u8), Vec<(u64, VocalExpression)>>::new();
    for event in actions {
        let (instance_id, key, expression) = match event.action {
            Action::Vocal {
                instance_id,
                key,
                expression,
            } => (instance_id, key, expression),
            Action::NoteOn {
                instance_id, key, ..
            } if song.channels[event.channel]
                .rack
                .modules
                .iter()
                .any(|m| m.instance_id == instance_id && m.type_id == 53) =>
            {
                (instance_id, key, VocalExpression::default())
            }
            _ => continue,
        };
        timelines
            .entry((event.channel, instance_id, key))
            .or_default()
            .push((event.frame, expression));
    }
    timelines
        .into_iter()
        .map(|((channel, instance_id, key), changes)| Timeline {
            channel,
            instance_id,
            key,
            changes,
        })
        .collect()
}

#[cfg(test)]
mod binding_tests {
    use super::*;
    #[test]
    fn vocal_lyric_vowels_preserve_the_panel_vowel_instead_of_overriding_it() {
        assert_eq!(["A", "E", "I", "O", "U"].map(vowel), [255; 5]);
    }
    #[test]
    fn vocal_restore_timeline_resets_at_the_next_note_on() {
        use crate::song::{ChannelKind, SynthKind};
        let mut song = Song::default();
        song.add_channel(ChannelKind::Synth(SynthKind::VocalSynth));
        let id = song.channels[1]
            .rack
            .modules
            .iter()
            .find(|m| m.type_id == 53)
            .unwrap()
            .instance_id;
        let action = |frame, action| ScheduledAction {
            frame,
            channel: 1,
            action,
        };
        let events = [
            action(
                0,
                Action::Vocal {
                    instance_id: id,
                    key: 60,
                    expression: VocalExpression {
                        cents: 300.0,
                        ..Default::default()
                    },
                },
            ),
            action(
                100,
                Action::NoteOn {
                    instance_id: id,
                    key: 60,
                    velocity: 1.0,
                    pan: 0.5,
                },
            ),
        ];
        let held = timelines(&song, &events);
        assert_eq!(held[0].changes[0].1.cents, 300.0);
        assert_eq!(held[0].changes[1].1.cents, 0.0);
    }
}
