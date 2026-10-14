//! The inverted popup menu: a grid of text items in `text` colour with `background` ink. No border, shadow or rounding.

use std::borrow::Cow;

use eframe::egui::{
    self, Align2, Area, Color32, Context, Id, LayerId, Order, Pos2, Rect, ScrollArea, Sense, Vec2,
    pos2, scroll_area::ScrollBarVisibility, vec2,
};

use crate::metrics::{FontSize, Metrics};
use crate::theme::DISABLED_ALPHA;
use crate::ui::{UiCtx, anim};

// Sizes in U.
const ROW_H: f32 = 1.25;
/// Rows shrink to fit a popup into its bounds (the 14-row Project Scale picker in a 16.9 U high window), down to
/// this; below it the popup scrolls.
const MIN_ROW_H: f32 = 1.0;
pub(crate) const PAD_X: f32 = 0.5;
/// The `dot` flag: a filled circle at the item's right edge, with a gap between it and the text.
const DOT_R: f32 = 0.1;
const DOT_GAP: f32 = 0.25;
/// RGB factor of the hovered or pressed item's band: slightly darker.
const BAND_SHADE: f32 = 0.85;
/// The opening sweep's bar width (device px).
const BAR_PX: f32 = 2.0;
/// A message box's hovered or pressed item: a `text` band at this alpha (as the side browser's rows).
const BOX_BAND_ALPHA: f32 = 0.15;

/// One cell of a popup. Cells sit at (`col`, `row`); the grid is as large as the highest column and row.
#[derive(Clone, Debug, PartialEq)]
pub struct PopupItem<T> {
    /// What selecting the item returns; `None` makes it a header: section font, never selectable.
    pub id: Option<T>,
    pub label: Cow<'static, str>,
    pub col: usize,
    pub row: usize,
    /// Dimmed and not selectable.
    pub disabled: bool,
    /// A small filled circle at the right edge ("new content", "option active").
    pub dot: bool,
    /// Selecting the item does not close the popup.
    pub keep_open: bool,
    /// The value the menu stands for: drawn as a `background` bar with a `text` label.
    pub current: bool,
}

impl<T> PopupItem<T> {
    pub fn new(id: T, col: usize, row: usize, label: impl Into<Cow<'static, str>>) -> Self {
        Self {
            id: Some(id),
            ..Self::header(col, row, label)
        }
    }

    pub fn header(col: usize, row: usize, label: impl Into<Cow<'static, str>>) -> Self {
        Self {
            id: None,
            label: label.into(),
            col,
            row,
            disabled: false,
            dot: false,
            keep_open: false,
            current: false,
        }
    }

    pub fn is_header(&self) -> bool {
        self.id.is_none()
    }
}

/// What the popup wants this frame.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct PopupResult<T> {
    /// The item selected this frame.
    pub selected: Option<T>,
    /// The caller should stop showing the popup: a click outside it, Escape, or an item that is not `keep_open`.
    pub close: bool,
}

/// Whether popup `id` was opened this frame: it was not shown the frame before (callers show it every frame while it
/// is open), or never. Every pass of the frame agrees.
fn just_opened(ctx: &Context, id: Id) -> bool {
    let (key, frame) = (id.with("opened"), ctx.cumulative_frame_nr());
    ctx.data_mut(|d| {
        let (opened, seen) = d.get_temp::<(u64, u64)>(key).unwrap_or((frame, frame));
        let opened = if seen + 1 < frame { frame } else { opened };
        d.insert_temp(key, (opened, frame));
        opened == frame
    })
}

/// Shows the popup centred on `center`, kept inside `bounds`, in the foreground layer. Call every frame
/// while it is open. `id` names the popup; item widgets get ids derived from it and their (column, row).
///
/// The popup is modal. A full-screen backdrop just below it takes every click outside the popup, so those
/// clicks close it (`close`) and never reach the widgets underneath, and Escape closes it too. A backdrop that
/// first appears this frame cannot have seen the click that opened the popup.
///
/// Escape is consumed here, so with two popups open the one shown first in the frame takes it: show a popup opened
/// over another before that one (egui still draws the newer on top), as a container shows its popups before its
/// own Escape check.
///
/// It opens with the menu sweep, one step per drawn frame: the screen dims (`background` α 0.6) at once, a 2 px bar
/// grows at the popup's left edge to full height, then the fill grows rightwards to full width ([`Sweep`]); the items
/// show from frame 23. The items take input from the first frame, and a close is instant. A popup not shown for a frame
/// opens from the start again.
pub fn show<T: Copy>(
    ctx: &Context,
    id: Id,
    center: Pos2,
    bounds: Rect,
    items: &[PopupItem<T>],
    cx: &UiCtx,
) -> PopupResult<T> {
    let (phase, a) = anim::sweep(anim::frame(ctx, id));
    if phase < 3 {
        ctx.request_repaint();
    }
    // The dim goes into the popup's own layer before the popup: under it, over everything else.
    let ink = cx.theme.colors.background.to_color32();
    ctx.layer_painter(LayerId::new(Order::Foreground, id))
        .rect_filled(ctx.content_rect(), 0.0, anim::dim(ink, anim::DIM_ALPHA));
    show_as(
        ctx,
        id,
        center,
        bounds,
        items,
        cx,
        Sweep { phase, a },
        false,
    )
}

/// A message box's items (`views::message`), in its own look: `text` labels on the `background` box the caller paints
/// under them (in the popup's layer, `Order::Foreground` with `id`), without the popup's fill, sweep or dim. The items
/// take input whether `drawn` or not.
pub(crate) fn show_on_box<T: Copy>(
    ctx: &Context,
    id: Id,
    center: Pos2,
    bounds: Rect,
    items: &[PopupItem<T>],
    cx: &UiCtx,
    drawn: bool,
) -> PopupResult<T> {
    let phase = if drawn { 3 } else { 0 };
    show_as(
        ctx,
        id,
        center,
        bounds,
        items,
        cx,
        Sweep { phase, a: 1.0 },
        true,
    )
}

/// The rect a popup of `items` centred on `center` takes inside `bounds`.
pub(crate) fn popup_rect<T>(
    ctx: &Context,
    items: &[PopupItem<T>],
    center: Pos2,
    bounds: Rect,
    m: &Metrics,
) -> Rect {
    geometry(ctx, items, center, bounds, m).0
}

