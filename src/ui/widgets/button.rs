//! The button: a round or rectangular background that differs between on and off, an optional halo behind it, optional
//! icon and text.
//!
//! A button bound to a parameter opens the parameter menu on a long press; the panel says so ([`Button::menu`]).
//!
//! The button reports clicks and nothing more: what a click flips (a parameter, a module setting, a momentary action)
//! is the panel's business, and so is the colour for each state, since the slot mapping differs from button to button.
//! [`Button::new`] starts from the common one, `text` disc / `menuFill` label when on and the reverse when off (the
//! Pitcher note buttons). The whole rect takes input, not only the disc. There is no disabled look: a disabled button
//! only stops reacting.
//!
//! The icon is tinted like the label: a single icon colour would vanish on the Tuner's channel-filled "on" disc, so the
//! icon takes the state's label colour. Its size is the standard icon size, an edge of `U × size factor × 4/3` whatever
//! the rect ([`Button::icon_scale`] is the factor, 1 normally), cut back so the icon never leaves its area, and on a
//! round button never leaves the disc.

use eframe::egui::{
    Align2, Color32, Id, Pos2, Rect, Response, Sense, Ui, Vec2, WidgetInfo, WidgetType,
};

use super::{ParamMenu, context_menu, hovered, primary_down_on};
use crate::icons::{IconId, draw_icon, icon_edge};
use crate::metrics::{FontSize, Metrics};
use crate::theme::ThemeColors;
use crate::ui::{UiCtx, focus};

/// A pressed button's colours, and a hovered button's fill, are this much brighter.
const PRESSED_GAIN: f32 = 1.15;

/// Alpha of the on-state label when the button has no off label colour.
const OFF_ALPHA: f32 = 0.25;

/// A button in `rect`. Start from [`Button::new`] and change the other fields with struct update syntax.
#[derive(Clone, Copy, Debug)]
pub struct Button<'a> {
    pub id: Id,
    pub rect: Rect,
    /// Centred in the rect, or in its lower third when there is an icon. Empty: none.
    pub text: &'a str,
    /// FontSize 2 unless the module picks another.
    pub font: FontSize,
    /// The icon, tinted like the label (the state's label colour), centred in the upper part of the rect (the whole
    /// rect without text).
    pub icon: Option<IconId>,
    /// The icon's size factor: its edge is `U × icon_scale × 4/3`, limited to what fits in its area and, for a round
    /// button, in the disc.
    pub icon_scale: f32,
    /// A disc rather than the full rect.
    pub round: bool,
    /// Round buttons: the disc's radius is `shape × 0.5 × min(w, h)`.
    pub shape: f32,
    /// A disc behind the button bigger by the pad in device pixels (for rect buttons the rect grown by half the pad on
    /// every side): the halo's colour and pad.
    pub halo: Option<(Color32, f32)>,
    pub fill_on: Color32,
    pub fill_off: Color32,
    pub text_on: Color32,
    /// The off label's colour (and icon tint); `None`: the on one at 25 % alpha.
    pub text_off: Option<Color32>,
    /// No input.
    pub disabled: bool,
    /// [`ParamMenu::Bound`] or [`ParamMenu::Linked`]: a long press or secondary click opens the parameter menu (its
    /// items without `Reset`). Otherwise there is none.
    pub menu: ParamMenu,
}

impl<'a> Button<'a> {
    pub fn new(id: Id, rect: Rect, text: &'a str, round: bool, colors: &ThemeColors) -> Self {
        let (text_color, menu_fill) = (colors.text.to_color32(), colors.menu_fill.to_color32());
        Self {
            id,
            rect,
            text,
            font: FontSize::Pt10,
            icon: None,
            icon_scale: 1.0,
            round,
            shape: 1.0,
            halo: None,
            fill_on: text_color,
            fill_off: menu_fill,
            text_on: menu_fill,
            text_off: Some(text_color),
            disabled: false,
            menu: ParamMenu::Unbound,
        }
    }
}

/// `color` as a pressed button draws it: RGB × 1.15, clamped, alpha unchanged.
pub(crate) fn pressed(color: Color32) -> Color32 {
    let [r, g, b, a] = color.to_srgba_unmultiplied();
    let brighter = |c: u8| (f32::from(c) * PRESSED_GAIN).round().min(255.0) as u8;
    Color32::from_rgba_unmultiplied(brighter(r), brighter(g), brighter(b), a)
}

/// Radius of a round button's disc.
fn disc_radius(rect: Rect, shape: f32) -> f32 {
    shape * 0.5 * rect.width().min(rect.height())
}

/// The icon's area and the text's area. With both, the icon's area is the upper two thirds (the icon keeps its standard
/// size, centred in it) and the text sits in the lower third; either alone has the whole rect.
fn content_rects(rect: Rect, has_icon: bool, has_text: bool) -> (Rect, Rect) {
    let third = rect.bottom() - rect.height() / 3.0;
    (
        if has_text {
            rect.with_max_y(third)
        } else {
            rect
        },
        if has_icon {
            rect.with_min_y(third)
        } else {
            rect
        },
    )
}

