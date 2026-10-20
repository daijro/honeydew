//! The dialog base: no animation, the screen dimmed (`background` at α 0.6) and the floating panel, both from the first
//! frame. The panel is `menuFill` with corners of min(0.4 U, half its short side), inside a 2 px border of
//! 0.55·`menuFill` + 0.45·`text`. Every dialog on the base paints its look through here: Song options, the Save dialog
//! (and its preset, sample-name and `Save as MIDI` forms), the tempo dialog, MIDI import, Fine tune, Channel settings
//! and the scale-guess result; message boxes draw the same panel ([`panel`]).
//!
//! Not on the base, as they look different: popups and menus, and the level and swing popups, which sit where they
//! were opened from.

use eframe::egui::{
    Color32, Context, Id, LayerId, Order, Painter, Rangef, Rect, Response, Stroke, StrokeKind,
};

use super::anim;
use crate::metrics::{FontSize, Metrics};
use crate::theme::ThemeColors;
use crate::ui::widgets::Button;

/// The widest a floating dialog gets, in U.
const MAX_W: f32 = 20.0;
/// The panel's corner radius in U, at most half its short side.
const CORNER: f32 = 0.4;
/// The border, in device px.
const BORDER_PX: f32 = 2.0;
/// A dialog button's corner radius, in U.
const BUTTON_CORNER: f32 = 0.25;

/// A floating dialog's width on `screen`: min(20 U, W − 4 px), centred.
pub(crate) fn floating_x(screen: Rect, m: &Metrics) -> Rangef {
    let w = m.u(MAX_W).min(screen.width() - 4.0 / m.ppp());
    let left = m.round_px(screen.center().x - w / 2.0);
    Rangef::new(left, left + m.round_px(w))
}

/// Paints the floating panel `rect` into `painter` ([module docs](self)).
pub(crate) fn panel(painter: &Painter, c: &ThemeColors, m: &Metrics, rect: Rect) {
    let corner = m.u(CORNER).min(rect.width().min(rect.height()) / 2.0);
    let mix = |a: Color32, b: Color32, t: f32| {
        let ch = |x: u8, y: u8| (f32::from(x) * (1.0 - t) + f32::from(y) * t).round() as u8;
        Color32::from_rgb(ch(a.r(), b.r()), ch(a.g(), b.g()), ch(a.b(), b.b()))
    };
    let fill = c.menu_fill.to_color32();
    let border = mix(fill, c.text.to_color32(), 0.45);
    painter.rect_filled(rect, corner, fill);
    let stroke = Stroke::new(BORDER_PX / m.ppp(), border);
    painter.rect_stroke(rect, corner, stroke, StrokeKind::Outside);
}

/// A dialog's button: a `text` fill with a `moduleFill` 12 pt label, corners 0.25 U.
pub(crate) fn button<'a>(
    id: Id,
    rect: Rect,
    label: &'a str,
    c: &ThemeColors,
    m: &Metrics,
) -> Button<'a> {
    Button {
        font: FontSize::Pt12,
        corner: m.u(BUTTON_CORNER),
        text_on: c.module_fill.to_color32(),
        ..Button::new(id, rect, label, false, c)
    }
}

/// Paints the base: `screen` dimmed into `dim`, then the panel `rect` into `boxed`. The two are the same painter for a
/// dialog drawn into its parent `Ui` (Song options, the tempo dialog), which is last in the frame.
pub(crate) fn paint_base(
    dim: &Painter,
    boxed: &Painter,
    (c, m): (&ThemeColors, &Metrics),
    screen: Rect,
    rect: Rect,
) {
    let ink = c.background.to_color32();
    dim.rect_filled(screen, 0.0, anim::dim(ink, anim::DIM_ALPHA));
    panel(boxed, c, m, rect);
}

/// [`paint_base`] for a dialog `id` whose contents are an `Area` `id` at `Order::Foreground`, over [`super::backdrop`]'s
/// area `id.with("backdrop")` at `Order::Middle`: the dim goes into the backdrop's layer and the box into the
/// contents', each straight into its layer before the area is shown, since egui lays a new area out unseen.
pub(crate) fn paint_in_layers(
    ctx: &Context,
    id: Id,
    look: (&ThemeColors, &Metrics),
    screen: Rect,
    rect: Rect,
) {
    let layer = |order, id| ctx.layer_painter(LayerId::new(order, id));
    let (dim, boxed) = (
        layer(Order::Middle, id.with("backdrop")),
        layer(Order::Foreground, id),
    );
    paint_base(&dim, &boxed, look, screen, rect);
}

/// The base of dialog `id`, box `rect` on `screen` ([`paint_in_layers`]), and its backdrop ([`super::backdrop`]): the
/// response says whether a click landed outside. Call it every frame the dialog shows, before its area.
pub(crate) fn dialog_box(
    ctx: &Context,
    id: Id,
    look: (&ThemeColors, &Metrics),
    screen: Rect,
    rect: Rect,
) -> Response {
    paint_in_layers(ctx, id, look, screen, rect);
    super::backdrop(ctx, id)
}

/// The rects painted in `shapes` with their fills, in paint order.
#[cfg(test)]
pub(crate) fn painted_rects<'a>(
    shapes: impl IntoIterator<Item = &'a eframe::egui::Shape>,
) -> Vec<(Rect, eframe::egui::Color32)> {
    shapes
        .into_iter()
        .filter_map(|s| match s {
            eframe::egui::Shape::Rect(r) => Some((r.rect, r.fill)),
            _ => None,
        })
        .collect()
}

/// [`painted_rects`] of whole frames, each with the clip rect it was painted under: `(rect, fill, clip)`.
#[cfg(test)]
pub(crate) fn painted_clipped(
    shapes: &[eframe::egui::epaint::ClippedShape],
) -> Vec<(Rect, eframe::egui::Color32, Rect)> {
    shapes
        .iter()
        .filter_map(|c| match &c.shape {
            eframe::egui::Shape::Rect(r) => Some((r.rect, r.fill, c.clip_rect)),
            _ => None,
        })
        .collect()
}

/// Asserts the base in `painted` (rects and fills in paint order): all of `screen` dimmed to `background` α 0.6, and
/// under it, `rect` in `menuFill`. The 0.6 is written out here rather than read from [`anim::DIM_ALPHA`].
#[cfg(test)]
#[track_caller]
pub(crate) fn assert_base(
    painted: &[(Rect, eframe::egui::Color32)],
    c: &ThemeColors,
    screen: Rect,
    rect: Rect,
) {
    let at = |rect, fill| painted.iter().position(|&p| p == (rect, fill));
    let dim = at(screen, c.background.to_color32().gamma_multiply(0.6));
    let boxed = at(rect, c.menu_fill.to_color32());
    assert!(
        dim.is_some() && boxed.is_some(),
        "dim {dim:?}, box {boxed:?} in {painted:?}"
    );
    assert!(dim < boxed, "the dim is under the box");
}
