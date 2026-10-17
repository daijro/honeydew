//! The drum editor's free note-grid: the view of a drum notes lane (kind 3, `DESc`@1 = 1), whose hits have real
//! lengths.
//!
//! The drum editor follows the current lane: kind 3 with flag 0 (`DESc`@1) is the step sequencer above, kind 3 with
//! flag 1 this free note-grid. Neither editor has a mode toggle.
//!
//! Shared with the step sequencer ([`step_seq`]): one row per pad with the same header column, drum mixer and
//! expand/collapse, row pitch, vertical zoom/scroll and `+` button; the row long-press menu with `Rotate left/right`
//! hidden; the time ruler and region menu; the graph-lane knob and layout; floating-button placement; the current-row
//! header fill. The Snap icon button replaces `SWING` in the ruler cell. The pieces are the step view's own
//! ([`step_seq::row_headers`], [`step_seq::overlays`], its layout, [`graph_lane`], [`FloatingRow`]); this view keeps
//! its own fold, current row, graph lane and views (the step-sequencer and free-grid editors each keep their own
//! state).
//!
//! The rest:
//! - **Time axis.** Pinch zoom works on both axes. Beat width = U·d/(4k), d = TDIV byte 0 (4 by default), k = 0.01 … 1
//!   (1 U … 100 U per beat at d = 4). A clip with no saved view opens at 4 U per beat. The horizontal scroll range is
//!   twice the clip length ([`edit::beat_w`], the scroller's range). A beat is the quarter-note beat, `PPQ` ticks.
//! - **Row background.** Each row is one `controlFill` band inset 1 px top and bottom, from beat 0 (or the grid's left
//!   edge, whichever is further right) to the grid's right edge. Vertical `background` lines over all rows, centred on
//!   each mark: 4 px at bar level, 2 px at beat level, 1 px finer; marks never closer than 0.6 U ([`edit::marks`]).
//!   Finer marks split the beat by the subdivision's prime factors, so the 1/d step is a level, then halve it down to a
//!   tick; a line of w px starts ⌊w/2⌋ px left of its mark.
//! - **Hits.** Rectangle x = start, width = length in px (no inset), y = row top + 2 px, height = row pitch − 4 px.
//!   A 1 px `background` frame (the rect grown 1 px) first, then the hit colour; every row's hits are drawn. Pop-in: a
//!   starts at 1, a ← 0.6·a per frame, 0 once under 0.01; width and height scale by 1.0515·sin(1.885·(1 − a)) about the
//!   centre. As in step mode, only a tap's hit pops, counted per 60 Hz frame of input time.
//! - **Hit colour (both drum modes).** The piano-roll note tint with t from the graph-lane page; selected hits use the
//!   selected tint. The page is the lane's item whether the lane shows or not.
//! - **Tap on an empty cell** creates a hit in that row, one undo step (if a selection is showing, the tap only clears
//!   it). Start = tap beat rounded to the nearest snap unit (not rounded with Snap off), ≥ 0. Length = the remembered
//!   length (default 1/4 beat), never below the minimum length. Velocity and pan = the remembered values (defaults 255
//!   and 127); pitch centred; repeat 0 ([`edit::new_hit`]).
//! - **Tap on a hit** selects it: a resize handle at its right end plus the floating `Copy` `Delete` `Snap` row; a tap
//!   on the selected hit deselects it. With the shared touch law ([`gesture`]): a touch is decided from its touch-down
//!   (a tap is released within 0.3 s and 0.5 U, a long press later); touch-down on a hit makes it the selection (inside
//!   the showing selection's frame it acts on the whole selection) with its handles and row hidden, and it drags at
//!   once; a tap while a selection showed only deselects; a long press shows the touch-down's selection. A move's
//!   release (as the piano roll's) leaves the moved hits selected with their handles and row hidden, a quiet selection,
//!   which shows no frame or handle to touch (a tap on empty grid still only deselects it); a tap on a hit then selects
//!   it with them, and a long press, a box, a right click, a paste or a key shows a selection again. On empty grid a
//!   long press starts the marquee at once (lifted in place it selects nothing, and shows `Paste`, below); a right
//!   click on a hit makes it the selection, on empty grid it shows `Paste`.
//! - **Drag a hit** moves it at once (no long press): the start snaps to the nearest snap unit, not before the clip
//!   start; rows move in whole steps (changing the pad), clamped to existing rows; overlaps allowed; release = one undo
//!   step ([`edit::moved`]). A drag on a selected hit moves the selection, a drag on another makes it the selection
//!   first, "touched"; the group keeps its shape at the clip start and the row edges.
//! - **Resize handle** drags the right edge; the end snaps to the nearest snap unit; minimum length = snap unit/8, or
//!   1/128 beat with Snap off ([`edit::resized`]). Every selected hit's end moves by the grabbed one's change; the
//!   handle is the piano roll's, a 0.6 U disc just past the end.
//! - **Marquee and long press.** The marquee selects hits whose extent overlaps it and then shows the resize handle.
//!   Hit-testing uses [start, start + length] (no minimum touch width). A long press on empty grid without dragging
//!   shows the Paste button when the clipboard is set. As in the automation editor, the floating `Copy` also stores the
//!   copied hits in the drum editors' clipboard ([`step_seq::hit_clipboard`], shared with step mode); while it is set,
//!   a stationary long press and a right click on empty grid show one round `Paste` bubble at the finger,
//!   which dragging or another press hides. Its tap lands the earliest copied hit on the
//!   nearest snap unit under it, the others at their offsets with their pads and lengths, in one final edit
//!   ([`edit::paste`]); they are the selection (a lone one is remembered).
//! - **Remembered values (both drum modes).** Whenever exactly one hit becomes the selection (touched, tapped or pasted
//!   alone), the editor remembers length = max(its length, minimum length), its velocity and pan. Editing Velocity or
//!   Panning in the graph lane also updates them ([`Remembered`]: the step view's
//!   [`step_seq::StepSeqView::remembered`], which the host passes in, so both modes share it). As in the piano roll,
//!   where a new note is as long as the last one pressed, resized ones included, a move or resize that ends with one
//!   hit selected remembers that hit as it left it.
//! - **Menu actions.** The floating `Snap` quantizes the selection like the piano roll's. `Fill every / each 2 / each 4
//!   step` places hits every N steps with length N/4 beat here, velocity 255 ([`edit::fill`]; `Copy` is the piano
//!   roll's too, the copies right after the selection).
//! - **Graph lane** pages `Velocity`, `Panning`, `Pitch` (no `Repeat`); the per-step column background as in step mode;
//!   each bar spans start … start + length with no insets. A bar is at least 1 px wide, so a zero-length hit's shows.
//!
//! The graph lane's knob (as in the step view and the piano roll): its 2×2 U touch box wins wherever it overlaps the
//! list's first item or a row header cut off at the rows' bottom, however thin that header's visible part (see
//! [`graph_lane`]).
//!
//! The Snap button: free note-grid mode shows only the Snap icon button, 1.5 U × (1.5 U − 1 px), in the ruler row's
//! top-left cell; a tap toggles snap on/off, and a long press opens the snap list. It is the piano roll's look and snap
//! model ([`Snap`], Auto by default, on); the editor keeps it in memory. A gesture's end fits the loop to the hits
//! ([`crate::song::Clip::fit_loop`], as the piano roll's notes: their ends count, and it may shrink), unless it put
//! them back where they were; the zoom and scroll are kept per clip, as the rows' are.
//!
//! [`DrumGridView::show`] reads the song and returns a [`StepAction`] (the step view's: the row menu's and the ruler's
//! actions, and [`StepAction::Hits`] for this view's own edits); [`DrumGridView::apply`] makes the edit.