/// [`show`] at sweep step `sweep`, or a message box's items when `on_box` ([`show_on_box`]; phase 0 while hidden).
#[allow(clippy::too_many_arguments)]
fn show_as<T: Copy>(
    ctx: &Context,
    id: Id,
    center: Pos2,
    bounds: Rect,
    items: &[PopupItem<T>],
    cx: &UiCtx,
    sweep: Sweep,
    on_box: bool,
) -> PopupResult<T> {
    let m = cx.m;
    let (rect, cells) = geometry(ctx, items, center, bounds, m);
    let c = &cx.theme.colors;
    // A message box's items are light on its dark box, which takes the place of the popup's fill.
    let (fill, ink) = match on_box {
        false => (c.text.to_color32(), c.background.to_color32()),
        true => (c.background.to_color32(), c.text.to_color32()),
    };
    let band = match on_box {
        false => darken(fill, BAND_SHADE),
        true => ink.gamma_multiply(BOX_BAND_ALPHA),
    };
    let mut result = PopupResult {
        selected: None,
        close: false,
    };

    // Below the popup (`Foreground`) but above the screens (`Background`). Not in the popup's own layer order:
    // egui raises a clicked area to the top of its order, which would put the backdrop over the popup.
    let backdrop = crate::ui::backdrop(ctx, id);
    let escape = crate::ui::close_pressed(ctx);
    result.close |=
        escape || backdrop.clicked() || backdrop.secondary_clicked() || backdrop.middle_clicked();

    // Taller than its bounds even at the smallest rows, or wider (a long list in columns, the SoundFont Player's patch
    // menu): it scrolls that way (drag or wheel), with no scroll bar.
    let scrolls = [
        rect.width() > bounds.width(),
        rect.height() > bounds.height(),
    ];
    // egui keeps a scroll area's offset, so a popup is moved to its current item (the first one) as it opens, centred in
    // the view, and left to the user's scrolling after that.
    let opened = just_opened(ctx, id);
    let current = items.iter().position(|i| i.current).map(|i| cells[i]);
    let mut contents = |ui: &mut egui::Ui| {
        // Inside a scroll area the contents move with the scroll offset.
        let offset = ui.max_rect().min - rect.min;
        let rect = rect.translate(offset);
        // Makes the area as large as the popup, so it also swallows clicks on its disabled cells.
        ui.allocate_exact_size(rect.size(), Sense::hover());
        let painter = ui.painter();
        if let Some(shape) = sweep.fill(rect, m).filter(|_| !on_box) {
            painter.rect_filled(shape, 0.0, fill);
        }
        // Until the sweep is done (or while a message box opens) the items take input but are not drawn.
        let mut painter = painter.clone();
        if sweep.phase < 3 {
            painter.set_invisible();
        }
        for (item, &cell) in items.iter().zip(&cells) {
            let cell = cell.translate(offset);
            if let (Some(value), false) = (item.id, item.disabled) {
                let row_id = id.with((item.col, item.row));
                let response = crate::ui::focus::interact(ui, cell, row_id, Sense::click());
                response.widget_info(|| {
                    egui::WidgetInfo::labeled(egui::WidgetType::Button, true, &item.label)
                });
                if response.hovered() || response.is_pointer_button_down_on() {
                    painter.rect_filled(cell, 0.0, band);
                }
                if response.clicked() {
                    result.selected = Some(value);
                    result.close |= !item.keep_open;
                }
            }
            // The current item inverts: a `background` bar under a `text` label. Only `disabled` dims: the item is
            // drawn at 30 % alpha as a whole, so over the `text` fill an ink label lands at 30 % and a current item's
            // bar at 30 %, with its `text` label unchanged on top. A header is drawn in the full ink.
            let dim = |c: Color32| {
                if item.disabled {
                    c.gamma_multiply(DISABLED_ALPHA)
                } else {
                    c
                }
            };
            let color = if item.current { fill } else { dim(ink) };
            if item.current {
                painter.rect_filled(cell, 0.0, dim(ink));
            }
            painter.text(
                pos2(cell.left() + m.u(PAD_X), cell.center().y),
                Align2::LEFT_CENTER,
                item.label.as_ref(),
                font(item, m),
                color,
            );
            if item.dot {
                let dot = pos2(cell.right() - m.u(PAD_X + DOT_R), cell.center().y);
                painter.circle_filled(dot, m.u(DOT_R), color);
            }
        }
    };
    Area::new(id)
        .order(Order::Foreground)
        .fixed_pos(rect.min)
        .constrain(false)
        .fade_in(false)
        .show(ctx, |ui| {
            if scrolls.contains(&true) {
                // A plain wheel scrolls a popup that scrolls one way only, as shift + wheel would sideways.
                ui.style_mut().always_scroll_the_only_direction = true;
                let mut area = ScrollArea::new(scrolls)
                    .id_salt(id.with("scroll"))
                    .max_width(bounds.width())
                    .max_height(bounds.height())
                    .min_scrolled_width(bounds.width())
                    .min_scrolled_height(bounds.height())
                    .scroll_bar_visibility(ScrollBarVisibility::AlwaysHidden);
                if let (true, Some(cell)) = (opened, current) {
                    let to = |center: f32, content: f32, view: f32| {
                        (center - view / 2.0).clamp(0.0, (content - view).max(0.0))
                    };
                    let center = cell.center() - rect.min;
                    if scrolls[0] {
                        let x = to(center.x, rect.width(), bounds.width());
                        area = area.horizontal_scroll_offset(x);
                    }
                    if scrolls[1] {
                        let y = to(center.y, rect.height(), bounds.height());
                        area = area.vertical_scroll_offset(y);
                    }
                }
                area.show(ui, contents);
            } else {
                contents(ui);
            }
        });
    result
}

/// Where a popup list's opening sweep is in the frame it draws ([`anim::sweep`]): phase 1 (the bar), 2 (the fill) or
/// 3 (done), and a.
#[derive(Clone, Copy, Debug, PartialEq)]
struct Sweep {
    phase: u8,
    a: f32,
}

