//! The automation editor offscreen. U = 32 points at ppp 1 and 2 (the desktop boost), so the layout is the same in
//! points at both densities.

use eframe::egui::{
    self, Color32, Context, Modifiers, PointerButton, Pos2, RawInput, Rect, Shape, Vec2,
    epaint::ClippedShape, pos2, vec2,
};

use super::*;
use crate::metrics::{DEFAULT_UI_SCALE, Metrics};
use crate::song::points::{self, PointKey, value_at_row};
use crate::song::{AutomationTarget, Song};
use crate::theme::{channel_color, theme_by_id};

const SCREEN: Rect = Rect {
    min: pos2(0.0, 0.0),
    max: pos2(1170.0, 540.0),
};
/// The editor: its ruler, from a little above the screen's top down to y 48 (so that all below it lies on whole
/// points), then G = 286 with pad 16, so V = 254 (2 points a row). The grid starts at x 50 (1.5 U + ceil(U/16)); a
/// clip with no saved view shows 128 points a beat from its first point's beat, here 0, so tick t sits at x 50 + t.
const RECT: Rect = Rect {
    min: pos2(0.0, RULER_BOTTOM - 52.0),
    max: pos2(1050.0, 334.0),
};
/// The ruler's bottom in [`RECT`].
const RULER_BOTTOM: f32 = 48.0;

/// [`RECT`] at `ppp`: the ruler is cut to whole device pixels (52.5 points at ppp 2), so its top moves to keep its
/// bottom at [`RULER_BOTTOM`].
fn rect_at(ppp: f32) -> Rect {
    let m = Metrics::from_pixels_per_point(ppp, crate::metrics::DEFAULT_UI_SCALE);
    RECT.with_min_y(RULER_BOTTOM - crate::ui::timeline::ruler_h(&m))
}
const GRID_X: f32 = 50.0;
const TOP: f32 = 64.0;
const V: f32 = 254.0;
const MAX: u16 = 65534;
const LINE: points::Shape = points::Shape {
    kind: points::ShapeKind::Linear,
    tension: 50,
};
const TARGET: AutomationTarget = AutomationTarget {
    instance_id: 0,
    param: 4,
};

fn p(tick: u32, value: u16) -> Point {
    Point {
        tick,
        value,
        ..Default::default()
    }
}

/// Where a touch binds `q`: its tick's x and its row's y (r without the +0.5).
fn touch(q: Point) -> Pos2 {
    pos2(
        GRID_X + q.tick as f32,
        TOP + 2.0 * points::row(q.value, MAX) as f32,
    )
}

/// Where `q` is drawn: half a row lower.
fn drawn(q: Point) -> Pos2 {
    touch(q) + vec2(0.0, 1.0)
}

/// The master with an automation lane on its Pitch Bend (at 0.5; its generic display reads percent), holding a 4-beat
/// clip at song tick 0 with `pts`.
fn song(pts: &[Point]) -> (Song, ClipKey) {
    song_on(TARGET, pts)
}

/// [`song`] with the lane on `target`.
fn song_on(target: AutomationTarget, pts: &[Point]) -> (Song, ClipKey) {
    let mut song = Song::default();
    let ch = &mut song.channels[0];
    ch.params.0[4] = 0.5;
    ch.add_automation_lane(target);
    let lane = (ch.lanes.iter())
        .position(|l| l.target == Some(target))
        .expect("the lane");
    let clip = ch.lanes[lane].add_clip(0, 4.0);
    points::set_points(clip, PointKey::Automation, pts.to_vec());
    let id = clip.id;
    (
        song,
        ClipKey {
            channel: 0,
            lane,
            id,
        },
    )
}

fn colors() -> (Color32, Color32, Color32, Color32) {
    let theme = theme_by_id(0);
    let c = &theme.colors;
    let ink = channel_color(Song::default().channels[0].hue, &theme).to_color32();
    (
        c.background.to_color32(),
        c.text.to_color32(),
        c.menu_fill.to_color32(),
        ink,
    )
}

struct Rig {
    ctx: Context,
    ed: AutomationEditor,
    song: Song,
    key: ClipKey,
    opts: Options,
    rect: Rect,
    ppp: f32,
    time: f64,
    /// Frames that reported an edit, and the seeks asked for.
    edits: usize,
    seeks: Vec<u64>,
    shapes: Vec<ClippedShape>,
    /// Held through every frame (Shift: the selection modifier).
    modifiers: Modifiers,
}

impl Rig {
    fn new(pts: &[Point]) -> Self {
        Self::at(pts, 1.0, RECT)
    }

    fn at(pts: &[Point], ppp: f32, rect: Rect) -> Self {
        let (song, key) = song(pts);
        let mut rig = Self {
            ctx: Context::default(),
            ed: AutomationEditor::default(),
            song,
            key,
            opts: Options::default(),
            rect,
            ppp,
            time: 0.0,
            edits: 0,
            seeks: Vec::new(),
            shapes: Vec::new(),
            modifiers: Modifiers::NONE,
        };
        for _ in 0..3 {
            rig.frame(vec![]);
        }
        rig
    }

    fn frame(&mut self, events: Vec<egui::Event>) {
        self.frame_after(0.1, events);
    }

    fn frame_after(&mut self, dt: f64, events: Vec<egui::Event>) {
        self.time += dt;
        let mut raw = RawInput {
            screen_rect: Some(SCREEN),
            time: Some(self.time),
            modifiers: self.modifiers,
            events,
            ..Default::default()
        };
        let viewport = raw.viewports.entry(raw.viewport_id).or_default();
        viewport.native_pixels_per_point = Some(self.ppp);
        let (theme, m) = (
            theme_by_id(0),
            Metrics::from_pixels_per_point(self.ppp, DEFAULT_UI_SCALE),
        );
        let cx = UiCtx {
            theme: &theme,
            m: &m,
        };
        let (ed, song, key, opts, rect) =
            (&mut self.ed, &mut self.song, self.key, self.opts, self.rect);
        let mut out = Output::default();
        let output = self.ctx.run(raw, |ctx| {
            egui::CentralPanel::default()
                .frame(egui::Frame::NONE)
                .show(ctx, |ui| out = ed.show(ui, &cx, rect, song, key, &opts));
        });
        self.shapes = output.shapes;
        self.edits += usize::from(out.edited);
        self.seeks.extend(out.seek);
    }

    fn button(&mut self, pos: Pos2, pressed: bool) {
        self.frame(vec![egui::Event::PointerButton {
            pos,
            button: PointerButton::Primary,
            pressed,
            modifiers: self.modifiers,
        }]);
    }

    fn press(&mut self, pos: Pos2) {
        self.frame(vec![egui::Event::PointerMoved(pos)]);
        self.button(pos, true);
    }

    fn release(&mut self, pos: Pos2) {
        self.button(pos, false);
        // Lets the floating row or a popup lay itself out.
        self.frame(vec![]);
    }

    fn tap(&mut self, pos: Pos2) {
        self.press(pos);
        self.release(pos);
    }

    /// Presses at `from`, moves through `offsets` from it, one frame each, and releases at the last.
    fn drag(&mut self, from: Pos2, offsets: &[Vec2]) {
        self.press(from);
        for &d in offsets {
            self.frame(vec![egui::Event::PointerMoved(from + d)]);
        }
        self.release(from + offsets.last().copied().unwrap_or_default());
    }

    /// Pressed at `pos` and held there past 300 ms: a long press.
    fn hold(&mut self, pos: Pos2) {
        self.press(pos);
        self.frame_after(0.31, vec![]);
    }

    fn move_to(&mut self, pos: Pos2) {
        self.frame(vec![egui::Event::PointerMoved(pos)]);
    }

    fn clip(&self) -> &crate::song::Clip {
        self.key.clip(&self.song).expect("the clip")
    }

    fn points(&self) -> Vec<Point> {
        points::points(self.clip(), PointKey::Automation)
    }

    fn text_at(&self, label: &str) -> Option<Pos2> {
        self.shapes.iter().find_map(|s| match &s.shape {
            Shape::Text(t) if t.galley.text() == label => Some(t.visual_bounding_rect().center()),
            _ => None,
        })
    }

    fn texts(&self, color: Color32) -> Vec<(String, Pos2)> {
        (self.shapes.iter())
            .filter_map(|s| match &s.shape {
                Shape::Text(t) if t.galley.job.sections[0].format.color == color => Some((
                    t.galley.text().to_owned(),
                    t.visual_bounding_rect().center(),
                )),
                _ => None,
            })
            .collect()
    }

