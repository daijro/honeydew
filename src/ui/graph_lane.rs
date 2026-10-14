//! The editors' graph lane: a strip across the editor's bottom, under the grid, with a vertical item list on its left
//! (`Vel` / `Pan` in the piano roll; `Velocity` / `Panning` / `Pitch` / `Repeat` in the drum editor, `Repeat` only in
//! step-sequencer mode) and a value area that shows and paints one property of the notes or hits.
//!
//! This module holds what both editors share: the lane's size and its knob ([`GraphLane`]), the strip's common
//! drawing and list ([`frame`]), the value laws ([`Prop`]), the note tint and the piano roll's paint rule
//! ([`paint_notes`]). Each editor draws its own value area and routes its own edits.
//!
//! - The knob's press growth reaches its 10 px over [`GROW_S`]; the release wobble is a damped cosine
//!   ([`WOBBLE_S`], [`WOBBLE_PERIOD`]). With the lane hidden the knob sits on the editor's bottom edge, clipped to
//!   the editor (its upper half shows and takes touches), so it never covers the transport bar.
//! - A knob drag released below 0.1 hides the lane and keeps the last f; the next show restores it.
//! - The knob's 2×2 U touch box wins wherever it overlaps what lies under it: the list's first item and, in the drum
//!   editor, a row header cut off at the rows' bottom, whose visible part may be a sliver. A header's taps have a
//!   whole row elsewhere. It holds because the editor interacts the knob last and the knob senses drags: egui then
//!   gives it every press in its box, even over a click-only widget thinner than its thin-target rule.
//! - The note tint is the muted playlist clip's tint (the miniature's three stops) over the channel colour, or over
//!   the selected (whitened) colour for selected notes. Its middle stop is 0.66, shared with the playlist through
//!   `clips::sample`, so the piano roll's, the drum grid's and the step sequencer's hit colours move with it.
//! - The drum editor's spacer (the rest of the header column, the mixer's c·12 U, while c > 0) shows the current
//!   row's slot's sample ([`spacer_wave`]) by the audio clip editor's waveform law: the whole source sample fitted to
//!   the spacer's width (no Speed, stretch or Sample Start), REV mirrored, in the plain opaque channel colour. A slot
//!   whose sample is missing leaves it empty.
//! - The `Show graph editor` / `Close graph editor` shortcuts and the gamepad steps are not wired. `showGraphEditor`
//!   is bound only to the gamepad's LB+RB+↓, the app reads no gamepad (`keymap.rs` skips those entries), and neither
//!   built-in keyboard map binds it, so nothing could fire it. The focus system (`ui/focus.rs`) is there: wiring them
//!   is a global action (`app/keys.rs`) that shows this lane and focuses a container round it, and a `close` that
//!   hides it while the focus is inside. Each editor remembers the last Vel / Pan painted for its new notes and hits
//!   (its remembered note or hit).

use std::f64::consts::TAU;

use eframe::egui::{Color32, Id, Mesh, Painter, Rangef, Rect, Sense, Ui, Vec2, pos2};

use crate::metrics::Metrics;
use crate::sample_cache::Waveform;
use crate::song::Event;
use crate::theme::{Rgba, ThemeColors};
use crate::ui::audio_editor::{Wave, add_column, columns, pixel_columns, wave_rows};
use crate::ui::clips::{mix, sample, tint_stops};
use crate::ui::piano_roll::notes::note;
use crate::ui::widgets::{Bar, ItemList, item_list_index};
use crate::ui::{UiCtx, gesture, primary_down_live};

/// The lane is at least this tall, in U.
const MIN_H: f32 = 2.0;
/// The height fraction f: its cap, the release below which the lane hides, and the first show's.
const F_MAX: f32 = 0.7;
const F_HIDE: f32 = 0.1;
const F_FIRST: f32 = 0.5;
/// The gap over the list and the value area, in device pixels.
const GAP_PX: f32 = 4.0;
/// The knob: its touch box and its dot, in U; a tap is no long press ([`gesture::is_long`]).
const KNOB_BOX: f32 = 2.0;
const KNOB_D: f32 = 1.0;
/// The knob's press growth, in device pixels, and the time it takes.
const GROW_PX: f32 = 10.0;
const GROW_S: f64 = 0.1;
/// The release wobble: how long, and its period.
const WOBBLE_S: f64 = 0.3;
const WOBBLE_PERIOD: f64 = 0.12;
/// A drum lane's Panning / Pitch dead zone at the centre, in U.
const DEAD_ZONE: f32 = 1.0;

/// A note or drum-hit property the lane edits.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Prop {
    /// Velocity, /255.
    Velocity,
    /// Pan, /254 in the lane, centre 127.
    Pan,
    /// A drum hit's per-hit Pitch (`value`), /65534, centre 0x7FFF.
    Pitch,
    /// A drum hit's per-hit Repeat count: 0..=7 in the lane (a file may hold 8).
    Repeat,
}

/// The piano roll's items.
pub const ROLL_PROPS: [Prop; 2] = [Prop::Velocity, Prop::Pan];
pub const ROLL_LABELS: [&str; 2] = ["Vel", "Pan"];
/// The drum editor's items; [`drum_items`] says how many show.
pub const DRUM_PROPS: [Prop; 4] = [Prop::Velocity, Prop::Pan, Prop::Pitch, Prop::Repeat];
pub const DRUM_LABELS: [&str; 4] = ["Velocity", "Panning", "Pitch", "Repeat"];

