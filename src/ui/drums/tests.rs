use eframe::egui::epaint::ClippedShape;
use eframe::egui::{self, Context, Event, PointerButton, RawInput, Shape, Vec2, pos2, vec2};

use super::*;
use crate::metrics::{DEFAULT_UI_SCALE, Metrics};
use crate::midi::CcLink;
use crate::theme::theme_by_id;
use crate::ui::ctrl_menu::CtrlAction;
use crate::ui::offscreen::key_tap;

fn near(a: f32, b: f32) -> bool {
    (a - b).abs() < 1e-3
}

fn area(x: f32, y: f32, w: f32, h: f32) -> Rect {
    Rect::from_min_size(pos2(x, y), vec2(w, h))
}

// Pad grid geometry.

#[test]
fn pad_size_and_centres_follow_the_formulas() {
    // (area, screen width, U) → (r, top, left edge of the 16 r wide grid), all px.
    let cases = [
        // Device-like: the height gives 107, the width caps it at 106, the screen's 20 U floor is 106.
        (
            area(0.0, 0.0, 1700.0, 600.0),
            3120.0,
            85.0,
            106.0,
            27.0,
            2.0,
        ),
        // Height-limited: floor(400 / 5.58) = 71; top = floor(20 + (400 − floor(6·71·0.86)) / 2).
        (
            area(10.0, 20.0, 2000.0, 400.0),
            2000.0,
            32.0,
            71.0,
            37.0,
            442.0,
        ),
        // Width-limited: floor(800 / 16) = 50.
        (area(0.0, 0.0, 800.0, 1000.0), 800.0, 32.0, 50.0, 371.0, 0.0),
    ];
    for (area, screen, u, r, top, left) in cases {
        let g = pad_grid(area, screen, u);
        assert_eq!(g.r, r, "{area:?}");
        for (i, c) in g.centres.iter().enumerate() {
            let (row, col) = match i {
                0..7 => (0, i),
                7..15 => (1, i - 7),
                _ => (2, i - 15),
            };
            let shift = if row == 1 { 0.0 } else { r };
            let x = left + r + 2.0 * r * col as f32 + shift;
            let y = top + r + 1.72 * r * row as f32;
            assert!(near(c.x, x) && near(c.y, y), "{area:?} pad {i}: {c:?}");
        }
    }
}

#[test]
fn the_lower_size_bound_wins_and_the_grid_top_stays_in_the_area() {
    // 100 px high gives 17 and 300 px wide caps at 18, but the floor is floor(min(1280, 20 U) / 16) = 40. The
    // centred top would be above the area, so it is the area's top.
    let g = pad_grid(area(0.0, 50.0, 300.0, 100.0), 1280.0, 32.0);
    assert_eq!(g.r, 40.0);
    // Pad 0 sits r below the area's top; pad 7 (the middle row) starts the 16 r grid centred on the 300 px area.
    assert_eq!(g.centres[0], pos2((300.0 - 640.0) / 2.0 + 80.0, 90.0));
    assert_eq!(g.centres[7].x, (300.0 - 640.0) / 2.0 + 40.0);
    // A narrow screen lowers the floor.
    assert_eq!(pad_grid(area(0.0, 0.0, 300.0, 100.0), 320.0, 32.0).r, 20.0);
}

#[test]
fn a_point_hits_the_pad_within_r_of_its_centre() {
    // A touch hits a pad within radius R of its centre.
    let g = pad_grid(area(0.0, 0.0, 1700.0, 600.0), 3120.0, 85.0);
    let r = g.r;
    for (i, &c) in g.centres.iter().enumerate() {
        assert_eq!(g.pad_at(c), Some(i));
    }
    let c7 = g.centres[7];
    // Pad 8 is 2 r to the right: the circles meet halfway.
    assert_eq!(g.pad_at(c7 + vec2(r - 1.0, 0.0)), Some(7));
    assert_eq!(g.pad_at(c7 + vec2(r + 1.0, 0.0)), Some(8));
    // Left of the first pad of the middle row, and 1.1 r above a top-row pad (inside the honeycomb cell, whose apex is
    // 2 r / √3 ≈ 1.155 r up, but outside the circle): nothing.
    assert_eq!(g.pad_at(c7 - vec2(r + 1.0, 0.0)), None);
    assert_eq!(g.pad_at(g.centres[3] - vec2(0.0, 1.1 * r)), None);
    assert_eq!(g.pad_at(g.centres[3] - vec2(0.0, r - 1.0)), Some(3));
    // Between rows the circles overlap a little (1.99 r apart), and the nearer centre wins: 0.8 r below pad 0 is still
    // pad 0; half an r left of that and lower it is pad 7's.
    assert_eq!(g.pad_at(g.centres[0] + vec2(0.0, 0.8 * r)), Some(0));
    assert_eq!(g.pad_at(g.centres[0] + vec2(-0.5 * r, 0.95 * r)), Some(7));
}

/// A pad's visible circumradius for the pad size `size` (px): trunc(1.5 size) seen through the circle mask, 1/√2 of it.
fn visible(size: f32) -> f32 {
    (1.5 * size).trunc() * std::f32::consts::FRAC_1_SQRT_2
}

#[test]
fn the_worked_example_r_106_draws_r_159_seen_as_a_hex_of_circumradius_112_17_px_apart() {
    // U = 85, r = 106, R = 159, at ppp 1 and 2.
    for ppp in [1.0, 2.0] {
        let c = pos2(500.0, 300.0);
        let Shape::Path(path) = hex(c, 106.0, ppp, Color32::RED) else {
            panic!("a polygon");
        };
        assert!(path.closed);
        assert_eq!((path.fill, path.stroke.width), (Color32::RED, 0.0));
        // The outline's vertices, (0.86 R, −0.5 R) … (−0.006 R, −R), scaled by 1/√2 about the centre.
        let s = 159.0 * std::f32::consts::FRAC_1_SQRT_2 / ppp;
        let want = [
            (0.86, -0.5),
            (0.86, 0.5),
            (-0.006, 1.0),
            (-0.872, 0.5),
            (-0.872, -0.5),
            (-0.006, -1.0),
        ]
        .map(|(x, y)| c + s * vec2(x, y));
        assert_eq!(path.points.len(), 6);
        for (p, w) in path.points.iter().zip(want) {
            assert!((*p - w).length() < 1e-3, "{p:?} vs {w:?}");
        }
        // Visible circumradius ≈ 112 px, flat-to-flat ≈ 195 px, and so ≈ 17 px apart at the 2 r pitch.
        let px = |v: f32| v * ppp;
        let radius = px(c.y - path.points[5].y);
        let flat = px(path.points[0].x - path.points[3].x);
        assert!((radius - 112.4).abs() < 0.1, "{radius}");
        assert!((flat - 194.7).abs() < 0.1, "{flat}");
        assert!((2.0 * 106.0 - flat - 17.3).abs() < 0.1);
    }
}

// Strip geometry.

#[test]
fn strips_stack_at_the_area_height_over_the_rows_less_a_pixel() {
    let a = area(10.0, 20.0, 600.0, 200.0);
    assert_eq!(strip_rect(a, 4, 0, 1.0), area(10.0, 20.0, 600.0, 49.0));
    assert_eq!(strip_rect(a, 4, 3, 1.0), area(10.0, 170.0, 600.0, 49.0));
    assert_eq!(strip_rect(a, 4, 1, 2.0), area(10.0, 70.0, 600.0, 49.5));
}

#[test]
fn each_page_splits_the_layer_by_the_spec_weights() {
    // A 600 × 60 px strip at U = 32: a 96 px header, then the 504 px layer. Pages are 45 px rows, centred.
    let strip = area(0.0, 0.0, 600.0, 60.0);
    let widths = |page| {
        let (header, controls) = place_strip(page, strip, 32.0);
        assert_eq!(header, area(0.0, 0.0, 96.0, 60.0));
        for (_, r) in &controls {
            assert_eq!((r.top(), r.height()), (7.0, 45.0), "{page:?}");
        }
        controls
            .iter()
            .map(|(ctl, r)| (*ctl, r.left(), r.width()))
            .collect::<Vec<_>>()
    };
    use Ctl::*;
    // Weights 1 · 1 · 2 · 4 over 504 px.
    assert_eq!(
        widths(StripPage::Mix),
        [
            (Mute, 96.0, 63.0),
            (Solo, 159.0, 63.0),
            (Pan, 222.0, 126.0),
            (Level, 348.0, 252.0)
        ]
    );
    // 1 · 1 · 2 · 2 · 2, the knobs with 8 px (0.25 U) right margins: 480 px / 8.
    assert_eq!(
        widths(StripPage::Pitch),
        [
            (Sample, 96.0, 60.0),
            (Rev, 156.0, 60.0),
            (Pitch, 216.0, 120.0),
            (Stretch, 344.0, 120.0),
            (Start, 472.0, 120.0)
        ]
    );
    // Four flex 1 knobs with 8 px right margins: 472 / 4.
    assert_eq!(
        widths(StripPage::Envelope),
        [
            (Atk, 96.0, 118.0),
            (Dec, 222.0, 118.0),
            (Sus, 348.0, 118.0),
            (Rel, 474.0, 118.0)
        ]
    );
    // 2 · 1 · 1 · 2 · 1.2: 70 px per unit of weight.
    assert_eq!(
        widths(StripPage::Options),
        [
            (Hold, 96.0, 140.0),
            (Cut, 236.0, 70.0),
            (Poly, 306.0, 70.0),
            (Shift, 376.0, 140.0),
            (Aux, 516.0, 84.0)
        ]
    );
}

