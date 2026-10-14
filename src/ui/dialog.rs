//! The dialog base: no animation, the screen dimmed (`background` at α 0.6) and the box in `moduleFill`, both from the
//! first frame. Every dialog on the base paints its look through here: Song options, the Save dialog (and its preset,
//! sample-name and `Save as MIDI` forms), the tempo dialog, MIDI import, Fine tune, Channel settings and the
//! scale-guess result.
//!
//! Not on the base, as they look different: message boxes (`background`, opening from the centre), popups and menus,
//! and the level and swing popups, which sit where they were opened from.

use eframe::egui::{Context, Id, LayerId, Order, Painter, Rect, Response};

use super::anim;
use crate::theme::ThemeColors;

/// Paints the base: `screen` dimmed into `dim`, then `rect` in `moduleFill` into `boxed`. The two are the same painter
/// for a dialog drawn into its parent `Ui` (Song options, the tempo dialog), which is last in the frame.
pub(crate) fn paint_base(
    dim: &Painter,
    boxed: &Painter,
    c: &ThemeColors,
    screen: Rect,
    rect: Rect,
) {
    let ink = c.background.to_color32();
    dim.rect_filled(screen, 0.0, anim::dim(ink, anim::DIM_ALPHA));
    boxed.rect_filled(rect, 0.0, c.module_fill.to_color32());
}

/// [`paint_base`] for a dialog `id` whose contents are an `Area` `id` at `Order::Foreground`, over [`super::backdrop`]'s
/// area `id.with("backdrop")` at `Order::Middle`: the dim goes into the backdrop's layer and the box into the
/// contents', each straight into its layer before the area is shown, since egui lays a new area out unseen.
pub(crate) fn paint_in_layers(ctx: &Context, id: Id, c: &ThemeColors, screen: Rect, rect: Rect) {
    let layer = |order, id| ctx.layer_painter(LayerId::new(order, id));
    let (dim, boxed) = (
        layer(Order::Middle, id.with("backdrop")),
        layer(Order::Foreground, id),
    );
    paint_base(&dim, &boxed, c, screen, rect);
}

/// The base of dialog `id`, box `rect` on `screen` ([`paint_in_layers`]), and its backdrop ([`super::backdrop`]): the
/// response says whether a click landed outside. Call it every frame the dialog shows, before its area.
pub(crate) fn dialog_box(
    ctx: &Context,
    id: Id,
    c: &ThemeColors,
    screen: Rect,
    rect: Rect,
) -> Response {
    paint_in_layers(ctx, id, c, screen, rect);
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
/// under it, `rect` in `moduleFill`. The 0.6 is written out here rather than read from [`anim::DIM_ALPHA`].
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
    let boxed = at(rect, c.module_fill.to_color32());
    assert!(
        dim.is_some() && boxed.is_some(),
        "dim {dim:?}, box {boxed:?} in {painted:?}"
    );
    assert!(dim < boxed, "the dim is under the box");
}
