//! Knobs, sliders, the mixer fader, toggles, spinners, item lists, buttons, text labels and the display placeholder,
//! with the drag rules and long-press menu of the knobs, sliders (the fader among them) and toggles.
//!
//! Each widget draws inside its own rect from the layout, label strip included, and edits one normalised value in
//! [0, 1]. They know nothing about parameters: the panel passes the default, whether the control is bipolar, the
//! menu step count and the text law, and reads [`Response::changed`]. Spinners and item lists come in two forms:
//! bound to a menu parameter (the value is `menu_value(index, n)`, the index `menu_index(value, n)`) and
//! unbound, editing a plain item index for pages and UI state. Buttons only report clicks; the panel decides
//! what they toggle.
//!
//! Knob and slider input ([`Law`]): drags are relative, so a touch never jumps the value. Knobs (the graph knob and the
//! arc slider too) follow vertical drags only, +0.24 of the range per inch dragged up at the DPI the app is given
//! ([`Metrics::dpi`]); sliders (the fader and the tag slider too) move by d / L along their track, so that the thumb
//! follows the pointer. Ctrl (cmd on macOS, the `fineTweak` modifier) makes both four times finer. A touch-down less
//! than 0.3 s after the previous one on the same control sets its reset value, [`Control::default`]; a tap never steps
//! the value. The primary button only, and no wheel: it goes to the enclosing scroller. Toggles flip on click. A NaN
//! value is drawn as the default and an out-of-range one as the nearest end; neither is written back until the user
//! changes it.
//!
//! Desktop hover: the one control under the mouse that takes input ([`hovered`]) is lit: a button's fill × 1.15, a
//! slider's track halo, a knob's wash with its dot × 1.05. The cursor never changes.
//!
//! Each widget tells accessibility tools its role, name and value (`Response::widget_info`). Each enabled one is a
//! keyboard focus target ([`crate::ui::focus`]): a focused knob or slider takes `increment` / `decrement`, ± 0.05 of
//! the range and 0.0125 fine, or one step of a control with n steps, fine or not ([`stepped`]); `open` (Enter) presses
//! a focused button, toggle, spinner or list item.
//!
//! A long press (mouse or touch, 300 ms) or secondary click opens the parameter menu: `Reset` (knobs and sliders
//! only), then `Link to MIDI CC`, `Link to Mod Wheel`, `Unlink from MIDI CC` (dimmed unless the parameter has a CC
//! link, [`ParamMenu::Linked`]), `Add automation track` and, only on a knob or slider whose parameter the fine tune
//! dialog supports ([`Control::fine_tune`]), `Fine tune control`.
//!
//! A control bound to a parameter also reports its press and its parameter menu's pick of a CTRL item as a
//! [`Signal`], which its owner reads with [`signal`] right after drawing it: the app's recent control, which the app
//! then acts on as on the CTRL menu's pick (the items are the same [`CtrlAction`]s).
//!
//! Texts are plain text: placed in their rect by their alignment and never clipped, shrunk or cut short, so a label
//! wider than its control overhangs it evenly. Only the ring knob's and the arc slider's values (fit-only sizing) and
//! the graph knob's label (fitted to its strip) change size; the truncated and shrink routines ([`truncated`],
//! [`shrink`]) are for the card header, the dialogs, the drum strips and the settings tab's labels.
//! The one clipped text is the XY pad's axis names, drawn with its grid inside the pad (they are placed inside it
//! anyway). The panel clips everything to its module card.

use eframe::egui::{
    Align2, Color32, Id, Painter, PointerButton, Pos2, Rect, Response, Sense, Ui, Vec2, WidgetInfo,
};

use crate::metrics::{FontSize, Metrics};
use crate::params::{menu_index, menu_value};
use crate::ui::ctrl_menu::{self, CtrlAction};
use crate::ui::focus::{self, Kind};
use crate::ui::popup::{self, PopupItem};
use crate::ui::{UiCtx, gesture, primary_down_live};

mod button;
pub(crate) use button::{brighter, pressed};
mod fader;
mod knob;
mod list;
mod menu_button;
mod plain;
mod slider;
mod spinner;
mod text_toggle;
mod toggle;

