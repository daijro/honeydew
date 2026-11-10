//! The `Generate` panel, a contributor feature (PR #87): chords and a melody for the clip, in the
//! project's key and scale or the panel's own ([`super::compose`]). The transport bar's GENERATE opens it over the
//! piano roll. While it is open its result ([`super::compose::generate`]) is in the clip in place of the clip's notes,
//! as `Apply` leaves it, so the song plays it (the roll's `audition`); `Apply` keeps it as one undo
//! step, and any other close puts the clip back.
//!
//! The panel is the dialog look without the dim, so the preview shows: 14 U wide at the roll's right edge, centred on
//! its height and kept inside the screen. Top to bottom, scrolling when the screen is short: the title; under
//! `Progression & melody` a row each for `Preset` (Pop, Minor, Lo-fi, Epic, Random), `Bars` (1, 2, 4, 8), `Lead`
//! (Chords: a melody written over them; Melody: the chords fitted under the clip's own notes) and `Seed` (its number
//! and `New seed`); under `Key, scale & register` the `Key` and `Scale` (`Project` follows the song's) and the
//! `Octave`; under `Notes & rhythm` the `Length`, `Rests`, `Arpeggios` and `Chords`. Each choice is a dropdown, modal
//! over the panel too, so one opens at a time (#107). Then `Reset`, `Cancel` and `Apply`. A tap outside, Escape or
//! Android's Back close the open dropdown, else the panel, as `Cancel`; but the transport's Play, and the keymap's
//! `play`, start and stop playback under it ([`crate::ui::backdrop_open_to_play`]).

use std::hash::{BuildHasher, RandomState};

use eframe::egui::{
    Align2, Area, Color32, Context, Id, LayerId, Order, Rect, ScrollArea, Sense, Ui, UiBuilder,
    pos2, scroll_area::ScrollBarVisibility, vec2,
};

use super::compose::{
    self, ARPS, CHORD_MODES, DURATIONS, KEYS, Lead, OCTAVES, PRESETS, RESTS, SCALE_CHOICES,
    Settings,
};
use crate::metrics::{FontSize, Metrics};
use crate::params::{menu_index, menu_value};
use crate::song::Event;
use crate::song::scales::SCALES;
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
/// As the Humanize panel's: three fit the 13 U row with their gaps.
const BUTTON: [f32; 2] = [3.5, 1.0];
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
/// The `Lead` choices ([`Lead`]).
const LEADS: [&str; 2] = ["Chords", "Melody"];

/// The panel's state: the preset, the bars, who leads and the seed, then the key, scale and register (0 following the
/// project) and the rhythm: the note length, the rests, the arpeggios and which chords to write.
#[derive(Clone, Copy, Debug, PartialEq)]
pub(crate) struct Panel {
    preset: usize,
    bars: usize,
    /// 0: the chords lead; 1: the melody.
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
    // Kept inside the screen beside a roll the rack narrows (#107).
    let left = (right - w).max(screen.left() + m.u(MARGIN));
    Rect::from_min_size(pos2(m.round_px(left), m.round_px(top)), vec2(w, h))
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
    // Play and Pause stay live, for the preview to be heard.
    let outside = crate::ui::backdrop_open_to_play(ctx, id);
    let mut close = outside.then_some(Close::Cancel);
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
    // After the rows, so that an open dropdown takes Escape (and Android's Back) first.
    if crate::ui::close_pressed(ctx) {
        close = close.or(Some(Close::Cancel));
    }
    close
}

/// A button's rect from its top-left.
fn button_rect(at: eframe::egui::Pos2, m: &Metrics) -> Rect {
    Rect::from_min_size(at, vec2(m.u(BUTTON[0]), m.u(BUTTON[1])))
}

