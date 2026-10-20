//! Dragging a note does not bring up its menu: it just selects it, and a second click opens it. An item drag's release
//! is neither a tap nor a long press: the item stays selected with its handles and floating row hidden, until a tap or
//! A long press on it shows them. The same in every editor with the shared law.

use eframe::egui::{Rect, Shape};

use super::*;
use crate::song::{DrumGrid, LaneKind};
use crate::ui::preview::ClipKey;

fn rect(rig: &Rig, id: egui::Id) -> Rect {
    rig.ctx.read_response(id).expect("drawn").rect
}

/// The rounded `text` rects of a handle's size centred in `area` last frame: the resize handles' capsule bodies.
fn handles(rig: &Rig, area: Rect) -> usize {
    let text = rig.app.theme.colors.text.to_color32();
    (rig.out.shapes.iter())
        .filter(|c| {
            matches!(&c.shape, Shape::Rect(r)
                if r.fill == text && r.corner_radius != egui::CornerRadius::ZERO
                    && area.contains(r.rect.center()) && (4.0..60.0).contains(&r.rect.width())
                    && r.rect.height() < 70.0)
        })
        .count()
}

/// The sequence on the item at `from` in `area`, whose selection draws `n` handles: dragged to `to` and let go, it
/// shows no floating row and no handle; a tap on it shows both, a second tap deselects it, and a long press shows both
/// again. Dragged back, an empty tap adds a piano note immediately; the other editors first deselect, then add.
/// Each is checked two frames on, the playlist's handles popping in from nothing on the first.
fn drag_then_tap(
    rig: &mut Rig,
    editor: &str,
    area: Rect,
    (from, to, empty): (Pos2, Pos2, Pos2),
    n: usize,
    items: &dyn Fn(&Rig) -> usize,
) {
    let shown = |rig: &mut Rig| {
        rig.frame(vec![]);
        rig.frame(vec![]);
        (rig.painted("Copy"), handles(rig, area))
    };
    let drag = |rig: &mut Rig, from: Pos2, to: Pos2| {
        rig.press(from);
        rig.drag_to(from + (to - from) / 2.0);
        rig.drag_to(to);
        rig.release(to);
    };
    drag(rig, from, to);
    assert_eq!(shown(rig), (false, 0), "{editor}: the drag's release");
    rig.click(to);
    assert_eq!(shown(rig), (true, n), "{editor}: a tap shows them");
    rig.click(to);
    assert_eq!(shown(rig), (false, 0), "{editor}: a second tap deselects");
    // A long press: released in place 0.4 s after the press (the rig's frames are 0.1 s apart).
    rig.press(to);
    for _ in 0..3 {
        rig.frame(vec![]);
    }
    rig.release(to);
    assert_eq!(shown(rig), (true, n), "{editor}: a long press shows them");
    drag(rig, to, from);
    assert_eq!(shown(rig), (false, 0), "{editor}: dragged back");
    let count = items(rig);
    rig.click(empty);
    let tapped = (shown(rig), items(rig));
    let added = usize::from(editor == "piano roll");
    assert_eq!(
        tapped,
        ((false, 0), count + added),
        "{editor}: an empty tap"
    );
    rig.click(empty);
    rig.frame(vec![]);
    assert_eq!(
        items(rig),
        count + 1,
        "{editor}: one new item after two taps"
    );
}

#[test]
fn a_dragged_item_keeps_its_row_and_handles_hidden_until_a_tap_in_every_editor() {
    // The piano roll: a tap adds a note at the visible cell's start, a beat (64 points) long.
    let mut rig = Rig::with_drums_and_synth();
    rig.edit(2);
    rig.slide_split();
    let grid = rect(&rig, egui::Id::new("piano_roll").with("grid"));
    let cell = pos2(60.0, 200.0);
    rig.click(cell);
    let on = cell + vec2(10.0, 0.0);
    let notes = |rig: &Rig| rig.app.song.channels[2].lanes[0].clips[0].events.len();
    let taps = (on, on + vec2(64.0, 0.0), pos2(400.0, 280.0));
    drag_then_tap(&mut rig, "piano roll", grid, taps, 1, &notes);

    // The drum note grid: a tap adds a hit (at 1 point a tick: 608..640, x 704..736).
    let mut rig = Rig::with_drums_and_synth();
    let drums = &mut rig.app.song.channels[1];
    let notes = drums.add_lane(LaneKind::Drums);
    drums.lanes[notes].grid = DrumGrid::Notes;
    drums.lanes[notes].add_clip(0, 16.0);
    rig.app.song_edited(SongEdit::Final, 0);
    let key = |channel, lane| ClipKey {
        channel,
        lane,
        id: 1,
    };
    rig.app.navigate(Nav::EditClip(key(1, notes)));
    rig.slide_split();
    rig.frame(vec![]);
    let grid = rect(&rig, egui::Id::new("drum_grid").with("grid"));
    let hit = pos2(710.0, grid.top() + U);
    rig.click(hit);
    let hits = |rig: &Rig| rig.app.song.channels[1].lanes[notes].clips[0].events.len();
    let taps = (hit, hit + vec2(64.0, 0.0), pos2(400.0, hit.y));
    drag_then_tap(&mut rig, "drum grid", grid, taps, 1, &hits);

    // The automation editor: a tap adds a point (4 U a beat, a tick a point). It draws no handle for one point.
    let mut rig = Rig::new();
    rig.app.song.channels[0].lanes[0].add_clip(0, 4.0);
    rig.app.song_edited(SongEdit::Final, 0);
    rig.app.navigate(Nav::EditClip(key(0, 0)));
    rig.slide_split();
    let grid = rect(&rig, egui::Id::new("automation_editor").with("grid"));
    let at = pos2(grid.left() + 64.0, grid.center().y);
    rig.click(at);
    rig.frame(vec![]);
    let points = |rig: &Rig| {
        let clip = &rig.app.song.channels[0].lanes[0].clips[0];
        crate::song::points::points(clip, crate::song::points::PointKey::Automation).len()
    };
    let taps = (at, at + vec2(128.0, 20.0), at + vec2(400.0, -40.0));
    drag_then_tap(&mut rig, "automation", grid, taps, 0, &points);

    // The playlist: a tap makes the clip current (a current clip drags at once); a notes clip has two handles.
    let mut rig = Rig::with_drums_and_synth();
    rig.app.song.channels[2].lanes[0].add_clip(0, 16.0);
    rig.frame(vec![]);
    let grid = Rect::from_min_max(pos2(1.5 * U, 1.5 * U), pos2(1170.0, PANEL_TOP));
    let at = lane(2);
    rig.click(at);
    // Its empty-tap item is the insert menu on the current track, past the clip's end (x 739 once moved).
    let offers = |rig: &Rig| usize::from(rig.painted("New Clip"));
    let taps = (at, at + vec2(77.0, 0.0), at + vec2(500.0, 0.0));
    drag_then_tap(&mut rig, "playlist", grid, taps, 2, &offers);
}