// Offscreen egui: the views in a real context, driven by synthetic pointer events (ppp 1, U = 32).

/// Points of vertical drag that move a knob a quarter of its range (0.24 per 192-point inch).
const KNOB_QUARTER: f32 = 200.0;
const SCREEN: Rect = Rect {
    min: pos2(0.0, 0.0),
    max: pos2(1000.0, 700.0),
};
const PADS: Rect = Rect {
    min: pos2(0.0, 0.0),
    max: pos2(1000.0, 400.0),
};
const STRIPS: Rect = Rect {
    min: pos2(0.0, 100.0),
    max: pos2(800.0, 292.0),
};
const U: f32 = 32.0;

#[derive(Clone, Copy, PartialEq)]
enum View {
    Pads,
    Strips,
}

struct Rig {
    ctx: Context,
    view: View,
    kit: DrumKit,
    state: DrumsState,
    /// The pads are locked (`lockpads`).
    locked: bool,
    actions: Vec<DrumsAction>,
    shapes: Vec<ClippedShape>,
    time: f64,
    /// The channel's CC links the strips are drawn with.
    links: Vec<CcLink>,
}

fn slot(name: &str, folder: &str) -> SamplerSlot {
    SamplerSlot {
        name: name.to_owned(),
        folder: folder.to_owned(),
        ..SamplerSlot::default()
    }
}

impl Rig {
    fn new(view: View) -> Self {
        let kit = DrumKit {
            slots: vec![
                slot("Kick 01", "Drum Samples\\Kick"),
                slot("Snare 01", "Drum Samples\\Snare"),
                slot("Hat 01", "Drum Samples\\Hat"),
            ],
            ..DrumKit::default()
        };
        let mut rig = Self {
            ctx: Context::default(),
            view,
            kit,
            state: DrumsState::default(),
            locked: false,
            actions: Vec::new(),
            shapes: Vec::new(),
            time: 0.0,
            links: Vec::new(),
        };
        rig.frame(vec![]);
        rig
    }

    fn frame(&mut self, events: Vec<Event>) {
        self.time += 1.0 / 60.0;
        let raw = RawInput {
            screen_rect: Some(SCREEN),
            time: Some(self.time),
            events,
            ..Default::default()
        };
        let (theme, m) = (
            theme_by_id(0),
            Metrics::from_pixels_per_point(1.0, DEFAULT_UI_SCALE),
        );
        let cx = UiCtx {
            theme: &theme,
            m: &m,
        };
        let (view, kit, state) = (self.view, &mut self.kit, &mut self.state);
        let (locked, links) = (self.locked, &self.links);
        let mut actions = Vec::new();
        let output = self.ctx.run(raw, |ctx| {
            egui::CentralPanel::default().show(ctx, |ui| {
                actions = match view {
                    View::Pads => show_pads(ui, &cx, PADS, kit, state, Color32::RED, locked),
                    View::Strips => show_strips(ui, &cx, STRIPS, kit, state, Color32::RED, links),
                };
            });
        });
        self.actions = actions;
        self.shapes = output.shapes;
    }

    fn button(&mut self, pos: Pos2, button: PointerButton, pressed: bool) {
        self.frame(vec![Event::PointerButton {
            pos,
            button,
            pressed,
            modifiers: Default::default(),
        }]);
    }

    fn press(&mut self, pos: Pos2) {
        self.frame(vec![Event::PointerMoved(pos)]);
        self.button(pos, PointerButton::Primary, true);
    }

    fn release(&mut self, pos: Pos2) {
        self.button(pos, PointerButton::Primary, false);
    }

    fn click(&mut self, pos: Pos2) {
        self.press(pos);
        self.release(pos);
    }

    /// Presses at `from`, moves by `by` in three steps, releases; returns every action reported on the way.
    fn drag(&mut self, from: Pos2, by: Vec2) -> Vec<DrumsAction> {
        self.press(from);
        let mut all = self.actions.clone();
        for k in 1..=3 {
            self.frame(vec![Event::PointerMoved(from + by * k as f32 / 3.0)]);
            all.extend(self.actions.clone());
        }
        self.release(from + by);
        all.extend(self.actions.clone());
        all
    }

    /// The centre of control `ctl` of strip `row` on the current page.
    fn at(&self, row: usize, ctl: Ctl) -> Pos2 {
        self.rect_at(row, ctl).center()
    }

    /// The rect of control `ctl` of strip `row` on the current page.
    fn rect_at(&self, row: usize, ctl: Ctl) -> Rect {
        let rows = self.kit.slots.len();
        let (_, controls) = place_strip(self.state.page, strip_rect(STRIPS, rows, row, 1.0), U);
        controls
            .into_iter()
            .find(|(c, _)| *c == ctl)
            .map(|(_, r)| r)
            .expect("on this page")
    }

    /// A horizontal slider's travel from 0 to 1: its track, 1 U shorter than the control.
    fn track(&self, row: usize, ctl: Ctl) -> f32 {
        self.rect_at(row, ctl).width() - U
    }

    fn header(&self, row: usize) -> Pos2 {
        let rows = self.kit.slots.len();
        place_strip(self.state.page, strip_rect(STRIPS, rows, row, 1.0), U)
            .0
            .center()
    }

    fn texts(&self) -> Vec<(String, Color32)> {
        self.shapes
            .iter()
            .filter_map(|c| match &c.shape {
                Shape::Text(t) => Some((t.galley.text().to_owned(), t.fallback_color)),
                _ => None,
            })
            .collect()
    }

    /// The centre of the first text shape reading `text`.
    fn text_at(&self, text: &str) -> Pos2 {
        self.shapes
            .iter()
            .find_map(|c| match &c.shape {
                Shape::Text(t) if t.galley.text() == text => {
                    Some(t.pos + t.galley.rect.center().to_vec2())
                }
                _ => None,
            })
            .expect("drawn")
    }

    fn count(&self, text: &str) -> usize {
        self.texts().iter().filter(|(t, _)| t == text).count()
    }

    /// Each hexagon but the level rings ([`Self::rings`]): fill and visible circumradius (its top vertex's height over
    /// its centre).
    fn hexes(&self) -> Vec<(Color32, f32)> {
        self.hexes_at()
            .into_iter()
            .map(|(c, _, r)| (c, r))
            .collect()
    }

    /// Each pad's level ring, in pad order: fill and centre.
    fn rings(&self) -> Vec<(Color32, Pos2)> {
        let rings = self.all_hexes().into_iter().filter(|h| glow(h.2));
        rings.map(|(c, centre, _)| (c, centre)).collect()
    }

    /// Each hexagon but the level rings: fill, centre and circumradius, in drawing order.
    fn hexes_at(&self) -> Vec<(Color32, Pos2, f32)> {
        self.all_hexes()
            .into_iter()
            .filter(|h| !glow(h.2))
            .collect()
    }

    fn all_hexes(&self) -> Vec<(Color32, Pos2, f32)> {
        self.shapes
            .iter()
            .filter_map(|c| match &c.shape {
                Shape::Path(p) if p.points.len() == 6 => {
                    let centre = p.points.iter().fold(Vec2::ZERO, |a, q| a + q.to_vec2()) / 6.0;
                    let top = p.points.iter().map(|q| q.y).fold(f32::INFINITY, f32::min);
                    Some((p.fill, centre.to_pos2(), centre.y - top))
                }
                _ => None,
            })
            .collect()
    }

    /// The pads held (not picked up).
    fn held(&self) -> Vec<usize> {
        let held = self.state.fingers.iter().filter(|f| f.lifted.is_none());
        held.map(|f| f.pad).collect()
    }
}

fn grid() -> PadGrid {
    pad_grid(PADS, SCREEN.width(), U)
}

/// A level ring's visible circumradius: size r + 2·ceil(U/16), U = 32. A held pad is r + 3, a pickup ring r + 2.
fn glow(radius: f32) -> bool {
    (radius - visible(grid().r + 4.0)).abs() < 0.5
}

