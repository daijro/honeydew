//! The on-screen keyboard: key layout, octave labels and painting; one row, or two (two-keyboard mode).

use std::ops::RangeInclusive;

use eframe::egui::{self, Align2, Color32, Id, Pos2, Rangef, Rect, Sense, pos2, vec2};

use crate::metrics::Metrics;
use crate::settings::Settings;
use crate::ui::main_screen::TRANSPORT_H;
use crate::ui::piano_roll::notes::in_scale;
use crate::ui::{Finger, UiCtx, fingers, focus_lost, widgets};

/// A black key's width, as a share of a white key's (1.4 U on a 2 U white key).
const BLACK_KEY_W: f32 = 0.7;
/// Black key height, as a fraction of the keyboard height.
const BLACK_KEY_H: f32 = 0.6;
/// The panel height the second row needs to show, in U (the button bar included): more than this.
const TWO_ROWS_MIN: f32 = 5.0;

/// How touches play the keys.
#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub struct KeyInput {
    /// The keys take touches; while false every touch is ignored. It also stands for a channel that supports key-lock
    /// too: only while it is on are there root keys, with their markers and octave labels.
    pub enabled: bool,
    /// Key-lock, while it is on for a channel that supports it: the scale's root (0 = C) and its tone mask (most
    /// significant bit the root).
    pub lock: Option<(u8, u16)>,
    /// Keyboard kind 1 (`Scale`) with key-lock on: the out-of-scale keys are removed from the layout, and the scale's
    /// tones are laid out side by side as white keys, so every key counts as white.
    pub all_white: bool,
    /// The notes held on the channel (bit k = MIDI k) besides the fingers' keys: typed, touched and MIDI notes. They
    /// are live input: held, T = 1, in every row that shows them.
    pub notes: u128,
    /// The notes sounding at the channel's input from note events (bit k = MIDI k): song playback, previews (the
    /// engine's [`honeydew_audio::device::AudioRenderer::sounding`]). T = 0.7; held, T = 1 wins.
    pub sounding: u128,
    /// The channel's key range (low, high): with the scale test, what "in range" means. `None`: every key.
    pub range: Option<(u8, u8)>,
}

/// What the keys play this frame, each key with its press's velocity: the keys held, and the keys tapped (pressed and
/// lifted within the frame), which are never held.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct Played {
    pub held: Vec<(u8, f32)>,
    pub tapped: Vec<(u8, f32)>,
}

/// The keys' scroll and zoom, which both rows share.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct KeyView {
    /// The left edge, in white keys from MIDI 0. The `Scale` layout starts at the same note: its left edge is the
    /// scale's tones below the white key this one starts on ([`scale_left`]).
    pub left: f32,
    /// A white key's width, in U: 1 … 8.
    pub white_w: f32,
}

impl Default for KeyView {
    /// White key 28 (MIDI 48) at the left edge, white keys 2 U wide.
    fn default() -> Self {
        Self {
            left: 28.0,
            white_w: 2.0,
        }
    }
}

impl KeyView {
    /// The view kept in range for a row of `slots` white keys `width` points wide at `u` points per U: the white key
    /// width within 1 … 8 U, and the left edge within 0 … `slots` less the white keys the width shows (the keys never
    /// scroll past either end of the range; a width that shows them all keeps the edge at 0).
    pub fn clamped(self, slots: f32, width: f32, u: f32) -> Self {
        let white_w = self.white_w.clamp(1.0, 8.0);
        let last = (slots - width / (u * white_w)).max(0.0);
        Self {
            left: self.left.clamp(0.0, last),
            white_w,
        }
    }
}

/// A key: its MIDI number and horizontal extent in points from the keyboard's left edge.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct KeyboardKey {
    pub midi: u8,
    pub black: bool,
    pub x: Rangef,
}

impl KeyboardKey {
    /// The key's rect inside `keyboard`; black keys cover its top part only.
    pub fn rect(&self, keyboard: Rect) -> Rect {
        let bottom = if self.black {
            keyboard.top() + keyboard.height() * BLACK_KEY_H
        } else {
            keyboard.bottom()
        };
        let left = keyboard.left();
        Rect::from_x_y_ranges(
            left + self.x.min..=left + self.x.max,
            keyboard.top()..=bottom,
        )
    }
}

fn is_black(midi: u8) -> bool {
    matches!(midi % 12, 1 | 3 | 6 | 8 | 10)
}

/// Whether `midi` takes a white key's slot: a white key, or with a `scale` (root, tone mask; the `Scale` layout) a
/// tone of that scale.
fn slot_key(midi: u8, scale: Option<(u8, u16)>) -> bool {
    match scale {
        Some((root, mask)) => in_scale(midi, root, mask),
        None => !is_black(midi),
    }
}

/// How many white keys' slots the keys of `range` take.
pub fn slots(range: RangeInclusive<u8>, scale: Option<(u8, u16)>) -> f32 {
    range.filter(|&k| slot_key(k, scale)).count() as f32
}

/// The keys of the MIDI `range` that show in a row `width` points wide, at `u` points per U and the shared `view`
/// (its left edge in this layout's slots): the range's first key at the view's slot 0, so a row an octave up shows each key over the key an octave below it. A
/// white key takes one slot `view.white_w` U wide; a black key, 0.7 of that, is centred on the boundary of the white
/// keys around it. With a `scale` (the `Scale` layout) only the scale's tones are keys, each a white key's slot.
pub fn keyboard_keys(
    width: f32,
    u: f32,
    view: KeyView,
    range: RangeInclusive<u8>,
    scale: Option<(u8, u16)>,
) -> Vec<KeyboardKey> {
    let w = u * view.white_w;
    let (first, last) = (*range.start(), *range.end());
    // Left edge of the next slot, in slots from the view's left edge.
    let mut slot = -view.left;
    let mut keys = Vec::new();
    for midi in first..=last {
        let black = scale.is_none() && is_black(midi);
        let x = if black {
            Rangef::new(slot * w, slot * w).expand(BLACK_KEY_W * w / 2.0)
        } else if slot_key(midi, scale) {
            slot += 1.0;
            Rangef::new((slot - 1.0) * w, slot * w)
        } else {
            continue;
        };
        if x.max > 0.0 && x.min < width {
            keys.push(KeyboardKey { midi, black, x });
        }
    }
    keys
}

/// The rows of the keys rect `r`, top to bottom, each with its MIDI range: one row, MIDI 0–119, over all of `r`; or
/// two, `r` split into equal halves with no spacing, the top half ceil(U/16) px short at its bottom and the bottom half
/// starting ceil(U/16) px lower and as much shorter: the second row (MIDI 12–119) over the main one (MIDI 0–107).
pub fn rows(r: Rect, two: bool, m: &Metrics) -> Vec<(Rect, RangeInclusive<u8>)> {
    if !two {
        return vec![(r, 0..=119)];
    }
    let (mid, t) = (r.top() + r.height() / 2.0, m.t());
    vec![
        (r.with_max_y(mid - t), 12..=119),
        (r.with_min_y(mid + t), 0..=107),
    ]
}

/// Whether the second row shows: the keyboard is a piano (`Classic`, `Scale`, or an automation lane's piano), two rows
/// are `on`, and the keyboard `panel` (the button bar included) is over 5 U tall.
pub fn second_row(piano: bool, on: bool, panel: Rect, m: &Metrics) -> bool {
    piano && on && panel.height() > m.u(TWO_ROWS_MIN)
}

/// The piano's place: the keys rect R ([`crate::ui::keyboard_settings::piano_rect`]), whether the second row shows
/// ([`second_row`]), and the shared scroll and zoom.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Piano {
    pub rect: Rect,
    pub two_rows: bool,
    pub view: KeyView,
}

impl From<Rect> for Piano {
    /// One row over `rect`, at the default view.
    fn from(rect: Rect) -> Self {
        Self {
            rect,
            two_rows: false,
            view: KeyView::default(),
        }
    }
}

/// A row of keys: its rect and the keys that show in it.
struct Row {
    rect: Rect,
    keys: Vec<KeyboardKey>,
}

/// The `Scale` layout's left edge, in the scale's tones, for the piano's `left` (white keys from MIDI 0): the tones
/// below the white key it starts on, plus the same fraction of a key. So the default view starts both layouts at
/// MIDI 48 (C3's octave, `C4` by the labels).
fn scale_left(left: f32, scale: (u8, u16)) -> f32 {
    let white = left.max(0.0).floor();
    let midi = (0..=127)
        .filter(|&k| !is_black(k))
        .nth(white as usize)
        .unwrap_or(127);
    let below = (0..midi).filter(|&k| slot_key(k, Some(scale))).count() as f32;
    below + (left - white)
}

impl Piano {
    /// The view after the keys follow a finger `dx` points (`left −= dx`) and zoom by `zoom` (factor, anchor x; the key
    /// under the anchor stays put), kept in the main row's range. Its `left` counts piano white keys in every layout
    /// (in the `Scale` layout a slot moves as a white key does).
    pub fn moved(&self, dx: f32, zoom: Option<(f32, f32)>, m: &Metrics) -> KeyView {
        let u = m.u(1.0);
        let mut view = self.view;
        view.left -= dx / (u * view.white_w);
        if let Some((factor, x)) = zoom {
            let at = view.left + (x - self.rect.left()) / (u * view.white_w);
            view.white_w = (view.white_w * factor).clamp(1.0, 8.0);
            view.left = at - (x - self.rect.left()) / (u * view.white_w);
        }
        let main = rows(self.rect, self.two_rows, m)
            .pop()
            .map_or(0..=119, |r| r.1);
        view.clamped(slots(main, None), self.rect.width(), u)
    }

    /// The rows' keys for keyboard kind 1's `scale` layout (or the piano's with none), at the view kept in the main
    /// row's range. A row with no height has no keys to show or play.
    fn rows(&self, m: &Metrics, scale: Option<(u8, u16)>) -> Vec<Row> {
        let rows = rows(self.rect, self.two_rows, m);
        let main = rows.last().map_or(0..=119, |r| r.1.clone());
        let mut view = self.view;
        if let Some(scale) = scale {
            view.left = scale_left(view.left, scale);
        }
        let view = view.clamped(slots(main, scale), self.rect.width(), m.u(1.0));
        (rows.into_iter())
            .filter(|(rect, _)| rect.height() > 0.0)
            .map(|(rect, range)| Row {
                rect,
                keys: keyboard_keys(rect.width(), m.u(1.0), view, range, scale),
            })
            .collect()
    }
}

/// Label of the C key that starts `midi`'s octave. Middle C (MIDI 60) is `C5`, or `C4` when `middle_c_is_c4`.
pub fn octave_label(midi: u8, middle_c_is_c4: bool) -> String {
    let octave = i32::from(midi / 12) - i32::from(middle_c_is_c4);
    format!("C{octave}")
}

/// The key under `pos`: black keys lie on top of white keys.
pub fn key_at(keys: &[KeyboardKey], keyboard: Rect, pos: Pos2) -> Option<KeyboardKey> {
    let hit = |black: bool| {
        keys.iter()
            .find(|k| k.black == black && k.rect(keyboard).contains(pos))
    };
    hit(true).or_else(|| hit(false)).copied()
}

