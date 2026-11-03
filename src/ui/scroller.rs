//! The editors' scroll and zoom controller. One [`Scroller`] per editor (playlist, piano roll, drum editor, automation
//! editor, audio editor): each frame the editor declares its [`Zones`] and lends its view as two [`Axis`]es (x, y) in
//! its own content units (beats, rows), and writes back what changed.

use eframe::egui::{self, Color32, Id, Pos2, Rangef, Rect, Sense, Vec2};

use crate::ui::{gesture, primary_down_live};

/// The index of an axis in the arrays here.
pub const X: usize = 0;
pub const Y: usize = 1;

/// One axis of an editor's view: `pos` = the content at the view's left (top) edge, `zoom` = content units a point.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Axis {
    pub pos: f64,
    pub zoom: f64,
    /// The scroll range: min and the range end.
    pub range: [f64; 2],
    /// zoomMin, zoomMax.
    pub zoom_range: [f64; 2],
    /// The view on screen along this axis: W (H) = its span.
    pub view: Rangef,
}

/// The editor's input zones: which input may do what where. `Rect::NOTHING` turns one off.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Zones {
    /// One-finger scroll per axis (x, y); a touch also zooms only the axes whose zone it started in.
    pub touch: [Rect; 2],
    /// The wheel zooms an axis over its zone (x, y). The first wheel zone under the cursor wins: zoom y, zoom x, then
    /// scroll.
    pub wheel_zoom: [Rect; 2],
    pub wheel_scroll: Rect,
    /// The trackpad pinch zooms x.
    pub pinch: Rect,
}

impl Zones {
    pub const NONE: Self = Self {
        touch: [Rect::NOTHING; 2],
        wheel_zoom: [Rect::NOTHING; 2],
        wheel_scroll: Rect::NOTHING,
        pinch: Rect::NOTHING,
    };
}

/// What the host says about a frame.
#[derive(Clone, Copy, Debug, Default)]
pub struct Options {
    /// The pointer is over an overlay (or a popup is open): no wheel, pinch or new touch is the editor's.
    pub covered: bool,
    /// Follow playhead runs: one-finger horizontal scrolling is off.
    pub follow: bool,
}

/// A touch the scroller had and nothing took, released before either axis left its calm zone.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Tap {
    /// The touch-down point.
    pub at: Pos2,
    /// Held more than 0.3 s: a long tap ([`gesture::is_long`]; exactly 0.3 s is a tap).
    pub long: bool,
}

/// What a frame did.
#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub struct Report {
    pub tap: Option<Tap>,
    /// Input scrolled (zoomed) the axis: a drag, a fling, the wheel, a pinch.
    pub scrolled: [bool; 2],
    pub zoomed: [bool; 2],
}

// The laws' numbers; lengths in U. Each axis stays still until the finger is out of its calm zone on it,
// [`gesture::CALM_U`] from the touch-down; a tap's longest hold is [`gesture::is_long`]'s.
/// A release flings an axis whose last move went further than this.
const FLING_U: f32 = 0.125;
/// Past either end the content moves this share of the finger's way.
const RUBBER: f64 = 0.2;
/// Inertia inside the range: v ← 0.9·v a frame.
const DECAY: f64 = 0.9;
/// The overscroll spring: v ← 0.5·(pos − max), then pos −= 0.2·v.
const SPRING: f64 = 0.5;
const SPRING_STEP: f64 = 0.2;
/// |v| below this stops.
const STOP: f64 = 0.001;
/// The two-finger zoom's threshold T, and T while the other axis zoomed on the previous move (the axis lock).
const PINCH_U: f32 = 2.0;
const PINCH_LOCK_U: f32 = 6.0;
/// The wheel's zoom: zoom ← zoom·(1 − 0.001·dy), the factor kept above 0.05.
pub(crate) const WHEEL_ZOOM: f32 = 0.001;
pub(crate) const WHEEL_FLOOR: f32 = 0.05;
/// Follow playhead puts the playhead this far into the view.
const FOLLOW_AT: f64 = 0.25;
/// The release flings only by a move in its own frame or the one before, and no older than this (a finger
/// that rested before the lift flings nothing; at down to 12.5 frames a second a fast lift still flings).
// ponytail: a fixed 80 ms cap, standing in for the small moves a resting finger keeps sending on a phone.
const FLING_STALE_S: f64 = 0.08;
/// The scrollbar's ink while pressed.
const PRESSED: f32 = 1.1;

