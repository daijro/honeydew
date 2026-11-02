//! Knob and slider input and desktop hover.
//! The rig's control is 96 × 128 at U = 32 with a 24-point label strip: the vertical slider's track is 72 points
//! long, the horizontal one's 64, and the knob's ring is D = 96 around (148, 176).

use eframe::egui::{CursorIcon, MouseWheelUnit, ScrollArea, Sense};

use super::*;

const V_TRACK: f32 = 72.0;
const H_TRACK: f32 = 64.0;
const KNOB_CENTRE: Pos2 = pos2(148.0, 176.0);
/// 1 inch at the DPI the app is given (192 per point at any density), in points.
const INCH: f32 = 192.0;

#[test]
fn a_knob_moves_0_24_of_the_range_per_inch_dragged_up_at_any_density() {
    for ppp in [1.0, 2.0, 2.625] {
        let m = Metrics::from_pixels_per_point(ppp, DEFAULT_UI_SCALE);
        assert!(
            near(Law::Knob(1.0).delta(vec2(0.0, -INCH), &m), 0.24),
            "{ppp}"
        );
        assert!(near(Law::Knob(1.0).delta(vec2(0.0, INCH / 2.0), &m), -0.12));
        // Horizontal motion is ignored.
        assert_eq!(Law::Knob(1.0).delta(vec2(INCH, 0.0), &m), 0.0);
        // A sensitivity s scales it: the drum strip's `Start` (s = 0.5) moves 0.12 per inch.
        assert!(near(Law::Knob(0.5).delta(vec2(0.0, -INCH), &m), 0.12));
        // Android reports physical DPI independently of its logical pixels per point and UI scale.
        for scale in [75, DEFAULT_UI_SCALE, 150] {
            let m = Metrics::from_display_dpi(ppp, scale, f32::INFINITY, Some(384.0));
            let inch = m.dpi() / ppp;
            assert!(near(Law::Knob(1.0).delta(vec2(0.0, -inch), &m), 0.24));
        }
    }
    let mut rig = Rig::new(Kind::Knob, 0.25);
    rig.drag(&[vec2(0.0, -10.0), vec2(0.0, -INCH)]);
    assert!(near(rig.value, 0.49), "{}", rig.value);
    rig.drag(&[vec2(10.0, 0.0), vec2(INCH, 0.0)]);
    assert!(near(rig.value, 0.49), "{}", rig.value);
}

#[test]
fn a_slider_thumb_follows_the_pointer_one_to_one() {
    let m = metrics();
    let track = |len, horizontal| Law::Track { len, horizontal };
    assert!(near(track(72.0, false).delta(vec2(5.0, -36.0), &m), 0.5));
    assert!(near(track(64.0, true).delta(vec2(16.0, 99.0), &m), 0.25));
    assert_eq!(track(0.0, true).delta(vec2(16.0, 0.0), &m), 0.0);

    let mut rig = Rig::new(Kind::VSlider, 0.25);
    rig.drag(&[vec2(0.0, -10.0), vec2(0.0, -V_TRACK / 2.0)]);
    assert!(near(rig.value, 0.75), "{}", rig.value);
    let mut rig = Rig::new(Kind::HSlider, 0.25);
    rig.drag(&[vec2(10.0, 0.0), vec2(H_TRACK / 4.0, 0.0)]);
    assert!(near(rig.value, 0.5), "{}", rig.value);
}

/// The slider thumb's outer disc: the largest circle drawn.
fn thumb(shapes: &[egui::epaint::ClippedShape]) -> Pos2 {
    let all = circles(shapes);
    all.iter()
        .max_by(|a, b| a.2.total_cmp(&b.2))
        .expect("thumb")
        .1
}

