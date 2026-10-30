//! Automation lanes from the panels' controls: the recent control, the CTRL menu's `Add automation track` and
//! `Link to recent control`, the parameter menu's `Add automation track`, and `Delete automation?` when a module that
//! lanes target is deleted.

use eframe::egui::{Rect, Shape};

use super::*;
use crate::app::automation::{DELETE_AUTOMATION_TAG, recent_control};
use crate::song::{AutomationTarget, Event, LaneKind, PPQ};
use crate::ui::clips::ClipRef;
use crate::ui::ctrl_menu::RecentControl;
use crate::ui::drums::Ctl;

fn target(instance_id: i32, param: i32) -> AutomationTarget {
    AutomationTarget { instance_id, param }
}

/// The Distortion's Drive: param 1 of the effect track's first module (instance 2).
const DRIVE: AutomationTarget = AutomationTarget {
    instance_id: 2,
    param: 1,
};
/// Its Gain, param 3.
const GAIN: AutomationTarget = AutomationTarget {
    instance_id: 2,
    param: 3,
};

impl Rig {
    /// Where the last frame painted `text`.
    pub(super) fn text_rect(&self, text: &str) -> Rect {
        (self.out.shapes.iter())
            .find_map(|s| match &s.shape {
                Shape::Text(t) if t.galley.text() == text => Some(t.visual_bounding_rect()),
                _ => None,
            })
            .unwrap_or_else(|| panic!("{text:?} not painted"))
    }

    /// Presses `at` and lets go.
    fn touch_at(&mut self, at: Pos2) {
        self.click(at);
        self.frame(vec![]);
    }

    /// The recent control's channel index and target.
    fn recent(&self) -> Option<(usize, AutomationTarget)> {
        let (id, target) = self.app.recent?;
        let channel = self.app.song.channels.iter().position(|c| c.id == id)?;
        Some((channel, target))
    }

    /// The undo steps there are (undone, then redone back).
    fn undos(&mut self) -> usize {
        let history = &mut self.app.history;
        let n = std::iter::from_fn(|| history.undo()).count();
        for _ in 0..n {
            history.redo();
        }
        n
    }

    /// Picks `item` from the open CTRL menu (its first frame only sizes it).
    pub(super) fn ctrl_pick(&mut self, item: &str) {
        self.click(CTRL);
        self.frame(vec![]);
        self.frame(vec![]);
        let at = self.text_rect(item).center();
        self.click(at);
        self.frame(vec![]);
    }
}

/// [`Rig::with_synth`] plus an effect track (2) holding a Distortion (instance 2), its rack open, with an empty log.
pub(super) fn effects_rig() -> Rig {
    let mut rig = Rig::with_synth();
    rig.app.add_channel(ChannelKind::EffectTrack, 0);
    let rack = &mut rig.app.song.channels[2].rack;
    crate::ui::modules_browser::add_module(rack, 6, InsertAt::End);
    assert_eq!(rack.modules[0].instance_id, 2);
    rig.app.song_edited(SongEdit::Final, 0);
    rig.open_rack(2);
    rig.slide_rack();
    rig.take();
    rig
}

// ---- the pure rules ----

#[test]
fn the_ctrl_menu_knows_the_recent_controls_channel_and_the_current_tracks_target() {
    let mut song = Song::default();
    song.add_channel(ChannelKind::Synth(SynthKind::MiniSynth));
    song.add_channel(ChannelKind::EffectTrack);
    crate::ui::modules_browser::add_module(&mut song.channels[2].rack, 6, InsertAt::End);
    song.channels[2].add_automation_lane(DRIVE);
    let lane = song.channels[2].lanes.len() - 1;
    let state = |recent, track| recent_control(&song, recent, track);
    // The current track automates another param of the recent control's channel: everything holds. Gain is continuous:
    // the fine tune dialog supports it.
    let all = RecentControl {
        on_current_channel: true,
        track_automates_other: true,
        fine_tune: true,
        ..RecentControl::default()
    };
    assert_eq!(state((2, GAIN), Some((2, lane))), all);
    // The same param: nothing to retarget to.
    let same = state((2, DRIVE), Some((2, lane)));
    assert!(same.on_current_channel && !same.track_automates_other);
    // Another channel's track, a track that is no automation track, no track at all.
    assert!(!state((1, target(0, 0)), Some((2, lane))).on_current_channel);
    let notes = state((1, target(0, 0)), Some((1, 0)));
    assert!(notes.on_current_channel && !notes.track_automates_other);
    let tunable = RecentControl {
        fine_tune: true,
        ..RecentControl::default()
    };
    assert_eq!(state((2, GAIN), None), tunable);
    // A channel MIDI-CC pseudo-parameter: channel params 4…133.
    for (param, cc) in [(3, false), (4, true), (133, true), (134, false)] {
        let cc_param = state((1, target(0, param)), None).channel_cc;
        assert_eq!(cc_param, cc, "{param}");
    }
}

// ---- the recent control ----

#[test]
fn the_recent_control_follows_presses_on_rack_mixer_and_drum_strip_controls() {
    let mut rig = effects_rig();
    assert_eq!(rig.recent(), None);
    // A rack knob: the module's instance and param.
    let drive = rig.text_rect("Drive").center();
    rig.touch_at(drive);
    assert_eq!(rig.recent(), Some((2, DRIVE)));
    // The mixer strip: instance 0. Channel 1's fader: its strip spans x 256..384, after the send's, as the strips
    // take the rack order (master, sends, the rest).
    rig.app.navigate(Nav::Show(View::Mixer));
    rig.frame(vec![]);
    rig.touch_at(pos2(320.0, 100.0));
    assert_eq!(rig.recent(), Some((1, target(0, 0))));
    // A drum slot strip: instance −slot id.
    rig.app.add_channel(ChannelKind::Drums, 0);
    let drums = rig.app.song.channels.len() - 1;
    let kit = crate::song::DrumKit::default_kit();
    let slot = kit.slots[1].file.id;
    rig.app.song.channels[drums].drum_kit = Some(kit);
    rig.app.song_edited(SongEdit::Final, 0);
    rig.edit(drums);
    rig.slide_split();
    let row = rig.app.slots.drum_editor.step.selected;
    let header = egui::Id::new("step_seq").with(("row", row));
    rig.click_id(header);
    rig.frame(vec![]);
    rig.frame(vec![]);
    let level = egui::Id::new(("drum_editor", drums))
        .with("drum strips")
        .with((1_usize, Ctl::Level));
    rig.click_id(level);
    assert_eq!(rig.recent(), Some((drums, target(-(slot as i32), 0))));
    // Held in memory: a new song clears it.
    rig.app.song = Song::default();
    rig.app.song_edited(SongEdit::Replaced, 0);
    assert_eq!(rig.app.recent, None);
}