pub(crate) mod edit;

use std::collections::HashMap;

use eframe::egui::{self, Id, Pos2, Rangef, Rect, Response, Sense, Ui, vec2};

use crate::song::{Event, LaneKind, Song, TimeSig};
use crate::theme::{Rgba, channel_color};
use crate::ui::clips::{
    FloatingRow, GridView, Marquee, Selection, TimeAxis, hold_square, paint_marquee,
};
use crate::ui::edit_keys::{self, EditKey};
use crate::ui::graph_lane::{self, GraphLane, Prop};
use crate::ui::piano_roll::notes::{auto_grid, remove_selected};
use crate::ui::piano_roll::{Snap, SnapMode};
use crate::ui::popup::{self, PopupItem};
use crate::ui::region::{self, TimeRange};
use crate::ui::scroller::{self, Axis, Scroller, X, Y};
use crate::ui::step_seq::edit as steps;
use crate::ui::step_seq::{
    self, DrumPads, Fold, GRAPH_KNOB_X, Layout, MIXER_W, Overlay, Remembered, RowView, StepAction,
    TOP_H, hit_color, hit_tints, lane_split, pop_in,
};
use crate::ui::timeline::{self, KnobGesture, PlayheadKnob, RangeRuler, RangeSelect};
use crate::ui::widgets::{self, SnapTouch};
use crate::ui::{UiCtx, gesture, pinch_press, primary_down_live};
use edit::{BEAT, Place};

/// The Snap button's cell, 1.5 U wide.
const SNAP_W: f32 = 1.5;
/// The pop-in: a ← 0.6·a per frame (0 once under 0.01, [`pop_in`]); the scale law's constants.
const POP_RATE: f32 = 0.6;
const POP_GAIN: f32 = 1.0515;
const POP_W: f32 = 1.885;
/// The floating buttons over the selection (free note-grid mode).
const OPS: [&str; 3] = ["Copy", "Delete", "Snap"];

fn id() -> Id {
    Id::new("drum_grid")
}

/// Which clip a view belongs to: (channel, lane, clip id), the id `None` while the lane has no clip.
type ViewKey = (usize, usize, Option<u32>);

/// A clip's view: the rows' zoom and scroll, the time zoom k and the beat at the grid's left edge.
#[derive(Clone, Copy, Debug, PartialEq)]
pub(crate) struct ClipView {
    pub rows: RowView,
    pub k: f32,
    pub x: f64,
}

impl ClipView {
    /// A clip with no saved view, at subdivision `d` (`TDIV` byte 0): 4 U per beat, from beat 0.
    fn new(d: u8) -> Self {
        Self {
            rows: RowView::default(),
            k: edit::new_k(d),
            x: 0.0,
        }
    }
}

/// The pop-in's scale of a hit's width and height at `a`: 1.0515·sin(1.885·(1 − a)); 1 once it is over.
fn pop_scale(a: f32) -> f32 {
    if a <= 0.0 {
        1.0
    } else {
        POP_GAIN * (POP_W * (1.0 - a)).sin()
    }
}

#[derive(Default)]
enum Gesture {
    #[default]
    Idle,
    /// Down on empty grid, not yet a tap, a long press or a drag; `shown`: a selection was showing.
    Press {
        origin: Pos2,
        since: f64,
        shown: bool,
    },
    Pan,
    /// The marquee: its selection is live.
    Box(Marquee),
    Edit(Edit),
}

/// A move or resize in progress, recomputed from the hits as they were at its start.
struct Edit {
    resize: bool,
    /// The grabbed hit (in `orig`), whose start (move) or end (resize) snaps.
    anchor: usize,
    origin: Pos2,
    /// When it touched down (input time), whether a selection showed then, and which.
    since: f64,
    shown: bool,
    prior: Vec<bool>,
    orig: Vec<Event>,
    orig_sel: Vec<bool>,
    /// The hits that change.
    which: Vec<bool>,
    /// The pointer has left the calm zone; until then the hits stay.
    armed: bool,
    /// An ongoing edit went out, so the release sends the final one, even back where it started.
    reported: bool,
}

/// What the pointer is over: a selected hit's resize handle, or a hit.
#[derive(Clone, Copy, Debug, PartialEq)]
enum Target {
    Handle(usize),
    Hit(usize),
}

/// State of the free note-grid.
#[derive(Default)]
pub struct DrumGridView {
    /// The drum mixer's room between the headers and the grid, c·12 U, set by [`Self::show`] every frame.
    pub mixer_w: f32,
    /// The pointer is over an overlay; the transport's song tick, whether it plays, and whether Preview plays the
    /// shown clip. The host sets them each frame, as the step view's.
    pub covered: bool,
    pub playhead: u64,
    /// How far past `playhead` the playhead is drawn, as [`StepSeqView::lead`].
    pub lead: u64,
    pub playing: bool,
    /// REC is armed: the playhead disc shows the record icon. The host sets it each frame.
    pub armed: bool,
    pub previewing: bool,
    /// Per row, its pad's output peaks (L, R), as [`StepSeqView::levels`]. The host sets it each frame.
    pub levels: Vec<[f32; 2]>,
    /// The Snap button's state.
    pub snap: Snap,
    pub graph: GraphLane,
    /// The drum mixer's fold, this editor's own.
    pub(crate) fold: Fold,
    /// The current row (pad).
    pub(crate) current: usize,
    /// Each clip's zoom and scroll.
    pub(crate) views: HashMap<ViewKey, ClipView>,
    /// The clip the selection belongs to and its events as this view last left them: anything else is an outside
    /// change (another clip, an undo), which drops the selection and any gesture.
    pub(crate) seen: Option<(ViewKey, Vec<Event>)>,
    /// One flag per event of the clip.
    sel: Vec<bool>,
    /// The selection is quiet: kept, its handles and floating row hidden, as a move's release leaves it (module docs).
    quiet: bool,
    /// The current row's header is held (its pulse: the step view's).
    press: bool,
    last: Option<Layout>,
    overlay: Option<Overlay>,
    graph_drag: Option<(Vec<Event>, bool)>,
    /// The hits before the gesture whose ongoing [`StepAction::Hits`] edits [`Self::apply`] is making.
    hits_from: Option<Vec<Event>>,
    gesture: Gesture,
    region: RangeSelect,
    /// The playhead knob on the ruler.
    knob: PlayheadKnob,
    row: FloatingRow,
    snap_menu: bool,
    /// Hits a tap added, popping in, and the input time each first showed.
    pops: Vec<(Event, Option<f64>)>,
    /// The scroll and zoom controller.
    scroller: Scroller,
    /// The `Paste` button shows, for the press at this point (module docs).
    paste: Option<Pos2>,
    /// The grid cursor (content tick, row): the beat and row of the last touched hit, where the keymap's `paste` and
    /// `open` act ([`Self::keys`]); `None` before any: tick 0 on the current row.
    cursor: Option<(f64, usize)>,
}