#[test]
fn touching_never_jumps_the_value_and_the_thumb_moves_with_the_pointer() {
    for kind in [Kind::Knob, Kind::VSlider] {
        let mut rig = Rig::new(kind, 0.2);
        // Pressed at the top of the body, far from the thumb at 0.2: nothing moves.
        let top = pos2(148.0, 141.0);
        rig.frame(vec![Event::PointerMoved(top)]);
        rig.button(top, PointerButton::Primary, true);
        rig.frame(vec![]);
        assert_eq!(rig.value, 0.2, "{kind:?}");
        rig.button(top, PointerButton::Primary, false);
        assert_eq!(rig.value, 0.2, "{kind:?}");
        assert!(!rig.changed);
    }
    let mut rig = Rig::new(Kind::VSlider, 0.2);
    let before = thumb(&rig.shapes);
    let top = pos2(148.0, 141.0);
    rig.frame(vec![Event::PointerMoved(top)]);
    rig.button(top, PointerButton::Primary, true);
    for dy in [-6.0, -12.0, -18.0] {
        rig.frame(vec![Event::PointerMoved(top + vec2(0.0, dy))]);
    }
    assert!(near(rig.value, 0.2 + 18.0 / V_TRACK), "{}", rig.value);
    let after = thumb(&rig.shapes);
    assert!(
        (after - before - vec2(0.0, -18.0)).length() < 1e-3,
        "{before:?} → {after:?}"
    );
}

#[test]
fn ctrl_or_cmd_is_fine_mode_at_a_quarter_and_shift_is_not() {
    for (modifiers, rate) in [
        (Modifiers::COMMAND, 0.25),
        (Modifiers::SHIFT, 1.0),
        (Modifiers::NONE, 1.0),
    ] {
        let mut rig = Rig::new(Kind::Knob, 0.25);
        rig.modifiers = modifiers;
        rig.drag(&[vec2(0.0, -10.0), vec2(0.0, -INCH)]);
        assert!(
            near(rig.value, 0.25 + 0.24 * rate),
            "{modifiers:?}: {}",
            rig.value
        );
        let mut rig = Rig::new(Kind::VSlider, 0.25);
        rig.modifiers = modifiers;
        rig.drag(&[vec2(0.0, -10.0), vec2(0.0, -V_TRACK / 2.0)]);
        assert!(
            near(rig.value, 0.25 + 0.5 * rate),
            "{modifiers:?}: {}",
            rig.value
        );
    }
}

#[test]
fn drags_clamp_to_the_range() {
    assert_eq!(drag(0.9, 0.5, false), 1.0);
    assert_eq!(drag(0.1, -0.5, false), 0.0);
    assert!(near(drag(0.5, 0.4, true), 0.6));
}

/// A touch-down on the control at time `at`, released a frame later.
fn tap_at(rig: &mut Rig, at: f64) {
    rig.time = at - 1.0 / 60.0;
    rig.button(RECT.center(), PointerButton::Primary, true);
}

fn release(rig: &mut Rig) {
    rig.button(RECT.center(), PointerButton::Primary, false);
}

#[test]
fn a_second_touch_down_within_0_3_s_resets_on_the_touch_down() {
    for kind in [Kind::Knob, Kind::VSlider, Kind::HSlider] {
        for (gap, resets) in [(0.29, true), (0.31, false)] {
            let mut rig = Rig::new(kind, 0.9);
            rig.default = 0.25;
            rig.frame(vec![Event::PointerMoved(RECT.center())]);
            tap_at(&mut rig, 1.0);
            release(&mut rig);
            // A single tap never steps the value.
            assert_eq!(rig.value, 0.9);
            tap_at(&mut rig, 1.0 + gap);
            // Already reset while the finger is down.
            assert_eq!(rig.value, if resets { 0.25 } else { 0.9 }, "{kind:?} {gap}");
            assert_eq!(rig.changed, resets);
            release(&mut rig);
            assert_eq!(rig.value, if resets { 0.25 } else { 0.9 }, "{kind:?} {gap}");
        }
    }
    // The window runs touch-down to touch-down: a first tap held 0.25 s still counts.
    let mut rig = Rig::new(Kind::Knob, 0.9);
    rig.default = 0.25;
    rig.frame(vec![Event::PointerMoved(RECT.center())]);
    tap_at(&mut rig, 1.0);
    rig.time = 1.25 - 1.0 / 60.0;
    release(&mut rig);
    tap_at(&mut rig, 1.29);
    assert_eq!(rig.value, 0.25);
}

