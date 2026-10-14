//! The tempo editor, opened from the transport's BPM button: buttons `-1`, `+1`, `-10`, `+10`, `TAP`, the value, a
//! close X (icon 19), the info text `Time: <byte0>/<byte1>`, the song's `TDIV` bytes: the subdivision, then the beats
//! per bar, and the dial.
//!
//! The dial and TAP: [`dial_bpm`], [`TapTempo`]. The step buttons keep to the dial's range, the main menu's
//! [`TEMPO_RANGE`].
//!
//! The editor is a dialog band like the save dialog, on the dialog base ([`super::dialog`]): the screen dimmed at α 0.6
//! and a `moduleFill` band, the dial at the left of one centred row `(dial) -10 -1 [value] +1 +10 TAP` of light
//! rectangles (as the PROJECT tab's `[-] [140.0 BPM] [+]`), the time signature in small text under the value, and the
//! close X as a light disc at the right. Escape closes too. The dial is a light 2.5 U disc with a `menuFill` dot that
//! turns with it (a full turn per BPM), 0.5 U from the row so that its reach (0.6 × its width) stops short of `-10`;
//! `TAP`'s flash is an `accent` halo; a tap whose clock has not moved on starts a new group.

use std::f64::consts::TAU;

use eframe::egui::{Align2, Id, Pos2, Rect, Sense, Ui, Vec2, pos2, vec2};

use crate::icons::{IconId, draw_icon};
use crate::metrics::{FontSize, Metrics};
use crate::song::TimeSig;
use crate::ui::buttons::filled_circle_button;
use crate::ui::dialog;
use crate::ui::main_menu::{TEMPO_RANGE, tempo_step};
use crate::ui::widgets::{Button, button};
use crate::ui::{UiCtx, primary_down_live};

// Layout, in U.
const BUTTON_W: f32 = 1.5;
const VALUE_W: f32 = 3.5;
const BUTTON_H: f32 = 1.25;
const GAP: f32 = 0.25;
/// The time-signature strip under the value.
const TIME_H: f32 = 0.75;
/// The dial's width, its gap to the row, its dot (diameter, and its centre's distance in from the rim).
const DIAL_D: f32 = 2.5;
const DIAL_GAP: f32 = 0.5;
const DOT_D: f32 = 0.3;
const DOT_IN: f32 = 0.4;
/// A touch this close to the dial's centre, × its width, turns it.
const DIAL_REACH: f32 = 0.6;
/// A touch no child takes, farther than this from the dial's centre, closes the dialog; nearer, it is swallowed.
const CLOSE_DISTANCE: f32 = 7.0;
/// The `TAP` flash's halo pad.
const FLASH_PAD: f32 = 0.5;
/// The close X: diameter, and its centre's distance from the right edge (as the transport's).
const CLOSE_D: f32 = 1.25;
const CLOSE_INSET: f32 = 0.75;
const CLOSE_ICON: f32 = 0.7;

/// The step buttons in their row order, with what each adds.
const STEPS: [(&str, f64); 4] = [("-10", -10.0), ("-1", -1.0), ("+1", 1.0), ("+10", 10.0)];

/// The dial's angle of the finger at `p`: atan2(centre y − y, x − centre x), counter-clockwise from 3 o'clock.
pub fn dial_angle(center: Pos2, p: Pos2) -> f64 {
    f64::from(center.y - p.y).atan2(f64::from(p.x - center.x))
}

/// `bpm` after the finger turned the dial from angle `from` to `to`: −Δ/2π, Δ wrapped to ±½ turn, so a full
/// clockwise turn is +1 BPM; clamped to [`TEMPO_RANGE`], [10, 999].
pub fn dial_bpm(bpm: f64, from: f64, to: f64) -> f64 {
    let turn = ((to - from) / TAU + 0.5).rem_euclid(1.0) - 0.5;
    (bpm - turn).clamp(TEMPO_RANGE.0, TEMPO_RANGE.1)
}

/// `bpm` rounded to a tenth.
fn tenths(bpm: f64) -> f64 {
    (bpm * 10.0).round() / 10.0
}

/// Tap tempo, a group of four taps: the 1st only records its time; the 2nd, 3rd and 4th each add 20/Δt (a third of
/// 60/Δt); the 4th gives the sum, clamped to [`crate::song::TEMPO_RANGE`] (the dial's is [`TEMPO_RANGE`]), and the next
/// tap starts a new group.
#[derive(Clone, Copy, Debug, Default)]
pub struct TapTempo {
    /// The last tap's time; `None` before the group's first.
    last: Option<f64>,
    /// The taps counted in the group so far (0…3): n.
    n: u8,
    sum: f64,
}

