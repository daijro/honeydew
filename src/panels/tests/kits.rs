//! The leaf kinds of widget kits C and D draw through the kits' widgets, not placeholders: each test looks for a mark
//! only the real widget makes.

use eframe::egui::{self, Color32, Event, PointerButton, Rect, Shape};

use super::Rig;
use super::api::{RECT, no_problems};
use crate::panels::build::*;
use crate::panels::place;
use crate::theme::theme_by_id;

impl Rig {
    /// The shapes painted inside `rect` (their bounding rect in it), text left out.
    fn shapes_in(&self, rect: Rect) -> Vec<Shape> {
        self.shapes
            .iter()
            .map(|c| c.shape.clone())
            .filter(|s| !matches!(s, Shape::Text(_)))
            .filter(|s| rect.expand(0.5).contains_rect(s.visual_bounding_rect()))
            .collect()
    }

    /// The texts painted inside `rect`.
    fn texts_in(&self, rect: Rect) -> Vec<String> {
        self.texts()
            .into_iter()
            .filter(|(_, at)| rect.contains(at.center()))
            .map(|(t, _)| t)
            .collect()
    }
}

/// The fill of a filled shape: circles, rects and closed paths.
fn fill(shape: &Shape) -> Option<Color32> {
    match shape {
        Shape::Circle(c) => Some(c.fill),
        Shape::Rect(r) => Some(r.fill),
        Shape::Path(p) if p.closed => Some(p.fill),
        _ => None,
    }
}

fn colors() -> crate::theme::ThemeColors {
    theme_by_id(0).colors
}

#[test]
fn a_right_label_toggle_is_the_kit_c_pill_with_its_label_on_the_right() {
    // Flanger (type 13): `Inv. Mix` p11.
    let panel = Panel::new(hstack([toggle_right(9, "Inv. Mix").p(11)]));
    no_problems(13, &panel);
    let rig = Rig::new(13, panel, RECT);
    let (rect, label) = (rig.rect_of(9), rig.text_rect("Inv. Mix"));
    // The label starts 1.7 U from the left, vertically centred; the pill is centred 0.75 U from the left.
    assert!(
        (label.left() - (rect.left() + 1.7 * super::U)).abs() < 1.0,
        "{label:?}"
    );
    assert!(
        (label.center().y - rect.center().y).abs() < 1.0,
        "{label:?}"
    );
    let pill = rig.shapes_in(rect).into_iter().find_map(|s| match s {
        Shape::Rect(r) if r.fill == colors().control_fill.to_color32() => Some(r.rect),
        _ => None,
    });
    let pill = pill.expect("pill drawn");
    assert!(
        (pill.center().x - (rect.left() + 0.75 * super::U)).abs() < 1.0,
        "{pill:?}"
    );
}

#[test]
fn a_text_toggle_has_no_background_and_a_text_button_lights_while_held() {
    // Tape Stop (type 42): p5 as a text toggle, and an unbound momentary text button (`AUTO`).
    let panel = Panel::new(hstack([
        text_toggle(6, "Play").p(5),
        text_toggle(7, "AUTO").momentary(),
    ]));
    no_problems(42, &panel);
    let mut rig = Rig::new(42, panel, RECT);
    let text = colors().text.to_color32();
    for id in [6, 7] {
        assert!(
            rig.shapes_in(rig.rect_of(id)).is_empty(),
            "#{id}: no background"
        );
    }
    let color = |rig: &Rig, s: &str| {
        rig.text_shape(s).unwrap().galley.job.sections[0]
            .format
            .color
    };
    assert_eq!(color(&rig, "Play"), text);
    assert_eq!(color(&rig, "AUTO"), text.gamma_multiply(0.25));
    let at = rig.rect_of(7).center();
    rig.frame(vec![Event::PointerMoved(at)]);
    rig.press(at, PointerButton::Primary, true);
    rig.frame(vec![]);
    assert_eq!(color(&rig, "AUTO"), text);
    assert_eq!(rig.state.settings[&7], 1.0);
    rig.press(at, PointerButton::Primary, false);
    rig.frame(vec![]);
    assert_eq!(color(&rig, "AUTO"), text.gamma_multiply(0.25));
    assert_eq!(rig.state.settings[&7], 0.0);
}

#[test]
fn a_menu_button_draws_its_caption_and_its_box() {
    // As 3x Osc's `Chord`, here on Tape Stop's p4 menu: `Chord: ` in `text`, then a `controlFill` box.
    let panel = Panel::new(hstack([menu_button(8, "Chord").p(4)]));
    no_problems(42, &panel);
    let rig = Rig::new(42, panel, RECT);
    let caption = rig.text_shape("Chord:   ").expect("caption drawn");
    assert_eq!(
        caption.galley.job.sections[0].format.color,
        colors().text.to_color32()
    );
    let control_fill = colors().control_fill.to_color32();
    assert!(
        rig.shapes_in(rig.rect_of(8))
            .iter()
            .any(|s| fill(s) == Some(control_fill))
    );
}

