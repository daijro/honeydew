//! The insert menu, `New Clip` and `Paste` in the playlist and a lone `Paste` in the clip editors, and its insert
//! cursor: where a new clip or a paste lands, at a snapped beat over the row it was asked for on.
//!
//! - Each button is 1.75 U × 1.5 U, `menuFill` with its label in `text` (8, 7 or 6 pt, the largest that fits); one
//!   that would do nothing is dimmed (black α 0.3 over it) and ignores its tap.
//! - The menu's left edge is 0.875 U left of the snapped beat, clamped inside the grid; its bottom 0.2 U above the row,
//!   or its top 0.2 U below it when there is no room above.
//! - The cursor is a t-wide `text` line between `background` α 0.3 rims a device pixel wide, at the snapped beat across
//!   the span the insert takes (a track row, the keys or pads of a paste). It shows with a flash, a `text` band
//!   0.6 U·f wider than the line at α 0.5·f, f = e^(−18·s) s seconds after the menu showed (about 0.74 a 60 Hz frame,
//!   gone in a quarter second).

use eframe::egui::{Align2, Color32, Context, Id, Painter, Rangef, Rect, Sense, Ui, pos2, vec2};

use crate::metrics::Metrics;
use crate::theme::Rgba;
use crate::ui::UiCtx;

/// A button's size, in U.
const BUTTON: [f32; 2] = [1.75, 1.5];
/// The gap between the menu and its row, in U.
const GAP: f32 = 0.2;
/// A dead button: black at this alpha over it.
const DIM: f32 = 0.3;
/// The cursor's rims: `background` at this alpha.
const RIM_A: f32 = 0.3;
/// The flash: extra width in U, alpha, and its decay rate k (f = e^(−k·s)).
const FLASH: [f32; 3] = [0.6, 0.5, 18.0];

/// What a press did to a showing menu.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum Tap {
    /// The live button of this index was clicked.
    Button(usize),
    /// A press anywhere off the menu: it hides.
    Away,
}

/// Where a menu of `n` buttons goes for the snapped beat at `x` over the row `row` (a y range): its left edge 0.875 U
/// left of `x`, clamped inside `grid`; its bottom 0.2 U above the row, or its top 0.2 U below it when there is no room
/// above (kept inside the grid).
pub(crate) fn place(grid: Rect, x: f32, row: Rangef, n: usize, m: &Metrics) -> Rect {
    let size = vec2(m.u(BUTTON[0]) * n as f32, m.u(BUTTON[1]));
    let left = (x - m.u(BUTTON[0]) / 2.0)
        .min(grid.right() - size.x)
        .max(grid.left());
    let above = row.min - m.u(GAP) - size.y;
    let top = if above >= grid.top() {
        above
    } else {
        (row.max + m.u(GAP))
            .min(grid.bottom() - size.y)
            .max(grid.top())
    };
    Rect::from_min_size(pos2(left, top), size)
}

/// The menu `id` of `items` (label, live) for the snapped beat at `x` over the row `row` ([`place`]) in `grid`, with
/// the insert cursor at `x` across `span`. Returns a live button's click, or [`Tap::Away`] for a press off the menu.
pub(crate) fn show(
    ui: &mut Ui,
    cx: &UiCtx,
    id: Id,
    grid: Rect,
    (x, row, span): (f32, Rangef, Rangef),
    items: &[(&str, bool)],
) -> Option<Tap> {
    let (m, c) = (cx.m, &cx.theme.colors);
    let rect = place(grid, x, row, items.len(), m);
    let age = age(ui.ctx(), id, rect);
    cursor(&ui.painter().with_clip_rect(grid), cx, x, span, age);
    let w = m.u(BUTTON[0]);
    let labels: Vec<&str> = items.iter().map(|i| i.0).collect();
    let font = crate::ui::clips::label_font(ui.ctx(), m, w, &labels);
    let mut tap = None;
    for (i, &(label, live)) in items.iter().enumerate() {
        let b = Rect::from_min_size(rect.min + vec2(w * i as f32, 0.0), vec2(w, rect.height()));
        // Drags too, so the grid under it takes no press of its own.
        let response = ui.interact(b, id.with(i), Sense::click_and_drag());
        let painter = ui.painter();
        painter.rect_filled(b, 0.0, c.menu_fill.to_color32());
        let text = c.text.to_color32();
        painter.text(b.center(), Align2::CENTER_CENTER, label, font.clone(), text);
        if !live {
            let dim = Color32::from_black_alpha((DIM * 255.0).round() as u8);
            painter.rect_filled(b, 0.0, dim);
        }
        if live && response.clicked() {
            tap = Some(Tap::Button(i));
        }
    }
    let away = ui.input(|i| {
        i.pointer.any_pressed() && (i.pointer.interact_pos()).is_some_and(|p| !rect.contains(p))
    });
    tap.or(away.then_some(Tap::Away))
}

/// The clip editors' lone `Paste` ([`show`]): `Some(true)` for its click, `Some(false)` for a press elsewhere.
pub(crate) fn paste(
    ui: &mut Ui,
    cx: &UiCtx,
    id: Id,
    grid: Rect,
    at: (f32, Rangef, Rangef),
) -> Option<bool> {
    show(ui, cx, id, grid, at, &[("Paste", true)]).map(|t| t == Tap::Button(0))
}

/// Button `i` of menu `id` as the last frame drew it.
#[cfg(test)]
pub(crate) fn button(ctx: &Context, id: Id, i: usize) -> Option<Rect> {
    ctx.read_response(id.with(i)).map(|r| r.rect)
}

/// Seconds since menu `id` showed at `at`: the clock restarts when it was not drawn the frame before, or moved.
fn age(ctx: &Context, id: Id, at: Rect) -> f64 {
    let (now, frame) = (ctx.input(|i| i.time), ctx.cumulative_frame_nr());
    let key = id.with("shown");
    let since = match ctx.data(|d| d.get_temp::<(f64, u64, Rect)>(key)) {
        Some((t, f, r)) if frame <= f + 1 && r == at => t,
        _ => now,
    };
    ctx.data_mut(|d| d.insert_temp(key, (since, frame, at)));
    now - since
}

/// The insert cursor at `x` across `span`, `age` seconds after it showed (module docs).
fn cursor(painter: &Painter, cx: &UiCtx, x: f32, span: Rangef, age: f64) {
    let (m, c) = (cx.m, &cx.theme.colors);
    let (px, t) = (1.0 / m.ppp(), m.t());
    let x = m.floor_px(x);
    let band = |w: f32| Rect::from_x_y_ranges(x - w / 2.0..=x + w / 2.0, span);
    let f = (-FLASH[2] * age as f32).exp();
    if f > 0.01 {
        let flash = Rgba {
            a: FLASH[1] * f,
            ..c.text
        };
        painter.rect_filled(band(t + m.u(FLASH[0]) * f), 0.0, flash.to_color32());
        painter.ctx().request_repaint();
    }
    let rim = Rgba {
        a: RIM_A,
        ..c.background
    };
    painter.rect_filled(band(t + 2.0 * px), 0.0, rim.to_color32());
    painter.rect_filled(band(t), 0.0, c.text.to_color32());
}
