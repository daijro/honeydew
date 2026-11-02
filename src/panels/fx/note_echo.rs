//! Note Echo (ID 47), UIHeight 11 U.
//!
//! The note-echo display (#6) draws the echo train of one note from the params alone, the same on every page:
//! - [`series`]: the markers k = 0…N, with the delay display's delay.
//! - [`Train`]: the plot area, the marker radius and the time and pitch axes with their grids.
//! - Draw order steps 1–7 in [`train`], and step 8's easing in [`glide`].
//!
//! Steps 3 and 4:
//!
//! 3. Horizontal pitch-grid lines, channel colour at 15 % opacity: pitch 0, the top/bottom edges, and ±k·s for k = 1, 2
//!    only (at most two per side), each only while k·s < M. Step s, with px = half-height/M per semitone:
//!    px < 0.45 U → s = 2^ceil(log2(0.45 U/px)); px > 2.5 U → s = 2^−ceil(log2(px/2.5 U)); otherwise s = 1.
//! 4. Pitch labels at those lines and at the edges (`+M` / `-M`), e.g. `+4`, `+8`, `+12`, `-4`. Format `%+.1f` when
//!    |v − round v| ≥ 0.001, else `%+d`. FontSize 1; colour = the per-RGBA average of `controlFill` and the channel
//!    colour, at 50 % transparency. Each label box is text width × FontSize 1 height with its bottom edge on its line,
//!    starting 0.25 U from the left edge. Pitch 0 has no label. There is no skip rule: labels overlap when an inner
//!    line lies within a label height of ±M.

use std::f32::consts::PI;

use eframe::egui::{Color32, Id, Pos2, Rect, Shape, Stroke, Ui, Vec2, pos2, vec2};

use super::leveller::animate;
use crate::metrics::Metrics;
use crate::panels::build::*;
use crate::params::{clamp01, module_spec, snap_echo_delay};
use crate::ui::UiCtx;

/// Param `i` in [0, 1]: the registry default when the vector is short, and 0 for a NaN (the FLM reader copies PRMS
/// as they are, and a NaN target would never be reached: the glide would repaint for as long as the card shows).
fn param(p: &[f32], i: usize) -> f32 {
    let default = || {
        module_spec(47)
            .and_then(|s| s.params.get(i))
            .map_or(0.0, |s| s.default)
    };
    clamp01(p.get(i).copied().unwrap_or_else(default))
}

/// `Repetitions` (p1): N = 0 … 32, round(32v) with halves away from zero, as the engine counts them.
fn repetitions(p: &[f32]) -> usize {
    (param(p, 1) * 32.0).round() as usize
}

/// The delay in beats: b = 1/32 + (8 − 1/32)·v², with v the snapped `Delay (sync)` (p3) while `Sync` (p4) is On and
/// `Delay` (p2) otherwise.
fn beats(p: &[f32]) -> f32 {
    let v = if param(p, 4) > 0.5 {
        snap_echo_delay(param(p, 3))
    } else {
        param(p, 2)
    };
    (8.0 - 1.0 / 32.0f32).mul_add(v * v, 1.0 / 32.0)
}

/// A −90 … 0 … +100 % knob (`Vel. Offset`, `Vel. Drift`) as a fraction, −0.9 … 1.
fn velocity_fraction(v: f32) -> f32 {
    if v < 0.5 {
        -0.9 * (1.0 - 2.0 * v)
    } else {
        2.0 * v - 1.0
    }
}

/// One marker of the echo train; k = 0 is the played note.
#[derive(Clone, Copy, Debug, PartialEq)]
struct Echo {
    /// Beats after the played note.
    t: f32,
    /// Velocity, 0 … 1.
    vel: f32,
    /// Semitones.
    pitch: f32,
    /// −1 (left) … 1 (right), as drawn (Ping-Pong applied).
    pan: f32,
}

/// The markers drawn, k = 0…N:
/// - t0 = 0, tk = tk−1 + Delay·TimeDrift^k;
/// - v0 = 0.5 + 0.5·Vel. Offset, vk = clamp(vk−1·Vel. Drift, 0, 1); echoes with vk < 1/127 are left out;
/// - pk = Pitch Offset + k·Pitch Drift (semitones);
/// - pan0 = Pan Offset, pank = clamp(pank−1 + Pan Drift, ±1); Ping-Pong (v > 0.5) negates repeats 2, 4, 6, … (the
///   first repeat and the display-only k = 0 keep their sign).
///
/// the drifts `Time Drift` (±30 %) and `Vel. Drift` (−90 … +100 %) are factors 1 + %/100; `Vel. Offset`
/// is the fraction −0.9 … 1; Ping-Pong negates the pan drawn, not the running sum.
fn series(p: &[f32]) -> Vec<Echo> {
    let b = beats(p);
    let time_drift = 1.0 + 60.0f32.mul_add(param(p, 9), -30.0) / 100.0;
    let vel_drift = 1.0 + velocity_fraction(param(p, 7));
    let pitch_offset = 48.0f32.mul_add(param(p, 5), -24.0);
    let pitch_drift = 24.0f32.mul_add(param(p, 8), -12.0);
    let pan_drift = param(p, 13) - 0.5;
    let ping_pong = param(p, 14) > 0.5;
    let (mut t, mut vel, mut pan) = (
        0.0,
        0.5 + 0.5 * velocity_fraction(param(p, 6)),
        2.0f32.mul_add(param(p, 12), -1.0),
    );
    let mut echoes = Vec::new();
    for k in 0..=repetitions(p) {
        if k > 0 {
            t += b * time_drift.powi(k as i32);
            vel = (vel * vel_drift).clamp(0.0, 1.0);
            pan = (pan + pan_drift).clamp(-1.0, 1.0);
        }
        if vel >= 1.0 / 127.0 {
            echoes.push(Echo {
                t,
                vel,
                pitch: (k as f32).mul_add(pitch_drift, pitch_offset),
                pan: if ping_pong && k > 0 && k % 2 == 0 {
                    -pan
                } else {
                    pan
                },
            });
        }
    }
    echoes
}

/// The power of two (of beats or semitones) whose spacing, at `unit` points each, is `lo`…`hi` points: from 1,
/// doubled while too close, then halved while too far apart. For the pitch grid (0.45 U … 2.5 U) this is step 3's
/// closed form exactly, 2^ceil(log2(lo/unit)) below `lo` and 2^−ceil(log2(unit/hi)) above `hi`, without log2's
/// rounding at the powers of two.
fn pow2_step(unit: f32, lo: f32, hi: f32) -> f32 {
    let mut s = 1.0f32;
    // Bounded, for a degenerate rect.
    for _ in 0..64 {
        if s * unit < lo {
            s *= 2.0;
        } else if s * unit > hi {
            s /= 2.0;
        } else {
            break;
        }
    }
    s
}

