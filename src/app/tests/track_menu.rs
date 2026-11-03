//! The playlist track header menu in the app: each item's action, one undo step
//! each, and what the app keeps by channel index following its channel when channels move.

use eframe::egui::{Event, Id};

use super::*;
use crate::song::{Event as SongEvent, LaneKind};
use crate::ui::clips::ClipRef;
use crate::ui::piano_roll::PianoRoll;
use crate::ui::preview::ClipKey;
use crate::ui::track_menu::TrackOp;
use crate::ui::views::preset_picker::CHANNEL_LIMIT;

/// Track menu item (column, row).
fn item(col: usize, row: usize) -> Id {
    Id::new("track_menu").with((col, row))
}

const HIDE: (usize, usize) = (0, 0);
const SETTINGS: (usize, usize) = (0, 1);
const CLONE: (usize, usize) = (0, 2);
const INSERT: (usize, usize) = (0, 3);
const DELETE_TRACK: (usize, usize) = (0, 4);
const DELETE_CHANNEL: (usize, usize) = (0, 5);
/// `Add Track` of an instrument, `Add StepSeq Track` and `Add Notes Track` of a drums channel.
const ADD_TRACK: (usize, usize) = (1, 3);
const ADD_STEP_SEQ: (usize, usize) = (1, 2);
const ADD_NOTES: (usize, usize) = (1, 3);

fn names(rig: &Rig) -> Vec<String> {
    (rig.app.song.channels.iter())
        .map(|c| c.name.clone())
        .collect()
}

fn lanes(rig: &Rig, channel: usize) -> Vec<LaneKind> {
    (rig.app.song.channels[channel].lanes.iter())
        .map(|l| l.kind)
        .collect()
}

fn note(pos: u32) -> SongEvent {
    SongEvent::Note {
        pos,
        len_beats: 1.0,
        key: 60,
        velocity: SongEvent::DEFAULT_VELOCITY,
        pan: SongEvent::DEFAULT_PAN,
        slide: false,
    }
}

fn undo() -> Vec<egui::Event> {
    key_tap(egui::Key::Z, egui::Modifiers::COMMAND)
}

fn redo() -> Vec<egui::Event> {
    key_tap(
        egui::Key::Z,
        egui::Modifiers::COMMAND | egui::Modifiers::SHIFT,
    )
}

fn select_all() -> Event {
    Event::Key {
        key: egui::Key::A,
        physical_key: None,
        pressed: true,
        repeat: false,
        modifiers: egui::Modifiers::COMMAND,
    }
}

fn enter() -> Event {
    Event::Key {
        key: egui::Key::Enter,
        physical_key: None,
        pressed: true,
        repeat: false,
        modifiers: Default::default(),
    }
}

impl Rig {
    /// Long-presses (a secondary click) row `row`'s header and picks the menu item at `at`.
    pub(super) fn track_menu(&mut self, row: usize, at: (usize, usize)) {
        let pos = header(row);
        self.frame(vec![Event::PointerMoved(pos)]);
        for pressed in [true, false] {
            self.frame(vec![Event::PointerButton {
                pos,
                button: egui::PointerButton::Secondary,
                pressed,
                modifiers: Default::default(),
            }]);
        }
        self.frame(vec![]);
        self.click_id(item(at.0, at.1));
        self.frame(vec![]);
    }

    /// Answers the open message: `Yes` (row `lines`) or `No` (the row after).
    fn answer(&mut self, yes: bool) {
        let Some(Overlay::Message { lines, .. }) = self.overlays().last().cloned() else {
            panic!("no message: {:?}", self.overlays());
        };
        let row = lines.len() + usize::from(!yes);
        self.frame(vec![]);
        self.click_id(Id::new("message").with((0_usize, row)));
        self.frame(vec![]);
    }

    /// The open message's lines.
    fn message(&self) -> Vec<String> {
        match self.overlays().last() {
            Some(Overlay::Message { lines, .. }) => lines.clone(),
            other => panic!("no message: {other:?}"),
        }
    }
}

#[test]
fn add_track_adds_a_track_of_the_primary_kind_below_it_one_undo_step() {
    let mut rig = Rig::with_drums_and_synth();
    rig.track_menu(2, ADD_TRACK);
    assert_eq!(lanes(&rig, 2), [LaneKind::Notes, LaneKind::Notes]);
    // The new track has a row of its own, under its primary: its menu is the sub-track's.
    rig.track_menu(3, DELETE_TRACK);
    assert_eq!(lanes(&rig, 2), [LaneKind::Notes]);
    rig.frame(undo());
    assert_eq!(lanes(&rig, 2), [LaneKind::Notes, LaneKind::Notes]);
    rig.frame(undo());
    assert_eq!(lanes(&rig, 2), [LaneKind::Notes]);
    // A drums channel adds either kind, before its automation tracks.
    let target = crate::song::AutomationTarget {
        instance_id: 0,
        param: 1,
    };
    rig.app.song.channels[1].add_automation_lane(target);
    rig.app.song_edited(SongEdit::Final, 0);
    rig.track_menu(1, ADD_STEP_SEQ);
    rig.track_menu(1, ADD_NOTES);
    assert_eq!(
        lanes(&rig, 1),
        [
            LaneKind::Drums,
            LaneKind::Drums,
            LaneKind::Drums,
            LaneKind::Automation
        ]
    );
}

#[test]
fn add_step_seq_and_add_notes_track_add_drum_lanes_of_flag_0_and_1_one_undo_step_each() {
    // `Add StepSeq Track` adds a kind-3 lane with flag 0, `Add Notes Track` a kind-3 lane with flag 1 (DESc@1), each
    // One undo step.
    use crate::song::DrumGrid::{Notes, Steps};
    use LaneKind::Drums;
    let mut rig = Rig::with_drums_and_synth();
    let grids = |rig: &Rig| {
        (rig.app.song.channels[1].lanes.iter())
            .map(|l| (l.kind, l.grid))
            .collect::<Vec<_>>()
    };
    assert_eq!(grids(&rig), [(Drums, Steps)]);
    rig.track_menu(1, ADD_NOTES);
    assert_eq!(grids(&rig), [(Drums, Steps), (Drums, Notes)]);
    rig.track_menu(1, ADD_STEP_SEQ);
    assert_eq!(
        grids(&rig),
        [(Drums, Steps), (Drums, Notes), (Drums, Steps)]
    );
    rig.frame(undo());
    assert_eq!(grids(&rig), [(Drums, Steps), (Drums, Notes)]);
    rig.frame(undo());
    assert_eq!(grids(&rig), [(Drums, Steps)]);
    rig.frame(redo());
    assert_eq!(grids(&rig), [(Drums, Steps), (Drums, Notes)]);
}

#[test]
fn burn_notes_puts_the_loops_notes_as_the_note_effects_play_them_on_a_new_track_one_undo_step() {
    use crate::song::PPQ;
    let mut rig = Rig::with_drums_and_synth();
    let synth = &mut rig.app.song.channels[2];
    // An octave up.
    let transpose = synth.rack.add(48);
    synth.rack.get_mut(transpose).unwrap().params[1] = 5.0 / 8.0;
    // A slide up a tone on the note at beat 2, as the slide plays it (from its anchor).
    let slide = |pos, key| SongEvent::Note {
        pos,
        len_beats: 0.5,
        key,
        velocity: SongEvent::DEFAULT_VELOCITY,
        pan: SongEvent::DEFAULT_PAN,
        slide: true,
    };
    let lane = &mut synth.lanes[0];
    let notes = [note(0), note(2 * PPQ), slide(2 * PPQ + PPQ / 4, 62)];
    lane.add_clip(0, 8.0).events = [&notes[..], &[note(5 * PPQ)]].concat();
    // Past the loop: it stays unmuted.
    lane.add_clip(8 * PPQ, 4.0).events = vec![note(0)];
    rig.app.song.play_loop = Some((PPQ, 6 * PPQ));
    rig.app.song_edited(SongEdit::Final, 0);
    let before = rig.app.song.clone();
    rig.app.track_op(2, 0, TrackOp::BurnNotes, 0);
    let screen = Overlay::Busy {
        title: "Burning notes to track...".into(),
    };
    assert_eq!(rig.overlays(), std::slice::from_ref(&screen));
    rig.frame(vec![]);
    assert_eq!(rig.overlays(), [screen], "it shows a frame first");
    rig.frame(vec![]);
    assert_eq!(rig.overlays(), []);
    let synth = &rig.app.song.channels[2];
    assert_eq!(lanes(&rig, 2), [LaneKind::Notes, LaneKind::Notes]);
    let muted: Vec<bool> = synth.lanes[0].clips.iter().map(|c| c.muted).collect();
    assert_eq!(muted, [true, false]);
    assert_eq!(synth.rack.get(transpose).unwrap().params[0], 0.0);
    // One clip over the loop, the notes starting in it an octave up, from its start, and the slide a tone above its
    // anchor still.
    let [clip] = &synth.lanes[1].clips[..] else {
        panic!("{:?}", synth.lanes[1].clips);
    };
    let spans = (clip.start_tick, clip.visible_len_beats, clip.loop_len_beats);
    assert_eq!(spans, (PPQ, 5.0, 5.0));
    let up = |pos| SongEvent::Note {
        pos,
        len_beats: 1.0,
        key: 72,
        velocity: SongEvent::DEFAULT_VELOCITY,
        pan: SongEvent::DEFAULT_PAN,
        slide: false,
    };
    let slid = slide(PPQ + PPQ / 4, 74);
    assert_eq!(clip.events, [up(PPQ), slid, up(4 * PPQ)]);
    assert_eq!(rig.app.main_screen.clips.track, Some((2, 1)), "current");
    // One undo step takes the track away and unmutes the clips; the note effects stay off, as no undo covers the rack.
    rig.frame(undo());
    let mut want = before.channels;
    want[2].rack.get_mut(transpose).unwrap().params[0] = 0.0;
    assert_eq!(rig.app.song.channels, want);
}

#[test]
fn a_track_added_before_the_automation_tracks_moves_what_is_kept_on_them() {
    let mut rig = Rig::with_drums_and_synth();
    let synth = &mut rig.app.song.channels[2];
    synth.add_automation_lane(crate::song::AutomationTarget {
        instance_id: 0,
        param: 1,
    });
    synth.lanes[1].add_clip(0, 4.0);
    rig.app.song_edited(SongEdit::Final, 0);
    let on = |lane| ClipRef {
        channel: 2,
        lane,
        id: 1,
    };
    rig.app.main_screen.clips.selected = Some(on(1));
    rig.app.track_op(2, 0, TrackOp::AddTrack, 0);
    assert_eq!(rig.app.main_screen.clips.selected, Some(on(2)));
    // Its undo moves it back.
    rig.frame(undo());
    assert_eq!(rig.app.main_screen.clips.selected, Some(on(1)));
}

#[test]
fn hide_tracks_hides_the_sub_track_rows_and_show_tracks_brings_them_back() {
    let mut rig = Rig::with_drums_and_synth();
    rig.track_menu(1, ADD_NOTES);
    // Rows: master, drums, its notes track, synth.
    assert_eq!(track_rows(&rig.app.song, false).len(), 4);
    // A clip selected on the notes track goes with it.
    rig.app.song.channels[1].lanes[1].add_clip(0, 16.0);
    rig.app.song_edited(SongEdit::Final, 0);
    rig.click(lane(2));
    let selected = |rig: &Rig| (rig.app.main_screen.clips.selected).map(|r| (r.channel, r.lane));
    assert_eq!(selected(&rig), Some((1, 1)));
    rig.track_menu(1, HIDE);
    assert!(rig.app.song.channels[1].zipped);
    assert_eq!(track_rows(&rig.app.song, false).len(), 3);
    assert_eq!(rig.app.main_screen.clips.selected, None);
    // The synth moved up a row: a tap on its header makes it current.
    rig.click(header(2));
    assert_eq!(rig.app.selected_channel, 2);
    // An undo step, with no rebuild (the audio never reads it).
    rig.click(OUTSIDE);
    rig.take();
    rig.frame(undo());
    assert!(!rig.app.song.channels[1].zipped);
    assert_eq!(
        rig.take(),
        ["ReplaceSong adopting"],
        "undo rebuilds as usual"
    );
    rig.frame(redo());
    rig.take();
    rig.track_menu(1, HIDE);
    assert!(!rig.app.song.channels[1].zipped, "Show Tracks");
    assert_eq!(rig.take(), Vec::<String>::new());
    // Adding a track shows the hidden ones.
    rig.track_menu(1, HIDE);
    rig.track_menu(1, ADD_NOTES);
    assert!(!rig.app.song.channels[1].zipped);
}

