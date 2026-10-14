//! The preset picker as the side browser panel: a module's presets, the rack presets, the drum kits or the chord sets,
//! from a root down through folders.
//!
//! - The root lists `My Files`, the user's folder for the scope, then `Default` (the module's init preset; the drum kit
//!   scope's built-in kit; the chord scope's `Major`) in a module, drum kit or chord scope. A rack has no init preset.
//!   Then, after `Recently Deleted` if it is there, the factory folders' folders and presets
//!   ([`PresetPicker::with_factory`]: `My Files, 808, 909, …`): every folder, then every preset, each kind from our own
//!   bank first ([`crate::content`]), then from imported content the user dropped in, folder by folder. They are
//!   read-only: no file action applies in them.
//! - A folder lists its folders, then the scope's preset files by name (the file stem): `A-Z` in the user's files, in
//!   index order (the bytes of their paths) in a factory folder. In a factory folder a folder sharing a listed program's
//!   or preset's name holds its samples, and is left out; in the user's files only a DW program's is (`read`).
//!
//! **Ordering.** At the root: the fixed rows, then the factory folders' folders, then their presets, each factory
//! folder's in index order. Inside `My Files` (and `Recently Deleted`) the rows are `A-Z`; a factory bank keeps the
//! index order, the bytewise path order.
//!
//! Our bank's rows come before imported content's. Where a root row of ours has the name and kind (folder or
//! preset) of an imported content row, ours gives way: the user's own copy of that bank is the one they asked for, and
//! ours stays loadable from songs and templates. A folder and a preset of the same name both list. The `A-Z` compare
//! is natural ([`natural`]: `Grand 2` before `Grand 10`), not the file browser's. The Original Content folder lists as
//! a factory folder, so its rows keep the bytes' order, our stand-in for an index.
//! - Down the left edge: `Search`, which keeps the rows whose name holds the query. No `Sort` (the folder rule above is
//!   the only ordering; the Modules picker's is its own) and no `Filter`.
//! - The header: at the root its title (the module's name, [`RACK_TITLE`], [`DRUMSET_TITLE`] or [`CHORD_SETS_TITLE`]);
//!   inside a folder the Back arrow and the breadcrumb, the root's title then each folder down to the open one, joined
//!   with " | ". Back, Ctrl+Up (`leaveFolder`) go up a folder; at the root the arrow is hidden.
//! - A pick stays picked: the caller may keep the picker open, turn the audition strip on over its row
//!   ([`PresetPicker::audition_on`]) or show the sample strip ([`PresetPicker::set_sample`]). Every list refresh (a
//!   folder entered or left, a search, select mode) turns the audition off and stops the sample.
//!
//! - [`PresetPicker::show_mut`] adds `Select` and the ≡ options menu below them, with the file browser's actions
//!   (SONGS' [`FileActions`]) on the user's files: inside `My Files` and `Recently Deleted`, dimmed at the root, whose
//!   `Default` is no file. `Delete` puts a preset into `Recently Deleted` at its place below the user folder; the root
//!   lists that copy of `My Files` as `Recently Deleted` once it exists, where `Restore` and `Delete permanently`
//!   apply. `Create folder` and `Rename` ask for the name in a field over the list; `Delete permanently` asks first,
//!   and a failure shows its message, each in a popup over the panel, which stays open.
//!
//! The caller keeps a [`PresetPicker`] while it is open and loads what [`PresetPicker::show`] returns. A folder is
//! read when it opens, not every frame.
//!
//! **The Instruments picker** ([`PresetPicker::instruments`]) is this picker over the whole user folder and the
//! instrument categories ([`categories`]):
//! - the root lists `My Files`, then the categories; a category lists its presets in index order, unsorted;
//! - `My Files` lists every folder, and the instrument files, `.instr` and audio files ([`categories::EXTS`]), a
//!   `.flmpst` only when its `HEAD` names an instrument other than the Slicer;
//! - every row 1.5 U tall; folders and presets alike in FontSize 4 (18 pt), as in every side browser;
//! - the left column is `Search`, with `Select` and ≡ only inside `My Files`;
//! - the search lists every folder and preset below the open folder, `My Files`' too at the root (a walk, made once a
//!   search needs it), the rows 2.2 U tall with their folder's path from the root under the name, in FontSize 1 at
//!   α 0.6. A found folder opens through the folders above it, and the search ends ([`crate::ui::search_bar`]). An
//!   empty query is the open folder's normal listing, even with the search bar shown. The path's folders are joined
//!   with `/`.

use std::cell::OnceCell;
use std::cmp::Ordering;
use std::io;
use std::iter::Peekable;
use std::path::{Path, PathBuf};
use std::str::Chars;

use eframe::egui::{Area, Context, Id, LayerId, Order, Pos2, Sense};

use crate::content;
use crate::project_files::content::{FOLD_CASE, inside};
use crate::project_files::{DWP_EXT, PRESET_EXT, SAMPLE_EXTS, fits};
use crate::ui::UiCtx;
use crate::ui::main_menu::library::RECENTLY_DELETED;
use crate::ui::main_menu::{
    FILE_MANAGER, FileActions, MenuState, Opt, SongEntry, SongLibrary, Step, bin_crumb,
    options_menu,
};
use crate::ui::modules_browser::{
    Chrome, PanelClick, PanelRow, ROW_H, RowKind, SideButton, SidePanel, crumb_text, layout,
};
use crate::ui::picker_preview::{Play, SampleStrip, Sound};
use crate::ui::popup::{self, PopupItem, PopupResult};
use crate::ui::search_bar::{FOUND_ROW_H, Found, Walk, matching};

/// The root row that opens the user's folder for the scope.
pub const MY_FILES: &str = "My Files";
/// The root row for a module scope's init preset (the module's defaults), the drum kit scope's built-in kit or the chord
/// scope's built-in set.
pub const DEFAULT: &str = "Default";
/// The rack scope's root title (the factory folder's name).
pub const RACK_TITLE: &str = "Rack Presets";
/// The drum kit scope's root title.
pub const DRUMSET_TITLE: &str = "Drumsets";
/// The chord set scope's root title.
pub const CHORD_SETS_TITLE: &str = "Chord Sets";

/// The left buttons: `Search`, then, with the file actions, `Select` and ≡, then `Import` if offered.
const SELECT: usize = 1;
const OPTIONS: usize = 2;
/// The button that copies a file in from outside ([`PresetPicker::with_import`]).
pub const IMPORT: &str = "Import";

