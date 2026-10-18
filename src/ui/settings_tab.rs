//! The SETTINGS tab of the main menu: one scrolling column of section headers and rows, editing a [`Settings`].
//!
//! The row pitch is 2 U, a control row 1.5 U (centred in its pitch), and the content column is the page's left + 2 U,
//! wide W - 4 U (the page is the menu panel: the tab hands this its panel-wide page). The list scrolls in whole rows.
//! Every toggle is the label-right pill in a rect that starts 0.5 U left of the column and ends at its right edge: the
//! rect is a `menuFill` bar (x 1.15 while pressed), the pill is centred 0.75 U in from its left, and the label runs
//! from 1.7 U to 0.2 U short of the right edge, 10 pt, always in `text`, drawn plain (it overflows when long). The
//! whole rect is the control, so a tap on the label flips the toggle too. The latency toggle is 2.5 U shorter, and
//! `Calibrate`, 2.5 U wide, takes the right end of its row. The bar is the colour of the menu body (`menuFill`), so it
//! is invisible at rest and shows only while pressed; the pill's on-dot is `text` (the root context colour, nothing in
//! the menu sets another) and its off-dot `menuFill`. The full-width buttons (`Share logs`, `Restore demo songs`,
//! `Restore default settings`, ...) are `text` fills, the light ones; `Scan Bluetooth MIDI Devices` is a `menuFill`
//! fill, so it reads as plain text with an icon.
//!
//! Headers are 12 pt text (the popup's section font); wide light buttons are 6 × 1 U (at most half the column) and
//! full-width ones 1 U tall; the metronome row is a 1 U label line with the level in percent on the right and the
//! slider under it. Rows the app cannot use yet (MIDI, Bluetooth, nearby sharing, `Share logs`, cloud backup) are drawn
//! at the disabled 30 % alpha and take no input. The slider's colour is `text`, the root context colour. Any other
//! label has the room left of its control, and uses shrink drawing in it (down a point at a time to 6 pt, then a
//! middle ellipsis keeping 5 tail characters): plain drawing would run it under its control at large UI scales on a
//! narrow screen. Where the label fits, as on the 20 U panel, it is drawn whole at its own size. There is no scroll
//! bar: the list scrolls by wheel or drag.
//!
//! `Input device:` is live: its popup is the MIDI device [`menu`] over the ports the app lends the tab
//! ([`SettingsTab::set_midi_ports`]), the pick is stored as the menu label in `Settings::midi_device`, and opening the
//! dropdown asks the app, once, to list the ports again ([`SettingsTab::take_midi_rescan`]). The button shows the stored
//! name even when its device is not connected, shrunk into the button as a device name can be any length.
//!
//! `Restore demo songs` asks the app to write our demo songs into the songs folder again
//! ([`SettingsTab::take_restore_demos`]), with no question first.
//!
//! `Sound credits` is a full-width light button: the main menu opens the credits screen ([`SettingsTab::take_credits`],
//! `views/credits.rs`).
//!
//! The list ends with `Original content:` and a row with the Original Content folder's path
//! ([`crate::project_files::CONTENT_FOLDER`], shortened in the middle like a label) and an `Open folder` light button,
//! which asks the app to show the folder in the system's file manager ([`SettingsTab::take_reveal`]). They show only
//! where there is such a folder ([`SettingsTab::content`]): on the desktop, not on the web.

use std::path::PathBuf;

use eframe::egui::{
    Align2, Color32, Id, Pos2, Rect, Sense, Shape, Ui, UiBuilder, WidgetInfo, WidgetType, vec2,
};

use crate::icons::{IconId, draw_icon};
use crate::metrics::{FontSize, UI_SCALE_FACTORS, ui_scale_menu_order};
use crate::midi::device::menu as midi_menu;
use crate::settings::{SOUND_MODES, Settings};
use crate::theme::{DISABLED_ALPHA, builtin_themes};
use crate::ui::popup::{self, PopupItem};
use crate::ui::widgets::{
    Button, Control, ParamMenu, Pill, button, h_slider, pill_toggle, pressed, shrink,
    truncated_text,
};
use crate::ui::{UiCtx, primary_down_live};

// Sizes in U (see the module docs).
/// Every row takes this much of the list, and scrolling moves by it.
const ROW_PITCH: f32 = 2.0;
/// A control row's height, centred in its pitch.
const ROW_H: f32 = 1.5;
/// The column is this far in from each side of the page.
const COLUMN_INSET: f32 = 2.0;
/// A toggle's rect starts this far left of the column.
const TOGGLE_OUT: f32 = 0.5;
/// The latency toggle is this much shorter, for `Calibrate`, which is this wide.
const LATENCY_SHORTER: f32 = 2.5;
const CALIBRATE_W: f32 = 2.5;
/// The metronome row's label line; the slider takes the rest.
const LEVEL_LABEL_H: f32 = 1.0;
/// The gap between a label and its control.
const MARGIN: f32 = 0.5;
const BUTTON_W: f32 = 6.0;
const BUTTON_H: f32 = 1.0;
/// The room kept on each side of a device name inside its button.
const BUTTON_PAD: f32 = 0.5;
/// The Bluetooth icon's size factor (an edge of 1 U) and the room it takes before the label.
const SCAN_ICON: f32 = 0.75;
const SCAN_ICON_W: f32 = 1.5;
/// The metronome level's room at the right end of its label line, wider than `100%`.
const PERCENT_W: f32 = 2.0;
/// The content folder row's button.
pub const OPEN_FOLDER: &str = "Open folder";
/// The rows at the end that show only with a content folder.
const CONTENT_ROWS: usize = 2;

/// What a row shows and edits.
#[derive(Clone, Copy)]
enum Kind {
    /// A section header.
    Header,
    /// A toggle bound to a setting.
    Toggle(fn(&mut Settings) -> &mut bool),
    /// A disabled toggle fixed in this state.
    FixedToggle(bool),
    /// `Enable audio recording latency compensation`: a toggle of the stored `Option<bool>`, shown ON while unset.
    Latency,
    /// A wide light button showing the current item; a tap opens the popup of items.
    Dropdown(Choice),
    /// `Input device:`: a dropdown over the MIDI ports, showing the stored name (shrunk to fit).
    MidiDropdown,
    /// A disabled wide light button with this text.
    FixedDropdown(&'static str),
    /// The Bluetooth icon and the label, disabled.
    Scan,
    /// The metronome level: the label line and a full-width slider.
    Level,
    /// A disabled full-width light button.
    DisabledButton,
    /// `Restore default settings`: a full-width light button asking Yes / No before it resets.
    Restore,
    /// `Restore demo songs`: a full-width light button the app answers ([`SettingsTab::take_restore_demos`]).
    RestoreDemos,
    /// The label and a disabled `Sign In` button.
    SignIn,
    /// The content folder's path (in place of the label) and the `Open folder` button.
    ContentFolder,
    /// `Sound credits`: a full-width light button that opens the credits screen ([`SettingsTab::take_credits`]).
    Credits,
    /// The plugin folders the user added (the label) and `Add folder` ([`SettingsTab::take_plugin_action`]).
    PluginFolders,
    /// The scan's state (the label) and `Rescan`, which scans every plugin file again.
    PluginScan,
    /// `Forget my plugin folders`: a full-width light button, enabled while there are some.
    ForgetPluginFolders,
}

/// What the plugin rows asked the app for ([`SettingsTab::take_plugin_action`]).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum PluginAction {
    /// Pick a folder to add, then scan.
    AddFolder,
    /// Scan every plugin file again.
    Rescan,
    /// The user's folders were forgotten: scan again without them.
    Forgot,
}

/// The plugin rows, before `Advanced:`, where plugins are hosted ([`SettingsTab::plugins`]).
const PLUGIN_ROWS: [Row; 4] = [
    row("Plugins (VST3, VST2, CLAP):", Kind::Header),
    row("Plugin folders", Kind::PluginFolders),
    row("Plugins", Kind::PluginScan),
    row("Forget my plugin folders", Kind::ForgetPluginFolders),
];

/// The dropdown rows.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Choice {
    SoundMode,
    Theme,
    Scaling,
}

struct Row {
    label: &'static str,
    kind: Kind,
}

const fn row(label: &'static str, kind: Kind) -> Row {
    Row { label, kind }
}

/// Every row of the SETTINGS tab, in order.
const ROWS: [Row; 36] = {
    use Kind::*;
    [
        row("MIDI:", Header),
        row(
            "Input device:",
            if cfg!(target_os = "android") {
                FixedDropdown("Not available")
            } else {
                MidiDropdown
            },
        ),
        row("MIDI driver:", FixedDropdown("Standard")),
        row("Scan Bluetooth MIDI Devices", Scan),
        row("Audio:", Header),
        row("Multi core processing", Toggle(|s| &mut s.multi_core)),
        row("Enable low latency mode", Toggle(|s| &mut s.low_latency)),
        row(
            "Enable background audio",
            Toggle(|s| &mut s.background_audio),
        ),
        row("Sound mode:", Dropdown(Choice::SoundMode)),
        row("Recording:", Header),
        row("Enable audio recording latency compensation", Latency),
        row(
            "Enable multitrack recording",
            Toggle(|s| &mut s.multitrack_recording),
        ),
        row(
            "Enable count-in when recording",
            Toggle(|s| &mut s.count_in),
        ),
        row("Metronome level", Level),
        row("User interface:", Header),
        row("Theme:", Dropdown(Choice::Theme)),
        row("Scaling:", Dropdown(Choice::Scaling)),
        row("Application:", Header),
        row("Enable ghost notes", Toggle(|s| &mut s.ghost_notes)),
        row(
            "Show key names on keyboard and piano roll",
            Toggle(|s| &mut s.key_names),
        ),
        row("Set middle C to C4", Toggle(|s| &mut s.middle_c_is_c4)),
        row(
            "Show range labels on keyboard",
            Toggle(|s| &mut s.range_labels),
        ),
        row(
            "Continuous scrolling during playback",
            Toggle(|s| &mut s.continuous_scroll),
        ),
        row(
            "Show CPU and RAM meters",
            Toggle(|s| &mut s.hardware_meters),
        ),
        row(
            "Enable sharing files with nearby devices",
            FixedToggle(false),
        ),
        row("Advanced:", Header),
        row("Share logs", DisabledButton),
        row("Restore demo songs", RestoreDemos),
        row("Restore default settings", Restore),
        row("Cloud backup:", Header),
        row("You are not signed in", SignIn),
        row("Sync automatically", FixedToggle(false)),
        row("Sync only when connected to WiFi", FixedToggle(true)),
        // The bundled sounds' sources.
        row("Sound credits", Credits),
        // The content folder ([`CONTENT_ROWS`]).
        row("Original content:", Header),
        row("Original content folder", ContentFolder),
    ]
};

/// The popups the tab opens.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Menu {
    Choice(Choice),
    /// The `Input device:` items: the MIDI device [`menu`], each carrying its index.
    Midi,
    /// The Yes / No confirm of `Restore default settings`.
    Restore,
}

/// The confirm's question, and the values its `Yes` and `No` items return.
const RESTORE_QUESTION: &str = "Are you sure you want to restore default settings?";
const CONFIRM_YES: usize = 1;
const CONFIRM_NO: usize = 0;
/// The tail a shortened device name keeps in the `Input device:` popup, as shrink drawing keeps on its button.
const DEVICE_TAIL_K: usize = 5;

