//! The THEME CREATOR page of the main menu (#67; `CMainMenu_ThemeCreator`, ui-structure.md §5.2). SETTINGS' `Theme
//! creator` `Edit` opens it; the SETTINGS tab, which stays underlined, goes back. One scrolling column:
//! - the header `THEME CREATOR`;
//! - a preview of the app in the theme being made: a `controlFill` bar with an `accent` button, a `moduleFill` rack card
//!   and playlist clips in three channel colours, a `menuFill` menu with its current item on `accent`, and keys (the
//!   last ones in the disabled colours). The app itself takes the theme as it changes, undimmed beside the panel;
//! - `Theme name` and its field (`Enter theme name`);
//! - `Load themes` (a menu of the built-in themes, then the user's), `Save` and `Reset`; then `Import` and `Export`;
//! - a row per colour role, keyed by its theme JSON key: its name, `RRGGBB` and a swatch. A tap opens its picker under
//!   it: a saturation (across) and value (up) field and a hue bar, both dragged, and a hex field; a tap closes it;
//! - `Channel color modifiers` (`Saturation`, `Lightness`) and `Playlist tracks` (`Background transparency`,
//!   `Note brightness`), each a 0–100 % slider over its theme option.
//!
//! Every change applies at once: the theme becomes the one in use (`Settings::custom_theme`), named after the field
//! (`Custom` while it is empty). Until it is saved, SETTINGS' `Theme:` warns before a pick replaces it
//! ([`ThemeCreator::unsaved`]). `Reset` goes back to the theme in use when the page opened, or the one `Load themes` or
//! `Import` loaded since. `Save` and `Export` need a name; the main menu's caller writes the file
//! ([`MainMenuAction::SaveTheme`], [`MainMenuAction::ExportTheme`]) and picks the one `Import` reads
//! ([`MainMenuAction::ImportTheme`], [`super::MainMenu::theme_imported`]).

use eframe::egui::ecolor::{hsv_from_rgb, rgb_from_hsv};
use eframe::egui::{
    Align2, Id, Mesh, Painter, Pos2, Rangef, Rect, Sense, Shape, Stroke, StrokeKind, TextEdit, Ui,
    lerp, pos2, vec2,
};

use super::{MainMenuAction, light_button, scroll, text_field};
use crate::metrics::FontSize;
use crate::settings::Settings;
use crate::theme::{
    MIN_NOTE_BRIGHTNESS, Rgba, Theme, ThemeColors, ThemeOptions, USER_THEME_ID, builtin_themes,
    channel_color, theme_by_id,
};
use crate::ui::UiCtx;
use crate::ui::popup::{self, PopupItem};
use crate::ui::scroller::Scroller;
use crate::ui::widgets::{Control, ParamMenu, h_slider, text_label};

// Sizes in U.
const HEADER_H: f32 = 1.5;
const PREVIEW_H: f32 = 7.0;
/// The preview clips' name strip, as the playlist's.
const CLIP_NAME_H: f32 = 0.7;
const GAP: f32 = 0.25;
const BUTTON_H: f32 = 1.0;
/// A colour row's pitch; its swatch, and the room of its hex text.
const ROW_H: f32 = 1.5;
const SWATCH_W: f32 = 2.0;
const HEX_W: f32 = 2.5;
/// The `Theme name` label's room before its field.
const NAME_LABEL_W: f32 = 5.0;
/// The picker: the saturation / value field, the hue bar and the hex field.
const FIELD_H: f32 = 4.0;
const HUE_H: f32 = 1.0;
const HEX_FIELD_H: f32 = 1.0;
/// A group header, and an option row (a label line over its slider).
const GROUP_H: f32 = 1.5;
const OPTION_H: f32 = 2.0;
/// The swatch's outline: `text` at this alpha.
const OUTLINE_ALPHA: f32 = 0.5;

/// The name of the theme while `Theme name` is empty.
const CUSTOM: &str = "Custom";
/// `Save` or `Export` with an empty `Theme name`.
pub const NAME_FIRST: &str = "Enter theme name";
/// The `Load themes` menu's value of the first user theme; the built-ins' are their ids.
const USER_THEMES: usize = 1000;

/// The colour rows, in order: the label and the role.
const ROLES: [&str; 11] = [
    "Accent",
    "Background",
    "Text",
    "Text on accent",
    "Control fill",
    "Menu fill",
    "Module fill",
    "White key",
    "Black key",
    "White key disabled",
    "Black key disabled",
];

