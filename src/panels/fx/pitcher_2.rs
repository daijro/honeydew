//! Pitcher 2 (ID 45), UIHeight 8 U.

use eframe::egui::{Color32, Painter, Rect, Ui, pos2, vec2};
use honeydew_audio::analysis;

use super::leveller::animate;
use super::live_analysis;
use super::pitcher::{Trail, pitch_class};
use crate::panels::build::*;
use crate::panels::setting_item;
use crate::ui::{UiCtx, widgets};

/// `Scale` (#15) items, with their note masks (most significant bit C); `Custom` has none.
const SCALES: [&str; 6] = [
    "Minor",
    "Major",
    "Pentatonic",
    "Chromatic",
    "Single",
    "Custom",
];
const MASKS: [Option<u16>; 6] = [
    Some(2906),
    Some(2773),
    Some(2708),
    Some(4095),
    Some(2048),
    None,
];
/// `Custom`: picking it leaves the notes unchanged, and a note edit that fits no scale selects it.
const CUSTOM: usize = 5;
const KEY: u32 = 14;
const SCALE: u32 = 15;
const SPEED: u32 = 16;

/// The note buttons' colours: no fill, `text` labels, the off label at 25 % (Pitcher 2 and Scale).
const NOTE_COLORS: ButtonColors = ButtonColors {
    fill_on: Ink::Clear,
    fill_off: Ink::Clear,
    text_on: Ink::Text,
    text_off: None,
};

/// The twelve rect note buttons `C` … `B`, `#first`… bound to `p_first`… (Pitcher 2 and Scale). The module draws
/// its note bars under them.
pub(super) fn note_buttons(first: u32, p_first: usize) -> impl Iterator<Item = Part> {
    (0..12).map(move |i| {
        button(first + i as u32, NOTES[i], false)
            .p(p_first + i)
            .colors(NOTE_COLORS)
    })
}

/// The note bars' alpha (display step 2; the Scale's too).
const BAR_ALPHA: f32 = 0.2;

/// The note row: the union of the note buttons `#first`….
fn note_row(rects: &[(u32, Rect)], first: u32) -> Option<Rect> {
    (rects.iter())
        .filter(|(n, _)| (first..first + 12).contains(n))
        .map(|&(_, r)| r)
        .reduce(Rect::union)
}

/// Display step 2 (the Scale draws the same): note `i`'s bar (C = 0) on the note row `row` (points) at `ppp`. With W
/// The row width in device pixels: x = int(row.x + i·W/12) + W/60, 0.6·W/12 wide, the row's full height. A fractional
/// `i` is a pitch class between notes (step 4's marker).
fn note_bar(row: Rect, i: f32, ppp: f32) -> Rect {
    let (x, w) = (row.left() * ppp, row.width() * ppp);
    let left = (x + i * w / 12.0).trunc() + w / 60.0;
    Rect::from_x_y_ranges(left / ppp..=(left + 0.6 * w / 12.0) / ppp, row.y_range())
}

/// How long a detection stays live (display step 3), s.
const LIVE: f32 = 0.1;

/// The detected pitch as the display keeps it: the time since the last detection, the input's pitch class and the
/// target's then, and the motion trail.
#[derive(Clone, Debug, PartialEq)]
struct Pitch {
    since: f32,
    class: f32,
    target: Option<usize>,
    trail: Trail,
}

impl Default for Pitch {
    fn default() -> Self {
        Self {
            since: f32::INFINITY,
            class: 0.0,
            target: None,
            trail: Trail::default(),
        }
    }
}

impl Pitch {
    /// After `dt` s with the feed's detected and target frequencies (Hz, 0 for none), the marker for the trail at `x`
    /// of the input class on the note row.
    fn step(self, dt: f32, (hz, target): (f32, f32), x: impl Fn(f32) -> f32) -> Pitch {
        let (since, class, target) = match pitch_class(hz, 440.0) {
            Some(class) => {
                let note = 69.0 + 12.0 * (target / 440.0).log2();
                let target = (target > 0.0 && note.is_finite())
                    .then(|| (note.round() as i32).rem_euclid(12) as usize);
                (0.0, class, target)
            }
            None => (self.since + dt, self.class, self.target),
        };
        let live = since <= LIVE;
        Pitch {
            since,
            class,
            target,
            trail: self.trail.step(dt, live.then(|| x(class))),
        }
    }
}

