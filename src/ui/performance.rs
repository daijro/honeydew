//! The performance monitor: the data the PROJECT page's Performance block shows, and its drawing. The block takes the
//! one [`Performance`], the shared data source (the transport's CPU meter reads it too), which the app updates once a
//! frame ([`Performance::update`]).
//!
//! The data updates once per drawn main-UI frame (the app does not call [`Performance::update`] while a loading screen
//! is up). The smoothed load jumps up to a higher load at once and decays toward a lower one with a 3 s time constant
//! (real time); the 3 s peak hold is real time, the 0.001 peak decay per frame. The idle overlay ends at the first
//! playback of the app session and never returns: a song load keeps it, and keeps the graph's history, which is
//! resized to the song (a length change stretches it) rather than cleared.
//!
//! - The status line is `CPU: <n> % | Underruns: <k>` ([`status_line`]): the smoothed load, and the audio callbacks
//!   that overran their buffer. The bar shows the smoothed load alone.
//!
//! - The graph's x axis is the song, from its start to its end (at least [`MIN_SPAN_BEATS`] beats): while the song
//!   plays, the load goes into the column under the song position, so a run fills the curve in. The sample is the raw
//!   load of the frame (the bar is the smoothed one). The data keeps [`BINS`] columns, each a share of the song; a view
//!   draws one per 2 px of its plot, each taken from the bin under it.
//! - The footer is `App memory:` and the transport `RAM` readout's value, `-` where unknown.

use eframe::egui::{Align2, Id, Pos2, Rect, Stroke, Ui, pos2, vec2};

use crate::metrics::FontSize;
use crate::theme::{RECORD_COLOR, Rgba, hsl_to_rgb};
use crate::ui::UiCtx;
use crate::ui::widgets::{Button, button, text_label as label};

/// The smoothed load `u` falls toward a lower raw load with this time constant, in seconds (it jumps up at once).
pub const DECAY_S: f32 = 3.0;
/// How long the peak is held, in seconds (real time).
pub const HOLD_S: f64 = 3.0;
/// What the peak falls by on each update once its hold has run out.
pub const PEAK_DECAY: f32 = 0.001;
/// The graph's columns in the data.
pub const BINS: usize = 1024;
/// The graph's shortest span, in beats.
pub const MIN_SPAN_BEATS: f64 = 4.0;
/// Song ticks a beat.
const TICKS_PER_BEAT: f64 = 128.0;

// The graph box's inner margins, in U: the axis labels' column, the top and right gap, the tick labels' row.
const GRAPH_LEFT: f32 = 1.5;
const GRAPH_GAP: f32 = 0.25;
const GRAPH_BOTTOM: f32 = 0.6;
/// Alpha of the graph's grid lines ("faint").
const GRID_ALPHA: f32 = 0.15;
/// The area under the curve.
const AREA_ALPHA: f32 = 0.2;
/// The idle overlay's `controlFill` alpha.
const IDLE_ALPHA: f32 = 0.3;
/// The curve's width, in device pixels (the editors' curve).
const CURVE_PX: f32 = 1.5;
/// The graph's sample spacing, in device pixels.
const SAMPLE_PX: f32 = 2.0;
/// The shown idle text.
pub const IDLE_TEXT: &str = "Press play to analyze performance";
/// The footer, `App memory: <size>`.
pub fn footer_line(app_memory: &str) -> String {
    format!("App memory: {app_memory}")
}

/// What the app reads for one update of the data.
#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub struct Reading {
    /// The audio callback's load, `0..=1`; 0 without audio.
    pub load: f32,
    /// The transport plays.
    pub playing: bool,
    /// The transport's song tick.
    pub tick: u64,
    /// Where the song ends, in beats.
    pub song_beats: f64,
    /// The audio callbacks that overran their buffer since the stream started.
    pub underruns: u64,
}

/// The data the Performance block draws: the smoothed load `u` (the bar), the underruns, the peak, and the graph.
#[derive(Clone, Debug, PartialEq)]
pub struct Performance {
    pub underruns: u64,
    underrun_base: u64,
    /// Plugin calls that could not acquire the instance lock before the callback's wait deadline.
    pub plugin_lock_misses: u64,
    plugin_lock_misses_base: u64,
    /// Contended plugin calls that did acquire the lock within that deadline.
    pub recovered_waits: u64,
    recovered_waits_base: u64,
    pub playing: bool,
    /// The song has played since the app started: the graph is no longer idle, and never is again.
    pub played: bool,
    /// The raw load of the last frame.
    pub load: f32,
    /// `u`: the load smoothed: up to a higher raw load at once, down toward a lower one with a [`DECAY_S`] time constant.
    pub smoothed: f32,
    /// When the last update was, in seconds of the frames' clock.
    updated: Option<f64>,
    /// The largest raw load, held [`HOLD_S`] and then falling [`PEAK_DECAY`] an update.
    pub peak: f32,
    /// When the peak was last reached, in seconds of the frames' clock.
    peak_at: f64,
    /// The song position as a share of the graph's span.
    pub position: f32,
    /// The transport `RAM` readout's value for the footer, `-` where unknown.
    pub app_memory: String,
    /// The bin the last update of a playing song wrote, until it stops.
    last_bin: Option<usize>,
    samples: Vec<Option<f32>>,
}