/// The scrolling rows in `column` ([module docs](self)).
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
    // Every choice opens a dropdown instead of stepping through values one tap at a time.
    let dropdown = |ui: &mut Ui,
                    r: Rect,
                    name: &str,
                    items: &[&str],
                    current: usize,
                    columns: Option<usize>| {
        let width = m.u(7.0).min(r.width());
        let at = pos2(r.right() - width, r.center().y - m.u(BUTTON[1]) / 2.0);
        let rect = Rect::from_min_size(at, vec2(width, m.u(BUTTON[1])));
        let control = widgets::Control {
            menu: widgets::ParamMenu::Off,
            column_len: columns,
            ..widgets::Control::new(id.with(name), rect, "", ink, 0.0)
        };
        let mut value = menu_value(current, items.len());
        if widgets::menu_button(ui, cx, &control, items, &mut value).changed() {
            menu_index(value, items.len())
        } else {
            current.min(items.len().saturating_sub(1))
        }
    };
    let action_button = |ui: &mut Ui, r: Rect, name: &str, text: &str| {
        let at = pos2(
            r.right() - m.u(BUTTON[0]),
            r.center().y - m.u(BUTTON[1]) / 2.0,
        );
        let b = dialog::button(id.with(name), button_rect(at, m), text, c, m);
        widgets::button(ui, cx, &b, true).clicked()
    };
    const BAR_LABELS: [&str; 4] = ["1", "2", "4", "8"];
    let keys: Vec<&str> = std::iter::once("Project")
        .chain(KEYS.iter().copied())
        .collect();
    let scales: Vec<&str> = std::iter::once("Project")
        .chain(SCALE_CHOICES.iter().map(|&i| SCALES[i]))
        .collect();
    let octaves: Vec<&str> = OCTAVES.iter().map(|(name, _)| *name).collect();
    let durations: Vec<&str> = DURATIONS.iter().map(|(name, _)| *name).collect();
    let rests: Vec<&str> = RESTS.iter().map(|(name, _)| *name).collect();
    let arps: Vec<&str> = ARPS.iter().map(|(name, _)| *name).collect();
    let presets: Vec<&str> = PRESETS.iter().map(|(name, _)| *name).collect();

    label(
        ui,
        next(HEADER),
        "Progression & melody",
        ink.gamma_multiply(DIM),
    );
    let r = next(CHOICE);
    label(ui, r, "Preset", text);
    panel.preset = dropdown(ui, r, "preset", &presets, panel.preset, None);
    let r = next(CHOICE);
    label(ui, r, "Bars", text);
    panel.bars = dropdown(ui, r, "bars", &BAR_LABELS, panel.bars, None);
    let r = next(CHOICE);
    label(ui, r, "Lead", text);
    panel.lead = dropdown(ui, r, "lead", &LEADS, panel.lead, None);
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
    if action_button(ui, r, "new seed", "New seed") {
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
    panel.key = dropdown(ui, r, "key", &keys, panel.key, Some(7));
    let r = next(CHOICE);
    label(ui, r, "Scale", text);
    panel.scale = dropdown(ui, r, "scale", &scales, panel.scale, Some(9));
    let r = next(CHOICE);
    label(ui, r, "Octave", text);
    panel.octave = dropdown(ui, r, "octave", &octaves, panel.octave, None);
    label(ui, next(HEADER), "Notes & rhythm", ink.gamma_multiply(DIM));
    let r = next(CHOICE);
    label(ui, r, "Length", text);
    panel.duration = dropdown(ui, r, "length", &durations, panel.duration, None);
    let r = next(CHOICE);
    label(ui, r, "Rests", text);
    panel.rests = dropdown(ui, r, "rests", &rests, panel.rests, None);
    let r = next(CHOICE);
    label(ui, r, "Arpeggios", text);
    panel.arp = dropdown(ui, r, "arps", &arps, panel.arp, None);
    let r = next(CHOICE);
    label(ui, r, "Chords", text);
    panel.chords = dropdown(ui, r, "chords", &CHORD_MODES, panel.chords, None);
}
