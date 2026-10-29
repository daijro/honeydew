//! The header column and its expanded mini mixer.

use eframe::egui::epaint::ClippedShape;
use eframe::egui::{Color32, Event, Modifiers, PointerButton, Pos2, Rect, Shape, pos2, vec2};
use honeydew_audio::MeterSnapshot;
use honeydew_audio::device::{ModuleKey, ModuleMeter};

use super::*;
use crate::metrics::{DEFAULT_UI_SCALE, Metrics};
use crate::song::SynthKind;
use crate::theme::theme_by_id;
use crate::ui::main_screen::track_rows;
use crate::ui::offscreen::Offscreen;

/// U at ppp 1, t = ceil(32/16) = 2, and the default 2 U row.
const U: f32 = 32.0;
const T: f32 = 2.0;
const ROW_H: f32 = 64.0;

fn r(x0: f32, y0: f32, x1: f32, y1: f32) -> Rect {
    Rect::from_min_max(pos2(x0, y0), pos2(x1, y1))
}

fn m(ppp: f32) -> Metrics {
    Metrics::from_pixels_per_point(ppp, DEFAULT_UI_SCALE)
}

fn ids(placed: &[(NodeId, Rect)]) -> Vec<NodeId> {
    placed.iter().map(|p| p.0).collect()
}

/// The x ranges of the parts, in U.
fn xs_u(placed: &[(NodeId, Rect)]) -> Vec<(NodeId, f32, f32)> {
    (placed.iter())
        .map(|&(id, r)| (id, r.left() / U, r.right() / U))
        .collect()
}

// ---- the stack ----

#[test]
fn at_c_1_the_stack_is_header_mute_solo_pan_and_level_at_their_u() {
    // A 13.5 U row, 2 U less the 1 px gap: header 0–1.5, mute 1.5–3, `Solo` 3–4.5, pan 4.5–7.25, level 7.25–13.25.
    // The same in points at ppp 1 and 2 (U = 32 points at both).
    for ppp in [1.0, 2.0] {
        let row = r(0.0, 0.0, 13.5 * U, ROW_H - 1.0 / ppp);
        let placed = parts(RowKind::Primary, row, true, &m(ppp));
        assert_eq!(
            xs_u(&placed),
            [
                (HEADER, 0.0, 1.5),
                (MUTE, 1.5, 3.0),
                (SOLO, 3.0, 4.5),
                (PAN, 4.5, 7.25),
                (LEVEL, 7.25, 13.25)
            ],
            "ppp {ppp}"
        );
        // Every part is the full row height less the gap.
        assert!(
            placed.iter().all(|p| p.1.y_range() == row.y_range()),
            "ppp {ppp}"
        );
    }
}

#[test]
fn the_masters_unsolo_all_takes_mute_and_solo_so_its_pan_and_level_line_up() {
    let row = r(0.0, 0.0, 13.5 * U, 63.0);
    let placed = parts(RowKind::Master, row, true, &m(1.0));
    assert_eq!(
        xs_u(&placed),
        [
            (HEADER, 0.0, 1.5),
            (UNSOLO, 1.5, 4.5),
            (PAN, 4.5, 7.25),
            (LEVEL, 7.25, 13.25)
        ]
    );
    assert_eq!(
        header_rect(RowKind::Master, row, true, &m(1.0)),
        r(0.0, 0.0, 1.5 * U, 63.0)
    );
    assert_eq!(
        header_rect(RowKind::Primary, row, true, &m(1.0)),
        r(0.0, 0.0, 1.5 * U, 63.0)
    );
}

#[test]
fn a_sub_track_row_is_a_4_5_u_header_and_an_8_75_u_slider() {
    let row = r(0.0, 64.0, 13.5 * U, 127.0);
    let placed = parts(RowKind::Sub, row, true, &m(2.0));
    assert_eq!(xs_u(&placed), [(HEADER, 0.0, 4.5), (SLIDER, 4.5, 13.25)]);
    assert_eq!(
        header_rect(RowKind::Sub, row, true, &m(1.0)).width(),
        4.5 * U
    );
}