impl Default for Performance {
    fn default() -> Self {
        Self {
            underruns: 0,
            underrun_base: 0,
            plugin_lock_misses: 0,
            plugin_lock_misses_base: 0,
            recovered_waits: 0,
            recovered_waits_base: 0,
            playing: false,
            played: false,
            load: 0.0,
            smoothed: 0.0,
            updated: None,
            peak: 0.0,
            peak_at: 0.0,
            position: 0.0,
            app_memory: "-".to_owned(),
            last_bin: None,
            samples: vec![None; BINS],
        }
    }
}

impl Performance {
    /// One update, for the frame at `now` (seconds): what the app reads goes in, `u` moves toward the load, the peak is
    /// held or falls, and a playing song records the load at its position. The app calls it once per drawn main-UI
    /// frame and not while a loading screen is up.
    pub fn update(&mut self, now: f64, reading: Reading) {
        self.underruns = reading.underruns.saturating_sub(self.underrun_base);
        self.playing = reading.playing;
        self.played |= reading.playing;
        self.load = if reading.load > 0.0 {
            reading.load.min(1.0)
        } else {
            0.0
        };
        // The span is the song's length now; the history keeps its shares, so a longer or shorter song stretches it.
        let span = reading.song_beats.max(MIN_SPAN_BEATS);
        self.position = (reading.tick as f64 / TICKS_PER_BEAT / span).clamp(0.0, 1.0) as f32;
        let dt = self.updated.map_or(0.0, |t| (now - t).max(0.0) as f32);
        self.updated = Some(now);
        self.smoothed = if self.load >= self.smoothed {
            self.load
        } else {
            self.smoothed + (1.0 - (-dt / DECAY_S).exp()) * (self.load - self.smoothed)
        };
        if self.load >= self.peak {
            (self.peak, self.peak_at) = (self.load, now);
        } else if now - self.peak_at >= HOLD_S {
            self.peak = (self.peak - PEAK_DECAY).max(0.0);
        }
        self.record();
    }

    /// Rebase the cumulative diagnostics so the next frame displays them from zero.
    pub fn clear_counters(&mut self) {
        self.underrun_base = self.underrun_base.saturating_add(self.underruns);
        self.plugin_lock_misses_base = self
            .plugin_lock_misses_base
            .saturating_add(self.plugin_lock_misses);
        self.recovered_waits_base = self
            .recovered_waits_base
            .saturating_add(self.recovered_waits);
        self.underruns = 0;
        self.plugin_lock_misses = 0;
        self.recovered_waits = 0;
    }

    /// Updates the cumulative plugin diagnostics while respecting the user's clear point.
    pub fn set_plugin_lock_stats(&mut self, misses: u64, recovered: u64) {
        self.plugin_lock_misses = misses.saturating_sub(self.plugin_lock_misses_base);
        self.recovered_waits = recovered.saturating_sub(self.recovered_waits_base);
    }

    /// Writes the load at the song position while the song plays.
    fn record(&mut self) {
        if !self.playing {
            self.last_bin = None;
            return;
        }
        let bin = ((self.position * BINS as f32) as usize).min(BINS - 1);
        // The song moves on by more than a bin between two frames when it is short: the bins it passed over took the
        // same load, so that the run has no gaps. A jump (a seek, a loop) is not bridged.
        let from = match self.last_bin {
            Some(prev) if bin > prev && bin - prev <= BINS / 8 => prev + 1,
            _ => bin,
        };
        self.samples[from..=bin].fill(Some(self.load));
        self.last_bin = Some(bin);
    }

    /// The recorded load under `fraction` (`0..1`) of the graph's width.
    pub fn sample(&self, fraction: f32) -> Option<f32> {
        let bin = ((fraction * BINS as f32) as usize).min(BINS - 1);
        self.samples[bin]
    }

    /// The bar and the graph still change with no new input: the smoothed load is on its way, the peak is falling, or
    /// the song plays.
    pub fn moving(&self) -> bool {
        self.playing || self.smoothed > 1e-3 || (self.peak - self.load).abs() > 1e-3
    }
}

/// `CPU: 12 % | Underruns: 0`: the smoothed load `u`, held to 0…1, rounded and unpadded, and the underruns.
pub fn status_line(u: f32, underruns: u64) -> String {
    let share = u.clamp(0.0, 1.0) * 100.0;
    format!("CPU: {share:.0} % | Underruns: {underruns}")
}

/// Adds cumulative plugin-lock diagnostics once either counter is nonzero.
pub fn status_line_with_locks(p: &Performance) -> String {
    let mut line = status_line(p.smoothed, p.underruns);
    if p.plugin_lock_misses != 0 || p.recovered_waits != 0 {
        use std::fmt::Write;
        let _ = write!(
            line,
            " | Plugin lock misses: {} | Recovered waits: {}",
            p.plugin_lock_misses, p.recovered_waits
        );
    }
    line
}

/// Where the parts go inside a monitor rect, from the line height `fh` and the hairline unit `t`: the status line, the
/// bar (2 t tall), the graph, the footer.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Layout {
    pub status: Rect,
    pub bar: Rect,
    pub graph: Rect,
    pub footer: Rect,
}

