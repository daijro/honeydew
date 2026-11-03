use super::*;

// Pure helpers.

#[test]
fn growth_is_the_value_or_the_distance_from_the_middle() {
    assert_eq!(growth(0.0, false), 0.0);
    assert_eq!(growth(0.3, false), 0.3);
    assert_eq!(growth(1.0, false), 1.0);
    assert_eq!(growth(0.5, true), 0.0);
    assert_eq!(growth(0.0, true), 1.0);
    assert_eq!(growth(1.0, true), 1.0);
    assert!(near(growth(0.25, true), 0.5));
}

#[test]
fn menu_steps_snap_ties_away_from_zero() {
    assert_eq!(snap(0.3, None, None), 0.3);
    // Five items: 0, 0.25, 0.5, 0.75, 1.
    assert_eq!(snap(0.3, Some(5), None), 0.25);
    assert_eq!(snap(0.4, Some(5), None), 0.5);
    assert_eq!(snap(1.0, Some(5), None), 1.0);
    // 0.5 on two items is a tie between 0 and 1: away from zero wins (the act-ID form).
    assert_eq!(snap(0.5, Some(2), None), 1.0);
}

#[test]
fn an_item_law_picks_the_step_at_a_tie() {
    // Halves up: 0.5 on two items reads item 1.
    let up = |v: f32| v.round() as usize;
    assert_eq!(snap(0.5, Some(2), Some(up)), 1.0);
    assert_eq!(snap(0.3, Some(2), Some(up)), 0.0);
}

#[test]
fn bipolar_follows_a_default_of_exactly_half() {
    let c = |default| Control::new(Id::new("c"), Rect::ZERO, "", Color32::RED, default);
    assert!(c(0.5).bipolar);
    assert!(!c(0.0).bipolar);
    assert!(!c(0.85312).bipolar);
}

#[test]
fn strips_are_0_75_u_and_only_there_when_wanted() {
    let m = metrics();
    let rect = Rect::from_min_size(pos2(10.0, 20.0), vec2(64.0, 128.0));
    let plain = Control::new(Id::new("c"), rect, "", Color32::RED, 0.0);
    assert_eq!(plain.strips(&m), (None, rect, None));
    let full = Control {
        label: "Gain",
        value_text: true,
        ..plain
    };
    let (label, body, value) = full.strips(&m);
    assert_eq!(
        label,
        Some(Rect::from_min_size(pos2(10.0, 20.0), vec2(64.0, 24.0)))
    );
    assert_eq!(
        body,
        Rect::from_min_size(pos2(10.0, 44.0), vec2(64.0, 80.0))
    );
    assert_eq!(
        value,
        Some(Rect::from_min_size(pos2(10.0, 124.0), vec2(64.0, 24.0)))
    );
}

#[test]
fn the_menu_is_reset_then_the_parameter_items_with_unlink_only_for_a_linked_param() {
    use CtrlAction::*;
    use MenuAction::{Pick, Reset};
    let rows = |items: &[PopupItem<MenuAction>]| {
        items
            .iter()
            .map(|i| (i.id, i.label.to_string(), i.row, i.disabled))
            .collect::<Vec<_>>()
    };
    let links = [
        (LinkMidiCc, "Link to MIDI CC"),
        (LinkModWheel, "Link to Mod Wheel"),
        (UnlinkMidiCc, "Unlink from MIDI CC"),
        (AddAutomationTrack, "Add automation track"),
    ];
    let want = |linked: bool| {
        let mut want: Vec<_> = links
            .iter()
            .enumerate()
            .map(|(row, &(a, l))| {
                let off = a == UnlinkMidiCc && !linked;
                (Some(Pick(a)), l.to_string(), row + 1, off)
            })
            .collect();
        want.insert(0, (Some(Reset), "Reset".to_owned(), 0, false));
        want
    };
    // `Unlink from MIDI CC` is dimmed unless the parameter has a CC link; the rest are always on.
    assert_eq!(
        rows(&menu_items(true, ParamMenu::Bound, false)),
        want(false)
    );
    assert_eq!(
        rows(&menu_items(true, ParamMenu::Linked, false)),
        want(true)
    );
    // Unbound knobs and sliders: Reset only. Unbound toggles and buttons: nothing.
    assert_eq!(
        rows(&menu_items(true, ParamMenu::Unbound, false)),
        want(false)[..1]
    );
    assert!(menu_items(false, ParamMenu::Unbound, false).is_empty());
    // Toggles: no Reset.
    let want: Vec<_> = want(false)[1..]
        .iter()
        .map(|(a, l, row, d)| (*a, l.clone(), row - 1, *d))
        .collect();
    assert_eq!(rows(&menu_items(false, ParamMenu::Bound, false)), want);
    assert!(
        menu_items(true, ParamMenu::Bound, false)
            .iter()
            .all(|i| i.col == 0 && !i.is_header())
    );
}