    fn rects(&self, color: Color32) -> Vec<Rect> {
        (self.shapes.iter())
            .filter_map(|s| match &s.shape {
                Shape::Rect(r) if r.fill == color => Some(r.rect),
                _ => None,
            })
            .collect()
    }

    /// The filled circles: centre, radius and colour.
    fn discs(&self) -> Vec<(Pos2, f32, Color32)> {
        (self.shapes.iter())
            .filter_map(|s| match &s.shape {
                Shape::Circle(c) if c.fill != Color32::TRANSPARENT => {
                    Some((c.center, c.radius, c.fill))
                }
                _ => None,
            })
            .collect()
    }

    /// The radius of the disc drawn at `at`.
    fn radius_at(&self, at: Pos2) -> f32 {
        (self.discs().into_iter())
            .find(|d| d.0.distance(at) < 0.01)
            .map_or(0.0, |d| d.1)
    }

    fn segments(&self) -> Vec<([Pos2; 2], f32, Color32)> {
        (self.shapes.iter())
            .filter_map(|s| match &s.shape {
                Shape::LineSegment { points, stroke } => {
                    Some((*points, stroke.width, stroke.color))
                }
                _ => None,
            })
            .collect()
    }

    fn selected(&self) -> Vec<bool> {
        self.ed.sel.clone()
    }

    /// The clip's view as the editor keeps it.
    fn view(&self) -> View {
        self.ed.views[&(self.key.channel, self.key.lane, self.key.id)]
    }

    fn set_loop(&mut self, beats: f64) {
        self.song.channels[0].lanes[self.key.lane].clips[0].loop_len_beats = beats;
    }

    /// The 1 px `text` guides: (vertical, the x or y of their middle), sorted.
    fn guides(&self) -> Vec<(bool, f32)> {
        let (_, text, _, _) = colors();
        let mut guides: Vec<(bool, f32)> = (self.rects(text).into_iter())
            .filter_map(|r| {
                if r.width() == 1.0 && r.y_range() == (48.0..=334.0) {
                    Some((true, r.center().x))
                } else if r.height() == 1.0 && r.x_range() == (GRID_X..=1050.0) {
                    Some((false, r.center().y))
                } else {
                    None
                }
            })
            .collect();
        guides.sort_by(|a, b| a.partial_cmp(b).expect("no NaN"));
        guides
    }
}

fn close(a: f32, b: f32, eps: f32) -> bool {
    (a - b).abs() <= eps
}

// Drawing.

#[test]
fn the_screen_draws_its_lines_axis_overlay_curve_and_points_at_ppp_1_and_2() {
    let (bg, _, menu, ink) = colors();
    let pts = [p(0, 0), p(256, MAX), p(384, 32767)];
    for ppp in [1.0, 2.0] {
        let px = 1.0 / ppp;
        let rig = Rig::at(&pts, ppp, rect_at(ppp));
        // 3: five 1 px `background` lines across the grid at value top − 0.5 px + V·{0, ¼, ½, ¾, 1}.
        let mut lines: Vec<f32> = (rig.rects(bg).iter())
            .filter(|r| r.x_range() == (GRID_X..=1050.0) && close(r.height(), px, 1e-4))
            .map(|r| r.center().y)
            .collect();
        lines.sort_by(f32::total_cmp);
        assert_eq!(lines.len(), 5, "ppp {ppp}: {lines:?}");
        for (i, y) in lines.iter().enumerate() {
            let want = TOP - 0.5 * px + V * i as f32 / 4.0;
            assert!(
                close(*y, want, 0.5 * px),
                "ppp {ppp}: line {i} at {y}, want {want}"
            );
        }
        // 4: the bar mark at bar 2 (tick 512) is 4 px wide over the grid; a beat's 2 px, a quarter beat's 1 px.
        let marks = |x: f32| {
            (rig.rects(bg).iter())
                .filter(|r| r.y_range() == (48.0..=334.0) && close(r.center().x, x, px))
                .map(|r| (r.width() / px).round() as u32)
                .collect::<Vec<_>>()
        };
        assert_eq!(marks(GRID_X + 512.0), [4], "ppp {ppp}");
        assert_eq!(marks(GRID_X + 128.0), [2], "ppp {ppp}");
        assert_eq!(marks(GRID_X + 32.0), [1], "ppp {ppp}");
        // 5: past the loop end (beat 4), black at α 0.15 over the value area.
        let overlay = rig.rects(Color32::from_black_alpha(38));
        assert_eq!(
            overlay,
            [Rect::from_min_max(
                pos2(GRID_X + 512.0, TOP),
                pos2(1050.0, TOP + V)
            )],
            "ppp {ppp}"
        );
        // 6: the init value (Pitch Bend 0.5): a 1 px `menuFill` line across the grid at top + V·(1 − v) − 0.5 px.
        // In the grid: the ruler's `menuFill` baseline spans the same width above it.
        let init: Vec<Rect> = (rig.rects(menu).into_iter())
            .filter(|r| r.x_range() == (GRID_X..=1050.0) && r.top() >= TOP)
            .collect();
        assert_eq!(init.len(), 1, "ppp {ppp}");
        assert!(close(init[0].height(), px, 1e-4));
        assert!(close(
            init[0].center().y,
            TOP + V / 2.0 - 0.5 * px,
            0.5 * px
        ));
        // 7: the curve in 1.5 px lines, the channel colour, then a horizontal line from the last point to the right
        // edge; nothing left of the first point.
        let at = |q: Point| drawn(q);
        let want = vec![
            ([at(pts[0]), at(pts[1])], 1.5 * px, ink),
            ([at(pts[1]), at(pts[2])], 1.5 * px, ink),
            ([at(pts[2]), pos2(1050.0, at(pts[2]).y)], 1.5 * px, ink),
        ];
        assert_eq!(rig.segments(), want, "ppp {ppp}");
        // Points of radius 0.2 U in the channel colour, centred at top + (r + 0.5)·V/127.
        for q in pts {
            assert_eq!(rig.radius_at(at(q)), 6.4, "ppp {ppp} {q:?}");
        }
        assert_eq!(at(pts[1]).y, TOP + 1.0);
        assert_eq!(at(pts[2]).y, TOP + 127.0);
        // The value axis: `100 75 50 25 0` in the channel colour, each centred in (x + 3 px, y − 0.5 U, w − 6 px,
        // 0.5 U), over a 2 px tick from x + 3 px to x + w − 3 px.
        let labels = rig.texts(ink);
        let names: Vec<&str> = labels.iter().map(|l| l.0.as_str()).collect();
        assert_eq!(names, ["100", "75", "50", "25", "0"], "ppp {ppp}");
        for (i, (_, c)) in labels.iter().enumerate() {
            let y = TOP + V * i as f32 / 4.0;
            assert!(close(c.x, 24.0, 1.0), "ppp {ppp}: {c:?}");
            assert!(close(c.y, y - 8.0, 2.0), "ppp {ppp}: {c:?} for tick {y}");
        }
        let ticks: Vec<Rect> = (rig.rects(ink).into_iter())
            .filter(|r| r.x_range() == (3.0 * px..=48.0 - 3.0 * px))
            .collect();
        assert_eq!(ticks.len(), 5, "ppp {ppp}");
        for (i, r) in ticks.iter().enumerate() {
            assert!(close(r.height(), 2.0 * px, 1e-4));
            assert!(
                close(r.center().y, TOP + V * i as f32 / 4.0, 0.5 * px),
                "{r:?}"
            );
        }
    }
}

#[test]
fn a_value_area_under_3_u_has_three_axis_ticks() {
    let (_, _, _, ink) = colors();
    for ppp in [1.0, 2.0] {
        // G = 90 < 3.5 U: no pad, V = 90 < 3 U.
        let rect = Rect::from_min_max(RECT.min, pos2(1050.0, RULER_BOTTOM + 90.0));
        let rig = Rig::at(&[p(0, 0)], ppp, rect);
        let labels = rig.texts(ink);
        let names: Vec<&str> = labels.iter().map(|l| l.0.as_str()).collect();
        assert_eq!(names, ["100", "50", "0"], "ppp {ppp}");
        assert!(close(labels[1].1.y, 48.0 + 45.0 - 8.0, 2.0), "{labels:?}");
    }
}