impl Axis {
    /// No such axis (the automation and audio editors' y): nothing to scroll or zoom.
    pub const OFF: Self = Self {
        pos: 0.0,
        zoom: 1.0,
        range: [0.0; 2],
        zoom_range: [1.0; 2],
        view: Rangef { min: 0.0, max: 0.0 },
    };

    /// The content the view shows: W·zoom.
    pub fn span(&self) -> f64 {
        f64::from(self.view.span()) * self.zoom
    }

    /// The last scroll position: rangeEnd − W·zoom, never below min.
    pub fn max(&self) -> f64 {
        (self.range[1] - self.span()).max(self.range[0])
    }

    /// `zoom` within the limits; zoomMin wins over a zoomMax capped below it.
    pub fn clamp_zoom(&self, zoom: f64) -> f64 {
        zoom.min(self.zoom_range[1]).max(self.zoom_range[0])
    }

    /// The zoom and the position within their limits.
    pub fn clamped(mut self) -> Self {
        self.zoom = self.clamp_zoom(self.zoom);
        self.pos = self.pos.clamp(self.range[0], self.max());
        self
    }

    /// The content at screen coordinate `at`.
    pub fn at(&self, at: f32) -> f64 {
        self.pos + f64::from(at - self.view.min) * self.zoom
    }

    /// Zooms to `zoom` (clamped), the content at screen coordinate `at` staying there; pos is clamped to the range.
    pub fn zoom_about(&mut self, zoom: f64, at: f32) {
        let under = self.at(at);
        self.zoom = self.clamp_zoom(zoom);
        let pos = under - f64::from(at - self.view.min) * self.zoom;
        self.pos = pos.clamp(self.range[0], self.max());
    }

    /// The finger moved `px` points: pos −= zoom·px, what lies past either end at a fifth (the rubber band).
    fn drag(&mut self, px: f32) {
        let (min, max) = (self.range[0], self.max());
        // In the finger's units the range is 1:1 and past it 5:1.
        let finger = if self.pos > max {
            max + (self.pos - max) / RUBBER
        } else if self.pos < min {
            min - (min - self.pos) / RUBBER
        } else {
            self.pos
        };
        let q = finger - self.zoom * f64::from(px);
        self.pos = if q > max {
            max + RUBBER * (q - max)
        } else if q < min {
            min - RUBBER * (min - q)
        } else {
            q
        };
    }

    fn outside(&self) -> bool {
        self.pos < self.range[0] || self.pos > self.max()
    }
}

/// The scroller's touch: the first finger (or the mouse), from a touch-down in a touch zone until its lift.
#[derive(Clone, Copy, Debug, PartialEq)]
struct Touch {
    origin: Pos2,
    last: Pos2,
    since: f64,
    /// It started in the touch zone of each axis.
    zone: [bool; 2],
    /// Each axis has left its calm zone.
    left: [bool; 2],
    /// Nothing took it: the scroller's own widget had the press, or the host claimed it. `yielded`: a child took it
    /// later.
    owned: bool,
    yielded: bool,
    /// A second finger came: from now on it only zooms.
    pinch: bool,
    /// The last move: its delta, when, and in which frame.
    moved: Option<(Vec2, f64, u64)>,
    /// The axes the previous two-finger move zoomed.
    zoomed: [bool; 2],
}

impl Touch {
    /// Whether the finger scrolls axis `a`: one finger the scroller has, started in the axis's zone (x is off while
    /// following the playhead).
    fn scrolls(&self, a: usize, follow: bool) -> bool {
        self.owned && !self.pinch && self.zone[a] && !(a == X && follow)
    }
}

/// The controller's state between frames: the touch, the fingers down, the fling.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct Scroller {
    touch: Option<Touch>,
    /// The touch the lift ended this frame: the editor decides its tap after the scroller's update.
    lifted: Option<Touch>,
    /// The fingers down (touch events), first down first.
    fingers: Vec<(egui::TouchId, Pos2)>,
    /// The fling per axis, content units a frame.
    v: [f64; 2],
    /// An axis let go past its end springs back.
    spring: [bool; 2],
    /// The frame the inertia last stepped.
    frame: u64,
}

