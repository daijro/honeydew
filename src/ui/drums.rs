//! The Drums channel: the hexagon pad grid and the pad editor strips, one per drum row, over a channel's [`DrumKit`].
//!
//! Both views are self-contained: the caller owns the kit and a [`DrumsState`], picks the rect, and acts on the
//! returned [`DrumsAction`]s (play a pad, send a changed slot param to the audio side). The geometry is pure
//! ([`pad_grid`], [`strip_rect`], [`place_strip`]) and in device pixels, like the layout engine.
//!
//! Strips draw with the widget kits: the no-label pill toggle and the sliders, and their own strip looks: knobs
//! captioned in a left column ([`widgets::strip_knob`]), `Solo`, `REV` and `Poly` as discs ([`widgets::disc_toggle`])
//! and the icon 21 button as a hidden disc under a `text` icon. Their header cells are left to the host: the step
//! view's row headers fill them, two lines that follow the drum mixer strip's own rule ([`header_names`]). The names
//! shown are [`row_lines`] on a row header and [`hex_lines`] on a pad. Text is plain, drawn in full and never clipped,
//! except those header lines and the pad names, which shrink to fit.

use std::collections::HashMap;

use eframe::egui::{
    Align2, Color32, Id, Painter, Pos2, Rect, Sense, Shape, Stroke, Ui, pos2, vec2,
};
use honeydew_audio::sample_edit::speed_semitones;

use crate::icons::IconId;
use crate::metrics::{FontSize, Metrics};
use crate::midi::{CcLink, links};
use crate::params::{DRUM_SLOT_PARAMS, ParamKind};
use crate::song::{AutomationTarget, ChannelKind, DrumKit, SamplerSlot, Song};
use crate::ui::ctrl_menu::Touch;
use crate::ui::fine_tune;
use crate::ui::layout::{self, Node, Size};
use crate::ui::popup::{self, PopupItem};
use crate::ui::widgets::{self, Button, Control, ParamMenu, Pill};
use crate::ui::{Finger, UiCtx, fingers, focus_lost};

#[cfg(test)]
mod tests;

/// Pads in the grid: rows of 7, 8 and 7.
pub const PAD_COUNT: usize = 22;
/// The first pad of each row, and one past the last pad.
const ROW_STARTS: [usize; 4] = [0, 7, 15, PAD_COUNT];

/// The pad grid's size and centres, in device pixels.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct PadGrid {
    /// The pad size r: half the horizontal pitch.
    pub r: f32,
    pub centres: [Pos2; PAD_COUNT],
}

/// The pad layout for the pad area `area`, a screen `screen_w` wide, at `u` per U (all device pixels): r =
/// floor(H / (2 · 3 · 0.93)), at most floor(W / 16) and at least floor(min(screen width, 20 U) / 16) (the lower bound
/// wins when they cross), on a centred honeycomb with pitch 2 r × 1.72 r whose even rows are half a pitch right.
pub fn pad_grid(area: Rect, screen_w: f32, u: f32) -> PadGrid {
    let (w, h) = (area.width(), area.height());
    let r = (h / (2.0 * 3.0 * 0.93))
        .floor()
        .min((w / 16.0).floor())
        .max((screen_w.min(20.0 * u) / 16.0).floor());
    let top = area
        .top()
        .max((area.top() + (h - (2.0 * r * 3.0 * 0.86).floor()) / 2.0).floor());
    let left = area.left() + (w - 16.0 * r) / 2.0;
    let centres = std::array::from_fn(|i| {
        let row = ROW_STARTS.iter().rposition(|&s| s <= i).unwrap_or(0);
        let col = (i - ROW_STARTS[row]) as f32;
        let shift = if row % 2 == 0 { r } else { 0.0 };
        pos2(
            left + r + 2.0 * r * col + shift,
            top + r + 1.72 * r * row as f32,
        )
    });
    PadGrid { r, centres }
}

/// The kit pad (slot index, [`DrumKit::slot_on_pad`]) hex pad `pad` plays: its `PADS` entry (read as pad → slot), which
/// rearranging the pads swaps. The kit's slot order, its drum events and the step rows stay as they are. An entry past
/// the kit's slots is an unused pad.
pub fn pad_slot(kit: &DrumKit, pad: usize) -> usize {
    usize::from(kit.file.pads[pad])
}

/// A ring fainter than this is not drawn, and asks for no more frames.
const RING_MIN: f32 = 1.0 / 255.0;

impl PadGrid {
    /// The pad hit at `p` (device pixels): within r of its centre (the circle inscribed in the 2 r pitch), the nearer
    /// centre where two circles overlap (rows are 1.99 r apart diagonally).
    pub fn pad_at(&self, p: Pos2) -> Option<usize> {
        (0..PAD_COUNT)
            .map(|i| (i, (p - self.centres[i]).length_sq()))
            .filter(|&(_, d)| d <= self.r * self.r)
            .min_by(|a, b| a.1.total_cmp(&b.1))
            .map(|(i, _)| i)
    }
}