#[test]
fn used_pads_take_the_channel_colour_and_show_their_names() {
    let rig = Rig::new(View::Pads);
    let g = grid();
    // U = 32: floor(400 / 5.58) = 71, capped at floor(1000 / 16) = 62.
    assert_eq!(g.r, 62.0);
    let hexes = rig.hexes();
    assert_eq!(hexes.len(), 22);
    let module_fill = theme_by_id(0).colors.module_fill.to_color32();
    for (i, &(color, radius)) in hexes.iter().enumerate() {
        assert_eq!(color, if i < 3 { Color32::RED } else { module_fill }, "{i}");
        // Drawn at R = trunc(1.5 r) = 93, seen at 93/√2 = 65.8.
        assert!(near(radius, visible(62.0)), "{i}: {radius}");
    }
    for text in ["Kick", "Kick 01", "Snare", "Snare 01", "Hat", "Hat 01"] {
        let found = rig.texts().into_iter().find(|(t, _)| t == text);
        assert_eq!(found.map(|(_, c)| c), Some(module_fill), "{text}");
    }
}

#[test]
fn long_names_shrink_to_a_box_1_5_r_wide() {
    let mut rig = Rig::new(View::Pads);
    rig.kit.slots[0] = slot("Hat Open Long Tail Take 2", "Drum Samples\\Open Hat Long");
    rig.frame(vec![]);
    // 1.5 r = 93 points.
    let room = 1.5 * 62.0;
    let widths: Vec<f32> = (rig.shapes.iter())
        .filter_map(|c| match &c.shape {
            Shape::Text(t) => Some(t.galley.rect.width()),
            _ => None,
        })
        .collect();
    assert_eq!(widths.len(), 6);
    assert!(widths.iter().all(|&w| w <= room), "{widths:?} > {room}");
}

#[test]
fn a_new_kit_has_only_unused_pads() {
    let mut rig = Rig::new(View::Pads);
    rig.kit = DrumKit::default();
    rig.frame(vec![]);
    let module_fill = theme_by_id(0).colors.module_fill.to_color32();
    assert!(rig.hexes().iter().all(|&(c, _)| c == module_fill));
    assert!(rig.texts().is_empty());
}

#[test]
fn pressing_a_pad_reports_it_and_grows_it_by_3_px() {
    let mut rig = Rig::new(View::Pads);
    rig.locked = true;
    let g = grid();
    rig.press(g.centres[1]);
    assert_eq!(rig.actions, [DrumsAction::PadDown(1)]);
    assert_eq!(rig.held(), [1]);
    rig.frame(vec![]);
    // Size r + 3 = 65 for the touched pad (R = 97, seen at 68.6), r = 62 for the others (seen at 65.8).
    let radii: Vec<f32> = rig.hexes().iter().map(|h| h.1).collect();
    assert!(
        near(radii[1], visible(65.0)) && near(radii[0], visible(62.0)),
        "{radii:?}"
    );
    // No glissando between pads: sliding onto pad 9 plays nothing, and the release ends pad 1.
    rig.frame(vec![Event::PointerMoved(g.centres[9])]);
    assert!(rig.actions.is_empty());
    assert_eq!(rig.held(), [1]);
    rig.release(g.centres[9]);
    assert_eq!(rig.actions, [DrumsAction::PadUp(1)]);
    assert!(rig.held().is_empty());
    // A press between the pads that slides onto one plays nothing either.
    rig.press(pos2(2.0, 2.0));
    rig.frame(vec![Event::PointerMoved(g.centres[0])]);
    rig.release(g.centres[0]);
    assert!(rig.actions.is_empty() && rig.held().is_empty());
    // Outside every pad nothing happens.
    rig.click(pos2(2.0, 2.0));
    assert!(rig.actions.is_empty());
}

#[test]
fn a_press_older_than_the_pads_state_plays_nothing() {
    // A song replace resets the pads while a press is held: the rest of that press, and its release, play nothing.
    let mut rig = Rig::new(View::Pads);
    let at = grid().centres[1];
    rig.press(at);
    assert_eq!(rig.actions, [DrumsAction::PadDown(1)]);
    rig.state = DrumsState::default();
    rig.frame(vec![]);
    assert!(rig.actions.is_empty());
    rig.release(at);
    assert!(rig.actions.is_empty());
}

#[test]
fn switching_the_page_switches_every_strip() {
    let mut rig = Rig::new(View::Strips);
    let pages = [
        (StripPage::Mix, &["Solo"][..]),
        (StripPage::Pitch, &["REV", "Pitch", "Stretch", "Start"][..]),
        (StripPage::Envelope, &["ATK", "DEC", "SUS", "REL"][..]),
        (
            StripPage::Options,
            &["Hold", "Cut...", "Poly", "Shift", "AUX"][..],
        ),
    ];
    for (page, _) in pages {
        rig.state.page = page;
        rig.frame(vec![]);
        for (other, others) in pages {
            for caption in others {
                let n = if other == page { 3 } else { 0 };
                assert_eq!(rig.count(caption), n, "{page:?}: {caption}");
            }
        }
    }
}

#[test]
fn strip_controls_edit_their_slot_params() {
    use DrumsAction::Param;
    let mut rig = Rig::new(View::Strips);
    // Mix: the pill flips Mute, the text toggle Solo, the sliders drag Pan and Level.
    rig.click(rig.at(1, Ctl::Mute));
    assert_eq!(rig.actions, [Param { slot: 1, param: 3 }]);
    assert_eq!(rig.kit.slots[1].params[3], 1.0);
    rig.click(rig.at(2, Ctl::Solo));
    assert_eq!(rig.actions, [Param { slot: 2, param: 4 }]);
    assert_eq!(rig.kit.slots[2].params[4], 1.0);
    rig.click(rig.at(2, Ctl::Solo));
    assert_eq!(rig.kit.slots[2].params[4], 0.0);
    let quarter = rig.track(0, Ctl::Level) / 4.0;
    let all = rig.drag(rig.at(0, Ctl::Level), vec2(-quarter, 0.0));
    assert!(all.iter().all(|a| *a == Param { slot: 0, param: 0 }) && !all.is_empty());
    assert!(near(rig.kit.slots[0].params[0], 0.8531 - 0.25));
    let quarter = rig.track(0, Ctl::Pan) / 4.0;
    rig.drag(rig.at(0, Ctl::Pan), vec2(quarter, 0.0));
    assert!(near(rig.kit.slots[0].params[1], 0.75));
    // Pitch: Speed snaps to its 49 steps; Start is the other bound knob.
    rig.state.page = StripPage::Pitch;
    rig.frame(vec![]);
    // 60 points up is +0.075: 0.575 lands on step 28 of 48.
    rig.drag(rig.at(1, Ctl::Pitch), vec2(0.0, -60.0));
    assert_eq!(rig.kit.slots[1].params[2], 28.0 / 48.0);
    // Start drags at half the knob law: 0.12 of the range per inch.
    let all = rig.drag(rig.at(2, Ctl::Start), vec2(0.0, KNOB_QUARTER));
    assert!(all.contains(&Param { slot: 2, param: 10 }));
    assert!(near(rig.kit.slots[2].params[10], 0.375));
    // Envelope: ATK DEC SUS REL are params 6 to 9.
    rig.state.page = StripPage::Envelope;
    rig.frame(vec![]);
    // ATK starts at 0 and goes up; the others start at 1 and go down.
    for (ctl, p, dy) in [
        (Ctl::Atk, 6, -KNOB_QUARTER),
        (Ctl::Dec, 7, KNOB_QUARTER),
        (Ctl::Sus, 8, KNOB_QUARTER),
        (Ctl::Rel, 9, KNOB_QUARTER),
    ] {
        let all = rig.drag(rig.at(0, ctl), vec2(0.0, dy));
        assert!(all.contains(&Param { slot: 0, param: p }), "{ctl:?}");
    }
    assert!(near(rig.kit.slots[0].params[6], 0.25) && near(rig.kit.slots[0].params[9], 0.75));
    // Options: Hold is Length (5).
    rig.state.page = StripPage::Options;
    rig.frame(vec![]);
    let half = rig.track(1, Ctl::Hold) / 2.0;
    rig.drag(rig.at(1, Ctl::Hold), vec2(half, 0.0));
    assert!(near(rig.kit.slots[1].params[5], 0.5));
    // Nothing else moved.
    assert_eq!(rig.kit.slots[1].params[0], DRUM_SLOT_PARAMS[0].default);
}

#[test]
fn a_bound_strip_controls_press_is_its_slot_param_touch() {
    let mut rig = Rig::new(View::Strips);
    rig.kit.slots[1].file.id = 7;
    let press = |rig: &mut Rig, row, ctl| {
        rig.state.touch = None;
        let at = rig.at(row, ctl);
        rig.press(at);
        let touch = rig.state.touch.take();
        rig.release(at);
        touch.map(|t| (t.target.instance_id, t.target.param, t.pick))
    };
    // Instance −slot id, the slot param: Level is 0, Mute 3.
    assert_eq!(press(&mut rig, 1, Ctl::Level), Some((-7, 0, None)));
    assert_eq!(press(&mut rig, 1, Ctl::Mute), Some((-7, 3, None)));
    // Bound to no slot param: no touch.
    rig.state.page = StripPage::Pitch;
    rig.frame(vec![]);
    assert_eq!(press(&mut rig, 1, Ctl::Stretch), None);
    assert_eq!(press(&mut rig, 1, Ctl::Pitch), Some((-7, 2, None)));
}

