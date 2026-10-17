//! Live editing in a Playground session. The shell's adapter ([`web`]) drives it; nothing here runs until the shell
//! connects. A remote change merges into the live song at the end of a quiet frame, keeping this person's views and
//! unsent work, and is no step of the native undo history. While connected, Undo and Redo go to the session, which
//! reverts only this person's operations.

use super::*;
use crate::collaboration::{Conflict, checkpoint, keep_local, merge, shared};
use crate::flm::chunk::{ContentPath, ContentRoot};
use crate::project_files::MAX_FILE_BYTES;
use crate::sample_cache::SampleKey;
use crate::song::Event;
use crate::ui::clips::{ClipRef, TimeAxis, clip_rect};
use crate::ui::piano_roll::notes::Geom;
use egui::{Color32, Pos2, Rect, Shape, Stroke, StrokeKind, pos2, vec2};
use serde::{Deserialize, Serialize};
use std::collections::VecDeque;
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
    /// This person's pointer and note selection as the editors drawn this frame leave them, and as the last frame
    /// left them: what [`HoneydewApp::live_presence`] reads.
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
    /// (a drag, a held control), no param or sample-row gesture, no channel params outside the history, no recording.
    fn live_quiet(&self, ctx: &egui::Context) -> bool {
        !ctx.input(|i| i.pointer.any_down())
            && !self.param_gesture
            && !self.row_drag
            && !self.params_moved
            && !self.recording()
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
        let ids = &self.channel_ids;
        let gone = |id: i32| before.channels.iter().any(|c| c.id == id) && !ids.contains(&id);
        self.main_screen.forget_keys_views(gone);
        let last = self.song.channels.len() - 1;
        self.selected_channel = self.selected_channel.clamp(last.min(1), last);
        self.rebuild_as(tick, Handover::Undo);
        Ok(replaced)
    }
}

/// Where a person is, for the others in the session: the view shown (`playlist`, `piano`, `drums`, `automation`,
/// `audio`, `mixer`); the clip a clip editor edits; the pointer while it is over the playlist or the piano roll; the
/// playlist's selected clips (unless the mixer covers the playlist) and the piano roll's selected notes. All in the
/// song's terms (channel ids, lanes, clip ids, ticks, keys), so each person draws it through their own zoom and scroll.
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
    /// (content tick, key) of each note.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    notes: Vec<(u32, u8)>,
}

/// The pointer: a song tick on a playlist track (channel id, lane), or a content tick on a key row of the piano roll.
#[derive(Clone, Copy, Debug, PartialEq, Serialize, Deserialize)]
pub(crate) struct Pointer {
    tick: i64,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    track: Option<(i32, usize)>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    key: Option<u8>,
}

#[derive(Default)]
struct Mine {
    pointer: Option<Pointer>,
    notes: Vec<(u32, u8)>,
}

/// Another person, as [`show_presence`] keeps them.
struct Peer {
    name: String,
    color: Color32,
    state: Presence,
}

/// The most selected clips, or notes, a presence carries.
const MAX_SELECTED: usize = 50;
/// The most characters of a name tag.
const MAX_NAME: usize = 24;

/// The shell's list of the others (`[{ id, name, color: "#rrggbb", state }]`), each time it changes; empty at the end.
/// It comes from other people: an entry that does not parse is skipped.
pub(crate) fn show_presence(others: &serde_json::Value) {
    let peer = |v: &serde_json::Value| {
        let hex = v["color"].as_str()?.strip_prefix('#')?;
        let channel = |i: usize| u8::from_str_radix(hex.get(i..i + 2)?, 16).ok();
        let mut state: Presence = serde_json::from_value(v["state"].clone()).ok()?;
        state.clips.truncate(MAX_SELECTED);
        state.notes.truncate(MAX_SELECTED);
        Some(Peer {
            name: v["name"]
                .as_str()
                .unwrap_or_default()
                .chars()
                .take(MAX_NAME)
                .collect(),
            color: Color32::from_rgb(channel(0)?, channel(2)?, channel(4)?),
            state,
        })
    };
    let others = (others.as_array().into_iter().flatten())
        .filter_map(peer)
        .collect();
    LIVE.with_borrow_mut(|live| {
        live.others = others;
        #[cfg(target_arch = "wasm32")]
        if let Some(ctx) = &live.ctx {
            ctx.request_repaint();
        }
    });
}

