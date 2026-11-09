//! The main menu's SONGS tab: the left column of outline circles (`Back`, `Search`, `Sort`, `Select`, ≡), the
//! breadcrumb, the folder title and the song list, and the big light circles `New`, `Save` and `Import` at the right
//! edge.
//!
//! The folder title is large and bold ([`TITLE_FONT`], #73), with no gap above it: the breadcrumb's line is there only
//! inside a folder, so at the root the title is level with `Back`. One row height; the songs' names are in
//! [`ROW_FONT`], the folders' bold and a little larger, [`FOLDER_FONT`] (#79). The rows are the browsers'
//! ([`search_bar::paint_row`]): the current one is the song last opened here, or the folder holding it.
//!
//! `Back` at `My Songs` goes up to the user folder ([`SongLibrary::user_folder`], #73), titled [`crate::APP_NAME`]: its
//! folders (`My Songs`, `My Tracks`, `My Samples`, …, the bin, `Original Content`), which are places, never its files,
//! and nothing is made or pasted there. `Back` stops there. Below it every folder but `My Songs` lists every file, and
//! a tap on a file that opens no song (a render, a sample) selects it, for its actions. `My Songs` there is the root
//! again. Every file action stays inside the user folder.
//!
//! Folders come first, then songs and project archives (`.zip`, in the same row style), each group in the chosen order
//! (`A-Z` by default, case-insensitive). The search bar ([`SearchBar`]) keeps the rows whose name holds the query,
//! ignoring case. A click on a song or an archive opens it, a click on a folder goes in, and `Back` comes out again. In
//! select mode a click toggles the row instead.
//!
//! The browser (the left column, the search bar, the breadcrumb, the title and the list, and the big circles, which
//! are its `newSongInBrowser` / `saveSongInBrowser` actions) is a focus container; its list is
//! [`crate::ui::focus::SONGS`], where the focus goes each time the page shows and on `showMainBrowser`. `startSearch`
//! (Ctrl+F) shows the bar while the focus is inside the browser and no popup, name field or dialog is open; Escape
//! hides it, else stops select mode, before the menu closes ([`SongsTab::escape`]). Each showing of the page resets
//! the search. A non-empty query lists the songs below the open folder, recursively (the side browser's rule,
//! [`crate::ui::search_bar::Walk`]), with the folder's path from `My Songs` under the name. The bar's browser rect is
//! the content left of the big circles' column, so the bar stops short of them.
//!
//! A secondary tap (right click) on a row enters select mode, selects the row and opens the ≡ menu at the pointer.
//!
//! Preview (#106, a Honeydew addition): a tap that selects an audio file shows the sample pickers' strip
//! ([`crate::ui::picker_preview::SampleStrip`]) under the list, which shrinks by its height: the file's name, then, once
//! the app decoded it ([`MainMenuAction::Preview`], [`SongsPreview::decoded`]), its waveform, playing once from the
//! start on the preview player. A tap that leaves it unselected, a new pick, the folder read again (another folder, a
//! file action, the menu opening) and leaving SONGS stop it; the strip's own play button and waveform work as in the
//! pickers. A `.mid` tap still opens it as a song.
//!
//! Touch (#72, after Android reports): a long press on a row ([`gesture::long_press`]) enters select
//! mode and selects the row, without the menu (egui's own long touch opens none either); taps then add or remove rows. In select mode a bar under the list shows
//! `N selected` and the ≡ menu's enabled actions as buttons ([`BAR_H`]), wrapping into more lines when they do not fit.
//! `Delete` of more than one item asks first (`Move N items to Recently Deleted?`).
//!
//! [`MainMenu::show_render`](super::MainMenu::show_render) opens the user folder's folder that holds a render
//! (`My Tracks`), the render current: the render message's `Open My Tracks`.
//!
//! The ≡ menu has the file actions that apply to songs, archives and folders ([`file_ops`]); `Insert at playhead` never
//! does. `Send to playlist` is there while the selection holds audio files (#106): it is
//! [`MainMenuAction::SendToPlaylist`] of those, which the app makes as files dropped off the playlist are made (a new
//! audio track for each, its clip at the playhead, one undo step), closing the menu. `Unzip` is there while exactly one
//! archive is selected:
//! it is [`MainMenuAction::Unzip`], which the app does, and the list is read again once it has. `Create folder` and
//! `Rename` ask for the name in a field under the title (`Enter folder name`, `Enter name`): Enter does it, Escape or a
//! click elsewhere drops it. `Delete` copies into `Recently Deleted` (at its place below the user folder) and removes
//! the item; the bin is listed by the root once it exists and is a place, never part of a selection. Its breadcrumb
//! reads `Recently Deleted (automatically deleted after 30 days)`, shortened in the middle when the trail is wider than
//! its line. `Restore` puts an item back at its original place. `Delete permanently` asks first. A failure shows its
//! message. The folder is read again after every action.
//!
//! On the web `Export` downloads each song, and is disabled while a folder is selected (the browser cannot download
//! a folder); `Show in file browser...` is left out there, as there is no file manager. A phone's menu ends in `Send`
//! (#72), the selected files to the system's share sheet, disabled while a folder is selected.
//!
//! `New` opens its menu: `Empty`, the header `Templates`, then the template names in byte order ([`all_templates`]):
//! our factory's `Templates` folder ([`crate::content::factory`]) and, with imported content in the content folder
//! ([`crate::project_files::CONTENT_FOLDER`]), its templates ([`SongLibrary::templates`]); a name both have is listed
//! once, as ours. A template is [`MainMenuAction::NewFromTemplate`] of its file. Without any template `New` asks for
//! the empty song at once (a menu of `Empty` alone is no choice). The empty song (`New`, or `Empty`) comes through the
//! Song options dialog ([`SongOptionsDialog`]), drawn over the screen: `OK` makes it with the options, `Cancel` or
//! Escape makes none.
//!
//! `Import` opens its menu, `Import file` and `Import folder`, where a folder can be imported ([`IMPORT_FOLDER`]);
//! on the web it picks a file at once.
//!
//! `Save` is a swipe button: a tap asks for the name ([`MainMenuAction::SaveNew`]), and a swipe left of [`SWIPE_SAVE`]
//! or more quick-saves ([`MainMenuAction::Save`], Ctrl+S's). The circle follows the pointer left, at most
//! [`SWIPE_MAX`], and springs back when let go; out of the calm zone a mostly vertical drag does neither. Its tooltip is
//! `Swipe for quick save`, with Ctrl+S, the keyboard's quick save.
//!
//! With imported content in the content folder the root also lists its demo songs as the read-only folder
//! [`super::library::DEMO_SONGS`] (the library refuses any change in it).

use std::cmp::Ordering;
use std::path::{Path, PathBuf};

