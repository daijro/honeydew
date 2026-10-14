//! The editors' time ruler: the playlist's, the piano roll's, the automation and audio editors' and both drum modes'.
//! Every ruler follows the drum step grid's law, t = ceil(U/16) px, h the ruler's height:
//! 1. a `background` fill;
//! 2. a `menuFill` baseline, t tall, at bottom − 2t, across the full width;
//! 3. the range, if any ([`RangeSelect`]);
//! 4. `menuFill` ticks with their bottoms at bottom − 3t: beat-level ticks 2t wide and 0.3h tall, step- and bar-level
//!    ticks t wide and 0.2h tall, spaced by the mark law ([`next`], [`spacings`], [`marks`]);
//! 5. bar badges on the top-level marks: a `text` disc of radius 6t on the bar line at mid-height, the 1-based bar number
//!    on it in font step 0, `menuFill`, skipped when the line is within 6t of the marks' left edge (the playlist's
//!    header column is not part of it).
//!
//! The law gives "every other bar" (3, 5, 7 …) at the playlist's and the piano roll's opening zooms and every bar at
//! the automation and audio editors' 4 U a beat. A tick is centred on its mark; the baseline's top is at bottom − 2t;
//! in Next, B/n and d/n are whole-number divisions; the text box rule max(text width + r, 0.75 U), for text centred on
//! the disc, never changes what is drawn.
//!
//! It scrolls and zooms: the caller passes tick 0's x and the beat width. Every editor's playhead is the shared line
//! ([`playhead_line`]) under a light disc holding a play triangle, the playhead knob ([`PlayheadKnob`], [`disc`]). A
//! tap on the ruler sets the playhead, and a range picked on it is the song's play loop, whose `+` button opens the
//! region menu ([`RangeSelect`]).

use eframe::egui::{
    self, Align2, Color32, Id, Painter, Pos2, Rangef, Rect, Sense, Shape, Vec2, pos2,
};

use crate::icons::{IconId, draw_icon};
use crate::metrics::{FontSize, Metrics};
use crate::song::{PPQ, TimeSig};
use crate::ui::buttons::filled_circle_button;
use crate::ui::clips::TimeAxis;
use crate::ui::piano_roll::notes::snap_round;
use crate::ui::region::{self, RegionOp, TimeRange};
use crate::ui::{UiCtx, gesture, pinch_press, popup, primary_down_live, widgets};

// Sizes in U.
pub const RULER_H: f32 = 1.5;
/// The marks are never closer than this, in U.
const MARK_U: f32 = 0.6;
/// Tick heights, in ruler heights; the badge's radius, in t.
const BEAT_TICK_H: f32 = 0.3;
const TICK_H: f32 = 0.2;
const BADGE_T: f32 = 6.0;
/// The playhead disc's square is this share of the ruler's height, and the disc's radius this many device px short of
/// half of it.
const DISC_SIDE: f32 = 1.35;
const DISC_INSET_PX: f32 = 3.0;
/// The playhead icon's size factor per U of the disc's diameter.
const PLAYHEAD_ICON: f32 = 0.4;
/// The line's shadow: `background` at this alpha.
const SHADOW_ALPHA: f32 = 0.3;
/// The range's edge handles, centred on each edge.
const HANDLE_W: f32 = 1.0;
/// The `+` button under the released edge.
const PLUS_D: f32 = 1.25;
/// How long a dropped playhead knob stays put waiting for the output to report the seek, in seconds.
const DROP_HOLD_S: f64 = 0.25;

/// A vertical line `px` device pixels wide, starting on the pixel column that contains `x`.
pub fn vline_px(painter: &Painter, x: f32, y: Rangef, px: f32, color: Color32, m: &Metrics) {
    let left = m.floor_px(x);
    painter.rect_filled(
        Rect::from_x_y_ranges(left..=left + px / m.ppp(), y),
        0.0,
        color,
    );
}

/// A mark's level: fine (step style), mid (beat style), or top (bar style with a badge).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Level {
    Fine,
    Mid,
    Top,
}

/// The mark spacing's step function Next(x) in beats, `d` steps a beat and `b` beats a bar (B = b·d), with
/// n = max(1, llround(x·d)) steps: for n ≥ d, 2n if n ≥ B or B/n is even, else 3n if B/n is divisible by 3, else a bar;
/// for n < d, 2n if d/n is even, else 3n if d/n is divisible by 3, else a beat. Always more than `x`'s n.
pub fn next(x: f64, d: i64, b: i64) -> f64 {
    let bar = b * d;
    let n = ((x * d as f64).round() as i64).max(1);
    let (whole, unit) = if n >= d { (bar, bar) } else { (d, d) };
    let q = whole / n;
    let steps = if (n >= d && n >= bar) || q % 2 == 0 {
        2 * n
    } else if q % 3 == 0 {
        3 * n
    } else {
        unit
    };
    steps as f64 / d as f64
}

/// The fine, mid and top spacings (s0, s1, s2) in beats, for marks at least `m` beats apart: from 1/d, 1 beat and b
/// beats, Next until at least m, clamp(d, 2, 4)·m and clamp(d, 2, 4)·clamp(b, 2, 4)·m.
pub fn spacings(m: f64, sig: TimeSig) -> [f64; 3] {
    let (d, b) = (i64::from(sig.steps_per_beat()), i64::from(sig.bar_beats()));
    // A bound past any zoom stops the walk (and NaN never starts it).
    let grow = |mut x: f64, bound: f64| {
        while x < bound.min(1e9) {
            x = next(x, d, b);
        }
        x
    };
    let (cd, cb) = (d.clamp(2, 4) as f64, b.clamp(2, 4) as f64);
    [
        grow(1.0 / d as f64, m),
        grow(1.0, cd * m),
        grow(b as f64, cd * cb * m),
    ]
}

/// The marks on steps `from..=to` (none before step 0): every s0, top-level on multiples of s2, mid-level on multiples
/// of s1, `s` the spacings in beats at `d` steps a beat.
pub fn marks(from: i64, to: i64, s: [f64; 3], d: u32) -> impl Iterator<Item = (i64, Level)> {
    let [fine, mid, top] = s.map(|x| ((x * f64::from(d)).round() as i64).max(1));
    let first = (from.max(0) + fine - 1) / fine * fine;
    (first..=to).step_by(fine as usize).map(move |j| {
        let level = if j % top == 0 {
            Level::Top
        } else if j % mid == 0 {
            Level::Mid
        } else {
            Level::Fine
        };
        (j, level)
    })
}

/// Paints the ruler over `rect` (module docs): its fill and baseline, then its ticks and badges clipped to `marks` (all
/// of `rect`, or the part right of a header column). Tick 0 is at `x0` (left of `marks` once scrolled), a beat is
/// `beat_w` wide and `sig` is the song's `TDIV`.
pub fn ruler(
    painter: &Painter,
    rect: Rect,
    marks: Rect,
    x0: f32,
    beat_w: f32,
    sig: TimeSig,
    cx: &UiCtx,
) {
    let (c, t) = (&cx.theme.colors, cx.m.t());
    painter.rect_filled(rect, 0.0, c.background.to_color32());
    let base = Rect::from_x_y_ranges(rect.x_range(), rect.bottom() - 2.0 * t..=rect.bottom() - t);
    painter.rect_filled(base, 0.0, c.menu_fill.to_color32());
    ruler_marks(painter, rect, marks, x0, beat_w, sig, cx);
}

/// The ruler's ticks and bar badges ([`ruler`]), clipped to `marks`.
fn ruler_marks(
    painter: &Painter,
    rect: Rect,
    marks: Rect,
    x0: f32,
    beat_w: f32,
    sig: TimeSig,
    cx: &UiCtx,
) {
    let (c, m) = (&cx.theme.colors, cx.m);
    if beat_w.is_nan() || beat_w <= 0.0 {
        return;
    }
    let painter = painter.with_clip_rect(painter.clip_rect().intersect(marks));
    let (ink, text) = (c.menu_fill.to_color32(), c.text.to_color32());
    let t = m.t();
    let d = sig.steps_per_beat();
    let step_w = beat_w / d as f32;
    let s = spacings(f64::from(m.u(MARK_U) / beat_w), sig);
    let radius = BADGE_T * t;
    let at = |x: f32| f64::from((x - x0) / step_w);
    let (from, to) = (
        at(marks.left()).floor() as i64,
        at(marks.right() + radius).ceil() as i64,
    );
    let bottom = rect.bottom() - 3.0 * t;
    let bar = i64::from(d * sig.bar_beats());
    for (j, level) in self::marks(from, to, s, d) {
        let x = x0 + j as f32 * step_w;
        let (w, h) = match level {
            Level::Mid => (2.0 * t, BEAT_TICK_H),
            Level::Fine | Level::Top => (t, TICK_H),
        };
        let tick = Rect::from_x_y_ranges(
            x - w / 2.0..=x + w / 2.0,
            bottom - h * rect.height()..=bottom,
        );
        painter.rect_filled(tick, 0.0, ink);
        if level == Level::Top && x - marks.left() >= radius {
            let center = pos2(x, rect.center().y);
            painter.circle_filled(center, radius, text);
            let font = m.font_id(FontSize::Pt6);
            let number = (j / bar + 1).to_string();
            painter.text(center, Align2::CENTER_CENTER, number, font, ink);
        }
    }
}

/// The playhead disc of a ruler `ruler_h` points tall: its square's side and the disc's radius, in points. The square is
/// 1.35 × the ruler's height, centred on the playhead x and the ruler's mid-height; the radius is side/2 − 3 px, so the
/// disc overhangs the ruler into the top of the grid.
pub fn disc(ruler_h: f32, m: &Metrics) -> (f32, f32) {
    let side = DISC_SIDE * ruler_h;
    (side, side / 2.0 - DISC_INSET_PX / m.ppp())
}

