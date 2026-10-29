//! Equalizer (ID 3), UIHeight 10 U.
//!
//! The EQ curve #2 is module-drawn ([`curve`]).
//!
//! The display area is filled `controlFill`. Normalised horizontal position x ∈ [0, 1] across it is the band frequency
//! parameter value itself (a log-frequency axis, since frequency = 32·608.8^v Hz). Vertical: 0 dB is mid-height and
//! ±12 dB fills the height.
//!
//! 1. A 1 px `moduleFill` line at mid-height (the 0 dB line).
//! 2. **Response bars** in `menuFill`: one bar every 4t px, 2t wide, rising from the bottom edge to (s + 1)·h/2, where
//!    s is the summed response clamped to [−1, 1]. Each band adds (2·gain − 1)·(½ − ½·cos(π·b)). Here
//!    b = d^(129 − 128·(bw + 0.002)^0.05) and d = 1 − |f − x′|. x′ is x for Peak, max(x, f) for LowShelf and min(x, f)
//!    for HiShelf, so a shelf is flat beyond its corner. Gain, frequency, bandwidth and kind are the band's normalised
//!    parameters.
//! 3. **Band handles**, non-selected bands first:
//! - A 1 px vertical `menuFill` line at the band frequency, full height.
//! - A disc at (frequency, bottom − gain·h) of radius trunc(0.3 U) + 0.25 U·p. Its colour is
//!   lerp(`menuFill`, `text`, 0.3·p) (at most 30 % `text`). p = proximity of the last pointer position (hover
//!   counts): max(0, (0.75 U − distance)/0.75 U). No ring or outline.
//! - The band number `1`…`4` centred on the disc, FontSize 2, `background` colour.
//! 4. The selected band (= current tab) is drawn last the same way, but the line is channel-coloured and the disc is
//!    lerp(channel, `text`, 0.3·p) (p = 1 while it is being dragged).
//!
//! **Touch.**
//! - A touch picks the nearest handle within its half-size + 1 U and selects that band's tab.
//! - Dragging (after 0.1 s) sets frequency from x and gain from y. Both are clamped to the display.
//! - A double tap (< 0.3 s) on a handle resets that band's gain and frequency to their defaults.
//!
//! a bar samples the response at its left edge. The 1 px lines sit on the device pixel at or before their
//! position (inside the display). Bars are cut at the display's right edge; handles are not clipped to it. The last
//! pointer position is egui's pointer anywhere on screen, hovering or pressed, kept relative to the display after the
//! pointer is gone (a lifted touch, a mouse that left the window). Touches only count on the display. A handle's
//! half-size is its disc radius. Only a press that picks a handle takes the drag; one from elsewhere scrolls the rack
//! as before. A drag moves the picked handle only when the pointer moves at least 0.1 s after the press. The double
//! tap is two presses on the same handle less than 0.3 s apart, neither of which dragged, and a band's defaults are
//! The registry's reset values.

use std::f32::consts::PI;

use eframe::egui::{Color32, Id, Pos2, Rect, Sense, Ui, Vec2, pos2};

use crate::metrics::Metrics;
use crate::panels::build::*;
use crate::params::module_spec;
use crate::ui::UiCtx;

const KIND: &str = "LowShelf · Peak · HiShelf";
const BANDS: usize = 4;
/// Touch: how long a press waits before it drags, and the most between the presses of a double tap (s).
const DRAG_DELAY: f64 = 0.1;
const DOUBLE_TAP: f64 = 0.3;

/// Band `k`'s (from 0) gain and frequency params.
const fn gain_p(k: usize) -> usize {
    1 + 4 * k
}
const fn freq_p(k: usize) -> usize {
    2 + 4 * k
}

/// Where a display's band handles keep their params: band `k`'s (from 0) gain and frequency, each normalised, which
/// place its handle (step 3) and which its drag sets (**Touch**). The Dynamic EQ's display shares the handles.
#[derive(Clone, Copy)]
pub(super) struct Handles {
    pub gain: fn(usize) -> usize,
    pub freq: fn(usize) -> usize,
}

const HANDLES: Handles = Handles {
    gain: gain_p,
    freq: freq_p,
};

impl Handles {
    /// Step 3: band `k`'s disc centre, (frequency, bottom − gain·h); a param the module lacks reads 0.5.
    fn centre(self, rect: Rect, params: &[f32], k: usize) -> Pos2 {
        let v = |p: usize| params.get(p).copied().unwrap_or(0.5);
        pos2(
            rect.left() + v((self.freq)(k)) * rect.width(),
            rect.bottom() - v((self.gain)(k)) * rect.height(),
        )
    }
}

/// Band `k`'s normalised gain, frequency, bandwidth and kind; a param the module lacks reads 0.5.
fn band(params: &[f32], k: usize) -> [f32; 4] {
    std::array::from_fn(|i| params.get(gain_p(k) + i).copied().unwrap_or(0.5))
}

/// Step 2: the summed response of the 4 bands at normalised `x`, clamped to [−1, 1] (±12 dB).
pub(crate) fn response(params: &[f32], x: f32) -> f32 {
    let s = (0..BANDS).map(|k| {
        let [gain, f, bw, kind] = band(params, k);
        // The shape the DSP plays, trunc(2v) in f64, not the selector's item.
        let x = match (2.0 * f64::from(kind)) as u8 {
            0 => x.max(f),
            2 => x.min(f),
            _ => x,
        };
        let d = 1.0 - (f - x).abs();
        let b = d.powf(129.0 - 128.0 * (bw + 0.002).powf(0.05));
        (2.0 * gain - 1.0) * (0.5 - 0.5 * (PI * b).cos())
    });
    s.sum::<f32>().clamp(-1.0, 1.0)
}