/// A drum row header's two lines: the custom pad name (`MAIN`@1051, "" = none) alone, else the sample file's parent
/// folder ([`SamplerSlot::pad_name`]) over its name without extension (`MAIN`@12). Every row header's names come from
/// here; the first is [`SamplerSlot::display_name`].
pub fn row_lines(slot: &SamplerSlot) -> (&str, &str) {
    let sample = match slot.file.custom_pad_name.as_str() {
        "" => slot.name.as_str(),
        _ => "",
    };
    (slot.display_name(), sample)
}

/// A hex pad's two lines: always the parent folder over the file name, never the custom pad name. Every pad's names
/// come from here.
pub fn hex_lines(slot: &SamplerSlot) -> (&str, &str) {
    (slot.pad_name(), &slot.name)
}

/// The pad outline's vertices, in R from the centre: a pointy-top hexagon walked edge by edge.
const HEX: [(f32, f32); 6] = [
    (0.86, -0.5),
    (0.86, 0.5),
    (-0.006, 1.0),
    (-0.872, 0.5),
    (-0.872, -0.5),
    (-0.006, -1.0),
];

/// A pad of size `size_px` (device pixels: r, or a layer's radius) around `center` (points), as it shows: the outline
/// at R = trunc(1.5 · size) scaled by 1/√2 about the centre, drawn directly as that visible hexagon:
/// circumradius ≈ 1.06 r, ≈ 0.16 r between neighbours on the 2 r × 1.72 r pitch, egui's own ~1 px anti-aliased edge.
fn hex(center: Pos2, size_px: f32, ppp: f32, color: Color32) -> Shape {
    let s = (1.5 * size_px).trunc() * std::f32::consts::FRAC_1_SQRT_2 / ppp;
    let points = HEX.iter().map(|&(x, y)| center + s * vec2(x, y)).collect();
    Shape::convex_polygon(points, color, Stroke::NONE)
}

/// The page every strip shows; the drum editor switches it for all of them at once.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum StripPage {
    #[default]
    Mix,
    Pitch,
    Envelope,
    Options,
}

impl StripPage {
    pub const ALL: [Self; 4] = [Self::Mix, Self::Pitch, Self::Envelope, Self::Options];
}

/// A strip control, named after its caption or its slot param.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum Ctl {
    Mute,
    Solo,
    Pan,
    Level,
    /// The icon button (icon 21).
    Sample,
    Rev,
    Pitch,
    Stretch,
    Start,
    Atk,
    Dec,
    Sus,
    Rel,
    Hold,
    Cut,
    Poly,
    Shift,
    Aux,
}

#[derive(Clone, Copy, Debug)]
enum Kind {
    /// Pill toggle without a label.
    Pill,
    /// A disc with a word ([`widgets::disc_toggle`]): k, and the word's size. Bound to its slot param, or (`REV`,
    /// `Poly`) to a `MAIN` flag.
    Disc(f32, FontSize),
    HSlider,
    /// The icon 21 button: a `moduleFill` disc, k = 0.9, under a `text` icon.
    Icon,
    /// Strip knob ([`widgets::strip_knob`]) with its value text law, `None` for no text.
    Knob(Option<fn(f32) -> String>),
    /// A text button that opens a menu.
    Menu,
}

/// One control of a page: what it is, its caption, its slot param, and its flex weight and right margin (U) in
/// the page row.
struct Spec {
    ctl: Ctl,
    kind: Kind,
    label: &'static str,
    param: Option<usize>,
    flex: f32,
    mr: f32,
}

const fn spec(
    ctl: Ctl,
    kind: Kind,
    label: &'static str,
    param: Option<usize>,
    flex: f32,
    mr: f32,
) -> Spec {
    Spec {
        ctl,
        kind,
        label,
        param,
        flex,
        mr,
    }
}

/// The knobs' right margin, in U.
const KNOB_MR: f32 = 0.25;
/// What a double click or `Reset` sets the strip's Level slider to.
const LEVEL_RESET: f32 = 0.90909;

/// `Solo` and `REV`: k 0.9, 8 pt; `Poly`: k 0.8, 10 pt.
const SOLO_REV: Kind = Kind::Disc(0.9, FontSize::Pt8);
const POLY: Kind = Kind::Disc(0.8, FontSize::Pt10);

