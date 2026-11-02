//! The spinner: a caption, the current item's text and a `^` and a `v` chevron. A tap on the text opens the inverted
//! popup of items with the current one marked; a tap on a chevron steps to the previous or next item.
//!
//! `^` is the item above in the popup list (the previous one), `v` the next, and neither wraps. The chevron zones are
//! the body above and below the value text, full width, which leaves the caption and the text to the popup.
//!
//! The popup can run over several columns ([`Control::column_len`], the Arpeggiator's 3 × 8) and show items that
//! cannot be picked ([`Control::disabled_items`], the Scale's `Custom`); the chevrons skip those.

use eframe::egui::{
    Align2, Id, Pos2, Rect, Response, Sense, Shape, Stroke, Ui, WidgetInfo, WidgetType, pos2,
};

use super::{Control, close_popup, popup_anchor, through_index};
use crate::metrics::FontSize;
use crate::ui::popup::{self, PopupItem};
use crate::ui::{UiCtx, focus};

/// The value text is FontSize 2.
const VALUE_FONT: FontSize = FontSize::Pt10;
/// Distance of a chevron's apex from the value centre, in U.
const APEX: f32 = 0.85;
/// A chevron's arms reach this far sideways and back from the apex, in t (4 t by 4 t).
const ARM: f32 = 4.0;
/// Taps farther than this from the value centre, in U, are chevron taps: halfway between the text's edge and the
/// chevron's inner end.
const TEXT_ZONE: f32 = 0.5;

/// Which chevron: `^` above the value or `v` below it.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Chevron {
    Up,
    Down,
}

/// The chevron's three points (arm, apex, arm) for the value centre `center`: the apex is 0.85 U above (`^`) or
/// below (`v`) it and the arms go 4 t sideways and 4 t back towards the centre.
fn chevron(center: Pos2, which: Chevron, u: f32, t: f32) -> [Pos2; 3] {
    let arm = ARM * t;
    let (apex_y, arm_y) = match which {
        Chevron::Up => (center.y - APEX * u, center.y - APEX * u + arm),
        Chevron::Down => (center.y + APEX * u, center.y + APEX * u - arm),
    };
    [
        pos2(center.x - arm, arm_y),
        pos2(center.x, apex_y),
        pos2(center.x + arm, arm_y),
    ]
}

/// The tap zones of `^` and `v`: the parts of `body` more than half a U above and below its centre.
fn zones(body: Rect, u: f32) -> (Rect, Rect) {
    let (above, below) = (
        body.center().y - TEXT_ZONE * u,
        body.center().y + TEXT_ZONE * u,
    );
    (
        body.with_max_y(above.clamp(body.top(), body.bottom())),
        body.with_min_y(below.clamp(body.top(), body.bottom())),
    )
}

/// The id of spinner `id`'s item popup.
pub(super) fn popup_id(id: Id) -> Id {
    id.with("item popup")
}

/// Opens control `id`'s item popup, centred on `at`.
pub(super) fn open_popup(ui: &Ui, id: Id, at: Pos2) {
    super::open_popup(ui, popup_id(id), at);
}

/// Shows control `c`'s item popup while it is open: `items` in `c.column_len` rows per column, `current` marked
/// and `c.disabled_items` dimmed. Returns the item picked this frame; the popup closes on a pick, on a tap outside
/// it, and when there are no items.
pub(super) fn item_popup(
    ui: &Ui,
    cx: &UiCtx,
    c: &Control,
    items: &[&str],
    current: usize,
) -> Option<usize> {
    grid_popup(ui, cx, c.id, || {
        popup_items(items, current, c.column_len, c.disabled_items)
    })
}

/// [`item_popup`] with the popup's items laid out by the caller; `rows` is only called while the popup is open.
fn grid_popup(
    ui: &Ui,
    cx: &UiCtx,
    id: Id,
    rows: impl FnOnce() -> Vec<PopupItem<usize>>,
) -> Option<usize> {
    let menu_id = popup_id(id);
    let at = popup_anchor(ui, menu_id)?;
    let ctx = ui.ctx();
    let rows = rows();
    let result = popup::show(ctx, menu_id, at, ctx.content_rect(), &rows, cx);
    if result.close || rows.is_empty() {
        close_popup(ui, menu_id);
    }
    result.selected
}

