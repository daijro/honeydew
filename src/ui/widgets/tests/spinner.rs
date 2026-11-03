use super::*;

#[test]
fn a_spinner_shows_its_caption_the_current_item_and_two_chevrons() {
    let channel = Color32::RED;
    let rig = Rig::new(Kind::Spinner, 0.5);
    let texts = texts(&rig.shapes);
    // Item 2 of 5; the caption in FontSize 1 (16 points at U = 32), the value in FontSize 2 (20).
    let size = |name: &str| {
        let t = rig.shapes.iter().find_map(|c| match &c.shape {
            egui::Shape::Text(t) if t.galley.text() == name => Some(t.clone()),
            _ => None,
        });
        t.expect(name).galley.job.sections[0].format.font_id.size
    };
    assert!(texts.iter().any(|(s, c, _)| s == "Label" && *c == channel));
    assert!(texts.iter().any(|(s, c, _)| s == "Square" && *c == channel));
    assert_eq!((size("Label"), size("Square")), (16.0, 20.0));
    // Two three-point polylines in the channel colour, one hairline (2 px) wide.
    let t = metrics().t();
    let chevrons: Vec<_> = rig
        .shapes
        .iter()
        .filter_map(|c| match &c.shape {
            egui::Shape::Path(p) if !p.closed => Some(p.clone()),
            _ => None,
        })
        .collect();
    assert_eq!(chevrons.len(), 2);
    for p in &chevrons {
        assert_eq!(p.points.len(), 3);
        assert_eq!(p.stroke.width, t);
        assert_eq!(p.stroke.color, egui::epaint::ColorMode::Solid(channel));
    }
    // `^` first: apex above the value centre; `v` mirrors it below.
    assert!((chevrons[0].points[1].y - (176.0 - 0.85 * U)).abs() < 1e-3);
    assert!((chevrons[1].points[1].y - (176.0 + 0.85 * U)).abs() < 1e-3);
}

#[test]
fn spinner_texts_are_drawn_in_full_centred_on_caption_and_value() {
    const LONG: &str = "An item wider than 3 U";
    let m = metrics();
    let mut rig = Rig::new(Kind::Spinner, 1.0);
    rig.items = &[LONG; 5];
    rig.label = "A caption wider than 3 U";
    rig.frame(vec![]);
    let boxes = text_boxes(&rig.shapes);
    assert_eq!(boxes.len(), 2, "{boxes:?}");
    assert_plain(&boxes[0], m.font_id(FontSize::Pt8).size, pos2(148.0, 112.0));
    assert_plain(&boxes[1], m.font_id(FontSize::Pt10).size, VALUE);
    assert!(
        boxes.iter().all(|b| b.1.width() > RECT.width()),
        "{boxes:?}"
    );
}

#[test]
fn the_chevrons_step_one_item_and_stop_at_the_ends() {
    let mut rig = Rig::new(Kind::Spinner, 0.5);
    // `^` goes up the list (item 2 → 1), `v` down it.
    rig.click(UP);
    assert_eq!(rig.value, 0.25);
    assert!(rig.changed && !rig.popup_is_open());
    rig.click(UP);
    assert_eq!(rig.value, 0.0);
    // Clamped, not wrapped, and nothing is reported.
    rig.click(UP);
    assert_eq!(rig.value, 0.0);
    assert!(!rig.changed);
    for want in [0.25, 0.5, 0.75, 1.0] {
        rig.click(DOWN);
        assert_eq!(rig.value, want);
    }
    rig.click(DOWN);
    assert_eq!(rig.value, 1.0);
    assert!(!rig.changed && !rig.popup_is_open());
}

#[test]
fn stepping_walks_every_item_for_any_item_count() {
    for n in [1, 2, 3, 5, 8, 12] {
        let mut rig = Rig::new(Kind::Spinner, 0.0);
        rig.items = &ITEMS[..n];
        rig.frame(vec![]);
        for (k, name) in ITEMS.iter().take(n).enumerate() {
            assert_eq!(rig.value, menu_value(k, n), "{k} of {n}");
            let shown = texts(&rig.shapes);
            assert!(shown.iter().any(|(s, ..)| s == name), "{k} of {n}");
            rig.click(DOWN);
        }
        // Past the last item: it stays.
        assert_eq!(rig.value, menu_value(n - 1, n));
    }
}

