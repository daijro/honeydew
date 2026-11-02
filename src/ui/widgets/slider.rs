//! Vertical and horizontal sliders.
//! The horizontal one is the vertical one turned on its side: same strips, the track runs left to right.

use eframe::egui::{Pos2, Rect, Response, Ui, vec2};

use super::{Control, Law, drag_control, growth, hovered, primary_down_on};
use crate::ui::UiCtx;

/// Alpha of the channel halo around the track while the slider is touched or hovered.
pub(super) const HALO: f32 = 0.4;
/// Width of that halo, in device pixels.
pub(super) const HALO_PX: f32 = 2.0;

/// The track: 3 t thick, centred across the body, inset 0.5 U from both ends of the body along it.
pub(super) fn track(body: Rect, horizontal: bool, t: f32, u: f32) -> Rect {
    let c = body.center();
    if horizontal {
        Rect::from_x_y_ranges(
            body.left() + 0.5 * u..=body.right() - 0.5 * u,
            c.y - 1.5 * t..=c.y + 1.5 * t,
        )
    } else {
        Rect::from_x_y_ranges(
            c.x - 1.5 * t..=c.x + 1.5 * t,
            body.top() + 0.5 * u..=body.bottom() - 0.5 * u,
        )
    }
}

/// The point at fraction `f` of the track's centre line, from its low end (bottom, or left when horizontal).
pub(super) fn point(track: Rect, horizontal: bool, f: f32) -> Pos2 {
    let c = track.center();
    if horizontal {
        Pos2::new(track.left() + f * track.width(), c.y)
    } else {
        Pos2::new(c.x, track.bottom() - f * track.height())
    }
}

/// The channel-coloured fill over the track: from the low end to the value, or from the middle when bipolar.
pub(super) fn fill(track: Rect, horizontal: bool, v: f32, bipolar: bool) -> Rect {
    let from = if bipolar { 0.5 } else { 0.0 };
    let half = if horizontal {
        vec2(0.0, track.height() / 2.0)
    } else {
        vec2(track.width() / 2.0, 0.0)
    };
    Rect::from_two_pos(point(track, horizontal, from), point(track, horizontal, v)).expand2(half)
}

/// Outer thumb radius: 4.5 t at rest, growing to 5.985 t at the far end. The inner disc is 2 t smaller.
fn thumb_radius(growth: f32, t: f32) -> f32 {
    (4.5 + 1.485 * growth) * t
}

/// The thumb's centre and outer radius at value `v`: it sits on the value and grows away from rest (the low end,
/// or the middle when bipolar).
fn thumb(track: Rect, horizontal: bool, v: f32, bipolar: bool, t: f32) -> (Pos2, f32) {
    (
        point(track, horizontal, v),
        thumb_radius(growth(v, bipolar), t),
    )
}

/// A vertical slider: label strip on top, optional value strip at the bottom, the track between.
pub fn v_slider(
    ui: &mut Ui,
    cx: &UiCtx,
    c: &Control,
    value: &mut f32,
    display: impl Fn(f32) -> String,
) -> Response {
    slider(ui, cx, c, value, display, false)
}

/// A horizontal slider: the strips of [`v_slider`], with the track running left to right.
pub fn h_slider(
    ui: &mut Ui,
    cx: &UiCtx,
    c: &Control,
    value: &mut f32,
    display: impl Fn(f32) -> String,
) -> Response {
    slider(ui, cx, c, value, display, true)
}

fn slider(
    ui: &mut Ui,
    cx: &UiCtx,
    c: &Control,
    value: &mut f32,
    display: impl Fn(f32) -> String,
    horizontal: bool,
) -> Response {
    let (m, colors) = (cx.m, &cx.theme.colors);
    let (_, body, _) = c.strips(m);
    let t = m.t();
    let track = track(body, horizontal, t, m.u(1.0));
    let len = if horizontal {
        track.width()
    } else {
        track.height()
    };
    let (response, v) = drag_control(ui, cx, c, value, Law::Track { len, horizontal });
    let painter = ui.painter();
    c.strip_texts(painter, m, v, display);

    let (body_ink, ink) = match c.flat {
        Some(flat) => (flat, flat),
        None if c.disabled => (
            colors.control_fill.to_color32(),
            colors.menu_fill.to_color32(),
        ),
        None => (colors.control_fill.to_color32(), c.channel),
    };
    if primary_down_on(ui, &response) || hovered(ui, &response) {
        painter.rect_filled(
            track.expand(HALO_PX / m.ppp()),
            0.0,
            c.channel.gamma_multiply(HALO),
        );
    }
    painter.rect_filled(track, 0.0, body_ink);
    painter.rect_filled(fill(track, horizontal, v, c.bipolar), 0.0, ink);
    let (center, r) = thumb(track, horizontal, v, c.bipolar, t);
    painter.circle_filled(center, r, body_ink);
    painter.circle_filled(center, r - 2.0 * t, ink);
    response
}

#[cfg(test)]
mod tests {
    use super::*;
    use eframe::egui::pos2;

