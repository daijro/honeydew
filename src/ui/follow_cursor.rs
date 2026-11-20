//! Playlist-only screen-space follow anchor. Never emits a transport command.
use crate::ui::UiCtx;
use eframe::egui::{self, Rect, Stroke, pos2, vec2};

#[derive(Default)]
pub(super) struct FollowCursor {
    pub(super) position: Option<u16>,
    grab: Option<f32>,
}

impl FollowCursor {
    pub(super) fn fraction(&self) -> f64 {
        f64::from(self.position.unwrap_or(250).clamp(50, 950)) / 1000.0
    }

    pub(super) fn rect(&self, marks: Rect, cx: &UiCtx) -> Rect {
        let x = marks.left() + marks.width() * self.fraction() as f32;
        Rect::from_center_size(
            pos2(x, marks.top() + cx.m.u(0.32)),
            vec2(cx.m.u(1.2), cx.m.u(0.64)),
        )
        .intersect(marks)
    }

    /// Invisible padding: same visual cap, a 1.8 U wide by 1.3 U tall grab zone inside the ruler.
    fn hit_rect(&self, marks: Rect, cx: &UiCtx) -> Rect {
        let r = self.rect(marks, cx);
        Rect::from_center_size(r.center(), vec2(cx.m.u(1.8), cx.m.u(1.3))).intersect(marks)
    }

    /// Returns true for the complete owned gesture, including its release. Raw hit-testing is deliberate:
    /// the ruler and seek knob overlap this small, explicitly reserved handle and must not claim its press.
    pub(super) fn input(
        &mut self,
        ui: &egui::Ui,
        marks: Rect,
        enabled: bool,
        covered: bool,
        cx: &UiCtx,
    ) -> bool {
        let was = self.grab.is_some();
        if !enabled || covered || marks.width() <= 0.0 {
            self.grab = None;
            return was;
        }
        let (pressed, down, pos, pinch) = ui.input(|i| {
            (
                i.pointer.primary_pressed(),
                i.pointer.primary_down(),
                i.pointer.interact_pos(),
                i.multi_touch().is_some_and(|t| t.num_touches > 1),
            )
        });
        if pinch {
            self.grab = None;
            return was;
        }
        if pressed
            && ui.rect_contains_pointer(self.hit_rect(marks, cx))
            && let Some(p) = pos.filter(|p| self.hit_rect(marks, cx).contains(*p))
        {
            let anchor = marks.left() + marks.width() * self.fraction() as f32;
            self.grab = Some(p.x - anchor);
        }
        let owned = self.grab.is_some();
        if let Some(grab) = self.grab {
            if let Some(p) = pos {
                self.position = Some(
                    (((p.x - grab - marks.left()) / marks.width()).clamp(0.05, 0.95) * 1000.0)
                        .round() as u16,
                );
            }
            if !down {
                self.grab = None;
            }
            ui.ctx().request_repaint();
        }
        owned
    }