/// Step 2: the response bars in `rect`, one every 4t from the left edge, 2t wide (cut at the right edge), from the
/// bottom up to (s + 1)·h/2.
pub(crate) fn bars(rect: Rect, m: &Metrics, params: &[f32]) -> Vec<Rect> {
    response_bars(rect, m, |x| response(params, x))
}

/// [`bars`] of response `s(x)` in [−1, 1] at normalised `x`.
pub(super) fn response_bars(rect: Rect, m: &Metrics, s: impl Fn(f32) -> f32) -> Vec<Rect> {
    let (step, w) = (4.0 * m.t(), 2.0 * m.t());
    let n = (rect.width() / step).ceil() as usize;
    (0..n)
        .map(|i| {
            let x = rect.left() + i as f32 * step;
            let s = s((x - rect.left()) / rect.width());
            let top = rect.bottom() - (s + 1.0) * rect.height() / 2.0;
            Rect::from_min_max(pos2(x, top), pos2((x + w).min(rect.right()), rect.bottom()))
        })
        .collect()
}

/// Step 3: band `k`'s disc centre, (frequency, bottom − gain·h).
#[cfg(test)]
pub(crate) fn centre(rect: Rect, params: &[f32], k: usize) -> Pos2 {
    HANDLES.centre(rect, params, k)
}

/// Step 3: a disc's radius at pointer proximity `p`, trunc(0.3 U) device px + 0.25 U·p.
pub(crate) fn radius(m: &Metrics, p: f32) -> f32 {
    (0.3 * m.u_px()).trunc() / m.ppp() + m.u(0.25) * p
}

/// Touch: the band whose handle is nearest `at` within its radius + 1 U; ties go to the lower band.
fn pick(rect: Rect, m: &Metrics, params: &[f32], at: Pos2, h: Handles) -> Option<usize> {
    let reach = radius(m, 0.0) + m.u(1.0);
    (0..BANDS)
        .map(|k| (k, at.distance(h.centre(rect, params, k))))
        .filter(|&(_, d)| d <= reach)
        .min_by(|a, b| a.1.total_cmp(&b.1))
        .map(|(k, _)| k)
}

/// The touch state in egui temp data under the display's id: the band the press holds and since when, the last
/// press on a handle (band, time) for the double tap, and the last pointer position from the display's corner.
#[derive(Clone, Copy, Default)]
struct Touch {
    held: Option<(usize, f64)>,
    last: Option<(usize, f64)>,
    pointer: Option<Vec2>,
}

/// What a pass of [`touch`] leaves for the drawing: the params it changed, the last pointer position and the band
/// the press holds.
pub(super) struct Touched {
    pub set: Vec<usize>,
    pointer: Option<Pos2>,
    held: Option<usize>,
}

/// The display's touch handling (the module doc's **Touch**), for handles `h`.
pub(super) fn touch(
    ui: &Ui,
    m: &Metrics,
    id: Id,
    rect: Rect,
    module: &mut ModuleInstance,
    state: &mut PanelState,
    h: Handles,
) -> Touched {
    // Clicks only: the drag is claimed when a press picks a handle, so one from elsewhere scrolls the rack.
    let response = ui.interact(rect, id, Sense::click());
    let (now, pressed, moved, down, pointer) = ui.input(|i| {
        let p = &i.pointer;
        let moved = p.delta() != Vec2::ZERO;
        (
            i.time,
            p.primary_pressed(),
            moved,
            p.primary_down(),
            p.latest_pos(),
        )
    });
    let mut t: Touch = ui.data(|d| d.get_temp(id)).unwrap_or_default();
    t.pointer = pointer.map(|p| p - rect.min).or(t.pointer);
    // egui's long touch (a touch held still 0.8 s) drops the press: take it back as a drag.
    if response.long_touched() && t.held.is_some() {
        ui.ctx().set_dragged_id(id);
    }
    let at = response.interact_pointer_pos().filter(|_| {
        response.is_pointer_button_down_on()
            || response.long_touched()
            || (pressed && response.clicked())
    });
    let mut set = Vec::new();
    match (at, t.held) {
        (Some(at), _) if pressed => {
            let k = pick(rect, m, &module.params, at, h);
            t.held = k.map(|k| (k, now));
            if k.is_some() && down {
                ui.ctx().set_dragged_id(id);
            }
            let double = t
                .last
                .is_some_and(|(j, then)| Some(j) == k && now - then < DOUBLE_TAP);
            t.last = if double { None } else { k.map(|k| (k, now)) };
            if let Some(k) = k {
                state.tab = k;
            }
            if let (Some(k), true, Some(spec)) = (k, double, module_spec(module.type_id)) {
                for p in [(h.gain)(k), (h.freq)(k)] {
                    if let Some(s) = spec.params.get(p) {
                        write(module, p, s.reset, &mut set);
                    }
                }
            }
        }
        (Some(at), Some((k, since))) if moved && now - since >= DRAG_DELAY => {
            let f = ((at.x - rect.left()) / rect.width()).clamp(0.0, 1.0);
            let gain = ((rect.bottom() - at.y) / rect.height()).clamp(0.0, 1.0);
            write(module, (h.freq)(k), f, &mut set);
            write(module, (h.gain)(k), gain, &mut set);
            // A press that dragged is no tap.
            t.last = None;
        }
        (Some(_), _) => {}
        (None, _) => t.held = None,
    }
    ui.data_mut(|d| d.insert_temp(id, t));
    Touched {
        set,
        pointer: t.pointer.map(|v| rect.min + v),
        held: t.held.map(|(k, _)| k),
    }
}

