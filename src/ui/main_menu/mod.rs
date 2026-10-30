//! The main menu, opened from the app logo. It is a `menuFill` panel on the right of the screen, 20 U wide or the whole
//! screen if that is narrower:
//! - a header with `Help` at the left, the close X at the right, and centred between them the logo glyph, [`APP_NAME`]
//!   and our version;
//! - the tab row `SONGS PROJECT SETTINGS` across the panel in equal cells, the active tab underlined in `text`;
//! - the active tab below.
//!
//! [`MainMenu::show`] paints it and returns what the user asked for as a [`MainMenuAction`]; the app routes those
//! (opening, saving …). The close X, a click outside the panel, or Escape closes the menu; the header itself does not.
//! SETTINGS shows the [`SettingsTab`] in [`shell_layout`]'s content rect. `Help` opens the Help menu.

use eframe::egui::{
    Align, Align2, Color32, Id, Painter, Pos2, Rect, Response, RichText, Sense, Stroke, TextEdit,
    Ui, emath::TSTransform, pos2, vec2,
};

use crate::APP_NAME;
use crate::icons::{IconId, draw_icon};
use crate::metrics::{FontSize, Metrics};
use crate::settings::Settings;
use crate::theme::DISABLED_ALPHA;
use crate::ui::buttons::{
    boxed_circle_button, close_x, filled_circle_button, outline_circle_button,
};
use crate::ui::popup::{PopupItem, PopupResult};
use crate::ui::settings_tab::SettingsTab;
use crate::ui::widgets::{Button, button, text_label, truncated};
use crate::ui::{UiCtx, anim};

mod file_ops;
pub mod library;
mod project;
mod songs;

pub(crate) use file_ops::rename_shadowed_folders;
#[cfg(not(target_arch = "wasm32"))]
pub use file_ops::reveal;
pub(crate) use file_ops::{
    CONVERTING, FILE_MANAGER, FileActions, MenuState, Opt, Step, delete, options_menu,
};
pub use library::{MemoryLibrary, SONGS_FOLDER, SongEntry, SongLibrary};
#[cfg(not(target_arch = "wasm32"))]
pub use library::{NativeLibrary, app_data_dir, songs_dir, user_dir};
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
/// `Help`'s and the close X's cells at the header's ends.
const HEADER_CELL: f32 = 2.0;
/// The close X's icon size factor.
const CLOSE_ICON: f32 = 0.6;
/// A tab under the mouse: `text` at this alpha over its cell.
const TAB_HOVER_ALPHA: f32 = 0.05;
/// A tab label too wide for its cell keeps this many characters after its middle ellipsis.
const TAB_TAIL_K: usize = 5;
/// The pressed tab's (and row's) lighter box: `text` at this alpha over the panel.
const PRESSED_ALPHA: f32 = 0.15;
/// A text field's empty-field hint and selection fill: `text` at this alpha. Not a disabled state (that is
/// [`DISABLED_ALPHA`]), though the same value.
const HINT_ALPHA: f32 = 0.3;

/// The version shown after the app name.
pub fn version() -> String {
    env!("CARGO_PKG_VERSION").to_owned()
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
    /// Close the menu: the close X, a click outside the panel, or Escape.
    Close,
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
    /// SETTINGS `Open folder` or `Open user folder`: show the folder this file is in (the Original Content folder's
    /// README, the user folder's `My Songs`) in the system's file manager.
    Reveal(std::path::PathBuf),
    /// SETTINGS `Restore demo songs`: write our demo songs into the library again
    /// ([`crate::content::restore_demo_songs`]).
    RestoreDemoSongs,
    /// Help → `What's new`: show the What's New dialog.
    WhatsNew,
}

/// The Help menu's links, after `What's new`: the project's own pages.
pub const HELP_LINKS: [(&str, &str); 2] = [
    ("User manual", "https://github.com/daijro/honeydew#readme"),
    (
        "Report a problem",
        "https://github.com/daijro/honeydew/issues",
    ),
];

/// The Help menu at `at`: `What's new` (item 0), then [`HELP_LINKS`].
fn help_menu(ctx: &eframe::egui::Context, cx: &UiCtx, id: Id, at: Pos2) -> PopupResult<usize> {
    let labels = std::iter::once("What's new").chain(HELP_LINKS.iter().map(|l| l.0));
    let items: Vec<PopupItem<usize>> = (labels.enumerate())
        .map(|(row, label)| PopupItem::new(row, 0, row, label))
        .collect();
    crate::ui::preset_picker::modal(ctx, cx, id, at, &items)
}

/// Where the menu's parts go on a screen.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct ShellLayout {
    /// The right of the screen: 20 U wide, or the whole screen if that is narrower or portrait
    /// ([`is_portrait`](crate::metrics::is_portrait)).
    pub panel: Rect,
    pub header: Rect,
    /// Directly under the header, across the whole panel.
    pub tabs: Rect,
    /// Where the active tab draws: the settings page goes here.
    pub content: Rect,
}

