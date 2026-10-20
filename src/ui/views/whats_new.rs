//! The What's New dialog: what changed in this version of the app, in its own words. Help → `What's new` opens it, and
//! on phones and tablets it opens once by itself at the first start of a new notes version ([`due`]), but not on a
//! fresh install.
//!
//! A floating dialog, centred: `menuFill`, corners min(0.4 U, half the short side), in a 2 px border of
//! 0.55·`menuFill` + 0.45·`text`, over the screen dimmed to α 0.6. It is min(12 U, W − 2.5 U) wide, padded
//! round(max(0.8 U, a 10 pt line)), and as tall as its content and padding, at most the window less 1.25 U at the top and
//! bottom (0.5 U in portrait); past that the content scrolls (wheel or drag) with a `text` scrollbar. The content, all
//! FontSize 2 in `text`: `What's new` left and the notes version right (α 0.6), 0.55 U, a divider, 0.55 U, then each
//! section's title, 0.2 U and its wrapped body (α 0.8), 0.55 U apart, then 0.85 U and `Get started`, the content's
//! width and 1.35 U tall, an `accent` fill with a `textOnAccent` label and corners 0.25 U. `Get started`, a tap outside
//! and Escape close it.

use std::sync::Arc;

use eframe::egui::{
    self, Align2, Area, Color32, Galley, Id, LayerId, Order, Rect, Sense, pos2, vec2,
};

use super::{AppCtx, Nav};
use crate::metrics::FontSize;
use crate::ui::anim;
use crate::ui::widgets::pressed;

/// The notes' version: the dialog opens by itself once while the stored `whatsNewSeen` is older.
pub const NOTES_VERSION: &str = "0.3";

/// The sections, title and body.
const SECTIONS: [(&str, &str); 7] = [
    (
        "Refreshed look",
        "A new transport bar with Undo and Redo always at hand and a Solo clip button, a play box on every ruler, \
         rounded clips and new icons. Editors close with a ribbon, the logo and the rack tab float over the playlist, \
         the rack has rounded cards, the main menu has a new header, and dialogs float over the screen.",
    ),
    (
        "Lemon Studio, built in",
        "Lemon Trio, Lemon Sampler and Lemon Parametric EQ come built in on every platform, with Lemon's own \
         controls right in their rack cards. Lemon Reverb and Lemon Limiter are new effects under ECHO and LEVEL.",
    ),
    (
        "Any window, any orientation",
        "The layout follows the window it has. On a narrow or portrait screen the transport bar takes two rows, and \
         the rack, the browsers and the main menu take the full width.",
    ),
    (
        "Scale and animation",
        "SETTINGS has a Scaling slider from 60 to 200 % and an Animation setting: Full, Reduced or Minimal.",
    ),
    (
        "Smart solo",
        "Soloing a channel in the mixer also solos the effect channels it sends to. Unsolo All, on the master, \
         clears every solo at once.",
    ),
    (
        "New tools",
        "Burn Notes to Track bakes a channel's note effects into a new track. Convert to FLAC shrinks a WAV file in \
         the browser. Drag audio files from your computer or the sample browser onto the playlist to make clips.",
    ),
    (
        "Desktop",
        "Tooltips name the buttons and their shortcuts, and Ctrl+[ and Ctrl+] jump a bar back and forth. An \
         installed plugin's pop-out button opens its own window.",
    ),
];

// Sizes in U.
const WIDTH: f32 = 12.0;
const SIDE_ROOM: f32 = 2.5;
const PAD: f32 = 0.8;
const RADIUS: f32 = 0.4;
const MARGIN_LANDSCAPE: f32 = 1.25;
const MARGIN_PORTRAIT: f32 = 0.5;
const GAP: f32 = 0.55;
const TITLE_GAP: f32 = 0.2;
const BUTTON_GAP: f32 = 0.85;
const BUTTON_H: f32 = 1.35;
const BUTTON_RADIUS: f32 = 0.25;
const SCROLLBAR_INSET: f32 = 0.25;
/// The border, in device pixels, and its mix of `text` into `menuFill`.
const BORDER_PX: f32 = 2.0;
const BORDER_TEXT: f32 = 0.45;
const VERSION_ALPHA: f32 = 0.6;
const BODY_ALPHA: f32 = 0.8;
const DIVIDER_ALPHA: f32 = 0.18;

