//! REC's live note recording. Touched keys, typed keys, MIDI notes and drum pads all come here, after they sound
//! ([`NoteRecord::keys`]). While REC is armed, the transport plays and no count-in runs, a take per recording channel
//! (the armed instrument and drum channels, else the current channel: Honeydew's own) records that channel's keys as
//! played (no quantize) into one clip of its own; a song-position jump, a change of the recording channels, arming
//! and disarming each end the takes, and the app makes the takes that recorded and end in one frame one undo step.

use super::HoneydewApp;
use super::auto_record::start;
use super::tracks::LaneShift;
use crate::song::{Channel, Clip, DrumGrid, Event, Lane, LaneKind, PPQ, Song};
use crate::ui::preview;
use crate::ui::views::SongEdit;

/// The shortest recorded length, in beats.
const MIN_BEATS: f64 = 0.01;

/// A live key, after it sounded: (channel, key, its velocity in [0, 1] at a press, `None` at a release).
pub(super) type Key = (usize, u8, Option<f32>);

/// What the frame is, as the recording sees it.
pub(super) struct Now {
    /// The current channel, and its current lane if it has one.
    pub channel: usize,
    pub lane: Option<usize>,
    /// The channels that record ([`HoneydewApp::record_channels`]); the master, 0, records nothing.
    pub channels: Vec<usize>,
    pub armed: bool,
    pub playing: bool,
    pub counting_in: bool,
    /// The song position jumped this frame (a seek).
    pub jumped: bool,
    /// The song position, in ticks.
    pub tick: u64,
}

#[derive(Default)]
pub(super) struct NoteRecord {
    /// The live keys since the last frame, in order.
    pub(super) keys: Vec<Key>,
    /// A key was pressed while REC was armed and the transport stopped: it starts playback.
    pub(super) start: bool,
    /// The keys held now, from every source: (channel, key, velocity). Play's cut drops its keys
    /// ([`HoneydewApp::cut_live_keys`]).
    pub(super) held: Vec<(usize, u8, f32)>,
    /// The takes under way, one per recording channel.
    takes: Vec<Take>,
    /// A take made a track this frame and holds its index as it is now: the app's re-key of that change, which comes
    /// next, leaves the takes' clips where they are ([`Self::rekey`]).
    shifted: bool,
}

/// A take under way.
struct Take {
    /// Its channel's id.
    channel: i32,
    /// The song position it began at, and the last one it saw.
    start: u64,
    last: u64,
    /// Its clip (lane, id), once a note made it.
    clip: Option<(usize, u32)>,
    /// The notes still open: key, position in the clip, and row (the note's key, or the hit's slot).
    open: Vec<(u8, u32, u8)>,
}

impl NoteRecord {
    /// Ends each take the frame `now` ends: Stop and seeks leave its open notes at length 0; the loop wrap closes
    /// them where the take last was; disarming and its channel no longer recording close them now. Whether an ended
    /// take recorded anything, which the app closes as one undo step before the next takes record.
    pub(super) fn end(&mut self, song: &mut Song, now: &Now) -> bool {
        let recording: Vec<i32> = (now.channels.iter())
            .filter_map(|&c| song.channels.get(c).map(|c| c.id))
            .collect();
        let bar = song.time_sig.bar_beats();
        let mut recorded = false;
        self.takes.retain_mut(|take| {
            let at = if !now.playing || now.jumped {
                None
            } else if now.tick < take.last {
                Some(take.last)
            } else if now.armed && !now.counting_in && recording.contains(&take.channel) {
                take.last = now.tick;
                return true;
            } else {
                Some(now.tick)
            };
            let ch = song.channels.iter_mut().find(|c| c.id == take.channel);
            if let (Some(at), Some(ch)) = (at, ch) {
                close(ch, take, None, at, bar);
            }
            recorded |= take.clip.is_some();
            false
        });
        recorded
    }

    /// The frame's keys ([`Self::keys`]): a take starts on each recording channel once `now` records, with every key
    /// still held on that channel as a new note at its start; then each key of a channel with a take records. `None`
    /// when the song is unchanged, else the lanes the frame added.
    pub(super) fn record(&mut self, song: &mut Song, now: &Now) -> Option<Vec<LaneShift>> {
        // The re-key that follows the frame that set it consumed it.
        debug_assert!(!self.shifted, "a lane added without the app's re-key");
        let mut changed = None;
        let recording = now.armed && now.playing && !now.counting_in;
        for &channel in now.channels.iter().filter(|&&c| c > 0 && recording) {
            let Some(ch) = song.channels.get(channel) else {
                continue;
            };
            if self.takes.iter().any(|t| t.channel == ch.id) {
                continue;
            }
            self.takes.push(Take {
                channel: ch.id,
                start: now.tick,
                last: now.tick,
                clip: None,
                open: Vec::new(),
            });
            for (c, key, velocity) in self.held.clone() {
                if c == channel {
                    self.press(song, now, channel, key, velocity, &mut changed);
                }
            }
        }
        for (channel, key, velocity) in std::mem::take(&mut self.keys) {
            self.held.retain(|h| (h.0, h.1) != (channel, key));
            if let Some(v) = velocity {
                self.held.push((channel, key, v));
            }
            let id = song.channels.get(channel).map(|c| c.id);
            let Some(take) = self.takes.iter_mut().find(|t| Some(t.channel) == id) else {
                continue;
            };
            match velocity {
                Some(v) => self.press(song, now, channel, key, v, &mut changed),
                None => {
                    let bar = song.time_sig.bar_beats();
                    if close(&mut song.channels[channel], take, Some(key), now.tick, bar) {
                        changed.get_or_insert_with(Vec::new);
                    }
                }
            }
        }
        changed
    }