impl Scroller {
    /// One frame of input over `zones` (module docs), `id` being the scroller's widget, which lies under everything the
    /// editor draws later, and `u` the U in points. Call it before the editor's own widgets, every frame the editor
    /// shows. The axes' positions may leave their ranges while a finger pulls them past an end and until the spring has
    /// brought them back; otherwise they are kept in range.
    pub fn update(
        &mut self,
        ui: &egui::Ui,
        id: Id,
        u: f32,
        zones: &Zones,
        axes: &mut [Axis; 2],
        opts: Options,
    ) -> Report {
        let mut out = Report::default();
        let before = *axes;
        self.lifted = None;
        let frame = ui.ctx().cumulative_frame_nr();
        let all = [
            zones.wheel_scroll,
            zones.pinch,
            zones.touch[X],
            zones.touch[Y],
        ]
        .into_iter()
        .chain(zones.wheel_zoom)
        .fold(Rect::NOTHING, Rect::union);
        let area = zones.touch[X].union(zones.touch[Y]);
        let response = ui.interact(area, id, Sense::drag());
        let down = primary_down_live(ui, id);
        let (pressed, pos, now, hover) = ui.input(|i| {
            let p = &i.pointer;
            (p.primary_pressed(), p.interact_pos(), i.time, p.hover_pos())
        });
        let (wheel, pinch, any_touch) = ui.input(|i| {
            (
                if i.modifiers.command {
                    Vec2::ZERO
                } else {
                    i.raw_scroll_delta
                },
                if i.multi_touch().is_none() {
                    i.zoom_delta()
                } else {
                    1.0
                },
                i.any_touches(),
            )
        });
        let touches: Vec<_> = ui.input(|i| {
            (i.events.iter())
                .filter_map(|e| match *e {
                    egui::Event::Touch { id, phase, pos, .. } => Some((id, phase, pos)),
                    _ => None,
                })
                .collect()
        });

        if opts.follow {
            (self.v[X], self.spring[X]) = (0.0, false);
        }

        // The wheel by its first zone under the cursor, and the trackpad pinch. Any wheel event stops a fling.
        if wheel != Vec2::ZERO {
            (self.v, self.spring) = ([0.0; 2], [false; 2]);
        }
        if !opts.covered
            && let Some(p) = hover
        {
            let over = |r: Rect| r.contains(p) && ui.rect_contains_pointer(r);
            if wheel != Vec2::ZERO {
                let factor = f64::from((1.0 - WHEEL_ZOOM * wheel.y).max(WHEEL_FLOOR));
                if let Some(a) = [Y, X].into_iter().find(|&a| over(zones.wheel_zoom[a])) {
                    let ax = &mut axes[a];
                    ax.zoom_about(ax.zoom * factor, p[a]);
                } else if over(zones.wheel_scroll) {
                    for (a, ax) in axes.iter_mut().enumerate() {
                        ax.pos =
                            (ax.pos - ax.zoom * f64::from(wheel[a])).clamp(ax.range[0], ax.max());
                    }
                }
            }
            if pinch != 1.0 && over(zones.pinch) {
                let x = &mut axes[X];
                x.zoom_about(x.zoom / f64::from(pinch), p.x);
            }
        }
        // The keymap's zoom actions (`ui/edit_keys.rs`): z ← z·(1 ∓ 0.1) a press, about the view's centre. An axis the
        // editor lacks (`Axis::OFF`) has no zoom range to move in.
        if !opts.covered {
            let k = crate::ui::edit_keys::zoom(ui.ctx());
            for a in [X, Y] {
                if k[a] != 1.0 {
                    let ax = &mut axes[a];
                    ax.zoom_about(ax.zoom * k[a], ax.view.center());
                }
            }
        }
        for a in [X, Y] {
            out.scrolled[a] |= axes[a].pos != before[a].pos && axes[a].zoom == before[a].zoom;
            out.zoomed[a] |= axes[a].zoom != before[a].zoom;
        }

        // A touch-down in a touch zone is the scroller's touch; a child took it unless the scroller's widget had it.
        // (Pressed and let go in one frame, a trackpad's tap-to-click, it is the scroller's where its widget is the
        // top one that takes drags.)
        if pressed
            && !opts.covered
            && let Some(p) = pos
        {
            let zone = [zones.touch[X].contains(p), zones.touch[Y].contains(p)];
            self.touch = (zone[X] || zone[Y]).then_some(Touch {
                origin: p,
                last: p,
                since: now,
                zone,
                left: [false; 2],
                owned: response.is_pointer_button_down_on() || (!down && response.hovered()),
                yielded: false,
                pinch: false,
                moved: None,
                zoomed: [false; 2],
            });
            if self.touch.is_some() {
                self.v = [0.0; 2];
            }
        }

        // The fingers: a second one makes the touch a zoom; each move of either zooms about the other.
        for (finger, phase, p) in touches {
            match phase {
                // A finger that goes down off the editor (another control, an overlay) is not one of its fingers.
                egui::TouchPhase::Start
                    if !all.contains(p) || ui.ctx().layer_id_at(p) != Some(ui.layer_id()) => {}
                egui::TouchPhase::Start => {
                    self.fingers.retain(|f| f.0 != finger);
                    self.fingers.push((finger, p));
                    if let Some(t) = self.touch.as_mut().filter(|_| self.fingers.len() > 1) {
                        t.pinch = true;
                    }
                }
                egui::TouchPhase::Move => {
                    let Some(i) = self.fingers.iter().position(|f| f.0 == finger) else {
                        continue;
                    };
                    let from = std::mem::replace(&mut self.fingers[i].1, p);
                    let other = (self.fingers.iter()).find(|f| f.0 != finger).map(|f| f.1);
                    if let (Some(t), Some(anchor)) =
                        (self.touch.as_mut().filter(|t| t.pinch), other)
                    {
                        pinch_move(t, (from, p), anchor, u, axes, &mut out);
                    }
                }
                egui::TouchPhase::End | egui::TouchPhase::Cancel => {
                    self.fingers.retain(|f| f.0 != finger);
                }
            }
        }
        if !any_touch {
            self.fingers.clear();
        }

        // One finger: each axis calm until 0.5 U out, then 1:1, the rubber band past the ends.
        if let Some(t) = self.touch.as_mut()
            && let Some(p) = pos
        {
            let delta = p - t.last;
            t.last = p;
            if delta != Vec2::ZERO {
                t.moved = Some((delta, now, frame));
            }
            for a in [X, Y] {
                t.left[a] |= (p[a] - t.origin[a]).abs() > u * gesture::CALM_U;
                if t.left[a] && t.scrolls(a, opts.follow) && delta[a] != 0.0 {
                    axes[a].drag(delta[a]);
                    out.scrolled[a] = true;
                }
            }
        }

        // The lift: a tap if neither axis left its calm zone, else a fling per axis whose last move was fast.
        if let Some(t) = self.touch.filter(|_| !down) {
            if t.owned && !t.pinch {
                if t.left == [false; 2] {
                    out.tap = Some(Tap {
                        at: t.origin,
                        long: gesture::is_long(now - t.since),
                    });
                } else if let Some((d, ..)) =
                    (t.moved).filter(|m| frame - m.2 <= 1 && now - m.1 <= FLING_STALE_S)
                {
                    // `Animation` `Minimal`: no inertia, the scroll stops where the finger left it.
                    let inertia = crate::ui::anim::inertia(ui.ctx());
                    for a in [X, Y] {
                        if inertia
                            && t.left[a]
                            && t.scrolls(a, opts.follow)
                            && d[a].abs() > u * FLING_U
                        {
                            self.v[a] = axes[a].zoom * f64::from(d[a]);
                        }
                    }
                }
            }
            self.spring = axes.map(|ax| ax.outside());
            (self.touch, self.lifted) = (None, Some(t));
        }

        // Inertia and the spring, once a drawn frame while not touching (the release frame included).
        if self.touch.is_none() && frame != self.frame {
            self.frame = frame;
            for a in [X, Y] {
                out.scrolled[a] |= self.coast(a, &mut axes[a]);
            }
        }

        // Anything not pulled, flung or springing stays in range.
        for a in [X, Y] {
            let pulled = self
                .touch
                .is_some_and(|t| t.left[a] && t.scrolls(a, opts.follow));
            if self.v[a] == 0.0 && !self.spring[a] && !pulled {
                axes[a] = axes[a].clamped();
            }
        }
        if self.v != [0.0; 2] || self.spring.contains(&true) {
            ui.ctx().request_repaint();
        }
        out
    }