impl Sweep {
    /// The part of the popup's `rect` filled this frame, on whole device pixels: the bar, the growing fill or all.
    fn fill(&self, rect: Rect, m: &Metrics) -> Option<Rect> {
        let bar = BAR_PX / m.ppp();
        let shape = match self.phase {
            1 => Rect::from_center_size(
                pos2(rect.left() + bar / 2.0, rect.center().y),
                vec2(bar, m.round_px((1.0 - self.a) * rect.height())),
            ),
            2 => rect
                .with_max_x(rect.left() + m.round_px(bar + (1.0 - self.a) * (rect.width() - bar))),
            _ => rect,
        };
        (shape.height() > 0.0).then_some(shape)
    }
}

fn font<T>(item: &PopupItem<T>, m: &Metrics) -> egui::FontId {
    m.font_id(if item.is_header() {
        FontSize::Pt12
    } else {
        FontSize::Pt10
    })
}

fn darken(color: Color32, factor: f32) -> Color32 {
    let ch = |v: u8| (f32::from(v) * factor).round() as u8;
    Color32::from_rgb(ch(color.r()), ch(color.g()), ch(color.b()))
}

/// The popup's screen rect and the rect of each item's cell (in `items` order).
fn geometry<T>(
    ctx: &Context,
    items: &[PopupItem<T>],
    center: Pos2,
    bounds: Rect,
    m: &Metrics,
) -> (Rect, Vec<Rect>) {
    let text_w: Vec<f32> = items
        .iter()
        .map(|item| {
            ctx.fonts_mut(|f| {
                f.layout_no_wrap(item.label.to_string(), font(item, m), Color32::PLACEHOLDER)
                    .size()
                    .x
            })
        })
        .collect();
    let (size, cells) = grid(items, &text_w, bounds.height(), m);
    let rect = place(size, center, bounds, m);
    (
        rect,
        cells
            .iter()
            .map(|c| c.translate(rect.min.to_vec2()))
            .collect(),
    )
}

/// Lays the items out given the width of each label (`text_w`, in `items` order). Returns the popup's size
/// and each item's cell relative to the popup's top-left. Columns are as wide as their widest item, so all
/// items of a column share one width. Rows are 1.25 U, or less when that is taller than `max_h`, the height of the
/// bounds (never below 1 U).
fn grid<T>(items: &[PopupItem<T>], text_w: &[f32], max_h: f32, m: &Metrics) -> (Vec2, Vec<Rect>) {
    let cols = items.iter().map(|i| i.col + 1).max().unwrap_or(0);
    let rows = items.iter().map(|i| i.row + 1).max().unwrap_or(0);
    let mut col_w = vec![0.0f32; cols];
    for (item, &w) in items.iter().zip(text_w) {
        let dot = if item.dot {
            m.u(DOT_GAP + 2.0 * DOT_R)
        } else {
            0.0
        };
        col_w[item.col] = col_w[item.col].max(m.round_px(w + dot + 2.0 * m.u(PAD_X)));
    }
    // Whole device pixels, rounded down when shrinking so that the rows fit.
    let fit = m.floor_px(max_h / rows.max(1) as f32);
    let row_h = m
        .round_px(m.u(ROW_H))
        .min(fit)
        .max(m.round_px(m.u(MIN_ROW_H)));
    let mut col_x = vec![0.0f32; cols];
    for c in 1..cols {
        col_x[c] = col_x[c - 1] + col_w[c - 1];
    }
    let cells = items
        .iter()
        .map(|item| {
            Rect::from_min_size(
                pos2(col_x[item.col], item.row as f32 * row_h),
                vec2(col_w[item.col], row_h),
            )
        })
        .collect();
    (vec2(col_w.iter().sum(), rows as f32 * row_h), cells)
}

