//! The automation editor view: the clip of an automation lane in the editor area, drawn and edited by
//! [`AutomationEditor`]. It opens on a clip's `Edit`, has `Undo` and `Preview`, and goes back to the playlist once its
//! clip is gone.

use std::collections::HashMap;

use eframe::egui;

use super::{AppCtx, Nav, held_clip};
use crate::song::{LaneKind, Song};
use crate::ui::automation::{AutomationEditor, Options};
use crate::ui::preview::ClipKey;

/// What the automation editor keeps between frames.
#[derive(Default)]
pub struct State {
    /// Each channel's current automation lane and clip id, by channel index (the app re-keys them when channels or
    /// tracks move, `app/tracks.rs`).
    pub(crate) clips: HashMap<usize, (usize, u32)>,
    /// The editor: its snap, the selection and every clip's view, shared by the channels.
    pub(crate) editor: AutomationEditor,
    /// The init value of the shown clip's target while the song holds its automated value for the frame (the app sets
    /// it each frame, [`Options::init`]).
    pub(crate) init: Option<f32>,
}

impl State {
    /// The clip `channel`'s editor edits, while it is an automation clip.
    pub fn current(&self, song: &Song, channel: usize) -> Option<ClipKey> {
        held_clip(
            song,
            channel,
            self.clips.get(&channel),
            LaneKind::Automation,
        )
    }
}

/// Draws `channel`'s automation editor in `app.regions.editor`, on the clip the app points it at
/// ([`super::Slots::follow`], which collapses the editor once that clip is gone). Each gesture or op is one final edit; a
/// ruler tap moves the playhead.
pub fn show(state: &mut State, ui: &mut egui::Ui, app: &mut AppCtx, channel: usize) -> Option<Nav> {
    let Some(key) = state.current(app.song, channel) else {
        state.clips.remove(&channel);
        return None;
    };
    let opts = Options {
        playhead: app.snapshot.map_or(0, |s| s.transport_tick),
        lead: app.audio.playhead_lead(),
        playing: app.playing,
        armed: app.armed,
        previewing: app.previewing,
        covered: app.covered,
        continuous: app.settings.continuous_scroll,
        init: state.init,
        sample_rate: app.audio.sample_rate().map_or(0.0, |r| r as f32),
    };
    let rect = app.regions.editor;
    // The value snap is an app setting: the snap menu's pick is saved with the settings.
    state.editor.value_snap = app.settings.automation_value_snap;
    let out = (state.editor).show(ui, &app.cx, rect, app.song, key, &opts);
    app.settings.automation_value_snap = state.editor.value_snap;
    if out.edited {
        app.song_edited(true);
    }
    if let Some(tick) = out.seek {
        app.set_playhead(tick);
    }
    if out.play {
        app.play();
    }
    if out.dropped.is_some() {
        app.seek = out.dropped;
    }
    None
}
