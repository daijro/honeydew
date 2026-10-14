//! The Snap button of the ruler row's top-left cell, shared by the clip editors (and the playlist's ruler).
//!
//! A disc of radius 0.325·min(w, h) of its cell, with a 2 px `text` halo ring. Off: a `background` disc with icon 15 in
//! `text`. On: a `text` disc with the icon in `menuFill`. A tap toggles Snap; a long press (300 ms within the 0.5 U
//! calm zone) or a right click opens the snap-mode menu, which the caller shows.
//!
//! Drawing (shared with the playlist ruler's button): the halo is a `text` disc of radius r + 2 px under the fill disc
//! of radius r (two filled discs, no seam); the icon's size factor is 0.6. The whole cell takes the touch.
//! [`paint_snap_button`] and [`snap_touch`] are its two halves, for a caller that paints and senses at different points
//! of its draw order; [`snap_button`] does both.

use eframe::egui::{Id, Painter, Pos2, Rect, Sense, Ui, Vec2};

use crate::icons::{IconId, draw_icon};
use crate::ui::{UiCtx, gesture};

/// The icon's size factor.
const ICON: f32 = 0.6;
/// The halo's width, in device px.
const HALO_PX: f32 = 2.0;

/// What a touch on the Snap button asks for.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum SnapTouch {
    /// A tap: toggle Snap.
    Toggle,
    /// A long press or a right click: open the snap-mode menu.
    Menu,
}

/// The disc's centre and radius in `cell`.
pub fn snap_disc(cell: Rect) -> (Pos2, f32) {
    (cell.center(), 0.325 * cell.width().min(cell.height()))
}

/// The Snap button in `cell`, drawn `on` or off and sensing its touches (module docs). Returns what a touch asked for
/// this frame.
pub fn snap_button(ui: &mut Ui, id: Id, cell: Rect, on: bool, cx: &UiCtx) -> Option<SnapTouch> {
    paint_snap_button(ui.painter(), cell, on, cx);
    snap_touch(ui, id, cell, cx)
}

/// Paints the Snap button in `cell`, `on` or off: the `text` halo disc, the fill disc and icon 15.
pub fn paint_snap_button(painter: &Painter, cell: Rect, on: bool, cx: &UiCtx) {
    let (c, m) = (&cx.theme.colors, cx.m);
    let (center, r) = snap_disc(cell);
    let text = c.text.to_color32();
    let (fill, ink) = if on {
        (text, c.menu_fill.to_color32())
    } else {
        (c.background.to_color32(), text)
    };
    painter.circle_filled(center, r + HALO_PX / m.ppp(), text);
    painter.circle_filled(center, r, fill);
    draw_icon(painter, cell, IconId::Snap, ink, ICON, m);
}

/// The Snap button's touches over `cell`, by the shared button law ([`gesture::press`]): a tap toggles, a long press
/// (300 ms within the 0.5 U calm zone, at the threshold) or a right click asks for the menu. It senses drags, so that
/// no widget under it that senses drags takes its press; as on every control that senses drags, a press egui made a
/// drag (6 points of travel) is neither a tap nor a long press.
pub fn snap_touch(ui: &mut Ui, id: Id, cell: Rect, cx: &UiCtx) -> Option<SnapTouch> {
    let mut response = ui.interact(cell, id, Sense::click_and_drag());
    let press = gesture::press(ui, &mut response, cx.m, Vec2::length);
    if press.long || response.secondary_clicked() {
        Some(SnapTouch::Menu)
    } else {
        press.tap.then_some(SnapTouch::Toggle)
    }
}

#[cfg(test)]
mod tests {
    use eframe::egui::{
        CentralPanel, Context, Event, Frame, PointerButton, RawInput, Shape, pos2, vec2,
    };

    use super::*;
    use crate::metrics::{DEFAULT_UI_SCALE_INDEX, Metrics};
    use crate::theme::theme_by_id;

    struct Rig {
        ctx: Context,
        time: f64,
        m: Metrics,
        on: bool,
    }

    impl Rig {
        fn new(ppp: f32, on: bool) -> Self {
            Self {
                ctx: Context::default(),
                time: 0.0,
                m: Metrics::from_pixels_per_point(ppp, DEFAULT_UI_SCALE_INDEX),
                on,
            }
        }

        /// The ruler row's left cell at the origin: 1.5 U × (1.5 U − 1 px).
        fn cell(&self) -> Rect {
            let u = self.m.u(1.0);
            Rect::from_min_size(pos2(0.0, 0.0), vec2(1.5 * u, 1.5 * u - 1.0 / self.m.ppp()))
        }

