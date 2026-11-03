//! The Spreader panel (ID 44), UIHeight 8 U.

use eframe::egui::{Color32, Id, Mesh, Painter, Pos2, Rect, Shape, Stroke, Ui, vec2};
use honeydew_audio::analysis::{self, PETAL_BINS, PetalView};

use super::live_analysis;
use crate::panels::build::*;
use crate::ui::UiCtx;

/// `Mono`: off a `moduleFill` disc with a `text` label, on a `text` disc with a `moduleFill` label.
const MONO: ButtonColors = ButtonColors {
    fill_on: Ink::Text,
    fill_off: Ink::ModuleFill,
    text_on: Ink::ModuleFill,
    text_off: Some(Ink::Text),
};

/// The vectorscope at rest and without a feed: balance 0 and an empty petal.
const REST: PetalView = PetalView {
    balance: 0.0,
    bins: [0.0; PETAL_BINS],
};

/// The petal's values at 7.5° steps from 15° to 165°, as the Analyzer's: the 11 bins with 0.9 × the smaller
/// neighbour inserted between each pair, the scallops. (The petal has 22 points; the 21 between 15° and 165° are the
/// ones that carry data, radius 0 outside.)
fn scallops(bins: &[f32; PETAL_BINS]) -> [f32; 2 * PETAL_BINS - 1] {
    std::array::from_fn(|k| match k % 2 {
        0 => bins[k / 2],
        _ => 0.9 * bins[k / 2].min(bins[k / 2 + 1]),
    })
}

/// The petal's outline in the scope centred `c` of radius `r`: from C out to 15°, Catmull-Rom through the scallops at
/// 16 steps per 15° (radius = value·0.9 R, never below 0), and back to C from 165°.
pub(super) fn petal(c: Pos2, r: f32, bins: &[f32; PETAL_BINS]) -> Vec<Pos2> {
    let values = scallops(bins);
    let last = values.len() - 1;
    let at = |i: isize| values[i.clamp(0, last as isize) as usize];
    let steps = 16 * (values.len() - 1) / 2;
    let mut points = vec![c];
    points.extend((0..=steps).map(|step| {
        let t = step as f32 * last as f32 / steps as f32;
        let (i, f) = (t.floor() as isize, t.fract());
        let (p0, p1, p2, p3) = (at(i - 1), at(i), at(i + 1), at(i + 2));
        let v = 0.5
            * (2.0 * p1
                + (p2 - p0) * f
                + (2.0 * p0 - 5.0 * p1 + 4.0 * p2 - p3) * f * f
                + (3.0 * p1 - p0 - 3.0 * p2 + p3) * f * f * f);
        polar(c, v.max(0.0) * 0.9 * r, 15.0 + 7.5 * t)
    }));
    points.push(c);
    points
}

/// The petal drawn: each step's triangle (C, previous point, point) filled G at 20 % (one mesh), then the 1 px
/// G outline. Nothing for an empty petal.
pub(super) fn draw_petal(
    painter: &Painter,
    c: Pos2,
    r: f32,
    bins: &[f32; PETAL_BINS],
    ink: Color32,
    px: f32,
) {
    if bins.iter().all(|&v| v <= 0.0) {
        return;
    }
    let points = petal(c, r, bins);
    let mut mesh = Mesh::default();
    let fill = ink.gamma_multiply(0.2);
    mesh.colored_vertex(c, fill);
    for &p in &points[1..points.len() - 1] {
        mesh.colored_vertex(p, fill);
    }
    for i in 1..points.len() as u32 - 2 {
        mesh.add_triangle(0, i, i + 1);
    }
    painter.add(Shape::mesh(mesh));
    painter.add(Shape::line(points, Stroke::new(px, ink)));
}

/// The point at angle `deg` (counter-clockwise from +x, y up) at distance `r` from `c`.
pub(super) fn polar(c: Pos2, r: f32, deg: f32) -> Pos2 {
    let (sin, cos) = deg.to_radians().sin_cos();
    c + r * vec2(cos, -sin)
}

/// The vectorscope geometry, shared with the Analyzer, in the plot rect `p` (no labels: the control rect): the
/// centre C = (P.x + Pw/2, Pb) and R = min(Ph, Pw/2).
pub(super) fn scope(p: Rect) -> (Pos2, f32) {
    (p.center_bottom(), p.height().min(p.width() / 2.0))
}