/// The playhead line, the same in every editor grid: at `x`, from the top of `grid` to its bottom, first a `background`
/// shadow at alpha 0.3, 2(t + 1) px wide, then a `text` line 2t px wide, both centred on the pixel nearest `x`
/// (t = ceil(U/16) px). It is drawn whenever `x` lies within the grid's width, stopped or playing; `painter` clips it
/// (the playlist's lets its left half show in the gap after the header column).
pub fn playhead_line(painter: &Painter, grid: Rect, x: f32, cx: &UiCtx) {
    painter.extend(playhead_line_shapes(grid, x, cx));
}

/// The shapes of [`playhead_line`], none while `x` is off the grid: for a paint slot reserved under other shapes.
pub fn playhead_line_shapes(grid: Rect, x: f32, cx: &UiCtx) -> Vec<Shape> {
    if !grid.x_range().contains(x) {
        return Vec::new();
    }
    let (c, m) = (&cx.theme.colors, cx.m);
    let (ppp, t) = (m.ppp(), m.t() * m.ppp());
    let x = (x * ppp).round();
    let band = |half: f32, color: Color32| {
        let xs = (x - half) / ppp..=(x + half) / ppp;
        Shape::rect_filled(Rect::from_x_y_ranges(xs, grid.y_range()), 0.0, color)
    };
    let shadow = c.background.to_color32().gamma_multiply(SHADOW_ALPHA);
    vec![band(t + 1.0, shadow), band(t, c.text.to_color32())]
}

/// The playhead disc's fade target in every editor: shown only while the transport is stopped and no floating row
/// (`menu`) shows, so while playing only the line shows. The ruler's keyboard focus is no condition here: no ruler
/// takes the focus (`ui::focus`).
pub fn knob_shown(playing: bool, menu: bool) -> bool {
    !playing && !menu
}

/// The playhead's disc of radius `r` at `center`: `text`, with `icon` in `menuFill` on it, both at `alpha`.
fn paint_disc(painter: &Painter, center: Pos2, r: f32, alpha: f32, icon: IconId, cx: &UiCtx) {
    let (c, m) = (&cx.theme.colors, cx.m);
    painter.circle_filled(center, r, c.text.to_color32().gamma_multiply(alpha));
    let ink = c.menu_fill.to_color32().gamma_multiply(alpha);
    let factor = PLAYHEAD_ICON * 2.0 * r / m.u(1.0);
    let square = Rect::from_center_size(center, Vec2::splat(2.0 * r));
    draw_icon(painter, square, icon, ink, factor, m);
}

/// The rulers' time range and its region menu, the same in the playlist, the piano roll and the drum editor:
/// - a tap (released within 0.3 s) sets the playhead to the grid line nearest it (the host stops a playing transport
///   instead);
/// - a long press (0.3 s) starts a range on the grid line nearest the press, and the finger drags the other edge
///   along the grid; moving more than 0.5 U up or down first cancels it, moving sideways does not. A press on an
///   edge's 1 U handle drags that edge at once;
/// - the released edge gets a `+` button below the ruler, which opens the region menu.
///
/// The range itself is the song's play loop, which the host keeps; this keeps only the gesture. It is drawn on the
/// ruler alone: a `controlFill` rect (`moduleFill` while an edge is dragged) from the ruler's top, 3 t short of its
/// bottom, the ticks and numbers over it, and a bar in the channel colour on each edge, 4 t wide (12 t while
/// dragged), t = ceil(U/16) px. The `+` (font 3) is drawn in 12 pt, the fourth of the app's sizes, and stays while the
/// range has the edge it sits on; a press held over a focus loss taps nothing.
#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub struct RangeSelect {
    gesture: Gesture,
    /// The edge (a tick) whose `+` button shows.
    plus: Option<u32>,
    /// The region menu is open, centred here.
    menu: Option<Pos2>,
}

#[derive(Clone, Copy, Debug, Default, PartialEq)]
enum Gesture {
    #[default]
    Idle,
    /// Pressed at `origin`, egui time `since`: a tap or, once held long enough, a range.
    Press { origin: Pos2, since: f64 },
    /// An edge being dragged, now at `moving`, the other one at `fixed`; the range was `was` before the drag, which a
    /// pinch puts back.
    Edge {
        fixed: u32,
        moving: u32,
        was: Option<TimeRange>,
    },
    /// Cancelled: nothing until the button is up.
    Cancelled,
}

/// A ruler that takes a range, for [`RangeSelect::show`].
#[derive(Clone, Copy, Debug)]
pub struct RangeRuler {
    /// The ruler strip the gestures are made on (its marks right of any header column).
    pub strip: Rect,
    pub axis: TimeAxis,
    /// The grid the playhead and the edges snap to, in ticks; `None`: whole ticks.
    pub grid: Option<f64>,
    /// The song's `TDIV`, for the marks over the range.
    pub sig: TimeSig,
    /// The edges' colour: the channel's.
    pub ink: Color32,
    /// The range, in the axis's ticks.
    pub range: Option<TimeRange>,
    /// The playhead in the axis's ticks, for the menu's playhead items; `None` when it is not on this ruler.
    pub playhead: Option<f64>,
}

/// What a ruler's gestures did this frame.
#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub struct RulerOutput {
    /// The range is now this (`None`: cleared), in the axis's ticks.
    pub range: Option<Option<TimeRange>>,
    /// A tap: the playhead goes to this tick.
    pub playhead: Option<u32>,
    /// A region-menu item picked, with the range it is for (`Deselect` is reported as the range cleared).
    pub pick: Option<(RegionOp, TimeRange)>,
}

impl RangeSelect {
    /// A press waits for its long press (or its release, a tap): meanwhile its drag is the editor's scroller's.
    pub fn pending(&self) -> bool {
        matches!(self.gesture, Gesture::Press { .. })
    }

    /// An edge is dragged: the touch is the ruler's.
    pub fn dragging(&self) -> bool {
        matches!(self.gesture, Gesture::Edge { .. })
    }

    /// The view scrolled or zoomed sideways: the `+` hides and a press waiting for its long press is cancelled.
    pub fn view_moved(&mut self) {
        self.plus = None;
        if self.pending() {
            self.gesture = Gesture::Cancelled;
        }
    }

    /// Closes the region menu and the range's `+` button: their editor stopped showing. The range stays (the song's).
    pub fn close_popups(&mut self) {
        (self.plus, self.menu) = (None, None);
    }

    /// The gestures on `at.strip`, the range over the ruler (paint the ruler first), its `+` button and the region menu
    /// (`scale_tools`: the piano roll's items too). The host stores [`RulerOutput::range`] as the song's play loop.
    pub fn show(
        &mut self,
        ui: &mut egui::Ui,
        id: Id,
        at: &RangeRuler,
        scale_tools: bool,
        cx: &UiCtx,
    ) -> RulerOutput {
        let items = |range, playhead| region::items(range, playhead, scale_tools);
        self.show_with(ui, id, at, &items, cx)
    }

    /// [`Self::show`] with the region menu's `items` for a range and the playhead (the audio clip editor's
    /// [`region::audio_items`]).
    pub fn show_with(
        &mut self,
        ui: &mut egui::Ui,
        id: Id,
        at: &RangeRuler,
        items: &dyn Fn(TimeRange, Option<f64>) -> Vec<popup::PopupItem<RegionOp>>,
        cx: &UiCtx,
    ) -> RulerOutput {
        let m = cx.m;
        let mut out = RulerOutput::default();
        let response = ui.interact(at.strip, id, Sense::click_and_drag());
        // Every frame, for their latches: a press held over a focus loss is up, and a second finger is a pinch.
        let down = primary_down_live(ui, id);
        let pinch = pinch_press(ui, id);
        let (now, pressed, released, pos) = ui.input(|i| {
            let p = &i.pointer;
            let pos = p.interact_pos().filter(|_| down || !p.primary_down());
            (i.time, p.primary_pressed(), p.primary_released(), pos)
        });
        let tick = |x: f32| snap_round(at.axis.tick(x).max(0.0), at.grid) as u32;
        let x = |tick: u32| at.axis.x(tick.into());

        if pressed
            && response.is_pointer_button_down_on()
            && let Some(p) = pos
        {
            // On an edge's handle, the nearer edge.
            let handle = at.range.and_then(|r| {
                [(r.start, r.end), (r.end, r.start)]
                    .into_iter()
                    .map(|(edge, other)| ((p.x - x(edge)).abs(), edge, other))
                    .filter(|&(d, ..)| d <= m.u(HANDLE_W) / 2.0)
                    .min_by(|a, b| a.0.total_cmp(&b.0))
            });
            self.gesture = match handle {
                Some((_, moving, fixed)) => Gesture::Edge {
                    fixed,
                    moving,
                    was: at.range,
                },
                None => Gesture::Press {
                    origin: p,
                    since: now,
                },
            };
        }
        if pinch && self.gesture != Gesture::Idle {
            // A half-dragged range goes back to what it was.
            if let Gesture::Edge { was, .. } = self.gesture
                && was != at.range
            {
                out.range = Some(was);
            }
            self.gesture = Gesture::Cancelled;
        }
        if let Gesture::Press { origin, since } = self.gesture {
            // The shared law with the headers' vertical calm zone.
            let held = gesture::is_long(now - since);
            let left = pos.is_some_and(|p| gesture::vertical(p - origin) > m.u(gesture::CALM_U));
            if left && !held {
                self.gesture = Gesture::Cancelled;
            } else if held {
                let fixed = tick(origin.x);
                self.gesture = Gesture::Edge {
                    fixed,
                    moving: fixed,
                    was: at.range,
                };
                self.plus = None;
                if at.range.is_some() {
                    out.range = Some(None);
                }
            } else if !down {
                if released {
                    out.playhead = Some(tick(pos.unwrap_or(origin).x));
                }
                self.gesture = Gesture::Idle;
            } else {
                ui.ctx()
                    .request_repaint_after_secs((gesture::LONG_PRESS_S - (now - since)) as f32);
            }
        }
        if let Gesture::Edge { fixed, moving, .. } = &mut self.gesture {
            if let Some(p) = pos {
                *moving = tick(p.x);
            }
            let range = (*moving != *fixed).then(|| TimeRange {
                start: (*fixed).min(*moving),
                end: (*fixed).max(*moving),
            });
            if range != at.range {
                out.range = Some(range);
            }
            if !down {
                self.plus = range.map(|_| *moving);
                self.gesture = Gesture::Idle;
            }
        }
        if !down && self.gesture == Gesture::Cancelled {
            self.gesture = Gesture::Idle;
        }

        let range = out.range.unwrap_or(at.range);
        self.paint(ui, at, range, cx);
        if let Some(r) = range
            && let Some(edge) = self.plus.filter(|&e| e == r.start || e == r.end)
            && at.strip.x_range().contains(x(edge))
        {
            let (c, d) = (&cx.theme.colors, m.u(PLUS_D));
            let center = pos2(x(edge), at.strip.bottom() + d / 2.0);
            let button =
                filled_circle_button(ui, id.with("plus"), center, d, c.menu_fill.to_color32());
            let font = m.font_id(FontSize::Pt12);
            ui.painter().text(
                center,
                Align2::CENTER_CENTER,
                "+",
                font,
                c.text.to_color32(),
            );
            if button.clicked() {
                self.menu = Some(center);
            }
        }
        let Some(range) = range else {
            (self.plus, self.menu) = (None, None);
            return out;
        };
        let Some(center) = self.menu else {
            return out;
        };
        let ctx = ui.ctx();
        let items = items(range, at.playhead);
        let result = popup::show(ctx, id.with("menu"), center, ctx.content_rect(), &items, cx);
        if result.close {
            self.menu = None;
            ctx.request_repaint();
        }
        match result.selected {
            Some(RegionOp::Deselect) => out.range = Some(None),
            Some(op) => out.pick = Some((op, range)),
            None => {}
        }
        out
    }