/// How many of [`DRUM_PROPS`] the drum editor lists: `Repeat` only in step-sequencer mode.
pub fn drum_items(step_mode: bool) -> usize {
    if step_mode { 4 } else { 3 }
}

impl Prop {
    /// The value stored for a paint at `v` in [0, 1]: Vel = clamp(trunc(255 v), 1, 255), Pan = clamp(trunc(254 v), 0,
    /// 254), Pitch = trunc(65534 v), Repeat = round(7 v).
    pub fn stored(self, v: f32) -> u16 {
        let v = f64::from(v.clamp(0.0, 1.0));
        let x = match self {
            Self::Velocity => (255.0 * v).trunc().max(1.0),
            Self::Pan => (254.0 * v).trunc(),
            Self::Pitch => (65534.0 * v).trunc(),
            Self::Repeat => (7.0 * v).round(),
        };
        x as u16
    }

    /// Where a stored value sits in [0, 1] for drawing: vel/255, pan/254 (a file's 255 is clamped), pitch/65534,
    /// repeat/7.
    pub fn frac(self, value: u16) -> f32 {
        let scale = match self {
            Self::Velocity => 255.0,
            Self::Pan => 254.0,
            Self::Pitch => 65534.0,
            Self::Repeat => 7.0,
        };
        (f32::from(value) / scale).min(1.0)
    }

    /// Where a stored value sits on the note tint on this page (both drum modes): Velocity
    /// vel/255; Panning min(1, 2·|pan/254 − 0.5|); Pitch min(1, 2·|pitch/65535 − 0.5|); Repeat clamp(r/8, 0, 1).
    pub fn tint_t(self, value: u16) -> f32 {
        let v = f32::from(value);
        let off_centre = |scale: f32| (2.0 * (v / scale - 0.5).abs()).min(1.0);
        match self {
            Self::Velocity => v / 255.0,
            Self::Pan => off_centre(254.0),
            Self::Pitch => off_centre(65535.0),
            Self::Repeat => (v / 8.0).min(1.0),
        }
    }

    /// How far this page pulls the tint's bottom stop toward its middle one ([`tint`]): Velocity and Repeat 0.5,
    /// Panning 0.75, Pitch none.
    pub fn pull(self) -> f32 {
        match self {
            Self::Velocity | Self::Repeat => 0.5,
            Self::Pan => 0.75,
            Self::Pitch => 0.0,
        }
    }

    /// `e`'s value, `None` for an event without this property.
    pub fn get(self, e: &Event) -> Option<u16> {
        match (self, *e) {
            (Self::Velocity, Event::Note { velocity, .. } | Event::Drum { velocity, .. }) => {
                Some(velocity.into())
            }
            (Self::Pan, Event::Note { pan, .. } | Event::Drum { pan, .. }) => Some(pan.into()),
            (Self::Pitch, Event::Drum { value, .. }) => Some(value),
            (Self::Repeat, Event::Drum { repeat, .. }) => Some(repeat.into()),
            _ => None,
        }
    }

    /// Sets `e`'s value to `v` (a byte field saturates); events without this property stay as they are.
    pub fn set(self, e: &mut Event, v: u16) {
        let byte = u8::try_from(v).unwrap_or(u8::MAX);
        match (self, e) {
            (Self::Velocity, Event::Note { velocity, .. } | Event::Drum { velocity, .. }) => {
                *velocity = byte;
            }
            (Self::Pan, Event::Note { pan, .. } | Event::Drum { pan, .. }) => *pan = byte,
            (Self::Pitch, Event::Drum { value, .. }) => *value = v,
            (Self::Repeat, Event::Drum { repeat, .. }) => *repeat = byte,
            _ => {}
        }
    }
}

/// The paint value v = (bottom − y)/h for a pointer at `y` over the value area `content`, y clamped to it.
pub fn v_at(content: Rect, y: f32) -> f32 {
    (content.bottom() - y.clamp(content.top(), content.bottom())) / content.height()
}

/// [`v_at`] with the drum lane's Panning / Pitch dead zone: a 1 U zone at the centre gives 0.5, the rest maps linearly
/// onto 0–0.5 below and 0.5–1 above.
pub fn v_centred(content: Rect, y: f32, m: &Metrics) -> f32 {
    let half = content.height() / 2.0;
    let dz = (m.u(DEAD_ZONE) / 2.0).min(half);
    let d = content.center().y - y.clamp(content.top(), content.bottom());
    if d.abs() <= dz || half <= dz {
        return 0.5;
    }
    0.5 + d.signum() * 0.5 * (d.abs() - dz) / (half - dz)
}

/// The note tint's stops for `content` (see the module doc), the bottom one pulled `pull` of the way toward the
/// middle one.
pub fn tint(colors: &ThemeColors, content: Rgba, pull: f32) -> [Rgba; 3] {
    let [lo, mid, hi] = tint_stops(colors, content);
    [mix(lo, mid, pull), mid, hi]
}

