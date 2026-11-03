//! The arc slider (Tape Stop uses four of them).

use std::f32::consts::TAU;

use eframe::egui::{Color32, Id, Painter, Pos2, Response, Shape, Stroke, Ui, vec2};

use super::knob::{WASH, ring, value_width};
use super::text::fit_only;
use super::{Control, Law, drag_control, hovered, primary_down_on};
use crate::ui::UiCtx;

/// Degrees between the points of a sector's arc.
const STEP_DEG: f32 = 3.0;
/// Width of a separator, which is also how much narrower than its segment a filled segment is, in degrees.
const GAP_DEG: f32 = 2.0;
/// Half the angle of the segmented style's pointer, in degrees.
const POINTER_DEG: f32 = 4.5;
/// The pointer's tip and base, in U outside the ring's centre line.
const POINTER_TIP_U: f32 = 0.3;
const POINTER_BASE_U: f32 = 0.7;
/// How far the touch pulse swells the arc, in device pixels at f = 1.
const PULSE_PX: f32 = 10.0;
/// The pulse value q is multiplied by this every frame, counted at 60 frames a second.
const PULSE_DECAY: f32 = 0.6;
/// Below this q the pulse is over.
const PULSE_FLOOR: f32 = 0.01;
/// The handle's (and the knob's dot's) size while touched or hovered.
pub(super) const FOCUS_GROWTH: f32 = 1.05;

/// What makes an arc slider besides its [`Control`]. Angles are in degrees, counter-clockwise from +x with y up.
/// The main colour is the control's channel colour, and `steps` of two or more makes the arc segmented, one segment
/// per step.
#[derive(Clone, Copy, Debug)]
pub struct ArcSlider {
    pub start: f32,
    pub span: f32,
    /// Value 0 sits at `start + span` and the fill grows back towards `start`.
    pub mirrored: bool,
    /// Punches the ring out of the sectors and draws the separators. The default is transparent; Tape Stop uses
    /// `moduleFill`.
    pub track: Color32,
}

impl ArcSlider {
    /// The angle `x` degrees along the arc from its value-0 end.
    fn angle(&self, x: f32) -> f32 {
        if self.mirrored {
            self.start + self.span - x
        } else {
            self.start + x
        }
    }

    /// The fill's moving end at value `v`, where the handle sits.
    fn value_angle(&self, v: f32) -> f32 {
        self.angle(self.span * v)
    }

    /// The fill at value `v`, from the value-0 end to [`Self::value_angle`].
    fn fill(&self, v: f32) -> (f32, f32) {
        (self.angle(0.0), self.value_angle(v))
    }

    /// The filled part of segment `k` of `n`: a − 2° wide, from the segment's value-0 side.
    fn segment(&self, n: usize, k: usize) -> (f32, f32) {
        let a = self.span / n as f32;
        (
            self.angle(k as f32 * a),
            self.angle((k + 1) as f32 * a - GAP_DEG),
        )
    }

    /// Separator `j` of `n`: 2° wide, ending on the boundary after segment `j`.
    fn separator(&self, n: usize, j: usize) -> (f32, f32) {
        let end = (j + 1) as f32 * self.span / n as f32;
        (self.angle(end - GAP_DEG), self.angle(end))
    }

    /// The centre angle of segment `k` of `n`, where the pointer sits.
    fn segment_centre(&self, n: usize, k: usize) -> f32 {
        self.angle((k as f32 + 0.5) * self.span / n as f32)
    }
}

/// The point at angle `deg` (counter-clockwise from +x, y up) on the circle of radius `r` around `center`.
fn polar(center: Pos2, r: f32, deg: f32) -> Pos2 {
    let (sin, cos) = deg.to_radians().sin_cos();
    center + r * vec2(cos, -sin)
}