/// Display steps 3–5 on the note row `row`: while the pitch is live, the top 5 % of the target note's bar in the
/// channel colour if that note is on, and only then the live marker, the input's class through the bar formula, as wide
/// as the highlight, in the bottom 5 % at 33 %; then the trail of the marker in the bottom 5 % at a·33 %.
fn pitch_marks(
    painter: &Painter,
    row: Rect,
    ppp: f32,
    notes: &[f32],
    pitch: &Pitch,
    channel: Color32,
) {
    let (h, px) = (row.height(), 1.0 / ppp);
    let on = |i: usize| notes.get(i).is_some_and(|&v| v > 0.5);
    let target = pitch.target.filter(|&i| pitch.since <= LIVE && on(i));
    if let Some(i) = target {
        let bar = note_bar(row, i as f32, ppp);
        painter.rect_filled(bar.with_max_y(row.top() + 0.05 * h), 0.0, channel);
        let marker = note_bar(row, pitch.class, ppp).with_min_y(row.top() + 0.95 * h);
        painter.rect_filled(marker, 0.0, channel.gamma_multiply(0.33));
    }
    let centre = row.top() + 0.975 * h;
    for e in &pitch.trail.entries {
        let half = e.a * 0.05 * h / 2.0;
        let quad = Rect::from_x_y_ranges(Trail::span(e, px), centre - half..=centre + half);
        painter.rect_filled(quad, 0.0, channel.gamma_multiply(e.a * 0.33));
    }
}

/// Display steps 1–2, drawn before the children so that the clear note buttons `#first`… draw their labels on top: the
/// note row filled `controlFill`, then per note a bar in the channel colour when on (`notes[i]` ≥ 0.5), else
/// `controlFill`, at 20 %. The Scale draws the same row, with the same off-bar colour.
pub(super) fn note_bars(
    ui: &Ui,
    cx: &UiCtx,
    rects: &[(u32, Rect)],
    first: u32,
    notes: &[f32],
    channel: Color32,
) {
    let Some(row) = note_row(rects, first) else {
        return;
    };
    let (painter, fill) = (ui.painter(), cx.theme.colors.control_fill.to_color32());
    painter.rect_filled(row, 0.0, fill);
    for i in 0..12 {
        let ink = match notes.get(i) {
            Some(&v) if v > 0.5 => channel,
            _ => fill,
        };
        let bar = note_bar(row, i as f32, cx.m.ppp());
        painter.rect_filled(bar, 0.0, ink.gamma_multiply(BAR_ALPHA));
    }
}

/// Display step 6: the box of the `Speed` labels on the slider rect `s`, (S.x + 0.025w, S.y + 0.3h, 0.95w, 0.75h).
fn speed_box(s: Rect) -> Rect {
    let (w, h) = (s.width(), s.height());
    Rect::from_min_size(s.min + vec2(0.025 * w, 0.3 * h), vec2(0.95 * w, 0.75 * h))
}

/// Display step 6: the `Speed` labels and where each hangs in [`speed_box`]: the top lines from its top edge, the
/// bottom lines from its bottom edge.
const SPEED_LABELS: [(&str, Align2); 6] = [
    ("Slow", Align2::LEFT_TOP),
    ("-100", Align2::LEFT_BOTTOM),
    ("Natural", Align2::CENTER_TOP),
    ("0", Align2::CENTER_BOTTOM),
    ("Fast", Align2::RIGHT_TOP),
    ("100", Align2::RIGHT_BOTTOM),
];

/// Display step 7: the six `Speed` ticks on the slider rect `s`, `t` wide: the centre pair at x = S.x + 0.5w − t/2
/// (y 0.45h and 0.75375h, 0.1125h tall), then the side pairs at x = S.x + 0.08675w and S.x + 0.91325w − t (y 0.50625h
/// and 0.75375h, 0.05625h tall).
fn speed_ticks(s: Rect, t: f32) -> [Rect; 6] {
    let (w, h) = (s.width(), s.height());
    let tick = |x: f32, y: f32, tall: f32| {
        Rect::from_min_size(pos2(s.left() + x, s.top() + y * h), vec2(t, tall * h))
    };
    let (mid, left, right) = (0.5 * w - t / 2.0, 0.08675 * w, 0.91325 * w - t);
    [
        tick(mid, 0.45, 0.1125),
        tick(mid, 0.75375, 0.1125),
        tick(left, 0.50625, 0.05625),
        tick(left, 0.75375, 0.05625),
        tick(right, 0.50625, 0.05625),
        tick(right, 0.75375, 0.05625),
    ]
}

