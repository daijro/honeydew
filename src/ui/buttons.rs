//! Small round buttons, the shared close X and the captioned transport button. Callers draw the content (icon, glyph
//! or text) into the returned response's rect. Every interactive widget takes an explicit [`Id`].

use eframe::egui::{
    self, Align2, Color32, FontId, Id, Pos2, Rect, Response, Sense, Stroke, Vec2, vec2,
};

use crate::metrics::{FontSize, Metrics};
use crate::theme::ThemeColors;
use crate::ui::widgets::{hovered, pressed};
use crate::ui::{UiCtx, focus};

/// Height of the caption strip at the bottom of a captioned button, in U.
const CAPTION_H: f32 = 0.6;
/// A held tab's radius factor.
const TAB_PRESSED_GROWTH: f32 = 1.05;

/// The response of a circular button. egui hit-tests the bounding square, so a click in the square's corner
/// reaches the widget; [`DiscResponse::clicked`] only counts clicks on the disc (distance from the centre
/// ≤ radius). The wrapped `response` is the square's: its `hovered()` and the like are not narrowed.
pub struct DiscResponse {
    pub response: Response,
    center: Pos2,
    radius: f32,
}

impl DiscResponse {
    /// The bounding square.
    pub fn rect(&self) -> Rect {
        self.response.rect
    }

    /// Clicked, with the pointer on the disc. A click by the keyboard (Space / Enter on the focused button) has
    /// no pointer position and counts.
    pub fn clicked(&self) -> bool {
        self.response.clicked()
            && self
                .response
                .interact_pointer_pos()
                .is_none_or(|p| p.distance(self.center) <= self.radius)
    }
}

fn disc_button(
    ui: &mut egui::Ui,
    id: Id,
    center: Pos2,
    diameter: f32,
    sense: Sense,
) -> DiscResponse {
    let square = Rect::from_center_size(center, Vec2::splat(diameter));
    DiscResponse {
        response: focus::interact(ui, square, id, sense),
        center,
        radius: diameter / 2.0,
    }
}

/// A circle `diameter` points across with a 1-px outline in `color`. It senses drags too (and ignores them), as
/// [`filled_circle_button`] does: the floating buttons sit over clips and notes, which a drag must not reach.
pub fn outline_circle_button(
    ui: &mut egui::Ui,
    id: Id,
    center: Pos2,
    diameter: f32,
    color: Color32,
    m: &Metrics,
) -> DiscResponse {
    let response = disc_button(ui, id, center, diameter, Sense::click_and_drag());
    let px = 1.0 / m.ppp();
    // Inset by half the stroke so the ring stays inside the diameter.
    ui.painter()
        .circle_stroke(center, (diameter - px) / 2.0, Stroke::new(px, color));
    response
}

/// A filled circle `diameter` points across. Off-screen parts are clipped, which also makes the quarter-
/// and half-disc tabs on the screen edges. It senses drags too (and ignores them), so that a drag starting
/// on it does not fall through to a drag-sensing widget underneath, such as the scrolling track area.
pub fn filled_circle_button(
    ui: &mut egui::Ui,
    id: Id,
    center: Pos2,
    diameter: f32,
    fill: Color32,
) -> DiscResponse {
    filled_disc(ui, id, center, diameter, fill, false, 1.0)
}

/// [`filled_circle_button`] for the corner and edge tabs (the logo, the rack tab, the show-keyboard `↖`): while held,
/// its fill is brighter (RGB × 1.15) and its radius 5 % larger.
pub fn tab_button(
    ui: &mut egui::Ui,
    id: Id,
    center: Pos2,
    diameter: f32,
    fill: Color32,
) -> DiscResponse {
    filled_disc(ui, id, center, diameter, fill, true, 1.0)
}

/// A light bubble (the floating rows' buttons and the transport's `Undo` and `Redo`): a [`filled_circle_button`] in
/// `text`, painted at `scale` of its size (a pop-in; the touch takes the whole disc), with `label` centred on it in
/// `menuFill`, the playhead disc's and the corner logo's pairing. Hovered or held on the disc it lights as a filled
/// disc does (RGB × 1.15), the label still dark on it.
#[allow(clippy::too_many_arguments)]
pub fn bubble_button(
    ui: &mut egui::Ui,
    id: Id,
    center: Pos2,
    diameter: f32,
    scale: f32,
    label: &str,
    font: FontId,
    c: &ThemeColors,
) -> DiscResponse {
    let response = filled_disc(ui, id, center, diameter, c.text.to_color32(), false, scale);
    let ink = c.menu_fill.to_color32();
    (ui.painter()).text(center, Align2::CENTER_CENTER, label, font, ink);
    response
}

