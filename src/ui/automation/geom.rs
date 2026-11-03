//! The frame the automation editor and the audio clip editor share: the ruler row and its Snap cell, the left column,
//! the grid and its value area of 127 fixed rows, the horizontal view (zoom limits, scroll range, the initial view),
//! the playhead and the value axis.
//!
//! The top margin is 0 here, as in the piano roll and the drum editor: the host passes the rect below its window
//! margin. The zoom limits take d, [`TimeSig::steps_per_beat`] (TDIV byte 0). Rows and values are
//! [`crate::song::points::row`] / [`crate::song::points::value_at_row`]; this module turns rows into y.

use eframe::egui::{Pos2, Rangef, Rect, pos2, vec2};

use crate::metrics::Metrics;
use crate::song::points::{self, Point};
use crate::song::{PPQ, TimeSig};
use crate::ui::clips::TimeAxis;
use crate::ui::scroller::{Axis, Zones};
use crate::ui::timeline::ruler_h;

/// The left column's width (the Snap cell, the value axis), in U. The ruler row is [`ruler_h`] tall.
pub const BAR: f32 = 1.5;
/// A touch finds the nearest point within this many U.
pub const HIT_R: f32 = 1.25;
/// A clip with no saved view opens at this many U per beat.
const NEW_BEAT_U: f32 = 4.0;
/// The value range is always 127 rows of V/127.
const ROWS: f64 = 127.0;

/// The editor's rects. `ruler` is the whole ruler row (the `background` rect over the margin and the ruler); `snap`
/// its left cell, 1.5 U × (ruler h − 1 px); `time` the time ruler, from x + 1.5 U + ceil(U/16). Below the ruler: `grid`
/// (G high) from the same x to the right edge, `value` the grid inset by pad at top and bottom (V high), `column` the
/// 1.5 U column left of the grid (G high) and `axis` its value-axis part (V high).
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Frame {
    pub ruler: Rect,
    pub snap: Rect,
    pub time: Rect,
    pub grid: Rect,
    pub value: Rect,
    pub column: Rect,
    pub axis: Rect,
}

/// One value-axis tick: a 2 px line across `line` at `y`, and its `label` centred in `text`.
#[derive(Clone, Debug, PartialEq)]
pub struct AxisTick {
    pub y: f32,
    pub line: Rangef,
    pub label: String,
    pub text: Rect,
}

impl Frame {
    /// The frame of an editor in `rect`. G = h − ruler, pad = clamp(min(0.5 U, G − 3.5 U), 0, ∞), V = G − 2·pad.
    pub fn new(rect: Rect, m: &Metrics) -> Self {
        let (u, bar, h) = (m.u(1.0), m.u(BAR), ruler_h(m));
        let ruler = Rect::from_x_y_ranges(rect.x_range(), rect.top()..=rect.top() + h);
        let left = (rect.left() + bar + m.t()).min(rect.right());
        let g = (rect.bottom() - ruler.bottom()).max(0.0);
        let pad = (0.5 * u).min(g - 3.5 * u).max(0.0);
        let below = Rangef::new(ruler.bottom(), ruler.bottom() + g);
        let value = Rangef::new(below.min + pad, below.max - pad);
        let column = Rangef::new(rect.left(), rect.left() + bar);
        Self {
            ruler,
            snap: Rect::from_min_size(rect.min, vec2(bar, h - 1.0 / m.ppp())),
            time: Rect::from_x_y_ranges(left..=rect.right(), ruler.y_range()),
            grid: Rect::from_x_y_ranges(left..=rect.right(), below),
            value: Rect::from_x_y_ranges(left..=rect.right(), value),
            column: Rect::from_x_y_ranges(column, below),
            axis: Rect::from_x_y_ranges(column, value),
        }
    }

    /// The y of row `r`: top + r·V/127. Touches find points here (r without the +0.5).
    pub fn row_y(&self, r: f64) -> f32 {
        self.value.top() + (r * f64::from(self.value.height()) / ROWS) as f32
    }

