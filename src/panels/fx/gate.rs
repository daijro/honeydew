//! Gate (ID 27), UIHeight 8 U. Pages `LEVEL` (0) and `ENVELOPE` (1).

use eframe::egui::{Color32, Id, Rect, Ui, pos2, vec2};

use honeydew_audio::analysis;

use super::leveller::animate;
use super::live_analysis;
use crate::metrics::Metrics;
use crate::panels::build::*;
use crate::ui::{UiCtx, widgets};

/// The marker lines' thickness, U.
const LINE_U: f32 = 0.05;
/// How fast a peak marker falls back to the level, dB per second.
const PEAK_FALL_DB: f32 = 20.0;

/// One level meter's geometry in its display rect (#4 Left and #5 Right).
#[derive(Debug)]
struct Meter {
    /// The caption's box, the top 0.75 U.
    caption: Rect,
    /// The trough: x + 0.4 w, 0.2 w wide; from y + 1.25 U, h − 1.75 U tall.
    bar: Rect,
    /// The opening threshold line across the bar, centred on y(Threshold).
    open: Rect,
    /// The closing threshold line at y(Threshold − Asymmetry), while that is above −60 dB.
    close: Option<Rect>,
    /// The `%.0f dB` Threshold readout and its box: 0.5 U tall, centred on the opening line, in the gap from this bar's
    /// right edge to the right meter's bar. Only the Left meter draws it.
    text: (String, Rect),
}

/// The meter in `r` at Threshold p1 and Asymmetry p2. Its lines sit on the 66 dB law y(dB) = bottom −
/// barH·(dB + 60)/66, with Threshold = p1·66 − 60 dB and Asymmetry 66·p2² dB (quadratic, 0–66 dB).
fn meter(r: Rect, m: &Metrics, p1: f32, p2: f32) -> Meter {
    let bar = Rect::from_min_size(
        pos2(r.left() + 0.4 * r.width(), r.top() + m.u(1.25)),
        vec2(0.2 * r.width(), r.height() - m.u(1.75)),
    );
    let y = |db: f32| bar.bottom() - bar.height() * (db + 60.0) / 66.0;
    let line = |db: f32| {
        let half = m.u(LINE_U / 2.0);
        Rect::from_x_y_ranges(bar.x_range(), y(db) - half..=y(db) + half)
    };
    let threshold = p1 * 66.0 - 60.0;
    let closing = threshold - 66.0 * p2 * p2;
    // The right meter is this one's twin in the equal rect beside it: its bar starts 0.4 w past this rect.
    let gap = Rect::from_x_y_ranges(
        bar.right()..=r.right() + 0.4 * r.width(),
        y(threshold) - m.u(0.25)..=y(threshold) + m.u(0.25),
    );
    Meter {
        caption: r.with_max_y(r.top() + m.u(0.75)),
        bar,
        open: line(threshold),
        close: (closing > -60.0).then(|| line(closing)),
        text: (format!("{threshold:.0} dB"), gap),
    }
}

/// #4: the Left meter, and the Threshold readout in the gap to the Right one.
fn left(
    ui: &mut Ui,
    cx: &UiCtx,
    id: Id,
    r: Rect,
    module: &mut ModuleInstance,
    _: &mut PanelState,
    ink: Color32,
) -> Vec<usize> {
    level_meter(ui, cx, id, r, module, ink, true);
    Vec::new()
}

/// #5: the Right meter.
fn right(
    ui: &mut Ui,
    cx: &UiCtx,
    id: Id,
    r: Rect,
    module: &mut ModuleInstance,
    _: &mut PanelState,
    ink: Color32,
) -> Vec<usize> {
    level_meter(ui, cx, id, r, module, ink, false);
    Vec::new()
}

/// A channel's level from the Gate's detector (its RMS over the last 256 samples), dB, clamped to −60…+6 as the fill
/// takes it; silence (−60) without a feed. A NaN is silence.
fn level(ui: &Ui, module: &ModuleInstance, left: bool) -> f32 {
    let levels = live_analysis(ui, module.instance_id).and_then(|s| analysis::gate(&s));
    let db = levels.map_or(-60.0, |l| l[usize::from(!left)]);
    if db.is_nan() {
        -60.0
    } else {
        db.clamp(-60.0, 6.0)
    }
}

/// The peak marker after `dt` s: up to the level at once, else falling towards it at [`PEAK_FALL_DB`].
fn peak_step(peak: f32, dt: f32, level: f32) -> f32 {
    (peak - PEAK_FALL_DB * dt).max(level)
}