#[test]
fn the_point_animations_follow_a_falling_0_6_a_frame() {
    // Pop-in: 1.0515·sin(1.885·(1 − a)), from nothing to about 1.
    assert_eq!(pop_scale(1.0), 0.0);
    assert!(close(pop_scale(0.0), 1.0, 1e-3));
    assert!(close(pop_scale(0.6), 1.0515 * (1.885f32 * 0.4).sin(), 1e-6));
    // Dragged: 2 − 0.6a with a floored at 0.01, so it grows from 1.4 to about 2.
    assert!(close(drag_scale(1.0), 1.4, 1e-6));
    assert!(close(drag_scale(0.6), 1.64, 1e-6));
    assert!(close(drag_scale(0.0), 1.994, 1e-6));
    // Released: 1 + (1 − a)·sin(2π·(1 − a)), back at 1 at both ends.
    assert_eq!(wobble_scale(1.0), 1.0);
    assert!(close(wobble_scale(0.0), 1.0, 1e-6));
    assert!(close(
        wobble_scale(0.6),
        1.0 + 0.4 * (0.8 * std::f32::consts::PI).sin(),
        1e-6
    ));
}

// Gestures.

#[test]
fn the_shared_long_press_time_and_calm_zone_decide_a_touch_on_empty_grid() {
    // `gesture::LONG_PRESS_S` and `CALM_U` (0.3 s; 0.5 U, 16 points at U = 32): released at 0.28 s, or moved 0.45 U, a
    // press on empty grid is a tap (a point); at 0.32 s it is a long press, 0.55 U out a drag: nothing.
    let points_after = |secs: f64, by: f32| {
        let mut rig = Rig::new(&[]);
        let p = pos2(700.0, 250.0);
        rig.press(p);
        let (mut to, mut held) = (p, 0.0);
        if by != 0.0 {
            to = p + vec2(by, 0.0);
            rig.move_to(to);
            held = 0.1;
        }
        let up = egui::Event::PointerButton {
            pos: to,
            button: PointerButton::Primary,
            pressed: false,
            modifiers: Modifiers::NONE,
        };
        rig.frame_after(secs - held, vec![up]);
        rig.points().len()
    };
    assert_eq!(points_after(0.28, 0.0), 1);
    assert_eq!(points_after(0.32, 0.0), 0);
    assert_eq!(points_after(0.2, 0.45 * 32.0), 1);
    assert_eq!(points_after(0.2, 0.55 * 32.0), 0);
}

#[test]
fn a_tap_on_empty_grid_creates_a_selected_point_and_with_a_selection_only_deselects() {
    let mut rig = Rig::new(&[]);
    assert_eq!(rig.ed.snap, Snap::default());
    // Tick 70 rounds to the 32-tick Auto step at 128 points a beat; row (y − top)·127/V = 10.
    rig.tap(pos2(GRID_X + 70.0, TOP + 20.0));
    let first = p(64, value_at_row(10.0, MAX));
    assert_eq!(
        (rig.points(), rig.selected(), rig.edits),
        (vec![first], vec![true], 1)
    );
    rig.tap(pos2(700.0, 250.0));
    assert_eq!(
        (rig.points(), rig.selected(), rig.edits),
        (vec![first], vec![false], 1)
    );
    rig.tap(pos2(700.0, 250.0));
    let second = p(640, value_at_row(93.0, MAX));
    assert_eq!(rig.points(), [first, second]);
    assert_eq!((rig.selected(), rig.edits), (vec![false, true], 2));
    // The loop fits the content: the smallest 4·2ᵏ beats past tick 640.
    assert_eq!(rig.clip().loop_len_beats, 8.0);
    // The top and bottom rows: the editor's maximum and 0.
    let mut rig = Rig::new(&[]);
    rig.tap(pos2(GRID_X + 64.0, TOP));
    rig.tap(pos2(700.0, 250.0));
    rig.tap(pos2(GRID_X + 128.0, TOP + V - 0.01));
    assert_eq!(rig.points(), [p(64, MAX), p(128, 0)]);
}

#[test]
fn a_long_press_on_snap_opens_the_snap_menu_and_a_pick_turns_snap_on() {
    let mut rig = Rig::new(&[]);
    // The playhead knob, which covers the Snap button at beat 0 and takes its touches, at beat 2.
    rig.opts.playhead = 256;
    rig.tap(pos2(24.0, 24.0));
    assert!(!rig.ed.snap.on);
    rig.frame(vec![egui::Event::PointerMoved(pos2(24.0, 24.0))]);
    for pressed in [true, false] {
        rig.frame(vec![egui::Event::PointerButton {
            pos: pos2(24.0, 24.0),
            button: PointerButton::Secondary,
            pressed,
            modifiers: Modifiers::NONE,
        }]);
    }
    rig.frame(vec![]);
    let beat = rig.text_at("Beat").expect("the snap menu");
    assert!(rig.text_at("1/6 Step").is_some() && rig.text_at("Bar").is_some());
    rig.tap(beat);
    assert_eq!(
        rig.ed.snap,
        Snap {
            on: true,
            mode: crate::ui::piano_roll::SnapMode::Beat
        }
    );
    // A beat's grid: tick 70 rounds to 128.
    rig.tap(pos2(GRID_X + 70.0, TOP + 20.0));
    assert_eq!(rig.points(), [p(128, value_at_row(10.0, MAX))]);
}

#[test]
fn snap_off_creates_on_the_finger_tick_and_a_long_press_without_a_clipboard_does_nothing() {
    let mut rig = Rig::new(&[]);
    // The playhead knob, which covers the Snap button at beat 0 and takes its touches, at beat 2.
    rig.opts.playhead = 256;
    // The Snap button's cell, top-left: a tap toggles snap.
    rig.tap(pos2(24.0, 24.0));
    assert!(!rig.ed.snap.on);
    rig.hold(pos2(GRID_X + 70.0, TOP + 20.0));
    assert_eq!(rig.text_at("Paste"), None);
    rig.release(pos2(GRID_X + 70.0, TOP + 20.0));
    assert_eq!((rig.points(), rig.edits), (vec![], 0));
    assert_eq!(rig.text_at("Paste"), None);
    rig.tap(pos2(GRID_X + 70.0, TOP + 20.0));
    assert_eq!(rig.points(), [p(70, value_at_row(10.0, MAX))]);
}

#[test]
fn a_new_point_pops_in() {
    let mut rig = Rig::new(&[]);
    let at = pos2(GRID_X + 64.0, TOP + 20.0);
    rig.press(at);
    rig.button(at, false);
    let q = drawn(p(64, value_at_row(10.0, MAX)));
    // Its first frame: a = 1, radius 0; then a ← 0.6a per 60 Hz frame.
    assert_eq!(rig.radius_at(q), 0.0);
    rig.frame_after(1.0 / 60.0, vec![]);
    assert!(close(rig.radius_at(q), 6.4 * pop_scale(0.6), 1e-3));
    rig.frame_after(1.0, vec![]);
    assert_eq!(rig.radius_at(q), 6.4);
}

#[test]
fn dragging_a_point_moves_it_in_one_edit_at_release_growing_then_wobbling() {
    let pts = [p(0, 0), p(256, 0)];
    let mut rig = Rig::new(&pts);
    let from = touch(pts[1]);
    rig.press(from);
    // It grows at once: 2 − 0.6a.
    assert!(close(rig.radius_at(drawn(pts[1])), 6.4 * 1.4, 1e-3));
    rig.frame_after(1.0 / 60.0, vec![]);
    assert!(close(rig.radius_at(drawn(pts[1])), 6.4 * 1.64, 1e-3));
    // Within 0.5 U nothing moves; past it the point follows the finger, the song only at the release.
    rig.move_to(from + vec2(8.0, 0.0));
    assert_eq!(rig.radius_at(drawn(pts[1]) + vec2(8.0, 0.0)), 0.0);
    rig.move_to(from + vec2(64.0, -40.0));
    let moved = p(320, value_at_row(106.0, MAX));
    assert!(rig.radius_at(drawn(moved)) > 6.4);
    assert_eq!((rig.points(), rig.edits), (pts.to_vec(), 0));
    rig.button(from + vec2(64.0, -40.0), false);
    assert_eq!(rig.points(), [pts[0], moved]);
    assert_eq!((rig.selected(), rig.edits), (vec![false, true], 1));
    // The release wobbles: 1 + (1 − a)·sin(2π(1 − a)).
    assert_eq!(rig.radius_at(drawn(moved)), 6.4);
    rig.frame_after(1.0 / 60.0, vec![]);
    assert!(close(
        rig.radius_at(drawn(moved)),
        6.4 * wobble_scale(0.6),
        1e-3
    ));
    // A drag that ends where it started edits nothing.
    rig.drag(touch(moved), &[vec2(40.0, 0.0), vec2(0.0, 0.0)]);
    assert_eq!(rig.edits, 1);
}

