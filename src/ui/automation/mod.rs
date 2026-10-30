//! The automation editor: the control points of one automation clip, on the frame [`geom`] lays out. It edits only the
//! current lane's target (key = param + 1); there is no parameter selector.
//!
//! **Drawing**: 1 `background`; 2 the grid in `controlFill`; 3 five 1 px `background` lines at value
//! top − 0.5 px + V·{0, ¼, ½, ¾, 1}; 4 the piano roll's vertical marks; 5 black α 0.15 over the value area past the
//! loop end; 6 a 1 px `menuFill` line at the target's init value v, y = top + V·(1 − v) − 0.5 px; 7 the curve (1.5 px
//! lines; nothing left of the first point, a level line from the last to the right edge), then the points
//! (radius 0.2 U). A segment is `text` when either end is selected, a point when it is; else the channel colour. The
//! value axis left of the grid: N = 5 ticks from V ≥ 3 U, else 3, in the channel colour.
//!
//! **Values** (GitHub issue #21; [`value`]): the axis labels the
//! target's value at each tick's fraction of the range in its own units (the registry's lane text, `120 BPM` on the
//! tempo lane, percent without a display); the selected points, a drag's live, and the point under a hovering mouse
//! show their value in a label beside them. `Edit value` (the floating row's third button) types one value for every
//! selected point ([`AutomationEditor::value_panel`]); Up and Down nudge the selection a grid step (Shift: ten). The
//! snap menu's `Value` column sets the value snap, an app setting: `Off`, `Rows` (the rows' law, the default), `1 %`
//! … `25 %`, or a discrete target's `Steps`. A grid's lines show under the curve while its snap is on, and dragged,
//! created and pasted points land on it ([`value::Land`]); off, values move continuously.
//!
//! **Gestures** (the shared law, [`gesture::is_long`] and [`gesture::left_calm`]): a tap (released within 300 ms,
//! moved ≤ 0.5 U) on empty grid creates a point, or only clears a shown selection; a press on a point (the nearest
//! within 1.25 U) drags it, or the whole selection when it lands inside the showing selection's touch frame; released
//! without movement it is a tap, which selects the point, or only deselects when a selection showed at the touch-down,
//! or after more than 300 ms a long press, which shows the touch-down's selection without toggling. On empty grid a
//! long press (300 ms within 0.5 U) offers the `Paste` insert menu at the finger's snapped beat when the clipboard
//! holds points ([`paste_menu`]); dragging from there makes a marquee ([`Marquee`]) whose points are the selection
//! live. Shift (the selection modifier) starts one at once. With two or more points selected, the ↔ bar under the
//! selection box's bottom-right scales them in time about the box's start (the box's end, with no grab offset, goes to
//! the finger's tick, snapped, at least grid/8 after the start: [`box_end`]). Each gesture is one final edit at its
//! release, only when the points changed; meanwhile only the drawing follows the finger. A move's release (as the piano
//! roll's) leaves the moved points selected with the floating row hidden, a quiet selection whose frame and handle take
//! no touch (a tap on empty grid still only clears it): a tap on a point then selects it with the row, and a long
//! press, a box, a paste or a key shows a selection again.
//!
//! - b (bar marks, the `Bar` snap, the clip's loop rule) is [`TimeSig::bar_beats`], TDIV byte 1; the zoom limits take
//!   d, byte 0 ([`geom::zoom_limits`]).
//! - The ruler row is the other editors' (`timeline::ruler`, its range and region menu, the playhead) over the
//!   `background` rect of draw step 1; its range is the song's play loop, every op but `Duplicate` clears it.
//! - The playhead is the song tick less the clip's origin, wrapped modulo the loop, so it always shows; `Copy/Move to
//!   playhead` use it.
//! - Lines are snapped to whole device pixels; vertical marks span the grid, the five lines and the init line its
//!   width. The selection guides go under the curve, the marquee (the step view's box) over it.
//! - The selection box as drawn is the points' box grown by their radius (a lone point's own disc square): it pulses
//!   (`text` at α 0.15 + 0.05·cos(2π·p), an 80-frame cycle, [`clips::paint_pulse`]) whenever a point is selected, a
//!   marquee's catch and a drag's points too. The ↔ bar sits under its bottom-right ([`clips::editor_handles`],
//!   checked before the points), grows while held and wobbles at its release; a point's touch hides it. The touch
//!   frame (the box grown by 1.25 U) is the selection the floating row is placed over and re-shown by; it is not
//!   drawn. The row hides while a point or the bar is held and from a completed long press on.
//! - The view is the scroller's ([`scroller`]): one finger scrolls the time on the ruler band only (a drag on empty
//!   grid moves nothing), two zoom it there; the wheel zooms it over the ruler and the grid, as does a trackpad pinch
//!   over the grid.
//! - Created and pasted points pop in; every point of a drag (or scale) grows while it lasts and wobbles at its
//!   release; animations are keyed by the point's tick and value.
//! - The `Paste` menu hides at any other press; it pastes at its snapped beat.
//! - Continuous scrolling turns the time ruler's long press and range drags off; a tap still moves the playhead, which
//!   the app turns into a stop while playing. A drag's beats come from the view as it is each frame, so a held point
//!   stays on the beat under the finger while the view moves.
//! - The saved view is kept per clip key (channel, lane, clip id) in memory: the app re-keys it when tracks move
//!   ([`AutomationEditor::rekey`]), and a view whose clip is gone goes.

pub mod geom;
mod shape;
pub mod value;

use std::collections::HashMap;
use std::f32::consts::TAU;

use eframe::egui::text::{CCursor, CCursorRange};
use eframe::egui::text_edit::TextEditState;
use eframe::egui::{
    self, Align, Align2, Area, Color32, Id, Key, Margin, Order, Painter, Pos2, Rangef, Rect,
    Response, Sense, Stroke, TextEdit, Ui, pos2, vec2,
};

use crate::metrics::{FontSize, Metrics};
use crate::params::menu_value;
use crate::song::points::{self, LAST_ROW, Point, PointKey};
use crate::song::{Clip, LaneKind, PPQ, Song, TimeSig};
use crate::theme::channel_color;
use crate::ui::clips::{
    self, FloatingRow, GridView, HandlePress, Marquee, Part, Selection, TimeAxis, hold_square,
    paint_marquee,
};
use crate::ui::drum_grid::edit::{MARK_GAP, marks};
use crate::ui::edit_keys::{self, EditKey};
use crate::ui::insert;
use crate::ui::piano_roll::Snap;
use crate::ui::piano_roll::notes::{auto_grid, box_end, snap_round};
use crate::ui::popup;
use crate::ui::preview::ClipKey;
use crate::ui::region::{self, RegionOp, TimeRange};
use crate::ui::scroller::{self, Axis, Scroller, X};
use crate::ui::step_seq::pop_in;
use crate::ui::timeline::{self, KnobGesture, PlayheadKnob, RangeRuler, RangeSelect, vline_px};
use crate::ui::widgets::{self, SnapTouch};
use crate::ui::{UiCtx, dialog, gesture, pinch_press, primary_down_live};
use geom::{Frame, HIT_R, View};
use value::{Land, Pick, Target, ValueGrid, ValueSnap};

const KEY: PointKey = PointKey::Automation;

