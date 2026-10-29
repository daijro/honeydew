//! MIDI input in the app: live keys, pad notes and `CCLK` links. A fake device stands in for the controller: its
//! messages go straight into the link's queue, and no test opens a port.

use std::any::Any;
use std::cell::RefCell;
use std::rc::Rc;
use std::sync::mpsc::SyncSender;

use super::*;
use crate::app::midi::{BURST_SECS, Midi};
use crate::midi::device::{Message, MidiChoice, MidiLink};
use crate::midi::links;
use crate::song::DrumGrid;

/// The fake system: the ports it lists, and the sender of the last open (where the controller's messages go).
#[derive(Default)]
pub(super) struct Device {
    ports: RefCell<Vec<String>>,
    tx: RefCell<Option<SyncSender<Message>>>,
    opens: RefCell<usize>,
    lists: RefCell<usize>,
}

impl Device {
    /// A link to this device, as `MidiLink::platform` makes one.
    fn link(self: &Rc<Self>) -> MidiLink {
        let (opener, lister) = (self.clone(), self.clone());
        MidiLink::new(
            move |_, tx, _| {
                *opener.tx.borrow_mut() = Some(tx);
                *opener.opens.borrow_mut() += 1;
                Ok(Box::new(()) as Box<dyn Any>)
            },
            move || {
                *lister.lists.borrow_mut() += 1;
                lister.ports.borrow().clone()
            },
            None,
        )
    }
}

impl Rig {
    /// Plugs a fake keyboard into the app; the next frame opens it.
    pub(super) fn plug_midi(&mut self) -> Rc<Device> {
        let device = Rc::new(Device::default());
        device.ports.borrow_mut().push("Keys 20:0".to_owned());
        self.app.midi.link = device.link();
        self.frame(vec![]);
        assert_eq!(*device.opens.borrow(), 1, "opened at once natively");
        device
    }

    /// The controller sends `messages`; the next frame plays them.
    pub(super) fn midi(&mut self, device: &Device, messages: &[&[u8]]) {
        self.midi_from(device, "Keys", messages);
    }

    /// [`Self::midi`] from the device named `name`.
    fn midi_from(&mut self, device: &Device, name: &str, messages: &[&[u8]]) {
        for message in messages {
            let tx = device.tx.borrow();
            let message = (name.to_owned(), message.to_vec());
            tx.as_ref().expect("open").send(message).unwrap();
        }
        self.frame(vec![]);
    }
}

/// The undo steps there are (undone, then redone back).
fn undo_steps(rig: &mut Rig) -> usize {
    let history = &mut rig.app.history;
    let n = std::iter::from_fn(|| history.undo()).count();
    for _ in 0..n {
        history.redo();
    }
    n
}

/// [`Rig::with_synth`] with a second MiniSynth (2), channel 1 selected, a fake keyboard plugged in, and an empty log.
fn midi_rig() -> (Rig, Rc<Device>) {
    let mut rig = Rig::with_synth();
    rig.app
        .add_channel(ChannelKind::Synth(SynthKind::MiniSynth), 0);
    rig.app.selected_channel = 1;
    let device = rig.plug_midi();
    rig.take();
    (rig, device)
}

/// Links `cc` to the param `target` of channel `channel`, as `Link to MIDI CC` stores it.
fn link(rig: &mut Rig, channel: usize, cc: u8, target: AutomationTarget) {
    let raw = &mut rig.app.song.channels[channel].file.cc_links;
    links::link(raw, cc, target.instance_id, target.param);
}

const LEVEL: AutomationTarget = AutomationTarget {
    instance_id: 0,
    param: 0,
};
const PAN: AutomationTarget = AutomationTarget {
    instance_id: 0,
    param: 1,
};

#[test]
fn a_chord_plays_every_key_on_the_selected_channel_and_each_ends_on_its_own() {
    let (mut rig, keys) = midi_rig();
    rig.midi(
        &keys,
        &[&[0x90, 60, 100], &[0x91, 64, 90], &[0x9F, 67, 127]],
    );
    assert_eq!(
        rig.take(),
        ["NoteOn 1 60 100", "NoteOn 1 64 90", "NoteOn 1 67 127"],
        "omni: every MIDI channel plays"
    );
    rig.midi(&keys, &[&[0x80, 64, 0]]);
    assert_eq!(rig.take(), ["NoteOff 1 64"]);
    // A note-on with velocity 0 is a note-off.
    rig.midi(&keys, &[&[0x90, 60, 0], &[0x80, 67, 64]]);
    assert_eq!(rig.take(), ["NoteOff 1 60", "NoteOff 1 67"]);
    // Sysex, clock and malformed bytes do nothing.
    rig.midi(&keys, &[&[0xF0, 0x7E, 0xF7], &[0xF8], &[0x90, 60]]);
    assert_eq!(rig.take(), Vec::<String>::new());
}

#[test]
fn a_key_held_across_a_selection_change_ends_on_the_channel_it_began_on() {
    let (mut rig, keys) = midi_rig();
    rig.midi(&keys, &[&[0x90, 60, 100]]);
    rig.click(header(2));
    assert_eq!(rig.app.selected_channel, 2);
    rig.take();
    rig.midi(&keys, &[&[0x90, 62, 100], &[0x80, 60, 0], &[0x80, 62, 0]]);
    assert_eq!(rig.take(), ["NoteOn 2 62 100", "NoteOff 1 60"]);
    // 62 went on and off in one batch: its note-off waits for the tap gate.
    rig.frame(vec![]);
    assert_eq!(rig.take(), ["NoteOff 2 62"]);
}