/// The tab's own state: the open popup, if any, where it is centred, the last frame the tab was shown in, and how far
/// the list is scrolled.
#[derive(Default)]
pub struct SettingsTab {
    menu: Option<(Menu, Pos2)>,
    shown_frame: u64,
    /// How far the list is dragged or wheeled, in points. It is drawn at the whole row nearest to it.
    offset: f32,
    /// `Calibrate` was tapped: the app starts the latency calibration and clears it.
    pub calibrate: bool,
    /// The MIDI input ports the `Input device:` menu lists ([`Self::set_midi_ports`]).
    midi_ports: Vec<String>,
    /// The `Input device:` dropdown opened since [`Self::take_midi_rescan`].
    midi_rescan: bool,
    /// The Original Content folder the last rows show; `None` hides them. The caller keeps it current.
    pub content: Option<PathBuf>,
    /// `Open folder` was tapped, until [`Self::take_reveal`].
    reveal: bool,
    /// `Restore demo songs` was tapped, until [`Self::take_restore_demos`].
    restore_demos: bool,
    /// `Sound credits` was tapped, until [`Self::take_credits`].
    credits: bool,
    /// A plugin row's tap, until [`Self::take_plugin_action`].
    plugin_action: Option<PluginAction>,
    /// The plugin rows show: the app hosts plugins (the desktop). The caller sets it.
    pub plugins: bool,
}

impl SettingsTab {
    /// Lends the tab the MIDI input ports (`MidiLink::ports`) for the `Input device:` menu. Call it every frame
    /// before [`Self::show`]; a menu that is open follows the ports.
    pub fn set_midi_ports(&mut self, ports: &[String]) {
        ports.clone_into(&mut self.midi_ports);
    }

    /// Whether the `Input device:` dropdown opened since the last call, once per open: the app lists the ports again
    /// then (`MidiLink::rescan`), for the open dropdown to show devices plugged in since. The tap that opens it is
    /// also the user gesture the web needs to ask for MIDI access.
    pub fn take_midi_rescan(&mut self) -> bool {
        std::mem::take(&mut self.midi_rescan)
    }

    /// The folder to show in the file manager once `Open folder` was tapped, once.
    pub fn take_reveal(&mut self) -> Option<PathBuf> {
        std::mem::take(&mut self.reveal).then(|| self.content.clone())?
    }

    /// Whether `Restore demo songs` was tapped since the last call, once a tap: the app writes the demos.
    pub fn take_restore_demos(&mut self) -> bool {
        std::mem::take(&mut self.restore_demos)
    }

    /// Whether `Sound credits` was tapped since the last call, once a tap: the main menu opens the credits screen.
    pub fn take_credits(&mut self) -> bool {
        std::mem::take(&mut self.credits)
    }

    /// What a plugin row asked for since the last call, once a tap.
    pub fn take_plugin_action(&mut self) -> Option<PluginAction> {
        self.plugin_action.take()
    }

    /// The rows shown: all but the content folder's without one, and the plugin rows before `Advanced:` where plugins
    /// are hosted.
    fn rows(&self) -> Vec<&'static Row> {
        let rows = match self.content {
            Some(_) => &ROWS[..],
            None => &ROWS[..ROWS.len() - CONTENT_ROWS],
        };
        let plugins = if self.plugins { &PLUGIN_ROWS[..] } else { &[] };
        let at = (rows.iter().position(|r| r.label == "Advanced:")).unwrap_or(rows.len());
        (rows[..at].iter())
            .chain(plugins)
            .chain(&rows[at..])
            .collect()
    }

    /// Draws the tab in `page` (the menu panel's width, below its tab row) and applies the user's edits to `settings`;
    /// the app saves and applies what changed.
    pub fn show(&mut self, ui: &mut Ui, cx: &UiCtx, page: Rect, settings: &mut Settings) {
        // A popup left open when the tab was hidden (another tab, the menu closed) does not come back with it.
        let frame = ui.ctx().cumulative_frame_nr();
        if frame > self.shown_frame + 1 {
            self.menu = None;
        }
        self.shown_frame = frame;
        let m = cx.m;
        let rows = self.rows();
        let pitch = m.u(ROW_PITCH);
        // Drags and the wheel move the list; widgets that sense clicks let drags through to it. It scrolls in whole rows.
        let id = Id::new("settings_tab").with("scroll");
        let drag = ui.interact(page, id, Sense::drag());
        let mut moved = drag.drag_delta().y;
        if ui.rect_contains_pointer(page) {
            moved += ui.input(|i| i.smooth_scroll_delta.y);
        }
        let hidden_rows = ((rows.len() as f32 * pitch - page.height()) / pitch).ceil();
        self.offset = (self.offset - moved).clamp(0.0, hidden_rows.max(0.0) * pitch);
        let top = page.top() - (self.offset / pitch).round() * pitch;
        let column = page.shrink2(vec2(m.u(COLUMN_INSET), 0.0)).x_range();
        let outer_clip = ui.clip_rect();
        ui.set_clip_rect(outer_clip.intersect(page));
        for (index, row) in rows.iter().enumerate() {
            let y = top + index as f32 * pitch;
            let slot = Rect::from_x_y_ranges(column, y..=y + pitch);
            // The toggle's rect reaches out of the column: the page is what is visible.
            if ui.is_rect_visible(slot.expand2(vec2(m.u(TOGGLE_OUT), 0.0))) {
                self.row(ui, cx, index, row, slot, settings);
            }
        }
        ui.set_clip_rect(outer_clip);
        self.popup(ui, cx, settings);
    }

    /// Draws row `index` in its `slot` of the list (a pitch tall) and applies its edit.
    fn row(
        &mut self,
        ui: &mut Ui,
        cx: &UiCtx,
        index: usize,
        row: &Row,
        slot: Rect,
        s: &mut Settings,
    ) {
        let (m, colors) = (cx.m, &cx.theme.colors);
        // Headers and the metronome row take the whole slot; the other rows are a control row, centred in it.
        let r = match row.kind {
            Kind::Header | Kind::Level => slot,
            _ => Rect::from_center_size(slot.center(), vec2(slot.width(), m.u(ROW_H))),
        };
        let text = colors.text.to_color32();
        let id = row_id(index);
        let mut ui = ui.new_child(UiBuilder::new().max_rect(r));
        if row.kind.disabled() {
            ui.multiply_opacity(DISABLED_ALPHA);
        }
        let ui = &mut ui;
        // A control `w` × `h` U at the row's right end, and the label's room left of it.
        let right = |w: f32, h: f32| {
            let size = vec2(m.u(w), m.u(h));
            Rect::from_min_size(r.right_center() - vec2(size.x, size.y / 2.0), size)
        };
        let before = |control: Rect| r.with_max_x(control.left() - m.u(MARGIN));
        // The level's percent: plain text, which has the room it needs.
        let plain = |ui: &Ui, room: Rect, string: &str, font, align: Align2| {
            ui.painter().text(
                align.pos_in_rect(&room),
                align,
                string,
                m.font_id(font),
                text,
            );
        };
        // The label: shrink drawing in its room, so that it never runs under the control.
        let label = |ui: &Ui, room: Rect, font: FontSize| {
            let align = Align2::LEFT_CENTER;
            shrink(
                ui.painter(),
                m,
                room,
                align,
                row.label,
                font.points() as u8,
                text,
            );
        };
        let light_button = |ui: &mut Ui, rect: Rect, label: &str, enabled: bool| {
            let b = Button {
                disabled: !enabled,
                ..Button::new(id, rect, label, false, colors)
            };
            button(ui, cx, &b, true).clicked()
        };
        // At most half the row, so that the label keeps room at large scales.
        let button_rect = right(BUTTON_W.min(r.width() / 2.0 / m.u(1.0)), BUTTON_H);
        match row.kind {
            Kind::Header => label(ui, r, FontSize::Pt12),
            Kind::Toggle(_) | Kind::FixedToggle(_) | Kind::Latency => {
                // The rect starts 0.5 U left of the column and ends at its right edge; the latency one is 2.5 U
                // shorter, for `Calibrate`, which is 2.5 U wide at the right end, only while the toggle is OFF
                // (compensating).
                let latency = matches!(row.kind, Kind::Latency);
                let shorter = if latency { LATENCY_SHORTER } else { 0.0 };
                let rect = Rect::from_x_y_ranges(
                    r.left() - m.u(TOGGLE_OUT)..=r.right() - m.u(shorter),
                    r.y_range(),
                );
                if latency && s.latency_compensation == Some(false) {
                    let at = right(CALIBRATE_W, BUTTON_H);
                    let b = Button::new(id.with("calibrate"), at, "Calibrate", false, colors);
                    self.calibrate |= button(ui, cx, &b, !cfg!(target_os = "android")).clicked();
                }
                let (on, field) = match row.kind {
                    Kind::Toggle(field) => (*field(s), Some(field)),
                    Kind::FixedToggle(on) => (on, None),
                    Kind::Latency => (s.latency_compensation != Some(false), None),
                    _ => unreachable!("a toggle row"),
                };
                // Settings are not parameters: no parameter menu. The label is the pill's, plain and in `text`, and
                // names the control for accessibility tools.
                let c = Control {
                    disabled: row.kind.disabled(),
                    menu: ParamMenu::Unbound,
                    label_font: FontSize::Pt10,
                    ..Control::new(id, rect, row.label, text, 0.0)
                };
                let mut v = if on { 1.0 } else { 0.0 };
                let pill = Pill {
                    label_right: true,
                    ..Pill::default()
                };
                // The `menuFill` bar under the pill and the label, lighter while pressed: put in its place once the
                // toggle has told whether it is.
                let bar = ui.painter().add(Shape::Noop);
                let response = pill_toggle(ui, cx, &c, pill, &mut v, |_| String::new());
                let held = response.is_pointer_button_down_on() && primary_down_live(ui, id);
                let fill = colors.menu_fill.to_color32();
                let fill = if held { pressed(fill) } else { fill };
                ui.painter().set(bar, Shape::rect_filled(rect, 0.0, fill));
                let changed = response.changed();
                match (changed, field) {
                    (true, Some(field)) => *field(s) = v >= 0.5,
                    (true, None) if matches!(row.kind, Kind::Latency) => {
                        s.latency_compensation = Some(v >= 0.5);
                    }
                    _ => {}
                }
            }
            Kind::Dropdown(choice) => {
                label(ui, before(button_rect), FontSize::Pt10);
                if light_button(ui, button_rect, &choice.label(s), true) {
                    self.menu = Some((Menu::Choice(choice), button_rect.center()));
                }
            }
            Kind::MidiDropdown => {
                label(ui, before(button_rect), FontSize::Pt10);
                let b = Button::new(id, button_rect, "", false, colors);
                let response = button(ui, cx, &b, true);
                // The name it draws itself (shrunk) is its name for accessibility tools too.
                response
                    .widget_info(|| WidgetInfo::labeled(WidgetType::Button, true, &s.midi_device));
                if response.clicked() {
                    self.menu = Some((Menu::Midi, button_rect.center()));
                    self.midi_rescan = true;
                }
                // A device name is the device's, of any length: shrink drawing in the button, in the ink the button
                // would give a label of its own.
                let room = button_rect.shrink2(vec2(m.u(BUTTON_PAD), 0.0));
                let ink = colors.menu_fill.to_color32();
                let (align, pt) = (Align2::CENTER_CENTER, FontSize::Pt10.points() as u8);
                shrink(ui.painter(), m, room, align, &s.midi_device, pt, ink);
            }
            Kind::FixedDropdown(value) => {
                label(ui, before(button_rect), FontSize::Pt10);
                light_button(ui, button_rect, value, false);
            }
            Kind::Scan => {
                ui.interact(r, id, Sense::hover());
                let (icon, rest) = r.split_left_right_at_x(r.left() + m.u(SCAN_ICON_W));
                draw_icon(ui.painter(), icon, IconId::WirelessMidi, text, SCAN_ICON, m);
                label(ui, rest, FontSize::Pt10);
            }
            Kind::Level => {
                let (line, body) = r.split_top_bottom_at_y(r.top() + m.u(LEVEL_LABEL_H));
                let (room, percent_room) =
                    line.split_left_right_at_x(line.right() - m.u(PERCENT_W));
                label(ui, room, FontSize::Pt10);
                let percent = format!("{}%", (s.metronome_level * 100.0).round());
                plain(
                    ui,
                    percent_room,
                    &percent,
                    FontSize::Pt10,
                    Align2::RIGHT_CENTER,
                );
                let c = level_control(id, body, text);
                h_slider(ui, cx, &c, &mut s.metronome_level, |_| String::new());
            }
            Kind::DisabledButton | Kind::Restore | Kind::RestoreDemos | Kind::Credits => {
                let rect = Rect::from_center_size(r.center(), vec2(r.width(), m.u(BUTTON_H)));
                let enabled = !row.kind.disabled();
                if light_button(ui, rect, row.label, enabled) && enabled {
                    match row.kind {
                        Kind::Restore => self.menu = Some((Menu::Restore, rect.center())),
                        Kind::Credits => self.credits = true,
                        _ => self.restore_demos = true,
                    }
                }
            }
            Kind::SignIn => {
                label(ui, before(button_rect), FontSize::Pt10);
                light_button(ui, button_rect, "Sign In", false);
            }
            Kind::PluginFolders => {
                let folders = match s.plugin_folders.len() {
                    0 => "The standard folders, and yabridge's".to_owned(),
                    _ => format!("Also: {}", s.plugin_folders.join("; ")),
                };
                let (room, size) = (before(button_rect), FontSize::Pt10.points() as u8);
                shrink(
                    ui.painter(),
                    m,
                    room,
                    Align2::LEFT_CENTER,
                    &folders,
                    size,
                    text,
                );
                if light_button(ui, button_rect, "Add folder", true) {
                    self.plugin_action = Some(PluginAction::AddFolder);
                }
            }
            Kind::PluginScan => {
                let state = if crate::plugins::scanning() {
                    "Scanning for plugins…".to_owned()
                } else {
                    let found = crate::plugins::available().len();
                    let (_, failed) = crate::plugins::scan_counts();
                    match failed {
                        0 => format!("{found} plugins found"),
                        _ => format!("{found} plugins found, {failed} files did not load"),
                    }
                };
                let (room, size) = (before(button_rect), FontSize::Pt10.points() as u8);
                shrink(
                    ui.painter(),
                    m,
                    room,
                    Align2::LEFT_CENTER,
                    &state,
                    size,
                    text,
                );
                if light_button(ui, button_rect, "Rescan", !crate::plugins::scanning()) {
                    self.plugin_action = Some(PluginAction::Rescan);
                }
            }
            Kind::ForgetPluginFolders => {
                let rect = Rect::from_center_size(r.center(), vec2(r.width(), m.u(BUTTON_H)));
                let some = !s.plugin_folders.is_empty();
                if light_button(ui, rect, row.label, some) && some {
                    s.plugin_folders.clear();
                    self.plugin_action = Some(PluginAction::Forgot);
                }
            }
            Kind::ContentFolder => {
                let path =
                    (self.content.as_deref()).map_or_else(String::new, |p| p.display().to_string());
                let (room, size) = (before(button_rect), FontSize::Pt10.points() as u8);
                shrink(
                    ui.painter(),
                    m,
                    room,
                    Align2::LEFT_CENTER,
                    &path,
                    size,
                    text,
                );
                let native = cfg!(all(not(target_arch = "wasm32"), not(target_os = "android")));
                if light_button(ui, button_rect, OPEN_FOLDER, native) {
                    self.reveal = true;
                }
            }
        }
    }

    /// Shows the open dropdown's popup and applies the pick.
    fn popup(&mut self, ui: &Ui, cx: &UiCtx, s: &mut Settings) {
        let Some((menu, at)) = self.menu else {
            return;
        };
        let ctx = ui.ctx();
        let mut items = popup_items(menu, s, &self.midi_ports);
        if menu == Menu::Midi {
            // A device name of any length: one too wide for the screen keeps its head and tail (the pick goes by
            // index, so it stores the whole name). The room leaves the item's side padding (1 U).
            let (font, room) = (
                cx.m.font_id(FontSize::Pt10),
                ctx.content_rect().width() - cx.m.u(1.0),
            );
            for item in &mut items {
                let shown =
                    truncated_text(ui.painter(), &item.label, font.clone(), room, DEVICE_TAIL_K);
                item.label = shown.into();
            }
        }
        let result = popup::show(ctx, popup_id(), at, ctx.content_rect(), &items, cx);
        match (menu, result.selected) {
            (Menu::Choice(choice), Some(value)) => choice.set(s, value),
            (Menu::Midi, Some(index)) => {
                if let Some(pick) = midi_menu(&self.midi_ports).get(index) {
                    s.midi_device = pick.label().to_owned();
                }
            }
            // What the first run did stays done: the demos are not written over the user's copies at the next start.
            (Menu::Restore, Some(CONFIRM_YES)) => {
                *s = Settings {
                    demo_songs_restored: s.demo_songs_restored,
                    ..Settings::default()
                }
            }
            _ => {}
        }
        if result.close {
            self.menu = None;
        }
    }
}