    /// One frame of the fling or the spring on axis `a`: inside the range v ← 0.9·v, pos −= v; past an end
    /// v ← 0.5·(pos − max) (or −0.5·(min − pos)), pos −= 0.2·v; |v| < 0.001 stops, on the end (the last sliver of
    /// overshoot snaps). Returns whether it moved.
    fn coast(&mut self, a: usize, ax: &mut Axis) -> bool {
        if self.v[a] == 0.0 && !self.spring[a] {
            return false;
        }
        let (min, max) = (ax.range[0], ax.max());
        let v = &mut self.v[a];
        if ax.pos > max {
            *v = SPRING * (ax.pos - max);
            ax.pos -= SPRING_STEP * *v;
        } else if ax.pos < min {
            *v = -SPRING * (min - ax.pos);
            ax.pos -= SPRING_STEP * *v;
        } else {
            *v *= DECAY;
            ax.pos -= *v;
        }
        if v.abs() < STOP {
            *v = 0.0;
            self.spring[a] = false;
            ax.pos = ax.pos.clamp(min, max);
        }
        true
    }

    /// Axis `a`'s range shrank under a position the editor keeps (the step grid's 2L when its loop shrinks; the
    /// position is not clamped): a position past an end, untouched, springs back over the next frames instead of
    /// jumping. Call it before [`Self::update`], every frame: it leaves a live touch alone (the finger holds the
    /// view; its lift springs every axis anyway) and the other axis to the usual clamp. That is why it is not
    /// [`Self::let_go`], which ends the touch and springs both axes.
    pub fn spring_back(&mut self, a: usize, ax: &Axis) {
        if self.touch.is_none() && ax.outside() {
            self.spring[a] = true;
        }
    }