/// A rect of `size` centred on `center`, moved to lie inside `bounds` (pinned to its top-left if too large).
fn place(size: Vec2, center: Pos2, bounds: Rect, m: &Metrics) -> Rect {
    let fit = |c: f32, s: f32, lo: f32, hi: f32| m.round_px(c - s / 2.0).min(hi - s).max(lo);
    let min = pos2(
        fit(center.x, size.x, bounds.min.x, bounds.max.x),
        fit(center.y, size.y, bounds.min.y, bounds.max.y),
    );
    Rect::from_min_size(min, size)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::metrics::DEFAULT_UI_SCALE_INDEX;
    use crate::theme::theme_by_id;
    use crate::ui::offscreen::key_tap;
    use eframe::egui::{Event, PointerButton, RawInput};

    /// `U` = 32 points at ppp 1.
    fn metrics() -> Metrics {
        Metrics::from_pixels_per_point(1.0, DEFAULT_UI_SCALE_INDEX)
    }

    fn item(col: usize, row: usize, label: &'static str) -> PopupItem<usize> {
        PopupItem::new(col * 10 + row, col, row, label)
    }

    #[test]
    fn a_column_shares_one_width_and_columns_sit_side_by_side() {
        let m = metrics();
        let items = [item(0, 0, "a"), item(0, 1, "b"), item(1, 0, "c")];
        let (size, cells) = grid(&items, &[90.0, 40.0, 20.0], TALL, &m);
        // Padding is 0.5 U on both sides: 32 points in all. Column 0 follows its widest item (90), listed first.
        assert_eq!(
            cells[0],
            Rect::from_min_size(pos2(0.0, 0.0), vec2(122.0, 40.0))
        );
        assert_eq!(
            cells[1],
            Rect::from_min_size(pos2(0.0, 40.0), vec2(122.0, 40.0))
        );
        assert_eq!(
            cells[2],
            Rect::from_min_size(pos2(122.0, 0.0), vec2(52.0, 40.0))
        );
        assert_eq!(size, vec2(122.0 + 52.0, 80.0));
    }

    #[test]
    fn the_grid_spans_to_the_highest_row_of_any_column() {
        let m = metrics();
        let items = [item(0, 0, "a"), item(1, 3, "b")];
        let (size, _) = grid(&items, &[10.0, 10.0], TALL, &m);
        assert_eq!(size.y, 4.0 * 40.0);
    }

    /// Bounds tall enough for every popup of these tests.
    const TALL: f32 = 600.0;

    /// One column of `rows` items.
    fn column(rows: usize) -> Vec<PopupItem<usize>> {
        (0..rows).map(|r| item(0, r, "x")).collect()
    }

    #[test]
    fn a_popup_too_tall_for_its_bounds_shrinks_its_rows_to_fit() {
        let m = metrics();
        let rows = column(14);
        let w = [10.0; 14];
        // Fits: 1.25 U rows, as always.
        assert_eq!(grid(&rows, &w, 14.0 * 40.0, &m).0.y, 14.0 * 40.0);
        assert_eq!(grid(&rows, &w, TALL, &m).0.y, 14.0 * 40.0);
        // The Project Scale picker's 14 rows in a 540-point window: whole-pixel rows that fit.
        let (size, cells) = grid(&rows, &w, 540.0, &m);
        assert_eq!(cells[1].top(), 38.0);
        assert!(size.y <= 540.0, "{size:?}");
        // At ppp 2 too.
        let m2 = Metrics::from_pixels_per_point(2.0, DEFAULT_UI_SCALE_INDEX);
        let (size, cells) = grid(&rows, &w, 540.0, &m2);
        assert_eq!(cells[1].top(), 38.5);
        assert!(size.y <= 540.0, "{size:?}");
    }

    #[test]
    fn rows_never_shrink_below_one_u() {
        let m = metrics();
        let (size, cells) = grid(&column(30), &[10.0; 30], 540.0, &m);
        assert_eq!(cells[1].top(), 32.0);
        assert_eq!(size.y, 30.0 * 32.0);
    }

    #[test]
    fn a_dot_widens_its_column() {
        let m = metrics();
        let plain = [item(0, 0, "a")];
        let dotted = [PopupItem {
            dot: true,
            ..item(0, 0, "a")
        }];
        let width = |items: &[PopupItem<usize>]| grid(items, &[50.0], TALL, &m).0.x;
        // Gap 0.25 U + diameter 0.2 U = 14.4 points.
        assert!((width(&dotted) - width(&plain) - 14.0).abs() <= 1.0);
    }

    #[test]
    fn placement_centres_then_clamps_inside_the_bounds() {
        let bounds = Rect::from_min_size(pos2(0.0, 0.0), vec2(1000.0, 500.0));
        let size = vec2(200.0, 100.0);
        let at = |c: Pos2| place(size, c, bounds, &metrics());
        assert_eq!(at(pos2(500.0, 250.0)).center(), pos2(500.0, 250.0));
        assert_eq!(at(pos2(10.0, 250.0)).min, pos2(0.0, 200.0));
        assert_eq!(at(pos2(990.0, 250.0)).max, pos2(1000.0, 300.0));
        assert_eq!(at(pos2(500.0, 0.0)).min, pos2(400.0, 0.0));
        assert_eq!(at(pos2(500.0, 499.0)).max, pos2(600.0, 500.0));
        // Too large for the bounds: pinned to the top-left.
        assert_eq!(
            place(vec2(2000.0, 900.0), pos2(500.0, 250.0), bounds, &metrics()).min,
            pos2(0.0, 0.0)
        );
    }

    // Offscreen egui: the popup in a real context, driven by synthetic pointer events.
    const MENU_ID: &str = "test_popup";

    struct Rig {
        ctx: Context,
        items: Vec<PopupItem<usize>>,
        pointer: Pos2,
        time: f64,
    }

    const BOUNDS: Rect = Rect {
        min: pos2(0.0, 0.0),
        max: pos2(800.0, 600.0),
    };
    const CENTER: Pos2 = pos2(400.0, 300.0);

    impl Rig {
        fn new(items: Vec<PopupItem<usize>>) -> Self {
            let mut rig = Self {
                ctx: Context::default(),
                items,
                pointer: Pos2::ZERO,
                time: 0.0,
            };
            // The first frames size the area; settle before any input. (The opening sweep is off in unit tests unless a
            // test enables it, as `sweep_tests` do.)
            for _ in 0..3 {
                rig.frame(vec![]);
            }
            rig
        }

        fn frame(&mut self, events: Vec<Event>) -> PopupResult<usize> {
            self.frame_with_shapes(events).0
        }

        fn frame_with_shapes(
            &mut self,
            events: Vec<Event>,
        ) -> (PopupResult<usize>, Vec<egui::epaint::ClippedShape>) {
            self.time += 0.1;
            let raw = RawInput {
                screen_rect: Some(BOUNDS),
                time: Some(self.time),
                events,
                ..Default::default()
            };
            let (theme, m) = (theme_by_id(0), metrics());
            let cx = UiCtx {
                theme: &theme,
                m: &m,
            };
            let mut result = None;
            let output = self.ctx.run(raw, |ctx| {
                result = Some(show(
                    ctx,
                    Id::new(MENU_ID),
                    CENTER,
                    BOUNDS,
                    &self.items,
                    &cx,
                ));
            });
            (result.unwrap(), output.shapes)
        }

        /// The colour each label was painted in, by label.
        fn label_colors(&mut self) -> Vec<(String, Color32)> {
            let (_, shapes) = self.frame_with_shapes(vec![]);
            shapes
                .into_iter()
                .filter_map(|clipped| match clipped.shape {
                    egui::Shape::Text(text) => Some((
                        text.galley.text().to_owned(),
                        text.galley.job.sections[0].format.color,
                    )),
                    _ => None,
                })
                .collect()
        }

        /// Centre of the cell at (`col`, `row`).
        fn cell(&self, col: usize, row: usize) -> Pos2 {
            self.cell_rect(col, row).center()
        }

        fn cell_rect(&self, col: usize, row: usize) -> Rect {
            let i = self
                .items
                .iter()
                .position(|it| it.col == col && it.row == row)
                .unwrap();
            let (_, cells) = geometry(&self.ctx, &self.items, CENTER, BOUNDS, &metrics());
            cells[i]
        }

        /// Moves to `pos`, presses and releases; returns the result of the release frame.
        fn click(&mut self, pos: Pos2) -> PopupResult<usize> {
            self.click_with(pos, PointerButton::Primary)
        }

        fn click_with(&mut self, pos: Pos2, button: PointerButton) -> PopupResult<usize> {
            let button = |pressed| Event::PointerButton {
                pos,
                button,
                pressed,
                modifiers: Default::default(),
            };
            self.frame(vec![Event::PointerMoved(pos)]);
            self.pointer = pos;
            self.frame(vec![button(true)]);
            self.frame(vec![button(false)])
        }
    }

    fn menu() -> Vec<PopupItem<usize>> {
        vec![
            PopupItem::header(0, 0, "Head:"),
            item(0, 1, "Plain"),
            PopupItem {
                disabled: true,
                ..item(0, 2, "Off")
            },
            PopupItem {
                keep_open: true,
                ..item(0, 3, "Stay")
            },
            item(1, 1, "Second"),
        ]
    }

    #[test]
    fn selecting_an_item_returns_its_id_and_closes() {
        let mut rig = Rig::new(menu());
        let at = rig.cell(0, 1);
        assert_eq!(
            rig.click(at),
            PopupResult {
                selected: Some(1),
                close: true
            }
        );
        let at = rig.cell(1, 1);
        assert_eq!(
            rig.click(at),
            PopupResult {
                selected: Some(11),
                close: true
            }
        );
    }

    #[test]
    fn a_keep_open_item_selects_without_closing() {
        let mut rig = Rig::new(menu());
        let at = rig.cell(0, 3);
        assert_eq!(
            rig.click(at),
            PopupResult {
                selected: Some(3),
                close: false
            }
        );
    }

    #[test]
    fn headers_and_disabled_items_do_nothing() {
        let mut rig = Rig::new(menu());
        for (col, row) in [(0, 0), (0, 2)] {
            let at = rig.cell(col, row);
            assert_eq!(
                rig.click(at),
                PopupResult {
                    selected: None,
                    close: false
                },
                "({col}, {row})"
            );
        }
    }

    #[test]
    fn only_disabled_items_are_dimmed_headers_are_not() {
        let ink = theme_by_id(0).colors.background.to_color32();
        let mut rig = Rig::new(vec![
            PopupItem::header(0, 0, "Header:"),
            item(0, 1, "Plain"),
            PopupItem {
                disabled: true,
                ..item(0, 2, "Off")
            },
            // A header that is also disabled, as the Project Scale picker's are.
            PopupItem {
                disabled: true,
                ..PopupItem::header(1, 0, "Both:")
            },
        ]);
        let colors = rig.label_colors();
        let color = |label: &str| colors.iter().find(|(l, _)| l == label).unwrap().1;
        let alpha = |label: &str| f32::from(color(label).a()) / 255.0;
        // Header and plain item: the full ink, not dimmed.
        assert_eq!(color("Header:"), ink);
        assert_eq!(color("Plain"), ink);
        // Disabled, and header + disabled: 30 % alpha.
        assert!((alpha("Off") - 0.3).abs() < 0.01, "{}", alpha("Off"));
        assert!((alpha("Both:") - 0.3).abs() < 0.01, "{}", alpha("Both:"));
    }

    #[test]
    fn a_click_outside_closes() {
        let mut rig = Rig::new(menu());
        assert_eq!(
            rig.click(pos2(5.0, 5.0)),
            PopupResult {
                selected: None,
                close: true
            }
        );
    }

    #[test]
    fn a_secondary_or_middle_click_outside_closes_too() {
        for button in [PointerButton::Secondary, PointerButton::Middle] {
            let mut rig = Rig::new(menu());
            assert_eq!(
                rig.click_with(pos2(5.0, 5.0), button),
                PopupResult {
                    selected: None,
                    close: true
                },
                "{button:?}"
            );
        }
    }

    #[test]
    fn the_popup_appears_at_full_strength_without_a_fade() {
        // `fade_in(false)`: from its first visible frame the popup is opaque (an egui fade starts near 0 and lowers
        // every shape's opacity).
        let fill = theme_by_id(0).colors.text.to_color32();
        let mut rig = Rig {
            ctx: Context::default(),
            items: menu(),
            pointer: Pos2::ZERO,
            time: 0.0,
        };
        let mut seen = 0;
        for _ in 0..3 {
            // Frames 10 ms apart, well inside egui's fade time (the rig steps 100 ms).
            rig.time -= 0.09;
            let (_, shapes) = rig.frame_with_shapes(vec![]);
            for clipped in &shapes {
                match &clipped.shape {
                    egui::Shape::Text(t) => {
                        assert_eq!(t.opacity_factor, 1.0, "{}", t.galley.text())
                    }
                    egui::Shape::Rect(r) if r.rect.size() == rig.cell_rect(0, 0).size() => {}
                    // The screen's dim.
                    egui::Shape::Rect(r) if r.rect == BOUNDS => {}
                    egui::Shape::Rect(r) => {
                        assert_eq!(r.fill, fill);
                        seen += 1;
                    }
                    _ => {}
                }
            }
        }
        assert!(seen >= 2, "{seen}");
    }

    /// One frame with up to two popups open, `inner` shown first, then `outer`, over a red screen rect in the
    /// background layer. Returns each shown popup's result and what was painted.
    fn nested_frame(
        ctx: &Context,
        time: f64,
        events: Vec<Event>,
        inner: bool,
        outer: bool,
    ) -> [Option<PopupResult<usize>>; 2] {
        let raw = RawInput {
            screen_rect: Some(BOUNDS),
            time: Some(time),
            events,
            ..Default::default()
        };
        let (theme, m) = (theme_by_id(0), metrics());
        let cx = UiCtx {
            theme: &theme,
            m: &m,
        };
        let mut results = [None, None];
        let _ = ctx.run(raw, |ctx| {
            egui::CentralPanel::default().show(ctx, |ui| {
                ui.painter().rect_filled(BOUNDS, 0.0, Color32::RED);
            });
            if inner {
                let items = [item(0, 0, "Inner")];
                results[0] = Some(show(ctx, Id::new("inner"), INNER_AT, BOUNDS, &items, &cx));
            }
            if outer {
                let items = [item(0, 0, "Outer")];
                results[1] = Some(show(ctx, Id::new("outer"), CENTER, BOUNDS, &items, &cx));
            }
        });
        results
    }

    const INNER_AT: Pos2 = pos2(400.0, 330.0);

    #[test]
    fn a_nested_popup_shown_before_its_parent_paints_on_top_and_takes_escape_first() {
        // `show` consumes Escape, so the first popup shown in a frame takes it. A popup opened over another is shown
        // before it (as the main menu draws its tabs before its own Escape check), and stays on top: egui stacks
        // areas by when they appeared, not by the order of the calls.
        let ctx = Context::default();
        let mut time = 0.0;
        let mut frame = |events: Vec<Event>, inner: bool, outer: bool| {
            time += 0.1;
            nested_frame(&ctx, time, events, inner, outer)
        };
        for _ in 0..3 {
            frame(vec![], false, true);
        }
        for _ in 0..3 {
            frame(vec![], true, true);
        }
        // Paint order: the screen, then the outer popup, then the inner one.
        let raw = RawInput {
            screen_rect: Some(BOUNDS),
            time: Some(10.0),
            ..Default::default()
        };
        let (theme, m) = (theme_by_id(0), metrics());
        let cx = UiCtx {
            theme: &theme,
            m: &m,
        };
        let shapes = ctx
            .run(raw, |ctx| {
                egui::CentralPanel::default().show(ctx, |ui| {
                    ui.painter().rect_filled(BOUNDS, 0.0, Color32::RED);
                });
                show(
                    ctx,
                    Id::new("inner"),
                    INNER_AT,
                    BOUNDS,
                    &[item(0, 0, "Inner")],
                    &cx,
                );
                show(
                    ctx,
                    Id::new("outer"),
                    CENTER,
                    BOUNDS,
                    &[item(0, 0, "Outer")],
                    &cx,
                );
            })
            .shapes;
        let index = |label: &str| {
            shapes
                .iter()
                .position(|c| matches!(&c.shape, egui::Shape::Text(t) if t.galley.text() == label))
                .unwrap()
        };
        let screen = shapes
            .iter()
            .position(|c| matches!(&c.shape, egui::Shape::Rect(r) if r.fill == Color32::RED))
            .unwrap();
        assert!(screen < index("Outer") && index("Outer") < index("Inner"));

        let escape = || key_tap(egui::Key::Escape, Default::default());
        let closed = |r: Option<PopupResult<usize>>| r.unwrap().close;
        // One press closes the inner popup only; the next one the outer.
        let [inner, outer] = frame(escape(), true, true);
        assert!(closed(inner) && !closed(outer));
        let [_, outer] = frame(escape(), false, true);
        assert!(closed(outer));
    }

    #[test]
    fn a_held_escape_closes_the_inner_popup_only_and_its_repeats_close_nothing() {
        // A press of a key already held is ignored.
        let ctx = Context::default();
        let mut time = 0.0;
        let mut frame = |events: Vec<Event>, inner: bool| {
            time += 0.1;
            nested_frame(&ctx, time, events, inner, true)
        };
        for inner in [false, false, false, true, true, true] {
            frame(vec![], inner);
        }
        let [press, release] =
            <[Event; 2]>::try_from(key_tap(egui::Key::Escape, Default::default())).unwrap();
        let closed = |r: Option<PopupResult<usize>>| r.unwrap().close;
        let [inner, outer] = frame(vec![press.clone()], true);
        assert!(closed(inner) && !closed(outer));
        // Still held: egui marks the OS's auto-repeats so, and the outer popup stays.
        for _ in 0..3 {
            let [_, outer] = frame(vec![press.clone()], false);
            assert!(!closed(outer));
        }
        let [_, outer] = frame(vec![release], false);
        assert!(!closed(outer));
        let [_, outer] = frame(vec![press], false);
        assert!(closed(outer));
    }

    #[test]
    fn escape_closes() {
        let mut rig = Rig::new(menu());
        let escape = |pressed| Event::Key {
            key: egui::Key::Escape,
            physical_key: None,
            pressed,
            repeat: false,
            modifiers: Default::default(),
        };
        assert_eq!(
            rig.frame(vec![escape(true)]),
            PopupResult {
                selected: None,
                close: true
            }
        );
    }

    #[test]
    fn the_current_item_is_a_background_bar_with_a_text_label() {
        let colors = theme_by_id(0).colors;
        let (text, bg) = (colors.text.to_color32(), colors.background.to_color32());
        let mut rig = Rig::new(vec![
            item(0, 0, "Plain"),
            PopupItem {
                current: true,
                ..item(0, 1, "Now")
            },
            PopupItem {
                current: true,
                disabled: true,
                ..item(0, 2, "Now off")
            },
        ]);
        let (_, shapes) = rig.frame_with_shapes(vec![]);
        let bars = |fill: Color32| -> Vec<Rect> {
            shapes
                .iter()
                .filter_map(|clipped| match &clipped.shape {
                    egui::Shape::Rect(r) if r.fill == fill => Some(r.rect),
                    _ => None,
                })
                .collect()
        };
        assert_eq!(bars(bg), [rig.cell_rect(0, 1)]);
        // A disabled item is drawn at 30 % alpha, its bar included.
        assert_eq!(bars(bg.gamma_multiply(0.3)), [rig.cell_rect(0, 2)]);

        let colors = rig.label_colors();
        let color = |label: &str| colors.iter().find(|(l, _)| l == label).unwrap().1;
        assert_eq!(color("Plain"), bg);
        assert_eq!(color("Now"), text);
        // Disabled and current: the item is drawn at 30 % as a whole, so the dimmed bar keeps a plain `text` label,
        // the colour the composited label would land on (it stays readable).
        assert_eq!(color("Now off"), text);
    }

    #[test]
    fn a_popup_taller_than_the_screen_scrolls() {
        // 30 rows at the 1 U floor: 960 points in a 600-point screen.
        let mut rig = Rig::new(column(30));
        let (_, shapes) = rig.frame_with_shapes(vec![]);
        // Everything is clipped to the screen, and the popup uses all of its height.
        for s in &shapes {
            assert_eq!(s.clip_rect.y_range(), BOUNDS.y_range());
        }
        // The bottom of the screen shows an early row; after scrolling down, a late one.
        let bottom = pos2(CENTER.x, BOUNDS.bottom() - 10.0);
        rig.frame(vec![Event::PointerMoved(bottom)]);
        for _ in 0..20 {
            rig.frame(vec![Event::MouseWheel {
                unit: egui::MouseWheelUnit::Point,
                delta: vec2(0.0, -200.0),
                modifiers: Default::default(),
            }]);
        }
        for _ in 0..10 {
            rig.frame(vec![]);
        }
        assert_eq!(rig.click(bottom).selected, Some(29));
    }

    /// 40 one-row columns of about 50 points: 2000 points in an 800-point screen.
    fn wide() -> Rig {
        Rig::new((0..40).map(|c| item(c, 0, "x")).collect())
    }

    /// The row the popup's one row is on, and a point on the screen's right edge on it.
    fn right_edge() -> Pos2 {
        pos2(BOUNDS.right() - 10.0, CENTER.y)
    }

    #[test]
    fn a_popup_wider_than_the_screen_scrolls_sideways_by_the_wheel() {
        // The SoundFont Player's patch menu: a bank of 128 patches is 11 columns of 12, wider than the screen. The
        // popup is pinned to the left edge, its width the screen's.
        let mut rig = wide();
        let (_, shapes) = rig.frame_with_shapes(vec![]);
        for s in &shapes {
            assert_eq!(s.clip_rect.x_range(), BOUNDS.x_range());
        }
        // The right edge shows an early column; after the wheel, the last (a plain wheel: the only direction).
        let right = right_edge();
        rig.frame(vec![Event::PointerMoved(right)]);
        for _ in 0..30 {
            rig.frame(vec![Event::MouseWheel {
                unit: egui::MouseWheelUnit::Point,
                delta: vec2(0.0, -200.0),
                modifiers: Default::default(),
            }]);
        }
        for _ in 0..10 {
            rig.frame(vec![]);
        }
        assert_eq!(rig.click(right).selected, Some(390));
    }

    #[test]
    fn a_popup_wider_than_the_screen_scrolls_sideways_by_a_drag() {
        let mut rig = wide();
        // A tap on the early column at the right edge selects it; dragging left instead scrolls and selects nothing.
        let right = right_edge();
        let button = |pos, pressed| Event::PointerButton {
            pos,
            button: PointerButton::Primary,
            pressed,
            modifiers: Default::default(),
        };
        rig.frame(vec![Event::PointerMoved(right)]);
        rig.frame(vec![button(right, true)]);
        let mut at = right;
        for _ in 0..30 {
            at.x -= 40.0;
            rig.frame(vec![Event::PointerMoved(at)]);
        }
        let released = rig.frame(vec![button(at, false)]);
        assert_eq!(released.selected, None);
        for _ in 0..10 {
            rig.frame(vec![]);
        }
        assert_eq!(rig.click(right).selected, Some(390));
    }

    /// Where the last frame painted `text`.
    fn text_rect(shapes: &[egui::epaint::ClippedShape], text: &str) -> Rect {
        let found = shapes.iter().find_map(|c| match &c.shape {
            egui::Shape::Text(t) if t.galley.text() == text => {
                Some(t.galley.rect.translate(t.pos.to_vec2()))
            }
            _ => None,
        });
        found.unwrap_or_else(|| panic!("{text:?} not drawn"))
    }

    #[test]
    fn a_scrolling_popup_opens_with_its_current_item_in_view_each_time_it_opens() {
        // 40 columns; the current item is far to the right. egui keeps a scroll area's offset, so a popup opened again
        // would stay where it was left: it scrolls to the current item whenever it opens, and only then.
        let items = (0..40).map(|c| PopupItem {
            current: c == 35,
            ..PopupItem::new(c, c, 0, format!("c{c}"))
        });
        let mut rig = Rig::new(items.collect());
        let in_view = |rig: &mut Rig, text: &str| {
            let (_, shapes) = rig.frame_with_shapes(vec![]);
            let r = text_rect(&shapes, text);
            r.left() >= BOUNDS.left() && r.right() <= BOUNDS.right()
        };
        assert!(in_view(&mut rig, "c35") && !in_view(&mut rig, "c0"));
        // Scrolled away by the wheel, it stays there while open.
        rig.frame(vec![Event::PointerMoved(CENTER)]);
        for _ in 0..40 {
            rig.frame(vec![Event::MouseWheel {
                unit: egui::MouseWheelUnit::Point,
                delta: vec2(0.0, 300.0),
                modifiers: Default::default(),
            }]);
        }
        assert!(in_view(&mut rig, "c0") && !in_view(&mut rig, "c35"));
        // Closed (not shown for a few frames) and opened again: the current item is back in view.
        for _ in 0..3 {
            let _ = rig.ctx.run(RawInput::default(), |_| {});
        }
        assert!(in_view(&mut rig, "c35") && !in_view(&mut rig, "c0"));
    }

    #[test]
    fn labels_may_be_owned() {
        let theme_names = ["Classic Gray".to_owned(), "Slate".to_owned()];
        let mut rig = Rig::new(
            theme_names
                .iter()
                .enumerate()
                .map(|(i, name)| PopupItem::new(i, 0, i, name.clone()))
                .collect(),
        );
        let at = rig.cell(0, 1);
        assert_eq!(rig.click(at).selected, Some(1));
    }
}