impl Kind {
    /// Drawn at the disabled alpha, without input.
    fn disabled(self) -> bool {
        matches!(
            self,
            Kind::FixedToggle(_)
                | Kind::FixedDropdown(_)
                | Kind::Scan
                | Kind::SignIn
                | Kind::DisabledButton
        )
    }
}

impl Choice {
    /// The items in menu order, as (stored value, label).
    fn items(self) -> Vec<(usize, String)> {
        match self {
            Choice::SoundMode => SOUND_MODES
                .iter()
                .map(|&(id, label)| (usize::from(id), label.to_owned()))
                .collect(),
            Choice::Theme => builtin_themes()
                .into_iter()
                .map(|t| (t.id as usize, t.name))
                .collect(),
            Choice::Scaling => ui_scale_menu_order()
                .into_iter()
                .map(|i| (i, format!("{}%", (UI_SCALE_FACTORS[i] * 100.0).round())))
                .collect(),
        }
    }

    /// The stored value.
    fn get(self, s: &Settings) -> usize {
        match self {
            Choice::SoundMode => usize::from(s.sound_mode),
            Choice::Theme => s.theme as usize,
            Choice::Scaling => s.ui_scale_index,
        }
    }

    /// Stores `value`, one of [`Choice::items`]' values.
    fn set(self, s: &mut Settings, value: usize) {
        match self {
            Choice::SoundMode => s.sound_mode = value as u8,
            Choice::Theme => s.theme = value as u32,
            Choice::Scaling => s.ui_scale_index = value,
        }
    }

    /// The current item's label, the button's text. A value that is no item shows the first item.
    fn label(self, s: &Settings) -> String {
        let (current, mut items) = (self.get(s), self.items());
        let pos = items.iter().position(|&(v, _)| v == current).unwrap_or(0);
        items.swap_remove(pos).1
    }
}

/// The metronome slider in `rect`: a double click resets it to the default level, and it fills from its low end
/// (`Control::new` reads the 0.5 default as bipolar). It is bound to no parameter, so its long-press menu is
/// `Reset` alone.
fn level_control(id: Id, rect: Rect, color: Color32) -> Control<'static> {
    Control {
        bipolar: false,
        menu: ParamMenu::Unbound,
        ..Control::new(id, rect, "", color, Settings::default().metronome_level)
    }
}

/// The popup's items. A dropdown's: one column in menu order, the current one marked, each carrying its stored value
/// (the `Input device:` menu's, over `midi_ports`, its index). A stored device that is not connected marks none.
fn popup_items(menu: Menu, s: &Settings, midi_ports: &[String]) -> Vec<PopupItem<usize>> {
    // (stored value, label, current) per item.
    let items: Vec<(usize, String, bool)> = match menu {
        Menu::Choice(choice) => {
            let current = choice.get(s);
            (choice.items().into_iter())
                .map(|(value, label)| (value, label, value == current))
                .collect()
        }
        Menu::Midi => (midi_menu(midi_ports).iter().enumerate())
            .map(|(i, c)| (i, c.label().to_owned(), c.label() == s.midi_device))
            .collect(),
        Menu::Restore => {
            // The confirm: the question as a header, then `Yes` and `No`.
            return vec![
                PopupItem::header(0, 0, RESTORE_QUESTION),
                PopupItem::new(CONFIRM_YES, 0, 1, "Yes"),
                PopupItem::new(CONFIRM_NO, 0, 2, "No"),
            ];
        }
    };
    (items.into_iter().enumerate())
        .map(|(row, (value, label, current))| PopupItem {
            current,
            ..PopupItem::new(value, 0, row, label)
        })
        .collect()
}

/// The id of row `index`'s control.
fn row_id(index: usize) -> Id {
    Id::new("settings_tab").with(index)
}

/// The id of the dropdown popup.
fn popup_id() -> Id {
    Id::new("settings_tab").with("popup")
}

#[cfg(test)]
mod tests {
    use eframe::egui::{self, Event, PointerButton, RawInput, pos2, vec2};

    use super::*;
    use crate::metrics::{DEFAULT_UI_SCALE_INDEX, Metrics, UI_SCALE_FACTORS};
    use crate::theme::theme_by_id;

    /// The index of the row labelled `label`.
    fn index(label: &str) -> usize {
        ROWS.iter().position(|r| r.label == label).unwrap()
    }

    #[test]
    fn rows_follow_the_spec_table_in_order() {
        let labels: Vec<&str> = ROWS.iter().map(|r| r.label).collect();
        assert_eq!(
            labels,
            [
                "MIDI:",
                "Input device:",
                "MIDI driver:",
                "Scan Bluetooth MIDI Devices",
                "Audio:",
                "Multi core processing",
                "Enable low latency mode",
                "Enable background audio",
                "Sound mode:",
                "Recording:",
                "Enable audio recording latency compensation",
                "Enable multitrack recording",
                "Enable count-in when recording",
                "Metronome level",
                "User interface:",
                "Theme:",
                "Scaling:",
                "Application:",
                "Enable ghost notes",
                "Show key names on keyboard and piano roll",
                "Set middle C to C4",
                "Show range labels on keyboard",
                "Continuous scrolling during playback",
                "Show CPU and RAM meters",
                "Enable sharing files with nearby devices",
                "Advanced:",
                "Share logs",
                "Restore demo songs",
                "Restore default settings",
                "Cloud backup:",
                "You are not signed in",
                "Sync automatically",
                "Sync only when connected to WiFi",
                "Sound credits",
                "Original content:",
                "Original content folder",
            ]
        );
        // Section headers, and only they, end in a colon (the `Input device:` style rows are dropdowns).
        for r in &ROWS {
            let header = matches!(r.kind, Kind::Header);
            let dropdown = matches!(
                r.kind,
                Kind::Dropdown(_) | Kind::MidiDropdown | Kind::FixedDropdown(_)
            );
            assert_eq!(r.label.ends_with(':'), header || dropdown, "{}", r.label);
        }
    }

