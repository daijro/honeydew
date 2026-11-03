//! The one touch law of the app: a press is decided from its touch-down. Held more than 300 ms ([`LONG_PRESS_S`],
//! [`is_long`]) without leaving the calm zone, 0.5 U from where it went down ([`CALM_U`], [`left_calm`]; Euclidean, the
//! track headers' and rulers' vertical only, [`vertical`]; the scroller's per axis), it is a long press; released
//! sooner, a tap; out of the zone first, a drag. The editors' grids (playlist, piano roll, drum editor, automation and
//! audio editors) decide their touches with [`is_long`] and [`left_calm`]; the headers, buttons and controls with
//! [`press`] / [`long_press`]. egui's own `long_touched` counts touches only, and only after 0.8 s.

use eframe::egui::{Pos2, Rect, Response, Ui, Vec2, response::Flags};

use crate::metrics::Metrics;
use crate::song::Event;
use crate::ui::focus_lost;

/// How long a press is held before it is a long press (s).
pub const LONG_PRESS_S: f64 = 0.3;
/// The calm zone's radius (U).
pub const CALM_U: f32 = 0.5;
/// [`press`] fires this early: the frame a repaint wakes for can come a hair before it is due.
const WAKE_EARLY_S: f64 = 1e-3;

/// Whether a press held `held` seconds is a long press: held more than [`LONG_PRESS_S`]. Released at or before it, it
/// is a tap.
///
/// This one boundary holds for every touch: the playhead knob, the scroller's tap and long tap and the graph lane's
/// knob. A press held exactly 0.3 s is a tap everywhere.
pub fn is_long(held: f64) -> bool {
    held > LONG_PRESS_S
}

/// Whether the pointer at `pos` has left the round calm zone about the touch-down `origin`: more than [`CALM_U`] from
/// it (Euclidean), `u` points a U. A frame without a pointer (a touch's lift) has not.
pub fn left_calm(origin: Pos2, pos: Option<Pos2>, u: f32) -> bool {
    pos.is_some_and(|p| p.distance(origin) > u * CALM_U)
}

/// Whether the long press on `response` comes due this frame: true on that one frame (for every call in it). Its
/// callers open their menu then, at the threshold, not on release (for buttons and controls; grid items, which act
/// on release, can read [`press`]).
///
/// The release that ends a long press is no click: `response.clicked()` turns false. A press that leaves the calm zone,
/// or sees a second finger or a focus loss, never becomes one; nor does one that egui made a drag, which on a widget
/// that senses drags happens after 6 points of travel (a knob or slider being dragged is not long pressed). egui's
/// other drag start, a press held still 0.8 s, is no travel: it cancels nothing (only a frame stalled past it sees it
/// before the 300 ms come due). Call it every frame `response` is shown.
pub fn long_press(ui: &Ui, response: &mut Response, m: &Metrics) -> bool {
    press(ui, response, m, Vec2::length).long
}

/// The calm zone of the track headers: vertical travel only.
pub fn vertical(d: Vec2) -> f32 {
    d.y.abs()
}

/// What a primary press that went down on a widget came to this frame. At most one of `long`, `tap` and
/// `long_released` is set, except that a release in the first frame after the 300 ms came due (no frame between) is
/// both `long` and `long_released`.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct Press {
    /// Down, inside the calm zone and short of 300 ms: still undecided.
    pub pending: bool,
    /// The long press came due this frame.
    pub long: bool,
    /// Down after its long press.
    pub held: bool,
    /// Released inside the calm zone before its 300 ms.
    pub tap: bool,
    /// Released after its long press, wherever the pointer went.
    pub long_released: bool,
}

