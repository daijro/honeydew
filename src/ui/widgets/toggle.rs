//! The pill toggle: a vertical pill whose dot sits up (on) or down (off), with the label above it, right of it, or
//! none. Effects and instruments have the same control, so both use this one. The mixer mute shows the inverted
//! meaning: its dot is up while the value is off.
//!
//! A right label is drawn in full from 1.7 U, past the rect when it is longer (`HQ`, `Pulse Mode`, `Inv. Fdbk`):
//! plain text, like every widget text.

use eframe::egui::{Align2, Pos2, Rect, Response, Sense, Ui, WidgetInfo, WidgetType, pos2, vec2};

use super::{Control, clean, context_menu, toggle_on};
use crate::metrics::Metrics;
use crate::ui::{UiCtx, focus};

/// Right labels: the pill's centre and the label's start from the rect's left edge, in U.
const RIGHT_PILL_X: f32 = 0.75;
const RIGHT_LABEL_X: f32 = 1.7;

/// How a pill toggle is laid out and what its dot shows. The default is the label-above toggle of [`toggle`].
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct Pill {
    /// The label right of the pill: the pill is centred 0.75 U from the left edge (vertically centred) and the label
    /// starts 1.7 U from it, vertically centred, in `label_font` (10 pt in the instrument tables) and drawn in full,
    /// unclipped. `value_text` is ignored: no `On` / `Off` strip is drawn or reserved. With an empty label the pill
    /// is centred in the rect (the no-label toggle).
    pub label_right: bool,
    /// The dot is up (channel) while the value is off, exactly 0: the mixer mute, "dot up = not muted". A click
    /// still flips the value.
    pub inverted: bool,
}

/// The pill (10 t × 18 t, centred on `center`) and the dot's centre: 4 t above the middle when on, 4 t below when
/// off.
fn switch(center: Pos2, on: bool, t: f32) -> (Rect, Pos2) {
    let pill = Rect::from_center_size(center, vec2(10.0 * t, 18.0 * t));
    let dy = if on { -4.0 * t } else { 4.0 * t };
    (pill, center + vec2(0.0, dy))
}

/// The pill's centre and, for a right label, the left-centre point the label hangs from. A label-above toggle has
/// its pill centred in the body between the top label and `On` / `Off` strips; a right-label toggle without a label
/// has it centred in the rect.
fn place(c: &Control, m: &Metrics, label_right: bool) -> (Pos2, Option<Pos2>) {
    let r = c.rect;
    match (label_right, c.label.is_empty()) {
        (false, _) => (c.strips(m).1.center(), None),
        (true, true) => (r.center(), None),
        (true, false) => (
            pos2(r.left() + m.u(RIGHT_PILL_X), r.center().y),
            Some(pos2(r.left() + m.u(RIGHT_LABEL_X), r.center().y)),
        ),
    }
}

/// A primary click writes 1 from exactly 0 and 0 from anything else ([`toggle_on`]); a long press or secondary click
/// opens the parameter menu, which has no `Reset`. Returns the response and the value to draw.
pub(super) fn flip(ui: &mut Ui, cx: &UiCtx, c: &Control, value: &mut f32) -> (Response, f32) {
    let sense = if c.disabled {
        Sense::hover()
    } else {
        Sense::click()
    };
    let mut response = focus::interact(ui, c.rect, c.id, sense);
    // The menu first: the release after its long press is no click.
    context_menu(ui, cx, c.id, c.menu, false, false, &mut response);
    if response.clicked() {
        *value = if toggle_on(clean(*value, c.default)) {
            0.0
        } else {
            1.0
        };
        response.mark_changed();
    }
    let v = clean(*value, c.default);
    let (enabled, on) = (!c.disabled, toggle_on(v));
    response.widget_info(|| WidgetInfo::selected(WidgetType::Checkbox, enabled, on, c.label));
    (response, v)
}

