//! Slide notes: the sequencer sends no note-on or note-off for a slide note (EVN2 @19 = 1); it sends one slide event
//! per tick, which every instrument of the channel applies to every voice (its pads never get them).

use super::EngineError;
use super::prepare::{Action, MAX_PLAYS, ScheduledAction, beat_frame};
use crate::audio_bridge::playing_clips;
use crate::song::{Event, LaneKind, PPQ, Song};

/// A note of a notes lane as the slide law reads it: song ticks (its end may fall between two), and its bytes. `len`
/// is its whole length and `cut` its clip's end: it sounds until `end`, the earlier of the two ends.
#[derive(Clone, Copy)]
struct Played {
    start: u64,
    len: f64,
    cut: f64,
    end: f64,
    key: u8,
    velocity: u8,
    pan: u8,
    slide: bool,
}

impl Played {
    /// `event`, at song tick `tick` of a clip that ends at tick `cut`, as the slide law reads it; `None` for an event
    /// that is not a note, and for a note whose length is not finite.
    fn of(tick: u64, event: &Event, cut: f64) -> Option<Self> {
        let Event::Note {
            len_beats,
            key,
            velocity,
            pan,
            slide,
            ..
        } = *event
        else {
            return None;
        };
        let len = len_beats * f64::from(PPQ);
        len_beats.is_finite().then_some(Self {
            start: tick,
            len,
            cut,
            end: (tick as f64 + len).min(cut),
            key,
            velocity: velocity.max(1),
            pan,
            slide,
        })
    }

    /// It sounds at tick `tick`: start ≤ tick < end.
    fn sounds_at(&self, tick: u64) -> bool {
        !self.slide && self.start <= tick && (tick as f64) < self.end
    }

    /// N: its length in ticks, at least 1.
    fn ticks(&self) -> u64 {
        (self.len.round() as u64).max(1)
    }

    /// How many of its N slide events come before its clip's end: those at ticks start…start + N − 1 under `cut`.
    fn kept(&self) -> u64 {
        self.ticks()
            .min((self.cut - self.start as f64).ceil().max(0.0) as u64)
    }
}

/// The slide events of `channel`'s notes lanes up to `end_beat`, as actions:
/// - the anchor is the highest-key non-slide note sounding at the slide's start; without one the slide does nothing;
/// - one event per tick, from the slide's start tick to its end tick − 1 (N = its length in ticks, at least 1), event
///   k = 1…N at prev + (k/N)·(target − prev): pitch slideKey − anchorKey semitones, pan (slidePan − anchorPan)/254,
///   velocity (slideVel − anchorVel)/255, from the bytes;
/// - prev carries on to the next slide while its anchor still sounds at that slide's start; a new anchor resets it to 0;
/// - nothing follows the last event: the offsets stay;
/// - a slide past its clip's end keeps the rate its whole length gives (the glide time is the slide note's length) and
///   loses the events at or past the clip end, as a note's off is clamped there; a note sounds only to its clip's end,
///   so an anchor too.
///
/// The run always ends at the end tick. slides are read from the notes as stored, before the note
/// effects, which rewrite only notes (a Transpose above shifts slide and anchor alike). A song whose slides would make
/// more events than the play budget is refused (`TooManyEvents`). A note whose length is not finite (in memory
/// only: a file's reads as 0) is skipped, slide or anchor, as the note path skips it.
pub(super) fn actions(
    song: &Song,
    channel: usize,
    end_beat: f64,
    sample_rate: f32,
    tempo_bpm: f32,
    actions: &mut Vec<ScheduledAction>,
) -> Result<(), EngineError> {
    let ppq = f64::from(PPQ);
    let notes = played(song, channel, end_beat);
    let slides = notes.iter().filter(|note| note.slide);
    if slides
        .clone()
        .map(Played::kept)
        .fold(0, u64::saturating_add)
        > MAX_PLAYS as u64
    {
        return Err(EngineError::TooManyEvents);
    }
    let mut prev = [0.0_f32; 3];
    for (slide, anchor, new) in anchored(&notes) {
        if new {
            prev = [0.0; 3];
        }
        let Some(a) = anchor else {
            continue;
        };
        let target = [
            f32::from(slide.key) - f32::from(a.key),
            (f32::from(slide.pan) - f32::from(a.pan)) / 254.0,
            (f32::from(slide.velocity) - f32::from(a.velocity)) / 255.0,
        ];
        let n = slide.ticks();
        let at = |k: u64| {
            let t = k as f32 / n as f32;
            [0, 1, 2].map(|i| prev[i] + t * (target[i] - prev[i]))
        };
        for k in 1..=slide.kept() {
            let [pitch, pan, velocity] = at(k);
            let tick = slide.start + k - 1;
            actions.push(ScheduledAction {
                frame: beat_frame(tick as f64 / ppq, sample_rate, tempo_bpm)?,
                channel,
                action: Action::Slide {
                    key: slide.key,
                    pitch,
                    pan,
                    velocity,
                },
            });
        }
        prev = at(slide.kept());
    }
    Ok(())
}