#[test]
fn clone_channel_puts_a_copy_below_and_selects_it_one_undo_step() {
    let mut rig = Rig::with_drums_and_synth();
    rig.app.song.channels[1].lanes[0].add_clip(0, 4.0);
    rig.app.song_edited(SongEdit::Final, 0);
    rig.track_menu(1, CLONE);
    assert_eq!(names(&rig), ["MASTER", "Drums", "Drums 2", "MiniSynth"]);
    assert_eq!(
        rig.app.song.channels[2].lanes,
        rig.app.song.channels[1].lanes
    );
    assert_eq!(rig.app.selected_channel, 2);
    rig.frame(undo());
    assert_eq!(names(&rig), ["MASTER", "Drums", "MiniSynth"]);
}

#[test]
fn insert_channel_puts_the_picked_kind_above_the_track_and_selects_it() {
    let mut rig = Rig::with_drums_and_synth();
    rig.track_menu(2, INSERT);
    // `Instrument` in the add-channel popup.
    rig.click_id(Id::new("insert_channel_popup").with((0_usize, 2_usize)));
    assert_eq!(names(&rig), ["MASTER", "Drums", "Instrument", "MiniSynth"]);
    assert_eq!(rig.app.selected_channel, 2);
    // It holds a SuperSaw and opens the Instruments picker; undo waits while that modal picker is open.
    assert_eq!(rig.app.song.channels[2].rack.modules[0].type_id, 23);
    let target = crate::ui::rack::PresetTarget::Instruments;
    let picker = Overlay::PresetPicker { channel: 2, target };
    assert_eq!(rig.overlays(), [picker]);
    rig.frame(escape());
    rig.frame(undo());
    assert_eq!(names(&rig), ["MASTER", "Drums", "MiniSynth"]);
    assert_eq!(rig.app.selected_channel, 2, "the synth, still selected");
}

#[test]
fn clone_and_insert_on_the_master_leave_it_first_and_alone() {
    // The master's menu has neither item (`ui::track_menu`); a stray pick still changes nothing.
    let mut rig = Rig::with_drums_and_synth();
    rig.app.selected_channel = 2;
    rig.take();
    let song = rig.app.song.clone();
    rig.app.track_op(0, 0, TrackOp::CloneChannel, 0);
    assert_eq!(rig.app.song, song);
    assert_eq!(rig.app.selected_channel, 2);
    assert!(rig.take().is_empty(), "no rebuild");
    // An insert at the master's place goes below it.
    rig.app.insert_channel(0, ChannelKind::Instrument, 0);
    assert_eq!(names(&rig), ["MASTER", "Instrument", "Drums", "MiniSynth"]);
    assert_eq!(rig.app.selected_channel, 1);
}

#[test]
fn delete_channel_asks_and_only_yes_deletes_one_undo_step() {
    let mut rig = Rig::with_drums_and_synth();
    rig.track_menu(1, DELETE_CHANNEL);
    assert_eq!(
        rig.message(),
        ["Delete channel? This action cannot be undone."]
    );
    rig.answer(false);
    assert_eq!(names(&rig), ["MASTER", "Drums", "MiniSynth"]);
    assert_eq!(rig.overlays(), []);
    rig.track_menu(1, DELETE_CHANNEL);
    rig.answer(true);
    assert_eq!(names(&rig), ["MASTER", "MiniSynth"]);
    rig.frame(undo());
    assert_eq!(names(&rig), ["MASTER", "Drums", "MiniSynth"]);
    rig.frame(redo());
    assert_eq!(names(&rig), ["MASTER", "MiniSynth"]);
}

#[test]
fn deleting_a_send_names_its_users_and_routes_them_to_the_master() {
    let mut rig = Rig::with_drums_and_synth();
    rig.app.add_channel(ChannelKind::EffectTrack, 0);
    rig.app.song.channels[1].output_bus = 1;
    rig.app.song_edited(SongEdit::Final, 0);
    rig.track_menu(3, DELETE_CHANNEL);
    assert_eq!(
        rig.message(),
        ["Another channel has this FX channel as the output destination. Delete FX channel?"]
    );
    rig.answer(false);
    rig.app.song.channels[2].output_bus = 1;
    rig.app.song_edited(SongEdit::Final, 0);
    rig.track_menu(3, DELETE_CHANNEL);
    assert_eq!(
        rig.message(),
        ["Other channels have this FX channel as the output destination. Delete FX channel?"]
    );
    rig.answer(true);
    assert_eq!(names(&rig), ["MASTER", "Drums", "MiniSynth"]);
    assert!(rig.app.song.channels.iter().all(|c| c.output_bus == -1));
}

#[test]
fn delete_track_deletes_an_empty_track_and_asks_after_emptying_one() {
    let mut rig = Rig::with_drums_and_synth();
    rig.track_menu(2, ADD_TRACK);
    rig.track_menu(2, ADD_TRACK);
    rig.app.song.channels[2].lanes[1].add_clip(0, 4.0).events = vec![note(0)];
    rig.app.song_edited(SongEdit::Final, 0);
    // An empty track goes at once.
    rig.track_menu(4, DELETE_TRACK);
    assert_eq!(lanes(&rig, 2).len(), 2);
    assert_eq!(rig.overlays(), []);
    // One with clips loses them, then the question is whether the emptied track goes too: no keeps it.
    rig.track_menu(3, DELETE_TRACK);
    assert!(rig.app.song.channels[2].lanes[1].clips.is_empty());
    assert_eq!(rig.message(), ["Delete empty tracks?"]);
    rig.answer(false);
    assert_eq!(lanes(&rig, 2).len(), 2);
    rig.frame(undo());
    assert_eq!(rig.app.song.channels[2].lanes[1].clips.len(), 1);
    rig.track_menu(3, DELETE_TRACK);
    rig.answer(true);
    assert_eq!(lanes(&rig, 2), [LaneKind::Notes]);
    // Two steps: the track, then its clips.
    rig.frame(undo());
    assert_eq!(lanes(&rig, 2).len(), 2);
    assert!(rig.app.song.channels[2].lanes[1].clips.is_empty());
    rig.frame(undo());
    assert_eq!(rig.app.song.channels[2].lanes[1].clips.len(), 1);
}

#[test]
fn a_playlist_delete_asks_to_delete_what_it_emptied_in_its_own_undo_step() {
    // Drums and MiniSynth one clip each, Instrument one on each of three tracks.
    let mut rig = Rig::with_drums_and_synth();
    rig.app.add_bare_channel(ChannelKind::Instrument, 0);
    let ch = &mut rig.app.song.channels[3];
    ch.add_lane(LaneKind::Notes);
    ch.add_lane(LaneKind::Notes);
    let mut clip = |channel: usize, lane: usize| {
        let id = rig.app.song.channels[channel].lanes[lane]
            .add_clip(0, 4.0)
            .id;
        ClipRef { channel, lane, id }
    };
    let (drums, synth) = (clip(1, 0), clip(2, 0));
    let (first, second, third) = (clip(3, 0), clip(3, 1), clip(3, 2));
    rig.app.song_edited(SongEdit::Final, 0);
    let all = ["MASTER", "Drums", "MiniSynth", "Instrument"];
    let delete = |rig: &mut Rig, group: Vec<ClipRef>| {
        rig.app.main_screen.clips.group = group;
        rig.frame(key_tap(egui::Key::Delete, egui::Modifiers::NONE));
        rig.frame(vec![]);
    };
    let clips = |rig: &Rig| -> Vec<usize> {
        let lanes = rig.app.song.channels.iter().flat_map(|c| &c.lanes);
        lanes.map(|l| l.clips.len()).collect()
    };
    // One selection over two channels empties both: one question names them, and its yes is in the delete's step.
    delete(&mut rig, vec![drums, synth]);
    assert_eq!(
        rig.message(),
        [
            "Delete the 2 empty channels too?",
            "\"Drums\", \"MiniSynth\""
        ]
    );
    rig.answer(true);
    assert_eq!(names(&rig), ["MASTER", "Instrument"]);
    rig.frame(undo());
    assert_eq!(
        (names(&rig), clips(&rig)),
        (all.map(String::from).to_vec(), vec![0, 1, 1, 1, 1, 1])
    );
    // A channel and two tracks of another (never the track a clip is left on): Keep keeps the delete alone.
    delete(&mut rig, vec![drums, second, third]);
    assert_eq!(
        rig.message(),
        ["Delete the channel \"Drums\" and remove 2 empty lanes too?"]
    );
    rig.answer(false);
    assert_eq!(
        (names(&rig).len(), clips(&rig)),
        (4, vec![0, 0, 1, 1, 0, 0])
    );
    rig.frame(undo());
    assert_eq!(clips(&rig), [0, 1, 1, 1, 1, 1]);
    // Live, the delete waits for the answer: delete and yes are one change, one step of the session's undo.
    struct Session;
    impl Drop for Session {
        fn drop(&mut self) {
            crate::app::live::disconnect();
        }
    }
    let _session = Session;
    crate::app::live::connect();
    rig.frame(vec![]);
    let reported = crate::app::live::changes();
    delete(&mut rig, vec![second, third]);
    assert_eq!(rig.message(), ["Remove the 2 empty lanes too?"]);
    assert_eq!(crate::app::live::changes(), reported);
    rig.answer(true);
    assert_eq!(lanes(&rig, 3), [LaneKind::Notes]);
    assert_eq!(rig.app.song.channels[3].lanes[0].clips[0].id, first.id);
    assert_eq!(crate::app::live::changes(), reported + 1);
}

#[test]
fn channel_settings_renames_and_recolours_the_channel_one_undo_step_each() {
    let mut rig = Rig::with_drums_and_synth();
    rig.track_menu(2, SETTINGS);
    assert_eq!(
        rig.overlays(),
        [Overlay::ChannelSettings {
            channel: 2,
            name: "MiniSynth".into()
        }]
    );
    let dialog = Id::new("channel_settings");
    // The name field has the focus: select all, type, Enter.
    rig.frame(vec![select_all(), Event::Text("Lead".into())]);
    rig.frame(vec![enter()]);
    rig.frame(vec![]);
    assert_eq!(rig.app.song.channels[2].name, "Lead");
    assert_eq!(rig.overlays(), [], "Enter closes it");
    // A blank name is ignored, and Escape drops what was typed.
    for (typed, close) in [(" ", vec![enter()]), ("Bass", escape())] {
        rig.track_menu(2, SETTINGS);
        rig.frame(vec![select_all(), Event::Text(typed.into())]);
        rig.frame(close);
        rig.frame(vec![]);
        assert_eq!(rig.app.song.channels[2].name, "Lead", "{typed:?}");
        assert_eq!(rig.overlays(), []);
    }
    // The colour: a drag along the slider, one step.
    rig.track_menu(2, SETTINGS);
    let hue = rig.app.song.channels[2].hue;
    rig.frame(vec![]);
    let slider = rig
        .ctx
        .read_response(dialog.with("color"))
        .expect("the slider")
        .rect;
    // The dialog sits above the transport bar.
    assert!(slider.bottom() < PANEL_TOP, "{slider:?}");
    rig.press(slider.center());
    rig.drag_to(slider.center() + vec2(40.0, 0.0));
    rig.drag_to(slider.center() + vec2(80.0, 0.0));
    rig.release(slider.center() + vec2(80.0, 0.0));
    let moved = rig.app.song.channels[2].hue;
    assert_ne!(moved, hue);
    rig.frame(escape());
    assert_eq!(rig.overlays(), []);
    rig.frame(undo());
    assert_eq!(rig.app.song.channels[2].hue, hue);
    assert_eq!(rig.app.song.channels[2].name, "Lead");
    rig.frame(undo());
    assert_eq!(rig.app.song.channels[2].name, "MiniSynth");
    // An instrument channel's MIDI input: its arm button arms it, one step.
    rig.track_menu(2, SETTINGS);
    rig.frame(vec![]);
    let arm = rig.ctx.read_response(dialog.with("arm")).expect("arm").rect;
    rig.click(arm.center());
    assert!(rig.app.song.channels[2].file.record_armed);
    rig.frame(escape());
    rig.frame(undo());
    assert!(!rig.app.song.channels[2].file.record_armed);
    // A gone channel closes it.
    rig.track_menu(2, SETTINGS);
    rig.app.song.delete_channel(2);
    rig.app.song_edited(SongEdit::Final, 0);
    rig.frame(vec![]);
    assert_eq!(rig.overlays(), []);
}

