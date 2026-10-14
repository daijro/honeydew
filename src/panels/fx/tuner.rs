//! The Tuner panel (ID 24), UIHeight 7 U. Its controls are module settings, not params.

use std::f32::consts::{FRAC_PI_2, PI};

use eframe::egui::{Color32, Id, Pos2, Rect, Shape, Stroke, StrokeKind, Ui, Vec2, pos2, vec2};

use honeydew_audio::analysis;

use super::leveller::animate;
use super::live_analysis;
use crate::panels::build::*;
use crate::panels::setting_item;
use crate::ui::{UiCtx, widgets};

/// The channel kind the card sets: 0 (absent) without a rack, the construction state; 1 on an audio-input channel;
/// 2 on any other channel.
const AUDIO: Show = Show::Flag(flag::CHANNEL_KIND, 1);
const OTHER: Show = Show::Flag(flag::CHANNEL_KIND, 2);

/// Input button: on = channel disc with a `moduleFill` icon, off = `moduleFill` disc with a channel icon.
const INPUT_COLORS: ButtonColors = ButtonColors {
    fill_on: Ink::Channel,
    fill_off: Ink::ModuleFill,
    text_on: Ink::ModuleFill,
    text_off: Some(Ink::Channel),
};

pub fn panel() -> Option<Panel> {
    Some(
        Panel::new(hstack([
            vstack([
                spinner(1, "Mode").items(&["Steady", "Reactive"]),
                spinner(2, "Visual").items(&["Basic", "Strobe", "Orbit"]),
            ]),
            display(3).w(Size::Flex(6.0)).draw(tuner),
            vstack([
                knob(4, "Reference")
                    .default(0.5)
                    .value_fn(reference)
                    .show(AUDIO),
                v_slider(5, "Reference")
                    .value_text()
                    .default(0.5)
                    .value_fn(reference)
                    .show(Show::Not(&AUDIO)),
                vstack([
                    button(6, "", true)
                        .icon(IconId::AudioRecord)
                        .shape(0.7)
                        .halo(4.0)
                        .colors(INPUT_COLORS)
                        .default(1.0),
                    text(7, "Input")
                        .align(Align2::CENTER_CENTER)
                        .h(Size::U(0.35)),
                ])
                .show(Show::Not(&OTHER)),
            ]),
        ]))
        .hook(follow),
    )
}

/// `Reference`: A4 = 400 + 80·v Hz.
fn reference(v: f32) -> String {
    format!("{:.0} Hz", 400.0 + 80.0 * v)
}

/// `Reference` (#4, #5) is one setting with two looks, and the Input button (#6) is off on a channel that is not an
/// audio input.
fn follow(_: &mut ModuleInstance, state: &mut PanelState, edited: &[u32]) -> Vec<usize> {
    for (from, to) in [(4, 5), (5, 4)] {
        if edited.contains(&from)
            && let Some(&v) = state.settings.get(&from)
        {
            state.settings.insert(to, v);
        }
    }
    if state.flags.get(flag::CHANNEL_KIND) == Some(&2) {
        state.settings.insert(6, 0.0);
    }
    Vec::new()
}

/// `Visual` (#2) items.
const BASIC: usize = 0;
const STROBE: usize = 1;
const ORBIT: usize = 2;

/// RGB factor of colours A and B after 4 s without pitch.
const DIM: f32 = 0.75;
/// The Orbit tilt's targets: while a pitch has been shown and silence is under 4 s, and otherwise.
const TILT_SHOWN: f32 = -1.4923;
const TILT: f32 = -FRAC_PI_2;
/// The strings after 12 s without pitch; the first ones too.
const HZ: &str = "--- Hz";
const CENTS: &str = "--- cent";
const STATUS: &str = "Waiting for input...";
const UNSTABLE: &str = "Unstable pitch...";
/// Silence, s, after which the colours dim and dev decays, and after which the strings reset.
const DIM_AFTER: f32 = 4.0;
const RESET_AFTER: f32 = 12.0;
/// Input, s, without a stable reading before the status says so.
const UNSTABLE_AFTER: f32 = 1.0;
/// Readings this close together count as input (the detector reads every ~11 ms).
const INPUT_GAP: f32 = 0.25;
/// The pop's least spacing, s.
const POP_EVERY: f32 = 3.0;
/// A frame, s, for the per-frame laws (as the Compressor's).
const FRAME: f32 = 1.0 / 60.0;
/// Short note names, C = 0.
const NAMES: [&str; 12] = [
    "C", "C#", "D", "D#", "E", "F", "F#", "G", "G#", "A", "A#", "B",
];

/// `Mode` (#1): the averaging window and the stability tolerance (semitones) of Steady and Reactive.
const MODES: [(usize, f32); 2] = [(4, 0.1), (3, 1.0)];

/// The pitch state, kept between frames and stepped by the detector's readings (the Tuner's feed).
#[derive(Clone, Debug, PartialEq)]
struct Pitch {
    /// The detector's update count last read.
    updates: Option<u32>,
    /// The `Mode` item, and the latest readings (MIDI semitones on the reference) for its window.
    mode: usize,
    window: Vec<f32>,
    /// The displayed pitch, once a stable reading came.
    disp: Option<f32>,
    /// Seconds since the last reading with a pitch (a fresh Tuner's has expired), and of input without a stable one.
    silence: f32,
    unstable: f32,
    /// dev, semitones: the displayed pitch's distance from its note, decaying after 4 s of silence.
    dev: f32,
    in_tune: bool,
    /// Seconds since the last pop started (a fresh Tuner's at 0: no pop within the first 3 s), and into a running one.
    since_pop: f32,
    pop: Option<f32>,
    /// The strobe phase and the Orbit tilt.
    phase: f32,
    tilt: f32,
}

impl Default for Pitch {
    fn default() -> Self {
        Self {
            updates: None,
            mode: 0,
            window: Vec::new(),
            disp: None,
            silence: f32::INFINITY,
            unstable: 0.0,
            dev: 0.0,
            in_tune: false,
            since_pop: 0.0,
            pop: None,
            phase: 0.0,
            tilt: TILT,
        }
    }
}

