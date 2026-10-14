//! The mixer fader: the vertical slider's track and fill with a rectangular cap instead of the round thumb, and two
//! unity-gain ticks.

use eframe::egui::{Rect, Response, Ui, pos2, vec2};

use super::slider::{HALO, HALO_PX, fill, point, track};
use super::{Control, Law, drag_control, hovered};
use crate::ui::UiCtx;

/// The level value at unity gain, where the ticks sit (14.688 % of the track from the top).
pub const UNITY: f32 = 0.85312;
/// Fader rects at least this tall (U) are shrunk at both ends and get the tall cap.
const TALL: f32 = 6.5;

/// What a fader paints, in drawing order after the track's fill.
#[derive(Clone, Copy, Debug, PartialEq)]
struct Parts {
    track: Rect,
    fill: Rect,
    /// Left and right of the track at the unity value: 0.5 U × 2 px, `menuFill`.
    ticks: [Rect; 2],
    /// `moduleFill`, 1.25 U wide, 1.875 U tall (0.625 U on a short fader), centred on the value.
    cap: Rect,
    /// The cap inset 2 px, channel colour.
    cap_inner: Rect,
    /// 2 px across the cap's middle, inset 4 px from its sides, `menuFill`.
    cap_line: Rect,
}

/// The fader's parts in `rect` at value `v`; `t` and `u` in points, `px` is one device pixel in points.
fn parts(rect: Rect, v: f32, t: f32, u: f32, px: f32) -> Parts {
    let tall = rect.height() >= TALL * u;
    let body = if tall {
        rect.shrink2(vec2(0.0, 0.875 * u))
    } else {
        rect
    };
    let track = track(body, false, t, u);
    let cap_h = if tall { 1.875 } else { 0.625 } * u;
    let cap = Rect::from_center_size(point(track, false, v), vec2(1.25 * u, cap_h));
    let tick_y = point(track, false, UNITY).y - px;
    let tick = |x: f32| Rect::from_min_size(pos2(x, tick_y), vec2(0.5 * u, 2.0 * px));
    Parts {
        track,
        fill: fill(track, false, v, false),
        ticks: [tick(track.left() - 0.5 * u), tick(track.right())],
        cap,
        cap_inner: cap.shrink(2.0 * px),
        cap_line: Rect::from_center_size(cap.center(), vec2(cap.width() - 8.0 * px, 2.0 * px)),
    }
}

/// A fader in `c.rect` (no label or value strips): drags as a vertical slider, a double tap resets to `c.default`.
pub fn fader(ui: &mut Ui, cx: &UiCtx, c: &Control, value: &mut f32) -> Response {
    let (m, colors) = (cx.m, &cx.theme.colors);
    let at = |v| parts(c.rect, v, m.t(), m.u(1.0), 1.0 / m.ppp());
    let len = at(0.0).track.height();
    let law = Law::Track {
        len,
        horizontal: false,
    };
    let (response, v) = drag_control(ui, cx, c, value, law);
    let p = at(v);
    let (menu_fill, painter) = (colors.menu_fill.to_color32(), ui.painter());
    // Desktop hover, as the slider's.
    if hovered(ui, &response) {
        let halo = c.channel.gamma_multiply(HALO);
        painter.rect_filled(p.track.expand(HALO_PX / m.ppp()), 0.0, halo);
    }
    painter.rect_filled(p.track, 0.0, colors.control_fill.to_color32());
    painter.rect_filled(p.fill, 0.0, c.channel);
    for tick in p.ticks {
        painter.rect_filled(tick, 0.0, menu_fill);
    }
    painter.rect_filled(p.cap, 0.0, colors.module_fill.to_color32());
    painter.rect_filled(p.cap_inner, 0.0, c.channel);
    painter.rect_filled(p.cap_line, 0.0, menu_fill);
    response
}

#[cfg(test)]
mod tests {
    use super::*;

    fn near(a: f32, b: f32) -> bool {
        (a - b).abs() < 1e-3
    }

    const U: f32 = 32.0;
    const T: f32 = 2.0;

