//! The Tape Stop panel (ID 42), UIHeight 8.4 U.

use eframe::egui::{Color32, Rect, Shape, Stroke, Ui, Vec2, pos2, vec2};
use honeydew_audio::analysis;

use super::live_analysis;
use crate::panels::build::*;
use crate::params::module_spec;
use crate::ui::UiCtx;
use crate::ui::widgets;

/// `Sync` (p7) above 0.5 shows the 10-step arcs, so at exactly 0.5 the continuous ones show.
const SYNC: Show = Show::Above(7, 0.5);
const FREE: Show = Show::Not(&SYNC);

/// The play button: on, the channel icon on a `controlFill` disc; off, the icon in
/// `controlFill` with no disc.
const PLAY: ButtonColors = ButtonColors {
    fill_on: Ink::ControlFill,
    fill_off: Ink::ModuleFill,
    text_on: Ink::Channel,
    text_off: Some(Ink::ControlFill),
};

/// The `Sync` toggle's state text, the same in both states.
fn sync_text(_: f32) -> String {
    "Sync".to_owned()
}

/// Drawn after the controls. Over each visible arc `r`: its value readout, shrink-drawn from FontSize 2 (down to
/// 6 pt, then truncated keeping 5 tail characters) in the spool's middle column (r.x + 0.3625 w, r.y, 0.275 w,
/// r.h), and `Start Time` or `Stop Time` top-aligned in the bottom 0.75 U, FontSize 2, plain. Over the
/// `Warp` slider S: the warp curve Env(x, 2v − 1) at (S.x + 0.25 w, S.y, 0.5 w, 0.75 U), 4 px wide, and
/// `Warp` top-aligned in S's bottom 0.75 U. Each arc's spool (at rest) is drawn first, under its texts.
fn readouts(
    ui: &Ui,
    cx: &UiCtx,
    module: &ModuleInstance,
    _: &PanelState,
    rects: &[(u32, Rect)],
    _: Rect,
    channel: Color32,
) {
    let theta = spools(ui, module);
    let (painter, m) = (ui.painter(), cx.m);
    let name = |r: Rect, label: &str| {
        let strip = r.with_min_y(r.bottom() - m.u(0.75));
        let font = m.font_id(FontSize::Pt10);
        painter.text(strip.center_top(), Align2::CENTER_TOP, label, font, channel);
    };
    for &(n, r) in rects {
        let (p, label) = match n {
            1 => (2, "Start Time"),
            2 => (4, "Start Time"),
            5 => (6, "Warp"),
            6 => (1, "Stop Time"),
            7 => (3, "Stop Time"),
            _ => continue,
        };
        if n != 5 {
            let fill = cx.theme.colors.module_fill.to_color32();
            painter.extend(spool(r, theta, channel, fill));
        }
        let Some((spec, &v)) = module_spec(module.type_id)
            .and_then(|s| s.params.get(p))
            .zip(module.params.get(p))
        else {
            continue;
        };
        if n == 5 {
            let plot = Rect::from_min_size(
                pos2(r.left() + 0.25 * r.width(), r.top()),
                vec2(0.5 * r.width(), m.u(0.75)),
            );
            let lw = WARP_LINE_PX / m.ppp();
            widgets::curve_plot(painter, plot, lw, m.ppp(), channel, |x| {
                widgets::env(x, 2.0 * v - 1.0)
            });
        } else {
            let column = Rect::from_min_size(
                pos2(r.left() + 0.3625 * r.width(), r.top()),
                vec2(0.275 * r.width(), r.height()),
            );
            let text = spec.display_text(v);
            widgets::shrink(
                painter,
                m,
                column,
                Align2::CENTER_CENTER,
                &text,
                10,
                channel,
            );
        }
        name(r, label);
    }
}

/// The spools' angle θ (degrees) at rest and without a feed.
const THETA: f32 = 0.0;