/// Dialog boxes: screen dim α 0.6, then the floating panel, from the dialog's first frame.
#[test]
fn channel_settings_is_on_the_dialog_base_the_dim_then_the_panel() {
    let mut rig = Rig::with_drums_and_synth();
    rig.track_menu(2, SETTINGS);
    assert_eq!(rig.overlays().len(), 1, "the dialog's first frame");
    let screen = egui::Rect::from_min_size(egui::Pos2::ZERO, egui::vec2(1170.0, 540.0));
    let m = crate::metrics::Metrics::from_pixels_per_point(1.0, crate::metrics::DEFAULT_UI_SCALE);
    // 20 U × 5.75 U on an instrument channel (10 U wide on others), centred in the content area above the transport
    // bar.
    let area = crate::ui::main_screen::Regions::new(screen, &m).area;
    let boxed = egui::Rect::from_center_size(area.center(), egui::vec2(20.0 * 32.0, 5.75 * 32.0));
    let painted = crate::ui::dialog::painted_rects(rig.out.shapes.iter().map(|s| &s.shape));
    let colors = crate::theme::theme_by_id(0).colors;
    crate::ui::dialog::assert_base(&painted, &colors, screen, boxed);
}

/// Drums (1), MiniSynth (2) with a clip and another MiniSynth (3) with a clip, as loaded; the audio started, the
/// keyboard on channel 3, and an empty log.
fn three_channels() -> Rig {
    let mut rig = Rig::with_drums_and_synth();
    rig.app
        .add_bare_channel(ChannelKind::Synth(SynthKind::MiniSynth), 0);
    for channel in [2, 3] {
        rig.app.song.channels[channel].lanes[0]
            .add_clip(0, 4.0)
            .events = vec![note(0)];
    }
    rig.app.song_edited(SongEdit::Replaced, 0);
    rig.app.selected_channel = 3;
    rig.click(OUTSIDE);
    rig.take();
    rig
}

/// What the app keeps for channel `c`'s clip 1 on lane 0, set up by hand: its piano roll shown with its rack over it,
/// Preview and the clip selection.
fn keep_state_of(rig: &mut Rig, c: usize) {
    let key = ClipKey {
        channel: c,
        lane: 0,
        id: 1,
    };
    rig.app.navigate(Nav::EditClip(key));
    rig.app.navigate(Nav::Open(Overlay::Rack(c)));
    (rig.app.slots.piano_roll.rolls).insert(c, PianoRoll::new(c, 0, Some(1)));
    rig.app.preview = Some(key);
    rig.app.main_screen.clips.selected = Some(ClipRef {
        channel: c,
        lane: 0,
        id: 1,
    });
}

/// Where the app's per-channel state points: (selected, overlays, rolls as (key, channel), Preview's channel, the
/// selected clip's channel, the view).
type Kept = (
    usize,
    Vec<Overlay>,
    Vec<(usize, usize)>,
    Option<usize>,
    Option<usize>,
    View,
);

fn state(rig: &Rig) -> Kept {
    let mut rolls: Vec<(usize, usize)> = (rig.app.slots.piano_roll.rolls.iter())
        .map(|(&k, r)| (k, r.channel))
        .collect();
    rolls.sort();
    (
        rig.app.selected_channel,
        rig.overlays(),
        rolls,
        rig.app.preview.map(|k| k.channel),
        rig.app.main_screen.clips.selected.map(|r| r.channel),
        rig.app.nav.view,
    )
}

#[test]
fn a_deleted_channel_moves_what_is_kept_for_the_channels_below_it_and_undo_moves_it_back() {
    let mut rig = three_channels();
    keep_state_of(&mut rig, 3);
    let before = state(&rig);
    assert_eq!(
        before,
        (
            3,
            vec![Overlay::Rack(3)],
            vec![(3, 3)],
            Some(3),
            Some(3),
            View::PianoRoll(3)
        )
    );
    rig.app.track_op(2, 0, TrackOp::DeleteChannel, 0);
    rig.answer(true);
    assert_eq!(names(&rig), ["MASTER", "Drums", "MiniSynth 2"]);
    assert_eq!(
        state(&rig),
        (
            2,
            vec![Overlay::Rack(2)],
            vec![(2, 2)],
            Some(2),
            Some(2),
            View::PianoRoll(2)
        )
    );
    rig.frame(undo());
    assert_eq!(state(&rig), before);
    rig.frame(redo());
    assert_eq!(
        state(&rig),
        (
            2,
            vec![Overlay::Rack(2)],
            vec![(2, 2)],
            Some(2),
            Some(2),
            View::PianoRoll(2)
        )
    );
}

#[test]
fn a_deleted_channel_takes_its_own_state_and_held_note_with_it() {
    let mut rig = three_channels();
    keep_state_of(&mut rig, 3);
    // A key held on channel 3. Preview, set by hand, sends its loop.
    rig.press(E4);
    assert_eq!(rig.take(), ["SetLoop 0 512", "NoteOn 3 52 108"]);
    rig.app.delete_channel(3, 0);
    // The note stops in the engine it sounds in, before the rebuild.
    assert_eq!(rig.take(), ["NoteOff 3 52", "ReplaceSong adopting"]);
    // Its rack shows the current channel.
    let rack = vec![Overlay::Rack(2)];
    assert_eq!(state(&rig), (2, rack, vec![], None, None, View::Playlist));
    // The key is up already: its release sends nothing.
    rig.release(E4);
    assert_eq!(rig.take(), Vec::<String>::new());
}

#[test]
fn an_inserted_or_cloned_channel_moves_what_is_kept_for_the_channels_below_it() {
    let mut rig = three_channels();
    keep_state_of(&mut rig, 2);
    rig.app.track_op(1, 0, TrackOp::CloneChannel, 0);
    assert_eq!(
        state(&rig),
        (
            2,
            vec![Overlay::Rack(3)],
            vec![(3, 3)],
            Some(3),
            Some(3),
            View::PianoRoll(3)
        ),
        "the clone (2) is selected"
    );
    // A new Drums channel opens its drumset picker too, over its own drum editor, which
    // closed the rack; the rest follows its channel.
    rig.app.insert_channel(1, ChannelKind::Drums, 0);
    let picker = Overlay::PresetPicker {
        channel: 1,
        target: PresetTarget::Drumset,
    };
    assert_eq!(
        state(&rig),
        (
            1,
            vec![picker],
            vec![(4, 4)],
            Some(4),
            Some(4),
            View::DrumEditor(1)
        )
    );
}

#[test]
fn deleting_a_track_moves_the_lanes_kept_below_it() {
    let mut rig = three_channels();
    let synth = &mut rig.app.song.channels[2];
    synth.add_lane(LaneKind::Notes);
    synth.add_lane(LaneKind::Notes);
    synth.lanes[2].add_clip(0, 4.0);
    rig.app.song_edited(SongEdit::Final, 0);
    let key = ClipKey {
        channel: 2,
        lane: 2,
        id: 1,
    };
    rig.app.preview = Some(key);
    (rig.app.slots.piano_roll.rolls).insert(2, PianoRoll::new(2, 2, Some(1)));
    rig.app.main_screen.clips.selected = Some(ClipRef {
        channel: 2,
        lane: 2,
        id: 1,
    });
    rig.app.track_op(2, 1, TrackOp::DeleteTrack, 0);
    assert_eq!(rig.app.preview, Some(ClipKey { lane: 1, ..key }));
    assert_eq!(rig.app.slots.piano_roll.rolls[&2].lane, 1);
    assert_eq!(rig.app.main_screen.clips.selected.map(|r| r.lane), Some(1));
    // The track they are on goes: Preview, the roll and the selection with it.
    rig.app.track_op(2, 1, TrackOp::DeleteTrack, 0);
    rig.answer(true);
    assert_eq!(rig.app.preview, None);
    assert!(!rig.app.slots.piano_roll.rolls.contains_key(&2));
    assert_eq!(rig.app.main_screen.clips.selected, None);
}

#[test]
fn undoing_a_track_delete_puts_the_drum_editor_back_on_its_track() {
    // Regression: the editor stayed on index 1 (the moved track) instead of following its lane.
    let mut rig = Rig::with_drums_and_synth();
    rig.track_menu(1, ADD_NOTES);
    rig.track_menu(1, ADD_STEP_SEQ);
    assert_eq!(
        lanes(&rig, 1),
        [LaneKind::Drums, LaneKind::Drums, LaneKind::Drums]
    );
    rig.app.song.channels[1].lanes[2].add_clip(0, 4.0);
    rig.app.song_edited(SongEdit::Final, 0);
    let d2 = ClipKey {
        channel: 1,
        lane: 2,
        id: 1,
    };
    rig.app.navigate(Nav::EditClip(d2));
    rig.slide_split();
    rig.app.navigate(Nav::Show(View::Playlist));
    let step = |rig: &Rig| {
        let step = &rig.app.slots.drum_editor.step;
        (step.lane, step.views.contains_key(&(1, step.lane, Some(1))))
    };
    assert_eq!(step(&rig), (2, true));
    rig.app.track_op(1, 1, TrackOp::DeleteTrack, 0);
    assert_eq!(step(&rig), (1, true));
    rig.frame(undo());
    assert_eq!(step(&rig), (2, true));
    rig.frame(redo());
    assert_eq!(step(&rig), (1, true));
}

#[test]
fn undoing_a_track_delete_puts_the_roll_and_preview_back_on_their_track() {
    // Regression: after two undos the roll and Preview edited lane 1's clip 1 instead of lane 2's.
    let mut rig = three_channels();
    let synth = &mut rig.app.song.channels[2];
    for _ in 0..2 {
        let lane = synth.add_lane(LaneKind::Notes);
        synth.lanes[lane].add_clip(0, 4.0).events = vec![note(0)];
    }
    rig.app.song_edited(SongEdit::Final, 0);
    let key = ClipKey {
        channel: 2,
        lane: 2,
        id: 1,
    };
    rig.app.navigate(Nav::EditClip(key));
    rig.slide_split();
    rig.app.preview = Some(key);
    rig.app.navigate(Nav::Show(View::Playlist));
    let on = |rig: &Rig| {
        let roll = &rig.app.slots.piano_roll.rolls[&2];
        (roll.lane, roll.clip_id, rig.app.preview.map(|k| k.lane))
    };
    // Its clips go, then the track.
    rig.app.track_op(2, 1, TrackOp::DeleteTrack, 0);
    rig.answer(true);
    assert_eq!(lanes(&rig, 2).len(), 2);
    assert_eq!(on(&rig), (1, Some(1), Some(1)));
    rig.frame(undo());
    assert_eq!(on(&rig), (2, Some(1), Some(2)));
    rig.frame(undo());
    assert_eq!(on(&rig), (2, Some(1), Some(2)));
    assert_eq!(rig.app.preview, Some(key));
}

