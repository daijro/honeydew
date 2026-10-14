//! Autoduck (ID 29), UIHeight 6.5 U.
//!
//! The envelope graph (#7) is module-drawn: [`graph`] draws one ducking cycle, [`duck`], on a gridded frame.

use eframe::egui::{Color32, Id, Rect, Stroke, StrokeKind, Ui};

use crate::panels::build::*;
use crate::ui::{UiCtx, widgets};

/// The envelope graph's grid lines, as (fraction of E, alpha of `background`).
const GRID: [(f32, f32); 3] = [(0.25, 0.3), (0.5, 0.7), (0.75, 0.3)];
/// The curve's line width, device pixels.
const CURVE_PX: f32 = 2.0;

/// The release segment's shape for type `kind` = int(p7·5) and c = Curve, on e in [0, 1].
fn release(e: f32, c: f32, kind: usize) -> f32 {
    let s_curve = |n: f32| {
        let k = (1.0 - c) * n + 1.0;
        if e < 0.5 {
            0.5 * (2.0 * e).powf(k)
        } else {
            1.0 - 0.5 * (2.0 * (1.0 - e)).powf(k)
        }
    };
    match kind {
        0 => e.powf(0.5 + 0.5 * c),
        1 => e.powf(0.1 + 0.9 * c),
        2 => e.powf(3.0 - 2.0 * c),
        3 => e.powf(6.0 - 5.0 * c),
        4 => s_curve(2.0),
        _ => s_curve(5.0),
    }
}

/// The curve: F(x) = 1 − sqrt(Ducking)·e(t), one ducking cycle across x in [0, 1], gain 1 at the top. All inputs are
/// The normalised params; a missing param reads 0.
fn duck(params: &[f32], x: f32) -> f32 {
    let v = |i: usize| params.get(i).copied().unwrap_or(0.0);
    let (ducking, c, shift, hold) = (v(1), v(3), v(5), v(6));
    let kind = ((v(7) * 5.0) as usize).min(5);
    let t = (x + 0.5 - shift).rem_euclid(1.0);
    let a = 0.5 * v(2) + 0.001;
    let end = v(4).max(0.001);
    let hold_end = hold * end + a;
    // A clamp of e to ≤ 1 would never act here: t/a < 1 in the attack, and the release starts below 1. Time is
    // checked before Hold, as the engine does: past Time the envelope is 0, even inside a longer hold.
    let e = if t < a {
        (t / a).powf(0.5 + 0.5 * c)
    } else if t >= end {
        0.0
    } else if t <= hold_end {
        1.0
    } else {
        release(1.0 - (t - hold_end) / (end - hold_end), c, kind)
    };
    1.0 - ducking.sqrt() * e
}

/// The envelope graph's step 2: the 1 px grid lines across E (points) at `ppp`, each with its `background` alpha;
/// vertical then horizontal per fraction. Snapped to whole pixels by the XY pad's [`widgets::line`].
fn grid(e: Rect, ppp: f32) -> [(Rect, f32); 6] {
    let mut lines = [(Rect::NOTHING, 0.0); 6];
    for (i, (f, alpha)) in GRID.into_iter().enumerate() {
        lines[2 * i] = (
            widgets::line(e, true, e.left() + f * e.width(), 1.0, ppp),
            alpha,
        );
        lines[2 * i + 1] = (
            widgets::line(e, false, e.top() + f * e.height(), 1.0, ppp),
            alpha,
        );
    }
    lines
}