/// What the user did in the picker this frame. A `Close` is the end of it: drop it. After a pick the caller keeps it
/// open or drops it, as its picker's rule says.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum PickerAction {
    /// `Import` ([`PresetPicker::with_import`]): the caller brings a file in; the picker stays open.
    Import,
    /// The close X, a click outside the panel, or Escape.
    Close,
    /// `Default`: the module's init preset, its defaults; in the drum kit scope, the built-in kit; in the chord scope, the
    /// built-in set.
    Default,
    /// A preset file, by the path its library listed it under: read it with [`SongLibrary::read`].
    Pick(PathBuf),
}

/// The Instruments picker's title.
pub const INSTRUMENTS_TITLE: &str = "Instruments";
/// The Instruments picker's rows ([`PresetPicker::instruments`]), in U.
const INSTRUMENT_ROW_H: f32 = 1.5;

/// A second root beside `My Files` ([`PresetPicker::with_root`]).
#[derive(Clone, Debug, PartialEq, Eq)]
struct Extra {
    name: &'static str,
    /// The folder it opens; `None`: there is none, and the row is dimmed.
    path: Option<PathBuf>,
    /// The files it lists, in place of the scope's.
    exts: Vec<&'static str>,
}

/// What a row is.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Item<'a> {
    /// `My Files`, opening this folder; `None`: there is no user folder, and it is dimmed.
    MyFiles(Option<&'a Path>),
    /// The second root ([`PresetPicker::with_root`]).
    Extra(&'a Extra),
    Default,
    /// `Recently Deleted`: the bin's copy of `My Files`, at this path.
    Bin(&'a Path),
    /// A folder or a preset of the open folder.
    Entry(&'a SongEntry),
    /// A folder or a preset the search found below the open folder.
    Found(&'a Found),
}

impl<'a> Item<'a> {
    fn name(self) -> &'a str {
        match self {
            Self::MyFiles(_) => MY_FILES,
            Self::Extra(extra) => extra.name,
            Self::Default => DEFAULT,
            Self::Bin(_) => RECENTLY_DELETED,
            Self::Entry(entry) => &entry.name,
            Self::Found(found) => &found.entry.name,
        }
    }

    /// A folder row: it opens.
    fn is_folder(self) -> bool {
        match self {
            Self::MyFiles(_) | Self::Extra(_) | Self::Bin(_) => true,
            Self::Entry(e) => e.is_folder,
            Self::Found(f) => f.entry.is_folder,
            Self::Default => false,
        }
    }
}

/// What a click asks of the picker.
enum Go {
    Done(PickerAction),
    Back,
    Open(SongEntry),
    /// A found folder: these folders, outermost first, then the search ends.
    Into(Vec<SongEntry>),
    /// Select mode: the row's selection toggles.
    Toggle(PathBuf),
    /// `Select` toggles select mode.
    Select,
    /// The ≡ menu.
    Options,
    /// Enter in the name field.
    Name,
}

/// A popup over the panel.
enum Popup {
    /// The ≡ menu, centred on its button.
    Options(Pos2),
    /// A failure's message, with `OK`.
    Message(String),
    /// `Delete permanently`'s question, with `Yes` / `No`.
    Confirm(String),
}

/// The open picker's state. Make a fresh one each time it opens.
pub struct PresetPicker {
    /// The root's title.
    title: String,
    my_files: Option<PathBuf>,
    /// The scope's file types, any of them matched without case.
    exts: Vec<&'static str>,
    /// The root lists `Default`.
    default_row: bool,
    /// Only the presets that load into this module type are listed ([`Self::of_type`]).
    module_type: Option<u16>,
    /// A second root, listing other files ([`Self::with_root`]).
    extra: Option<Extra>,
    /// The factory folders whose folders and presets the root lists after `Default`.
    factory: Vec<PathBuf>,
    /// The root's factory rows, one listing per factory folder, read once; `None` reads them again.
    factory_listing: Option<Vec<Vec<SongEntry>>>,
    /// The folders opened from the root, `My Files` (or `Recently Deleted`) first; empty at the root.
    folders: Vec<SongEntry>,
    /// The open folder's rows, read once; `None` reads them again (at the root: whether the bin is there).
    listing: Option<Vec<SongEntry>>,
    panel: SidePanel,
    /// The bin's copy of `My Files` while it exists, which the root lists (only with the file actions).
    bin: Option<PathBuf>,
    /// The library's bin ([`SongLibrary::trash`]), read with the listing: a folder in it is in the bin, however it
    /// was reached (the Instruments picker's `My Files` › `Recently Deleted`).
    trash: Option<PathBuf>,
    /// The selected rows' paths while in select mode.
    selection: Option<Vec<PathBuf>>,
    /// The ≡ menu's clipboard, question and moves, as SONGS has them. It lives while the picker is open.
    actions: FileActions,
    /// The left edge offers `Import`.
    import: bool,
    /// What `Rename` renames while the name field asks; `None`: `Create folder`.
    renaming: Option<SongEntry>,
    popup: Option<Popup>,
    /// The last pick, by its path (`None` in it: `Default`): the audition strip's row.
    picked: Option<Option<PathBuf>>,
    /// The plays [`Self::set_sample`] made, until they are taken.
    held_plays: Vec<Play>,
    /// The Instruments picker's categories ([`Self::instruments`]); `None`: another picker.
    categories: Option<Vec<categories::Category>>,
    /// The search rows below the open folder, walked once a search needs them, until the folder is read again.
    found: OnceCell<Vec<Found>>,
    /// The web: a pick of our bundled content waiting for its download ([`content::web::picked`]).
    download: Option<(PathBuf, content::web::Ticket)>,
}

impl PresetPicker {
    /// A picker for one scope, at its root.
    /// - `title`: the module's display name, [`RACK_TITLE`], [`DRUMSET_TITLE`] or [`CHORD_SETS_TITLE`].
    /// - `my_files`: the user's folder for the scope, which need not exist yet (it lists as empty); `None` dims
    ///   `My Files`.
    /// - `ext`: the scope's preset files, [`crate::project_files::PRESET_EXT`], [`crate::project_files::RACK_EXT`],
    ///   [`crate::project_files::DRUMSET_EXT`] or [`crate::project_files::CHORDS_EXT`], matched without case.
    /// - `default_row`: the root lists `Default` (a module, drum kit or chord set scope).
    pub fn new(
        title: impl Into<String>,
        my_files: Option<PathBuf>,
        ext: &'static str,
        default_row: bool,
    ) -> Self {
        Self {
            title: title.into(),
            my_files,
            exts: vec![ext],
            default_row,
            module_type: None,
            extra: None,
            factory: Vec::new(),
            factory_listing: None,
            folders: Vec::new(),
            listing: None,
            panel: SidePanel::default(),
            bin: None,
            trash: None,
            selection: None,
            actions: FileActions::default(),
            import: false,
            renaming: None,
            popup: None,
            picked: None,
            held_plays: Vec::new(),
            categories: None,
            found: OnceCell::new(),
            download: None,
        }
    }

    /// The Instruments picker (see the module docs): `My Files` is the whole user folder, then `categories`. Give it
    /// [`INSTRUMENTS_TITLE`], [`categories::EXTS`] and no `Default`.
    pub fn instruments(self, categories: Vec<categories::Category>) -> Self {
        Self {
            categories: Some(categories),
            ..self
        }
    }

    /// Lists the files of any of `exts` in place of `new`'s one type (the sample picker's audio files; MiniSynth's and
    /// GMSynth's legacy presets, [`crate::project_files::preset_exts`]).
    pub fn of_exts(self, exts: &[&'static str]) -> Self {
        Self {
            exts: exts.to_vec(),
            ..self
        }
    }

    /// Lists only the presets whose `HEAD` names module type `type_id` (or that load into it: a legacy Reverb into a
    /// Reverb v2, [`crate::flm::preset_fits`]). A folder's presets are read when it opens.
    pub fn of_type(self, type_id: u16) -> Self {
        Self {
            module_type: Some(type_id),
            ..self
        }
    }

    /// Adds a root `name` beside `My Files`, opening `path` (`None` dims it) and listing the files of `exts` from there
    /// down, whatever they hold: the SoundFont Player's `My SoundFonts` (`.sf2` files are its presets). It is
    /// read-only: the file actions are not offered in it.
    pub fn with_root(
        self,
        name: &'static str,
        path: Option<PathBuf>,
        exts: &[&'static str],
    ) -> Self {
        let extra = Extra {
            name,
            path,
            exts: exts.to_vec(),
        };
        Self {
            extra: Some(extra),
            ..self
        }
    }

    /// Lists the folders and presets of each of `folders` (content folders, read-only) at the root, after `Default`.
    pub fn with_factory(self, folders: impl IntoIterator<Item = PathBuf>) -> Self {
        Self {
            factory: folders.into_iter().collect(),
            ..self
        }
    }

    /// Offers `Import` down the left edge, after the other buttons, when `import` ([`PickerAction::Import`]).
    pub fn with_import(self, import: bool) -> Self {
        Self {
            import: import && !cfg!(target_os = "android"),
            ..self
        }
    }

    /// Opens the folders from the root down to the file `path`'s, as taps on them would, and makes it the pick, its row
    /// scrolled into view as the list first shows (a pad's sample). It stops at the deepest folder listed on the way.
    /// Folders and the row match without case where the file system ignores it ([`FOLD_CASE`]).
    pub fn reveal(&mut self, library: &dyn SongLibrary, path: &Path) {
        self.read_factory(library);
        loop {
            // The deepest folder row holding it: a factory folder lies inside the user folder.
            let rows = self.rows("").into_iter().filter_map(|item| match item {
                Item::MyFiles(Some(dir)) => Some(folder(MY_FILES, dir)),
                Item::Entry(e) if e.is_folder => Some(e.clone()),
                _ => None,
            });
            let next = rows.filter(|f| inside(path, &f.path, FOLD_CASE));
            let Some(next) = next.max_by_key(|f| f.path.components().count()) else {
                break;
            };
            self.listing = Some(self.read_open(library, &next.path));
            self.folders.push(next);
        }
        self.picked = Some(Some(path.to_owned()));
        self.panel.reveal = true;
    }

    /// Reads the open folder again: a file came into it from outside the picker (`Import`).
    pub fn reload(&mut self) {
        self.listing = None;
    }

    /// The open folder is the second root, or one inside it.
    fn in_extra(&self) -> bool {
        let root = self.folders.first();
        let extra = self.extra.as_ref().and_then(|e| e.path.as_ref());
        extra.is_some() && root.map(|r| &r.path) == extra
    }

    /// The module type it lists the presets of; `None` for a rack scope.
    pub fn module_type(&self) -> Option<u16> {
        self.module_type
    }

    /// Turns the audition strip on over the picked row (a pick loaded into an instrument), until the next refresh.
    pub fn audition_on(&mut self) {
        self.panel.audition.on = true;
        // Each pick moves the focus to the strip.
        self.panel.auditioned = false;
    }

    /// Releases the audition strip's held note (take it after).
    pub fn release(&mut self) {
        self.panel.audition.release();
    }

    /// Whether the audition strip is on.
    pub fn auditioning(&self) -> bool {
        self.panel.audition.on
    }

    /// The pick is loading on a worker: its row pulses `LOADING...`, with or without the audition strip. It does while
    /// its download runs too.
    pub fn set_loading(&mut self, loading: bool) {
        self.panel.audition.loading = loading || self.download.is_some();
    }

    /// The audition strip's notes since the last call, for the picked module's channel.
    pub fn take_sounds(&mut self) -> Vec<Sound> {
        self.panel.audition.take_sounds()
    }

    /// Shows `strip` for the last pick; any sample playing stops.
    pub fn set_sample(&mut self, strip: SampleStrip) {
        let mut plays = self.take_plays();
        if let Some(old) = &mut self.panel.sample {
            old.stop();
            plays.extend(old.take_plays());
        }
        let mut strip = strip;
        plays.extend(strip.take_plays());
        self.panel.sample = Some(strip);
        self.held_plays = plays;
    }

    /// The name the sample strip shows, if it shows one.
    #[cfg(test)]
    pub(crate) fn sample_name(&self) -> Option<&str> {
        self.panel.sample.as_ref().map(|s| s.name.as_str())
    }

    /// The sample strip's decoded sample, if it shows one.
    pub fn sample_data(&self) -> Option<std::sync::Arc<honeydew_audio::sample::SampleData>> {
        let wave = self.panel.sample.as_ref()?.wave.as_ref()?;
        Some(wave.data.clone())
    }

    /// What the sample strip asks of the preview player since the last call.
    pub fn take_plays(&mut self) -> Vec<Play> {
        let mut plays = std::mem::take(&mut self.held_plays);
        if let Some(strip) = &mut self.panel.sample {
            plays.extend(strip.take_plays());
        }
        plays
    }

    /// What the ≡ file actions moved (`Some` where to) or deleted since the last call, oldest first, for the open
    /// song's file to follow, as SONGS' moves do ([`crate::project_files::ProjectFiles::path_moved`]).
    pub fn take_moves(&mut self) -> Vec<(PathBuf, Option<PathBuf>)> {
        self.actions.take_moves()
    }

    /// The sample strip stops, if it plays (take its plays after).
    pub fn stop_sample(&mut self) {
        if let Some(strip) = &mut self.panel.sample {
            strip.stop();
        }
    }

    /// The picker closes: the held note is released and the sample stops (take both after).
    pub fn closing(&mut self) {
        self.release();
        self.stop_sample();
    }

    /// Shows the picker over everything else, listing from `library`; call it every frame while it is open. It is
    /// modal and takes Escape as the Modules browser does (`SidePanel::show`). It offers no file action.
    pub fn show(
        &mut self,
        ctx: &Context,
        cx: &UiCtx,
        library: &dyn SongLibrary,
    ) -> Option<PickerAction> {
        let action = self.run(ctx, cx, &mut ReadOnly(library), false);
        self.downloaded(action)
    }

    /// [`Self::show`], with `Select` and the ≡ file actions on the user's files, which change `library`.
    pub fn show_mut(
        &mut self,
        ctx: &Context,
        cx: &UiCtx,
        library: &mut dyn SongLibrary,
    ) -> Option<PickerAction> {
        let action = self.run(ctx, cx, library, true);
        self.downloaded(action)
    }

    /// The web: a pick of our bundled content not in memory yet waits for its download, its row pulsing
    /// `LOADING...`, and comes out once that is done. Any other action, and the picker's close, drop it and stop the
    /// download (a close does not wait for a download, which can take tens of seconds).
    fn downloaded(&mut self, action: Option<PickerAction>) -> Option<PickerAction> {
        let pick = match action {
            Some(PickerAction::Pick(path)) => Some(path),
            Some(other) => {
                self.download = None;
                return Some(other);
            }
            None => None,
        };
        let picked = content::web::picked(&mut self.download, pick);
        self.panel.audition.loading |= self.download.is_some();
        picked.map(PickerAction::Pick)
    }

    fn run(
        &mut self,
        ctx: &Context,
        cx: &UiCtx,
        library: &mut dyn SongLibrary,
        actions: bool,
    ) -> Option<PickerAction> {
        if self.listing.is_none() {
            self.found = OnceCell::new();
            self.trash = library.trash();
            self.listing = Some(match self.folders.last() {
                Some(folder) => self.read_open(library, &folder.path),
                None => {
                    let bin = self.my_files.as_deref().and_then(|f| library.bin_path(f));
                    // The Instruments picker's `My Files` is the whole user folder, the bin one of its folders.
                    let instruments = self.categories.is_some();
                    self.bin = bin.filter(|bin| actions && !instruments && library.exists(bin));
                    Vec::new()
                }
            });
        }
        let id = Id::new("preset_picker");
        // Before the panel, which would take Escape.
        if let Some(popup) = &self.popup {
            self.popup_shown(popup_items(popup, self.menu_state()), ctx, cx, id, library);
        }
        if self.folders.is_empty() && self.factory_listing.is_none() {
            self.read_factory(library);
        }
        // Out of `self` while it shows, as the rows borrow `self`.
        let mut panel = std::mem::take(&mut self.panel);
        let header = self.header();
        let back = !self.folders.is_empty();
        let mut buttons = vec![SideButton::Search];
        // The user's presets only: a factory folder and the second root are read-only.
        let mine = back && !self.in_factory() && !self.in_extra();
        let instruments = self.categories.is_some();
        // The Instruments picker shows them inside `My Files` only.
        if actions && (mine || !instruments) {
            let selecting = mine && self.selection.is_some();
            buttons.extend([
                SideButton::Toggle("Select", selecting, mine),
                SideButton::Menu(mine),
            ]);
        }
        let import = Some(buttons.len()).filter(|_| self.import);
        if import.is_some() {
            buttons.push(SideButton::Plain(IMPORT, true));
        }
        let chrome = Chrome {
            id,
            title: &header,
            back,
            buttons: &buttons,
            popup: self.popup.is_some(),
        };
        let lib = &*library;
        let click = panel.show(ctx, cx, &chrome, |query| {
            // An empty query is the normal folder listing, even with the bar shown; any other lists what is below
            // the open folder.
            let searching = !query.is_empty();
            let rows = match searching {
                true => self.search_rows(query, lib),
                false => self.rows(query),
            };
            rows.into_iter()
                .map(|item| PanelRow {
                    name: item.name(),
                    folder: item.is_folder(),
                    h: match instruments {
                        _ if searching => FOUND_ROW_H,
                        true => INSTRUMENT_ROW_H,
                        false => ROW_H,
                    },
                    sub: match item {
                        Item::Found(found) => Some(&found.place),
                        _ => None,
                    },
                    kind: match item {
                        Item::MyFiles(None) => RowKind::Disabled,
                        Item::Extra(Extra { path: None, .. }) => RowKind::Disabled,
                        Item::Entry(e) if self.selected(&e.path) => RowKind::Selected(item),
                        Item::Found(f) if self.selected(&f.entry.path) => RowKind::Selected(item),
                        _ => RowKind::Enabled(item),
                    },
                    picked: self.is_picked(item),
                })
                .collect()
        });
        let selecting = mine && self.selection.is_some();
        let go = click.map(|click| match click {
            PanelClick::Close => Go::Done(PickerAction::Close),
            PanelClick::Button(i) if Some(i) == import => Go::Done(PickerAction::Import),
            PanelClick::Button(SELECT) => Go::Select,
            PanelClick::Button(_) => Go::Options,
            PanelClick::Back => Go::Back,
            PanelClick::Enter => Go::Name,
            PanelClick::Row(Item::MyFiles(path)) => {
                Go::Open(folder(MY_FILES, path.unwrap_or(Path::new(""))))
            }
            PanelClick::Row(Item::Extra(extra)) => {
                let path = extra.path.as_deref().unwrap_or(Path::new(""));
                Go::Open(folder(extra.name, path))
            }
            PanelClick::Row(Item::Bin(path)) => Go::Open(folder(RECENTLY_DELETED, path)),
            PanelClick::Row(Item::Default) => Go::Done(PickerAction::Default),
            PanelClick::Row(Item::Entry(e)) if selecting => Go::Toggle(e.path.clone()),
            PanelClick::Row(Item::Entry(e)) if e.is_folder => Go::Open(e.clone()),
            PanelClick::Row(Item::Entry(e)) => Go::Done(PickerAction::Pick(e.path.clone())),
            PanelClick::Row(Item::Found(f)) if selecting => Go::Toggle(f.entry.path.clone()),
            PanelClick::Row(Item::Found(f)) if f.entry.is_folder => Go::Into(f.folders()),
            PanelClick::Row(Item::Found(f)) => Go::Done(PickerAction::Pick(f.entry.path.clone())),
        });
        self.panel = panel;
        match go? {
            Go::Done(action) => {
                match &action {
                    PickerAction::Pick(path) => self.picked = Some(Some(path.clone())),
                    PickerAction::Default => self.picked = Some(None),
                    _ => {}
                }
                return Some(action);
            }
            Go::Back => drop(self.folders.pop()),
            Go::Open(folder) => self.folders.push(folder),
            Go::Into(folders) => {
                self.folders.extend(folders);
                self.panel.end_search();
            }
            Go::Toggle(path) => {
                let selection = self.selection.get_or_insert_default();
                match selection.iter().position(|p| *p == path) {
                    Some(at) => drop(selection.remove(at)),
                    None => selection.push(path),
                }
                return None;
            }
            Go::Select => {
                self.selection = self.selection.is_none().then(Vec::new);
                self.panel.refreshed();
                return None;
            }
            Go::Options => {
                let at = layout(ctx.content_rect(), 1.0, cx.m).button(OPTIONS);
                self.popup = Some(Popup::Options(at));
                return None;
            }
            Go::Name => {
                let renaming = self.renaming.take();
                if let Some(text) = self.panel.take_prompt()
                    && !text.trim().is_empty()
                {
                    let dir = self.folders.last().map(|f| f.path.clone());
                    let done = self
                        .actions
                        .name(renaming.as_ref(), &text, dir.as_deref(), library);
                    self.changed(done);
                }
                return None;
            }
        }
        // The open folder changed: read it, from the top, with nothing selected.
        self.listing = None;
        self.panel.scroll_y = 0.0;
        self.panel.refreshed();
        if let Some(selection) = &mut self.selection {
            selection.clear();
        }
        None
    }

    /// Shows the open popup, made of `items`, and does what was picked in it.
    fn popup_shown(
        &mut self,
        items: Items,
        ctx: &Context,
        cx: &UiCtx,
        id: Id,
        library: &mut dyn SongLibrary,
    ) {
        let center = ctx.content_rect().center();
        match items {
            Items::Options(at, items) => {
                let result = modal(ctx, cx, id.with("options menu"), at, &items);
                if result.close {
                    self.popup = None;
                }
                if let Some(opt) = result.selected {
                    self.apply(opt, library);
                }
            }
            Items::Answer(items) => {
                let result = modal(ctx, cx, id.with("message"), center, &items);
                if result.close {
                    self.popup = None;
                    // `OK`, or the question's answer: Escape or a click outside is no.
                    if let Some(done) = self
                        .actions
                        .answered(result.selected == Some(true), library)
                    {
                        self.changed(done);
                    }
                }
            }
        }
    }

    /// Does the ≡ menu's `opt` on the selection.
    fn apply(&mut self, opt: Opt, library: &mut dyn SongLibrary) {
        let dir = self.folders.last().map(|f| f.path.clone());
        match self.actions.apply(opt, self.picked(), dir, library) {
            Step::SelectAll => {
                let paths = self.shown().into_iter().map(|e| e.path.clone());
                self.selection = Some(paths.collect());
            }
            Step::Nothing => {}
            Step::Ask(renaming, text) => {
                let title = match renaming {
                    Some(_) => "Enter name",
                    None => "Enter folder name",
                };
                self.renaming = renaming;
                self.panel.ask(title, text);
            }
            Step::Confirm(question) => self.popup = Some(Popup::Confirm(question)),
            Step::Changed(done) => self.changed(done),
            // An export may write into the folder; the selection stays.
            Step::Kept(done) => {
                self.listing = None;
                self.failed(done);
            }
        }
    }

    /// An action ran that may have changed the folder: read it again and select nothing; a failure shows its message.
    fn changed(&mut self, done: Result<(), String>) {
        self.listing = None;
        if let Some(selection) = &mut self.selection {
            selection.clear();
        }
        self.failed(done);
    }

    fn failed(&mut self, done: Result<(), String>) {
        if let Err(message) = done {
            self.popup = Some(Popup::Message(message));
        }
    }

    fn selected(&self, path: &Path) -> bool {
        self.selection
            .as_ref()
            .is_some_and(|s| s.iter().any(|p| p == path))
    }

    /// The files and folders the list shows: the open folder's for the query, or the search's results.
    fn shown(&self) -> Vec<&SongEntry> {
        let query = self.panel.query();
        let found = self.found.get().map(|found| matching(found, query));
        match found.filter(|_| !query.is_empty()) {
            Some(found) => found.into_iter().map(|f| &f.entry).collect(),
            None => (self.rows(query).into_iter())
                .filter_map(|item| match item {
                    Item::Entry(e) => Some(e),
                    _ => None,
                })
                .collect(),
        }
    }

    /// The selected rows in the list's order, by their file names (the rows show a preset's stem).
    fn picked(&self) -> Vec<SongEntry> {
        let shown = self.shown();
        let picked = shown.into_iter().filter(|e| self.selected(&e.path));
        let named = |e: &SongEntry| SongEntry {
            name: e
                .path
                .file_name()
                .map_or_else(|| e.name.clone(), |n| n.to_string_lossy().into_owned()),
            ..e.clone()
        };
        picked.map(named).collect()
    }

    /// The open folder is the library's bin or in it: the bin's copy of `My Files` from the root's row, or the
    /// Instruments picker's `My Files` › `Recently Deleted`. The ≡ menu then offers `Restore` and `Delete permanently`
    /// (which asks), never `Delete`, which would remove a binned file for good.
    fn in_trash(&self) -> bool {
        let open = self.folders.last().map(|f| &f.path);
        open.zip(self.trash.as_ref())
            .is_some_and(|(open, trash)| open.starts_with(trash))
    }

    /// The open folder was reached through the root's bin row (the header's trail then starts with the bin's note).
    fn opened_from_bin_row(&self) -> bool {
        self.bin.is_some() && self.folders.first().map(|f| &f.path) == self.bin.as_ref()
    }

    /// What the ≡ menu depends on.
    fn menu_state(&self) -> MenuState {
        let picked = self.picked();
        MenuState {
            selected: picked.len(),
            in_trash: self.in_trash(),
            paste: self.actions.can_paste(),
            // The web downloads each file: a folder cannot be.
            export: !cfg!(target_os = "android")
                && (cfg!(not(target_arch = "wasm32")) || picked.iter().all(|e| !e.is_folder)),
            reveal: FILE_MANAGER,
            ..MenuState::default()
        }
    }

    /// The open folder is a factory folder, or one inside it, or a category: read-only.
    fn in_factory(&self) -> bool {
        let first = self.folders.first().map(|f| &f.path);
        first.is_some_and(|f| self.factory.iter().any(|d| f.starts_with(d))) || self.in_category()
    }

    /// The open folder is one of the Instruments picker's categories.
    fn in_category(&self) -> bool {
        (self.folders.first()).is_some_and(|f| self.category(&f.path).is_some())
    }

    /// The category whose folder is `path`.
    fn category(&self, path: &Path) -> Option<&categories::Category> {
        let categories = self.categories.as_deref().unwrap_or_default();
        categories.iter().find(|c| c.folder.path == path)
    }

    /// The rows of `folder`, wherever it is below the root: a category's presets, or the folder's listing ([`read`]),
    /// in the second root's file types inside it, a factory folder's samples left out; in the Instruments picker's `My
    /// Files` a `.flmpst` only when it is an instrument's ([`categories::preset_file`]).
    fn read_open(&self, library: &dyn SongLibrary, folder: &Path) -> Vec<SongEntry> {
        if let Some(category) = self.category(folder) {
            return category.items.clone();
        }
        let extra = self.extra.as_ref().filter(|e| {
            let path = e.path.as_deref();
            path.is_some_and(|p| folder.starts_with(p))
        });
        let (exts, module_type) = match extra {
            Some(extra) => (&extra.exts[..], None),
            None => (&self.exts[..], self.module_type),
        };
        let factory = self.factory.iter().any(|d| folder.starts_with(d));
        let mut rows = read(library, folder, exts, module_type, factory);
        if self.categories.is_some() {
            let instrument = |e: &SongEntry| {
                let preset =
                    (e.path.extension()).is_some_and(|x| x.eq_ignore_ascii_case(PRESET_EXT));
                !preset
                    || library
                        .read(&e.path)
                        .is_ok_and(|b| categories::lists_preset(&b))
            };
            rows.retain(|e| e.is_folder || instrument(e));
        }
        rows
    }

    /// The search's folders and files below the open folder ([`search_bar::Walk`]), walked once: in a folder its rows;
    /// at the root those of `My Files`, the second root, the factory folders and the categories, as the root lists
    /// them. Each folder lists in the picker's order ([`Self::ordered`]).
    fn found(&self, library: &dyn SongLibrary) -> &[Found] {
        self.found.get_or_init(|| {
            let list = |folder: &Path| self.ordered(folder, self.read_open(library, folder));
            let mut walk = Walk::skipping(library.trash());
            match self.folders.last() {
                Some(open) => {
                    let place: Vec<&str> = self.folders.iter().map(|f| f.name.as_str()).collect();
                    walk.add(&list, list(&open.path), &place.join("/"));
                }
                None => {
                    // `Default` is a row of its own; the bin's presets are deleted, no results.
                    let rows = self.rows("").into_iter().filter_map(|item| match item {
                        Item::MyFiles(Some(path)) => Some(folder(MY_FILES, path)),
                        Item::Extra(Extra {
                            name,
                            path: Some(path),
                            ..
                        }) => Some(folder(name, path)),
                        Item::Entry(e) => Some(e.clone()),
                        _ => None,
                    });
                    walk.add(&list, rows.collect(), "");
                }
            }
            walk.found
        })
    }

    /// `rows` of `folder` in the order the picker lists them: a category's as they are, a factory folder's by index
    /// ([`indexed`]), the user's folders first, then files, each `A-Z`.
    fn ordered(&self, folder: &Path, mut rows: Vec<SongEntry>) -> Vec<SongEntry> {
        if self.category(folder).is_some() {
            return rows;
        }
        match self.factory.iter().any(|d| folder.starts_with(d)) {
            true => rows
                .sort_by(|a, b| (b.is_folder.cmp(&a.is_folder)).then_with(|| a.path.cmp(&b.path))),
            false => rows.sort_by(|a, b| {
                (b.is_folder.cmp(&a.is_folder)).then_with(|| natural(&a.name, &b.name))
            }),
        }
        rows
    }

    /// The search rows for `query`: at the root `Default` if its name holds it, then the found folders and presets that
    /// do.
    fn search_rows<'a>(&'a self, query: &str, library: &dyn SongLibrary) -> Vec<Item<'a>> {
        let default = self.folders.is_empty() && self.default_row;
        let default = default && DEFAULT.to_lowercase().contains(&query.to_lowercase());
        let default = default.then_some(Item::Default);
        default
            .into_iter()
            .chain(self.found_rows(query, self.found(library)))
            .collect()
    }

    /// The search rows of `found` whose name holds `query`, ignoring case.
    fn found_rows<'a>(&self, query: &str, found: &'a [Found]) -> Vec<Item<'a>> {
        matching(found, query)
            .into_iter()
            .map(Item::Found)
            .collect()
    }

    /// Reads the root's factory rows: the folders and presets of every factory folder. A row of our own bank gives way
    /// to an imported content row of the same name and kind.
    fn read_factory(&mut self, library: &dyn SongLibrary) {
        let read = |folder: &PathBuf| read(library, folder, &self.exts, self.module_type, true);
        let mut listings: Vec<Vec<SongEntry>> = self.factory.iter().map(read).collect();
        let theirs: Vec<(bool, String)> = (listings.iter().flatten())
            .filter(|e| !content::is_factory(&e.path))
            .map(|e| (e.is_folder, e.name.clone()))
            .collect();
        let clash = |e: &SongEntry| (theirs.iter()).any(|(f, n)| *f == e.is_folder && *n == e.name);
        for listing in &mut listings {
            listing.retain(|e| !content::is_factory(&e.path) || !clash(e));
        }
        self.factory_listing = Some(listings);
    }

    /// The rows for `query`: at the root `My Files`, then `Default` in a module scope, then `Recently Deleted` once it
    /// holds the scope's presets, then the factory folders' folders, then their presets: each kind from every factory
    /// folder in turn, ours first, each folder's in index order (the DW Sampler's `EXP` categories after the others);
    /// in a folder its folders, then its presets, each `A-Z` in the user's files and in index order in a factory
    /// folder. Only those whose name holds `query`, ignoring case.
    fn rows(&self, query: &str) -> Vec<Item<'_>> {
        if self.folders.is_empty() {
            let lower = query.to_lowercase();
            let my_files = Item::MyFiles(self.my_files.as_deref());
            let root = std::iter::once(my_files)
                .chain(self.extra.as_ref().map(Item::Extra))
                .chain(self.default_row.then_some(Item::Default))
                .chain(self.bin.as_deref().map(Item::Bin));
            let root = root.filter(|item| item.name().to_lowercase().contains(&lower));
            let factory = self.factory_listing.as_deref().unwrap_or_default();
            let mut factory: Vec<_> = factory.iter().flat_map(|f| indexed(f, query)).collect();
            // At the root: every folder before every preset; the sort is stable, so each kind keeps the folders' order.
            factory.sort_by_key(|e| !e.is_folder);
            let categories = self.categories.as_deref().unwrap_or_default();
            let categories = (categories.iter().map(|c| &c.folder))
                .filter(|e| e.name.to_lowercase().contains(&lower));
            let rows = root.chain(factory.into_iter().chain(categories).map(Item::Entry));
            return rows.collect();
        }
        let listing = self.listing.as_deref().unwrap_or_default();
        // A category keeps its index order.
        if self.in_category() {
            let lower = query.to_lowercase();
            let rows = listing
                .iter()
                .filter(|e| e.name.to_lowercase().contains(&lower));
            return rows.map(Item::Entry).collect();
        }
        let mut rows = indexed(listing, query);
        if !self.in_factory() {
            rows.sort_by(|a, b| {
                (b.is_folder.cmp(&a.is_folder)).then_with(|| natural(&a.name, &b.name))
            });
        }
        rows.into_iter().map(Item::Entry).collect()
    }

    /// The rows' names, for `query`, once the folders `path` are opened from the root one by one, as clicks would.
    #[cfg(test)]
    pub(crate) fn names_in(&mut self, library: &dyn SongLibrary, path: &[&str]) -> Vec<String> {
        self.read_factory(library);
        for name in path {
            let rows = self.rows("");
            let folder = match rows.into_iter().find(|i| i.name() == *name) {
                Some(Item::MyFiles(Some(path))) => SongEntry {
                    name: MY_FILES.into(),
                    path: path.to_owned(),
                    is_folder: true,
                    recency: 0,
                },
                Some(Item::Entry(e)) if e.is_folder => e.clone(),
                other => panic!("{name}: {other:?}"),
            };
            self.folders.push(folder);
            let path = self.folders.last().unwrap().path.clone();
            self.listing = Some(self.read_open(library, &path));
        }
        self.rows("")
            .into_iter()
            .map(|i| i.name().to_owned())
            .collect()
    }

    /// The root's title.
    #[cfg(test)]
    pub(crate) fn root_title(&self) -> &str {
        &self.title
    }

    /// The path of the row named `name`, a file the picker would pick.
    #[cfg(test)]
    pub(crate) fn path_of(&self, name: &str) -> Option<PathBuf> {
        match self.rows("").into_iter().find(|i| i.name() == name)? {
            Item::Entry(e) if !e.is_folder => Some(e.path.clone()),
            _ => None,
        }
    }

    /// The header's text: the root's title at the root; in a folder the breadcrumb ([`crumb_text`]) from the root's
    /// title down to the open folder. The bin's crumb is its note (`Recently Deleted (automatically deleted after 30
    /// days)`), as SONGS': first in the trail from the root's bin row, or in its place below the Instruments picker's
    /// `My Files`.
    fn header(&self) -> String {
        if self.folders.is_empty() {
            return self.title.clone();
        }
        let (first, folders) = match self.opened_from_bin_row() {
            true => (bin_crumb(), &self.folders[1..]),
            false => (self.title.clone(), &self.folders[..]),
        };
        // The bin opened as a folder of `My Files` (the Instruments picker's) reads as the bin's note too.
        let names: Vec<String> = (folders.iter())
            .map(|f| match Some(&f.path) == self.trash.as_ref() {
                true => bin_crumb(),
                false => f.name.clone(),
            })
            .collect();
        crumb_text(std::iter::once(first.as_str()).chain(names.iter().map(String::as_str)))
    }

    /// `item` is the last pick.
    fn is_picked(&self, item: Item) -> bool {
        match (item, &self.picked) {
            (Item::Default, Some(None)) => true,
            (Item::Entry(e), Some(Some(path))) => !e.is_folder && same_file(&e.path, path),
            (Item::Found(f), Some(Some(path))) => same_file(&f.entry.path, path),
            _ => false,
        }
    }
}