#[cfg(test)]
mod sweep_tests {
    //! The menu opening animation of popup lists, frame by frame.
    use super::*;
    use crate::metrics::DEFAULT_UI_SCALE_INDEX;
    use crate::theme::theme_by_id;
    use eframe::egui::{Event, PointerButton, RawInput, Shape, epaint::ClippedShape};

    const SCREEN: Rect = Rect {
        min: pos2(0.0, 0.0),
        max: pos2(800.0, 600.0),
    };
    const CENTER: Pos2 = pos2(400.0, 300.0);
    const ID: &str = "sweep";

    fn items() -> Vec<PopupItem<usize>> {
        ["Load Sample", "Clone", "Delete"]
            .into_iter()
            .enumerate()
            .map(|(row, label)| PopupItem::new(row, 0, row, label))
            .collect()
    }

    /// The popup shown for `frames` drawn frames at ppp `ppp` (each 1/60 s on), with `events` in the last. Returns
    /// the last frame's result, shapes and whether it asked for another frame at once.
    struct Rig {
        ctx: Context,
        ppp: f32,
        time: f64,
    }

    impl Rig {
        fn new(ppp: f32) -> Self {
            let ctx = Context::default();
            anim::enable(&ctx);
            Self {
                ctx,
                ppp,
                time: 0.0,
            }
        }

