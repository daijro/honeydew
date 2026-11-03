//! The song-changed flag: what sets
//! it, and what does not, as the unsaved-changes question sees it. The drum strips' cases are in `editor_wiring.rs`.

use super::rack_wiring::with_effects;
use super::*;
use crate::panels::{self, PanelState};
use crate::song::AutomationTarget;
use crate::song::points::{self, Point, PointKey};
use crate::ui::rack::{content_rect, flow, rack_rect};
use honeydew_audio::device::AudioRenderer;

/// The centre of control `id` of the first card of channel 2's rack (`with_effects`), as placed.
fn control(rig: &Rig, id: u32) -> Pos2 {
    let m = Metrics::from_pixels_per_point(1.0, DEFAULT_UI_SCALE);
    let screen = egui::Rect::from_min_size(Pos2::ZERO, vec2(1170.0, 540.0));
    let rack = rack_rect(Regions::new(screen, &m).area, 1.0, false, &m);
    let module = rig.app.song.channels[2].rack.modules[0].clone();
    let heights = [crate::ui::rack::card_height(&module)];
    let card = flow(rack, 0.0, U, false, &heights, true).cards[0];
    let panel = panels::panel_for(module.type_id).unwrap();
    let placed = panels::place(
        panel,
        &module,
        &PanelState::default(),
        content_rect(card, 5.5, U),
        U,
    );
    placed.iter().find(|(l, _)| l.id == id).unwrap().1.center()
}

#[test]
fn a_knob_reset_back_to_its_saved_value_still_asks() {
    // Any control bound to a param sets the flag on every value change: a drag step, a double-tap reset.
    let mut rig = with_effects(&[6]);
    rig.app.files.saved();
    let saved = rig.app.song.clone();
    let drive = control(&rig, 3);
    let value = |rig: &Rig| rig.app.song.channels[2].rack.modules[0].params[1];
    rig.press(drive);
    rig.drag_to(drive - vec2(0.0, 200.0));
    rig.release(drive - vec2(0.0, 200.0));
    assert_ne!(value(&rig), saved.channels[2].rack.modules[0].params[1]);
    // A double tap resets it on the second touch-down.
    rig.click(drive);
    rig.button(drive, true);
    rig.release(drive);
    rig.frame(vec![]);
    assert_eq!(value(&rig), saved.channels[2].rack.modules[0].params[1]);
    assert!(rig.app.files.modified());
}

#[test]
fn an_edit_undone_is_still_a_change() {
    // Saving an undo step sets the flag; only a save, a new song or a load clears it.
    let mut rig = Rig::with_synth();
    rig.app.files.saved();
    let saved = rig.app.song.tempo_bpm;
    rig.app.song.tempo_bpm = saved + 10.0;
    rig.app.song_edited(SongEdit::Final, 0);
    rig.frame(z(egui::Modifiers::NONE));
    assert_eq!(rig.app.song.tempo_bpm, saved);
    assert!(rig.app.files.modified());
}

#[test]
fn automation_playback_sets_nothing() {
    // Value updates the app makes itself, such as automation playback, do not set the flag.
    let mut rig = Rig::with_synth();
    let level = AutomationTarget {
        instance_id: 0,
        param: 0,
    };
    let clip = (rig.app.song.channels[1].add_automation_lane(level)).add_clip(0, 16.0);
    let ends = vec![
        Point {
            tick: 0,
            value: 0,
            ..Default::default()
        },
        Point {
            tick: 2047,
            value: 65534,
            ..Default::default()
        },
    ];
    points::set_points(clip, PointKey::Automation, ends);
    rig.app.song_edited(SongEdit::Final, 0);
    rig.frame(vec![]);
    rig.app.files.saved();
    rig.click(PLAY);
    for tick in [256, 512, 1024] {
        {
            let mut rec = rig.rec.0.borrow_mut();
            rec.tick = tick;
            let engine = rec.engine.as_mut().expect("an engine");
            engine.render(&mut vec![0.0; 24_000 * 2], 2);
        }
        rig.frame(vec![]);
    }
    assert!(rig.app.main_screen.playing);
    assert!(!rig.app.files.modified());
}

#[test]
fn quantize_is_a_change() {
    // Quantize re-syncs (`SongEdit::Resync`); a step that moved notes sets the flag.
    let mut rig = Rig::with_drums_and_synth();
    rig.edit(2);
    rig.slide_split();
    // A tap adds a note on the step nearest it, then it is put off the grid by 5 ticks.
    let cell = pos2(60.0, 200.0);
    rig.click(cell);
    let pos = |rig: &Rig| match rig.app.song.channels[2].lanes[0].clips[0].events[0] {
        crate::song::Event::Note { pos, .. } => pos,
        _ => unreachable!("a note"),
    };
    let on_grid = pos(&rig);
    if let crate::song::Event::Note { pos, .. } =
        &mut rig.app.song.channels[2].lanes[0].clips[0].events[0]
    {
        *pos += 5;
    }
    rig.app.song_edited(SongEdit::Final, 0);
    rig.frame(vec![]);
    rig.app.files.saved();
    // A long press selects it; the floating row's `Snap` quantizes it.
    let on = cell + vec2(10.0, 0.0);
    rig.press(on);
    for _ in 0..3 {
        rig.frame(vec![]);
    }
    rig.release(on);
    rig.frame(vec![]);
    rig.click_id(egui::Id::new(("note_button", 2_usize)));
    assert_eq!(pos(&rig), on_grid);
    assert!(rig.app.files.modified());
}

#[test]
fn a_save_mid_gesture_then_an_edit_that_changes_nothing_sets_nothing() {
    // A channel param moved under a held pointer is saved with the song; the undo steps take it
    // then, so a final edit after it that changes nothing finds nothing new.
    let mut rig = Rig::with_synth();
    rig.press(OUTSIDE);
    rig.app.song.channels[1].params.0[4] = 0.75;
    rig.app.song_edited(SongEdit::ChannelParam, 0);
    rig.app.files.saved();
    rig.frame(vec![]);
    rig.app.song_edited(SongEdit::Final, 0);
    assert!(!rig.app.files.modified());
}