impl DrumGridView {
    /// Another person's change came into the song: the selection follows its hits ([`crate::app::live::follow`]), and
    /// the change is no outside change that drops it.
    #[cfg(any(target_arch = "wasm32", test))]
    pub(crate) fn follow(&mut self, song: &Song) {
        let Some(((channel, lane, id), seen)) = &mut self.seen else {
            return;
        };
        let clip = (song.channels.get(*channel)).and_then(|c| c.lanes.get(*lane));
        let clip = clip.and_then(|l| l.clips.iter().find(|c| Some(c.id) == *id));
        let events = clip.map_or(&[][..], |c| &c.events);
        self.sel = crate::app::live::follow(seen, &self.sel, events, Event::pos);
        *seen = events.to_vec();
    }

    /// Paints the view over `ui`'s rect from the drum lane `lane` of `song.channels[channel]` and its clip `clip`, by id
    /// (an empty grid while the lane has no such clip). Draws nothing when there is no such drum lane. New hits take
    /// `remembered`, which selections and graph edits update.
    #[allow(clippy::too_many_arguments)]
    pub fn show(
        &mut self,
        ui: &mut Ui,
        cx: &UiCtx,
        song: &Song,
        channel: usize,
        (lane_index, clip_id): (usize, Option<u32>),
        pads: &dyn DrumPads,
        remembered: &mut Remembered,
    ) -> Option<StepAction> {
        self.last = None;
        let ch = song.channels.get(channel)?;
        let lane = (ch.lanes.get(lane_index)).filter(|l| l.kind == LaneKind::Drums)?;
        let clip = lane.clips.iter().find(|c| Some(c.id) == clip_id);
        let (colors, m) = (&cx.theme.colors, cx.m);
        let (text, bg) = (colors.text.to_color32(), colors.background.to_color32());
        let color = channel_color(ch.hue, cx.theme);
        let ink = color.to_color32();
        let rows = steps::row_count(lane, pads.count());
        let (bpb, sig) = (song.time_sig.bar_beats(), song.time_sig);
        let d = sig.subdivision;
        let loop_beats = clip.map_or(f64::from(bpb), |c| c.loop_len_beats);
        let (rect, graph_rect) = lane_split(ui.max_rect(), m, &self.graph);
        let (now, dt) = ui.input(|i| (i.time, i.stable_dt));
        if self.fold.step(now, dt) {
            ui.ctx().request_repaint();
        }
        self.mixer_w = self.fold.c * m.u(MIXER_W);

        // The selection follows the clip; an outside change drops it.
        let key = (channel, lane_index, clip.map(|c| c.id));
        let events = clip.map_or(&[][..], |c| &c.events);
        if (self.seen.as_ref()).is_none_or(|(k, e)| *k != key || e != events) {
            if self.seen.as_ref().is_some_and(|s| s.0 != key) {
                // Another clip's view: a fling of the last one stops, and its pops go.
                self.scroller = Scroller::default();
                self.pops.clear();
            }
            self.seen = Some((key, events.to_vec()));
            self.sel = vec![false; events.len()];
            self.gesture = Gesture::Idle;
            self.graph_drag = None;
            self.hits_from = None;
        }

        // The pops: one starts when its hit first shows; one whose hit never came is dropped.
        self.pops.retain_mut(|(e, at)| match at {
            None if events.contains(e) => {
                *at = Some(now);
                true
            }
            None => false,
            Some(t) => pop_in(POP_RATE, now - *t) > 0.0,
        });
        if !self.pops.is_empty() {
            ui.ctx().request_repaint();
        }

        // The clip's zoom and scroll, by the scroller: one finger scrolls the rows below the ruler and the time over
        // the grid columns (a touch on empty grid until it is a box; the headers take only taps), two zoom; the wheel
        // zooms the rows over the header column and the time over the ruler, and scrolls over the grid, where a
        // trackpad pinch zooms the time. The time scrolls over twice the loop.
        let u = m.u(1.0);
        let body = rect.with_min_y(rect.top() + m.u(TOP_H));
        let mut view = (self.views.get(&key).copied()).unwrap_or_else(|| ClipView::new(d));
        let laid = Layout::new(rect, rows, 1, m, self.mixer_w, view.rows);
        let k_zoom = |k: f32| 4.0 * f64::from(k) / f64::from(u * f32::from(d.max(1)));
        let mut axes = [
            Axis {
                pos: view.x,
                zoom: k_zoom(view.k.clamp(edit::K_RANGE[0], edit::K_RANGE[1])),
                range: [0.0, 2.0 * loop_beats],
                zoom_range: edit::K_RANGE.map(k_zoom),
                view: laid.grid().x_range(),
            },
            step_seq::rows_axis(view.rows, rows, body, u),
        ];
        let was = axes;
        let opts = scroller::Options {
            covered: self.covered,
            follow: false,
        };
        let zones = step_seq::zones(&laid);
        let report = (self.scroller).update(ui, id().with("scroll"), u, &zones, &mut axes, opts);
        if axes[X].zoom != was[X].zoom {
            view.k = (axes[X].zoom * f64::from(u * f32::from(d.max(1))) / 4.0) as f32;
        }
        if axes[X] != was[X] {
            view.x = axes[X].pos;
        }
        step_seq::take_rows(&mut view.rows, was[Y], axes[Y], u);
        let grid = ui.interact(laid.grid(), id().with("grid"), Sense::click_and_drag());
        crate::ui::focus::register(ui, grid.id, grid.rect, crate::ui::focus::Kind::Container);
        let (l, place) = self.place(rect, rows, m, d, &view);
        let auto = auto_grid(place.axis.beat_w, u, sig);
        let snap = self.snap.active(auto, sig);
        let grid_ticks = self.snap.grid(auto, sig);
        let min_beats = edit::min_len(snap) / BEAT;

        // The gestures.
        let before = self.sel.clone();
        let mut action = self.grid_input(ui, &grid, &place, events, snap, remembered);
        if self.sel != before {
            remember_lone(events, &self.sel, remembered, min_beats);
        }
        // A gesture's end: a lone hit is remembered as the gesture left it (a resize's new length; module docs).
        if let Some(StepAction::Hits {
            events,
            selected,
            final_edit: true,
            ..
        }) = &action
        {
            remember_lone(events, selected, remembered, min_beats);
        }
        self.views.insert(key, view);
        self.last = Some(l);
        self.current = self.current.min(rows.saturating_sub(1));
        let painter = ui.painter_at(rect);
        painter.rect_filled(rect, 0.0, bg);
        let snap_cell = Rect::from_min_size(rect.min, vec2(m.u(SNAP_W), m.u(TOP_H) - 1.0 / l.ppp));
        self.snap_button(ui, snap_cell, cx);

        // The rows scroll under the strip: everything below is clipped to their area, input included.
        let outer = ui.clip_rect();
        ui.set_clip_rect(outer.intersect(l.body()));
        let painter = ui.painter().clone();
        let prop = graph_lane::DRUM_PROPS[self.graph.item.min(graph_lane::drum_items(false) - 1)];
        self.paint_grid(&painter, &l, &place, events, (color, prop), sig, cx, now);
        #[cfg(any(target_arch = "wasm32", test))]
        crate::app::live::drum_grid(
            ui,
            cx,
            song,
            (&l, &place),
            key,
            (
                &self.sel,
                match &self.gesture {
                    Gesture::Edit(e) if e.reported => Some((&e.orig[..], &e.which[..])),
                    _ => None,
                },
            ),
            self.covered,
        );
        let heads = (&mut self.current, &mut self.press, &mut self.fold);
        if let Some((row, at)) =
            step_seq::row_headers(ui, &painter, cx, id(), &l, pads, &self.levels, ink, heads)
        {
            self.overlay = Some(Overlay::Menu { row, at });
        }
        // The marquee: a square growing while a touch is held on empty grid, then the box.
        let marquee = match self.gesture {
            Gesture::Press { origin, since, .. } => {
                ui.ctx().request_repaint();
                hold_square(origin, u, now - since)
            }
            Gesture::Box(marquee) if marquee.dragged => Some(marquee.drawn(u)),
            _ => None,
        };
        if let Some(rect) = marquee {
            paint_marquee(&painter, rect, cx);
        }
        let boxing = matches!(self.gesture, Gesture::Box(_));
        if let Some(edit) = self.floating_row(ui, cx, &l, &place, events, grid_ticks, boxing) {
            let (events, selected) = edit;
            remember_lone(&events, &selected, remembered, min_beats);
            action = Some(StepAction::Hits {
                events,
                selected,
                final_edit: true,
                quiet: false,
            });
        }
        if step_seq::hit_clipboard(ui.ctx()).is_empty() {
            self.paste = None;
        }
        if let Some(finger) = self.paste {
            let (rect, tap) = step_seq::paste_button(ui, cx, id().with("paste"), finger);
            let hits = step_seq::hit_clipboard(ui.ctx());
            if tap.is_some() {
                self.paste = None;
            }
            if tap == Some(true) && !hits.is_empty() {
                self.quiet = false;
                let (mut events, mut sel) = (events.to_vec(), self.sel.clone());
                let tick = place.axis.tick(rect.center().x);
                edit::paste(&mut events, &mut sel, &hits, tick, snap);
                remember_lone(&events, &sel, remembered, min_beats);
                action = Some(hits_edit(events, sel, true));
            }
        }
        if step_seq::add_drum(ui, &painter, id().with("add"), &l, pads.count(), text) {
            action = Some(StepAction::AddDrum);
        }
        ui.set_clip_rect(outer);
        let graph = match graph_rect {
            Some(strip) => {
                let lane = (strip, loop_beats * BEAT, prop, (color, sig));
                self.graph_lane(ui, cx, lane, &l, &place, events, remembered)
            }
            None => self.graph_drag.take().and_then(|(_, sent)| {
                sent.then(|| hits_edit(events.to_vec(), self.sel.clone(), true))
            }),
        };

        // The ruler over the grid, its range and region menu, and the playhead (the step view's).
        let ruler = l.swing_strip().with_min_x(l.grid_left());
        let ruler_painter = ui.painter_at(ruler);
        let axis = place.axis;
        timeline::ruler(
            &ruler_painter,
            ruler,
            ruler,
            axis.origin,
            axis.beat_w,
            sig,
            cx,
        );
        let origin = region::clip_origin(clip);
        let playhead = region::playhead_tick(clip, self.playhead, self.previewing);
        let at = RangeRuler {
            strip: ruler,
            axis,
            grid: snap,
            sig,
            ink,
            range: TimeRange::in_clip(song.play_loop, origin),
            playhead,
        };
        if report.scrolled[X] || report.zoomed[X] {
            self.region.view_moved();
        }
        let out = self.region.show(ui, id().with("region"), &at, false, cx);
        // A ruler press waiting for its long press is the scroller's too, until its range starts.
        if self.region.pending() {
            self.scroller.claim();
        } else if self.region.dragging() {
            self.scroller.yield_touch();
        }
        if let Some(range) = out.range {
            let play_loop = range.and_then(|r| r.song_loop(origin));
            action = Some(StepAction::PlayLoop(play_loop));
        }
        if let Some(tick) = out.playhead {
            let tick = region::seek_tick(clip, tick, self.previewing);
            action = Some(StepAction::SetPlayhead(tick));
        }
        if let Some((op, range)) = out.pick {
            let playhead = playhead.map_or(0, |p| p as u32);
            action = Some(StepAction::Region {
                op,
                range,
                playhead,
            });
        }
        // Cut at the header column's right edge, as the step view's: at beat 0 its left half shows in the t gap. While
        // the knob is dragged, the line and the disc follow the finger; a drop seeks within [the view's left edge, s + L].
        let line = painter.with_clip_rect(
            painter
                .clip_rect()
                .intersect(Rect::everything_right_of(l.column_right())),
        );
        let drawn = region::playhead_tick(clip, self.playhead + self.lead, self.previewing);
        let end = loop_beats * BEAT;
        let x = (self.knob).line_x(ui, cx.m, &at, end, axis.x(drawn.unwrap_or(0.0)));
        timeline::playhead_line(&line, l.grid(), x, cx);
        self.knob.armed = self.armed;
        let shown = timeline::knob_shown(self.playing, self.row.showing());
        match (self.knob).show(ui, id().with("knob"), ruler, x, shown, cx) {
            Some(KnobGesture::Tap) => action = Some(StepAction::Play),
            Some(KnobGesture::Drop(x)) => {
                let tick = self.knob.drop_tick(ui, &at, end, x) as u32;
                let tick = region::seek_tick(clip, tick, self.previewing);
                action = Some(StepAction::Seek(tick));
            }
            None => {}
        }

        self.snap_menu(ui, snap_cell, cx);
        let rect = ui.max_rect();
        let out = step_seq::overlays(
            &mut self.overlay,
            ui.ctx(),
            rect,
            cx,
            rows,
            pads,
            ink,
            false,
        )
        .or(graph)
        .or(action);
        // After the popups, which take Escape first.
        let keys = self.keys(ui, events, snap, remembered, min_beats);
        out.or(keys)
    }

