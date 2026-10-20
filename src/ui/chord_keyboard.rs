//! The chord keyboard of keyboard kinds 2 `Chords` and 3 `Strum`: eight strips of cells, one chord of the set each.
//!
//! The keyboard keeps its fingers, its sounding voices and the pitches' reference counts between frames, and reports
//! note-ons and note-offs ([`Note`]) for the app to send on the current channel's live input, the piano's path.
//!
//! - Onsets run on egui's input clock (the integration's real-time clock); each goes out with the first frame at or
//!   after its time, and the keyboard asks for that frame.
//! - A pitch outside MIDI 0–127 is not played (the engine takes MIDI keys); its voice keeps the rest.
//! - With one cell (the editor open), the cell runs from the column's top to the label's top, and so does its touch.
//! - A finger that went down on a strip's border (off its cells and label) plays nothing until it lifts; a finger on a
//!   lit cell that Hold turned off plays nothing more either.
//! - With Hold on, a finger sliding between cells moves its voice as without Hold, and its release latches the cell it
//!   is over.
//! - A slide from a cell onto the label, or off the strip, changes nothing until the finger moves onto a cell again.
//! - A new set, octave shift, project key or kind (any change of the pitches) stops every note first, as a key or set
//!   change does; so does a frame that does not show the keyboard, or a focus loss for the fingers' voices.
//! - The editor's single cell plays inversion 0 of the Chords voicing in both kinds, with no Timing.

use std::collections::BTreeMap;

use eframe::egui::{Align2, Color32, Id, Pos2, Rect, Sense, Ui, pos2, vec2};

use crate::metrics::{FontSize, Metrics};
use crate::song::chords::{Chord, ChordSet, STRIPS};
use crate::theme::Rgba;
use crate::ui::keyboard_settings::KEYS;
use crate::ui::{Finger, UiCtx, fingers, focus_lost};

/// Cells per strip: Chords (inversions +3 … −2), Strum (indices 11 … 0).
pub const CHORD_CELLS: usize = 6;
pub const STRUM_CELLS: usize = 12;
/// The cell column's least height, in U.
const COLUMN_MIN: f32 = 6.0;
/// How far the wheels' column moves the strips in, in U (the `showWheels` setting alone decides it).
const WHEELS_W: f32 = 4.0;
/// The root-position dot's radius, in U, and the Chords cell it marks (4th from the top).
const DOT_R: f32 = 1.0 / 8.0;
const ROOT_CELL: usize = 3;
/// A released cell's highlight is kept this much per frame, and gone below the floor.
const FADE: f32 = 0.5;
const FADE_FLOOR: f32 = 0.001;
/// The Timing knob's reach, in ms either way: t = (2v − 1)·40.
pub const TIMING_MS: f32 = 40.0;
/// The label row's brightening while fully opaque.
const LABEL_GAIN: f32 = 1.1111;

/// The quality names: 0 major has none.
pub const QUALITY_NAMES: [&str; 7] = ["", "min", "aug", "dim", "Sus2", "Sus4", "power"];
/// The extension names.
pub const EXTENSION_NAMES: [(i32, &str); 7] = [
    (9, "6"),
    (10, "7"),
    (11, "Maj7"),
    (13, "b9"),
    (14, "9"),
    (17, "11"),
    (18, "#11"),
];

/// The chord keyboard's rect in the keys area `keys`: 4 U in while the `showWheels` setting is on, whether or not the
/// wheels show.
pub fn chord_rect(keys: Rect, wheels_setting: bool, m: &Metrics) -> Rect {
    let x = if wheels_setting { m.u(WHEELS_W) } else { 0.0 };
    keys.with_min_x(keys.left() + x)
}

/// The eight strips of `rect`, equal and with no spacing, left to right.
pub fn strips(rect: Rect) -> [Rect; STRIPS] {
    let w = rect.width() / STRIPS as f32;
    std::array::from_fn(|i| {
        Rect::from_x_y_ranges(
            rect.left() + i as f32 * w..=rect.left() + (i + 1) as f32 * w,
            rect.y_range(),
        )
    })
}

/// A strip's parts: the label row at its bottom and the cells, top to bottom.
#[derive(Clone, Debug, PartialEq)]
pub struct StripLayout {
    pub label: Rect,
    pub cells: Vec<Rect>,
}

/// The parts of `strip` with `count` cells, d = ceil(U/16) px: the label row (x + d, max(y, y + h − U), w − 2d, U), and
/// the cell column (x + d, y + d, w − 2d, h − 2d − U), never shorter than 6 U, top-aligned, split into equal cells. One
/// cell runs from the column's top to the label's top.
pub fn strip_layout(strip: Rect, count: usize, m: &Metrics) -> StripLayout {
    let (d, u) = (m.t(), m.u(1.0));
    let (x, w) = (strip.left() + d, strip.width() - 2.0 * d);
    let label_top = (strip.bottom() - u).max(strip.top());
    let label = Rect::from_min_size(pos2(x, label_top), vec2(w, u));
    let top = strip.top() + d;
    if count == 1 {
        let cell = Rect::from_x_y_ranges(x..=x + w, top..=label_top.max(top));
        return StripLayout {
            label,
            cells: vec![cell],
        };
    }
    let h = (strip.height() - 2.0 * d - u).max(m.u(COLUMN_MIN)) / count as f32;
    let cells = (0..count)
        .map(|i| Rect::from_min_size(pos2(x, top + i as f32 * h), vec2(w, h)))
        .collect();
    StripLayout { label, cells }
}

