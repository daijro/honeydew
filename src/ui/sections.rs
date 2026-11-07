//! The playlist's song sections (#80, a Honeydew addition): each tints the track rows' cells over its range in its colour
//! at α [`TINT_ALPHA`], under the clips, and names itself on the ruler, where a tap on its name opens its dialog
//! ([`crate::ui::views::section_settings`]). The ruler's region menu ends with `Section…`, which makes the range's
//! section ([`crate::song::Song::section_at`]).

use eframe::egui::{Align2, Painter, Pos2, Rect, Ui, pos2, vec2};

use crate::metrics::FontSize;
use crate::song::Section;
use crate::theme::channel_color;
use crate::ui::UiCtx;
use crate::ui::clips::TimeAxis;
use crate::ui::main_screen::TrackRow;
use crate::ui::popup::PopupItem;
use crate::ui::region::{self, RegionOp, TimeRange};
use crate::ui::timeline::BOX_W;
use crate::ui::widgets::truncated;

/// The tint's alpha over the rows: low, so that it reads on every theme.
const TINT_ALPHA: f32 = 0.2;
/// The name tag on the ruler, in U: its height, its bottom above the ruler's bottom edge (clear of the baseline), the
/// text's inset and the corner radius. Under the bar labels.
const TAG_H: f32 = 0.6;
const TAG_BOTTOM: f32 = 0.15;
const TAG_PAD: f32 = 0.15;
const TAG_R: f32 = 0.1;
/// The section's line along the ruler over its range, under its tag, in hairlines t.
const LINE_T: f32 = 2.0;

/// The playlist's region menu: the editors' items ([`region::items`]), then `Section…`.
pub fn region_items(range: TimeRange, playhead: Option<f64>) -> Vec<PopupItem<RegionOp>> {
    let mut items = region::items(range, playhead, false);
    items.push(PopupItem::new(
        RegionOp::Section,
        0,
        items.len(),
        "Section…",
    ));
    items
}

/// Tints the cells of the track `rows` (each with its top, `row_h` tall, the last device pixel the gap) in the clip
/// `grid` over each section's range on `axis`, in its colour at α [`TINT_ALPHA`]: the rows only, not the background
/// below the last one.
pub fn tint(
    ui: &Ui,
    grid: Rect,
    rows: &[(TrackRow, f32)],
    row_h: f32,
    axis: TimeAxis,
    sections: &[Section],
    cx: &UiCtx,
) {
    let (painter, px) = (ui.painter_at(grid), 1.0 / cx.m.ppp());
    for s in sections {
        let color = channel_color(s.hue, cx.theme).to_color32();
        let color = color.gamma_multiply(TINT_ALPHA);
        let x = axis.x(s.start.into())..=axis.x(s.end.into());
        for &(t, top) in rows {
            let cells = Rect::from_x_y_ranges(x.clone(), top..=top + row_h * t.rows() - px);
            painter.rect_filled(cells, 0.0, color);
        }
    }
}

/// Each section's name on the ruler `strip` (the marks part) on `axis`: a line in its colour along the ruler over its
/// range, and on it at the range's start (or the strip's left edge while that is scrolled past) a tag in its colour
/// with the name in the contrasting text colour, as wide as the name and never past the range. A tag the play box
/// would cover (it shows right of the playhead at `knob` while the transport is stopped) starts right of the box.
/// Returns each shown tag, by section index, for its tap.
pub fn ruler(
    painter: &Painter,
    strip: Rect,
    axis: TimeAxis,
    sections: &[Section],
    knob: Option<f32>,
    cx: &UiCtx,
) -> Vec<(usize, Rect)> {
    let (m, colors) = (cx.m, &cx.theme.colors);
    let painter = painter.with_clip_rect(painter.clip_rect().intersect(strip));
    let font = m.font_id(FontSize::Pt8);
    let bottom = strip.bottom() - m.u(TAG_BOTTOM);
    let mut tags = Vec::new();
    for (i, s) in sections.iter().enumerate() {
        let (x0, x1) = (axis.x(s.start.into()), axis.x(s.end.into()));
        if x1 <= strip.left() || x0 >= strip.right() {
            continue;
        }
        let color = channel_color(s.hue, cx.theme);
        let ink = colors.best_contrasting(color, &colors.contrast_candidates());
        let line = Rect::from_x_y_ranges(x0..=x1, bottom - LINE_T * m.t()..=bottom);
        painter.rect_filled(line, 0.0, color.to_color32());
        let (ink, pad) = (ink.to_color32(), m.u(TAG_PAD));
        let galley = painter.layout_no_wrap(s.name.clone(), font.clone(), ink);
        let need = galley.size().x + 2.0 * pad;
        let mut left = x0.max(strip.left());
        if let Some(k) = knob.filter(|&k| k < left + need && left < k + m.u(BOX_W)) {
            left = k + m.u(BOX_W);
        }
        let room = x1 - left;
        if room <= 2.0 * pad {
            continue;
        }
        let tag = Rect::from_min_max(
            pos2(left, bottom - m.u(TAG_H)),
            pos2(left + need.min(room), bottom),
        );
        painter.rect_filled(tag, m.u(TAG_R), color.to_color32());
        let text = tag.shrink2(vec2(pad, 0.0));
        match need <= room {
            true => painter.galley(
                text.left_center() - vec2(0.0, galley.size().y / 2.0),
                galley,
                ink,
            ),
            false => truncated(
                &painter,
                text,
                Align2::LEFT_CENTER,
                &s.name,
                font.clone(),
                ink,
                5,
            ),
        }
        tags.push((i, tag));
    }
    tags
}

/// The section whose tag (of [`ruler`]'s `tags`) is at `p`, the last drawn (topmost) first.
pub fn tag_at(tags: &[(usize, Rect)], p: Pos2) -> Option<usize> {
    tags.iter().rev().find(|t| t.1.contains(p)).map(|t| t.0)
}
