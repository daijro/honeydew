//! The scroller's laws, frame by frame (60 frames a second, U = 32 points).

use eframe::egui::{
    Event, Modifiers, MouseWheelUnit, PointerButton, Sense, TouchDeviceId, TouchId, TouchPhase,
    pos2, vec2,
};

use super::*;
use crate::ui::offscreen::Offscreen;

const U: f32 = 32.0;

/// A playlist-like editor: the ruler is y 0..40 right of x 50, the headers x 0..50 below it, the grid the rest.
fn zones() -> Zones {
    let (ruler, headers, grid) = (
        Rect::from_min_max(pos2(50.0, 0.0), pos2(800.0, 40.0)),
        Rect::from_min_max(pos2(0.0, 40.0), pos2(50.0, 400.0)),
        Rect::from_min_max(pos2(50.0, 40.0), pos2(800.0, 400.0)),
    );
    Zones {
        touch: [ruler.union(grid), headers.union(grid)],
        wheel_zoom: [ruler, headers],
        wheel_scroll: grid,
        pinch: grid,
    }
}

/// x: beats, 30 points each, 25 shown of 0..100 (max 75); y: rows, 40 points each, 9 shown of 0..30 (max 21).
fn axes() -> [Axis; 2] {
    [
        Axis {
            pos: 10.0,
            zoom: 1.0 / 30.0,
            range: [0.0, 100.0],
            zoom_range: [1.0 / 300.0, 1.0 / 10.0],
            view: Rangef::new(50.0, 800.0),
        },
        Axis {
            pos: 2.0,
            zoom: 1.0 / 40.0,
            range: [0.0, 30.0],
            zoom_range: [1.0 / 160.0, 1.0 / 20.0],
            view: Rangef::new(40.0, 400.0),
        },
    ]
}

struct Rig {
    off: Offscreen,
    s: Scroller,
    axes: [Axis; 2],
    zones: Zones,
    opts: Options,
    /// A widget over the zones that takes drags (a child control), and whether the host claims every touch for the
    /// scroller or yields it, after the update.
    child: Option<Rect>,
    /// An area on a layer over the editor's (an overlay).
    overlay: Option<Rect>,
    claim: bool,
    yields: bool,
}

impl Rig {
    fn new() -> Self {
        Self {
            off: Offscreen::new(),
            s: Scroller::default(),
            axes: axes(),
            zones: zones(),
            opts: Options::default(),
            child: None,
            overlay: None,
            claim: false,
            yields: false,
        }
    }

    fn frame(&mut self, events: Vec<Event>) -> Report {
        let mut out = Report::default();
        let Self {
            off,
            s,
            axes,
            zones,
            opts,
            child,
            overlay,
            claim,
            yields,
        } = self;
        off.frame(events, &mut |ui, _| {
            out = s.update(ui, Id::new("scroller"), U, zones, axes, *opts);
            if let Some(r) = *overlay {
                egui::Area::new(Id::new("overlay"))
                    .order(egui::Order::Foreground)
                    .fixed_pos(r.min)
                    .show(ui.ctx(), |ui| {
                        ui.allocate_exact_size(r.size(), Sense::click());
                    });
            }
            if let Some(r) = *child {
                ui.interact(r, Id::new("child"), Sense::drag());
            }
            if *claim {
                s.claim();
            }
            if *yields {
                s.yield_touch();
            }
        });
        out
    }

    fn button(&mut self, pos: Pos2, pressed: bool) -> Report {
        self.frame(vec![Event::PointerButton {
            pos,
            button: PointerButton::Primary,
            pressed,
            modifiers: Modifiers::NONE,
        }])
    }

    fn press(&mut self, at: Pos2) -> Report {
        self.frame(vec![Event::PointerMoved(at)]);
        self.button(at, true)
    }

    fn to(&mut self, at: Pos2) -> Report {
        self.frame(vec![Event::PointerMoved(at)])
    }