    #[test]
    fn each_toggle_row_edits_its_own_setting() {
        let toggles: Vec<_> = ROWS
            .iter()
            .filter_map(|r| match r.kind {
                Kind::Toggle(field) => Some((r.label, field)),
                _ => None,
            })
            .collect();
        assert_eq!(toggles.len(), 11);
        for (label, field) in &toggles {
            let mut s = Settings::default();
            *field(&mut s) = !*field(&mut s);
            // Exactly one toggle changed.
            let differ = toggles
                .iter()
                .filter(|(_, other)| *other(&mut s) != *other(&mut Settings::default()))
                .count();
            assert_eq!(differ, 1, "{label}");
        }
    }

    #[test]
    fn dropdowns_list_the_spec_items_and_store_their_ids() {
        let labels = |c: Choice| -> Vec<String> { c.items().into_iter().map(|(_, l)| l).collect() };
        let values = |c: Choice| -> Vec<usize> { c.items().into_iter().map(|(v, _)| v).collect() };
        assert_eq!(
            labels(Choice::SoundMode),
            ["ULTRASAFE", "SAFE", "FAST", "NATIVE"]
        );
        assert_eq!(values(Choice::SoundMode), [2, 0, 1, 3]);
        assert_eq!(
            labels(Choice::Theme),
            [
                "Classic Gray",
                "Deep Purple",
                "Slate",
                "Paper",
                "Clay",
                "Ground",
                "Mint",
                "High Contrast A",
                "High Contrast B"
            ]
        );
        assert_eq!(values(Choice::Theme), (0..9).collect::<Vec<_>>());
        assert_eq!(
            labels(Choice::Scaling),
            [
                "60%", "75%", "90%", "100%", "110%", "125%", "150%", "175%", "200%"
            ]
        );
        assert_eq!(values(Choice::Scaling), [8, 0, 4, 1, 5, 2, 3, 6, 7]);
        // The buttons show the current item.
        let s = Settings::default();
        assert_eq!(Choice::SoundMode.label(&s), "NATIVE");
        assert_eq!(Choice::Theme.label(&s), "Classic Gray");
        assert_eq!(Choice::Scaling.label(&s), "100%");
        let mut s = Settings::default();
        for c in [Choice::SoundMode, Choice::Theme, Choice::Scaling] {
            let (value, label) = c.items()[1].clone();
            c.set(&mut s, value);
            assert_eq!(c.get(&s), value);
            assert_eq!(c.label(&s), label);
        }
        assert_eq!(SOUND_MODES[1], (0, "SAFE"));
    }

    #[test]
    fn the_popup_marks_the_current_item() {
        let s = Settings {
            ui_scale_index: 3,
            ..Settings::default()
        };
        let items = popup_items(Menu::Choice(Choice::Scaling), &s, &[]);
        let current: Vec<_> = items
            .iter()
            .filter(|i| i.current)
            .map(|i| &*i.label)
            .collect();
        assert_eq!(current, ["150%"]);
        assert!(
            items
                .iter()
                .enumerate()
                .all(|(row, i)| i.row == row && i.col == 0)
        );
    }

    // Offscreen: the tab in a real context, driven by synthetic pointer events.

    /// The tab in the right half of a 1170-point-wide screen at U = 32, `height` points tall (see [`Rig::at`] for
    /// other scales and widths).
    struct Rig {
        ctx: egui::Context,
        tab: SettingsTab,
        settings: Settings,
        /// The screen's size in points; the tab reaches its right edge.
        width: f32,
        height: f32,
        ppp: f32,
        /// The UI-scale index feeding `U`.
        scale: usize,
        time: f64,
        /// `show` reported a change on some frame since the last [`Rig::take_changed`].
        changed: bool,
        /// The tab's left edge; it reaches the screen's right edge.
        left: f32,
        /// What the last frame painted.
        shapes: Vec<egui::epaint::ClippedShape>,
        /// Frames run without showing the tab (another menu tab is up).
        hidden: bool,
        /// The last frame's accessibility tree, once `ctx.enable_accesskit()` is on.
        access: Option<egui::accesskit::TreeUpdate>,
    }

    impl Rig {
        /// Tall enough to show every row (2 U each) without scrolling.
        fn new() -> Self {
            Self::with_height(2400.0)
        }

        fn with_height(height: f32) -> Self {
            let mut rig = Self {
                ctx: egui::Context::default(),
                tab: SettingsTab::default(),
                settings: Settings::default(),
                width: 1170.0,
                height,
                ppp: 1.0,
                scale: DEFAULT_UI_SCALE_INDEX,
                time: 0.0,
                changed: false,
                left: 585.0,
                shapes: Vec::new(),
                hidden: false,
                access: None,
            };
            rig.frame(vec![]);
            rig
        }

        /// The tab `tab_u` U wide at pixels-per-point `ppp` and UI-scale index `scale`, filling a screen as wide
        /// (the main menu's panel is min(20 U, screen width)), tall enough to show every row.
        fn at(ppp: f32, scale: usize, tab_u: f32) -> Self {
            let u = Metrics::from_pixels_per_point(ppp, scale).u(1.0);
            let mut rig = Self::new();
            (rig.ppp, rig.scale, rig.width, rig.left) = (ppp, scale, tab_u * u, 0.0);
            rig.height = 100.0 * u;
            // The first frame at a new size settles egui's layout memory; the second reads it.
            rig.frame(vec![]);
            rig.frame(vec![]);
            rig
        }

        fn rect(&self) -> Rect {
            Rect::from_min_max(pos2(self.left, 0.0), pos2(self.width, self.height))
        }

        fn frame(&mut self, events: Vec<Event>) {
            self.time += 0.1;
            let mut raw = RawInput {
                screen_rect: Some(Rect::from_min_size(
                    Pos2::ZERO,
                    vec2(self.width, self.height),
                )),
                events,
                time: Some(self.time),
                ..Default::default()
            };
            raw.viewports
                .entry(raw.viewport_id)
                .or_default()
                .native_pixels_per_point = Some(self.ppp);
            let (theme, m) = (
                theme_by_id(0),
                Metrics::from_pixels_per_point(self.ppp, self.scale),
            );
            let cx = UiCtx {
                theme: &theme,
                m: &m,
            };
            let rect = self.rect();
            let Self {
                ctx,
                tab,
                settings,
                hidden,
                ..
            } = self;
            let mut changed = false;
            let out = ctx.run(raw, |ctx| {
                egui::CentralPanel::default()
                    .frame(egui::Frame::NONE)
                    .show(ctx, |ui| {
                        if !*hidden {
                            let before = settings.clone();
                            tab.show(ui, &cx, rect, settings);
                            changed = *settings != before;
                        }
                    });
            });
            self.changed |= changed;
            self.shapes = out.shapes;
            self.access = out.platform_output.accesskit_update;
        }

        fn button(&mut self, pos: Pos2, pressed: bool) {
            self.frame(vec![Event::PointerButton {
                pos,
                button: PointerButton::Primary,
                pressed,
                modifiers: Default::default(),
            }]);
        }

        fn click(&mut self, pos: Pos2) {
            self.frame(vec![Event::PointerMoved(pos)]);
            self.button(pos, true);
            self.button(pos, false);
        }

        /// The rect of the widget `id` drew last frame.
        fn rect_of(&self, id: Id) -> Rect {
            self.ctx
                .read_response(id)
                .unwrap_or_else(|| panic!("{id:?} not drawn"))
                .rect
        }

        fn click_row(&mut self, label: &str) {
            let at = self.rect_of(row_id(index(label))).center();
            self.click(at);
        }

        /// The texts painted last frame.
        fn texts(&self) -> Vec<String> {
            self.shapes
                .iter()
                .filter_map(|s| match &s.shape {
                    egui::Shape::Text(t) => Some(t.galley.text().to_owned()),
                    _ => None,
                })
                .collect()
        }

        /// `U` in points.
        fn ctx_u(&self) -> f32 {
            Metrics::from_pixels_per_point(self.ppp, self.scale).u(1.0)
        }

        /// The bounding rect and text of every text painted last frame.
        fn text_rects(&self) -> Vec<(Rect, String)> {
            self.shapes
                .iter()
                .filter_map(|s| match &s.shape {
                    egui::Shape::Text(t) => {
                        Some((t.visual_bounding_rect(), t.galley.text().to_owned()))
                    }
                    _ => None,
                })
                .collect()
        }

        /// The label painted for row `label` in the band `y` (its control's rows): the text starting furthest left.
        fn label_rect(&self, y: std::ops::RangeInclusive<f32>) -> (Rect, String) {
            self.text_rects()
                .into_iter()
                .filter(|(r, _)| y.contains(&r.center().y))
                .min_by(|a, b| a.0.min.x.total_cmp(&b.0.min.x))
                .expect("the row paints a label")
        }

        fn key(&mut self, key: egui::Key) {
            self.frame(vec![Event::Key {
                key,
                physical_key: None,
                pressed: true,
                repeat: false,
                modifiers: Default::default(),
            }]);
        }

        fn take_changed(&mut self) -> bool {
            std::mem::take(&mut self.changed)
        }
    }

    /// Pixel densities and tab widths (in U) the layout is checked at: 200 % is where the labels are widest.
    const SCALE_200: usize = 7;
    const NARROW_U: f32 = 9.0;
    const WIDE_U: f32 = 20.0;

    #[test]
    fn a_label_never_runs_under_its_control_at_200_percent_on_a_narrow_screen() {
        for ppp in [1.0, 2.0] {
            let rig = Rig::at(ppp, SCALE_200, NARROW_U);
            for r in ROWS.iter().filter(|r| {
                matches!(
                    r.kind,
                    Kind::Dropdown(_) | Kind::MidiDropdown | Kind::FixedDropdown(_) | Kind::SignIn
                )
            }) {
                let control = rig.rect_of(row_id(index(r.label)));
                let (label, shown) = rig.label_rect(control.y_range().into());
                assert!(
                    !label.intersects(control),
                    "ppp {ppp}: `{shown}` {label:?} runs under its control {control:?}"
                );
                // And it stays inside the tab.
                assert!(label.left() >= rig.left, "ppp {ppp}: `{shown}` {label:?}");
            }
        }
    }

    #[test]
    fn the_metronome_label_stops_short_of_its_percent() {
        for (ppp, tab_u) in [(1.0, NARROW_U), (2.0, NARROW_U), (1.0, 7.0), (2.0, 7.0)] {
            let rig = Rig::at(ppp, SCALE_200, tab_u);
            let body = rig.rect_of(row_id(index("Metronome level")));
            let line = (body.top() - rig.ctx_u() * LEVEL_LABEL_H)..=body.top();
            let texts: Vec<_> = rig
                .text_rects()
                .into_iter()
                .filter(|(r, _)| line.contains(&r.center().y))
                .collect();
            let [(label, _), (percent, _)] = &texts[..] else {
                panic!("a label and a percent, got {texts:?}");
            };
            assert!(
                label.right() <= percent.left(),
                "ppp {ppp}: {label:?} {percent:?}"
            );
        }
    }

    #[test]
    fn a_label_that_cannot_fit_shrinks_then_keeps_its_head_and_tail() {
        // Shrink drawing: 1 pt at a time down to 6 pt, then a middle ellipsis keeping 5 tail characters.
        // The Rig installs no app font, so this runs on egui's default font: the glyph widths differ from the app
        // font's, and the test pins only the shape of the result (head, `...`, 5-character tail, the 6 pt floor),
        // never the exact characters kept in the middle.
        let rig = Rig::at(1.0, SCALE_200, NARROW_U);
        let long = "Scan Bluetooth MIDI Devices";
        let control = rig.rect_of(row_id(index(long)));
        let (label, shown) = rig.label_rect(control.y_range().into());
        assert!(
            shown.starts_with("Scan") && shown.contains("...") && shown.ends_with("vices"),
            "{shown}"
        );
        // 6 pt: smaller than the 10 pt line of a label that fits.
        let fits = rig.rect_of(row_id(index("Set middle C to C4")));
        let (whole, text) = rig.label_rect(fits.y_range().into());
        assert_eq!(text, "Set middle C to C4");
        assert!(label.height() < whole.height(), "{label:?} {whole:?}");
    }