/// A half-circle guide arc of radius `r` around `c`, 0…180° in 6° segments: 31 points from 180° over the top to 0°,
/// clockwise on screen.
pub(super) fn half_arc(c: Pos2, r: f32) -> Vec<Pos2> {
    (0..=30)
        .map(|i| polar(c, r, 180.0 - 6.0 * i as f32))
        .collect()
}

/// The centre of a balance dot: 0.95·min(Ph, Pw) from C at 90° + balance·45°, so a left-heavy signal moves it left.
pub(super) fn balance_dot(p: Rect, balance: f32) -> Pos2 {
    let (c, _) = scope(p);
    polar(c, 0.95 * p.height().min(p.width()), 90.0 + 45.0 * balance)
}

/// The vectorscope (#3) of the Spreader's output, from its feed (the engine bins and smooths it).
/// First the module's backdrop: the `controlFill` upper half-disc of radius 0.9·min(h, w/2), the outer guide arc's
/// polygon, and a 2 px `background` baseline across its diameter. Then the scope with a transparent frame: the 1 px
/// `background` guides (the 45° and 135° lines of 0.9 R, the arcs at 0.9, 0.6 and 0.3 R), the petal (none at rest),
/// and the balance dots of radius 5 px, the fixed `background` one straight up and G's at the balance.
fn vectorscope(
    ui: &mut Ui,
    cx: &UiCtx,
    _: Id,
    p: Rect,
    module: &mut ModuleInstance,
    _: &mut PanelState,
    ink: Color32,
) -> Vec<usize> {
    let feed = live_analysis(ui, module.instance_id).and_then(|s| analysis::spreader(&s));
    let mut scope_feed = feed.unwrap_or(REST);
    // A NaN draws as nothing: no petal, the balance at the centre.
    let unit = |v: f32, lo: f32| if v.is_nan() { 0.0 } else { v.clamp(lo, 1.0) };
    scope_feed.bins = scope_feed.bins.map(|v| unit(v, 0.0));
    let balance = unit(scope_feed.balance, -1.0);
    let (painter, px) = (ui.painter(), 1.0 / cx.m.ppp());
    let colors = &cx.theme.colors;
    let (fill, bg) = (
        colors.control_fill.to_color32(),
        colors.background.to_color32(),
    );
    let (c, r) = scope(p);
    painter.add(Shape::convex_polygon(
        half_arc(c, 0.9 * r),
        fill,
        Stroke::NONE,
    ));
    let half = vec2(0.9 * r, 0.0);
    painter.line_segment([c - half, c + half], Stroke::new(2.0 * px, bg));
    let hairline = Stroke::new(px, bg);
    for deg in [45.0, 135.0] {
        painter.line_segment([c, polar(c, 0.9 * r, deg)], hairline);
    }
    for k in [0.9, 0.6, 0.3] {
        painter.add(Shape::line(half_arc(c, k * r), hairline));
    }
    draw_petal(painter, c, r, &scope_feed.bins, ink, px);
    painter.circle_filled(balance_dot(p, 0.0), 5.0 * px, bg);
    painter.circle_filled(balance_dot(p, balance), 5.0 * px, ink);
    Vec::new()
}

pub fn panel() -> Option<Panel> {
    Some(Panel::new(hstack([
        vstack([knob(1, "Width").p(1), knob(2, "Separation").p(4)]).ml(0.5),
        display(3)
            .w(Size::Percent(60.0))
            .h(Size::Percent(85.0))
            .draw(vectorscope),
        vstack([
            knob(4, "Low Bypass").p(2),
            button(5, "Mono", true)
                .p(3)
                .colors(MONO)
                .halo(2.0)
                .w(Size::Flex(0.9))
                .margins(0.4, 0.0, -0.4, 0.0),
        ])
        .mr(0.5),
    ])))
}

