//! A song section's dialog (#80, a Honeydew addition), in the `Channel Settings` dialog's style
//! ([`super::channel_settings`]): the `Section Settings` title and the close X over the `Section Name` field, the
//! `Section Color` caption with `Color` in the picked colour, the channel colour slider (the hue) and `Delete`. The
//! playlist ruler's `Section…` and a tap on a section's name there open it.

use eframe::egui::{
    self, Align, Align2, Area, Id, Key, Margin, Order, Rect, Sense, TextEdit, pos2, vec2,
};

use super::{AppCtx, Nav, Overlay};
use crate::metrics::FontSize;
use crate::song::Song;
use crate::theme::channel_color;
use crate::ui::widgets::{Button, Control, ParamMenu, button, h_slider};

/// The box, in U.
const SIZE: [f32; 2] = [10.0, 7.5];
/// Inset of the rows from the box's sides.
const PAD: f32 = 0.5;
/// Rows from the box's top: (top, height), in U.
const TITLE: (f32, f32) = (0.25, 1.0);
const CAPTION: (f32, f32) = (1.5, 0.5);
const FIELD: (f32, f32) = (2.0, 1.25);
const COLOR: (f32, f32) = (3.5, 0.5);
const SLIDER: (f32, f32) = (4.0, 1.5);
const DELETE: (f32, f32) = (5.75, 1.25);
/// The close X's cell at the top-right corner.
const CLOSE: f32 = 1.5;

/// Opens section `section`'s dialog, its field holding the section's name.
pub fn open(song: &Song, section: usize) -> Nav {
    let name = (song.sections.get(section)).map_or_else(String::new, |s| s.name.clone());
    Nav::Open(Overlay::SectionSettings { section, name })
}

/// Shows section `section`'s dialog, modal, `name` holding its field's text. As the channel's: the name applies on
/// Enter or a click outside (or the X), which close it, a blank one ignored, and Escape closes it without the name; the
/// colour applies as the slider moves. `Delete` removes the section and closes it. Each is an undo step with no
/// rebuild (the audio never reads sections), a drag's at its release. A gone section closes it.
pub fn show(ui: &mut egui::Ui, app: &mut AppCtx, section: usize, name: &mut String) -> Option<Nav> {
    let Some(s) = app.song.sections.get(section) else {
        return Some(Nav::Close);
    };
    let ctx = ui.ctx().clone();
    let (theme, m) = (app.cx.theme, app.cx.m);
    let c = &theme.colors;
    let (text, bg) = (c.text.to_color32(), c.background.to_color32());
    let rect = Rect::from_center_size(app.regions.area.center(), vec2(m.u(SIZE[0]), m.u(SIZE[1])));
    let row = |(top, h): (f32, f32)| {
        Rect::from_x_y_ranges(
            rect.left() + m.u(PAD)..=rect.right() - m.u(PAD),
            rect.top() + m.u(top)..=rect.top() + m.u(top + h),
        )
    };
    let id = Id::new("section_settings");
    let backdrop = crate::ui::dialog::dialog_box(&ctx, id, (c, m), app.regions.screen, rect);
    let escape = crate::ui::close_pressed(&ctx);
    let mut close = backdrop.clicked() || backdrop.secondary_clicked() || escape;
    let (mut hue, mut delete) = (s.hue, false);
    Area::new(id)
        .order(Order::Foreground)
        .fixed_pos(rect.min)
        .constrain(false)
        .fade_in(false)
        .show(&ctx, |ui| {
            ui.allocate_exact_size(rect.size(), Sense::hover());
            let painter = ui.painter().clone();
            let title = row(TITLE).with_min_x(rect.left()).with_max_x(rect.right());
            let font = m.font_id(FontSize::Pt12);
            painter.text(
                title.center(),
                Align2::CENTER_CENTER,
                "Section Settings",
                font,
                text,
            );
            let font = m.font_id(FontSize::Pt8);
            let caption = row(CAPTION);
            painter.text(
                caption.left_center(),
                Align2::LEFT_CENTER,
                "Section Name",
                font.clone(),
                text,
            );

            let field = row(FIELD);
            painter.rect_filled(field, 0.0, bg);
            let visuals = ui.visuals_mut();
            visuals.text_cursor.stroke.color = text;
            visuals.selection.bg_fill = c.accent.to_color32().gamma_multiply(0.5);
            visuals.selection.stroke.color = text;
            let edit = TextEdit::singleline(&mut *name)
                .id(id.with("name"))
                .hint_text("Enter Section Name")
                .frame(false)
                .margin(Margin::symmetric(m.u(0.25) as i8, 0))
                .font(m.font_id(FontSize::Pt10))
                .text_color(text)
                .desired_width(f32::INFINITY)
                .min_size(field.size())
                .vertical_align(Align::Center);
            let mut response = ui.put(field, edit);
            crate::android_text::edit(&mut response, name, usize::MAX);
            if response.lost_focus() && ui.input(|i| i.key_pressed(Key::Enter)) {
                close = true;
            } else if crate::android_text::REFOCUS && ui.memory(|mem| mem.focused().is_none()) {
                response.request_focus();
            }

            // `Section Color`, its `Color` in the picked colour (the reporter's ask), over the channel colour slider.
            let ink = channel_color(hue, theme).to_color32();
            let caption = row(COLOR);
            let word = |s: &str, ink| painter.layout_no_wrap(s.into(), font.clone(), ink);
            let (left, right) = (word("Section ", text), word("Color", ink));
            let x = caption.center().x - (left.size().x + right.size().x) / 2.0;
            let y = caption.center().y - left.size().y / 2.0;
            let gap = left.size().x;
            painter.galley(pos2(x, y), left, text);
            painter.galley(pos2(x + gap, y), right, ink);
            let control = Control {
                menu: ParamMenu::Off,
                bipolar: false,
                ..Control::new(id.with("color"), row(SLIDER), "", ink, hue)
            };
            h_slider(ui, &app.cx, &control, &mut hue, |_| String::new());

            let delete_at = row(DELETE).shrink2(vec2(m.u(2.0), 0.0));
            let b = Button {
                fill_off: bg,
                corner: m.u(0.15),
                ..Button::new(id.with("delete"), delete_at, "Delete", false, c)
            };
            delete = button(ui, &app.cx, &b, false).clicked();
            let x = Rect::from_min_size(
                rect.right_top() - vec2(m.u(CLOSE), 0.0),
                vec2(m.u(CLOSE), m.u(CLOSE)),
            );
            let surface = (c.menu_fill.to_color32(), 0.375);
            close |= crate::ui::buttons::close_x(ui, &app.cx, id.with("close"), x, surface, 0.6)
                .clicked();
        });
    let s = &mut app.song.sections[section];
    let mut edited = hue != s.hue;
    s.hue = hue;
    let renamed = name.trim();
    if close && !escape && !renamed.is_empty() && renamed != s.name {
        s.name = renamed.to_owned();
        edited = true;
    }
    if delete {
        app.song.sections.remove(section);
        (close, edited) = (true, true);
    }
    if edited {
        app.param_edited();
    }
    if close {
        ctx.memory_mut(|mem| mem.surrender_focus(id.with("name")));
    }
    close.then_some(Nav::Close)
}