impl Pitch {
    /// After `dt` s, with the feed's reading (update count, Hz; 0 Hz is no pitch), `mode` and the A4 `reference` Hz.
    /// A reading is new when the count moved. A stable window (full, within the mode's tolerance) moves the display:
    /// it jumps by whole semitones towards the target (the window's mean in Steady, the reading in Reactive), then
    /// glides, disp = target + (disp − target)·0.8.
    fn step(mut self, dt: f32, reading: Option<(u32, f32)>, mode: usize, reference: f32) -> Pitch {
        let (size, tolerance) = MODES[mode.min(1)];
        if mode != self.mode {
            (self.mode, self.window) = (mode, Vec::new());
        }
        self.silence += dt;
        self.unstable = if self.silence <= INPUT_GAP {
            self.unstable + dt
        } else {
            0.0
        };
        if let Some((updates, hz)) = reading
            && self.updates != Some(updates)
        {
            self.updates = Some(updates);
            let note = 12.0 * (hz / reference).log2() + 69.0;
            if hz > 0.0 && note.is_finite() {
                self.silence = 0.0;
                self.window.push(note);
                if self.window.len() > size {
                    self.window.remove(0);
                }
                let (lo, hi) = (self.window.iter())
                    .fold((f32::MAX, f32::MIN), |(lo, hi), &p| (lo.min(p), hi.max(p)));
                if self.window.len() == size && hi - lo <= tolerance {
                    let target = match mode {
                        0 => self.window.iter().sum::<f32>() / size as f32,
                        _ => note,
                    };
                    let disp = self.disp.unwrap_or(target);
                    let jumped = disp + (target - disp).round();
                    self.disp = Some(target + (jumped - target) * 0.8);
                    self.unstable = 0.0;
                }
            } else {
                self.window.clear();
            }
        }
        let frames = dt / FRAME;
        let live = self.silence <= DIM_AFTER && self.disp.is_some();
        self.dev = match self.disp {
            Some(disp) if live => disp - disp.round(),
            _ => settle(self.dev * 0.9f32.powf(frames), 0.0),
        };
        let in_tune = live && self.dev.abs() < 0.05;
        self.since_pop += dt;
        self.pop = self
            .pop
            .map(|t| t + dt)
            .filter(|&t| t / FRAME + 1e-3 < 12.0);
        if in_tune && !self.in_tune && self.since_pop >= POP_EVERY {
            (self.pop, self.since_pop) = (Some(0.0), 0.0);
        }
        self.in_tune = in_tune;
        let cents = (100.0 * self.dev).round();
        self.phase = (self.phase + cents * 0.0005 * frames).rem_euclid(1.0);
        let target = if live { TILT_SHOWN } else { TILT };
        self.tilt = settle(
            target + (self.tilt - target) * 0.99f32.powf(10.0 * frames),
            target,
        );
        self
    }

    /// The pop animation's value: 0.1 … 1 over its first 10 frames, then 0.5 and 0.
    fn pop(&self) -> f32 {
        // Whole frames, forgiving the f32 sum of the steps.
        match self.pop.map(|t| (t / FRAME + 1e-3) as usize) {
            Some(k @ 0..=9) => 0.1 * (k + 1) as f32,
            Some(10) => 0.5,
            _ => 0.0,
        }
    }

    /// Still moving: shown, popping, or easing back to rest.
    fn moving(&self) -> bool {
        self.silence <= RESET_AFTER || self.pop.is_some() || self.dev != 0.0 || self.tilt != TILT
    }

    /// What the display shows on the A4 `reference` Hz.
    fn view(&self, reference: f32) -> View {
        let shown = self.disp.filter(|_| self.silence <= RESET_AFTER);
        let status = if self.silence > RESET_AFTER {
            STATUS
        } else if self.unstable > UNSTABLE_AFTER {
            UNSTABLE
        } else {
            ""
        };
        View {
            dim: if self.silence > DIM_AFTER { DIM } else { 1.0 },
            dev: self.dev,
            in_tune: self.in_tune,
            pop: self.pop(),
            phase: self.phase,
            tilt: self.tilt,
            note: self.disp.map(|disp| disp.round() as i32),
            hz: shown.map_or(HZ.to_owned(), |p| {
                format!("{:.1} Hz", reference * 2f32.powf(p / 12.0 - 5.75))
            }),
            cents: shown.map_or(CENTS.to_owned(), |_| {
                format!("{:+} cent", (100.0 * self.dev).trunc() as i32)
            }),
            status,
        }
    }
}

/// `v` on `target` once within 1e-4 of it, so an easing comes to rest.
fn settle(v: f32, target: f32) -> f32 {
    if (v - target).abs() < 1e-4 { target } else { v }
}

/// What the display draws: the dimming factor, dev, in tune, the pop, the strobe phase, the Orbit tilt, the
/// note shown (its name stays after the strings reset; none before the first), and the strings.
#[derive(Clone, Debug, PartialEq)]
struct View {
    dim: f32,
    dev: f32,
    in_tune: bool,
    pop: f32,
    phase: f32,
    tilt: f32,
    note: Option<i32>,
    hz: String,
    cents: String,
    status: &'static str,
}

impl View {
    /// A fresh Tuner's, which no pitch has reached: it is in the full-silence state.
    #[cfg(test)]
    fn rest() -> View {
        Pitch::default().view(440.0)
    }

    /// Note `note`'s short name and octave, int(note/12): [`tuner`] takes the app octave offset off the note first.
    fn name(note: i32) -> String {
        format!(
            "{}{}",
            NAMES[note.rem_euclid(12) as usize],
            note.div_euclid(12)
        )
    }

    /// The note name, empty before any note.
    fn note_name(&self) -> String {
        self.note.map_or(String::new(), View::name)
    }

    /// The neighbour note, a semitone towards dev, and its own cents (`%+d` of trunc).
    fn neighbour(&self) -> Option<(String, String)> {
        let note = self.note?;
        let step = if self.dev < 0.0 { -1 } else { 1 };
        let cents = (100.0 * (self.dev - step as f32)).trunc() as i32;
        Some((View::name(note + step), format!("{cents:+}")))
    }

    /// Colour B before dimming: lerp(`text`, channel, 1 − 2|dev|).
    fn b(&self, text: Color32, channel: Color32) -> Color32 {
        text.lerp_to_gamma(channel, (1.0 - 2.0 * self.dev.abs()).clamp(0.0, 1.0))
    }
}

/// The rects the Tuner derives from its display rect d, in points.
#[derive(Debug)]
struct Rects {
    /// The Basic meter bar and the strobe bar.
    a: Rect,
    b: Rect,
    /// The Hz, cents and status text.
    c: Rect,
    d: Rect,
    e: Rect,
    /// The triangles above A, left of B and right of B.
    f: Rect,
    g: Rect,
    h: Rect,
}

/// The Tuner's rects for display rect `d`, with `u` points per U.
fn rects(d: Rect, u: f32) -> Rects {
    let bar = |w: f32, h: f32| {
        Rect::from_min_size(
            pos2(d.center().x - w / 2.0, d.bottom() - 0.7 * u),
            vec2(w, h),
        )
    };
    let (a, b) = (
        bar(0.85 * d.width(), 0.2 * u),
        bar(0.74 * d.width(), 0.1 * u),
    );
    let text = |x: f32, w: f32| Rect::from_min_size(pos2(x, d.top() + 0.1 * u), vec2(w, u));
    let mark = |x: f32, y: f32| Rect::from_min_size(pos2(x, y), Vec2::splat(0.4 * u));
    Rects {
        a,
        b,
        c: text(d.left() + 0.82 * u, 2.0 * u),
        d: text(d.right() - 2.82 * u, 2.0 * u),
        e: text(d.left() + 0.25 * d.width(), 0.5 * d.width()),
        f: mark(a.center().x - 0.2 * u, a.top() - 0.7 * u),
        g: mark(b.left() - 0.8 * u, b.center().y - 0.2 * u),
        h: mark(b.right() + 0.4 * u, b.center().y - 0.2 * u),
    }
}

