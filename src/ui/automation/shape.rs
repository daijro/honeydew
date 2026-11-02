//! `Set shape` (Honeydew's own): the floating row's button for the selected points opens a small popup over the
//! selection, the four shapes as buttons (each with its curve drawn above its name; the selection's own shape
//! highlighted while all its points agree) and the **Tension** slider under them (a double tap resets it to 50 %). Each
//! pick shows at once on the editor's curve; a tap outside the popup or Escape closes it, and what it set becomes one
//! edit (one undo step) on the segments that start at the selected points. Until a shape (or a tension) is picked, each
//! point keeps its own.
//!
//! - The box: [`dialog::panel`] (no dim, so the curve stays in sight), 13.75 U × 5.75 U, inset 0.5 U: four buttons
//!   3 U × 2 U, 0.25 U apart (`text` with a `moduleFill` ink when current, else `controlFill` with a `text` ink), then
//!   0.25 U down a 1 U row with `Tension <n>%` (FontSize 12 pt, `text`) and the 1.5 U slider row
//!   ([`widgets::fine_tune_slider`] in the channel colour).
//! - Placed left of the selection's touch frame, 0.5 U from it and centred on it in height, so that the segments it
//!   shapes (which start at the points and run right) stay in sight; where that does not fit, 0.5 U above the frame,
//!   centred on it, else 0.5 U below it, else as high as it fits. Kept 0.5 U inside the screen.

use eframe::egui::{
    Align2, Area, Color32, Context, Id, Order, Pos2, Rect, Sense, Stroke, pos2, vec2,
};

use crate::metrics::FontSize;
use crate::song::points::{Point, Shape, ShapeKind, shape_value};
use crate::ui::widgets::{self, Button, Control, ParamMenu};
use crate::ui::{UiCtx, dialog};

// Sizes in U.
const INSET: f32 = 0.5;
const BUTTON_W: f32 = 3.0;
const BUTTON_H: f32 = 2.0;
const GAP: f32 = 0.25;
const LABEL_H: f32 = 1.0;
const SLIDER_H: f32 = 1.5;
/// Between the popup and the selection's touch frame, and the screen's edge.
const MARGIN: f32 = 0.5;
/// A button's curve: inset this much of the button's height from its upper part's edges, 1.5 px wide.
const ICON_INSET: f32 = 0.2;
const ICON_PX: f32 = 1.5;

/// The popup while open: what it sets on the selected points so far, each part `None` until picked.
#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub(super) struct Shaping {
    kind: Option<ShapeKind>,
    tension: Option<u8>,
}

impl Shaping {
    /// `pts` with the picks on the selected ones.
    pub(super) fn on(self, pts: &[Point], sel: &[bool]) -> Vec<Point> {
        (pts.iter().zip(sel))
            .map(|(p, &s)| match s {
                true => Point {
                    shape: Shape {
                        kind: self.kind.unwrap_or(p.shape.kind),
                        tension: self.tension.unwrap_or(p.shape.tension),
                    },
                    ..*p
                },
                false => *p,
            })
            .collect()
    }
}

/// The segment from `a` to `z` on screen with shape `s` ([`shape_value`] on y, which is affine in the value): a
/// stepped shape with upright risers, a curve in pieces about 2 points wide.
pub(super) fn curve(s: Shape, a: Pos2, z: Pos2) -> Vec<Pos2> {
    let y = |x: f64| shape_value(s, a.y.into(), z.y.into(), x) as f32;
    let at = |x: f64, y: f32| pos2(a.x + (z.x - a.x) * x as f32, y);
    match s.risers() {
        Some(risers) => {
            let mut path = vec![a];
            for x in risers.into_iter().chain([1.0]) {
                let before = path[path.len() - 1].y;
                // Just after the riser (Square at 0 % rises at x = 0).
                path.extend([at(x, before), at(x, y(x.max(f64::MIN_POSITIVE)))]);
            }
            path
        }
        None => {
            let n = ((z.x - a.x) / 2.0).ceil().clamp(1.0, 512.0) as usize;
            (0..=n)
                .map(|i| {
                    let x = i as f64 / n as f64;
                    at(x, y(x))
                })
                .collect()
        }
    }
}

/// The popup's box for the selection's touch frame `frame` on `screen` ([module docs](self)).
fn place(frame: Rect, screen: Rect, u: f32) -> Rect {
    let size = vec2(
        2.0 * INSET + 4.0 * BUTTON_W + 3.0 * GAP,
        2.0 * INSET + BUTTON_H + GAP + LABEL_H + SLIDER_H,
    ) * u;
    let (gap, inside) = (MARGIN * u, screen.shrink(MARGIN * u));
    let beside = frame.left() - gap - size.x;
    if beside >= inside.left() {
        let top = (frame.center().y - size.y / 2.0)
            .clamp(inside.top(), (inside.bottom() - size.y).max(inside.top()));
        return Rect::from_min_size(pos2(beside, top), size);
    }
    let above = frame.top() - gap - size.y;
    let top = if above >= inside.top() {
        above
    } else if frame.bottom() + gap + size.y <= inside.bottom() {
        frame.bottom() + gap
    } else {
        inside.top()
    };
    let left = (frame.center().x - size.x / 2.0)
        .clamp(inside.left(), (inside.right() - size.x).max(inside.left()));
    Rect::from_min_size(pos2(left, top), size)
}