/// The envelope graph (#7): on E = #7 grown by 1 px, a `controlFill` fill, the grid, the curve plot of [`duck`]
/// on #7 (2 px, channel colour) and a 1 px `background` outline. No playhead; nothing to edit.
fn graph(
    ui: &mut Ui,
    cx: &UiCtx,
    _: Id,
    rect: Rect,
    module: &mut ModuleInstance,
    _: &mut PanelState,
    ink: Color32,
) -> Vec<usize> {
    let (ppp, colors) = (cx.m.ppp(), &cx.theme.colors);
    let painter = ui.painter();
    let e = rect.expand(1.0 / ppp);
    let background = colors.background.to_color32();
    painter.rect_filled(e, 0.0, colors.control_fill.to_color32());
    for (line, alpha) in grid(e, ppp) {
        painter.rect_filled(line, 0.0, background.gamma_multiply(alpha));
    }
    widgets::curve_plot(painter, rect, CURVE_PX / ppp, ppp, ink, |x| {
        duck(&module.params, x)
    });
    painter.rect_stroke(
        e,
        0.0,
        Stroke::new(1.0 / ppp, background),
        StrokeKind::Inside,
    );
    Vec::new()
}

pub fn panel() -> Option<Panel> {
    Some(Panel::new(hstack([
        spinner(1, "").p(7),
        v_slider(2, "Attack").p(2).value_text(),
        v_slider(3, "Hold").p(6).value_text(),
        v_slider(4, "Ducking").p(1).value_text(),
        v_slider(5, "Curve").p(3).value_text(),
        v_slider(6, "Time").p(4).value_text(),
        vstack([
            // The envelope graph.
            display(7)
                .h(Size::Flex(1.5))
                .margins(0.25, 0.5, 0.25, 0.5)
                .draw(graph),
            h_slider(8, "Shift").p(5),
        ])
        .w(Size::Flex(3.0)),
    ])))
}

#[cfg(test)]
pub const REFERENCE: Reference = Reference {
    size: [14.0, 4.49],
    settings: &[],
    cases: &[Case {
        rows: &[
            row(1, Widget::Spinner, "", [0.0, 0.0, 1.55, 4.49]).p(7),
            row(2, Widget::VSlider, "Attack", [1.55, 0.0, 1.56, 4.49])
                .p(2)
                .value_text(),
            row(3, Widget::VSlider, "Hold", [3.11, 0.0, 1.55, 4.49])
                .p(6)
                .value_text(),
            row(4, Widget::VSlider, "Ducking", [4.66, 0.0, 1.56, 4.49])
                .p(1)
                .value_text(),
            row(5, Widget::VSlider, "Curve", [6.22, 0.0, 1.55, 4.49])
                .p(3)
                .value_text(),
            row(6, Widget::VSlider, "Time", [7.77, 0.0, 1.56, 4.49])
                .p(4)
                .value_text(),
            row(7, Widget::Display, "", [9.83, 0.25, 3.67, 2.39])
                .h(Size::Flex(1.5))
                .margins(0.25, 0.5, 0.25, 0.5),
            row(8, Widget::HSlider, "Shift", [9.33, 2.89, 4.67, 1.6]).p(5),
        ],
        ..Case::DEFAULT
    }],
};

#[cfg(test)]
mod tests {
    use eframe::egui::{Shape, pos2, vec2};

    use super::*;
    use crate::panels::place;
    use crate::panels::tests::{U, display_shapes};
    use crate::params::menu_value;
    use crate::theme::theme_by_id;

    fn near(a: f32, b: f32) -> bool {
        (a - b).abs() < 1e-4
    }

    /// The registry defaults: Ducking .501, Attack .1, Curve .501, Time 1, Shift .5, Hold 0, Accel.
    fn defaults() -> Vec<f32> {
        ModuleInstance::new(29, 2).params
    }

    /// Full ducking, no attack (a = 0.001) or hold, Time 1, no shift: the release runs from x = 0.001 to 1 and
    /// F = 1 − release(e).
    fn release_only(kind: usize, c: f32) -> Vec<f32> {
        let mut p = defaults();
        (p[1], p[2], p[3], p[4], p[5], p[6], p[7]) =
            (1.0, 0.0, c, 1.0, 0.5, 0.0, menu_value(kind, 6));
        p
    }