    fn wheel(&mut self, at: Pos2, delta: Vec2) -> Report {
        self.frame(vec![
            Event::PointerMoved(at),
            Event::MouseWheel {
                unit: MouseWheelUnit::Point,
                delta,
                modifiers: Modifiers::NONE,
            },
        ])
    }

    fn pos(&self) -> [f64; 2] {
        [self.axes[X].pos, self.axes[Y].pos]
    }
}

fn touch(id: u64, phase: TouchPhase, pos: Pos2) -> Event {
    Event::Touch {
        device_id: TouchDeviceId(1),
        id: TouchId(id),
        phase,
        pos,
        force: None,
    }
}

#[track_caller]
fn near(a: f64, b: f64) {
    assert!((a - b).abs() < 1e-6, "{a} != {b}");
}

#[test]
fn each_axis_stays_calm_for_half_a_u_then_follows_the_finger_one_to_one() {
    let mut rig = Rig::new();
    rig.press(pos2(400.0, 200.0));
    // 10 and 5 points: both calm.
    rig.to(pos2(410.0, 205.0));
    assert_eq!(rig.pos(), [10.0, 2.0]);
    // y is 25 points out, x 12: only y follows, by this move's 20 points.
    let r = rig.to(pos2(412.0, 225.0));
    near(rig.axes[Y].pos, 2.0 - 20.0 / 40.0);
    assert_eq!(rig.axes[X].pos, 10.0);
    assert_eq!(r.scrolled, [false, true]);
    // x is 40 points out: it follows by this move's 28 points.
    rig.to(pos2(440.0, 225.0));
    near(rig.axes[X].pos, 10.0 - 28.0 / 30.0);
    // From then on 1:1 both ways.
    rig.to(pos2(430.0, 245.0));
    near(rig.axes[X].pos, 10.0 - 18.0 / 30.0);
    near(rig.axes[Y].pos, 1.0);
}

#[test]
fn past_either_end_the_finger_moves_the_content_a_fifth_as_far() {
    let mut rig = Rig::new();
    rig.axes[Y].pos = 0.25;
    rig.press(pos2(400.0, 200.0));
    // 30 points down: 10 to the top end 1:1, the other 20 at 0.2.
    rig.to(pos2(400.0, 230.0));
    near(rig.axes[Y].pos, -0.2 * 20.0 / 40.0);
    rig.to(pos2(400.0, 280.0));
    near(rig.axes[Y].pos, -0.1 - 0.2 * 50.0 / 40.0);
    // Back towards the range, still past the end: a fifth too.
    rig.to(pos2(400.0, 270.0));
    near(rig.axes[Y].pos, -0.3);
}

#[test]
fn released_past_the_end_the_overshoot_shrinks_ten_percent_a_frame_and_settles_on_the_end() {
    let mut rig = Rig::new();
    rig.axes[Y].pos = 0.0;
    rig.press(pos2(400.0, 200.0));
    // 15 points: still calm. Then 100 more, all past the end.
    rig.to(pos2(400.0, 215.0));
    rig.to(pos2(400.0, 315.0));
    near(rig.axes[Y].pos, -0.2 * 100.0 / 40.0);
    // From the release frame on the spring takes over: v ← 0.5·(pos − min), pos −= 0.2·v.
    rig.button(pos2(400.0, 315.0), false);
    near(rig.axes[Y].pos, -0.45);
    let mut over = 0.45;
    for _ in 0..20 {
        let r = rig.frame(vec![]);
        over *= 0.9;
        near(rig.axes[Y].pos, -over);
        assert!(r.scrolled[Y]);
    }
    // |v| < 0.001 stops it, on the end.
    for _ in 0..60 {
        rig.frame(vec![]);
    }
    assert_eq!(rig.axes[Y].pos, 0.0);
    assert_eq!(rig.frame(vec![]).scrolled, [false; 2]);
}

