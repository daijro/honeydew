//! Limiter (ID 28), UIHeight 6 U.

use eframe::egui::{Color32, Id, Pos2, Rect, Shape, Stroke, Ui, pos2};
use honeydew_audio::analysis::{self, HISTORY, HistoryView};

use super::live_analysis;
use crate::metrics::Metrics;
use crate::panels::build::*;
use crate::ui::UiCtx;

/// The grid's levels: k = 0, 5/22 … 20/22 of the height is −60, −45, −30, −15 and 0 dB. The band
/// graphs draw every other one: −60, −30 and 0 dB.
const GRID_DB: [f32; 5] = [-60.0, -45.0, -30.0, -15.0, 0.0];

/// The top of the 1 px row at `db` on the history's 66 dB scale: (y + h − 1 px) − h·(60 + dB)/66, so −60 dB is the
/// last row and 0 dB is 60/66 of the height up.
fn scale_y(r: Rect, m: &Metrics, db: f32) -> f32 {
    r.bottom() - 1.0 / m.ppp() - r.height() * (60.0 + db) / 66.0
}

/// The history display (#3): no background fill, the grid's full-width 1 px rows in `text` at 10 %, the `series` (a
/// reset history without a feed: none), then the `threshold` row (dB) in `text`. With `band`, the Multiband band
/// graph: a `controlFill` fill first and only 3 grid rows.
pub(super) fn history(
    ui: &Ui,
    cx: &UiCtx,
    r: Rect,
    band: bool,
    threshold: Option<f32>,
    series: Option<(&HistoryView, Color32)>,
) {
    let (painter, m) = (ui.painter(), cx.m);
    let text = cx.theme.colors.text.to_color32();
    let row = |db: f32, color: Color32| {
        let y = scale_y(r, m, db);
        let rect = Rect::from_x_y_ranges(r.x_range(), y..=y + 1.0 / m.ppp());
        painter.rect_filled(rect, 0.0, color);
    };
    if band {
        painter.rect_filled(r, 0.0, cx.theme.colors.control_fill.to_color32());
    }
    for &db in GRID_DB.iter().step_by(if band { 2 } else { 1 }) {
        row(db, text.gamma_multiply(0.1));
    }
    if let Some((series, ink)) = series {
        painter.extend(columns(r, m, series, text, ink));
    }
    if let Some(t) = threshold {
        row(t, text);
    }
}

/// Steps 2–4 for `series` in `r`, column i of cw = w/128 at x + i·cw, newest at the right: the input bars in `text`
/// at 20 % (min(f·h, h) from the bottom), the output bars in the channel colour `ink` at 30 % (f·h, not clamped), and
/// the 1 px gain trace in `ink` through (x + i·cw, y + h − f·h). Empty bars are not drawn; the trace starts at the
/// first entry written (a reset history holds no gain, f = 0).
fn columns(r: Rect, m: &Metrics, series: &HistoryView, text: Color32, ink: Color32) -> Vec<Shape> {
    let cw = r.width() / HISTORY as f32;
    let bar = |i: usize, h: f32, color: Color32| {
        let x = r.left() + i as f32 * cw;
        (h > 0.0).then(|| {
            Shape::rect_filled(
                Rect::from_x_y_ranges(x..=x + cw, r.bottom() - h..=r.bottom()),
                0.0,
                color,
            )
        })
    };
    let inputs = (series.input.iter().enumerate()).filter_map(|(i, &f)| {
        bar(
            i,
            (f * r.height()).min(r.height()),
            text.gamma_multiply(0.2),
        )
    });
    let outputs = (series.output.iter().enumerate())
        .filter_map(|(i, &f)| bar(i, f * r.height(), ink.gamma_multiply(0.3)));
    let mut shapes: Vec<Shape> = inputs.chain(outputs).collect();
    let first = series.gain.iter().position(|&f| f > 0.0).unwrap_or(HISTORY);
    let trace: Vec<Pos2> = (first..HISTORY)
        .map(|i| {
            pos2(
                r.left() + i as f32 * cw,
                r.bottom() - series.gain[i] * r.height(),
            )
        })
        .collect();
    if trace.len() > 1 {
        shapes.push(Shape::line(trace, Stroke::new(1.0 / m.ppp(), ink)));
    }
    shapes
}

/// #3: the history from the Limiter's feed, with the threshold T = p4·24 − 24 dB (`Thr.`).
fn graph(
    ui: &mut Ui,
    cx: &UiCtx,
    _: Id,
    r: Rect,
    module: &mut ModuleInstance,
    _: &mut PanelState,
    ink: Color32,
) -> Vec<usize> {
    let threshold = module.params.get(4).map(|v| v * 24.0 - 24.0);
    let series = live_analysis(ui, module.instance_id).and_then(|s| analysis::limiter(&s));
    history(
        ui,
        cx,
        r,
        false,
        threshold,
        series.as_ref().map(|s| (s, ink)),
    );
    Vec::new()
}