/// The level meters, steps 1–5: the caption in the channel colour, FontSize 1; the `controlFill` trough; the
/// channel-coloured RMS fill, (L/60 + 1)·barH up from the bottom (0 dB already fills the bar, +6 dB overshoots it by
/// 10 %); the `moduleFill` closing line, then the `text` opening line; the channel peak marker, while above −60 dB; the
/// readout (Left only) in the channel colour.
fn level_meter(
    ui: &Ui,
    cx: &UiCtx,
    id: Id,
    r: Rect,
    module: &ModuleInstance,
    ink: Color32,
    left: bool,
) {
    let colors = &cx.theme.colors;
    // A param the module lacks reads as its default, 0.
    let p = |i: usize| module.params.get(i).copied().unwrap_or_default();
    let meter = meter(r, cx.m, p(1), p(2));
    let level = level(ui, module, left);
    let peak = animate(ui, id, -60.0, |peak, dt| peak_step(peak, dt, level));
    if peak > level {
        ui.ctx().request_repaint();
    }
    let caption = if left { "Left" } else { "Right" };
    let center = Align2::CENTER_CENTER;
    widgets::text_label(ui, cx, meter.caption, caption, FontSize::Pt8, center, ink);
    let painter = ui.painter();
    painter.rect_filled(meter.bar, 0.0, colors.control_fill.to_color32());
    let fill = (level / 60.0 + 1.0) * meter.bar.height();
    if fill > 0.0 {
        let bottom = meter.bar.bottom();
        let rect = Rect::from_x_y_ranges(meter.bar.x_range(), bottom - fill..=bottom);
        painter.rect_filled(rect, 0.0, ink);
    }
    if let Some(close) = meter.close {
        painter.rect_filled(close, 0.0, colors.module_fill.to_color32());
    }
    painter.rect_filled(meter.open, 0.0, colors.text.to_color32());
    if peak > -60.0 {
        let y = meter.bar.bottom() - meter.bar.height() * (peak + 60.0) / 66.0;
        let half = cx.m.u(LINE_U / 2.0);
        let line = Rect::from_x_y_ranges(meter.bar.x_range(), y - half..=y + half);
        painter.rect_filled(line, 0.0, ink);
    }
    if left {
        let (text, at) = &meter.text;
        widgets::text_label(ui, cx, *at, text, FontSize::Pt8, center, ink);
    }
}

pub fn panel() -> Option<Panel> {
    Some(Panel::new(vstack([
        tabs(1, "LEVEL · ENVELOPE").h(Size::U(1.25)),
        hstack([
            knob(2, "Asymmetry").p(2).w(Size::Flex(2.0)),
            v_slider(3, "Threshold").p(1),
            // The Left and Right level meters.
            display(4).draw(left),
            display(5).draw(right),
            knob(6, "Mix").p(7).w(Size::Flex(2.0)),
        ])
        .show(Show::Tab(0)),
        hstack([
            graph_knob(7, "A. Curve", false).p(4).w(Size::Flex(2.0)),
            v_slider(8, "Attack").p(3).value_text(),
            v_slider(9, "Hold").p(8).value_text(),
            v_slider(10, "Release").p(5).value_text(),
            graph_knob(11, "R. Curve", true).p(6).w(Size::Flex(2.0)),
        ])
        .show(Show::Tab(1)),
    ])))
}

#[cfg(test)]
const TABS: Row = row(1, Widget::Tabs, "LEVEL · ENVELOPE", [0.0, 0.0, 14.0, 1.25]).h(Size::U(1.25));