#[test]
fn the_ctrl_menu_enables_add_automation_track_with_a_recent_control_and_the_link_on_another_param()
{
    let mut rig = effects_rig();
    let dim = crate::theme::theme_by_id(0)
        .colors
        .background
        .to_color32()
        .gamma_multiply(0.3);
    let enabled = |rig: &mut Rig| -> Vec<&'static str> {
        rig.click(CTRL);
        rig.frame(vec![]);
        rig.frame(vec![]);
        let items = [
            "Link to MIDI CC",
            "Link to Mod Wheel",
            "Unlink from MIDI CC",
            "Add automation track",
            "Link to recent control",
            "Fine tune control",
        ];
        let on = items
            .into_iter()
            .filter(|i| rig.color_of(i) != Some(dim))
            .collect();
        rig.frame(escape());
        rig.frame(vec![]);
        on
    };
    assert_eq!(enabled(&mut rig), Vec::<&str>::new());
    rig.open_rack(2);
    rig.slide_rack();
    let drive = rig.text_rect("Drive").center();
    rig.touch_at(drive);
    // Drive is continuous: the fine tune dialog supports it.
    assert_eq!(
        enabled(&mut rig),
        [
            "Link to MIDI CC",
            "Link to Mod Wheel",
            "Add automation track",
            "Fine tune control"
        ]
    );
    // The playlist's selected clip is on an automation track of Gain: Drive can be linked to it.
    let lane = rig.app.song.channels[2].add_automation_lane(GAIN);
    let id = lane.add_clip(0, 4.0).id;
    let lane = rig.app.song.channels[2].lanes.len() - 1;
    rig.app.main_screen.clips.selected = Some(ClipRef {
        channel: 2,
        lane,
        id,
    });
    assert_eq!(
        enabled(&mut rig),
        [
            "Link to MIDI CC",
            "Link to Mod Wheel",
            "Add automation track",
            "Link to recent control",
            "Fine tune control"
        ]
    );
    // Once Drive has a CC link (on its own channel), it can be unlinked.
    rig.ctrl_pick("Link to Mod Wheel");
    assert_eq!(
        enabled(&mut rig),
        [
            "Link to MIDI CC",
            "Link to Mod Wheel",
            "Unlink from MIDI CC",
            "Add automation track",
            "Link to recent control",
            "Fine tune control"
        ]
    );
}

// ---- `Add automation track` ----

#[test]
fn add_automation_track_adds_an_empty_lane_at_the_song_beat_and_opens_its_editor_in_one_undo_step()
{
    let mut rig = effects_rig();
    let drive = rig.text_rect("Drive").center();
    rig.touch_at(drive);
    // The playhead at beat 5¾ of a 3-beat bar (TDIV byte 1): the clip starts at beat 5 and loops every 3 beats.
    rig.app.song.time_sig = crate::song::TimeSig {
        subdivision: 4,
        beats_per_bar: 3,
    };
    rig.rec.0.borrow_mut().tick = 5 * u64::from(PPQ) + 96;
    rig.frame(vec![]);
    let (undos, lanes) = (rig.undos(), rig.app.song.channels[2].lanes.len());
    // Another channel is selected: the owning channel takes its place.
    rig.app.selected_channel = 1;
    rig.ctrl_pick("Add automation track");
    let ch = &rig.app.song.channels[2];
    assert_eq!(ch.lanes.len(), lanes + 1);
    let lane = (ch.lanes.iter())
        .position(|l| l.target == Some(DRIVE))
        .expect("a Drive lane");
    let l = &ch.lanes[lane];
    assert_eq!(
        (l.kind, l.label.as_str()),
        (LaneKind::Automation, "Drive : Distortion")
    );
    let [clip] = &l.clips[..] else {
        panic!("{:?}", l.clips)
    };
    assert_eq!((clip.start_tick, clip.loop_len_beats), (5 * PPQ, 3.0));
    assert!(clip.events.is_empty(), "the param stays at its init value");
    // The channel is selected and its automation editor is open on the clip.
    assert_eq!(rig.app.nav.view, View::AutomationEditor(2));
    let key = rig.app.slots.automation_editor.current(&rig.app.song, 2);
    assert_eq!(key.map(|k| (k.lane, k.id)), Some((lane, clip.id)));
    assert_eq!(rig.app.selected_channel, 2);
    assert_eq!(rig.undos(), undos + 1);
    // No duplicate check: a second one adds another lane.
    rig.ctrl_pick("Add automation track");
    let drives = (rig.app.song.channels[2].lanes.iter()).filter(|l| l.target == Some(DRIVE));
    assert_eq!(drives.count(), 2);
}

#[test]
fn add_automation_track_on_the_master_strip_selects_the_master() {
    let mut rig = effects_rig();
    // The master's strip, left of channel 1's (x 0..128): its Level, instance 0.
    rig.app.navigate(Nav::Show(View::Mixer));
    rig.frame(vec![]);
    rig.touch_at(pos2(64.0, 100.0));
    assert_eq!(rig.recent(), Some((0, target(0, 0))));
    rig.app.selected_channel = 1;
    let lanes = rig.app.song.channels[0].lanes.len();
    rig.ctrl_pick("Add automation track");
    assert_eq!(rig.app.song.channels[0].lanes.len(), lanes + 1);
    assert_eq!(rig.app.nav.view, View::AutomationEditor(0));
    // The new lane is current, and so is its channel.
    assert_eq!(rig.app.selected_channel, 0);
}

#[test]
fn a_knobs_add_automation_track_opens_the_automation_editor_on_a_new_lane_named_param_colon_module()
{
    // The acceptance: long-press a knob, pick `Add automation track`.
    let mut rig = effects_rig();
    let gain = rig.text_rect("Gain").center();
    rig.secondary_click(gain);
    rig.frame(vec![]);
    let undos = rig.undos();
    let item = rig.text_rect("Add automation track").center();
    rig.click(item);
    rig.frame(vec![]);
    assert_eq!(rig.recent(), Some((2, GAIN)));
    assert_eq!(rig.app.nav.view, View::AutomationEditor(2));
    let key = (rig.app.slots.automation_editor)
        .current(&rig.app.song, 2)
        .expect("the editor's clip");
    let lane = &rig.app.song.channels[2].lanes[key.lane];
    assert_eq!(lane.label, "Gain : Distortion");
    assert_eq!(lane.target, Some(GAIN));
    assert!(key.clip(&rig.app.song).unwrap().events.is_empty());
    assert_eq!(rig.undos(), undos + 1);
    assert_eq!(rig.app.nav.overlays, [], "the rack closed");
}