/// A filled pie slice of radius `r` between two angles (in either order), or nothing when it is empty.
fn sector(center: Pos2, r: f32, (a, b): (f32, f32), color: Color32) -> Option<Shape> {
    let (lo, hi) = (a.min(b), a.max(b));
    // `!(… > 0)` also skips NaN.
    if !(hi - lo > 0.0 && r > 0.0) {
        return None;
    }
    let n = ((hi - lo) / STEP_DEG).ceil() as usize;
    // From the centre, then clockwise on screen (from `hi` down to `lo`): epaint fans the fill out of the first
    // point, which is right for a slice of any span.
    let arc = (0..=n).map(|i| polar(center, r, hi - (hi - lo) * i as f32 / n as f32));
    let points = std::iter::once(center).chain(arc).collect();
    Some(Shape::convex_polygon(points, color, Stroke::NONE))
}

/// The track-colour sector of radius Ri over the full span, which leaves a ring of the sectors under it, drawn as a
/// whole disc: a sector's anti-aliased edges would lie on the fill's edges and let a hairline of it through. The two
/// look the same when the track is transparent (the default: no ring, a pie) or the card's own colour (Tape Stop's
/// `moduleFill`).
fn punch_out(painter: &Painter, center: Pos2, ri: f32, track: Color32) {
    painter.circle_filled(center, ri, track);
}

/// The touch pulse's swell f for the pulse value `q`: 1 − q while touched (q floored at 0.01), then a damped wobble
/// sin((1 − q)·2π)·(1 − q) after release until q drops below 0.01.
fn swell(touched: bool, q: f32) -> f32 {
    if touched {
        1.0 - q.max(PULSE_FLOOR)
    } else if q >= PULSE_FLOOR {
        (1.0 - q) * ((1.0 - q) * TAU).sin()
    } else {
        0.0
    }
}

/// The radius factor s = 1 + f·10/h of the touch pulse ([`pulse_growth`]'s f), h being the control height in pixels:
/// the one knob law of the knobs, the arc sliders, the rulers' playhead knob and the playlist's held header cell.
pub(crate) fn pulse(ui: &Ui, id: Id, touched: bool, h_px: f32) -> f32 {
    // At least a pixel high: a zero-height rect must not divide by zero.
    1.0 + pulse_growth(ui, id, touched) / h_px.max(1.0)
}

/// The touch pulse of control `id`, `touched` or not, as a growth in device pixels: f·10 ([`swell`]), up to 10 px while
/// held, then a damped wobble; the step grids' current-row header grows by it. q restarts at 1 on touch-down and on
/// release and is multiplied by 0.6 every frame before it is drawn, that frame included (the touch-down frame already
/// shows 1 − 0.6); here 0.6 per sixtieth of a second, so that the pulse lasts as long at any frame rate. Asks for
/// frames while it moves.
pub(crate) fn pulse_growth(ui: &Ui, id: Id, touched: bool) -> f32 {
    let key = id.with("pulse");
    let now = ui.input(|i| i.time);
    let (was, since) = ui
        .data(|d| d.get_temp::<(bool, f64)>(key))
        .unwrap_or((false, f64::NEG_INFINITY));
    let since = if touched == was {
        since
    } else {
        ui.data_mut(|d| d.insert_temp(key, (touched, now)));
        now
    };
    let q = PULSE_DECAY.powf(((now - since) * 60.0) as f32 + 1.0);
    if q >= PULSE_FLOOR {
        ui.ctx().request_repaint();
    }
    swell(touched, q) * PULSE_PX
}