/// The display, all of it before the children: the note bars on the note row #2–#13 (p1–p12), the detected pitch's
/// marks from the Pitcher 2's feed, then on TUNING the `Speed` labels and ticks on the slider #16, in the channel
/// colour.
fn display(
    ui: &Ui,
    cx: &UiCtx,
    module: &ModuleInstance,
    _: &PanelState,
    rects: &[(u32, Rect)],
    _: Rect,
    channel: Color32,
) {
    let notes = module.params.get(1..13).unwrap_or_default();
    note_bars(ui, cx, rects, 2, notes, channel);
    if let Some(row) = note_row(rects, 2) {
        let ppp = cx.m.ppp();
        let feed = live_analysis(ui, module.instance_id).and_then(|s| analysis::pitcher2(&s));
        let id = ui.id().with(("pitcher 2 pitch", module.instance_id));
        let x = |class: f32| note_bar(row, class, ppp).left();
        let pitch = animate(ui, id, Pitch::default(), |p, dt| {
            p.step(dt, feed.unwrap_or_default(), x)
        });
        if pitch.since <= LIVE || !pitch.trail.entries.is_empty() {
            ui.ctx().request_repaint();
        }
        pitch_marks(ui.painter(), row, ppp, notes, &pitch, channel);
    }
    let Some(&(_, s)) = rects.iter().find(|(n, _)| *n == SPEED) else {
        return;
    };
    for (text, align) in SPEED_LABELS {
        widgets::text_label(ui, cx, speed_box(s), text, FontSize::Pt8, align, channel);
    }
    for tick in speed_ticks(s, cx.m.t()) {
        ui.painter().rect_filled(tick, 0.0, channel);
    }
}

/// `mask` (12 bits, most significant C) transposed up `key` semitones: rotated right.
fn transpose(mask: u16, key: usize) -> u16 {
    let key = key % 12;
    ((mask >> key) | (mask << (12 - key))) & 0xFFF
}

/// `Key` or `Scale` pick (Pitcher 2 and the Scale): writes `masks[scale]` transposed to `key` to the 12 `notes` (C
/// first) and returns the offsets of the notes it changed. A scale without a mask (`Custom`) leaves them.
pub(super) fn write_scale(
    notes: &mut [f32],
    masks: &[Option<u16>],
    key: usize,
    scale: usize,
) -> Vec<usize> {
    let Some(mask) = masks.get(scale).copied().flatten() else {
        return Vec::new();
    };
    let mask = transpose(mask, key);
    let mut changed = Vec::new();
    for (i, v) in notes.iter_mut().enumerate() {
        let on = f32::from(u8::from(mask >> (11 - i) & 1 == 1));
        if *v != on {
            *v = on;
            changed.push(i);
        }
    }
    changed
}

/// After a note edit: the scale whose mask, transposed to `key`, the 12 `notes` are, or `custom`. No two masks of a
/// list are the same, so at most one fits.
pub(super) fn scale_of(notes: &[f32], masks: &[Option<u16>], key: usize, custom: usize) -> usize {
    let mask = notes.iter().fold(0, |m, &v| (m << 1) | u16::from(v > 0.5));
    let fits = |s: usize| {
        masks
            .get(s)
            .copied()
            .flatten()
            .is_some_and(|m| transpose(m, key) == mask)
    };
    (0..masks.len()).find(|&s| fits(s)).unwrap_or(custom)
}

/// The scale logic on the settings `Key` and `Scale` and the notes p1–p12. Only user edits drive it.
// ponytail: automation of p1–p12 does not re-select the Scale entry; read the notes every pass if that matters.
fn couple(module: &mut ModuleInstance, state: &mut PanelState, edited: &[u32]) -> Vec<usize> {
    let item = |n: u32| state.settings.get(&n).map_or(0, |&v| setting_item(v));
    let (key, scale) = (item(KEY), item(SCALE));
    let Some(notes) = module.params.get_mut(1..13) else {
        return Vec::new();
    };
    if edited.iter().any(|n| [KEY, SCALE].contains(n)) {
        let changed = write_scale(notes, &MASKS, key, scale);
        return changed.into_iter().map(|i| 1 + i).collect();
    }
    if edited.iter().any(|n| (2..=13).contains(n)) {
        let picked = scale_of(notes, &MASKS, key, CUSTOM);
        state.settings.insert(SCALE, picked as f32);
    }
    Vec::new()
}

/// A loaded module's `Scale` (not read from its blob): the scale its notes p1–p12 are in `Key`, else `Custom`.
pub(crate) fn derive_scale(module: &ModuleInstance, state: &mut PanelState) {
    let key = state.settings.get(&KEY).map_or(0, |&v| setting_item(v));
    if let Some(notes) = module.params.get(1..13) {
        let scale = scale_of(notes, &MASKS, key, CUSTOM);
        state.settings.insert(SCALE, scale as f32);
    }
}