fn role(c: &mut ThemeColors, i: usize) -> &mut Rgba {
    match i {
        0 => &mut c.accent,
        1 => &mut c.background,
        2 => &mut c.text,
        3 => &mut c.text_on_accent,
        4 => &mut c.control_fill,
        5 => &mut c.menu_fill,
        6 => &mut c.module_fill,
        7 => &mut c.white_key,
        8 => &mut c.black_key,
        9 => &mut c.white_key_disabled,
        _ => &mut c.black_key_disabled,
    }
}

/// The option groups: the header, then each slider's label and its option's default.
const GROUPS: [(&str, [(&str, f32); 2]); 2] = [
    (
        "Channel color modifiers",
        [("Saturation", 0.9), ("Lightness", 0.63)],
    ),
    (
        "Playlist tracks",
        [("Background transparency", 1.0), ("Note brightness", 0.65)],
    ),
];

fn option(o: &mut ThemeOptions, group: usize, i: usize) -> &mut f32 {
    match (group, i) {
        (0, 0) => &mut o.accent_saturation,
        (0, _) => &mut o.accent_lightness,
        (_, 0) => &mut o.playlist_track_background_transparency,
        _ => &mut o.playlist_track_note_brightness,
    }
}

/// The page's state: the theme being made, the name field, what `Reset` goes back to, the open picker, the open
/// `Load themes` menu and the scroll.
#[derive(Default)]
pub struct ThemeCreator {
    /// `None` until the page first shows ([`Self::open`]).
    work: Option<Theme>,
    name: String,
    /// The theme `Reset` goes back to, with the `theme` and `customTheme` settings that name it.
    base: Option<(Theme, u32, String)>,
    /// The colour row whose picker is open, and its hue, saturation and value: kept, as a gray has no hue of its own.
    picker: Option<(usize, [f32; 3])>,
    /// The picker's hex field.
    hex: String,
    /// `Load themes`' menu, open at this point.
    load_menu: Option<Pos2>,
    /// The theme JSON the last edit put in use, until `Save`, a load or `Reset` ([`Self::unsaved`]).
    edited: Option<String>,
    offset: f32,
    kinetic: Scroller,
    content_h: f32,
}

impl ThemeCreator {
    /// Starts over on `theme`, the one in use, which `s` names: `Reset` comes back to it. A user theme brings its name.
    pub fn open(&mut self, theme: &Theme, s: &Settings) {
        self.name = match s.custom_theme.is_empty() {
            true => String::new(),
            false => theme.name.clone(),
        };
        self.work = Some(theme.clone());
        self.base = Some((theme.clone(), s.theme, s.custom_theme.clone()));
        (self.picker, self.load_menu, self.offset, self.edited) = (None, None, 0.0, None);
    }

    /// The theme in use is this page's work, edited since it was last saved or loaded: SETTINGS' `Theme:` warns
    /// before a pick replaces it.
    pub fn unsaved(&self, s: &Settings) -> bool {
        self.edited.as_ref() == Some(&s.custom_theme)
    }

    /// Puts `theme` in use (a `builtin` one by its id) and starts over on it.
    pub fn load(&mut self, theme: Theme, builtin: bool, s: &mut Settings) {
        match builtin {
            true => (s.theme, s.custom_theme) = (theme.id, String::new()),
            false => s.custom_theme = theme.to_json(),
        }
        self.open(&theme, s);
    }

    /// The theme being made, named, as the one in use.
    fn apply(&mut self, s: &mut Settings) -> Theme {
        let name = match self.name.trim() {
            "" => CUSTOM,
            name => name,
        };
        let work = self.work.get_or_insert_with(|| s.theme());
        (work.id, work.name) = (USER_THEME_ID, name.to_owned());
        s.custom_theme = work.to_json();
        work.clone()
    }

