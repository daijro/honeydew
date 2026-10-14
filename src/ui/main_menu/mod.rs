//! The main menu, opened from the app logo. It is a `menuFill` panel on the right of the screen, 20 U wide or the whole
//! screen if that is narrower:
//! - a header with the logo glyph, [`APP_NAME`] over our version string, and the light `Help` and `Quit` circles;
//! - the tab row `SONGS PROJECT SETTINGS` across the panel in equal cells, the active tab underlined in `text`;
//! - the active tab below.
//!
//! [`MainMenu::show`] paints it and returns what the user asked for as a [`MainMenuAction`]; the app routes those
//! (opening, saving, quitting …). A click outside the panel, on the header logo, or Escape closes the menu. SETTINGS
//! shows the [`SettingsTab`] in [`shell_layout`]'s content rect. `Help` is disabled.

use eframe::egui::{
    Align, Align2, Color32, Id, Painter, Pos2, Rect, Response, RichText, Sense, Stroke, TextEdit,
    Ui, emath::TSTransform, pos2, vec2,
};

use crate::APP_NAME;
use crate::icons::{IconId, draw_icon};
use crate::metrics::{FontSize, Metrics};
use crate::settings::Settings;
use crate::theme::DISABLED_ALPHA;
use crate::ui::buttons::{boxed_circle_button, filled_circle_button, outline_circle_button};
use crate::ui::settings_tab::SettingsTab;
use crate::ui::widgets::{Button, button, text_label};
use crate::ui::{UiCtx, anim};

mod file_ops;
pub mod library;
mod project;
mod songs;

pub(crate) use file_ops::rename_shadowed_folders;
#[cfg(not(target_arch = "wasm32"))]
pub use file_ops::reveal;
pub(crate) use file_ops::{FILE_MANAGER, FileActions, MenuState, Opt, Step, delete, options_menu};
pub use library::{MemoryLibrary, SONGS_FOLDER, SongEntry, SongLibrary};
#[cfg(not(target_arch = "wasm32"))]
pub use library::{NativeLibrary, songs_dir};
pub use project::{ProjectInfo, TEMPO_RANGE, duration, link_url, meta_secs, meta_time, tempo_step};
pub use songs::Sort;
pub(crate) use songs::bin_crumb;
pub use songs::visible;

// Sizes in U.
const PANEL_W: f32 = 20.0;
const HEADER_H: f32 = 1.75;
const TAB_H: f32 = 1.25;
/// Inset of the header, tab strip and content from the panel's edges, and the gap under the tab strip.
const PAD: f32 = 0.5;
const GAP: f32 = 0.25;
/// `Help` and `Quit`.
const HEADER_BUTTON_D: f32 = 1.5;
/// The pressed tab's (and row's) lighter box: `text` at this alpha over the panel.
const PRESSED_ALPHA: f32 = 0.15;
/// A text field's empty-field hint and selection fill: `text` at this alpha. Not a disabled state (that is
/// [`DISABLED_ALPHA`]), though the same value.
const HINT_ALPHA: f32 = 0.3;

/// The version shown under the app name.
pub fn version() -> String {
    format!("Version {}", env!("CARGO_PKG_VERSION"))
}

/// The main menu's tabs, in strip order.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum Tab {
    #[default]
    Songs,
    Project,
    /// The settings page ([`SettingsTab`]).
    Settings,
}

impl Tab {
    pub const ALL: [Tab; 3] = [Tab::Songs, Tab::Project, Tab::Settings];

    pub fn label(self) -> &'static str {
        match self {
            Tab::Songs => "SONGS",
            Tab::Project => "PROJECT",
            Tab::Settings => "SETTINGS",
        }
    }
}

