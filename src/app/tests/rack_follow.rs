//! The open rack follows the current channel. A header or clip tap, the keyboard's `Select clip`, a clone, an undo or
//! A delete that makes another channel current shows that channel's rack as opening it there would, swapped in place
//! (no new slide) and with no edit; the master's too once it is current.

use eframe::egui::Id;

use super::rack_wiring::{instance_id, layout, with_effects};
use super::*;
use crate::panels::{self, PanelState};
use crate::song::AutomationTarget;
use crate::ui::modules_browser::{InsertAt, add_module};
use crate::ui::rack::content_rect;

/// The ≡ button of `channel`'s first card, which only that channel's rack draws.
fn card_button(rig: &Rig, channel: usize) -> Id {
    let id = instance_id(rig, channel, 0);
    Id::new(("rack", channel))
        .with(("rack card", id))
        .with("menu")
}

/// The channels with a card whose rack the last two frames drew (egui looks widgets up a pass back).
fn drawn(rig: &Rig) -> Vec<usize> {
    let channels = 0..rig.app.song.channels.len();
    channels
        .filter(|&c| !rig.app.song.channels[c].rack.modules.is_empty())
        .filter(|&c| rig.ctx.read_response(card_button(rig, c)).is_some())
        .collect()
}

/// The point `beat` beats into track row `row` of the playlist (a beat is 38.4 points from x 48).
fn at(row: usize, beat: f32) -> Pos2 {
    pos2(48.0 + beat * 38.4, header(row).y)
}

/// Two MiniSynths (1, 2), each with a clip at beat 0, in the undo history; the second is current.
fn rig() -> Rig {
    let mut rig = Rig::with_synth();
    (rig.app).add_bare_channel(ChannelKind::Synth(SynthKind::MiniSynth), 0);
    for channel in [1, 2] {
        rig.app.song.channels[channel].lanes[0].add_clip(0, 4.0);
    }
    rig.app.song_edited(SongEdit::Final, 0);
    rig.frame(vec![]);
    rig
}

#[test]
fn a_header_tap_shows_its_channels_rack_with_no_edit() {
    let mut rig = rig();
    rig.open_rack(1);
    rig.slide_rack();
    assert_eq!(drawn(&rig), [1]);
    rig.app.files.saved();
    let steps = rig.app.history.steps();
    rig.click(header(2));
    assert_eq!(rig.app.selected_channel, 2);
    assert_eq!(rig.overlays(), [Overlay::Rack(2)]);
    rig.frame(vec![]);
    rig.frame(vec![]);
    assert_eq!(drawn(&rig), [2]);
    assert!(!rig.app.files.modified());
    assert_eq!(rig.app.history.steps(), steps);
    // A closed rack stays closed.
    rig.app.navigate(Nav::Close);
    rig.click(header(1));
    assert_eq!(rig.app.selected_channel, 1);
    assert_eq!(rig.overlays(), []);
}

#[test]
fn a_clip_tap_on_another_channel_shows_its_rack() {
    let mut rig = rig();
    rig.open_rack(2);
    rig.slide_rack();
    rig.click(at(1, 1.0));
    assert_eq!(rig.app.selected_channel, 1);
    assert_eq!(rig.overlays(), [Overlay::Rack(1)]);
    rig.frame(vec![]);
    rig.frame(vec![]);
    assert_eq!(drawn(&rig), [1]);
}

#[test]
fn the_keyboards_select_clip_on_another_channel_shows_its_rack() {
    let mut rig = rig();
    rig.open_rack(2);
    rig.slide_rack();
    // A long press selects synth 1's clip without making it current, and puts the grid cursor and the focus there.
    let clip = at(1, 1.0);
    rig.press(clip);
    for _ in 0..3 {
        rig.frame(vec![]);
    }
    rig.release(clip);
    assert_eq!(rig.overlays(), [Overlay::Rack(2)]);
    // `open` is `Select clip`: synth 1 is current.
    rig.frame(key_tap(egui::Key::Enter, egui::Modifiers::NONE));
    assert_eq!(rig.app.selected_channel, 1);
    assert_eq!(rig.overlays(), [Overlay::Rack(1)]);
    rig.frame(vec![]);
    rig.frame(vec![]);
    assert_eq!(drawn(&rig), [1]);
}

