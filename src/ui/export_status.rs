//! Honest, bounded export feedback. The graphic represents timeline work, not PCM.
use eframe::egui::{self, Color32, Id, Sense, Stroke, pos2, vec2};
use crate::ui::UiCtx;
use crate::ui::views::Nav;
use crate::project_files::export::RENDERED_TAG;

pub(crate) fn fraction(done: usize, total: usize) -> Option<f32> {
    (total > 0).then(|| (done.min(total) as f64 / total as f64) as f32)
}

fn screen(ctx: &egui::Context, cx: &UiCtx, cancel: bool, content: impl FnOnce(&mut egui::Ui) -> Option<Nav>) -> Option<Nav> {
    let rect = ctx.content_rect();
    let mut action = None;
    egui::Area::new(Id::new("export_status"))
        .order(egui::Order::Foreground).fixed_pos(rect.min).fade_in(false)
        .sense(Sense::click_and_drag()).show(ctx, |ui| {
            ui.set_min_size(rect.size());
            ui.painter().rect_filled(rect, 0.0, cx.theme.colors.background.to_color32());
            egui::ScrollArea::vertical().max_height((rect.height() - if cancel { 64.0 } else { 0.0 }).max(32.0)).show(ui, |ui| {
                ui.add_space((rect.height() * 0.1).min(36.0));
                ui.horizontal(|ui| {
                    ui.add_space((rect.width() * 0.06).min(40.0));
                    ui.vertical(|ui| {
                        ui.set_width((rect.width() * 0.88).min(640.0));
                        action = content(ui);
                    });
                });
            });
            if cancel {
                // Unscrolled footer: cancellation stays reachable on short landscape screens.
                let footer = egui::Rect::from_min_max(
                    pos2(rect.left() + 12.0, (rect.bottom() - 52.0).max(rect.top())),
                    pos2(rect.right() - 12.0, rect.bottom() - 8.0));
                ui.painter().rect_filled(footer.expand(4.0), 0.0, cx.theme.colors.background.to_color32());
                if ui.put(footer, egui::Button::new("CANCEL EXPORT")).clicked() {
                    action = Some(Nav::Answer("export_cancel", true));
                }
            }
        });
    action
}

pub(crate) fn progress(ctx: &egui::Context, cx: &UiCtx, title: &str, done: usize, total: usize, stage: &str, format: &str) -> Option<Nav> {
    screen(ctx, cx, true, |ui| {
        ui.heading("EXPORT AUDIO");
        ui.add_space(12.0);
        ui.label(title);
        ui.label(format);
        ui.add_space(12.0);
        ui.label(stage);
        let confirmed = fraction(done, total);
        // Animate only toward confirmed work, never advance on a timer alone.
        let shown = ctx.animate_value_with_time(Id::new(("export_fraction", title, stage)), confirmed.unwrap_or(0.0),
            if done == 0 || total == 0 { 0.0 } else { 0.12 });
        let (r, _) = ui.allocate_exact_size(vec2(ui.available_width(), 100.0), Sense::hover());
        let ink = cx.theme.colors.text.to_color32();
        let bright = Color32::from_rgb(255, 210, 65);
        let bars = ((r.width() / 5.0) as usize).clamp(1, 128);
        for i in 0..bars {
            let t = i as f32 / bars as f32;
            // Decorative fixed silhouette: explicitly not a measured audio waveform.
            let height = 7.0 + 35.0 * ((i as f32 * 0.63).sin() * (i as f32 * 0.17).cos()).abs();
            let x = r.left() + (i as f32 + 0.5) * r.width() / bars as f32;
            let color = if t < shown { bright } else { ink.gamma_multiply(0.15) };
            ui.painter().line_segment([pos2(x, r.center().y - height), pos2(x, r.center().y + height)], Stroke::new(2.0, color));
        }
        let text = confirmed.map_or_else(|| "Stage progress unavailable".into(), |f| format!("{:.1}% of this pass · {done} / {total} timeline frames", f * 100.0));
        ui.label(text);
        ui.small("Waveform-shaped progress display — not an audio preview. No overall ETA is estimated.");
        ui.add_space(20.0);
        ui.small("Cancels between render blocks. Final encoder flush, engine setup and atomic file writes cannot be interrupted mid-call.");
        None
    })
}

pub(crate) fn result(ctx: &egui::Context, cx: &UiCtx, lines: &[String], success: bool, open: bool, share: bool) -> Option<Nav> {
    screen(ctx, cx, false, |ui| {
        ui.heading(if success { "EXPORT COMPLETE" } else { "EXPORT FAILED" });
        ui.add_space(16.0);
        for line in lines { ui.label(line); }
        ui.add_space(20.0);
        let mut action = None;
        ui.horizontal_wrapped(|ui| {
            let button = |ui: &mut egui::Ui, text| ui.add(egui::Button::new(text).min_size(vec2(100.0, 40.0))).clicked();
            if button(ui, "OK") { action = Some(Nav::Answer(RENDERED_TAG, false)); }
            if open && button(ui, "OPEN MY TRACKS") { action = Some(Nav::Answer(RENDERED_TAG, true)); }
            if share && button(ui, "SHARE / SEND") { action = Some(Nav::Answer("export_share", true)); }
            if button(ui, if success { "EXPORT AGAIN" } else { "RETRY / SETTINGS" }) { action = Some(Nav::Answer("export_again", true)); }
        });
        action
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn export_progress_is_confirmed_clamped_and_unknown_when_no_total() {
        assert_eq!(fraction(0, 100), Some(0.0));
        assert_eq!(fraction(25, 100), Some(0.25));
        assert_eq!(fraction(200, 100), Some(1.0));
        assert_eq!(fraction(0, 0), None);
    }
}