#[test]
fn the_selected_channels_cc_pitch_bend_and_aftertouch_are_live_values_not_edits() {
    let (mut rig, keys) = midi_rig();
    let (song, undos) = (rig.app.song.clone(), undo_steps(&mut rig));
    // CC 7 is channel param 14 (`SetParam`, instance 0).
    rig.midi(&keys, &[&[0xB0, 7, 127]]);
    assert_eq!(rig.take(), ["SetParam 1 0 14 1.000"]);
    rig.midi(&keys, &[&[0xE0, 0x00, 0x40], &[0xD0, 127], &[0xB3, 64, 0]]);
    assert_eq!(
        rig.take(),
        [
            "LiveParam 1 4 0.500",
            "LiveParam 1 5 1.000",
            "SetParam 1 0 71 0.000"
        ]
    );
    // A program change is ignored live.
    rig.midi(&keys, &[&[0xC0, 5]]);
    assert_eq!(rig.take(), Vec::<String>::new());
    rig.next_frame_at(rig.time + 1.0);
    rig.frame(vec![]);
    assert!(rig.app.song == song, "no song edit");
    assert_eq!(undo_steps(&mut rig), undos);
}

#[test]
fn a_cc_linked_on_another_channel_moves_that_param_and_not_the_selected_cc_param_but_cc_1_moves_both()
 {
    let (mut rig, keys) = midi_rig();
    link(&mut rig, 2, 20, LEVEL);
    link(&mut rig, 2, 1, PAN);
    let pan = rig.app.song.channels[1].params.0[1];
    rig.midi(&keys, &[&[0xB0, 20, 0]]);
    assert_eq!(rig.take(), ["SetParam 2 0 0 0.000"]);
    assert_eq!(rig.app.song.channels[2].params.0[0], 0.0);
    // An unlinked CC still moves the selected channel's own param.
    rig.midi(&keys, &[&[0xB0, 21, 127]]);
    assert_eq!(rig.take(), ["SetParam 1 0 28 1.000"]);
    // The mod wheel moves its link and the selected channel's CC 1 param.
    rig.midi(&keys, &[&[0xB5, 1, 127]]);
    assert_eq!(rig.take(), ["SetParam 2 0 1 1.000", "SetParam 1 0 8 1.000"]);
    assert_eq!(rig.app.song.channels[2].params.0[1], 1.0);
    assert_eq!(rig.app.song.channels[1].params.0[1], pan, "not linked here");
}

#[test]
fn links_fire_with_the_master_selected_while_notes_and_its_cc_params_go_nowhere() {
    let mut rig = Rig::new();
    rig.click(OUTSIDE);
    assert_eq!(rig.app.selected_channel, 0);
    let keys = rig.plug_midi();
    link(&mut rig, 0, 20, LEVEL);
    rig.take();
    rig.midi(&keys, &[&[0x90, 60, 100], &[0xB0, 7, 100], &[0x80, 60, 0]]);
    assert_eq!(rig.take(), Vec::<String>::new());
    rig.midi(&keys, &[&[0xB0, 20, 127]]);
    assert_eq!(rig.take(), ["SetParam 0 0 0 1.000"]);
    assert_eq!(rig.app.song.channels[0].params.0[0], 1.0);
}

#[test]
fn a_cc_linked_to_a_drum_pad_moves_that_slots_param() {
    let (mut rig, keys) = midi_rig();
    rig.app.add_default_drums(0);
    rig.app.selected_channel = 1;
    let slot = rig.app.song.channels[3].drum_kit.as_ref().unwrap().slots[1]
        .file
        .id;
    let pan = AutomationTarget {
        instance_id: -(slot as i32),
        param: 1,
    };
    link(&mut rig, 3, 74, pan);
    rig.take();
    rig.midi(&keys, &[&[0xB0, 74, 127]]);
    assert_eq!(rig.take(), [format!("SetParam 3 -{slot} 1 1.000")]);
    let kit = rig.app.song.channels[3].drum_kit.as_ref().unwrap();
    assert_eq!(kit.slot_by_id(slot).unwrap().params[1], 1.0);
}

#[test]
fn linked_moves_save_no_undo_step_and_reach_the_undo_steps_once_their_burst_ends() {
    // A moved param sets the song-changed flag with no undo step; the burst only batches the carry.
    let (mut rig, keys) = midi_rig();
    link(&mut rig, 1, 20, LEVEL);
    rig.app.song_edited(SongEdit::Final, 0);
    rig.app.files.saved();
    let undos = undo_steps(&mut rig);
    // A burst: moves 0.1 s apart.
    for value in [10, 40, 70, 100] {
        rig.midi(&keys, &[&[0xB0, 20, value]]);
    }
    let last = rig.time;
    let moved = 100.0 / 127.0;
    assert_eq!(rig.app.song.channels[1].params.0[0], moved);
    assert!(rig.app.files.modified());
    rig.next_frame_at(last + BURST_SECS - 0.1);
    rig.frame(vec![]);
    assert!(rig.app.params_moved, "not in the steps yet");
    let delay = rig.out.viewport_output[&egui::ViewportId::ROOT].repaint_delay;
    assert!(delay.as_secs_f64() <= 0.1 + 1e-3, "{delay:?}");
    rig.next_frame_at(last + BURST_SECS + 0.01);
    rig.frame(vec![]);
    assert!(!rig.app.params_moved);
    assert_eq!(undo_steps(&mut rig), undos);
    // An undo takes back the link before them, and the level stays.
    rig.frame(z(egui::Modifiers::NONE));
    assert!(rig.app.song.channels[1].file.cc_links.is_empty());
    assert_eq!(rig.app.song.channels[1].params.0[0], moved);
}

#[test]
fn rec_records_a_linked_move_of_the_recent_control_as_it_does_a_hand_move() {
    use crate::song::points::{self, PointKey};
    let (mut rig, keys) = midi_rig();
    link(&mut rig, 1, 20, LEVEL);
    rig.app.recent = Some((rig.app.song.channels[1].id, LEVEL));
    rig.app.settings.count_in = false;
    rig.app.main_screen.armed = true;
    rig.frame(vec![]);
    rig.midi(&keys, &[&[0xB0, 20, 64]]);
    assert!(
        rig.app.main_screen.playing,
        "the move started the transport"
    );
    rig.midi(&keys, &[&[0xB0, 20, 32]]);
    let lanes = &rig.app.song.channels[1].lanes;
    let lane = lanes.iter().find(|l| l.target == Some(LEVEL));
    let clip = &lane.expect("a recorded lane").clips[0];
    let last = points::points(clip, PointKey::Automation).pop();
    assert_eq!(last.map(|p| p.value), Some(points::raw(32.0 / 127.0)));
}