/// The parts of the monitor rect `rect`; both views share it.
pub fn layout(rect: Rect, fh: f32, t: f32) -> Layout {
    let part =
        |dy: f32, h: f32| Rect::from_min_size(rect.min + vec2(0.0, dy), vec2(rect.width(), h));
    Layout {
        status: part(0.0, fh),
        bar: part(fh + t, 2.0 * t),
        graph: part(fh + 8.0 * t, rect.height() - 2.0 * fh - 9.0 * t),
        footer: part(rect.height() - fh, fh),
    }
}

/// The saturation and lightness of `color` (HSL).
fn saturation_lightness(color: Rgba) -> (f32, f32) {
    let (max, min) = (
        color.r.max(color.g).max(color.b),
        color.r.min(color.g).min(color.b),
    );
    let l = (max + min) / 2.0;
    let d = max - min;
    let s = if d == 0.0 {
        0.0
    } else {
        d / (1.0 - (2.0 * l - 1.0).abs())
    };
    (s, l)
}

/// The mix `a * (1 - k) + b * k` of two colours, opaque.
fn mix(a: Rgba, b: Rgba, k: f32) -> Rgba {
    let f = |a: f32, b: f32| (a * (1.0 - k) + b * k).clamp(0.0, 1.0);
    Rgba::rgb(f(a.r, b.r), f(a.g, b.g), f(a.b, b.b))
}

fn hsl(h: f32, s: f32, l: f32) -> Rgba {
    let (r, g, b) = hsl_to_rgb(h, s, l);
    Rgba::rgb(r, g, b)
}

/// The bar's colour for the smoothed load `u`: HSL(0.3 - 0.3 u, `recordColor`'s saturation and lightness), mixed with
/// `text` as text × (1 - √u) + hsl × √u.
pub fn bar_color(u: f32, text: Rgba) -> Rgba {
    let u = u.clamp(0.0, 1.0);
    let (s, l) = saturation_lightness(RECORD_COLOR);
    mix(text, hsl(0.3 - 0.3 * u, s, l), u.sqrt())
}

/// The curve's colour for a load `v`: `text` up to 0.5, then the stops 0.65 = HSL(1/6, .9, .5), 0.8 = HSL(1/12, .9,
/// .5) and 1.0 = HSL(0, .9, .5), blended between them.
pub fn curve_color(v: f32, text: Rgba) -> Rgba {
    let stops = [
        (0.5, text),
        (0.65, hsl(1.0 / 6.0, 0.9, 0.5)),
        (0.8, hsl(1.0 / 12.0, 0.9, 0.5)),
        (1.0, hsl(0.0, 0.9, 0.5)),
    ];
    let v = v.clamp(0.0, 1.0);
    let after = stops
        .iter()
        .position(|s| v <= s.0)
        .unwrap_or(stops.len() - 1);
    match after {
        0 => text,
        i => {
            let (lo, hi) = (stops[i - 1], stops[i]);
            mix(lo.1, hi.1, (v - lo.0) / (hi.0 - lo.0))
        }
    }
}

/// Draws the Performance block of `p` in `rect`: the status line, the load bar, the graph and the footer ([`layout`]).
/// Nothing else paints.
pub fn show(ui: &mut Ui, cx: &UiCtx, rect: Rect, p: &Performance) -> bool {
    let (m, c) = (cx.m, &cx.theme.colors);
    let font = m.font_id(FontSize::Pt10);
    let fh = ui.ctx().fonts_mut(|f| f.row_height(&font));
    let parts = layout(rect, fh, m.t());
    let text = c.text.to_color32();
    let line = status_line_with_locks(p);
    let clear_w = m.u(3.5);
    let clear_gap = m.u(0.5);
    let clear = parts.status.with_min_x(parts.status.right() - clear_w);
    let status = parts.status.with_max_x(clear.left() - clear_gap);
    label(
        ui,
        cx,
        status,
        &line,
        FontSize::Pt10,
        Align2::LEFT_CENTER,
        text,
    );
    let clear = Button::new(Id::new("performance_clear"), clear, "Clear", false, c);
    let cleared = button(ui, cx, &clear, true).clicked();
    bar(ui, cx, parts.bar, p);
    graph(ui, cx, parts.graph, p);
    label(
        ui,
        cx,
        parts.footer,
        &footer_line(&p.app_memory),
        FontSize::Pt10,
        Align2::LEFT_CENTER,
        text,
    );
    cleared
}

/// The load bar: `background`, and the smoothed load filled in its colour.
fn bar(ui: &Ui, cx: &UiCtx, rect: Rect, p: &Performance) {
    let c = &cx.theme.colors;
    let painter = ui.painter();
    painter.rect_filled(rect, 0.0, c.background.to_color32());
    let color = bar_color(p.smoothed, c.text).to_color32();
    let u = p.smoothed.clamp(0.0, 1.0);
    painter.rect_filled(rect.with_max_x(rect.left() + rect.width() * u), 0.0, color);
}