/// Where the train is drawn in the display rect, in points: the plot area and the time and pitch axes.
#[derive(Clone, Copy, Debug, PartialEq)]
struct Train {
    /// The plot area: the rect inset 0.5 U on every side.
    plot: Rect,
    /// The marker radius: min(0.35 U, plot width / (3·(N + 1))), at least max(0.12 U, 1 px).
    r: f32,
    /// x of the played note (t = 0): the plot's left edge + r.
    x0: f32,
    /// Points per beat: the last echo drawn lands min(2 U, 8 % of the plot width) from the plot's right edge.
    beat: f32,
    /// Points per semitone, up from the centre line (pitch 0).
    semi: f32,
    /// The semitones the half-height covers: M = max(12, max |pk|).
    m: f32,
}

impl Train {
    /// The layout of `echoes` ([`series`] of `p`) in the display `rect`. "width" in the radius rule and
    /// "the span" are the plot width; M and the last echo are taken over the markers drawn; with the played note
    /// alone the time axis spans one delay.
    fn new(rect: Rect, p: &[f32], echoes: &[Echo], m: &Metrics) -> Self {
        let plot = rect.shrink(m.u(0.5));
        let n = repetitions(p) as f32;
        let r = (m.u(0.35).min(plot.width() / (3.0 * (n + 1.0)))).max(m.u(0.12).max(1.0 / m.ppp()));
        let x0 = plot.left() + r;
        let last = plot.right() - m.u(2.0).min(0.08 * plot.width());
        let span = echoes.last().map_or(0.0, |e| e.t);
        let span = if span > 0.0 { span } else { beats(p) };
        let max = echoes.iter().fold(12.0f32, |a, e| a.max(e.pitch.abs()));
        Self {
            plot,
            r,
            x0,
            beat: (last - x0) / span,
            semi: plot.height() / 2.0 / max,
            m: max,
        }
    }

    /// The y of pitch `v` (semitones): higher pitch is drawn higher.
    fn y(&self, v: f32) -> f32 {
        self.plot.center().y - v * self.semi
    }

    /// The target centre of marker `e`.
    fn at(&self, e: &Echo) -> Pos2 {
        pos2(e.t.mul_add(self.beat, self.x0), self.y(e.pitch))
    }

    /// Step 2: the x of the vertical time-grid lines, a power-of-two number of beats 0.6–2.5 U apart, from the
    /// played note to the plot's right edge, at most 64 (the lines start at t = 0).
    fn time_lines(&self, m: &Metrics) -> Vec<f32> {
        let step = pow2_step(self.beat, m.u(0.6), m.u(2.5)) * self.beat;
        (0..64)
            .map(|j| (j as f32).mul_add(step, self.x0))
            .take_while(|&x| x <= self.plot.right())
            .collect()
    }

    /// Step 3: the pitches of the horizontal grid lines, top to bottom: +M, 2s and s while below M, 0, then the same
    /// mirrored down to −M.
    fn pitch_lines(&self, m: &Metrics) -> Vec<f32> {
        let s = pow2_step(self.semi, m.u(0.45), m.u(2.5));
        let inner: Vec<f32> = [2.0 * s, s].into_iter().filter(|&v| v < self.m).collect();
        let mut lines = vec![self.m];
        lines.extend(&inner);
        lines.push(0.0);
        lines.extend(inner.iter().rev().map(|v| -v));
        lines.push(-self.m);
        lines
    }
}

/// Step 4: `%+.1f` when |v − round v| ≥ 0.001, else `%+d` (of round v).
fn pitch_label(v: f32) -> String {
    if (v - v.round()).abs() >= 0.001 {
        format!("{v:+.1}")
    } else {
        format!("{:+}", v.round() as i32)
    }
}

/// Step 7's pan crescent of the disc at `c` of radius `r`: 24 segments from the top of the disc to its bottom,
/// inset by `inset`, bulging towards the pan side by sqrt(|pan|) of the inset radius. Pan 0 is a straight line, full
/// pan a half circle just inside the rim.
fn crescent(c: Pos2, r: f32, pan: f32, inset: f32) -> Vec<Pos2> {
    let ri = (r - inset).max(0.0);
    let bulge = pan.signum() * pan.abs().sqrt() * ri;
    (0..=24)
        .map(|i| {
            let a = PI * i as f32 / 24.0;
            pos2(c.x + bulge * a.sin(), c.y - ri * a.cos())
        })
        .collect()
}

/// Step 8 for one redraw: each marker moves the fraction `k` of the way from `from` to its target `to`; one with no
/// start (a new echo) enters at x = `entry`, on its target's y. A marker that ends within `snap` of its target lands
/// on it (easing never arrives).
fn ease(from: &[Pos2], to: &[Pos2], entry: f32, snap: f32, k: f32) -> Vec<Pos2> {
    (to.iter().enumerate())
        .map(|(i, &t)| {
            let s = from.get(i).copied().unwrap_or(pos2(entry, t.y));
            let next = s + (t - s) * k;
            if (t - next).length() < snap { t } else { next }
        })
        .collect()
}

/// A 60 Hz frame (s): "per redraw" is one of these of input time, as for the other eased displays.
const FRAME: f32 = 1.0 / 60.0;

/// The markers' drawn centres, relative to the display's corner so that scrolling the rack moves them along, kept in
/// egui temp data under the leaf's id (with the input time of its last step, [`animate`]).
#[derive(Clone)]
struct Glide {
    /// The display's size: a change snaps the markers.
    size: Vec2,
    /// Whether the markers were still under way at the last step.
    moving: bool,
    at: Vec<Pos2>,
}

/// Step 8: "markers ease halfway to their targets on every redraw, and new echoes glide in from the right. A resize
/// snaps them." The first draw snaps too. A redraw is a 60 Hz frame of input time, so a marker covers
/// 1 − 0.5^(60·dt) of the way in `dt` s, and a second pass of one frame (no input time) moves nothing; the first
/// step after the markers rested counts as one frame, however long the display sat idle; new echoes enter at the
/// display's right edge; only the centres ease (radius, colour and pan follow at once); a repaint is requested until
/// every marker has landed.
fn glide(ui: &Ui, id: Id, rect: Rect, to: &[Pos2], snap: f32) -> Vec<Pos2> {
    let to: Vec<Pos2> = to.iter().map(|&p| (p - rect.min).to_pos2()).collect();
    let size = rect.size();
    let rest = Glide {
        size,
        moving: false,
        at: to.clone(),
    };
    let g = animate(ui, id, rest.clone(), |g, dt| {
        if g.size != size {
            return rest;
        }
        let dt = if g.moving { dt } else { dt.min(FRAME) };
        let at = ease(
            &g.at,
            &to,
            rect.width(),
            snap,
            1.0 - 0.5f32.powf(dt / FRAME),
        );
        Glide {
            size,
            moving: at != to,
            at,
        }
    });
    if g.moving {
        ui.ctx().request_repaint();
    }
    g.at.iter().map(|&p| rect.min + p.to_vec2()).collect()
}