    #[test]
    fn labels_are_drawn_whole_at_full_size_when_the_tab_is_wide() {
        // The menu panel is 20 U at most; every label fits its room there at 100 % and 200 % alike.
        for (ppp, scale) in [(1.0, DEFAULT_UI_SCALE_INDEX), (2.0, SCALE_200)] {
            let rig = Rig::at(ppp, scale, WIDE_U);
            let sizes: Vec<_> = rig
                .shapes
                .iter()
                .filter_map(|s| match &s.shape {
                    egui::Shape::Text(t) => Some((t.galley.text().to_owned(), t.galley.size().y)),
                    _ => None,
                })
                .collect();
            let height = |label: &str| {
                sizes
                    .iter()
                    .find(|(t, _)| t == label)
                    .unwrap_or_else(|| {
                        panic!("ppp {ppp}: `{label}` shortened or missing in {sizes:?}")
                    })
                    .1
            };
            let ten = height("Multi core processing");
            for r in ROWS.iter().filter(|r| {
                !matches!(
                    r.kind,
                    Kind::Header | Kind::DisabledButton | Kind::Restore | Kind::ContentFolder
                )
            }) {
                assert_eq!(
                    height(r.label),
                    ten,
                    "ppp {ppp}: `{}` not at 10 pt",
                    r.label
                );
            }
            // The headers are 12 pt: the height a 12 pt galley has in this context, not merely taller than ten.
            let m = Metrics::from_pixels_per_point(ppp, scale);
            let twelve = rig.ctx.fonts_mut(|f| {
                f.layout_no_wrap("Hg".into(), m.font_id(FontSize::Pt12), Color32::WHITE)
                    .size()
                    .y
            });
            assert!(twelve > ten, "ppp {ppp}: 12 pt {twelve} vs 10 pt {ten}");
            // `Original content:` (BYO) is the last section, below this rig's visible height; its row has its own
            // test (`texts.contains("Original content:")`).
            for r in ROWS
                .iter()
                .filter(|r| matches!(r.kind, Kind::Header) && r.label != "Original content:")
            {
                assert_eq!(
                    height(r.label),
                    twelve,
                    "ppp {ppp}: `{}` not at 12 pt",
                    r.label
                );
            }
        }
    }

