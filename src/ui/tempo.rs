//! The tempo dialog, opened from the transport's BPM button. It covers the screen, dimmed to `background` α 0.8, with
//! no open or close animation. Round the screen's centre C:
//! - a `background` disc 7.5 U in radius;
//! - the dial: a `text` ring (outer radius 3.75 U, 6 device px wide, easing outward to 12 px while it turns); inside it
//!   the tempo ([`bpm_text`], FontSize 4) over `BPM` and `Time: <byte0>/<byte1>` (the song's `TDIV` bytes, FontSize 1);
//!   and a `text` dot 2.85 U out, frac(BPM) of a turn clockwise from 3 o'clock (0.4 U, 0.5 U while the dial turns);
//! - light discs `-1`, `+1` (1.4 U) and `-10`, `+10` (1.75 U) on the upper sides, labels `menuFill` FontSize 4;
//! - below the ring, the metronome toggle (icon 47, `TempoTap`, 1.4 U) and `TAP` (1.75 U × [`TapTempo::size`]):
//!   `controlFill` discs in a 2 px `text` outline, `TAP` in FontSize 1; the metronome on is a `text` disc with a
//!   `menuFill` icon;
//! - the close X at the screen's top-right corner: a 1.4 U `text` disc in a 2 px `moduleFill` outline, its X in
//!   `moduleFill`, centred in a 2 U box.
//!
//! The buttons sit on slots 22.5° apart, 5.125 U from C, and fire on release. A touch-down within 4.5 U of C turns the
//! dial ([`dial_bpm`]), before any button takes it. The steps round ([`step_bpm`]); `TAP` is [`TapTempo`]. A tap outside
//! the disc that no button takes closes the dialog, as do the X, Escape and a focus loss; one on the disc does nothing.

use std::f64::consts::TAU;

use eframe::egui::{Align2, Id, Pos2, Rect, Response, Sense, Stroke, Ui, Vec2, pos2, vec2};

use crate::icons::IconId;
use crate::metrics::{FontSize, Metrics};
use crate::song::TimeSig;
use crate::ui::main_menu::TEMPO_RANGE;
use crate::ui::widgets::{Button, button};
use crate::ui::{UiCtx, anim, focus_lost, let_go_press, primary_down_live};

// Sizes in U, line widths in device pixels.
const DIM_ALPHA: f32 = 0.8;
const DISC_R: f32 = 7.5;
/// The ring's outer radius, and its width at rest and while the dial turns (it grows outward).
const RING_R: f32 = 3.75;
const RING_PX: f32 = 6.0;
const RING_TURNING_PX: f32 = 12.0;
/// The ring's ease keeps this much of the way each 1/60 s: e^(−6.33/60).
const RING_KEEP: f32 = 0.9;
/// A touch-down this close to C turns the dial.
const DIAL_REACH: f32 = 4.5;
const DOT_AT: f32 = 2.85;
const DOT_R: f32 = 0.4;
const DOT_TURNING_R: f32 = 0.5;
/// The centres of `BPM` and of the time signature, below C.
const BPM_Y: f32 = 0.75;
const TIME_Y: f32 = 1.75;
const OUTLINE_PX: f32 = 2.0;
const CLOSE_BOX: f32 = 2.0;
const CLOSE_D: f32 = 1.4;

/// The step buttons: label, step, centre from C (y down) and diameter.
const STEPS: [(&str, f64, Vec2, f32); 4] = [
    ("-1", -1.0, vec2(-4.26, -2.85), 1.4),
    ("+1", 1.0, vec2(4.26, -2.85), 1.4),
    ("-10", -10.0, vec2(-5.03, -1.0), 1.75),
    ("+10", 10.0, vec2(5.03, -1.0), 1.75),
];
/// The metronome toggle's and `TAP`'s centres from C, and diameters (`TAP`'s at size factor 1).
const METRONOME: (Vec2, f32) = (vec2(-2.85, 4.26), 1.4);
const TAP: (Vec2, f32) = (vec2(2.85, 4.26), 1.75);

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