/// The note-echo display (#6), draw order steps 1–8, inside its rect. It edits nothing.
///
/// the grid lines span the plot; the pitch lines are 1 px; the labels start 0.25 U inside the display's
/// left edge, their bottom on the line's top device pixel; the per-RGBA average rounds to the nearest (egui's
/// `lerp_to_gamma`); the played note's second outline and its vertical line are inset 0.08 U like the crescents; an
/// echo's disc is `controlFill` blended to the channel colour by its velocity.
fn train(
    ui: &mut Ui,
    cx: &UiCtx,
    id: Id,
    rect: Rect,
    module: &mut ModuleInstance,
    _: &mut PanelState,
    ink: Color32,
) -> Vec<usize> {
    let m = cx.m;
    let (fill, text) = (
        cx.theme.colors.control_fill.to_color32(),
        cx.theme.colors.text.to_color32(),
    );
    let px = 1.0 / m.ppp();
    let outline = Stroke::new(px, text);
    let painter = ui.painter().with_clip_rect(rect.intersect(ui.clip_rect()));
    let echoes = series(&module.params);
    let train = Train::new(rect, &module.params, &echoes, m);
    let plot = train.plot;
    // 1.
    painter.rect_filled(rect, 0.0, fill);
    // 2.
    for x in train.time_lines(m) {
        let x = m.floor_px(x);
        let line = Rect::from_x_y_ranges(x..=x + px, plot.y_range());
        painter.rect_filled(line, 0.0, ink.gamma_multiply(0.2));
    }
    // 3.
    let pitches = train.pitch_lines(m);
    for &v in &pitches {
        let y = m.floor_px(train.y(v));
        let line = Rect::from_x_y_ranges(plot.x_range(), y..=y + px);
        painter.rect_filled(line, 0.0, ink.gamma_multiply(0.15));
    }
    // 4. The galley is egui's line box of the text: its bottom on the line.
    let font = m.font_id(FontSize::Pt8);
    let color = fill.lerp_to_gamma(ink, 0.5).gamma_multiply(0.5);
    for &v in pitches.iter().filter(|&&v| v != 0.0) {
        let at = pos2(rect.left() + m.u(0.25), m.floor_px(train.y(v)));
        painter.text(at, Align2::LEFT_BOTTOM, pitch_label(v), font.clone(), color);
    }
    // 8.
    let targets: Vec<Pos2> = echoes.iter().map(|e| train.at(e)).collect();
    let at = glide(ui, id, rect, &targets, 0.5 * px);
    // 5.
    if at.len() >= 2 {
        painter.add(Shape::line(at.clone(), Stroke::new(px, ink)));
    }
    // 6.
    let (r, inset) = (train.r, m.u(0.08));
    if let Some(&c) = at.first() {
        painter.circle_filled(c, r, ink);
        painter.circle_stroke(c, r, outline);
        painter.circle_stroke(c, r - inset, outline);
        let half = vec2(0.0, r - inset);
        painter.line_segment([c - half, c + half], outline);
    }
    // 7.
    for (e, &c) in echoes.iter().zip(&at).skip(1) {
        painter.circle_filled(c, r, fill.lerp_to_gamma(ink, e.vel));
        painter.circle_stroke(c, r, outline);
        painter.add(Shape::line(crescent(c, r, e.pan, inset), outline));
    }
    Vec::new()
}

/// `Sync` (p4) shows `Delay (sync)`, off `Delay`.
const SYNC: Show = Show::Item(4, 1);
const FREE: Show = Show::Item(4, 0);

/// `Stop` as drawn while echoes are pending: a channel fill with the best-contrasting theme colour for its label.
const STOP: ButtonColors = ButtonColors {
    fill_on: Ink::Channel,
    fill_off: Ink::Channel,
    text_on: Ink::Contrast,
    text_off: Some(Ink::Contrast),
};

/// Echoes are pending (flag `pending`, which the card sets): `Stop` is enabled. Absent, both its colours are at 50 %.
const PENDING: Show = Show::Flag(flag::PENDING, 1);

pub fn panel() -> Option<Panel> {
    let delay =
        |id: u32, label, p: usize| knob(id, label).p(p).h(Size::U(4.0)).w(Size::Percent(100.0));
    Some(
        Panel::new(hstack([
            vstack([
                delay(1, "Repetitions", 1),
                delay(2, "Delay", 2).show(FREE),
                delay(3, "Delay (sync)", 3).show(SYNC),
            ])
            .w(Size::Percent(33.0))
            .mt(0.75),
            vstack([
                hstack([
                    tabs(4, "TIME · PITCH · VELOCITY · PAN").h(Size::U(1.25)),
                    hstack([button(5, "Stop", false)
                        .p(15)
                        .momentary()
                        .colors(STOP)
                        .enabled(PENDING)
                        .h(Size::U(1.0))])
                    .w(Size::U(1.5)),
                ])
                .h(Size::U(1.25)),
                display(6).h(Size::U(5.0)).draw(train),
                hstack([
                    toggle(7, "Sync").p(4).value_text(),
                    knob(8, "Delay Rand.").p(11),
                    knob(9, "Time Drift").p(9),
                    knob(10, "Length Drift").p(10),
                ])
                .show(Show::Tab(0)),
                hstack([knob(11, "Pitch Offset").p(5), knob(12, "Pitch Drift").p(8)])
                    .show(Show::Tab(1)),
                hstack([knob(13, "Vel. Offset").p(6), knob(14, "Vel. Drift").p(7)])
                    .show(Show::Tab(2)),
                hstack([
                    knob(15, "Pan Offset").p(12),
                    knob(16, "Pan Drift").p(13),
                    toggle(17, "Ping-Pong").p(14).value_text(),
                ])
                .show(Show::Tab(3)),
            ]),
        ]))
        .hook(sync),
    )
}

/// Writing `Delay (sync)` (#3) while Sync is On snaps p3 to its item's value, and toggling Sync (#7) re-applies the
/// delay param that becomes visible (snapping p3 the same way).
fn sync(module: &mut ModuleInstance, _: &mut PanelState, edited: &[u32]) -> Vec<usize> {
    let toggled = edited.contains(&7);
    let on = module.params.get(4).is_some_and(|&v| v > 0.5);
    match module.params.get_mut(3) {
        Some(p3) if on && (toggled || edited.contains(&3)) => {
            *p3 = snap_echo_delay(*p3);
            vec![3]
        }
        _ if toggled && !on => vec![2],
        _ => Vec::new(),
    }
}

/// A delay knob of the left column: 4 U high, the column's width.
#[cfg(test)]
const fn delay(n: u32, label: &'static str, y: f32, p: usize) -> Row {
    row(n, Widget::Knob, label, [0.0, y, 4.62, 4.0])
        .p(p)
        .w(Size::Percent(100.0))
        .h(Size::U(4.0))
}
/// The rows above the page, then the page's (the TIME page, Sync on).
#[cfg(test)]
const TOP: [Row; 5] = [
    delay(1, "Repetitions", 0.75, 1),
    delay(3, "Delay (sync)", 4.75, 3),
    row(
        4,
        Widget::Tabs,
        "TIME · PITCH · VELOCITY · PAN",
        [4.62, 0.0, 7.88, 1.25],
    )
    .h(Size::U(1.25)),
    // Momentary; a channel fill with the best-contrasting theme colour for its label, enabled while echoes are
    // pending.
    row(
        5,
        Widget::Button { round: false },
        "Stop",
        [12.5, 0.12, 1.5, 1.0],
    )
    .p(15)
    .h(Size::U(1.0))
    .momentary()
    .colors(ButtonColors {
        fill_on: Ink::Channel,
        fill_off: Ink::Channel,
        text_on: Ink::Contrast,
        text_off: Some(Ink::Contrast),
    })
    .enabled(Show::Flag(flag::PENDING, 1)),
    row(6, Widget::Display, "", [4.62, 1.25, 9.38, 5.0]).h(Size::U(5.0)),
];
#[cfg(test)]
const TIME: [Row; 4] = [
    row(7, Widget::Toggle, "Sync", [4.62, 6.25, 2.34, 2.74])
        .p(4)
        .value_text(),
    row(8, Widget::Knob, "Delay Rand.", [6.96, 6.25, 2.35, 2.74]).p(11),
    row(9, Widget::Knob, "Time Drift", [9.31, 6.25, 2.34, 2.74]).p(9),
    row(10, Widget::Knob, "Length Drift", [11.65, 6.25, 2.35, 2.74]).p(10),
];

