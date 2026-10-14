//! The Waveshaper panel (ID 41), UIHeight 11 U.
//!
//! The transfer curve over the current page's XY pad is module-drawn, by the overlay [`transfer`].

use eframe::egui::{Color32, Pos2, Rect, Shape, Stroke, Ui, pos2};
// The shape functions are the engine's, so the curve is the sound's.
use honeydew_audio::analysis;
use honeydew_audio::waveshaper::{shape, shape_index};

use super::live_analysis;
use crate::panels::build::*;
use crate::ui::UiCtx;

/// `Sync` (p16) on the LFO page.
const SYNC: Show = Show::Item(16, 1);
const FREE: Show = Show::Item(16, 0);

/// The XY pads, one per page.
const PADS: [u32; 4] = [4, 9, 15, 25];
/// The curve's line width (U) and the radius of the dot on its first point (device pixels, as the pad's own dot).
const LINE_U: f32 = 0.07;
const DOT_PX: f32 = 3.0;

/// The transfer curve: the polyline of `f` over `pad` (points) at `ppp`. Pixel column i = 0…W−1 of the pad, at its
/// centre, takes in = 2i/(W − 1) − 1 and y = top + (0.5 − f/2)·h, clamped to the pad. Empty under two columns.
fn transfer_points(pad: Rect, ppp: f32, f: impl Fn(f32) -> f32) -> Vec<Pos2> {
    // The pad lies on whole device pixels; `as` turns a negative or NaN width into 0.
    let w = (pad.width() * ppp).round() as usize;
    if w < 2 {
        return Vec::new();
    }
    (0..w)
        .map(|i| {
            let input = 2.0 * i as f32 / (w - 1) as f32 - 1.0;
            let y = pad.top() + (0.5 - f(input) / 2.0) * pad.height();
            pos2(
                pad.left() + (i as f32 + 0.5) / ppp,
                y.clamp(pad.top(), pad.bottom()),
            )
        })
        .collect()
}

/// The transfer curve: while the module is enabled (p0 ≥ 0.5), over the visible XY pad, the current Shape
/// function as a channel-coloured polyline 0.07 U wide ([`transfer_points`]), then a 3 px dot on its first point.
fn transfer(
    ui: &Ui,
    cx: &UiCtx,
    module: &ModuleInstance,
    _: &PanelState,
    rects: &[(u32, Rect)],
    _: Rect,
    channel: Color32,
) {
    if module.params.first().is_none_or(|&on| on < 0.5) {
        return;
    }
    let v = |i: usize| module.params.get(i).copied().unwrap_or(0.0);
    // X and Y as the shaper uses them: p3 and p4 plus the LFO part (LFO × X Mod·0.5, p19, and LFO × Y Mod·0.5, p20),
    // clamped to 0…1 and smoothed per sample with coefficient 1 − 44.1/sr, from the engine's feed; without one, as
    // with no LFO output, p3 and p4.
    let effective = live_analysis(ui, module.instance_id).and_then(|s| analysis::waveshaper(&s));
    let (x, y) = effective
        .filter(|(x, y)| x.is_finite() && y.is_finite())
        .unwrap_or((v(3), v(4)));
    let index = shape_index(v(1));
    let (painter, ppp) = (ui.painter(), cx.m.ppp());
    for &(_, pad) in rects.iter().filter(|(n, _)| PADS.contains(n)) {
        let points = transfer_points(pad, ppp, |input| shape(index, x, y, input));
        let Some(&first) = points.first() else {
            continue;
        };
        painter.add(Shape::line(points, Stroke::new(cx.m.u(LINE_U), channel)));
        painter.circle_filled(first, DOT_PX / ppp, channel);
    }
}

