//! Analyzer (ID 26), UIHeight 10 U. Pages `SPECTRUM` (0) … `SETTINGS` (5), with the tab strip at the bottom. Its only
//! param is `Enable`: every control is a module setting.
//!
//! The views draw the engine's analysis of the module's input (`honeydew_audio::analysis::analyzer`) in graph frames:
//! The axes with their ticks and labels, the view's curves, the marker, then the Freeze and Zoom buttons and the
//! selection overlay. Without a feed a view shows its frame alone.
//!
//! What the frames hold (Freeze, Zoom mode, the selections, the marker, the spectrogram's history) is the card's, in
//! egui temp data ([`Graphs`]), and none of it is saved: a load re-applies each display's own default selection, and
//! writers emit the new-module view state, which the blob keeps. A loaded module's scope trigger, which a load keeps,
//! is read from its blob.
//!
//! Details:
//! - Freeze and Zoom: a `controlFill` tile with the icon in G, the colours swapped while on.
//! - Any change of the page or of a setting is a control update (so is a load or an undo that changes them). Releasing
//!   the Spectrum's zoom (turning it off) copies its selection onto the Spectrogram; a control update with the Spectrum
//!   out of zoom mode copies nothing.
//! - The Spectrogram's dB range is its selection's y part of the Spectrum's full −90…+12 dB (its default is the whole
//!   range), as is its frequency range of the x part.
//! - The scope's X labels stay relative to the trigger in zoom mode too, where the trigger sits at the selection's
//!   width × the trigger fraction. Its overlay cycles on a double tap like the generic one.
//! - The marker's value is the drawn curve's at its x (the scope's the middle of a column's extent); without a curve
//!   point there (no pitch, no feed) neither dot nor readout shows.

use std::ops::Range;
use std::sync::Arc;

use eframe::egui::{
    Color32, Id, Mesh, Painter, Pos2, Rect, Sense, Shape, Stroke, StrokeKind, Ui, Vec2, pos2, vec2,
};
use honeydew_audio::analysis::{self, AnalyzerView, analyzer as engine};

use super::live_analysis;
use super::spreader::{balance_dot, draw_petal, half_arc, polar, scope};
use crate::panels::build::*;
use crate::panels::setting_item;
use crate::ui::{UiCtx, widgets};

/// The Spectrogram shows in 3D (`Mode` #3), unless its zoom is being done on the Spectrum.
const SPECTROGRAM: Show = Show::All(&[Show::Setting(3, 1), Show::Flag(SPECTRUM_ZOOM, 0)]);

pub fn panel() -> Option<Panel> {
    // Every view is a graph frame.
    let graph = |n: u32| display(n).margins(0.3, 0.0, 0.3, 0.7);
    let side = |page: usize, top: Part, bottom: Part| {
        vstack([top, bottom])
            .w(Size::U(2.0))
            .mb(0.6)
            .show(Show::Tab(page))
    };
    let slider = |n: u32, label: &'static str, value: fn(f32) -> String| {
        v_slider(n, label)
            .value_text()
            .w(Size::U(2.0))
            .mb(0.6)
            .default(0.5)
            .value_fn(value)
    };
    Some(Panel::new(vstack([
        hstack([
            // `Mode` 2D shows the spectrum, 3D the spectrogram.
            graph(1).show(Show::Not(&SPECTROGRAM)).draw(spectrum),
            graph(2).show(SPECTROGRAM).draw(spectrogram),
            side(
                0,
                spinner(3, "Mode").items(&["2D", "3D"]),
                spinner(4, "Scale").items(&["Log", "Lin"]),
            ),
        ])
        .show(Show::Tab(0)),
        hstack([graph(5).draw(oscilloscope), slider(6, "Threshold", level)]).show(Show::Tab(1)),
        hstack([
            graph(7).draw(loudness),
            side(
                2,
                spinner(8, "Mode").items(&["LUFS 0.4s", "LUFS 3s", "True Peak", "RMS", "Peak"]),
                knob(9, "Target").default(1.0).value_fn(target),
            ),
        ])
        .show(Show::Tab(2)),
        hstack([graph(10).mr(0.7).draw(vectorscope)]).show(Show::Tab(3)),
        hstack([
            graph(11).draw(pitch_track),
            slider(12, "Reference", reference),
        ])
        .show(Show::Tab(4)),
        vstack([
            spacer(13),
            hstack([
                spinner(14, "Channel")
                    .items(&["Left", "Right", "L + R"])
                    .default(2.0),
                spinner(15, "Reactivity")
                    .items(&["Slow", "Medium", "Fast"])
                    .default(1.0),
                spinner(16, "DC Block").items(&["On", "Off"]).default(1.0),
                knob(17, "Input Gain").default(0.5).value_fn(gain),
            ])
            .h(Size::U(3.5))
            .show(Show::Tab(5)),
            spacer(18),
        ])
        .show(Show::Tab(5)),
        tabs(
            19,
            "SPECTRUM · SCOPE · LOUDNESS · STEREO · PITCH · SETTINGS",
        )
        .h(Size::U(1.25)),
    ])))
}

// The value texts, in each control's units.

/// `Threshold`: the trigger level 10^(2v − 2), 0.01…1.
fn level(v: f32) -> String {
    format!("{:.2}", 10f32.powf(2.0 * v - 2.0))
}

/// `Target`: 102v − 90 dB, with `+` above 0 as the loudness view's labels.
fn target(v: f32) -> String {
    let db = 102.0 * v - 90.0;
    if db > 0.0 {
        format!("+{db:.1} dB")
    } else {
        format!("{db:.1} dB")
    }
}

/// `Reference`: A4 = 400 + 80v Hz, as the Tuner's.
fn reference(v: f32) -> String {
    format!("{:.0} Hz", 400.0 + 80.0 * v)
}

/// `Input Gain`: ×10^(2v − 1), ×0.10…×10.00.
fn gain(v: f32) -> String {
    format!("×{:.2}", 10f32.powf(2.0 * v - 1.0))
}

// ---- Graph frames ----

/// A graph axis over its visible range `lo`…`hi`: linear, or the Spectrum's squared log of the full range
/// `fmin`…`fmax`, x = (ln(f + 1 − fmin)/ln(fmax − fmin + 1))².
#[derive(Clone, Copy, Debug)]
struct Axis {
    lo: f32,
    hi: f32,
    log: Option<(f32, f32)>,
}

impl Axis {
    const fn linear(lo: f32, hi: f32) -> Axis {
        Axis { lo, hi, log: None }
    }

    /// The full-range position of `v` (the squared log), or `v` itself.
    fn full(&self, v: f32) -> f32 {
        match self.log {
            Some((fmin, fmax)) => {
                ((v + 1.0 - fmin).max(1.0).ln() / (fmax - fmin + 1.0).ln()).powi(2)
            }
            None => v,
        }
    }

    fn unfull(&self, p: f32) -> f32 {
        match self.log {
            Some((fmin, fmax)) => (p.max(0.0).sqrt() * (fmax - fmin + 1.0).ln()).exp() - 1.0 + fmin,
            None => p,
        }
    }

    /// `v`'s place along the visible range, 0…1.
    fn pos(&self, v: f32) -> f32 {
        let (lo, hi) = (self.full(self.lo), self.full(self.hi));
        (self.full(v) - lo) / (hi - lo)
    }

    /// The value at place `p` along the visible range.
    fn value(&self, p: f32) -> f32 {
        let (lo, hi) = (self.full(self.lo), self.full(self.hi));
        self.unfull(lo + p * (hi - lo))
    }

    /// The part from place `from` to place `to`.
    fn part(self, from: f32, to: f32) -> Axis {
        Axis {
            lo: self.value(from),
            hi: self.value(to),
            ..self
        }
    }
}

/// Tick generation: the candidates inside the visible range, then the value at each of `n` even places unless a tick
/// lies within 0.8/(n − 1) of it; sorted by place. Returns (place, value).
fn ticks(axis: &Axis, candidates: &[f32], n: usize) -> Vec<(f32, f32)> {
    let mut ticks: Vec<(f32, f32)> = (candidates.iter())
        .map(|&v| (axis.pos(v), v))
        .filter(|&(p, _)| (-1e-4..=1.0 + 1e-4).contains(&p))
        .collect();
    let gap = 0.8 / (n - 1) as f32;
    for k in 0..n {
        let p = k as f32 / (n - 1) as f32;
        if !ticks.iter().any(|&(q, _)| (q - p).abs() < gap) {
            ticks.push((p, axis.value(p)));
        }
    }
    ticks.sort_by(|a, b| a.0.total_cmp(&b.0));
    ticks
}

/// The tick labels by `format` (value, decimals), with a decimal more while two neighbours read the same, up to 3.
fn labels(ticks: &[(f32, f32)], format: impl Fn(f32, usize) -> String) -> Vec<String> {
    let mut decimals = 0;
    loop {
        let texts: Vec<String> = ticks.iter().map(|&(_, v)| format(v, decimals)).collect();
        if decimals == 3 || texts.windows(2).all(|w| w[0] != w[1]) {
            return texts;
        }
        decimals += 1;
    }
}

/// `{+}%.{p}f dB`: `+` only above 0.
fn db_label(v: f32, p: usize) -> String {
    let text = format!("{v:.p$} dB");
    if v > 0.0 && !text.starts_with('0') {
        format!("+{text}")
    } else {
        text
    }
}

/// `%.{p}f {k}Hz`: over 999 in kHz.
fn hz_label(v: f32, p: usize) -> String {
    if v > 999.0 {
        format!("{:.p$} kHz", v / 1000.0)
    } else {
        format!("{v:.p$} Hz")
    }
}

/// An axis's labels: its candidates and the format of a value at some decimals.
type Labels<'a> = Option<(&'a [f32], fn(f32, usize) -> String)>;

/// A view's frame: its options, axes and label formats.
struct Frame<'a> {
    x: Axis,
    y: Axis,
    x_labels: Labels<'a>,
    y_labels: Labels<'a>,
    /// The grid lines drawn: horizontal (per Y tick) and vertical (per X tick).
    grid: (bool, bool),
}

impl Frame<'_> {
    /// The plot rect P in the control rect `r`: 0.5 U less at the bottom with X labels, 0.6 U less at the left with Y
    /// labels.
    fn plot(&self, r: Rect, u: f32) -> Rect {
        let mut p = r;
        if self.x_labels.is_some() {
            p.max.y -= 0.5 * u;
        }
        if self.y_labels.is_some() {
            p.min.x += 0.6 * u;
        }
        p
    }

    /// The axes, steps 1–5 in `r`: P filled `controlFill`, the horizontal grid and Y labels, the vertical grid and X
    /// labels; 9 X ticks and 7 Y ticks. Returns P.
    fn draw(&self, ui: &Ui, cx: &UiCtx, r: Rect, ink: Color32) -> Rect {
        let (u, px) = (cx.m.u(1.0), 1.0 / cx.m.ppp());
        let p = self.plot(r, u);
        let colors = &cx.theme.colors;
        ui.painter()
            .rect_filled(p, 0.0, colors.control_fill.to_color32());
        let line = Stroke::new(px, colors.background.to_color32());
        let y_ticks = self
            .y_labels
            .map(|(c, _)| ticks(&self.y, c, 7))
            .unwrap_or_else(|| ticks(&self.y, &[], 7));
        let y_texts = self.y_labels.map(|(_, f)| labels(&y_ticks, f));
        for (k, &(place, _)) in y_ticks.iter().enumerate() {
            let y = p.bottom() - place * p.height();
            if self.grid.0 {
                ui.painter().hline(p.x_range(), y, line);
            }
            if let Some(texts) = &y_texts {
                let at = Rect::from_min_size(
                    pos2(p.left() - 0.5 * u, y - 0.2 * u),
                    vec2(0.3 * u, 0.4 * u),
                );
                widgets::text_label(
                    ui,
                    cx,
                    at,
                    &texts[k],
                    FontSize::Pt8,
                    Align2::RIGHT_CENTER,
                    ink,
                );
            }
        }
        let x_ticks = self
            .x_labels
            .map(|(c, _)| ticks(&self.x, c, 9))
            .unwrap_or_else(|| ticks(&self.x, &[], 9));
        let x_texts = self.x_labels.map(|(_, f)| labels(&x_ticks, f));
        for (k, &(place, _)) in x_ticks.iter().enumerate() {
            let x = p.left() + place * p.width();
            if self.grid.1 {
                ui.painter().vline(x, p.y_range(), line);
            }
            if let Some(texts) = &x_texts {
                let at = Rect::from_min_size(pos2(x, p.bottom() + 0.2 * u), vec2(0.5 * u, 0.4 * u));
                widgets::text_label(
                    ui,
                    cx,
                    at,
                    &texts[k],
                    FontSize::Pt8,
                    Align2::CENTER_CENTER,
                    ink,
                );
            }
        }
        p
    }
}