#[test]
fn a_fast_last_move_flings_on_release_and_the_fling_decays_by_nine_tenths_a_frame() {
    let mut rig = Rig::new();
    rig.press(pos2(400.0, 200.0));
    rig.to(pos2(440.0, 200.0));
    rig.to(pos2(460.0, 200.0));
    let at = 10.0 - 60.0 / 30.0;
    near(rig.axes[X].pos, at);
    // v = zoom·20 points; each frame from the release on v ← 0.9·v, pos −= v.
    let (mut v, mut pos) = (20.0 / 30.0, at);
    for k in 0..5 {
        if k == 0 {
            rig.button(pos2(460.0, 200.0), false);
        } else {
            rig.frame(vec![]);
        }
        v *= 0.9;
        pos -= v;
        near(rig.axes[X].pos, pos);
    }
    assert_eq!(rig.axes[Y].pos, 2.0, "y did not move");
    // Any wheel event stops it, even one over no zone of the editor.
    rig.wheel(pos2(1000.0, 500.0), vec2(0.0, 1.0));
    let stopped = rig.axes[X].pos;
    for _ in 0..5 {
        rig.frame(vec![]);
    }
    assert_eq!(rig.axes[X].pos, stopped);
}

#[test]
fn a_slow_or_stale_last_move_does_not_fling() {
    // The last move 3 points (≤ 0.125 U).
    let mut rig = Rig::new();
    rig.press(pos2(400.0, 200.0));
    rig.to(pos2(440.0, 200.0));
    rig.to(pos2(443.0, 200.0));
    rig.button(pos2(443.0, 200.0), false);
    let at = rig.axes[X].pos;
    for _ in 0..5 {
        rig.frame(vec![]);
    }
    assert_eq!(rig.axes[X].pos, at);
    // A fast move, then the finger rests (no move for 10 frames) before the lift.
    let mut rig = Rig::new();
    rig.press(pos2(400.0, 200.0));
    rig.to(pos2(440.0, 200.0));
    for _ in 0..10 {
        rig.frame(vec![]);
    }
    rig.button(pos2(440.0, 200.0), false);
    let at = rig.axes[X].pos;
    rig.frame(vec![]);
    assert_eq!(rig.axes[X].pos, at);
}

#[test]
fn a_touch_that_leaves_neither_calm_zone_is_a_tap_or_a_long_tap() {
    let mut rig = Rig::new();
    let at = pos2(400.0, 200.0);
    rig.press(at);
    // 12 points each way: over 0.5 U from the touch-down, but neither axis left its own calm zone.
    rig.to(at + vec2(12.0, 12.0));
    let r = rig.button(at + vec2(12.0, 12.0), false);
    assert_eq!(r.tap, Some(Tap { at, long: false }));
    assert_eq!(rig.pos(), [10.0, 2.0]);
    rig.press(at);
    for _ in 0..18 {
        assert_eq!(rig.frame(vec![]).tap, None);
    }
    assert_eq!(rig.button(at, false).tap, Some(Tap { at, long: true }));
    // A touch that scrolled is no tap.
    rig.press(at);
    rig.to(at + vec2(0.0, 30.0));
    rig.to(at);
    assert_eq!(rig.button(at, false).tap, None);
}