    /// The keymap's editing actions while the grid holds the keyboard (`ui/edit_keys.rs`), as the piano roll's on the
    /// hits: `copy` fills the hit clipboard ([`step_seq::hit_clipboard`]), `cut` and `delete` remove the selection,
    /// `paste` lands the clipboard at the cursor's beat as the `Paste` button does (the hits keep their pads), `open`
    /// adds a hit at the cursor as a tap would, `close` deselects; `selectAll` is the piano roll's only. Each edit one
    /// final edit; each action hides the floating row and the `Paste` button. No editing action during a touch.
    #[allow(clippy::too_many_arguments)]
    fn keys(
        &mut self,
        ui: &Ui,
        events: &[Event],
        snap: Option<f64>,
        remembered: &mut Remembered,
        min_beats: f64,
    ) -> Option<StepAction> {
        let ctx = ui.ctx();
        if !matches!(self.gesture, Gesture::Idle) {
            return None;
        }
        let k = edit_keys::take(ctx)?;
        self.row.hide();
        // A key shows the selection's handles, as before a move left it quiet.
        (self.paste, self.quiet) = (None, false);
        ctx.request_repaint();
        let (mut events, mut sel) = (events.to_vec(), self.sel.clone());
        let chosen: Vec<Event> = (events.iter().zip(&sel))
            .filter(|(_, s)| **s)
            .map(|(e, _)| *e)
            .collect();
        if matches!(k, EditKey::Copy | EditKey::Cut) && !chosen.is_empty() {
            step_seq::set_hit_clipboard(ctx, chosen);
        }
        let (tick, row) = self.cursor.unwrap_or((0.0, self.current));
        match k {
            EditKey::Close => {
                self.sel.fill(false);
                return None;
            }
            EditKey::Copy | EditKey::SelectAll => return None,
            EditKey::Cut | EditKey::Delete => {
                if !remove_selected(&mut events, &mut sel) {
                    return None;
                }
            }
            EditKey::Paste => {
                let hits = step_seq::hit_clipboard(ctx);
                if hits.is_empty() {
                    return None;
                }
                edit::paste(&mut events, &mut sel, &hits, tick, snap);
                remember_lone(&events, &sel, remembered, min_beats);
            }
            EditKey::Open => {
                let hit = edit::new_hit(row as u8, tick, snap, remembered);
                // Only on an empty cell, as the step view's: a hit of the pad over the new one's start is one.
                let start = f64::from(hit.pos());
                let over = |(pad, pos, len): (u8, f64, f64)| {
                    usize::from(pad) == row && pos <= start && start < pos + len
                };
                if events.iter().filter_map(edit::hit).any(over) {
                    return None;
                }
                edit::insert(&mut events, &mut sel, hit);
                self.pops.push((hit, None));
                return Some(tapped(events, sel));
            }
        }
        Some(hits_edit(events, sel, true))
    }