#[test]
fn a_rack_sliding_in_swaps_its_channel_in_place() {
    let mut rig = rig();
    rig.open_rack(1);
    rig.frame(vec![]);
    let x = |rig: &Rig, c: usize| {
        rig.ctx
            .read_response(card_button(rig, c))
            .unwrap()
            .rect
            .left()
    };
    let sliding = x(&rig, 1);
    rig.click(header(2));
    rig.frame(vec![]);
    // Further in, not back at the screen's edge.
    let swapped = x(&rig, 2);
    assert!(swapped < sliding, "{swapped} {sliding}");
    rig.slide_rack();
    assert!(x(&rig, 2) < swapped);
    assert_eq!(drawn(&rig), [2]);
}

#[test]
fn a_clone_its_undo_and_deletes_take_the_rack_to_the_current_channel_down_to_the_master() {
    let mut rig = rig();
    rig.open_rack(2);
    rig.slide_rack();
    // `Clone Channel`: the clone (3) is current.
    rig.track_menu(2, (0, 2));
    assert_eq!(rig.app.selected_channel, 3);
    assert_eq!(rig.overlays(), [Overlay::Rack(3)]);
    // Its undo: synth 2 is current again.
    rig.frame(key_tap(egui::Key::Z, egui::Modifiers::COMMAND));
    assert_eq!(rig.app.song.channels.len(), 3);
    assert_eq!(rig.overlays(), [Overlay::Rack(2)]);
    // The rack's channel deleted: synth 1 is current; then the master, alone.
    rig.app.delete_channel(2, 0);
    assert_eq!(rig.overlays(), [Overlay::Rack(1)]);
    rig.frame(vec![]);
    assert_eq!(drawn(&rig), [1]);
    rig.app.delete_channel(1, 0);
    assert_eq!(rig.app.selected_channel, 0);
    assert_eq!(rig.overlays(), [Overlay::Rack(0)]);
    rig.frame(vec![]);
}

#[test]
fn an_undo_mid_knob_drag_that_takes_the_racks_channel_away_ends_the_drag() {
    let mut rig = with_effects(&[6]);
    let content = content_rect(layout(&rig, 2).cards[0], 5.5, U);
    let module = rig.app.song.channels[2].rack.modules[0].clone();
    let panel = panels::panel_for(6).unwrap();
    let placed = panels::place(panel, &module, &PanelState::default(), content, U);
    let drive = placed.iter().find(|(l, _)| l.id == 3).unwrap().1.center();
    rig.press(drive);
    rig.drag_to(drive - vec2(0.0, 50.0));
    let synth = rig.app.song.channels[1].rack.clone();
    // Ctrl+Z mid-drag takes the effect track's add: the MiniSynth is current, and its rack shows.
    rig.frame(key_tap(egui::Key::Z, egui::Modifiers::COMMAND));
    assert_eq!(rig.app.song.channels.len(), 2);
    assert_eq!(rig.overlays(), [Overlay::Rack(1)]);
    rig.take();
    rig.drag_to(drive - vec2(0.0, 150.0));
    rig.release(drive - vec2(0.0, 150.0));
    rig.frame(vec![]);
    assert!(rig.ctx.dragged_id().is_none());
    assert_eq!(
        rig.app.song.channels[1].rack, synth,
        "the drag moved nothing"
    );
    let sent = rig.take();
    assert!(!sent.iter().any(|s| s.starts_with("SetParam")), "{sent:?}");
    assert_eq!(drawn(&rig), [1]);
}

#[test]
fn the_fine_tune_and_card_menu_of_a_channel_an_undo_takes_away_go_with_it() {
    // Ctrl+Z works under the fine tune dialog: the effect track's add goes, its dialog with it, and the MiniSynth is
    // current.
    let mut rig = with_effects(&[6]);
    let target = AutomationTarget {
        instance_id: 0,
        param: 0,
    };
    let fine_tune = Overlay::FineTune { channel: 2, target };
    rig.app.navigate(Nav::Open(fine_tune.clone()));
    rig.frame(vec![]);
    assert_eq!(rig.overlays(), [Overlay::Rack(2), fine_tune]);
    rig.frame(key_tap(egui::Key::Z, egui::Modifiers::COMMAND));
    assert_eq!(rig.app.song.channels.len(), 2);
    assert_eq!(rig.overlays(), [Overlay::Rack(1)]);
    // No key undoes under a card menu; were one to, the menu goes with its channel.
    let mut rig = with_effects(&[6]);
    let id = instance_id(&rig, 2, 0);
    rig.click_id(card_button(&rig, 2));
    rig.frame(vec![]);
    let item = Id::new(("rack", 2_usize))
        .with(("rack card menu", id))
        .with((0_usize, 0_usize));
    assert!(rig.ctx.read_response(item).is_some());
    rig.undo_under_modal();
    assert_eq!(rig.overlays(), [Overlay::Rack(1)]);
    rig.frame(vec![]);
    assert!(rig.ctx.read_response(item).is_none());
    assert_eq!(drawn(&rig), [1]);
}