#[cfg(test)]
pub const REFERENCE: Reference = Reference {
    size: [14.0, 8.99],
    settings: &[],
    cases: &[
        Case {
            name: "TIME, Sync on",
            rows: &[
                TOP[0], TOP[1], TOP[2], TOP[3], TOP[4], TIME[0], TIME[1], TIME[2], TIME[3],
            ],
            ..Case::DEFAULT
        },
        Case {
            name: "TIME, Sync off",
            params: &[(4, 0.0)],
            rows: &[
                TOP[0],
                // "in the same slot"
                delay(2, "Delay", 4.75, 2),
                TOP[2],
                TOP[3],
                TOP[4],
                TIME[0],
                TIME[1],
                TIME[2],
                TIME[3],
            ],
            ..Case::DEFAULT
        },
        Case {
            name: "PITCH",
            tab: 1,
            rows: &[
                TOP[0],
                TOP[1],
                TOP[2],
                TOP[3],
                TOP[4],
                row(11, Widget::Knob, "Pitch Offset", [4.62, 6.25, 4.69, 2.74]).p(5),
                row(12, Widget::Knob, "Pitch Drift", [9.31, 6.25, 4.69, 2.74]).p(8),
            ],
            ..Case::DEFAULT
        },
        Case {
            name: "VELOCITY",
            tab: 2,
            rows: &[
                TOP[0],
                TOP[1],
                TOP[2],
                TOP[3],
                TOP[4],
                row(13, Widget::Knob, "Vel. Offset", [4.62, 6.25, 4.69, 2.74]).p(6),
                row(14, Widget::Knob, "Vel. Drift", [9.31, 6.25, 4.69, 2.74]).p(7),
            ],
            ..Case::DEFAULT
        },
        Case {
            name: "PAN",
            tab: 3,
            rows: &[
                TOP[0],
                TOP[1],
                TOP[2],
                TOP[3],
                TOP[4],
                row(15, Widget::Knob, "Pan Offset", [4.62, 6.25, 3.12, 2.74]).p(12),
                row(16, Widget::Knob, "Pan Drift", [7.74, 6.25, 3.13, 2.74]).p(13),
                row(17, Widget::Toggle, "Ping-Pong", [10.87, 6.25, 3.12, 2.74])
                    .p(14)
                    .value_text()
                    .slack(
                        1.0,
                        "f32 rounding in reference geometry: 10.87 + 3.12 ends 1 px short of 14",
                    ),
            ],
            ..Case::DEFAULT
        },
    ],
};

#[cfg(test)]
mod tests {
    use super::*;

    /// The hook the panel runs, which the tests go through: a panel without it fails them.
    fn hook(m: &mut ModuleInstance, s: &mut PanelState, edited: &[u32]) -> Vec<usize> {
        (panel().unwrap().hook.expect("a hook"))(m, s, edited)
    }

    /// The train draws the engine's count, round(32v) with halves away from zero.
    #[test]
    fn repetitions_round_halves_up_as_the_engine_does() {
        let mut p = ModuleInstance::new(47, 2).params;
        for (v, count) in [(0.5 / 32.0, 1), (2.5 / 32.0, 3), (1.0, 32)] {
            p[1] = v;
            assert_eq!(repetitions(&p), count, "v {v}");
        }
    }

    #[test]
    fn writing_the_synced_delay_snaps_it_while_sync_is_on() {
        let (mut module, mut state) = (ModuleInstance::new(47, 2), PanelState::default());
        // Between "1" (0.3487) and "2" (0.4971), on the side of "1".
        module.params[3] = 0.4;
        assert_eq!(hook(&mut module, &mut state, &[3]), [3]);
        assert_eq!(module.params[3], snap_echo_delay(0.4));
        // Sync Off: no snapping.
        module.params[4] = 0.0;
        module.params[3] = 0.4;
        assert!(hook(&mut module, &mut state, &[3]).is_empty());
        assert_eq!(module.params[3], 0.4);
    }

    #[test]
    fn toggling_sync_reapplies_the_delay_that_becomes_visible() {
        let (mut module, mut state) = (ModuleInstance::new(47, 2), PanelState::default());
        module.params[3] = 0.4;
        // Off: `Delay` (p2) is sent again as it is.
        module.params[4] = 0.0;
        let free = module.params[2];
        assert_eq!(hook(&mut module, &mut state, &[7]), [2]);
        assert_eq!(module.params[2], free);
        // On: `Delay (sync)` is written again, which snaps it.
        module.params[4] = 1.0;
        assert_eq!(hook(&mut module, &mut state, &[7]), [3]);
        assert_eq!(module.params[3], snap_echo_delay(0.4));
        // Other edits leave both alone.
        module.params[3] = 0.4;
        assert!(hook(&mut module, &mut state, &[8]).is_empty());
        assert_eq!(module.params[3], 0.4);
    }

    // ---- the note-echo display (#6) ----

    use eframe::egui::{Context, RawInput, Shape, Stroke, pos2, vec2};

    use crate::metrics::DEFAULT_UI_SCALE;
    use crate::panels::tests::{Rig, U, card, display_shapes};
    use crate::theme::theme_by_id;

    fn defaults() -> Vec<f32> {
        ModuleInstance::new(47, 2).params
    }

    fn with(changes: &[(usize, f32)]) -> Vec<f32> {
        let mut p = defaults();
        for &(i, v) in changes {
            p[i] = v;
        }
        p
    }

    fn near(a: f32, b: f32) -> bool {
        (a - b).abs() < 1e-3
    }

    fn near2(a: Pos2, b: Pos2) -> bool {
        near(a.x, b.x) && near(a.y, b.y)
    }

    fn metrics(ppp: f32) -> Metrics {
        Metrics::from_pixels_per_point(ppp, DEFAULT_UI_SCALE)
    }

    /// #6 at the reference: (4.62, 1.25, 9.38, 5) U.
    fn display_rect() -> Rect {
        Rect::from_min_size(pos2(4.62 * U, 1.25 * U), vec2(9.38 * U, 5.0 * U))
    }

    fn layout(rect: Rect, p: &[f32], ppp: f32) -> (Vec<Echo>, Train) {
        let echoes = series(p);
        let train = Train::new(rect, p, &echoes, &metrics(ppp));
        (echoes, train)
    }