    /// The range over the ruler (type docs).
    fn paint(&self, ui: &egui::Ui, at: &RangeRuler, range: Option<TimeRange>, cx: &UiCtx) {
        let Some(r) = range else { return };
        let (c, m) = (&cx.theme.colors, cx.m);
        let t = m.t();
        let ys = at.strip.top()..=at.strip.bottom() - 3.0 * t;
        let x = |tick: u32| at.axis.x(tick.into());
        let rect = Rect::from_x_y_ranges(x(r.start)..=x(r.end), ys.clone());
        let moving = match self.gesture {
            Gesture::Edge { moving, .. } => Some(moving),
            _ => None,
        };
        let fill = if moving.is_some() {
            c.module_fill
        } else {
            c.control_fill
        };
        let painter = ui.painter_at(at.strip);
        painter.rect_filled(rect, 0.0, fill.to_color32());
        let over = painter.with_clip_rect(painter.clip_rect().intersect(rect));
        let (x0, beat_w) = (at.axis.origin, at.axis.beat_w);
        ruler_marks(&over, at.strip, at.strip, x0, beat_w, at.sig, cx);
        for edge in [r.start, r.end] {
            let w = if moving == Some(edge) { 12.0 } else { 4.0 } * t;
            let bar = Rect::from_x_y_ranges(x(edge) - w / 2.0..=x(edge) + w / 2.0, ys.clone());
            painter.rect_filled(bar, 0.0, at.ink);
        }
    }
}

/// One frame of the playhead disc's fade: 20 % of the way to `target`, snapping onto it within 0.001.
pub fn fade(alpha: f32, target: f32) -> f32 {
    let next = alpha + 0.2 * (target - alpha);
    if (target - next).abs() < 0.001 {
        target
    } else {
        next
    }
}

/// The beat a dragged playhead knob shows and its drop seeks to, as a song tick: the finger's `tick` clamped between
/// the view-left tick `left` and the song end `end`, then round(tick/unit)·unit on the `grid` (whole ticks without one,
/// Snap off), never past the end.
pub fn knob_seek(tick: f64, grid: Option<f64>, left: f64, end: f64) -> u64 {
    let end = end.max(0.0);
    snap_round(tick.max(left).min(end), grid).clamp(0.0, end) as u64
}

impl RangeRuler {
    /// The tick a playhead knob dropped at `x` on this ruler seeks to, in the axis's ticks: [`knob_seek`] from the
    /// ruler's left edge to `end`, on its grid. The drum editors pass s + L; the piano roll and the automation and
    /// audio editors pass L − 1 tick in content ticks (their playhead is folded into the loop, where L is its start
    /// again).
    pub fn knob_tick(&self, x: f32, end: f64) -> u64 {
        knob_seek(
            self.axis.tick(x),
            self.grid,
            self.axis.tick(self.strip.left()),
            end,
        )
    }
}

/// What the playhead knob did this frame.
#[derive(Clone, Copy, Debug, PartialEq)]
pub enum KnobGesture {
    /// A tap: start playback.
    Tap,
    /// A drag released at this x: the host seeks to the beat under it ([`knob_seek`]).
    Drop(f32),
}

/// The playhead knob: the light disc with a play icon on the ruler.
/// - A tap (released within 0.3 s, never out of the 0.5 U calm zone: the shared law, [`gesture`]) is
///   [`KnobGesture::Tap`].
/// - Moving more than 0.5 U sideways makes the press a drag: the disc follows the finger (the host maps
///   [`PlayheadKnob::dragging`] to the snapped beat), and the release is [`KnobGesture::Drop`]. A press that moved
///   only up or down does nothing.
/// - It grows while held and wobbles once let go by the knobs' own law ([`widgets::pulse`]), and fades ([`fade`])
///   toward the host's target, tracking the live playhead as it fades: the drum editor's rule in every editor
///   ([`knob_shown`]), the playlist's only while on the grid too. Fading out it takes no new press.
///
/// It is drawn as the editors' playhead disc ([`disc`]): on its ruler's middle line, its square 1.35 × the ruler's
/// height, a `text` disc of radius side/2 − 3 px and its icon in `menuFill`, both at the fade's alpha; the icon is REC
/// (icon 5) while record is armed, else play (icon 3). Neither drawn nor touched while the playhead is off the ruler.
///
/// The disc's square is its hit area; a press held over a focus loss does nothing.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct PlayheadKnob {
    /// Record is armed: the disc shows icon 5 (REC) in place of 3 (play). The host sets it each frame.
    pub armed: bool,
    alpha: f32,
    press: Option<KnobPress>,
    /// The last drop, a tick on the host's axis, and when: the playhead shows there until the output reports it
    /// ([`Self::line_x`]).
    pub(crate) dropped: Option<(u64, f64)>,
}

#[derive(Clone, Copy, Debug, PartialEq)]
struct KnobPress {
    origin: Pos2,
    since: f64,
    moved: bool,
    drag: bool,
}

impl Default for PlayheadKnob {
    /// Shown from the first frame.
    fn default() -> Self {
        Self {
            armed: false,
            alpha: 1.0,
            press: None,
            dropped: None,
        }
    }
}

impl PlayheadKnob {
    /// While the knob is dragged: the finger's x this frame. The host maps it to a beat ([`knob_seek`]) and passes
    /// that beat's x to [`Self::show`], so that the line and the disc both follow the finger on the snap grid.
    pub fn dragging(&self, ui: &egui::Ui, m: &Metrics) -> Option<f32> {
        let press = self.press?;
        // `interact_pos`, not `latest_pos`: a touch's lift clears the latter in its release frame (`PointerGone`).
        let x = ui.input(|i| i.pointer.interact_pos())?.x;
        // From the frame the finger first goes more than 0.5 U sideways, as [`Self::show`] decides it.
        (press.drag || (x - press.origin.x).abs() > m.u(gesture::CALM_U)).then_some(x)
    }

    /// The x the host draws its playhead (line and disc) at on `at`, `end` being the drop's upper clamp
    /// ([`RangeRuler::knob_tick`]):
    /// - while the knob is dragged, the tick its drop would seek to, so that the line and the disc follow the finger;
    /// - after a drop ([`Self::drop_tick`]), that tick, until the output reports a tick in its beat (`at.playhead`) or
    ///   for a quarter second: the output reports the seek a frame or more later, and meanwhile the disc must not
    ///   flash back;
    /// - else `live`.
    pub fn line_x(
        &mut self,
        ui: &egui::Ui,
        m: &Metrics,
        at: &RangeRuler,
        end: f64,
        live: f32,
    ) -> f32 {
        let now = ui.input(|i| i.time);
        if let Some((tick, since)) = self.dropped {
            let beat = tick as f64..tick as f64 + f64::from(PPQ);
            if at.playhead.is_some_and(|t| beat.contains(&t)) || now - since >= DROP_HOLD_S {
                self.dropped = None;
            } else {
                ui.ctx()
                    .request_repaint_after_secs((DROP_HOLD_S - (now - since)) as f32);
            }
        }
        let dragged = self.dragging(ui, m).map(|x| at.knob_tick(x, end));
        (dragged.or(self.dropped.map(|d| d.0))).map_or(live, |t| at.axis.x(t as f64))
    }

    /// The tick a drop at `x` ([`KnobGesture::Drop`]) on `at` seeks to, up to `end`; [`Self::line_x`] shows the playhead
    /// there until the output reports it.
    pub fn drop_tick(&mut self, ui: &egui::Ui, at: &RangeRuler, end: f64, x: f32) -> u64 {
        let tick = at.knob_tick(x, end);
        self.dropped = Some((tick, ui.input(|i| i.time)));
        tick
    }