#[test]
fn add_midi_cc_track_picks_a_cc_by_name_for_a_lane_on_the_selected_channel_in_one_undo_step() {
    let mut rig = Rig::with_synth();
    let (undos, lanes) = (rig.undos(), rig.app.song.channels[1].lanes.len());
    rig.ctrl_pick("Add MIDI CC track…");
    // The picker, past its slide-in: search, then the one match.
    for _ in 0..3 {
        rig.frame(vec![]);
    }
    rig.click_id(egui::Id::new("cc_picker").with("search"));
    rig.frame(vec![egui::Event::Text("bright".into())]);
    rig.frame(vec![]);
    rig.click_id(egui::Id::new("cc_picker").with(("row", 0_usize)));
    rig.frame(vec![]);
    let ch = &rig.app.song.channels[1];
    assert_eq!(ch.lanes.len(), lanes + 1);
    let cc_74 = Some(target(0, 81));
    let lane = ch.lanes.iter().position(|l| l.target == cc_74).unwrap();
    assert_eq!(ch.lanes[lane].label, "CC 74 Brightness");
    assert_eq!(ch.lanes[lane].clips.len(), 1);
    assert_eq!(rig.app.nav.view, View::AutomationEditor(1));
    assert_eq!(rig.undos(), undos + 1);
    // A plain automation lane in the file: it saves and reopens as it is.
    let read = crate::flm::read(&crate::flm::write(&rig.app.song).unwrap()).unwrap();
    let l = &read.channels[1].lanes[lane];
    assert_eq!((l.target, l.label.as_str()), (cc_74, "CC 74 Brightness"));
    rig.frame(z(egui::Modifiers::NONE));
    assert_eq!(rig.app.song.channels[1].lanes.len(), lanes);
}

#[test]
fn a_send_lane_is_named_after_its_target_channel() {
    // `Send Level (to <name>)`: the name of the channel the send goes to.
    let mut rig = effects_rig();
    let to = rig.app.song.channels[2].id;
    let send = crate::song::SendState {
        target_channel_id: to,
        ..crate::song::SendState::default()
    };
    rig.app.song.channels[1].file.sends.push(send);
    rig.app.song_edited(SongEdit::Final, 0);
    let id = rig.app.song.channels[1].id;
    rig.app.recent = Some((id, target(0, 0x2000 + to)));
    rig.frame(vec![]);
    rig.ctrl_pick("Add automation track");
    let lanes = &rig.app.song.channels[1].lanes;
    let lane = lanes
        .iter()
        .find(|l| l.target == Some(target(0, 0x2000 + to)));
    assert_eq!(lane.unwrap().label, "Send Level (to Effect 2)");
    // `Link to recent control` names a retargeted lane the same way.
    let lane = lanes.len() - 1;
    rig.app.recent = Some((id, target(0, 0x1000 + to)));
    rig.app.main_screen.clips.selected = Some(ClipRef {
        channel: 1,
        lane,
        id: rig.app.song.channels[1].lanes[lane].clips[0].id,
    });
    rig.app.navigate(Nav::Show(View::Playlist));
    rig.frame(vec![]);
    rig.ctrl_pick("Link to recent control");
    let l = &rig.app.song.channels[1].lanes[lane];
    assert_eq!(l.target, Some(target(0, 0x1000 + to)));
    assert_eq!(l.label, "Enable Send (to Effect 2)");
}

#[test]
fn a_lane_added_before_others_moves_what_is_kept_on_them() {
    // The Drive lane sorts before a Gain lane already there: the playlist's selected clip on the Gain lane follows it.
    let mut rig = effects_rig();
    let gain = rig.app.song.channels[2].add_automation_lane(GAIN);
    let id = gain.add_clip(0, 4.0).id;
    let lane = rig.app.song.channels[2].lanes.len() - 1;
    rig.app.song_edited(SongEdit::Final, 0);
    let selected = ClipRef {
        channel: 2,
        lane,
        id,
    };
    rig.app.main_screen.clips.selected = Some(selected);
    rig.open_rack(2);
    rig.slide_rack();
    let drive = rig.text_rect("Drive").center();
    rig.touch_at(drive);
    rig.ctrl_pick("Add automation track");
    let lanes = &rig.app.song.channels[2].lanes;
    assert_eq!(
        lanes[lane].target,
        Some(DRIVE),
        "inserted in the Gain lane's place"
    );
    let moved = ClipRef {
        lane: lane + 1,
        ..selected
    };
    assert_eq!(rig.app.main_screen.clips.selected, Some(moved));
    assert_eq!(lanes[lane + 1].target, Some(GAIN));
}

// ---- `Link to recent control` ----

/// The payloads of the `EVN2` chunks `song` is written with (an 8-byte header: the tag, then the u32 size).
fn evn2(song: &Song) -> Vec<Vec<u8>> {
    let bytes = crate::flm::write(song).unwrap();
    (0..bytes.len().saturating_sub(8))
        .filter(|&i| &bytes[i..i + 4] == b"EVN2")
        .map(|i| {
            let size = u32::from_le_bytes(bytes[i + 4..i + 8].try_into().unwrap()) as usize;
            bytes[i + 8..i + 8 + size].to_vec()
        })
        .collect()
}

/// The keys of every record `song` is written with.
fn written_keys(song: &Song) -> Vec<i16> {
    let keys = |payload: &[u8]| -> Vec<i16> {
        let size = usize::from(u16::from_le_bytes([payload[0], payload[1]]));
        (payload[2..].chunks_exact(size))
            .map(|r| i16::from_le_bytes([r[12], r[13]]))
            .collect()
    };
    evn2(song).iter().flat_map(|p| keys(p)).collect()
}

#[test]
fn link_to_recent_control_retargets_the_lane_keeps_its_points_and_rewrites_their_keys() {
    let mut rig = effects_rig();
    // A Gain lane with two points, named by the user, read from a file (its raw EVN2 kept).
    let gain = rig.app.song.channels[2].add_automation_lane(GAIN);
    gain.file.custom_label = "Mine".into();
    let clip = gain.add_clip(0, 4.0);
    clip.events = vec![
        Event::Automation {
            pos: 0,
            value: 0.0,
            control: true,
        },
        Event::Automation {
            pos: 256,
            value: 1.0,
            control: true,
        },
    ];
    let (id, events) = (clip.id, clip.events.clone());
    let lane = rig.app.song.channels[2].lanes.len() - 1;
    // Gain is param 3: key 4. The bytes kept as a file had them (a non-canonical EVN2 is kept raw).
    assert_eq!(written_keys(&rig.app.song), [4, 4]);
    let [raw] = &evn2(&rig.app.song)
        .into_iter()
        .filter(|p| p.len() > 2)
        .collect::<Vec<_>>()[..]
    else {
        panic!("one clip with events")
    };
    rig.app.song.channels[2].lanes[lane].clips[0]
        .file
        .raw_events = crate::song::Raw(Some(raw.clone()));
    rig.app.song_edited(SongEdit::Final, 0);
    // Its clip is the playlist's selection; Drive is pressed.
    rig.app.main_screen.clips.selected = Some(ClipRef {
        channel: 2,
        lane,
        id,
    });
    rig.open_rack(2);
    rig.slide_rack();
    let drive = rig.text_rect("Drive").center();
    rig.touch_at(drive);
    let undos = rig.undos();
    rig.ctrl_pick("Link to recent control");
    let l = &rig.app.song.channels[2].lanes[lane];
    assert_eq!(l.target, Some(DRIVE));
    assert_eq!(l.label, "Drive : Distortion");
    assert_eq!(l.file.custom_label, "", "the custom name is cleared");
    assert_eq!(l.clips[0].events, events, "the points stay");
    assert_eq!(rig.undos(), undos + 1);
    // Every point's key is Drive's now, param 1 + 1: the stale kept bytes are not written back.
    assert_eq!(written_keys(&rig.app.song), [2, 2]);
}