/// The spools' angle θ (degrees), shared by both, from the Tape Stop's feed: the engine turns it by −0.005° × the tape
/// playback rate per sample while the sequencer plays, wrapped into [0, 360); it stands while stopped. While it
/// turns the panel asks for every frame, so the spools spin smoothly.
fn spools(ui: &Ui, module: &ModuleInstance) -> f32 {
    let theta = live_analysis(ui, module.instance_id).and_then(|s| analysis::tape_stop(&s));
    let theta = theta.filter(|t| t.is_finite()).unwrap_or(THETA);
    let id = ui.id().with(("tape stop spools", module.instance_id));
    let turned = ui.data_mut(|d| {
        let last = d.get_temp::<f32>(id);
        d.insert_temp(id, theta);
        last != Some(theta)
    });
    if turned && theta != THETA {
        ui.ctx().request_repaint();
    }
    theta
}

/// The spool in the arc rect `r` at angle `theta`: d = min(w, h), centred; θ in degrees with y down, P(φ) the point
/// at φ on the 0.35 d circle. In order: the channel disc 0.35 d, the `moduleFill` disc 0.2905 d, the three channel
/// spokes (centre, P(θ + k), P(θ + k + 60)) for k = 0, 120, 240, the channel hub 0.175 d, the `moduleFill` hole 0.14 d
/// and three `moduleFill` notches of 0.021 d centred on the 0.14 d circle at θ + 90, θ + 210, θ + 330.
fn spool(r: Rect, theta: f32, channel: Color32, fill: Color32) -> Vec<Shape> {
    let (c, d) = (r.center(), r.width().min(r.height()));
    let at = |f: f32, deg: f32| c + f * d * Vec2::angled(deg.to_radians());
    let spoke = |k: f32| {
        let points = vec![c, at(0.35, theta + k), at(0.35, theta + k + 60.0)];
        Shape::convex_polygon(points, channel, Stroke::NONE)
    };
    let notch = |k: f32| Shape::circle_filled(at(0.14, theta + k), 0.021 * d, fill);
    vec![
        Shape::circle_filled(c, 0.35 * d, channel),
        Shape::circle_filled(c, 0.2905 * d, fill),
        spoke(0.0),
        spoke(120.0),
        spoke(240.0),
        Shape::circle_filled(c, 0.175 * d, channel),
        Shape::circle_filled(c, 0.14 * d, fill),
        notch(90.0),
        notch(210.0),
        notch(330.0),
    ]
}

/// The warp curve's line width, device pixels.
const WARP_LINE_PX: f32 = 4.0;

pub fn panel() -> Option<Panel> {
    // The arcs: channel colour on a `moduleFill` track, no label or value strip. On the menu params p3 / p4 they have
    // 10 segments. Their readouts and names are the overlay's.
    let left = |id: u32| arc(id, 135.0, 90.0, true, Ink::ModuleFill).ml(1.0);
    let right = |id: u32| arc(id, 315.0, 90.0, false, Ink::ModuleFill).mr(1.0);
    Some(
        Panel::new(hstack([
            left(1).p(2).show(FREE),
            left(2).p(4).show(SYNC),
            vstack([
                toggle(3, "")
                    .p(7)
                    .value_text()
                    .value_fn(sync_text)
                    .font(FontSize::Pt10),
                button(4, "", true)
                    .p(5)
                    .icon(IconId::Play)
                    .shape(0.9)
                    .colors(PLAY)
                    .h(Size::Flex(0.8))
                    .margins(0.2, 0.0, 0.2, 0.0),
                // `Warp`: its name and curve are the overlay's.
                h_slider(5, "").p(6),
            ])
            .w(Size::Flex(0.67)),
            right(6).p(1).show(FREE),
            right(7).p(3).show(SYNC),
        ]))
        .overlay(readouts),
    )
}

/// The left arcs: 135°, 90° span, mirrored, track `moduleFill`.
#[cfg(test)]
const LEFT: Widget = Widget::Arc {
    start: 135.0,
    span: 90.0,
    mirrored: true,
    track: Ink::ModuleFill,
};
/// The right arcs: 315°, 90° span.
#[cfg(test)]
const RIGHT: Widget = Widget::Arc {
    start: 315.0,
    span: 90.0,
    mirrored: false,
    track: Ink::ModuleFill,
};
#[cfg(test)]
const ARC_H: &str =
    "f32 rounding: content height is 6.38999 U (UIHeight 8.4), the reference is 6.39";