    /// Draws the page in `area` and applies its edits to `s`. `user_themes` are what `Load themes` lists after the
    /// built-ins.
    pub(super) fn show(
        &mut self,
        ui: &mut Ui,
        cx: &UiCtx,
        area: Rect,
        s: &mut Settings,
        user_themes: &[Theme],
    ) -> Option<MainMenuAction> {
        if self.work.is_none() {
            self.open(cx.theme, s);
        }
        let m = cx.m;
        let text = cx.theme.colors.text.to_color32();
        let id = Id::new("theme_creator");
        let offset = scroll(
            ui,
            id.with("scroll"),
            m.u(1.0),
            &mut self.kinetic,
            area,
            self.content_h,
            &mut self.offset,
        );
        let outer_clip = ui.clip_rect();
        ui.set_clip_rect(outer_clip.intersect(area));
        let top = area.top() - offset;
        let mut y = top;
        let mut band = |h: f32| {
            let r = Rect::from_x_y_ranges(area.x_range(), y..=y + m.u(h));
            y += m.u(h);
            r
        };
        let mut action = None;
        let mut edited = false;
        let label = |ui: &Ui, rect: Rect, s: &str, font| {
            text_label(ui, cx, rect, s, font, Align2::LEFT_CENTER, text);
        };

        label(ui, band(HEADER_H), "THEME CREATOR", FontSize::Pt18);
        let work = self.work.as_ref().expect("opened above");
        preview(ui.painter(), cx, band(PREVIEW_H), work);
        band(GAP);

        // The name, and the buttons.
        let row = band(BUTTON_H);
        let (room, field) = row.split_left_right_at_x(row.left() + m.u(NAME_LABEL_W));
        label(ui, room, "Theme name", FontSize::Pt10);
        let edit = TextEdit::singleline(&mut self.name)
            .id(id.with("name"))
            .char_limit(64);
        let mut response = text_field(ui, cx, field, NAME_FIRST, edit);
        crate::android_text::edit(&mut response, &mut self.name, 64);
        band(GAP);
        let mut tapped = None;
        for names in [&["Load themes", "Save", "Reset"][..], &["Import", "Export"]] {
            let row = band(BUTTON_H);
            band(GAP);
            let n = names.len() as f32;
            let w = (row.width() - (n - 1.0) * m.u(GAP)) / n;
            for (i, name) in names.iter().enumerate() {
                let left = row.left() + i as f32 * (w + m.u(GAP));
                let rect = Rect::from_x_y_ranges(left..=left + w, row.y_range());
                if light_button(ui, cx, id.with(*name), rect, name, true) {
                    tapped = Some((*name, rect));
                }
            }
        }
        match tapped {
            Some(("Load themes", rect)) => self.load_menu = Some(rect.center()),
            Some(("Save" | "Export", _)) if self.name.trim().is_empty() => {
                action = Some(MainMenuAction::Message(NAME_FIRST.into()));
            }
            Some(("Save", _)) => {
                self.edited = None;
                action = Some(MainMenuAction::SaveTheme(self.apply(s)));
            }
            Some(("Export", _)) => action = Some(MainMenuAction::ExportTheme(self.apply(s))),
            Some(("Import", _)) => action = Some(MainMenuAction::ImportTheme),
            Some(_) => {
                if let Some((theme, builtin, custom)) = self.base.clone() {
                    (s.theme, s.custom_theme) = (builtin, custom);
                    self.open(&theme, s);
                }
            }
            None => {}
        }

        // The colour rows, and the open one's picker.
        let colors = &mut self.work.as_mut().expect("opened above").colors;
        for (i, name) in ROLES.into_iter().enumerate() {
            let row = band(ROW_H);
            let line = Rect::from_center_size(row.center(), vec2(row.width(), m.u(1.0)));
            if ui
                .interact(row, id.with(("role", i)), Sense::click())
                .clicked()
            {
                self.picker = match self.picker {
                    Some((open, _)) if open == i => None,
                    _ => {
                        let c = *role(colors, i);
                        let (h, s, v) = hsv_from_rgb([c.r, c.g, c.b]);
                        self.hex = c.to_hex();
                        Some((i, [h, s, v]))
                    }
                };
            }
            let color = *role(colors, i);
            let swatch = line.with_min_x(line.right() - m.u(SWATCH_W));
            ui.painter().rect_filled(swatch, 0.0, color.to_color32());
            let outline = Stroke::new(1.0 / m.ppp(), text.gamma_multiply(OUTLINE_ALPHA));
            (ui.painter()).rect_stroke(swatch, 0.0, outline, StrokeKind::Inside);
            let hex = line.with_max_x(swatch.left() - m.u(2.0 * GAP));
            let hex = hex.with_min_x(hex.right() - m.u(HEX_W));
            let right = Align2::RIGHT_CENTER;
            let font = FontSize::Pt10;
            text_label(ui, cx, hex, &color.to_hex(), font, right, text);
            label(ui, line, name, font);
            let Some((_, hsv)) = self.picker.as_mut().filter(|(open, _)| *open == i) else {
                continue;
            };
            let field = band(FIELD_H);
            band(GAP);
            let hue = band(HUE_H);
            band(GAP);
            let hex_row = band(HEX_FIELD_H);
            band(GAP);
            let mut dragged = false;
            let pick = ui.interact(field, id.with("field"), Sense::click_and_drag());
            if let Some(p) = pick.interact_pointer_pos() {
                hsv[1] = ((p.x - field.left()) / field.width()).clamp(0.0, 1.0);
                hsv[2] = 1.0 - ((p.y - field.top()) / field.height()).clamp(0.0, 1.0);
                dragged = true;
            }
            let bar = ui.interact(hue, id.with("hue"), Sense::click_and_drag());
            if let Some(p) = bar.interact_pointer_pos() {
                hsv[0] = ((p.x - hue.left()) / hue.width()).clamp(0.0, 1.0);
                dragged = true;
            }
            let field_w = m.u(HEX_W + SWATCH_W + GAP);
            let hex_field = hex_row.with_min_x(hex_row.right() - field_w);
            label(ui, hex_row, "Hex", font);
            let edit = TextEdit::singleline(&mut self.hex)
                .id(id.with("hex"))
                .char_limit(7);
            let mut response = text_field(ui, cx, hex_field, "RRGGBB", edit);
            crate::android_text::edit(&mut response, &mut self.hex, 7);
            if dragged {
                let [r, g, b] = rgb_from_hsv((hsv[0], hsv[1], hsv[2]));
                let new = Rgba::rgb(r, g, b);
                if new != color {
                    *role(colors, i) = new;
                    self.hex = new.to_hex();
                    edited = true;
                }
            } else if response.changed()
                && let Some(new) = Rgba::from_hex_str(self.hex.trim())
                && new != color
            {
                let (h, s, v) = hsv_from_rgb([new.r, new.g, new.b]);
                *hsv = [h, s, v];
                *role(colors, i) = new;
                edited = true;
            }
            hsv_field(ui.painter(), cx, field, hue, *hsv);
        }

        // The option groups.
        let options = &mut self.work.as_mut().expect("opened above").options;
        for (g, (header, sliders)) in GROUPS.into_iter().enumerate() {
            label(ui, band(GROUP_H), header, FontSize::Pt12);
            for (i, (name, default)) in sliders.into_iter().enumerate() {
                let row = band(OPTION_H);
                let (line, body) = row.split_top_bottom_at_y(row.top() + m.u(1.0));
                let value = option(options, g, i);
                let percent = format!("{:.0}%", *value * 100.0);
                label(ui, line, name, FontSize::Pt10);
                let right = Align2::RIGHT_CENTER;
                text_label(ui, cx, line, &percent, FontSize::Pt10, right, text);
                let c = Control {
                    bipolar: false,
                    menu: ParamMenu::Unbound,
                    ..Control::new(id.with(("option", g, i)), body, "", text, default)
                };
                let before = *value;
                h_slider(ui, cx, &c, value, |_| String::new());
                // The muted-clip tint divides by the note brightness.
                if (g, i) == (1, 1) {
                    *value = value.max(MIN_NOTE_BRIGHTNESS);
                }
                edited |= *value != before;
            }
        }
        band(GAP);
        self.content_h = y - top;
        ui.set_clip_rect(outer_clip);
        if edited {
            self.apply(s);
            self.edited = Some(s.custom_theme.clone());
        }

        // `Load themes`: the built-in themes, then the user's.
        if let Some(at) = self.load_menu {
            let builtins = builtin_themes()
                .into_iter()
                .map(|t| (t.id as usize, t.name));
            let users =
                (user_themes.iter().enumerate()).map(|(i, t)| (USER_THEMES + i, t.name.clone()));
            let items: Vec<PopupItem<usize>> = (builtins.chain(users).enumerate())
                .map(|(row, (value, name))| PopupItem::new(value, 0, row, name))
                .collect();
            let ctx = ui.ctx().clone();
            let result = popup::show(&ctx, id.with("load"), at, ctx.content_rect(), &items, cx);
            if let Some(value) = result.selected {
                match value.checked_sub(USER_THEMES) {
                    Some(i) => {
                        if let Some(theme) = user_themes.get(i) {
                            self.load(theme.clone(), false, s);
                        }
                    }
                    None => self.load(theme_by_id(value as u32), true, s),
                }
            }
            if result.close {
                self.load_menu = None;
            }
        }
        action
    }
}