/// A factory folder's rows for `query` (a factory bank keeps the index order): those whose name holds `query`, ignoring
/// case; folders first, then files, each by the bytes of their paths.
fn indexed<'a>(entries: &'a [SongEntry], query: &str) -> Vec<&'a SongEntry> {
    let query = query.to_lowercase();
    let mut rows: Vec<_> = (entries.iter())
        .filter(|e| e.name.to_lowercase().contains(&query))
        .collect();
    rows.sort_by(|a, b| (b.is_folder.cmp(&a.is_folder)).then_with(|| a.path.cmp(&b.path)));
    rows
}

/// The `A-Z` compare of the user's files, natural alphanumeric: a run of digits by its value, anything else without
/// case, so `Grand 2` comes before `Grand 10`; names that tie so (`Grand 02`, `Grand 2`) compare by their bytes.
pub(crate) fn natural(a: &str, b: &str) -> Ordering {
    let (mut x, mut y) = (a.chars().peekable(), b.chars().peekable());
    loop {
        let order = match (x.peek().copied(), y.peek().copied()) {
            (None, None) => return a.cmp(b),
            (None, Some(_)) => return Ordering::Less,
            (Some(_), None) => return Ordering::Greater,
            (Some(c), Some(d)) if c.is_ascii_digit() && d.is_ascii_digit() => {
                let (n, m) = (number(&mut x), number(&mut y));
                n.len().cmp(&m.len()).then_with(|| n.cmp(&m))
            }
            (Some(c), Some(d)) => {
                x.next();
                y.next();
                c.to_lowercase().cmp(d.to_lowercase())
            }
        };
        if order.is_ne() {
            return order;
        }
    }
}