#[test]
fn a_touch_outside_every_touch_zone_or_taken_by_a_child_is_not_the_scrollers() {
    // The corner above the headers is in no touch zone.
    let mut rig = Rig::new();
    rig.press(pos2(20.0, 20.0));
    rig.to(pos2(20.0, 120.0));
    rig.to(pos2(220.0, 120.0));
    assert_eq!(rig.button(pos2(220.0, 120.0), false).tap, None);
    assert_eq!(rig.pos(), [10.0, 2.0]);
    assert_eq!(rig.s.left_calm(), None);
    // A widget over the grid that takes drags gets the touch first.
    let mut rig = Rig::new();
    rig.child = Some(Rect::from_min_size(pos2(500.0, 200.0), vec2(100.0, 100.0)));
    rig.frame(vec![]);
    rig.press(pos2(550.0, 250.0));
    rig.to(pos2(550.0, 350.0));
    assert_eq!(rig.button(pos2(550.0, 350.0), false).tap, None);
    assert_eq!(rig.pos(), [10.0, 2.0]);
    // Unless the host hands it on (a grid's empty space): then it scrolls and taps.
    rig.claim = true;
    rig.press(pos2(550.0, 250.0));
    assert_eq!(rig.s.left_calm(), Some(false));
    rig.to(pos2(550.0, 290.0));
    assert_eq!(rig.s.left_calm(), Some(true));
    near(rig.axes[Y].pos, 2.0 - 40.0 / 40.0);
    // A child that takes it later (a marquee, a ruler range) stops it: no more scroll, no fling.
    rig.yields = true;
    rig.frame(vec![]);
    rig.to(pos2(550.0, 330.0));
    rig.button(pos2(550.0, 330.0), false);
    rig.frame(vec![]);
    near(rig.axes[Y].pos, 1.0);
}

#[test]
fn follow_playhead_turns_the_one_finger_x_scroll_off() {
    let mut rig = Rig::new();
    rig.opts.follow = true;
    rig.press(pos2(400.0, 200.0));
    rig.to(pos2(500.0, 210.0));
    rig.to(pos2(600.0, 250.0));
    assert_eq!(rig.axes[X].pos, 10.0);
    near(rig.axes[Y].pos, 2.0 - 40.0 / 40.0);
}

/// The first finger down at `a` (driving the pointer, as an integration does), the second at `b`.
fn two_fingers(rig: &mut Rig, a: Pos2, b: Pos2) {
    rig.frame(vec![Event::PointerMoved(a)]);
    rig.frame(vec![
        touch(1, TouchPhase::Start, a),
        Event::PointerButton {
            pos: a,
            button: PointerButton::Primary,
            pressed: true,
            modifiers: Modifiers::NONE,
        },
    ]);
    rig.frame(vec![touch(2, TouchPhase::Start, b)]);
}

#[test]
fn two_fingers_zoom_each_axis_about_the_other_finger_past_the_thresholds() {
    let mut rig = Rig::new();
    let (a, b) = (pos2(300.0, 200.0), pos2(500.0, 200.0));
    two_fingers(&mut rig, a, b);
    // Spread 200 → 250 points in x: zoom ← zoom·200/250, the content under the first finger stays. In y the fingers
    // are 0 apart (≤ T = 2 U): no zoom.
    // In y they go from 0 to 170 apart: the previous separation is within T, so y does not zoom.
    let r = rig.frame(vec![touch(2, TouchPhase::Move, pos2(550.0, 370.0))]);
    let under = 10.0 + 250.0 / 30.0;
    near(rig.axes[X].zoom, 1.0 / 30.0 * 0.8);
    near(rig.axes[X].pos, under - 250.0 * rig.axes[X].zoom);
    assert_eq!((rig.axes[Y].zoom, rig.axes[Y].pos), (1.0 / 40.0, 2.0));
    assert_eq!(r.zoomed, [true, false]);
    assert_eq!(r.scrolled, [false; 2]);
    // x zoomed on that move, so y needs 6 U (192 points) on this one: 170 → 180 apart is not enough (it is over 5 U).
    rig.frame(vec![touch(2, TouchPhase::Move, pos2(550.0, 380.0))]);
    assert_eq!(rig.axes[Y].zoom, 1.0 / 40.0);
    // x did not zoom on that move (no x motion): 2 U again, and 180 → 190 zooms y about the first finger.
    rig.frame(vec![touch(2, TouchPhase::Move, pos2(550.0, 390.0))]);
    near(rig.axes[Y].zoom, 1.0 / 40.0 * 180.0 / 190.0);
    near(rig.axes[Y].pos, 6.0 - 160.0 * rig.axes[Y].zoom);
    // Pinching together zooms out (6 U apart at least: y zoomed on the previous move), then to the clamp.
    let zoom = rig.axes[X].zoom;
    rig.frame(vec![touch(2, TouchPhase::Move, pos2(500.0, 390.0))]);
    assert!(rig.axes[X].zoom > zoom);
    rig.frame(vec![touch(2, TouchPhase::Move, pos2(370.0, 390.0))]);
    rig.frame(vec![touch(2, TouchPhase::Move, pos2(1100.0, 390.0))]);
    rig.frame(vec![touch(2, TouchPhase::Move, pos2(365.0, 390.0))]);
    near(rig.axes[X].zoom, 1.0 / 10.0);
    // The fingers lift: no tap, no fling.
    let lift = rig.frame(vec![
        touch(2, TouchPhase::End, pos2(365.0, 390.0)),
        touch(1, TouchPhase::End, a),
        Event::PointerButton {
            pos: a,
            button: PointerButton::Primary,
            pressed: false,
            modifiers: Modifiers::NONE,
        },
    ]);
    assert_eq!(lift.tap, None);
    let at = rig.pos();
    rig.frame(vec![]);
    assert_eq!(rig.pos(), at);
}

