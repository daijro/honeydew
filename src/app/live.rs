//! Live editing in a Playground session. The shell's adapter ([`web`]) drives it; nothing here runs until the shell
//! connects. A remote change merges into the live song at the end of a quiet frame, keeping this person's views and
//! unsent work, and is no step of the native undo history. While connected, Undo and Redo go to the session, which
//! reverts only this person's operations.

use super::*;
use crate::collaboration::{Conflict, checkpoint, keep_local, merge, shared};
use crate::flm::chunk::{ContentPath, ContentRoot};
use crate::params::CHANNEL_PARAMS;
use crate::project_files::MAX_FILE_BYTES;
use crate::sample_cache::SampleKey;
use crate::song::TimeSig;
use crate::song::points::{self, Point, PointKey};
use crate::song::{Event, PPQ};
use crate::ui::automation::geom::Frame;
use crate::ui::clips::{ClipRef, TimeAxis, clip_rect, find};
use crate::ui::drum_grid::{self, edit::Place};
use crate::ui::main_screen::track_rows;
use crate::ui::piano_roll::notes::{Geom, TOP_KEY, note};
use crate::ui::step_seq::{self, Layout};
use egui::{Color32, Pos2, Rect, Shape, Stroke, StrokeKind, pos2, vec2};
use serde::{Deserialize, Serialize};
use std::collections::{BTreeSet, VecDeque};
use std::path::{Path, PathBuf};

/// A remote change waiting for a quiet frame: `edited` relative to `base`, or with no base the whole shared song.
struct Remote {
    base: Option<Vec<u8>>,
    edited: Vec<u8>,
    done: Done,
}

/// Settles a remote change: a whole adopted song gives back the shared song it replaced, for the shell's draft.
pub(crate) type Done = Box<dyn FnOnce(Result<Option<Vec<u8>>, Conflict>)>;

#[derive(Default)]
struct Live {
    connected: bool,
    /// The last frame was not quiet: a checkpoint now would catch a gesture half done.
    busy: bool,
    /// The song as last compared, and its shared projection as last reported.
    seen: Option<Song>,
    shared: Option<Song>,
    remote: VecDeque<Remote>,
    /// Shared changes reported so far.
    changes: u64,
    /// A sample arrived: the next quiet frame rebuilds the engine, which decodes it.
    reload: bool,
    /// This person's pointer and notes (selected, dragged) as the editors drawn this frame leave them, and as the last
    /// frame left them: what [`HoneydewApp::live_presence`] reads.
    drawn: Mine,
    mine: Mine,
    /// The others in the session, as the shell last listed them.
    others: Vec<Peer>,
    #[cfg(test)]
    undos: Vec<bool>,
    #[cfg(target_arch = "wasm32")]
    web: Option<web::Hooks>,
    /// For a repaint when the shell queues a change between frames.
    #[cfg(target_arch = "wasm32")]
    ctx: Option<egui::Context>,
}

thread_local! {
    static LIVE: std::cell::RefCell<Live> = std::cell::RefCell::default();
}

pub(crate) fn connect() {
    LIVE.with_borrow_mut(|live| {
        (live.connected, live.seen, live.shared) = (true, None, None);
    });
}

/// Ends the session's hold on Undo; changes still waiting fail.
pub(crate) fn disconnect() {
    let waiting = LIVE.with_borrow_mut(|live| {
        live.connected = false;
        live.others.clear();
        #[cfg(target_arch = "wasm32")]
        {
            live.web = None;
        }
        std::mem::take(&mut live.remote)
    });
    for remote in waiting {
        (remote.done)(Err(Conflict("Live editing disconnected".into())));
    }
}

pub(crate) fn queue(base: Option<Vec<u8>>, edited: Vec<u8>, done: Done) {
    let done = LIVE.with_borrow_mut(|live| {
        if !live.connected {
            return Some(done);
        }
        live.remote.push_back(Remote { base, edited, done });
        #[cfg(target_arch = "wasm32")]
        if let Some(ctx) = &live.ctx {
            ctx.request_repaint();
        }
        None
    });
    if let Some(done) = done {
        done(Err(Conflict("Live editing is not connected".into())));
    }
}

/// Passes Undo or Redo to the session while connected: whether it did.
pub(crate) fn undo(redo: bool) -> bool {
    let connected = LIVE.with_borrow_mut(|live| {
        #[cfg(test)]
        if live.connected {
            live.undos.push(redo);
        }
        live.connected
    });
    #[cfg(target_arch = "wasm32")]
    if connected {
        web::undo(redo);
    }
    connected
}

#[cfg(test)]
pub(crate) fn changes() -> u64 {
    LIVE.with_borrow(|live| live.changes)
}

#[cfg(test)]
pub(crate) fn undos() -> Vec<bool> {
    LIVE.with_borrow(|live| live.undos.clone())
}

/// The user files `song` plays (samples, programs, SoundFonts, shapes), as the shell names them: the `@L@` path's
/// relative part. Built-in (`@R@Built-in`), content-folder (`@R@`) and temporary files are no one's to share.
pub(crate) fn user_media(song: &Song) -> Vec<String> {
    let mut ids: Vec<String> = SampleCache::referenced(song)
        .into_iter()
        .filter_map(|key| match key {
            SampleKey::File(ContentPath::Shorthand {
                root: ContentRoot::User,
                relative,
            }) => Some(relative),
            _ => None,
        })
        .collect();
    ids.sort();
    ids.dedup();
    ids
}

/// [`user_media`] of a shared checkpoint.
pub(crate) fn assets_of(bytes: &[u8]) -> Result<Vec<String>, Conflict> {
    Ok(user_media(&checkpoint::decode(bytes)?))
}

fn media_key(id: &str) -> ContentPath {
    ContentPath::Shorthand {
        root: ContentRoot::User,
        relative: id.into(),
    }
}

impl HoneydewApp {
    fn media_file(&self, id: &str) -> Option<PathBuf> {
        self.files.resolve(&media_key(id))
    }

    /// The live song's user media: (id, file name, whether this tab has the file).
    pub(crate) fn live_assets(&self) -> Vec<(String, String, bool)> {
        (user_media(&self.song).into_iter())
            .map(|id| {
                let have = (self.media_file(&id)).is_some_and(|p| self.files.user_file_exists(&p));
                let name = Path::new(&id)
                    .file_name()
                    .map(|n| n.to_string_lossy().into_owned());
                (id, name.unwrap_or_default(), have)
            })
            .collect()
    }

    pub(crate) fn live_read_asset(&self, id: &str) -> Option<Vec<u8>> {
        self.files.read_user_file(&self.media_file(id)?).ok()
    }

    /// Keeps a sample that arrived from the session as Import keeps one (this tab's user copies), never over a file
    /// already here: two people's different files under one path each keep their own. Its missing mark goes and the
    /// next quiet frame's engine decodes it. No song change, no undo step.
    pub(crate) fn live_write_asset(&mut self, id: &str, bytes: Vec<u8>) {
        let Some(path) = self.media_file(id) else {
            return;
        };
        if bytes.len() as u64 > MAX_FILE_BYTES
            || self.files.user_file_exists(&path)
            || self.files.write_user_file(&path, bytes).is_err()
        {
            return;
        }
        self.audio.samples.forget(&SampleKey::File(media_key(id)));
        LIVE.with_borrow_mut(|live| {
            live.reload = true;
            #[cfg(target_arch = "wasm32")]
            if let Some(ctx) = &live.ctx {
                ctx.request_repaint();
            }
        });
    }

    /// The canonical shared song, unless the last frame was busy: then the next quiet frame reports it again.
    pub(crate) fn live_checkpoint(&self) -> Result<Vec<u8>, Conflict> {
        let busy = LIVE.with_borrow_mut(|live| {
            if live.busy {
                (live.seen, live.shared) = (None, None);
            }
            live.busy
        });
        if busy {
            return Err(Conflict("busy".into()));
        }
        checkpoint::encode(&self.song)
    }

    /// Nothing under way that a remote change would cut short or a proposal would catch half done: no pointer down
    /// (a drag, a held control), no param or sample-row gesture, no channel params outside the history, no recording, no
    /// question about what a playlist delete emptied (its yes joins the delete's change).
    fn live_quiet(&self, ctx: &egui::Context) -> bool {
        !ctx.input(|i| i.pointer.any_down())
            && !self.param_gesture
            && !self.row_drag
            && !self.params_moved
            && !self.recording()
            && !matches!(self.asked, Some(tracks::Asked::Emptied(_)))
    }