pub use button::{Button, button};
pub use fader::{UNITY, fader};
pub use knob::{knob, strip_knob};
pub use list::{Bar, ItemList, item_list, item_list_index};
pub use menu_button::menu_button;
pub use plain::{display_area, separator, text_label};
pub use slider::{h_slider, v_slider};
pub use spinner::{spinner, spinner_button, spinner_grid, spinner_index};
pub use text_toggle::{disc_toggle, text_button, text_toggle};
pub use toggle::{Pill, pill_toggle, toggle};

/// Height of the label strip at the top and of the value strip at the bottom, in U.
pub(crate) const STRIP_H: f32 = 0.75;
/// The share of its range a knob moves per inch dragged up.
const KNOB_PER_INCH: f32 = 0.24;
/// Drag factor k while the fine-tweak modifier is held.
const FINE: f32 = 0.25;
/// A touch-down this soon after the previous one on the same control resets it, in seconds.
const DOUBLE_TAP_S: f64 = 0.3;
/// An `increment` / `decrement` key press's step; [`FINE`] times it for the fine ones.
const KEY_STEP: f32 = 0.05;

/// What a knob, slider, toggle or spinner needs besides its value and value text. Build it with [`Control::new`] and
/// change the other fields with struct update syntax.
#[derive(Clone, Copy, Debug)]
pub struct Control<'a> {
    pub id: Id,
    /// The whole control, label and value strips included.
    pub rect: Rect,
    /// The main text line. Empty: no label strip, the body takes its place.
    pub label: &'a str,
    /// FontSize 1 (8 pt) unless the module picks another; the value strip uses it too.
    pub label_font: FontSize,
    /// The module's channel colour: labels, values, ring, fill and dot.
    pub channel: Color32,
    /// The reset value: what a double tap and `Reset` set.
    pub default: f32,
    /// Rests at the middle: the ring and thumb grow with |v − 0.5| and slider fills start at the track's middle.
    /// Its default is exactly 0.5, which is what [`Control::new`] sets.
    pub bipolar: bool,
    /// A menu-kind parameter with this many items: drags snap to its steps.
    pub steps: Option<usize>,
    /// The item a value reads where that is not [`menu_index`] (a `ParamKind::MenuBy` law): drags snap to it, so a
    /// tie lands on the item the text shows.
    pub item: Option<fn(f32) -> usize>,
    /// Sliders: the value readout strip at the bottom. Toggles: the `On` / `Off` strip. Knobs always show the value
    /// inside the ring; leave it off for them.
    pub value_text: bool,
    /// No input, and sliders draw their fill and thumb in `menuFill`.
    pub disabled: bool,
    /// Sliders: track, thumb rim, value bar and thumb centre all in this colour, input unchanged (the Arpeggiator's
    /// `Rotation` with Rhythm Off: `moduleFill`).
    pub flat: Option<Color32>,
    /// Spinners and menu buttons: the popup's items per column, filled top to bottom and then left to right; `None` is
    /// one column.
    pub column_len: Option<usize>,
    /// Spinners and menu buttons: items shown but not selectable, in the popup (dimmed) and by the spinner's
    /// chevrons (skipped).
    pub disabled_items: &'a [usize],
    /// What a long press or secondary click opens.
    pub menu: ParamMenu,
    /// Knobs and sliders bound to a parameter: the fine tune dialog supports it ([`crate::ui::fine_tune::supported`]),
    /// so the parameter menu ends with `Fine tune control`.
    pub fine_tune: bool,
}

/// The parameter menu of a knob, slider, toggle or button: knobs and sliders start it with `Reset`, and a control
/// bound to a parameter adds the CTRL items. A menu with no items does not open.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ParamMenu {
    /// Bound to a parameter: `Reset` (knobs and sliders), then the CTRL items, `Unlink from MIDI CC` dimmed.
    Bound,
    /// Bound to a parameter that has a MIDI CC link (`CCLK`): `Unlink from MIDI CC` enabled.
    Linked,
    /// Bound to none: `Reset` alone on knobs and sliders, nothing on toggles and buttons.
    Unbound,
    /// No menu at all, such as the drum editor's swing slider in its own popup.
    Off,
}

impl ParamMenu {
    /// The menu of a control bound to a parameter, `linked` when that has a MIDI CC link.
    pub fn bound(linked: bool) -> Self {
        if linked { Self::Linked } else { Self::Bound }
    }
}