    #[test]
    fn the_default_train_is_the_note_and_8_echoes_half_a_beat_apart_at_pitch_0() {
        let echoes = series(&defaults());
        assert_eq!(echoes.len(), 9);
        for (k, e) in echoes.iter().enumerate() {
            assert!(near(e.t, 0.5 * k as f32), "{k}: {e:?}");
            assert_eq!((e.vel, e.pitch, e.pan), (0.5, 0.0, 0.0), "{k}");
        }
    }

    #[test]
    fn the_delay_is_the_synced_item_while_sync_is_on_else_the_free_delay() {
        // `Delay` at 1 (8 beats), `Delay (sync)` at `1/8`.
        let delays = [(2, 1.0), (3, 0.1085)];
        assert!(near(series(&with(&delays))[1].t, 0.125));
        let free = with(&[delays[0], delays[1], (4, 0.0)]);
        assert!(near(series(&free)[1].t, 8.0));
        // Synced delays snap: `Delay (sync)` at 0.4 is `1`.
        assert!(near(series(&with(&[(3, 0.4)]))[1].t, 1.0));
    }

    #[test]
    fn time_drift_scales_each_step_by_its_power() {
        // +30 %: t1 = 1.3·b, t2 = t1 + 1.3²·b.
        let e = series(&with(&[(9, 1.0)]));
        assert!(near(e[1].t, 0.5 * 1.3));
        assert!(near(e[2].t, 0.5 * 1.3 + 0.5 * 1.69));
        // −30 %.
        assert!(near(series(&with(&[(9, 0.0)]))[1].t, 0.35));
    }

    #[test]
    fn pitch_drift_adds_per_echo_and_ping_pong_mirrors_repeats_2_4_6() {
        // Pitch Drift +2, Pan Offset +0.5, Ping-Pong. The k = 0 marker and repeat 1 keep their sign.
        let e = series(&with(&[(8, 14.0 / 24.0), (12, 0.75), (14, 1.0)]));
        assert_eq!(e.len(), 9);
        for (k, e) in e.iter().enumerate() {
            assert!(near(e.pitch, 2.0 * k as f32), "{k}: {e:?}");
            assert!(
                near(e.pan, if k > 0 && k % 2 == 0 { -0.5 } else { 0.5 }),
                "{k}: {e:?}"
            );
        }
        // Pitch Offset −24 places the played note too. Pan Drift +50 % accumulates and clamps; Ping-Pong mirrors
        // what is drawn, not the sum.
        let e = series(&with(&[(5, 0.0), (13, 1.0), (14, 1.0)]));
        assert_eq!(e[0].pitch, -24.0);
        let pans: Vec<f32> = e.iter().map(|e| e.pan).collect();
        assert_eq!(pans[..6], [0.0, 0.5, -1.0, 1.0, -1.0, 1.0]);
        // Without Ping-Pong.
        let pans: Vec<f32> = series(&with(&[(13, 0.0)])).iter().map(|e| e.pan).collect();
        assert_eq!(pans[..4], [0.0, -0.5, -1.0, -1.0]);
    }

    #[test]
    fn ping_pong_is_on_above_one_half_only() {
        // Pan Offset +1, Pan Drift 0: repeat 1 is +1 and repeat 2 is −1 only with Ping-Pong on (v14 > 0.5).
        let pans = |v| -> Vec<f32> {
            series(&with(&[(12, 1.0), (14, v)]))
                .iter()
                .map(|e| e.pan)
                .collect()
        };
        assert_eq!(pans(0.5)[..3], [1.0, 1.0, 1.0]);
        assert_eq!(pans(0.5001)[..3], [1.0, 1.0, -1.0]);
    }

    #[test]
    fn a_velocity_decay_drops_the_tail() {
        // Vel. Drift −90 %: 0.5, 0.05, then 0.005 < 1/127.
        let e = series(&with(&[(7, 0.0)]));
        assert_eq!(e.len(), 2);
        assert!(near(e[1].vel, 0.05));
        // Vel. Offset and Vel. Drift +100 %: 1, then held at 1.
        let e = series(&with(&[(6, 1.0), (7, 1.0)]));
        assert_eq!(e.len(), 9);
        assert!(e.iter().all(|e| e.vel == 1.0));
        // Vel. Offset −90 %: the played note at 0.05.
        assert!(near(series(&with(&[(6, 0.0)]))[0].vel, 0.05));
        // No repetitions: the played note alone.
        assert_eq!(series(&with(&[(1, 0.0)])).len(), 1);
    }

    #[test]
    fn the_markers_span_the_plot_inset_half_a_unit() {
        for ppp in [1.0, 2.0] {
            let (e, t) = layout(display_rect(), &defaults(), ppp);
            let plot = Rect::from_min_size(pos2(5.12 * U, 1.75 * U), vec2(8.38 * U, 4.0 * U));
            assert!(
                near2(t.plot.min, plot.min) && near2(t.plot.max, plot.max),
                "{:?}",
                t.plot
            );
            // r = min(0.35 U, 8.38 U / (3·9)).
            assert!(near(t.r, 8.38 * U / 27.0));
            // The played note touches the left edge; the last echo leaves min(2 U, 8 % of 8.38 U) free.
            assert!(near(t.at(&e[0]).x, 5.12 * U + t.r));
            assert!(near(t.at(&e[8]).x, 13.5 * U - 0.08 * 8.38 * U));
            assert!(near(t.at(&e[4]).x, (t.at(&e[0]).x + t.at(&e[8]).x) / 2.0));
            // Pitch 0 is the centre line.
            assert!(e.iter().all(|e| near(t.at(e).y, 3.75 * U)));
        }
        // A 40 U display: 8 % of 39 U is more than 2 U.
        let wide = Rect::from_min_size(pos2(0.0, 0.0), vec2(40.0 * U, 5.0 * U));
        let (e, t) = layout(wide, &defaults(), 1.0);
        assert!(near(t.at(&e[8]).x, 37.5 * U));
    }

    #[test]
    fn the_marker_radius_is_capped_by_the_count_and_floored() {
        for ppp in [1.0, 2.0] {
            let r = |n: f32| layout(display_rect(), &with(&[(1, n / 32.0)]), ppp).1.r;
            assert!(near(r(0.0), 0.35 * U));
            assert!(near(r(8.0), 8.38 * U / 27.0));
            // 8.38 U / 99 < 0.12 U.
            assert!(near(r(32.0), 0.12 * U));
        }
    }

    #[test]
    fn the_pitch_axis_covers_at_least_an_octave_each_way() {
        let (_, t) = layout(display_rect(), &defaults(), 1.0);
        assert_eq!(t.m, 12.0);
        // Pitch Drift +2: M = 16, the last echo on the top edge and pitch 8 halfway up.
        let (e, t) = layout(display_rect(), &with(&[(8, 14.0 / 24.0)]), 1.0);
        assert!(near(t.m, 16.0));
        assert!(near(t.at(&e[8]).y, 1.75 * U));
        assert!(near(t.at(&e[4]).y, 2.75 * U));
        // Lower pitch is drawn lower.
        let (e, t) = layout(display_rect(), &with(&[(5, 0.0)]), 1.0);
        assert!(near(t.m, 24.0) && near(t.at(&e[0]).y, 5.75 * U));
    }