    /// The knob on `ruler` (the time ruler: [`disc`] gives its square and radius) at the live playhead `x`, given every
    /// frame (while it is dragged, the host's mapping of [`Self::dragging`]); `shown` is the fade's target. The step
    /// grid passes "stopped" to show it only then.
    pub fn show(
        &mut self,
        ui: &mut egui::Ui,
        id: Id,
        ruler: Rect,
        x: f32,
        shown: bool,
        cx: &UiCtx,
    ) -> Option<KnobGesture> {
        let m = cx.m;
        let (d, r) = disc(ruler.height(), m);
        let center = pos2(x, ruler.center().y);
        let on = ruler.x_range().contains(x);
        // Every frame, for their latches: a press held over a focus loss is up, and a second finger is a pinch's,
        // which ends the press (the shared gesture law): no tap, no drop.
        let down = primary_down_live(ui, id);
        if pinch_press(ui, id) {
            self.press = None;
        }
        let (now, pressed, released, pos) = ui.input(|i| {
            let p = &i.pointer;
            let pos = p.interact_pos();
            (i.time, p.primary_pressed(), p.primary_released(), pos)
        });
        let live = on && (shown || self.press.is_some());
        let hit = Rect::from_center_size(center, Vec2::splat(d)).with_max_y(ruler.bottom());
        let response = live.then(|| ui.interact(hit, id, Sense::click_and_drag()));
        if pressed
            && response.is_some_and(|r| r.is_pointer_button_down_on())
            && let Some(origin) = pos
        {
            self.press = Some(KnobPress {
                origin,
                since: now,
                moved: false,
                drag: false,
            });
        }
        let mut out = None;
        if let Some(press) = &mut self.press {
            if let Some(p) = pos.filter(|_| down) {
                press.moved |= gesture::left_calm(press.origin, Some(p), m.u(1.0));
                press.drag |= (p.x - press.origin.x).abs() > m.u(gesture::CALM_U);
            }
            if !down {
                if released {
                    out = if press.drag {
                        pos.map(|p| KnobGesture::Drop(p.x))
                    } else if !press.moved && !gesture::is_long(now - press.since) {
                        Some(KnobGesture::Tap)
                    } else {
                        None
                    };
                }
                self.press = None;
            }
        }

        let target = if shown || self.press.is_some() {
            1.0
        } else {
            0.0
        };
        self.alpha = fade(self.alpha, target);
        let scale = widgets::pulse(ui, id, self.press.is_some(), d * m.ppp());
        if self.alpha != target {
            ui.ctx().request_repaint();
        }
        if self.alpha > 0.0 && on {
            let icon = if self.armed {
                IconId::Record
            } else {
                IconId::Play
            };
            paint_disc(ui.painter(), center, r * scale, self.alpha, icon, cx);
        }
        out
    }
}

#[cfg(test)]
mod tests {
    mod range {
        use eframe::egui::{Color32, Event, Id, Modifiers, PointerButton, Pos2, Rect, Ui, pos2};

        use super::super::{RangeRuler, RangeSelect, RulerOutput};
        use crate::theme::theme_by_id;
        use crate::ui::UiCtx;
        use crate::ui::clips::TimeAxis;
        use crate::ui::offscreen::Offscreen;
        use crate::ui::region::{RegionOp, TimeRange};

        /// A ruler across the top: tick 0 at x 48, 64 points a beat, a grid of beats. U = 32.
        const STRIP: Rect = Rect {
            min: pos2(0.0, 0.0),
            max: pos2(1170.0, 48.0),
        };
        const AXIS: TimeAxis = TimeAxis {
            origin: 48.0,
            beat_w: 64.0,
        };
        const INK: Color32 = Color32::from_rgb(200, 40, 90);

        fn beat(b: f32) -> Pos2 {
            pos2(48.0 + b * 64.0, 24.0)
        }

        /// The host's side: it keeps the range, as the song keeps its play loop, and the playhead.
        struct Rig {
            screen: Offscreen,
            select: RangeSelect,
            range: Option<TimeRange>,
            playhead: Option<f64>,
            /// Every output with something in it, and the shapes of the last frame.
            outs: Vec<RulerOutput>,
            shapes: Vec<eframe::egui::epaint::ClippedShape>,
        }

        impl Rig {
            fn new() -> Self {
                Self {
                    screen: Offscreen::new(),
                    select: RangeSelect::default(),
                    range: None,
                    playhead: Some(0.0),
                    outs: Vec::new(),
                    shapes: Vec::new(),
                }
            }

            fn frame_after(&mut self, dt: f64, events: Vec<Event>) {
                self.screen.time += dt - 1.0 / 60.0;
                let Self {
                    screen,
                    select,
                    range,
                    playhead,
                    ..
                } = self;
                let ruler = RangeRuler {
                    strip: STRIP,
                    axis: AXIS,
                    grid: Some(128.0),
                    sig: crate::song::TimeSig::default(),
                    ink: INK,
                    range: *range,
                    playhead: *playhead,
                };
                let mut out = RulerOutput::default();
                self.shapes = screen.frame(events, &mut |ui: &mut Ui, cx: &UiCtx| {
                    out = select.show(ui, Id::new("ruler"), &ruler, false, cx);
                });
                if let Some(r) = out.range {
                    self.range = r;
                }
                if out != RulerOutput::default() {
                    self.outs.push(out);
                }
            }

            fn frame(&mut self, events: Vec<Event>) {
                self.frame_after(1.0 / 60.0, events);
            }

            fn press(&mut self, pos: Pos2, pressed: bool) {
                self.frame(vec![
                    Event::PointerMoved(pos),
                    Event::PointerButton {
                        pos,
                        button: PointerButton::Primary,
                        pressed,
                        modifiers: Modifiers::NONE,
                    },
                ]);
            }

            fn tap(&mut self, pos: Pos2) {
                self.frame(vec![Event::PointerMoved(pos)]);
                self.press(pos, true);
                self.press(pos, false);
            }

            /// A long press at `from` (held 0.3 s), dragged to `to` and released there.
            fn select(&mut self, from: Pos2, to: Pos2) {
                self.frame(vec![Event::PointerMoved(from)]);
                self.press(from, true);
                self.frame_after(0.31, vec![]);
                self.frame(vec![Event::PointerMoved(from + (to - from) / 2.0)]);
                self.frame(vec![Event::PointerMoved(to)]);
                self.press(to, false);
                // A new popup or button lays itself out first.
                self.frame(vec![]);
            }

            /// Drags from `from` to `to` at once (no long press) and releases.
            fn drag(&mut self, from: Pos2, to: Pos2) {
                self.frame(vec![Event::PointerMoved(from)]);
                self.press(from, true);
                self.frame(vec![Event::PointerMoved(to)]);
                self.press(to, false);
                self.frame(vec![]);
            }

            fn text(&self, label: &str) -> Option<Pos2> {
                self.shapes.iter().find_map(|s| match &s.shape {
                    eframe::egui::Shape::Text(t) if t.galley.text() == label => {
                        Some(t.visual_bounding_rect().center())
                    }
                    _ => None,
                })
            }

            fn rects(&self, color: Color32) -> Vec<Rect> {
                (self.shapes.iter())
                    .filter_map(|s| match &s.shape {
                        eframe::egui::Shape::Rect(r) if r.fill == color => Some(r.rect),
                        _ => None,
                    })
                    .collect()
            }

            fn pick(&mut self, label: &str) {
                let at = self.text(label).expect(label);
                self.tap(at);
            }

            /// The `+` button under the released edge.
            fn plus(&self) -> Option<Pos2> {
                self.text("+")
            }

            fn taps(&self) -> Vec<u32> {
                self.outs.iter().filter_map(|o| o.playhead).collect()
            }
        }

        const BEATS_1_3: TimeRange = TimeRange {
            start: 128,
            end: 384,
        };

        #[test]
        fn a_tap_sets_the_playhead_to_the_nearest_grid_line() {
            let mut rig = Rig::new();
            rig.tap(beat(2.4));
            rig.tap(beat(2.6));
            assert_eq!(rig.taps(), [256, 384]);
            assert_eq!(rig.range, None);
            // Held past 0.3 s, it is no tap.
            rig.frame(vec![Event::PointerMoved(beat(1.0))]);
            rig.press(beat(1.0), true);
            rig.frame_after(0.31, vec![]);
            rig.press(beat(1.0), false);
            assert_eq!(rig.taps(), [256, 384]);
        }

        #[test]
        fn a_long_press_starts_a_range_on_the_grid_and_the_finger_drags_its_other_edge() {
            let mut rig = Rig::new();
            rig.select(beat(1.2), beat(3.4));
            assert_eq!(rig.range, Some(BEATS_1_3));
            // Back past the press: the other way round.
            rig.select(beat(3.4), beat(0.9));
            assert_eq!(
                rig.range,
                Some(TimeRange {
                    start: 128,
                    end: 384
                })
            );
            assert!(rig.taps().is_empty());
            // Released on its start: length 0, no range.
            rig.select(beat(2.0), beat(2.1));
            assert_eq!(rig.range, None);
        }

        #[test]
        fn moving_up_or_down_before_the_long_press_cancels_it_but_sideways_does_not() {
            let mut rig = Rig::new();
            rig.frame(vec![Event::PointerMoved(beat(1.0))]);
            rig.press(beat(1.0), true);
            rig.frame(vec![Event::PointerMoved(
                beat(1.0) + eframe::egui::vec2(0.0, 17.0),
            )]);
            rig.frame_after(0.31, vec![Event::PointerMoved(beat(3.0))]);
            rig.press(beat(3.0), false);
            assert_eq!((rig.range, rig.taps()), (None, vec![]));
            // Sideways first: the range still starts where the press was.
            rig.frame(vec![Event::PointerMoved(beat(1.0))]);
            rig.press(beat(1.0), true);
            rig.frame(vec![Event::PointerMoved(beat(1.3))]);
            rig.frame_after(0.31, vec![Event::PointerMoved(beat(3.0))]);
            rig.press(beat(3.0), false);
            assert_eq!(rig.range, Some(BEATS_1_3));
        }

        #[test]
        fn an_edges_handle_drags_that_edge_at_once() {
            let mut rig = Rig::new();
            rig.select(beat(1.0), beat(3.0));
            // Within half a U of the end.
            rig.drag(beat(3.0) + eframe::egui::vec2(12.0, 0.0), beat(5.0));
            assert_eq!(
                rig.range,
                Some(TimeRange {
                    start: 128,
                    end: 640
                })
            );
            rig.drag(beat(1.0) - eframe::egui::vec2(12.0, 0.0), beat(2.0));
            assert_eq!(
                rig.range,
                Some(TimeRange {
                    start: 256,
                    end: 640
                })
            );
            assert!(rig.taps().is_empty());
        }