#[test]
fn a_tapped_point_shows_its_guides_and_copy_delete_and_another_tap_deselects() {
    let (_, text, _, ink) = colors();
    let pts = [p(0, 0), p(256, 32767), p(448, 0)];
    let mut rig = Rig::new(&pts);
    rig.tap(touch(pts[1]));
    assert_eq!((rig.selected(), rig.edits), (vec![false, true, false], 0));
    // Selected: the point and both its segments in `text`.
    assert!(
        rig.discs()
            .iter()
            .any(|d| d.0 == drawn(pts[1]) && d.2 == text)
    );
    let colors: Vec<Color32> = rig.segments().iter().map(|s| s.2).collect();
    assert_eq!(colors, [text, text, ink]);
    // 1 px `text` guides at the box's left and top, across the grid (a one-point box is no wider or taller).
    let guides: Vec<Rect> = (rig.rects(text).into_iter())
        .filter(|r| r.width() == 1.0 || r.height() == 1.0)
        .collect();
    assert_eq!(guides.len(), 2, "{guides:?}");
    assert!(
        guides
            .iter()
            .any(|r| r.y_range() == (48.0..=334.0) && close(r.center().x, GRID_X + 256.0, 0.5))
    );
    assert!(
        guides
            .iter()
            .any(|r| r.x_range() == (GRID_X..=1050.0) && close(r.center().y, TOP + 127.0, 0.5))
    );
    // `Copy` `Delete` `Edit value` `Set shape` above the touch frame (the box grown 1.25 U), centred on the finger.
    assert!(rig.text_at("Copy").is_some() && rig.text_at("Delete").is_some());
    assert!(rig.text_at("Edit\nvalue").is_some());
    let frame_top = TOP + 127.0 - 40.0;
    let row = rig.ed.row.rect().expect("the row");
    assert!(close(row.top(), frame_top - 6.4 - 48.0, 1e-3), "{row:?}");
    assert!(close(row.center().x, GRID_X + 256.0, 0.5), "{row:?}");
    assert!(close(row.width(), OPS.len() as f32 * 1.6 * 32.0, 1e-3));
    // A held point hides the row at once; the release is a tap that deselects.
    rig.press(touch(pts[1]));
    assert_eq!(rig.ed.row.rect(), None);
    rig.release(touch(pts[1]));
    assert_eq!((rig.selected(), rig.edits), (vec![false; 3], 0));
    assert_eq!((rig.text_at("Copy"), rig.ed.row.rect()), (None, None));
}

#[test]
fn a_long_press_on_a_point_shows_its_selection_without_toggling_and_a_tap_inside_the_frame_deselects()
 {
    // A long press on a point, with the mouse held 0.31 s.
    let pts = [p(0, 0), p(256, 32767), p(448, 0)];
    let mut rig = Rig::new(&pts);
    rig.hold(touch(pts[1]));
    assert_eq!(rig.ed.row.rect(), None, "hidden while held");
    rig.release(touch(pts[1]));
    assert_eq!(rig.selected(), [false, true, false]);
    assert!(rig.text_at("Copy").is_some() && rig.text_at("Delete").is_some());
    // Again on the selected point: it stays selected, the row back.
    rig.hold(touch(pts[1]));
    rig.release(touch(pts[1]));
    assert_eq!(rig.selected(), [false, true, false]);
    assert!(rig.ed.row.rect().is_some());
    // A quick tap on it deselects.
    rig.tap(touch(pts[1]));
    assert_eq!(rig.selected(), [false; 3]);
    assert_eq!(rig.edits, 0);
}

#[test]
fn copy_fills_the_clipboard_and_delete_removes_the_selection_in_one_edit() {
    let pts = [p(0, 0), p(256, 32767), p(448, 0)];
    let mut rig = Rig::new(&pts);
    rig.tap(touch(pts[1]));
    let copy = rig.text_at("Copy").expect("Copy");
    rig.tap(copy);
    assert_eq!(clipboard(&rig.ctx), [(2.0, 32767.0 / 65534.0, LINE)]);
    assert_eq!((rig.points(), rig.edits), (pts.to_vec(), 0));
    let delete = rig.text_at("Delete").expect("Delete stays");
    rig.tap(delete);
    assert_eq!(rig.points(), [pts[0], pts[2]]);
    assert_eq!((rig.selected(), rig.edits), (vec![false; 2], 1));
}

#[test]
fn a_long_press_shows_paste_where_the_finger_is_and_paste_lands_the_earliest_point_on_the_snapped_beat()
 {
    let mut rig = Rig::new(&[p(0, 0)]);
    set_clipboard(&rig.ctx, vec![(1.0, 0.5, LINE), (1.5, 1.0, LINE)]);
    let finger = pos2(300.0, 250.0);
    rig.hold(finger);
    assert!(
        rig.text_at("Paste").is_some(),
        "Paste appears before release"
    );
    let menu = egui::Id::new("automation_editor").with("paste");
    let paste = crate::ui::insert::button(&rig.ctx, menu, 0).expect("the Paste menu");
    assert!(paste.bottom() < finger.y, "over the finger");
    assert_eq!(rig.points(), [p(0, 0)]);
    assert_eq!(rig.edits, 0);
    rig.release(finger);
    assert_eq!(rig.points(), [p(0, 0)], "the original lift does not paste");
    assert_eq!(rig.edits, 0);
    rig.tap(paste.center());
    // Tick 250 snaps to 256; the second keeps its half beat; values × 65534.
    assert_eq!(rig.points(), [p(0, 0), p(256, 32767), p(320, MAX)]);
    assert_eq!((rig.selected(), rig.edits), (vec![false, true, true], 1));
    assert_eq!(rig.text_at("Paste"), None);
    // A press elsewhere hides it.
    rig.hold(pos2(800.0, 150.0));
    rig.release(pos2(800.0, 150.0));
    assert!(rig.text_at("Paste").is_some());
    rig.tap(pos2(900.0, 300.0));
    assert_eq!(rig.text_at("Paste"), None);
    // An emptied clipboard and a second finger both dismiss a held bubble.
    let edits = rig.edits;
    rig.hold(finger);
    set_clipboard(&rig.ctx, Vec::new());
    rig.frame(vec![]);
    assert_eq!(rig.text_at("Paste"), None);
    rig.release(finger);
    set_clipboard(&rig.ctx, vec![(0.0, 0.5, LINE)]);
    rig.hold(finger);
    let touches = [finger, finger + vec2(100.0, 50.0)]
        .into_iter()
        .enumerate()
        .map(|(i, pos)| egui::Event::Touch {
            device_id: egui::TouchDeviceId(1),
            id: egui::TouchId(i as u64),
            phase: egui::TouchPhase::Start,
            pos,
            force: None,
        });
    rig.frame(touches.collect());
    assert_eq!(rig.text_at("Paste"), None);
    assert_eq!(rig.edits, edits);
}

#[test]
fn a_long_press_that_moves_draws_a_marquee_and_the_scale_bar_scales() {
    let pts = [p(0, 0), p(128, 32767), p(256, 32767), p(448, MAX)];
    let mut rig = Rig::new(&pts);
    set_clipboard(&rig.ctx, vec![(0.0, 0.5, LINE)]);
    rig.hold(pos2(150.0, 150.0));
    assert!(rig.text_at("Paste").is_some());
    rig.move_to(pos2(250.0, 200.0));
    assert_eq!(rig.text_at("Paste"), None);
    rig.move_to(pos2(330.0, 230.0));
    // The selection is live: already before the lift.
    assert_eq!(rig.selected(), [false, true, true, false]);
    rig.release(pos2(330.0, 230.0));
    // Ticks 100…280 and rows 43…83 hold the two middle points (their one-row bands).
    assert_eq!(
        (rig.selected(), rig.edits),
        (vec![false, true, true, false], 0)
    );
    // The ↔ bar: under the bottom-right of the points' box as drawn (the centres' box, ticks 128…256 on row y
    // TOP + 127, grown by the 6.4-point radius), 34 points wide, its top 3.84 points below. The box's end goes to the
    // finger's tick (no grab offset), snapped: 373 → 384, so the box scales ×2 about its left edge.
    rig.drag(
        pos2(GRID_X + 245.0, TOP + 147.0),
        &[vec2(64.0, 0.0), vec2(128.0, 0.0)],
    );
    assert_eq!(rig.points(), [pts[0], pts[1], p(384, 32767), pts[3]]);
    assert_eq!(rig.edits, 1);
}