/// The notes of `channel`'s notes lanes up to `end_beat`, as the slide law reads them, in start order.
fn played(song: &Song, channel: usize, end_beat: f64) -> Vec<Played> {
    let mut notes = Vec::new();
    let lanes = song.channels[channel].lanes.iter();
    for lane in lanes.filter(|lane| lane.kind == LaneKind::Notes) {
        for clip in playing_clips(lane) {
            let cut = f64::from(clip.start_tick) + clip.visible_ticks();
            let to = (end_beat * f64::from(PPQ)).ceil() as u64;
            let events = clip.events_in_song_range(0, to);
            notes.extend(events.filter_map(|(tick, event)| Played::of(tick, event, cut)));
        }
    }
    notes.sort_by_key(|note| note.start);
    notes
}

/// Each slide of `notes` (in start order) with its anchor, and whether the anchor was found anew there (prev resets):
/// the anchor stays while it still sounds at the slide's start, else it is the highest-key non-slide note sounding
/// there, if any.
fn anchored(notes: &[Played]) -> Vec<(Played, Option<Played>, bool)> {
    let mut anchor: Option<Played> = None;
    let mut out = Vec::new();
    for slide in notes.iter().filter(|note| note.slide) {
        let new = !anchor.is_some_and(|anchor| anchor.sounds_at(slide.start));
        if new {
            let sounding = notes.iter().filter(|note| note.sounds_at(slide.start));
            anchor = sounding.max_by_key(|note| note.key).copied();
        }
        out.push((*slide, anchor, new));
    }
    out
}