#[test]
fn fine_tune_control_ends_a_bound_menu_only_when_the_dialog_supports_it() {
    use CtrlAction::*;
    let labels = |items: &[PopupItem<MenuAction>]| {
        items
            .iter()
            .map(|i| (i.id, i.label.to_string(), i.row, i.disabled))
            .collect::<Vec<_>>()
    };
    // A fifth row after `Add automation track`, enabled, present only for a supported control.
    for menu in [ParamMenu::Bound, ParamMenu::Linked] {
        let with = menu_items(true, menu, true);
        let without = menu_items(true, menu, false);
        assert_eq!(with.len(), without.len() + 1, "{menu:?}");
        assert_eq!(labels(&with[..without.len()]), labels(&without));
        let last = &labels(&with)[without.len()];
        let want = (
            Some(MenuAction::Pick(FineTune)),
            "Fine tune control".to_owned(),
            5,
            false,
        );
        assert_eq!(*last, want, "{menu:?}");
    }
    // Never on an unbound control.
    assert_eq!(
        labels(&menu_items(true, ParamMenu::Unbound, true)),
        labels(&menu_items(true, ParamMenu::Unbound, false))
    );
    assert!(menu_items(false, ParamMenu::Unbound, true).is_empty());
}

#[test]
fn a_supported_knobs_fine_tune_control_is_signalled_as_its_pick() {
    let mut rig = Rig::new(Kind::Knob, 0.25);
    rig.fine_tune = true;
    rig.click_with(RECT.center(), PointerButton::Secondary);
    rig.frame(vec![]);
    let menu = rig.menu_rect().expect("menu open");
    // Reset, the four link and lane items, then Fine tune control: the sixth row.
    let at = pos2(
        menu.left() + 10.0,
        menu.bottom() - 0.5 * menu.height() / 6.0,
    );
    rig.click(at);
    let want = Signal {
        pressed: false,
        pick: Some(CtrlAction::FineTune),
    };
    assert_eq!(rig.signal, want);
    assert_eq!(rig.value, 0.25, "the value stays");
    rig.frame(vec![]);
    assert!(!rig.popup_is_open(), "closed");
}

#[test]
fn the_parameter_menu_and_the_spinner_popup_have_their_own_ids() {
    let id = Id::new("w");
    assert_ne!(menu_id(id), popup_id(id));
}

/// Points of pointer travel over a knob's full range (0.24 per inch of 192 points) and over the rig's vertical
/// slider track.
const KNOB_RANGE: f32 = 800.0;
const V_TRACK: f32 = 72.0;

#[test]
fn dragging_up_a_full_range_sweeps_a_knob_and_a_v_slider_over_it() {
    for (kind, default, range) in [
        (Kind::Knob, 0.0, KNOB_RANGE),
        (Kind::Knob, 0.5, KNOB_RANGE),
        (Kind::VSlider, 0.0, V_TRACK),
        (Kind::VSlider, 0.5, V_TRACK),
    ] {
        let mut rig = Rig::new(kind, 0.0);
        rig.default = default;
        // In steps, past egui's drag threshold; the points before the drag was recognised count too.
        rig.drag(&[vec2(0.0, -10.0), vec2(0.0, -50.0), vec2(0.0, -range)]);
        assert!(near(rig.value, 1.0), "{}", rig.value);
        // Back down a quarter: 0.75.
        rig.drag(&[vec2(0.0, 10.0), vec2(0.0, range / 4.0)]);
        assert!(near(rig.value, 0.75), "{}", rig.value);
    }
}

#[test]
fn h_sliders_follow_horizontal_drags() {
    // The track is 64 points long.
    let mut rig = Rig::new(Kind::HSlider, 0.0);
    rig.drag(&[vec2(10.0, 0.0), vec2(32.0, 0.0)]);
    assert!(near(rig.value, 0.5), "{}", rig.value);
    // Vertical movement does nothing.
    rig.drag(&[vec2(0.0, -10.0), vec2(0.0, -2.0 * U)]);
    assert!(near(rig.value, 0.5), "{}", rig.value);
}

