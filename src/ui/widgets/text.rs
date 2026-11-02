//! The text routines. Plain drawing is egui's own `Painter::text` at the alignment's point of the rect: placed, never
//! clipped or shortened, and epaint rounds the glyphs to whole pixels. The others are here: truncated drawing (a middle
//! ellipsis keeping `k` tail characters), shrink drawing and fit-only sizing. Controls not using one of these are
//! drawn plain.

use std::sync::Arc;

use eframe::egui::{Align2, Color32, FontId, Galley, Painter, Rect, Vec2, vec2};

use crate::metrics::Metrics;

/// The smallest size the shrink and fit-only searches go down to, in points.
const MIN_PT: u8 = 6;
/// The tail kept when shrink drawing falls back to truncated drawing at 6 pt.
const SHRINK_K: usize = 5;
/// The ellipsis: three ASCII periods.
const DOTS: &str = "...";

/// What truncated drawing shows of `text` in `max_w`: `text` when its `whole` width fits (equality fits), else the
/// head, `...` and the tail. The tail starts as the last t = min(⌊(n − 3)/2⌋, `k`) characters and the head as the
/// first n − 3 − t; then, measured as the sum of `char_w` plus `dots` (the width of `...`), the head loses its last
/// character while it is longer than the tail, else the tail its first, until it fits. Three characters or fewer
/// give `...`; nothing fits when even `...` alone is too wide.
fn truncate(
    text: &str,
    k: usize,
    max_w: f32,
    whole: f32,
    dots: f32,
    mut char_w: impl FnMut(char) -> f32,
) -> String {
    if whole <= max_w {
        return text.to_owned();
    }
    let chars: Vec<char> = text.chars().collect();
    let n = chars.len();
    if n <= 3 {
        return DOTS.to_owned();
    }
    // The head and tail lengths.
    let mut tail = ((n - 3) / 2).min(k);
    let mut head = n - 3 - tail;
    let w: Vec<f32> = chars.iter().map(|&c| char_w(c)).collect();
    let width = |r: std::ops::Range<usize>| w[r].iter().sum::<f32>();
    while width(0..head) + dots + width(n - tail..n) > max_w {
        if head > tail {
            head -= 1;
        } else if tail > 0 {
            tail -= 1;
        } else {
            // Even `...` alone is too wide.
            return String::new();
        }
    }
    let mut shown: String = chars[..head].iter().collect();
    shown.push_str(DOTS);
    shown.extend(&chars[n - tail..]);
    shown
}

/// Truncated drawing: `text` at `font` placed in `rect` by `align`, shortened to the rect's width with a middle
/// ellipsis that keeps up to `k` tail characters (`k` = 0: an end ellipsis).
pub fn truncated(
    painter: &Painter,
    rect: Rect,
    align: Align2,
    text: &str,
    font: FontId,
    color: Color32,
    k: usize,
) {
    let shown = truncated_text(painter, text, font.clone(), rect.width(), k);
    painter.text(align.pos_in_rect(&rect), align, shown, font, color);
}

/// What [`truncated`] draws of `text` at `font` in `max_w`: for a label laid out elsewhere (a popup item).
pub fn truncated_text(painter: &Painter, text: &str, font: FontId, max_w: f32, k: usize) -> String {
    let color = Color32::PLACEHOLDER;
    let whole = painter
        .layout_no_wrap(text.to_owned(), font.clone(), color)
        .size()
        .x;
    let dots = painter
        .layout_no_wrap(DOTS.to_owned(), font.clone(), color)
        .size()
        .x;
    // Each character laid out alone, not epaint's `glyph_width`: that ignores the font's `FontTweak::scale`. The
    // placeholder colour shares the galley cache across colours.
    let char_w = |c: char| {
        painter
            .layout_no_wrap(c.into(), font.clone(), color)
            .size()
            .x
    };
    truncate(text, k, max_w, whole, dots, char_w)
}

/// The layout of `text` at the largest point size from `from` down to 6 pt, a point at a time, that fits `size`
/// (an infinite height checks the width only): `Ok`, or `Err` with the 6 pt layout when none does.
fn first_fit(
    painter: &Painter,
    m: &Metrics,
    text: &str,
    from: u8,
    size: Vec2,
    color: Color32,
) -> Result<Arc<Galley>, Arc<Galley>> {
    let layout =
        |pt: u8| painter.layout_no_wrap(text.to_owned(), m.font_id_pt(f32::from(pt)), color);
    let fits = |g: &Arc<Galley>| g.size().x <= size.x && g.size().y <= size.y;
    (MIN_PT..=from.max(MIN_PT))
        .rev()
        .map(layout)
        .find(fits)
        .ok_or_else(|| layout(MIN_PT))
}