/// The digits at the start of `chars`, taken off it, without their leading zeros.
fn number(chars: &mut Peekable<Chars>) -> String {
    let digits: String = std::iter::from_fn(|| chars.next_if(char::is_ascii_digit)).collect();
    digits.trim_start_matches('0').to_owned()
}

/// `a` and `b` name the same file: component by component, without case where the file system ignores it
/// ([`FOLD_CASE`]), as a revealed pad sample's path may be written ([`PresetPicker::reveal`]).
fn same_file(a: &Path, b: &Path) -> bool {
    inside(a, b, FOLD_CASE) && inside(b, a, FOLD_CASE)
}

fn folder(name: &str, path: &Path) -> SongEntry {
    SongEntry {
        name: name.into(),
        path: path.to_owned(),
        is_folder: true,
        recency: 0,
    }
}

/// A popup's items: the ≡ menu at its button, or a message's lines and choices (`true` is `OK` or `Yes`).
enum Items {
    Options(Pos2, Vec<PopupItem<Opt>>),
    Answer(Vec<PopupItem<bool>>),
}

/// The items of `popup`, the ≡ menu's from `state`. A message is its line, then `OK`; the question its line, then
/// `Yes` / `No` (as the app's messages).
fn popup_items(popup: &Popup, state: MenuState) -> Items {
    let (line, choices): (&str, &[(bool, &'static str)]) = match popup {
        Popup::Options(at) => return Items::Options(*at, options_menu(state)),
        Popup::Message(line) => (line, &[(true, "OK")]),
        Popup::Confirm(line) => (line, &[(true, "Yes"), (false, "No")]),
    };
    let header = PopupItem::header(0, 0, line.to_owned());
    let choices = (1..)
        .zip(choices)
        .map(|(row, &(yes, label))| PopupItem::new(yes, 0, row, label));
    Items::Answer(std::iter::once(header).chain(choices).collect())
}

/// `items` in a popup centred on `at`, modal over the panel too: the popup's own backdrop lies under the panel
/// (`Order::Middle`), so a blocker above the panel takes the clicks outside the popup, and closes it.
pub(crate) fn modal<T: Copy>(
    ctx: &Context,
    cx: &UiCtx,
    id: Id,
    at: Pos2,
    items: &[PopupItem<T>],
) -> PopupResult<T> {
    let screen = ctx.content_rect();
    let blocker = Area::new(id.with("blocker"))
        .order(Order::Foreground)
        .fixed_pos(screen.min)
        .constrain(false)
        .fade_in(false)
        .sense(Sense::click_and_drag())
        .show(ctx, |ui| {
            ui.allocate_exact_size(screen.size(), Sense::hover())
        })
        .response;
    let mut result = popup::show(ctx, id, at, screen, items, cx);
    result.close |= blocker.clicked() || blocker.secondary_clicked() || blocker.middle_clicked();
    // Both above the panel, however a press raised it; the blocker, made first, stays under the popup.
    ctx.move_to_top(blocker.layer_id);
    ctx.move_to_top(LayerId::new(Order::Foreground, id));
    result
}

/// A library seen through [`PresetPicker::show`]: it lists and reads, and changes nothing.
struct ReadOnly<'a>(&'a dyn SongLibrary);

impl SongLibrary for ReadOnly<'_> {
    fn list(&self, folder: Option<&Path>) -> io::Result<Vec<SongEntry>> {
        self.0.list(folder)
    }

    fn read(&self, path: &Path) -> io::Result<Vec<u8>> {
        self.0.read(path)
    }

    fn store(&mut self, _: &str, _: &[u8]) -> io::Result<()> {
        Err(io::ErrorKind::Unsupported.into())
    }

    /// The search leaves the bin out ([`crate::ui::search_bar::Walk`]).
    fn trash(&self) -> Option<PathBuf> {
        self.0.trash()
    }
}