#[test]
fn a_drag_past_the_end_turns_back_at_once() {
    let mut rig = Rig::new(Kind::VSlider, 0.5);
    // Up a whole track clamps at 1 halfway; the next quarter track down gives 0.75, not 1.
    let up = vec2(0.0, -V_TRACK);
    rig.drag(&[vec2(0.0, -10.0), up, up + vec2(0.0, V_TRACK / 4.0)]);
    assert!(near(rig.value, 0.75), "{}", rig.value);
}

#[test]
fn menu_parameters_snap_to_their_steps_and_small_moves_add_up() {
    let mut rig = Rig::new(Kind::Knob, 0.0);
    rig.steps = Some(5);
    // 0.3 of the range: snaps to 0.25.
    rig.drag(&[vec2(0.0, -10.0), vec2(0.0, -0.3 * KNOB_RANGE)]);
    assert_eq!(rig.value, 0.25);
    // Moves of an eighth of a step (25 points) still add up: 233 points in all take 0.25 to 0.54, which snaps to 0.5.
    let offsets: Vec<Vec2> = (0..10).map(|i| vec2(0.0, -8.0 - 25.0 * i as f32)).collect();
    rig.drag(&offsets);
    assert_eq!(rig.value, 0.5);
}

#[test]
fn a_double_tap_resets_to_the_default() {
    // A unipolar default other than 0.5, so the reset cannot be a hard-coded middle.
    // A stepped control resets to the default too, even one off its steps.
    for (kind, steps) in [
        (Kind::Knob, None),
        (Kind::VSlider, None),
        (Kind::HSlider, None),
        (Kind::Knob, Some(3)),
    ] {
        let mut rig = Rig::new(kind, 0.9);
        rig.default = 0.25;
        rig.steps = steps;
        rig.click(RECT.center());
        assert_eq!(rig.value, 0.9);
        rig.click(RECT.center());
        assert_eq!(rig.value, 0.25);
    }
}

#[test]
fn the_menu_reset_item_resets_and_closes() {
    let mut rig = Rig::new(Kind::VSlider, 0.9);
    rig.default = 0.25;
    rig.click_with(RECT.center(), PointerButton::Secondary);
    rig.frame(vec![]);
    let menu = rig.menu_rect().expect("menu open");
    // Five rows: Reset, then the four link and automation items.
    let row = |r: f32| {
        pos2(
            menu.left() + 10.0,
            menu.top() + (r + 0.5) * menu.height() / 5.0,
        )
    };
    // Row 3 (Unlink from MIDI CC, no link) is disabled: nothing happens and the menu stays.
    rig.click(row(3.0));
    assert_eq!(rig.value, 0.9);
    assert!(rig.menu_rect().is_some());
    // Row 0: Reset.
    rig.click(row(0.0));
    assert_eq!(rig.value, 0.25);
    rig.frame(vec![]);
    assert!(!rig.popup_is_open(), "closed after Reset");
}

/// Opens the parameter menu with a secondary click and picks its `Add automation track` row: the last of `rows`.
fn pick_add_automation(rig: &mut Rig, rows: f32) {
    rig.click_with(RECT.center(), PointerButton::Secondary);
    rig.frame(vec![]);
    let menu = rig.menu_rect().expect("menu open");
    let at = pos2(
        menu.left() + 10.0,
        menu.bottom() - 0.5 * menu.height() / rows,
    );
    rig.frame(vec![Event::PointerMoved(at)]);
    rig.button(at, PointerButton::Primary, true);
    assert_eq!(rig.signal, Signal::default(), "not on the press");
    rig.button(at, PointerButton::Primary, false);
}

#[test]
fn a_bound_controls_press_is_signalled_in_its_frame_only() {
    for kind in [Kind::Knob, Kind::HSlider, Kind::Toggle] {
        let mut rig = Rig::new(kind, 0.25);
        rig.frame(vec![Event::PointerMoved(RECT.center())]);
        assert_eq!(rig.signal, Signal::default());
        rig.button(RECT.center(), PointerButton::Primary, true);
        let pressed = Signal {
            pressed: true,
            pick: None,
        };
        assert_eq!(rig.signal, pressed, "{kind:?}");
        // Held, then released: no more presses.
        rig.frame(vec![]);
        assert_eq!(rig.signal, Signal::default(), "{kind:?}");
        rig.button(RECT.center(), PointerButton::Primary, false);
        assert_eq!(rig.signal, Signal::default(), "{kind:?}");
    }
}