/// What a spinner or menu button tells accessibility tools: a combo box named by its caption, showing `item`.
pub(super) fn combo_info(c: &Control, item: &str) -> WidgetInfo {
    WidgetInfo {
        current_text_value: Some(item.to_owned()),
        ..WidgetInfo::labeled(WidgetType::ComboBox, !c.disabled, c.label)
    }
}

/// The item index after tapping `which` on a list of `len` items: the previous or the next one that is not
/// `disabled`, and the end stays.
fn step(index: usize, len: usize, which: Chevron, disabled: &[usize]) -> usize {
    let index = index.min(len.saturating_sub(1));
    let pickable = |i: &usize| !disabled.contains(i);
    let next = match which {
        Chevron::Up => (0..index).rev().find(pickable),
        Chevron::Down => (index + 1..len).find(pickable),
    };
    next.unwrap_or(index)
}

/// The popup's items: one per label, `column_len` to a column (all in one when `None`), the current one flagged and
/// the `disabled` ones dimmed. Each carries its own index, which is what picking it selects.
fn popup_items(
    items: &[&str],
    current: usize,
    column_len: Option<usize>,
    disabled: &[usize],
) -> Vec<PopupItem<usize>> {
    let len = column_len.unwrap_or(items.len()).max(1);
    items
        .iter()
        .enumerate()
        .map(|(i, &label)| PopupItem {
            current: i == current,
            disabled: disabled.contains(&i),
            ..PopupItem::new(i, i / len, i % len, label.to_owned())
        })
        .collect()
}

/// A spinner bound to a menu parameter: `value` is its normalised value, `items` the menu's labels.
pub fn spinner(ui: &mut Ui, cx: &UiCtx, c: &Control, items: &[&str], value: &mut f32) -> Response {
    through_index(value, items.len(), |index| {
        spinner_index(ui, cx, c, items, index)
    })
}

/// A spinner that is not a parameter: it edits the index of the current item. Only `c.id`, `c.rect`, `c.label`,
/// `c.label_font`, `c.channel`, `c.disabled`, `c.value_text` (it reserves the bottom strip), and the popup's and
/// chevrons' `c.column_len` and `c.disabled_items` matter. An index past the end reads as the last item, and is only
/// replaced when the user picks one.
pub fn spinner_index(
    ui: &mut Ui,
    cx: &UiCtx,
    c: &Control,
    items: &[&str],
    index: &mut usize,
) -> Response {
    spinner_grid(ui, cx, c, items, index, |current| {
        popup_items(items, current, c.column_len, c.disabled_items)
    })
}

/// A [`spinner_index`] whose popup the caller lays out instead of one column of `items`: for pickers with columns and
/// headers, such as `Project Scale`. `grid` builds the popup's items for the current index, only while the popup is
/// open; each item's id is an index into `items`, which the chevrons step through in order.
pub fn spinner_grid(
    ui: &mut Ui,
    cx: &UiCtx,
    c: &Control,
    items: &[&str],
    index: &mut usize,
    grid: impl FnOnce(usize) -> Vec<PopupItem<usize>>,
) -> Response {
    let (m, n) = (cx.m, items.len());
    let current = (*index).min(n.saturating_sub(1));
    let sense = if c.disabled || n == 0 {
        Sense::hover()
    } else {
        Sense::click()
    };
    let mut response = focus::interact(ui, c.rect, c.id, sense);
    let (_, body, _) = c.strips(m);
    let mut picked = None;

    // The tap decides first: a chevron steps, anything else opens the popup, centred on the control.
    if response.clicked() {
        let at = response.interact_pointer_pos().unwrap_or(c.rect.center());
        let (up, down) = zones(body, m.u(1.0));
        if up.contains(at) {
            picked = Some(step(current, n, Chevron::Up, c.disabled_items));
        } else if down.contains(at) {
            picked = Some(step(current, n, Chevron::Down, c.disabled_items));
        } else {
            open_popup(ui, c.id, c.rect.center());
        }
    }
    // Act on what was picked, not on the fact that something was.
    picked = grid_popup(ui, cx, c.id, || grid(current)).or(picked);
    let current = match picked {
        Some(i) if i != current => {
            *index = i;
            response.mark_changed();
            i
        }
        _ => current,
    };
    let item = items.get(current).copied().unwrap_or_default();
    response.widget_info(|| combo_info(c, item));
    paint(ui, cx, c, item);
    response
}