    /// The x where the linear release e = 1 − (x − 0.001)/0.999 is `e`, in [`release_only`].
    fn at_release(e: f32) -> f32 {
        0.001 + (1.0 - e) * 0.999
    }

    #[test]
    fn duck_at_the_defaults_attacks_holds_and_releases() {
        let p = defaults();
        let depth = 0.501f32.sqrt();
        // a = 0.5·0.1 + 0.001 = 0.051; Shift 0.5 makes t = x.
        assert!(near(duck(&p, 0.0), 1.0));
        // Half the attack: (1/2)^(0.5 + 0.5·0.501).
        assert!(near(duck(&p, 0.0255), 1.0 - depth * 0.5f32.powf(0.7505)));
        // The end of the attack is full ducking (no hold).
        assert!(near(duck(&p, 0.051), 1.0 - depth));
        // The release, Accel: e^(0.5 + 0.5c) of e = 1 − (t − 0.051)/0.949.
        let e: f32 = 1.0 - (0.5 - 0.051) / 0.949;
        assert!(near(duck(&p, 0.5), 1.0 - depth * e.powf(0.7505)));
        // Time 1: the cycle ends back at gain 1, where it started.
        assert!(near(duck(&p, 1.0), 1.0));
    }

    #[test]
    fn duck_holds_for_hold_times_time_after_the_attack() {
        let mut p = defaults();
        (p[1], p[2], p[4], p[6]) = (1.0, 0.0, 0.5, 0.5);
        // a = 0.001, T = 0.5, holdEnd = 0.25 + 0.001; then 0 after T.
        assert!(near(duck(&p, 0.1), 0.0));
        assert!(near(duck(&p, 0.25), 0.0));
        assert!(duck(&p, 0.3) > 0.0 && duck(&p, 0.3) < 1.0);
        assert!(near(duck(&p, 0.5), 1.0));
        assert!(near(duck(&p, 0.9), 1.0));
    }

    #[test]
    fn duck_with_time_shorter_than_the_attack_ducks_until_the_attack_ends() {
        let mut p = defaults();
        // Attack 1: a = 0.501; Time 0.2: holdEnd = 0.501 > T, so no release; 0 after the hold. Curve 1: a linear
        // attack.
        (p[1], p[2], p[3], p[4], p[6]) = (1.0, 1.0, 1.0, 0.2, 0.0);
        assert!(near(duck(&p, 0.25), 1.0 - 0.25 / 0.501));
        assert!(near(duck(&p, 0.501), 0.0));
        assert!(near(duck(&p, 0.6), 1.0));
    }

    /// The graph checks Time before Hold, as the engine does: past Time the envelope is 0, even
    /// inside a hold that runs longer.
    #[test]
    fn duck_ends_at_time_even_inside_the_hold() {
        let mut p = defaults();
        // a = 0.101, T = 0.3, holdEnd = 0.3 + 0.101 = 0.401.
        (p[1], p[2], p[4], p[6]) = (1.0, 0.2, 0.3, 1.0);
        assert!(near(duck(&p, 0.2), 0.0), "held");
        assert!(near(duck(&p, 0.35), 1.0), "past Time");
    }

