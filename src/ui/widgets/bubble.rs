//! Honeydew change (#171): the value bubble. While a knob or slider is dragged, by the pointer or by another finger
//! (#68), a small bubble near the finger or pointer shows the control's value text, follows the drag and goes with the
//! release. It sits centred above the point, 2 U clear on touch so the finger leaves it in sight and 0.75 U above a
//! mouse pointer; beside it, level with it, where the screen's top leaves no room; always inside the screen. The
//! tooltip's look ([`crate::ui::tooltip`]), its text FontSize 3: `text` on a `menuFill` rounded rect (radius 0.25 U)
//! with a 1 px `text` α 0.15 outline, padded 0.4 U × 0.2 U. A control with no value text has none. Settings' `Show
//! value while dragging` turns it off ([`set_shown`]).

use eframe::egui::{Context, Id, LayerId, Order, Pos2, Rect, Stroke, StrokeKind, Vec2, pos2, vec2};

use crate::metrics::FontSize;
use crate::ui::UiCtx;

// Sizes in U.
const TOUCH_GAP: f32 = 2.0;
const MOUSE_GAP: f32 = 0.75;
const PAD_X: f32 = 0.4;
const PAD_Y: f32 = 0.2;
const RADIUS: f32 = 0.25;
const OUTLINE_ALPHA: f32 = 0.15;

fn shown_id() -> Id {
    Id::new("value bubble shown")
}

/// Settings' `Show value while dragging`, which the app sets every frame. Unset it is on, except in tests, where it is
/// off until the test sets it.
pub fn set_shown(ctx: &Context, on: bool) {
    ctx.data_mut(|d| d.insert_temp(shown_id(), on));
}

/// Draws the bubble with `text` for a drag at `at` (`touch`: a finger's), unless the setting is off or there is no text.
pub(super) fn show(ctx: &Context, cx: &UiCtx, at: Pos2, touch: bool, text: String) {
    let on = ctx.data(|d| d.get_temp(shown_id())).unwrap_or(!cfg!(test));
    if !on || text.is_empty() {
        return;
    }
    let (m, c) = (cx.m, &cx.theme.colors);
    let painter = ctx.layer_painter(LayerId::new(Order::Tooltip, Id::new("value bubble")));
    let ink = c.text.to_color32();
    let galley = painter.layout_no_wrap(text, m.font_id(FontSize::Pt12), ink);
    let pad = vec2(m.u(PAD_X), m.u(PAD_Y));
    let gap = m.u(if touch { TOUCH_GAP } else { MOUSE_GAP });
    let rect = place(ctx.content_rect(), at, galley.size() + 2.0 * pad, gap);
    let rect = Rect::from_min_size(
        pos2(m.round_px(rect.left()), m.round_px(rect.top())),
        rect.size(),
    );
    let radius = m.u(RADIUS);
    painter.rect_filled(rect, radius, c.menu_fill.to_color32());
    let outline = Stroke::new(1.0 / m.ppp(), ink.gamma_multiply(OUTLINE_ALPHA));
    painter.rect_stroke(rect, radius, outline, StrokeKind::Inside);
    painter.galley(rect.min + pad, galley, ink);
}

/// Where a bubble of `size` goes for a drag at `at`: centred above it, `gap` clear; where `screen`'s top leaves no
/// room, beside it (right, else left), `gap` clear and level with it; then kept inside `screen`.
fn place(screen: Rect, at: Pos2, size: Vec2, gap: f32) -> Rect {
    let min = if at.y - gap - size.y >= screen.top() {
        pos2(at.x - size.x / 2.0, at.y - gap - size.y)
    } else if at.x + gap + size.x <= screen.right() {
        pos2(at.x + gap, at.y - size.y / 2.0)
    } else {
        pos2(at.x - gap - size.x, at.y - size.y / 2.0)
    };
    let max = (screen.max - size).max(screen.min);
    Rect::from_min_size(min.clamp(screen.min, max), size)
}