const MIX: [Spec; 4] = [
    spec(Ctl::Mute, Kind::Pill, "", Some(3), 1.0, 0.0),
    spec(Ctl::Solo, SOLO_REV, "Solo", Some(4), 1.0, 0.0),
    spec(Ctl::Pan, Kind::HSlider, "", Some(1), 2.0, 0.0),
    spec(Ctl::Level, Kind::HSlider, "", Some(0), 4.0, 0.0),
];
const PITCH: [Spec; 5] = [
    spec(Ctl::Sample, Kind::Icon, "", None, 1.0, 0.0),
    spec(Ctl::Rev, SOLO_REV, "REV", None, 1.0, 0.0),
    spec(
        Ctl::Pitch,
        Kind::Knob(Some(pitch_text)),
        "Pitch",
        Some(2),
        2.0,
        KNOB_MR,
    ),
    spec(
        Ctl::Stretch,
        Kind::Knob(Some(stretch_text)),
        "Stretch",
        None,
        2.0,
        KNOB_MR,
    ),
    spec(
        Ctl::Start,
        Kind::Knob(Some(start_text)),
        "Start",
        Some(10),
        2.0,
        KNOB_MR,
    ),
];
const ENVELOPE: [Spec; 4] = [
    spec(Ctl::Atk, Kind::Knob(None), "ATK", Some(6), 1.0, KNOB_MR),
    spec(Ctl::Dec, Kind::Knob(None), "DEC", Some(7), 1.0, KNOB_MR),
    spec(Ctl::Sus, Kind::Knob(None), "SUS", Some(8), 1.0, KNOB_MR),
    spec(Ctl::Rel, Kind::Knob(None), "REL", Some(9), 1.0, KNOB_MR),
];
const OPTIONS: [Spec; 5] = [
    spec(Ctl::Hold, Kind::HSlider, "Hold", Some(5), 2.0, 0.0),
    spec(Ctl::Cut, Kind::Menu, "Cut...", None, 1.0, 0.0),
    spec(Ctl::Poly, POLY, "Poly", None, 1.0, 0.0),
    spec(
        Ctl::Shift,
        Kind::Knob(Some(shift_text)),
        "Shift",
        None,
        2.0,
        0.0,
    ),
    spec(Ctl::Aux, Kind::Menu, "AUX", None, 1.2, 0.0),
];

fn page_specs(page: StripPage) -> &'static [Spec] {
    match page {
        StripPage::Mix => &MIX,
        StripPage::Pitch => &PITCH,
        StripPage::Envelope => &ENVELOPE,
        StripPage::Options => &OPTIONS,
    }
}

/// `Pitch`'s text: the slot's Speed step trunc(48v − 24) by the audio's fused law, a plain integer.
pub fn pitch_text(v: f32) -> String {
    speed_semitones(v).to_string()
}

/// `Start`'s text: the slot's own law.
fn start_text(v: f32) -> String {
    DRUM_SLOT_PARAMS[10].display_text(v)
}

/// `Stretch` at `v`: the `STR1` ratio 2^(2v − 1), 0.5 … 2.
pub fn stretch_ratio(v: f32) -> f64 {
    (2.0 * f64::from(v) - 1.0).exp2()
}

/// Where `Stretch` sits for a stored ratio: log2(ratio)/2 + 0.5, held to the knob's ends; at rest when not a number.
pub fn stretch_value(ratio: f64) -> f32 {
    let v = (ratio.log2() / 2.0 + 0.5) as f32;
    if v.is_nan() { 0.5 } else { v.clamp(0.0, 1.0) }
}

/// `Stretch`'s text: the ratio, `%.2f`.
pub fn stretch_text(v: f32) -> String {
    format!("{:.2}", stretch_ratio(v))
}

/// `Shift` at `v`: its step k = trunc(64v − 32), an offset of k/128 beat (±32 ticks). The centre step is twice as wide.
pub fn shift_step(v: f32) -> i32 {
    (64.0 * v - 32.0).trunc() as i32
}

/// `Shift`'s text: trunc(offset × 400), percent of a sixteenth, −100 … 100.
pub fn shift_text(v: f32) -> String {
    ((f64::from(shift_step(v)) / 128.0 * 400.0).trunc() as i32).to_string()
}

/// The `Cut...` menu, by `SamplerSlot::cut_group`.
pub const CUT_ITEMS: [&str; 4] = ["No cut group", "Cut group A", "Cut group B", "Cut group C"];

/// Strip `row` of `rows` stacked in `area` (points): pitch = area height / rows, each strip one device pixel
/// shorter.
pub fn strip_rect(area: Rect, rows: usize, row: usize, ppp: f32) -> Rect {
    let pitch = area.height() / rows.max(1) as f32;
    Rect::from_min_size(
        pos2(area.left(), area.top() + pitch * row as f32),
        vec2(area.width(), pitch - 1.0 / ppp),
    )
}

/// The leaf id of the strip's header cell; controls are numbered by their place on the page.
const HEADER: u32 = u32::MAX;

/// Lays a strip out (device pixels, `u` per U): a 3 U header cell, then a layer holding the four pages, of which
/// only `page` shows, as a row 0.75 × the strip height. Returns the header's rect and each control's.
pub fn place_strip(page: StripPage, strip: Rect, u: f32) -> (Rect, Vec<(Ctl, Rect)>) {
    let pages = StripPage::ALL.map(|p| {
        let leaves = page_specs(p)
            .iter()
            .enumerate()
            .map(|(i, s)| Node::leaf(i as u32).w(Size::Flex(s.flex)).mr(s.mr));
        Node::hstack(leaves)
            .h(Size::Percent(75.0))
            .visible(p == page)
    });
    let root = Node::hstack([Node::leaf(HEADER).w(Size::U(3.0)), Node::layer(pages)]);
    let rects = layout::layout(&root, strip, u);
    let header = layout::rect_of(&rects, HEADER).unwrap_or(Rect::NOTHING);
    let specs = page_specs(page);
    let controls = rects
        .into_iter()
        .filter(|&(id, _)| id != HEADER)
        .map(|(id, r)| (specs[id as usize].ctl, r))
        .collect();
    (header, controls)
}