impl TapTempo {
    /// Records a tap at `now` (seconds); the group's tempo on its 4th tap.
    pub fn tap(&mut self, now: f64) -> Option<f64> {
        match self.last {
            Some(last) if now > last => {
                self.sum += 20.0 / (now - last);
                self.n += 1;
            }
            _ => {
                *self = Self {
                    n: 1,
                    ..Self::default()
                }
            }
        }
        self.last = Some(now);
        if self.n < 4 {
            return None;
        }
        let (lo, hi) = crate::song::TEMPO_RANGE;
        let bpm = self.sum.clamp(lo, hi);
        *self = Self::default();
        Some(bpm)
    }

    /// The `TAP` button's size factor: 0.8 + 0.05·n.
    pub fn size(&self) -> f32 {
        0.8 + 0.05 * f32::from(self.n)
    }
}

/// The editor's value: `%.1f BPM`, as the PROJECT tab's `140.0 BPM`.
pub fn bpm_text(bpm: f64) -> String {
    format!("{:.1} BPM", tenths(bpm))
}

/// The transport readout: the number alone, without a trailing `.0` (`140`, `92.5`).
pub fn transport_bpm_text(bpm: f64) -> String {
    let bpm = tenths(bpm);
    if bpm.fract() == 0.0 {
        format!("{bpm:.0}")
    } else {
        format!("{bpm:.1}")
    }
}

/// Where the editor's parts go in its band.
#[derive(Debug)]
struct Layout {
    /// The dial's centre.
    dial: Pos2,
    /// In [`STEPS`] order.
    steps: [Rect; 4],
    value: Rect,
    /// `TAP` at size factor 1.
    tap: Rect,
    time: Rect,
    close: Pos2,
}

/// The dial and the row `-10 -1 [value] +1 +10 TAP` right of it, centred in `rect` with the time strip under the
/// value; the close X at the right.
fn layout(rect: Rect, m: &Metrics) -> Layout {
    let widths = [BUTTON_W, BUTTON_W, VALUE_W, BUTTON_W, BUTTON_W, BUTTON_W].map(|w| m.u(w));
    let total = m.u(DIAL_D + DIAL_GAP) + widths.iter().sum::<f32>() + m.u(GAP) * 5.0;
    let top = rect.center().y - m.u(BUTTON_H + TIME_H) / 2.0;
    let left = rect.center().x - total / 2.0;
    let mut x = left + m.u(DIAL_D + DIAL_GAP);
    let row = widths.map(|w| {
        let r = Rect::from_min_size(pos2(x, top), vec2(w, m.u(BUTTON_H)));
        x += w + m.u(GAP);
        r
    });
    let value = row[2];
    Layout {
        dial: pos2(left + m.u(DIAL_D) / 2.0, rect.center().y),
        steps: [row[0], row[1], row[3], row[4]],
        value,
        tap: row[5],
        time: Rect::from_min_size(value.left_bottom(), vec2(value.width(), m.u(TIME_H))),
        close: pos2(rect.right() - m.u(CLOSE_INSET), rect.center().y),
    }
}

/// What the user did in the tempo editor this frame.
#[derive(Clone, Copy, Debug, PartialEq)]
pub enum TempoAction {
    /// A new tempo. `done` ends the gesture (an undo step): it is set for the step buttons and `TAP`, and on the
    /// release that ends a turn of the dial, which sends the final value again.
    Set { bpm: f64, done: bool },
    /// The close X or Escape.
    Close,
}

/// A turn of the dial in progress.
#[derive(Clone, Copy, Debug)]
struct Turn {
    /// The finger's last angle.
    angle: f64,
    /// The tempo it last left: any other means the song changed under it (an undo), which ends it.
    bpm: f64,
    /// It changed the tempo.
    changed: bool,
}

/// The tempo editor's state.
#[derive(Debug, Default)]
pub struct TempoEditor {
    tap: TapTempo,
    /// `TAP`'s flash, 1 on a tap and × 0.8 a frame.
    flash: f32,
    dial: Option<Turn>,
    /// The frame it was last shown in: a gap means it was opened again, which arms `TAP`.
    shown: Option<u64>,
}

