//! A finger's tap in the web build (issue #35): eframe's web backend reports it twice, once from the pointer events and
//! once from the touch events.

use eframe::egui::{Event, TouchDeviceId, TouchId, TouchPhase};

use super::*;

/// The frames eframe 0.33's web backend (`web/events.rs`) makes of a finger's tap at `at`: `pointerdown` pushes a
/// press and runs a frame at once; `touchstart` pushes the press again with the touch, for the next frame; `pointerup`
/// pushes the release and runs a frame at once; `touchend` pushes the release again, `PointerGone` and the touch's end.
/// Without the `pointerdown` frame when `lib.rs` keeps a finger's `pointerdown` from eframe.
fn web_tap(at: Pos2, pointerdown: bool) -> Vec<Vec<Event>> {
    let touch = |phase| Event::Touch {
        device_id: TouchDeviceId(0),
        id: TouchId(1),
        phase,
        pos: at,
        force: Some(1.0),
    };
    let (press, release) = (button_event(at, true), button_event(at, false));
    let frames = [
        vec![press.clone()],
        vec![press, touch(TouchPhase::Start)],
        vec![release.clone()],
        vec![release, Event::PointerGone, touch(TouchPhase::End)],
    ];
    frames.into_iter().skip(usize::from(!pointerdown)).collect()
}

#[test]
fn a_finger_tap_on_the_web_toggles_the_current_tracks_column_once() {
    // The master's track is the current one: a click on its header toggles the column.
    let mut rig = Rig::new();
    rig.click(header(0));
    assert_eq!(rig.app.main_screen.column.1, 1.0);
    rig.click(header(0));
    assert_eq!(rig.app.main_screen.column.1, 0.0);
    for pointerdown in [true, false] {
        for open in [1.0, 0.0] {
            for events in web_tap(header(0), pointerdown) {
                rig.frame(events);
            }
            assert_eq!(rig.app.main_screen.column.1, open, "{pointerdown}");
        }
    }
}