#[test]
fn collapsed_or_animating_the_header_is_the_whole_row_and_nothing_else_takes_space() {
    for kind in [RowKind::Primary, RowKind::Master, RowKind::Sub] {
        // c = 0.5: 7.5 U.
        let row = r(0.0, 0.0, 7.5 * U, 63.0);
        let placed = parts(kind, row, false, &m(1.0));
        assert_eq!(ids(&placed), [HEADER]);
        assert_eq!(header_rect(kind, row, false, &m(1.0)), row);
    }
}

#[test]
fn the_meter_law() {
    let near = |a: f32, b: f32| (a - b).abs() < 1e-4;
    // 0 dB: (60/66)^1.6667.
    assert!(near(meter(1.0), (60.0f32 / 66.0).powf(1.6667)));
    // +6 dB reaches the top; louder is clamped.
    assert!(near(meter(10f32.powf(6.0 / 20.0)), 1.0));
    assert_eq!(meter(4.0), 1.0);
    // −60 dB is the bottom, and 0.001 or less is 0.
    assert!(meter(0.0011) > 0.0 && meter(0.0011) < 1e-3);
    assert_eq!(meter(0.001), 0.0);
    assert_eq!(meter(0.0), 0.0);
    assert_eq!(meter(f32::NAN), 0.0);
}

// ---- drawn ----

/// The master (0), a MiniSynth (1) with an automation track on its Pan (channel scope, lane 1) and one on its module's
/// param 1 (lane 2), and a Drums channel (2).
fn song() -> Song {
    let mut song = Song::default();
    song.add_channel(ChannelKind::Synth(SynthKind::MiniSynth));
    song.add_channel(ChannelKind::Drums);
    let module = module(&song);
    let ch = &mut song.channels[1];
    ch.add_automation_lane(module);
    ch.add_automation_lane(AutomationTarget {
        instance_id: 0,
        param: 1,
    });
    song
}

/// The MiniSynth module's param 1.
fn module(song: &Song) -> AutomationTarget {
    AutomationTarget {
        instance_id: song.channels[1].rack.modules[0].instance_id,
        param: 1,
    }
}

struct Rig {
    screen: Offscreen,
    song: Song,
    expanded: bool,
    current: (usize, usize),
    press: f32,
    multitrack: bool,
    meters: Vec<ModuleMeter>,
    /// Every change since the rig was made, and the last touch.
    changes: Vec<(usize, AutomationTarget, f32)>,
    touch: Option<(usize, Touch)>,
    shapes: Vec<ClippedShape>,
}

impl Rig {
    fn new(expanded: bool) -> Self {
        let mut rig = Self {
            screen: Offscreen::new(),
            song: song(),
            expanded,
            current: (1, 0),
            press: 1.0,
            multitrack: false,
            meters: Vec::new(),
            changes: Vec::new(),
            touch: None,
            shapes: Vec::new(),
        };
        rig.frame(vec![]);
        rig
    }

    fn width(&self) -> f32 {
        if self.expanded { 13.5 * U } else { 1.5 * U }
    }

    fn frame(&mut self, events: Vec<Event>) {
        let rows = track_rows(&self.song, false);
        let shown: Vec<(TrackRow, f32)> = (rows.iter().enumerate())
            .map(|(i, &row)| (row, i as f32 * ROW_H))
            .collect();
        let col = Column {
            left: 0.0,
            width: self.width(),
            expanded: self.expanded,
            current: self.current,
            press: self.press,
            multitrack: self.multitrack,
            meters: Some(&self.meters),
        };
        let clip = r(0.0, 0.0, col.width, 540.0);
        let (song, mut out) = (&mut self.song, Output::default());
        self.shapes = self.screen.frame(events, &mut |ui, cx| {
            out = show(ui, cx, song, &shown, ROW_H, &col, clip);
        });
        self.changes.extend(out.changes);
        self.touch = out.touch.or(self.touch);
    }

    fn button(&mut self, pos: Pos2, pressed: bool) {
        self.frame(vec![
            Event::PointerMoved(pos),
            Event::PointerButton {
                pos,
                button: PointerButton::Primary,
                pressed,
                modifiers: Modifiers::NONE,
            },
        ]);
    }

    fn tap(&mut self, pos: Pos2) {
        self.frame(vec![Event::PointerMoved(pos)]);
        self.button(pos, true);
        self.button(pos, false);
    }

