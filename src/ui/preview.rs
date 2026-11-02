//! Preview (SOLO CLIP): the current clip of the current track played alone over its span, the clip as the playlist
//! draws it (issue #46, [`span`]). The engine gets a filtered copy of the song; the song itself never changes
//! except its play loop, which switching Preview on or moving it saves clamped into the clip ([`clamped_loop`]).

use crate::song::{Clip, Song};
use crate::ui::views::{Slots, View};

/// A clip of the song: its channel, its lane in the channel and its id in the lane.
///
/// The channel and lane are indices: the app re-keys a kept `ClipKey` (its Preview) when channels or tracks move
/// (`app/tracks.rs`), or it would name another clip.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct ClipKey {
    pub channel: usize,
    pub lane: usize,
    pub id: u32,
}

impl ClipKey {
    pub fn clip(self, song: &Song) -> Option<&Clip> {
        let lane = song.channels.get(self.channel)?.lanes.get(self.lane)?;
        lane.clips.iter().find(|c| c.id == self.id)
    }
}

/// The current clip: the one the piano roll, the drum editor, the automation editor or the audio editor shown edits, if
/// it exists. The playlist and the mixer edit none.
pub fn current(view: View, slots: &Slots, song: &Song) -> Option<ClipKey> {
    let key = match view {
        View::PianoRoll(channel) => {
            let roll = slots.piano_roll.rolls.get(&channel)?;
            ClipKey {
                channel: roll.channel,
                lane: roll.lane,
                id: roll.clip_id?,
            }
        }
        View::DrumEditor(channel) => {
            let step = &slots.drum_editor.step;
            ClipKey {
                channel,
                lane: step.lane,
                id: step.clip?,
            }
        }
        View::AutomationEditor(channel) => slots.automation_editor.current(song, channel)?,
        View::AudioEditor(channel) => slots.audio_editor.current(song, channel)?,
        View::Playlist | View::Mixer => return None,
    };
    key.clip(song).is_some().then_some(key)
}

/// The span Preview plays, `[s, e)` in song ticks: the clip as the playlist draws it, from its start over its visible
/// length (0: the loop length, [`Clip::visible_ticks`]), repeats included.
///
/// Issue #46: SOLO CLIP loops the clip as drawn. The old span was one loop length from where the content
/// starts (clip start − content offset), so a clip trimmed shorter than its loop, or one with a content offset, played
/// outside its box. `None` without a loop length ([`Clip::loop_ticks`]: nothing plays) or a visible length under a tick.
pub fn span(clip: &Clip) -> Option<(i64, i64)> {
    clip.loop_ticks()?;
    let s = i64::from(clip.start_tick);
    Some((s, s + clip.visible_ticks().round() as i64)).filter(|(s, e)| s < e)
}

/// The range Preview loops over: each song loop marker is kept if it lies in [s, e]; otherwise loop start becomes s and
/// loop end becomes e, each judged separately; if the result is empty, [s, e] is used. No play loop is the file's
/// (0, 0). `None` without a [`span`].
pub fn looped(clip: &Clip, play_loop: Option<(u32, u32)>) -> Option<(i64, i64)> {
    let (s, e) = span(clip)?;
    let (a, b) = play_loop.unwrap_or_default();
    let keep = |m: u32, or: i64| {
        Some(i64::from(m))
            .filter(|m| (s..=e).contains(m))
            .unwrap_or(or)
    };
    let (a, b) = (keep(a, s), keep(b, e));
    Some(if a < b { (a, b) } else { (s, e) })
}

/// The song's play loop as Preview saves it when it is switched on or moves to another clip: the stored song loop
/// markers are first clamped into the [`span`] [s, e] and saved as the song's loop range; an empty result becomes no
/// range (0, 0). `None` also without a span.
pub fn clamped_loop(clip: &Clip, play_loop: Option<(u32, u32)>) -> Option<(u32, u32)> {
    let (s, e) = span(clip)?;
    let (a, b) = play_loop.unwrap_or_default();
    let at = |m: u32| u32::try_from(i64::from(m).clamp(s, e)).ok();
    at(a).zip(at(b)).filter(|(a, b)| a < b)
}

