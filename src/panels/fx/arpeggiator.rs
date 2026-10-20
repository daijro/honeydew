//! Arpeggiator (ID 51), UIHeight 10 U.
//!
//! The `Rhythm` caption (#16, #28): a plain text label centred horizontally and vertically in its 0.7 U strip (full
//! column width), font step 2, channel colour, no truncation.
//!
//! The rhythm ring (#18, #30, [`rhythm_display`]): a ring of radius 0.35 × width centred in the layer, step i of n at
//! angle (pos + i)·2π/n − 90° (12 o'clock first, clockwise). The patterns R1 … R21 and their step counts are the params
//! layer's, [`ARP_RHYTHM_PATTERNS`]; the Rotate text rounds them in f32, the snap in f64.
//! - With a rhythm: the ring at α 0.65, line 0.04 U. An active step is a `moduleFill` disc of radius R − 1 px, a
//!   channel disc of radius R at α 0.65 and an α 0.65 outline of 0.024 U, R = 0.24 U·morph; a resting step the
//!   `moduleFill` disc and the outline. A start ring fixed at 12 o'clock on the ring (not rotated): α 0.85, line
//!   0.05 U, radius 0.384 U.
//! - Rhythm Off: the ring at α 0.35; 4 dots at 12, 3, 6 and 9 o'clock: a `moduleFill` disc of 0.24 U, then a channel
//!   disc at α 0.35, then an α 0.35 outline of 0.024 U.
//! - Rotation spring (step units): dt = min(time since last frame, 0.05 s), 0 on the first frame. target = Rotation
//!   mod n; d = target − pos wrapped to (−n/2, n/2]; vel ← (vel + 304·dt·d)·0.7^(60·dt); pos ← (pos + vel·dt) mod n.
//!   pos snaps to target when a pattern first becomes valid ([`Ring`], [`spring`]).
//! - Step-count morph: t = clamp(elapsed/0.3 s, 0, 1), e = t²(3 − 2t). Angles ease from the old spacing to the new
//!   by e. Dots only in the new count grow with scale e; dots only in the old count shrink with scale 1 − e
//!   ([`ease`], [`dots`]).
//!
//! The `Rotation` slider (#19, #31) always takes touch. With a rhythm, its track and thumb rim are `controlFill` and
//! its value bar and thumb centre the channel colour. While its arp's Rhythm is Off, all four parts are `moduleFill`.
//! Every write snaps the value: Rhythm Off → 0.5; otherwise k = round((2v − 1)(n − 1)) and v = (k + n − 1)/(2(n − 1)),
//! n = the Rhythm's step count. A Rhythm change re-snaps the kept bipolar position to the new step count
//! ([`Leaf::lit`](crate::panels::Leaf::lit), [`snap`], the hook [`snap_rotations`]).

use eframe::egui::{Color32, Id, Rect, Stroke, Ui, vec2};
use honeydew_audio::analysis;

use super::leveller::animate;
use super::live_analysis;
use crate::panels::build::*;
use crate::params::{ARP_RHYTHM_PATTERNS, arp_pattern, arp_steps};
use crate::ui::UiCtx;

/// `Sync` (p3) shows the `Rate` knob, off `Rate (ms)`.
const SYNC: Show = Show::Item(3, 1);
const FREE: Show = Show::Item(3, 0);
/// An arp's `Rhythm` (ARP 1 p14, ARP 2 p25) is on a pattern, not `Off`: its `Rotation` is in its colours.
const RHYTHM_1: Show = Show::Not(&Show::Item(14, 0));
const RHYTHM_2: Show = Show::Not(&Show::Item(25, 0));

fn pct(v: f32) -> Size {
    Size::Percent(v)
}

/// A page: 6 U high under a 0.5 U gap.
fn page(k: usize, columns: Vec<Part>) -> Part {
    hstack(columns).h(Size::U(6.0)).mt(0.5).show(Show::Tab(k))
}

/// A column or block `[h 100%; margin t0.25 r0 b0.25 l0]`.
fn inset(part: Part) -> Part {
    part.h(pct(100.0)).margins(0.25, 0.0, 0.25, 0.0)
}

/// A control `[w 100%, h flex 1]`.
fn full(part: Part) -> Part {
    part.w(pct(100.0))
}

/// ARP 1 or ARP 2 (page `k`), controls from `#n`, params of Rate, Start note, Arp, Gate, Rhythm, Rotation, Repeats,
/// Retrig, Range and Octave shift, when `Rotation` is lit, and its rhythm display.
fn arp(k: usize, n: u32, p: [usize; 10], lit: Show, ring: Draw) -> Part {
    let [rate, start, arp, gate, rhythm] = [p[0], p[1], p[2], p[3], p[4]];
    let [rotation, repeats, retrig, range, shift] = [p[5], p[6], p[7], p[8], p[9]];
    page(
        k,
        vec![
            inset(hstack([
                inset(vstack([
                    full(knob(n, "Rate").p(rate)),
                    full(knob(n + 1, "Start note").p(start)),
                ])),
                inset(vstack([
                    full(spinner(n + 2, "Arp").p(arp).items_per_column(8)),
                    full(knob(n + 3, "Gate").p(gate)),
                ])),
            ])),
            inset(vstack([
                // The `Rhythm` caption: centred both ways in its 0.7 U strip, font step 2, channel colour.
                text(n + 4, "Rhythm")
                    .font(FontSize::Pt10)
                    .align(Align2::CENTER_CENTER)
                    .h(Size::U(0.7)),
                layer([
                    // `Rhythm`: the knob's own label is hidden.
                    knob(n + 5, "").p(rhythm).w(pct(100.0)).h(pct(50.0)),
                    // The rhythm ring around it.
                    display(n + 6).w(pct(100.0)).h(pct(100.0)).draw(ring),
                ])
                .w(pct(100.0)),
                // `Rotation`, with no label strip; all `moduleFill` while the arp's Rhythm is Off.
                h_slider(n + 7, "")
                    .p(rotation)
                    .w(pct(100.0))
                    .h(Size::U(0.4))
                    .lit(lit),
            ])),
            inset(hstack([
                inset(vstack([
                    full(knob(n + 8, "Repeats").p(repeats)),
                    full(spinner(n + 9, "Retrig").p(retrig)),
                ])),
                inset(vstack([
                    full(knob(n + 10, "Range").p(range)),
                    full(knob(n + 11, "Octave shift").p(shift)),
                ])),
            ])),
        ],
    )
}

/// Alphas of the channel colour: the ring, active discs and outlines with a rhythm; the start ring; the
/// ring, discs and outlines while Off.
const LIT: f32 = 0.65;
const START: f32 = 0.85;
const OFF: f32 = 0.35;
/// The step-count morph (s).
const MORPH: f64 = 0.3;
/// The spring's longest time step (s).
const MAX_DT: f64 = 0.05;
/// The spring is at rest, and snaps to its target, within this (steps, steps/s), so the repaints stop.
const REST: f32 = 1e-3;

/// The integer shift of normalised `Rotation` on a pattern of `len` steps: k = round((2v − 1)(n − 1)). In f64, as
/// the engine computes it: f32 rounds some values next to a tie (0.05 on 6 steps) the other way.
fn shift(rotation: f32, len: usize) -> i32 {
    ((2.0 * f64::from(rotation) - 1.0) * (len as f64 - 1.0)).round() as i32
}

