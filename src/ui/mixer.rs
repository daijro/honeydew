//! The mixer view: one 4 U strip per channel, master first, in a row that scrolls sideways.
//!
//! Gestures: the fader and the pan knob take the slider and knob laws ([`widgets`]), Mute and `Solo` the two-state tap;
//! soloing never clears another strip. A tap anywhere else on a strip makes its channel current
//! ([`MixerState::selected`]). A touch that starts off the controls (or on a toggle, which takes no drags) scrolls the
//! row by the editors' [`scroller`] law: still within 0.5 U of the touch-down, then 1:1, a fifth past either end, a
//! fling on release and a spring back from past an end; a short one that stays calm is a strip tap. The wheel steps a
//! strip per half strip of travel.
//!
//! A strip edits its channel's `RPRM` params in the song (0 Level, 1 Pan, 2 Mute, 3 Solo) and [`show`] returns each
//! edit as `(channel, param index, value)`. The app sends it on as
//! `SetParam { channel, instance_id: 0, index, value }`: instance 0 is the channel strip.
//! A strip control's press or parameter menu pick is the app's recent control ([`MixerState::touch`]); its menu's
//! `Unlink from MIDI CC` follows the channel's `CCLK`.

use eframe::egui::{Align2, Pos2, Rect, Ui, UiBuilder, Vec2, pos2, vec2};

use crate::icons::draw_icon;
use crate::metrics::FontSize;
use crate::midi::links;
use crate::song::{AutomationTarget, ChannelKind, Song};
use crate::theme::channel_color;
use crate::ui::UiCtx;
use crate::ui::ctrl_menu::Touch;
use crate::ui::fine_tune;
use crate::ui::layout::{Node, NodeId, Size, layout};
use crate::ui::main_screen::kind_icon;
use crate::ui::scroller::{self, Axis, Scroller, X, Zones};
use crate::ui::widgets::{self, Control, ParamMenu, Pill};

/// Strip width in U, which is also the scroll unit (the wheel's step).
pub const STRIP_W: f32 = 4.0;

/// The leaves of a strip. The four controls' ids are their channel param indices.
const FADER: NodeId = 0;
const KNOB: NodeId = 1;
const MUTE: NodeId = 2;
const SOLO: NodeId = 3;
/// The bottom cell: the channel icon, filled with the channel colour when the strip is selected.
const CELL: NodeId = 4;
const SPACER: NodeId = 5;
/// The name slot: the top 0.75 U, the strip's width, shown at every height.
const NAME: NodeId = 6;

#[derive(Clone, Debug, Default, PartialEq)]
pub struct MixerState {
    /// How far the row is scrolled, in points: 0 ..= the last strip at the right edge, or past either end while a
    /// touch pulls it there and its spring brings it back.
    pub scroll: f32,
    /// The channel whose bottom cell is filled. A strip tap off its controls selects its channel.
    pub selected: Option<usize>,
    /// Wheel travel not yet turned into a step, in points: less than half a strip.
    pub wheel: f32,
    /// The pointer is over an overlay: the wheel is not the mixer's. The host sets it each frame.
    pub covered: bool,
    /// A strip control's touch (instance 0, its param), with the strip's channel: the host takes it.
    pub touch: Option<(usize, Touch)>,
    /// The row's scroll controller: the touch it has, its fling and spring.
    scroller: Scroller,
}

/// A strip `h` U tall: the 0.75 U name slot, fader (60 % wide, flex), pan knob (80 % wide, 2 U), the mute/solo row
/// (1.75 U), the bottom cell (1.5 U), each hidden at or below its height threshold; a flex spacer stands in for the
/// fader. The master keeps the mute/solo row's space, empty, so that its fader and knob line up with the others'.
fn strip_tree(h: f32, master: bool) -> Node {
    Node::vstack([
        Node::leaf(NAME).h(Size::U(0.75)),
        Node::leaf(FADER).w(Size::Percent(60.0)).visible(h > 4.0),
        Node::leaf(SPACER).visible(h <= 4.0),
        Node::leaf(KNOB)
            .w(Size::Percent(80.0))
            .h(Size::U(2.0))
            .visible(h > 7.5),
        Node::hstack([
            Node::leaf(MUTE).visible(!master),
            Node::leaf(SOLO).visible(!master),
        ])
        .h(Size::U(1.75))
        .visible(h > 10.5),
        Node::leaf(CELL).h(Size::U(1.5)).visible(h > 2.25),
    ])
}

/// The rects (device pixels) of the parts a strip in `rect_px` shows, in drawing order, spacers left out.
fn place_strip(rect_px: Rect, u_px: f32, master: bool) -> Vec<(NodeId, Rect)> {
    let mut placed = layout(&strip_tree(rect_px.height() / u_px, master), rect_px, u_px);
    placed.retain(|(id, _)| *id != SPACER);
    placed
}

/// Scrolls the row by the touches over `rect` that no strip control takes ([`scroller`]: the toggles take only taps,
/// so their drags scroll too), and steps it by the wheel; the row is `content` points long. Returns where a strip tap
/// went down.
fn scroll_row(ui: &Ui, state: &mut MixerState, rect: Rect, content: f32, u: f32) -> Option<Pos2> {
    let mut axes = [
        Axis {
            pos: f64::from(state.scroll),
            zoom: 1.0,
            range: [0.0, f64::from(content)],
            zoom_range: [1.0; 2],
            view: rect.x_range(),
        },
        Axis::OFF,
    ];
    let zones = Zones {
        touch: [rect, Rect::NOTHING],
        ..Zones::NONE
    };
    let opts = scroller::Options {
        covered: state.covered,
        follow: false,
    };
    let id = ui.id().with("mixer");
    let report = (state.scroller).update(ui, id, u, &zones, &mut axes, opts);
    // One strip per half strip of accumulated wheel travel, so a trackpad's many small events don't each step (any
    // wheel event stops a fling, in the scroller).
    // ponytail: the remainder (< half a strip) waits for the next event however late; clear it after a pause
    // if stale travel is ever noticed.
    let wheel = ui.input(|i| i.raw_scroll_delta);
    if wheel != Vec2::ZERO && !state.covered && ui.rect_contains_pointer(rect) {
        let unit = STRIP_W * u;
        state.wheel += wheel.x + wheel.y;
        let steps = (state.wheel / (unit / 2.0)).trunc();
        state.wheel -= steps * unit / 2.0;
        if steps != 0.0 {
            let x = &mut axes[X];
            x.pos = (x.pos - f64::from(steps * unit)).clamp(x.range[0], x.max());
        }
    }
    state.scroll = axes[X].pos as f32;
    report.tap.filter(|t| !t.long).map(|t| t.at)
}

