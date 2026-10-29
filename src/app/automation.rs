//! Automation lanes from the panels' controls: the recent control,
//! the CTRL menu's `Add automation track`, `Add MIDI CC track…` and `Link to recent control`, the parameter menu's
//! `Add automation track`
//! (the MIDI CC items of both menus go on to `app/midi.rs`; `Fine tune control` opens the fine tune dialog), and the
//! `Delete automation?` question a card's `Delete` asks for a module that lanes target, as do
//! `Clear Rack`, a `Replace` and a rack preset's load, held back until the answer.
//!
//! The recent control is the channel (by id, which no move changes) and the [`AutomationTarget`] of the last control
//! bound to a parameter that was pressed: a rack card's module or common param, a mixer strip's (instance 0) or a drum
//! slot strip's (instance −slot id). It is held in memory and cleared with a new song.
//!
//! While the song plays, the controls show what the automation sets ([`HoneydewApp::show_automation`]).

use super::HoneydewApp;
use super::tracks::LaneShift;
use crate::audio_bridge::playing_clips;
use crate::audio_engine::strip_param;
use crate::midi::links;
use crate::song::{AutomationTarget, Channel, Clip, Event, Lane, LaneKind, PPQ, Song};
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

/// A param the screens show at the value its automation has set ([`HoneydewApp::show_automation`]): its channel's id
/// and target, the song tick and value of the automation event that set it, and the song's own value, which comes back
/// after the screens.
pub(super) struct Automated {
    channel: i32,
    target: AutomationTarget,
    at: u64,
    value: f32,
    saved: f32,
}

/// The end of the song ticks whose plays of `clip` have happened by song tick `tick`: none plays past its visible
/// length.
fn reach(clip: &Clip, tick: u64) -> u64 {
    let end = (f64::from(clip.start_tick) + clip.visible_ticks()).ceil() as u64;
    (tick + 1).min(end.saturating_add(1))
}

/// The last play of an automation event of `clip` at or before song tick `tick`, as (song tick, value): past the clip's
/// end, its last one. The loop's worth of plays before [`reach`] holds it.
fn last_play(clip: &Clip, tick: u64) -> Option<(u64, f32)> {
    let to = reach(clip, tick);
    let from = to.saturating_sub(clip.loop_ticks()?.ceil() as u64 + 1);
    let plays = clip.events_in_song_range(from.max(clip.start_tick.into()), to);
    let values = plays.filter_map(|(at, e)| match *e {
        Event::Automation { value, .. } => Some((at, value)),
        _ => None,
    });
    values.last()
}

/// The latest [`last_play`] of `lane`'s playing clips. The clip that reaches furthest goes first: a clip that ends before
/// what it played is not read.
fn lane_value(lane: &Lane, tick: u64) -> Option<(u64, f32)> {
    let first = playing_clips(lane).max_by_key(|c| reach(c, tick))?;
    let mut last = last_play(first, tick);
    for clip in playing_clips(lane) {
        let later = |at: u64| last.is_none_or(|(best, _)| at > best);
        if !std::ptr::eq(clip, first)
            && later(reach(clip, tick))
            && let Some(play) = last_play(clip, tick).filter(|p| later(p.0))
        {
            last = Some(play);
        }
    }
    last
}

/// The own value of the target of automation clip `key` while the screens show another (`automated`,
/// [`HoneydewApp::show_automation`]): the automation editor's init line.
pub(super) fn automated_init(automated: &[Automated], song: &Song, key: ClipKey) -> Option<f32> {
    let ch = song.channels.get(key.channel)?;
    let target = ch.lanes.get(key.lane)?.target?;
    let a = (automated.iter()).find(|a| (a.channel, a.target) == (ch.id, target))?;
    Some(a.saved)
}

impl HoneydewApp {
    /// The tempo the master's tempo automation has set by song tick `tick`: its last event played by then, `None` while
    /// none has (the song's tempo plays).
    pub(super) fn automated_tempo(&self, tick: u64) -> Option<f64> {
        let master = self.song.channels.first()?;
        let lanes = (master.lanes.iter()).filter(|l| l.target == Some(crate::song::TEMPO_TARGET));
        let (_, value) = lanes
            .filter_map(|l| lane_value(l, tick))
            .max_by_key(|&(at, _)| at)?;
        Some(crate::song::automated_tempo(value))
    }

