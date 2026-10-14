//! The keyboard settings panel, opened by the transport's icon-48 button. From the left: `Reset` (outline circle), the
//! `Project Key`, `Project Scale` and `Keyboard` spinners with their captions above, two icon buttons (icons 12 and
//! 13), and in `Chords` and `Strum` the chord controls: `ChordSet` ≡, `Octave`, `Velocity`, and in `Chords` only
//! `Timing` and `Hold`. Which parts show depends on the channel and kind (see `Shown`). The quick scale buttons
//! (`Major`, `Harmonic`, `Melodic`, `Minor`) are left out on purpose.
//!
//! The panel is a strip in the bottom keyboard panel, placed by [`strip_rect`]; the keys below it get [`keys_rect`].
//!
//! - The panel takes the rect it is given (the strip), filled with `moduleFill` with a 1 px `background` border inside.
//! - `Reset` puts every value of the panel back to its default (`C`, `Chromatic`, `Classic`, both toggles off): so two
//!   rows go off with the wheels and key-lock. It resets the chords too: the default set, `Octave` 0, `Timing` 0,
//!   `Velocity` 0.5, `Hold` off; and the chord editor closes.
//! - Icon 12 (two faders) toggles the pitch and mod wheels and icon 13 (two bars) the second keyboard row: the keyboard
//!   panel's `showWheels` and `showTwoRows`. A button is lit (a `text` disc under a `background` icon, as the browser's
//!   `Search`) while its toggle is on.
//! - The `Keyboard` spinner picks any of the four kinds. `Chords` and `Strum` put the chord keyboard
//!   ([`crate::ui::chord_keyboard`]) in the piano's place; `ChordSet` ≡ `Open Editor` opens the chord editor
//!   ([`crate::ui::chord_editor`]) under its strips; only `Close Editor` and Reset close it.
//! - The panel's scale is its place in [`SCALES`], the picker read column by column; the chevrons step in that order.
//!   The song file's key-lock scale id follows its own stored list instead (see [`scale_from_stored`]).
//! - Sizes, in U: `Reset` is a 1.5 U outline circle with a 6 pt label, as the browser's text buttons, centred in a
//!   2 U column; the spinners are 2.75 U tall (a 0.75 U caption strip, 8 pt, over a 2 U body) and 2.5, 5 and 3 U
//!   wide, 0.25 U apart; the icon buttons are 1.25 U outline circles, as the transport's, in 1.5 U columns. The chord
//!   controls follow: `ChordSet` 4.5 U, `Octave` 2 U and each knob 2 U wide, then `Hold` in a 1.5 U column.
//!   Everything is centred on the panel's middle line, in `text`.

use eframe::egui::{Align2, Color32, Id, Pos2, Rect, Sense, Stroke, StrokeKind, Ui, pos2, vec2};

use crate::icons::{IconId, draw_icon};
use crate::metrics::{FontSize, Metrics};
use crate::ui::UiCtx;
use crate::ui::buttons::filled_circle_button;
use crate::ui::chord_keyboard::TIMING_MS;
use crate::ui::main_screen::TRANSPORT_H;
use crate::ui::popup;
use crate::ui::popup::PopupItem;
use crate::ui::widgets::{Control, ParamMenu, knob, spinner_grid, spinner_index};

/// The scale list (the picker's order) and the stored-id mapping live with the song's key lock.
pub use crate::song::scales::{CHROMATIC, SCALES, scale_from_stored, scale_to_stored};

// Sizes in U (see above).
const RESET_COLUMN: f32 = 2.0;
const RESET_D: f32 = 1.5;
const SPINNER_H: f32 = 2.75;
const SPINNER_W: [f32; 3] = [2.5, 5.0, 3.0];
const GAP: f32 = 0.25;
const ICON_COLUMN: f32 = 1.5;
const ICON_D: f32 = 1.25;
const ICON_SIZE: f32 = 0.7;
/// The chord controls' widths: the `ChordSet` box, the `Octave` spinner and each knob; a spinner's caption.
const CHORD_SET_W: f32 = 4.5;
const OCTAVE_W: f32 = 2.0;
const KNOB_W: f32 = 2.0;
const CAPTION_H: f32 = 0.75;
/// The least chord velocity.
const MIN_VELOCITY: f32 = 1.0 / 127.0;