// ---- Freeze, Zoom and the marker ----

/// The panel flag ([`PanelState::flags`]) that is on while the Spectrogram's zoom is done on the Spectrum, which then
/// shows in 3D.
const SPECTRUM_ZOOM: &str = "analyzer spectrum zoom";

/// A selection: fractions of a view's full range, x and w from the left, y and h from the bottom.
#[derive(Clone, Copy, Debug, PartialEq)]
struct Sel {
    x: f32,
    w: f32,
    y: f32,
    h: f32,
}

impl Sel {
    const WHOLE: Sel = Sel {
        x: 0.0,
        w: 1.0,
        y: 0.0,
        h: 1.0,
    };

    /// The part of the full ranges `(x, y)` it covers.
    fn of(self, (x, y): (Axis, Axis)) -> (Axis, Axis) {
        (
            x.part(self.x, self.x + self.w),
            y.part(self.y, self.y + self.h),
        )
    }

    /// Its rect in P.
    fn rect(self, p: Rect) -> Rect {
        Rect::from_min_max(
            pos2(
                p.left() + self.x * p.width(),
                p.bottom() - (self.y + self.h) * p.height(),
            ),
            pos2(
                p.left() + (self.x + self.w) * p.width(),
                p.bottom() - self.y * p.height(),
            ),
        )
    }
}

/// The graph frames, in [`Graphs::graphs`] order.
#[derive(Clone, Copy, Debug, PartialEq)]
enum View {
    Spectrum,
    Spectrogram,
    Scope,
    Loudness,
    Stereo,
    Pitch,
}

const VIEWS: [View; 6] = [
    View::Spectrum,
    View::Spectrogram,
    View::Scope,
    View::Loudness,
    View::Stereo,
    View::Pitch,
];

/// −60…+6 dB of −90…+12 dB as a selection's y and h.
const DB_Y: f32 = f32::from_bits(0x3E96_9698);
const DB_H: f32 = f32::from_bits(0x3F25_A5A6);

impl View {
    /// Its new-module selection, which every re-application shows.
    fn fresh(self) -> Sel {
        match self {
            View::Spectrum => Sel {
                y: DB_Y,
                h: DB_H,
                ..Sel::WHOLE
            },
            View::Spectrogram | View::Stereo => Sel::WHOLE,
            // 0.5 s, ±1.0 of ±1.5.
            View::Scope => Sel {
                x: 0.0,
                w: 0.5,
                y: f32::from_bits(0x3E2A_AAAC),
                h: f32::from_bits(0x3F2A_AAAB),
            },
            // The last 10 s.
            View::Loudness => Sel {
                x: 0.5,
                w: 0.5,
                y: DB_Y,
                h: DB_H,
            },
            // The last 5 s, notes 48–90 of 24–108.
            View::Pitch => Sel {
                x: 0.5,
                w: 0.5,
                y: f32::from_bits(0x3E92_4924),
                h: 0.5,
            },
        }
    }

    /// What it shows until a selection is first applied: the new-module one, but the scope's 0.1 s and the pitch track's
    /// notes 48–72.
    fn initial(self) -> Sel {
        match self {
            View::Scope => Sel {
                w: 0.1,
                ..self.fresh()
            },
            View::Pitch => Sel {
                h: 24.0 / 84.0,
                ..self.fresh()
            },
            _ => self.fresh(),
        }
    }

    /// The smallest selection (w, h).
    fn min(self) -> (f32, f32) {
        match self {
            View::Spectrum | View::Spectrogram => (0.2, 0.5),
            View::Scope => (0.05, 0.1),
            View::Loudness => (0.05, 0.5),
            View::Pitch => (0.1, 0.1),
            View::Stereo => (1.0, 1.0),
        }
    }
}

/// What a press on the selection overlay holds.
#[derive(Clone, Copy, Debug, PartialEq)]
enum Grab {
    /// Corner k: 0 top left, 1 top right, 2 bottom right, 3 bottom left.
    Corner(usize),
    /// The whole selection.
    Move,
    /// The scope's trigger handle.
    Trigger,
}

/// One graph frame's state.
#[derive(Clone, Debug)]
struct Graph {
    /// Freeze is on: the view holds [`Self::held`].
    frozen: bool,
    /// The analysis it draws, and the input time it came at.
    held: Option<Arc<[f32]>>,
    at: f64,
    /// Zoom mode: the full range shows, with the selection overlay on top.
    zoom: bool,
    /// The selection the overlay edits, and the one the view shows outside zoom mode.
    sel: Sel,
    view: Sel,
    /// The last selection the user dragged: the double tap's third stop.
    user: Option<Sel>,
    /// The overlay's press: what it holds, the selection then and where it began.
    grab: Option<(Grab, Sel, Pos2)>,
    /// The marker's point as fractions of P from its top left (a drag may take it out of P).
    marker: Option<Vec2>,
}

/// The spectrogram's history: spectra by input time, one per 0.1 s, kept for 5 s.
type History = Vec<(f64, Arc<Vec<f32>>)>;

/// An Analyzer card's graph frames, kept in egui temp data between frames ([`Self::load`], [`Self::store`]).
#[derive(Clone, Debug)]
struct Graphs {
    graphs: [Graph; 6],
    /// The scope's trigger, a fraction of its view's width.
    trigger: f32,
    /// The page and the settings as the views last saw them: any change is a control update.
    seen: Option<(usize, Vec<(u32, u32)>)>,
    history: History,
}

impl Graphs {
    /// A new card's, the trigger as `module`'s blob holds it (0.25 without one). A module with a blob came from a song,
    /// whose load ends by re-applying each view's default selection, so it shows its new-module selection; a new module
    /// shows its initial one.
    fn new(module: &ModuleInstance) -> Graphs {
        let blob = module.file.custom_data.as_deref();
        let trigger = blob
            .and_then(|b| Some(f32::from_le_bytes(b.get(76..80)?.try_into().ok()?)))
            .filter(|v| !v.is_nan())
            .map_or(0.25, |v| v.clamp(0.0, 1.0));
        Graphs {
            graphs: VIEWS.map(|view| Graph {
                frozen: false,
                held: None,
                at: 0.0,
                zoom: false,
                sel: view.fresh(),
                view: if blob.is_some() {
                    view.fresh()
                } else {
                    view.initial()
                },
                user: None,
                grab: None,
                marker: None,
            }),
            trigger,
            seen: None,
            history: History::new(),
        }
    }

    fn id(ui: &Ui, module: &ModuleInstance) -> Id {
        ui.id().with(("analyzer graphs", module.instance_id))
    }

    /// The card's graphs, after the control update since the last frame if there was one: every zoom mode off and
    /// each stored selection shown, and when it chose 3D (`Mode` #3) the spectrogram history emptied.
    fn load(ui: &Ui, module: &ModuleInstance, state: &mut PanelState) -> Graphs {
        let mut graphs: Graphs = ui
            .data(|d| d.get_temp(Self::id(ui, module)))
            .unwrap_or_else(|| Graphs::new(module));
        let mut settings: Vec<(u32, u32)> = (state.settings.iter())
            .map(|(&k, &v)| (k, v.to_bits()))
            .collect();
        settings.sort_unstable();
        let seen = Some((state.tab, settings));
        if let Some((_, before)) = &graphs.seen
            && graphs.seen != seen
        {
            let mode = |s: &[(u32, u32)]| {
                let v = s
                    .iter()
                    .find(|&&(k, _)| k == 3)
                    .map(|&(_, v)| f32::from_bits(v));
                v.map(setting_item)
            };
            if mode(before) != Some(1) && setting_item(setting(state, 3, 0.0)) == 1 {
                graphs.history.clear();
            }
            for view in VIEWS {
                graphs.zoom_off(view, state);
            }
        }
        graphs.seen = seen;
        graphs
    }

    fn store(self, ui: &Ui, module: &ModuleInstance) {
        ui.data_mut(|d| d.insert_temp(Self::id(ui, module), self));
    }

    /// Turns `view`'s zoom mode off, and shows its stored selection. Releasing the Spectrum's zoom copies its
    /// selection onto the Spectrogram, which shows again in 3D.
    fn zoom_off(&mut self, view: View, state: &mut PanelState) {
        let graph = &mut self.graphs[view as usize];
        let released = std::mem::take(&mut graph.zoom);
        (graph.view, graph.grab) = (graph.sel, None);
        if view == View::Spectrum && released {
            let sel = graph.sel;
            let spectrogram = &mut self.graphs[View::Spectrogram as usize];
            (spectrogram.sel, spectrogram.view) = (sel, sel);
            state.flags.remove(SPECTRUM_ZOOM);
        }
    }

    /// The analysis `view` draws: the rack's latest, or while frozen what it held.
    fn data(&mut self, view: View, ui: &Ui, module: &ModuleInstance) -> Option<AnalyzerView> {
        let graph = &mut self.graphs[view as usize];
        if !graph.frozen {
            graph.held = live_analysis(ui, module.instance_id);
            graph.at = ui.input(|i| i.time);
        }
        graph.held.as_deref().and_then(analysis::analyzer)
    }

    /// The axes `view` draws on, from its full ranges `full`: those in zoom mode, else the part its selection shows.
    fn axes(&self, view: View, full: (Axis, Axis)) -> (Axis, Axis) {
        let graph = &self.graphs[view as usize];
        if graph.zoom {
            full
        } else {
            graph.view.of(full)
        }
    }

    /// The overlay's handles in P: each corner, or the scope's right-hand corners and its trigger handle on P's top.
    fn handles(&self, view: View, p: Rect) -> Vec<(Grab, Pos2)> {
        let sel = self.graphs[view as usize].sel;
        let r = sel.rect(p);
        if view == View::Scope {
            let trigger = pos2(p.left() + sel.w * self.trigger * p.width(), p.top());
            vec![
                (Grab::Corner(1), r.right_top()),
                (Grab::Corner(2), r.right_bottom()),
                (Grab::Trigger, trigger),
            ]
        } else {
            let corners = [
                r.left_top(),
                r.right_top(),
                r.right_bottom(),
                r.left_bottom(),
            ];
            (0..4).map(|k| (Grab::Corner(k), corners[k])).collect()
        }
    }