    #[test]
    fn the_time_grid_is_a_power_of_two_of_beats_0_6_to_2_5_u_apart() {
        let lines = |p: &[f32]| {
            let (_, t) = layout(display_rect(), p, 1.0);
            (t.time_lines(&metrics(1.0)), t)
        };
        // Defaults: 1 beat is 7.40 U / 4 = 1.85 U, 5 lines from the played note to the plot's right edge.
        let (x, t) = lines(&defaults());
        assert!(near(t.beat, (8.38 - 8.38 / 27.0 - 0.08 * 8.38) * U / 4.0));
        assert_eq!(x.len(), 5);
        for (j, &x) in x.iter().enumerate() {
            assert!(near(x, t.x0 + j as f32 * t.beat), "{j}");
        }
        // `1/8` delay: 8 echoes span 1 beat (7.40 U), so a quarter beat (1.85 U).
        let (x, t) = lines(&with(&[(3, 0.1085)]));
        assert!(near(x[1] - x[0], t.beat / 4.0));
        // 32 echoes of 8 beats (Sync Off): 256 beats, every 32 beats (0.95 U), 9 lines.
        let (x, t) = lines(&with(&[(1, 1.0), (4, 0.0), (2, 1.0)]));
        assert!(near(x[1] - x[0], 32.0 * t.beat));
        assert_eq!(x.len(), 9);
        for (x, t) in [
            lines(&defaults()),
            lines(&with(&[(1, 1.0), (4, 0.0), (2, 1.0)])),
        ] {
            let gap = (x[1] - x[0]) / U;
            assert!((0.6..=2.5).contains(&gap), "{gap}");
            assert!(*x.last().unwrap() <= t.plot.right());
        }
    }

    #[test]
    fn the_pitch_step_doubles_below_0_45_u_and_halves_above_2_5_u_per_semitone() {
        // px < 0.45 U → s = 2^ceil(log2(0.45 U/px)); px > 2.5 U → s = 2^−ceil(log2(px/2.5 U)); otherwise s = 1.
        let s = |px: f32| pow2_step(px * U, 0.45 * U, 2.5 * U);
        assert_eq!(s(0.45), 1.0);
        assert_eq!(s(2.5), 1.0);
        assert_eq!(s(1.0), 1.0);
        assert_eq!(s(0.449), 2.0);
        assert_eq!(s(0.225), 2.0);
        assert_eq!(s(0.224), 4.0);
        // M = 12 over 1 U: 0.45 U / (1/12 U) = 5.4.
        assert_eq!(s(1.0 / 12.0), 8.0);
        assert_eq!(s(2.501), 0.5);
        assert_eq!(s(5.0), 0.5);
        assert_eq!(s(5.001), 0.25);
        // 12 U / 2.5 U = 4.8.
        assert_eq!(s(12.0), 0.125);
    }

    #[test]
    fn the_pitch_grid_is_0_the_edges_and_at_most_two_steps_each_side() {
        let values = |rect: Rect, p: &[f32]| {
            let (_, t) = layout(rect, p, 1.0);
            t.pitch_lines(&metrics(1.0))
        };
        // M = 12 over 2 U: every 4 semitones (0.67 U).
        let lines = [12.0, 8.0, 4.0, 0.0, -4.0, -8.0, -12.0];
        assert_eq!(values(display_rect(), &defaults()), lines);
        // Pitch Drift +2: M = 16 (f32 noise aside), s = 4; 3s = 12 < M is not drawn (k = 1, 2 only).
        let drift = values(display_rect(), &with(&[(8, 14.0 / 24.0)]));
        let lines = [16.0, 8.0, 4.0, 0.0, -4.0, -8.0, -16.0];
        assert!(
            drift.len() == 7 && drift.iter().zip(lines).all(|(&a, b)| near(a, b)),
            "{drift:?}"
        );
        // 32 echoes rising 12 from +24: M = 408, every 128 semitones (0.63 U), but not ±384.
        let high = with(&[(1, 1.0), (5, 1.0), (8, 1.0)]);
        let lines = [408.0, 256.0, 128.0, 0.0, -128.0, -256.0, -408.0];
        assert_eq!(values(display_rect(), &high), lines);
        // A display 3 U high: M = 12 over 1 U, s = 8; 2s = 16 ≥ M.
        let short = Rect::from_min_size(display_rect().min, vec2(9.38 * U, 3.0 * U));
        assert_eq!(values(short, &defaults()), [12.0, 8.0, 0.0, -8.0, -12.0]);
        // A display 1.2 U high: s = 64 ≥ M, so pitch 0 and the edges.
        let low = Rect::from_min_size(display_rect().min, vec2(9.38 * U, 1.2 * U));
        assert_eq!(values(low, &defaults()), [12.0, 0.0, -12.0]);
    }

    #[test]
    fn pitch_labels_are_signed_with_one_decimal_off_the_integers() {
        assert_eq!(pitch_label(4.0), "+4");
        assert_eq!(pitch_label(12.0), "+12");
        assert_eq!(pitch_label(-4.0), "-4");
        assert_eq!(pitch_label(19.2), "+19.2");
        assert_eq!(pitch_label(-0.5), "-0.5");
        // `%+.1f` when |v − round v| ≥ 0.001, else `%+d`.
        assert_eq!(pitch_label(24.04), "+24.0");
        assert_eq!(pitch_label(-24.96), "-25.0");
        assert_eq!(pitch_label(12.0015), "+12.0");
        assert_eq!(pitch_label(12.0009), "+12");
        // f32 noise (Pitch Offset −5, Pitch Drift −2.5, 8 echoes) is still an integer.
        assert_eq!(pitch_label(-25.000002), "-25");
    }

    #[test]
    fn the_pan_crescent_runs_down_the_disc_and_bulges_to_the_pan_side() {
        let (c, r, inset) = (pos2(100.0, 50.0), 10.0, 2.56);
        let ri = r - inset;
        // Pan 0: a straight line from the top to the bottom, inset.
        let line = crescent(c, r, 0.0, inset);
        assert_eq!(line.len(), 25);
        assert!(line.iter().all(|p| p.x == c.x));
        assert!(near(line[0].y, c.y - ri) && near(line[24].y, c.y + ri));
        // Full pan: a half circle just inside the rim, on the pan side.
        for pan in [1.0f32, -1.0] {
            let arc = crescent(c, r, pan, inset);
            assert_eq!(arc.len(), 25);
            assert!(arc.iter().all(|p| near(p.distance(c), ri)), "{pan}");
            assert!(arc.iter().all(|p| (p.x - c.x) * pan > -1e-4), "{pan}");
            assert!(near(arc[12].x, c.x + pan * ri));
        }
        // A quarter pan bulges by sqrt(0.25) = half the radius.
        assert!(near(crescent(c, r, 0.25, inset)[12].x, c.x + 0.5 * ri));
    }