#[cfg(test)]
pub const REFERENCE: Reference = Reference {
    size: [14.0, 5.99],
    settings: &[],
    cases: &[
        Case {
            name: "LEVEL",
            rows: &[
                TABS,
                row(2, Widget::Knob, "Asymmetry", [0.0, 1.25, 4.0, 4.74])
                    .p(2)
                    .w(Size::Flex(2.0)),
                row(3, Widget::VSlider, "Threshold", [4.0, 1.25, 2.0, 4.74]).p(1),
                row(4, Widget::Display, "", [6.0, 1.25, 2.0, 4.74]),
                row(5, Widget::Display, "", [8.0, 1.25, 2.0, 4.74]),
                row(6, Widget::Knob, "Mix", [10.0, 1.25, 4.0, 4.74])
                    .p(7)
                    .w(Size::Flex(2.0)),
            ],
            ..Case::DEFAULT
        },
        Case {
            name: "ENVELOPE",
            tab: 1,
            rows: &[
                TABS,
                row(
                    7,
                    Widget::GraphKnob { falling: false },
                    "A. Curve",
                    [0.0, 1.25, 4.0, 4.74],
                )
                .p(4)
                .w(Size::Flex(2.0)),
                row(8, Widget::VSlider, "Attack", [4.0, 1.25, 2.0, 4.74])
                    .p(3)
                    .value_text(),
                row(9, Widget::VSlider, "Hold", [6.0, 1.25, 2.0, 4.74])
                    .p(8)
                    .value_text(),
                row(10, Widget::VSlider, "Release", [8.0, 1.25, 2.0, 4.74])
                    .p(5)
                    .value_text(),
                row(
                    11,
                    Widget::GraphKnob { falling: true },
                    "R. Curve",
                    [10.0, 1.25, 4.0, 4.74],
                )
                .p(6)
                .w(Size::Flex(2.0)),
            ],
            ..Case::DEFAULT
        },
    ],
};

#[cfg(test)]
mod tests {
    use eframe::egui::{Shape, pos2};

    use super::*;
    use crate::panels::fx::limiter::tests::{metrics, near, u};
    use crate::panels::tests::{U, display_shapes};
    use crate::panels::{panel_for, place};
    use crate::theme::theme_by_id;

    /// The reference content rect: 14 × 5.99 U at the origin.
    const CONTENT: Rect = Rect {
        min: pos2(0.0, 0.0),
        max: pos2(14.0 * U, 5.99 * U),
    };

    /// The reference rects of #4 and #5.
    const LEFT: [f32; 4] = [6.0, 1.25, 2.0, 4.74];
    const RIGHT: [f32; 4] = [8.0, 1.25, 2.0, 4.74];

    fn at([x, y, w, h]: [f32; 4]) -> Rect {
        u(x, y, w, h)
    }

    #[test]
    fn the_bars_and_captions_sit_where_the_spec_puts_them() {
        // Bars at x 6.8 and 8.8, 0.2 w = 0.4 U wide, y 2.5–5.49; captions in the top 0.75 U of each rect.
        for ppp in [1.0, 2.0] {
            let m = metrics(ppp);
            for (r, x) in [(LEFT, 6.8), (RIGHT, 8.8)] {
                let meter = meter(at(r), &m, 0.0, 0.0);
                assert!(
                    near(meter.bar, u(x, 2.5, 0.4, 2.99)),
                    "ppp {ppp}: {:?}",
                    meter.bar
                );
                assert!(
                    near(meter.caption, u(r[0], 1.25, 2.0, 0.75)),
                    "ppp {ppp}: {:?}",
                    meter.caption
                );
            }
        }
    }

    #[test]
    fn the_opening_line_follows_the_threshold_on_the_66_db_scale() {
        // Threshold = p1·66 − 60 dB at y = bottom − barH·(dB + 60)/66: p1 = 0 is −60 dB on the bottom (5.49 U), 0.5 is
        // −27 dB at 5.49 − 2.99/2, 1 is +6 dB on the top (2.5 U). The line is 0.05 U thick, across the bar.
        for ppp in [1.0, 2.0] {
            let m = metrics(ppp);
            for (p1, y) in [(0.0, 5.49), (0.5, 5.49 - 2.99 / 2.0), (1.0, 2.5)] {
                let open = meter(at(LEFT), &m, p1, 0.0).open;
                assert!(
                    near(open, u(6.8, y - 0.025, 0.4, 0.05)),
                    "ppp {ppp}, p1 {p1}: {open:?}"
                );
            }
        }
    }

    #[test]
    fn the_closing_line_shows_only_above_minus_60_db() {
        let m = metrics(1.0);
        let close = |p1: f32, p2: f32| meter(at(RIGHT), &m, p1, p2).close;
        // Threshold − Asymmetry (66·p2² dB, quadratic): the defaults give −60 dB, hidden, and so does +6 − 66.
        assert_eq!(close(0.0, 0.0), None);
        assert_eq!(close(1.0, 1.0), None);
        // p2 = 0.5 is 16.5 dB below the threshold: −27 − 16.5 = −43.5 dB, a quarter of the bar up.
        let line = close(0.5, 0.5).expect("above −60 dB");
        assert!(
            near(line, u(8.8, 5.49 - 2.99 / 4.0 - 0.025, 0.4, 0.05)),
            "{line:?}"
        );
        // Quadratic, not linear: p2 = 0.25 is 4.125 dB below, −31.125 dB, 28.875/66 of the bar up.
        let line = close(0.5, 0.25).expect("above −60 dB");
        assert!(
            near(line, u(8.8, 5.49 - 2.99 * 28.875 / 66.0 - 0.025, 0.4, 0.05)),
            "{line:?}"
        );
    }

