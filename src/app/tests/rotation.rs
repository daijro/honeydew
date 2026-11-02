//! The screen resized between frames: what a phone's rotation does to the surface. The rig's screen is the surface, so
//! a test turns it and lays the next frames out on the new one — the app must draw there and take its touches there,
//! and the views whose layout is the tightest on a short screen must draw on it too.

use super::*;

/// The four ways a phone holds it, a small window's shape, and back to the rig's own.
const SCREENS: [egui::Vec2; 6] = [
    egui::vec2(1170.0, 540.0),
    egui::vec2(540.0, 1170.0),
    egui::vec2(1568.0, 707.0),
    egui::vec2(707.0, 1568.0),
    egui::vec2(240.0, 180.0),
    egui::vec2(1170.0, 540.0),
];

/// The screen `rig` is drawn on now.
fn screen(rig: &Rig) -> egui::Rect {
    egui::Rect::from_min_size(Pos2::ZERO, rig.screen)
}

/// The playlist lays out on whatever screen it is drawn on, and a tap on the transport bar goes where that screen puts
/// it (a layout left on the old screen would put the bar elsewhere).
#[test]
fn a_rotation_lays_the_playlist_out_on_the_new_screen_and_keeps_its_taps() {
    let mut rig = Rig::with_synth();
    for screen in SCREENS {
        rig.screen = screen;
        for _ in 0..3 {
            rig.frame(vec![]);
        }
        assert!(!rig.out.shapes.is_empty(), "{screen:?}: nothing drawn");
    }
    rig.screen = egui::vec2(1170.0, 540.0);
    for _ in 0..3 {
        rig.frame(vec![]);
    }
    rig.click(PLAY);
    assert!(rig.app.main_screen.playing, "a tap on the play button");
}

/// The bottom editors draw on a rotated screen too, their close ribbon on it, in the drum editor and in the piano roll.
#[test]
fn the_bottom_editors_draw_on_a_rotated_screen() {
    let mut rig = Rig::with_drums_and_synth();
    for channel in [1, 2] {
        rig.app.song.channels[channel].lanes[0].add_clip(0, 4.0);
    }
    rig.app.song_edited(SongEdit::Final, 0);
    rig.frame(vec![]);
    let ribbon = egui::Id::new("editor_close");
    for (nav, view) in [
        (Nav::Edit(1), View::DrumEditor(1)),
        (Nav::Edit(2), View::PianoRoll(2)),
    ] {
        for size in SCREENS {
            rig.screen = size;
            rig.app.navigate(nav);
            for _ in 0..3 {
                rig.frame(vec![]);
            }
            assert_eq!(rig.app.nav.view, view, "{size:?}");
            assert!(!rig.out.shapes.is_empty(), "{view:?} on {size:?}");
            // The editor's close ribbon hangs at its top right, inside the screen it is drawn on.
            let at = rig.ctx.read_response(ribbon).expect("the ribbon").rect;
            assert!(
                screen(&rig).contains_rect(at),
                "{view:?} on {size:?}: {at:?}"
            );
        }
    }
}
