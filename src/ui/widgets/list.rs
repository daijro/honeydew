//! Item lists: tab strips, horizontal selectors and vertical list selectors.
//!
//! The items split the rect into equal cells (columns for a horizontal list, rows for a vertical one). Each cell
//! has a 2 t marker bar on the list's bar side and its text is centred in the cell without 3 t on that side. The
//! selected item is drawn in `text` with a `text` bar, the others in the channel colour with a `menuFill` bar. A
//! boxed list has no bars: its selected cell gets a `menuFill` box inset 2 t.
//!
//! Header items are drawn at FontSize 3 and, as in the popup menus, cannot be selected. A pressed cell gets a light
//! wash (`text` at 10 %) as its press highlight.

use eframe::egui::{Align2, Color32, Id, Pos2, Rect, Response, Sense, Ui, WidgetInfo, WidgetType};

use super::{primary_down_on, through_index};
use crate::metrics::FontSize;
use crate::ui::UiCtx;
use crate::ui::focus::{self, Kind};

/// Alpha of the wash under a pressed cell.
const PRESS_WASH: f32 = 0.1;

/// Where each cell's marker bar sits, which also says how the list runs.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Bar {
    /// Horizontal list (tab strips, h-selectors): the cells are columns side by side.
    Bottom,
    /// Vertical list, the default: the cells are rows.
    Right,
    /// Vertical list with its bars on the left.
    Left,
    /// Horizontal list with no bars: the selected cell gets a `menuFill` box inset 2 t (MiniSynth's FX sub-tabs).
    Boxed,
}

/// A list: `items` are the labels, `headers` the indices of the header items.
#[derive(Clone, Copy, Debug)]
pub struct ItemList<'a> {
    pub id: Id,
    pub rect: Rect,
    pub items: &'a [&'a str],
    pub headers: &'a [usize],
    pub bar: Bar,
    /// The module's channel colour: every unselected item.
    pub channel: Color32,
    /// No input.
    pub disabled: bool,
}

impl<'a> ItemList<'a> {
    pub fn new(id: Id, rect: Rect, items: &'a [&'a str], bar: Bar, channel: Color32) -> Self {
        Self {
            id,
            rect,
            items,
            headers: &[],
            bar,
            channel,
            disabled: false,
        }
    }
}

/// One item's rects: the whole cell, its marker bar (a boxed list's box) and the area its text is centred in.
#[derive(Clone, Copy, Debug, PartialEq)]
struct Cell {
    cell: Rect,
    bar: Rect,
    text: Rect,
}

/// Cell `index` of `len` equal cells filling `rect`. `t` is the hairline unit.
fn cell(rect: Rect, len: usize, index: usize, bar: Bar, t: f32) -> Cell {
    // Both edges come from the same expression, so neighbours meet exactly and the last one ends at the rect.
    let edge = |lo: f32, hi: f32, k: usize| {
        if k >= len {
            hi
        } else {
            lo + (hi - lo) * k as f32 / len as f32
        }
    };
    let cell = match bar {
        Bar::Bottom | Bar::Boxed => Rect::from_x_y_ranges(
            edge(rect.left(), rect.right(), index)..=edge(rect.left(), rect.right(), index + 1),
            rect.y_range(),
        ),
        Bar::Right | Bar::Left => Rect::from_x_y_ranges(
            rect.x_range(),
            edge(rect.top(), rect.bottom(), index)..=edge(rect.top(), rect.bottom(), index + 1),
        ),
    };
    let (bar, text) = match bar {
        Bar::Bottom => (
            cell.with_min_y(cell.bottom() - 2.0 * t),
            cell.with_max_y(cell.bottom() - 3.0 * t),
        ),
        // The box takes the bar's place; the text is centred in it.
        Bar::Boxed => (cell.shrink(2.0 * t), cell.shrink(2.0 * t)),
        Bar::Right => (
            cell.with_min_x(cell.right() - 2.0 * t),
            cell.with_max_x(cell.right() - 3.0 * t),
        ),
        Bar::Left => (
            cell.with_max_x(cell.left() + 2.0 * t),
            cell.with_min_x(cell.left() + 3.0 * t),
        ),
    };
    Cell { cell, bar, text }
}

/// The cell under `pos`; the far edges belong to the last cell.
fn cell_at(rect: Rect, len: usize, bar: Bar, pos: Pos2) -> Option<usize> {
    if len == 0 || !rect.contains(pos) {
        return None;
    }
    let along = match bar {
        Bar::Bottom | Bar::Boxed => (pos.x - rect.left()) / rect.width(),
        Bar::Right | Bar::Left => (pos.y - rect.top()) / rect.height(),
    };
    Some(((along * len as f32) as usize).min(len - 1))
}