    /// The takes' clips while they record, as (channel, lane, clip id).
    pub(super) fn clips(&self, song: &Song) -> Vec<(usize, usize, u32)> {
        (self.takes.iter())
            .filter_map(|take| {
                let (lane, id) = take.clip?;
                let channel = song.channels.iter().position(|c| c.id == take.channel)?;
                Some((channel, lane, id))
            })
            .collect()
    }

    /// The song was replaced: the takes, the held keys and the queued ones were the old song's channels'.
    pub(super) fn forget(&mut self) {
        *self = Self::default();
    }

    /// The channels or tracks moved (the app's re-key; `ids`: the channel ids before): the held and queued keys go with
    /// their channels, and the takes' clips with their tracks, unless a take made that change itself
    /// ([`Self::shifted`]). A key or clip whose channel or track went is dropped.
    pub(super) fn rekey(
        &mut self,
        ids: &[i32],
        channel: &dyn Fn(usize) -> Option<usize>,
        track: &dyn Fn(usize, usize) -> Option<(usize, usize)>,
    ) {
        self.held
            .retain_mut(|h| channel(h.0).map(|c| h.0 = c).is_some());
        self.keys
            .retain_mut(|k| channel(k.0).map(|c| k.0 = c).is_some());
        // Takes on other channels than the one a take added a track to keep their tracks.
        let shifted = std::mem::take(&mut self.shifted);
        for take in self.takes.iter_mut().filter(|_| !shifted) {
            let old = ids.iter().position(|&id| id == take.channel);
            take.clip = (take.clip).and_then(|(lane, id)| Some((track(old?, lane)?.1, id)));
        }
    }

    /// `song` as the engine plays it during the takes: without their clips. `None` while no take has a clip.
    pub(super) fn without_take(&self, song: &Song) -> Option<Song> {
        let clips = self.clips(song);
        if clips.is_empty() {
            return None;
        }
        let mut song = song.clone();
        for (channel, lane, id) in clips {
            if let Some(lane) = song.channels[channel].lanes.get_mut(lane) {
                lane.clips.retain(|c| c.id != id);
            }
        }
        Some(song)
    }

    /// `key` pressed at `velocity` during the take of `channel`: what its first lane records ("When notes are
    /// recorded"), a note, or a hit on each slot the key plays; nothing on an audio or automation channel. A key still
    /// open closes first. Only the current channel has a current lane.
    fn press(
        &mut self,
        song: &mut Song,
        now: &Now,
        channel: usize,
        key: u8,
        velocity: f32,
        changed: &mut Option<Vec<LaneShift>>,
    ) {
        let ch = &mut song.channels[channel];
        let take = (self.takes.iter_mut()).find(|t| t.channel == ch.id);
        let take = take.expect("a take");
        let bar = song.time_sig.bar_beats();
        let drums = match ch.lanes.first().map(|l| l.kind) {
            Some(LaneKind::Drums) => true,
            Some(LaneKind::Audio) => return,
            // A notes, automation or no first lane: notes (the piano shows on all three).
            _ => false,
        };
        let rows: Vec<u8> = match &ch.drum_kit {
            _ if !drums => vec![key],
            Some(kit) => (0..kit.slots.len())
                .filter(|&slot| kit.pad_note(slot) == Some(key))
                .filter_map(|slot| u8::try_from(slot).ok())
                .collect(),
            None => Vec::new(),
        };
        if rows.is_empty() {
            return;
        }
        close(ch, take, Some(key), now.tick, bar);
        let shifts = changed.get_or_insert_with(Vec::new);
        let (lane, id) = match take
            .clip
            .filter(|&(l, id)| clip_mut(ch, drums, l, id).is_some())
        {
            Some(clip) => clip,
            None => {
                // A drum take's clip starts from the take's start.
                let from = start(if drums { take.start } else { now.tick });
                let current = now.lane.filter(|_| channel == now.channel);
                let (lane, id, new) = choose(ch, drums, from, current, bar);
                shifts.extend(new.map(|lane| LaneShift::new(channel, lane, true)));
                self.shifted |= new.is_some();
                (lane, id)
            }
        };
        take.clip = Some((lane, id));
        let clip = clip_mut(ch, drums, lane, id).expect("chosen");
        let pos =
            u32::try_from(now.tick.saturating_sub(clip.start_tick.into())).unwrap_or(u32::MAX);
        // clamp(trunc(255·v), 1, 255): `as` truncates and saturates.
        let velocity = ((255.0 * velocity) as u8).max(1);
        for row in rows {
            let event = if drums {
                Event::Drum {
                    pos,
                    len_beats: 0.0,
                    pad: row,
                    velocity,
                    pan: Event::DEFAULT_PAN,
                    value: Event::DEFAULT_PITCH,
                    repeat: 0,
                }
            } else {
                Event::Note {
                    pos,
                    len_beats: 0.0,
                    key: row,
                    velocity,
                    pan: Event::DEFAULT_PAN,
                    slide: false,
                }
            };
            let i = clip.events.partition_point(|e| e.pos() <= pos);
            clip.events.insert(i, event);
            take.open.push((key, pos, row));
        }
        grow(clip, now.tick, bar);
    }
}

