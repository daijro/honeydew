//! The ring knob and the graph knob.

use std::f32::consts::TAU;

use eframe::egui::{Align2, Pos2, Rect, Response, Ui, Vec2, vec2};

use super::arc::{FOCUS_GROWTH, pulse};
use super::curve::{curve_plot, env};
use super::text::{fit_both, fit_only};
use super::{Control, Law, drag_control, growth, hovered, primary_down_on};
use crate::ui::UiCtx;

/// Alpha of the channel wash inside the ring while the knob is touched or hovered.
pub(super) const WASH: f32 = 0.05;
/// Line width of the graph knob's curve, in device pixels.
const GRAPH_LINE_PX: f32 = 4.0;

/// The drawn parts of a ring knob of diameter `d` (the body's shorter side). The arc slider has the same radii.
#[derive(Clone, Copy, Debug, PartialEq)]
pub(super) struct Ring {
    /// Radius of the channel disc, 0.48 D − s.
    pub(super) outer: f32,
    /// Radius of the `moduleFill` disc on top of it, 0.48 D − 3 s. The ring left between them is 2 s wide.
    pub(super) inner: f32,
    /// The indicator dot's centre, on the ring's centre line.
    pub(super) dot: Pos2,
    /// Radius of the channel dot, 1.5 s′.
    pub(super) dot_r: f32,
    /// Radius of the `moduleFill` halo under the dot, 1.5 s′ + 2 t.
    pub(super) halo_r: f32,
}

/// Ring geometry at value `v`. The stroke s grows from `t` at rest to s′ = min(0.05 D, 2 t) at the far end.
pub(super) fn ring(center: Pos2, d: f32, v: f32, bipolar: bool, t: f32) -> Ring {
    let s_max = (0.05 * d).min(2.0 * t);
    let s = t + (s_max - t) * growth(v, bipolar);
    let (sin, cos) = dot_angle(v).sin_cos();
    Ring {
        outer: 0.48 * d - s,
        inner: 0.48 * d - 3.0 * s,
        // Clockwise from down, on screen (y down): down is +y, a quarter turn on is -x.
        dot: center + (0.48 * d - 2.0 * s) * vec2(-sin, cos),
        dot_r: 1.5 * s_max,
        halo_r: 1.5 * s_max + 2.0 * t,
    }
}

/// The indicator's angle in radians, clockwise from straight down: (0.05 + 0.9 v) turns.
fn dot_angle(v: f32) -> f32 {
    (0.05 + 0.9 * v) * TAU
}

/// The width the value text is fitted to: the inner-disc diameter less 4 hairlines.
pub(super) fn value_width(inner: f32, t: f32) -> f32 {
    2.0 * inner - 4.0 * t
}

/// Draws the ring and indicator dot of a ring knob at value `v` in `c`'s body. While `response` is pressed with the
/// primary button the knob swells, and wobbles back on release (the arc slider's pulse); pressed or hovered, the face
/// is washed and the dot grows × 1.05. Returns the ring's centre and geometry.
fn dial(ui: &Ui, cx: &UiCtx, c: &Control, response: &Response, v: f32) -> (Pos2, Ring) {
    let m = cx.m;
    let module_fill = cx.theme.colors.module_fill.to_color32();
    let painter = ui.painter();
    let (_, body, _) = c.strips(m);
    let touched = primary_down_on(ui, response);
    let lit = touched || hovered(ui, response);
    let swell = pulse(ui, c.id, touched, c.rect.height() * m.ppp());
    let center = body.center();
    let r = ring(center, diameter(body) * swell, v, c.bipolar, m.t());
    painter.circle_filled(center, r.outer, c.channel);
    painter.circle_filled(center, r.inner, module_fill);
    let grow = if lit {
        painter.circle_filled(center, r.inner, c.channel.gamma_multiply(WASH));
        FOCUS_GROWTH
    } else {
        1.0
    };
    painter.circle_filled(r.dot, grow * r.halo_r, module_fill);
    painter.circle_filled(r.dot, grow * r.dot_r, c.channel);
    (center, r)
}