    fn drag(&mut self, from: Pos2, to: Pos2) {
        self.frame(vec![Event::PointerMoved(from)]);
        self.button(from, true);
        for k in 1..=4 {
            self.frame(vec![Event::PointerMoved(
                from + (to - from) * k as f32 / 4.0,
            )]);
        }
        self.button(to, false);
    }

    /// The filled rects of the last frame, in drawing order.
    fn rects(&self) -> Vec<(Rect, Color32)> {
        (self.shapes.iter())
            .filter_map(|s| match &s.shape {
                Shape::Rect(r) => Some((r.rect, r.fill)),
                _ => None,
            })
            .collect()
    }

    fn circles(&self) -> Vec<(Pos2, f32, Color32)> {
        (self.shapes.iter())
            .filter_map(|s| match &s.shape {
                Shape::Circle(c) => Some((c.center, c.radius, c.fill)),
                _ => None,
            })
            .collect()
    }

    /// The texts of the last frame: text, colour, font size, galley rect.
    fn texts(&self) -> Vec<(String, Color32, f32, Rect)> {
        (self.shapes.iter())
            .filter_map(|s| match &s.shape {
                Shape::Text(t) => {
                    let format = &t.galley.job.sections[0].format;
                    let rect = t.galley.rect.translate(t.pos.to_vec2());
                    Some((
                        t.galley.text().to_owned(),
                        format.color,
                        format.font_id.size,
                        rect,
                    ))
                }
                _ => None,
            })
            .collect()
    }

    /// The convex polygons' points and fills.
    fn triangles(&self) -> Vec<(Vec<Pos2>, Color32)> {
        (self.shapes.iter())
            .filter_map(|s| match &s.shape {
                Shape::Path(p) if p.points.len() == 3 => Some((p.points.clone(), p.fill)),
                _ => None,
            })
            .collect()
    }

    fn ink(&self, channel: usize) -> Color32 {
        let theme = theme_by_id(0);
        channel_color(self.song.channels[channel].hue, &theme).to_color32()
    }
}

fn colors() -> crate::theme::ThemeColors {
    theme_by_id(0).colors
}

/// Row `i`'s rect less its gap, in a column `w` wide.
fn row(i: usize, w: f32) -> Rect {
    r(0.0, i as f32 * ROW_H, w, (i + 1) as f32 * ROW_H - 1.0)
}

#[test]
fn rows_are_module_fill_and_sub_tracks_control_fill_never_the_channel_colour() {
    // Rows: master, the synth, its two automation tracks, the drums.
    for expanded in [false, true] {
        let rig = Rig::new(expanded);
        let w = rig.width();
        let c = colors();
        let rects = rig.rects();
        for (i, fill) in [
            (0, c.module_fill),
            (1, c.module_fill),
            (2, c.control_fill),
            (3, c.control_fill),
            (4, c.module_fill),
        ] {
            assert!(
                rects.contains(&(row(i, w), fill.to_color32())),
                "row {i}, expanded {expanded}"
            );
        }
        // No row is filled with its channel colour (only the current synth's cell, all of its row while collapsed).
        assert!(!(rects.iter()).any(|&(r, fill)| r.width() == w && fill == rig.ink(2)));
        let synth = (rects.iter()).filter(|&&(_, fill)| fill == rig.ink(1) && w > 1.5 * U);
        assert!(
            synth.clone().all(|&(r, _)| r.width() < w),
            "expanded {expanded}"
        );
    }
}

