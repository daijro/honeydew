//! The `Humanize` panel (issue #29), where `Humanize` used to have no dialog and no settings. The note menu's
//! `Humanize` opens it over the piano roll, and the roll draws its result
//! ([`super::notes::humanize`]) in place of the notes while it is open, recomputed from the clip's notes every frame:
//! the song itself waits for `Apply`, so a preview is no edit and makes no undo step.
//!
//! The panel is the dialog look ([`dialog::panel`]) without the dim, so the preview shows: min(14 U, W − 1 U) wide, at
//! the roll's right edge less 0.5 U, centred on the roll's height and kept 0.5 U inside the screen, at most the screen
//! less 1 U tall. Top to bottom: the title, then rows that scroll when they do not fit (a phone), then `Reset`,
//! `Cancel` and `Apply`. The rows: each slider under its section's header (`text` at α 0.5) with its name on the left
//! and its value on the right (10 pt `text`), the sliders in the channel colour (double tap: its default):
//! - `Randomize`: `Timing` σ 0–0.1 beat (0.025), `Length` σ 0–50 % of the note's length (0), `Velocity` σ 0–20 % of
//!   full scale (5 %);
//! - `Modulators`, bipolar: `Velocity to Length` and `Pitch to Length` (the issue's `Velocity → Length` and
//!   `Pitch → Length`: the app font has no arrow), −100 to +100 % (0);
//! - `Base adjustments`, bipolar: `Velocity` −50 to +50 % of full scale, `Length` ×0.25 to ×4 (log), `Timing` ±0.25
//!   beat in whole ticks;
//! - `Seed`, its number (0–99999, a fresh one at each opening) and, at the right, `New seed`;
//! - `Apply to`: `Selected` (with a selection only) or `All`, the current one lit, at the right; `Selected` when the
//!   panel opened on a selection.
//!
//! `Reset` puts every slider and `Apply to` back as the panel opened, keeping the seed. `Apply` writes the result as
//! one undo step and closes; `Cancel`, Escape or a tap outside the panel (a backdrop takes it, and holds the keyboard)
//! close it with the notes as they were.

use std::hash::{BuildHasher, RandomState};

use eframe::egui::{
    Align2, Area, Color32, Context, Id, LayerId, Order, Rect, ScrollArea, Sense, Ui, UiBuilder,
    pos2, scroll_area::ScrollBarVisibility, vec2,
};

use super::notes::{Humanize, humanize};
use crate::metrics::{FontSize, Metrics};
use crate::song::Event;
use crate::ui::widgets::{self, Control, ParamMenu};
use crate::ui::{UiCtx, dialog};

// Sizes in U.
const WIDTH: f32 = 14.0;
/// Between the panel and the screen's or the roll's edges; the panel's padding; the gap above the buttons.
const MARGIN: f32 = 0.5;
const TITLE: f32 = 1.25;
const HEADER: f32 = 0.75;
const ROW: f32 = 1.0;
/// The seed and `Apply to` rows, and the buttons' row.
const CHOICE: f32 = 1.25;
const LABEL_W: f32 = 4.5;
const VALUE_W: f32 = 2.75;
const BUTTON: [f32; 2] = [3.5, 1.0];
/// The scrolling rows' height: three headers, eight sliders, two choice rows.
const BODY: f32 = 3.0 * HEADER + 8.0 * ROW + 2.0 * CHOICE;
/// The whole panel, when the screen has room.
const HEIGHT: f32 = 2.0 * MARGIN + TITLE + BODY + MARGIN + CHOICE;
/// Seeds run 0..SEEDS.
const SEEDS: u64 = 100_000;
/// The section headers' `text` alpha.
const DIM: f32 = 0.5;

/// The sliders, in order, with their defaults (the plain `Humanize`); their sections are [`SECTIONS`].
const LABELS: [&str; 8] = [
    "Timing",
    "Length",
    "Velocity",
    "Velocity to Length",
    "Pitch to Length",
    "Velocity",
    "Length",
    "Timing",
];
const DEFAULTS: [f32; 8] = [0.25, 0.0, 0.25, 0.5, 0.5, 0.5, 0.5, 0.5];
const SECTIONS: [(&str, std::ops::Range<usize>); 3] = [
    ("Randomize", 0..3),
    ("Modulators", 3..5),
    ("Base adjustments", 5..8),
];

/// The panel's state: its sliders' values, the seed and `Apply to`.
#[derive(Clone, Copy, Debug, PartialEq)]
pub(crate) struct Panel {
    values: [f32; 8],
    seed: u64,
    all: bool,
    /// A selection showed when it opened: `Apply to` starts at `Selected`, which it offers.
    selection: bool,
}

