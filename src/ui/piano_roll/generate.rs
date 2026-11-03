//! The `Generate` panel: chords and a melody for the clip, in the project's key and scale ([`super::compose`]). The
//! note menu's `Generate...` opens it over the piano roll, and the roll draws its result ([`super::compose::generate`])
//! in place of the notes while it is open, recomputed from the clip every frame: the song itself waits for `Apply`, so
//! a preview is no edit and makes no undo step (as the Humanize panel's).
//!
//! The panel is the dialog look without the dim, so the preview shows: 14 U wide at the roll's right edge, centred on
//! its height and kept inside the screen. Top to bottom: the title, then a row each for `Preset` (Pop, Minor, Lo-fi,
//! Epic — tapping it steps to the next), `Bars` (1, 2, 4, 8), `Lead` (Chords: a melody written over them; Melody: the
//! chords fitted under the clip's own notes) and `Seed` (its number and `New seed`), then `Reset`, `Cancel` and
//! `Apply`.

use std::hash::{BuildHasher, RandomState};

use eframe::egui::{
    Align2, Area, Color32, Context, Id, LayerId, Order, Rect, ScrollArea, Sense, Ui, UiBuilder,
    pos2, scroll_area::ScrollBarVisibility, vec2,
};

use super::compose::{
    self, ARPS, CHORD_MODES, DURATIONS, KEYS, Lead, OCTAVES, PRESETS, RESTS, SCALE_CHOICES, Settings,
};
use crate::song::scales::SCALES;
use crate::metrics::{FontSize, Metrics};
use crate::song::Event;
use crate::ui::widgets;
use crate::ui::{UiCtx, dialog};

// Sizes in U.
const WIDTH: f32 = 14.0;
/// Between the panel and the screen's or the roll's edges; the panel's padding; the gap above the buttons.
const MARGIN: f32 = 0.5;
const TITLE: f32 = 1.25;
const HEADER: f32 = 0.75;
/// A row with its choice button.
const CHOICE: f32 = 1.25;
const LABEL_W: f32 = 4.5;
const BUTTON: [f32; 2] = [4.5, 1.0];
/// The scrolling rows' height: three headers and eleven choice rows.
const BODY: f32 = 3.0 * HEADER + 11.0 * CHOICE;
/// The whole panel, when the screen has room.
const HEIGHT: f32 = 2.0 * MARGIN + TITLE + BODY + MARGIN + CHOICE;
/// Seeds run 0..SEEDS.
const SEEDS: u64 = 100_000;
/// The section header's `text` alpha.
const DIM: f32 = 0.5;
/// The `Bars` choices.
const BARS: [u8; 4] = [1, 2, 4, 8];

/// The panel's state: the preset, the bars, who leads and the seed, then the key, scale and register (0 following the
/// project) and the rhythm: the note length, the rests, the arpeggios and which chords to write.
#[derive(Clone, Copy, Debug, PartialEq)]
pub(crate) struct Panel {
    preset: usize,
    bars: usize,
    lead: usize,
    seed: u64,
    /// 0: the project's; else 1 + a [`KEYS`] index.
    key: usize,
    /// 0: the project's; else 1 + a [`SCALE_CHOICES`] index.
    scale: usize,
    octave: usize,
    duration: usize,
    rests: usize,
    arp: usize,
    chords: usize,
}

/// How the panel closed.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum Close {
    Apply,
    Cancel,
}

impl Panel {
    /// Pop, 2 bars, the chords leading, on seed `seed` (kept to 0..100 000).
    pub(crate) fn new(seed: u64) -> Self {
        Self {
            preset: 0,
            bars: 1,
            lead: 0,
            seed: seed % SEEDS,
            key: 0,
            scale: 0,
            octave: 2,
            duration: 2,
            rests: 1,
            arp: 0,
            chords: 0,
        }
    }

    /// What the rows say ([module docs](self)).
    pub(crate) fn settings(&self) -> Settings {
        Settings {
            preset: PRESETS[self.preset.min(PRESETS.len() - 1)].1,
            bars: BARS[self.bars.min(BARS.len() - 1)],
            seed: self.seed,
            lead: if self.lead == 0 {
                Lead::Chords
            } else {
                Lead::Melody
            },
            key: (self.key > 0).then(|| (self.key - 1) as u8),
            scale: (self.scale > 0).then(|| SCALE_CHOICES[self.scale - 1]),
            octave: OCTAVES[self.octave.min(OCTAVES.len() - 1)].1,
            duration: DURATIONS[self.duration.min(DURATIONS.len() - 1)].1,
            rest: RESTS[self.rests.min(RESTS.len() - 1)].1,
            arp: ARPS[self.arp.min(ARPS.len() - 1)].1,
            chords: self.chords.min(CHORD_MODES.len() - 1) as u8,
        }
    }