    /// The row at `y`: (y − top)·127/V, what a tap creates a point from.
    pub fn row_at(&self, y: f32) -> f64 {
        f64::from(y - self.value.top()) * ROWS / f64::from(self.value.height())
    }

    /// The drawn centre y of a point of `value`: top + (r + 0.5)·V/127, r = its row of `max`.
    pub fn point_y(&self, value: u16, max: u16) -> f32 {
        self.row_y(points::row(value, max) + 0.5)
    }

    /// The point a touch at `p` binds: the nearest within 1.25 U, at (x of its tick, [`Self::row_y`] of its row).
    pub fn hit(
        &self,
        axis: TimeAxis,
        pts: &[Point],
        max: u16,
        p: Pos2,
        m: &Metrics,
    ) -> Option<usize> {
        let at = |q: &Point| pos2(axis.x(q.tick.into()), self.row_y(points::row(q.value, max)));
        (pts.iter().enumerate())
            .map(|(i, q)| (i, at(q).distance(p)))
            .filter(|&(_, d)| d <= m.u(HIT_R))
            .min_by(|a, b| a.1.total_cmp(&b.1))
            .map(|(i, _)| i)
    }

    /// The value axis: N = 5 ticks when V ≥ 3 U, else 3; tick i at top + V·i/(N − 1), a line from x + 3 px to
    /// x + w − 3 px, labelled 100 − 100·i/(N − 1) in (x + 3 px, y − 0.5 U, w − 6 px, 0.5 U).
    pub fn axis_ticks(&self, m: &Metrics) -> Vec<AxisTick> {
        let n = if self.axis.height() >= m.u(3.0) { 5 } else { 3 };
        (0..n)
            .map(|i| {
                let label = (100 - 100 * i / (n - 1)).to_string();
                self.axis_tick(1.0 - i as f32 / (n - 1) as f32, label, m)
            })
            .collect()
    }

    /// The axis tick at fraction `f` of the range (1 at the top), y = top + V·(1 − f), labelled `label`.
    pub fn axis_tick(&self, f: f32, label: String, m: &Metrics) -> AxisTick {
        let (a, px) = (self.axis, 1.0 / m.ppp());
        let y = a.top() + a.height() * (1.0 - f);
        AxisTick {
            y,
            line: Rangef::new(a.left() + 3.0 * px, a.right() - 3.0 * px),
            label,
            text: Rect::from_min_size(
                pos2(a.left() + 3.0 * px, y - m.u(0.5)),
                vec2(a.width() - 6.0 * px, m.u(0.5)),
            ),
        }
    }
}

/// The beat widths (points per beat) the zoom allows: (12/U)·4/d … (0.01/U)·4/d beats per point, so U·d/48 …
/// 25·U·d points per beat, d = `sig`'s subdivision, TDIV byte 0.
pub fn zoom_limits(u: f32, sig: TimeSig) -> [f32; 2] {
    let ud = u * sig.steps_per_beat() as f32;
    [ud / 48.0, ud * 25.0]
}

/// The horizontal view: the beat at the grid's left edge and the beat width in points.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct View {
    pub left: f64,
    pub beat_w: f32,
}

impl View {
    /// A clip with no saved view: from its first point's beat (`first`, a tick; else 0), at 4 U per beat.
    pub fn initial(first: Option<u32>, u: f32) -> Self {
        Self {
            left: f64::from(first.unwrap_or(0)) / f64::from(PPQ),
            beat_w: NEW_BEAT_U * u,
        }
    }

    /// The time axis of a grid whose left edge is at `grid_left`.
    pub fn axis(self, grid_left: f32) -> TimeAxis {
        TimeAxis {
            origin: grid_left - (self.left * f64::from(self.beat_w)) as f32,
            beat_w: self.beat_w,
        }
    }
}

