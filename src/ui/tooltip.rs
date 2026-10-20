//! Desktop tooltips: a control names its tooltip as it draws ([`tip`]); after the frame [`show`] draws the one under
//! the pointer, with the shortcut of its keymap action.
//!
//! - **Which.** A control's tooltip is its own or, if it has none, its parent's: of the tips whose rect holds the
//!   pointer, the smallest. Only a control the pointer is over in its own layer counts ([`Ui::rect_contains_pointer`]),
//!   so a popup or a panel over it hides it; and a modal shown after it in the frame ([`crate::ui::modal_shown`],
//!   [`cover`]) covers it, in whatever layer.
//! - **Timing.** The pointer rests on the same control within 4 px for 0.6 s; when a tooltip hid less than 1 s ago the
//!   next shows at once. Moving more than 4 px, or onto another control, hides it and starts the wait again. A press,
//!   a click or a key hides it until the pointer moves again. Mouse pointers only: a touch leaves no hover.
//! - **Look.** The label (FontSize 2), a 0.8 U gap and the shortcut (FontSize 2, `text` α 0.5) on the right, padded
//!   0.4 U × 0.2 U, at most half the window − 0.8 U wide (the label end-ellipsised), on a `menuFill` rounded rect
//!   (radius 0.25 U) with a 1 px `text` α 0.15 outline; it fades in at 10 per second. It is centred under the control,
//!   0.25 U below it, or 0.25 U above where it would leave the window's bottom, and kept inside the window.
//! - **Shortcut.** The action's first key binding in the keymap ([`crate::keymap::Keymap::shortcut`]), `CTRL+K`.

use eframe::egui::{
    Align2, Color32, Context, Event, Id, LayerId, Order, Pos2, Rect, Stroke, StrokeKind, Ui, pos2,
    vec2,
};

use crate::keymap;
use crate::metrics::FontSize;
use crate::ui::UiCtx;
use crate::ui::widgets::truncated_text;

// Times in seconds, sizes in U (the move tolerance in device pixels).
const DWELL: f64 = 0.6;
const RECENT: f64 = 1.0;
const MOVE_PX: f32 = 4.0;
const FADE_PER_S: f32 = 10.0;
const GAP: f32 = 0.8;
const PAD_X: f32 = 0.4;
const PAD_Y: f32 = 0.2;
const RADIUS: f32 = 0.25;
const OFFSET: f32 = 0.25;
const SHORTCUT_ALPHA: f32 = 0.5;
const OUTLINE_ALPHA: f32 = 0.15;

/// A control's tooltip, as it named it this frame.
#[derive(Clone, Debug, PartialEq)]
struct Tip {
    rect: Rect,
    label: &'static str,
    /// The keymap action whose shortcut it shows.
    action: Option<&'static str>,
}

/// The tips named this frame (only those under the pointer), by frame number, and how many of them a modal covers.
#[derive(Clone, Default)]
struct Named(u64, Vec<Tip>, usize);

/// What carries over from frame to frame.
#[derive(Clone, Default)]
struct State {
    /// The tip the pointer rests on, where it came to rest, and since when.
    resting: Option<(Tip, Pos2, f64)>,
    /// The tip is up, and how far it has faded in.
    shown: bool,
    alpha: f32,
    /// When a tip that was up last hid.
    hidden_at: Option<f64>,
    /// A press or a key hid it: nothing shows until the pointer leaves this point.
    suppressed: Option<Pos2>,
    /// The last frame's time.
    time: f64,
}

fn named_id() -> Id {
    Id::new("tooltip").with("named")
}

fn state_id() -> Id {
    Id::new("tooltip").with("state")
}

/// Names `label` (and the shortcut of keymap `action`) as the tooltip of the control in `rect`. Call it as the control
/// draws, every frame; it costs nothing unless the pointer is over the control.
pub fn tip(ui: &Ui, rect: Rect, label: &'static str, action: Option<&'static str>) {
    if !ui.rect_contains_pointer(rect) {
        return;
    }
    named(ui.ctx(), |named| {
        named.1.push(Tip {
            rect,
            label,
            action,
        });
    });
}

/// A modal is shown from here on in the frame: it covers every tip named so far.
pub(crate) fn cover(ctx: &Context) {
    named(ctx, |named| named.2 = named.1.len());
}

/// This frame's [`Named`], to change.
fn named(ctx: &Context, f: impl FnOnce(&mut Named)) {
    let frame = ctx.cumulative_frame_nr();
    ctx.data_mut(|d| {
        let named = d.get_temp_mut_or_default::<Named>(named_id());
        if named.0 != frame {
            *named = Named(frame, Vec::new(), 0);
        }
        f(named);
    });
}