#[test]
fn a_strip_controls_menu_unlinks_only_a_linked_slot_param_and_unbound_knobs_have_no_link_items() {
    let mut rig = Rig::new(View::Strips);
    rig.kit.slots[1].file.id = 7;
    // The channel links CC 20 to slot 7's Level (instance −7, param 0).
    rig.links = vec![CcLink {
        cc: 20,
        instance_id: -7,
        param: 0,
    }];
    let menu = |rig: &mut Rig, row, ctl| {
        let at = rig.at(row, ctl);
        rig.frame(vec![Event::PointerMoved(at)]);
        rig.button(at, PointerButton::Secondary, true);
        rig.button(at, PointerButton::Secondary, false);
        rig.frame(vec![]);
    };
    let escape = |rig: &mut Rig| rig.frame(key_tap(egui::Key::Escape, Default::default()));
    let unlink = |rig: &mut Rig, row, ctl| {
        menu(rig, row, ctl);
        // The secondary press was a touch of its own.
        rig.state.touch = None;
        rig.click(rig.text_at("Unlink from MIDI CC"));
        let touch = rig.state.touch.take();
        escape(rig);
        touch.map(|t| (t.target.instance_id, t.target.param, t.pick))
    };
    assert_eq!(
        unlink(&mut rig, 1, Ctl::Level),
        Some((-7, 0, Some(CtrlAction::UnlinkMidiCc)))
    );
    // Its Mute is not linked: dimmed, nothing picked.
    assert_eq!(unlink(&mut rig, 1, Ctl::Mute), None);
    // Stretch is bound to no slot param: Reset only.
    rig.state.page = StripPage::Pitch;
    rig.frame(vec![]);
    menu(&mut rig, 1, Ctl::Stretch);
    assert_eq!(rig.count("Reset"), 1);
    assert_eq!(rig.count("Link to MIDI CC"), 0);
}

#[test]
fn strip_controls_offer_fine_tune_control_for_slot_params_but_mute_and_solo() {
    let mut rig = Rig::new(View::Strips);
    let offers = |rig: &mut Rig, ctl| {
        let at = rig.at(1, ctl);
        rig.frame(vec![Event::PointerMoved(at)]);
        rig.button(at, PointerButton::Secondary, true);
        rig.button(at, PointerButton::Secondary, false);
        rig.frame(vec![]);
        let open = rig.count("Reset") == 1 || rig.count("Add automation track") == 1;
        assert!(open, "{ctl:?}: the menu is open");
        let offered = rig.count("Fine tune control") == 1;
        rig.frame(key_tap(egui::Key::Escape, Default::default()));
        rig.frame(vec![]);
        offered
    };
    // Mix page: Level and Pan, not Mute or Solo (their menus have no Reset: toggles).
    assert!(offers(&mut rig, Ctl::Level));
    assert!(offers(&mut rig, Ctl::Pan));
    assert!(!offers(&mut rig, Ctl::Mute));
    assert!(!offers(&mut rig, Ctl::Solo));
    // Pitch page: the Pitch knob (Speed, p2) and Start (p10); Stretch is bound to no slot param.
    rig.state.page = StripPage::Pitch;
    rig.frame(vec![]);
    assert!(offers(&mut rig, Ctl::Pitch));
    assert!(offers(&mut rig, Ctl::Start));
    assert!(!offers(&mut rig, Ctl::Stretch));
    // The envelope knobs (p6…p9) and Hold (Length, p5).
    rig.state.page = StripPage::Envelope;
    rig.frame(vec![]);
    for ctl in [Ctl::Atk, Ctl::Dec, Ctl::Sus, Ctl::Rel] {
        assert!(offers(&mut rig, ctl), "{ctl:?}");
    }
    rig.state.page = StripPage::Options;
    rig.frame(vec![]);
    assert!(offers(&mut rig, Ctl::Hold));
}

#[test]
fn the_cut_menu_picks_the_cut_group() {
    let mut rig = Rig::new(View::Strips);
    rig.state.page = StripPage::Options;
    rig.frame(vec![]);
    rig.click(rig.at(1, Ctl::Cut));
    rig.frame(vec![]);
    for item in CUT_ITEMS {
        assert_eq!(rig.count(item), 1, "{item}");
    }
    rig.click(rig.text_at("Cut group B"));
    assert_eq!(rig.actions, [DrumsAction::CutGroup(1)]);
    assert_eq!(rig.kit.slots[1].cut_group, 2);
    rig.frame(vec![]);
    assert_eq!(rig.count("Cut group B"), 0, "closed");
}

// ---- The strip controls' looks, value laws and actions ----

#[test]
fn the_strip_value_laws() {
    // Pitch: trunc(48v − 24), no sign or unit; truncated toward zero, not rounded (0.96 and −0.96 are 0).
    for (v, want) in [
        (0.0, "-24"),
        (0.25, "-12"),
        (0.48, "0"),
        (0.5, "0"),
        (0.52, "0"),
        (0.75, "12"),
        (1.0, "24"),
    ] {
        assert_eq!(pitch_text(v), want, "{v}");
    }
    // Stretch: ratio 2^(2v − 1), `%.2f`; the knob sits at log2(ratio)/2 + 0.5.
    for (v, ratio, text) in [(0.0, 0.5, "0.50"), (0.5, 1.0, "1.00"), (1.0, 2.0, "2.00")] {
        assert_eq!(stretch_ratio(v), ratio, "{v}");
        assert_eq!(stretch_text(v), text, "{v}");
        assert_eq!(stretch_value(ratio), v, "{ratio}");
    }
    assert_eq!(stretch_text(0.75), "1.41");
    // Out of the knob's range, or not a number: held to an end, or at rest.
    assert_eq!((stretch_value(4.0), stretch_value(f64::NAN)), (1.0, 0.5));
    // Shift: k = trunc(64v − 32), the centre step twice as wide (31/64 < v < 33/64 is 0).
    let below = |v: f32| f32::from_bits(v.to_bits() - 1);
    let above = |v: f32| f32::from_bits(v.to_bits() + 1);
    let (lo, hi) = (31.0 / 64.0, 33.0 / 64.0);
    assert_eq!(
        [lo, above(lo), 0.5, below(hi), hi].map(shift_step),
        [-1, 0, 0, 0, 1]
    );
    assert_eq!((shift_step(0.0), shift_step(1.0)), (-32, 32));
    // Text: trunc(k/128 beat × 400), percent of a sixteenth: k = 1 → 3, 8 → 25, 32 → 100.
    let at = |k: i32| (k + 32) as f32 / 64.0;
    for (k, want) in [
        (1, "3"),
        (5, "15"),
        (-5, "-15"),
        (8, "25"),
        (32, "100"),
        (0, "0"),
        (-1, "-3"),
        (-32, "-100"),
    ] {
        assert_eq!(shift_text(at(k)), want, "k {k}");
    }
}

impl Rig {
    /// Every text shape: text, centre, font size (px) and colour.
    fn text_shapes(&self) -> Vec<(String, Pos2, f32, Color32)> {
        (self.shapes.iter())
            .filter_map(|c| match &c.shape {
                Shape::Text(t) => Some((
                    t.galley.text().to_owned(),
                    t.pos + t.galley.rect.center().to_vec2(),
                    t.galley.job.sections[0].format.font_id.size,
                    t.fallback_color,
                )),
                _ => None,
            })
            .collect()
    }

    /// The text shapes centred inside `rect`.
    fn texts_in(&self, rect: Rect) -> Vec<(String, Pos2, f32, Color32)> {
        let shapes = self.text_shapes().into_iter();
        shapes.filter(|t| rect.contains(t.1)).collect()
    }

    /// The last disc drawn centred on `at`: radius and fill.
    fn disc(&self, at: Pos2) -> Option<(f32, Color32)> {
        (self.shapes.iter().rev()).find_map(|c| match &c.shape {
            Shape::Circle(c) if (c.center - at).length() < 0.01 => Some((c.radius, c.fill)),
            _ => None,
        })
    }

    /// A frame with the pointer gone: nothing hovered.
    fn away(&mut self) {
        self.frame(vec![Event::PointerGone]);
    }

    /// Opens control `ctl`'s parameter menu on strip `row` with a secondary click.
    fn param_menu(&mut self, row: usize, ctl: Ctl) {
        let at = self.at(row, ctl);
        self.frame(vec![Event::PointerMoved(at)]);
        self.button(at, PointerButton::Secondary, true);
        self.button(at, PointerButton::Secondary, false);
        self.frame(vec![]);
    }
}