#[test]
fn undoing_an_add_track_lets_go_of_what_was_on_the_new_track() {
    // Regression: the roll and the selection stayed on the track that was gone.
    let mut rig = three_channels();
    rig.app.track_op(2, 0, TrackOp::AddTrack, 0);
    // A clip on it, kept out of the history: the undo goes straight back to before the track.
    rig.app.song.channels[2].lanes[1].add_clip(0, 4.0);
    (rig.app.slots.piano_roll.rolls).insert(2, PianoRoll::new(2, 1, Some(1)));
    rig.app.main_screen.clips.selected = Some(ClipRef {
        channel: 2,
        lane: 1,
        id: 1,
    });
    rig.frame(undo());
    assert_eq!(lanes(&rig, 2), [LaneKind::Notes]);
    assert!(!rig.app.slots.piano_roll.rolls.contains_key(&2));
    assert_eq!(rig.app.main_screen.clips.selected, None);
}

#[test]
fn a_drum_editor_on_a_channel_that_moves_keeps_its_clip_and_views() {
    let mut rig = Rig::new();
    rig.app
        .add_channel(ChannelKind::Synth(SynthKind::MiniSynth), 0);
    rig.app.add_default_drums(0);
    for start in [0, 512] {
        rig.app.song.channels[2].lanes[0].add_clip(start, 4.0);
    }
    rig.app.song_edited(SongEdit::Final, 0);
    rig.app.navigate(Nav::EditClip(ClipKey {
        channel: 2,
        lane: 0,
        id: 2,
    }));
    rig.slide_split();
    rig.frame(vec![]);
    let step = |rig: &Rig, c: usize| {
        let step = &rig.app.slots.drum_editor.step;
        (step.clip, step.views.contains_key(&(c, 0, Some(2))))
    };
    assert_eq!(step(&rig, 2), (Some(2), true));
    rig.app.track_op(1, 0, TrackOp::DeleteChannel, 0);
    rig.answer(true);
    assert_eq!(rig.app.nav.view, View::DrumEditor(1));
    assert_eq!(step(&rig, 1), (Some(2), true));
}

#[test]
fn a_drum_editor_on_a_channel_that_moves_keeps_its_step_selection() {
    // Regression: the selection's clip key was not re-keyed, so the next frame dropped the selection.
    let mut rig = Rig::new();
    rig.app.add_default_drums(0);
    rig.edit(1);
    rig.slide_split();
    let hit = rig.step(0, 0);
    rig.click(hit);
    // A long press selects it (the rig's frames are 0.1 s apart).
    rig.press(hit);
    for _ in 0..3 {
        rig.frame(vec![]);
    }
    rig.release(hit);
    rig.frame(vec![]);
    assert!(rig.painted("Copy"), "selected");
    rig.app.insert_bare_channel(1, ChannelKind::Instrument, 0);
    assert_eq!(rig.app.nav.view, View::DrumEditor(2));
    // The new channel took the keyboard: the drums' track current again keeps its editor open.
    rig.app.selected_channel = 2;
    rig.frame(vec![]);
    rig.frame(vec![]);
    assert!(rig.painted("Copy"), "still selected");
}

#[test]
fn escape_closes_a_screen_popup_before_the_overlay_under_it() {
    // Regression: Escape went to Keyboard settings, under the track menu.
    let mut rig = Rig::with_drums_and_synth();
    rig.app.navigate(Nav::Open(Overlay::KeyboardSettings));
    rig.frame(vec![]);
    rig.secondary_click(header(1));
    rig.frame(vec![]);
    assert!(rig.app.main_screen.track_menu.is_some());
    rig.frame(escape());
    assert_eq!(rig.app.main_screen.track_menu, None);
    assert_eq!(rig.overlays(), [Overlay::KeyboardSettings]);
    // The next is the playlist's, which holds the keys under the strip (the focused control first): it
    // deselects, and the strip stays. With the focus on the bar, out of the grids, Escape closes the strip.
    rig.frame(escape());
    assert_eq!(rig.overlays(), [Overlay::KeyboardSettings]);
    crate::ui::focus::focus(&rig.ctx, egui::Id::new(crate::ui::focus::TRANSPORT));
    rig.frame(vec![]);
    rig.frame(escape());
    assert_eq!(rig.overlays(), []);
}

#[test]
fn an_undo_that_moves_channels_or_tracks_drops_the_open_question() {
    // Regression: after two undos `Delete channel?` on X named Y. (Ids are no longer reused: X took id 3 from the
    // HEAD@354 counter, and Y is back with its own 2; the question goes all the same.)
    let mut rig = Rig::with_drums_and_synth();
    rig.track_menu(2, DELETE_CHANNEL);
    rig.answer(true);
    rig.app.add_channel(ChannelKind::EffectTrack, 0);
    let ids = |rig: &Rig| crate::app::tracks::channel_ids(&rig.app.song);
    assert_eq!(ids(&rig), [0, 1, 3]);
    rig.track_menu(2, DELETE_CHANNEL);
    rig.undo_under_modal();
    assert_eq!(rig.overlays(), []);
    rig.frame(undo());
    assert_eq!(names(&rig), ["MASTER", "Drums", "MiniSynth"]);
    assert_eq!(ids(&rig), [0, 1, 2]);
    // A track's question goes the same way.
    rig.track_menu(2, ADD_TRACK);
    rig.app.song.channels[2].lanes[1].add_clip(0, 4.0);
    rig.app.song_edited(SongEdit::Final, 0);
    rig.track_menu(3, DELETE_TRACK);
    assert_eq!(rig.message(), ["Delete empty tracks?"]);
    for _ in 0..3 {
        rig.undo_under_modal();
    }
    assert_eq!(lanes(&rig, 2), [LaneKind::Notes]);
    assert_eq!(rig.overlays(), []);
}

#[test]
fn a_track_that_has_clips_again_when_the_question_is_answered_stays() {
    // Regression: Ctrl+Z under the open question brought the clips back, and Yes deleted them with the track.
    let mut rig = Rig::with_drums_and_synth();
    rig.track_menu(2, ADD_TRACK);
    rig.app.song.channels[2].lanes[1].add_clip(0, 4.0);
    rig.app.song_edited(SongEdit::Final, 0);
    rig.track_menu(3, DELETE_TRACK);
    assert_eq!(rig.message(), ["Delete empty tracks?"]);
    rig.undo_under_modal();
    assert_eq!(rig.app.song.channels[2].lanes[1].clips.len(), 1);
    rig.answer(true);
    assert_eq!(rig.app.song.channels[2].lanes[1].clips.len(), 1);
}

// ---- Load Drumset / Save Drumset ----

use super::rack_wiring::{dialog_save, picker_row};
use crate::project_files::mem::MemFiles;
use crate::project_files::{LOAD_FAILED, PresetKind, ProjectFiles};
use crate::song::DrumKit;
use crate::ui::rack::PresetTarget;

const LOAD_DRUMSET: (usize, usize) = (1, 0);
const SAVE_DRUMSET: (usize, usize) = (1, 1);

/// [`Rig::with_drums_and_synth`] on the web's files (a save downloads under the file name and the session keeps a
/// copy), and a handle on them.
fn drums_on_the_web() -> (Rig, MemFiles) {
    let mut rig = Rig::with_drums_and_synth();
    let mem = MemFiles::default();
    mem.0.borrow_mut().names_only = true;
    rig.app.files = ProjectFiles::with_provider(Box::new(mem.clone()));
    (rig, mem)
}

fn kit(rig: &Rig) -> DrumKit {
    rig.app.song.channels[1].drum_kit.clone().unwrap()
}

fn hit(pad: u8) -> SongEvent {
    SongEvent::Drum {
        pos: 0,
        len_beats: 0.25,
        pad,
        velocity: SongEvent::DEFAULT_VELOCITY,
        pan: SongEvent::DEFAULT_PAN,
        value: SongEvent::DEFAULT_PITCH,
        repeat: 0,
    }
}

#[test]
fn save_drumset_then_load_drumset_brings_the_kit_back_in_one_undo_step() {
    let (mut rig, mem) = drums_on_the_web();
    let menu: Vec<_> = crate::ui::track_menu::track_menu(&rig.app.song, 1, 0)
        .into_iter()
        .filter(|i| i.col == 1 && i.row < 2)
        .map(|i| (i.label.into_owned(), i.disabled))
        .collect();
    assert_eq!(
        menu,
        [
            ("Load Drumset".to_owned(), false),
            ("Save Drumset".to_owned(), false)
        ]
    );
    // A kit of its own: a pad renamed, moved and sent to a channel, and a clone.
    let edit = rig.app.song.channels[1].drum_kit.as_mut().unwrap();
    edit.slots[0].name = "Mine".into();
    edit.slots[0].params[0] = 0.3;
    (
        edit.slots[0].file.output_bus,
        edit.slots[0].file.send_target,
    ) = (1, 2);
    let copy = edit.slots[2].clone();
    edit.insert_slot(3, copy);
    rig.app.song_edited(SongEdit::Final, 0);
    rig.frame(vec![]);
    let saved = kit(&rig);
    let song = rig.app.song.clone();
    rig.take();

    // `Save Drumset`: the dialog of that title, proposing the channel's name; `SAVE` writes My Drumsets/Drums.tmpl.
    rig.track_menu(1, SAVE_DRUMSET);
    assert_eq!(
        rig.overlays(),
        [Overlay::PresetDialog {
            name: "Drums".into(),
            kind: PresetKind::Drumset,
        }]
    );
    rig.frame(vec![]);
    assert!(rig.color_of("Save Drumset").is_some(), "the title is drawn");
    assert!(rig.color_of("Drumset (.tmpl)").is_some(), "the file type");
    dialog_save(&mut rig);
    rig.frame(vec![]);
    assert_eq!(rig.overlays(), []);
    let path = std::path::Path::new("My Drumsets/Drums.tmpl");
    assert_eq!(mem.0.borrow().saves, [(path.to_owned(), false)]);
    let file = crate::flm::read_drumset(&rig.app.files.read_user_file(path).unwrap()).unwrap();
    assert_eq!(file, saved);
    // No song edit, nothing for the audio.
    assert_eq!(rig.app.song, song);
    assert_eq!(rig.take(), Vec::<String>::new());

    // Another kit: two pads, and a hit on pad 5, past them.
    let ch = &mut rig.app.song.channels[1];
    ch.drum_kit.as_mut().unwrap().slots.truncate(2);
    ch.lanes[0].add_clip(0, 4.0).events.push(hit(5));
    rig.app.song_edited(SongEdit::Final, 0);
    rig.frame(vec![]);
    rig.take();
    let before = rig.app.song.clone();

    // `Load Drumset`: the `Drumsets` picker; `My Files`, then the kit.
    rig.track_menu(1, LOAD_DRUMSET);
    let picker = Overlay::PresetPicker {
        channel: 1,
        target: PresetTarget::Drumset,
    };
    assert_eq!(rig.overlays(), [picker]);
    picker_row(&mut rig, 0);
    assert!(rig.color_of("Drumsets").is_some(), "the picker's title");
    picker_row(&mut rig, 0);
    assert_eq!(rig.overlays(), []);
    assert_eq!(rig.take(), ["ReplaceSong adopting no-resync"]);
    // The saved kit, its slots with new ids after the channel's last (9): ids are never reused. The send
    // names a channel of the song it was saved from: the pad plays into its channel.
    let loaded = kit(&rig);
    let ids: Vec<u32> = loaded.slots.iter().map(|s| s.file.id).collect();
    assert_eq!(ids, (10..=18).collect::<Vec<_>>());
    assert_eq!(loaded.file.last_slot_id, 18);
    let routing = |kit: &DrumKit| (kit.slots[0].file.output_bus, kit.slots[0].file.send_target);
    assert_eq!(routing(&loaded), (-1, -1));
    let unnumbered = |kit: &DrumKit| {
        let mut slots = kit.slots.clone();
        for s in &mut slots {
            (s.file.id, s.file.output_bus, s.file.send_target) = (0, -1, -1);
        }
        slots
    };
    assert_eq!(unnumbered(&loaded), unnumbered(&saved));
    // The hits stay, the one on pad 5 too.
    assert_eq!(rig.app.song.channels[1].lanes, before.channels[1].lanes);
    // One undo step. The pad ids stay taken: the kit's last id is not undone, so none comes back.
    rig.frame(undo());
    let mut want = before.channels.clone();
    let last = loaded.file.last_slot_id;
    want[1].drum_kit.as_mut().unwrap().file.last_slot_id = last;
    assert_eq!(rig.app.song.channels, want);
    rig.frame(redo());
    assert_eq!(kit(&rig), loaded);
}

