//! The XY pad (MultiFX; the Waveshaper has one per page): its frame, handle and axis names. The Waveshaper's transfer
//! curve is display content, drawn over it later.

use eframe::egui::{Pos2, Rect, Response, Sense, Ui, Vec2, WidgetInfo, WidgetType, pos2};

use super::slider::HALO;
use super::{Control, clean, primary_down_on};
use crate::ui::UiCtx;

/// Grid lines at every tenth: 11 a side, 1 px thick.
const GRID: usize = 10;
const GRID_PX: f32 = 1.0;
/// Thickness of the crosshair lines and radius of the dot, in device pixels.
const CROSS_PX: f32 = 2.0;
const DOT_PX: f32 = 3.0;
/// Width of the halo around a pressed pad, in device pixels.
const HALO_PX: f32 = 3.0;
/// From an axis name to its crosshair line, in U.
const NAME_GAP: f32 = 0.1;

/// The point of `rect` at (x, y): x from the left edge, y from the bottom one (Y up).
fn pad_point(rect: Rect, x: f32, y: f32) -> Pos2 {
    pos2(
        rect.left() + x * rect.width(),
        rect.bottom() - y * rect.height(),
    )
}

/// The values at `p`, the inverse of [`pad_point`], each clamped to [0, 1] (0 on an empty axis).
fn pad_values(rect: Rect, p: Pos2) -> (f32, f32) {
    let along = |d: f32, len: f32| {
        if len > 0.0 {
            (d / len).clamp(0.0, 1.0)
        } else {
            0.0
        }
    };
    (
        along(p.x - rect.left(), rect.width()),
        along(rect.bottom() - p.y, rect.height()),
    )
}

/// A line `px` device pixels thick across `rect`, centred on `at` (an x for a vertical line, a y for a horizontal
/// one), snapped to whole pixels and kept inside the rect. Also the module-drawn displays' grid and level lines.
pub(crate) fn line(rect: Rect, vertical: bool, at: f32, px: f32, ppp: f32) -> Rect {
    let (along, across) = if vertical {
        (rect.x_range(), rect.y_range())
    } else {
        (rect.y_range(), rect.x_range())
    };
    let thick = px / ppp;
    let start = ((at * ppp - px / 2.0).round() / ppp)
        .min(along.max - thick)
        .max(along.min);
    let band = start..=start + thick;
    if vertical {
        Rect::from_x_y_ranges(band, across)
    } else {
        Rect::from_x_y_ranges(across, band)
    }
}

/// Where the axis names of sizes `x` and `y` go for the crosshair at `at`: X right of the vertical line, Y above the
/// horizontal one, `gap` from them, each on the other side when it would leave `rect`. Along its line each sits `gap`
/// in from the pad's edge farther from the other line, so the two never meet.
fn names(rect: Rect, at: Pos2, x: Vec2, y: Vec2, gap: f32) -> [Rect; 2] {
    let left = if at.x + gap + x.x > rect.right() {
        at.x - gap - x.x
    } else {
        at.x + gap
    };
    let top = if at.y - gap - y.y < rect.top() {
        at.y + gap
    } else {
        at.y - gap - y.y
    };
    let x_top = if at.y > rect.center().y {
        rect.top() + gap
    } else {
        rect.bottom() - gap - x.y
    };
    let y_left = if at.x < rect.center().x {
        rect.right() - gap - y.x
    } else {
        rect.left() + gap
    };
    [
        Rect::from_min_size(pos2(left, x_top), x),
        Rect::from_min_size(pos2(y_left, top), y),
    ]
}

/// An XY pad in `c.rect`, bound to `x` and `y` (Y up). A primary press sets both values to the pointer, and a drag
/// follows it, clamped to the pad. `c.default` is what a NaN draws as, on either axis. The label is the axis names,
/// `"X · Y"` (none when empty), drawn in the label font and the channel colour; the strips are not used.
pub fn xy_pad(ui: &mut Ui, cx: &UiCtx, c: &Control, x: &mut f32, y: &mut f32) -> Response {
    let sense = if c.disabled {
        Sense::hover()
    } else {
        Sense::click_and_drag()
    };
    let mut response = ui.interact(c.rect, c.id, sense);
    let touched = primary_down_on(ui, &response);
    if touched && let Some(p) = ui.input(|i| i.pointer.interact_pos()) {
        let (px, py) = pad_values(c.rect, p);
        // `!=`: a NaN the user acted on is replaced.
        if px != *x || py != *y {
            (*x, *y) = (px, py);
            response.mark_changed();
        }
    }
    response.widget_info(|| WidgetInfo::labeled(WidgetType::Slider, !c.disabled, c.label));
    let (m, colors) = (cx.m, &cx.theme.colors);
    let ppp = m.ppp();
    let painter = ui.painter();
    if touched {
        painter.rect_filled(
            c.rect.expand(HALO_PX / ppp),
            0.0,
            c.channel.gamma_multiply(HALO),
        );
    }
    // The grid and the crosshair stay in the pad (they only leave it when it is empty); the dot is drawn whole, so
    // that values at the edges are not half cut.
    let pad = painter.with_clip_rect(c.rect.intersect(painter.clip_rect()));
    pad.rect_filled(c.rect, 0.0, colors.control_fill.to_color32());
    let grid = colors.background.to_color32();
    for i in 0..=GRID {
        let f = i as f32 / GRID as f32;
        let at = pad_point(c.rect, f, f);
        pad.rect_filled(line(c.rect, true, at.x, GRID_PX, ppp), 0.0, grid);
        pad.rect_filled(line(c.rect, false, at.y, GRID_PX, ppp), 0.0, grid);
    }
    let at = pad_point(c.rect, clean(*x, c.default), clean(*y, c.default));
    pad.rect_filled(line(c.rect, true, at.x, CROSS_PX, ppp), 0.0, c.channel);
    pad.rect_filled(line(c.rect, false, at.y, CROSS_PX, ppp), 0.0, c.channel);
    painter.circle_filled(at, DOT_PX / ppp, c.channel);
    let galleys: Vec<_> = (c.label.split(" · ").filter(|t| !t.is_empty()))
        .map(|t| pad.layout_no_wrap(t.to_owned(), m.font_id(c.label_font), c.channel))
        .collect();
    let size = |i: usize| galleys.get(i).map_or(Vec2::ZERO, |g| g.size());
    let rects = names(c.rect, at, size(0), size(1), m.u(NAME_GAP));
    for (galley, r) in galleys.into_iter().zip(rects) {
        pad.galley(r.min, galley, c.channel);
    }
    response
}