/// An edit's points and the selection that goes with them.
type Edit = (Vec<Point>, Vec<bool>);
/// A point's radius (U) and the curve's width (device pixels).
const POINT_R: f32 = 0.2;
const LINE_PX: f32 = 1.5;
/// Past the loop end: black at α 0.15.
const PAST_END: u8 = 38;
/// The animations' a ← 0.6·a per 60 Hz frame; while dragged it stops at 0.01.
const ANIM_RATE: f32 = 0.6;
const DRAG_FLOOR: f32 = 0.01;
/// The floating buttons over a selection.
const OPS: [&str; 4] = ["Copy", "Delete", "Edit\nvalue", "Set shape"];
/// The value grid's lines: at least this many U apart, `text` at this alpha.
pub(crate) const GRID_GAP: f32 = 0.25;
pub(crate) const GRID_ALPHA: f32 = 0.15;
/// A point's value label: its gap from the point's edge and its padding, in U.
const LABEL_GAP: f32 = 0.15;
const LABEL_PAD: f32 = 0.15;
/// The `Edit value` panel's size and padding, its rows' (top, height) in U from its top, and its buttons' width.
const PANEL: [f32; 2] = [8.0, 4.5];
const PANEL_PAD: f32 = 0.5;
const TITLE_ROW: (f32, f32) = (0.25, 1.0);
const FIELD_ROW: (f32, f32) = (1.25, 1.25);
const BUTTON_ROW: (f32, f32) = (2.75, 1.25);
const BUTTON_W: f32 = 3.0;

fn id() -> Id {
    Id::new("automation_editor")
}

/// The app-wide automation clipboard: (beat, value/max) pairs in egui's memory, so it outlives songs and views and is
/// never saved. The audio editor's envelope lanes share it.
pub fn clipboard(ctx: &egui::Context) -> Vec<points::Copied> {
    ctx.data(|d| d.get_temp(id().with("clipboard")))
        .unwrap_or_default()
}

pub fn set_clipboard(ctx: &egui::Context, points: Vec<points::Copied>) {
    ctx.data_mut(|d| d.insert_temp(id().with("clipboard"), points));
}

/// The points a marquee selects: those its region (the grid-clamped box, a point until dragged) touches.
fn marquee_sel(marquee: Marquee, f: &Frame, ax: TimeAxis, pts: &[Point]) -> Vec<bool> {
    let r = marquee.region(f.grid);
    let ticks = [ax.tick(r.left()), ax.tick(r.right())];
    let rows = [f.row_at(r.top()), f.row_at(r.bottom())];
    points::marquee(pts, ticks, rows, KEY.max())
}

/// A new point's radius scale (pop-in): 1.0515·sin(1.885·(1 − a)).
pub(crate) fn pop_scale(a: f32) -> f32 {
    1.0515 * (1.885 * (1.0 - a)).sin()
}

/// A dragged point's radius scale: 2 − 0.6a, a floored at 0.01 (it grows to about 2×).
pub(crate) fn drag_scale(a: f32) -> f32 {
    2.0 - 0.6 * a.max(DRAG_FLOOR)
}

/// A released point's radius scale (wobble): 1 + (1 − a)·sin(2π·(1 − a)).
pub(crate) fn wobble_scale(a: f32) -> f32 {
    1.0 + (1.0 - a) * (TAU * (1.0 - a)).sin()
}

/// What the host lends the editor for a frame.
#[derive(Clone, Copy, Debug, Default)]
pub struct Options {
    /// The transport's song tick.
    pub playhead: u64,
    /// How far past `playhead` the playhead is drawn ([`crate::audio_link::AudioLink::playhead_lead`]): the line, the
    /// disc and follow playhead only, never an edit.
    pub lead: u64,
    pub playing: bool,
    /// REC is armed: the playhead disc shows the record icon.
    pub armed: bool,
    /// Preview plays this clip.
    pub previewing: bool,
    /// The pointer is over an overlay: the wheel and zoom are not the editor's.
    pub covered: bool,
    /// The setting `Continuous scrolling during playback`.
    pub continuous: bool,
    /// The target's init value, while the song holds another for the frame (its automated value at the playhead,
    /// `app/automation.rs`); `None`: the song's.
    pub init: Option<f32>,
    /// The audio's sample rate (0 without audio), which some value texts read.
    pub sample_rate: f32,
}

/// What a frame did.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct Output {
    /// The song changed: one final edit (an undo step). The play loop alone is no edit.
    pub edited: bool,
    /// A ruler tap: the playhead goes to this song tick.
    pub seek: Option<u64>,
    /// A tap on the playhead knob: the transport plays.
    pub play: bool,
    /// The playhead knob's drop: seek to this song tick, playing or stopped, as the playlist's knob does.
    pub dropped: Option<u64>,
}

#[derive(Clone, Copy, Debug, PartialEq)]
enum Anim {
    Pop,
    Wobble,
}

#[derive(Clone, Debug, Default)]
enum Gesture {
    #[default]
    Idle,
    /// Down on empty grid at `origin` since input time `since`: a tap, a pan or a long press.
    Press {
        origin: Pos2,
        since: f64,
    },
    Pan,
    /// The marquee from a long press, or at once with Shift: its selection is live; a stationary hold offers Paste.
    Box(Marquee),
    Drag(Drag),
}

/// A point (or the ↔ bar) held: the points as they were, recomputed from the finger every frame.
#[derive(Clone, Debug)]
struct Drag {
    /// The ↔ bar: a scale.
    scale: bool,
    origin: Pos2,
    /// The content tick under the press: the view may move under the finger (continuous scrolling).
    origin_tick: f64,
    since: f64,
    orig: Vec<Point>,
    sel: Vec<bool>,
    /// A selection showed at the touch-down: a tap deselects.
    shown: bool,
    /// The finger left the tap's 0.5 U.
    armed: bool,
}

/// The editor's state, one for every channel's automation editor.
#[derive(Default)]
pub struct AutomationEditor {
    /// Snap on, mode `Auto`, when the editor starts.
    pub snap: Snap,
    /// Each clip's view, by (channel, lane, clip id).
    pub(crate) views: HashMap<(usize, usize, u32), View>,
    /// The clip the selection belongs to and its points as the editor last left them: anything else (another clip, an
    /// undo) drops the selection and any gesture.
    seen: Option<(ClipKey, Vec<Point>)>,
    /// One flag per point.
    sel: Vec<bool>,
    /// The selection is quiet: kept, its floating row hidden, as a move's release leaves it (module docs).
    quiet: bool,
    gesture: Gesture,
    region: RangeSelect,
    /// The playhead knob on the ruler.
    knob: PlayheadKnob,
    row: FloatingRow,
    /// The selection box's pulse ([`clips::paint_pulse`]) and its ↔ bar's press animation.
    pulse: f32,
    handle_press: HandlePress,
    snap_menu: bool,
    /// The `Paste` button shows, for a long press here.
    paste: Option<Pos2>,
    anims: Vec<(Point, Anim, f64)>,
    /// When the press on the ruler began, while continuous scrolling keeps its range off.
    ruler_press: f64,
    /// The grid cursor (content tick, value row): where the last touched point was touched, where the keymap's
    /// `paste` and `open` act ([`Self::keys`]); `None` before any: tick 0 on the middle row.
    cursor: Option<(f64, f64)>,
    /// The view's scroll and zoom controller.
    scroller: Scroller,
    /// The value snap (an app setting the host keeps, [`value`]).
    pub value_snap: ValueSnap,
    /// How values land this frame: the value snap on the shown target.
    land: Land,
    /// The `Edit value` panel's text while it is open.
    typing: Option<String>,
    /// The `Set shape` popup, while open ([`shape`]).
    shaping: Option<shape::Shaping>,
}

/// The clip `key` names, on an automation lane.
fn clip_of(song: &Song, key: ClipKey) -> Option<&Clip> {
    let lane = song.channels.get(key.channel)?.lanes.get(key.lane)?;
    key.clip(song).filter(|_| lane.kind == LaneKind::Automation)
}

fn clip_mut(song: &mut Song, key: ClipKey) -> Option<&mut Clip> {
    let lane = song
        .channels
        .get_mut(key.channel)?
        .lanes
        .get_mut(key.lane)?;
    lane.clips.iter_mut().find(|c| c.id == key.id)
}

/// `w` device pixels thick across `xs`, centred on `y` (on whole pixels).
fn hline(painter: &Painter, xs: Rangef, y: f32, w: f32, color: Color32, m: &Metrics) {
    let px = 1.0 / m.ppp();
    let top = m.round_px(y - w * px / 2.0);
    painter.rect_filled(Rect::from_x_y_ranges(xs, top..=top + w * px), 0.0, color);
}