/// How the panel closed.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum Close {
    Apply,
    Cancel,
}

impl Panel {
    /// The defaults on seed `seed` (kept to 0..100 000), on the selection if there is one.
    pub(crate) fn new(seed: u64, selection: bool) -> Self {
        Self {
            values: DEFAULTS,
            seed: seed % SEEDS,
            all: !selection,
            selection,
        }
    }

    /// What the sliders and `Apply to` say ([module docs](self)).
    pub(crate) fn settings(&self) -> Humanize {
        let [t, l, v, vl, pl, vo, ls, to] = self.values.map(f64::from);
        Humanize {
            timing: 0.1 * t,
            length: 0.5 * l,
            velocity: 0.2 * v,
            vel_to_len: 2.0 * vl - 1.0,
            pitch_to_len: 2.0 * pl - 1.0,
            vel_offset: vo - 0.5,
            len_scale: 4f64.powf(2.0 * ls - 1.0),
            time_offset: ((2.0 * to - 1.0) * 32.0).round(),
            all: self.all,
        }
    }

    /// `events` as `Apply` would leave them, in their order, `sel` the roll's selection.
    pub(crate) fn preview(&self, events: &[Event], sel: &[bool]) -> Vec<Event> {
        humanize(events, sel, &self.settings(), self.seed)
    }
}

/// Slider `i`'s value text.
fn value_text(i: usize, s: &Humanize) -> String {
    match i {
        0 => format!("{:.3} beat", s.timing),
        1 => format!("{:.0} %", s.length * 100.0),
        2 => format!("{:.0} %", s.velocity * 100.0),
        3 => format!("{:+.0} %", s.vel_to_len * 100.0),
        4 => format!("{:+.0} %", s.pitch_to_len * 100.0),
        5 => format!("{:+.0} %", s.vel_offset * 100.0),
        6 => format!("×{:.2}", s.len_scale),
        _ => format!("{:+.3} beat", s.time_offset / 128.0),
    }
}

/// The panel's rect on `screen` beside the roll `over` ([module docs](self)), on whole device pixels.
fn layout(screen: Rect, over: Rect, m: &Metrics) -> Rect {
    let w = m.round_px(m.u(WIDTH).min(screen.width() - m.u(2.0 * MARGIN)));
    let h = m.round_px(m.u(HEIGHT).min(screen.height() - m.u(2.0 * MARGIN)));
    let right = (over.right() - m.u(MARGIN)).min(screen.right() - m.u(MARGIN));
    let top = (over.center().y - h / 2.0)
        .min(screen.bottom() - m.u(MARGIN) - h)
        .max(screen.top() + m.u(MARGIN));
    Rect::from_min_size(pos2(m.round_px(right - w), m.round_px(top)), vec2(w, h))
}

/// One frame of the panel beside the roll `over`, its sliders in `ink`; `Some` when it closes.
pub(crate) fn show(
    ctx: &Context,
    cx: &UiCtx,
    id: Id,
    over: Rect,
    ink: Color32,
    panel: &mut Panel,
) -> Option<Close> {
    let (m, c) = (cx.m, &cx.theme.colors);
    let rect = layout(ctx.content_rect(), over, m);
    // In the panel's layer, before its area, which egui lays out unseen on its first frame.
    dialog::panel(
        &ctx.layer_painter(LayerId::new(Order::Foreground, id)),
        c,
        m,
        rect,
    );
    let backdrop = crate::ui::backdrop(ctx, id);
    let outside = backdrop.clicked() || backdrop.secondary_clicked();
    let mut close = (crate::ui::close_pressed(ctx) || outside).then_some(Close::Cancel);
    Area::new(id)
        .order(Order::Foreground)
        .fixed_pos(rect.min)
        .constrain(false)
        .fade_in(false)
        .show(ctx, |ui| {
            // The whole panel, so it swallows the touches that hit no control.
            ui.allocate_exact_size(rect.size(), Sense::hover());
            let inner = rect.shrink(m.u(MARGIN));
            let text = c.text.to_color32();
            ui.painter().text(
                pos2(inner.left(), inner.top() + m.u(TITLE) / 2.0),
                Align2::LEFT_CENTER,
                "Humanize",
                m.font_id(FontSize::Pt12),
                text,
            );
            let buttons = inner.with_min_y(inner.bottom() - m.u(CHOICE));
            let room = Rect::from_x_y_ranges(
                inner.x_range(),
                inner.top() + m.u(TITLE)..=buttons.top() - m.u(MARGIN),
            );
            let mut scrolled = ui.new_child(UiBuilder::new().max_rect(room));
            ScrollArea::vertical()
                .id_salt(id.with("scroll"))
                .max_height(room.height())
                .scroll_bar_visibility(ScrollBarVisibility::AlwaysHidden)
                .show(&mut scrolled, |ui| {
                    let (column, _) =
                        ui.allocate_exact_size(vec2(room.width(), m.u(BODY)), Sense::hover());
                    rows(ui, cx, id, column, ink, panel);
                });
            let button = |ui: &mut Ui, x: f32, name: &str, label: &str| {
                let at = pos2(x, buttons.center().y - m.u(BUTTON[1]) / 2.0);
                let b = dialog::button(id.with(name), button_rect(at, m), label, c, m);
                widgets::button(ui, cx, &b, true).clicked()
            };
            if button(ui, inner.left(), "reset", "Reset") {
                *panel = Panel::new(panel.seed, panel.selection);
            }
            let step = m.u(BUTTON[0] + MARGIN);
            if button(
                ui,
                inner.right() - step - m.u(BUTTON[0]),
                "cancel",
                "Cancel",
            ) {
                close = Some(Close::Cancel);
            }
            if button(ui, inner.right() - m.u(BUTTON[0]), "apply", "Apply") {
                close = Some(Close::Apply);
            }
        });
    close
}