/// A control that looks like a spinner showing `item` and is a button: a tap anywhere on it is the caller's (the
/// response's click), with no popup and no step. The SoundFont Player's `Patch`, which opens a picker.
pub fn spinner_button(ui: &mut Ui, cx: &UiCtx, c: &Control, item: &str) -> Response {
    let sense = if c.disabled {
        Sense::hover()
    } else {
        Sense::click()
    };
    let response = focus::interact(ui, c.rect, c.id, sense);
    response.widget_info(|| combo_info(c, item));
    paint(ui, cx, c, item);
    response
}

/// Draws spinner `c` showing `item`: the caption, the value text and both chevrons.
fn paint(ui: &Ui, cx: &UiCtx, c: &Control, item: &str) {
    let m = cx.m;
    let (_, body, _) = c.strips(m);
    let painter = ui.painter();
    c.strip_texts(painter, m, 0.0, |_| String::new());
    painter.text(
        body.center(),
        Align2::CENTER_CENTER,
        item,
        m.font_id(VALUE_FONT),
        c.channel,
    );
    let stroke = Stroke::new(m.t(), c.channel);
    for which in [Chevron::Up, Chevron::Down] {
        let points = chevron(body.center(), which, m.u(1.0), m.t());
        painter.add(Shape::line(points.to_vec(), stroke));
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use eframe::egui::{pos2, vec2};

    fn near(a: Pos2, b: Pos2) -> bool {
        (a - b).length() < 1e-3
    }

    #[test]
    fn chevrons_follow_the_spec_check_at_u_85() {
        // Apex 0.85 U above / below the centre, arms 4 t sideways and 4 t back. At
        // U = 85 px, t = 6 px: apex 72.25 px, arms 24 px.
        let c = pos2(300.0, 400.0);
        let up = chevron(c, Chevron::Up, 85.0, 6.0);
        // The apex is the middle point, 72.25 above the centre; the arms end 24 below it.
        assert!(near(up[1], pos2(300.0, 400.0 - 72.25)), "{up:?}");
        assert!(near(up[0], pos2(276.0, 400.0 - 72.25 + 24.0)), "{up:?}");
        assert!(near(up[2], pos2(324.0, 400.0 - 72.25 + 24.0)), "{up:?}");
        let down = chevron(c, Chevron::Down, 85.0, 6.0);
        assert!(near(down[1], pos2(300.0, 400.0 + 72.25)), "{down:?}");
        assert!(near(down[0], pos2(276.0, 400.0 + 72.25 - 24.0)), "{down:?}");
        assert!(near(down[2], pos2(324.0, 400.0 + 72.25 - 24.0)), "{down:?}");
    }

    #[test]
    fn the_tap_zones_cover_the_chevrons_and_leave_the_value_to_the_popup() {
        let (u, t) = (32.0, 2.0);
        let body = Rect::from_min_size(pos2(100.0, 124.0), vec2(96.0, 104.0));
        let centre = body.center();
        let (up, down) = zones(body, u);
        assert!(body.contains_rect(up) && body.contains_rect(down));
        // Each chevron's three points are inside its own zone, and the value text is in neither.
        for p in chevron(centre, Chevron::Up, u, t) {
            assert!(up.contains(p) && !down.contains(p), "{p:?}");
        }
        for p in chevron(centre, Chevron::Down, u, t) {
            assert!(down.contains(p) && !up.contains(p), "{p:?}");
        }
        assert!(!up.contains(centre) && !down.contains(centre));
        // Full width: a tap beside the chevron counts too.
        assert!(up.contains(pos2(body.left() + 1.0, body.top() + 1.0)));
        assert!(down.contains(pos2(body.right() - 1.0, body.bottom() - 1.0)));
    }

    #[test]
    fn up_steps_to_the_previous_item_down_to_the_next_and_both_clamp() {
        use Chevron::{Down, Up};
        let step = |index, len, which| step(index, len, which, &[]);
        assert_eq!(step(2, 5, Up), 1);
        assert_eq!(step(2, 5, Down), 3);
        // The ends do not wrap.
        assert_eq!(step(0, 5, Up), 0);
        assert_eq!(step(4, 5, Down), 4);
        assert_eq!(step(0, 1, Up), 0);
        assert_eq!(step(0, 1, Down), 0);
        assert_eq!(step(0, 0, Down), 0);
        assert_eq!(step(3, 0, Up), 0);
        // An index past the end (stale state) comes back into range.
        assert_eq!(step(9, 5, Down), 4);
        assert_eq!(step(9, 5, Up), 3);
    }

    #[test]
    fn the_chevrons_skip_disabled_items_and_stay_when_none_is_left() {
        use Chevron::{Down, Up};
        assert_eq!(step(1, 5, Down, &[2]), 3);
        assert_eq!(step(3, 5, Up, &[2]), 1);
        assert_eq!(step(1, 5, Down, &[2, 3, 4]), 1);
        assert_eq!(step(1, 5, Up, &[0]), 1);
        // From a disabled current item (the Scale's `Custom`) the chevrons lead back to the others.
        assert_eq!(step(4, 5, Up, &[4]), 3);
    }

    #[test]
    fn a_grid_spinner_builds_its_popup_only_while_it_is_open() {
        use crate::ui::offscreen::Offscreen;
        use crate::ui::widgets::Control;
        use eframe::egui::Color32;

        let rect = Rect::from_min_size(pos2(100.0, 100.0), vec2(96.0, 128.0));
        let mut screen = Offscreen::new();
        let (mut index, built) = (0, std::cell::Cell::new(0));
        let mut f = |ui: &mut Ui, cx: &UiCtx| {
            let c = Control::new(Id::new("s"), rect, "Scale", Color32::RED, 0.0);
            spinner_grid(ui, cx, &c, &["a", "b"], &mut index, |current| {
                built.set(built.get() + 1);
                popup_items(&["a", "b"], current, None, &[])
            });
        };
        for _ in 0..5 {
            screen.frame(vec![], &mut f);
        }
        assert_eq!(built.get(), 0);
        // A tap on the value text opens the popup (the text sits under the caption strip's 0.75 U).
        screen.click(pos2(rect.center().x, rect.top() + 24.0 + 52.0), &mut f);
        assert!(built.get() > 0);
    }

    #[test]
    fn the_popup_lists_every_item_in_one_column_with_the_current_one_flagged() {
        let items = popup_items(&["a", "b", "c"], 1, None, &[]);
        assert_eq!(items.len(), 3);
        for (i, item) in items.iter().enumerate() {
            assert_eq!(item.id, Some(i));
            assert_eq!((item.col, item.row), (0, i));
            assert_eq!(item.current, i == 1);
            assert!(!item.disabled && !item.is_header() && !item.keep_open);
        }
        assert_eq!(items[2].label, "c");
    }

    #[test]
    fn a_column_length_fills_columns_top_to_bottom_and_disabled_items_are_flagged() {
        let labels = ["x"; 24];
        let items = popup_items(&labels, 0, Some(8), &[23]);
        for (i, item) in items.iter().enumerate() {
            assert_eq!((item.col, item.row), (i / 8, i % 8), "{i}");
            assert_eq!(item.disabled, i == 23);
        }
    }
}