#[test]
fn modulate_recent_control_sets_a_modulators_target_in_one_undo_step() {
    let mut rig = effects_rig();
    let ch = &mut rig.app.song.channels[2];
    let modulator = ch.rack.add(honeydew_audio::modulator::MODULATOR);
    let id = ch.id;
    rig.app.song_edited(SongEdit::Final, 0);
    rig.frame(vec![]);
    let drive = rig.text_rect("Drive").center();
    rig.touch_at(drive);
    let undos = rig.undos();
    card_item(&mut rig, "Modulator", "Modulate recent control");
    rig.frame(vec![]);
    let set = vec![crate::song::Modulation {
        modulator,
        channel: id,
        target: DRIVE,
    }];
    assert_eq!(rig.app.song.channels[2].file.modulations, set);
    assert_eq!(rig.undos(), undos + 1);
    // Undone, the Modulator drives nothing again.
    let ctx = rig.ctx.clone();
    rig.app.step(&ctx, crate::history::Step::Undo, 0);
    assert_eq!(rig.app.song.channels[2].file.modulations, []);
}

// ---- `Delete automation?` ----

/// [`effects_rig`] with a Drive lane (one clip) and a Chorus (instance 3) without lanes; its rack open.
fn automated_rig() -> Rig {
    let mut rig = effects_rig();
    let ch = &mut rig.app.song.channels[2];
    crate::ui::modules_browser::add_module(&mut ch.rack, 11, InsertAt::End);
    ch.add_automation_lane(DRIVE).add_clip(0, 4.0);
    rig.app.song_edited(SongEdit::Final, 0);
    rig.frame(vec![]);
    rig
}

/// Picks `item` from the ≡ menu of the card titled `title`.
fn card_item(rig: &mut Rig, title: &str, item: &str) {
    let name = rig.text_rect(title);
    // The ≡ is the header row's last cell, 0.75 U in from the card's right edge: the rack's (1170) less its 1.5 U gutter.
    rig.click(pos2(1170.0 - 2.25 * U, name.center().y));
    rig.frame(vec![]);
    let at = rig.text_rect(item).center();
    rig.click(at);
}

/// Picks `Delete` from the ≡ menu of the card titled `title`.
fn delete_card(rig: &mut Rig, title: &str) {
    card_item(rig, title, "Delete");
    // A message's first frame only sizes it.
    rig.frame(vec![]);
    rig.frame(vec![]);
}

/// Picks `item` from the ≡ menu of channel 2's open rack.
fn rack_item(rig: &mut Rig, item: &str) {
    rig.click_id(egui::Id::new(("rack", 2_usize)).with("rack menu button"));
    rig.frame(vec![]);
    let at = rig.text_rect(item).center();
    rig.click(at);
}

/// Answers the open message with `button`.
fn answer(rig: &mut Rig, button: &str) {
    // A message's first frame only sizes it.
    rig.frame(vec![]);
    rig.frame(vec![]);
    let at = rig.text_rect(button).center();
    rig.click(at);
    rig.frame(vec![]);
}

fn asking(rig: &Rig) -> Option<&'static str> {
    rig.app.nav.overlays.iter().find_map(|o| match o {
        Overlay::Message { tag, .. } => Some(*tag),
        _ => None,
    })
}

fn modules(rig: &Rig) -> Vec<u16> {
    let rack = &rig.app.song.channels[2].rack;
    rack.modules.iter().map(|m| m.type_id).collect()
}

fn drive_lanes(rig: &Rig) -> usize {
    let lanes = rig.app.song.channels[2].lanes.iter();
    lanes.filter(|l| l.target == Some(DRIVE)).count()
}

#[test]
fn deleting_a_module_without_lanes_asks_nothing() {
    let mut rig = automated_rig();
    delete_card(&mut rig, "Chorus");
    assert_eq!(asking(&rig), None);
    assert_eq!(modules(&rig), [6]);
}

#[test]
fn deleting_an_automated_module_asks_and_delete_removes_its_lanes_in_one_undo_step() {
    let mut rig = automated_rig();
    let undos = rig.undos();
    delete_card(&mut rig, "Distortion");
    assert_eq!(asking(&rig), Some(DELETE_AUTOMATION_TAG));
    assert!(rig.painted("Delete automation?"));
    assert_eq!(modules(&rig), [6, 11], "kept until the answer");
    let at = rig.text_rect("Delete").center();
    rig.click(at);
    rig.frame(vec![]);
    assert_eq!(asking(&rig), None);
    assert_eq!(modules(&rig), [11]);
    assert_eq!(drive_lanes(&rig), 0);
    assert_eq!(rig.undos(), undos + 1);
}

#[test]
fn keep_deletes_the_module_and_keeps_its_lanes_with_no_undo_step() {
    let mut rig = automated_rig();
    let undos = rig.undos();
    delete_card(&mut rig, "Distortion");
    let at = rig.text_rect("Keep").center();
    rig.click(at);
    rig.frame(vec![]);
    assert_eq!(asking(&rig), None);
    assert_eq!(modules(&rig), [11]);
    assert_eq!(drive_lanes(&rig), 1, "their target then plays nothing");
    // The rack is not covered by undo, and no lane changed.
    assert_eq!(rig.undos(), undos);
}

#[test]
fn escape_on_the_question_deletes_the_module_and_keeps_its_lanes_with_no_undo_step() {
    let mut rig = automated_rig();
    let undos = rig.undos();
    delete_card(&mut rig, "Distortion");
    assert_eq!(asking(&rig), Some(DELETE_AUTOMATION_TAG));
    rig.frame(escape());
    rig.frame(vec![]);
    assert_eq!(asking(&rig), None);
    assert_eq!(modules(&rig), [11]);
    assert_eq!(drive_lanes(&rig), 1);
    assert_eq!(rig.undos(), undos);
}

#[test]
fn delete_keeps_the_playlist_selection_on_a_later_lane() {
    // A Chorus lane after the Drive lane, its clip selected: it moves up a lane when the Drive lane goes.
    let mut rig = automated_rig();
    let chorus = rig.app.song.channels[2].add_automation_lane(target(3, 1));
    let id = chorus.add_clip(0, 4.0).id;
    rig.app.song_edited(SongEdit::Final, 0);
    let lanes = &rig.app.song.channels[2].lanes;
    let drive = lanes.iter().position(|l| l.target == Some(DRIVE)).unwrap();
    let lane = (lanes.iter())
        .position(|l| l.target == Some(target(3, 1)))
        .unwrap();
    assert!(lane > drive);
    let selected = ClipRef {
        channel: 2,
        lane,
        id,
    };
    rig.app.main_screen.clips.selected = Some(selected);
    rig.frame(vec![]);
    delete_card(&mut rig, "Distortion");
    let at = rig.text_rect("Delete").center();
    rig.click(at);
    rig.frame(vec![]);
    assert_eq!(drive_lanes(&rig), 0);
    let moved = ClipRef {
        lane: lane - 1,
        ..selected
    };
    assert_eq!(rig.app.main_screen.clips.selected, Some(moved));
    let l = &rig.app.song.channels[2].lanes[lane - 1];
    assert_eq!(l.target, Some(target(3, 1)));
}