// The note bars, the live pitch marks and the `Speed` labels and ticks are the underlay.
pub fn panel() -> Option<Panel> {
    let panel = Panel::new(vstack([
        tabs(1, "TUNING · EXPRESSION · OPTIONS").h(Size::U(1.25)),
        hstack(note_buttons(2, 1))
            .h(Size::U(1.5))
            .margins(0.4, 0.0, 0.2, 0.0),
        hstack([
            spinner(KEY, "Key").items(&NOTES),
            spinner(SCALE, "Scale").items(&SCALES),
            h_slider(SPEED, "Speed").p(13).w(Size::Flex(2.0)),
            toggle(17, "Formants").p(16),
            knob(18, "F-Shift").p(17),
        ])
        .show(Show::Tab(0)),
        hstack([
            knob(19, "Humanization").p(20),
            knob(20, "Sensitivity").p(23),
            knob(21, "Stability").p(19),
            knob(22, "Vibrato").p(24),
            toggle(23, "Process Sibilants").p(26),
        ])
        .show(Show::Tab(1)),
        hstack([
            knob(24, "Mix").p(22),
            spinner(25, "Pitch Ref.").p(18),
            toggle(26, "Low Latency").p(25),
        ])
        .show(Show::Tab(2)),
    ]));
    Some(panel.underlay(display).hook(couple))
}

/// The note names, C first: the note row's labels and the `Key` items (Pitcher 2 and Scale).
pub(super) const NOTES: [&str; 12] = [
    "C", "C#", "D", "D#", "E", "F", "F#", "G", "G#", "A", "A#", "B",
];

/// Where the twelve note buttons of a 14 U row sit, as the tables print them (Pitcher 2 and Scale).
#[cfg(test)]
pub(super) const NOTE_X: [f32; 12] = [
    0.0, 1.16, 2.33, 3.5, 4.66, 5.83, 7.0, 8.16, 9.33, 10.5, 11.66, 12.83,
];
#[cfg(test)]
pub(super) const NOTE_W: [f32; 12] = [
    1.16, 1.17, 1.17, 1.16, 1.17, 1.17, 1.16, 1.17, 1.17, 1.16, 1.17, 1.16,
];

/// The note bars `rig`'s last frame drew under the note buttons `#first`…, as their fills, C first. Checks that the
/// row fill (`controlFill`) comes first, that each bar lies where [`note_bar`] puts it, and that all of them come
/// before the first note label.
#[cfg(test)]
pub(super) fn drawn_bars(rig: &crate::panels::tests::Rig, first: u32) -> Vec<Color32> {
    use eframe::egui::Shape;
    let row = (first..first + 12)
        .map(|n| rig.rect_of(n))
        .reduce(Rect::union)
        .unwrap();
    let at = |rect: Rect| {
        let found = rig
            .shapes
            .iter()
            .enumerate()
            .find_map(|(k, c)| match &c.shape {
                Shape::Rect(r) if r.rect == rect => Some((k, r.fill)),
                _ => None,
            });
        found.unwrap_or_else(|| panic!("nothing drawn at {rect:?}"))
    };
    let label = rig
        .shapes
        .iter()
        .position(|c| matches!(&c.shape, Shape::Text(t) if t.galley.text() == "C"));
    let fill = at(row);
    let control_fill = crate::theme::theme_by_id(0)
        .colors
        .control_fill
        .to_color32();
    assert_eq!(fill.1, control_fill, "the row fill");
    let bars: Vec<_> = (0..12).map(|i| at(note_bar(row, i as f32, 1.0))).collect();
    let label = label.expect("the C label");
    assert!(
        bars.iter().all(|&(k, _)| fill.0 < k && k < label),
        "{fill:?} {bars:?} C at {label}"
    );
    bars.into_iter().map(|(_, fill)| fill).collect()
}

#[cfg(test)]
const TABS: Row = row(
    1,
    Widget::Tabs,
    "TUNING · EXPRESSION · OPTIONS",
    [0.0, 0.0, 14.0, 1.25],
)
.h(Size::U(1.25));

/// Note button `i` (C = 0), `#2`… on p1…: no fill, `text` labels only, the off label at 25 %.
#[cfg(test)]
const fn note(i: usize) -> Row {
    let rect = [NOTE_X[i], 1.65, NOTE_W[i], 1.5];
    row(
        2 + i as u32,
        Widget::Button { round: false },
        NOTES[i],
        rect,
    )
    .p(1 + i)
    .colors(ButtonColors {
        fill_on: Ink::Clear,
        fill_off: Ink::Clear,
        text_on: Ink::Text,
        text_off: None,
    })
}