#[test]
fn the_first_fingers_zone_decides_which_axes_a_pinch_zooms() {
    // Down on the ruler (the x touch zone only): the pinch zooms x alone.
    let mut rig = Rig::new();
    two_fingers(&mut rig, pos2(300.0, 20.0), pos2(500.0, 300.0));
    rig.frame(vec![touch(2, TouchPhase::Move, pos2(550.0, 380.0))]);
    assert_ne!(rig.axes[X].zoom, 1.0 / 30.0);
    assert_eq!(rig.axes[Y].zoom, 1.0 / 40.0);
    // Down outside every touch zone: no pinch at all.
    let mut rig = Rig::new();
    two_fingers(&mut rig, pos2(20.0, 20.0), pos2(500.0, 300.0));
    rig.frame(vec![touch(2, TouchPhase::Move, pos2(550.0, 380.0))]);
    assert_eq!(rig.axes, axes());
}

#[test]
fn the_wheel_zooms_over_the_zoom_zones_about_the_cursor_and_scrolls_over_the_scroll_zone() {
    let mut rig = Rig::new();
    // The headers: y only, zoom ← zoom·(1 − 0.001·dy), the row under the cursor kept.
    let r = rig.wheel(pos2(20.0, 200.0), vec2(0.0, 200.0));
    near(rig.axes[Y].zoom, 0.8 / 40.0);
    near(rig.axes[Y].pos, 6.0 - 160.0 * 0.8 / 40.0);
    assert_eq!(rig.axes[X], axes()[X]);
    assert_eq!(r.zoomed, [false, true]);
    // The ruler: x only.
    rig.wheel(pos2(400.0, 20.0), vec2(0.0, -100.0));
    near(rig.axes[X].zoom, 1.1 / 30.0);
    near(rig.axes[X].pos, 10.0 + 350.0 / 30.0 - 350.0 * 1.1 / 30.0);
    // The grid: pos_x −= zoom·dx, pos_y −= zoom·dy.
    let (x, y) = (rig.axes[X].pos, rig.axes[Y].pos);
    let r = rig.wheel(pos2(400.0, 200.0), vec2(-30.0, 40.0));
    near(rig.axes[X].pos, x + 30.0 * 1.1 / 30.0);
    near(rig.axes[Y].pos, y - 40.0 * 0.8 / 40.0);
    assert_eq!(r.scrolled, [true, true]);
    // Clamped to the range, no rubber band; the zoom to its limits.
    rig.wheel(pos2(400.0, 200.0), vec2(0.0, 5000.0));
    assert_eq!(rig.axes[Y].pos, 0.0);
    rig.wheel(pos2(20.0, 200.0), vec2(0.0, 900.0));
    near(rig.axes[Y].zoom, 1.0 / 160.0);
    // Only the first zone under the cursor counts, zoom y before zoom x.
    let mut rig = Rig::new();
    rig.zones.wheel_zoom[X] = rig.zones.wheel_zoom[Y];
    rig.wheel(pos2(20.0, 200.0), vec2(0.0, 200.0));
    assert_eq!(rig.axes[X], axes()[X]);
    assert_ne!(rig.axes[Y], axes()[Y]);
}