/// What a write of `Rotation` `v` keeps on a rhythm of `n` steps (`None`: Off): 0.5 while Off, else
/// v = (k + n − 1)/(2(n − 1)) for k = [`shift`].
fn snap(v: f32, n: Option<usize>) -> f32 {
    match n {
        Some(n) if n > 1 => (shift(v, n) + n as i32 - 1) as f32 / (2 * (n - 1)) as f32,
        _ => 0.5,
    }
}

/// The panel's hook: a write of an arp's `Rotation` (#19, #31) or a change of its `Rhythm` (#17, #29) snaps that
/// Rotation ([`snap`]), so the history and the audio get the snapped value. Returns the Rotations it moved.
fn snap_rotations(module: &mut ModuleInstance, _: &mut PanelState, edited: &[u32]) -> Vec<usize> {
    let mut moved = Vec::new();
    for (rhythm, ids) in [(14, [17, 19]), (25, [29, 31])] {
        if !ids.iter().any(|id| edited.contains(id)) {
            continue;
        }
        let n = module.params.get(rhythm).and_then(|&r| arp_steps(r));
        if let Some(v) = module.params.get_mut(rhythm + 1) {
            let snapped = snap(*v, n);
            if snapped != *v {
                *v = snapped;
                moved.push(rhythm + 1);
            }
        }
    }
    moved
}

/// `d` wrapped to (−n/2, n/2]: the shortest way round a ring of `n` steps.
fn wrap(d: f32, n: f32) -> f32 {
    let d = d.rem_euclid(n);
    if d > n / 2.0 { d - n } else { d }
}

/// One step of the rotation spring by `dt` s: the new position and velocity, in steps, on a ring of `n` steps.
fn spring(pos: f32, vel: f32, target: f32, n: f32, dt: f32) -> (f32, f32) {
    let vel = (vel + 304.0 * dt * wrap(target - pos, n)) * 0.7f32.powf(60.0 * dt);
    ((pos + vel * dt).rem_euclid(n), vel)
}

/// The morph's easing of `t` (clamped to [0, 1]): t²(3 − 2t).
fn ease(t: f32) -> f32 {
    let t = t.clamp(0.0, 1.0);
    t * t * (3.0 - 2.0 * t)
}

/// A step's dot: degrees clockwise from 12 o'clock, its morph scale, and whether its step is active.
#[derive(Clone, Copy, Debug, PartialEq)]
struct Dot {
    angle: f32,
    scale: f32,
    on: bool,
}

/// The dots of pattern `to` (`x` on), morphing from `from` by `e`, turned by `pos` steps. A dot only in the
/// old count shows the old pattern's step.
fn dots(to: &[u8], from: &[u8], pos: f32, e: f32) -> Vec<Dot> {
    let (new, old) = (360.0 / to.len() as f32, 360.0 / from.len() as f32);
    let spacing = old + (new - old) * e;
    (0..to.len().max(from.len()))
        .map(|i| {
            let (step, scale) = match to.get(i) {
                Some(&s) if i < from.len() => (s, 1.0),
                Some(&s) => (s, e),
                None => (from[i], 1.0 - e),
            };
            let angle = (pos + i as f32) * spacing;
            Dot {
                angle,
                scale,
                on: step == b'x',
            }
        })
        .collect()
}

/// What the ring animates, in egui temp data under the display's id while a pattern shows: the pattern drawn and the
/// one it morphs from (indices into [`ARP_RHYTHM_PATTERNS`]) since `start`, and the rotation spring (steps, steps/s)
/// as of `last` (input times, s).
#[derive(Clone, Copy, Debug, PartialEq)]
struct Ring {
    to: usize,
    from: usize,
    start: f64,
    pos: f32,
    vel: f32,
    last: f64,
}

impl Ring {
    /// A pattern that just became valid at `now`: at rest on its target, no morph.
    fn new(pattern: usize, target: f32, now: f64) -> Self {
        Self {
            to: pattern,
            from: pattern,
            // Well past the morph: `now - MORPH` reads as 0.29999… s after f64 rounding.
            start: now - 2.0 * MORPH,
            pos: target,
            vel: 0.0,
            last: now,
        }
    }

    /// The frame at `now`: a new `pattern` starts a morph from the drawn one, which ends (`from` = `to`, so a shrink
    /// stops drawing its vanished dots) once e reaches 1; the spring steps towards `target`. Returns whether it is
    /// still moving.
    fn step(&mut self, pattern: usize, target: f32, now: f64) -> bool {
        if pattern != self.to {
            (self.from, self.to, self.start) = (self.to, pattern, now);
        }
        if self.eased(now) >= 1.0 {
            self.from = self.to;
        }
        let n = ARP_RHYTHM_PATTERNS[self.to].len() as f32;
        let dt = (now - self.last).clamp(0.0, MAX_DT) as f32;
        (self.pos, self.vel) = spring(self.pos, self.vel, target, n, dt);
        self.last = now;
        if wrap(target - self.pos, n).abs() < REST && self.vel.abs() < REST {
            (self.pos, self.vel) = (target, 0.0);
        }
        self.pos != target || self.from != self.to
    }

    /// The morph's e at `now`.
    fn eased(&self, now: f64) -> f32 {
        ease(((now - self.start) / MORPH) as f32)
    }
}

/// How long a step's pulse falls from 1 to 0, s.
const PULSE: f32 = 0.12;
/// The playhead's radius (U) and alphas, with a rhythm and while Off.
const HEAD_U: f32 = 0.12;
const HEAD: f32 = 0.35;
const HEAD_OFF: f32 = 0.5;
/// The steps of the Off ring: its 4 dots, a pulse each quarter cycle (one arp step each).
const OFF_STEPS: usize = 4;

/// Each step dot's pulse: 1 when the playhead reaches it, falling linearly to 0 over [`PULSE`]. `last` is the arp step
/// The playhead was on.
#[derive(Clone, Copy, Debug, Default, PartialEq)]
struct Pulses {
    last: Option<i64>,
    values: [f32; 8],
}

impl Pulses {
    /// After `dt` s with the playhead at `step` (steps; `None`: not running); the playhead on a new step `k` fires the
    /// pulse of dot `dot(k mod n)`, if any.
    fn step(
        mut self,
        dt: f32,
        step: Option<f32>,
        n: usize,
        dot: impl Fn(usize) -> Option<usize>,
    ) -> Pulses {
        self.values = self.values.map(|v| (v - dt / PULSE).max(0.0));
        let k = step.map(|s| s.floor() as i64);
        if let Some(k) = k.filter(|&k| Some(k) != self.last)
            && let Some(i) = dot(k.rem_euclid(n as i64) as usize).filter(|&i| i < 8)
        {
            self.values[i] = 1.0;
        }
        self.last = k;
        self
    }
}

/// The channel colour `ink` at `alpha`, with `add` added to its R, G, B and alpha (clamped): a pulsing dot. Without a
/// pulse, exactly `ink` at `alpha` as the resting ring draws it.
fn lit(ink: Color32, alpha: f32, add: f32) -> Color32 {
    if add <= 0.0 {
        return ink.gamma_multiply(alpha);
    }
    let [r, g, b, _] = ink.to_srgba_unmultiplied().map(|c| f32::from(c) / 255.0);
    let byte = |v: f32| ((v + add).clamp(0.0, 1.0) * 255.0).round() as u8;
    Color32::from_rgba_unmultiplied(byte(r), byte(g), byte(b), byte(alpha))
}