/// What a point of a strip is.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Hit {
    /// The label row, which wins where it overlaps the cells.
    Label,
    Cell(usize),
    /// Neither: the strip's border, or outside it.
    Off,
}

impl StripLayout {
    pub fn hit(&self, p: Pos2) -> Hit {
        if self.label.contains(p) {
            return Hit::Label;
        }
        self.cells
            .iter()
            .position(|c| c.contains(p))
            .map_or(Hit::Off, Hit::Cell)
    }
}

/// A chord's name: the long root name (the Project Key's), then the quality's name and a space unless it is major, then
/// the highest extension's name. `D min ` keeps its trailing space.
pub fn chord_name(chord: &Chord) -> String {
    let root = KEYS[chord.base_note.rem_euclid(12) as usize];
    let quality = usize::try_from(chord.quality)
        .ok()
        .and_then(|q| QUALITY_NAMES.get(q))
        .filter(|q| !q.is_empty())
        .map_or(String::new(), |q| format!("{q} "));
    let top = (chord.extensions.iter().rev())
        .find_map(|e| EXTENSION_NAMES.iter().find(|(v, _)| v == e))
        .map_or("", |(_, name)| name);
    format!("{root} {quality}{top}")
}

/// The pitches of `cell` of a strip playing `chord` with the octave `shift`: Chords mode cells (top to bottom) play
/// inversions +3 … −2, Strum mode cells single indices 11 … 0 of the compact voicing, and the editor's one cell
/// inversion 0.
pub fn cell_pitches(chord: &Chord, strum: bool, editor: bool, cell: usize, shift: i32) -> Vec<i32> {
    if editor {
        return chord.inversion(0, shift);
    }
    if strum {
        let list = chord.list(true);
        let j = (STRUM_CELLS - 1 - cell) as i32;
        return vec![chord.note(&list, j, shift)];
    }
    chord.inversion(3 - cell as i32, shift)
}

/// A note-on (at a velocity) or note-off for the live input.
#[derive(Clone, Copy, Debug, PartialEq)]
pub enum Note {
    On(u8, f32),
    Off(u8),
}

/// What the keyboard plays from, this frame.
#[derive(Clone, Copy, Debug)]
pub struct ChordInput<'a> {
    /// The set in use: transposed to the project key, the default by scale while untouched.
    pub set: &'a ChordSet,
    /// Strum mode (kind 3): 12 single-note cells, no Timing.
    pub strum: bool,
    /// The `Octave` setting, −2 … +2.
    pub shift: i32,
    /// The `Velocity` setting, [1/127, 1].
    pub velocity: f32,
    /// The Timing in ms, −40 … +40.
    pub timing_ms: f32,
    /// Chord Hold (KEYB).
    pub hold: bool,
    /// The chord editor is open: one cell per strip, the target's at full strength.
    pub editor: bool,
    /// The keys take touches.
    pub enabled: bool,
}

/// Who keeps a voice on.
#[derive(Clone, Copy, Debug, PartialEq)]
enum Owner {
    Finger(u64),
    /// Latched by Hold, left on by a release on the label, or the editor's button.
    Latched,
    Editor,
}

#[derive(Clone, Debug)]
struct Voice {
    owner: Owner,
    strip: usize,
    cell: usize,
    sounding: Vec<i32>,
    /// Onsets still to come: (time, pitch).
    pending: Vec<(f64, i32)>,
}

#[derive(Clone, Copy, Debug)]
struct Touch {
    finger: u64,
    strip: usize,
    /// The cell under it while it plays one; `None` before it reaches a cell, or once it played nothing.
    cell: Option<usize>,
    /// It went down on the label row and has not reached a cell yet.
    from_label: bool,
    /// It plays nothing until it lifts (down on the border, or on a lit cell that Hold turned off).
    spent: bool,
}

/// The chord keyboard's state between frames.
#[derive(Clone, Debug, Default)]
pub struct ChordKeyboard {
    touches: Vec<Touch>,
    voices: Vec<Voice>,
    /// How many voices hold each pitch.
    counts: BTreeMap<i32, u32>,
    /// Each cell's highlight b.
    glow: [[f32; STRUM_CELLS]; STRIPS],
    /// The pitches' sources as last played: a change stops every note.
    made_from: Option<(ChordSet, i32, bool)>,
    /// Hold as last seen: turning it off releases the latched cells.
    held: bool,
    /// The chord editor's target: the strip last touched.
    pub target: usize,
}

impl ChordKeyboard {
    /// Every note stops and every finger is let go, as when the keyboard is not shown.
    pub fn release_all(&mut self) -> Vec<Note> {
        let mut out = Vec::new();
        self.stop_all(&mut out);
        self.touches.clear();
        out
    }