impl<'a> Control<'a> {
    pub fn new(id: Id, rect: Rect, label: &'a str, channel: Color32, default: f32) -> Self {
        Self {
            id,
            rect,
            label,
            label_font: FontSize::Pt8,
            channel,
            default,
            bipolar: default == 0.5,
            steps: None,
            item: None,
            value_text: false,
            disabled: false,
            flat: None,
            column_len: None,
            disabled_items: &[],
            menu: ParamMenu::Bound,
            fine_tune: false,
        }
    }

    /// The label strip (when there is a label), the body, and the value strip (when `value_text`), top to bottom.
    fn strips(&self, m: &Metrics) -> (Option<Rect>, Rect, Option<Rect>) {
        let h = m.u(STRIP_H);
        let mut body = self.rect;
        // Each split stays inside what is left, so a rect shorter than its strips leaves an empty body, never an
        // inverted one.
        let label = (!self.label.is_empty()).then(|| {
            let (strip, rest) = body.split_top_bottom_at_y((body.top() + h).min(body.bottom()));
            body = rest;
            strip
        });
        let value = self.value_text.then(|| {
            let (rest, strip) = body.split_top_bottom_at_y((body.bottom() - h).max(body.top()));
            body = rest;
            strip
        });
        (label, body, value)
    }

    /// Paints the label strip and, when present, the value strip with `value`'s text, centred on them.
    fn strip_texts(
        &self,
        painter: &Painter,
        m: &Metrics,
        value: f32,
        display: impl Fn(f32) -> String,
    ) {
        let (label, _, value_strip) = self.strips(m);
        let font = m.font_id(self.label_font);
        if let Some(rect) = label {
            painter.text(
                rect.center(),
                Align2::CENTER_CENTER,
                self.label,
                font.clone(),
                self.channel,
            );
        }
        if let Some(rect) = value_strip {
            painter.text(
                rect.center(),
                Align2::CENTER_CENTER,
                display(value),
                font,
                self.channel,
            );
        }
    }
}

/// The value a widget shows and starts editing from: NaN reads as the default, anything else is clamped to [0, 1].
/// The bound value itself is only written when the user changes it.
fn clean(v: f32, default: f32) -> f32 {
    if v.is_nan() {
        default
    } else {
        v.clamp(0.0, 1.0)
    }
}

/// The primary button is down on `response`, on a live press ([`primary_down_live`]: not one held over a focus
/// loss). Touch feedback (wash, halo, swell) follows only that button, as drags and taps do;
/// `is_pointer_button_down_on` alone is true for any button.
fn primary_down_on(ui: &Ui, response: &Response) -> bool {
    // Every frame, whatever the press: the latch must see the focus loss.
    let live = primary_down_live(ui, response.id);
    response.is_pointer_button_down_on() && live
}

/// How far `v` is from rest, in [0, 1]: `v` itself, or |v − 0.5|·2 when bipolar. Ring strokes and slider thumbs
/// grow with it.
fn growth(v: f32, bipolar: bool) -> f32 {
    if bipolar { (v - 0.5).abs() * 2.0 } else { v }
}

/// How pointer travel moves a knob or slider.
#[derive(Clone, Copy, Debug)]
enum Law {
    /// Vertical only: [`KNOB_PER_INCH`] of the range per inch dragged up, times the control's sensitivity s
    /// (1 but for the drum strip's `Start`, 0.5).
    Knob(f32),
    /// Along a track `len` points long, left to right or bottom to top: the thumb follows the pointer 1:1.
    Track { len: f32, horizontal: bool },
}

impl Law {
    /// The value change for `d` points of pointer travel, before the fine factor.
    fn delta(self, d: Vec2, m: &Metrics) -> f32 {
        match self {
            Law::Knob(s) => -d.y * m.ppp() * KNOB_PER_INCH * s / m.dpi(),
            Law::Track { len, horizontal } if len > 0.0 => {
                (if horizontal { d.x } else { -d.y }) / len
            }
            Law::Track { .. } => 0.0,
        }
    }
}

/// The drag value after a change of `delta`, [`FINE`] times that when `fine`, kept in [0, 1].
fn drag(acc: f32, delta: f32, fine: bool) -> f32 {
    let k = if fine { FINE } else { 1.0 };
    (acc + k * delta).clamp(0.0, 1.0)
}

