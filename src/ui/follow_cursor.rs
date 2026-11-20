//! Playlist-only screen-space follow anchor. Never emits a transport command.
use eframe::egui::{self, Rect, Stroke, pos2, vec2};
use crate::ui::UiCtx;

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
        Rect::from_center_size(pos2(x, marks.top() + cx.m.u(0.32)), vec2(cx.m.u(1.2), cx.m.u(0.64)))
            .intersect(marks)
    }

    /// Returns true for the complete owned gesture, including its release. Raw hit-testing is deliberate:
    /// the ruler and seek knob overlap this small, explicitly reserved handle and must not claim its press.
    pub(super) fn input(&mut self, ui: &egui::Ui, marks: Rect, enabled: bool, covered: bool, cx: &UiCtx) -> bool {
        let was = self.grab.is_some();
        if !enabled || covered || marks.width() <= 0.0 {
            self.grab = None;
            return was;
        }
        let (pressed, down, pos, pinch) = ui.input(|i| (
            i.pointer.primary_pressed(), i.pointer.primary_down(), i.pointer.interact_pos(),
            i.multi_touch().is_some_and(|t| t.num_touches > 1),
        ));
        if pinch {
            self.grab = None;
            return was;
        }
        if pressed && let Some(p) = pos.filter(|p| self.rect(marks, cx).contains(*p)) {
            let anchor = marks.left() + marks.width() * self.fraction() as f32;
            self.grab = Some(p.x - anchor);
        }
        let owned = self.grab.is_some();
        if let Some(grab) = self.grab {
            if let Some(p) = pos {
                self.position = Some((((p.x - grab - marks.left()) / marks.width()).clamp(0.05, 0.95) * 1000.0).round() as u16);
            }
            if !down { self.grab = None; }
            ui.ctx().request_repaint();
        }
        owned
    }

    pub(super) fn paint(&self, ui: &egui::Ui, marks: Rect, cx: &UiCtx) {
        let rect = self.rect(marks, cx);
        let ink = egui::Color32::from_rgb(255, 213, 64);
        let painter = ui.painter_at(marks);
        painter.rect_filled(rect, cx.m.u(0.12), cx.theme.colors.background.to_color32());
        painter.rect_stroke(rect, cx.m.u(0.12), Stroke::new(cx.m.t(), ink), egui::StrokeKind::Inside);
        let c = rect.center();
        let d = cx.m.u(0.15);
        let left = pos2(rect.left() + d, c.y);
        let right = pos2(rect.right() - d, c.y);
        let stroke = Stroke::new(cx.m.t(), ink);
        for points in [[left, right], [left, left + vec2(d, -d)], [left, left + vec2(d, d)],
            [right, right + vec2(-d, -d)], [right, right + vec2(-d, d)]] {
            painter.line_segment(points, stroke);
        }
        if ui.rect_contains_pointer(rect) {
            ui.ctx().set_cursor_icon(egui::CursorIcon::ResizeHorizontal);
        }
        crate::ui::tooltip::tip(ui, rect, "Follow position: drag left/right. Changes the Playlist view only, never seeks playback.", None);
    }
}