    /// The editor's button held (`Some`, the edited chord's pitches) or let go: its voice, which plays at once.
    pub fn editor_voice(&mut self, pitches: Option<Vec<i32>>, velocity: f32) -> Vec<Note> {
        let mut out = Vec::new();
        let old = self.voices.iter().position(|v| v.owner == Owner::Editor);
        match (old, pitches) {
            (Some(i), None) => self.stop(i, &mut out),
            (Some(i), Some(p)) => self.slide(i, self.voices[i].cell, p, velocity, &mut out),
            (None, Some(p)) => {
                let strip = self.target;
                self.start(Owner::Editor, (strip, 0), p, None, velocity, &mut out);
            }
            (None, None) => {}
        }
        out
    }

    /// One note on of `pitch`: a pitch already sounding is sent off, then on again.
    fn on(&mut self, pitch: i32, velocity: f32, out: &mut Vec<Note>) {
        let Ok(key) = u8::try_from(pitch).map(|k| k.min(127)) else {
            return;
        };
        if pitch > 127 {
            return;
        }
        let n = self.counts.entry(pitch).or_default();
        if *n > 0 {
            out.push(Note::Off(key));
        }
        *n += 1;
        out.push(Note::On(key, velocity));
    }

    /// One holder of `pitch` lets go: note-off once the last one has.
    fn off(&mut self, pitch: i32, out: &mut Vec<Note>) {
        let Some(n) = self.counts.get_mut(&pitch) else {
            return;
        };
        *n -= 1;
        if *n == 0 {
            self.counts.remove(&pitch);
            out.push(Note::Off(pitch as u8));
        }
    }

    /// A new voice for `owner` on (strip, cell) playing `pitches`: at once, or with `timing` (now, t in ms) note k of
    /// them, low to high for t ≥ 0 and high to low below, at now + k·|t|.
    fn start(
        &mut self,
        owner: Owner,
        (strip, cell): (usize, usize),
        mut pitches: Vec<i32>,
        timing: Option<(f64, f32)>,
        velocity: f32,
        out: &mut Vec<Note>,
    ) {
        pitches.sort_unstable();
        let mut voice = Voice {
            owner,
            strip,
            cell,
            sounding: Vec::new(),
            pending: Vec::new(),
        };
        let (now, t) = timing.unwrap_or((0.0, 0.0));
        if t < 0.0 {
            pitches.reverse();
        }
        for (k, pitch) in pitches.into_iter().enumerate() {
            let at = now + k as f64 * f64::from(t.abs()) / 1000.0;
            if k == 0 || t == 0.0 {
                self.on(pitch, velocity, out);
                voice.sounding.push(pitch);
            } else {
                voice.pending.push((at, pitch));
            }
        }
        self.voices.push(voice);
    }

    /// Voice `i` stops: its onsets still to come are dropped (those due by now went out with the frame's tick), and
    /// every pitch it sounds goes off.
    fn stop(&mut self, i: usize, out: &mut Vec<Note>) {
        let voice = self.voices.remove(i);
        for pitch in voice.sounding {
            self.off(pitch, out);
        }
    }

    fn stop_all(&mut self, out: &mut Vec<Note>) {
        while !self.voices.is_empty() {
            self.stop(0, out);
        }
        for t in &mut self.touches {
            t.cell = None;
            t.spent = true;
        }
    }

    /// Voice `i` slides onto `cell`, playing `pitches`: pitches only in the old set are cancelled or turned off, pitches
    /// only in the new one go on at once, and shared ones stay as they are.
    fn slide(
        &mut self,
        i: usize,
        cell: usize,
        pitches: Vec<i32>,
        velocity: f32,
        out: &mut Vec<Note>,
    ) {
        let mut voice = std::mem::replace(
            &mut self.voices[i],
            Voice {
                owner: Owner::Latched,
                strip: 0,
                cell: 0,
                sounding: Vec::new(),
                pending: Vec::new(),
            },
        );
        voice.pending.retain(|(_, p)| pitches.contains(p));
        let (keep, gone): (Vec<i32>, Vec<i32>) = voice
            .sounding
            .into_iter()
            .partition(|p| pitches.contains(p));
        for p in gone {
            self.off(p, out);
        }
        voice.sounding = keep;
        for p in pitches {
            if !voice.sounding.contains(&p) && !voice.pending.iter().any(|&(_, q)| q == p) {
                self.on(p, velocity, out);
                voice.sounding.push(p);
            }
        }
        voice.cell = cell;
        self.voices[i] = voice;
    }

    /// Onsets due by `now` go out.
    fn tick(&mut self, now: f64, velocity: f32, out: &mut Vec<Note>) {
        for i in 0..self.voices.len() {
            let due: Vec<i32> = (self.voices[i].pending.iter())
                .filter(|&&(at, _)| at <= now)
                .map(|&(_, p)| p)
                .collect();
            self.voices[i].pending.retain(|&(at, _)| at > now);
            for p in due {
                self.on(p, velocity, out);
                self.voices[i].sounding.push(p);
            }
        }
    }

    fn finger_voice(&self, finger: u64) -> Option<usize> {
        self.voices
            .iter()
            .position(|v| v.owner == Owner::Finger(finger))
    }