/// A list bound to a menu parameter: `value` is its normalised value and the selected item is
/// `menu_index(value, n)`.
pub fn item_list(ui: &mut Ui, cx: &UiCtx, list: &ItemList, value: &mut f32) -> Response {
    through_index(value, list.items.len(), |index| {
        item_list_index(ui, cx, list, index)
    })
}

/// A list that is not a parameter (page tabs, UI state): `selected` is the index of the selected item. An index
/// past the end selects nothing until the user picks an item.
pub fn item_list_index(ui: &mut Ui, cx: &UiCtx, list: &ItemList, selected: &mut usize) -> Response {
    let (m, colors) = (cx.m, &cx.theme.colors);
    let (rect, len, t) = (list.rect, list.items.len(), m.t());
    let sense = if list.disabled {
        Sense::hover()
    } else {
        Sense::click()
    };
    let mut response = ui.interact(rect, list.id, sense);
    let pick = |pos: Option<Pos2>| {
        pos.and_then(|p| cell_at(rect, len, list.bar, p))
            .filter(|i| !list.headers.contains(i))
    };
    if response.clicked()
        && let Some(i) = pick(response.interact_pointer_pos())
        && i != *selected
    {
        *selected = i;
        response.mark_changed();
    }
    // Each item is a focus target of its own; `open` on the focused one picks it, as a tap does.
    if !list.disabled {
        for i in (0..len).filter(|i| !list.headers.contains(i)) {
            let item = list.id.with(("item", i));
            focus::register(ui, item, cell(rect, len, i, list.bar, t).cell, Kind::Button);
            if focus::activated(ui.ctx(), item) && i != *selected {
                *selected = i;
                response.mark_changed();
            }
        }
    }
    // A group named by its selected item: the list has no caption.
    let shown = list.items.get(*selected).copied().unwrap_or_default();
    response.widget_info(|| WidgetInfo::labeled(WidgetType::RadioGroup, !list.disabled, shown));
    let pressed = primary_down_on(ui, &response)
        .then(|| pick(ui.input(|i| i.pointer.interact_pos())))
        .flatten();

    let (text, menu_fill) = (colors.text.to_color32(), colors.menu_fill.to_color32());
    let painter = ui.painter();
    for (i, label) in list.items.iter().enumerate() {
        let is_selected = i == *selected;
        let cell = cell(rect, len, i, list.bar, t);
        if pressed == Some(i) {
            painter.rect_filled(cell.cell, 0.0, text.gamma_multiply(PRESS_WASH));
        }
        // Bars: `text` under the selected item, `menuFill` under the others. Boxed: only the selected item has its
        // `menuFill` box. A cell too small for its marker has none.
        let boxed = list.bar == Bar::Boxed;
        if (is_selected || !boxed) && cell.bar.is_positive() {
            let fill = if is_selected && !boxed {
                text
            } else {
                menu_fill
            };
            painter.rect_filled(cell.bar, 0.0, fill);
        }
        let font = if list.headers.contains(&i) {
            FontSize::Pt12
        } else {
            FontSize::Pt10
        };
        // Plain text: an item wider than its cell overhangs it.
        painter.text(
            cell.text.center(),
            Align2::CENTER_CENTER,
            label,
            m.font_id(font),
            if is_selected { text } else { list.channel },
        );
    }
    response
}

#[cfg(test)]
mod tests {
    use super::*;
    use eframe::egui::{pos2, vec2};

    const T: f32 = 2.0;

    #[test]
    fn horizontal_cells_are_equal_columns_with_the_bar_at_the_bottom() {
        let rect = Rect::from_min_size(pos2(10.0, 20.0), vec2(140.0, 40.0));
        let c = |i| cell(rect, 4, i, Bar::Bottom, T);
        assert_eq!(
            c(0),
            Cell {
                cell: Rect::from_min_max(pos2(10.0, 20.0), pos2(45.0, 60.0)),
                // 2 t high at the bottom of the cell, the full cell width.
                bar: Rect::from_min_max(pos2(10.0, 56.0), pos2(45.0, 60.0)),
                // The text area gives up 3 t at the bottom.
                text: Rect::from_min_max(pos2(10.0, 20.0), pos2(45.0, 54.0)),
            }
        );
        assert_eq!((c(3).cell.left(), c(3).cell.right()), (115.0, 150.0));
        // The cells tile the rect without gaps.
        for i in 0..3 {
            assert_eq!(c(i).cell.right(), c(i + 1).cell.left());
        }
        assert_eq!(c(3).cell.right(), rect.right());
    }