fn colors() -> (Color32, Color32) {
    let colors = theme_by_id(0).colors;
    (colors.text.to_color32(), colors.module_fill.to_color32())
}

#[test]
fn strip_knobs_caption_a_left_column_and_show_a_fixed_8_pt_value_on_the_disc() {
    let mut rig = Rig::new(View::Strips);
    rig.kit.slots[0].params[10] = 0.0;
    let column = |rect: Rect| Rect::from_min_size(rect.min, vec2(1.25 * U, rect.height()));
    let check = |rig: &Rig, ctl: Ctl, caption: &str, value: Option<&str>| {
        let rect = rig.rect_at(0, ctl);
        let texts = rig.texts_in(rect);
        let (text, at, px, color) = texts
            .iter()
            .find(|t| t.0 == caption)
            .cloned()
            .unwrap_or_else(|| panic!("{caption}: {texts:?}"));
        assert_eq!((text.as_str(), px, color), (caption, 16.0, Color32::RED));
        assert!(
            (at - column(rect).center()).length() < 1.0,
            "{caption} at {at:?}"
        );
        let disc = rect.with_min_x(rect.left() + 1.25 * U);
        let values: Vec<_> = texts.iter().filter(|t| t.0 != caption).collect();
        match value {
            Some(value) => {
                assert_eq!(values.len(), 1, "{caption}: {values:?}");
                let (text, at, px, color) = values[0].clone();
                assert_eq!((text.as_str(), px, color), (value, 16.0, Color32::RED));
                assert!((at - disc.center()).length() < 1.0, "{value} at {at:?}");
            }
            None => assert!(values.is_empty(), "{caption}: {values:?}"),
        }
    };
    rig.state.page = StripPage::Pitch;
    rig.frame(vec![]);
    check(&rig, Ctl::Pitch, "Pitch", Some("0"));
    check(&rig, Ctl::Stretch, "Stretch", Some("1.00"));
    // Never fitted: the widest Start text stays 8 pt.
    check(&rig, Ctl::Start, "Start", Some("-500"));
    rig.state.page = StripPage::Envelope;
    rig.frame(vec![]);
    for (ctl, caption) in [
        (Ctl::Atk, "ATK"),
        (Ctl::Dec, "DEC"),
        (Ctl::Sus, "SUS"),
        (Ctl::Rel, "REL"),
    ] {
        check(&rig, ctl, caption, None);
    }
    rig.state.page = StripPage::Options;
    rig.frame(vec![]);
    check(&rig, Ctl::Shift, "Shift", Some("0"));
}

#[test]
fn strip_knobs_reset_to_their_own_values_rel_to_0() {
    let mut rig = Rig::new(View::Strips);
    let reset = |rig: &mut Rig, ctl: Ctl| {
        // A second apart from any other touch, then two touch-downs: the second resets.
        rig.time += 1.0;
        let at = rig.at(1, ctl);
        rig.click(at);
        rig.press(at);
        rig.release(at);
    };
    rig.state.page = StripPage::Envelope;
    rig.frame(vec![]);
    for (ctl, p, want) in [
        (Ctl::Atk, 6, 0.0),
        (Ctl::Dec, 7, 1.0),
        (Ctl::Sus, 8, 1.0),
        (Ctl::Rel, 9, 0.0),
    ] {
        rig.kit.slots[1].params[p] = 0.3;
        reset(&mut rig, ctl);
        assert_eq!(rig.kit.slots[1].params[p], want, "{ctl:?}");
    }
    rig.state.page = StripPage::Pitch;
    rig.frame(vec![]);
    for (ctl, p) in [(Ctl::Pitch, 2), (Ctl::Start, 10)] {
        rig.kit.slots[1].params[p] = 0.3;
        reset(&mut rig, ctl);
        assert_eq!(rig.kit.slots[1].params[p], 0.5, "{ctl:?}");
    }
}

#[test]
fn rev_poly_and_solo_are_discs_lit_in_text_when_on() {
    let (text, module_fill) = colors();
    let mut rig = Rig::new(View::Strips);
    // (page, control, k, label px)
    for (page, ctl, k, px) in [
        (StripPage::Pitch, Ctl::Rev, 0.9, 16.0),
        (StripPage::Options, Ctl::Poly, 0.8, 20.0),
        (StripPage::Mix, Ctl::Solo, 0.9, 16.0),
    ] {
        rig.state.page = page;
        rig.away();
        let rect = rig.rect_at(1, ctl);
        let r = k * 0.5 * rect.width().min(rect.height());
        let look = |rig: &Rig| {
            let label = rig.texts_in(rect).into_iter().next().expect("a label");
            let disc = rig.disc(rect.center()).expect("a disc");
            assert!(near(disc.0, r), "{ctl:?}: {} vs {r}", disc.0);
            assert_eq!(label.2, px, "{ctl:?}");
            (disc.1, label.3)
        };
        // `Poly` starts on (MAIN@1050 defaults to 1), the others off.
        let on = ctl == Ctl::Poly;
        let lit = |on: bool| {
            if on {
                (text, module_fill)
            } else {
                (module_fill, text)
            }
        };
        assert_eq!(look(&rig), lit(on), "{ctl:?}");
        rig.click(rect.center());
        rig.away();
        assert_eq!(look(&rig), lit(!on), "{ctl:?} flipped");
        // A press grows the disc by 5 %.
        rig.press(rect.center());
        rig.frame(vec![]);
        let grown = rig.disc(rect.center()).expect("a disc").0;
        assert!(near(grown, 1.05 * r), "{ctl:?}: {grown}");
        rig.release(rect.center());
        rig.click(rect.center());
    }
}

#[test]
fn rev_and_poly_toggle_the_slots_main_flags() {
    let mut rig = Rig::new(View::Strips);
    rig.state.page = StripPage::Pitch;
    rig.frame(vec![]);
    rig.click(rig.at(1, Ctl::Rev));
    assert_eq!(rig.actions, [DrumsAction::Reverse(1)]);
    assert!(rig.kit.slots[1].file.reverse);
    rig.click(rig.at(1, Ctl::Rev));
    assert!(!rig.kit.slots[1].file.reverse);
    rig.state.page = StripPage::Options;
    rig.frame(vec![]);
    rig.click(rig.at(2, Ctl::Poly));
    assert_eq!(rig.actions, [DrumsAction::Poly(2)]);
    assert!(!rig.kit.slots[2].file.overlapping_voices);
    // Neither is bound to a slot param: no touch, and no parameter menu.
    assert_eq!(rig.state.touch, None);
    rig.param_menu(2, Ctl::Poly);
    assert_eq!(rig.count("Add automation track"), 0);
}

#[test]
fn the_icon_button_is_a_hidden_disc_under_a_text_icon_and_asks_for_the_pads_sample_picker() {
    let (text, module_fill) = colors();
    let mut rig = Rig::new(View::Strips);
    rig.state.page = StripPage::Pitch;
    rig.away();
    let rect = rig.rect_at(2, Ctl::Sample);
    let r = 0.9 * 0.5 * rect.width().min(rect.height());
    let disc = rig.disc(rect.center()).expect("a disc");
    assert!(near(disc.0, r) && disc.1 == module_fill, "{disc:?}");
    // Every tap acts: no on state.
    for _ in 0..2 {
        rig.click(rect.center());
        assert_eq!(rig.actions, [DrumsAction::LoadSample(2)]);
        rig.away();
        assert_eq!(rig.disc(rect.center()).map(|d| d.1), Some(module_fill));
    }
    // The icon is the only ink: no `text` disc of the button's size behind it.
    let behind = |c: &ClippedShape| {
        matches!(&c.shape, Shape::Circle(c)
            if c.fill == text && c.radius > r / 2.0 && rect.contains(c.center))
    };
    assert!(!rig.shapes.iter().any(behind));
}