#[test]
fn closing_the_drumset_picker_or_a_kit_that_does_not_read_changes_nothing() {
    let (mut rig, _) = drums_on_the_web();
    let path = std::path::Path::new("My Drumsets/Broken.tmpl");
    rig.app
        .files
        .keep_user_file(path, b"LPMTPADS\x02\0\0\0\0\0".to_vec());
    // A kit of no pads (kits normally hold 4–14), listed after it.
    let empty = std::path::Path::new("My Drumsets/Empty.tmpl");
    rig.app.files.keep_user_file(empty, b"LPMT".to_vec());
    rig.frame(vec![]);
    rig.take();
    let song = rig.app.song.clone();
    // Escape closes the picker.
    rig.track_menu(1, LOAD_DRUMSET);
    picker_row(&mut rig, 0);
    rig.frame(vec![egui::Event::Key {
        key: egui::Key::Escape,
        physical_key: None,
        pressed: true,
        repeat: false,
        modifiers: Default::default(),
    }]);
    rig.frame(vec![]);
    assert_eq!(rig.overlays(), []);
    assert_eq!(rig.app.song, song);
    // A PADS shorter than 22: the load error, and no edit.
    rig.track_menu(1, LOAD_DRUMSET);
    picker_row(&mut rig, 0);
    picker_row(&mut rig, 0);
    assert_eq!(rig.message()[0], LOAD_FAILED);
    assert_eq!(rig.app.song, song);
    assert_eq!(rig.take(), Vec::<String>::new());
    rig.answer(true);
    // The empty kit loads nothing, and says nothing.
    rig.track_menu(1, LOAD_DRUMSET);
    picker_row(&mut rig, 0);
    picker_row(&mut rig, 1);
    assert_eq!(rig.overlays(), []);
    assert_eq!(rig.app.song, song);
    assert_eq!(rig.take(), Vec::<String>::new());
    // No undo step: an undo takes back the synth's add, the edit before.
    rig.frame(vec![]);
    rig.frame(undo());
    assert_eq!(names(&rig), ["MASTER", "Drums"]);
}

// ---- A new Drums channel ----

/// The pads of a new Drums channel: one empty pad, id 1.
fn one_empty_pad() -> DrumKit {
    let mut kit = DrumKit::default();
    kit.insert_slot(0, crate::song::SamplerSlot::default());
    kit
}

#[test]
fn a_new_drums_channel_is_drum_with_one_empty_pad_and_the_drumset_picker_whose_close_keeps_it() {
    let mut rig = Rig::new();
    rig.app.insert_channel(1, ChannelKind::Drums, 0);
    rig.frame(vec![]);
    assert_eq!(names(&rig), ["MASTER", "Drum"]);
    assert_eq!(lanes(&rig, 1), [LaneKind::Drums]);
    assert_eq!(
        rig.app.song.channels[1].lanes[0].grid,
        crate::song::DrumGrid::Steps
    );
    assert_eq!(kit(&rig), one_empty_pad());
    assert_eq!(rig.app.selected_channel, 1);
    let picker = Overlay::PresetPicker {
        channel: 1,
        target: PresetTarget::Drumset,
    };
    assert_eq!(rig.overlays(), [picker]);
    picker_row(&mut rig, 0);
    assert!(rig.color_of("Drumsets").is_some(), "the picker's title");
    // Closing it loads nothing: the one empty pad stays.
    rig.frame(vec![egui::Event::Key {
        key: egui::Key::Escape,
        physical_key: None,
        pressed: true,
        repeat: false,
        modifiers: Default::default(),
    }]);
    rig.frame(vec![]);
    assert_eq!(rig.overlays(), []);
    assert_eq!(kit(&rig), one_empty_pad());
    // A second one is numbered as any taken name is.
    rig.app.insert_channel(2, ChannelKind::Drums, 0);
    assert_eq!(names(&rig), ["MASTER", "Drum", "Drum 2"]);
}

#[test]
fn the_drumset_pickers_default_is_the_built_in_kit_in_one_undo_step() {
    // The picker's root offers `Default` (row 1), the eight built-in pads
    // (`DrumKit::default_kit`); our factory kits come after it.
    let mut rig = Rig::new();
    rig.app.insert_channel(1, ChannelKind::Drums, 0);
    for _ in 0..3 {
        rig.frame(vec![]);
    }
    assert!(rig.color_of("Default").is_some(), "listed at the root");
    picker_row(&mut rig, 1);
    rig.frame(vec![]);
    assert_eq!(rig.overlays(), []);
    // The built-in kit, its slots with new ids after the empty pad's (1): ids are never reused.
    let loaded = kit(&rig);
    let ids: Vec<u32> = loaded.slots.iter().map(|s| s.file.id).collect();
    assert_eq!(ids, (2..=9).collect::<Vec<_>>());
    let unnumbered = |kit: &DrumKit| {
        let mut slots = kit.slots.clone();
        for s in &mut slots {
            s.file.id = 0;
        }
        slots
    };
    assert_eq!(unnumbered(&loaded), unnumbered(&DrumKit::default_kit()));
    // The kit's last id is not undone, so no pad id comes back.
    rig.frame(undo());
    let mut want = one_empty_pad();
    want.file.last_slot_id = 9;
    assert_eq!(kit(&rig), want);
}

#[test]
fn the_playlists_plus_drums_is_drum_with_one_empty_pad_and_the_drumset_picker() {
    // The add-channel popup's `Drums` (column 0, row 1) goes through the same rule as Insert Channel's.
    let mut rig = Rig::new();
    let center = |rig: &Rig, id: Id| rig.ctx.read_response(id).unwrap().rect.center();
    rig.click(center(&rig, Id::new("add_channel_button")));
    rig.frame(vec![]);
    rig.click(center(
        &rig,
        Id::new("add_channel_popup").with((0_usize, 1_usize)),
    ));
    assert_eq!(names(&rig), ["MASTER", "Drum"]);
    assert_eq!(kit(&rig), one_empty_pad());
    assert_eq!(rig.app.selected_channel, 1);
    let picker = Overlay::PresetPicker {
        channel: 1,
        target: PresetTarget::Drumset,
    };
    assert_eq!(rig.overlays(), [picker]);
}

#[test]
fn a_new_channel_past_the_id_limit_adds_nothing_and_says_why() {
    // The add-channel `+`, `Insert Channel` and `Clone Channel` alike.
    let mut rig = Rig::with_drums_and_synth();
    rig.app.song.file.next_channel_id = crate::song::CHANNEL_ID_LIMIT;
    let channels = rig.app.song.channels.len();
    let says_why = |rig: &mut Rig| {
        assert_eq!(rig.message(), [CHANNEL_LIMIT]);
        assert_eq!(rig.app.song.channels.len(), channels);
        rig.frame(escape());
        rig.frame(vec![]);
        assert_eq!(rig.overlays(), []);
    };
    rig.app.add_channel(ChannelKind::EffectTrack, 0);
    rig.frame(vec![]);
    says_why(&mut rig);
    rig.app.insert_channel(1, ChannelKind::Drums, 0);
    rig.frame(vec![]);
    says_why(&mut rig);
    rig.track_menu(2, CLONE);
    says_why(&mut rig);
}

#[test]
fn channel_settings_is_a_dialog_box_over_the_screen_dimmed_at_once() {
    // Dialog boxes do not animate.
    let mut rig = Rig::with_drums_and_synth();
    crate::ui::anim::enable(&rig.ctx);
    rig.track_menu(2, SETTINGS);
    let colors = rig.app.theme.colors;
    let screen = egui::Rect::from_min_size(egui::Pos2::ZERO, vec2(1170.0, 540.0));
    let fills: Vec<(egui::Rect, egui::Color32)> = (rig.out.shapes.iter())
        .filter_map(|c| match &c.shape {
            egui::Shape::Rect(r) => Some((r.rect, r.fill)),
            _ => None,
        })
        .collect();
    let dim = colors.background.to_color32().gamma_multiply(0.6);
    assert!(fills.contains(&(screen, dim)), "the dim");
    let menu_fill = colors.menu_fill.to_color32();
    // An instrument channel's: 20 U.
    let boxed = (fills.iter()).find(|(r, f)| *f == menu_fill && (r.width() - 640.0).abs() < 0.5);
    assert!(boxed.is_some(), "the 20 U box");
}

// Track reorder: a header held 300 ms then dragged vertically.

impl Rig {
    /// Holds row `row`'s header still for 300 ms (a frame every 0.1 s): the hold makes its track current.
    fn hold_header(&mut self, row: usize) {
        self.press(header(row));
        for _ in 0..3 {
            self.frame(vec![]);
        }
    }
}

/// Two frames on, the channel each playlist row shows, top to bottom, read from where each track row's header is (its
/// widget keeps the track row's id wherever it shows; `read_response` gives the pass before the last).
fn shown(rig: &mut Rig) -> Vec<usize> {
    rig.frame(vec![]);
    rig.frame(vec![]);
    let tracks = track_rows(&rig.app.song, false);
    let top = |i: usize| {
        let id = Id::new(("track_header", i));
        rig.ctx.read_response(id).expect("drawn").rect.top()
    };
    let mut rows: Vec<usize> = (0..tracks.len()).collect();
    rows.sort_by(|&a, &b| top(a).total_cmp(&top(b)));
    rows.iter().map(|&i| tracks[i].channel).collect()
}

const NAMES: [&str; 4] = ["MASTER", "Drums", "MiniSynth", "MiniSynth 2"];

#[test]
fn a_held_header_dragged_down_reorders_the_rows_live_and_its_drop_is_one_undo_step() {
    let mut rig = three_channels();
    rig.hold_header(1);
    assert_eq!(rig.app.selected_channel, 1, "the hold selects the track");
    // A row down, then another: the Drums swap with each row in turn; the song waits for the drop.
    rig.drag_to(header(2));
    assert_eq!(shown(&mut rig), [0, 2, 1, 3]);
    rig.drag_to(header(3));
    assert_eq!(shown(&mut rig), [0, 2, 3, 1]);
    assert_eq!(names(&rig), NAMES);
    assert!(!rig.app.history.can_undo());
    assert_eq!(
        rig.take(),
        Vec::<String>::new(),
        "no rebuild while it drags"
    );
    rig.release(header(3));
    let moved = ["MASTER", "MiniSynth", "MiniSynth 2", "Drums"];
    assert_eq!(names(&rig), moved);
    assert_eq!(shown(&mut rig), [0, 1, 2, 3]);
    assert_eq!(
        rig.app.selected_channel, 3,
        "the keyboard stays on the Drums"
    );
    assert_eq!(
        rig.take(),
        ["ReplaceSong adopting no-resync"],
        "one rebuild, at the drop"
    );
    // A save and load keep the order.
    let saved = crate::flm::read(&crate::flm::write(&rig.app.song).unwrap()).unwrap();
    let saved: Vec<_> = saved.channels.iter().map(|c| c.name.clone()).collect();
    assert_eq!(saved, moved);
    // One undo step.
    rig.frame(undo());
    assert_eq!(names(&rig), NAMES);
    assert_eq!(rig.app.selected_channel, 1);
    assert!(!rig.app.history.can_undo());
    rig.frame(redo());
    assert_eq!(names(&rig), moved);
}

