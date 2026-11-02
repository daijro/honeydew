use super::*;

// Item lists.

const BARS: [Bar; 3] = [Bar::Bottom, Bar::Right, Bar::Left];

/// The click position of cell `i` of 4 in the 96 × 128 control: a strip's columns are 24 wide, the rows of a
/// vertical list 32 high.
fn cell_pos(bar: Bar, i: usize) -> Pos2 {
    match bar {
        Bar::Bottom => pos2(100.0 + 24.0 * (i as f32 + 0.5), 164.0),
        _ => pos2(148.0, 100.0 + 32.0 * (i as f32 + 0.5)),
    }
}

#[test]
fn a_click_selects_the_cell_under_it_for_every_direction() {
    for bar in BARS {
        let mut rig = Rig::new(Kind::List(bar), 0.0);
        rig.items = &ITEMS[..4];
        for i in [2, 0, 3, 1] {
            rig.click(cell_pos(bar, i));
            assert_eq!(rig.value, menu_value(i, 4), "{i}");
            assert!(rig.changed);
        }
        // The selected cell again: no change, and the value stays exactly as it is.
        rig.value = 0.3;
        rig.click(cell_pos(bar, 1));
        assert_eq!((rig.value, rig.changed), (0.3, false));
    }
}

#[test]
fn an_unbound_list_reports_the_selected_index() {
    for bar in BARS {
        let mut rig = Rig::new(Kind::ListIndex(bar), 0.0);
        rig.items = &ITEMS[..4];
        rig.click(cell_pos(bar, 3));
        assert_eq!(rig.index, 3);
        assert!(rig.changed);
        rig.click(cell_pos(bar, 3));
        assert_eq!((rig.index, rig.changed), (3, false));
        assert_eq!(rig.value, 0.0);
    }
}

#[test]
fn headers_are_drawn_larger_and_cannot_be_selected() {
    let mut rig = Rig::new(Kind::ListIndex(Bar::Right), 0.0);
    rig.items = &ITEMS[..4];
    rig.headers = &[0];
    rig.index = 2;
    rig.click(cell_pos(Bar::Right, 0));
    assert_eq!((rig.index, rig.changed), (2, false));
    rig.click(cell_pos(Bar::Right, 3));
    assert_eq!(rig.index, 3);
    // FontSize 3 (12 pt = 24 points at U = 32) for the header, FontSize 2 (20) for the rest.
    let sizes: Vec<(String, f32)> = rig
        .shapes
        .iter()
        .filter_map(|c| match &c.shape {
            egui::Shape::Text(t) => Some((
                t.galley.text().to_owned(),
                t.galley.job.sections[0].format.font_id.size,
            )),
            _ => None,
        })
        .collect();
    assert_eq!(
        sizes,
        [
            ("Sine", 24.0),
            ("Saw", 20.0),
            ("Square", 20.0),
            ("Triangle", 20.0)
        ]
        .map(|(s, z)| (s.to_owned(), z))
    );
}

#[test]
fn the_selected_item_is_text_coloured_with_a_text_bar_and_the_rest_channel_with_menu_fill() {
    let colors = theme_by_id(0).colors;
    let (text, menu_fill) = (colors.text.to_color32(), colors.menu_fill.to_color32());
    let t = metrics().t();
    for bar in BARS {
        let mut rig = Rig::new(Kind::ListIndex(bar), 0.0);
        rig.items = &ITEMS[..4];
        rig.index = 1;
        rig.frame(vec![]);
        // Each text in its cell's text area, text for the selected item, the channel colour for the rest.
        let shown = texts(&rig.shapes);
        assert_eq!(shown.len(), 4);
        for (i, (name, color, _)) in shown.iter().enumerate() {
            assert_eq!(name, ITEMS[i]);
            assert_eq!(*color, if i == 1 { text } else { Color32::RED }, "{name}");
        }
        // One 2 t bar per cell, light for the selected one and dark for the rest, on the bar's side.
        let bars: Vec<_> = rects(&rig.shapes)
            .into_iter()
            .filter(|(_, r)| match bar {
                Bar::Bottom => r.height() == 2.0 * t,
                _ => r.width() == 2.0 * t,
            })
            .collect();
        assert_eq!(bars.len(), 4, "{bar:?}");
        for (i, (fill, r)) in bars.iter().enumerate() {
            assert_eq!(*fill, if i == 1 { text } else { menu_fill }, "{bar:?} {i}");
            match bar {
                Bar::Bottom => assert_eq!(r.bottom(), RECT.bottom()),
                Bar::Right => assert_eq!(r.right(), RECT.right()),
                Bar::Left => assert_eq!(r.left(), RECT.left()),
                Bar::Boxed => unreachable!("boxed lists have no bars"),
            }
        }
        // The text is centred in the area left of the bar (3 t shorter on the bar's side), and not clipped.
        let first = &text_boxes(&rig.shapes)[0];
        let area = match bar {
            Bar::Bottom => {
                Rect::from_min_max(RECT.min, pos2(RECT.left() + 24.0, RECT.bottom() - 3.0 * t))
            }
            Bar::Right => {
                Rect::from_min_max(RECT.min, pos2(RECT.right() - 3.0 * t, RECT.top() + 32.0))
            }
            Bar::Left => Rect::from_min_max(
                pos2(RECT.left() + 3.0 * t, RECT.top()),
                pos2(RECT.right(), RECT.top() + 32.0),
            ),
            Bar::Boxed => unreachable!("boxed lists have no bars"),
        };
        assert!(
            (first.1.center() - area.center()).length() < 1e-3,
            "{bar:?} {first:?}"
        );
        assert!(first.4.contains_rect(first.1), "{bar:?} {first:?}");
    }
}

