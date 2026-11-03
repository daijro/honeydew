//! The keyboard settings strip, opened by the transport's icon-48 button. In order: `Reset`, the `Project Key` and
//! `Project Scale` spinners, then for a drum channel the `Lock` padlock, and for a notes channel the `Keyboard` spinner
//! and the wheels toggle (icon 12), then in `Chords` and `Strum` the chord controls (`ChordSet` ≡, `Octave`,
//! `Velocity`, and in `Chords` only `Timing` and `Hold`), in `Classic` and `Scale` the two-rows toggle (icon 13).
//! Which parts show depends on the channel and kind (see `Shown`). The quick scale buttons (`Major`, `Harmonic`,
//! `Melodic`, `Minor`) do not exist.
//!
//! The strip sits in the bottom keyboard panel under the transport bar, placed by [`strip_rect`]; the keys below it get
//! [`keys_rect`]. It is 3 U tall with one row of controls and 6.25 U with two, two when 2 U a control do not fit the
//! width ([`strip_height`]); the controls sit in equal cells at most 5 U wide, centred ([`layout`]).
//!
//! - The strip takes the rect it is given, filled with `moduleFill` with a 1 px `background` border inside.
//! - `Reset` puts every value of the panel back to its default (`C`, `Chromatic`, `Classic`, both toggles off): so two
//!   rows go off with the wheels and key-lock. It resets the chords too: the default set, `Octave` 0, `Timing` 0,
//!   `Velocity` 0.5, `Hold` off; and the chord editor closes. The pads' lock stays.
//! - Icon 12 (two faders) toggles the pitch and mod wheels, icon 13 (two bars) the second keyboard row and the padlock
//!   (icon 39) the drum pads' lock: the settings `showWheels`, `showTwoRows` and `lockpads`. A button is lit (a `text`
//!   disc under a `moduleFill` icon) while its toggle is on.
//! - The `Keyboard` spinner picks any of the four kinds. `Chords` and `Strum` put the chord keyboard
//!   ([`crate::ui::chord_keyboard`]) in the piano's place; `ChordSet` ≡ `Open Editor` opens the chord editor
//!   ([`crate::ui::chord_editor`]) under its strips; only `Close Editor` and Reset close it.
//! - The panel's scale is its place in [`SCALES`], the picker read column by column; the chevrons step in that order.
//!   The song file's key-lock scale id follows its own stored list instead (see [`scale_from_stored`]).
//! - `Reset` is a 1.5 U outline circle with a 6 pt label and the icon buttons 1.25 U ones, each centred in its rect;
//!   spinners have a 0.75 U caption strip (8 pt) over their body. Everything is in `text`.

use eframe::egui::{Align2, Color32, Id, Pos2, Rect, Sense, Stroke, StrokeKind, Ui, pos2, vec2};

use crate::icons::{IconId, draw_icon};
use crate::metrics::{FontSize, Metrics};
use crate::ui::UiCtx;
use crate::ui::buttons::filled_circle_button;
use crate::ui::chord_keyboard::TIMING_MS;
use crate::ui::popup;
use crate::ui::popup::PopupItem;
use crate::ui::widgets::{Control, ParamMenu, knob, spinner_grid, spinner_index};

/// The scale list (the picker's order) and the stored-id mapping live with the song's key lock.
pub use crate::song::scales::{CHROMATIC, SCALES, scale_from_stored, scale_to_stored};

// Sizes in U (see above).
const RESET_D: f32 = 1.5;
const ICON_D: f32 = 1.25;
const ICON_SIZE: f32 = 0.7;
/// A spinner's caption, and the `ChordSet` box's.
const CAPTION_H: f32 = 0.75;
/// The divider between two rows of controls: `text` at this alpha.
const DIVIDER_ALPHA: f32 = 0.18;
/// The least chord velocity.
const MIN_VELOCITY: f32 = 1.0 / 127.0;

/// The strip's height in U with one row of controls, and with two (0.125 U over the rows, 2.5 U a row, 0.75 U between
/// them, 0.375 U under them).
pub const STRIP_H: f32 = 3.0;
pub const STRIP_H2: f32 = 6.25;
const ROW_TOP: f32 = 0.125;
const ROW_H: f32 = 2.5;
const ROW_GAP: f32 = 0.75;
/// A control's least width in a one-row strip, and a cell's greatest width, in U.
const CELL_MIN: f32 = 2.0;
const CELL_MAX: f32 = 5.0;
/// How far the pads and the sample row sit below the keys' top, in U.
const PADS_Y: f32 = 0.35;
/// A pitch or mod wheel's width, in U.
const WHEEL_W: f32 = 2.0;
/// The room under the wheels, and their greatest height, in U.
const WHEEL_ROOM: f32 = 0.5;
const WHEEL_MAX: f32 = 5.0;
/// The keyboard panel's default preferred height (`preferredKbdSizeCells`), in U.
pub const PANEL_H: f32 = 5.0;

/// The strip's height in U for `n` controls on a panel `w` points wide: one row while 2 U each fit, else two.
pub fn strip_height(n: usize, w: f32, m: &Metrics) -> f32 {
    if m.u(CELL_MIN) * n as f32 <= w {
        STRIP_H
    } else {
        STRIP_H2
    }
}

/// The settings strip under the transport `bar`, across it, `h` U tall (0 while closed). It does not slide.
pub fn strip_rect(bar: Rect, h: f32, m: &Metrics) -> Rect {
    Rect::from_x_y_ranges(bar.x_range(), bar.bottom()..=bar.bottom() + m.u(h))
}

/// The keys' area of the bottom keyboard `panel`: under the bar and the `strip`. Its height K has no minimum: on a panel
/// shorter than the bar and the open strip it is negative, and the content is laid out below the panel's bottom. Only
/// K = 0 hides the content.
pub fn keys_rect(panel: Rect, strip: Rect) -> Rect {
    panel.with_min_y(strip.bottom())
}

/// The drum pads' and the sample row's area in the keys area `keys`: 0.35 U lower, and as much shorter. The chord
/// keyboard takes `keys` itself.
pub fn pads_rect(keys: Rect, m: &Metrics) -> Rect {
    keys.with_min_y(keys.top() + m.u(PADS_Y))
}

/// The piano's keys rect R in the keys area `keys`: 4 U narrower on the left while the wheels show.
pub fn piano_rect(keys: Rect, wheels: bool, m: &Metrics) -> Rect {
    let x = if wheels { m.u(2.0 * WHEEL_W) } else { 0.0 };
    keys.with_min_x(keys.left() + x)
}

/// The `MOD` and `PITCH` wheels, left to right from the keys area's left edge, each [`WHEEL_W`] wide, from the keys'
/// top (under an open strip): min(K − 0.5 U, 5 U) tall, centred in K − 0.5 U when capped.
pub fn wheel_rects(keys: Rect, m: &Metrics) -> [Rect; 2] {
    let room = keys.height() - m.u(WHEEL_ROOM);
    let h = room.min(m.u(WHEEL_MAX));
    let top = keys.top() + (room - h) / 2.0;
    [0.0, WHEEL_W]
        .map(|x| Rect::from_min_size(pos2(keys.left() + m.u(x), top), vec2(m.u(WHEEL_W), h)))
}