#[test]
fn stretch_follows_the_finger_in_its_text_and_sets_the_str1_ratio_on_release() {
    let mut rig = Rig::new(View::Strips);
    rig.state.page = StripPage::Pitch;
    rig.frame(vec![]);
    let at = rig.at(1, Ctl::Stretch);
    rig.press(at);
    for k in 1..=3 {
        rig.frame(vec![Event::PointerMoved(
            at - vec2(0.0, KNOB_QUARTER * k as f32 / 3.0),
        )]);
        assert!(rig.actions.is_empty(), "{:?}", rig.actions);
    }
    // Held: the slot keeps ratio 1, the knob reads 2^(2·0.75 − 1).
    let str1 = |rig: &Rig| {
        let file = &rig.kit.slots[1].file;
        (file.stretch_mode, file.stretch_ratio)
    };
    assert_eq!(str1(&rig), (0, 1.0));
    assert_eq!(rig.count("1.41"), 1);
    rig.release(at - vec2(0.0, KNOB_QUARTER));
    assert_eq!(rig.actions, [DrumsAction::Stretch(1)]);
    let (mode, ratio) = str1(&rig);
    assert!(
        mode == 1 && (ratio - 2f64.sqrt()).abs() < 1e-6,
        "{mode} {ratio}"
    );
    rig.frame(vec![]);
    assert_eq!(rig.count("1.41"), 1, "the knob shows the stored ratio");
    // `Reset` from the parameter menu, with no finger on the knob, applies at once: ratio 1, mode 0.
    rig.param_menu(1, Ctl::Stretch);
    rig.click(rig.text_at("Reset"));
    assert_eq!(rig.actions, [DrumsAction::Stretch(1)]);
    assert_eq!(str1(&rig), (0, 1.0));
    // The same ratio again does nothing: a double tap at 1.00.
    rig.time += 1.0;
    rig.click(at);
    rig.press(at);
    assert!(rig.actions.is_empty());
    rig.release(at);
    assert!(rig.actions.is_empty());
    assert_eq!(str1(&rig), (0, 1.0));
}

#[test]
fn shift_writes_the_beat_offset_and_its_control_value_when_its_step_changes() {
    let mut rig = Rig::new(View::Strips);
    rig.state.page = StripPage::Options;
    rig.frame(vec![]);
    let at = rig.at(1, Ctl::Shift);
    let file = |rig: &Rig| rig.kit.slots[1].file.clone();
    // 216 points up: v = 0.77, k = 17, offset 17/128 beat, shown as 53 (% of a sixteenth). Written at the release.
    let all = rig.drag(at, vec2(0.0, -216.0));
    assert_eq!(all, [DrumsAction::Shift(1)]);
    let after = file(&rig);
    assert_eq!(after.beat_offset, 17.0 / 128.0);
    assert!(near(after.beat_offset_control, 0.77), "{after:?}");
    rig.frame(vec![]);
    assert_eq!(rig.count("53"), 1);
    // A move inside the step (8 points: +0.01) writes nothing, @1045 included.
    rig.time += 1.0;
    let all = rig.drag(at, vec2(0.0, -8.0));
    assert!(all.is_empty(), "{all:?}");
    assert_eq!(file(&rig), after);
}

#[test]
fn a_stretch_that_lands_on_the_stored_ratio_does_nothing() {
    // A stretch to the stored ratio does nothing. At ratio 2 the knob is at its top: down, then back up past it, ends
    // where it started.
    let mut rig = Rig::new(View::Strips);
    (
        rig.kit.slots[1].file.stretch_mode,
        rig.kit.slots[1].file.stretch_ratio,
    ) = (1, 2.0);
    rig.state.page = StripPage::Pitch;
    rig.frame(vec![]);
    let at = rig.at(1, Ctl::Stretch);
    rig.press(at);
    let mut all = Vec::new();
    for dy in [50.0, 100.0, -100.0, -300.0] {
        rig.frame(vec![Event::PointerMoved(at + vec2(0.0, dy))]);
        all.extend(rig.actions.clone());
    }
    assert_eq!(rig.count("2.00"), 1, "back at the top");
    rig.release(at - vec2(0.0, 300.0));
    all.extend(rig.actions.clone());
    assert!(all.is_empty(), "{all:?}");
    assert_eq!(rig.kit.slots[1].file.stretch_ratio, 2.0);
}

#[test]
fn the_cut_menu_shows_no_current_item() {
    // Its current item is never synced from the slot; the header badge is the only display.
    let mut rig = Rig::new(View::Strips);
    rig.kit.slots[1].cut_group = 2;
    rig.state.page = StripPage::Options;
    rig.frame(vec![]);
    rig.click(rig.at(1, Ctl::Cut));
    rig.frame(vec![]);
    let color = |rig: &Rig, item: &str| rig.texts().into_iter().find(|t| t.0 == item).map(|t| t.1);
    let plain = color(&rig, "Cut group A");
    assert!(plain.is_some());
    for item in CUT_ITEMS {
        assert_eq!(color(&rig, item), plain, "{item}");
    }
}

/// The text shapes showing `full`, whole or with an ellipsis: text, the rect its galley covers, font size (px).
fn showing(rig: &Rig, full: &str) -> Vec<(String, Rect, f32)> {
    let shows = |t: &str| {
        t == full
            || t.split_once("...")
                .is_some_and(|(h, _)| !h.is_empty() && full.starts_with(h))
    };
    rig.shapes
        .iter()
        .filter_map(|c| match &c.shape {
            Shape::Text(t) if shows(t.galley.text()) => Some((
                t.galley.text().to_owned(),
                Rect::from_min_size(t.pos, t.galley.size()),
                t.galley.job.sections[0].format.font_id.size,
            )),
            _ => None,
        })
        .collect()
}

/// `shown` is `full` with a middle ellipsis: the head a prefix, the tail (returned) a suffix.
fn cut<'a>(shown: &'a str, full: &str) -> &'a str {
    let (head, tail) = shown
        .split_once("...")
        .unwrap_or_else(|| panic!("{shown}: no ..."));
    assert!(
        !head.is_empty() && full.starts_with(head) && full.ends_with(tail),
        "{shown}"
    );
    tail
}

#[test]
fn the_header_cells_keep_their_room_but_are_neither_drawn_nor_sensed() {
    // The host's row headers (the step view's) fill them.
    let mut rig = Rig::new(View::Strips);
    rig.frame(vec![]);
    assert!(rig.texts().iter().all(|(t, _)| t != "Hat"), "a header name");
    rig.click(rig.header(1));
    rig.frame(vec![Event::PointerMoved(rig.header(1))]);
    rig.button(rig.header(1), PointerButton::Secondary, true);
    rig.button(rig.header(1), PointerButton::Secondary, false);
    assert!(rig.actions.is_empty());
    assert!(rig.header(1).x < rig.at(1, Ctl::Mute).x);
}

/// Width, font size (px) and clip rect of the text shape reading `text`.
fn drawn(rig: &Rig, text: &str) -> Option<(f32, f32, Rect)> {
    rig.shapes.iter().find_map(|c| match &c.shape {
        Shape::Text(t) if t.galley.text() == text => Some((
            t.galley.rect.width(),
            t.galley.job.sections[0].format.font_id.size,
            c.clip_rect,
        )),
        _ => None,
    })
}

#[test]
fn pad_names_shrink_to_fit_and_are_truncated_at_6_pt() {
    // Both lines shrink to fit, a point at a time down to 6 pt, then are truncated at 6 pt keeping 5 tail characters.
    // A box 1.5 r wide, centred 0.25 U above / below the pad's centre.
    let mut rig = Rig::new(View::Pads);
    let long = "A sample name far too long for any pad";
    rig.kit.slots[0].name = long.to_owned();
    rig.kit.slots[1].name = "Snare Tight".to_owned();
    rig.frame(vec![]);
    // The text box: 1.5 r = 93 px.
    let max_w = 1.5 * 62.0;
    let found = showing(&rig, long);
    assert_eq!(found.len(), 1, "{found:?}");
    let (shown, rect, px) = found[0].clone();
    assert_eq!((cut(&shown, long), px), ("y pad", 12.0));
    assert!(rect.width() <= max_w, "{shown}: {rect:?}");
    let below = grid().centres[0] + vec2(0.0, 0.25 * U);
    assert!((rect.center() - below).length() < 1.0, "{rect:?}");
    // A name that fits keeps its 10 pt (20 px); a longer one steps down a point at a time, in full.
    assert_eq!(drawn(&rig, "Snare 01").map(|s| s.1), None);
    assert_eq!(drawn(&rig, "Hat 01").map(|s| s.1), Some(20.0));
    let (w, px, _) = drawn(&rig, "Snare Tight").expect("drawn");
    assert!(w <= max_w && px < 20.0 && px > 12.0, "{w} {px}");
    let above = grid().centres[1] - vec2(0.0, 0.25 * U);
    assert!((rig.text_at("Snare") - above).length() < 1.0);
}

#[test]
fn the_level_slider_resets_to_its_own_value_not_the_stored_default() {
    let mut rig = Rig::new(View::Strips);
    assert_eq!(rig.kit.slots[1].params[0], 0.8531);
    // A double tap resets the strip's Level to 0.90909, not to the 0.8531 a new slot stores, on the second
    // touch-down.
    let at = rig.at(1, Ctl::Level);
    rig.click(at);
    assert_eq!(rig.kit.slots[1].params[0], 0.8531);
    rig.press(at);
    assert_eq!(rig.kit.slots[1].params[0], 0.90909);
    assert_eq!(rig.actions, [DrumsAction::Param { slot: 1, param: 0 }]);
    rig.release(at);
    // Other bound controls reset to their stored default.
    let pan = rig.at(1, Ctl::Pan);
    rig.kit.slots[1].params[1] = 0.75;
    // A second apart, so these two clicks are not the third and fourth of a run.
    rig.time += 1.0;
    rig.click(pan);
    rig.click(pan);
    assert_eq!(rig.kit.slots[1].params[1], 0.5);
}