/// The 4 strobe bands in B: B.w/7 wide and B high, starting 1.75 band widths apart, shifted by phase·B.w and wrapped
/// inside B, so a band over its right edge goes on from its left one (as two rects).
fn bands(b: Rect, phase: f32) -> Vec<Rect> {
    let band = |x0: f32, x1: f32| Rect::from_x_y_ranges(x0..=x1, b.y_range());
    let w = b.width() / 7.0;
    let mut rects = Vec::new();
    for k in 0..4 {
        let x = b.left() + (0.25 * k as f32 + phase).rem_euclid(1.0) * b.width();
        rects.push(band(x, (x + w).min(b.right())));
        if x + w > b.right() {
            rects.push(band(b.left(), b.left() + x + w - b.right()));
        }
    }
    rects
}

/// The 11 grid lines on one axis: at i/10 of `len` from `lo`.
fn tenths(lo: f32, len: f32) -> [f32; 11] {
    std::array::from_fn(|i| lo + len * i as f32 / 10.0)
}

/// `c` with RGB × `k`, alpha unchanged.
fn shade(c: Color32, k: f32) -> Color32 {
    let [r, g, b, a] = c.to_srgba_unmultiplied();
    let s = |v: u8| (f32::from(v) * k).round() as u8;
    Color32::from_rgba_unmultiplied(s(r), s(g), s(b), a)
}

/// The triangles: Basic's in F pointing down at A, raised 0.2 U·pop; Strobe's in G pointing right and in H pointing
/// left, at B, each 0.3 U·pop further out. Orbit has none. `u` points per U.
fn marks(r: &Rects, visual: usize, pop: f32, u: f32) -> Vec<[Pos2; 3]> {
    let (up, out) = (vec2(0.0, -0.2 * u * pop), vec2(0.3 * u * pop, 0.0));
    match visual {
        BASIC => vec![[r.f.left_top(), r.f.right_top(), r.f.center_bottom()].map(|p| p + up)],
        STROBE => vec![
            [r.g.left_top(), r.g.left_bottom(), r.g.right_center()].map(|p| p - out),
            [r.h.right_top(), r.h.right_bottom(), r.h.left_center()].map(|p| p + out),
        ],
        _ => Vec::new(),
    }
}

/// The readouts, FontSize 2 in colour A: Hz left-aligned and vertically centred in C, the status centred in E, and
/// except in Orbit the cents right- and top-aligned in D.
fn readouts<'a>(r: &Rects, visual: usize, view: &'a View) -> Vec<(Rect, Align2, &'a str)> {
    let mut texts = vec![
        (r.c, Align2::LEFT_CENTER, view.hz.as_str()),
        (r.e, Align2::CENTER_CENTER, view.status),
    ];
    if visual != ORBIT {
        texts.push((r.d, Align2::RIGHT_TOP, view.cents.as_str()));
    }
    texts
}

/// Basic's deviation fill in A: A's height, round(|cents|) % of its width, from its centre, right when sharp.
fn deviation(a: Rect, dev: f32) -> Option<Rect> {
    let w = (100.0 * dev).abs().round() / 100.0 * a.width();
    let x = a.center().x;
    let span = if dev > 0.0 { x..=x + w } else { x - w..=x };
    (w > 0.0).then(|| Rect::from_x_y_ranges(span, a.y_range()))
}

/// An Orbit dot: its index i, its centre, depth Z, outer and inner disc radii (points), the RGB factor of its colour,
/// and f = 1 − (Z − R)/Dz.
#[derive(Debug)]
struct Dot {
    i: usize,
    at: Pos2,
    z: f32,
    outer: f32,
    inner: f32,
    shade: f32,
    f: f32,
}

/// The orbit: the ring of 10 dots around `centre` at `tilt` and `dev`, far to near; `u` points per U.
fn orbit(centre: Pos2, u: f32, ppp: f32, tilt: f32, dev: f32) -> Vec<Dot> {
    // R = 5 U, Dz = 6 U + 360 px; the radii are in px.
    let (r, dz) = (5.0 * u, 6.0 * u + 360.0 / ppp);
    let rot = -PI * dev.signum() * 2.0 * dev * dev;
    let mut dots: Vec<Dot> = (0..10)
        .map(|i| {
            let theta = FRAC_PI_2 + rot + (i as f32 * 36.0).to_radians();
            let z = dz + r * theta.sin() * tilt.sin();
            let f = 1.0 - (z - r) / dz;
            let (size, rim) = if i % 5 == 0 {
                (120.0, 3.0 + 4.0 * f.powi(3))
            } else {
                (12.0, 2.0 + f.powi(3))
            };
            let outer = size * r / z / ppp;
            Dot {
                i,
                at: centre + vec2(r * theta.cos(), r * theta.sin() * tilt.cos() + 0.3 * u),
                z,
                outer,
                inner: (outer - rim / ppp).max(0.0),
                shade: 1.0 - 0.25 * (z - r) / dz,
                f,
            }
        })
        .collect();
    dots.sort_by(|a, b| b.z.total_cmp(&a.z));
    dots
}