/// The graph box: `0 %`..`100 %` up the left, `1`..`6` along the bottom, faint grid lines, the load by song position
/// (one sample for each 2 device pixels), the song position, and the idle overlay while the song does not play.
fn graph(ui: &Ui, cx: &UiCtx, rect: Rect, p: &Performance) {
    let (m, c) = (cx.m, &cx.theme.colors);
    let text = c.text.to_color32();
    let painter = ui.painter();
    painter.rect_filled(rect, 0.0, c.background.to_color32());
    let plot = Rect::from_min_max(
        pos2(rect.left() + m.u(GRAPH_LEFT), rect.top() + m.u(GRAPH_GAP)),
        pos2(
            rect.right() - m.u(GRAPH_GAP),
            rect.bottom() - m.u(GRAPH_BOTTOM),
        ),
    );
    let grid = text.gamma_multiply(GRID_ALPHA);
    let hair = m.t();
    for i in 0..5 {
        let y = plot.bottom() - i as f32 * plot.height() / 4.0;
        let line = Rect::from_x_y_ranges(plot.x_range(), y - hair / 2.0..=y + hair / 2.0);
        painter.rect_filled(line, 0.0, grid);
        let tag = Rect::from_x_y_ranges(
            rect.left()..=plot.left() - m.u(GRAPH_GAP),
            y - m.u(0.5)..=y + m.u(0.5),
        );
        let s = format!("{} %", i * 25);
        label(ui, cx, tag, &s, FontSize::Pt6, Align2::RIGHT_CENTER, text);
    }
    for i in 0..6 {
        let x = plot.left() + (i as f32 + 0.5) * plot.width() / 6.0;
        let line = Rect::from_x_y_ranges(x - hair / 2.0..=x + hair / 2.0, plot.y_range());
        painter.rect_filled(line, 0.0, grid);
        let tag = Rect::from_x_y_ranges(x - m.u(0.5)..=x + m.u(0.5), plot.bottom()..=rect.bottom());
        let s = (i + 1).to_string();
        label(ui, cx, tag, &s, FontSize::Pt6, Align2::CENTER_CENTER, text);
    }
    curve(ui, cx, plot, p);
    // A 1 px `text` line at the song position.
    let x = plot.left() + p.position * plot.width();
    let one = 1.0 / m.ppp();
    let line = Rect::from_x_y_ranges(x..=x + one, plot.y_range());
    painter.rect_filled(line, 0.0, text);
    if !p.played {
        let dim = c.control_fill.to_color32().gamma_multiply(IDLE_ALPHA);
        painter.rect_filled(rect, 0.0, dim);
        label(
            ui,
            cx,
            rect,
            IDLE_TEXT,
            FontSize::Pt12,
            Align2::CENTER_CENTER,
            text,
        );
    }
}

/// The recorded load as a curve over `plot`, one sample for each [`SAMPLE_PX`] device pixels: the area under it in
/// avg(`text`, the curve's colour) at α 0.2, and the line in the curve's colour.
fn curve(ui: &Ui, cx: &UiCtx, plot: Rect, p: &Performance) {
    let (m, c) = (cx.m, &cx.theme.colors);
    let painter = ui.painter();
    let columns = (plot.width() * m.ppp() / SAMPLE_PX).floor().max(1.0) as usize;
    let step = plot.width() / columns as f32;
    let mut last: Option<Pos2> = None;
    for i in 0..columns {
        let Some(v) = p.sample((i as f32 + 0.5) / columns as f32) else {
            last = None;
            continue;
        };
        let (x, y) = (
            plot.left() + i as f32 * step,
            plot.bottom() - v * plot.height(),
        );
        let color = curve_color(v, c.text);
        let area = mix(c.text, color, 0.5);
        let area = Rgba {
            a: AREA_ALPHA,
            ..area
        };
        let column = Rect::from_x_y_ranges(x..=x + step, y..=plot.bottom());
        painter.rect_filled(column, 0.0, area.to_color32());
        let here = pos2(x + step / 2.0, y);
        if let Some(from) = last {
            let width = CURVE_PX / m.ppp();
            painter.line_segment([from, here], Stroke::new(width, color.to_color32()));
        }
        last = Some(here);
    }
}

#[cfg(test)]
mod tests {
    use eframe::egui::{
        CentralPanel, Color32, Context, Frame, RawInput, Shape, epaint::ClippedShape,
    };

    use super::*;
    use crate::metrics::{DEFAULT_UI_SCALE, Metrics};
    use crate::theme::{Theme, theme_by_id};

    fn theme() -> Theme {
        theme_by_id(0)
    }

    fn reading(load: f32) -> Reading {
        Reading {
            load,
            ..Reading::default()
        }
    }

    fn near(a: f32, b: f32) -> bool {
        (a - b).abs() < 1e-5
    }

    #[test]
    fn a_monitor_rect_is_laid_out_as_screens_md_says() {
        // (x, y, w, h), fh = the line height, t = the hairline: status (x, y, w, fh), bar (x, y + fh + t, w, 2t), graph
        // (x, y + fh + 8t, w, h - 2fh - 9t), footer (x, y + h - fh, w, fh).
        let rect = Rect::from_min_size(pos2(10.0, 20.0), vec2(500.0, 160.0));
        let (fh, t) = (16.0, 2.0);
        let l = layout(rect, fh, t);
        let at = |x: f32, y: f32, w: f32, h: f32| Rect::from_min_size(pos2(x, y), vec2(w, h));
        assert_eq!(l.status, at(10.0, 20.0, 500.0, 16.0));
        assert_eq!(l.bar, at(10.0, 20.0 + 16.0 + 2.0, 500.0, 4.0));
        assert_eq!(
            l.graph,
            at(10.0, 20.0 + 16.0 + 16.0, 500.0, 160.0 - 32.0 - 18.0)
        );
        assert_eq!(l.footer, at(10.0, 20.0 + 160.0 - 16.0, 500.0, 16.0));
    }

