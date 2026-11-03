use super::*;

// Buttons.

#[test]
fn a_button_reports_primary_clicks_only_and_nothing_when_disabled() {
    let mut rig = Rig::new(Kind::Button, 0.0);
    rig.click(RECT.center());
    assert!(rig.clicked);
    rig.click_with(RECT.center(), PointerButton::Secondary);
    assert!(!rig.clicked);
    rig.disabled = true;
    rig.click(RECT.center());
    assert!(!rig.clicked);
}

#[test]
fn a_param_bound_button_opens_the_parameter_menu_without_reset() {
    // Buttons bound to a parameter show the parameter menu's items; others show nothing.
    let mut rig = Rig::new(Kind::Button, 0.0);
    rig.click_with(RECT.center(), PointerButton::Secondary);
    rig.frame(vec![]);
    assert!(rig.menu_rect().is_none());
    rig.style = |b| Button {
        menu: ParamMenu::Bound,
        ..b
    };
    rig.click_with(RECT.center(), PointerButton::Secondary);
    rig.frame(vec![]);
    assert!(rig.menu_rect().is_some());
    assert!(!rig.clicked);
    let texts: Vec<String> = texts(&rig.shapes).into_iter().map(|t| t.0).collect();
    assert!(texts.iter().any(|t| t == "Link to MIDI CC"), "{texts:?}");
    assert!(texts.iter().any(|t| t == "Add automation track"));
    assert!(!texts.iter().any(|t| t == "Reset"));
}

#[test]
fn a_round_button_is_a_disc_with_the_on_and_off_fills_and_labels() {
    let colors = theme_by_id(0).colors;
    let (text, menu_fill) = (colors.text.to_color32(), colors.menu_fill.to_color32());
    // Default colours (the Pitcher note buttons): on = `text` disc with a `menuFill` label, off = the reverse.
    for (on, fill, label) in [(true, text, menu_fill), (false, menu_fill, text)] {
        let mut rig = Rig::new(Kind::Button, 0.0);
        rig.on = on;
        rig.style = |b| Button { shape: 0.7, ..b };
        rig.frame(vec![]);
        let discs = circles(&rig.shapes);
        assert_eq!(discs.len(), 1);
        // 0.7 × half the short side (96).
        assert_eq!(discs[0], (fill, RECT.center(), 0.7 * 48.0));
        let shown = texts(&rig.shapes);
        assert_eq!(shown.len(), 1);
        assert_eq!((shown[0].0.as_str(), shown[0].1), ("Mono", label));
    }
}

#[test]
fn a_halo_is_a_larger_disc_behind_the_button_by_the_pad_in_device_pixels() {
    let mut rig = Rig::new(Kind::Button, 0.0);
    rig.style = |b| Button {
        shape: 0.7,
        halo: Some((Color32::GREEN, 4.0)),
        ..b
    };
    rig.frame(vec![]);
    let discs = circles(&rig.shapes);
    // The halo first (ppp 1: 4 px = 4 points), then the button over it.
    assert_eq!(discs.len(), 2);
    assert_eq!(discs[0], (Color32::GREEN, RECT.center(), 0.7 * 48.0 + 4.0));
    assert_eq!(discs[1].2, 0.7 * 48.0);
}

#[test]
fn the_halo_pad_is_in_device_pixels_at_any_density() {
    // At 2 pixels per point a 4 px pad is 2 points.
    let theme = theme_by_id(0);
    let m = Metrics::from_pixels_per_point(2.0, DEFAULT_UI_SCALE);
    let cx = UiCtx {
        theme: &theme,
        m: &m,
    };
    let b = Button {
        shape: 0.7,
        halo: Some((Color32::GREEN, 4.0)),
        ..Button::new(Id::new("b"), RECT, "", true, &theme.colors)
    };
    let output = Context::default().run(
        RawInput {
            screen_rect: Some(SCREEN),
            viewports: [(
                egui::ViewportId::ROOT,
                egui::ViewportInfo {
                    native_pixels_per_point: Some(2.0),
                    ..Default::default()
                },
            )]
            .into_iter()
            .collect(),
            ..Default::default()
        },
        |ctx| {
            egui::CentralPanel::default().show(ctx, |ui| {
                button(ui, &cx, &b, true);
            });
        },
    );
    let discs = circles(&output.shapes);
    assert_eq!(discs.len(), 2);
    assert_eq!(discs[0], (Color32::GREEN, RECT.center(), 0.7 * 48.0 + 2.0));
}

