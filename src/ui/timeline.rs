//! The editors' time ruler: the playlist's, the piano roll's, the automation and audio editors' and both drum modes'.
//! Every ruler is 1.65 U tall ([`RULER_H`]) and follows one law, h its height, q = ceil(0.0375·U) px its hairline
//! ([`q`]):
//! 1. a `background` fill;
//! 2. a `menuFill` baseline, ceil(1.5·q) px tall, q above the bottom, across the full width ([`baseline_top`]);
//! 3. the range, if any ([`RangeSelect`]);
//! 4. `menuFill` ticks q wide standing on the baseline, spaced by the mark law ([`next`], [`spacings`], [`marks`]):
//!    0.85 of the height above the baseline for the labelled (top-level) marks, 0.35 for mid marks, 0.25 for fine ones;
//! 5. on each top-level mark the 1-based bar number in 8 pt `text` at α 0.8, left-aligned 0.15 U right of the bar line,
//!    its top 0.15·(h − 2q) below the ruler top; none for a bar line left of the marks' left edge (the playlist's
//!    header column is not part of it);
//! 6. the scroll thumb while the view shows less than the content ([`scroll_thumb`]): display only.
//!
//! The law labels every bar at the editors' opening zooms (the playlist's 1.2 U a beat, the piano roll's 2 U, the
//! automation and audio editors' 4 U) and every other bar at 0.8 U. In Next, B/n and d/n are whole-number divisions.
//!
//! It scrolls and zooms: the caller passes tick 0's x and the beat width. Every editor's playhead is the shared grid
//! line ([`playhead_line`]) and, on the ruler, the play box ([`PlayheadKnob`]) right of it. A tap on the ruler sets
//! the playhead, and a range picked on it is the song's play loop, whose `+` button opens the region menu
//! ([`RangeSelect`]).

use eframe::egui::{
    self, Align2, Color32, CornerRadius, Id, Painter, Pos2, Rangef, Rect, Sense, Shape, pos2, vec2,
};

use crate::icons::{IconId, draw_icon};
use crate::metrics::{FontSize, Metrics};
use crate::song::{PPQ, TimeSig};
use crate::theme::RECORD_COLOR;
use crate::ui::buttons::filled_circle_button;
use crate::ui::clips::TimeAxis;
use crate::ui::piano_roll::notes::snap_round;
use crate::ui::region::{self, RegionOp, TimeRange};
use crate::ui::{UiCtx, anim, gesture, pinch_press, popup, primary_down_live, widgets};

// Sizes in U.
/// Every ruler's height: the playlist's and every clip editor's.
pub const RULER_H: f32 = 1.65;
/// The marks are never closer than this, in U.
const MARK_U: f32 = 0.6;
/// Tick heights, in shares of the ruler top to the baseline: labelled, mid and fine marks.
const TOP_TICK: f32 = 0.85;
const MID_TICK: f32 = 0.35;
const FINE_TICK: f32 = 0.25;
/// A bar label's left inset from its bar line, in U, and its top, in shares of h − 2q below the ruler top.
const LABEL_X: f32 = 0.15;
const LABEL_Y: f32 = 0.15;
const LABEL_ALPHA: f32 = 0.8;
/// The ruler hairline q, in U (rounded up to whole device px).
const Q_U: f32 = 0.0375;
/// The scroll thumb: its track stops this short of the ruler's right end; `text` at this alpha.
const THUMB_END: f32 = 0.15;
const THUMB_ALPHA: f32 = 0.3;
/// The play box: its width, its inset from the ruler's top and bottom, its right corners' radius, the icon's scale and
/// alpha, how far its line reaches below the ruler (in ruler heights), and its touch area around the playhead x.
const BOX_W: f32 = 1.72;
const BOX_INSET: f32 = 0.08;
const BOX_R: f32 = 0.15;
const BOX_ICON: f32 = 0.82;
const BOX_ICON_ALPHA: f32 = 0.85;
const BOX_LINE_BELOW: f32 = 0.175;
const BOX_TOUCH_LEFT: f32 = 0.56;
const BOX_TOUCH_RIGHT: f32 = 2.29;
const BOX_TOUCH_Y: f32 = 0.29;
/// The armed box's fill: this share of `recordColor` over `menuFill`.
const BOX_ARMED_MIX: f32 = 0.22;
/// The play box's fade keeps this much of the way each 1/60 s (k = 13.33 a second), snapping within this.
const FADE_KEEP: f32 = 0.8;
const FADE_SNAP: f32 = 0.001;
/// The line's shadow: `background` at this alpha.
const SHADOW_ALPHA: f32 = 0.3;
/// The range's edge handles, centred on each edge.
const HANDLE_W: f32 = 1.0;
/// The `+` button under the released edge.
const PLUS_D: f32 = 1.25;
/// How long a dropped play box stays put waiting for the output to report the seek, in seconds.
const DROP_HOLD_S: f64 = 0.25;