    /// The end of a frame: the remote changes waiting, then a report if the shared song changed, once quiet.
    pub(super) fn live_frame(&mut self, ctx: &egui::Context, tick: u64) {
        let quiet = self.live_quiet(ctx);
        let waiting = LIVE.with_borrow_mut(|live| {
            #[cfg(target_arch = "wasm32")]
            live.ctx.get_or_insert_with(|| ctx.clone());
            live.busy = !quiet;
            live.mine = std::mem::take(&mut live.drawn);
            if live.connected && quiet {
                std::mem::take(&mut live.remote)
            } else {
                VecDeque::new()
            }
        });
        if quiet && LIVE.with_borrow_mut(|live| std::mem::take(&mut live.reload)) {
            self.rebuild_as(tick, Handover::Undo);
        }
        for remote in waiting {
            let result = self.live_apply(tick, remote.base.as_deref(), &remote.edited);
            (remote.done)(result);
            ctx.request_repaint();
        }
        let changed = LIVE.with_borrow_mut(|live| {
            if !live.connected || !quiet || live.seen.as_ref() == Some(&self.song) {
                return false;
            }
            live.seen = Some(self.song.clone());
            let now = shared(&self.song);
            if live.shared.as_ref() == Some(&now) {
                return false;
            }
            live.shared = Some(now);
            live.changes += 1;
            true
        });
        #[cfg(target_arch = "wasm32")]
        if changed {
            web::changed();
        }
        #[cfg(not(target_arch = "wasm32"))]
        let _ = changed;
    }

    /// Merges a remote change into the song as an undo step's restore does ([`Self::step`]), with no step of its own.
    // ponytail: mirrors `step`'s install sequence; share one helper upstream if either changes.
    fn live_apply(
        &mut self,
        tick: u64,
        base: Option<&[u8]>,
        edited: &[u8],
    ) -> Result<Option<Vec<u8>>, Conflict> {
        let edited = checkpoint::decode(edited)?;
        let replaced = base
            .is_none()
            .then(|| checkpoint::encode(&self.song).ok())
            .flatten();
        let mut song = match base {
            // The live song as its checkpoint reads: what the format settles (a new channel's rack place) is no edit
            // of this person's for the merge to keep.
            Some(base) => {
                let settled = checkpoint::decode(&checkpoint::encode(&self.song)?)?;
                merge(&checkpoint::decode(base)?, &settled, &edited)?
            }
            None => edited,
        };
        keep_local(&mut song, &self.song);
        let shifts = tracks::lane_shifts(&self.song, &song);
        tracks::carry_current_clips(&self.song, &mut song, &shifts);
        let before = std::mem::replace(&mut self.song, song);
        self.history.amend(&before, &self.song);
        self.files.song_changed();
        self.rekey(&shifts);
        // The editors' selections follow their notes, hits and points into the changed clip ([`follow`]).
        for roll in self.slots.piano_roll.rolls.values_mut() {
            roll.follow(&self.song);
        }
        self.slots.drum_editor.grid.follow(&self.song);
        self.slots.automation_editor.editor.follow(&self.song);
        self.slots.audio_editor.editor.follow(&self.song);
        let ids = &self.channel_ids;
        let gone = |id: i32| before.channels.iter().any(|c| c.id == id) && !ids.contains(&id);
        self.main_screen.forget_keys_views(gone);
        self.selected_channel = self.selected_channel.min(self.song.channels.len() - 1);
        self.rebuild_as(tick, Handover::Undo);
        Ok(replaced)
    }
}

/// An editor's selection, a flag for each of the items it last saw (`old`), carried over a remote change to the items
/// now there (`new`): each selected item to an equal one at its place (`at`, by which both are sorted), copy for copy.
/// One the change moved, changed or removed drops out.
pub(crate) fn follow<T: PartialEq>(
    old: &[T],
    sel: &[bool],
    new: &[T],
    at: impl Fn(&T) -> u32,
) -> Vec<bool> {
    let mut out = vec![false; new.len()];
    let mut from = 0;
    for (x, _) in old.iter().zip(sel).filter(|(_, s)| **s) {
        while from < new.len() && at(&new[from]) < at(x) {
            from += 1;
        }
        let same = (from..new.len())
            .take_while(|&i| at(&new[i]) == at(x))
            .find(|&i| !out[i] && new[i] == *x);
        if let Some(i) = same {
            out[i] = true;
        }
    }
    out
}

/// Where a person is, for the others in the session: the view shown (`playlist`, `piano`, `drums`, `automation`,
/// `audio`, `mixer`); the clip a clip editor edits; the pointer while it is over the playlist or an editor; what they
/// selected (the playlist's clips unless the mixer covers the playlist, an editor's notes, hits or points); and a
/// gesture under way, where it has things so far (the song changes only when it ends: meanwhile `clips`, `notes` and
/// `points` name them where they were). All in the song's terms (channel ids, lanes, clip ids, ticks, keys, values),
/// so each person draws it through their own zoom and scroll.
#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize)]
pub(crate) struct Presence {
    view: String,
    /// (channel id, lane, clip id).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    clip: Option<(i32, usize, u32)>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pointer: Option<Pointer>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    clips: Vec<(i32, usize, u32)>,
    /// (content tick, key) of each note; in the drum editor, of each hit, its pad for a key.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    notes: Vec<(u32, u8)>,
    /// (channel id, lane, start tick, length in ticks) of each dragged clip.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    drag_clips: Vec<(i32, usize, u32, f64)>,
    /// (content tick, key or pad, length in ticks) of each dragged note or hit.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    drag_notes: Vec<(u32, u8, f64)>,
    /// (content tick, value) of each selected automation or envelope point, and of each dragged one where the drag has
    /// it so far.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    points: Vec<(u32, u16)>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    drag_points: Vec<(u32, u16)>,
    /// The audio editor's envelope (0 volume, 1 pan, 2 speed), its range (start and end, content ticks), and a stretch
    /// under way: the content tick it has the sample end at so far.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    env: Option<u8>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    range: Option<(f64, f64)>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    stretch: Option<f64>,
    /// A mixer fader or knob held: its channel id, param (0 level, 1 pan) and value so far.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    param: Option<(i32, u8, f32)>,
}

/// The pointer: a song tick on a playlist track (channel id, lane), or a content tick on a key row of the piano roll or
/// a pad row of the drum editor, and `y` how far down that row (0 its top, 1 its bottom; its middle when missing). In
/// the automation and audio editors, a content tick and no row: `y` is how far down the values' span, a little past
/// either end over its margins. In the mixer, `track` is a strip's channel (lane 0), `tick` how far across it (0 its
/// left, 1 its right) and `y` how far down. Ticks may be fractional.
#[derive(Clone, Copy, Debug, PartialEq, Serialize, Deserialize)]
pub(crate) struct Pointer {
    tick: f64,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    track: Option<(i32, usize)>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    key: Option<u8>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    y: Option<f64>,
}

#[derive(Default)]
struct Mine {
    pointer: Option<Pointer>,
    notes: Vec<(u32, u8)>,
    drag_notes: Vec<(u32, u8, f64)>,
    points: Vec<(u32, u16)>,
    drag_points: Vec<(u32, u16)>,
    env: Option<u8>,
    range: Option<(f64, f64)>,
    stretch: Option<f64>,
    param: Option<(i32, u8, f32)>,
}

/// Another person, as [`show_presence`] keeps them, with their marks as the playlist and the editor shown last drew
/// them.
struct Peer {
    id: String,
    name: String,
    color: Color32,
    state: Presence,
    on_playlist: Shown,
    on_editor: Shown,
}

/// A person's moving marks as a view last drew them, eased toward their latest state: the pointer (tick, row), the
/// ghosts of a drag (tick, row, length in ticks) and spans of ticks (the audio editor's range and stretch). A row is a
/// track's index in the track list or a key's row from the top, plus the fraction down it; or a fraction of a plot's
/// values' span.
#[derive(Default)]
struct Shown {
    pointer: Option<[f64; 2]>,
    ghosts: Vec<[f64; 3]>,
    spans: Vec<[f64; 2]>,
}