#[test]
fn the_rulers_region_menu_runs_the_ops_on_the_points_in_one_edit() {
    // A ramp; Delete over beat 1…2 holds the s value, then jumps at e; Delete space closes the gap into a jump.
    for (op, want) in [
        (
            "Delete",
            vec![
                p(0, 0),
                p(128, 16384),
                p(256, 16384),
                p(256, 32767),
                p(512, MAX),
            ],
        ),
        (
            "Delete space",
            vec![p(0, 0), p(128, 16384), p(128, 32767), p(384, MAX)],
        ),
    ] {
        let mut rig = Rig::new(&[p(0, 0), p(512, MAX)]);
        rig.hold(pos2(GRID_X + 128.0, 24.0));
        rig.move_to(pos2(GRID_X + 200.0, 24.0));
        rig.move_to(pos2(GRID_X + 256.0, 24.0));
        rig.release(pos2(GRID_X + 256.0, 24.0));
        assert_eq!(rig.song.play_loop, Some((128, 256)));
        let plus = rig.text_at("+").expect("the + button");
        rig.tap(plus);
        let item = rig.text_at(op).expect("the menu");
        assert!(rig.text_at("Copy to playhead").is_some());
        rig.tap(item);
        assert_eq!(rig.points(), want, "{op}");
        assert_eq!(rig.edits, 1, "{op}");
        assert_eq!(rig.song.play_loop, None);
    }
}

#[test]
fn a_ruler_tap_seeks_to_the_snapped_tick() {
    // Past the play box's touch area at tick 0: tick 200 on the 32-tick grid.
    let mut rig = Rig::new(&[p(0, 0)]);
    rig.tap(pos2(GRID_X + 200.0, 24.0));
    assert_eq!(rig.seeks, [192]);
}

#[test]
fn continuous_scrolling_keeps_the_playhead_a_quarter_in_while_playing_a_loop_longer_than_the_view()
{
    let (_, text, _, _) = colors();
    // The playhead's line (its play box fades out while playing, `timeline::knob_shown`): `text`, narrower than U.
    let handle = |rig: &Rig| {
        (rig.rects(text).into_iter())
            .find(|r| r.width() < 32.0 && r.top() < TOP && r.bottom() > TOP + V)
            .map(|r| crate::ui::timeline::testing::line_x(r, 1.0))
    };
    let mut rig = Rig::new(&[p(0, 0)]);
    let at = |rig: &mut Rig, playhead: u64, playing: bool, continuous: bool| {
        rig.opts = Options {
            playhead,
            playing,
            continuous,
            ..Options::default()
        };
        rig.frame(vec![]);
        handle(rig)
    };
    // A 4-beat loop is shorter than the 7.8 beats shown: no following.
    assert_eq!(at(&mut rig, 384, true, true), Some(GRID_X + 384.0));
    rig.song.channels[0].lanes[rig.key.lane].clips[0].loop_len_beats = 16.0;
    // Off, or stopped: the view stays, so the playhead at x 1330 is off the 1050-point grid and its line is not drawn.
    assert_eq!(at(&mut rig, 1280, true, false), None);
    assert_eq!(at(&mut rig, 1280, false, true), None);
    // On and playing: a quarter of the 1000-point grid in, never below beat 0.
    assert_eq!(at(&mut rig, 1280, true, true), Some(GRID_X + 250.0));
    assert_eq!(at(&mut rig, 128, true, true), Some(GRID_X + 128.0));
    // Meanwhile the ruler takes no range.
    rig.opts.playhead = 1280;
    rig.hold(pos2(GRID_X + 300.0, 24.0));
    rig.move_to(pos2(GRID_X + 400.0, 24.0));
    rig.release(pos2(GRID_X + 400.0, 24.0));
    assert_eq!(rig.song.play_loop, None);
}

#[test]
fn the_playhead_is_the_song_position_wrapped_modulo_the_loop() {
    let (_, text, _, _) = colors();
    let mut rig = Rig::new(&[p(0, 0)]);
    rig.opts.playhead = 512 + 100;
    rig.frame(vec![]);
    let x = timeline::testing::play_box_x(&rig.shapes, 32.0).expect("the play box");
    assert!((x - (GRID_X + 100.0)).abs() < 1e-3, "{x}");
    // Stopped, its line shows too: `text` down the grid, centred on the playhead.
    let line = (rig.rects(text).into_iter())
        .find(|r| {
            r.width() < 32.0
                && crate::ui::timeline::testing::line_x(*r, 1.0) == GRID_X + 100.0
                && r.top() >= RULER_BOTTOM
        })
        .expect("the playhead line");
    assert!(line.top() < TOP && line.bottom() > TOP + V, "{line:?}");
}

// Edge cases.

#[test]
fn the_init_line_sits_at_1_minus_v_and_a_tail_past_every_point_spans_the_grid() {
    let (_, text, menu, _) = colors();
    for ppp in [1.0, 2.0] {
        let px = 1.0 / ppp;
        let mut rig = Rig::at(&[p(0, 0), p(256, 32767)], ppp, rect_at(ppp));
        rig.song.channels[0].params.0[4] = 0.25;
        rig.frame(vec![]);
        // In the grid: the ruler's `menuFill` baseline spans the same width above it.
        let init: Vec<Rect> = (rig.rects(menu).into_iter())
            .filter(|r| r.x_range() == (GRID_X..=1050.0) && r.top() >= TOP)
            .collect();
        assert_eq!(init.len(), 1, "ppp {ppp}");
        let want = TOP + V * 0.75 - 0.5 * px;
        assert!(
            close(init[0].center().y, want, 0.5 * px),
            "ppp {ppp}: {init:?}"
        );
        // Scrolled 4 beats on, both points lie left of the view: one level line across the whole grid, `text` while
        // the last point is selected; the segment between them is not drawn.
        rig.set_loop(16.0);
        let key = (rig.key.channel, rig.key.lane, rig.key.id);
        rig.ed.views.insert(
            key,
            View {
                left: 4.0,
                beat_w: 128.0,
            },
        );
        rig.ed.sel = vec![false, true];
        rig.frame(vec![]);
        let y = drawn(p(256, 32767)).y;
        assert_eq!(
            rig.segments(),
            [([pos2(GRID_X, y), pos2(1050.0, y)], 1.5 * px, text)],
            "ppp {ppp}"
        );
    }
}

#[test]
fn a_two_point_marquee_draws_four_guides_and_the_row_stays_hidden_while_boxing() {
    let pts = [p(0, 0), p(128, 16384), p(256, 49150), p(448, MAX)];
    let mut rig = Rig::new(&pts);
    rig.tap(touch(pts[3]));
    assert!(rig.ed.row.rect().is_some());
    rig.hold(pos2(GRID_X + 100.0, TOP + 50.0));
    rig.move_to(pos2(GRID_X + 200.0, TOP + 120.0));
    assert!(matches!(rig.ed.gesture, Gesture::Box { .. }));
    assert_eq!(rig.ed.row.rect(), None);
    rig.move_to(pos2(GRID_X + 280.0, TOP + 200.0));
    rig.release(pos2(GRID_X + 280.0, TOP + 200.0));
    assert_eq!(rig.selected(), [false, true, true, false]);
    // Left and right at ticks 128 and 256, top and bottom at the two points' drawn rows.
    let guides = rig.guides();
    let want = [
        (false, drawn(pts[2]).y),
        (false, drawn(pts[1]).y),
        (true, GRID_X + 128.0),
        (true, GRID_X + 256.0),
    ];
    assert_eq!(guides.len(), 4, "{guides:?}");
    for (g, w) in guides.iter().zip(want) {
        assert!(g.0 == w.0 && close(g.1, w.1, 0.5), "{guides:?}");
    }
}