#[cfg(test)]
const MIDDLE: [Row; 3] = [
    row(3, Widget::Toggle, "", [5.49, 0.0, 3.01, 2.13])
        .p(7)
        .value_text()
        .font(FontSize::Pt10)
        .value("Sync"),
    // Icon 3, shape 0.9; the icon in the channel colour, the two further slots `controlFill`.
    row(
        4,
        Widget::Button { round: true },
        "",
        [5.49, 2.33, 3.01, 1.72],
    )
    .p(5)
    .h(Size::Flex(0.8))
    .margins(0.2, 0.0, 0.2, 0.0)
    .icon(IconId::Play)
    .shape(0.9)
    .colors(ButtonColors {
        fill_on: Ink::ControlFill,
        fill_off: Ink::ModuleFill,
        text_on: Ink::Channel,
        text_off: Some(Ink::ControlFill),
    }),
    row(5, Widget::HSlider, "", [5.49, 4.25, 3.01, 2.14]).p(6),
];

#[cfg(test)]
pub const REFERENCE: Reference = Reference {
    size: [14.0, 6.39],
    settings: &[],
    cases: &[
        Case {
            name: "Sync off",
            rows: &[
                row(1, LEFT, "", [1.0, 0.0, 4.49, 6.38])
                    .p(2)
                    .margins(0.0, 0.0, 0.0, 1.0)
                    .slack(1.0, ARC_H),
                MIDDLE[0],
                MIDDLE[1],
                MIDDLE[2],
                row(6, RIGHT, "", [8.5, 0.0, 4.5, 6.38])
                    .p(1)
                    .margins(0.0, 1.0, 0.0, 0.0)
                    .slack(1.0, ARC_H),
            ],
            ..Case::DEFAULT
        },
        Case {
            // `Sync` (p7) > 0.5 shows the 10-step arcs, so exactly 0.5 keeps the continuous ones.
            name: "Sync at 0.5",
            params: &[(7, 0.5)],
            rows: &[
                row(1, LEFT, "", [1.0, 0.0, 4.49, 6.38])
                    .p(2)
                    .margins(0.0, 0.0, 0.0, 1.0)
                    .slack(1.0, ARC_H),
                MIDDLE[0],
                MIDDLE[1],
                MIDDLE[2],
                row(6, RIGHT, "", [8.5, 0.0, 4.5, 6.38])
                    .p(1)
                    .margins(0.0, 1.0, 0.0, 0.0)
                    .slack(1.0, ARC_H),
            ],
            ..Case::DEFAULT
        },
        Case {
            name: "Sync on",
            params: &[(7, 1.0)],
            // The 10-step arcs in the continuous ones' slots.
            rows: &[
                row(2, LEFT, "", [1.0, 0.0, 4.49, 6.38])
                    .p(4)
                    .margins(0.0, 0.0, 0.0, 1.0)
                    .slack(1.0, ARC_H),
                MIDDLE[0],
                MIDDLE[1],
                MIDDLE[2],
                row(7, RIGHT, "", [8.5, 0.0, 4.5, 6.38])
                    .p(3)
                    .margins(0.0, 1.0, 0.0, 0.0)
                    .slack(1.0, ARC_H),
            ],
            ..Case::DEFAULT
        },
    ],
};

#[cfg(test)]
mod tests {
    use eframe::egui::{self, Context, Pos2, RawInput, Shape};

    use super::*;
    use crate::metrics::{DEFAULT_UI_SCALE_INDEX, Metrics};
    use crate::params::menu_value;
    use crate::theme::theme_by_id;

    const U: f32 = 32.0;

    fn metrics() -> Metrics {
        Metrics::from_pixels_per_point(1.0, DEFAULT_UI_SCALE_INDEX)
    }