use eframe::egui::text::{CCursor, CCursorRange};
use eframe::egui::text_edit::TextEditState;
use eframe::egui::{
    Align2, Context, Id, Key, PointerButton, Pos2, Rect, Sense, TextEdit, Ui, pos2, vec2,
};

use super::file_ops::{FILE_MANAGER, FileActions, MenuState, Moved, Opt, Step, options_menu};
use super::library::{BIN_DAYS, RECENTLY_DELETED, SONGS_FOLDER, SongEntry, SongLibrary};
use super::{Circle, CircleStyle, MainMenuAction, circle_button, scroll, text_field};
use crate::content::{self, Factory};
use crate::icons::IconId;
use crate::metrics::FontSize;
use crate::project_files::is_audio;
use crate::sample_cache::Waveform;
use crate::ui::UiCtx;
use crate::ui::picker_preview::{Play, STRIP_H, SampleStrip};
use crate::ui::popup::{self, PopupItem};
use crate::ui::scroller::{self, Scroller};
use crate::ui::search_bar::{self, Found, ROW_H, RowLook, SearchBar, Walk, matching};
use crate::ui::song_options::{SongOptionsAction, SongOptionsDialog};
use crate::ui::widgets::{Button, button, text_label, truncated, truncated_bold};
use crate::ui::{anim, focus, gesture};

// Sizes in U.
/// The left column (outline circles) and the right column (big light circles): width, diameter and step.
const SIDE_COL_W: f32 = 2.0;
const SIDE_D: f32 = 1.5;
const SIDE_STEP: f32 = 1.75;
const BIG_COL_W: f32 = 2.5;
const BIG_D: f32 = 2.0;
const BIG_STEP: f32 = 2.5;
/// Breadcrumb line, folder title line, name field (with the gap under it) and list row heights. The title's line is
/// the left column's step, so at the root its text is level with `Back`.
const CRUMB_H: f32 = 0.75;
const TITLE_H: f32 = SIDE_STEP;
const SEARCH_H: f32 = 1.0;
/// The gap under the name field.
const FIELD_GAP: f32 = 0.25;
/// A song's row font, and a folder's: a third of the way to the title's (#79).
const ROW_FONT: FontSize = FontSize::Pt12;
const FOLDER_FONT: FontSize = FontSize::Pt14;
/// The folder title's font, set apart from the rows (#73).
const TITLE_FONT: FontSize = FontSize::Pt18;
/// The characters a too-long breadcrumb keeps after its middle ellipsis, at most: far more than the modules browser's
/// 5, as the nearest parent folder is what the trail is for. The middle ellipsis then settles at about half the line
/// for the end.
const CRUMB_TAIL_K: usize = 40;
/// The tail a too-long folder title keeps after its middle ellipsis: the browsers' titles' 5.
const NAME_TAIL_K: usize = 5;
/// The longest name, in characters.
const TEXT_LIMIT: usize = 255;
/// `Save`'s swipe: let go pulled this far left, it quick-saves; the circle follows the pointer at most `SWIPE_MAX`.
const SWIPE_SAVE: f32 = 0.75;
const SWIPE_MAX: f32 = 1.0;
/// Select mode's bar under the list: its buttons' height, its lines' step, the gap after the count and between buttons,
/// and a button's padding each side of its label.
const BAR_H: f32 = 1.0;
const BAR_STEP: f32 = 1.25;
const BAR_GAP: f32 = 0.25;
const BAR_PAD: f32 = 0.25;

/// `Import` has a menu with `Import folder`: everywhere but the web, which picks no folders to copy.
const IMPORT_FOLDER: bool = cfg!(not(target_arch = "wasm32"));

/// The `Sort` menu's orderings, as in the file browsers.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum Sort {
    #[default]
    AZ,
    ZA,
    /// The most recently changed first.
    Recent,
}

impl Sort {
    const ALL: [Sort; 3] = [Sort::AZ, Sort::ZA, Sort::Recent];

    /// Its stored `SongBrowserOrdering`: its place in the menu.
    pub fn id(self) -> u8 {
        self as u8
    }

    /// The ordering stored as `id`, if any.
    pub fn from_id(id: u8) -> Option<Self> {
        Self::ALL.get(usize::from(id)).copied()
    }

    fn label(self) -> &'static str {
        match self {
            Sort::AZ => "A-Z",
            Sort::ZA => "Z-A",
            Sort::Recent => "Recent",
        }
    }
}

/// The rows to show: the entries whose name holds `query` (ignoring case), folders first, then songs, each group in
/// `sort` order. Names compare without case, then byte by byte.
pub fn visible<'a>(entries: &'a [SongEntry], sort: Sort, query: &str) -> Vec<&'a SongEntry> {
    let query = query.to_lowercase();
    let mut rows: Vec<_> = entries
        .iter()
        .filter(|e| e.name.to_lowercase().contains(&query))
        .collect();
    rows.sort_by(|a, b| {
        b.is_folder.cmp(&a.is_folder).then_with(|| match sort {
            Sort::AZ => by_name(a, b),
            Sort::ZA => by_name(b, a),
            Sort::Recent => b.recency.cmp(&a.recency).then_with(|| by_name(a, b)),
        })
    });
    rows
}

/// The name field of `Create folder` or `Rename`.
struct Prompt {
    /// What `Rename` renames; `None`: `Create folder`.
    rename: Option<SongEntry>,
    text: String,
    /// Just opened: the field takes the keyboard focus.
    focus: bool,
}

impl Prompt {
    fn new(rename: Option<SongEntry>, text: String) -> Self {
        Self {
            rename,
            text,
            focus: true,
        }
    }

    /// The text-input box's title.
    fn title(&self) -> &'static str {
        match self.rename {
            Some(_) => "Enter name",
            None => "Enter folder name",
        }
    }
}