/// The label colour for the state.
fn text_color(on: bool, text_on: Color32, text_off: Option<Color32>) -> Color32 {
    if on {
        text_on
    } else {
        text_off.unwrap_or_else(|| text_on.gamma_multiply(OFF_ALPHA))
    }
}

/// Edge of the largest square whose centre is `offset` from a disc's centre and that lies inside the disc: its far
/// corner, at (|dx| + e/2, |dy| + e/2), is on the circle. 0 when the centre is outside the disc.
fn edge_in_disc(offset: Vec2, radius: f32) -> f32 {
    let (a, b) = (offset.x.abs(), offset.y.abs());
    let room = 2.0 * radius * radius - (a - b) * (a - b);
    // Half the edge, from 2h² + 2(a + b)h + a² + b² − r² = 0.
    let half = (room.max(0.0).sqrt() - (a + b)) / 2.0;
    2.0 * half.max(0.0)
}

/// The size factor (for [`draw_icon`]) of an icon centred in `area`: `scale`, the standard edge (`U × scale × 4/3`
/// independent of the rect), cut back so that the icon's square lies inside `area` and, for a round button, inside
/// `disc` (its centre and radius). The pixel snapping of the square (`icon_square`) can push it up to one device pixel
/// out on the top and left, so the limits keep two pixels of room.
fn icon_factor(area: Rect, scale: f32, disc: Option<(Pos2, f32)>, m: &Metrics) -> f32 {
    let mut room = area.width().min(area.height());
    if let Some((center, radius)) = disc {
        room = room.min(edge_in_disc(area.center() - center, radius));
    }
    let room = (room - 2.0 / m.ppp()).max(0.0);
    scale.min(room / icon_edge(1.0, m))
}

