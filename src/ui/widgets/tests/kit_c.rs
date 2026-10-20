use std::cell::Cell;

use eframe::egui::Ui;

use super::*;

// Widget kit C: pill toggle variants, text toggles and buttons, the menu button, boxed lists and separators. U = 32,
// t = 2 throughout.

const T: f32 = 2.0;

/// A widget drawn once per frame.
type Widget<'a> = Box<dyn FnMut(&mut Ui, &UiCtx) -> Response + 'a>;

/// An offscreen context that runs one widget per frame. The widget's state lives in `Cell`s the test owns.
struct Bench<'a> {
    ctx: Context,
    time: f64,
    widget: Widget<'a>,
    changed: bool,
    clicked: bool,
    shapes: Vec<egui::epaint::ClippedShape>,
}

impl<'a> Bench<'a> {
    fn new(widget: impl FnMut(&mut Ui, &UiCtx) -> Response + 'a) -> Self {
        let mut bench = Self {
            ctx: Context::default(),
            time: 0.0,
            widget: Box::new(widget),
            changed: false,
            clicked: false,
            shapes: Vec::new(),
        };
        bench.frame(vec![]);
        bench
    }

    fn frame(&mut self, events: Vec<Event>) {
        self.time += 1.0 / 60.0;
        let raw = RawInput {
            screen_rect: Some(SCREEN),
            time: Some(self.time),
            events,
            ..Default::default()
        };
        let (theme, m) = (theme_by_id(0), metrics());
        let cx = UiCtx {
            theme: &theme,
            m: &m,
        };
        let widget = &mut self.widget;
        let mut flags = (false, false);
        let output = self.ctx.run(raw, |ctx| {
            egui::CentralPanel::default().show(ctx, |ui| {
                let response = widget(ui, &cx);
                flags = (response.changed(), response.clicked());
            });
        });
        (self.changed, self.clicked) = flags;
        self.shapes = output.shapes;
    }

    fn button(&mut self, pos: Pos2, button: PointerButton, pressed: bool) {
        self.frame(vec![Event::PointerButton {
            pos,
            button,
            pressed,
            modifiers: Modifiers::NONE,
        }]);
    }

    fn click_with(&mut self, pos: Pos2, button: PointerButton) {
        self.frame(vec![Event::PointerMoved(pos)]);
        self.button(pos, button, true);
        self.button(pos, button, false);
    }

    fn click(&mut self, pos: Pos2) {
        self.click_with(pos, PointerButton::Primary);
    }

    fn data_has(&self, id: Id) -> bool {
        self.ctx.data(|d| d.get_temp::<Anchor>(id)).is_some()
    }
}

fn colors() -> crate::theme::ThemeColors {
    theme_by_id(0).colors
}

/// Filled rects drawn inside `area`: fill, rect and corner radius.
fn fills_in(shapes: &[egui::epaint::ClippedShape], area: Rect) -> Vec<(Color32, Rect, f32)> {
    shapes
        .iter()
        .filter_map(|c| match &c.shape {
            egui::Shape::Rect(r)
                if area.contains_rect(r.rect) && r.fill != Color32::TRANSPARENT =>
            {
                Some((r.fill, r.rect, r.corner_radius.nw as f32))
            }
            _ => None,
        })
        .collect()
}

/// A text shape: text, colour, font size, where the galley starts and the clip rect.
type Run = (String, Color32, f32, Pos2, Rect);

fn text_runs(shapes: &[egui::epaint::ClippedShape]) -> Vec<Run> {
    shapes
        .iter()
        .filter_map(|c| match &c.shape {
            egui::Shape::Text(t) => Some((
                t.galley.text().to_owned(),
                t.fallback_color,
                t.galley.job.sections[0].format.font_id.size,
                t.pos,
                c.clip_rect,
            )),
            _ => None,
        })
        .collect()
}

fn pt_near(a: Pos2, b: Pos2) -> bool {
    (a - b).length() < 1e-3
}

// Pill toggle.

/// A label-right toggle at the 3.68 × 1.5 U of SuperSaw's `Pulse Mode`, at (100, 100).
const WIDE: Rect = Rect {
    min: pos2(100.0, 100.0),
    max: pos2(100.0 + 3.68 * U, 100.0 + 1.5 * U),
};

fn pill_bench<'a>(rect: Rect, label: &'a str, style: Pill, value: &'a Cell<f32>) -> Bench<'a> {
    pill_bench_with(rect, label, style, value, false)
}