/// Preview's loop as the engine plays it in the [`solo`] song: [`looped`] in song ticks. `None` without a [`span`].
pub fn engine_loop(clip: &Clip, play_loop: Option<(u32, u32)>) -> Option<(u32, u32)> {
    let (s, e) = looped(clip, play_loop)?;
    let tick = |t: i64| u32::try_from(t.max(0)).unwrap_or(u32::MAX);
    Some((tick(s), tick(e))).filter(|(s, e)| s < e)
}

/// What the engine plays while previewing `key`: `song` with every clip but `key`'s removed. That one stays as it is,
/// so inside its [`span`] it plays exactly as in the song (same content offset and repeats, an audio clip's sample
/// included; issue #46). A muted clip stays silent: it is removed too, as the engine plays no muted clip
/// anyway. `None` when the clip is gone or has no span. Its play loop is Preview's ([`engine_loop`]), which the engine
/// reads at build.
pub fn solo(song: &Song, key: ClipKey) -> Option<Song> {
    let clip = key.clip(song)?;
    span(clip)?;
    let play_loop = engine_loop(clip, song.play_loop);
    let kept = (!clip.muted).then(|| clip.clone());
    let mut solo = Song {
        play_loop,
        ..song.clone()
    };
    for lane in solo.channels.iter_mut().flat_map(|c| &mut c.lanes) {
        lane.clips.clear();
    }
    solo.channels[key.channel].lanes[key.lane]
        .clips
        .extend(kept);
    Some(solo)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::song::{AutomationTarget, ChannelKind, Event, Lane, LaneKind, SynthKind};

    fn note(pos: u32, key: u8) -> Event {
        Event::Note {
            pos,
            len_beats: 1.0,
            key,
            velocity: Event::DEFAULT_VELOCITY,
            pan: Event::DEFAULT_PAN,
            slide: false,
        }
    }

    /// Master (with an automation clip), Drums (1) with a clip, MiniSynth (2) with clips 1 and 2.
    fn song() -> Song {
        let mut song = Song::default();
        song.add_channel(ChannelKind::Drums);
        song.add_channel(ChannelKind::Synth(SynthKind::MiniSynth));
        song.channels[0].lanes[0].add_clip(0, 8.0);
        song.channels[1].lanes[0].add_clip(0, 4.0);
        let synth = &mut song.channels[2];
        synth.lanes[0].add_clip(512, 4.0).events = vec![note(0, 60), note(384, 67)];
        synth.lanes[0].add_clip(1024, 4.0).events = vec![note(0, 72)];
        let mut automation = Lane::new(LaneKind::Automation);
        automation.target = Some(AutomationTarget {
            instance_id: 0,
            param: 0,
        });
        automation.add_clip(0, 4.0);
        synth.lanes.push(automation);
        song
    }

    const SYNTH_1: ClipKey = ClipKey {
        channel: 2,
        lane: 0,
        id: 1,
    };

    fn clip(start_tick: u32, offset_beats: f64, loop_len_beats: f64) -> Clip {
        let mut lane = Lane::new(LaneKind::Notes);
        let clip = lane.add_clip(start_tick, loop_len_beats);
        clip.offset_beats = offset_beats;
        clip.clone()
    }

    #[test]
    fn the_span_is_the_clip_as_drawn() {
        // Issue #46: s = start, e = s + visible length, 0 being the loop (ticks, 128 a beat).
        assert_eq!(span(&clip(512, 0.0, 4.0)), Some((512, 1024)));
        // The content offset plays no part.
        assert_eq!(span(&clip(512, 1.0, 4.0)), Some((512, 1024)));
        assert_eq!(span(&clip(0, 1.0, 4.0)), Some((0, 512)));
        assert_eq!(span(&clip(64, 1.0, 2.0)), Some((64, 320)));
        // Shown longer than its loop: every repeat; trimmed shorter: only what shows.
        let mut long = clip(512, 0.0, 4.0);
        long.visible_len_beats = 9.0;
        assert_eq!(span(&long), Some((512, 1664)));
        long.visible_len_beats = 2.0;
        assert_eq!(span(&long), Some((512, 768)));
        // No loop length, no span.
        assert_eq!(span(&clip(512, 0.0, 0.0)), None);
        assert_eq!(span(&clip(512, 0.0, f64::NAN)), None);
        // Nor under one tick: no loop, as the playlist and the audio bridge have it (`Clip::loop_ticks`).
        assert_eq!(span(&clip(512, 0.0, 1e-6)), None);
        assert_eq!(span(&clip(512, 0.0, 1.0 / 128.0)), Some((512, 513)));
    }

    #[test]
    fn however_far_the_content_is_offset_the_span_starts_at_the_clip() {
        assert_eq!(span(&clip(0, 4.0, 4.0)), Some((0, 512)));
        assert_eq!(span(&clip(0, 5.0, 4.0)), Some((0, 512)));
        assert_eq!(span(&clip(128, 10.0, 4.0)), Some((128, 640)));
    }

    #[test]
    fn each_loop_marker_outside_the_span_becomes_the_spans_own() {
        let c = clip(512, 0.0, 4.0);
        // [s, e] = [512, 1024], edges included.
        for (play_loop, want) in [
            (Some((640, 768)), (640, 768)),
            (Some((512, 1024)), (512, 1024)),
            // The start before s becomes s, the end after e becomes e: each judged alone.
            (Some((256, 768)), (512, 768)),
            (Some((768, 1152)), (768, 1024)),
            (Some((0, 2000)), (512, 1024)),
            (Some((0, 128)), (512, 1024)),
            // Empty once judged: the span.
            (Some((256, 512)), (512, 1024)),
            (Some((1024, 1152)), (512, 1024)),
            // No loop is (0, 0).
            (None, (512, 1024)),
        ] {
            assert_eq!(looped(&c, play_loop), Some(want), "{play_loop:?}");
        }
        // An offset clip at 0: its span is [0, 512].
        let early = clip(0, 5.0, 4.0);
        assert_eq!(looped(&early, Some((0, 256))), Some((0, 256)));
        assert_eq!(looped(&early, Some((128, 512))), Some((128, 512)));
        assert_eq!(looped(&early, None), Some((0, 512)));
        assert_eq!(looped(&clip(512, 0.0, 0.0), Some((640, 768))), None);
    }

    #[test]
    fn the_engine_loops_previews_loop_in_song_ticks() {
        let c = clip(512, 0.0, 4.0);
        assert_eq!(engine_loop(&c, Some((640, 768))), Some((640, 768)));
        assert_eq!(engine_loop(&c, Some((256, 768))), Some((512, 768)));
        assert_eq!(engine_loop(&c, None), Some((512, 1024)));
        // However far the content is offset, the clip's box.
        assert_eq!(engine_loop(&clip(0, 5.0, 4.0), None), Some((0, 512)));
        assert_eq!(engine_loop(&clip(0, 8.0, 4.0), None), Some((0, 512)));
        assert_eq!(engine_loop(&clip(512, 0.0, 0.0), Some((640, 768))), None);
    }

    #[test]
    fn the_saved_loop_is_the_songs_clamped_into_the_span() {
        // [s, e] = [512, 1024].
        let c = clip(512, 0.0, 4.0);
        for (play_loop, want) in [
            (Some((640, 768)), Some((640, 768))),
            (Some((256, 768)), Some((512, 768))),
            (Some((768, 2000)), Some((768, 1024))),
            (Some((0, 2000)), Some((512, 1024))),
            // Empty: no range.
            (Some((0, 512)), None),
            (Some((1024, 1152)), None),
            (None, None),
        ] {
            assert_eq!(clamped_loop(&c, play_loop), want, "{play_loop:?}");
        }
        // The offset plays no part: an offset clip at 0 clamps into [0, 512].
        let early = clip(0, 1.0, 4.0);
        assert_eq!(clamped_loop(&early, Some((0, 512))), Some((0, 512)));
        assert_eq!(clamped_loop(&early, Some((512, 640))), None);
        assert_eq!(
            clamped_loop(&clip(0, 9.0, 4.0), Some((0, 512))),
            Some((0, 512))
        );
        // Trimmed to two beats: [512, 768].
        let mut trimmed = c;
        trimmed.visible_len_beats = 2.0;
        assert_eq!(clamped_loop(&trimmed, Some((640, 1000))), Some((640, 768)));
    }

    #[test]
    fn the_solo_song_keeps_only_the_clip_as_it_is() {
        let mut song = song();
        song.channels[2].lanes[0].clips[0].offset_beats = 1.0;
        song.channels[2].lanes[0].clips[0].visible_len_beats = 6.0;
        let solo = solo(&song, SYNTH_1).expect("the clip exists");
        // Channels, racks and everything but the clips and the play loop are the song's.
        assert_eq!(solo.channels.len(), song.channels.len());
        // The engine loops the span [512, 1280), which no play loop narrows.
        assert_eq!(song.play_loop, None);
        assert_eq!(solo.play_loop, Some((512, 1280)));
        assert_eq!(solo.tempo_bpm, song.tempo_bpm);
        assert_eq!(solo.channels[2].rack, song.channels[2].rack);
        let clips: Vec<(usize, usize, &Clip)> = (solo.channels.iter().enumerate())
            .flat_map(|(c, ch)| {
                ch.lanes
                    .iter()
                    .enumerate()
                    .map(move |(l, lane)| (c, l, lane))
            })
            .flat_map(|(c, l, lane)| lane.clips.iter().map(move |clip| (c, l, clip)))
            .collect();
        assert_eq!(clips.len(), 1, "{clips:?}");
        let (c, l, kept) = clips[0];
        assert_eq!((c, l, kept.id), (2, 0, 1));
        // As in the song: its start, offset and visible length, so content 384 at 768 and, wrapped, content 0 at 896.
        assert_eq!(kept, &song.channels[2].lanes[0].clips[0]);
        let played: Vec<u64> = kept.events_in_song_range(0, 4096).map(|(t, _)| t).collect();
        assert_eq!(played, [768, 896]);
    }

    #[test]
    fn an_offset_solo_clip_at_the_song_start_keeps_its_offset() {
        let mut song = song();
        let c = &mut song.channels[2].lanes[0].clips[0];
        (c.start_tick, c.offset_beats) = (0, 5.0);
        let solo = solo(&song, SYNTH_1).expect("the clip exists");
        let kept = &solo.channels[2].lanes[0].clips[0];
        assert_eq!((kept.start_tick, kept.offset_beats), (0, 5.0));
        // Five beats into its one-bar content, as in the song: content 384 at 256, content 0 at 384.
        let played: Vec<u64> = kept.events_in_song_range(0, 4096).map(|(t, _)| t).collect();
        assert_eq!(played, [256, 384]);
    }

    #[test]
    fn an_audio_clip_keeps_its_offset() {
        let mut song = song();
        song.add_channel(ChannelKind::AudioClip);
        let key = ClipKey {
            channel: 3,
            lane: 0,
            id: 1,
        };
        let lane = &mut song.channels[3].lanes[0];
        assert_eq!(lane.kind, LaneKind::Audio);
        lane.add_clip(512, 4.0).offset_beats = 1.0;
        // Issue #46: previewed as in the song, sample position 128 plays at 512.
        for (start, offset) in [(512, 1.0), (0, 1.0), (0, 5.0)] {
            let clip = &mut song.channels[3].lanes[0].clips[0];
            (clip.start_tick, clip.offset_beats) = (start, offset);
            let solo = solo(&song, key).expect("the clip exists");
            let kept = &solo.channels[3].lanes[0].clips[0];
            assert_eq!(
                (kept.start_tick, kept.offset_beats),
                (start, offset),
                "{start} {offset}"
            );
        }
    }

    #[test]
    fn the_current_clip_is_the_one_the_editor_shown_edits() {
        use crate::ui::piano_roll::PianoRoll;
        let song = song();
        let mut slots = Slots::default();
        // The roll of channel 2 edits clip 2; a roll whose clip is gone, or that has none yet, has no current clip.
        slots
            .piano_roll
            .rolls
            .insert(2, PianoRoll::new(2, 0, Some(2)));
        assert_eq!(
            current(View::PianoRoll(2), &slots, &song),
            Some(ClipKey { id: 2, ..SYNTH_1 })
        );
        for id in [Some(9), None] {
            slots.piano_roll.rolls.insert(2, PianoRoll::new(2, 0, id));
            assert_eq!(current(View::PianoRoll(2), &slots, &song), None, "{id:?}");
        }
        // A roll not drawn yet.
        assert_eq!(current(View::PianoRoll(1), &slots, &song), None);
        // The drum editor's lane and clip id; none held, or one that is gone, is no current clip.
        let drums = ClipKey {
            channel: 1,
            lane: 0,
            id: 1,
        };
        assert_eq!(current(View::DrumEditor(1), &slots, &song), None);
        slots.drum_editor.step.clip = Some(1);
        assert_eq!(current(View::DrumEditor(1), &slots, &song), Some(drums));
        slots.drum_editor.step.clip = Some(3);
        assert_eq!(current(View::DrumEditor(1), &slots, &song), None);
        // Neither the playlist nor the mixer edits a clip.
        assert_eq!(current(View::Playlist, &slots, &song), None);
        assert_eq!(current(View::Mixer, &slots, &song), None);
    }

    #[test]
    fn the_automation_and_audio_editors_current_clip_is_the_one_held_for_their_channel() {
        let mut song = song();
        song.add_channel(ChannelKind::AudioClip);
        song.channels[3].lanes[0].add_clip(0, 4.0);
        let mut slots = Slots::default();
        // Nothing held yet.
        assert_eq!(current(View::AutomationEditor(2), &slots, &song), None);
        assert_eq!(current(View::AudioEditor(3), &slots, &song), None);
        // Channel 2's automation lane (1) and channel 3's audio lane, clip 1 each.
        slots.automation_editor.clips.insert(2, (1, 1));
        slots.audio_editor.clips.insert(3, (0, 1));
        let automation = ClipKey {
            channel: 2,
            lane: 1,
            id: 1,
        };
        let audio = ClipKey {
            channel: 3,
            lane: 0,
            id: 1,
        };
        assert_eq!(
            current(View::AutomationEditor(2), &slots, &song),
            Some(automation)
        );
        assert_eq!(current(View::AudioEditor(3), &slots, &song), Some(audio));
        // A clip that is gone, or a lane of another kind (the notes lane's clip 1), has none.
        for held in [(1, 9), (0, 1)] {
            slots.automation_editor.clips.insert(2, held);
            assert_eq!(
                current(View::AutomationEditor(2), &slots, &song),
                None,
                "{held:?}"
            );
        }
        slots.audio_editor.clips.insert(3, (0, 9));
        assert_eq!(current(View::AudioEditor(3), &slots, &song), None);
    }

    #[test]
    fn a_muted_clip_stays_silent_and_a_gone_clip_has_no_solo() {
        let mut song = song();
        song.channels[2].lanes[0].clips[0].muted = true;
        let solo = solo(&song, SYNTH_1).expect("the clip exists");
        assert!(
            solo.channels
                .iter()
                .flat_map(|c| &c.lanes)
                .all(|l| l.clips.is_empty())
        );
        let gone = ClipKey { id: 9, ..SYNTH_1 };
        assert_eq!(super::solo(&song, gone), None);
        let no_lane = ClipKey { lane: 5, ..SYNTH_1 };
        assert_eq!(super::solo(&song, no_lane), None);
        assert_eq!(gone.clip(&song), None);
        assert_eq!(SYNTH_1.clip(&song).map(|c| c.start_tick), Some(512));
    }
}