        fn frame(
            &mut self,
            show: bool,
            events: Vec<Event>,
        ) -> (PopupResult<usize>, Vec<ClippedShape>, bool) {
            self.time += 1.0 / 60.0;
            let mut raw = RawInput {
                screen_rect: Some(SCREEN),
                time: Some(self.time),
                events,
                ..Default::default()
            };
            raw.viewports
                .entry(raw.viewport_id)
                .or_default()
                .native_pixels_per_point = Some(self.ppp);
            let theme = theme_by_id(0);
            let m = Metrics::from_pixels_per_point(self.ppp, DEFAULT_UI_SCALE_INDEX);
            let cx = UiCtx {
                theme: &theme,
                m: &m,
            };
            let mut result = PopupResult {
                selected: None,
                close: false,
            };
            let output = self.ctx.run(raw, |ctx| {
                if show {
                    result = super::show(ctx, Id::new(ID), CENTER, SCREEN, &items(), &cx);
                }
            });
            let now = output.viewport_output[&egui::ViewportId::ROOT]
                .repaint_delay
                .is_zero();
            (result, output.shapes, now)
        }

        /// The popup's full rect.
        fn rect(&self) -> Rect {
            let m = Metrics::from_pixels_per_point(self.ppp, DEFAULT_UI_SCALE_INDEX);
            geometry(&self.ctx, &items(), CENTER, SCREEN, &m).0
        }
    }

