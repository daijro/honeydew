//! The automation editor in the app: a tap adds a point and a drag moves it, each one undo step.

use super::*;
use crate::song::points::{self, Point, PointKey, value_at_row};
use crate::ui::preview::ClipKey;

const MAX: u16 = 65534;

/// The master's automation clip at bar 1, open in the automation editor; the audio started and an empty log.
fn editor_rig() -> Rig {
    let mut rig = Rig::new();
    rig.app.song.channels[0].lanes[0].add_clip(0, 4.0);
    rig.app.song_edited(SongEdit::Final, 0);
    rig.click(OUTSIDE);
    rig.take();
    rig.app.navigate(Nav::EditClip(ClipKey {
        channel: 0,
        lane: 0,
        id: 1,
    }));
    rig.slide_split();
    assert_eq!(rig.app.nav.view, View::AutomationEditor(0));
    rig
}

fn pts(rig: &Rig) -> Vec<Point> {
    let clip = &rig.app.song.channels[0].lanes[0].clips[0];
    points::points(clip, PointKey::Automation)
}

#[test]
fn a_tap_adds_a_point_and_a_drag_moves_it_each_in_one_undo_step() {
    let mut rig = editor_rig();
    let grid = (rig
        .ctx
        .read_response(egui::Id::new("automation_editor").with("grid")))
    .expect("the grid")
    .rect;
    // The value area: the grid inset by min(0.5 U, G − 3.5 U); 127 rows. A new clip's view: 4 U (128 points) a beat
    // from beat 0, so a tick is a point.
    let pad = 16f32.min(grid.height() - 112.0).max(0.0);
    let (top, v) = (grid.top() + pad, grid.height() - 2.0 * pad);
    let at = pos2(grid.left() + 64.0, top + 10.0 * v / 127.0);
    rig.click(at);
    rig.frame(vec![]);
    let first = Point {
        tick: 64,
        value: value_at_row(10.0, MAX),
        ..Default::default()
    };
    assert_eq!(pts(&rig), [first]);
    // A beat right and 20 rows down, in one step at the release.
    rig.press(at);
    rig.drag_to(at + vec2(64.0, 0.0));
    let to = at + vec2(128.0, 20.0 * v / 127.0);
    rig.drag_to(to);
    assert_eq!(pts(&rig), [first]);
    rig.release(to);
    rig.frame(vec![]);
    let moved = Point {
        tick: 192,
        value: value_at_row(points::row(first.value, MAX) + 20.0, MAX),
        ..Default::default()
    };
    assert_eq!(pts(&rig), [moved]);
    rig.frame(z(egui::Modifiers::NONE));
    assert_eq!(pts(&rig), [first]);
    rig.frame(z(egui::Modifiers::NONE));
    assert_eq!(pts(&rig), []);
    // The setup's step is next: the clip goes, and the editor with it.
    rig.frame(z(egui::Modifiers::NONE));
    rig.frame(vec![]);
    assert_eq!(rig.app.nav.view, View::Playlist);
}

#[test]
fn the_editors_saved_view_follows_its_track() {
    use crate::song::AutomationTarget;
    use crate::ui::track_menu::TrackOp;
    let mut rig = Rig::with_drums_and_synth();
    let level = AutomationTarget {
        instance_id: 0,
        param: 0,
    };
    let clip = (rig.app.song.channels[2].add_automation_lane(level)).add_clip(0, 16.0);
    let ends = vec![
        Point {
            tick: 0,
            value: 0,
            ..Default::default()
        },
        Point {
            tick: 2047,
            value: MAX,
            ..Default::default()
        },
    ];
    points::set_points(clip, PointKey::Automation, ends);
    rig.app.song_edited(SongEdit::Final, 0);
    rig.app.navigate(Nav::EditClip(ClipKey {
        channel: 2,
        lane: 1,
        id: 1,
    }));
    rig.slide_split();
    rig.frame(vec![]);
    // Panned two beats on (128 points a beat) on the ruler band, the touch x zone.
    let grid = (rig
        .ctx
        .read_response(egui::Id::new("automation_editor").with("grid")))
    .expect("the grid")
    .rect;
    let from = pos2(grid.left() + 600.0, grid.top() - 0.75 * U);
    rig.press(from);
    rig.drag_to(from - vec2(128.0, 0.0));
    rig.drag_to(from - vec2(256.0, 0.0));
    rig.release(from - vec2(256.0, 0.0));
    let left = |rig: &Rig, lane: usize| {
        let views = &rig.app.slots.automation_editor.editor.views;
        views.get(&(2, lane, 1)).map(|v| v.left)
    };
    assert_eq!(left(&rig, 1), Some(2.0));
    // A track added before it: the view goes with the clip to lane 2.
    rig.app.track_op(2, 0, TrackOp::AddTrack, 0);
    rig.frame(vec![]);
    assert_eq!(rig.app.nav.view, View::AutomationEditor(2));
    assert_eq!((left(&rig, 1), left(&rig, 2)), (None, Some(2.0)));
}

/// Where the last frame painted `text`.
fn text_at(rig: &Rig, text: &str) -> Pos2 {
    let found = rig.out.shapes.iter().find_map(|s| match &s.shape {
        egui::Shape::Text(t) if t.galley.text() == text => Some(t.visual_bounding_rect().center()),
        _ => None,
    });
    found.unwrap_or_else(|| panic!("{text:?} not painted"))
}

#[test]
fn set_shape_square_on_two_selected_points_is_one_undo_step() {
    use crate::song::points::{Shape, ShapeKind};
    let mut rig = editor_rig();
    let pt = |tick, value| Point {
        tick,
        value,
        ..Point::default()
    };
    let line = vec![pt(0, 0), pt(128, MAX), pt(256, 0)];
    let clip = &mut rig.app.song.channels[0].lanes[0].clips[0];
    points::set_points(clip, PointKey::Automation, line.clone());
    rig.app.song_edited(SongEdit::Final, 0);
    rig.frame(vec![]);
    let grid = (rig
        .ctx
        .read_response(egui::Id::new("automation_editor").with("grid")))
    .expect("the grid")
    .rect;
    let pad = 16f32.min(grid.height() - 112.0).max(0.0);
    let (top, bottom) = (grid.top() + pad, grid.bottom() - pad);
    // A long press on empty grid, then a box over the last two points (a tick is a point).
    let from = pos2(grid.left() + 60.0, top + 2.0);
    rig.press(from);
    for _ in 0..4 {
        rig.frame(vec![]);
    }
    rig.drag_to(pos2(grid.left() + 300.0, bottom - 2.0));
    rig.release(pos2(grid.left() + 300.0, bottom - 2.0));
    rig.frame(vec![]);
    rig.click(text_at(&rig, "Set shape"));
    // A new area shows from its second frame.
    rig.frame(vec![]);
    rig.frame(vec![]);
    // The pick shows on the curve only while the popup is open; its close is the edit.
    rig.click(text_at(&rig, "Square"));
    rig.frame(vec![]);
    assert_eq!(pts(&rig), line);
    rig.click(pos2(1100.0, 530.0));
    rig.frame(vec![]);
    let square = Shape {
        kind: ShapeKind::Square,
        tension: 50,
    };
    let mut want = line.clone();
    (want[1].shape, want[2].shape) = (square, square);
    assert_eq!(pts(&rig), want);
    rig.frame(z(egui::Modifiers::NONE));
    assert_eq!(pts(&rig), line);
}
