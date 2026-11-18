//! The channel properties dialog, the track menu's `Channel Settings`: the title and the close X over the
//! `Channel Name` field and the RGB color wheel/hex picker. Legacy `CHHD` hue is retained. The `Channel Name` field is inline,
//! with `Enter Channel Name` as its hint. A floating dialog centred in the content area, above the transport bar, over
//! the screen dimmed to α 0.6. An instrument or drum channel's dialog is twice as wide, its MIDI input (BanaForge's
//! own, [`midi_rows`]) on the right.

use eframe::egui::{
    self, Align, Align2, Area, Color32, Id, Key, Margin, Order, Rect, Sense, TextEdit, Ui, vec2,
};

use super::{AppCtx, Nav};
use crate::icons::IconId;
use crate::metrics::FontSize;
use crate::midi::device::{MidiChoice, menu};
use crate::params::{menu_index, menu_value};
use crate::song::ChannelFileState;
use crate::theme::channel_color;
use crate::ui::UiCtx;
use crate::ui::widgets::{Button, Control, ParamMenu, button, menu_button};

/// The box, in U.
const SIZE: [f32; 2] = [10.0, 10.5];
/// Inset of the rows from the box's sides.
const PAD: f32 = 0.5;
/// Rows from the box's top: (top, height), in U.
const TITLE: (f32, f32) = (0.25, 1.0);
const CAPTION: (f32, f32) = (1.5, 0.5);
const FIELD: (f32, f32) = (2.0, 1.25);
const SLIDER: (f32, f32) = (3.5, 6.5);
/// The close X's cell at the top-right corner.
const CLOSE: f32 = 1.5;
/// The monitor and arm buttons' disc: 0.7 × half the rect height, as the rack's input row; and the gap after each.
const ICON_DISC: f32 = 0.7;
const BUTTON_GAP: f32 = 0.25;
/// MIDI input names its devices: not on Android or iOS, which have no MIDI input yet.
const MIDI_DEVICES: bool = cfg!(not(mobile));
/// The `MIDI Channel` menu: All, then 1…16, six to a column.
const MIDI_CHANNELS: [&str; 17] = [
    "All", "1", "2", "3", "4", "5", "6", "7", "8", "9", "10", "11", "12", "13", "14", "15", "16",
];

