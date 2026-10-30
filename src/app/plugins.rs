//! Third-party plugins in the app (desktop; `crate::plugins`): the scan's start and results, the instances kept in
//! step with the song every frame, the param changes a plugin's own editor makes, and what went wrong.
//!
//! A plugin's editor moving one of its params is a hand move on the module: the song value, a `SetParam` for the
//! engine's saved value, the song-changed flag and no undo step ([`SongEdit::ChannelParam`], as the rack's controls),
//! and it is the recent control, so REC records it and the CTRL menu automates it.

use eframe::egui;
use honeydew_audio::device::AudioCommand;

use super::HoneydewApp;
use crate::plugins;
use crate::song::AutomationTarget;
use crate::ui::views::{Nav, Overlay, SongEdit};

/// The message tag of the plugins' notices.
const NOTICE_TAG: &str = "plugin notice";

impl HoneydewApp {
    /// At the start: the cached scan, and a background scan of new and changed plugin files.
    pub(super) fn start_plugins(&mut self, ctx: &egui::Context) {
        let cache = self
            .storage
            .as_deref()
            .and_then(|s| s.get_string(plugins::CACHE_KEY));
        plugins::start(ctx, cache, &self.settings.plugin_folders);
    }

    /// Every frame, early: the scan's result kept, the instances matched to the song, the plugins' own param changes
    /// applied, and their notices shown once no other message is.
    pub(super) fn plugins_frame(&mut self, ctx: &egui::Context, tick: u64) {
        if plugins::poll_scan()
            && let Some(storage) = self.storage.as_deref_mut()
        {
            storage.set_string(plugins::CACHE_KEY, plugins::cache_json());
            self.settings_unflushed = true;
        }
        plugins::sync(&self.song);
        if plugins::adopt_params(&mut self.song) {
            self.song_edited(SongEdit::RackRebuild, tick);
        }
        let mut moved = false;
        for m in plugins::idle() {
            let Some(channel) = self.song.channels.iter().position(|c| c.id == m.channel_id) else {
                continue;
            };
            let target = AutomationTarget {
                instance_id: m.instance_id,
                param: m.param,
            };
            self.recent = Some((m.channel_id, target));
            let Some(value) = self.song.channels[channel]
                .param_mut(target)
                .filter(|_| m.gesture == 1)
            else {
                continue;
            };
            *value = m.value.clamp(0.0, 1.0);
            self.audio.send(AudioCommand::SetParam {
                channel,
                instance_id: m.instance_id,
                index: m.param,
                value: m.value.clamp(0.0, 1.0),
            });
            moved = true;
        }
        if moved {
            self.song_edited(SongEdit::ChannelParam, tick);
        }
        let message = |o: &Overlay| matches!(o, Overlay::Message { .. });
        if !self.nav.overlays.iter().any(message) {
            let lines = plugins::take_notices();
            if !lines.is_empty() {
                self.navigate(Nav::Open(Overlay::Message {
                    tag: NOTICE_TAG,
                    lines,
                    confirm: None,
                    extra: None,
                }));
            }
        }
        if let Some(after) = plugins::idle_after() {
            ctx.request_repaint_after(after);
        }
    }
}