/// How long the others' marks take to go most (1 - 1/e) of the way to where they last were, in seconds.
const EASE: f64 = 0.08;

impl Shown {
    /// `dt` seconds on toward `to`; `px`: points a tick and a row. A first sight, another set of ghosts or a jump of
    /// more than `far` points goes at once. Whether a mark is still on its way.
    fn step(&mut self, to: Shown, dt: f32, px: [f64; 2], far: f64) -> bool {
        let k = 1.0 - (-f64::from(dt.clamp(0.0, 0.1)) / EASE).exp();
        let mut moving = false;
        match (&mut self.pointer, to.pointer) {
            (Some(s), Some(t)) => moving |= ease(s, t, k, px, far),
            (s, t) => *s = t,
        }
        moving |= ease_all(&mut self.ghosts, to.ghosts, k, [px[0], px[1], px[0]], far);
        moving | ease_all(&mut self.spans, to.spans, k, [px[0]; 2], far)
    }
}

/// [`ease`] for each of a set of marks; another set goes at once.
fn ease_all<const N: usize>(
    s: &mut Vec<[f64; N]>,
    t: Vec<[f64; N]>,
    k: f64,
    px: [f64; N],
    far: f64,
) -> bool {
    if s.len() != t.len() {
        *s = t;
        return false;
    }
    let mut moving = false;
    for (s, t) in s.iter_mut().zip(t) {
        moving |= ease(s, t, k, px, far);
    }
    moving
}

/// `s` moved `k` of the way to `t`, all of it after a jump of more than `far` points or once under half a point remains
/// (`px`: points a unit of each part). Whether it is still on its way.
fn ease<const N: usize>(s: &mut [f64; N], t: [f64; N], k: f64, px: [f64; N], far: f64) -> bool {
    let off = |s: &[f64; N]| {
        (s.iter().zip(t).zip(px))
            .map(|((s, t), px)| ((t - s) * px).abs())
            .fold(0.0, f64::max)
    };
    if off(s) > far {
        *s = t;
    }
    for (s, t) in s.iter_mut().zip(t) {
        *s += (t - *s) * k;
    }
    let moving = off(s) >= 0.5;
    if !moving {
        *s = t;
    }
    moving
}

/// The most selected clips, or notes, a presence carries; the same for dragged ones.
const MAX_SELECTED: usize = 50;
/// The most characters of a name tag, its ellipsis included.
const MAX_NAME: usize = 24;

/// The shell's list of the others (`[{ id, name, color: "#rrggbb", state }]`), each time it changes; empty at the end.
/// It comes from other people: an entry that does not parse is skipped. A person still in the same view and clip keeps
/// their marks where they were drawn, to ease on from there.
pub(crate) fn show_presence(others: &serde_json::Value) {
    let peer = |v: &serde_json::Value| {
        let hex = v["color"].as_str()?.strip_prefix('#')?;
        let channel = |i: usize| u8::from_str_radix(hex.get(i..i + 2)?, 16).ok();
        let mut state: Presence = serde_json::from_value(v["state"].clone()).ok()?;
        state.clips.truncate(MAX_SELECTED);
        state.notes.truncate(MAX_SELECTED);
        state.drag_clips.truncate(MAX_SELECTED);
        state.drag_notes.truncate(MAX_SELECTED);
        state.points.truncate(MAX_SELECTED);
        state.drag_points.truncate(MAX_SELECTED);
        let text = |key: &str| v[key].as_str().unwrap_or_default().chars();
        let mut name: String = text("name").take(MAX_NAME).collect();
        if text("name").nth(MAX_NAME).is_some() {
            name.pop();
            name.push('…');
        }
        Some(Peer {
            id: text("id").take(64).collect(),
            name,
            color: Color32::from_rgb(channel(0)?, channel(2)?, channel(4)?),
            state,
            on_playlist: Shown::default(),
            on_editor: Shown::default(),
        })
    };
    let mut others: Vec<Peer> = (others.as_array().into_iter().flatten())
        .filter_map(peer)
        .collect();
    LIVE.with_borrow_mut(|live| {
        for peer in &mut others {
            let same = |o: &&mut Peer| {
                o.id == peer.id
                    && o.state.view == peer.state.view
                    && o.state.clip == peer.state.clip
            };
            if let Some(old) = live.others.iter_mut().find(same) {
                peer.on_playlist = std::mem::take(&mut old.on_playlist);
                peer.on_editor = std::mem::take(&mut old.on_editor);
            }
        }
        live.others = others;
        #[cfg(target_arch = "wasm32")]
        if let Some(ctx) = &live.ctx {
            ctx.request_repaint();
        }
    });
}

/// A remote tick or length, unless it is no number or too far off to draw.
fn tick(t: f64) -> Option<f64> {
    (t.abs() <= f64::from(u32::MAX)).then_some(t)
}

/// A remote fraction down a row: its middle when missing or off.
fn down(y: Option<f64>) -> f64 {
    y.filter(|y| (0.0..=1.0).contains(y)).unwrap_or(0.5)
}

/// `x` in hundredths: finer than a point at any zoom, and short in JSON.
fn fine(x: f64) -> f64 {
    (x * 100.0).round() / 100.0
}

/// `d` points down a row `h` tall, as a fraction of it in thousandths.
fn fraction(d: f32, h: f32) -> f64 {
    (f64::from(d / h).clamp(0.0, 1.0) * 1000.0).round() / 1000.0
}

/// The pointer at `at` over playlist `rows` ((channel, lane, row rect) as drawn) on time `axis`.
fn playlist_pointer(
    song: &Song,
    rows: &[(usize, usize, Rect)],
    axis: TimeAxis,
    at: Pos2,
) -> Option<Pointer> {
    let &(channel, lane, rect) = rows.iter().find(|r| r.2.y_range().contains(at.y))?;
    Some(Pointer {
        tick: fine(axis.tick(at.x)),
        track: Some((song.channels.get(channel)?.id, lane)),
        key: None,
        y: Some(fraction(at.y - rect.top(), rect.height())),
    })
}

/// Where `state`'s pointer and drag are among the playlist's `tracks` ((channel, lane) in track-list order), in
/// [`Shown`]'s terms. What does not resolve is left out.
fn playlist_targets(song: &Song, tracks: &[(usize, usize)], state: &Presence) -> Shown {
    let row = |(id, lane): (i32, usize)| {
        let channel = song.channels.iter().position(|c| c.id == id)?;
        Some(tracks.iter().position(|&t| t == (channel, lane))? as f64)
    };
    let pointer = (state.pointer).and_then(|p| Some([tick(p.tick)?, row(p.track?)? + down(p.y)]));
    let ghosts = (state.drag_clips.iter())
        .filter_map(|&(id, lane, start, len)| {
            Some([f64::from(start), row((id, lane))?, tick(len)?.max(0.0)])
        })
        .collect();
    Shown {
        pointer,
        ghosts,
        ..Shown::default()
    }
}

/// The blocks of `state`'s selected clips on this playlist ([`clip_rect`]). What does not resolve, or lies on a track
/// scrolled out of `rows`, is left out.
fn playlist_outlines(
    song: &Song,
    rows: &[(usize, usize, Rect)],
    axis: TimeAxis,
    state: &Presence,
) -> Vec<Mark> {
    (state.clips.iter())
        .filter_map(|&(id, lane, clip)| {
            let channel = song.channels.iter().position(|c| c.id == id)?;
            let &(.., rect) = rows.iter().find(|r| (r.0, r.1) == (channel, lane))?;
            let lane = song.channels[channel].lanes.get(lane)?;
            let clip = lane.clips.iter().find(|c| c.id == clip)?;
            Some(Mark::Box(clip_rect(clip, rect, axis)))
        })
        .collect()
}

/// The hovered point over `area`, while nothing covers it and it is inside what `ui` shows.
fn hovered(ui: &egui::Ui, area: Rect, covered: bool) -> Option<Pos2> {
    let at = ui.input(|i| i.pointer.hover_pos())?;
    (!covered && area.contains(at) && ui.clip_rect().contains(at)).then_some(at)
}

/// The frame's time step, and the jump in points past which the others' marks in `grid` go at once: a quarter of it.
fn pace(ui: &egui::Ui, grid: Rect) -> (f32, f64) {
    let far = f64::from(grid.width().max(grid.height())) / 4.0;
    (ui.input(|i| i.stable_dt), far)
}