/// What one frame of the popup did.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub(super) struct Shown {
    /// A tap outside or Escape: it closes.
    pub close: bool,
}

/// One frame of the popup `id` over the selection's touch frame `frame`, editing `shaping`; `own` is the selected
/// points' shared shape, if they share one; `ink` the channel colour.
pub(super) fn popup(
    ctx: &Context,
    cx: &UiCtx,
    id: Id,
    frame: Rect,
    (shaping, own): (&mut Shaping, Option<Shape>),
    ink: Color32,
) -> Shown {
    let (m, c) = (cx.m, &cx.theme.colors);
    let u = m.u(1.0);
    let rect = place(frame, ctx.content_rect(), u);
    let rect = Rect::from_min_size(
        pos2(m.round_px(rect.left()), m.round_px(rect.top())),
        rect.size(),
    );
    // Below the box, over the screens: a tap outside closes it. No dim: the curve shows the picks.
    let backdrop = crate::ui::backdrop(ctx, id);
    let close = crate::ui::close_pressed(ctx) || backdrop.clicked() || backdrop.secondary_clicked();
    let kind = shaping.kind.or(own.map(|s| s.kind));
    let tension = shaping.tension.or(own.map(|s| s.tension)).unwrap_or(50);
    Area::new(id)
        .order(Order::Foreground)
        .fixed_pos(rect.min)
        .constrain(false)
        .fade_in(false)
        .show(ctx, |ui| {
            ui.allocate_exact_size(rect.size(), Sense::hover());
            dialog::panel(ui.painter(), c, m, rect);
            let (fill, text) = (c.text.to_color32(), c.control_fill.to_color32());
            let top = rect.top() + INSET * u;
            for (i, k) in ShapeKind::ALL.into_iter().enumerate() {
                let left = rect.left() + (INSET + i as f32 * (BUTTON_W + GAP)) * u;
                let cell = Rect::from_min_size(pos2(left, top), vec2(BUTTON_W, BUTTON_H) * u);
                let on = kind == Some(k);
                let b = Button {
                    fill_off: text,
                    text_off: Some(fill),
                    ..dialog::button(id.with(("kind", i)), cell, "", c, m)
                };
                if widgets::button(ui, cx, &b, on).clicked() {
                    shaping.kind = Some(k);
                }
                // Its curve in the upper two thirds (at the popup's tension), its name in the lower third.
                let label = if on { b.text_on } else { fill };
                let third = cell.height() / 3.0;
                let icon = cell
                    .with_max_y(cell.bottom() - third)
                    .shrink(ICON_INSET * cell.height());
                let s = Shape { kind: k, tension };
                let path = curve(s, icon.left_bottom(), icon.right_top());
                ui.painter().add(eframe::egui::Shape::line(
                    path,
                    Stroke::new(ICON_PX / m.ppp(), label),
                ));
                let name = cell.with_min_y(cell.bottom() - third);
                ui.painter().text(
                    name.center(),
                    Align2::CENTER_CENTER,
                    label_of(k),
                    m.font_id(FontSize::Pt10),
                    label,
                );
            }
            let row = |y: f32, h: f32| {
                Rect::from_x_y_ranges(
                    rect.left() + INSET * u..=rect.right() - INSET * u,
                    y..=y + h * u,
                )
            };
            let label = row(top + (BUTTON_H + GAP) * u, LABEL_H);
            let text = format!("Tension {tension}%");
            ui.painter().text(
                label.center(),
                Align2::CENTER_CENTER,
                text,
                m.font_id(FontSize::Pt12),
                fill,
            );
            let slider = row(label.bottom(), SLIDER_H);
            let control = Control {
                menu: ParamMenu::Off,
                ..Control::new(id.with("tension"), slider, "", ink, 0.5)
            };
            let mut v = f32::from(tension) / 100.0;
            if widgets::fine_tune_slider(ui, cx, &control, &mut v).changed() {
                shaping.tension = Some((v * 100.0).round() as u8);
            }
        });
    Shown { close }
}

/// A shape's button label.
fn label_of(kind: ShapeKind) -> &'static str {
    match kind {
        ShapeKind::Linear => "Linear",
        ShapeKind::Square => "Square",
        ShapeKind::Smooth => "Smooth",
        ShapeKind::Stairs => "Stairs",
    }
}
