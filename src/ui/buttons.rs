//! Small round buttons and the captioned transport button. Callers draw the content (icon, glyph or text)
//! into the returned response's rect. Every interactive widget takes an explicit [`Id`].

use eframe::egui::{self, Align2, Color32, FontId, Id, Pos2, Rect, Response, Sense, Stroke, Vec2};

use crate::metrics::{FontSize, Metrics};
use crate::theme::ThemeColors;
use crate::ui::focus;
use crate::ui::widgets::{hovered, pressed};

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
