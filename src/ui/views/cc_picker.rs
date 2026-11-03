//! `Add MIDI CC track…`'s picker (the CTRL menu's, the track menu's; issue #19): the side browser panel listing MIDI CC
//! 0–126 (there is no CC 127 channel param) by their lane names ([`ChannelParams::name`]), which `Search` filters by
//! number or name. A pick closes it and is the app's ([`State::picked`]). The X, a click outside, Escape or the channel
//! being gone close it.

use eframe::egui;

use super::{AppCtx, Nav};
use crate::song::{AutomationTarget, ChannelParams};
use crate::ui::modules_browser::{Chrome, PanelClick, PanelRow, RowKind, SideButton, SidePanel};

/// The channel params of MIDI CC 0 … 126.
const CC_PARAMS: std::ops::RangeInclusive<i32> = 7..=133;

/// The open picker, afresh on each opening, and its pick.
#[derive(Default)]
pub struct State {
    panel: SidePanel,
    /// The egui pass it was last drawn in: a gap means it was closed in between.
    pass: u64,
    /// The pick: the channel and the CC's channel param. The app takes it after the frame.
    pub picked: Option<(usize, AutomationTarget)>,
}

/// Draws the picker for a lane on `channel`.
pub fn show(state: &mut State, ui: &mut egui::Ui, app: &mut AppCtx, channel: usize) -> Option<Nav> {
    if app.song.channels.get(channel).is_none() {
        return Some(Nav::Close);
    }
    let pass = ui.ctx().cumulative_pass_nr();
    if state.pass + 1 != pass {
        state.panel = SidePanel::default();
    }
    state.pass = pass;
    let names: Vec<(i32, String)> = CC_PARAMS
        .filter_map(|p| Some((p, ChannelParams::name(p as usize)?)))
        .collect();
    let chrome = Chrome {
        id: egui::Id::new("cc_picker"),
        title: "MIDI CC",
        back: false,
        buttons: &[SideButton::Search],
        popup: false,
    };
    let click = state.panel.show(ui.ctx(), &app.cx, &chrome, |query| {
        let query = query.to_lowercase();
        (names.iter())
            .filter(|(_, name)| name.to_lowercase().contains(&query))
            .map(|(param, name)| PanelRow {
                name,
                folder: false,
                sub: None,
                kind: RowKind::Enabled(*param),
                picked: false,
            })
            .collect()
    });
    match click? {
        PanelClick::Row(param) => {
            let target = AutomationTarget {
                instance_id: 0,
                param,
            };
            state.picked = Some((channel, target));
        }
        PanelClick::Close => {}
        _ => return None,
    }
    Some(Nav::Close)
}