/// The selected points' box on screen, from the points' drawn centres.
fn sel_box(pts: &[Point], sel: &[bool], at: impl Fn(&Point) -> Pos2) -> Option<Rect> {
    let mut chosen = pts.iter().zip(sel).filter(|(_, s)| **s).map(|(p, _)| at(p));
    let first = chosen.next()?;
    Some(chosen.fold(Rect::from_min_max(first, first), |r, p| {
        r.union(Rect::from_min_max(p, p))
    }))
}

impl AutomationEditor {
    /// Another person's change came into the song: the selection follows its points ([`crate::app::live::follow`]),
    /// and the change is no outside change that drops it.
    #[cfg(any(target_arch = "wasm32", test))]
    pub(crate) fn follow(&mut self, song: &Song) {
        let Some((key, seen)) = &mut self.seen else {
            return;
        };
        let pts = clip_of(song, *key).map_or_else(Vec::new, |c| points::points(c, KEY));
        self.sel = crate::app::live::follow(seen, &self.sel, &pts, |p| p.tick);
        *seen = pts;
    }

    /// The editor stops showing with a touch it will not see end (`app::left_view`): the touch ends as a pinch's
    /// does, changing nothing; a drag's preview goes with it.
    pub(crate) fn end_touch(&mut self) {
        self.gesture = Gesture::Idle;
        self.paste = None;
    }

    /// Follows every clip's view and the selection's clip to `track(channel, lane)` after channels or tracks moved;
    /// what was on a gone track goes.
    pub(crate) fn rekey(&mut self, track: &dyn Fn(usize, usize) -> Option<(usize, usize)>) {
        let views = std::mem::take(&mut self.views).into_iter();
        let moved = |((c, l, id), v)| track(c, l).map(|(c, l)| ((c, l, id), v));
        self.views = views.filter_map(moved).collect();
        self.seen = (self.seen.take()).and_then(|(k, pts)| {
            let (channel, lane) = track(k.channel, k.lane)?;
            Some((ClipKey { channel, lane, ..k }, pts))
        });
    }

