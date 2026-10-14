//! Screens and small widgets, all custom-painted.

use eframe::egui;

use crate::metrics::Metrics;
use crate::theme::Theme;

pub mod add_channel_menu;
pub mod buttons;
pub mod clips;
pub mod drum_grid;
pub mod drums;
pub mod graph_lane;
pub mod keyboard;
pub mod layout;
pub mod main_menu;
pub mod main_screen;
pub mod mixer;
pub mod modules_browser;
pub mod piano_roll;
pub mod popup;
pub mod preset_picker;
pub mod preview;
pub mod rack;
pub mod region;
pub mod save_dialog;
pub mod settings_tab;
pub mod song_options;
pub mod soundfont_picker;
pub mod step_seq;
pub mod timeline;
pub mod track_menu;
pub mod views;
pub mod widgets;
// Transport extras.
pub mod audio_editor;
pub mod automation;
pub mod ctrl_menu;
pub mod keyboard_settings;
#[cfg(test)]
mod offscreen;
pub mod tempo;
pub mod transport_extras;
// The keyboard-area sample row.
pub mod sample_row;
// The shared long press and the menu and panel animations.
pub mod anim;
pub mod gesture;
// The keyboard panel's hand resize and scroll zone.
pub mod panel_resize;
// The pitch and mod wheels.
pub mod wheels;
// The chord keyboard of the Chords and Strum kinds.
pub mod chord_keyboard;
// The chord editor.
pub mod chord_editor;
// The playlist's header column and its mini mixer.
pub mod mini_mixer;
// The side browser's audition and sample strips.
pub mod picker_preview;
// The fine tune dialog.
pub mod fine_tune;
// The MIDI import dialog.
pub mod midi_import;
// The dialog base: the dim and the `moduleFill` box.
pub mod dialog;
// The performance monitor's data and drawing: the PROJECT block and the floating overlay.
pub mod performance;
// The keymap's editing and zoom actions in the editor grids.
pub mod edit_keys;

pub mod split;

/// What every painter needs: the theme and this frame's metrics. The app builds it once per frame and
/// passes it down by reference.
#[derive(Clone, Copy)]
pub struct UiCtx<'a> {
    pub theme: &'a Theme,
    pub m: &'a Metrics,
}

/// The window lost the focus this frame; on the web also when the page is hidden.
pub fn focus_lost(input: &egui::InputState) -> bool {
    (input.events.iter()).any(|e| matches!(e, egui::Event::WindowFocused(false)))
}

/// The primary button is down on a live press. egui keeps the button down when the focus goes (switching tabs
/// mid-hold) and the release may never come, so a press held over a focus loss is stale, and holds nothing, until
/// the button is seen up or pressed again. Call it every frame the widget `id` shows: its latch is in egui's memory.
pub fn primary_down_live(ui: &egui::Ui, id: egui::Id) -> bool {
    let (lost, pressed, down) = ui.input(|i| {
        (
            focus_lost(i),
            i.pointer.primary_pressed(),
            i.pointer.primary_down(),
        )
    });
    let stale = ui.data_mut(|d| {
        let stale = d.get_temp_mut_or_default::<bool>(id.with("stale press"));
        *stale = (*stale && !pressed || lost) && down;
        *stale
    });
    down && !stale
}

/// The press on the widget `id` is stale ([`primary_down_live`]) from now on, as after a focus loss: the widget goes
/// with it held, and if it shows again under that press, it holds nothing until the release or a new press.
pub fn let_go_press(ctx: &egui::Context, id: egui::Id) {
    ctx.data_mut(|d| d.insert_temp(id.with("stale press"), true));
}

/// A second finger has touched since the last press: the press is a pinch's, and its click is no tap (the first
/// finger drives the pointer, so a pinch it held still ends in a click). Call it every frame the widget `id` shows:
/// its latch is in egui's memory.
pub fn pinch_press(ui: &egui::Ui, id: egui::Id) -> bool {
    let (multi, pressed) = ui.input(|i| (i.multi_touch().is_some(), i.pointer.any_pressed()));
    ui.data_mut(|d| {
        let pinch = d.get_temp_mut_or_default::<bool>(id.with("pinch press"));
        *pinch = *pinch && !pressed || multi;
        *pinch
    })
}
// The editors' scroll and zoom controller.
pub mod scroller;

/// The temp-data key of the pass that last showed a modal backdrop.
const MODAL_PASS: &str = "modal backdrop pass";

/// Marks this pass as showing a modal: a message, dialog, side panel, menu or popup that takes every click outside it.
/// [`backdrop`] marks it; a modal with its own outside catcher (the main menu) calls this every frame it shows. Undo
/// and redo wait meanwhile ([`crate::history::shortcut`]).
pub(crate) fn modal_shown(ctx: &egui::Context) {
    let pass = ctx.cumulative_pass_nr();
    ctx.data_mut(|d| d.insert_temp(egui::Id::new(MODAL_PASS), pass));
    // The keyboard focus stays inside it: what it draws from here on.
    focus::modal(ctx, egui::Order::Background);
}

/// The temp-data key of [`hand_close`].
const CLOSE_PASS: &str = "close handed to the topmost overlay";

/// Hands this pass's `close` press, which [`views::take_escape`] held back from the overlays below, to the topmost
/// one: the next [`close_pressed`] in this pass takes it.
pub(crate) fn hand_close(ctx: &egui::Context) {
    let pass = ctx.cumulative_pass_nr();
    ctx.data_mut(|d| d.insert_temp(egui::Id::new(CLOSE_PASS), pass));
}

