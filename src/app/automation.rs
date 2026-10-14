//! Automation lanes from the panels' controls: the recent control,
//! the CTRL menu's `Add automation track` and `Link to recent control`, the parameter menu's `Add automation track`
//! (the MIDI CC items of both menus go on to `app/midi.rs`; `Fine tune control` opens the fine tune dialog), and the
//! `Delete automation?` question a card's `Delete` asks for a module that lanes target, as do
//! `Clear Rack`, a `Replace` and a rack preset's load, held back until the answer.
//!
//! The recent control is the channel (by id, which no move changes) and the [`AutomationTarget`] of the last control
//! bound to a parameter that was pressed: a rack card's module or common param, a mixer strip's (instance 0) or a drum
//! slot strip's (instance −slot id). It is held in memory and cleared with a new song.

use super::HoneydewApp;
use super::tracks::LaneShift;
use crate::midi::links;
use crate::song::{AutomationTarget, Channel, Lane, LaneKind, PPQ, Song};
use crate::ui::ctrl_menu::{CtrlAction, RecentControl};
use crate::ui::fine_tune;
use crate::ui::preview::ClipKey;
use crate::ui::rack::Held;
use crate::ui::views::{Nav, Overlay, SongEdit, View, preset_picker};

/// The tag of the `Delete automation?` message.
pub(crate) const DELETE_AUTOMATION_TAG: &str = "delete automation";

/// What `Delete automation?` waits on: a card's `Delete` of the module with this instance id, or a held edit with its
/// channel as it was when asked.
// One at a time, in the app: a `Box` would save nothing worth it.
#[allow(clippy::large_enum_variant)]
pub(super) enum Pending {
    Delete(i32),
    Held(Channel, Held),
}

/// What the CTRL menu knows of the recent control `recent` (channel index, target), with `track` the current track
/// (channel and lane index), if any. The "current channel" is the current track's, the one
/// whose rack its target names; its CC links are its own channel's `CCLK`, where `Link to MIDI CC` stores them. The
/// fine tune dialog's support is [`fine_tune::param`]'s.
pub(super) fn recent_control(
    song: &Song,
    (channel, target): (usize, AutomationTarget),
    track: Option<(usize, usize)>,
) -> RecentControl {
    let lane = track.and_then(|(c, l)| Some((c, song.channels.get(c)?.lanes.get(l)?)));
    let automation = lane.filter(|(_, l)| l.kind == LaneKind::Automation);
    let cc_links = song.channels.get(channel).map(|c| &c.file.cc_links[..]);
    RecentControl {
        channel_cc: target.instance_id == 0 && (4..=133).contains(&target.param),
        cc_linked: cc_links.is_some_and(|raw| links::linked(raw, target.instance_id, target.param)),
        on_current_channel: lane.is_some_and(|(c, _)| c == channel),
        track_automates_other: automation.is_some_and(|(_, l)| l.target != Some(target)),
        fine_tune: fine_tune::param(song, channel, target).is_some(),
    }
}

/// Adds a lane for `target` to `ch` (`Add automation track`), named `label` ([`Song::automation_label`]): kind 1,
/// no duplicate check, with one empty clip at `start` (ticks) looping every `beats`. Returns its lane index and clip
/// id.
fn add_lane(
    ch: &mut Channel,
    (target, label): (AutomationTarget, String),
    start: u32,
    beats: f64,
) -> (usize, u32) {
    let lane = ch.add_automation_lane(target);
    lane.label = label;
    let id = lane.add_clip(start, beats).id;
    let lane: *const Lane = lane;
    let index = (ch.lanes.iter()).position(|l| std::ptr::eq(l, lane));
    (index.expect("just added"), id)
}

/// Re-targets automation lane `lane` of `ch` to `target` (`Link to recent control`): its auto name follows (`label`,
/// [`Song::automation_label`]), its custom name goes, its points stay (the writer keys them by the lane's param).
/// Whether it changed.
fn link(ch: &mut Channel, lane: usize, (target, label): (AutomationTarget, String)) -> bool {
    let Some(l) = ch.lanes.get_mut(lane) else {
        return false;
    };
    if l.kind != LaneKind::Automation || l.target == Some(target) {
        return false;
    }
    (l.target, l.label) = (Some(target), label);
    l.file.custom_label.clear();
    true
}