/// The keyboard panel's height for a preferred height of `preferred` U on a screen `screen_h` points tall:
/// trunc(preferred · U) px, within the collapsed height `c` … the screen's height.
pub fn panel_target(preferred: f32, screen_h: f32, c: f32, m: &Metrics) -> f32 {
    ((preferred * m.u_px()).trunc() / m.ppp()).clamp(c, screen_h.max(c))
}

/// The panel's height `h` (points) one frame on toward `target`, going `k` of the way ([`crate::ui::anim::step`] at
/// [`crate::ui::anim::PANEL_KEEP`]: half the way a frame at 60 fps), snapping to the target within 2 px.
pub fn panel_tick(h: f32, target: f32, k: f32, m: &Metrics) -> f32 {
    let h = h + k * (target - h);
    if ((target - h) * m.ppp()).abs() < 2.0 {
        target
    } else {
        h
    }
}

/// The `Project Key` picker, one column.
pub const KEYS: [&str; 12] = [
    "C", "C# / Db", "D", "D# / Eb", "E", "F", "F# / Gb", "G", "G# / Ab", "A", "A# / Bb", "B",
];

/// The `Project Scale` picker's column headers.
const SCALE_HEADERS: [&str; 3] = ["Major", "Minor", "Other"];
/// How many scales each column holds.
const SCALE_COLUMNS: [usize; 3] = [9, 11, 13];

/// The `Keyboard` choices.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum KeyboardMode {
    #[default]
    Classic,
    Scale,
    Chords,
    Strum,
}

impl KeyboardMode {
    pub const ALL: [Self; 4] = [Self::Classic, Self::Scale, Self::Chords, Self::Strum];
    const LABELS: [&str; 4] = ["Classic", "Scale", "Chords", "Strum"];
}

/// The chord keyboard's values on the strip (`Chords` and `Strum`): the song's `KEYB`, which the view sets every frame.
#[derive(Clone, Debug, PartialEq)]
pub struct ChordControls {
    /// The `ChordSet` button's text: `Default: <set>`, `<set>` or `* <set>`.
    pub set_label: String,
    /// The `Octave` shift, −2 … +2.
    pub octave: i32,
    /// `Velocity`, [1/127, 1].
    pub velocity: f32,
    /// `Timing`, in ms (−40 … +40; the knob's v is (ms/40 + 1)/2).
    pub timing_ms: f32,
    pub hold: bool,
}

impl Default for ChordControls {
    fn default() -> Self {
        Self {
            set_label: "Default: Major".into(),
            octave: 0,
            velocity: 0.5,
            timing_ms: 0.0,
            hold: false,
        }
    }
}

/// The `ChordSet` ≡ items.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum SetItem {
    Default,
    Load,
    Save,
    /// `Open Editor`, or `Close Editor` while it is open.
    Editor,
}

/// The `Octave` items: the shift is the index − 2.
pub const OCTAVES: [&str; 5] = ["-2", "-1", "0", "+1", "+2"];

/// The panel's values.
#[derive(Clone, Debug, PartialEq)]
pub struct KeyboardSettings {
    /// Index into [`KEYS`].
    pub key: usize,
    /// The picker's display index: an index into [`SCALES`], the picker read column by column. It is not the id the
    /// song file stores; [`scale_from_stored`] and [`scale_to_stored`] convert.
    pub scale: usize,
    pub mode: KeyboardMode,
    /// Icon 12: the pitch and mod wheels are shown.
    pub wheels: bool,
    /// Icon 13: the keyboard has two rows.
    pub two_rows: bool,
    /// The current channel's first lane is a notes lane (or it has none): `Keyboard`, icon 12 and the chord controls
    /// show only then. The view sets it every frame.
    pub notes: bool,
    /// The current channel's first lane is a drum lane: `Lock` shows. The view sets it every frame.
    pub drums: bool,
    /// `Lock`: the drum pads cannot be rearranged (`lockpads`).
    pub lock: bool,
    /// The chord keyboard's values.
    pub chords: ChordControls,
    /// The chord editor is open (`ChordSet` ≡ `Open Editor`).
    pub editor: bool,
    /// The `ChordSet` ≡ popup is open, centred here.
    set_menu: Option<Pos2>,
}

impl Default for KeyboardSettings {
    fn default() -> Self {
        Self {
            key: 0,
            scale: CHROMATIC,
            mode: KeyboardMode::Classic,
            wheels: false,
            two_rows: false,
            notes: true,
            drums: false,
            lock: true,
            chords: ChordControls::default(),
            editor: false,
            set_menu: None,
        }
    }
}

impl KeyboardMode {
    /// `Chords` or `Strum`: the chord keyboard.
    pub fn chords(self) -> bool {
        matches!(self, Self::Chords | Self::Strum)
    }
}

/// What the user changed this frame; the panel's state already holds it.
#[derive(Clone, Copy, Debug, PartialEq)]
pub enum KeyboardSettingsAction {
    /// Every value went back to its default.
    Reset,
    Key(usize),
    Scale(usize),
    Mode(KeyboardMode),
    Wheels(bool),
    TwoRows(bool),
    /// `Lock`: the pads' lock.
    Lock(bool),
    /// The `Octave` shift, −2 … +2.
    Octave(i32),
    Velocity(f32),
    /// `Timing`, in ms.
    Timing(f32),
    Hold(bool),
    /// A `ChordSet` ≡ item.
    ChordSet(SetItem),
}

/// The `Project Scale` popup: three columns, each a dimmed header at row 0 and its scales from row 1; each item
/// selects its scale id, and `current` is flagged.
fn scale_popup(current: usize) -> Vec<PopupItem<usize>> {
    let mut items = Vec::with_capacity(SCALE_HEADERS.len() + SCALES.len());
    let mut id = 0;
    for (col, (header, len)) in SCALE_HEADERS.into_iter().zip(SCALE_COLUMNS).enumerate() {
        items.push(PopupItem {
            disabled: true,
            ..PopupItem::header(col, 0, header)
        });
        for row in 1..=len {
            items.push(PopupItem {
                current: id == current,
                ..PopupItem::new(id, col, row, SCALES[id])
            });
            id += 1;
        }
    }
    items
}

/// Which parts of the strip show: `Reset`, `Project Key` and `Project Scale` always; `Lock` for a drum channel;
/// `Keyboard` and icon 12 for a notes channel; icon 13 for one not in `Chords` or `Strum`; `ChordSet`, `Octave` and
/// `Velocity` for one in them; `Timing` and `Hold` in `Chords` only.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
struct Shown {
    lock: bool,
    mode: bool,
    wheels: bool,
    two_rows: bool,
    chords: bool,
    timing: bool,
}

impl Shown {
    fn of(notes: bool, drums: bool, mode: KeyboardMode) -> Self {
        Self {
            lock: drums,
            mode: notes,
            wheels: notes,
            two_rows: notes && !mode.chords(),
            chords: notes && mode.chords(),
            timing: notes && mode == KeyboardMode::Chords,
        }
    }