    #[test]
    fn the_threshold_text_is_centred_in_the_gap_on_its_line() {
        // `%.0f dB` of the Threshold, in a 0.5 U box centred on the opening line, from the left bar's right edge
        // (7.2 U) to the right bar's left edge (8.8 U).
        for ppp in [1.0, 2.0] {
            let m = metrics(ppp);
            for (p1, text, y) in [
                (0.0, "-60 dB", 5.49),
                (0.5, "-27 dB", 5.49 - 2.99 / 2.0),
                (1.0, "6 dB", 2.5),
            ] {
                let (got, at_) = meter(at(LEFT), &m, p1, 0.0).text;
                assert_eq!(got, text);
                assert!(
                    near(at_, u(7.2, y - 0.25, 1.6, 0.5)),
                    "ppp {ppp}, p1 {p1}: {at_:?}"
                );
            }
        }
    }

    /// What `#leaf` draws at `params` (empty: the defaults), as (kind, colour, text) in drawing order.
    fn drawn(leaf: u32, params: &[f32], ppp: f32) -> Vec<(&'static str, Color32, String)> {
        display_shapes(27, leaf, params, &PanelState::default(), CONTENT, ppp)
            .into_iter()
            .map(|s| match s {
                Shape::Rect(r) => ("rect", r.fill, String::new()),
                Shape::Text(t) => (
                    "text",
                    t.galley.job.sections[0].format.color,
                    t.galley.text().to_owned(),
                ),
                other => panic!("{other:?}"),
            })
            .collect()
    }

    #[test]
    fn each_meter_draws_its_caption_trough_and_lines_and_the_left_one_the_text() {
        let colors = theme_by_id(0).colors;
        let (trough, text, module) = (
            colors.control_fill.to_color32(),
            colors.text.to_color32(),
            colors.module_fill.to_color32(),
        );
        let s = String::new;
        for ppp in [1.0, 2.0] {
            // The defaults: no closing line, and at rest no level fill or peak.
            assert_eq!(
                drawn(4, &[], ppp),
                [
                    ("text", Color32::RED, "Left".to_owned()),
                    ("rect", trough, s()),
                    ("rect", text, s()),
                    ("text", Color32::RED, "-60 dB".to_owned()),
                ],
                "ppp {ppp}"
            );
            assert_eq!(
                drawn(5, &[], ppp),
                [
                    ("text", Color32::RED, "Right".to_owned()),
                    ("rect", trough, s()),
                    ("rect", text, s())
                ],
                "ppp {ppp}"
            );
            // The closing line, under the opening one.
            let mut params = ModuleInstance::new(27, 2).params;
            params[1] = 0.5;
            params[2] = 0.25;
            assert_eq!(
                drawn(5, &params, ppp),
                [
                    ("text", Color32::RED, "Right".to_owned()),
                    ("rect", trough, s()),
                    ("rect", module, s()),
                    ("rect", text, s()),
                ],
                "ppp {ppp}"
            );
        }
    }