    /// While the song plays, the controls of each automated param show the value its automation has set by the drawn
    /// playhead `tick`, as the engine plays it: the last event at or before it on the playing clips of the lanes that
    /// target it, past a clip's end its last (audio-engine.md, re-sync step 4). MIDI CC #7, #8 and #10 show on the
    /// Level and Pan they drive; a param no event has reached keeps its own value. The controls draw the song, so for
    /// the screens the song holds those values; [`Self::hide_automation`] puts its own back.
    pub(super) fn show_automation(&mut self, tick: u64) {
        let automated = &mut self.automated;
        for ch in &self.song.channels {
            for lane in (ch.lanes.iter()).filter(|l| l.kind == LaneKind::Automation) {
                let (Some(t), Some((at, value))) = (lane.target, lane_value(lane, tick)) else {
                    continue;
                };
                let target = AutomationTarget {
                    param: strip_param(t.instance_id, t.param),
                    ..t
                };
                match (automated.iter_mut()).find(|a| (a.channel, a.target) == (ch.id, target)) {
                    Some(a) if a.at > at => {}
                    Some(a) => (a.at, a.value) = (at, value),
                    None => automated.push(Automated {
                        channel: ch.id,
                        target,
                        at,
                        value,
                        saved: value,
                    }),
                }
            }
        }
        // A MIDI CC the song keeps no value for has no control to show it on. The recent control while its press lasts
        // shows its own value, under the finger: a drag writes back any other value it is drawn at.
        let held = self.recent.filter(|_| self.recent_held);
        let song = &mut self.song;
        automated.retain_mut(|a| {
            if held == Some((a.channel, a.target)) {
                return false;
            }
            let ch = song.channels.iter_mut().find(|c| c.id == a.channel);
            let Some(v) = ch.and_then(|c| c.param_mut(a.target)) else {
                return false;
            };
            a.saved = std::mem::replace(v, a.value);
            true
        });
    }

    /// After the screens: each param [`Self::show_automation`] showed gets the song's own value back, and so does an
    /// edit held for `Delete automation?`, which the screens made from the song as shown. A control moved meanwhile
    /// keeps the value it was moved to: moving a control sets its init value (automation.md §4b). `replaced`: the
    /// screens put another song in its place.
    pub(super) fn hide_automation(&mut self, replaced: bool) {
        let slots = &mut self.slots;
        let held = [
            &mut slots.rack.ask_remove,
            &mut slots.modules_browser.ask_remove,
            &mut slots.preset_picker.ask_remove,
        ];
        let held = held.into_iter().flatten().map(|h| &mut h.after);
        let channels = (self.song.channels.iter_mut()).chain(held);
        for ch in channels.filter(|_| !replaced) {
            let id = ch.id;
            for a in self.automated.iter().filter(|a| a.channel == id) {
                if let Some(v) = ch.param_mut(a.target).filter(|v| **v == a.value) {
                    *v = a.saved;
                }
            }
        }
        self.automated.clear();
    }

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
                self.recent_held |= touch.pick.is_none();
                pick = touch.pick.or(pick);
            }
        }
        // The tempo dialog's changes make the master's tempo the recent control: `Add automation track` automates it.
        if std::mem::take(&mut slots.tempo_editor.touched)
            && let Some(master) = self.song.channels.first()
        {
            self.recent = Some((master.id, crate::song::TEMPO_TARGET));
        }
        pick
    }

    /// After the frame's edit: the CTRL menu's pick, else a parameter menu's (`pick`), on the recent control, and the
    /// rack's `Delete` or a held edit (`Clear Rack`, a `Replace`, a rack preset's load) that must ask, then its answer.
    /// Each lane op and CC link change is one undo step. `tick` is the transport position.
    pub(super) fn automation(&mut self, pick: Option<CtrlAction>, tick: u64) {
        match self.slots.ctrl_menu.picked.take().or(pick) {
            Some(CtrlAction::AddAutomationTrack) => {
                if let Some((channel, target)) = self.recent_index() {
                    self.add_automation_track(channel, target, tick);
                }
            }
            Some(CtrlAction::AddCcTrack) => {
                self.navigate(Nav::Open(Overlay::CcPicker(self.selected_channel)))
            }
            Some(CtrlAction::LinkRecentControl) => self.link_recent_control(tick),
            Some(CtrlAction::LinkMidiCc) => self.learn_cc(),
            Some(CtrlAction::LinkModWheel) => self.link_cc(Some(1), tick),
            Some(CtrlAction::UnlinkMidiCc) => self.link_cc(None, tick),
            Some(CtrlAction::FineTune) => self.fine_tune(),
            None => {}
        }
        // The CC picker's pick: a lane of the CC's channel param.
        if let Some((channel, target)) = self.slots.cc_picker.picked.take()
            && channel < self.song.channels.len()
        {
            self.add_automation_track(channel, target, tick);
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

    /// `Add automation track` for `target` on `channel` (the recent control, or `Add MIDI CC track…`'s pick): a new lane
    /// with one empty clip at floor(song beat), b beats long; the lanes re-keyed, the channel selected and the
    /// automation editor opened on the clip.
    fn add_automation_track(&mut self, channel: usize, target: AutomationTarget, tick: u64) {
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
        self.selected_channel = channel;
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