/// [`long_press`] in full: the press on `response` with the calm zone's distance law `calm` (of the travel from
/// touch-down; [`Vec2::length`] is the round zone, [`vertical`] the headers'). A press that leaves the calm zone before
/// its long press reports nothing more. Call it every frame `response` is shown, once for each law.
pub fn press(ui: &Ui, response: &mut Response, m: &Metrics, calm: fn(Vec2) -> f32) -> Press {
    let id = response.id.with("long press");
    let frame = ui.ctx().cumulative_frame_nr();
    let (down, released, start, origin, pos, now, cancel) = ui.input(|i| {
        let p = &i.pointer;
        (
            p.primary_down(),
            p.primary_released(),
            p.press_start_time(),
            p.press_origin(),
            p.latest_pos(),
            i.time,
            focus_lost(i) || i.multi_touch().is_some(),
        )
    });
    if !down && !released {
        ui.data_mut(|d| d.remove::<Hold>(id));
        return Press::default();
    }
    // A hold not asked about last frame is stale: its widget was hidden over its release.
    let mut hold = ui
        .data(|d| d.get_temp::<Hold>(id))
        .filter(|h| h.seen + 1 >= frame);
    // A press that went down on `response` starts a hold, which then follows that press wherever the pointer goes
    // (egui forgets a click-only widget's press once it moves 6 points).
    if down && hold.map(|h| h.start) != start {
        hold = start
            .zip(origin)
            .filter(|_| response.is_pointer_button_down_on())
            .map(|(start, origin)| Hold {
                start,
                origin,
                cancelled: false,
                fired: None,
                seen: frame,
            });
    }
    let Some(mut h) = hold else {
        ui.data_mut(|d| d.remove::<Hold>(id));
        // A press released in its own frame (or one whose hold went stale) was never held: egui's click decides.
        return Press {
            tap: !down && response.clicked(),
            ..Press::default()
        };
    };
    h.seen = frame;
    let click = ui.ctx().options(|o| o.input_options.max_click_dist);
    let dragged = response.dragged() && pos.is_some_and(|p| p.distance(h.origin) > click);
    h.cancelled |= h.fired.is_none()
        && (cancel || dragged || pos.is_some_and(|p| calm(p - h.origin) > m.u(CALM_U)));
    // Due a hair early ([`WAKE_EARLY_S`]). A release in the first frame after the hold came due (no frame in between)
    // is a long press too.
    let held = now - h.start;
    if !h.cancelled && h.fired.is_none() {
        if is_long(held + WAKE_EARLY_S) {
            h.fired = Some(frame);
        } else if down {
            ui.ctx()
                .request_repaint_after_secs((LONG_PRESS_S - held) as f32);
        }
    }
    if h.fired.is_some() {
        response.flags.remove(Flags::CLICKED);
    }
    ui.data_mut(|d| d.insert_temp(id, h));
    let fired = h.fired.is_some();
    Press {
        pending: down && !h.cancelled && !fired,
        long: h.fired == Some(frame),
        held: down && fired,
        tap: !down && !h.cancelled && !fired,
        long_released: !down && fired,
    }
}

/// A primary press held on a widget: when and where it went down, whether it can no longer be a long press, the
/// frame it became one, and the last frame it was asked about.
#[derive(Clone, Copy)]
struct Hold {
    start: f64,
    origin: Pos2,
    cancelled: bool,
    fired: Option<u64>,
    seen: u64,
}

/// The right-button erase: a right press on a note or hit of the piano roll or the drum editor deletes it, and dragging
/// on deletes every one the pointer's path crosses, each frame's whole path from the last point (a fast swipe skips
/// none); no selection box shows, and the whole swipe is one undo step. Touches are unchanged. The swipe under way: the
/// pointer's last point, the editor's events as it found them with their selection flags, and which of them it deleted.
#[derive(Clone, Debug, PartialEq)]
pub(crate) struct Erase {
    last: Pos2,
    pub orig: Vec<Event>,
    sel: Vec<bool>,
    pub gone: Vec<bool>,
}

impl Erase {
    /// The swipe a right press starts this frame: one that went down on `grid` at `at` with the primary button up, over
    /// `events` and their selection `sel`.
    pub(crate) fn start(
        ui: &Ui,
        grid: &Response,
        at: Option<Pos2>,
        events: &[Event],
        sel: &[bool],
    ) -> Option<Self> {
        let pressed = ui.input(|i| {
            let p = &i.pointer;
            p.secondary_pressed() && !p.primary_down() && i.multi_touch().is_none()
        });
        let at = at.filter(|_| pressed && grid.is_pointer_button_down_on())?;
        let mut sel = sel.to_vec();
        sel.resize(events.len(), false);
        Some(Self {
            last: at,
            orig: events.to_vec(),
            sel,
            gone: vec![false; events.len()],
        })
    }

