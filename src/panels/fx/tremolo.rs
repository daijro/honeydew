//! The Tremolo panel (ID 46), UIHeight 9 U.
//!
//! The display (#5) is module-drawn: [`wave`] plots one cycle of the effect's LFO ([`osc`], scaled by [`gain`])
//! between the `Max` and `Min` lines.

use std::f32::consts::FRAC_PI_2;

use eframe::egui::{Color32, Id, Pos2, Rect, Shape, Stroke, Ui, pos2, vec2};

use honeydew_audio::analysis;

use super::live_analysis;
use crate::panels::build::*;
use crate::ui::{UiCtx, widgets};

/// The `Max` and `Min` lines' width, device pixels.
const LEVEL_PX: f32 = 2.0;
/// The waveform's segments across the width.
const SEGMENTS: usize = 256;
/// Its line width, U.
const WAVE_W: f32 = 0.07;

/// The effect's oscillator value in [−1, 1] at `phase` in [0, 1] from Shape (p8), Smooth (p9), Skew
/// (p10) and Width (p11). A missing param reads 0.
fn osc(params: &[f32], phase: f32) -> f32 {
    let v = |i: usize| params.get(i).copied().unwrap_or(0.0);
    let skew = 0.001 + 0.998 * v(10);
    let width = 0.999 - 1.998 * v(11);
    let t = (phase / skew).min((1.0 - phase) / (1.0 - skew));
    let u = ((2.0 * t - 1.0 - width) / (1.0 - width.abs())).clamp(-1.0, 1.0);
    let e = 2f32.powf(4.0 * (2.0 * v(8) - 1.0));
    let a = u.signum() * FRAC_PI_2 * u.abs().powf(e);
    let linear = a / FRAC_PI_2;
    let smooth = 0.001 + 0.998 * v(9);
    linear + smooth * (a.sin() - linear)
}

/// The effect's target gain for oscillator output `o`, between Min (p4) and Max (p3); its peak is 99 %
/// of the way to Max. Min above Max (only a loaded file can hold it) reads Max, without the panic of `clamp`.
fn gain(params: &[f32], o: f32) -> f32 {
    let v = |i: usize| params.get(i).copied().unwrap_or(0.0);
    let (max, min) = (v(3), v(4));
    (min + 0.495 * (max - min) * (o + 1.0)).max(min).min(max)
}

/// The y of level `v` in the display (bottom = 0, top = 1).
fn level_y(rect: Rect, v: f32) -> f32 {
    rect.bottom() - rect.height() * v
}

/// The 2 px line across `rect` at level `v`, centred on its y, on whole pixels at `ppp`. It
/// may overhang the display by half its width at v = 0 and 1, so [`widgets::line`] keeps it inside `rect` grown by
/// that.
fn level_line(rect: Rect, v: f32, ppp: f32) -> Rect {
    let room = rect.expand2(vec2(0.0, LEVEL_PX / 2.0 / ppp));
    widgets::line(room, false, level_y(rect, v), LEVEL_PX, ppp)
}

/// The waveform's 257 points, phase i/256 across the full width of `rect`.
///
/// phase 0…1 of the left oscillator, without the Phase Offset.
fn wave(params: &[f32], rect: Rect) -> Vec<Pos2> {
    (0..=SEGMENTS)
        .map(|i| {
            let phase = i as f32 / SEGMENTS as f32;
            let g = gain(params, osc(params, phase));
            pos2(rect.left() + rect.width() * phase, level_y(rect, g))
        })
        .collect()
}

/// The display: a `controlFill` fill, 2 px `moduleFill` lines at Max and Min, and the waveform, 0.07 U wide in
/// the channel colour. Nothing to edit.
fn plot(
    ui: &mut Ui,
    cx: &UiCtx,
    _: Id,
    rect: Rect,
    module: &mut ModuleInstance,
    _: &mut PanelState,
    ink: Color32,
) -> Vec<usize> {
    let (m, colors) = (cx.m, &cx.theme.colors);
    let painter = ui.painter();
    painter.rect_filled(rect, 0.0, colors.control_fill.to_color32());
    for p in [3, 4] {
        let v = module.params.get(p).copied().unwrap_or(0.0);
        painter.rect_filled(
            level_line(rect, v, m.ppp()),
            0.0,
            colors.module_fill.to_color32(),
        );
    }
    painter.add(Shape::line(
        wave(&module.params, rect),
        Stroke::new(m.u(WAVE_W), ink),
    ));
    // With a feed: the follower dot on the curve at the left oscillator's phase, as the wave is drawn.
    let phase = live_analysis(ui, module.instance_id).and_then(|s| analysis::tremolo(&s));
    if let Some(phase) = phase.filter(|p| (0.0..=1.0).contains(p)) {
        ui.painter()
            .circle_filled(follower(&module.params, rect, phase), m.u(DOT_U), ink);
    }
    Vec::new()
}