impl TempoEditor {
    /// Paints the editor over `rect` for the current tempo `bpm` and time signature `time_sig`. Returns what the
    /// user did: a `Set` on every frame of a turn that changes the value, and a done `Set` when it ends.
    pub fn show(
        &mut self,
        ui: &mut Ui,
        cx: &UiCtx,
        rect: Rect,
        bpm: f64,
        time_sig: TimeSig,
    ) -> Option<TempoAction> {
        let (m, c) = (cx.m, &cx.theme.colors);
        let (text, menu_fill) = (c.text.to_color32(), c.menu_fill.to_color32());
        let id = Id::new("tempo_editor");
        let frame = ui.ctx().cumulative_frame_nr();
        if self.shown.is_none_or(|f| frame > f + 1) {
            *self = Self::default();
        }
        self.shown = Some(frame);
        let l = layout(rect, m);
        let mut set = None;
        // Modal: takes every touch on the screen that no child (added after it, so on top) takes; undo waits.
        crate::ui::modal_shown(ui.ctx());
        let screen = ui.ctx().content_rect();
        let backdrop = ui.interact(screen, id.with("backdrop"), Sense::click_and_drag());
        // On the dialog base: the screen dimmed at α 0.6, then a `moduleFill` band, from the first frame. Drawn last,
        // so on top.
        dialog::paint_base(ui.painter(), ui.painter(), c, screen, rect);

        for ((label, delta), r) in STEPS.into_iter().zip(l.steps) {
            let b = Button::new(id.with(label), r, label, false, c);
            if button(ui, cx, &b, true).clicked() {
                set = Some((tempo_step(bpm, delta), true));
            }
        }
        let tap = Rect::from_center_size(l.tap.center(), l.tap.size() * self.tap.size());
        let mut b = Button::new(id.with("TAP"), tap, "TAP", false, c);
        b.halo = (self.flash > 0.0).then(|| {
            (
                c.accent.to_color32().gamma_multiply(self.flash),
                m.u(FLASH_PAD) * m.ppp(),
            )
        });
        let tapped = button(ui, cx, &b, true).clicked();
        if tapped && let Some(v) = self.tap.tap(ui.input(|i| i.time)) {
            set = Some((v, true));
        }
        // Drawn from the next frame on; gone once it is less than one step of 8-bit alpha.
        self.flash = match self.flash * 0.8 {
            _ if tapped => 1.0,
            f if f < 1.0 / 255.0 => 0.0,
            f => f,
        };
        if self.flash > 0.0 {
            ui.ctx().request_repaint();
        }

        // The dial: a press within its reach starts a turn, which follows the finger until the release.
        let stopped = self.dial(ui, id.with("dial"), l.dial, bpm, &mut set, m);
        let painter = ui.painter();
        let r = m.u(DIAL_D) / 2.0;
        painter.circle_filled(l.dial, r, text);
        let turn = bpm.rem_euclid(1.0) * TAU;
        let at = r - m.u(DOT_IN);
        let dot = l.dial + at * Vec2::new(turn.sin() as f32, -turn.cos() as f32);
        painter.circle_filled(dot, m.u(DOT_D) / 2.0, menu_fill);

        // The value is a button that does nothing: it takes its touch, so it never closes the dialog.
        ui.interact(l.value, id.with("value"), Sense::click());
        painter.rect_filled(l.value, 0.0, text);
        let font = m.font_id(FontSize::Pt10);
        painter.text(
            l.value.center(),
            Align2::CENTER_CENTER,
            bpm_text(bpm),
            font,
            menu_fill,
        );
        let TimeSig {
            subdivision,
            beats_per_bar,
        } = time_sig;
        painter.text(
            l.time.center(),
            Align2::CENTER_CENTER,
            format!("Time: {subdivision}/{beats_per_bar}"),
            m.font_id(FontSize::Pt8),
            text,
        );

        let close = filled_circle_button(ui, id.with("close"), l.close, m.u(CLOSE_D), text);
        let square = Rect::from_center_size(l.close, vec2(m.u(CLOSE_D), m.u(CLOSE_D)));
        draw_icon(
            ui.painter(),
            square,
            IconId::Close,
            c.background.to_color32(),
            CLOSE_ICON,
            m,
        );
        let escape = crate::ui::close_pressed(ui.ctx());
        let outside = backdrop.clicked()
            && (backdrop.interact_pointer_pos())
                .is_some_and(|p| p.distance(l.dial) > m.u(CLOSE_DISTANCE));
        if close.clicked() || escape || outside {
            return Some(TempoAction::Close);
        }
        set.filter(|&(v, _)| v != bpm || stopped)
            .map(|(bpm, done)| TempoAction::Set { bpm, done })
    }