/// The `Sort` menu, the current order highlighted.
fn sort_menu(current: Sort) -> Vec<PopupItem<Sort>> {
    Sort::ALL
        .into_iter()
        .enumerate()
        .map(|(row, sort)| PopupItem {
            current: sort == current,
            ..PopupItem::new(sort, 0, row, sort.label())
        })
        .collect()
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Popup {
    Sort,
    Options,
    /// `New`'s menu, over [`SongsTab::templates`].
    New,
    /// `Import`'s menu ([`IMPORT_FOLDER`]).
    Import,
}

/// `New`'s menu: `Empty`, the header `Templates`, then the `templates`; an item holds the index of
/// the template it opens (`None`: the empty song).
fn new_menu(templates: &[SongEntry]) -> Vec<PopupItem<Option<usize>>> {
    let head = [
        PopupItem::new(None, 0, 0, "Empty"),
        PopupItem::header(0, 1, "Templates"),
    ];
    let templates = templates.iter().enumerate();
    let templates = templates.map(|(i, t)| PopupItem::new(Some(i), 0, i + 2, t.name.clone()));
    head.into_iter().chain(templates).collect()
}

/// `New`'s templates: the `.songtmpl` files of our factory's `Templates` folder, then the library's
/// ([`SongLibrary::templates`], the Original Content's), each named by its stem, in byte order. A name that both have
/// is ours (the menu lists it once).
pub(super) fn all_templates(factory: &Factory, library: &dyn SongLibrary) -> Vec<SongEntry> {
    let folder = Path::new(content::ROOT).join(content::TEMPLATES);
    let ours = factory.list(Some(&folder)).unwrap_or_default();
    let mut all: Vec<SongEntry> = (ours.into_iter().filter(|e| !e.is_folder))
        .filter_map(|e| {
            let name = e.name.strip_suffix(".songtmpl")?.to_owned();
            Some(SongEntry { name, ..e })
        })
        .collect();
    for theirs in library.templates() {
        if all.iter().all(|t| t.name != theirs.name) {
            all.push(theirs);
        }
    }
    all.sort_by(|a, b| a.name.as_bytes().cmp(b.name.as_bytes()));
    all
}

/// SONGS' preview of an audio file (#106): the sample strip under the list, and what it asks of the preview player.
#[derive(Default)]
pub struct SongsPreview {
    /// The file and its strip, while it shows.
    strip: Option<(PathBuf, SampleStrip)>,
    /// What a strip that went asked for, until taken.
    plays: Vec<Play>,
}

impl SongsPreview {
    /// Shows `path`'s strip, its name only until the app decoded it; one playing stops.
    fn pick(&mut self, path: &Path) {
        self.end();
        let name = path.file_stem().unwrap_or_default().to_string_lossy();
        self.strip = Some((path.to_owned(), SampleStrip::new(name.into_owned(), None)));
    }

    /// `path`'s decode landed (`wave`; `None`: it does not decode): its strip, if it still shows, plays it from the
    /// start.
    pub fn decoded(&mut self, path: &Path, wave: Option<Waveform>) {
        if let Some((shown, strip)) = &mut self.strip
            && shown == path
        {
            *strip = SampleStrip::new(std::mem::take(&mut strip.name), wave);
        }
    }

    /// The strip goes, stopped.
    pub fn end(&mut self) {
        if let Some((_, mut strip)) = self.strip.take() {
            strip.stop();
            self.plays.extend(strip.take_plays());
        }
    }

    /// The strip stops, if it plays (Play started the song).
    pub fn stop(&mut self) {
        if let Some((_, strip)) = &mut self.strip {
            strip.stop();
        }
    }

    /// What to send to the preview player since the last call, in order.
    pub fn take_plays(&mut self) -> Vec<Play> {
        let mut plays = std::mem::take(&mut self.plays);
        if let Some((_, strip)) = &mut self.strip {
            plays.extend(strip.take_plays());
        }
        plays
    }

    /// The decoded sample the strip plays.
    pub fn data(&self) -> Option<std::sync::Arc<honeydew_audio::sample::SampleData>> {
        Some(self.strip.as_ref()?.1.wave.as_ref()?.data.clone())
    }
}

/// The SONGS tab's state.
#[derive(Default)]
pub(super) struct SongsTab {
    /// The folders opened from the root, outermost first.
    folders: Vec<SongEntry>,
    /// Above `My Songs` (#73): the user folder ([`SongLibrary::user_folder`]) the folders are opened from, the top.
    up: Option<PathBuf>,
    /// The open folder's entries, read once; `None` reads them again.
    listing: Option<Vec<SongEntry>>,
    /// The search's songs below the open folder, walked once a query needs them, until the folder is read again.
    found: Option<Vec<Found>>,
    /// The list's order: the caller keeps it as `SongBrowserOrdering` ([`super::MainMenu::show`]).
    pub(super) sort: Sort,
    search: SearchBar,
    /// The frame the page last drew in: a gap means it shows anew.
    drawn: Option<u64>,
    /// The selected rows' paths while in select mode.
    pub(super) selection: Option<Vec<PathBuf>>,
    /// How far the list is scrolled up, in points, and its touch and fling ([`scroll`]).
    scroll: f32,
    kinetic: Scroller,
    /// The open popup and the point it is centred on.
    popup: Option<(Popup, Pos2)>,
    /// The name field, while `Create folder` or `Rename` asks.
    prompt: Option<Prompt>,
    /// The ≡ menu's clipboard, `Delete permanently`'s question and what moved.
    actions: FileActions,
    /// The song templates `New`'s menu offers, read as it opens.
    templates: Vec<SongEntry>,
    /// Where `New`'s menu reads our templates: `None`, [`crate::content::factory`].
    pub(super) factory: Option<&'static Factory>,
    /// The Song options dialog, while it asks for a new empty song.
    new_song: Option<SongOptionsDialog>,
    /// The song last opened from the list: the current row, or the folder holding it.
    current: Option<PathBuf>,
    /// `Save`'s swipe.
    swipe: Swipe,
    /// `Delete` of several items, waiting for its question's answer.
    deleting: Option<Vec<SongEntry>>,
    /// The preview of a tapped audio file (#106).
    pub(super) preview: SongsPreview,
}

/// `Save`'s swipe (a swipe button, `Swipe for quick save`): how far the circle is pulled left, in points, and the
/// press's direction once it left the calm zone (`true`: sideways).
#[derive(Default)]
struct Swipe {
    dx: f32,
    sideways: Option<bool>,
}

impl Swipe {
    /// This frame of a drag on control `id`, `u` points a U. Out of the calm zone the dominant axis decides: sideways,
    /// the circle follows the pointer left, at most [`SWIPE_MAX`]; up or down, nothing. Let go, it springs back (the
    /// panels' ease). Returns whether it was let go pulled [`SWIPE_SAVE`] or more.
    fn update(&mut self, ctx: &Context, id: Id, u: f32) -> bool {
        let (origin, pos, released) = ctx.input(|i| {
            let p = &i.pointer;
            (p.press_origin(), p.latest_pos(), p.primary_released())
        });
        if ctx.dragged_id() == Some(id)
            && let Some((o, p)) = origin.zip(pos)
        {
            let d = p - o;
            if self.sideways.is_none() && gesture::left_calm(o, Some(p), u) {
                self.sideways = Some(d.x.abs() > d.y.abs());
            }
            if self.sideways == Some(true) {
                self.dx = (-d.x).clamp(0.0, u * SWIPE_MAX);
            }
            return false;
        }
        let swiped = self.sideways.take() == Some(true) && self.dx >= u * SWIPE_SAVE;
        self.dx -= self.dx * anim::step(ctx, anim::PANEL_KEEP);
        if self.dx < 0.5 / ctx.pixels_per_point() {
            self.dx = 0.0;
        } else {
            ctx.request_repaint();
        }
        released && swiped
    }
}

impl SongsTab {
    /// The open folder, `None` at the root (`My Songs`).
    pub(super) fn folder(&self) -> Option<&std::path::Path> {
        (self.folders.last().map(|f| f.path.as_path())).or(self.up.as_deref())
    }

    /// Sets the list's order: its menu's pick, or the stored `SongBrowserOrdering`, which `Restore default settings`
    /// may change. A new one drops the search's results, walked in the old order, so they follow it.
    pub(super) fn set_sort(&mut self, sort: Sort) {
        if std::mem::replace(&mut self.sort, sort) != sort {
            self.found = None;
        }
    }

    /// Reads the open folder again on the next frame.
    pub(super) fn refresh(&mut self) {
        self.listing = None;
    }

    /// Paints the tab in `area` and handles its input.
    pub(super) fn show(
        &mut self,
        ui: &mut Ui,
        cx: &UiCtx,
        area: Rect,
        library: &mut dyn SongLibrary,
    ) -> Option<MainMenuAction> {
        let m = cx.m;
        let c = &cx.theme.colors;
        let text = c.text.to_color32();
        let id = Id::new("main_menu_songs");
        let ctx = ui.ctx().clone();
        // Each showing of the page (the menu opening on it, or its tab coming back) resets the search and moves the
        // focus into the list.
        let frame = ctx.cumulative_frame_nr();
        if self.drawn.is_none_or(|f| frame > f + 1) {
            self.search = SearchBar::default();
            focus::enter(&ctx, Id::new(focus::SONGS));
        }
        self.drawn = Some(frame);
        let (browser_id, field) = (id.with("browser"), id.with("search"));
        let modal = self.popup.is_some() || self.prompt.is_some() || self.new_song.is_some();
        if !modal && self.search.start_key(&ctx, browser_id, field) {
            self.searched();
        }
        let mut action = None;
        if self.listing.is_none() {
            self.preview.end();
            let folder = self.folder();
            // An unreadable folder shows as empty.
            self.listing = Some(library.list(folder).unwrap_or_default());
            self.found = None;
        }
        // `Recently Deleted` and `Unzipped songs`: places, never selected; the menu changes inside the bin.
        let places = library.places();
        let is_place = |path: &Path| places.iter().any(|p| p == path);
        let trash = library.trash();
        let in_trash = self
            .folders
            .first()
            .is_some_and(|f| trash.as_ref() == Some(&f.path));

        // The browser: everything left of the big circles' column.
        let browser = area.with_max_x(area.right() - m.u(BIG_COL_W));
        focus::open(ui, browser_id, browser);
        // Left column.
        let x = area.left() + m.u(SIDE_COL_W) / 2.0;
        let side = |i: usize| pos2(x, area.top() + m.u(SIDE_STEP) * (i as f32 + 0.5));
        let toggles = [
            (
                "Back",
                None,
                false,
                !self.folders.is_empty() || self.up.is_none() && library.user_folder().is_some(),
            ),
            ("Search", None, self.search.shown(), true),
            ("Sort", None, false, true),
            ("Select", None, self.selection.is_some(), true),
            ("", Some(IconId::Menu), false, true),
        ];
        for (i, (name, icon, on, enabled)) in toggles.into_iter().enumerate() {
            let style = if on {
                CircleStyle::Light
            } else {
                CircleStyle::Outline
            };
            let at = side(i);
            let circle = Circle {
                icon,
                font: FontSize::Pt6,
                style,
                enabled,
                ..Circle::new(id.with(("side", i)), at, m.u(SIDE_D), name)
            };
            // `leaveFolder` (`Go back`) is Back's step.
            // Not while a field (the search, a rename) has the keyboard.
            let back = i == 0
                && enabled
                && !ui.ctx().wants_keyboard_input()
                && crate::keymap::pressed(ui.ctx(), "leaveFolder");
            if !circle_button(ui, cx, &circle) && !back {
                continue;
            }
            match i {
                0 => {
                    if self.folders.pop().is_none() {
                        self.up = library.user_folder();
                    }
                    self.open_folder_changed();
                }
                1 => {
                    match self.search.shown() {
                        true => self.search.end(),
                        false => self.search.start(),
                    }
                    self.searched();
                }
                2 => self.popup = Some((Popup::Sort, at)),
                3 => self.selection = self.selection.is_none().then(Vec::new),
                _ => self.popup = Some((Popup::Options, at)),
            }
        }

        // Right column. `Save` is a swipe button: a tap asks for the name, a swipe quick-saves ([`Swipe`]).
        let x = area.right() - m.u(BIG_COL_W) / 2.0;
        for (i, (name, act)) in [
            ("New", None),
            ("Save", Some(MainMenuAction::SaveNew)),
            ("Import", Some(MainMenuAction::Import)),
        ]
        .into_iter()
        .enumerate()
        {
            let big = id.with(("big", i));
            let mut at = pos2(x, area.top() + m.u(BIG_STEP) * (i as f32 + 0.5));
            if i == 1 {
                if self.swipe.update(&ctx, big, m.u(1.0)) {
                    action = Some(MainMenuAction::Save);
                }
                let rest = Rect::from_center_size(at, vec2(m.u(BIG_D), m.u(BIG_D)));
                crate::ui::tooltip::tip(ui, rest, "Swipe for quick save", Some("saveProject"));
                at.x -= self.swipe.dx;
            }
            if circle_button(ui, cx, &Circle::new(big, at, m.u(BIG_D), name)) {
                match act {
                    Some(MainMenuAction::Import) if IMPORT_FOLDER => {
                        self.popup = Some((Popup::Import, at))
                    }
                    Some(act) => action = Some(act),
                    None => {
                        let factory = self.factory.unwrap_or_else(content::factory);
                        self.templates = all_templates(factory, library);
                        match self.templates.is_empty() {
                            true => self.new_song = Some(SongOptionsDialog::default()),
                            false => self.popup = Some((Popup::New, at)),
                        }
                    }
                }
            }
        }

        // The search bar, then the breadcrumb and the title under it.
        let mid = Rect::from_x_y_ranges(
            area.left() + m.u(SIDE_COL_W)..=area.right() - m.u(BIG_COL_W),
            area.y_range(),
        );
        let mut y = mid.top();
        if self.search.shown() {
            y += m.u(search_bar::DROP);
            if self.search.show(ui, cx, field, browser) {
                self.scroll_home();
            }
        }
        let mut line = |h: f32| {
            let r = Rect::from_x_y_ranges(mid.x_range(), y..=y + m.u(h));
            y += m.u(h);
            r
        };
        // No gap above the title. The breadcrumb's line is there only when it has a trail
        // (inside a folder), so at the root the title starts at the top.
        let trail = self.breadcrumb(in_trash);
        if !trail.is_empty() {
            // A trail too long for the line (the bin's note is long) is shortened in the middle, as a browser's is.
            truncated(
                ui.painter(),
                line(CRUMB_H),
                Align2::LEFT_CENTER,
                &trail,
                m.font_id(FontSize::Pt8),
                text,
                CRUMB_TAIL_K,
            );
        }
        // So is the title, which would run under the big circles.
        truncated_bold(
            ui.painter(),
            line(TITLE_H),
            Align2::LEFT_CENTER,
            self.title(),
            m.font_id(TITLE_FONT),
            text,
            NAME_TAIL_K,
        );
        // `Enter` in the name field does the action; anything else that takes the focus away drops it.
        let mut commit = None;
        if let Some(prompt) = &mut self.prompt {
            let label = line(CRUMB_H);
            let field = line(SEARCH_H);
            line(FIELD_GAP);
            if self.popup.is_none() && crate::ui::close_pressed(ui.ctx()) {
                commit = Some(false);
            }
            let title = prompt.title();
            let field_id = id.with("prompt");
            if prompt.focus {
                // The cursor at the end of the name, not where the last name field left it.
                let mut state = TextEditState::load(ui.ctx(), field_id).unwrap_or_default();
                let end = CCursor::new(prompt.text.chars().count());
                state.cursor.set_char_range(Some(CCursorRange::one(end)));
                state.store(ui.ctx(), field_id);
            }
            text_label(
                ui,
                cx,
                label,
                title,
                FontSize::Pt8,
                Align2::LEFT_CENTER,
                text,
            );
            let edit = TextEdit::singleline(&mut prompt.text)
                .id(field_id)
                .char_limit(TEXT_LIMIT);
            let mut response = text_field(ui, cx, field, title, edit);
            crate::android_text::edit(&mut response, &mut prompt.text, TEXT_LIMIT);
            if std::mem::take(&mut prompt.focus) {
                response.request_focus();
            }
            if response.lost_focus() {
                commit = commit.or(Some(ui.input(|i| i.key_pressed(Key::Enter))));
            }
        }
        // The list: the open folder's rows, or the search's songs below it, each with its folder's path.
        let query = self.search.query();
        if !query.is_empty() && self.found.is_none() {
            self.found = Some(self.walk(library));
        }
        let listing = self.listing.as_deref().unwrap_or_default();
        let rows: Vec<(&SongEntry, Option<&Found>)> =
            match self.found.as_deref().filter(|_| !query.is_empty()) {
                Some(found) => (matching(found, query).into_iter())
                    .map(|f| (&f.entry, Some(f)))
                    .collect(),
                None => (visible(listing, self.sort, query).into_iter())
                    .map(|e| (e, None))
                    .collect(),
            };
        // Select mode's bar under the list: `N selected`, then the ≡ menu's enabled actions, each placed at its left,
        // line and width, a line holding what fits.
        let bar = (self.selection.as_ref()).map(|s| {
            let items = options_menu(self.menu_state(library, in_trash));
            let enabled: Vec<_> = items.into_iter().filter(|i| !i.disabled).collect();
            (format!("{} selected", s.len()), enabled)
        });
        let (gap, pad, step) = (m.u(BAR_GAP), m.u(BAR_PAD), m.u(BAR_STEP));
        let width = |label: &str| {
            let font = m.font_id(FontSize::Pt8);
            ui.painter()
                .layout_no_wrap(label.to_owned(), font, text)
                .size()
                .x
        };
        let (mut placed, mut lines) = (Vec::new(), 0);
        if let Some((count, items)) = &bar {
            let mut x = mid.left() + width(count) + gap;
            for item in items {
                let w = width(&item.label) + 2.0 * pad;
                if x + w > mid.right() && x > mid.left() {
                    (lines, x) = (lines + 1, mid.left());
                }
                placed.push((x, lines, w));
                x += w + gap;
            }
            lines += 1;
        }
        // The preview's strip at the bottom; the list and the bar above it.
        let strip_h = self.preview.strip.as_ref().map_or(0.0, |_| m.u(STRIP_H));
        let bar_top = (mid.bottom() - strip_h - lines as f32 * step).max(y);
        let list = Rect::from_x_y_ranges(mid.x_range(), y..=bar_top);
        let row_h = m.u(ROW_H);
        // Select mode's checkboxes grow in.
        let checks = (self.selection.as_ref()).map(|_| {
            crate::ui::anim::browser_slide(crate::ui::anim::elapsed(&ctx, id.with("select")))
        });
        // Under the Song options dialog the list keeps still: the wheel is the dialog's.
        let offset = match self.new_song {
            Some(_) => self.scroll,
            None => scroll(
                ui,
                id.with("scroll"),
                m.u(1.0),
                &mut self.kinetic,
                list,
                rows.len() as f32 * row_h,
                &mut self.scroll,
            ),
        };
        let painter = ui.painter_at(list);
        // The names are truncated drawing, which stops before the big circles (not clipped): the list clips its
        // scrolled rows itself.
        let outer_clip = ui.clip_rect();
        ui.set_clip_rect(outer_clip.intersect(list));
        focus::open(ui, Id::new(focus::SONGS), list);
        let (mut clicked, mut context) = (None, None);
        for (i, &(entry, found)) in rows.iter().enumerate() {
            let rect = Rect::from_min_size(
                pos2(list.left(), list.top() - offset + i as f32 * row_h),
                vec2(list.width(), row_h),
            );
            if !rect.intersects(list) {
                continue;
            }
            let mut response = crate::ui::focus::interact(
                ui,
                rect.intersect(list),
                id.with(("row", &entry.path)),
                Sense::click(),
            );
            let long = gesture::long_press(ui, &mut response, m);
            let selected = (self.selection.as_ref()).is_some_and(|s| s.contains(&entry.path));
            let current = self
                .current
                .as_ref()
                .is_some_and(|c| match entry.is_folder {
                    true => c.starts_with(&entry.path),
                    false => *c == entry.path,
                });
            // Centred in the whole row: the clip cuts a half-scrolled one. One row height for songs and folders.
            let look = RowLook {
                name: &entry.name,
                place: found.map(|f| f.place.as_str()),
                font: if entry.is_folder {
                    FOLDER_FONT
                } else {
                    ROW_FONT
                },
                ink: text,
                current,
                pressed: response.is_pointer_button_down_on(),
                hovered: crate::ui::widgets::hovered(ui, &response),
                select: checks
                    .filter(|_| !is_place(&entry.path))
                    .map(|t| (t, selected)),
                bold: entry.is_folder,
                mark: None,
            };
            search_bar::paint_row(&painter, cx, rect, &look);
            // A right click, not egui's long touch (`secondary_clicked`): a touch's long press only selects.
            let right = response.clicked_by(PointerButton::Secondary);
            if (right || long) && !is_place(&entry.path) {
                let at = response.interact_pointer_pos().unwrap_or(rect.center());
                context = Some((entry.path.clone(), (!long).then_some(at)));
            }
            if response.clicked() {
                // A found folder opens through the folders above it.
                let folders = found.map_or_else(|| vec![entry.clone()], Found::folders);
                clicked = Some((entry.clone(), folders, found.is_some()));
            }
        }
        focus::close(&ctx, Id::new(focus::SONGS));
        ui.set_clip_rect(outer_clip);
        // Over the rows: a finger on the thumb drags it (#111).
        let (thumb, content_h) = (id.with("scrollbar"), rows.len() as f32 * row_h);
        scroller::list_scrollbar(ui, thumb, list, m.u(1.0), content_h, &mut self.scroll, text);
        let mut bar_pick = None;
        if let Some((count, items)) = &bar {
            let line = |i: usize| bar_top + (i as f32 + 0.5) * step;
            let first = Rect::from_x_y_ranges(mid.x_range(), bar_top..=bar_top + step);
            text_label(
                ui,
                cx,
                first,
                count,
                FontSize::Pt8,
                Align2::LEFT_CENTER,
                text,
            );
            for (item, &(x, i, w)) in items.iter().zip(&placed) {
                let at = pos2(x, line(i) - m.u(BAR_H) / 2.0);
                let rect = Rect::from_min_size(at, vec2(w, m.u(BAR_H)));
                let b = Button {
                    font: FontSize::Pt8,
                    ..Button::new(id.with(("bar", item.row)), rect, &item.label, false, c)
                };
                if button(ui, cx, &b, true).clicked() {
                    bar_pick = item.id;
                }
            }
        }
        if let Some((_, strip)) = &mut self.preview.strip {
            strip.show(ui, cx, mid, id.with("preview"));
        }
        focus::close(&ctx, browser_id);
        let all: Vec<PathBuf> = rows
            .iter()
            .filter(|(e, _)| !is_place(&e.path))
            .map(|(e, _)| e.path.clone())
            .collect();
        // An audio file the tap selects is previewed; one it unselects stops.
        let tapped = (clicked.as_ref())
            .filter(|(e, ..)| !e.is_folder && is_audio(&e.path))
            .map(|(e, ..)| e.path.clone());
        if let Some((entry, folders, found)) = clicked {
            if let Some(selection) = &mut self.selection
                && !is_place(&entry.path)
            {
                if let Some(at) = selection.iter().position(|p| *p == entry.path) {
                    selection.remove(at);
                } else {
                    selection.push(entry.path);
                }
            } else if entry.is_folder {
                self.folders.extend(folders);
                // `My Songs` from above it is the root again, with its places.
                if self.up.is_some()
                    && self.folders.first().map(|f| f.path.as_path()) == library.root()
                {
                    self.up = None;
                    self.folders.remove(0);
                }
                self.open_folder_changed();
                // A found folder ends the search.
                if found {
                    self.search.end();
                }
            } else if !crate::project_files::is_project(&entry.path) {
                // A render opens no song: a tap selects it, for its actions.
                self.selection = Some(vec![entry.path]);
            } else {
                self.current = Some(entry.path.clone());
                action = Some(MainMenuAction::Open(entry.path));
            }
        }
        if let Some(path) = tapped {
            if (self.selection.as_ref()).is_some_and(|s| s.contains(&path)) {
                self.preview.pick(&path);
                action = Some(MainMenuAction::Preview(path));
            } else if self.preview.strip.as_ref().is_some_and(|(p, _)| *p == path) {
                self.preview.end();
            }
        }
        // A secondary tap or a long press: select mode with the row selected; a secondary tap also opens the ≡ menu at
        // the pointer.
        if let Some((path, at)) = context {
            let selection = self.selection.get_or_insert_default();
            if !selection.contains(&path) {
                selection.push(path);
            }
            if let Some(at) = at {
                self.popup = Some((Popup::Options, at));
            }
        }
        if let Some(opt) = bar_pick {
            action = self.apply(opt, library, all.clone()).or(action);
        }

        // Popups last, over everything.
        if let Some((kind, at)) = self.popup {
            let ctx = ui.ctx().clone();
            let close = match kind {
                Popup::New => {
                    let items = new_menu(&self.templates);
                    let result = popup::show(&ctx, id.with("new"), at, area, &items, cx);
                    match result.selected {
                        Some(Some(i)) => {
                            let path = self.templates[i].path.clone();
                            action = Some(MainMenuAction::NewFromTemplate(path));
                        }
                        Some(None) => self.new_song = Some(SongOptionsDialog::default()),
                        None => {}
                    }
                    result.close
                }
                Popup::Import => {
                    let items = [
                        PopupItem::new(false, 0, 0, "Import file"),
                        PopupItem::new(true, 0, 1, "Import folder"),
                    ];
                    let result = popup::show(&ctx, id.with("import"), at, area, &items, cx);
                    action = match result.selected {
                        Some(true) => Some(MainMenuAction::ImportFolder),
                        Some(false) => Some(MainMenuAction::Import),
                        None => action,
                    };
                    result.close
                }
                Popup::Sort => {
                    let result =
                        popup::show(&ctx, id.with("sort"), at, area, &sort_menu(self.sort), cx);
                    if let Some(sort) = result.selected {
                        self.set_sort(sort);
                    }
                    result.close
                }
                Popup::Options => {
                    let items = options_menu(self.menu_state(library, in_trash));
                    let result = popup::show(&ctx, id.with("options"), at, area, &items, cx);
                    if let Some(opt) = result.selected {
                        action = self.apply(opt, library, all).or(action);
                    }
                    result.close
                }
            };
            if close {
                self.popup = None;
                ctx.request_repaint();
            }
        }
        if let Some(enter) = commit
            && let Some(prompt) = self.prompt.take()
            && enter
            && !prompt.text.trim().is_empty()
        {
            let dir = self.open_dir(library);
            let renaming = prompt.rename.as_ref();
            let done = (self.actions).name(renaming, &prompt.text, dir.as_deref(), library);
            action = self.changed(done).or(action);
        }
        // The Song options dialog, over everything.
        let screen = ui.max_rect();
        if let Some(dialog) = &mut self.new_song {
            match dialog.show(ui, cx, screen) {
                Some(SongOptionsAction::Ok(options)) => {
                    self.new_song = None;
                    action = Some(MainMenuAction::NewSong(options));
                }
                Some(SongOptionsAction::Cancel) => self.new_song = None,
                None => {}
            }
        }
        action
    }

    /// The search bar showed or hid (`startSearch`, a `Search` tap): select mode is left and the list refreshes.
    fn searched(&mut self) {
        self.selection = None;
        self.scroll_home();
    }

    /// Escape in the browser: hides the search bar (leaving select mode, the focus on `Search`), else
    /// stops select mode. Returns whether it did either; if not, the menu closes.
    pub(super) fn escape(&mut self, ctx: &eframe::egui::Context) -> bool {
        if self.search.shown() {
            self.search.end();
            self.scroll_home();
            focus::move_to(ctx, Id::new("main_menu_songs").with(("side", 1)));
        } else if self.selection.is_none() {
            return false;
        }
        self.selection = None;
        true
    }

    /// What the ≡ menu (and select mode's bar) depends on; `in_trash`: the open folder is in the bin.
    fn menu_state(&self, library: &dyn SongLibrary, in_trash: bool) -> MenuState {
        let picked = self.picked();
        MenuState {
            selected: picked.len(),
            archive: matches!(&picked[..], [e] if e.is_archive()),
            // `Convert to FLAC` needs the pickers' loading screen.
            wav: false,
            audio: picked.iter().any(|e| !e.is_folder && is_audio(&e.path)),
            in_trash,
            paste: self.actions.can_paste(),
            export: !cfg!(mobile)
                && (cfg!(not(target_arch = "wasm32")) || picked.iter().all(|e| !e.is_folder)),
            reveal: FILE_MANAGER,
            // Original content (`Demo Songs`): what would change it is dimmed.
            // The user folder's top holds the app's files: nothing is made or pasted there.
            locked_folder: (self.open_dir(library)).is_some_and(|d| library.read_only(&d))
                || self.up.is_some() && self.folders.is_empty(),
            locked_picked: picked.iter().any(|e| library.read_only(&e.path)),
            send: cfg!(mobile).then(|| picked.iter().all(|e| !e.is_folder)),
        }
    }

    /// The selected rows, in the folder's order, or the search's.
    fn picked(&self) -> Vec<SongEntry> {
        let Some(selection) = &self.selection else {
            return Vec::new();
        };
        let found = self
            .found
            .as_deref()
            .filter(|_| !self.search.query().is_empty());
        let entries: Vec<&SongEntry> = match found {
            Some(found) => found.iter().map(|f| &f.entry).collect(),
            None => self.listing.iter().flatten().collect(),
        };
        (entries.into_iter())
            .filter(|e| selection.contains(&e.path))
            .cloned()
            .collect()
    }

    /// The search's songs below the open folder ([`Walk`]): each folder in the page's order, the path from `My Songs`.
    fn walk(&self, library: &dyn SongLibrary) -> Vec<Found> {
        let ordered = |entries: &[SongEntry]| -> Vec<SongEntry> {
            visible(entries, self.sort, "")
                .into_iter()
                .cloned()
                .collect()
        };
        let list = |folder: &Path| ordered(&library.list(Some(folder)).unwrap_or_default());
        let names = self.folders.iter().map(|f| f.name.as_str());
        let place: Vec<&str> = std::iter::once(self.top()).chain(names).collect();
        let mut walk = Walk::skipping(library.trash());
        let rows = ordered(self.listing.as_deref().unwrap_or_default());
        walk.add(&list, rows, &place.join("/"));
        walk.found
    }

    /// The open folder, the library's root at the root.
    fn open_dir(&self, library: &dyn SongLibrary) -> Option<PathBuf> {
        self.folder().or(library.root()).map(Path::to_owned)
    }

    /// Does the ≡ menu's `opt` on the selection; `all` is what `Select all` selects.
    fn apply(
        &mut self,
        opt: Opt,
        library: &mut dyn SongLibrary,
        all: Vec<PathBuf>,
    ) -> Option<MainMenuAction> {
        let dir = self.open_dir(library);
        if opt == Opt::Unzip {
            // The ≡ menu offers it for exactly one archive.
            let picked = self.picked();
            return match &picked[..] {
                [e] if e.is_archive() => Some(MainMenuAction::Unzip(e.path.clone())),
                _ => None,
            };
        }
        let picked = self.picked();
        if opt == Opt::SendToPlaylist {
            let files = picked
                .into_iter()
                .filter(|e| !e.is_folder && is_audio(&e.path));
            return Some(MainMenuAction::SendToPlaylist(
                files.map(|e| e.path).collect(),
            ));
        }
        if opt == Opt::Delete && picked.len() > 1 {
            let question = format!("Move {} items to {RECENTLY_DELETED}?", picked.len());
            self.deleting = Some(picked);
            return Some(MainMenuAction::ConfirmDelete(question));
        }
        match self.actions.apply(opt, picked, dir, library) {
            Step::SelectAll => self.selection = Some(all),
            Step::Nothing => {}
            Step::Ask(rename, text) => self.prompt = Some(Prompt::new(rename, text)),
            Step::Confirm(question) => return Some(MainMenuAction::ConfirmDelete(question)),
            Step::Changed(done) => return self.changed(done),
            // An export may write into the folder; the selection stays.
            Step::Kept(done) => {
                self.listing = None;
                return done.err().map(MainMenuAction::Message);
            }
        }
        None
    }

    /// `Delete permanently`'s question, or that of a `Delete` of several, was answered: `yes` removes what it asked
    /// about.
    pub(super) fn delete_answered(
        &mut self,
        yes: bool,
        library: &mut dyn SongLibrary,
    ) -> Option<MainMenuAction> {
        if let Some(picked) = self.deleting.take() {
            let dir = self.open_dir(library);
            let step = yes.then(|| self.actions.apply(Opt::Delete, picked, dir, library));
            return match step {
                Some(Step::Changed(done)) => self.changed(done),
                _ => None,
            };
        }
        let done = self.actions.answered(yes, library)?;
        self.changed(done)
    }

    /// Opens the user folder's folder that holds `file` (`My Tracks`, for a render) from its top, with `file` current.
    pub(super) fn show_render(&mut self, file: &Path, library: &dyn SongLibrary) {
        let user = library.user_folder();
        let listed = user.as_deref().and_then(|u| library.list(Some(u)).ok());
        let place =
            (listed.into_iter().flatten()).find(|e| e.is_folder && file.starts_with(&e.path));
        self.up = place.as_ref().and(user);
        self.folders = place.into_iter().collect();
        self.current = Some(file.to_owned());
        self.selection = None;
        self.open_folder_changed();
    }

    /// What the file actions moved (`Some` where to) or deleted since the last call, oldest first.
    pub(super) fn take_moves(&mut self) -> Vec<Moved> {
        self.actions.take_moves()
    }

    /// An action ran that may have changed the folder: read it again and select nothing. A failure becomes its
    /// message.
    fn changed(&mut self, done: Result<(), String>) -> Option<MainMenuAction> {
        self.listing = None;
        if let Some(selection) = &mut self.selection {
            selection.clear();
        }
        done.err().map(MainMenuAction::Message)
    }

    /// The list back at its top, a fling stopped.
    fn scroll_home(&mut self) {
        (self.scroll, self.kinetic) = (0.0, Scroller::default());
    }

    /// The open folder changed: read it, from the top.
    fn open_folder_changed(&mut self) {
        self.listing = None;
        self.scroll_home();
        if let Some(selection) = &mut self.selection {
            selection.clear();
        }
    }

    /// The folders above the open one, from the root: empty at the root. In the bin the first is `Recently Deleted
    /// (automatically deleted after 30 days)` in place of the bin's own name and `My Songs`.
    fn breadcrumb(&self, in_trash: bool) -> String {
        let Some((_, above)) = self.folders.split_last() else {
            return String::new();
        };
        let (first, below) = match in_trash {
            true => (bin_crumb(), above.get(1..).unwrap_or_default()),
            false => (self.top().to_owned(), above),
        };
        std::iter::once(first.as_str())
            .chain(below.iter().map(|f| f.name.as_str()))
            .collect::<Vec<_>>()
            .join(" / ")
    }

    /// The open folder's name.
    fn title(&self) -> &str {
        self.folders.last().map_or(self.top(), |f| &f.name)
    }

    /// Where the folders are opened from: `My Songs`, or the user folder above it.
    fn top(&self) -> &'static str {
        match self.up {
            Some(_) => crate::APP_NAME,
            None => SONGS_FOLDER,
        }
    }
}