/// The follower dot's radius, U.
const DOT_U: f32 = 0.15;

/// The point of the waveform at `phase`.
fn follower(params: &[f32], rect: Rect, phase: f32) -> Pos2 {
    let g = gain(params, osc(params, phase));
    pos2(rect.left() + rect.width() * phase, level_y(rect, g))
}

/// The limits, coupled as the effect couples them: a write of Max (p3) below Min (p4) lowers Min to it, and of
/// Min above Max raises Max to it, so the song holds what the audio does. Returns the param it moved.
fn limits(params: &mut [f32], edited: usize) -> Option<usize> {
    let (max, min) = (*params.get(3)?, *params.get(4)?);
    let (other, to) = match edited {
        3 if min > max => (4, max),
        4 if max < min => (3, min),
        _ => return None,
    };
    params[other] = to;
    Some(other)
}

/// The panel's edits of `Max` (#4) and `Min` (#6): [`limits`].
fn couple(module: &mut ModuleInstance, _: &mut PanelState, edited: &[u32]) -> Vec<usize> {
    let params = edited.iter().filter_map(|&n| match n {
        4 => Some(3),
        6 => Some(4),
        _ => None,
    });
    params
        .filter_map(|p| limits(&mut module.params, p))
        .collect()
}

/// A write of `module`'s param `index` outside its panel (the fine tune dialog, a track's slider, a MIDI CC link):
/// a Tremolo's other limit follows as on its panel ([`limits`]). Other modules have no such coupling.
pub(crate) fn couple_param(module: &mut ModuleInstance, index: i32) {
    if let (46, Ok(index)) = (module.type_id, usize::try_from(index)) {
        limits(&mut module.params, index);
    }
}

/// `Sync` (p7) swaps the `Rate` knob for the spinner.
const SYNC: Show = Show::Item(7, 1);
const FREE: Show = Show::Item(7, 0);

pub fn panel() -> Option<Panel> {
    let panel = Panel::new(hstack([
        vstack([
            knob(1, "Shape").p(8),
            knob(2, "Rate").p(1).mt(0.5).show(FREE),
            spinner(3, "Rate").p(2).mt(0.5).show(SYNC),
        ]),
        vstack([
            hstack([
                tag(4, "Max", true).p(3),
                // The waveform between the Max and Min levels.
                display(5).w(Size::Flex(6.5)).h(Size::Flex(0.88)).draw(plot),
                tag(6, "Min", false).p(4),
            ])
            .margins(0.2, 0.0, 0.4, 0.0),
            hstack([
                toggle(7, "Sync").p(7).w(Size::Flex(0.5)).mb(0.5),
                knob(8, "Smooth").p(9),
                knob(9, "Skew").p(10),
                knob(10, "Width").p(11),
            ])
            .h(Size::Flex(0.5)),
        ])
        .w(Size::Flex(3.0)),
        vstack([knob(11, "Phase").p(5), knob(12, "Stereo").p(6).mt(0.5)]),
    ]));
    Some(panel.hook(couple))
}

/// The rows after the left column, the same in every case.
#[cfg(test)]
const REST: [Row; 9] = [
    row(
        4,
        Widget::Tag { faces_right: true },
        "Max",
        [2.8, 0.2, 0.98, 4.26],
    )
    .p(3),
    row(5, Widget::Display, "", [3.78, 0.45, 6.43, 3.75])
        .w(Size::Flex(6.5))
        .h(Size::Flex(0.88)),
    row(
        6,
        Widget::Tag { faces_right: false },
        "Min",
        [10.21, 0.2, 0.99, 4.26],
    )
    .p(4),
    row(7, Widget::Toggle, "Sync", [2.8, 4.86, 1.2, 1.63])
        .p(7)
        .w(Size::Flex(0.5))
        .margins(0.0, 0.0, 0.5, 0.0),
    row(8, Widget::Knob, "Smooth", [4.0, 4.86, 2.4, 2.13]).p(9),
    row(9, Widget::Knob, "Skew", [6.4, 4.86, 2.4, 2.13]).p(10),
    row(10, Widget::Knob, "Width", [8.8, 4.86, 2.4, 2.13]).p(11),
    row(11, Widget::Knob, "Phase", [11.2, 0.0, 2.8, 3.24]).p(5),
    row(12, Widget::Knob, "Stereo", [11.2, 3.74, 2.8, 3.25])
        .p(6)
        .margins(0.5, 0.0, 0.0, 0.0),
];
#[cfg(test)]
const SHAPE: Row = row(1, Widget::Knob, "Shape", [0.0, 0.0, 2.8, 3.24]).p(8);