/// What the drums views keep between frames. The caller keeps one per Drums channel.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct DrumsState {
    /// The page every strip shows.
    pub page: StripPage,
    /// The fingers on the pads. A finger holds its pad (drawn at r + 3) until it lifts or picks the pad up. When the
    /// pad view stops being drawn while pads are held (the view is closed or switched mid-press), no `PadUp` comes:
    /// the caller sends `PadUp` for the pads held itself and clears them.
    pub fingers: Vec<PadFinger>,
    /// Each pad's drop-target ring strength h, in [0, 1].
    pub rings: [f32; PAD_COUNT],
    /// A held `Stretch` or `Shift` knob's value, by slot and control, until the finger lifts and the slot takes it
    /// ([`held_knob`]).
    pub settings: HashMap<(usize, Ctl), f32>,
    /// A strip control bound to a slot param was pressed, or picked an item of its parameter menu (instance −slot
    /// id): the app's recent control. The host takes it.
    pub touch: Option<Touch>,
    /// Per slot, its output's peak L = max(left, right) this frame, from the engine's slot meters; the host sets it.
    /// Empty, or a slot past its end, is silent.
    pub peaks: Vec<f32>,
    /// The song's send channels (id, name), for the `AUX` menu: the host sets it each frame ([`send_channels`]).
    pub sends: Vec<(i32, String)>,
    /// The open `Cut...` or `AUX` popup: its slot, its control and its centre. The host closes it with the view.
    pub menu: Option<(usize, Ctl, Pos2)>,
}

/// A peak `l` (linear) on the app's meter scale, −60 … +6 dB over 0 … 1: (20·log10 l + 60)/66, 0 below l = 0.001 (the
/// pad glow's base, 0.30303·log10 L + 0.90909).
pub fn meter_level(l: f32) -> f32 {
    if l < 0.001 {
        return 0.0;
    }
    ((20.0 * l.log10() + 60.0) / 66.0).clamp(0.0, 1.0)
}

/// A finger on the pads: the pad it went down on, where it is and where it went down (points), and, once it has
/// picked its pad up, how far the pad has shrunk (f, from 0 toward 1).
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct PadFinger {
    pub id: u64,
    pub pad: usize,
    pub at: Pos2,
    origin: Pos2,
    pub lifted: Option<f32>,
}

/// What happened in a drums view this frame.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum DrumsAction {
    /// The pointer went down on the pad: play it.
    PadDown(usize),
    /// The press on the pad ended.
    PadUp(usize),
    /// A picked-up pad was dropped on another: swap the two pads' `PADS` entries ([`pad_slot`]), one undo step.
    Swap(usize, usize),
    /// Slot param `param` of slot `slot` changed in the kit: send it on.
    Param { slot: usize, param: usize },
    /// The slot's cut group changed.
    CutGroup(usize),
    /// The slot's send target (`LINk`@10) changed in the kit: a final song edit (the routing changes).
    SendTarget(usize),
    /// The slot's `AUX` picked `Create FX Channel`: the host makes a send channel the slot targets
    /// ([`create_pad_send`]).
    CreateFxChannel(usize),
    /// The slot's icon 21 button: make its row current and open the pad sample picker on it (its `Load Sample`).
    LoadSample(usize),
    /// `REV` flipped the slot's reverse (`MAIN`@1036) in the kit: send it on, nothing to re-render.
    Reverse(usize),
    /// `Poly` flipped the slot's overlap (`MAIN`@1050) in the kit: send it on.
    Poly(usize),
    /// `Shift` changed the slot's beat offset step (`MAIN`@1037, @1045) in the kit: its hits move at the next build.
    Shift(usize),
    /// `Stretch` changed the slot's `STR1` ratio and mode in the kit: re-import its sample.
    Stretch(usize),
}