    /// The shapes the overlay draws for control `#n` in `rect` at U = 32.
    fn drawn(module: &ModuleInstance, n: u32, rect: Rect) -> Vec<Shape> {
        let ctx = Context::default();
        let (theme, m) = (theme_by_id(0), metrics());
        let cx = UiCtx {
            theme: &theme,
            m: &m,
        };
        let out = ctx.run(RawInput::default(), |ctx| {
            egui::CentralPanel::default().show(ctx, |ui| {
                let state = PanelState::default();
                readouts(ui, &cx, module, &state, &[(n, rect)], rect, Color32::RED);
            });
        });
        out.shapes.into_iter().map(|c| c.shape).collect()
    }

    /// The texts among `shapes`: text, font size and where.
    fn texts(shapes: &[Shape]) -> Vec<(String, f32, Rect)> {
        let text = |s: &Shape| match s {
            Shape::Text(t) => Some((
                t.galley.text().to_owned(),
                t.galley.job.sections[0].format.font_id.size,
                t.galley.rect.translate(t.pos.to_vec2()),
            )),
            _ => None,
        };
        shapes.iter().filter_map(text).collect()
    }

    #[test]
    fn a_spool_is_its_rim_reel_spokes_hub_hole_and_notches() {
        // The spool in the reference arc rect (4.49 × 6.38 U): d = min(w, h) = 4.49 U, centred; θ in degrees with y
        // down, P(φ) on the 0.35 d circle.
        let (channel, fill) = (Color32::RED, Color32::BLUE);
        let r = Rect::from_min_size(pos2(1.0 * U, 0.0), vec2(4.49 * U, 6.38 * U));
        let (c, d) = (r.center(), 4.49 * U);
        let circle = |s: &Shape| match s {
            Shape::Circle(k) => (k.center, k.radius, k.fill),
            _ => panic!("{s:?}"),
        };
        let triangle = |s: &Shape| match s {
            Shape::Path(p) if p.closed && p.fill == channel => p.points.clone(),
            _ => panic!("{s:?}"),
        };
        let near = |a: Pos2, b: Pos2| (a - b).length() < 1e-3;
        // How far (degrees) the angle of a point around c, y down, is from `deg`.
        let off = |p: Pos2, deg: f32| {
            ((p - c).angle().to_degrees() - deg + 180.0).rem_euclid(360.0) - 180.0
        };
        for theta in [0.0, 30.0] {
            let shapes = spool(r, theta, channel, fill);
            assert_eq!(shapes.len(), 10);
            // 1–2, 4–5: the channel disc 0.35 d, the `moduleFill` disc 0.2905 d, the channel hub 0.175 d, the
            // `moduleFill` hole 0.14 d.
            for (i, k, ink) in [
                (0, 0.35, channel),
                (1, 0.2905, fill),
                (5, 0.175, channel),
                (6, 0.14, fill),
            ] {
                let (at, radius, color) = circle(&shapes[i]);
                assert!(
                    near(at, c) && (radius - k * d).abs() < 1e-3 && color == ink,
                    "θ {theta}, #{i}"
                );
            }
            // 3: the spokes (centre, P(θ + k), P(θ + k + 60)) for k = 0, 120, 240.
            for (i, k) in [(2, 0.0), (3, 120.0), (4, 240.0)] {
                let points = triangle(&shapes[i]);
                assert_eq!(points.len(), 3);
                assert!(near(points[0], c), "θ {theta}, k {k}");
                for (p, deg) in [(points[1], theta + k), (points[2], theta + k + 60.0)] {
                    assert!(
                        ((p - c).length() - 0.35 * d).abs() < 1e-3,
                        "θ {theta}, k {k}"
                    );
                    assert!(
                        off(p, deg).abs() < 1e-2,
                        "θ {theta}, k {k}: {}",
                        off(p, deg)
                    );
                }
            }
            // 6: the notches, radius 0.021 d, centred on the 0.14 d circle at θ + 90, θ + 210, θ + 330.
            for (i, deg) in [(7, 90.0), (8, 210.0), (9, 330.0)] {
                let (at, radius, color) = circle(&shapes[i]);
                assert!(
                    ((at - c).length() - 0.14 * d).abs() < 1e-3,
                    "θ {theta}, #{i}"
                );
                assert!(off(at, theta + deg).abs() < 1e-2, "θ {theta}, #{i}");
                assert!(
                    (radius - 0.021 * d).abs() < 1e-4 && color == fill,
                    "θ {theta}, #{i}"
                );
            }
        }
        // At θ = 0 the first spoke starts at 3 o'clock and turns clockwise on screen, and the first notch is straight
        // down.
        let shapes = spool(r, 0.0, channel, fill);
        let first = triangle(&shapes[2]);
        assert!(near(first[1], c + vec2(0.35 * d, 0.0)) && first[2].y > c.y);
        assert!(near(circle(&shapes[7]).0, c + vec2(0.0, 0.14 * d)));
    }