#[test]
fn an_outside_change_drops_the_selection_before_a_press_reads_it() {
    let pts = [p(0, 0), p(256, 0)];
    let mut rig = Rig::new(&pts);
    rig.tap(touch(pts[1]));
    assert_eq!(rig.selected(), [false, true]);
    // An undo brings back a third point.
    let lane = rig.key.lane;
    let clip = &mut rig.song.channels[0].lanes[lane].clips[0];
    let third = p(384, MAX);
    points::set_points(clip, PointKey::Automation, vec![pts[0], pts[1], third]);
    rig.frame(vec![]);
    assert_eq!(rig.selected(), [false; 3]);
    rig.press(touch(third));
    assert_eq!(rig.selected(), [false, false, true]);
    rig.release(touch(third));
    assert_eq!(rig.edits, 0);
}

#[test]
fn a_tap_allows_half_a_u_of_movement() {
    // On empty grid: 0.6 U is a pan, 0.4 U a tap that creates.
    let mut rig = Rig::new(&[]);
    let at = pos2(GRID_X + 64.0, TOP + 20.0);
    rig.drag(at, &[vec2(19.2, 0.0)]);
    assert_eq!((rig.points(), rig.edits), (vec![], 0));
    rig.drag(at, &[vec2(12.8, 0.0)]);
    assert_eq!(rig.points(), [p(64, value_at_row(10.0, MAX))]);
    // On a selected point: 0.4 U is a tap, which deselects; 0.6 U a drag, which moves it.
    let q = rig.points()[0];
    rig.drag(touch(q), &[vec2(0.0, 12.8)]);
    assert_eq!((rig.selected(), rig.edits), (vec![false], 1));
    rig.drag(touch(q), &[vec2(0.0, 19.2)]);
    let moved = p(64, value_at_row(points::row(q.value, MAX) + 10.0, MAX));
    assert_eq!((rig.points(), rig.edits), (vec![moved], 2));
}

#[test]
fn a_drag_moves_whole_rows_from_the_fingers_rows() {
    // V = 254: 2 points a row. 40.9 points is 20.45 rows, 41.1 is 20.55: they round to 20 and 21.
    for (dy, rows) in [(40.9, 20.0), (41.1, 21.0)] {
        let q = p(256, 32767);
        let mut rig = Rig::new(&[q]);
        rig.drag(touch(q), &[vec2(0.0, dy)]);
        let want = p(256, value_at_row(points::row(q.value, MAX) + rows, MAX));
        assert_eq!(rig.points(), [want], "{dy}");
    }
}

#[test]
fn one_finger_scrolls_the_time_on_the_ruler_band_only_and_two_need_the_first_there() {
    // The touch x zone is the ruler band, there is no touch y.
    let mut rig = Rig::new(&[p(0, 0), p(1024, MAX)]);
    rig.set_loop(16.0);
    rig.frame(vec![]);
    assert_eq!(rig.view().left, 0.0);
    // On empty grid: nothing scrolls.
    rig.drag(pos2(600.0, 200.0), &[vec2(-128.0, 0.0), vec2(-192.0, 0.0)]);
    assert_eq!(rig.view().left, 0.0);
    // On the ruler, 192 points left, the first move already past the calm zone: 1.5 beats on, 1:1, and no range.
    rig.drag(pos2(600.0, 24.0), &[vec2(-128.0, 0.0), vec2(-192.0, 0.0)]);
    assert_eq!(rig.view().left, 1.5);
    assert_eq!(rig.radius_at(drawn(p(1024 - 192, MAX))), 6.4);
    assert_eq!((rig.edits, rig.song.play_loop), (0, None));
    // Two fingers, the first on the grid: no zoom; the first on the ruler: the time zooms (300 → 200 points: out 1.5
    // times), about the first.
    use egui::TouchPhase::{End, Move, Start};
    let touch = |id, phase, pos| egui::Event::Touch {
        device_id: egui::TouchDeviceId(1),
        id: egui::TouchId(id),
        phase,
        pos,
        force: None,
    };
    let pinch = |rig: &mut Rig, a: Pos2| {
        let button = |pressed| egui::Event::PointerButton {
            pos: a,
            button: PointerButton::Primary,
            pressed,
            modifiers: Modifiers::NONE,
        };
        rig.frame(vec![egui::Event::PointerMoved(a)]);
        rig.frame(vec![touch(1, Start, a), button(true)]);
        rig.frame(vec![touch(2, Start, a + vec2(300.0, 150.0))]);
        rig.frame(vec![touch(2, Move, a + vec2(200.0, 100.0))]);
        rig.frame(vec![
            touch(2, End, a + vec2(200.0, 100.0)),
            touch(1, End, a),
            button(false),
            egui::Event::PointerGone,
        ]);
    };
    let before = rig.view();
    pinch(&mut rig, pos2(300.0, 200.0));
    assert_eq!(rig.view(), before);
    pinch(&mut rig, pos2(300.0, 24.0));
    assert!(
        (rig.view().beat_w - before.beat_w / 1.5).abs() < 1e-3,
        "{:?}",
        rig.view()
    );
    assert_eq!(rig.edits, 0);
}

#[test]
fn duplicate_keeps_the_play_loop() {
    let ramp = [p(0, 0), p(512, MAX)];
    let mut rig = Rig::new(&ramp);
    rig.hold(pos2(GRID_X + 128.0, 24.0));
    rig.move_to(pos2(GRID_X + 256.0, 24.0));
    rig.release(pos2(GRID_X + 256.0, 24.0));
    let plus = rig.text_at("+").expect("the + button");
    rig.tap(plus);
    let item = rig.text_at("Duplicate").expect("the menu");
    rig.tap(item);
    let mut want = ramp.to_vec();
    points::duplicate(&mut want, 128, 256);
    assert_eq!((rig.points(), rig.edits), (want, 1));
    assert_eq!(rig.song.play_loop, Some((128, 256)));
}

/// Continuous scrolling at the playhead `tick` over a 16-beat loop: the view follows.
fn following(rig: &mut Rig, tick: u64) {
    rig.opts = Options {
        playhead: tick,
        playing: true,
        continuous: true,
        ..Options::default()
    };
    rig.set_loop(16.0);
    rig.frame(vec![]);
}

#[test]
fn while_following_a_ruler_tap_still_moves_the_playhead_but_takes_no_range() {
    let mut rig = Rig::new(&[p(0, 0)]);
    following(&mut rig, 1280);
    // The view's left is 10 − 7.8125/4 beats: tick 1030 sits on the grid's left edge.
    assert_eq!(rig.view().left, 10.0 - 7.8125 / 4.0);
    // The play box fades out while playing (a fifth of the way a frame); at alpha 0 it takes no touch.
    for _ in 0..40 {
        rig.frame(vec![]);
    }
    rig.tap(pos2(GRID_X + 250.0 + 70.0, 24.0));
    // Tick 1280 + 70 on the 32-tick grid; the app stops the playing transport instead.
    assert_eq!(rig.seeks, [1344]);
    // A long press takes no range, and its release seeks nothing.
    rig.hold(pos2(GRID_X + 300.0, 24.0));
    rig.move_to(pos2(GRID_X + 400.0, 24.0));
    rig.release(pos2(GRID_X + 400.0, 24.0));
    rig.hold(pos2(GRID_X + 300.0, 24.0));
    rig.release(pos2(GRID_X + 300.0, 24.0));
    assert_eq!((rig.song.play_loop, rig.seeks.len()), (None, 1));
}

#[test]
fn while_following_a_dragged_point_keeps_to_the_beat_under_the_finger() {
    let q = p(1280, 0);
    let mut rig = Rig::new(&[p(0, 0), q]);
    following(&mut rig, 1280);
    // Tick 1280 sits a quarter in: x 300.
    let from = pos2(300.0, touch(q).y);
    rig.press(from);
    rig.move_to(from + vec2(64.0, 0.0));
    // The view moves a beat on under the still finger: it is over tick 1472.
    rig.opts.playhead = 1280 + 128;
    rig.frame(vec![]);
    rig.release(from + vec2(64.0, 0.0));
    assert_eq!(rig.points(), [p(0, 0), p(1472, 0)]);
}