#[test]
fn delete_follows_the_automation_editor_off_a_removed_lane() {
    let mut rig = automated_rig();
    let lane = (rig.app.song.channels[2].lanes.iter())
        .position(|l| l.target == Some(DRIVE))
        .unwrap();
    let id = rig.app.song.channels[2].lanes[lane].clips[0].id;
    rig.app.navigate(Nav::EditClip(ClipKey {
        channel: 2,
        lane,
        id,
    }));
    rig.slide_split();
    assert_eq!(rig.app.nav.view, View::AutomationEditor(2));
    rig.app.navigate(Nav::Open(Overlay::Rack(2)));
    rig.slide_rack();
    delete_card(&mut rig, "Distortion");
    let at = rig.text_rect("Delete").center();
    rig.click(at);
    rig.frame(vec![]);
    assert_eq!(rig.app.nav.view, View::Playlist);
}

// ---- `Delete automation?` for the other edits that remove modules ----

#[test]
fn replace_asks_and_delete_removes_the_replaced_modules_lanes_in_one_undo_step() {
    let mut rig = automated_rig();
    let undos = rig.undos();
    card_item(&mut rig, "Distortion", "Replace");
    // The Modules browser, past its slide-in: search, then the first match.
    for _ in 0..3 {
        rig.frame(vec![]);
    }
    rig.click_id(egui::Id::new("modules_browser").with("search"));
    rig.frame(vec![egui::Event::Text("Tape D".into())]);
    rig.frame(vec![]);
    rig.click_id(egui::Id::new("modules_browser").with(("row", 1_usize)));
    rig.frame(vec![]);
    assert_eq!(asking(&rig), Some(DELETE_AUTOMATION_TAG));
    assert_eq!(modules(&rig), [6, 11], "kept until the answer");
    answer(&mut rig, "Delete");
    assert_eq!(asking(&rig), None);
    assert_eq!(modules(&rig), [18, 11]);
    assert_eq!(drive_lanes(&rig), 0);
    assert_eq!(rig.undos(), undos + 1);
}

#[test]
fn clear_rack_asks_once_for_every_automated_module_and_delete_removes_their_lanes() {
    let mut rig = automated_rig();
    let chorus = target(3, 1);
    rig.app.song.channels[2]
        .add_automation_lane(chorus)
        .add_clip(0, 4.0);
    rig.app.song_edited(SongEdit::Final, 0);
    let undos = rig.undos();
    rack_item(&mut rig, "Clear Rack");
    assert_eq!(asking(&rig), Some(DELETE_AUTOMATION_TAG));
    assert_eq!(modules(&rig), [6, 11], "kept until the answer");
    answer(&mut rig, "Delete");
    assert_eq!(asking(&rig), None, "asked once");
    assert_eq!(modules(&rig), Vec::<u16>::new());
    let lanes = &rig.app.song.channels[2].lanes;
    assert!(
        lanes
            .iter()
            .all(|l| l.target != Some(DRIVE) && l.target != Some(chorus))
    );
    assert_eq!(rig.undos(), undos + 1);
}

/// Loads a rack preset of module types `types` into channel 2's open rack with its ≡ `Load Rack preset`: a file kept
/// this session in `My Racks`, through a web provider.
fn load_rack_preset(rig: &mut Rig, types: &[u16]) {
    let mem = MemFiles::default();
    mem.0.borrow_mut().names_only = true;
    rig.app.files = ProjectFiles::with_provider(Box::new(mem));
    let modules: Vec<_> = (types.iter())
        .map(|&t| crate::song::ModuleInstance::new(t, 0))
        .collect();
    let bytes = crate::flm::write_rack(&modules).unwrap();
    rig.app
        .files
        .keep_user_file(std::path::Path::new("My Racks/Mine.flmrack"), bytes);
    rack_item(rig, "Load Rack preset");
    // `My Files`, then the preset.
    super::rack_wiring::picker_row(rig, 0);
    super::rack_wiring::picker_row(rig, 0);
    rig.frame(vec![]);
}

#[test]
fn a_rack_preset_load_asks_and_keep_loads_it_and_keeps_the_lanes_with_no_undo_step() {
    let mut rig = automated_rig();
    let undos = rig.undos();
    load_rack_preset(&mut rig, &[10]);
    assert_eq!(asking(&rig), Some(DELETE_AUTOMATION_TAG));
    assert_eq!(modules(&rig), [6, 11], "kept until the answer");
    answer(&mut rig, "Keep");
    assert_eq!(asking(&rig), None);
    assert_eq!(modules(&rig), [10]);
    assert_eq!(drive_lanes(&rig), 1);
    assert_eq!(rig.undos(), undos);
}

#[test]
fn a_held_edit_is_dropped_when_an_undo_under_the_question_changed_its_channel() {
    let mut rig = automated_rig();
    rack_item(&mut rig, "Clear Rack");
    assert_eq!(asking(&rig), Some(DELETE_AUTOMATION_TAG));
    // The undo takes back the Drive lane; the Chorus, added in that edit, stays (the rack is not covered by undo).
    rig.undo_under_modal();
    assert_eq!((modules(&rig), drive_lanes(&rig)), (vec![6, 11], 0));
    answer(&mut rig, "Delete");
    assert_eq!(asking(&rig), None);
    assert_eq!(
        modules(&rig),
        [6, 11],
        "the cleared rack it held is not put back"
    );
}

#[test]
fn delete_leaves_a_lane_that_already_targeted_nothing() {
    // A lane on instance 99, which no module has: `Delete` removes the cleared Distortion's lane, not that one.
    let mut rig = automated_rig();
    let ch = &mut rig.app.song.channels[2];
    ch.add_automation_lane(target(99, 1)).add_clip(0, 4.0);
    rig.app.song_edited(SongEdit::Final, 0);
    rack_item(&mut rig, "Clear Rack");
    answer(&mut rig, "Delete");
    assert_eq!(drive_lanes(&rig), 0);
    let lanes = rig.app.song.channels[2].lanes.iter();
    let targets: Vec<_> = lanes.map(|l| l.target).collect();
    assert_eq!(targets, [Some(target(0, 0)), Some(target(99, 1))]);
}

#[test]
fn a_rack_whose_lanes_target_nothing_it_removes_asks_nothing() {
    let mut rig = effects_rig();
    let ch = &mut rig.app.song.channels[2];
    ch.add_automation_lane(target(99, 1)).add_clip(0, 4.0);
    rig.app.song_edited(SongEdit::Final, 0);
    rig.frame(vec![]);
    let undos = rig.undos();
    rack_item(&mut rig, "Clear Rack");
    rig.frame(vec![]);
    rig.frame(vec![]);
    assert_eq!(asking(&rig), None);
    assert_eq!(modules(&rig), Vec::<u16>::new());
    assert_eq!(rig.undos(), undos, "the rack");
}