        #[test]
        fn the_released_edges_plus_button_opens_the_region_menu() {
            let mut rig = Rig::new();
            rig.select(beat(1.0), beat(3.0));
            assert_eq!(rig.text("Deselect"), None, "no menu yet");
            let plus = rig.plus().expect("the + button");
            // Centred on the released edge, its top on the ruler's bottom: a 1.25 U `menuFill` disc.
            assert!((plus.x - beat(3.0).x).abs() < 1.0, "{plus:?}");
            assert!((plus.y - (48.0 + 20.0)).abs() < 3.0, "{plus:?}");
            let menu_fill = theme_by_id(0).colors.menu_fill.to_color32();
            let disc = rig.shapes.iter().find_map(|s| match &s.shape {
                eframe::egui::Shape::Circle(c) if c.fill == menu_fill => Some((c.center, c.radius)),
                _ => None,
            });
            assert_eq!(disc, Some((pos2(beat(3.0).x, 68.0), 20.0)));
            rig.tap(plus);
            rig.frame(vec![]);
            assert!(rig.text("Deselect").is_some());
            assert_eq!(rig.text("Snap to Scale"), None);
            rig.pick("Delete space");
            assert_eq!(
                rig.outs.last().and_then(|o| o.pick),
                Some((RegionOp::DeleteSpace, BEATS_1_3))
            );
            rig.frame(vec![]);
            assert_eq!(rig.text("Deselect"), None, "the menu closed");
        }

        #[test]
        fn deselect_clears_the_range_and_its_button() {
            let mut rig = Rig::new();
            rig.select(beat(1.0), beat(3.0));
            rig.tap(rig.plus().unwrap());
            rig.frame(vec![]);
            rig.pick("Deselect");
            rig.frame(vec![]);
            assert_eq!(rig.range, None);
            assert_eq!(rig.plus(), None);
            assert!(rig.outs.iter().all(|o| o.pick.is_none()));
        }

        #[test]
        fn the_playhead_inside_the_range_or_off_the_ruler_disables_its_items() {
            for playhead in [Some(200.0), None] {
                let mut rig = Rig::new();
                rig.playhead = playhead;
                rig.select(beat(1.0), beat(3.0));
                rig.tap(rig.plus().unwrap());
                rig.frame(vec![]);
                rig.pick("Copy to playhead");
                rig.pick("Move to playhead");
                assert!(rig.outs.iter().all(|o| o.pick.is_none()), "{playhead:?}");
            }
            let mut rig = Rig::new();
            rig.playhead = Some(384.0);
            rig.select(beat(1.0), beat(3.0));
            rig.tap(rig.plus().unwrap());
            rig.frame(vec![]);
            rig.pick("Copy to playhead");
            assert_eq!(
                rig.outs.last().and_then(|o| o.pick),
                Some((RegionOp::CopyToPlayhead, BEATS_1_3))
            );
        }

        #[test]
        fn the_range_is_a_rect_on_the_ruler_with_an_edge_bar_on_each_side() {
            let theme = theme_by_id(0);
            let (control, module) = (
                theme.colors.control_fill.to_color32(),
                theme.colors.module_fill.to_color32(),
            );
            let mut rig = Rig::new();
            rig.frame(vec![Event::PointerMoved(beat(1.0))]);
            rig.press(beat(1.0), true);
            rig.frame_after(0.31, vec![]);
            rig.frame(vec![Event::PointerMoved(beat(3.0))]);
            // While dragged: moduleFill, the dragged edge 12 t (t = 2 points at U 32) wide, the other 4 t.
            let (x1, x3) = (beat(1.0).x, beat(3.0).x);
            assert!(
                rig.rects(module)
                    .contains(&Rect::from_x_y_ranges(x1..=x3, 0.0..=42.0))
            );
            let mut bars = rig.rects(INK);
            bars.sort_by(|a, b| a.left().total_cmp(&b.left()));
            assert_eq!(
                bars,
                [
                    Rect::from_x_y_ranges(x1 - 4.0..=x1 + 4.0, 0.0..=42.0),
                    Rect::from_x_y_ranges(x3 - 12.0..=x3 + 12.0, 0.0..=42.0)
                ]
            );
            rig.press(beat(3.0), false);
            assert!(
                rig.rects(control)
                    .contains(&Rect::from_x_y_ranges(x1..=x3, 0.0..=42.0))
            );
            assert_eq!(rig.rects(module), []);
            assert!(rig.rects(INK).iter().all(|r| r.width() == 8.0));
            // Nothing over the editor below the ruler but the `+`.
            assert!((rig.rects(control).iter().chain(&rig.rects(INK))).all(|r| r.bottom() <= 48.0));
        }

        #[test]
        fn a_focus_loss_ends_an_edge_drag_where_it_was_and_taps_nothing() {
            let mut rig = Rig::new();
            rig.frame(vec![Event::PointerMoved(beat(1.0))]);
            rig.press(beat(1.0), true);
            rig.frame_after(0.31, vec![]);
            rig.frame(vec![Event::PointerMoved(beat(3.0))]);
            rig.frame(vec![Event::WindowFocused(false)]);
            // egui keeps the button down: back in the window, the pointer only hovers.
            rig.frame(vec![
                Event::WindowFocused(true),
                Event::PointerMoved(beat(6.0)),
            ]);
            assert_eq!(rig.range, Some(BEATS_1_3));
            // A press gone stale before its 0.3 s: no tap.
            rig.frame(vec![Event::PointerMoved(beat(5.0))]);
            rig.press(beat(5.0), true);
            rig.frame(vec![Event::WindowFocused(false)]);
            rig.frame(vec![Event::WindowFocused(true)]);
            assert!(rig.taps().is_empty());
        }

        #[test]
        fn a_pinch_selects_nothing_and_taps_nothing() {
            use eframe::egui::TouchPhase::{End, Move, Start};
            let touch = |id, phase, pos| Event::Touch {
                device_id: eframe::egui::TouchDeviceId(1),
                id: eframe::egui::TouchId(id),
                phase,
                pos,
                force: None,
            };
            let mut rig = Rig::new();
            let at = beat(5.0);
            rig.frame(vec![Event::PointerMoved(at)]);
            rig.frame(vec![
                touch(1, Start, at),
                Event::PointerButton {
                    pos: at,
                    button: PointerButton::Primary,
                    pressed: true,
                    modifiers: Modifiers::NONE,
                },
            ]);
            rig.frame(vec![touch(2, Start, beat(9.0))]);
            rig.frame_after(0.31, vec![touch(2, Move, beat(10.0))]);
            rig.frame(vec![touch(2, End, beat(10.0))]);
            rig.frame(vec![
                touch(1, End, at),
                Event::PointerButton {
                    pos: at,
                    button: PointerButton::Primary,
                    pressed: false,
                    modifiers: Modifiers::NONE,
                },
                Event::PointerGone,
            ]);
            assert_eq!((rig.range, rig.taps()), (None, vec![]));
        }

        #[test]
        fn a_pinch_during_a_range_drag_puts_the_range_back_as_it_was() {
            // A pinch must not keep the half-dragged loop.
            use eframe::egui::TouchPhase::{self, End, Move, Start};
            let finger = |rig: &mut Rig, id, phase: TouchPhase, pos, dt| {
                let mut events = vec![Event::Touch {
                    device_id: eframe::egui::TouchDeviceId(1),
                    id: eframe::egui::TouchId(id),
                    phase,
                    pos,
                    force: None,
                }];
                if id == 1 {
                    events.push(Event::PointerMoved(pos));
                    if phase != Move {
                        events.push(Event::PointerButton {
                            pos,
                            button: PointerButton::Primary,
                            pressed: phase == Start,
                            modifiers: Modifiers::NONE,
                        });
                    }
                }
                rig.frame_after(dt, events);
            };
            let pinched = |rig: &mut Rig, from: Pos2, long: f64| {
                finger(rig, 1, Start, from, 1.0 / 60.0);
                finger(rig, 1, Move, from, long);
                finger(rig, 1, Move, beat(8.0), 1.0 / 60.0);
                assert_ne!(rig.range, Some(BEATS_1_3), "dragged live");
                finger(rig, 2, Start, beat(12.0), 1.0 / 60.0);
                finger(rig, 1, Move, beat(9.0), 1.0 / 60.0);
                finger(rig, 2, End, beat(12.0), 1.0 / 60.0);
                finger(rig, 1, End, beat(9.0), 1.0 / 60.0);
                rig.frame(vec![Event::PointerGone]);
                assert_eq!(rig.range, Some(BEATS_1_3));
                assert_eq!(rig.plus(), None);
            };
            // A new range from a long press, which cleared the old one; then the old one's end edge.
            let mut rig = Rig::new();
            rig.select(beat(1.0), beat(3.0));
            pinched(&mut rig, beat(5.0), 0.31);
            pinched(&mut rig, beat(3.0), 1.0 / 60.0);
            assert!(rig.taps().is_empty());
        }
    }

    /// The playhead line and disc every editor shares.
    mod line {
        use eframe::egui::{Color32, Context, LayerId, Pos2, RawInput, Rect, Shape, pos2, vec2};

        use super::super::{disc, playhead_line};
        use crate::metrics::{DEFAULT_UI_SCALE_INDEX, Metrics};
        use crate::theme::theme_by_id;
        use crate::ui::UiCtx;

        /// A grid from (100, 60) to (900, 400) points, a ruler 1.5 U tall above it.
        const GRID: Rect = Rect {
            min: pos2(100.0, 60.0),
            max: pos2(900.0, 400.0),
        };