#[test]
fn a_tap_within_one_frame_still_plays_the_pad() {
    let mut rig = Rig::new(View::Pads);
    let at = grid().centres[4];
    rig.frame(vec![Event::PointerMoved(at)]);
    let button = |pressed| Event::PointerButton {
        pos: at,
        button: PointerButton::Primary,
        pressed,
        modifiers: Default::default(),
    };
    rig.frame(vec![button(true), button(false)]);
    assert_eq!(
        rig.actions,
        [DrumsAction::PadDown(4), DrumsAction::PadUp(4)]
    );
    assert!(rig.state.fingers.is_empty());
    // A tap outside every pad still does nothing.
    rig.frame(vec![Event::PointerMoved(pos2(2.0, 2.0))]);
    let off = |pressed| Event::PointerButton {
        pos: pos2(2.0, 2.0),
        button: PointerButton::Primary,
        pressed,
        modifiers: Default::default(),
    };
    rig.frame(vec![off(true), off(false)]);
    assert!(rig.actions.is_empty());
}

#[test]
fn pad_fans_and_names_are_both_drawn_unclipped() {
    let rig = Rig::new(View::Pads);
    for c in &rig.shapes {
        if matches!(c.shape, Shape::Mesh(_) | Shape::Text(_)) {
            assert!(c.clip_rect.contains_rect(SCREEN), "{:?}", c.clip_rect);
        }
    }
}

#[test]
fn a_row_header_shows_the_custom_name_alone_else_the_folder_over_the_file_name() {
    // Line 1 is the custom name (`MAIN`@1051) if present, else the sample file's parent folder; line 2, the file name
    // (`MAIN`@12), only without a custom name.
    let mut slot = SamplerSlot {
        name: "Big Kick".into(),
        folder: "My Samples/Kicks".into(),
        ..SamplerSlot::default()
    };
    assert_eq!(row_lines(&slot), ("Kicks", "Big Kick"));
    slot.file.custom_pad_name = "Mine".into();
    assert_eq!(row_lines(&slot), ("Mine", ""));
}

#[test]
fn a_hex_pad_shows_the_folder_over_the_file_name_and_never_the_custom_name() {
    let mut slot = SamplerSlot {
        name: "Big Kick".into(),
        folder: "Drum Samples\\Kicks".into(),
        ..SamplerSlot::default()
    };
    assert_eq!(hex_lines(&slot), ("Kicks", "Big Kick"));
    slot.file.custom_pad_name = "Mine".into();
    assert_eq!(hex_lines(&slot), ("Kicks", "Big Kick"));
    // Drawn so on the pad.
    let mut rig = Rig::new(View::Pads);
    rig.kit.slots[0].file.custom_pad_name = "Mine".into();
    rig.frame(vec![]);
    assert_eq!((rig.count("Kick"), rig.count("Kick 01")), (1, 1));
    assert_eq!(rig.count("Mine"), 0);
}

#[test]
fn the_meter_level_is_minus_60_to_plus_6_db_over_0_to_1() {
    // f = (20·log10 L + 60)/66 = 0.30303·log10 L + 0.90909, 0 below L = 0.001, capped at 1.
    assert_eq!(meter_level(0.0), 0.0);
    assert_eq!(meter_level(0.0009), 0.0);
    assert!(meter_level(0.001).abs() < 1e-6);
    assert!((meter_level(1.0) - 60.0 / 66.0).abs() < 1e-6);
    assert!((meter_level(0.1) - 40.0 / 66.0).abs() < 1e-6);
    assert_eq!(meter_level(4.0), 1.0);
}

#[test]
fn every_pad_has_a_level_ring_2_hairlines_out_mixed_toward_the_channel_colour() {
    // mix(channel·g, controlFill·(1 − g)), so a silent pad's ring is controlFill.
    let mut rig = Rig::new(View::Pads);
    // Slot 0 at full level, slot 1 below the floor, slot 2 at −30 dB; the unused pads have no peak.
    rig.state.peaks = vec![1.0, 0.0005, 0.0316];
    rig.frame(vec![]);
    let rings = rig.rings();
    assert_eq!(rings.len(), 22);
    let centres = grid().centres;
    assert!(
        rings
            .iter()
            .zip(centres)
            .all(|(h, c)| (h.1 - c).length() <= 1.0)
    );
    let control = theme_by_id(0).colors.control_fill.to_color32();
    let glow = |l: f32| control.lerp_to_gamma(Color32::RED, meter_level(l).powf(1.6667));
    assert_eq!(rings[0].0, glow(1.0));
    assert_eq!(rings[1].0, control, "a silent pad");
    assert_eq!(rings[2].0, glow(0.0316));
    assert!(rings[3..].iter().all(|h| h.0 == control), "unused pads");
}

#[test]
fn a_rearranged_pad_glows_with_the_slot_it_plays() {
    // Pads 0 and 4 swapped: slot 0 sounds, so pad 4 glows and pad 0 (slot 4, silent) is at rest.
    let mut rig = Rig::new(View::Pads);
    rig.kit.file.pads.swap(0, 4);
    rig.state.peaks = vec![1.0];
    rig.frame(vec![]);
    let rings = rig.rings();
    let control = theme_by_id(0).colors.control_fill.to_color32();
    let lit = control.lerp_to_gamma(Color32::RED, meter_level(1.0).powf(1.6667));
    assert_eq!((rings[4].0, rings[0].0), (lit, control));
}

// Touches and rearranging.

fn touch(id: u64, phase: egui::TouchPhase, pos: Pos2) -> Event {
    Event::Touch {
        device_id: egui::TouchDeviceId(0),
        id: egui::TouchId(id),
        phase,
        pos,
        force: None,
    }
}

use egui::TouchPhase::{End, Move, Start};

#[test]
fn every_finger_plays_its_own_pad_and_one_on_a_held_pad_ends_that_note_first() {
    let mut rig = Rig::new(View::Pads);
    let c = grid().centres;
    rig.frame(vec![touch(1, Start, c[0])]);
    rig.frame(vec![touch(2, Start, c[1])]);
    assert_eq!(rig.actions, [DrumsAction::PadDown(1)]);
    assert_eq!(rig.held(), [0, 1]);
    // A third finger on pad 0: the first finger's note ends, then pad 0 plays again; the first finger's lift is
    // nothing more.
    rig.frame(vec![touch(3, Start, c[0] + vec2(5.0, 0.0))]);
    assert_eq!(
        rig.actions,
        [DrumsAction::PadUp(0), DrumsAction::PadDown(0)]
    );
    assert_eq!(rig.held(), [1, 0]);
    rig.frame(vec![touch(1, End, c[0])]);
    assert!(rig.actions.is_empty());
    rig.frame(vec![touch(2, End, c[1]), touch(3, End, c[0])]);
    assert_eq!(rig.actions, [DrumsAction::PadUp(1), DrumsAction::PadUp(0)]);
    assert!(rig.state.fingers.is_empty());
}

#[test]
fn a_held_pad_moved_past_trunc_1_5_u_is_picked_up_unless_the_pads_are_locked() {
    // U = 32: trunc(48) px on x or y.
    let mut rig = Rig::new(View::Pads);
    let c = grid().centres[0];
    rig.frame(vec![touch(1, Start, c)]);
    rig.frame(vec![touch(1, Move, c + vec2(48.0, -48.0))]);
    assert!(rig.actions.is_empty());
    assert_eq!(rig.held(), [0]);
    rig.frame(vec![touch(1, Move, c + vec2(0.0, 49.0))]);
    assert_eq!(rig.actions, [DrumsAction::PadUp(0)]);
    assert!(rig.held().is_empty() && rig.state.fingers[0].lifted.is_some());
    // Locked, the pad stays held wherever the finger goes.
    let mut rig = Rig::new(View::Pads);
    rig.locked = true;
    rig.frame(vec![touch(1, Start, c)]);
    rig.frame(vec![touch(1, Move, c + vec2(200.0, 100.0))]);
    assert!(rig.actions.is_empty());
    assert_eq!(rig.held(), [0]);
}

/// Pad 0 picked up by finger 1 and carried to `to`; the frame that picks it up is the first of its shrinking.
fn carry(rig: &mut Rig, to: Pos2) {
    let c = grid().centres[0];
    rig.frame(vec![touch(1, Start, c)]);
    rig.frame(vec![touch(1, Move, to)]);
}