/// Sets param `p` to `v` and records it in `set` when that changes it.
fn write(module: &mut ModuleInstance, p: usize, v: f32, set: &mut Vec<usize>) {
    if let Some(slot) = module.params.get_mut(p).filter(|s| **s != v) {
        *slot = v;
        set.push(p);
    }
}

/// The EQ curve (#2): steps 1–4 after the touch.
fn curve(
    ui: &mut Ui,
    cx: &UiCtx,
    id: Id,
    rect: Rect,
    module: &mut ModuleInstance,
    state: &mut PanelState,
    ink: Color32,
) -> Vec<usize> {
    let touched = touch(ui, cx.m, id, rect, module, state, HANDLES);
    let (m, colors, params) = (cx.m, &cx.theme.colors, &module.params);
    let painter = ui.painter();
    let px = 1.0 / m.ppp();
    painter.rect_filled(rect, 0.0, colors.control_fill.to_color32());
    let mid = m.floor_px(rect.center().y);
    let zero = Rect::from_x_y_ranges(rect.x_range(), mid..=mid + px);
    painter.rect_filled(zero, 0.0, colors.module_fill.to_color32());
    for bar in bars(rect, m, params) {
        painter.rect_filled(bar, 0.0, colors.menu_fill.to_color32());
    }
    handles(ui, cx, rect, params, state.tab, &touched, ink, HANDLES);
    touched.set
}

/// Steps 3 and 4: the band handles `h` in `rect`, band `selected` last in `ink`, after a [`touch`] pass.
#[allow(clippy::too_many_arguments)]
pub(super) fn handles(
    ui: &Ui,
    cx: &UiCtx,
    rect: Rect,
    params: &[f32],
    selected: usize,
    &Touched { pointer, held, .. }: &Touched,
    ink: Color32,
    h: Handles,
) {
    let (m, colors) = (cx.m, &cx.theme.colors);
    let [menu, text] = [colors.menu_fill, colors.text].map(|c| c.to_color32());
    let painter = ui.painter();
    let px = 1.0 / m.ppp();
    let order = (0..BANDS)
        .filter(|&k| k != selected)
        .chain((selected < BANDS).then_some(selected));
    for k in order {
        let c = h.centre(rect, params, k);
        let p = match pointer {
            _ if held == Some(k) => 1.0,
            Some(at) => ((m.u(0.75) - at.distance(c)) / m.u(0.75)).max(0.0),
            _ => 0.0,
        };
        let line = if k == selected { ink } else { menu };
        let disc = line.lerp_to_gamma(text, 0.3 * p);
        let x = m.floor_px(c.x).min(rect.right() - px);
        painter.rect_filled(Rect::from_x_y_ranges(x..=x + px, rect.y_range()), 0.0, line);
        painter.circle_filled(c, radius(m, p), disc);
        let font = m.font_id(FontSize::Pt10);
        let name = (k + 1).to_string();
        painter.text(
            c,
            Align2::CENTER_CENTER,
            name,
            font,
            colors.background.to_color32(),
        );
    }
}

pub fn panel() -> Option<Panel> {
    let band = |k: usize, first: u32| {
        let p = 1 + 4 * k;
        hstack([
            knob(first, "Gain").p(p),
            knob(first + 1, "Frequency").p(p + 1),
            knob(first + 2, "Bandwidth").p(p + 2),
            spinner(first + 3, "Channel").p(17 + k),
            list(first + 4, KIND, Bar::Left).p(p + 3),
        ])
        .show(Show::Tab(k))
    };
    Some(Panel::new(vstack([
        vstack([
            tabs(1, "BAND 1 · BAND 2 · BAND 3 · BAND 4").h(Size::U(1.25)),
            display(2).draw(curve),
        ]),
        band(0, 3),
        band(1, 8),
        band(2, 13),
        band(3, 18),
    ])))
}

#[cfg(test)]
const TABS: Row = row(
    1,
    Widget::Tabs,
    "BAND 1 · BAND 2 · BAND 3 · BAND 4",
    [0.0, 0.0, 14.0, 1.25],
)
.h(Size::U(1.25));
#[cfg(test)]
const CURVE: Row = row(2, Widget::Display, "", [0.0, 1.25, 14.0, 2.74]);