pub fn panel() -> Option<Panel> {
    // Every page: left column, XY pad, right column, under a 0.5 U gap.
    let page = |k: usize, columns: [Part; 3]| hstack(columns).mt(0.5).show(Show::Tab(k));
    // The pad of every page: X = p3, Y = p4, no axis names. The transfer curve over it is the overlay's, [`transfer`].
    let pad = |id: u32| xy_pad(id, 3, 4).w(Size::U(7.5));
    let caption = |id: u32, label| text(id, label).font(FontSize::Pt10).h(Size::U(1.0));
    Some(
        Panel::new(vstack([
            tabs(1, "SHAPE · AMP · EQ · LFO").h(Size::U(1.0)),
            page(
                0,
                [
                    vstack([
                        spinner(2, "Shape").p(1),
                        knob(3, "Stereo Spread").p(7).mt(0.5),
                    ]),
                    pad(4),
                    vstack([hstack([v_slider(5, "X").p(3), v_slider(6, "Y").p(4)])]),
                ],
            ),
            page(
                1,
                [
                    vstack([
                        knob(7, "Drive").p(5).mb(0.5),
                        toggle(8, "Auto Gain").p(2).value_text(),
                    ]),
                    pad(9),
                    vstack([knob(10, "Post Gain").p(6).mb(0.5), knob(11, "Mix").p(8)]),
                ],
            ),
            page(
                2,
                [
                    vstack([
                        caption(12, "Pre Filter"),
                        knob(13, "High Cut").p(9).mb(0.5),
                        knob(14, "Low Cut").p(10),
                    ]),
                    pad(15),
                    vstack([
                        caption(16, "Post EQ"),
                        hstack([v_slider(17, "Bass").p(11), v_slider(18, "Treble").p(14)]),
                        hstack([
                            v_slider(19, "Mid Freq").p(12),
                            v_slider(20, "Mid Gain").p(13),
                        ]),
                    ]),
                ],
            ),
            page(
                3,
                [
                    vstack([
                        hstack([spinner(21, "LFO Shape").p(15)]),
                        hstack([
                            spinner(22, "Rate").p(17).show(SYNC),
                            v_slider(23, "Rate").p(18).show(FREE),
                            toggle(24, "Sync").p(16).value_text(),
                        ]),
                    ]),
                    pad(25),
                    vstack([knob(26, "X Mod").p(19).mb(0.5), knob(27, "Y Mod").p(20)]),
                ],
            ),
        ]))
        .overlay(transfer),
    )
}

#[cfg(test)]
const TABS: Row = row(
    1,
    Widget::Tabs,
    "SHAPE · AMP · EQ · LFO",
    [0.0, 0.0, 14.0, 1.0],
)
.h(Size::U(1.0));
/// The XY pad of page `#n`, the same on every page.
#[cfg(test)]
const fn pad(n: u32) -> Row {
    row(n, Widget::XyPad, "", [3.25, 1.5, 7.5, 7.49])
        .xy(3, 4)
        .w(Size::U(7.5))
}
#[cfg(test)]
const LFO: [Row; 4] = [
    TABS,
    row(21, Widget::Spinner, "LFO Shape", [0.0, 1.5, 3.25, 3.74]).p(15),
    // `Rate` (#22 or #23) goes here.
    row(24, Widget::Toggle, "Sync", [1.62, 5.24, 1.63, 3.75])
        .p(16)
        .value_text(),
    pad(25),
];
#[cfg(test)]
const MODS: [Row; 2] = [
    row(26, Widget::Knob, "X Mod", [10.75, 1.5, 3.25, 3.49])
        .p(19)
        .margins(0.0, 0.0, 0.5, 0.0),
    row(27, Widget::Knob, "Y Mod", [10.75, 5.49, 3.25, 3.5]).p(20),
];