/// The playlist's presence, after its clips: this person's pointer over its `grid` (on `rows`, as drawn on `axis`), and
/// the others' pointers, selected clips and drags, on the rows Settings `Show hidden tracks` (`show_hidden`) leaves.
/// Nothing while not connected.
pub(crate) fn playlist(
    ui: &egui::Ui,
    cx: &UiCtx,
    song: &Song,
    (grid, rows, axis): (Rect, &[(usize, usize, Rect)], TimeAxis),
    (covered, show_hidden): (bool, bool),
) {
    LIVE.with_borrow_mut(|live| {
        if !live.connected {
            return;
        }
        if let Some(p) =
            hovered(ui, grid, covered).and_then(|at| playlist_pointer(song, rows, axis, at))
        {
            live.drawn.pointer = Some(p);
        }
        if live.others.is_empty() {
            return;
        }
        let tracks: Vec<_> = (track_rows(song, show_hidden).iter())
            .map(|t| (t.channel, t.lane))
            .collect();
        // Row `v` (a track's index plus the fraction down it), moved down that fraction, while its track shows.
        let row = |v: f64| {
            let i = v.floor().clamp(0.0, tracks.len().saturating_sub(1) as f64);
            let &(channel, lane) = tracks.get(i as usize)?;
            let &(.., rect) = rows.iter().find(|r| (r.0, r.1) == (channel, lane))?;
            Some(rect.translate(vec2(0.0, (v - i) as f32 * rect.height())))
        };
        let row_h = rows.first().map_or(1.0, |r| r.2.height());
        let px = [f64::from(axis.beat_w) / f64::from(PPQ), f64::from(row_h)];
        let (dt, far) = pace(ui, grid);
        let mut moving = false;
        let mut all = Vec::new();
        for peer in live.others.iter_mut().filter(|p| p.state.view != "mixer") {
            let targets = playlist_targets(song, &tracks, &peer.state);
            moving |= peer.on_playlist.step(targets, dt, px, far);
            let shown = &peer.on_playlist;
            let ghosts = (shown.ghosts.iter())
                .filter_map(|&[t, v, len]| {
                    let x = axis.x(t)..=axis.x(t + len);
                    Some(Mark::Box(Rect::from_x_y_ranges(x, row(v)?.y_range())))
                })
                .collect();
            let marks = Marks {
                tip: (shown.pointer).and_then(|[t, v]| Some(pos2(axis.x(t), row(v)?.top()))),
                outlines: playlist_outlines(song, rows, axis, &peer.state),
                ghosts,
                ..Marks::default()
            };
            all.push((peer.color, peer.name.as_str(), marks));
        }
        paint(&ui.painter_at(grid), cx.m, &all);
        if moving {
            ui.ctx().request_repaint();
        }
    });
}

/// A note move or resize under way, or a right-button swipe: the clip's events as it found them, and which of them it
/// changes (deletes).
pub(crate) type NoteDrag<'a> = (&'a [Event], &'a [bool]);

/// A note editor's grid, as it draws it: content ticks across on `axis`, `count` rows `pitch` points tall from `top`
/// down, keys counting up the rows (`flip`: the piano roll's) or down them (drum pads); `item` an event's start, length
/// and key or pad, and `foot` an item's drawn box in the box of its ticks and row.
struct Rows<'a> {
    axis: TimeAxis,
    top: f32,
    pitch: f32,
    count: usize,
    flip: bool,
    item: fn(&Event) -> Option<(f64, f64, u8)>,
    foot: &'a dyn Fn(Rect) -> Rect,
}

impl Rows<'_> {
    /// The row of `key`, from the top.
    fn row(&self, key: u8) -> Option<usize> {
        let k = usize::from(key);
        (k < self.count).then(|| if self.flip { self.count - 1 - k } else { k })
    }

    /// The key of row `row`.
    fn key(&self, row: usize) -> Option<u8> {
        let k = if self.flip {
            self.count.checked_sub(row + 1)?
        } else {
            row
        };
        (row < self.count).then(|| u8::try_from(k).ok()).flatten()
    }

    /// The drawn box of an item from tick `t`, `len` ticks long, on row `v` (from the top, fractional).
    fn item_box(&self, t: f64, v: f64, len: f64) -> Rect {
        let y = self.top + v as f32 * self.pitch;
        let r = Rect::from_x_y_ranges(self.axis.x(t)..=self.axis.x(t + len), y..=y + self.pitch);
        (self.foot)(r)
    }
}

/// A note editor's presence, after its notes: this person's pointer over its `grid` (on `rows`), selected notes (or
/// hits) and the move or resize under way, if any; and the pointers, selected notes and drags of the others in `view`
/// on the same clip. `editing`: the channel, lane and clip it edits; `selected`: whether the clip's event (`i`, `e`)
/// is.
fn notes(
    ui: &egui::Ui,
    cx: &UiCtx,
    song: &Song,
    (view, grid, rows): (&str, Rect, &Rows),
    editing: (usize, usize, Option<u32>),
    (selected, drag): (&Selected, Option<NoteDrag>),
    covered: bool,
) {
    let (channel, lane, Some(id)) = editing else {
        return;
    };
    let Some(clip) = (song.channels.get(channel))
        .and_then(|c| c.lanes.get(lane)?.clips.iter().find(|c| c.id == id))
    else {
        return;
    };
    let editing = (song.channels[channel].id, lane, id);
    LIVE.with_borrow_mut(|live| {
        if !live.connected {
            return;
        }
        if let Some(at) = hovered(ui, grid, covered) {
            let v = (at.y - rows.top) / rows.pitch;
            if let Some(key) = (v >= 0.0).then(|| rows.key(v as usize)).flatten() {
                live.drawn.pointer = Some(Pointer {
                    tick: fine(rows.axis.tick(at.x)),
                    track: None,
                    key: Some(key),
                    y: Some(fraction(v.fract() * rows.pitch, rows.pitch)),
                });
            }
        }
        let notes = |events: &[Event], which: &Selected| -> Vec<(u32, u8, f64)> {
            (events.iter().enumerate())
                .filter(|&(i, e)| which(i, e))
                .filter_map(|(_, e)| {
                    let (pos, len, key) = (rows.item)(e)?;
                    Some((pos as u32, key, fine(len)))
                })
                .take(MAX_SELECTED)
                .collect()
        };
        let now = notes(&clip.events, selected);
        let at = |notes: &[(u32, u8, f64)]| notes.iter().map(|n| (n.0, n.1)).collect();
        // A drag's notes are where it found them in the song the others have, and where it has them so far.
        (live.drawn.notes, live.drawn.drag_notes) = match drag {
            Some((found, which)) => (at(&notes(found, &|i, _| which.get(i) == Some(&true))), now),
            None => (at(&now), Vec::new()),
        };
        let here = |s: &Presence| s.view == view && s.clip == Some(editing);
        let px = [
            f64::from(rows.axis.beat_w) / f64::from(PPQ),
            f64::from(rows.pitch),
        ];
        let (dt, far) = pace(ui, grid);
        let row = |key: u8| rows.row(key).map(|r| r as f64);
        let mut moving = false;
        let mut all = Vec::new();
        for peer in live.others.iter_mut().filter(|p| here(&p.state)) {
            let state = &peer.state;
            let targets = Shown {
                pointer: (state.pointer)
                    .and_then(|p| Some([tick(p.tick)?, row(p.key?)? + down(p.y)])),
                ghosts: (state.drag_notes.iter())
                    .filter_map(|&(pos, key, len)| {
                        Some([f64::from(pos), row(key)?, tick(len)?.max(0.0)])
                    })
                    .collect(),
                ..Shown::default()
            };
            moving |= peer.on_editor.step(targets, dt, px, far);
            let shown = &peer.on_editor;
            // Their selected notes that are still here.
            let outlines = (state.notes.iter())
                .filter_map(|&(pos, key)| {
                    let found = |e: &&Event| {
                        (rows.item)(e).is_some_and(|(p, _, k)| (p as u32, k) == (pos, key))
                    };
                    let (p, len, _) = (rows.item)(clip.events.iter().find(found)?)?;
                    Some(Mark::Box(rows.item_box(p, row(key)?, len)))
                })
                .collect();
            let marks = Marks {
                tip: (shown.pointer)
                    .map(|[t, v]| pos2(rows.axis.x(t), rows.top + v as f32 * rows.pitch)),
                outlines,
                ghosts: (shown.ghosts.iter())
                    .map(|&[t, v, len]| Mark::Box(rows.item_box(t, v, len)))
                    .collect(),
                ..Marks::default()
            };
            all.push((peer.color, peer.name.as_str(), marks));
        }
        paint(&ui.painter_at(grid), cx.m, &all);
        if moving {
            ui.ctx().request_repaint();
        }
    });
}