#[test]
fn a_held_rack_preset_load_shows_the_modules_it_skipped_once_answered() {
    // A MiniSynth, which an effect track does not take, and a Chorus.
    let mut rig = automated_rig();
    load_rack_preset(&mut rig, &[0, 10]);
    assert_eq!(asking(&rig), Some(DELETE_AUTOMATION_TAG));
    answer(&mut rig, "Keep");
    assert_eq!(modules(&rig), [10]);
    let skipped = rig.app.nav.overlays.iter().find_map(|o| match o {
        Overlay::Message { tag, lines, .. } if *tag == crate::ui::views::preset_picker::TAG => {
            Some(lines.clone())
        }
        _ => None,
    });
    assert_eq!(skipped, Some(vec!["Skipped 1 incompatible module".into()]));
}

// ---- the fine tune dialog ----

/// The fine tune dialog open on the top of the overlays, if it is.
fn fine_tuning(rig: &Rig) -> Option<(usize, AutomationTarget)> {
    match rig.app.nav.overlays.last()? {
        Overlay::FineTune { channel, target } => Some((*channel, *target)),
        _ => None,
    }
}

/// The dialog's slider row on the 1170 × 540 screen (U = 32): y 259…313, its track from x 80 to 1090.
const TUNE_Y: f32 = 286.0;
fn tune_x(v: f32) -> f32 {
    112.0 + v * 946.0
}

/// Drags the dialog's thumb from value `from` by `dv` of the track, over a few frames, and lets go.
fn tune_drag(rig: &mut Rig, from: f32, dv: f32) {
    let start = pos2(tune_x(from), TUNE_Y);
    rig.press(start);
    for f in [0.25, 0.5, 1.0] {
        rig.drag_to(pos2(tune_x(from + f * dv), TUNE_Y));
    }
    rig.release(pos2(tune_x(from + dv), TUNE_Y));
    rig.frame(vec![]);
}

#[test]
fn fine_tune_control_follows_the_dialogs_support_over_the_three_sources() {
    let mut song = Song::default();
    song.add_channel(ChannelKind::EffectTrack);
    song.add_channel(ChannelKind::Drums);
    crate::ui::modules_browser::add_module(&mut song.channels[1].rack, 6, InsertAt::End);
    let slot = -(song.channels[2].drum_kit.as_ref().unwrap().slots[0].file.id as i32);
    let tunable = |channel, t| recent_control(&song, (channel, t), None).fine_tune;
    // The channel: Level, Pan, a send level; not Mute, Solo, a CC param or Enable Send.
    assert!(tunable(1, target(0, 0)) && tunable(1, target(0, 1)));
    assert!(tunable(1, target(0, 0x2000)), "its send to the master");
    for p in [2, 3, 10, 0x1000] {
        assert!(!tunable(1, target(0, p)), "{p}");
    }
    // A drum pad: Level, Sample Start; not Mute or Solo.
    assert!(tunable(2, target(slot, 0)) && tunable(2, target(slot, 10)));
    assert!(!tunable(2, target(slot, 3)) && !tunable(2, target(slot, 4)));
    // The Distortion: Drive and the common Mix, not Enable or the Kind menu.
    assert!(tunable(1, DRIVE) && tunable(1, target(2, AutomationTarget::MIX)));
    assert!(!tunable(1, target(2, 0)) && !tunable(1, target(2, 5)));
}

#[test]
fn the_ctrl_menus_fine_tune_control_opens_the_dialog_on_the_recent_control() {
    let mut rig = effects_rig();
    let drive = rig.text_rect("Drive").center();
    rig.touch_at(drive);
    rig.ctrl_pick("Fine tune control");
    assert_eq!(fine_tuning(&rig), Some((2, DRIVE)));
    rig.frame(vec![]);
    let name = rig.app.song.channels[2].name.clone();
    assert!(rig.painted(&format!("{name} | Distortion | Drive")));
    let text = crate::params::module_spec(6).unwrap().params[1].display_text(0.25);
    assert!(rig.painted(&text), "the value text {text:?}");
    // The close button: 2 U at the box's top right (y 142…206), the box 1 U in from the screen's sides.
    rig.click(pos2(1170.0 - 64.0, 142.0 + 32.0));
    rig.frame(vec![]);
    assert_eq!(fine_tuning(&rig), None);
    assert_eq!(rig.app.nav.overlays, [Overlay::Rack(2)], "the rack stays");
}

#[test]
fn a_drag_in_the_dialog_sets_the_param_live_and_saves_no_undo_step() {
    let mut rig = effects_rig();
    let drive = rig.text_rect("Drive").center();
    rig.touch_at(drive);
    rig.ctrl_pick("Fine tune control");
    rig.take();
    let undos = rig.undos();
    tune_drag(&mut rig, 0.25, 0.5);
    let v = rig.app.song.channels[2].param(DRIVE).unwrap();
    assert!((v - 0.75).abs() < 1e-3, "{v}");
    let sent = rig.take();
    assert!(
        sent.iter().all(|c| c.starts_with("SetParam 2 2 1 ")),
        "{sent:?}"
    );
    assert_eq!(sent.last().unwrap(), &format!("SetParam 2 2 1 {v:.3}"));
    assert!(rig.painted(&crate::params::module_spec(6).unwrap().params[1].display_text(v)));
    assert_eq!(rig.undos(), undos, "r36 C");
    tune_drag(&mut rig, v, -0.25);
    let w = rig.app.song.channels[2].param(DRIVE).unwrap();
    assert!(w < v, "{w} {v}");
    assert_eq!(rig.undos(), undos);
    assert_eq!(fine_tuning(&rig), Some((2, DRIVE)), "still open");
    // An undo takes back the effect track's add before them, its redo brings the Distortion back as the last drag left
    // it.
    rig.frame(z(egui::Modifiers::NONE));
    assert_eq!(rig.app.song.channels.len(), 2);
    rig.frame(z(egui::Modifiers::SHIFT));
    assert_eq!(rig.app.song.channels[2].param(DRIVE), Some(w));
}

#[test]
fn a_double_tap_in_the_dialog_resets_the_param_to_its_default_with_no_undo_step() {
    let mut rig = effects_rig();
    *rig.app.song.channels[2].param_mut(DRIVE).unwrap() = 0.9;
    rig.app.song_edited(SongEdit::Final, 0);
    let drive = rig.text_rect("Drive").center();
    rig.touch_at(drive);
    rig.ctrl_pick("Fine tune control");
    let undos = rig.undos();
    rig.take();
    // Two touch-downs 0.2 s apart, away from the thumb: no jump, then the reset.
    let at = pos2(tune_x(0.1), TUNE_Y);
    rig.press(at);
    rig.release(at);
    assert_eq!(rig.app.song.channels[2].param(DRIVE), Some(0.9));
    rig.button(at, true);
    rig.release(at);
    rig.frame(vec![]);
    // Drive's default (the module default).
    assert_eq!(rig.app.song.channels[2].param(DRIVE), Some(0.25));
    assert_eq!(rig.take(), ["SetParam 2 2 1 0.250"]);
    assert_eq!(rig.undos(), undos);
}