    #[test]
    fn the_bar_colour_runs_from_green_to_red_and_is_mixed_with_text() {
        let text = theme().colors.text;
        // HSL(0.3 - 0.3 u, recordColor's saturation and lightness (1, 0.5)), mixed with `text` as
        // text x (1 - sqrt(u)) + hsl x sqrt(u).
        assert_eq!(bar_color(0.0, text).to_color32(), text.to_color32());
        let red = bar_color(1.0, text);
        assert_eq!(red.to_color32(), Color32::from_rgb(255, 0, 0));
        for u in [0.04, 0.25, 0.64] {
            let (r, g, b) = hsl_to_rgb(0.3 - 0.3 * u, 1.0, 0.5);
            let k = f32::sqrt(u);
            let want = [(text.r, r), (text.g, g), (text.b, b)].map(|(t, h)| t * (1.0 - k) + h * k);
            let got = bar_color(u, text);
            assert!(
                near(got.r, want[0]) && near(got.g, want[1]) && near(got.b, want[2]),
                "u {u}: {got:?} want {want:?}"
            );
        }
        // Clamped: a load past 1 is the full colour.
        assert_eq!(bar_color(7.0, text), bar_color(1.0, text));
    }

    #[test]
    fn the_curve_colour_has_the_spec_stops() {
        let text = theme().colors.text;
        let at = |v: f32| curve_color(v, text);
        // Up to 0.5 it is `text`; the stops are HSL(1/6, .9, .5) at 0.65, HSL(1/12, .9, .5) at 0.8 and HSL(0, .9, .5) at 1.
        for v in [0.0, 0.3, 0.5] {
            assert_eq!(at(v), text, "{v}");
        }
        for (v, h) in [(0.65, 1.0 / 6.0), (0.8, 1.0 / 12.0), (1.0, 0.0)] {
            let want = hsl(h, 0.9, 0.5);
            let got = at(v);
            assert!(
                near(got.r, want.r) && near(got.g, want.g) && near(got.b, want.b),
                "{v}: {got:?} {want:?}"
            );
        }
        // Between two stops, the blend.
        let (lo, hi) = (hsl(1.0 / 6.0, 0.9, 0.5), hsl(1.0 / 12.0, 0.9, 0.5));
        let mid = at(0.725);
        assert!(near(mid.g, (lo.g + hi.g) / 2.0), "{mid:?}");
    }

    #[test]
    fn u_jumps_up_to_the_load_and_decays_with_a_3_s_time_constant() {
        let mut p = Performance::default();
        p.update(1.0, reading(0.8));
        assert_eq!(p.smoothed, 0.8);
        // Down: 3 s later it has gone 1 − 1/e of the way, whatever the frames between.
        p.update(4.0, reading(0.2));
        assert!(
            near(p.smoothed, 0.2 + 0.6 / std::f32::consts::E),
            "{}",
            p.smoothed
        );
        let mut q = Performance::default();
        q.update(1.0, reading(0.8));
        for k in 1..=30 {
            q.update(1.0 + 0.1 * f64::from(k), reading(0.2));
        }
        assert!(
            (q.smoothed - p.smoothed).abs() < 1e-4,
            "{} vs {}",
            q.smoothed,
            p.smoothed
        );
        // And up again at once.
        p.update(4.1, reading(0.9));
        assert_eq!(p.smoothed, 0.9);
    }

    #[test]
    fn the_peak_is_the_largest_load_held_3_s_of_real_time_then_falls_a_thousandth_a_frame() {
        let mut p = Performance::default();
        p.update(10.0, reading(0.8));
        assert_eq!(p.peak, 0.8);
        // Held for 3 s of the clock, however many frames come: a slow one and a burst of fast ones alike.
        for now in [11.0, 12.0, 12.5, 12.9, 12.99, 12.999] {
            p.update(now, reading(0.1));
            assert_eq!(p.peak, 0.8, "at {now}");
        }
        for k in 0..50 {
            p.update(12.9991 + f64::from(k) * 1e-5, reading(0.1));
        }
        assert_eq!(p.peak, 0.8, "frames do not shorten the hold");
        // Then a thousandth a frame.
        p.update(13.0, reading(0.1));
        assert!(near(p.peak, 0.799), "{}", p.peak);
        p.update(13.016, reading(0.1));
        assert!(near(p.peak, 0.798), "{}", p.peak);
        // A new maximum is held afresh.
        p.update(14.0, reading(0.9));
        assert_eq!(p.peak, 0.9);
        p.update(16.9, reading(0.1));
        assert_eq!(p.peak, 0.9);
        p.update(17.0, reading(0.1));
        assert!(near(p.peak, 0.899), "{}", p.peak);
        // The peak never goes below 0.
        let mut p = Performance::default();
        p.update(1.0, reading(0.0005));
        for k in 0..20 {
            p.update(5.0 + f64::from(k), reading(0.0));
        }
        assert_eq!(p.peak, 0.0);
    }