    /// The rest of `view`'s frame over its curve, in P, with its input: the marker (when `marker` formats its readout
    /// from an (x, y) value pair; `curve` is the drawn curve in P, on `axes`), the Freeze and Zoom buttons, and in
    /// zoom mode the selection overlay.
    #[allow(clippy::too_many_arguments)]
    fn finish(
        &mut self,
        ui: &mut Ui,
        cx: &UiCtx,
        id: Id,
        p: Rect,
        view: View,
        state: &mut PanelState,
        ink: Color32,
        axes: (Axis, Axis),
        curve: &[Pos2],
        marker: Option<fn(f32, f32) -> String>,
    ) {
        let (u, px) = (cx.m.u(1.0), 1.0 / cx.m.ppp());
        let colors = &cx.theme.colors;
        let (text, fill) = (colors.text.to_color32(), colors.control_fill.to_color32());
        let zoom = self.graphs[view as usize].zoom;
        // The plot takes the touches the buttons (on top of it) do not; the overlay's handles reach 0.6 U beyond P.
        let area = if zoom { p.expand(0.6 * u) } else { p };
        let plot = ui.interact(area, id.with("plot"), Sense::click_and_drag());
        // The marker.
        let graph = &self.graphs[view as usize];
        if let (Some(format), false, Some(m)) = (marker, zoom, graph.marker)
            && (0.0..=1.0).contains(&m.x)
            && (0.0..=1.0).contains(&m.y)
        {
            let x = p.left() + m.x * p.width();
            let near = curve.iter().filter(|q| (q.x - x).abs() <= 2.0 * px);
            if let Some(q) = near.min_by(|a, b| (a.x - x).abs().total_cmp(&(b.x - x).abs())) {
                ui.painter().circle_filled(pos2(x, q.y), 5.0 * px, text);
                let value = (
                    axes.0.value(m.x),
                    axes.1.value((p.bottom() - q.y) / p.height()),
                );
                let at = Rect::from_min_size(p.min + vec2(0.4 * u, 0.1 * u), vec2(3.0 * u, u));
                let readout = format(value.0, value.1);
                widgets::text_label(
                    ui,
                    cx,
                    at,
                    &readout,
                    FontSize::Pt10,
                    Align2::LEFT_CENTER,
                    ink,
                );
            }
        }
        // The buttons: Freeze at P.right − 1.4 U, Zoom 1.4 U further left, 0.4 U below P's top.
        let tile =
            |k: f32| Rect::from_min_size(pos2(p.right() - k * u, p.top() + 0.4 * u), vec2(u, u));
        let button = |ui: &mut Ui, k: f32, name: &str, icon: IconId, on: bool| {
            let rect = tile(k);
            let b = widgets::Button {
                icon: Some(icon),
                fill_on: ink,
                fill_off: fill,
                text_on: fill,
                text_off: Some(ink),
                ..widgets::Button::new(id.with(name), rect, "", false, colors)
            };
            widgets::button(ui, cx, &b, on).clicked()
        };
        let frozen = button(ui, 1.4, "freeze", IconId::Freeze, graph.frozen);
        let zoomed = view != View::Stereo && button(ui, 2.8, "zoom", IconId::Search, zoom);
        // The overlay.
        if zoom {
            let painter = ui.painter();
            let r = graph.sel.rect(p);
            if view == View::Scope {
                let x = p.left() + graph.sel.w * self.trigger * p.width();
                painter.vline(x, p.y_range(), Stroke::new(px, text.gamma_multiply(0.5)));
            }
            painter.rect_stroke(r, 0.0, Stroke::new(cx.m.t(), text), StrokeKind::Middle);
            painter.rect_filled(r, 0.0, text.gamma_multiply(0.1));
            for (_, at) in self.handles(view, p) {
                painter.circle_filled(at, 0.2 * u, text);
            }
        }
        // The input. A press on a button is the button's: the plot, which also takes drags, ignores it.
        let origin = ui.input(|i| i.pointer.press_origin());
        let tiles = if view == View::Stereo { 1 } else { 2 };
        let on_button =
            origin.is_some_and(|o| (1..=tiles).any(|k| tile(1.4 * k as f32).contains(o)));
        let graph = &mut self.graphs[view as usize];
        if frozen {
            graph.frozen = !graph.frozen;
        }
        if zoomed {
            match view {
                // Zooming in 3D is done on the 2D view.
                View::Spectrogram => {
                    self.graphs[View::Spectrum as usize].zoom = true;
                    state.flags.insert(SPECTRUM_ZOOM, 1);
                }
                _ if zoom => self.zoom_off(view, state),
                _ => graph.zoom = true,
            }
        } else if !on_button {
            if zoom {
                self.overlay_input(&plot, view, p, u);
            } else if marker.is_some()
                && plot.is_pointer_button_down_on()
                && let Some(at) = plot.interact_pointer_pos()
            {
                graph.marker = Some((at - p.min) / p.size());
            }
        }
        // What the input changed shows on the next frame.
        let touched = plot.is_pointer_button_down_on() || plot.clicked() || plot.double_clicked();
        if frozen || zoomed || touched {
            ui.ctx().request_repaint();
        }
    }

    /// The selection overlay's input: a double tap cycles the default selection, the whole area and the last user
    /// rectangle; a press within 0.6 U of a handle drags it, one inside the selection moves it, within the view's
    /// minimum size and the full range. The scope's keeps its left edge at 0 and its amplitude symmetric, and its
    /// trigger handle moves the trigger.
    fn overlay_input(&mut self, plot: &eframe::egui::Response, view: View, p: Rect, u: f32) {
        let handles = self.handles(view, p);
        let graph = &mut self.graphs[view as usize];
        if plot.double_clicked() {
            graph.grab = None;
            graph.sel = if graph.sel == view.fresh() {
                Sel::WHOLE
            } else if graph.sel == Sel::WHOLE {
                graph.user.unwrap_or(view.fresh())
            } else {
                view.fresh()
            };
            return;
        }
        let down = plot.is_pointer_button_down_on();
        let pointer = plot.interact_pointer_pos();
        if !down {
            if let Some((_, start, _)) = graph.grab.take()
                && graph.sel != start
            {
                graph.user = Some(graph.sel);
            }
            return;
        }
        let Some(at) = pointer else { return };
        if graph.grab.is_none() {
            let origin = plot.ctx.input(|i| i.pointer.press_origin()).unwrap_or(at);
            let nearest = (handles.iter())
                .map(|&(grab, h)| (grab, h.distance(origin)))
                .filter(|&(_, d)| d <= 0.6 * u)
                .min_by(|a, b| a.1.total_cmp(&b.1));
            let inside = view != View::Scope && graph.sel.rect(p).contains(origin);
            let grab = nearest.map(|(g, _)| g).or(inside.then_some(Grab::Move));
            graph.grab = grab.map(|g| (g, graph.sel, origin));
        }
        let Some((grab, start, origin)) = graph.grab else {
            return;
        };
        let (dx, dy) = (
            (at.x - origin.x) / p.width(),
            (origin.y - at.y) / p.height(),
        );
        let (min_w, min_h) = view.min();
        let s = &mut graph.sel;
        match (view, grab) {
            (View::Scope, Grab::Trigger) => {
                self.trigger = ((at.x - p.left()) / (s.w * p.width())).clamp(0.0, 1.0);
            }
            (View::Scope, Grab::Corner(k)) => {
                // Top right: up grows it; bottom right: down grows it; both ends move.
                let dh = if k == 1 { 2.0 * dy } else { -2.0 * dy };
                s.w = (start.w + dx).clamp(min_w, 1.0);
                s.h = (start.h + dh).clamp(min_h, 1.0);
                s.y = 0.5 - s.h / 2.0;
            }
            (_, Grab::Move) => {
                s.x = (start.x + dx).clamp(0.0, 1.0 - s.w);
                s.y = (start.y + dy).clamp(0.0, 1.0 - s.h);
            }
            (_, Grab::Corner(k)) => {
                let (mut l, mut r) = (start.x, start.x + start.w);
                let (mut b, mut t) = (start.y, start.y + start.h);
                if k == 0 || k == 3 {
                    l = (l + dx).clamp(0.0, r - min_w);
                } else {
                    r = (r + dx).clamp(l + min_w, 1.0);
                }
                if k < 2 {
                    t = (t + dy).clamp(b + min_h, 1.0);
                } else {
                    b = (b + dy).clamp(0.0, t - min_h);
                }
                *s = Sel {
                    x: l,
                    w: r - l,
                    y: b,
                    h: t - b,
                };
            }
            (_, Grab::Trigger) => {}
        }
    }
}

/// Setting `n`, or `default`.
fn setting(state: &PanelState, n: u32, default: f32) -> f32 {
    state.settings.get(&n).copied().unwrap_or(default)
}

/// The audio's sample rate, 48 kHz without audio.
fn rate(ui: &Ui) -> f32 {
    match crate::panels::sample_rate(ui) {
        r if r > 0.0 => r,
        _ => 48_000.0,
    }
}

/// Each step of the device-pixel columns across P, `step` px apart, as (left, right) x.
fn columns(p: Rect, ppp: f32, step: f32) -> Vec<(f32, f32)> {
    let n = ((p.width() * ppp / step).floor() as usize).max(1);
    (0..=n)
        .map(|i| {
            let x = p.left() + (i as f32 * step / ppp).min(p.width());
            (x - step / ppp, x)
        })
        .collect()
}

/// The area under `points` down to `bottom`, filled `fill` (two triangles per segment), then a 1 px line in `ink`;
/// only the segments `keep` takes.
fn fill_and_line(
    painter: &Painter,
    points: &[Pos2],
    bottom: f32,
    (fill, ink, px): (Color32, Color32, f32),
    keep: impl Fn(Pos2, Pos2) -> bool,
) {
    let mut mesh = Mesh::default();
    let mut runs: Vec<Vec<Pos2>> = vec![Vec::new()];
    for w in points.windows(2) {
        if !keep(w[0], w[1]) {
            runs.push(Vec::new());
            continue;
        }
        let i = mesh.vertices.len() as u32;
        for v in [w[0], w[1], pos2(w[1].x, bottom), pos2(w[0].x, bottom)] {
            mesh.colored_vertex(v, fill);
        }
        mesh.add_triangle(i, i + 1, i + 2);
        mesh.add_triangle(i, i + 2, i + 3);
        let run = runs.last_mut().expect("a run");
        if run.is_empty() {
            run.push(w[0]);
        }
        run.push(w[1]);
    }
    if mesh.is_empty() {
        return;
    }
    painter.add(Shape::mesh(mesh));
    for run in runs.into_iter().filter(|r| r.len() > 1) {
        painter.add(Shape::line(run, Stroke::new(px, ink)));
    }
}

/// Note names, C = 0.
const NAMES: [&str; 12] = [
    "C", "C#", "D", "D#", "E", "F", "F#", "G", "G#", "A", "A#", "B",
];

/// `%s%d`: MIDI note `n`'s name and octave, int(n/12) less the app octave offset `O`: middle C is C5 at 0, C4 at 1
/// (`Set middle C to C4`, the card's [`flag::OCTAVE_OFFSET`]).
fn note_name<const O: i32>(n: i32) -> String {
    let n = n - 12 * O;
    format!("{}{}", NAMES[n.rem_euclid(12) as usize], n.div_euclid(12))
}

/// Of `[offset 0, offset 1]`, the one for the card's app octave offset ([`flag::OCTAVE_OFFSET`]).
fn octave<T>(state: &PanelState, [off, on]: [T; 2]) -> T {
    if state.flags.get(flag::OCTAVE_OFFSET) == Some(&1) {
        on
    } else {
        off
    }
}

// ---- SPECTRUM ----

/// The Spectrum's and Spectrogram's full frequency axis: 20–20 000 Hz, `Scale` (#4) Log (the squared log) or Lin.
fn freq_axis(state: &PanelState) -> Axis {
    let log = setting_item(setting(state, 4, 0.0)) == 0;
    Axis {
        lo: 20.0,
        hi: 20_000.0,
        log: log.then_some((20.0, 20_000.0)),
    }
}

/// The Spectrum's full ranges: 20–20 000 Hz and −90…+12 dB.
fn spectrum_full(state: &PanelState) -> (Axis, Axis) {
    (freq_axis(state), Axis::linear(-90.0, 12.0))
}

/// The spectrum's level at `f` Hz: its bins (k·rate/2048 Hz) read at a fractional bin, Catmull-Rom with the Log scale
/// and linearly with Lin, plus the +4.5 dB/octave tilt around 1 kHz.
fn level_at(bins: &[f32], f: f32, rate: f32, smooth: bool) -> f32 {
    let x = f * engine::FFT as f32 / rate;
    let i = (x.floor() as isize).clamp(0, bins.len() as isize - 2);
    let t = x - i as f32;
    let b = |k: isize| bins[k.clamp(0, bins.len() as isize - 1) as usize];
    let v = if smooth {
        let (p0, p1, p2, p3) = (b(i - 1), b(i), b(i + 1), b(i + 2));
        0.5 * (2.0 * p1
            + (p2 - p0) * t
            + (2.0 * p0 - 5.0 * p1 + 4.0 * p2 - p3) * t * t
            + (3.0 * p1 - p0 - 3.0 * p2 + p3) * t * t * t)
    } else {
        b(i) + (b(i + 1) - b(i)) * t
    };
    v + 4.5 * (f / 1000.0).log2()
}