/// `Set MIDI Note` learns in both drum modes: the step sequencer, or the free note-grid (`grid`).
fn set_midi_note_learns_the_next_key_as_one_undo_step_and_the_key_plays(grid: DrumGrid) {
    let (mut rig, keys) = midi_rig();
    rig.app.add_default_drums(0);
    rig.app.song.channels[3].lanes[0].grid = grid;
    // The engine plays the grid set here, so that the edit below changes only the slot.
    rig.app.rebuild(0);
    rig.edit(3);
    rig.slide_split();
    rig.frame(vec![]);
    let view = match grid {
        DrumGrid::Steps => "step_seq",
        DrumGrid::Notes => "drum_grid",
    };
    let row = |pad: usize| egui::Id::new(view).with(("row", pad));
    let header = rig.ctx.read_response(row(1)).expect("drawn").rect;
    rig.secondary_click(header.center());
    rig.frame(vec![]);
    // Pad 1 has no note: `Set MIDI Note` is row 5.
    rig.click_id(
        egui::Id::new("step_seq")
            .with("row menu")
            .with((0_usize, 5_usize)),
    );
    rig.take();
    let undos = undo_steps(&mut rig);
    let note = |rig: &Rig| {
        let kit = rig.app.song.channels[3].drum_kit.as_ref().unwrap();
        kit.slots[1].midi_note()
    };
    assert_eq!(note(&rig), None);
    rig.midi(&keys, &[&[0x90, 40, 100], &[0x90, 41, 100]]);
    assert_eq!(note(&rig), Some(40), "the first key only");
    // The edit rebuilds the engine, then the key plays: with its new pad, once the engine maps it.
    assert_eq!(
        rig.take(),
        [
            "ReplaceSong adopting no-resync",
            "NoteOn 3 40 100",
            "NoteOn 3 41 100"
        ]
    );
    assert_eq!(undo_steps(&mut rig), undos + 1);
    rig.midi(&keys, &[&[0x90, 42, 100]]);
    assert_eq!(note(&rig), Some(40), "learned once");
}

#[test]
fn set_midi_note_learns_in_the_step_sequencer() {
    set_midi_note_learns_the_next_key_as_one_undo_step_and_the_key_plays(DrumGrid::Steps);
}

#[test]
fn set_midi_note_learns_in_the_free_note_grid() {
    set_midi_note_learns_the_next_key_as_one_undo_step_and_the_key_plays(DrumGrid::Notes);
}

#[test]
fn hiding_the_page_releases_the_held_keys() {
    let (mut rig, keys) = midi_rig();
    rig.midi(&keys, &[&[0x90, 60, 100], &[0x90, 64, 100]]);
    rig.take();
    rig.app.on_hidden();
    assert_eq!(rig.take(), ["NoteOff 1 60", "NoteOff 1 64"]);
}

#[test]
fn what_the_keyboard_sent_while_the_page_was_hidden_is_dropped_on_its_return() {
    let (mut rig, keys) = midi_rig();
    link(&mut rig, 1, 20, LEVEL);
    rig.app.song_edited(SongEdit::Final, 0);
    let (song, undos) = (rig.app.song.clone(), undo_steps(&mut rig));
    rig.take();
    rig.app.on_hidden();
    // The hidden page runs no frame: these wait in the queue.
    let tx = keys.tx.borrow().clone().expect("open");
    for message in [
        [0x90, 60, 100],
        [0xB0, 20, 0],
        [0x80, 60, 0],
        [0x90, 62, 100],
    ] {
        tx.send(("Keys".to_owned(), message.to_vec())).unwrap();
    }
    rig.frame(vec![]);
    assert_eq!(rig.take(), Vec::<String>::new());
    rig.next_frame_at(rig.time + 1.0);
    rig.frame(vec![]);
    assert!(rig.app.song == song, "no linked move");
    assert_eq!(undo_steps(&mut rig), undos);
    // What comes after the return plays.
    rig.midi(&keys, &[&[0x90, 64, 100]]);
    assert_eq!(rig.take(), ["NoteOn 1 64 100"]);
}

#[test]
fn a_replaced_song_releases_the_held_keys() {
    let (mut rig, keys) = midi_rig();
    rig.midi(&keys, &[&[0x90, 60, 100], &[0x90, 64, 100]]);
    rig.take();
    rig.app.song_edited(SongEdit::Replaced, 0);
    let log = rig.take();
    assert_eq!(log[..2], ["NoteOff 1 60", "NoteOff 1 64"]);
    assert_eq!(log.iter().filter(|c| c.starts_with("NoteOff")).count(), 2);
}

#[test]
fn an_unplugged_keyboard_releases_its_held_keys_when_the_link_reconnects() {
    let (mut rig, keys) = midi_rig();
    rig.midi(&keys, &[&[0x90, 60, 100]]);
    rig.take();
    keys.ports.borrow_mut().clear();
    rig.next_frame_at(rig.time + 2.0);
    rig.frame(vec![]);
    assert_eq!(rig.take(), ["NoteOff 1 60"]);
}

#[test]
fn a_key_the_old_connection_queued_is_released_by_the_reconnect_in_the_same_frame() {
    let (mut rig, keys) = midi_rig();
    // The note-on waits in the queue when the frame finds the keyboard gone: the frame drains it, then releases.
    let tx = keys.tx.borrow().clone().expect("open");
    tx.send(("Keys".to_owned(), vec![0x90, 60, 100])).unwrap();
    keys.ports.borrow_mut().clear();
    rig.next_frame_at(rig.time + 2.0);
    rig.frame(vec![]);
    assert_eq!(rig.take(), ["NoteOn 1 60 100"]);
    // On and off in one frame: the note-off waits for the tap gate.
    rig.frame(vec![]);
    assert_eq!(rig.take(), ["NoteOff 1 60"]);
}

