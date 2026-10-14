//! The keymap's global actions, tested at the app root after the screens: the focused control and its ancestors (the
//! editor grid's editing and zoom actions, `ui/edit_keys.rs`) had the keys first. They go before undo / redo and the
//! typing-keyboard notes, so a key they take never plays a note.
//!
//! Details:
//! - Like undo and redo, they wait while a text field has the keyboard or a modal shows. The exceptions act on
//!   the modal on top: `showTempo` closes the tempo dialog, `showMainBrowser` / `showMainSettings` switch the open main
//!   menu's page.
//! - The three-way `showRack` / `showMainMixer` / `showTransport` (hidden → show and focus; shown, focus outside →
//!   focus; focus inside → hide, the focus back to the root) go through [`HoneydewApp::focus_panel`] and
//!   [`HoneydewApp::clear_focus`] on KF's focus containers (`ui/focus.rs`); a panel shown and focused at once keeps the
//!   focus that was asked for until it draws.
//! - `showRack` opens the selected channel's rack, as the edge tab does, and hides any open one.
//! - `focusEditor` leaves the mixer for the playlist, then asks [`HoneydewApp::focus_panel`] for the bottom editor, or the
//!   playlist while none is open; it opens no editor.
//! - `enterKbdFocus` toggles: into the keyboard panel, else back to the root ([`HoneydewApp::clear_focus`]), as a hidden
//!   panel's focus does.
//! - `showMainBrowser` moves the focus into the SONGS list ([`focus::SONGS`]) whether or not the menu was open, so
//!   `startSearch` works there. `startSearch` itself is no global action: the browsers take it.

use eframe::egui::{self, InputState};

use super::HoneydewApp;
use crate::keymap::{self, Keymap};
use crate::ui::focus;
use crate::ui::views::{MenuTab, Nav, Overlay, View};

/// A global action.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum Global {
    Play,
    Rewind,
    ArmRecord,
    /// `showMainBrowser` (SONGS) and `showMainSettings` (SETTINGS).
    MainMenu(MenuTab),
    Panel(Panel),
    Tempo,
    FocusEditor,
    KbdFocus,
    /// `toggleKeyboardPanel`: the tap toggle with the release snap.
    TogglePanel,
}

/// What the three-way actions show, hide and focus, and the keyboard panel of `enterKbdFocus`.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum Panel {
    Rack,
    Mixer,
    /// The bottom keyboard panel (the "transport bar").
    Transport,
    Editor,
    Keyboard,
}

const GLOBALS: [(&str, Global); 12] = [
    ("play", Global::Play),
    ("rewind", Global::Rewind),
    ("armRecord", Global::ArmRecord),
    ("showMainBrowser", Global::MainMenu(MenuTab::Songs)),
    ("showMainSettings", Global::MainMenu(MenuTab::Settings)),
    ("showRack", Global::Panel(Panel::Rack)),
    ("showMainMixer", Global::Panel(Panel::Mixer)),
    ("showTransport", Global::Panel(Panel::Transport)),
    ("showTempo", Global::Tempo),
    ("focusEditor", Global::FocusEditor),
    ("enterKbdFocus", Global::KbdFocus),
    ("toggleKeyboardPanel", Global::TogglePanel),
];

/// The global actions pressed this frame that `allowed` lets through, in [`GLOBALS`] order, their keys consumed.
pub(crate) fn take(
    keys: &Keymap,
    input: &mut InputState,
    allowed: impl Fn(Global) -> bool,
) -> Vec<Global> {
    (GLOBALS.iter())
        .filter(|&&(action, g)| allowed(g) && keys.consume_press(input, action))
        .map(|&(_, g)| g)
        .collect()
}

impl HoneydewApp {
    /// This frame's global actions. `rewind` is the frame's REV flag and `navs` its navigation, which the frame
    /// applies after.
    pub(super) fn global_keys(
        &mut self,
        ctx: &egui::Context,
        rewind: &mut bool,
        navs: &mut Vec<Nav>,
    ) {
        if ctx.wants_keyboard_input() {
            return;
        }
        let modal = crate::ui::modal_open(ctx);
        let top = self.nav.overlays.last();
        let hidden = self.main_screen.panel_hidden;
        let allowed = |g| match g {
            // The panel's size toggles only while it shows.
            Global::TogglePanel => !modal && !hidden,
            Global::MainMenu(_) => !modal || matches!(top, Some(Overlay::MainMenu(_))),
            Global::Tempo => !modal || top == Some(&Overlay::TempoEditor),
            _ => !modal,
        };
        let keys = keymap::for_os(ctx.os());
        for g in ctx.input_mut(|i| take(keys, i, allowed)) {
            self.global(ctx, g, rewind, navs);
        }
    }