#[cfg(test)]
pub const REFERENCE: Reference = Reference {
    size: [14.0, 8.99],
    settings: &[],
    cases: &[
        Case {
            name: "SHAPE",
            rows: &[
                TABS,
                row(2, Widget::Spinner, "Shape", [0.0, 1.5, 3.25, 3.49]).p(1),
                row(3, Widget::Knob, "Stereo Spread", [0.0, 5.49, 3.25, 3.5])
                    .p(7)
                    .margins(0.5, 0.0, 0.0, 0.0),
                pad(4),
                row(5, Widget::VSlider, "X", [10.75, 1.5, 1.62, 7.49]).p(3),
                row(6, Widget::VSlider, "Y", [12.37, 1.5, 1.63, 7.49]).p(4),
            ],
            ..Case::DEFAULT
        },
        Case {
            name: "AMP",
            tab: 1,
            rows: &[
                TABS,
                row(7, Widget::Knob, "Drive", [0.0, 1.5, 3.25, 3.49])
                    .p(5)
                    .margins(0.0, 0.0, 0.5, 0.0),
                row(8, Widget::Toggle, "Auto Gain", [0.0, 5.49, 3.25, 3.5])
                    .p(2)
                    .value_text(),
                pad(9),
                row(10, Widget::Knob, "Post Gain", [10.75, 1.5, 3.25, 3.49])
                    .p(6)
                    .margins(0.0, 0.0, 0.5, 0.0),
                row(11, Widget::Knob, "Mix", [10.75, 5.49, 3.25, 3.5]).p(8),
            ],
            ..Case::DEFAULT
        },
        Case {
            name: "EQ",
            tab: 2,
            rows: &[
                TABS,
                row(12, Widget::Text, "Pre Filter", [0.0, 1.5, 3.25, 1.0])
                    .font(FontSize::Pt10)
                    .h(Size::U(1.0)),
                row(13, Widget::Knob, "High Cut", [0.0, 2.5, 3.25, 2.99])
                    .p(9)
                    .margins(0.0, 0.0, 0.5, 0.0),
                row(14, Widget::Knob, "Low Cut", [0.0, 5.99, 3.25, 3.0]).p(10),
                pad(15),
                row(16, Widget::Text, "Post EQ", [10.75, 1.5, 3.25, 1.0])
                    .font(FontSize::Pt10)
                    .h(Size::U(1.0)),
                row(17, Widget::VSlider, "Bass", [10.75, 2.5, 1.62, 3.24]).p(11),
                row(18, Widget::VSlider, "Treble", [12.37, 2.5, 1.63, 3.24]).p(14),
                row(19, Widget::VSlider, "Mid Freq", [10.75, 5.74, 1.62, 3.25]).p(12),
                row(20, Widget::VSlider, "Mid Gain", [12.37, 5.74, 1.63, 3.25]).p(13),
            ],
            ..Case::DEFAULT
        },
        Case {
            name: "LFO, Sync off",
            tab: 3,
            rows: &[
                LFO[0],
                LFO[1],
                row(23, Widget::VSlider, "Rate", [0.0, 5.24, 1.62, 3.75]).p(18),
                LFO[2],
                LFO[3],
                MODS[0],
                MODS[1],
            ],
            ..Case::DEFAULT
        },
        Case {
            name: "LFO, Sync on",
            tab: 3,
            params: &[(16, 1.0)],
            rows: &[
                LFO[0],
                LFO[1],
                // The spinner in the slider's slot.
                row(22, Widget::Spinner, "Rate", [0.0, 5.24, 1.62, 3.75]).p(17),
                LFO[2],
                LFO[3],
                MODS[0],
                MODS[1],
            ],
            ..Case::DEFAULT
        },
    ],
};

#[cfg(test)]
mod tests {
    use eframe::egui::{self, Context, RawInput, Shape, pos2, vec2};

    use super::*;
    use crate::metrics::{DEFAULT_UI_SCALE_INDEX, Metrics};
    use crate::panels::tests::{Rig, U, card};
    use crate::params::menu_value;
    use crate::theme::theme_by_id;

    const SMOOTH: usize = 0;
    const ANALOG: usize = 1;
    const MOSHPIT: usize = 2;
    const FOLDBACK: usize = 3;
    const CHAINSAW: usize = 4;
    const RECTIFUZZ: usize = 5;
    const CYCLONE: usize = 6;

    /// A pad on whole device pixels at ppp 1 and 2: 240 × 239 points.
    const PAD: Rect = Rect {
        min: pos2(104.0, 48.0),
        max: pos2(344.0, 287.0),
    };

    fn near(a: f32, b: f32, tol: f32) -> bool {
        (a - b).abs() < tol
    }

    /// `shape(index, x, y, ·)` at each `(input, expected)`.
    fn check(index: usize, x: f32, y: f32, cases: &[(f32, f32)]) {
        for &(input, want) in cases {
            let got = shape(index, x, y, input);
            assert!(
                near(got, want, 1e-4),
                "shape {index} at x {x}, y {y}, in {input}: {got}, want {want}"
            );
        }
    }