/// The rhythm display of the arp whose `Rhythm` is p`RHYTHM` (its `Rotation` is the next param), over its knob; see
/// The module doc. Keyed on input time, so a repeated pass draws the same. With the Arpeggiator's feed, while this arp
/// engine runs, the playhead at 360°·phase from 12 o'clock (phase: the step within the pattern, or within the Off
/// ring's 4) and the step pulses. The playhead is white drawn in `text`, the theme's nearest role.
fn rhythm_display<const RHYTHM: usize>(
    ui: &mut Ui,
    cx: &UiCtx,
    id: Id,
    rect: Rect,
    module: &mut ModuleInstance,
    _: &mut PanelState,
    ink: Color32,
) -> Vec<usize> {
    let param = |p: usize| module.params.get(p).copied().unwrap_or(0.0);
    let (centre, r0, u) = (rect.center(), 0.35 * rect.width(), cx.m.u(1.0));
    let at = |angle: f32, r: f32| {
        let a = angle.to_radians();
        centre + r * vec2(a.sin(), -a.cos())
    };
    let module_fill = cx.theme.colors.module_fill.to_color32();
    let outline = |alpha| Stroke::new(0.024 * u, ink.gamma_multiply(alpha));
    // Arp 1's rhythm is p14, Arp 2's p25.
    let engine = usize::from(RHYTHM != 14);
    let feed = live_analysis(ui, module.instance_id).and_then(|s| analysis::arpeggiator(&s));
    let step = feed
        .and_then(|engines| engines[engine])
        .filter(|s| s.is_finite());
    let pattern = arp_pattern(param(RHYTHM));
    let n = pattern.map_or(OFF_STEPS, |p| ARP_RHYTHM_PATTERNS[p].len());
    let rotation = shift(param(RHYTHM + 1), n);
    let hit = |k: usize| match pattern {
        // The dot on the playhead's angle: the step the engine plays is mask[(k − shift) mod n].
        Some(p) => {
            let i = (k as i32 - rotation).rem_euclid(n as i32) as usize;
            (ARP_RHYTHM_PATTERNS[p].as_bytes()[i] == b'x').then_some(i)
        }
        None => Some(k),
    };
    let pulses = animate(ui, id.with("pulses"), Pulses::default(), |p, dt| {
        p.step(dt, step, n, hit)
    });
    if step.is_some() || pulses.values.iter().any(|&v| v > 0.0) {
        ui.ctx().request_repaint();
    }
    let head = |painter: &eframe::egui::Painter, alpha: f32| {
        if let Some(step) = step {
            let phase = step.rem_euclid(n as f32) / n as f32;
            let text = cx.theme.colors.text.to_color32();
            painter.circle_filled(
                at(360.0 * phase, r0),
                HEAD_U * u,
                text.gamma_multiply(alpha),
            );
        }
    };
    let Some(p) = pattern else {
        ui.data_mut(|d| d.remove::<Ring>(id));
        let painter = ui.painter();
        painter.circle_stroke(centre, r0, Stroke::new(0.04 * u, ink.gamma_multiply(OFF)));
        for (k, angle) in [0.0, 90.0, 180.0, 270.0].into_iter().enumerate() {
            let pulse = pulses.values[k];
            painter.circle_filled(at(angle, r0), 0.24 * u, module_fill);
            painter.circle(
                at(angle, r0),
                0.24 * u * (1.0 + 0.12 * pulse),
                lit(ink, OFF, 0.2 * pulse),
                outline(OFF),
            );
        }
        head(painter, HEAD_OFF);
        return Vec::new();
    };
    let target = rotation.rem_euclid(n as i32) as f32;
    let now = ui.input(|i| i.time);
    let (ring, moving) = ui.data_mut(|d| {
        let ring = d.get_temp_mut_or_insert_with(id, || Ring::new(p, target, now));
        let moving = ring.step(p, target, now);
        (*ring, moving)
    });
    if moving {
        ui.ctx().request_repaint();
    }
    let (painter, px) = (ui.painter(), 1.0 / cx.m.ppp());
    painter.circle_stroke(centre, r0, Stroke::new(0.04 * u, ink.gamma_multiply(LIT)));
    let (to, from) = (
        ARP_RHYTHM_PATTERNS[ring.to].as_bytes(),
        ARP_RHYTHM_PATTERNS[ring.from].as_bytes(),
    );
    for (i, dot) in dots(to, from, ring.pos, ring.eased(now))
        .into_iter()
        .enumerate()
    {
        let pulse = if dot.on {
            pulses.values.get(i).copied().unwrap_or(0.0)
        } else {
            0.0
        };
        let (c, r) = (
            at(dot.angle, r0),
            0.24 * u * dot.scale * (1.0 + 0.2 * pulse),
        );
        painter.circle_filled(c, (r - px).max(0.0), module_fill);
        let fill = if dot.on {
            lit(ink, LIT, 0.35 * pulse)
        } else {
            Color32::TRANSPARENT
        };
        painter.circle(c, r, fill, outline(LIT));
    }
    painter.circle_stroke(
        at(0.0, r0),
        0.384 * u,
        Stroke::new(0.05 * u, ink.gamma_multiply(START)),
    );
    head(painter, HEAD);
    Vec::new()
}

pub fn panel() -> Option<Panel> {
    let cell = |part: Part, h: f32| part.w(pct(100.0)).h(pct(h));
    let panel = Panel::new(vstack([
        tabs(1, "GLOBAL · ARP 1 · ARP 2").h(Size::U(1.25)),
        page(
            0,
            vec![
                inset(
                    vstack([
                        cell(toggle(2, "Arp 1").p(19), 50.0),
                        cell(toggle(3, "Arp 2").p(30), 50.0),
                    ])
                    .w(pct(10.0)),
                ),
                inset(
                    vstack([
                        cell(h_slider(4, "Velocity 1").p(7), 50.0),
                        cell(h_slider(5, "Velocity 2").p(8), 50.0),
                    ])
                    .w(pct(27.5)),
                ),
                inset(
                    vstack([
                        cell(knob(6, "Rate").p(1), 60.0).show(SYNC),
                        cell(knob(7, "Rate (ms)").p(2), 60.0).show(FREE),
                        cell(toggle(8, "Sync").p(3), 40.0),
                    ])
                    .w(pct(25.0)),
                ),
                inset(vstack([cell(v_slider(9, "Swing").p(4), 100.0)]).w(pct(18.75))),
                vstack([
                    full(toggle(10, "Latch").p(6)),
                    full(spinner(11, "Swing rate").p(5)),
                ])
                .w(pct(18.75))
                .h(pct(100.0)),
            ],
        ),
        arp(
            1,
            12,
            [31, 13, 10, 9, 14, 15, 17, 18, 11, 12],
            RHYTHM_1,
            rhythm_display::<14>,
        ),
        arp(
            2,
            24,
            [32, 24, 21, 20, 25, 26, 28, 29, 22, 23],
            RHYTHM_2,
            rhythm_display::<25>,
        ),
    ]));
    Some(panel.hook(snap_rotations))
}

#[cfg(test)]
const TABS: Row = row(
    1,
    Widget::Tabs,
    "GLOBAL · ARP 1 · ARP 2",
    [0.0, 0.0, 14.0, 1.25],
)
.h(Size::U(1.25));
#[cfg(test)]
const FULL: Size = Size::Percent(100.0);

/// A GLOBAL control `[w 100%, h pct%]`.
#[cfg(test)]
const fn cell(row: Row, pct: f32) -> Row {
    row.w(FULL).h(Size::Percent(pct))
}