    /// The layout and the hits' place for `view` (its scrolls as the scroller left them).
    fn place(
        &self,
        rect: Rect,
        rows: usize,
        m: &crate::metrics::Metrics,
        d: u8,
        view: &ClipView,
    ) -> (Layout, Place) {
        let l = Layout::new(rect, rows, 1, m, self.mixer_w, view.rows).scrolled(view.rows.scroll);
        let bw = edit::beat_w(m.u(1.0), d, view.k);
        let grid = l.grid();
        let axis = TimeAxis {
            origin: grid.left() - view.x as f32 * bw,
            beat_w: bw,
        };
        let place = Place {
            axis,
            row_top: l.row_top(),
            pitch: l.pitch,
            rows,
            u: m.u(1.0),
        };
        (l, place)
    }

    /// The handle or hit at `p`: a selected hit's handle first (none while the selection is quiet), then the topmost
    /// hit.
    fn target(&self, place: &Place, events: &[Event], p: Pos2) -> Option<Target> {
        let handle = (0..events.len()).rev().find(|&i| {
            !self.quiet
                && self.sel.get(i) == Some(&true)
                && place
                    .handle(&events[i])
                    .is_some_and(|h| h.center().distance(p) <= h.width() / 2.0)
        });
        handle
            .map(Target::Handle)
            .or_else(|| place.hit_at(events, p).map(Target::Hit))
    }