#[test]
fn stepping_from_an_off_grid_value_snaps_to_the_next_item() {
    // 0.3 on five items reads item 1; one step down is item 2 (0.5), one up is item 0.
    let mut rig = Rig::new(Kind::Spinner, 0.3);
    rig.click(DOWN);
    assert_eq!(rig.value, 0.5);
    let mut rig = Rig::new(Kind::Spinner, 0.3);
    rig.click(UP);
    assert_eq!(rig.value, 0.0);
    // Garbage in reads as the first item.
    let mut rig = Rig::new(Kind::Spinner, f32::NAN);
    assert!(texts(&rig.shapes).iter().any(|(s, ..)| s == "Sine"));
    rig.click(DOWN);
    assert_eq!(rig.value, 0.25);
    let rig = Rig::new(Kind::Spinner, 7.0);
    assert!(texts(&rig.shapes).iter().any(|(s, ..)| s == "Noise"));
}

#[test]
fn tapping_the_value_opens_the_popup_and_a_pick_sets_that_item() {
    let mut rig = Rig::new(Kind::Spinner, 0.5);
    let menu = rig.open_popup(VALUE);
    assert!(rig.popup_is_open());
    assert_eq!(rig.value, 0.5);
    // Five rows of 1.25 U.
    assert_eq!(menu.height(), 5.0 * ROW);
    rig.pick(menu, 3);
    assert_eq!(rig.value, 0.75);
    assert!(rig.changed);
    rig.frame(vec![]);
    assert!(!rig.popup_is_open());
    // Another item, another payload: row 0.
    let menu = rig.open_popup(VALUE);
    rig.pick(menu, 0);
    assert_eq!(rig.value, 0.0);
}

#[test]
fn the_popup_marks_the_current_item_with_the_inverted_bar() {
    let background = theme_by_id(0).colors.background.to_color32();
    for (value, current) in [(0.5, 2), (0.0, 0), (1.0, 4)] {
        let mut rig = Rig::new(Kind::Spinner, value);
        let menu = rig.open_popup(VALUE);
        let bars: Vec<Rect> = rig
            .shapes
            .iter()
            .filter_map(|c| match &c.shape {
                egui::Shape::Rect(r) if r.fill == background && r.rect.height() == ROW => {
                    Some(r.rect)
                }
                _ => None,
            })
            .collect();
        assert_eq!(bars.len(), 1, "{value}");
        assert_eq!(bars[0].top(), menu.top() + current as f32 * ROW);
        assert_eq!(bars[0].width(), menu.width());
    }
}

#[test]
fn picking_the_current_item_closes_the_popup_and_leaves_the_value_alone() {
    // 0.3 reads item 1; picking item 1 must not snap the value to 0.25.
    let mut rig = Rig::new(Kind::Spinner, 0.3);
    let menu = rig.open_popup(VALUE);
    rig.pick(menu, 1);
    assert_eq!(rig.value, 0.3);
    assert!(!rig.changed);
    rig.frame(vec![]);
    assert!(!rig.popup_is_open());
}

#[test]
fn escape_and_a_click_outside_close_the_popup_without_a_change() {
    let mut rig = Rig::new(Kind::Spinner, 0.5);
    rig.open_popup(VALUE);
    rig.frame(vec![Event::Key {
        key: egui::Key::Escape,
        physical_key: None,
        pressed: true,
        repeat: false,
        modifiers: Modifiers::NONE,
    }]);
    rig.frame(vec![]);
    assert!(!rig.popup_is_open());
    assert_eq!(rig.value, 0.5);
    rig.open_popup(VALUE);
    rig.click(pos2(700.0, 500.0));
    rig.frame(vec![]);
    assert!(!rig.popup_is_open());
    assert_eq!(rig.value, 0.5);
}