#[test]
fn a_trackpad_pinch_zooms_x_over_the_pinch_zone_about_the_gesture_point() {
    let mut rig = Rig::new();
    rig.frame(vec![
        Event::PointerMoved(pos2(400.0, 200.0)),
        Event::Zoom(2.0),
    ]);
    near(rig.axes[X].zoom, 1.0 / 60.0);
    near(rig.axes[X].pos, 10.0 + 350.0 / 30.0 - 350.0 / 60.0);
    assert_eq!(rig.axes[Y], axes()[Y]);
    // Over the ruler, outside the pinch zone: nothing.
    rig.frame(vec![
        Event::PointerMoved(pos2(400.0, 20.0)),
        Event::Zoom(2.0),
    ]);
    near(rig.axes[X].zoom, 1.0 / 60.0);
    // Covered by an overlay: no wheel and no pinch.
    rig.opts.covered = true;
    rig.frame(vec![
        Event::PointerMoved(pos2(400.0, 200.0)),
        Event::Zoom(2.0),
    ]);
    rig.wheel(pos2(400.0, 200.0), vec2(0.0, 100.0));
    near(rig.axes[X].zoom, 1.0 / 60.0);
    near(rig.axes[Y].pos, 2.0);
}

#[test]
fn follow_puts_the_playhead_a_quarter_in_within_the_range() {
    let mut x = axes()[X];
    follow(&mut x, 40.0);
    near(x.pos, 40.0 - 25.0 / 4.0);
    follow(&mut x, 2.0);
    assert_eq!(x.pos, 0.0);
    follow(&mut x, 99.0);
    assert_eq!(x.pos, 75.0);
}

#[test]
fn the_scrollbar_shows_the_visible_fraction_and_drags_by_the_range_over_its_length() {
    let mut off = Offscreen::new();
    let bar = Rect::from_min_max(pos2(795.0, 40.0), pos2(800.0, 400.0));
    let mut y = axes()[Y];
    let ink = Color32::from_rgb(100, 150, 200);
    let frame = |off: &mut Offscreen, y: &mut Axis, events| {
        let mut thumb = None;
        let shapes = off.frame(events, &mut |ui, _| {
            thumb = scrollbar(ui, Id::new("bar"), bar, 16.0, y, ink);
        });
        // The rounded ends: discs of half the thickness, in the thumb's colour.
        let ends: Vec<_> = (shapes.iter())
            .filter_map(|s| match &s.shape {
                egui::Shape::Circle(c) => Some((c.center, c.radius, c.fill)),
                _ => None,
            })
            .collect();
        (thumb, ends)
    };
    // 9 of 30 rows show, from row 2: the thumb is 9/30 of 360 points long, 2/30 down.
    let (thumb, ends) = frame(&mut off, &mut y, vec![]);
    let t = thumb.expect("drawn");
    assert_eq!((t.top(), t.height()), (40.0 + 24.0, 108.0));
    assert_eq!(
        ends,
        [
            (pos2(797.5, 66.5), 2.5, ink),
            (pos2(797.5, 169.5), 2.5, ink)
        ]
    );
    // Dragged 60 points: pos += 60·30/360 rows, the thumb under the finger; 1.1 times brighter while pressed.
    let grab = t.center();
    frame(&mut off, &mut y, vec![Event::PointerMoved(grab)]);
    let press = Event::PointerButton {
        pos: grab,
        button: PointerButton::Primary,
        pressed: true,
        modifiers: Modifiers::NONE,
    };
    frame(&mut off, &mut y, vec![press]);
    let (thumb, ends) = frame(
        &mut off,
        &mut y,
        vec![Event::PointerMoved(grab + vec2(0.0, 60.0))],
    );
    near(y.pos, 7.0);
    assert_eq!(ends[0].2, Color32::from_rgb(110, 165, 220));
    let (thumb, _) = (thumb.expect("drawn"), ends);
    assert!((thumb.center().y - (grab.y + 60.0)).abs() < 1e-3);
    // Nothing to scroll: no bar.
    y.range = [0.0, 9.0];
    assert_eq!(frame(&mut off, &mut y, vec![]).0, None);
}

