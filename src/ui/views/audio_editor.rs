//! The audio clip editor view: the clip of an audio lane in the editor area. It opens on a clip's `Edit` or on an audio
//! channel's `Edit`, has `Undo` and `Preview`, and goes back to the playlist once its clip is gone. [`AudioEditor`]
//! draws and edits it; its edits go through the funnel, one undo step each, but a region op goes to the app
//! ([`State::request`]), which saves the edited sample first. The clip's sample comes from the app's sample cache,
//! decoded on first sight.

use std::collections::HashMap;

use eframe::egui::{self, UiBuilder};

use super::{AppCtx, Nav, held_clip};
use crate::sample_cache;
use crate::song::{LaneKind, Song};
use crate::ui::audio_editor::{self, Action, At, AudioEditor, Transport};
use crate::ui::preview::ClipKey;

/// What the audio editor keeps between frames.
#[derive(Default)]
pub struct State {
    /// Each channel's current audio lane and clip id, by channel index (the app re-keys them when channels or tracks
    /// move, `app/tracks.rs`).
    pub(crate) clips: HashMap<usize, (usize, u32)>,
    /// The editor: its lane, snap, views and gestures.
    pub(crate) editor: AudioEditor,
    /// A region op for the app (`app/audio_edit.rs`): its envelope part and its audio part are one edit, made once the
    /// edited sample is saved.
    pub(crate) request: Option<(At, Action)>,
    /// The name the `New sample name` dialog's `SAVE` gave, for the app.
    pub(crate) named: Option<String>,
}

impl State {
    /// The clip `channel`'s editor edits, while it is an audio clip.
    pub fn current(&self, song: &Song, channel: usize) -> Option<ClipKey> {
        held_clip(song, channel, self.clips.get(&channel), LaneKind::Audio)
    }
}

/// Draws `channel`'s audio editor in `app.regions.editor`, on the clip it holds for the channel (the app points it at the
/// playlist's current clip, [`super::Slots::follow`], which collapses the editor once that clip is gone).
pub fn show(state: &mut State, ui: &mut egui::Ui, app: &mut AppCtx, channel: usize) -> Option<Nav> {
    let Some(key) = state.current(app.song, channel) else {
        state.clips.remove(&channel);
        return None;
    };
    let at = (key.channel, key.lane, key.id);
    let clip = &app.song.channels[at.0].lanes[at.1].clips;
    let slot = (clip.iter().find(|c| c.id == at.2)).and_then(|c| c.file.sample.as_deref());
    let sample =
        slot.and_then(|s| (app.audio.samples).waveform(s, &sample_cache::reader(app.files)));
    let transport = Transport {
        tick: app.snapshot.map_or(0, |s| s.transport_tick),
        lead: app.audio.playhead_lead(),
        playing: app.playing,
        armed: app.armed,
        previewing: app.previewing,
        covered: app.covered,
    };
    let mut ui = ui.new_child(
        UiBuilder::new()
            .id(("audio_editor", channel))
            .max_rect(app.regions.editor),
    );
    let actions = (state.editor).show(&mut ui, &app.cx, app.song, at, sample.as_ref(), transport);
    for action in actions {
        match action {
            Action::SetPlayhead(tick) => app.set_playhead(tick),
            Action::Play => app.play(),
            Action::Seek(tick) => app.seek = Some(tick),
            // Its envelope part waits for its audio part: the app makes both, after the save.
            Action::Region { .. } => state.request = Some((at, action)),
            action => {
                if audio_editor::apply(app.song, at, &action) {
                    app.song_edited(true);
                }
            }
        }
    }
    None
}