#[test]
fn unbound_or_disabled_controls_signal_nothing() {
    for (unbound, disabled) in [(true, false), (false, true)] {
        let mut rig = Rig::new(Kind::Knob, 0.25);
        (rig.unbound, rig.disabled) = (unbound, disabled);
        rig.frame(vec![Event::PointerMoved(RECT.center())]);
        rig.button(RECT.center(), PointerButton::Primary, true);
        assert_eq!(rig.signal, Signal::default(), "unbound {unbound}");
    }
}

#[test]
fn add_automation_track_is_signalled_and_closes_the_menu() {
    // A knob's menu: Reset and the four link and automation items; a toggle's: no Reset.
    for (kind, rows) in [(Kind::Knob, 5.0), (Kind::Toggle, 4.0)] {
        let mut rig = Rig::new(kind, 0.25);
        pick_add_automation(&mut rig, rows);
        let add = Signal {
            pressed: false,
            pick: Some(CtrlAction::AddAutomationTrack),
        };
        assert_eq!(rig.signal, add, "{kind:?}");
        assert_eq!(rig.value, 0.25, "{kind:?}: the value stays");
        rig.frame(vec![]);
        assert_eq!(rig.signal, Signal::default());
        assert!(!rig.popup_is_open(), "{kind:?}: closed");
    }
}

#[test]
fn each_link_item_is_signalled_as_its_pick_and_unlink_only_when_linked() {
    use CtrlAction::*;
    // A knob's menu: Reset, then rows 1…4.
    for (row, pick, linked) in [
        (1.0, LinkMidiCc, false),
        (2.0, LinkModWheel, false),
        (3.0, UnlinkMidiCc, true),
    ] {
        let mut rig = Rig::new(Kind::Knob, 0.25);
        rig.linked = linked;
        rig.click_with(RECT.center(), PointerButton::Secondary);
        rig.frame(vec![]);
        let menu = rig.menu_rect().expect("menu open");
        let at = pos2(
            menu.left() + 10.0,
            menu.top() + (row + 0.5) * menu.height() / 5.0,
        );
        rig.click(at);
        let want = Signal {
            pressed: false,
            pick: Some(pick),
        };
        assert_eq!(rig.signal, want, "{pick:?}");
        assert_eq!(rig.value, 0.25, "{pick:?}: the value stays");
        rig.frame(vec![]);
        assert!(!rig.popup_is_open(), "{pick:?}: closed");
    }
}

#[test]
fn toggles_flip_on_click() {
    let mut rig = Rig::new(Kind::Toggle, 0.0);
    rig.click(RECT.center());
    assert_eq!(rig.value, 1.0);
    assert!(rig.changed);
    rig.frame(vec![]);
    assert!(!rig.changed);
    rig.click(RECT.center());
    assert_eq!(rig.value, 0.0);
    // A secondary click opens the menu and does not flip.
    rig.click_with(RECT.center(), PointerButton::Secondary);
    assert_eq!(rig.value, 0.0);
    assert!(rig.menu_rect().is_some());
}

#[test]
fn slider_shapes_follow_the_disabled_and_bipolar_flags() {
    let colors = theme_by_id(0).colors;
    let (menu_fill, control_fill) = (
        colors.menu_fill.to_color32(),
        colors.control_fill.to_color32(),
    );
    let t = metrics().t();
    for kind in [Kind::VSlider, Kind::HSlider] {
        for (disabled, default) in [(false, 0.0), (true, 0.0), (false, 0.5)] {
            let mut rig = Rig::new(kind, 0.75);
            rig.default = default;
            rig.disabled = disabled;
            rig.frame(vec![]);
            let mut rects = Vec::new();
            let mut circles = Vec::new();
            for clipped in &rig.shapes {
                match &clipped.shape {
                    // Not the panel's background.
                    egui::Shape::Rect(r) if RECT.contains_rect(r.rect) => {
                        rects.push((r.fill, r.rect))
                    }
                    egui::Shape::Circle(c) => circles.push((c.fill, c.radius)),
                    _ => {}
                }
            }
            // Track, then fill; outer thumb, then inner thumb, both inks menuFill when disabled.
            let ink = if disabled { menu_fill } else { Color32::RED };
            assert_eq!(rects.len(), 2);
            assert_eq!((rects[0].0, rects[1].0), (control_fill, ink), "{disabled}");
            // The fill covers 0.75 of the track from the low end, or 0.25 from the middle when bipolar; the thumb
            // grows with v (0.75), or with |v - 0.5|·2 (0.5) when bipolar.
            let bipolar = default == 0.5;
            let (track, fill) = (rects[0].1, rects[1].1);
            let along = |r: Rect| match kind {
                Kind::HSlider => r.width(),
                _ => r.height(),
            };
            let share = if bipolar { 0.25 } else { 0.75 };
            assert!(
                near(along(fill), share * along(track)),
                "{fill:?} {track:?}"
            );
            let growth = if bipolar { 0.5 } else { 0.75 };
            let r = (4.5 + 1.485 * growth) * t;
            assert_eq!(circles.len(), 2);
            assert_eq!(
                (circles[0].0, circles[1].0),
                (control_fill, ink),
                "{disabled}"
            );
            assert!(
                near(circles[0].1, r) && near(circles[1].1, r - 2.0 * t),
                "{circles:?}"
            );
        }
    }
}