/// The bin's breadcrumb.
pub(crate) fn bin_crumb() -> String {
    format!("{RECENTLY_DELETED} (automatically deleted after {BIN_DAYS} days)")
}

/// Names without case first, then byte by byte, so that the order is total.
fn by_name(a: &SongEntry, b: &SongEntry) -> Ordering {
    a.name
        .to_lowercase()
        .cmp(&b.name.to_lowercase())
        .then_with(|| a.name.cmp(&b.name))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn entry(name: &str, is_folder: bool, recency: u64) -> SongEntry {
        SongEntry {
            name: name.to_owned(),
            path: PathBuf::from(format!("/songs/{name}")),
            is_folder,
            recency,
        }
    }

    fn sample() -> Vec<SongEntry> {
        vec![
            entry("beta.flm", false, 30),
            entry("Alpha.flm", false, 10),
            entry("Demo songs", true, 5),
            entry("Gamma.flm", false, 20),
            entry("Archive", true, 40),
        ]
    }

    fn names(rows: &[&SongEntry]) -> Vec<String> {
        rows.iter().map(|e| e.name.clone()).collect()
    }

    #[test]
    fn folders_come_first_then_songs_in_the_chosen_order() {
        let entries = sample();
        assert_eq!(
            names(&visible(&entries, Sort::AZ, "")),
            [
                "Archive",
                "Demo songs",
                "Alpha.flm",
                "beta.flm",
                "Gamma.flm"
            ]
        );
        assert_eq!(
            names(&visible(&entries, Sort::ZA, "")),
            [
                "Demo songs",
                "Archive",
                "Gamma.flm",
                "beta.flm",
                "Alpha.flm"
            ]
        );
        assert_eq!(
            names(&visible(&entries, Sort::Recent, "")),
            [
                "Archive",
                "Demo songs",
                "beta.flm",
                "Gamma.flm",
                "Alpha.flm"
            ]
        );
    }

    #[test]
    fn equal_names_without_case_keep_a_fixed_order() {
        let entries = vec![entry("b.flm", false, 0), entry("B.flm", false, 0)];
        assert_eq!(names(&visible(&entries, Sort::AZ, "")), ["B.flm", "b.flm"]);
        assert_eq!(
            names(&visible(&entries, Sort::Recent, "")),
            ["B.flm", "b.flm"]
        );
    }

    #[test]
    fn search_keeps_names_holding_the_query_ignoring_case() {
        let entries = sample();
        assert_eq!(names(&visible(&entries, Sort::AZ, "MA")), ["Gamma.flm"]);
        assert_eq!(
            names(&visible(&entries, Sort::AZ, "a.F")),
            ["Alpha.flm", "beta.flm", "Gamma.flm"]
        );
        assert_eq!(names(&visible(&entries, Sort::AZ, "demo")), ["Demo songs"]);
        assert!(visible(&entries, Sort::AZ, "zzz").is_empty());
    }

    #[test]
    fn the_sort_menu_highlights_the_current_order() {
        let menu = sort_menu(Sort::ZA);
        let cells: Vec<_> = menu
            .iter()
            .map(|i| (i.label.as_ref(), i.row, i.current, i.id))
            .collect();
        assert_eq!(
            cells,
            [
                ("A-Z", 0, false, Some(Sort::AZ)),
                ("Z-A", 1, true, Some(Sort::ZA)),
                ("Recent", 2, false, Some(Sort::Recent)),
            ]
        );
    }

    #[test]
    fn the_breadcrumb_names_the_folders_above_the_open_one() {
        let mut tab = SongsTab::default();
        assert_eq!(
            (tab.breadcrumb(false).as_str(), tab.title()),
            ("", "My Songs")
        );
        tab.folders.push(entry("Demo songs", true, 0));
        assert_eq!(
            (tab.breadcrumb(false).as_str(), tab.title()),
            ("My Songs", "Demo songs")
        );
        tab.folders.push(entry("Old", true, 0));
        assert_eq!(
            (tab.breadcrumb(false).as_str(), tab.title()),
            ("My Songs / Demo songs", "Old")
        );
    }

    /// The bin breadcrumb reads `Recently Deleted (automatically deleted after 30 days)`.
    #[test]
    fn the_bin_breadcrumb_says_how_long_files_are_kept() {
        let note = "Recently Deleted (automatically deleted after 30 days)";
        let mut tab = SongsTab::default();
        tab.folders.push(entry("Recently Deleted", true, 0));
        assert_eq!(
            (tab.breadcrumb(true).as_str(), tab.title()),
            (note, "Recently Deleted")
        );
        // The bin stands in for the root in the trail.
        tab.folders.push(entry("My Songs", true, 0));
        assert_eq!(
            (tab.breadcrumb(true).as_str(), tab.title()),
            (note, "My Songs")
        );
        tab.folders.push(entry("F", true, 0));
        assert_eq!(
            (tab.breadcrumb(true), tab.title()),
            (format!("{note} / My Songs"), "F")
        );
    }
}