    /// The parts shown, in the strip's order.
    fn parts(self) -> Vec<Part> {
        use Part::*;
        [
            (Reset, true),
            (Key, true),
            (Scale, true),
            (Lock, self.lock),
            (Mode, self.mode),
            (Wheels, self.wheels),
            (ChordSet, self.chords),
            (Octave, self.chords),
            (Velocity, self.chords),
            (Timing, self.timing),
            (Hold, self.timing),
            (TwoRows, self.two_rows),
        ]
        .into_iter()
        .filter_map(|(part, on)| on.then_some(part))
        .collect()
    }
}

/// A control of the strip.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Part {
    Reset,
    Key,
    Scale,
    Lock,
    Mode,
    Wheels,
    ChordSet,
    Octave,
    Velocity,
    Timing,
    Hold,
    TwoRows,
}

impl Part {
    /// Its rect's offset below the row's top and its height, in U: spinners and knobs (0, 2.5), buttons (0.45, 2.33),
    /// `ChordSet` (0, 2.6: its 0.75 U caption, then its ≡ box from 0.45 U).
    fn span(self) -> (f32, f32) {
        match self {
            Part::Reset | Part::Lock | Part::Wheels | Part::Hold | Part::TwoRows => (0.45, 2.33),
            Part::ChordSet => (0.0, 2.6),
            _ => (0.0, ROW_H),
        }
    }
}

/// The number of controls the strip shows for the current channel: a notes channel (or none), a drum channel, or
/// another, and the keyboard kind.
pub fn strip_controls(notes: bool, drums: bool, mode: KeyboardMode) -> usize {
    Shown::of(notes, drums, mode).parts().len()
}

/// Where the parts go in the strip `rect`: cells min(5 U, w / per row) wide, per row = ⌈N / rows⌉ (two rows when 2 U a
/// control do not fit, [`strip_height`]), the first row's cells centred on the strip and the parts filling row 1, then
/// row 2, left to right (a shorter row 2 starts under row 1's first cell); each part's rect is its cell's x and width
/// over its [`Part::span`]. With two rows, also the divider between them: 1 px, inset 0.5 U, 0.24 U above row 2.
fn layout(rect: Rect, shown: Shown, m: &Metrics) -> (Vec<(Part, Rect)>, Option<Rect>) {
    let parts = shown.parts();
    let n = parts.len();
    let rows = if strip_height(n, rect.width(), m) == STRIP_H {
        1
    } else {
        2
    };
    let per_row = n.div_ceil(rows);
    let cell = m.u(CELL_MAX).min(rect.width() / per_row as f32);
    let x0 = rect.center().x - cell * per_row as f32 / 2.0;
    let row_y = |row: usize| rect.top() + m.u(ROW_TOP + row as f32 * (ROW_H + ROW_GAP));
    let placed = (parts.into_iter().enumerate())
        .map(|(i, part)| {
            let (dy, h) = part.span();
            let at = pos2(
                x0 + (i % per_row) as f32 * cell,
                row_y(i / per_row) + m.u(dy),
            );
            (part, Rect::from_min_size(at, vec2(cell, m.u(h))))
        })
        .collect();
    let divider = (rows == 2).then(|| {
        let y = row_y(1) - m.u(0.24);
        let x = rect.left() + m.u(0.5)..=rect.right() - m.u(0.5);
        Rect::from_x_y_ranges(x, y..=y + 1.0 / m.ppp())
    });
    (placed, divider)
}

/// The strip's knobs, where the panel's resize grab does not start: `Velocity` and `Timing` while they show, in the
/// strip `rect`.
pub fn knob_rects(rect: Rect, notes: bool, mode: KeyboardMode, m: &Metrics) -> Vec<Rect> {
    let (parts, _) = layout(rect, Shown::of(notes, false, mode), m);
    (parts.into_iter())
        .filter(|(p, _)| matches!(p, Part::Velocity | Part::Timing))
        .map(|(_, r)| r)
        .collect()
}

/// A round button of the strip (`Reset`, `Lock`, `Hold`, icons 12 and 13), centred in its rect: a `moduleFill` disc
/// with a `text` foreground, the other way round while `lit`, ringed by a 1 px `text` outline (on the `moduleFill` strip
/// the disc alone would not show). Returns whether it was clicked, and the foreground's colour.
fn round_button(
    ui: &mut Ui,
    cx: &UiCtx,
    id: Id,
    center: Pos2,
    d: f32,
    lit: bool,
) -> (bool, Color32) {
    let c = &cx.theme.colors;
    let (text, fill) = (c.text.to_color32(), c.module_fill.to_color32());
    let (back, ink) = if lit { (text, fill) } else { (fill, text) };
    let clicked = filled_circle_button(ui, id, center, d, back).clicked();
    let px = 1.0 / cx.m.ppp();
    ui.painter()
        .circle_stroke(center, (d - px) / 2.0, Stroke::new(px, text));
    (clicked, ink)
}

