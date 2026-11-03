//! The fine tune dialog's slot ([`Overlay::FineTune`](super::Overlay::FineTune)): [`crate::ui::fine_tune`]'s dialog
//! on one param of one channel, wired as a hand move: it sets the param, records it as a hand move does and refreshes
//! the value text.
//!
//! Each change is written to the param in the song and sent to the audio as `SetParam` (a drum pad's address is its
//! −slot id, a send level's 0x2000 + its target), a channel param edit: the song-changed flag and no undo step. REC
//! records it as it records the rack's knobs: the param is the recent control. A DW Sampler's kept Attack or Release
//! init value follows it, and a note effect's edit rebuilds the engine once the pointer is up, as on its rack card, or
//! as the dialog closes mid-drag. A gone param (its module deleted, or its channel deleted or undone away) closes the
//! dialog.

use eframe::egui;
use honeydew_audio::device::AudioCommand;

use super::{AppCtx, Nav};
use crate::params::TextCx;
use crate::song::AutomationTarget;
use crate::theme::channel_color;
use crate::ui::fine_tune::{self, Dialog};

/// What the slot keeps between frames.
#[derive(Debug, Default)]
pub struct State {
    /// A note effect's param changed in the drag under way: the engine is rebuilt when it ends.
    note_edit: bool,
}

/// Shows the dialog for `target` on channel `channel`. Returns `Nav::Close` when it closes or its param is gone.
pub fn show(
    state: &mut State,
    ui: &mut egui::Ui,
    app: &mut AppCtx,
    channel: usize,
    target: AutomationTarget,
) -> Option<Nav> {
    let param = fine_tune::param(app.song, channel, target);
    let ch = app.song.channels.get(channel);
    let (Some(param), Some(ch)) = (param, ch) else {
        return Some(Nav::Close);
    };
    let mut value = ch.param(target)?;
    // A module param's text may read the module's other params, the tempo and the sample rate.
    let module = (target.instance_id > 0).then(|| ch.rack.get(target.instance_id));
    let params = module
        .flatten()
        .map(|m| m.params.clone())
        .unwrap_or_default();
    let env = TextCx {
        params: &params,
        tempo_bpm: app.song.tempo_bpm,
        sample_rate: app.audio.sample_rate().map_or(0.0, |r| r as f32),
    };
    let dialog = Dialog {
        title: &param.title,
        ink: channel_color(ch.hue, app.cx.theme).to_color32(),
        reset: param.reset,
    };
    let text = |v| param.spec.display_text_in(v, env);
    let screen = app.regions.screen;
    let shown = fine_tune::show(ui.ctx(), &app.cx, screen, &dialog, &mut value, text);
    if shown.changed {
        let ch = &mut app.song.channels[channel];
        if let Some(v) = ch.param_mut(target) {
            *v = value;
        }
        if let Some(module) = ch.rack.get_mut(target.instance_id) {
            module.init_follows(target.param);
            crate::panels::couple_param(module, target.param);
        }
        app.audio.send(AudioCommand::SetParam {
            channel,
            instance_id: target.instance_id,
            index: target.param,
            value,
        });
        app.channel_param_edited();
        state.note_edit |= param.note_effect;
    }
    // The drag's end, or the dialog closing under it (Escape mid-drag): the note effect's edit is built in once.
    if state.note_edit && (shown.close || !ui.input(|i| i.pointer.any_down())) {
        state.note_edit = false;
        app.song_edited(true);
    }
    shown.close.then_some(Nav::Close)
}