    /// Draws the clip `key` of `song` (an automation lane's) in `rect` and takes its gestures, editing `song` in place.
    /// Draws nothing when there is no such clip.
    pub fn show(
        &mut self,
        ui: &mut Ui,
        cx: &UiCtx,
        rect: Rect,
        song: &mut Song,
        key: ClipKey,
        opts: &Options,
    ) -> Output {
        let mut out = Output::default();
        let sig = song.time_sig;
        let b = sig.bar_beats();
        let Some(clip) = clip_of(song, key) else {
            return out;
        };
        let (m, u) = (cx.m, cx.m.u(1.0));
        let max = KEY.max();
        let mut pts = points::points(clip, KEY);
        let (mut loop_beats, origin) = (clip.loop_len_beats, region::clip_origin(Some(clip)));
        let now = ui.input(|i| i.time);
        let lane_target = song.channels[key.channel].lanes[key.lane].target;
        let target = Target::of(song, key.channel, lane_target, opts.sample_rate);
        self.land = self.value_snap.land(target.steps(), max);

        // The selection follows the clip; an outside change drops it.
        if (self.seen.as_ref()).is_none_or(|(k, p)| *k != key || *p != pts) {
            if self.seen.as_ref().is_some_and(|(k, _)| *k != key) {
                // Another clip: a fling of the last one's view stops.
                self.scroller = Scroller::default();
            }
            self.seen = Some((key, pts.clone()));
            self.sel = vec![false; pts.len()];
            self.gesture = Gesture::Idle;
            (self.paste, self.typing, self.shaping) = (None, None, None);
        }

        // The view: the clip's saved one, the wheel and zoom, continuous scrolling. Views of gone clips go.
        (self.views)
            .retain(|&(channel, lane, id), _| ClipKey { channel, lane, id }.clip(song).is_some());
        let f = Frame::new(rect, m);
        let vkey = (key.channel, key.lane, key.id);
        let mut view = (self.views.get(&vkey).copied())
            .unwrap_or_else(|| View::initial(pts.first().map(|p| p.tick), u));
        // The scroller (no y): one finger scrolls the time on the ruler band only, two zoom it there; the wheel zooms
        // it over the ruler and the grid, where a trackpad pinch zooms it too. Continuous scrolling keeps the playhead
        // a quarter in, one finger scrolling no time.
        let playhead = geom::playhead(
            opts.playhead as f64 - origin as f64,
            loop_beats * f64::from(PPQ),
        );
        let drawn = geom::playhead(
            (opts.playhead + opts.lead) as f64 - origin as f64,
            loop_beats * f64::from(PPQ),
        );
        let mut axes = [geom::axis(view, u, sig, loop_beats, f.grid), Axis::OFF];
        let was = axes[X];
        let follow = opts.continuous && opts.playing && loop_beats > axes[X].span();
        let options = scroller::Options {
            covered: opts.covered,
            follow,
        };
        let zones = geom::zones(&f);
        let report = (self.scroller).update(ui, id().with("scroll"), u, &zones, &mut axes, options);
        if follow {
            scroller::follow(&mut axes[X], drawn / f64::from(PPQ));
        }
        view = geom::take(view, was, axes[X]);
        let ax = view.axis(f.grid.left());
        let snap = self.snap.active(auto_grid(ax.beat_w, u, sig), sig);

        // The gestures.
        let grid = ui.interact(f.grid, id().with("grid"), Sense::click_and_drag());
        crate::ui::focus::register(ui, grid.id, grid.rect, crate::ui::focus::Kind::Container);
        let edit = self.input(ui, &grid, &f, ax, &pts, snap, m, now);
        // A release's edit shows at once.
        if let Some((new, sel)) = edit {
            out.edited = self.commit(song, key, new, sel, b);
            let clip = clip_of(song, key).expect("committed to it");
            (pts, loop_beats) = (points::points(clip, KEY), clip.loop_len_beats);
        }
        let loop_ticks = loop_beats * f64::from(PPQ);
        let mut edit = None;
        self.views.insert(vkey, view);

        // What shows: a drag's points follow the finger; the song changes at its release.
        let pos = ui.input(|i| i.pointer.interact_pos());
        let (shown_pts, shown_sel) = match (&self.gesture, pos) {
            (Gesture::Drag(d), Some(p)) if d.armed => self.dragged(d, p, &f, ax, snap),
            _ => None,
        }
        .unwrap_or_else(|| (pts.clone(), self.sel.clone()));
        let at = |q: &Point| pos2(ax.x(q.tick.into()), f.point_y(q.value, max));
        let sbox = sel_box(&shown_pts, &shown_sel, at);
        let handle = self.handle(&shown_sel, sbox, m);
        // The point under a mouse with no button down: its value shows.
        let hover = (grid.hover_pos())
            .filter(|_| !ui.input(|i| i.pointer.any_down()) && self.typing.is_none())
            .and_then(|p| f.hit(ax, &shown_pts, max, p, m));

        let ch = &song.channels[key.channel];
        let color = channel_color(ch.hue, cx.theme).to_color32();
        // `Set shape`: its popup over the selection, its picks on the curve, one edit at its close.
        if let Some(mut shaping) = self.shaping.take() {
            let mut own = (pts.iter().zip(&self.sel))
                .filter(|(_, s)| **s)
                .map(|(p, _)| p.shape);
            let first = own.next();
            let own = first.filter(|&f| own.all(|s| s == f));
            let frame = sbox.map(|r| r.expand(m.u(HIT_R)));
            let close = frame.is_none_or(|r| {
                let id = id().with("shape");
                shape::popup(ui.ctx(), cx, id, r, (&mut shaping, own), color).close
            });
            let new = shaping.on(&pts, &self.sel);
            if !close {
                self.shaping = Some(shaping);
            } else if new != pts {
                edit = Some((new, self.sel.clone()));
            }
        }
        let shown_pts = match self.shaping {
            Some(shaping) => shaping.on(&shown_pts, &shown_sel),
            None => shown_pts,
        };
        let init = (opts.init).or_else(|| (ch.lanes[key.lane].target).and_then(|t| ch.param(t)));
        self.paint(
            ui,
            cx,
            &f,
            ax,
            (&shown_pts, &shown_sel),
            sbox,
            (color, init),
            (sig, loop_ticks),
            (&target, hover),
            now,
        );
        // The ↔ bar under the selection box's bottom-right.
        let held = match &self.gesture {
            Gesture::Drag(d) if d.scale => Some((Part::Scale, d.since)),
            _ => None,
        };
        self.handle_press.follow(held, now);
        if let Some((bar, _)) = handle {
            let growth = self.handle_press.growth(ui.ctx(), Part::Scale, now);
            clips::paint_handle(&ui.painter_at(f.grid), cx, bar, (1.0, growth), true);
        }
        #[cfg(any(target_arch = "wasm32", test))]
        crate::app::live::points(
            ui,
            cx,
            song,
            (
                "automation",
                &crate::app::live::Plot {
                    frame: &f,
                    axis: ax,
                    key: KEY,
                    radius: m.u(POINT_R),
                    line: LINE_PX / m.ppp(),
                },
                (key.channel, key.lane, key.id),
            ),
            (
                (&shown_pts, &shown_sel),
                match &self.gesture {
                    Gesture::Drag(d) if d.armed => Some((&d.orig[..], &d.sel[..])),
                    _ => None,
                },
            ),
            (None, None),
            opts.covered,
        );

        // The floating row over the selection, the `Paste` button.
        // Hidden while a point is held, and by a long press or its marquee.
        let hidden = self.quiet
            || self.paste.is_some()
            || self.typing.is_some()
            || self.shaping.is_some()
            || matches!(self.gesture, Gesture::Drag(_) | Gesture::Box(_));
        let handles: Vec<Rect> = handle.into_iter().map(|h| h.0).collect();
        let one_row = {
            let mut rows = (shown_pts.iter().zip(&shown_sel))
                .filter(|(_, s)| **s)
                .map(|(p, _)| p.value);
            rows.next().is_some_and(|v| rows.all(|w| w == v))
        };
        let selection = (sbox.filter(|_| !hidden)).map(|r| Selection {
            rect: r.expand(m.u(HIT_R)),
            one_row,
            handles: &handles,
        });
        let grid_view = GridView {
            grid: f.grid,
            map: [ax.origin, f.value.top(), ax.beat_w, f.value.height()],
        };
        self.row.covered = opts.covered;
        if let Some((i, _)) = self
            .row
            .show(ui, "automation_button", selection, grid_view, &OPS, cx)
        {
            match i {
                0 => set_clipboard(ui.ctx(), points::copy(&pts, &self.sel, max)),
                1 => {
                    let keep = (pts.iter().zip(&self.sel)).filter(|(_, s)| !**s);
                    let keep: Vec<Point> = keep.map(|(p, _)| *p).collect();
                    let n = keep.len();
                    edit = Some((keep, vec![false; n]));
                }
                2 => self.open_value(&target, &pts),
                3 => self.shaping = Some(shape::Shaping::default()),
                _ => {}
            }
            ui.ctx().request_repaint();
        }
        if let Some(new) = self.paste_button(ui, cx, (&f, ax), &pts, snap, now) {
            (edit, self.quiet) = (Some(new), false);
        }
        if let Some(new) = self.value_panel(ui, cx, f.grid, &target, &pts, now) {
            edit = Some(new);
        }

        // The ruler, its range and region menu, and the playhead.
        let ruler = ui.painter_at(f.time);
        timeline::ruler(&ruler, f.time, f.time, ax.origin, ax.beat_w, sig, cx);
        let ruler_at = RangeRuler {
            strip: f.time,
            axis: ax,
            grid: snap,
            sig,
            ink: color,
            range: TimeRange::in_clip(song.play_loop, origin),
            playhead: Some(playhead),
        };
        if report.scrolled[X] || report.zoomed[X] {
            self.region.view_moved();
        }
        let picked = self
            .region
            .show(ui, id().with("region"), &ruler_at, false, cx);
        timeline::scroll_thumb(&ruler, f.time, ax, (0.0, loop_ticks), cx);
        // A ruler press waiting for its long press is the scroller's too, until its range starts.
        if self.region.pending() {
            self.scroller.claim();
        } else if self.region.dragging() {
            self.scroller.yield_touch();
        }
        let mut tap = None;
        if follow {
            // Over the ruler, so that it takes no long press or range drag meanwhile; a tap still moves the playhead.
            let off = ui.interact(f.time, id().with("ruler off"), Sense::click_and_drag());
            if off.is_pointer_button_down_on() && ui.input(|i| i.pointer.primary_pressed()) {
                self.ruler_press = now;
            }
            if off.clicked()
                && !gesture::is_long(now - self.ruler_press)
                && let Some(p) = off.interact_pointer_pos()
            {
                tap = Some(snap_round(ax.tick(p.x).max(0.0), snap) as u32);
            }
        }
        if let Some(range) = picked.range {
            song.play_loop = range.and_then(|r| r.song_loop(origin));
        }
        if let Some(tick) = picked.playhead.or(tap) {
            out.seek = Some(region::seek_tick(clip_of(song, key), tick, opts.previewing));
        }
        if let Some((op, range)) = picked.pick {
            if !op.keeps_loop() {
                song.play_loop = None;
            }
            let mut new = pts.clone();
            region_op(&mut new, op, range, playhead as u32, loop_ticks as u32);
            if new != pts {
                let n = new.len();
                edit = Some((new, vec![false; n]));
            }
        }
        // Clipped at the value column's right edge: at beat 0 the line's left half shows in the t gap. While the knob
        // is dragged, the line and the disc follow the finger.
        let line = ui.painter_at(f.grid.with_min_x(f.column.right()));
        let x = (self.knob).line_x(ui, m, &ruler_at, loop_ticks - 1.0, ax.x(drawn));
        self.snap_button(ui, f.snap, color, cx);
        // After the Snap button, which the disc overhangs at beat 0 (the disc on top), and whose touches the knob takes
        // there while it shows, as the playlist's.
        timeline::playhead_line(&line, f.grid, x, cx);
        self.knob.armed = opts.armed;
        let shown = timeline::knob_shown(opts.playing, self.row.showing());
        match (self.knob).show(ui, id().with("knob"), f.time, x, shown, cx) {
            Some(KnobGesture::Tap) => out.play = true,
            Some(KnobGesture::Drop(x)) => {
                let tick = self.knob.drop_tick(ui, &ruler_at, loop_ticks - 1.0, x) as u32;
                out.dropped = Some(region::seek_tick(clip_of(song, key), tick, opts.previewing));
            }
            None => {}
        }
        self.snap_menu(ui, f.snap, cx, target.steps().is_some());
        // After the menus, which take Escape first.
        if let Some(new) = self.keys(ui, &pts, snap, now) {
            edit = Some(new);
        }

        if let Some((new, sel)) = edit {
            out.edited |= self.commit(song, key, new, sel, b);
        }
        out
    }