/// [`filled_circle_button`], with [`tab_button`]'s pressed look when `tab`, painted at `scale` of its size. Hovered with
/// the pointer on the disc, the fill is RGB × 1.15 too, never twice.
fn filled_disc(
    ui: &mut egui::Ui,
    id: Id,
    center: Pos2,
    diameter: f32,
    fill: Color32,
    tab: bool,
    scale: f32,
) -> DiscResponse {
    let response = disc_button(ui, id, center, diameter, Sense::click_and_drag());
    let r = &response.response;
    let on_disc = r
        .hover_pos()
        .is_some_and(|p| p.distance(center) <= diameter / 2.0);
    let (fill, diameter) = if tab && r.is_pointer_button_down_on() {
        (pressed(fill), diameter * TAB_PRESSED_GROWTH)
    } else if on_disc && hovered(ui, r) {
        (pressed(fill), diameter)
    } else {
        (fill, diameter)
    };
    ui.painter()
        .circle_filled(center, diameter * scale / 2.0, fill);
    response
}

/// A filled circle `diameter` across whose touch area is the `hit` x `hit` box round it: any press in the box is the
/// button's, and it lights as [`filled_circle_button`] does while hovered.
pub fn boxed_circle_button(
    ui: &mut egui::Ui,
    id: Id,
    center: Pos2,
    diameter: f32,
    hit: f32,
    fill: Color32,
) -> Response {
    let box_ = Rect::from_center_size(center, Vec2::splat(hit));
    let response = focus::interact(ui, box_, id, Sense::click_and_drag());
    let fill = if hovered(ui, &response) {
        pressed(fill)
    } else {
        fill
    };
    ui.painter().circle_filled(center, diameter / 2.0, fill);
    response
}

/// A floating panel button (the show-keyboard `↖`): `shape`, a rounded rect of corner radius `r`, filled `moduleFill`
/// over a soft drop shadow (12 stacked black layers, α 0.25 together at the edge, spreading 0.45 U, 0.16 U down) and
/// inside a rim t wide of 0.7·`background` + 0.3·`text`. A hovering mouse makes it 5 % larger about its centre and its
/// fill ×1.25. `hit` is its touch area; the caller draws the icon.
pub fn floating_button(
    ui: &mut egui::Ui,
    cx: &UiCtx,
    id: Id,
    shape: Rect,
    r: f32,
    hit: Rect,
) -> Response {
    const LAYERS: u8 = 12;
    let (c, m) = (&cx.theme.colors, cx.m);
    // Drags too, so that none falls through to the view under it.
    let response = focus::interact(ui, hit, id, Sense::click_and_drag());
    let hover = hovered(ui, &response);
    let (shape, r) = if hover {
        (shape.scale_from_center(1.05), r * 1.05)
    } else {
        (shape, r)
    };
    let painter = ui.painter();
    // Each layer's α, so that the 12 make 0.25 where all overlap.
    let alpha = 1.0 - 0.75f32.powf(1.0 / f32::from(LAYERS));
    let shadow = shape.translate(vec2(0.0, m.u(0.16)));
    for i in (1..=LAYERS).rev() {
        let spread = m.u(0.45) * f32::from(i) / f32::from(LAYERS);
        let black = Color32::from_black_alpha((alpha * 255.0).round() as u8);
        painter.rect_filled(shadow.expand(spread), r + spread, black);
    }
    let (bg, text) = (c.background, c.text);
    let mix = |a: f32, b: f32| 0.7 * a + 0.3 * b;
    let rim = crate::theme::Rgba::rgb(mix(bg.r, text.r), mix(bg.g, text.g), mix(bg.b, text.b));
    let t = m.t();
    painter.rect_filled(shape.expand(t), r + t, rim.to_color32());
    let fill = c.module_fill.to_color32();
    let fill = if hover { hover_lit(fill) } else { fill };
    painter.rect_filled(shape, r, fill);
    response
}

/// `rect` filled with a vertical gradient from `top` to `bottom`.
pub fn gradient(painter: &egui::Painter, rect: Rect, top: Color32, bottom: Color32) {
    let mut mesh = egui::Mesh::default();
    for (p, color) in [
        (rect.left_top(), top),
        (rect.right_top(), top),
        (rect.right_bottom(), bottom),
        (rect.left_bottom(), bottom),
    ] {
        mesh.colored_vertex(p, color);
    }
    mesh.add_triangle(0, 1, 2);
    mesh.add_triangle(0, 2, 3);
    painter.add(mesh);
}

/// `color` under the mouse: RGB × 1.25, clamped, alpha kept.
pub fn hover_lit(color: Color32) -> Color32 {
    let [r, g, b, a] = color.to_srgba_unmultiplied();
    let lit = |c: u8| (f32::from(c) * 1.25).round().min(255.0) as u8;
    Color32::from_rgba_unmultiplied(lit(r), lit(g), lit(b), a)
}