#[test]
fn a_secondary_press_is_no_tap() {
    let mut rig = Rig::new(Kind::Knob, 0.9);
    rig.default = 0.25;
    rig.frame(vec![Event::PointerMoved(RECT.center())]);
    rig.button(RECT.center(), PointerButton::Secondary, true);
    rig.button(RECT.center(), PointerButton::Secondary, false);
    rig.frame(vec![Event::PointerMoved(RECT.center())]);
    rig.button(RECT.center(), PointerButton::Primary, true);
    assert_eq!(rig.value, 0.9);
}

/// The knob's outer ring radius in the last frame: its first channel-colour circle at the centre.
fn ring_radius(rig: &Rig) -> f32 {
    circles(&rig.shapes)
        .into_iter()
        .find(|c| c.0 == Color32::RED && (c.1 - KNOB_CENTRE).length() < 1e-3)
        .expect("ring")
        .2
}

fn repainting(rig: &Rig) -> bool {
    rig.ctx.has_requested_repaint()
}

#[test]
fn a_held_knob_grows_then_wobbles_back_on_release() {
    let t = metrics().t();
    // h = 128 px at ppp 1; D = 96.
    let outer = |s: f32| knob::ring(KNOB_CENTRE, 96.0 * s, 0.3, false, t).outer;
    let mut rig = Rig::new(Kind::Knob, 0.3);
    rig.default = 0.0;
    rig.frame(vec![Event::PointerMoved(KNOB_CENTRE)]);
    rig.button(KNOB_CENTRE, PointerButton::Primary, true);
    // a ← 0.6 a every frame from 1, the touch-down frame included: it shows a = 0.6. Floored at 0.01.
    let mut a: f32 = 0.6;
    for n in 0..15 {
        let s = 1.0 + (1.0 - a.max(0.01)) * 10.0 / 128.0;
        assert!(
            (ring_radius(&rig) - outer(s)).abs() < 1e-3,
            "held frame {n}"
        );
        rig.frame(vec![]);
        a *= 0.6;
    }
    rig.button(KNOB_CENTRE, PointerButton::Primary, false);
    // The same from the release frame on.
    let mut a: f32 = 0.6;
    let mut wobbled = false;
    for n in 0..12 {
        let f = if a >= 0.01 {
            (1.0 - a) * ((1.0 - a) * std::f32::consts::TAU).sin()
        } else {
            0.0
        };
        wobbled |= f < 0.0;
        assert!(
            (ring_radius(&rig) - outer(1.0 + f * 10.0 / 128.0)).abs() < 1e-3,
            "released frame {n}"
        );
        if a >= 0.01 {
            assert!(repainting(&rig), "released frame {n}");
        }
        rig.frame(vec![]);
        a *= 0.6;
    }
    assert!(wobbled, "the release dips below rest");
    // Settled: no more repaints asked for.
    rig.frame(vec![]);
    rig.frame(vec![]);
    assert!(!repainting(&rig));
}

#[test]
fn the_knob_value_text_updates_live_and_nothing_pops_up() {
    let mut rig = Rig::new(Kind::Knob, 0.25);
    let start = RECT.center();
    rig.frame(vec![Event::PointerMoved(start)]);
    rig.button(start, PointerButton::Primary, true);
    rig.frame(vec![Event::PointerMoved(start + vec2(0.0, -10.0))]);
    rig.frame(vec![Event::PointerMoved(start + vec2(0.0, -INCH))]);
    let shown: Vec<String> = texts(&rig.shapes).into_iter().map(|t| t.0).collect();
    // Only the label and the live value: no popup.
    assert_eq!(shown, ["Label", "0.49"]);
}

// Desktop hover.

/// A frame with the pointer at `at` and no button down.
fn hover(rig: &mut Rig, at: Pos2) {
    rig.frame(vec![Event::PointerMoved(at)]);
    rig.frame(vec![]);
}