    #[test]
    fn vertical_lists_put_the_bar_on_the_right_by_default() {
        let rect = Rect::from_min_size(pos2(0.0, 0.0), vec2(60.0, 90.0));
        let c = cell(rect, 3, 1, Bar::Right, T);
        assert_eq!(
            c.cell,
            Rect::from_min_max(pos2(0.0, 30.0), pos2(60.0, 60.0))
        );
        assert_eq!(
            c.bar,
            Rect::from_min_max(pos2(56.0, 30.0), pos2(60.0, 60.0))
        );
        assert_eq!(
            c.text,
            Rect::from_min_max(pos2(0.0, 30.0), pos2(54.0, 60.0))
        );
        for i in 0..2 {
            assert_eq!(
                cell(rect, 3, i, Bar::Right, T).cell.bottom(),
                cell(rect, 3, i + 1, Bar::Right, T).cell.top()
            );
        }
        assert_eq!(cell(rect, 3, 2, Bar::Right, T).cell.bottom(), rect.bottom());
    }

    #[test]
    fn bar_left_mirrors_the_bar_and_the_text_margin() {
        let rect = Rect::from_min_size(pos2(0.0, 0.0), vec2(60.0, 90.0));
        let c = cell(rect, 3, 1, Bar::Left, T);
        assert_eq!(
            c.cell,
            Rect::from_min_max(pos2(0.0, 30.0), pos2(60.0, 60.0))
        );
        assert_eq!(c.bar, Rect::from_min_max(pos2(0.0, 30.0), pos2(4.0, 60.0)));
        assert_eq!(
            c.text,
            Rect::from_min_max(pos2(6.0, 30.0), pos2(60.0, 60.0))
        );
    }

    #[test]
    fn hit_testing_follows_the_cells_and_the_list_direction() {
        let rect = Rect::from_min_size(pos2(10.0, 20.0), vec2(120.0, 30.0));
        let at = |x: f32, y: f32| cell_at(rect, 4, Bar::Bottom, pos2(x, y));
        assert_eq!(at(11.0, 21.0), Some(0));
        assert_eq!(at(39.9, 49.0), Some(0));
        assert_eq!(at(40.1, 21.0), Some(1));
        assert_eq!(at(129.0, 30.0), Some(3));
        // The far edge belongs to the last cell; outside is nothing.
        assert_eq!(at(130.0, 50.0), Some(3));
        assert_eq!(at(9.0, 30.0), None);
        assert_eq!(at(131.0, 30.0), None);
        assert_eq!(at(50.0, 19.0), None);
        // Vertical: y picks the cell.
        let tall = Rect::from_min_size(pos2(0.0, 0.0), vec2(40.0, 90.0));
        for bar in [Bar::Right, Bar::Left] {
            assert_eq!(cell_at(tall, 3, bar, pos2(5.0, 29.0)), Some(0));
            assert_eq!(cell_at(tall, 3, bar, pos2(35.0, 31.0)), Some(1));
            assert_eq!(cell_at(tall, 3, bar, pos2(5.0, 89.0)), Some(2));
        }
        // An empty list has no cells.
        assert_eq!(cell_at(tall, 0, Bar::Right, pos2(5.0, 5.0)), None);
    }

    #[test]
    fn boxed_cells_are_columns_whose_box_and_text_are_the_cell_inset_2t() {
        let rect = Rect::from_min_size(pos2(10.0, 20.0), vec2(140.0, 40.0));
        let c = |i| cell(rect, 4, i, Bar::Boxed, T);
        let inset = Rect::from_min_max(pos2(14.0, 24.0), pos2(41.0, 56.0));
        assert_eq!(
            c(0),
            Cell {
                cell: Rect::from_min_max(pos2(10.0, 20.0), pos2(45.0, 60.0)),
                bar: inset,
                text: inset,
            }
        );
        assert_eq!((c(3).cell.left(), c(3).cell.right()), (115.0, 150.0));
        // Hit testing runs along x, as for a tab strip.
        assert_eq!(cell_at(rect, 4, Bar::Boxed, pos2(50.0, 58.0)), Some(1));
        assert_eq!(cell_at(rect, 4, Bar::Boxed, pos2(149.0, 21.0)), Some(3));
    }
}