/// `key` under key-lock (the scale `mask` on `root`): itself when in the scale, else the nearest scale tone above it
/// when `up`, below it otherwise, or the other way at the end of the key range.
fn locked(key: u8, up: bool, root: u8, mask: u16) -> u8 {
    let tone = |&k: &u8| in_scale(k, root, mask);
    let (above, below) = ((key..=127).find(tone), (0..=key).rev().find(tone));
    let found = if up { above.or(below) } else { below.or(above) };
    found.unwrap_or(key)
}

/// The key a finger at `pos` plays: the key under it in the row it is on, snapped by key-lock toward the side of the
/// key's centre the finger is on (the right side from the centre itself); and its press velocity, (finger y − key
/// top) / key height, at least 1/127 (a velocity of 0 is no note).
fn play_at(rows: &[Row], pos: Pos2, input: KeyInput) -> Option<(u8, f32)> {
    let row = rows.iter().find(|r| r.rect.contains(pos))?;
    let key = key_at(&row.keys, row.rect, pos)?;
    let r = key.rect(row.rect);
    let midi = match input.lock {
        Some((root, mask)) => locked(key.midi, pos.x >= r.center().x, root, mask),
        None => key.midi,
    };
    Some((midi, ((pos.y - r.top()) / r.height()).max(1.0 / 127.0)))
}

/// A finger on the keys: the key it plays now (none while it is off the keys) and its press's velocity.
#[derive(Clone, Copy, Debug, PartialEq)]
struct Held {
    finger: u64,
    key: Option<u8>,
    velocity: f32,
}

/// Draws the keyboard over `piano` (a [`Piano`], or a rect for one row at the default view) and plays it. Every finger
/// ([`fingers`]) holds the key under it, at the velocity of its press; sliding onto another key, in either row, plays
/// that one at the same velocity (glissando), and off the keys (or between the rows) plays nothing. A finger pressed
/// and lifted within the frame is a tap. Both rows play the same way and return together: one live input. Keys held or
/// tapped, and the channel's notes on ([`KeyInput::notes`]), turn `channel` (the selected channel's colour) by the
/// press law ([`press_tick`]), in every row that shows them; the notes that only sound ([`KeyInput::sounding`]) aim at
/// 0.7 of the way. A focus loss lets go of every finger (the lift may never come), and so does a frame without the
/// keyboard (it never sees the lift). `settings` give the labels; `input` how touches play, and with kind 1's key-lock
/// ([`KeyInput::all_white`]) the scale's layout. The drawing is [`paint`]'s. The keys are clipped to R's left and right
/// edges only (never over the wheels; overflow below is drawn); a row with no height is not drawn.
pub fn keyboard(
    ui: &mut egui::Ui,
    piano: impl Into<Piano>,
    cx: &UiCtx,
    channel: Color32,
    settings: &Settings,
    input: KeyInput,
) -> Played {
    let piano = piano.into();
    let m = cx.m;
    let response = ui.interact(piano.rect, Id::new("keyboard"), Sense::click_and_drag());
    let scale = input.lock.filter(|_| input.all_white);
    let rows = piano.rows(m, scale);
    // The fingers as the last frame left them, if it drew the keyboard.
    let (state, pass) = (response.id.with("fingers"), ui.ctx().cumulative_pass_nr());
    let mut held = ui
        .data(|d| d.get_temp::<(u64, Vec<Held>)>(state))
        .filter(|(drawn, _)| drawn + 1 == pass && input.enabled && !ui.input(focus_lost))
        .map_or_else(Vec::new, |(_, held)| held);
    let mut played = Played::default();
    let mut fresh = Vec::new();
    for finger in fingers(ui, response.id)
        .into_iter()
        .filter(|_| input.enabled)
    {
        match finger {
            Finger::Down(id, pos) => {
                held.retain(|h| h.finger != id);
                if let Some((key, velocity)) = play_at(&rows, pos, input) {
                    held.push(Held {
                        finger: id,
                        key: Some(key),
                        velocity,
                    });
                    fresh.push(id);
                }
            }
            Finger::Move(id, pos) => {
                if let Some(h) = held.iter_mut().find(|h| h.finger == id) {
                    h.key = play_at(&rows, pos, input).map(|k| k.0);
                }
            }
            Finger::Up(id, _) => {
                let Some(i) = held.iter().position(|h| h.finger == id) else {
                    continue;
                };
                let h = held.remove(i);
                if let (Some(key), true) = (h.key, fresh.contains(&id)) {
                    played.tapped.push((key, h.velocity));
                }
            }
        }
    }
    for h in &held {
        if let Some(key) = h.key.filter(|&k| !played.held.iter().any(|p| p.0 == k)) {
            played.held.push((key, h.velocity));
        }
    }
    ui.data_mut(|d| d.insert_temp(state, (pass, held)));

    // Each note's press amount p, one draw on: live input, the fingers' keys and the channel's notes on (typed,
    // touched, MIDI), is held (T = 1); a note that only sounds at the channel's input, T = 0.7.
    let held =
        (played.held.iter().chain(&played.tapped)).fold(input.notes, |bits, p| bits | 1 << p.0);
    let target = |midi: u8| match (held >> midi & 1, input.sounding >> midi & 1) {
        (1, _) => 1.0,
        (_, 1) => 0.7,
        _ => 0.0,
    };
    let press_id = response.id.with("press");
    let press = ui.data_mut(|d| {
        let press = d.get_temp_mut_or(press_id, [0.0f32; 128]);
        for (midi, p) in (0..=127).zip(press.iter_mut()) {
            *p = press_tick(*p, target(midi));
        }
        *press
    });
    // While a note only sounds (no live key holds it), the frames poll the engine for its note-off.
    if input.sounding & !held != 0 || (0..=127).zip(press).any(|(midi, p)| p != target(midi)) {
        ui.ctx().request_repaint();
    }
    // Clipped to R's sides only (a key partly shown never draws over the wheels); below, overflow is not clipped.
    let sides = Rect::from_x_y_ranges(piano.rect.x_range(), Rangef::EVERYTHING);
    let painter = ui.painter().with_clip_rect(ui.clip_rect().intersect(sides));
    let lit = held | input.sounding;
    paint(&painter, &rows, cx, channel, settings, input, &press, lit);
    played
}

/// Every finger on the keys is let go, their notes silenced (Preview on or off): one still down plays nothing until it
/// lifts and presses again.
pub fn let_go(ctx: &egui::Context) {
    let state = Id::new("keyboard").with("fingers");
    ctx.data_mut(|d| d.remove::<(u64, Vec<Held>)>(state));
}

/// Paints the `rows`, each key at its `press` amount p, each fill c as mix(c) = `channel`·p + c·(1 − p). Each row's
/// rect is filled with `controlFill`; on a row 0.5 U tall or more, first every white key (step 1 in A, step 2 in B, the
/// separators in `background`), then the black keys (step 1 in A; step 2 while in range or its note is `sounding`, bit
/// k = MIDI k, T > 0) and every label, in ascending key order. A is the channel colour on a root key, else B:
/// `whiteKey` / `blackKey` in range, `whiteKeyDisabled` / `blackKeyDisabled` out of it; but a white key's B is
/// `whiteKey` whatever its range while key-lock is on in `Scale` mode ([`KeyInput::all_white`]).
///
/// - Roots: the keys of the project key's pitch class (C under Chromatic), only for a channel that supports key-lock
///   (the keys play a channel, [`KeyInput::enabled`]; the app's key-lock takes every channel).
/// - In range: in the scale while key-lock is on ([`KeyInput::lock`], set while the scale is not Chromatic), and in
///   the channel's key range ([`KeyInput::range`]).
/// - Labels, 12 pt shrunk to fit and placed centre-bottom: a root's [`key_name`] with its octave always; any other
///   key's name while key names are on, it is in range, and it is not a black key under Chromatic. In
///   `blackKeyDisabled` on a white root and other black keys, else `whiteKeyDisabled`; labels keep that ink as the key lights.
/// - Not drawn: the grab inset (m = 0: nothing grabs the keys; the bar's scroll gesture is not built) and the range
///   borders of `showRangeIndexLabels`.
#[allow(clippy::too_many_arguments)]
fn paint(
    painter: &egui::Painter,
    rows: &[Row],
    cx: &UiCtx,
    channel: Color32,
    settings: &Settings,
    input: KeyInput,
    press: &[f32; 128],
    sounding: u128,
) {
    let (colors, m) = (&cx.theme.colors, cx.m);
    let (ppp, u) = (m.ppp(), m.u_px());
    let [white, white_off, black, black_off, bg] = [
        colors.white_key,
        colors.white_key_disabled,
        colors.black_key,
        colors.black_key_disabled,
        colors.background,
    ]
    .map(|c| c.to_color32());
    let fill = |r: Rect, color: Color32| painter.rect_filled(r / ppp, 0.0, color);
    let p = |midi: u8| press[usize::from(midi)].clamp(0.0, 1.0);
    let mix = |c: Color32, midi: u8| c.lerp_to_gamma(channel, p(midi));
    let is_root = |midi: u8| input.enabled && midi % 12 == input.lock.map_or(0, |l| l.0);
    let in_range = |midi: u8| {
        input
            .lock
            .is_none_or(|(root, mask)| in_scale(midi, root, mask))
            && input
                .range
                .is_none_or(|(low, high)| (low..=high).contains(&midi))
    };
    for row in rows {
        painter.rect_filled(row.rect, 0.0, colors.control_fill.to_color32());
        if row.rect.height() < m.u(0.5) {
            continue;
        }
        let rect = row.rect * ppp;
        let x =
            |k: &KeyboardKey| Rangef::new(rect.left() + k.x.min * ppp, rect.left() + k.x.max * ppp);
        let y = rect.y_range();
        for key in row.keys.iter().filter(|k| !k.black) {
            let (midi, r) = (key.midi, white_px(x(key), y, u));
            let a = match (is_root(midi), in_range(midi)) {
                (true, _) => channel,
                (false, true) => white,
                (false, false) => white_off,
            };
            // Always `whiteKey` while key-lock is on in `Scale` mode.
            let b = if input.all_white || in_range(midi) {
                white
            } else {
                white_off
            };
            fill(r.body, mix(a, midi));
            fill(r.upper, mix(b, midi));
            for sep in r.seps {
                fill(sep, bg);
            }
        }
        for key in &row.keys {
            let (midi, root, range) = (key.midi, is_root(key.midi), in_range(key.midi));
            let (label, color) = if key.black {
                let r = black_px(x(key), y, u, p(midi));
                let b = if range { black } else { black_off };
                fill(r.body, mix(if root { channel } else { b }, midi));
                if range || sounding >> midi & 1 == 1 {
                    fill(r.border, bg);
                    fill(r.inner, mix(b, midi));
                }
                (r.label, if root { white_off } else { black_off })
            } else {
                let label = white_px(x(key), y, u).label;
                (label, if root { black_off } else { white_off })
            };
            let named = settings.key_names && range && !(key.black && input.lock.is_none());
            if root || named {
                let text = key_name(midi, root, settings.middle_c_is_c4);
                let at = label / ppp;
                widgets::shrink(painter, m, at, Align2::CENTER_BOTTOM, &text, 12, color);
            }
        }
    }
}

