//! The Spacer panel (ID 34), UIHeight 7 U.

use eframe::egui::{Color32, Id, Painter, Rect, Ui};
use honeydew_audio::analysis;

use super::leveller::animate;
use super::live_analysis;
use crate::metrics::Metrics;
use crate::panels::build::*;
use crate::ui::UiCtx;

/// The correlation at rest and without a feed: none, 0.
const REST: f32 = 0.0;
/// A peak marker's hold before it returns, s, and its return speed, per s.
const HOLD: f32 = 0.5;
const RETURN: f32 = 0.05;

/// The correlation bar: from the centre to c ∈ [−1, 1] (+1 on the right edge), extended by 0.075 U, half on
/// each end, so c = 0 leaves a 0.075 U stub on the centre. Full height.
fn bar(rect: Rect, c: f32, m: &Metrics) -> Rect {
    let (mid, half) = (rect.center().x, m.u(0.075) / 2.0);
    let to = mid + c * rect.width() / 2.0;
    Rect::from_x_y_ranges(mid.min(to) - half..=mid.max(to) + half, rect.y_range())
}

/// The peak markers: the most positive and most negative recent correlation, each held for [`HOLD`] s and then
/// returning towards 0 at [`RETURN`] per s, never inside the current value. Both start at 0.
#[derive(Clone, Copy, Debug, Default, PartialEq)]
struct Peaks {
    high: f32,
    high_age: f32,
    low: f32,
    low_age: f32,
}

impl Peaks {
    fn step(self, dt: f32, c: f32) -> Peaks {
        let (high, high_age) = if c >= self.high {
            (c, 0.0)
        } else if self.high_age + dt <= HOLD {
            (self.high, self.high_age + dt)
        } else {
            let falling = self.high_age + dt - HOLD.max(self.high_age);
            (
                (self.high - RETURN * falling).max(c.max(0.0)),
                self.high_age + dt,
            )
        };
        let (low, low_age) = if c <= self.low {
            (c, 0.0)
        } else if self.low_age + dt <= HOLD {
            (self.low, self.low_age + dt)
        } else {
            let rising = self.low_age + dt - HOLD.max(self.low_age);
            (
                (self.low + RETURN * rising).min(c.min(0.0)),
                self.low_age + dt,
            )
        };
        Peaks {
            high,
            high_age,
            low,
            low_age,
        }
    }
}

/// The marker 0.075 U wide centred on correlation `c`, full height.
fn marker(rect: Rect, c: f32, m: &Metrics) -> Rect {
    let x = rect.center().x + c * rect.width() / 2.0;
    let half = m.u(0.075) / 2.0;
    Rect::from_x_y_ranges(x - half..=x + half, rect.y_range())
}

/// A correlation meter: a `controlFill` trough, the bar at correlation `c` in the channel colour `ink`, then the
/// `text` peak markers. A marker at 0 is not drawn (at rest it would hide the channel stub).
fn meter(painter: &Painter, cx: &UiCtx, rect: Rect, ink: Color32, c: f32, peaks: Peaks) {
    painter.rect_filled(rect, 0.0, cx.theme.colors.control_fill.to_color32());
    painter.rect_filled(bar(rect, c, cx.m), 0.0, ink);
    for peak in [peaks.low, peaks.high].into_iter().filter(|&p| p != 0.0) {
        let text = cx.theme.colors.text.to_color32();
        painter.rect_filled(marker(rect, peak, cx.m), 0.0, text);
    }
}

/// Meter `k` (0 input, 1 output) from the Spacer's correlation feed, with its peak markers kept under `id`.
fn phase(ui: &Ui, cx: &UiCtx, id: Id, rect: Rect, module: &ModuleInstance, ink: Color32, k: usize) {
    let feed = live_analysis(ui, module.instance_id).and_then(|s| analysis::spacer(&s));
    // A NaN is no correlation.
    let c = feed.map_or(REST, |c| c[k]);
    let c = if c.is_nan() { REST } else { c.clamp(-1.0, 1.0) };
    let peaks = animate(ui, id, Peaks::default(), |p, dt| p.step(dt, c));
    if peaks.high > c.max(0.0) || peaks.low < c.min(0.0) {
        ui.ctx().request_repaint();
    }
    meter(ui.painter(), cx, rect, ink, c, peaks);
}

/// #8, the input phase.
fn input_phase(
    ui: &mut Ui,
    cx: &UiCtx,
    id: Id,
    rect: Rect,
    module: &mut ModuleInstance,
    _: &mut PanelState,
    ink: Color32,
) -> Vec<usize> {
    phase(ui, cx, id, rect, module, ink, 0);
    Vec::new()
}

