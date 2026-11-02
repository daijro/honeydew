//! Widget kit D in an offscreen context: arc slider, tag slider, graph knob and XY pad.

use eframe::egui::epaint::{ClippedShape, PathShape};

use super::*;

#[derive(Clone, Copy, Debug)]
enum Kit {
    Arc(ArcSlider),
    /// Faces right.
    Tag(bool),
    /// Falling.
    Graph(bool),
    Pad,
}

/// Tape Stop's arcs (`moduleFill` is not needed: the track only punches out).
const LEFT: ArcSlider = ArcSlider {
    start: 135.0,
    span: 90.0,
    mirrored: true,
    track: Color32::BLUE,
};
const RIGHT: ArcSlider = ArcSlider {
    start: 315.0,
    span: 90.0,
    mirrored: false,
    track: Color32::BLUE,
};

const KITS: [Kit; 6] = [
    Kit::Arc(LEFT),
    Kit::Arc(RIGHT),
    Kit::Tag(true),
    Kit::Tag(false),
    Kit::Graph(false),
    Kit::Pad,
];

struct KitRig {
    ctx: Context,
    kit: Kit,
    rect: Rect,
    /// The value, or the pad's X.
    value: f32,
    /// The pad's Y.
    y: f32,
    default: f32,
    steps: Option<usize>,
    label: &'static str,
    value_text: bool,
    disabled: bool,
    ppp: f32,
    time: f64,
    changed: bool,
    shapes: Vec<ClippedShape>,
    /// The last frame tessellated, as device-pixel vertex positions.
    vertices: Vec<Pos2>,
}

impl KitRig {
    fn new(kit: Kit, value: f32) -> Self {
        Self::with(kit, value, |_| {})
    }

    /// A rig changed by `setup` before its first frame.
    fn with(kit: Kit, value: f32, setup: impl FnOnce(&mut Self)) -> Self {
        let mut rig = Self {
            ctx: Context::default(),
            kit,
            rect: RECT,
            value,
            y: value,
            default: 0.0,
            steps: None,
            label: "",
            value_text: false,
            disabled: false,
            ppp: 1.0,
            time: 0.0,
            changed: false,
            shapes: Vec::new(),
            vertices: Vec::new(),
        };
        setup(&mut rig);
        rig.frame(vec![]);
        rig
    }

