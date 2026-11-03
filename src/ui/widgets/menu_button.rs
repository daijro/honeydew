//! The menu button (3x Osc `Chord`): the caption `<label>:   ` (label, colon, three spaces) in `text` 10 pt, then a
//! `controlFill` box with the current item in the channel colour, 10 pt, centred with a 0.2 U inset (plain text: the
//! even inset leaves it centred on the box). The caption takes its own width from the rect's left, the box the rest.
//! A tap on the box opens the spinner's popup; the caption takes no input.

use eframe::egui::{Align2, Rect, Response, Sense, Ui};

use super::spinner::{combo_info, item_popup, open_popup};
use super::{Control, through_index};
use crate::metrics::FontSize;
use crate::ui::{UiCtx, focus};

/// Caption and value are FontSize 2.
const FONT: FontSize = FontSize::Pt10;

/// The box: `rect` less the caption's width at the left (nothing left when the caption is wider).
fn box_rect(rect: Rect, caption_width: f32) -> Rect {
    rect.with_min_x((rect.left() + caption_width).min(rect.right()))
}

/// A menu button bound to a menu parameter: `value` is its normalised value, `items` the menu's labels. Only
/// `c.id`, `c.rect`, `c.label`, `c.channel`, `c.disabled` and the popup's `c.column_len` and `c.disabled_items` matter.
/// An empty label draws no caption.
pub fn menu_button(
    ui: &mut Ui,
    cx: &UiCtx,
    c: &Control,
    items: &[&str],
    value: &mut f32,
) -> Response {
    through_index(value, items.len(), |index| {
        let (m, colors, n) = (cx.m, &cx.theme.colors, items.len());
        let font = m.font_id(FONT);
        let caption = (!c.label.is_empty()).then(|| {
            let text = format!("{}:   ", c.label);
            ui.painter()
                .layout_no_wrap(text, font.clone(), colors.text.to_color32())
        });
        let boxed = box_rect(c.rect, caption.as_ref().map_or(0.0, |g| g.size().x));
        let sense = if c.disabled || n == 0 {
            Sense::hover()
        } else {
            Sense::click()
        };
        let mut response = focus::interact(ui, boxed, c.id, sense);

        let current = (*index).min(n.saturating_sub(1));
        if response.clicked() {
            open_popup(ui, c.id, boxed.center());
        }
        let current = match item_popup(ui, cx, c, items, current) {
            Some(i) if i != current => {
                *index = i;
                response.mark_changed();
                i
            }
            _ => current,
        };

        let text = items.get(current).copied().unwrap_or_default();
        response.widget_info(|| combo_info(c, text));

        let painter = ui.painter();
        if let Some(galley) = caption {
            let pos = Align2::LEFT_CENTER
                .anchor_size(c.rect.left_center(), galley.size())
                .min;
            painter.galley(pos, galley, colors.text.to_color32());
        }
        painter.rect_filled(boxed, 0.0, colors.control_fill.to_color32());
        painter.text(boxed.center(), Align2::CENTER_CENTER, text, font, c.channel);
        response
    })
}