    #[test]
    fn each_arc_draws_its_spool_at_rest_before_its_texts() {
        // Draw order: the spool in r, then the value text and the name. The Warp slider has none.
        let fill = theme_by_id(0).colors.module_fill.to_color32();
        let arc = Rect::from_min_size(pos2(8.5 * U, 0.0), vec2(4.5 * U, 6.38 * U));
        for n in [1, 2, 6, 7] {
            // Past the offscreen panel's own background.
            let shapes = drawn(&ModuleInstance::new(42, 2), n, arc);
            let shapes = &shapes[shapes
                .iter()
                .position(|s| !matches!(s, Shape::Rect(_)))
                .unwrap()..];
            assert_eq!(
                shapes[..10],
                spool(arc, 0.0, Color32::RED, fill)[..],
                "#{n}"
            );
            assert!(
                shapes[10..].iter().all(|s| matches!(s, Shape::Text(_))),
                "#{n}"
            );
        }
        let shapes = drawn(&ModuleInstance::new(42, 2), 5, arc);
        assert!(!shapes.iter().any(|s| matches!(s, Shape::Circle(_))));
    }

    #[test]
    fn a_feed_turns_both_spools_to_its_angle() {
        // θ from the engine (here 120°) turns every arc's spool alike; without a feed they stand at 0.
        let fill = theme_by_id(0).colors.module_fill.to_color32();
        let arc = Rect::from_min_size(pos2(8.5 * U, 0.0), vec2(4.5 * U, 6.38 * U));
        let ctx = Context::default();
        let (theme, m) = (theme_by_id(0), metrics());
        let cx = UiCtx {
            theme: &theme,
            m: &m,
        };
        for n in [1, 6] {
            let out = ctx.run(RawInput::default(), |ctx| {
                egui::CentralPanel::default().show(ctx, |ui| {
                    let feed = (honeydew_audio::MeterSnapshot::default(), &[120.0][..]);
                    crate::panels::post_feed(ui, 2, Some(feed));
                    let module = ModuleInstance::new(42, 2);
                    readouts(
                        ui,
                        &cx,
                        &module,
                        &PanelState::default(),
                        &[(n, arc)],
                        arc,
                        Color32::RED,
                    );
                });
            });
            let shapes: Vec<Shape> = out.shapes.into_iter().map(|c| c.shape).collect();
            let at = shapes
                .iter()
                .position(|s| !matches!(s, Shape::Rect(_)))
                .unwrap();
            assert_eq!(
                shapes[at..at + 10],
                spool(arc, 120.0, Color32::RED, fill)[..],
                "#{n}"
            );
        }
    }