    /// Draws the keyboard over `rect` (its [`chord_rect`]) in the channel colour `channel` and plays its touches.
    /// Returns the notes for the live input.
    pub fn show(
        &mut self,
        ui: &mut Ui,
        cx: &UiCtx,
        rect: Rect,
        channel: Rgba,
        input: ChordInput,
    ) -> Vec<Note> {
        let m = cx.m;
        let mut out = Vec::new();
        let now = ui.input(|i| i.time);
        let velocity = input.velocity.clamp(1.0 / 127.0, 1.0);
        // New pitches stop every note first.
        let source = (input.set.clone(), input.shift, input.strum);
        if self.made_from.as_ref() != Some(&source) {
            // The editor's own voice goes on: its edit is the change.
            if self.made_from.is_some() {
                while let Some(i) = self.voices.iter().position(|v| v.owner != Owner::Editor) {
                    self.stop(i, &mut out);
                }
                for t in &mut self.touches {
                    (t.cell, t.spent) = (None, true);
                }
            }
            self.made_from = Some(source);
        }
        // Hold turned off releases the latched cells.
        if self.held && !input.hold {
            while let Some(i) = self.voices.iter().position(|v| v.owner == Owner::Latched) {
                self.stop(i, &mut out);
            }
        }
        self.held = input.hold;
        self.tick(now, velocity, &mut out);

        let count = if input.editor {
            1
        } else if input.strum {
            STRUM_CELLS
        } else {
            CHORD_CELLS
        };
        let strips = strips(rect);
        let layouts: Vec<StripLayout> = strips.iter().map(|&s| strip_layout(s, count, m)).collect();
        let response = ui.interact(rect, Id::new("chord_keyboard"), Sense::click_and_drag());
        if ui.input(focus_lost) || !input.enabled {
            for t in std::mem::take(&mut self.touches) {
                if let Some(i) = self.finger_voice(t.finger) {
                    self.stop(i, &mut out);
                }
            }
        }
        let chord = |s: usize| &input.set.chords[s.min(input.set.chords.len() - 1)];
        let pitches = |s: usize, cell: usize| {
            cell_pitches(chord(s), input.strum, input.editor, cell, input.shift)
        };
        let timing = (!input.strum && !input.editor).then_some((now, input.timing_ms));
        for finger in fingers(ui, response.id)
            .into_iter()
            .filter(|_| input.enabled)
        {
            match finger {
                Finger::Down(id, pos) => {
                    let Some(strip) = strips.iter().position(|s| s.contains(pos)) else {
                        continue;
                    };
                    self.target = strip;
                    let mut touch = Touch {
                        finger: id,
                        strip,
                        cell: None,
                        from_label: false,
                        spent: false,
                    };
                    match layouts[strip].hit(pos) {
                        Hit::Label => touch.from_label = true,
                        Hit::Off => touch.spent = true,
                        Hit::Cell(cell) => {
                            let latched = self.voices.iter().position(|v| {
                                v.owner == Owner::Latched && (v.strip, v.cell) == (strip, cell)
                            });
                            match latched {
                                Some(i) if input.hold => {
                                    self.stop(i, &mut out);
                                    touch.spent = true;
                                }
                                _ => {
                                    if input.hold {
                                        self.stop_all(&mut out);
                                    }
                                    let owner = Owner::Finger(id);
                                    let p = pitches(strip, cell);
                                    self.start(owner, (strip, cell), p, timing, velocity, &mut out);
                                    touch.cell = Some(cell);
                                }
                            }
                        }
                    }
                    self.touches.push(touch);
                }
                Finger::Move(id, pos) => {
                    let Some(k) = self.touches.iter().position(|t| t.finger == id) else {
                        continue;
                    };
                    let t = self.touches[k];
                    let Hit::Cell(cell) = layouts[t.strip].hit(pos) else {
                        continue;
                    };
                    if t.spent || t.cell == Some(cell) {
                        continue;
                    }
                    if t.from_label {
                        // From the label onto a cell: every strip's notes stop, then it plays as a press.
                        self.stop_all(&mut out);
                        let owner = Owner::Finger(id);
                        let p = pitches(t.strip, cell);
                        self.start(owner, (t.strip, cell), p, timing, velocity, &mut out);
                    } else if let Some(i) = self.finger_voice(id) {
                        self.slide(i, cell, pitches(t.strip, cell), velocity, &mut out);
                    }
                    let t = &mut self.touches[k];
                    (t.cell, t.from_label, t.spent) = (Some(cell), false, false);
                }
                Finger::Up(id, pos) => {
                    let Some(k) = self.touches.iter().position(|t| t.finger == id) else {
                        continue;
                    };
                    let t = self.touches.remove(k);
                    let Some(i) = self.finger_voice(id) else {
                        continue;
                    };
                    match layouts[t.strip].hit(pos) {
                        // On the label the cell stays on; over a cell, Hold latches it.
                        Hit::Label => self.voices[i].owner = Owner::Latched,
                        Hit::Cell(_) if input.hold => self.voices[i].owner = Owner::Latched,
                        _ => self.stop(i, &mut out),
                    }
                }
            }
        }
        // The cells' highlights: on while a voice holds the cell, then fading.
        for (s, glows) in self.glow.iter_mut().enumerate() {
            for (c, b) in glows.iter_mut().enumerate() {
                if self.voices.iter().any(|v| (v.strip, v.cell) == (s, c)) {
                    *b = 1.0;
                } else {
                    *b *= FADE;
                    if *b < FADE_FLOOR {
                        *b = 0.0;
                    }
                }
            }
        }
        if let Some(next) = (self.voices.iter())
            .flat_map(|v| v.pending.iter().map(|p| p.0))
            .reduce(f64::min)
        {
            ui.ctx()
                .request_repaint_after_secs((next - now).max(0.0) as f32);
        }
        if self.glow.iter().flatten().any(|&b| b > 0.0 && b < 1.0) {
            ui.ctx().request_repaint();
        }
        self.paint(ui, cx, &strips, &layouts, channel, input);
        out
    }