/// What the user asked the main menu for. The app routes them; the menu changes nothing outside itself, except the
/// metadata texts it edits in [`ProjectInfo`].
#[derive(Clone, Debug, PartialEq)]
pub enum MainMenuAction {
    /// Close the menu: a click outside the panel, on the header logo, or Escape.
    Close,
    /// The header's `Help` (disabled).
    Help,
    /// The header's `Quit` (disabled on the web).
    Quit,
    /// SONGS `New` (or its menu's `Empty`), then the Song options dialog's `OK`: a new empty song with these options.
    NewSong(crate::ui::song_options::SongOptions),
    /// A template picked from SONGS `New`'s menu: a new untitled song from the `.songtmpl` at this path, in
    /// [`crate::content::factory`] or in the library ([`SongLibrary::templates`]).
    NewFromTemplate(std::path::PathBuf),
    /// SONGS or PROJECT `Save`.
    Save,
    /// PROJECT `Save New`: save under a new name.
    SaveNew,
    /// SONGS `Import`: the operating system's file picker.
    Import,
    /// A song or an archive in the SONGS list: open the file at this [`SongEntry::path`] (read it with
    /// [`SongLibrary::read`]).
    Open(std::path::PathBuf),
    /// SONGS `Unzip` on the archive at this [`SongEntry::path`]: extract it, and show how it went.
    Unzip(std::path::PathBuf),
    /// PROJECT `-` / `+`: the new tempo in BPM, already kept in [`TEMPO_RANGE`].
    SetTempo(f64),
    /// PROJECT `Reset` next to the time spent. The app asks, then sets it to 0.
    ResetTimeSpent,
    /// PROJECT `Hide` next to the time spent. The app asks, then sets it to −1 (META@256).
    HideTimeSpent,
    /// PROJECT `Lock metadata forever`. The app asks, then sets the song's lock (META@776), which
    /// [`ProjectInfo::metadata_locked`] shows.
    LockMetadata,
    /// PROJECT `Open` next to the online link: open this address ([`link_url`]) in the browser.
    OpenLink(String),
    /// A SONGS file action failed: show this message with `OK`.
    Message(String),
    /// SONGS `Delete permanently`: ask this question, `Yes` / `No`, and hand the answer to
    /// [`MainMenu::delete_answered`].
    ConfirmDelete(String),
    /// PROJECT `Quick Render`: render the last audio format with the current render options, no dialog.
    QuickRender,
    /// SETTINGS `Open folder`: show this folder (the Original Content folder) in the system's file manager.
    Reveal(std::path::PathBuf),
    /// PROJECT `Detach`: show the floating performance monitor and close the menu.
    DetachPerformance,
    /// PROJECT `Attach`, which `Detach` becomes while the monitor is shown: hide it, and leave the menu open.
    AttachPerformance,
    /// SETTINGS `Restore demo songs`: write our demo songs into the library again
    /// ([`crate::content::restore_demo_songs`]).
    RestoreDemoSongs,
}

/// Where the menu's parts go on a screen.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct ShellLayout {
    /// The right of the screen: 20 U wide, or the whole screen if that is narrower.
    pub panel: Rect,
    pub header: Rect,
    /// Directly under the header, across the whole panel.
    pub tabs: Rect,
    /// Where the active tab draws: the settings page goes here.
    pub content: Rect,
}

/// Lays the menu out on `screen`.
pub fn shell_layout(screen: Rect, m: &Metrics) -> ShellLayout {
    let panel = screen.with_min_x(screen.right() - m.u(PANEL_W).min(screen.width()));
    let inner = panel.shrink(m.u(PAD));
    let header = inner
        .with_min_y(panel.top())
        .with_max_y(panel.top() + m.u(HEADER_H));
    let tabs = Rect::from_x_y_ranges(
        panel.x_range(),
        header.bottom()..=header.bottom() + m.u(TAB_H),
    );
    let content = inner.with_min_y(tabs.bottom() + m.u(GAP));
    ShellLayout {
        panel,
        header,
        tabs,
        content,
    }
}

/// The main menu's state: the active tab and each tab's own state.
#[derive(Default)]
pub struct MainMenu {
    pub tab: Tab,
    songs: songs::SongsTab,
    project: project::ProjectTab,
}

impl MainMenu {
    /// A menu showing `tab`.
    pub fn new(tab: Tab) -> Self {
        Self {
            tab,
            ..Self::default()
        }
    }