#[test]
fn a_hovered_button_fill_is_1_15_times_brighter() {
    let colors = theme_by_id(0).colors;
    let fill = |rig: &Rig| circles(&rig.shapes)[0].0;
    let mut rig = Rig::new(Kind::Button, 0.0);
    assert_eq!(fill(&rig), colors.menu_fill.to_color32());
    hover(&mut rig, RECT.center());
    assert_eq!(fill(&rig), pressed(colors.menu_fill.to_color32()));
    // Only the fill: the label keeps its colour.
    let label = texts(&rig.shapes)[0].1;
    assert_eq!(label, colors.text.to_color32());
    // Pressed as well: no brighter than pressed.
    rig.button(RECT.center(), PointerButton::Primary, true);
    assert_eq!(fill(&rig), pressed(colors.menu_fill.to_color32()));
    // Off the button: back to rest.
    rig.button(RECT.center(), PointerButton::Primary, false);
    hover(&mut rig, pos2(500.0, 500.0));
    assert_eq!(fill(&rig), colors.menu_fill.to_color32());
}

#[test]
fn the_hover_brightening_clamps_and_keeps_alpha() {
    assert_eq!(
        pressed(Color32::from_rgb(240, 100, 0)),
        Color32::from_rgb(255, 115, 0)
    );
    assert_eq!(
        pressed(Color32::from_rgba_unmultiplied(240, 100, 0, 200)).a(),
        200
    );
}

#[test]
fn a_hovered_slider_shows_its_track_grown_2_px_at_0_4() {
    for (kind, track) in [
        (
            Kind::VSlider,
            Rect::from_x_y_ranges(145.0..=151.0, 140.0..=212.0),
        ),
        (
            Kind::HSlider,
            Rect::from_x_y_ranges(116.0..=180.0, 173.0..=179.0),
        ),
    ] {
        let mut rig = Rig::new(kind, 0.5);
        let halo = (Color32::RED.gamma_multiply(0.4), track.expand(2.0));
        assert!(!rects(&rig.shapes).contains(&halo), "{kind:?} at rest");
        hover(&mut rig, RECT.center() + vec2(30.0, 30.0));
        assert!(
            rects(&rig.shapes).contains(&halo),
            "{kind:?}: {:?}",
            rects(&rig.shapes)
        );
    }
}

#[test]
fn a_hovered_knob_tints_its_face_and_grows_its_dot_by_5_percent() {
    let t = metrics().t();
    let r = knob::ring(KNOB_CENTRE, 96.0, 0.3, false, t);
    let module_fill = theme_by_id(0).colors.module_fill.to_color32();
    let mut rig = Rig::new(Kind::Knob, 0.3);
    rig.default = 0.0;
    rig.frame(vec![]);
    let at_rest = circles(&rig.shapes);
    hover(&mut rig, KNOB_CENTRE + vec2(0.0, 40.0));
    let want = [
        (Color32::RED, KNOB_CENTRE, r.outer),
        (module_fill, KNOB_CENTRE, r.inner),
        (Color32::RED.gamma_multiply(0.05), KNOB_CENTRE, r.inner),
        (module_fill, r.dot, 1.05 * r.halo_r),
        (Color32::RED, r.dot, 1.05 * r.dot_r),
    ];
    let got = circles(&rig.shapes);
    assert_eq!(got.len(), want.len(), "{got:?}");
    for (g, w) in got.iter().zip(&want) {
        assert!(
            g.0 == w.0 && (g.1 - w.1).length() < 1e-3 && (g.2 - w.2).abs() < 1e-3,
            "{g:?} vs {w:?}"
        );
    }
    hover(&mut rig, pos2(500.0, 500.0));
    assert_eq!(circles(&rig.shapes), at_rest);
}

#[test]
fn disabled_controls_take_no_hover() {
    for kind in [Kind::Knob, Kind::VSlider, Kind::Button] {
        let mut rig = Rig::new(kind, 0.5);
        rig.disabled = true;
        rig.frame(vec![]);
        let rest = (circles(&rig.shapes), rects(&rig.shapes));
        hover(&mut rig, RECT.center());
        assert_eq!((circles(&rig.shapes), rects(&rig.shapes)), rest, "{kind:?}");
    }
}