    /// The right button is still down: the swipe goes on.
    pub(crate) fn held(ui: &Ui) -> bool {
        ui.input(|i| i.pointer.secondary_down())
    }

    /// The pointer on to `p` (`None`: it stays): its path from the last point deletes every event whose box (`rect`) it
    /// crosses. When that deleted more, the events and selection flags left.
    pub(crate) fn sweep(
        &mut self,
        p: Option<Pos2>,
        rect: impl Fn(&Event) -> Option<Rect>,
    ) -> Option<(Vec<Event>, Vec<bool>)> {
        let (a, b) = (self.last, p.unwrap_or(self.last));
        self.last = b;
        let mut more = false;
        for (e, gone) in self.orig.iter().zip(&mut self.gone) {
            if !*gone && rect(e).is_some_and(|r| crosses(r, a, b)) {
                (*gone, more) = (true, true);
            }
        }
        more.then(|| {
            let left = (self.orig.iter().zip(&self.sel).zip(&self.gone)).filter(|(_, g)| !**g);
            left.map(|((e, s), _)| (*e, *s)).unzip()
        })
    }

    /// Whether it deleted anything.
    pub(crate) fn erased(&self) -> bool {
        self.gone.contains(&true)
    }
}

/// Whether the path from `a` to `b` (a point when they are equal) crosses `r`, without its right and bottom edges (so
/// an edge two boxes share is one box's only). Slab clipping of the segment against both axes.
fn crosses(r: Rect, a: Pos2, b: Pos2) -> bool {
    if !r.is_positive() {
        return false;
    }
    let (mut t0, mut t1) = (0.0_f32, 1.0_f32);
    for (o, d, lo, hi) in [
        (a.x, b.x - a.x, r.min.x, r.max.x),
        (a.y, b.y - a.y, r.min.y, r.max.y),
    ] {
        if d == 0.0 {
            if !(lo..hi).contains(&o) {
                return false;
            }
        } else {
            let (u, v) = ((lo - o) / d, (hi - o) / d);
            (t0, t1) = (t0.max(u.min(v)), t1.min(u.max(v)));
        }
    }
    t0 < t1
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::ui::offscreen::{SCREEN, metrics};
    use eframe::egui::{
        CentralPanel, Context, Event, Id, PointerButton, Pos2, RawInput, Rect, Sense, ViewportId,
        pos2, vec2,
    };

    const RECT: Rect = Rect {
        min: pos2(100.0, 100.0),
        max: pos2(200.0, 200.0),
    };
    const AT: Pos2 = pos2(150.0, 150.0);

    /// A rect sensing `sense`, run through [`long_press`] in an offscreen context; each frame logs (long, clicked)
    /// and keeps the repaint delay it asked for.
    struct Rig {
        ctx: Context,
        time: f64,
        sense: Sense,
        /// The calm zone's law; `None` is the round zone, checked against [`long_press`] too.
        calm: Option<fn(Vec2) -> f32>,
        log: Vec<(bool, bool)>,
        /// Each frame's [`Press`].
        presses: Vec<Press>,
        repaint: f64,
        /// The rect is drawn (and asked).
        shown: bool,
    }

    impl Rig {
        fn new(sense: Sense) -> Self {
            let mut rig = Self {
                ctx: Context::default(),
                time: 0.0,
                sense,
                calm: None,
                log: Vec::new(),
                presses: Vec::new(),
                repaint: f64::INFINITY,
                shown: true,
            };
            rig.frame(0.1, vec![Event::PointerMoved(AT)]);
            rig
        }

        /// One frame `dt` seconds after the last.
        fn frame(&mut self, dt: f64, events: Vec<Event>) -> (bool, bool) {
            self.time += dt;
            let raw = RawInput {
                screen_rect: Some(SCREEN),
                time: Some(self.time),
                events,
                ..Default::default()
            };
            let (sense, calm, m, shown) = (self.sense, self.calm, metrics(), self.shown);
            let (mut out, mut p) = ((false, false), Press::default());
            let output = self.ctx.run(raw, |ctx| {
                CentralPanel::default().show(ctx, |ui| {
                    if !shown {
                        return;
                    }
                    let mut r = ui.interact(RECT, Id::new("held"), sense);
                    // Twice, as a widget and its caller both may: the same answer.
                    p = press(ui, &mut r.clone(), &m, calm.unwrap_or(Vec2::length));
                    assert_eq!(press(ui, &mut r, &m, calm.unwrap_or(Vec2::length)), p);
                    if calm.is_none() {
                        assert_eq!(long_press(ui, &mut r, &m), p.long);
                    }
                    out = (p.long, r.clicked());
                });
            });
            self.presses.push(p);
            self.repaint = output.viewport_output[&ViewportId::ROOT]
                .repaint_delay
                .as_secs_f64();
            self.log.push(out);
            out
        }

        fn button(
            &mut self,
            dt: f64,
            pos: Pos2,
            button: PointerButton,
            pressed: bool,
        ) -> (bool, bool) {
            let event = Event::PointerButton {
                pos,
                button,
                pressed,
                modifiers: Default::default(),
            };
            self.frame(dt, vec![event])
        }

        /// Presses at [`AT`], holds still for `secs` (a frame every 50 ms and one at `secs`) and releases a frame
        /// later. Returns whether a long press came due, and whether the release clicked.
        fn hold(&mut self, secs: f64) -> (bool, bool) {
            self.log.clear();
            self.button(0.1, AT, PointerButton::Primary, true);
            let mut held = 0.0;
            while held + 0.05 < secs {
                self.frame(0.05, vec![]);
                held += 0.05;
            }
            self.frame(secs - held, vec![]);
            let (_, clicked) = self.button(1.0 / 60.0, AT, PointerButton::Primary, false);
            (self.log.iter().any(|f| f.0), clicked)
        }

        fn any_long(&self) -> bool {
            self.log.iter().any(|f| f.0)
        }
    }

    #[test]
    fn a_grid_press_is_long_past_300_ms_and_leaves_the_calm_zone_past_half_a_u() {
        // Released at 300 ms it is still a tap; any later, a long press.
        assert!(!is_long(0.3));
        assert!(!is_long(0.29));
        assert!(is_long(0.301));
        // Euclidean, 0.5 U: 16 points at U = 32; a lift's frame has no pointer and stays calm.
        let u = 32.0;
        let at = |d: f32| Some(AT + vec2(d, d) / 2f32.sqrt());
        assert!(!left_calm(AT, at(16.0), u));
        assert!(left_calm(AT, at(16.1), u));
        assert!(!left_calm(AT, None, u));
    }

    #[test]
    fn a_mouse_held_still_300_ms_is_a_long_press_on_that_frame_and_its_release_no_click() {
        for sense in [Sense::click(), Sense::click_and_drag()] {
            let mut rig = Rig::new(sense);
            assert_eq!(rig.hold(0.3), (true, false), "{sense:?}");
            // On exactly one frame: the one at 300 ms.
            assert_eq!(rig.log.iter().filter(|f| f.0).count(), 1);
            assert!(rig.log[rig.log.len() - 2].0);
        }
    }

    #[test]
    fn a_press_released_at_250_ms_is_a_click_and_no_long_press() {
        for sense in [Sense::click(), Sense::click_and_drag()] {
            let mut rig = Rig::new(sense);
            assert_eq!(rig.hold(0.25), (false, true), "{sense:?}");
        }
    }

    #[test]
    fn a_press_released_in_the_first_frame_after_its_300_ms_is_a_long_press() {
        let mut rig = Rig::new(Sense::click());
        rig.button(0.1, AT, PointerButton::Primary, true);
        assert_eq!(
            rig.button(0.35, AT, PointerButton::Primary, false),
            (true, false)
        );
        assert!(!rig.frame(0.1, vec![]).0, "once");
    }

    /// A press and its release in one frame (a slow frame).
    fn one_frame_click(rig: &mut Rig) -> Press {
        let button = |pressed| Event::PointerButton {
            pos: AT,
            button: PointerButton::Primary,
            pressed,
            modifiers: Default::default(),
        };
        rig.frame(0.1, vec![button(true), button(false)]);
        *rig.presses.last().unwrap()
    }

    #[test]
    fn a_press_released_in_its_own_frame_is_a_tap() {
        for sense in [Sense::click(), Sense::click_and_drag()] {
            let mut rig = Rig::new(sense);
            let tap = Press {
                tap: true,
                ..Press::default()
            };
            assert_eq!(one_frame_click(&mut rig), tap, "{sense:?}");
            assert!(rig.log.last().unwrap().1, "and a click");
        }
    }

    #[test]
    fn a_hold_left_by_a_widget_hidden_at_its_release_never_haunts_a_later_click() {
        let mut rig = Rig::new(Sense::click());
        rig.button(0.1, AT, PointerButton::Primary, true);
        rig.frame(0.4, vec![]);
        assert!(rig.any_long());
        // Hidden from its release on, for a while.
        rig.shown = false;
        rig.button(0.1, AT, PointerButton::Primary, false);
        rig.frame(0.1, vec![]);
        // Back in the frame of a one-frame click (which egui gives it no click for: it was not there to be hit): no
        // ghost long press from the old hold.
        rig.shown = true;
        assert_eq!(one_frame_click(&mut rig), Press::default());
        // And the next click is a tap and a click.
        let tap = Press {
            tap: true,
            ..Press::default()
        };
        assert_eq!(one_frame_click(&mut rig), tap);
        assert!(rig.log.last().unwrap().1, "the click stands");
    }

    #[test]
    fn the_hold_asks_for_the_frame_its_300_ms_come_due() {
        let mut rig = Rig::new(Sense::click());
        // The press and the frame after it repaint at once anyway; a still frame after them waits for the hold. egui
        // takes one predicted frame (1/60 s) off the delay asked for.
        rig.button(0.1, AT, PointerButton::Primary, true);
        rig.frame(0.1, vec![]);
        rig.frame(0.05, vec![]);
        assert!(
            (rig.repaint - (0.15 - 1.0 / 60.0)).abs() < 1e-3,
            "{}",
            rig.repaint
        );
        // Nothing once it came due.
        rig.frame(0.15, vec![]);
        rig.frame(0.1, vec![]);
        assert!(rig.repaint > 1e9, "{}", rig.repaint);
    }

    #[test]
    fn leaving_the_calm_zone_cancels_even_after_coming_back() {
        let u = metrics().u(1.0);
        for (by, long) in [(0.45 * u, true), (0.55 * u, false)] {
            let mut rig = Rig::new(Sense::click());
            rig.button(0.1, AT, PointerButton::Primary, true);
            // Diagonal: the distance is Euclidean.
            let out = AT + vec2(by, by) / 2f32.sqrt();
            rig.frame(0.05, vec![Event::PointerMoved(out)]);
            rig.frame(0.05, vec![Event::PointerMoved(AT)]);
            rig.frame(0.25, vec![]);
            assert_eq!(rig.any_long(), long, "{by}");
        }
    }

    #[test]
    fn a_widget_that_senses_drags_is_dragged_after_6_points_and_not_long_pressed() {
        for (sense, long) in [(Sense::click(), true), (Sense::click_and_drag(), false)] {
            let mut rig = Rig::new(sense);
            rig.button(0.1, AT, PointerButton::Primary, true);
            rig.frame(0.05, vec![Event::PointerMoved(AT + vec2(8.0, 0.0))]);
            rig.frame(0.3, vec![]);
            rig.button(0.1, AT, PointerButton::Primary, false);
            assert_eq!(rig.any_long(), long, "{sense:?}");
        }
    }

    #[test]
    fn a_still_press_whose_next_frame_comes_after_eguis_0_8_s_drag_start_is_still_a_long_press() {
        // A stalled frame: egui makes the still press a drag by time alone, which is no travel.
        for sense in [Sense::click(), Sense::click_and_drag()] {
            let mut rig = Rig::new(sense);
            rig.button(0.1, AT, PointerButton::Primary, true);
            assert_eq!(rig.frame(1.0, vec![]), (true, false), "{sense:?}");
        }
    }

    #[test]
    fn a_press_reports_pending_then_tap_or_long_held_and_long_released() {
        let mut rig = Rig::new(Sense::click());
        rig.button(0.1, AT, PointerButton::Primary, true);
        rig.frame(0.1, vec![]);
        let pending = Press {
            pending: true,
            ..Press::default()
        };
        assert_eq!(rig.presses[1..], [pending, pending]);
        rig.button(0.1, AT, PointerButton::Primary, false);
        let tap = Press {
            tap: true,
            ..Press::default()
        };
        assert_eq!(rig.presses.last(), Some(&tap));
        rig.presses.clear();
        rig.button(0.1, AT, PointerButton::Primary, true);
        rig.frame(0.3, vec![]);
        rig.frame(0.1, vec![Event::PointerMoved(AT + vec2(0.0, 100.0))]);
        rig.button(0.1, AT, PointerButton::Primary, false);
        rig.frame(0.1, vec![]);
        let held = Press {
            held: true,
            ..Press::default()
        };
        let long = Press { long: true, ..held };
        let released = Press {
            long_released: true,
            ..Press::default()
        };
        assert_eq!(
            rig.presses,
            [pending, long, held, released, Press::default()]
        );
    }

    #[test]
    fn the_vertical_calm_zone_lets_the_pointer_wander_sideways() {
        let u = metrics().u(1.0);
        for (by, long) in [
            (vec2(3.0 * u, 0.45 * u), true),
            (vec2(0.0, 0.55 * u), false),
            (vec2(0.0, -0.55 * u), false),
        ] {
            let mut rig = Rig::new(Sense::click());
            rig.calm = Some(vertical);
            rig.button(0.1, AT, PointerButton::Primary, true);
            rig.frame(0.1, vec![Event::PointerMoved(AT + by)]);
            rig.frame(0.2, vec![]);
            assert_eq!(rig.any_long(), long, "{by:?}");
            assert!(!rig.presses.last().unwrap().pending);
        }
    }

    #[test]
    fn a_finger_held_still_300_ms_is_a_long_press_too() {
        use eframe::egui::{TouchDeviceId, TouchId, TouchPhase};
        let mut rig = Rig::new(Sense::click());
        let touch = Event::Touch {
            device_id: TouchDeviceId(0),
            id: TouchId(0),
            phase: TouchPhase::Start,
            pos: AT,
            force: None,
        };
        let press = Event::PointerButton {
            pos: AT,
            button: PointerButton::Primary,
            pressed: true,
            modifiers: Default::default(),
        };
        rig.frame(0.1, vec![touch, press]);
        assert_eq!(rig.frame(0.3, vec![]), (true, false));
    }

    #[test]
    fn a_secondary_hold_a_focus_loss_or_a_press_from_elsewhere_is_none() {
        let mut secondary = Rig::new(Sense::click());
        secondary.button(0.1, AT, PointerButton::Secondary, true);
        secondary.frame(0.3, vec![]);
        secondary.button(0.1, AT, PointerButton::Secondary, false);
        let mut focus = Rig::new(Sense::click());
        focus.button(0.1, AT, PointerButton::Primary, true);
        focus.frame(0.1, vec![Event::WindowFocused(false)]);
        focus.frame(0.25, vec![]);
        let mut elsewhere = Rig::new(Sense::click());
        let outside = pos2(50.0, 50.0);
        elsewhere.frame(0.1, vec![Event::PointerMoved(outside)]);
        elsewhere.button(0.1, outside, PointerButton::Primary, true);
        elsewhere.frame(0.05, vec![Event::PointerMoved(AT)]);
        elsewhere.frame(0.3, vec![]);
        for rig in [secondary, focus, elsewhere] {
            assert!(!rig.any_long());
        }
    }

    #[test]
    fn the_next_quick_tap_after_a_long_press_clicks_again() {
        let mut rig = Rig::new(Sense::click());
        assert_eq!(rig.hold(0.4), (true, false));
        assert_eq!(rig.hold(0.1), (false, true));
        // And a stale hold never comes due by itself.
        for _ in 0..5 {
            assert!(!rig.frame(0.2, vec![]).0);
        }
    }
}