/// Every ruler's height in points: [`RULER_H`] U cut to whole device pixels (140 px at U = 85).
pub fn ruler_h(m: &Metrics) -> f32 {
    (RULER_H * m.u_px()).trunc() / m.ppp()
}

/// The ruler hairline q = ceil(0.0375·U) px, in points.
pub fn q(m: &Metrics) -> f32 {
    (Q_U * m.u_px()).ceil() / m.ppp()
}

/// The ruler `rect`'s baseline top: the line is ceil(1.5·q) px tall, thicker than the hairline q, its bottom q above
/// the ruler's bottom.
fn baseline_top(rect: Rect, m: &Metrics) -> f32 {
    let q_px = (Q_U * m.u_px()).ceil();
    rect.bottom() - (q_px + (1.5 * q_px).ceil()) / m.ppp()
}

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
/// beats, Next until at least m, clamp(d, 2, 4)·m and 0.5·clamp(d, 2, 4)·clamp(b, 2, 4)·m.
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
        grow(b as f64, 0.5 * cd * cb * m),
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

/// Paints the ruler over `rect` (module docs): its fill and baseline, then its ticks and labels clipped to `marks` (all
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
    let (c, q) = (&cx.theme.colors, q(cx.m));
    painter.rect_filled(rect, 0.0, c.background.to_color32());
    let base = Rect::from_x_y_ranges(rect.x_range(), baseline_top(rect, cx.m)..=rect.bottom() - q);
    painter.rect_filled(base, 0.0, c.menu_fill.to_color32());
    ruler_marks(painter, rect, marks, x0, beat_w, sig, cx);
}

/// The ruler's ticks and bar labels ([`ruler`]), clipped to `marks`.
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
    let ink = c.menu_fill.to_color32();
    let label = c.text.to_color32().gamma_multiply(LABEL_ALPHA);
    let q = q(m);
    let d = sig.steps_per_beat();
    let step_w = beat_w / d as f32;
    let s = spacings(f64::from(m.u(MARK_U) / beat_w), sig);
    let at = |x: f32| f64::from((x - x0) / step_w);
    let (from, to) = (
        at(marks.left()).floor() as i64,
        at(marks.right()).ceil() as i64,
    );
    let baseline = baseline_top(rect, m);
    let tall = baseline - rect.top();
    let font = m.font_id(FontSize::Pt8);
    let label_top = rect.top() + LABEL_Y * (rect.height() - 2.0 * q);
    let bar = i64::from(d * sig.bar_beats());
    for (j, level) in self::marks(from, to, s, d) {
        let x = x0 + j as f32 * step_w;
        let h = match level {
            Level::Top => TOP_TICK,
            Level::Mid => MID_TICK,
            Level::Fine => FINE_TICK,
        };
        let tick = Rect::from_x_y_ranges(x - q / 2.0..=x + q / 2.0, baseline - h * tall..=baseline);
        painter.rect_filled(tick, 0.0, ink);
        if level == Level::Top && x >= marks.left() {
            let number = (j / bar + 1).to_string();
            let at = pos2(x + m.u(LABEL_X), label_top);
            painter.text(at, Align2::LEFT_TOP, number, font.clone(), label);
        }
    }
}