/// The ring's diameter at rest in `body`.
fn diameter(body: Rect) -> f32 {
    body.width().min(body.height()).max(0.0)
}

/// A ring knob in `c.rect`: label strip on top, the ring centred in the rest with the value text inside.
pub fn knob(
    ui: &mut Ui,
    cx: &UiCtx,
    c: &Control,
    value: &mut f32,
    display: impl Fn(f32) -> String,
) -> Response {
    let (response, v) = drag_control(ui, cx, c, value, Law::Knob(1.0));
    let m = cx.m;
    c.strip_texts(ui.painter(), m, v, &display);
    let (center, _) = dial(ui, cx, c, &response, v);
    // Fit-only sizing: 12 pt down to 6 pt, and at 6 pt it is drawn whatever its width. Fitted to the resting ring,
    // so that the press swell never steps the size mid-press.
    let (_, body, _) = c.strips(m);
    let rest = ring(center, diameter(body), v, c.bipolar, m.t());
    let max_w = value_width(rest.inner, m.t());
    let galley = fit_only(ui.painter(), m, &display(v), max_w, c.channel);
    ui.painter()
        .galley(center - galley.size() / 2.0, galley, c.channel);
    response
}

/// The caption column of a drum-strip knob, in U.
const STRIP_CAPTION_W: f32 = 1.25;

/// A drum-strip knob: `c.label` in a column 1.25 U wide on the left, full height, centred, in `c.label_font` (8 pt);
/// the ring in the rest of the rect; and `display`'s text centred on the ring at that same fixed size, never fitted (an
/// empty text is none). Drags move it `s` times the knob law (`Start`: 0.5).
pub fn strip_knob(
    ui: &mut Ui,
    cx: &UiCtx,
    c: &Control,
    value: &mut f32,
    s: f32,
    display: impl Fn(f32) -> String,
) -> Response {
    let (response, v) = drag_control(ui, cx, c, value, Law::Knob(s));
    let m = cx.m;
    let split = (c.rect.left() + m.u(STRIP_CAPTION_W)).min(c.rect.right());
    let (caption, disc) = c.rect.split_left_right_at_x(split);
    let font = m.font_id(c.label_font);
    let at = Align2::CENTER_CENTER;
    (ui.painter()).text(caption.center(), at, c.label, font.clone(), c.channel);
    let face = Control {
        rect: disc,
        label: "",
        ..*c
    };
    let (center, _) = dial(ui, cx, &face, &response, v);
    let text = display(v);
    if !text.is_empty() {
        ui.painter().text(center, at, text, font, c.channel);
    }
    response
}

/// A graph knob (the Gate's `A. Curve` and `R. Curve`): a ring knob without value text, its label auto-sized
/// to the label strip, with the envelope curve Env(x, 2v − 1) ([`env`]) plotted on top in a 4 px channel line.
/// `falling` plots Env(1 − x, 2v − 1). The plot is a square centred on the ring, its side the inner disc's radius.
pub fn graph_knob(
    ui: &mut Ui,
    cx: &UiCtx,
    c: &Control,
    value: &mut f32,
    falling: bool,
) -> Response {
    let (response, v) = drag_control(ui, cx, c, value, Law::Knob(1.0));
    let m = cx.m;
    if let (Some(strip), ..) = c.strips(m) {
        let galley = fit_both(ui.painter(), m, c.label, strip.size(), c.channel);
        ui.painter()
            .galley(strip.center() - galley.size() / 2.0, galley, c.channel);
    }
    let (center, r) = dial(ui, cx, c, &response, v);
    let plot = Rect::from_center_size(center, Vec2::splat(r.inner.max(0.0)));
    let shape = 2.0 * v - 1.0;
    let lw = GRAPH_LINE_PX / m.ppp();
    curve_plot(ui.painter(), plot, lw, m.ppp(), c.channel, |x| {
        env(if falling { 1.0 - x } else { x }, shape)
    });
    response
}

#[cfg(test)]
mod tests {
    use super::*;
    use eframe::egui::pos2;

    fn near(a: f32, b: f32) -> bool {
        (a - b).abs() < 1e-3
    }