/// The tab strip and an ARP page: `#n`… and the params of Rate, Start note, Arp, Gate, Rhythm, Rotation, Repeats,
/// Retrig, Range and Octave shift, and when `Rotation` is lit.
#[cfg(test)]
const fn arp_rows(n: u32, p: [usize; 10], active: Show) -> [Row; 13] {
    [
        TABS,
        row(n, Widget::Knob, "Rate", [0.0, 1.75, 2.33, 3.0])
            .p(p[0])
            .w(FULL),
        row(n + 1, Widget::Knob, "Start note", [0.0, 4.75, 2.33, 3.0])
            .p(p[1])
            .w(FULL),
        // 24 items, 3 columns of 8.
        row(n + 2, Widget::Spinner, "Arp", [2.33, 1.75, 2.33, 3.0])
            .p(p[2])
            .w(FULL)
            .items_per_column(8),
        row(n + 3, Widget::Knob, "Gate", [2.33, 4.75, 2.33, 3.0])
            .p(p[3])
            .w(FULL),
        // The `Rhythm` caption: centred both ways in its 0.7 U strip, font step 2, channel colour.
        row(n + 4, Widget::Text, "Rhythm", [4.66, 1.75, 4.67, 0.7])
            .font(FontSize::Pt10)
            .align(Align2::CENTER_CENTER)
            .h(Size::U(0.7)),
        // The knob's own label is hidden.
        row(n + 5, Widget::Knob, "", [4.66, 3.67, 4.67, 2.45])
            .p(p[4])
            .w(FULL)
            .h(Size::Percent(50.0)),
        row(n + 6, Widget::Display, "", [4.66, 2.45, 4.67, 4.9])
            .w(FULL)
            .h(FULL),
        // No label.
        row(n + 7, Widget::HSlider, "", [4.66, 7.35, 4.67, 0.4])
            .p(p[5])
            .w(FULL)
            .h(Size::U(0.4))
            .lit(active),
        row(n + 8, Widget::Knob, "Repeats", [9.33, 1.75, 2.33, 3.0])
            .p(p[6])
            .w(FULL),
        row(n + 9, Widget::Spinner, "Retrig", [9.33, 4.75, 2.33, 3.0])
            .p(p[7])
            .w(FULL),
        row(n + 10, Widget::Knob, "Range", [11.66, 1.75, 2.34, 3.0])
            .p(p[8])
            .w(FULL),
        row(
            n + 11,
            Widget::Knob,
            "Octave shift",
            [11.66, 4.75, 2.34, 3.0],
        )
        .p(p[9])
        .w(FULL),
    ]
}
#[cfg(test)]
const ARP_1: [Row; 13] = arp_rows(12, [31, 13, 10, 9, 14, 15, 17, 18, 11, 12], RHYTHM_1);
#[cfg(test)]
const ARP_2: [Row; 13] = arp_rows(24, [32, 24, 21, 20, 25, 26, 28, 29, 22, 23], RHYTHM_2);
/// GLOBAL before and after the `Rate` knob's slot.
#[cfg(test)]
const LEFT: [Row; 5] = [
    TABS,
    cell(
        row(2, Widget::Toggle, "Arp 1", [0.0, 1.75, 1.4, 3.0]).p(19),
        50.0,
    ),
    cell(
        row(3, Widget::Toggle, "Arp 2", [0.0, 4.75, 1.4, 3.0]).p(30),
        50.0,
    ),
    cell(
        row(4, Widget::HSlider, "Velocity 1", [1.4, 1.75, 3.85, 3.0]).p(7),
        50.0,
    ),
    cell(
        row(5, Widget::HSlider, "Velocity 2", [1.4, 4.75, 3.85, 3.0]).p(8),
        50.0,
    ),
];
#[cfg(test)]
const RIGHT: [Row; 4] = [
    cell(
        row(8, Widget::Toggle, "Sync", [5.25, 5.35, 3.5, 2.4]).p(3),
        40.0,
    ),
    cell(
        row(9, Widget::VSlider, "Swing", [8.75, 1.75, 2.62, 6.0]).p(4),
        100.0,
    ),
    row(10, Widget::Toggle, "Latch", [11.37, 1.75, 2.63, 3.0])
        .p(6)
        .w(FULL),
    row(11, Widget::Spinner, "Swing rate", [11.37, 4.75, 2.63, 3.0])
        .p(5)
        .w(FULL),
];

#[cfg(test)]
pub const REFERENCE: Reference = Reference {
    size: [14.0, 7.99],
    settings: &[],
    cases: &[
        Case {
            name: "GLOBAL, Sync on",
            rows: &[
                LEFT[0],
                LEFT[1],
                LEFT[2],
                LEFT[3],
                LEFT[4],
                cell(
                    row(6, Widget::Knob, "Rate", [5.25, 1.75, 3.5, 3.6]).p(1),
                    60.0,
                ),
                RIGHT[0],
                RIGHT[1],
                RIGHT[2],
                RIGHT[3],
            ],
            ..Case::DEFAULT
        },
        Case {
            name: "GLOBAL, Sync off",
            params: &[(3, 0.0)],
            rows: &[
                LEFT[0],
                LEFT[1],
                LEFT[2],
                LEFT[3],
                LEFT[4],
                // `Rate (ms)` in the `Rate` knob's slot.
                cell(
                    row(7, Widget::Knob, "Rate (ms)", [5.25, 1.75, 3.5, 3.6]).p(2),
                    60.0,
                ),
                RIGHT[0],
                RIGHT[1],
                RIGHT[2],
                RIGHT[3],
            ],
            ..Case::DEFAULT
        },
        Case {
            name: "ARP 1",
            tab: 1,
            rows: &ARP_1,
            ..Case::DEFAULT
        },
        Case {
            name: "ARP 2",
            tab: 2,
            rows: &ARP_2,
            ..Case::DEFAULT
        },
    ],
};

#[cfg(test)]
mod tests {
    use eframe::egui::epaint::{CircleShape, TextShape};
    use eframe::egui::{Pos2, Shape, pos2, vec2};

    use super::*;
    use crate::panels::tests::{Rig, U, display_shapes};
    use crate::params::menu_value;
    use crate::theme::theme_by_id;

    /// The normalised `Rhythm` of pattern Rk (0 is `Off`).
    fn r(k: usize) -> f32 {
        menu_value(k, ARP_RHYTHM_PATTERNS.len() + 1)
    }

