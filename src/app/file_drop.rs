//! Audio files dropped on the app ([`file_drag`]): from outside it, or a browser's file dragged onto the playlist.
//!
//! A file from outside the app is kept in the user's `My Samples` first, as `Import` keeps one, and the clip refers to
//! the copy (`PTH1`); a browser's file is in the library already, and so are the files SONGS' `Send to playlist` sends
//! (#106), which make clips as files dropped off the playlist do. The files then decode on the worker, behind the
//! loading screen [`IMPORTING`] when that takes longer than a frame, and each becomes an audio clip:
//! - dropped on an audio track: there, at the snapped beat, the files one after another;
//! - dropped between channels: a new Audio channel for each file, inserted there in order, its clip at the snapped beat
//!   (beat 0 off the grid);
//! - dropped elsewhere, or where the drop's position is unknown: new Audio channels at the end, their clips at the
//!   playhead.
//!
//! One drop is one undo step, and an ordinary song edit in a live session. The last new channel becomes the current one.
//! A file that does not decode makes nothing, and the drop ends on a message naming it.
//!
//! #117: Tesla's `Send to arranger as audio clip` is sent as SONGS' `Send to playlist` sends: its file kept in the user's
//! `My Recordings` first, as Tesla's `Save` keeps one there.
//!
//! #167 (a Honeydew change): the notes of Tesla's `Convert to score and dump to piano roll` become a new clip at the
//! playhead ([`HoneydewApp::place_score`]).

use std::path::PathBuf;
use std::sync::mpsc::{Receiver, TryRecvError};

use eframe::egui;

use super::{HoneydewApp, SongEdit};
use crate::metrics::Metrics;
use crate::project_files::IMPORT_FAILED;
use crate::song::{Channel, ChannelKind, Clip, Event, Lane, LaneKind, PPQ, SynthKind};
use crate::ui::UiCtx;
use crate::ui::clips::{ClipRef, end_tick};
use crate::ui::file_drag::{self, Dropped, Place};
use crate::ui::views::sample_picker::{Decoded, drop_clip, drop_decode};
use crate::ui::views::{Nav, Overlay, preset_picker};

/// The loading screen's title.
pub(super) const IMPORTING: &str = "Importing...";
/// The failure message's tag.
const FAILED_TAG: &str = "file_drop";

/// A drop decoding on the worker.
pub(super) struct Importing {
    done: Receiver<Vec<(PathBuf, Decoded)>>,
    place: Option<Place>,
    /// The playhead at the drop: where clips off the playlist go.
    tick: u64,
    /// The frame it started in.
    frame: u64,
    /// The loading screen is up.
    shown: bool,
}

impl HoneydewApp {
    /// After the screens, each frame: the drag's curtain, drop target and name ([`file_drag::end_frame`]); a drop
    /// starts its import, and an import whose decode landed makes its clips. A drop while one imports is dropped.
    pub(super) fn file_drop(&mut self, ctx: &egui::Context, m: &Metrics, tick: u64) {
        // #117: what Tesla sent to the arranger this frame (the module docs).
        for sent in os_plugkit::embed::take_sent() {
            let bytes = std::fs::read(&sent).ok();
            let kept = bytes.and_then(|b| self.files.keep_saved(&sent, b));
            let _ = std::fs::remove_file(&sent);
            file_drag::send(ctx, kept.into_iter().collect());
        }
        for notes in os_plugkit::embed::take_scored() {
            self.place_score(&notes, tick);
        }
        let cx = UiCtx {
            theme: &self.theme,
            m,
        };
        if let Some(dropped) = file_drag::end_frame(ctx, &cx)
            && self.importing.is_none()
        {
            let (paths, place) = match dropped {
                // ponytail: the copies are made on the UI thread, before the loading screen can show; move them to the
                // worker if dropping very large files stalls the window.
                Dropped::External(files, place) => {
                    let kept = files.iter().filter_map(|f| self.files.keep_dropped(f));
                    (kept.collect(), place)
                }
                Dropped::Library(path, place) => (vec![path], Some(place)),
                Dropped::Sent(paths) => (paths, None),
            };
            if !paths.is_empty() {
                self.importing = Some(Importing {
                    done: drop_decode(&self.song, paths, &self.files),
                    place,
                    tick,
                    frame: ctx.cumulative_frame_nr(),
                    shown: false,
                });
            }
            ctx.request_repaint();
        }
        let Some(importing) = &mut self.importing else {
            return;
        };
        let picks = match importing.done.try_recv() {
            Err(TryRecvError::Empty) => {
                // Longer than a frame: the loading screen.
                if importing.frame < ctx.cumulative_frame_nr() && !importing.shown {
                    importing.shown = true;
                    let title = IMPORTING.into();
                    self.navigate(Nav::Open(Overlay::Busy { title }));
                }
                ctx.request_repaint();
                return;
            }
            done => done.unwrap_or_default(),
        };
        let Importing {
            place, tick, shown, ..
        } = self.importing.take().expect("importing");
        if shown {
            let screen = |o: &Overlay| matches!(o, Overlay::Busy { title } if title == IMPORTING);
            self.nav.overlays.retain(|o| !screen(o));
        }
        self.place_clips(picks, place, tick);
        ctx.request_repaint();
    }