/// #9, the output phase.
fn output_phase(
    ui: &mut Ui,
    cx: &UiCtx,
    id: Id,
    rect: Rect,
    module: &mut ModuleInstance,
    _: &mut PanelState,
    ink: Color32,
) -> Vec<usize> {
    phase(ui, cx, id, rect, module, ink, 1);
    Vec::new()
}

pub fn panel() -> Option<Panel> {
    // `-` left, the caption centred, `+` right.
    let caption = |n: u32, name: &'static str| {
        hstack([
            text(n, "-").align(Align2::LEFT_CENTER),
            text(n + 1, name).align(Align2::CENTER_CENTER),
            text(n + 2, "+").align(Align2::RIGHT_CENTER),
        ])
        .margins(0.0, 0.5, 0.0, 0.5)
    };
    Some(Panel::new(vstack([
        hstack([
            spinner(1, "L/R Inversion").p(4),
            knob(2, "L/R Delay").p(1),
            knob(3, "Separation").p(3),
            knob(4, "Pan").p(2),
            spinner(5, "M/S Inversion").p(6),
            knob(6, "M/S Delay").p(5),
            spinner(7, "Mode").p(7),
        ]),
        // The input and output correlation meters.
        hstack([
            display(8).margins(0.0, 0.5, 0.0, 0.5).draw(input_phase),
            display(9).margins(0.0, 0.5, 0.0, 0.5).draw(output_phase),
        ])
        .h(Size::U(0.15))
        .mb(0.1),
        hstack([caption(10, "input phase"), caption(13, "output phase")]).h(Size::U(0.5)),
    ])))
}

#[cfg(test)]
pub const REFERENCE: Reference = Reference {
    size: [14.0, 4.99],
    settings: &[],
    cases: &[Case {
        rows: &[
            row(1, Widget::Spinner, "L/R Inversion", [0.0, 0.0, 2.0, 4.24]).p(4),
            row(2, Widget::Knob, "L/R Delay", [2.0, 0.0, 2.0, 4.24]).p(1),
            row(3, Widget::Knob, "Separation", [4.0, 0.0, 2.0, 4.24]).p(3),
            row(4, Widget::Knob, "Pan", [6.0, 0.0, 2.0, 4.24]).p(2),
            row(5, Widget::Spinner, "M/S Inversion", [8.0, 0.0, 2.0, 4.24]).p(6),
            row(6, Widget::Knob, "M/S Delay", [10.0, 0.0, 2.0, 4.24]).p(5),
            row(7, Widget::Spinner, "Mode", [12.0, 0.0, 2.0, 4.24]).p(7),
            row(8, Widget::Display, "", [0.5, 4.24, 6.0, 0.15]).margins(0.0, 0.5, 0.0, 0.5),
            row(9, Widget::Display, "", [7.5, 4.24, 6.0, 0.15]).margins(0.0, 0.5, 0.0, 0.5),
            row(10, Widget::Text, "-", [0.5, 4.49, 2.0, 0.5]),
            row(11, Widget::Text, "input phase", [2.5, 4.49, 2.0, 0.5])
                .align(Align2::CENTER_CENTER),
            row(12, Widget::Text, "+", [4.5, 4.49, 2.0, 0.5]).align(Align2::RIGHT_CENTER),
            row(13, Widget::Text, "-", [7.5, 4.49, 2.0, 0.5]),
            row(14, Widget::Text, "output phase", [9.5, 4.49, 2.0, 0.5])
                .align(Align2::CENTER_CENTER),
            row(15, Widget::Text, "+", [11.5, 4.49, 2.0, 0.5]).align(Align2::RIGHT_CENTER),
        ],
        ..Case::DEFAULT
    }],
};

#[cfg(test)]
mod tests {
    use eframe::egui::{Shape, pos2, vec2};

    use super::*;
    use crate::metrics::DEFAULT_UI_SCALE_INDEX;
    use crate::panels::tests::{U, display_shapes};
    use crate::theme::theme_by_id;

    fn close(a: f32, b: f32) -> bool {
        (a - b).abs() < 1e-3
    }