impl KeyboardSettings {
    /// Paints the panel over `rect` and returns what the user changed.
    pub fn show(&mut self, ui: &mut Ui, cx: &UiCtx, rect: Rect) -> Option<KeyboardSettingsAction> {
        use KeyboardSettingsAction as A;
        let (m, c) = (cx.m, &cx.theme.colors);
        let text = c.text.to_color32();
        let id = Id::new("keyboard_settings");
        let shown = Shown::of(self.notes, self.drums, self.mode);
        let (parts, divider) = layout(rect, shown, m);
        let mut action = None;
        // `moduleFill`, with a 1 px `background` border inside.
        let px = 1.0 / m.ppp();
        let painter = ui.painter();
        painter.rect_filled(rect, 0.0, c.module_fill.to_color32());
        let border = Stroke::new(px, c.background.to_color32());
        painter.rect_stroke(rect.shrink(px / 2.0), 0.0, border, StrokeKind::Middle);
        if let Some(divider) = divider {
            painter.rect_filled(divider, 0.0, text.gamma_multiply(DIVIDER_ALPHA));
        }
        let at = |part: Part| (parts.iter()).find_map(|&(p, r)| (p == part).then_some(r));
        let control = |name: &'static str, r: Rect| Control::new(id.with(name), r, name, text, 0.0);
        let font = m.font_id(FontSize::Pt6);
        for &(part, r) in &parts {
            match part {
                Part::Reset => {
                    let (reset, ink) =
                        round_button(ui, cx, id.with("reset"), r.center(), m.u(RESET_D), false);
                    ui.painter().text(
                        r.center(),
                        Align2::CENTER_CENTER,
                        "Reset",
                        font.clone(),
                        ink,
                    );
                    if reset {
                        // The chord controls and the chord editor too; not the pads' lock.
                        *self = Self {
                            notes: self.notes,
                            drums: self.drums,
                            lock: self.lock,
                            ..Self::default()
                        };
                        action = Some(A::Reset);
                    }
                }
                Part::Key => {
                    let key = control("Project Key", r);
                    if spinner_index(ui, cx, &key, &KEYS, &mut self.key).changed() {
                        action = Some(A::Key(self.key));
                    }
                }
                Part::Scale => {
                    let scales = control("Project Scale", r);
                    if spinner_grid(ui, cx, &scales, &SCALES, &mut self.scale, scale_popup)
                        .changed()
                    {
                        action = Some(A::Scale(self.scale));
                    }
                }
                Part::Mode => {
                    let mut mode = self.mode as usize;
                    let modes = control("Keyboard", r);
                    if spinner_index(ui, cx, &modes, &KeyboardMode::LABELS, &mut mode).changed() {
                        // Another kind leaves the chord editor open: it shows again with the chord keyboard.
                        self.mode = KeyboardMode::ALL[mode];
                        action = Some(A::Mode(self.mode));
                    }
                }
                Part::Lock | Part::Wheels | Part::TwoRows => {
                    let (name, icon, on) = match part {
                        Part::Lock => ("lock", IconId::Locked, &mut self.lock),
                        Part::Wheels => ("wheels", IconId::ChannelVolume, &mut self.wheels),
                        _ => ("two_rows", IconId::Tracks, &mut self.two_rows),
                    };
                    let d = m.u(ICON_D);
                    let (clicked, ink) = round_button(ui, cx, id.with(name), r.center(), d, *on);
                    if clicked {
                        *on = !*on;
                        action = Some(match part {
                            Part::Lock => A::Lock(*on),
                            Part::Wheels => A::Wheels(*on),
                            _ => A::TwoRows(*on),
                        });
                    }
                    let square = Rect::from_center_size(r.center(), vec2(d, d));
                    draw_icon(ui.painter(), square, icon, ink, ICON_SIZE, m);
                    let tip = match part {
                        Part::Wheels => Some("Pitch and mod wheels"),
                        Part::TwoRows => Some("Two keyboard rows"),
                        _ => None,
                    };
                    if let Some(tip) = tip {
                        crate::ui::tooltip::tip(ui, square, tip, None);
                    }
                }
                // The chord controls, together below.
                Part::ChordSet | Part::Octave | Part::Velocity | Part::Timing | Part::Hold => {}
            }
        }
        if shown.chords {
            action = self.chord_controls(ui, cx, &at, shown).or(action);
        }
        action
    }

    /// The chord keyboard's controls: `ChordSet` ≡, `Octave`, `Velocity`, and in `Chords` `Timing` and `Hold`.
    fn chord_controls(
        &mut self,
        ui: &mut Ui,
        cx: &UiCtx,
        at: &dyn Fn(Part) -> Option<Rect>,
        shown: Shown,
    ) -> Option<KeyboardSettingsAction> {
        let part = |p: Part| at(p).expect("a chord control shown");
        use KeyboardSettingsAction as A;
        let (m, c) = (cx.m, &cx.theme.colors);
        let text = c.text.to_color32();
        let id = Id::new("keyboard_settings");
        let mut action = None;

        // `ChordSet`: a plain spinner in `text`, its caption over the set's name and a ≡; a tap opens its items.
        let chord_set = part(Part::ChordSet);
        let caption = chord_set.with_max_y(chord_set.top() + m.u(CAPTION_H));
        let body = chord_set.with_min_y(caption.bottom());
        let painter = ui.painter();
        let small = m.font_id(FontSize::Pt8);
        painter.text(
            caption.center(),
            Align2::CENTER_CENTER,
            "ChordSet",
            small,
            text,
        );
        // A focus target too: `open` (Enter) on it opens the items.
        let button = crate::ui::focus::interact(ui, body, id.with("chord_set"), Sense::click());
        // The set's name, centred left of the ≡ at the box's right, shrunk to fit a narrow cell.
        let label = &self.chords.set_label;
        let room = body.with_max_x(body.right() - m.u(1.1));
        crate::ui::widgets::shrink(
            ui.painter(),
            m,
            room,
            Align2::CENTER_CENTER,
            label,
            10,
            text,
        );
        // The ≡: three hairlines at the box's right.
        let (t, x) = (m.t(), body.right() - m.u(0.6));
        for k in [-1.0, 0.0, 1.0] {
            let y = body.center().y + k * m.u(0.2);
            let bar = Rect::from_center_size(pos2(x, y), vec2(m.u(0.5), t));
            ui.painter().rect_filled(bar, 0.0, text);
        }
        if button.clicked() {
            self.set_menu = Some(body.center());
        }
        if let Some(at) = self.set_menu {
            let editor = if self.editor {
                "Close Editor"
            } else {
                "Open Editor"
            };
            let items: Vec<PopupItem<SetItem>> = [
                (SetItem::Default, "Default"),
                (SetItem::Load, "Load"),
                (SetItem::Save, "Save"),
                (SetItem::Editor, editor),
            ]
            .into_iter()
            .enumerate()
            .map(|(row, (item, label))| PopupItem::new(item, 0, row, label))
            .collect();
            let ctx = ui.ctx().clone();
            let result = popup::show(
                &ctx,
                id.with("chord_set_menu"),
                at,
                ctx.content_rect(),
                &items,
                cx,
            );
            if result.close {
                self.set_menu = None;
            }
            if let Some(item) = result.selected {
                if item == SetItem::Editor {
                    self.editor = !self.editor;
                    // Opening moves the keyboard focus into the editor. It is no modal: the popup's way back to the box
                    // goes (a touch on the item left none), so no control gone later sends the focus there.
                    if self.editor {
                        crate::ui::focus::drop_way_back(&ctx);
                        crate::ui::focus::move_to(&ctx, crate::ui::chord_editor::first_button());
                    }
                }
                action = Some(A::ChordSet(item));
            }
        }

        let mut octave = (self.chords.octave + 2).clamp(0, 4) as usize;
        let octaves = Control::new(id.with("Octave"), part(Part::Octave), "Octave", text, 0.0);
        if spinner_index(ui, cx, &octaves, &OCTAVES, &mut octave).changed() {
            self.chords.octave = octave as i32 - 2;
            action = Some(A::Octave(self.chords.octave));
        }
        let knob_control = |name: &'static str, r: Rect, default: f32| Control {
            menu: ParamMenu::Unbound,
            ..Control::new(id.with(name), r, name, text, default)
        };
        let mut v = self.chords.velocity;
        let velocity = knob_control("Velocity", part(Part::Velocity), 0.5);
        let shown_v = |v: f32| format!("{}", (v.max(MIN_VELOCITY) * 127.0).round());
        if knob(ui, cx, &velocity, &mut v, shown_v).changed() {
            self.chords.velocity = v.max(MIN_VELOCITY);
            action = Some(A::Velocity(self.chords.velocity));
        }
        if shown.timing {
            let mut v = (self.chords.timing_ms / TIMING_MS + 1.0) / 2.0;
            let timing = knob_control("Timing", part(Part::Timing), 0.5);
            let ms = |v: f32| (2.0 * v - 1.0) * TIMING_MS;
            if knob(ui, cx, &timing, &mut v, |v| format!("{} ms", ms(v).round())).changed() {
                self.chords.timing_ms = ms(v);
                action = Some(A::Timing(self.chords.timing_ms));
            }
            let d = m.u(ICON_D);
            let hold = part(Part::Hold).center();
            let (clicked, ink) = round_button(ui, cx, id.with("hold"), hold, d, self.chords.hold);
            ui.painter().text(
                hold,
                Align2::CENTER_CENTER,
                "Hold",
                m.font_id(FontSize::Pt6),
                ink,
            );
            if clicked {
                self.chords.hold = !self.chords.hold;
                action = Some(A::Hold(self.chords.hold));
            }
        }
        action
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::metrics::DEFAULT_UI_SCALE;
    use crate::ui::main_screen::Regions;
    use crate::ui::offscreen::{Offscreen, SCREEN, metrics};
    use crate::ui::widgets::pressed;
    use eframe::egui::{self, epaint::ClippedShape, pos2, vec2};