        /// Runs `f` on a background painter at `ppp` and returns the rects (in device px, with their fill) and the
        /// circles (centre, radius in points, fill, clip rect) it painted.
        type Circles = Vec<(Pos2, f32, Color32, Rect)>;
        fn paint(
            ppp: f32,
            f: impl Fn(&eframe::egui::Painter, &UiCtx),
        ) -> (Vec<(Rect, Color32)>, Circles, Vec<Color32>) {
            let m = Metrics::from_pixels_per_point(ppp, DEFAULT_UI_SCALE_INDEX);
            let theme = theme_by_id(0);
            let ctx = Context::default();
            let mut raw = RawInput {
                screen_rect: Some(Rect::from_min_size(Pos2::ZERO, vec2(1000.0, 500.0))),
                ..Default::default()
            };
            raw.viewports
                .entry(raw.viewport_id)
                .or_default()
                .native_pixels_per_point = Some(ppp);
            let out = ctx.run(raw, |ctx| {
                let painter = ctx.layer_painter(LayerId::background());
                f(
                    &painter,
                    &UiCtx {
                        theme: &theme,
                        m: &m,
                    },
                );
            });
            let (mut rects, mut circles, mut paths) = (Vec::new(), Vec::new(), Vec::new());
            for s in out.shapes {
                match s.shape {
                    Shape::Rect(r) => rects.push((r.rect * ppp, r.fill)),
                    Shape::Circle(c) => circles.push((c.center, c.radius, c.fill, s.clip_rect)),
                    Shape::Path(p) => paths.push(p.fill),
                    _ => {}
                }
            }
            (rects, circles, paths)
        }

        fn line(ppp: f32, x: f32) -> Vec<(Rect, Color32)> {
            paint(ppp, |p, cx| playhead_line(p, GRID, x, cx)).0
        }

        #[test]
        fn a_2t_px_text_line_on_a_2t_plus_2_px_background_shadow_down_the_grid() {
            let c = theme_by_id(0).colors;
            let shadow = c.background.to_color32().gamma_multiply(0.3);
            let text = c.text.to_color32();
            // ppp 1, U = 32: t = 2 px. ppp 2, U = 64 px: t = 4 px. The x is the nearest pixel's.
            for (ppp, t) in [(1.0f32, 2.0), (2.0, 4.0)] {
                let x = 300.25;
                let px = (x * ppp).round();
                let ys = GRID.top() * ppp..=GRID.bottom() * ppp;
                let band = |half: f32| Rect::from_x_y_ranges(px - half..=px + half, ys.clone());
                assert_eq!(
                    line(ppp, x),
                    [(band(t + 1.0), shadow), (band(t), text)],
                    "ppp {ppp}"
                );
            }
        }

        #[test]
        fn the_width_follows_t_only_twelve_px_and_fourteen_with_the_shadow_at_u_85() {
            // U = 85 px (ppp 2.65625): t = 6.
            let widths = |ppp: f32| {
                line(ppp, 400.0)
                    .iter()
                    .map(|(r, _)| r.width().round())
                    .collect::<Vec<_>>()
            };
            assert_eq!(widths(2.65625), [14.0, 12.0]);
            assert_eq!(widths(1.0), [6.0, 4.0]);
        }

        #[test]
        fn it_shows_only_while_its_x_lies_on_the_grid() {
            assert_eq!(line(1.0, GRID.left()).len(), 2);
            assert_eq!(line(1.0, GRID.right()).len(), 2);
            assert!(line(1.0, GRID.left() - 0.5).is_empty());
            assert!(line(1.0, GRID.right() + 0.5).is_empty());
        }

        #[test]
        fn the_disc_is_a_1_35_ruler_square_less_3_px_of_radius() {
            // U = 32 points at both: a 48-point ruler. 3 px is 3 points at ppp 1 and 1.5 at ppp 2.
            for (ppp, r) in [(1.0, 29.4), (2.0, 30.9)] {
                let m = Metrics::from_pixels_per_point(ppp, DEFAULT_UI_SCALE_INDEX);
                let (side, radius) = disc(48.0, &m);
                assert!(
                    (side - 64.8).abs() < 1e-4 && (radius - r).abs() < 1e-4,
                    "{side} {radius}"
                );
            }
        }
    }

    /// The playhead knob.
    mod knob {
        use eframe::egui::{Event, Id, Modifiers, PointerButton, Pos2, Rect, Shape, pos2, vec2};

        use super::super::{KnobGesture, PlayheadKnob, fade, knob_seek};
        use crate::ui::offscreen::Offscreen;

        /// U = 32: the playlist's knob on its 1.5 U time ruler from x 48 (a 1.35 × 48 square, a disc of 32.4 − 3 =
        /// 29.4), riding a playhead at x 200 on the ruler's mid-line y 24.
        const D: f32 = 1.35 * 48.0;
        const R: f32 = 29.4;
        const AT: Pos2 = pos2(200.0, 24.0);
        const RULER: Rect = Rect {
            min: pos2(48.0, 0.0),
            max: pos2(1170.0, 48.0),
        };
        const U: f32 = 32.0;

        struct Rig {
            screen: Offscreen,
            knob: PlayheadKnob,
            /// The live playhead's x, and the fade's target.
            x: f32,
            shown: bool,
            outs: Vec<KnobGesture>,
            /// The last frame's discs: centre, radius and alpha.
            discs: Vec<(Pos2, f32, u8)>,
        }

        impl Rig {
            fn new() -> Self {
                let mut rig = Self {
                    screen: Offscreen::new(),
                    knob: PlayheadKnob::default(),
                    x: AT.x,
                    shown: true,
                    outs: Vec::new(),
                    discs: Vec::new(),
                };
                rig.frame(vec![]);
                rig
            }

            fn frame_after(&mut self, dt: f64, events: Vec<Event>) {
                self.screen.time += dt - 1.0 / 60.0;
                let (knob, x, shown) = (&mut self.knob, self.x, self.shown);
                let mut out = None;
                let shapes = self.screen.frame(events, &mut |ui, cx| {
                    // The host's side: a drag shows where the finger is (no grid here).
                    let x = knob.dragging(ui, cx.m).unwrap_or(x);
                    out = knob.show(ui, Id::new("knob"), RULER, x, shown, cx);
                });
                self.outs.extend(out);
                self.discs = (shapes.iter())
                    .filter_map(|s| match &s.shape {
                        Shape::Circle(c) => Some((c.center, c.radius, c.fill.a())),
                        _ => None,
                    })
                    .collect();
            }

            fn frame(&mut self, events: Vec<Event>) {
                self.frame_after(1.0 / 60.0, events);
            }

            fn button_after(&mut self, dt: f64, pos: Pos2, pressed: bool) {
                let button = Event::PointerButton {
                    pos,
                    button: PointerButton::Primary,
                    pressed,
                    modifiers: Modifiers::NONE,
                };
                self.frame_after(dt, vec![Event::PointerMoved(pos), button]);
            }

            /// Presses at `from`, moves to `to` and lets go there `held` seconds after the press.
            fn gesture(&mut self, from: Pos2, to: Pos2, held: f64) -> Vec<KnobGesture> {
                self.outs.clear();
                self.frame(vec![Event::PointerMoved(from)]);
                self.button_after(1.0 / 60.0, from, true);
                self.frame(vec![Event::PointerMoved(to)]);
                self.button_after(held - 1.0 / 60.0, to, false);
                std::mem::take(&mut self.outs)
            }

            /// The one disc drawn: centre, radius, alpha.
            fn disc(&self) -> Option<(Pos2, f32, u8)> {
                assert!(self.discs.len() <= 1, "{:?}", self.discs);
                self.discs.first().copied()
            }
        }

        #[test]
        fn a_quick_still_tap_starts_playback_and_a_slow_or_moved_press_does_nothing() {
            let mut rig = Rig::new();
            assert_eq!(rig.gesture(AT, AT, 0.1), [KnobGesture::Tap]);
            // Up to 0.5 U of wander is still a tap.
            let wander = AT + vec2(0.3 * U, 0.39 * U);
            assert_eq!(rig.gesture(AT, wander, 0.1), [KnobGesture::Tap]);
            // Held 0.3 s.
            assert_eq!(rig.gesture(AT, AT, 0.31), []);
            // Moved 0.6 U up: no tap, and no drag either.
            assert_eq!(rig.gesture(AT, AT - vec2(0.0, 0.6 * U), 0.1), []);
            // Off the knob: nothing.
            assert_eq!(rig.gesture(AT + vec2(D, 0.0), AT + vec2(D, 0.0), 0.1), []);
            // A press held over a focus loss does nothing.
            rig.frame(vec![Event::PointerMoved(AT)]);
            rig.button_after(1.0 / 60.0, AT, true);
            rig.frame(vec![Event::WindowFocused(false)]);
            rig.button_after(1.0 / 60.0, AT, false);
            assert_eq!(rig.outs, []);
        }

        #[test]
        fn a_second_finger_cancels_a_knob_drag_which_then_drops_nothing() {
            // The shared gesture law: a second finger ends the one-finger gesture.
            use eframe::egui::TouchPhase::{End, Move, Start};
            let touch = |id, phase, pos| Event::Touch {
                device_id: eframe::egui::TouchDeviceId(1),
                id: eframe::egui::TouchId(id),
                phase,
                pos,
                force: None,
            };
            let button = |pos, pressed| Event::PointerButton {
                pos,
                button: PointerButton::Primary,
                pressed,
                modifiers: Modifiers::NONE,
            };
            let mut rig = Rig::new();
            rig.frame(vec![Event::PointerMoved(AT)]);
            rig.frame(vec![touch(1, Start, AT), button(AT, true)]);
            let to = pos2(300.0, AT.y);
            rig.frame(vec![touch(1, Move, to), Event::PointerMoved(to)]);
            assert_eq!(rig.disc().unwrap().0, to);
            rig.frame(vec![touch(2, Start, pos2(600.0, 30.0))]);
            rig.frame(vec![touch(2, End, pos2(600.0, 30.0))]);
            // Back on the live playhead, and the lift drops nothing.
            assert_eq!(rig.disc().unwrap().0, AT);
            rig.frame(vec![touch(1, Move, to), Event::PointerMoved(to)]);
            rig.frame(vec![
                touch(1, End, to),
                button(to, false),
                Event::PointerGone,
            ]);
            assert_eq!(rig.disc().unwrap().0, AT);
            assert_eq!(rig.outs, []);
        }