    #[test]
    fn a_playing_song_records_the_load_under_its_position_and_a_stopped_one_does_not() {
        let mut p = Performance::default();
        // Beat 2 of an 8-beat song: a quarter of the way.
        let at = |playing, load| Reading {
            load,
            playing,
            tick: 2 * 128,
            song_beats: 8.0,
            ..Reading::default()
        };
        p.update(1.0, at(false, 0.4));
        p.update(1.1, at(false, 0.4));
        assert!(near(p.position, 0.25));
        assert_eq!(p.sample(0.25), None, "stopped: nothing is recorded");
        p.update(1.2, at(true, 0.4));
        assert_eq!(p.sample(0.25), Some(0.4));
        assert_eq!(p.sample(0.5), None);
        assert_eq!(p.sample(0.0), None);
        // Stopping keeps the run.
        p.update(1.3, at(false, 0.0));
        assert_eq!(p.sample(0.25), Some(0.4));
        // A longer or shorter song resizes the history, it does not clear it: the run keeps its shares.
        p.update(
            1.4,
            Reading {
                song_beats: 16.0,
                ..at(false, 0.0)
            },
        );
        assert_eq!(p.sample(0.25), Some(0.4));
        assert!(near(p.position, 0.125), "the position is of the new length");
    }

    #[test]
    fn the_graph_is_idle_until_the_first_playback_of_the_session_and_never_again() {
        // Idle ends at the first playback of the app session and is not re-entered (a song load does not reset it):
        // stopping does not bring it back, and nothing here ever clears it.
        let mut p = Performance::default();
        assert!(!p.played);
        p.update(1.0, reading(0.2));
        assert!(!p.played, "not playing: still idle");
        p.update(
            1.1,
            Reading {
                playing: true,
                ..reading(0.2)
            },
        );
        assert!(p.played);
        p.update(1.2, reading(0.0));
        assert!(p.played, "stopped again: not idle again");
        p.update(
            1.3,
            Reading {
                song_beats: 99.0,
                ..reading(0.0)
            },
        );
        assert!(p.played, "another song's length: not idle again");
    }

    #[test]
    fn the_footer_names_the_apps_memory() {
        assert_eq!(footer_line("241 MB"), "App memory: 241 MB");
        assert_eq!(Performance::default().app_memory, "-");
    }

    #[test]
    fn a_run_has_no_gaps_where_the_song_moved_more_than_a_bin_between_updates() {
        let mut p = Performance {
            playing: true,
            load: 0.3,
            ..Performance::default()
        };
        let at = |p: &mut Performance, bin: usize| {
            p.position = (bin as f32 + 0.5) / BINS as f32;
            p.record();
        };
        at(&mut p, 10);
        at(&mut p, 14);
        let filled: Vec<usize> = (0..BINS).filter(|&b| p.samples[b].is_some()).collect();
        assert_eq!(filled, [10, 11, 12, 13, 14]);
        // A jump (a seek, a loop) is not bridged, nor is a stop and start.
        at(&mut p, 500);
        at(&mut p, 2);
        assert!(p.samples[3].is_none() && p.samples[499].is_none());
        p.playing = false;
        p.record();
        p.playing = true;
        at(&mut p, 40);
        assert!(p.samples[39].is_none() && p.samples[40].is_some());
    }

    /// One frame of `show` over `rect` at `ppp`: what it painted, and the metrics.
    fn painted(ppp: f32, rect: Rect, p: &Performance) -> (Vec<ClippedShape>, Metrics) {
        let ctx = Context::default();
        let mut raw = RawInput {
            screen_rect: Some(Rect::from_min_size(Pos2::ZERO, vec2(1170.0, 540.0))),
            ..Default::default()
        };
        raw.viewports
            .entry(raw.viewport_id)
            .or_default()
            .native_pixels_per_point = Some(ppp);
        let (theme, m) = (
            theme(),
            Metrics::from_pixels_per_point(ppp, DEFAULT_UI_SCALE),
        );
        let cx = UiCtx {
            theme: &theme,
            m: &m,
        };
        let out = ctx.run(raw, |ctx| {
            CentralPanel::default()
                .frame(Frame::NONE)
                .show(ctx, |ui| show(ui, &cx, rect, p));
        });
        (out.shapes, m)
    }

    fn rects(shapes: &[ClippedShape]) -> Vec<(Rect, Color32)> {
        shapes
            .iter()
            .filter_map(|s| match &s.shape {
                Shape::Rect(r) => Some((r.rect, r.fill)),
                _ => None,
            })
            .collect()
    }

    /// The monitor of `rect` at ppp 1 and 2 with its parts' rects, for the drawing tests.
    fn monitors() -> Vec<(f32, Rect)> {
        [1.0, 2.0]
            .into_iter()
            .map(|ppp| {
                (
                    ppp,
                    Rect::from_min_size(pos2(100.0, 50.0), vec2(512.0, 288.0)),
                )
            })
            .collect()
    }

    fn fh(ppp: f32) -> f32 {
        let ctx = Context::default();
        let mut raw = RawInput::default();
        raw.viewports
            .entry(raw.viewport_id)
            .or_default()
            .native_pixels_per_point = Some(ppp);
        // The fonts exist from the first frame on.
        let m = Metrics::from_pixels_per_point(ppp, DEFAULT_UI_SCALE);
        let mut height = 0.0;
        let _ = ctx.run(raw, |ctx| {
            height = ctx.fonts_mut(|f| f.row_height(&m.font_id(FontSize::Pt10)))
        });
        height
    }