#[test]
fn only_the_current_channels_cell_is_filled_with_its_colour_and_its_icon_drawn_in_module_fill() {
    for expanded in [false, true] {
        let rig = Rig::new(expanded);
        let cell = |i: usize| row(i, 1.5 * U);
        // The synth (row 1) is current: its 1.5 U cell at full row height.
        assert!(
            rig.rects().contains(&(cell(1), rig.ink(1))),
            "expanded {expanded}"
        );
        assert!(!(rig.rects().iter()).any(|&(r, f)| r == cell(4) && f == rig.ink(2)));
        let icon_fills: Vec<Color32> = (rig.shapes.iter())
            .filter_map(|s| match &s.shape {
                Shape::Path(p) if p.points.len() > 3 && cell(1).contains(p.points[0]) => {
                    Some(p.fill)
                }
                Shape::Rect(rr)
                    if rr.rect.width() < 1.5 * U && cell(1).contains(rr.rect.center()) =>
                {
                    Some(if rr.fill == Color32::TRANSPARENT {
                        rr.stroke.color
                    } else {
                        rr.fill
                    })
                }
                _ => None,
            })
            .collect();
        assert!(
            !icon_fills.is_empty()
                && icon_fills
                    .iter()
                    .all(|&f| f == colors().module_fill.to_color32())
        );
        // The drums' icon is in its channel colour, on the row.
        let drums_icon = (rig.shapes.iter()).any(|s| match &s.shape {
            Shape::Rect(rr) => {
                cell(4).contains(rr.rect.center())
                    && rr.fill == rig.ink(2)
                    && rr.rect.width() < 1.5 * U
            }
            _ => false,
        });
        assert!(drums_icon, "expanded {expanded}");
    }
}

#[test]
fn the_current_channels_rows_are_drawn_last() {
    let mut rig = Rig::new(false);
    let order = |rig: &Rig| -> Vec<usize> {
        let mut rows: Vec<usize> = (rig.rects().iter())
            .filter_map(|&(rr, _)| (0..5).find(|&i| rr == row(i, 1.5 * U)))
            .collect();
        // The current cell's fill is its whole collapsed row.
        rows.dedup();
        rows
    };
    assert_eq!(order(&rig), [0, 4, 1, 2, 3]);
    rig.current = (2, 0);
    rig.frame(vec![]);
    assert_eq!(order(&rig), [0, 1, 2, 3, 4]);
    rig.current = (0, 0);
    rig.frame(vec![]);
    assert_eq!(order(&rig), [1, 2, 3, 4, 0]);
}

#[test]
fn the_current_cell_grows_vertically_only_by_its_press_and_the_icon_stays() {
    let mut rig = Rig::new(false);
    // 1 + 10/h at h = 63 points: 10 px more, centred.
    rig.press = 1.0 + 10.0 / 63.0;
    rig.frame(vec![]);
    let cell = row(1, 1.5 * U);
    let grown = Rect::from_center_size(cell.center(), vec2(cell.width(), 73.0));
    let fill = rig
        .rects()
        .into_iter()
        .find(|&(_, f)| f == rig.ink(1))
        .unwrap()
        .0;
    assert!(
        (fill.height() - 73.0).abs() < 1e-3 && fill.x_range() == grown.x_range(),
        "{fill:?}"
    );
    assert!((fill.center().y - cell.center().y).abs() < 1e-3);
}

#[test]
fn controls_and_labels_show_only_at_c_1() {
    let rig = Rig::new(false);
    assert!(rig.texts().is_empty());
    assert!(rig.circles().is_empty(), "no slider thumbs");
    let rig = Rig::new(true);
    let texts: Vec<String> = rig.texts().into_iter().map(|t| t.0).collect();
    // Master: no `Solo`. The synth, the drums: `Solo` each.
    assert_eq!(texts.iter().filter(|t| *t == "Solo").count(), 2);
    assert!(texts.contains(&"MASTER".to_owned()));
}

#[test]
fn the_name_is_8_pt_right_and_top_in_the_channel_colour_with_the_kinds_default() {
    let mut rig = Rig::new(true);
    rig.song.channels[2].name = String::new();
    rig.song.add_channel(ChannelKind::EffectTrack);
    rig.song.channels[3].name = String::new();
    rig.frame(vec![]);
    let texts = rig.texts();
    let find = |label: &str| texts.iter().find(|t| t.0 == label).cloned().expect(label);
    // 8 pt at U = 32: floor(32·8/16) = 16 px.
    let (_, color, size, rect) = find("MiniSynth");
    assert_eq!((color, size), (rig.ink(1), 16.0));
    // Right-aligned 0.5 U from the row's right edge, its top t below the row's.
    assert!(
        (rect.right() - (13.5 * U - 0.5 * U)).abs() < 1.0,
        "{rect:?}"
    );
    assert!((rect.top() - (ROW_H + T)).abs() < 1e-3, "{rect:?}");
    find("MASTER");
    assert_eq!(find("CHANNEL").1, rig.ink(2));
    find("AUX");
    // A sub-track row shows its track's name.
    let label = rig.song.channels[1].lanes[1].label.clone();
    assert!(!label.is_empty());
    assert_eq!(find(&label).3.top(), 2.0 * ROW_H + T);
}