/// A label-above toggle in `c.rect`: label strip on top (when labelled), optional `On` / `Off` strip at the bottom
/// (the text is `display(value)`), the pill centred between. See [`pill_toggle`].
pub fn toggle(
    ui: &mut Ui,
    cx: &UiCtx,
    c: &Control,
    value: &mut f32,
    display: impl Fn(f32) -> String,
) -> Response {
    pill_toggle(ui, cx, c, Pill::default(), value, display)
}

/// A pill toggle in `c.rect`, laid out as `pill` says. On is any value but 0 ([`toggle_on`]; exactly 0 when inverted);
/// a primary click flips the value to 0 or 1, and the parameter menu opens without `Reset`.
pub fn pill_toggle(
    ui: &mut Ui,
    cx: &UiCtx,
    c: &Control,
    pill: Pill,
    value: &mut f32,
    display: impl Fn(f32) -> String,
) -> Response {
    let (response, v) = flip(ui, cx, c, value);
    let (m, colors) = (cx.m, &cx.theme.colors);

    let painter = ui.painter();
    let (center, label) = place(c, m, pill.label_right);
    if !pill.label_right {
        c.strip_texts(painter, m, v, display);
    } else if let Some(at) = label {
        let font = m.font_id(c.label_font);
        painter.text(at, Align2::LEFT_CENTER, c.label, font, c.channel);
    }
    let t = m.t();
    let on = toggle_on(v) != pill.inverted;
    let (pill, dot) = switch(center, on, t);
    painter.rect_filled(pill, 5.0 * t, colors.control_fill.to_color32());
    let dot_color = if on {
        c.channel
    } else {
        colors.menu_fill.to_color32()
    };
    painter.circle_filled(dot, 2.0 * t, dot_color);
    response
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::metrics::DEFAULT_UI_SCALE;
    use eframe::egui::{Color32, Id};

    #[test]
    fn pill_is_10t_by_18t_and_the_dot_moves_4t_from_the_middle() {
        let c = pos2(50.0, 60.0);
        let (pill, on) = switch(c, true, 2.0);
        assert_eq!(pill, Rect::from_center_size(c, vec2(20.0, 36.0)));
        assert_eq!(on, pos2(50.0, 52.0));
        let (pill_off, off) = switch(c, false, 2.0);
        assert_eq!(pill_off, pill);
        assert_eq!(off, pos2(50.0, 68.0));
    }

    #[test]
    fn a_right_label_puts_the_pill_0_75_u_in_and_the_label_at_1_7_u() {
        // U = 32.
        let m = Metrics::from_pixels_per_point(1.0, DEFAULT_UI_SCALE);
        let rect = |w| Rect::from_min_size(pos2(10.0, 20.0), vec2(w, 48.0));
        let c = |w, label| Control::new(Id::new("t"), rect(w), label, Color32::RED, 0.0);
        let (centre, at) = place(&c(120.0, "Sync"), &m, true);
        assert_eq!(centre, pos2(10.0 + 24.0, 44.0));
        let at = at.unwrap();
        assert!((at - pos2(10.0 + 54.4, 44.0)).length() < 1e-3, "{at:?}");
        // A rect narrower than 1.7 U: the label still starts at 1.7 U, outside it.
        let (_, at) = place(&c(40.0, "Sync"), &m, true);
        assert!((at.unwrap().x - 64.4).abs() < 1e-3);
        // No label: the pill centred in the rect, either style.
        for right in [false, true] {
            assert_eq!(place(&c(120.0, ""), &m, right), (pos2(70.0, 44.0), None));
        }
        // Label above: the pill centred below the 0.75 U strip.
        assert_eq!(
            place(&c(120.0, "Sync"), &m, false),
            (pos2(70.0, 56.0), None)
        );
        // A right-label toggle ignores `value_text`: nothing is reserved at the bottom.
        let shown = |label| Control {
            value_text: true,
            ..c(120.0, label)
        };
        assert_eq!(place(&shown(""), &m, true), (pos2(70.0, 44.0), None));
        assert_eq!(place(&shown("Sync"), &m, true).0, pos2(34.0, 44.0));
        // Label above with `On` / `Off`: centred between the two strips.
        assert_eq!(place(&shown("Sync"), &m, false).0, pos2(70.0, 44.0));
    }
}