/// [`pill_bench`] with the `On` / `Off` strip asked for or not.
fn pill_bench_with<'a>(
    rect: Rect,
    label: &'a str,
    style: Pill,
    value: &'a Cell<f32>,
    value_text: bool,
) -> Bench<'a> {
    Bench::new(move |ui, cx| {
        let c = Control {
            label_font: FontSize::Pt10,
            value_text,
            ..Control::new(Id::new("pill"), rect, label, Color32::RED, 0.0)
        };
        let mut v = value.get();
        let response = pill_toggle(ui, cx, &c, style, &mut v, |v| {
            if v >= 0.5 { "On" } else { "Off" }.to_owned()
        });
        value.set(v);
        response
    })
}

/// The texts of the last frame that were cut short with an ellipsis.
fn elided(shapes: &[egui::epaint::ClippedShape]) -> Vec<String> {
    shapes
        .iter()
        .filter_map(|c| match &c.shape {
            egui::Shape::Text(t) if t.galley.elided => Some(t.galley.text().to_owned()),
            _ => None,
        })
        .collect()
}

/// The pill (its rect and corner radius) and the dot (centre, radius, fill) of the last frame.
fn pill_and_dot(bench: &Bench, area: Rect) -> ((Rect, f32), (Pos2, f32, Color32)) {
    let pill = fills_in(&bench.shapes, area.expand(20.0))
        .into_iter()
        .find(|(fill, ..)| *fill == colors().control_fill.to_color32())
        .map(|(_, r, radius)| (r, radius))
        .expect("pill");
    let dot = circles(&bench.shapes)
        .into_iter()
        .map(|(fill, c, r)| (c, r, fill))
        .next()
        .expect("dot");
    (pill, dot)
}

#[test]
fn a_right_label_toggle_puts_the_pill_0_75_u_in_and_the_label_from_1_7_u() {
    let value = Cell::new(1.0);
    let style = Pill {
        label_right: true,
        ..Pill::default()
    };
    let bench = pill_bench(WIDE, "Pulse Mode", style, &value);
    // The pill: 10 t × 18 t, radius 5 t, centred at x + 0.75 U and the rect's vertical centre.
    let centre = pos2(100.0 + 0.75 * U, WIDE.center().y);
    let ((pill, radius), (dot, dot_r, dot_fill)) = pill_and_dot(&bench, WIDE);
    assert_eq!(
        pill,
        Rect::from_center_size(centre, vec2(10.0 * T, 18.0 * T))
    );
    assert_eq!(radius, 5.0 * T);
    // On: the dot 4 t above the centre, radius 2 t, in the channel colour.
    assert!(pt_near(dot, centre - vec2(0.0, 4.0 * T)), "{dot:?}");
    assert_eq!((dot_r, dot_fill), (2.0 * T, Color32::RED));
    // The label: 10 pt (20 at U = 32), channel, starting at x + 1.7 U, vertically centred, in full: `Pulse Mode`
    // (about 3.1 U) runs past the rect's right edge. No On / Off text.
    let texts = text_runs(&bench.shapes);
    assert_eq!(texts.len(), 1, "{texts:?}");
    let (text, color, size, pos, _) = &texts[0];
    assert_eq!(
        (text.as_str(), *color, *size),
        ("Pulse Mode", Color32::RED, 20.0)
    );
    assert!(elided(&bench.shapes).is_empty());
    assert!((pos.x - (100.0 + 1.7 * U)).abs() < 1e-3, "{pos:?}");
    let width = text_width(&bench, "Pulse Mode");
    assert!(pos.x + width > WIDE.right(), "{width}");
    let galley = bench
        .shapes
        .iter()
        .find_map(|c| match &c.shape {
            egui::Shape::Text(t) => Some(t.galley.rect.translate(t.pos.to_vec2())),
            _ => None,
        })
        .unwrap();
    assert!(
        (galley.center().y - WIDE.center().y).abs() < 1e-3,
        "{galley:?}"
    );
}

#[test]
fn a_right_label_toggle_reserves_no_on_off_strip() {
    let style = Pill {
        label_right: true,
        ..Pill::default()
    };
    for (rect, label, centre) in [
        (WIDE, "Retrig", pos2(100.0 + 0.75 * U, WIDE.center().y)),
        (RECT, "", RECT.center()),
    ] {
        let value = Cell::new(1.0);
        let bench = pill_bench_with(rect, label, style, &value, true);
        let ((pill, _), _) = pill_and_dot(&bench, rect);
        assert_eq!(pill.center(), centre, "{label:?}");
        let texts: Vec<_> = text_runs(&bench.shapes).into_iter().map(|r| r.0).collect();
        assert!(texts.iter().all(|t| t != "On"), "{texts:?}");
    }
}