/// The strip's height while it is open, in U.
pub const STRIP_H: f32 = 3.0;
/// How far the pads and the sample row sit below the keys' top, in U.
const PADS_Y: f32 = 0.35;
/// A pitch or mod wheel's width, in U.
const WHEEL_W: f32 = 2.0;
/// The preferred panel height that turning two rows on raises to, in U.
pub const TWO_ROWS_H: f32 = 6.0;
/// The keyboard panel's default preferred height (`preferredKbdSize`), in U.
pub const PANEL_H: f32 = 5.0;

/// The settings strip in the bottom keyboard `panel` (the button bar, then the keys): the panel's x and width, under
/// the button bar, [`STRIP_H`] tall while `open` and none when closed. It does not slide.
pub fn strip_rect(panel: Rect, open: bool, m: &Metrics) -> Rect {
    let top = panel.top() + m.u(TRANSPORT_H);
    let height = if open { m.u(STRIP_H) } else { 0.0 };
    Rect::from_x_y_ranges(panel.x_range(), top..=top + height)
}

/// The keys' area of the bottom keyboard `panel`: under the button bar and the strip, so it loses [`STRIP_H`] while the
/// strip is `open`. Its height K has no minimum: on a panel shorter than the bar and the open strip it is negative, and
/// the content is laid out below the panel's bottom. Only K = 0 hides the content.
pub fn keys_rect(panel: Rect, open: bool, m: &Metrics) -> Rect {
    panel.with_min_y(strip_rect(panel, open, m).bottom())
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

/// The `MOD` and `PITCH` wheels of the keyboard `panel`, left to right from its left edge, each [`WHEEL_W`] wide, under
/// the button bar: min(panel h − 2.25 U, 5 U) tall, centred in the extra height when capped. They ignore the strip (and
/// overlap it while it is open).
pub fn wheel_rects(panel: Rect, m: &Metrics) -> [Rect; 2] {
    let room = panel.height() - m.u(2.25);
    let h = room.min(m.u(5.0));
    let top = panel.top() + m.u(TRANSPORT_H) + (room - h) / 2.0;
    [0.0, WHEEL_W]
        .map(|x| Rect::from_min_size(pos2(panel.left() + m.u(x), top), vec2(m.u(WHEEL_W), h)))
}

/// The keyboard panel's height for a preferred height of `preferred` U on a screen `screen_h` points tall: within
/// trunc(1.75 U) px … the screen's height.
pub fn panel_target(preferred: f32, screen_h: f32, m: &Metrics) -> f32 {
    let min = crate::ui::panel_resize::panel_min(m);
    m.u(preferred).clamp(min, screen_h.max(min))
}

/// One UI tick of the panel's height `h` toward `target` (points): h ← h + 0.5·(target − h), snapping to the target
/// within 2 px (a tick is a drawn frame, as the rack's slide).
pub fn panel_tick(h: f32, target: f32, m: &Metrics) -> f32 {
    let h = h + 0.5 * (target - h);
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

/// Which parts of the strip show: `Reset`, `Project Key` and `Project Scale` always; `Keyboard` and icon 12 for a notes
/// channel; icon 13 for one not in `Chords` or `Strum`; `ChordSet`, `Octave` and `Velocity` for one in them; `Timing`
/// and `Hold` in `Chords` only.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
struct Shown {
    mode: bool,
    wheels: bool,
    two_rows: bool,
    chords: bool,
    timing: bool,
}

impl Shown {
    fn of(notes: bool, mode: KeyboardMode) -> Self {
        Self {
            mode: notes,
            wheels: notes,
            two_rows: notes && !mode.chords(),
            chords: notes && mode.chords(),
            timing: notes && mode == KeyboardMode::Chords,
        }
    }
}

/// Where the panel's parts go: the parts shown, left to right, each spinner or knob followed by a 0.25 U gap, each
/// icon button in a 1.5 U column. A part hidden takes no room (its place is where it would go).
#[derive(Debug)]
struct Layout {
    reset: Pos2,
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

fn layout(rect: Rect, shown: Shown, m: &Metrics) -> Layout {
    let y = rect.center().y;
    let mut x = rect.left() + m.u(RESET_COLUMN);
    let mut boxed = |w: f32, on: bool| {
        let r = Rect::from_min_size(
            pos2(x, y - m.u(SPINNER_H) / 2.0),
            vec2(m.u(w), m.u(SPINNER_H)),
        );
        if on {
            x = r.right() + m.u(GAP);
        }
        r
    };
    let key = boxed(SPINNER_W[0], true);
    let scale = boxed(SPINNER_W[1], true);
    let mode = boxed(SPINNER_W[2], shown.mode);
    let mut column = |on: bool| {
        let c = pos2(x + m.u(ICON_COLUMN) / 2.0, y);
        if on {
            x += m.u(ICON_COLUMN);
        }
        c
    };
    let wheels = column(shown.wheels);
    let two_rows = column(shown.two_rows);
    let mut boxed = |w: f32, on: bool| {
        let r = Rect::from_min_size(
            pos2(x, y - m.u(SPINNER_H) / 2.0),
            vec2(m.u(w), m.u(SPINNER_H)),
        );
        if on {
            x = r.right() + m.u(GAP);
        }
        r
    };
    let chord_set = boxed(CHORD_SET_W, shown.chords);
    let octave = boxed(OCTAVE_W, shown.chords);
    let velocity = boxed(KNOB_W, shown.chords);
    let timing = boxed(KNOB_W, shown.timing);
    Layout {
        reset: pos2(rect.left() + m.u(RESET_COLUMN) / 2.0, y),
        key,
        scale,
        mode,
        wheels,
        two_rows,
        chord_set,
        octave,
        velocity,
        timing,
        hold: pos2(x + m.u(ICON_COLUMN) / 2.0, y),
    }
}

/// The strip's knobs, where the panel's resize grab does not start: `Velocity` and `Timing` while they show, in the
/// strip `rect`.
pub fn knob_rects(rect: Rect, notes: bool, mode: KeyboardMode, m: &Metrics) -> Vec<Rect> {
    let shown = Shown::of(notes, mode);
    let l = layout(rect, shown, m);
    [(l.velocity, shown.chords), (l.timing, shown.timing)]
        .into_iter()
        .filter_map(|(r, on)| on.then_some(r))
        .collect()
}

/// A round button of the strip (`Reset`, `Hold`, icons 12 and 13): a `moduleFill` disc with a `text` foreground, the
/// other way round while `lit`, ringed by a 1 px `text` outline (on the `moduleFill` strip the disc alone would not
/// show). Returns whether it was clicked, and the foreground's colour.
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
        let shown = Shown::of(self.notes, self.mode);
        let l = layout(rect, shown, m);
        let mut action = None;
        // `moduleFill`, with a 1 px `background` border inside.
        let px = 1.0 / m.ppp();
        let painter = ui.painter();
        painter.rect_filled(rect, 0.0, c.module_fill.to_color32());
        let border = Stroke::new(px, c.background.to_color32());
        painter.rect_stroke(rect.shrink(px / 2.0), 0.0, border, StrokeKind::Middle);

        let (reset, ink) = round_button(ui, cx, id.with("reset"), l.reset, m.u(RESET_D), false);
        let font = m.font_id(FontSize::Pt6);
        ui.painter()
            .text(l.reset, Align2::CENTER_CENTER, "Reset", font.clone(), ink);
        if reset {
            // The chord controls and the chord editor too.
            *self = Self {
                notes: self.notes,
                ..Self::default()
            };
            action = Some(A::Reset);
        }

        let control = |name: &'static str, r: Rect| Control::new(id.with(name), r, name, text, 0.0);
        if spinner_index(ui, cx, &control("Project Key", l.key), &KEYS, &mut self.key).changed() {
            action = Some(A::Key(self.key));
        }
        let scales = control("Project Scale", l.scale);
        if spinner_grid(ui, cx, &scales, &SCALES, &mut self.scale, scale_popup).changed() {
            action = Some(A::Scale(self.scale));
        }
        if shown.mode {
            let mut mode = self.mode as usize;
            let modes = control("Keyboard", l.mode);
            if spinner_index(ui, cx, &modes, &KeyboardMode::LABELS, &mut mode).changed() {
                // Another kind leaves the chord editor open: it shows again with the chord keyboard.
                self.mode = KeyboardMode::ALL[mode];
                action = Some(A::Mode(self.mode));
            }
        }
        let d = m.u(ICON_D);
        for (name, icon, center, on, toggled, show) in [
            (
                "wheels",
                IconId::ChannelVolume,
                l.wheels,
                &mut self.wheels,
                A::Wheels as fn(bool) -> A,
                shown.wheels,
            ),
            (
                "two_rows",
                IconId::Tracks,
                l.two_rows,
                &mut self.two_rows,
                A::TwoRows,
                shown.two_rows,
            ),
        ] {
            if !show {
                continue;
            }
            let (clicked, ink) = round_button(ui, cx, id.with(name), center, d, *on);
            if clicked {
                *on = !*on;
                action = Some(toggled(*on));
            }
            let at = Rect::from_center_size(center, vec2(d, d));
            draw_icon(ui.painter(), at, icon, ink, ICON_SIZE, m);
        }
        if shown.chords {
            action = self.chord_controls(ui, cx, &l, shown).or(action);
        }
        action
    }

    /// The chord keyboard's controls: `ChordSet` ≡, `Octave`, `Velocity`, and in `Chords` `Timing` and `Hold`.
    fn chord_controls(
        &mut self,
        ui: &mut Ui,
        cx: &UiCtx,
        l: &Layout,
        shown: Shown,
    ) -> Option<KeyboardSettingsAction> {
        use KeyboardSettingsAction as A;
        let (m, c) = (cx.m, &cx.theme.colors);
        let text = c.text.to_color32();
        let id = Id::new("keyboard_settings");
        let mut action = None;

        // `ChordSet`: a plain spinner in `text`, its caption over the set's name and a ≡; a tap opens its items.
        let (caption, body) = l
            .chord_set
            .split_top_bottom_at_y(l.chord_set.top() + m.u(CAPTION_H));
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
        // The set's name, centred left of the ≡ at the box's right.
        let label = &self.chords.set_label;
        let font = m.font_id(FontSize::Pt10);
        let at = pos2(
            (body.left() + body.right() - m.u(1.1)) / 2.0,
            body.center().y,
        );
        ui.painter()
            .text(at, Align2::CENTER_CENTER, label, font, text);
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
        let octaves = Control::new(id.with("Octave"), l.octave, "Octave", text, 0.0);
        if spinner_index(ui, cx, &octaves, &OCTAVES, &mut octave).changed() {
            self.chords.octave = octave as i32 - 2;
            action = Some(A::Octave(self.chords.octave));
        }
        let knob_control = |name: &'static str, r: Rect, default: f32| Control {
            menu: ParamMenu::Unbound,
            ..Control::new(id.with(name), r, name, text, default)
        };
        let mut v = self.chords.velocity;
        let velocity = knob_control("Velocity", l.velocity, 0.5);
        let shown_v = |v: f32| format!("{}", (v.max(MIN_VELOCITY) * 127.0).round());
        if knob(ui, cx, &velocity, &mut v, shown_v).changed() {
            self.chords.velocity = v.max(MIN_VELOCITY);
            action = Some(A::Velocity(self.chords.velocity));
        }
        if shown.timing {
            let mut v = (self.chords.timing_ms / TIMING_MS + 1.0) / 2.0;
            let timing = knob_control("Timing", l.timing, 0.5);
            let ms = |v: f32| (2.0 * v - 1.0) * TIMING_MS;
            if knob(ui, cx, &timing, &mut v, |v| format!("{} ms", ms(v).round())).changed() {
                self.chords.timing_ms = ms(v);
                action = Some(A::Timing(self.chords.timing_ms));
            }
            let d = m.u(ICON_D);
            let (clicked, ink) = round_button(ui, cx, id.with("hold"), l.hold, d, self.chords.hold);
            ui.painter().text(
                l.hold,
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
    use crate::metrics::DEFAULT_UI_SCALE_INDEX;
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

    /// The bottom keyboard panel (the button bar and the keyboard) of a 1170 × 540 point screen, with the regions
    /// it comes from, at `ppp` device pixels per point.
    fn bottom_panel(ppp: f32) -> (Rect, Regions, Metrics) {
        let m = Metrics::from_pixels_per_point(ppp, DEFAULT_UI_SCALE_INDEX);
        let regions = Regions::new(SCREEN, &m);
        let panel = Rect::from_min_max(regions.transport.min, regions.keyboard.max);
        (panel, regions, m)
    }

    fn near(a: f32, b: f32, what: &str) {
        assert!((a - b).abs() < 1e-3, "{what}: {a} vs {b}");
    }

    #[test]
    fn the_strip_is_3_u_under_the_1_75_u_button_bar_across_the_panel_and_the_keys_start_below_it() {
        for ppp in [1.0, 2.0] {
            let (panel, regions, m) = bottom_panel(ppp);
            let u = m.u(1.0);
            let strip = strip_rect(panel, true, &m);
            near(strip.left(), panel.left(), "x");
            near(strip.width(), panel.width(), "width");
            near(strip.top(), panel.top() + 1.75 * u, "y");
            near(strip.height(), 3.0 * u, "height");
            // The button bar is the transport bar, so the strip starts where the keyboard band does.
            near(strip.top(), regions.keyboard.top(), "keyboard band top");
            // The keys start below it and lose exactly 3 U.
            let keys = keys_rect(panel, true, &m);
            near(keys.top(), strip.bottom(), "keys top");
            near(keys.bottom(), panel.bottom(), "keys bottom");
            near(
                keys.height(),
                regions.keyboard.height() - 3.0 * u,
                "keys height",
            );
            assert_eq!((keys.left(), keys.right()), (panel.left(), panel.right()));
            // Closed: no strip at all (it does not slide), and the keys have their whole band.
            let closed = strip_rect(panel, false, &m);
            assert_eq!((closed.height(), closed.top()), (0.0, strip.top()), "{ppp}");
            assert_eq!(keys_rect(panel, false, &m), regions.keyboard);
            // The screen's regions follow: open, the strip and the shorter keys; closed again, as they began.
            let open = regions.with_strip(true, &m);
            assert_eq!((open.strip, open.keyboard), (strip, keys), "{ppp}");
            assert_eq!(open.transport, regions.transport);
            let shut = open.with_strip(false, &m);
            assert_eq!((shut.strip, shut.keyboard), (closed, regions.keyboard));
            assert_eq!((regions.strip, regions.keyboard), (closed, shut.keyboard));
            // Hidden with the panel, the strip goes down with it, closed.
            let hidden = regions.without_panel();
            assert_eq!(hidden.strip.top(), hidden.keyboard.top());
            assert_eq!(hidden.strip.height(), 0.0);
        }
    }

    #[test]
    fn every_part_lies_inside_the_strip_at_both_densities() {
        for ppp in [1.0, 2.0] {
            let (panel, _, m) = bottom_panel(ppp);
            let strip = strip_rect(panel, true, &m);
            let l = layout(strip, Shown::of(true, KeyboardMode::Classic), &m);
            let circle = |c: Pos2, d: f32| Rect::from_center_size(c, vec2(d, d));
            for (what, r) in [
                ("reset", circle(l.reset, m.u(RESET_D))),
                ("key", l.key),
                ("scale", l.scale),
                ("mode", l.mode),
                ("wheels", circle(l.wheels, m.u(ICON_D))),
                ("two rows", circle(l.two_rows, m.u(ICON_D))),
            ] {
                assert!(strip.contains_rect(r), "{what} {r:?} outside {strip:?}");
            }
        }
    }

    #[test]
    fn the_parts_sit_left_to_right_centred_on_the_panel() {
        let m = metrics();
        let l = layout(panel(), Shown::of(true, KeyboardMode::Classic), &m);
        let y = panel().center().y;
        assert_eq!(l.reset, pos2(panel().left() + m.u(1.0), y));
        assert_eq!(l.key.left(), panel().left() + m.u(2.0));
        assert_eq!(l.scale.left(), l.key.right() + m.u(0.25));
        assert_eq!(l.mode.left(), l.scale.right() + m.u(0.25));
        for r in [l.key, l.scale, l.mode] {
            assert_eq!(r.center().y, y);
            assert_eq!(r.height(), m.u(2.75));
        }
        assert_eq!(
            [l.key.width(), l.scale.width(), l.mode.width()],
            [m.u(2.5), m.u(5.0), m.u(3.0)]
        );
        assert_eq!(l.wheels, pos2(l.mode.right() + m.u(1.0), y));
        assert_eq!(l.two_rows, pos2(l.wheels.x + m.u(1.5), y));
    }

    /// The settings strip of the 1170 × 540 screen at U = 32, open in the default 5 U panel (3 U under its 1.75 U
    /// button bar).
    fn panel() -> Rect {
        let m = metrics();
        Regions::new(SCREEN, &m).with_strip(true, &m).strip
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

        fn layout(&self) -> Layout {
            let s = &self.settings;
            layout(panel(), Shown::of(s.notes, s.mode), &metrics())
        }
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
            rig.click(chevrons(l.mode).1);
            assert_eq!(rig.settings.mode, want);
            assert_eq!(rig.actions.last(), Some(&A::Mode(want)));
        }
        assert!(rig.texts().iter().any(|t| t == "Strum"));
        rig.click(chevrons(l.mode).0);
        assert_eq!(rig.settings.mode, KeyboardMode::Chords);
        // The scale steps in reading order, column by column.
        rig.click(chevrons(l.scale).0);
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
        let m = Metrics::from_pixels_per_point(ppp, DEFAULT_UI_SCALE_INDEX);
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
                // The strip is placed whatever the panel's height, and opening it never resizes the panel.
                let strip = strip_rect(panel, open, &m);
                near(strip.top(), panel.top() + 1.75 * u, &what);
                near(strip.height(), if open { 3.0 * u } else { 0.0 }, &what);
                let keys = keys_rect(panel, open, &m);
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
                let area = pads_rect(keys_rect(panel, true, &m), &m);
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
    fn the_wheels_sit_under_the_bar_2_u_wide_at_most_5_u_tall_centred_when_capped_and_narrow_the_piano()
     {
        for ppp in [1.0, 2.0] {
            // (panel h, wheel h, wheel top under the panel top), in U.
            for (h, wheel, top) in [
                (6.0, 3.75, 1.75),
                (7.25, 5.0, 1.75),
                (9.25, 5.0, 2.75),
                (5.0, 2.75, 1.75),
            ] {
                let (panel, m) = panel_of(h, ppp);
                let u = m.u(1.0);
                let what = format!("{h} U, ppp {ppp}");
                let [wheel_mod, pitch] = wheel_rects(panel, &m);
                for (r, x) in [(wheel_mod, 0.0), (pitch, 2.0)] {
                    near(r.left(), panel.left() + x * u, &what);
                    near(r.width(), 2.0 * u, &what);
                    near(r.top(), panel.top() + top * u, &what);
                    near(r.height(), wheel * u, &what);
                }
                // They ignore the strip: the same rects with it open (they overlap it).
                let keys = keys_rect(panel, true, &m);
                assert!(wheel_mod.top() < keys.top(), "{what}");
                // With them shown, the piano starts 4 U right and is 4 U narrower.
                let r = piano_rect(keys, true, &m);
                near(r.left(), keys.left() + 4.0 * u, &what);
                near(r.width(), keys.width() - 4.0 * u, &what);
                assert_eq!((r.top(), r.bottom()), (keys.top(), keys.bottom()));
                assert_eq!(piano_rect(keys, false, &m), keys);
            }
        }
    }

    #[test]
    fn the_panel_height_stays_in_range_and_halves_its_way_to_the_target_snapping_within_2_px() {
        for ppp in [1.0, 2.0] {
            let m = Metrics::from_pixels_per_point(ppp, DEFAULT_UI_SCALE_INDEX);
            let u = m.u(1.0);
            // trunc(1.75 U) px … the screen height.
            let min = (1.75 * m.u_px()).trunc() / ppp;
            assert_eq!(panel_target(5.0, 540.0, &m), 5.0 * u);
            assert_eq!(panel_target(TWO_ROWS_H, 540.0, &m), 6.0 * u);
            assert_eq!(panel_target(0.0, 540.0, &m), min);
            assert_eq!(panel_target(100.0, 540.0, &m), 540.0);
            // h ← h + 0.5·(t − h) per tick, snapping within 2 px.
            let (from, to) = (5.0 * u, 6.0 * u);
            let mut h = from;
            let mut ticks = vec![];
            while h != to {
                h = panel_tick(h, to, &m);
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
            assert_eq!(panel_tick(to, from, &m), from + 0.5 * u);
            assert_eq!(panel_tick(to, to, &m), to);
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
        assert!(!rig.disc(l.wheels) && !rig.disc(l.two_rows));
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
        // The chord controls follow icon 12 left to right, each spinner or knob 0.25 U from the next.
        let u = metrics().u(1.0);
        assert_eq!(l.chord_set.left(), l.wheels.x + 0.75 * u);
        assert_eq!(l.octave.left(), l.chord_set.right() + 0.25 * u);
        assert_eq!(l.velocity.left(), l.octave.right() + 0.25 * u);
        assert_eq!(l.timing.left(), l.velocity.right() + 0.25 * u);
        assert_eq!(l.hold.x, l.timing.right() + 0.25 * u + 0.75 * u);
        assert!(panel().contains_rect(Rect::from_center_size(l.hold, vec2(1.25 * u, 1.25 * u))));
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