#[test]
fn insert_channel_on_the_current_track_keeps_its_index_and_shows_the_new_channels_rack() {
    let mut rig = rig();
    rig.open_rack(2);
    rig.slide_rack();
    // `Insert Channel`, then `Effect` in its popup: the new channel takes index 2, synth 2 moves to 3.
    rig.track_menu(2, (0, 3));
    rig.click_id(Id::new("insert_channel_popup").with((0_usize, 4_usize)));
    assert_eq!(rig.app.song.channels[2].kind, ChannelKind::EffectTrack);
    assert_eq!(rig.app.selected_channel, 2);
    assert_eq!(rig.overlays(), [Overlay::Rack(2)]);
}

#[test]
fn a_drag_on_a_deleted_channels_rack_moves_nothing_on_the_channel_in_its_place() {
    // Two effect tracks whose Drive has the same widget id at index 2.
    let mut rig = with_effects(&[6]);
    rig.app.add_bare_channel(ChannelKind::EffectTrack, 0);
    add_module(&mut rig.app.song.channels[3].rack, 6, InsertAt::End);
    rig.app.song_edited(SongEdit::Final, 0);
    rig.app.selected_channel = 2;
    rig.frame(vec![]);
    let drive = knob(&rig, 2, 0, 3);
    rig.press(drive);
    rig.drag_to(drive - vec2(0.0, 50.0));
    rig.app.delete_channel(2, 0);
    assert_eq!(rig.overlays(), [Overlay::Rack(2)]);
    let fx = rig.app.song.channels[2].rack.clone();
    rig.take();
    rig.drag_to(drive - vec2(0.0, 150.0));
    rig.release(drive - vec2(0.0, 150.0));
    rig.frame(vec![]);
    assert_eq!(rig.app.song.channels[2].rack, fx, "the drag moved nothing");
    let sent = rig.take();
    assert!(!sent.iter().any(|s| s.starts_with("SetParam")), "{sent:?}");
}

#[test]
fn a_keyboard_switch_mid_drag_still_rebuilds_a_note_effects_edit() {
    // A Randomizer (a note effect) on synth 2, under its minimized MiniSynth.
    let mut rig = rig();
    let rack = &mut rig.app.song.channels[2].rack;
    rack.modules[0].minimized = true;
    add_module(rack, 49, InsertAt::End);
    rig.app.song_edited(SongEdit::Final, 0);
    rig.open_rack(2);
    rig.slide_rack();
    // The grid cursor on synth 1's clip (a long press selects it without making it current).
    let clip = at(1, 1.0);
    rig.press(clip);
    for _ in 0..3 {
        rig.frame(vec![]);
    }
    rig.release(clip);
    let card = (rig.app.song.channels[2].rack.modules.iter())
        .position(|m| m.type_id == 49)
        .unwrap();
    let velocity = knob(&rig, 2, card, 2);
    rig.press(velocity);
    rig.drag_to(velocity - vec2(0.0, 50.0));
    // The keys back on the playlist grid, then `Select clip`: synth 1 is current mid-drag.
    crate::ui::focus::focus(&rig.ctx, Id::new("playlist grid"));
    rig.frame(vec![]);
    rig.frame(key_tap(egui::Key::Enter, egui::Modifiers::NONE));
    assert_eq!(rig.app.selected_channel, 1);
    assert_eq!(rig.overlays(), [Overlay::Rack(1)]);
    rig.take();
    rig.release(velocity - vec2(0.0, 50.0));
    rig.frame(vec![]);
    let sent = rig.take();
    assert!(
        sent.iter().any(|s| s.starts_with("ReplaceSong")),
        "{sent:?}"
    );
}

/// The centre of param `param`'s control on card `card` of `channel`'s open rack, unscrolled.
fn knob(rig: &Rig, channel: usize, card: usize, param: u32) -> Pos2 {
    let module = &rig.app.song.channels[channel].rack.modules[card];
    let height = crate::ui::rack::ui_height(module.type_id).unwrap();
    let content = content_rect(layout(rig, channel).cards[card], height, U);
    let panel = panels::panel_for(module.type_id).unwrap();
    let placed = panels::place(panel, module, &PanelState::default(), content, U);
    placed
        .iter()
        .find(|(l, _)| l.id == param)
        .unwrap()
        .1
        .center()
}