#[test]
fn pill_geometry_holds_at_two_pixels_per_point() {
    // ppp 2: U = 64 px, t = ceil(64 / 16) = 4 px = 2 points.
    let theme = theme_by_id(0);
    let m = Metrics::from_pixels_per_point(2.0, DEFAULT_UI_SCALE);
    let (u, t) = (m.u(1.0), m.t());
    assert_eq!((m.u_px(), t * 2.0), (64.0, (m.u_px() / 16.0).ceil()));
    let cx = UiCtx {
        theme: &theme,
        m: &m,
    };
    let right = Rect::from_min_size(pos2(100.0, 100.0), vec2(3.68 * u, 1.5 * u));
    let bare = Rect::from_min_size(pos2(300.0, 100.0), vec2(2.08 * u, 2.0 * u));
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
                let style = Pill {
                    label_right: true,
                    ..Pill::default()
                };
                let c = Control {
                    label_font: FontSize::Pt10,
                    ..Control::new(Id::new("r"), right, "HQ", Color32::RED, 0.0)
                };
                pill_toggle(ui, &cx, &c, style, &mut 1.0, |_| String::new());
                let c = Control::new(Id::new("b"), bare, "", Color32::RED, 0.0);
                pill_toggle(ui, &cx, &c, Pill::default(), &mut 0.0, |_| String::new());
            });
        },
    );
    let control_fill = theme.colors.control_fill.to_color32();
    let pills: Vec<_> = output
        .shapes
        .iter()
        .filter_map(|c| match &c.shape {
            egui::Shape::Rect(r) if r.fill == control_fill => Some((r.rect, r.corner_radius.nw)),
            _ => None,
        })
        .collect();
    let right_centre = pos2(100.0 + 0.75 * u, right.center().y);
    let size = vec2(10.0 * t, 18.0 * t);
    assert_eq!(
        pills,
        [
            (Rect::from_center_size(right_centre, size), 10),
            (Rect::from_center_size(bare.center(), size), 10),
        ]
    );
    // The dots: on 4 t up in the channel colour, off 4 t down in menuFill, radius 2 t (4 px).
    assert_eq!(
        circles(&output.shapes),
        [
            (Color32::RED, right_centre - vec2(0.0, 4.0 * t), 2.0 * t),
            (
                theme.colors.menu_fill.to_color32(),
                bare.center() + vec2(0.0, 4.0 * t),
                2.0 * t
            ),
        ]
    );
    // The label hangs from x + 1.7 U, 20 points (40 px) high, in full.
    let texts = text_runs(&output.shapes);
    assert_eq!(texts.len(), 1);
    assert_eq!((texts[0].0.as_str(), texts[0].2), ("HQ", 20.0));
    assert!((texts[0].3.x - (100.0 + 1.7 * u)).abs() < 1e-3);
    assert!(elided(&output.shapes).is_empty());
}

#[test]
fn a_toggle_without_a_label_centres_the_pill_in_its_rect_in_either_style() {
    for label_right in [false, true] {
        let value = Cell::new(0.0);
        let style = Pill {
            label_right,
            ..Pill::default()
        };
        let bench = pill_bench(RECT, "", style, &value);
        let ((pill, _), (dot, _, fill)) = pill_and_dot(&bench, RECT);
        assert_eq!(pill.center(), RECT.center(), "{label_right}");
        // Off: 4 t below the centre, menuFill.
        assert!(pt_near(dot, RECT.center() + vec2(0.0, 4.0 * T)));
        assert_eq!(fill, colors().menu_fill.to_color32());
        assert!(text_runs(&bench.shapes).is_empty());
    }
}

#[test]
fn the_label_above_style_is_the_effect_toggle() {
    let value = Cell::new(1.0);
    let bench = pill_bench(RECT, "Sync", Pill::default(), &value);
    // The label in the top 0.75 U strip, the pill centred in the rest.
    let ((pill, _), _) = pill_and_dot(&bench, RECT);
    assert_eq!(
        pill.center(),
        pos2(
            RECT.center().x,
            RECT.top() + 0.75 * U + (RECT.height() - 0.75 * U) / 2.0
        )
    );
    let texts = text_runs(&bench.shapes);
    assert_eq!(texts.len(), 1);
    assert_eq!(texts[0].0, "Sync");
}