    #[test]
    fn markers_ease_halfway_and_new_ones_enter_from_the_right() {
        let to = [pos2(10.0, 4.0), pos2(20.0, 6.0)];
        assert_eq!(
            ease(&[pos2(0.0, 0.0)], &to, 100.0, 0.5, 0.5),
            [pos2(5.0, 2.0), pos2(60.0, 6.0)]
        );
        // Within the snap distance of the target they land.
        assert_eq!(ease(&[pos2(9.5, 4.0)], &to[..1], 100.0, 0.5, 0.5), [to[0]]);
        // Echoes gone are dropped.
        assert_eq!(ease(&to, &to[..1], 100.0, 0.5, 0.5), [to[0]]);
    }

    /// One pass of [`glide`] to `to` at input time `t` s on a context that keeps its state between passes.
    fn glide_at(ctx: &Context, t: f64, to: &[Pos2]) -> Vec<Pos2> {
        let rect = Rect::from_min_size(pos2(10.0, 10.0), vec2(100.0, 50.0));
        let raw = RawInput {
            time: Some(t),
            ..Default::default()
        };
        let mut out = Vec::new();
        let _ = ctx.run(raw, |ctx| {
            let ui = Ui::new(ctx.clone(), Id::new("t"), Default::default());
            out = glide(&ui, Id::new("train"), rect, to, 0.5);
        });
        out
    }

    #[test]
    fn a_pass_run_twice_in_one_frame_eases_once() {
        let ctx = Context::default();
        let frame = 1.0 / 60.0;
        // The first pass snaps.
        assert_eq!(glide_at(&ctx, 0.0, &[pos2(20.0, 20.0)]), [pos2(20.0, 20.0)]);
        // A 60 Hz frame later it eases halfway, and its second pass (no input time passed) stays there.
        for _ in 0..2 {
            let at = glide_at(&ctx, frame, &[pos2(60.0, 40.0)]);
            assert!(near2(at[0], pos2(40.0, 30.0)), "{at:?}");
        }
        let at = glide_at(&ctx, 2.0 * frame, &[pos2(60.0, 40.0)]);
        assert!(near2(at[0], pos2(50.0, 35.0)), "{at:?}");
    }

    #[test]
    fn the_easing_halves_the_distance_per_60_hz_of_input_time_whatever_the_frame_rate() {
        let to = [pos2(60.0, 40.0)];
        // 120 Hz: two frames are one 60 Hz frame's halving, not two.
        let ctx = Context::default();
        glide_at(&ctx, 0.0, &[pos2(20.0, 20.0)]);
        let first = glide_at(&ctx, 1.0 / 120.0, &to);
        assert!(
            near(first[0].x, 20.0 + 40.0 * (1.0 - 0.5f32.sqrt())),
            "{first:?}"
        );
        let second = glide_at(&ctx, 2.0 / 120.0, &to);
        assert!(near2(second[0], pos2(40.0, 30.0)), "{second:?}");
        // A throttled 10 Hz tab, once under way: six halvings in a frame, which land it.
        let ctx = Context::default();
        glide_at(&ctx, 0.0, &[pos2(20.0, 20.0)]);
        glide_at(&ctx, 1.0 / 60.0, &to);
        assert_eq!(glide_at(&ctx, 1.0 / 60.0 + 0.1, &to), to);
    }

    #[test]
    fn the_first_step_after_a_rest_is_one_frame_however_long_the_rest() {
        let ctx = Context::default();
        glide_at(&ctx, 0.0, &[pos2(20.0, 20.0)]);
        // 30 s on, a new target: it glides a frame's halving, it does not snap; its second pass adds nothing.
        for _ in 0..2 {
            let at = glide_at(&ctx, 30.0, &[pos2(60.0, 40.0)]);
            assert!(near2(at[0], pos2(40.0, 30.0)), "{at:?}");
        }
    }

    #[test]
    fn a_nan_param_reads_0_and_the_markers_come_to_rest() {
        // The FLM reader keeps a PRMS value as it is: NaN would make every target NaN, which never equals itself,
        // so the glide would repaint at full rate for as long as the card shows.
        for p in [2, 5, 8, 9] {
            assert_eq!(param(&with(&[(p, f32::NAN)]), p), 0.0, "p{p}");
            let mut rig = Rig::new(47, panel().unwrap(), card(11.0).1);
            rig.module.params[p] = f32::NAN;
            for _ in 0..90 {
                rig.frame(vec![]);
            }
            assert!(centres(&rig).iter().all(|c| c.is_finite()), "p{p}");
            assert!(!rig.ctx.has_requested_repaint(), "p{p}");
        }
    }

    /// The display's shapes in the last frame: those clipped to its rect.
    fn drawn(rig: &Rig) -> Vec<Shape> {
        let rect = rig.rect_of(6);
        (rig.shapes.iter())
            .filter(|c| c.clip_rect == rect)
            .map(|c| c.shape.clone())
            .collect()
    }

    /// The drawn marker centres: the points of the polyline (step 5).
    fn centres(rig: &Rig) -> Vec<Pos2> {
        (drawn(rig).into_iter())
            .find_map(|s| match s {
                Shape::Path(p) => Some(p.points),
                _ => None,
            })
            .expect("a polyline")
    }

    fn targets(rig: &Rig) -> Vec<Pos2> {
        let (e, t) = layout(rig.rect_of(6), &rig.module.params, 1.0);
        e.iter().map(|e| t.at(e)).collect()
    }

    #[test]
    fn markers_ease_halfway_per_frame_and_snap_on_a_resize() {
        let mut rig = Rig::new(47, panel().unwrap(), card(11.0).1);
        let rect = rig.rect_of(6);
        // The first frame snaps.
        let before = targets(&rig);
        assert!(
            centres(&rig)
                .iter()
                .zip(&before)
                .all(|(&a, &b)| near2(a, b))
        );
        // Pitch Offset +6, Pitch Drift +2 and a 9th echo: every marker moves more than a pixel.
        rig.module.params[5] = 0.625;
        rig.module.params[8] = 14.0 / 24.0;
        rig.module.params[1] = 9.0 / 32.0;
        rig.frame(vec![]);
        let (after, now) = (targets(&rig), centres(&rig));
        assert_eq!(now.len(), 10);
        for k in 0..9 {
            assert!(
                near2(now[k], before[k] + (after[k] - before[k]) / 2.0),
                "{k}: {:?} {:?} {:?}",
                before[k],
                now[k],
                after[k]
            );
        }
        // The new echo glides in from the right edge.
        assert!(near2(
            now[9],
            pos2((rect.right() + after[9].x) / 2.0, after[9].y)
        ));
        // Halfway again on the next frame.
        rig.frame(vec![]);
        let next = centres(&rig);
        for k in 0..10 {
            assert!(near2(next[k], now[k] + (after[k] - now[k]) / 2.0), "{k}");
        }
        // A resize snaps them.
        rig.module.params[5] = 0.5;
        rig.rect = rig.rect.with_max_x(rig.rect.right() - U);
        rig.frame(vec![]);
        assert_ne!(rig.rect_of(6).size(), rect.size());
        let (to, now) = (targets(&rig), centres(&rig));
        assert!(now.iter().zip(&to).all(|(&a, &b)| near2(a, b)));
    }

    /// The reference content rect: 14 U × 8.99 U.
    fn content() -> Rect {
        Rect::from_min_size(pos2(0.0, 0.0), vec2(14.0 * U, 8.99 * U))
    }