#[test]
fn on_the_web_the_link_opens_at_the_first_click_only() {
    let mut rig = Rig::new();
    let keys = Rc::new(Device::default());
    rig.app.midi = Midi::default();
    (rig.app.midi.link, rig.app.midi.on_gesture) = (keys.link(), true);
    rig.frame(vec![]);
    rig.frame(vec![]);
    assert_eq!(*keys.opens.borrow(), 0, "never at startup");
    rig.click(OUTSIDE);
    assert_eq!(*keys.opens.borrow(), 1);
    rig.click(OUTSIDE);
    assert_eq!(*keys.opens.borrow(), 1);
}

#[test]
fn the_input_device_setting_reopens_the_link_and_a_repick_changes_nothing() {
    let (mut rig, keys) = midi_rig();
    rig.midi(&keys, &[&[0x90, 60, 100]]);
    rig.take();
    rig.app.settings.midi_device = "Keys".into();
    rig.frame(vec![]);
    assert_eq!(*keys.opens.borrow(), 2);
    assert_eq!(
        rig.app.midi.link.choice(),
        &MidiChoice::Named("Keys".into())
    );
    assert_eq!(rig.take(), ["NoteOff 1 60"], "the reconnect releases");
    rig.app.settings.midi_device = "Keys".into();
    rig.frame(vec![]);
    assert_eq!(*keys.opens.borrow(), 2, "the same pick");
    rig.app.settings.midi_device = "<no midi input>".into();
    rig.frame(vec![]);
    assert_eq!(rig.app.midi.link.choice(), &MidiChoice::None);
    assert_eq!(*keys.opens.borrow(), 2, "no input opens nothing");
}

#[test]
fn the_input_device_dropdown_lists_the_links_ports_and_rescans_once_per_open() {
    let mut rig = Rig::new();
    let keys = Rc::new(Device::default());
    keys.ports.borrow_mut().push("Pad Box 24:0".to_owned());
    // The web's link: nothing opens until a click.
    rig.app.midi = Midi::default();
    (rig.app.midi.link, rig.app.midi.on_gesture) = (keys.link(), true);
    rig.open_menu(MenuTab::Settings);
    assert_eq!((*keys.opens.borrow(), *keys.lists.borrow()), (0, 0));
    let texts = |rig: &Rig| -> Vec<String> {
        (rig.out.shapes.iter())
            .filter_map(|s| match &s.shape {
                egui::Shape::Text(t) => Some(t.galley.text().to_owned()),
                _ => None,
            })
            .collect()
    };
    // `Input device:` is row 1. Its tap opens the link and lists the ports once more.
    rig.click_id(egui::Id::new("settings_tab").with(1_usize));
    assert_eq!((*keys.opens.borrow(), *keys.lists.borrow()), (1, 2));
    rig.frame(vec![]);
    rig.frame(vec![]);
    assert!(
        texts(&rig).contains(&"Pad Box".to_owned()),
        "{:?}",
        texts(&rig)
    );
    assert_eq!(*keys.lists.borrow(), 2, "once per open");
}

// ---- `Link to MIDI CC`, `Link to Mod Wheel`, `Unlink from MIDI CC` (`CCLK`) ----

/// The Distortion's Drive and Gain on the effect track (2): params 1 and 3 of instance 2.
const DRIVE: AutomationTarget = AutomationTarget {
    instance_id: 2,
    param: 1,
};
const GAIN: AutomationTarget = AutomationTarget {
    instance_id: 2,
    param: 3,
};

/// `Link to MIDI CC`'s prompt.
const PROMPT: &str = "Tweak a control on your MIDI controller to assign it to the parameter";

/// The effects rig ([`super::automation::effects_rig`]: channel 2 selected, its rack open) with a fake keyboard, Drive
/// the recent control, and an empty log.
fn link_rig() -> (Rig, Rc<Device>) {
    let mut rig = super::automation::effects_rig();
    let keys = rig.plug_midi();
    let drive = rig.text_rect("Drive").center();
    rig.click(drive);
    rig.frame(vec![]);
    rig.take();
    (rig, keys)
}

/// Channel `channel`'s `CCLK` bytes.
fn cclk(rig: &Rig, channel: usize) -> Vec<u8> {
    rig.app.song.channels[channel].file.cc_links.clone()
}

/// `CCLK` records: {i32 CC, i32 module id, i32 param} each, little-endian.
fn records(links: &[[i32; 3]]) -> Vec<u8> {
    links
        .iter()
        .flatten()
        .flat_map(|f| f.to_le_bytes())
        .collect()
}

fn param(rig: &Rig, target: AutomationTarget) -> f32 {
    rig.app.song.channels[2].param(target).unwrap()
}

fn learning(rig: &Rig) -> bool {
    rig.app.nav.overlays.contains(&Overlay::LearnCc)
}

#[test]
fn the_recent_controls_cc_link_is_read_from_its_own_channel() {
    let mut song = Song::default();
    song.add_channel(ChannelKind::Synth(SynthKind::MiniSynth));
    song.add_channel(ChannelKind::Synth(SynthKind::MiniSynth));
    links::link(&mut song.channels[1].file.cc_links, 20, 0, 1);
    let linked = |channel, target| {
        crate::app::automation::recent_control(&song, (channel, target), None).cc_linked
    };
    assert!(linked(1, PAN));
    assert!(!linked(1, LEVEL), "another param");
    assert!(!linked(2, PAN), "another channel");
}