#[cfg(test)]
pub const REFERENCE: Reference = Reference {
    size: [14.0, 6.99],
    settings: &[],
    cases: &[
        Case {
            name: "Sync off",
            rows: &[
                SHAPE,
                row(2, Widget::Knob, "Rate", [0.0, 3.74, 2.8, 3.25])
                    .p(1)
                    .margins(0.5, 0.0, 0.0, 0.0),
                REST[0],
                REST[1],
                REST[2],
                REST[3],
                REST[4],
                REST[5],
                REST[6],
                REST[7],
                REST[8],
            ],
            ..Case::DEFAULT
        },
        Case {
            name: "Sync on",
            params: &[(7, 1.0)],
            rows: &[
                SHAPE,
                // The spinner in the knob's slot.
                row(3, Widget::Spinner, "Rate", [0.0, 3.74, 2.8, 3.25])
                    .p(2)
                    .margins(0.5, 0.0, 0.0, 0.0),
                REST[0],
                REST[1],
                REST[2],
                REST[3],
                REST[4],
                REST[5],
                REST[6],
                REST[7],
                REST[8],
            ],
            ..Case::DEFAULT
        },
    ],
};

#[cfg(test)]
mod tests {
    use super::*;
    use crate::panels::place;
    use crate::panels::tests::{U, display_shapes};
    use crate::theme::theme_by_id;

    fn near(a: f32, b: f32) -> bool {
        (a - b).abs() < 1e-4
    }

    /// The registry defaults: Max 1, Min 0, Shape .5, Smooth 1, Skew .5, Width .5.
    fn defaults() -> Vec<f32> {
        ModuleInstance::new(46, 2).params
    }

    /// The defaults with params `(p, v)` set.
    fn with(set: &[(usize, f32)]) -> Vec<f32> {
        let mut p = defaults();
        for &(i, v) in set {
            p[i] = v;
        }
        p
    }

    #[test]
    fn osc_at_the_defaults_is_a_rounded_triangle_from_its_minimum() {
        let p = defaults();
        // "begins at its minimum", peaks half way (Skew 0.5), crosses 0 at the quarters.
        assert!(near(osc(&p, 0.0), -1.0));
        assert!(near(osc(&p, 0.5), 1.0));
        assert!(near(osc(&p, 0.25), 0.0));
        // Rounded: sin(−π/4) blended with the straight line by smooth = 0.999.
        assert!(near(osc(&p, 0.125), -0.706_900));
        assert!(near(osc(&p, 1.0), -1.0));
    }

    #[test]
    fn osc_stays_within_1_for_any_params() {
        let ends = [0.0, 0.5, 1.0];
        for shape in ends {
            for smooth in ends {
                for skew in ends {
                    for width in ends {
                        let p = with(&[(8, shape), (9, smooth), (10, skew), (11, width)]);
                        for i in 0..=1000 {
                            let o = osc(&p, i as f32 / 1000.0);
                            assert!(
                                o.abs() <= 1.0,
                                "{shape} {smooth} {skew} {width} at {i}: {o}"
                            );
                        }
                    }
                }
            }
        }
    }

    #[test]
    fn osc_follows_skew_width_shape_and_smooth() {
        // Skew 1 (0.999): a rising ramp, half way up at phase 0.5.
        assert!(near(osc(&with(&[(10, 1.0)]), 0.5), 0.001_572));
        // Width 1 (−0.999): square-ish, at +1 almost at once.
        let square = with(&[(11, 1.0)]);
        assert!(near(osc(&square, 0.0), -1.0));
        assert!(near(osc(&square, 0.001), 1.0));
        assert!(near(osc(&square, 0.25), 1.0));
        // Smooth 0 (0.001): almost the straight line.
        assert!(near(osc(&with(&[(9, 0.0)]), 0.125), -0.500_207));
        // Shape 0: exponent 1/16, nearly square; Shape 1: exponent 16, nearly flat.
        assert!(near(osc(&with(&[(8, 0.0)]), 0.125), -0.997_743));
        assert!(near(osc(&with(&[(8, 1.0)]), 0.125), -0.000_024));
    }