    const C: Pos2 = pos2(100.0, 100.0);

    #[test]
    fn instrument_spec_check_at_u_85() {
        // U = 85 px (t = 6), D = 180 px, value 0.5 with origin 0.5: s = 6, ring radii
        // 80.4 / 68.4 px, dot 13.5 px with a 25.5 px halo.
        let r = ring(C, 180.0, 0.5, true, 6.0);
        assert!(near(r.outer, 80.4) && near(r.inner, 68.4), "{r:?}");
        assert!(near(r.dot_r, 13.5) && near(r.halo_r, 25.5), "{r:?}");
    }

    #[test]
    fn stroke_grows_from_t_at_rest_to_s_prime_at_the_far_end() {
        let t = 6.0;
        // Rest: s = t. Unipolar rest is 0, bipolar rest is 0.5.
        for (v, bipolar) in [(0.0, false), (0.5, true)] {
            let r = ring(C, 180.0, v, bipolar, t);
            assert!(
                near(r.outer, 86.4 - t) && near(r.inner, 86.4 - 3.0 * t),
                "{v} {r:?}"
            );
        }
        // Far end: s = s′ = min(0.05 · 180, 2 t) = 9. Bipolar knobs reach it at either end.
        for (v, bipolar) in [(1.0, false), (0.0, true), (1.0, true)] {
            let r = ring(C, 180.0, v, bipolar, t);
            assert!(near(r.outer, 77.4) && near(r.inner, 59.4), "{v} {r:?}");
        }
        // Halfway: s = 7.5.
        let r = ring(C, 180.0, 0.5, false, t);
        assert!(near(r.outer, 78.9) && near(r.inner, 63.9), "{r:?}");
        // s′ is capped by 2 t on a large knob: D = 400 gives min(20, 12) = 12.
        let r = ring(C, 400.0, 1.0, false, t);
        assert!(near(r.outer, 192.0 - 12.0) && near(r.dot_r, 18.0), "{r:?}");
    }

    #[test]
    fn the_dot_uses_s_prime_whatever_the_value() {
        for v in [0.0, 0.3, 1.0] {
            let r = ring(C, 180.0, v, false, 6.0);
            assert!(near(r.dot_r, 13.5) && near(r.halo_r, 25.5), "{v} {r:?}");
        }
    }

    #[test]
    fn indicator_angle_runs_clockwise_from_seven_to_five_oclock() {
        assert!(near(dot_angle(0.0).to_degrees(), 18.0));
        assert!(near(dot_angle(0.5).to_degrees(), 180.0));
        assert!(near(dot_angle(1.0).to_degrees(), 342.0));
    }

    #[test]
    fn the_dot_rides_the_ring_centre_line() {
        // Bipolar at rest: straight up (12 o'clock), radius 0.48 D − 2 s = 86.4 − 12.
        let r = ring(C, 180.0, 0.5, true, 6.0);
        assert!(
            near(r.dot.x, 100.0) && near(r.dot.y, 100.0 - 74.4),
            "{:?}",
            r.dot
        );
        // v = 0 sits left of straight down (towards 7 o'clock), v = 1 right of it (towards 5 o'clock).
        let (lo, hi) = (
            ring(C, 180.0, 0.0, false, 6.0),
            ring(C, 180.0, 1.0, false, 6.0),
        );
        let rm = 86.4 - 12.0;
        let (sin, cos) = 18f32.to_radians().sin_cos();
        assert!(
            near(lo.dot.x, 100.0 - rm * sin) && near(lo.dot.y, 100.0 + rm * cos),
            "{:?}",
            lo.dot
        );
        let rm = 86.4 - 18.0;
        assert!(
            near(hi.dot.x, 100.0 + rm * sin) && near(hi.dot.y, 100.0 + rm * cos),
            "{:?}",
            hi.dot
        );
        // Clockwise means through 9 o'clock first: v = 0.25 sits near 9 o'clock, well left of the centre.
        let q = ring(C, 180.0, 0.25, false, 6.0);
        assert!(q.dot.x < 100.0 - 50.0, "{:?}", q.dot);
    }
}