/// Of the tips named this frame, the one whose rect holds `at`: the smallest (a control's own before its parent's).
fn under(tips: &[Tip], at: Pos2) -> Option<&Tip> {
    (tips.iter())
        .filter(|t| t.rect.contains(at))
        .min_by(|a, b| a.rect.area().total_cmp(&b.rect.area()))
}

/// Draws the tooltip of the control under the pointer, if it is due. Call it once a frame, after every control drew.
pub fn show(ctx: &Context, cx: &UiCtx) {
    if cfg!(target_os = "android") {
        return;
    }
    let frame = ctx.cumulative_frame_nr();
    let tips = ctx.data_mut(|d| d.get_temp::<Named>(named_id()));
    let tips = (tips.filter(|n| n.0 == frame))
        .map(|n| n.1[n.2..].to_vec())
        .unwrap_or_default();
    let mut s = ctx.data_mut(|d| d.get_temp::<State>(state_id()).unwrap_or_default());
    let (now, pointer, pressed, held) = ctx.input(|i| {
        let key = i
            .events
            .iter()
            .any(|e| matches!(e, Event::Key { pressed: true, .. }));
        let pressed = key || i.pointer.any_pressed() || i.any_touches();
        (i.time, i.pointer.hover_pos(), pressed, i.pointer.any_down())
    });
    let dt = (now - s.time).clamp(0.0, 0.1) as f32;
    s.time = now;
    let tolerance = MOVE_PX / ctx.pixels_per_point();
    let hide = |s: &mut State| {
        if s.shown {
            s.hidden_at = Some(now);
        }
        (s.shown, s.alpha, s.resting) = (false, 0.0, None);
    };
    match pointer {
        Some(at) if pressed => {
            s.suppressed = Some(at);
            hide(&mut s);
        }
        // No pointer, or a drag: none.
        None => hide(&mut s),
        Some(_) if held => hide(&mut s),
        Some(at) => {
            if s.suppressed.is_some_and(|p| p.distance(at) > tolerance) {
                s.suppressed = None;
            }
            match under(&tips, at).filter(|_| s.suppressed.is_none()) {
                None => hide(&mut s),
                Some(t) => {
                    let same = s.resting.as_ref().is_some_and(|(r, p, _)| {
                        (r.label, r.action) == (t.label, t.action) && p.distance(at) <= tolerance
                    });
                    if !same {
                        hide(&mut s);
                        s.resting = Some((t.clone(), at, now));
                    } else if let Some(r) = &mut s.resting {
                        // The control may move under a resting pointer (an animation): follow it.
                        r.0.rect = t.rect;
                    }
                }
            }
        }
    }
    if let Some((_, _, since)) = s.resting {
        let recent = s.hidden_at.is_some_and(|h| since - h < RECENT);
        s.shown |= recent || now - since >= DWELL;
        if !s.shown {
            ctx.request_repaint_after_secs((DWELL - (now - since)) as f32);
        }
    }
    if s.shown {
        s.alpha = (s.alpha + FADE_PER_S * dt).min(1.0);
        if s.alpha < 1.0 {
            ctx.request_repaint();
        }
        if let Some((t, _, _)) = &s.resting {
            draw(ctx, cx, t, s.alpha);
        }
    }
    ctx.data_mut(|d| d.insert_temp(state_id(), s));
}