/// How many knob tints and lit button fills [`lit_with_pointer_at`] painted, with no press.
fn lit(at: Pos2) -> (usize, usize) {
    let (tints, fills, _) = lit_with_pointer_at(at, false);
    (tints, fills)
}

/// Two knobs side by side sharing an edge, and a button over the right one's lower part. Returns how many knob
/// tints and lit button fills the frame with the pointer at `at` painted, and the cursor it asked for.
fn lit_with_pointer_at(at: Pos2, press: bool) -> (usize, usize, CursorIcon) {
    let ctx = Context::default();
    let (theme, m) = (theme_by_id(0), metrics());
    let cx = UiCtx {
        theme: &theme,
        m: &m,
    };
    let left = Rect::from_min_size(pos2(100.0, 100.0), vec2(96.0, 128.0));
    let right = left.translate(vec2(96.0, 0.0));
    let under = Rect::from_min_size(pos2(196.0, 200.0), vec2(96.0, 28.0));
    let mut out = None;
    let mut time = 0.0;
    let mut events = vec![vec![Event::PointerMoved(at)], vec![]];
    if press {
        events.push(vec![Event::PointerButton {
            pos: at,
            button: PointerButton::Primary,
            pressed: true,
            modifiers: Modifiers::NONE,
        }]);
        events.push(vec![Event::PointerMoved(at + vec2(0.0, -20.0))]);
    }
    for events in events {
        time += 1.0 / 60.0;
        let raw = RawInput {
            screen_rect: Some(SCREEN),
            time: Some(time),
            events,
            ..Default::default()
        };
        let output = ctx.run(raw, |ctx| {
            egui::CentralPanel::default().show(ctx, |ui| {
                for (i, rect) in [left, right].into_iter().enumerate() {
                    let c = Control::new(Id::new(("k", i)), rect, "", Color32::RED, 0.0);
                    knob(ui, &cx, &c, &mut 0.3, |v| format!("{v:.2}"));
                }
                let b = Button::new(Id::new("b"), under, "", false, &theme.colors);
                button(ui, &cx, &b, false);
            });
        });
        out = Some(output);
    }
    let output = out.unwrap();
    let tints = faint(&output.shapes, 0.05);
    let lit = pressed(theme.colors.menu_fill.to_color32());
    let fills = (output.shapes.iter())
        .filter(|c| matches!(&c.shape, egui::Shape::Rect(r) if r.fill == lit))
        .count();
    (tints, fills, output.platform_output.cursor_icon)
}

/// The circles painted in the channel colour at `alpha`.
fn faint(shapes: &[egui::epaint::ClippedShape], alpha: f32) -> usize {
    let ink = Color32::RED.gamma_multiply(alpha);
    (circles(shapes).into_iter()).filter(|c| c.0 == ink).count()
}

#[test]
fn exactly_one_control_is_hovered_and_only_under_the_pointer() {
    // On the shared edge: one knob.
    assert_eq!(lit(pos2(196.0, 150.0)), (1, 0));
    // Over the button on top of the right knob: the button alone.
    assert_eq!(lit(pos2(240.0, 210.0)), (0, 1));
    // Just outside the knobs, inside egui's interact radius: nothing.
    assert_eq!(lit(pos2(150.0, 231.0)), (0, 0));
    assert_eq!(lit(pos2(97.0, 150.0)), (0, 0));
}

#[test]
fn only_a_clickable_control_under_the_mouse_changes_the_cursor() {
    // Over a knob, held or not: the default cursor; over the button: the clickable one, until a press drags off it.
    for (at, press, cursor) in [
        (pos2(150.0, 150.0), false, CursorIcon::Default),
        (pos2(150.0, 150.0), true, CursorIcon::Default),
        (pos2(240.0, 210.0), false, CursorIcon::PointingHand),
        (pos2(240.0, 210.0), true, CursorIcon::Default),
    ] {
        assert_eq!(lit_with_pointer_at(at, press).2, cursor, "{at:?} {press}");
    }
}