    #[test]
    fn each_release_type_shapes_the_release_at_c_0_half_and_1() {
        // (type, c, shaped value at e = 0.25, at e = 0.75)
        let table: [(usize, f32, f32, f32); 18] = [
            // Accel e^(0.5 + 0.5c)
            (0, 0.0, 0.5, 0.866_025),
            (0, 0.5, 0.353_553, 0.805_927),
            (0, 1.0, 0.25, 0.75),
            // Accel 2 e^(0.1 + 0.9c)
            (1, 0.0, 0.870_551, 0.971_642),
            (1, 0.5, 0.466_516, 0.853_658),
            (1, 1.0, 0.25, 0.75),
            // Decel e^(3 − 2c)
            (2, 0.0, 0.015_625, 0.421_875),
            (2, 0.5, 0.0625, 0.5625),
            (2, 1.0, 0.25, 0.75),
            // Decel 2 e^(6 − 5c)
            (3, 0.0, 0.000_244, 0.177_979),
            (3, 0.5, 0.007_813, 0.365_354),
            (3, 1.0, 0.25, 0.75),
            // S-Curve, n = 2: k = 3, 2, 1
            (4, 0.0, 0.0625, 0.9375),
            (4, 0.5, 0.125, 0.875),
            (4, 1.0, 0.25, 0.75),
            // S-Curve 2, n = 5: k = 6, 3.5, 1
            (5, 0.0, 0.007_813, 0.992_188),
            (5, 0.5, 0.044_194, 0.955_806),
            (5, 1.0, 0.25, 0.75),
        ];
        for (kind, c, low, high) in table {
            let p = release_only(kind, c);
            for (e, want) in [(0.25, low), (0.75, high)] {
                let got = 1.0 - duck(&p, at_release(e));
                assert!(near(got, want), "type {kind} c {c} e {e}: {got} != {want}");
            }
            // Every type holds 1 and ends at 0.
            assert!(near(duck(&p, 0.001), 0.0), "type {kind}");
            assert!(near(duck(&p, 1.0), 1.0), "type {kind}");
        }
    }

    #[test]
    fn the_attack_shape_is_the_same_for_every_type() {
        for kind in 0..6 {
            let mut p = release_only(kind, 0.0);
            (p[2], p[4]) = (1.0, 1.0);
            // a = 0.501: e = 0.25 at t = 0.12525, shaped e^0.5.
            assert!(near(duck(&p, 0.125_25), 0.5), "type {kind}");
        }
    }

    #[test]
    fn shift_moves_the_phase_and_wraps() {
        let base = defaults();
        for (shift, x, t) in [
            (0.0, 0.6, 0.1),
            (1.0, 0.2, 0.7),
            (0.25, 0.9, 0.15),
            (0.75, 0.1, 0.85),
        ] {
            let mut p = base.clone();
            p[5] = shift;
            // t = x + 0.5 − Shift wrapped into 0…1; Shift 0.5 reads t = x.
            assert!(near(duck(&p, x), duck(&base, t)), "shift {shift} x {x}");
        }
    }

    #[test]
    fn the_grid_lines_sit_at_quarters_of_e_with_their_alphas() {
        let r = |x0: f32, y0: f32, x1: f32, y1: f32| Rect::from_min_max(pos2(x0, y0), pos2(x1, y1));
        // E 119 × 79 px at (10, 20): 25 % at 39.75 → pixel 39, 50 % at 69.5 → 69, 75 % at 99.25 → 99.
        let e = Rect::from_min_size(pos2(10.0, 20.0), vec2(119.0, 79.0));
        assert_eq!(
            grid(e, 1.0),
            [
                (r(39.0, 20.0, 40.0, 99.0), 0.3),
                (r(10.0, 39.0, 129.0, 40.0), 0.3),
                (r(69.0, 20.0, 70.0, 99.0), 0.7),
                (r(10.0, 59.0, 129.0, 60.0), 0.7),
                (r(99.0, 20.0, 100.0, 99.0), 0.3),
                (r(10.0, 79.0, 129.0, 80.0), 0.3),
            ]
        );
        // ppp 2: E 239 × 159 device px; 1 px is half a point.
        let e = Rect::from_min_size(pos2(10.0, 20.0), vec2(119.5, 79.5));
        assert_eq!(
            grid(e, 2.0),
            [
                (r(39.5, 20.0, 40.0, 99.5), 0.3),
                (r(10.0, 39.5, 129.5, 40.0), 0.3),
                (r(69.5, 20.0, 70.0, 99.5), 0.7),
                (r(10.0, 59.5, 129.5, 60.0), 0.7),
                (r(99.5, 20.0, 100.0, 99.5), 0.3),
                (r(10.0, 79.5, 129.5, 80.0), 0.3),
            ]
        );
    }