#[test]
fn an_inverted_toggle_shows_on_for_a_value_below_half_and_still_flips_on_click() {
    let value = Cell::new(0.0);
    let style = Pill {
        inverted: true,
        ..Pill::default()
    };
    let mut bench = pill_bench(RECT, "", style, &value);
    // Not muted (0): dot up, channel.
    let (_, (dot, _, fill)) = pill_and_dot(&bench, RECT);
    assert!(pt_near(dot, RECT.center() - vec2(0.0, 4.0 * T)));
    assert_eq!(fill, Color32::RED);
    // A click flips the value as for any toggle: muted (1), dot down, menuFill.
    bench.click(RECT.center());
    assert_eq!(value.get(), 1.0);
    assert!(bench.changed);
    bench.frame(vec![]);
    let (_, (dot, _, fill)) = pill_and_dot(&bench, RECT);
    assert!(pt_near(dot, RECT.center() + vec2(0.0, 4.0 * T)));
    assert_eq!(fill, colors().menu_fill.to_color32());
    // NaN reads as the default (0): shown on, and left as it is.
    value.set(f32::NAN);
    bench.frame(vec![]);
    let (_, (_, _, fill)) = pill_and_dot(&bench, RECT);
    assert_eq!(fill, Color32::RED);
    assert!(value.get().is_nan());
}

#[test]
fn pill_toggles_flip_on_primary_clicks_only() {
    let value = Cell::new(0.0);
    let style = Pill {
        label_right: true,
        ..Pill::default()
    };
    let mut bench = pill_bench(WIDE, "Retrig", style, &value);
    // The whole rect takes input, the label too.
    bench.click(pos2(WIDE.right() - 10.0, WIDE.center().y));
    assert_eq!(value.get(), 1.0);
    for button in [PointerButton::Secondary, PointerButton::Middle] {
        bench.click_with(WIDE.center(), button);
        assert_eq!(value.get(), 1.0, "{button:?}");
    }
    // The secondary click opened the parameter menu.
    assert!(bench.data_has(menu_id(Id::new("pill"))));
}

#[test]
fn a_right_label_is_drawn_in_full_from_1_7_u_whatever_the_rect() {
    let value = Cell::new(0.0);
    let style = Pill {
        label_right: true,
        ..Pill::default()
    };
    let label = "A label far longer than any strip";
    let narrow = Rect::from_min_size(pos2(100.0, 100.0), vec2(1.0 * U, 1.5 * U));
    for rect in [
        WIDE,
        narrow,
        Rect::from_min_size(pos2(100.0, 100.0), Vec2::ZERO),
    ] {
        let bench = pill_bench(rect, label, style, &value);
        let texts = text_runs(&bench.shapes);
        assert_eq!(texts.len(), 1, "{texts:?}");
        assert_eq!(texts[0].0, label);
        assert!((texts[0].3.x - (rect.left() + 1.7 * U)).abs() < 1e-3);
        assert!(elided(&bench.shapes).is_empty(), "{rect:?}");
        assert!(
            bench
                .shapes
                .iter()
                .all(|c| c.shape.visual_bounding_rect().is_finite())
        );
    }
}

// Text toggle and text button.

/// A text toggle or (with `None`) a text button `Reverse` in `RECT`, 10 pt.
fn text_bench<'a>(value: Option<&'a Cell<f32>>, disabled: bool) -> Bench<'a> {
    Bench::new(move |ui, cx| {
        let c = Control {
            label_font: FontSize::Pt10,
            disabled,
            ..Control::new(Id::new("text"), RECT, "Reverse", Color32::RED, 0.0)
        };
        match value {
            Some(value) => {
                let mut v = value.get();
                let response = text_toggle(ui, cx, &c, &mut v);
                value.set(v);
                response
            }
            None => text_button(ui, cx, &c),
        }
    })
}

/// The one text of the last frame, `Reverse` at 10 pt in full and centred in the rect (the 0.2 U inset is the
/// same on both sides, so it leaves the centre where it is): its colour.
fn the_text(bench: &Bench) -> Color32 {
    let texts = text_runs(&bench.shapes);
    assert_eq!(texts.len(), 1, "{texts:?}");
    assert_eq!(texts[0].0, "Reverse");
    assert_plain(&text_boxes(&bench.shapes)[0], 20.0, RECT.center());
    texts[0].1
}