/// The piano roll's presence ([`notes`]) on its geometry `g`. `roll`: the channel, lane and clip it edits.
pub(crate) fn piano_roll(
    ui: &egui::Ui,
    cx: &UiCtx,
    song: &Song,
    (grid, g): (Rect, &Geom),
    roll: (usize, usize, Option<u32>),
    (selected, drag): (&[bool], Option<NoteDrag>),
    covered: bool,
) {
    let px = 1.0 / cx.m.ppp();
    // A note is drawn 1 px short of its end and of its row's bottom.
    let foot = |r: Rect| {
        Rect::from_min_max(
            r.min,
            pos2((r.right() - px).max(r.left() + px), r.bottom() - px),
        )
    };
    let rows = Rows {
        axis: TimeAxis {
            origin: g.origin.x,
            beat_w: g.beat_w,
        },
        top: g.origin.y,
        pitch: g.row_h,
        count: usize::from(TOP_KEY) + 1,
        flip: true,
        item: note,
        foot: &foot,
    };
    let selected = |i: usize, _: &Event| selected.get(i) == Some(&true);
    let at = ("piano", grid, &rows);
    notes(ui, cx, song, at, roll, (&selected, drag), covered);
}

/// Whether a note editor's event (index, event) is selected.
type Selected<'a> = dyn Fn(usize, &Event) -> bool + 'a;

/// A drum hit's start, length and pad, as a note editor's item.
fn drum_hit(e: &Event) -> Option<(f64, f64, u8)> {
    let (pad, start, len) = drum_grid::edit::hit(e)?;
    Some((start, len, pad))
}

/// The drum editor's note grid's presence ([`notes`]) on its layout `l` and hits' `place`; hits are drawn snapped
/// across, inset 2 px top and bottom. `editing`: the channel, lane and clip it edits.
pub(crate) fn drum_grid(
    ui: &egui::Ui,
    cx: &UiCtx,
    song: &Song,
    (l, place): (&Layout, &Place),
    editing: (usize, usize, Option<u32>),
    (selected, drag): (&[bool], Option<NoteDrag>),
    covered: bool,
) {
    let px = 1.0 / l.ppp;
    let foot = |r: Rect| {
        Rect::from_x_y_ranges(
            l.snap(r.left())..=l.snap(r.right()),
            l.snap(r.top()) + 2.0 * px..=l.snap(r.bottom()) - 2.0 * px,
        )
    };
    let rows = Rows {
        axis: place.axis,
        top: place.row_top,
        pitch: place.pitch,
        count: place.rows,
        flip: false,
        item: drum_hit,
        foot: &foot,
    };
    let selected = |i: usize, _: &Event| selected.get(i) == Some(&true);
    let at = ("drums", l.grid(), &rows);
    notes(ui, cx, song, at, editing, (&selected, drag), covered);
}

/// The step sequencer's presence ([`notes`]) on its layout `l` in time signature `sig`: a hit is drawn on its step's
/// cell ([`step_seq::active_rect`]). `editing`: the channel, lane and clip it edits; `selection`: the selected cells,
/// and a right-button swipe under way, if any: the hits as it found them and which of them it deleted.
pub(crate) fn steps(
    ui: &egui::Ui,
    cx: &UiCtx,
    song: &Song,
    (l, sig): (&Layout, TimeSig),
    editing: (usize, usize, Option<u32>),
    (selection, drag): (&BTreeSet<(u8, usize)>, Option<NoteDrag>),
    covered: bool,
) {
    // The cell of the step a box of ticks starts on (its hit's step: the nearest) and of the row at its middle.
    let foot = |r: Rect| {
        let step = ((r.left() - l.x0) / l.step_w).round().max(0.0) as usize;
        let row = ((r.center().y - l.row_top()) / l.pitch).floor().max(0.0) as usize;
        step_seq::active_rect(l.cell(row, step), l.pitch, l.ppp)
    };
    let rows = Rows {
        axis: TimeAxis {
            origin: l.x0,
            beat_w: l.step_w * sig.steps_per_beat() as f32,
        },
        top: l.row_top(),
        pitch: l.pitch,
        count: l.rows,
        flip: false,
        item: drum_hit,
        foot: &foot,
    };
    let selected = |_: usize, e: &Event| {
        step_seq::edit::cell_of(e, sig).is_some_and(|c| selection.contains(&c))
    };
    let at = ("drums", l.grid(), &rows);
    notes(ui, cx, song, at, editing, (&selected, drag), covered);
}

/// A point editor's plot as it draws it: its frame and time axis (content ticks), which points it edits, their dots'
/// radius and the curve's width.
pub(crate) struct Plot<'a> {
    pub frame: &'a Frame,
    pub axis: TimeAxis,
    pub key: PointKey,
    pub radius: f32,
    pub line: f32,
}

/// Points, and one flag each: whether it is selected.
pub(crate) type PointSel<'a> = (&'a [Point], &'a [bool]);