    pub(super) fn paint(&self, ui: &egui::Ui, marks: Rect, cx: &UiCtx) {
        let rect = self.rect(marks, cx);
        let ink = egui::Color32::from_rgb(255, 213, 64);
        let painter = ui.painter_at(marks);
        painter.rect_filled(rect, cx.m.u(0.12), cx.theme.colors.background.to_color32());
        painter.rect_stroke(
            rect,
            cx.m.u(0.12),
            Stroke::new(cx.m.t(), ink),
            egui::StrokeKind::Inside,
        );
        let c = rect.center();
        let d = cx.m.u(0.15);
        let left = pos2(rect.left() + d, c.y);
        let right = pos2(rect.right() - d, c.y);
        let stroke = Stroke::new(cx.m.t(), ink);
        for points in [
            [left, right],
            [left, left + vec2(d, -d)],
            [left, left + vec2(d, d)],
            [right, right + vec2(-d, -d)],
            [right, right + vec2(-d, d)],
        ] {
            painter.line_segment(points, stroke);
        }
        if ui.rect_contains_pointer(rect) {
            ui.ctx().set_cursor_icon(egui::CursorIcon::ResizeHorizontal);
        }
        crate::ui::tooltip::tip(
            ui,
            rect,
            "Follow position: drag left/right. Changes the Playlist view only, never seeks playback.",
            None,
        );
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::metrics::{DEFAULT_UI_SCALE, Metrics};
    use eframe::egui::{Context, Event, Modifiers, PointerButton, Pos2, RawInput};

    fn frame(
        ctx: &Context,
        cursor: &mut FollowCursor,
        events: Vec<Event>,
        enabled: bool,
        covered: bool,
    ) -> bool {
        let mut owned = false;
        let _ = ctx.run(
            RawInput {
                screen_rect: Some(Rect::from_min_size(Pos2::ZERO, vec2(1000.0, 500.0))),
                events,
                ..Default::default()
            },
            |ctx| {
                egui::CentralPanel::default().show(ctx, |ui| {
                    let m = Metrics::from_pixels_per_point(1.0, DEFAULT_UI_SCALE);
                    let theme = crate::theme::theme_by_id(0);
                    let cx = UiCtx {
                        m: &m,
                        theme: &theme,
                    };
                    owned = cursor.input(
                        ui,
                        Rect::from_min_max(pos2(100.0, 0.0), pos2(900.0, 53.0)),
                        enabled,
                        covered,
                        &cx,
                    );
                });
            },
        );
        owned
    }

    fn button(p: egui::Pos2, pressed: bool) -> Event {
        Event::PointerButton {
            pos: p,
            button: PointerButton::Primary,
            pressed,
            modifiers: Modifiers::NONE,
        }
    }

    #[test]
    fn only_enabled_uncovered_handle_can_start_a_follow_adjustment() {
        for (enabled, covered) in [(false, false), (true, true)] {
            let ctx = Context::default();
            let mut cursor = FollowCursor::default();
            frame(&ctx, &mut cursor, vec![], enabled, covered);
            let p = pos2(300.0, 10.0);
            assert!(!frame(
                &ctx,
                &mut cursor,
                vec![Event::PointerMoved(p), button(p, true)],
                enabled,
                covered
            ));
            frame(
                &ctx,
                &mut cursor,
                vec![Event::PointerMoved(pos2(800.0, 10.0))],
                enabled,
                covered,
            );
            assert_eq!(cursor.fraction(), 0.25);
        }
        let ctx = Context::default();
        let mut cursor = FollowCursor::default();
        frame(&ctx, &mut cursor, vec![], true, false);
        let outside = pos2(600.0, 35.0);
        assert!(!frame(
            &ctx,
            &mut cursor,
            vec![Event::PointerMoved(outside), button(outside, true)],
            true,
            false
        ));
        assert_eq!(cursor.fraction(), 0.25);
    }

    #[test]
    fn padded_follow_handle_can_be_grabbed_outside_unchanged_visual_cap() {
        let ctx = Context::default();
        let mut cursor = FollowCursor::default();
        frame(&ctx, &mut cursor, vec![], true, false);
        // Visual cap ends at x319.2/y20.48 at this scale; enlarged touch zone extends to x328.8/y31.
        let p = pos2(324.0, 25.0);
        assert!(frame(
            &ctx,
            &mut cursor,
            vec![Event::PointerMoved(p), button(p, true)],
            true,
            false
        ));
        let end = pos2(484.0, 25.0);
        frame(
            &ctx,
            &mut cursor,
            vec![Event::PointerMoved(end), button(end, false)],
            true,
            false,
        );
        assert!((cursor.fraction() - 0.45).abs() < 0.002);
    }

    #[test]
    fn adjustment_owns_release_and_clamps_to_visible_timeline() {
        let ctx = Context::default();
        let mut cursor = FollowCursor::default();
        frame(&ctx, &mut cursor, vec![], true, false);
        let p = pos2(300.0, 10.0);
        assert!(frame(
            &ctx,
            &mut cursor,
            vec![Event::PointerMoved(p), button(p, true)],
            true,
            false
        ));
        assert!(frame(
            &ctx,
            &mut cursor,
            vec![Event::PointerMoved(pos2(2000.0, 10.0))],
            true,
            false
        ));
        assert_eq!(cursor.position, Some(950));
        let end = pos2(-200.0, 10.0);
        assert!(frame(
            &ctx,
            &mut cursor,
            vec![Event::PointerMoved(end), button(end, false)],
            true,
            false
        ));
        assert_eq!(cursor.position, Some(50));
        assert!(!frame(&ctx, &mut cursor, vec![], true, false));
    }
}