    fn fill() -> Color32 {
        theme_by_id(0).colors.text.to_color32()
    }

    /// The `text`-filled rects painted (the popup's fill, bar or band).
    fn fills(shapes: &[ClippedShape]) -> Vec<Rect> {
        (shapes.iter())
            .filter_map(|c| match &c.shape {
                Shape::Rect(r) if r.fill == fill() => Some(r.rect),
                _ => None,
            })
            .collect()
    }

    fn texts(shapes: &[ClippedShape]) -> usize {
        (shapes.iter())
            .filter(|c| matches!(&c.shape, Shape::Text(_)))
            .count()
    }

    fn near(a: f32, b: f32) -> bool {
        (a - b).abs() <= 0.51
    }

    #[test]
    fn the_screen_dims_from_the_first_frame_and_the_bar_then_the_fill_sweep_open_in_11_frames_each()
    {
        for ppp in [1.0, 2.0] {
            let mut rig = Rig::new(ppp);
            let px = 1.0 / ppp;
            let dim = theme_by_id(0)
                .colors
                .background
                .to_color32()
                .gamma_multiply(0.6);
            for n in 1..=24 {
                let (_, shapes, again) = rig.frame(true, vec![]);
                let rect = rig.rect();
                // The dim, every frame, over the whole screen and under the popup.
                let dims: Vec<usize> = (shapes.iter().enumerate())
                    .filter(|(_, c)| matches!(&c.shape, Shape::Rect(r) if r.fill == dim && r.rect == SCREEN))
                    .map(|(i, _)| i)
                    .collect();
                assert_eq!(dims.len(), 1, "ppp {ppp} frame {n}");
                let fills = fills(&shapes);
                assert!(fills.len() <= 1, "{fills:?}");
                let k = |first: i32| 0.4f32.powi(n - first);
                match n {
                    1..=11 => {
                        // A 2 px bar at the left edge, (1 − a)·H tall, centred on the popup.
                        let h = (1.0 - k(1)) * rect.height();
                        match fills.first() {
                            None => assert!(h < px, "frame {n}"),
                            Some(bar) => {
                                assert!(near(bar.width() * ppp, 2.0), "frame {n}: {bar:?}");
                                assert_eq!(bar.left(), rect.left());
                                assert!(
                                    near(bar.height() * ppp, h * ppp),
                                    "frame {n}: {bar:?} {h}"
                                );
                                assert!(
                                    (bar.center().y - rect.center().y).abs() <= px,
                                    "frame {n}"
                                );
                            }
                        }
                    }
                    12..=22 => {
                        let band = fills[0];
                        let w = 2.0 * px + (1.0 - k(12)) * (rect.width() - 2.0 * px);
                        assert!(near(band.width() * ppp, w * ppp), "frame {n}: {band:?} {w}");
                        assert_eq!((band.left(), band.y_range()), (rect.left(), rect.y_range()));
                    }
                    _ => assert_eq!(fills, [rect], "frame {n}"),
                }
                // The items show from frame 23; the animation asks for frames until then (frame 23 lays out new glyphs,
                // which asks for one more frame of its own).
                assert_eq!(texts(&shapes) > 0, n >= 23, "frame {n}");
                if n != 23 {
                    assert_eq!(again, n < 23, "frame {n}");
                }
            }
        }
    }

