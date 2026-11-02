//! The tag slider: Tremolo's `Max` and `Min` beside its display.

use eframe::egui::{Align2, Pos2, Rect, Response, Shape, Stroke, Ui, pos2};

use super::{Control, Law, drag_control, primary_down_on};
use crate::metrics::FontSize;
use crate::ui::UiCtx;

/// Height of the tag, over the control height.
const TAG_H: f32 = 0.12;
/// Length of its pointer, over the control height.
const POINTER: f32 = 0.06;
/// Alpha of the tag's channel colour.
const TAG_ALPHA: f32 = 0.2;
/// How far the text's rect sits towards the pointer from the bar, over the control height.
const TEXT_SHIFT: f32 = 0.015;

/// The tag's centre line at value `v`: y = bottom − 0.06 h − 0.88 h·v, so the 0.12 h tall tag touches the bottom
/// at 0 and the top at 1.
fn tag_y(rect: Rect, v: f32) -> f32 {
    rect.bottom() - (TAG_H / 2.0 + (1.0 - TAG_H) * v) * rect.height()
}

/// The tag at value `v`: a bar 0.12 h tall across the rect, ending in a pointer 0.06 h long whose tip touches the
/// rect's right edge (`faces_right`) or left edge. One convex outline.
fn tag(rect: Rect, v: f32, faces_right: bool) -> [Pos2; 5] {
    let (h, y) = (rect.height(), tag_y(rect, v));
    let (top, bottom) = (y - TAG_H / 2.0 * h, y + TAG_H / 2.0 * h);
    let (back, tip, base) = ends(rect, faces_right);
    [
        pos2(back, top),
        pos2(base, top),
        pos2(tip, y),
        pos2(base, bottom),
        pos2(back, bottom),
    ]
}

/// The x of the tag's back (the far edge), of its tip and of the pointer's base, 0.06 h from the tip (at most the
/// whole width).
fn ends(rect: Rect, faces_right: bool) -> (f32, f32, f32) {
    let len = (POINTER * rect.height()).min(rect.width());
    if faces_right {
        (rect.left(), rect.right(), rect.right() - len)
    } else {
        (rect.right(), rect.left(), rect.left() + len)
    }
}

/// The rect the tag's text is centred in: the bar (from the pointer's base to the far edge, 0.12 h tall at the
/// tag's y) moved 0.015 h towards the pointer.
fn text_rect(rect: Rect, v: f32, faces_right: bool) -> Rect {
    let (h, y) = (rect.height(), tag_y(rect, v));
    let (back, _, base) = ends(rect, faces_right);
    let shift = if faces_right { TEXT_SHIFT } else { -TEXT_SHIFT } * h;
    Rect::from_x_y_ranges(
        back.min(base) + shift..=back.max(base) + shift,
        y - TAG_H / 2.0 * h..=y + TAG_H / 2.0 * h,
    )
}

/// A tag slider in `c.rect`: the tag moves over the full height (bottom = 0, top = 1) and points right
/// (`faces_right`, towards a display on its right) or left. Vertical drags move it (up increases) as the vertical
/// slider's, following the pointer over its 0.88 h of travel. There is no label or value strip, and no hover look (it
/// has no track). The tag carries `c.label`, or `display(value)` while it is dragged, in FontSize 2: plain text,
/// centred, neither shrunk nor clipped.
pub fn tag_slider(
    ui: &mut Ui,
    cx: &UiCtx,
    c: &Control,
    value: &mut f32,
    faces_right: bool,
    display: impl Fn(f32) -> String,
) -> Response {
    // The tag travels 0.88 h from 0 to 1.
    let law = Law::Track {
        len: (1.0 - TAG_H) * c.rect.height(),
        horizontal: false,
    };
    let (response, v) = drag_control(ui, cx, c, value, law);
    // An empty rect has no tag to draw.
    if c.rect.width() > 0.0 && c.rect.height() > 0.0 {
        let fill = c.channel.gamma_multiply(TAG_ALPHA);
        let outline = tag(c.rect, v, faces_right).to_vec();
        ui.painter()
            .add(Shape::convex_polygon(outline, fill, Stroke::NONE));
    }
    let text = if primary_down_on(ui, &response) {
        display(v)
    } else {
        c.label.to_owned()
    };
    // Plain text drawing: centred, never shrunk or clipped; wider text overflows evenly.
    ui.painter().text(
        text_rect(c.rect, v, faces_right).center(),
        Align2::CENTER_CENTER,
        text,
        cx.m.font_id(FontSize::Pt10),
        c.channel,
    );
    response
}