/// The picker's saturation / value `field` at hue `hsv[0]` and its `hue` bar, each marked where `hsv` is: a `text` ring
/// in a `background` one, a `text` bar in a `background` outline.
fn hsv_field(painter: &Painter, cx: &UiCtx, field: Rect, hue: Rect, hsv: [f32; 3]) {
    const N: u32 = 8;
    let at = |h: f32, s: f32, v: f32| {
        let [r, g, b] = rgb_from_hsv((h, s, v));
        Rgba::rgb(r, g, b).to_color32()
    };
    let mut mesh = Mesh::default();
    for yi in 0..=N {
        for xi in 0..=N {
            let (s, v) = (xi as f32 / N as f32, 1.0 - yi as f32 / N as f32);
            let p = pos2(lerp(field.x_range(), s), lerp(field.y_range(), 1.0 - v));
            mesh.colored_vertex(p, at(hsv[0], s, v));
            if xi < N && yi < N {
                let tl = yi * (N + 1) + xi;
                mesh.add_triangle(tl, tl + 1, tl + N + 1);
                mesh.add_triangle(tl + 1, tl + N + 1, tl + N + 2);
            }
        }
    }
    // The hue bar: exact between its 7 stops.
    let base = mesh.vertices.len() as u32;
    for k in 0..=6 {
        let h = k as f32 / 6.0;
        let x = lerp(hue.x_range(), h);
        mesh.colored_vertex(pos2(x, hue.top()), at(h, 1.0, 1.0));
        mesh.colored_vertex(pos2(x, hue.bottom()), at(h, 1.0, 1.0));
        if k < 6 {
            let i = base + 2 * k;
            mesh.add_triangle(i, i + 1, i + 2);
            mesh.add_triangle(i + 1, i + 2, i + 3);
        }
    }
    painter.add(Shape::mesh(mesh));
    let c = &cx.theme.colors;
    let (ink, rim) = (c.text.to_color32(), c.background.to_color32());
    let px = 1.0 / cx.m.ppp();
    let dot = pos2(
        lerp(field.x_range(), hsv[1]),
        lerp(field.y_range(), 1.0 - hsv[2]),
    );
    let r = cx.m.u(0.25);
    painter.circle_stroke(dot, r, Stroke::new(3.0 * px, rim));
    painter.circle_stroke(dot, r, Stroke::new(1.5 * px, ink));
    let x = lerp(hue.x_range(), hsv[0]);
    let mark = Rect::from_center_size(pos2(x, hue.center().y), vec2(cx.m.u(0.2), hue.height()));
    painter.rect_filled(mark, 0.0, ink);
    painter.rect_stroke(mark, 0.0, Stroke::new(px, rim), StrokeKind::Outside);
}