/// The ruler `rect`'s scroll thumb (display only; a touch on it is a ruler touch), the ruler on `axis` showing ticks
/// `from` to `from + len` of content: while the ruler spans less than that, a `text` α 0.3 bar t tall, t below the
/// ruler top, on a track from the ruler's left to 0.15 U short of its right: span/length of the track wide (at least
/// t), at (view start − from)/length along it, clamped inside the track.
pub fn scroll_thumb(
    painter: &Painter,
    rect: Rect,
    axis: TimeAxis,
    (from, len): (f64, f64),
    cx: &UiCtx,
) {
    let t = cx.m.t();
    let (start, span) = (
        axis.tick(rect.left()),
        axis.tick(rect.right()) - axis.tick(rect.left()),
    );
    // Shown only while the ruler spans less than the content (never for a NaN span).
    let shown = span < len && len > 0.0;
    if !shown {
        return;
    }
    let track = (rect.width() - cx.m.u(THUMB_END)).max(0.0);
    let w = (track * (span / len) as f32).clamp(t.min(track), track);
    let x = rect.left() + (track * ((start - from) / len) as f32).clamp(0.0, track - w);
    let bar = Rect::from_min_size(pos2(x, rect.top() + t), vec2(w, t));
    let ink = cx
        .theme
        .colors
        .text
        .to_color32()
        .gamma_multiply(THUMB_ALPHA);
    painter.rect_filled(bar, 0.0, ink);
}

/// The playhead line, the same in every editor grid: at `x`, from the top of `grid` to its bottom, a `text` line
/// ceil(1.5·t) px wide on a `background` α 0.3 shadow 1 px wider each side (t = ceil(U/16) px), on whole device pixels
/// about the pixel edge nearest `x` (an odd width's extra pixel right of it). It is drawn whenever `x` lies within the
/// grid's width, stopped or playing; `painter` clips it (the playlist's lets its left half show in the gap after the
/// header column). The ruler part is the play box's.
pub fn playhead_line(painter: &Painter, grid: Rect, x: f32, cx: &UiCtx) {
    painter.extend(playhead_line_shapes(grid, x, cx));
}

/// The shapes of [`playhead_line`], none while `x` is off the grid: for a paint slot reserved under other shapes.
pub fn playhead_line_shapes(grid: Rect, x: f32, cx: &UiCtx) -> Vec<Shape> {
    if !grid.x_range().contains(x) {
        return Vec::new();
    }
    line_shapes(x, grid.y_range(), 1.0, cx)
}

/// The playhead line's two bands at `x` over `ys` ([`playhead_line`]), at `alpha`.
fn line_shapes(x: f32, ys: Rangef, alpha: f32, cx: &UiCtx) -> Vec<Shape> {
    let (c, m) = (&cx.theme.colors, cx.m);
    let (ppp, t) = (m.ppp(), m.t() * m.ppp());
    let w = (1.5 * t).ceil();
    let left = (x * ppp).round() - (w / 2.0).floor();
    let band = |from: f32, w: f32, color: Color32| {
        let xs = from / ppp..=(from + w) / ppp;
        Shape::rect_filled(Rect::from_x_y_ranges(xs, ys), 0.0, color)
    };
    let shadow = c
        .background
        .to_color32()
        .gamma_multiply(SHADOW_ALPHA * alpha);
    let text = c.text.to_color32().gamma_multiply(alpha);
    vec![band(left - 1.0, w + 2.0, shadow), band(left, w, text)]
}