    /// The keymap's editing actions while the grid holds the keyboard (`ui/edit_keys.rs`), on the selected points as
    /// the floating row's: `copy` fills the [`clipboard`], `cut` too, `cut` and `delete` remove the selection, `paste`
    /// lands the clipboard at the cursor's beat as the `Paste` button does, `open` adds a point at the cursor as a tap
    /// would (`Add point`), `close` deselects; `selectAll` is the piano roll's only. Returns the edit; each action
    /// hides the floating row and the `Paste` button. No editing action during a touch.
    #[allow(clippy::too_many_arguments)]
    fn keys(&mut self, ui: &Ui, pts: &[Point], snap: Option<f64>, now: f64) -> Option<Edit> {
        let ctx = ui.ctx();
        if !matches!(self.gesture, Gesture::Idle) || self.shaping.is_some() {
            return None;
        }
        // Up and Down nudge a selection by a grid step, Shift by ten (a Honeydew addition): the grid takes the
        // arrows from the keyboard focus while it holds a selection.
        if self.sel.contains(&true) {
            crate::ui::focus::hold_vertical(ctx, id().with("grid"));
            if let Some((up, shift)) = edit_keys::vertical(ctx) {
                let lines = if shift { 10 * up } else { up };
                let edit = value::nudge(pts, &self.sel, lines, self.land, KEY.max())?;
                self.wobble(&edit.0, &edit.1, now);
                return Some(edit);
            }
        }
        let k = edit_keys::take(ctx)?;
        self.row.hide();
        // A key shows the selection again, as before a move left it quiet.
        (self.paste, self.quiet) = (None, false);
        ctx.request_repaint();
        let max = KEY.max();
        let chosen = self.sel.contains(&true);
        if matches!(k, EditKey::Copy | EditKey::Cut) && chosen {
            set_clipboard(ctx, points::copy(pts, &self.sel, max));
        }
        let (tick, row) = self.cursor.unwrap_or((0.0, LAST_ROW / 2.0));
        let mut edit = match k {
            EditKey::Close => {
                self.sel.fill(false);
                return None;
            }
            EditKey::Copy | EditKey::SelectAll => return None,
            EditKey::Cut | EditKey::Delete if chosen => {
                let keep = (pts.iter().zip(&self.sel)).filter(|(_, s)| !**s);
                let keep: Vec<Point> = keep.map(|(p, _)| *p).collect();
                let n = keep.len();
                return Some((keep, vec![false; n]));
            }
            EditKey::Cut | EditKey::Delete => return None,
            EditKey::Paste => {
                let clipboard = clipboard(ctx);
                if clipboard.is_empty() {
                    return None;
                }
                points::paste(pts, &clipboard, tick, snap, max)
            }
            EditKey::Open => points::create(pts, tick, row, snap, max),
        };
        value::landed(&mut edit.0, &edit.1, 0.0, self.land, max);
        self.pop(&edit.0, &edit.1, now);
        Some(edit)
    }

    /// Taps, presses, long presses and drags on the grid (module docs). Returns the edit a release made and the pan.
    #[allow(clippy::too_many_arguments)]
    fn input(
        &mut self,
        ui: &Ui,
        grid: &Response,
        f: &Frame,
        ax: TimeAxis,
        pts: &[Point],
        snap: Option<f64>,
        m: &Metrics,
        now: f64,
    ) -> Option<Edit> {
        let max = KEY.max();
        let down = primary_down_live(ui, grid.id);
        // A second finger makes a pinch, which only zooms: it ends any gesture, changing nothing. So does a press held
        // over a focus loss, which is up and stale (`primary_down_live`): no tap, drop or `Paste`.
        let stale = !down && ui.input(|i| i.pointer.primary_down());
        if pinch_press(ui, grid.id) || stale || ui.input(|i| !i.focused || crate::ui::focus_lost(i))
        {
            self.gesture = Gesture::Idle;
            self.paste = None;
            return None;
        }
        let (pressed, pos, shift) = ui.input(|i| {
            let p = &i.pointer;
            (p.primary_pressed(), p.interact_pos(), i.modifiers.shift)
        });
        // Shift, or the keymap's `editorSelect` (Ctrl/Cmd held).
        let shift = shift || crate::keymap::held(ui.ctx(), "editorSelect");
        let u = m.u(1.0);
        let moved = |o: Pos2| gesture::left_calm(o, pos, u);
        self.quiet &= self.sel.contains(&true);
        if grid.is_pointer_button_down_on() {
            // A long press needs a frame once its time has passed.
            ui.ctx().request_repaint();
        }

        if pressed
            && grid.is_pointer_button_down_on()
            && let Some(p) = pos
        {
            self.paste = None;
            let target = (!shift).then(|| self.target(f, ax, pts, p, m)).flatten();
            // The showing selection's touch frame (the box grown by the hit radius): a touch inside acts on it whole.
            // A quiet selection is not showing.
            let at = |q: &Point| pos2(ax.x(q.tick.into()), f.point_y(q.value, max));
            let frame = sel_box(pts, &self.sel, at).map(|b| b.expand(m.u(HIT_R)));
            let shown = self.sel.contains(&true) && !self.quiet;
            let inside = shown && frame.is_some_and(|r| r.contains(p));
            let drag = |scale, sel: &[bool]| {
                Gesture::Drag(Drag {
                    scale,
                    origin: p,
                    origin_tick: ax.tick(p.x),
                    since: now,
                    orig: pts.to_vec(),
                    sel: sel.to_vec(),
                    shown: shown && !scale,
                    armed: false,
                })
            };
            self.gesture = match target {
                _ if shift => Gesture::Box(Marquee::at(p, true)),
                Some((i, scale)) => {
                    // The point becomes the cursor target.
                    self.cursor = Some((ax.tick(p.x).max(0.0), f.row_at(p.y)));
                    if !scale && !self.sel[i] && !inside {
                        // Touched: it becomes the selection.
                        self.sel = (0..pts.len()).map(|j| j == i).collect();
                    }
                    drag(scale, &self.sel)
                }
                None if inside => drag(false, &self.sel),
                None => Gesture::Press {
                    origin: p,
                    since: now,
                },
            };
        }

        let mut edit = None;
        match std::mem::take(&mut self.gesture) {
            Gesture::Press { origin, since } => {
                let held = gesture::is_long(now - since);
                self.gesture = if held && down {
                    // The marquee, following the finger from now on: what it touches is the selection (under the
                    // finger: nothing on empty grid).
                    let mut marquee = Marquee::at(origin, false);
                    marquee.follow(pos, u);
                    (self.sel, self.quiet) = (marquee_sel(marquee, f, ax, pts), false);
                    if !marquee.dragged {
                        self.offer_paste(ui, origin);
                    }
                    Gesture::Box(marquee)
                } else if moved(origin) {
                    // A drag: no tap and no marquee, and nothing scrolls here (the touch x zone is the ruler band).
                    Gesture::Pan
                } else if !down {
                    if held {
                        // Released in the frame the long press came due.
                        self.offer_paste(ui, origin);
                    } else if self.sel.contains(&true) {
                        // A tap: a shown (or quiet) selection only clears; else a new point, selected.
                        self.sel.fill(false);
                    } else {
                        let (mut new, sel) =
                            points::create(pts, ax.tick(origin.x), f.row_at(origin.y), snap, max);
                        value::landed(&mut new, &sel, 0.0, self.land, max);
                        self.pop(&new, &sel, now);
                        edit = Some((new, sel));
                    }
                    Gesture::Idle
                } else {
                    // Its square grows while held.
                    ui.ctx().request_repaint();
                    Gesture::Press { origin, since }
                };
            }
            Gesture::Pan if down => self.gesture = Gesture::Pan,
            Gesture::Box(mut marquee) => {
                // Live: the points the box touches are the selection, every frame.
                marquee.follow(pos, u);
                (self.sel, self.quiet) = (marquee_sel(marquee, f, ax, pts), false);
                if marquee.dragged {
                    self.paste = None;
                } else {
                    self.offer_paste(ui, marquee.from);
                }
                if down {
                    self.gesture = Gesture::Box(marquee);
                }
            }
            Gesture::Drag(mut d) => {
                d.armed |= moved(d.origin);
                if down {
                    self.gesture = Gesture::Drag(d);
                } else {
                    // A move's release leaves its selection quiet; a tap, a long press or a scale shows it.
                    self.quiet = d.armed && !d.scale;
                    let result = match pos {
                        Some(p) if d.armed => self.dragged(&d, p, f, ax, snap),
                        _ => None,
                    };
                    match result {
                        Some((new, sel)) => {
                            self.wobble(&new, &sel, now);
                            edit = Some((new, sel));
                        }
                        None => {
                            self.wobble(&d.orig, &d.sel, now);
                            // A tap while a selection showed deselects it; else the point alone (out of a quiet
                            // selection too) is the selection. A long press only shows the touch-down's selection.
                            if !d.armed && !d.scale && !gesture::is_long(now - d.since) {
                                let point = f.hit(ax, pts, max, d.origin, m).filter(|_| !d.shown);
                                self.sel = (0..pts.len()).map(|j| Some(j) == point).collect();
                            }
                        }
                    }
                }
            }
            Gesture::Pan | Gesture::Idle => {}
        }
        edit
    }