/// The tab strip and the note row, which every page shows, then the page's rows.
#[cfg(test)]
macro_rules! rows {
    ($($row:expr),* $(,)?) => {
        &[
            TABS, note(0), note(1), note(2), note(3), note(4), note(5), note(6), note(7), note(8), note(9),
            note(10), note(11), $($row),*
        ]
    };
}

#[cfg(test)]
pub const REFERENCE: Reference = Reference {
    size: [14.0, 5.99],
    settings: &[(14, 0.0), (15, 0.0)],
    cases: &[
        Case {
            name: "TUNING",
            rows: rows![
                row(14, Widget::Spinner, "Key", [0.0, 3.35, 2.33, 2.64])
                    .items("C · C# · D · D# · E · F · F# · G · G# · A · A# · B"),
                row(15, Widget::Spinner, "Scale", [2.33, 3.35, 2.33, 2.64])
                    .items("Minor · Major · Pentatonic · Chromatic · Single · Custom"),
                row(16, Widget::HSlider, "Speed", [4.66, 3.35, 4.67, 2.64])
                    .p(13)
                    .w(Size::Flex(2.0)),
                row(17, Widget::Toggle, "Formants", [9.33, 3.35, 2.33, 2.64]).p(16),
                row(18, Widget::Knob, "F-Shift", [11.66, 3.35, 2.34, 2.64]).p(17),
            ],
            ..Case::DEFAULT
        },
        Case {
            name: "EXPRESSION",
            tab: 1,
            rows: rows![
                row(19, Widget::Knob, "Humanization", [0.0, 3.35, 2.8, 2.64]).p(20),
                row(20, Widget::Knob, "Sensitivity", [2.8, 3.35, 2.8, 2.64]).p(23),
                row(21, Widget::Knob, "Stability", [5.6, 3.35, 2.8, 2.64]).p(19),
                row(22, Widget::Knob, "Vibrato", [8.4, 3.35, 2.8, 2.64]).p(24),
                row(
                    23,
                    Widget::Toggle,
                    "Process Sibilants",
                    [11.2, 3.35, 2.8, 2.64]
                )
                .p(26),
            ],
            ..Case::DEFAULT
        },
        Case {
            name: "OPTIONS",
            tab: 2,
            rows: rows![
                row(24, Widget::Knob, "Mix", [0.0, 3.35, 4.66, 2.64]).p(22),
                row(25, Widget::Spinner, "Pitch Ref.", [4.66, 3.35, 4.67, 2.64]).p(18),
                row(26, Widget::Toggle, "Low Latency", [9.33, 3.35, 4.67, 2.64]).p(25),
            ],
            ..Case::DEFAULT
        },
    ],
};

#[cfg(test)]
mod tests {

    #[test]
    fn a_note_at_exactly_half_is_off_as_in_the_engine() {
        // Two-state law: on at > 0.5, as note_effects.rs plays it.
        assert_eq!(scale_of(&[0.5; 12], &[Some(0)], 0, 9), 0);
        assert_eq!(scale_of(&[0.51; 12], &[Some(0)], 0, 9), 9);
    }
    use eframe::egui::Vec2;

    use super::*;
    use crate::panels::tests::{Rig, U, card};

    /// The hook the panel runs, which the tests go through: a panel without it fails them.
    fn couple(m: &mut ModuleInstance, s: &mut PanelState, edited: &[u32]) -> Vec<usize> {
        (panel().unwrap().hook.expect("a hook"))(m, s, edited)
    }

    /// The notes p1–p12 that are on, as note names.
    fn on(m: &ModuleInstance) -> Vec<&str> {
        (0..12)
            .filter(|&i| m.params[1 + i] > 0.5)
            .map(|i| NOTES[i])
            .collect()
    }

    /// The reference note row (#2–#13 span 0, 1.65, 14, 1.5 U) at U = 32 points.
    fn reference_row() -> Rect {
        Rect::from_min_size(pos2(0.0, 1.65 * U), vec2(14.0 * U, 1.5 * U))
    }

    fn near(a: Vec2, b: Vec2) -> bool {
        (a - b).length() < 1e-3
    }