#[test]
fn a_long_name_is_cut_in_the_middle_keeping_five_characters() {
    let mut rig = Rig::new(true);
    rig.song.channels[1].name =
        "A very long channel name that cannot fit in its row, not even at eight points".into();
    rig.frame(vec![]);
    let texts = rig.texts();
    let cut = texts
        .iter()
        .find(|t| t.0.contains("..."))
        .expect("a truncated name");
    assert!(cut.0.ends_with("oints"), "{}", cut.0);
    assert!(cut.3.width() <= 12.5 * U, "{:?}", cut.3);
}

/// A channel's part `id` at c = 1 (row `i`).
fn part(kind: RowKind, i: usize, id: NodeId) -> Rect {
    (parts(kind, row(i, 13.5 * U), true, &m(1.0)).into_iter())
        .find(|p| p.0 == id)
        .unwrap()
        .1
}

#[test]
fn mute_is_the_inverted_pill_and_a_tap_toggles_param_2() {
    let mut rig = Rig::new(true);
    let mute = part(RowKind::Primary, 1, MUTE);
    let dot = |rig: &Rig| {
        (rig.circles().into_iter())
            .find(|c| mute.contains(c.0) && c.1 == 2.0 * T)
            .map(|c| (c.0.y - mute.center().y, c.2))
            .unwrap()
    };
    // Mute 0, audible: the dot up (−4t) in the channel colour.
    assert_eq!(dot(&rig), (-4.0 * T, rig.ink(1)));
    rig.tap(mute.center());
    let target = AutomationTarget {
        instance_id: 0,
        param: 2,
    };
    assert_eq!(rig.changes, [(1, target, 1.0)]);
    assert_eq!(rig.song.channels[1].params.0[2], 1.0);
    // Muted: down (+4t) in `menuFill`.
    assert_eq!(dot(&rig), (4.0 * T, colors().menu_fill.to_color32()));
    // The touch goes to the app's recent control.
    assert_eq!(rig.touch.map(|t| (t.0, t.1.target)), Some((1, target)));
}

#[test]
fn the_invisible_disc_behind_the_pill_shows_while_pressed() {
    let mut rig = Rig::new(true);
    let mute = part(RowKind::Primary, 1, MUTE);
    let disc = |rig: &Rig| {
        (rig.circles().into_iter())
            .find(|c| c.0 == mute.center() && c.1 > 2.0 * T)
            .map(|c| (c.1, c.2))
            .unwrap()
    };
    let r0 = 0.4 * mute.width().min(mute.height());
    assert_eq!(disc(&rig), (r0, colors().module_fill.to_color32()));
    rig.frame(vec![Event::PointerMoved(mute.center())]);
    rig.button(mute.center(), true);
    rig.frame(vec![]);
    let pressed = widgets::pressed(colors().module_fill.to_color32());
    assert_eq!(disc(&rig), (r0 * 1.05, pressed));
}

#[test]
fn solo_is_a_text_disc_with_module_fill_text_while_on_and_a_tap_toggles_param_3() {
    let mut rig = Rig::new(true);
    let solo = part(RowKind::Primary, 1, SOLO);
    let (text, module_fill) = (
        colors().text.to_color32(),
        colors().module_fill.to_color32(),
    );
    let look = |rig: &Rig| {
        let disc = (rig.circles().into_iter())
            .find(|c| c.0 == solo.center())
            .unwrap();
        let label = (rig.texts().into_iter())
            .find(|t| t.0 == "Solo" && solo.contains(t.3.center()))
            .unwrap();
        (disc.1, disc.2, label.1, label.2)
    };
    let r = 0.4 * solo.width().min(solo.height());
    // Off: an invisible `moduleFill` disc under `text`; 8 pt (16 px).
    assert_eq!(look(&rig), (r, module_fill, text, 16.0));
    rig.tap(solo.center());
    assert_eq!(rig.song.channels[1].params.0[3], 1.0);
    // Off the button (a hovered one is lit).
    rig.frame(vec![Event::PointerMoved(pos2(1000.0, 500.0))]);
    assert_eq!(look(&rig), (r, text, module_fill, 16.0));
    // Not exclusive: the drums' Solo goes on too, the synth's stays.
    rig.tap(part(RowKind::Primary, 4, SOLO).center());
    assert_eq!(
        (
            rig.song.channels[1].params.0[3],
            rig.song.channels[2].params.0[3]
        ),
        (1.0, 1.0)
    );
    rig.tap(solo.center());
    assert_eq!(rig.song.channels[1].params.0[3], 0.0);
    let solo3 = AutomationTarget {
        instance_id: 0,
        param: 3,
    };
    assert_eq!(
        rig.changes,
        [(1, solo3, 1.0), (2, solo3, 1.0), (1, solo3, 0.0)]
    );
}