    /// A turn of the dial centred at `center` for the tempo `bpm`: each move sets −Δangle/2π more ([`dial_bpm`]), and
    /// the release that ends a turn that changed it sets the value again, done. True on that release. A tempo the turn
    /// did not leave ends it at once, with nothing more.
    fn dial(
        &mut self,
        ui: &mut Ui,
        id: Id,
        center: Pos2,
        bpm: f64,
        set: &mut Option<(f64, bool)>,
        m: &Metrics,
    ) -> bool {
        let reach = DIAL_REACH * m.u(DIAL_D);
        let hit = Rect::from_center_size(center, Vec2::splat(2.0 * reach));
        let response = ui.interact(hit, id, Sense::click_and_drag());
        let down = primary_down_live(ui, id);
        let (pressed, pos) = ui.input(|i| (i.pointer.primary_pressed(), i.pointer.interact_pos()));
        if pressed
            && response.is_pointer_button_down_on()
            && let Some(p) = pos.filter(|p| p.distance(center) <= reach)
        {
            self.dial = Some(Turn {
                angle: dial_angle(center, p),
                bpm,
                changed: false,
            });
        }
        if self.dial.is_some_and(|turn| turn.bpm != bpm) {
            self.dial = None;
        }
        let Some(turn) = &mut self.dial else {
            return false;
        };
        if let Some(p) = pos.filter(|_| down) {
            let angle = dial_angle(center, p);
            let v = dial_bpm(bpm, turn.angle, angle);
            turn.angle = angle;
            if v != bpm {
                (turn.bpm, turn.changed) = (v, true);
                *set = Some((v, false));
            }
        }
        if down {
            return false;
        }
        let changed = turn.changed;
        self.dial = None;
        if changed {
            *set = Some((bpm, true));
        }
        changed
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    use crate::ui::offscreen::{Offscreen, SCREEN, metrics};
    use eframe::egui::{self, Color32, Shape};

    /// A 3 U band across the middle of the screen.
    const BAND: Rect = Rect {
        min: pos2(0.0, 222.0),
        max: pos2(1170.0, 318.0),
    };

    struct Rig {
        screen: Offscreen,
        editor: TempoEditor,
        bpm: f64,
        actions: Vec<TempoAction>,
        shapes: Vec<egui::epaint::ClippedShape>,
        /// The clicks that got through to a widget under the editor, over the whole screen.
        under: usize,
    }

    impl Rig {
        fn new(bpm: f64) -> Self {
            let mut rig = Self {
                screen: Offscreen::new(),
                editor: TempoEditor::default(),
                bpm,
                actions: Vec::new(),
                shapes: Vec::new(),
                under: 0,
            };
            rig.frame();
            rig
        }

        /// Runs `input` on the screen with a frame closure that shows the editor and applies `Set`.
        fn run(&mut self, input: impl FnOnce(&mut Offscreen, &mut dyn FnMut(&mut Ui, &UiCtx))) {
            let (editor, bpm, actions) = (&mut self.editor, &mut self.bpm, &mut self.actions);
            let under = &mut self.under;
            input(&mut self.screen, &mut |ui, cx| {
                *under += usize::from(
                    ui.interact(SCREEN, Id::new("under"), Sense::click())
                        .clicked(),
                );
                let sig = TimeSig {
                    subdivision: 3,
                    beats_per_bar: 4,
                };
                if let Some(action) = editor.show(ui, cx, BAND, *bpm, sig) {
                    if let TempoAction::Set { bpm: v, .. } = action {
                        *bpm = v;
                    }
                    actions.push(action);
                }
            });
        }

        fn frame(&mut self) {
            let mut shapes = Vec::new();
            self.run(|s, f| shapes = s.frame(vec![], f));
            self.shapes = shapes;
        }

        fn click(&mut self, pos: Pos2) {
            self.run(|s, f| s.click(pos, f));
        }

        /// Presses on `points[0]`, moves through the rest one frame each and lets go on the last.
        fn turn(&mut self, points: &[Pos2]) {
            let offsets: Vec<Vec2> = points[1..].iter().map(|&p| p - points[0]).collect();
            self.run(|s, f| s.drag(points[0], &offsets, f));
        }

        fn layout(&self) -> Layout {
            layout(BAND, &metrics())
        }

        /// The points a finger 0.8 dial radius out passes, from 3 o'clock round clockwise (as seen) `quarters`
        /// quarter turns, or anticlockwise for negative ones.
        fn round(&self, quarters: i32) -> Vec<Pos2> {
            let (c, r) = (self.layout().dial, 0.8 * metrics().u(DIAL_D) / 2.0);
            let compass = [vec2(r, 0.0), vec2(0.0, r), vec2(-r, 0.0), vec2(0.0, -r)];
            let at = |q: i32| c + compass[q.rem_euclid(4) as usize];
            let step = quarters.signum();
            (0..=quarters.abs()).map(|i| at(i * step)).collect()
        }

        /// The `done` flags of the `Set`s so far.
        fn dones(&self) -> Vec<bool> {
            (self.actions.iter())
                .map(|a| match a {
                    TempoAction::Set { done, .. } => *done,
                    TempoAction::Close => panic!("closed"),
                })
                .collect()
        }

        /// The `TAP` button's rect as drawn the frame before the last.
        fn tap_rect(&self) -> Rect {
            let id = Id::new("tempo_editor").with("TAP");
            self.screen.ctx.read_response(id).expect("TAP").rect
        }

        /// The alpha of the last frame's halo round `TAP` (0.25 U out), if it has one; it is in `accent`.
        fn flash(&self) -> Option<u8> {
            let tap = self.layout().tap;
            let tap = Rect::from_center_size(tap.center(), tap.size() * self.editor.tap.size());
            let (halo, accent) = (tap.expand(8.0), theme_accent());
            self.shapes.iter().find_map(|s| match &s.shape {
                Shape::Rect(r) if r.rect == halo => {
                    let want = accent.gamma_multiply(f32::from(r.fill.a()) / 255.0);
                    assert!(r.fill.r().abs_diff(want.r()) <= 1, "{:?}", r.fill);
                    Some(r.fill.a())
                }
                _ => None,
            })
        }
    }

    fn theme_accent() -> Color32 {
        crate::theme::theme_by_id(0).colors.accent.to_color32()
    }

    /// Size factors, to f32 precision (of a rect's width, read back).
    fn approx(a: f32, b: f32) -> bool {
        (a - b).abs() < 1e-5
    }

    #[test]
    fn the_dial_sits_left_of_one_centred_row_with_the_close_x_at_the_right() {
        let m = metrics();
        let l = layout(BAND, &m);
        let row = [
            l.steps[0], l.steps[1], l.value, l.steps[2], l.steps[3], l.tap,
        ];
        for pair in row.windows(2) {
            assert!(
                (pair[1].left() - pair[0].right() - m.u(0.25)).abs() < 1e-3,
                "{pair:?}"
            );
            assert_eq!(pair[0].center().y, pair[1].center().y);
        }
        // The 2.5 U dial, 0.5 U left of `-10`; dial and row centred together.
        assert_eq!(l.dial.y, BAND.center().y);
        assert_eq!(row[0].left() - (l.dial.x + m.u(1.25)), m.u(0.5));
        assert!((l.dial.x - m.u(1.25) + row[5].right() - 2.0 * BAND.center().x).abs() < 1e-3);
        // Its reach, 0.6 × its width, stops short of `-10`.
        assert!(l.dial.x + 0.6 * m.u(2.5) < row[0].left());
        assert_eq!(l.value.width(), m.u(3.5));
        assert_eq!(l.tap.size(), vec2(m.u(1.5), m.u(1.25)));
        // The time text under the value; the block of both is centred vertically.
        assert_eq!(l.time.top(), l.value.bottom());
        assert!((l.value.top() + l.time.bottom() - 2.0 * BAND.center().y).abs() < 1e-3);
        assert_eq!(l.close, pos2(BAND.right() - m.u(0.75), BAND.center().y));
        for r in row {
            assert!(BAND.contains_rect(r));
        }
        assert!(BAND.contains_rect(Rect::from_center_size(l.dial, Vec2::splat(m.u(2.5)))));
    }

    #[test]
    fn the_step_buttons_add_and_clamp() {
        let mut rig = Rig::new(140.0);
        let l = rig.layout();
        for (i, want) in [(3, 150.0), (2, 151.0), (1, 150.0), (0, 140.0)] {
            rig.click(l.steps[i].center());
            assert_eq!(rig.bpm, want, "button {i}");
        }
        // The dial's range: up to 999 (a dialled 450 stays 451 on `+1`).
        rig.bpm = 450.0;
        rig.click(l.steps[2].center());
        assert_eq!(rig.bpm, 451.0);
        rig.bpm = 995.0;
        rig.click(l.steps[3].center());
        assert_eq!(rig.bpm, 999.0);
        rig.bpm = 15.0;
        rig.click(l.steps[0].center());
        assert_eq!(rig.bpm, 10.0);
        // A step that changes nothing (at the top of the range) says nothing.
        rig.bpm = TEMPO_RANGE.1;
        rig.actions.clear();
        rig.click(l.steps[3].center());
        assert_eq!(rig.actions, []);
    }

    #[test]
    fn the_dial_turns_minus_the_angle_over_two_pi_wrapped_to_half_a_turn_and_clamps() {
        use std::f64::consts::{FRAC_PI_2, PI};
        let c = pos2(100.0, 100.0);
        // atan2(centre y − y, x − centre x): up is +π/2, down −π/2.
        assert_eq!(dial_angle(c, pos2(110.0, 100.0)), 0.0);
        assert_eq!(dial_angle(c, pos2(100.0, 90.0)), FRAC_PI_2);
        assert_eq!(dial_angle(c, pos2(100.0, 110.0)), -FRAC_PI_2);
        // Clockwise (as seen, y down) is +: a quarter turn from 3 to 6 o'clock is +0.25 BPM.
        assert_eq!(dial_bpm(140.0, 0.0, -FRAC_PI_2), 140.25);
        assert_eq!(dial_bpm(140.0, -FRAC_PI_2, 0.0), 139.75);
        // Across ±π the step is the short way round: from just above 9 o'clock to just below it.
        let d = dial_bpm(140.0, PI - 0.1, -PI + 0.1);
        assert!((d - (140.0 - 0.2 / TAU)).abs() < 1e-12, "{d}");
        // Clamped to [10, 999].
        assert_eq!(dial_bpm(998.9, 0.0, -FRAC_PI_2), 999.0);
        assert_eq!(dial_bpm(10.1, -FRAC_PI_2, 0.0), 10.0);
    }

    #[test]
    fn a_full_clockwise_turn_of_the_dial_adds_one_bpm_and_its_release_is_done() {
        let mut rig = Rig::new(140.0);
        let points = rig.round(4);
        rig.turn(&points);
        assert!((rig.bpm - 141.0).abs() < 1e-9, "{}", rig.bpm);
        // A change on each of the four moves, then the release re-sends the final value as done.
        assert_eq!(rig.dones(), [false, false, false, false, true]);
        // Half a turn back.
        rig.actions.clear();
        let points = rig.round(-2);
        rig.turn(&points);
        assert!((rig.bpm - 140.5).abs() < 1e-9, "{}", rig.bpm);
        assert_eq!(rig.dones(), [false, false, true]);
        // A touch and release that turn nothing set nothing.
        rig.actions.clear();
        let at = rig.layout().dial + vec2(30.0, 0.0);
        rig.click(at);
        assert_eq!(rig.actions, []);
    }

    #[test]
    fn the_dial_takes_touches_within_six_tenths_of_its_width_of_its_centre() {
        let mut rig = Rig::new(140.0);
        let c = rig.layout().dial;
        // 0.6 × 2.5 U = 48 points out, along the diagonal: just inside turns, just outside does not.
        let diagonal = |d: f32| c + vec2(d, d) / 2f32.sqrt();
        // A quarter turn clockwise as seen (y down): (x, y) → (−y, x) about the centre.
        let quarter = |from: Pos2| [from, c + vec2(c.y - from.y, from.x - c.x)];
        rig.turn(&quarter(diagonal(47.0)));
        assert!((rig.bpm - 140.25).abs() < 1e-6, "{}", rig.bpm);
        rig.turn(&quarter(diagonal(49.0)));
        assert!((rig.bpm - 140.25).abs() < 1e-6, "{}", rig.bpm);
    }

    #[test]
    fn a_touch_that_hits_nothing_closes_the_dialog_only_over_seven_u_from_the_dials_centre() {
        let mut rig = Rig::new(140.0);
        let (c, m) = (rig.layout().dial, metrics());
        // Past the dial's reach (0.6 × its width) and within 7 U: swallowed. In the band's gap to `-10`, in the
        // dial's square corner, above and below the band (y 222..318), and out to the left of the band's row.
        let diagonal = |d: f32| c + vec2(d, d) / 2f32.sqrt();
        let swallowed = [
            ("gap to -10", c + vec2(m.u(1.6), 0.0)),
            ("corner", diagonal(49.0)),
            ("above the band", c - vec2(0.0, m.u(4.0))),
            ("below the band", c + vec2(0.0, m.u(4.0))),
            ("left, 6.9 U", c - vec2(m.u(6.9), 0.0)),
            ("above, 6.9 U", c - vec2(0.0, m.u(6.9))),
            ("diagonal, 6.9 U", diagonal(m.u(6.9))),
        ];
        for (what, at) in swallowed {
            rig.click(at);
            assert_eq!(rig.actions, [], "{what}");
            assert_eq!(rig.bpm, 140.0, "{what}");
        }
        // None of it reached what is under the dialog.
        assert_eq!(rig.under, 0);
        // Over 7 U: the dialog closes.
        let closing = [
            ("left, 7.1 U", c - vec2(m.u(7.1), 0.0)),
            ("above, 7.1 U", c - vec2(0.0, m.u(7.1))),
            ("below, 7.1 U", c + vec2(0.0, m.u(7.1))),
            ("diagonal, 7.1 U", diagonal(m.u(7.1))),
            ("the far corner", pos2(1160.0, 10.0)),
        ];
        for (what, at) in closing {
            rig.click(at);
            assert_eq!(rig.actions, [TempoAction::Close], "{what}");
            rig.actions.clear();
        }
        assert_eq!(rig.under, 0);
    }

    #[test]
    fn the_child_buttons_get_a_touch_first_so_far_from_the_dial_they_do_not_close_it() {
        let mut rig = Rig::new(140.0);
        let l = rig.layout();
        // `+10` and `TAP` are over 7 U from the dial's centre: they take the touch and nothing closes.
        assert!(l.steps[3].center().distance(l.dial) > metrics().u(7.0));
        rig.click(l.steps[3].center());
        assert_eq!(
            rig.actions,
            [TempoAction::Set {
                bpm: 150.0,
                done: true
            }]
        );
        rig.actions.clear();
        rig.click(l.tap.center());
        assert_eq!(rig.actions, []);
        // So does the value, which does nothing: its right half is over 7 U out.
        let value = l.value.right_center() - vec2(4.0, 0.0);
        assert!(value.distance(l.dial) > metrics().u(7.0));
        rig.click(value);
        assert_eq!(rig.actions, []);
        assert_eq!(rig.under, 0);
        // A touch that starts on a button and ends over nothing is no tap on nothing.
        rig.run(|s, f| s.drag(l.steps[3].center(), &[vec2(0.0, 90.0)], f));
        assert_eq!(rig.actions, []);
        // The dial takes its touch inside 0.6 × its width and the dialog stays.
        rig.click(l.dial + vec2(30.0, 0.0));
        assert_eq!(rig.actions, []);
    }

    #[test]
    fn four_taps_set_the_mean_of_their_three_tempos_and_the_next_tap_starts_over() {
        let mut tap = TapTempo::default();
        assert!(approx(tap.size(), 0.8));
        // 0.5 s, 0.4 s and 0.6 s apart: 40 + 50 + 33.3.
        let times = [10.0, 10.5, 10.9, 11.5];
        let got: Vec<_> = times.into_iter().map(|t| tap.tap(t)).collect();
        assert_eq!(got[..3], [None, None, None]);
        assert!((got[3].unwrap() - (40.0 + 50.0 + 20.0 / 0.6)).abs() < 1e-9);
        // The counter restarts: the next tap only records.
        assert!(approx(tap.size(), 0.8));
        for (t, size) in [(30.0, 0.85), (30.1, 0.9), (30.2, 0.95)] {
            assert_eq!(tap.tap(t), None);
            assert!(approx(tap.size(), size), "{}", tap.size());
        }
        // 600 BPM.
        assert!((tap.tap(30.3).unwrap() - 600.0).abs() < 1e-9);
        // Clamped to [1, 999].
        let mut tap = TapTempo::default();
        let fast: Vec<_> = (0..4).map(|i| tap.tap(f64::from(i) * 0.01)).collect();
        assert_eq!(fast[3], Some(999.0));
        let slow: Vec<_> = (0..4).map(|i| tap.tap(f64::from(i) * 100.0)).collect();
        assert_eq!(slow[3], Some(1.0));
        // A clock that has not moved on starts the group again.
        assert_eq!(tap.tap(5.0), None);
        assert_eq!(tap.tap(5.0), None);
        assert!(approx(tap.size(), 0.85));
    }

    #[test]
    fn tap_grows_with_each_tap_of_a_group_and_flashes_and_opening_arms_it() {
        let mut rig = Rig::new(140.0);
        let base = rig.layout().tap;
        let size = |rig: &Rig| rig.tap_rect().width() / base.width();
        assert!(approx(size(&rig), 0.8));
        rig.click(base.center());
        // egui reads back the rects of the frame before.
        rig.frame();
        rig.frame();
        assert!(approx(size(&rig), 0.85), "{}", size(&rig));
        // The flash: drawn at 1 the frame after the tap, then × 0.8 a frame.
        let mut rig = Rig::new(140.0);
        rig.click(base.center());
        let mut flashes = Vec::new();
        for _ in 0..3 {
            rig.frame();
            flashes.push(rig.flash());
        }
        assert_eq!(flashes, [Some(255), Some(204), Some(163)]);
        for _ in 0..30 {
            rig.frame();
        }
        assert_eq!(rig.flash(), None);
        // Two taps, then the editor is closed for a frame: reopened, the group starts again.
        let mut rig = Rig::new(140.0);
        rig.click(base.center());
        rig.screen.time += 0.5;
        rig.click(base.center());
        rig.run(|s, _| {
            s.frame(vec![], &mut |_, _| {});
        });
        // A click's release is three frames after the last one's.
        for _ in 0..3 {
            rig.screen.time += 0.5 - 3.0 / 60.0;
            rig.click(base.center());
            assert_eq!(rig.bpm, 140.0);
        }
        // The fourth tap of the new group: three taps 0.5 s apart, 120 BPM.
        rig.screen.time += 0.5 - 3.0 / 60.0;
        rig.click(base.center());
        assert!((rig.bpm - 120.0).abs() < 1e-6, "{}", rig.bpm);
        assert_eq!(rig.dones(), [true]);
    }

    #[test]
    fn the_close_x_and_escape_close() {
        let mut rig = Rig::new(140.0);
        let at = rig.layout().close;
        rig.click(at);
        assert_eq!(rig.actions, [TempoAction::Close]);
        let escape = egui::Event::Key {
            key: egui::Key::Escape,
            physical_key: None,
            pressed: true,
            repeat: false,
            modifiers: Default::default(),
        };
        rig.run(|s, f| {
            s.frame(vec![escape], f);
        });
        assert_eq!(rig.actions, [TempoAction::Close, TempoAction::Close]);
    }

    #[test]
    fn the_value_and_the_time_signature_are_shown() {
        // `Time: <byte0>/<byte1>`: the rig's song has subdivision 3 (byte 0), 4 beats a bar (byte 1).
        let mut rig = Rig::new(92.5);
        rig.frame();
        let texts: Vec<String> = Offscreen::texts(&rig.shapes)
            .into_iter()
            .map(|(t, _)| t)
            .collect();
        for want in ["-10", "-1", "92.5 BPM", "+1", "+10", "TAP", "Time: 3/4"] {
            assert!(texts.iter().any(|t| t == want), "{want} in {texts:?}");
        }
    }

    /// The dialog base: screen dim α 0.6, then a `moduleFill` box, from the first frame (the tempo dialog is a dialog
    /// band like the Save dialog's).
    #[test]
    fn the_dialog_base_dims_the_screen_then_fills_the_band_with_module_fill() {
        // The rig's first frame.
        let rig = Rig::new(140.0);
        let painted = crate::ui::dialog::painted_rects(rig.shapes.iter().map(|s| &s.shape));
        let colors = crate::theme::theme_by_id(0).colors;
        crate::ui::dialog::assert_base(&painted, &colors, SCREEN, BAND);
        // The band takes no other fill: the old `menuFill` band is gone from the dialog.
        let menu_fill = colors.menu_fill.to_color32();
        assert!(!painted.iter().any(|&(r, f)| r == BAND && f == menu_fill));
    }

    #[test]
    fn the_editor_shows_one_decimal_and_the_transport_drops_a_zero_one() {
        assert_eq!(bpm_text(140.0), "140.0 BPM");
        assert_eq!(bpm_text(99.56), "99.6 BPM");
        // Both round halves up, as one value: not `{:.1}`'s ties to even.
        assert_eq!(bpm_text(140.25), "140.3 BPM");
        assert_eq!(transport_bpm_text(140.25), "140.3");
        assert_eq!(transport_bpm_text(140.0), "140");
        assert_eq!(transport_bpm_text(140.04), "140");
        assert_eq!(transport_bpm_text(92.5), "92.5");
    }
}