#[cfg(test)]
mod tests {
    use super::*;
    use eframe::egui::pos2;

    fn near(a: f32, b: f32) -> bool {
        (a - b).abs() < 1e-3
    }

    /// Tremolo's `Max` at the reference: 0.98 × 4.26 U, with U = 100 for round numbers.
    const R: Rect = Rect {
        min: pos2(0.0, 0.0),
        max: pos2(98.0, 426.0),
    };

    #[test]
    fn the_tag_runs_over_the_full_height() {
        // 0.06 h = 25.56, 0.88 h = 374.88.
        assert!(near(tag_y(R, 0.0), 426.0 - 25.56));
        assert!(near(tag_y(R, 1.0), 25.56));
        assert!(near(tag_y(R, 0.5), 213.0));
    }

    #[test]
    fn a_right_facing_tag_points_at_the_right_edge() {
        let y = tag_y(R, 0.0);
        let p = tag(R, 0.0, true);
        // Bar from the left edge to 0.06 h before the right edge, 0.12 h tall; tip on the right edge at y.
        let (top, bottom, base) = (y - 25.56, y + 25.56, 98.0 - 25.56);
        let want = [
            pos2(0.0, top),
            pos2(base, top),
            pos2(98.0, y),
            pos2(base, bottom),
            pos2(0.0, bottom),
        ];
        for (a, b) in p.iter().zip(want) {
            assert!(a.distance(b) < 1e-3, "{p:?}");
        }
        // At 0 the tag sits on the bottom edge.
        assert!(near(bottom, 426.0));
    }

    #[test]
    fn a_left_facing_tag_is_the_mirror_image() {
        for v in [0.0, 0.3, 1.0] {
            let (r, l) = (tag(R, v, true), tag(R, v, false));
            let mirrored = |p: Pos2| pos2(98.0 - p.x, p.y);
            for p in r {
                assert!(
                    l.iter().any(|q| q.distance(mirrored(p)) < 1e-3),
                    "{v}: {l:?}"
                );
            }
            // The tip is on the left edge.
            assert!(l.iter().any(|q| near(q.x, 0.0) && near(q.y, tag_y(R, v))));
        }
    }

    #[test]
    fn the_text_rect_is_the_bar_moved_towards_the_pointer() {
        // 0.06 h = 25.56, 0.015 h = 6.39; the bar is 98 − 25.56 = 72.44 wide and 0.12 h = 51.12 tall at the tag's y.
        for v in [0.0, 0.5, 1.0] {
            let y = tag_y(R, v);
            let rows = y - 25.56..=y + 25.56;
            // Facing right: the bar runs from the left edge to the pointer's base, then moves right.
            let want = Rect::from_x_y_ranges(6.39..=72.44 + 6.39, rows.clone());
            let got = text_rect(R, v, true);
            assert!(
                got.min.distance(want.min) < 1e-3 && got.max.distance(want.max) < 1e-3,
                "{got:?}"
            );
            // Facing left: from the pointer's base to the right edge, moved left.
            let want = Rect::from_x_y_ranges(25.56 - 6.39..=98.0 - 6.39, rows);
            let got = text_rect(R, v, false);
            assert!(
                got.min.distance(want.min) < 1e-3 && got.max.distance(want.max) < 1e-3,
                "{got:?}"
            );
        }
    }

    #[test]
    fn the_pointer_never_reaches_past_the_far_edge() {
        // A rect narrower than 0.06 h: the pointer takes the whole width.
        let thin = Rect::from_min_max(pos2(0.0, 0.0), pos2(10.0, 426.0));
        assert!(
            tag(thin, 0.5, true)
                .iter()
                .all(|p| (0.0..=10.0).contains(&p.x))
        );
        assert!(
            tag(thin, 0.5, false)
                .iter()
                .all(|p| (0.0..=10.0).contains(&p.x))
        );
    }
}