/// The pad grid in `rect` (points) for `kit`, in the `channel` colour. No background: the keyboard panel's
/// `controlFill` shows through. Pads with a slot ([`pad_slot`]) are channel-filled with the pad name (12 pt) over the
/// sample name (10 pt), others `moduleFill`. Every finger ([`fingers`]) plays the pad it goes down on, until it lifts,
/// wherever it goes (no glissando); a finger on a pad another finger holds ends that one's note first, as does a finger
/// down again without its lift. Unless `locked` (`lockpads`), a held pad moved more than trunc(1.5 U) on x or y is
/// picked up and its note ends: drawn under the finger in the channel colour without its names, it shrinks toward 85 %
/// (f ← f + 0.2·(1 − f), size trunc((1 − 0.15·f)·r)), over a ring on the pad it would land on (size r + ceil(U/16),
/// h ← 0.8·h + 0.2 there and h ← 0.8·h elsewhere, `controlFill` h of the way to the channel colour), while its home
/// shows `moduleFill`. Dropped on another pad it swaps the two ([`DrumsAction::Swap`]); dropped anywhere else nothing
/// happens. A focus loss lets go of every finger (the lift may never come). Held pads are released only by a frame of
/// this view: see [`DrumsState::fingers`].
pub fn show_pads(
    ui: &mut Ui,
    cx: &UiCtx,
    rect: Rect,
    kit: &DrumKit,
    state: &mut DrumsState,
    channel: Color32,
    locked: bool,
) -> Vec<DrumsAction> {
    let (m, colors) = (cx.m, &cx.theme.colors);
    let ppp = m.ppp();
    let screen_w = ui.ctx().content_rect().width() * ppp;
    let grid = pad_grid(rect * ppp, screen_w, m.u_px());
    let response = ui.interact(rect, ui.id().with("drum pads"), Sense::click_and_drag());
    let pad_at = |p: Pos2| grid.pad_at(p * ppp);

    let mut actions = Vec::new();
    if ui.input(focus_lost) {
        let held = state.fingers.drain(..).filter(|f| f.lifted.is_none());
        actions.extend(held.map(|f| DrumsAction::PadUp(f.pad)));
    }
    let reach = (1.5 * m.u_px()).trunc();
    for finger in fingers(ui, response.id) {
        match finger {
            Finger::Down(id, at) => {
                let pad = pad_at(at);
                // The finger's own hold (its lift was lost) and another finger's hold on this pad end first.
                let ended = (state.fingers).extract_if(.., |f| {
                    f.id == id || (Some(f.pad) == pad && f.lifted.is_none())
                });
                let held = ended.filter(|f| f.lifted.is_none());
                actions.extend(held.map(|f| DrumsAction::PadUp(f.pad)));
                let Some(pad) = pad else {
                    continue;
                };
                state.fingers.push(PadFinger {
                    id,
                    pad,
                    at,
                    origin: at,
                    lifted: None,
                });
                actions.push(DrumsAction::PadDown(pad));
            }
            Finger::Move(id, at) => {
                let Some(f) = state.fingers.iter_mut().find(|f| f.id == id) else {
                    continue;
                };
                f.at = at;
                let d = (at - f.origin) * ppp;
                if f.lifted.is_none() && !locked && (d.x.abs() > reach || d.y.abs() > reach) {
                    f.lifted = Some(0.0);
                    actions.push(DrumsAction::PadUp(f.pad));
                }
            }
            Finger::Up(id, at) => {
                let Some(i) = state.fingers.iter().position(|f| f.id == id) else {
                    continue;
                };
                let f = state.fingers.remove(i);
                match (f.lifted, pad_at(at)) {
                    (None, _) => actions.push(DrumsAction::PadUp(f.pad)),
                    (Some(_), Some(to)) if to != f.pad => {
                        actions.push(DrumsAction::Swap(f.pad, to))
                    }
                    _ => {}
                }
            }
        }
    }
    // Once a frame: picked-up pads shrink, and the pads they are over ring.
    let mut targets = Vec::new();
    for f in &mut state.fingers {
        if let Some(shrink) = &mut f.lifted {
            *shrink += 0.2 * (1.0 - *shrink);
            targets.extend(pad_at(f.at).filter(|&to| to != f.pad));
        }
    }
    for (i, h) in state.rings.iter_mut().enumerate() {
        *h = 0.8 * *h + if targets.contains(&i) { 0.2 } else { 0.0 };
    }
    let lifted = |pad| {
        state
            .fingers
            .iter()
            .any(|f| f.pad == pad && f.lifted.is_some())
    };
    if state.fingers.iter().any(|f| f.lifted.is_some()) || state.rings.iter().any(|&h| h > RING_MIN)
    {
        ui.ctx().request_repaint();
    }

    // Neither the pads nor the names are clipped to the area (text is never clipped, and the pads go with it): a grid
    // at its lower size bound can be larger than the area. Draw order: level glows, drop highlights, pad bodies and
    // their names, the dragged pads.
    let painter = ui.painter();
    let (module_fill, control_fill) = (
        colors.module_fill.to_color32(),
        colors.control_fill.to_color32(),
    );
    let r = grid.r;
    let slot = |pad| kit.slot_on_pad(pad_slot(kit, pad));
    let t = (m.u_px() / 16.0).ceil();
    // The level glows: size r + 2·ceil(U/16), `controlFill` g of the way to the channel colour, g =
    // (0.30303·log10 L + 0.90909)^1.6667, L the peak of the slot the pad plays (0 below 0.001); `controlFill` at rest.
    for (i, &c) in grid.centres.iter().enumerate() {
        let peak = state.peaks.get(pad_slot(kit, i)).copied().unwrap_or(0.0);
        let g = meter_level(peak).powf(1.6667);
        painter.add(hex(
            c / ppp,
            r + 2.0 * t,
            ppp,
            control_fill.lerp_to_gamma(channel, g),
        ));
    }
    // The drop highlights: size r + ceil(U/16), `controlFill` h of the way to the channel colour.
    for (i, &c) in grid.centres.iter().enumerate() {
        let h = state.rings[i];
        if h > RING_MIN {
            let ring = control_fill.lerp_to_gamma(channel, h);
            painter.add(hex(c / ppp, r + t, ppp, ring));
        }
    }
    for (i, &c) in grid.centres.iter().enumerate() {
        let held = (state.fingers.iter()).any(|f| f.pad == i && f.lifted.is_none());
        let size = if held { r + 3.0 } else { r };
        let used = slot(i).is_some() && !lifted(i);
        painter.add(hex(
            c / ppp,
            size,
            ppp,
            if used { channel } else { module_fill },
        ));
    }
    // Names in a box 1.5 r wide centred on the pad, 2 r tall, shrunk to fit (truncated at 6 pt): the folder at 12 pt
    // centred 0.25 U above the centre, the file at 10 pt 0.25 U below it. On top of every pad (a held pad grows into
    // its neighbours' gaps).
    for (i, &c) in grid.centres.iter().enumerate() {
        let Some(slot) = slot(i).filter(|_| !lifted(i)) else {
            continue;
        };
        let (pad, sample) = hex_lines(slot);
        for (text, pt, dy) in [(pad, 12, -0.25), (sample, 10, 0.25)] {
            let line =
                Rect::from_center_size(c / ppp + vec2(0.0, m.u(dy)), vec2(1.5 * r, 2.0 * r) / ppp);
            widgets::shrink(
                painter,
                m,
                line,
                Align2::CENTER_CENTER,
                text,
                pt,
                module_fill,
            );
        }
    }
    // Picked-up pads over everything, under their fingers.
    for f in &state.fingers {
        if let Some(shrink) = f.lifted {
            let size = ((1.0 - 0.15 * shrink) * r).trunc();
            painter.add(hex(f.at, size, ppp, channel));
        }
    }
    actions
}