    #[test]
    fn the_shape_index_is_round_p1_times_6() {
        for i in 0..7 {
            assert_eq!(shape_index(menu_value(i, 7)), i);
        }
        assert_eq!(shape_index(0.58), 3);
        assert_eq!(shape_index(0.59), 4);
        assert_eq!(shape_index(0.0), SMOOTH);
        assert_eq!(shape_index(1.0), CYCLONE);
    }

    #[test]
    fn smooth_is_a_normalised_soft_clip() {
        // x = 0.5: a = 0 → 1e-4, k = 5.5; the norm |g(1 + 1e4)| ≈ 1, so f ≈ 5.5 in / √((5.5 in − 1e-4)² + 1).
        let default = [
            (-1.0, -0.983853),
            (-0.5, -0.939763),
            (0.0, 0.0),
            (0.5, 0.939824),
            (1.0, 0.983887),
        ];
        check(SMOOTH, 0.5, 0.5, &default);
        // x = 0.75, y = 0: a = 0.5, k = 1, f = g(in)/|g(3)|, g(u) = u/√((u − 0.5)² + 1).
        check(
            SMOOTH,
            0.75,
            0.0,
            &[(1.0, 0.802773), (-1.0, -0.497859), (0.0, 0.0)],
        );
        // x = 0.25, y = 0: a = −0.5, the mirror image: the norm |g(−3)|, sgn a = −1.
        check(SMOOTH, 0.25, 0.0, &[(1.0, 0.497859), (-1.0, -0.802773)]);
    }

    #[test]
    fn analog_is_a_shifted_tanh() {
        // x = 0.5: a = 0, f = tanh(5.5 in).
        let default = [
            (-1.0, -0.999967),
            (-0.5, -0.991860),
            (0.0, 0.0),
            (0.5, 0.991860),
            (1.0, 0.999967),
        ];
        check(ANALOG, 0.5, 0.5, &default);
        // x = 0.75, y = 0: a = 0.5, k = 1, f = (tanh 0.5 + tanh(in − 0.5))/(1 + tanh 0.5).
        check(
            ANALOG,
            0.75,
            0.0,
            &[(1.0, 0.632121), (0.5, 0.316060), (-1.0, -0.303007)],
        );
        // x = 0.25: a = −0.5, f = (tanh(−0.5) + tanh(in + 0.5))/(1 + tanh |−0.5|).
        check(ANALOG, 0.25, 0.0, &[(1.0, 0.303007)]);
    }

    #[test]
    fn foldback_is_a_sine_fold() {
        // x = y = 0.5: k = 4.5, f = sin(4.5 in).
        let default = [
            (-1.0, 0.977530),
            (-0.5, -0.778073),
            (0.0, 0.0),
            (0.5, 0.778073),
            (1.0, -0.977530),
        ];
        check(FOLDBACK, 0.5, 0.5, &default);
        // x = 0 shifts by 2: sin 2 at in = 0; y = 1 makes k = 8: sin(8·0.25 + 2) = sin 4.
        check(FOLDBACK, 0.0, 0.0, &[(0.0, 0.909297)]);
        check(FOLDBACK, 0.0, 1.0, &[(0.25, -0.756802)]);
    }

    #[test]
    fn chainsaw_wraps_q_every_2p() {
        // x = y = 0.5: a = 0.55, p = 0.4, q = frac((in + 1)/0.8): rising 2q/a − 1 below a, falling above.
        check(
            CHAINSAW,
            0.5,
            0.5,
            &[
                (-1.0, -1.0),
                // q = 0.5 on the rise, and again one and two periods (0.8) on.
                (-0.6, 0.818182),
                (0.2, 0.818182),
                (1.0, 0.818182),
                // q = 0.875 on the fall: (2 − 1.75)/0.45 − 1.
                (-0.3, -0.444444),
                // Past the wrap: q = 0.05.
                (-0.16, -0.818182),
            ],
        );
        // x = 1, y = 0: a = 1, p = 0.1, never past the rise: a saw 2q − 1 of period 0.2.
        check(
            CHAINSAW,
            1.0,
            0.0,
            &[(-0.95, -0.5), (-0.5, 0.0), (0.03, -0.7)],
        );
    }

