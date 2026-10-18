//! The Modules browser, adding to a channel's rack.

use eframe::egui;

use super::{AppCtx, Nav, Overlay, preset_picker};
use crate::plugins;
use crate::song::ModuleInstance;
use crate::ui::modules_browser::{BrowserAction, InsertAt, ModulesBrowser, add_module, add_plugin};
use crate::ui::rack::{Held, Outcome, PresetTarget, hold_removal, paste, takes_instruments};

/// The flash of a `Paste` with nothing on the clipboard that the channel takes.
pub const CANNOT_PASTE: &str = "Cannot paste module";

/// The open browser. It starts afresh on each opening, however it was last closed: the slide-in, an empty search.
#[derive(Default)]
pub struct State {
    browser: ModulesBrowser,
    /// The egui pass it was last drawn in: a gap means it was closed in between.
    pass: u64,
    /// A `Replace` of a module automation lanes target, held back ([`hold_removal`]): the app asks `Delete automation?`
    /// first.
    pub(crate) ask_remove: Option<Held>,
    /// The module this session's last pick added, by instance id: the next pick replaces it, in its slot.
    added: Option<i32>,
}

/// Draws the browser, modal, over the left of the screen. A pick adds the module to `channel`'s rack at `at` (an
/// instrument only where the channel takes one: its SYNTH rows are dimmed elsewhere) with no undo step, and the browser
/// stays open: each later pick of the session (an opening starts one) replaces the module the last one added, in its
/// slot, so the whole browse adds one module. `Paste` (always enabled) copies the `clipboard` modules that fit the
/// channel ([`paste`]), a final song edit, and closes it; a `Paste` of nothing that fits flashes [`CANNOT_PASTE`]
/// instead. A `Replace` of a module automation lanes target is held back for the app's question
/// ([`State::ask_remove`]), and closes it. `Rack Preset` gives way to the rack preset picker, for the same place. The
/// X, a click outside, Escape or the channel being gone close it. It opens in the ordering the stored
/// `ModuleBrowserOrdering` names, else Category, and stores what its `Sort` menu picks there at once. A DW Sampler
/// picked loads Close Grand, through `picker` ([`preset_picker::load_close_grand`]).
pub fn show(
    state: &mut State,
    picker: &mut preset_picker::State,
    ui: &mut egui::Ui,
    app: &mut AppCtx,
    channel: usize,
    at: InsertAt,
    clipboard: &[ModuleInstance],
) -> Option<Nav> {
    let pass = ui.ctx().cumulative_pass_nr();
    if state.pass + 1 != pass {
        // Its ordering is the stored one, applied at each opening.
        state.browser = ModulesBrowser::default();
        state.browser.sort = app.settings.module_ordering();
        state.added = None;
    }
    state.pass = pass;
    let action = match app.song.channels.get(channel) {
        Some(ch) => {
            let instruments = takes_instruments(ch);
            let action = state.browser.show(ui.ctx(), &app.cx, instruments);
            // A pick from `Sort`'s menu is saved as `ModuleBrowserOrdering`, the active one too; an opening saves
            // nothing.
            if let Some(sort) = state.browser.sort_picked.take() {
                app.settings.module_browser_ordering = sort.id();
            }
            action?
        }
        None => BrowserAction::Close,
    };
    let before = app.song.channels.get(channel).cloned();
    let edited = match (action, app.song.channels.get_mut(channel)) {
        (BrowserAction::Pick(type_id), Some(ch)) => {
            let at = state.added.map_or(at, InsertAt::Replace);
            let id = add_module(&mut ch.rack, type_id, at);
            state.added = Some(id);
            // Every new DW Sampler loads Close Grand.
            preset_picker::load_close_grand(picker, app.files, ch.id, id, type_id);
            true
        }
        (BrowserAction::Plugin(uid), Some(ch)) => {
            let at = state.added.map_or(at, InsertAt::Replace);
            match add_plugin(&mut ch.rack, ch.id, uid, at) {
                Ok(id) => {
                    state.added = Some(id);
                    // Its own editor opens at once if it has one; a failure shows on its card.
                    if let Some(plugins::State::Live(info)) = plugins::state(ch.id, id)
                        && info.has_editor
                    {
                        let _ = plugins::show_editor(ch.id, id, true);
                    }
                    true
                }
                Err(why) => {
                    app.files.flash(why);
                    false
                }
            }
        }
        (BrowserAction::Paste, Some(ch)) => {
            let pasted = paste(ch, clipboard, at) == Outcome::ChainChanged;
            if !pasted {
                app.files.flash(CANNOT_PASTE);
            }
            pasted
        }
        (BrowserAction::RackPreset, Some(_)) => {
            let target = PresetTarget::RackInsert(at);
            return Some(Nav::Swap(Overlay::PresetPicker { channel, target }));
        }
        _ => false,
    };
    if edited {
        let ch = &mut app.song.channels[channel];
        match before.and_then(|before| hold_removal(ch, before)) {
            Some(after) => {
                state.ask_remove = Some(Held {
                    channel,
                    after,
                    skipped: 0,
                });
                return Some(Nav::Close);
            }
            None if matches!(action, BrowserAction::Pick(_) | BrowserAction::Plugin(_)) => {
                // A pick marks the song changed, with no undo step.
                app.files.song_changed();
                app.pick_loaded();
                return None;
            }
            None => app.song_edited(true),
        }
    }
    Some(Nav::Close)
}