#[test]
fn a_boxed_list_boxes_only_the_selected_item() {
    // MiniSynth FX sub-tabs, unbound, on `DELAY`.
    let panel = Panel::new(hstack([
        list(9, "DIST · DELAY · CHORUS", Bar::Boxed).default(1.0)
    ]));
    let rig = Rig::new(42, panel, RECT);
    let menu_fill = colors().menu_fill.to_color32();
    let rect = rig.rect_of(9);
    let boxes: Vec<Rect> = rig
        .shapes_in(rect)
        .into_iter()
        .filter_map(|s| match s {
            Shape::Rect(r) if r.fill == menu_fill => Some(r.rect),
            _ => None,
        })
        .collect();
    assert_eq!(boxes.len(), 1, "{boxes:?}");
    let third = rect.width() / 3.0;
    assert!(
        boxes[0].center().x > rect.left() + third
            && boxes[0].center().x < rect.left() + 2.0 * third
    );
}

#[test]
fn an_arc_draws_through_the_arc_slider() {
    // Tape Stop's left arcs: continuous p2 and 10-segment p4, mirrored from 135°, track `moduleFill`.
    let panel = Panel::new(hstack([
        arc(1, 135.0, 90.0, true, Ink::ModuleFill).p(2),
        arc(2, 135.0, 90.0, true, Ink::ModuleFill).p(4),
    ]));
    no_problems(42, &panel);
    let rig = Rig::new(42, panel, RECT);
    let module_fill = colors().module_fill.to_color32();
    // No label and no value strip: nothing written in either arc (a knob would show its value in the ring).
    for id in [1, 2] {
        assert!(rig.texts_in(rig.rect_of(id)).is_empty(), "#{id}");
    }
    // The segmented arc: ten `moduleFill` separators.
    let separators = rig
        .shapes_in(rig.rect_of(2))
        .iter()
        .filter(|s| matches!(s, Shape::Path(p) if p.closed && p.fill == module_fill))
        .count();
    assert_eq!(separators, 10);
}

#[test]
fn a_tag_draws_through_the_tag_slider() {
    // Tremolo's `Max` (type 46, p3): a channel tag at 20 % pointing right, with its label.
    let panel = Panel::new(hstack([tag(4, "Max", true).p(3)]));
    no_problems(46, &panel);
    let rig = Rig::new(46, panel, RECT);
    let tag_fill = Color32::RED.gamma_multiply(0.2);
    let rect = rig.rect_of(4);
    assert!(
        rig.shapes_in(rect)
            .iter()
            .any(|s| matches!(s, Shape::Path(p) if p.closed && p.fill == tag_fill))
    );
    assert!(rect.contains(rig.text_rect("Max").center()));
}

#[test]
fn a_graph_knob_plots_its_curve_and_shows_no_value() {
    // The Gate's `A. Curve` (type 27, p4).
    let panel = Panel::new(hstack([graph_knob(7, "A. Curve", false).p(4)]));
    no_problems(27, &panel);
    let rig = Rig::new(27, panel, RECT);
    let rect = rig.rect_of(7);
    assert_eq!(rig.texts_in(rect), ["A. Curve"]);
    // A 4 px open polyline in the channel colour.
    let curve = rig.shapes_in(rect).into_iter().any(|s| {
        matches!(s, Shape::Path(p) if !p.closed && p.stroke.width == 4.0
            && p.stroke.color == egui::epaint::ColorMode::Solid(Color32::RED))
    });
    assert!(curve);
}

#[test]
fn an_xy_pad_draws_through_kit_d() {
    // MultiFX (type 4): the pad's grid, 11 lines each way in `background`, and the crosshair.
    let panel = Panel::new(hstack([xy_pad(4, 2, 3)]));
    let rig = Rig::new(4, panel, RECT);
    let background = colors().background.to_color32();
    let shapes = rig.shapes_in(rig.rect_of(4));
    let grid = shapes
        .iter()
        .filter(|s| fill(s) == Some(background))
        .count();
    assert_eq!(grid, 22);
    assert!(
        shapes
            .iter()
            .filter(|s| fill(s) == Some(Color32::RED))
            .count()
            >= 3
    );
    // No label, no axis names (the Waveshaper's pads).
    assert!(rig.texts_in(rig.rect_of(4)).is_empty());
}