#[test]
fn spinners_answer_only_the_primary_button_and_ignore_input_when_disabled() {
    let mut rig = Rig::new(Kind::Spinner, 0.5);
    rig.click_with(DOWN, PointerButton::Secondary);
    rig.click_with(VALUE, PointerButton::Secondary);
    rig.frame(vec![]);
    assert_eq!(rig.value, 0.5);
    assert!(!rig.popup_is_open());

    let mut rig = Rig::new(Kind::Spinner, 0.5);
    rig.disabled = true;
    rig.click(UP);
    rig.click(DOWN);
    rig.click(VALUE);
    rig.frame(vec![]);
    assert_eq!(rig.value, 0.5);
    assert!(!rig.popup_is_open());
    // An empty item list has nothing to step or open either.
    let mut rig = Rig::new(Kind::Spinner, 0.0);
    rig.items = &[];
    rig.click(DOWN);
    rig.click(VALUE);
    rig.frame(vec![]);
    assert!(!rig.popup_is_open() && !rig.changed);
}

#[test]
fn an_unbound_spinner_edits_an_item_index() {
    let mut rig = Rig::new(Kind::SpinnerIndex, 0.0);
    rig.index = 1;
    rig.click(DOWN);
    assert_eq!(rig.index, 2);
    assert!(rig.changed);
    let menu = rig.open_popup(VALUE);
    rig.pick(menu, 4);
    assert_eq!(rig.index, 4);
    rig.click(DOWN);
    assert_eq!((rig.index, rig.changed), (4, false));
    // The value is not touched.
    assert_eq!(rig.value, 0.0);
    // A stale index shows the last item and does not change by itself.
    let mut rig = Rig::new(Kind::SpinnerIndex, 0.0);
    rig.index = 9;
    rig.frame(vec![]);
    assert!(texts(&rig.shapes).iter().any(|(s, ..)| s == "Noise"));
    assert_eq!(rig.index, 9);
}

/// Where `text` was painted last (the popup is painted over the panel): the rect of its galley.
fn text_rect(rig: &Rig, text: &str) -> Rect {
    rig.shapes
        .iter()
        .rev()
        .find_map(|c| match &c.shape {
            egui::Shape::Text(t) if t.galley.text() == text => {
                Some(t.galley.rect.translate(t.pos.to_vec2()))
            }
            _ => None,
        })
        .unwrap_or_else(|| panic!("{text} not drawn"))
}

#[test]
fn a_multi_column_popup_fills_its_columns_top_to_bottom() {
    let mut rig = Rig::new(Kind::Spinner, 0.0);
    // Twelve items in columns of four (the Arpeggiator's `Arp` has 24 in columns of 8).
    (rig.items, rig.column_len) = (&ITEMS, Some(4));
    rig.open_popup(VALUE);
    // The first row holds items 0, 4 and 8, left to right; item 5 is under item 4.
    let [sine, noise, step, ramp] = ["Sine", "Noise", "Step", "Ramp"].map(|t| text_rect(&rig, t));
    assert!((sine.center().y - noise.center().y).abs() < 1.0);
    assert!((noise.center().y - step.center().y).abs() < 1.0);
    assert!(sine.center().x < noise.center().x && noise.center().x < step.center().x);
    assert!((ramp.center().y - noise.center().y - ROW).abs() < 1.0);
    // Picking in the second column selects item 5.
    rig.click(ramp.center());
    assert_eq!(rig.value, menu_value(5, 12));
}

#[test]
fn a_disabled_item_cannot_be_picked_and_the_chevrons_skip_it() {
    // The Scale's `Custom` (its last item) shows but cannot be picked.
    let mut rig = Rig::new(Kind::Spinner, menu_value(1, 5));
    rig.disabled_items = &[2, 4];
    let menu = rig.open_popup(VALUE);
    rig.pick(menu, 2);
    assert_eq!(rig.value, menu_value(1, 5));
    assert!(!rig.changed);
    rig.frame(vec![]);
    if rig.popup_is_open() {
        rig.click(pos2(0.0, 0.0));
    }
    // `v` from item 1 jumps over item 2 to item 3, and stays there: item 4 is disabled too.
    rig.click(DOWN);
    assert_eq!(rig.value, menu_value(3, 5));
    rig.click(DOWN);
    assert_eq!(rig.value, menu_value(3, 5));
    assert!(!rig.changed);
    rig.click(UP);
    assert_eq!(rig.value, menu_value(1, 5));
}