/// The Spectrum's curve in P: one point every 2 px, the highest level over the column (narrow peaks survive), held
/// down to P's bottom and, with `clamp_top`, to its top.
#[allow(clippy::too_many_arguments)]
fn spectrum_points(
    p: Rect,
    ppp: f32,
    x: &Axis,
    y: &Axis,
    bins: &[f32],
    rate: f32,
    smooth: bool,
    clamp_top: bool,
) -> Vec<Pos2> {
    columns(p, ppp, 2.0)
        .into_iter()
        .map(|(from, to)| {
            let (f0, f1) = (
                x.value((from - p.left()) / p.width()),
                x.value((to - p.left()) / p.width()),
            );
            let mut level = level_at(bins, f1, rate, smooth);
            let (k0, k1) = (
                (f0 * engine::FFT as f32 / rate).ceil() as usize,
                (f1 * engine::FFT as f32 / rate) as usize,
            );
            for (k, &bin) in bins.iter().enumerate().take(k1 + 1).skip(k0) {
                let f = k as f32 * rate / engine::FFT as f32;
                if f >= f0.max(1.0) {
                    level = level.max(bin + 4.5 * (f / 1000.0).log2());
                }
            }
            let q = y.pos(level).max(0.0);
            let q = if clamp_top { q.min(1.0) } else { q };
            pos2(to, p.bottom() - q * p.height())
        })
        .collect()
}

const SPECTRUM_Y: [f32; 11] = [
    12.0, 6.0, 0.0, -6.0, -12.0, -20.0, -30.0, -40.0, -50.0, -60.0, -75.0,
];
const SPECTRUM_X: [f32; 8] = [50.0, 150.0, 450.0, 1000.0, 2000.0, 4000.0, 7000.0, 12000.0];
const SPECTROGRAM_X: [f32; 8] = [50.0, 200.0, 500.0, 1000.0, 2000.0, 4000.0, 7000.0, 12000.0];

/// The Spectrum's marker readout: `%.0f Hz (%s%d) : %.1f dB`, the note round(12·log2(f/440) + 69).
fn spectrum_marker<const O: i32>(f: f32, db: f32) -> String {
    let note = (12.0 * (f / 440.0).log2() + 69.0).round() as i32;
    format!("{f:.0} Hz ({}) : {db:.1} dB", note_name::<O>(note))
}

/// #1, the Spectrum: its selection of 20–20 000 Hz and −90…+12 dB (−60…+6 dB at first), filled G at 20 % under a 1 px
/// G line.
fn spectrum(
    ui: &mut Ui,
    cx: &UiCtx,
    id: Id,
    r: Rect,
    module: &mut ModuleInstance,
    state: &mut PanelState,
    ink: Color32,
) -> Vec<usize> {
    let mut graphs = Graphs::load(ui, module, state);
    let (x, y) = graphs.axes(View::Spectrum, spectrum_full(state));
    let log = x.log.is_some();
    let frame = Frame {
        x,
        y,
        x_labels: Some((if log { &SPECTRUM_X } else { &[] }, hz_label)),
        y_labels: Some((&SPECTRUM_Y, db_label)),
        grid: (true, true),
    };
    let p = frame.draw(ui, cx, r, ink);
    let mut curve = Vec::new();
    if let Some(view) = graphs.data(View::Spectrum, ui, module) {
        let (ppp, rate) = (cx.m.ppp(), rate(ui));
        curve = spectrum_points(p, ppp, &x, &y, &view.spectrum, rate, log, true);
        let look = (ink.gamma_multiply(0.2), ink, 1.0 / ppp);
        fill_and_line(ui.painter(), &curve, p.bottom(), look, |_, _| true);
    }
    let marker = Some(octave(
        state,
        [
            spectrum_marker::<0> as fn(f32, f32) -> String,
            spectrum_marker::<1>,
        ],
    ));
    graphs.finish(
        ui,
        cx,
        id,
        p,
        View::Spectrum,
        state,
        ink,
        (x, y),
        &curve,
        marker,
    );
    graphs.store(ui, module);
    Vec::new()
}

/// #2, the Spectrogram (`Mode` 3D): 12 rows over 5 s, the newest at the front. Row i (d = i/11) has its baseline
/// Ph·i/12 up, Pw·(1 − 0.4d) wide and centred, its selection's dB range in 5·Ph/12·(1 − 0.8d), and shows the spectrum
/// of 5 s·d ago (the panel keeps them; a row older than what it kept is left out). Drawn back to front, one point every
/// 8 px: filled down to its baseline in opaque `controlFill`, then a 1 px G line at alpha 1 − 0.66d, each segment only
/// where both ends are below P's top.
fn spectrogram(
    ui: &mut Ui,
    cx: &UiCtx,
    id: Id,
    r: Rect,
    module: &mut ModuleInstance,
    state: &mut PanelState,
    ink: Color32,
) -> Vec<usize> {
    let mut graphs = Graphs::load(ui, module, state);
    // Its zoom is done on the Spectrum (and copied here): the x part is the frequency range, the y part the dB range.
    let (x, level) = graphs.graphs[View::Spectrogram as usize]
        .view
        .of(spectrum_full(state));
    let log = x.log.is_some();
    let frame = Frame {
        x,
        y: Axis::linear(0.0, 5.0),
        x_labels: Some((if log { &SPECTROGRAM_X } else { &[] }, hz_label)),
        y_labels: Some((&[], |v, _| format!("{v:.1} s"))),
        grid: (false, false),
    };
    let p = frame.draw(ui, cx, r, ink);
    let feed = graphs.data(View::Spectrogram, ui, module);
    let graph = &graphs.graphs[View::Spectrogram as usize];
    let (now, frozen) = (graph.at, graph.frozen);
    let history = &mut graphs.history;
    if !frozen {
        if let Some(view) = &feed
            && history.last().is_none_or(|(t, _)| now - t >= 0.1)
        {
            history.push((now, Arc::new(view.spectrum.clone())));
        }
        history.retain(|(t, _)| now - t <= 5.2);
    }
    let (ppp, px, rate) = (cx.m.ppp(), 1.0 / cx.m.ppp(), rate(ui));
    let control = cx.theme.colors.control_fill.to_color32();
    for i in (0..12).rev() {
        let d = i as f32 / 11.0;
        let back = 5.0 * f64::from(d);
        let age = |t: f64| (now - t - back).abs();
        let Some((_, spectrum)) = (history.iter())
            .filter(|(t, _)| age(*t) <= 0.1)
            .min_by(|a, b| age(a.0).total_cmp(&age(b.0)))
        else {
            continue;
        };
        let w = p.width() * (1.0 - 0.4 * d);
        let row = Rect::from_min_max(
            pos2(
                p.center().x - w / 2.0,
                p.bottom()
                    - p.height() * i as f32 / 12.0
                    - 5.0 * p.height() / 12.0 * (1.0 - 0.8 * d),
            ),
            pos2(
                p.center().x + w / 2.0,
                p.bottom() - p.height() * i as f32 / 12.0,
            ),
        );
        let points = spectrum_points(row, ppp / 4.0, &x, &level, spectrum, rate, log, false);
        let look = (control, ink.gamma_multiply(1.0 - 0.66 * d), px);
        let below = |a: Pos2, b: Pos2| a.y >= p.top() && b.y >= p.top();
        fill_and_line(ui.painter(), &points, row.bottom(), look, below);
    }
    let axes = (x, frame.y);
    graphs.finish(
        ui,
        cx,
        id,
        p,
        View::Spectrogram,
        state,
        ink,
        axes,
        &[],
        None,
    );
    graphs.store(ui, module);
    Vec::new()
}

// ---- SCOPE ----

/// The trigger level from `Threshold` (#6): 10^(2v − 2).
fn trigger_level(state: &PanelState) -> f32 {
    10f32.powf(2.0 * setting(state, 6, 0.5) - 2.0)
}

/// The buckets of the scope's last second (oldest first) a view shows, `pre` before its trigger and `post` from it: the
/// latest trigger (a rising edge that arms below 0.99·level and fires at ≥ level) with `post` after it, or once that is
/// `free` buckets late (0.2 s) the latest `pre` + `post` (free-running).
fn scope_window(
    scope: &[(f32, f32)],
    level: f32,
    pre: usize,
    post: usize,
    free: usize,
) -> Range<usize> {
    let n = scope.len();
    let (pre, post) = (pre.min(n), post.min(n - pre.min(n)));
    let mut armed = false;
    let mut trigger = None;
    for (k, &(low, high)) in scope.iter().enumerate().take(n - post) {
        if armed && high >= level {
            trigger = Some(k);
            armed = false;
        }
        if low < 0.99 * level {
            armed = true;
        }
    }
    match trigger.filter(|&k| k >= pre && n - k <= post + free) {
        Some(k) => k - pre..k + post,
        None => n - (pre + post)..n,
    }
}

/// #5, the Oscilloscope: its selection of 1 s and ±1.5 (0.1 s and ±1.0 at first, then 0.5 s), the trigger at the
/// trigger fraction of the width (25 %: −25…+75 ms). One point every 2 px: a column's min and max of the samples since
/// the previous point, a G rect over their extent where it is taller than a pixel, else a 1 px G line.
fn oscilloscope(
    ui: &mut Ui,
    cx: &UiCtx,
    id: Id,
    r: Rect,
    module: &mut ModuleInstance,
    state: &mut PanelState,
    ink: Color32,
) -> Vec<usize> {
    let mut graphs = Graphs::load(ui, module, state);
    let graph = &graphs.graphs[View::Scope as usize];
    // The seconds shown and those before the trigger: the view's, or in zoom mode the whole second with the trigger at
    // the selection's place.
    let (span, pre) = if graph.zoom {
        (1.0, graph.sel.w * graphs.trigger)
    } else {
        (graph.view.w, graph.view.w * graphs.trigger)
    };
    let (_, y) = graphs.axes(
        View::Scope,
        (Axis::linear(0.0, 1.0), Axis::linear(-1.5, 1.5)),
    );
    let x = Axis::linear(-1000.0 * pre, 1000.0 * (span - pre));
    let frame = Frame {
        x,
        y,
        x_labels: Some((&[0.0], |v, p| format!("{v:.p$} ms"))),
        y_labels: Some((&[], |v, p| format!("{v:.p$}"))),
        grid: (true, true),
    };
    let p = frame.draw(ui, cx, r, ink);
    let mut curve = Vec::new();
    if let Some(view) = graphs.data(View::Scope, ui, module) {
        let buckets = |s: f32| (s * 1000.0 * engine::SCOPE_PER_MS as f32).round() as usize;
        let free = buckets(0.2);
        let level = trigger_level(state);
        let range = scope_window(&view.scope, level, buckets(pre), buckets(span - pre), free);
        let window = &view.scope[range];
        let (px, painter) = (1.0 / cx.m.ppp(), ui.painter());
        let at = |v: f32| p.bottom() - y.pos(v).clamp(0.0, 1.0) * p.height();
        let mut last: Option<Pos2> = None;
        for (from, to) in columns(p, cx.m.ppp(), 2.0) {
            let span = |x: f32| ((x - p.left()) / p.width() * window.len() as f32) as usize;
            let end = window.len().saturating_sub(1);
            let buckets = &window[span(from.max(p.left())).min(end)..=span(to).min(end)];
            let low = buckets.iter().map(|b| b.0).fold(f32::INFINITY, f32::min);
            let high = buckets
                .iter()
                .map(|b| b.1)
                .fold(f32::NEG_INFINITY, f32::max);
            if !low.is_finite() || !high.is_finite() {
                continue;
            }
            let (top, bottom) = (at(high), at(low));
            if bottom - top > px {
                let x0 = last.map_or(to, |l| l.x);
                painter.rect_filled(
                    Rect::from_x_y_ranges(x0..=to.max(x0 + px), top..=bottom),
                    0.0,
                    ink,
                );
            } else if let Some(l) = last {
                painter.line_segment([l, pos2(to, top)], Stroke::new(px, ink));
            }
            let point = pos2(to, (top + bottom) / 2.0);
            curve.push(point);
            last = Some(point);
        }
    }
    let marker = Some((|ms, v| format!("{ms:.0} ms : {v:.2}")) as fn(f32, f32) -> String);
    graphs.finish(
        ui,
        cx,
        id,
        p,
        View::Scope,
        state,
        ink,
        (x, y),
        &curve,
        marker,
    );
    graphs.store(ui, module);
    Vec::new()
}