/// A button's rect from its top-left.
fn button_rect(at: eframe::egui::Pos2, m: &Metrics) -> Rect {
    Rect::from_min_size(at, vec2(m.u(BUTTON[0]), m.u(BUTTON[1])))
}

/// The scrolling rows in `column`: the sections' sliders, the seed and `Apply to`.
fn rows(ui: &mut Ui, cx: &UiCtx, id: Id, column: Rect, ink: Color32, panel: &mut Panel) {
    let (m, c) = (cx.m, &cx.theme.colors);
    let (font, text) = (m.font_id(FontSize::Pt10), c.text.to_color32());
    let mut y = column.top();
    let mut next = |h: f32| {
        y += m.u(h);
        Rect::from_x_y_ranges(column.x_range(), y - m.u(h)..=y)
    };
    let label = |ui: &Ui, r: Rect, s: &str, color: Color32| {
        let at = pos2(r.left(), r.center().y);
        ui.painter()
            .text(at, Align2::LEFT_CENTER, s, font.clone(), color);
    };
    let settings = panel.settings();
    for (header, sliders) in SECTIONS {
        label(ui, next(HEADER), header, text.gamma_multiply(DIM));
        for i in sliders {
            let r = next(ROW);
            label(ui, r, LABELS[i], text);
            let track = r
                .with_min_x(r.left() + m.u(LABEL_W))
                .with_max_x(r.right() - m.u(VALUE_W));
            let control = Control {
                menu: ParamMenu::Off,
                ..Control::new(id.with(("row", i)), track, "", ink, DEFAULTS[i])
            };
            widgets::h_slider(ui, cx, &control, &mut panel.values[i], |_| String::new());
            let at = pos2(r.right(), r.center().y);
            let value = value_text(i, &settings);
            ui.painter()
                .text(at, Align2::RIGHT_CENTER, value, font.clone(), text);
        }
    }
    // A choice row's buttons line up from its right end, the `k`th from the right.
    let choice = |ui: &mut Ui, r: Rect, k: usize, name: &str, s: &str, on: bool, enabled: bool| {
        let at = pos2(
            r.right() - m.u(BUTTON[0]) - k as f32 * m.u(BUTTON[0] + MARGIN),
            r.center().y - m.u(BUTTON[1]) / 2.0,
        );
        let b = widgets::Button {
            fill_off: c.control_fill.to_color32(),
            text_off: Some(text.gamma_multiply(if enabled { 1.0 } else { DIM })),
            disabled: !enabled,
            ..dialog::button(id.with(name), button_rect(at, m), s, c, m)
        };
        widgets::button(ui, cx, &b, on).clicked()
    };
    let r = next(CHOICE);
    label(ui, r, "Seed", text);
    let at = pos2(r.left() + m.u(LABEL_W), r.center().y);
    ui.painter().text(
        at,
        Align2::LEFT_CENTER,
        panel.seed.to_string(),
        font.clone(),
        text,
    );
    if choice(ui, r, 0, "new seed", "New seed", false, true) {
        let now = ui.input(|i| i.time).to_bits();
        panel.seed = RandomState::new().hash_one(now) % SEEDS;
    }
    let r = next(CHOICE);
    label(ui, r, "Apply to", text);
    let all = panel.all;
    if choice(ui, r, 1, "selected", "Selected", !all, panel.selection) {
        panel.all = false;
    }
    if choice(ui, r, 0, "all", "All", all, true) {
        panel.all = true;
    }
}
