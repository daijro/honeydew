//! The `•••` CTRL menu of the transport bar: [`crate::ui::ctrl_menu`]'s popup standing on the CTRL button.

use eframe::egui;

use super::{AppCtx, Nav};
use crate::ui::ctrl_menu::{CtrlMenu, IMPLEMENTED, cc_channel, menu_items};

/// What the CTRL menu keeps between frames, and the recent control the app gives it.
pub type State = CtrlMenu;

/// Shows the popup on `app.regions.ctrl`, its items enabled by `state.recent` and the selected channel ([`IMPLEMENTED`]
/// only). A pick goes to
/// `state.picked` for the app. A pick, a tap outside it (the CTRL button included) or Escape closes it.
pub fn show(state: &mut State, ui: &mut egui::Ui, app: &mut AppCtx) -> Option<Nav> {
    state.open = true;
    let cc = cc_channel(app.song, *app.selected);
    let items = menu_items(state.recent.as_ref(), cc, &IMPLEMENTED);
    if let Some(action) = state.show(ui.ctx(), app.regions.ctrl, &items, &app.cx) {
        state.picked = Some(action);
    }
    (!state.open).then_some(Nav::Close)
}