    use KeyboardSettingsAction as A;

    #[test]
    fn the_lists_are_the_spec_tables() {
        assert_eq!(KEYS.len(), 12);
        assert_eq!(SCALE_COLUMNS.iter().sum::<usize>(), SCALES.len());
        assert_eq!(SCALES[CHROMATIC], "Chromatic");
        assert_eq!(
            KeyboardMode::LABELS,
            ["Classic", "Scale", "Chords", "Strum"]
        );
        let defaults = KeyboardSettings::default();
        assert_eq!(
            (KEYS[defaults.key], SCALES[defaults.scale], defaults.mode),
            ("C", "Chromatic", KeyboardMode::Classic)
        );
    }

    #[test]
    fn the_scale_popup_has_three_columns_under_dimmed_headers() {
        let items = scale_popup(CHROMATIC);
        let headers: Vec<_> = items
            .iter()
            .filter(|i| i.is_header())
            .map(|i| (i.col, i.row, &*i.label, i.disabled))
            .collect();
        assert_eq!(
            headers,
            [
                (0, 0, "Major", true),
                (1, 0, "Minor", true),
                (2, 0, "Other", true)
            ]
        );
        let cell = |label: &str| {
            let i = items.iter().find(|i| i.label == label).unwrap();
            (i.col, i.row, i.id, i.current, i.disabled)
        };
        assert_eq!(cell("Major"), (0, 0, None, false, true));
        assert_eq!(cell("Phrygian Dominant"), (0, 9, Some(8), false, false));
        assert_eq!(cell("Minor Harmonic"), (1, 1, Some(9), false, false));
        assert_eq!(cell("Locrian"), (1, 11, Some(19), false, false));
        assert_eq!(cell("Chromatic"), (2, 1, Some(CHROMATIC), true, false));
        assert_eq!(cell("Whole Tone"), (2, 13, Some(32), false, false));
        assert_eq!(items.len(), 3 + SCALES.len());
    }

    fn near(a: f32, b: f32, what: &str) {
        assert!((a - b).abs() < 1e-3, "{what}: {a} vs {b}");
    }

    #[test]
    fn the_strip_takes_two_rows_when_2_u_a_control_do_not_fit_and_centres_its_cells() {
        let m = metrics();
        let u = m.u(1.0);
        // Classic on a notes channel: Reset, Key, Scale, Keyboard, icons 12 and 13.
        let shown = Shown::of(true, false, KeyboardMode::Classic);
        assert_eq!(strip_controls(true, false, KeyboardMode::Classic), 6);
        assert_eq!(
            (
                strip_height(6, 12.0 * u, &m),
                strip_height(6, 12.0 * u - 1.0, &m)
            ),
            (STRIP_H, STRIP_H2)
        );
        // One row on a wide strip: 5 U cells, centred.
        let wide = Rect::from_min_size(pos2(0.0, 0.0), vec2(40.0 * u, 3.0 * u));
        let (parts, divider) = layout(wide, shown, &m);
        assert_eq!(divider, None);
        assert_eq!(parts[0].1.left(), 20.0 * u - 15.0 * u);
        assert!(
            parts
                .iter()
                .all(|(_, r)| r.width() == 5.0 * u && r.top() < 0.6 * u)
        );
        // Two rows on a narrow one: 3 cells a row, each w / 3; row 2 is 3.25 U lower, under a divider.
        let narrow = Rect::from_min_size(pos2(0.0, 0.0), vec2(9.0 * u, 6.25 * u));
        let (parts, divider) = layout(narrow, shown, &m);
        assert!(parts.iter().all(|(_, r)| r.width() == 3.0 * u));
        let (key, mode) = (parts[1].1, parts[3].1);
        assert_eq!(
            (key.top(), mode.top(), mode.left()),
            (0.125 * u, 3.375 * u, 0.0)
        );
        assert!(divider.is_some_and(|d| d.top() < mode.top() && d.left() == 0.5 * u));
        // A drum channel: Reset, Key, Scale and Lock.
        assert_eq!(
            Shown::of(false, true, KeyboardMode::Chords).parts().last(),
            Some(&Part::Lock)
        );
    }

    /// The settings strip of the 1170 × 540 screen at U = 32, open in the default 5 U panel (3 U under its 1.75 U
    /// button bar).
    fn panel() -> Rect {
        let m = metrics();
        Regions::new(SCREEN, &m).with_strip(STRIP_H, &m).strip
    }

    struct Rig {
        screen: Offscreen,
        settings: KeyboardSettings,
        actions: Vec<KeyboardSettingsAction>,
        shapes: Vec<ClippedShape>,
    }

    impl Rig {
        fn new() -> Self {
            let mut rig = Self {
                screen: Offscreen::new(),
                settings: KeyboardSettings::default(),
                actions: Vec::new(),
                shapes: Vec::new(),
            };
            rig.frame();
            rig
        }

        fn run(&mut self, input: impl FnOnce(&mut Offscreen, &mut dyn FnMut(&mut Ui, &UiCtx))) {
            let (settings, actions) = (&mut self.settings, &mut self.actions);
            input(&mut self.screen, &mut |ui, cx| {
                actions.extend(settings.show(ui, cx, panel()));
            });
        }

        fn frame(&mut self) {
            let (settings, actions, shapes) =
                (&mut self.settings, &mut self.actions, &mut self.shapes);
            *shapes = self.screen.frame(vec![], &mut |ui, cx| {
                actions.extend(settings.show(ui, cx, panel()));
            });
        }

        fn click(&mut self, pos: Pos2) {
            self.run(|s, f| s.click(pos, f));
            self.frame();
        }

        fn texts(&self) -> Vec<String> {
            Offscreen::texts(&self.shapes)
                .into_iter()
                .map(|(t, _)| t)
                .collect()
        }

        /// The centre of the painted text `label` (the last one painted, so a popup's over the panel's).
        fn text_at(&self, label: &str) -> Pos2 {
            self.shapes
                .iter()
                .rev()
                .find_map(|s| match &s.shape {
                    egui::Shape::Text(t) if t.galley.text() == label => {
                        Some(t.pos + t.galley.rect.center().to_vec2())
                    }
                    _ => None,
                })
                .unwrap_or_else(|| panic!("{label} not painted"))
        }