        #[test]
        fn a_sideways_drag_drops_where_the_finger_lets_go_and_the_disc_follows_it() {
            let mut rig = Rig::new();
            rig.frame(vec![Event::PointerMoved(AT)]);
            rig.button_after(1.0 / 60.0, AT, true);
            // 0.5 U is no drag yet: the disc stays.
            rig.frame(vec![Event::PointerMoved(AT + vec2(0.5 * U, 0.0))]);
            assert_eq!(rig.disc().unwrap().0, AT);
            rig.frame(vec![Event::PointerMoved(pos2(300.0, 40.0))]);
            assert_eq!(rig.disc().unwrap().0, pos2(300.0, AT.y));
            // However long it took.
            rig.button_after(1.0, pos2(310.0, 40.0), false);
            assert_eq!(rig.outs, [KnobGesture::Drop(310.0)]);
            // Then the disc is the playhead's again.
            rig.frame(vec![]);
            assert_eq!(rig.disc().unwrap().0, AT);
        }

        #[test]
        fn the_disc_grows_while_held_and_wobbles_once_let_go_by_the_knob_law() {
            // The knobs' own law (`widgets::pulse`, its 0.6 a sixtieth of a second; the rig's frames are 1/60 s).
            let mut rig = Rig::new();
            assert!((rig.disc().unwrap().1 - R).abs() < 1e-4);
            rig.frame(vec![Event::PointerMoved(AT)]);
            rig.button_after(1.0 / 60.0, AT, true);
            let radius = |rig: &Rig| rig.disc().unwrap().1;
            // The square is h = D px tall: r = R·(1 + k·10/D).
            let r = |k: f32| R * (1.0 + k * 10.0 / D);
            // Held: k = 1 − max(0.01, 0.6·a), a = 1, 0.6, 0.36 …
            for k in [0.4, 0.64, 0.784] {
                assert!((radius(&rig) - r(k)).abs() < 1e-3, "{}", radius(&rig));
                rig.frame(vec![]);
            }
            for _ in 0..20 {
                rig.frame(vec![]);
            }
            assert!((radius(&rig) - r(0.99)).abs() < 1e-3);
            // Let go: k = (1 − a)·sin(2π(1 − a)), a restarting at 1 and decaying in the release frame already (as on
            // touch-down), down to a = 0.01, then R.
            rig.button_after(1.0 / 60.0, AT, false);
            let mut a: f32 = 0.6;
            while a >= 0.01 {
                let want = r((1.0 - a) * (std::f32::consts::TAU * (1.0 - a)).sin());
                assert!(
                    (radius(&rig) - want).abs() < 1e-3,
                    "a {a}: {}",
                    radius(&rig)
                );
                rig.frame(vec![]);
                a *= 0.6;
            }
            assert!((radius(&rig) - R).abs() < 1e-4);
        }

        #[test]
        fn the_disc_fades_a_fifth_of_the_way_a_frame_toward_its_target_tracking_the_live_playhead()
        {
            assert_eq!(fade(1.0, 0.0), 0.8);
            assert_eq!(fade(0.5, 1.0), 0.6);
            // Snapping within 0.001 of the target.
            assert_eq!(fade(0.00124, 0.0), 0.0);
            assert_eq!(fade(0.0013, 0.0), 0.0013 * 0.8);
            assert_eq!(fade(0.9988, 1.0), 1.0);

            let mut rig = Rig::new();
            let disc = |rig: &Rig| {
                rig.disc()
                    .map(|(at, r, a)| (at, (r * 10.0).round() / 10.0, a))
            };
            assert_eq!(disc(&rig), Some((AT, R, 255)));
            // Its target off (playback started, on the step grid): it fades out tracking the live playhead.
            rig.shown = false;
            rig.frame(vec![]);
            assert_eq!(disc(&rig), Some((AT, R, 204)));
            rig.x = 260.0;
            rig.frame(vec![]);
            assert_eq!(disc(&rig), Some((pos2(260.0, AT.y), R, 163)));
            // Fading out, it takes no tap.
            assert_eq!(rig.gesture(pos2(260.0, AT.y), pos2(260.0, AT.y), 0.1), []);
            for _ in 0..40 {
                rig.frame(vec![]);
            }
            assert_eq!(disc(&rig), None);
            // Back on, it fades in at the playhead.
            (rig.x, rig.shown) = (300.0, true);
            rig.frame(vec![]);
            assert_eq!(disc(&rig), Some((pos2(300.0, AT.y), R, 51)));
            // Off the ruler it is neither drawn nor touched, whatever its alpha.
            let mut rig = Rig::new();
            rig.x = 20.0;
            rig.frame(vec![]);
            assert_eq!(disc(&rig), None);
            assert_eq!(rig.gesture(pos2(20.0, AT.y), pos2(20.0, AT.y), 0.1), []);
        }

        #[test]
        fn a_drop_seeks_to_the_nearest_grid_line_between_the_view_left_and_the_song_end() {
            // A beat grid of 128 ticks, a song 32 beats long, the view from tick 0.
            let end = 4096.0;
            assert_eq!(knob_seek(447.0, Some(128.0), 0.0, end), 384);
            assert_eq!(knob_seek(449.0, Some(128.0), 0.0, end), 512);
            // Snap off: unrounded (whole ticks).
            assert_eq!(knob_seek(447.4, None, 0.0, end), 447);
            assert_eq!(knob_seek(-70.0, Some(128.0), 0.0, end), 0);
            assert_eq!(knob_seek(5000.0, Some(128.0), 0.0, end), 4096);
            // Clamped first, then rounded: left of a view scrolled to tick 300, it rounds from 300.
            assert_eq!(knob_seek(100.0, Some(128.0), 300.0, end), 256);
            assert_eq!(knob_seek(100.0, None, 300.0, end), 300);
            // A song end off the grid, an empty song, and a view scrolled past the end.
            assert_eq!(knob_seek(4000.0, Some(128.0), 0.0, 3950.0), 3950);
            assert_eq!(knob_seek(300.0, Some(128.0), 0.0, 0.0), 0);
            assert_eq!(knob_seek(5000.0, None, 4500.0, end), 4096);
        }
    }

    /// The bar-number badges.
    mod badge {
        use eframe::egui::{Color32, Context, LayerId, Pos2, RawInput, Rect, Shape, pos2, vec2};

        use super::super::ruler;
        use crate::metrics::{DEFAULT_UI_SCALE_INDEX, Metrics};
        use crate::theme::theme_by_id;
        use crate::ui::UiCtx;

        /// What a ruler painted: the `text`-filled discs (centre, radius) and the texts (label, colour, font size,
        /// centre of the galley), all in points.
        struct Painted {
            m: Metrics,
            ruler: Rect,
            discs: Vec<(Pos2, f32)>,
            texts: Vec<(String, Color32, f32, Pos2)>,
        }

        /// A ruler 1.5 U tall across 40 U whose marks start at `left` U; tick 0 is at `x0` U, a bar 8 U wide (4 beats).
        /// `ppp` 1 gives U = 32, 2.5 gives U = 80.
        fn paint(ppp: f32, left: f32, x0: f32) -> Painted {
            let m = Metrics::from_pixels_per_point(ppp, DEFAULT_UI_SCALE_INDEX);
            let theme = theme_by_id(0);
            let text = theme.colors.text.to_color32();
            let ruler_rect = Rect::from_min_size(pos2(0.0, 0.0), vec2(m.u(40.0), m.u(1.5)));
            let marks = ruler_rect.with_min_x(m.u(left));
            let ctx = Context::default();
            let raw = RawInput {
                screen_rect: Some(ruler_rect),
                ..Default::default()
            };
            let output = ctx.run(raw, |ctx| {
                let painter = ctx.layer_painter(LayerId::background());
                let cx = UiCtx {
                    theme: &theme,
                    m: &m,
                };
                let sig = crate::song::TimeSig::default();
                ruler(&painter, ruler_rect, marks, m.u(x0), m.u(2.0), sig, &cx);
            });
            let (mut discs, mut texts) = (Vec::new(), Vec::new());
            // The painter's clip rect is not applied to the shapes: keep those that can show.
            for s in output
                .shapes
                .into_iter()
                .filter(|s| s.clip_rect.is_positive())
            {
                match s.shape {
                    Shape::Circle(c) if c.fill == text && ruler_rect.contains(c.center) => {
                        discs.push((c.center, c.radius));
                    }
                    Shape::Text(t) if ruler_rect.contains(t.pos) => texts.push((
                        t.galley.text().to_owned(),
                        t.galley.job.sections[0].format.color,
                        t.galley.job.sections[0].format.font_id.size,
                        t.pos + t.galley.rect.center().to_vec2(),
                    )),
                    _ => {}
                }
            }
            Painted {
                m,
                ruler: ruler_rect,
                discs,
                texts,
            }
        }

        fn labels(p: &Painted) -> Vec<&str> {
            p.texts.iter().map(|t| t.0.as_str()).collect()
        }

        #[test]
        fn the_disc_radius_is_six_hairline_units_at_the_ruler_mid_height() {
            // U = 32: t = 2 px, r = 12 px. U = 80 (ppp 2.5): t = 5 px, r = 30 px = 12 points. U = 85
            // (ppp 2.65625): t = ceil(85/16) = 6 px, r = 36 px.
            for (ppp, radius_px) in [(1.0, 12.0), (2.5, 30.0), (2.65625, 36.0)] {
                let p = paint(ppp, 1.5, 1.5);
                assert_eq!(
                    p.discs.len(),
                    2,
                    "bars 3 and 5 of the first 40 U (bar = 8 U)"
                );
                for (i, &(center, r)) in p.discs.iter().enumerate() {
                    assert!((r * ppp - radius_px).abs() < 1e-3, "ppp {ppp}: {}", r * ppp);
                    assert_eq!(center.y, p.ruler.center().y, "ppp {ppp}");
                    // On the bar line: bar 3 is 16 U right of tick 0, bar 5 is 32 U.
                    assert_eq!(center.x, p.m.u(1.5 + 16.0 + 16.0 * i as f32), "ppp {ppp}");
                }
            }
        }