#[test]
fn disabled_controls_ignore_input() {
    for kind in [Kind::Knob, Kind::VSlider, Kind::HSlider, Kind::Toggle] {
        let mut rig = Rig::new(kind, 0.0);
        rig.disabled = true;
        rig.drag(&[vec2(0.0, -10.0), vec2(0.0, -2.0 * U)]);
        rig.click(RECT.center());
        rig.click(RECT.center());
        rig.click_with(RECT.center(), PointerButton::Secondary);
        assert_eq!(rig.value, 0.0);
        assert!(rig.menu_rect().is_none());
    }
}

#[test]
fn only_the_primary_button_drags() {
    for kind in [Kind::Knob, Kind::VSlider, Kind::HSlider] {
        for button in [PointerButton::Secondary, PointerButton::Middle] {
            let mut rig = Rig::new(kind, 0.5);
            let d = -2.0 * U;
            rig.drag_with(button, &[vec2(-10.0, -10.0), vec2(-d, d)]);
            assert_eq!(rig.value, 0.5, "{button:?}");
        }
    }
}

/// The circles and rects painted in the channel colour at `alpha` of its own: the knob's wash, the slider's halo.
fn faint_channel(shapes: &[egui::epaint::ClippedShape], alpha: f32) -> usize {
    let ink = Color32::RED.gamma_multiply(alpha);
    shapes
        .iter()
        .filter(|c| match &c.shape {
            egui::Shape::Circle(c) => c.fill == ink,
            egui::Shape::Rect(r) => r.fill == ink,
            _ => false,
        })
        .count()
}

#[test]
fn only_a_primary_press_lights_the_wash_and_the_halo() {
    for (kind, alpha) in [(Kind::Knob, 0.05), (Kind::VSlider, 0.4)] {
        for (button, lit) in [
            (PointerButton::Primary, 1),
            (PointerButton::Secondary, 0),
            (PointerButton::Middle, 0),
        ] {
            let mut rig = Rig::new(kind, 0.5);
            rig.frame(vec![Event::PointerMoved(RECT.center())]);
            rig.button(RECT.center(), button, true);
            // Held off the control, where the hover (which lights the same wash and halo) is gone.
            rig.frame(vec![Event::PointerMoved(pos2(700.0, 500.0))]);
            assert_eq!(faint_channel(&rig.shapes, alpha), lit, "{button:?}");
        }
    }
}

/// Every shape of the frame has a finite bounding rect: no NaN or infinite geometry.
fn all_finite(shapes: &[egui::epaint::ClippedShape]) -> bool {
    shapes
        .iter()
        .all(|c| c.shape.visual_bounding_rect().is_finite())
}

#[test]
fn a_nan_value_draws_as_the_default_and_stays_until_the_user_moves_it() {
    for kind in [Kind::Knob, Kind::VSlider, Kind::HSlider, Kind::Toggle] {
        let mut rig = Rig::new(kind, f32::NAN);
        rig.value_text = true;
        rig.default = 0.25;
        rig.frame(vec![]);
        assert!(all_finite(&rig.shapes), "{kind:?}");
        assert!(rig.value.is_nan() && !rig.changed);
        // The default's text, not "NaN".
        assert!(texts(&rig.shapes).iter().all(|(s, ..)| !s.contains("NaN")));
    }
    // A drag starts from the default.
    let mut rig = Rig::new(Kind::Knob, f32::NAN);
    rig.default = 0.25;
    rig.drag(&[vec2(0.0, -10.0), vec2(0.0, -KNOB_RANGE / 4.0)]);
    assert!(near(rig.value, 0.5), "{}", rig.value);
}