    /// Shows PROJECT and scrolls its Performance block into view as it draws (the CPU / RAM labels' tap).
    pub fn show_performance(&mut self) {
        self.project.show_performance();
    }

    /// Reads the SONGS folder again on the next frame: call it after the library changed (a song was saved).
    pub fn refresh(&mut self) {
        self.songs.refresh();
    }

    /// The factory whose `Templates` SONGS `New`'s menu lists ([`crate::content::factory`] until set; tests set a fake).
    pub fn set_factory(&mut self, factory: &'static crate::content::Factory) {
        self.songs.factory = Some(factory);
    }

    /// The folder open in the SONGS tab, `None` at its root.
    pub fn songs_folder(&self) -> Option<&std::path::Path> {
        self.songs.folder()
    }

    /// The answer to [`MainMenuAction::ConfirmDelete`]'s question: `yes` deletes for good. Returns a failure's
    /// [`MainMenuAction::Message`].
    pub fn delete_answered(
        &mut self,
        yes: bool,
        library: &mut dyn SongLibrary,
    ) -> Option<MainMenuAction> {
        self.songs.delete_answered(yes, library)
    }

    /// What the SONGS file actions moved (`Some` where to) or deleted since the last call, oldest first, for the
    /// open song's file to follow ([`ProjectFiles::path_moved`](crate::project_files::ProjectFiles::path_moved)).
    pub fn take_moves(&mut self) -> Vec<(std::path::PathBuf, Option<std::path::PathBuf>)> {
        self.songs.take_moves()
    }

    /// Paints the menu over `ui`'s rect (the screen) and handles its input. `library` feeds the SONGS tab (whose file
    /// actions change it), `project` the PROJECT tab, and `settings` the SETTINGS tab (the tab's state and the
    /// settings it edits). Returns what the user asked for this frame.
    pub fn show(
        &mut self,
        ui: &mut Ui,
        cx: &UiCtx,
        library: &mut dyn SongLibrary,
        project: &mut ProjectInfo,
        settings: (&mut SettingsTab, &mut Settings),
    ) -> Option<MainMenuAction> {
        let m = cx.m;
        let c = &cx.theme.colors;
        let screen = ui.max_rect();
        let id = Id::new("main_menu");
        let l = shell_layout(screen, m);
        // The opening slide: the panel is drawn shifted right by trunc(u⁴·W) device px while the screen dims to
        // α 0.6·(1 − u⁴); a tab change slides the page in again. Only the drawing moves: the touch areas are at rest
        // from the first frame, and the handlers never check the animation.
        let ctx = ui.ctx().clone();
        let (n, page_n) = (
            anim::frame(&ctx, id),
            anim::frame(&ctx, id.with(("page", self.tab as u8))),
        );
        let ease = |n| anim::menu_slide(n).powi(4);
        let shift = |n| (ease(n) * l.panel.width() * m.ppp()).trunc() / m.ppp();
        // The page counts from the menu's opening, or from its tab's change since.
        let page_dx = if page_n < n { shift(page_n) } else { 0.0 };
        let dx = shift(n);
        if dx > 0.0 || page_dx > 0.0 || ease(n) > 0.0 {
            ctx.request_repaint();
        }
        // Modal: everything left of the panel closes it ([`Self::panel`]), and undo and redo wait.
        crate::ui::modal_shown(&ctx);
        let dim = anim::DIM_ALPHA * (1.0 - ease(n));
        (ui.painter()).rect_filled(screen, 0.0, anim::dim(c.background.to_color32(), dim));
        let slide = |x: f32| TSTransform::from_translation(vec2(x, 0.0));
        ui.with_visual_transform(slide(dx), |ui| {
            self.panel(ui, cx, &l, id, page_dx, library, project, settings)
        })
        .inner
    }