/// Shows `channel`'s dialog, modal, `name` holding its field's text. The name applies on Enter, which closes the
/// dialog, or on a click outside, which closes it too; a blank one is ignored, and Escape closes it without the name
/// (as the Save dialog's Escape drops its field). The colour applies as the
/// slider moves, the MIDI input as it is picked. All are param edits (the audio never reads them): one undo step each,
/// a drag's at its release. A gone channel closes it.
pub fn show(ui: &mut egui::Ui, app: &mut AppCtx, channel: usize, name: &mut String) -> Option<Nav> {
    let Some(ch) = app.song.channels.get(channel) else {
        return Some(Nav::Close);
    };
    let ctx = ui.ctx().clone();
    let (theme, m) = (app.cx.theme, app.cx.m);
    let c = &theme.colors;
    let (text, bg) = (c.text.to_color32(), c.background.to_color32());
    let screen = app.regions.screen;
    let area = app.regions.area;
    // Two columns for an instrument or drum channel, as wide as a floating dialog gets (min(20 U, W − 4 px)).
    let midi = ch.kind.plays_notes();
    let width = if midi {
        m.u(2.0 * SIZE[0]).min(screen.width() - 4.0 / m.ppp())
    } else {
        m.u(SIZE[0])
    };
    let rect = Rect::from_center_size(area.center(), vec2(width, m.u(SIZE[1]).min((area.height() - 4.0 / m.ppp()).max(1.0))));
    let (left, right) = if midi {
        rect.split_left_right_at_fraction(0.5)
    } else {
        (rect, rect)
    };
    let rows = |column: Rect| {
        move |(top, h): (f32, f32)| {
            Rect::from_x_y_ranges(
                column.left() + m.u(PAD)..=column.right() - m.u(PAD),
                rect.top() + top / SIZE[1] * rect.height()..=rect.top() + (top + h) / SIZE[1] * rect.height(),
            )
        }
    };
    let row = rows(left);
    let id = Id::new("channel_settings");
    // A dialog box (`dialog::dialog_box`): no animation, the screen dimmed to α 0.6 under the floating panel from the
    // first frame, over a backdrop that takes every click outside it.
    let backdrop = crate::ui::dialog::dialog_box(&ctx, id, (c, m), screen, rect);
    let escape = crate::ui::close_pressed(&ctx);
    let mut close = backdrop.clicked() || backdrop.secondary_clicked() || escape;
    let mut rgb = ch.file.color_rgb;
    let color_id = id.with(("rgb", ch.id));
    let original_ink = channel_color(ch.color_token(), theme).to_color32();
    // The MIDI layer's devices, by name.
    let devices: Vec<String> = (menu(app.midi_devices).into_iter())
        .filter_map(|c| match c {
            MidiChoice::Named(name) => Some(name),
            _ => None,
        })
        .collect();
    let (mut midi_edited, mut rescan) = (false, false);
    Area::new(id)
        .order(Order::Foreground)
        .fixed_pos(rect.min)
        .constrain(false)
        .fade_in(false)
        .show(&ctx, |ui| {
            ui.allocate_exact_size(rect.size(), Sense::hover());
            let painter = ui.painter().clone();
            let title = rows(rect)(TITLE);
            let font = m.font_id(FontSize::Pt12);
            painter.text(
                title.center(),
                Align2::CENTER_CENTER,
                "Channel Settings",
                font,
                text,
            );
            let caption = row(CAPTION);
            let font = m.font_id(FontSize::Pt8);
            painter.text(
                caption.left_center(),
                Align2::LEFT_CENTER,
                "Channel Name",
                font,
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
                .hint_text("Enter Channel Name")
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
                // On Android a focused field opens the system text editor over the dialog, on iOS the keyboard:
                // there a tap on the field renames (#68).
                response.request_focus();
            }

            if let Some(next) = color_wheel(ui, color_id, row(SLIDER), original_ink) {
                rgb = Some(next);
            }
            let ink = rgb.map_or(original_ink, |[r, g, b]| Color32::from_rgb(r, g, b));
            if midi {
                let colors = &theme.colors;
                let contrast = colors
                    .best_contrasting(crate::theme::Rgba::rgb(f32::from(ink.r()) / 255.0, f32::from(ink.g()) / 255.0, f32::from(ink.b()) / 255.0), &colors.contrast_candidates());
                let file = &mut app.song.channels[channel].file;
                let inks = (ink, contrast.to_color32());
                (midi_edited, rescan) =
                    midi_rows(ui, &app.cx, id, &rows(right), inks, &devices, file);
            }
            // The close X, as a click outside: the name applies.
            let x = Rect::from_min_size(
                rect.right_top() - vec2(m.u(CLOSE), 0.0),
                vec2(m.u(CLOSE), m.u(CLOSE)),
            );
            let surface = (c.menu_fill.to_color32(), 0.375);
            close |= crate::ui::buttons::close_x(ui, &app.cx, id.with("close"), x, surface, 0.6)
                .clicked();
        });
    let ch = &mut app.song.channels[channel];
    let mut edited = rgb != ch.file.color_rgb || midi_edited;
    app.midi_rescan |= rescan;
    ch.file.color_rgb = rgb;
    let renamed = name.trim();
    if close && !escape && !renamed.is_empty() && renamed != ch.name {
        ch.name = renamed.to_owned();
        edited = true;
    }
    if edited {
        app.param_edited();
    }
    if close {
        // Or the undo shortcut stays the field's until egui notices it is gone.
        ctx.memory_mut(|mem| mem.surrender_focus(id.with("name")));
    }
    close.then_some(Nav::Close)
}