        /// Where each part shows (`Rect::NOTHING` or the origin for one hidden): spinners, knobs and `ChordSet` by
        /// rect, buttons by centre.
        fn layout(&self) -> TestLayout {
            let s = &self.settings;
            let (parts, _) = layout(panel(), Shown::of(s.notes, s.drums, s.mode), &metrics());
            let at = |p| {
                (parts.iter())
                    .find_map(|&(q, r)| (q == p).then_some(r))
                    .unwrap_or(Rect::NOTHING)
            };
            let centre = |p| {
                (parts.iter())
                    .find_map(|&(q, r)| (q == p).then_some(r.center()))
                    .unwrap_or_default()
            };
            TestLayout {
                reset: centre(Part::Reset),
                lock: centre(Part::Lock),
                key: at(Part::Key),
                scale: at(Part::Scale),
                mode: at(Part::Mode),
                wheels: centre(Part::Wheels),
                two_rows: centre(Part::TwoRows),
                chord_set: at(Part::ChordSet),
                octave: at(Part::Octave),
                velocity: at(Part::Velocity),
                timing: at(Part::Timing),
                hold: centre(Part::Hold),
            }
        }
    }

    struct TestLayout {
        reset: Pos2,
        lock: Pos2,
        key: Rect,
        scale: Rect,
        mode: Rect,
        wheels: Pos2,
        two_rows: Pos2,
        chord_set: Rect,
        octave: Rect,
        velocity: Rect,
        timing: Rect,
        hold: Pos2,
    }

    /// The `^` and `v` taps of a spinner in `rect`: the body is under the 0.75 U caption strip.
    fn chevrons(rect: Rect) -> (Pos2, Pos2) {
        let u = metrics().u(1.0);
        let body = rect.with_min_y(rect.top() + 0.75 * u);
        (
            body.center() - vec2(0.0, 0.85 * u),
            body.center() + vec2(0.0, 0.85 * u),
        )
    }

    #[test]
    fn the_defaults_are_painted_with_their_captions() {
        let rig = Rig::new();
        let texts = rig.texts();
        for want in [
            "Reset",
            "Project Key",
            "C",
            "Project Scale",
            "Chromatic",
            "Keyboard",
            "Classic",
        ] {
            assert!(texts.iter().any(|t| t == want), "{want} in {texts:?}");
        }
    }

    #[test]
    fn the_chevrons_step_the_key_and_the_keyboard_kind() {
        let mut rig = Rig::new();
        let l = rig.layout();
        rig.click(chevrons(l.key).1);
        rig.click(chevrons(l.key).1);
        rig.click(chevrons(l.key).0);
        assert_eq!(rig.actions, [A::Key(1), A::Key(2), A::Key(1)]);
        assert_eq!(rig.settings.key, 1);
        assert!(rig.texts().iter().any(|t| t == "C# / Db"));
        // `Keyboard`: 0 Classic, 1 Scale, 2 Chords, 3 Strum.
        for want in [
            KeyboardMode::Scale,
            KeyboardMode::Chords,
            KeyboardMode::Strum,
        ] {
            // The chord kinds bring more controls: narrower cells.
            rig.click(chevrons(rig.layout().mode).1);
            assert_eq!(rig.settings.mode, want);
            assert_eq!(rig.actions.last(), Some(&A::Mode(want)));
        }
        assert!(rig.texts().iter().any(|t| t == "Strum"));
        rig.click(chevrons(rig.layout().mode).0);
        assert_eq!(rig.settings.mode, KeyboardMode::Chords);
        // The scale steps in reading order, column by column.
        rig.click(chevrons(rig.layout().scale).0);
        assert_eq!(rig.settings.scale, CHROMATIC - 1);
        assert_eq!(rig.actions.last(), Some(&A::Scale(19)));
    }

    #[test]
    fn the_scale_picker_opens_in_three_columns_and_picks_a_scale() {
        let mut rig = Rig::new();
        let l = rig.layout();
        rig.click(l.scale.center() + vec2(0.0, 0.375 * metrics().u(1.0)));
        // The headers are painted dimmed: the popup's ink at 30 %.
        let ink = crate::theme::theme_by_id(0).colors.background.to_color32();
        let colors = Offscreen::texts(&rig.shapes);
        let color = |label: &str| colors.iter().rev().find(|(t, _)| t == label).unwrap().1;
        assert_eq!(color("Other"), ink.gamma_multiply(0.3));
        assert_eq!(color("Dorian"), ink);
        let dorian = rig.text_at("Dorian");
        rig.click(dorian);
        assert_eq!(rig.settings.scale, 17);
        assert_eq!(rig.actions, [A::Scale(17)]);
        assert!(
            !rig.texts().iter().any(|t| t == "Other"),
            "the popup closed"
        );
    }

    #[test]
    fn the_wheels_and_two_rows_toggle_lit_and_reset_turns_everything_off() {
        let mut rig = Rig::new();
        let l = rig.layout();
        let text = crate::theme::theme_by_id(0).colors.text.to_color32();
        // Lit: a `text` disc under the icon (a `moduleFill` one while off), centred on the button; the pointer over it
        // brightens it (× 1.15).
        let lit = |rig: &Rig, at: Pos2| {
            rig.shapes.iter().any(|s| match &s.shape {
                egui::Shape::Circle(c) => {
                    c.center == at && (c.fill == text || c.fill == pressed(text))
                }
                _ => false,
            })
        };
        // Icon 12 toggles the wheels, lit while on.
        assert!(!lit(&rig, l.wheels));
        rig.click(l.wheels);
        assert_eq!(rig.actions, [A::Wheels(true)]);
        assert!(rig.settings.wheels && lit(&rig, l.wheels));
        rig.click(l.wheels);
        assert_eq!(rig.actions.last(), Some(&A::Wheels(false)));
        assert!(!rig.settings.wheels && !lit(&rig, l.wheels));
        rig.actions.clear();
        // Icon 13 toggles two rows, lit while on.
        assert!(!lit(&rig, l.two_rows));
        rig.click(l.two_rows);
        assert_eq!(rig.actions, [A::TwoRows(true)]);
        assert!(rig.settings.two_rows && lit(&rig, l.two_rows));
        rig.click(l.two_rows);
        assert_eq!(rig.actions.last(), Some(&A::TwoRows(false)));
        assert!(!rig.settings.two_rows && !lit(&rig, l.two_rows));
        // The mode's text is at full strength.
        let ink = |label: &str| {
            rig.shapes.iter().find_map(|s| match &s.shape {
                egui::Shape::Text(t) if t.galley.text() == label => {
                    Some(t.galley.rows[0].visuals.mesh.vertices[0].color)
                }
                _ => None,
            })
        };
        assert_eq!(ink("Classic"), Some(text));
        assert_eq!(ink("Project Key"), Some(text));

        // Reset turns two rows off with the wheels and key-lock (the key and the scale), and the kind back to Classic.
        rig.click(l.two_rows);
        rig.click(l.wheels);
        rig.click(chevrons(l.key).1);
        rig.click(chevrons(l.mode).1);
        assert!(rig.settings.two_rows && rig.settings.wheels);
        rig.click(l.reset);
        assert_eq!(rig.actions.last(), Some(&A::Reset));
        assert_eq!(rig.settings, KeyboardSettings::default());
        assert!(!lit(&rig, l.two_rows) && !lit(&rig, l.wheels));
    }