/// Draws the button in state `on` and returns its response; `clicked()` is a primary click.
pub fn button(ui: &mut Ui, cx: &UiCtx, b: &Button, on: bool) -> Response {
    let m = cx.m;
    let sense = if b.disabled {
        Sense::hover()
    } else {
        Sense::click()
    };
    let mut response = focus::interact(ui, b.rect, b.id, sense);
    response.widget_info(|| WidgetInfo::labeled(WidgetType::Button, !b.disabled, b.text));
    context_menu(ui, cx, b.id, b.menu, false, false, &mut response);

    let painter = ui.painter();
    // Pressed, every colour is drawn brighter. Hovered on desktop, the fill alone, and never twice.
    let down = primary_down_on(ui, &response);
    let shade = if down { pressed } else { |c| c };
    let fill = if on { b.fill_on } else { b.fill_off };
    let fill = if down || hovered(ui, &response) {
        pressed(fill)
    } else {
        fill
    };
    let halo = b.halo.map(|(color, pad)| (shade(color), pad / m.ppp()));
    if b.round {
        let r = disc_radius(b.rect, b.shape);
        if let Some((color, pad)) = halo {
            painter.circle_filled(b.rect.center(), r + pad, color);
        }
        painter.circle_filled(b.rect.center(), r, fill);
    } else {
        // A rect's halo grows by half the pad on every side.
        if let Some((color, pad)) = halo {
            painter.rect_filled(b.rect.expand(pad / 2.0), 0.0, color);
        }
        painter.rect_filled(b.rect, 0.0, fill);
    }

    let (icon_area, text_area) = content_rects(b.rect, b.icon.is_some(), !b.text.is_empty());
    let ink = shade(text_color(on, b.text_on, b.text_off));
    if let Some(icon) = b.icon {
        let disc = b
            .round
            .then(|| (b.rect.center(), disc_radius(b.rect, b.shape)));
        let factor = icon_factor(icon_area, b.icon_scale, disc, m);
        draw_icon(painter, icon_area, icon, ink, factor, m);
    }
    painter.text(
        text_area.center(),
        Align2::CENTER_CENTER,
        b.text,
        m.font_id(b.font),
        ink,
    );
    response
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::icons::icon_edge;
    use crate::metrics::DEFAULT_UI_SCALE_INDEX;
    use eframe::egui::{pos2, vec2};

    #[test]
    fn a_round_button_is_a_disc_of_shape_times_half_the_short_side() {
        let tall = Rect::from_min_size(pos2(5.0, 5.0), vec2(56.0, 69.0));
        assert_eq!(disc_radius(tall, 0.7), 0.7 * 0.5 * 56.0);
        let wide = Rect::from_min_size(pos2(5.0, 5.0), vec2(80.0, 40.0));
        assert_eq!(disc_radius(wide, 1.0), 20.0);
        assert_eq!(disc_radius(wide, 0.0), 0.0);
    }

    #[test]
    fn the_icon_takes_the_upper_part_and_the_text_the_lower_third() {
        let rect = Rect::from_min_size(pos2(10.0, 30.0), vec2(60.0, 90.0));
        // Text alone, or an icon alone: the whole rect.
        assert_eq!(content_rects(rect, false, true).1, rect);
        assert_eq!(content_rects(rect, true, false).0, rect);
        // Both: the icon above, the text in the lower third.
        let (icon, text) = content_rects(rect, true, true);
        assert_eq!(icon, Rect::from_min_max(pos2(10.0, 30.0), pos2(70.0, 90.0)));
        assert_eq!(
            text,
            Rect::from_min_max(pos2(10.0, 90.0), pos2(70.0, 120.0))
        );
    }

    #[test]
    fn the_off_text_is_the_on_text_at_a_quarter_alpha_unless_set() {
        let on = Color32::from_rgb(200, 100, 40);
        let off = Color32::from_rgb(1, 2, 3);
        assert_eq!(text_color(true, on, None), on);
        assert_eq!(text_color(true, on, Some(off)), on);
        assert_eq!(text_color(false, on, Some(off)), off);
        assert_eq!(text_color(false, on, None), on.gamma_multiply(0.25));
        assert_eq!(text_color(false, on, None).a(), 64);
    }

    fn m(ppp: f32) -> Metrics {
        Metrics::from_pixels_per_point(ppp, DEFAULT_UI_SCALE_INDEX)
    }

    #[test]
    fn the_largest_square_in_a_disc_has_its_far_corner_on_the_circle() {
        // Centred: the inscribed square, edge r·√2.
        let r = 20.0;
        assert!((edge_in_disc(vec2(0.0, 0.0), r) - r * 2f32.sqrt()).abs() < 1e-4);
        // Off centre: the far corner (the one away from the centre) touches the circle.
        for off in [vec2(0.0, -8.0), vec2(5.0, -7.0), vec2(-6.0, 3.0)] {
            let half = edge_in_disc(off, r) / 2.0;
            let corner = vec2(off.x.abs() + half, off.y.abs() + half);
            assert!((corner.length() - r).abs() < 1e-3, "{off:?}: {corner:?}");
        }
        // A centre outside the disc has no room.
        assert_eq!(edge_in_disc(vec2(30.0, 0.0), r), 0.0);
    }

    #[test]
    fn the_icon_has_the_icons_md_edge_unless_its_area_or_disc_is_smaller() {
        for ppp in [1.0, 2.0] {
            let m = m(ppp);
            let u = m.u(1.0);
            let big = Rect::from_min_size(pos2(0.0, 0.0), vec2(5.0 * u, 5.0 * u));
            // U × 4/3 for a size factor of 1, whatever the rect; the scale multiplies it.
            assert_eq!(icon_factor(big, 1.0, None, &m), 1.0);
            assert_eq!(icon_factor(big, 0.5, None, &m), 0.5);
            assert!((icon_edge(icon_factor(big, 1.0, None, &m), &m) - 4.0 / 3.0 * u).abs() < 1e-4);
            // A small area cuts the icon back to its shorter side, less 2 device pixels for the pixel snapping.
            let small = Rect::from_min_size(pos2(0.0, 0.0), vec2(2.0 * u, 0.9 * u));
            let edge = icon_edge(icon_factor(small, 1.0, None, &m), &m);
            assert!(edge <= 0.9 * u && edge > 0.9 * u - 3.0 / ppp, "{edge}");
            // A disc cuts it back to the inscribed square.
            let rect = Rect::from_min_size(pos2(0.0, 0.0), vec2(1.75 * u, 2.15 * u));
            let disc = Some((rect.center(), 0.7 * 0.5 * 1.75 * u));
            let edge = icon_edge(icon_factor(rect, 1.0, disc, &m), &m);
            let inscribed = 2f32.sqrt() * 0.7 * 0.5 * 1.75 * u;
            assert!(
                edge <= inscribed && edge > inscribed - 3.0 / ppp,
                "{edge} vs {inscribed}"
            );
        }
    }

    #[test]
    fn the_snapped_icon_square_stays_inside_the_disc_at_any_pixel_density() {
        use crate::icons::icon_square;
        for ppp in [1.0, 1.25, 1.5, 2.0, 2.625, 3.0] {
            for index in 0..crate::metrics::UI_SCALE_FACTORS.len() {
                let m = Metrics::from_pixels_per_point(ppp, index);
                let u = m.u(1.0);
                // An odd position, so that snapping moves the square's corner.
                let rect = Rect::from_min_size(pos2(100.3, 100.7), vec2(1.75 * u, 2.15 * u));
                let radius = 0.7 * 0.5 * 1.75 * u;
                let factor = icon_factor(rect, 1.0, Some((rect.center(), radius)), &m);
                let square = icon_square(rect, factor, &m);
                for corner in [
                    square.left_top(),
                    square.right_top(),
                    square.left_bottom(),
                    square.right_bottom(),
                ] {
                    let d = (corner - rect.center()).length();
                    assert!(
                        d <= radius,
                        "ppp {ppp} scale {index}: {square:?}, {d} > {radius}"
                    );
                }
            }
        }
    }
}