    fn near(a: f32, b: f32) -> bool {
        (a - b).abs() < 1e-3
    }

    fn rect(x0: f32, y0: f32, x1: f32, y1: f32) -> Rect {
        Rect::from_min_max(pos2(x0, y0), pos2(x1, y1))
    }

    #[test]
    fn instrument_spec_check_at_u_85() {
        // U = 85 px (t = 6): track 18 × 126 px, thumb 27 / 15 px at rest.
        let body = rect(0.0, 0.0, 100.0, 126.0 + 85.0);
        let tr = track(body, false, 6.0, 85.0);
        assert_eq!(tr, rect(41.0, 42.5, 59.0, 168.5));
        assert_eq!(thumb_radius(0.0, 6.0), 27.0);
        assert_eq!(thumb_radius(0.0, 6.0) - 2.0 * 6.0, 15.0);
        // Turned on its side.
        let body = rect(0.0, 0.0, 126.0 + 85.0, 100.0);
        assert_eq!(track(body, true, 6.0, 85.0), rect(42.5, 41.0, 168.5, 59.0));
    }

    #[test]
    fn thumb_grows_to_5_985_t_at_the_far_end() {
        assert!(near(thumb_radius(1.0, 6.0), 5.985 * 6.0));
        assert!(near(thumb_radius(0.5, 2.0), (4.5 + 1.485 / 2.0) * 2.0));
    }

    #[test]
    fn bipolar_thumbs_grow_symmetrically_from_the_middle() {
        let t = 2.0;
        for (tr, horizontal) in [
            (rect(0.0, 0.0, 4.0, 100.0), false),
            (rect(0.0, 0.0, 100.0, 4.0), true),
        ] {
            let at = |v| thumb(tr, horizontal, v, true, t);
            // At rest in the middle: 4.5 t.
            assert_eq!(at(0.5), (point(tr, horizontal, 0.5), 9.0));
            // A quarter of the range either side: half the growth, the same both ways.
            let (lo, hi) = (at(0.25), at(0.75));
            let half = (4.5 + 1.485 / 2.0) * t;
            assert!(near(lo.1, half) && near(hi.1, half), "{lo:?} {hi:?}");
            assert_eq!(lo.0, point(tr, horizontal, 0.25));
            assert_eq!(hi.0, point(tr, horizontal, 0.75));
            // Full size at both ends.
            assert!(near(at(0.0).1, 5.985 * t) && near(at(1.0).1, 5.985 * t));
            // Unipolar for contrast: rest at the low end, a quarter of the growth at 0.25.
            let (_, r) = thumb(tr, horizontal, 0.25, false, t);
            assert!(near(r, (4.5 + 1.485 / 4.0) * t), "{r}");
            assert_eq!(thumb(tr, horizontal, 0.0, false, t).1, 9.0);
        }
    }

    #[test]
    fn the_thumb_sits_at_the_value_from_the_low_end() {
        let tr = rect(0.0, 0.0, 4.0, 100.0);
        assert_eq!(point(tr, false, 0.0), pos2(2.0, 100.0));
        assert_eq!(point(tr, false, 0.25), pos2(2.0, 75.0));
        assert_eq!(point(tr, false, 1.0), pos2(2.0, 0.0));
        let tr = rect(0.0, 0.0, 100.0, 4.0);
        assert_eq!(point(tr, true, 0.0), pos2(0.0, 2.0));
        assert_eq!(point(tr, true, 0.25), pos2(25.0, 2.0));
        assert_eq!(point(tr, true, 1.0), pos2(100.0, 2.0));
    }

    #[test]
    fn unipolar_fill_runs_from_the_low_end_to_the_value() {
        let tr = rect(0.0, 0.0, 4.0, 100.0);
        assert_eq!(fill(tr, false, 0.25, false), rect(0.0, 75.0, 4.0, 100.0));
        assert_eq!(fill(tr, false, 1.0, false), tr);
        assert_eq!(fill(tr, false, 0.0, false).height(), 0.0);
        let tr = rect(0.0, 0.0, 100.0, 4.0);
        assert_eq!(fill(tr, true, 0.25, false), rect(0.0, 0.0, 25.0, 4.0));
    }

    #[test]
    fn bipolar_fill_runs_from_the_middle_to_the_value() {
        let tr = rect(0.0, 0.0, 4.0, 100.0);
        assert_eq!(fill(tr, false, 0.25, true), rect(0.0, 50.0, 4.0, 75.0));
        assert_eq!(fill(tr, false, 0.75, true), rect(0.0, 25.0, 4.0, 50.0));
        assert_eq!(fill(tr, false, 0.5, true).height(), 0.0);
        let tr = rect(0.0, 0.0, 100.0, 4.0);
        assert_eq!(fill(tr, true, 0.25, true), rect(25.0, 0.0, 50.0, 4.0));
        assert_eq!(fill(tr, true, 1.0, true), rect(50.0, 0.0, 100.0, 4.0));
    }
}
