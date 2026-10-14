//! The text toggle and the text button: a word with no background, 10 pt in the instrument tables (`c.label_font`),
//! centred in the rect inset 0.2 U left and right. Lit, it is `text`; dim,
//! `text` at 25 % alpha. The text is plain text, so the even inset leaves it centred on the rect, and a word wider
//! than that overhangs it evenly.
//!
//! A text toggle is bound to a parameter and is lit while on: it flips and opens the parameter menu like the pill
//! toggle. A text button (`AUTO`, `LOAD SAMPLE`, `Prev`, `Next`) has no parameter and no menu: it is lit only while
//! the primary button holds it down, and reports the press as a click. Both ignore `c.channel`.
//!
//! The drum strips' `Solo`, `REV` and `Poly` are not these but filled discs ([`disc_toggle`]).

use eframe::egui::{Align2, Response, Sense, Ui, WidgetInfo, WidgetType};

use super::toggle::flip;
use super::{Control, hovered, pressed, primary_down_on, toggle_on};
use crate::ui::{UiCtx, focus};

/// Alpha of the dim text.
const DIM_ALPHA: f32 = 0.25;

/// Draws `c.label` lit or dim, centred in `c.rect`.
fn draw(ui: &Ui, cx: &UiCtx, c: &Control, lit: bool) {
    let text = cx.theme.colors.text.to_color32();
    let color = if lit {
        text
    } else {
        text.gamma_multiply(DIM_ALPHA)
    };
    let font = cx.m.font_id(c.label_font);
    ui.painter()
        .text(c.rect.center(), Align2::CENTER_CENTER, c.label, font, color);
}

/// A text toggle bound to `value`: lit while on (any value but 0, [`toggle_on`]). A primary click flips it to 0 or 1;
/// the parameter menu opens without `Reset`.
pub fn text_toggle(ui: &mut Ui, cx: &UiCtx, c: &Control, value: &mut f32) -> Response {
    let (response, v) = flip(ui, cx, c, value);
    draw(ui, cx, c, toggle_on(v));
    response
}

/// A momentary text button: dim at rest, lit while the primary button is down on it. `clicked()` is the press
/// (primary, released on the button).
pub fn text_button(ui: &mut Ui, cx: &UiCtx, c: &Control) -> Response {
    let sense = if c.disabled {
        Sense::hover()
    } else {
        Sense::click()
    };
    let response = focus::interact(ui, c.rect, c.id, sense);
    response.widget_info(|| WidgetInfo::labeled(WidgetType::Button, !c.disabled, c.label));
    draw(ui, cx, c, primary_down_on(ui, &response));
    response
}

/// How much a drum-strip disc grows while the primary button holds it.
const DISC_PRESS: f32 = 1.05;

/// A drum-strip text button (`Solo`, `REV`, `Poly`): a disc of radius k·min(w, h)/2, 5 % larger while the primary
/// button holds it, with `c.label` centred on it in `c.label_font`. On, the disc is `text` and the label `moduleFill`;
/// off, the disc is `moduleFill` (unseen on the strip's row) and the label `text` at full alpha. It flips `value` as a
/// text toggle does, with `c.menu` its parameter menu; hovered, its disc is × 1.15.
pub fn disc_toggle(ui: &mut Ui, cx: &UiCtx, c: &Control, k: f32, value: &mut f32) -> Response {
    let (response, v) = flip(ui, cx, c, value);
    let colors = &cx.theme.colors;
    let (text, module_fill) = (colors.text.to_color32(), colors.module_fill.to_color32());
    let (fill, ink) = if toggle_on(v) {
        (text, module_fill)
    } else {
        (module_fill, text)
    };
    let fill = if hovered(ui, &response) {
        pressed(fill)
    } else {
        fill
    };
    let grow = if primary_down_on(ui, &response) {
        DISC_PRESS
    } else {
        1.0
    };
    let r = grow * k * 0.5 * c.rect.width().min(c.rect.height());
    let painter = ui.painter();
    painter.circle_filled(c.rect.center(), r, fill);
    let font = cx.m.font_id(c.label_font);
    painter.text(c.rect.center(), Align2::CENTER_CENTER, c.label, font, ink);
    response
}