/// An instrument or drum channel's MIDI input in the right column (`row`), level with the left one's rows: the
/// `MIDI Input` caption; the `Device` menu, `All` and `devices` (and the chosen one while it is missing), level with
/// the name field; then the monitor (CHHD@1056) and arm (CHHD@1060) buttons, which the rack's audio-input row has on
/// an Audio Record channel, drawn as there (a `text` disc with a `moduleFill` icon, while on a disc in the channel
/// colour with the contrast colour: `inks`), and the `MIDI Channel` menu, `All` and 1…16, level with the slider. Where
/// MIDI input names no devices, the `MIDI Channel` menu takes the `Device` menu's place. Edits `file`. Whether
/// anything changed, and whether the device menu opened (the app lists the ports again).
fn midi_rows(
    ui: &mut Ui,
    cx: &UiCtx,
    id: Id,
    row: &dyn Fn((f32, f32)) -> Rect,
    (ink, contrast): (Color32, Color32),
    devices: &[String],
    file: &mut ChannelFileState,
) -> (bool, bool) {
    let (m, colors) = (cx.m, &cx.theme.colors);
    let caption = row(CAPTION);
    let (font, text) = (m.font_id(FontSize::Pt8), colors.text.to_color32());
    (ui.painter()).text(
        caption.left_center(),
        Align2::LEFT_CENTER,
        "MIDI Input",
        font,
        text,
    );
    let mut changed = false;
    let mut buttons = row(SLIDER);
    let toggles = [
        (IconId::AudioRecord, "monitor", &mut file.input_monitoring),
        (IconId::Record, "arm", &mut file.record_armed),
    ];
    for (icon, name, on) in toggles {
        let (rect, rest) = buttons.split_left_right_at_x(buttons.left() + buttons.height());
        buttons = rest.with_min_x((rest.left() + m.u(BUTTON_GAP)).min(rest.right()));
        let b = Button {
            icon: Some(icon),
            shape: ICON_DISC,
            fill_on: ink,
            text_on: contrast,
            fill_off: text,
            text_off: Some(colors.module_fill.to_color32()),
            ..Button::new(id.with(name), rect, "", true, colors)
        };
        if button(ui, cx, &b, *on).clicked() {
            *on = !*on;
            changed = true;
        }
    }
    let field = row(FIELD);
    let side = Rect::from_center_size(buttons.center(), vec2(buttons.width(), field.height()));
    let (device, channel) = if MIDI_DEVICES {
        (Some(field), side)
    } else {
        (None, field)
    };
    let input = &mut file.midi_input;
    let mut opened = false;
    if let Some(rect) = device {
        let chosen = input.device.clone();
        let named = devices.iter().map(String::as_str);
        let mut items: Vec<&str> = std::iter::once("All").chain(named).collect();
        // A device unplugged stays chosen, and matches nothing until it is back.
        items.extend(
            chosen
                .as_deref()
                .filter(|d| !devices.iter().any(|x| x == d)),
        );
        let at = (chosen.as_deref()).and_then(|d| items.iter().skip(1).position(|x| *x == d));
        let mut value = menu_value(at.map_or(0, |i| i + 1), items.len());
        let c = Control {
            menu: ParamMenu::Off,
            ..Control::new(id.with("device"), rect, "Device", ink, 0.0)
        };
        let response = menu_button(ui, cx, &c, &items, &mut value);
        opened = response.clicked();
        if response.changed() {
            let i = menu_index(value, items.len());
            input.device = (i > 0).then(|| items[i].to_owned());
            changed = true;
        }
    }
    let n = MIDI_CHANNELS.len();
    let mut value = menu_value(input.channel.map_or(0, |c| usize::from(c) + 1), n);
    let c = Control {
        menu: ParamMenu::Off,
        column_len: Some(6),
        ..Control::new(id.with("midi channel"), channel, "MIDI Channel", ink, 0.0)
    };
    if menu_button(ui, cx, &c, &MIDI_CHANNELS, &mut value).changed() {
        let i = menu_index(value, n);
        input.channel = (i > 0).then(|| (i - 1) as u8);
        changed = true;
    }
    (changed, opened)
}

fn accepted_rgb(text: &str) -> Option<[u8; 3]> {
    let c = crate::theme::Rgba::from_hex_str(text.trim())?.to_color32();
    let rgb = [c.r(), c.g(), c.b()];
    (rgb != [0, 0, 0] && rgb != [255, 255, 255]).then_some(rgb)
}