// ---- LOUDNESS ----

const LOUDNESS_Y: [f32; 12] = [
    12.0, 6.0, 0.0, -6.0, -12.0, -20.0, -30.0, -40.0, -50.0, -60.0, -75.0, -90.0,
];

/// The engine meter (Peak, True Peak, RMS, LUFS 0.4 s, LUFS 3 s) of `Mode` (#8) item `item` (`LUFS 0.4s`, `LUFS 3s`,
/// `True Peak`, `RMS`, `Peak`), and whether it is in LUFS.
fn meter_of(item: usize) -> (usize, bool) {
    match item {
        0 => (3, true),
        1 => (4, true),
        2 => (1, false),
        3 => (2, false),
        _ => (0, false),
    }
}

/// #7, Loudness: its selection of the last 20 s and −90…+12 dB (−60…+6 dB over the last 10 s at first), now at the
/// right; one point every 2 px, each the mean of its column's 50 ms entries; G at 20 % between the curve and the target
/// line where it is above, and the 1 px G line; the target line (`Target` #9, 102v − 90 dB) in `text` while in view;
/// the readouts.
fn loudness(
    ui: &mut Ui,
    cx: &UiCtx,
    id: Id,
    r: Rect,
    module: &mut ModuleInstance,
    state: &mut PanelState,
    ink: Color32,
) -> Vec<usize> {
    let mut graphs = Graphs::load(ui, module, state);
    let full = (Axis::linear(20.0, 0.0), Axis::linear(-90.0, 12.0));
    let (x, y) = graphs.axes(View::Loudness, full);
    let frame = Frame {
        x,
        y,
        x_labels: Some((&[], |v, p| format!("{v:.p$} s"))),
        y_labels: Some((&LOUDNESS_Y, db_label)),
        grid: (true, true),
    };
    let p = frame.draw(ui, cx, r, ink);
    let (u, px) = (cx.m.u(1.0), 1.0 / cx.m.ppp());
    let target = 102.0 * setting(state, 9, 1.0) - 90.0;
    let target_y = p.bottom() - y.pos(target) * p.height();
    let text = cx.theme.colors.text.to_color32();
    let feed = graphs.data(View::Loudness, ui, module);
    let painter = ui.painter();
    let (meter, lufs) = meter_of(setting_item(setting(state, 8, 0.0)));
    let mut points = Vec::new();
    if let Some(view) = &feed {
        let entries = &view.loudness[meter];
        // Seconds back from now to each column's edges.
        let back = |at: f32| x.value((at - p.left()) / p.width());
        points = columns(p, cx.m.ppp(), 2.0)
            .into_iter()
            .filter_map(|(from, to)| {
                let (newer, older) = (back(to), back(from.max(p.left())));
                let index = |s: f32| entries.len() as isize - 1 - (s / 0.05) as isize;
                let (a, b) = (index(older).max(0), index(newer).max(0));
                let column: Vec<f32> = (a..=b)
                    .map(|i| entries[i as usize])
                    .filter(|&v| v > engine::FLOOR)
                    .collect();
                (!column.is_empty()).then(|| {
                    let mean = column.iter().sum::<f32>() / column.len() as f32;
                    pos2(to, p.bottom() - y.pos(mean).clamp(0.0, 1.0) * p.height())
                })
            })
            .collect();
        let above: Vec<Pos2> = points
            .iter()
            .map(|q| pos2(q.x, q.y.min(target_y)))
            .collect();
        let mut mesh = Mesh::default();
        for w in above.windows(2) {
            let i = mesh.vertices.len() as u32;
            for v in [w[0], w[1], pos2(w[1].x, target_y), pos2(w[0].x, target_y)] {
                mesh.colored_vertex(v, ink.gamma_multiply(0.2));
            }
            mesh.add_triangle(i, i + 1, i + 2);
            mesh.add_triangle(i, i + 2, i + 3);
        }
        painter.add(Shape::mesh(mesh));
        painter.add(Shape::line(points.clone(), Stroke::new(px, ink)));
    }
    if (0.0..=1.0).contains(&y.pos(target)) {
        painter.hline(p.x_range(), target_y, Stroke::new(px, text));
    }
    // The readouts: A, then B and C each 0.8 U after the one before, Pw/3 − 0.8 U wide, 1 U high at Pb − 1.1 U.
    let w = p.width() / 3.0 - 0.8 * u;
    let a = Rect::from_min_size(pos2(p.left() + 0.4 * u, p.bottom() - 1.1 * u), vec2(w, u));
    let boxes = [0.0, 1.0, 2.0].map(|k| a.translate(vec2(k * (w + 0.8 * u), 0.0)));
    let mut texts: Vec<(usize, String, Align2)> = Vec::new();
    if let Some(view) = &feed {
        let max = view.maxima[meter];
        if max > engine::FLOOR {
            let unit = if lufs { "LUFS" } else { "db" };
            texts.push((0, format!("Peak: {max:.1} {unit}"), Align2::LEFT_CENTER));
        }
        if lufs && let (Some(integrated), Some(range)) = (view.integrated, view.range) {
            texts.push((
                1,
                format!("Integrated: {integrated:.1} LUFS"),
                Align2::CENTER_CENTER,
            ));
            texts.push((2, format!("Range: {range:.1} LU"), Align2::RIGHT_CENTER));
        }
    }
    for (k, text, align) in texts {
        let b = boxes[k];
        ui.painter().rect_filled(
            b,
            0.0,
            cx.theme
                .colors
                .control_fill
                .to_color32()
                .gamma_multiply(0.5),
        );
        widgets::text_label(ui, cx, b, &text, FontSize::Pt8, align, ink);
    }
    let marker = Some((|s, db| format!("{s:.2} s : {db:.1} dB")) as fn(f32, f32) -> String);
    graphs.finish(
        ui,
        cx,
        id,
        p,
        View::Loudness,
        state,
        ink,
        (x, y),
        &points,
        marker,
    );
    graphs.store(ui, module);
    Vec::new()
}

// ---- STEREO ----

/// #10, the Vectorscope (no labels, no ticks, no zoom) on a `controlFill` frame: the guides, the `L` and `R` labels,
/// the petal, the balance dots, then the interferometer child: 2 s of correlations, newest at the right, mirrored about
/// its middle; then its Freeze button.
fn vectorscope(
    ui: &mut Ui,
    cx: &UiCtx,
    id: Id,
    p: Rect,
    module: &mut ModuleInstance,
    state: &mut PanelState,
    ink: Color32,
) -> Vec<usize> {
    let mut graphs = Graphs::load(ui, module, state);
    let (u, px) = (cx.m.u(1.0), 1.0 / cx.m.ppp());
    let colors = &cx.theme.colors;
    let (fill, bg) = (
        colors.control_fill.to_color32(),
        colors.background.to_color32(),
    );
    let feed = graphs.data(View::Stereo, ui, module);
    let painter = ui.painter();
    painter.rect_filled(p, 0.0, fill);
    let (c, r) = scope(p);
    let hairline = Stroke::new(px, bg);
    for deg in [45.0, 135.0] {
        painter.line_segment([c, polar(c, 0.9 * r, deg)], hairline);
    }
    for k in [0.9, 0.6, 0.3] {
        painter.add(Shape::line(half_arc(c, k * r), hairline));
    }
    let d = 0.9 * r * std::f32::consts::FRAC_1_SQRT_2;
    let label = |x: f32, text: &str| {
        let at = Rect::from_min_size(pos2(x, p.bottom() - (0.8 * u + d)), vec2(0.4 * u, 0.4 * u));
        widgets::text_label(ui, cx, at, text, FontSize::Pt10, Align2::CENTER_CENTER, bg);
    };
    label(c.x - (0.8 * u + d), "L");
    label(c.x + d + 0.4 * u, "R");
    let painter = ui.painter();
    if let Some(view) = &feed {
        draw_petal(painter, c, r, &view.petal.bins, ink, px);
    }
    painter.circle_filled(balance_dot(p, 0.0), 5.0 * px, bg);
    let balance = feed
        .as_ref()
        .map_or(0.0, |v| v.petal.balance.clamp(-1.0, 1.0));
    painter.circle_filled(balance_dot(p, balance), 5.0 * px, ink);
    // The interferometer.
    let child = Rect::from_min_size(
        p.min + vec2(0.4 * u, 0.4 * u),
        vec2(0.125 * p.width(), p.height() / 6.0),
    );
    painter.rect_filled(child, 0.0, fill);
    if let Some(view) = &feed {
        let shown = ((2.0 * rate(ui) / analysis::BLOCK as f32).ceil() as usize)
            .min(view.correlations.len());
        let values = &view.correlations[view.correlations.len() - shown..];
        let cols = columns(child, cx.m.ppp(), 2.0);
        let v = |x: f32| {
            let i =
                (((x - child.left()) / child.width()) * (values.len() - 1) as f32).round() as usize;
            0.5 + 0.5 * values[i.min(values.len() - 1)].clamp(-1.0, 1.0)
        };
        let upper: Vec<Pos2> = cols
            .iter()
            .map(|&(_, x)| pos2(x, child.bottom() - v(x) * child.height()))
            .collect();
        let lower: Vec<Pos2> = cols
            .iter()
            .map(|&(_, x)| pos2(x, child.top() + v(x) * child.height()))
            .collect();
        let mut mesh = Mesh::default();
        for (w, l) in upper.windows(2).zip(lower.windows(2)) {
            if v(w[1].x) < 0.5 {
                let i = mesh.vertices.len() as u32;
                for q in [w[0], w[1], l[1], l[0]] {
                    mesh.colored_vertex(q, ink.gamma_multiply(0.2));
                }
                mesh.add_triangle(i, i + 1, i + 2);
                mesh.add_triangle(i, i + 2, i + 3);
            }
        }
        painter.add(Shape::mesh(mesh));
        painter.add(Shape::line(upper, Stroke::new(px, ink)));
        painter.add(Shape::line(lower, Stroke::new(px, ink)));
    }
    let axes = (Axis::linear(0.0, 1.0), Axis::linear(0.0, 1.0));
    graphs.finish(ui, cx, id, p, View::Stereo, state, ink, axes, &[], None);
    graphs.store(ui, module);
    Vec::new()
}

// ---- PITCH ----

/// `%s%d`: the note name and octave of MIDI note `v`.
fn note_label<const O: i32>(v: f32, _: usize) -> String {
    note_name::<O>(v.round() as i32)
}

/// The pitch track's marker readout: `%s%d %+d%%`, the note, its octave and the cents off it.
fn pitch_marker<const O: i32>(_: f32, note: f32) -> String {
    let n = note.round();
    let cents = ((note - n) * 100.0).round() as i32;
    format!("{} {cents:+}%", note_name::<O>(n as i32))
}