/// A point editor's presence, after its points: this person's pointer over the plot, their selected points and the
/// move or scale under way, if any (`now`: the points and selection as they show; `found`: as the gesture found
/// them), and in the audio editor its range and a stretch under way (`extra`, content ticks); and the pointers,
/// selections and gestures of the others in `view` on the same clip. A gesture's points show as ghosts where it has
/// them, and the curve it redraws through them as a stroke. `editing`: the channel, lane and clip it edits.
pub(crate) fn points(
    ui: &egui::Ui,
    cx: &UiCtx,
    song: &Song,
    (view, plot, editing): (&str, &Plot, (usize, usize, u32)),
    (now, found): (PointSel, Option<PointSel>),
    extra: (Option<(f64, f64)>, Option<f64>),
    covered: bool,
) {
    let (channel, lane, id) = editing;
    let Some(clip) = (song.channels.get(channel))
        .and_then(|c| c.lanes.get(lane)?.clips.iter().find(|c| c.id == id))
    else {
        return;
    };
    let editing = (song.channels[channel].id, lane, id);
    let env = match plot.key {
        PointKey::Envelope(lane) => u8::try_from(lane).ok(),
        PointKey::Automation => None,
    };
    let (f, axis, max) = (plot.frame, plot.axis, plot.key.max());
    let span = f.value;
    if span.height() <= 0.0 {
        return;
    }
    LIVE.with_borrow_mut(|live| {
        if !live.connected {
            return;
        }
        if let Some(at) = hovered(ui, f.grid, covered) {
            let y = f64::from((at.y - span.top()) / span.height());
            live.drawn.pointer = Some(Pointer {
                tick: fine(axis.tick(at.x)),
                track: None,
                key: None,
                y: Some((y * 1000.0).round() / 1000.0),
            });
        }
        let pick = |(pts, sel): PointSel| -> Vec<(u32, u16)> {
            (pts.iter().zip(sel))
                .filter(|(_, s)| **s)
                .map(|(p, _)| (p.tick, p.value))
                .take(MAX_SELECTED)
                .collect()
        };
        // A gesture's points are where it found them in the song the others have, and where it has them so far.
        (live.drawn.points, live.drawn.drag_points) = match found {
            Some(found) => (pick(found), pick(now)),
            None => (pick(now), Vec::new()),
        };
        let (range, stretch) = extra;
        live.drawn.env = env;
        live.drawn.range = range.map(|(a, b)| (fine(a), fine(b)));
        live.drawn.stretch = stretch.map(fine);
        let here = |s: &Presence| s.view == view && s.clip == Some(editing);
        // A value's place down the span, as a fraction of it: its drawn centre's.
        let level = |value: u16| (points::row(value, max) + 0.5) / 127.0;
        let at = |t: f64, v: f64| pos2(axis.x(t), span.top() + v as f32 * span.height());
        let px = [
            f64::from(axis.beat_w) / f64::from(PPQ),
            f64::from(span.height()),
        ];
        let (dt, far) = pace(ui, f.grid);
        let base = points::points(clip, plot.key);
        let mut moving = false;
        let mut all = Vec::new();
        for peer in live.others.iter_mut().filter(|p| here(&p.state)) {
            let state = &peer.state;
            // Their points are this person's only on the same envelope.
            let fits = |&&(_, v): &&(u32, u16)| state.env == env && v <= max;
            let pointer = (state
                .pointer
                .filter(|p| p.track.is_none() && p.key.is_none()))
            .and_then(|p| Some([tick(p.tick)?, p.y.filter(|y| (-1.0..=2.0).contains(y))?]));
            let targets = Shown {
                pointer,
                ghosts: (state.drag_points.iter().filter(fits))
                    .map(|&(t, v)| [f64::from(t), level(v), 0.0])
                    .collect(),
                spans: [
                    (state.range).and_then(|(a, b)| Some([tick(a)?, tick(b)?])),
                    (state.stretch).and_then(|end| Some([0.0, tick(end)?])),
                ]
                .into_iter()
                .flatten()
                .collect(),
            };
            moving |= peer.on_editor.step(targets, dt, px, far);
            let shown = &peer.on_editor;
            let selected: Vec<_> = state.points.iter().filter(fits).collect();
            let dot = |p: Pos2| Mark::Dot(p, plot.radius);
            let mut outlines: Vec<Mark> = (base.iter())
                .filter(|p| selected.contains(&&(p.tick, p.value)))
                .map(|p| dot(at(f64::from(p.tick), level(p.value))))
                .collect();
            let moved: Vec<Pos2> = (shown.ghosts.iter()).map(|&[t, v, _]| at(t, v)).collect();
            let mut ghosts: Vec<Mark> = moved.iter().map(|&p| dot(p)).collect();
            let mut strokes = Vec::new();
            if !moved.is_empty() {
                // The curve as the gesture has it: the points it left where they are, its own where they show now.
                let mut gone = selected.clone();
                let mut curve: Vec<(Pos2, bool)> = (base.iter())
                    .filter(
                        |p| match gone.iter().position(|q| **q == (p.tick, p.value)) {
                            Some(i) => {
                                gone.swap_remove(i);
                                false
                            }
                            None => true,
                        },
                    )
                    .map(|p| (at(f64::from(p.tick), level(p.value)), false))
                    .chain(moved.iter().map(|&p| (p, true)))
                    .collect();
                curve.sort_by(|a, b| a.0.x.total_cmp(&b.0.x));
                let lines = redrawn(&curve, f.grid.right());
                strokes = lines.into_iter().map(|line| (line, plot.line)).collect();
            }
            // The range's outline, and the sample's extent so far under a stretch.
            let mut spans = shown.spans.iter();
            let extent =
                |&[a, b]: &[f64; 2]| Rect::from_x_y_ranges(axis.x(a)..=axis.x(b), span.y_range());
            if state.range.is_some() {
                outlines.extend(spans.next().map(|s| Mark::Box(extent(s))));
            }
            if state.stretch.is_some() {
                ghosts.extend(spans.next().map(|s| Mark::Box(extent(s))));
            }
            let marks = Marks {
                tip: shown.pointer.map(|[t, v]| at(t, v)),
                outlines,
                ghosts,
                strokes,
                value: None,
            };
            all.push((peer.color, peer.name.as_str(), marks));
        }
        paint(&ui.painter_at(f.grid), cx.m, &all);
        if moving {
            ui.ctx().request_repaint();
        }
    });
}

/// The polylines of a point editor's `curve` (its points left to right, and which a gesture moved) that the gesture
/// redraws: each segment with a moved end, and the level line on from a moved last point to `right`.
fn redrawn(curve: &[(Pos2, bool)], right: f32) -> Vec<Vec<Pos2>> {
    let mut lines: Vec<Vec<Pos2>> = Vec::new();
    let mut open = false;
    for w in curve.windows(2) {
        let on = w[0].1 || w[1].1;
        match lines.last_mut() {
            Some(line) if on && open => line.push(w[1].0),
            _ if on => lines.push(vec![w[0].0, w[1].0]),
            _ => {}
        }
        open = on;
    }
    if let Some(&(last, true)) = curve.last() {
        let end = pos2(right.max(last.x), last.y);
        match lines.last_mut() {
            Some(line) if open => line.push(end),
            _ => lines.push(vec![last, end]),
        }
    }
    lines
}

/// The mixer's presence, after its strips (`unit` points wide from `rect`'s left, scrolled by `scroll`, in rack order;
/// their controls' ids are `base` with (channel, param)): this person's pointer over a strip and the fader or knob
/// they hold, if any; and the pointers of the others in the mixer, each with the value they are setting tagged under
/// their name.
pub(crate) fn mixer(
    ui: &egui::Ui,
    cx: &UiCtx,
    song: &Song,
    (rect, unit, scroll): (Rect, f32, f32),
    base: egui::Id,
    covered: bool,
) {
    LIVE.with_borrow_mut(|live| {
        if !live.connected || unit <= 0.0 || rect.height() <= 0.0 {
            return;
        }
        let order = song.rack_order();
        if let Some(at) = hovered(ui, rect, covered) {
            let x = (at.x - rect.left() + scroll) / unit;
            if let Some(&i) = (x >= 0.0).then(|| order.get(x as usize)).flatten() {
                live.drawn.pointer = Some(Pointer {
                    tick: fraction(x.fract(), 1.0),
                    track: Some((song.channels[i].id, 0)),
                    key: None,
                    y: Some(fraction(at.y - rect.top(), rect.height())),
                });
            }
        }
        // The fader (param 0) or the knob (1) held, by the mixer's ids for them.
        let held = ui.ctx().dragged_id().and_then(|id| {
            (order.iter())
                .flat_map(|&i| [(i, 0_u8), (i, 1)])
                .find(|&(i, param)| base.with((i, u32::from(param))) == id)
        });
        live.drawn.param = held.map(|(i, param)| {
            let c = &song.channels[i];
            (c.id, param, c.params.0[usize::from(param)])
        });
        let place = |id: i32| order.iter().position(|&i| song.channels[i].id == id);
        let px = [f64::from(unit), f64::from(rect.height())];
        let (dt, far) = pace(ui, rect);
        let mut moving = false;
        let mut all = Vec::new();
        for peer in live.others.iter_mut().filter(|p| p.state.view == "mixer") {
            let state = &peer.state;
            let pointer = state.pointer.and_then(|p| {
                let x = Some(p.tick).filter(|x| (0.0..=1.0).contains(x))?;
                Some([place(p.track?.0)? as f64 + x, down(p.y)])
            });
            let targets = Shown {
                pointer,
                ..Shown::default()
            };
            moving |= peer.on_editor.step(targets, dt, px, far);
            let value = (state.param)
                .filter(|&(id, param, v)| {
                    param <= 1 && (0.0..=1.0).contains(&v) && place(id).is_some()
                })
                .map(|(_, param, v)| CHANNEL_PARAMS[usize::from(param)].display_text(v));
            let tip = (peer.on_editor.pointer).map(|[x, y]| {
                let left = rect.left() - scroll + x as f32 * unit;
                pos2(left, rect.top() + y as f32 * rect.height())
            });
            let marks = Marks {
                tip,
                value,
                ..Marks::default()
            };
            all.push((peer.color, peer.name.as_str(), marks));
        }
        paint(&ui.painter_at(rect), cx.m, &all);
        if moving {
            ui.ctx().request_repaint();
        }
    });
}

/// One of a person's marks: an item's box (a clip, a note, a hit, a range), or a point's dot (its centre and radius).
#[derive(Clone, Copy, Debug)]
enum Mark {
    Box(Rect),
    Dot(Pos2, f32),
}

/// What a person shows in a view, in this person's screen terms: their pointer's tip, outlines around what they
/// selected, ghosts of what their drag has moved so far, and the lines it redraws (a curve through dragged points,
/// with the width the editor draws it at), and a value they are setting, tagged under their name.
#[derive(Default)]
struct Marks {
    tip: Option<Pos2>,
    outlines: Vec<Mark>,
    ghosts: Vec<Mark>,
    strokes: Vec<(Vec<Pos2>, f32)>,
    value: Option<String>,
}