#[cfg(test)]
#[rustfmt::skip]
pub const REFERENCE: Reference = Reference {
    size: [14.0, 7.99],
    settings: &[],
    cases: &[
        Case {
            name: "BAND 1",
            rows: &[
                TABS,
                CURVE,
                row(3, Widget::Knob, "Gain", [0.0, 3.99, 2.8, 4.0]).p(1),
                row(4, Widget::Knob, "Frequency", [2.8, 3.99, 2.8, 4.0]).p(2),
                row(5, Widget::Knob, "Bandwidth", [5.6, 3.99, 2.8, 4.0]).p(3),
                row(6, Widget::Spinner, "Channel", [8.4, 3.99, 2.8, 4.0]).p(17),
                row(7, Widget::List(Bar::Left), KIND, [11.2, 3.99, 2.8, 4.0]).p(4),
            ],
            ..Case::DEFAULT
        },
        Case {
            name: "BAND 2",
            tab: 1,
            rows: &[
                TABS,
                CURVE,
                row(8, Widget::Knob, "Gain", [0.0, 3.99, 2.8, 4.0]).p(5),
                row(9, Widget::Knob, "Frequency", [2.8, 3.99, 2.8, 4.0]).p(6),
                row(10, Widget::Knob, "Bandwidth", [5.6, 3.99, 2.8, 4.0]).p(7),
                row(11, Widget::Spinner, "Channel", [8.4, 3.99, 2.8, 4.0]).p(18),
                row(12, Widget::List(Bar::Left), KIND, [11.2, 3.99, 2.8, 4.0]).p(8),
            ],
            ..Case::DEFAULT
        },
        Case {
            name: "BAND 3",
            tab: 2,
            rows: &[
                TABS,
                CURVE,
                row(13, Widget::Knob, "Gain", [0.0, 3.99, 2.8, 4.0]).p(9),
                row(14, Widget::Knob, "Frequency", [2.8, 3.99, 2.8, 4.0]).p(10),
                row(15, Widget::Knob, "Bandwidth", [5.6, 3.99, 2.8, 4.0]).p(11),
                row(16, Widget::Spinner, "Channel", [8.4, 3.99, 2.8, 4.0]).p(19),
                row(17, Widget::List(Bar::Left), KIND, [11.2, 3.99, 2.8, 4.0]).p(12),
            ],
            ..Case::DEFAULT
        },
        Case {
            name: "BAND 4",
            tab: 3,
            rows: &[
                TABS,
                CURVE,
                row(18, Widget::Knob, "Gain", [0.0, 3.99, 2.8, 4.0]).p(13),
                row(19, Widget::Knob, "Frequency", [2.8, 3.99, 2.8, 4.0]).p(14),
                row(20, Widget::Knob, "Bandwidth", [5.6, 3.99, 2.8, 4.0]).p(15),
                row(21, Widget::Spinner, "Channel", [8.4, 3.99, 2.8, 4.0]).p(20),
                row(22, Widget::List(Bar::Left), KIND, [11.2, 3.99, 2.8, 4.0]).p(16),
            ],
            ..Case::DEFAULT
        },
    ],
};

#[cfg(test)]
mod tests {
    use eframe::egui::{Event, PointerButton, Shape, pos2, vec2};

    use super::*;
    use crate::metrics::DEFAULT_UI_SCALE;
    use crate::panels::tests::{Rig, U, card, display_shapes};
    use crate::params::{menu_value, module_spec};
    use crate::theme::theme_by_id;

    const TYPE_ID: u16 = 3;

    /// The reference display rect (0, 1.25, 14, 2.74 U).
    const RECT: Rect = Rect {
        min: pos2(0.0, 1.25 * U),
        max: pos2(14.0 * U, 3.99 * U),
    };

    fn metrics(ppp: f32) -> Metrics {
        Metrics::from_pixels_per_point(ppp, DEFAULT_UI_SCALE)
    }

    fn defaults() -> Vec<f32> {
        module_spec(TYPE_ID)
            .unwrap()
            .params
            .iter()
            .map(|p| p.default)
            .collect()
    }

    /// The defaults with band 1 at `gain`, `f` and kind `kind` (0 LowShelf, 1 Peak, 2 HiShelf).
    fn one_band(gain: f32, f: f32, kind: usize) -> Vec<f32> {
        let mut p = defaults();
        (p[1], p[2], p[4]) = (gain, f, menu_value(kind, 3));
        p
    }

    fn close(a: f32, b: f32) -> bool {
        (a - b).abs() < 1e-4
    }

    #[test]
    fn the_response_at_the_defaults_is_flat_at_0_db() {
        for i in 0..=100 {
            assert_eq!(response(&defaults(), i as f32 / 100.0), 0.0);
        }
    }

    #[test]
    fn a_peak_at_full_gain_reaches_1_at_its_frequency_and_falls_off_symmetrically() {
        let p = one_band(1.0, 0.5, 1);
        assert!(close(response(&p, 0.5), 1.0));
        let mut last = 1.0;
        for d in [0.02, 0.05, 0.1, 0.2, 0.4] {
            let (lo, hi) = (response(&p, 0.5 - d), response(&p, 0.5 + d));
            assert!(close(lo, hi), "{d}: {lo} {hi}");
            assert!(lo < last, "{d}: {lo} after {last}");
            last = lo;
        }
        // At 0 gain the same peak is a dip to −1.
        assert!(close(response(&one_band(0.0, 0.5, 1), 0.5), -1.0));
    }

    #[test]
    fn the_curve_reads_each_kind_as_the_dsp_plays_it() {
        // Kind is trunc(2v) in f64 in the DSP: 0.75 is a Peak, not the selector's nearest item HiShelf.
        let mut p = one_band(1.0, 0.5, 1);
        p[4] = 0.75;
        for x in [0.2, 0.8] {
            assert!(
                close(response(&p, x), response(&one_band(1.0, 0.5, 1), x)),
                "{x}"
            );
        }
    }

    #[test]
    fn a_shelf_is_flat_beyond_its_corner() {
        let low = one_band(1.0, 0.5, 0);
        let high = one_band(1.0, 0.5, 2);
        for x in [0.0, 0.2, 0.5] {
            assert!(close(response(&low, x), 1.0), "low {x}");
            assert!(close(response(&high, 1.0 - x), 1.0), "high {}", 1.0 - x);
        }
        assert!(response(&low, 0.7) < 0.9);
        assert!(response(&high, 0.3) < 0.9);
    }

    #[test]
    fn the_sum_is_clamped_to_plus_minus_1() {
        let mut p = one_band(1.0, 0.5, 1);
        (p[5], p[6], p[8]) = (1.0, 0.5, menu_value(1, 3));
        assert_eq!(response(&p, 0.5), 1.0);
        (p[1], p[5]) = (0.0, 0.0);
        assert_eq!(response(&p, 0.5), -1.0);
    }