    /// A stationary long press offers Paste while the clipboard holds points.
    fn offer_paste(&mut self, ui: &Ui, origin: Pos2) {
        self.paste = (!clipboard(ui.ctx()).is_empty()).then_some(origin);
    }

    /// What a press at `p` holds: (the ↔ bar, its first point's index) first, else (the nearest point, false).
    fn target(
        &self,
        f: &Frame,
        ax: TimeAxis,
        pts: &[Point],
        p: Pos2,
        m: &Metrics,
    ) -> Option<(usize, bool)> {
        let at = |q: &Point| pos2(ax.x(q.tick.into()), f.point_y(q.value, KEY.max()));
        let sbox = sel_box(pts, &self.sel, at);
        if let Some((_, zone)) = self.handle(&self.sel, sbox, m)
            && zone.contains(p)
        {
            let i = self.sel.iter().position(|s| *s)?;
            return Some((i, true));
        }
        f.hit(ax, pts, KEY.max(), p, m).map(|i| (i, false))
    }

    /// The ↔ bar and its touch zone: with ≥ 2 selected points, under the bottom-right of their box as drawn (the
    /// points' centres' box grown by their radius). None while the selection is quiet or a point is held.
    fn handle(&self, sel: &[bool], sbox: Option<Rect>, m: &Metrics) -> Option<(Rect, Rect)> {
        let count = sel.iter().filter(|s| **s).count();
        let held = matches!(&self.gesture, Gesture::Drag(d) if !d.scale);
        let b = sbox.filter(|_| count >= 2 && !self.quiet && !held)?;
        let (_, bar, zone) = clips::editor_handles(b.expand(m.u(POINT_R)), false, true, m).pop()?;
        Some((bar, zone))
    }

    /// A drag's points and selection with the finger at `p`, `None` when they did not change.
    fn dragged(
        &self,
        d: &Drag,
        p: Pos2,
        f: &Frame,
        ax: TimeAxis,
        snap: Option<f64>,
    ) -> Option<Edit> {
        // In beats from the view as it is now.
        let dt = ax.tick(p.x) - d.origin_tick;
        if d.scale {
            // No grab offset: the box's end goes to the finger's tick.
            points::scaled_to(&d.orig, &d.sel, |left| box_end(ax.tick(p.x), left, snap))
        } else {
            let (dr, max) = (f.row_at(p.y) - f.row_at(d.origin.y), KEY.max());
            if self.land.on && self.land.step.is_none() {
                return points::moved(&d.orig, &d.sel, dt, dr, snap, max);
            }
            // Snap off or a grid: the time moves as with rows, the values continuously or onto the grid.
            let (mut pts, sel) = points::moved(&d.orig, &d.sel, dt, 0.0, snap, max)
                .unwrap_or_else(|| (d.orig.clone(), d.sel.clone()));
            let dv = -dr * f64::from(max) / LAST_ROW;
            value::landed(&mut pts, &sel, dv, self.land, max);
            (pts != d.orig).then_some((pts, sel))
        }
    }

    fn pop(&mut self, pts: &[Point], sel: &[bool], now: f64) {
        self.animate(pts, sel, Anim::Pop, now);
    }

    fn wobble(&mut self, pts: &[Point], sel: &[bool], now: f64) {
        self.animate(pts, sel, Anim::Wobble, now);
    }

    fn animate(&mut self, pts: &[Point], sel: &[bool], anim: Anim, now: f64) {
        let chosen = pts.iter().zip(sel).filter(|(_, s)| **s);
        self.anims.extend(chosen.map(|(p, _)| (*p, anim, now)));
    }

    /// The radius scale of `p` (selected: `selected`) now.
    fn scale(&self, p: &Point, selected: bool, now: f64) -> f32 {
        if let Gesture::Drag(d) = &self.gesture
            && selected
        {
            return drag_scale(pop_in(ANIM_RATE, now - d.since));
        }
        match self.anims.iter().rev().find(|a| a.0 == *p) {
            Some(&(_, Anim::Pop, t)) => {
                let a = pop_in(ANIM_RATE, now - t);
                if a > 0.0 { pop_scale(a) } else { 1.0 }
            }
            Some(&(_, Anim::Wobble, t)) => wobble_scale(pop_in(ANIM_RATE, now - t)),
            None => 1.0,
        }
    }