#[test]
fn the_multi_fx_pad_names_its_axes_for_the_fx_kind() {
    // The X and Y names in FontSize 1 and the channel colour, inside the pad.
    let mut rig = Rig::new(4, crate::panels::panel_for(4).unwrap().clone(), RECT);
    let pad = rig.rect_of(4);
    assert_eq!(rig.texts_in(pad), ["Feedback", "Filter"]);
    let t = rig.text_shape("Filter").unwrap();
    let format = &t.galley.job.sections[0].format;
    let size = super::metrics().font_id(crate::metrics::FontSize::Pt8).size;
    assert_eq!((format.font_id.size, format.color), (size, Color32::RED));
    rig.module.params[1] = crate::params::menu_value(6, 8);
    rig.frame(vec![]);
    assert_eq!(rig.texts_in(pad), ["Panning", "Distance"]);
}

#[test]
fn never_hides_a_part() {
    // The Tuner's `Reference` knob is hidden in the construction state.
    let panel = Panel::new(hstack([
        knob(4, "Reference").show(Show::Never),
        v_slider(5, "Reference"),
    ]));
    no_problems(24, &panel);
    let module = ModuleInstance::new(24, 2);
    let got = place(&panel, &module, &PanelState::new(&panel), RECT, super::U);
    assert_eq!(got.iter().map(|(l, _)| l.id).collect::<Vec<_>>(), [5]);
}

// ---- the parameters each kind passes on to its widget ----

/// The closed paths filled `color` inside `rect`.
fn polygons(rig: &Rig, rect: Rect, color: Color32) -> Vec<Vec<egui::Pos2>> {
    rig.shapes_in(rect)
        .into_iter()
        .filter_map(|s| match s {
            Shape::Path(p) if p.closed && p.fill == color => Some(p.points),
            _ => None,
        })
        .collect()
}

#[test]
fn a_tag_points_the_way_it_faces() {
    // Tremolo: `Max` points right at the display, `Min` left.
    let panel = Panel::new(hstack([
        tag(4, "Max", true).p(3),
        tag(6, "Min", false).p(4),
    ]));
    let rig = Rig::new(46, panel, RECT);
    for (id, faces_right) in [(4, true), (6, false)] {
        let rect = rig.rect_of(id);
        let tag = polygons(&rig, rect, Color32::RED.gamma_multiply(0.2));
        let [tag] = tag.as_slice() else {
            panic!("#{id}: {tag:?}")
        };
        // The tip alone touches the edge it faces; the back's two corners touch the other one.
        let (tip, back) = if faces_right {
            (rect.right(), rect.left())
        } else {
            (rect.left(), rect.right())
        };
        let on = |x: f32| tag.iter().filter(|p| (p.x - x).abs() < 1e-3).count();
        assert_eq!((on(tip), on(back)), (1, 2), "#{id}: {tag:?}");
    }
}

/// The angle of `p` around `center`, in degrees counter-clockwise from +x with y up, in [0, 360).
fn angle(center: egui::Pos2, p: egui::Pos2) -> f32 {
    (-(p.y - center.y))
        .atan2(p.x - center.x)
        .to_degrees()
        .rem_euclid(360.0)
}

#[test]
fn an_arc_puts_its_segments_where_start_span_and_mirroring_say() {
    // Tape Stop's synced `Start Time` (p4, 10 segments, default item 3), drawn four ways, one arc per panel: the
    // pointer reaches past the arc's rect.
    let module_fill = colors().module_fill.to_color32();
    let arc_rig = |start: f32, span: f32, mirrored: bool| {
        let panel = Panel::new(hstack(
            [arc(1, start, span, mirrored, Ink::ModuleFill).p(4)],
        ));
        Rig::new(42, panel, RECT)
    };
    let everywhere = Rect::EVERYTHING;
    // The pointer sits on the centre of segment 3, counted from the value-0 end: start + 3.5 segments, or for a
    // mirrored arc start + span − 3.5 segments.
    for (start, span, mirrored, want) in [
        (135.0, 90.0, true, 225.0 - 31.5),
        (135.0, 90.0, false, 135.0 + 31.5),
        (315.0, 90.0, false, 346.5),
        (135.0, 60.0, false, 135.0 + 21.0),
    ] {
        let rig = arc_rig(start, span, mirrored);
        let center = rig.rect_of(1).center();
        let pointers: Vec<_> = polygons(&rig, everywhere, Color32::RED)
            .into_iter()
            .filter(|p| p.len() == 3)
            .collect();
        let [pointer] = pointers.as_slice() else {
            panic!("{start} {span}: {pointers:?}")
        };
        let got = angle(center, pointer[0]);
        assert!(
            (got - want).abs() < 0.1,
            "{start}/{span}/{mirrored}: pointer at {got}°, want {want}°"
        );
    }
    // Separators are 2° wide and end on the segment boundaries, on the value-0 side of each boundary.
    let separators = |mirrored: bool| {
        let rig = arc_rig(135.0, 90.0, mirrored);
        let center = rig.rect_of(1).center();
        let mut mids: Vec<f32> = polygons(&rig, everywhere, module_fill)
            .iter()
            .filter(|p| p.len() == 3)
            .map(|p| (angle(center, p[1]) + angle(center, p[2])) / 2.0)
            .collect();
        mids.sort_by(f32::total_cmp);
        mids
    };
    let want = |first: f32| (0..10).map(|j| first + 9.0 * j as f32).collect::<Vec<_>>();
    let close = |a: &[f32], b: &[f32]| {
        a.len() == b.len() && a.iter().zip(b).all(|(a, b)| (a - b).abs() < 0.1)
    };
    assert!(
        close(&separators(false), &want(143.0)),
        "{:?}",
        separators(false)
    );
    assert!(
        close(&separators(true), &want(136.0)),
        "{:?}",
        separators(true)
    );
}