    /// The current touch is the scroller's, though a widget over its zone had the press (a grid that takes every
    /// press hands on one it does not use, such as empty space, or a ruler press still waiting for its long press).
    pub fn claim(&mut self) {
        if let Some(t) = self.touch.as_mut().filter(|t| !t.yielded) {
            t.owned = true;
        }
    }

    /// A child took the current touch (a marquee, a ruler range): no more scrolling, no tap and no fling from it. A
    /// second finger still zooms.
    pub fn yield_touch(&mut self) {
        if let Some(t) = self.touch.as_mut() {
            (t.owned, t.yielded) = (false, true);
        }
    }

    /// A child that took the touch let it go, released or cancelled, leaving `axes` where it moved them (the rack's
    /// held card): the touch is over for the scroller too, and an axis left past an end springs back with the normal
    /// law. Called once, at the hand-back, with the finger possibly still down (so not [`Self::spring_back`], which
    /// waits for no touch).
    pub fn let_go(&mut self, axes: &[Axis; 2]) {
        self.touch = None;
        for a in [X, Y] {
            self.spring[a] |= axes[a].outside();
        }
    }

    /// While the scroller has a one-finger touch, or in the frame its lift ended it: whether either axis has left its
    /// calm zone (it scrolls, and is no tap). `None` without such a touch.
    pub fn left_calm(&self) -> Option<bool> {
        (self.touch.or(self.lifted))
            .filter(|t| t.owned && !t.pinch)
            .map(|t| t.left.contains(&true))
    }
}