    #[test]
    fn a_note_bar_starts_at_int_of_its_column_plus_w_60_and_is_0_6_column_wide() {
        // ppp 1: W = 448 px, columns at i·37.33 px. ppp 2: W = 896 px, columns at i·74.67 px. The bar is W/20 wide.
        let int1 = [0, 37, 74, 112, 149, 186, 224, 261, 298, 336, 373, 410];
        let int2 = [0, 74, 149, 224, 298, 373, 448, 522, 597, 672, 746, 821];
        let row = reference_row();
        for (ppp, ints) in [(1.0, int1), (2.0, int2)] {
            let w = 448.0 * ppp;
            for (i, int) in ints.into_iter().enumerate() {
                let bar = note_bar(row, i as f32, ppp);
                let x = (int as f32 + w / 60.0) / ppp;
                assert!(
                    near(bar.min.to_vec2(), vec2(x, 1.65 * U)),
                    "ppp {ppp}, {i}: {bar:?}"
                );
                assert!(
                    near(bar.size(), vec2(22.4, 1.5 * U)),
                    "ppp {ppp}, {i}: {bar:?}"
                );
            }
        }
        // Off the origin, int() takes row.x + i·W/12.
        let bar = note_bar(row.translate(vec2(10.0, 0.0)), 11.0, 1.0);
        assert!(
            (bar.left() - (10.0 + 410.0 + 448.0 / 60.0)).abs() < 1e-3,
            "{bar:?}"
        );
    }

    #[test]
    fn a_detection_stays_live_for_a_tenth_of_a_second() {
        let x = |class: f32| 10.0 * class;
        // 220 Hz (A, 9) snapping to 207.65 Hz (G#, 8).
        let p = Pitch::default().step(0.0, (220.0, 207.6523), x);
        assert_eq!((p.since, p.class, p.target), (0.0, 9.0, Some(8)));
        assert_eq!(p.trail.entries.len(), 1);
        // Nothing detected: still live 0.05 s on, the class kept; not 0.2 s on, and the trail adds nothing.
        let p = p.step(0.05, (0.0, 0.0), x);
        assert!(p.since <= LIVE && p.class == 9.0 && p.target == Some(8));
        let p = p.step(0.15, (0.0, 0.0), x);
        assert!(p.since > LIVE);
        assert_eq!(p.trail.entries.len(), 1);
    }

    #[test]
    fn a_live_pitch_highlights_its_target_and_marks_the_input_below() {
        let row = reference_row();
        let ppp = 2.0;
        let notes: Vec<f32> = (0..12).map(|i| f32::from(u8::from(i == 8))).collect();
        let pitch = Pitch::default().step(0.0, (220.0, 207.6523), |c| note_bar(row, c, ppp).left());
        let shapes = |notes: &[f32]| {
            let out = eframe::egui::Context::default().run(Default::default(), |ctx| {
                let ui = Ui::new(
                    ctx.clone(),
                    eframe::egui::Id::new("row"),
                    Default::default(),
                );
                pitch_marks(ui.painter(), row, ppp, notes, &pitch, Color32::RED);
            });
            (out.shapes.into_iter())
                .map(|c| match c.shape {
                    eframe::egui::Shape::Rect(r) => (r.rect, r.fill),
                    other => panic!("{other:?}"),
                })
                .collect::<Vec<_>>()
        };
        let got = shapes(&notes);
        let h = row.height();
        let bar = note_bar(row, 8.0, ppp);
        assert_eq!(got[0], (bar.with_max_y(row.top() + 0.05 * h), Color32::RED));
        let marker = note_bar(row, 9.0, ppp).with_min_y(row.top() + 0.95 * h);
        assert_eq!(got[1], (marker, Color32::RED.gamma_multiply(0.33)));
        // The trail's first entry, 1 px either side of the marker's left edge, a full 5 % tall at 33 %.
        let (quad, fill) = got[2];
        assert!((quad.width() - 2.0 / ppp).abs() < 1e-4 && (quad.height() - 0.05 * h).abs() < 1e-3);
        assert_eq!(fill, Color32::RED.gamma_multiply(0.33));
        // With G# off there is no highlight and so no marker: only the trail.
        assert_eq!(shapes(&[0.0; 12]).len(), 1);
    }

    #[test]
    fn the_speed_labels_hang_from_their_box_edges() {
        // S at the reference (4.66, 3.35, 4.67, 2.64 U): the box (4.77675, 4.142, 4.4365, 1.98 U).
        let s = Rect::from_min_size(pos2(4.66 * U, 3.35 * U), vec2(4.67 * U, 2.64 * U));
        let b = speed_box(s);
        assert!(near(b.min.to_vec2(), vec2(4.77675, 4.142) * U), "{b:?}");
        assert!(near(b.size(), vec2(4.4365, 1.98) * U), "{b:?}");
        // Each line's anchor: left 4.77675, centre 6.995, right 9.21325 U; top 4.142, bottom 6.122 U.
        let anchors = [
            ("Slow", 4.77675, 4.142),
            ("-100", 4.77675, 6.122),
            ("Natural", 6.995, 4.142),
            ("0", 6.995, 6.122),
            ("Fast", 9.21325, 4.142),
            ("100", 9.21325, 6.122),
        ];
        assert_eq!(SPEED_LABELS.len(), anchors.len());
        for (text, x, y) in anchors {
            let (_, align) = SPEED_LABELS.iter().find(|(t, _)| *t == text).expect(text);
            let at = align.pos_in_rect(&b);
            assert!(near(at.to_vec2(), vec2(x, y) * U), "{text} at {at:?}");
        }
    }