/// The session's pointer arrow, tip first, in points of a 32-point U (the desktop's at the default UI scale).
const ARROW: [(f32, f32); 7] = [
    (0.0, 0.0),
    (12.5, 11.0),
    (7.3, 11.0),
    (10.4, 17.3),
    (7.6, 18.6),
    (4.5, 12.2),
    (0.0, 16.0),
];
/// A ghost's fill and edge, and a stroke under way: its person's colour at these opacities.
pub(crate) const GHOST: f32 = 0.28;
pub(crate) const GHOST_EDGE: f32 = 0.85;
pub(crate) const STROKE: f32 = 0.7;

/// White or near-black, whichever reads on `color`: near-black once its relative luminance passes 0.55.
fn ink(color: Color32) -> Color32 {
    let c = egui::Rgba::from(color);
    let luminance = 0.2126 * c.r() + 0.7152 * c.g() + 0.0722 * c.b();
    if luminance > 0.55 {
        Color32::from_gray(0x11)
    } else {
        Color32::WHITE
    }
}

/// The others' `marks` over `painter`'s area, each in their colour and stacked as the session's look has them:
/// everyone's ghosts and strokes, then everyone's selection outlines, then everyone's pointer with their name tag. Sizes
/// follow the UI scale (`m`).
fn paint(painter: &egui::Painter, m: &Metrics, all: &[(Color32, &str, Marks)]) {
    let k = m.u(1.0 / 32.0);
    let shown = painter.clip_rect();
    let near = shown.expand(4.0 * k);
    // Cut to what shows (a remote clip may reach far past it), keeping its edges there.
    let cut = |rect: Rect| Some(rect.intersect(near)).filter(|r| r.is_positive());
    for (color, _, marks) in all {
        let fill = color.gamma_multiply(GHOST);
        // Never under 1.5 device pixels, so a dark colour's ghost keeps its edge on the dark grid at any UI scale.
        let edge = Stroke::new(
            (1.5 * k).max(1.5 / m.ppp()),
            color.gamma_multiply(GHOST_EDGE),
        );
        for &ghost in &marks.ghosts {
            match ghost {
                Mark::Box(rect) => {
                    if let Some(rect) = cut(rect) {
                        painter.rect(rect, 0.0, fill, edge, StrokeKind::Inside);
                    }
                }
                Mark::Dot(c, r) if near.contains(c) => {
                    painter.circle_filled(c, r, fill);
                    painter.circle_stroke(c, (r - edge.width / 2.0).max(0.0), edge);
                }
                Mark::Dot(..) => {}
            }
        }
        for (line, w) in &marks.strokes {
            let stroke = Stroke::new(w.max(2.0 * k), color.gamma_multiply(STROKE));
            painter.add(Shape::line(line.clone(), stroke));
        }
    }
    for (color, _, marks) in all {
        let stroke = Stroke::new(2.0 * k, *color);
        for &outline in &marks.outlines {
            match outline {
                Mark::Box(rect) => {
                    if let Some(rect) = cut(rect) {
                        painter.rect_stroke(rect, 0.0, stroke, StrokeKind::Outside);
                    }
                }
                Mark::Dot(c, r) if near.contains(c) => {
                    painter.circle_stroke(c, r + stroke.width / 2.0, stroke);
                }
                Mark::Dot(..) => {}
            }
        }
    }
    for &(color, name, ref marks) in all {
        let Some(tip) = marks.tip.filter(|p| shown.contains(*p)) else {
            continue;
        };
        let arrow = |at: Pos2| ARROW.iter().map(|&(x, y)| at + k * vec2(x, y)).collect();
        let shadow = Color32::from_black_alpha(64);
        painter.add(Shape::convex_polygon(
            arrow(tip + vec2(0.0, k)),
            shadow,
            Stroke::NONE,
        ));
        let outline = Stroke::new(1.5 * k, Color32::WHITE);
        painter.add(Shape::convex_polygon(arrow(tip), color, outline));
        // The name's tag, and under it the value's.
        let mut at = tip + k * vec2(12.0, 15.0);
        for text in [Some(name), marks.value.as_deref()].into_iter().flatten() {
            if text.is_empty() {
                continue;
            }
            let font = egui::FontId::proportional(m.floor_px(12.0 * k));
            let text = painter.layout_no_wrap(text.to_owned(), font, ink(color));
            let pad = k * vec2(7.0, 3.0);
            let tag = Rect::from_min_size(
                pos2(m.round_px(at.x), m.round_px(at.y)),
                text.size() + 2.0 * pad,
            );
            painter.rect_filled(tag, 6.0 * k, color);
            painter.galley(tag.min + pad, text, ink(color));
            at.y = tag.bottom() + 2.0 * k;
        }
    }
}

impl HoneydewApp {
    /// This person's [`Presence`], for the others.
    pub(crate) fn live_presence(&self) -> Presence {
        let view = self.nav.view;
        let id = |channel: usize| Some(self.song.channels.get(channel)?.id);
        let clip = |r: ClipRef| Some((id(r.channel)?, r.lane, r.id));
        let editor = !matches!(view, View::Playlist | View::Mixer);
        let audio = matches!(view, View::AudioEditor(_));
        let screen = &self.main_screen.clips;
        let show = self.settings.show_hidden_tracks;
        let selected = match screen.group.is_empty() {
            true => screen.selected.into_iter().collect(),
            false => screen.group.clone(),
        };
        let drag = screen.dragging();
        // A drag's clips are where it found them in the song the others have.
        let found = |r: ClipRef| drag.iter().find(|d| d.1 == r).map_or(r, |d| d.0);
        let dragged = |&(_, r): &(ClipRef, ClipRef)| {
            let c = find(&self.song, r)?;
            let len = fine(c.visible_ticks());
            Some((id(r.channel)?, r.lane, c.start_tick, len))
        };
        LIVE.with_borrow(|live| Presence {
            view: match view {
                View::Playlist => "playlist",
                View::PianoRoll(_) => "piano",
                View::DrumEditor(_) => "drums",
                View::Mixer => "mixer",
                View::AutomationEditor(_) => "automation",
                View::AudioEditor(_) => "audio",
            }
            .into(),
            clip: (editor.then(|| screen.clip_for(&self.song, self.selected_channel, show)))
                .flatten()
                .and_then(clip),
            pointer: live.mine.pointer,
            clips: match view {
                View::Mixer => Vec::new(),
                _ => (selected.into_iter().map(found))
                    .filter_map(clip)
                    .take(MAX_SELECTED)
                    .collect(),
            },
            notes: match view {
                View::PianoRoll(_) | View::DrumEditor(_) => live.mine.notes.clone(),
                _ => Vec::new(),
            },
            drag_clips: drag.iter().filter_map(dragged).take(MAX_SELECTED).collect(),
            drag_notes: match view {
                View::PianoRoll(_) | View::DrumEditor(_) => live.mine.drag_notes.clone(),
                _ => Vec::new(),
            },
            points: match view {
                View::AutomationEditor(_) | View::AudioEditor(_) => live.mine.points.clone(),
                _ => Vec::new(),
            },
            drag_points: match view {
                View::AutomationEditor(_) | View::AudioEditor(_) => live.mine.drag_points.clone(),
                _ => Vec::new(),
            },
            env: live.mine.env.filter(|_| audio),
            range: live.mine.range.filter(|_| audio),
            stretch: live.mine.stretch.filter(|_| audio),
            param: live.mine.param.filter(|_| view == View::Mixer),
        })
    }
}

