//! The step grid's ruler beyond the editors' shared one ([`crate::ui::timeline::ruler`]): the scroll indicator, and the
//! playhead folded into the loop. The step grid's ruler maps song ticks: content beat 0 sits at song beat s, so its bar
//! numbers are song bars.

use eframe::egui::{Color32, Painter, Rect, pos2, vec2};

use crate::song::PPQ;

/// Ticks per beat as `f64`.
pub const BEAT: f64 = PPQ as f64;

/// The scroll indicator on the ruler `rect`, `t` the hairline, for (posX, the beats in view, the loop L) in beats: only
/// while the view spans less than the loop, a bar t tall in the channel colour `ink` on the ruler's top, at
/// X + posX·W/L, W·(visible/L) wide. It maps [0, L], so it slides off the right past the loop.
pub fn indicator(
    painter: &Painter,
    rect: Rect,
    (pos, visible, loop_beats): (f64, f64, f64),
    ink: Color32,
    t: f32,
) {
    if visible < loop_beats {
        let w = f64::from(rect.width()) / loop_beats;
        let x = rect.left() + (pos * w) as f32;
        let bar = Rect::from_min_size(pos2(x, rect.top()), vec2((visible * w) as f32, t));
        painter.rect_filled(bar, 0.0, ink);
    }
}

/// The song tick `pos` folded into the clip whose content tick 0 plays at song tick `origin`, as a content tick in
/// [0, `loop_ticks`) (s + ((pos − s) mod L), negative values wrapped); 0 without a loop.
pub fn folded(pos: u64, origin: i64, loop_ticks: f64) -> f64 {
    if loop_ticks > 0.0 {
        (pos as f64 - origin as f64).rem_euclid(loop_ticks)
    } else {
        0.0
    }
}