    /// Touches and drags on the grid (module docs). Returns the edit. A touch on empty grid is the scroller's until it
    /// is a box.
    fn grid_input(
        &mut self,
        ui: &Ui,
        grid: &Response,
        place: &Place,
        events: &[Event],
        snap: Option<f64>,
        remembered: &Remembered,
    ) -> Option<StepAction> {
        let (held, pressed, pos, now) = ui.input(|i| {
            let p = &i.pointer;
            // Unlike `latest_pos`, `interact_pos` is kept through a touch's lift.
            (
                p.primary_down(),
                p.primary_pressed(),
                p.interact_pos(),
                i.time,
            )
        });
        // A press held over a focus loss is up: it ends the gesture as a release, and starts nothing.
        let down = primary_down_live(ui, grid.id);
        let stale = held && !down || ui.input(|i| !i.focused || crate::ui::focus_lost(i));
        let pos = pos.filter(|_| !stale);
        let moved = |o: Pos2| gesture::left_calm(o, pos, place.u);
        let multi = ui.input(|i| i.multi_touch().is_some());
        let mut action = None;
        let clipboard = || !step_seq::hit_clipboard(ui.ctx()).is_empty();
        if multi || stale {
            self.paste = None;
        }
        if pressed && grid.is_pointer_button_down_on() {
            self.paste = None;
        }
        // A second finger makes a pinch, which only zooms: it ends a touch, putting back what it moved and the
        // selection its touch-down made.
        if multi && let Gesture::Edit(edit) = std::mem::replace(&mut self.gesture, Gesture::Pan) {
            self.sel = edit.prior;
            if edit.reported {
                action = Some(hits_edit(edit.orig, edit.orig_sel, true));
            }
        }
        let only = |i: usize, n: usize| (0..n).map(|j| j == i).collect::<Vec<_>>();
        self.quiet &= self.sel.contains(&true);

        // The keymap's `editorSelect` (Ctrl/Cmd held): a touch-down anywhere on the grid is a marquee at once.
        if pressed
            && down
            && !multi
            && grid.is_pointer_button_down_on()
            && matches!(self.gesture, Gesture::Idle)
            && crate::keymap::held(ui.ctx(), "editorSelect")
            && let Some(at) = pos
        {
            // As the selection modifier's on empty grid: the scroller lets the touch go.
            self.gesture = Gesture::Box(Marquee::at(at, true));
            self.scroller.yield_touch();
        }
        // Touch-down: on a hit or a handle the edit starts at once (hidden handles and row); on empty grid the touch
        // waits to be decided.
        if pressed
            && down
            && !multi
            && grid.is_pointer_button_down_on()
            && matches!(self.gesture, Gesture::Idle)
            && let Some(at) = pos
        {
            // A quiet selection is not showing: its frame takes no touch.
            let shown = self.sel.contains(&true) && !self.quiet;
            let frame = (events.iter().zip(&self.sel))
                .filter(|(_, s)| **s)
                .filter_map(|(e, _)| place.rect(e))
                .reduce(Rect::union);
            let inside = shown && frame.is_some_and(|f| f.contains(at));
            let prior = self.sel.clone();
            let target = self.target(place, events, at);
            if let Some(Target::Hit(i) | Target::Handle(i)) = target
                && let Some((pad, ..)) = edit::hit(&events[i])
            {
                self.cursor = Some((place.axis.tick(at.x).max(0.0), usize::from(pad)));
            }
            let anchor = match target {
                Some(Target::Handle(i)) => Some((true, i)),
                Some(Target::Hit(i)) if self.sel[i] => Some((false, i)),
                Some(Target::Hit(i)) if !inside => {
                    self.sel = only(i, events.len());
                    Some((false, i))
                }
                // Inside the showing selection's frame: it moves, the first selected hit snapping.
                _ if inside => self.sel.iter().position(|s| *s).map(|i| (false, i)),
                _ => None,
            };
            self.gesture = match anchor {
                Some((resize, anchor)) => Gesture::Edit(Edit {
                    resize,
                    anchor,
                    origin: at,
                    since: now,
                    shown,
                    prior,
                    orig: events.to_vec(),
                    orig_sel: self.sel.clone(),
                    which: self.sel.clone(),
                    armed: false,
                    reported: false,
                }),
                // A quiet selection counts as showing here: the tap only deselects.
                None => Gesture::Press {
                    origin: at,
                    since: now,
                    shown: self.sel.contains(&true),
                },
            };
            // Empty grid is the scroller's; a hit or a handle is the edit's.
            if matches!(self.gesture, Gesture::Press { .. }) {
                self.scroller.claim();
            }
        }

        // A right click on a hit makes it the selection; on empty grid it is `Paste`, while the clipboard is set.
        if grid.secondary_clicked()
            && !down
            && let Some(p) = grid.interact_pointer_pos()
        {
            match self.target(place, events, p) {
                Some(Target::Hit(i) | Target::Handle(i)) => {
                    (self.sel, self.quiet) = (only(i, events.len()), false);
                }
                None if clipboard() => self.paste = Some(p),
                None => {}
            }
        }

        // A press on empty grid held past a tap is a long press: the marquee, which the finger then drags (lifted
        // without a drag it is `Paste`, while the clipboard is set); with the selection modifier it drags at once.
        let shift = ui.input(|i| i.modifiers.shift);
        if let Gesture::Press { origin, since, .. } = self.gesture
            && (gesture::is_long(now - since) || shift)
        {
            self.gesture = Gesture::Box(Marquee::at(origin, shift));
            self.scroller.yield_touch();
        }
        let up = !down;
        match std::mem::take(&mut self.gesture) {
            Gesture::Press {
                origin,
                since,
                shown,
            } => {
                // Out of the scroller's calm zone on either axis it scrolls: no tap, no marquee.
                self.gesture = if self.scroller.left_calm().unwrap_or_else(|| moved(origin)) {
                    Gesture::Pan
                } else if up {
                    if stale {
                    } else if shown {
                        self.sel.fill(false);
                    } else if let Some(row) = place.row(origin.y) {
                        let hit =
                            edit::new_hit(row as u8, place.axis.tick(origin.x), snap, remembered);
                        let (mut events, mut sel) = (events.to_vec(), self.sel.clone());
                        edit::insert(&mut events, &mut sel, hit);
                        self.pops.push((hit, None));
                        action = Some(tapped(events, sel));
                    }
                    Gesture::Idle
                } else {
                    Gesture::Press {
                        origin,
                        since,
                        shown,
                    }
                };
            }
            Gesture::Pan if down => self.gesture = Gesture::Pan,
            Gesture::Box(mut marquee) => {
                // Live: what the box touches is the selection, every frame.
                marquee.follow(pos, place.u);
                (self.sel, self.quiet) = (place.in_box(events, marquee.region(grid.rect)), false);
                self.paste =
                    (!stale && !multi && !marquee.dragged && clipboard()).then_some(marquee.from);
                if !up {
                    self.gesture = Gesture::Box(marquee);
                }
            }
            Gesture::Edit(mut edit) => {
                edit.armed |= moved(edit.origin);
                if edit.armed
                    && let Some(p) = pos
                {
                    let d = p - edit.origin;
                    let dt = f64::from(d.x / place.axis.beat_w) * BEAT;
                    let (new, sel) = if edit.resize {
                        let new = edit::resized(&edit.orig, &edit.which, edit.anchor, dt, snap);
                        (new, edit.orig_sel.clone())
                    } else {
                        let dr = (d.y / place.pitch).round() as i32;
                        let (orig, sel) = (&edit.orig, &edit.orig_sel);
                        edit::moved(
                            orig,
                            sel,
                            &edit.which,
                            edit.anchor,
                            dt,
                            dr,
                            place.rows,
                            snap,
                        )
                    };
                    if new != events {
                        edit.reported = true;
                        action = Some(StepAction::Hits {
                            events: new,
                            selected: sel,
                            final_edit: false,
                            quiet: false,
                        });
                    }
                }
                if !up {
                    self.gesture = Gesture::Edit(edit);
                    return action;
                }
                // A move's release leaves its selection quiet; a tap, a long press or a resize shows it.
                self.quiet = edit.armed && !edit.resize;
                if edit.reported {
                    // This frame's own move, or the hits as the last one left them.
                    action = Some(match action {
                        Some(StepAction::Hits {
                            events, selected, ..
                        }) => hits_edit(events, selected, true),
                        _ => hits_edit(events.to_vec(), self.sel.clone(), true),
                    });
                } else if !edit.armed && !stale && !gesture::is_long(now - edit.since) {
                    // A tap while a selection showed only deselects; else the hit alone (out of a quiet selection
                    // too) is the selection, which a long press shows as the touch-down left it.
                    self.sel = if edit.shown {
                        vec![false; events.len()]
                    } else {
                        only(edit.anchor, events.len())
                    };
                }
            }
            Gesture::Pan | Gesture::Idle => {}
        }
        action
    }
    /// The rows' bands, the marks over them and the hits, with the selected hits' handles (module docs).
    #[allow(clippy::too_many_arguments)]
    fn paint_grid(
        &mut self,
        painter: &egui::Painter,
        l: &Layout,
        place: &Place,
        events: &[Event],
        (color, prop): (Rgba, Prop),
        sig: TimeSig,
        cx: &UiCtx,
        now: f64,
    ) {
        let colors = &cx.theme.colors;
        let (text, bg) = (colors.text.to_color32(), colors.background.to_color32());
        let px = 1.0 / l.ppp;
        let grid = l.grid();
        let axis = place.axis;
        // A `controlFill` band per row, 1 px in from its top and bottom, from beat 0 or the grid's left edge.
        let left = l.snap(axis.x(0.0)).max(grid.left());
        let fill = colors.control_fill.to_color32();
        for row in l.visible_rows() {
            let span = l.span(l.row_top(), l.pitch, row);
            let band = Rect::from_x_y_ranges(left..=grid.right(), span.min + px..=span.max);
            painter.rect_filled(band, 0.0, fill);
        }
        // `background` lines over the rows: w px from ⌊w/2⌋ px left of each mark.
        let ys = Rangef::new(l.row_top(), l.row_top() + place.rows as f32 * l.pitch);
        let (t0, t1) = (axis.tick(grid.left()), axis.tick(grid.right()));
        for (tick, w) in edit::marks(t0, t1, axis.beat_w, l.u, sig, edit::MARK_GAP) {
            let x = l.snap(axis.x(tick)) - (w / 2.0).floor() * px;
            painter.rect_filled(Rect::from_x_y_ranges(x..=x + w * px, ys), 0.0, bg);
        }

        // The hits, in clip order: a 1 px `background` frame, then the hit colour; a tap's new hit pops in.
        let tints = hit_tints(colors, color, prop);
        for (i, e) in events.iter().enumerate() {
            let (Some(r), Some(value)) = (place.rect(e), prop.get(e)) else {
                continue;
            };
            if !r.expand(px).intersects(grid) {
                continue;
            }
            let span = l.span(
                l.row_top(),
                l.pitch,
                usize::from(edit::hit(e).map_or(0, |h| h.0)),
            );
            let mut hit = Rect::from_x_y_ranges(
                l.snap(r.left())..=l.snap(r.right()),
                span.min + 2.0 * px..=span.max - px,
            );
            let a = (self.pops.iter().rev().find(|p| p.0 == *e))
                .and_then(|p| p.1)
                .map_or(0.0, |t0| pop_in(POP_RATE, now - t0));
            if a > 0.0 {
                hit = Rect::from_center_size(hit.center(), hit.size() * pop_scale(a));
            }
            let selected = self.sel.get(i) == Some(&true);
            painter.rect_filled(hit.expand(px), 0.0, bg);
            painter.rect_filled(hit, 0.0, hit_color(&tints, prop, value, selected));
        }
        // A touch on a hit hides the handles until it ends, and a quiet selection has none.
        let touched = self.quiet || matches!(self.gesture, Gesture::Edit(_));
        for (e, _) in (events.iter().zip(&self.sel)).filter(|(_, s)| **s && !touched) {
            if let Some(h) = place.handle(e) {
                painter.circle_filled(h.center(), h.width() / 2.0, text);
            }
        }
    }

    /// The Snap button in `cell` ([`widgets::snap_button`]): a tap toggles snap; a long press or a right click opens
    /// the snap menu.
    fn snap_button(&mut self, ui: &mut Ui, cell: Rect, cx: &UiCtx) {
        match widgets::snap_button(ui, id().with("snap"), cell, self.snap.on, cx) {
            Some(SnapTouch::Menu) => self.snap_menu = true,
            Some(SnapTouch::Toggle) => self.snap.on = !self.snap.on,
            None => {}
        }
    }