/// The tint at `t` in [0, 1].
pub fn tint_at(stops: &[Rgba; 3], t: f32) -> Color32 {
    sample(stops, t.clamp(0.0, 1.0)).to_color32()
}

/// Selected notes and hits: the channel colour moved this far toward `text` ("whitened").
pub const SELECTED_TINT: f32 = 0.5;

/// The note and hit colours on the page `prop` (both drum modes): the note tint over the
/// channel colour `color` and over its selected ([`SELECTED_TINT`]) colour, the bottom stops pulled as the page says.
pub fn hit_tints(colors: &ThemeColors, color: Rgba, prop: Prop) -> [[Rgba; 3]; 2] {
    let pale = mix(color, colors.text, SELECTED_TINT);
    [color, pale].map(|c| tint(colors, c, prop.pull()))
}

/// The colour of a note or hit whose `prop` is `value`, from [`hit_tints`], `selected` or not.
pub fn hit_color(tints: &[[Rgba; 3]; 2], prop: Prop, value: u16, selected: bool) -> Color32 {
    tint_at(&tints[usize::from(selected)], prop.tint_t(value))
}

/// Sets `prop` to `value` on every eligible note whose extent [pos, pos + len) holds `tick`: all the notes the roll
/// shows, or only the selected ones while any is selected. Returns whether a note changed.
pub fn paint_notes(
    events: &mut [Event],
    selected: &[bool],
    prop: Prop,
    tick: f64,
    value: u16,
) -> bool {
    let any = selected.contains(&true);
    let mut changed = false;
    for (i, e) in events.iter_mut().enumerate() {
        let Some((pos, len, _)) = note(e) else {
            continue;
        };
        let eligible = !any || selected.get(i) == Some(&true);
        if eligible && pos <= tick && tick < pos + len && prop.get(e) != Some(value) {
            prop.set(e, value);
            changed = true;
        }
    }
    changed
}

/// The lane's state in one editor: the item shown, whether it is open and how tall, and its knob's gesture.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct GraphLane {
    /// The selected item, an index into the editor's list.
    pub item: usize,
    /// Shown. A new editor's lane is hidden.
    pub open: bool,
    /// The last height fraction shown, which the next show restores.
    pub f: f32,
    /// A knob drag in progress: its f, which the lane shows meanwhile.
    drag: Option<f32>,
    /// When the knob was pressed (egui time), while it is.
    pressed: Option<f64>,
    /// When it was let go, and its growth then (device pixels): the wobble.
    released: Option<(f64, f32)>,
}

impl Default for GraphLane {
    fn default() -> Self {
        Self {
            item: 0,
            open: false,
            f: F_FIRST,
            drag: None,
            pressed: None,
            released: None,
        }
    }
}

impl GraphLane {
    /// `area` (the editor below its top margin, H tall) split into the part above the lane and the lane strip, which
    /// is max(f·H, 2 U) tall (at most H), its top on a device pixel; no strip while hidden.
    pub fn split(&self, area: Rect, m: &Metrics) -> (Rect, Option<Rect>) {
        self.split_below(area, area.top(), m)
    }

    /// [`Self::split`] of the `editor` below its top margin (H tall) whose strip stays below `y` (the piano roll's
    /// ruler): the part from `y` down to the lane, and the strip, max(f·H, 2 U) tall but none of it above `y`.
    pub fn split_below(&self, editor: Rect, y: f32, m: &Metrics) -> (Rect, Option<Rect>) {
        let area = editor.with_min_y(y);
        let Some(f) = self.drag.or(self.open.then_some(self.f)) else {
            return (area, None);
        };
        let h = (f * editor.height()).max(m.u(MIN_H)).min(area.height());
        let top = m.round_px(area.bottom() - h);
        (area.with_max_y(top), Some(area.with_min_y(top)))
    }

    /// The knob of the lane in the editor `area` (below its top margin), centred at `x` on the grid/lane boundary, or
    /// on the editor's bottom edge while the lane is hidden: a tap toggles the lane, a vertical drag sizes it
    /// (f = 1 − (finger y − top)/H, at most 0.7; released below 0.1 it hides). Draw it after the grid and the lane.
    pub fn knob(&mut self, ui: &mut Ui, cx: &UiCtx, id: Id, area: Rect, x: f32) {
        self.knob_below(ui, cx, id, (area, area.top()), x);
    }