/// The adapter object the Playground shell drives (`shell/live.js`):
/// - `format`: the document format and build;
/// - `checkpoint()`: the live song's canonical shared bytes; throws `busy` while a gesture is under way;
/// - `merge(base, current, edited)`: the pure model merge, for the session's coordinator;
/// - `apply(base, edited)`: a promise that merges `edited` relative to `base` (null: adopts it whole, resolving to the
///   shared song it replaced) at the end of the next quiet frame, or rejects with the conflict;
/// - `connect({ changed, undo })`: `changed()` after a quiet frame whose shared song changed, `undo(redo)` instead of
///   the native Undo and Redo. Both are called from inside a frame: the shell must defer its calls back;
/// - `disconnect()`: native Undo again, from the current song;
/// - `assets()` → `[{ id, name, have }]`: the user samples the live song plays ([`super::user_media`]), and
///   `assetsOf(bytes)` → the same ids for a checkpoint; `readAsset(id)` → their bytes; `writeAsset(id, name, bytes)`
///   keeps one that arrived ([`HoneydewApp::live_write_asset`]). They throw `busy` while the app is not running.
/// - `presence()` → this person's [`super::Presence`] (throws `busy` while the app is not running), and
///   `showPresence(others)` with the shell's `[{ id, name, color, state }]`: the others, drawn in the playlist and the
///   piano roll ([`super::show_presence`]).
#[cfg(target_arch = "wasm32")]
pub(crate) mod web {
    use super::{HoneydewApp, LIVE};
    use crate::collaboration::{Conflict, checkpoint};
    use crate::history::History;
    use wasm_bindgen::{JsCast as _, JsValue, closure::Closure};
    use web_sys::js_sys::{Array, Function, JSON, Object, Promise, Reflect, Uint8Array, global};

    pub(super) struct Hooks {
        changed: Function,
        undo: Function,
    }

    fn error(conflict: Conflict) -> JsValue {
        JsValue::from_str(&conflict.0)
    }

    fn hook(pick: impl FnOnce(&Hooks) -> &Function) -> Option<Function> {
        LIVE.with_borrow(|live| live.web.as_ref().map(|hooks| pick(hooks).clone()))
    }

    pub(super) fn changed() {
        if let Some(changed) = hook(|hooks| &hooks.changed) {
            let _ = changed.call0(&JsValue::NULL);
        }
    }

    pub(super) fn undo(redo: bool) {
        if let Some(undo) = hook(|hooks| &hooks.undo) {
            let _ = undo.call1(&JsValue::NULL, &redo.into());
        }
    }

    /// Offers the adapter as `globalThis.playgroundLiveAdapter`, and to `globalThis.playgroundLive.attach` when the
    /// shell is already there. A page without the shell never connects.
    pub(crate) fn attach(runner: &eframe::WebRunner) -> Result<(), JsValue> {
        let adapter = Object::new();
        let set =
            |name: &str, value: JsValue| Reflect::set(&adapter, &name.into(), &value).map(drop);
        set(
            "format",
            format!("honeydew:1:{}", env!("CARGO_PKG_VERSION")).into(),
        )?;
        let app = runner.clone();
        let checkpoint = move || -> Result<Uint8Array, JsValue> {
            let app = app
                .app_mut::<HoneydewApp>()
                .ok_or("Honeydew is not running")?;
            Ok(app.live_checkpoint().map_err(error)?.as_slice().into())
        };
        set(
            "checkpoint",
            Closure::<dyn Fn() -> _>::new(checkpoint).into_js_value(),
        )?;
        let merge = |base: Uint8Array,
                     current: Uint8Array,
                     edited: Uint8Array|
         -> Result<Uint8Array, JsValue> {
            let merged =
                checkpoint::merge_bytes(&base.to_vec(), &current.to_vec(), &edited.to_vec());
            Ok(merged.map_err(error)?.as_slice().into())
        };
        set(
            "merge",
            Closure::<dyn Fn(_, _, _) -> _>::new(merge).into_js_value(),
        )?;
        let apply = |base: JsValue, edited: Uint8Array| -> Promise {
            let mut job = Some((
                base.dyn_ref::<Uint8Array>().map(Uint8Array::to_vec),
                edited.to_vec(),
            ));
            Promise::new(&mut |resolve, reject| {
                let Some((base, edited)) = job.take() else {
                    return;
                };
                super::queue(
                    base,
                    edited,
                    Box::new(move |result| {
                        let _ = match result {
                            Ok(None) => resolve.call0(&JsValue::NULL),
                            Ok(Some(replaced)) => resolve
                                .call1(&JsValue::NULL, &Uint8Array::from(replaced.as_slice())),
                            Err(conflict) => reject.call1(&JsValue::NULL, &error(conflict)),
                        };
                    }),
                );
            })
        };
        set(
            "apply",
            Closure::<dyn Fn(_, _) -> _>::new(apply).into_js_value(),
        )?;
        let connect = |hooks: JsValue| -> Result<(), JsValue> {
            let get = |name: &str| Reflect::get(&hooks, &name.into())?.dyn_into::<Function>();
            let hooks = Hooks {
                changed: get("changed")?,
                undo: get("undo")?,
            };
            super::connect();
            LIVE.with_borrow_mut(|live| {
                live.web = Some(hooks);
                if let Some(ctx) = &live.ctx {
                    ctx.request_repaint();
                }
            });
            Ok(())
        };
        set(
            "connect",
            Closure::<dyn Fn(_) -> _>::new(connect).into_js_value(),
        )?;
        let app = runner.clone();
        let disconnect = move || {
            super::disconnect();
            // Native Undo starts from the song as the session left it: its steps hold no one else's edits.
            if let Some(mut app) = app.app_mut::<HoneydewApp>() {
                app.history = History::new(&app.song);
            }
        };
        set(
            "disconnect",
            Closure::<dyn Fn()>::new(disconnect).into_js_value(),
        )?;
        let app = runner.clone();
        let assets = move || -> Result<Array, JsValue> {
            let app = app.app_mut::<HoneydewApp>().ok_or("busy")?;
            let list = Array::new();
            for (id, name, have) in app.live_assets() {
                let asset = Object::new();
                Reflect::set(&asset, &"id".into(), &id.into())?;
                Reflect::set(&asset, &"name".into(), &name.into())?;
                Reflect::set(&asset, &"have".into(), &have.into())?;
                list.push(&asset);
            }
            Ok(list)
        };
        set(
            "assets",
            Closure::<dyn Fn() -> _>::new(assets).into_js_value(),
        )?;
        let assets_of = |bytes: Uint8Array| -> Result<Array, JsValue> {
            let ids = super::assets_of(&bytes.to_vec()).map_err(error)?;
            Ok(ids.into_iter().map(JsValue::from).collect())
        };
        set(
            "assetsOf",
            Closure::<dyn Fn(_) -> _>::new(assets_of).into_js_value(),
        )?;
        let app = runner.clone();
        let read_asset = move |id: String| -> Result<Uint8Array, JsValue> {
            let app = app.app_mut::<HoneydewApp>().ok_or("busy")?;
            let bytes = app
                .live_read_asset(&id)
                .ok_or_else(|| format!("No sample {id}"))?;
            Ok(bytes.as_slice().into())
        };
        set(
            "readAsset",
            Closure::<dyn Fn(_) -> _>::new(read_asset).into_js_value(),
        )?;
        let app = runner.clone();
        let write_asset = move |id: String, _name: JsValue, bytes: Uint8Array| {
            let mut app = app.app_mut::<HoneydewApp>().ok_or("busy")?;
            app.live_write_asset(&id, bytes.to_vec());
            Ok::<_, JsValue>(())
        };
        set(
            "writeAsset",
            Closure::<dyn Fn(_, _, _) -> _>::new(write_asset).into_js_value(),
        )?;
        let app = runner.clone();
        let presence = move || -> Result<JsValue, JsValue> {
            let app = app.app_mut::<HoneydewApp>().ok_or("busy")?;
            let state = serde_json::to_string(&app.live_presence()).map_err(|e| e.to_string())?;
            JSON::parse(&state)
        };
        set(
            "presence",
            Closure::<dyn Fn() -> _>::new(presence).into_js_value(),
        )?;
        let show_presence = |others: JsValue| -> Result<(), JsValue> {
            let others = JSON::stringify(&others)?.as_string().unwrap_or_default();
            super::show_presence(&serde_json::from_str(&others).unwrap_or_default());
            Ok(())
        };
        set(
            "showPresence",
            Closure::<dyn Fn(_) -> _>::new(show_presence).into_js_value(),
        )?;
        let global = global();
        Reflect::set(&global, &"playgroundLiveAdapter".into(), &adapter)?;
        let attach = Reflect::get(&global, &"playgroundLive".into())
            .and_then(|live| Reflect::get(&live, &"attach".into()))
            .ok()
            .and_then(|attach| attach.dyn_into::<Function>().ok());
        if let Some(attach) = attach {
            attach.call1(&global, &adapter)?;
        }
        Ok(())
    }
}