    #[test]
    fn gain_scales_the_osc_between_min_and_max() {
        let p = defaults();
        assert!(near(gain(&p, -1.0), 0.0));
        assert!(near(gain(&p, 0.0), 0.495));
        // The peak reaches 99 % of the range.
        assert!(near(gain(&p, 1.0), 0.99));
        let p = with(&[(3, 0.8), (4, 0.3)]);
        assert!(near(gain(&p, -1.0), 0.3));
        assert!(near(gain(&p, 1.0), 0.795));
        // Min above Max (a loaded file): no panic, the gain stays at Max.
        let p = with(&[(3, 0.2), (4, 0.8)]);
        assert!(near(gain(&p, -1.0), 0.2));
        assert!(near(gain(&p, 1.0), 0.2));
    }

    #[test]
    fn a_level_line_is_2_px_centred_on_its_level() {
        let r = |x0: f32, y0: f32, x1: f32, y1: f32| Rect::from_min_max(pos2(x0, y0), pos2(x1, y1));
        let rect = r(10.0, 10.0, 210.0, 130.0);
        assert_eq!(level_y(rect, 0.0), 130.0);
        assert_eq!(level_y(rect, 0.5), 70.0);
        assert_eq!(level_y(rect, 1.0), 10.0);
        assert_eq!(level_line(rect, 0.0, 1.0), r(10.0, 129.0, 210.0, 131.0));
        assert_eq!(level_line(rect, 0.25, 1.0), r(10.0, 99.0, 210.0, 101.0));
        assert_eq!(level_line(rect, 1.0, 1.0), r(10.0, 9.0, 210.0, 11.0));
        // ppp 2: 2 px is 1 point; y = 130 − 0.3·120 = 94 → device rows 187, 188.
        assert_eq!(level_line(rect, 0.3, 2.0), r(10.0, 93.5, 210.0, 94.5));
        // Off the pixel grid: y = 130 − 0.33·120 = 90.4 → rows 89.4 rounded to 89, so 89..91.
        assert_eq!(level_line(rect, 0.33, 1.0), r(10.0, 89.0, 210.0, 91.0));
    }

    #[test]
    fn the_wave_has_256_segments_across_the_full_width() {
        let rect = Rect::from_min_size(pos2(10.0, 20.0), vec2(205.0, 120.0));
        let points = wave(&defaults(), rect);
        assert_eq!(points.len(), SEGMENTS + 1);
        assert_eq!(points[0], pos2(10.0, 140.0));
        assert!(near(points[128].x, 10.0 + 102.5));
        assert!(near(points[128].y, 140.0 - 0.99 * 120.0));
        assert!(near(points[256].x, 215.0));
        assert!(near(points[256].y, 140.0));
        // Scaled between Min and Max.
        let p = with(&[(3, 0.75), (4, 0.25)]);
        let points = wave(&p, rect);
        assert!(near(points[0].y, level_y(rect, 0.25)));
        assert!(near(points[128].y, level_y(rect, 0.25 + 0.99 * 0.5)));
    }

    /// The content rect at the reference, and #4, #5 and #6 (points) at `ppp`.
    fn reference(ppp: f32) -> (Rect, [Rect; 3]) {
        let content = Rect::from_min_size(pos2(0.0, 0.0), vec2(14.0 * U, 6.99 * U));
        let module = ModuleInstance::new(46, 2);
        let panel = panel().unwrap();
        let placed = place(
            &panel,
            &module,
            &PanelState::default(),
            content * ppp,
            U * ppp,
        );
        let rect = |n: u32| placed.iter().find(|(l, _)| l.id == n).unwrap().1 / ppp;
        (content, [rect(4), rect(5), rect(6)])
    }

    #[test]
    fn the_display_draws_fill_max_min_and_the_wave_in_order() {
        let colors = theme_by_id(0).colors;
        for ppp in [1.0, 2.0] {
            let (content, [_, rect, _]) = reference(ppp);
            let p = with(&[(3, 0.75), (4, 0.25)]);
            let shapes = display_shapes(46, 5, &p, &PanelState::default(), content, ppp);
            assert_eq!(shapes.len(), 4, "ppp {ppp}");
            let Shape::Rect(fill) = &shapes[0] else {
                panic!("{:?}", shapes[0])
            };
            assert_eq!(
                (fill.rect, fill.fill),
                (rect, colors.control_fill.to_color32())
            );
            for (shape, v) in shapes[1..3].iter().zip([0.75, 0.25]) {
                let Shape::Rect(line) = shape else {
                    panic!("{shape:?}")
                };
                assert_eq!(line.rect, level_line(rect, v, ppp));
                assert_eq!(line.fill, colors.module_fill.to_color32());
            }
            let Shape::Path(wave_shape) = &shapes[3] else {
                panic!("{:?}", shapes[3])
            };
            assert_eq!(wave_shape.points, wave(&p, rect));
            assert_eq!(wave_shape.points.len(), 257);
            assert_eq!(wave_shape.stroke.width, 0.07 * U);
            assert_eq!(
                wave_shape.stroke.color,
                eframe::epaint::ColorMode::Solid(Color32::RED)
            );
        }
    }