    #[test]
    fn cyclone_adds_a_damped_sine() {
        // x = y = 0.5: a = 0.8, k = −0.03 (y is not above 0.5).
        check(CYCLONE, 0.5, 0.5, &[(0.0, 0.0), (0.5, 0.479902)]);
        // x = y = 1: a = 1, k = 4: f = in + exp(−e·in²)·sin(8π in), sin = 1 at in = 1/16.
        check(CYCLONE, 1.0, 1.0, &[(1.0 / 16.0, 1.051938)]);
        // x = y = 0: a = 0.6, k = −4.
        check(CYCLONE, 0.0, 0.0, &[(0.1, -0.346432)]);
    }

    #[test]
    fn moshpit_runs_through_its_9_points() {
        // x = y = 0.5: d = 0.5, e = 0, m = 0.
        let default = [
            (-1.0, -1.0),
            (-0.75, -0.75),
            (-0.19, -0.1),
            (-0.1, -0.02),
            (0.0, 0.0),
            (0.1, 0.02),
            (0.19, 0.1),
            (0.75, 0.75),
            (1.0, 1.0),
        ];
        check(MOSHPIT, 0.5, 0.5, &default);
        // x = 0.25, y = 0.2: d = 0.25, e = 0.6, m = 0.6.
        let points = [
            (-1.0, -0.778),
            (-0.75, -0.6),
            (-0.22, -0.1),
            (-0.055, -0.548),
            (0.0, 0.0),
            (0.055, 0.548),
            (0.22, 0.1),
            (0.75, 0.6),
            (1.0, 0.778),
        ];
        check(MOSHPIT, 0.25, 0.2, &points);
        // Half-way along a segment, t = 0.5, s by the curve c of its end point: linear (c = 0), t^(1 − c) for c > 0,
        // 1 − (1 − t)^(1 + c) for c < 0.
        let halves = [
            (-0.485, -0.6 + 0.5 * 0.5),
            (-0.1375, -0.1 + 0.5f32.powf(0.5) * -0.448),
            (-0.0275, -0.548 + (1.0 - 0.5f32.powf(0.7)) * 0.548),
            (0.875, 0.6 + 0.5f32.powf(1.0 - 0.096) * 0.178),
        ];
        check(MOSHPIT, 0.25, 0.2, &halves);
    }

    #[test]
    fn rectifuzz_is_a_full_wave_rectifier_at_the_centre() {
        // x = y = 0.5: points (−1, 1), (−0.999, 1), (0, 0), (0.999, 1), (1, 1), all straight: |in|/0.999 up to 1.
        for input in [-1.0, -0.75, -0.5, -0.25, 0.0, 0.3, 0.7, 1.0] {
            let got = shape(RECTIFUZZ, 0.5, 0.5, input);
            assert!(near(got, f32::abs(input), 1e-3), "in {input}: {got}");
        }
        // x = 0.75: Yf = 1, X0 = −0.4995; flat 1 down to X0, then straight to (0, 0).
        check(
            RECTIFUZZ,
            0.75,
            0.5,
            &[(-0.75, 1.0), (-0.25, 1.0 - 0.2495 / 0.4995)],
        );
        // x = 0.25, y = 1: Yf = 0, the negative half silent; c = −0.8 into (0.999, 1): s = 1 − (1 − t)^0.2.
        check(
            RECTIFUZZ,
            0.25,
            1.0,
            &[(-0.5, 0.0), (0.4995, 1.0 - 0.5f32.powf(0.2))],
        );
        // x = 1: X0 = 0, a zero-width segment (0, 1) → (0, 0) that no input falls in.
        check(
            RECTIFUZZ,
            1.0,
            0.5,
            &[(-0.5, 1.0), (0.0, 0.0), (0.999, 1.0)],
        );
        // x = 0: Yf = −1, and X0 = −1.998 clamped up to −0.999: straight from (−0.999, −1) to (0, 0).
        check(RECTIFUZZ, 0.0, 0.5, &[(-0.5, -0.5005)]);
    }