#[test]
fn the_wheel_passes_through_knobs_and_sliders_to_the_scroller() {
    let (theme, m) = (theme_by_id(0), metrics());
    let cx = UiCtx {
        theme: &theme,
        m: &m,
    };
    for kind in [Kind::Knob, Kind::VSlider, Kind::HSlider] {
        let ctx = Context::default();
        let mut value = 0.5;
        let mut offset = 0.0;
        let mut time = 0.0;
        let over = pos2(48.0, 64.0);
        for frame in 0..30 {
            time += 1.0 / 60.0;
            let events = match frame {
                0 => vec![Event::PointerMoved(over)],
                1 => vec![Event::MouseWheel {
                    unit: MouseWheelUnit::Point,
                    delta: vec2(0.0, -60.0),
                    modifiers: Modifiers::NONE,
                }],
                _ => vec![],
            };
            let raw = RawInput {
                screen_rect: Some(SCREEN),
                time: Some(time),
                events,
                ..Default::default()
            };
            let _ = ctx.run(raw, |ctx| {
                egui::CentralPanel::default()
                    .frame(egui::Frame::NONE)
                    .show(ctx, |ui| {
                        let out = ScrollArea::vertical().max_height(300.0).show(ui, |ui| {
                            let (rect, _) =
                                ui.allocate_exact_size(vec2(96.0, 128.0), Sense::hover());
                            let c = Control::new(Id::new("w"), rect, "Label", Color32::RED, 0.5);
                            let display = |v: f32| format!("{v:.2}");
                            match kind {
                                Kind::Knob => knob(ui, &cx, &c, &mut value, display),
                                Kind::VSlider => v_slider(ui, &cx, &c, &mut value, display),
                                _ => h_slider(ui, &cx, &c, &mut value, display),
                            };
                            ui.allocate_space(vec2(96.0, 2000.0));
                        });
                        offset = out.state.offset.y;
                    });
            });
        }
        assert_eq!(value, 0.5, "{kind:?}");
        assert!(offset > 30.0, "{kind:?}: the scroller moved {offset}");
    }
}

#[test]
fn a_key_step_is_0_05_or_0_0125_fine_kept_in_range_and_one_item_on_a_control_with_steps() {
    use crate::ui::focus::Step;
    let (up, down) = (
        Step {
            up: true,
            fine: false,
        },
        Step {
            up: false,
            fine: false,
        },
    );
    let (fine_up, fine_down) = (
        Step {
            up: true,
            fine: true,
        },
        Step {
            up: false,
            fine: true,
        },
    );
    assert!(near(stepped(0.5, up, None, None), 0.55));
    assert!(near(stepped(0.5, down, None, None), 0.45));
    assert!(near(stepped(0.5, fine_up, None, None), 0.5125));
    assert!(near(stepped(0.5, fine_down, None, None), 0.4875));
    assert_eq!(stepped(0.98, up, None, None), 1.0);
    assert_eq!(stepped(0.01, fine_down, None, None), 0.0);
    // n = 5: 1/(n − 1) = 0.25 a step, fine or not, from the item the value reads; held at the ends.
    assert_eq!(stepped(0.5, up, Some(5), None), 0.75);
    assert_eq!(stepped(0.5, fine_down, Some(5), None), 0.25);
    assert_eq!(stepped(0.6, fine_up, Some(5), None), 0.75);
    assert_eq!(stepped(1.0, up, Some(5), None), 1.0);
    assert_eq!(stepped(0.0, down, Some(5), None), 0.0);
    assert_eq!(stepped(0.0, up, Some(1), None), 0.0);
    // A `MenuBy` law: the item it reads, here trunc(4v), is the one stepped from.
    let trunc: fn(f32) -> usize = |v| (4.0 * v) as usize;
    assert_eq!(stepped(0.74, up, Some(5), Some(trunc)), 0.75);
}