/// The automation and audio editors' input zones: one finger scrolls the time on the ruler band only (no touch y); the
/// wheel zooms it over the ruler and the grid (the zoom-x zone covers both and comes before the scroll zone), where a
/// trackpad pinch zooms it too.
pub fn zones(f: &Frame) -> Zones {
    Zones {
        touch: [f.time, Rect::NOTHING],
        wheel_zoom: [f.time.union(f.grid), Rect::NOTHING],
        wheel_scroll: f.grid,
        pinch: f.grid,
    }
}

/// `view` as the scroller's x axis over the `grid`: beats over 0 … twice the loop (`loop_beats`), within the zoom
/// limits.
pub fn axis(view: View, u: f32, sig: TimeSig, loop_beats: f64, grid: Rect) -> Axis {
    let [lo, hi] = zoom_limits(u, sig);
    Axis {
        pos: view.left,
        zoom: 1.0 / f64::from(view.beat_w),
        range: [0.0, 2.0 * loop_beats],
        zoom_range: [1.0 / f64::from(hi), 1.0 / f64::from(lo)],
        view: grid.x_range(),
    }
}

/// `view` with what changed in its axis since `was` ([`axis`]).
pub fn take(view: View, was: Axis, x: Axis) -> View {
    View {
        left: if x.pos != was.pos { x.pos } else { view.left },
        beat_w: if x.zoom != was.zoom {
            (1.0 / x.zoom) as f32
        } else {
            view.beat_w
        },
    }
}