#[cfg(test)]
pub const REFERENCE: Reference = Reference {
    size: [14.0, 5.99],
    settings: &[],
    cases: &[Case {
        rows: &[
            row(1, Widget::Knob, "Width", [0.5, 0.0, 2.29, 2.99]).p(1),
            row(2, Widget::Knob, "Separation", [0.5, 2.99, 2.29, 3.0]).p(4),
            row(3, Widget::Display, "", [2.79, 0.44, 8.41, 5.1])
                .w(Size::Percent(60.0))
                .h(Size::Percent(85.0)),
            row(4, Widget::Knob, "Low Bypass", [11.2, 0.0, 2.3, 2.99]).p(2),
            row(
                5,
                Widget::Button { round: true },
                "Mono",
                [11.31, 3.39, 2.07, 3.0],
            )
            .p(3)
            .w(Size::Flex(0.9))
            .margins(0.4, 0.0, -0.4, 0.0)
            .halo(2.0)
            .colors(ButtonColors {
                fill_on: Ink::Text,
                fill_off: Ink::ModuleFill,
                text_on: Ink::ModuleFill,
                text_off: Some(Ink::Text),
            }),
        ],
        ..Case::DEFAULT
    }],
};

#[cfg(test)]
mod tests {
    use eframe::egui::{Shape, Stroke, pos2, vec2};

    use super::*;
    use crate::panels::place;
    use crate::panels::tests::{U, display_shapes};
    use crate::theme::theme_by_id;

    fn near(a: Pos2, b: Pos2) -> bool {
        (a - b).length() < 1e-3
    }

    /// The reference content rect, 14 × 5.99 U.
    const CONTENT: Rect = Rect {
        min: pos2(0.0, 0.0),
        max: pos2(14.0 * U, 5.99 * U),
    };

    #[test]
    fn the_scope_sits_on_the_bottom_centre_with_the_smaller_of_its_height_and_half_width() {
        // C = (P.x + Pw/2, Pb), R = min(Ph, Pw/2). The Spreader's reference #3 (2.79, 0.44, 8.41, 5.1) U is
        // width-limited; the Analyzer's STEREO view (0.7, 0.3, 12.6, 6.14) U is height-limited.
        let spreader = Rect::from_min_size(pos2(2.79 * U, 0.44 * U), vec2(8.41 * U, 5.1 * U));
        let (c, r) = scope(spreader);
        assert!(
            near(c, pos2(6.995 * U, 5.54 * U)) && (r - 4.205 * U).abs() < 1e-3,
            "{c:?} {r}"
        );
        let analyzer = Rect::from_min_size(pos2(0.7 * U, 0.3 * U), vec2(12.6 * U, 6.14 * U));
        assert!((scope(analyzer).1 - 6.14 * U).abs() < 1e-3);
        // The balance dots: 0.95·min(Ph, Pw) from C, straight up at balance 0, 45° further left at +1 (left-heavy).
        let up = balance_dot(spreader, 0.0);
        assert!(
            near(up, pos2(6.995 * U, 5.54 * U - 0.95 * 5.1 * U)),
            "{up:?}"
        );
        let left = balance_dot(spreader, 1.0);
        let d = 0.95 * 5.1 * U * std::f32::consts::FRAC_1_SQRT_2;
        assert!(near(left, pos2(6.995 * U - d, 5.54 * U - d)), "{left:?}");
    }

    #[test]
    fn a_guide_arc_is_the_upper_half_circle_in_6_degree_segments() {
        let c = pos2(100.0, 200.0);
        let arc = half_arc(c, 50.0);
        assert_eq!(arc.len(), 31);
        // From 180° (left) over the top to 0° (right): clockwise on screen.
        assert!(near(arc[0], pos2(50.0, 200.0)) && near(arc[15], pos2(100.0, 150.0)));
        assert!(near(arc[30], pos2(150.0, 200.0)));
        for (i, p) in arc.iter().enumerate() {
            assert!(
                ((*p - c).length() - 50.0).abs() < 1e-3 && p.y <= c.y + 1e-3,
                "{i}: {p:?}"
            );
            let deg = 180.0 - 6.0 * i as f32;
            assert!(near(*p, polar(c, 50.0, deg)), "{i}");
        }
        // Counter-clockwise from +x with y up.
        assert!(near(polar(c, 10.0, 90.0), pos2(100.0, 190.0)));
        assert!(near(
            polar(c, 10.0, 45.0),
            c + 10.0 * vec2(0.5f32.sqrt(), -(0.5f32.sqrt()))
        ));
    }