    #[test]
    fn a_feed_adds_the_follower_dot_at_the_lfo_phase_on_the_wave() {
        use honeydew_audio::MeterSnapshot;
        for ppp in [1.0, 2.0] {
            let (content, [_, rect, _]) = reference(ppp);
            let p = with(&[(3, 0.75), (4, 0.25)]);
            for phase in [0.0, 0.3, 1.0] {
                let shapes = crate::panels::tests::display_shapes_fed(
                    46,
                    5,
                    &p,
                    &PanelState::default(),
                    content,
                    ppp,
                    (MeterSnapshot::default(), &[phase]),
                );
                assert_eq!(shapes.len(), 5, "ppp {ppp}");
                let Shape::Circle(dot) = &shapes[4] else {
                    panic!("{:?}", shapes[4])
                };
                let want = wave(&p, rect);
                let on = follower(&p, rect, phase);
                assert!(
                    near(dot.radius, 0.15 * U) && dot.fill == Color32::RED,
                    "{dot:?}"
                );
                assert!(
                    near(dot.center.x, on.x) && near(dot.center.y, on.y),
                    "{dot:?}"
                );
                // On the drawn wave: phase 0 and 1 are its ends.
                if phase != 0.3 {
                    let end = want[(phase * 256.0) as usize];
                    assert!(near(dot.center.x, end.x) && near(dot.center.y, end.y));
                }
            }
        }
    }

    #[test]
    fn the_max_and_min_lines_meet_the_tag_pointers() {
        for ppp in [1.0, 2.0] {
            let (content, [max_tag, rect, min_tag]) = reference(ppp);
            for v in [0.0, 0.5, 1.0] {
                let p = with(&[(3, v), (4, v)]);
                let shapes = display_shapes(46, 5, &p, &PanelState::default(), content, ppp);
                for (shape, tag) in shapes[1..3].iter().zip([max_tag, min_tag]) {
                    let Shape::Rect(line) = shape else {
                        panic!("{shape:?}")
                    };
                    // The tag slider: y = bottom − 0.06 h − 0.88 h·v of the tag's rect.
                    let pointer = tag.bottom() - 0.06 * tag.height() - 0.88 * tag.height() * v;
                    let off = (line.rect.center().y - pointer).abs() * ppp;
                    assert!(
                        off <= 1.0,
                        "ppp {ppp} v {v}: {off} px off ({rect:?}, {tag:?})"
                    );
                }
            }
        }
    }

    #[test]
    fn the_hook_moves_the_other_limit_only_past_it_and_only_for_an_edit_of_max_or_min() {
        let mut m = ModuleInstance::new(46, 2);
        let mut state = PanelState::default();
        let mut run = |m: &mut ModuleInstance, max: f32, min: f32, edited: &[u32]| {
            (m.params[3], m.params[4]) = (max, min);
            let moved = couple(m, &mut state, edited);
            (moved, m.params[3], m.params[4])
        };
        // Max (#4) below Min lowers Min; Min (#6) above Max raises Max.
        assert_eq!(run(&mut m, 0.3, 0.5, &[4]), (vec![4], 0.3, 0.3));
        assert_eq!(run(&mut m, 0.3, 0.6, &[6]), (vec![3], 0.6, 0.6));
        // Within the limits, or another control's edit (Shape, #1), or none: nothing moves.
        assert_eq!(run(&mut m, 0.8, 0.5, &[4]), (vec![], 0.8, 0.5));
        assert_eq!(run(&mut m, 0.3, 0.5, &[1]), (vec![], 0.3, 0.5));
        assert_eq!(run(&mut m, 0.3, 0.5, &[]), (vec![], 0.3, 0.5));
        // Outside the panel, by param; another module is left alone.
        couple_param(&mut m, 4);
        assert_eq!((m.params[3], m.params[4]), (0.5, 0.5));
        let mut other = ModuleInstance::new(6, 2);
        (other.params[3], other.params[4]) = (0.2, 1.0);
        couple_param(&mut other, 4);
        assert_eq!(other.params[3], 0.2);
    }
}