/// The two lines of a drum row's header (as on the drum mixer strip and the step sequencer's row headers), left-aligned
/// 2 hairlines into `header` and at most its width less 7 hairlines. The pad name is shrunk from 12 pt (truncated at
/// 6 pt, keeping 5 tail characters) and centred on the header; the sample name is truncated at 6 pt, keeping 5, 0.5 U
/// lower, and not drawn when "" ([`row_lines`] with a custom name).
pub fn header_names(
    painter: &Painter,
    m: &Metrics,
    header: Rect,
    pad: &str,
    sample: &str,
    color: Color32,
) {
    let t = m.t();
    let left = header.left() + 2.0 * t;
    let line = Rect::from_x_y_ranges(
        left..=left + (header.width() - 7.0 * t).max(0.0),
        header.y_range(),
    );
    widgets::shrink(painter, m, line, Align2::LEFT_CENTER, pad, 12, color);
    if sample.is_empty() {
        return;
    }
    let below = line.translate(vec2(0.0, m.u(0.5)));
    let font = m.font_id(FontSize::Pt6);
    widgets::truncated(painter, below, Align2::LEFT_CENTER, sample, font, color, 5);
}

/// The pad editor strips in `rect` (points), one per slot of `kit`, all on `state.page`. Each keeps its header cell's
/// room empty, for the host's row header. Controls bound to a slot param edit it in the kit and report it, and their
/// press or parameter menu pick goes to [`DrumsState::touch`]; `links` are the channel's MIDI CC links (its `CCLK`),
/// which enable a linked param's `Unlink from MIDI CC`.
pub fn show_strips(
    ui: &mut Ui,
    cx: &UiCtx,
    rect: Rect,
    kit: &mut DrumKit,
    state: &mut DrumsState,
    channel: Color32,
    links: &[CcLink],
) -> Vec<DrumsAction> {
    let m = cx.m;
    let ppp = m.ppp();
    let base = ui.id().with("drum strips");
    let rows = kit.slots.len();
    let mut actions = Vec::new();
    for (row, slot) in kit.slots.iter_mut().enumerate() {
        let strip = strip_rect(rect, rows, row, ppp);
        let (_, controls) = place_strip(state.page, strip * ppp, m.u_px());
        let instance_id = -(slot.file.id as i32);
        for (spec, (_, r)) in page_specs(state.page).iter().zip(controls) {
            let id = base.with((row, spec.ctl));
            let rect = r / ppp;
            let menu = match spec.param {
                Some(p) => ParamMenu::bound(links::has(links, instance_id, p as i32)),
                None => ParamMenu::Unbound,
            };
            if let Some(action) = control(ui, cx, spec, id, rect, menu, row, slot, state, channel) {
                actions.push(action);
            }
            if let Some(param) = spec.param {
                let target = AutomationTarget {
                    instance_id,
                    param: param as i32,
                };
                if let Some(touch) = Touch::new(target, widgets::signal(ui, id)) {
                    state.touch = Some(touch);
                }
            }
        }
    }
    actions
}