#[test]
fn a_rect_button_fills_its_rect_and_grows_its_halo_by_half_the_pad() {
    let mut rig = Rig::new(Kind::Button, 0.0);
    rig.style = |b| Button {
        round: false,
        fill_on: Color32::BLUE,
        fill_off: Color32::RED,
        halo: Some((Color32::GREEN, 2.0)),
        ..b
    };
    for (on, fill) in [(true, Color32::BLUE), (false, Color32::RED)] {
        rig.on = on;
        rig.frame(vec![]);
        let r = rects(&rig.shapes);
        // The halo is the rect grown by pad/2 on every side; the fill is the rect itself.
        assert!(r.contains(&(Color32::GREEN, RECT.expand(1.0))), "{r:?}");
        assert!(r.contains(&(fill, RECT)), "{r:?}");
        assert!(circles(&rig.shapes).is_empty());
    }
}

#[test]
fn a_pressed_button_draws_every_colour_brighter() {
    // While pressed, halo, fill, icon and text are drawn with RGB × 1.15, clamped, alpha unchanged.
    let mut rig = Rig::new(Kind::Button, 0.0);
    rig.style = |b| Button {
        fill_off: Color32::from_rgb(100, 120, 200),
        text_off: Some(Color32::from_rgb(40, 60, 80)),
        halo: Some((Color32::from_rgb(10, 200, 250), 4.0)),
        icon: Some(IconId::Play),
        ..b
    };
    let fills = |rig: &Rig| circles(&rig.shapes).iter().map(|c| c.0).collect::<Vec<_>>();
    let text = |rig: &Rig| texts(&rig.shapes).iter().find(|t| t.0 == "Mono").unwrap().1;
    let icon = |rig: &Rig, ink: Color32| !painted_in(&rig.shapes, ink).is_empty();
    rig.frame(vec![]);
    let rest = [
        Color32::from_rgb(10, 200, 250),
        Color32::from_rgb(100, 120, 200),
    ];
    assert_eq!(fills(&rig), rest);
    assert_eq!(text(&rig), Color32::from_rgb(40, 60, 80));
    let pressed_ink = Color32::from_rgb(46, 69, 92);
    assert!(!icon(&rig, pressed_ink));
    rig.frame(vec![Event::PointerMoved(RECT.center())]);
    rig.button(RECT.center(), PointerButton::Primary, true);
    rig.frame(vec![]);
    assert_eq!(
        fills(&rig),
        [
            Color32::from_rgb(12, 230, 255),
            Color32::from_rgb(115, 138, 230)
        ]
    );
    assert_eq!(text(&rig), Color32::from_rgb(46, 69, 92));
    assert!(icon(&rig, pressed_ink));
    // Released and off the button (or the hover lights the fill): back to rest. A translucent colour keeps its alpha.
    rig.button(RECT.center(), PointerButton::Primary, false);
    rig.frame(vec![Event::PointerMoved(pos2(700.0, 500.0))]);
    assert_eq!(fills(&rig), rest);
    // (Color32 stores premultiplied channels, so a translucent colour's RGB comes back within one step.)
    let half = super::super::button::pressed(Color32::from_rgba_unmultiplied(100, 120, 200, 128));
    assert_eq!(half.a(), 128);
    let rgb = half.to_srgba_unmultiplied();
    for (got, want) in rgb.iter().zip([115, 138, 230]) {
        assert!(got.abs_diff(want) <= 1, "{rgb:?}");
    }
}