#[test]
fn pan_and_level_follow_the_finger_one_to_one_and_a_double_tap_resets_them() {
    let mut rig = Rig::new(true);
    let level = part(RowKind::Primary, 1, LEVEL);
    let pan = part(RowKind::Master, 0, PAN);
    // The level track runs 0.5 U in from both ends: a quarter of it is +0.25.
    let track = level.width() - U;
    let v0 = rig.song.channels[1].params.0[0];
    let from = pos2(level.left() + 0.5 * U + v0 * track, level.center().y);
    rig.drag(from, from + vec2(0.25 * track, 0.0));
    let v = rig.song.channels[1].params.0[0];
    assert!((v - (v0 + 0.25).min(1.0)).abs() < 1e-3, "{v0} → {v}");
    assert!(rig.changes.iter().all(|c| c.1.param == 0 && c.0 == 1));
    // A double tap: the Level reset, 0.85312.
    rig.tap(from);
    rig.tap(from);
    assert_eq!(rig.song.channels[1].params.0[0], widgets::UNITY);
    // The master's pan: dragged, then reset to the middle.
    let mid = pan.center();
    rig.drag(mid, mid - vec2(0.25 * (pan.width() - U), 0.0));
    assert!((rig.song.channels[0].params.0[1] - 0.25).abs() < 1e-3);
    rig.tap(pos2(pan.left() + 0.5 * U + 0.25 * (pan.width() - U), mid.y));
    rig.tap(pos2(pan.left() + 0.5 * U + 0.25 * (pan.width() - U), mid.y));
    assert_eq!(rig.song.channels[0].params.0[1], 0.5);
}

#[test]
fn the_sliders_are_3t_tracks_whose_thumbs_grow_from_4_5t_to_5_985t() {
    let mut rig = Rig::new(true);
    rig.song.channels[1].params.0[1] = 1.0;
    rig.song.channels[1].params.0[0] = 0.0;
    rig.frame(vec![]);
    let (pan, level) = (
        part(RowKind::Primary, 1, PAN),
        part(RowKind::Primary, 1, LEVEL),
    );
    let control_fill = colors().control_fill.to_color32();
    let thumb = |at: Rect| {
        (rig.circles().into_iter())
            .filter(|c| at.contains(c.0) && c.2 == control_fill)
            .map(|c| c.1)
            .next()
            .unwrap()
    };
    // Pan at the far right of a bipolar range: 5.985 t. Level at 0: 4.5 t.
    assert!((thumb(pan) - 5.985 * T).abs() < 1e-3);
    assert!((thumb(level) - 4.5 * T).abs() < 1e-3);
    // The tracks: 3t tall, centred, 0.5 U in.
    let tracks: Vec<Rect> = (rig.rects().into_iter())
        .filter(|&(rr, f)| {
            f == control_fill && rr.height() == 3.0 * T && level.contains(rr.center())
        })
        .map(|p| p.0)
        .collect();
    assert_eq!(
        tracks,
        [r(
            level.left() + 0.5 * U,
            level.center().y - 3.0,
            level.right() - 0.5 * U,
            level.center().y + 3.0
        )]
    );
}

fn peak(channel: usize, left: f32, right: f32) -> ModuleMeter {
    ModuleMeter {
        key: ModuleKey {
            channel,
            instance_id: 0,
        },
        value: MeterSnapshot {
            peak_left: left,
            peak_right: right,
            ..MeterSnapshot::default()
        },
        analysis: Vec::new(),
    }
}