#[test]
fn a_text_toggle_is_text_when_on_and_a_quarter_alpha_when_off_with_no_background() {
    let text = colors().text.to_color32();
    let value = Cell::new(1.0);
    let mut bench = text_bench(Some(&value), false);
    assert_eq!(the_text(&bench), text);
    assert!(fills_in(&bench.shapes, RECT).is_empty());
    assert!(circles(&bench.shapes).is_empty());
    bench.click(RECT.center());
    assert_eq!(value.get(), 0.0);
    assert!(bench.changed);
    bench.frame(vec![]);
    assert_eq!(the_text(&bench), text.gamma_multiply(0.25));
    // NaN draws as the default (off) and is left alone.
    value.set(f32::NAN);
    bench.frame(vec![]);
    assert_eq!(the_text(&bench), text.gamma_multiply(0.25));
    assert!(value.get().is_nan());
}

#[test]
fn a_text_toggle_flips_on_primary_clicks_and_opens_the_parameter_menu_on_secondary() {
    let value = Cell::new(0.0);
    let mut bench = text_bench(Some(&value), false);
    bench.click(RECT.center());
    assert_eq!(value.get(), 1.0);
    bench.click_with(RECT.center(), PointerButton::Middle);
    bench.click_with(RECT.center(), PointerButton::Secondary);
    assert_eq!(value.get(), 1.0);
    assert!(bench.data_has(menu_id(Id::new("text"))));
    // Disabled: neither.
    let value = Cell::new(0.0);
    let mut bench = text_bench(Some(&value), true);
    bench.click(RECT.center());
    bench.click_with(RECT.center(), PointerButton::Secondary);
    assert_eq!(value.get(), 0.0);
    assert!(!bench.data_has(menu_id(Id::new("text"))));
}

#[test]
fn a_text_button_is_dim_at_rest_lit_while_pressed_and_reports_the_press() {
    let text = colors().text.to_color32();
    let dim = text.gamma_multiply(0.25);
    let mut bench = text_bench(None, false);
    assert_eq!(the_text(&bench), dim);
    // Press: lit, no click yet.
    bench.frame(vec![Event::PointerMoved(RECT.center())]);
    bench.button(RECT.center(), PointerButton::Primary, true);
    bench.frame(vec![]);
    assert_eq!(the_text(&bench), text);
    assert!(!bench.clicked);
    // Release: the click, and dim again.
    bench.button(RECT.center(), PointerButton::Primary, false);
    assert!(bench.clicked);
    bench.frame(vec![]);
    assert_eq!(the_text(&bench), dim);
    assert!(!bench.clicked);
    // Other buttons neither light it nor click it, and it opens no menu (no parameter).
    for button in [PointerButton::Secondary, PointerButton::Middle] {
        bench.button(RECT.center(), button, true);
        bench.frame(vec![]);
        assert_eq!(the_text(&bench), dim, "{button:?}");
        bench.button(RECT.center(), button, false);
        assert!(!bench.clicked, "{button:?}");
    }
    assert!(!bench.data_has(menu_id(Id::new("text"))));
    // Disabled: no press, no click.
    let mut bench = text_bench(None, true);
    bench.frame(vec![Event::PointerMoved(RECT.center())]);
    bench.button(RECT.center(), PointerButton::Primary, true);
    bench.frame(vec![]);
    assert_eq!(the_text(&bench), dim);
    bench.button(RECT.center(), PointerButton::Primary, false);
    assert!(!bench.clicked);
}

#[test]
fn text_toggles_and_buttons_draw_long_text_in_full_and_survive_empty_rects() {
    for rect in [RECT, Rect::from_min_size(pos2(100.0, 100.0), Vec2::ZERO)] {
        let value = Cell::new(1.0);
        let mut bench = Bench::new(|ui, cx| {
            let c = Control::new(
                Id::new("t"),
                rect,
                "A text toggle wider than 3 U",
                Color32::RED,
                0.0,
            );
            let mut v = value.get();
            text_toggle(ui, cx, &c, &mut v);
            text_button(
                ui,
                cx,
                &Control {
                    id: Id::new("b"),
                    ..c
                },
            )
        });
        bench.frame(vec![]);
        let boxes = text_boxes(&bench.shapes);
        assert_eq!(boxes.len(), 2);
        for b in &boxes {
            assert!(b.1.width() > RECT.width(), "{b:?}");
            assert_plain(b, metrics().font_id(FontSize::Pt8).size, rect.center());
        }
        assert!(
            bench
                .shapes
                .iter()
                .all(|c| c.shape.visual_bounding_rect().is_finite())
        );
    }
}

