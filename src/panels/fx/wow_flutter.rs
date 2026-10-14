//! The Wow & Flutter panel (ID 43), UIHeight 9 U.

use eframe::egui::{Color32, Id, Pos2, Rect, Shape, Stroke, Ui, pos2};
use honeydew_audio::analysis;

use super::live_analysis;
use crate::panels::build::*;
use crate::ui::UiCtx;

/// `Sync` (p7) swaps the three rate knobs for spinners.
const SYNC: Show = Show::Item(7, 1);
const FREE: Show = Show::Item(7, 0);

pub fn panel() -> Option<Panel> {
    Some(Panel::new(hstack([
        vstack([
            knob(1, "Wow Rate").p(1).show(FREE),
            spinner(2, "Wow Rate").p(2).show(SYNC),
            knob(3, "Wow Amount").p(3).mt(0.5),
        ]),
        vstack([
            knob(4, "Flutter Rate").p(4).show(FREE),
            spinner(5, "Flutter Rate").p(5).show(SYNC),
            knob(6, "Flutter Amount").p(6).mt(0.5),
        ]),
        vstack([
            toggle(7, "Sync").p(7).w(Size::Flex(0.5)).mb(0.5),
            display(8).h(Size::Flex(1.5)).draw(wobble),
        ])
        .w(Size::Flex(2.0)),
        vstack([
            knob(9, "Random Rate").p(8).show(FREE),
            spinner(10, "Random Rate").p(9).show(SYNC),
            knob(11, "Random Amount").p(10).mt(0.5),
        ]),
    ])))
}

/// The stored delay after a reset, as a fraction of the Y span: a reset fills the buffer with the midpoint.
const RESET: f32 = 0.5;

/// The wobble graph (#8): the trace's y for a stored value `v` (a fraction of the Y span) in the plot rect
/// `p`, y = Pb − clamp(v)·Ph.
fn trace_y(p: Rect, v: f32) -> f32 {
    p.bottom() - v.clamp(0.0, 1.0) * p.height()
}

/// The wobble graph's trace: one point every 2 device pixels across `p`, from its left edge to its right, each the
/// average of the trace `points` (oldest first, the engine's averages of the 120 000-entry ring) that fall in its
/// column. A NaN is the midpoint.
fn trace(p: Rect, ppp: f32, points: &[f32]) -> Vec<Pos2> {
    let n = (((p.width() * ppp).round() as usize) / 2 + 1).max(2);
    let len = points.len();
    (0..n)
        .map(|j| {
            let column = &points[j * len / n..((j + 1) * len / n).max(j * len / n + 1).min(len)];
            let mean = column.iter().sum::<f32>() / column.len() as f32;
            let v = if mean.is_nan() { RESET } else { mean };
            pos2(
                p.left() + p.width() * j as f32 / (n - 1) as f32,
                trace_y(p, v),
            )
        })
        .collect()
}

/// The wobble graph (#8): a graph frame with no grid, labels or buttons, so P is the whole control, filled
/// `controlFill`, and the trace, a 1 px G line across P: the Wow & Flutter's feed, or at rest (no feed) the reset trace,
/// flat at the midpoint. The overflow fill never shows, as the values are clamped into P.
fn wobble(
    ui: &mut Ui,
    cx: &UiCtx,
    _: Id,
    p: Rect,
    module: &mut ModuleInstance,
    _: &mut PanelState,
    ink: Color32,
) -> Vec<usize> {
    let points = live_analysis(ui, module.instance_id).and_then(|s| analysis::wow_flutter(&s));
    let painter = ui.painter();
    painter.rect_filled(p, 0.0, cx.theme.colors.control_fill.to_color32());
    let stroke = Stroke::new(1.0 / cx.m.ppp(), ink);
    match points {
        Some(points) => {
            painter.add(Shape::line(trace(p, cx.m.ppp(), &points), stroke));
        }
        None => {
            let y = trace_y(p, RESET);
            painter.line_segment([pos2(p.left(), y), pos2(p.right(), y)], stroke);
        }
    }
    Vec::new()
}

/// The rows every case shows: the amount knobs, `Sync` and the wobble trace.
#[cfg(test)]
const FIXED: [Row; 5] = [
    row(3, Widget::Knob, "Wow Amount", [0.0, 3.74, 2.8, 3.25])
        .p(3)
        .margins(0.5, 0.0, 0.0, 0.0),
    row(6, Widget::Knob, "Flutter Amount", [2.8, 3.74, 2.8, 3.25])
        .p(6)
        .margins(0.5, 0.0, 0.0, 0.0),
    row(7, Widget::Toggle, "Sync", [7.0, 0.0, 2.8, 2.59])
        .p(7)
        .w(Size::Flex(0.5))
        .margins(0.0, 0.0, 0.5, 0.0),
    row(8, Widget::Display, "", [5.6, 3.09, 5.6, 3.9]).h(Size::Flex(1.5)),
    row(11, Widget::Knob, "Random Amount", [11.2, 3.74, 2.8, 3.25])
        .p(10)
        .margins(0.5, 0.0, 0.0, 0.0),
];