/// Draws the mixer in `rect` and returns the channel params the user changed this frame, already written to `song`,
/// as `(channel, param index, new value)`.
pub fn show(
    ui: &mut Ui,
    cx: &UiCtx,
    song: &mut Song,
    state: &mut MixerState,
    rect: Rect,
) -> Vec<(usize, i32, f32)> {
    let (m, colors) = (cx.m, &cx.theme.colors);
    let (unit, ppp) = (m.u(STRIP_W), m.ppp());
    // Before the strips' controls: the row gets the touches that start between them, and the drags of the toggles.
    let base = ui.id().with("mixer");
    let content = song.channels.len() as f32 * unit;
    let tap = scroll_row(ui, state, rect, content, m.u(1.0));

    ui.painter()
        .rect_filled(rect, 0.0, colors.module_fill.to_color32());
    let mut ui = ui.new_child(UiBuilder::new().max_rect(rect));
    ui.set_clip_rect(rect.intersect(ui.clip_rect()));
    let mut changes = Vec::new();
    // In rack order, which a playlist move leaves alone: the file order is the mixer's.
    for (p, i) in song.rack_order().into_iter().enumerate() {
        let channel = &mut song.channels[i];
        let left = rect.left() + p as f32 * unit - state.scroll;
        if left >= rect.right() || left + unit <= rect.left() {
            continue;
        }
        let strip = Rect::from_min_size(pos2(left, rect.top()), vec2(unit, rect.height()));
        let color = channel_color(channel.hue, cx.theme).to_color32();
        let icon = kind_icon(channel.kind);
        let master = channel.kind == ChannelKind::Master;
        let name = channel.name.as_str();
        let links = links::links(&channel.file.cc_links);
        let params = &mut channel.params.0;
        let placed = place_strip(strip * ppp, m.u_px(), master);
        // A tap on the strip that no control is under (the four controls' ids are 0 ..= 3).
        if let Some(p) = tap
            && strip.contains(p)
            && !(placed.iter()).any(|&(leaf, r)| leaf <= SOLO && (r / ppp).contains(p))
        {
            state.selected = Some(i);
        }
        for (leaf, r) in placed {
            let (id, r) = (base.with((i, leaf)), r / ppp);
            // Each control is bound to its strip's channel param `leaf` (instance 0).
            let control = |label, default| Control {
                menu: ParamMenu::bound(links::has(&links, 0, leaf as i32)),
                fine_tune: fine_tune::channel_param(leaf as i32),
                ..Control::new(id, r, label, color, default)
            };
            let none = |_| String::new();
            let changed = match leaf {
                // In the channel colour, truncated keeping 5 tail characters.
                NAME => {
                    let font = m.font_id(FontSize::Pt10);
                    widgets::truncated(
                        ui.painter(),
                        r,
                        Align2::CENTER_CENTER,
                        name,
                        font,
                        color,
                        5,
                    );
                    false
                }
                // Reset to 0.85312, unity, not the 0.8531 a new channel stores.
                FADER => {
                    let c = control("", widgets::UNITY);
                    widgets::fader(&mut ui, cx, &c, &mut params[0]).changed()
                }
                KNOB => {
                    widgets::knob(&mut ui, cx, &control("", 0.5), &mut params[1], none).changed()
                }
                // Dot up = not muted: the inverted pill, bound to the Mute param itself.
                MUTE => {
                    let inverted = Pill {
                        inverted: true,
                        ..Pill::default()
                    };
                    let c = control("", 0.0);
                    widgets::pill_toggle(&mut ui, cx, &c, inverted, &mut params[2], none).changed()
                }
                SOLO => widgets::text_toggle(&mut ui, cx, &control("Solo", 0.0), &mut params[3])
                    .changed(),
                _ => {
                    let ink = if state.selected == Some(i) {
                        ui.painter().rect_filled(r, 0.0, color);
                        colors.background.to_color32()
                    } else {
                        color
                    };
                    draw_icon(ui.painter(), r, icon, ink, 1.0, m);
                    false
                }
            };
            if changed {
                changes.push((i, leaf as i32, params[leaf as usize]));
            }
            let target = AutomationTarget {
                instance_id: 0,
                param: leaf as i32,
            };
            if let Some(touch) = Touch::new(target, widgets::signal(&ui, id)) {
                state.touch = Some((i, touch));
            }
        }
    }
    changes
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::metrics::{DEFAULT_UI_SCALE_INDEX, FontSize, Metrics};
    use crate::song::{ChannelParams, SynthKind};
    use crate::theme::theme_by_id;
    use crate::ui::ctrl_menu::CtrlAction;
    use crate::ui::layout::rect_of;
    use crate::ui::offscreen::key_tap;
    use eframe::egui::epaint::ClippedShape;
    use eframe::egui::{
        self, Color32, Context, Event, Modifiers, MouseWheelUnit, PointerButton, Pos2, RawInput,
        Shape,
    };

    /// `U` at ppp 1.
    const U: f32 = 32.0;
    const UNIT: f32 = STRIP_W * U;
    /// The wheel travel that steps the row by one strip.
    const HALF: f32 = UNIT / 2.0;

    fn ids(h: f32, master: bool) -> Vec<NodeId> {
        let rect = Rect::from_min_size(Pos2::ZERO, vec2(UNIT, h));
        place_strip(rect, U, master)
            .into_iter()
            .map(|(id, _)| id)
            .collect()
    }

    #[test]
    fn each_part_hides_at_its_height_threshold() {
        // At its threshold a part is hidden; one pixel taller, it shows.
        // The name slot always shows.
        let all = [NAME, FADER, KNOB, MUTE, SOLO, CELL];
        assert_eq!(ids(13.0 * U, false), all);
        assert_eq!(ids(10.5 * U + 1.0, false), all);
        assert_eq!(ids(10.5 * U, false), [NAME, FADER, KNOB, CELL]);
        assert_eq!(ids(7.5 * U + 1.0, false), [NAME, FADER, KNOB, CELL]);
        assert_eq!(ids(7.5 * U, false), [NAME, FADER, CELL]);
        assert_eq!(ids(4.0 * U + 1.0, false), [NAME, FADER, CELL]);
        assert_eq!(ids(4.0 * U, false), [NAME, CELL]);
        assert_eq!(ids(2.25 * U + 1.0, false), [NAME, CELL]);
        assert_eq!(ids(2.25 * U, false), [NAME]);
    }

    #[test]
    fn the_strip_column_follows_section_8() {
        // 12 U at U = 32: spacer 24, fader 24..216 (6 U of flex), knob 216..280, row 280..336, cell 336..384.
        let placed = place_strip(
            Rect::from_min_size(Pos2::ZERO, vec2(UNIT, 12.0 * U)),
            U,
            false,
        );
        let at = |id| rect_of(&placed, id).unwrap();
        let r = |x0, y0, x1, y1| Rect::from_min_max(pos2(x0, y0), pos2(x1, y1));
        assert_eq!(at(NAME), r(0.0, 0.0, 128.0, 24.0));
        // 0.6 × 128 = 76.8 wide, centred from 25.6: edges truncated.
        assert_eq!(at(FADER), r(25.0, 24.0, 102.0, 216.0));
        // 0.8 × 128 = 102.4 wide from 12.8.
        assert_eq!(at(KNOB), r(12.0, 216.0, 115.0, 280.0));
        assert_eq!(at(MUTE), r(0.0, 280.0, 64.0, 336.0));
        assert_eq!(at(SOLO), r(64.0, 280.0, 128.0, 336.0));
        assert_eq!(at(CELL), r(0.0, 336.0, 128.0, 384.0));
        // Without a fader the spacer keeps the cell at the bottom.
        let low = place_strip(
            Rect::from_min_size(Pos2::ZERO, vec2(UNIT, 3.0 * U)),
            U,
            false,
        );
        assert_eq!(rect_of(&low, CELL), Some(r(0.0, 48.0, 128.0, 96.0)));
    }

    #[test]
    fn the_master_strip_has_no_mute_or_solo_but_keeps_the_row() {
        let rect = Rect::from_min_size(Pos2::ZERO, vec2(UNIT, 12.0 * U));
        assert_eq!(ids(12.0 * U, true), [NAME, FADER, KNOB, CELL]);
        let (master, other) = (place_strip(rect, U, true), place_strip(rect, U, false));
        for id in [NAME, FADER, KNOB, CELL] {
            assert_eq!(rect_of(&master, id), rect_of(&other, id), "{id}");
        }
    }

    #[test]
    fn the_row_scrolls_until_its_last_strip_meets_the_right_edge() {
        // max = rangeEnd − W, never below 0: the row's axis in the scroller, `strips` strips long.
        let max_scroll = |strips: usize, view_w: f32| {
            let axis = Axis {
                pos: 0.0,
                zoom: 1.0,
                range: [0.0, f64::from(strips as f32 * UNIT)],
                zoom_range: [1.0; 2],
                view: eframe::egui::Rangef::new(0.0, view_w),
            };
            axis.max() as f32
        };
        assert_eq!(max_scroll(10, 3.5 * UNIT), 6.5 * UNIT);
        assert_eq!(max_scroll(10, 3.0 * UNIT), 7.0 * UNIT);
        assert_eq!(max_scroll(3, 3.5 * UNIT), 0.0);
        assert_eq!(max_scroll(0, 0.5 * UNIT), 0.0);
    }

    // Offscreen: the view in a real context, driven by synthetic pointer events.

    /// The view: 3.5 strips by 12 U at (20, 10).
    const VIEW: Rect = Rect {
        min: pos2(20.0, 10.0),
        max: pos2(20.0 + 3.5 * UNIT, 10.0 + 12.0 * U),
    };

    struct Rig {
        ctx: Context,
        song: Song,
        state: MixerState,
        /// The rect the mixer is drawn in.
        view: Rect,
        time: f64,
        /// Every change reported since the rig was made.
        changes: Vec<(usize, i32, f32)>,
        /// What the last frame painted.
        shapes: Vec<ClippedShape>,
        /// Held through every frame.
        modifiers: Modifiers,
        /// The last frame asked for the next one at once.
        repaint: bool,
    }

    impl Rig {
        /// The master and `extra` more channels.
        fn new(extra: usize) -> Self {
            let mut song = Song::default();
            for _ in 0..extra {
                song.add_channel(ChannelKind::Synth(SynthKind::MiniSynth));
            }
            let mut rig = Self {
                ctx: Context::default(),
                song,
                state: MixerState::default(),
                view: VIEW,
                time: 0.0,
                changes: Vec::new(),
                shapes: Vec::new(),
                modifiers: Modifiers::NONE,
                repaint: false,
            };
            rig.frame(vec![]);
            rig
        }

        fn frame(&mut self, events: Vec<Event>) {
            self.time += 1.0 / 60.0;
            let raw = RawInput {
                screen_rect: Some(Rect::from_min_size(Pos2::ZERO, vec2(800.0, 600.0))),
                time: Some(self.time),
                events,
                modifiers: self.modifiers,
                ..Default::default()
            };
            let (theme, m) = (
                theme_by_id(0),
                Metrics::from_pixels_per_point(1.0, DEFAULT_UI_SCALE_INDEX),
            );
            let cx = UiCtx {
                theme: &theme,
                m: &m,
            };
            let (song, state, changes) = (&mut self.song, &mut self.state, &mut self.changes);
            let view = self.view;
            let out = self.ctx.run(raw, |ctx| {
                egui::CentralPanel::default().show(ctx, |ui| {
                    changes.extend(show(ui, &cx, song, state, view));
                });
            });
            self.shapes = out.shapes;
            self.repaint = out.viewport_output[&egui::ViewportId::ROOT].repaint_delay
                == std::time::Duration::ZERO;
        }

        /// Frames with no input for `secs` seconds.
        fn idle(&mut self, secs: f64) {
            let end = self.time + secs;
            while self.time < end {
                self.frame(vec![]);
            }
        }

        fn press(&mut self, pos: Pos2) {
            self.frame(vec![Event::PointerMoved(pos)]);
            self.button(pos, true);
        }

        fn move_to(&mut self, pos: Pos2) {
            self.frame(vec![Event::PointerMoved(pos)]);
        }

        /// The circles the last frame painted with their centre in `rect`: (centre, fill).
        fn dots(&self, rect: Rect) -> Vec<(Pos2, Color32)> {
            let circles = self.shapes.iter().filter_map(|s| match &s.shape {
                Shape::Circle(c) if rect.contains(c.center) => Some((c.center, c.fill)),
                _ => None,
            });
            circles.collect()
        }

        /// The texts the last frame painted centred in `rect`: (text, colour, font size).
        fn texts(&self, rect: Rect) -> Vec<(String, Color32, f32)> {
            let texts = self.shapes.iter().filter_map(|s| match &s.shape {
                Shape::Text(t) if rect.contains(t.visual_bounding_rect().center()) => {
                    let format = &t.galley.job.sections[0].format;
                    Some((
                        t.galley.text().to_owned(),
                        format.color,
                        format.font_id.size,
                    ))
                }
                _ => None,
            });
            texts.collect()
        }

        /// The texts the last frame painted centred in `rect`: (text, where, font size).
        fn text_rects(&self, rect: Rect) -> Vec<(String, Rect, f32)> {
            let texts = self.shapes.iter().filter_map(|s| match &s.shape {
                Shape::Text(t) if rect.contains(t.visual_bounding_rect().center()) => Some((
                    t.galley.text().to_owned(),
                    t.visual_bounding_rect(),
                    t.galley.job.sections[0].format.font_id.size,
                )),
                _ => None,
            });
            texts.collect()
        }

        fn button(&mut self, pos: Pos2, pressed: bool) {
            self.frame(vec![Event::PointerButton {
                pos,
                button: PointerButton::Primary,
                pressed,
                modifiers: Default::default(),
            }]);
        }

        fn click(&mut self, pos: Pos2) {
            self.frame(vec![Event::PointerMoved(pos)]);
            self.button(pos, true);
            self.button(pos, false);
        }

        /// One wheel event of `dy` points (negative scrolls the row towards its end).
        fn wheel(&mut self, dy: f32) {
            self.frame(vec![Event::MouseWheel {
                unit: MouseWheelUnit::Point,
                delta: vec2(0.0, dy),
                modifiers: Default::default(),
            }]);
        }

        /// Presses at `from`, moves through `offsets` from there, releases.
        fn drag(&mut self, from: Pos2, offsets: &[egui::Vec2]) {
            self.frame(vec![Event::PointerMoved(from)]);
            self.button(from, true);
            for &d in offsets {
                self.frame(vec![Event::PointerMoved(from + d)]);
            }
            self.button(from + *offsets.last().unwrap(), false);
        }

        /// Part `id` of strip `i` at the current scroll, in points.
        fn part(&self, i: usize, id: NodeId) -> Rect {
            let (view, scroll) = (self.view, self.state.scroll);
            let left = view.left() + i as f32 * UNIT - scroll;
            let strip = Rect::from_min_size(pos2(left, view.top()), vec2(UNIT, view.height()));
            rect_of(&place_strip(strip, U, i == 0), id).expect("shown")
        }
    }

    #[test]
    fn the_strips_follow_the_rack_order_which_a_playlist_move_leaves_alone() {
        // Three MiniSynths, then a send: the send's strip comes right after the master's (file kind order).
        let mut rig = Rig::new(3);
        rig.song.add_channel(ChannelKind::EffectTrack);
        rig.song.channels[4].name = "FX 1".into();
        // Wide enough for all five.
        rig.view = Rect::from_min_size(VIEW.min, vec2(5.0 * UNIT, VIEW.height()));
        let names = |rig: &mut Rig| -> Vec<String> {
            rig.frame(vec![]);
            (0..5)
                .map(|p| {
                    let slot = rig.part(p, NAME);
                    rig.texts(slot)
                        .first()
                        .map(|t| t.0.clone())
                        .unwrap_or_default()
                })
                .collect()
        };
        let want = ["MASTER", "FX 1", "MiniSynth", "MiniSynth 2", "MiniSynth 3"];
        assert_eq!(names(&mut rig), want);
        // MiniSynth 3 to the top of the playlist and the master below it: the strips stay.
        assert!(rig.song.set_playlist(&[3, 0, 1, 2, 4]));
        assert_eq!(names(&mut rig), want);
        // Read channels keep the order they were read in, whatever their ids: the synths read in reverse.
        for (name, read) in [("MiniSynth 3", 5), ("MiniSynth 2", 6), ("MiniSynth", 7)] {
            let c = rig
                .song
                .channels
                .iter_mut()
                .find(|c| c.name == name)
                .unwrap();
            c.file.rack_order = crate::song::Raw(Some(read));
        }
        let read = ["MASTER", "FX 1", "MiniSynth 3", "MiniSynth 2", "MiniSynth"];
        assert_eq!(names(&mut rig), read);
    }

    #[test]
    fn dragging_a_fader_writes_the_level_and_reports_it() {
        let mut rig = Rig::new(2);
        // Down a quarter of the range: the thumb follows the pointer along the track, the fader less 0.5 U at each
        // end (and 0.875 U more when it is 6.5 U tall).
        let fader = rig.part(1, FADER);
        let ends = if fader.height() >= 6.5 * U { 2.75 } else { 1.0 };
        let quarter = (fader.height() - ends * U) / 4.0;
        rig.drag(fader.center(), &[vec2(0.0, 10.0), vec2(0.0, quarter)]);
        let level = rig.song.channels[1].params.level();
        assert!((level - (0.8531 - 0.25)).abs() < 1e-5, "{level}");
        assert!(!rig.changes.is_empty());
        assert!(
            rig.changes
                .iter()
                .all(|&(ch, index, _)| (ch, index) == (1, 0))
        );
        assert_eq!(rig.changes.last(), Some(&(1, 0, level)));
        // The other strips kept their defaults.
        assert_eq!(rig.song.channels[0].params, ChannelParams::default());
        assert_eq!(rig.song.channels[2].params, ChannelParams::default());
    }

    #[test]
    fn dragging_the_pan_knob_writes_param_1() {
        let mut rig = Rig::new(1);
        rig.drag(
            rig.part(1, KNOB).center(),
            &[vec2(0.0, -10.0), vec2(0.0, -200.0)],
        );
        let pan = rig.song.channels[1].params.pan();
        assert!((pan - 0.75).abs() < 1e-5, "{pan}");
        assert_eq!(rig.changes.last(), Some(&(1, 1, pan)));
    }

    /// The pointer travel that moves strip `i`'s fader by its whole range: its track's length.
    fn fader_track(rig: &Rig, i: usize) -> f32 {
        let fader = rig.part(i, FADER);
        let ends = if fader.height() >= 6.5 * U { 2.75 } else { 1.0 };
        fader.height() - ends * U
    }

    #[test]
    fn ctrl_drags_the_fader_and_the_pan_knob_a_quarter_as_far() {
        let mut rig = Rig::new(1);
        rig.modifiers = Modifiers::COMMAND;
        let quarter = fader_track(&rig, 1) / 4.0;
        rig.drag(
            rig.part(1, FADER).center(),
            &[vec2(0.0, 10.0), vec2(0.0, quarter)],
        );
        let level = rig.song.channels[1].params.level();
        assert!((level - (0.8531 - 0.0625)).abs() < 1e-5, "{level}");
        // 200 points up is 0.25 of the range at full rate.
        rig.drag(
            rig.part(1, KNOB).center(),
            &[vec2(0.0, -10.0), vec2(0.0, -200.0)],
        );
        let pan = rig.song.channels[1].params.pan();
        assert!((pan - 0.5625).abs() < 1e-5, "{pan}");
    }

    /// Two touch-downs on `at` a frame apart, the first released: the second one resets.
    fn double_tap(rig: &mut Rig, at: Pos2) {
        rig.click(at);
        rig.button(at, true);
        rig.button(at, false);
    }

    #[test]
    fn a_double_tap_resets_the_fader_to_unity_and_the_pan_to_the_centre() {
        let mut rig = Rig::new(2);
        let quarter = fader_track(&rig, 2) / 4.0;
        rig.drag(
            rig.part(2, FADER).center(),
            &[vec2(0.0, 10.0), vec2(0.0, quarter)],
        );
        rig.drag(
            rig.part(2, KNOB).center(),
            &[vec2(0.0, -10.0), vec2(0.0, -200.0)],
        );
        rig.changes.clear();
        rig.idle(0.5);
        // Slow taps are taps only: they change nothing.
        rig.click(rig.part(2, FADER).center());
        rig.idle(0.3);
        rig.click(rig.part(2, FADER).center());
        assert!(rig.changes.is_empty());
        rig.idle(0.5);
        // 0.85312, the Level reset value, which is not quite the 0.8531 a new channel stores.
        let (fader, knob) = (rig.part(2, FADER).center(), rig.part(2, KNOB).center());
        double_tap(&mut rig, fader);
        assert_eq!(rig.song.channels[2].params.level(), widgets::UNITY);
        assert_eq!(rig.changes, [(2, 0, 0.85312)]);
        double_tap(&mut rig, knob);
        assert_eq!(rig.song.channels[2].params.pan(), 0.5);
        assert_eq!(rig.changes, [(2, 0, 0.85312), (2, 1, 0.5)]);
    }

    #[test]
    fn mute_and_solo_taps_follow_the_two_state_law() {
        // Drawn on for any value but 0; a tap writes 1 from exactly 0 and 0 from anything else, 0.5 (a file's) too.
        let mut rig = Rig::new(1);
        rig.song.channels[1].params.0[2] = 0.5;
        rig.song.channels[1].params.0[3] = 0.5;
        rig.frame(vec![]);
        let (mute, solo) = (rig.part(1, MUTE), rig.part(1, SOLO));
        let theme = theme_by_id(0);
        let menu = theme.colors.menu_fill.to_color32();
        // Inverted: Mute 0.5 draws the muted side, dot down.
        assert_eq!(rig.dots(mute), [(mute.center() + vec2(0.0, 8.0), menu)]);
        assert_eq!(rig.texts(solo)[0].1, theme.colors.text.to_color32());
        rig.click(mute.center());
        rig.click(solo.center());
        assert_eq!(rig.changes, [(1, 2, 0.0), (1, 3, 0.0)]);
        rig.click(solo.center());
        assert_eq!(rig.changes[2], (1, 3, 1.0));
    }

    #[test]
    fn solo_is_not_exclusive() {
        let mut rig = Rig::new(2);
        let solos =
            |rig: &Rig| -> Vec<f32> { (rig.song.channels.iter()).map(|c| c.params.0[3]).collect() };
        rig.click(rig.part(1, SOLO).center());
        rig.click(rig.part(2, SOLO).center());
        assert_eq!(rig.changes, [(1, 3, 1.0), (2, 3, 1.0)]);
        assert_eq!(solos(&rig), [0.0, 1.0, 1.0]);
        // Unsoloing one leaves the other soloed.
        rig.click(rig.part(1, SOLO).center());
        assert_eq!(solos(&rig), [0.0, 0.0, 1.0]);
        assert_eq!(rig.changes.len(), 3);
    }

    #[test]
    fn mute_and_solo_clicks_flip_params_2_and_3() {
        let mut rig = Rig::new(1);
        let (mute, solo) = (rig.part(1, MUTE).center(), rig.part(1, SOLO).center());
        rig.click(mute);
        assert_eq!(rig.changes, [(1, 2, 1.0)]);
        rig.click(solo);
        rig.click(mute);
        assert_eq!(rig.changes, [(1, 2, 1.0), (1, 3, 1.0), (1, 2, 0.0)]);
        assert_eq!(rig.song.channels[1].params.0[..4], [0.8531, 0.5, 0.0, 1.0]);
        // The master's row is empty: a click there changes nothing.
        let row = Rect::from_min_max(
            rig.part(0, KNOB).left_bottom(),
            rig.part(0, CELL).right_top(),
        );
        rig.click(row.center());
        assert_eq!(rig.changes.len(), 3);
        assert_eq!(rig.song.channels[0].params, ChannelParams::default());
    }

    #[test]
    fn a_strip_controls_press_is_its_channel_param_touch() {
        let mut rig = Rig::new(2);
        let press = |rig: &mut Rig, i, id| {
            rig.state.touch = None;
            let at = rig.part(i, id).center();
            rig.frame(vec![Event::PointerMoved(at)]);
            rig.button(at, true);
            let touch = rig.state.touch.take();
            rig.button(at, false);
            touch.map(|(channel, t)| (channel, t.target.instance_id, t.target.param, t.pick))
        };
        // Instance 0, the strip's param: Level, Pan, Mute, Solo.
        assert_eq!(press(&mut rig, 2, FADER), Some((2, 0, 0, None)));
        assert_eq!(press(&mut rig, 1, KNOB), Some((1, 0, 1, None)));
        assert_eq!(press(&mut rig, 1, MUTE), Some((1, 0, 2, None)));
        assert_eq!(press(&mut rig, 0, FADER), Some((0, 0, 0, None)));
        // The bottom cell is no control.
        assert_eq!(press(&mut rig, 1, CELL), None);
        assert_eq!(rig.state.touch, None);
    }

    #[test]
    fn a_strip_controls_menu_unlinks_only_a_param_its_channel_links() {
        let mut rig = Rig::new(2);
        // Channel 1's CCLK links CC 20 to its Pan (instance 0, param 1); channel 2 links nothing.
        crate::midi::links::link(&mut rig.song.channels[1].file.cc_links, 20, 0, 1);
        let unlink = |rig: &mut Rig, i, part| {
            let at = rig.part(i, part).center();
            let button = |pressed| Event::PointerButton {
                pos: at,
                button: PointerButton::Secondary,
                pressed,
                modifiers: Default::default(),
            };
            rig.frame(vec![Event::PointerMoved(at)]);
            rig.frame(vec![button(true)]);
            rig.frame(vec![button(false)]);
            rig.frame(vec![]);
            let screen = Rect::from_min_size(Pos2::ZERO, vec2(800.0, 600.0));
            let texts = rig.text_rects(screen);
            let item = texts.iter().find(|(t, ..)| t == "Unlink from MIDI CC");
            let item = item.expect("menu open").1.center();
            // The secondary press was a touch of its own.
            rig.state.touch = None;
            rig.frame(vec![Event::PointerMoved(item)]);
            rig.button(item, true);
            rig.button(item, false);
            let touch = rig.state.touch.take();
            rig.frame(key_tap(egui::Key::Escape, Default::default()));
            touch.map(|(channel, t)| (channel, t.target.param, t.pick))
        };
        assert_eq!(
            unlink(&mut rig, 1, KNOB),
            Some((1, 1, Some(CtrlAction::UnlinkMidiCc)))
        );
        // Dimmed on channel 2, and on channel 1's Level: nothing picked.
        assert_eq!(unlink(&mut rig, 2, KNOB), None);
        assert_eq!(unlink(&mut rig, 1, FADER), None);
    }

    #[test]
    fn level_and_pan_offer_fine_tune_control_and_mute_and_solo_do_not() {
        let mut rig = Rig::new(2);
        let offers = |rig: &mut Rig, part| {
            let at = rig.part(1, part).center();
            let button = |pressed| Event::PointerButton {
                pos: at,
                button: PointerButton::Secondary,
                pressed,
                modifiers: Default::default(),
            };
            rig.frame(vec![Event::PointerMoved(at)]);
            rig.frame(vec![button(true)]);
            rig.frame(vec![button(false)]);
            rig.frame(vec![]);
            let screen = Rect::from_min_size(Pos2::ZERO, vec2(800.0, 600.0));
            let texts = rig.text_rects(screen);
            let has = |label: &str| texts.iter().any(|(t, ..)| t == label);
            assert!(has("Add automation track"), "the menu is open");
            let offered = has("Fine tune control");
            rig.frame(key_tap(egui::Key::Escape, Default::default()));
            rig.frame(vec![]);
            offered
        };
        assert!(offers(&mut rig, FADER));
        assert!(offers(&mut rig, KNOB));
        assert!(!offers(&mut rig, MUTE));
        assert!(!offers(&mut rig, SOLO));
    }

    #[test]
    fn a_tap_off_the_controls_makes_its_strip_current() {
        let mut rig = Rig::new(2);
        assert_eq!(rig.state.selected, None);
        // The bottom cell, the name slot, beside the fader, and the master's empty mute/solo row.
        let beside = |rig: &Rig, i| {
            let fader = rig.part(i, FADER);
            pos2(fader.left() - 8.0, fader.center().y)
        };
        let row = |rig: &Rig| {
            Rect::from_min_max(
                rig.part(0, KNOB).left_bottom(),
                rig.part(0, CELL).right_top(),
            )
            .center()
        };
        for (at, i) in [
            (rig.part(2, CELL).center(), 2),
            (rig.part(1, NAME).center(), 1),
            (beside(&rig, 2), 2),
            (row(&rig), 0),
            (rig.part(1, CELL).center(), 1),
        ] {
            rig.click(at);
            assert_eq!(rig.state.selected, Some(i), "{at:?}");
        }
        assert!(rig.changes.is_empty());
        // A tap on a control is the control's: Mute flips, the strip stays as it was.
        rig.click(rig.part(2, MUTE).center());
        rig.click(rig.part(2, SOLO).center());
        rig.click(rig.part(2, FADER).center());
        rig.click(rig.part(2, KNOB).center());
        assert_eq!(rig.state.selected, Some(1));
        assert_eq!(rig.changes, [(2, 2, 1.0), (2, 3, 1.0)]);
    }

    /// Strip 1's name slot at scroll 0: no control, so a touch there is the row's.
    fn between() -> Pos2 {
        pos2(VIEW.left() + 1.5 * UNIT, VIEW.top() + 0.3 * U)
    }

    #[test]
    fn a_strip_tap_is_released_within_0_5_u_before_0_3_s() {
        let mut rig = Rig::new(2);
        let from = between();
        // 10 points sideways and 12 down (15.6 from the touch-down, inside the 16 of 0.5 U): still a tap.
        rig.press(from);
        rig.move_to(from + vec2(10.0, 12.0));
        rig.button(from + vec2(10.0, 12.0), false);
        assert_eq!(rig.state.selected, Some(1));
        // Held 0.27 s (0.25 s and the release frame): still a tap.
        let cell = rig.part(2, CELL).center();
        rig.press(cell);
        rig.idle(0.25);
        rig.button(cell, false);
        assert_eq!(rig.state.selected, Some(2));
        // Held 0.3 s: a long tap, which the mixer ignores.
        let cell = rig.part(1, CELL).center();
        rig.press(cell);
        rig.idle(0.3);
        rig.button(cell, false);
        assert_eq!(rig.state.selected, Some(2));
        // Out of the calm zone and back: a scroll, not a tap.
        rig.press(cell);
        rig.move_to(cell + vec2(17.0, 0.0));
        rig.move_to(cell);
        rig.button(cell, false);
        assert_eq!(rig.state.selected, Some(2));
        assert!(rig.changes.is_empty());
    }

    #[test]
    fn a_tap_pressed_and_released_in_one_frame_is_a_strip_tap() {
        // A trackpad's tap-to-click delivers both edges together.
        let mut rig = Rig::new(2);
        let tap = |rig: &mut Rig, at: Pos2| {
            rig.frame(vec![
                Event::PointerMoved(at),
                Event::PointerButton {
                    pos: at,
                    button: PointerButton::Primary,
                    pressed: true,
                    modifiers: Default::default(),
                },
                Event::PointerButton {
                    pos: at,
                    button: PointerButton::Primary,
                    pressed: false,
                    modifiers: Default::default(),
                },
            ]);
        };
        let (name, mute) = (rig.part(2, NAME).center(), rig.part(1, MUTE).center());
        tap(&mut rig, name);
        assert_eq!(rig.state.selected, Some(2));
        // On a control it is the control's.
        tap(&mut rig, mute);
        assert_eq!(rig.state.selected, Some(2));
        assert_eq!(rig.changes, [(1, 2, 1.0)]);
    }

    #[test]
    fn a_touch_held_still_for_a_second_still_scrolls() {
        // A finger, as a touch screen sends it: touch events with the pointer events. Held still this long, egui
        // takes it for a long touch and drops the press from the row's response.
        let mut rig = Rig::new(9);
        let from = between();
        let touch = |phase, pos| Event::Touch {
            device_id: egui::TouchDeviceId(0),
            id: egui::TouchId(0),
            phase,
            pos,
            force: None,
        };
        let button = |pos, pressed| Event::PointerButton {
            pos,
            button: PointerButton::Primary,
            pressed,
            modifiers: Default::default(),
        };
        rig.frame(vec![
            touch(egui::TouchPhase::Start, from),
            Event::PointerMoved(from),
            button(from, true),
        ]);
        rig.idle(1.0);
        for dx in [-17.0, -117.0] {
            let at = from + vec2(dx, 0.0);
            rig.frame(vec![
                touch(egui::TouchPhase::Move, at),
                Event::PointerMoved(at),
            ]);
        }
        assert_eq!(rig.state.scroll, 117.0);
        let at = from + vec2(-117.0, 0.0);
        rig.frame(vec![
            touch(egui::TouchPhase::End, at),
            button(at, false),
            Event::PointerGone,
        ]);
        assert_eq!(rig.state.selected, None);
    }

    #[test]
    fn a_touch_scrolls_the_row_past_0_5_u_and_then_follows_one_to_one() {
        // Ten strips in 3.5.
        let mut rig = Rig::new(9);
        let from = between();
        rig.press(from);
        // Up to 0.5 U (16 points) sideways, and any distance up or down: still.
        for d in [vec2(-10.0, 0.0), vec2(-16.0, 0.0), vec2(-16.0, 100.0)] {
            rig.move_to(from + d);
            assert_eq!(rig.state.scroll, 0.0, "{d:?}");
        }
        // Past it, each move moves the row by as much, both ways.
        for (dx, scroll) in [(-18.0, 2.0), (-58.0, 42.0), (-50.0, 34.0), (-53.0, 37.0)] {
            rig.move_to(from + vec2(dx, 0.0));
            assert_eq!(rig.state.scroll, scroll, "{dx}");
        }
        // The last move was 3 points, no more than 0.125 U: no fling, and the row rests where it was let go.
        rig.button(from + vec2(-53.0, 0.0), false);
        rig.idle(1.0);
        assert_eq!(rig.state.scroll, 37.0);
        assert!(!rig.repaint);
        assert_eq!(rig.state.selected, None);
        // The strips take their input where they are drawn now.
        rig.click(rig.part(3, MUTE).center());
        assert_eq!(rig.changes, [(3, 2, 1.0)]);
    }

    #[test]
    fn past_either_end_the_row_follows_a_fifth_and_springs_back_when_let_go() {
        let mut rig = Rig::new(9);
        let from = between();
        rig.press(from);
        // 17 points right of the touch-down at scroll 0: all of it past the start, a fifth of it shown.
        rig.move_to(from + vec2(17.0, 0.0));
        assert!(
            (rig.state.scroll + 3.4).abs() < 1e-4,
            "{}",
            rig.state.scroll
        );
        rig.move_to(from + vec2(67.0, 0.0));
        assert!(
            (rig.state.scroll + 13.4).abs() < 1e-4,
            "{}",
            rig.state.scroll
        );
        // Back the same way: 67 points of finger to the start, then 1:1.
        rig.move_to(from + vec2(-10.0, 0.0));
        assert!(
            (rig.state.scroll - 10.0).abs() < 1e-4,
            "{}",
            rig.state.scroll
        );
        rig.move_to(from + vec2(60.0, 0.0));
        rig.move_to(from + vec2(62.0, 0.0));
        let over = rig.state.scroll;
        assert!((over + 12.4).abs() < 1e-4, "{over}");
        // Let go: the overshoot shrinks by 10 % a frame, the release frame included, and the row comes to rest at 0.
        rig.button(from + vec2(62.0, 0.0), false);
        assert!((rig.state.scroll - 0.9 * over).abs() < 1e-4);
        rig.frame(vec![]);
        assert!((rig.state.scroll - 0.81 * over).abs() < 1e-4);
        // Still springing 0.2 s on, long after the input: the spring asks for its frames.
        rig.idle(0.2);
        assert!(rig.state.scroll < -1.0, "{}", rig.state.scroll);
        assert!(rig.repaint);
        rig.idle(2.0);
        assert_eq!(rig.state.scroll, 0.0);
        assert!(!rig.repaint);
        // The far end: 10 strips in 3.5 stop with the last at the right edge, 6.5 strips in.
        for _ in 0..20 {
            rig.wheel(-HALF);
        }
        assert_eq!(rig.state.scroll, 6.5 * UNIT);
        rig.press(from);
        rig.move_to(from + vec2(-17.0, 0.0));
        rig.move_to(from + vec2(-117.0, 0.0));
        assert!((rig.state.scroll - (6.5 * UNIT + 23.4)).abs() < 1e-3);
        rig.button(from + vec2(-117.0, 0.0), false);
        rig.idle(2.0);
        assert_eq!(rig.state.scroll, 6.5 * UNIT);
    }

    #[test]
    fn a_flick_flings_the_row_slowing_by_0_9_a_frame() {
        // Twenty strips.
        let mut rig = Rig::new(19);
        let from = between();
        rig.press(from);
        rig.move_to(from + vec2(-20.0, 0.0));
        rig.move_to(from + vec2(-50.0, 0.0));
        assert_eq!(rig.state.scroll, 50.0);
        // The last move was 30 points: v = 30 a frame, ×0.9 before each frame's step.
        rig.button(from + vec2(-50.0, 0.0), false);
        assert!(
            (rig.state.scroll - 77.0).abs() < 1e-3,
            "{}",
            rig.state.scroll
        );
        rig.frame(vec![]);
        assert!(
            (rig.state.scroll - 101.3).abs() < 1e-3,
            "{}",
            rig.state.scroll
        );
        // Still flying 0.2 s on, long after the input: the fling asks for its frames.
        rig.idle(0.2);
        assert!(rig.repaint);
        // 30 × 0.9 / 0.1 = 270 in all, less a tail below 0.01 once v drops under 0.001.
        rig.idle(3.0);
        assert!(
            (rig.state.scroll - 320.0).abs() < 0.01,
            "{}",
            rig.state.scroll
        );
        assert!(!rig.repaint);
        assert_eq!(rig.state.selected, None);
    }

    #[test]
    fn a_last_move_of_more_than_0_125_u_flings() {
        // 0.125 U is 4 points: a last move of 4 lets the row rest, one of 5 flings it (v = 5, ×0.9 a frame).
        for (last, after) in [(4.0, 21.0), (5.0, 26.5)] {
            let mut rig = Rig::new(19);
            let from = between();
            rig.press(from);
            rig.move_to(from + vec2(-17.0, 0.0));
            rig.move_to(from + vec2(-17.0 - last, 0.0));
            rig.button(from + vec2(-17.0 - last, 0.0), false);
            assert!(
                (rig.state.scroll - after).abs() < 1e-4,
                "{last}: {}",
                rig.state.scroll
            );
        }
    }

    #[test]
    fn a_fling_into_an_end_springs_back_to_it() {
        let mut rig = Rig::new(19);
        let from = between();
        rig.press(from);
        rig.move_to(from + vec2(-17.0, 0.0));
        rig.move_to(from + vec2(-57.0, 0.0));
        rig.move_to(from + vec2(-17.0, 0.0));
        // At 17 after a 40-point move right: the fling (36 on the first frame) runs past the start.
        assert_eq!(rig.state.scroll, 17.0);
        rig.button(from + vec2(-17.0, 0.0), false);
        assert_eq!(rig.state.scroll, -19.0);
        rig.idle(3.0);
        assert_eq!(rig.state.scroll, 0.0);
        assert!(!rig.repaint);
    }

    #[test]
    fn a_touch_down_or_a_wheel_event_stops_a_fling() {
        let flung = || {
            let mut rig = Rig::new(19);
            let from = between();
            rig.press(from);
            rig.move_to(from + vec2(-20.0, 0.0));
            rig.move_to(from + vec2(-50.0, 0.0));
            rig.button(from + vec2(-50.0, 0.0), false);
            let before = rig.state.scroll;
            rig.frame(vec![]);
            assert!(rig.state.scroll > before);
            rig
        };
        // A touch-down on a control.
        let mut rig = flung();
        let knob = pos2(VIEW.left() + 2.5 * UNIT, rig.part(2, KNOB).center().y);
        rig.press(knob);
        let held = rig.state.scroll;
        rig.idle(0.5);
        assert_eq!(rig.state.scroll, held);
        rig.button(knob, false);
        rig.idle(0.5);
        assert_eq!(rig.state.scroll, held);
        // A wheel event too small to step.
        let mut rig = flung();
        rig.wheel(-1.0);
        let held = rig.state.scroll;
        rig.idle(0.5);
        assert_eq!(rig.state.scroll, held);
    }

    #[test]
    fn the_wheel_accumulates_and_steps_a_strip_per_half_strip() {
        let mut rig = Rig::new(9);
        rig.frame(vec![Event::PointerMoved(VIEW.center())]);
        // Trackpad-sized events add up: 3 × 20 is short of 64, the fourth passes it.
        for _ in 0..3 {
            rig.wheel(-20.0);
        }
        assert_eq!(rig.state.scroll, 0.0);
        rig.wheel(-20.0);
        assert_eq!(rig.state.scroll, UNIT);
        // The 16 left over count towards the next step: 16 + 48 = 64.
        rig.wheel(-47.0);
        assert_eq!(rig.state.scroll, UNIT);
        rig.wheel(-1.0);
        assert_eq!(rig.state.scroll, 2.0 * UNIT);
        // Opposite events cancel instead of each stepping.
        for _ in 0..4 {
            rig.wheel(40.0);
            rig.wheel(-40.0);
        }
        assert_eq!(rig.state.scroll, 2.0 * UNIT);
        // A flick of 2.5 half strips steps two strips and keeps the half.
        rig.wheel(2.5 * HALF);
        assert_eq!(rig.state.scroll, 0.0);
        rig.wheel(-2.5 * HALF);
        assert_eq!(rig.state.scroll, 2.0 * UNIT);
        // Sideways events scroll too.
        rig.frame(vec![Event::MouseWheel {
            unit: MouseWheelUnit::Point,
            delta: vec2(-HALF, 0.0),
            modifiers: Default::default(),
        }]);
        assert_eq!(rig.state.scroll, 3.0 * UNIT);
        // Outside the view the wheel does nothing, and adds nothing.
        rig.frame(vec![Event::PointerMoved(pos2(5.0, 5.0))]);
        rig.wheel(-HALF);
        assert_eq!(rig.state.scroll, 3.0 * UNIT);
    }

    #[test]
    fn the_mute_pill_is_inverted_dot_up_when_not_muted() {
        let mut rig = Rig::new(1);
        let theme = theme_by_id(0);
        let mute = rig.part(1, MUTE);
        let channel = channel_color(rig.song.channels[1].hue, &theme).to_color32();
        let menu = theme.colors.menu_fill.to_color32();
        // Mute = 0: the dot is 4 t (t = 2) above the middle, in the channel colour.
        assert_eq!(rig.song.channels[1].params.0[2], 0.0);
        assert_eq!(rig.dots(mute), [(mute.center() - vec2(0.0, 8.0), channel)]);
        // A click mutes: Mute = 1, and the dot drops and goes menuFill.
        rig.click(mute.center());
        assert_eq!(rig.changes, [(1, 2, 1.0)]);
        assert_eq!(rig.song.channels[1].params.0[2], 1.0);
        assert_eq!(rig.dots(mute), [(mute.center() + vec2(0.0, 8.0), menu)]);
        // And back.
        rig.click(mute.center());
        assert_eq!(rig.changes, [(1, 2, 1.0), (1, 2, 0.0)]);
        assert_eq!(rig.dots(mute), [(mute.center() - vec2(0.0, 8.0), channel)]);
    }

    #[test]
    fn solo_is_an_8_pt_text_toggle_dim_until_on() {
        let mut rig = Rig::new(1);
        let theme = theme_by_id(0);
        let solo = rig.part(1, SOLO);
        let text = theme.colors.text.to_color32();
        let size = Metrics::from_pixels_per_point(1.0, DEFAULT_UI_SCALE_INDEX)
            .font_id(FontSize::Pt8)
            .size;
        let dim = text.gamma_multiply(0.25);
        // Off: dim `Solo`, and no pill.
        assert_eq!(rig.texts(solo), [("Solo".to_owned(), dim, size)]);
        assert!(rig.dots(solo).is_empty());
        // On: lit.
        rig.click(solo.center());
        assert_eq!(rig.changes, [(1, 3, 1.0)]);
        assert_eq!(rig.texts(solo), [("Solo".to_owned(), text, size)]);
        assert!(rig.dots(solo).is_empty());
    }

    /// 10 pt at U = 32.
    const NAME_SIZE: f32 = 20.0;

    #[test]
    fn a_long_channel_name_is_truncated_keeping_its_last_5_characters() {
        let mut rig = Rig::new(1);
        let full = "A channel name far too long for its strip";
        rig.song.channels[1].name = full.to_owned();
        rig.frame(vec![]);
        let slot = rig.part(1, NAME);
        let texts = rig.text_rects(slot);
        let [(shown, at, size)] = &texts[..] else {
            panic!("one name: {texts:?}");
        };
        let (head, tail) = shown
            .split_once("...")
            .unwrap_or_else(|| panic!("{shown}: no ..."));
        assert!(full.starts_with(head) && head.len() > 5, "{shown}");
        assert_eq!(tail, "strip", "{shown}");
        assert_eq!(*size, NAME_SIZE);
        // Centred in the slot and no wider than it.
        assert!(
            (at.center().x - slot.center().x).abs() < 1.0,
            "{at:?} {slot:?}"
        );
        assert!(at.width() <= slot.width(), "{at:?} {slot:?}");
    }

    #[test]
    fn a_short_channel_name_is_drawn_whole_centred_in_its_slot() {
        let rig = Rig::new(1);
        for i in [0, 1] {
            let (slot, name) = (rig.part(i, NAME), &rig.song.channels[i].name);
            let texts = rig.text_rects(slot);
            let [(shown, at, size)] = &texts[..] else {
                panic!("one name: {texts:?}");
            };
            assert_eq!(shown, name);
            assert_eq!(*size, NAME_SIZE);
            assert!(
                (at.center().x - slot.center().x).abs() < 1.0,
                "{at:?} {slot:?}"
            );
        }
    }

    #[test]
    fn the_name_is_in_the_channel_colour_at_every_strip_height() {
        let theme = theme_by_id(0);
        let mut rig = Rig::new(1);
        // Full height, the 2.25 U threshold (name only) and a strip shorter than the name slot.
        for h in [VIEW.height(), 2.25 * U, 0.5 * U] {
            rig.view = Rect::from_min_size(VIEW.min, vec2(VIEW.width(), h));
            rig.frame(vec![]);
            for i in [0, 1] {
                let channel = &rig.song.channels[i];
                let color = channel_color(channel.hue, &theme).to_color32();
                let texts = rig.texts(rig.part(i, NAME));
                assert_eq!(
                    texts,
                    [(channel.name.clone(), color, NAME_SIZE)],
                    "strip {i} at {h}"
                );
            }
        }
    }
}