/// `-1`, `+1`, `-10`, `+10`: `bpm + step` rounded (halves away from zero), kept in [`TEMPO_RANGE`]. So 140.5 + 1 is
/// 142 and 140.4 + 1 is 141.
pub fn step_bpm(bpm: f64, step: f64) -> f64 {
    (bpm + step).round().clamp(TEMPO_RANGE.0, TEMPO_RANGE.1)
}

/// `bpm` rounded to a tenth.
fn tenths(bpm: f64) -> f64 {
    (bpm * 10.0).round() / 10.0
}

/// Tap tempo, a group of four taps: the 1st only records its time; the 2nd, 3rd and 4th each add 20/Δt (a third of
/// 60/Δt); the 4th gives the sum, clamped to [`crate::song::TEMPO_RANGE`] (the dial's is [`TEMPO_RANGE`]), and the next
/// tap starts a new group. No timeout.
#[derive(Clone, Copy, Debug, Default)]
pub struct TapTempo {
    /// The last tap's time; `None` before the group's first.
    last: Option<f64>,
    /// The taps counted in the group so far (0…3).
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
            // A clock that has not moved on starts the group again.
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

    /// The `TAP` button's size factor, 0.8 + 0.05 × the tap count, which is 1 before the group's first tap: 0.85 idle,
    /// 1 before its 4th.
    pub fn size(&self) -> f32 {
        0.8 + 0.05 * f32::from(self.n + 1)
    }
}