    #[test]
    fn a_wider_bandwidth_falls_off_slower() {
        let mut narrow = one_band(1.0, 0.5, 1);
        let mut wide = narrow.clone();
        (narrow[3], wide[3]) = (0.1, 0.9);
        assert!(response(&narrow, 0.6) < response(&wide, 0.6));
    }

    #[test]
    fn bars_are_every_4t_2t_wide_up_to_the_response() {
        for ppp in [1.0, 2.0] {
            let m = metrics(ppp);
            // t = 2 points at U = 32 points either way: 56 bars 8 apart, 4 wide.
            let flat = bars(RECT, &m, &defaults());
            assert_eq!(flat.len(), 56, "ppp {ppp}");
            for (i, b) in flat.iter().enumerate() {
                assert_eq!(b.left(), 8.0 * i as f32, "ppp {ppp} bar {i}");
                assert_eq!(b.width(), 4.0);
                assert_eq!(b.bottom(), RECT.bottom());
                // s = 0: half the height.
                assert!(close(b.top(), RECT.center().y), "{b:?}");
            }
            // A full-gain peak at bar 28's left edge (x = 224 = w/2) fills it to the top.
            let peak = bars(RECT, &m, &one_band(1.0, 0.5, 1));
            assert!(close(peak[28].top(), RECT.top()));
            assert!(peak[0].top() > RECT.top() + 1.0);
        }
    }

    #[test]
    fn a_bar_past_the_right_edge_is_cut_there() {
        let m = metrics(1.0);
        let rect = RECT.with_max_x(441.0);
        let cut = bars(rect, &m, &defaults());
        assert_eq!(cut.len(), 56);
        assert_eq!(cut[55].right(), 441.0);
    }

    #[test]
    fn a_handle_sits_at_frequency_and_gain_with_its_radius_growing_by_proximity() {
        let mut p = defaults();
        (p[9], p[10]) = (0.75, 0.25);
        let c = centre(RECT, &p, 2);
        assert!(close(c.x, 0.25 * 14.0 * U) && close(c.y, RECT.bottom() - 0.75 * 2.74 * U));
        // trunc(0.3 U) device px, then + 0.25 U·p.
        for (ppp, rest) in [(1.0, 9.0), (2.0, 9.5)] {
            let m = metrics(ppp);
            assert_eq!(radius(&m, 0.0), rest, "ppp {ppp}");
            assert_eq!(radius(&m, 1.0), rest + 8.0, "ppp {ppp}");
            assert_eq!(radius(&m, 0.5), rest + 4.0, "ppp {ppp}");
        }
    }

    /// The reference content rect, 14 × 7.99 U.
    fn content() -> Rect {
        Rect::from_min_size(pos2(0.0, 0.0), vec2(14.0 * U, 7.99 * U))
    }

    fn on_tab(tab: usize) -> PanelState {
        PanelState {
            tab,
            ..PanelState::default()
        }
    }

    #[test]
    fn the_display_draws_fill_0_db_line_bars_then_the_handles_in_theme_roles() {
        let colors = theme_by_id(0).colors;
        let [control, module, menu, background] = [
            colors.control_fill,
            colors.module_fill,
            colors.menu_fill,
            colors.background,
        ]
        .map(|c| c.to_color32());
        for ppp in [1.0, 2.0] {
            let shapes = display_shapes(TYPE_ID, 2, &[], &on_tab(0), content(), ppp);
            assert_eq!(shapes.len(), 2 + 56 + 4 * 3, "ppp {ppp}");
            let rect = match &shapes[0] {
                Shape::Rect(r) => {
                    assert_eq!(r.fill, control);
                    r.rect
                }
                s => panic!("{s:?}"),
            };
            let Shape::Rect(line) = &shapes[1] else {
                panic!("{:?}", shapes[1])
            };
            assert_eq!(line.fill, module);
            assert_eq!(line.rect.height(), 1.0 / ppp);
            assert_eq!(line.rect.x_range(), rect.x_range());
            assert!((line.rect.top() - rect.center().y).abs() < 1.0 / ppp);
            for s in &shapes[2..58] {
                assert!(matches!(s, Shape::Rect(r) if r.fill == menu), "{s:?}");
            }
            for (i, band) in shapes[58..].chunks(3).enumerate() {
                let (Shape::Rect(l), Shape::Circle(d), Shape::Text(n)) =
                    (&band[0], &band[1], &band[2])
                else {
                    panic!("{band:?}")
                };
                let selected = i == 3;
                let ink = if selected { Color32::RED } else { menu };
                assert_eq!((l.fill, d.fill), (ink, ink), "band {i}");
                assert_eq!(l.rect.width(), 1.0 / ppp);
                assert_eq!(l.rect.y_range(), rect.y_range());
                let f = [0.2, 0.4, 0.6, 0.8][if selected { 0 } else { i + 1 }];
                assert!(
                    close(d.center.x, rect.left() + f * rect.width()),
                    "band {i}"
                );
                assert!(close(d.center.y, rect.center().y));
                assert_eq!(d.radius, radius(&metrics(ppp), 0.0));
                assert_eq!(n.fallback_color, background);
                let name = (if selected { 1 } else { i + 2 }).to_string();
                assert_eq!(n.galley.text(), name);
                assert!(close(
                    n.galley.rect.translate(n.pos.to_vec2()).center().x,
                    d.center.x
                ));
            }
        }
    }