    #[test]
    fn the_speed_ticks_are_t_wide_at_the_spec_fractions_of_the_slider() {
        // S at the reference, t = 2 px = 0.0625 U (U = 32 px).
        let s = Rect::from_min_size(pos2(4.66 * U, 3.35 * U), vec2(4.67 * U, 2.64 * U));
        let (mid, left, right) = (6.96375, 5.0651225, 8.862_377);
        let (hi, lo, side_hi) = (4.538, 5.3399, 4.6865);
        let (tall, short) = (0.297, 0.1485);
        let want = [
            (mid, hi, tall),
            (mid, lo, tall),
            (left, side_hi, short),
            (left, lo, short),
            (right, side_hi, short),
            (right, lo, short),
        ];
        for (tick, (x, y, h)) in speed_ticks(s, 2.0).into_iter().zip(want) {
            assert!(
                near(tick.min.to_vec2(), vec2(x, y) * U),
                "{tick:?} at {x}, {y}"
            );
            assert!(near(tick.size(), vec2(2.0, h * U)), "{tick:?}: {h}");
        }
    }

    #[test]
    fn the_note_bars_follow_p1_to_p12_under_the_note_buttons() {
        let control_fill = crate::theme::theme_by_id(0)
            .colors
            .control_fill
            .to_color32();
        let (on, off) = (
            Color32::RED.gamma_multiply(0.2),
            control_fill.gamma_multiply(0.2),
        );
        let mut rig = Rig::new(45, panel().unwrap(), card(8.0).1);
        // C minor by default: C D D# F G G# A#.
        let minor = [on, off, on, on, off, on, off, on, on, off, on, off];
        assert_eq!(drawn_bars(&rig, 2), minor);
        // Notes follow their params, not the Scale setting: C# on, C off.
        rig.module.params[1] = 0.0;
        rig.module.params[2] = 1.0;
        rig.frame(vec![]);
        let mut moved = minor;
        moved.swap(0, 1);
        assert_eq!(drawn_bars(&rig, 2), moved);
    }

    #[test]
    fn the_speed_labels_and_ticks_frame_the_slider_on_tuning_only() {
        let mut rig = Rig::new(45, panel().unwrap(), card(8.0).1);
        let s = rig.rect_of(SPEED);
        let m = crate::metrics::Metrics::from_pixels_per_point(1.0, 100);
        let (b, size) = (speed_box(s), m.font_id(FontSize::Pt8).size);
        for (text, align) in SPEED_LABELS {
            let shape = rig
                .text_shape(text)
                .unwrap_or_else(|| panic!("{text} not drawn"));
            let format = &shape.galley.job.sections[0].format;
            assert_eq!(
                (format.color, format.font_id.size),
                (Color32::RED, size),
                "{text}"
            );
            let at = align.pos_in_rect(&rig.text_rect(text));
            assert!(
                (at - align.pos_in_rect(&b)).length() < 1.0,
                "{text} at {at:?} in {b:?}"
            );
        }
        let ticks = speed_ticks(s, 2.0);
        let drawn = |rig: &Rig, tick: Rect| {
            rig.shapes.iter().any(|c| {
                matches!(&c.shape, eframe::egui::Shape::Rect(r) if r.rect == tick && r.fill == Color32::RED)
            })
        };
        assert!(ticks.iter().all(|&t| drawn(&rig, t)));
        for tab in [1, 2] {
            rig.state.tab = tab;
            rig.frame(vec![]);
            assert!(
                SPEED_LABELS
                    .iter()
                    .all(|(text, _)| !rig.drawn(text) || *text == "0"),
                "tab {tab}"
            );
            // Nor anything at a label's anchor: `0` is too common to look for by name.
            for (_, align) in SPEED_LABELS {
                let anchor = align.pos_in_rect(&b);
                let there = rig
                    .texts()
                    .into_iter()
                    .find(|(_, r)| (align.pos_in_rect(r) - anchor).length() < 1.0);
                assert_eq!(there, None, "tab {tab}");
            }
            assert!(ticks.iter().all(|&t| !drawn(&rig, t)), "tab {tab}");
            assert_eq!(drawn_bars(&rig, 2).len(), 12, "the bars on every tab");
        }
    }