    #[test]
    fn an_item_can_be_picked_before_it_is_drawn_and_a_click_outside_closes_at_once() {
        let mut rig = Rig::new(1.0);
        rig.frame(true, vec![]);
        rig.frame(true, vec![]);
        let m = Metrics::from_pixels_per_point(1.0, DEFAULT_UI_SCALE_INDEX);
        let cell = geometry(&rig.ctx, &items(), CENTER, SCREEN, &m).1[1].center();
        let button = |pressed| Event::PointerButton {
            pos: cell,
            button: PointerButton::Primary,
            pressed,
            modifiers: Default::default(),
        };
        rig.frame(true, vec![Event::PointerMoved(cell)]);
        rig.frame(true, vec![button(true)]);
        let (result, ..) = rig.frame(true, vec![button(false)]);
        assert_eq!(result.selected, Some(1), "frame 5");
        // Reopened: from the start again. A click outside on its second frame closes it.
        rig.frame(false, vec![]);
        rig.frame(true, vec![]);
        let out = pos2(5.0, 5.0);
        let at = |pressed| Event::PointerButton {
            pos: out,
            button: PointerButton::Primary,
            pressed,
            modifiers: Default::default(),
        };
        rig.frame(true, vec![Event::PointerMoved(out)]);
        rig.frame(true, vec![at(true)]);
        let (result, shapes, _) = rig.frame(true, vec![at(false)]);
        assert!(result.close);
        assert_eq!(texts(&shapes), 0, "still opening");
    }
}