// Menu button.

/// 3x Osc `Chord`: 8 × 1.5 U at (100, 100).
const CHORD: Rect = Rect {
    min: pos2(100.0, 100.0),
    max: pos2(100.0 + 8.0 * U, 100.0 + 1.5 * U),
};

fn menu_bench<'a>(rect: Rect, label: &'a str, value: &'a Cell<f32>, disabled: bool) -> Bench<'a> {
    Bench::new(move |ui, cx| {
        let c = Control {
            disabled,
            ..Control::new(Id::new("menu"), rect, label, Color32::RED, 0.0)
        };
        let mut v = value.get();
        let response = menu_button(ui, cx, &c, &ITEMS[..5], &mut v);
        value.set(v);
        response
    })
}

/// The caption's run, the value's run and the box (the `controlFill` rect) of the last frame.
fn menu_parts(bench: &Bench) -> (Run, Run, Rect) {
    let texts = text_runs(&bench.shapes);
    assert_eq!(texts.len(), 2, "{texts:?}");
    let fills = fills_in(&bench.shapes, CHORD);
    assert_eq!(fills.len(), 1, "{fills:?}");
    let (fill, rect, radius) = fills[0];
    assert_eq!((fill, radius), (colors().control_fill.to_color32(), 0.0));
    (texts[0].clone(), texts[1].clone(), rect)
}

/// The width of `text` at 10 pt (20 points at U = 32).
fn text_width(bench: &Bench, text: &str) -> f32 {
    let font = metrics().font_id(FontSize::Pt10);
    bench
        .ctx
        .fonts_mut(|f| f.layout_no_wrap(text.to_owned(), font, Color32::WHITE))
        .size()
        .x
}

#[test]
fn a_menu_button_draws_its_caption_then_a_control_fill_box_with_the_value() {
    let value = Cell::new(0.5);
    let bench = menu_bench(CHORD, "Chord", &value, false);
    let (caption, item, boxed) = menu_parts(&bench);
    // `<label>:   ` in text, 10 pt, from the left edge and vertically centred.
    assert_eq!(
        (caption.0.as_str(), caption.1, caption.2),
        ("Chord:   ", colors().text.to_color32(), 20.0)
    );
    assert!((caption.3.x - CHORD.left()).abs() < 1e-3, "{:?}", caption.3);
    // The box starts right after the caption (its three spaces included) and fills the rest of the rect.
    let width = text_width(&bench, "Chord:   ");
    assert!(width > text_width(&bench, "Chord:") + 1.0);
    assert_eq!(
        boxed,
        Rect::from_x_y_ranges(CHORD.left() + width..=CHORD.right(), CHORD.y_range())
    );
    // The value: the current item, 10 pt channel, centred in the box inset 0.2 U.
    assert_eq!(
        (item.0.as_str(), item.1, item.2),
        ("Square", Color32::RED, 20.0)
    );
    let boxes = text_boxes(&bench.shapes);
    assert_plain(&boxes[1], 20.0, boxed.center());
}

#[test]
fn a_tap_on_the_box_opens_the_spinner_popup_and_a_pick_sets_the_value() {
    let value = Cell::new(0.0);
    let mut bench = menu_bench(CHORD, "Chord", &value, false);
    let (.., boxed) = menu_parts(&bench);
    // The caption is not part of the button.
    bench.click(pos2(CHORD.left() + 5.0, CHORD.center().y));
    assert!(!bench.data_has(popup_id(Id::new("menu"))));
    bench.click(boxed.center());
    assert!(bench.data_has(popup_id(Id::new("menu"))));
    bench.frame(vec![]);
    let popup = bench
        .ctx
        .memory(|m| m.area_rect(popup_id(Id::new("menu"))))
        .expect("popup on screen");
    // Row 3 is "Triangle": item 3 of 5.
    bench.click(pos2(popup.left() + 10.0, popup.top() + 3.5 * ROW));
    assert_eq!(value.get(), menu_value(3, 5));
    assert!(bench.changed);
    bench.frame(vec![]);
    assert!(!bench.data_has(popup_id(Id::new("menu"))));
    assert_eq!(menu_parts(&bench).1.0, "Triangle");
}