    #[test]
    fn at_rest_the_scope_is_the_half_disc_baseline_guides_and_both_balance_dots() {
        let colors = theme_by_id(0).colors;
        let (fill, bg) = (
            colors.control_fill.to_color32(),
            colors.background.to_color32(),
        );
        let panel = panel().unwrap();
        let module = ModuleInstance::new(44, 2);
        for ppp in [1.0, 2.0] {
            let placed = place(
                &panel,
                &module,
                &PanelState::default(),
                CONTENT * ppp,
                U * ppp,
            );
            let p = placed.iter().find(|(l, _)| l.id == 3).unwrap().1 / ppp;
            let reference = Rect::from_min_size(pos2(2.79 * U, 0.44 * U), vec2(8.41 * U, 5.1 * U));
            assert!(
                (p.min - reference.min).length() < 1.0 && (p.max - reference.max).length() < 1.0
            );
            let (c, r) = scope(p);
            let px = 1.0 / ppp;
            let hairline = Stroke::new(px, bg);
            let mut expected = vec![
                // Before the scope: the `controlFill` upper half-disc of radius 0.9 R and a 2 px `background`
                // baseline across its diameter.
                Shape::convex_polygon(half_arc(c, 0.9 * r), fill, Stroke::NONE),
                Shape::line_segment(
                    [c - vec2(0.9 * r, 0.0), c + vec2(0.9 * r, 0.0)],
                    Stroke::new(2.0 * px, bg),
                ),
                // The guides, 1 px `background`: the 45° and 135° lines, then the arcs at 0.9, 0.6 and 0.3 R.
                Shape::line_segment([c, polar(c, 0.9 * r, 45.0)], hairline),
                Shape::line_segment([c, polar(c, 0.9 * r, 135.0)], hairline),
            ];
            for k in [0.9, 0.6, 0.3] {
                expected.push(Shape::line(half_arc(c, k * r), hairline));
            }
            // No petal at rest; the fixed `background` dot straight up, then G's at balance 0 over it, radius 5 px.
            let up = balance_dot(p, 0.0);
            expected.push(Shape::circle_filled(up, 5.0 * px, bg));
            expected.push(Shape::circle_filled(up, 5.0 * px, Color32::RED));
            let shapes = display_shapes(44, 3, &[], &PanelState::default(), CONTENT, ppp);
            assert_eq!(shapes, expected, "ppp {ppp}");
        }
    }

    #[test]
    fn the_petal_runs_through_its_bins_and_scallops_from_15_to_165_degrees() {
        let c = pos2(100.0, 200.0);
        let mut bins = [0.0; PETAL_BINS];
        bins[5] = 1.0;
        bins[2] = 0.5;
        let points = petal(c, 100.0, &bins);
        // C, then 161 steps (16 per 15° over 150°), then C.
        assert_eq!(points.len(), 163);
        assert_eq!((points[0], points[162]), (c, c));
        // On a bin's angle the radius is value·0.9 R: 90° (bin 5) is straight up at 90; 45° (bin 2) at 45.
        assert!(
            near(points[1 + 80], pos2(100.0, 200.0 - 90.0)),
            "{:?}",
            points[81]
        );
        assert!(
            near(points[1 + 32], polar(c, 45.0, 45.0)),
            "{:?}",
            points[33]
        );
        // Between two bins the scallop dips to 0.9 × the smaller: 97.5° between 1.0 and 0 is 0.
        assert!(near(points[1 + 88], c));
        // A feed through the panel: the petal mesh and its outline, between the guides and the balance dots.
        let feed = [0.5, 0.0, 0.0, 0.0, 0.0, 0.0, 1.0, 0.0, 0.0, 0.0, 0.0, 0.0];
        for ppp in [1.0, 2.0] {
            let shapes = crate::panels::tests::display_shapes_fed(
                44,
                3,
                &[],
                &PanelState::default(),
                CONTENT,
                ppp,
                (honeydew_audio::MeterSnapshot::default(), &feed),
            );
            assert_eq!(shapes.len(), 11, "ppp {ppp}");
            assert!(matches!(shapes[7], Shape::Mesh(_)), "{:?}", shapes[7]);
            let Shape::Path(outline) = &shapes[8] else {
                panic!("{:?}", shapes[8])
            };
            assert_eq!(outline.points.len(), 163);
            // The G dot leans left with balance 0.5.
            let Shape::Circle(dot) = &shapes[10] else {
                panic!()
            };
            let Shape::Circle(fixed) = &shapes[9] else {
                panic!()
            };
            assert!(dot.center.x < fixed.center.x && dot.fill == Color32::RED);
        }
    }
}