#[test]
fn views_follow_their_tracks_and_go_with_their_clips() {
    let mut rig = Rig::new(&[p(0, 0)]);
    let (c, l, id) = (rig.key.channel, rig.key.lane, rig.key.id);
    rig.tap(touch(p(0, 0)));
    // A track inserted above: the view and the selection's clip move down a lane; a gone track's go.
    let saved = rig.view();
    rig.ed.rekey(&|ch, lane| Some((ch, lane + 1)));
    assert_eq!(rig.ed.views.get(&(c, l + 1, id)), Some(&saved));
    assert!(!rig.ed.views.contains_key(&(c, l, id)));
    assert_eq!(rig.ed.seen.as_ref().map(|s| s.0.lane), Some(l + 1));
    rig.ed.rekey(&|_, _| None);
    assert!(rig.ed.views.is_empty() && rig.ed.seen.is_none());
    // A clip deleted: its view goes when the editor next shows.
    let mut rig = Rig::new(&[p(0, 0)]);
    let lane = rig.key.lane;
    let second = rig.song.channels[0].lanes[lane].add_clip(2048, 4.0).id;
    let first = rig.key;
    rig.key = ClipKey {
        id: second,
        ..first
    };
    rig.frame(vec![]);
    assert_eq!(rig.ed.views.len(), 2);
    rig.song.channels[0].lanes[lane]
        .clips
        .retain(|c| c.id != first.id);
    rig.frame(vec![]);
    let keys: Vec<_> = rig.ed.views.keys().copied().collect();
    assert_eq!(keys, [(0, lane, second)]);
}

#[test]
fn a_long_press_or_a_shift_marquee_hides_the_row() {
    let pts = [p(0, 0), p(256, 32767), p(448, 0)];
    let mut rig = Rig::new(&pts);
    rig.tap(touch(pts[1]));
    assert!(rig.ed.row.rect().is_some());
    // From the long press on; its marquee selects what lies under the finger, nothing on empty grid; let go in place
    // without a clipboard, it stays hidden.
    rig.hold(pos2(800.0, 300.0));
    assert_eq!(rig.ed.row.rect(), None);
    rig.release(pos2(800.0, 300.0));
    assert_eq!((rig.ed.row.rect(), rig.selected()), (None, vec![false; 3]));
    // Shift starts a marquee at once: hidden while it is drawn.
    rig.tap(touch(pts[2]));
    assert!(rig.ed.row.rect().is_some());
    rig.modifiers = Modifiers::SHIFT;
    rig.press(pos2(800.0, 300.0));
    rig.move_to(pos2(850.0, 310.0));
    assert!(matches!(rig.ed.gesture, Gesture::Box { .. }));
    assert_eq!(rig.ed.row.rect(), None);
    rig.release(pos2(850.0, 310.0));
    assert_eq!(rig.selected(), [false; 3]);
}

#[test]
fn the_automation_wheel_zooms_the_time_on_the_ruler_and_on_the_grid() {
    // The zoom-x zone covers both and comes first.
    let pts = [p(0, 0), p(256, 32767)];
    let mut rig = Rig::new(&pts);
    rig.song.channels[0].lanes[rig.key.lane].clips[0].loop_len_beats = 64.0;
    rig.frame(vec![]);
    let key = (rig.key.channel, rig.key.lane, rig.key.id);
    for at in [pos2(400.0, 20.0), pos2(400.0, 200.0)] {
        let before = rig.ed.views[&key];
        let beat = |v: View| v.left + f64::from((at.x - GRID_X) / v.beat_w);
        rig.frame(vec![
            egui::Event::PointerMoved(at),
            egui::Event::MouseWheel {
                unit: egui::MouseWheelUnit::Point,
                delta: vec2(0.0, 200.0),
                modifiers: Modifiers::NONE,
            },
        ]);
        let after = rig.ed.views[&key];
        assert!(
            (after.beat_w - before.beat_w * 1.25).abs() < 1e-3,
            "{at:?}: {after:?}"
        );
        assert!((beat(after) - beat(before)).abs() < 1e-4, "{at:?}");
    }
}

// The keymap's editing and zoom actions (`ui/edit_keys.rs`).

impl Rig {
    /// A press of `key` with `mods` and its release, in one frame.
    fn key(&mut self, key: egui::Key, mods: Modifiers) {
        self.frame(crate::ui::edit_keys::tests::tap(key, mods).into());
    }
}

#[test]
fn the_editing_keys_copy_cut_delete_and_paste_points_at_the_touched_beat_one_edit_each() {
    let cmd = Modifiers::COMMAND;
    let pts = [p(0, 0), p(256, 32767), p(640, 0)];
    let mut rig = Rig::new(&pts);
    rig.tap(touch(pts[1]));
    rig.key(egui::Key::C, cmd);
    assert_eq!(clipboard(&rig.ctx), [(2.0, 32767.0 / 65534.0, LINE)]);
    rig.key(egui::Key::Escape, Modifiers::NONE);
    assert_eq!((rig.selected(), rig.edits), (vec![false; 3], 0));
    // Touching the last point 20 points right of it puts the cursor on tick 660: the paste lands on 672.
    rig.tap(touch(pts[2]) + vec2(20.0, 0.0));
    rig.key(egui::Key::V, cmd);
    assert_eq!(rig.points(), [pts[0], pts[1], pts[2], p(672, 32767)]);
    assert_eq!(
        (rig.selected(), rig.edits),
        (vec![false, false, false, true], 1)
    );
    // Cut takes the pasted point; Delete with nothing selected edits nothing; Delete on a touched point removes it.
    rig.key(egui::Key::X, cmd);
    assert_eq!((rig.points(), rig.edits), (pts.to_vec(), 2));
    rig.key(egui::Key::Delete, Modifiers::NONE);
    assert_eq!(rig.edits, 2);
    rig.tap(touch(pts[0]));
    rig.key(egui::Key::Delete, Modifiers::NONE);
    assert_eq!((rig.points(), rig.edits), (vec![pts[1], pts[2]], 3));
}

#[test]
fn enter_adds_a_selected_point_at_the_cursor_in_one_edit() {
    let mut rig = Rig::new(&[p(512, 0)]);
    // No point touched yet: tick 0, the middle row.
    rig.key(egui::Key::Enter, Modifiers::NONE);
    let mid = value_at_row(63.0, MAX);
    assert_eq!(rig.points(), [p(0, mid), p(512, 0)]);
    assert_eq!((rig.selected(), rig.edits), (vec![true, false], 1));
}

#[test]
fn ctrl_held_drags_a_marquee_from_a_point_instead_of_moving_it() {
    // The keymap's `editorSelect`.
    let pts = [p(0, 0), p(128, 32767), p(256, 32767), p(448, MAX)];
    let mut rig = Rig::new(&pts);
    rig.modifiers = Modifiers::COMMAND;
    rig.drag(touch(pts[1]), &[vec2(60.0, 20.0), vec2(140.0, 40.0)]);
    assert_eq!((rig.points(), rig.edits), (pts.to_vec(), 0));
    assert_eq!(rig.selected(), [false, true, true, false]);
}

#[test]
fn at_its_opening_4_u_a_beat_the_ruler_labels_every_bar() {
    // Every ruler follows the same law: at 4 U a beat s2 is one bar, so bar 2 (beat 4, 512 points right of the grid's
    // left at U = 32) has its label, just right of its line.
    let rig = Rig::new(&[p(0, 0)]);
    let (_, text, _, _) = colors();
    let labels = rig.texts(text.gamma_multiply(0.8));
    let two = labels.iter().find(|t| t.0 == "2").expect("bar 2's label");
    let line = GRID_X + 512.0;
    assert!(two.1.x > line && two.1.x < line + 16.0, "{labels:?}");
}

/// The value column covers only what lies left of its right edge (the playhead line's z-order): at beat 0 the line's
/// left half shows in the t gap, x 48 … 50.
#[test]
fn at_beat_0_the_lines_left_half_shows_in_the_gap_after_the_value_column() {
    let (_, text, _, _) = colors();
    let mut rig = Rig::new(&[p(0, 0)]);
    rig.opts.playhead = 0;
    rig.frame(vec![]);
    let shown: Vec<Rect> = (rig.shapes.iter())
        .filter_map(|s| match &s.shape {
            Shape::Rect(r)
                if r.fill == text && r.rect.width() < 32.0 && r.rect.top() >= RULER_BOTTOM =>
            {
                Some(r.rect.intersect(s.clip_rect))
            }
            _ => None,
        })
        .collect();
    assert!(
        shown
            .iter()
            .any(|r| r.left() < GRID_X && r.right() > GRID_X),
        "{shown:?}"
    );
}