    /// Draw steps 1–7, the value axis, the selection guides and the marquee (module docs).
    #[allow(clippy::too_many_arguments)]
    fn paint(
        &mut self,
        ui: &Ui,
        cx: &UiCtx,
        f: &Frame,
        ax: TimeAxis,
        (pts, sel): (&[Point], &[bool]),
        sbox: Option<Rect>,
        (color, init): (Color32, Option<f32>),
        (sig, loop_ticks): (TimeSig, f64),
        (target, hover): (&Target, Option<usize>),
        now: f64,
    ) {
        let (c, m) = (&cx.theme.colors, cx.m);
        let px = 1.0 / m.ppp();
        let (bg, text) = (c.background.to_color32(), c.text.to_color32());
        let (grid, value) = (f.grid, f.value);
        // 1, 2.
        let painter = ui.painter_at(f.ruler.union(f.column).union(grid));
        painter.rect_filled(painter.clip_rect(), 0.0, bg);
        painter.rect_filled(grid, 0.0, c.control_fill.to_color32());
        let gp = ui.painter_at(grid);
        // A line of w px down the grid starts ⌊w/2⌋ px left of its x (the drum grid's marks).
        let vline = |x: f32, w: f32, color| {
            let x = m.round_px(x) - (w / 2.0).floor() * px;
            vline_px(&gp, x, grid.y_range(), w, color, m);
        };
        // 3.
        for i in 0..5 {
            let y = value.top() - 0.5 * px + value.height() * i as f32 / 4.0;
            hline(&gp, grid.x_range(), y, 1.0, bg, m);
        }
        // The value grid with value snap on: a line through the centre of each value it lands on, when at least
        // 0.25 U apart (rows never are at the editor's sizes).
        if let Some(g) = self.land.grid() {
            let max = KEY.max();
            let apart = (f.point_y(0, max) - f.point_y(max, max)) * (g / f64::from(max)) as f32;
            if apart >= m.u(GRID_GAP) {
                for v in self.land.lines(max) {
                    let ink = text.gamma_multiply(GRID_ALPHA);
                    hline(&gp, grid.x_range(), f.point_y(v, max), 1.0, ink, m);
                }
            }
        }
        // 4.
        let (t0, t1) = (ax.tick(grid.left()), ax.tick(grid.right()));
        for (tick, w) in marks(t0, t1, ax.beat_w, m.u(1.0), sig, MARK_GAP) {
            vline(ax.x(tick), w, bg);
        }
        // 5.
        let end = ax.x(loop_ticks).max(grid.left());
        if end < grid.right() {
            let past = Rect::from_x_y_ranges(end..=grid.right(), value.y_range());
            gp.rect_filled(past, 0.0, Color32::from_black_alpha(PAST_END));
        }
        // 6.
        if let Some(v) = init {
            let y = value.top() + value.height() * (1.0 - v) - 0.5 * px;
            hline(&gp, grid.x_range(), y, 1.0, c.menu_fill.to_color32(), m);
        }
        // The selection guides: left and top, right and bottom too once the box is wider or taller than 1 px.
        if let Some(r) = sbox {
            vline(r.left(), 1.0, text);
            hline(&gp, grid.x_range(), r.top(), 1.0, text, m);
            if r.width() > px {
                vline(r.right(), 1.0, text);
            }
            if r.height() > px {
                hline(&gp, grid.x_range(), r.bottom(), 1.0, text, m);
            }
        }
        // 7.
        let at = |q: &Point| pos2(ax.x(q.tick.into()), f.point_y(q.value, KEY.max()));
        let stroke = |on: bool| Stroke::new(LINE_PX * px, if on { text } else { color });
        for (i, w) in pts.windows(2).enumerate() {
            let (a, z, s) = (at(&w[0]), at(&w[1]), w[0].shape);
            if z.x >= grid.left() && a.x <= grid.right() {
                let stroke = stroke(sel[i] || sel[i + 1]);
                if s == points::Shape::default() {
                    gp.line_segment([a, z], stroke);
                } else {
                    gp.add(egui::Shape::line(shape::curve(s, a, z), stroke));
                }
            }
        }
        if let Some(last) = pts.last() {
            let a = at(last);
            let from = pos2(a.x.max(grid.left()), a.y);
            gp.line_segment([from, pos2(grid.right(), a.y)], stroke(sel[pts.len() - 1]));
        }
        self.anims.retain(|a| pop_in(ANIM_RATE, now - a.2) > 0.0);
        let mut moving = !self.anims.is_empty();
        let r = m.u(POINT_R);
        for (q, &s) in pts.iter().zip(sel) {
            let p = at(q);
            if (grid.left() - r..=grid.right() + r).contains(&p.x) {
                gp.circle_filled(p, r * self.scale(q, s, now), if s { text } else { color });
            }
        }
        if let Gesture::Drag(d) = &self.gesture {
            moving |= pop_in(ANIM_RATE, now - d.since) > DRAG_FLOOR;
        }
        if moving {
            ui.ctx().request_repaint();
        }
        // The selection box pulsing over the selected points as drawn, a marquee's catch too.
        clips::paint_pulse(&gp, cx, sbox.map(|b| b.expand(r)), 0.0, &mut self.pulse);
        // The marquee: a square growing while a touch is held on empty grid, then the box.
        let marquee = match self.gesture {
            Gesture::Press { origin, since } => hold_square(origin, m.u(1.0), now - since),
            Gesture::Box(marquee) if marquee.dragged => Some(marquee.drawn(m.u(1.0))),
            _ => None,
        };
        if let Some(rect) = marquee {
            paint_marquee(&gp, rect, cx);
        }
        // The values of the selected points (a drag's live) and of the hovered one, in the target's units.
        let mut taken = Vec::new();
        for (i, (q, &s)) in pts.iter().zip(sel).enumerate() {
            if s || hover == Some(i) {
                let r = r * self.scale(q, s, now);
                value_label(&gp, cx, at(q), r, &target.value_text(q.value), &mut taken);
            }
        }

        // The value axis: `background`, ticks and labels in the channel colour.
        painter.rect_filled(f.axis, 0.0, bg);
        // In the target's units ([`Target::axis_text`]): the value at each tick's fraction of the range, fitted from
        // FontSize 1 down to 6 pt, then truncated keeping 5 tail characters. A discrete target with no more steps than
        // ticks has a tick at each step instead; no label repeats one above it.
        let ticks = f.axis_ticks(m);
        let fracs: Vec<f32> = match target.steps() {
            Some(n) if n <= ticks.len() => (0..n).rev().map(|k| menu_value(k, n)).collect(),
            _ => (0..ticks.len())
                .map(|i| 1.0 - i as f32 / (ticks.len() - 1) as f32)
                .collect(),
        };
        let mut shown: Vec<String> = Vec::new();
        for v in fracs {
            let t = f.axis_tick(v, target.axis_text(v), m);
            hline(&painter, t.line, t.y, 2.0, color, m);
            if !shown.contains(&t.label) {
                widgets::shrink(
                    &painter,
                    m,
                    t.text,
                    Align2::CENTER_CENTER,
                    &t.label,
                    8,
                    color,
                );
                shown.push(t.label);
            }
        }
    }

    /// The `Paste` insert menu ([`paste_menu`]); a separate tap commits the clipboard.
    fn paste_button(
        &mut self,
        ui: &mut Ui,
        cx: &UiCtx,
        (f, ax): (&Frame, TimeAxis),
        pts: &[Point],
        snap: Option<f64>,
        now: f64,
    ) -> Option<Edit> {
        let finger = self.paste?;
        let clipboard = clipboard(ui.ctx());
        if clipboard.is_empty() {
            self.paste = None;
            return None;
        }
        let tick = ax.tick(finger.x);
        let tap = paste_menu(
            ui,
            cx,
            id().with("paste"),
            f,
            ax.x(snap_round(tick, snap).max(0.0)),
            finger,
        )?;
        self.paste = None;
        if tap {
            let (mut new, sel) = points::paste(pts, &clipboard, tick, snap, KEY.max());
            value::landed(&mut new, &sel, 0.0, self.land, KEY.max());
            self.pop(&new, &sel, now);
            return Some((new, sel));
        }
        None
    }

    /// `Edit value`: the panel opens on the selected points' value text when they all read the same, else empty.
    fn open_value(&mut self, target: &Target, pts: &[Point]) {
        let chosen = (pts.iter().zip(&self.sel)).filter(|(_, s)| **s);
        let mut texts = chosen.map(|(p, _)| target.value_text(p.value));
        let first = texts.next();
        self.typing = Some(first.filter(|t| texts.all(|u| u == *t)).unwrap_or_default());
    }

    /// The `Edit value` panel while open (a Honeydew addition): a dialog box over the grid's centre with the title, a
    /// field of the typed value (all of it selected as it opens, so typing replaces it) and `Cancel` `OK`. Enter or
    /// `OK` sets every selected point to the typed value ([`Target::parse`]) as one edit and closes it; a text that
    /// names no value keeps it open. Escape, `Cancel` or a tap outside closes it.
    fn value_panel(
        &mut self,
        ui: &Ui,
        cx: &UiCtx,
        grid: Rect,
        target: &Target,
        pts: &[Point],
        now: f64,
    ) -> Option<Edit> {
        let mut text = self.typing.take()?;
        let ctx = ui.ctx().clone();
        let (c, m) = (&cx.theme.colors, cx.m);
        let ink = c.text.to_color32();
        let id = id().with("edit value");
        let field_id = id.with("field");
        let size = vec2(m.u(PANEL[0]), m.u(PANEL[1]));
        let min = grid.center() - size / 2.0;
        let rect = Rect::from_min_size(pos2(m.round_px(min.x), m.round_px(min.y)), size);
        let backdrop = dialog::dialog_box(&ctx, id, (c, m), ctx.content_rect(), rect);
        let mut close =
            backdrop.clicked() || backdrop.secondary_clicked() || crate::ui::close_pressed(&ctx);
        let mut ok = false;
        Area::new(id)
            .order(Order::Foreground)
            .fixed_pos(rect.min)
            .constrain(false)
            .fade_in(false)
            .show(&ctx, |ui| {
                ui.allocate_exact_size(rect.size(), Sense::hover());
                let pad = m.u(PANEL_PAD);
                let row = |(top, h): (f32, f32)| {
                    Rect::from_x_y_ranges(
                        rect.left() + pad..=rect.right() - pad,
                        rect.top() + m.u(top)..=rect.top() + m.u(top + h),
                    )
                };
                let painter = ui.painter().clone();
                let title = row(TITLE_ROW).center();
                let font = m.font_id(FontSize::Pt12);
                painter.text(title, Align2::CENTER_CENTER, "Edit value", font, ink);
                let field = row(FIELD_ROW);
                painter.rect_filled(field, 0.0, c.background.to_color32());
                let visuals = ui.visuals_mut();
                visuals.text_cursor.stroke.color = ink;
                visuals.selection.bg_fill = c.accent.to_color32().gamma_multiply(0.5);
                visuals.selection.stroke.color = ink;
                let edit = TextEdit::singleline(&mut text)
                    .id(field_id)
                    .frame(false)
                    .margin(Margin::symmetric(m.u(0.25) as i8, 0))
                    .font(m.font_id(FontSize::Pt10))
                    .text_color(ink)
                    .desired_width(f32::INFINITY)
                    .min_size(field.size())
                    .vertical_align(Align::Center);
                let mut response = ui.put(field, edit);
                crate::android_text::edit(&mut response, &mut text, usize::MAX);
                ok = response.lost_focus() && ui.input(|i| i.key_pressed(Key::Enter));
                if !response.has_focus() && !response.lost_focus() {
                    response.request_focus();
                    let mut state = TextEditState::load(ui.ctx(), field_id).unwrap_or_default();
                    let all =
                        CCursorRange::two(CCursor::new(0), CCursor::new(text.chars().count()));
                    state.cursor.set_char_range(Some(all));
                    state.store(ui.ctx(), field_id);
                }
                let buttons = row(BUTTON_ROW);
                let w = m.u(BUTTON_W);
                for (part, x) in [("Cancel", buttons.left()), ("OK", buttons.right() - w)] {
                    let at = Rect::from_x_y_ranges(x..=x + w, buttons.y_range());
                    let b = dialog::button(id.with(part), at, part, c, m);
                    if widgets::button(ui, cx, &b, true).clicked() {
                        (ok, close) = (ok || part == "OK", close || part == "Cancel");
                    }
                }
            });
        let mut edit = None;
        if let Some(v) = ok.then(|| target.parse(&text, KEY.max())).flatten() {
            let set = |(p, s): (&Point, &bool)| Point {
                value: if *s { v } else { p.value },
                ..*p
            };
            let new: Vec<Point> = pts.iter().zip(&self.sel).map(set).collect();
            self.wobble(&new, &self.sel.clone(), now);
            edit = (new != pts).then(|| (new, self.sel.clone()));
            close = true;
        }
        if close {
            ctx.memory_mut(|mem| mem.surrender_focus(field_id));
            ctx.request_repaint();
        } else {
            self.typing = Some(text);
        }
        edit
    }