#[test]
fn a_pressed_cell_gets_a_light_wash() {
    let wash = theme_by_id(0).colors.text.to_color32().gamma_multiply(0.1);
    let mut rig = Rig::new(Kind::ListIndex(Bar::Bottom), 0.0);
    rig.items = &ITEMS[..4];
    rig.frame(vec![]);
    assert!(rects(&rig.shapes).iter().all(|(f, _)| *f != wash));
    let at = cell_pos(Bar::Bottom, 2);
    rig.frame(vec![Event::PointerMoved(at)]);
    rig.button(at, PointerButton::Primary, true);
    let washed: Vec<_> = rects(&rig.shapes)
        .into_iter()
        .filter(|(f, _)| *f == wash)
        .collect();
    assert_eq!(washed.len(), 1);
    assert_eq!(washed[0].1.left(), 148.0);
    // Other buttons do not press.
    rig.button(at, PointerButton::Primary, false);
    rig.button(at, PointerButton::Secondary, true);
    assert!(rects(&rig.shapes).iter().all(|(f, _)| *f != wash));
}

#[test]
fn lists_answer_only_the_primary_button_and_ignore_input_when_disabled() {
    for bar in BARS {
        let mut rig = Rig::new(Kind::ListIndex(bar), 0.0);
        rig.items = &ITEMS[..4];
        rig.click_with(cell_pos(bar, 2), PointerButton::Secondary);
        assert_eq!(rig.index, 0);
        rig.disabled = true;
        rig.click(cell_pos(bar, 2));
        assert_eq!(rig.index, 0);
        assert!(rig.menu_rect().is_none());
    }
}

#[test]
fn list_items_are_never_truncated_or_clipped() {
    // Eight rows of 16 points: shorter than a 10 pt line, which overflows its row evenly instead of being cut. The
    // items are plain text.
    let mut rig = Rig::new(Kind::ListIndex(Bar::Right), 0.0);
    rig.items = &ITEMS[..8];
    rig.frame(vec![]);
    let boxes = text_boxes(&rig.shapes);
    assert_eq!(boxes.len(), 8);
    for (i, b) in boxes.iter().enumerate() {
        assert!(b.1.height() > 16.0, "{b:?}");
        let row = RECT.top() + 16.0 * (i as f32 + 0.5);
        assert!((b.1.center().y - row).abs() < 1e-3, "{b:?}");
        assert!(b.4.contains_rect(b.1), "{b:?}");
        assert_eq!(b.0, ITEMS[i]);
        assert!(!b.3, "{b:?}");
    }
    // Wider than its cell: in full.
    let mut rig = Rig::new(Kind::ListIndex(Bar::Bottom), 0.0);
    rig.items = &ITEMS[..4];
    rig.frame(vec![]);
    let triangle = text_boxes(&rig.shapes)
        .into_iter()
        .find(|b| b.0 == "Triangle");
    let triangle = triangle.expect("Triangle drawn whole");
    assert!(triangle.1.width() > 24.0 && !triangle.3, "{triangle:?}");
}