#[test]
fn escape_closes_the_dialog() {
    let mut rig = effects_rig();
    let drive = rig.text_rect("Drive").center();
    rig.touch_at(drive);
    rig.ctrl_pick("Fine tune control");
    assert_eq!(fine_tuning(&rig), Some((2, DRIVE)));
    rig.frame(escape());
    rig.frame(vec![]);
    assert_eq!(fine_tuning(&rig), None);
    assert_eq!(rig.app.nav.overlays, [Overlay::Rack(2)]);
}

/// Opens the parameter menu of the control at `at` and returns whether it offers `Fine tune control`; picks it when it
/// does.
fn pick_fine_tune(rig: &mut Rig, at: Pos2) -> bool {
    rig.secondary_click(at);
    rig.frame(vec![]);
    assert!(rig.painted("Add automation track"), "the menu is open");
    if !rig.painted("Fine tune control") {
        rig.frame(escape());
        rig.frame(vec![]);
        return false;
    }
    let item = rig.text_rect("Fine tune control").center();
    rig.click(item);
    // The dialog opens after the pick's frame; egui lays its contents out unseen on their first frame.
    rig.frame(vec![]);
    rig.frame(vec![]);
    true
}

#[test]
fn a_rack_knobs_parameter_menu_offers_fine_tune_control_and_opens_the_dialog() {
    let mut rig = effects_rig();
    let gain = rig.text_rect("Gain").center();
    assert!(pick_fine_tune(&mut rig, gain));
    assert_eq!(rig.recent(), Some((2, GAIN)));
    assert_eq!(fine_tuning(&rig), Some((2, GAIN)));
}

#[test]
fn a_mixer_faders_dialog_tunes_the_channel_level_on_its_strip() {
    let mut rig = effects_rig();
    rig.app.navigate(Nav::Show(View::Mixer));
    rig.frame(vec![]);
    // Channel 1's fader: strips are 128 px wide in rack order (TRK).
    let strip = rig
        .app
        .song
        .rack_order()
        .iter()
        .position(|&c| c == 1)
        .unwrap() as f32;
    assert!(pick_fine_tune(&mut rig, pos2(strip * 128.0 + 64.0, 100.0)));
    assert_eq!(fine_tuning(&rig), Some((1, target(0, 0))));
    let name = rig.app.song.channels[1].name.clone();
    assert!(rig.painted(&format!("{name} | Channel | Level")));
    rig.take();
    tune_drag(&mut rig, 0.8531, -0.1);
    let sent = rig.take();
    assert!(!sent.is_empty());
    assert!(
        sent.iter().all(|c| c.starts_with("SetParam 1 0 0 ")),
        "{sent:?}"
    );
    let level = rig.app.song.channels[1].params.level();
    assert!((level - 0.7531).abs() < 1e-3, "{level}");
}

#[test]
fn a_drum_pad_strips_dialog_names_the_pad_and_resets_to_the_spec_value() {
    let mut rig = effects_rig();
    rig.app.add_default_drums(0);
    let drums = rig.app.song.channels.len() - 1;
    let slot = &mut rig.app.song.channels[drums]
        .drum_kit
        .as_mut()
        .unwrap()
        .slots[1];
    slot.file.custom_pad_name = "Snare".into();
    slot.params[0] = 0.3;
    let id = -(slot.file.id as i32);
    rig.app.song_edited(SongEdit::Final, 0);
    rig.edit(drums);
    rig.slide_split();
    let row = rig.app.slots.drum_editor.step.selected;
    rig.click_id(egui::Id::new("step_seq").with(("row", row)));
    rig.frame(vec![]);
    rig.frame(vec![]);
    let level = egui::Id::new(("drum_editor", drums))
        .with("drum strips")
        .with((1_usize, Ctl::Level));
    let at = rig.ctx.read_response(level).expect("drawn").rect.center();
    assert!(pick_fine_tune(&mut rig, at));
    assert_eq!(fine_tuning(&rig), Some((drums, target(id, 0))));
    let name = rig.app.song.channels[drums].name.clone();
    assert!(rig.painted(&format!("{name} | Snare | Level")));
    // The double tap: 0.85312, not the strip slider's own 0.90909.
    rig.take();
    let at = pos2(tune_x(0.9), TUNE_Y);
    rig.click(at);
    rig.button(at, true);
    rig.release(at);
    let kit = rig.app.song.channels[drums].drum_kit.as_ref().unwrap();
    assert_eq!(
        kit.slot_by_id(id.unsigned_abs()).unwrap().params[0],
        0.85312
    );
    assert_eq!(rig.take(), [format!("SetParam {drums} {id} 0 0.853")]);
}

/// [`effects_rig`] plus a Randomizer, a note effect, with the fine tune dialog open on its Velocity (p2, continuous)
/// and an empty log. Returns the target.
fn randomizer_rig() -> (Rig, AutomationTarget) {
    let mut rig = effects_rig();
    let rack = &mut rig.app.song.channels[2].rack;
    let instance_id = crate::ui::modules_browser::add_module(rack, 49, InsertAt::End);
    rig.app.song_edited(SongEdit::Final, 0);
    let velocity = target(instance_id, 2);
    let id = rig.app.song.channels[2].id;
    rig.app.recent = Some((id, velocity));
    rig.frame(vec![]);
    rig.ctrl_pick("Fine tune control");
    assert_eq!(fine_tuning(&rig), Some((2, velocity)));
    rig.take();
    (rig, velocity)
}

#[test]
fn a_note_effects_param_rebuilds_the_engine_once_the_drag_ends() {
    let (mut rig, velocity) = randomizer_rig();
    let undos = rig.undos();
    let start = pos2(tune_x(0.0), TUNE_Y);
    rig.press(start);
    rig.drag_to(pos2(tune_x(0.3), TUNE_Y));
    assert!(
        !rig.take().iter().any(|c| c.starts_with("ReplaceSong")),
        "not mid-drag"
    );
    rig.release(pos2(tune_x(0.3), TUNE_Y));
    rig.frame(vec![]);
    assert!(
        rig.take().iter().any(|c| c.starts_with("ReplaceSong")),
        "at its end"
    );
    assert_eq!(rig.undos(), undos, "no undo step (r36 C)");
    // A gone module closes the dialog.
    rig.app.song.channels[2].rack.remove(velocity.instance_id);
    rig.app.song_edited(SongEdit::Final, 0);
    rig.frame(vec![]);
    rig.frame(vec![]);
    assert_eq!(fine_tuning(&rig), None);
}

#[test]
fn a_rack_knob_on_a_stepped_param_does_not_offer_it() {
    // The MiniSynth (channel 1): Transp is Transpose, 25 semitone steps; Modifier is continuous.
    let mut rig = Rig::with_synth();
    rig.open_rack(1);
    rig.slide_rack();
    let transp = rig.text_rect("Transp").center();
    assert!(!pick_fine_tune(&mut rig, transp));
    let modifier = rig.text_rect("Modifier").center();
    assert!(pick_fine_tune(&mut rig, modifier));
    let minisynth = rig.app.song.channels[1].rack.modules[0].instance_id;
    assert_eq!(fine_tuning(&rig), Some((1, target(minisynth, 7))));
}