    #[test]
    fn the_display_draws_steps_1_to_7_in_order() {
        let colors = theme_by_id(0).colors;
        let (fill, text, ink) = (
            colors.control_fill.to_color32(),
            colors.text.to_color32(),
            Color32::RED,
        );
        for ppp in [1.0, 2.0] {
            let shapes = display_shapes(47, 6, &[], &PanelState::default(), content(), ppp);
            let kinds: String = (shapes.iter())
                .map(|s| match s {
                    Shape::Rect(_) => 'R',
                    Shape::Text(_) => 'T',
                    Shape::Path(_) => 'P',
                    Shape::Circle(_) => 'C',
                    Shape::LineSegment { .. } => 'L',
                    _ => '?',
                })
                .collect();
            // Fill, 5 time lines, 7 pitch lines, 6 labels, the polyline, the played note, 8 echoes.
            let order = format!(
                "R{}{}{}PCCCL{}",
                "R".repeat(5),
                "R".repeat(7),
                "T".repeat(6),
                "CCP".repeat(8)
            );
            assert_eq!(kinds, order, "ppp {ppp}");
            // 1. The fill is the display rect.
            let Shape::Rect(bg) = &shapes[0] else {
                unreachable!()
            };
            assert_eq!(bg.fill, fill);
            let rect = bg.rect;
            // On whole device pixels.
            let spec = display_rect();
            assert!(
                (rect.min - spec.min).length() < 1.0 && (rect.size() - spec.size()).length() < 1.0,
                "{rect:?}"
            );
            let (e, t) = layout(rect, &defaults(), ppp);
            let px = 1.0 / ppp;
            // 2. Time lines: 1 px over the plot, channel at 20 %.
            for (s, x) in shapes[1..6].iter().zip(t.time_lines(&metrics(ppp))) {
                let Shape::Rect(l) = s else { unreachable!() };
                assert_eq!(l.fill, ink.gamma_multiply(0.2));
                assert!(near(l.rect.width(), px) && (l.rect.left() - x).abs() < px);
                assert_eq!(l.rect.y_range(), t.plot.y_range());
            }
            // 3. Pitch lines: 1 px across the plot, channel at 15 %, top to bottom.
            for (s, v) in shapes[6..13].iter().zip(t.pitch_lines(&metrics(ppp))) {
                let Shape::Rect(l) = s else { unreachable!() };
                assert_eq!(l.fill, ink.gamma_multiply(0.15));
                assert!(near(l.rect.height(), px) && (l.rect.top() - t.y(v)).abs() < px);
                assert_eq!(l.rect.x_range(), t.plot.x_range());
            }
            // 4. Labels: FontSize 1 in the per-RGBA average of `controlFill` (49, 55, 59) and the channel (255, 0, 0),
            //    (152, 28, 30), at 50 % (premultiplied); each box text width × FontSize 1 high, its bottom on its line,
            //    from 0.25 U in; none at 0.
            let label = Color32::from_rgba_premultiplied(76, 14, 15, 128);
            let font = metrics(ppp).font_id(FontSize::Pt8);
            let labels: Vec<_> = (shapes[13..19].iter())
                .map(|s| {
                    let Shape::Text(l) = s else { unreachable!() };
                    let section = &l.galley.job.sections[0].format;
                    assert_eq!(section.color, label);
                    assert_eq!(section.font_id, font);
                    let r = Rect::from_min_size(l.pos, l.galley.size());
                    assert!((r.left() - (rect.left() + 0.25 * U)).abs() <= px, "{r:?}");
                    (l.galley.text().to_owned(), r.bottom())
                })
                .collect();
            let names: Vec<_> = labels.iter().map(|(s, _)| s.as_str()).collect();
            assert_eq!(names, ["+12", "+8", "+4", "-4", "-8", "-12"]);
            for ((_, bottom), v) in labels.iter().zip([12.0, 8.0, 4.0, -4.0, -8.0, -12.0]) {
                let line = metrics(ppp).floor_px(t.y(v));
                assert!((bottom - line).abs() < 1e-3, "{v}: {bottom} vs {line}");
            }
            // 5. The polyline through the markers, 1 px, channel.
            let Shape::Path(line) = &shapes[19] else {
                unreachable!()
            };
            assert_eq!(line.points.len(), 9);
            assert!(line.points.iter().zip(&e).all(|(&p, e)| near2(p, t.at(e))));
            assert_eq!(line.stroke.width, px);
            // 6. The played note: a channel disc, two `text` outlines and a vertical `text` line.
            let c0 = t.at(&e[0]);
            let inner = t.r - 0.08 * U;
            let circle = |s: &Shape| {
                let Shape::Circle(c) = s else { unreachable!() };
                assert!(near2(c.center, c0));
                (c.radius, c.fill, c.stroke)
            };
            assert_eq!(circle(&shapes[20]), (t.r, ink, Stroke::NONE));
            assert_eq!(
                circle(&shapes[21]),
                (t.r, Color32::TRANSPARENT, Stroke::new(px, text))
            );
            let (r2, _, s2) = circle(&shapes[22]);
            assert!(near(r2, inner) && s2 == Stroke::new(px, text));
            let Shape::LineSegment { points, stroke } = &shapes[23] else {
                unreachable!()
            };
            assert!(
                near2(points[0], c0 - vec2(0.0, inner)) && near2(points[1], c0 + vec2(0.0, inner))
            );
            assert_eq!(*stroke, Stroke::new(px, text));
            // 7. Each echo: a disc halfway to `controlFill` (velocity 0.5), a `text` outline and a straight crescent.
            for (k, s) in shapes[24..].chunks(3).enumerate() {
                let c = t.at(&e[k + 1]);
                let Shape::Circle(disc) = &s[0] else {
                    unreachable!()
                };
                assert!(near2(disc.center, c) && near(disc.radius, t.r));
                assert_eq!(disc.fill, fill.lerp_to_gamma(ink, 0.5));
                let Shape::Circle(rim) = &s[1] else {
                    unreachable!()
                };
                assert_eq!((rim.radius, rim.stroke), (t.r, Stroke::new(px, text)));
                let Shape::Path(pan) = &s[2] else {
                    unreachable!()
                };
                assert!(
                    pan.points
                        .iter()
                        .zip(crescent(c, t.r, 0.0, 0.08 * U))
                        .all(|(&a, b)| near2(a, b))
                );
            }
        }
    }

    #[test]
    fn ping_pong_crescents_alternate_sides() {
        // Pan Offset +1 with Ping-Pong: echo 1 keeps its sign and bulges right, echo 2 left.
        let p = with(&[(12, 1.0), (14, 1.0)]);
        let shapes = display_shapes(47, 6, &p, &PanelState::default(), content(), 1.0);
        let crescents: Vec<_> = (shapes.iter().skip(20))
            .filter_map(|s| match s {
                Shape::Path(p) => Some(p.points[12].x - p.points[0].x),
                _ => None,
            })
            .collect();
        assert_eq!(crescents.len(), 8);
        assert!(crescents[0] > 0.0 && crescents[1] < 0.0 && crescents[2] > 0.0);
    }
}