/// A primary touch-down on `response` less than [`DOUBLE_TAP_S`] after the previous one on control `id`. Every
/// touch-down restarts the window.
pub(crate) fn double_tap(ui: &Ui, id: Id, response: &Response) -> bool {
    let (pressed, now) = ui.input(|i| (i.pointer.primary_pressed(), i.time));
    if !(pressed && response.is_pointer_button_down_on()) {
        return false;
    }
    let key = id.with("touch-down");
    let last = ui.data(|d| d.get_temp::<f64>(key));
    ui.data_mut(|d| d.insert_temp(key, now));
    last.is_some_and(|t| now - t < DOUBLE_TAP_S)
}

/// `response`'s control is the one hovered control: the pointer is over it (egui also hovers the nearest widget within
/// its interact radius) and it takes input. egui hovers a click-only widget together with a drag widget under it; the
/// drag one yields, so that exactly one is lit.
pub(crate) fn hovered(ui: &Ui, response: &Response) -> bool {
    let (ctx, sense) = (ui.ctx(), response.sense);
    let under_a_click_widget = || {
        let hovered = ctx.interaction_snapshot(|s| s.hovered.clone());
        (hovered.into_iter()).any(|id| {
            id != response.id
                && ctx
                    .read_response(id)
                    .is_some_and(|r| r.sense.senses_click())
        })
    };
    (response.hover_pos()).is_some_and(|p| response.interact_rect.contains(p))
        && sense.senses_click()
        && !(sense.senses_drag() && under_a_click_widget())
}

/// `v` on the step of a menu with `steps` items that `item` reads ([`item_at`]), or `v` itself.
fn snap(v: f32, steps: Option<usize>, item: Option<fn(f32) -> usize>) -> f32 {
    steps.map_or(v, |n| menu_value(item_at(v, n, item), n))
}

/// The item of `n` that `v` reads: `item`'s law (a `ParamKind::MenuBy`), else the nearest ([`menu_index`]).
fn item_at(v: f32, n: usize, item: Option<fn(f32) -> usize>) -> usize {
    item.map_or_else(|| menu_index(v, n), |f| f(v).min(n.saturating_sub(1)))
}

/// Runs an index-based widget on a menu parameter's normalised `value`. The widget edits the item index
/// `menu_index(value, n)` (which also reads NaN and out-of-range values as the nearest item). The value is only
/// written back, as `menu_value(index, n)`, when the widget reports a change, so an untouched value stays as it
/// was.
fn through_index(
    value: &mut f32,
    n: usize,
    widget: impl FnOnce(&mut usize) -> Response,
) -> Response {
    let mut index = menu_index(*value, n);
    let response = widget(&mut index);
    if response.changed() {
        *value = menu_value(index, n);
    }
    response
}

/// Interaction of a knob or slider: primary-button drag by `law`, double-tap reset and the parameter menu. Returns the response, marked changed
/// when the user moved `value`, and the value to draw ([`clean`]: an untouched NaN or out-of-range value is drawn
/// as the default or the nearest end and left as it is).
fn drag_control(
    ui: &mut Ui,
    cx: &UiCtx,
    c: &Control,
    value: &mut f32,
    law: Law,
) -> (Response, f32) {
    let sense = if c.disabled {
        Sense::hover()
    } else {
        Sense::click_and_drag()
    };
    let mut response = ui.interact(c.rect, c.id, sense);
    let mut v = clean(*value, c.default);
    let mut edited = false;
    if !c.disabled {
        focus::register(ui, c.id, c.rect, Kind::Value);
        if let Some(step) = focus::take_step(ui.ctx(), c.id) {
            v = stepped(v, step, c.steps, c.item);
            edited = true;
        }
    }
    // A press whose long press opened the menu drags nothing under it.
    let menu_open = ui.data(|d| d.get_temp::<Anchor>(menu_id(c.id)).is_some());
    if response.dragged_by(PointerButton::Primary) && !menu_open {
        // The unsnapped, clamped drag value lives in memory for the length of the drag, so that small moves add
        // up between steps and a pointer that went past an end turns the value back as soon as it returns.
        let acc_id = c.id.with("drag");
        let (acc, delta) = if response.drag_started() {
            // egui only starts a drag after the pointer moved a few points: count those too.
            let travel = ui.input(|i| Some(i.pointer.interact_pos()? - i.pointer.press_origin()?));
            (v, travel.unwrap_or_default())
        } else {
            let acc = ui.data(|d| d.get_temp(acc_id)).unwrap_or(v);
            (acc, response.drag_delta())
        };
        // The keymap's `fineTweak` (Ctrl/Cmd held).
        let fine = crate::keymap::held(ui.ctx(), "fineTweak");
        let acc = drag(acc, law.delta(delta, cx.m), fine);
        ui.data_mut(|d| d.insert_temp(acc_id, acc));
        v = snap(acc, c.steps, c.item);
        edited = true;
    }
    // The menu first: it must be drawn every frame it is open, whatever else happened.
    let action = context_menu(ui, cx, c.id, c.menu, true, c.fine_tune, &mut response);
    if action == Some(MenuAction::Reset) || double_tap(ui, c.id, &response) {
        v = c.default;
        edited = true;
    }
    // `!=` and not a comparison with the clean value: a NaN the user acted on is replaced.
    if edited && v != *value {
        *value = v;
        response.mark_changed();
    }
    response.widget_info(|| WidgetInfo::slider(!c.disabled, f64::from(v), c.label));
    (response, v)
}