/// #11, the Pitch track: its selection of notes 24–108 over the last 10 s (notes 48–72 over 5 s at first), newest at
/// the right, on `Reference` (#12, A4 = 400 + 80v Hz). Black keys get a `background` band at 50 %, E–F and B–C a 1 px
/// line; the 1 px G curve, one point every 2 px, skips a jump of 7 semitones or more and an end at P's top or bottom.
fn pitch_track(
    ui: &mut Ui,
    cx: &UiCtx,
    id: Id,
    r: Rect,
    module: &mut ModuleInstance,
    state: &mut PanelState,
    ink: Color32,
) -> Vec<usize> {
    let mut graphs = Graphs::load(ui, module, state);
    let full = (Axis::linear(10.0, 0.0), Axis::linear(24.0, 108.0));
    let (x, y) = graphs.axes(View::Pitch, full);
    let frame = Frame {
        x,
        y,
        x_labels: Some((&[], |v, _| format!("{v:.1} s"))),
        y_labels: Some((
            &[],
            octave(
                state,
                [note_label::<0> as fn(f32, usize) -> String, note_label::<1>],
            ),
        )),
        grid: (false, true),
    };
    let p = frame.draw(ui, cx, r, ink);
    let (px, band) = (
        1.0 / cx.m.ppp(),
        cx.theme.colors.background.to_color32().gamma_multiply(0.5),
    );
    let painter = ui.painter();
    let at = |n: f32| p.bottom() - y.pos(n) * p.height();
    for n in (y.lo.floor() as i32)..=(y.hi.ceil() as i32) {
        match n.rem_euclid(12) {
            1 | 3 | 6 | 8 | 10 => {
                let (top, bottom) = (
                    at(n as f32 + 0.5).max(p.top()),
                    at(n as f32 - 0.5).min(p.bottom()),
                );
                if top < bottom {
                    painter.rect_filled(
                        Rect::from_x_y_ranges(p.x_range(), top..=bottom),
                        0.0,
                        band,
                    );
                }
            }
            4 | 11 if (p.top()..p.bottom()).contains(&at(n as f32 + 0.5)) => {
                painter.hline(p.x_range(), at(n as f32 + 0.5), Stroke::new(px, band));
            }
            _ => {}
        }
    }
    let mut curve = Vec::new();
    if let Some(view) = graphs.data(View::Pitch, ui, module) {
        let reference = 400.0 + 80.0 * setting(state, 12, 0.5);
        let entries = &view.pitch;
        let mut last: Option<(Pos2, f32)> = None;
        for (_, column) in columns(p, cx.m.ppp(), 2.0) {
            let back = x.value((column - p.left()) / p.width());
            let i = entries.len() as isize - 1 - (back / 0.05) as isize;
            let hz = if i >= 0 { entries[i as usize] } else { 0.0 };
            let note = 69.0 + 12.0 * (hz / reference).log2();
            let point = (hz > 0.0 && note.is_finite())
                .then(|| (pos2(column, at(note).clamp(p.top(), p.bottom())), note));
            if let (Some((a, na)), Some((b, nb))) = (last, point) {
                let edge = |q: Pos2| q.y <= p.top() || q.y >= p.bottom();
                if (na - nb).abs() < 7.0 && !edge(a) && !edge(b) {
                    painter.line_segment([a, b], Stroke::new(px, ink));
                }
            }
            curve.extend(point.map(|(q, _)| q));
            last = point;
        }
    }
    let marker = Some(octave(
        state,
        [
            pitch_marker::<0> as fn(f32, f32) -> String,
            pitch_marker::<1>,
        ],
    ));
    graphs.finish(
        ui,
        cx,
        id,
        p,
        View::Pitch,
        state,
        ink,
        (x, y),
        &curve,
        marker,
    );
    graphs.store(ui, module);
    Vec::new()
}

#[cfg(test)]
const TABS: Row = row(
    19,
    Widget::Tabs,
    "SPECTRUM · SCOPE · LOUDNESS · STEREO · PITCH · SETTINGS",
    [0.0, 6.74, 14.0, 1.25],
)
.h(Size::U(1.25));

/// A graph of the five views with a side column.
#[cfg(test)]
const fn graph(id: u32) -> Row {
    row(id, Widget::Display, "", [0.7, 0.3, 11.3, 6.14]).margins(0.3, 0.0, 0.3, 0.7)
}

#[cfg(test)]
const fn side(id: u32, widget: Widget, label: &'static str, top: bool) -> Row {
    let y = if top { 0.0 } else { 3.07 };
    row(id, widget, label, [12.0, y, 2.0, 3.07])
}

#[cfg(test)]
const fn slider(id: u32, label: &'static str) -> Row {
    row(id, Widget::VSlider, label, [12.0, 0.0, 2.0, 6.14])
        .value_text()
        .w(Size::U(2.0))
        .margins(0.0, 0.0, 0.6, 0.0)
}

#[cfg(test)]
pub const REFERENCE: Reference = Reference {
    size: [14.0, 7.99],
    // The controls' defaults; lists and spinners hold the item index.
    settings: &[
        (3, 0.0),
        (4, 0.0),
        (6, 0.5),
        (8, 0.0),
        (9, 1.0),
        (12, 0.5),
        (14, 2.0),
        (15, 1.0),
        (16, 1.0),
        (17, 0.5),
    ],
    cases: &[
        Case {
            name: "SPECTRUM",
            rows: &[
                graph(1),
                side(3, Widget::Spinner, "Mode", true).items("2D · 3D"),
                side(4, Widget::Spinner, "Scale", false).items("Log · Lin"),
                TABS,
            ],
            ..Case::DEFAULT
        },
        // `Mode` 3D: the spectrogram takes the spectrum's slot.
        Case {
            name: "SPECTRUM 3D",
            settings: &[(3, 1.0)],
            rows: &[
                graph(2),
                side(3, Widget::Spinner, "Mode", true).items("2D · 3D"),
                side(4, Widget::Spinner, "Scale", false).items("Log · Lin"),
                TABS,
            ],
            ..Case::DEFAULT
        },
        Case {
            name: "SCOPE",
            tab: 1,
            rows: &[graph(5), slider(6, "Threshold").value("0.10"), TABS],
            ..Case::DEFAULT
        },
        Case {
            name: "LOUDNESS",
            tab: 2,
            rows: &[
                graph(7),
                side(8, Widget::Spinner, "Mode", true)
                    .items("LUFS 0.4s · LUFS 3s · True Peak · RMS · Peak"),
                side(9, Widget::Knob, "Target", false).value("+12.0 dB"),
                TABS,
            ],
            ..Case::DEFAULT
        },
        Case {
            name: "STEREO",
            tab: 3,
            rows: &[
                row(10, Widget::Display, "", [0.7, 0.3, 12.6, 6.14]).margins(0.3, 0.7, 0.3, 0.7),
                TABS,
            ],
            ..Case::DEFAULT
        },
        Case {
            name: "PITCH",
            tab: 4,
            rows: &[graph(11), slider(12, "Reference").value("440 Hz"), TABS],
            ..Case::DEFAULT
        },
        Case {
            name: "SETTINGS",
            tab: 5,
            rows: &[
                row(13, Widget::Spacer, "", [0.0, 0.0, 14.0, 1.62]),
                row(14, Widget::Spinner, "Channel", [0.0, 1.62, 3.5, 3.5])
                    .items("Left · Right · L + R"),
                row(15, Widget::Spinner, "Reactivity", [3.5, 1.62, 3.5, 3.5])
                    .items("Slow · Medium · Fast"),
                row(16, Widget::Spinner, "DC Block", [7.0, 1.62, 3.5, 3.5]).items("On · Off"),
                row(17, Widget::Knob, "Input Gain", [10.5, 1.62, 3.5, 3.5]).value("×1.00"),
                row(18, Widget::Spacer, "", [0.0, 5.12, 14.0, 1.62]),
                TABS,
            ],
            ..Case::DEFAULT
        },
    ],
};

#[cfg(test)]
mod live {
    use eframe::egui::Shape;
    use honeydew_audio::{AudioEffect, MeterSnapshot, ProcessContext};

    use super::*;
    use crate::panels::tests::{U, display_shapes_fed};

    /// The Analyzer's slab after `seconds` of `signal(t) -> (L, R)` at 48 kHz.
    fn slab(seconds: f32, signal: impl Fn(f32) -> (f32, f32)) -> Vec<f32> {
        let mut effect = AudioEffect::new(26, 48_000.0).unwrap();
        let frames = (seconds * 48_000.0) as usize;
        let (mut l, mut r): (Vec<f32>, Vec<f32>) =
            (0..frames).map(|i| signal(i as f32 / 48_000.0)).unzip();
        let context = ProcessContext {
            sample_rate: 48_000.0,
            tempo_bpm: 120.0,
            start_beat: 0.0,
            playing: true,
        };
        for (l, r) in l.chunks_mut(512).zip(r.chunks_mut(512)) {
            effect.process(l, r, context).unwrap();
        }
        effect.analysis().to_vec()
    }

    fn tone(hz: f32, amplitude: f32) -> impl Fn(f32) -> (f32, f32) {
        move |t| {
            let v = amplitude * (std::f32::consts::TAU * hz * t).sin();
            (v, v)
        }
    }

    fn content() -> Rect {
        Rect::from_min_size(pos2(0.0, 0.0), vec2(14.0 * U, 7.99 * U))
    }

    /// What view `leaf` on page `tab` draws with `slab` posted, at `ppp`, with `settings`.
    fn drawn(leaf: u32, tab: usize, settings: &[(u32, f32)], slab: &[f32], ppp: f32) -> Vec<Shape> {
        let mut state = PanelState {
            tab,
            ..PanelState::default()
        };
        state.settings.extend(settings.iter().copied());
        display_shapes_fed(
            26,
            leaf,
            &[],
            &state,
            content(),
            ppp,
            (MeterSnapshot::default(), slab),
        )
    }

    fn texts(shapes: &[Shape]) -> Vec<String> {
        (shapes.iter())
            .filter_map(|s| match s {
                Shape::Text(t) => Some(t.galley.text().to_owned()),
                _ => None,
            })
            .collect()
    }

    #[test]
    fn the_spectrum_frame_labels_follow_the_tick_rule() {
        // The default Log labels are 20 Hz … 20 kHz through the candidates; Y −60 … +6 dB by the candidates.
        let x = Axis {
            lo: 20.0,
            hi: 20_000.0,
            log: Some((20.0, 20_000.0)),
        };
        let xs = labels(&ticks(&x, &SPECTRUM_X, 9), hz_label);
        assert_eq!(
            xs,
            [
                "20 Hz", "50 Hz", "150 Hz", "450 Hz", "1 kHz", "2 kHz", "4 kHz", "7 kHz", "12 kHz",
                "20 kHz"
            ]
        );
        let ys = labels(&ticks(&Axis::linear(-60.0, 6.0), &SPECTRUM_Y, 7), db_label);
        assert_eq!(
            ys,
            [
                "-60 dB", "-50 dB", "-40 dB", "-30 dB", "-20 dB", "-12 dB", "-6 dB", "0 dB",
                "+6 dB"
            ]
        );
        // The oscilloscope's Y has no candidates: 7 even ticks, −1.0 … 1.0 once a decimal tells them apart.
        let scope = labels(&ticks(&Axis::linear(-1.0, 1.0), &[], 7), |v, p| {
            format!("{v:.p$}")
        });
        assert_eq!(scope, ["-1.0", "-0.7", "-0.3", "0.0", "0.3", "0.7", "1.0"]);
        // The log map: 1 kHz is (ln 981 / ln 19981)² of the way.
        let want = (981f32.ln() / 19_981f32.ln()).powi(2);
        assert!((x.pos(1000.0) - want).abs() < 1e-5 && (x.value(want) - 1000.0).abs() < 0.1);
    }

    #[test]
    fn the_scope_triggers_on_a_rising_edge_or_runs_free() {
        let level = 0.1;
        // Silence, then a step up at bucket 850 (150 ms ago): the window is 825…925.
        let mut scope = vec![(0.0, 0.0); 1000];
        for b in &mut scope[850..] {
            *b = (0.5, 0.5);
        }
        assert_eq!(scope_window(&scope, level, 25, 75, 200), 825..925);
        // The same edge 400 ms ago is more than 0.2 s without a trigger: free-running.
        scope.rotate_left(250);
        assert_eq!(scope_window(&scope, level, 25, 75, 200), 900..1000);
        // No trigger: the latest 100.
        assert_eq!(
            scope_window(&[(0.0, 0.0); 1000], level, 25, 75, 200),
            900..1000
        );
        // That edge (now 400 buckets back) in a 0.5 s view with the trigger at half: 250 before it and 250 after.
        assert_eq!(scope_window(&scope, level, 250, 250, 200), 350..850);
    }

