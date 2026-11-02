//! The keyboard panel's hand resize and tap toggle, and the piano's scroll zone over the panel's button bar and settings
//! strip (the scroll and fling follow the editors' scroll and zoom law).
//!
//! - The grab follows the primary pointer (the mouse, or the first finger) from a press in the zone: the bar and the
//!   strip, less the strip's `Velocity` and `Timing` knobs (and the wheels, which are controls too; where the open strip
//!   covers them it is the strip's). The press must land in the panel's own layer, under no overlay or popup. A button
//!   there still takes its tap; a drag that starts on it is the grab's.
//! - The first axis past its calm distance decides what the touch does: more than 1 U vertically resizes, more than
//!   0.5 U horizontally (the scroller's calm zone) scrolls the keys, only from a press in the scroll zone and only while
//!   the drag is mostly sideways (|dx| > |dy|), so that a wobbly upward drag still resizes. The move
//!   that crosses 1 U only starts the resize, so the panel does not jump; the moves after it resize.
//! - egui gathers a frame's moves into one delta, so the last move event is the last frame whose pointer moved.
//! - A fling runs the scroller's law in points: v ← 0.9·v, then the keys move by v, stopping under 0.001 or at an end of
//!   their range. Any touch-down in the panel stops it (as in the mixer), a key's among them.
//! - The sizes are kept in U ([`crate::settings::Settings`]), not in pixels, so that the panel keeps its
//!   size in U across UI scales.

use eframe::egui::{Id, Pos2, Rect, Sense, Ui, Vec2};

use crate::metrics::Metrics;
use crate::ui::gesture;
use crate::ui::main_screen::TRANSPORT_H;
use crate::ui::scroller::{WHEEL_FLOOR, WHEEL_ZOOM};

/// The vertical travel, in U, past which a grab resizes.
const RESIZE_U: f32 = 1.0;
/// The least open height over the collapsed one, in U: a let-go height under c + 0.75 U collapses to the bar alone.
pub const OPEN_MIN: f32 = 0.75;
/// A let-go height follows the last move this many times over: h' = h − 10·Δy.
const THROW: f32 = 10.0;
/// The least last move, in U, that flings the keys, and the fling's speed kept per frame.
const FLING_MIN_U: f32 = 0.125;
const DECAY: f32 = 0.9;

/// c, the collapsed panel's height (the transport bar alone, `rows` rows of 1.75 U): trunc(c · U) px, in points.
pub fn collapsed(rows: u8, m: &Metrics) -> f32 {
    (m.u_px() * TRANSPORT_H * f32::from(rows)).trunc() / m.ppp()
}

/// The height a resize let go at `h` settles at, after a last move of `last_dy` points, for the collapsed height `c` on
/// a screen `screen_h` tall (M): h' = h − 10·Δy within [c, M]; M when h' > M − c, c when h' < c + 0.75 U, else h'.
pub fn released(h: f32, last_dy: f32, c: f32, screen_h: f32, m: &Metrics) -> f32 {
    let full = screen_h.max(c);
    let thrown = (h - THROW * last_dy).clamp(c, full);
    if thrown > full - c {
        full
    } else if thrown < c + m.u(OPEN_MIN) {
        c
    } else {
        thrown
    }
}

/// The tap toggle's height (the bar's empty space, the strip's, `toggleKeyboardPanel`): collapsed (0, snapped up to
/// c) while the `preferred` height is over the collapsed `c`, else the restore height max(`recent`, c + 0.75 U); either
/// through the release snap ([`released`]). All in points. "Over" is by more than half a device pixel: a height kept
/// in U comes back a hair off the bar's own.
pub fn toggled(preferred: f32, recent: f32, c: f32, screen_h: f32, m: &Metrics) -> f32 {
    let target = if (preferred - c) * m.ppp() > 0.5 {
        0.0
    } else {
        recent.max(c + m.u(OPEN_MIN))
    };
    released(target, 0.0, c, screen_h, m)
}

/// The preferred height when the transport bar's rows change, its collapsed height going from `old` to `new` (all in
/// points): a collapsed panel stays collapsed at `new`; an open one is at least `new` + 0.75 U.
pub fn rows_changed(preferred: f32, old: f32, new: f32, m: &Metrics) -> f32 {
    if (preferred - old) * m.ppp() > 0.5 {
        preferred.max(new + m.u(OPEN_MIN))
    } else {
        new
    }
}