impl HoneydewApp {
    /// The channels REC's live notes record into: every armed instrument or drum channel (CHHD@1060, Honeydew's own on
    /// them), else the current channel.
    pub(super) fn record_channels(&self) -> Vec<usize> {
        let channels = self.song.channels.iter().enumerate();
        let armed = channels.filter(|(_, c)| c.kind.plays_notes() && c.file.record_armed);
        let armed: Vec<usize> = armed.map(|(i, _)| i).collect();
        if armed.is_empty() {
            vec![self.selected_channel]
        } else {
            armed
        }
    }

    /// A live key after it sounded, the drum pads' too: REC hears it at the frame's end ([`Self::record_notes`]), and a
    /// press while REC is armed and the transport stopped starts playback.
    pub(super) fn live_key(&mut self, key: Key) {
        let screen = &self.main_screen;
        self.note_record.start |= key.2.is_some() && screen.armed && !screen.playing;
        self.note_record.keys.push(key);
    }

    /// Live note recording after the frame's keys, at song tick `tick`; `jumped`: the frame seeked. A
    /// take that recorded and ends is one undo step, closed before the next take records; meanwhile its notes are an
    /// ongoing edit, and the engine plays without them (`rebuild`). The new clip is no current clip; an open editor whose
    /// empty clip the take replaced collapses, as the current clip went ([`crate::ui::views::Slots::follow`]).
    pub(super) fn record_notes(&mut self, jumped: bool, tick: u64) {
        let open = preview::current(self.nav.view, &self.slots, &self.song);
        let channel = self.selected_channel;
        let current = open
            .map(|k| (k.channel, k.lane))
            .or_else(|| self.current_track());
        let screen = &self.main_screen;
        let now = Now {
            channel,
            lane: current.filter(|c| c.0 == channel).map(|c| c.1),
            channels: self.record_channels(),
            armed: screen.armed,
            playing: screen.playing,
            counting_in: self.audio.counting_in(),
            jumped,
            tick,
        };
        // An automation take that ended this frame closes in the same step ([`Self::auto_record`]).
        let automation = std::mem::take(&mut self.auto_record.ended);
        if self.note_record.end(&mut self.song, &now) | automation {
            // A note take only adds notes, which are written as they play: no re-sync. An automation take's points
            // re-sync.
            let edit = if automation {
                SongEdit::Final
            } else {
                SongEdit::Quiet
            };
            self.song_edited(edit, tick);
        }
        let Some(added) = self.note_record.record(&mut self.song, &now) else {
            return;
        };
        self.rekey(&added);
        // A take sets the song-changed flag while it records too.
        self.files.song_changed();
        self.song_edited(SongEdit::Ongoing, tick);
    }
}

/// A notes lane: kind 0, or on a drum channel kind 3 with flag 1.
fn notes(lane: &Lane, drums: bool) -> bool {
    if drums {
        lane.kind == LaneKind::Drums && lane.grid == DrumGrid::Notes
    } else {
        lane.kind == LaneKind::Notes
    }
}

/// Clip `id` of notes lane `lane`.
fn clip_mut(ch: &mut Channel, drums: bool, lane: usize, id: u32) -> Option<&mut Clip> {
    let lane = ch.lanes.get_mut(lane).filter(|l| notes(l, drums))?;
    lane.clips.iter_mut().find(|c| c.id == id)
}

/// The clip a take's first note goes to, a new one at `start`: on the `current` lane if it is a notes lane that no clip
/// with notes covers there (an empty clip there is replaced); else on the first notes lane no clip covers there; else
/// on a new notes lane. A drum channel has no current lane here. (lane, clip id, the lane if it is new).
fn choose(
    ch: &mut Channel,
    drums: bool,
    start: u64,
    current: Option<usize>,
    bar: u32,
) -> (usize, u32, Option<usize>) {
    let covers = |c: &Clip| {
        let from = f64::from(c.start_tick);
        (from..from + c.visible_ticks()).contains(&(start as f64))
    };
    let current = current.filter(|&l| !drums && ch.lanes.get(l).is_some_and(|l| notes(l, false)));
    let replaced = current.filter(|&l| {
        let clips = &mut ch.lanes[l].clips;
        let free = !clips.iter().any(|c| covers(c) && !c.events.is_empty());
        clips.retain(|c| !free || !covers(c));
        free
    });
    let free = replaced
        .or_else(|| (ch.lanes.iter()).position(|l| notes(l, drums) && !l.clips.iter().any(covers)));
    let (lane, new) = match free {
        Some(lane) => (lane, None),
        None if drums => {
            let lane = ch.add_lane(LaneKind::Drums);
            ch.lanes[lane].grid = DrumGrid::Notes;
            (lane, Some(lane))
        }
        None => {
            let lane = ch.add_lane(LaneKind::Notes);
            (lane, Some(lane))
        }
    };
    let start = u32::try_from(start).unwrap_or(u32::MAX);
    (lane, ch.lanes[lane].add_clip(start, f64::from(bar)).id, new)
}