#[cfg(test)]
pub const REFERENCE: Reference = Reference {
    size: [14.0, 6.99],
    settings: &[],
    cases: &[
        Case {
            name: "Sync off",
            rows: &[
                row(1, Widget::Knob, "Wow Rate", [0.0, 0.0, 2.8, 3.24]).p(1),
                FIXED[0],
                row(4, Widget::Knob, "Flutter Rate", [2.8, 0.0, 2.8, 3.24]).p(4),
                FIXED[1],
                FIXED[2],
                FIXED[3],
                row(9, Widget::Knob, "Random Rate", [11.2, 0.0, 2.8, 3.24]).p(8),
                FIXED[4],
            ],
            ..Case::DEFAULT
        },
        Case {
            name: "Sync on",
            params: &[(7, 1.0)],
            // "by spinners in the same slots"
            rows: &[
                row(2, Widget::Spinner, "Wow Rate", [0.0, 0.0, 2.8, 3.24]).p(2),
                FIXED[0],
                row(5, Widget::Spinner, "Flutter Rate", [2.8, 0.0, 2.8, 3.24]).p(5),
                FIXED[1],
                FIXED[2],
                FIXED[3],
                row(10, Widget::Spinner, "Random Rate", [11.2, 0.0, 2.8, 3.24]).p(9),
                FIXED[4],
            ],
            ..Case::DEFAULT
        },
    ],
};

#[cfg(test)]
mod tests {
    use eframe::egui::{Shape, Stroke, pos2, vec2};

    use super::*;
    use crate::panels::tests::{U, display_shapes};
    use crate::theme::theme_by_id;

    #[test]
    fn the_trace_maps_the_y_span_from_the_bottom_up() {
        // y = Pb − clamp(value)·Ph, P = the whole control, here the reference #8 (5.6, 3.09, 5.6, 3.9) U.
        let p = Rect::from_min_size(pos2(5.6 * U, 3.09 * U), vec2(5.6 * U, 3.9 * U));
        for (v, y) in [
            (0.0, p.bottom()),
            (0.5, p.bottom() - 1.95 * U),
            (1.0, p.top()),
            (-1.0, p.bottom()),
            (2.0, p.top()),
        ] {
            assert!((trace_y(p, v) - y).abs() < 1e-3, "{v}");
        }
    }

    #[test]
    fn at_rest_the_graph_is_its_fill_and_a_flat_centre_line() {
        // "a reset fills the buffer with the midpoint, which shows as a flat centre line": 1 px, G (the channel).
        let content = Rect::from_min_size(pos2(0.0, 0.0), vec2(14.0 * U, 6.99 * U));
        let fill = theme_by_id(0).colors.control_fill.to_color32();
        for ppp in [1.0, 2.0] {
            let shapes = display_shapes(43, 8, &[], &PanelState::default(), content, ppp);
            let [Shape::Rect(r), Shape::LineSegment { points, stroke }] = &shapes[..] else {
                panic!("ppp {ppp}: {shapes:?}");
            };
            let p = r.rect;
            let reference = Rect::from_min_size(pos2(5.6 * U, 3.09 * U), vec2(5.6 * U, 3.9 * U));
            assert!(
                (p.min - reference.min).length() < 1.0 && (p.max - reference.max).length() < 1.0
            );
            assert_eq!(r.fill, fill);
            let y = p.bottom() - 0.5 * p.height();
            assert_eq!(
                *points,
                [pos2(p.left(), y), pos2(p.right(), y)],
                "ppp {ppp}"
            );
            assert_eq!(*stroke, Stroke::new(1.0 / ppp, Color32::RED), "ppp {ppp}");
        }
    }

    #[test]
    fn a_feed_draws_the_trace_one_point_every_2_px_averaging_its_columns() {
        use eframe::egui::Shape;
        use honeydew_audio::MeterSnapshot;
        use honeydew_audio::analysis::TRACE_POINTS;
        // The oldest half of the ring at 0.25, the newest at 0.75 (the slab's next point is 0: oldest first).
        let mut slab = vec![0.0; 1 + TRACE_POINTS];
        for (i, v) in slab[1..].iter_mut().enumerate() {
            *v = if i < TRACE_POINTS / 2 { 0.25 } else { 0.75 };
        }
        let content = Rect::from_min_size(pos2(0.0, 0.0), vec2(14.0 * U, 6.99 * U));
        for ppp in [1.0, 2.0] {
            let shapes = crate::panels::tests::display_shapes_fed(
                43,
                8,
                &[],
                &PanelState::default(),
                content,
                ppp,
                (MeterSnapshot::default(), &slab),
            );
            let [Shape::Rect(r), Shape::Path(line)] = &shapes[..] else {
                panic!("ppp {ppp}: {shapes:?}");
            };
            let p = r.rect;
            let n = ((p.width() * ppp).round() as usize) / 2 + 1;
            assert_eq!(line.points.len(), n, "ppp {ppp}");
            assert_eq!(
                line.points[0],
                pos2(p.left(), p.bottom() - 0.25 * p.height())
            );
            let last = line.points[n - 1];
            assert!(
                (last.x - p.right()).abs() < 1e-3
                    && (last.y - (p.bottom() - 0.75 * p.height())).abs() < 1e-3
            );
            // Newest at the right: the step up sits in the middle.
            let mid = line.points.iter().position(|q| q.y < p.center().y).unwrap();
            assert!((mid as f32 / n as f32 - 0.5).abs() < 0.02, "{mid} of {n}");
            assert_eq!(line.stroke.width, 1.0 / ppp);
        }
    }
}