pub fn panel() -> Option<Panel> {
    Some(Panel::new(hstack([
        v_slider(1, "In Gain").p(1).value_text(),
        v_slider(2, "Thr.").p(4).value_text(),
        // The 6.4 s history.
        display(3).w(Size::Flex(5.0)).mb(0.5).draw(graph),
        v_slider(4, "Release").p(3).value_text(),
        v_slider(5, "Out Gain").p(2).value_text(),
    ])))
}

#[cfg(test)]
pub const REFERENCE: Reference = Reference {
    size: [14.0, 3.99],
    settings: &[],
    cases: &[Case {
        rows: &[
            row(1, Widget::VSlider, "In Gain", [0.0, 0.0, 1.55, 3.99])
                .p(1)
                .value_text(),
            row(2, Widget::VSlider, "Thr.", [1.55, 0.0, 1.56, 3.99])
                .p(4)
                .value_text(),
            row(3, Widget::Display, "", [3.11, 0.0, 7.77, 3.49])
                .w(Size::Flex(5.0))
                .margins(0.0, 0.0, 0.5, 0.0),
            row(4, Widget::VSlider, "Release", [10.88, 0.0, 1.56, 3.99])
                .p(3)
                .value_text(),
            row(5, Widget::VSlider, "Out Gain", [12.44, 0.0, 1.56, 3.99])
                .p(2)
                .value_text(),
        ],
        ..Case::DEFAULT
    }],
};

#[cfg(test)]
pub(super) mod tests {
    use eframe::egui::{Shape, pos2, vec2};

    use super::*;
    use crate::metrics::DEFAULT_UI_SCALE;
    use crate::panels::tests::{U, display_shapes};
    use crate::theme::theme_by_id;

    /// U = 32 points at either density.
    pub(in crate::panels::fx) fn metrics(ppp: f32) -> Metrics {
        Metrics::from_pixels_per_point(ppp, DEFAULT_UI_SCALE)
    }

    /// A rect given in U.
    pub(in crate::panels::fx) fn u(x: f32, y: f32, w: f32, h: f32) -> Rect {
        Rect::from_min_size(pos2(x * U, y * U), vec2(w * U, h * U))
    }

    /// The filled rects `shapes` holds, as (rect, fill); panics on anything else.
    pub(in crate::panels::fx) fn rects(shapes: Vec<Shape>) -> Vec<(Rect, Color32)> {
        let rect = |s: Shape| match s {
            Shape::Rect(r) => (r.rect, r.fill),
            other => panic!("{other:?}"),
        };
        shapes.into_iter().map(rect).collect()
    }

    /// The 1 px row at k of the height: (y + h − 1 px) − h·k, as a rect.
    pub(in crate::panels::fx) fn row_at(r: Rect, ppp: f32, k: f32) -> Rect {
        let y = r.bottom() - 1.0 / ppp - r.height() * k;
        Rect::from_x_y_ranges(r.x_range(), y..=y + 1.0 / ppp)
    }

    #[test]
    fn rows_sit_on_the_66_db_scale_one_device_pixel_up_from_the_bottom() {
        // The reference rect (3.11, 0, 7.77, 3.49) U: −60 dB is the last row, 0 dB is 60/66 of the height up.
        let r = Rect::from_min_size(pos2(3.11 * U, 0.0), vec2(7.77 * U, 3.49 * U));
        for ppp in [1.0, 2.0] {
            let m = metrics(ppp);
            let bottom = 3.49 * U - 1.0 / ppp;
            for (db, k) in GRID_DB.into_iter().zip([0.0, 5.0, 10.0, 15.0, 20.0]) {
                let want = bottom - 3.49 * U * k / 22.0;
                assert!(
                    (scale_y(r, &m, db) - want).abs() < 1e-3,
                    "ppp {ppp}, {db} dB"
                );
            }
            // The threshold T = p4·24 − 24: p4 = 0 is −24 dB (36/66 up), p4 = 1 is 0 dB, on the top grid line.
            assert!((scale_y(r, &m, -24.0) - (bottom - 3.49 * U * 36.0 / 66.0)).abs() < 1e-3);
        }
    }