/// Closes the take's open notes of `key` (every one for `None`) at song tick `at`: release − note start, at least
/// [`MIN_BEATS`]. Whether one closed.
fn close(ch: &mut Channel, take: &mut Take, key: Option<u8>, at: u64, bar: u32) -> bool {
    let drums = ch.lanes.first().is_some_and(|l| l.kind == LaneKind::Drums);
    let clip = take
        .clip
        .and_then(|(lane, id)| clip_mut(ch, drums, lane, id));
    let Some(clip) = clip else {
        take.open.clear();
        return false;
    };
    let mut closed = false;
    take.open.retain(|&(k, at_pos, row)| {
        if key.is_some_and(|key| key != k) {
            return true;
        }
        let held = at.saturating_sub(u64::from(clip.start_tick) + u64::from(at_pos));
        let beats = (held as f64 / f64::from(PPQ)).max(MIN_BEATS);
        let open = clip.events.iter_mut().find_map(|e| match e {
            Event::Note {
                pos,
                len_beats,
                key: r,
                ..
            }
            | Event::Drum {
                pos,
                len_beats,
                pad: r,
                ..
            } if (*pos, *r, *len_beats) == (at_pos, row, 0.0) => Some(len_beats),
            _ => None,
        });
        if let Some(len) = open {
            *len = beats;
        }
        closed = true;
        false
    });
    if closed {
        grow(clip, at, bar);
    }
    closed
}