    #[test]
    fn the_bar_paints_the_smoothed_load_in_its_colour_and_no_peak() {
        for (ppp, rect) in monitors() {
            let p = Performance {
                smoothed: 0.5,
                peak: 0.8,
                playing: true,
                played: true,
                ..Performance::default()
            };
            let (shapes, m) = painted(ppp, rect, &p);
            let t = m.t();
            let bar = layout(rect, fh(ppp), t).bar;
            let colors = theme().colors;
            let all = rects(&shapes);
            // The bar rect is `background`; u of it is filled in the colour of u; no line marks the peak.
            assert!(
                all.contains(&(bar, colors.background.to_color32())),
                "ppp {ppp}: {bar:?}"
            );
            let color = bar_color(0.5, colors.text).to_color32();
            let fill = bar.with_max_x(bar.left() + bar.width() * 0.5);
            assert!(
                all.contains(&(fill, color)),
                "ppp {ppp}: fill {fill:?} in {color:?}"
            );
            let x = bar.left() + bar.width() * 0.8;
            let line = Rect::from_x_y_ranges(x..=x + t, bar.y_range());
            assert!(!all.contains(&(line, color)), "ppp {ppp}: peak {line:?}");
            assert_eq!(bar.height(), 2.0 * t);
        }
    }

    #[test]
    fn only_the_bar_and_the_graph_box_paint_fills() {
        // No panel background, border, title or close X.
        for (ppp, rect) in monitors() {
            let p = Performance {
                playing: true,
                played: true,
                ..Performance::default()
            };
            let (shapes, m) = painted(ppp, rect, &p);
            let parts = layout(rect, fh(ppp), m.t());
            let clear = parts.status.with_min_x(parts.status.right() - m.u(3.5));
            for (r, fill) in rects(&shapes)
                .into_iter()
                .filter(|(_, f)| *f != Color32::TRANSPARENT)
            {
                assert!(
                    parts.bar.contains_rect(r)
                        || parts.graph.contains_rect(r)
                        || clear.contains_rect(r),
                    "ppp {ppp}: a fill {r:?} {fill:?} outside the bar and the graph"
                );
            }
            // The labels are the status line and the footer, and both are FontSize 2.
            let texts: Vec<_> = shapes
                .iter()
                .filter_map(|s| match &s.shape {
                    Shape::Text(t) => {
                        Some((t.galley.text().to_owned(), t.galley.rect.height(), t.pos))
                    }
                    _ => None,
                })
                .collect();
            let status = texts
                .iter()
                .find(|t| t.0.starts_with("CPU:"))
                .expect("status line");
            let footer = texts
                .iter()
                .find(|t| t.0.starts_with("App memory:"))
                .expect("footer");
            assert_eq!(status.0, "CPU: 0 % | Underruns: 0");
            assert!(
                (status.1 - footer.1).abs() < 0.01,
                "ppp {ppp}: both labels in one font"
            );
            // Each at its own line: the status line's text starts in the status rect, the footer's in the footer's
            // (within a pixel or two of the line's own height).
            assert!(
                parts.status.expand(2.0).contains(status.2),
                "ppp {ppp}: {:?}",
                status.2
            );
            assert!(
                parts.footer.expand(2.0).contains(footer.2),
                "ppp {ppp}: {:?}",
                footer.2
            );
        }
    }

    #[test]
    fn the_graph_has_a_sample_for_each_2_pixels_coloured_by_load_with_the_area_under_it() {
        for (ppp, rect) in monitors() {
            // A run at one load across the whole song, and the transport halfway.
            let mut p = Performance {
                playing: true,
                played: true,
                position: 0.5,
                ..Performance::default()
            };
            for bin in p.samples.iter_mut() {
                *bin = Some(0.7);
            }
            let (shapes, m) = painted(ppp, rect, &p);
            let graph = layout(rect, fh(ppp), m.t()).graph;
            let plot = Rect::from_min_max(
                pos2(graph.left() + m.u(GRAPH_LEFT), graph.top() + m.u(GRAPH_GAP)),
                pos2(
                    graph.right() - m.u(GRAPH_GAP),
                    graph.bottom() - m.u(GRAPH_BOTTOM),
                ),
            );
            let colors = theme().colors;
            let curve = curve_color(0.7, colors.text);
            // The area: avg(text, curve) at alpha 0.2, one column each 2 device pixels, from the load's height down.
            let area = Rgba {
                a: 0.2,
                ..mix(colors.text, curve, 0.5)
            }
            .to_color32();
            let top = plot.bottom() - 0.7 * plot.height();
            let columns: Vec<Rect> = rects(&shapes)
                .into_iter()
                .filter(|(r, c)| *c == area && (r.top() - top).abs() < 0.01)
                .map(|(r, _)| r)
                .collect();
            let want = (plot.width() * ppp / 2.0).floor() as usize;
            assert_eq!(columns.len(), want, "ppp {ppp}");
            for r in &columns {
                assert!((r.width() * ppp - 2.0).abs() < 0.1, "ppp {ppp}: {r:?}");
                assert!((r.bottom() - plot.bottom()).abs() < 0.01);
            }
            // The curve: segments in the curve's colour.
            let segments = shapes
                .iter()
                .filter(|s| matches!(&s.shape, Shape::LineSegment { stroke, .. } if stroke.color == curve.to_color32()))
                .count();
            assert_eq!(segments, want - 1, "ppp {ppp}");
            // A 1 px `text` line at the song position.
            let x = plot.left() + 0.5 * plot.width();
            let line = Rect::from_x_y_ranges(x..=x + 1.0 / ppp, plot.y_range());
            assert!(
                rects(&shapes).contains(&(line, colors.text.to_color32())),
                "ppp {ppp}: {line:?}"
            );
        }
    }