    #[test]
    fn a_tall_fader_shrinks_by_0_875_u_and_gets_the_tall_cap() {
        // 8 U tall, 2.4 U wide (0.6 × the 4 U strip) at (10, 20).
        let rect = Rect::from_min_size(pos2(10.0, 20.0), vec2(2.4 * U, 8.0 * U));
        let p = parts(rect, 0.5, T, U, 1.0);
        // Shrunk 0.875 U top and bottom, then the slider's track: 3 t wide, inset 0.5 U.
        let (top, bottom) = (20.0 + 1.375 * U, 20.0 + 8.0 * U - 1.375 * U);
        let cx = rect.center().x;
        assert_eq!(
            p.track,
            Rect::from_x_y_ranges(cx - 3.0..=cx + 3.0, top..=bottom)
        );
        // The fill runs from the bottom to the value.
        assert_eq!(p.fill, p.track.with_min_y(p.track.center().y));
        // Cap 1.25 × 1.875 U on the value; inner inset 2 px; the line 2 px tall, 8 px narrower.
        let at = p.track.center();
        assert_eq!(p.cap, Rect::from_center_size(at, vec2(1.25 * U, 1.875 * U)));
        assert_eq!(p.cap_inner, p.cap.shrink(2.0));
        assert_eq!(
            p.cap_line,
            Rect::from_center_size(at, vec2(1.25 * U - 8.0, 2.0))
        );
    }

    #[test]
    fn a_short_fader_keeps_its_rect_and_gets_the_short_cap() {
        let rect = Rect::from_min_size(pos2(0.0, 0.0), vec2(2.4 * U, 4.0 * U));
        let p = parts(rect, 1.0, T, U, 1.0);
        assert_eq!(p.track.y_range(), 0.5 * U..=3.5 * U);
        assert_eq!(
            p.cap,
            Rect::from_center_size(pos2(1.2 * U, 0.5 * U), vec2(1.25 * U, 0.625 * U))
        );
        assert_eq!(p.fill, p.track);
    }

    #[test]
    fn the_tall_rules_start_at_exactly_6_5_u() {
        let p = |h: f32| {
            parts(
                Rect::from_min_size(pos2(0.0, 0.0), vec2(2.4 * U, h)),
                0.0,
                T,
                U,
                1.0,
            )
        };
        let tall = p(6.5 * U);
        assert_eq!(tall.track.top(), 1.375 * U);
        assert_eq!(tall.cap.height(), 1.875 * U);
        let short = p(6.5 * U - 1.0);
        assert_eq!(short.track.top(), 0.5 * U);
        assert_eq!(short.cap.height(), 0.625 * U);
    }

    #[test]
    fn the_unity_ticks_sit_at_14_688_percent_from_the_top() {
        let rect = Rect::from_min_size(pos2(0.0, 0.0), vec2(2.4 * U, 10.0 * U));
        let p = parts(rect, 0.0, T, U, 0.5);
        let tr = p.track;
        for tick in p.ticks {
            assert!(
                near((tick.center().y - tr.top()) / tr.height(), 0.14688),
                "{tick:?}"
            );
            // 0.5 U × 2 px (px = 0.5 points here).
            assert_eq!(tick.size(), vec2(0.5 * U, 1.0));
        }
        // Flush against the track's sides.
        assert_eq!(p.ticks[0].right(), tr.left());
        assert_eq!(p.ticks[1].left(), tr.right());
        // Device pixels: 2 px inset on a ppp 2 screen is 1 point.
        assert_eq!(p.cap_inner, p.cap.shrink(1.0));
    }

    #[test]
    fn device_check_at_u_51() {
        // At 60 % scale and 510 dpi (ppp 510 / 192 with the desktop boost), U = 51 px and t = 4 px, and the cap's inner
        // rect is about 60 × 92 px. The parts are measured in device pixels (px = 1).
        let m = crate::metrics::Metrics::from_pixels_per_point(510.0 / 192.0, 8);
        let (u, t) = (m.u_px(), (m.t() * m.ppp()).round());
        assert_eq!((u, t), (51.0, 4.0));
        let rect = Rect::from_min_size(pos2(0.0, 0.0), vec2(0.6 * 4.0 * u, 10.0 * u));
        let inner = parts(rect, UNITY, t, u, 1.0).cap_inner;
        assert_eq!(
            (inner.width().round(), inner.height().round()),
            (60.0, 92.0)
        );
    }
}