/// Lays the menu out on `screen`.
pub fn shell_layout(screen: Rect, m: &Metrics) -> ShellLayout {
    let w = match crate::metrics::is_portrait(screen) {
        true => screen.width(),
        false => m.u(PANEL_W).min(screen.width()),
    };
    let panel = screen.with_min_x(screen.right() - w);
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
    /// The Help menu, open at this point (the `Help` button's centre).
    help_menu: Option<Pos2>,
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
            anim::elapsed(&ctx, id),
            anim::elapsed(&ctx, id.with(("page", self.tab as u8))),
        );
        let ease = |t| anim::menu_slide(t).powi(4);
        let shift = |t| (ease(t) * l.panel.width() * m.ppp()).trunc() / m.ppp();
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

        // Header: `Help` in a 2 U cell at the left, the close X in one at the right, and between them, centred as a
        // group, the logo, our name and the version. The header itself is no button.
        let cell =
            |left: f32| Rect::from_x_y_ranges(left..=left + m.u(HEADER_CELL), header.y_range());
        let (help, close) = (cell(header.left()), cell(header.right() - m.u(HEADER_CELL)));
        // `Help`: plain text, opening the Help menu.
        text_label(
            ui,
            cx,
            help,
            "Help",
            FontSize::Pt10,
            Align2::CENTER_CENTER,
            text,
        );
        if crate::ui::focus::interact(ui, help, id.with("Help"), Sense::click()).clicked() {
            self.help_menu = Some(help.center());
        }
        let surface = (c.module_fill.to_color32(), 0.375);
        if close_x(ui, cx, id.with("close"), close, surface, CLOSE_ICON).clicked() {
            action = Some(MainMenuAction::Close);
        }
        self.title(
            ui,
            cx,
            header.with_min_x(help.right()).with_max_x(close.left()),
        );
        if let Some(at) = self.help_menu {
            let picked = help_menu(ui.ctx(), cx, id.with("help_menu"), at);
            if picked.close {
                self.help_menu = None;
            }
            action = picked.selected.map(|i| match i {
                0 => MainMenuAction::WhatsNew,
                i => MainMenuAction::OpenLink(HELP_LINKS[i - 1].1.into()),
            });
        }

        // Tab row: equal cells, each label 10 pt and centred in its cell, a middle ellipsis keeping 5 tail characters
        // when the cell is too narrow. The current tab is underlined (2t) along its cell's bottom; under the mouse a
        // cell lights (`text` α 0.05).
        let font = m.font_id(FontSize::Pt10);
        let cell_w = l.tabs.width() / Tab::ALL.len() as f32;
        for (i, tab) in Tab::ALL.into_iter().enumerate() {
            let left = l.tabs.left() + i as f32 * cell_w;
            let cell = Rect::from_x_y_ranges(left..=left + cell_w, l.tabs.y_range());
            let response =
                crate::ui::focus::interact(ui, cell, id.with(("tab", i)), Sense::click());
            let painter = ui.painter();
            if crate::ui::widgets::hovered(ui, &response) {
                painter.rect_filled(cell, 0.0, text.gamma_multiply(TAB_HOVER_ALPHA));
            }
            if response.is_pointer_button_down_on() {
                pressed_box(painter, cell, cx);
            }
            let label = cell.shrink2(vec2(m.u(GAP), 0.0));
            let center = Align2::CENTER_CENTER;
            truncated(
                painter,
                label,
                center,
                tab.label(),
                font.clone(),
                text,
                TAB_TAIL_K,
            );
            if tab == self.tab {
                let line = cell.with_min_y(cell.bottom() - 2.0 * m.t());
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

impl MainMenu {
    /// The header's centred group in `room` (between the two cells): the logo (a header-high square), 5 px, our name
    /// in the largest size from 18 pt down to 8 pt that leaves room for the rest, 0.25 U, then the version in 8 pt at
    /// α 0.6 on the name's baseline.
    fn title(&self, ui: &Ui, cx: &UiCtx, room: Rect) {
        let m = cx.m;
        let text = cx.theme.colors.text.to_color32();
        let painter = ui.painter();
        let layout =
            |s: &str, pt: f32, color| painter.layout_no_wrap(s.to_owned(), m.font_id_pt(pt), color);
        let version = layout(&version(), FontSize::Pt8.points(), text.gamma_multiply(0.6));
        let logo = room.height();
        let gaps = 5.0 / m.ppp() + m.u(GAP);
        let fixed = logo + gaps + version.size().x;
        let name = (8..=18)
            .rev()
            .map(|pt| layout(APP_NAME, pt as f32, text))
            .find(|g| fixed + g.size().x <= room.width())
            .unwrap_or_else(|| layout(APP_NAME, 8.0, text));
        let left = room.center().x - (fixed + name.size().x) / 2.0;
        let square = Rect::from_min_size(pos2(left, room.top()), vec2(logo, logo));
        draw_icon(painter, square, IconId::AppLogo, text, 1.0, m);
        let x = square.right() + 5.0 / m.ppp();
        let top = room.center().y - name.size().y / 2.0;
        // The version's baseline on the name's.
        let baseline =
            |g: &eframe::egui::Galley| (g.rows[0].row.glyphs.first()).map_or(0.0, |gl| gl.pos.y);
        let version_top = top + baseline(&name) - baseline(&version);
        let name_w = name.size().x;
        painter.galley(pos2(x, top), name, text);
        painter.galley(pos2(x + name_w + m.u(GAP), version_top), version, text);
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