/// After a recorded press or release at song tick `at`: the loop fits the content (the automatic length rule), and the
/// stored length, kept as it was, becomes exactly `at` − clip start when `at` lies past it (an unset length counts as
/// 0).
fn grow(clip: &mut Clip, at: u64, bar: u32) {
    let stored = clip.visible_len_beats;
    clip.fit_loop(bar);
    let beats = at.saturating_sub(clip.start_tick.into()) as f64 / f64::from(PPQ);
    clip.visible_len_beats = if beats > stored.max(0.0) {
        beats
    } else {
        stored
    };
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::song::{ChannelKind, SynthKind};

    const BEAT: u64 = PPQ as u64;
    const SYNTH: usize = 1;
    const DRUMS: usize = 2;

    /// A MiniSynth (1, its notes lane) and Drums (2, its step lane and the default kit: slot i plays 48 + i).
    fn song() -> Song {
        let mut song = Song::default();
        song.add_channel(ChannelKind::Synth(SynthKind::MiniSynth));
        song.add_channel(ChannelKind::Drums);
        song
    }

    /// Armed and rolling on `channel` at `tick`, no current lane.
    fn now(channel: usize, tick: u64) -> Now {
        Now {
            channel,
            lane: None,
            channels: vec![channel],
            armed: true,
            playing: true,
            counting_in: false,
            jumped: false,
            tick,
        }
    }

    fn on(channel: usize, key: u8, v: f32) -> Key {
        (channel, key, Some(v))
    }

    fn off(channel: usize, key: u8) -> Key {
        (channel, key, None)
    }

    /// One frame as the app runs it: the take ends or goes on, then `keys` record. (Whether a take that recorded
    /// ended, the lanes the frame added: `None` when it changed nothing.)
    fn frame(
        rec: &mut NoteRecord,
        song: &mut Song,
        now: &Now,
        keys: &[Key],
    ) -> (bool, Option<usize>) {
        rec.keys.extend_from_slice(keys);
        let ended = rec.end(song, now);
        let added = rec.record(song, now).map(|added| added.len());
        // The app's re-key of the frame (`HoneydewApp::record_notes`): the channels and tracks stay here.
        let ids: Vec<_> = song.channels.iter().map(|c| c.id).collect();
        rec.rekey(&ids, &Some, &|c, l| Some((c, l)));
        (ended, added)
    }

    /// Channel `c`'s events on lanes of `kind`: (lane, clip start, pos, row, velocity, length).
    fn events(song: &Song, c: usize) -> Vec<(usize, u32, u32, u8, u8, f64)> {
        let lanes = song.channels[c].lanes.iter().enumerate();
        let clips = lanes.flat_map(|(l, lane)| lane.clips.iter().map(move |c| (l, c)));
        clips
            .flat_map(|(l, c)| {
                c.events.iter().filter_map(move |e| match *e {
                    Event::Note {
                        pos,
                        len_beats,
                        key: row,
                        velocity,
                        ..
                    }
                    | Event::Drum {
                        pos,
                        len_beats,
                        pad: row,
                        velocity,
                        ..
                    } => Some((l, c.start_tick, pos, row, velocity, len_beats)),
                    _ => None,
                })
            })
            .collect()
    }

    #[test]
    fn a_note_records_where_and_as_hard_as_it_was_played_with_its_held_length() {
        let mut song = song();
        let mut rec = NoteRecord::default();
        // Beat 5 + 3 ticks: no quantize; its clip on the synth's notes lane at floor(5/4)·4 = 4.
        assert_eq!(
            frame(
                &mut rec,
                &mut song,
                &now(SYNTH, 5 * BEAT + 3),
                &[on(SYNTH, 60, 0.5)]
            ),
            (false, Some(0))
        );
        let note = |len| vec![(0, 4 * PPQ, PPQ + 3, 60, 127, len)];
        assert_eq!(
            events(&song, SYNTH),
            note(0.0),
            "length 0 at the press; 255·0.5 = 127.5: 127"
        );
        frame(
            &mut rec,
            &mut song,
            &now(SYNTH, 6 * BEAT + 67),
            &[off(SYNTH, 60)],
        );
        assert_eq!(events(&song, SYNTH), note(1.5));
        // A release at its press: the shortest length. Velocity clamps into 1…255.
        frame(
            &mut rec,
            &mut song,
            &now(SYNTH, 7 * BEAT),
            &[on(SYNTH, 62, 0.001), off(SYNTH, 62)],
        );
        frame(
            &mut rec,
            &mut song,
            &now(SYNTH, 7 * BEAT),
            &[on(SYNTH, 64, 1.5)],
        );
        let [.., quiet, loud] = &events(&song, SYNTH)[..] else {
            panic!()
        };
        assert_eq!((quiet.3, quiet.4, quiet.5), (62, 1, MIN_BEATS));
        assert_eq!((loud.3, loud.4), (64, 255));
        let clip = &song.channels[SYNTH].lanes[0].clips[0];
        let Event::Note { pan, slide, .. } = clip.events[0] else {
            panic!()
        };
        assert_eq!((pan, slide), (Event::DEFAULT_PAN, false));
        assert!(
            frame(
                &mut rec,
                &mut song,
                &Now {
                    playing: false,
                    ..now(SYNTH, 8 * BEAT)
                },
                &[]
            )
            .0
        );
    }

    #[test]
    fn a_key_pressed_again_while_its_note_is_open_closes_it_first() {
        let mut song = song();
        let mut rec = NoteRecord::default();
        frame(&mut rec, &mut song, &now(SYNTH, 0), &[on(SYNTH, 60, 1.0)]);
        frame(
            &mut rec,
            &mut song,
            &now(SYNTH, BEAT),
            &[on(SYNTH, 60, 1.0)],
        );
        frame(
            &mut rec,
            &mut song,
            &now(SYNTH, 3 * BEAT),
            &[off(SYNTH, 60)],
        );
        let lens: Vec<_> = events(&song, SYNTH).iter().map(|e| (e.2, e.5)).collect();
        assert_eq!(lens, [(0, 1.0), (PPQ, 2.0)]);
    }

    #[test]
    fn the_clip_grows_to_each_press_and_release_exactly_and_keeps_that_length() {
        let mut song = song();
        let mut rec = NoteRecord::default();
        let clip = |song: &Song| {
            let c = &song.channels[SYNTH].lanes[0].clips[0];
            (c.visible_len_beats, c.loop_len_beats)
        };
        // A note on the clip's start: no stored length yet, the automatic loop of one bar.
        frame(
            &mut rec,
            &mut song,
            &now(SYNTH, 4 * BEAT),
            &[on(SYNTH, 60, 1.0)],
        );
        assert_eq!(clip(&song), (0.0, 4.0));
        frame(
            &mut rec,
            &mut song,
            &now(SYNTH, 5 * BEAT + 32),
            &[off(SYNTH, 60)],
        );
        assert_eq!(clip(&song), (1.25, 4.0));
        frame(
            &mut rec,
            &mut song,
            &now(SYNTH, 9 * BEAT + 64),
            &[on(SYNTH, 62, 1.0)],
        );
        assert_eq!(
            clip(&song),
            (5.5, 8.0),
            "not rounded; the loop doubles to hold it"
        );
        // An earlier press grows nothing; the length stays after the take.
        frame(
            &mut rec,
            &mut song,
            &Now {
                jumped: true,
                ..now(SYNTH, 0)
            },
            &[],
        );
        assert_eq!(clip(&song), (5.5, 8.0));
    }

    #[test]
    fn the_takes_clip_goes_on_the_current_notes_lane_replacing_an_empty_clip_there() {
        let mut song = song();
        let ch = &mut song.channels[SYNTH];
        let second = ch.add_lane(LaneKind::Notes);
        let old = ch.lanes[second].add_clip(2 * PPQ, 4.0).id;
        let mut rec = NoteRecord::default();
        // Beat 5: start 4, which the empty clip at beat 2 covers. It goes; the new clip is another one.
        let current = Now {
            lane: Some(second),
            ..now(SYNTH, 5 * BEAT)
        };
        frame(&mut rec, &mut song, &current, &[on(SYNTH, 60, 1.0)]);
        let clips = &song.channels[SYNTH].lanes[second].clips;
        assert_eq!(
            clips
                .iter()
                .map(|c| (c.start_tick, c.id != old))
                .collect::<Vec<_>>(),
            [(4 * PPQ, true)]
        );
        assert!(song.channels[SYNTH].lanes[0].clips.is_empty());
    }

    #[test]
    fn a_clip_with_notes_on_the_current_lane_sends_the_search_to_the_first_free_notes_lane_then_a_new_one()
     {
        let mut song = song();
        let ch = &mut song.channels[SYNTH];
        let note = Event::Note {
            pos: 0,
            len_beats: 1.0,
            key: 40,
            velocity: 100,
            pan: Event::DEFAULT_PAN,
            slide: false,
        };
        ch.lanes[0].add_clip(0, 8.0).events.push(note);
        let second = ch.add_lane(LaneKind::Notes);
        ch.lanes[second].add_clip(4 * PPQ, 4.0).events.push(note);
        let third = ch.add_lane(LaneKind::Notes);
        // The current lane (the second) has notes over beat 4; the first lane is taken there too: the third.
        let mut rec = NoteRecord::default();
        let current = Now {
            lane: Some(second),
            ..now(SYNTH, 6 * BEAT)
        };
        frame(
            &mut rec,
            &mut song,
            &current,
            &[on(SYNTH, 60, 1.0), off(SYNTH, 60)],
        );
        assert_eq!(song.channels[SYNTH].lanes[third].clips.len(), 1);
        // Overdub: nothing else changed. The next take finds no free notes lane at beat 4: a new one, appended.
        frame(
            &mut rec,
            &mut song,
            &Now {
                armed: false,
                ..current
            },
            &[],
        );
        let (ended, added) = frame(
            &mut rec,
            &mut song,
            &now(SYNTH, 7 * BEAT),
            &[on(SYNTH, 62, 1.0)],
        );
        assert_eq!((ended, added), (false, Some(1)));
        let lanes = &song.channels[SYNTH].lanes;
        assert_eq!(lanes.len(), 4);
        assert_eq!(lanes[3].clips[0].start_tick, 4 * PPQ);
        let kept: Vec<_> = events(&song, SYNTH)
            .iter()
            .map(|e| (e.0, e.3, e.5))
            .collect();
        assert_eq!(
            kept,
            [(0, 40, 1.0), (1, 40, 1.0), (2, 60, MIN_BEATS), (3, 62, 0.0)]
        );
    }

    #[test]
    fn a_channel_without_lanes_records_notes_and_an_audio_channel_nothing() {
        let mut song = song();
        song.channels[SYNTH].lanes.clear();
        let mut rec = NoteRecord::default();
        frame(&mut rec, &mut song, &now(SYNTH, 0), &[on(SYNTH, 60, 1.0)]);
        assert_eq!(song.channels[SYNTH].lanes.len(), 1);
        assert_eq!(song.channels[SYNTH].lanes[0].kind, LaneKind::Notes);
        song.add_channel(ChannelKind::AudioClip);
        let mut rec = NoteRecord::default();
        let (_, added) = frame(&mut rec, &mut song, &now(3, 0), &[on(3, 60, 1.0)]);
        assert_eq!(added, None);
        assert_eq!(song.channels[3].lanes.len(), 1);
    }

    #[test]
    fn nothing_records_unless_armed_rolling_past_the_count_in_on_a_channel() {
        for quiet in [
            Now {
                armed: false,
                ..now(SYNTH, BEAT)
            },
            Now {
                playing: false,
                ..now(SYNTH, BEAT)
            },
            Now {
                counting_in: true,
                ..now(SYNTH, BEAT)
            },
            now(0, BEAT),
        ] {
            let mut song = song();
            let mut rec = NoteRecord::default();
            assert_eq!(
                frame(&mut rec, &mut song, &quiet, &[on(quiet.channel, 60, 1.0)]),
                (false, None)
            );
            assert!(rec.takes.is_empty());
            // A key still held when the take starts records from its start.
            frame(&mut rec, &mut song, &now(SYNTH, 2 * BEAT), &[]);
            frame(
                &mut rec,
                &mut song,
                &now(SYNTH, 3 * BEAT),
                &[off(SYNTH, 60)],
            );
            let held = if quiet.channel == SYNTH {
                vec![(0, 0, 2 * PPQ, 60, 255, 1.0)]
            } else {
                vec![]
            };
            assert_eq!(events(&song, SYNTH), held);
        }
    }

    #[test]
    fn a_loop_wrap_closes_held_notes_where_the_take_last_was_and_records_them_again_in_the_next_take()
     {
        let mut song = song();
        let mut rec = NoteRecord::default();
        frame(
            &mut rec,
            &mut song,
            &now(SYNTH, BEAT),
            &[on(SYNTH, 60, 1.0)],
        );
        frame(&mut rec, &mut song, &now(SYNTH, 3 * BEAT + 100), &[]);
        assert_eq!(
            frame(&mut rec, &mut song, &now(SYNTH, 10), &[]),
            (true, Some(1))
        );
        frame(&mut rec, &mut song, &now(SYNTH, BEAT), &[off(SYNTH, 60)]);
        let notes: Vec<_> = events(&song, SYNTH)
            .iter()
            .map(|e| (e.0, e.2, e.5))
            .collect();
        assert_eq!(
            notes,
            [
                (0, PPQ, (2 * PPQ + 100) as f64 / 128.0),
                (1, 10, 118.0 / 128.0)
            ]
        );
    }

    #[test]
    fn stop_and_seeks_leave_held_notes_at_length_0_and_a_seek_still_armed_starts_a_new_take() {
        for stop in [false, true] {
            let mut song = song();
            let mut rec = NoteRecord::default();
            frame(
                &mut rec,
                &mut song,
                &now(SYNTH, BEAT),
                &[on(SYNTH, 60, 1.0)],
            );
            let jump = if stop {
                Now {
                    playing: false,
                    armed: false,
                    ..now(SYNTH, 2 * BEAT)
                }
            } else {
                Now {
                    jumped: true,
                    ..now(SYNTH, 8 * BEAT)
                }
            };
            assert!(frame(&mut rec, &mut song, &jump, &[]).0);
            frame(
                &mut rec,
                &mut song,
                &Now {
                    jumped: false,
                    ..jump
                },
                &[off(SYNTH, 60)],
            );
            let notes: Vec<_> = events(&song, SYNTH)
                .iter()
                .map(|e| (e.1, e.2, e.5))
                .collect();
            if stop {
                assert_eq!(notes, [(0, PPQ, 0.0)], "the release records nothing");
            } else {
                // The engine-level seek kept REC armed: the key still held is the new take's note, at beat 8.
                assert_eq!(notes, [(0, PPQ, 0.0), (8 * PPQ, 0, MIN_BEATS)]);
            }
        }
    }

    #[test]
    fn disarming_and_a_channel_change_close_held_notes_now() {
        let mut song = song();
        let mut rec = NoteRecord::default();
        frame(
            &mut rec,
            &mut song,
            &now(SYNTH, BEAT),
            &[on(SYNTH, 60, 1.0)],
        );
        assert!(
            frame(
                &mut rec,
                &mut song,
                &Now {
                    armed: false,
                    ..now(SYNTH, 2 * BEAT)
                },
                &[]
            )
            .0
        );
        // Armed again: a new take, which holds the key from its start. Then the drums become current.
        frame(&mut rec, &mut song, &now(SYNTH, 3 * BEAT), &[]);
        assert!(frame(&mut rec, &mut song, &now(DRUMS, 4 * BEAT), &[]).0);
        frame(
            &mut rec,
            &mut song,
            &now(DRUMS, 5 * BEAT),
            &[off(SYNTH, 60)],
        );
        let notes: Vec<_> = events(&song, SYNTH)
            .iter()
            .map(|e| (e.0, e.2, e.5))
            .collect();
        assert_eq!(notes, [(0, PPQ, 1.0), (1, 3 * PPQ, 1.0)]);
    }

    #[test]
    fn a_drum_key_records_a_hit_on_each_slot_it_plays_on_a_flag_1_lane_from_the_takes_start() {
        let mut song = song();
        let kit = song.channels[DRUMS].drum_kit.as_mut().unwrap();
        // Slot 3 also plays 48.
        kit.slots[3].file.midi_key = 48;
        let mut rec = NoteRecord::default();
        // The take starts at beat 3, its first hit at beat 6: the clip at floor(3/4)·4 = 0.
        frame(&mut rec, &mut song, &now(DRUMS, 3 * BEAT), &[]);
        let (_, added) = frame(
            &mut rec,
            &mut song,
            &now(DRUMS, 6 * BEAT),
            &[on(DRUMS, 48, 1.0)],
        );
        assert_eq!(added, Some(1));
        frame(
            &mut rec,
            &mut song,
            &now(DRUMS, 6 * BEAT + 1),
            &[off(DRUMS, 48), on(DRUMS, 127, 1.0)],
        );
        let lane = &song.channels[DRUMS].lanes[1];
        assert_eq!((lane.kind, lane.grid), (LaneKind::Drums, DrumGrid::Notes));
        let hits = [
            (1, 0, 6 * PPQ, 0, 255, MIN_BEATS),
            (1, 0, 6 * PPQ, 3, 255, MIN_BEATS),
        ];
        assert_eq!(events(&song, DRUMS), hits, "key 127 plays no slot");
        // The next take reuses the first flag-1 lane no clip covers at its start.
        let ch = &mut song.channels[DRUMS];
        let free = ch.add_lane(LaneKind::Drums);
        ch.lanes[free].grid = DrumGrid::Notes;
        frame(
            &mut rec,
            &mut song,
            &now(DRUMS, BEAT),
            &[on(DRUMS, 49, 1.0)],
        );
        assert_eq!(song.channels[DRUMS].lanes[free].clips.len(), 1);
    }

    #[test]
    fn an_automation_first_channel_records_notes_and_an_audio_first_one_nothing() {
        // Only an audio first lane records nothing; the piano shows on an automation-first channel.
        let mut song = song();
        song.add_channel(ChannelKind::EffectTrack);
        let mut rec = NoteRecord::default();
        let (_, added) = frame(&mut rec, &mut song, &now(3, 0), &[on(3, 60, 1.0)]);
        assert_eq!(added, Some(1));
        let kinds: Vec<_> = song.channels[3].lanes.iter().map(|l| l.kind).collect();
        assert_eq!(kinds, [LaneKind::Notes, LaneKind::Automation]);
        assert_eq!(events(&song, 3).len(), 1);
    }

    #[test]
    fn a_key_on_another_channel_during_a_take_leaves_the_take_alone() {
        // A key still sounding on another channel (a typed or MIDI key ends where it began).
        let mut song = song();
        let mut rec = NoteRecord::default();
        frame(&mut rec, &mut song, &now(SYNTH, 0), &[on(SYNTH, 60, 1.0)]);
        let before = song.clone();
        let (_, changed) = frame(
            &mut rec,
            &mut song,
            &now(SYNTH, BEAT),
            &[on(DRUMS, 48, 1.0), off(DRUMS, 48), off(1 + DRUMS, 60)],
        );
        assert_eq!(changed, None);
        assert_eq!(song, before);
    }

    #[test]
    fn held_keys_follow_their_channel_and_a_new_song_forgets_them() {
        // No phantom note in the next take.
        let mut song = song();
        let mut rec = NoteRecord::default();
        let stopped = Now {
            playing: false,
            ..now(SYNTH, 0)
        };
        frame(&mut rec, &mut song, &stopped, &[on(SYNTH, 48, 1.0)]);
        // The synth moves to 2, the drums to 1: the key held on the synth goes with it.
        let ids = [
            song.channels[0].id,
            song.channels[1].id,
            song.channels[2].id,
        ];
        song.channels.swap(1, 2);
        // A key queued for the next frame goes with its channel too.
        rec.keys.push(off(SYNTH, 50));
        rec.rekey(&ids, &|c| [0, 2, 1].get(c).copied(), &|c, l| {
            Some(([0, 2, 1][c], l))
        });
        assert_eq!(rec.keys, [off(2, 50)]);
        frame(&mut rec, &mut song, &now(1, BEAT), &[]);
        assert_eq!(events(&song, 1), [], "nothing on the drums");
        frame(
            &mut rec,
            &mut song,
            &Now {
                armed: false,
                ..now(2, BEAT)
            },
            &[],
        );
        frame(&mut rec, &mut song, &now(2, 2 * BEAT), &[]);
        assert_eq!(
            events(&song, 2).len(),
            1,
            "the synth's key, from its take's start"
        );
        // A new song: nothing is held any more.
        rec.forget();
        let mut song = self::song();
        frame(&mut rec, &mut song, &now(SYNTH, BEAT), &[]);
        assert_eq!(events(&song, SYNTH), []);
    }

    #[test]
    fn a_take_clip_goes_with_its_track_unless_the_take_made_the_track_itself() {
        let mut song = song();
        let mut rec = NoteRecord::default();
        let ch = &mut song.channels[SYNTH];
        ch.lanes[0].add_clip(0, 4.0).events.push(Event::Note {
            pos: 0,
            len_beats: 1.0,
            key: 40,
            velocity: 100,
            pan: Event::DEFAULT_PAN,
            slide: false,
        });
        // The take makes lane 1 and its clip: the app's re-key of that change moves nothing of its own.
        rec.keys.push(on(SYNTH, 60, 1.0));
        let added = rec.record(&mut song, &now(SYNTH, BEAT));
        assert_eq!(added.map(|a| a.len()), Some(1));
        let ids: Vec<_> = song.channels.iter().map(|c| c.id).collect();
        let same = |c: usize| Some(c);
        rec.rekey(&ids, &same, &|c, l| {
            Some((c, if c == SYNTH && l >= 1 { l + 1 } else { l }))
        });
        assert_eq!(rec.clips(&song), [(SYNTH, 1, 1)]);
        // A track op then puts a lane before it: the take's clip goes with its track.
        song.channels[SYNTH]
            .lanes
            .insert(0, Lane::new(LaneKind::Notes));
        rec.rekey(&ids, &same, &|c, l| {
            Some((c, if c == SYNTH { l + 1 } else { l }))
        });
        assert_eq!(rec.clips(&song), [(SYNTH, 2, 1)]);
        frame(
            &mut rec,
            &mut song,
            &now(SYNTH, 2 * BEAT),
            &[off(SYNTH, 60)],
        );
        assert_eq!(song.channels[SYNTH].lanes[2].clips[0].events.len(), 1);
        let Event::Note { len_beats, .. } = song.channels[SYNTH].lanes[2].clips[0].events[0] else {
            panic!()
        };
        assert_eq!(len_beats, 1.0);
    }

    #[test]
    fn without_take_leaves_only_the_takes_clip_out() {
        let mut song = song();
        let mut rec = NoteRecord::default();
        assert!(rec.without_take(&song).is_none(), "no take");
        frame(
            &mut rec,
            &mut song,
            &now(SYNTH, BEAT),
            &[on(SYNTH, 60, 1.0)],
        );
        let played = rec.without_take(&song).unwrap();
        assert!(played.channels[SYNTH].lanes[0].clips.is_empty());
        let mut expected = song.clone();
        expected.channels[SYNTH].lanes[0].clips.clear();
        assert_eq!(played, expected);
    }
}