/// Display #3 (fill, grid and visual), from the Tuner's pitch feed (its detector's readings), kept under `id`.
/// Colour A is the channel, colour B lerp(`text`, channel, 1 − 2|dev|); both dim after 4 s without pitch. Without a
/// feed (no stream) a fresh Tuner rests: dimmed, the reset strings, no note, never in tune.
fn tuner(
    ui: &mut Ui,
    cx: &UiCtx,
    id: Id,
    d: Rect,
    module: &mut ModuleInstance,
    state: &mut PanelState,
    channel: Color32,
) -> Vec<usize> {
    let reading = live_analysis(ui, module.instance_id).and_then(|s| analysis::tuner(&s));
    let setting = |n: u32, default: f32| state.settings.get(&n).copied().unwrap_or(default);
    let (mode, reference) = (
        setting_item(setting(1, 0.0)),
        400.0 + 80.0 * setting(5, 0.5),
    );
    let pitch = animate(ui, id, Pitch::default(), |p, dt| {
        p.step(dt, reading, mode, reference)
    });
    if pitch.moving() {
        ui.ctx().request_repaint();
    }
    let mut view = pitch.view(reference);
    // The app octave offset (`Set middle C to C4`) names each note as the one an octave down, as the keyboard does.
    let offset = state.flags.get(flag::OCTAVE_OFFSET).copied().unwrap_or(0) as i32;
    view.note = view.note.map(|n| n - 12 * offset);
    let (painter, m, colors) = (ui.painter(), cx.m, &cx.theme.colors);
    let (u, px) = (m.u(1.0), 1.0 / m.ppp());
    let control = colors.control_fill.to_color32();
    painter.rect_filled(d, 0.0, control);
    let grid = Stroke::new(px, colors.background.to_color32());
    for x in tenths(d.left(), d.width()) {
        painter.vline(x, d.y_range(), grid);
    }
    for y in tenths(d.top(), d.height()) {
        painter.hline(d.x_range(), y, grid);
    }
    let a = shade(channel, view.dim);
    let b_raw = view.b(colors.text.to_color32(), channel);
    let b = shade(b_raw, view.dim);
    let visual = setting_item(state.settings.get(&2).copied().unwrap_or(0.0));
    let r = rects(d, u);
    if visual == BASIC {
        let frame = Stroke::new(2.0 * px, a);
        painter.rect_stroke(r.a.expand(5.0 * px), 0.0, frame, StrokeKind::Middle);
        if let Some(fill) = deviation(r.a, view.dev) {
            painter.rect_filled(fill, 0.0, b);
        }
    }
    if visual == STROBE {
        for band in bands(r.b, view.phase) {
            painter.rect_filled(band, 0.0, b);
        }
    }
    // Outlined always, filled when in tune.
    let fill = if view.in_tune {
        a
    } else {
        Color32::TRANSPARENT
    };
    for t in marks(&r, visual, view.pop, u) {
        painter.add(Shape::convex_polygon(
            t.to_vec(),
            fill,
            Stroke::new(1.5 * px, a),
        ));
    }
    let name = view.note_name();
    if visual != ORBIT && !name.is_empty() {
        // Basic shifts it up 0.1 U, Strobe down.
        let shift = if visual == BASIC { -0.1 * u } else { 0.1 * u };
        let at = d.translate(vec2(0.0, shift));
        widgets::text_label(ui, cx, at, &name, FontSize::Pt64, Align2::CENTER_CENTER, b);
    }
    if visual == ORBIT {
        // The panel's centre is d's: its 1 : 6 : 1 columns centre d.
        let neighbour = view.neighbour();
        for dot in orbit(d.center(), u, m.ppp(), view.tilt, view.dev) {
            let outer = shade(b_raw, view.dim * dot.shade);
            let painter = ui.painter();
            painter.circle_filled(dot.at, dot.outer, outer);
            // Dot 0 in tune is solid.
            let solid = dot.i == 0 && view.in_tune;
            painter.circle_filled(dot.at, dot.inner, if solid { outer } else { control });
            let label = match dot.i {
                0 => Some((
                    name.clone(),
                    format!("{:+}", (100.0 * view.dev).trunc() as i32),
                )),
                5 => neighbour.clone(),
                _ => None,
            };
            let Some((note, cents)) = label.filter(|(n, _)| !n.is_empty()) else {
                continue;
            };
            let ink = match (view.in_tune, dot.i) {
                (true, 0) => control,
                (true, _) => continue,
                _ => outer.gamma_multiply(dot.f.clamp(0.0, 1.0)),
            };
            let at = Rect::from_center_size(dot.at, vec2(2.0 * u, u));
            let center = Align2::CENTER_CENTER;
            widgets::text_label(ui, cx, at, &note, FontSize::Pt18, center, ink);
            let below = at.translate(vec2(0.0, 0.5 * u));
            widgets::text_label(ui, cx, below, &cents, FontSize::Pt8, center, ink);
        }
    }
    for (rect, align, text) in readouts(&r, visual, &view) {
        widgets::text_label(ui, cx, rect, text, FontSize::Pt10, align, a);
    }
    Vec::new()
}

#[cfg(test)]
const SIDE: [Row; 3] = [
    row(1, Widget::Spinner, "Mode", [0.0, 0.0, 1.75, 2.49]).items("Steady · Reactive"),
    row(2, Widget::Spinner, "Visual", [0.0, 2.49, 1.75, 2.5]).items("Basic · Strobe · Orbit"),
    row(3, Widget::Display, "", [1.75, 0.0, 10.5, 4.99]).w(Size::Flex(6.0)),
];

#[cfg(test)]
const INPUT: [Row; 2] = [
    // Icon 22, shape 0.7, halo pad 4 px; on = channel fill, off = `moduleFill` fill, label `moduleFill` when on and
    // channel when off.
    row(
        6,
        Widget::Button { round: true },
        "",
        [12.25, 2.49, 1.75, 2.15],
    )
    .icon(IconId::AudioRecord)
    .shape(0.7)
    .halo(4.0)
    .colors(ButtonColors {
        fill_on: Ink::Channel,
        fill_off: Ink::ModuleFill,
        text_on: Ink::ModuleFill,
        text_off: Some(Ink::Channel),
    }),
    row(7, Widget::Text, "Input", [12.25, 4.64, 1.75, 0.35])
        .h(Size::U(0.35))
        .align(Align2::CENTER_CENTER),
];

#[cfg(test)]
pub const REFERENCE: Reference = Reference {
    size: [14.0, 4.99],
    settings: &[(1, 0.0), (2, 0.0), (4, 0.5), (5, 0.5), (6, 1.0)],
    cases: &[
        // Without a rack: the construction state.
        Case {
            name: "no rack",
            rows: &[
                SIDE[0],
                SIDE[1],
                SIDE[2],
                row(5, Widget::VSlider, "Reference", [12.25, 0.0, 1.75, 2.49])
                    .value_text()
                    .value("440 Hz"),
                INPUT[0],
                INPUT[1],
            ],
            ..Case::DEFAULT
        },
        // The knob takes the slider's slot.
        Case {
            name: "audio-input channel",
            flags: &[(flag::CHANNEL_KIND, 1)],
            rows: &[
                SIDE[0],
                SIDE[1],
                SIDE[2],
                row(4, Widget::Knob, "Reference", [12.25, 0.0, 1.75, 2.49]).value("440 Hz"),
                INPUT[0],
                INPUT[1],
            ],
            ..Case::DEFAULT
        },
        // The slider takes the whole column.
        Case {
            name: "other channel",
            flags: &[(flag::CHANNEL_KIND, 2)],
            rows: &[
                SIDE[0],
                SIDE[1],
                SIDE[2],
                row(5, Widget::VSlider, "Reference", [12.25, 0.0, 1.75, 4.99])
                    .value_text()
                    .value("440 Hz"),
            ],
            ..Case::DEFAULT
        },
    ],
};

#[cfg(test)]
mod tests {
    use eframe::egui::epaint::ColorMode;

    use super::*;
    use crate::metrics::{DEFAULT_UI_SCALE_INDEX, Metrics};
    use crate::panels::tests::{U, display_shapes};
    use crate::theme::theme_by_id;