#[test]
fn link_to_midi_cc_waits_for_any_cc_then_links_it_and_the_cc_moves_the_param() {
    let (mut rig, keys) = link_rig();
    let undos = undo_steps(&mut rig);
    rig.ctrl_pick("Link to MIDI CC");
    assert!(learning(&rig));
    rig.frame(vec![]);
    assert!(rig.painted(PROMPT) && rig.painted("Cancel"));
    // The next CC, of any number on any MIDI channel, is learned, not played.
    rig.midi(&keys, &[&[0xB3, 74, 100]]);
    assert_eq!(cclk(&rig, 2), records(&[[74, 2, 1]]));
    assert!(!learning(&rig), "the prompt closed");
    assert!(
        !rig.take().iter().any(|c| c.starts_with("SetParam")),
        "the learned CC moves nothing"
    );
    assert_eq!(undo_steps(&mut rig), undos + 1);
    // From now on the CC sets Drive to its value, unscaled; the selected channel's own CC 74 param stays.
    rig.midi(&keys, &[&[0xB0, 74, 127]]);
    assert_eq!(rig.take(), ["SetParam 2 2 1 1.000"]);
    assert_eq!(param(&rig, DRIVE), 1.0);
    // The link is one undo step; the move makes none: an undo takes back the link, and Drive stays.
    rig.next_frame_at(rig.time + 1.0);
    rig.frame(vec![]);
    rig.frame(z(egui::Modifiers::NONE));
    assert_eq!(cclk(&rig, 2), [0u8; 0]);
    assert_eq!(param(&rig, DRIVE), 1.0);
}

#[test]
fn a_knobs_link_to_midi_cc_learns_and_the_rest_of_the_twist_moves_it() {
    let (mut rig, keys) = link_rig();
    let gain = rig.text_rect("Gain").center();
    rig.secondary_click(gain);
    rig.frame(vec![]);
    let item = rig.text_rect("Link to MIDI CC").center();
    rig.click(item);
    rig.frame(vec![]);
    assert!(learning(&rig));
    // A twist: the first CC links Gain, the ones after it in the same frame move it.
    rig.midi(&keys, &[&[0xB0, 21, 10], &[0xB0, 21, 64]]);
    assert_eq!(cclk(&rig, 2), records(&[[21, 2, 3]]));
    assert_eq!(param(&rig, GAIN), 64.0 / 127.0);
}

#[test]
fn link_to_mod_wheel_links_cc_1_at_once_and_it_still_moves_the_selected_channels_cc_1() {
    let (mut rig, keys) = link_rig();
    let undos = undo_steps(&mut rig);
    rig.ctrl_pick("Link to Mod Wheel");
    assert!(!learning(&rig), "no prompt");
    assert_eq!(cclk(&rig, 2), records(&[[1, 2, 1]]));
    assert_eq!(undo_steps(&mut rig), undos + 1);
    // Linking it again changes nothing: no step.
    rig.ctrl_pick("Link to Mod Wheel");
    assert_eq!(cclk(&rig, 2), records(&[[1, 2, 1]]));
    assert_eq!(undo_steps(&mut rig), undos + 1);
    rig.take();
    rig.midi(&keys, &[&[0xB0, 1, 127]]);
    assert_eq!(rig.take(), ["SetParam 2 2 1 1.000", "SetParam 2 0 8 1.000"]);
}

#[test]
fn unlink_removes_every_link_of_the_param_from_its_channel_in_one_undo_step() {
    let (mut rig, keys) = link_rig();
    // As a file may hold them: Drive on CC 20 and CC 21, Gain on CC 22.
    rig.app.song.channels[2].file.cc_links = records(&[[20, 2, 1], [22, 2, 3], [21, 2, 1]]);
    rig.app.song_edited(SongEdit::Final, 0);
    let undos = undo_steps(&mut rig);
    rig.ctrl_pick("Unlink from MIDI CC");
    assert_eq!(cclk(&rig, 2), records(&[[22, 2, 3]]));
    assert_eq!(undo_steps(&mut rig), undos + 1);
    rig.take();
    // CC 20 drives nothing now: the selected channel's own CC 20 param (27) gets it.
    rig.midi(&keys, &[&[0xB0, 20, 127]]);
    assert_eq!(rig.take(), ["SetParam 2 0 27 1.000"]);
    // Undo brings both links back.
    rig.frame(z(egui::Modifiers::NONE));
    assert_eq!(
        cclk(&rig, 2),
        records(&[[20, 2, 1], [22, 2, 3], [21, 2, 1]])
    );
}

#[test]
fn cancel_escape_or_a_tap_outside_stops_waiting_and_the_cc_plays_as_before() {
    for close in ["Cancel", "Escape", "outside"] {
        let (mut rig, keys) = link_rig();
        let undos = undo_steps(&mut rig);
        rig.ctrl_pick("Link to MIDI CC");
        rig.frame(vec![]);
        match close {
            "Cancel" => {
                let cancel = rig.text_rect("Cancel").center();
                rig.click(cancel);
            }
            "Escape" => rig.frame(escape()),
            _ => {
                assert!(!rig.text_rect(PROMPT).contains(OUTSIDE));
                rig.click(OUTSIDE);
            }
        }
        rig.frame(vec![]);
        assert!(!learning(&rig), "{close}");
        rig.take();
        rig.midi(&keys, &[&[0xB0, 74, 127]]);
        assert_eq!(rig.take(), ["SetParam 2 0 81 1.000"], "{close}");
        assert_eq!(cclk(&rig, 2), [0u8; 0], "{close}");
        assert_eq!(undo_steps(&mut rig), undos, "{close}");
    }
}