#[test]
fn expanded_meters_are_two_bars_inside_the_level_slider() {
    let mut rig = Rig::new(true);
    rig.meters = vec![peak(2, 1.0, 0.5)];
    rig.frame(vec![]);
    let level = part(RowKind::Primary, 4, LEVEL);
    let ink = rig.ink(2);
    let mut bars: Vec<Rect> = (rig.rects().into_iter())
        .filter(|&(rr, f)| {
            f == ink
                && level.contains(rr.center())
                && rr.height() < 0.5 * U
                && rr.height() != 3.0 * T
        })
        .map(|p| p.0)
        .collect();
    bars.sort_by(|a, b| b.bottom().total_cmp(&a.bottom()));
    let h = level.height() / 8.0;
    let lower_bottom = level.bottom() - 0.25 * U;
    let len = level.width() - U;
    let near =
        |a: Rect, b: Rect| (a.min - b.min).length() < 1e-3 && (a.max - b.max).length() < 1e-3;
    let want = |bottom: f32, v: f32| {
        r(
            level.left() + 0.5 * U,
            bottom - h,
            level.left() + 0.5 * U + len * meter(v),
            bottom,
        )
    };
    assert_eq!(bars.len(), 2, "{bars:?}");
    assert!(near(bars[0], want(lower_bottom, 1.0)), "{:?}", bars[0]);
    assert!(
        near(bars[1], want(lower_bottom - h - 1.0, 0.5)),
        "{:?}",
        bars[1]
    );
}

#[test]
fn collapsed_meters_are_two_t_wide_bars_at_the_cells_right_edge() {
    let mut rig = Rig::new(false);
    rig.meters = vec![peak(1, 1.0, 1.0), peak(2, 1.0, 0.0)];
    rig.frame(vec![]);
    let module_fill = colors().module_fill.to_color32();
    let bars = |rig: &Rig, i: usize, color: Color32| -> Vec<Rect> {
        let cell = row(i, 1.5 * U);
        (rig.rects().into_iter())
            .filter(|&(rr, f)| f == color && rr.width() == T && cell.contains(rr.center()))
            .map(|p| p.0)
            .collect()
    };
    // The current synth's in `moduleFill` on its filled cell, x = right − 3t and right − 2t, from the bottom − 8t.
    let cell = row(1, 1.5 * U);
    let span = cell.height() - 16.0 * T;
    let top = cell.bottom() - 8.0 * T - span * meter(1.0);
    assert_eq!(
        bars(&rig, 1, module_fill),
        [
            r(48.0 - 3.0 * T, top, 48.0 - 2.0 * T, cell.bottom() - 8.0 * T),
            r(48.0 - 2.0 * T, top, 48.0 - T, cell.bottom() - 8.0 * T)
        ]
    );
    // The drums' in their colour; the silent right one is not drawn.
    assert_eq!(bars(&rig, 4, rig.ink(2)).len(), 1);
    // Expanded, none in the cells.
    rig.expanded = true;
    rig.frame(vec![]);
    assert!(bars(&rig, 1, module_fill).is_empty());
}

#[test]
fn hidden_sub_tracks_show_a_down_triangle_in_the_cell() {
    let mut rig = Rig::new(false);
    assert!(rig.triangles().is_empty());
    rig.song.channels[1].zipped = true;
    rig.frame(vec![]);
    // Rows now: master, synth, drums. The synth is current: `moduleFill`.
    let cell = row(1, 1.5 * U);
    let (cx, b) = (cell.center().x, cell.bottom());
    assert_eq!(
        rig.triangles(),
        [(
            vec![
                pos2(cx - 2.0 * T, b - 4.0 * T),
                pos2(cx + 2.0 * T, b - 4.0 * T),
                pos2(cx, b - 2.0 * T)
            ],
            colors().module_fill.to_color32()
        )]
    );
}

#[test]
fn the_current_sub_track_has_a_menu_fill_triangle_pointing_right() {
    let mut rig = Rig::new(false);
    rig.current = (1, 2);
    rig.frame(vec![]);
    // Row 3 is lane 2 of the synth.
    let cy = row(3, 1.5 * U).center().y;
    assert_eq!(
        rig.triangles(),
        [(
            vec![
                pos2(0.0, cy - 0.4 * U),
                pos2(0.4 * U, cy),
                pos2(0.0, cy + 0.4 * U)
            ],
            colors().menu_fill.to_color32()
        )]
    );
}