    /// The snap menu, while open: one column, the current mode highlighted; a pick sets it and turns snap on.
    fn snap_menu(&mut self, ui: &Ui, cell: Rect, cx: &UiCtx) {
        if !self.snap_menu {
            return;
        }
        let ctx = ui.ctx();
        let items: Vec<PopupItem<SnapMode>> = (SnapMode::ALL.iter().enumerate())
            .map(|(row, &mode)| PopupItem {
                current: mode == self.snap.mode,
                ..PopupItem::new(mode, 0, row, mode.label())
            })
            .collect();
        let bounds = ctx.content_rect();
        let result = popup::show(
            ctx,
            id().with("snap menu"),
            cell.center(),
            bounds,
            &items,
            cx,
        );
        if let Some(mode) = result.selected {
            self.snap = Snap { on: true, mode };
        }
        if result.close {
            self.snap_menu = false;
            ctx.request_repaint();
        }
    }

    /// `Copy`, `Delete` and `Snap` over the selection, in a [`FloatingRow`] (waiting for a marquee); `grid` is the
    /// snap mode's unit, on or off. Returns the hits and selection a button left.
    #[allow(clippy::too_many_arguments)]
    fn floating_row(
        &mut self,
        ui: &mut Ui,
        cx: &UiCtx,
        l: &Layout,
        place: &Place,
        events: &[Event],
        grid: f64,
        boxing: bool,
    ) -> Option<(Vec<Event>, Vec<bool>)> {
        let chosen: Vec<&Event> = (events.iter().zip(&self.sel))
            .filter(|(_, s)| **s)
            .map(|(e, _)| e)
            .collect();
        let handles: Vec<Rect> = chosen.iter().filter_map(|e| place.handle(e)).collect();
        let pads = || chosen.iter().filter_map(|e| edit::hit(e)).map(|h| h.0);
        // A quiet selection has no row, which a release on it brings back neither.
        let sel = (chosen.iter().filter_map(|e| place.rect(e)))
            .reduce(Rect::union)
            .filter(|_| !boxing && !self.quiet && self.paste.is_none())
            .map(|rect| Selection {
                rect,
                one_row: pads().min() == pads().max(),
                handles: &handles,
            });
        let view = GridView {
            grid: l.grid(),
            map: [place.axis.origin, l.row_top(), place.axis.beat_w, l.pitch],
        };
        self.row.covered = self.covered;
        let (i, _) = self.row.show(ui, "grid_button", sel, view, &OPS, cx)?;
        let (mut events, mut sel) = (events.to_vec(), self.sel.clone());
        match i {
            0 => {
                step_seq::set_hit_clipboard(ui.ctx(), chosen.iter().map(|&&e| e).collect());
                edit::copy_selected(&mut events, &mut sel, grid);
            }
            1 => {
                remove_selected(&mut events, &mut sel);
            }
            _ => edit::quantize_selected(&mut events, &mut sel, grid),
        }
        ui.ctx().request_repaint();
        Some((events, sel))
    }

    /// The graph lane in `strip` (the step view's frame and laws): `Velocity`, `Panning`, `Pitch` in the header column,
    /// the mixer's room left to the host (the spacer), and under the grid a column per step (alternating per beat, all
    /// `controlFill` past the loop's `loop_ticks`) and the current row's hits, each bar over the hit's length, which a
    /// drag paints.
    #[allow(clippy::too_many_arguments)]
    fn graph_lane(
        &mut self,
        ui: &mut Ui,
        cx: &UiCtx,
        (strip, loop_ticks, prop, (color, sig)): (Rect, f64, Prop, (Rgba, TimeSig)),
        l: &Layout,
        place: &Place,
        events: &[Event],
        remembered: &mut Remembered,
    ) -> Option<StepAction> {
        let (m, colors) = (cx.m, &cx.theme.colors);
        let ink = color.to_color32();
        let labels = &graph_lane::DRUM_LABELS[..graph_lane::drum_items(false)];
        let item = self.graph.item;
        let list = id().with("graph list");
        let content = graph_lane::frame(
            ui,
            cx,
            list,
            strip,
            l.header_w,
            labels,
            ink,
            &mut self.graph.item,
        );
        // The grid drew this frame in the old page's colours.
        if self.graph.item != item {
            ui.ctx().request_repaint();
        }
        let values = content.with_min_x(l.grid_left());
        let axis = place.axis;
        let pad = u8::try_from(self.current).ok()?;

        let painter = ui.painter_at(values);
        let px = 1.0 / l.ppp;
        let step = sig.step_ticks();
        let first = (axis.tick(values.left()) / step).floor().max(0.0) as u64;
        let last = (axis.tick(values.right()) / step).ceil().max(0.0) as u64;
        for s in first..last {
            let t = s as f64 * step;
            let even = (s / u64::from(sig.steps_per_beat())).is_multiple_of(2);
            let fill = if even || t > loop_ticks {
                colors.control_fill
            } else {
                colors.module_fill
            };
            let x = l.snap(axis.x(t))..=l.snap(axis.x(t + step)) - px;
            painter.rect_filled(
                Rect::from_x_y_ranges(x, values.y_range()),
                0.0,
                fill.to_color32(),
            );
        }
        graph_lane::drum_band(&painter, values, prop, cx);
        let tints = hit_tints(colors, color, prop);
        for pass in [false, true] {
            for (e, _) in (events.iter().zip(&self.sel)).filter(|(_, s)| **s == pass) {
                let (Some((p, start, len)), Some(v)) = (edit::hit(e), prop.get(e)) else {
                    continue;
                };
                if p != pad {
                    continue;
                }
                let x0 = l.snap(axis.x(start));
                let x = Rangef::new(x0, l.snap(axis.x(start + len)).max(x0 + px));
                let fill = hit_color(&tints, prop, v, pass);
                graph_lane::drum_bar(&painter, prop, v, x, values, fill, m);
            }
        }

        // The paint: at touch-down and at every move, the eligible hits whose extent holds x take the value under y.
        let response = ui.interact(values, id().with("graph"), Sense::drag());
        let live = primary_down_live(ui, response.id);
        // A second finger makes a pinch: it ends the drag, putting back what it painted.
        let pinch = pinch_press(ui, response.id);
        // What it painted (if anything) put back: a touch that changed nothing.
        if pinch && let Some((orig, _)) = self.graph_drag.take() {
            return Some(StepAction::GraphCancel(orig));
        }
        if response.drag_started() && live && !pinch {
            self.graph_drag = Some((events.to_vec(), false));
        }
        if response.dragged()
            && live
            && let Some(p) = response.interact_pointer_pos()
            && self.graph_drag.is_some()
        {
            let v = if matches!(prop, Prop::Pan | Prop::Pitch) {
                graph_lane::v_centred(values, p.y, m)
            } else {
                graph_lane::v_at(values, p.y)
            };
            let value = prop.stored(v);
            let mut new = events.to_vec();
            if edit::paint(&mut new, &self.sel, pad, prop, axis.tick(p.x), value) {
                remembered.graph(prop, value);
                if let Some((_, sent)) = self.graph_drag.as_mut() {
                    *sent = true;
                }
                return Some(hits_edit(new, self.sel.clone(), false));
            }
        }
        if (response.drag_stopped() || !live)
            && let Some((orig, sent)) = self.graph_drag.take()
        {
            // A gesture that changes nothing clears the selection, makes no undo step and re-syncs (its own undo).
            if events == orig.as_slice() {
                self.sel.fill(false);
                return Some(StepAction::GraphCancel(orig));
            }
            return sent.then(|| hits_edit(events.to_vec(), self.sel.clone(), true));
        }
        None
    }