    #[test]
    fn the_selected_band_draws_last() {
        for tab in 0..4 {
            let shapes = display_shapes(TYPE_ID, 2, &[], &on_tab(tab), content(), 1.0);
            let names: Vec<String> = shapes
                .iter()
                .filter_map(|s| match s {
                    Shape::Text(t) => Some(t.galley.text().to_owned()),
                    _ => None,
                })
                .collect();
            let mut want: Vec<String> = (1..=4)
                .filter(|&n| n != tab + 1)
                .map(|n| n.to_string())
                .collect();
            want.push((tab + 1).to_string());
            assert_eq!(names, want);
        }
    }

    #[test]
    fn off_step_channel_and_kind_show_their_nearest_item() {
        // Channel 0.2 plays Both but its menu shows Left; Kind 0.3 plays LowShelf but its selector is on Peak, so a
        // tap on Peak changes nothing.
        let mut rig = rig();
        (rig.module.params[17], rig.module.params[4]) = (0.2, 0.3);
        rig.frame(vec![]);
        let texts = rig.texts();
        let at = |name: &str| {
            texts
                .iter()
                .find(|(t, _)| t == name)
                .map(|(_, r)| r.center())
        };
        assert!(at("Left").is_some() && at("Both").is_none());
        rig.click(at("Peak").expect("the Kind selector"));
        assert_eq!(rig.module.params[4], 0.3);
        rig.click(at("HiShelf").unwrap());
        assert_eq!(rig.module.params[4], 1.0);
    }

    // ---- touch ----

    fn rig() -> Rig {
        Rig::new(TYPE_ID, panel().unwrap(), card(10.0).1)
    }

    fn handle(rig: &Rig, k: usize) -> Pos2 {
        centre(rig.rect_of(2), &rig.module.params, k)
    }

    fn wait(rig: &mut Rig, frames: usize) {
        for _ in 0..frames {
            rig.frame(vec![]);
        }
    }

    fn move_to(rig: &mut Rig, pos: Pos2) {
        rig.frame(vec![Event::PointerMoved(pos)]);
    }

    #[test]
    fn a_tap_on_a_handle_selects_its_band() {
        let mut rig = rig();
        let before = rig.module.params.clone();
        let near = handle(&rig, 2) + vec2(1.2 * U, 0.0);
        rig.click(near);
        assert_eq!(rig.state.tab, 2);
        assert_eq!(rig.module.params, before);
        // Half-size (9) + 1 U from band 1's handle is the edge of its reach.
        let reach = radius(&metrics(1.0), 0.0) + U;
        rig.click(handle(&rig, 0) - vec2(reach + 1.0, 0.0));
        assert_eq!(rig.state.tab, 2);
        rig.click(handle(&rig, 0) - vec2(reach - 1.0, 0.0));
        assert_eq!(rig.state.tab, 0);
    }

    #[test]
    fn a_tap_picks_the_nearest_handle() {
        let mut rig = rig();
        let (a, b) = (handle(&rig, 0), handle(&rig, 1));
        rig.click(a + (b - a) * 0.6);
        assert_eq!(rig.state.tab, 1);
        rig.click(a + (b - a) * 0.4);
        assert_eq!(rig.state.tab, 0);
    }

    #[test]
    fn a_drag_after_0_1_s_sets_frequency_and_gain_clamped_to_the_display() {
        let mut rig = rig();
        let rect = rig.rect_of(2);
        let h = handle(&rig, 2);
        move_to(&mut rig, h);
        rig.press(h, PointerButton::Primary, true);
        wait(&mut rig, 7);
        move_to(
            &mut rig,
            pos2(
                rect.left() + 0.25 * rect.width(),
                rect.bottom() - 0.75 * rect.height(),
            ),
        );
        assert!(
            close(rig.module.params[10], 0.25),
            "{}",
            rig.module.params[10]
        );
        assert!(
            close(rig.module.params[9], 0.75),
            "{}",
            rig.module.params[9]
        );
        assert!(rig.changed.contains(&9) && rig.changed.contains(&10));
        assert_eq!(rig.state.tab, 2);
        move_to(&mut rig, rect.right_top() + vec2(50.0, -50.0));
        assert_eq!((rig.module.params[9], rig.module.params[10]), (1.0, 1.0));
        // Further past the clamped corner: nothing changes, so nothing is returned .
        move_to(&mut rig, rect.right_top() + vec2(80.0, -80.0));
        assert!(rig.changed.is_empty(), "{:?}", rig.changed);
        move_to(&mut rig, rect.left_bottom() + vec2(-50.0, 50.0));
        assert_eq!((rig.module.params[9], rig.module.params[10]), (0.0, 0.0));
        rig.press(rect.left_bottom(), PointerButton::Primary, false);
        // The other bands did not move.
        assert_eq!(&rig.module.params[1..9], &defaults()[1..9]);
    }

    #[test]
    fn a_drag_within_0_1_s_moves_nothing() {
        let mut rig = rig();
        let before = rig.module.params.clone();
        let h = handle(&rig, 2);
        move_to(&mut rig, h);
        rig.press(h, PointerButton::Primary, true);
        move_to(&mut rig, h + vec2(30.0, -20.0));
        move_to(&mut rig, h + vec2(60.0, -30.0));
        rig.press(h + vec2(60.0, -30.0), PointerButton::Primary, false);
        assert_eq!(rig.module.params, before);
        assert_eq!(rig.state.tab, 2);
    }