    #[test]
    fn the_history_at_rest_is_its_grid_and_threshold() {
        let text = theme_by_id(0).colors.text.to_color32();
        let content = Rect::from_min_size(pos2(0.0, 0.0), vec2(14.0 * U, 3.99 * U));
        for ppp in [1.0, 2.0] {
            for (p4, k) in [(0.0, 36.0 / 66.0), (1.0, 60.0 / 66.0)] {
                let mut params = ModuleInstance::new(28, 2).params;
                params[4] = p4;
                let got = rects(display_shapes(
                    28,
                    3,
                    &params,
                    &PanelState::default(),
                    content,
                    ppp,
                ));
                // No background fill: five grid rows in `text` at 10 %, then the threshold row in `text`. No series.
                assert_eq!(got.len(), 6, "ppp {ppp}: {got:?}");
                // The display rect, from its −60 dB row: the last one.
                let r = Rect::from_x_y_ranges(got[0].0.x_range(), 0.0..=got[0].0.max.y);
                let ks = [0.0, 5.0, 10.0, 15.0, 20.0].map(|k| k / 22.0);
                for (i, &k) in ks.iter().enumerate() {
                    assert_eq!(got[i].1, text.gamma_multiply(0.1), "ppp {ppp}, row {i}");
                    assert!(
                        near(got[i].0, row_at(r, ppp, k)),
                        "ppp {ppp}, row {i}: {:?}",
                        got[i].0
                    );
                }
                assert_eq!(got[5].1, text, "ppp {ppp}");
                assert!(
                    near(got[5].0, row_at(r, ppp, k)),
                    "ppp {ppp}, p4 {p4}: {:?}",
                    got[5].0
                );
                // Laid out within a pixel of the reference rect.
                assert!(
                    (r.left() - 3.11 * U).abs() <= 1.0 && (r.width() - 7.77 * U).abs() <= 1.0,
                    "{r:?}"
                );
                assert!((r.height() - 3.49 * U).abs() <= 1.0, "{r:?}");
            }
        }
    }

    #[test]
    fn a_feed_draws_the_input_and_output_bars_and_the_gain_trace() {
        // The newest two entries: input at 0 dB (60/66) and +12 dB (clamped to h), output at −24 dB (36/66), gain
        // at −6 dB (54/66). The rest of the history is empty: no bars, and the trace starts at the first entry.
        use honeydew_audio::MeterSnapshot;
        let text = theme_by_id(0).colors.text.to_color32();
        let content = Rect::from_min_size(pos2(0.0, 0.0), vec2(14.0 * U, 3.99 * U));
        let mut slab = vec![0.0; 1 + 3 * HISTORY];
        for (k, f) in [(0, 60.0 / 66.0), (1, 36.0 / 66.0), (2, 54.0 / 66.0)] {
            slab[1 + k * HISTORY + 1] = f;
            slab[1 + k * HISTORY] = f;
        }
        slab[1] = 1.2;
        slab[0] = 2.0;
        for ppp in [1.0, 2.0] {
            let shapes = crate::panels::tests::display_shapes_fed(
                28,
                3,
                &ModuleInstance::new(28, 2).params,
                &PanelState::default(),
                content,
                ppp,
                (MeterSnapshot::default(), &slab),
            );
            // Grid (5), input bars (2), output bars (2), the trace, the threshold.
            assert_eq!(shapes.len(), 11, "ppp {ppp}: {shapes:?}");
            // The display rect, from its −60 dB row: the last one.
            let Shape::Rect(grid) = &shapes[0] else {
                panic!()
            };
            let r = Rect::from_x_y_ranges(grid.rect.x_range(), 0.0..=grid.rect.max.y);
            let h = r.height();
            let cw = r.width() / HISTORY as f32;
            let bars: Vec<(Rect, Color32)> = rects(shapes[5..9].to_vec());
            let col = |i: usize, height: f32| {
                let x = r.left() + i as f32 * cw;
                Rect::from_x_y_ranges(x..=x + cw, r.bottom() - height..=r.bottom())
            };
            let newest = HISTORY - 1;
            assert!(near(bars[0].0, col(newest - 1, h)), "ppp {ppp}: {bars:?}");
            assert!(
                near(bars[1].0, col(newest, h * 60.0 / 66.0)),
                "ppp {ppp}: {bars:?}"
            );
            assert_eq!(bars[0].1, text.gamma_multiply(0.2));
            assert!(
                near(bars[3].0, col(newest, h * 36.0 / 66.0)),
                "ppp {ppp}: {bars:?}"
            );
            assert_eq!(bars[3].1, Color32::RED.gamma_multiply(0.3));
            let Shape::Path(trace) = &shapes[9] else {
                panic!("{:?}", shapes[9])
            };
            assert_eq!(trace.points.len(), 2);
            let y = r.bottom() - h * 54.0 / 66.0;
            assert!(
                (trace.points[1].y - y).abs() < 1e-3
                    && (trace.points[1].x - col(newest, 0.0).left()).abs() < 1e-3
            );
            assert_eq!(trace.stroke.width, 1.0 / ppp);
        }
    }

    pub(in crate::panels::fx) fn near(a: Rect, b: Rect) -> bool {
        (a.min - b.min).length() < 1e-3 && (a.max - b.max).length() < 1e-3
    }
}