impl HoneydewApp {
    /// The recent control's channel index and target, while both still exist.
    pub(super) fn recent_index(&self) -> Option<(usize, AutomationTarget)> {
        let (id, target) = self.recent?;
        let channel = self.song.channels.iter().position(|c| c.id == id)?;
        self.param_value(channel, target)?;
        Some((channel, target))
    }

    /// The current track (channel, lane): the open automation editor's lane, else the playlist's selected clip's.
    pub(super) fn current_track(&self) -> Option<(usize, usize)> {
        let editor = match self.nav.view {
            View::AutomationEditor(c) => self.slots.automation_editor.current(&self.song, c),
            _ => None,
        };
        (editor.map(|k| (k.channel, k.lane)))
            .or_else(|| self.main_screen.clips.selected.map(|r| (r.channel, r.lane)))
    }

    /// The CTRL menu's view of the recent control, `None` while there is none: the app gives it the menu every frame.
    pub(super) fn ctrl_recent(&self) -> Option<RecentControl> {
        let recent = self.recent_index()?;
        Some(recent_control(&self.song, recent, self.current_track()))
    }

    /// Takes the frame's touches of bound controls from the rack, the mixer and the drum strips, as they were drawn:
    /// the last one is the recent control. Returns the item one picked from its parameter menu (which made it recent
    /// first), for [`Self::automation`] once the frame's edit is in.
    pub(super) fn take_touches(&mut self) -> Option<CtrlAction> {
        let slots = &mut self.slots;
        let touches = [
            slots.rack.touch.take(),
            slots.mixer.touch.take(),
            slots.drum_editor.touch.take(),
            self.main_screen.touch.take(),
        ];
        let mut pick = None;
        for (channel, touch) in touches.into_iter().flatten() {
            if let Some(ch) = self.song.channels.get(channel) {
                self.recent = Some((ch.id, touch.target));
                pick = touch.pick.or(pick);
            }
        }
        pick
    }

    /// After the frame's edit: the CTRL menu's pick, else a parameter menu's (`pick`), on the recent control, and the
    /// rack's `Delete` or a held edit (`Clear Rack`, a `Replace`, a rack preset's load) that must ask, then its answer.
    /// Each lane op and CC link change is one undo step. `tick` is the transport position.
    pub(super) fn automation(&mut self, pick: Option<CtrlAction>, tick: u64) {
        match self.slots.ctrl_menu.picked.take().or(pick) {
            Some(CtrlAction::AddAutomationTrack) => self.add_automation_track(tick),
            Some(CtrlAction::LinkRecentControl) => self.link_recent_control(tick),
            Some(CtrlAction::LinkMidiCc) => self.learn_cc(),
            Some(CtrlAction::LinkModWheel) => self.link_cc(Some(1), tick),
            Some(CtrlAction::UnlinkMidiCc) => self.link_cc(None, tick),
            Some(CtrlAction::FineTune) => self.fine_tune(),
            None => {}
        }
        if let Some((channel, instance_id)) = self.slots.rack.ask_delete.take()
            && let Some(ch) = self.song.channels.get(channel)
        {
            self.delete_asked = Some((ch.id, Pending::Delete(instance_id)));
            self.ask_delete_automation();
        }
        let slots = &mut self.slots;
        let held = [
            &mut slots.rack.ask_remove,
            &mut slots.modules_browser.ask_remove,
            &mut slots.preset_picker.ask_remove,
        ];
        if let Some(held) = held.into_iter().filter_map(Option::take).last()
            && let Some(ch) = self.song.channels.get(held.channel)
        {
            self.delete_asked = Some((ch.id, Pending::Held(ch.clone(), held)));
            self.ask_delete_automation();
        }
        self.delete_answered(tick);
    }

    fn ask_delete_automation(&mut self) {
        self.navigate(Nav::Open(Overlay::Message {
            tag: DELETE_AUTOMATION_TAG,
            lines: vec!["Delete automation?".into()],
            confirm: Some(("Delete".into(), "Keep".into())),
            extra: None,
        }));
    }

