//! The channel properties dialog, the track menu's `Channel Settings`: the title and the close X over the
//! `Channel Name` field and the `Channel Color` slider (the hue, `CHHD` @1036). The `Channel Name` field is inline,
//! with `Enter Channel Name` as its hint. A floating dialog centred in the content area, above the transport bar, over
//! the screen dimmed to α 0.6. An instrument or drum channel's dialog is twice as wide, its MIDI input (Honeydew's
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
use crate::ui::widgets::{Button, Control, ParamMenu, button, h_slider, menu_button};

/// The box, in U.
const SIZE: [f32; 2] = [10.0, 5.75];
/// Inset of the rows from the box's sides.
const PAD: f32 = 0.5;
/// Rows from the box's top: (top, height), in U.
const TITLE: (f32, f32) = (0.25, 1.0);
const CAPTION: (f32, f32) = (1.5, 0.5);
const FIELD: (f32, f32) = (2.0, 1.25);
const SLIDER: (f32, f32) = (3.5, 2.0);
/// The close X's cell at the top-right corner.
const CLOSE: f32 = 1.5;
/// The monitor and arm buttons' disc: 0.7 × half the rect height, as the rack's input row; and the gap after each.
const ICON_DISC: f32 = 0.7;
const BUTTON_GAP: f32 = 0.25;
/// MIDI input names its devices: not on Android, which has no MIDI input yet.
const MIDI_DEVICES: bool = cfg!(not(target_os = "android"));
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
    let rect = Rect::from_center_size(area.center(), vec2(width, m.u(SIZE[1])));
    let (left, right) = if midi {
        rect.split_left_right_at_fraction(0.5)
    } else {
        (rect, rect)
    };
    let rows = |column: Rect| {
        move |(top, h): (f32, f32)| {
            Rect::from_x_y_ranges(
                column.left() + m.u(PAD)..=column.right() - m.u(PAD),
                rect.top() + m.u(top)..=rect.top() + m.u(top + h),
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
    let mut hue = ch.hue;
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
            } else if ui.memory(|mem| mem.focused().is_none()) {
                response.request_focus();
            }

            let ink = channel_color(hue, theme).to_color32();
            let control = Control {
                menu: ParamMenu::Off,
                bipolar: false,
                ..Control::new(id.with("color"), row(SLIDER), "Channel Color", ink, hue)
            };
            h_slider(ui, &app.cx, &control, &mut hue, |_| String::new());
            if midi {
                let colors = &theme.colors;
                let contrast = colors
                    .best_contrasting(channel_color(hue, theme), &colors.contrast_candidates());
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
    let mut edited = hue != ch.hue || midi_edited;
    app.midi_rescan |= rescan;
    ch.hue = hue;
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