    /// [`Self::knob`] of the lane [`Self::split_below`] places: in the `editor` below its top margin (H, and the
    /// drag's top), on the part below `y`.
    pub fn knob_below(
        &mut self,
        ui: &mut Ui,
        cx: &UiCtx,
        id: Id,
        (editor, y): (Rect, f32),
        x: f32,
    ) {
        let m = cx.m;
        let area = editor.with_min_y(y);
        let (_, strip) = self.split_below(editor, y, m);
        let center = pos2(x, strip.map_or(area.bottom(), |s| s.top()));
        let touch = Rect::from_center_size(center, Vec2::splat(m.u(KNOB_BOX))).intersect(area);
        let response = ui.interact(touch, id, Sense::click_and_drag());
        // A press held over a focus loss is up (see `primary_down_live`).
        let live = primary_down_live(ui, id);
        let now = ui.input(|i| i.time);
        let down = response.is_pointer_button_down_on() && live;
        if down && self.pressed.is_none() {
            self.pressed = Some(now);
        }
        // A press and release within one frame leave no press time: a tap too.
        if response.clicked() && self.pressed.is_none_or(|t| !gesture::is_long(now - t)) {
            self.open = !self.open;
        }
        if response.dragged()
            && live
            && let Some(p) = response.interact_pointer_pos()
        {
            self.drag = Some((1.0 - (p.y - editor.top()) / editor.height()).clamp(0.0, F_MAX));
        }
        if (response.drag_stopped() || !live)
            && let Some(f) = self.drag.take()
        {
            if f < F_HIDE {
                self.open = false;
            } else {
                (self.open, self.f) = (true, f);
            }
        }
        if !down && let Some(t) = self.pressed.take() {
            self.released = Some((now, grow(now - t)));
        }

        // The dot: 1 U in `text` with a 1 px `background` rim at 30 %, grown while pressed, wobbling once let go.
        let px = 1.0 / m.ppp();
        let (growth, moving) = press_growth(self.pressed, self.released, now);
        if moving {
            ui.ctx().request_repaint();
        }
        let r = (m.u(KNOB_D) + growth * px) / 2.0;
        let colors = &cx.theme.colors;
        let painter = ui.painter_at(area);
        painter.circle_filled(center, r, colors.text.to_color32());
        let rim = colors.background.to_color32().gamma_multiply(0.3);
        painter.circle_stroke(center, r + px / 2.0, (px, rim));
    }
}

/// The knob's growth `held` seconds into a press, in device pixels (and every editor handle's: the audio editor's
/// stretch handle, the playlist selection's).
pub(crate) fn grow(held: f64) -> f32 {
    GROW_PX * (held / GROW_S).clamp(0.0, 1.0) as f32
}

/// An editor handle's press growth now, in device pixels, and whether it still moves: the graph lane knob's law,
/// shared by the audio editor's stretch handle and the playlist selection's handles. Pressed since `pressed`,
/// [`grow`]; let go at `t` grown `g`, the damped cosine g·cos(2πs/[`WOBBLE_PERIOD`])·(1 − s/[`WOBBLE_S`]) for
/// [`WOBBLE_S`]; else 0. It is not the knob law of the knobs (`widgets::pulse`, an exponential 0.6 a frame), which
/// knobs, sliders and header cells use.
pub(crate) fn press_growth(
    pressed: Option<f64>,
    released: Option<(f64, f32)>,
    now: f64,
) -> (f32, bool) {
    match (pressed, released) {
        (Some(t), _) => (grow(now - t), true),
        (None, Some((t, g))) if now - t < WOBBLE_S => {
            let s = now - t;
            let wobble = (TAU * s / WOBBLE_PERIOD).cos() * (1.0 - s / WOBBLE_S);
            (g * wobble as f32, true)
        }
        _ => (0.0, false),
    }
}

/// The lane strip's common drawing and its item list: `background` over the strip, a 2 px channel-colour line 1 px
/// below its top, then under the 4 px gap the list (`labels`, bar right, `list_w` wide, unselected items in `color`)
/// picking `item` (clamped to the items). Returns the strip below the gap, the list's and the value area's height.
#[allow(clippy::too_many_arguments)]
pub fn frame(
    ui: &mut Ui,
    cx: &UiCtx,
    id: Id,
    strip: Rect,
    list_w: f32,
    labels: &[&str],
    color: Color32,
    item: &mut usize,
) -> Rect {
    let px = 1.0 / cx.m.ppp();
    let painter = ui.painter_at(strip);
    painter.rect_filled(strip, 0.0, cx.theme.colors.background.to_color32());
    let line = Rect::from_x_y_ranges(strip.x_range(), strip.top() + px..=strip.top() + 3.0 * px);
    painter.rect_filled(line, 0.0, color);
    let content = content(strip, cx.m);
    let list = content.with_max_x(strip.left() + list_w);
    *item = (*item).min(labels.len().saturating_sub(1));
    item_list_index(
        ui,
        cx,
        &ItemList::new(id, list, labels, Bar::Right, color),
        item,
    );
    content
}

/// The lane strip `strip` below its 4 px gap: the rows of the list, the drum editor's spacer and the value area.
pub fn content(strip: Rect, m: &Metrics) -> Rect {
    strip.with_min_y((strip.top() + GAP_PX / m.ppp()).min(strip.bottom()))
}

/// The drum editor's spacer `spacer` (the expanded mixer's room in the header column) showing a sample's waveform in
/// `ink`, by the audio clip editor's waveform law (a 1 px column a device pixel from min/max peaks, mono centred at
/// scale h/2, stereo halves at h/4 and 3h/4 at scale h/4, a column under 1 px a 1 px line, a `reversed` sample
/// mirrored): the whole sample fitted to the spacer's width.
pub fn spacer_wave(
    painter: &Painter,
    spacer: Rect,
    wave: &Waveform,
    reversed: bool,
    ink: Color32,
    m: &Metrics,
) {
    let px = 1.0 / m.ppp();
    let (x0, width) = pixel_columns(spacer, m);
    let frames = wave.data.frames();
    // One "beat" a frame: `columns` then mirrors a reversed sample over exactly the frames it shows.
    let fit = Wave {
        frames,
        per_beat: 1.0,
        reversed,
    };
    let fpp = frames as f64 / width.max(1) as f64;
    let rows = wave_rows(spacer, wave.summary.len() > 1);
    let mut mesh = Mesh::default();
    for ((samples, summary), &row) in wave.channels().zip(&rows) {
        let cols = columns(samples, summary, fit, 0.0, fpp, width);
        for (i, peak) in cols.into_iter().enumerate() {
            add_column(&mut mesh, x0 + i as f32 * px, peak, row, px, ink);
        }
    }
    painter.add(mesh);
}