    #[test]
    fn key_and_scale_write_the_notes_and_a_note_edit_picks_the_scale() {
        // At most one scale fits a set of notes.
        let masks: Vec<u16> = MASKS.iter().flatten().copied().collect();
        assert!(
            masks
                .iter()
                .enumerate()
                .all(|(i, m)| !masks[..i].contains(m))
        );
        let mut m = ModuleInstance::new(45, 2);
        let mut state = PanelState::new(&panel().unwrap());
        assert_eq!(on(&m), ["C", "D", "D#", "F", "G", "G#", "A#"], "C minor");
        // D major: the mask rotated right by 2, transposed up.
        state.settings.insert(KEY, 2.0);
        state.settings.insert(SCALE, 1.0);
        let changed = couple(&mut m, &mut state, &[SCALE]);
        assert_eq!(on(&m), ["C#", "D", "E", "F#", "G", "A", "B"]);
        assert_eq!(changed, [1, 2, 4, 5, 6, 7, 9, 10, 11, 12]);
        // Nothing edited: nothing moves.
        assert!(couple(&mut m, &mut state, &[]).is_empty());
        // A Key edit transposes the scale: G major.
        state.settings.insert(KEY, 7.0);
        assert_eq!(couple(&mut m, &mut state, &[KEY]), [1, 2]);
        assert_eq!(on(&m), ["C", "D", "E", "F#", "G", "A", "B"]);
        // A note edit (D tapped off and on again) on notes that fit a scale selects it: in E, G major's notes are E
        // minor.
        state.settings.insert(KEY, 4.0);
        assert!(couple(&mut m, &mut state, &[4]).is_empty());
        assert_eq!(state.settings[&SCALE], 0.0, "E minor");
        // C# on as well: eight notes fit no scale, so `Custom`; then a Key edit leaves the notes.
        m.params[2] = 1.0;
        couple(&mut m, &mut state, &[3]);
        assert_eq!(state.settings[&SCALE], CUSTOM as f32);
        let before = m.params.clone();
        assert!(couple(&mut m, &mut state, &[KEY]).is_empty());
        assert_eq!(m.params, before);
    }

    /// `Custom` can be picked and leaves the notes unchanged: the `v` chevron from `Single` goes there, and the notes
    /// stay.
    #[test]
    fn custom_can_be_picked_and_leaves_the_notes() {
        use crate::metrics::{DEFAULT_UI_SCALE, Metrics};
        use crate::panels::{place, show};
        use crate::theme::theme_by_id;
        use crate::ui::UiCtx;
        use eframe::egui::{
            CentralPanel, Color32, Context, Event, PointerButton, RawInput, Rect, pos2, vec2,
        };

        let panel = panel().unwrap();
        let mut m = ModuleInstance::new(45, 2);
        let mut state = PanelState::new(&panel);
        state.settings.insert(SCALE, 4.0);
        let before = m.params.clone();
        let (theme, metrics) = (
            theme_by_id(0),
            Metrics::from_pixels_per_point(1.0, DEFAULT_UI_SCALE),
        );
        let cx = UiCtx {
            theme: &theme,
            m: &metrics,
        };
        let u = metrics.u(1.0);
        let content = Rect::from_min_size(pos2(0.0, 0.0), vec2(14.0 * u, 5.99 * u));
        let placed = place(&panel, &m, &state, content, u);
        let r = placed.iter().find(|(l, _)| l.id == SCALE).unwrap().1;
        // The `v` chevron: 0.85 U below the item text, which is centred under the 0.75 U caption.
        let down = pos2(
            r.center().x,
            (r.top() + 0.75 * u + r.bottom()) / 2.0 + 0.85 * u,
        );
        let ctx = Context::default();
        let screen = Rect::from_min_size(pos2(0.0, 0.0), vec2(800.0, 600.0));
        let press = |pressed| Event::PointerButton {
            pos: down,
            button: PointerButton::Primary,
            pressed,
            modifiers: Default::default(),
        };
        for events in [
            vec![Event::PointerMoved(down)],
            vec![press(true)],
            vec![press(false)],
        ] {
            let raw = RawInput {
                screen_rect: Some(screen),
                events,
                ..Default::default()
            };
            let _ = ctx.run(raw, |ctx| {
                CentralPanel::default().show(ctx, |ui| {
                    show(ui, &cx, &panel, &mut m, &mut state, content, Color32::RED);
                });
            });
        }
        assert_eq!(state.settings[&SCALE], CUSTOM as f32);
        assert_eq!(m.params, before);
    }
}