    #[test]
    fn at_zero_the_bar_is_a_0_075_u_stub_on_the_centre() {
        // #8 at the reference: 0.5, 4.24, 6, 0.15 U, so its centre is at 3.5 U.
        let eight = Rect::from_min_size(pos2(0.5 * U, 4.24 * U), vec2(6.0 * U, 0.15 * U));
        for ppp in [1.0, 2.0] {
            let m = Metrics::from_pixels_per_point(ppp, DEFAULT_UI_SCALE_INDEX);
            let stub = bar(eight, 0.0, &m);
            assert!(
                close(stub.left(), 3.4625 * U) && close(stub.right(), 3.5375 * U),
                "{stub:?}"
            );
            assert_eq!(stub.y_range(), eight.y_range());
            // +1 reaches the right edge, −1 the left, each with its half of the 0.075 U.
            let full = bar(eight, 1.0, &m);
            assert!(
                close(full.left(), 3.4625 * U) && close(full.right(), 6.5375 * U),
                "{full:?}"
            );
            let full = bar(eight, -1.0, &m);
            assert!(
                close(full.left(), 0.4625 * U) && close(full.right(), 3.5375 * U),
                "{full:?}"
            );
        }
    }

    #[test]
    fn peak_markers_hold_half_a_second_then_return_towards_0() {
        let rest = Peaks::default();
        // Out at once to a new extreme.
        let p = rest.step(0.1, 0.8).step(0.1, -0.4);
        assert_eq!((p.high, p.low), (0.8, -0.4));
        // Held for 0.5 s after it was reached (the high's 0.1 s already passed), then 0.05 per second back.
        let p = p.step(0.4, 0.0);
        assert_eq!(p.high, 0.8);
        let p = p.step(1.0, 0.0);
        assert!(close(p.high, 0.8 - 0.05), "{p:?}");
        assert!(close(p.low, -0.4 + 0.05 * 0.9), "{p:?}");
        // Never inside the current value: a held 0.5 correlation keeps the high marker there.
        let p = Peaks::default().step(0.1, 0.9).step(20.0, 0.5);
        assert_eq!(p.high, 0.5);
        // Back at 0 they stay.
        let p = Peaks::default().step(0.1, 0.1).step(10.0, 0.0);
        assert_eq!((p.high, p.low), (0.0, 0.0));
    }

    #[test]
    fn a_feed_moves_each_bar_to_its_correlation() {
        // Input +1 (mono), output −0.5: #8's bar reaches the right edge, #9's goes halfway left; the markers stand at
        // the values (0 is not drawn).
        use honeydew_audio::MeterSnapshot;
        let content = Rect::from_min_size(pos2(0.0, 0.0), vec2(14.0 * U, 4.99 * U));
        let text = theme_by_id(0).colors.text.to_color32();
        let feed = [1.0, -0.5];
        for ppp in [1.0, 2.0] {
            let m = Metrics::from_pixels_per_point(ppp, DEFAULT_UI_SCALE_INDEX);
            for (n, c) in [(8, 1.0), (9, -0.5)] {
                let shapes = crate::panels::tests::display_shapes_fed(
                    34,
                    n,
                    &[],
                    &PanelState::default(),
                    content,
                    ppp,
                    (MeterSnapshot::default(), &feed[..]),
                );
                let [Shape::Rect(trough), Shape::Rect(b), Shape::Rect(peak)] = &shapes[..] else {
                    panic!("ppp {ppp} #{n}: {shapes:?}")
                };
                let want = bar(trough.rect, c, &m);
                assert!(
                    close(b.rect.left(), want.left()) && close(b.rect.right(), want.right()),
                    "ppp {ppp} #{n}: {b:?}"
                );
                assert_eq!(peak.fill, text);
                assert!(close(
                    peak.rect.center().x,
                    trough.rect.center().x + c * trough.rect.width() / 2.0
                ));
            }
        }
    }

    #[test]
    fn at_rest_both_meters_are_a_trough_and_the_stub() {
        let control = theme_by_id(0).colors.control_fill.to_color32();
        let content = Rect::from_min_size(pos2(0.0, 0.0), vec2(14.0 * U, 4.99 * U));
        for ppp in [1.0, 2.0] {
            for n in [8, 9] {
                let shapes = display_shapes(34, n, &[], &PanelState::default(), content, ppp);
                let [Shape::Rect(trough), Shape::Rect(stub)] = &shapes[..] else {
                    panic!("ppp {ppp} #{n}: {shapes:?}")
                };
                assert_eq!(
                    (trough.fill, stub.fill),
                    (control, Color32::RED),
                    "ppp {ppp} #{n}"
                );
                let centre = trough.rect.center().x;
                assert!(
                    close(stub.rect.width(), 0.075 * U) && close(stub.rect.center().x, centre),
                    "{stub:?}"
                );
                assert_eq!(stub.rect.y_range(), trough.rect.y_range());
            }
        }
    }
}