#[test]
fn a_module_param_track_shows_a_menu_fill_slider_bound_to_the_param() {
    let mut rig = Rig::new(true);
    let menu_fill = colors().menu_fill.to_color32();
    // Row 2 is the Pan lane (channel scope: no slider), row 3 the module lane.
    let target = module(&rig.song);
    let (pan, module) = (part(RowKind::Sub, 2, SLIDER), part(RowKind::Sub, 3, SLIDER));
    let in_rect = |at: Rect| {
        (rig.circles().into_iter())
            .filter(|c| at.contains(c.0))
            .count()
    };
    assert_eq!(in_rect(pan), 0);
    assert_eq!(in_rect(module), 2);
    let track = (rig.rects().into_iter())
        .find(|&(rr, _)| rr.height() == 3.0 * T && module.contains(rr.center()));
    assert_eq!(track.map(|t| t.1), Some(menu_fill));
    let rim = (rig.circles().into_iter())
        .find(|c| module.contains(c.0))
        .map(|c| c.2);
    assert_eq!(rim, Some(menu_fill));
    // It shows the live value and sets it.
    let v0 = rig.song.channels[1].param(target).unwrap();
    let len = module.width() - U;
    let from = pos2(module.left() + 0.5 * U + v0 * len, module.center().y);
    let to = from + vec2(0.1 * len, 0.0);
    rig.drag(from, to);
    let v = rig.song.channels[1].param(target).unwrap();
    assert!((v - (v0 + 0.1).min(1.0)).abs() < 1e-3, "{v0} → {v}");
    assert_eq!(rig.changes.last(), Some(&(1, target, v)));
    // A double tap: the param's reset value.
    let spec = crate::app::auto_record::spec(&rig.song.channels[1], target).unwrap();
    let at = pos2(module.left() + 0.5 * U + v * len, module.center().y);
    rig.tap(at);
    rig.tap(at);
    assert_eq!(rig.song.channels[1].param(target), Some(spec.reset));
}

#[test]
fn an_armed_channel_keeps_its_dot_in_the_icons_ink() {
    let mut rig = Rig::new(false);
    rig.song.add_channel(ChannelKind::AudioRecord);
    rig.song.channels[3].file.record_armed = true;
    rig.multitrack = true;
    rig.frame(vec![]);
    let cell = row(5, 1.5 * U);
    let dot = (rig.circles().into_iter())
        .find(|c| (c.1 - 0.1 * U).abs() < 1e-3)
        .unwrap();
    assert_eq!(
        (dot.0, dot.2),
        (cell.min + vec2(0.3 * U, 0.3 * U), rig.ink(3))
    );
}

#[test]
fn a_tremolo_max_tracks_slider_below_min_lowers_min_in_the_song() {
    // Module 46 (Tremolo): Max below Min lowers Min to it, as the effect does with the audio's copy. Row 3's lane
    // automates a Tremolo's Max (p3) instead.
    let mut rig = Rig::new(true);
    let old = module(&rig.song);
    let ch = &mut rig.song.channels[1];
    let id = ch.rack.add(46);
    let tremolo = ch.rack.get_mut(id).unwrap();
    (tremolo.params[3], tremolo.params[4]) = (0.8, 0.5);
    let max = AutomationTarget {
        instance_id: id,
        param: 3,
    };
    let lane = ch.lanes.iter_mut().find(|l| l.target == Some(old)).unwrap();
    lane.target = Some(max);
    rig.frame(vec![]);
    let slider = part(RowKind::Sub, 3, SLIDER);
    let len = slider.width() - U;
    let from = pos2(slider.left() + 0.5 * U + 0.8 * len, slider.center().y);
    rig.drag(from, from - vec2(0.5 * len, 0.0));
    let params = &rig.song.channels[1].rack.get(id).unwrap().params;
    assert!((params[3] - 0.3).abs() < 1e-3, "{}", params[3]);
    assert_eq!(params[4], params[3], "Min follows Max down");
}