#[test]
fn a_mixer_strips_link_names_the_channel_and_a_drum_pads_its_slot() {
    let (mut rig, keys) = link_rig();
    // Channel 1's fader on the mixer: instance 0, Level (param 0). Strips are 128 px wide in rack order (TRK).
    let strip = rig
        .app
        .song
        .rack_order()
        .iter()
        .position(|&c| c == 1)
        .unwrap() as f32;
    rig.app.navigate(Nav::Show(View::Mixer));
    rig.frame(vec![]);
    rig.secondary_click(pos2(strip * 128.0 + 64.0, 100.0));
    rig.frame(vec![]);
    let item = rig.text_rect("Link to Mod Wheel").center();
    rig.click(item);
    rig.frame(vec![]);
    assert_eq!(cclk(&rig, 1), records(&[[1, 0, 0]]));
    assert_eq!(
        cclk(&rig, 2),
        [0u8; 0],
        "the control's channel, not the selected one"
    );
    // A drum pad strip's Level: instance −slot id.
    rig.app.add_default_drums(0);
    let drums = rig.app.song.channels.len() - 1;
    let slot = rig.app.song.channels[drums]
        .drum_kit
        .as_ref()
        .unwrap()
        .slots[1]
        .file
        .id as i32;
    rig.edit(drums);
    rig.slide_split();
    let row = rig.app.slots.drum_editor.step.selected;
    rig.click_id(egui::Id::new("step_seq").with(("row", row)));
    rig.frame(vec![]);
    rig.frame(vec![]);
    let level = egui::Id::new(("drum_editor", drums))
        .with("drum strips")
        .with((1_usize, crate::ui::drums::Ctl::Level));
    let at = rig.ctx.read_response(level).expect("drawn").rect.center();
    rig.secondary_click(at);
    rig.frame(vec![]);
    let item = rig.text_rect("Link to Mod Wheel").center();
    rig.click(item);
    rig.frame(vec![]);
    assert_eq!(cclk(&rig, drums), records(&[[1, -slot, 0]]));
    rig.take();
    rig.midi(&keys, &[&[0xB0, 1, 0]]);
    let kit = rig.app.song.channels[drums].drum_kit.as_ref().unwrap();
    assert_eq!(kit.slot_by_id(slot as u32).unwrap().params[0], 0.0);
}

#[test]
fn a_second_cc_linked_to_the_param_adds_a_link_and_unlink_removes_both() {
    let (mut rig, keys) = link_rig();
    rig.ctrl_pick("Link to Mod Wheel");
    rig.ctrl_pick("Link to MIDI CC");
    rig.midi(&keys, &[&[0xB0, 74, 100]]);
    // Both CCs drive Drive: each link is stored; only an exact duplicate is not.
    assert_eq!(cclk(&rig, 2), records(&[[1, 2, 1], [74, 2, 1]]));
    rig.take();
    rig.midi(&keys, &[&[0xB0, 74, 0]]);
    assert_eq!(rig.take(), ["SetParam 2 2 1 0.000"]);
    rig.midi(&keys, &[&[0xB0, 1, 127]]);
    assert_eq!(param(&rig, DRIVE), 1.0);
    let undos = undo_steps(&mut rig);
    rig.ctrl_pick("Unlink from MIDI CC");
    assert_eq!(cclk(&rig, 2), [0u8; 0]);
    assert_eq!(undo_steps(&mut rig), undos + 1);
}

#[test]
fn a_message_over_the_prompt_holds_the_learning_until_it_closes() {
    let (mut rig, keys) = link_rig();
    rig.ctrl_pick("Link to MIDI CC");
    rig.app.navigate(Nav::Open(Overlay::Message {
        tag: "test",
        lines: vec!["Something else".into()],
        confirm: None,
        extra: None,
    }));
    rig.frame(vec![]);
    rig.frame(vec![]);
    assert!(!rig.painted(PROMPT), "the prompt waits under the message");
    rig.take();
    // The CC is no answer to a prompt nobody sees: it plays as usual.
    rig.midi(&keys, &[&[0xB0, 74, 127]]);
    assert_eq!(cclk(&rig, 2), [0u8; 0]);
    assert_eq!(rig.take(), ["SetParam 2 0 81 1.000"]);
    assert!(learning(&rig), "still waiting");
    // The message's OK: the prompt is back, and the next CC links.
    let ok = rig.text_rect("OK").center();
    rig.click(ok);
    rig.frame(vec![]);
    assert!(rig.painted(PROMPT));
    rig.midi(&keys, &[&[0xB0, 74, 100]]);
    assert_eq!(cclk(&rig, 2), records(&[[74, 2, 1]]));
    assert!(!learning(&rig));
}

#[test]
fn a_midi_cc_1_moves_the_mod_wheels_value_and_a_new_song_forgets_it() {
    // CC 1 is the MOD wheel's param 8.
    let (mut rig, keys) = midi_rig();
    assert_eq!(rig.app.wheel_values()[0], 0.0);
    rig.midi(&keys, &[&[0xB0, 1, 127]]);
    assert_eq!(rig.take(), ["SetParam 1 0 8 1.000"]);
    assert_eq!(rig.app.wheel_values()[0], 1.0);
    let id = rig.app.song.channels[1].id;
    assert_eq!(rig.app.mod_wheel.get(&id), Some(&1.0));
    rig.app.song_edited(SongEdit::Replaced, 0);
    assert!(rig.app.mod_wheel.is_empty());
}

#[test]
fn an_armed_midi_note_starts_playback_and_records_on_the_current_channel_at_v_times_0_007874() {
    // Live note recording: the MIDI channel is ignored, and a note-on at velocity 0 is the note-off.
    let (mut rig, keys) = midi_rig();
    rig.app.settings.count_in = false;
    rig.app.main_screen.armed = true;
    rig.rec.0.borrow_mut().tick = 3 * u64::from(crate::song::PPQ);
    rig.midi(&keys, &[&[0x95, 60, 127]]);
    assert!(rig.app.main_screen.playing);
    let log = rig.take();
    // Its note goes out right after the Play it starts, so the Play's all-notes-off does not cut it.
    assert_eq!(
        log[..2],
        ["Play", "NoteOn 1 60 127"],
        "it sounds from the Play"
    );
    rig.rec.0.borrow_mut().tick = 4 * u64::from(crate::song::PPQ);
    rig.midi(&keys, &[&[0x95, 60, 0]]);
    let ppq = crate::song::PPQ;
    let clip = &rig.app.song.channels[1].lanes[0].clips[0];
    let [
        crate::song::Event::Note {
            pos,
            len_beats,
            key,
            velocity,
            ..
        },
    ] = clip.events[..]
    else {
        panic!("{:?}", clip.events)
    };
    // 255 · 127 · 0.007874 = 254.99…: 254.
    assert_eq!(
        (clip.start_tick, pos, key, velocity, len_beats),
        (0, 3 * ppq, 60, 254, 1.0)
    );
}