    /// `events` as `Apply` would leave them, in the project's key and scale, `bpb` beats a bar.
    pub(crate) fn preview(&self, events: &[Event], scale: (u8, u16), bpb: f64) -> Vec<Event> {
        compose::generate(&self.settings(), scale, events, bpb)
    }
}

/// The panel's rect on `screen` beside the roll `over`, on whole device pixels.
fn layout(screen: Rect, over: Rect, m: &Metrics) -> Rect {
    let w = m.round_px(m.u(WIDTH).min(screen.width() - m.u(2.0 * MARGIN)));
    let h = m.round_px(m.u(HEIGHT).min(screen.height() - m.u(2.0 * MARGIN)));
    let right = (over.right() - m.u(MARGIN)).min(screen.right() - m.u(MARGIN));
    let top = (over.center().y - h / 2.0)
        .min(screen.bottom() - m.u(MARGIN) - h)
        .max(screen.top() + m.u(MARGIN));
    Rect::from_min_size(pos2(m.round_px(right - w), m.round_px(top)), vec2(w, h))
}

/// One frame of the panel beside the roll `over`; `Some` when it closes.
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
                "Generate",
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
                *panel = Panel::new(panel.seed);
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

/// The scrolling rows in `column`: the preset, bars, lead and seed.
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
    // The row's choice button, at its right end: tapping it steps to the next choice.
    let choice = |ui: &mut Ui, r: Rect, name: &str, s: &str| {
        let at = pos2(
            r.right() - m.u(BUTTON[0]),
            r.center().y - m.u(BUTTON[1]) / 2.0,
        );
        let b = dialog::button(id.with(name), button_rect(at, m), s, c, m);
        widgets::button(ui, cx, &b, true).clicked()
    };
    label(
        ui,
        next(HEADER),
        "Progression & melody",
        ink.gamma_multiply(DIM),
    );
    let r = next(CHOICE);
    label(ui, r, "Preset", text);
    if choice(ui, r, "preset", PRESETS[panel.preset].0) {
        panel.preset = (panel.preset + 1) % PRESETS.len();
    }
    let r = next(CHOICE);
    label(ui, r, "Bars", text);
    if choice(ui, r, "bars", &BARS[panel.bars].to_string()) {
        panel.bars = (panel.bars + 1) % BARS.len();
    }
    let r = next(CHOICE);
    label(ui, r, "Lead", text);
    let lead = if panel.lead == 0 { "Chords" } else { "Melody" };
    if choice(ui, r, "lead", lead) {
        panel.lead = 1 - panel.lead;
    }
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
    if choice(ui, r, "new seed", "New seed") {
        let now = ui.input(|i| i.time).to_bits();
        panel.seed = RandomState::new().hash_one(now) % SEEDS;
    }
    label(
        ui,
        next(HEADER),
        "Key, scale & register",
        ink.gamma_multiply(DIM),
    );
    let r = next(CHOICE);
    label(ui, r, "Key", text);
    let key = if panel.key == 0 {
        "Project"
    } else {
        KEYS[panel.key - 1]
    };
    if choice(ui, r, "key", key) {
        panel.key = (panel.key + 1) % (KEYS.len() + 1);
    }
    let r = next(CHOICE);
    label(ui, r, "Scale", text);
    let scale = if panel.scale == 0 {
        "Project"
    } else {
        SCALES[SCALE_CHOICES[panel.scale - 1]]
    };
    if choice(ui, r, "scale", scale) {
        panel.scale = (panel.scale + 1) % (SCALE_CHOICES.len() + 1);
    }
    let r = next(CHOICE);
    label(ui, r, "Octave", text);
    if choice(ui, r, "octave", OCTAVES[panel.octave].0) {
        panel.octave = (panel.octave + 1) % OCTAVES.len();
    }
    label(ui, next(HEADER), "Notes & rhythm", ink.gamma_multiply(DIM));
    let r = next(CHOICE);
    label(ui, r, "Length", text);
    if choice(ui, r, "length", DURATIONS[panel.duration].0) {
        panel.duration = (panel.duration + 1) % DURATIONS.len();
    }
    let r = next(CHOICE);
    label(ui, r, "Rests", text);
    if choice(ui, r, "rests", RESTS[panel.rests].0) {
        panel.rests = (panel.rests + 1) % RESTS.len();
    }
    let r = next(CHOICE);
    label(ui, r, "Arpeggios", text);
    if choice(ui, r, "arps", ARPS[panel.arp].0) {
        panel.arp = (panel.arp + 1) % ARPS.len();
    }
    let r = next(CHOICE);
    label(ui, r, "Chords", text);
    if choice(ui, r, "chords", CHORD_MODES[panel.chords]) {
        panel.chords = (panel.chords + 1) % CHORD_MODES.len();
    }
}