    fn frame(&mut self, events: Vec<Event>) {
        self.time += 1.0 / 60.0;
        let mut raw = RawInput {
            screen_rect: Some(SCREEN),
            time: Some(self.time),
            events,
            ..Default::default()
        };
        raw.viewports
            .entry(raw.viewport_id)
            .or_default()
            .native_pixels_per_point = Some(self.ppp);
        let theme = theme_by_id(0);
        let c = Control {
            steps: self.steps,
            value_text: self.value_text,
            disabled: self.disabled,
            ..Control::new(
                Id::new("w"),
                self.rect,
                self.label,
                Color32::RED,
                self.default,
            )
        };
        let (kit, value, y) = (self.kit, &mut self.value, &mut self.y);
        let mut changed = false;
        let output = self.ctx.run(raw, |ctx| {
            let m = Metrics::new(ctx, DEFAULT_UI_SCALE);
            let cx = UiCtx {
                theme: &theme,
                m: &m,
            };
            // No panel fill: only the widget is painted.
            egui::CentralPanel::default()
                .frame(egui::Frame::NONE)
                .show(ctx, |ui| {
                    let display = |v: f32| format!("{v:.2}");
                    let response = match kit {
                        Kit::Arc(arc) => arc_slider(ui, &cx, &c, &arc, value, display),
                        Kit::Tag(right) => tag_slider(ui, &cx, &c, value, right, display),
                        Kit::Graph(falling) => graph_knob(ui, &cx, &c, value, falling),
                        Kit::Pad => xy_pad(ui, &cx, &c, value, y),
                    };
                    changed = response.changed();
                });
        });
        self.changed = changed;
        let ppp = output.pixels_per_point;
        self.vertices = self
            .ctx
            .tessellate(output.shapes.clone(), ppp)
            .into_iter()
            .flat_map(|p| match p.primitive {
                egui::epaint::Primitive::Mesh(mesh) => mesh.vertices,
                _ => Vec::new(),
            })
            .map(|v| (v.pos.to_vec2() * ppp).to_pos2())
            .collect();
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

    /// Presses `button` at the control's centre, moves through `offsets` (from the press point), releases. A
    /// separate gesture: it starts a double-tap window after the last.
    fn drag_with(&mut self, button: PointerButton, offsets: &[Vec2]) {
        self.time += DOUBLE_TAP_S;
        let start = self.rect.center();
        self.frame(vec![Event::PointerMoved(start)]);
        self.button(start, button, true);
        for &d in offsets {
            self.frame(vec![Event::PointerMoved(start + d)]);
        }
        let end = start + offsets.last().copied().unwrap_or_default();
        self.button(end, button, false);
    }

    fn drag(&mut self, offsets: &[Vec2]) {
        self.drag_with(PointerButton::Primary, offsets);
    }
}

/// Geometry computed two ways in f32 agrees to a thousandth of a point.
fn close(a: f32, b: f32) -> bool {
    (a - b).abs() < 1e-3
}

/// The filled paths (sectors, tags, pointers) and the open ones (curves) of a frame.
fn paths(shapes: &[ClippedShape]) -> Vec<PathShape> {
    shapes
        .iter()
        .filter_map(|c| match &c.shape {
            egui::Shape::Path(p) => Some(p.clone()),
            _ => None,
        })
        .collect()
}

/// The angle of `p` around `center`, in degrees counter-clockwise from +x with y up, in [0, 360).
fn angle_of(center: Pos2, p: Pos2) -> f32 {
    let d = p - center;
    (-d.y).atan2(d.x).to_degrees().rem_euclid(360.0)
}

/// The arc's discs other than the track-colour punch-out: the handle (and the wash while pressed).
fn handle(shapes: &[ClippedShape]) -> Vec<(Color32, Pos2, f32)> {
    circles(shapes)
        .into_iter()
        .filter(|c| c.0 != Color32::BLUE)
        .collect()
}

/// The inverse of the arc's value-to-angle mapping: how far along `arc` the angle `deg` is, in [0, 1].
fn arc_value(arc: &ArcSlider, deg: f32) -> f32 {
    let along = if arc.mirrored {
        arc.start + arc.span - deg
    } else {
        deg - arc.start
    };
    let along = along.rem_euclid(360.0);
    // A hair below 0 must not wrap to the far side.
    let along = if along > 359.0 { along - 360.0 } else { along };
    along / arc.span
}

// Arc slider.

#[test]
fn the_arc_handle_sits_at_the_value_and_reads_back_mirrored_or_not() {
    let t = metrics().t();
    for arc in [LEFT, RIGHT] {
        for v in [0.0, 0.25, 0.5, 1.0] {
            let rig = KitRig::new(Kit::Arc(arc), v);
            // The handle: a controlFill disc under a channel one, both on the ring's centre line.
            let handle = handle(&rig.shapes);
            assert_eq!(handle.len(), 2, "{handle:?}");
            let (fill, at, r) = handle[1];
            assert_eq!(fill, Color32::RED);
            assert_eq!(handle[0].1, at);
            // D = 96, th = t + (min(4.8, 2t) − t)·v, Rm = 0.48 D − 2 th.
            let th = t + t * v;
            assert!(close(at.distance(RECT.center()), 46.08 - 2.0 * th));
            assert!(close(r, 1.5 * 2.0 * t) && close(handle[0].2, 3.0 * t + 2.0 * t));
            let back = arc_value(&arc, angle_of(RECT.center(), at));
            assert!((back - v).abs() < 1e-4, "{arc:?} {v}: {back}");
        }
    }
}

#[test]
fn the_left_arc_rises_on_the_left_and_the_right_arc_on_the_right() {
    for (arc, left_side) in [(LEFT, true), (RIGHT, false)] {
        let at = |v| handle(&KitRig::new(Kit::Arc(arc), v).shapes)[1].1;
        let (low, high) = (at(0.0), at(1.0));
        assert!(high.y < low.y, "{arc:?}");
        assert_eq!(low.x < RECT.center().x, left_side);
        assert_eq!(high.x < RECT.center().x, left_side);
    }
}

#[test]
fn arcs_follow_vertical_drags_up_increasing() {
    for arc in [LEFT, RIGHT] {
        // The knob's law: 0.24 of the range per 192-point inch.
        let mut rig = KitRig::new(Kit::Arc(arc), 0.0);
        rig.drag(&[vec2(0.0, -10.0), vec2(0.0, -200.0)]);
        assert!(close(rig.value, 0.25), "{}", rig.value);
        // Horizontal moves do nothing.
        rig.drag(&[vec2(10.0, 0.0), vec2(2.0 * U, 0.0)]);
        assert!(close(rig.value, 0.25), "{}", rig.value);
    }
}

#[test]
fn the_arc_fill_runs_from_the_value_0_end() {
    let control_fill = theme_by_id(0).colors.control_fill.to_color32();
    // At 0.5: the left arc fills 225° → 180°, the right one 315° → 360°.
    for (arc, from, to) in [(LEFT, 225.0, 180.0), (RIGHT, 315.0, 360.0)] {
        let rig = KitRig::new(Kit::Arc(arc), 0.5);
        let sectors = paths(&rig.shapes);
        let fills: Vec<_> = sectors.iter().map(|p| p.fill).collect();
        // controlFill sector, main sector, then the track punch-out: a disc of radius Ri = 0.48 D − 3 th (th = 3 at
        // 0.5).
        assert_eq!(fills, [control_fill, Color32::RED], "{arc:?}");
        let punch = circles(&rig.shapes)[0];
        assert_eq!((punch.0, punch.1), (Color32::BLUE, RECT.center()));
        assert!(close(punch.2, 46.08 - 9.0), "{punch:?}");
        let main = &sectors[1].points;
        let ends = [main[1], main[main.len() - 1]].map(|p| angle_of(RECT.center(), p));
        let same = |a: f32, b: f32| (a - b).rem_euclid(360.0).min((b - a).rem_euclid(360.0)) < 1e-2;
        assert!(
            ends.iter().any(|e| same(*e, from)) && ends.iter().any(|e| same(*e, to)),
            "{arc:?} {ends:?}"
        );
    }
}

#[test]
fn segmented_arcs_snap_and_point_at_the_selected_segment() {
    for arc in [LEFT, RIGHT] {
        let mut rig = KitRig::with(Kit::Arc(arc), 0.0, |r| r.steps = Some(10));
        // 0.3 of the range snaps to step 3 of 9.
        rig.drag(&[vec2(0.0, -10.0), vec2(0.0, -240.0)]);
        assert!(close(rig.value, 3.0 / 9.0), "{}", rig.value);
        rig.frame(vec![]);
        // No handle: the selected segment is a channel sector, the pointer a channel triangle outside the ring.
        assert!(handle(&rig.shapes).is_empty());
        let red: Vec<_> = paths(&rig.shapes)
            .into_iter()
            .filter(|p| p.fill == Color32::RED)
            .collect();
        assert_eq!(red.len(), 2);
        let pointer = &red[1].points;
        assert_eq!(pointer.len(), 3);
        // Ten 9° segments: segment 3 is centred 31.5° along the arc.
        let tip = pointer
            .iter()
            .copied()
            .min_by(|a, b| {
                a.distance(RECT.center())
                    .total_cmp(&b.distance(RECT.center()))
            })
            .unwrap();
        let along = arc_value(&arc, angle_of(RECT.center(), tip)) * arc.span;
        assert!(close(along, 31.5), "{arc:?}: {along}");
        // The separators: ten sectors in the track colour.
        let blue = paths(&rig.shapes)
            .iter()
            .filter(|p| p.fill == Color32::BLUE)
            .count();
        assert_eq!(blue, 10);
    }
}

#[test]
fn a_pressed_arc_washes_its_middle_and_swells_until_released() {
    let t = metrics().t();
    let mut rig = KitRig::new(Kit::Arc(RIGHT), 0.0);
    let rest = handle(&rig.shapes)[0].2;
    assert!(close(rest, 5.0 * t));
    rig.frame(vec![Event::PointerMoved(RECT.center())]);
    rig.button(RECT.center(), PointerButton::Primary, true);
    for _ in 0..12 {
        rig.frame(vec![]);
    }
    let wash = Color32::RED.gamma_multiply(0.05);
    let lit = handle(&rig.shapes);
    assert!(lit.iter().any(|c| c.0 == wash), "{lit:?}");
    // Held: s = 1 + 0.99·10/128, and the handle ×1.05.
    let held = lit.iter().find(|c| c.0 != wash).unwrap().2;
    assert!(
        close(held, rest * (1.0 + 0.99 * 10.0 / 128.0) * 1.05),
        "{held}"
    );
    rig.button(RECT.center(), PointerButton::Primary, false);
    // Off the arc, or it stays lit by the hover.
    rig.frame(vec![Event::PointerMoved(pos2(700.0, 500.0))]);
    for _ in 0..12 {
        rig.frame(vec![]);
    }
    let after = handle(&rig.shapes);
    assert_eq!(after.len(), 2);
    assert!(close(after[0].2, rest), "{after:?}");
}

#[test]
fn the_arc_label_is_plain_text_and_its_value_is_fitted_to_the_inner_disc() {
    const LONG: &str = "A long label, wider than 3 U";
    let m = metrics();
    let mut rig = KitRig::with(Kit::Arc(RIGHT), 0.25, |r| {
        r.label = LONG;
        r.value_text = true;
    });
    rig.frame(vec![]);
    let boxes = text_boxes(&rig.shapes);
    let shown: Vec<_> = boxes.iter().map(|t| t.0.as_str()).collect();
    assert_eq!(shown, [LONG, "0.25"]);
    // The label as on every control: 8 pt, in full, centred on the top strip, not clipped to the control.
    let strip = 0.75 * U;
    assert!(boxes[0].1.width() > RECT.width());
    assert_plain(
        &boxes[0],
        m.font_id(FontSize::Pt8).size,
        pos2(148.0, 100.0 + strip / 2.0),
    );
    // The value in its bottom strip, fit-only: the body is 96 × 80, so D = 80, s = 2.5 and the fit width is
    // 2 (0.48 · 80 − 3 · 2.5) − 4 t = 53.8 points. `0.25` fits at 12 pt, taller than the strip: the height is not
    // checked.
    assert_plain(
        &boxes[1],
        m.font_id_pt(12.0).size,
        pos2(148.0, 228.0 - strip / 2.0),
    );
    assert!(boxes[1].1.width() <= 2.0 * (0.48 * 80.0 - 7.5) - 4.0 * m.t());
    assert!(boxes[1].1.height() > strip);
}

#[test]
fn a_long_arc_value_shrinks_a_point_at_a_time_to_6_pt_and_is_never_truncated() {
    let (theme, m) = (theme_by_id(0), metrics());
    let cx = UiCtx {
        theme: &theme,
        m: &m,
    };
    // Labelled, with value text: the body is 96 × 80 centred on (148, 164), so D = 80, and the value strip is the
    // bottom 0.75 U. Fit-only sizing: the first size from 12 pt down, a point at a time, whose width fits the
    // inner-disc diameter less 4 hairlines; else 6 pt, overflowing in full.
    let v = 0.25;
    let inner = knob::ring(pos2(148.0, 164.0), 80.0, v, false, m.t()).inner;
    let fits = knob::value_width(inner, m.t());
    for (value, size) in [
        ("0.25", Some(12)),
        ("-12.5 dB", None),
        ("A value text far too long", Some(6)),
    ] {
        let ctx = Context::default();
        let raw = RawInput {
            screen_rect: Some(SCREEN),
            ..Default::default()
        };
        let output = ctx.run(raw, |ctx| {
            egui::CentralPanel::default().show(ctx, |ui| {
                let c = Control {
                    value_text: true,
                    ..Control::new(Id::new("a"), RECT, "Label", Color32::RED, 0.0)
                };
                arc_slider(ui, &cx, &c, &RIGHT, &mut v.clone(), |_| value.to_owned());
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
        match size {
            Some(size) => assert_eq!(fitted, size, "{value}"),
            None => assert!(fitted > 6 && fitted < 12, "{value}: {fitted}"),
        }
        let boxes = text_boxes(&output.shapes);
        let shown = boxes
            .iter()
            .find(|b| b.0 == value)
            .unwrap_or_else(|| panic!("{value}: not drawn in full: {boxes:?}"));
        assert_plain(
            shown,
            m.font_id_pt(f32::from(fitted)).size,
            pos2(148.0, 228.0 - 0.75 * U / 2.0),
        );
        // Only the 6 pt fallback overflows.
        assert_eq!(shown.1.width() <= fits, size != Some(6), "{shown:?}");
    }
}

// Tag slider.

#[test]
fn the_tag_sits_at_its_value_and_reads_back_both_ways() {
    let h = RECT.height();
    for right in [true, false] {
        for v in [0.0, 0.25, 1.0] {
            let rig = KitRig::new(Kit::Tag(right), v);
            let tags = paths(&rig.shapes);
            assert_eq!(tags.len(), 1);
            let tag = &tags[0];
            assert_eq!(tag.fill, Color32::RED.gamma_multiply(0.2));
            let b = Rect::from_points(&tag.points);
            // 0.12 h tall, across the full width, the tip on the display side at the tag's middle.
            assert!(close(b.height(), 0.12 * h) && close(b.width(), RECT.width()));
            let tip_x = if right { RECT.right() } else { RECT.left() };
            assert!(
                tag.points
                    .iter()
                    .any(|p| close(p.x, tip_x) && close(p.y, b.center().y))
            );
            let back = (RECT.bottom() - 0.06 * h - b.center().y) / (0.88 * h);
            assert!(close(back, v), "{right} {v}: {back}");
        }
        // Over the full height: on the bottom edge at 0, the top edge at 1.
        let at = |v| Rect::from_points(&paths(&KitRig::new(Kit::Tag(right), v).shapes)[0].points);
        assert!(close(at(0.0).bottom(), RECT.bottom()) && close(at(1.0).top(), RECT.top()));
    }
}

#[test]
fn tags_follow_vertical_drags_up_increasing() {
    // The tag follows the pointer over its 0.88 h of travel.
    let travel = 0.88 * RECT.height();
    for right in [true, false] {
        let mut rig = KitRig::new(Kit::Tag(right), 0.5);
        rig.drag(&[vec2(0.0, -10.0), vec2(0.0, -travel / 4.0)]);
        assert!(close(rig.value, 0.75), "{}", rig.value);
        rig.drag(&[vec2(0.0, 10.0), vec2(0.0, travel / 2.0)]);
        assert!(close(rig.value, 0.25), "{}", rig.value);
    }
}

/// The tag's text: string, centre of its layout box (not of the ink) and the clip rect it was painted under.
fn tag_text(shapes: &[ClippedShape]) -> (String, Pos2, Rect) {
    let texts: Vec<_> = shapes
        .iter()
        .filter_map(|c| match &c.shape {
            egui::Shape::Text(t) => Some((
                t.galley.text().to_owned(),
                t.galley.rect.translate(t.pos.to_vec2()).center(),
                c.clip_rect,
            )),
            _ => None,
        })
        .collect();
    assert_eq!(texts.len(), 1, "{texts:?}");
    texts[0].clone()
}

#[test]
fn the_tag_shows_its_label_centred_beside_the_pointer_and_its_value_while_dragged() {
    let h = RECT.height();
    for right in [true, false] {
        let mut rig = KitRig::with(Kit::Tag(right), 0.25, |r| r.label = "Max");
        let (text, at, clip) = tag_text(&rig.shapes);
        assert_eq!(text, "Max");
        // Centred in the bar moved 0.015 h towards the pointer: the bar is w − 0.06 h wide from the far edge.
        let bar_mid = if right {
            RECT.left() + (RECT.width() - 0.06 * h) / 2.0 + 0.015 * h
        } else {
            RECT.right() - (RECT.width() - 0.06 * h) / 2.0 - 0.015 * h
        };
        let y = RECT.bottom() - 0.06 * h - 0.88 * h * 0.25;
        assert!(
            (at.x - bar_mid).abs() < 1.0 && (at.y - y).abs() < 1.0,
            "{right}: {at:?}"
        );
        // Plain text: not clipped to the control.
        assert!(!RECT.contains_rect(clip), "{clip:?}");
        // While dragged: the value, following the tag; the label again once released.
        let start = RECT.center();
        rig.frame(vec![Event::PointerMoved(start)]);
        rig.button(start, PointerButton::Primary, true);
        let up = vec2(0.0, 0.25 * 0.88 * h);
        rig.frame(vec![Event::PointerMoved(start - vec2(0.0, 10.0))]);
        rig.frame(vec![Event::PointerMoved(start - up)]);
        let (text, at, _) = tag_text(&rig.shapes);
        assert_eq!(text, "0.50");
        let y = RECT.bottom() - 0.06 * h - 0.88 * h * 0.5;
        assert!((at.y - y).abs() < 1.0, "{at:?}");
        rig.button(start - up, PointerButton::Primary, false);
        assert_eq!(tag_text(&rig.shapes).0, "Max");
    }
}

#[test]
fn the_tag_text_is_10_pt_and_overflows_rather_than_shrinking() {
    // A narrow control and a long label: the text keeps its size and spills out evenly on both sides.
    let narrow = Rect::from_min_size(pos2(100.0, 100.0), vec2(20.0, 128.0));
    let rig = KitRig::with(Kit::Tag(true), 0.5, |r| {
        r.rect = narrow;
        r.label = "A long label";
    });
    let galley = rig
        .shapes
        .iter()
        .find_map(|c| match &c.shape {
            egui::Shape::Text(t) => Some(t.clone()),
            _ => None,
        })
        .unwrap();
    assert_eq!(galley.galley.text(), "A long label");
    assert!(galley.visual_bounding_rect().width() > narrow.width());
    let font = galley.galley.job.sections[0].format.font_id.size;
    assert_eq!(font, metrics().font_id(crate::metrics::FontSize::Pt10).size);
}

// Graph knob.

/// The graph knob's curve: the open path.
fn curve(shapes: &[ClippedShape]) -> PathShape {
    let open: Vec<_> = paths(shapes).into_iter().filter(|p| !p.closed).collect();
    assert_eq!(open.len(), 1);
    open[0].clone()
}

#[test]
fn the_graph_knob_plots_its_envelope_in_the_middle_of_the_ring() {
    let t = metrics().t();
    let lw = 4.0;
    // Body 96 × 104 under the label strip, centre (148, 176), D = 96. The plot is a square as wide as the inner
    // radius 0.48 D − 3 s, s = t + (min(0.05 D, 2 t) − t)·v = t + t·v; the samples stay lw/2 inside it.
    let frame = |v: f32| {
        let side = 46.08 - 3.0 * (t + t * v);
        let plot = Rect::from_center_size(pos2(148.0, 176.0), Vec2::splat(side));
        (plot, plot.shrink(lw / 2.0))
    };
    let label = |r: &mut KitRig| r.label = "A. Curve";
    // A. Curve at its default 0: fully logarithmic, from the bottom left up to the top right, above the diagonal.
    let rig = KitRig::with(Kit::Graph(false), 0.0, label);
    let (plot, inner) = frame(0.0);
    let line = curve(&rig.shapes);
    assert_eq!(line.stroke.width, lw);
    let (first, last) = (line.points[0], *line.points.last().unwrap());
    assert!(first.distance(inner.left_bottom()) < 1e-3, "{first:?}");
    assert!(last.distance(inner.right_top()) < 1e-3, "{last:?}");
    let mid = line.points[line.points.len() / 2];
    assert!(mid.y < plot.center().y - plot.height() / 4.0, "{mid:?}");
    // R. Curve at its default 1: A(1 − x), falling fast from the top left.
    let rig = KitRig::with(Kit::Graph(true), 1.0, label);
    let (plot, inner) = frame(1.0);
    let line = curve(&rig.shapes);
    let (first, last) = (line.points[0], *line.points.last().unwrap());
    assert!(first.distance(inner.left_top()) < 1e-3, "{first:?}");
    assert!(last.distance(inner.right_bottom()) < 0.1, "{last:?}");
    let mid = line.points[line.points.len() / 2];
    assert!(mid.y > plot.center().y + plot.height() / 4.0, "{mid:?}");
    // 0.5: a straight line (Env(x, 0) = x), rising or falling.
    let (_, inner) = frame(0.5);
    for falling in [false, true] {
        let rig = KitRig::with(Kit::Graph(falling), 0.5, label);
        let line = curve(&rig.shapes);
        let n = line.points.len();
        for (i, p) in line.points.iter().enumerate() {
            let f = i as f32 / (n - 1) as f32;
            let f = if falling { 1.0 - f } else { f };
            assert!(
                close(p.y, inner.bottom() - inner.height() * f),
                "{i}: {p:?}"
            );
        }
    }
}

#[test]
fn the_graph_knob_has_its_label_and_no_value_text() {
    let mut rig = KitRig::with(Kit::Graph(false), 0.3, |r| r.label = "R. Curve");
    rig.frame(vec![]);
    let texts = texts(&rig.shapes);
    assert_eq!(texts.len(), 1, "{texts:?}");
    assert_eq!(texts[0].0, "R. Curve");
    // Auto-sized to the 24-point strip: smaller than 12 pt (24 points at U = 32), and inside the control.
    let galley = rig.shapes.iter().find_map(|c| match &c.shape {
        egui::Shape::Text(t) => Some(t.galley.size()),
        _ => None,
    });
    assert!(galley.unwrap().y <= 24.0, "{galley:?}");
    // A label too long for the strip even at 6 pt falls back to 6 pt and is drawn in full, centred on the strip.
    const LONG: &str = "A long label, wider than 3 U";
    let rig = KitRig::with(Kit::Graph(false), 0.3, |r| r.label = LONG);
    let boxes = text_boxes(&rig.shapes);
    assert_eq!(boxes.len(), 1);
    assert!(boxes[0].1.width() > RECT.width());
    assert_plain(
        &boxes[0],
        metrics().font_id_pt(6.0).size,
        pos2(148.0, 112.0),
    );
}

#[test]
fn a_graph_knob_paints_ring_inner_disc_halo_and_dot_in_that_order() {
    let m = metrics();
    let module_fill = theme_by_id(0).colors.module_fill.to_color32();
    // No label: the body is the whole 96 × 128 rect, D = 96; a held press swells D by 0.99 · 10 px over 128, and
    // the pressed dot grows × 1.05.
    let center = RECT.center();
    let check = |rig: &KitRig, swell: f32, pressed: bool| {
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
    let mut rig = KitRig::new(Kit::Graph(false), 0.3);
    check(&rig, 1.0, false);
    rig.frame(vec![Event::PointerMoved(center)]);
    rig.button(center, PointerButton::Primary, true);
    for _ in 0..12 {
        rig.frame(vec![]);
    }
    check(&rig, 1.0 + 0.99 * 10.0 / 128.0, true);
}

#[test]
fn graph_knobs_turn_like_knobs() {
    let mut rig = KitRig::new(Kit::Graph(false), 0.0);
    rig.drag(&[vec2(0.0, -10.0), vec2(0.0, -400.0)]);
    assert!(close(rig.value, 0.5), "{}", rig.value);
}

// XY pad.

/// The pad's crosshair: the vertical and the horizontal channel line.
fn crosshair(shapes: &[ClippedShape]) -> (Rect, Rect) {
    let lines: Vec<_> = rects(shapes)
        .into_iter()
        .filter(|r| r.0 == Color32::RED)
        .map(|r| r.1)
        .collect();
    assert_eq!(lines.len(), 2, "{lines:?}");
    (lines[0], lines[1])
}

#[test]
fn a_click_sets_both_values_with_y_up_and_the_crosshair_reads_back() {
    let mut rig = KitRig::new(Kit::Pad, 0.5);
    rig.click_with(
        pos2(RECT.left() + 0.25 * 96.0, RECT.top() + 0.25 * 128.0),
        PointerButton::Primary,
    );
    assert_eq!((rig.value, rig.y), (0.25, 0.75));
    rig.frame(vec![]);
    let (v, h) = crosshair(&rig.shapes);
    // 2 px lines across the pad, through the values.
    assert_eq!((v.width(), v.height()), (2.0, 128.0));
    assert_eq!((h.width(), h.height()), (96.0, 2.0));
    let x = (v.center().x - RECT.left()) / RECT.width();
    let y = (RECT.bottom() - h.center().y) / RECT.height();
    assert!(
        (x - 0.25).abs() <= 1.0 / 96.0 && (y - 0.75).abs() <= 1.0 / 128.0,
        "{x} {y}"
    );
    // The dot at the crossing, 3 px.
    let dots = circles(&rig.shapes);
    assert_eq!(dots.len(), 1);
    assert_eq!(dots[0].1, pos2(124.0, 132.0));
    assert_eq!(dots[0].2, 3.0);
}

#[test]
fn a_pad_drag_follows_the_pointer_and_clamps_to_the_pad() {
    let mut rig = KitRig::new(Kit::Pad, 0.0);
    // From the centre: right 24, up 32 = a quarter of each side.
    rig.frame(vec![Event::PointerMoved(RECT.center())]);
    rig.button(RECT.center(), PointerButton::Primary, true);
    assert_eq!((rig.value, rig.y), (0.5, 0.5));
    rig.frame(vec![Event::PointerMoved(RECT.center() + vec2(24.0, -32.0))]);
    assert_eq!((rig.value, rig.y), (0.75, 0.75));
    rig.frame(vec![Event::PointerMoved(
        RECT.center() + vec2(500.0, 500.0),
    )]);
    assert_eq!((rig.value, rig.y), (1.0, 0.0));
    assert!(rig.changed);
}

#[test]
fn the_pad_draws_an_11_by_11_grid_and_a_halo_while_pressed() {
    let colors = theme_by_id(0).colors;
    let grid = colors.background.to_color32();
    let mut rig = KitRig::new(Kit::Pad, 0.5);
    let lines: Vec<_> = rects(&rig.shapes)
        .into_iter()
        .filter(|r| r.0 == grid)
        .map(|r| r.1)
        .collect();
    assert_eq!(lines.iter().filter(|r| r.width() == 1.0).count(), 11);
    assert_eq!(lines.iter().filter(|r| r.height() == 1.0).count(), 11);
    assert!(lines.iter().all(|r| RECT.contains_rect(*r)));
    let halo = Color32::RED.gamma_multiply(0.4);
    let halos = |rig: &KitRig| {
        rig.shapes
            .iter()
            .filter(|c| matches!(&c.shape, egui::Shape::Rect(r) if r.fill == halo))
            .count()
    };
    assert_eq!(halos(&rig), 0);
    rig.frame(vec![Event::PointerMoved(RECT.center())]);
    rig.button(RECT.center(), PointerButton::Primary, true);
    rig.frame(vec![]);
    assert_eq!(halos(&rig), 1);
}

#[test]
fn the_dot_is_drawn_whole_at_the_pads_corners() {
    for (x, y) in [(0.0, 0.0), (1.0, 1.0), (0.0, 1.0)] {
        let rig = KitRig::with(Kit::Pad, x, |r| r.y = y);
        let dot = rig
            .shapes
            .iter()
            .find_map(|c| match &c.shape {
                egui::Shape::Circle(d) => Some((d.visual_bounding_rect(), c.clip_rect)),
                _ => None,
            })
            .expect("dot");
        assert!(dot.1.contains_rect(dot.0), "{x} {y}: {dot:?}");
    }
}

#[test]
fn a_disabled_pad_ignores_input() {
    let mut rig = KitRig::with(Kit::Pad, 0.5, |r| r.disabled = true);
    rig.click_with(RECT.min + vec2(5.0, 5.0), PointerButton::Primary);
    rig.drag(&[vec2(30.0, 30.0)]);
    assert_eq!((rig.value, rig.y), (0.5, 0.5));
}

// All of kit D.

#[test]
fn only_the_primary_button_moves_kit_d() {
    let d = 2.0 * U;
    for kit in KITS {
        for (button, moves) in [
            (PointerButton::Primary, true),
            (PointerButton::Secondary, false),
            (PointerButton::Middle, false),
        ] {
            let mut rig = KitRig::new(kit, 0.5);
            rig.drag_with(button, &[vec2(10.0, -10.0), vec2(d / 2.0, -d)]);
            if let Kit::Pad = kit {
                rig.click_with(RECT.min + vec2(5.0, 5.0), button);
            }
            let moved = rig.value != 0.5 || rig.y != 0.5;
            assert_eq!(moved, moves, "{kit:?} {button:?}");
        }
    }
}

#[test]
fn nan_values_draw_as_the_default_and_stay_until_the_user_moves_them() {
    for kit in KITS {
        let mut rig = KitRig::with(kit, f32::NAN, |r| {
            r.default = 0.25;
            r.label = "Label";
            r.value_text = matches!(kit, Kit::Arc(_));
        });
        rig.frame(vec![]);
        assert!(
            rig.vertices
                .iter()
                .all(|p| p.x.is_finite() && p.y.is_finite()),
            "{kit:?}"
        );
        assert!(
            rig.value.is_nan() && rig.y.is_nan() && !rig.changed,
            "{kit:?}"
        );
        assert!(texts(&rig.shapes).iter().all(|(s, ..)| !s.contains("NaN")));
    }
    // The arc's handle and the graph knob's curve are drawn at the default.
    for arc in [LEFT, RIGHT] {
        let rig = KitRig::with(Kit::Arc(arc), f32::NAN, |r| r.default = 0.25);
        let at = handle(&rig.shapes)[0].1;
        let v = arc_value(&arc, angle_of(RECT.center(), at));
        assert!(close(v, 0.25), "{arc:?}: {v}");
    }
    for falling in [false, true] {
        let nan = KitRig::with(Kit::Graph(falling), f32::NAN, |r| r.default = 0.25);
        let at_default = KitRig::new(Kit::Graph(falling), 0.25);
        assert_eq!(curve(&nan.shapes).points, curve(&at_default.shapes).points);
    }
    // The pad's crosshair sits at the default.
    let rig = KitRig::with(Kit::Pad, f32::NAN, |r| r.default = 0.25);
    let (v, h) = crosshair(&rig.shapes);
    assert!((v.center().x - (RECT.left() + 24.0)).abs() <= 1.0, "{v:?}");
    assert!(
        (h.center().y - (RECT.bottom() - 32.0)).abs() <= 1.0,
        "{h:?}"
    );
    // A drag starts from the default.
    let mut rig = KitRig::with(Kit::Tag(true), f32::NAN, |r| r.default = 0.25);
    rig.drag(&[vec2(0.0, -10.0), vec2(0.0, -0.25 * 0.88 * RECT.height())]);
    assert!(close(rig.value, 0.5), "{}", rig.value);
}

#[test]
fn kit_d_renders_inside_the_screen_at_ppp_1_and_2() {
    for kit in KITS {
        let rect = |rig: &KitRig| {
            rig.vertices.iter().fold(Rect::NOTHING, |r, p| {
                r.union(Rect::from_min_size(*p, Vec2::ZERO))
            })
        };
        let one = KitRig::with(kit, 0.6, |r| r.label = "Label");
        let two = KitRig::with(kit, 0.6, |r| {
            r.label = "Label";
            r.ppp = 2.0;
        });
        assert!(!one.vertices.is_empty() && !two.vertices.is_empty());
        let (a, b) = (rect(&one), rect(&two));
        assert!(a.is_finite() && b.is_finite(), "{kit:?}");
        // The same picture at twice the pixels: the drawn extent scales, within a few pixels of feathering.
        let screen = Rect::from_min_size(Pos2::ZERO, SCREEN.size() * 2.0);
        assert!(screen.contains_rect(b), "{kit:?} {b:?}");
        assert!(
            (b.width() - 2.0 * a.width()).abs() <= 4.0,
            "{kit:?} {a:?} {b:?}"
        );
        assert!(
            (b.height() - 2.0 * a.height()).abs() <= 4.0,
            "{kit:?} {a:?} {b:?}"
        );
    }
}

#[test]
fn pixel_sized_lines_stay_one_and_four_pixels_at_ppp_2() {
    let grid = theme_by_id(0).colors.background.to_color32();
    let rig = KitRig::with(Kit::Pad, 0.5, |r| r.ppp = 2.0);
    let thin = rects(&rig.shapes)
        .into_iter()
        .filter(|r| r.0 == grid)
        .all(|r| r.1.width() == 0.5 || r.1.height() == 0.5);
    assert!(thin);
    let rig = KitRig::with(Kit::Graph(false), 0.5, |r| r.ppp = 2.0);
    assert_eq!(curve(&rig.shapes).stroke.width, 2.0);
}

#[test]
fn kit_d_in_empty_rects_draws_nothing_out_of_bounds() {
    for kit in KITS {
        for rect in [
            Rect::from_min_size(pos2(100.0, 100.0), vec2(96.0, 0.0)),
            Rect::from_min_size(pos2(100.0, 100.0), vec2(0.0, 128.0)),
            Rect::from_min_size(pos2(100.0, 100.0), Vec2::ZERO),
        ] {
            let mut rig = KitRig::with(kit, 0.5, |r| r.rect = rect);
            rig.frame(vec![Event::PointerMoved(rect.center())]);
            rig.button(rect.center(), PointerButton::Primary, true);
            rig.frame(vec![Event::PointerMoved(rect.center() + vec2(5.0, -5.0))]);
            assert!(
                rig.vertices
                    .iter()
                    .all(|p| p.x.is_finite() && p.y.is_finite()),
                "{kit:?} {rect:?}"
            );
            assert!(
                rig.value.is_finite() && rig.y.is_finite(),
                "{kit:?} {rect:?}"
            );
        }
    }
}