#[test]
fn a_value_wider_than_its_spinner_is_drawn_in_full() {
    // GMSynth's MW DEST: `Osc. Mod.` in full in a 2.42 U column, 2.59 U wide at 10 pt.
    let (theme, m) = (theme_by_id(0), metrics());
    let cx = UiCtx {
        theme: &theme,
        m: &m,
    };
    let rect = Rect::from_min_size(pos2(100.0, 100.0), vec2(2.42 * U, 4.0 * U));
    let raw = RawInput {
        screen_rect: Some(SCREEN),
        ..Default::default()
    };
    let output = Context::default().run(raw, |ctx| {
        egui::CentralPanel::default().show(ctx, |ui| {
            let c = Control::new(Id::new("mw"), rect, "MW DEST", Color32::RED, 0.0);
            spinner_index(ui, &cx, &c, &["Osc. Mod."], &mut 0);
        });
    });
    let boxes = text_boxes(&output.shapes);
    let value = boxes
        .iter()
        .find(|b| b.0 == "Osc. Mod.")
        .expect("the full value");
    assert!(value.1.width() > rect.width(), "{value:?}");
    let body = pos2(
        rect.center().x,
        rect.top() + 0.75 * U + (rect.height() - 0.75 * U) / 2.0,
    );
    assert_plain(value, m.font_id(FontSize::Pt10).size, body);
}

#[test]
fn a_popup_closes_when_its_control_stops_being_drawn() {
    // An undo can hide a control while its menu is open (Tremolo's `Sync` hides the Rate knob); a redo that brings
    // the control back must not bring the menu back with it.
    for (kind, at, button, marker) in [
        (Kind::Knob, RECT.center(), PointerButton::Secondary, "Reset"),
        (Kind::Spinner, VALUE, PointerButton::Primary, "Saw"),
    ] {
        let mut rig = Rig::new(kind, 0.0);
        let shown = |rig: &Rig| texts(&rig.shapes).iter().any(|t| t.0 == marker);
        rig.click_with(at, button);
        // Open, and it stays open while its control is drawn.
        for _ in 0..3 {
            rig.frame(vec![]);
            assert!(rig.popup_is_open() && shown(&rig), "{kind:?}");
        }
        rig.hidden = true;
        rig.frame(vec![]);
        assert!(!shown(&rig), "{kind:?}");
        rig.hidden = false;
        rig.frame(vec![]);
        assert!(!rig.popup_is_open() && !shown(&rig), "{kind:?}");
        rig.frame(vec![]);
        assert!(!shown(&rig), "{kind:?}");
    }
}

#[test]
fn a_rect_shorter_than_its_strips_leaves_an_empty_body_never_an_inverted_one() {
    // The strips are 0.75 U = 24 points; with a caption and `value_text` a spinner needs 48.
    let m = metrics();
    for h in [0.0, 10.0, 23.0, 24.0, 30.0, 47.0, 48.0, 100.0] {
        let rect = Rect::from_min_size(pos2(100.0, 100.0), vec2(96.0, h));
        for (label, value_text) in [("Scale", false), ("", true), ("Scale", true)] {
            let c = Control {
                value_text,
                ..Control::new(Id::new("s"), rect, label, Color32::RED, 0.0)
            };
            let (top, body, bottom) = c.strips(&m);
            let case = format!("{h} {label:?} {value_text}");
            assert!(
                body.height() >= 0.0 && rect.contains_rect(body),
                "{case}: {body:?}"
            );
            for strip in [top, bottom].into_iter().flatten() {
                assert!(
                    strip.height() >= 0.0 && rect.contains_rect(strip),
                    "{case}: {strip:?}"
                );
            }
        }
    }
}

#[test]
fn a_spinner_shorter_than_its_strips_takes_taps() {
    // A tap used to clamp the chevron zones to an inverted body, and panic.
    for (h, value_text) in [(10.0, false), (23.0, false), (40.0, true), (47.0, true)] {
        let rect = Rect::from_min_size(pos2(100.0, 100.0), vec2(96.0, h));
        let mut screen = crate::ui::offscreen::Offscreen::new();
        let mut index = 0;
        let mut f = |ui: &mut egui::Ui, cx: &UiCtx| {
            let c = Control {
                value_text,
                ..Control::new(Id::new("s"), rect, "Scale", Color32::RED, 0.0)
            };
            spinner_index(ui, cx, &c, &["a", "b"], &mut index);
        };
        for at in [
            rect.center(),
            rect.left_top() + vec2(1.0, 1.0),
            rect.right_bottom() - vec2(1.0, 1.0),
        ] {
            screen.click(at, &mut f);
        }
    }
}