    /// The panel laid out at rest in `l`, its page drawn `page_dx` right of its rest (the tab change's slide).
    #[allow(clippy::too_many_arguments)]
    fn panel(
        &mut self,
        ui: &mut Ui,
        cx: &UiCtx,
        l: &ShellLayout,
        id: Id,
        page_dx: f32,
        library: &mut dyn SongLibrary,
        project: &mut ProjectInfo,
        settings: (&mut SettingsTab, &mut Settings),
    ) -> Option<MainMenuAction> {
        let m = cx.m;
        let c = &cx.theme.colors;
        let text = c.text.to_color32();
        let screen = ui.max_rect();
        let mut action = None;

        // Everything left of the panel closes the menu; the panel itself swallows what its widgets do not take.
        let outside = screen.with_max_x(l.panel.left());
        if ui
            .interact(outside, id.with("outside"), Sense::click_and_drag())
            .clicked()
        {
            action = Some(MainMenuAction::Close);
        }
        ui.interact(l.panel, id.with("panel"), Sense::click_and_drag());
        // Panel fills: the body, the full panel, is `menuFill`; the header (1.75 U) is
        // `moduleFill`; the tab row is `menuFill` too, so it has none of its own.
        ui.painter()
            .rect_filled(l.panel, 0.0, c.menu_fill.to_color32());
        let header = Rect::from_x_y_ranges(l.panel.x_range(), l.header.y_range());
        ui.painter()
            .rect_filled(header, 0.0, c.module_fill.to_color32());

        // Header.
        let logo = Rect::from_min_size(l.header.min, vec2(l.header.height(), l.header.height()));
        draw_icon(ui.painter(), logo, IconId::AppLogo, text, 1.0, m);
        if ui.interact(logo, id.with("logo"), Sense::click()).clicked() {
            action = Some(MainMenuAction::Close);
        }
        let d = m.u(HEADER_BUTTON_D);
        let names = Rect::from_x_y_ranges(
            logo.right() + m.u(GAP)..=l.header.right() - 2.0 * d - 2.0 * m.u(GAP),
            l.header.y_range(),
        );
        let (upper, lower) = names.split_top_bottom_at_fraction(0.55);
        text_label(
            ui,
            cx,
            upper,
            APP_NAME,
            FontSize::Pt12,
            Align2::LEFT_BOTTOM,
            text,
        );
        text_label(
            ui,
            cx,
            lower,
            &version(),
            FontSize::Pt8,
            Align2::LEFT_TOP,
            text,
        );
        for (i, (name, act, enabled)) in [
            ("Quit", MainMenuAction::Quit, !cfg!(target_arch = "wasm32")),
            ("Help", MainMenuAction::Help, false),
        ]
        .into_iter()
        .enumerate()
        {
            let center = pos2(
                l.header.right() - d / 2.0 - i as f32 * (d + m.u(GAP)),
                l.header.center().y,
            );
            let circle = Circle {
                enabled,
                ..Circle::new(id.with(name), center, d, name)
            };
            if circle_button(ui, cx, &circle) {
                action = Some(act);
            }
        }

        // Tab row: equal cells, each label 10 pt and centred in its cell. A label is never
        // shrunk, cut or clipped to its cell: one wider than its cell runs evenly over its neighbours.
        let font = m.font_id(FontSize::Pt10);
        let cell_w = l.tabs.width() / Tab::ALL.len() as f32;
        for (i, tab) in Tab::ALL.into_iter().enumerate() {
            let left = l.tabs.left() + i as f32 * cell_w;
            let cell = Rect::from_x_y_ranges(left..=left + cell_w, l.tabs.y_range());
            let galley = ui
                .painter()
                .layout_no_wrap(tab.label().to_owned(), font.clone(), text);
            let response =
                crate::ui::focus::interact(ui, cell, id.with(("tab", i)), Sense::click());
            let painter = ui.painter();
            let label_rect = Rect::from_center_size(cell.center(), galley.size());
            if response.is_pointer_button_down_on() {
                pressed_box(painter, label_rect.expand(m.u(GAP)), cx);
            }
            painter.galley(label_rect.min, galley, text);
            if tab == self.tab {
                let y = label_rect.bottom() + m.u(0.1);
                let line = Rect::from_x_y_ranges(label_rect.x_range(), y..=y + m.t());
                painter.rect_filled(line, 0.0, text);
            }
            if response.clicked() {
                self.tab = tab;
            }
        }

        // The active tab.
        let page = TSTransform::from_translation(vec2(page_dx, 0.0));
        let tab_action = ui
            .with_visual_transform(page, |ui| match self.tab {
                Tab::Songs => {
                    // Its sort is the stored `SongBrowserOrdering`, and a pick from its menu is stored there.
                    let (_, settings) = settings;
                    self.songs.set_sort(settings.song_ordering());
                    let action = self.songs.show(ui, cx, l.content, library);
                    settings.song_browser_ordering = self.songs.sort.id();
                    action
                }
                Tab::Project => self.project.show(ui, cx, l.content, project),
                Tab::Settings => {
                    let (tab, settings) = settings;
                    // The page: the panel's width, not the content's inset ones (the column is 2 U in from its sides).
                    let page = l
                        .content
                        .with_min_x(l.panel.left())
                        .with_max_x(l.panel.right());
                    tab.show(ui, cx, page, settings);
                    let restore = tab.take_restore_demos();
                    (tab.take_reveal().map(MainMenuAction::Reveal))
                        .or(restore.then_some(MainMenuAction::RestoreDemoSongs))
                }
            })
            .inner;
        action = tab_action.or(action);
        // After the tabs: an open popup takes Escape first; in SONGS' browser it hides the search bar, else stops
        // select mode, before it closes the menu.
        if crate::ui::close_pressed(ui.ctx())
            && !(self.tab == Tab::Songs && self.songs.escape(ui.ctx()))
        {
            action = Some(MainMenuAction::Close);
        }
        action
    }
}