/// Draws tip `t` at `alpha` on the tooltip layer.
fn draw(ctx: &Context, cx: &UiCtx, t: &Tip, alpha: f32) {
    let (m, c) = (cx.m, &cx.theme.colors);
    let screen = ctx.content_rect();
    let painter = ctx.layer_painter(LayerId::new(Order::Tooltip, Id::new("tooltip")));
    let font = m.font_id(FontSize::Pt10);
    let text = c.text.to_color32().gamma_multiply(alpha);
    let keys = keymap::for_os(ctx.os());
    let shortcut = t.action.and_then(|a| keys.shortcut(a)).unwrap_or_default();
    let layout = |s: &str| painter.layout_no_wrap(s.to_owned(), font.clone(), Color32::PLACEHOLDER);
    let short = layout(shortcut);
    let gap = if shortcut.is_empty() { 0.0 } else { m.u(GAP) };
    let max_w = (screen.width() / 2.0 - m.u(GAP)).max(0.0);
    let pad = vec2(m.u(PAD_X), m.u(PAD_Y));
    let label_room = (max_w - 2.0 * pad.x - gap - short.size().x).max(0.0);
    let label = truncated_text(&painter, t.label, font.clone(), label_room, 0);
    let label_w = layout(&label).size().x;
    let h = short.size().y.max(layout("Ag").size().y);
    let size = vec2(label_w + gap + short.size().x, h) + 2.0 * pad;
    let below = t.rect.bottom() + m.u(OFFSET);
    let top = if below + size.y > screen.bottom() {
        t.rect.top() - m.u(OFFSET) - size.y
    } else {
        below
    };
    let left = t.rect.center().x - size.x / 2.0;
    let left = left.min(screen.right() - size.x).max(screen.left());
    let top = top.min(screen.bottom() - size.y).max(screen.top());
    let rect = Rect::from_min_size(pos2(left, top), size);
    let radius = m.u(RADIUS);
    painter.rect_filled(rect, radius, c.menu_fill.to_color32().gamma_multiply(alpha));
    let outline = c.text.to_color32().gamma_multiply(OUTLINE_ALPHA * alpha);
    let stroke = Stroke::new(1.0 / m.ppp(), outline);
    painter.rect_stroke(rect, radius, stroke, StrokeKind::Inside);
    let inner = rect.shrink2(pad);
    painter.text(
        inner.left_center(),
        Align2::LEFT_CENTER,
        label,
        font.clone(),
        text,
    );
    if !shortcut.is_empty() {
        let ink = text.gamma_multiply(SHORTCUT_ALPHA);
        painter.text(
            inner.right_center(),
            Align2::RIGHT_CENTER,
            shortcut,
            font,
            ink,
        );
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::metrics::{DEFAULT_UI_SCALE, Metrics};
    use crate::theme::theme_by_id;
    use eframe::egui::{RawInput, Sense, Shape};

    /// One frame at `time` with the pointer at `at` (and a click there, `down`): a 40 × 20 control at (100, 100) named
    /// `Tempo`. Whether the tooltip drew.
    fn frame(ctx: &Context, time: f64, at: Pos2, down: bool) -> bool {
        frame_with(ctx, time, at, down, false)
    }

    /// [`frame`], with a modal shown after the control when `modal`.
    fn frame_with(ctx: &Context, time: f64, at: Pos2, down: bool, modal: bool) -> bool {
        let mut events = vec![Event::PointerMoved(at)];
        if down {
            for pressed in [true, false] {
                events.push(Event::PointerButton {
                    pos: at,
                    button: eframe::egui::PointerButton::Primary,
                    pressed,
                    modifiers: Default::default(),
                });
            }
        }
        let input = RawInput {
            time: Some(time),
            events,
            screen_rect: Some(Rect::from_min_size(Pos2::ZERO, vec2(800.0, 600.0))),
            ..Default::default()
        };
        let theme = theme_by_id(0);
        let out = ctx.run(input, |ctx| {
            let m = Metrics::new(ctx, DEFAULT_UI_SCALE);
            let cx = UiCtx {
                theme: &theme,
                m: &m,
            };
            eframe::egui::CentralPanel::default().show(ctx, |ui| {
                let rect = Rect::from_min_size(pos2(100.0, 100.0), vec2(40.0, 20.0));
                ui.interact(rect, Id::new("control"), Sense::click());
                tip(ui, rect, "Tempo", None);
                if modal {
                    crate::ui::modal_shown(ctx);
                }
            });
            show(ctx, &cx);
        });
        let shapes = out.shapes.iter().map(|s| &s.shape);
        let text = |s: &Shape| matches!(s, Shape::Text(t) if t.galley.text() == "Tempo");
        shapes.into_iter().any(text)
    }

    #[test]
    fn a_tooltip_shows_after_a_rest_and_at_once_after_a_recent_one() {
        let ctx = Context::default();
        let on = pos2(110.0, 110.0);
        assert!(!frame(&ctx, 0.0, on, false));
        assert!(!frame(&ctx, 0.5, on, false));
        assert!(frame(&ctx, 0.7, on, false));
        // Moving within 4 px keeps it; further hides it and waits again, but a tip that hid less than 1 s ago shows
        // again at once.
        assert!(frame(&ctx, 0.8, pos2(113.0, 110.0), false));
        assert!(frame(&ctx, 0.9, pos2(130.0, 110.0), false));
        // Off the control it hides; back after the 1 s, it waits again.
        assert!(!frame(&ctx, 1.0, pos2(300.0, 300.0), false));
        assert!(!frame(&ctx, 2.5, on, false));
        assert!(frame(&ctx, 3.2, on, false));
        // A press hides it until the pointer moves.
        assert!(!frame(&ctx, 3.3, on, true));
        assert!(!frame(&ctx, 4.5, on, false));
        assert!(!frame(&ctx, 4.6, pos2(120.0, 110.0), false));
        assert!(frame(&ctx, 5.3, pos2(120.0, 110.0), false));
        // A modal shown after the control covers it.
        assert!(!frame_with(&ctx, 5.4, pos2(120.0, 110.0), false, true));
    }
}
