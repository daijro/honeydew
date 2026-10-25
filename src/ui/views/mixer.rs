//! The main mixer view: [`crate::ui::mixer`]'s strips, wired to the audio.

use eframe::egui;
use honeydew_audio::device::AudioCommand;

use super::{AppCtx, Nav};
use crate::ui::mixer::{self, MixerState};

/// What the mixer keeps between frames: its scroll and the strip selection.
pub type State = MixerState;

/// Draws the mixer in `app.regions.editor`. A changed param is already in the song; it goes to the channel strip
/// (instance 0) live, and sets the song-changed flag with no undo step. A strip tap off its controls makes the channel
/// current, the master's too (its rack holds the master effects). A strip control's touch stays in
/// [`MixerState::touch`] for the app's recent control.
pub fn show(state: &mut State, ui: &mut egui::Ui, app: &mut AppCtx) -> Option<Nav> {
    state.selected = Some(*app.selected);
    state.covered = app.covered;
    let changes = mixer::show(ui, &app.cx, app.song, state, app.regions.editor);
    if !changes.is_empty() {
        app.channel_param_edited();
    }
    for (channel, index, value) in changes {
        app.audio.send(AudioCommand::SetParam {
            channel,
            instance_id: 0,
            index,
            value,
        });
    }
    if let Some(channel) = state.selected {
        *app.selected = channel;
    }
    None
}