    /// `x, y, w, h` in U as a rect in points (U = 32 points at any ppp).
    fn u_rect(x: f32, y: f32, w: f32, h: f32) -> Rect {
        Rect::from_min_size(pos2(x * U, y * U), vec2(w * U, h * U))
    }

    /// The reference display rect, (1.75, 0, 10.5, 4.99) U.
    fn reference_d() -> Rect {
        u_rect(1.75, 0.0, 10.5, 4.99)
    }

    /// Colour A at rest: `Color32::RED`, the test channel, dimmed to RGB × 0.75.
    const A: Color32 = Color32::from_rgb(191, 0, 0);

    #[test]
    fn rects_a_to_h_follow_the_display_rect() {
        for ppp in [1.0, 2.0] {
            let u = Metrics::from_pixels_per_point(ppp, DEFAULT_UI_SCALE_INDEX).u(1.0);
            let r = rects(reference_d(), u);
            let want = [
                ('A', r.a, u_rect(2.5375, 4.29, 8.925, 0.2)),
                ('B', r.b, u_rect(3.115, 4.29, 7.77, 0.1)),
                ('C', r.c, u_rect(2.57, 0.1, 2.0, 1.0)),
                ('D', r.d, u_rect(9.43, 0.1, 2.0, 1.0)),
                ('E', r.e, u_rect(4.375, 0.1, 5.25, 1.0)),
                ('F', r.f, u_rect(6.8, 3.59, 0.4, 0.4)),
                ('G', r.g, u_rect(2.315, 4.14, 0.4, 0.4)),
                ('H', r.h, u_rect(11.285, 4.14, 0.4, 0.4)),
            ];
            for (name, got, want) in want {
                let off = (got.min - want.min).length() + (got.max - want.max).length();
                assert!(off < 1e-3, "ppp {ppp}, {name}: {got:?}, want {want:?}");
            }
        }
    }

    #[test]
    fn the_strobe_bands_start_a_quarter_of_b_apart_at_phase_0() {
        // B = (3.115, 4.29, 7.77, 0.1) U: bands B.w/7 = 1.11 U wide, 1.75 band widths apart.
        let want = [3.115, 5.0575, 7.0, 8.9425].map(|x| u_rect(x, 4.29, 1.11, 0.1));
        for (got, want) in bands(rects(reference_d(), U).b, 0.0).into_iter().zip(want) {
            let off = (got.min - want.min).length() + (got.max - want.max).length();
            assert!(off < 1e-3, "{got:?}, want {want:?}");
        }
    }

    #[test]
    fn the_grid_has_eleven_lines_at_tenths() {
        let t = tenths(56.0, 336.0);
        assert_eq!((t[0], t[1], t[5], t[10]), (56.0, 89.6, 224.0, 392.0));
    }

    #[test]
    fn shade_scales_rgb_and_keeps_alpha() {
        assert_eq!(
            shade(Color32::from_rgb(200, 100, 40), 0.75),
            Color32::from_rgb(150, 75, 30)
        );
        let half = shade(Color32::from_rgba_unmultiplied(200, 100, 40, 128), 0.5);
        assert_eq!(half.a(), 128);
    }

    #[test]
    fn each_visual_has_its_triangles_and_readouts() {
        let r = rects(reference_d(), U);
        // Basic: F points down at A. Strobe: G points right and H left, both at B. Orbit: none.
        let f = [r.f.left_top(), r.f.right_top(), r.f.center_bottom()];
        let g = [r.g.left_top(), r.g.left_bottom(), r.g.right_center()];
        let h = [r.h.right_top(), r.h.right_bottom(), r.h.left_center()];
        assert_eq!(marks(&r, BASIC, 0.0, U), vec![f]);
        assert_eq!(marks(&r, STROBE, 0.0, U), vec![g, h]);
        assert!(marks(&r, ORBIT, 0.0, U).is_empty());
        // The 12 s reset strings; Orbit has no cents line.
        let hz = (r.c, Align2::LEFT_CENTER, "--- Hz");
        let status = (r.e, Align2::CENTER_CENTER, "Waiting for input...");
        let cents = (r.d, Align2::RIGHT_TOP, "--- cent");
        assert_eq!(readouts(&r, BASIC, &View::rest()), vec![hz, status, cents]);
        assert_eq!(readouts(&r, STROBE, &View::rest()), vec![hz, status, cents]);
        assert_eq!(readouts(&r, ORBIT, &View::rest()), vec![hz, status]);
    }

    #[test]
    fn the_orbit_at_rest_is_a_flat_ring_drawn_far_to_near() {
        let c = pos2(7.0 * U, 2.495 * U);
        // Per ppp, by hand (R = 5 U, Dz = 6 U + 360 px): dot 0 and dot 5 as (outer, inner, shade) in
        // points, then dot 1 (minor, near) the same way.
        let want = [
            (
                1.0,
                [
                    (48.9796, 45.2003, 0.89493),
                    (26.9663, 23.9663, 0.75),
                    (4.5438, 2.3996, 0.88109),
                ],
            ),
            (
                2.0,
                [
                    (45.2830, 42.5100, 0.96505),
                    (18.0451, 16.5451, 0.75),
                    (3.9578, 2.7223, 0.94452),
                ],
            ),
        ];
        for (ppp, [dot0, dot5, dot1]) in want {
            let dots = orbit(c, U, ppp, -FRAC_PI_2, 0.0);
            assert_eq!(dots.len(), 10);
            // Far to near: dot 5 (straight back) first, dots 1 and 9 next to last, dot 0 (the front) last.
            assert!(dots.windows(2).all(|w| w[0].z >= w[1].z), "ppp {ppp}");
            for (k, want) in [(0, dot5), (7, dot1), (8, dot1), (9, dot0)] {
                let got = (dots[k].outer, dots[k].inner, dots[k].shade);
                let off = (got.0 - want.0).abs() + (got.1 - want.1).abs() + (got.2 - want.2).abs();
                assert!(off < 1e-3, "ppp {ppp}, dot {k}: {got:?}, want {want:?}");
            }
            assert!((dots[0].at.x - c.x).abs() < 1e-3 && (dots[9].at.x - c.x).abs() < 1e-3);
            // Seen edge-on at tilt −π/2: one row 0.3 U below the centre, at x = cx + R·cos θᵢ, θᵢ = π/2 + i·36°.
            assert!(
                dots.iter().all(|d| (d.at.y - (c.y + 0.3 * U)).abs() < 1e-3),
                "ppp {ppp}"
            );
            let sorted = |mut v: Vec<f32>| {
                v.sort_by(f32::total_cmp);
                v
            };
            let xs = sorted(dots.iter().map(|d| d.at.x - c.x).collect());
            let want = sorted(
                (0..10)
                    .map(|i| 5.0 * U * (90.0 + 36.0 * i as f32).to_radians().cos())
                    .collect(),
            );
            assert!(
                xs.iter().zip(&want).all(|(x, w)| (x - w).abs() < 1e-3),
                "ppp {ppp}: {xs:?}"
            );
        }
    }