#[test]
fn past_the_top_a_header_swaps_with_the_master_whose_own_move_rebuilds_nothing() {
    let mut rig = three_channels();
    // Down a row and back: no edit, no rebuild.
    rig.hold_header(1);
    rig.drag_to(header(2));
    assert_eq!(shown(&mut rig), [0, 2, 1, 3]);
    rig.drag_to(header(1));
    rig.release(header(1));
    assert_eq!(names(&rig), NAMES);
    assert!(!rig.app.history.can_undo());
    assert_eq!(rig.take(), Vec::<String>::new());
    // MiniSynth 2 past the top: the master takes its row.
    rig.hold_header(3);
    let above = header(0) - vec2(0.0, 40.0);
    rig.drag_to(above);
    assert_eq!(shown(&mut rig), [3, 1, 2, 0]);
    rig.release(above);
    let moved = ["MASTER", "MiniSynth 2", "Drums", "MiniSynth"];
    assert_eq!(names(&rig), moved);
    assert_eq!(rig.app.song.playlist(), [1, 2, 3, 0]);
    assert_eq!(rig.take(), ["ReplaceSong adopting no-resync"]);
    // The master alone up a row: no channel changes its index, so nothing is rebuilt; one undo step.
    rig.hold_header(3);
    rig.drag_to(header(2));
    rig.release(header(2));
    assert_eq!(
        (names(&rig), rig.app.song.playlist()),
        (moved.map(String::from).to_vec(), vec![1, 2, 0, 3])
    );
    assert_eq!(rig.take(), Vec::<String>::new());
    rig.frame(vec![]);
    rig.frame(undo());
    assert_eq!(rig.app.song.playlist(), [1, 2, 3, 0]);
    rig.frame(undo());
    assert_eq!(
        (names(&rig), rig.app.song.playlist()),
        (NAMES.map(String::from).to_vec(), vec![0, 1, 2, 3])
    );
    assert!(!rig.app.history.can_undo());
}

#[test]
fn a_moved_channel_takes_what_is_kept_for_it_and_its_held_note_cannot_stick() {
    let mut rig = three_channels();
    keep_state_of(&mut rig, 3);
    let before = state(&rig);
    rig.press(E4);
    assert_eq!(rig.take(), ["SetLoop 0 512", "NoteOn 3 52 108"]);
    // The drop's edit: MiniSynth 2 below the master.
    rig.app.set_playlist(&[0, 3, 1, 2], 0);
    assert_eq!(
        state(&rig),
        (
            1,
            vec![Overlay::Rack(1)],
            vec![(1, 1)],
            Some(1),
            Some(1),
            View::PianoRoll(1)
        )
    );
    // The rebuild drops the engine the note sounds in; the key's release goes to the channel's new index.
    assert_eq!(rig.take(), ["ReplaceSong adopting"]);
    rig.release(E4);
    assert_eq!(rig.take(), ["NoteOff 1 52"]);
    rig.frame(undo());
    assert_eq!(state(&rig), before);
}

#[test]
fn the_automation_editor_and_its_saved_view_follow_a_moved_channel() {
    use crate::song::AutomationTarget;
    let mut rig = three_channels();
    // A Send Level lane on the Drums for MiniSynth 2 (by its id), and a clip on it open in the automation editor.
    let id = rig.app.song.channels[3].id;
    let send_level = AutomationTarget {
        instance_id: 0,
        param: 0x2000 + id,
    };
    (rig.app.song.channels[1].add_automation_lane(send_level)).add_clip(0, 16.0);
    rig.app.song_edited(SongEdit::Final, 0);
    rig.app.navigate(Nav::EditClip(ClipKey {
        channel: 1,
        lane: 1,
        id: 1,
    }));
    rig.slide_split();
    rig.frame(vec![]);
    assert_eq!(rig.app.nav.view, View::AutomationEditor(1));
    let views = |rig: &Rig| -> Vec<_> {
        let views = &rig.app.slots.automation_editor.editor.views;
        views.keys().copied().collect()
    };
    assert_eq!(views(&rig), [(1, 1, 1)]);
    rig.app.set_playlist(&[0, 2, 3, 1], 0);
    rig.frame(vec![]);
    assert_eq!(rig.app.nav.view, View::AutomationEditor(3));
    assert_eq!(views(&rig), [(3, 1, 1)]);
    assert_eq!(rig.app.slots.automation_editor.clips.get(&3), Some(&(1, 1)));
    // The lane still names its send by id.
    let lane = &rig.app.song.channels[3].lanes[1];
    assert_eq!(lane.target, Some(send_level));
    assert_eq!(rig.app.song.channels[2].id, id);
}

#[test]
fn a_marquee_group_follows_its_clips_through_undos_made_while_the_playlist_is_away() {
    // Channels A (1) and B (2) with clips 1 and 2 each. Delete A, add C, select B's clips by a marquee,
    // open the mixer and undo twice: the group is B's clips again, now on channel 2, and its Delete takes them.
    let mut rig = Rig::new();
    for _ in 0..2 {
        rig.app
            .add_bare_channel(ChannelKind::Synth(SynthKind::MiniSynth), 0);
    }
    for channel in [1, 2] {
        let lane = &mut rig.app.song.channels[channel].lanes[0];
        lane.add_clip(0, 4.0);
        lane.add_clip(6 * crate::song::PPQ, 4.0);
    }
    rig.app.song_edited(SongEdit::Final, 0);
    rig.click(OUTSIDE);
    rig.app.delete_channel(1, 0);
    rig.app
        .add_bare_channel(ChannelKind::Synth(SynthKind::MiniSynth), 0);
    rig.frame(vec![]);
    // B's first clip (beats 0 … 4 of track 1, 38.4 points a beat) is the current clip: a tap on it.
    rig.click(pos2(48.0 + 2.0 * 38.4, header(1).y));
    let current = |channel| ClipRef {
        channel,
        lane: 0,
        id: 1,
    };
    assert_eq!(rig.current_clip(), Some(current(1)));
    // B is track 1 now: a long press right of its clips, dragged over both.
    let (from, to) = (pos2(800.0, header(1).y), pos2(100.0, header(1).y));
    rig.press(from);
    for _ in 0..4 {
        rig.frame(vec![]);
    }
    rig.drag_to(to);
    rig.release(to);
    let b = |channel| {
        [1, 2].map(|id| ClipRef {
            channel,
            lane: 0,
            id,
        })
    };
    assert_eq!(rig.app.main_screen.clips.group, b(1));
    rig.app.navigate(Nav::Show(View::Mixer));
    rig.frame(vec![]);
    rig.frame(undo());
    rig.frame(undo());
    rig.app.navigate(Nav::Show(View::Playlist));
    rig.frame(vec![]);
    assert_eq!(rig.app.song.channels.len(), 3);
    assert_eq!(rig.app.main_screen.clips.group, b(2));
    // The current clip went back with B too (not A's clip 1, now on channel 1 again).
    assert_eq!(rig.current_clip(), Some(current(2)));
    // Its row back by a long press on it, its Delete takes B's clips, not A's.
    let on = pos2(100.0, header(2).y);
    rig.press(on);
    for _ in 0..4 {
        rig.frame(vec![]);
    }
    rig.release(on);
    rig.frame(vec![]);
    rig.click_id(Id::new(("clip_button", 1_usize)));
    let clips = |c: usize| rig.app.song.channels[c].lanes[0].clips.len();
    assert_eq!((clips(1), clips(2)), (2, 0));
}

#[test]
fn an_undo_under_a_header_drag_ends_the_drag_whether_or_not_it_changes_the_channel_count() {
    // Regression: the drag kept its order of the song before the undo; a channel fewer made it index past the end
    // (a panic), the same count dropped the stale order on the song the undo left.
    for count_changes in [true, false] {
        let mut rig = three_channels();
        if count_changes {
            rig.app
                .add_bare_channel(ChannelKind::Synth(SynthKind::MiniSynth), 0);
        } else {
            rig.app.set_playlist(&[0, 2, 1, 3], 0);
        }
        rig.frame(vec![]);
        let undone = names(&rig);
        rig.hold_header(1);
        rig.drag_to(header(2));
        rig.frame(undo());
        // The finger goes on and is let go: nothing moves, and nothing breaks.
        rig.drag_to(header(3));
        rig.release(header(3));
        rig.frame(vec![]);
        let after = names(&rig);
        assert_ne!(after, undone, "{count_changes}: the undo");
        assert_eq!(rig.app.song.channels.len(), 4);
        assert_eq!(after, NAMES, "{count_changes}: no drop");
        assert!(!rig.app.history.can_undo(), "{count_changes}");
    }
}

#[test]
fn a_current_sub_track_follows_its_channel_when_channels_move() {
    let mut rig = three_channels();
    rig.app.song.channels[3].add_lane(LaneKind::Notes);
    rig.app.song_edited(SongEdit::Final, 0);
    rig.app.main_screen.clips.track = Some((3, 1));
    rig.app.set_playlist(&[0, 3, 1, 2], 0);
    assert_eq!(rig.app.main_screen.clips.track, Some((1, 1)));
    rig.frame(undo());
    assert_eq!(rig.app.main_screen.clips.track, Some((3, 1)));
    // Its channel deleted, it goes.
    rig.app.delete_channel(3, 0);
    assert_eq!(rig.app.main_screen.clips.track, None);
}

// ---- Import MIDI Tracks and Save as MIDI ----

use crate::midi::MidiMsg;
use crate::smf::import::{ImportOpts, import, scan};
use crate::smf::{Smf, SmfEvent, smf_of};
use crate::ui::midi_import::MidiImport;

/// `Import MIDI Tracks` and `Save as MIDI` of a notes channel; `Save as MIDI` of a drums channel.
const IMPORT_MIDI: (usize, usize) = (1, 0);
const SAVE_MIDI: (usize, usize) = (1, 1);
const SAVE_DRUMS_MIDI: (usize, usize) = (1, 4);

/// The MIDI import dialog's widget `what`.
fn import_id(what: &str) -> Id {
    Id::new("midi_import").with(what)
}

/// A format-1 file at 96 ticks a beat: per track, its name and its events as (tick, message).
fn smf(tracks: &[(&str, Vec<(u64, MidiMsg)>)]) -> Vec<u8> {
    let tracks = (tracks.iter())
        .map(|(name, events)| {
            let mut track = vec![(0, SmfEvent::Name(name.as_bytes().to_vec()))];
            track.extend(events.iter().map(|&(t, m)| (t, SmfEvent::Voice(m))));
            track
        })
        .collect();
    crate::smf::codec::write(&Smf {
        division: 96,
        tracks,
    })
}

/// Two notes on MIDI channel `ch` (0-based), a breath-control CC (#2, not one `Only supported` keeps) and a bend.
fn tune(ch: u8) -> Vec<(u64, MidiMsg)> {
    use MidiMsg::*;
    vec![
        (
            0,
            NoteOn {
                ch,
                key: 60,
                vel: 100,
            },
        ),
        (
            0,
            Cc {
                ch,
                cc: 2,
                value: 64,
            },
        ),
        (
            96,
            NoteOff {
                ch,
                key: 60,
                vel: 0,
            },
        ),
        (192, PitchBend { ch, value: 8192 }),
        (
            192,
            NoteOn {
                ch,
                key: 64,
                vel: 50,
            },
        ),
        (
            480,
            NoteOff {
                ch,
                key: 64,
                vel: 0,
            },
        ),
    ]
}

