//! The fine tune dialog's slider: the mixer fader's skin turned on its side. The horizontal slider track (3 t thick,
//! inset 0.5 U from its ends) in `controlFill` with a channel-colour fill, and a `moduleFill` thumb, half-height
//! 0.625 U and half-width 0.3125 U, with a channel-colour inset of 2 px and a 2 px `menuFill` centre line across it. A
//! control at least 6.5 U tall is inset 0.875 U top and bottom and gets the 0.9375 U half-width. While pressed, a
//! channel-colour halo at α 0.4 extends 2 px around the thumb; desktop hover lights the track as on every slider.
//!
//! Input is the slider's ([`Law::Track`]): relative drag, the fine-tweak modifier and the double-tap reset to
//! [`Control::default`].

use eframe::egui::{Rect, Response, Ui, vec2};

use super::slider::{HALO, HALO_PX, fill, point, track};
use super::{Control, Law, drag_control, hovered, primary_down_on};
use crate::ui::UiCtx;

/// The thumb's half-height, in U.
const HALF_H: f32 = 0.625;
/// The thumb's half-width on a tall control, and on one under [`TALL`], in U.
const HALF_W: f32 = 0.9375;
const HALF_W_SHORT: f32 = 0.3125;
/// Controls at least this tall (U) are inset [`TALL_INSET`] U top and bottom and get the wide thumb.
const TALL: f32 = 6.5;
const TALL_INSET: f32 = 0.875;

/// What the slider paints, in drawing order after the halos.
#[derive(Clone, Copy, Debug, PartialEq)]
struct Parts {
    track: Rect,
    fill: Rect,
    /// `moduleFill`, centred on the value.
    thumb: Rect,
    /// The thumb inset 2 px, channel colour.
    thumb_inner: Rect,
    /// 2 px wide down the thumb's middle, inset 4 px from its top and bottom (the fader's line turned with it),
    /// `menuFill`.
    thumb_line: Rect,
}

/// The slider's parts in `rect` at value `v`, its fill from the middle when `bipolar`; `t` and `u` in points, `px` is
/// one device pixel in points.
fn parts(rect: Rect, v: f32, bipolar: bool, t: f32, u: f32, px: f32) -> Parts {
    let tall = rect.height() >= TALL * u;
    let body = if tall {
        rect.shrink2(vec2(0.0, TALL_INSET * u))
    } else {
        rect
    };
    let track = track(body, true, t, u);
    let half_w = if tall { HALF_W } else { HALF_W_SHORT } * u;
    let thumb = Rect::from_center_size(point(track, true, v), vec2(half_w, HALF_H * u) * 2.0);
    Parts {
        track,
        fill: fill(track, true, v, bipolar),
        thumb,
        thumb_inner: thumb.shrink(2.0 * px),
        thumb_line: Rect::from_center_size(
            thumb.center(),
            vec2(2.0 * px, thumb.height() - 8.0 * px),
        ),
    }
}

/// The fine tune slider in `c.rect` (no label or value strips): drags as a horizontal slider, a double tap resets to
/// `c.default`, and `c.menu` is its parameter menu.
pub fn fine_tune_slider(ui: &mut Ui, cx: &UiCtx, c: &Control, value: &mut f32) -> Response {
    let (m, colors) = (cx.m, &cx.theme.colors);
    let px = 1.0 / m.ppp();
    let at = |v| parts(c.rect, v, c.bipolar, m.t(), m.u(1.0), px);
    let law = Law::Track {
        len: at(0.0).track.width(),
        horizontal: true,
    };
    let (response, v) = drag_control(ui, cx, c, value, law);
    // Every frame, for its latch.
    let pressed = primary_down_on(ui, &response);
    let p = at(v);
    let (halo, painter) = (c.channel.gamma_multiply(HALO), ui.painter());
    if hovered(ui, &response) {
        painter.rect_filled(p.track.expand(HALO_PX * px), 0.0, halo);
    }
    painter.rect_filled(p.track, 0.0, colors.control_fill.to_color32());
    painter.rect_filled(p.fill, 0.0, c.channel);
    if pressed {
        painter.rect_filled(p.thumb.expand(HALO_PX * px), 0.0, halo);
    }
    painter.rect_filled(p.thumb, 0.0, colors.module_fill.to_color32());
    painter.rect_filled(p.thumb_inner, 0.0, c.channel);
    painter.rect_filled(p.thumb_line, 0.0, colors.menu_fill.to_color32());
    response
}