/// The close X of every dialog and panel: icon 19 in `text`, `icon` × its standard size, on a disc of `surface` (the
/// colour the X sits on) `radius` × the short side of `rect` in radius. So at rest only the X shows; under the mouse
/// the disc lights (RGB × 1.25). The whole `rect` takes the touch.
pub fn close_x(
    ui: &mut egui::Ui,
    cx: &UiCtx,
    id: Id,
    rect: Rect,
    (surface, radius): (Color32, f32),
    icon: f32,
) -> Response {
    let response = focus::interact(ui, rect, id, Sense::click());
    if hovered(ui, &response) {
        let r = radius * rect.width().min(rect.height());
        (ui.painter()).circle_filled(rect.center(), r, hover_lit(surface));
    }
    let text = cx.theme.colors.text.to_color32();
    crate::icons::draw_icon(
        ui.painter(),
        rect,
        crate::icons::IconId::Close,
        text,
        icon,
        cx.m,
    );
    response
}

/// A transport button filling `rect`, with a small-caps `caption` (`FontSize::Pt6`) along its bottom.
/// Returns the response and the rect above the caption, for the content.
pub fn captioned_button(
    ui: &mut egui::Ui,
    id: Id,
    rect: Rect,
    caption: &str,
    color: Color32,
    m: &Metrics,
) -> (Response, Rect) {
    let response = focus::interact(ui, rect, id, Sense::click());
    let (content, caption_rect) = rect.split_top_bottom_at_y(rect.bottom() - m.u(CAPTION_H));
    ui.painter().text(
        caption_rect.center(),
        Align2::CENTER_CENTER,
        caption,
        m.font_id(FontSize::Pt6),
        color,
    );
    (response, content)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::ui::offscreen::Offscreen;
    use eframe::egui::{Event, Key, pos2};

    const CENTER: Pos2 = pos2(100.0, 100.0);
    const D: f32 = 40.0;

    /// One frame of a filled disc button; whether it was clicked.
    fn frame(rig: &mut Offscreen, events: Vec<Event>) -> bool {
        let mut clicked = false;
        rig.frame(events, &mut |ui, _| {
            clicked = filled_circle_button(ui, Id::new("disc"), CENTER, D, Color32::RED).clicked();
        });
        clicked
    }

    #[test]
    fn a_click_counts_on_the_disc_but_not_in_the_squares_corner() {
        for (at, on_disc) in [(CENTER, true), (CENTER + Vec2::splat(D / 2.0 - 1.0), false)] {
            let mut rig = Offscreen::new();
            let mut clicked = false;
            rig.click(at, &mut |ui, _| {
                clicked |=
                    filled_circle_button(ui, Id::new("disc"), CENTER, D, Color32::RED).clicked();
            });
            assert_eq!(clicked, on_disc, "{at:?}");
        }
    }

    #[test]
    fn a_hovered_filled_disc_or_tab_is_1_15_times_brighter_on_the_disc_only() {
        let fill = Color32::from_rgb(100, 120, 200);
        for tab in [false, true] {
            let mut rig = Offscreen::new();
            let drawn = |rig: &mut Offscreen, events| {
                let shapes = rig.frame(events, &mut |ui, _| {
                    filled_disc(ui, Id::new("disc"), CENTER, D, fill, tab, 1.0);
                });
                let discs: Vec<_> = (shapes.iter())
                    .filter_map(|c| match &c.shape {
                        egui::Shape::Circle(c) => Some((c.fill, c.radius)),
                        _ => None,
                    })
                    .collect();
                discs
            };
            drawn(&mut rig, vec![]);
            // On the disc: brighter, the same size (only a held tab grows).
            let on = drawn(
                &mut rig,
                vec![Event::PointerMoved(CENTER + Vec2::splat(5.0))],
            );
            assert_eq!(on, [(Color32::from_rgb(115, 138, 230), D / 2.0)], "{tab}");
            // In the square's corner, off the disc, and off the button: at rest.
            for at in [CENTER + Vec2::splat(D / 2.0 - 1.0), pos2(500.0, 500.0)] {
                let off = drawn(&mut rig, vec![Event::PointerMoved(at)]);
                assert_eq!(off, [(fill, D / 2.0)], "{tab} {at:?}");
            }
        }
    }

    #[test]
    fn enter_on_the_focused_button_clicks_it_without_a_pointer() {
        let mut rig = Offscreen::new();
        frame(&mut rig, vec![]);
        rig.ctx.memory_mut(|m| m.request_focus(Id::new("disc")));
        frame(&mut rig, vec![]);
        let key = |pressed| Event::Key {
            key: Key::Enter,
            physical_key: None,
            pressed,
            repeat: false,
            modifiers: Default::default(),
        };
        assert!(frame(&mut rig, vec![key(true), key(false)]));
    }
}