    /// What display #3 draws with `Visual` on `visual`, at `ppp` in the reference content rect (14 × 4.99 U).
    fn drawn(visual: usize, ppp: f32) -> Vec<Shape> {
        let mut state = PanelState::default();
        state.settings.insert(2, visual as f32);
        let content = Rect::from_min_size(pos2(0.0, 0.0), vec2(14.0 * U, 4.99 * U));
        display_shapes(24, 3, &[], &state, content, ppp)
    }

    /// The first two layers, the same for every visual: d filled `controlFill`, then 11 vertical and 11
    /// horizontal 1 px `background` lines at i/10. Returns d and the shapes after the grid.
    fn fill_and_grid(shapes: &[Shape], ppp: f32) -> (Rect, &[Shape]) {
        let colors = theme_by_id(0).colors;
        let Some(Shape::Rect(fill)) = shapes.first() else {
            panic!("{shapes:?}");
        };
        assert_eq!(fill.fill, colors.control_fill.to_color32());
        let d = fill.rect;
        // The layout's display rect, on whole pixels.
        let off = (d.min - reference_d().min).length() + (d.max - reference_d().max).length();
        assert!(off <= 1.0, "ppp {ppp}: {d:?}");
        for (k, s) in shapes[1..23].iter().enumerate() {
            let Shape::LineSegment { points, stroke } = s else {
                panic!("ppp {ppp}, grid line {k}: {s:?}");
            };
            assert_eq!(
                *stroke,
                Stroke::new(1.0 / ppp, colors.background.to_color32())
            );
            let i = (k % 11) as f32 / 10.0;
            let want = if k < 11 {
                let x = d.left() + i * d.width();
                [pos2(x, d.top()), pos2(x, d.bottom())]
            } else {
                let y = d.top() + i * d.height();
                [pos2(d.left(), y), pos2(d.right(), y)]
            };
            let off = (points[0] - want[0]).length() + (points[1] - want[1]).length();
            assert!(
                off < 1e-3,
                "ppp {ppp}, grid line {k}: {points:?}, want {want:?}"
            );
        }
        (d, &shapes[23..])
    }

    /// The readouts at the end of `shapes`: each at its rect's anchor, FontSize 2, colour A.
    fn assert_readouts(shapes: &[Shape], want: &[(Rect, Align2, &str)], ppp: f32) {
        let size = Metrics::from_pixels_per_point(ppp, DEFAULT_UI_SCALE_INDEX)
            .font_id(FontSize::Pt10)
            .size;
        let texts = &shapes[shapes.len() - want.len()..];
        for (s, &(rect, align, text)) in texts.iter().zip(want) {
            let Shape::Text(t) = s else {
                panic!("ppp {ppp}, {text}: {s:?}");
            };
            let format = &t.galley.job.sections[0].format;
            assert_eq!(
                (t.galley.text(), format.color, format.font_id.size),
                (text, A, size)
            );
            let at = t.galley.rect.translate(t.pos.to_vec2());
            let off = align.pos_in_rect(&at) - align.pos_in_rect(&rect);
            assert!(off.length() < 0.01, "ppp {ppp}, {text}: {at:?} in {rect:?}");
        }
    }

    #[test]
    fn basic_at_rest_frames_a_and_outlines_the_f_triangle() {
        for ppp in [1.0, 2.0] {
            let shapes = drawn(BASIC, ppp);
            let (d, visual) = fill_and_grid(&shapes, ppp);
            let r = rects(d, U);
            let [Shape::Rect(frame), Shape::Path(f), _, _, _] = visual else {
                panic!("ppp {ppp}: {visual:?}");
            };
            // A grown by 5 px, outlined 2 px wide in colour A, no fill.
            assert_eq!(frame.rect, r.a.expand(5.0 / ppp));
            assert_eq!(
                (frame.stroke, frame.stroke_kind),
                (Stroke::new(2.0 / ppp, A), StrokeKind::Middle)
            );
            assert_eq!(frame.fill, Color32::TRANSPARENT);
            // No pitch is never in tune: outlined 1.5 px in colour A, not filled.
            assert_eq!(f.points, marks(&r, BASIC, 0.0, U)[0].to_vec());
            assert!(f.closed && f.fill == Color32::TRANSPARENT, "ppp {ppp}");
            assert_eq!(
                (f.stroke.width, &f.stroke.color),
                (1.5 / ppp, &ColorMode::Solid(A))
            );
            assert_readouts(visual, &readouts(&r, BASIC, &View::rest()), ppp);
        }
    }

    #[test]
    fn strobe_at_rest_draws_four_still_bands_and_outlines_g_and_h() {
        for ppp in [1.0, 2.0] {
            let shapes = drawn(STROBE, ppp);
            let (d, visual) = fill_and_grid(&shapes, ppp);
            let r = rects(d, U);
            let [band @ .., Shape::Path(g), Shape::Path(h), _, _, _] = visual else {
                panic!("ppp {ppp}: {visual:?}");
            };
            // The 4 bands in colour B (= A at dev 0), then the triangles.
            let band: Vec<_> = band
                .iter()
                .map(|s| match s {
                    Shape::Rect(b) if b.fill == A => b.rect,
                    _ => panic!("ppp {ppp}: {s:?}"),
                })
                .collect();
            assert_eq!(band, bands(r.b, 0.0), "ppp {ppp}");
            for (path, want) in [(g, r.g), (h, r.h)] {
                assert!(
                    want.expand(1e-3)
                        .contains_rect(Rect::from_points(&path.points))
                );
                assert!(
                    path.closed && path.fill == Color32::TRANSPARENT,
                    "ppp {ppp}"
                );
                assert_eq!(
                    (path.stroke.width, &path.stroke.color),
                    (1.5 / ppp, &ColorMode::Solid(A))
                );
            }
            let want: Vec<_> = marks(&r, STROBE, 0.0, U)
                .iter()
                .map(|t| t.to_vec())
                .collect();
            assert_eq!(vec![g.points.clone(), h.points.clone()], want);
            assert_readouts(visual, &readouts(&r, STROBE, &View::rest()), ppp);
        }
    }