/// How a circle button is drawn.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum CircleStyle {
    /// A `text` disc with `menuFill` ink.
    Light,
    /// A `text` ring with `text` ink.
    Outline,
    /// A `text` disc with a 2 px `moduleFill` rim and `moduleFill` ink: PROJECT's `Detach`, `Hide` and `Reset`.
    Rimmed,
}

/// A round button with a short label or an icon.
struct Circle<'a> {
    id: Id,
    center: Pos2,
    diameter: f32,
    label: &'a str,
    /// Drawn instead of the label.
    icon: Option<IconId>,
    font: FontSize,
    style: CircleStyle,
    /// Disabled: no input, drawn at [`DISABLED_ALPHA`] (only the ink, on a light disc).
    enabled: bool,
    /// The touch area, a square this many points across (a [`CircleStyle::Rimmed`] button's box), instead of the
    /// disc's own.
    hit: Option<f32>,
}

impl<'a> Circle<'a> {
    /// A light circle with a `FontSize::Pt8` label.
    fn new(id: Id, center: Pos2, diameter: f32, label: &'a str) -> Self {
        Self {
            id,
            center,
            diameter,
            label,
            icon: None,
            font: FontSize::Pt8,
            style: CircleStyle::Light,
            enabled: true,
            hit: None,
        }
    }
}