        #[test]
        fn the_number_is_font_step_0_in_menu_fill_centred_on_the_disc() {
            let p = paint(1.0, 1.5, 1.5);
            assert_eq!(labels(&p), ["3", "5"]);
            let menu_fill = theme_by_id(0).colors.menu_fill.to_color32();
            for (i, (_, color, size, center)) in p.texts.iter().enumerate() {
                assert_eq!(*color, menu_fill);
                // Step 0 is 6 pt: floor(32 · 6 / 16) = 12 px.
                assert_eq!(*size, 12.0);
                assert!(center.distance(p.discs[i].0) < 0.5, "{center:?}");
            }
            // And U = 80: floor(80 · 6 / 16) = 30 px.
            assert_eq!(paint(2.5, 1.5, 1.5).texts[0].2 * 2.5, 30.0);
        }

        #[test]
        fn a_bar_line_closer_than_one_radius_to_the_marks_left_edge_has_no_badge() {
            // Radius 12 px at U = 32; marks start at 1.5 U = 48 px, so bar 3's line at 48 + 12 = 60 px is the
            // first that shows. x0 = line − 16 U (bar 3 is two bars, 512 px, right of tick 0).
            let line = |px: f32| (px - 512.0) / 32.0;
            assert_eq!(labels(&paint(1.0, 1.5, line(60.0))), ["3", "5", "7"]);
            assert_eq!(labels(&paint(1.0, 1.5, line(59.5))), ["5", "7"]);
            // Under the header column it is gone too, and unscrolled, bar 1's line is on the edge.
            assert_eq!(labels(&paint(1.0, 1.5, line(20.0))), ["5", "7"]);
            assert_eq!(labels(&paint(1.0, 1.5, 1.5)), ["3", "5"]);
            assert_eq!(paint(1.0, 1.5, 1.5).discs[0].0.x, 48.0 + 512.0);
        }

        #[test]
        fn at_2_u_a_beat_every_other_bar_from_the_third_has_one() {
            let p = paint(1.0, 0.0, 0.0);
            assert_eq!(labels(&p), ["3", "5"]);
        }
    }

    mod law {
        use eframe::egui::{Color32, Context, LayerId, RawInput, Rect, Shape, pos2, vec2};

        use super::super::{Level, marks, next, ruler, spacings};
        use crate::metrics::{DEFAULT_UI_SCALE_INDEX, Metrics};
        use crate::song::TimeSig;
        use crate::theme::theme_by_id;
        use crate::ui::UiCtx;

        const SIG: TimeSig = TimeSig {
            subdivision: 4,
            beats_per_bar: 4,
        };

        /// The rects (fill, rect) and the badge numbers a 1.5 U ruler across 40 U paints at U = 32, from tick 0 at its
        /// left, a beat `beat_u` U wide.
        fn paint(beat_u: f32) -> (Vec<(Color32, Rect)>, Vec<String>) {
            let m = Metrics::from_pixels_per_point(1.0, DEFAULT_UI_SCALE_INDEX);
            let theme = theme_by_id(0);
            let rect = Rect::from_min_size(pos2(0.0, 0.0), vec2(m.u(40.0), m.u(1.5)));
            let ctx = Context::default();
            let raw = RawInput {
                screen_rect: Some(rect),
                ..Default::default()
            };
            let output = ctx.run(raw, |ctx| {
                let painter = ctx.layer_painter(LayerId::background());
                let cx = UiCtx {
                    theme: &theme,
                    m: &m,
                };
                ruler(&painter, rect, rect, 0.0, m.u(beat_u), SIG, &cx);
            });
            let (mut rects, mut texts) = (Vec::new(), Vec::new());
            for s in output.shapes {
                match s.shape {
                    Shape::Rect(r) => rects.push((r.fill, r.rect)),
                    Shape::Text(t) => texts.push(t.galley.text().to_owned()),
                    _ => {}
                }
            }
            (rects, texts)
        }

        #[test]
        fn the_ruler_is_a_background_fill_with_a_menu_fill_baseline() {
            // `background` (#21272B in Classic Gray). The baseline: t = 2 tall at bottom − 2t.
            let colors = theme_by_id(0).colors;
            let (rects, _) = paint(4.0);
            assert_eq!(
                colors.background.to_color32(),
                Color32::from_rgb(0x21, 0x27, 0x2B)
            );
            let fill = Rect::from_min_size(pos2(0.0, 0.0), vec2(1280.0, 48.0));
            assert_eq!(rects[0], (colors.background.to_color32(), fill));
            let base = Rect::from_min_max(pos2(0.0, 44.0), pos2(1280.0, 46.0));
            assert_eq!(rects[1], (colors.menu_fill.to_color32(), base));
        }

        #[test]
        fn beat_ticks_are_2t_by_0_3h_and_step_and_bar_ticks_t_by_0_2h_over_the_baseline() {
            // 4 U a beat at U = 32: 128 points a beat, ticks every step (32 points); bottoms at 48 − 3t = 42.
            let ink = theme_by_id(0).colors.menu_fill.to_color32();
            let (rects, _) = paint(4.0);
            let ticks: Vec<Rect> = rects
                .iter()
                .filter(|r| r.0 == ink)
                .map(|r| r.1)
                .skip(1)
                .collect();
            let near = |a: Rect, b: Rect| {
                (a.min - b.min).length() < 1e-3 && (a.max - b.max).length() < 1e-3
            };
            let tick = |x: f32, w: f32, h: f32| {
                Rect::from_x_y_ranges(x - w / 2.0..=x + w / 2.0, 42.0 - h..=42.0)
            };
            // Bar 1's line (top level, bar style), step 1, beat 1.
            assert!(near(ticks[0], tick(0.0, 2.0, 9.6)), "{:?}", ticks[0]);
            assert!(near(ticks[1], tick(32.0, 2.0, 9.6)), "{:?}", ticks[1]);
            assert!(near(ticks[4], tick(128.0, 4.0, 14.4)), "{:?}", ticks[4]);
        }

        #[test]
        fn at_4_u_a_beat_every_bar_has_a_badge_at_2_u_every_other_one_zoomed_out_every_fourth() {
            // The automation and audio editors open at 4 U a beat, where s2 = 1 bar: bar 2 shows (bar 1 sits on the
            // left edge). The piano roll's 2 U and the playlist's 1.2 U give s2 = 2 bars: 3, 5, 7.
            assert_eq!(paint(4.0).1, ["2", "3"]);
            assert_eq!(paint(2.0).1, ["3", "5"]);
            assert_eq!(paint(1.2).1, ["3", "5", "7", "9"]);
            // At 0.8 U a beat, s2 = 4 bars: 1 (on the edge), 5, 9, 13.
            assert_eq!(paint(0.8).1, ["5", "9", "13"]);
        }

        #[test]
        fn the_spacings_clamp_d_and_b_to_2_4() {
            // s1's bound is clamp(d, 2, 4)·m, s2's clamp(d, 2, 4)·clamp(b, 2, 4)·m.
            let sig = |d, b| TimeSig {
                subdivision: d,
                beats_per_bar: b,
            };
            assert_eq!(spacings(0.6 * 0.33, sig(8, 4)), [0.25, 1.0, 4.0]);
            assert_eq!(spacings(0.6, sig(4, 7)), [1.0, 7.0, 14.0]);
        }

        #[test]
        fn the_marks_follow_next_and_its_spacings() {
            let sig = |d, b| TimeSig {
                subdivision: d,
                beats_per_bar: b,
            };
            // d = 4, b = 4: B = 16 steps.
            assert_eq!(next(0.25, 4, 4), 0.5);
            assert_eq!(next(0.5, 4, 4), 1.0);
            assert_eq!(next(1.0, 4, 4), 2.0);
            assert_eq!(next(4.0, 4, 4), 8.0);
            // d = 3, b = 3: a step, then 1 beat (d/n = 3), then a bar (B/n = 3), then 2 bars.
            assert_eq!(next(1.0 / 3.0, 3, 3), 1.0);
            assert_eq!(next(1.0, 3, 3), 3.0);
            assert_eq!(next(3.0, 3, 3), 6.0);
            // b = 5: from a beat, B/n = 5 → a bar.
            assert_eq!(next(1.0, 4, 5), 5.0);
            // At 3.03 U a beat (m = 0.6 U = 0.198 beat): steps, beats and bars; zoomed in to 10 U a beat (m = 0.06), the same.
            assert_eq!(spacings(0.6 * 0.33, sig(4, 4)), [0.25, 1.0, 4.0]);
            assert_eq!(spacings(0.06, sig(4, 4)), [0.25, 1.0, 4.0]);
            // A beat 1 U wide (m = 0.6): fine marks every 2 steps... no: every beat (0.25 → 0.5 → 1); mid 4 beats; top 16.
            assert_eq!(spacings(0.6, sig(4, 4)), [1.0, 4.0, 16.0]);
            let levels: Vec<_> = marks(0, 16, [0.25, 1.0, 4.0], 4).collect();
            assert_eq!(levels.len(), 17);
            assert_eq!(levels[0], (0, Level::Top));
            assert_eq!(levels[4], (4, Level::Mid));
            assert_eq!(levels[5], (5, Level::Fine));
            assert_eq!(levels[16], (16, Level::Top));
            assert_eq!(
                marks(-7, 5, [0.5, 1.0, 4.0], 4).next(),
                Some((0, Level::Top))
            );
            // Every s0 in song steps: from step 3, the marks every 2 steps start at 4.
            let steps: Vec<i64> = marks(3, 9, [0.5, 1.0, 4.0], 4).map(|m| m.0).collect();
            assert_eq!(steps, [4, 6, 8]);
        }
    }
}