/// `folder`'s folders and files of any of `exts`, each file named by its stem; with `module_type`, only the presets that
/// load into that module type ([`crate::project_files::fits`]). A missing or unreadable folder is empty, and so is an
/// unreadable preset.
///
/// A folder sharing a program's stem, without case, is that program's sample folder and is left out: a DW program's
/// zones are in the folder next to the `.dwp` that has the `.dwp`'s base name, and the Essential Pianos presets keep
/// theirs so too.
/// - `hide_samples` (a read-only factory or content folder): a program is any listed file but an audio file
///   ([`SAMPLE_EXTS`]); a picker that lists no programs there (the sample picker) lists the folder.
/// - Otherwise (the user's files): only a `.dwp`, listed or not, as before (now without case too). A folder the user
///   makes beside their preset of its name is theirs, and lists.
fn read(
    library: &dyn SongLibrary,
    folder: &Path,
    exts: &[&str],
    module_type: Option<u16>,
    hide_samples: bool,
) -> Vec<SongEntry> {
    let entries = library.list(Some(folder)).unwrap_or_default();
    // The `.dwp` files' stems, listed or not, before `keep` names the files by their stems.
    let dwps: Vec<String> = (entries.iter())
        .map(|e| Path::new(&e.name))
        .filter(|name| (name.extension()).is_some_and(|x| x.eq_ignore_ascii_case(DWP_EXT)))
        .filter_map(|name| Some(name.file_stem()?.to_string_lossy().to_lowercase()))
        .collect();
    let keep = |mut entry: SongEntry| {
        if !entry.is_folder {
            let name = Path::new(&entry.name);
            if !name
                .extension()
                .is_some_and(|e| exts.iter().any(|x| e.eq_ignore_ascii_case(x)))
            {
                return None;
            }
            // ponytail: reads each `.flmpst` whole for its HEAD, once per folder opened; a header-only read on the
            // library if user folders grow large.
            if !fits(name, module_type, || library.read(&entry.path).ok()) {
                return None;
            }
            entry.name = name.file_stem()?.to_string_lossy().into_owned();
        }
        Some(entry)
    };
    let listed: Vec<SongEntry> = entries.into_iter().filter_map(keep).collect();
    let audio = |e: &SongEntry| {
        let ext = e.path.extension();
        ext.is_some_and(|x| SAMPLE_EXTS.iter().any(|a| x.eq_ignore_ascii_case(a)))
    };
    let programs: Vec<String> = match hide_samples {
        true => (listed.iter())
            .filter(|e| !e.is_folder && !audio(e))
            .map(|e| e.name.to_lowercase())
            .collect(),
        false => dwps,
    };
    let samples = |e: &SongEntry| e.is_folder && programs.contains(&e.name.to_lowercase());
    listed.into_iter().filter(|e| !samples(e)).collect()
}

pub mod categories;

#[cfg(test)]
mod tests;