    #[test]
    fn the_views_draw_the_engines_analysis() {
        let data = slab(4.0, tone(1000.0, 0.5));
        for ppp in [1.0, 2.0] {
            // Spectrum: the frame, then a fill mesh and the curve, whose highest point is at 1 kHz.
            let shapes = drawn(1, 0, &[], &data, ppp);
            let curve = longest_path(&shapes);
            assert!(shapes.iter().any(|s| matches!(s, Shape::Mesh(_))));
            let Shape::Rect(fill) = &shapes[0] else {
                panic!()
            };
            let p = fill.rect;
            let top = curve
                .points
                .iter()
                .fold(curve.points[0], |a, &b| if b.y < a.y { b } else { a });
            let x = freq_axis(&PanelState::default());
            let at = p.left() + x.pos(1000.0) * p.width();
            assert!(
                (top.x - at).abs() <= 4.0 / ppp,
                "ppp {ppp}: {top:?}, 1 kHz at {at}"
            );
            // −6 dB of a sine, no tilt at 1 kHz: 54/66 up P.
            let want = p.bottom() - 54.0 / 66.0 * p.height();
            assert!(
                (top.y - want).abs() < 0.03 * p.height(),
                "ppp {ppp}: {top:?}, want {want}"
            );
            // Loudness, LUFS 0.4 s: the readouts.
            let shapes = drawn(7, 2, &[], &data, ppp);
            let texts = texts(&shapes);
            assert!(
                texts
                    .iter()
                    .any(|t| t.starts_with("Peak: ") && t.ends_with(" LUFS")),
                "{texts:?}"
            );
            assert!(
                texts.iter().any(|t| t.starts_with("Integrated: ")),
                "{texts:?}"
            );
            assert!(texts.iter().any(|t| t.starts_with("Range: ")), "{texts:?}");
            // Peak mode (item 4): `db`, no integrated.
            let texts = texts_of(drawn(7, 2, &[(8, 4.0)], &data, ppp));
            assert!(texts.iter().any(|t| t == "Peak: -6.0 db"), "{texts:?}");
            assert!(!texts.iter().any(|t| t.starts_with("Integrated")));
            // Vectorscope: a mono petal, and the interferometer's two lines.
            let shapes = drawn(10, 3, &[], &data, ppp);
            assert_eq!(
                shapes
                    .iter()
                    .filter(|s| matches!(s, Shape::Mesh(_)))
                    .count(),
                2
            );
            // Scope: a trace inside P.
            let shapes = drawn(5, 1, &[], &data, ppp);
            assert!(
                shapes
                    .iter()
                    .filter(|s| matches!(s, Shape::Rect(r) if r.fill == Color32::RED))
                    .count()
                    > 20
            );
        }
    }

    fn texts_of(shapes: Vec<Shape>) -> Vec<String> {
        texts(&shapes)
    }

    /// The longest path drawn: a view's curve (the buttons' icons are short).
    fn longest_path(shapes: &[Shape]) -> eframe::egui::epaint::PathShape {
        (shapes.iter())
            .filter_map(|s| match s {
                Shape::Path(p) => Some(p.clone()),
                _ => None,
            })
            .max_by_key(|p| p.points.len())
            .expect("a curve")
    }

    #[test]
    fn the_pitch_track_follows_the_tone_on_its_reference() {
        // 220 Hz is A3 = MIDI 57 on A4 = 440 Hz.
        let data = slab(3.0, tone(220.0, 0.5));
        for ppp in [1.0, 2.0] {
            let shapes = drawn(11, 4, &[], &data, ppp);
            let Shape::Rect(fill) = &shapes[0] else {
                panic!()
            };
            let p = fill.rect;
            let y = p.bottom() - (57.0 - 48.0) / 24.0 * p.height();
            let curve: Vec<_> = (shapes.iter())
                .filter_map(|s| match s {
                    Shape::LineSegment { points, stroke } if stroke.color == Color32::RED => {
                        Some(points)
                    }
                    _ => None,
                })
                .collect();
            assert!(curve.len() > 50, "ppp {ppp}: {}", curve.len());
            assert!(
                curve
                    .iter()
                    .all(|s| (s[0].y - y).abs() < 1.0 && (s[1].y - y).abs() < 1.0)
            );
        }
    }

    #[test]
    fn without_a_feed_each_view_is_its_frame() {
        for (leaf, tab) in [(1, 0), (5, 1), (7, 2), (10, 3), (11, 4)] {
            let mut state = PanelState {
                tab,
                ..PanelState::default()
            };
            state.seed(crate::panels::panel_for(26).unwrap());
            let shapes =
                crate::panels::tests::display_shapes(26, leaf, &[], &state, content(), 1.0);
            assert!(
                !shapes.iter().any(|s| matches!(s, Shape::Mesh(_))),
                "#{leaf}"
            );
        }
    }

    // ---- Freeze, Zoom and the marker, through the panel with synthetic input ----

    /// An Analyzer card in an offscreen context at U = 32 (ppp 1), the rack posting `slab` each frame (`None`: none).
    struct Card {
        ctx: eframe::egui::Context,
        module: ModuleInstance,
        state: PanelState,
        time: f64,
        slab: Option<Vec<f32>>,
        shapes: Vec<Shape>,
        /// The id of the `Ui` the panel draws in.
        ui: Id,
    }

    const CARD: Rect = Rect {
        min: pos2(20.0, 60.0),
        max: pos2(20.0 + 14.0 * U, 60.0 + 7.99 * U),
    };

    impl Card {
        fn new(tab: usize, settings: &[(u32, f32)], slab: Option<Vec<f32>>) -> Card {
            let mut card = Card {
                ctx: eframe::egui::Context::default(),
                module: ModuleInstance::new(26, 2),
                state: PanelState {
                    tab,
                    ..PanelState::default()
                },
                time: 0.0,
                slab,
                shapes: Vec::new(),
                ui: Id::NULL,
            };
            card.state.settings.extend(settings.iter().copied());
            card.frame(vec![]);
            card
        }

        fn frame(&mut self, events: Vec<eframe::egui::Event>) {
            use crate::metrics::{DEFAULT_UI_SCALE_INDEX, Metrics};
            self.time += 1.0 / 60.0;
            let raw = eframe::egui::RawInput {
                screen_rect: Some(Rect::from_min_size(Pos2::ZERO, vec2(800.0, 600.0))),
                time: Some(self.time),
                events,
                ..Default::default()
            };
            let theme = crate::theme::theme_by_id(0);
            let m = Metrics::from_pixels_per_point(1.0, DEFAULT_UI_SCALE_INDEX);
            let cx = UiCtx {
                theme: &theme,
                m: &m,
            };
            let panel = crate::panels::panel_for(26).unwrap();
            let (module, state, slab, ui_id) =
                (&mut self.module, &mut self.state, &self.slab, &mut self.ui);
            let output = self.ctx.run(raw, |ctx| {
                eframe::egui::CentralPanel::default().show(ctx, |ui| {
                    crate::panels::set_text_env(ctx, 120.0, 48_000.0);
                    let feed = slab.as_deref().map(|s| (MeterSnapshot::default(), s));
                    crate::panels::post_feed(ui, module.instance_id, feed);
                    crate::panels::show(ui, &cx, panel, module, state, CARD, Color32::RED);
                    *ui_id = ui.id();
                });
            });
            self.shapes = output.shapes.into_iter().map(|c| c.shape).collect();
        }

        fn button(&mut self, at: Pos2, pressed: bool) {
            self.frame(vec![eframe::egui::Event::PointerButton {
                pos: at,
                button: eframe::egui::PointerButton::Primary,
                pressed,
                modifiers: Default::default(),
            }]);
        }

        fn move_to(&mut self, at: Pos2) {
            self.frame(vec![eframe::egui::Event::PointerMoved(at)]);
        }

        fn click(&mut self, at: Pos2) {
            self.move_to(at);
            self.button(at, true);
            self.button(at, false);
            self.frame(vec![]);
        }

        /// A double tap: two clicks well within 0.3 s, 0.65 s after any click before (egui counts a third click within
        /// 0.6 s as a triple).
        fn double_tap(&mut self, at: Pos2) {
            for _ in 0..40 {
                self.frame(vec![]);
            }
            self.move_to(at);
            for _ in 0..2 {
                self.button(at, true);
                self.button(at, false);
            }
            self.frame(vec![]);
        }

        fn drag(&mut self, from: Pos2, to: Pos2) {
            self.move_to(from);
            self.button(from, true);
            for k in 1..=4 {
                self.move_to(from + (to - from) * k as f32 / 4.0);
            }
            self.button(to, false);
            self.frame(vec![]);
        }

        fn texts(&self) -> Vec<String> {
            texts(&self.shapes)
        }

        fn graphs(&self) -> Graphs {
            let id = self.ui.with(("analyzer graphs", self.module.instance_id));
            self.ctx.data(|d| d.get_temp(id)).expect("the graphs")
        }

        fn shown(&self, n: u32) -> bool {
            let placed = crate::panels::place(
                crate::panels::panel_for(26).unwrap(),
                &self.module,
                &self.state,
                CARD,
                U,
            );
            placed.iter().any(|(l, _)| l.id == n)
        }

        /// P of graph `n`: its control rect less the label margins (none on the vectorscope).
        fn plot(&self, n: u32) -> Rect {
            let placed = crate::panels::place(
                crate::panels::panel_for(26).unwrap(),
                &self.module,
                &self.state,
                CARD,
                U,
            );
            let r = placed
                .into_iter()
                .find(|(l, _)| l.id == n)
                .expect("shown")
                .1;
            if n == 10 {
                r
            } else {
                Rect::from_min_max(
                    pos2(r.left() + 0.6 * U, r.top()),
                    pos2(r.right(), r.bottom() - 0.5 * U),
                )
            }
        }

        /// The centre of graph `n`'s Freeze (k 1.4) or Zoom (k 2.8) button.
        fn tile(&self, n: u32, k: f32) -> Pos2 {
            let p = self.plot(n);
            pos2(p.right() - k * U + 0.5 * U, p.top() + 0.9 * U)
        }

        /// The x where the Spectrum's curve peaks.
        fn peak_x(&self) -> f32 {
            let curve = longest_path(&self.shapes);
            (curve.points.iter())
                .min_by(|a, b| a.y.total_cmp(&b.y))
                .unwrap()
                .x
        }
    }

    /// x of `f` Hz in the default Spectrum's P.
    fn hz_x(p: Rect, f: f32) -> f32 {
        p.left() + freq_axis(&PanelState::default()).pos(f) * p.width()
    }

    #[test]
    fn freeze_holds_the_view_until_it_is_turned_off() {
        let mut card = Card::new(0, &[], Some(slab(4.0, tone(1000.0, 0.5))));
        let p = card.plot(1);
        assert!((card.peak_x() - hz_x(p, 1000.0)).abs() < 4.0);
        card.click(card.tile(1, 1.4));
        assert!(card.graphs().graphs[View::Spectrum as usize].frozen);
        card.slab = Some(slab(4.0, tone(4000.0, 0.5)));
        card.frame(vec![]);
        assert!((card.peak_x() - hz_x(p, 1000.0)).abs() < 4.0, "held");
        card.click(card.tile(1, 1.4));
        assert!((card.peak_x() - hz_x(p, 4000.0)).abs() < 4.0, "live again");
        // The vectorscope has Freeze too, and no Zoom.
        let mut card = Card::new(3, &[], Some(slab(1.0, tone(1000.0, 0.5))));
        card.click(card.tile(10, 1.4));
        card.click(card.tile(10, 2.8));
        let stereo = &card.graphs().graphs[View::Stereo as usize];
        assert!(stereo.frozen && !stereo.zoom);
    }

    #[test]
    fn the_marker_reads_the_curve_where_touched() {
        let mut card = Card::new(0, &[], Some(slab(4.0, tone(1000.0, 0.5))));
        let p = card.plot(1);
        let at = pos2(hz_x(p, 1000.0), p.center().y);
        card.click(at);
        // `%.0f Hz (%s%d) : %.1f dB`: about 1 kHz, B6 (note 83), about −6 dB.
        let readout = (card.texts().into_iter())
            .find(|t| t.contains(" Hz (") && t.ends_with(" dB"))
            .expect("a readout");
        let (hz, rest) = readout.split_once(" Hz (B6) : ").expect(&readout);
        let hz: f32 = hz.parse().unwrap();
        let db: f32 = rest.trim_end_matches(" dB").parse().unwrap();
        assert!(
            (hz - 1000.0).abs() < 30.0 && (-9.0..-5.0).contains(&db),
            "{readout}"
        );
        let text = crate::theme::theme_by_id(0).colors.text.to_color32();
        let dot = card.shapes.iter().any(
            |s| matches!(s, Shape::Circle(c) if c.fill == text && (c.center.x - at.x).abs() < 1.0),
        );
        assert!(dot);
        // Zoom mode hides it.
        card.click(card.tile(1, 2.8));
        assert!(!card.texts().iter().any(|t| t.contains(" Hz (")));
        // The pitch track's readout: note, octave, cents.
        let mut card = Card::new(4, &[], Some(slab(6.0, tone(220.0, 0.5))));
        let p = card.plot(11);
        card.click(pos2(p.right() - 0.5 * U, p.center().y));
        assert!(
            card.texts().iter().any(|t| t == "A4 +0%"),
            "{:?}",
            card.texts()
        );
    }