/// The keymap's `close` (Escape by default: `Close dialog`, `Dismiss menu`, `Dismiss dropdown`) for the
/// dialog, menu or popup that has the keyboard: the press [`hand_close`] handed on, or this frame's, consumed. One
/// press closes one level: an auto-repeat of a held Escape is consumed and does nothing.
pub(crate) fn close_pressed(ctx: &egui::Context) -> bool {
    let (pass, id) = (ctx.cumulative_pass_nr(), egui::Id::new(CLOSE_PASS));
    if ctx.data(|d| d.get_temp::<u64>(id)) == Some(pass) {
        ctx.data_mut(|d| d.remove::<u64>(id));
        return true;
    }
    crate::keymap::pressed(ctx, "close")
}

/// A modal showed in this pass so far ([`modal_shown`]).
pub(crate) fn modal_open(ctx: &egui::Context) -> bool {
    let pass = ctx.cumulative_pass_nr();
    ctx.data(|d| d.get_temp::<u64>(egui::Id::new(MODAL_PASS))) == Some(pass)
}

/// A modal showed in the last completed pass: [`modal_open`] read after the frame, for tests.
#[cfg(test)]
pub(crate) fn modal_was_open(ctx: &egui::Context) -> bool {
    let pass = ctx.cumulative_pass_nr();
    ctx.data(|d| d.get_temp::<u64>(egui::Id::new(MODAL_PASS))) == Some(pass.wrapping_sub(1))
}

/// The backdrop of modal `id`: a full-screen area `id.with("backdrop")` at `Order::Middle`, below the modal (at
/// `Foreground`) and above the screens, that takes every click outside it. It marks the pass ([`modal_shown`]). Call
/// it every frame the modal shows; its response says whether a click landed outside.
pub(crate) fn backdrop(ctx: &egui::Context, id: egui::Id) -> egui::Response {
    modal_shown(ctx);
    // What it draws above its backdrop.
    focus::modal(ctx, egui::Order::Foreground);
    let screen = ctx.content_rect();
    egui::Area::new(id.with("backdrop"))
        .order(egui::Order::Middle)
        .fixed_pos(screen.min)
        .constrain(false)
        .fade_in(false)
        .sense(egui::Sense::click_and_drag())
        .show(ctx, |ui| {
            ui.allocate_exact_size(screen.size(), egui::Sense::hover())
        })
        .response
}

/// One finger's step this frame ([`fingers`]): its id (a touch's id, or [`MOUSE`]) and where it is.
#[derive(Clone, Copy, Debug, PartialEq)]
pub enum Finger {
    Down(u64, egui::Pos2),
    Move(u64, egui::Pos2),
    Up(u64, egui::Pos2),
}

/// The primary mouse button's finger id.
pub const MOUSE: u64 = u64::MAX;

/// This frame's finger steps over the widget `id`, in order. Every touch (egui's `Event::Touch`) is a finger of its
/// own; the primary mouse button is one more, in frames without touch events (egui's integrations repeat the first
/// touch as the pointer, in the same frames). A finger goes down only where `id` is the top interactive widget (last
/// frame's widget rects, as egui's own hit test), then moves and lifts wherever it goes. The caller keeps the fingers
/// it holds, and lets go of them all on a [`focus_lost`]: the lift may never come.
pub fn fingers(ui: &egui::Ui, id: egui::Id) -> Vec<Finger> {
    fingers_on(ui, |w| w == id)
}

/// [`fingers`] over every widget whose id `on` takes: a finger goes down where one of them is the top one.
pub fn fingers_on(ui: &egui::Ui, on: impl Fn(egui::Id) -> bool) -> Vec<Finger> {
    use egui::{Event, PointerButton, TouchPhase};
    let ctx = ui.ctx();
    let on_top = |pos: egui::Pos2| {
        let layers: Vec<egui::LayerId> = ctx.memory(|m| m.layer_ids().collect());
        ctx.viewport(|v| {
            (layers.iter())
                .flat_map(|&layer| v.prev_pass.widgets.get_layer(layer))
                .filter(|w| w.sense.interactive() && w.interact_rect.contains(pos))
                .last()
                .is_some_and(|w| on(w.id))
        })
    };
    let events = ui.input(|i| i.events.clone());
    let touches = events.iter().any(|e| matches!(e, Event::Touch { .. }));
    let down = |finger, pos| on_top(pos).then_some(Finger::Down(finger, pos));
    let primary = |button| button == PointerButton::Primary;
    (events.into_iter())
        .filter_map(|e| match e {
            Event::Touch { id, phase, pos, .. } => match phase {
                TouchPhase::Start => down(id.0, pos),
                TouchPhase::Move => Some(Finger::Move(id.0, pos)),
                TouchPhase::End | TouchPhase::Cancel => Some(Finger::Up(id.0, pos)),
            },
            _ if touches => None,
            Event::PointerMoved(pos) => Some(Finger::Move(MOUSE, pos)),
            Event::PointerButton {
                pos,
                button,
                pressed,
                ..
            } if primary(button) => match pressed {
                true => down(MOUSE, pos),
                false => Some(Finger::Up(MOUSE, pos)),
            },
            _ => None,
        })
        .collect()
}
// Keyboard focus and the focus cursor.
pub mod focus;
// The browsers' search bar.
pub mod search_bar;