#[test]
fn a_menu_button_reads_nan_as_the_first_item_and_ignores_input_when_disabled() {
    let value = Cell::new(f32::NAN);
    let mut bench = menu_bench(CHORD, "Chord", &value, true);
    assert_eq!(menu_parts(&bench).1.0, "Sine");
    let (.., boxed) = menu_parts(&bench);
    for button in [
        PointerButton::Primary,
        PointerButton::Secondary,
        PointerButton::Middle,
    ] {
        bench.click_with(boxed.center(), button);
    }
    assert!(!bench.data_has(popup_id(Id::new("menu"))));
    assert!(value.get().is_nan());
}

#[test]
fn a_menu_button_draws_its_texts_in_full_whatever_its_rect() {
    let value = Cell::new(1.0);
    let narrow = Rect::from_min_size(pos2(100.0, 100.0), vec2(2.0 * U, 1.5 * U));
    let empty = Rect::from_min_size(pos2(100.0, 100.0), Vec2::ZERO);
    for rect in [CHORD, narrow, empty] {
        let bench = menu_bench(rect, "A caption wider than 2 U", &value, false);
        let boxes = text_boxes(&bench.shapes);
        assert_eq!(boxes.len(), 2, "{rect:?}");
        // The caption hangs from the left edge; the value is centred in what is left of the rect.
        assert_eq!(boxes[0].0, "A caption wider than 2 U:   ");
        assert!(
            (boxes[0].1.left_center() - rect.left_center()).length() < 1e-3,
            "{rect:?}"
        );
        assert!(boxes[0].1.width() > 2.0 * U && !boxes[0].3, "{rect:?}");
        assert!(
            boxes.iter().all(|b| b.4.contains_rect(b.1)),
            "{rect:?} {boxes:?}"
        );
        assert_eq!(boxes[1].0, "Noise");
        assert!(
            bench
                .shapes
                .iter()
                .all(|c| c.shape.visual_bounding_rect().is_finite())
        );
    }
}

// Boxed list.

/// MiniSynth's FX sub-tabs: 4 items in 8 × 1.25 U at (100, 100), 64 points a cell.
const TABS: Rect = Rect {
    min: pos2(100.0, 100.0),
    max: pos2(100.0 + 8.0 * U, 100.0 + 1.25 * U),
};
const FX: [&str; 4] = ["DIST", "DELAY", "CHORUS", "PHASER"];

#[test]
fn a_boxed_list_boxes_the_selected_cell_in_menu_fill_inset_2t_and_draws_no_bars() {
    let index = Cell::new(1usize);
    let mut bench = Bench::new(|ui, cx| {
        let list = ItemList::new(Id::new("fx"), TABS, &FX, Bar::Boxed, Color32::RED);
        let mut i = index.get();
        let response = item_list_index(ui, cx, &list, &mut i);
        index.set(i);
        response
    });
    let cell = |i: usize| {
        Rect::from_x_y_ranges(
            TABS.left() + 64.0 * i as f32..=TABS.left() + 64.0 * (i + 1) as f32,
            TABS.y_range(),
        )
    };
    let menu_fill = colors().menu_fill.to_color32();
    assert_eq!(
        fills_in(&bench.shapes, TABS),
        [(menu_fill, cell(1).shrink(2.0 * T), 0.0)]
    );
    // Selected text in `text`, the rest in the channel colour, each centred in its box.
    let texts = text_runs(&bench.shapes);
    let colours: Vec<_> = texts.iter().map(|r| (r.0.as_str(), r.1)).collect();
    let text = colors().text.to_color32();
    assert_eq!(
        colours,
        [
            ("DIST", Color32::RED),
            ("DELAY", text),
            ("CHORUS", Color32::RED),
            ("PHASER", Color32::RED)
        ]
    );
    for (i, b) in text_boxes(&bench.shapes).iter().enumerate() {
        assert!((b.1.center() - cell(i).center()).length() < 1e-3, "{b:?}");
        assert!(b.4.contains_rect(b.1), "{b:?}");
    }
    // A click moves the box.
    bench.click(cell(3).center());
    assert_eq!(index.get(), 3);
    assert!(bench.changed);
    bench.frame(vec![]);
    assert_eq!(
        fills_in(&bench.shapes, TABS),
        [(menu_fill, cell(3).shrink(2.0 * T), 0.0)]
    );
}