/// The dial's value: `%.1f` of the tempo truncated to tenths, so 140.99 shows `140.9`.
pub fn bpm_text(bpm: f64) -> String {
    format!("{:.1}", (bpm * 10.0).trunc() / 10.0)
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

/// What the user did in the tempo dialog this frame.
#[derive(Clone, Copy, Debug, PartialEq)]
pub enum TempoAction {
    /// A new tempo. `done` ends the gesture (an undo step): it is set for the step buttons and `TAP`, and on the
    /// release that ends a turn of the dial, which sends the final value again.
    Set { bpm: f64, done: bool },
    /// The metronome button: the metronome flag flips.
    Metronome,
    /// The close X, Escape, a focus loss or a tap off the disc.
    Close,
}

/// What a button does.
#[derive(Clone, Copy)]
enum Hit {
    Step(f64),
    Metronome,
    Tap,
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

/// The tempo dialog's state.
#[derive(Debug, Default)]
pub struct TempoEditor {
    tap: TapTempo,
    dial: Option<Turn>,
    /// The ring's width in device pixels; 0 before the first frame.
    ring: f32,
    /// The frame it was last shown in: a gap means it was opened again, which arms `TAP`.
    shown: Option<u64>,
}

impl TempoEditor {
    /// Paints the dialog centred on `screen` for the tempo `bpm`, the time signature `time_sig` and the metronome flag
    /// `metronome`. Returns what the user did: a `Set` on every frame of a turn that changes the value, and a done
    /// `Set` when it ends.
    pub fn show(
        &mut self,
        ui: &mut Ui,
        cx: &UiCtx,
        screen: Rect,
        bpm: f64,
        time_sig: TimeSig,
        metronome: bool,
    ) -> Option<TempoAction> {
        let (m, c) = (cx.m, &cx.theme.colors);
        let (text, control_fill) = (c.text.to_color32(), c.control_fill.to_color32());
        let id = Id::new("tempo_editor");
        let frame = ui.ctx().cumulative_frame_nr();
        if self.shown.is_none_or(|f| frame > f + 1) {
            *self = Self::default();
        }
        self.shown = Some(frame);
        let center = screen.center();
        let disc = |(offset, d): (Vec2, f32)| {
            Rect::from_center_size(center + m.u(1.0) * offset, Vec2::splat(m.u(d)))
        };

        // Modal: takes every touch on the screen that no button (added after it, so on top) takes; undo waits.
        crate::ui::modal_shown(ui.ctx());
        let whole = ui.ctx().content_rect();
        let backdrop = ui.interact(whole, id.with("backdrop"), Sense::click_and_drag());
        let background = c.background.to_color32();
        ui.painter()
            .rect_filled(whole, 0.0, anim::dim(background, DIM_ALPHA));
        ui.painter().circle_filled(center, m.u(DISC_R), background);

        let mut hits: Vec<(Response, Hit)> = Vec::with_capacity(STEPS.len() + 2);
        for (label, step, offset, d) in STEPS {
            let b = Button {
                font: FontSize::Pt18,
                ..Button::new(id.with(label), disc((offset, d)), label, true, c)
            };
            hits.push((button(ui, cx, &b, true), Hit::Step(step)));
        }
        let b = Button {
            icon: Some(IconId::TempoTap),
            fill_off: control_fill,
            halo: (!metronome).then_some((text, OUTLINE_PX)),
            ..Button::new(id.with("metronome"), disc(METRONOME), "", true, c)
        };
        hits.push((button(ui, cx, &b, metronome), Hit::Metronome));
        crate::ui::tooltip::tip(ui, b.rect, "Metronome", None);
        let (offset, d) = TAP;
        let b = Button {
            font: FontSize::Pt8,
            fill_off: control_fill,
            halo: Some((text, OUTLINE_PX)),
            ..Button::new(
                id.with("TAP"),
                disc((offset, d * self.tap.size())),
                "TAP",
                true,
                c,
            )
        };
        hits.push((button(ui, cx, &b, false), Hit::Tap));

        // The dial takes a touch-down within its reach before the buttons: one that went down on a button there neither
        // fires it nor lights it.
        let on_dialog = std::iter::once(&backdrop)
            .chain(hits.iter().map(|(r, _)| r))
            .any(Response::is_pointer_button_down_on);
        let mut set = None;
        let (turning, stopped) =
            self.dial(ui, id.with("dial"), center, bpm, on_dialog, &mut set, m);
        let mut toggled = false;
        for (response, hit) in &hits {
            if turning {
                let_go_press(ui.ctx(), response.id);
                continue;
            }
            if !response.clicked() {
                continue;
            }
            match *hit {
                Hit::Step(step) => set = Some((step_bpm(bpm, step), true)),
                Hit::Metronome => toggled = true,
                Hit::Tap => {
                    if let Some(v) = self.tap.tap(ui.input(|i| i.time)) {
                        set = Some((v, true));
                    }
                }
            }
        }

        self.paint_dial(ui, center, bpm, time_sig, cx);

        let module_fill = c.module_fill.to_color32();
        let corner = pos2(screen.right() - m.u(CLOSE_BOX), screen.top());
        let b = Button {
            icon: Some(IconId::Close),
            shape: CLOSE_D / CLOSE_BOX,
            halo: Some((module_fill, OUTLINE_PX)),
            text_on: module_fill,
            ..Button::new(
                id.with("close"),
                Rect::from_min_size(corner, Vec2::splat(m.u(CLOSE_BOX))),
                "",
                true,
                c,
            )
        };
        let close = button(ui, cx, &b, true).clicked();
        let escape = crate::ui::close_pressed(ui.ctx());
        let outside = backdrop.clicked()
            && (backdrop.interact_pointer_pos()).is_some_and(|p| p.distance(center) > m.u(DISC_R));
        if close || escape || outside || ui.input(focus_lost) {
            return Some(TempoAction::Close);
        }
        if toggled {
            return Some(TempoAction::Metronome);
        }
        set.filter(|&(v, _)| v != bpm || stopped)
            .map(|(bpm, done)| TempoAction::Set { bpm, done })
    }

    /// The ring, the texts inside it and the dot, thicker and bigger while the dial turns.
    fn paint_dial(&mut self, ui: &Ui, center: Pos2, bpm: f64, time_sig: TimeSig, cx: &UiCtx) {
        let (m, text) = (cx.m, cx.theme.colors.text.to_color32());
        let turning = self.dial.is_some();
        let target = if turning { RING_TURNING_PX } else { RING_PX };
        self.ring = if self.ring == 0.0 {
            target
        } else {
            self.ring + (target - self.ring) * anim::step(ui.ctx(), RING_KEEP)
        };
        if (self.ring - target).abs() < 0.05 {
            self.ring = target;
        } else {
            ui.ctx().request_repaint();
        }
        let painter = ui.painter();
        let (width, inner) = (self.ring / m.ppp(), m.u(RING_R) - RING_PX / m.ppp());
        painter.circle_stroke(center, inner + width / 2.0, Stroke::new(width, text));

        let label = |dy: f32, s: String, size: FontSize| {
            let at = center + vec2(0.0, m.u(dy));
            painter.text(at, Align2::CENTER_CENTER, s, m.font_id(size), text);
        };
        label(0.0, bpm_text(bpm), FontSize::Pt18);
        label(BPM_Y, "BPM".into(), FontSize::Pt8);
        let TimeSig {
            subdivision,
            beats_per_bar,
        } = time_sig;
        let time = format!("Time: {subdivision}/{beats_per_bar}");
        label(TIME_Y, time, FontSize::Pt8);

        let turn = bpm.rem_euclid(1.0) * TAU;
        let dot = center + m.u(DOT_AT) * vec2(turn.cos() as f32, turn.sin() as f32);
        let r = if turning { DOT_TURNING_R } else { DOT_R };
        painter.circle_filled(dot, m.u(r), text);
    }

    /// A turn of the dial centred at `center` for the tempo `bpm`, started by a touch-down on the dialog (`on_dialog`)
    /// within its reach: each move sets −Δangle/2π more ([`dial_bpm`]), and the release that ends a turn that changed it
    /// sets the value again, done. Returns whether a turn ran this frame, and whether it ended, changed, on this
    /// frame's release. A tempo the turn did not leave ends it at once, with nothing more.
    #[allow(clippy::too_many_arguments)]
    fn dial(
        &mut self,
        ui: &Ui,
        id: Id,
        center: Pos2,
        bpm: f64,
        on_dialog: bool,
        set: &mut Option<(f64, bool)>,
        m: &Metrics,
    ) -> (bool, bool) {
        let down = primary_down_live(ui, id);
        let (pressed, pos) = ui.input(|i| (i.pointer.primary_pressed(), i.pointer.interact_pos()));
        if pressed
            && on_dialog
            && let Some(p) = pos.filter(|p| p.distance(center) <= m.u(DIAL_REACH))
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
            return (false, false);
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
            return (true, false);
        }
        let changed = turn.changed;
        self.dial = None;
        if changed {
            *set = Some((bpm, true));
        }
        (true, changed)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    use crate::ui::offscreen::{Offscreen, SCREEN, metrics};
    use eframe::egui;

    /// The screen's centre, C.
    const C: Pos2 = pos2(585.0, 270.0);

    struct Rig {
        screen: Offscreen,
        editor: TempoEditor,
        bpm: f64,
        actions: Vec<TempoAction>,
        shapes: Vec<egui::epaint::ClippedShape>,
        /// The clicks that got through to a widget under the dialog, over the whole screen.
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

        /// Runs `input` on the screen with a frame closure that shows the dialog and applies `Set`.
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
                if let Some(action) = editor.show(ui, cx, SCREEN, *bpm, sig, false) {
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

        /// The `done` flags of the `Set`s so far.
        fn dones(&self) -> Vec<bool> {
            (self.actions.iter())
                .map(|a| match a {
                    TempoAction::Set { done, .. } => *done,
                    other => panic!("{other:?}"),
                })
                .collect()
        }

        /// The rect button `name` was drawn in, the frame before the last.
        fn rect(&self, name: &str) -> Rect {
            let id = Id::new("tempo_editor").with(name);
            self.screen.ctx.read_response(id).expect(name).rect
        }
    }

    /// From `from`, `quarters` quarter turns round C, clockwise as seen (y down), or anticlockwise for negative ones.
    fn round(from: Pos2, quarters: i32) -> Vec<Pos2> {
        let mut points = vec![from];
        for _ in 0..quarters.abs() {
            let d = *points.last().unwrap() - C;
            let d = if quarters > 0 {
                vec2(-d.y, d.x)
            } else {
                vec2(d.y, -d.x)
            };
            points.push(C + d);
        }
        points
    }

    /// `n` U from C at `degrees` clockwise from 3 o'clock.
    fn polar(n: f32, degrees: f32) -> Pos2 {
        let a = degrees.to_radians();
        C + metrics().u(n) * vec2(a.cos(), a.sin())
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
    fn the_steps_round_after_adding_and_keep_to_10_999() {
        assert_eq!(step_bpm(140.5, 1.0), 142.0);
        assert_eq!(step_bpm(140.4, 1.0), 141.0);
        // Halves away from zero: 139.5 is 140.
        assert_eq!(step_bpm(140.5, -1.0), 140.0);
        assert_eq!(step_bpm(140.0, 10.0), 150.0);
        assert_eq!(step_bpm(995.0, 10.0), 999.0);
        assert_eq!(step_bpm(15.0, -10.0), 10.0);
        assert_eq!(step_bpm(10.4, -1.0), 10.0);
    }

    #[test]
    fn four_taps_set_the_mean_of_their_three_tempos_and_the_next_tap_starts_over() {
        let approx = |a: f32, b: f32| (a - b).abs() < 1e-5;
        let mut tap = TapTempo::default();
        assert!(approx(tap.size(), 0.85));
        // 0.5 s, 0.4 s and 0.6 s apart: 40 + 50 + 33.3, not rounded.
        let times = [10.0, 10.5, 10.9, 11.5];
        let mut got = Vec::new();
        for (t, size) in times.into_iter().zip([0.9, 0.95, 1.0, 0.85]) {
            got.push(tap.tap(t));
            assert!(approx(tap.size(), size), "{}", tap.size());
        }
        assert_eq!(got[..3], [None, None, None]);
        assert!((got[3].unwrap() - (40.0 + 50.0 + 20.0 / 0.6)).abs() < 1e-9);
        // The 5th tap starts a new group, however long after: it only records.
        assert_eq!(tap.tap(100.0), None);
        for t in [100.1, 100.2] {
            assert_eq!(tap.tap(t), None);
        }
        // 600 BPM.
        assert!((tap.tap(100.3).unwrap() - 600.0).abs() < 1e-9);
        // Clamped to [1, 999].
        let mut tap = TapTempo::default();
        let fast: Vec<_> = (0..4).map(|i| tap.tap(f64::from(i) * 0.01)).collect();
        assert_eq!(fast[3], Some(999.0));
        let slow: Vec<_> = (0..4).map(|i| tap.tap(f64::from(i) * 100.0)).collect();
        assert_eq!(slow[3], Some(1.0));
    }

    #[test]
    fn a_full_clockwise_turn_of_the_dial_adds_one_bpm_and_its_release_is_done() {
        let mut rig = Rig::new(140.0);
        rig.turn(&round(polar(2.0, 0.0), 4));
        assert!((rig.bpm - 141.0).abs() < 1e-9, "{}", rig.bpm);
        // A change on each of the four moves, then the release re-sends the final value as done.
        assert_eq!(rig.dones(), [false, false, false, false, true]);
        // Half a turn back.
        rig.actions.clear();
        rig.turn(&round(polar(2.0, 0.0), -2));
        assert!((rig.bpm - 140.5).abs() < 1e-9, "{}", rig.bpm);
        assert_eq!(rig.dones(), [false, false, true]);
        // A touch and release that turn nothing set nothing.
        rig.actions.clear();
        rig.click(polar(1.0, 0.0));
        assert_eq!(rig.actions, []);
    }

    #[test]
    fn the_dial_takes_a_touch_down_within_4_5_u_before_the_buttons() {
        let mut rig = Rig::new(140.0);
        // In `+10`'s square, 4.3 U from C: a quarter turn turns the dial, and `+10` neither fires nor lights.
        let on_plus_10 = C + metrics().u(1.0) * vec2(4.3, -0.4);
        assert!(rig.rect("+10").contains(on_plus_10));
        rig.turn(&round(on_plus_10, 1));
        assert!((rig.bpm - 140.25).abs() < 1e-9, "{}", rig.bpm);
        assert_eq!(rig.dones(), [false, true]);
        // A tap there is the dial's too: nothing.
        rig.actions.clear();
        rig.click(on_plus_10);
        assert_eq!(rig.actions, []);
        // 4.6 U straight up, no button there: no turn.
        rig.actions.clear();
        rig.turn(&round(polar(4.6, -90.0), 1));
        assert_eq!(rig.actions, []);
        // Out of the dial's reach, `+10` fires.
        rig.click(rig.rect("+10").center());
        assert_eq!(rig.bpm, 150.0);
    }

    #[test]
    fn a_tap_off_the_disc_the_x_escape_and_a_focus_loss_close_and_one_on_the_disc_does_nothing() {
        let mut rig = Rig::new(140.0);
        // On the disc, outside the dial and the buttons: swallowed.
        for at in [polar(6.0, -90.0), polar(7.4, 90.0), polar(7.4, 180.0)] {
            rig.click(at);
            assert_eq!(rig.actions, [], "{at:?}");
        }
        // Off the disc: closed.
        for at in [polar(7.6, 180.0), polar(7.6, 0.0), pos2(20.0, 20.0)] {
            rig.click(at);
            assert_eq!(rig.actions, [TempoAction::Close], "{at:?}");
            rig.actions.clear();
        }
        // None of it reached what is under the dialog.
        assert_eq!(rig.under, 0);
        rig.click(rig.rect("close").center());
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
        rig.run(|s, f| {
            s.frame(vec![egui::Event::WindowFocused(false)], f);
        });
        assert_eq!(rig.actions, [TempoAction::Close; 3]);
        // The metronome button asks for the flag to flip.
        rig.actions.clear();
        rig.click(rig.rect("metronome").center());
        assert_eq!(rig.actions, [TempoAction::Metronome]);
    }

    #[test]
    fn tap_grows_with_each_tap_of_a_group_and_opening_again_starts_a_new_group() {
        let mut rig = Rig::new(140.0);
        let base = rig.rect("TAP");
        let size = |rig: &Rig| rig.rect("TAP").width() / base.width() * 0.85;
        rig.click(base.center());
        // egui reads back the rects of the frame before.
        rig.frame();
        rig.frame();
        assert!((size(&rig) - 0.9).abs() < 1e-4, "{}", size(&rig));
        // Two taps, then the dialog is closed for a frame: opened again, the group starts again.
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
    fn the_value_bpm_the_time_signature_and_the_buttons_are_shown() {
        // `Time: <byte0>/<byte1>`: the rig's song has subdivision 3 (byte 0), 4 beats a bar (byte 1).
        let mut rig = Rig::new(92.5);
        rig.frame();
        let texts: Vec<String> = Offscreen::texts(&rig.shapes)
            .into_iter()
            .map(|(t, _)| t)
            .collect();
        for want in ["-1", "+1", "-10", "+10", "92.5", "BPM", "Time: 3/4", "TAP"] {
            assert!(texts.iter().any(|t| t == want), "{want} in {texts:?}");
        }
    }

    #[test]
    fn the_dial_truncates_to_a_tenth_and_the_transport_rounds_and_drops_a_zero_one() {
        assert_eq!(bpm_text(140.0), "140.0");
        assert_eq!(bpm_text(140.99), "140.9");
        assert_eq!(bpm_text(140.25), "140.2");
        assert_eq!(transport_bpm_text(140.25), "140.3");
        assert_eq!(transport_bpm_text(140.0), "140");
        assert_eq!(transport_bpm_text(140.04), "140");
        assert_eq!(transport_bpm_text(92.5), "92.5");
    }
}