/// The preferred height (points) the keyboard settings strip opens the panel to: while the panel was `hidden`,
/// collapsed or had no keys area under the strip `strip` points tall, min(M, max(`recent`, c + strip + 6.5 U)); `None`
/// keeps the size.
pub fn strip_opened(
    preferred: f32,
    recent: f32,
    c: f32,
    strip: f32,
    screen_h: f32,
    hidden: bool,
    m: &Metrics,
) -> Option<f32> {
    let no_keys = (preferred - c - strip) * m.ppp() <= 0.5;
    (hidden || no_keys).then(|| recent.max(c + strip + m.u(6.5)).min(screen_h.max(c)))
}

/// What the panel's sizes become, in points: the preferred height, and the recent one when it changes.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Sizes {
    pub preferred: f32,
    pub recent: Option<f32>,
}

impl Sizes {
    /// The sizes after a resize settles at `h`: the recent height too while `h` is over the collapsed `c`.
    pub fn settled(h: f32, c: f32) -> Self {
        Self {
            preferred: h,
            recent: (h > c).then_some(h),
        }
    }
}

/// Where a grab may start.
#[derive(Clone, Debug, PartialEq)]
pub struct Zone {
    /// The button bar and the settings strip (none while it is closed).
    pub bar: Rect,
    pub strip: Rect,
    /// The strip's knobs, where no grab starts.
    pub knobs: Vec<Rect>,
    /// The scroll zone's left edge: the wheels column's right edge while the wheels show, else the panel's left.
    pub scroll_left: f32,
}

impl Zone {
    fn grabs(&self, p: Pos2) -> bool {
        (self.bar.contains(p) || self.strip.contains(p))
            && !self.knobs.iter().any(|k| k.contains(p))
    }

    fn scrolls(&self, p: Pos2) -> bool {
        self.grabs(p) && p.x >= self.scroll_left
    }
}

/// What a grab does, decided by its first axis past its calm distance.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Mode {
    Calm,
    Resize,
    Scroll,
}

#[derive(Clone, Copy, Debug)]
struct Touch {
    origin: Pos2,
    mode: Mode,
    /// It went down in the scroll zone.
    scrolls: bool,
    /// The pointer's travel in the last frame it moved.
    last: Vec2,
}

/// The tap toggle over `zone`: a tap (calm, released within 0.3 s) that no button took. Call it before the bar's buttons
/// are drawn: under them, a drag-only layer keeps drags from a view drawn under a tall panel, and the click layer over it
/// takes only the taps no button takes.
pub fn tap(ui: &Ui, zone: &Zone, m: &Metrics) -> bool {
    let id = Id::new("keyboard_panel_grab");
    let rect = zone.bar.union(zone.strip);
    ui.interact(rect, id.with("drags"), Sense::drag());
    let mut background = ui.interact(rect, id, Sense::click());
    gesture::press(ui, &mut background, m, Vec2::length).tap
}

/// The grab's state between frames, and the keys' fling.
#[derive(Clone, Debug, Default)]
pub struct PanelGrab {
    touch: Option<Touch>,
    /// The keys' fling, in points a frame (the keys move by it: `left −= fling`).
    pub fling: f32,
}

/// What the grab did this frame.
#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub struct Grabbed {
    /// The panel grows by this many points now (a resize under way; negative shrinks it).
    pub grow: f32,
    /// A resize was let go: its last move's y travel, for [`released`].
    pub released: Option<f32>,
    /// The keys follow the finger by this many points (`left −= scroll`), or fling by it.
    pub scroll: f32,
    /// The keys zoom by this factor, anchored at this x.
    pub zoom: Option<(f32, f32)>,
}

impl PanelGrab {
    /// A resize is under way: the panel follows the finger, not its preferred height.
    pub fn resizing(&self) -> bool {
        self.touch.is_some_and(|t| t.mode == Mode::Resize)
    }