/// Fit-only sizing (knob and arc-slider values): `text` laid out at the largest size from 12 pt down to 6 pt whose
/// width is at most `max_w`, else at 6 pt, to overflow plainly. Never shortened.
pub(crate) fn fit_only(
    painter: &Painter,
    m: &Metrics,
    text: &str,
    max_w: f32,
    color: Color32,
) -> Arc<Galley> {
    first_fit(painter, m, text, 12, vec2(max_w, f32::INFINITY), color).unwrap_or_else(|g| g)
}

/// Fitted to `size` like [`fit_only`] but on both axes: the graph knob's label in its strip.
pub(crate) fn fit_both(
    painter: &Painter,
    m: &Metrics,
    text: &str,
    size: Vec2,
    color: Color32,
) -> Arc<Galley> {
    first_fit(painter, m, text, 12, size, color).unwrap_or_else(|g| g)
}

/// Shrink drawing: `text` placed in `rect` by `align` at the largest size from `from` pt down to 6 pt whose width
/// fits the rect; when 6 pt is still too wide, truncated drawing at 6 pt keeping 5 tail characters.
pub fn shrink(
    painter: &Painter,
    m: &Metrics,
    rect: Rect,
    align: Align2,
    text: &str,
    from: u8,
    color: Color32,
) {
    match first_fit(
        painter,
        m,
        text,
        from,
        vec2(rect.width(), f32::INFINITY),
        color,
    ) {
        Ok(galley) => {
            let pos = align
                .anchor_size(align.pos_in_rect(&rect), galley.size())
                .min;
            painter.galley(pos, galley, color);
        }
        Err(_) => truncated(
            painter,
            rect,
            align,
            text,
            m.font_id_pt(f32::from(MIN_PT)),
            color,
            SHRINK_K,
        ),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Every character 1 wide and `...` 1.5 wide, so that the ellipsis is narrower than the three it replaces.
    fn cut(text: &str, k: usize, max_w: f32) -> String {
        truncate(text, k, max_w, text.chars().count() as f32, 1.5, |_| 1.0)
    }

    #[test]
    fn text_that_fits_is_kept_whole_and_equality_fits() {
        assert_eq!(cut("Reverb Hall Large", 5, 17.0), "Reverb Hall Large");
        assert_eq!(cut("Reverb Hall Large", 5, 100.0), "Reverb Hall Large");
    }

    #[test]
    fn k_5_keeps_the_tail_and_shortens_the_longer_side_first() {
        // The 3 characters past the head go first, then the head down to the tail's length, then the two take turns,
        // the tail first on a tie.
        let full = "Reverb Hall Large";
        assert_eq!(cut(full, 5, 16.9), "Reverb Ha...Large");
        assert_eq!(cut(full, 5, 15.5), "Reverb Ha...Large");
        assert_eq!(cut(full, 5, 15.4), "Reverb H...Large");
        assert_eq!(cut(full, 5, 11.5), "Rever...Large");
        assert_eq!(cut(full, 5, 11.4), "Rever...arge");
        assert_eq!(cut(full, 5, 10.4), "Reve...arge");
        assert_eq!(cut(full, 5, 9.4), "Reve...rge");
        assert_eq!(cut(full, 5, 1.5), "...");
    }

    #[test]
    fn k_0_is_an_end_ellipsis() {
        assert_eq!(cut("Long name here", 0, 10.0), "Long nam...");
        assert_eq!(cut("Long name here", 0, 1.5), "...");
    }

    #[test]
    fn the_tail_is_at_most_half_of_what_is_left() {
        // n = 9: t = min(⌊6 / 2⌋, 8) = 3, head = 3.
        assert_eq!(cut("abcdefghi", 8, 8.5), "abc...ghi");
        assert_eq!(cut("abcdefghi", 8, 7.4), "abc...hi");
    }

    #[test]
    fn three_characters_or_fewer_give_dots_and_nothing_fits_when_dots_do_not() {
        assert_eq!(cut("abc", 5, 2.0), "...");
        assert_eq!(cut("abc", 5, 0.0), "...");
        assert_eq!(cut("abcd", 5, 1.4), "");
        assert_eq!(cut("abcdefgh", 0, 0.0), "");
    }

    #[test]
    fn the_fit_sums_character_widths_but_the_whole_test_uses_the_whole_width() {
        // Kerned, the whole string fits; summed it would not. Not kerned enough: summed widths decide.
        assert_eq!(truncate("AVAVAV", 5, 5.0, 5.0, 1.5, |_| 1.0), "AVAVAV");
        let w = |c| if c == 'A' { 2.0 } else { 0.5 };
        assert_eq!(truncate("AVAVAV", 5, 4.9, 5.0, 1.5, w), "AV...V");
        assert_eq!(truncate("AVAVAV", 5, 4.4, 5.0, 1.5, w), "A...V");
    }
}