#[test]
fn key_lock_snaps_a_midi_note_as_it_sounds_and_records_and_its_off_ends_the_snapped_note() {
    // Key-lock snaps a MIDI note before it is recorded, as for typed keys: downward first.
    let (mut rig, keys) = midi_rig();
    // C major: C# (61) plays C (60).
    rig.app.song.key_scale = crate::song::scales::scale_to_stored(0);
    rig.app.settings.count_in = false;
    rig.app.main_screen.armed = true;
    rig.midi(&keys, &[&[0x90, 61, 100]]);
    rig.rec.0.borrow_mut().tick = u64::from(crate::song::PPQ);
    rig.midi(&keys, &[&[0x80, 61, 0]]);
    let log = rig.take();
    assert!(log.contains(&"NoteOn 1 60 100".to_owned()), "{log:?}");
    assert!(log.contains(&"NoteOff 1 60".to_owned()), "{log:?}");
    let clip = &rig.app.song.channels[1].lanes[0].clips[0];
    let keys: Vec<_> = (clip.events.iter())
        .map(|e| match *e {
            crate::song::Event::Note { key, len_beats, .. } => (key, len_beats),
            _ => panic!(),
        })
        .collect();
    assert_eq!(keys, [(60, 1.0)]);
}

/// MIDI notes are live input: a held one lights its key in both rows of two-keyboard mode,
/// and its note-off lets both fade out.
#[test]
fn a_midi_note_lights_its_key_in_both_rows_until_its_note_off() {
    let (mut rig, keys) = midi_rig();
    rig.app.settings.show_two_rows = true;
    rig.app.settings.preferred_kbd_size = 6.0;
    for _ in 0..10 {
        rig.frame(vec![]);
    }
    let [top, bottom] = Rig::rows_at(6.0, true)[..] else {
        panic!()
    };
    // C5 (60): the top row's first key, and the bottom row's eighth.
    let c5 = [
        pos2(U, top.top() + 0.8 * top.height()),
        pos2(15.0 * U, bottom.top() + 0.8 * bottom.height()),
    ];
    rig.midi(&keys, &[&[0x90, 60, 100]]);
    rig.frame(vec![]);
    for at in c5 {
        assert!(rig.key_level(at) > 0.9, "{at:?}: {}", rig.key_level(at));
    }
    rig.midi(&keys, &[&[0x80, 60, 0]]);
    for _ in 0..40 {
        rig.frame(vec![]);
    }
    for at in c5 {
        assert_eq!(rig.key_level(at), 0.0, "{at:?}");
    }
}

/// C major key-lock on `rig`'s song.
fn c_major(rig: &mut Rig) {
    rig.app.song.key_scale = crate::song::scales::scale_to_stored(0);
}

#[test]
fn a_key_locked_midi_note_ends_with_the_key_its_press_played_after_the_channel_changes() {
    // The release ends what the press started, as a typed key's; never re-snapped at release.
    let (mut rig, keys) = midi_rig();
    rig.app.add_default_drums(0);
    rig.app.selected_channel = 1;
    rig.frame(vec![]);
    c_major(&mut rig);
    rig.take();
    rig.midi(&keys, &[&[0x90, 61, 100]]);
    // The drums (no key-lock on pads) become current; the key is released there.
    rig.app.selected_channel = 3;
    rig.frame(vec![]);
    rig.midi(&keys, &[&[0x80, 61, 0]]);
    assert_eq!(rig.take(), ["NoteOn 1 60 100", "NoteOff 1 60"]);
}

#[test]
fn a_key_locked_midi_note_ends_with_its_key_after_the_scale_changes_and_lights_that_key() {
    let (mut rig, keys) = midi_rig();
    c_major(&mut rig);
    rig.midi(&keys, &[&[0x90, 61, 100]]);
    // The keys light the note that sounds, C (60), not the controller's C# (61).
    let notes = rig.app.key_input().notes;
    assert_eq!((notes >> 60 & 1, notes >> 61 & 1), (1, 0));
    let chromatic = crate::song::scales::CHROMATIC;
    rig.app.song.key_scale = crate::song::scales::scale_to_stored(chromatic);
    rig.midi(&keys, &[&[0x80, 61, 0]]);
    assert_eq!(rig.take(), ["NoteOn 1 60 100", "NoteOff 1 60"]);
    assert_eq!(rig.app.key_input().notes, 0);
    // A held key ends with its snapped note when the link reconnects (`release_all`) too.
    c_major(&mut rig);
    rig.midi(&keys, &[&[0x90, 61, 100]]);
    rig.app.release_midi();
    assert_eq!(rig.take(), ["NoteOn 1 60 100", "NoteOff 1 60"]);
}

/// Play start stops the preview player, and the open picker's sample strip with it (its play button back),
/// whose stop then reaches the player too. With REC armed, a MIDI key starts playback while the pad picker is open
/// (modal: the play button and Space are under it) and its pick plays.
#[test]
fn play_start_stops_the_open_pickers_sample_strip() {
    let mut rig = Rig::new();
    rig.app.add_default_drums(0);
    let wav = crate::sample_cache::tests::wav(&[0; 4410]);
    (rig.app.files).keep_user_file(std::path::Path::new("My Samples/A.wav"), wav);
    rig.click(OUTSIDE);
    let keys = rig.plug_midi();
    rig.app.selected_channel = 1;
    rig.app.navigate(Nav::Open(Overlay::SamplePicker {
        channel: 1,
        pad: None,
    }));
    for row in [0, 1, 0] {
        super::rack_wiring::picker_row(&mut rig, row);
    }
    assert!(
        rig.take()
            .contains(&"Preview 4410 frames from 0".to_owned())
    );
    rig.app.main_screen.armed = true;
    rig.midi(&keys, &[&[0x90, 36, 100]]);
    assert!(rig.app.main_screen.playing);
    rig.frame(vec![]);
    let sent = rig.take();
    let at = |c: &str| sent.iter().position(|s| s == c);
    assert!(
        at("Play").is_some() && at("Play") < at("Preview stop"),
        "{sent:?}"
    );
}

