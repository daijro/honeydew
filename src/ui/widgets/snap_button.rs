//! The Snap button of the ruler row's top-left cell, shared by the clip editors (and the playlist's ruler).
//!
//! Icon only, no disc or ring, the icon's size factor 0.6: on, icon 15 in the caller's `ink` (`text` in the playlist,
//! the channel colour in the clip editors); off, icon 79 (the magnet with a slash) in `menuFill`. A tap toggles Snap; a
//! long press (300 ms within the 0.5 U calm zone) or a right click opens the snap-mode menu, which the caller shows.
//! The whole cell takes the touch.
//! [`paint_snap_button`] and [`snap_touch`] are its two halves, for a caller that paints and senses at different points
//! of its draw order; [`snap_button`] does both.

use eframe::egui::{Color32, Id, Painter, Rect, Sense, Ui, Vec2};

use crate::icons::{IconId, draw_icon};
use crate::ui::{UiCtx, gesture};

/// The icon's size factor.
const ICON: f32 = 0.6;

/// What a touch on the Snap button asks for.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum SnapTouch {
    /// A tap: toggle Snap.
    Toggle,
    /// A long press or a right click: open the snap-mode menu.
    Menu,
}

/// The Snap button in `cell`, drawn `on` (in `ink`) or off and sensing its touches (module docs). Returns what a touch
/// asked for this frame.
pub fn snap_button(
    ui: &mut Ui,
    id: Id,
    cell: Rect,
    (on, ink): (bool, Color32),
    cx: &UiCtx,
) -> Option<SnapTouch> {
    paint_snap_button(ui.painter(), cell, (on, ink), cx);
    snap_touch(ui, id, cell, cx)
}

/// Paints the Snap button in `cell`: icon 15 in `ink` while `on`, else icon 79 in `menuFill`.
pub fn paint_snap_button(painter: &Painter, cell: Rect, (on, ink): (bool, Color32), cx: &UiCtx) {
    let (icon, ink) = if on {
        (IconId::Snap, ink)
    } else {
        (IconId::SnapOff, cx.theme.colors.menu_fill.to_color32())
    };
    draw_icon(painter, cell, icon, ink, ICON, cx.m);
}

/// The Snap button's touches over `cell`, by the shared button law ([`gesture::press`]): a tap toggles, a long press
/// (300 ms within the 0.5 U calm zone, at the threshold) or a right click asks for the menu. It senses drags, so that
/// no widget under it that senses drags takes its press; as on every control that senses drags, a press egui made a
/// drag (6 points of travel) is neither a tap nor a long press.
pub fn snap_touch(ui: &mut Ui, id: Id, cell: Rect, cx: &UiCtx) -> Option<SnapTouch> {
    let mut response = ui.interact(cell, id, Sense::click_and_drag());
    crate::ui::tooltip::tip(ui, cell, "Beat snapping", Some("toggleBeatSnap"));
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
    use crate::metrics::{DEFAULT_UI_SCALE, Metrics};
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
                m: Metrics::from_pixels_per_point(ppp, DEFAULT_UI_SCALE),
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
                    touch = snap_button(ui, Id::new("snap"), cell, (on, Color32::RED), &cx);
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