    fn paint(
        &self,
        ui: &Ui,
        cx: &UiCtx,
        strips: &[Rect; STRIPS],
        layouts: &[StripLayout],
        channel: Rgba,
        input: ChordInput,
    ) {
        let (m, colors) = (cx.m, &cx.theme.colors);
        let painter = ui.painter();
        let ink = channel.to_color32();
        let (d, line) = (m.t(), 1.0 / m.ppp());
        for (s, (strip, layout)) in strips.iter().zip(layouts).enumerate() {
            let a = if input.editor && s != self.target {
                0.5
            } else {
                1.0
            };
            // 1. Opaque black (a fixed colour, not a theme role), 2. the channel colour at a over it.
            painter.rect_filled(*strip, 0.0, Color32::BLACK);
            painter.rect_filled(*strip, 0.0, ink.gamma_multiply(a));
            // 3. The cells, each with a 1 px `background` line along its bottom.
            for cell in &layout.cells {
                painter.rect_filled(*cell, 0.0, colors.control_fill.to_color32());
                let bottom = cell.with_min_y(cell.bottom() - line);
                painter.rect_filled(bottom, 0.0, colors.background.to_color32());
            }
            // 4. The root-position dot (Chords, editor closed).
            if !input.strum && !input.editor {
                let at = layout.cells[ROOT_CELL].center();
                painter.circle_filled(at, m.u(DOT_R), ink);
            }
            // 5. The highlights, inset by d.
            for (c, cell) in layout.cells.iter().enumerate() {
                let b = self.glow[s][c];
                if b > 0.0 {
                    painter.rect_filled(cell.shrink(d), 0.0, ink.gamma_multiply(b));
                }
            }
            // 6. The label row, brighter while opaque; 7. the chord's name over it.
            let label = if a == 1.0 { brighter(channel) } else { channel };
            let fill = label.to_color32().gamma_multiply(a);
            painter.rect_filled(layout.label, 0.0, fill);
            let chord = &input.set.chords[s.min(input.set.chords.len() - 1)];
            let text = colors.best_contrasting(label, &colors.contrast_candidates());
            painter.text(
                layout.label.center(),
                Align2::CENTER_CENTER,
                chord_name(chord),
                m.font_id(FontSize::Pt10),
                text.to_color32(),
            );
        }
    }
}