/// One draw of a key's press amount `p` toward its target T: p + k·(T − p), k = 0.9 while rising, else 0.175. Within
/// 0.001 of T it lands on T (invisible, and the frames it asks for stop).
pub fn press_tick(p: f32, target: f32) -> f32 {
    let k = if target > p { 0.9 } else { 0.175 };
    let p = p + k * (target - p);
    if (target - p).abs() < 0.001 {
        target
    } else {
        p
    }
}

/// A white key's rects in device pixels, for its extent `x` and `y` (px, unrounded) at `u` px per U: the fill (step 1),
/// the overpaint from the top to trunc(bottom − 0.1 U) (step 2; below it the ceil(0.1 U) marker strip keeps the fill's
/// colour), the 2 px separators centred on its left and right edges, and the label's rect.
#[derive(Debug, PartialEq)]
struct WhitePx {
    body: Rect,
    upper: Rect,
    seps: [Rect; 2],
    label: Rect,
}

fn white_px(x: Rangef, y: Rangef, u: f32) -> WhitePx {
    let body = Rect::from_x_y_ranges(x.min.trunc()..=x.max.trunc(), y.min.trunc()..=y.max.trunc());
    let sep = |e: f32| Rect::from_x_y_ranges(e - 1.0..=e + 1.0, body.y_range());
    WhitePx {
        body,
        upper: body.with_max_y((y.max - 0.1 * u).trunc()),
        seps: [sep(body.left()), sep(body.right())],
        label: Rect::from_x_y_ranges(x, y.min..=y.max - 0.1 * u),
    }
}

/// A black key's rects in device pixels, for its extent `x` (0.7 W centred on its boundary b) and the keys' `y` (px,
/// unrounded) at `u` px per U and press amount `p`: the fill (step 1), trunc(b ± 0.35 W) wide and trunc(0.6 H′) tall
/// from trunc(top); the upper part's `background` border and its inner fill (step 2),
/// t = trunc(top + h − 1 − 0.1 U + 0.08 U·p) − top tall, which leave a lip of the fill's colour below them; and the
/// label's rect.
#[derive(Debug, PartialEq)]
struct BlackPx {
    body: Rect,
    border: Rect,
    inner: Rect,
    label: Rect,
}

fn black_px(x: Rangef, y: Rangef, u: f32, p: f32) -> BlackPx {
    let top = y.min.trunc();
    let bottom = (y.min + BLACK_KEY_H * y.span()).trunc();
    let body = Rect::from_x_y_ranges(x.min.trunc()..=x.max.trunc(), top..=bottom);
    let border = body.with_max_y((bottom - 1.0 - 0.1 * u + 0.08 * u * p).trunc());
    let inner = border.shrink2(vec2(1.0, 0.0));
    BlackPx {
        body,
        border,
        inner,
        label: inner.with_max_y(inner.bottom() - 0.1 * u),
    }
}

/// A key's label: its note name, with sharps; a root key's adds its octave, floor(n′/12), where n′ is the MIDI number
/// less 12 when `middle_c_is_c4` (so MIDI 60 is `C5`, or `C4`).
pub fn key_name(midi: u8, root: bool, middle_c_is_c4: bool) -> String {
    const NAMES: [&str; 12] = [
        "C", "C#", "D", "D#", "E", "F", "F#", "G", "G#", "A", "A#", "B",
    ];
    let name = NAMES[usize::from(midi % 12)];
    if !root {
        return name.to_owned();
    }
    let n = i32::from(midi) - 12 * i32::from(middle_c_is_c4);
    format!("{name}{}", n.div_euclid(12))
}