    /// The Snap button in `cell` ([`widgets::snap_button`]): a tap toggles snap, a long press or a right click opens
    /// the snap menu.
    fn snap_button(&mut self, ui: &mut Ui, cell: Rect, ink: Color32, cx: &UiCtx) {
        match widgets::snap_button(ui, id().with("snap"), cell, (self.snap.on, ink), cx) {
            Some(SnapTouch::Menu) => self.snap_menu = true,
            Some(SnapTouch::Toggle) => self.snap.on = !self.snap.on,
            None => {}
        }
    }

    /// The snap menu while open: the time modes under `Time`, the current one highlighted, a pick sets it and turns
    /// snap on; beside them (a Honeydew addition) the value snap under `Value`: `Off`, then the grid sizes (`Steps` only
    /// on a discrete target), the current one highlighted, a pick sets it and turns value snap on.
    fn snap_menu(&mut self, ui: &Ui, cell: Rect, cx: &UiCtx, discrete: bool) {
        if !self.snap_menu {
            return;
        }
        let ctx = ui.ctx();
        let grids = (ValueGrid::ALL.into_iter()).filter(|&g| discrete || g != ValueGrid::Steps);
        let grids = grids.map(|g| (g, g.label()));
        let items = value::snap_items(self.snap.mode, (self.value_snap, "Value"), grids);
        let result = popup::show(
            ctx,
            id().with("snap menu"),
            cell.center(),
            ctx.content_rect(),
            &items,
            cx,
        );
        match result.selected {
            Some(Pick::Time(mode)) => self.snap = Snap { on: true, mode },
            Some(Pick::Value(grid)) => self.value_snap = self.value_snap.picked(grid),
            None => {}
        }
        if result.close {
            self.snap_menu = false;
            ctx.request_repaint();
        }
    }

    /// Makes `new` the clip's points and fits its loop: one final edit when the clip changed. The selection is `sel`.
    fn commit(
        &mut self,
        song: &mut Song,
        key: ClipKey,
        new: Vec<Point>,
        sel: Vec<bool>,
        b: u32,
    ) -> bool {
        let Some(clip) = clip_mut(song, key) else {
            return false;
        };
        let before = clip.clone();
        points::set_points(clip, KEY, new);
        clip.fit_loop(b);
        let now = points::points(clip, KEY);
        self.sel = if sel.len() == now.len() {
            sel
        } else {
            vec![false; now.len()]
        };
        self.seen = Some((key, now));
        *clip != before
    }
}

/// A point's value `text` beside it (centre `at`, drawn radius `r`): above and right of it, left where that passes
/// the grid's right edge, below where it passes the top, so it never covers the point; `text` on a `background` pill,
/// clipped to the grid (`gp`'s clip). One that would overlap a label drawn before (`taken`) is left out.
pub(crate) fn value_label(
    gp: &Painter,
    cx: &UiCtx,
    at: Pos2,
    r: f32,
    text: &str,
    taken: &mut Vec<Rect>,
) {
    let (c, m) = (&cx.theme.colors, cx.m);
    let ink = c.text.to_color32();
    let galley = gp.layout_no_wrap(text.to_owned(), m.font_id(FontSize::Pt8), ink);
    let pad = m.u(LABEL_PAD);
    let size = galley.size() + vec2(2.0 * pad, 0.0);
    let (grid, gap) = (gp.clip_rect(), r + m.u(LABEL_GAP));
    let x = match at.x + gap + size.x <= grid.right() {
        true => at.x + gap,
        false => at.x - gap - size.x,
    };
    let y = match at.y - gap - size.y >= grid.top() {
        true => at.y - gap - size.y,
        false => at.y + gap,
    };
    let rect = Rect::from_min_size(pos2(m.round_px(x), m.round_px(y)), size);
    if taken.iter().any(|t| t.intersects(rect)) {
        return;
    }
    taken.push(rect);
    gp.rect_filled(rect, pad, c.background.to_color32());
    gp.galley(rect.min + vec2(pad, 0.0), galley, ink);
}

/// The region menu's `op` on `pts` (the current key only) for `range`, the playhead at content tick `p` and the loop
/// ending at `loop_end`.
fn region_op(pts: &mut Vec<Point>, op: RegionOp, range: TimeRange, p: u32, loop_end: u32) {
    let (s, e) = (range.start, range.end);
    match op {
        RegionOp::InsertSpace => points::insert_space(pts, s, e),
        RegionOp::Duplicate => points::duplicate(pts, s, e),
        RegionOp::CopyToPlayhead | RegionOp::MoveToPlayhead => {
            points::to_playhead(pts, s, e, p, op == RegionOp::MoveToPlayhead);
        }
        RegionOp::Delete => points::delete(pts, s, e),
        RegionOp::DeleteSpace => points::delete_space(pts, s, e),
        RegionOp::Trim => points::trim(pts, s, e, loop_end),
        RegionOp::Deselect
        | RegionOp::SnapToScale
        | RegionOp::GuessScale
        | RegionOp::InsertSilence
        | RegionOp::Clear
        | RegionOp::Normalize
        | RegionOp::FadeIn
        | RegionOp::FadeOut => {}
    }
}

/// The automation and audio editors' `Paste` insert menu ([`insert::paste`]) for the press at `finger`: over the
/// finger at the snapped beat's `x` (their 127 rows are too fine to place it by),
/// the insert cursor across the value area (a paste keeps the values). `Some(true)` for its click, `Some(false)` for a
/// press elsewhere.
pub(crate) fn paste_menu(
    ui: &mut Ui,
    cx: &UiCtx,
    id: Id,
    f: &Frame,
    x: f32,
    finger: Pos2,
) -> Option<bool> {
    let row = Rangef::point(finger.y);
    insert::paste(ui, cx, id, f.grid, (x, row, f.value.y_range()))
}

#[cfg(test)]
mod tests;