    /// The toggle rows of the list.
    fn toggles() -> impl Iterator<Item = &'static Row> + Clone {
        ROWS.iter().filter(|r| {
            matches!(
                r.kind,
                Kind::Toggle(_) | Kind::FixedToggle(_) | Kind::Latency
            )
        })
    }

    /// The height of the galley painted for `text` in the last frame, and that of "Hg" at 10 pt in this context.
    fn heights(rig: &Rig, m: &Metrics, text: &str) -> (f32, f32) {
        let painted = rig.shapes.iter().find_map(|s| match &s.shape {
            egui::Shape::Text(t) if t.galley.text() == text => Some(t.galley.size().y),
            _ => None,
        });
        let ten = rig.ctx.fonts_mut(|f| {
            f.layout_no_wrap("Hg".into(), m.font_id(FontSize::Pt10), Color32::WHITE)
                .size()
                .y
        });
        (
            painted.unwrap_or_else(|| panic!("`{text}` not painted")),
            ten,
        )
    }

    /// The filled rects the last frame painted, with their fills.
    fn fills(rig: &Rig) -> Vec<(Rect, Color32)> {
        (rig.shapes.iter())
            .filter_map(|s| match &s.shape {
                egui::Shape::Rect(r) => Some((r.rect, r.fill)),
                _ => None,
            })
            .collect()
    }

    /// The `(config)` rigs the settings geometry is checked at: ppp 1 and 2, and 200 % at ppp 2.
    const CONFIGS: [(f32, usize); 3] = [
        (1.0, DEFAULT_UI_SCALE_INDEX),
        (2.0, DEFAULT_UI_SCALE_INDEX),
        (2.0, SCALE_200),
    ];

    #[test]
    fn rows_are_2_u_apart_in_a_column_2_u_in_from_the_page_and_control_rows_are_1_5_u() {
        // The row pitch is 2 U and every control row 1.5 U tall; the content column is the page's left + 2 U, W - 4 U
        // wide.
        for (ppp, scale) in CONFIGS {
            let rig = Rig::at(ppp, scale, WIDE_U);
            let u = Metrics::from_pixels_per_point(ppp, scale).u(1.0);
            let at = format!("ppp {ppp} scale {scale}");
            let column = (rig.left + 2.0 * u, rig.width - 2.0 * u);
            let centre = |label: &str| rig.rect_of(row_id(index(label))).center().y;
            // Three rows in a section, then a dropdown and the next header's toggle: one pitch each.
            let first = centre("Multi core processing");
            assert!(
                (centre("Enable low latency mode") - first - 2.0 * u).abs() < 0.01,
                "{at}"
            );
            assert!(
                (centre("Enable background audio") - first - 4.0 * u).abs() < 0.01,
                "{at}"
            );
            assert!(
                (centre("Sound mode:") - first - 6.0 * u).abs() < 0.01,
                "{at}"
            );
            for label in ["Multi core processing", "Sound mode:", "Input device:"] {
                let control = rig.rect_of(row_id(index(label)));
                let want = if label.ends_with(':') { u } else { 1.5 * u };
                assert!(
                    (control.height() - want).abs() < 0.01,
                    "{at}: `{label}` {control:?}"
                );
            }
            // The wide buttons end at the column's right edge; the section headers start at its left.
            let button = rig.rect_of(row_id(index("Input device:")));
            assert!((button.right() - column.1).abs() < 0.01, "{at}: {button:?}");
            let header = (rig.text_rects().into_iter())
                .find(|(_, t)| t == "MIDI:")
                .expect("a header")
                .0;
            assert!(
                (header.left() - column.0).abs() < 0.2 * u,
                "{at}: {header:?}"
            );
        }
    }

    #[test]
    fn a_toggle_is_a_menu_fill_bar_half_a_u_left_of_the_column_with_the_pill_and_the_label_in_it() {
        // Each toggle rect starts 0.5 U left of the column and ends at the column's right edge; the pill is centred at
        // x + 0.75 U (page left + 2.25 U); the label runs from x + 1.7 U to the right edge − 0.2 U, FontSize 2,
        // left-aligned, vertically centred, in `text`. The latency toggle is 2.5 U shorter.
        let colors = theme_by_id(0).colors;
        let (menu_fill, text, control_fill) = (
            colors.menu_fill.to_color32(),
            colors.text.to_color32(),
            colors.control_fill.to_color32(),
        );
        for (ppp, scale) in CONFIGS {
            let rig = Rig::at(ppp, scale, WIDE_U);
            let m = Metrics::from_pixels_per_point(ppp, scale);
            let (u, t) = (m.u(1.0), m.t());
            let column_right = rig.width - 2.0 * u;
            let fills = fills(&rig);
            assert_eq!(toggles().count(), 15);
            for r in toggles() {
                let at = format!("ppp {ppp} scale {scale}: `{}`", r.label);
                let control = rig.rect_of(row_id(index(r.label)));
                let shorter = if matches!(r.kind, Kind::Latency) {
                    2.5 * u
                } else {
                    0.0
                };
                // The rect: 1.5 U tall, from 0.5 U left of the column (page left + 1.5 U) to its right edge.
                let want = Rect::from_x_y_ranges(
                    rig.left + 1.5 * u..=column_right - shorter,
                    control.y_range(),
                );
                assert!(
                    (control.min - want.min).abs().max_elem() < 0.01,
                    "{at}: {control:?}"
                );
                assert!(
                    (control.max - want.max).abs().max_elem() < 0.01,
                    "{at}: {control:?}"
                );
                assert!((control.height() - 1.5 * u).abs() < 0.01, "{at}");
                // It is painted whole as a `menuFill` bar (not pressed).
                // (A disabled row is drawn at the disabled alpha, bar, pill and label alike.)
                let dim = |c: Color32| {
                    if r.kind.disabled() {
                        c.gamma_multiply(DISABLED_ALPHA)
                    } else {
                        c
                    }
                };
                assert!(
                    fills.contains(&(control, dim(menu_fill))),
                    "{at}: no bar {control:?}"
                );
                // The pill: 10 t by 18 t, centred at the page's left + 2.25 U.
                let pill = Rect::from_center_size(
                    pos2(rig.left + 2.25 * u, control.center().y),
                    vec2(10.0 * t, 18.0 * t),
                );
                let near = |a: &Rect| {
                    (a.min - pill.min).abs().max_elem() < 0.01
                        && (a.max - pill.max).abs().max_elem() < 0.01
                };
                assert!(
                    fills
                        .iter()
                        .any(|(a, c)| *c == dim(control_fill) && near(a)),
                    "{at}: no pill {pill:?}"
                );
                // The label: from x + 1.7 U (within 0.1 U), whole, 10 pt, in `text`, ending 0.2 U short of the right.
                let (label, shown) = rig.label_rect(control.y_range().into());
                assert_eq!(shown, r.label, "{at}");
                let start = control.left() + 1.7 * u;
                assert!(
                    (label.left() - start).abs() <= 0.1 * u,
                    "{at}: label at {} for {start}",
                    label.left()
                );
                // (The latency label is the one that overflows with the test font, which is wider than the app's.)
                if !matches!(r.kind, Kind::Latency) {
                    assert!(
                        label.right() <= control.right() - 0.2 * u,
                        "{at}: {label:?}"
                    );
                }
                let (height, ten) = heights(&rig, &m, r.label);
                assert!((height - ten).abs() < 0.01, "{at}: {height} vs 10 pt {ten}");
                let ink = rig.shapes.iter().find_map(|s| match &s.shape {
                    egui::Shape::Text(g) if g.galley.text() == r.label => {
                        Some(g.galley.job.sections[0].format.color)
                    }
                    _ => None,
                });
                assert_eq!(ink, Some(text), "{at}");
            }
        }
    }

    #[test]
    fn on_a_narrow_page_a_toggle_label_overflows_plain_with_its_pill_gap_kept() {
        // The label is "drawn plain (no ellipsis; long text overflows)": not shrunk, and still 1.7 U from the rect's
        // left, 0.95 U - 5 t (the pill's right half) from the pill.
        for (ppp, scale) in [(1.0, SCALE_200), (2.0, SCALE_200)] {
            let rig = Rig::at(ppp, scale, NARROW_U);
            let m = Metrics::from_pixels_per_point(ppp, scale);
            let (u, t) = (m.u(1.0), m.t());
            let mut overflowed = 0;
            for r in toggles() {
                let at = format!("ppp {ppp}: `{}`", r.label);
                let control = rig.rect_of(row_id(index(r.label)));
                let (label, shown) = rig.label_rect(control.y_range().into());
                assert_eq!(shown, r.label, "{at}: whole, no ellipsis");
                let (height, ten) = heights(&rig, &m, r.label);
                assert!(
                    (height - ten).abs() < 0.01,
                    "{at}: not shrunk ({height} vs {ten})"
                );
                let pill_right = control.left() + 0.75 * u + 5.0 * t;
                assert!(
                    (label.left() - pill_right - (0.95 * u - 5.0 * t)).abs() <= 0.1 * u,
                    "{at}: gap {}",
                    label.left() - pill_right
                );
                overflowed += usize::from(label.right() > control.right());
            }
            assert!(
                overflowed > 0,
                "ppp {ppp}: the page is narrow enough for some label to overflow"
            );
        }
    }

    #[test]
    fn a_pressed_toggle_bar_is_brighter_and_the_whole_rect_is_the_control() {
        // "Each toggle paints its whole 1.5 U rect as a `menuFill` bar, x 1.15 brighter while pressed"; the rect is the
        // control, so the label flips it too (and a tap beside the rect does not).
        let menu_fill = theme_by_id(0).colors.menu_fill.to_color32();
        let mut rig = Rig::new();
        let control = rig.rect_of(row_id(index("Set middle C to C4")));
        let on_label = pos2(control.left() + 9.0 * 32.0, control.center().y);
        assert!(fills(&rig).contains(&(control, menu_fill)));
        rig.frame(vec![Event::PointerMoved(on_label)]);
        rig.button(on_label, true);
        assert!(fills(&rig).contains(&(control, pressed(menu_fill))), "held");
        assert!(
            !rig.settings.middle_c_is_c4,
            "not flipped before the release"
        );
        rig.button(on_label, false);
        assert!(rig.settings.middle_c_is_c4, "the label flips it");
        assert!(fills(&rig).contains(&(control, menu_fill)), "released");
        // The pill and the bar's own left end flip it too; half a U left of the rect does not.
        rig.click(pos2(control.left() + 0.75 * 32.0, control.center().y));
        assert!(!rig.settings.middle_c_is_c4);
        rig.click(pos2(control.left() + 2.0, control.center().y));
        assert!(rig.settings.middle_c_is_c4);
        rig.click(pos2(control.left() - 0.25 * 32.0, control.center().y));
        assert!(rig.settings.middle_c_is_c4, "outside the rect");
    }

    #[test]
    fn each_toggle_is_named_by_its_label_for_accessibility_tools() {
        use egui::accesskit::{NodeId, Role};
        let mut rig = Rig::new();
        rig.ctx.enable_accesskit();
        rig.frame(vec![]);
        let update = rig.access.as_ref().expect("accesskit on");
        for r in toggles() {
            let id = NodeId(row_id(index(r.label)).value());
            let (_, node) = (update.nodes.iter().find(|(n, _)| *n == id)).expect("a node");
            assert_eq!(node.label(), Some(r.label), "`{}`", r.label);
            assert_eq!(node.role(), Role::CheckBox, "`{}`", r.label);
        }
    }

    #[test]
    fn the_pill_dots_are_text_when_on_and_menu_fill_when_off_4_t_from_the_middle() {
        // The on-dot is `text`, the off-dot `menuFill`; radius 2 t, 4 t above (on) or below (off) the pill's centre.
        let colors = theme_by_id(0).colors;
        let (text, menu_fill) = (colors.text.to_color32(), colors.menu_fill.to_color32());
        for (ppp, scale) in CONFIGS {
            let mut rig = Rig::at(ppp, scale, WIDE_U);
            let m = Metrics::from_pixels_per_point(ppp, scale);
            let (u, t) = (m.u(1.0), m.t());
            assert!(Settings::default().ghost_notes && !Settings::default().middle_c_is_c4);
            let dots = |rig: &Rig, label: &str| -> Vec<(egui::Pos2, Color32)> {
                let control = rig.rect_of(row_id(index(label)));
                (rig.shapes.iter())
                    .filter_map(|s| match &s.shape {
                        egui::Shape::Circle(c)
                            if (c.radius - 2.0 * t).abs() < 0.01
                                && control.y_range().contains(c.center.y)
                                && (c.center.x - (rig.left + 2.25 * u)).abs() < 0.01 =>
                        {
                            Some((c.center, c.fill))
                        }
                        _ => None,
                    })
                    .collect()
            };
            let at = format!("ppp {ppp} scale {scale}");
            for (label, on) in [("Enable ghost notes", true), ("Set middle C to C4", false)] {
                let middle = rig.rect_of(row_id(index(label))).center().y;
                let found = dots(&rig, label);
                let want = [(
                    pos2(
                        rig.left + 2.25 * u,
                        middle + if on { -4.0 * t } else { 4.0 * t },
                    ),
                    if on { text } else { menu_fill },
                )];
                assert_eq!(found.len(), 1, "{at}: {label}: {found:?}");
                assert!(
                    (found[0].0 - want[0].0).length() < 0.01 && found[0].1 == want[0].1,
                    "{at}: {label}: {found:?} vs {want:?}"
                );
            }
            // Flipped: the dots swap.
            rig.click_row("Enable ghost notes");
            let middle = rig.rect_of(row_id(index("Enable ghost notes"))).center().y;
            let found = dots(&rig, "Enable ghost notes");
            assert!(
                (found[0].0.y - (middle + 4.0 * t)).abs() < 0.01 && found[0].1 == menu_fill,
                "{at}: {found:?}"
            );
        }
    }

    #[test]
    fn the_full_width_buttons_are_light_text_fills_and_the_bluetooth_row_is_plain() {
        // `Share logs`, `Restore demo songs`, `Restore default settings` and the other full-width buttons are filled
        // with `text`; `Scan Bluetooth MIDI Devices` is filled with `menuFill`, so it reads as plain text with an icon.
        let colors = theme_by_id(0).colors;
        let text = colors.text.to_color32();
        for (ppp, scale) in [(1.0, DEFAULT_UI_SCALE_INDEX), (2.0, DEFAULT_UI_SCALE_INDEX)] {
            let rig = Rig::at(ppp, scale, WIDE_U);
            let u = Metrics::from_pixels_per_point(ppp, scale).u(1.0);
            let fills = fills(&rig);
            for (label, dimmed) in [
                ("Share logs", true),
                ("Restore demo songs", false),
                ("Restore default settings", false),
            ] {
                let button = rig.rect_of(row_id(index(label)));
                assert!(
                    (button.width() - (rig.width - rig.left - 4.0 * u)).abs() < 0.01
                        && (button.height() - u).abs() < 0.01,
                    "`{label}` {button:?}"
                );
                let fill = if dimmed {
                    text.gamma_multiply(DISABLED_ALPHA)
                } else {
                    text
                };
                assert!(
                    fills.contains(&(button, fill)),
                    "ppp {ppp}: `{label}` is not a `text` fill: {button:?}"
                );
            }
            // The Bluetooth row paints no light fill: its ground is the menu's own `menuFill`.
            let row = rig.rect_of(row_id(index("Scan Bluetooth MIDI Devices")));
            for (r, f) in &fills {
                let over = r.y_range().intersects(row.y_range());
                let light = *f == text || *f == text.gamma_multiply(DISABLED_ALPHA);
                assert!(
                    !(over && light),
                    "ppp {ppp}: a light fill {r:?} {f:?} in the Bluetooth row"
                );
            }
        }
    }

    #[test]
    fn the_list_stops_at_its_last_row() {
        // "Scrolling moves in whole 2 U rows" and no further than the list: wheeling far past the end leaves the last row
        // in view, within a row of the bottom, and a second wheel changes nothing.
        let mut rig = Rig::with_height(540.0);
        let last = row_id(index("Sound credits"));
        let over = pos2(800.0, 300.0);
        rig.frame(vec![Event::PointerMoved(over)]);
        let wheel = |rig: &mut Rig| {
            for _ in 0..10 {
                rig.frame(vec![Event::MouseWheel {
                    unit: egui::MouseWheelUnit::Point,
                    delta: vec2(0.0, -500.0),
                    modifiers: Default::default(),
                }]);
            }
            for _ in 0..30 {
                rig.frame(vec![]);
            }
        };
        wheel(&mut rig);
        let rect = rig.rect_of(last);
        // The row's slot is 2 U (64) tall and centres its 1.5 U control: the last slot ends at or just above the bottom.
        let slot_bottom = rect.center().y + 32.0;
        assert!(
            slot_bottom <= 540.0 + 0.01 && slot_bottom > 540.0 - 64.0,
            "{rect:?}"
        );
        wheel(&mut rig);
        assert_eq!(rig.rect_of(last), rect, "no further");
        // Wheeling back up past the top stops at the first row.
        for _ in 0..3 {
            for _ in 0..10 {
                rig.frame(vec![Event::MouseWheel {
                    unit: egui::MouseWheelUnit::Point,
                    delta: vec2(0.0, 500.0),
                    modifiers: Default::default(),
                }]);
            }
        }
        for _ in 0..30 {
            rig.frame(vec![]);
        }
        // The first row is back where it rests: the header's slot above it, 2 U, and 0.25 U to its 1.5 U control.
        let first = rig.rect_of(row_id(index("Input device:")));
        assert_eq!(first.top(), 64.0 + 16.0, "{first:?}");
    }

    #[test]
    fn the_list_scrolls_in_whole_2_u_rows() {
        // "Scrolling moves in whole 2 U rows."
        let mut rig = Rig::with_height(540.0);
        let id = row_id(index("Multi core processing"));
        let top = rig.rect_of(id).top();
        let over = pos2(800.0, 300.0);
        rig.frame(vec![Event::PointerMoved(over)]);
        let mut seen = std::collections::BTreeSet::new();
        for delta in [-9.0, -9.0, -9.0, -30.0, -17.0, -41.0, -23.0] {
            rig.frame(vec![Event::MouseWheel {
                unit: egui::MouseWheelUnit::Point,
                delta: vec2(0.0, delta),
                modifiers: Default::default(),
            }]);
            for _ in 0..30 {
                rig.frame(vec![]);
            }
            let rows = (top - rig.rect_of(id).top()) / 64.0;
            assert!((rows - rows.round()).abs() < 1e-3, "{rows} rows");
            seen.insert(rows.round() as i32);
        }
        assert!(seen.len() > 2, "it did scroll: {seen:?}");
    }

    #[test]
    fn a_toggle_tap_flips_its_setting_and_reports_a_change() {
        let mut rig = Rig::new();
        assert!(!rig.take_changed());
        rig.click_row("Set middle C to C4");
        assert!(rig.settings.middle_c_is_c4);
        assert!(rig.take_changed());
        rig.click_row("Show CPU and RAM meters");
        assert!(!rig.settings.hardware_meters);
        rig.click_row("Set middle C to C4");
        assert_eq!(
            rig.settings,
            Settings {
                hardware_meters: false,
                ..Settings::default()
            }
        );
    }

    #[test]
    fn the_latency_toggle_shows_on_while_unset_and_stores_what_a_tap_sets() {
        // Unset shows ON, so the first tap turns it OFF (compensation on).
        let mut rig = Rig::new();
        let label = "Enable audio recording latency compensation";
        rig.click_row(label);
        assert_eq!(rig.settings.latency_compensation, Some(false));
        assert!(rig.take_changed());
        rig.click_row(label);
        assert_eq!(rig.settings.latency_compensation, Some(true));
    }

    #[test]
    fn calibrate_shows_only_while_the_latency_toggle_is_off_and_a_tap_asks_the_app() {
        // Compensation is on exactly when the toggle is OFF, and `Calibrate` is visible only then; unset shows ON. The
        // toggle is then 2.5 U shorter and `Calibrate`, 2.5 U wide, takes the right end of the row.
        let mut rig = Rig::new();
        let label = "Enable audio recording latency compensation";
        assert!(!rig.texts().contains(&"Calibrate".to_owned()), "unset");
        let on = rig.rect_of(row_id(index(label)));
        rig.click_row(label);
        assert_eq!(rig.settings.latency_compensation, Some(false));
        rig.frame(vec![]);
        let calibrate = rig.rect_of(row_id(index(label)).with("calibrate"));
        let toggle = rig.rect_of(row_id(index(label)));
        assert_eq!(toggle, on, "the toggle is as short with `Calibrate` hidden");
        assert_eq!(calibrate.width(), 2.5 * 32.0);
        assert_eq!(calibrate.right(), rig.width - 2.0 * 32.0);
        assert_eq!(calibrate.left(), toggle.right());
        assert_eq!(calibrate.center().y, toggle.center().y);
        assert_eq!(calibrate.height(), 32.0);
        let (text, name) = rig.label_rect(toggle.top()..=toggle.bottom());
        assert_eq!(name, label);
        // (The label is plain text: a long one runs on under `Calibrate`.)
        assert!(text.left() > toggle.left() + 1.5 * 32.0, "{text:?}");
        assert!(rig.texts().contains(&"Calibrate".to_owned()));
        assert!(!rig.tab.calibrate);
        rig.click(calibrate.center());
        assert!(rig.tab.calibrate, "the app starts it and clears this");
        assert_eq!(rig.settings.latency_compensation, Some(false), "no toggle");
        // ON again: hidden.
        rig.click_row(label);
        rig.frame(vec![]);
        assert_eq!(rig.settings.latency_compensation, Some(true));
        assert!(!rig.texts().contains(&"Calibrate".to_owned()));
    }

    #[test]
    fn a_dropdown_opens_its_popup_and_the_pick_is_stored() {
        let mut rig = Rig::new();
        rig.click_row("Theme:");
        assert_eq!(
            rig.tab.menu.map(|(c, _)| c),
            Some(Menu::Choice(Choice::Theme))
        );
        assert!(!rig.take_changed());
        // `Mint` is the seventh item.
        let mint = rig.rect_of(popup_id().with((0_usize, 6_usize))).center();
        rig.click(mint);
        assert_eq!(rig.settings.theme, 6);
        assert!(rig.take_changed());
        assert_eq!(rig.tab.menu, None);

        rig.click_row("Scaling:");
        let pct150 = rig.rect_of(popup_id().with((0_usize, 6_usize))).center();
        rig.click(pct150);
        assert_eq!(UI_SCALE_FACTORS[rig.settings.ui_scale_index], 1.5);

        rig.click_row("Sound mode:");
        let safe = rig.rect_of(popup_id().with((0_usize, 1_usize))).center();
        rig.click(safe);
        assert_eq!(rig.settings.sound_mode, 0);
    }

    #[test]
    fn picking_the_current_item_or_closing_the_popup_changes_nothing() {
        let mut rig = Rig::new();
        rig.click_row("Theme:");
        let classic = rig.rect_of(popup_id().with((0_usize, 0_usize))).center();
        rig.click(classic);
        assert_eq!(rig.tab.menu, None);
        rig.click_row("Theme:");
        // A click outside the popup closes it.
        rig.click(pos2(10.0, 10.0));
        assert_eq!(rig.tab.menu, None);
        assert!(!rig.take_changed());
        assert_eq!(rig.settings, Settings::default());
    }

    // The `Input device:` row.

    fn ports(names: &[&str]) -> Vec<String> {
        names.iter().map(|n| (*n).to_owned()).collect()
    }

    /// The labels of the `Input device:` popup over `names`, and the labels marked current.
    fn midi_menu_of(s: &Settings, names: &[&str]) -> (Vec<String>, Vec<String>) {
        let items = popup_items(Menu::Midi, s, &ports(names));
        let labels = |marked: bool| {
            (items.iter())
                .filter(|i| i.current || !marked)
                .map(|i| i.label.to_string())
                .collect()
        };
        (labels(false), labels(true))
    }

    #[test]
    fn the_input_device_menu_lists_none_any_then_each_device_once() {
        let s = Settings::default();
        let (labels, current) = midi_menu_of(&s, &[]);
        assert_eq!(labels, ["<no midi input>", "Any available"]);
        assert_eq!(current, ["Any available"]);
        let (labels, current) = midi_menu_of(&s, &["Keys 49 20:0", "Pad Box 24:0"]);
        assert_eq!(
            labels,
            ["<no midi input>", "Any available", "Keys 49", "Pad Box"]
        );
        assert_eq!(current, ["Any available"]);
        // ALSA's client:port numbers are no part of a name; a device with two ports is listed once.
        let (labels, _) = midi_menu_of(&s, &["Keys 49 20:0", "Pad Box 24:0", "Keys 49 20:1"]);
        assert_eq!(labels.len(), 4);
        // Every item is a cell of one column, carrying its index.
        let items = popup_items(Menu::Midi, &s, &ports(&["Keys 49 20:0"]));
        assert!(
            items
                .iter()
                .enumerate()
                .all(|(row, i)| i.id == Some(row) && i.row == row && i.col == 0 && !i.is_header())
        );
    }

    #[test]
    fn the_current_input_device_is_highlighted_and_a_missing_one_marks_none() {
        for (stored, marked) in [
            ("<no midi input>", vec!["<no midi input>"]),
            ("Any available", vec!["Any available"]),
            ("Pad Box", vec!["Pad Box"]),
            // Only the whole name: no prefix match.
            ("Pad Box 2", vec![]),
            ("Gone Keys", vec![]),
        ] {
            let s = Settings {
                midi_device: stored.into(),
                ..Settings::default()
            };
            let (_, current) = midi_menu_of(&s, &["Keys 49 20:0", "Pad Box 24:0"]);
            assert_eq!(current, marked, "{stored}");
        }
    }

    #[test]
    fn picking_an_input_device_stores_its_menu_label() {
        let mut rig = Rig::new();
        rig.tab
            .set_midi_ports(&ports(&["Keys 49 20:0", "Pad Box 24:0"]));
        rig.click_row("Input device:");
        assert_eq!(rig.tab.menu.map(|(m, _)| m), Some(Menu::Midi));
        assert!(!rig.take_changed());
        // `Pad Box` is the fourth item.
        let pad = rig.rect_of(popup_id().with((0_usize, 3_usize))).center();
        rig.click(pad);
        assert_eq!(rig.settings.midi_device, "Pad Box");
        assert!(rig.take_changed());
        assert_eq!(rig.tab.menu, None);
        // The first two items are `<no midi input>` and `Any available`.
        for (row, label) in [(0_usize, "<no midi input>"), (1, "Any available")] {
            rig.click_row("Input device:");
            let at = rig.rect_of(popup_id().with((0_usize, row))).center();
            rig.click(at);
            assert_eq!(rig.settings.midi_device, label);
        }
        // Only the device changed.
        assert_eq!(rig.settings, Settings::default());
    }

    #[test]
    fn the_input_device_button_is_named_after_the_stored_device_for_accessibility_tools() {
        use egui::accesskit::{NodeId, Role};
        let mut rig = Rig::new();
        rig.settings.midi_device = "Pad Box".into();
        rig.ctx.enable_accesskit();
        rig.frame(vec![]);
        let update = rig.access.as_ref().expect("accesskit on");
        let id = NodeId(row_id(index("Input device:")).value());
        let (_, node) = (update.nodes.iter().find(|(n, _)| *n == id)).expect("a node");
        assert_eq!((node.role(), node.label()), (Role::Button, Some("Pad Box")));
    }

    #[test]
    fn a_device_name_too_long_for_the_screen_is_shortened_in_the_popup_and_picked_whole() {
        let mut rig = Rig::new();
        let long = format!("Keys {} end49", "very long name ".repeat(20));
        rig.tab.set_midi_ports(std::slice::from_ref(&long));
        rig.click_row("Input device:");
        rig.frame(vec![]);
        let shown = (rig.texts().into_iter())
            .find(|t| t.starts_with("Keys "))
            .expect("the device");
        assert!(
            shown.contains("...") && shown.ends_with("end49") && shown.len() < long.len(),
            "{shown}"
        );
        let item = rig.rect_of(popup_id().with((0_usize, 2_usize)));
        assert!(item.width() <= rig.width, "{item:?}");
        rig.click(item.center());
        assert_eq!(rig.settings.midi_device, long);
    }

    #[test]
    fn closing_the_input_device_popup_keeps_the_stored_device() {
        let mut rig = Rig::new();
        rig.settings.midi_device = "Gone Keys".into();
        rig.take_changed();
        rig.click_row("Input device:");
        rig.click(pos2(10.0, 10.0));
        assert_eq!(rig.tab.menu, None);
        assert_eq!(rig.settings.midi_device, "Gone Keys");
        assert!(!rig.take_changed());
    }

    #[test]
    fn opening_the_input_device_dropdown_asks_for_one_rescan() {
        let mut rig = Rig::new();
        assert!(!rig.tab.take_midi_rescan());
        rig.click_row("Input device:");
        assert!(rig.tab.take_midi_rescan());
        assert!(!rig.tab.take_midi_rescan());
        // Not again for the frames it stays open, nor for a pick.
        for _ in 0..5 {
            rig.frame(vec![]);
        }
        assert!(!rig.tab.take_midi_rescan());
        let any = rig.rect_of(popup_id().with((0_usize, 1_usize))).center();
        rig.click(any);
        assert!(!rig.tab.take_midi_rescan());
        // Every open asks again, closed by a click outside or not.
        rig.click_row("Input device:");
        rig.click(pos2(10.0, 10.0));
        assert!(rig.tab.take_midi_rescan());
        // The other dropdowns do not.
        rig.click_row("Theme:");
        assert!(rig.tab.menu.is_some());
        assert!(!rig.tab.take_midi_rescan());
    }

    #[test]
    fn the_open_input_device_popup_follows_the_ports() {
        let mut rig = Rig::new();
        rig.click_row("Input device:");
        rig.frame(vec![]);
        let listed = |rig: &Rig| rig.texts().iter().any(|t| t == "Keys 49");
        assert!(rig.texts().iter().any(|t| t == "<no midi input>"));
        assert!(!listed(&rig));
        rig.tab.set_midi_ports(&ports(&["Keys 49 20:0"]));
        rig.frame(vec![]);
        assert!(listed(&rig));
        rig.tab.set_midi_ports(&[]);
        rig.frame(vec![]);
        assert!(!listed(&rig));
        assert!(rig.tab.menu.is_some());
    }

    #[test]
    fn the_input_device_button_shows_the_stored_name_even_when_it_is_not_connected() {
        let mut rig = Rig::new();
        let shown = |rig: &Rig, text: &str| rig.texts().iter().any(|t| t == text);
        assert!(shown(&rig, "Any available"));
        rig.settings.midi_device = "Gone Keys".into();
        rig.frame(vec![]);
        assert!(shown(&rig, "Gone Keys"));
        assert!(!shown(&rig, "Any available"));
        // The driver stays a disabled `Standard`.
        assert!(shown(&rig, "Standard"));
    }

    #[test]
    fn a_long_device_name_stays_inside_its_button() {
        let long = "Launchkey Mini MK3:Launchkey Mini MK3 LK Mini MIDI In";
        for ppp in [1.0, 2.0] {
            for (scale, tab_u) in [(DEFAULT_UI_SCALE_INDEX, WIDE_U), (SCALE_200, NARROW_U)] {
                let mut rig = Rig::at(ppp, scale, tab_u);
                rig.settings.midi_device = long.into();
                rig.frame(vec![]);
                let button = rig.rect_of(row_id(index("Input device:")));
                let inside: Vec<_> = (rig.text_rects().into_iter())
                    .filter(|(r, _)| button.contains(r.center()))
                    .collect();
                let [(name, shown)] = &inside[..] else {
                    panic!("ppp {ppp} scale {scale}: one text in the button, got {inside:?}");
                };
                assert!(
                    button.expand(1.0).contains_rect(*name),
                    "ppp {ppp} scale {scale}: `{shown}` {name:?} runs out of {button:?}"
                );
                // It shrank or was cut: it is not the plain 10 pt name.
                assert_ne!(shown, long, "ppp {ppp} scale {scale}");
            }
        }
        // A name that fits is drawn whole, centred.
        let mut rig = Rig::at(1.0, DEFAULT_UI_SCALE_INDEX, WIDE_U);
        rig.settings.midi_device = "Keys 49".into();
        rig.frame(vec![]);
        let button = rig.rect_of(row_id(index("Input device:")));
        let (name, shown) = (rig.text_rects().into_iter())
            .find(|(r, _)| button.contains(r.center()))
            .unwrap();
        assert_eq!(shown, "Keys 49");
        assert!((name.center().x - button.center().x).abs() < 1.0);
    }

    #[test]
    fn disabled_rows_take_no_input() {
        let mut rig = Rig::new();
        rig.settings = edited();
        for label in [
            "MIDI driver:",
            "Scan Bluetooth MIDI Devices",
            "Enable sharing files with nearby devices",
            "Share logs",
            "You are not signed in",
            "Sync automatically",
            "Sync only when connected to WiFi",
        ] {
            rig.click_row(label);
            assert_eq!(rig.tab.menu, None, "{label}");
            assert!(
                !rig.ctx
                    .read_response(row_id(index(label)))
                    .unwrap()
                    .sense
                    .senses_click(),
                "{label}"
            );
        }
        assert!(!rig.take_changed());
        assert_eq!(rig.settings, edited());
    }

    /// Settings away from the defaults.
    fn edited() -> Settings {
        Settings {
            midi_device: "Keys 49".into(),
            theme: 4,
            key_names: true,
            hardware_meters: false,
            metronome_level: 0.9,
            ..Settings::default()
        }
    }

    /// The confirm popup's items: the question (a header, row 0), then `Yes` and `No`.
    const YES: (usize, usize) = (0, 1);
    const NO: (usize, usize) = (0, 2);

    #[test]
    fn restore_default_settings_asks_first_and_only_yes_resets() {
        let mut rig = Rig::new();
        rig.settings = edited();
        rig.click_row("Restore default settings");
        // A Yes / No confirm.
        assert!(rig.tab.menu.is_some());
        // An area's first frame is egui's invisible sizing pass.
        rig.frame(vec![]);
        assert!(
            rig.texts()
                .iter()
                .any(|t| t == "Are you sure you want to restore default settings?")
        );
        assert_eq!(rig.settings, edited());
        let no = rig.rect_of(popup_id().with(NO)).center();
        rig.click(no);
        assert!(rig.tab.menu.is_none());
        // Escape closes it too.
        rig.click_row("Restore default settings");
        rig.key(egui::Key::Escape);
        assert!(rig.tab.menu.is_none());
        assert!(!rig.take_changed());
        assert_eq!(rig.settings, edited());

        rig.click_row("Restore default settings");
        let yes = rig.rect_of(popup_id().with(YES)).center();
        rig.click(yes);
        assert_eq!(rig.settings, Settings::default());
        assert!(rig.take_changed());
        assert!(rig.tab.menu.is_none());
    }

    /// `Restore default settings` leaves the demo songs done: the next start does not write them again.
    #[test]
    fn restore_default_settings_keeps_the_demo_songs_restored() {
        let mut rig = Rig::new();
        rig.settings = Settings {
            demo_songs_restored: true,
            ..edited()
        };
        rig.click_row("Restore default settings");
        let yes = rig.rect_of(popup_id().with(YES)).center();
        rig.click(yes);
        let kept = Settings {
            demo_songs_restored: true,
            ..Settings::default()
        };
        assert_eq!(rig.settings, kept);
    }

    /// `Restore demo songs`: the app writes the demos, once a tap, with no question first.
    #[test]
    fn restore_demo_songs_asks_the_app_once_a_tap() {
        let mut rig = Rig::new();
        assert!(!rig.tab.take_restore_demos());
        rig.click_row("Restore demo songs");
        assert!(rig.tab.take_restore_demos());
        assert!(!rig.tab.take_restore_demos(), "asked once");
        assert_eq!(rig.tab.menu, None, "no question");
        assert!(!rig.take_changed(), "no setting changes");
    }

    #[test]
    fn a_secondary_click_on_a_settings_toggle_opens_no_menu() {
        let mut rig = Rig::new();
        let at = rig.rect_of(row_id(index("Set middle C to C4"))).center();
        rig.frame(vec![Event::PointerMoved(at)]);
        for pressed in [true, false] {
            rig.frame(vec![Event::PointerButton {
                pos: at,
                button: PointerButton::Secondary,
                pressed,
                modifiers: Default::default(),
            }]);
        }
        rig.frame(vec![]);
        assert!(
            !rig.texts().iter().any(|t| t.contains("MIDI CC")),
            "{:?}",
            rig.texts()
        );
        assert_eq!(rig.settings, Settings::default());
    }

    #[test]
    fn a_secondary_click_on_the_level_slider_offers_reset_only() {
        // A slider's menu is `Reset`, then the parameter items only if it is bound to a parameter.
        let mut rig = Rig::new();
        let at = rig.rect_of(row_id(index("Metronome level"))).center();
        rig.frame(vec![Event::PointerMoved(at)]);
        for pressed in [true, false] {
            rig.frame(vec![Event::PointerButton {
                pos: at,
                button: PointerButton::Secondary,
                pressed,
                modifiers: Default::default(),
            }]);
        }
        rig.frame(vec![]);
        let texts = rig.texts();
        assert!(texts.iter().any(|t| t == "Reset"), "{texts:?}");
        assert!(
            !texts.iter().any(|t| t.contains("MIDI CC")
                || t.contains("Mod Wheel")
                || t.contains("automation")),
            "{texts:?}"
        );
    }

    #[test]
    fn an_open_popup_is_dropped_while_the_tab_is_hidden() {
        let mut rig = Rig::new();
        rig.click_row("Theme:");
        assert!(rig.tab.menu.is_some());
        rig.hidden = true;
        rig.frame(vec![]);
        rig.hidden = false;
        rig.frame(vec![]);
        assert!(rig.tab.menu.is_none());
        assert!(
            rig.ctx
                .read_response(popup_id().with((0_usize, 0_usize)))
                .is_none()
        );
    }

    #[test]
    fn the_level_slider_fills_from_its_low_end() {
        let rect = Rect::from_min_size(pos2(0.0, 0.0), vec2(500.0, 40.0));
        let c = level_control(Id::new("level"), rect, egui::Color32::WHITE);
        // Kit A reads a default of exactly 0.5 as bipolar; a level is not.
        assert!(!c.bipolar);
        assert_eq!(c.default, Settings::default().metronome_level);
    }

    #[test]
    fn dragging_the_metronome_slider_sets_the_level() {
        let mut rig = Rig::new();
        let slider = rig.rect_of(row_id(index("Metronome level")));
        // The slider spans the column under the label line.
        assert!(slider.width() > 14.0 * 32.0, "{slider:?}");
        // The thumb follows the pointer along the track, 1 U shorter than the slider: a quarter of it left takes a
        // quarter off.
        let quarter = (slider.width() - 32.0) / 4.0;
        let from = slider.center();
        rig.frame(vec![Event::PointerMoved(from)]);
        rig.button(from, true);
        for k in 1..=4 {
            rig.frame(vec![Event::PointerMoved(
                from - vec2(quarter * k as f32 / 4.0, 0.0),
            )]);
        }
        rig.button(from - vec2(quarter, 0.0), false);
        assert!(
            (rig.settings.metronome_level - 0.25).abs() < 1e-4,
            "{}",
            rig.settings.metronome_level
        );
        assert!(rig.take_changed());
    }

    #[test]
    fn the_full_width_level_slider_follows_the_pointer_over_its_whole_track() {
        // v += d / L, L the track length, so the long track needs a long drag: 4 U, the old kit's full range, is only a
        // fraction of it.
        let mut rig = Rig::new();
        rig.settings.metronome_level = 0.0;
        let slider = rig.rect_of(row_id(index("Metronome level")));
        assert!(slider.width() > 14.0 * 32.0, "{slider:?}");
        let from = slider.center();
        rig.frame(vec![Event::PointerMoved(from)]);
        rig.button(from, true);
        for dx in [32.0, 64.0, 96.0, 128.0] {
            rig.frame(vec![Event::PointerMoved(from + vec2(dx, 0.0))]);
        }
        rig.button(from + vec2(128.0, 0.0), false);
        let track = slider.width() - 32.0;
        assert!(
            (rig.settings.metronome_level - 128.0 / track).abs() < 1e-5,
            "{}",
            rig.settings.metronome_level
        );
    }

    #[test]
    fn the_list_scrolls_to_reach_the_last_rows() {
        let mut rig = Rig::with_height(540.0);
        let last = row_id(index("Restore default settings"));
        let below = |rig: &Rig| {
            rig.ctx
                .read_response(last)
                .is_none_or(|r| r.rect.top() >= 540.0)
        };
        assert!(below(&rig), "the list fits without scrolling");
        let over = pos2(800.0, 300.0);
        rig.frame(vec![Event::PointerMoved(over)]);
        for _ in 0..10 {
            rig.frame(vec![Event::MouseWheel {
                unit: egui::MouseWheelUnit::Point,
                delta: vec2(0.0, -500.0),
                modifiers: Default::default(),
            }]);
        }
        rig.frame(vec![]);
        let rect = rig.rect_of(last);
        assert!(rect.bottom() <= 540.0 && rect.top() >= 0.0, "{rect:?}");
        rig.settings.key_names = true;
        rig.click(rect.center());
        let yes = rig.rect_of(popup_id().with(YES)).center();
        rig.click(yes);
        assert_eq!(rig.settings, Settings::default());
    }

    #[test]
    fn the_original_content_rows_show_the_folder_and_open_it_on_the_desktop_only() {
        let folder = std::path::PathBuf::from("/home/me/.local/share/honeydew/Original Content");
        let mut rig = Rig::at(1.0, DEFAULT_UI_SCALE_INDEX, 20.0);
        // Without a content folder (the web) the rows are not there.
        assert!(
            !rig.texts()
                .iter()
                .any(|t| t.starts_with("Original content"))
        );
        assert!(!rig.texts().contains(&OPEN_FOLDER.to_owned()));
        rig.tab.content = Some(folder.clone());
        rig.frame(vec![]);
        rig.frame(vec![]);
        let texts = rig.texts();
        assert!(texts.contains(&"Original content:".to_owned()), "{texts:?}");
        assert!(texts.contains(&OPEN_FOLDER.to_owned()));
        // The path, shortened in the middle if it must be, keeps its end.
        assert!(
            texts.iter().any(|t| t.ends_with("Original Content")),
            "{texts:?}"
        );
        assert_eq!(rig.tab.take_reveal(), None);
        let at = rig
            .rect_of(row_id(index("Original content folder")))
            .center();
        rig.click(at);
        assert_eq!(rig.tab.take_reveal(), Some(folder));
        assert_eq!(rig.tab.take_reveal(), None, "asked once");
        assert!(!rig.changed, "no setting changes");
    }
}