    #[test]
    fn a_feed_fills_each_bar_by_its_channels_rms_and_marks_its_peak() {
        // Left at −30 dB, right at −60 (silent): the left fill is half the bar, (−30/60 + 1)·barH; its peak marker
        // sits on the 66 dB law at −30 dB, after the lines. The right meter draws no fill and no marker.
        use honeydew_audio::MeterSnapshot;
        let feed = [-30.0, -60.0];
        for ppp in [1.0, 2.0] {
            let m = metrics(ppp);
            let state = PanelState::default();
            let fed = |leaf| {
                crate::panels::tests::display_shapes_fed(
                    27,
                    leaf,
                    &[],
                    &state,
                    CONTENT,
                    ppp,
                    (MeterSnapshot::default(), &feed[..]),
                )
            };
            let rects = |shapes: Vec<Shape>| -> Vec<(Rect, Color32)> {
                (shapes.into_iter())
                    .filter_map(|s| match s {
                        Shape::Rect(r) => Some((r.rect, r.fill)),
                        _ => None,
                    })
                    .collect()
            };
            let left = rects(fed(4));
            let bar = left[0].0;
            let fill = Rect::from_x_y_ranges(
                bar.x_range(),
                bar.bottom() - bar.height() / 2.0..=bar.bottom(),
            );
            assert_eq!(left[1].1, Color32::RED, "ppp {ppp}: {left:?}");
            assert!(near(left[1].0, fill), "ppp {ppp}: {:?}", left[1].0);
            let y = bar.bottom() - bar.height() * 30.0 / 66.0;
            let half = m.u(LINE_U / 2.0);
            let marker = Rect::from_x_y_ranges(bar.x_range(), y - half..=y + half);
            assert_eq!(left.last().map(|r| r.1), Some(Color32::RED));
            assert!(near(left.last().unwrap().0, marker), "ppp {ppp}: {left:?}");
            assert!(
                rects(fed(5)).iter().all(|r| r.1 != Color32::RED),
                "ppp {ppp}"
            );
        }
        // A level above +6 dB fills 110 % of the bar; a NaN is silence.
        let mut over = [10.0, f32::NAN];
        let shapes = crate::panels::tests::display_shapes_fed(
            27,
            4,
            &[],
            &PanelState::default(),
            CONTENT,
            1.0,
            (MeterSnapshot::default(), &over[..]),
        );
        let fills: Vec<Rect> = (shapes.iter())
            .filter_map(|s| match s {
                Shape::Rect(r) if r.fill == Color32::RED => Some(r.rect),
                _ => None,
            })
            .collect();
        let Shape::Rect(bar) = &shapes[1] else {
            panic!()
        };
        assert!(
            (fills[0].height() - 1.1 * bar.rect.height()).abs() < 1e-3,
            "{fills:?}"
        );
        over.swap(0, 1);
        let right = crate::panels::tests::display_shapes_fed(
            27,
            4,
            &[],
            &PanelState::default(),
            CONTENT,
            1.0,
            (MeterSnapshot::default(), &over[..]),
        );
        assert!(
            right
                .iter()
                .all(|s| !matches!(s, Shape::Rect(r) if r.fill == Color32::RED))
        );
    }

    #[test]
    fn a_peak_jumps_up_at_once_and_falls_back_towards_the_level() {
        assert_eq!(peak_step(-60.0, 0.0, -12.0), -12.0);
        assert_eq!(peak_step(-12.0, 0.5, -40.0), -22.0);
        assert_eq!(peak_step(-22.0, 10.0, -40.0), -40.0);
    }

    #[test]
    fn the_shapes_land_on_the_laid_out_rects() {
        // Through the layout: the bars within a device pixel of 6.8 and 8.8 U, y 2.5–5.49 U.
        for ppp in [1.0, 2.0] {
            for (leaf, x) in [(4, 6.8), (5, 8.8)] {
                let shapes = display_shapes(27, leaf, &[], &PanelState::default(), CONTENT, ppp);
                let Shape::Rect(bar) = &shapes[1] else {
                    panic!("{shapes:?}")
                };
                let want = u(x, 2.5, 0.4, 2.99);
                assert!(
                    (bar.rect.min - want.min).length() <= 1.5 / ppp
                        && (bar.rect.max - want.max).length() <= 1.5 / ppp,
                    "ppp {ppp}, #{leaf}: {:?}",
                    bar.rect
                );
            }
            // The readout's box, which #4 finds from its own rect, ends on #5's drawn bar.
            let m = metrics(ppp);
            let (module, state) = (ModuleInstance::new(27, 2), PanelState::default());
            let panel = panel_for(27).expect("a panel");
            let placed = place(panel, &module, &state, CONTENT * ppp, m.u_px());
            let left = placed
                .iter()
                .find(|(l, _)| l.id == 4)
                .map(|(_, r)| *r / ppp)
                .expect("#4");
            let shapes = display_shapes(27, 5, &[], &state, CONTENT, ppp);
            let Shape::Rect(bar) = &shapes[1] else {
                panic!("{shapes:?}")
            };
            let gap = meter(left, &m, 0.0, 0.0).text.1;
            assert!(
                (gap.right() - bar.rect.left()).abs() < 1e-3,
                "ppp {ppp}: {gap:?}, {:?}",
                bar.rect
            );
        }
    }
}