    /// A bottom panel `h` U tall at the bottom of the 540-point screen, at `ppp`.
    fn panel_of(h: f32, ppp: f32) -> (Rect, Metrics) {
        let m = Metrics::from_pixels_per_point(ppp, DEFAULT_UI_SCALE);
        let panel =
            Rect::from_x_y_ranges(SCREEN.x_range(), SCREEN.bottom() - m.u(h)..=SCREEN.bottom());
        (panel, m)
    }

    #[test]
    fn the_pads_and_the_sample_row_sit_0_35_u_under_the_keys_top_and_the_keys_have_no_minimum_height()
     {
        for ppp in [1.0, 2.0] {
            for (h, open, k) in [
                (6.0, false, 4.25),
                (6.0, true, 1.25),
                (4.75, true, 0.0),
                // Shorter than the bar and the open strip: K < 0, the content laid out below the panel's bottom.
                (4.0, true, -0.75),
                (1.75, true, -3.0),
            ] {
                let (panel, m) = panel_of(h, ppp);
                let u = m.u(1.0);
                let what = format!("{h} U, open {open}, ppp {ppp}");
                // The strip is placed whatever the panel's height.
                let bar = panel.with_max_y(panel.top() + 1.75 * u);
                let strip = strip_rect(bar, if open { STRIP_H } else { 0.0 }, &m);
                near(strip.top(), panel.top() + 1.75 * u, &what);
                near(strip.height(), if open { 3.0 * u } else { 0.0 }, &what);
                let keys = keys_rect(panel, strip);
                near(keys.top(), strip.bottom(), &what);
                near(keys.height(), k * u, &what);
                assert_eq!(keys.x_range(), panel.x_range(), "{what}");
                let pads = pads_rect(keys, &m);
                near(pads.top(), keys.top() + 0.35 * u, &what);
                near(pads.height(), (k - 0.35) * u, &what);
                assert_eq!(pads.x_range(), panel.x_range(), "{what}");
            }
        }
    }

    #[test]
    fn on_a_short_panel_the_pads_keep_their_least_size_top_aligned_and_overflow_below() {
        // Neither scrolling nor shrinking further.
        for ppp in [1.0, 2.0] {
            for h in [6.0, 5.0, 4.0] {
                let (panel, m) = panel_of(h, ppp);
                let bar = panel.with_max_y(panel.top() + m.u(1.75));
                let area = pads_rect(keys_rect(panel, strip_rect(bar, STRIP_H, &m)), &m);
                let g = crate::ui::drums::pad_grid(area * ppp, SCREEN.width() * ppp, m.u_px());
                let what = format!("{h} U, ppp {ppp}");
                // r = floor(min(screen w, 20 U) / 16) = 40 px a point, though the area is 0.9 U tall or less.
                assert_eq!(g.r, 40.0 * ppp, "{what}");
                // Pad 0 hangs r under the area's top; the grid runs past the panel's bottom.
                near(g.centres[0].y, area.top() * ppp + g.r, &what);
                assert!(g.centres[15].y - g.r > panel.bottom() * ppp, "{what}");
            }
        }
    }

    #[test]
    fn the_wheels_start_at_the_keys_top_at_most_5_u_tall_centred_when_capped_and_narrow_the_piano()
    {
        for ppp in [1.0, 2.0] {
            // (keys area K, wheel h, wheel top under the keys' top), in U.
            for (k, wheel, top) in [(4.25, 3.75, 0.0), (5.5, 5.0, 0.0), (7.5, 5.0, 1.0)] {
                let (keys, m) = panel_of(k, ppp);
                let u = m.u(1.0);
                let what = format!("{k} U, ppp {ppp}");
                let [wheel_mod, pitch] = wheel_rects(keys, &m);
                for (r, x) in [(wheel_mod, 0.0), (pitch, 2.0)] {
                    near(r.left(), keys.left() + x * u, &what);
                    near(r.width(), 2.0 * u, &what);
                    near(r.top(), keys.top() + top * u, &what);
                    near(r.height(), wheel * u, &what);
                }
                // With them shown, the piano starts 4 U right and is 4 U narrower.
                let r = piano_rect(keys, true, &m);
                near(r.left(), keys.left() + 4.0 * u, &what);
                near(r.width(), keys.width() - 4.0 * u, &what);
                assert_eq!(piano_rect(keys, false, &m), keys);
            }
        }
    }

    #[test]
    fn the_panel_height_stays_in_range_and_eases_to_the_target_snapping_within_2_px() {
        for ppp in [1.0, 2.0] {
            let m = Metrics::from_pixels_per_point(ppp, DEFAULT_UI_SCALE);
            let u = m.u(1.0);
            // c … the screen height.
            let min = crate::ui::panel_resize::collapsed(1, &m);
            assert_eq!(panel_target(5.0, 540.0, min, &m), 5.0 * u);
            assert_eq!(panel_target(0.0, 540.0, min, &m), min);
            assert_eq!(panel_target(100.0, 540.0, min, &m), 540.0);
            // h ← h + 0.5·(t − h) a frame at 60 fps, snapping within 2 px.
            let (from, to) = (5.0 * u, 6.0 * u);
            let mut h = from;
            let mut ticks = vec![];
            while h != to {
                h = panel_tick(h, to, 0.5, &m);
                ticks.push(h);
            }
            let px = |h: f32| (to - h) * ppp;
            // At ppp 1, U = 32 px: 16, 8, 4, 2 px short (2 is not within 2 px), then 1 → snapped; at ppp 2 (U = 64
            // px) one more halving.
            let want: Vec<f32> = if ppp == 1.0 {
                vec![16.0, 8.0, 4.0, 2.0, 0.0]
            } else {
                vec![32.0, 16.0, 8.0, 4.0, 2.0, 0.0]
            };
            assert_eq!(
                ticks.iter().map(|&h| px(h)).collect::<Vec<_>>(),
                want,
                "ppp {ppp}"
            );
            // Down works the same way, and a target reached stays put.
            assert_eq!(panel_tick(to, from, 0.5, &m), from + 0.5 * u);
            assert_eq!(panel_tick(to, to, 0.5, &m), to);
        }
    }

    // ---- the strip's look and visibility, and the chord controls ----

    impl Rig {
        fn with(notes: bool, mode: KeyboardMode) -> Self {
            let mut rig = Self::new();
            (rig.settings.notes, rig.settings.mode) = (notes, mode);
            rig.frame();
            rig
        }

        fn painted(&self, label: &str) -> bool {
            self.texts().iter().any(|t| t == label)
        }