/// Draws one strip control of slot `row`, with its parameter menu `menu`, and returns what it did.
#[allow(clippy::too_many_arguments)]
fn control(
    ui: &mut Ui,
    cx: &UiCtx,
    spec: &Spec,
    id: Id,
    rect: Rect,
    menu: ParamMenu,
    row: usize,
    slot: &mut SamplerSlot,
    state: &mut DrumsState,
    channel: Color32,
) -> Option<DrumsAction> {
    let colors = &cx.theme.colors;
    let pspec = spec.param.map(|p| DRUM_SLOT_PARAMS[p]);
    // Unbound knobs rest at the middle: both are ± adjustments.
    let default = pspec.map_or(0.5, |p| p.default);
    // The strip's own resets: Level 0.90909, not the 0.8531 a new slot stores; `REL` 0, not its default 1.
    let reset = match spec.ctl {
        Ctl::Level => LEVEL_RESET,
        Ctl::Rel => 0.0,
        _ => default,
    };
    let c = Control {
        menu,
        fine_tune: spec.param.is_some_and(|p| fine_tune::pad_param(p as i32)),
        ..Control::new(id, rect, spec.label, channel, reset)
    };
    let changed = match (spec.kind, spec.param) {
        (Kind::Pill, Some(p)) => {
            let pill = Pill {
                label_right: true,
                ..Pill::default()
            };
            let value = &mut slot.params[p];
            widgets::pill_toggle(ui, cx, &c, pill, value, |_| String::new()).changed()
        }
        (Kind::Disc(k, font), param) => {
            let c = Control {
                label_font: font,
                ..c
            };
            let Some(p) = param else {
                return flag(ui, cx, &c, k, row, slot, spec.ctl);
            };
            widgets::disc_toggle(ui, cx, &c, k, &mut slot.params[p]).changed()
        }
        (Kind::HSlider, Some(p)) => {
            let value = &mut slot.params[p];
            widgets::h_slider(ui, cx, &c, value, |_| String::new()).changed()
        }
        (Kind::Knob(text), Some(p)) => {
            let steps = match pspec.map(|p| p.kind) {
                Some(ParamKind::Continuous { display }) => display.step_count(),
                _ => None,
            };
            let c = Control { steps, ..c };
            // `Start` drags at half the knob law.
            let s = if spec.ctl == Ctl::Start { 0.5 } else { 1.0 };
            let text = |v| text.map_or_else(String::new, |text| text(v));
            widgets::strip_knob(ui, cx, &c, &mut slot.params[p], s, text).changed()
        }
        (Kind::Knob(_), None) => return held_knob(ui, cx, &c, spec.ctl, row, slot, state),
        (Kind::Icon, _) => {
            let b = Button {
                icon: Some(IconId::AudioClip),
                shape: 0.9,
                fill_off: colors.module_fill.to_color32(),
                ..Button::new(id, rect, "", true, colors)
            };
            // No on state: every tap acts.
            let clicked = widgets::button(ui, cx, &b, false).clicked();
            return clicked.then_some(DrumsAction::LoadSample(row));
        }
        // Not the menu button (used once, by 3x Osc): a text button with a fixed caption that opens a list.
        (Kind::Menu, _) => {
            // Text buttons are 10 pt.
            let text = Control {
                label_font: FontSize::Pt10,
                ..c
            };
            let clicked = widgets::text_button(ui, cx, &text).clicked();
            if clicked {
                state.menu = Some((row, spec.ctl, rect.center()));
            }
            let at = (state.menu)
                .filter(|&(r, ctl, _)| (r, ctl) == (row, spec.ctl))
                .map(|m| m.2)?;
            let (picked, close) = if spec.ctl == Ctl::Aux {
                aux_pick(ui, cx, id.with("menu"), at, slot, &state.sends, row)
            } else {
                let (picked, close) = cut_menu(ui, cx, id.with("menu"), at, slot);
                (picked.then_some(DrumsAction::CutGroup(row)), close)
            };
            if close {
                state.menu = None;
            }
            return picked;
        }
        (Kind::Pill | Kind::HSlider, None) => false,
    };
    changed
        .then_some(spec.param)
        .flatten()
        .map(|param| DrumsAction::Param { slot: row, param })
}

/// `REV` or `Poly` (`ctl`): a disc over the slot's `MAIN` reverse or overlap flag. A flip is reported for the
/// host to send on.
fn flag(
    ui: &mut Ui,
    cx: &UiCtx,
    c: &Control,
    k: f32,
    row: usize,
    slot: &mut SamplerSlot,
    ctl: Ctl,
) -> Option<DrumsAction> {
    let (flag, action) = match ctl {
        Ctl::Rev => (&mut slot.file.reverse, DrumsAction::Reverse(row)),
        _ => (&mut slot.file.overlapping_voices, DrumsAction::Poly(row)),
    };
    let mut v = f32::from(u8::from(*flag));
    if !widgets::disc_toggle(ui, cx, c, k, &mut v).changed() {
        return None;
    }
    *flag = v > 0.0;
    Some(action)
}