#[cfg(test)]
mod tests {
    use super::*;
    use eframe::egui::pos2;

    const U: f32 = 32.0;
    const T: f32 = 2.0;

    /// The dialog's slider row: a third of the 5 U content, 1000 points wide, at (64, 259.33).
    fn row() -> Rect {
        Rect::from_min_size(pos2(64.0, 206.0 + 160.0 / 3.0), vec2(1000.0, 160.0 / 3.0))
    }

    #[test]
    fn a_short_control_gets_the_narrow_thumb_on_a_track_across_it() {
        // ppp 1: px = 1 point.
        let (rect, px) = (row(), 1.0);
        let p = parts(rect, 0.25, false, T, U, px);
        // The track: 3 t thick, centred across the row, inset 0.5 U from both ends.
        let cy = rect.center().y;
        assert_eq!(
            p.track,
            Rect::from_x_y_ranges(64.0 + 16.0..=1064.0 - 16.0, cy - 3.0..=cy + 3.0)
        );
        // The fill from the left end to the value.
        assert_eq!(p.fill, p.track.with_max_x(80.0 + 0.25 * 968.0));
        // Thumb 2 × 0.3125 U wide, 2 × 0.625 U tall, on the value; inset 2 px; the line 2 px wide, 8 px shorter.
        let at = pos2(80.0 + 0.25 * 968.0, cy);
        assert_eq!(p.thumb, Rect::from_center_size(at, vec2(20.0, 40.0)));
        assert_eq!(p.thumb_inner, p.thumb.shrink(2.0));
        assert_eq!(p.thumb_line, Rect::from_center_size(at, vec2(2.0, 32.0)));
    }

    #[test]
    fn at_ppp_2_the_pixel_insets_halve_in_points() {
        // ppp 2: U = 32 points (64 px), t = 2 points (4 px), px = 0.5 point.
        let m =
            crate::metrics::Metrics::from_pixels_per_point(2.0, crate::metrics::DEFAULT_UI_SCALE);
        assert_eq!((m.u(1.0), m.t()), (U, T));
        let p = parts(row(), 1.0, false, m.t(), m.u(1.0), 1.0 / m.ppp());
        let at = pos2(1064.0 - 16.0, row().center().y);
        assert_eq!(p.thumb, Rect::from_center_size(at, vec2(20.0, 40.0)));
        assert_eq!(p.thumb_inner, p.thumb.shrink(1.0));
        assert_eq!(p.thumb_line, Rect::from_center_size(at, vec2(1.0, 36.0)));
        assert_eq!(p.fill, p.track);
    }

    #[test]
    fn from_6_5_u_tall_the_control_is_inset_and_the_thumb_is_wide() {
        let p = |h: f32| {
            parts(
                Rect::from_min_size(pos2(0.0, 0.0), vec2(400.0, h)),
                0.0,
                false,
                T,
                U,
                1.0,
            )
        };
        let tall = p(6.5 * U);
        assert_eq!(tall.thumb.size(), vec2(1.875 * U, 1.25 * U));
        // Inset 0.875 U top and bottom: the track stays centred across what is left.
        assert_eq!(tall.track.center().y, 3.25 * U);
        assert_eq!(tall.thumb.center(), pos2(0.5 * U, 3.25 * U));
        let short = p(6.5 * U - 1.0);
        assert_eq!(short.thumb.size(), vec2(0.625 * U, 1.25 * U));
    }

    #[test]
    fn a_bipolar_fill_runs_from_the_middle() {
        let p = parts(row(), 0.25, true, T, U, 1.0);
        let mid = p.track.center().x;
        assert_eq!((p.fill.left(), p.fill.right()), (80.0 + 0.25 * 968.0, mid));
        assert_eq!(parts(row(), 0.5, true, T, U, 1.0).fill.width(), 0.0);
    }
}