#[test]
fn a_press_held_over_a_focus_loss_is_not_drawn_pressed_until_a_new_press() {
    // egui keeps the button down when the focus goes, and the release may never come (`primary_down_live`). The
    // label tells: a press brightens it, the hover (the pointer stays on the button) only the fill.
    let mut rig = Rig::new(Kind::Button, 0.0);
    rig.style = |b| Button {
        text_off: Some(Color32::from_rgb(100, 120, 200)),
        ..b
    };
    let fill = |rig: &Rig| texts(&rig.shapes)[0].1;
    let (rest, lit) = (
        Color32::from_rgb(100, 120, 200),
        Color32::from_rgb(115, 138, 230),
    );
    rig.frame(vec![Event::PointerMoved(RECT.center())]);
    rig.button(RECT.center(), PointerButton::Primary, true);
    assert_eq!(fill(&rig), lit);
    rig.frame(vec![Event::WindowFocused(false)]);
    assert_eq!(fill(&rig), rest);
    rig.frame(vec![Event::WindowFocused(true)]);
    rig.frame(vec![]);
    assert_eq!(fill(&rig), rest);
    // A new press is live again.
    rig.button(RECT.center(), PointerButton::Primary, false);
    rig.button(RECT.center(), PointerButton::Primary, true);
    assert_eq!(fill(&rig), lit);
}

#[test]
fn the_off_label_dims_the_on_label_unless_an_off_colour_is_set() {
    let mut rig = Rig::new(Kind::Button, 0.0);
    rig.style = |b| Button {
        text_on: Color32::from_rgb(200, 100, 40),
        text_off: None,
        ..b
    };
    rig.frame(vec![]);
    assert_eq!(
        texts(&rig.shapes)[0].1,
        Color32::from_rgb(200, 100, 40).gamma_multiply(0.25)
    );
    rig.on = true;
    rig.frame(vec![]);
    assert_eq!(texts(&rig.shapes)[0].1, Color32::from_rgb(200, 100, 40));
}

#[test]
fn an_icon_button_puts_the_icon_above_and_the_text_in_the_lower_third() {
    let mut rig = Rig::new(Kind::Button, 0.0);
    rig.style = |b| Button {
        icon: Some(IconId::Record),
        text_on: Color32::GREEN,
        text_off: Some(Color32::BLUE),
        ..b
    };
    let upper = Rect::from_min_max(RECT.min, pos2(RECT.right(), RECT.bottom() - 128.0 / 3.0));
    // The icon takes the label's colour in each state.
    for (on, ink, other) in [
        (true, Color32::GREEN, Color32::BLUE),
        (false, Color32::BLUE, Color32::GREEN),
    ] {
        rig.on = on;
        rig.frame(vec![]);
        // The label is centred in the lower third of the 128-point rect.
        let lower = pos2(RECT.center().x, RECT.bottom() - 128.0 / 6.0);
        assert_plain(&text_boxes(&rig.shapes)[0], 20.0, lower);
        let painted = |ink: Color32| painted_in(&rig.shapes, ink);
        assert!(painted(other).is_empty(), "{on}");
        let icon = painted(ink);
        assert!(!icon.is_empty(), "{on}");
        // Inside the rect's upper part.
        for r in icon {
            assert!(upper.expand(1.0).contains_rect(r), "{r:?} vs {upper:?}");
        }
    }
}

#[test]
fn list_items_spinner_values_and_button_labels_are_drawn_in_full() {
    // List items are plain text in panels: in full, centred on their 48-point cells.
    const LONG: &str = "An item wider than its cell";
    let mut rig = Rig::new(Kind::ListIndex(Bar::Bottom), 0.0);
    rig.items = &[LONG, LONG];
    rig.frame(vec![]);
    let boxes = text_boxes(&rig.shapes);
    assert_eq!(boxes.len(), 2);
    let t = metrics().t();
    for (i, b) in boxes.iter().enumerate() {
        assert_eq!(b.0, LONG);
        assert!(b.1.width() > 48.0, "{b:?}");
        let cell = pos2(
            RECT.left() + 48.0 * (i as f32 + 0.5),
            (RECT.top() + RECT.bottom() - 3.0 * t) / 2.0,
        );
        assert_plain(b, 20.0, cell);
    }
    // Spinner values and button labels are plain text: in full, centred, not clipped to the control.
    let mut rig = Rig::new(Kind::Spinner, 0.0);
    rig.items = &["An item wider than 3 U"];
    rig.frame(vec![]);
    let value = text_boxes(&rig.shapes).pop().unwrap();
    assert!(value.1.width() > RECT.width());
    assert_plain(&value, 20.0, VALUE);
    let mut rig = Rig::new(Kind::Button, 0.0);
    rig.style = |b| Button {
        text: "A label wider than 3 U",
        ..b
    };
    rig.frame(vec![]);
    let label = text_boxes(&rig.shapes).pop().unwrap();
    assert!(label.1.width() > RECT.width());
    assert_plain(&label, 20.0, RECT.center());
}