#[test]
fn a_finger_that_goes_down_off_the_editor_does_not_join_its_pinch() {
    // The first finger on the grid; a second one outside every zone (a keyboard-panel key, say) moves.
    let mut rig = Rig::new();
    let (a, b) = (pos2(300.0, 200.0), pos2(900.0, 500.0));
    two_fingers(&mut rig, a, b);
    rig.frame(vec![touch(2, TouchPhase::Move, pos2(950.0, 520.0))]);
    assert_eq!(
        rig.axes[X].zoom,
        1.0 / 30.0,
        "a finger outside the editor zoomed it"
    );
    // Nor does one on another layer over the zones (an overlay drawn above the editor).
    let mut rig = Rig::new();
    rig.overlay = Some(Rect::from_min_size(pos2(450.0, 150.0), vec2(200.0, 200.0)));
    rig.frame(vec![]);
    two_fingers(&mut rig, a, pos2(500.0, 200.0));
    rig.frame(vec![touch(2, TouchPhase::Move, pos2(550.0, 200.0))]);
    assert_eq!(
        rig.axes[X].zoom,
        1.0 / 30.0,
        "a finger on an overlay zoomed it"
    );
}

#[test]
fn a_fast_lift_flings_at_18_frames_a_second() {
    // The release flings by the move of the frame before it, however long that frame was (here 1/18 s).
    let mut rig = Rig::new();
    let slow = 1.0 / 18.0 - 1.0 / 60.0;
    rig.press(pos2(400.0, 200.0));
    rig.off.time += slow;
    rig.to(pos2(440.0, 200.0));
    rig.off.time += slow;
    rig.to(pos2(480.0, 200.0));
    rig.off.time += slow;
    rig.button(pos2(480.0, 200.0), false);
    let at = rig.axes[X].pos;
    rig.off.time += slow;
    rig.frame(vec![]);
    assert_ne!(
        rig.axes[X].pos, at,
        "a 40-points-a-frame drag did not fling at 18 fps"
    );
}

#[test]
fn a_lift_after_a_rest_of_two_or_three_frames_does_not_fling() {
    // The release flings only by a move in its own frame or the one before, even within 80 ms.
    for rest in [2, 3] {
        let mut rig = Rig::new();
        rig.press(pos2(400.0, 200.0));
        rig.to(pos2(440.0, 200.0));
        rig.to(pos2(480.0, 200.0));
        for _ in 0..rest {
            rig.frame(vec![]);
        }
        rig.button(pos2(480.0, 200.0), false);
        let at = rig.axes[X].pos;
        rig.frame(vec![]);
        assert_eq!(
            rig.axes[X].pos,
            at,
            "{rest} frames ({} ms)",
            rest * 1000 / 60
        );
    }
}

#[test]
fn the_lifts_frame_still_tells_whether_the_touch_left_its_calm_zone() {
    // The editors decide a tap at the lift, after the scroller's update: that frame keeps the answer.
    let mut rig = Rig::new();
    let at = pos2(400.0, 200.0);
    rig.press(at);
    rig.to(at + vec2(12.0, 12.0));
    rig.button(at + vec2(12.0, 12.0), false);
    assert_eq!(rig.s.left_calm(), Some(false));
    rig.frame(vec![]);
    assert_eq!(rig.s.left_calm(), None);
    rig.press(at);
    rig.to(at + vec2(0.0, 30.0));
    rig.button(at + vec2(0.0, 30.0), false);
    assert_eq!(rig.s.left_calm(), Some(true));
}