/// `v` after one `increment` / `decrement` key press: ± [`KEY_STEP`], × [`FINE`] when fine, kept in [0, 1]; a control
/// with `steps` items moves to the next or previous item ([`item_at`]), fine or not, and stays at the ends.
fn stepped(v: f32, step: focus::Step, steps: Option<usize>, item: Option<fn(f32) -> usize>) -> f32 {
    match steps {
        Some(n) => {
            let i = item_at(v, n, item);
            let i = if step.up { i + 1 } else { i.saturating_sub(1) };
            menu_value(i.min(n.saturating_sub(1)), n)
        }
        None => {
            let k = if step.fine { FINE } else { 1.0 };
            let sign = if step.up { 1.0 } else { -1.0 };
            (v + sign * k * KEY_STEP).clamp(0.0, 1.0)
        }
    }
}

/// The id of control `id`'s parameter menu (the spinner's item popup has its own, [`spinner::popup_id`]).
fn menu_id(id: Id) -> Id {
    id.with("parameter menu")
}

/// What a control bound to a parameter reported this frame.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct Signal {
    /// A pointer button went down on it.
    pub pressed: bool,
    /// The CTRL item picked from its parameter menu (`Link to MIDI CC` … `Add automation track`).
    pub pick: Option<CtrlAction>,
}

/// Control `id`'s [`Signal`] of this frame, taken: call it right after drawing the control. Unbound and disabled
/// controls signal nothing.
pub fn signal(ui: &Ui, id: Id) -> Signal {
    ui.data_mut(|d| d.remove_temp(id.with("signal")))
        .unwrap_or_default()
}

/// Shows control `id`'s parameter menu while it is open; a long press ([`gesture::long_press`], mouse or touch) or a
/// secondary click on `response` opens it, centred on the pointer, and the release after a long press is no click.
/// `reset`: the control is a knob or slider, whose menu starts with `Reset`; `fine_tune`: it ends with `Fine tune
/// control`. Returns the item picked this frame.
fn context_menu(
    ui: &Ui,
    cx: &UiCtx,
    id: Id,
    menu: ParamMenu,
    reset: bool,
    fine_tune: bool,
    response: &mut Response,
) -> Option<MenuAction> {
    let links = matches!(menu, ParamMenu::Bound | ParamMenu::Linked);
    let reset = reset && menu != ParamMenu::Off;
    if !reset && !links {
        return None;
    }
    if links && response.is_pointer_button_down_on() && ui.input(|i| i.pointer.any_pressed()) {
        post(ui, id, |s| s.pressed = true);
    }
    let menu_id = menu_id(id);
    let long = gesture::long_press(ui, response, cx.m);
    if response.clicked_by(PointerButton::Secondary) || long {
        let at = response
            .interact_pointer_pos()
            .unwrap_or(response.rect.center());
        open_popup(ui, menu_id, at);
    }
    let at = popup_anchor(ui, menu_id)?;
    // Built only while the menu is open, not every frame of every control.
    let items = menu_items(reset, menu, fine_tune);
    let ctx = ui.ctx();
    let result = popup::show(ctx, menu_id, at, ctx.content_rect(), &items, cx);
    if result.close {
        close_popup(ui, menu_id);
    }
    if let Some(MenuAction::Pick(action)) = result.selected {
        post(ui, id, |s| s.pick = Some(action));
    }
    result.selected
}