/// A remote tick, unless it is too far off to draw.
fn tick(t: i64) -> Option<f64> {
    (t.unsigned_abs() <= u64::from(u32::MAX)).then_some(t as f64)
}

/// The pointer at `at` over playlist `rows` ((channel, lane, row rect) as drawn) on time `axis`.
fn playlist_pointer(
    song: &Song,
    rows: &[(usize, usize, Rect)],
    axis: TimeAxis,
    at: Pos2,
) -> Option<Pointer> {
    let &(channel, lane, _) = rows.iter().find(|r| r.2.y_range().contains(at.y))?;
    let track = Some((song.channels.get(channel)?.id, lane));
    let tick = axis.tick(at.x).round() as i64;
    Some(Pointer {
        tick,
        track,
        key: None,
    })
}

/// Where `state` shows on this playlist: its pointer's tip, and its selected clips' blocks ([`clip_rect`]). What does not
/// resolve, or lies on a track scrolled out of `rows`, is left out.
fn playlist_marks(
    song: &Song,
    rows: &[(usize, usize, Rect)],
    axis: TimeAxis,
    state: &Presence,
) -> (Option<Pos2>, Vec<Rect>) {
    let row = |(id, lane): (i32, usize)| {
        let channel = song.channels.iter().position(|c| c.id == id)?;
        let &(.., rect) = rows.iter().find(|r| (r.0, r.1) == (channel, lane))?;
        Some((channel, rect))
    };
    let pointer = state.pointer.and_then(|p| {
        let (_, rect) = row(p.track?)?;
        Some(pos2(axis.x(tick(p.tick)?), rect.center().y))
    });
    let clips = (state.clips.iter())
        .filter_map(|&(id, lane, clip)| {
            let (channel, rect) = row((id, lane))?;
            let clip = song.channels[channel]
                .lanes
                .get(lane)?
                .clips
                .iter()
                .find(|c| c.id == clip)?;
            Some(clip_rect(clip, rect, axis))
        })
        .collect();
    (pointer, clips)
}

/// The pointer at `at` over the piano roll's `grid`, on geometry `g`.
fn roll_pointer(g: &Geom, at: Pos2) -> Option<Pointer> {
    let key = Some(g.key(at.y)?);
    Some(Pointer {
        tick: g.tick(at.x).round() as i64,
        track: None,
        key,
    })
}

/// Where `state` shows on a piano roll of clip `events` on geometry `g`: its pointer's tip, and its selected notes.
fn roll_marks(g: &Geom, events: &[Event], state: &Presence) -> (Option<Pos2>, Vec<Rect>) {
    let pointer = state.pointer.and_then(|p| {
        let key = p.key.filter(|&k| k <= 127)?;
        Some(pos2(g.x(tick(p.tick)?), g.y(key) + g.row_h / 2.0))
    });
    let notes = (state.notes.iter())
        .filter_map(|&(pos, key)| {
            let note = |e: &&Event| matches!(**e, Event::Note { pos: p, key: k, .. } if (p, k) == (pos, key));
            g.note_rect(events.iter().find(note)?)
        })
        .collect();
    (pointer, notes)
}

/// The hovered point over `area`, while nothing covers it and it is inside what `ui` shows.
fn hovered(ui: &egui::Ui, area: Rect, covered: bool) -> Option<Pos2> {
    let at = ui.input(|i| i.pointer.hover_pos())?;
    (!covered && area.contains(at) && ui.clip_rect().contains(at)).then_some(at)
}

/// The playlist's presence, after its clips: this person's pointer over its `grid` (on `rows`, as drawn on `axis`), and
/// the others' pointers and selected clips. Nothing while not connected.
pub(crate) fn playlist(
    ui: &egui::Ui,
    cx: &UiCtx,
    song: &Song,
    (grid, rows, axis): (Rect, &[(usize, usize, Rect)], TimeAxis),
    covered: bool,
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
        let painter = ui.painter_at(grid);
        for peer in live.others.iter().filter(|p| p.state.view != "mixer") {
            paint(
                &painter,
                cx,
                peer,
                playlist_marks(song, rows, axis, &peer.state),
            );
        }
    });
}