    /// The steps of pattern Rk.
    fn bits(k: usize) -> &'static [u8] {
        ARP_RHYTHM_PATTERNS[k - 1].as_bytes()
    }

    /// The content rect at the reference: 14 × 7.99 U.
    fn content() -> Rect {
        Rect::from_min_size(pos2(0.0, 0.0), vec2(14.0 * U, 7.99 * U))
    }

    fn near(a: f32, b: f32) -> bool {
        (a - b).abs() < 1e-4
    }

    /// What display `#leaf` on page `tab` draws in one frame with the defaults changed by `set`, at `ppp`.
    fn circles(tab: usize, leaf: u32, set: &[(usize, f32)], ppp: f32) -> Vec<CircleShape> {
        let mut params = ModuleInstance::new(51, 2).params;
        for &(p, v) in set {
            params[p] = v;
        }
        let state = PanelState {
            tab,
            ..PanelState::default()
        };
        let shapes = display_shapes(51, leaf, &params, &state, content(), ppp);
        let circle = |s: Shape| match s {
            Shape::Circle(c) => c,
            other => panic!("not a circle: {other:?}"),
        };
        shapes.into_iter().map(circle).collect()
    }

    /// The angle of `at` around `centre`, in degrees clockwise from 12 o'clock, in [0, 360).
    fn angle(centre: Pos2, at: Pos2) -> f32 {
        let d = at - centre;
        d.x.atan2(-d.y).to_degrees().rem_euclid(360.0)
    }

    /// Angles `a` and `b` (degrees) are the same direction, to 0.01°.
    fn same_way(a: f32, b: f32) -> bool {
        ((a - b + 180.0).rem_euclid(360.0) - 180.0).abs() < 0.01
    }

    #[test]
    fn a_step_reached_fires_its_dots_pulse_which_falls_over_0_12_s() {
        // R2 "x.x" (3 steps), no rotation: steps 0 and 2 are on.
        let dot = |k: usize| (bits(2)[k] == b'x').then_some(k);
        let p = Pulses::default().step(0.0, Some(0.2), 3, dot);
        assert_eq!(p.values[0], 1.0);
        // Halfway down 0.06 s on; the same step fires nothing new.
        let p = p.step(0.06, Some(0.9), 3, dot);
        assert!(near(p.values[0], 0.5), "{p:?}");
        // Step 1 is off: no pulse. Step 2 fires.
        let p = p
            .step(0.01, Some(1.1), 3, dot)
            .step(0.01, Some(2.0), 3, dot);
        assert_eq!((p.values[1], p.values[2]), (0.0, 1.0));
        assert!(near(p.values[0], 0.5 - 0.02 / 0.12));
        // Stopped: no new pulse, the last ones fall to 0.
        let p = p.step(1.0, None, 3, dot);
        assert_eq!(p.values, [0.0; 8]);
        // A pulse adds to R, G, B and alpha; none is the resting colour itself.
        let ink = Color32::from_rgb(200, 100, 0);
        assert_eq!(lit(ink, LIT, 0.0), ink.gamma_multiply(LIT));
        assert_eq!(
            lit(ink, LIT, 0.35).to_srgba_unmultiplied(),
            [255, 189, 89, 255]
        );
    }

    /// One frame of ARP 1's rhythm display at `ppp` with the defaults changed by `set`, the Arpeggiator's feed `slab`
    /// posted as the rack posts it, at input time `t`, in `ctx`.
    fn fed(
        ctx: &eframe::egui::Context,
        t: f64,
        set: &[(usize, f32)],
        slab: [f32; 4],
        ppp: f32,
    ) -> Vec<Shape> {
        let (theme, m) = (
            theme_by_id(0),
            crate::metrics::Metrics::from_pixels_per_point(ppp, crate::metrics::DEFAULT_UI_SCALE),
        );
        let cx = UiCtx {
            theme: &theme,
            m: &m,
        };
        let feed = (honeydew_audio::MeterSnapshot::default(), slab.to_vec());
        crate::panels::fx::leveller::feed_pass(ctx, t, 48_000.0, Some(feed), |ui| {
            let mut module = ModuleInstance::new(51, 2);
            for &(p, v) in set {
                module.params[p] = v;
            }
            let rect = Rect::from_min_size(pos2(4.66 * U, 2.45 * U), vec2(4.67 * U, 4.9 * U));
            rhythm_display::<14>(
                ui,
                &cx,
                Id::new("ring"),
                rect,
                &mut module,
                &mut PanelState::default(),
                Color32::RED,
            );
        })
    }

    #[test]
    fn a_running_arp_draws_its_playhead_at_the_steps_phase() {
        let text = theme_by_id(0).colors.text.to_color32();
        let rect = Rect::from_min_size(pos2(4.66 * U, 2.45 * U), vec2(4.67 * U, 4.9 * U));
        for ppp in [1.0, 2.0] {
            // Off: the 4-step ring; step 2.5 is 225° round, at α 0.5, radius 0.12 U, last.
            let ctx = eframe::egui::Context::default();
            let shapes = fed(&ctx, 0.0, &[(14, 0.0)], [1.0, 2.5, 0.0, 0.0], ppp);
            let Some(Shape::Circle(head)) = shapes.last() else {
                panic!("{shapes:?}")
            };
            assert!(
                same_way(angle(rect.center(), head.center), 225.0),
                "{head:?}"
            );
            assert!(near(
                (head.center - rect.center()).length(),
                0.35 * rect.width()
            ));
            assert_eq!(
                (head.radius, head.fill),
                (0.12 * U, text.gamma_multiply(0.5))
            );
            // Its dot at 180° (step 2) pulses: bigger and brighter than the resting one at 0°.
            let dots: Vec<&CircleShape> = (shapes.iter())
                .filter_map(|s| match s {
                    Shape::Circle(c) if c.stroke.width > 0.0 && c.center != rect.center() => {
                        Some(c)
                    }
                    _ => None,
                })
                .collect();
            let at = |deg: f32| {
                dots.iter()
                    .find(|c| same_way(angle(rect.center(), c.center), deg))
                    .unwrap()
            };
            assert!(at(180.0).radius > at(0.0).radius, "{dots:?}");
            // R3 "x..x" with a rotation of +1: step 1 plays mask[0], drawn at 90°; α 0.35 now.
            let set = [(14, r(3)), (15, 2.0 / 3.0)];
            let ctx = eframe::egui::Context::default();
            let shapes = fed(&ctx, 0.0, &set, [1.0, 1.25, 0.0, 0.0], ppp);
            let Some(Shape::Circle(head)) = shapes.last() else {
                panic!()
            };
            assert!(
                same_way(angle(rect.center(), head.center), 360.0 * 1.25 / 4.0),
                "{head:?}"
            );
            assert_eq!(head.fill, text.gamma_multiply(0.35));
            // Not running: no playhead.
            let ctx = eframe::egui::Context::default();
            let shapes = fed(&ctx, 0.0, &set, [0.0, 0.0, 0.0, 0.0], ppp);
            assert!(
                !shapes
                    .iter()
                    .any(|s| matches!(s, Shape::Circle(c) if c.fill == text.gamma_multiply(0.35)))
            );
        }
    }

    #[test]
    fn the_rhythm_knob_picks_the_params_layers_patterns() {
        // The table itself is pinned once, in `params::tests`.
        assert_eq!(
            (arp_pattern(0.0), arp_pattern(r(1)), arp_pattern(1.0)),
            (None, Some(0), Some(20))
        );
    }

    #[test]
    fn rotation_is_a_whole_shift_within_plus_minus_len_minus_one() {
        for len in [2, 6, 8] {
            let most = len as i32 - 1;
            assert_eq!(
                (shift(0.0, len), shift(0.5, len), shift(1.0, len)),
                (-most, 0, most)
            );
        }
        // (2v − 1)·5 for R7's 6 steps: 1.0 and −0.6.
        assert_eq!((shift(0.6, 6), shift(0.44, 6)), (1, -1));
    }

    #[test]
    fn the_shift_rounds_in_f64_as_the_engine_does() {
        // (2·0.05 − 1)·5 is the tie −4.5: in f32 it lands on −4.5 and rounds to −5; the engine's f64 gives
        // −4.4999999925… and −4. The ring must light the step the arp plays.
        let engine = ((2.0 * f64::from(0.05f32) - 1.0) * 5.0).round() as i32;
        assert_eq!(engine, -4);
        assert_eq!(shift(0.05, 6), engine);
    }

    #[test]
    fn a_write_snaps_to_a_whole_step_or_to_the_centre_while_off() {
        // Off → 0.5; else k = round((2v − 1)(n − 1)), v = (k + n − 1)/(2(n − 1)).
        assert_eq!([0.0, 0.3, 0.9].map(|v| snap(v, None)), [0.5; 3]);
        // 6 steps: 0.725 → k = round(2.25) = 2 → 7/10; 0.6 is k = 1 already; the ends stay.
        assert_eq!(
            [0.725, 0.6, 0.0, 1.0, 0.5].map(|v| snap(v, Some(6))),
            [0.7, 0.6, 0.0, 1.0, 0.5]
        );
        // A snapped value keeps its step and snaps to itself.
        for n in 2..=8 {
            for i in 0..=1000 {
                let v = i as f32 / 1000.0;
                let s = snap(v, Some(n));
                assert_eq!(
                    (shift(s, n), snap(s, Some(n))),
                    (shift(v, n), s),
                    "{v} on {n} steps"
                );
            }
        }
    }

    #[test]
    fn the_hook_snaps_a_rotation_write_and_re_snaps_on_a_rhythm_change() {
        let mut module = ModuleInstance::new(51, 2);
        let mut state = PanelState::default();
        // R7 (6 steps), Rotation k = 2.
        (module.params[14], module.params[15]) = (r(7), 0.725);
        assert_eq!(
            snap_rotations(&mut module, &mut state, &[]),
            Vec::<usize>::new()
        );
        assert_eq!(module.params[15], 0.725, "no write, no snap");
        assert_eq!(snap_rotations(&mut module, &mut state, &[19]), [15]);
        assert_eq!(module.params[15], 0.7);
        // Already on a step: nothing to report.
        assert_eq!(
            snap_rotations(&mut module, &mut state, &[19]),
            Vec::<usize>::new()
        );
        // R14 has 8 steps: 0.7 is (0.4)(7) = 2.8, k = 3, v = 10/14.
        module.params[14] = r(14);
        assert_eq!(snap_rotations(&mut module, &mut state, &[17]), [15]);
        assert_eq!(module.params[15], 10.0 / 14.0);
        // Off: back to the centre.
        module.params[14] = 0.0;
        assert_eq!(snap_rotations(&mut module, &mut state, &[17]), [15]);
        assert_eq!(module.params[15], 0.5);
        // ARP 2's Rhythm (#29) and Rotation (#31) are p25 and p26; ARP 1's stay.
        (module.params[15], module.params[25], module.params[26]) = (0.9, r(7), 0.725);
        assert_eq!(snap_rotations(&mut module, &mut state, &[29]), [26]);
        assert_eq!((module.params[15], module.params[26]), (0.9, 0.7));
        (module.params[25], module.params[26]) = (0.0, 0.9);
        assert_eq!(snap_rotations(&mut module, &mut state, &[31]), [26]);
        assert_eq!(module.params[26], 0.5);
    }

    #[test]
    fn the_spring_steps_by_hand_values() {
        // From rest at 0 towards 1 on 4 steps at 60 fps: d = 1, vel = (0 + 304/60)·0.7 = 3.546667,
        // pos = vel/60 = 0.059111; then d = 0.940889, vel = (3.546667 + 304/60·0.940889)·0.7 = 5.819686,
        // pos = 0.059111 + 5.819686/60 = 0.156106.
        let (pos, vel) = spring(0.0, 0.0, 1.0, 4.0, 1.0 / 60.0);
        assert!(near(pos, 0.059111) && near(vel, 3.546667), "{pos} {vel}");
        let (pos, vel) = spring(pos, vel, 1.0, 4.0, 1.0 / 60.0);
        assert!(near(pos, 0.156106) && near(vel, 5.819686), "{pos} {vel}");
    }

    #[test]
    fn the_spring_clamps_dt_to_0_05_s_and_its_first_frame_is_0() {
        let r3 = arp_pattern(r(3)).unwrap();
        // A pattern that just became valid sits on its target, and its first frame (dt 0) moves nothing.
        let mut ring = Ring::new(r3, 3.0, 10.0);
        assert_eq!((ring.pos, ring.vel), (3.0, 0.0));
        assert!(!ring.step(r3, 3.0, 10.0));
        assert_eq!((ring.pos, ring.vel), (3.0, 0.0));
        // A second later: dt is 0.05 s, so vel = (0 + 304·0.05·1)·0.7³ = 5.2136 and pos = 0.26068 towards 1.
        let mut ring = Ring::new(r3, 0.0, 10.0);
        assert!(ring.step(r3, 1.0, 11.0));
        assert!(
            near(ring.pos, 0.26068) && near(ring.vel, 5.2136),
            "{ring:?}"
        );
        // A repeated pass at the same time steps by 0.
        let before = ring;
        ring.step(r3, 1.0, 11.0);
        assert_eq!(ring, before);
    }

    #[test]
    fn the_spring_takes_the_shortest_way_round() {
        // 0 → 3 on 4 steps is one step back: d = −1, and pos wraps below 0 to 4 − 0.059111.
        let (pos, vel) = spring(0.0, 0.0, 3.0, 4.0, 1.0 / 60.0);
        assert!(near(pos, 3.940889) && near(vel, -3.546667), "{pos} {vel}");
        // Half way round is (−n/2, n/2]: forwards.
        let (pos, vel) = spring(0.0, 0.0, 2.0, 4.0, 1.0 / 60.0);
        assert!(near(pos, 0.118222) && near(vel, 7.093333), "{pos} {vel}");
        assert_eq!(
            [
                wrap(-2.0, 4.0),
                wrap(2.0, 4.0),
                wrap(3.0, 4.0),
                wrap(-5.0, 4.0)
            ],
            [2.0, 2.0, -1.0, -1.0]
        );
    }

    #[test]
    fn the_spring_comes_to_rest_on_its_target() {
        let r7 = arp_pattern(r(7)).unwrap();
        let mut ring = Ring::new(r7, 0.0, 0.0);
        let frames = (1..=120)
            .take_while(|&f| ring.step(r7, 5.0, f64::from(f) / 60.0))
            .count();
        assert!(frames < 120, "still moving: {ring:?}");
        assert_eq!((ring.pos, ring.vel), (5.0, 0.0));
    }

    #[test]
    fn a_new_pattern_morphs_from_the_one_drawn_with_smoothstep() {
        assert_eq!(
            [-1.0, 0.0, 0.25, 0.5, 0.75, 1.0, 2.0].map(ease),
            [0.0, 0.0, 0.15625, 0.5, 0.84375, 1.0, 1.0]
        );
        let [r1, r3] = [1, 3].map(|k| arp_pattern(r(k)).unwrap());
        let mut ring = Ring::new(r1, 0.0, 5.0);
        assert_eq!(ring.eased(5.0), 1.0, "no morph at first");
        assert!(ring.step(r3, 0.0, 6.0));
        assert_eq!((ring.from, ring.to, ring.start), (r1, r3, 6.0));
        assert_eq!(
            [6.0, 6.075, 6.15, 6.3].map(|t| ring.eased(t)),
            [0.0, 0.15625, 0.5, 1.0]
        );
        assert!(!ring.step(r3, 0.0, 6.31));
        // A shrink ends with only the new pattern's dots: none at scale 0 left behind.
        assert!(ring.step(r1, 0.0, 7.0));
        assert_eq!((ring.from, ring.to), (r3, r1));
        assert!(!ring.step(r1, 0.0, 7.3), "6.99999… + 0.3 s: e is 1");
        assert_eq!(ring.from, r1);
        assert_eq!(dots(bits(1), bits(1), ring.pos, ring.eased(7.3)).len(), 2);
        // A fresh ring is past any morph at any time, f64 rounding included.
        for now in [0.1, 0.7, 6.3, 1e4 + 0.1] {
            assert_eq!(Ring::new(r1, 0.0, now).eased(now), 1.0, "{now}");
        }
    }

    #[test]
    fn new_dots_grow_and_old_dots_shrink_while_the_spacing_eases() {
        let look = |d: &[Dot]| {
            d.iter()
                .map(|d| (d.angle, d.scale, d.on))
                .collect::<Vec<_>>()
        };
        // R1 (x.) → R3 (x..x): spacing 180° → 90°; dots 2 and 3 are new and grow with e.
        let grow = [
            (0.0, 1.0, true),
            (180.0, 1.0, false),
            (360.0, 0.0, false),
            (540.0, 0.0, true),
        ];
        assert_eq!(look(&dots(bits(3), bits(1), 0.0, 0.0)), grow);
        let half = [
            (0.0, 1.0, true),
            (135.0, 1.0, false),
            (270.0, 0.5, false),
            (405.0, 0.5, true),
        ];
        assert_eq!(look(&dots(bits(3), bits(1), 0.0, 0.5)), half);
        let done = [
            (0.0, 1.0, true),
            (90.0, 1.0, false),
            (180.0, 1.0, false),
            (270.0, 1.0, true),
        ];
        assert_eq!(look(&dots(bits(3), bits(1), 0.0, 1.0)), done);
        // R3 → R1: dots 2 and 3 are only in the old count, shrink with 1 − e and show the old steps.
        let shrink = [
            (0.0, 1.0, true),
            (135.0, 1.0, false),
            (270.0, 0.5, false),
            (405.0, 0.5, true),
        ];
        assert_eq!(look(&dots(bits(1), bits(3), 0.0, 0.5)), shrink);
        assert_eq!(
            look(&dots(bits(1), bits(3), 0.0, 1.0))[2..],
            [(360.0, 0.0, false), (540.0, 0.0, true)]
        );
    }

    #[test]
    fn pos_turns_every_dot_clockwise() {
        // Step i sits at (pos + i)·360/n: R7 (x..xx.) turned by one step puts step 0 at 60°.
        let turned = dots(bits(7), bits(7), 1.0, 1.0);
        let angles = turned.iter().map(|d| d.angle).collect::<Vec<_>>();
        assert_eq!(angles, [60.0, 120.0, 180.0, 240.0, 300.0, 360.0]);
        let on = turned.iter().map(|d| d.on).collect::<Vec<_>>();
        assert_eq!(on, [true, false, false, true, true, false]);
    }

    #[test]
    fn the_ring_with_a_rhythm_has_the_spec_shapes_and_alphas() {
        // R7 (x..xx.) turned one step (Rotation 0.6, k = 1): the ring α 0.65 line 0.04 U; per step a
        // `moduleFill` disc of 0.24 U − 1 px, then the 0.24 U disc (channel α 0.65 when active, none at rest) with an
        // α 0.65 outline of 0.024 U; the start ring at 12 o'clock, unturned: α 0.85, line 0.05 U, radius 0.384 U.
        let (ink, colors) = (Color32::RED, theme_by_id(0).colors);
        let module_fill = colors.module_fill.to_color32();
        for ppp in [1.0, 2.0] {
            let c = circles(1, 18, &[(14, r(7)), (15, 0.6)], ppp);
            assert_eq!(c.len(), 1 + 2 * 6 + 1, "at ppp {ppp}");
            let (ring, px) = (c[0], 1.0 / ppp);
            let centre = pos2((4.66 + 4.67 / 2.0) * U, (2.45 + 4.9 / 2.0) * U);
            assert!(
                (ring.center - centre).length() <= px,
                "{ring:?} at ppp {ppp}"
            );
            assert!(
                (ring.radius - 0.35 * 4.67 * U).abs() <= 0.5 * px,
                "{ring:?} at ppp {ppp}"
            );
            assert_eq!(ring.fill, Color32::TRANSPARENT);
            assert_eq!(
                (ring.stroke.width, ring.stroke.color),
                (0.04 * U, ink.gamma_multiply(0.65))
            );
            for k in 0..6 {
                let (disc, dot) = (c[1 + 2 * k], c[2 + 2 * k]);
                assert_eq!(disc.center, dot.center);
                assert!(
                    same_way(angle(ring.center, dot.center), 60.0 * (k + 1) as f32),
                    "step {k}"
                );
                assert!(((dot.center - ring.center).length() - ring.radius).abs() < 0.01);
                assert_eq!(
                    (disc.radius, disc.fill, disc.stroke.width),
                    (0.24 * U - px, module_fill, 0.0)
                );
                let fill = if [0, 3, 4].contains(&k) {
                    ink.gamma_multiply(0.65)
                } else {
                    Color32::TRANSPARENT
                };
                let look = (dot.radius, dot.fill, dot.stroke.width, dot.stroke.color);
                assert_eq!(
                    look,
                    (0.24 * U, fill, 0.024 * U, ink.gamma_multiply(0.65)),
                    "step {k} at ppp {ppp}"
                );
            }
            let start = c[13];
            assert!((start.center - (ring.center - vec2(0.0, ring.radius))).length() < 0.01);
            assert_eq!(
                (start.radius, start.fill),
                (0.384 * U, Color32::TRANSPARENT)
            );
            assert_eq!(
                (start.stroke.width, start.stroke.color),
                (0.05 * U, ink.gamma_multiply(0.85))
            );
        }
    }

    #[test]
    fn off_draws_the_dim_ring_and_four_dots_and_no_start_ring() {
        // Rhythm Off: ring α 0.35; at 12, 3, 6 and 9 o'clock a `moduleFill` disc of 0.24 U, then a channel
        // disc at α 0.35 with an α 0.35 outline of 0.024 U, whatever the Rotation.
        let (ink, module_fill) = (Color32::RED, theme_by_id(0).colors.module_fill.to_color32());
        let dim = ink.gamma_multiply(0.35);
        for ppp in [1.0, 2.0] {
            let c = circles(1, 18, &[(14, 0.0), (15, 0.9)], ppp);
            assert_eq!(c.len(), 1 + 2 * 4);
            assert_eq!(
                (c[0].fill, c[0].stroke.width, c[0].stroke.color),
                (Color32::TRANSPARENT, 0.04 * U, dim)
            );
            for k in 0..4 {
                let (disc, dot) = (c[1 + 2 * k], c[2 + 2 * k]);
                assert!(
                    (angle(c[0].center, dot.center) - 90.0 * k as f32).abs() < 0.01,
                    "{dot:?}"
                );
                assert_eq!(
                    (disc.center, disc.radius, disc.fill),
                    (dot.center, 0.24 * U, module_fill)
                );
                let look = (dot.radius, dot.fill, dot.stroke.width, dot.stroke.color);
                assert_eq!(look, (0.24 * U, dim, 0.024 * U, dim));
            }
        }
    }

    #[test]
    fn arp_2_s_ring_follows_its_own_rhythm_and_rotation() {
        // #30 reads p25 and p26, not ARP 1's p14 and p15. R2 (x.x) at v = 1 turns by +2: steps 0 and 2 (active) at
        // 240° and 120°, step 1 at 0°.
        let c = circles(2, 30, &[(25, r(2)), (26, 1.0), (14, r(7))], 1.0);
        let lit = Color32::RED.gamma_multiply(0.65);
        let mut on: Vec<_> = (c.iter().filter(|d| d.fill == lit))
            .map(|d| angle(c[0].center, d.center).round())
            .collect();
        on.sort_by(f32::total_cmp);
        assert_eq!(on, [120.0, 240.0]);
    }

    /// The outlines of the step dots in the last frame (stroke 0.024 U): their angles (0.1°) and radii.
    fn rig_dots(rig: &Rig) -> Vec<(f32, f32)> {
        let centre = rig.rect_of(18).center();
        let dots = rig.shapes.iter().filter_map(|c| match &c.shape {
            Shape::Circle(d) if d.stroke.width == 0.024 * U => Some(d),
            _ => None,
        });
        dots.map(|d| ((angle(centre, d.center) * 10.0).round() / 10.0, d.radius))
            .collect()
    }

    #[test]
    fn the_ring_draws_over_the_rhythm_knob() {
        // The Layer draws the knob (#17) first, then the display (#18) over it.
        let mut rig = Rig::new(51, panel().unwrap(), content());
        rig.state.tab = 1;
        rig.module.params[14] = r(7);
        rig.frame(vec![]);
        let ring_r = 0.35 * rig.rect_of(18).width();
        let ring = rig
            .shapes
            .iter()
            .position(|c| matches!(&c.shape, Shape::Circle(c) if c.radius == ring_r));
        let ring = ring.expect("the ring");
        // The display's 14 shapes (ring, 6 steps of 2, start ring) from there; the knob's all before them.
        let in_knob = |c: &&eframe::egui::epaint::ClippedShape| {
            rig.rect_of(17)
                .contains_rect(c.shape.visual_bounding_rect())
        };
        assert!(
            rig.shapes[..ring].iter().any(|c| in_knob(&c)),
            "the knob before the ring"
        );
        assert!(
            !rig.shapes[ring + 14..].iter().any(|c| in_knob(&c)),
            "nothing of the knob after the ring"
        );
    }

    #[test]
    fn a_rotation_change_springs_the_dots_round_and_repaints_until_at_rest() {
        let mut rig = Rig::new(51, panel().unwrap(), content());
        rig.state.tab = 1;
        rig.module.params[14] = r(7);
        rig.frame(vec![]);
        assert_eq!(rig_dots(&rig)[0], (0.0, 0.24 * U));
        assert!(!rig.ctx.has_requested_repaint());
        // k = 1: one frame (1/60 s) moves pos 0.059111 steps, 3.5° of R7's 60°.
        rig.module.params[15] = 0.6;
        rig.frame(vec![]);
        assert_eq!(rig_dots(&rig)[0].0, 3.5);
        assert!(rig.ctx.has_requested_repaint());
        for _ in 0..120 {
            rig.frame(vec![]);
        }
        assert_eq!(rig_dots(&rig)[0].0, 60.0);
        assert!(!rig.ctx.has_requested_repaint());
    }

    #[test]
    fn a_new_step_count_morphs_the_dots_over_0_3_s_and_keeps_repainting_until_then() {
        let mut rig = Rig::new(51, panel().unwrap(), content());
        rig.state.tab = 1;
        rig.module.params[14] = r(1);
        rig.frame(vec![]);
        let d = 0.24 * U;
        let looks = |rig: &Rig, want: &[(f32, f32)]| {
            let have = rig_dots(rig);
            let close = |(a, s): (f32, f32), (wa, ws): (f32, f32)| {
                (a - wa).abs() <= 0.1 && (s - ws).abs() < 1e-3
            };
            let ok = have.len() == want.len() && have.iter().zip(want).all(|(&h, &w)| close(h, w));
            assert!(ok, "{have:?}, want {want:?}");
        };
        looks(&rig, &[(0.0, d), (180.0, d)]);
        // R1 → R3 (x..x): the two new dots start at nothing, R1's spacing.
        rig.module.params[14] = r(3);
        rig.frame(vec![]);
        looks(&rig, &[(0.0, d), (180.0, d), (0.0, 0.0), (180.0, 0.0)]);
        assert!(rig.ctx.has_requested_repaint());
        // 0.15 s: e = 0.5, spacing 135°, the new dots at half size.
        for _ in 0..9 {
            rig.frame(vec![]);
        }
        looks(
            &rig,
            &[(0.0, d), (135.0, d), (270.0, d / 2.0), (45.0, d / 2.0)],
        );
        for _ in 0..9 {
            rig.frame(vec![]);
        }
        looks(&rig, &[(0.0, d), (90.0, d), (180.0, d), (270.0, d)]);
        rig.frame(vec![]);
        assert!(!rig.ctx.has_requested_repaint());
    }

    #[test]
    fn a_rhythm_knob_drag_reports_the_rhythm_and_the_re_snapped_rotation_in_one_pass() {
        // R7 (6 steps) with Rotation k = 2 (0.7). Dragging `Rhythm` (#17) up 200 points adds a quarter of the range: R7 + 5.25
        // items, R12 (7 steps), where 0.7 re-snaps to k = round(0.4·6) = 2, v = 8/12. One pass reports both: the knob's
        // p14, then the hook's p15 (a second pass would repeat them).
        let mut rig = Rig::new(51, panel().unwrap(), content());
        rig.state.tab = 1;
        (rig.module.params[14], rig.module.params[15]) = (r(7), 0.7);
        rig.frame(vec![]);
        let at = rig.rect_of(17).center();
        rig.frame(vec![eframe::egui::Event::PointerMoved(at)]);
        rig.press(at, eframe::egui::PointerButton::Primary, true);
        rig.frame(vec![eframe::egui::Event::PointerMoved(
            at - vec2(0.0, 200.0),
        )]);
        assert_eq!(rig.changed, [14, 15]);
        assert_eq!(
            (rig.module.params[14], rig.module.params[15]),
            (r(12), 8.0 / 12.0)
        );
        rig.press(
            at - vec2(0.0, 200.0),
            eframe::egui::PointerButton::Primary,
            false,
        );
    }

    #[test]
    fn a_double_tap_resets_an_arpeggiator_knob_to_0() {
        // The Arpeggiator's knobs reset to 0, not the registry default (Gate 0.4444).
        let mut rig = Rig::new(51, panel().unwrap(), content());
        rig.state.tab = 1;
        rig.module.params[9] = 0.7;
        rig.frame(vec![]);
        let gate = rig.rect_of(15).center();
        rig.click(gate);
        assert_eq!(rig.module.params[9], 0.7);
        rig.click(gate);
        assert_eq!(rig.module.params[9], 0.0);
    }

    #[test]
    fn the_rhythm_captions_are_centred_in_font_step_2_in_the_channel_colour() {
        // The `Rhythm` caption (#16 / #28): centred both ways in its 0.7 U strip, full column width, font step 2
        // (10 pt: 20 points at U = 32), the channel colour.
        let mut rig = Rig::new(51, panel().unwrap(), content());
        for (tab, id, column) in [(1, 16, 17), (2, 28, 29)] {
            rig.state.tab = tab;
            rig.frame(vec![]);
            let (strip, knob) = (rig.rect_of(id), rig.rect_of(column));
            assert!((strip.height() - 0.7 * U).abs() <= 1.0, "#{id}: {strip:?}");
            assert_eq!(strip.x_range(), knob.x_range(), "#{id}: the column's width");
            let text = rig.shapes.iter().find_map(|c| match &c.shape {
                Shape::Text(t) if t.galley.text() == "Rhythm" => Some(t.clone()),
                _ => None,
            });
            let TextShape { pos, galley, .. } = text.expect("the caption");
            let drawn = galley.rect.translate(pos.to_vec2());
            assert!(
                (drawn.center() - strip.center()).length() <= 1.0,
                "#{id}: {drawn:?} in {strip:?}"
            );
            let format = &galley.job.sections[0].format;
            assert_eq!(
                (format.font_id.size, format.color),
                (20.0, Color32::RED),
                "#{id}"
            );
        }
    }
}