    /// The graph lane's knob for the view `rect`, as the step view's: at 1.4 U from its left, on the lane's top or
    /// the view's bottom edge. The host calls it after the drum mixer's strips.
    pub fn graph_knob(&mut self, ui: &mut Ui, cx: &UiCtx, rect: Rect) {
        let m = cx.m;
        let below = (rect, rect.top() + m.u(TOP_H));
        let x = rect.left() + m.u(GRAPH_KNOB_X);
        (self.graph).knob_below(ui, cx, id().with("graph knob"), below, x);
    }

    /// Closes the popups over the view (the row menu and its prompts, the snap menu, the ruler's region menu and `+`,
    /// the `Paste` button): the editor stopped showing. A gesture under way ends with it, a graph-lane drag too, and its
    /// edit's start ([`Self::apply`]'s `hits_from`) with it: back on screen, a held finger drags nothing. Returns whether
    /// that ended a drag whose ongoing edits went out: the host closes their undo step (a final edit), as the drag's
    /// release would.
    pub(crate) fn close_popups(&mut self) -> bool {
        (self.overlay, self.snap_menu, self.paste) = (None, false, None);
        self.hits_from = None;
        let gesture = std::mem::take(&mut self.gesture);
        self.region.close_popups();
        let graph = self.graph_drag.take().is_some_and(|(_, sent)| sent);
        graph || matches!(gesture, Gesture::Edit(Edit { reported: true, .. }))
    }

    /// This editor as another channel's starts it: a fresh view keeping the fold, every clip's view and the snap.
    pub(crate) fn kept(self) -> Self {
        Self {
            fold: self.fold,
            views: self.views,
            snap: self.snap,
            ..Self::default()
        }
    }

    /// Follows every clip's view and the selection's clip to `key(old)` after channels or tracks moved; a view whose
    /// clip is gone goes.
    pub(crate) fn rekey(&mut self, key: &dyn Fn(ViewKey) -> Option<ViewKey>) {
        let views = std::mem::take(&mut self.views).into_iter();
        self.views = views.filter_map(|(k, v)| key(k).map(|k| (k, v))).collect();
        self.seen = self.seen.take().and_then(|(k, e)| key(k).map(|k| (k, e)));
    }

    /// The last frame's top of row 0 and row pitch, in points, for the drum mixer's strips; `None` when the view drew
    /// nothing.
    pub(crate) fn row_band(&self) -> Option<(f32, f32)> {
        self.last.map(|l| (l.top - l.scroll, l.pitch))
    }

    /// Feeds a MIDI key to the view: while `Set MIDI Note` waits, routes it to that pad and closes the prompt (as
    /// [`step_seq::StepSeqView::midi_key`]).
    pub fn midi_key(&mut self, note: u8) -> Option<StepAction> {
        let Some(Overlay::LearnMidi(row)) = self.overlay else {
            return None;
        };
        self.overlay = None;
        Some(StepAction::SetMidiNote {
            pad: row as u8,
            note: Some(note),
        })
    }

    /// Makes `action`'s edit on clip `clip` of the drum lane `lane` of `song.channels[channel]`: this view's hits, the
    /// row menu's `Fill` (the free note-grid's) and `Clear`, `Clone` and `Delete` (the rows of every drum clip of the
    /// channel), and the ruler's region items and play loop. Without that clip (by id) the clip edits do nothing: no
    /// clip is ever made here. `Rename` and `SetMidiNote` change only the kit (the host's).
    pub fn apply(
        &mut self,
        song: &mut Song,
        channel: usize,
        (lane_index, clip_id): (usize, Option<u32>),
        action: &StepAction,
    ) {
        match *action {
            StepAction::PlayLoop(play_loop) => song.play_loop = play_loop,
            StepAction::Region { op, .. } if !op.keeps_loop() => song.play_loop = None,
            _ => {}
        }
        let (bpb, sig) = (song.time_sig.bar_beats(), song.time_sig);
        let Some(ch) = song.channels.get_mut(channel) else {
            return;
        };
        match *action {
            StepAction::Clone { pad } => {
                steps::clone_row(&mut ch.lanes, pad);
                self.current = usize::from(pad) + 1;
                return;
            }
            StepAction::Delete { pad } => {
                steps::delete_row(&mut ch.lanes, pad);
                if self.current > usize::from(pad) {
                    self.current -= 1;
                }
                return;
            }
            _ => {}
        }
        let Some(lane) = (ch.lanes.get_mut(lane_index)).filter(|l| l.kind == LaneKind::Drums)
        else {
            return;
        };
        let Some(clip) = lane.clips.iter_mut().find(|c| Some(c.id) == clip_id) else {
            return;
        };
        let mut sel = None;
        match action {
            StepAction::Hits {
                events,
                selected,
                final_edit,
                ..
            } => {
                if !*final_edit {
                    self.hits_from.get_or_insert_with(|| clip.events.clone());
                }
                clip.events.clone_from(events);
                // Only the gesture's end fits the loop, and only when it changed the hits: a drag out past the end
                // and back, or one a pinch puts back, leaves it as it was.
                if *final_edit && self.hits_from.take().as_ref() != Some(events) {
                    clip.fit_loop(bpb);
                }
                sel = Some(selected.clone());
            }
            StepAction::GraphCancel(orig) => {
                self.hits_from = None;
                clip.events.clone_from(orig);
                sel = Some(self.sel.clone());
            }
            &StepAction::Fill { pad, every } => {
                edit::fill(clip, pad, every, sig);
                clip.fit_loop(bpb);
            }
            &StepAction::Clear { pad } => {
                steps::clear(clip, pad);
                clip.fit_loop(bpb);
            }
            &StepAction::Region {
                op,
                range,
                playhead,
            } => region::edit_clip(clip, op, range, playhead, bpb),
            _ => return,
        }
        self.sel = sel.unwrap_or_else(|| vec![false; clip.events.len()]);
        self.seen = Some(((channel, lane_index, Some(clip.id)), clip.events.clone()));
    }
}

/// A [`StepAction::Hits`] edit.
fn hits_edit(events: Vec<Event>, selected: Vec<bool>, final_edit: bool) -> StepAction {
    StepAction::Hits {
        events,
        selected,
        final_edit,
        quiet: false,
    }
}

/// The [`StepAction::Hits`] edit of a tap's new hit: no re-sync.
fn tapped(events: Vec<Event>, selected: Vec<bool>) -> StepAction {
    StepAction::Hits {
        events,
        selected,
        final_edit: true,
        quiet: true,
    }
}

/// When `sel` holds exactly one hit of `events`, it is remembered, at least `min_beats` long.
fn remember_lone(events: &[Event], sel: &[bool], remembered: &mut Remembered, min_beats: f64) {
    let mut chosen = (events.iter().zip(sel)).filter(|(_, s)| **s);
    if let (Some((e, _)), None) = (chosen.next(), chosen.next()) {
        remembered.select(e, min_beats);
    }
}

#[cfg(test)]
mod tests;