    #[test]
    fn the_curve_has_one_point_per_pixel_column_input_left_to_right() {
        for ppp in [1.0, 2.0] {
            let points = transfer_points(PAD, ppp, |input| input);
            // W = 240 points of pad = 240·ppp columns; column i at its centre, in = 2i/(W − 1) − 1.
            let w = (240.0 * ppp) as usize;
            assert_eq!(points.len(), w, "ppp {ppp}");
            for (i, p) in points.iter().enumerate() {
                let input = 2.0 * i as f32 / (w - 1) as f32 - 1.0;
                assert!(
                    near(p.x, PAD.left() + (i as f32 + 0.5) / ppp, 1e-3),
                    "ppp {ppp}, #{i}: {p:?}"
                );
                // y = top + (0.5 − f/2)·h.
                let y = PAD.top() + (0.5 - input / 2.0) * PAD.height();
                assert!(near(p.y, y, 1e-3), "ppp {ppp}, #{i}: {p:?}");
            }
            // +1 at the top, −1 at the bottom.
            assert!(
                near(points[0].y, PAD.bottom(), 1e-3) && near(points[w - 1].y, PAD.top(), 1e-3)
            );
        }
    }

    #[test]
    fn the_curve_is_clamped_to_the_pad() {
        for ppp in [1.0, 2.0] {
            let points = transfer_points(PAD, ppp, |input| 3.0 * input);
            assert!(
                points
                    .iter()
                    .all(|p| (PAD.top()..=PAD.bottom()).contains(&p.y)),
                "ppp {ppp}"
            );
            assert_eq!(
                (points[0].y, points[points.len() - 1].y),
                (PAD.bottom(), PAD.top())
            );
        }
        // Too narrow for two columns: nothing.
        let thin = Rect::from_min_size(PAD.min, vec2(1.0, 10.0));
        assert!(transfer_points(thin, 1.0, |input| input).is_empty());
    }

    /// The shapes the overlay draws for the visible controls `rects` at `ppp`, U = 32 points.
    fn drawn(module: &ModuleInstance, rects: &[(u32, Rect)], ppp: f32) -> Vec<Shape> {
        let (theme, m) = (
            theme_by_id(0),
            Metrics::from_pixels_per_point(ppp, DEFAULT_UI_SCALE_INDEX),
        );
        let cx = UiCtx {
            theme: &theme,
            m: &m,
        };
        let out = Context::default().run(RawInput::default(), |ctx| {
            let ui = egui::Ui::new(ctx.clone(), egui::Id::new("overlay"), Default::default());
            transfer(
                &ui,
                &cx,
                module,
                &PanelState::default(),
                rects,
                PAD,
                Color32::RED,
            );
        });
        out.shapes.into_iter().map(|c| c.shape).collect()
    }

    #[test]
    fn the_curve_is_a_channel_line_with_a_dot_on_its_first_point() {
        let mut module = ModuleInstance::new(41, 2);
        // Foldback, X 0.2, Y 0.9.
        module.params[1] = menu_value(FOLDBACK, 7);
        (module.params[3], module.params[4]) = (0.2, 0.9);
        for ppp in [1.0, 2.0] {
            let shapes = drawn(&module, &[(4, PAD)], ppp);
            let [Shape::Path(line), Shape::Circle(dot)] = &shapes[..] else {
                panic!("ppp {ppp}: {shapes:?}");
            };
            // 0.07 U wide, the channel colour.
            assert!(
                !line.closed && near(line.stroke.width, 0.07 * U, 1e-4),
                "ppp {ppp}"
            );
            assert_eq!(
                line.stroke.color,
                egui::epaint::ColorMode::Solid(Color32::RED)
            );
            let want = transfer_points(PAD, ppp, |input| shape(FOLDBACK, 0.2, 0.9, input));
            assert_eq!(line.points, want, "ppp {ppp}");
            // A 3 px dot (radius, as the pad's own) on the first point.
            assert_eq!(
                (dot.center, dot.radius, dot.fill),
                (want[0], 3.0 / ppp, Color32::RED),
                "ppp {ppp}"
            );
        }
    }

    #[test]
    fn the_curve_shows_only_while_the_module_is_enabled() {
        let mut module = ModuleInstance::new(41, 2);
        for (p0, shown) in [(1.0, true), (0.5, true), (0.49, false), (0.0, false)] {
            module.params[0] = p0;
            assert_eq!(
                !drawn(&module, &[(4, PAD)], 1.0).is_empty(),
                shown,
                "p0 {p0}"
            );
        }
    }