    #[test]
    fn orbit_at_rest_draws_ten_ringed_dots_then_hz_and_status() {
        let control = theme_by_id(0).colors.control_fill.to_color32();
        for ppp in [1.0, 2.0] {
            let shapes = drawn(ORBIT, ppp);
            let (d, visual) = fill_and_grid(&shapes, ppp);
            let r = rects(d, U);
            assert_eq!(visual.len(), 20 + 2, "ppp {ppp}");
            // The panel's centre is d's: the 1 : 6 : 1 columns centre d.
            let dots = orbit(d.center(), U, ppp, -FRAC_PI_2, 0.0);
            for (pair, dot) in visual.chunks(2).zip(&dots) {
                let [Shape::Circle(outer), Shape::Circle(inner)] = pair else {
                    panic!("ppp {ppp}: {pair:?}");
                };
                assert_eq!((outer.center, outer.radius), (dot.at, dot.outer));
                assert_eq!(outer.fill, shade(Color32::RED, 0.75 * dot.shade));
                // Not in tune: dot 0's inner disc is `controlFill` like the rest, so it reads as a ring.
                assert_eq!(
                    (inner.center, inner.radius, inner.fill),
                    (dot.at, dot.inner, control)
                );
            }
            assert_readouts(visual, &readouts(&r, ORBIT, &View::rest()), ppp);
        }
    }

    /// `pitch` after `n` readings of `hz`, one a frame (1/60 s apart), counting on from `count`.
    fn read(mut pitch: Pitch, hz: f32, n: u32, count: &mut u32, mode: usize) -> Pitch {
        for _ in 0..n {
            *count += 1;
            pitch = pitch.step(FRAME, Some((*count, hz)), mode, 440.0);
        }
        pitch
    }

    /// The frequency of MIDI note `p` on A4 = 440 Hz.
    fn hz(p: f32) -> f32 {
        440.0 * 2f32.powf((p - 69.0) / 12.0)
    }

    fn close(a: f32, b: f32) -> bool {
        (a - b).abs() < 1e-3
    }

    #[test]
    fn a_full_stable_window_moves_the_display_by_whole_semitones_then_a_glide() {
        let mut n = 0;
        // Steady: 4 readings within 0.1 semitone. Three show nothing yet.
        let p = read(Pitch::default(), hz(57.0), 3, &mut n, 0);
        assert_eq!(p.disp, None);
        let p = read(p, hz(57.0), 1, &mut n, 0);
        assert!(close(p.disp.unwrap(), 57.0));
        // A reading seen again (the count did not move) changes nothing.
        assert_eq!(
            p.clone().step(FRAME, Some((n, hz(60.0))), 0, 440.0).window,
            p.window
        );
        // 30 cents sharp: no whole-semitone jump, then disp = target + (disp − target)·0.8 per stable reading.
        let p = read(p, hz(57.3), 4, &mut n, 0);
        assert!(close(p.disp.unwrap(), 57.3 - 0.3 * 0.8), "{p:?}");
        // Two semitones up: jumps by 2, then glides.
        let before = p.disp.unwrap();
        let p = read(p, hz(59.0), 4, &mut n, 0);
        let jumped = before + 2.0;
        assert!(
            close(p.disp.unwrap(), 59.0 + (jumped - 59.0) * 0.8),
            "{p:?}"
        );
        // Reactive: 3 readings within a semitone, the raw reading the target.
        let mut n = 0;
        let p = read(Pitch::default(), hz(57.0), 2, &mut n, 1);
        assert_eq!(p.disp, None);
        let p = read(p, hz(57.4), 1, &mut n, 1);
        assert!(close(p.disp.unwrap(), 57.4));
        // A mode change restarts the averaging.
        let p = p.step(FRAME, None, 0, 440.0);
        assert!(p.window.is_empty());
    }

    #[test]
    fn silence_dims_after_4_s_and_resets_the_strings_after_12_keeping_the_note() {
        let mut n = 0;
        let p = read(Pitch::default(), 440.0, 4, &mut n, 0);
        let view = p.view(440.0);
        assert_eq!(
            (view.dim, view.note_name().as_str(), view.status),
            (1.0, "A5", "")
        );
        assert_eq!(
            (view.hz.as_str(), view.cents.as_str()),
            ("440.0 Hz", "+0 cent")
        );
        assert!(view.in_tune);
        // Silence (0 Hz readings): 4.5 s on, dimmed and no longer in tune; 12.5 s on, the reset strings.
        let p = read(p, 0.0, 270, &mut n, 0);
        let view = p.view(440.0);
        assert!(view.dim == DIM && !view.in_tune && view.hz == "440.0 Hz");
        let p = read(p, 0.0, 480, &mut n, 0);
        let view = p.view(440.0);
        assert_eq!(
            (view.hz.as_str(), view.cents.as_str(), view.status),
            (HZ, CENTS, STATUS)
        );
        assert_eq!(view.note_name(), "A5");
        // dev decays ×0.9 a frame once silent 4 s: from 20 cents flat.
        let mut n = 0;
        let p = read(Pitch::default(), hz(68.8), 4, &mut n, 0);
        assert!(close(p.dev, -0.2));
        let p = p.step(4.0, None, 0, 440.0);
        assert!(close(p.dev, -0.2), "{}", p.dev);
        let p = p.step(FRAME, None, 0, 440.0).step(FRAME, None, 0, 440.0);
        assert!(close(p.dev, -0.2 * 0.81), "{}", p.dev);
    }

    #[test]
    fn a_second_of_unsteady_input_says_so() {
        let mut n = 0;
        let mut p = Pitch::default();
        for k in 0..70 {
            p = read(p, hz(if k % 2 == 0 { 57.0 } else { 59.0 }), 1, &mut n, 0);
        }
        assert_eq!(p.view(440.0).status, UNSTABLE);
        assert_eq!(p.disp, None);
        // A stable reading clears it.
        let p = read(p, hz(57.0), 4, &mut n, 0);
        assert_eq!(p.view(440.0).status, "");
    }

    #[test]
    fn going_in_tune_pops_once_every_3_s_but_not_in_the_first_3() {
        let mut n = 0;
        let p = read(Pitch::default(), 440.0, 4, &mut n, 0);
        assert!(p.in_tune && p.pop.is_none(), "no pop within the first 3 s");
        // Out of tune (8 cents sharp after the glide towards 40), then back in after 3 s, once the glide brings dev
        // under 5 cents: the pop runs 0.1 … 1 over 10 frames, then 0.5, then 0.
        let p = read(p, hz(69.4), 4, &mut n, 0);
        assert!(!p.in_tune);
        let p = p.step(3.0, None, 0, 440.0);
        let mut p = read(p, 440.0, 5, &mut n, 0);
        assert!(!p.in_tune, "still gliding in");
        p = read(p, 440.0, 1, &mut n, 0);
        assert!(p.in_tune);
        let mut pops = vec![p.pop()];
        for _ in 0..12 {
            p = p.step(FRAME, None, 0, 440.0);
            pops.push(p.pop());
        }
        let want = [
            0.1, 0.2, 0.3, 0.4, 0.5, 0.6, 0.7, 0.8, 0.9, 1.0, 0.5, 0.0, 0.0,
        ];
        assert!(pops.iter().zip(want).all(|(a, b)| close(*a, b)), "{pops:?}");
        assert!(p.pop.is_none());
    }