/// The playhead shown: content tick `tick` wrapped modulo the loop (`loop_ticks`; none below a tick).
pub fn playhead(tick: f64, loop_ticks: f64) -> f64 {
    if loop_ticks >= 1.0 {
        tick.rem_euclid(loop_ticks)
    } else {
        tick
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const MAX: u16 = 65534;

    /// U = 32 points at both densities (the desktop boost), so the frames match but for the 1 px Snap cell gap.
    fn metrics(ppp: f32) -> Metrics {
        let m = Metrics::from_pixels_per_point(ppp, 100);
        assert_eq!(m.u(1.0), 32.0);
        m
    }

    /// The ruler's bottom in [`frame`]: everything below it lies on whole points.
    const RULER_BOTTOM: f32 = 68.0;

    /// The ruler's top in [`frame`] at `ppp` (the ruler's height above [`RULER_BOTTOM`]).
    fn top(ppp: f32) -> f32 {
        RULER_BOTTOM - ruler_h(&metrics(ppp))
    }

    /// The frame of an editor from x 10, 400 wide, its ruler ending at y [`RULER_BOTTOM`] and its rect at 20 + `h`
    /// (G = `h` − 48).
    fn frame(h: f32, ppp: f32) -> Frame {
        Frame::new(
            Rect::from_min_max(pos2(10.0, top(ppp)), pos2(410.0, 20.0 + h)),
            &metrics(ppp),
        )
    }

    #[test]
    fn the_frame_at_two_sizes_and_densities() {
        for ppp in [1.0, 2.0] {
            let px = 1.0 / ppp;
            let f = frame(300.0, ppp);
            let h = ruler_h(&metrics(ppp));
            assert_eq!(
                f.ruler,
                Rect::from_min_max(pos2(10.0, top(ppp)), pos2(410.0, 68.0))
            );
            assert_eq!(
                f.snap,
                Rect::from_min_size(pos2(10.0, top(ppp)), vec2(48.0, h - px))
            );
            // x + 1.5 U + ceil(U/16): 2 points at either density.
            assert_eq!(
                f.time,
                Rect::from_min_max(pos2(60.0, top(ppp)), pos2(410.0, 68.0))
            );
            // G = 252, pad = 0.5 U, V = 220.
            assert_eq!(
                f.grid,
                Rect::from_min_max(pos2(60.0, 68.0), pos2(410.0, 320.0))
            );
            assert_eq!(
                f.value,
                Rect::from_min_max(pos2(60.0, 84.0), pos2(410.0, 304.0))
            );
            assert_eq!(
                f.column,
                Rect::from_min_max(pos2(10.0, 68.0), pos2(58.0, 320.0))
            );
            assert_eq!(
                f.axis,
                Rect::from_min_max(pos2(10.0, 84.0), pos2(58.0, 304.0))
            );
            // G − 3.5 U = 8 < 0.5 U: pad 8.
            let f = frame(48.0 + 120.0, ppp);
            assert_eq!((f.value.top(), f.value.height()), (76.0, 104.0));
            // G < 3.5 U: no pad.
            let f = frame(48.0 + 90.0, ppp);
            assert_eq!(f.value, f.grid);
            // No room: nothing below the ruler.
            let f = frame(30.0, ppp);
            assert_eq!(f.grid.height(), 0.0);
        }
    }

    #[test]
    fn rows_and_ys() {
        let f = frame(48.0 + 16.0 + 254.0 + 16.0, 1.0);
        // V = 254: 2 points a row.
        assert_eq!((f.value.top(), f.value.height()), (84.0, 254.0));
        assert_eq!(f.row_y(0.0), 84.0);
        assert_eq!(f.row_y(126.0), 84.0 + 252.0);
        assert_eq!(f.row_at(84.0 + 254.0), 127.0);
        assert_eq!(f.row_at(f.row_y(63.5)), 63.5);
        // Drawn half a row below: the top value's centre is at row 0.5, 0's at row 126.5.
        assert_eq!(f.point_y(MAX, MAX), 85.0);
        assert_eq!(f.point_y(0, MAX), 84.0 + 253.0);
        assert_eq!(f.point_y(65535, 65535), 85.0);
        assert_eq!(f.point_y(32767, MAX), 84.0 + 127.0);
    }

    #[test]
    fn a_touch_binds_the_nearest_point_within_1_25_u() {
        let m = metrics(1.0);
        let f = frame(300.0, 1.0);
        let axis = View {
            left: 2.0,
            beat_w: 128.0,
        }
        .axis(f.grid.left());
        let pts = [
            Point {
                tick: 256,
                value: MAX,
                ..Default::default()
            },
            Point {
                tick: 288,
                value: MAX,
                ..Default::default()
            },
        ];
        // The first point sits at (60, 84) for touches: row 0, without the +0.5.
        let hit = |x: f32, y: f32| f.hit(axis, &pts, MAX, pos2(x, y), &m);
        assert_eq!(hit(60.0 - 40.0, 84.0), Some(0));
        assert_eq!(hit(60.0 - 40.01, 84.0), None);
        assert_eq!(hit(60.0, 84.0 + 40.0), Some(0));
        // 40.5 below the touch row but within 40 of the drawn centre: no hit.
        assert_eq!(hit(60.0, 84.0 + 40.5), None);
        // The second is 32 points right: the nearer wins.
        assert_eq!(hit(75.0, 90.0), Some(0));
        assert_eq!(hit(77.0, 90.0), Some(1));
        assert_eq!(f.hit(axis, &[], MAX, pos2(60.0, 84.0), &m), None);
    }

    #[test]
    fn the_value_axis_has_five_ticks_from_3_u() {
        for ppp in [1.0, 2.0] {
            let (m, px) = (metrics(ppp), 1.0 / ppp);
            let f = frame(300.0, ppp);
            let ticks = f.axis_ticks(&m);
            let ys: Vec<f32> = ticks.iter().map(|t| t.y).collect();
            assert_eq!(ys, [84.0, 139.0, 194.0, 249.0, 304.0]);
            let labels: Vec<&str> = ticks.iter().map(|t| t.label.as_str()).collect();
            assert_eq!(labels, ["100", "75", "50", "25", "0"]);
            assert_eq!(ticks[1].line, Rangef::new(10.0 + 3.0 * px, 58.0 - 3.0 * px));
            assert_eq!(
                ticks[1].text,
                Rect::from_min_size(
                    pos2(10.0 + 3.0 * px, 139.0 - 16.0),
                    vec2(48.0 - 6.0 * px, 16.0)
                )
            );
            // V = 96 = 3 U: still five; below: three.
            assert_eq!(frame(48.0 + 96.0, ppp).axis_ticks(&m).len(), 5);
            let ticks = frame(48.0 + 90.0, ppp).axis_ticks(&m);
            let labels: Vec<&str> = ticks.iter().map(|t| t.label.as_str()).collect();
            assert_eq!(labels, ["100", "50", "0"]);
            assert_eq!(ticks[1].y, 68.0 + 45.0);
        }
    }

    #[test]
    fn zoom_scroll_and_the_initial_view() {
        // (12/U)·4/d … (0.01/U)·4/d beats per point, d = TDIV byte 0: U·d/48 … 25·U·d points per beat.
        // Byte 1 (b) plays no part.
        let sig = |subdivision, beats_per_bar| TimeSig {
            subdivision,
            beats_per_bar,
        };
        assert_eq!(zoom_limits(32.0, sig(4, 4)), [32.0 * 4.0 / 48.0, 3200.0]);
        assert_eq!(zoom_limits(32.0, sig(4, 3)), zoom_limits(32.0, sig(4, 4)));
        assert_eq!(zoom_limits(32.0, sig(3, 4)), [2.0, 2400.0]);
        assert_eq!(zoom_limits(32.0, sig(8, 4)), [32.0 * 8.0 / 48.0, 6400.0]);
        assert_eq!(zoom_limits(32.0, sig(8, 7)), zoom_limits(32.0, sig(8, 4)));
        assert_eq!(zoom_limits(32.0, sig(0, 4)), zoom_limits(32.0, sig(1, 4)));
        // The first point's beat at 4 U per beat; beat 0 without points.
        let v = View::initial(Some(256), 32.0);
        assert_eq!((v.left, v.beat_w), (2.0, 128.0));
        assert_eq!(View::initial(None, 32.0).left, 0.0);
        // As the scroller's axis: scroll 0 … 2 × loop, 8 beats less the 350/128 shown.
        let grid = Rect::from_min_size(pos2(0.0, 0.0), vec2(350.0, 100.0));
        let clamped = |v: View, loop_beats| {
            let was = axis(v, 32.0, TimeSig::default(), loop_beats, grid);
            take(v, was, was.clamped())
        };
        let max = 8.0 - 350.0 / 128.0;
        assert_eq!(clamped(v, 4.0), v);
        assert!((clamped(View { left: 7.0, ..v }, 4.0).left - max).abs() < 1e-6);
        assert_eq!(clamped(View { left: -1.0, ..v }, 4.0).left, 0.0);
        // A grid wider than twice the loop: no scroll.
        assert_eq!(clamped(v, 1.0).left, 0.0);
        let wide = View {
            left: 0.0,
            beat_w: 1e6,
        };
        assert_eq!(clamped(wide, 4.0).beat_w, 3200.0);
        let narrow = View {
            left: 0.0,
            beat_w: 1.0,
        };
        assert_eq!(clamped(narrow, 4.0).beat_w, 32.0 / 12.0);
        // Its zones: one finger on the ruler band only; the wheel zooms over the ruler and the grid.
        let f = frame(300.0, 1.0);
        let z = zones(&f);
        assert_eq!(z.touch, [f.time, Rect::NOTHING]);
        assert_eq!(z.wheel_zoom, [f.time.union(f.grid), Rect::NOTHING]);
        assert_eq!((z.wheel_scroll, z.pinch), (f.grid, f.grid));
        // The axis puts the view's left beat on the grid's left edge.
        assert_eq!(v.axis(60.0).x(256.0), 60.0);
        assert_eq!(v.axis(60.0).x(384.0), 188.0);
    }

    #[test]
    fn the_playhead_wraps_modulo_the_loop() {
        assert_eq!(playhead(600.0, 512.0), 88.0);
        assert_eq!(playhead(512.0, 512.0), 0.0);
        assert_eq!(playhead(-10.0, 512.0), 502.0);
        assert_eq!(playhead(600.0, 0.0), 600.0);
    }
}