// Static widgets.

#[test]
fn a_text_label_anchors_its_text_in_the_rect_and_does_not_clip_it() {
    use egui::Align2;
    let theme = theme_by_id(0);
    let m = metrics();
    let cx = UiCtx {
        theme: &theme,
        m: &m,
    };
    let ctx = Context::default();
    let rect = Rect::from_min_size(pos2(350.0, 100.0), vec2(96.0, 64.0));
    for (align, left, top) in [
        (Align2::LEFT_CENTER, true, false),
        (Align2::RIGHT_TOP, false, true),
    ] {
        let output = ctx.run(
            RawInput {
                screen_rect: Some(SCREEN),
                ..Default::default()
            },
            |ctx| {
                egui::CentralPanel::default().show(ctx, |ui| {
                    text_label(
                        ui,
                        &cx,
                        rect,
                        "A rhythm label wider than 3 U",
                        FontSize::Pt10,
                        align,
                        Color32::GOLD,
                    );
                });
            },
        );
        let found: Vec<_> = output
            .shapes
            .iter()
            .filter_map(|c| match &c.shape {
                egui::Shape::Text(t) => Some((t.clone(), c.clip_rect)),
                _ => None,
            })
            .collect();
        assert_eq!(found.len(), 1);
        let (shape, clip) = &found[0];
        assert_eq!(shape.galley.text(), "A rhythm label wider than 3 U");
        assert_eq!(shape.fallback_color, Color32::GOLD);
        assert_eq!(shape.galley.job.sections[0].format.font_id.size, 20.0);
        // Plain text: in full, overflowing the rect on the side away from the anchor, and not clipped to it.
        let text = Rect::from_min_size(shape.pos, shape.galley.size());
        assert!(!shape.galley.elided && text.width() > rect.width());
        assert!(clip.contains_rect(text), "{text:?} in {clip:?}");
        assert!(if left {
            (text.left() - rect.left()).abs() < 1e-3
        } else {
            (text.right() - rect.right()).abs() < 1e-3
        });
        assert!(if top {
            (text.top() - rect.top()).abs() < 1e-3
        } else {
            (text.center().y - rect.center().y).abs() < 1.0
        });
    }
}

#[test]
fn the_display_placeholder_is_a_control_fill_rect_with_a_one_pixel_background_outline() {
    let theme = theme_by_id(0);
    let m = Metrics::from_pixels_per_point(2.0, DEFAULT_UI_SCALE);
    let cx = UiCtx {
        theme: &theme,
        m: &m,
    };
    let ctx = Context::default();
    let rect = Rect::from_min_size(pos2(100.0, 100.0), vec2(200.0, 100.0));
    let output = ctx.run(
        RawInput {
            screen_rect: Some(SCREEN),
            viewports: [(
                egui::ViewportId::ROOT,
                egui::ViewportInfo {
                    native_pixels_per_point: Some(2.0),
                    ..Default::default()
                },
            )]
            .into_iter()
            .collect(),
            ..Default::default()
        },
        |ctx| {
            egui::CentralPanel::default().show(ctx, |ui| display_area(ui, &cx, rect));
        },
    );
    let ours: Vec<_> = output
        .shapes
        .iter()
        .filter_map(|c| match &c.shape {
            egui::Shape::Rect(r) if rect.contains_rect(r.rect) => Some(r.clone()),
            _ => None,
        })
        .collect();
    assert_eq!(ours.len(), 1);
    let r = &ours[0];
    assert_eq!(r.rect, rect);
    assert_eq!(r.fill, theme.colors.control_fill.to_color32());
    // One device pixel is half a point at 2 pixels per point, drawn inside the rect.
    assert_eq!(r.stroke.width, 0.5);
    assert_eq!(r.stroke.color, theme.colors.background.to_color32());
    assert_eq!(r.stroke_kind, egui::StrokeKind::Inside);
}