    #[test]
    fn every_arc_shows_its_own_readout_and_name() {
        let pt10 = metrics().font_id(FontSize::Pt10).size;
        let mut module = ModuleInstance::new(42, 2);
        // Four different values: Start Time 2.50 s (p2), synced 3/4 (p4); Stop Time 1.00 s (p1), synced 1/2 (p3).
        module.params[2] = 0.5;
        module.params[4] = menu_value(4, 10);
        let arc = Rect::from_min_size(pos2(0.0, 0.0), vec2(4.5 * U, 6.39 * U));
        let column = arc.left() + 0.3625 * arc.width()..=arc.left() + 0.6375 * arc.width();
        for (n, value, name) in [
            (1, "2.50 s", "Start Time"),
            (2, "3/4", "Start Time"),
            (6, "1.00 s", "Stop Time"),
            (7, "1/2", "Stop Time"),
        ] {
            let texts = texts(&drawn(&module, n, arc));
            let [(v, _, at), (label, size, label_at)] = &texts[..] else {
                panic!("#{n}: {texts:?}");
            };
            assert_eq!((v.as_str(), label.as_str()), (value, name), "#{n}");
            assert!(
                column.contains(&at.left()) && column.contains(&at.right()),
                "#{n}"
            );
            assert_eq!(*size, pt10, "#{n}");
            assert!(label_at.top() >= arc.bottom() - 0.75 * U - 0.5, "#{n}");
        }
    }

    #[test]
    fn a_readout_too_long_for_its_spool_shrinks_then_loses_its_middle() {
        let mut module = ModuleInstance::new(42, 2);
        // `16 beats` on a narrow synced arc: still too wide at 6 pt, so its middle goes, keeping 5 tail characters.
        module.params[3] = 1.0;
        let narrow = Rect::from_min_size(pos2(0.0, 0.0), vec2(110.0, 120.0));
        let (value, size, at) = texts(&drawn(&module, 7, narrow)).remove(0);
        assert!(value.contains("...") && value != "16 beats", "{value}");
        assert_eq!(size, metrics().font_id_pt(6.0).size);
        assert!(at.width() <= 0.275 * narrow.width() + 0.5, "{at:?}");
    }

    #[test]
    fn the_warp_slider_has_its_name_and_curve() {
        // S = the slider's rect: `Warp` top-aligned in its bottom 0.75 U at FontSize 2, and the curve in the middle
        // half of its top 0.75 U.
        let s = Rect::from_min_size(pos2(10.0, 20.0), vec2(3.01 * U, 2.14 * U));
        let shapes = drawn(&ModuleInstance::new(42, 2), 5, s);
        let texts = texts(&shapes);
        let [(name, size, at)] = &texts[..] else {
            panic!("{texts:?}");
        };
        assert_eq!(name, "Warp");
        assert_eq!(*size, metrics().font_id(FontSize::Pt10).size);
        assert!(
            at.top() >= s.bottom() - 0.75 * U - 0.5 && (at.center().x - s.center().x).abs() < 0.5
        );
        let plot = Rect::from_min_size(
            pos2(s.left() + 0.25 * s.width(), s.top()),
            vec2(0.5 * s.width(), 0.75 * U),
        );
        let curve = shapes.iter().find(|s| matches!(s, Shape::Path(_)));
        let bounds = curve.expect("the warp curve").visual_bounding_rect();
        assert!(
            plot.expand(0.5).contains_rect(bounds),
            "{bounds:?} in {plot:?}"
        );
        // Away from the straight line (v = 0.5) the curve bends: warp > 0 towards the exponential, its middle
        // below the diagonal (lower on screen), warp < 0 towards the logarithmic, above it.
        for (v, below) in [(1.0, true), (0.75, true), (0.0, false)] {
            let mut module = ModuleInstance::new(42, 2);
            module.params[6] = v;
            let shapes = drawn(&module, 5, s);
            let Some(Shape::Path(path)) = shapes.iter().find(|s| matches!(s, Shape::Path(_)))
            else {
                panic!("v {v}: no curve");
            };
            let off = |p: &&Pos2| (p.x - plot.center().x).abs();
            let mid = path
                .points
                .iter()
                .min_by(|a, b| off(a).total_cmp(&off(b)))
                .unwrap();
            assert_eq!(mid.y > plot.center().y, below, "v {v}: {mid:?} in {plot:?}");
        }
    }
}