/// [`Rig::with_drums_and_synth`] with `bytes` as the file the open dialog picks, and a handle on the files.
fn picking(bytes: &[u8]) -> (Rig, MemFiles) {
    let mut rig = Rig::with_drums_and_synth();
    let mem = MemFiles::with_file("/in/Tune.mid", bytes);
    rig.app.files = ProjectFiles::with_provider(Box::new(mem.clone()));
    rig.frame(vec![]);
    rig.take();
    (rig, mem)
}

/// `song` with `bytes` imported into channel `current` as the track menu imports it.
fn imported(song: &Song, current: usize, bytes: &[u8], opts: ImportOpts) -> Song {
    let mut song = song.clone();
    import(
        &mut song,
        current,
        bytes,
        &opts,
        false,
        crate::content::factory(),
    )
    .expect("imports");
    song
}

#[test]
fn import_midi_tracks_of_one_midi_channel_goes_into_the_channel_at_once_in_one_undo_step() {
    let bytes = smf(&[("Lead", tune(4))]);
    let (mut rig, mem) = picking(&bytes);
    let before = rig.app.song.clone();
    rig.track_menu(2, IMPORT_MIDI);
    assert_eq!(mem.0.borrow().accepts, [&["mid", "midi"][..]]);
    // No dialog: Split off, automation on, Only supported off, into the channel whose menu it is.
    assert_eq!(rig.overlays(), []);
    let opts = ImportOpts {
        channels: 1 << 4,
        ..ImportOpts::ONE_CHANNEL
    };
    let want = imported(&before, 2, &bytes, opts);
    assert_eq!(rig.app.song.channels, want.channels);
    let labels: Vec<&str> = (rig.app.song.channels[2].lanes.iter())
        .map(|l| l.label.as_str())
        .collect();
    assert_eq!(
        labels[1..],
        ["Lead", "CHN #4 : Pitch Wheel", "CHN #4 : MIDI CC #2"]
    );
    assert_eq!(rig.app.song.tempo_bpm, before.tempo_bpm);
    assert_eq!(rig.take(), ["ReplaceSong adopting"]);
    // One undo step.
    rig.frame(undo());
    assert_eq!(rig.app.song.channels, before.channels);
    rig.frame(redo());
    assert_eq!(rig.app.song.channels, want.channels);
}

#[test]
fn import_midi_tracks_of_a_file_with_no_channel_does_nothing() {
    use MidiMsg::*;
    // A program change and a note-off alone make no channel.
    let bytes = smf(&[(
        "Empty",
        vec![
            (0, Program { ch: 0, program: 3 }),
            (
                9,
                NoteOff {
                    ch: 0,
                    key: 60,
                    vel: 0,
                },
            ),
        ],
    )]);
    let (mut rig, _) = picking(&bytes);
    let before = rig.app.song.clone();
    rig.track_menu(2, IMPORT_MIDI);
    assert_eq!(rig.overlays(), []);
    assert_eq!(rig.app.song, before);
    assert_eq!(rig.take(), Vec::<String>::new());
}

#[test]
fn a_file_that_does_not_read_shows_the_midi_load_error() {
    let (mut rig, _) = picking(b"RIFF....");
    let before = rig.app.song.clone();
    rig.track_menu(2, IMPORT_MIDI);
    assert_eq!(rig.message()[0], crate::smf::import::LOAD_FAILED);
    assert_eq!(rig.app.song, before);
}

#[test]
fn several_midi_channels_open_the_dialog_whose_import_makes_a_channel_each_in_one_undo_step() {
    let bytes = smf(&[("Lead", tune(4)), ("Bass", tune(1)), ("Kit", tune(9))]);
    let (mut rig, _) = picking(&bytes);
    let before = rig.app.song.clone();
    rig.track_menu(2, IMPORT_MIDI);
    assert_eq!(rig.overlays(), [Overlay::MidiImport]);
    // Its contents from its second frame on, as every dialog box's (an egui area's first frame only sizes it).
    rig.frame(vec![]);
    assert!(rig.painted("MIDI Import:"));
    assert!(rig.painted("Channel 10: Drums"));
    assert_eq!(rig.app.song, before, "nothing yet");
    // MIDI channel 2 (Bass) off; the rest as they start: Split, automation and Only supported on.
    rig.click_id(import_id("channel").with(1_usize));
    rig.click_id(import_id("import"));
    rig.frame(vec![]);
    // One summary of the sound each new channel got.
    assert!(
        matches!(&rig.overlays()[..], [Overlay::Message { lines, .. }] if lines[0] == crate::project_files::STAND_INS && lines.len() == 3)
    );
    let opts = ImportOpts {
        split: true,
        automation: true,
        only_supported: true,
        channels: 1 << 4 | 1 << 9,
    };
    let want = imported(&before, 2, &bytes, opts);
    // The new channels' GUIDs are random.
    let same = |a: &[crate::song::Channel], b: &[crate::song::Channel]| {
        a.len() == b.len() && a.iter().zip(b).all(|(a, b)| a.same_edit(b))
    };
    assert!(same(&rig.app.song.channels, &want.channels));
    assert_eq!(
        names(&rig)[3..],
        [
            "Lead (MIDI Channel 4: Acoustic Grand Piano)",
            "Kit (MIDI Channel 9: Drums)"
        ]
    );
    rig.frame(vec![]);
    let ok = rig.text_rect("OK").center();
    rig.click(ok);
    rig.frame(undo());
    assert_eq!(rig.app.song.channels, before.channels);
}

#[test]
fn split_off_imports_into_the_channel_and_what_the_app_keeps_by_track_follows() {
    let bytes = smf(&[("Lead", tune(4)), ("Bass", tune(1))]);
    let (mut rig, _) = picking(&bytes);
    // The synth has an automation track (Level), its clip held by the automation editor.
    let synth = &mut rig.app.song.channels[2];
    let level = crate::song::AutomationTarget {
        instance_id: 0,
        param: 0,
    };
    synth.add_automation_lane(level).add_clip(0, 4.0);
    rig.app.song_edited(SongEdit::Final, 0);
    rig.frame(vec![]);
    rig.app.slots.automation_editor.clips.insert(2, (1, 1));
    let before = rig.app.song.clone();
    rig.track_menu(2, IMPORT_MIDI);
    rig.click_id(import_id("Split MIDI channels"));
    rig.click_id(import_id("import"));
    rig.frame(vec![]);
    let opts = ImportOpts {
        split: false,
        automation: true,
        only_supported: true,
        channels: 1 << 4 | 1 << 1,
    };
    let want = imported(&before, 2, &bytes, opts);
    assert_eq!(rig.app.song.channels, want.channels);
    assert_eq!(names(&rig), ["MASTER", "Drums", "MiniSynth"]);
    // Two notes tracks and two pitch-wheel tracks, the Level one among the automation in file order: it is track 3 now.
    let labels: Vec<&str> = (rig.app.song.channels[2].lanes.iter())
        .map(|l| l.label.as_str())
        .collect();
    assert_eq!(labels[1..3], ["Lead", "Bass"]);
    assert_eq!(rig.app.song.channels[2].lanes[3].target, Some(level));
    assert_eq!(rig.app.slots.automation_editor.clips[&2], (3, 1));
    rig.frame(undo());
    assert_eq!(rig.app.song.channels, before.channels);
    assert_eq!(rig.app.slots.automation_editor.clips[&2], (1, 1));
}

#[test]
fn cancel_closes_the_dialog_and_imports_nothing() {
    let bytes = smf(&[("Lead", tune(4)), ("Bass", tune(1))]);
    let (mut rig, mem) = picking(&bytes);
    let before = rig.app.song.clone();
    rig.track_menu(2, IMPORT_MIDI);
    // A modal dialog: `Ctrl+O` asks for no file under it.
    rig.frame(vec![egui::Event::Key {
        key: egui::Key::O,
        physical_key: None,
        pressed: true,
        repeat: false,
        modifiers: egui::Modifiers::COMMAND,
    }]);
    assert_eq!(mem.0.borrow().accepts.len(), 1);
    rig.click_id(import_id("cancel"));
    rig.frame(vec![]);
    assert_eq!(rig.overlays(), []);
    assert_eq!(rig.app.song, before);
    assert_eq!(rig.take(), Vec::<String>::new());
}

/// The file the last save wrote, and its path.
fn saved(mem: &MemFiles) -> (std::path::PathBuf, Vec<u8>) {
    let path = mem.0.borrow().saves.last().expect("a save").0.clone();
    let bytes = mem.0.borrow().files[&path].clone();
    (path, bytes)
}

/// The MIDI channels of the file's note events.
fn note_channels(bytes: &[u8]) -> Vec<u8> {
    let smf = crate::smf::codec::read(bytes).expect("reads");
    let mut channels: Vec<u8> = (smf.tracks.iter().flatten())
        .filter_map(|(_, e)| match e {
            SmfEvent::Voice(MidiMsg::NoteOn { ch, .. }) => Some(*ch),
            _ => None,
        })
        .collect();
    channels.dedup();
    channels
}

#[test]
fn save_as_midi_writes_only_that_channel_on_midi_channel_1_into_my_midi() {
    let (mut rig, mem) = picking(b"");
    // Notes on the drums (pad 2) and the synth (key 69, beat 1).
    rig.app.song.channels[1].lanes[0]
        .add_clip(0, 4.0)
        .events
        .push(hit(2));
    rig.app.song.channels[2].lanes[0]
        .add_clip(0, 4.0)
        .events
        .push(note(crate::song::PPQ));
    rig.app.song_edited(SongEdit::Final, 0);
    rig.frame(vec![]);
    let song = rig.app.song.clone();
    rig.take();
    // The dialog titled `Save as MIDI`, MIDI only, proposing the channel's name.
    rig.track_menu(2, SAVE_MIDI);
    assert_eq!(
        rig.overlays(),
        [Overlay::SaveDialog {
            name: "MiniSynth".into()
        }]
    );
    rig.frame(vec![]);
    assert!(rig.painted("Save as MIDI"));
    assert!(rig.painted("MIDI Sequence"));
    assert!(!rig.painted("FLM"), "no format row");
    dialog_save(&mut rig);
    rig.frame(vec![]);
    assert_eq!(rig.overlays(), []);
    let (path, bytes) = saved(&mem);
    assert_eq!(path, std::path::Path::new("My MIDI/MiniSynth.mid"));
    assert_eq!(bytes, smf_of(&song, Some(2)).unwrap());
    assert_eq!(note_channels(&bytes), [0]);
    assert!(rig.painted("Saved!"));
    // No song edit.
    assert_eq!(rig.app.song, song);
    assert_eq!(rig.take(), Vec::<String>::new());
    // A drums channel's drum lane goes on MIDI channel 10.
    rig.track_menu(1, SAVE_DRUMS_MIDI);
    assert_eq!(
        rig.overlays(),
        [Overlay::SaveDialog {
            name: "Drums".into()
        }]
    );
    dialog_save(&mut rig);
    rig.frame(vec![]);
    let (path, bytes) = saved(&mem);
    assert_eq!(path, std::path::Path::new("My MIDI/Drums.mid"));
    assert_eq!(bytes, smf_of(&song, Some(1)).unwrap());
    assert_eq!(note_channels(&bytes), [9]);
    // The song's own Save dialog afterwards has its format row again, and its MIDI is the whole song's.
    rig.app.files.save_as(&rig.app.song);
    rig.frame(vec![]);
    rig.frame(vec![]);
    assert!(rig.painted("FLM"));
    assert!(!rig.painted("Save as MIDI"));
    let l = crate::ui::save_dialog::layout(
        egui::Rect::from_min_size(egui::Pos2::ZERO, vec2(1170.0, 540.0)),
        &crate::metrics::Metrics::from_pixels_per_point(1.0, crate::metrics::DEFAULT_UI_SCALE),
    );
    rig.click(l.formats[3].center());
    rig.click(l.save.center());
    rig.frame(vec![]);
    let (path, bytes) = saved(&mem);
    assert_eq!(path.parent(), Some(std::path::Path::new("My MIDI")));
    assert_eq!(bytes, smf_of(&song, None).unwrap());
    assert_eq!(note_channels(&bytes), [9, 1]);
}