        /// A disc is painted centred on `at`.
        fn disc(&self, at: Pos2) -> bool {
            (self.shapes.iter())
                .any(|s| matches!(&s.shape, egui::Shape::Circle(c) if c.center == at))
        }
    }

    #[test]
    fn the_strip_shows_its_parts_by_the_channels_first_lane_and_the_kind() {
        // Not a notes channel: Reset, Project Key and Project Scale only.
        let rig = Rig::with(false, KeyboardMode::Chords);
        let l = rig.layout();
        for (label, want) in [
            ("Reset", true),
            ("Project Key", true),
            ("Project Scale", true),
            ("Keyboard", false),
            ("ChordSet", false),
            ("Octave", false),
            ("Velocity", false),
        ] {
            assert_eq!(rig.painted(label), want, "{label}, no notes lane");
        }
        assert!(!rig.disc(l.wheels) && !rig.disc(l.two_rows) && !rig.disc(l.lock));
        // A drum channel: the padlock too, lit while the pads are locked.
        let mut rig = Rig::new();
        (rig.settings.notes, rig.settings.drums) = (false, true);
        rig.frame();
        let lock = rig.layout().lock;
        assert!(rig.disc(lock) && !rig.painted("Keyboard"));
        rig.click(lock);
        assert_eq!(
            (rig.actions.last(), rig.settings.lock),
            (Some(&A::Lock(false)), false)
        );
        // A notes channel, a piano kind: Keyboard and both icons, no chord controls.
        let rig = Rig::with(true, KeyboardMode::Scale);
        let l = rig.layout();
        assert!(rig.painted("Keyboard") && !rig.painted("ChordSet") && !rig.painted("Hold"));
        assert!(rig.disc(l.wheels) && rig.disc(l.two_rows));
        // Chords: icon 12, no icon 13, and ChordSet, Octave, Velocity, Timing and Hold.
        let rig = Rig::with(true, KeyboardMode::Chords);
        let l = rig.layout();
        for label in [
            "Keyboard",
            "ChordSet",
            "Default: Major",
            "Octave",
            "Velocity",
            "Timing",
            "Hold",
        ] {
            assert!(rig.painted(label), "{label} in Chords");
        }
        assert!(rig.disc(l.wheels) && !rig.disc(l.two_rows));
        assert!(l.chord_set.left() > l.wheels.x && l.hold.x > l.timing.right());
        // Strum: no Timing and no Hold.
        let rig = Rig::with(true, KeyboardMode::Strum);
        assert!(rig.painted("ChordSet") && rig.painted("Velocity"));
        assert!(!rig.painted("Timing") && !rig.painted("Hold"));
    }

    #[test]
    fn the_strip_is_module_fill_bordered_in_background_with_text_controls_and_module_fill_buttons()
    {
        let rig = Rig::with(true, KeyboardMode::Chords);
        let c = crate::theme::theme_by_id(0).colors;
        let (text, module) = (c.text.to_color32(), c.module_fill.to_color32());
        let strip = panel();
        assert!(rig.shapes.iter().any(|s| matches!(&s.shape,
            egui::Shape::Rect(r) if r.rect == strip && r.fill == module)));
        assert!(rig.shapes.iter().any(|s| matches!(&s.shape,
            egui::Shape::Rect(r) if r.stroke.color == c.background.to_color32() && r.stroke.width == 1.0
                && strip.contains_rect(r.rect))));
        // Every caption, value and label in `text`.
        let colors = Offscreen::texts(&rig.shapes);
        for label in [
            "Project Key",
            "C",
            "Keyboard",
            "Chords",
            "ChordSet",
            "Octave",
            "0",
            "Velocity",
            "Timing",
            "Reset",
            "Hold",
        ] {
            let (_, color) = colors
                .iter()
                .find(|(t, _)| t == label)
                .unwrap_or_else(|| panic!("{label}"));
            assert_eq!(*color, text, "{label}");
        }
        // Reset and Hold: `moduleFill` discs.
        let l = rig.layout();
        for at in [l.reset, l.hold] {
            assert!(
                rig.shapes.iter().any(|s| matches!(&s.shape,
                egui::Shape::Circle(d) if d.center == at && d.fill == module)),
                "{at:?}"
            );
        }
    }

    #[test]
    fn the_chord_controls_report_octave_velocity_timing_hold_and_the_chord_set_items() {
        let mut rig = Rig::with(true, KeyboardMode::Chords);
        let l = rig.layout();
        // `v` steps down the list, `-2` … `+2`.
        rig.click(chevrons(l.octave).1);
        assert_eq!(rig.actions, [A::Octave(1)]);
        assert!(rig.painted("+1"));
        rig.click(l.hold);
        assert_eq!(rig.actions.last(), Some(&A::Hold(true)));
        assert!(rig.settings.chords.hold);
        // The knobs drag up: more velocity, a later Timing.
        let u = metrics().u(1.0);
        for (r, what) in [(l.velocity, "Velocity"), (l.timing, "Timing")] {
            let at = r.center() + vec2(0.0, 0.375 * u);
            rig.run(|s, f| s.drag(at, &[vec2(0.0, -20.0), vec2(0.0, -60.0)], f));
            match rig.actions.last() {
                Some(A::Velocity(v)) if what == "Velocity" => assert!(*v > 0.5, "{v}"),
                Some(A::Timing(ms)) if what == "Timing" => {
                    assert!(*ms > 0.0 && *ms <= 40.0, "{ms}")
                }
                other => panic!("{what}: {other:?}"),
            }
        }
        // ChordSet ≡: Default, Load, Save, Open Editor; the editor's item toggles it and reads `Close Editor` after.
        rig.click(l.chord_set.center() + vec2(0.0, 0.375 * u));
        for _ in 0..24 {
            rig.frame();
        }
        for item in ["Default", "Load", "Save", "Open Editor"] {
            assert!(rig.painted(item), "{item}");
        }
        let open = rig.text_at("Open Editor");
        rig.click(open);
        assert_eq!(rig.actions.last(), Some(&A::ChordSet(SetItem::Editor)));
        assert!(rig.settings.editor);
        rig.click(l.chord_set.center() + vec2(0.0, 0.375 * u));
        for _ in 0..24 {
            rig.frame();
        }
        assert!(rig.painted("Close Editor"));
    }

    #[test]
    fn the_chord_editor_closes_on_reset_but_not_on_another_kind_and_chordset_has_no_box() {
        let mut rig = Rig::with(true, KeyboardMode::Chords);
        let fill = crate::theme::theme_by_id(0)
            .colors
            .control_fill
            .to_color32();
        let l = rig.layout();
        assert!(!rig.shapes.iter().any(|s| matches!(&s.shape,
            egui::Shape::Rect(r) if r.fill == fill && l.chord_set.contains_rect(r.rect))));
        rig.settings.editor = true;
        // Only `Close Editor`, `Reset` and gamepad Menu close it; another kind does not.
        rig.click(chevrons(l.mode).1);
        assert_eq!(rig.settings.mode, KeyboardMode::Strum);
        assert!(rig.settings.editor);
        rig.click(l.reset);
        assert!(!rig.settings.editor);
    }
}