#[test]
fn an_out_of_range_value_draws_clamped_and_stays_until_the_user_moves_it() {
    let mut rig = Rig::new(Kind::VSlider, 2.0);
    rig.value_text = true;
    rig.frame(vec![]);
    assert_eq!(rig.value, 2.0);
    assert!(!rig.changed);
    assert!(texts(&rig.shapes).iter().any(|(s, ..)| s == "1.00"));
    // Down a quarter of the 48-point track (the value strip shortens it) from the clamped end.
    rig.drag(&[vec2(0.0, 10.0), vec2(0.0, 12.0)]);
    assert!(near(rig.value, 0.75), "{}", rig.value);
}

#[test]
fn long_labels_are_drawn_in_full_centred_on_their_strips() {
    // Plain text drawing: not clipped to the control, not shrunk, no ellipsis.
    const LONG: &str = "A long label, wider than 3 U";
    let m = metrics();
    let (pt8, strip) = (m.font_id(FontSize::Pt8).size, 0.75 * U);
    for kind in [Kind::Knob, Kind::VSlider, Kind::HSlider, Kind::Toggle] {
        let mut rig = Rig::new(kind, 0.5);
        rig.label = LONG;
        rig.value_text = !matches!(kind, Kind::Knob);
        rig.frame(vec![]);
        let boxes = text_boxes(&rig.shapes);
        let label = boxes.iter().find(|b| b.0 == LONG).expect("label");
        assert!(label.1.width() > RECT.width(), "{kind:?}");
        assert_plain(label, pt8, pos2(RECT.center().x, RECT.top() + strip / 2.0));
        if rig.value_text {
            let value = boxes.iter().find(|b| b.0 != LONG).expect("value");
            assert_plain(
                value,
                pt8,
                pos2(RECT.center().x, RECT.bottom() - strip / 2.0),
            );
        }
    }
}

#[test]
fn the_knob_value_shrinks_to_its_ring_down_to_6_pt_and_is_never_clipped() {
    let (theme, m) = (theme_by_id(0), metrics());
    let cx = UiCtx {
        theme: &theme,
        m: &m,
    };
    // Body 96 × 104 centred on (148, 176), D = 96, unipolar at 0: s = t and the inner radius is 0.48 D − 3 t, so
    // the text may be 2 × (r − 2 t) wide.
    let fits = 2.0 * (0.48 * 96.0 - 5.0 * m.t());
    // Fit-only sizing: the first size from 12 pt down, a point at a time, whose width fits; else 6 pt, overflowing.
    // `-12.5 dB` fits somewhere in between.
    for (value, size) in [
        ("0", Some(12.0)),
        ("-12.5 dB", None),
        ("A value text far too long", Some(6.0)),
    ] {
        let raw = RawInput {
            screen_rect: Some(SCREEN),
            ..Default::default()
        };
        let ctx = Context::default();
        let output = ctx.run(raw, |ctx| {
            egui::CentralPanel::default().show(ctx, |ui| {
                let c = Control::new(Id::new("k"), RECT, "Label", Color32::RED, 0.0);
                knob(ui, &cx, &c, &mut 0.0, |_| value.to_owned());
            });
        });
        let width = |pt: u8| {
            let font = m.font_id_pt(f32::from(pt));
            ctx.fonts_mut(|f| {
                f.layout_no_wrap(value.to_owned(), font, Color32::RED)
                    .size()
                    .x
            })
        };
        let fitted = (6u8..=12).rev().find(|&pt| width(pt) <= fits).unwrap_or(6);
        if let Some(size) = size {
            assert_eq!(f32::from(fitted), size, "{value}");
        } else {
            assert!(fitted > 6 && fitted < 12, "{value}: {fitted}");
        }
        let boxes = text_boxes(&output.shapes);
        let shown = boxes.iter().find(|b| b.0 == value).expect("value");
        assert_plain(
            shown,
            m.font_id_pt(f32::from(fitted)).size,
            pos2(148.0, 176.0),
        );
        assert_eq!(
            shown.1.width() <= fits,
            fitted > 6 || width(6) <= fits,
            "{shown:?}"
        );
    }
}