#[test]
fn a_graph_knob_plots_its_curve_rising_or_falling() {
    // The Gate's `A. Curve` rises from the bottom left; `R. Curve` falls from the top left.
    let panel = Panel::new(hstack([
        graph_knob(7, "A. Curve", false).p(4),
        graph_knob(11, "R. Curve", true).p(6),
    ]));
    let rig = Rig::new(27, panel, RECT);
    for (id, falling) in [(7, false), (11, true)] {
        let curve = rig
            .shapes_in(rig.rect_of(id))
            .into_iter()
            .find_map(|s| match s {
                Shape::Path(p) if !p.closed && p.stroke.width == 4.0 => Some(p.points),
                _ => None,
            });
        let curve = curve.unwrap_or_else(|| panic!("#{id}: no curve"));
        let (first, last) = (curve[0], curve[curve.len() - 1]);
        // Screen y grows downwards: a rising curve ends higher than it starts.
        assert_eq!(first.y < last.y, falling, "#{id}: {first:?} → {last:?}");
        assert!(first.x < last.x);
    }
}

#[test]
fn a_menu_button_shows_its_item_and_edits_its_param_or_setting() {
    let panel = Panel::new(vstack([
        // Bound: Tape Stop's p4 (default item 3, `1/2`), its popup in columns of 4 with `3/4` (4) disabled.
        menu_button(8, "Sync")
            .p(4)
            .items_per_column(4)
            .disabled_items(&[4]),
        // Unbound: three items, starting on the second.
        menu_button(9, "Chord")
            .items(&["Major", "Minor", "Sus"])
            .default(1.0),
    ]));
    no_problems(42, &panel);
    let mut rig = Rig::new(42, panel, RECT);
    assert!(rig.rect_of(8).contains(rig.text_rect("1/2").center()));
    assert!(rig.rect_of(9).contains(rig.text_rect("Minor").center()));
    // The bound popup: four rows, so `Off` (0) and `3/4` (4) share a row, `3/4` one column right; picking the
    // disabled `3/4` does nothing, `1 beat` (5) sets the param.
    rig.click(rig.rect_of(8).right_center() - egui::vec2(10.0, 0.0));
    rig.frame(vec![]);
    let (off, three_quarters) = (rig.text_rect("Off"), rig.text_rect("3/4"));
    assert!((off.center().y - three_quarters.center().y).abs() < 1.0);
    assert!(three_quarters.center().x > off.center().x);
    rig.click(three_quarters.center());
    assert!(rig.changed.is_empty());
    rig.frame(vec![]);
    rig.click(rig.text_rect("1 beat").center());
    assert_eq!(rig.module.params[4], crate::params::menu_value(5, 10));
    assert_eq!(rig.changed, [4]);
    // The unbound one keeps an item index: `Sus` is 2.
    rig.click(rig.rect_of(9).right_center() - egui::vec2(10.0, 0.0));
    rig.frame(vec![]);
    rig.click(rig.text_rect("Sus").center());
    assert_eq!(rig.state.settings[&9], 2.0);
    assert!(rig.changed.is_empty());
    assert!(rig.drawn("Sus"));
}

#[test]
fn text_toggles_and_right_labels_default_to_font_size_2() {
    let panel = Panel::new(hstack([
        text_toggle(6, "Play").p(5),
        toggle_right(5, "Sync").p(7),
        toggle(3, "Snap").p(7),
        text_toggle(7, "Small").p(5).font(FontSize::Pt8),
    ]));
    let rig = Rig::new(42, panel, RECT);
    let size = |s: &str| {
        rig.text_shape(s).unwrap().galley.job.sections[0]
            .format
            .font_id
            .size
    };
    let pt = |f| super::metrics().font_id(f).size;
    assert_eq!(size("Play"), pt(FontSize::Pt10));
    assert_eq!(size("Sync"), pt(FontSize::Pt10));
    // Other labels stay FontSize 1, and `.font()` overrides the default.
    assert_eq!(size("Snap"), pt(FontSize::Pt8));
    assert_eq!(size("Small"), pt(FontSize::Pt8));
}