/// `Stretch` or `Shift` (`ctl`, on slot `row`): a knob over a slot field. While a finger holds it, only the knob and
/// its text follow: the value waits in [`DrumsState::settings`]. When it lifts, or at once for a change with no finger
/// on it (a key, `Reset`), the slot takes it, but only when its step changes:
/// - `Stretch`: the `STR1` ratio 2^(2v − 1) ([`stretch_ratio`]), mode 1 exactly when the ratio is not 1: a re-import.
/// - `Shift`: the beat offset k/128 (`MAIN`@1037, k = [`shift_step`]) with v as its control value (@1045).
fn held_knob(
    ui: &mut Ui,
    cx: &UiCtx,
    c: &Control,
    ctl: Ctl,
    row: usize,
    slot: &mut SamplerSlot,
    state: &mut DrumsState,
) -> Option<DrumsAction> {
    let stretch = ctl == Ctl::Stretch;
    let (key, text) = ((row, ctl), if stretch { stretch_text } else { shift_text });
    let file = &mut slot.file;
    let held = state.settings.remove(&key);
    let mut v = held.unwrap_or(if stretch {
        stretch_value(file.stretch_ratio)
    } else {
        file.beat_offset_control
    });
    let response = widgets::strip_knob(ui, cx, c, &mut v, 1.0, text);
    if held.is_none() && !response.changed() {
        return None;
    }
    if response.is_pointer_button_down_on() {
        state.settings.insert(key, v);
        return None;
    }
    if stretch {
        let ratio = stretch_ratio(v);
        if ratio == file.stretch_ratio {
            return None;
        }
        (file.stretch_ratio, file.stretch_mode) = (ratio, u8::from(ratio != 1.0));
        return Some(DrumsAction::Stretch(row));
    }
    let k = f64::from(shift_step(v));
    if k == file.beat_offset * 128.0 {
        return None;
    }
    (file.beat_offset, file.beat_offset_control) = (k / 128.0, v);
    Some(DrumsAction::Shift(row))
}

/// The song's send channels as the `AUX` menu lists them: (channel id, name), in song order.
pub fn send_channels(song: &Song) -> Vec<(i32, String)> {
    (song.channels.iter())
        .filter(|c| c.kind == ChannelKind::EffectTrack)
        .map(|c| (c.id, c.name.clone()))
        .collect()
}

/// `Create FX Channel` from slot `slot` of channel `channel`'s kit ([`DrumsAction::CreateFxChannel`]): a new send
/// channel at the end of `song`, which the slot targets. Returns its index; `None` for no such slot, or once the
/// channel ids are used up.
pub fn create_pad_send(song: &mut Song, channel: usize, slot: usize) -> Option<usize> {
    let kit = song.channels.get(channel)?.drum_kit.as_ref()?;
    kit.slots.get(slot)?;
    let new = song.insert_channel(song.channels.len(), ChannelKind::EffectTrack)?;
    let id = song.channels[new].id;
    let kit = song.channels[channel].drum_kit.as_mut()?;
    kit.slots[slot].file.send_target = id;
    Some(new)
}

/// What the `AUX` menu picks: a target id (−1 `Default`), or `Create FX Channel`.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum AuxPick {
    Target(i32),
    Create,
}

/// The pad Send menu for a pad targeting `target`: `Default` (−1, through the drum channel), each send channel of
/// `sends` by name (its id), then `Create FX Channel`; the current target highlighted. Master and regular
/// channels are not offered.
fn aux_menu(target: i32, sends: &[(i32, String)]) -> Vec<PopupItem<AuxPick>> {
    let targets = std::iter::once((-1, "Default".to_owned())).chain(sends.iter().cloned());
    let mut items: Vec<_> = targets
        .enumerate()
        .map(|(row, (id, name))| PopupItem {
            current: id == target,
            ..PopupItem::new(AuxPick::Target(id), 0, row, name)
        })
        .collect();
    items.push(PopupItem::new(
        AuxPick::Create,
        0,
        items.len(),
        "Create FX Channel",
    ));
    items
}

/// The `AUX` popup of slot `row`, centred on `at`. A picked target is written to the slot; `Create FX Channel` is left
/// to the host. Returns that, and whether the popup closes.
fn aux_pick(
    ui: &Ui,
    cx: &UiCtx,
    id: Id,
    at: Pos2,
    slot: &mut SamplerSlot,
    sends: &[(i32, String)],
    row: usize,
) -> (Option<DrumsAction>, bool) {
    let items = aux_menu(slot.file.send_target, sends);
    let ctx = ui.ctx();
    let result = popup::show(ctx, id, at, ctx.content_rect(), &items, cx);
    let picked = match result.selected {
        Some(AuxPick::Create) => Some(DrumsAction::CreateFxChannel(row)),
        Some(AuxPick::Target(id)) if id != slot.file.send_target => {
            slot.file.send_target = id;
            Some(DrumsAction::SendTarget(row))
        }
        _ => None,
    };
    (picked, result.close)
}

/// The `Cut...` popup centred on `at`. No item is current: the menu is never synced from the slot (the row header's
/// badge shows the group). Returns whether a new group was picked, and whether the popup closes.
fn cut_menu(ui: &Ui, cx: &UiCtx, id: Id, at: Pos2, slot: &mut SamplerSlot) -> (bool, bool) {
    let current = usize::from(slot.cut_group);
    let items: Vec<_> = (CUT_ITEMS.iter().enumerate())
        .map(|(i, &label)| PopupItem::new(i, 0, i, label))
        .collect();
    let ctx = ui.ctx();
    let result = popup::show(ctx, id, at, ctx.content_rect(), &items, cx);
    let picked = match result.selected {
        Some(i) if i != current => {
            slot.cut_group = i as u8;
            true
        }
        _ => false,
    };
    (picked, result.close)
}