#[test]
fn the_keymaps_zoom_scales_z_by_a_tenth_about_the_views_centre_within_the_zoom_range() {
    // `zoomIn…` z ← z·0.9, `zoomOut…` z ← z·1.1, the anchor fixed, z clamped; Shift picks y (`ui/edit_keys.rs`).
    use crate::ui::edit_keys::tests::tap;
    use eframe::egui::Key;
    let (cmd, shift) = (Modifiers::COMMAND, Modifiers::COMMAND | Modifiers::SHIFT);
    let centre = |ax: &Axis| ax.at(ax.view.center());
    let close = |a: f64, b: f64| (a - b).abs() < 1e-9;
    let mut rig = Rig::new();
    let before = rig.axes;
    let out = rig.frame(tap(Key::Equals, cmd).into());
    assert!(close(rig.axes[X].zoom, before[X].zoom * 0.9));
    assert!(close(centre(&rig.axes[X]), centre(&before[X])));
    assert_eq!(rig.axes[Y], before[Y]);
    assert_eq!(out.zoomed, [true, false]);
    rig.frame(tap(Key::Minus, shift).into());
    assert!(close(rig.axes[Y].zoom, before[Y].zoom * 1.1));
    assert!(close(centre(&rig.axes[Y]), centre(&before[Y])));
    // Clamped to zoomMin.
    for _ in 0..40 {
        rig.frame(tap(Key::Equals, cmd).into());
    }
    assert_eq!(rig.axes[X].zoom, before[X].zoom_range[0]);
    // An axis the editor lacks does not move, and nothing zooms while the editor is covered.
    let mut rig = Rig::new();
    rig.axes[Y] = Axis::OFF;
    rig.frame(tap(Key::Equals, shift).into());
    assert_eq!(rig.axes[Y], Axis::OFF);
    rig.opts.covered = true;
    rig.frame(tap(Key::Equals, cmd).into());
    assert_eq!(rig.axes[X], axes()[X]);
}

#[test]
fn let_go_ends_the_touch_and_springs_back_an_axis_past_its_end_only() {
    // Pulled 0.5 rows past the top, then a child that took the touch lets it go with the finger still down.
    let mut rig = Rig::new();
    rig.axes[Y].pos = 0.0;
    rig.press(pos2(400.0, 200.0));
    rig.to(pos2(400.0, 215.0));
    rig.to(pos2(400.0, 315.0));
    near(rig.axes[Y].pos, -0.5);
    rig.s.let_go(&rig.axes);
    // The overshoot shrinks 10 % a frame; the finger moves nothing, and its lift is no tap and flings nothing.
    let mut over = 0.5;
    for i in 0..6 {
        let r = if i < 5 {
            rig.to(pos2(400.0, 340.0 + 10.0 * i as f32))
        } else {
            rig.button(pos2(400.0, 380.0), false)
        };
        over *= 0.9;
        near(rig.axes[Y].pos, -over);
        assert_eq!(r.tap, None);
    }
    assert_eq!(rig.axes[X].pos, 10.0);
    // In range: nothing springs, and a lift right after a fast move flings nothing.
    let mut rig = Rig::new();
    rig.press(pos2(400.0, 200.0));
    rig.to(pos2(400.0, 230.0));
    let at = rig.axes[Y].pos;
    near(at, 2.0 - 30.0 / 40.0);
    rig.s.let_go(&rig.axes);
    let r = rig.button(pos2(400.0, 230.0), false);
    assert_eq!(r.tap, None);
    for _ in 0..3 {
        rig.frame(vec![]);
        assert_eq!(rig.pos(), [10.0, at]);
    }
}