/// The Tuner's Input button: round, shape 0.7, halo 4 px, icon 22 (here the `AudioRecord` glyph), in a 1.75 U ×
/// 2.15 U rect. Ink colours differ from every fill and the halo.
fn tuner_input(b: Button) -> Button {
    Button {
        rect: Rect::from_min_size(RECT.min, vec2(1.75 * U, 2.15 * U)),
        text: "",
        shape: 0.7,
        halo: Some((Color32::YELLOW, 4.0)),
        icon: Some(IconId::AudioRecord),
        text_on: Color32::GREEN,
        text_off: Some(Color32::BLUE),
        ..b
    }
}

#[test]
fn the_tuner_input_icon_stays_inside_its_disc_in_both_states() {
    let rect = Rect::from_min_size(RECT.min, vec2(1.75 * U, 2.15 * U));
    let radius = 0.7 * 0.5 * 1.75 * U;
    for (on, ink) in [(true, Color32::GREEN), (false, Color32::BLUE)] {
        let mut rig = Rig::new(Kind::Button, 0.0);
        rig.style = tuner_input;
        rig.on = on;
        rig.frame(vec![]);
        let icon = painted_in(&rig.shapes, ink);
        assert!(!icon.is_empty(), "{on}");
        for r in icon {
            // Every corner of the glyph's bounding box is on or inside the disc.
            for corner in [
                r.left_top(),
                r.right_top(),
                r.left_bottom(),
                r.right_bottom(),
            ] {
                let d = (corner - rect.center()).length();
                assert!(
                    d <= radius,
                    "{on}: corner {corner:?} is {d} from the centre, radius {radius}"
                );
            }
        }
    }
}

fn rect_icon(b: Button) -> Button {
    Button {
        round: false,
        icon: Some(IconId::Record),
        text_on: Color32::GREEN,
        ..b
    }
}

fn rect_icon_half(b: Button) -> Button {
    Button {
        icon_scale: 0.5,
        ..rect_icon(b)
    }
}

#[test]
fn a_rect_buttons_icon_has_the_icons_md_edge_centred_in_its_area_and_scales() {
    // A 96 × 128 rect with a label: the icon area is the upper two thirds (96 × 85.3), far bigger than the icon.
    let edge = 4.0 / 3.0 * U;
    let area_centre = pos2(RECT.center().x, RECT.top() + 128.0 / 3.0);
    let bounds = |style: for<'a> fn(Button<'a>) -> Button<'a>| {
        let mut rig = Rig::new(Kind::Button, 0.0);
        rig.style = style;
        rig.on = true;
        rig.frame(vec![]);
        let icon = painted_in(&rig.shapes, Color32::GREEN);
        assert!(!icon.is_empty());
        icon.iter().fold(Rect::NOTHING, |a, r| a.union(*r))
    };
    let (full, half) = (bounds(rect_icon), bounds(rect_icon_half));
    // Not the area's 85 points: the glyph fits the 42.7-point icon square (U × 4/3), centred in the area.
    assert!(
        full.width() <= edge + 1.0 && full.height() <= edge + 1.0,
        "{full:?}"
    );
    assert!(full.width() > 0.3 * edge, "{full:?}");
    assert!((full.center() - area_centre).length() <= 1.5, "{full:?}");
    // A size factor of 0.5 halves it.
    assert!(
        (half.width() - full.width() / 2.0).abs() <= 1.0,
        "{half:?} vs {full:?}"
    );
    assert!((half.center() - area_centre).length() <= 1.5, "{half:?}");
}