/// A small picture of the app in `t`, filling `r`: a `controlFill` bar with the song name and an `accent` `PLAY`
/// button (`textOnAccent`), a `moduleFill` rack card with three channels, their playlist clips (the channel colour at
/// the track background transparency, the name and notes in the playlist's content colour, `clips::tint`), a `menuFill` menu with its current item on
/// `accent`, and two octaves of keys, the top five in the disabled colours.
fn preview(painter: &Painter, cx: &UiCtx, r: Rect, t: &Theme) {
    let (m, c, o) = (cx.m, &t.colors, &t.options);
    let u = m.u(1.0);
    let ink = |x: Rgba| x.to_color32();
    let font = |size| m.font_id(size);
    painter.rect_filled(r, 0.0, ink(c.background));

    let bar = r.with_max_y(r.top() + u);
    painter.rect_filled(bar, 0.0, ink(c.control_fill));
    let left = Align2::LEFT_CENTER;
    let name = bar.left_center() + vec2(0.3 * u, 0.0);
    painter.text(name, left, "My Song", font(FontSize::Pt10), ink(c.text));
    let play = Rect::from_min_max(
        pos2(bar.right() - 3.0 * u, bar.top() + 0.15 * u),
        pos2(bar.right() - 0.15 * u, bar.bottom() - 0.15 * u),
    );
    painter.rect_filled(play, 0.15 * u, ink(c.accent));
    let center = Align2::CENTER_CENTER;
    painter.text(
        play.center(),
        center,
        "PLAY",
        font(FontSize::Pt8),
        ink(c.text_on_accent),
    );

    let keys = r.with_min_y(r.bottom() - 1.5 * u);
    let body = Rect::from_x_y_ranges(r.x_range(), bar.bottom()..=keys.top()).shrink(0.25 * u);
    let card = body.with_max_x(body.left() + body.width() * 0.25);
    let menu = body.with_min_x(body.right() - body.width() * 0.3);
    let lanes = Rect::from_x_y_ranges(
        card.right() + 0.25 * u..=menu.left() - 0.25 * u,
        body.y_range(),
    );
    painter.rect_filled(card, 0.0, ink(c.module_fill));
    let lane_h = body.height() / 3.0;
    for (i, (h, channel)) in [(0.1, "Kick"), (0.45, "Bass"), (0.8, "Keys")]
        .into_iter()
        .enumerate()
    {
        let channel_ink = channel_color(h, t);
        let band = |x: Rangef| {
            let top = body.top() + i as f32 * lane_h;
            Rect::from_x_y_ranges(x, top..=top + lane_h).shrink2(vec2(0.0, 0.1 * u))
        };
        let row = band(card.x_range()).shrink2(vec2(0.15 * u, 0.0));
        painter.rect_filled(row.with_max_x(row.left() + 0.2 * u), 0.0, ink(channel_ink));
        let at = row.left_center() + vec2(0.45 * u, 0.0);
        painter.text(at, left, channel, font(FontSize::Pt8), ink(c.text));
        let start = lanes.left() + i as f32 * 0.15 * lanes.width();
        let clip = band(Rangef::new(start, start + 0.6 * lanes.width()));
        // As the playlist draws a clip: the body at the track background transparency, then the name and the notes
        // in the content colour, which a low contrast between the two shows here as it will there.
        let a = o.playlist_track_background_transparency.clamp(0.0, 1.0);
        painter.rect_filled(clip, 0.0, Rgba { a, ..channel_ink }.to_color32());
        let note = ink(crate::ui::clips::tint(t, channel_ink).0);
        let (name, notes) = clip.split_top_bottom_at_y(clip.top() + CLIP_NAME_H * u);
        let at = name.left_center() + vec2(0.15 * u, 0.0);
        painter.text(at, left, channel, font(FontSize::Pt8), note);
        for k in 0..4 {
            let x = notes.left() + (0.1 + 0.22 * k as f32) * notes.width();
            let y = notes.top() + (0.1 + 0.2 * ((k * 2 + i) % 4) as f32) * notes.height();
            let n = Rect::from_min_size(
                pos2(x, y),
                vec2(0.15 * notes.width(), 0.15 * notes.height()),
            );
            painter.rect_filled(n, 0.0, note);
        }
    }
    painter.rect_filled(menu, 0.0, ink(c.menu_fill));
    let item_h = menu.height() / 3.0;
    for (k, item) in ["Clone", "Rename", "Delete"].into_iter().enumerate() {
        let top = menu.top() + k as f32 * item_h;
        let row = Rect::from_x_y_ranges(menu.x_range(), top..=top + item_h);
        let fg = if k == 1 {
            painter.rect_filled(row, 0.0, ink(c.accent));
            c.text_on_accent
        } else {
            c.text
        };
        let at = row.left_center() + vec2(0.3 * u, 0.0);
        painter.text(at, left, item, font(FontSize::Pt8), ink(fg));
    }

    // Two octaves of white keys with their black keys; the last five white ones are out of range.
    const WHITE: usize = 14;
    let w = keys.width() / WHITE as f32;
    let px = 1.0 / m.ppp();
    let disabled = |k: usize| k >= WHITE - 5;
    for k in 0..WHITE {
        let key = Rect::from_x_y_ranges(
            keys.left() + k as f32 * w..=keys.left() + (k + 1) as f32 * w - px,
            keys.y_range(),
        );
        let fill = if disabled(k) {
            c.white_key_disabled
        } else {
            c.white_key
        };
        painter.rect_filled(key, 0.0, ink(fill));
    }
    for k in 0..WHITE - 1 {
        // No black key after E and B.
        if matches!(k % 7, 2 | 6) {
            continue;
        }
        let x = keys.left() + (k + 1) as f32 * w;
        let key = Rect::from_min_size(
            pos2(x - 0.3 * w, keys.top()),
            vec2(0.6 * w, 0.6 * keys.height()),
        );
        let fill = if disabled(k + 1) {
            c.black_key_disabled
        } else {
            c.black_key
        };
        painter.rect_filled(key, 0.0, ink(fill));
    }
}