    /// Applies global action `g`, as [`Self::global_keys`] does.
    pub(super) fn global(
        &mut self,
        ctx: &egui::Context,
        g: Global,
        rewind: &mut bool,
        navs: &mut Vec<Nav>,
    ) {
        match g {
            Global::Play => self.main_screen.playing ^= true,
            Global::Rewind => *rewind = true,
            Global::ArmRecord => self.main_screen.armed ^= true,
            Global::MainMenu(tab) => {
                match self.nav.overlays.last_mut() {
                    Some(Overlay::MainMenu(open)) => *open = tab,
                    _ => navs.push(Nav::Open(Overlay::MainMenu(tab))),
                }
                // `showMainBrowser` also puts the focus in the SONGS list, where `startSearch` works.
                if tab == MenuTab::Songs {
                    let songs = egui::Id::new(focus::SONGS);
                    focus::enter(ctx, songs);
                    focus::focus(ctx, songs);
                }
            }
            Global::Panel(panel) if !self.panel_shown(panel) => {
                self.show_panel(panel, navs);
                self.focus_panel(ctx, panel);
            }
            Global::Panel(panel) => {
                if !self.focus_panel(ctx, panel) {
                    self.hide_panel(panel, navs);
                    self.clear_focus(ctx);
                }
            }
            Global::Tempo => navs.push(Nav::Toggle(Overlay::TempoEditor)),
            // Hides the main mixer, then focuses the editor.
            Global::FocusEditor => {
                if self.nav.view == View::Mixer {
                    navs.push(Nav::Show(View::Playlist));
                }
                self.focus_panel(ctx, Panel::Editor);
            }
            // A toggle: into the keyboard panel, or (already there) back to the app root.
            Global::KbdFocus => {
                if !self.focus_panel(ctx, Panel::Keyboard) {
                    self.clear_focus(ctx);
                }
            }
            Global::TogglePanel => {
                let m = crate::metrics::Metrics::new(ctx, self.settings.ui_scale_index);
                let sizes = super::toggle_snapped(&self.settings, ctx.content_rect().height(), &m);
                self.set_panel_sizes(sizes, &m);
            }
        }
    }

    /// Moves the keyboard focus into `panel`'s focus container (`ui/focus.rs`) unless it is inside already, and says
    /// whether it moved. The keyboard panel is the transport's container.
    pub(crate) fn focus_panel(&mut self, ctx: &egui::Context, panel: Panel) -> bool {
        let id = egui::Id::new(match panel {
            Panel::Rack => focus::RACK,
            Panel::Mixer => focus::MIXER,
            Panel::Transport | Panel::Keyboard => focus::TRANSPORT,
            Panel::Editor if matches!(self.nav.view, View::Playlist | View::Mixer) => {
                focus::PLAYLIST
            }
            Panel::Editor => focus::EDITOR,
        });
        let outside = !focus::within(ctx, id);
        if outside {
            focus::focus(ctx, id);
        }
        outside
    }

    /// Gives the keyboard focus back to the app root: a panel hidden while it had it, `enterKbdFocus` from inside the
    /// keyboard panel.
    pub(crate) fn clear_focus(&mut self, ctx: &egui::Context) {
        focus::clear(ctx);
    }

    fn panel_shown(&self, panel: Panel) -> bool {
        match panel {
            Panel::Rack => (self.nav.overlays.iter()).any(|o| matches!(o, Overlay::Rack(_))),
            Panel::Mixer => self.nav.view == View::Mixer,
            Panel::Transport => !self.main_screen.panel_hidden,
            Panel::Editor | Panel::Keyboard => true,
        }
    }

    fn show_panel(&mut self, panel: Panel, navs: &mut Vec<Nav>) {
        match panel {
            Panel::Rack => navs.push(Nav::Open(Overlay::Rack(self.selected_channel))),
            Panel::Mixer => navs.push(Nav::Show(View::Mixer)),
            Panel::Transport => self.main_screen.panel_hidden = false,
            Panel::Editor | Panel::Keyboard => {}
        }
    }

    fn hide_panel(&mut self, panel: Panel, navs: &mut Vec<Nav>) {
        match panel {
            Panel::Rack => (self.nav.overlays).retain(|o| !matches!(o, Overlay::Rack(_))),
            // Showing the mixer again goes back to the playlist.
            Panel::Mixer => navs.push(Nav::Show(View::Mixer)),
            Panel::Transport => self.main_screen.panel_hidden = true,
            Panel::Editor | Panel::Keyboard => {}
        }
    }
}