#[test]
fn save_as_midi_of_a_channel_with_nothing_to_write_says_so() {
    let (mut rig, mem) = picking(b"");
    rig.track_menu(2, SAVE_MIDI);
    dialog_save(&mut rig);
    rig.frame(vec![]);
    assert_eq!(rig.message(), [crate::project_files::export::NO_DATA]);
    assert_eq!(mem.0.borrow().saves, []);
}

// ---- Undo after a step that changes a kept track, the one-channel path, gone channels, names ----

#[test]
fn an_undo_of_a_step_that_extends_one_automation_track_and_adds_two_keeps_the_editor_on_it() {
    let mut rig = Rig::with_drums_and_synth();
    let t = |param| crate::song::AutomationTarget {
        instance_id: 0,
        param,
    };
    rig.app.song.channels[2]
        .add_automation_lane(t(0))
        .add_clip(0, 4.0);
    rig.app.song_edited(SongEdit::Final, 0);
    rig.frame(vec![]);
    // One step, as one automation-recording take: Level's track gets a clip, two new tracks follow it.
    let ch = &mut rig.app.song.channels[2];
    ch.lanes[1].add_clip(16 * crate::song::PPQ, 4.0);
    ch.add_automation_lane(t(1))
        .add_clip(16 * crate::song::PPQ, 4.0);
    ch.add_automation_lane(t(2))
        .add_clip(16 * crate::song::PPQ, 4.0);
    rig.app.song_edited(SongEdit::Final, 0);
    rig.frame(vec![]);
    assert_eq!(rig.app.song.channels[2].lanes[1].target, Some(t(0)));
    // The automation editor holds Level's first clip (track 1, clip 1).
    rig.app.slots.automation_editor.clips.insert(2, (1, 1));
    rig.frame(undo());
    assert_eq!(rig.app.song.channels[2].lanes.len(), 2);
    assert_eq!(
        rig.app.slots.automation_editor.clips.get(&2),
        Some(&(1, 1)),
        "undo"
    );
    rig.frame(redo());
    assert_eq!(rig.app.song.channels[2].lanes.len(), 4);
    assert_eq!(
        rig.app.slots.automation_editor.clips.get(&2),
        Some(&(1, 1)),
        "redo"
    );
}

#[test]
fn the_one_channel_import_ignores_a_stray_note_off_on_another_midi_channel() {
    use MidiMsg::*;
    // Notes on channel 5 (index 4) only; a stray note-off of the same key on channel 1, after the real note-off (tick
    // 96) and before the next note (192): with every MIDI channel imported it would lengthen the note.
    let mut events = tune(4);
    events.push((
        150,
        NoteOff {
            ch: 0,
            key: 60,
            vel: 0,
        },
    ));
    events.sort_by_key(|e| e.0);
    let bytes = smf(&[("Lead", events)]);
    let (mut rig, _) = picking(&bytes);
    rig.track_menu(2, IMPORT_MIDI);
    assert_eq!(rig.overlays(), []);
    let lane = &rig.app.song.channels[2].lanes[1];
    let SongEvent::Note { len_beats, .. } = lane.clips[0].events[0] else {
        panic!("no note")
    };
    assert_eq!(len_beats, 1.0, "the stray note-off changed it");
}

#[test]
fn an_import_shows_the_hidden_tracks_of_its_channel() {
    let bytes = smf(&[("Lead", tune(4))]);
    let (mut rig, _) = picking(&bytes);
    let synth = &mut rig.app.song.channels[2];
    synth.add_lane(LaneKind::Notes);
    synth.zipped = true;
    rig.app.song_edited(SongEdit::Final, 0);
    rig.frame(vec![]);
    rig.track_menu(2, IMPORT_MIDI);
    assert_eq!(lanes(&rig, 2).len(), 3 + 2, "imported");
    assert!(!rig.app.song.channels[2].zipped);
}

/// [`picking`] with a third channel, a MiniSynth (3), whose track menu's `item` opens a dialog; then the channel goes
/// under it (as an undo would take it).
fn gone_under(bytes: &[u8], item: (usize, usize)) -> (Rig, MemFiles) {
    let (mut rig, mem) = picking(bytes);
    rig.app
        .add_bare_channel(ChannelKind::Synth(SynthKind::MiniSynth), 0);
    rig.frame(vec![]);
    rig.track_menu(3, item);
    assert_eq!(rig.overlays().len(), 1);
    rig.app.delete_channel(3, 0);
    assert_eq!(rig.app.song.channels.len(), 3);
    assert_eq!(rig.overlays().len(), 1, "the dialog stays");
    rig.frame(vec![]);
    (rig, mem)
}

#[test]
fn with_split_on_an_import_makes_its_channels_though_its_channel_went_under_the_dialog() {
    let bytes = smf(&[("Lead", tune(4)), ("Bass", tune(1))]);
    let (mut rig, _) = gone_under(&bytes, IMPORT_MIDI);
    let before = rig.app.song.clone();
    rig.click_id(import_id("import"));
    rig.frame(vec![]);
    assert!(
        matches!(&rig.overlays()[..], [Overlay::Message { lines, .. }] if lines[0] == crate::project_files::STAND_INS)
    );
    let want = imported(
        &before,
        0,
        &bytes,
        MidiImport::new(scan(&bytes).unwrap()).opts,
    );
    assert_eq!(
        names(&rig)[3..],
        [
            "Lead (MIDI Channel 4: Acoustic Grand Piano)",
            "Bass (MIDI Channel 1: Acoustic Grand Piano)"
        ]
    );
    assert_eq!(rig.app.song.channels.len(), want.channels.len());
    assert_eq!(rig.app.song.channels[..3], before.channels[..]);
}

#[test]
fn with_split_off_an_import_into_a_channel_gone_under_the_dialog_imports_nothing() {
    let bytes = smf(&[("Lead", tune(4)), ("Bass", tune(1))]);
    let (mut rig, _) = gone_under(&bytes, IMPORT_MIDI);
    let before = rig.app.song.clone();
    rig.click_id(import_id("Split MIDI channels"));
    rig.click_id(import_id("import"));
    rig.frame(vec![]);
    assert_eq!(rig.overlays(), []);
    assert_eq!(rig.app.song, before);
}

#[test]
fn save_as_midi_of_a_channel_gone_under_the_dialog_closes_it_and_saves_nothing() {
    let (mut rig, mem) = gone_under(b"", SAVE_MIDI);
    dialog_save(&mut rig);
    rig.frame(vec![]);
    assert_eq!(rig.overlays(), []);
    assert_eq!(mem.0.borrow().saves, []);
}

#[test]
fn save_as_midi_proposes_the_channel_name_made_a_file_name() {
    let (mut rig, mem) = picking(b"");
    let synth = &mut rig.app.song.channels[2];
    synth.name = "Lead (MIDI Channel 4: Close Grand)".into();
    synth.lanes[0].add_clip(0, 4.0).events.push(note(0));
    rig.app.song_edited(SongEdit::Final, 0);
    rig.frame(vec![]);
    rig.track_menu(2, SAVE_MIDI);
    let name = "Lead (MIDI Channel 4_ Close Grand)";
    assert_eq!(rig.overlays(), [Overlay::SaveDialog { name: name.into() }]);
    dialog_save(&mut rig);
    rig.frame(vec![]);
    assert_eq!(rig.overlays(), []);
    assert_eq!(
        saved(&mem).0,
        std::path::Path::new("My MIDI").join(format!("{name}.mid"))
    );
}

/// `Hide track`, column 0's last item, hides a track everywhere but in playback, one undo step: its row goes and the
/// rows below it move up, its notes are no ghost notes in another channel's roll, and the song sounds the same.
/// The current track moves off it, so paste goes elsewhere. Settings `Show hidden tracks` brings its row back, where its
/// menu reads `Show track`. An editor open on the track closes as it hides.
#[test]
fn hide_track_takes_its_row_and_ghost_notes_but_not_its_sound() {
    use crate::audio_engine::SongAudioEngine;
    use crate::theme::channel_color;
    use honeydew_audio::device::AudioRenderer;
    const HIDE_TRACK: (usize, usize) = (0, 6);
    let mut rig = Rig::with_drums_and_synth();
    rig.app
        .add_channel(ChannelKind::Synth(SynthKind::MiniSynth), 0);
    rig.app.song.channels[3].lanes[0]
        .add_clip(0, 4.0)
        .events
        .push(note(0));
    rig.app.song_edited(SongEdit::Final, 0);
    let rows = |rig: &Rig| -> Vec<usize> {
        let show = rig.app.settings.show_hidden_tracks;
        (track_rows(&rig.app.song, show).iter())
            .map(|t| t.channel)
            .collect()
    };
    let ghost = channel_color(rig.app.song.channels[3].hue, &rig.app.theme)
        .to_color32()
        .gamma_multiply(crate::ui::piano_roll::GHOST_ALPHA);
    // The synth's roll, open and closed again.
    let ghosts = |rig: &mut Rig| {
        rig.edit(2);
        rig.slide_split();
        let n = rig.rects(ghost).len();
        rig.app.navigate(Nav::Collapse);
        rig.frame(vec![]);
        n
    };
    let sound = |rig: &Rig| {
        let mut engine = SongAudioEngine::new(&rig.app.song, 48_000.0).unwrap();
        engine.play();
        let mut output = vec![0.0; 8192 * 2];
        engine.render(&mut output, 2);
        output
    };
    let before = sound(&rig);
    assert!(before.iter().any(|s| s.abs() > 0.01));
    assert_eq!(rows(&rig), [0, 1, 2, 3]);
    assert_eq!(ghosts(&mut rig), 1);
    // The bass clip tapped (its track current, the grid cursor on it) and copied.
    rig.click(pos2(48.0 + 2.0 * 38.4, header(3).y));
    rig.frame(key_tap(egui::Key::C, egui::Modifiers::COMMAND));
    assert_eq!(rig.app.selected_channel, 3);
    rig.track_menu(3, HIDE_TRACK);
    assert!(rig.app.song.channels[3].lanes[0].hidden);
    assert_eq!(rows(&rig), [0, 1, 2]);
    // The current track moves to the nearest one shown, where a paste lands: the hidden track takes nothing.
    assert_eq!(rig.app.selected_channel, 2);
    let clips = |rig: &Rig| [2, 3].map(|c| rig.app.song.channels[c].lanes[0].clips.len());
    let [synth, bass] = clips(&rig);
    rig.frame(key_tap(egui::Key::V, egui::Modifiers::COMMAND));
    assert_eq!(clips(&rig), [synth + 1, bass]);
    rig.frame(undo());
    assert_eq!(ghosts(&mut rig), 0);
    assert!(sound(&rig) == before, "a hidden track plays as before");
    rig.frame(undo());
    assert_eq!(rows(&rig), [0, 1, 2, 3]);
    rig.frame(redo());
    assert_eq!(rows(&rig), [0, 1, 2]);
    rig.app.settings.show_hidden_tracks = true;
    rig.frame(vec![]);
    assert_eq!(rows(&rig), [0, 1, 2, 3]);
    rig.track_menu(3, HIDE_TRACK);
    assert!(!rig.app.song.channels[3].lanes[0].hidden, "`Show track`");
    // Hidden under its open editor, the editor closes.
    rig.app.settings.show_hidden_tracks = false;
    rig.edit(3);
    rig.slide_split();
    assert_eq!(rig.app.nav.view, View::PianoRoll(3));
    rig.app.track_op(3, 0, TrackOp::ToggleHidden, 0);
    rig.frame(vec![]);
    assert_eq!(rig.app.nav.view, View::Playlist);
}