/// A drum value area's centre band: 1 U tall across `values` at its centre.
fn band(values: Rect, m: &Metrics) -> Rect {
    let cy = values.center().y;
    Rect::from_x_y_ranges(values.x_range(), cy - m.u(0.5)..=cy + m.u(0.5))
}

/// Draws the centre band of the drum value area `values` on the Panning and Pitch pages: a 1 px `background` rim
/// around `moduleFill`.
pub fn drum_band(painter: &Painter, values: Rect, prop: Prop, cx: &UiCtx) {
    if matches!(prop, Prop::Pan | Prop::Pitch) {
        let (m, colors) = (cx.m, &cx.theme.colors);
        let band = band(values, m);
        painter.rect_filled(band, 0.0, colors.background.to_color32());
        let px = 1.0 / m.ppp();
        painter.rect_filled(band.shrink(px), 0.0, colors.module_fill.to_color32());
    }
}

/// A drum hit's bar over `x` in the value area `values`, at `value` of `prop`, in `fill`.
/// Velocity: a solid bar from the bottom, h·vel/255. Panning / Pitch: with t = 2v − 1 the bar grows up from the
/// centre band's top (t > 0) or down from its bottom (t < 0) by |t|·(h − 1 U)/2, and a dot of radius 2·ceil(U/16) sits
/// at the band's centre at the bar's mid-x. Repeat: r + 1 stacked blocks, each h/8 tall with 1 px gaps, from the
/// bottom. Edges in y are on device pixels.
pub fn drum_bar(
    painter: &Painter,
    prop: Prop,
    value: u16,
    x: Rangef,
    values: Rect,
    fill: Color32,
    m: &Metrics,
) {
    let (bottom, h, px) = (values.bottom(), values.height(), 1.0 / m.ppp());
    let band = band(values, m);
    let f = prop.frac(value);
    let bar = |y0: f32, y1: f32| Rect::from_x_y_ranges(x, m.round_px(y0)..=m.round_px(y1));
    match prop {
        Prop::Velocity => {
            painter.rect_filled(bar(bottom - h * f, bottom), 0.0, fill);
        }
        Prop::Pan | Prop::Pitch => {
            let t = 2.0 * f - 1.0;
            let len = t.abs() * (h - m.u(1.0)) / 2.0;
            if t > 0.0 {
                painter.rect_filled(bar(band.top() - len, band.top()), 0.0, fill);
            } else if t < 0.0 {
                painter.rect_filled(bar(band.bottom(), band.bottom() + len), 0.0, fill);
            }
            painter.circle_filled(pos2(x.center(), band.center().y), 2.0 * m.t(), fill);
        }
        Prop::Repeat => {
            for k in 0..=value {
                let k = f32::from(k);
                let block = bar(bottom - (k + 1.0) * h / 8.0 + px, bottom - k * h / 8.0);
                painter.rect_filled(block, 0.0, fill);
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use eframe::egui::epaint::ClippedShape;
    use eframe::egui::{Event as In, PointerButton, Pos2, Shape, vec2};

    use super::*;
    use crate::metrics::DEFAULT_UI_SCALE_INDEX;
    use crate::song::PPQ;
    use crate::ui::offscreen::Offscreen;

    fn n(pos: u32, ticks: f64, key: u8) -> Event {
        Event::Note {
            pos,
            len_beats: ticks / f64::from(PPQ),
            key,
            velocity: Event::DEFAULT_VELOCITY,
            pan: Event::DEFAULT_PAN,
            slide: false,
        }
    }

    fn hit() -> Event {
        Event::Drum {
            pos: 0,
            len_beats: 0.25,
            pad: 0,
            velocity: Event::DEFAULT_VELOCITY,
            pan: Event::DEFAULT_PAN,
            value: Event::DEFAULT_PITCH,
            repeat: 0,
        }
    }

    fn metrics() -> Metrics {
        Metrics::from_pixels_per_point(1.0, DEFAULT_UI_SCALE_INDEX)
    }

    #[test]
    fn the_selectors_list_the_spec_items_and_repeat_only_in_step_sequencer_mode() {
        assert_eq!(ROLL_LABELS, ["Vel", "Pan"]);
        assert_eq!(ROLL_PROPS, [Prop::Velocity, Prop::Pan]);
        assert_eq!(DRUM_LABELS, ["Velocity", "Panning", "Pitch", "Repeat"]);
        assert_eq!(
            DRUM_PROPS,
            [Prop::Velocity, Prop::Pan, Prop::Pitch, Prop::Repeat]
        );
        assert_eq!(drum_items(true), 4);
        assert_eq!(
            &DRUM_LABELS[..drum_items(false)],
            ["Velocity", "Panning", "Pitch"]
        );
    }

    #[test]
    fn stored_values_follow_the_laws() {
        let stored = |p: Prop, v: f32| p.stored(v);
        // Vel = clamp(trunc(255 v), 1, 255).
        assert_eq!(stored(Prop::Velocity, 0.0), 1);
        assert_eq!(stored(Prop::Velocity, 0.5), 127);
        assert_eq!(stored(Prop::Velocity, 1.0), 255);
        // Pan = clamp(trunc(254 v), 0, 254): the middle is 127.
        assert_eq!(stored(Prop::Pan, 0.0), 0);
        assert_eq!(stored(Prop::Pan, 0.5), 127);
        assert_eq!(stored(Prop::Pan, 1.0), 254);
        // Pitch = trunc(65534 v): the middle is 0x7FFF.
        assert_eq!(stored(Prop::Pitch, 0.5), 0x7FFF);
        assert_eq!(stored(Prop::Pitch, 1.0), 65534);
        assert_eq!(stored(Prop::Pitch, 0.0), 0);
        // Repeat = round(7 v), 0..=7.
        assert_eq!(stored(Prop::Repeat, 0.0), 0);
        assert_eq!(stored(Prop::Repeat, 0.5), 4);
        assert_eq!(stored(Prop::Repeat, 0.49), 3);
        assert_eq!(stored(Prop::Repeat, 1.0), 7);
        // Outside [0, 1]: clamped.
        assert_eq!(stored(Prop::Velocity, 1.5), 255);
        assert_eq!(stored(Prop::Pan, -0.5), 0);
        // And back for drawing.
        assert_eq!(Prop::Velocity.frac(255), 1.0);
        assert_eq!(Prop::Pan.frac(127), 0.5);
        assert_eq!(Prop::Pan.frac(255), 1.0);
        assert_eq!(Prop::Pitch.frac(0x7FFF), 0.5);
        assert_eq!(Prop::Repeat.frac(7), 1.0);
    }

    #[test]
    fn the_pointer_gives_v_with_the_drum_centre_dead_zone() {
        let m = metrics();
        // 128 tall: v = (bottom − y)/h, y clamped to the area.
        let content = Rect::from_min_max(pos2(0.0, 100.0), pos2(50.0, 228.0));
        assert_eq!(v_at(content, 228.0), 0.0);
        assert_eq!(v_at(content, 164.0), 0.5);
        assert_eq!(v_at(content, 100.0), 1.0);
        assert_eq!(v_at(content, 50.0), 1.0);
        assert_eq!(v_at(content, 300.0), 0.0);
        // The 1 U zone around the centre (164 ± 16) is 0.5; the rest maps 48 points each way onto 0.5.
        for y in [148.0, 160.0, 164.0, 180.0] {
            assert_eq!(v_centred(content, y, &m), 0.5, "{y}");
        }
        assert_eq!(v_centred(content, 100.0, &m), 1.0);
        assert_eq!(v_centred(content, 124.0, &m), 0.75);
        assert_eq!(v_centred(content, 204.0, &m), 0.25);
        assert_eq!(v_centred(content, 228.0, &m), 0.0);
        assert_eq!(v_centred(content, 400.0, &m), 0.0);
    }

    #[test]
    fn values_come_from_and_go_to_the_event_fields() {
        let mut note = n(0, 32.0, 60);
        assert_eq!(Prop::Velocity.get(&note), Some(204));
        assert_eq!(Prop::Pan.get(&note), Some(127));
        assert_eq!(Prop::Pitch.get(&note), None);
        assert_eq!(Prop::Repeat.get(&note), None);
        Prop::Velocity.set(&mut note, 300);
        Prop::Pan.set(&mut note, 40);
        Prop::Repeat.set(&mut note, 3);
        assert_eq!(
            note,
            Event::Note {
                pos: 0,
                len_beats: 0.25,
                key: 60,
                velocity: 255,
                pan: 40,
                slide: false,
            }
        );
        let mut h = hit();
        Prop::Velocity.set(&mut h, 10);
        Prop::Pan.set(&mut h, 200);
        Prop::Pitch.set(&mut h, 0x9000);
        Prop::Repeat.set(&mut h, 5);
        assert_eq!(
            DRUM_PROPS.map(|p| p.get(&h)),
            [Some(10), Some(200), Some(0x9000), Some(5)]
        );
    }

    #[test]
    fn a_paint_sets_every_eligible_note_whose_extent_holds_the_tick() {
        // A chord at 0..64, a note at 64..96, one at 128..160.
        let notes = vec![
            n(0, 64.0, 60),
            n(0, 64.0, 64),
            n(64, 32.0, 60),
            n(128, 32.0, 60),
        ];
        let vel = |events: &[Event]| -> Vec<u16> {
            events
                .iter()
                .filter_map(|e| Prop::Velocity.get(e))
                .collect()
        };
        // Every note of the chord at that x.
        let mut events = notes.clone();
        assert!(paint_notes(
            &mut events,
            &[false; 4],
            Prop::Velocity,
            10.0,
            50
        ));
        assert_eq!(vel(&events), [50, 50, 204, 204]);
        // A note's end is not in it; its start is.
        let mut events = notes.clone();
        assert!(paint_notes(
            &mut events,
            &[false; 4],
            Prop::Velocity,
            64.0,
            9
        ));
        assert_eq!(vel(&events), [204, 204, 9, 204]);
        // Between notes nothing; the same value again: false.
        assert!(!paint_notes(
            &mut events,
            &[false; 4],
            Prop::Velocity,
            100.0,
            1
        ));
        assert!(!paint_notes(
            &mut events,
            &[false; 4],
            Prop::Velocity,
            70.0,
            9
        ));
        // With a selection only the selected notes are eligible.
        let sel = [false, true, false, true];
        let mut events = notes.clone();
        assert!(paint_notes(&mut events, &sel, Prop::Pan, 10.0, 0));
        assert!(!paint_notes(&mut events, &sel, Prop::Pan, 70.0, 0));
        let pan: Vec<u16> = events.iter().filter_map(|e| Prop::Pan.get(e)).collect();
        assert_eq!(pan, [127, 0, 127, 127]);
        // Notes the roll does not show (key bytes above 127) are left alone.
        let mut high = vec![n(0, 32.0, 200)];
        assert!(!paint_notes(&mut high, &[false], Prop::Velocity, 10.0, 1));
    }

    #[test]
    fn the_lane_is_hidden_first_then_max_f_h_or_2_u_tall() {
        let m = metrics();
        let area = Rect::from_min_max(pos2(0.0, 48.0), pos2(400.0, 348.0));
        let mut lane = GraphLane::default();
        assert_eq!(lane.split(area, &m).1, None);
        assert_eq!(lane.split(area, &m).0, area);
        // The first show: f = 0.5 of the 300 below the margin.
        lane.open = true;
        let (above, strip) = lane.split(area, &m);
        assert_eq!(strip, Some(area.with_min_y(198.0)));
        assert_eq!(above, area.with_max_y(198.0));
        // At least 2 U, at most H.
        lane.f = 0.1;
        assert_eq!(lane.split(area, &m).1.unwrap().height(), 64.0);
        let short = area.with_min_y(300.0);
        assert_eq!(lane.split(short, &m).1.unwrap(), short);
    }

    // The knob, offscreen.

    /// The editor below its margin, 300 tall from y 48.
    const AREA: Rect = Rect {
        min: Pos2 { x: 0.0, y: 48.0 },
        max: Pos2 { x: 400.0, y: 348.0 },
    };

    /// The knob of `lane` in [`AREA`] at x 24, for an offscreen frame.
    fn knob(lane: &mut GraphLane) -> impl FnMut(&mut Ui, &UiCtx) + '_ {
        move |ui, cx| lane.knob(ui, cx, Id::new("knob"), AREA, 24.0)
    }

    #[test]
    fn a_long_press_on_the_knob_toggles_nothing() {
        // Held 0.35 s in place (egui still calls its release a click): past the shared 300 ms, so no tap.
        let mut lane = GraphLane::default();
        let mut s = Offscreen::new();
        let at = pos2(24.0, 345.0);
        let button = |pressed| In::PointerButton {
            pos: at,
            button: PointerButton::Primary,
            pressed,
            modifiers: Default::default(),
        };
        s.frame(vec![In::PointerMoved(at)], &mut knob(&mut lane));
        s.frame(vec![button(true)], &mut knob(&mut lane));
        s.time += 0.35;
        s.frame(vec![button(false)], &mut knob(&mut lane));
        assert!(!lane.open);
        // A quick one toggles.
        s.click(at, &mut knob(&mut lane));
        assert!(lane.open);
    }

    #[test]
    fn the_knob_toggles_the_lane_on_a_tap_and_sizes_it_on_a_drag() {
        let m = metrics();
        let top = |lane: &GraphLane| lane.split(AREA, &m).1.map(|s| s.top());
        let mut lane = GraphLane::default();
        let mut s = Offscreen::new();
        s.frame(vec![], &mut knob(&mut lane));
        // Hidden, the knob sits on the bottom edge: its upper half takes the tap, which shows the lane at f 0.5.
        s.click(pos2(24.0, 345.0), &mut knob(&mut lane));
        assert!(lane.open);
        assert_eq!(top(&lane), Some(198.0));
        // A tap on the boundary hides it.
        s.click(pos2(24.0, 198.0), &mut knob(&mut lane));
        assert_eq!(top(&lane), None);
        // A drag up from the bottom edge to y 108 (f 0.8) shows it, capped at 0.7.
        let up = [vec2(0.0, -100.0), vec2(0.0, -237.0)];
        s.drag(pos2(24.0, 345.0), &up, &mut knob(&mut lane));
        assert_eq!((lane.open, lane.f), (true, 0.7));
        assert_eq!(top(&lane), Some(138.0));
        // Down to y 228: f 0.4.
        s.drag(
            pos2(24.0, 138.0),
            &[vec2(0.0, 30.0), vec2(0.0, 90.0)],
            &mut knob(&mut lane),
        );
        assert!((lane.f - 0.4).abs() < 1e-6, "{}", lane.f);
        assert_eq!(top(&lane), Some(228.0));
        // Released below 0.1 it hides, keeping f; the next tap shows it at 0.4 again.
        s.drag(
            pos2(24.0, 228.0),
            &[vec2(0.0, 30.0), vec2(0.0, 117.0)],
            &mut knob(&mut lane),
        );
        assert_eq!(top(&lane), None);
        s.click(pos2(24.0, 345.0), &mut knob(&mut lane));
        assert_eq!(top(&lane), Some(228.0));
    }

    #[test]
    fn a_press_and_release_in_one_frame_is_a_tap() {
        let mut lane = GraphLane::default();
        let mut s = Offscreen::new();
        let at = pos2(24.0, 345.0);
        s.frame(vec![In::PointerMoved(at)], &mut knob(&mut lane));
        let button = |pressed| In::PointerButton {
            pos: at,
            button: PointerButton::Primary,
            pressed,
            modifiers: Default::default(),
        };
        s.frame(vec![button(true), button(false)], &mut knob(&mut lane));
        assert!(lane.open);
    }

    #[test]
    fn the_knob_grows_while_pressed_and_wobbles_back_once_let_go() {
        let text = crate::theme::theme_by_id(0).colors.text.to_color32();
        let radius = |shapes: &[ClippedShape]| {
            shapes.iter().find_map(|c| match &c.shape {
                Shape::Circle(c) if c.fill == text => Some(c.radius),
                _ => None,
            })
        };
        let mut lane = GraphLane::default();
        let mut s = Offscreen::new();
        let at = pos2(24.0, 345.0);
        s.frame(vec![In::PointerMoved(at)], &mut knob(&mut lane));
        // A 1 U dot, growing by 10 px over the first 0.1 s of a press.
        let button = |pressed| In::PointerButton {
            pos: at,
            button: PointerButton::Primary,
            pressed,
            modifiers: Default::default(),
        };
        assert_eq!(
            radius(&s.frame(vec![button(true)], &mut knob(&mut lane))),
            Some(16.0)
        );
        for _ in 0..6 {
            s.frame(vec![], &mut knob(&mut lane));
        }
        assert_eq!(radius(&s.frame(vec![], &mut knob(&mut lane))), Some(21.0));
        // Let go, it swings below its size and settles within 0.3 s.
        s.frame(vec![button(false)], &mut knob(&mut lane));
        let mut sizes = Vec::new();
        for _ in 0..20 {
            sizes.extend(radius(&s.frame(vec![], &mut knob(&mut lane))));
        }
        assert!(sizes.iter().any(|&r| r < 16.0), "{sizes:?}");
        assert_eq!(sizes.last(), Some(&16.0));
    }

    #[test]
    fn each_page_places_a_value_on_the_tint_and_pulls_its_bottom_stop() {
        let t = |p: Prop, v: u16| p.tint_t(v);
        let near = |a: f32, b: f32| (a - b).abs() < 1e-4;
        // Velocity: vel/255, pulled 0.5.
        assert_eq!(Prop::Velocity.pull(), 0.5);
        assert_eq!(t(Prop::Velocity, 0), 0.0);
        assert!(near(t(Prop::Velocity, 102), 0.4));
        assert_eq!(t(Prop::Velocity, 255), 1.0);
        // Panning: min(1, 2·|pan/254 − 0.5|), pulled 0.75: the centre is 0, both ends 1, a file's 255 too.
        assert_eq!(Prop::Pan.pull(), 0.75);
        assert_eq!(t(Prop::Pan, 127), 0.0);
        assert_eq!(t(Prop::Pan, 0), 1.0);
        assert_eq!(t(Prop::Pan, 254), 1.0);
        assert_eq!(t(Prop::Pan, 255), 1.0);
        assert!(near(t(Prop::Pan, 190), 0.496_06));
        // Pitch: min(1, 2·|pitch/65535 − 0.5|), no pull.
        assert_eq!(Prop::Pitch.pull(), 0.0);
        assert!(t(Prop::Pitch, 0x7FFF) < 1e-4);
        assert_eq!(t(Prop::Pitch, 0), 1.0);
        assert_eq!(t(Prop::Pitch, 65535), 1.0);
        assert!(near(t(Prop::Pitch, 0xBFFF), 0.5));
        // Repeat: clamp(r/8, 0, 1), pulled 0.5.
        assert_eq!(Prop::Repeat.pull(), 0.5);
        assert_eq!(t(Prop::Repeat, 0), 0.0);
        assert_eq!(t(Prop::Repeat, 4), 0.5);
        assert_eq!(t(Prop::Repeat, 8), 1.0);
        assert_eq!(t(Prop::Repeat, 9), 1.0);
    }

    #[test]
    fn the_tint_runs_through_the_spec_stops_with_the_bottom_pulled() {
        let colors = &crate::theme::theme_by_id(0).colors;
        let c = Rgba::rgb(0.2, 0.8, 0.5);
        let [lo, mid, hi] = tint_stops(colors, c);
        let pulled = tint(colors, c, 0.5);
        assert_eq!(pulled, [mix(lo, mid, 0.5), mid, hi]);
        assert_eq!(tint_at(&pulled, 0.0), mix(lo, mid, 0.5).to_color32());
        assert_eq!(tint_at(&pulled, 1.0), hi.to_color32());
    }
}