/// `Burn Notes to Track`: the slides of `channel`'s notes lanes (up to `end_beat`) that start in song ticks [`start`,
/// `end`), for a clip there whose other notes are `notes` (positions from `start`). Playback plays a slide as an offset
/// from its anchor, read from the notes as stored, so each keeps its key, pan and velocity less its anchor's, now from
/// its anchor among `notes` by the same rule: a note effect that moved the anchor moves the slide with it, and with
/// none they are unchanged. A slide with no anchor on either side does nothing and is left out.
// ponytail: a slide that ran past its own clip's end, which cut its events there, runs on to the span's end; it would
// need a length of its own and another rate to stop there.
pub(super) fn burned(
    song: &Song,
    channel: usize,
    end_beat: f64,
    notes: &[Event],
    start: u32,
    end: u32,
) -> Vec<Event> {
    let span = u64::from(start)..u64::from(end);
    let aims: Vec<(Played, Played)> = (anchored(&played(song, channel, end_beat)).into_iter())
        .filter(|(slide, ..)| span.contains(&slide.start))
        .filter_map(|(slide, anchor, _)| Some((slide, anchor?)))
        .collect();
    let cut = f64::from(end);
    let mut clip: Vec<Played> = (notes.iter())
        .filter_map(|event| Played::of(span.start + u64::from(event.pos()), event, cut))
        .chain(aims.iter().map(|&(slide, _)| slide))
        .collect();
    clip.sort_by_key(|note| note.start);
    // The byte `to` + `by` − `from`, clamped to `range`.
    let moved = |to: u8, by: u8, from: u8, range: std::ops::RangeInclusive<i16>| {
        let value = i16::from(to) + i16::from(by) - i16::from(from);
        value.clamp(*range.start(), *range.end()) as u8
    };
    // The slides come in the same order on both sides: a stable sort by start.
    (aims.iter().zip(anchored(&clip)))
        .filter_map(|(&(slide, a), (_, b, _))| {
            let b = b?;
            Some(Event::Note {
                pos: (slide.start - span.start) as u32,
                len_beats: slide.len / f64::from(PPQ),
                key: moved(b.key, slide.key, a.key, 0..=127),
                velocity: moved(b.velocity, slide.velocity, a.velocity, 1..=255),
                pan: moved(b.pan, slide.pan, a.pan, 0..=255),
                slide: true,
            })
        })
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::song::{ChannelKind, SynthKind};

    fn note(tick: u32, ticks: u32, key: u8, velocity: u8, pan: u8, slide: bool) -> Event {
        Event::Note {
            pos: tick,
            len_beats: f64::from(ticks) / 128.0,
            key,
            velocity,
            pan,
            slide,
        }
    }

    /// The slide events of a MiniSynth channel holding `notes`, as (tick, [pitch, pan, velocity]) at 120 BPM and
    /// 48 kHz (a tick is 187.5 frames).
    fn slides(notes: &[Event]) -> Vec<(u64, [f32; 3])> {
        let mut song = synth();
        song.channels[1].lanes[0]
            .add_clip(0, 16.0)
            .events
            .extend_from_slice(notes);
        song_slides(&song)
    }

    /// 120 BPM and a MiniSynth on channel 1.
    fn synth() -> Song {
        let mut song = Song {
            tempo_bpm: 120.0,
            ..Song::default()
        };
        song.add_channel(ChannelKind::Synth(SynthKind::MiniSynth));
        song
    }

    /// [`slides`] of channel 1 of `song`, up to beat 16.
    fn song_slides(song: &Song) -> Vec<(u64, [f32; 3])> {
        let mut prepared = Vec::new();
        actions(song, 1, 16.0, 48_000.0, 120.0, &mut prepared).unwrap();
        (prepared.iter())
            .map(|action| match action.action {
                Action::Slide {
                    pitch,
                    pan,
                    velocity,
                    ..
                } => (
                    (action.frame as f64 / 187.5).floor() as u64,
                    [pitch, pan, velocity],
                ),
                _ => unreachable!(),
            })
            .collect()
    }

    fn close(actual: &[(u64, [f32; 3])], expected: &[(u64, [f32; 3])]) {
        assert_eq!(actual.len(), expected.len(), "{actual:?}");
        for ((tick, got), (at, want)) in actual.iter().zip(expected) {
            assert_eq!(tick, at);
            for (g, w) in got.iter().zip(want) {
                assert!((g - w).abs() < 1e-6, "tick {tick}: {got:?} vs {want:?}");
            }
        }
    }

    #[test]
    fn a_slide_steps_linearly_to_its_targets_one_tick_at_a_time() {
        // Anchor key 60, velocity 100, pan 127; slide key 67, velocity 151, pan 254, 4 ticks from tick 128.
        let notes = [
            note(0, 512, 60, 100, 127, false),
            note(128, 4, 67, 151, 254, true),
        ];
        let target = [7.0, 127.0 / 254.0, 51.0 / 255.0];
        let step = |k: f32| target.map(|t| t * k / 4.0);
        let expected = [
            (128, step(1.0)),
            (129, step(2.0)),
            (130, step(3.0)),
            (131, step(4.0)),
        ];
        close(&slides(&notes), &expected);
    }

    #[test]
    fn the_anchor_is_the_highest_non_slide_note_sounding_at_the_slide_start() {
        // 72 ends at the slide's start and 76 starts after it: 64 is the highest sounding there.
        let notes = [
            note(0, 512, 60, 100, 127, false),
            note(0, 256, 64, 100, 127, false),
            note(0, 128, 72, 100, 127, false),
            note(129, 512, 76, 100, 127, false),
            note(128, 1, 67, 100, 127, true),
        ];
        close(&slides(&notes), &[(128, [3.0, 0.0, 0.0])]);
        // Nothing sounding at the start: the slide does nothing.
        assert!(
            slides(&[
                note(0, 64, 60, 100, 127, false),
                note(64, 4, 67, 100, 127, true)
            ])
            .is_empty()
        );
    }

    #[test]
    fn prev_carries_on_the_same_anchor_and_a_new_anchor_resets_it() {
        // Slides to +7 then to +4 on anchor 60; then 60 ends, and a slide on the new anchor 62 starts from 0.
        let notes = [
            note(0, 300, 60, 100, 127, false),
            note(100, 2, 67, 100, 127, true),
            note(200, 2, 64, 100, 127, true),
            note(300, 300, 62, 100, 127, false),
            note(400, 2, 64, 100, 127, true),
        ];
        let expected = [
            (100, [3.5, 0.0, 0.0]),
            (101, [7.0, 0.0, 0.0]),
            (200, [5.5, 0.0, 0.0]),
            (201, [4.0, 0.0, 0.0]),
            (400, [1.0, 0.0, 0.0]),
            (401, [2.0, 0.0, 0.0]),
        ];
        close(&slides(&notes), &expected);
    }

    /// A slide past its clip's end is cut there, not rescaled. Its glide keeps the rate its whole length gives (the
    /// glide time is the slide note's length); the events past the clip end are dropped.
    #[test]
    fn a_slide_past_its_clip_end_is_cut_at_its_rate() {
        // The 16-beat clip ends at tick 2048; the slide starts 4 ticks before, 8 ticks long.
        let notes = [
            note(0, 2048, 60, 100, 127, false),
            note(2044, 8, 68, 100, 127, true),
        ];
        let expected = [1.0, 2.0, 3.0, 4.0].map(|k| (2043 + k as u64, [8.0 * k / 8.0, 0.0, 0.0]));
        close(&slides(&notes), &expected);
    }

    /// A slide cut at its clip end leaves the offsets where its last kept event put them, so the next slide under the
    /// same anchor glides on from there, not from the cut slide's target.
    #[test]
    fn a_slide_cut_at_its_clip_end_hands_on_the_offset_it_reached() {
        let mut song = synth();
        let channel = &mut song.channels[1];
        // The anchor, key 60, sounds over both slides from another lane.
        let anchor = note(0, 2048, 60, 100, 127, false);
        channel.lanes[0].add_clip(0, 16.0).events.push(anchor);
        let index = channel.add_lane(LaneKind::Notes);
        let lane = &mut channel.lanes[index];
        // A slide to 68 from tick 124, 8 ticks long, in a clip ending at tick 128: its first 4 events reach +4.
        lane.add_clip(0, 1.0)
            .events
            .push(note(124, 8, 68, 100, 127, true));
        // The next clip's slide to 72, 4 ticks from tick 128: from +4 to +12.
        lane.add_clip(128, 1.0)
            .events
            .push(note(0, 4, 72, 100, 127, true));
        let expected = [1.0, 2.0, 3.0, 4.0, 6.0, 8.0, 10.0, 12.0];
        let expected = expected.map(|pitch| [pitch, 0.0, 0.0]);
        let expected: Vec<_> = (124..132).zip(expected).collect();
        close(&song_slides(&song), &expected);
    }

    #[test]
    fn slides_past_the_play_budget_are_refused() {
        let mut song = Song::default();
        song.add_channel(ChannelKind::Synth(SynthKind::MiniSynth));
        // A slide ends with its clip, so the clip must be long: 100 000 beats are 12.8 M ticks.
        let clip = song.channels[1].lanes[0].add_clip(0, 100_000.0);
        clip.events.push(note(0, 1, 60, 100, 127, true));
        if let Event::Note { len_beats, .. } = &mut clip.events[0] {
            *len_beats = 1e9;
        }
        let result = actions(&song, 1, 100_000.0, 48_000.0, 120.0, &mut Vec::new());
        assert!(matches!(result, Err(EngineError::TooManyEvents)));
    }
}