/// The play box's fade target in every editor: shown only while the transport is stopped and no floating row (`menu`)
/// shows, so while playing only the grid line shows. The ruler's keyboard focus is no condition here: no ruler takes
/// the focus (`ui::focus`).
pub fn knob_shown(playing: bool, menu: bool) -> bool {
    !playing && !menu
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

/// The beat a dragged play box shows and its drop seeks to, as a song tick: the finger's `tick` clamped between the
/// view-left tick `left` and the range end `end`, then round(tick/unit)·unit on the `grid` (whole ticks without one,
/// Snap off), never past the end.
pub fn knob_seek(tick: f64, grid: Option<f64>, left: f64, end: f64) -> u64 {
    let end = end.max(0.0);
    snap_round(tick.max(left).min(end), grid).clamp(0.0, end) as u64
}

impl RangeRuler {
    /// The tick a play box dropped at `x` on this ruler seeks to, in the axis's ticks: [`knob_seek`] from the ruler's
    /// left edge to `end`, on its grid. The drum editors pass s + L; the piano roll and the automation and audio
    /// editors pass L − 1 tick in content ticks (their playhead is folded into the loop, where L is its start again).
    pub fn knob_tick(&self, x: f32, end: f64) -> u64 {
        knob_seek(
            self.axis.tick(x),
            self.grid,
            self.axis.tick(self.strip.left()),
            end,
        )
    }
}

/// What the play box did this frame.
#[derive(Clone, Copy, Debug, PartialEq)]
pub enum KnobGesture {
    /// A tap: start playback.
    Tap,
    /// A drag released with the playhead at this x (the finger less its grab offset): the host seeks to the beat under
    /// it ([`knob_seek`]).
    Drop(f32),
}

/// The play box: the playhead's handle on the ruler, right of the playhead line.
/// - Its box: left edge on the playhead x, 1.72 U wide, h − 0.16 U tall, 0.08 U in from the ruler's top and bottom,
///   square left corners and 0.15 U right ones. `menuFill`, with icon 3 (play) in `text` at α 0.85·a, scale 0.82; with
///   record armed icon 5 in `recordColor` at α a on 0.78·`menuFill` + 0.22·`recordColor`. It draws the ruler part of
///   the playhead line on itself ([`playhead_line`]'s bands), from its top to 0.175 h below the ruler. Hovered, the
///   fill is ×1.25. Clipped to the ruler's width at its right end.
/// - It keeps its size pressed, hovered, dragged and fading: no hover scale, press growth or wobble.
/// - It fades toward the host's target ([`knob_shown`]): a ← a + (target − a)(1 − e^(−13.33·Δt)), snapping within
///   0.001, tracking the live playhead as it fades (jumping with SETTINGS `Animation` reduced). At a = 0 it takes no
///   touch.
/// - Touch area: 0.56 U left to 2.29 U right of the playhead x, 0.29 U above to 0.29 U below the ruler. A tap
///   (released within 0.3 s, within the 0.5 U calm zone: the shared law, [`gesture`]) is [`KnobGesture::Tap`].
/// - Moving more than 0.5 U sideways makes the press a drag: the playhead follows the finger less its grab offset (the
///   box does not jump under the finger; the host maps [`PlayheadKnob::dragging`] to the snapped beat), and the
///   release is [`KnobGesture::Drop`]. A press that moved only up or down does nothing.
///
/// Neither drawn nor touched while the playhead is off the ruler; a press held over a focus loss does nothing.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct PlayheadKnob {
    /// Record is armed: the box shows icon 5 (REC) in place of 3 (play). The host sets it each frame.
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
    /// The finger's x less the playhead's at the press: kept through a drag.
    grab: f32,
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
    /// While the box is dragged: the playhead's x this frame, the finger's less the grab offset. The host maps it to a
    /// beat ([`knob_seek`]) and passes that beat's x to [`Self::show`], so that the line and the box both follow the
    /// finger on the snap grid.
    pub fn dragging(&self, ui: &egui::Ui, m: &Metrics) -> Option<f32> {
        let press = self.press?;
        // `interact_pos`, not `latest_pos`: a touch's lift clears the latter in its release frame (`PointerGone`).
        let x = ui.input(|i| i.pointer.interact_pos())?.x;
        // From the frame the finger first goes more than 0.5 U sideways, as [`Self::show`] decides it.
        (press.drag || (x - press.origin.x).abs() > m.u(gesture::CALM_U)).then_some(x - press.grab)
    }

    /// The x the host draws its playhead (line and box) at on `at`, `end` being the drop's upper clamp
    /// ([`RangeRuler::knob_tick`]):
    /// - while the box is dragged, the tick its drop would seek to, so that the line and the box follow the finger;
    /// - after a drop ([`Self::drop_tick`]), that tick, until the output reports a tick in its beat (`at.playhead`) or
    ///   for a quarter second: the output reports the seek a frame or more later, and meanwhile the box must not
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

    /// The tick a drop at `x` ([`KnobGesture::Drop`]) on `at` seeks to, up to `end`; [`Self::line_x`] shows the
    /// playhead there until the output reports it.
    pub fn drop_tick(&mut self, ui: &egui::Ui, at: &RangeRuler, end: f64, x: f32) -> u64 {
        let tick = at.knob_tick(x, end);
        self.dropped = Some((tick, ui.input(|i| i.time)));
        tick
    }

    /// The box on `ruler` (the time ruler) at the playhead `x`, given every frame (while it is dragged, the host's
    /// mapping of [`Self::dragging`]); `shown` is the fade's target.
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
        let live = on && (self.alpha > 0.0 || self.press.is_some());
        let hit = Rect::from_x_y_ranges(
            x - m.u(BOX_TOUCH_LEFT)..=x + m.u(BOX_TOUCH_RIGHT),
            ruler.top() - m.u(BOX_TOUCH_Y)..=ruler.bottom() + m.u(BOX_TOUCH_Y),
        );
        let response = live.then(|| ui.interact(hit, id, Sense::click_and_drag()));
        if pressed
            && response
                .as_ref()
                .is_some_and(|r| r.is_pointer_button_down_on())
            && let Some(origin) = pos
        {
            self.press = Some(KnobPress {
                origin,
                grab: origin.x - x,
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
                        pos.map(|p| KnobGesture::Drop(p.x - press.grab))
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
        let next = self.alpha + (target - self.alpha) * anim::step(ui.ctx(), FADE_KEEP);
        self.alpha = if (target - next).abs() < FADE_SNAP {
            target
        } else {
            next
        };
        if self.alpha != target {
            ui.ctx().request_repaint();
        }
        if self.alpha > 0.0 && on {
            let hovered = response.is_some_and(|r| r.hovered());
            self.paint(ui, ruler, x, hovered, cx);
        }
        out
    }

    /// The box at the playhead `x` on `ruler`, hovered or not (type docs).
    fn paint(&self, ui: &egui::Ui, ruler: Rect, x: f32, hovered: bool, cx: &UiCtx) {
        let (c, m, a) = (&cx.theme.colors, cx.m, self.alpha);
        let inset = m.u(BOX_INSET);
        let rect = Rect::from_x_y_ranges(
            x..=x + m.u(BOX_W),
            ruler.top() + inset..=ruler.bottom() - inset,
        );
        let menu_fill = c.menu_fill.to_color32();
        let (fill, icon, ink) = if self.armed {
            let fill = crate::ui::clips::mix(c.menu_fill, RECORD_COLOR, BOX_ARMED_MIX);
            let ink = RECORD_COLOR.to_color32().gamma_multiply(a);
            (fill.to_color32(), IconId::Record, ink)
        } else {
            let ink = c.text.to_color32().gamma_multiply(BOX_ICON_ALPHA * a);
            (menu_fill, IconId::Play, ink)
        };
        let fill = if hovered {
            widgets::brighter(fill, 1.25)
        } else {
            fill
        };
        let r = m.u(BOX_R) as u8;
        let radius = CornerRadius {
            nw: 0,
            sw: 0,
            ne: r,
            se: r,
        };
        // Clipped to the ruler's width, and down to the line's end below it.
        let below = ruler.bottom() + BOX_LINE_BELOW * ruler.height();
        let clip = Rect::from_x_y_ranges(ruler.x_range(), ruler.top()..=below);
        let painter = ui.painter().with_clip_rect(ui.clip_rect().intersect(clip));
        painter.rect_filled(rect, radius, fill.gamma_multiply(a));
        painter.extend(line_shapes(x, Rangef::new(rect.top(), below), a, cx));
        draw_icon(&painter, rect, icon, ink, BOX_ICON, m);
    }
}

/// Finding the play box in a frame's shapes, for the editors' tests.
#[cfg(test)]
pub(crate) mod testing {
    use eframe::egui::epaint::{ClippedShape, RectShape};
    use eframe::egui::{Rect, Shape};

    use crate::theme::RECORD_COLOR;

    /// The play boxes among `shapes`, in paint order: the rects with square left and rounded right corners.
    pub(crate) fn play_boxes(shapes: &[ClippedShape]) -> impl Iterator<Item = &RectShape> {
        shapes.iter().filter_map(|s| match &s.shape {
            Shape::Rect(r) if r.corner_radius.nw == 0 && r.corner_radius.ne > 0 => Some(r),
            _ => None,
        })
    }

    /// The playhead x a play box `rect` shows at U = `u`: its centre less half its width.
    pub(crate) fn playhead_x(rect: Rect, u: f32) -> f32 {
        rect.center().x - u * super::BOX_W / 2.0
    }

    /// The playhead x a playhead line's band `rect` (the line or its shadow) shows at `ppp`: the pixel edge it is drawn
    /// about, an odd width's extra pixel right of it.
    pub(crate) fn line_x(rect: Rect, ppp: f32) -> f32 {
        rect.left() + (rect.width() * ppp / 2.0).floor() / ppp
    }

    /// The first play box's playhead x ([`playhead_x`]) at U = `u`, if one is drawn.
    pub(crate) fn play_box_x(shapes: &[ClippedShape], u: f32) -> Option<f32> {
        play_boxes(shapes).next().map(|r| playhead_x(r.rect, u))
    }

    /// The first play box shows the REC icon (a `recordColor` disc on it), not the play icon.
    pub(crate) fn play_box_armed(shapes: &[ClippedShape]) -> bool {
        let Some(rect) = play_boxes(shapes).next().map(|r| r.rect) else {
            return false;
        };
        let rec = RECORD_COLOR.to_color32();
        (shapes.iter()).any(
            |s| matches!(&s.shape, Shape::Circle(c) if c.fill == rec && rect.contains(c.center)),
        )
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

    /// The playhead line every editor grid shares.
    mod line {
        use eframe::egui::{Context, LayerId, Pos2, RawInput, Rect, Shape, pos2, vec2};

        use super::super::playhead_line;
        use crate::metrics::{DEFAULT_UI_SCALE, Metrics};
        use crate::theme::theme_by_id;
        use crate::ui::UiCtx;

        const GRID: Rect = Rect {
            min: pos2(100.0, 60.0),
            max: pos2(900.0, 400.0),
        };

        /// How many bands `playhead_line` paints at `x`.
        fn bands(x: f32) -> usize {
            let m = Metrics::from_pixels_per_point(1.0, DEFAULT_UI_SCALE);
            let theme = theme_by_id(0);
            let raw = RawInput {
                screen_rect: Some(Rect::from_min_size(Pos2::ZERO, vec2(1000.0, 500.0))),
                ..Default::default()
            };
            let out = Context::default().run(raw, |ctx| {
                let painter = ctx.layer_painter(LayerId::background());
                let cx = UiCtx {
                    theme: &theme,
                    m: &m,
                };
                playhead_line(&painter, GRID, x, &cx);
            });
            (out.shapes.iter())
                .filter(|s| matches!(s.shape, Shape::Rect(_)))
                .count()
        }

        #[test]
        fn it_shows_only_while_its_x_lies_on_the_grid() {
            assert_eq!(bands(GRID.left()), 2);
            assert_eq!(bands(GRID.right()), 2);
            assert_eq!(bands(GRID.left() - 0.5), 0);
            assert_eq!(bands(GRID.right() + 0.5), 0);
        }
    }

    /// The play box.
    mod knob {
        use eframe::egui::{Event, Id, Modifiers, PointerButton, Pos2, Rect, Shape, pos2, vec2};

        use super::super::{KnobGesture, PlayheadKnob, knob_seek};
        use crate::ui::offscreen::Offscreen;

        /// U = 32: the playlist's box on its 1.65 U time ruler from x 48, riding a playhead at x 200; a press at AT is on
        /// the box's left end, the grab offset 4 points.
        const AT: Pos2 = pos2(204.0, 26.0);
        const RULER: Rect = Rect {
            min: pos2(48.0, 0.0),
            max: pos2(1170.0, 52.8),
        };
        const U: f32 = 32.0;

        struct Rig {
            screen: Offscreen,
            knob: PlayheadKnob,
            /// The live playhead's x, and the fade's target.
            x: f32,
            shown: bool,
            outs: Vec<KnobGesture>,
            /// The last frame's boxes (`menuFill` rects): left edge and alpha.
            boxes: Vec<(f32, u8)>,
        }

        impl Rig {
            fn new() -> Self {
                let mut rig = Self {
                    screen: Offscreen::new(),
                    knob: PlayheadKnob::default(),
                    x: 200.0,
                    shown: true,
                    outs: Vec::new(),
                    boxes: Vec::new(),
                };
                rig.frame(vec![]);
                rig
            }

            fn frame_after(&mut self, dt: f64, events: Vec<Event>) {
                self.screen.time += dt - 1.0 / 60.0;
                let (knob, x, shown) = (&mut self.knob, self.x, self.shown);
                let mut out = None;
                let shapes = self.screen.frame(events, &mut |ui, cx| {
                    // The host's side: a drag shows where the playhead goes (no grid here).
                    let x = knob.dragging(ui, cx.m).unwrap_or(x);
                    out = knob.show(ui, Id::new("knob"), RULER, x, shown, cx);
                });
                self.outs.extend(out);
                // The box is the one rect with rounded (right) corners; its left edge is the centre less half its
                // 1.72 U.
                self.boxes = (shapes.iter())
                    .filter_map(|s| match &s.shape {
                        Shape::Rect(r) if r.corner_radius.ne > 0 => {
                            Some(((r.rect.center().x - 0.86 * U).round(), r.fill.a()))
                        }
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

            /// The one box drawn: its left edge and alpha.
            fn drawn(&self) -> Option<(f32, u8)> {
                assert!(self.boxes.len() <= 1, "{:?}", self.boxes);
                self.boxes.first().copied()
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
            // Its touch area reaches 2.29 U right of the playhead and 0.56 U left; beyond it, nothing.
            let right = pos2(200.0 + 2.2 * U, AT.y);
            assert_eq!(rig.gesture(right, right, 0.1), [KnobGesture::Tap]);
            let off = pos2(200.0 + 2.6 * U, AT.y);
            assert_eq!(rig.gesture(off, off, 0.1), []);
            let left = pos2(200.0 - 0.9 * U, AT.y);
            assert_eq!(rig.gesture(left, left, 0.1), []);
            // A press held over a focus loss does nothing.
            rig.frame(vec![Event::PointerMoved(AT)]);
            rig.button_after(1.0 / 60.0, AT, true);
            rig.frame(vec![Event::WindowFocused(false)]);
            rig.button_after(1.0 / 60.0, AT, false);
            assert_eq!(rig.outs, []);
        }

        #[test]
        fn a_second_finger_cancels_a_drag_which_then_drops_nothing() {
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
            let to = pos2(304.0, AT.y);
            rig.frame(vec![touch(1, Move, to), Event::PointerMoved(to)]);
            assert_eq!(rig.drawn().unwrap().0, 300.0);
            rig.frame(vec![touch(2, Start, pos2(600.0, 30.0))]);
            rig.frame(vec![touch(2, End, pos2(600.0, 30.0))]);
            // Back on the live playhead, and the lift drops nothing.
            assert_eq!(rig.drawn().unwrap().0, 200.0);
            rig.frame(vec![touch(1, Move, to), Event::PointerMoved(to)]);
            rig.frame(vec![
                touch(1, End, to),
                button(to, false),
                Event::PointerGone,
            ]);
            assert_eq!(rig.drawn().unwrap().0, 200.0);
            assert_eq!(rig.outs, []);
        }

        #[test]
        fn a_sideways_drag_keeps_the_grab_offset_and_drops_the_playhead_where_it_shows() {
            let mut rig = Rig::new();
            // Pressed 1 U into the box: the box keeps that offset to the finger.
            let at = pos2(200.0 + U, AT.y);
            rig.frame(vec![Event::PointerMoved(at)]);
            rig.button_after(1.0 / 60.0, at, true);
            // 0.5 U is no drag yet: the box stays.
            rig.frame(vec![Event::PointerMoved(at + vec2(0.5 * U, 0.0))]);
            assert_eq!(rig.drawn().unwrap().0, 200.0);
            rig.frame(vec![Event::PointerMoved(pos2(332.0, 40.0))]);
            assert_eq!(rig.drawn().unwrap().0, 300.0);
            // However long it took.
            rig.button_after(1.0, pos2(342.0, 40.0), false);
            assert_eq!(rig.outs, [KnobGesture::Drop(310.0)]);
            // Then the box is the playhead's again.
            rig.frame(vec![]);
            assert_eq!(rig.drawn().unwrap().0, 200.0);
        }

        #[test]
        fn the_box_fades_by_the_frame_time_law_tracking_the_live_playhead() {
            // A fifth of the way a frame (the tests' frames are 1/60 s), snapping within 0.001.
            let mut rig = Rig::new();
            assert_eq!(rig.drawn(), Some((200.0, 255)));
            // Its target off (playback started): it fades out tracking the live playhead.
            rig.shown = false;
            rig.frame(vec![]);
            let a = rig.drawn().unwrap();
            assert!(a.0 == 200.0 && a.1.abs_diff(204) <= 1, "{a:?}");
            rig.x = 260.0;
            rig.frame(vec![]);
            let a = rig.drawn().unwrap();
            assert!(a.0 == 260.0 && a.1.abs_diff(164) <= 1, "{a:?}");
            for _ in 0..60 {
                rig.frame(vec![]);
            }
            assert_eq!(rig.drawn(), None);
            // At a = 0 it takes no tap.
            assert_eq!(rig.gesture(pos2(264.0, AT.y), pos2(264.0, AT.y), 0.1), []);
            // Back on, it fades in at the playhead.
            (rig.x, rig.shown) = (300.0, true);
            rig.frame(vec![]);
            let a = rig.drawn().unwrap();
            assert!(a.0 == 300.0 && a.1.abs_diff(51) <= 1, "{a:?}");
            // Off the ruler it is neither drawn nor touched, whatever its alpha.
            let mut rig = Rig::new();
            rig.x = 20.0;
            rig.frame(vec![]);
            assert_eq!(rig.drawn(), None);
            assert_eq!(rig.gesture(pos2(24.0, AT.y), pos2(24.0, AT.y), 0.1), []);
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

    mod law {
        use eframe::egui::{Context, LayerId, RawInput, Rect, Shape, pos2, vec2};

        use super::super::{Level, marks, next, ruler, spacings};
        use crate::metrics::{DEFAULT_UI_SCALE, Metrics};
        use crate::song::TimeSig;
        use crate::theme::theme_by_id;
        use crate::ui::UiCtx;

        const SIG: TimeSig = TimeSig {
            subdivision: 4,
            beats_per_bar: 4,
        };

        /// The bar labels a ruler across 40 U paints at U = 32 whose marks start at `left` U, tick 0 at `x0` U and a
        /// beat `beat_u` U wide.
        fn labels(left: f32, x0: f32, beat_u: f32) -> Vec<String> {
            let m = Metrics::from_pixels_per_point(1.0, DEFAULT_UI_SCALE);
            let theme = theme_by_id(0);
            let rect = Rect::from_min_size(pos2(0.0, 0.0), vec2(m.u(40.0), m.u(1.65)));
            let raw = RawInput {
                screen_rect: Some(rect),
                ..Default::default()
            };
            let output = Context::default().run(raw, |ctx| {
                let painter = ctx.layer_painter(LayerId::background());
                let cx = UiCtx {
                    theme: &theme,
                    m: &m,
                };
                let marks = rect.with_min_x(m.u(left));
                ruler(&painter, rect, marks, m.u(x0), m.u(beat_u), SIG, &cx);
            });
            (output.shapes.into_iter())
                .filter_map(|s| match s.shape {
                    Shape::Text(t) => Some(t.galley.text().to_owned()),
                    _ => None,
                })
                .collect()
        }

        #[test]
        fn labels_show_every_bar_at_the_editors_opening_zooms_and_never_left_of_the_marks() {
            // At 4, 2 and 1.2 U a beat s2 is one bar; at 0.8 U, two bars (the top bound is halved).
            assert_eq!(labels(0.0, 0.0, 4.0), ["1", "2", "3"]);
            assert_eq!(labels(0.0, 0.0, 2.0), ["1", "2", "3", "4", "5", "6"]);
            assert_eq!(labels(0.0, 0.0, 0.8).len(), 7);
            // A bar line left of the marks' left edge (the playlist's header column) has none; one on it has.
            assert_eq!(labels(1.5, 1.4, 2.0)[0], "2");
            assert_eq!(labels(1.5, 1.5, 2.0)[0], "1");
        }

        #[test]
        fn the_spacings_clamp_d_and_b_to_2_4_and_halve_the_top_bound() {
            // s1's bound is clamp(d, 2, 4)·m, s2's 0.5·clamp(d, 2, 4)·clamp(b, 2, 4)·m.
            let sig = |d, b| TimeSig {
                subdivision: d,
                beats_per_bar: b,
            };
            assert_eq!(spacings(0.6 * 0.33, sig(8, 4)), [0.25, 1.0, 4.0]);
            assert_eq!(spacings(0.6, sig(4, 7)), [1.0, 7.0, 7.0]);
            assert_eq!(spacings(0.6, sig(4, 4)), [1.0, 4.0, 8.0]);
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
            assert_eq!(spacings(0.6 * 0.33, sig(4, 4)), [0.25, 1.0, 4.0]);
            assert_eq!(spacings(0.06, sig(4, 4)), [0.25, 1.0, 4.0]);
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