#[test]
fn a_boxed_list_too_small_for_its_box_draws_no_inverted_rect() {
    for rect in [
        Rect::from_min_size(pos2(100.0, 100.0), vec2(12.0, 6.0)),
        Rect::from_min_size(pos2(100.0, 100.0), Vec2::ZERO),
    ] {
        let bench = Bench::new(|ui, cx| {
            let list = ItemList::new(Id::new("fx"), rect, &FX, Bar::Boxed, Color32::RED);
            item_list_index(ui, cx, &list, &mut 0)
        });
        for c in &bench.shapes {
            if let egui::Shape::Rect(r) = &c.shape {
                assert!(r.rect.width() >= 0.0 && r.rect.height() >= 0.0, "{r:?}");
            }
            assert!(c.shape.visual_bounding_rect().is_finite());
        }
    }
}

// Rect elements.

#[test]
fn a_separator_fills_its_rect_with_text_at_a_tenth_alpha() {
    // 3x Osc Levels: a 0.1 U wide column.
    let rect = Rect::from_min_size(pos2(100.0, 100.0), vec2(0.1 * U, 4.0 * U));
    let bench = Bench::new(|ui, cx| {
        separator(ui, cx, rect);
        ui.interact(rect, Id::new("sep"), Sense::hover())
    });
    let ink = colors().text.to_color32().gamma_multiply(0.1);
    assert_eq!(fills_in(&bench.shapes, rect), [(ink, rect, 0.0)]);
    assert!(text_runs(&bench.shapes).is_empty());
}

#[test]
fn a_pill_bound_to_no_parameter_has_no_menu() {
    // Toggles bound to no parameter (the settings) show nothing; nor does one with no menu at all.
    for menu in [ParamMenu::Unbound, ParamMenu::Off] {
        let value = Cell::new(0.0);
        let mut bench = Bench::new(|ui, cx| {
            let c = Control {
                menu,
                ..Control::new(Id::new("pill"), RECT, "", Color32::RED, 0.0)
            };
            let mut v = value.get();
            let response = pill_toggle(ui, cx, &c, Pill::default(), &mut v, |_| String::new());
            value.set(v);
            response
        });
        bench.click_with(RECT.center(), PointerButton::Secondary);
        assert!(!bench.data_has(menu_id(Id::new("pill"))), "{menu:?}");
        assert_eq!(value.get(), 0.0);
        // Primary clicks still flip it.
        bench.click(RECT.center());
        assert_eq!(value.get(), 1.0);
    }
}

// Generic toggles draw on for any value but 0, and a tap writes 1 from exactly 0, else 0.

#[test]
fn a_pill_toggle_is_on_for_any_value_but_0_and_a_tap_writes_1_only_from_0() {
    for inverted in [false, true] {
        let style = Pill {
            inverted,
            ..Pill::default()
        };
        let value = Cell::new(0.0);
        let mut bench = pill_bench(RECT, "", style, &value);
        for (v, on) in [(0.0, false), (0.001, true), (0.5, true), (1.0, true)] {
            value.set(v);
            bench.frame(vec![]);
            let (_, (dot, _, fill)) = pill_and_dot(&bench, RECT);
            // The inverted flag swaps the sides: the dot is up (channel) while off.
            let (dy, ink) = if on != inverted {
                (-4.0 * T, Color32::RED)
            } else {
                (4.0 * T, colors().menu_fill.to_color32())
            };
            let at = format!("{v}, inverted {inverted}");
            assert!(pt_near(dot, RECT.center() + vec2(0.0, dy)), "{at}");
            assert_eq!(fill, ink, "{at}");
        }
        for (from, to) in [(0.5, 0.0), (0.0, 1.0), (0.001, 0.0)] {
            value.set(from);
            bench.click(RECT.center());
            assert_eq!(value.get(), to, "a tap from {from}, inverted {inverted}");
        }
    }
}

#[test]
fn a_text_toggle_is_lit_for_any_value_but_0_and_a_tap_writes_1_only_from_0() {
    let text = colors().text.to_color32();
    let value = Cell::new(0.0);
    let mut bench = text_bench(Some(&value), false);
    for (v, lit) in [(0.0, false), (0.001, true), (0.5, true), (1.0, true)] {
        value.set(v);
        bench.frame(vec![]);
        let want = if lit { text } else { text.gamma_multiply(0.25) };
        assert_eq!(the_text(&bench), want, "{v}");
    }
    for (from, to) in [(0.5, 0.0), (0.0, 1.0)] {
        value.set(from);
        bench.click(RECT.center());
        assert_eq!(value.get(), to, "a tap from {from}");
    }
}