    #[test]
    fn idle_the_graph_is_dimmed_with_control_fill_and_says_so_and_playing_it_is_not() {
        for (ppp, rect) in monitors() {
            let colors = theme().colors;
            let idle = Performance::default();
            let (shapes, m) = painted(ppp, rect, &idle);
            let graph = layout(rect, fh(ppp), m.t()).graph;
            let dim = colors.control_fill.to_color32().gamma_multiply(0.3);
            assert!(
                rects(&shapes).contains(&(graph, dim)),
                "ppp {ppp}: no overlay"
            );
            // The text is FontSize 3 (12 pt), centred on the graph.
            let text = shapes.iter().find_map(|s| match &s.shape {
                Shape::Text(t) if t.galley.text() == IDLE_TEXT => Some(t.clone()),
                _ => None,
            });
            let text = text.expect("the idle text");
            assert_eq!(
                text.galley.job.sections[0].format.font_id,
                m.font_id(FontSize::Pt12)
            );
            assert!((text.visual_bounding_rect().center().x - graph.center().x).abs() < 1.5);
            assert!((text.visual_bounding_rect().center().y - graph.center().y).abs() < 3.0);
            // Played once and stopped again: not idle again either.
            let stopped = Performance {
                played: true,
                ..Performance::default()
            };
            let (shapes, _) = painted(ppp, rect, &stopped);
            assert!(
                !rects(&shapes).contains(&(graph, dim)),
                "ppp {ppp}: idle again"
            );
            assert!(
                !shapes
                    .iter()
                    .any(|s| matches!(&s.shape, Shape::Text(t) if t.galley.text() == IDLE_TEXT))
            );
            // Playing: neither.
            let playing = Performance {
                playing: true,
                played: true,
                ..Performance::default()
            };
            let (shapes, _) = painted(ppp, rect, &playing);
            assert!(!rects(&shapes).contains(&(graph, dim)));
            assert!(
                !shapes
                    .iter()
                    .any(|s| matches!(&s.shape, Shape::Text(t) if t.galley.text() == IDLE_TEXT))
            );
        }
    }

    #[test]
    fn the_status_line_is_the_rounded_load_and_the_underruns() {
        assert_eq!(status_line(0.05, 0), "CPU: 5 % | Underruns: 0");
        assert_eq!(status_line(0.42, 3), "CPU: 42 % | Underruns: 3");
        assert_eq!(status_line(1.7, 0), "CPU: 100 % | Underruns: 0");
        assert_eq!(status_line(-0.2, 0), "CPU: 0 % | Underruns: 0");
    }

    #[test]
    fn plugin_lock_counters_appear_after_contention() {
        let p = Performance {
            smoothed: 0.05,
            plugin_lock_misses: 18,
            recovered_waits: 194,
            ..Performance::default()
        };
        assert_eq!(
            status_line_with_locks(&p),
            "CPU: 5 % | Underruns: 0 | Plugin lock misses: 18 | Recovered waits: 194"
        );
        assert_eq!(
            status_line_with_locks(&Performance::default()),
            "CPU: 0 % | Underruns: 0"
        );
    }

    #[test]
    fn clearing_counters_rebases_future_readings() {
        let mut p = Performance::default();
        p.update(
            1.0,
            Reading {
                underruns: 25,
                ..reading(0.0)
            },
        );
        p.set_plugin_lock_stats(104, 424);
        assert_eq!(
            (p.underruns, p.plugin_lock_misses, p.recovered_waits),
            (25, 104, 424)
        );

        p.clear_counters();
        assert_eq!(
            (p.underruns, p.plugin_lock_misses, p.recovered_waits),
            (0, 0, 0)
        );

        p.update(
            2.0,
            Reading {
                underruns: 27,
                ..reading(0.0)
            },
        );
        p.set_plugin_lock_stats(108, 430);
        assert_eq!(
            (p.underruns, p.plugin_lock_misses, p.recovered_waits),
            (2, 4, 6)
        );
    }

    #[test]
    fn the_status_line_shows_the_smoothed_load() {
        // u, not the raw load of the last frame.
        let p = Performance {
            load: 0.9,
            smoothed: 0.05,
            ..Performance::default()
        };
        for (ppp, rect) in monitors() {
            let (shapes, _) = painted(ppp, rect, &p);
            let status = "CPU: 5 % | Underruns: 0";
            assert!(
                shapes
                    .iter()
                    .any(|s| matches!(&s.shape, Shape::Text(t) if t.galley.text() == status)),
                "ppp {ppp}"
            );
        }
    }
}