// ---- a touch the editor stops seeing, and a press held over a focus loss ----

#[test]
fn a_drag_the_editor_stops_seeing_ends_with_no_edit() {
    let q = p(128, 40000);
    let mut rig = Rig::new(&[q]);
    rig.press(touch(q));
    rig.move_to(touch(q) + vec2(128.0, 0.0));
    // Hidden (another view took the editor's place), let go there, shown again: nothing lands at the pointer.
    rig.ed.end_touch();
    rig.release(touch(q) + vec2(256.0, 20.0));
    rig.frame(vec![]);
    assert_eq!(rig.points(), [q]);
    assert_eq!(rig.edits, 0);
}

#[test]
fn a_press_held_over_a_focus_loss_adds_no_point_moves_none_and_shows_no_paste() {
    let lost = |rig: &mut Rig| {
        rig.frame(vec![egui::Event::WindowFocused(false)]);
        rig.frame(vec![egui::Event::WindowFocused(true)]);
    };
    // On empty grid: no tap.
    let at = pos2(GRID_X + 300.0, TOP + 100.0);
    let mut rig = Rig::new(&[]);
    rig.press(at);
    lost(&mut rig);
    rig.release(at);
    assert_eq!(rig.points(), []);
    // A point's drag: the point stays where it was.
    let q = p(128, 40000);
    let mut rig = Rig::new(&[q]);
    rig.press(touch(q));
    rig.move_to(touch(q) + vec2(128.0, 0.0));
    lost(&mut rig);
    rig.release(touch(q) + vec2(128.0, 0.0));
    assert_eq!(rig.points(), [q]);
    assert_eq!(rig.edits, 0);
    // A long press with points copied: no `Paste`.
    let mut rig = Rig::new(&[]);
    set_clipboard(&rig.ctx, vec![(0.0, 0.5, LINE)]);
    rig.hold(at);
    assert!(rig.text_at("Paste").is_some());
    lost(&mut rig);
    assert_eq!(rig.text_at("Paste"), None);
    rig.release(at);
    assert_eq!(rig.text_at("Paste"), None);
    assert_eq!(rig.edits, 0);
}

/// The play box fades as the drum editor's (`timeline::knob_shown`, through `timeline::PlayheadKnob`): out while
/// playing, so that only the line shows, and back in once stopped.
#[test]
fn the_play_box_fades_out_while_playing_and_back_in_stopped() {
    let (_, _, menu, _) = colors();
    let mut rig = Rig::new(&[p(0, 0)]);
    let disc = |rig: &Rig| {
        timeline::testing::play_boxes(&rig.shapes)
            .next()
            .map(|b| b.fill)
    };
    rig.opts.playhead = 128;
    for (playing, fill) in [(false, Some(menu)), (true, None), (false, Some(menu))] {
        rig.opts.playing = playing;
        for _ in 0..40 {
            rig.frame(vec![]);
        }
        assert_eq!(disc(&rig), fill, "playing {playing}");
    }
}

// Values in the target's units, typed values and the value grid (a Honeydew addition, GitHub issue #21).

/// The channel Level: a dB param.
const LEVEL: AutomationTarget = AutomationTarget {
    instance_id: 0,
    param: 0,
};

impl Rig {
    /// A rig whose lane is on `target`.
    fn on(target: AutomationTarget, pts: &[Point]) -> Self {
        let mut rig = Self::new(pts);
        (rig.song, rig.key) = song_on(target, pts);
        rig.frame(vec![]);
        rig
    }

    /// Opens the snap menu with a right click on the Snap button (the playhead knob, which covers it at beat 0, is
    /// moved to beat 4 first).
    fn open_snap_menu(&mut self) {
        self.opts.playhead = 512;
        let at = pos2(24.0, 24.0);
        self.frame(vec![egui::Event::PointerMoved(at)]);
        for pressed in [true, false] {
            self.frame(vec![egui::Event::PointerButton {
                pos: at,
                button: PointerButton::Secondary,
                pressed,
                modifiers: Modifiers::NONE,
            }]);
        }
        self.frame(vec![]);
    }
}

#[test]
fn the_axis_labels_in_the_targets_units_a_pan_short_and_a_switch_at_its_steps() {
    let (_, _, _, ink) = colors();
    let labels = |target| {
        let rig = Rig::on(target, &[p(0, 0)]);
        let names: Vec<String> = rig.texts(ink).into_iter().map(|l| l.0).collect();
        let ticks: Vec<f32> = (rig.rects(ink).into_iter())
            .filter(|r| r.x_range() == (3.0..=45.0))
            .map(|r| r.center().y)
            .collect();
        (names, ticks)
    };
    // The channel Pan: `100% right` … `center` … `100% left` in short.
    let pan = AutomationTarget {
        instance_id: 0,
        param: 1,
    };
    assert_eq!(labels(pan).0, ["100R", "50R", "C", "50L", "100L"]);
    // A switch (Mute): a tick at each of its two steps, each labelled once.
    let mute = AutomationTarget {
        instance_id: 0,
        param: 2,
    };
    let (names, ticks) = labels(mute);
    assert_eq!(names, ["On", "Off"]);
    assert_eq!(ticks.len(), 2, "{ticks:?}");
    assert!(
        close(ticks[0], TOP, 0.5) && close(ticks[1], TOP + V, 0.5),
        "{ticks:?}"
    );
    // The channel Level in dB.
    let rig = Rig::on(LEVEL, &[p(0, 0)]);
    // The ticks at 1, ¾, ½, ¼ and 0 of the range: +6.0, −4.5, −16.5, −31.3 dB and −inf. (In the test font the middle
    // two do not fit 6 pt and truncate, keeping their tail.)
    let names: Vec<String> = rig.texts(ink).into_iter().map(|l| l.0).collect();
    assert_eq!(names.len(), 5, "{names:?}");
    assert_eq!(
        (names[0].as_str(), names[1].as_str()),
        ("+6.0 dB", "-4.5 dB")
    );
    assert!(
        names[2].starts_with("-16") && names[3].starts_with("-31"),
        "{names:?}"
    );
    assert!(names[..4].iter().all(|n| n.ends_with("dB")), "{names:?}");
    assert_eq!(names[4], "-inf.");
}

#[test]
fn edit_value_sets_the_selected_points_to_a_typed_value_in_one_edit() {
    let pts = [p(0, 0), p(128, 20000), p(256, 50000)];
    let mut rig = Rig::on(LEVEL, &pts);
    // A Shift marquee over the last two.
    rig.modifiers = Modifiers::SHIFT;
    let from = pos2(GRID_X + 100.0, TOP + 10.0);
    rig.drag(from, &[vec2(100.0, 100.0), vec2(200.0, 240.0)]);
    rig.modifiers = Modifiers::NONE;
    assert_eq!(rig.selected(), [false, true, true]);
    rig.tap(rig.text_at("Edit\nvalue").expect("Edit value"));
    rig.frame(vec![egui::Event::Text("-6 dB".into())]);
    rig.key(egui::Key::Enter, Modifiers::NONE);
    let got = rig.points();
    let level = crate::params::CHANNEL_PARAMS[0].display_text(points::unit(got[1].value));
    assert_eq!((got[1].value, level.as_str()), (got[2].value, "-6.0 dB"));
    assert_eq!((got[0], rig.edits), (pts[0], 1));
}

#[test]
fn with_value_snap_at_10_percent_a_dragged_point_lands_on_a_line_and_off_it_moves_freely() {
    let q = p(256, 32767);
    let mut rig = Rig::new(&[q]);
    rig.open_snap_menu();
    rig.tap(rig.text_at("10 %").expect("the value column"));
    // 30.5 rows up: 32767 + 30.5·65534/126 = 48630.6, nearest the 70 % line.
    rig.drag(touch(q), &[vec2(0.0, -61.0)]);
    let line = |k: u32| (f64::from(k) * 6553.4).round() as u16;
    assert_eq!(rig.points(), [p(256, line(7))]);
    // Off: 11.5 rows down is 5981.2 lower, on no line.
    rig.open_snap_menu();
    rig.tap(rig.text_at("Off").expect("Off"));
    rig.drag(touch(rig.points()[0]), &[vec2(0.0, 23.0)]);
    assert_eq!(rig.points(), [p(256, 39893)]);
    assert!((0..=10).all(|k| line(k) != 39893));
}