/// The piano roll's presence, after its notes: this person's pointer over its `grid` (on geometry `g`) and selected
/// notes, and the pointers and selected notes of the others in a piano roll on the same clip. `roll`: the channel, lane
/// and clip it edits.
pub(crate) fn piano_roll(
    ui: &egui::Ui,
    cx: &UiCtx,
    song: &Song,
    (grid, g): (Rect, &Geom),
    roll: (usize, usize, Option<u32>),
    selected: &[bool],
    covered: bool,
) {
    let (channel, lane, Some(id)) = roll else {
        return;
    };
    let Some(clip) = (song.channels.get(channel))
        .and_then(|c| c.lanes.get(lane)?.clips.iter().find(|c| c.id == id))
    else {
        return;
    };
    let shown = (song.channels[channel].id, lane, id);
    LIVE.with_borrow_mut(|live| {
        if !live.connected {
            return;
        }
        if let Some(p) = hovered(ui, grid, covered).and_then(|at| roll_pointer(g, at)) {
            live.drawn.pointer = Some(p);
        }
        live.drawn.notes = (clip.events.iter().zip(selected))
            .filter(|(_, s)| **s)
            .filter_map(|(e, _)| match *e {
                Event::Note { pos, key, .. } => Some((pos, key)),
                _ => None,
            })
            .take(MAX_SELECTED)
            .collect();
        let painter = ui.painter_at(grid);
        let here = |s: &Presence| s.view == "piano" && s.clip == Some(shown);
        for peer in live.others.iter().filter(|p| here(&p.state)) {
            paint(&painter, cx, peer, roll_marks(g, &clip.events, &peer.state));
        }
    });
}

/// A person's marks in their colour: outlines around their selection, then their pointer, an arrow with a name tag.
fn paint(
    painter: &egui::Painter,
    cx: &UiCtx,
    peer: &Peer,
    (pointer, outlines): (Option<Pos2>, Vec<Rect>),
) {
    let m = cx.m;
    let shown = painter.clip_rect();
    let stroke = Stroke::new(2.0 * m.t(), peer.color);
    for rect in outlines {
        // Cut to what shows (a remote clip may reach far past it), keeping its edges there.
        let rect = rect.intersect(shown.expand(4.0 * m.t()));
        if rect.is_positive() {
            painter.rect_stroke(rect, 0.0, stroke, StrokeKind::Outside);
        }
    }
    let Some(tip) = pointer.filter(|p| shown.contains(*p)) else {
        return;
    };
    let s = m.u(0.7);
    let arrow = vec![tip, tip + vec2(0.0, s), tip + vec2(0.7 * s, 0.7 * s)];
    painter.add(Shape::convex_polygon(
        arrow,
        peer.color,
        Stroke::new(m.t(), Color32::WHITE),
    ));
    if peer.name.is_empty() {
        return;
    }
    let [r, g, b, _] = peer.color.to_array();
    let light = 299 * u32::from(r) + 587 * u32::from(g) + 114 * u32::from(b) > 150_000;
    let ink = if light {
        Color32::BLACK
    } else {
        Color32::WHITE
    };
    let text = painter.layout_no_wrap(peer.name.clone(), m.font_id(FontSize::Pt10), ink);
    let pad = vec2(m.u(0.25), m.u(0.08));
    let tag = Rect::from_min_size(tip + vec2(0.5 * s, 0.85 * s), text.size() + 2.0 * pad);
    painter.rect_filled(tag, m.u(0.15), peer.color);
    painter.galley(tag.min + pad, text, ink);
}

impl HoneydewApp {
    /// This person's [`Presence`], for the others.
    pub(crate) fn live_presence(&self) -> Presence {
        let view = self.nav.view;
        let id = |channel: usize| Some(self.song.channels.get(channel)?.id);
        let clip = |r: ClipRef| Some((id(r.channel)?, r.lane, r.id));
        let editor = !matches!(view, View::Playlist | View::Mixer);
        let screen = &self.main_screen.clips;
        let selected = match screen.group.is_empty() {
            true => screen.selected.into_iter().collect(),
            false => screen.group.clone(),
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
            clip: (editor.then(|| screen.clip_for(&self.song, self.selected_channel)))
                .flatten()
                .and_then(clip),
            pointer: live.mine.pointer,
            clips: match view {
                View::Mixer => Vec::new(),
                _ => selected
                    .into_iter()
                    .filter_map(clip)
                    .take(MAX_SELECTED)
                    .collect(),
            },
            notes: match view {
                View::PianoRoll(_) => live.mine.notes.clone(),
                _ => Vec::new(),
            },
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