    #[test]
    fn the_strobe_turns_with_the_cents_and_the_orbit_tilts_while_shown() {
        let mut n = 0;
        // 20 cents sharp: +20·0.0005 a frame.
        let p = read(Pitch::default(), hz(69.2), 4, &mut n, 0);
        let p2 = p.clone().step(FRAME, None, 0, 440.0);
        assert!(close(p2.phase - p.phase, 0.01), "{} {}", p.phase, p2.phase);
        // The tilt eases ×0.99 ten times a frame towards −1.4923, from the frame the pitch shows (the 4th reading).
        let want = TILT_SHOWN + (TILT - TILT_SHOWN) * 0.99f32.powf(10.0);
        assert!(close(p.tilt, want), "{}", p.tilt);
    }

    #[test]
    fn a_tone_in_tune_shows_its_note_hz_and_cents_and_fills_the_triangle() {
        use crate::panels::fx::leveller::feed_pass;
        use honeydew_audio::MeterSnapshot;
        let (theme, m) = (
            theme_by_id(0),
            Metrics::from_pixels_per_point(2.0, DEFAULT_UI_SCALE_INDEX),
        );
        let cx = UiCtx {
            theme: &theme,
            m: &m,
        };
        let ctx = eframe::egui::Context::default();
        let mut shapes = Vec::new();
        for k in 0..5u32 {
            let feed = (MeterSnapshot::default(), vec![k as f32, 440.0]);
            shapes = feed_pass(&ctx, f64::from(k) / 60.0, 48_000.0, Some(feed), |ui| {
                let mut module = ModuleInstance::new(24, 2);
                let mut state = PanelState::default();
                tuner(
                    ui,
                    &cx,
                    Id::new("tuner"),
                    reference_d(),
                    &mut module,
                    &mut state,
                    Color32::RED,
                );
            });
        }
        let texts: Vec<(String, Color32)> = (shapes.iter())
            .filter_map(|s| match s {
                Shape::Text(t) => Some((
                    t.galley.text().to_owned(),
                    t.galley.job.sections[0].format.color,
                )),
                _ => None,
            })
            .collect();
        let names: Vec<&str> = texts.iter().map(|(t, _)| t.as_str()).collect();
        assert_eq!(names, ["A5", "440.0 Hz", "", "+0 cent"]);
        // Undimmed, in tune: colour B is the channel; the F triangle is filled with colour A.
        assert_eq!(texts[0].1, Color32::RED);
        let f = shapes.iter().find_map(|s| match s {
            Shape::Path(p) if p.closed => Some(p),
            _ => None,
        });
        assert_eq!(f.unwrap().fill, Color32::RED);
    }

    #[test]
    fn the_strobe_bands_wrap_inside_b() {
        let b = u_rect(0.0, 0.0, 7.0, 0.1);
        // Phase 0.9: band 0 starts 0.9·B.w in, 1 U wide: 0.3 U of it shows at the right, 0.7 U at the left.
        let got = bands(b, 0.9);
        assert_eq!(got.len(), 5);
        assert!((got[0].left() - 6.3 * U).abs() < 1e-3 && (got[0].right() - 7.0 * U).abs() < 1e-3);
        assert!((got[1].left()).abs() < 1e-3 && (got[1].right() - 0.3 * U).abs() < 1e-3);
    }

    #[test]
    fn the_deviation_fill_grows_from_as_centre_by_the_cents() {
        let a = u_rect(0.0, 0.0, 10.0, 0.2);
        let sharp = deviation(a, 0.25).unwrap();
        assert!((sharp.left() - 5.0 * U).abs() < 1e-3 && (sharp.width() - 2.5 * U).abs() < 1e-3);
        let flat = deviation(a, -0.1).unwrap();
        assert!((flat.right() - 5.0 * U).abs() < 1e-3 && (flat.width() - U).abs() < 1e-3);
        assert_eq!(deviation(a, 0.004), None);
    }

    /// The hook the panel runs, which the tests go through: a panel without it fails them.
    fn follow(m: &mut ModuleInstance, s: &mut PanelState, edited: &[u32]) -> Vec<usize> {
        (panel().unwrap().hook.expect("a hook"))(m, s, edited)
    }

    #[test]
    fn reference_is_one_setting_and_input_is_off_off_an_audio_channel() {
        let mut module = ModuleInstance::new(24, 2);
        let panel = panel().expect("a panel");
        let mut state = PanelState::new(&panel);
        state.settings.insert(4, 0.75);
        assert!(follow(&mut module, &mut state, &[4]).is_empty());
        assert_eq!(state.settings[&5], 0.75);
        state.settings.insert(5, 0.25);
        follow(&mut module, &mut state, &[5]);
        assert_eq!(state.settings[&4], 0.25);
        // No edit, no copy.
        state.settings.insert(5, 0.5);
        follow(&mut module, &mut state, &[]);
        assert_eq!(state.settings[&4], 0.25);
        // The Input button stays as it is on an audio-input channel and without a rack, and is off on any other.
        for (kind, input) in [(0, 1.0), (1, 1.0), (2, 0.0)] {
            state.flags.insert(flag::CHANNEL_KIND, kind);
            state.settings.insert(6, 1.0);
            assert!(follow(&mut module, &mut state, &[]).is_empty());
            assert_eq!(state.settings[&6], input, "channel kind {kind}");
        }
        assert_eq!(module.params, ModuleInstance::new(24, 2).params);
    }

    #[test]
    fn middle_c_reads_c5_or_c4_with_set_middle_c_to_c4() {
        use crate::panels::fx::leveller::feed_pass;
        use honeydew_audio::MeterSnapshot;
        let (theme, m) = (
            theme_by_id(0),
            Metrics::from_pixels_per_point(1.0, DEFAULT_UI_SCALE_INDEX),
        );
        let cx = UiCtx {
            theme: &theme,
            m: &m,
        };
        // MIDI 60 at A = 440 Hz; the card's octave offset 0 (off) and 1 (on).
        for (offset, want) in [(0, "C5"), (1, "C4")] {
            let ctx = eframe::egui::Context::default();
            let mut shapes = Vec::new();
            for k in 0..5u32 {
                let feed = (MeterSnapshot::default(), vec![k as f32, hz(60.0)]);
                shapes = feed_pass(&ctx, f64::from(k) / 60.0, 48_000.0, Some(feed), |ui| {
                    let mut module = ModuleInstance::new(24, 2);
                    let mut state = PanelState::default();
                    state.flags.insert(flag::OCTAVE_OFFSET, offset);
                    tuner(
                        ui,
                        &cx,
                        Id::new("tuner"),
                        reference_d(),
                        &mut module,
                        &mut state,
                        Color32::RED,
                    );
                });
            }
            let first = shapes.iter().find_map(|s| match s {
                Shape::Text(t) => Some(t.galley.text().to_owned()),
                _ => None,
            });
            assert_eq!(first.as_deref(), Some(want), "offset {offset}");
        }
    }
}