#[cfg(test)]
mod tests {
    use super::*;
    use eframe::egui::{pos2, vec2};

    const R: Rect = Rect {
        min: pos2(10.0, 20.0),
        max: pos2(110.0, 70.0),
    };

    #[test]
    fn values_map_to_the_pad_with_y_up_and_back() {
        assert_eq!(pad_point(R, 0.0, 0.0), pos2(10.0, 70.0));
        assert_eq!(pad_point(R, 1.0, 1.0), pos2(110.0, 20.0));
        assert_eq!(pad_point(R, 0.25, 0.75), pos2(35.0, 32.5));
        for (x, y) in [(0.0, 0.0), (0.25, 0.75), (0.5, 0.1), (1.0, 1.0)] {
            assert_eq!(pad_values(R, pad_point(R, x, y)), (x, y));
        }
    }

    #[test]
    fn points_outside_the_pad_clamp_and_empty_axes_read_0() {
        assert_eq!(pad_values(R, pos2(-50.0, 500.0)), (0.0, 0.0));
        assert_eq!(pad_values(R, pos2(500.0, -50.0)), (1.0, 1.0));
        let flat = Rect::from_min_max(pos2(10.0, 20.0), pos2(10.0, 20.0));
        assert_eq!(pad_values(flat, pos2(10.0, 20.0)), (0.0, 0.0));
    }

    #[test]
    fn axis_names_sit_beside_the_crosshair_and_flip_to_stay_in_the_pad() {
        let (x, y) = (vec2(30.0, 8.0), vec2(20.0, 8.0));
        // Crossing low and left: X right of the vertical line, at the top; Y above the horizontal one, at the right;
        // 2 from the lines and the edges.
        assert_eq!(
            names(R, pos2(35.0, 50.0), x, y, 2.0),
            [
                Rect::from_min_size(pos2(37.0, 22.0), x),
                Rect::from_min_size(pos2(88.0, 40.0), y),
            ]
        );
        // High and right: X at the bottom, left of its line by the right edge; Y at the left, under its line by the
        // top edge.
        assert_eq!(
            names(R, pos2(90.0, 25.0), x, y, 2.0),
            [
                Rect::from_min_size(pos2(58.0, 60.0), x),
                Rect::from_min_size(pos2(12.0, 27.0), y),
            ]
        );
        // In a corner the names keep apart.
        for at in [R.left_bottom(), R.right_top(), R.min, R.max] {
            let [a, b] = names(R, at, x, y, 2.0);
            assert!(
                !a.intersects(b) && R.contains_rect(a) && R.contains_rect(b),
                "{at:?}"
            );
        }
    }

    #[test]
    fn lines_snap_to_whole_pixels_inside_the_pad() {
        // 1 px at ppp 2 is half a point; centred on 35.3 it covers the pixel column 70..71 (35.0..35.5).
        assert_eq!(
            line(R, true, 35.3, 1.0, 2.0),
            Rect::from_x_y_ranges(35.0..=35.5, 20.0..=70.0)
        );
        // 2 px at ppp 1 centred on y = 32.5: rows 31.5 rounded to 32, so 32..34.
        assert_eq!(
            line(R, false, 32.5, 2.0, 1.0),
            Rect::from_x_y_ranges(10.0..=110.0, 32.0..=34.0)
        );
        // On an edge the line moves inside: the right edge, and the top edge.
        assert_eq!(
            line(R, true, 110.0, 2.0, 1.0),
            Rect::from_x_y_ranges(108.0..=110.0, 20.0..=70.0)
        );
        assert_eq!(
            line(R, false, 20.0, 1.0, 1.0),
            Rect::from_x_y_ranges(10.0..=110.0, 20.0..=21.0)
        );
    }
}