/// An arc slider in `c.rect`: label strip on top (when labelled), the value strip at the bottom (when `value_text`),
/// the arc centred in the body. The value is sized as the knob's (fit-only): 12 pt down to 6 pt until it fits the
/// inner-disc diameter less 4 hairlines. Vertical drags move it (up increases) by the knob's law:
/// −0.24 × sensitivity × Δy, at sensitivity 1 and with Δy in inches at the DPI the app is given, as the knob's (not
/// divided by the UI scale).
pub fn arc_slider(
    ui: &mut Ui,
    cx: &UiCtx,
    c: &Control,
    arc: &ArcSlider,
    value: &mut f32,
    display: impl Fn(f32) -> String,
) -> Response {
    let (response, v) = drag_control(ui, cx, c, value, Law::Knob(1.0));
    let (m, colors) = (cx.m, &cx.theme.colors);
    let painter = ui.painter();
    let (_, body, value_strip) = c.strips(m);
    let center = body.center();
    // The knob's radii: Ro = 0.48 D − th, Ri = 0.48 D − 3 th, the handle 1.5 thMax and 1.5 thMax + 2 t.
    let r = ring(
        center,
        body.width().min(body.height()).max(0.0),
        v,
        c.bipolar,
        m.t(),
    );
    // The label as on every control, then the value (both are drawn before the arc).
    Control {
        value_text: false,
        ..*c
    }
    .strip_texts(painter, m, v, &display);
    if let Some(strip) = value_strip {
        let galley = fit_only(
            painter,
            m,
            &display(v),
            value_width(r.inner, m.t()),
            c.channel,
        );
        painter.galley(strip.center() - galley.size() / 2.0, galley, c.channel);
    }

    let touched = primary_down_on(ui, &response);
    let s = pulse(ui, c.id, touched, c.rect.height() * m.ppp());
    // Hovered, the arc looks as the knob does: the wash and the grown handle.
    let lit = touched || hovered(ui, &response);
    let (ro, ri, rm) = (
        s * r.outer,
        (s * r.inner).max(0.0),
        (r.outer + r.inner) / 2.0,
    );
    let (main, control_fill) = (c.channel, colors.control_fill.to_color32());
    let full = (arc.start, arc.start + arc.span);
    painter.extend(sector(center, ro, full, control_fill));
    match c.steps.filter(|&n| n >= 2) {
        None => {
            painter.extend(sector(center, ro, arc.fill(v), main));
            punch_out(painter, center, ri, arc.track);
            if lit {
                painter.circle_filled(center, ri, main.gamma_multiply(WASH));
            }
            let grow = if lit { FOCUS_GROWTH } else { 1.0 };
            let at = polar(center, s * rm, arc.value_angle(v));
            painter.circle_filled(at, s * grow * r.halo_r, control_fill);
            painter.circle_filled(at, s * grow * r.dot_r, main);
        }
        Some(n) => {
            let k = super::item_at(v, n, c.item);
            painter.extend(sector(center, ro, arc.segment(n, k), main));
            punch_out(painter, center, ri, arc.track);
            for j in 0..n {
                painter.extend(sector(center, ro, arc.separator(n, j), arc.track));
            }
            if lit {
                painter.circle_filled(center, ri, main.gamma_multiply(WASH));
            }
            let (mid, u) = (arc.segment_centre(n, k), m.u(1.0));
            let base = |d: f32| polar(center, s * (rm + POINTER_BASE_U * u), mid + d);
            let pointer = vec![
                polar(center, s * (rm + POINTER_TIP_U * u), mid),
                base(-POINTER_DEG),
                base(POINTER_DEG),
            ];
            painter.add(Shape::convex_polygon(pointer, main, Stroke::NONE));
        }
    }
    response
}

#[cfg(test)]
mod tests {
    use super::*;
    use eframe::egui::pos2;

    fn near(a: f32, b: f32) -> bool {
        (a - b).abs() < 1e-3
    }

    fn near2(a: (f32, f32), b: (f32, f32)) -> bool {
        near(a.0, b.0) && near(a.1, b.1)
    }

    /// Tape Stop's arcs.
    const LEFT: ArcSlider = ArcSlider {
        start: 135.0,
        span: 90.0,
        mirrored: true,
        track: Color32::TRANSPARENT,
    };
    const RIGHT: ArcSlider = ArcSlider {
        start: 315.0,
        span: 90.0,
        mirrored: false,
        ..LEFT
    };

    #[test]
    fn tape_stop_arcs_run_from_the_bottom_up() {
        // Left: 225° (lower left) at 0 up to 135° (upper left) at 1. Right: 315° (lower right) up to 45° (405°).
        assert_eq!(LEFT.value_angle(0.0), 225.0);
        assert_eq!(LEFT.value_angle(0.5), 180.0);
        assert_eq!(LEFT.value_angle(1.0), 135.0);
        assert_eq!(RIGHT.value_angle(0.0), 315.0);
        assert_eq!(RIGHT.value_angle(0.25), 337.5);
        assert_eq!(RIGHT.value_angle(1.0), 405.0);
    }