    /// One frame of the grab over `zone` (the last frame's, so that a resize lays the panel out in the frame it moves):
    /// the pointer's drags and the wheel, read from the input with no widget. `covered`: an overlay lies over the
    /// pointer. Any touch-down stops the fling. The tap is [`tap`]'s.
    pub fn track(&mut self, ui: &Ui, zone: &Zone, covered: bool, m: &Metrics) -> Grabbed {
        let mut out = Grabbed::default();
        let (pressed, down, origin, pos, delta, wheel, zoom, multi, hover) = ui.input(|i| {
            let p = &i.pointer;
            (
                p.primary_pressed(),
                p.primary_down(),
                p.press_origin(),
                p.interact_pos(),
                p.delta(),
                i.raw_scroll_delta,
                i.zoom_delta_2d().x,
                i.multi_touch().is_some(),
                p.hover_pos(),
            )
        });
        let ours = |p: Pos2| ui.ctx().layer_id_at(p).is_none_or(|l| l == ui.layer_id());
        // The fling runs from the frame after the release on.
        let coasting = self.touch.is_none();
        if pressed {
            self.fling = 0.0;
            self.touch = origin
                .filter(|&p| !covered && zone.grabs(p) && ours(p))
                .map(|p| Touch {
                    origin: p,
                    mode: Mode::Calm,
                    scrolls: zone.scrolls(p),
                    last: Vec2::ZERO,
                });
        }
        if let Some(t) = &mut self.touch {
            if down {
                if delta != Vec2::ZERO {
                    t.last = delta;
                }
                let travel = pos.unwrap_or(t.origin) - t.origin;
                match t.mode {
                    _ if multi => {}
                    Mode::Calm if travel.y.abs() > m.u(RESIZE_U) => t.mode = Mode::Resize,
                    Mode::Calm
                        if t.scrolls
                            && travel.x.abs() > m.u(gesture::CALM_U)
                            && travel.x.abs() > travel.y.abs() =>
                    {
                        t.mode = Mode::Scroll
                    }
                    Mode::Calm => {}
                    Mode::Resize => out.grow = -delta.y,
                    Mode::Scroll => out.scroll = delta.x,
                }
                // Two fingers that went down in the scroll zone zoom the keys.
                if multi && t.scrolls && zoom != 1.0 {
                    out.zoom = Some((zoom, pos.unwrap_or(t.origin).x));
                }
            } else {
                let t = self.touch.take().expect("matched above");
                match t.mode {
                    Mode::Resize => out.released = Some(t.last.y),
                    Mode::Scroll
                        if t.last.x.abs() > m.u(FLING_MIN_U)
                            && crate::ui::anim::inertia(ui.ctx()) =>
                    {
                        self.fling = t.last.x
                    }
                    _ => {}
                }
            }
        }
        // The wheel (or a trackpad pinch) over the scroll zone zooms the keys, and stops a fling.
        if let Some(p) = hover.filter(|&p| !covered && zone.scrolls(p) && ours(p)) {
            let dy = if ui.input(|i| i.modifiers.command) {
                0.0
            } else {
                wheel.y
            };
            // The scroller's wheel law: points per key ← ·1/(1 − 0.001·dy), the divisor kept above 0.05.
            let wheel_zoom = 1.0 / (1.0 - WHEEL_ZOOM * dy).max(WHEEL_FLOOR);
            let factor = wheel_zoom * if multi { 1.0 } else { zoom };
            if factor != 1.0 {
                self.fling = 0.0;
                out.zoom = Some((factor, p.x));
            }
        }
        if coasting && self.touch.is_none() && self.fling != 0.0 {
            self.fling *= DECAY;
            if self.fling.abs() < 0.001 {
                self.fling = 0.0;
            }
            out.scroll = self.fling;
            ui.ctx().request_repaint();
        }
        out
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::metrics::DEFAULT_UI_SCALE;

    fn metrics(ppp: f32) -> Metrics {
        Metrics::from_pixels_per_point(ppp, DEFAULT_UI_SCALE)
    }

    #[test]
    fn a_let_go_height_follows_its_last_move_ten_times_and_snaps_to_full_or_collapsed_by_the_bars_rows()
     {
        for ppp in [1.0, 2.0] {
            let m = metrics(ppp);
            let (u, screen) = (m.u(1.0), 540.0);
            for rows in [1, 2] {
                let c = collapsed(rows, &m);
                assert_eq!(c, (1.75 * f32::from(rows) * m.u_px()).trunc() / ppp);
                // Still: h' = h; the last move 3 points up throws it 30 points taller.
                assert_eq!(released(8.0 * u, 0.0, c, screen, &m), 8.0 * u);
                assert_eq!(released(8.0 * u, -3.0, c, screen, &m), 8.0 * u + 30.0);
                // Past M − c: full screen.
                assert_eq!(released(screen - c + 1.0, 0.0, c, screen, &m), screen);
                assert_eq!(released(screen - c, 0.0, c, screen, &m), screen - c);
                // Under c + 0.75 U: collapsed; at it, it stays.
                assert_eq!(released(c + 0.75 * u - 0.5, 0.0, c, screen, &m), c);
                assert_eq!(released(c + 0.75 * u, 0.0, c, screen, &m), c + 0.75 * u);
                // The toggle: an open panel collapses; a collapsed one opens to the recent height, at least c + 0.75 U.
                assert_eq!(toggled(c + 1.0 / ppp, 7.0 * u, c, screen, &m), c);
                assert_eq!(toggled(c, 7.0 * u, c, screen, &m), 7.0 * u);
                assert_eq!(toggled(c, 0.0, c, screen, &m), c + 0.75 * u);
                assert_eq!(toggled(c, 900.0, c, screen, &m), screen);
            }
        }
    }

    #[test]
    fn a_row_switch_keeps_a_collapsed_panel_collapsed_and_the_strip_opens_a_panel_without_keys() {
        let m = metrics(1.0);
        let u = m.u(1.0);
        let (one, two) = (collapsed(1, &m), collapsed(2, &m));
        // Collapsed stays collapsed, either way; an open panel is at least the new c + 0.75 U.
        assert_eq!(rows_changed(one, one, two, &m), two);
        assert_eq!(rows_changed(two, two, one, &m), one);
        assert_eq!(rows_changed(4.0 * u, one, two, &m), two + 0.75 * u);
        assert_eq!(rows_changed(8.0 * u, one, two, &m), 8.0 * u);
        // The strip (3 U): a panel with keys keeps its size; hidden, collapsed or without keys it opens to
        // max(recent, c + strip + 6.5 U), at most the screen.
        let strip = 3.0 * u;
        assert_eq!(
            strip_opened(8.0 * u, 5.0 * u, one, strip, 540.0, false, &m),
            None
        );
        let opened = Some(one + strip + 6.5 * u);
        assert_eq!(
            strip_opened(8.0 * u, 5.0 * u, one, strip, 540.0, true, &m),
            opened
        );
        assert_eq!(
            strip_opened(one, 5.0 * u, one, strip, 540.0, false, &m),
            opened
        );
        assert_eq!(
            strip_opened(one + strip, 5.0 * u, one, strip, 540.0, false, &m),
            opened
        );
        assert_eq!(
            strip_opened(one, 14.0 * u, one, strip, 540.0, false, &m),
            Some(14.0 * u)
        );
        assert_eq!(
            strip_opened(one, 5.0 * u, one, strip, 300.0, false, &m),
            Some(300.0)
        );
    }

    #[test]
    fn a_collapsed_panel_kept_in_u_reopens_at_every_density_and_ui_scale() {
        // The settings keep sizes in U: the bar alone comes back as c / U · U, which may land an ulp off c (at
        // 150 % DPI and 110 % scale, U = 53 px).
        for ppp in [1.0, 1.25, 1.5, 2.0, 2.75, 3.0] {
            for factor in [60, 75, 90, 100, 110, 125, 150, 175, 200] {
                let m = Metrics::from_pixels_per_point(ppp, factor);
                let (u, c) = (m.u(1.0), collapsed(1, &m));
                let back = m.u(c / u);
                assert_eq!(
                    toggled(back, 7.0 * u, c, 2000.0, &m),
                    7.0 * u,
                    "ppp {ppp}, scale {factor}"
                );
                // And a panel a pixel over the bar still collapses.
                let over = m.u((c + 1.0 / ppp) / u);
                assert_eq!(toggled(over, 7.0 * u, c, 2000.0, &m), c, "ppp {ppp}");
            }
        }
    }
}