    #[test]
    fn the_curve_follows_the_shape_menu_and_x_y() {
        for index in 0..7 {
            let mut module = ModuleInstance::new(41, 2);
            module.params[1] = menu_value(index, 7);
            (module.params[3], module.params[4]) = (0.3, 0.7);
            let Some(Shape::Path(line)) = drawn(&module, &[(9, PAD)], 2.0).into_iter().next()
            else {
                panic!("shape {index}: no curve");
            };
            assert_eq!(
                line.points,
                transfer_points(PAD, 2.0, |input| shape(index, 0.3, 0.7, input)),
                "shape {index}"
            );
        }
    }

    #[test]
    fn a_feed_draws_the_curve_at_the_engines_x_and_y() {
        // The LFO has moved X and Y to 0.2 and 0.9 while p3 and p4 stay 0.3 and 0.7: the curve is the shaper's.
        let mut module = ModuleInstance::new(41, 2);
        (module.params[3], module.params[4]) = (0.3, 0.7);
        let index = shape_index(module.params[1]);
        let (theme, m) = (
            theme_by_id(0),
            Metrics::from_pixels_per_point(2.0, DEFAULT_UI_SCALE_INDEX),
        );
        let cx = UiCtx {
            theme: &theme,
            m: &m,
        };
        let out = Context::default().run(RawInput::default(), |ctx| {
            let ui = egui::Ui::new(ctx.clone(), egui::Id::new("overlay"), Default::default());
            let feed = (honeydew_audio::MeterSnapshot::default(), &[0.2, 0.9][..]);
            crate::panels::post_feed(&ui, 2, Some(feed));
            transfer(
                &ui,
                &cx,
                &module,
                &PanelState::default(),
                &[(9, PAD)],
                PAD,
                Color32::RED,
            );
        });
        let Some(Shape::Path(line)) = out.shapes.into_iter().map(|c| c.shape).next() else {
            panic!("no curve");
        };
        assert_eq!(
            line.points,
            transfer_points(PAD, 2.0, |input| shape(index, 0.2, 0.9, input))
        );
    }

    #[test]
    fn only_the_pads_carry_a_curve() {
        let module = ModuleInstance::new(41, 2);
        let other = Rect::from_min_size(pos2(400.0, 48.0), vec2(52.0, 240.0));
        let shapes = drawn(
            &module,
            &[(2, other), (5, other), (6, other), (25, PAD)],
            1.0,
        );
        assert_eq!(shapes.len(), 2);
        assert!(
            shapes
                .iter()
                .all(|s| PAD.expand(3.0).contains_rect(s.visual_bounding_rect()))
        );
    }

    #[test]
    fn every_page_draws_the_curve_over_its_own_pad() {
        let (_, content) = card(11.0);
        let mut rig = Rig::new(41, panel().unwrap(), content);
        for (tab, pad) in PADS.into_iter().enumerate() {
            rig.state.tab = tab;
            rig.frame(vec![]);
            let r = rig.rect_of(pad);
            let curves: Vec<_> = (rig.shapes.iter().enumerate())
                .filter_map(|(i, c)| match &c.shape {
                    Shape::Path(p) if !p.closed && near(p.stroke.width, 0.07 * U, 1e-4) => {
                        Some((i, p))
                    }
                    _ => None,
                })
                .collect();
            let [(at, line)] = &curves[..] else {
                panic!("page {tab}: {} curves", curves.len());
            };
            assert_eq!(
                line.points,
                transfer_points(r, 1.0, |input| shape(SMOOTH, 0.5, 0.5, input)),
                "page {tab}"
            );
            // Over the pad: after its crosshair dot (radius 3 at the pad's centre for X = Y = 0.5).
            let crossing = rig.shapes.iter().position(|c| {
                matches!(&c.shape, Shape::Circle(k) if k.center == r.center() && k.radius == 3.0)
            });
            assert!(
                crossing.is_some_and(|c| c < *at),
                "page {tab}: {crossing:?}, curve at {at}"
            );
        }
    }
}