/// Draws `c` and returns whether it was clicked (on the disc).
fn circle_button(ui: &mut Ui, cx: &UiCtx, c: &Circle) -> bool {
    let m = cx.m;
    let colors = &cx.theme.colors;
    let text = colors.text.to_color32();
    let dim = |color: Color32| {
        if c.enabled {
            color
        } else {
            color.gamma_multiply(DISABLED_ALPHA)
        }
    };
    let (response, ink) = match c.style {
        CircleStyle::Light => (
            filled_circle_button(ui, c.id, c.center, c.diameter, text),
            dim(colors.menu_fill.to_color32()),
        ),
        CircleStyle::Rimmed => {
            let module_fill = colors.module_fill.to_color32();
            let px = 1.0 / m.ppp();
            let radius = c.diameter / 2.0 - px;
            let rim = Stroke::new(2.0 * px, module_fill);
            if let Some(hit) = c.hit {
                let response = boxed_circle_button(ui, c.id, c.center, c.diameter, hit, text);
                ui.painter().circle_stroke(c.center, radius, rim);
                let rect = Rect::from_center_size(c.center, vec2(c.diameter, c.diameter));
                text_label(
                    ui,
                    cx,
                    rect,
                    c.label,
                    c.font,
                    Align2::CENTER_CENTER,
                    dim(module_fill),
                );
                return c.enabled && response.clicked();
            }
            let response = filled_circle_button(ui, c.id, c.center, c.diameter, text);
            ui.painter().circle_stroke(c.center, radius, rim);
            (response, dim(module_fill))
        }
        CircleStyle::Outline => {
            let ink = dim(text);
            (
                outline_circle_button(ui, c.id, c.center, c.diameter, ink, m),
                ink,
            )
        }
    };
    let rect = response.rect();
    match c.icon {
        Some(icon) => draw_icon(ui.painter(), rect, icon, ink, 0.6, m),
        None => text_label(ui, cx, rect, c.label, c.font, Align2::CENTER_CENTER, ink),
    }
    c.enabled && response.clicked()
}

/// A light rectangular button: a `text` rect with a `menuFill` `FontSize::Pt10` label, dimmed and inert when
/// disabled. Returns whether it was clicked.
fn light_button(ui: &mut Ui, cx: &UiCtx, id: Id, rect: Rect, label: &str, enabled: bool) -> bool {
    let colors = &cx.theme.colors;
    let mut b = Button::new(id, rect, label, false, colors);
    if !enabled {
        b.text_on = b.text_on.gamma_multiply(DISABLED_ALPHA);
        b.disabled = true;
    }
    button(ui, cx, &b, true).clicked()
}

/// The lighter box behind a pressed tab or row.
fn pressed_box(painter: &Painter, rect: Rect, cx: &UiCtx) {
    let box_color = cx
        .theme
        .colors
        .text
        .to_color32()
        .gamma_multiply(PRESSED_ALPHA);
    painter.rect_filled(rect, 0.0, box_color);
}

/// A dark (`background`) one-line text field filling `rect`, in `text` at `FontSize::Pt10`, with `hint` dimmed while
/// it is empty. `edit` brings the text, id, limits and whether it is interactive.
fn text_field(ui: &mut Ui, cx: &UiCtx, rect: Rect, hint: &str, edit: TextEdit) -> Response {
    let m = cx.m;
    let colors = &cx.theme.colors;
    let text = colors.text.to_color32();
    ui.painter()
        .rect_filled(rect, 0.0, colors.background.to_color32());
    let edit = edit
        .frame(false)
        .font(m.font_id(FontSize::Pt10))
        .text_color(text)
        .hint_text(RichText::new(hint).color(text.gamma_multiply(HINT_ALPHA)))
        .margin(vec2(m.u(GAP), 0.0))
        .vertical_align(Align::Center)
        .desired_width(f32::INFINITY);
    ui.scope(|ui| {
        let v = ui.visuals_mut();
        v.selection.bg_fill = text.gamma_multiply(HINT_ALPHA);
        v.selection.stroke.color = text;
        v.text_cursor.stroke.color = text;
        ui.put(rect, edit)
    })
    .inner
}

/// Applies this frame's wheel and drag over `area` to `offset` (points scrolled up), keeps it in
/// `0..=max(0, content_h - area_h)`, and returns it snapped to whole device pixels for painting. Widgets in the area
/// that sense clicks only let drags through to it.
fn scroll(ui: &mut Ui, id: Id, area: Rect, content_h: f32, offset: &mut f32) -> f32 {
    let drag = ui.interact(area, id, Sense::drag());
    let mut moved = drag.drag_delta().y;
    if ui.rect_contains_pointer(area) {
        moved += ui.input(|i| i.smooth_scroll_delta.y);
    }
    *offset = (*offset - moved).clamp(0.0, (content_h - area.height()).max(0.0));
    let ppp = ui.ctx().pixels_per_point();
    (*offset * ppp).round() / ppp
}

#[cfg(test)]
mod tests;