#[test]
fn a_picked_up_pad_is_drawn_under_the_finger_shrinking_toward_85_percent_over_an_empty_home() {
    let mut rig = Rig::new(View::Pads);
    let to = pos2(30.0, 380.0);
    carry(&mut rig, to);
    let module_fill = theme_by_id(0).colors.module_fill.to_color32();
    let r = grid().r;
    let mut f = 0.0;
    for _ in 0..4 {
        f += 0.2 * (1.0 - f);
        let hexes = rig.hexes_at();
        // Home first, moduleFill; the carried pad last, over everything, in the channel colour.
        assert_eq!(hexes[0].0, module_fill);
        let (fill, centre, radius) = *hexes.last().unwrap();
        assert_eq!(fill, Color32::RED);
        assert!((centre - to).length() <= 1.0, "{centre:?}");
        // Size trunc((1 − 0.15 f)·r).
        assert!(
            near(radius, visible(((1.0 - 0.15 * f) * r).trunc())),
            "{radius} vs f {f}"
        );
        assert!(rig.state.fingers[0].lifted.is_some_and(|s| near(s, f)));
        rig.frame(vec![]);
    }
    // No text on a picked-up pad, under the finger or at home.
    assert_eq!(rig.count("Kick") + rig.count("Kick 01"), 0);
    assert_eq!(rig.count("Snare"), 1);
}

#[test]
fn the_pad_under_a_picked_up_pad_rings_and_the_ring_fades_when_it_leaves() {
    let mut rig = Rig::new(View::Pads);
    let g = grid();
    carry(&mut rig, g.centres[5]);
    let control_fill = theme_by_id(0).colors.control_fill.to_color32();
    let mix = |h: f32| control_fill.lerp_to_gamma(Color32::RED, h);
    // Size r + ceil(32 / 16) = r + 2 around pad 5, in the channel colour h of the way from `controlFill`: h = 0.2,
    // then 0.36.
    let ring = |rig: &Rig, pad: usize| {
        (rig.hexes_at().into_iter())
            .find(|&(_, c, radius)| {
                (c - g.centres[pad]).length() <= 1.0 && near(radius, visible(g.r + 2.0))
            })
            .map(|(fill, ..)| fill)
    };
    assert_eq!(ring(&rig, 5), Some(mix(0.2)));
    rig.frame(vec![]);
    assert_eq!(ring(&rig, 5), Some(mix(0.36)));
    assert!(near(rig.state.rings[5], 0.36));
    // Over pad 6: pad 5 fades (0.8 h), pad 6 rings; its own home never rings.
    rig.frame(vec![touch(1, Move, g.centres[6])]);
    assert!(near(rig.state.rings[5], 0.288) && near(rig.state.rings[6], 0.2));
    rig.frame(vec![touch(1, Move, g.centres[0])]);
    assert!(near(rig.state.rings[0], 0.0) && near(rig.state.rings[6], 0.16));
}

#[test]
fn dropping_on_another_pad_swaps_the_two_and_anywhere_else_does_nothing() {
    let g = grid();
    let mut rig = Rig::new(View::Pads);
    carry(&mut rig, g.centres[5]);
    rig.frame(vec![touch(1, End, g.centres[5])]);
    assert_eq!(rig.actions, [DrumsAction::Swap(0, 5)]);
    assert!(rig.state.fingers.is_empty());
    // Between the pads, or back home: nothing.
    for to in [pos2(2.0, 2.0), g.centres[0] + vec2(0.0, 30.0)] {
        let mut rig = Rig::new(View::Pads);
        carry(&mut rig, g.centres[5]);
        rig.frame(vec![touch(1, Move, to)]);
        rig.frame(vec![touch(1, End, to)]);
        assert!(rig.actions.is_empty(), "{to:?}");
    }
}

#[test]
fn each_pad_shows_and_plays_the_slot_its_pads_entry_names() {
    let mut rig = Rig::new(View::Pads);
    rig.kit.file.pads.swap(0, 4);
    rig.frame(vec![]);
    let module_fill = theme_by_id(0).colors.module_fill.to_color32();
    let fills: Vec<Color32> = rig.hexes().iter().take(5).map(|h| h.0).collect();
    assert_eq!(
        fills,
        [
            module_fill,
            Color32::RED,
            Color32::RED,
            module_fill,
            Color32::RED
        ]
    );
    assert!((rig.text_at("Kick") - grid().centres[4]).length() < grid().r);
    assert_eq!((pad_slot(&rig.kit, 4), pad_slot(&rig.kit, 0)), (0, 4));
}

#[test]
fn a_finger_down_again_without_its_lift_lets_go_of_its_pad_first() {
    // A lost lift (the finger id reused): the old hold ends, so its note never hangs.
    let mut rig = Rig::new(View::Pads);
    let c = grid().centres;
    rig.frame(vec![touch(1, Start, c[0])]);
    rig.frame(vec![touch(1, Start, c[2])]);
    assert_eq!(
        rig.actions,
        [DrumsAction::PadUp(0), DrumsAction::PadDown(2)]
    );
    assert_eq!(rig.held(), [2]);
}

// ---- the pad Send menu ----

#[test]
fn the_aux_menu_is_default_the_send_channels_and_create_fx_channel() {
    let sends = [(4, "Effect 4".to_owned()), (9, "Verb".to_owned())];
    let items = |target| -> Vec<(String, bool)> {
        aux_menu(target, &sends)
            .into_iter()
            .map(|i| (i.label.into_owned(), i.current))
            .collect()
    };
    let item = |label: &str, current| (label.to_owned(), current);
    // The current target highlighted.
    assert_eq!(
        items(-1),
        [
            item("Default", true),
            item("Effect 4", false),
            item("Verb", false),
            item("Create FX Channel", false),
        ]
    );
    assert_eq!(items(9)[2], item("Verb", true));
    // A file's 0 (the master) is no item: none is current.
    assert!(items(0).iter().all(|(_, current)| !current));
}

#[test]
fn the_aux_menu_sets_the_pads_send_target() {
    let mut rig = Rig::new(View::Strips);
    rig.state.page = StripPage::Options;
    rig.state.sends = vec![(4, "Effect 4".to_owned())];
    rig.frame(vec![]);
    let open = |rig: &mut Rig| {
        rig.click(rig.at(1, Ctl::Aux));
        rig.frame(vec![]);
    };
    open(&mut rig);
    rig.click(rig.text_at("Effect 4"));
    assert_eq!(rig.actions, [DrumsAction::SendTarget(1)]);
    assert_eq!(rig.kit.slots[1].file.send_target, 4);
    rig.frame(vec![]);
    assert_eq!(rig.count("Effect 4"), 0, "closed");
    open(&mut rig);
    rig.click(rig.text_at("Default"));
    assert_eq!(rig.actions, [DrumsAction::SendTarget(1)]);
    assert_eq!(rig.kit.slots[1].file.send_target, -1);
    // `Create FX Channel` is the host's: a new send channel the pad then targets.
    rig.frame(vec![]);
    open(&mut rig);
    rig.click(rig.text_at("Create FX Channel"));
    assert_eq!(rig.actions, [DrumsAction::CreateFxChannel(1)]);
    assert_eq!(rig.kit.slots[1].file.send_target, -1);
}

#[test]
fn create_fx_channel_targets_a_new_effect_channel_from_the_pad() {
    let mut song = crate::song::Song::default();
    song.add_channel(crate::song::ChannelKind::Drums);
    song.add_channel(crate::song::ChannelKind::EffectTrack);
    assert_eq!(send_channels(&song), [(2, "Effect 2".to_owned())]);
    assert_eq!(create_pad_send(&mut song, 1, 3), Some(3));
    let new = &song.channels[3];
    assert_eq!(
        (new.kind, new.name.as_str()),
        (crate::song::ChannelKind::EffectTrack, "Effect 3")
    );
    let kit = song.channels[1].drum_kit.as_ref().unwrap();
    assert_eq!(kit.slots[3].file.send_target, 3);
    // No such slot: no channel.
    assert_eq!(create_pad_send(&mut song, 1, 99), None);
    assert_eq!(song.channels.len(), 4);
}

#[test]
fn the_pads_draw_no_background_and_rings_go_under_every_body() {
    // The keyboard panel's `controlFill` shows behind them: the view itself draws hexes and text only.
    let mut rig = Rig::new(View::Pads);
    // (The rig's egui panel fills the screen under them.)
    let own = |s: &Shape| match s {
        Shape::Rect(r) => r.rect != SCREEN,
        _ => !matches!(s, Shape::Path(_) | Shape::Text(_)),
    };
    assert!(!rig.shapes.iter().any(|c| own(&c.shape)), "a background");
    // Draw order: the level glows, the drop highlights, every body, then the dragged pad.
    let g = grid();
    carry(&mut rig, g.centres[20]);
    let all = rig.all_hexes();
    assert!(all[..22].iter().all(|h| glow(h.2)), "{all:?}");
    let hexes = rig.hexes_at();
    let ring = hexes.iter().position(|h| near(h.2, visible(g.r + 2.0)));
    assert_eq!(ring, Some(0), "{hexes:?}");
    assert_eq!((all.len(), hexes.len()), (22 + 1 + 22 + 1, 1 + 22 + 1));
}