/// Play cuts a MIDI key held, and it goes dark; a new press of that key lights it again,
/// even before its release.
#[test]
fn a_midi_key_cut_at_play_lights_again_at_its_next_press() {
    let (mut rig, keys) = midi_rig();
    rig.midi(&keys, &[&[0x90, 60, 100]]);
    assert_eq!(rig.app.sounding(1), 1 << 60);
    rig.click(PLAY);
    assert!(rig.app.main_screen.playing);
    assert_eq!(rig.app.sounding(1), 0, "cut at Play");
    rig.midi(&keys, &[&[0x90, 60, 100]]);
    assert_eq!(rig.app.sounding(1), 1 << 60, "pressed again");
}

#[test]
fn a_linked_move_of_the_tremolos_min_above_max_raises_max_in_the_song() {
    // Min above Max raises Max to it, as the effect does with the audio's copy.
    let (mut rig, keys) = midi_rig();
    let rack = &mut rig.app.song.channels[1].rack;
    crate::ui::modules_browser::add_module(rack, 46, InsertAt::End);
    let tremolo = rack.modules.last_mut().unwrap();
    (tremolo.params[3], tremolo.params[4]) = (0.5, 0.2);
    let min = AutomationTarget {
        instance_id: tremolo.instance_id,
        param: 4,
    };
    link(&mut rig, 1, 20, min);
    rig.midi(&keys, &[&[0xB0, 20, 127]]);
    let params = &rig.app.song.channels[1].rack.modules.last().unwrap().params;
    assert_eq!((params[3], params[4]), (1.0, 1.0));
}

#[test]
fn a_frames_midi_notes_and_live_controllers_reach_the_engine_in_input_order() {
    // The frame holds its live notes until its transport command, and its live controllers (CCs,
    // pitch bend, pressure) with them: a note-off then the sustain pedal down in one batch ends the note, unsustained.
    let (mut rig, keys) = midi_rig();
    rig.midi(&keys, &[&[0x90, 60, 100]]);
    rig.frame(vec![]);
    rig.take();
    rig.midi(
        &keys,
        &[&[0x80, 60, 0], &[0xB0, 64, 127], &[0xE0, 0x00, 0x40]],
    );
    assert_eq!(
        rig.take(),
        [
            "NoteOff 1 60",
            "SetParam 1 0 71 1.000",
            "LiveParam 1 4 0.500"
        ]
    );
}

#[test]
fn a_monitored_channel_plays_along_once_and_each_channel_takes_only_what_its_midi_input_passes() {
    let (mut rig, keys) = midi_rig();
    rig.app.song.channels[2].file.input_monitoring = true;
    rig.midi(&keys, &[&[0x90, 60, 100]]);
    assert_eq!(rig.take(), ["NoteOn 1 60 100", "NoteOn 2 60 100"]);
    rig.midi(&keys, &[&[0x80, 60, 0]]);
    assert_eq!(rig.take(), ["NoteOff 1 60", "NoteOff 2 60"]);
    // Selected and monitoring: once.
    rig.app.selected_channel = 2;
    rig.midi(&keys, &[&[0x90, 62, 100]]);
    assert_eq!(rig.take(), ["NoteOn 2 62 100"]);
    rig.midi(&keys, &[&[0x80, 62, 0]]);
    rig.app.selected_channel = 1;
    rig.take();
    // Channel 2 takes MIDI channel 2 only: a channel-1 note reaches the selected channel alone.
    rig.app.song.channels[2].file.midi_input.channel = Some(1);
    rig.midi(&keys, &[&[0x90, 64, 100], &[0x91, 65, 100]]);
    assert_eq!(
        rig.take(),
        ["NoteOn 1 64 100", "NoteOn 1 65 100", "NoteOn 2 65 100"]
    );
    rig.midi(&keys, &[&[0x80, 64, 0], &[0x81, 65, 0]]);
    rig.take();
    // The selected channel takes the device `Keys` only: `Pads` plays channel 2 on MIDI channel 2, and its pitch bend
    // follows.
    rig.app.song.channels[1].file.midi_input.device = Some("Keys".to_owned());
    rig.midi_from(&keys, "Pads", &[&[0x90, 67, 100], &[0x91, 69, 100]]);
    assert_eq!(rig.take(), ["NoteOn 2 69 100"]);
    rig.midi_from(&keys, "Pads", &[&[0xE1, 0x00, 0x40]]);
    assert_eq!(rig.take(), ["LiveParam 2 4 0.500"]);
}

#[test]
fn rec_records_midi_into_the_armed_channels_and_not_the_selected_one() {
    let (mut rig, keys) = midi_rig();
    rig.app.settings.count_in = false;
    rig.app.song.channels[2].file.record_armed = true;
    rig.app.main_screen.armed = true;
    rig.midi(&keys, &[&[0x90, 60, 100]]);
    assert!(rig.app.main_screen.playing);
    // Armed, not monitoring: channel 2 records without sounding; the selected channel sounds.
    let log = rig.take();
    assert!(log.contains(&"NoteOn 1 60 100".to_owned()), "{log:?}");
    assert!(!log.iter().any(|l| l.starts_with("NoteOn 2")), "{log:?}");
    rig.rec.0.borrow_mut().tick = u64::from(crate::song::PPQ);
    rig.midi(&keys, &[&[0x80, 60, 0]]);
    let notes = |rig: &Rig, c: usize| -> Vec<(u8, f64)> {
        let lanes = &rig.app.song.channels[c].lanes;
        let events = lanes.iter().flat_map(|l| &l.clips).flat_map(|c| &c.events);
        (events.filter_map(|e| match *e {
            crate::song::Event::Note { key, len_beats, .. } => Some((key, len_beats)),
            _ => None,
        }))
        .collect()
    };
    assert_eq!(notes(&rig, 2), [(60, 1.0)]);
    assert_eq!(notes(&rig, 1), []);
}