#[test]
fn a_pressed_knob_keeps_its_value_font_size() {
    // The press swell is a touch effect: the value stays fitted to the resting ring, so its size never steps
    // mid-press. Texts of every length, so that one sits near a fit boundary.
    let (theme, m) = (theme_by_id(0), metrics());
    let cx = UiCtx {
        theme: &theme,
        m: &m,
    };
    for n in 1..=12 {
        let value = "8".repeat(n);
        let ctx = Context::default();
        let mut time = 0.0;
        let mut size = |events: Vec<Event>| {
            time += 0.05;
            let raw = RawInput {
                screen_rect: Some(SCREEN),
                time: Some(time),
                events,
                ..Default::default()
            };
            let output = ctx.run(raw, |ctx| {
                egui::CentralPanel::default().show(ctx, |ui| {
                    let c = Control::new(Id::new("k"), RECT, "Label", Color32::RED, 0.0);
                    knob(ui, &cx, &c, &mut 0.0, |_| value.clone());
                });
            });
            let boxes = text_boxes(&output.shapes);
            boxes.iter().find(|b| b.0 == value).expect("value").2
        };
        let rest = size(vec![Event::PointerMoved(RECT.center())]);
        let press = Event::PointerButton {
            pos: RECT.center(),
            button: PointerButton::Primary,
            pressed: true,
            modifiers: Modifiers::NONE,
        };
        size(vec![press]);
        // Fully swollen after the 0.1 s press animation.
        for _ in 0..4 {
            assert_eq!(size(vec![]), rest, "{value}");
        }
    }
}

#[test]
fn a_knob_paints_ring_inner_disc_halo_and_dot_in_that_order() {
    let m = metrics();
    let module_fill = theme_by_id(0).colors.module_fill.to_color32();
    // Body 96 × 104 under the label strip: centre (148, 176), D = 96; a held press swells D by 0.99 · 10 px over 128,
    // and the pressed (and hovered) dot grows × 1.05.
    let center = pos2(148.0, 176.0);
    let check = |rig: &Rig, swell: f32, pressed: bool| {
        let r = knob::ring(center, 96.0 * swell, 0.3, false, m.t());
        let mut want = vec![
            (Color32::RED, center, r.outer),
            (module_fill, center, r.inner),
        ];
        if pressed {
            want.push((Color32::RED.gamma_multiply(0.05), center, r.inner));
        }
        let grow = if pressed { 1.05 } else { 1.0 };
        want.extend([
            (module_fill, r.dot, grow * r.halo_r),
            (Color32::RED, r.dot, grow * r.dot_r),
        ]);
        let got = circles(&rig.shapes);
        assert_eq!(got.len(), want.len(), "{got:?}");
        for (g, w) in got.iter().zip(&want) {
            assert!(
                g.0 == w.0 && (g.1 - w.1).length() < 1e-3 && (g.2 - w.2).abs() < 1e-3,
                "{g:?} vs {w:?}"
            );
        }
    };
    let mut rig = Rig::new(Kind::Knob, 0.3);
    rig.default = 0.0;
    rig.frame(vec![]);
    check(&rig, 1.0, false);
    rig.frame(vec![Event::PointerMoved(center)]);
    rig.button(center, PointerButton::Primary, true);
    for _ in 0..12 {
        rig.frame(vec![]);
    }
    check(&rig, 1.0 + 0.99 * 10.0 / 128.0, true);
}

#[test]
fn a_knob_in_a_zero_height_rect_draws_nothing_out_of_bounds() {
    let theme = theme_by_id(0);
    let m = metrics();
    let cx = UiCtx {
        theme: &theme,
        m: &m,
    };
    let ctx = Context::default();
    let flat = Rect::from_min_size(pos2(100.0, 100.0), vec2(96.0, 0.0));
    for events in [vec![], vec![Event::PointerMoved(flat.center())]] {
        let output = ctx.run(
            RawInput {
                screen_rect: Some(SCREEN),
                events,
                ..Default::default()
            },
            |ctx| {
                egui::CentralPanel::default().show(ctx, |ui| {
                    let c = Control::new(Id::new("k"), flat, "", Color32::RED, 0.0);
                    knob(ui, &cx, &c, &mut 0.5, |v| format!("{v}"));
                });
            },
        );
        assert!(all_finite(&output.shapes));
    }
}
