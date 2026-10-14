//! Widgets without input: the text label, the display placeholder and the separator. A spacer draws nothing, so it
//! has no widget: its leaf only takes layout space.
//! Graph displays use the display placeholder.

use eframe::egui::{Align2, Color32, Rect, Stroke, StrokeKind, Ui};

use crate::metrics::FontSize;
use crate::ui::UiCtx;

/// A static string in `rect`: `align` says which point of the rect the text hangs from (`Align2::CENTER_CENTER`,
/// `LEFT_CENTER`, `RIGHT_TOP` …). Its colour is usually the channel colour, so the caller passes it. Plain text: a
/// string wider than the rect overhangs it on the side away from the anchor, unclipped.
pub fn text_label(
    ui: &Ui,
    cx: &UiCtx,
    rect: Rect,
    text: &str,
    font: FontSize,
    align: Align2,
    color: Color32,
) {
    let font = cx.m.font_id(font);
    ui.painter()
        .text(align.pos_in_rect(&rect), align, text, font, color);
}

/// The placeholder for a module-drawn display: a `controlFill` rect with a one-pixel `background` outline. A later
/// milestone draws the meter, curve or graph on top of it.
pub fn display_area(ui: &Ui, cx: &UiCtx, rect: Rect) {
    let colors = &cx.theme.colors;
    ui.painter().rect(
        rect,
        0.0,
        colors.control_fill.to_color32(),
        Stroke::new(1.0 / cx.m.ppp(), colors.background.to_color32()),
        StrokeKind::Inside,
    );
}

/// Alpha of a separator.
const SEPARATOR_ALPHA: f32 = 0.1;

/// A separator (3x Osc MISC / Levels): `rect`, normally a 0.1 U column, filled `text` at 10 % alpha.
pub fn separator(ui: &Ui, cx: &UiCtx, rect: Rect) {
    let ink = cx
        .theme
        .colors
        .text
        .to_color32()
        .gamma_multiply(SEPARATOR_ALPHA);
    ui.painter().rect_filled(rect, 0.0, ink);
}