/// Hue/saturation on a genuine circular wheel, value below, exact hex entry.
fn color_wheel(ui: &mut Ui, id: Id, rect: Rect, current: Color32) -> Option<[u8; 3]> {
    use eframe::egui::{Mesh, Stroke, pos2};
    use eframe::egui::ecolor::{hsv_from_rgb, rgb_from_hsv};
    let (mut h, mut s, mut v) = hsv_from_rgb([f32::from(current.r()) / 255.0, f32::from(current.g()) / 255.0, f32::from(current.b()) / 255.0]);
    let side = rect.width().min((rect.height() - 64.0).max(8.0));
    let wheel = Rect::from_center_size(pos2(rect.center().x, rect.top() + side / 2.0), vec2(side, side));
    let radius = side * 0.48;
    let centre = wheel.center();
    let mut mesh = Mesh::default();
    let color = |h, s, v| {
        let [r, g, b] = rgb_from_hsv((h, s, v));
        crate::theme::Rgba::rgb(r, g, b).to_color32()
    };
    mesh.colored_vertex(centre, color(0.0, 0.0, v));
    for i in 0..=96 {
        let hue = i as f32 / 96.0;
        let angle = hue * std::f32::consts::TAU;
        mesh.colored_vertex(centre + vec2(angle.cos(), angle.sin()) * radius, color(hue, 1.0, v));
        if i < 96 { mesh.add_triangle(0, i + 1, i + 2); }
    }
    ui.painter().add(mesh);
    let response = ui.interact(wheel, id.with("wheel"), Sense::click_and_drag()).on_hover_text("Channel color wheel");
    let mut changed = false;
    if (response.clicked() || response.dragged()) && let Some(p) = response.interact_pointer_pos() {
        let delta = p - centre;
        h = (delta.y.atan2(delta.x) / std::f32::consts::TAU).rem_euclid(1.0);
        s = (delta.length() / radius).clamp(0.0, 1.0);
        changed = true;
    }
    let angle = h * std::f32::consts::TAU;
    let marker = centre + vec2(angle.cos(), angle.sin()) * s * radius;
    ui.painter().circle_stroke(marker, 4.0, Stroke::new(2.0, Color32::BLACK));
    ui.painter().circle_stroke(marker, 5.0, Stroke::new(1.0, Color32::WHITE));
    let brightness = Rect::from_min_size(pos2(rect.left(), wheel.bottom() + 2.0), vec2(rect.width(), 24.0));
    let response = ui.put(brightness, egui::Slider::new(&mut v, 0.01..=1.0).text("Value"));
    changed |= response.changed();
    if response.double_clicked() { v = 1.0; changed = true; }
    let key = id.with("hex");
    let mut hex = ui.data(|d| d.get_temp::<String>(key)).unwrap_or_else(|| format!("#{:02X}{:02X}{:02X}", current.r(), current.g(), current.b()));
    let mut next = None;
    if changed {
        let c = color(h, s, v);
        let text = format!("#{:02X}{:02X}{:02X}", c.r(), c.g(), c.b());
        if let Some(rgb) = accepted_rgb(&text) { hex = text; next = Some(rgb); }
    }
    let field = Rect::from_min_size(pos2(rect.left(), brightness.bottom() + 2.0), vec2(rect.width(), 24.0));
    let mut response = ui.put(field, TextEdit::singleline(&mut hex).id(key).hint_text("#RRGGBB"));
    crate::android_text::edit(&mut response, &mut hex, 7);
    if response.changed() { next = accepted_rgb(&hex); }
    response.on_hover_text("Hex RGB. Pure black and pure white are blocked for readability.");
    ui.data_mut(|d| d.insert_temp(key, hex));
    next
}

#[cfg(test)]
mod color_tests {
    use super::accepted_rgb;
    #[test]
    fn channel_hex_accepts_rgb_and_rejects_unreadable_extremes() {
        assert_eq!(accepted_rgb("#12abEF"), Some([0x12, 0xab, 0xef]));
        for value in ["#000000", "FFFFFF", "#fff", "#12345G", ""] {
            assert_eq!(accepted_rgb(value), None);
        }
    }
}