    #[test]
    fn note_names_take_the_app_octave_offset_of_set_middle_c_to_c4() {
        let off = ["C4", "E4", "G#4", "C5", "E5", "G#5", "C6"];
        let on = ["C3", "E3", "G#3", "C4", "E4", "G#4", "C5"];
        for (offset, labels, b, a) in [(0, off, "B6", "A4"), (1, on, "B5", "A3")] {
            let flagged = |mut card: Card| {
                card.state.flags.insert(flag::OCTAVE_OFFSET, offset);
                card.frame(vec![]);
                card
            };
            // The pitch track's axis (notes 48–72 at first) labels notes 48, 52 … 72: MIDI 60 is the fourth.
            let card = flagged(Card::new(4, &[], None));
            assert_eq!(card.texts()[..7], labels, "{offset}");
            // The spectrum marker on 1 kHz: note 83.
            let mut card = flagged(Card::new(0, &[], Some(slab(4.0, tone(1000.0, 0.5)))));
            let p = card.plot(1);
            card.click(pos2(hz_x(p, 1000.0), p.center().y));
            let note = format!(" Hz ({b}) : ");
            assert!(
                card.texts().iter().any(|t| t.contains(&note)),
                "{offset}: {:?}",
                card.texts()
            );
            // The pitch track's marker on 220 Hz: note 57.
            let mut card = flagged(Card::new(4, &[], Some(slab(6.0, tone(220.0, 0.5)))));
            let p = card.plot(11);
            card.click(pos2(p.right() - 0.5 * U, p.center().y));
            let note = format!("{a} +0%");
            assert!(card.texts().contains(&note), "{offset}: {:?}", card.texts());
        }
    }

    #[test]
    fn zoom_shows_the_full_range_and_its_selection_becomes_the_view() {
        let mut card = Card::new(0, &[], Some(slab(1.0, tone(1000.0, 0.5))));
        let has = |card: &Card, label: &str| card.texts().iter().any(|t| t == label);
        assert!(has(&card, "+6 dB") && has(&card, "-60 dB") && !has(&card, "-90 dB"));
        card.click(card.tile(1, 2.8));
        assert!(card.graphs().graphs[View::Spectrum as usize].zoom);
        assert!(has(&card, "+12 dB") && has(&card, "-90 dB"));
        // The overlay over the default selection: its outline in `text`, and four handles.
        let text = crate::theme::theme_by_id(0).colors.text.to_color32();
        let handles = (card.shapes.iter())
            .filter(|s| matches!(s, Shape::Circle(c) if c.fill == text && c.radius == 0.2 * U))
            .count();
        assert_eq!(handles, 4);
        // Its top left corner dragged to P's top left: −60…+12 dB, then the view once zoom is off.
        let p = card.plot(1);
        let corner = pos2(p.left(), p.bottom() - (DB_Y + DB_H) * p.height());
        card.drag(corner, p.left_top());
        let graph = &card.graphs().graphs[View::Spectrum as usize];
        assert!(
            (graph.sel.y + graph.sel.h - 1.0).abs() < 1e-4,
            "{:?}",
            graph.sel
        );
        assert_eq!(graph.user, Some(graph.sel));
        card.click(card.tile(1, 2.8));
        assert!(has(&card, "+12 dB") && has(&card, "-60 dB") && !has(&card, "-90 dB"));
        // The button presses never reached the plot: no marker, the selection where the drag left it.
        let graph = &card.graphs().graphs[View::Spectrum as usize];
        assert_eq!((graph.marker, graph.user), (None, Some(graph.sel)));
        // Releasing the Spectrum's zoom copies its selection onto the Spectrogram.
        let graphs = card.graphs();
        let spectrogram = &graphs.graphs[View::Spectrogram as usize];
        assert_eq!(spectrogram.view, graphs.graphs[View::Spectrum as usize].sel);
    }

    #[test]
    fn the_selection_keeps_its_minimum_and_a_double_tap_cycles_it() {
        let mut card = Card::new(0, &[], None);
        card.click(card.tile(1, 2.8));
        let p = card.plot(1);
        // The bottom right corner dragged far up and left stops at 0.2 × 0.5.
        let sel = card.graphs().graphs[View::Spectrum as usize].sel;
        card.drag(sel.rect(p).right_bottom(), p.left_top());
        let user = card.graphs().graphs[View::Spectrum as usize].sel;
        assert!(
            (user.w - 0.2).abs() < 1e-4 && (user.h - 0.5).abs() < 1e-4,
            "{user:?}"
        );
        // Dragging inside moves it, inside the full range.
        card.drag(
            user.rect(p).center(),
            user.rect(p).center() + vec2(-3.0 * p.width(), 0.0),
        );
        let moved = card.graphs().graphs[View::Spectrum as usize].sel;
        assert_eq!((moved.x, moved.w), (0.0, user.w));
        // Default → whole → the last user rectangle → default.
        let tap = p.center() + vec2(0.0, 0.3 * p.height());
        let mut stops = Vec::new();
        for _ in 0..4 {
            card.double_tap(tap);
            stops.push(card.graphs().graphs[View::Spectrum as usize].sel);
        }
        assert_eq!(
            stops,
            [
                View::Spectrum.fresh(),
                Sel::WHOLE,
                moved,
                View::Spectrum.fresh()
            ]
        );
    }

    #[test]
    fn a_control_update_turns_zoom_off_and_shows_each_stored_selection() {
        let mut card = Card::new(0, &[], None);
        card.click(card.tile(1, 2.8));
        assert!(card.graphs().graphs[View::Spectrum as usize].zoom);
        // `Scale` Lin.
        card.state.settings.insert(4, 1.0);
        card.frame(vec![]);
        let graphs = card.graphs();
        assert!(graphs.graphs.iter().all(|g| !g.zoom && g.view == g.sel));
        // The scope shows 0.1 s until then, and its stored 0.5 s after.
        let mut card = Card::new(1, &[], None);
        let has = |card: &Card, label: &str| card.texts().iter().any(|t| t == label);
        assert!(has(&card, "-25 ms") && has(&card, "75 ms"));
        card.state.settings.insert(6, 0.6);
        card.frame(vec![]);
        assert!(
            has(&card, "-125 ms") && has(&card, "375 ms"),
            "{:?}",
            card.texts()
        );
    }

    #[test]
    fn the_spectrograms_zoom_is_done_on_the_spectrum() {
        let mut card = Card::new(0, &[(3, 1.0)], None);
        assert!(card.shown(2) && !card.shown(1));
        card.click(card.tile(2, 2.8));
        assert!(card.shown(1) && !card.shown(2));
        assert!(card.graphs().graphs[View::Spectrum as usize].zoom);
        // Narrowed to 0.2 × 0.5 there, then released: the Spectrogram shows it.
        let p = card.plot(1);
        let sel = card.graphs().graphs[View::Spectrum as usize].sel;
        card.drag(sel.rect(p).right_bottom(), p.left_top());
        card.click(card.tile(1, 2.8));
        assert!(card.shown(2) && !card.shown(1));
        let graphs = card.graphs();
        let spectrogram = &graphs.graphs[View::Spectrogram as usize];
        assert_eq!(spectrogram.view, graphs.graphs[View::Spectrum as usize].sel);
        assert!((spectrogram.view.w - 0.2).abs() < 1e-4);
    }

    #[test]
    fn choosing_3d_empties_the_spectrogram_history() {
        let mut card = Card::new(0, &[(3, 1.0)], Some(slab(1.0, tone(1000.0, 0.5))));
        for _ in 0..30 {
            card.frame(vec![]);
        }
        assert!(card.graphs().history.len() >= 4);
        card.state.settings.insert(3, 0.0);
        card.frame(vec![]);
        card.state.settings.insert(3, 1.0);
        card.frame(vec![]);
        assert_eq!(card.graphs().history.len(), 1);
    }

    #[test]
    fn spectrogram_rows_need_history_and_stay_below_ps_top() {
        let fill = crate::theme::theme_by_id(0)
            .colors
            .control_fill
            .to_color32();
        let rows = |card: &Card| {
            (card.shapes.iter())
                .filter(|s| matches!(s, Shape::Mesh(m) if m.vertices.first().is_some_and(|v| v.color == fill)))
                .count()
        };
        // One spectrum kept: only the front row. 0.6 s later: rows 0 and 1 (0.45 s back), not row 2 (0.9 s back).
        let mut card = Card::new(0, &[(3, 1.0)], Some(slab(1.0, tone(1000.0, 0.5))));
        assert_eq!(rows(&card), 1);
        for _ in 0..36 {
            card.frame(vec![]);
        }
        assert_eq!(rows(&card), 2);
        // A spectrum far above the dB range: every row's segments would leave P's top, so none is drawn.
        let mut loud = slab(1.0, tone(1000.0, 0.5));
        loud[engine::SPECTRUM_AT..engine::SPECTRUM_AT + engine::BINS].fill(200.0);
        let card = Card::new(0, &[(3, 1.0)], Some(loud));
        assert_eq!(rows(&card), 0);
        assert!(
            !card
                .shapes
                .iter()
                .any(|s| matches!(s, Shape::Path(p) if p.points.len() > 20))
        );
    }

    #[test]
    fn the_scope_selection_stays_symmetric_down_to_its_minimum() {
        // The stored 0.5 s × ±1.0: its top right handle dragged past the centre to P's bottom stops at 0.1 high,
        // centred, its width and left edge kept.
        let mut card = Card::new(1, &[], None);
        card.click(card.tile(5, 2.8));
        let p = card.plot(5);
        let sel = card.graphs().graphs[View::Scope as usize].sel;
        card.drag(
            sel.rect(p).right_top(),
            pos2(sel.rect(p).right(), p.bottom()),
        );
        let sel = card.graphs().graphs[View::Scope as usize].sel;
        assert!(
            (sel.h - 0.1).abs() < 1e-6 && (sel.y - 0.45).abs() < 1e-6,
            "{sel:?}"
        );
        assert_eq!((sel.x, sel.w), (0.0, 0.5));
    }

    #[test]
    fn the_scope_trigger_handle_moves_the_trigger() {
        let mut card = Card::new(1, &[], None);
        card.click(card.tile(5, 2.8));
        let p = card.plot(5);
        // The stored 0.5 s with the trigger at 25 %: its handle at 0.125 of P on P's top; dragged to 0.25 of P, the
        // trigger is at half the selection.
        let handle = pos2(p.left() + 0.125 * p.width(), p.top());
        card.drag(handle, pos2(p.left() + 0.25 * p.width(), p.top() + 3.0));
        assert!(
            (card.graphs().trigger - 0.5).abs() < 0.01,
            "{}",
            card.graphs().trigger
        );
        card.click(card.tile(5, 2.8));
        let has = |card: &Card, label: &str| card.texts().iter().any(|t| t == label);
        assert!(
            has(&card, "-250 ms") && has(&card, "250 ms"),
            "{:?}",
            card.texts()
        );
        // A loaded module's trigger comes from its blob (byte 76), on its re-applied 0.5 s.
        let mut blob =
            crate::panels::cstm::cstm_from_settings(26, &Default::default(), None).unwrap();
        blob[76..80].copy_from_slice(&0.75f32.to_le_bytes());
        let mut card = Card::new(1, &[], None);
        card.ctx = eframe::egui::Context::default();
        card.module.file.custom_data = Some(blob);
        card.frame(vec![]);
        assert_eq!(card.graphs().trigger, 0.75);
        assert!(
            has(&card, "-375 ms") && has(&card, "125 ms"),
            "{:?}",
            card.texts()
        );
    }
}