    /// `Add automation track` for the recent control: a new lane on its channel with one empty clip at floor(song
    /// beat), b beats long; the lanes re-keyed, the channel selected (never the master: it is only selected while it is
    /// alone) and the automation editor opened on the clip.
    fn add_automation_track(&mut self, tick: u64) {
        let Some((channel, target)) = self.recent_index() else {
            return;
        };
        let start = u32::try_from(tick / u64::from(PPQ) * u64::from(PPQ)).unwrap_or(u32::MAX);
        let beats = f64::from(self.song.time_sig.bar_beats());
        let label = self
            .song
            .automation_label(channel, target)
            .unwrap_or_default();
        let ch = &mut self.song.channels[channel];
        let (lane, id) = add_lane(ch, (target, label), start, beats);
        self.rekey(&[LaneShift::new(channel, lane, true)]);
        self.song_edited(SongEdit::Final, tick);
        if channel > 0 {
            self.selected_channel = channel;
        }
        self.navigate(Nav::EditClip(ClipKey { channel, lane, id }));
    }

    /// `Link to recent control`: the current track, an automation lane of the recent control's channel, automates it.
    fn link_recent_control(&mut self, tick: u64) {
        let Some((channel, target)) = self.recent_index() else {
            return;
        };
        let Some((_, lane)) = self.current_track().filter(|&(c, _)| c == channel) else {
            return;
        };
        let label = self
            .song
            .automation_label(channel, target)
            .unwrap_or_default();
        if link(&mut self.song.channels[channel], lane, (target, label)) {
            self.song_edited(SongEdit::Final, tick);
        }
    }

    /// `Fine tune control`: the fine tune dialog opens on the recent control, if it supports it.
    fn fine_tune(&mut self) {
        let recent = self.recent_index();
        if let Some((channel, target)) =
            recent.filter(|&(c, t)| fine_tune::param(&self.song, c, t).is_some())
        {
            self.navigate(Nav::Open(Overlay::FineTune { channel, target }));
        }
    }

    /// The answer to `Delete automation?`, once it comes: the module goes (or the held edit is made) either way, and
    /// with `Delete` the lanes that target a module removed (but never a channel's last lane), in one undo step.
    /// Closing the message unanswered is `Keep`. A held edit is made only while its channel is as it was when
    /// asked: an undo under the question may have changed it.
    fn delete_answered(&mut self, tick: u64) {
        if self.delete_asked.is_none() {
            return;
        }
        let Some(delete) = self.nav.take_answer(DELETE_AUTOMATION_TAG) else {
            return;
        };
        let Some((id, pending)) = self.delete_asked.take() else {
            return;
        };
        let index = self.song.channels.iter().position(|c| c.id == id);
        let edit = match pending {
            Pending::Delete(instance_id) => index.and_then(|c| {
                let mut after = self.song.channels[c].clone();
                after.rack.remove(instance_id)?;
                Some((c, after, 0))
            }),
            Pending::Held(before, held) => index
                .filter(|&c| self.song.channels[c].same_edit(&before))
                .map(|c| (c, held.after, held.skipped)),
        };
        let Some((channel, after, skipped)) = edit else {
            return;
        };
        let before = std::mem::replace(&mut self.song.channels[channel], after);
        let ch = &mut self.song.channels[channel];
        let gone: Vec<i32> = (before.rack.modules.iter())
            .map(|m| m.instance_id)
            .filter(|&id| ch.rack.get(id).is_none())
            .collect();
        let targets = |l: &Lane| l.target.is_some_and(|t| gone.contains(&t.instance_id));
        let mut shifts = Vec::new();
        while delete
            && ch.lanes.len() > 1
            && let Some(lane) = ch.lanes.iter().position(targets)
        {
            ch.lanes.remove(lane);
            shifts.push(LaneShift::new(channel, lane, false));
        }
        self.rekey(&shifts);
        self.song_edited(SongEdit::Final, tick);
        if let Some(message) = preset_picker::skipped_message(skipped) {
            self.navigate(Nav::Open(message));
        }
    }
}