/// Adds to control `id`'s [`Signal`] for [`signal`].
fn post(ui: &Ui, id: Id, add: impl FnOnce(&mut Signal)) {
    ui.data_mut(|d| add(d.get_temp_mut_or_default(id.with("signal"))));
}

/// An open popup of a control (the parameter menu, a spinner's items): where it is centred, and the last frame it
/// was shown on.
#[derive(Clone, Copy, Debug)]
struct Anchor {
    at: Pos2,
    frame: u64,
}

/// Opens popup `id` centred on `at`.
fn open_popup(ui: &Ui, id: Id, at: Pos2) {
    let frame = ui.ctx().cumulative_frame_nr();
    ui.data_mut(|d| d.insert_temp(id, Anchor { at, frame }));
}

/// Where popup `id` is centred while it is open. Its control calls this every frame it is drawn, which keeps the
/// popup open; a popup whose control was not drawn last frame is closed instead (a page changed), so that it never
/// comes back with its control by itself. Undo and redo wait while it is open ([`crate::history::shortcut`]).
fn popup_anchor(ui: &Ui, id: Id) -> Option<Pos2> {
    let frame = ui.ctx().cumulative_frame_nr();
    ui.data_mut(|d| {
        let anchor = d.get_temp::<Anchor>(id)?;
        if anchor.frame + 1 < frame {
            d.remove::<Anchor>(id);
            return None;
        }
        d.insert_temp(id, Anchor { frame, ..anchor });
        Some(anchor.at)
    })
}

fn close_popup(ui: &Ui, id: Id) {
    ui.data_mut(|d| d.remove::<Anchor>(id));
}

/// The items of the parameter menu: `Reset`, the control's own, and the CTRL items, which the app acts on.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum MenuAction {
    Reset,
    Pick(CtrlAction),
}

/// The parameter menu (with `Reset` first when `reset`), one column; the CTRL items only when `menu` is bound, with
/// `Unlink from MIDI CC` dimmed unless it is [`ParamMenu::Linked`], and `Fine tune control` (row 5) present only when
/// `fine_tune`.
fn menu_items(reset: bool, menu: ParamMenu, fine_tune: bool) -> Vec<PopupItem<MenuAction>> {
    let bound = matches!(menu, ParamMenu::Bound | ParamMenu::Linked);
    let reset = reset.then(|| PopupItem::new(MenuAction::Reset, 0, 0, "Reset"));
    // Rows 1…4 are the CTRL menu's first four, row 5 its last.
    let fine_tune = ctrl_menu::ITEMS.last().filter(|_| fine_tune);
    let links = (ctrl_menu::ITEMS[..4].iter().chain(fine_tune))
        .filter(|_| bound)
        .map(|&(action, label)| PopupItem {
            disabled: action == CtrlAction::UnlinkMidiCc && menu != ParamMenu::Linked,
            ..PopupItem::new(MenuAction::Pick(action), 0, 0, label)
        });
    reset
        .into_iter()
        .chain(links)
        .enumerate()
        .map(|(row, item)| PopupItem { row, ..item })
        .collect()
}

#[cfg(test)]
mod tests;

// The effect-specific controls (Gate, MultiFX, Waveshaper, Tape Stop, Tremolo). The graph knob lives with the ring
// knob.

mod arc;
mod curve;
mod tag;
mod xy_pad;

pub use arc::{ArcSlider, arc_slider};
pub(crate) use arc::{pulse, pulse_growth};
pub use curve::{curve_plot, env};
pub use knob::graph_knob;
pub use tag::tag_slider;
pub(crate) use xy_pad::line;
pub use xy_pad::xy_pad;

// The text routines.

mod text;

pub use text::{shrink, truncated, truncated_text};

/// Whether a toggle or a two-state button bound to `v` is on: for any value but 0, so 0.5 and 0.001 too.
/// NaN (a corrupt file) is off, as 0 is. The inverted pill swaps the sides, and a tap writes 1 from off and 0 from on.
/// A text that reads the value keeps its own threshold: `On` from 0.5, the SoundFont `Envelope`'s `Custom` above it.
pub fn toggle_on(v: f32) -> bool {
    v != 0.0 && !v.is_nan()
}

// The fine tune dialog's slider.

mod fine_tune;

pub use fine_tune::fine_tune_slider;

// The clip editors' (and the playlist ruler's) Snap button.

mod snap_button;

pub use snap_button::{SnapTouch, paint_snap_button, snap_button, snap_touch};