    /// #7's rect (points) at the reference content rect and `ppp`.
    fn graph_rect(ppp: f32) -> Rect {
        let module = ModuleInstance::new(29, 2);
        let content = Rect::from_min_size(pos2(0.0, 0.0), vec2(14.0 * U * ppp, 4.49 * U * ppp));
        let panel = panel().unwrap();
        let placed = place(&panel, &module, &PanelState::default(), content, U * ppp);
        placed.into_iter().find(|(l, _)| l.id == 7).unwrap().1 / ppp
    }

    #[test]
    fn the_graph_draws_fill_grid_curve_outline_in_order() {
        let colors = theme_by_id(0).colors;
        let (fill, background) = (
            colors.control_fill.to_color32(),
            colors.background.to_color32(),
        );
        let content = Rect::from_min_size(pos2(0.0, 0.0), vec2(14.0 * U, 4.49 * U));
        for ppp in [1.0, 2.0] {
            let (rect, px) = (graph_rect(ppp), 1.0 / ppp);
            let e = rect.expand(px);
            let shapes = display_shapes(29, 7, &[], &PanelState::default(), content, ppp);
            assert_eq!(shapes.len(), 1 + 6 + 1 + 1, "ppp {ppp}");
            // 1. E filled `controlFill`.
            let Shape::Rect(first) = &shapes[0] else {
                panic!("{:?}", shapes[0])
            };
            assert_eq!((first.rect, first.fill), (e, fill));
            // 2. The grid, `background` at 30 / 70 / 30 %.
            for (shape, (at, alpha)) in shapes[1..7].iter().zip(grid(e, ppp)) {
                let Shape::Rect(line) = shape else {
                    panic!("{shape:?}")
                };
                assert_eq!(
                    (line.rect, line.fill),
                    (at, background.gamma_multiply(alpha))
                );
            }
            // 3. The curve plot on #7, 2 px, channel colour: one sample per device pixel of width.
            let Shape::Path(curve) = &shapes[7] else {
                panic!("{:?}", shapes[7])
            };
            assert_eq!(curve.stroke.width, 2.0 * px);
            assert_eq!(
                curve.stroke.color,
                eframe::epaint::ColorMode::Solid(Color32::RED)
            );
            assert_eq!(
                curve.points.len(),
                ((rect.width() - 2.0 * px) * ppp) as usize
            );
            assert!(curve.points.iter().all(|p| rect.contains(*p)));
            // 4. The 1 px `background` outline round E, inside it: the ring just outside #7.
            let Shape::Rect(outline) = &shapes[8] else {
                panic!("{:?}", shapes[8])
            };
            assert_eq!((outline.rect, outline.stroke_kind), (e, StrokeKind::Inside));
            assert_eq!(outline.stroke, Stroke::new(px, background));
            assert_eq!(outline.fill, Color32::TRANSPARENT);
        }
    }

    #[test]
    fn the_curve_starts_at_gain_1_and_ducks_by_sqrt_ducking() {
        let content = Rect::from_min_size(pos2(0.0, 0.0), vec2(14.0 * U, 4.49 * U));
        let rect = graph_rect(1.0);
        let shapes = display_shapes(29, 7, &[], &PanelState::default(), content, 1.0);
        let Shape::Path(curve) = &shapes[7] else {
            panic!()
        };
        // Gain 1 at the top: y = top + lw/2; the deepest point is 1 − sqrt(0.501) of the way up.
        assert!(near(curve.points[0].y, rect.top() + 1.0));
        let deepest = curve.points.iter().map(|p| p.y).fold(f32::MIN, f32::max);
        let want = rect.bottom() - 1.0 - (rect.height() - 2.0) * (1.0 - 0.501f32.sqrt());
        assert!((deepest - want).abs() < 1.0, "{deepest} vs {want}");
    }
}