/// The bottom keyboard panel's backdrop: `controlFill` over the whole `panel` (the bar, the keys and the pads alike);
/// while the `wheels` show, `moduleFill` behind them over (panel x, panel top + 1.75 U, 4 U, panel h), past the panel's
/// bottom; and a 1 px `background` line across the panel's top edge (inside it).
pub fn panel_backdrop(painter: &egui::Painter, panel: Rect, wheels: bool, cx: &UiCtx) {
    let (colors, m) = (&cx.theme.colors, cx.m);
    painter.rect_filled(panel, 0.0, colors.control_fill.to_color32());
    if wheels {
        let top = panel.top() + m.u(TRANSPORT_H);
        let column = Rect::from_min_size(pos2(panel.left(), top), vec2(m.u(4.0), panel.height()));
        painter.rect_filled(column, 0.0, colors.module_fill.to_color32());
    }
    let line = panel.with_max_y(panel.top() + 1.0 / m.ppp());
    painter.rect_filled(line, 0.0, colors.background.to_color32());
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::metrics::{DEFAULT_UI_SCALE_INDEX, Metrics};
    use crate::song::scales::SCALE_MASKS;
    use crate::theme::theme_by_id;
    use eframe::egui::{
        Event, PointerButton, RawInput, TouchDeviceId, TouchId, TouchPhase, pos2, vec2,
    };
    use std::time::Duration;

    const U: f32 = 32.0;
    /// The keyboard: 135 points tall, its black keys 81.
    const KEYBOARD: Rect = Rect {
        min: pos2(0.0, 405.0),
        max: pos2(1170.0, 540.0),
    };
    /// C major.
    const MAJOR: u16 = SCALE_MASKS[0];
    const PLAY: KeyInput = KeyInput {
        enabled: true,
        lock: None,
        all_white: false,
        notes: 0,
        sounding: 0,
        range: None,
    };

    /// A frame's input: its events, whether the keyboard is drawn, a widget over the keyboard's left half, and the
    /// channel's notes on (added to the run's).
    struct Frame {
        events: Vec<Event>,
        drawn: bool,
        cover: bool,
        notes: u128,
    }

    impl From<Vec<Event>> for Frame {
        fn from(events: Vec<Event>) -> Self {
            Self {
                events,
                drawn: true,
                cover: false,
                notes: 0,
            }
        }
    }

    /// Runs the keyboard in an offscreen context, one frame per entry of `frames`, played as `input`; returns what
    /// each frame played (nothing while not drawn) and its output.
    fn run_with(input: KeyInput, frames: Vec<Frame>) -> Vec<(Played, egui::FullOutput)> {
        run_on(KEYBOARD.into(), input, frames)
    }

    /// [`run_with`] over `piano`.
    fn run_on(
        piano: Piano,
        input: KeyInput,
        frames: Vec<Frame>,
    ) -> Vec<(Played, egui::FullOutput)> {
        let ctx = egui::Context::default();
        let screen = Rect::from_min_size(Pos2::ZERO, vec2(1170.0, 540.0));
        let theme = theme_by_id(0);
        let m = Metrics::from_pixels_per_point(1.0, DEFAULT_UI_SCALE_INDEX);
        let mut all = Vec::new();
        for frame in frames {
            let raw = RawInput {
                screen_rect: Some(screen),
                events: frame.events,
                ..Default::default()
            };
            let mut played = Played::default();
            let out = ctx.run(raw, |ctx| {
                egui::CentralPanel::default().show(ctx, |ui| {
                    let cx = UiCtx {
                        theme: &theme,
                        m: &m,
                    };
                    if frame.drawn {
                        let input = KeyInput {
                            notes: input.notes | frame.notes,
                            ..input
                        };
                        played =
                            keyboard(ui, piano, &cx, Color32::WHITE, &Settings::default(), input);
                    }
                    if frame.cover {
                        let left = KEYBOARD.with_max_x(KEYBOARD.center().x);
                        ui.interact(left, Id::new("cover"), Sense::click());
                    }
                });
            });
            all.push((played, out));
        }
        all
    }

    /// [`run_with`] for plain frames, the keys enabled and unlocked; returns what each frame played.
    fn run(frames: Vec<Vec<Event>>) -> Vec<Played> {
        played(PLAY, frames)
    }

    fn played(input: KeyInput, frames: Vec<Vec<Event>>) -> Vec<Played> {
        let frames = frames.into_iter().map(Frame::from).collect();
        run_with(input, frames).into_iter().map(|f| f.0).collect()
    }

    /// Each frame's held and tapped keys, without their velocities.
    fn keys(played: &[Played]) -> Vec<(Vec<u8>, Vec<u8>)> {
        let midi = |v: &[(u8, f32)]| v.iter().map(|k| k.0).collect();
        played
            .iter()
            .map(|p| (midi(&p.held), midi(&p.tapped)))
            .collect()
    }

    /// E4, near the keyboard's bottom: velocity 115 / 135.
    const E4: Pos2 = pos2(5.0 * U, 520.0);
    const E4_VELOCITY: f32 = 115.0 / 135.0;

    fn button(button: PointerButton, pressed: bool) -> Event {
        Event::PointerButton {
            pos: E4,
            button,
            pressed,
            modifiers: Default::default(),
        }
    }

    fn touch(id: u64, phase: TouchPhase, pos: Pos2) -> Event {
        Event::Touch {
            device_id: TouchDeviceId(0),
            id: TouchId(id),
            phase,
            pos,
            force: None,
        }
    }

    /// Presses `button` on E4 and keeps holding it; returns the held keys.
    fn held_after_press(b: PointerButton) -> Vec<u8> {
        // Frame 0 registers the widget, frame 1 presses, frame 2 keeps holding.
        let frames = vec![
            vec![],
            vec![Event::PointerMoved(E4), button(b, true)],
            vec![],
        ];
        keys(&run(frames))[2].0.clone()
    }

    fn near(a: f32, b: f32) -> bool {
        (a - b).abs() < 1e-4
    }

    #[test]
    fn a_tap_within_one_frame_is_a_tap_of_its_key_and_never_held() {
        let none = (vec![], vec![]);
        let tap = (vec![], vec![52]);
        // The press and release arrive together: E4 is tapped in that frame (the app sounds it for a short gate).
        let frames = vec![
            vec![],
            vec![Event::PointerMoved(E4)],
            vec![
                button(PointerButton::Primary, true),
                button(PointerButton::Primary, false),
            ],
            vec![],
        ];
        let played = run(frames);
        assert_eq!(
            keys(&played),
            [none.clone(), none.clone(), tap.clone(), none.clone()]
        );
        assert!(near(played[2].tapped[0].1, E4_VELOCITY));
        // The same on touch, where the finger lands and lifts within the frame and the pointer then goes.
        let frames = vec![
            vec![],
            vec![
                touch(1, TouchPhase::Start, E4),
                touch(1, TouchPhase::End, E4),
            ],
            vec![],
        ];
        assert_eq!(keys(&run(frames)), [none.clone(), tap, none.clone()]);
        // A two-frame click is held on its press frame, and is no tap on its release frame.
        let frames = vec![
            vec![],
            vec![
                Event::PointerMoved(E4),
                button(PointerButton::Primary, true),
            ],
            vec![button(PointerButton::Primary, false)],
        ];
        assert_eq!(
            keys(&run(frames)),
            [none.clone(), (vec![52], vec![]), none.clone()]
        );
        // A one-frame secondary click plays nothing.
        let frames = vec![
            vec![],
            vec![Event::PointerMoved(E4)],
            vec![
                button(PointerButton::Secondary, true),
                button(PointerButton::Secondary, false),
            ],
        ];
        assert_eq!(keys(&run(frames)), [none.clone(), none.clone(), none]);
    }

    #[test]
    fn a_press_held_over_a_focus_loss_is_not_held_until_a_new_press_and_its_key_fades() {
        let press = vec![
            Event::PointerMoved(E4),
            button(PointerButton::Primary, true),
        ];
        // egui keeps the button down when the focus goes; the release is lost with it.
        let frames = vec![
            vec![],
            press.clone(),
            vec![Event::WindowFocused(false)],
            vec![Event::WindowFocused(true)],
            vec![Event::PointerMoved(E4 + vec2(U, 0.0))],
            press,
        ];
        let frames = frames.into_iter().map(Frame::from).collect();
        let e4 = pos2(5.0 * U, 450.0);
        let seen: Vec<(Vec<u8>, Option<Color32>)> = run_with(PLAY, frames)
            .into_iter()
            .map(|(played, out)| {
                let shapes: Vec<egui::Shape> = out.shapes.into_iter().map(|s| s.shape).collect();
                (keys(&[played])[0].0.clone(), top_fill(&shapes, e4))
            })
            .collect();
        // Once the focus goes, E4 is let go of: its press amount falls until the new press.
        let white = theme_by_id(0).colors.white_key.to_color32();
        let mut p = 0.0;
        let want: Vec<(Vec<u8>, Option<Color32>)> = [false, true, false, false, false, true]
            .map(|held| {
                p = press_tick(p, if held { 1.0 } else { 0.0 });
                let keys = if held { vec![52] } else { vec![] };
                (keys, Some(white.lerp_to_gamma(Color32::WHITE, p)))
            })
            .into();
        assert_eq!(seen, want);
    }

    /// Whether the frame drew a key pressed: `whiteKey` 0.9 of the way to the channel colour, a fresh press.
    fn lit(out: &egui::FullOutput) -> bool {
        let white = theme_by_id(0).colors.white_key.to_color32();
        let pressed = white.lerp_to_gamma(Color32::WHITE, 0.9);
        (out.shapes.iter()).any(|s| matches!(&s.shape, egui::Shape::Rect(r) if r.fill == pressed))
    }

    #[test]
    fn only_the_primary_button_holds_a_key() {
        assert_eq!(held_after_press(PointerButton::Primary), [52]);
        assert!(held_after_press(PointerButton::Secondary).is_empty());
        assert!(held_after_press(PointerButton::Middle).is_empty());
    }

    #[test]
    fn a_fresh_press_is_as_loud_as_it_is_deep_on_its_key() {
        // Half way down E (white, 135 tall) and C# (black, 81 tall); near E's bottom.
        for (at, key, velocity) in [
            (pos2(5.0 * U, 472.5), 52, 0.5),
            (pos2(2.0 * U, 445.5), 49, 0.5),
            (E4, 52, E4_VELOCITY),
        ] {
            let played = run(vec![vec![], vec![touch(1, TouchPhase::Start, at)]]);
            let held = &played[1].held;
            assert_eq!(held.len(), 1);
            assert_eq!(held[0].0, key);
            assert!(near(held[0].1, velocity), "{at:?}: {held:?}");
        }
    }

    #[test]
    fn a_press_on_a_keys_top_edge_plays_at_the_least_velocity() {
        for at in [pos2(5.0 * U, KEYBOARD.top()), pos2(2.0 * U, KEYBOARD.top())] {
            let played = run(vec![vec![], vec![touch(1, TouchPhase::Start, at)]]);
            assert_eq!(played[1].held.len(), 1);
            assert!(near(played[1].held[0].1, 1.0 / 127.0), "{at:?}: {played:?}");
        }
    }

    #[test]
    fn a_sliding_finger_plays_each_key_it_reaches_at_its_press_velocity() {
        let (e, f, g) = (
            pos2(5.0 * U, 472.5),
            pos2(7.0 * U, 530.0),
            pos2(9.0 * U, 420.0),
        );
        let mv = |pos| vec![touch(1, TouchPhase::Move, pos)];
        let played = run(vec![
            vec![],
            vec![touch(1, TouchPhase::Start, e)],
            mv(f),
            // Off the keys: the finger plays nothing, and back on, it plays G at its press velocity.
            mv(pos2(7.0 * U, 300.0)),
            mv(g),
            vec![touch(1, TouchPhase::End, g)],
        ]);
        let held: Vec<Vec<(u8, f32)>> = played.into_iter().map(|p| p.held).collect();
        assert_eq!(
            held,
            [
                vec![],
                vec![(52, 0.5)],
                vec![(53, 0.5)],
                vec![],
                vec![(55, 0.5)],
                vec![]
            ]
        );
    }

    #[test]
    fn every_finger_holds_its_own_key() {
        let (c, e) = (pos2(U, 472.5), pos2(5.0 * U, 532.0));
        let played = run(vec![
            vec![],
            vec![touch(1, TouchPhase::Start, c)],
            vec![touch(2, TouchPhase::Start, e)],
            // A third finger taps D within the frame; the others hold on.
            vec![
                touch(3, TouchPhase::Start, pos2(3.0 * U, 472.5)),
                touch(3, TouchPhase::End, pos2(3.0 * U, 472.5)),
            ],
            vec![touch(1, TouchPhase::End, c)],
            vec![touch(2, TouchPhase::End, e)],
        ]);
        let deep = 127.0 / 135.0;
        let held: Vec<Vec<(u8, f32)>> = played.iter().map(|p| p.held.clone()).collect();
        assert_eq!(
            held,
            [
                vec![],
                vec![(48, 0.5)],
                vec![(48, 0.5), (52, deep)],
                vec![(48, 0.5), (52, deep)],
                vec![(52, deep)],
                vec![]
            ]
        );
        assert_eq!(played[3].tapped, [(50, 0.5)]);
    }

    #[test]
    fn the_pointer_echo_of_a_touch_is_no_finger_of_its_own() {
        // The integrations repeat the first touch as the pointer; here it lands elsewhere, to tell them apart.
        let echo = pos2(9.0 * U, 472.5);
        let press = Event::PointerButton {
            pos: echo,
            button: PointerButton::Primary,
            pressed: true,
            modifiers: Default::default(),
        };
        let played = run(vec![
            vec![],
            vec![
                touch(1, TouchPhase::Start, E4),
                Event::PointerMoved(echo),
                press,
            ],
            vec![touch(1, TouchPhase::Move, E4), Event::PointerMoved(echo)],
        ]);
        assert_eq!(keys(&played)[1..], [(vec![52], vec![]), (vec![52], vec![])]);
    }

    #[test]
    fn key_lock_snaps_an_out_of_scale_key_toward_the_fingers_side_of_its_centre() {
        let input = KeyInput {
            lock: Some((0, MAJOR)),
            ..PLAY
        };
        // C# (centred at 2 U) → C on its left half, D on its right; E is in the scale; F# (8 U) → F or G.
        for (x, y, key) in [
            (1.8, 420.0, 48),
            (2.2, 420.0, 50),
            (5.0, 520.0, 52),
            (7.9, 420.0, 53),
            (8.1, 420.0, 55),
        ] {
            let played = played(
                input,
                vec![vec![], vec![touch(1, TouchPhase::Start, pos2(x * U, y))]],
            );
            assert_eq!(keys(&played)[1].0, [key], "{x} U");
        }
        // A finger sliding across C#'s centre moves from C to D.
        let played = played(
            input,
            vec![
                vec![],
                vec![touch(1, TouchPhase::Start, pos2(1.8 * U, 420.0))],
                vec![touch(1, TouchPhase::Move, pos2(2.2 * U, 420.0))],
            ],
        );
        assert_eq!(keys(&played)[1..], [(vec![48], vec![]), (vec![50], vec![])]);
    }

    #[test]
    fn a_locked_key_goes_the_other_way_at_the_end_of_the_range() {
        // A scale of C alone: up from G9 (127) finds nothing, so it goes down to C9.
        let c_only = 0b1000_0000_0000;
        assert_eq!(locked(127, true, 0, c_only), 120);
        assert_eq!(locked(1, false, 0, c_only), 0);
        assert_eq!(locked(1, true, 0, c_only), 12);
        // D major: F → F# up, E down.
        assert_eq!(locked(65, true, 2, MAJOR), 66);
        assert_eq!(locked(65, false, 2, MAJOR), 64);
    }

    #[test]
    fn disabled_keys_ignore_touches() {
        let input = KeyInput {
            enabled: false,
            ..PLAY
        };
        let frames = vec![
            vec![].into(),
            vec![touch(1, TouchPhase::Start, E4)].into(),
            vec![touch(1, TouchPhase::End, E4)].into(),
        ];
        for (played, out) in run_with(input, frames) {
            assert_eq!(played, Played::default());
            assert!(!lit(&out));
        }
    }

    #[test]
    fn a_key_under_another_widget_takes_no_touch() {
        let covered = |events: Vec<Event>| Frame {
            events,
            drawn: true,
            cover: true,
            notes: 0,
        };
        let right = pos2(31.0 * U, 472.5);
        let frames = vec![
            covered(vec![]),
            covered(vec![touch(1, TouchPhase::Start, E4)]),
            covered(vec![touch(2, TouchPhase::Start, right)]),
        ];
        let played: Vec<Played> = run_with(PLAY, frames).into_iter().map(|f| f.0).collect();
        assert_eq!(keys(&played)[1..], [(vec![], vec![]), (vec![74], vec![])]);
    }

    #[test]
    fn a_keyboard_not_drawn_forgets_its_fingers() {
        let hidden = Frame {
            events: vec![],
            drawn: false,
            cover: false,
            notes: 0,
        };
        let frames = vec![
            vec![].into(),
            vec![touch(1, TouchPhase::Start, E4)].into(),
            hidden,
            vec![].into(),
        ];
        let played: Vec<Played> = run_with(PLAY, frames).into_iter().map(|f| f.0).collect();
        assert_eq!(keys(&played)[1].0, [52]);
        assert_eq!(played[3], Played::default());
    }

    #[test]
    fn in_the_scale_kind_with_key_lock_only_the_scales_tones_are_keys_and_all_count_as_white() {
        // Kind 1 with key-lock removes the out-of-scale keys from the layout. D major: C# D E F# G A B, seven to the
        // octave, so slot 28 is the first tone from MIDI 48, C#.
        let d_major = Some((2, MAJOR));
        let keys = keyboard_keys(KEYBOARD.width(), U, KeyView::default(), 0..=119, d_major);
        let midis: Vec<u8> = keys.iter().take(8).map(|k| k.midi).collect();
        assert_eq!(midis, [49, 50, 52, 54, 55, 57, 59, 61]);
        for (i, k) in keys.iter().enumerate() {
            assert!(!k.black, "{k:?}");
            assert_eq!(
                k.x,
                Rangef::new(i as f32 * 2.0 * U, (i + 1) as f32 * 2.0 * U)
            );
            assert!(in_scale(k.midi, 2, MAJOR));
        }
        // Where the piano has C#'s right edge over D, the layout has D, full height (every key counts as white).
        let overlap = pos2(2.0 * U + 1.0, 410.0);
        assert_eq!(key_at(&keys, KEYBOARD, overlap).map(|k| k.midi), Some(50));
        // The range: 70 slots (10 octaves of 7 tones) in one row, 63 in each of two.
        assert_eq!(slots(0..=119, d_major), 70.0);
        assert_eq!(slots(0..=107, d_major), 63.0);
        assert_eq!(
            slots(0..=119, Some((0, SCALE_MASKS[3]))),
            50.0,
            "a pentatonic"
        );
    }

    #[test]
    fn the_scale_layout_starts_at_midi_48_like_the_piano_whatever_the_scale() {
        // The default view's left edge is white key 28 (MIDI 48); the scale layout starts at the first tone from 48,
        // not at its 28th tone (which a pentatonic would put 5.6 octaves up).
        let m = Metrics::from_pixels_per_point(1.0, DEFAULT_UI_SCALE_INDEX);
        let piano = Piano::from(KEYBOARD);
        for (scale, first) in [
            ((0, MAJOR), 48),
            ((2, MAJOR), 49),
            ((0, SCALE_MASKS[3]), 48),
            ((1, SCALE_MASKS[3]), 49),
        ] {
            let rows = piano.rows(&m, Some(scale));
            assert_eq!(rows[0].keys[0].midi, first, "{scale:?}");
            assert_eq!(rows[0].keys[0].x.min, 0.0, "{scale:?}");
            // `slots(0..=47)`: the tones below 48.
            assert_eq!(
                scale_left(28.0, scale),
                slots(0..=47, Some(scale)),
                "{scale:?}"
            );
        }
        // A fraction of a key carries over.
        assert_eq!(scale_left(28.5, (0, SCALE_MASKS[3])), 20.5);
    }

    #[test]
    fn a_key_partly_shown_is_clipped_to_the_keys_sides_only() {
        // R starts 4 U in (the wheels' place) at a scroll that cuts the first key: nothing drawn left of R.
        let rect = KEYBOARD.with_min_x(4.0 * U);
        let piano = Piano {
            view: KeyView {
                left: 28.5,
                white_w: 2.0,
            },
            ..rect.into()
        };
        let out = run_on(piano, PLAY, vec![vec![].into()]);
        // The keys' shapes: every rect but the panel's own background over the whole screen.
        let screen = Rect::from_min_size(Pos2::ZERO, vec2(1170.0, 540.0));
        let clips: Vec<Rect> = (out[0].1.shapes.iter())
            .filter(|s| matches!(&s.shape, egui::Shape::Rect(r) if r.rect != screen))
            .map(|s| s.clip_rect)
            .collect();
        assert!(!clips.is_empty());
        for clip in clips {
            assert_eq!(clip.x_range(), rect.x_range());
            assert!(
                clip.bottom() >= 540.0,
                "not clipped at R's bottom: {clip:?}"
            );
        }
    }

    #[test]
    fn the_scale_kind_plays_and_draws_the_scale_layout_only_with_key_lock() {
        let black = theme_by_id(0).colors.black_key.to_color32();
        // Black keys: 0.6 of the row tall (the row's background may share their colour).
        let blacks = |out: &egui::FullOutput| {
            (out.shapes.iter())
                .filter(|s| matches!(&s.shape, egui::Shape::Rect(r) if r.fill == black && r.rect.height() < 100.0))
                .count()
        };
        let tap = |x: f32| vec![touch(1, TouchPhase::Start, pos2(x * U, 500.0))];
        // Kind 1 with key-lock on D major: no black keys drawn, and the second slot is D.
        let input = KeyInput {
            lock: Some((2, MAJOR)),
            all_white: true,
            ..PLAY
        };
        let out = run_with(input, vec![vec![].into(), tap(3.0).into()]);
        assert_eq!(blacks(&out[1].1), 0);
        assert_eq!(keys(&[out[1].0.clone()])[0].0, [50]);
        // Key-lock in another kind: the piano's layout, its black keys drawn, the touch snapped (E3 stays E3).
        let input = KeyInput {
            all_white: false,
            ..input
        };
        let out = run_with(input, vec![vec![].into(), tap(5.0).into()]);
        assert!(blacks(&out[1].1) > 0);
        assert_eq!(keys(&[out[1].0.clone()])[0].0, [52]);
    }

    /// A two-row piano over [`KEYBOARD`] (135 points): rows 405 … 470.5 and 474.5 … 540 (ceil(32/16) = 2 points
    /// either side of the middle).
    fn two_rows() -> Piano {
        Piano {
            two_rows: true,
            ..KEYBOARD.into()
        }
    }

    #[test]
    fn two_rows_split_the_keys_rect_into_halves_ceil_u_16_px_apart_at_ppp_1_and_2() {
        for ppp in [1.0, 2.0] {
            let m = Metrics::from_pixels_per_point(ppp, DEFAULT_UI_SCALE_INDEX);
            let t = (m.u_px() / 16.0).ceil() / ppp;
            for r in [
                KEYBOARD,
                Rect::from_min_size(pos2(4.0 * U, 300.0), vec2(800.0, 3.3 * U)),
            ] {
                let one = rows(r, false, &m);
                assert_eq!(one, [(r, 0..=119)]);
                let [(top, top_range), (bottom, bottom_range)] =
                    rows(r, true, &m).try_into().unwrap();
                let half = r.height() / 2.0;
                assert_eq!((top_range, bottom_range), (12..=119, 0..=107));
                assert_eq!((top.top(), top.x_range()), (r.top(), r.x_range()));
                assert_eq!(
                    (bottom.bottom(), bottom.x_range()),
                    (r.bottom(), r.x_range())
                );
                assert!((top.height() - (half - t)).abs() < 1e-3, "ppp {ppp}");
                assert!((bottom.height() - (half - t)).abs() < 1e-3, "ppp {ppp}");
                assert!(
                    (bottom.top() - top.bottom() - 2.0 * t).abs() < 1e-3,
                    "ppp {ppp}"
                );
            }
        }
    }

    #[test]
    fn the_second_row_shows_for_a_piano_with_two_rows_on_over_a_5_u_panel() {
        let m = Metrics::from_pixels_per_point(1.0, DEFAULT_UI_SCALE_INDEX);
        let panel = |h: f32| Rect::from_min_size(pos2(0.0, 300.0), vec2(1170.0, h));
        assert!(second_row(true, true, panel(5.0 * U + 1.0), &m));
        assert!(
            !second_row(true, true, panel(5.0 * U), &m),
            "over 5 U, not 5 U"
        );
        assert!(!second_row(true, false, panel(8.0 * U), &m));
        assert!(
            !second_row(false, true, panel(8.0 * U), &m),
            "chord keyboards, pads, the sample row"
        );
    }

    #[test]
    fn the_view_defaults_to_white_key_28_2_u_wide_and_stays_in_its_range() {
        let view = KeyView::default();
        assert_eq!((view.left, view.white_w), (28.0, 2.0));
        // White key 28 is MIDI 48.
        let keys = keyboard_keys(1170.0, U, view, 0..=119, None);
        assert_eq!((keys[0].midi, keys[0].x.min), (48, 0.0));
        // 70 white keys in one row, 63 in two; 1170 points show 18.28 of 2 U.
        assert_eq!(
            (
                slots(0..=119, None),
                slots(0..=107, None),
                slots(12..=119, None)
            ),
            (70.0, 63.0, 63.0)
        );
        let clamp = |left, white_w, slots| {
            let v = KeyView { left, white_w }.clamped(slots, 1170.0, U);
            (v.left, v.white_w)
        };
        assert_eq!(clamp(28.0, 2.0, 70.0), (28.0, 2.0));
        assert_eq!(clamp(-3.0, 2.0, 70.0), (0.0, 2.0));
        let shown = 1170.0 / (2.0 * U);
        assert_eq!(clamp(100.0, 2.0, 70.0), (70.0 - shown, 2.0));
        assert_eq!(clamp(100.0, 2.0, 63.0), (63.0 - shown, 2.0));
        assert_eq!(clamp(10.0, 0.5, 70.0), (10.0, 1.0));
        assert_eq!(clamp(10.0, 9.0, 70.0), (10.0, 8.0));
        // A width that shows every key (36.6 at 1 U, of 30) keeps the edge at 0.
        assert_eq!(clamp(5.0, 1.0, 30.0), (0.0, 1.0));
    }

    #[test]
    fn rows_share_the_view_so_each_top_key_is_an_octave_above_the_key_below_it() {
        let m = Metrics::from_pixels_per_point(1.0, DEFAULT_UI_SCALE_INDEX);
        for view in [
            KeyView::default(),
            KeyView {
                left: 0.0,
                white_w: 1.0,
            },
            KeyView {
                left: 40.5,
                white_w: 3.25,
            },
            KeyView {
                left: 100.0,
                white_w: 2.0,
            },
        ] {
            let piano = Piano { view, ..two_rows() };
            let rows = piano.rows(&m, None);
            let [top, bottom] = &rows[..] else { panic!() };
            assert!(!top.keys.is_empty());
            if view.left == 100.0 {
                // Scrolled past the end, the view stops there: the main row's last key, 107, ends at its right edge.
                let last = bottom.keys.last().unwrap();
                assert_eq!((last.midi, last.x.max), (107, bottom.rect.width()));
            }
            assert_eq!(top.keys.len(), bottom.keys.len(), "{view:?}");
            for k in &top.keys {
                let below = bottom.keys.iter().find(|b| b.midi + 12 == k.midi);
                assert_eq!(
                    below.map(|b| (b.x, b.black)),
                    Some((k.x, k.black)),
                    "{view:?} {k:?}"
                );
            }
        }
        // The full ranges, at 1 U from the left end on a wide enough row: 0–107 under 12–119; one row 0–119.
        let wide = KeyView {
            left: 0.0,
            white_w: 1.0,
        };
        let ends = |range: RangeInclusive<u8>| {
            let keys = keyboard_keys(80.0 * U, U, wide, range, None);
            (keys[0].midi, keys.last().unwrap().midi)
        };
        assert_eq!(
            (ends(0..=119), ends(0..=107), ends(12..=119)),
            ((0, 119), (0, 107), (12, 119))
        );
    }

    #[test]
    fn both_rows_play_one_live_input_with_velocity_glissando_and_key_lock() {
        // Top row 405 … 470.5 (65.5 tall): E is 64 there; bottom row 474.5 … 540: E is 52.
        let (top_e, bottom_e) = (pos2(5.0 * U, 438.0), pos2(5.0 * U, 507.25));
        let top_v = (438.0 - 405.0) / 65.5;
        let gap = pos2(5.0 * U, 472.5);
        let frames: Vec<Frame> = vec![
            vec![].into(),
            vec![touch(1, TouchPhase::Start, bottom_e)].into(),
            // A second finger on the top row: both rows hold together.
            vec![touch(2, TouchPhase::Start, top_e)].into(),
            // The first slides up into the gap (nothing), then onto the top row's F (65) at its press velocity.
            vec![touch(1, TouchPhase::Move, gap)].into(),
            vec![touch(1, TouchPhase::Move, pos2(7.0 * U, 450.0))].into(),
            vec![
                touch(1, TouchPhase::End, pos2(7.0 * U, 450.0)),
                touch(2, TouchPhase::End, top_e),
            ]
            .into(),
        ];
        let played: Vec<Played> = run_on(two_rows(), PLAY, frames)
            .into_iter()
            .map(|f| f.0)
            .collect();
        let held: Vec<Vec<(u8, f32)>> = played.into_iter().map(|p| p.held).collect();
        assert_eq!(
            held,
            [
                vec![],
                vec![(52, 0.5)],
                vec![(52, 0.5), (64, top_v)],
                vec![(64, top_v)],
                vec![(65, 0.5), (64, top_v)],
                vec![]
            ]
        );
        // Key-lock in the top row: C#5 (centred at 2 U) snaps to C5 on its left half, D5 on its right.
        let input = KeyInput {
            lock: Some((0, MAJOR)),
            ..PLAY
        };
        for (x, key) in [(1.8, 60), (2.2, 62)] {
            let frames = vec![
                vec![].into(),
                vec![touch(1, TouchPhase::Start, pos2(x * U, 420.0))].into(),
            ];
            let out = run_on(two_rows(), input, frames);
            assert_eq!(keys(&[out[1].0.clone()])[0].0, [key], "{x} U");
        }
    }

    #[test]
    fn a_held_note_lights_in_every_row_that_shows_it() {
        let frames = vec![
            vec![].into(),
            vec![touch(1, TouchPhase::Start, pos2(5.0 * U, 438.0))].into(),
        ];
        let out = run_on(two_rows(), PLAY, frames);
        // E5 in the top row, and E5 again in the bottom row (its top on the whole pixel), 7 white keys to the right.
        assert_eq!(
            fresh_presses(&out[1].1),
            [(4.0 * U, 405.0), (18.0 * U, 474.0)]
        );
    }

    /// The (left, top) of the white keys the frame drew freshly pressed: `whiteKey` 0.9 of the way to the channel
    /// colour, each key's fill and overpaint once.
    fn fresh_presses(out: &egui::FullOutput) -> Vec<(f32, f32)> {
        let white = theme_by_id(0).colors.white_key.to_color32();
        let pressed = white.lerp_to_gamma(Color32::WHITE, 0.9);
        let mut lit: Vec<(f32, f32)> = (out.shapes.iter())
            .filter_map(|s| match &s.shape {
                egui::Shape::Rect(r) if r.fill == pressed => Some((r.rect.left(), r.rect.top())),
                _ => None,
            })
            .collect();
        lit.dedup();
        lit
    }

    /// A key's press amount moves only on drawn frames: the keyboard asks for the next one while any key is still on
    /// its way, and for none once every key is there.
    #[test]
    fn the_keyboard_asks_for_frames_while_a_key_fades_and_none_once_it_is_out() {
        let frame = |notes: u128| Frame {
            notes,
            ..Frame::from(vec![])
        };
        let mut frames = vec![frame(0), frame(1 << 52)];
        frames.extend((0..40).map(|_| frame(0)));
        let seen = run_with(PLAY, frames);
        let white = theme_by_id(0).colors.white_key.to_color32();
        let e4 = pos2(5.0 * U, 450.0);
        let fill = |i: usize| {
            let shapes: Vec<egui::Shape> =
                (seen[i].1.shapes.iter()).map(|s| s.shape.clone()).collect();
            top_fill(&shapes, e4)
        };
        let delay = |i: usize| seen[i].1.viewport_output[&egui::ViewportId::ROOT].repaint_delay;
        // On (0.9), then fading (× 0.825 a draw): the next frame is asked for at once.
        assert_eq!(fill(1), Some(white.lerp_to_gamma(Color32::WHITE, 0.9)));
        assert_eq!(
            fill(2),
            Some(white.lerp_to_gamma(Color32::WHITE, 0.9 * 0.825))
        );
        for i in 1..=3 {
            assert_eq!(delay(i), Duration::ZERO, "frame {i}");
        }
        // Out (0.9 · 0.825^n under 0.001 after 35 draws): `whiteKey` again, and no frame asked for.
        let last = seen.len() - 1;
        assert_eq!(fill(last), Some(white));
        assert_eq!(delay(last), Duration::MAX);
    }

    #[test]
    fn a_tapped_key_lights_on_its_frame() {
        // Pressed and lifted within one frame: never held, yet live input (T = 1) for that draw.
        let white = theme_by_id(0).colors.white_key.to_color32();
        let tap = vec![
            touch(1, TouchPhase::Start, E4),
            touch(1, TouchPhase::End, E4),
        ];
        let shapes = drawn(PLAY, &Settings::default(), vec![tap]);
        let pressed = white.lerp_to_gamma(Color32::WHITE, 0.9);
        assert_eq!(top_fill(&shapes, pos2(5.0 * U, 450.0)), Some(pressed));
    }

    /// Typed (or MIDI) notes are live input as a finger's key is (T = 1): they light both rows of two-keyboard mode.
    #[test]
    fn a_typed_note_lights_in_both_rows() {
        let frames = vec![Frame {
            notes: 1 << 64,
            ..Frame::from(vec![])
        }];
        let out = run_on(two_rows(), PLAY, frames);
        assert_eq!(
            fresh_presses(&out[0].1),
            [(4.0 * U, 405.0), (18.0 * U, 474.0)]
        );
    }

    #[test]
    fn keys_outside_the_channels_range_take_the_disabled_colours() {
        // In range = the scale test and the channel's key range, 50 … 60 here.
        let colors = theme_by_id(0).colors;
        let input = KeyInput {
            range: Some((50, 60)),
            ..PLAY
        };
        let (_, out) = run_with(input, vec![vec![].into()]).remove(0);
        let shapes: Vec<egui::Shape> = out.shapes.into_iter().map(|s| s.shape).collect();
        let at = |x: f32, y: f32| top_fill(&shapes, pos2(x * U, y));
        let (white, off) = (
            colors.white_key.to_color32(),
            colors.white_key_disabled.to_color32(),
        );
        // C (48) and C# (49) are below the range; E (52) and C (60) are in it; D (62) is above it.
        assert_eq!(at(0.5, 520.0), Some(off));
        let black_off = colors.black_key_disabled.to_color32();
        assert_eq!(at(2.0, 420.0), Some(black_off));
        assert_eq!(at(5.0, 520.0), Some(white));
        assert_eq!(at(15.0, 520.0), Some(white));
        assert_eq!(at(17.0, 520.0), Some(off));
        // A root key out of range keeps its marker: A is the channel colour.
        assert_eq!(at(0.5, 538.0), Some(Color32::WHITE));
    }

    #[test]
    fn a_row_with_no_height_draws_and_plays_nothing() {
        let flat = Rect::from_x_y_ranges(KEYBOARD.x_range(), 540.0..=540.0);
        let frames = vec![
            vec![].into(),
            vec![touch(1, TouchPhase::Start, pos2(5.0 * U, 540.0))].into(),
        ];
        let white = theme_by_id(0).colors.white_key.to_color32();
        for (played, out) in run_on(flat.into(), PLAY, frames) {
            assert_eq!(played, Played::default());
            let drawn = |s: &egui::epaint::ClippedShape| matches!(&s.shape, egui::Shape::Rect(r) if r.fill == white);
            assert!(!out.shapes.iter().any(drawn), "a white key was drawn");
        }
    }

    fn whites(keys: &[KeyboardKey]) -> Vec<KeyboardKey> {
        keys.iter().copied().filter(|k| !k.black).collect()
    }

    #[test]
    fn white_keys_are_2u_wide_and_follow_cdefgab() {
        let keys = keyboard_keys(1170.0, U, KeyView::default(), 0..=119, None);
        let whites = whites(&keys);
        for (i, k) in whites.iter().enumerate() {
            assert_eq!(
                k.x,
                Rangef::new(i as f32 * 2.0 * U, (i + 1) as f32 * 2.0 * U)
            );
        }
        let midis: Vec<u8> = whites.iter().take(9).map(|k| k.midi).collect();
        assert_eq!(midis, [48, 50, 52, 53, 55, 57, 59, 60, 62]);
    }

    #[test]
    fn black_keys_sit_on_white_key_boundaries() {
        let keys = keyboard_keys(1170.0, U, KeyView::default(), 0..=119, None);
        let black: Vec<&KeyboardKey> = keys.iter().filter(|k| k.black).take(5).collect();
        assert_eq!(
            black.iter().map(|k| k.midi).collect::<Vec<_>>(),
            [49, 51, 54, 56, 58]
        );
        // C# is centred on the C/D boundary, 1.4 U wide.
        assert_eq!(
            black[0].x,
            Rangef::new(2.0 * U - 0.7 * U, 2.0 * U + 0.7 * U)
        );
        // F# is centred on the F/G boundary (after C D E F).
        assert_eq!(black[2].x.center(), 4.0 * 2.0 * U);
    }

    #[test]
    fn keys_fill_the_width_and_stop_after_it() {
        let keys = keyboard_keys(1170.0, U, KeyView::default(), 0..=119, None);
        let last = *whites(&keys).last().unwrap();
        assert!(last.x.min < 1170.0 && last.x.max >= 1170.0);
        assert!(keys.iter().all(|k| k.x.min < 1170.0));
        assert_eq!(whites(&keys).len(), 19);
    }

    #[test]
    fn black_keys_cover_the_top_06_of_the_keyboard() {
        let keyboard = Rect::from_min_size(pos2(10.0, 400.0), vec2(1170.0, 140.0));
        let keys = keyboard_keys(keyboard.width(), U, KeyView::default(), 0..=119, None);
        let (c, c_sharp) = (keys[0].rect(keyboard), keys[1].rect(keyboard));
        assert_eq!(
            c,
            Rect::from_min_size(pos2(10.0, 400.0), vec2(2.0 * U, 140.0))
        );
        assert_eq!(c_sharp.top(), 400.0);
        assert!((c_sharp.height() - 0.6 * 140.0).abs() < 1e-3);
        assert_eq!(c_sharp.center().x, 10.0 + 2.0 * U);
    }

    #[test]
    fn black_key_wins_where_it_overlaps_a_white_key() {
        let keyboard = Rect::from_min_size(pos2(0.0, 400.0), vec2(1170.0, 140.0));
        let keys = keyboard_keys(keyboard.width(), U, KeyView::default(), 0..=119, None);
        let boundary = 2.0 * U - 1.0;
        let midi = |pos| key_at(&keys, keyboard, pos).map(|k| k.midi);
        assert_eq!(midi(pos2(boundary, 410.0)), Some(49));
        assert_eq!(midi(pos2(boundary, 530.0)), Some(48));
        assert_eq!(midi(pos2(boundary, 300.0)), None);
    }

    #[test]
    fn octave_labels() {
        assert_eq!(octave_label(60, false), "C5");
        assert_eq!(octave_label(60, true), "C4");
        assert_eq!(octave_label(48, false), "C4");
        assert_eq!(octave_label(0, true), "C-1");
    }

    // Piano and pad visuals.

    #[test]
    fn the_worked_example_at_u_85_white_keys_170_marker_9_black_keys_119_separators_2_px() {
        let u = 85.0;
        let keys = keyboard_keys(3120.0, u, KeyView::default(), 0..=119, None);
        let (c, c_sharp, d) = (keys[0], keys[1], keys[2]);
        assert_eq!((c.midi, c.x.span(), c_sharp.midi), (48, 170.0, 49));
        // A 400 px row from y 600.
        let y = Rangef::new(600.0, 1000.0);
        let w = white_px(c.x, y, u);
        assert_eq!(w.body, Rect::from_x_y_ranges(0.0..=170.0, 600.0..=1000.0));
        // Step 2 stops at trunc(bottom − 0.1 U) = 991: a 9 px strip across the key's full width stays colour A.
        assert_eq!(w.upper, Rect::from_x_y_ranges(0.0..=170.0, 600.0..=991.0));
        // The separators: 2 px of `background` centred on both edges, the full key height.
        assert_eq!(
            w.seps,
            [
                Rect::from_x_y_ranges(-1.0..=1.0, 600.0..=1000.0),
                Rect::from_x_y_ranges(169.0..=171.0, 600.0..=1000.0)
            ]
        );
        // The label sits on the strip: (key x, key y, key w, key h − 0.1 U).
        assert_eq!(w.label, Rect::from_x_y_ranges(0.0..=170.0, 600.0..=991.5));
        // D's separators meet C's: one 2 px line between them.
        assert_eq!(white_px(d.x, y, u).seps[0], w.seps[1]);
        // C#: x = trunc(170 − 59.5) = 110, w = trunc(229.5) − 110 = 119; h = trunc(600 + 0.6·400) − 600 = 240.
        let b = black_px(c_sharp.x, y, u, 0.0);
        assert_eq!(b.body, Rect::from_x_y_ranges(110.0..=229.0, 600.0..=840.0));
        // At rest t = trunc(600 + 240 − 1 − 8.5) − 600 = 230: a 10 px lip in colour A, 1 px borders above it.
        assert_eq!(
            b.border,
            Rect::from_x_y_ranges(110.0..=229.0, 600.0..=830.0)
        );
        assert_eq!(b.inner, Rect::from_x_y_ranges(111.0..=228.0, 600.0..=830.0));
        assert_eq!(b.label, Rect::from_x_y_ranges(111.0..=228.0, 600.0..=821.5));
        // Fully pressed, the lip shrinks by 0.08 U: t = trunc(830.5 + 6.8) − 600 = 237.
        assert_eq!(black_px(c_sharp.x, y, u, 1.0).border.bottom(), 837.0);
    }

    #[test]
    fn black_keys_are_0_7_w_centred_on_their_boundary_truncated_with_no_per_key_offsets() {
        let u = 85.0;
        let keys = keyboard_keys(3120.0, u, KeyView::default(), 0..=119, None);
        let y = Rangef::new(0.0, 400.0);
        // C# D# | F# G# A#: boundaries at 1, 2, 4, 5 and 6 white keys (170 px each).
        let blacks: Vec<Rect> = (keys.iter().filter(|k| k.black).take(5))
            .map(|k| black_px(k.x, y, u, 0.0).body)
            .collect();
        for (body, b) in blacks.iter().zip([170.0, 340.0, 680.0, 850.0, 1020.0]) {
            let b: f32 = b;
            assert_eq!(body.left(), (b - 59.5).trunc(), "{b}");
            assert_eq!(body.right(), (b + 59.5).trunc(), "{b}");
            assert_eq!((body.top(), body.bottom()), (0.0, 240.0));
        }
        // A fractional row: the edges truncate. U = 32 at a white key of 1.37 U: b = 43.84, 0.35 W = 15.344.
        let keys = keyboard_keys(
            1000.0,
            32.0,
            KeyView {
                left: 28.0,
                white_w: 1.37,
            },
            0..=119,
            None,
        );
        let b = black_px(keys[1].x, Rangef::new(10.5, 145.5), 32.0, 0.0);
        assert_eq!((b.body.left(), b.body.right()), (28.0, 59.0));
        assert_eq!((b.body.top(), b.body.bottom()), (10.0, 91.0));
    }

    #[test]
    fn the_press_amount_rises_at_0_9_and_falls_at_0_175_per_draw() {
        assert!(near(press_tick(0.0, 1.0), 0.9));
        assert!(near(press_tick(0.9, 1.0), 0.99));
        assert!(near(press_tick(1.0, 0.0), 0.825));
        assert!(near(press_tick(0.825, 0.0), 0.825 * 0.825));
        // Song playback's 0.7: up at 0.9, down at 0.175.
        assert!(near(press_tick(0.0, 0.7), 0.63));
        assert!(near(press_tick(1.0, 0.7), 1.0 - 0.175 * 0.3));
        // Within 0.001 it lands (the frames stop).
        assert_eq!(press_tick(0.9995, 1.0), 1.0);
        assert_eq!(press_tick(0.0005, 0.0), 0.0);
    }

    #[test]
    fn key_names_use_sharps_and_a_root_key_adds_its_octave() {
        assert_eq!(key_name(48, true, false), "C4");
        assert_eq!(key_name(60, true, false), "C5");
        assert_eq!(key_name(60, true, true), "C4");
        assert_eq!(key_name(0, true, true), "C-1");
        assert_eq!(key_name(62, true, false), "D5");
        assert_eq!(key_name(61, false, false), "C#");
        assert_eq!(key_name(64, false, true), "E");
        assert_eq!(key_name(70, false, false), "A#");
    }

    /// One drawn frame of the keyboard over [`KEYBOARD`] in the channel colour [`Color32::WHITE`], played as `input`
    /// with `settings`, after the frames of `before` (touch events): its shapes.
    fn drawn(input: KeyInput, settings: &Settings, before: Vec<Vec<Event>>) -> Vec<egui::Shape> {
        let ctx = egui::Context::default();
        let screen = Rect::from_min_size(Pos2::ZERO, vec2(1170.0, 540.0));
        let theme = theme_by_id(0);
        let m = Metrics::from_pixels_per_point(1.0, DEFAULT_UI_SCALE_INDEX);
        let mut frames = vec![vec![]];
        frames.extend(before);
        let mut out = None;
        for events in frames {
            let raw = RawInput {
                screen_rect: Some(screen),
                events,
                ..Default::default()
            };
            out = Some(ctx.run(raw, |ctx| {
                egui::CentralPanel::default().show(ctx, |ui| {
                    let cx = UiCtx {
                        theme: &theme,
                        m: &m,
                    };
                    keyboard(ui, KEYBOARD, &cx, Color32::WHITE, settings, input);
                });
            }));
        }
        let shapes = out.unwrap().shapes.into_iter().map(|s| s.shape);
        shapes.collect()
    }

    /// The colour on top at `pos`: the last rect drawn over it (text aside).
    fn top_fill(shapes: &[egui::Shape], pos: Pos2) -> Option<Color32> {
        (shapes.iter().rev()).find_map(|s| match s {
            egui::Shape::Rect(r) if r.rect.contains(pos) => Some(r.fill),
            _ => None,
        })
    }

    /// Every text drawn: its string, colour and the rect its galley covers.
    fn texts(shapes: &[egui::Shape]) -> Vec<(String, Color32, Rect)> {
        (shapes.iter())
            .filter_map(|s| match s {
                egui::Shape::Text(t) => Some((
                    t.galley.text().to_owned(),
                    t.fallback_color,
                    Rect::from_min_size(t.pos, t.galley.size()),
                )),
                _ => None,
            })
            .collect()
    }

    #[test]
    fn a_root_c_has_a_channel_marker_strip_across_its_bottom_and_a_grey_octave_label_on_it() {
        let colors = theme_by_id(0).colors;
        let (white, label) = (
            colors.white_key.to_color32(),
            colors.black_key_disabled.to_color32(),
        );
        let shapes = drawn(PLAY, &Settings::default(), vec![]);
        // U = 32: the strip is ceil(3.2) = 4 px, 536 … 540, across C4 (0 … 64) less the separators.
        for (x, root) in [(2.0, true), (32.0, true), (62.0, true), (96.0, false)] {
            let marker = if root { Color32::WHITE } else { white };
            assert_eq!(top_fill(&shapes, pos2(x, 537.0)), Some(marker), "{x}");
            assert_eq!(top_fill(&shapes, pos2(x, 535.0)), Some(white), "{x}");
        }
        // Every C in view: C4, C5 and C6, each labelled with its octave in `blackKeyDisabled`, centred on the key and
        // sitting on its strip at 12 pt (24 px).
        let labels = texts(&shapes);
        let names: Vec<&str> = labels.iter().map(|l| l.0.as_str()).collect();
        assert_eq!(names, ["C4", "C5", "C6"]);
        for ((_, color, rect), left) in labels.iter().zip([0.0, 7.0 * 64.0, 14.0 * 64.0]) {
            assert_eq!(*color, label);
            assert!((rect.center().x - (left + 32.0)).abs() < 1.0, "{rect:?}");
            assert!((rect.bottom() - (540.0 - 3.2)).abs() < 0.5, "{rect:?}");
            assert!(rect.height() > 24.0, "{rect:?}");
        }
        // A 2 px `background` separator on each white key's edge, through the strip.
        let bg = colors.background.to_color32();
        for x in [63.5, 64.5, 127.5] {
            assert_eq!(top_fill(&shapes, pos2(x, 538.0)), Some(bg), "{x}");
        }
        // Without a channel that supports key-lock there are no roots: no marker and no label.
        let input = KeyInput {
            enabled: false,
            ..PLAY
        };
        let shapes = drawn(input, &Settings::default(), vec![]);
        assert_eq!(top_fill(&shapes, pos2(32.0, 537.0)), Some(white));
        assert!(texts(&shapes).is_empty());
    }

    #[test]
    fn out_of_scale_keys_are_disabled_the_project_key_is_the_root_and_black_keys_have_a_lip() {
        let colors = theme_by_id(0).colors;
        let c = |r: crate::theme::Rgba| r.to_color32();
        let d_major = KeyInput {
            lock: Some((2, MAJOR)),
            ..PLAY
        };
        let shapes = drawn(d_major, &Settings::default(), vec![]);
        let at = |x: f32, y: f32| top_fill(&shapes, pos2(x, y));
        // C (out of D major): `whiteKeyDisabled` over its strip and above it.
        assert_eq!(at(16.0, 420.0), Some(c(colors.white_key_disabled)));
        assert_eq!(at(16.0, 538.0), Some(c(colors.white_key_disabled)));
        // D, the root: the channel strip under `whiteKey`.
        assert_eq!(at(96.0, 538.0), Some(Color32::WHITE));
        assert_eq!(at(96.0, 500.0), Some(c(colors.white_key)));
        // C# (in the scale), 41 … 86 × 405 … 486: 1 px `background` borders, `blackKey` inside, and a lip below
        // t = trunc(405 + 81 − 1 − 3.2) − 405 = 76 in colour A (`blackKey`).
        assert_eq!(at(41.5, 420.0), Some(c(colors.background)));
        assert_eq!(at(64.0, 420.0), Some(c(colors.black_key)));
        assert_eq!(at(41.5, 483.0), Some(c(colors.black_key)));
        // D# (out of the scale) is `blackKeyDisabled` all over, with no step 2.
        assert_eq!(at(105.5, 420.0), Some(c(colors.black_key_disabled)));
        assert_eq!(at(128.0, 420.0), Some(c(colors.black_key_disabled)));
        // The root's label: `D4`, not `C4`.
        let names: Vec<String> = texts(&shapes).into_iter().map(|t| t.0).collect();
        assert_eq!(names, ["D4", "D5", "D6"]);
    }

    #[test]
    fn key_names_show_on_in_range_keys_but_not_on_black_keys_under_chromatic() {
        let colors = theme_by_id(0).colors;
        let names = Settings {
            key_names: true,
            ..Settings::default()
        };
        let first = |shapes: &[egui::Shape]| -> Vec<(String, Color32)> {
            let t = texts(shapes).into_iter().take(4);
            t.map(|(s, c, _)| (s, c)).collect()
        };
        // Chromatic: every white key is named, in `whiteKeyDisabled`; the black keys are not.
        let shapes = drawn(PLAY, &names, vec![]);
        let wkd = colors.white_key_disabled.to_color32();
        let bkd = colors.black_key_disabled.to_color32();
        assert_eq!(
            first(&shapes),
            [
                ("C4".into(), bkd),
                ("D".into(), wkd),
                ("E".into(), wkd),
                ("F".into(), wkd)
            ]
        );
        // D major: C is out of range (unnamed), C# in range and black (named, `blackKeyDisabled`), D the root.
        let d_major = KeyInput {
            lock: Some((2, MAJOR)),
            ..PLAY
        };
        let shapes = drawn(d_major, &names, vec![]);
        assert_eq!(
            first(&shapes),
            [
                ("C#".into(), bkd),
                ("D4".into(), bkd),
                ("E".into(), wkd),
                ("F#".into(), bkd)
            ]
        );
        // C#'s label sits in its inner upper part, 0.1 U above the lip: (42, 405, 43, 76 − 3.2).
        let (_, _, rect) = texts(&shapes)[0].clone();
        assert!((rect.center().x - 64.0).abs() < 1.0, "{rect:?}");
        assert!(
            (rect.bottom() - (405.0 + 76.0 - 3.2)).abs() < 0.5,
            "{rect:?}"
        );
    }

    #[test]
    fn a_held_key_turns_the_channel_colour_by_the_press_law() {
        let white = theme_by_id(0).colors.white_key.to_color32();
        let mix = |p: f32| white.lerp_to_gamma(Color32::WHITE, p);
        let e4 = pos2(5.0 * U, 450.0);
        let press = vec![touch(1, TouchPhase::Start, E4)];
        // Its first frame: p = 0.9; the next, 0.99.
        let shapes = drawn(PLAY, &Settings::default(), vec![press.clone()]);
        assert_eq!(top_fill(&shapes, e4), Some(mix(0.9)));
        let shapes = drawn(PLAY, &Settings::default(), vec![press.clone(), vec![]]);
        assert_eq!(top_fill(&shapes, e4), Some(mix(0.99)));
        // Released: p falls by 0.175 of the way per draw.
        let lift = vec![touch(1, TouchPhase::End, E4)];
        let shapes = drawn(PLAY, &Settings::default(), vec![press, lift]);
        assert_eq!(top_fill(&shapes, e4), Some(mix(0.9 * 0.825)));
    }

    /// A note that only sounds ([`KeyInput::sounding`]: song playback, a preview) aims at T = 0.7; one held live aims
    /// at 1, which wins.
    #[test]
    fn a_sounding_note_lights_at_0_7_and_a_live_note_at_1_wins() {
        let white = theme_by_id(0).colors.white_key.to_color32();
        let mix = |p: f32| white.lerp_to_gamma(Color32::WHITE, p);
        let e4 = pos2(5.0 * U, 450.0);
        let sounding = KeyInput {
            sounding: 1 << 52,
            ..PLAY
        };
        // Its first draw: p = 0.9 · 0.7; the next, 0.63 + 0.9 · 0.07.
        let shapes = drawn(sounding, &Settings::default(), vec![]);
        assert_eq!(top_fill(&shapes, e4), Some(mix(0.63)));
        let shapes = drawn(sounding, &Settings::default(), vec![vec![]]);
        assert_eq!(top_fill(&shapes, e4), Some(mix(0.693)));
        let held = KeyInput {
            notes: 1 << 52,
            ..sounding
        };
        let shapes = drawn(held, &Settings::default(), vec![]);
        assert_eq!(top_fill(&shapes, e4), Some(mix(0.9)));
    }

    /// The engine's keys reach the keyboard only on drawn frames: it asks for them while any note sounds, though the
    /// key has long reached T = 0.7, so the note-off shows.
    #[test]
    fn the_keyboard_asks_for_frames_while_a_note_sounds() {
        let input = KeyInput {
            sounding: 1 << 52,
            ..PLAY
        };
        let seen = run_with(input, (0..40).map(|_| Frame::from(vec![])).collect());
        let delay = seen[39].1.viewport_output[&egui::ViewportId::ROOT].repaint_delay;
        assert_eq!(delay, Duration::ZERO);
        // Held live too, it is the app's to repaint (its note-off is a frame's input): none asked for once lit.
        let held = KeyInput {
            notes: 1 << 52,
            ..input
        };
        let seen = run_with(held, (0..40).map(|_| Frame::from(vec![])).collect());
        let delay = seen[39].1.viewport_output[&egui::ViewportId::ROOT].repaint_delay;
        assert_eq!(delay, Duration::MAX);
    }

    #[test]
    fn a_root_black_key_has_a_channel_lip_and_a_white_key_disabled_label() {
        // C# major: the roots are the C#s, labelled `C#4` … in `whiteKeyDisabled`, their lips in the channel colour.
        let colors = theme_by_id(0).colors;
        let input = KeyInput {
            lock: Some((1, MAJOR)),
            ..PLAY
        };
        let shapes = drawn(input, &Settings::default(), vec![]);
        let first = texts(&shapes).into_iter().next().map(|t| (t.0, t.1));
        let wkd = colors.white_key_disabled.to_color32();
        assert_eq!(first, Some(("C#4".into(), wkd)));
        // C# (41 … 86 × 405 … 486): the lip below t = 76 in the channel colour, `blackKey` inside the borders above.
        assert_eq!(top_fill(&shapes, pos2(64.0, 483.0)), Some(Color32::WHITE));
        let black = colors.black_key.to_color32();
        assert_eq!(top_fill(&shapes, pos2(64.0, 420.0)), Some(black));
    }

    #[test]
    fn under_scale_key_lock_every_key_is_white_above_its_strip_whatever_its_range() {
        // White key step 2: B is always `whiteKey` while key-lock is on in `Scale` mode; A (the strip) keeps the range.
        let colors = theme_by_id(0).colors;
        let (white, off) = (
            colors.white_key.to_color32(),
            colors.white_key_disabled.to_color32(),
        );
        let input = KeyInput {
            lock: Some((2, MAJOR)),
            all_white: true,
            range: Some((50, 52)),
            ..PLAY
        };
        let shapes = drawn(input, &Settings::default(), vec![]);
        // C# (49), the first slot (0 … 64), is out of the range; D (50), the second, in it.
        for (x, strip) in [(32.0, off), (96.0, Color32::WHITE)] {
            assert_eq!(top_fill(&shapes, pos2(x, 500.0)), Some(white), "{x}");
            assert_eq!(top_fill(&shapes, pos2(x, 538.0)), Some(strip), "{x}");
        }
        // The piano layout under the same key-lock: G (55, 256 … 320), out of the range, is disabled all over.
        let shapes = drawn(
            KeyInput {
                all_white: false,
                ..input
            },
            &Settings::default(),
            vec![],
        );
        assert_eq!(top_fill(&shapes, pos2(288.0, 500.0)), Some(off));
        assert_eq!(top_fill(&shapes, pos2(288.0, 538.0)), Some(off));
    }

    /// [`paint`] alone over [`KEYBOARD`] in the channel colour [`Color32::WHITE`]: every key at press amount `p`, the
    /// notes of `sounding` (bit k = MIDI k) sounding.
    fn painted(input: KeyInput, p: f32, sounding: u128) -> Vec<egui::Shape> {
        let ctx = egui::Context::default();
        let theme = theme_by_id(0);
        let m = Metrics::from_pixels_per_point(1.0, DEFAULT_UI_SCALE_INDEX);
        let cx = UiCtx {
            theme: &theme,
            m: &m,
        };
        let rows = Piano::from(KEYBOARD).rows(&m, None);
        let settings = Settings::default();
        let out = ctx.run(RawInput::default(), |ctx| {
            egui::CentralPanel::default()
                .frame(egui::Frame::NONE)
                .show(ctx, |ui| {
                    let press = [p; 128];
                    paint(
                        ui.painter(),
                        &rows,
                        &cx,
                        Color32::WHITE,
                        &settings,
                        input,
                        &press,
                        sounding,
                    );
                });
        });
        out.shapes.into_iter().map(|s| s.shape).collect()
    }

    #[test]
    fn an_out_of_range_black_key_draws_its_upper_part_only_while_its_note_sounds() {
        // D major: D# (51; 105 … 150 × 405 … 486) is out of the scale. Fading at p = 0.5 after its note ended (T = 0)
        // it is one fill; while its note sounds (T > 0) it has its `background` borders and its lip too.
        let colors = theme_by_id(0).colors;
        let bg = colors.background.to_color32();
        let d_major = KeyInput {
            lock: Some((2, MAJOR)),
            ..PLAY
        };
        let fading = colors.black_key_disabled.to_color32();
        let fading = fading.lerp_to_gamma(Color32::WHITE, 0.5);
        let border = pos2(105.5, 420.0);
        assert_eq!(top_fill(&painted(d_major, 0.5, 0), border), Some(fading));
        assert_eq!(top_fill(&painted(d_major, 0.5, 1 << 51), border), Some(bg));
        // In range, it is always drawn: C# (49) at rest.
        assert_eq!(
            top_fill(&painted(d_major, 0.0, 0), pos2(41.5, 420.0)),
            Some(bg)
        );
    }

    #[test]
    fn the_panel_is_control_fill_under_a_1_px_background_line_with_module_fill_behind_the_wheels() {
        let colors = theme_by_id(0).colors;
        let panel = Rect::from_min_max(pos2(0.0, 380.0), pos2(1170.0, 540.0));
        for (ppp, wheels) in [(1.0, false), (2.0, true)] {
            let m = Metrics::from_pixels_per_point(ppp, DEFAULT_UI_SCALE_INDEX);
            let theme = theme_by_id(0);
            let cx = UiCtx {
                theme: &theme,
                m: &m,
            };
            let ctx = egui::Context::default();
            let out = ctx.run(RawInput::default(), |ctx| {
                egui::CentralPanel::default()
                    .frame(egui::Frame::NONE)
                    .show(ctx, |ui| panel_backdrop(ui.painter(), panel, wheels, &cx));
            });
            // (The egui panel paints a transparent rect of its own.)
            let shapes: Vec<egui::Shape> = (out.shapes.into_iter().map(|s| s.shape))
                .filter(|s| !matches!(s, egui::Shape::Rect(r) if r.fill == Color32::TRANSPARENT))
                .collect();
            let px = 1.0 / ppp;
            let column =
                Rect::from_min_size(pos2(0.0, 380.0 + m.u(1.75)), vec2(m.u(4.0), panel.height()));
            let mut want = vec![(panel, colors.control_fill)];
            if wheels {
                want.push((column, colors.module_fill));
            }
            want.push((panel.with_max_y(380.0 + px), colors.background));
            let got: Vec<(Rect, Color32)> = (shapes.iter())
                .filter_map(|s| match s {
                    egui::Shape::Rect(r) => Some((r.rect, r.fill)),
                    _ => None,
                })
                .collect();
            let want: Vec<(Rect, Color32)> =
                want.into_iter().map(|(r, c)| (r, c.to_color32())).collect();
            assert_eq!(got, want, "ppp {ppp}");
        }
    }
}