/// What the dialog keeps while it shows: how far its content is scrolled, in points.
#[derive(Default)]
pub struct State {
    scroll: f32,
}

/// Whether the notes are due by themselves: the stored `whatsNewSeen` is empty or an older version than
/// [`NOTES_VERSION`] (compared number by number).
pub fn due(seen: &str) -> bool {
    let parts = |v: &str| -> Vec<u32> {
        v.split('.')
            .map(|p| p.trim().parse().unwrap_or(0))
            .collect()
    };
    seen.trim().is_empty() || parts(seen) < parts(NOTES_VERSION)
}

/// `a` mixed into `b` by `t`.
fn mix(a: Color32, b: Color32, t: f32) -> Color32 {
    let ch = |x: u8, y: u8| (f32::from(x) * (1.0 - t) + f32::from(y) * t).round() as u8;
    Color32::from_rgb(ch(a.r(), b.r()), ch(a.g(), b.g()), ch(a.b(), b.b()))
}

/// Shows the dialog over everything; returns `Nav::Close` once it closes.
pub fn show(state: &mut State, ui: &mut egui::Ui, app: &mut AppCtx) -> Option<Nav> {
    let ctx = ui.ctx().clone();
    let (cx, screen) = (&app.cx, app.regions.screen);
    let (m, c) = (cx.m, &cx.theme.colors);
    let id = Id::new("whats_new");
    let outside = crate::ui::backdrop(&ctx, id).clicked();
    let dim = ctx.layer_painter(LayerId::new(Order::Middle, id.with("backdrop")));
    dim.rect_filled(
        screen,
        0.0,
        anim::dim(c.background.to_color32(), anim::DIM_ALPHA),
    );

    let font = m.font_id(FontSize::Pt10);
    let text = c.text.to_color32();
    let line_h = ctx.fonts_mut(|f| f.row_height(&font));
    let pad = m.round_px(m.u(PAD).max(line_h));
    let w = m
        .u(WIDTH)
        .min(screen.width() - m.u(SIDE_ROOM))
        .max(2.0 * pad);
    let inner_w = w - 2.0 * pad;
    let wrapped = |s: &str, color: Color32| -> Arc<Galley> {
        ctx.fonts_mut(|f| f.layout(s.to_owned(), font.clone(), color, inner_w))
    };
    let sections: Vec<(Arc<Galley>, Arc<Galley>)> = (SECTIONS.iter())
        .map(|(title, body)| {
            (
                wrapped(title, text),
                wrapped(body, text.gamma_multiply(BODY_ALPHA)),
            )
        })
        .collect();
    let sections_h: f32 = (sections.iter())
        .map(|(t, b)| t.size().y + m.u(TITLE_GAP) + b.size().y)
        .sum::<f32>()
        + m.u(GAP) * (sections.len().saturating_sub(1)) as f32;
    let content_h =
        line_h + 2.0 * m.u(GAP) + 1.0 / m.ppp() + sections_h + m.u(BUTTON_GAP) + m.u(BUTTON_H);
    let margin = m.u(if crate::metrics::is_portrait(screen) {
        MARGIN_PORTRAIT
    } else {
        MARGIN_LANDSCAPE
    });
    let h = (content_h + 2.0 * pad)
        .min(screen.height() - 2.0 * margin)
        .max(2.0 * pad);
    let rect = Rect::from_center_size(screen.center(), vec2(w, h));
    let view = rect.shrink(pad);
    let max_scroll = (content_h - view.height()).max(0.0);

    let mut close = outside || crate::keymap::pressed(&ctx, "close");
    Area::new(id)
        .order(Order::Foreground)
        .fixed_pos(rect.min)
        .constrain(false)
        .fade_in(false)
        .show(&ctx, |ui| {
            ui.set_min_size(rect.size());
            // The layer's own painter: the area's first frame (its sizing pass) paints nothing through its `Ui`.
            let painter = ctx.layer_painter(ui.layer_id());
            let radius = m.u(RADIUS).min(rect.width().min(rect.height()) / 2.0);
            let fill = c.menu_fill.to_color32();
            let border = BORDER_PX / m.ppp();
            painter.rect_filled(
                rect.expand(border),
                radius + border,
                mix(fill, text, BORDER_TEXT),
            );
            painter.rect_filled(rect, radius, fill);

            // Scrolling: the wheel over the dialog, or a drag on it.
            let drag = ui.interact(rect, id.with("scroll"), Sense::drag());
            let mut moved = drag.drag_delta().y;
            if ui.rect_contains_pointer(rect) {
                moved += ui.input(|i| i.smooth_scroll_delta.y);
            }
            state.scroll = (state.scroll - moved).clamp(0.0, max_scroll);

            let clipped = painter.with_clip_rect(view);
            let mut y = view.top() - state.scroll;
            let (left, right) = (view.left(), view.right());
            clipped.text(
                pos2(left, y),
                Align2::LEFT_TOP,
                "What's new",
                font.clone(),
                text,
            );
            let version = format!("v{NOTES_VERSION}");
            let faint = text.gamma_multiply(VERSION_ALPHA);
            clipped.text(
                pos2(right, y),
                Align2::RIGHT_TOP,
                version,
                font.clone(),
                faint,
            );
            y += line_h + m.u(GAP);
            let divider = Rect::from_x_y_ranges(left..=right, y..=y + 1.0 / m.ppp());
            clipped.rect_filled(divider, 0.0, text.gamma_multiply(DIVIDER_ALPHA));
            y += 1.0 / m.ppp() + m.u(GAP);
            for (i, (title, body)) in sections.into_iter().enumerate() {
                if i > 0 {
                    y += m.u(GAP);
                }
                let title_h = title.size().y;
                clipped.galley(pos2(left, y), title, text);
                y += title_h + m.u(TITLE_GAP);
                let body_h = body.size().y;
                clipped.galley(pos2(left, y), body, text);
                y += body_h;
            }
            y += m.u(BUTTON_GAP);
            let button = Rect::from_x_y_ranges(left..=right, y..=y + m.u(BUTTON_H));
            let shown = button.intersect(view);
            let response = if shown.height() > 0.0 {
                ui.interact(shown, id.with("get_started"), Sense::click())
            } else {
                ui.interact(Rect::NOTHING, id.with("get_started"), Sense::hover())
            };
            let accent = c.accent.to_color32();
            let accent = if response.is_pointer_button_down_on() || response.hovered() {
                pressed(accent)
            } else {
                accent
            };
            clipped.rect_filled(button, m.u(BUTTON_RADIUS), accent);
            let label = c.text_on_accent.to_color32();
            clipped.text(
                button.center(),
                Align2::CENTER_CENTER,
                "Get started",
                font.clone(),
                label,
            );
            close |= response.clicked();

            if max_scroll > 0.0 {
                let t = m.t();
                let x = rect.right() - m.u(SCROLLBAR_INSET) - t;
                let share = view.height() / content_h;
                let thumb_h = (view.height() * share).max(m.u(0.5));
                let top = view.top() + (view.height() - thumb_h) * state.scroll / max_scroll;
                let thumb = Rect::from_min_size(pos2(x, top), vec2(t, thumb_h));
                painter.rect_filled(thumb, 0.0, text);
            }
        });
    if close {
        state.scroll = 0.0;
        return Some(Nav::Close);
    }
    None
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_notes_are_due_for_an_empty_or_older_seen_version_only() {
        assert!(due(""));
        assert!(due("0.2"));
        assert!(due("0.2.9"));
        assert!(!due(NOTES_VERSION));
        assert!(!due("0.3.1"));
        assert!(!due("1.0"));
    }
}