    #[test]
    fn a_drag_from_no_handle_moves_nothing() {
        let mut rig = rig();
        let before = rig.module.params.clone();
        let far = rig.rect_of(2).left_top() + vec2(1.0, 1.0);
        move_to(&mut rig, far);
        rig.press(far, PointerButton::Primary, true);
        wait(&mut rig, 7);
        move_to(&mut rig, far + vec2(100.0, 30.0));
        assert_eq!(rig.module.params, before);
        assert_eq!(rig.state.tab, 0);
    }

    #[test]
    fn a_double_tap_on_a_handle_resets_its_gain_and_frequency() {
        let mut rig = rig();
        (rig.module.params[9], rig.module.params[10]) = (0.9, 0.3);
        wait(&mut rig, 1);
        let h = handle(&rig, 2);
        rig.click(h);
        assert_eq!((rig.module.params[9], rig.module.params[10]), (0.9, 0.3));
        rig.click(h);
        assert_eq!((rig.module.params[9], rig.module.params[10]), (0.5, 0.6));
        assert_eq!(rig.state.tab, 2);
    }

    #[test]
    fn taps_0_3_s_apart_are_not_a_double_tap() {
        let mut rig = rig();
        (rig.module.params[9], rig.module.params[10]) = (0.9, 0.3);
        wait(&mut rig, 1);
        let h = handle(&rig, 2);
        rig.click(h);
        wait(&mut rig, 18);
        rig.click(h);
        assert_eq!((rig.module.params[9], rig.module.params[10]), (0.9, 0.3));
    }

    #[test]
    fn a_held_handle_grows_and_the_selected_one_mixes_in_text() {
        let mut rig = rig();
        let h = handle(&rig, 2);
        move_to(&mut rig, h);
        rig.press(h, PointerButton::Primary, true);
        let colors = theme_by_id(0).colors;
        let held = Color32::RED.lerp_to_gamma(colors.text.to_color32(), 0.3);
        let disc = rig
            .shapes
            .iter()
            .find_map(|c| match &c.shape {
                Shape::Circle(d) if close(d.center.x, h.x) && close(d.center.y, h.y) => Some(*d),
                _ => None,
            })
            .expect("band 3's disc");
        assert_eq!(disc.radius, radius(&metrics(1.0), 1.0));
        assert_eq!(disc.fill, held);
    }

    /// Band `k`'s disc in the last frame.
    fn disc(rig: &Rig, k: usize) -> eframe::egui::epaint::CircleShape {
        let h = handle(rig, k);
        (rig.shapes.iter())
            .find_map(|c| match &c.shape {
                Shape::Circle(d) if close(d.center.x, h.x) && close(d.center.y, h.y) => Some(*d),
                _ => None,
            })
            .expect("the band's disc")
    }

    #[test]
    fn a_hovering_pointer_lights_a_disc_by_proximity_with_up_to_30_percent_text() {
        let mut rig = rig();
        let before = rig.module.params.clone();
        let rest = radius(&metrics(1.0), 0.0);
        // Band 2 is unselected: lerp(menuFill (67, 76, 81), text (199, 206, 218), 0.3·p). Band 1 is selected:
        // lerp(channel (255, 0, 0), text, 0.3·p).
        let cases = [
            (0.0, [67, 76, 81], [255, 0, 0]),
            (0.5, [87, 96, 102], [247, 31, 33]),
            (1.0, [107, 115, 122], [238, 62, 65]),
        ];
        for (p, unselected, selected) in cases {
            for (k, [r, g, b]) in [(1, unselected), (0, selected)] {
                // p = (0.75 U − distance)/0.75 U.
                let at = handle(&rig, k) + vec2(0.0, 0.75 * U * (1.0 - p));
                move_to(&mut rig, at);
                let d = disc(&rig, k);
                assert_eq!(d.fill, Color32::from_rgb(r, g, b), "band {k} p {p}");
                assert!(
                    close(d.radius, rest + 0.25 * U * p),
                    "band {k} p {p}: {}",
                    d.radius
                );
            }
        }
        // Hovering selects and changes nothing.
        assert_eq!(rig.state.tab, 0);
        assert_eq!(rig.module.params, before);
        // The last pointer position still counts once the pointer is gone.
        rig.frame(vec![Event::PointerGone]);
        assert_eq!(disc(&rig, 0).fill, Color32::from_rgb(238, 62, 65));
        // Away from every handle: p = 0.
        let away = rig.rect_of(2).left_top();
        move_to(&mut rig, away);
        assert_eq!(disc(&rig, 0).fill, Color32::RED);
    }

    #[test]
    fn a_press_that_dragged_is_not_half_a_double_tap() {
        let mut rig = rig();
        let h = handle(&rig, 2);
        move_to(&mut rig, h);
        rig.press(h, PointerButton::Primary, true);
        wait(&mut rig, 7);
        let to = h + vec2(10.0, -5.0);
        move_to(&mut rig, to);
        rig.press(to, PointerButton::Primary, false);
        let dragged = (rig.module.params[9], rig.module.params[10]);
        assert_ne!(dragged, (0.5, 0.6));
        // Re-grabbed within 0.3 s of the first press.
        let h = handle(&rig, 2);
        rig.click(h);
        assert_eq!((rig.module.params[9], rig.module.params[10]), dragged);
    }