        /// One frame `dt` after the last: the shapes, and the touch (which toggles `on`).
        fn frame(&mut self, dt: f64, events: Vec<Event>) -> (Vec<Shape>, Option<SnapTouch>) {
            self.time += dt;
            let raw = RawInput {
                screen_rect: Some(Rect::from_min_size(pos2(0.0, 0.0), vec2(400.0, 300.0))),
                time: Some(self.time),
                events,
                ..Default::default()
            };
            let theme = theme_by_id(0);
            let cx = UiCtx {
                theme: &theme,
                m: &self.m,
            };
            let (cell, on) = (self.cell(), self.on);
            let mut touch = None;
            let out = self.ctx.run(raw, |ctx| {
                CentralPanel::default().frame(Frame::NONE).show(ctx, |ui| {
                    touch = snap_button(ui, Id::new("snap"), cell, on, &cx);
                });
            });
            if touch == Some(SnapTouch::Toggle) {
                self.on = !self.on;
            }
            (out.shapes.into_iter().map(|s| s.shape).collect(), touch)
        }

        fn button(&mut self, dt: f64, pressed: bool) -> Option<SnapTouch> {
            let pos = self.cell().center();
            let button = PointerButton::Primary;
            let event = Event::PointerButton {
                pos,
                button,
                pressed,
                modifiers: Default::default(),
            };
            self.frame(dt, vec![Event::PointerMoved(pos), event]).1
        }
    }

    #[test]
    fn a_disc_of_0_325_min_w_h_with_a_2_px_text_halo_light_with_a_dark_icon_when_on_at_ppp_1_and_2()
    {
        let theme = theme_by_id(0);
        let c = &theme.colors;
        let (text, bg, menu) = (
            c.text.to_color32(),
            c.background.to_color32(),
            c.menu_fill.to_color32(),
        );
        for ppp in [1.0, 2.0] {
            for (on, fill, ink) in [(true, text, menu), (false, bg, text)] {
                let mut rig = Rig::new(ppp, on);
                let (shapes, _) = rig.frame(0.1, vec![]);
                let cell = rig.cell();
                let r = 0.325 * cell.height();
                let px = 1.0 / ppp;
                let circles: Vec<_> = (shapes.iter())
                    .filter_map(|s| match s {
                        Shape::Circle(c) => Some(*c),
                        _ => None,
                    })
                    .collect();
                // The halo, a `text` disc 2 px wider, under the fill disc.
                let discs: Vec<_> = (circles.iter())
                    .map(|c| (c.center, c.radius, c.fill, c.stroke.width))
                    .collect();
                assert_eq!(
                    discs,
                    [
                        (cell.center(), r + 2.0 * px, text, 0.0),
                        (cell.center(), r, fill, 0.0)
                    ],
                    "ppp {ppp} on {on}"
                );
                // At U = 32 points: r = 0.325·(48 − 1 px).
                assert!((r - 0.325 * (48.0 - px)).abs() < 1e-4, "{r}");
                // The magnet's pole tips are rects in the icon colour.
                let tips = (shapes.iter()).filter(|s| matches!(s, Shape::Rect(r) if r.fill == ink));
                assert_eq!(tips.count(), 2, "ppp {ppp} on {on}");
            }
        }
    }

    #[test]
    fn a_tap_toggles_and_a_long_press_or_a_right_click_asks_for_the_menu() {
        let mut rig = Rig::new(1.0, true);
        rig.frame(0.1, vec![]);
        assert_eq!(rig.button(0.05, true), None);
        assert_eq!(rig.button(0.05, false), Some(SnapTouch::Toggle));
        assert!(!rig.on);
        // Held past 300 ms: the menu, at the hold, and nothing at the release.
        rig.button(0.05, true);
        assert_eq!(rig.frame(0.31, vec![]).1, Some(SnapTouch::Menu));
        assert_eq!(rig.button(0.05, false), None);
        assert!(!rig.on);
        let at = rig.cell().center();
        let right = |pressed| Event::PointerButton {
            pos: at,
            button: PointerButton::Secondary,
            pressed,
            modifiers: Default::default(),
        };
        rig.frame(0.05, vec![right(true)]);
        assert_eq!(rig.frame(0.05, vec![right(false)]).1, Some(SnapTouch::Menu));
    }
}