/// One move of a finger of a two-finger touch from `from` to `to`, `anchor` the other finger: per axis in the first
/// finger's zone, with d = to − anchor and the previous separation d − Δ both over T (2 U, or 6 U while the other
/// axis zoomed on the previous move), zoom ← zoom·|(d − Δ)/d| about the anchor.
fn pinch_move(
    t: &mut Touch,
    (from, to): (Pos2, Pos2),
    anchor: Pos2,
    u: f32,
    axes: &mut [Axis; 2],
    out: &mut Report,
) {
    let delta = to - from;
    let mut zoomed = [false; 2];
    for a in [X, Y] {
        let threshold = u * if t.zoomed[1 - a] {
            PINCH_LOCK_U
        } else {
            PINCH_U
        };
        let d = to[a] - anchor[a];
        let prev = d - delta[a];
        if t.zone[a] && delta[a] != 0.0 && d.abs() > threshold && prev.abs() > threshold {
            let ax = &mut axes[a];
            let was = ax.zoom;
            ax.zoom_about(ax.zoom * f64::from((prev / d).abs()), anchor[a]);
            zoomed[a] = true;
            out.zoomed[a] |= ax.zoom != was;
        }
    }
    t.zoomed = zoomed;
}

/// The scrollbar along `bar`, vertical when it is taller than wide, for `axis`: drawn only while the view shows less
/// than the whole range. The thumb is the visible fraction of the bar, placed at pos, with rounded ends (radius half
/// the bar's thickness), in `ink`, 1.1 times brighter while pressed. A drag on it (within `hit` points across, centred
/// on the bar) moves pos by Δpx·range/barLength, within the range, so the thumb follows the finger. Returns the thumb
/// as drawn.
pub fn scrollbar(
    ui: &mut egui::Ui,
    id: Id,
    bar: Rect,
    hit: f32,
    axis: &mut Axis,
    ink: Color32,
) -> Option<Rect> {
    let a = if bar.height() > bar.width() { Y } else { X };
    let (len, thick) = (f64::from(bar.size()[a]), bar.size()[1 - a]);
    let whole = axis.range[1] - axis.range[0];
    if axis.span() >= whole || len <= 0.0 {
        return None;
    }
    let thumb = |axis: &Axis| {
        let size = axis.span() / whole * len;
        let lo = ((axis.pos - axis.range[0]) / whole * len).clamp(0.0, len - size);
        let along = Rangef::new(bar.min[a] + lo as f32, bar.min[a] + (lo + size) as f32);
        match a {
            X => Rect::from_x_y_ranges(along, bar.y_range()),
            _ => Rect::from_x_y_ranges(bar.x_range(), along),
        }
    };
    let mut grow = Vec2::ZERO;
    grow[1 - a] = ((hit - thick) / 2.0).max(0.0);
    let response = ui.interact(thumb(axis).expand2(grow), id, Sense::drag());
    let pressed = response.is_pointer_button_down_on();
    let d = ui.input(|i| i.pointer.delta())[a];
    if pressed && d != 0.0 {
        axis.pos = (axis.pos + f64::from(d) * whole / len).clamp(axis.range[0], axis.max());
    }
    let drawn = thumb(axis);
    let color = if pressed { brighter(ink, PRESSED) } else { ink };
    let r = thick / 2.0;
    let (mut start, mut end) = (drawn.center(), drawn.center());
    start[a] = drawn.min[a] + r;
    end[a] = (drawn.max[a] - r).max(start[a]);
    let painter = ui.painter();
    painter.rect_filled(
        Rect::from_two_pos(start, end).expand2(grow_across(a, r)),
        0.0,
        color,
    );
    painter.circle_filled(start, r, color);
    painter.circle_filled(end, r, color);
    Some(drawn)
}

/// `r` across axis `a`, nothing along it.
fn grow_across(a: usize, r: f32) -> Vec2 {
    let mut v = Vec2::ZERO;
    v[1 - a] = r;
    v
}

/// `c` with its colour channels times `k`.
fn brighter(c: Color32, k: f32) -> Color32 {
    let f = |v: u8| (f32::from(v) * k).round().min(255.0) as u8;
    Color32::from_rgba_premultiplied(f(c.r()), f(c.g()), f(c.b()), c.a())
}

/// Follow playhead: the view's left edge goes to the playhead less a quarter of the visible span, so the playhead sits
/// at 25 % of the width; kept in the range.
pub fn follow(axis: &mut Axis, playhead: f64) {
    axis.pos = (playhead - FOLLOW_AT * axis.span()).clamp(axis.range[0], axis.max());
}

#[cfg(test)]
mod tests;