    #[test]
    fn the_fill_starts_at_the_start_angle_or_at_the_far_end_when_mirrored() {
        assert_eq!(RIGHT.fill(0.25), (315.0, 337.5));
        // Mirrored: from start + span − span·v to start + span.
        assert_eq!(LEFT.fill(0.25), (225.0, 202.5));
        assert_eq!(LEFT.fill(0.0), (225.0, 225.0));
    }

    #[test]
    fn segments_are_counted_from_the_value_0_end_and_tile_with_their_separators() {
        // Ten segments of 9°: the fill is 7°, the separator the last 2°.
        assert!(near2(RIGHT.segment(10, 0), (315.0, 322.0)));
        assert!(near2(RIGHT.separator(10, 0), (322.0, 324.0)));
        assert!(near2(RIGHT.segment(10, 9), (396.0, 403.0)));
        assert!(near2(RIGHT.separator(10, 9), (403.0, 405.0)));
        assert!(near(RIGHT.segment_centre(10, 0), 319.5));
        // Mirrored: the same, counted backwards from start + span.
        assert!(near2(LEFT.segment(10, 0), (225.0, 218.0)));
        assert!(near2(LEFT.separator(10, 0), (218.0, 216.0)));
        assert!(near2(LEFT.segment(10, 9), (144.0, 137.0)));
        assert!(near2(LEFT.separator(10, 9), (137.0, 135.0)));
        assert!(near(LEFT.segment_centre(10, 9), 139.5));
    }

    #[test]
    fn angles_are_counter_clockwise_from_plus_x_with_y_up() {
        let c = pos2(100.0, 100.0);
        let at = |deg| polar(c, 10.0, deg);
        assert!(near(at(0.0).x, 110.0) && near(at(0.0).y, 100.0));
        // 90° is up: screen y decreases.
        assert!(near(at(90.0).x, 100.0) && near(at(90.0).y, 90.0));
        assert!(near(at(180.0).x, 90.0) && near(at(270.0).y, 110.0));
    }

    #[test]
    fn a_sector_is_the_centre_then_its_arc_and_empty_sectors_are_skipped() {
        let c = pos2(0.0, 0.0);
        let Some(Shape::Path(p)) = sector(c, 10.0, (315.0, 405.0), Color32::RED) else {
            panic!("no sector");
        };
        assert!(p.closed && p.fill == Color32::RED);
        assert_eq!(p.points[0], c);
        // The arc points all lie on the circle, and the ends are the two angles.
        assert!(
            p.points[1..]
                .iter()
                .all(|q| near(q.to_vec2().length(), 10.0))
        );
        let ends = [p.points[1], *p.points.last().unwrap()];
        for want in [polar(c, 10.0, 315.0), polar(c, 10.0, 405.0)] {
            assert!(ends.iter().any(|e| e.distance(want) < 1e-3), "{ends:?}");
        }
        // Either order gives the same slice.
        let Some(Shape::Path(q)) = sector(c, 10.0, (405.0, 315.0), Color32::RED) else {
            panic!("no sector");
        };
        assert_eq!(p.points, q.points);
        assert!(sector(c, 10.0, (225.0, 225.0), Color32::RED).is_none());
        assert!(sector(c, 0.0, (0.0, 90.0), Color32::RED).is_none());
    }

    #[test]
    fn the_pulse_swells_and_holds_while_touched_then_wobbles_back() {
        // Touched: f = 1 − q, rising as q decays, held at 0.99 once q is below its floor.
        assert_eq!(swell(true, 1.0), 0.0);
        assert!(near(swell(true, 0.6), 0.4));
        assert!(near(swell(true, 0.001), 0.99));
        // Released: sin((1 − q)·2π)·(1 − q), then rest.
        assert_eq!(swell(false, 1.0), 0.0);
        let p: f32 = 0.4;
        assert!(near(
            swell(false, 0.6),
            (p * std::f32::consts::TAU).sin() * p
        ));
        assert!(swell(false, 0.36) < 0.0);
        assert_eq!(swell(false, 0.009), 0.0);
        assert_eq!(swell(false, 0.0), 0.0);
    }
}