    /// #167 (a Honeydew change): the notes of Tesla's `Convert to score` as a new clip at song tick `tick` (the
    /// playhead) on the selected channel when its main track is a notes track, else on the first channel after it with
    /// one, else on a new MiniSynth channel at the end. It never lies over a clip: it takes that channel's first shown
    /// notes track with room under its span, else a new track of the channel. Its loop is fitted to the notes, its
    /// track current, one undo step. Each note's start and end are in beats from the clip's start, its velocity MIDI's
    /// (as a MIDI file's).
    pub(super) fn place_score(&mut self, notes: &[os_plugkit::embed::NoteOut], tick: u64) {
        let song = &mut self.song;
        let has_notes = |c: &Channel| c.lanes.first().is_some_and(|l| l.kind == LaneKind::Notes);
        let found =
            (self.selected_channel..song.channels.len()).find(|&c| has_notes(&song.channels[c]));
        let minisynth = ChannelKind::Synth(SynthKind::MiniSynth);
        let Some(channel) = found.or_else(|| song.insert_channel(song.channels.len(), minisynth))
        else {
            return self.navigate(Nav::Open(preset_picker::channel_limit_message()));
        };
        let bar = song.time_sig.bar_beats();
        let start = u32::try_from(tick).unwrap_or(u32::MAX);
        // The clip first, for its span.
        let mut clip = Lane::new(LaneKind::Notes)
            .add_clip(start, f64::from(bar))
            .clone();
        clip.events = (notes.iter())
            .map(|n| Event::Note {
                pos: (n.start * f64::from(PPQ)).round() as u32,
                len_beats: (n.end - n.start).max(0.0),
                key: n.key,
                velocity: (f64::from(n.velocity) * 0.007874 * 255.0) as u8,
                pan: Event::DEFAULT_PAN,
                slide: false,
            })
            .collect();
        clip.events.sort_by_key(Event::pos);
        clip.fit_loop(bar);
        let span = f64::from(start)..end_tick(&clip);
        let room = |l: &Lane| {
            let clear = |c: &Clip| end_tick(c) <= span.start || f64::from(c.start_tick) >= span.end;
            l.kind == LaneKind::Notes && !l.hidden && l.clips.iter().all(clear)
        };
        let ch = &mut song.channels[channel];
        let lane = match ch.lanes.iter().position(room) {
            Some(lane) => lane,
            None => ch.add_lane(LaneKind::Notes),
        };
        let new = ch.lanes[lane].add_clip(start, clip.loop_len_beats);
        new.events = clip.events;
        let at = ClipRef {
            channel,
            lane,
            id: new.id,
        };
        (self.main_screen.clips).make_current(song, &mut self.selected_channel, at);
        self.song_edited(SongEdit::Final, tick);
    }

    /// Makes the clips of a drop's decoded files `picks`, in drop order, at `place` (see the module docs): one final
    /// edit. A track no longer an audio track (an undo since the drop) drops as off the playlist does.
    fn place_clips(&mut self, picks: Vec<(PathBuf, Decoded)>, place: Option<Place>, tick: u64) {
        let song = &mut self.song;
        let audio = |c: usize, l: usize| {
            let lane = song.channels.get(c).and_then(|ch| ch.lanes.get(l));
            lane.is_some_and(|l| l.kind == LaneKind::Audio)
        };
        let (mut at, track, mut start) = match place {
            Some(Place::Track {
                channel,
                lane,
                tick,
            }) if audio(channel, lane) => (None, Some((channel, lane)), tick),
            Some(Place::Insert { at, tick }) => (Some(at.min(song.channels.len())), None, tick),
            _ => (
                Some(song.channels.len()),
                None,
                u32::try_from(tick).unwrap_or(u32::MAX),
            ),
        };
        let beats = (song.tempo_bpm, song.time_sig.bar_beats());
        let (mut made, mut failed, mut full) = (false, Vec::new(), false);
        for (path, decoded) in picks {
            if decoded.wave.is_none() {
                let why = decoded.error.unwrap_or_default();
                failed.push(format!("{}: {why}", path.display()));
                continue;
            }
            let lane = match (track, at) {
                (Some((channel, lane)), _) => &mut song.channels[channel].lanes[lane],
                (None, Some(index)) => {
                    let Some(index) = song.insert_channel(index, ChannelKind::AudioClip) else {
                        full = true;
                        break;
                    };
                    (at, self.selected_channel) = (Some(index + 1), index);
                    &mut song.channels[index].lanes[0]
                }
                (None, None) => unreachable!("a track or a channel index"),
            };
            let samples = &mut self.audio.samples;
            if let Some(end) = drop_clip(lane, start, &path, decoded, &self.files, samples, beats) {
                made = true;
                // On a track the next file follows; each new channel's clip starts at the drop.
                if track.is_some() {
                    start = end;
                }
            }
        }
        if made {
            self.song_edited(SongEdit::Final, tick);
        }
        if full {
            self.navigate(Nav::Open(preset_picker::channel_limit_message()));
        } else if !failed.is_empty() {
            let lines = std::iter::once(IMPORT_FAILED.to_owned()).chain(failed);
            self.navigate(Nav::Open(Overlay::Message {
                tag: FAILED_TAG,
                lines: lines.collect(),
                confirm: None,
                extra: None,
            }));
        }
    }
}