    #[test]
    fn a_touch_held_still_past_a_long_press_still_drags() {
        use eframe::egui::{TouchDeviceId, TouchId, TouchPhase};
        let touch = |phase, pos| Event::Touch {
            device_id: TouchDeviceId(1),
            id: TouchId(1),
            phase,
            pos,
            force: None,
        };
        let button = |pos, pressed| Event::PointerButton {
            pos,
            button: PointerButton::Primary,
            pressed,
            modifiers: Default::default(),
        };
        let mut rig = rig();
        let (rect, h) = (rig.rect_of(2), handle(&rig, 2));
        rig.frame(vec![
            touch(TouchPhase::Start, h),
            Event::PointerMoved(h),
            button(h, true),
        ]);
        // A second held still: egui's long touch (0.8 s) comes and goes.
        wait(&mut rig, 60);
        let to = pos2(
            rect.left() + 0.25 * rect.width(),
            rect.bottom() - 0.75 * rect.height(),
        );
        rig.frame(vec![touch(TouchPhase::Move, to), Event::PointerMoved(to)]);
        assert!(
            close(rig.module.params[10], 0.25),
            "{}",
            rig.module.params[10]
        );
        assert!(
            close(rig.module.params[9], 0.75),
            "{}",
            rig.module.params[9]
        );
        rig.frame(vec![touch(TouchPhase::End, to), button(to, false)]);
    }

    #[test]
    fn a_tap_pressed_and_released_in_one_frame_selects_too() {
        let mut rig = rig();
        let h = handle(&rig, 3);
        let button = |pressed| Event::PointerButton {
            pos: h,
            button: PointerButton::Primary,
            pressed,
            modifiers: Default::default(),
        };
        rig.frame(vec![Event::PointerMoved(h), button(true), button(false)]);
        assert_eq!(rig.state.tab, 3);
        // The pointer is up: the display holds no drag.
        wait(&mut rig, 1);
        assert_eq!(rig.ctx.dragged_id(), None);
    }

    /// The first `BAND 1` tab's top, in a rack of three Equalizers at ppp 1, after `events` frame by frame.
    struct RackRig {
        ctx: eframe::egui::Context,
        rack: crate::ui::rack::RackPanel,
        channel: crate::song::Channel,
        time: f64,
        top: f32,
    }

    impl RackRig {
        const SCREEN: Rect = Rect {
            min: pos2(0.0, 0.0),
            max: pos2(1280.0, 800.0),
        };

        fn new() -> Self {
            use crate::song::{Channel, ChannelKind};
            let mut channel = Channel::new("Lead".into(), ChannelKind::Instrument, 0.3);
            for _ in 0..3 {
                channel.rack.add(TYPE_ID);
            }
            let mut rig = Self {
                ctx: Default::default(),
                rack: Default::default(),
                channel,
                time: 0.0,
                top: 0.0,
            };
            rig.frame(vec![]);
            rig
        }

        fn frame(&mut self, events: Vec<Event>) {
            self.time += 1.0 / 60.0;
            let raw = eframe::egui::RawInput {
                screen_rect: Some(Self::SCREEN),
                time: Some(self.time),
                events,
                ..Default::default()
            };
            let (theme, m) = (theme_by_id(0), metrics(1.0));
            let cx = UiCtx {
                theme: &theme,
                m: &m,
            };
            let (rack, channel) = (&mut self.rack, &mut self.channel);
            let out = self.ctx.run(raw, |ctx| {
                eframe::egui::CentralPanel::default().show(ctx, |ui| {
                    let rect = crate::ui::rack::rack_rect(Self::SCREEN, 1.0, false, &m);
                    rack.show(ui, &cx, rect, 3, channel);
                });
            });
            self.top = out
                .shapes
                .iter()
                .filter_map(|c| match &c.shape {
                    Shape::Text(t) if t.galley.text() == "BAND 1" => Some(t.pos.y),
                    _ => None,
                })
                .fold(f32::INFINITY, f32::min);
        }

        /// The first card's display rect, unscrolled.
        fn display(&self) -> Rect {
            use crate::ui::rack::{card_height, content_rect, flow, rack_rect};
            let modules = &self.channel.rack.modules;
            let heights: Vec<f32> = modules.iter().map(card_height).collect();
            let rack = rack_rect(Self::SCREEN, 1.0, false, &metrics(1.0));
            let card = flow(rack, 0.0, U, false, &heights, true).cards[0];
            let content = content_rect(card, 10.0, U);
            let panel = panel().unwrap();
            let state = PanelState::default();
            crate::panels::place(&panel, &modules[0], &state, content, U)
                .into_iter()
                .find(|(l, _)| l.id == 2)
                .map(|(_, r)| r)
                .unwrap()
        }

        /// Presses at `from` and drags 120 points up in four frames, held.
        fn drag_up(&mut self, from: Pos2) {
            self.frame(vec![Event::PointerMoved(from)]);
            let press = Event::PointerButton {
                pos: from,
                button: PointerButton::Primary,
                pressed: true,
                modifiers: Default::default(),
            };
            self.frame(vec![press]);
            for i in 1..=4 {
                self.frame(vec![Event::PointerMoved(from - vec2(0.0, 30.0 * i as f32))]);
            }
        }
    }

    #[test]
    fn a_drag_from_where_no_handle_reaches_scrolls_the_rack() {
        let mut rig = RackRig::new();
        let (top, display) = (rig.top, rig.display());
        rig.drag_up(display.left_top() + vec2(2.0, 2.0));
        assert!(rig.top < top - 60.0, "{} after {top}", rig.top);
    }

    #[test]
    fn a_drag_from_a_handle_does_not_scroll_the_rack() {
        let mut rig = RackRig::new();
        let (top, display) = (rig.top, rig.display());
        let h = centre(display, &rig.channel.rack.modules[0].params, 0);
        rig.drag_up(h);
        assert_eq!(rig.top, top);
    }
}