#[test]
fn rec_records_a_fine_tune_drag_as_it_does_a_hand_move() {
    use crate::song::points::{self, PointKey};
    let mut rig = effects_rig();
    let drive = rig.text_rect("Drive").center();
    rig.touch_at(drive);
    rig.ctrl_pick("Fine tune control");
    rig.app.settings.count_in = false;
    rig.app.main_screen.armed = true;
    rig.frame(vec![]);
    tune_drag(&mut rig, 0.25, 0.25);
    assert!(
        rig.app.main_screen.playing,
        "the move started the transport"
    );
    let lanes = &rig.app.song.channels[2].lanes;
    let lane = lanes.iter().find(|l| l.target == Some(DRIVE));
    let clip = &lane.expect("a recorded Drive lane").clips[0];
    let last = points::points(clip, PointKey::Automation)
        .pop()
        .expect("points");
    let v = rig.app.song.channels[2].param(DRIVE).unwrap();
    assert_eq!(last.value, points::raw(v));
}

#[test]
fn escape_mid_drag_on_a_note_effects_param_still_rebuilds_the_engine() {
    let (mut rig, _) = randomizer_rig();
    let undos = rig.undos();
    rig.press(pos2(tune_x(0.0), TUNE_Y));
    rig.drag_to(pos2(tune_x(0.3), TUNE_Y));
    assert!(!rig.take().iter().any(|c| c.starts_with("ReplaceSong")));
    // Escape with the pointer still down: the dialog closes, and the edit it made is built in now.
    rig.frame(escape());
    assert_eq!(fine_tuning(&rig), None);
    assert!(
        rig.take().iter().any(|c| c.starts_with("ReplaceSong")),
        "rebuilt at the close"
    );
    rig.release(pos2(tune_x(0.3), TUNE_Y));
    rig.frame(vec![]);
    assert!(
        !rig.take().iter().any(|c| c.starts_with("ReplaceSong")),
        "once"
    );
    assert_eq!(rig.undos(), undos, "no undo step (r36 C)");
}

#[test]
fn fine_tune_from_the_level_dialogs_slider_closes_that_dialog_under_the_fine_tune_dialog() {
    let mut rig = effects_rig();
    // An effect's icon (left of its name in the card header) opens its Mix / Post Gain / Wet Pan dialog on a click.
    let name = rig.text_rect("Distortion");
    rig.click(pos2(name.left() - 1.2 * U, name.center().y));
    rig.frame(vec![]);
    assert!(rig.painted("Wet Pan"), "the level dialog is open");
    let mix = rig.text_rect("Mix").center();
    assert!(pick_fine_tune(&mut rig, mix));
    assert_eq!(
        fine_tuning(&rig),
        Some((2, target(2, AutomationTarget::MIX)))
    );
    assert!(!rig.painted("Wet Pan"), "the level dialog closed");
    let name = rig.app.song.channels[2].name.clone();
    assert!(rig.painted(&format!("{name} | Distortion | Mix")));
}

#[test]
fn a_dw_samplers_kept_attack_init_follows_a_fine_tune_drag() {
    let mut rig = Rig::with_synth();
    rig.app
        .add_channel(ChannelKind::Synth(SynthKind::DwSampler), 0);
    let channel = rig.app.song.channels.len() - 1;
    let ch = &mut rig.app.song.channels[channel];
    let module = &mut ch.rack.modules[0];
    assert_eq!(module.type_id, 1, "a DW Sampler");
    // A program's Attack and Release init values, kept apart from the params.
    module.keep_program_init(Some((0.9, 0.7)));
    let attack = target(module.instance_id, 1);
    let id = ch.id;
    rig.app.song_edited(SongEdit::Final, 0);
    rig.app.recent = Some((id, attack));
    rig.frame(vec![]);
    rig.ctrl_pick("Fine tune control");
    assert_eq!(fine_tuning(&rig), Some((channel, attack)));
    let v = rig.app.song.channels[channel].param(attack).unwrap();
    tune_drag(&mut rig, v, 0.2);
    let module = &rig.app.song.channels[channel].rack.modules[0];
    let now = module.params[1];
    assert!((now - (v + 0.2)).abs() < 1e-3, "{now}");
    assert_eq!(
        module.program_init(),
        [now, 0.7],
        "the Attack init follows, Release stays"
    );
}

#[test]
fn a_fine_tune_drag_of_the_tremolos_max_below_min_lowers_min_with_no_undo_step() {
    // Max below Min lowers Min to it, as the effect does with the audio's copy.
    let mut rig = Rig::with_synth();
    rig.app.add_channel(ChannelKind::EffectTrack, 0);
    let ch = &mut rig.app.song.channels[2];
    crate::ui::modules_browser::add_module(&mut ch.rack, 46, InsertAt::End);
    let max = target(ch.rack.modules[0].instance_id, 3);
    (ch.rack.modules[0].params[3], ch.rack.modules[0].params[4]) = (0.8, 0.5);
    let id = ch.id;
    rig.app.song_edited(SongEdit::Final, 0);
    rig.app.recent = Some((id, max));
    rig.frame(vec![]);
    rig.ctrl_pick("Fine tune control");
    assert_eq!(fine_tuning(&rig), Some((2, max)));
    let undos = rig.undos();
    rig.app.files.saved();
    tune_drag(&mut rig, 0.8, -0.5);
    let limits = |rig: &Rig| {
        let params = &rig.app.song.channels[2].rack.modules[0].params;
        (params[3], params[4])
    };
    let (high, low) = limits(&rig);
    assert!((high - 0.3).abs() < 1e-3, "{high}");
    assert_eq!(low, high, "Min follows Max down");
    // Channel params: the song-changed flag and no undo step; an undo takes back the effect track's add before them,
    // and its redo brings the Tremolo back with both moved limits.
    assert!(rig.app.files.modified());
    assert_eq!(rig.undos(), undos);
    rig.frame(z(egui::Modifiers::NONE));
    rig.frame(z(egui::Modifiers::SHIFT));
    assert_eq!(limits(&rig), (high, low));
}

#[test]
fn a_tempo_change_makes_the_tempo_recent_and_add_automation_track_automates_it() {
    // The tempo dialog's dial makes the master's tempo the recent control; CTRL's `Add automation track` gives the
    // master a `Tempo` lane and opens it.
    let mut rig = Rig::with_synth();
    rig.frame(vec![]);
    rig.click(BPM);
    rig.frame(vec![]);
    rig.press(dial(0));
    for quarter in 1..=4 {
        rig.drag_to(dial(quarter));
    }
    rig.release(dial(4));
    rig.frame(vec![]);
    assert_eq!(rig.recent(), Some((0, crate::song::TEMPO_TARGET)));
    rig.frame(escape());
    rig.frame(vec![]);
    rig.ctrl_pick("Add automation track");
    let lane = rig.app.song.channels[0].lanes.last().unwrap();
    assert_eq!(
        (lane.target, lane.label.as_str()),
        (Some(crate::song::TEMPO_TARGET), "Tempo")
    );
    assert_eq!(rig.app.nav.view, View::AutomationEditor(0));
}