/// `c` with its RGB × 1.1111, clamped.
fn brighter(c: Rgba) -> Rgba {
    let up = |v: f32| (v * LABEL_GAIN).min(1.0);
    Rgba {
        r: up(c.r),
        g: up(c.g),
        b: up(c.b),
        ..c
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::metrics::DEFAULT_UI_SCALE;
    use crate::ui::offscreen::Offscreen;
    use eframe::egui::{Event, PointerButton, TouchDeviceId, TouchId, TouchPhase};

    const U: f32 = 32.0;
    /// A tall chord keyboard: 440 points, so the six cells are 67.33 points each above a label row at 508.
    const RECT: Rect = Rect {
        min: pos2(0.0, 100.0),
        max: pos2(1170.0, 540.0),
    };

    fn metrics(ppp: f32) -> Metrics {
        Metrics::from_pixels_per_point(ppp, DEFAULT_UI_SCALE)
    }

    #[test]
    fn the_strips_are_eight_equal_with_a_u_label_at_the_bottom_and_a_column_of_at_least_6_u() {
        for ppp in [1.0, 2.0] {
            let m = metrics(ppp);
            let (u, d) = (m.u(1.0), m.t());
            let strips = strips(RECT);
            assert_eq!(strips[0].left(), 0.0);
            assert!((strips[7].right() - 1170.0).abs() < 1e-3);
            assert!(
                strips
                    .iter()
                    .all(|s| (s.width() - 1170.0 / 8.0).abs() < 1e-3)
            );
            let s = strips[2];
            let l = strip_layout(s, CHORD_CELLS, &m);
            assert_eq!(
                l.label,
                Rect::from_min_size(pos2(s.left() + d, 540.0 - u), vec2(s.width() - 2.0 * d, u))
            );
            assert_eq!(l.cells.len(), 6);
            assert_eq!(l.cells[0].top(), 100.0 + d);
            let h = (440.0 - 2.0 * d - u) / 6.0;
            assert!(
                l.cells
                    .iter()
                    .all(|c| (c.height() - h).abs() < 1e-3 && c.left() == s.left() + d)
            );
            // A short strip: the label at its top, the column still 6 U, overflowing below.
            let short = Rect::from_min_size(pos2(0.0, 436.0), vec2(146.25, 0.5 * u));
            let l = strip_layout(short, STRUM_CELLS, &m);
            assert_eq!(l.label.top(), 436.0);
            let column = l.cells.last().unwrap().bottom() - l.cells[0].top();
            assert!((column - 6.0 * u).abs() < 1e-3, "{column}");
            // One cell (the editor): from the column's top to the label's top.
            let l = strip_layout(s, 1, &m);
            assert_eq!(
                l.cells,
                [Rect::from_x_y_ranges(
                    s.left() + d..=s.right() - d,
                    100.0 + d..=540.0 - u
                )]
            );
        }
        // The wheels setting alone moves the strips 4 U in.
        let m = metrics(1.0);
        assert_eq!(chord_rect(RECT, true, &m).left(), 4.0 * U);
        assert_eq!(chord_rect(RECT, false, &m), RECT);
    }

    #[test]
    fn the_label_wins_where_it_overlaps_a_cell_and_the_border_is_neither() {
        let m = metrics(1.0);
        let s = strips(Rect::from_min_size(pos2(0.0, 436.0), vec2(1170.0, 104.0)))[0];
        let l = strip_layout(s, CHORD_CELLS, &m);
        // The column (6 U) runs past the label (508…540): there the label wins.
        assert_eq!(l.hit(pos2(70.0, 450.0)), Hit::Cell(0));
        assert_eq!(l.hit(pos2(70.0, 480.0)), Hit::Cell(1));
        assert_eq!(l.hit(pos2(70.0, 520.0)), Hit::Label);
        assert_eq!(l.hit(pos2(0.5, 450.0)), Hit::Off);
    }

    #[test]
    fn chord_names_are_the_root_the_quality_and_the_highest_extension() {
        let c = |base_note, quality, extensions: &[i32]| Chord {
            base_note,
            extensions: extensions.to_vec(),
            octave: 4,
            quality,
        };
        assert_eq!(chord_name(&c(0, 0, &[])), "C ");
        assert_eq!(chord_name(&c(2, 1, &[])), "D min ");
        assert_eq!(chord_name(&c(11, 3, &[])), "B dim ");
        assert_eq!(chord_name(&c(1, 6, &[])), "C# / Db power ");
        // Only the highest extension: C major + {11, 14} is `C 9`.
        assert_eq!(chord_name(&c(0, 0, &[11, 14])), "C 9");
        assert_eq!(chord_name(&c(9, 1, &[10])), "A min 7");
        assert_eq!(chord_name(&c(7, 4, &[18])), "G Sus2 #11");
        let set = ChordSet::default_set(false).transposed(0);
        let names: Vec<String> = set.chords.iter().map(chord_name).collect();
        assert_eq!(
            names,
            [
                "C ", "D min ", "E min ", "F ", "G ", "A min ", "B dim ", "C "
            ]
        );
    }

    #[test]
    fn chords_cells_play_inversions_3_to_minus_2_and_strum_cells_indices_11_to_0() {
        let set = ChordSet::default_set(false).transposed(0);
        let c = &set.chords[0];
        let at = |cell| cell_pitches(c, false, false, cell, 0);
        assert_eq!(at(3), [36, 40, 43], "the root position");
        assert_eq!(at(0), [48, 52, 55], "+3");
        assert_eq!(at(4), [31, 36, 40], "−1: the 5th an octave down, root, 3rd");
        assert_eq!(at(5), [28, 31, 36], "−2");
        // Strum: the bottom cell is the root at the chord octave, then up the compact list.
        let strum = |cell| cell_pitches(c, true, false, cell, 0);
        assert_eq!(strum(11), [36]);
        assert_eq!(strum(10), [40]);
        assert_eq!(strum(8), [48]);
        assert_eq!(strum(0), [36 + 3 * 12 + 7]);
        // The octave shift, and the editor's inversion 0.
        assert_eq!(cell_pitches(c, false, false, 3, -1), [24, 28, 31]);
        assert_eq!(cell_pitches(c, true, true, 0, 0), [36, 40, 43]);
    }

    /// A chord keyboard over [`RECT`] in an offscreen context, with its inputs.
    struct Rig {
        screen: Offscreen,
        kb: ChordKeyboard,
        set: ChordSet,
        strum: bool,
        timing_ms: f32,
        hold: bool,
        shift: i32,
        shown: bool,
        notes: Vec<Note>,
    }

    impl Rig {
        fn new() -> Self {
            let mut rig = Self {
                screen: Offscreen::new(),
                kb: ChordKeyboard::default(),
                set: ChordSet::default_set(false).transposed(0),
                strum: false,
                timing_ms: 0.0,
                hold: false,
                shift: 0,
                shown: true,
                notes: Vec::new(),
            };
            rig.frame(vec![]);
            rig.take();
            rig
        }

        fn frame(&mut self, events: Vec<Event>) {
            let Self { kb, set, notes, .. } = self;
            let input = ChordInput {
                set,
                strum: self.strum,
                shift: self.shift,
                velocity: 0.5,
                timing_ms: self.timing_ms,
                hold: self.hold,
                editor: false,
                enabled: true,
            };
            let shown = self.shown;
            self.screen.frame(events, &mut |ui, cx| {
                if shown {
                    notes.extend(kb.show(ui, cx, RECT, Rgba::hex(0x3080F0), input));
                } else {
                    notes.extend(kb.release_all());
                }
            });
        }

        /// The notes since the last call, as signed keys: +key on, −key off.
        fn take(&mut self) -> Vec<i32> {
            std::mem::take(&mut self.notes)
                .into_iter()
                .map(|n| match n {
                    Note::On(k, v) => {
                        assert_eq!(v, 0.5);
                        i32::from(k)
                    }
                    Note::Off(k) => -i32::from(k),
                })
                .collect()
        }

        fn touch(&mut self, id: u64, phase: TouchPhase, pos: Pos2) {
            self.frame(vec![Event::Touch {
                device_id: TouchDeviceId(0),
                id: TouchId(id),
                phase,
                pos,
                force: None,
            }]);
        }

        fn mouse(&mut self, pos: Pos2, pressed: Option<bool>) {
            let e = match pressed {
                None => Event::PointerMoved(pos),
                Some(pressed) => Event::PointerButton {
                    pos,
                    button: PointerButton::Primary,
                    pressed,
                    modifiers: Default::default(),
                },
            };
            self.frame(vec![e]);
        }
    }

    /// The centre of `cell` of strip `s` in [`RECT`] (six cells, or twelve with `strum`), or of its label.
    fn cell(s: usize, cell: usize, strum: bool) -> Pos2 {
        let l = strip_layout(strips(RECT)[s], if strum { 12 } else { 6 }, &metrics(1.0));
        l.cells[cell].center()
    }

    fn label(s: usize) -> Pos2 {
        strip_layout(strips(RECT)[s], 6, &metrics(1.0))
            .label
            .center()
    }

    #[test]
    fn a_press_plays_its_inversion_at_once_and_the_release_stops_it() {
        let mut rig = Rig::new();
        rig.touch(1, TouchPhase::Start, cell(0, 3, false));
        assert_eq!(rig.take(), [36, 40, 43]);
        assert_eq!(rig.kb.target, 0);
        rig.touch(1, TouchPhase::End, cell(0, 3, false));
        assert_eq!(rig.take(), [-36, -40, -43]);
        // The label plays nothing; another strip becomes the target.
        rig.touch(2, TouchPhase::Start, label(4));
        rig.touch(2, TouchPhase::End, label(4));
        assert_eq!(rig.take(), Vec::<i32>::new());
        assert_eq!(rig.kb.target, 4);
    }

    #[test]
    fn timing_spaces_the_onsets_low_to_high_or_high_to_low_and_a_release_cancels_the_rest() {
        // +40 ms: 0, 40, 80 ms on the real-time clock (frames 1/60 s apart).
        let mut rig = Rig::new();
        rig.timing_ms = 40.0;
        rig.touch(1, TouchPhase::Start, cell(0, 3, false));
        assert_eq!(rig.take(), [36]);
        let mut at = Vec::new();
        for frame in 1..=6 {
            rig.frame(vec![]);
            at.extend(rig.take().into_iter().map(|k| (frame, k)));
        }
        assert_eq!(at, [(3, 40), (5, 43)]);
        rig.touch(1, TouchPhase::End, cell(0, 3, false));
        assert_eq!(rig.take(), [-36, -40, -43]);
        // −40 ms: high to low; let go after the second onset, the third never comes.
        rig.timing_ms = -40.0;
        rig.touch(1, TouchPhase::Start, cell(0, 3, false));
        assert_eq!(rig.take(), [43]);
        for _ in 0..3 {
            rig.frame(vec![]);
        }
        assert_eq!(rig.take(), [40]);
        rig.touch(1, TouchPhase::End, cell(0, 3, false));
        assert_eq!(rig.take(), [-43, -40]);
        for _ in 0..6 {
            rig.frame(vec![]);
        }
        assert_eq!(rig.take(), Vec::<i32>::new());
    }

    #[test]
    fn a_slide_to_another_inversion_keeps_the_shared_pitches_and_stays_on_its_strip() {
        let mut rig = Rig::new();
        rig.mouse(cell(0, 3, false), None);
        rig.mouse(cell(0, 3, false), Some(true));
        assert_eq!(rig.take(), [36, 40, 43]);
        // Inversion 0 → −1 (31, 36, 40): 43 off, 31 on, 36 and 40 sustained.
        rig.mouse(cell(0, 4, false), None);
        assert_eq!(rig.take(), [-43, 31]);
        // Into another strip: nothing; the finger stays with its own.
        rig.mouse(cell(1, 2, false), None);
        assert_eq!(rig.take(), Vec::<i32>::new());
        // Let go off the cells (the next strip): everything stops.
        rig.mouse(cell(1, 2, false), Some(false));
        assert_eq!(rig.take(), [-36, -40, -31]);
    }

    #[test]
    fn strum_cells_are_single_notes_that_slide_off_and_on_with_no_timing() {
        let mut rig = Rig::new();
        rig.strum = true;
        rig.timing_ms = 40.0;
        rig.frame(vec![]);
        rig.mouse(cell(2, 11, true), None);
        rig.mouse(cell(2, 11, true), Some(true));
        // E min at octave 3: E = 40.
        assert_eq!(rig.take(), [40]);
        rig.mouse(cell(2, 10, true), None);
        rig.mouse(cell(2, 9, true), None);
        assert_eq!(rig.take(), [-40, 43, -43, 47]);
        rig.mouse(cell(2, 9, true), Some(false));
        assert_eq!(rig.take(), [-47]);
    }

    #[test]
    fn a_pitch_two_strips_share_retriggers_and_goes_off_with_its_last_holder() {
        let mut rig = Rig::new();
        // C (36 40 43) and E min +0 (40 43 47) share 40 and 43.
        rig.touch(1, TouchPhase::Start, cell(0, 3, false));
        rig.touch(2, TouchPhase::Start, cell(2, 3, false));
        assert_eq!(rig.take(), [36, 40, 43, -40, 40, -43, 43, 47]);
        rig.touch(1, TouchPhase::End, cell(0, 3, false));
        assert_eq!(rig.take(), [-36]);
        rig.touch(2, TouchPhase::End, cell(2, 3, false));
        assert_eq!(rig.take(), [-40, -43, -47]);
    }

    #[test]
    fn hold_latches_a_tap_a_new_tap_stops_all_first_a_lit_tap_turns_it_off_and_hold_off_releases() {
        let mut rig = Rig::new();
        rig.hold = true;
        rig.touch(1, TouchPhase::Start, cell(0, 3, false));
        rig.touch(1, TouchPhase::End, cell(0, 3, false));
        assert_eq!(rig.take(), [36, 40, 43], "latched");
        rig.touch(1, TouchPhase::Start, cell(3, 3, false));
        rig.touch(1, TouchPhase::End, cell(3, 3, false));
        assert_eq!(rig.take(), [-36, -40, -43, 41, 45, 48]);
        // A lit cell's tap turns it off.
        rig.touch(1, TouchPhase::Start, cell(3, 3, false));
        rig.touch(1, TouchPhase::End, cell(3, 3, false));
        assert_eq!(rig.take(), [-41, -45, -48]);
        // Released off the cells, a latched press goes off even with Hold.
        rig.mouse(cell(4, 3, false), None);
        rig.mouse(cell(4, 3, false), Some(true));
        rig.mouse(pos2(cell(4, 3, false).x, 50.0), None);
        rig.mouse(pos2(cell(4, 3, false).x, 50.0), Some(false));
        assert_eq!(rig.take(), [43, 47, 50, -43, -47, -50]);
        // Hold off releases every latched cell.
        rig.touch(1, TouchPhase::Start, cell(5, 3, false));
        rig.touch(1, TouchPhase::End, cell(5, 3, false));
        rig.take();
        rig.hold = false;
        rig.frame(vec![]);
        assert_eq!(rig.take(), [-45, -48, -52]);
    }

    #[test]
    fn a_release_on_the_label_leaves_the_cell_on_and_a_slide_from_the_label_stops_every_strip_first()
     {
        let mut rig = Rig::new();
        rig.mouse(cell(0, 3, false), None);
        rig.mouse(cell(0, 3, false), Some(true));
        rig.mouse(label(0), None);
        rig.mouse(label(0), Some(false));
        assert_eq!(rig.take(), [36, 40, 43], "left on");
        // From strip 1's label onto its root cell: every note stops, then D min plays.
        rig.mouse(label(1), Some(true));
        rig.mouse(cell(1, 3, false), None);
        assert_eq!(rig.take(), [-36, -40, -43, 38, 41, 45]);
        rig.mouse(cell(1, 3, false), Some(false));
        assert_eq!(rig.take(), [-38, -41, -45]);
    }

    #[test]
    fn a_new_octave_shift_or_set_stops_everything_and_so_does_hiding_the_keyboard() {
        let mut rig = Rig::new();
        rig.touch(1, TouchPhase::Start, cell(0, 3, false));
        rig.take();
        rig.shift = 1;
        rig.frame(vec![]);
        assert_eq!(rig.take(), [-36, -40, -43]);
        // The finger plays nothing more until it lifts.
        rig.touch(1, TouchPhase::Move, cell(0, 4, false));
        rig.touch(1, TouchPhase::End, cell(0, 4, false));
        assert_eq!(rig.take(), Vec::<i32>::new());
        rig.touch(2, TouchPhase::Start, cell(0, 3, false));
        assert_eq!(rig.take(), [48, 52, 55]);
        rig.set = ChordSet::default_set(true).transposed(0);
        rig.frame(vec![]);
        assert_eq!(rig.take(), [-48, -52, -55]);
        rig.touch(3, TouchPhase::Start, cell(0, 3, false));
        assert_eq!(rig.take(), [48, 51, 55], "C min");
        rig.shown = false;
        rig.frame(vec![]);
        assert_eq!(rig.take(), [-48, -51, -55]);
    }
}
