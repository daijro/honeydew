//! The Modules side browser: the modules a rack can add, by section, with search. The rack opens it from `Add Module`
//! and from a card's `Insert above` / `Insert below` / `Replace`; the caller keeps a [`ModulesBrowser`] while it is
//! open and applies a pick with [`add_module`], a `Paste` of the clipboard's modules with [`crate::ui::rack::paste`].
//! `Rack Preset` hands over to the rack preset picker.
//!
//! The SYNTH section (`3x Osc` to `Transistor Bass`) is a section like the others, with no rule that sets its rows
//! apart: a pick adds an instrument module with the registry defaults and a fresh id, from `Add Module` and from
//! `Insert above` / `Insert below` / `Replace`, like any module. But `Paste` puts instrument modules only into a
//! channel whose primary lane is a notes lane, and the picker follows the same rule: on any other channel (drums,
//! audio, effect track, master) the rows are dimmed and inert. The caller says which with `show`'s `instruments`, from
//! [`crate::ui::rack::takes_instruments`], the one predicate `Paste` and a rack preset's load use too.
//!
//! `Paste` is always listed and enabled: the clipboard holds the modules a card's `Copy` or the rack's `Copy Rack` put
//! there, and what does not fit the channel is the caller's `Cannot paste module`.
//!
//! `Sort` orders the list ([`ModuleOrdering`]): the picker's own "Category" is the sections table; its menu offers
//! `A-Z` and `Category` (never `Top Level`), the current one highlighted. Each opening is in Category unless the stored
//! `ModuleBrowserOrdering` names an ordering ([`crate::settings::Settings::module_ordering`], set into
//! [`ModulesBrowser::sort`]); the caller stores a pick there at once. "Top Level"'s item types are the actions, the
//! instruments and the effects (note effects included), its per-type index the module type id, and it has no section
//! rows; A-Z leaves out the grouping flags and so sorts the actions in with the modules.
//!
//! The panel itself, `SidePanel`, is shared with the preset picker ([`crate::ui::preset_picker`]).

use eframe::egui::text::{CCursor, CCursorRange};
use eframe::egui::text_edit::TextEditState;
use eframe::egui::{
    Align, Align2, Area, Context, Id, Key, LayerId, Order, Pos2, Rect, Sense, TextEdit,
    emath::TSTransform, pos2, vec2,
};

use crate::icons::{IconId, draw_icon};
use crate::keymap;
use crate::metrics::{FontSize, Metrics};
use crate::params::{self, Role};
use crate::song::{ModuleInstance, Rack, SynthKind};
use crate::theme::DISABLED_ALPHA;
use crate::ui::buttons::{filled_circle_button, outline_circle_button};
use crate::ui::picker_preview::{Audition, STRIP_H, SampleStrip};
use crate::ui::popup::PopupItem;
use crate::ui::preset_picker::{modal, natural};
use crate::ui::search_bar::{self, SearchBar};
use crate::ui::widgets::{shrink, truncated};
use crate::ui::{Finger, UiCtx, anim, fingers, focus, focus_lost};

use Entry::Module as M;
use SynthKind as S;

/// An entry of the list: the two actions, an instrument or an effect.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Entry {
    Paste,
    RackPreset,
    /// An instrument module; addable only into a channel that takes instruments.
    Synth(SynthKind),
    /// An effect or note effect, by type id.
    Module(u16),
    /// An installed third-party plugin (desktop): an instrument is addable only into a channel that takes instruments.
    Plugin(PluginEntry),
}

/// An installed plugin as the list shows it ([`crate::plugins::available`]).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct PluginEntry {
    /// `format:id`.
    pub uid: &'static str,
    /// Its name and format, `Name (VST3)`.
    pub name: &'static str,
    pub instrument: bool,
}

impl Entry {
    pub fn name(self) -> &'static str {
        match self {
            Self::Paste => "Paste",
            Self::RackPreset => "Rack Preset",
            Self::Synth(kind) => kind.label(),
            Self::Module(id) => params::module_spec(id).map_or("", |s| s.display_name),
            Self::Plugin(plugin) => plugin.name,
        }
    }
}

/// The sections of the installed plugins, after [`SECTIONS`]: instruments, then effects.
fn plugin_sections() -> [(&'static str, Vec<Entry>); 2] {
    use crate::song::plugin::intern;
    let (mut instruments, mut effects) = (Vec::new(), Vec::new());
    for plugin in crate::plugins::available() {
        let instrument = plugin.kind == crate::plugins::PluginKind::Instrument;
        let entry = Entry::Plugin(PluginEntry {
            uid: intern(&plugin.uid),
            name: intern(&format!("{} ({})", plugin.name, plugin.format)),
            instrument,
        });
        if instrument {
            &mut instruments
        } else {
            &mut effects
        }
        .push(entry);
    }
    [
        ("PLUGIN INSTRUMENTS", instruments),
        ("PLUGIN EFFECTS", effects),
    ]
}

/// A row of the list: a section header (upper case, on a darker band) or an entry.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Row {
    Section(&'static str),
    Item(Entry),
}

impl Row {
    /// The section label or the entry name.
    pub fn name(self) -> &'static str {
        match self {
            Self::Section(label) => label,
            Self::Item(entry) => entry.name(),
        }
    }
}

/// The Modules sections, in order. The first group has no header. Ids by display name; where a name is shared
/// (Reverb, Limiter, Autoduck) the current, non-legacy id.
const SECTIONS: [(Option<&str>, &[Entry]); 7] = [
    (None, &[Entry::Paste, Entry::RackPreset]),
    (
        Some("SYNTH"),
        &[
            Entry::Synth(S::ThreeXOsc),
            Entry::Synth(S::DwSampler),
            Entry::Synth(S::GmSynth),
            Entry::Synth(S::MiniSynth),
            Entry::Synth(S::Slicer),
            Entry::Synth(S::SoundFontPlayer),
            Entry::Synth(S::SuperSaw),
            Entry::Synth(S::TransistorBass),
        ],
    ),
    (Some("ECHO"), &[M(32), M(35), M(18), M(16)]),
    (
        Some("LEVEL"),
        &[M(29), M(8), M(6), M(27), M(20), M(28), M(36), M(41)],
    ),
    (
        Some("FILTER"),
        &[M(40), M(3), M(9), M(7), M(34), M(44), M(15), M(30)],
    ),
    (
        Some("MISC"),
        &[
            M(26),
            M(11),
            M(13),
            M(4),
            M(14),
            M(25),
            M(45),
            M(42),
            M(46),
            M(24),
            M(43),
        ],
    ),
    (Some("NOTE EFFECT"), &[M(51), M(47), M(49), M(50), M(48)]),
];

/// The Modules picker's orderings, in creation order, each with its `ModuleBrowserOrdering` id.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum ModuleOrdering {
    /// Every entry by its name, natural alphanumeric ([`natural`]).
    AZ = 1,
    /// By item type, then the per-type index ([`Entry::top_level`]), else alphanumeric. Never in this picker's menu.
    TopLevel = 2,
    /// The sections, each under its header row: the [`SECTIONS`] table. The picker's own.
    #[default]
    Category = 3,
}

impl ModuleOrdering {
    const ALL: [Self; 3] = [Self::AZ, Self::TopLevel, Self::Category];
    /// What `Sort`'s menu offers.
    const MENU: [Self; 2] = [Self::AZ, Self::Category];

    pub fn label(self) -> &'static str {
        match self {
            Self::AZ => "A-Z",
            Self::TopLevel => "Top Level",
            Self::Category => "Category",
        }
    }

    /// Its stored `ModuleBrowserOrdering`.
    pub fn id(self) -> i32 {
        self as i32
    }

    /// The ordering stored as `id`, if any.
    pub fn from_id(id: i32) -> Option<Self> {
        Self::ALL.into_iter().find(|o| o.id() == id)
    }
}

impl Entry {
    /// "Top Level"'s item type and per-type index: the actions, which have none, then the instruments, then the
    /// effects and note effects, each by module type id.
    fn top_level(self) -> (u8, u16) {
        match self {
            Self::Paste | Self::RackPreset => (0, 0),
            Self::Synth(kind) => (1, kind.type_id()),
            Self::Module(id) => (2, id),
            Self::Plugin(plugin) => (if plugin.instrument { 1 } else { 2 }, u16::MAX),
        }
    }
}

/// The list for `query` in `sort`: the entries whose name contains it, ignoring case. In `Category` each section
/// header is kept while one of its entries is; the other orderings have no section rows.
pub fn rows(query: &str, sort: ModuleOrdering) -> Vec<Row> {
    let query = query.to_lowercase();
    let hit = |e: &&Entry| e.name().to_lowercase().contains(&query);
    let plugins = plugin_sections();
    if sort != ModuleOrdering::Category {
        let entries = SECTIONS.iter().flat_map(|(_, entries)| entries.iter());
        let entries = entries.chain(plugins.iter().flat_map(|(_, entries)| entries));
        let mut entries: Vec<Entry> = entries.filter(hit).copied().collect();
        let key = |e: &Entry| match sort {
            ModuleOrdering::TopLevel => e.top_level(),
            _ => (0, 0),
        };
        entries.sort_by(|a, b| (key(a).cmp(&key(b))).then_with(|| natural(a.name(), b.name())));
        return entries.into_iter().map(Row::Item).collect();
    }
    let mut rows = Vec::new();
    let plugins = plugins
        .iter()
        .map(|(header, entries)| (Some(*header), &entries[..]));
    for (header, entries) in SECTIONS.into_iter().chain(plugins) {
        let mut hits = entries.iter().filter(hit).map(|&e| Row::Item(e)).peekable();
        if let (Some(header), Some(_)) = (header, hits.peek()) {
            rows.push(Row::Section(header));
        }
        rows.extend(hits);
    }
    rows
}

/// Where a picked module goes.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum InsertAt {
    /// `Add Module`: the end of the chain. A note effect goes after the note effects at the top instead, as note
    /// effects sit before the instrument.
    End,
    /// `Insert above` the module with this instance id; [`InsertAt::End`] if it is gone.
    Above(i32),
    /// `Insert below` the module with this instance id; [`InsertAt::End`] if it is gone.
    Below(i32),
    /// `Replace` the module with this instance id: it goes, and the pick takes its place. [`InsertAt::End`] if it is
    /// gone.
    Replace(i32),
}

/// Adds module `type_id` at `at`, with its registry defaults and a fresh instance id, which it returns.
pub fn add_module(rack: &mut Rack, type_id: u16, at: InsertAt) -> i32 {
    let next_to = match at {
        InsertAt::End => None,
        InsertAt::Above(id) => rack.index_of(id),
        InsertAt::Below(id) => rack.index_of(id).map(|i| i + 1),
        InsertAt::Replace(id) => {
            let index = rack.index_of(id);
            if let Some(i) = index {
                rack.modules.remove(i);
            }
            index
        }
    };
    let note = |id| params::module_spec(id).is_some_and(|s| s.role == Role::NoteEffect);
    let index = next_to.unwrap_or_else(|| {
        if note(type_id) {
            rack.modules.iter().take_while(|m| note(m.type_id)).count()
        } else {
            rack.modules.len()
        }
    });
    rack.insert(index, type_id)
}

/// Adds a module hosting plugin `uid` at `at`, where [`add_module`] would add a module, started on the channel with id
/// `channel_id` ([`crate::plugins::add`]); its instance id, or why the plugin did not start (the rack unchanged).
pub fn add_plugin(
    rack: &mut Rack,
    channel_id: i32,
    uid: &str,
    at: InsertAt,
) -> Result<i32, String> {
    let end = rack.modules.len();
    let (index, replaced) = match at {
        InsertAt::End => (end, None),
        InsertAt::Above(id) => (rack.index_of(id).unwrap_or(end), None),
        InsertAt::Below(id) => (rack.index_of(id).map_or(end, |i| i + 1), None),
        InsertAt::Replace(id) => (rack.index_of(id).unwrap_or(end), Some(id)),
    };
    let added = crate::plugins::add(rack, channel_id, uid, index)?;
    if let Some(id) = replaced {
        rack.remove(id);
    }
    Ok(added)
}

/// Adds a copy of `module` (a clipboard or rack preset module) at `at`, where [`add_module`] would add its type, with a
/// fresh instance id, which it returns. Everything else is copied: params, common params, `CSTM`, the card's minimized
/// state.
pub fn paste_module(rack: &mut Rack, module: &ModuleInstance, at: InsertAt) -> i32 {
    let instance_id = add_module(rack, module.type_id, at);
    let pasted = rack.get_mut(instance_id).expect("just added");
    *pasted = ModuleInstance {
        instance_id,
        ..module.clone()
    };
    instance_id
}

// Sizes in U. The width is 18 U, at most the screen's.
const WIDTH: f32 = 18.0;
/// The header: 2 U, a 1 px `text` line under it, the X 1.5 U wide 0.25 U from the right edge, the Back
/// arrow (icon 43) 1.5 U wide at the left.
const HEADER_H: f32 = 2.0;
const CLOSE_W: f32 = 1.5;
const CLOSE_RIGHT: f32 = 0.25;
const BACK_W: f32 = 1.5;
/// The name field's title line.
const CRUMB_H: f32 = 0.7;
/// The left column of round buttons, and the list right of it.
const COLUMN_W: f32 = 2.0;
const BUTTON_D: f32 = 1.5;
/// Between the header and the first button.
const BUTTON_GAP: f32 = 0.5;
/// Between one button and the next; the gap under the header stays [`BUTTON_GAP`].
const BUTTON_APART: f32 = 0.25;
/// The buttons' labels: 8 pt, shrunk to fit the button.
const BUTTON_LABEL_PT: u8 = 8;
/// A row's height (the Instruments picker's are taller, `PanelRow::h`).
pub(crate) const ROW_H: f32 = 1.25;
/// Left padding of the title, the row texts and the search text.
const PAD_X: f32 = 0.5;
const SCROLLBAR_W: f32 = 0.15;
/// Icon size factor of the close X (as the transport's), and of the ≡ button's icon (as SONGS').
const CLOSE_ICON: f32 = 0.7;
const MENU_ICON: f32 = 0.6;
/// The longest name the name field takes, in characters (as SONGS').
const NAME_LIMIT: usize = 255;
/// The tail a too-long title keeps after its middle ellipsis.
const TAIL_K: usize = 5;
/// The tail a too-long breadcrumb keeps at most, as SONGS' (the nearest folders, and the bin's `30 days`, stay
/// readable): its ellipsis then settles near the middle.
const CRUMB_TAIL_K: usize = 40;
/// The Back arrow's icon size factor.
const BACK_ICON: f32 = 0.7;
/// `text` alphas: the pressed row's band, the scrollbar thumb (disabled rows: [`DISABLED_ALPHA`]).
const PRESSED_ALPHA: f32 = 0.15;
const SCROLLBAR_ALPHA: f32 = 0.5;
/// Every row's text: folders, items, section bands and found rows alike, one size in every side browser (the folder
/// rows' FontSize 4). A long name keeps its middle ellipsis ([`TAIL_K`]).
const ROW_FONT: FontSize = FontSize::Pt18;
/// Row flash on pick: a tapped item row is lit with `text` at this alpha, fading out linearly over [`FLASH_S`]
/// seconds.
const FLASH_ALPHA: f32 = 0.6;
const FLASH_S: f64 = 0.3;

/// What the user did in the browser this frame. Either way the browser is done: drop it.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum BrowserAction {
    /// The close X, a click outside the panel, or Escape.
    Close,
    /// An instrument, effect or note effect, by type id; add it with [`add_module`].
    Pick(u16),
    /// An installed plugin, by `format:id`; add it with [`add_plugin`].
    Plugin(&'static str),
    /// `Paste`: add the clipboard's modules with [`crate::ui::rack::paste`].
    Paste,
    /// `Rack Preset`: open the rack preset picker in the browser's place, for the same insert point.
    RackPreset,
}

impl Entry {
    /// What a click on the entry does; `None`: it is dimmed and inert. `instruments`: the channel takes instruments.
    fn action(self, instruments: bool) -> Option<BrowserAction> {
        match self {
            Self::Paste => Some(BrowserAction::Paste),
            Self::RackPreset => Some(BrowserAction::RackPreset),
            Self::Synth(kind) => instruments.then_some(BrowserAction::Pick(kind.type_id())),
            Self::Module(id) => Some(BrowserAction::Pick(id)),
            Self::Plugin(plugin) => {
                (instruments || !plugin.instrument).then_some(BrowserAction::Plugin(plugin.uid))
            }
        }
    }
}

/// The open browser's state. Make a fresh one each time it opens.
#[derive(Default)]
pub struct ModulesBrowser {
    panel: SidePanel,
    /// The list's ordering: set it from `ModuleBrowserOrdering` as the browser opens.
    pub sort: ModuleOrdering,
    /// A pick from `Sort`'s menu, the active ordering included: take it and store it as `ModuleBrowserOrdering`.
    pub sort_picked: Option<ModuleOrdering>,
    /// `Sort`'s menu, open, centred on its button.
    sort_menu: Option<Pos2>,
}

impl ModulesBrowser {
    /// Shows the panel over everything else; call it every frame while it is open. `instruments`: the channel takes
    /// instruments, so the SYNTH rows are enabled (else dimmed). See `SidePanel::show` for how it is modal and takes
    /// Escape.
    pub fn show(&mut self, ctx: &Context, cx: &UiCtx, instruments: bool) -> Option<BrowserAction> {
        let id = Id::new("modules_browser");
        // Before the panel, which would take Escape: the menu takes it first.
        if let Some(at) = self.sort_menu {
            let items: Vec<_> = (ModuleOrdering::MENU.into_iter().enumerate())
                .map(|(row, sort)| PopupItem {
                    current: sort == self.sort,
                    ..PopupItem::new(sort, 0, row, sort.label())
                })
                .collect();
            let result = modal(ctx, cx, id.with("sort menu"), at, &items);
            self.sort_picked = result.selected.or(self.sort_picked);
            if let Some(sort) = result.selected.filter(|&s| s != self.sort) {
                // A sort is a list refresh, from the top.
                self.sort = sort;
                self.panel.refreshed();
                self.panel.scroll_y = 0.0;
            }
            if result.close {
                self.sort_menu = None;
            }
        }
        let chrome = Chrome {
            id,
            title: "Modules",
            back: false,
            buttons: &[SideButton::Search, SideButton::Plain("Sort", true)],
            popup: self.sort_menu.is_some(),
        };
        let sort = self.sort;
        let click = self.panel.show(ctx, cx, &chrome, |query| {
            let row = |row: Row| PanelRow {
                name: row.name(),
                folder: false,
                h: ROW_H,
                sub: None,
                kind: match row {
                    Row::Section(_) => RowKind::Section,
                    Row::Item(entry) => entry
                        .action(instruments)
                        .map_or(RowKind::Disabled, RowKind::Enabled),
                },
                picked: false,
            };
            rows(query, sort).into_iter().map(row).collect()
        })?;
        match click {
            PanelClick::Close => Some(BrowserAction::Close),
            PanelClick::Row(action) => Some(action),
            // `Sort`, the one button the panel hands back.
            PanelClick::Button(i) => {
                self.sort_menu = Some(layout(ctx.content_rect(), 1.0, cx.m).button(i));
                None
            }
            // The browser asks for no name and has no folders.
            PanelClick::Enter | PanelClick::Back => None,
        }
    }
}

// ---- the side browser panel, which the preset picker shares ----

/// A round button down the panel's left edge.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum SideButton<'a> {
    /// `Search`: toggles the search bar ([`SearchBar`]), which hands its query to the rows. The panel handles it.
    Search,
    /// Any other: its label, and whether it is enabled (else dimmed and inert). A click is the caller's.
    Plain(&'a str, bool),
    /// [`Self::Plain`] that is lit (a filled circle, as an open search's) while on: its label, whether it is on, and
    /// whether it is enabled (`Select`). One lit is select mode: Escape, `startSearch` and the search bar's hiding
    /// leave it with a [`PanelClick::Button`] for it, as a tap would.
    Toggle(&'a str, bool, bool),
    /// The ≡ options button (icon 27), enabled or not. Its id is `options`.
    Menu(bool),
}

/// The panel around its rows.
pub(crate) struct Chrome<'a> {
    pub id: Id,
    /// The header's text, top-left in large text: the root's title, or inside a folder its [`crumb_text`].
    pub title: &'a str,
    /// Inside a folder: the Back arrow shows at the header's left, and Back, Ctrl+Up (`leaveFolder`) give
    /// [`PanelClick::Back`]. At the root it is hidden.
    pub back: bool,
    /// Down the left edge, in order.
    pub buttons: &'a [SideButton<'a>],
    /// A popup of the caller's is open over the panel (its ≡ menu, a message): it has the keyboard, so `startSearch`
    /// waits.
    pub popup: bool,
}

/// The header's text inside a folder: the breadcrumb path joined with " | ", after 5 leading spaces (room for the
/// Back arrow).
pub(crate) fn crumb_text<'a>(parts: impl IntoIterator<Item = &'a str>) -> String {
    let parts: Vec<&str> = parts.into_iter().collect();
    format!("     {}", parts.join(" | "))
}

/// How a row looks, and what a click on it gives.
pub(crate) enum RowKind<T> {
    /// A section header on a darker band; inert.
    Section,
    /// A click gives this.
    Enabled(T),
    /// [`Self::Enabled`], selected: drawn as an inverted bar (`text`, with `menuFill` ink), as a current popup item.
    Selected(T),
    /// Dimmed and inert.
    Disabled,
}

/// A row of the list.
pub(crate) struct PanelRow<'a, T> {
    pub name: &'a str,
    /// A folder: a tap opens it, which is its own feedback, so it never flashes.
    pub folder: bool,
    /// Its height, in U ([`ROW_H`]).
    pub h: f32,
    /// A search result's folder path, under the name ([`search_bar::place_line`]).
    pub sub: Option<&'a str>,
    pub kind: RowKind<T>,
    /// The last pick: the audition strip sits over it while it is on. It is drawn as any row (it has no look of its
    /// own, and the loading pulse, a `text` fill under `menuFill` text, is drawn for a plain row).
    pub picked: bool,
}

/// What the user clicked in the panel this frame.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum PanelClick<T> {
    /// The close X, a click outside the panel, or Escape.
    Close,
    /// The left button at this index (never `Search`); also the lit [`SideButton::Toggle`] that select mode leaves.
    Button(usize),
    /// An enabled row; also the picked row a second finger re-picks through the audition strip.
    Row(T),
    /// Enter in the name field [`SidePanel::ask`] opened: take the name with [`SidePanel::take_prompt`].
    Enter,
    /// Back up one folder: the header's Back arrow, or `leaveFolder` (Ctrl+Up, Cmd+Up on a Mac), shown inside a
    /// folder only ([`Chrome::back`]).
    Back,
}

/// The open panel's state: the search, the scroll, the slide-in. Make a fresh one each time it opens: every opening
/// starts with the search reset and the keyboard focus moved into the browser.
#[derive(Default)]
pub(crate) struct SidePanel {
    search: SearchBar,
    /// It has drawn: the focus went into its list.
    opened: bool,
    /// The audition strip was on last frame: it took the focus as it came on.
    pub auditioned: bool,
    /// How far the list is scrolled up, in points.
    pub scroll_y: f32,
    /// It takes another panel's place, already in place: no slide-in.
    in_place: bool,
    /// The next frame scrolls the picked row just into view, once.
    pub reveal: bool,
    /// The name field, while the caller asks for a name.
    prompt: Option<Prompt>,
    /// The audition strip over the picked row (instrument presets).
    pub audition: Audition,
    /// The sample strip at the bottom (sample pickers), once a sample is picked.
    pub sample: Option<SampleStrip>,
    /// Row flash on pick: the item row last tapped, by index and name (a list read again may move another row to that
    /// index: the flash then shows on none), and when (input time, in seconds), while its flash runs.
    flash: Option<(usize, String, f64)>,
}

/// The name field over the list ([`SidePanel::ask`]).
struct Prompt {
    title: &'static str,
    text: String,
    /// Just opened: the field takes the keyboard focus, its cursor at the end.
    focus: bool,
}

/// Where the parts of the panel go on `screen`, at slide-in position `p` (0 = just off the left edge, 1 = in place):
/// the panel is drawn at x + (p − 1)·W.
pub(crate) struct Layout {
    pub panel: Rect,
    pub header: Rect,
    pub close: Pos2,
    /// The Back arrow's touch area, inside a folder.
    pub back: Rect,
    /// Below the header, right of the button column. An open search bar moves its top down 1.75 U.
    pub list: Rect,
    /// The centre of the first left button.
    first_button: Pos2,
    button_step: f32,
}

impl Layout {
    /// The centre of the left button at `i`.
    pub fn button(&self, i: usize) -> Pos2 {
        self.first_button + vec2(0.0, i as f32 * self.button_step)
    }
}

pub(crate) fn layout(screen: Rect, p: f32, m: &Metrics) -> Layout {
    let w = m.round_px(m.u(WIDTH).min(screen.width()));
    let left = m.round_px(screen.left() + (p - 1.0) * w);
    let panel = Rect::from_min_size(pos2(left, screen.top()), vec2(w, screen.height()));
    let header = panel.with_max_y(panel.top() + m.u(HEADER_H));
    Layout {
        panel,
        header,
        close: pos2(
            panel.right() - m.u(CLOSE_RIGHT + CLOSE_W / 2.0),
            header.center().y,
        ),
        back: header.with_max_x(left + m.u(BACK_W)),
        list: Rect::from_min_max(pos2(left + m.u(COLUMN_W), header.bottom()), panel.max),
        first_button: pos2(
            left + m.u(COLUMN_W) / 2.0,
            header.bottom() + m.u(BUTTON_GAP + BUTTON_D / 2.0),
        ),
        button_step: m.u(BUTTON_D + BUTTON_APART),
    }
}

impl SidePanel {
    /// Asks for a name in a field over the list (`Create folder`, `Rename`), under the small `title`, starting with
    /// `text`. Enter gives [`PanelClick::Enter`], and [`Self::take_prompt`] the name; Escape or a click elsewhere
    /// drops it, and the panel stays open.
    pub(crate) fn ask(&mut self, title: &'static str, text: String) {
        self.prompt = Some(Prompt {
            title,
            text,
            focus: true,
        });
    }

    /// The search query; empty while the bar is hidden.
    pub(crate) fn query(&self) -> &str {
        self.search.query()
    }

    /// The name [`Self::ask`] asked for, once Enter gave [`PanelClick::Enter`]; it closes the field.
    pub(crate) fn take_prompt(&mut self) -> Option<String> {
        self.prompt.take().map(|p| p.text)
    }

    /// A panel that takes another's place on screen, already in place: no slide-in (the SoundFont picker's patches).
    pub(crate) fn in_place() -> Self {
        Self {
            in_place: true,
            ..Self::default()
        }
    }

    /// Shows the panel over everything else; call it every frame while it is open. `rows` gives the list for the
    /// search query (empty while the bar is hidden: the normal folder listing). The panel is modal: a click outside it
    /// (primary, secondary or middle) closes it and never reaches the widgets underneath.
    ///
    /// The browser below the header (its buttons, search bar and list) is focus container `id.with("browser")`, its list
    /// `id.with("rows")`, where the focus goes as the panel opens. `startSearch` (Ctrl+F) shows the search bar while the
    /// focus is inside the browser.
    ///
    /// Escape: this consumes it, and acts in order: it drops an open name field, else hides the search bar,
    /// else leaves select mode, else closes. In the app the panel is an overlay above the rack, and the app hands Escape to
    /// the topmost overlay only (`views::take_escape`, `Slots::overlays`), so one press closes the panel and the
    /// rack under it never sees it. A caller outside that mechanism must call this before any other consumer of
    /// Escape in the frame, or that consumer takes the key and the panel stays open. The backdrop that blocks
    /// the clicks sits at `Order::Middle`, so everything it must cover (the rack and its screens) has to live
    /// below `Order::Middle`; a layer at or above it is not blocked.
    pub fn show<'r, T: Copy>(
        &mut self,
        ctx: &Context,
        cx: &UiCtx,
        chrome: &Chrome,
        rows: impl FnOnce(&str) -> Vec<PanelRow<'r, T>>,
    ) -> Option<PanelClick<T>> {
        let (c, m) = (&cx.theme.colors, cx.m);
        let (text, ink) = (c.text.to_color32(), c.background.to_color32());
        let screen = ctx.content_rect();
        let id = chrome.id;
        // The slide-in: p ← (p + 1)/2 a drawn frame, the screen dimmed to α 0.6·p. The panel's touch areas move with it
        // (egui's widgets are where they are painted).
        let p = match self.in_place {
            true => 1.0,
            false => anim::browser_slide(anim::frame(ctx, id)),
        };
        if p < 1.0 {
            ctx.request_repaint();
        }
        // Laid out at rest: the touch handlers never check the animation. Only the drawing slides (below).
        let at = layout(screen, 1.0, m);
        let dx = layout(screen, p, m).panel.left() - at.panel.left();
        // The dim, straight into the backdrop's layer: egui shows it even in the frame it first lays the area out.
        ctx.layer_painter(LayerId::new(Order::Middle, id.with("backdrop")))
            .rect_filled(screen, 0.0, anim::dim(ink, anim::DIM_ALPHA * p));

        // The popup's backdrop (popup.rs): below the panel, above the screens.
        let backdrop = crate::ui::backdrop(ctx, id);
        let (browser_id, rows_id, field) = (id.with("browser"), id.with("rows"), id.with("query"));
        if !std::mem::replace(&mut self.opened, true) {
            focus::enter(ctx, rows_id);
        }
        // After a pick that turned the audition strip on, the focus sits on the strip, outside the browser.
        let strip_id = id.with("audition");
        if self.audition.on && !std::mem::replace(&mut self.auditioned, true) {
            focus::move_to(ctx, strip_id);
        }
        self.auditioned &= self.audition.on;
        // Select mode, as a tap on its lit toggle leaves it.
        let selecting = (chrome.buttons.iter())
            .position(|b| matches!(b, SideButton::Toggle(_, true, _)))
            .map(PanelClick::Button);
        let outside =
            backdrop.clicked() || backdrop.secondary_clicked() || backdrop.middle_clicked();
        let mut click = outside.then_some(PanelClick::Close);
        // Escape drops an open name field first, then hides the search bar (leaving select mode, the focus on
        // `Search`), else leaves select mode, else closes.
        if crate::ui::close_pressed(ctx) && self.prompt.take().is_none() {
            if self.search.shown() {
                self.search.end();
                self.refreshed();
                focus::move_to(ctx, id.with("search"));
                click = click.or(selecting);
            } else {
                click = click.or(selecting).or(Some(PanelClick::Close));
            }
        }
        let search = chrome.buttons.contains(&SideButton::Search);
        let free = self.prompt.is_none() && !chrome.popup;
        if search && free && self.search.start_key(ctx, browser_id, field) {
            self.refreshed();
            click = click.or(selecting);
        }
        // The keys a text field would take are its own.
        let keys = keymap::for_os(ctx.os());
        let typing = ctx.wants_keyboard_input();
        if chrome.back && !typing && ctx.input_mut(|i| keys.consume_press(i, "leaveFolder")) {
            click = click.or(Some(PanelClick::Back));
        }

        Area::new(id)
            .order(Order::Foreground)
            .fixed_pos(at.panel.min)
            .constrain(false)
            .fade_in(false)
            .show(ctx, |ui| {
                // The whole panel, so it swallows the clicks that hit no control.
                ui.allocate_exact_size(at.panel.size(), Sense::hover());
                let painter = ui.painter().clone();
                painter.rect_filled(at.panel, 0.0, c.menu_fill.to_color32());
                // The header's text runs up to the close X: the title at the root, the breadcrumb in a folder (its
                // leading spaces clear the Back arrow).
                let close_left = at.close.x - m.u(CLOSE_W / 2.0);
                let title = Rect::from_x_y_ranges(
                    at.panel.left() + m.u(PAD_X)..=close_left - m.u(PAD_X),
                    at.header.y_range(),
                );
                let (font, left) = (m.font_id(FontSize::Pt18), Align2::LEFT_CENTER);
                let tail = if chrome.back { CRUMB_TAIL_K } else { TAIL_K };
                truncated(&painter, title, left, chrome.title, font, text, tail);
                let px = 1.0 / m.ppp();
                let line = at.header.with_min_y(at.header.bottom() - px);
                painter.rect_filled(line, 0.0, text);
                let close =
                    filled_circle_button(ui, id.with("close"), at.close, m.u(CLOSE_W), text);
                draw_icon(&painter, close.rect(), IconId::Close, ink, CLOSE_ICON, m);
                if close.clicked() {
                    click = Some(PanelClick::Close);
                }
                if chrome.back {
                    let back = ui.interact(at.back, id.with("back"), Sense::click());
                    draw_icon(&painter, at.back, IconId::Previous, text, BACK_ICON, m);
                    if back.clicked() {
                        click = Some(PanelClick::Back);
                    }
                }

                // The browser: the left buttons, the search bar and the list.
                let browser = at.panel.with_min_y(at.header.bottom());
                focus::open(ui, browser_id, browser);
                // Search toggles the bar; the others are the caller's.
                let d = m.u(BUTTON_D);
                for (i, &button) in chrome.buttons.iter().enumerate() {
                    let (label, icon, enabled) = match button {
                        SideButton::Search => ("Search", None, true),
                        SideButton::Plain(label, enabled)
                        | SideButton::Toggle(label, _, enabled) => (label, None, enabled),
                        SideButton::Menu(enabled) => ("options", Some(IconId::Menu), enabled),
                    };
                    let center = at.button(i);
                    let mut color = text;
                    if !enabled {
                        color = text.gamma_multiply(DISABLED_ALPHA);
                    }
                    // By label: `modules_browser` / `search` is the Search button's id.
                    let button_id = id.with(label.to_lowercase());
                    let response = outline_circle_button(ui, button_id, center, d, color, m);
                    if response.clicked() && enabled {
                        match button {
                            SideButton::Search => {
                                match self.search.shown() {
                                    true => self.search.end(),
                                    false => self.search.start(),
                                }
                                self.refreshed();
                                click = click.or(selecting);
                            }
                            _ => click = Some(PanelClick::Button(i)),
                        }
                    }
                    let on = match button {
                        SideButton::Search => self.search.shown(),
                        SideButton::Toggle(_, on, _) => on,
                        _ => false,
                    };
                    if on {
                        painter.circle_filled(center, d / 2.0, text);
                        color = ink;
                    }
                    match icon {
                        Some(icon) => {
                            draw_icon(&painter, response.rect(), icon, color, MENU_ICON, m)
                        }
                        None => {
                            let (rect, align) = (response.rect(), Align2::CENTER_CENTER);
                            shrink(&painter, m, rect, align, label, BUTTON_LABEL_PT, color);
                        }
                    }
                }

                let row_h = m.round_px(m.u(ROW_H));
                let mut list = at.list;
                // The sample strip takes the panel's bottom; the list shrinks by its height.
                if let Some(strip) = &mut self.sample {
                    list.max.y -= m.u(STRIP_H);
                    strip.show(ui, cx, at.panel, id);
                }
                if self.search.shown() {
                    list.min.y += m.u(search_bar::DROP);
                    if self.search.show(ui, cx, field, browser) {
                        self.refreshed();
                    }
                }

                if let Some(prompt) = &mut self.prompt {
                    let label = list.with_max_y(list.top() + m.u(CRUMB_H));
                    let field = Rect::from_x_y_ranges(
                        list.x_range(),
                        label.bottom()..=label.bottom() + row_h,
                    );
                    list.min.y = field.bottom();
                    let font = m.font_id(FontSize::Pt8);
                    let left = label.left_center() + vec2(m.u(PAD_X), 0.0);
                    painter.text(left, Align2::LEFT_CENTER, prompt.title, font, text);
                    painter.rect_filled(field, 0.0, c.control_fill.to_color32());
                    let field_id = id.with("prompt");
                    if prompt.focus {
                        // The cursor at the end of the name, not where the last name field left it.
                        let mut state = TextEditState::load(ui.ctx(), field_id).unwrap_or_default();
                        let end = CCursor::new(prompt.text.chars().count());
                        state.cursor.set_char_range(Some(CCursorRange::one(end)));
                        state.store(ui.ctx(), field_id);
                    }
                    let edit = TextEdit::singleline(&mut prompt.text)
                        .id(field_id)
                        .char_limit(NAME_LIMIT)
                        .frame(false)
                        .font(m.font_id(FontSize::Pt10))
                        .text_color(text)
                        .margin(vec2(m.u(PAD_X), 0.0))
                        .desired_width(field.width())
                        .min_size(field.size())
                        .vertical_align(Align::Center);
                    let mut response = ui.put(field, edit);
                    crate::android_text::edit(&mut response, &mut prompt.text, NAME_LIMIT);
                    if std::mem::take(&mut prompt.focus) {
                        response.request_focus();
                    }
                    // Enter names; anything else that takes the focus away drops the field.
                    if response.lost_focus() {
                        match ui.input(|i| i.key_pressed(Key::Enter)) {
                            true => click = Some(PanelClick::Enter),
                            false => self.prompt = None,
                        }
                    }
                }

                // Drags and the wheel scroll; rows sense clicks only, so a drag that starts on one still scrolls.
                let rows = rows(self.search.query());
                let heights: Vec<f32> = rows.iter().map(|r| m.round_px(m.u(r.h))).collect();
                let content_h: f32 = heights.iter().sum();
                if std::mem::take(&mut self.reveal)
                    && let Some(i) = rows.iter().position(|r| r.picked)
                {
                    let top: f32 = heights[..i].iter().sum();
                    let shown = self.scroll_y.max(top + heights[i] - list.height());
                    self.scroll_y = shown.min(top);
                }
                let drag = ui.interact(list, id.with("list"), Sense::drag());
                let mut moved = drag.drag_delta().y;
                if drag.contains_pointer() {
                    moved += ui.input(|i| i.smooth_scroll_delta.y);
                }
                self.scroll_y =
                    (self.scroll_y - moved).clamp(0.0, (content_h - list.height()).max(0.0));
                let scroll = m.round_px(self.scroll_y);

                let painter = ui.painter_at(list);
                focus::open(ui, rows_id, list);
                // The picked row's full rect, what a click on it gives, its index and name, for the audition strip.
                let mut picked = None;
                // The flashing row's full rect, when it shows.
                let mut flashed = None;
                let now = ui.input(|i| i.time);
                let mut top = list.top() - scroll;
                for (i, (row, row_h)) in rows.into_iter().zip(heights).enumerate() {
                    let rect = Rect::from_x_y_ranges(list.x_range(), top..=top + row_h);
                    top += row_h;
                    let top = rect.top();
                    if let (true, RowKind::Enabled(pick) | RowKind::Selected(pick)) =
                        (row.picked, &row.kind)
                    {
                        picked = Some((rect, *pick, i, row.name));
                    }
                    if top >= list.bottom() || top + row_h <= list.top() {
                        continue;
                    }
                    let selected = matches!(row.kind, RowKind::Selected(_));
                    let color = match row.kind {
                        RowKind::Section => {
                            painter.rect_filled(rect, 0.0, c.control_fill.to_color32());
                            text
                        }
                        RowKind::Enabled(pick) | RowKind::Selected(pick) => {
                            let row_id = id.with(("row", i));
                            let response = crate::ui::focus::interact(
                                ui,
                                rect.intersect(list),
                                row_id,
                                Sense::click(),
                            );
                            if response.clicked() {
                                click = Some(PanelClick::Row(pick));
                                // Row flash on pick: every tap on an item row starts it over, a re-tap of the
                                // picked row too; a tap in select mode only selects.
                                if !row.folder && selecting.is_none() {
                                    self.flash = Some((i, row.name.to_owned(), now));
                                }
                            }
                            if selected {
                                painter.rect_filled(rect, 0.0, text);
                                c.menu_fill.to_color32()
                            } else {
                                if response.is_pointer_button_down_on() {
                                    let band = text.gamma_multiply(PRESSED_ALPHA);
                                    painter.rect_filled(rect, 0.0, band);
                                }
                                text
                            }
                        }
                        RowKind::Disabled => text.gamma_multiply(DISABLED_ALPHA),
                    };
                    if (self.flash.as_ref()).is_some_and(|(f, name, _)| *f == i && name == row.name)
                    {
                        flashed = Some(rect);
                    }
                    let mut label = rect.shrink2(vec2(m.u(PAD_X), 0.0));
                    if let Some(place) = row.sub {
                        search_bar::place_line(&painter, m, &mut label, place, color);
                    }
                    let (left, font) = (Align2::LEFT_CENTER, m.font_id(ROW_FONT));
                    truncated(&painter, label, left, row.name, font, color, TAIL_K);
                }
                focus::close(ctx, rows_id);
                // The loading pulse over the picked row, the audition strip's or not (a Slicer's or a sample picker's
                // pick decoding on the worker pulses too).
                let shown = picked.filter(|(rect, ..)| rect.intersect(list).is_positive());
                if let Some((rect, ..)) = shown {
                    self.audition.paint(&painter, cx, rect, now);
                }
                if self.audition.on {
                    // Over the picked row, after it: it takes the touch before the row does.
                    if let Some((rect, ..)) = shown {
                        ui.interact(rect.intersect(list), strip_id, Sense::click_and_drag());
                    }
                    if ui.input(focus_lost) {
                        self.audition.release();
                    }
                    let steps = fingers(ui, strip_id);
                    let tapped = steps.iter().any(|s| matches!(s, Finger::Down(..)));
                    if self.audition.touch(&steps, picked.map(|(rect, ..)| rect))
                        && let Some((_, pick, ..)) = picked
                    {
                        click = Some(PanelClick::Row(pick));
                    }
                    // Enter is the strip's while it has the focus; a note it holds goes as the focus leaves.
                    let (pressed, down) = match focus::focused(ctx) == Some(strip_id) && !typing {
                        true => {
                            ctx.input_mut(|i| (keys.consume_press(i, "open"), keys.down(i, "open")))
                        }
                        false => (false, false),
                    };
                    self.audition.key(pressed, down);
                    // Row flash on pick: the strip takes a re-tap of the picked row and Enter on it (they play it):
                    // those flash it too.
                    if let (true, Some((_, _, i, name))) = (tapped || pressed, picked) {
                        self.flash = Some((i, name.to_owned(), now));
                    }
                }
                // Row flash on pick, over the row and whatever marks it (the loading pulse), from the frame after the
                // tap: a picker that closes on its pick shows no flash at all. It repaints only while it runs.
                if let Some((.., t0)) = self.flash {
                    let left = 1.0 - ((now - t0) / FLASH_S) as f32;
                    match left > 0.0 {
                        true => {
                            if let (true, Some(rect)) = (now > t0, flashed) {
                                let fill = text.gamma_multiply(FLASH_ALPHA * left);
                                painter.rect_filled(rect, 0.0, fill);
                            }
                            ui.ctx().request_repaint();
                        }
                        false => self.flash = None,
                    }
                }
                if content_h > list.height() {
                    let w = m.u(SCROLLBAR_W);
                    let thumb = Rect::from_min_size(
                        pos2(
                            list.right() - w,
                            list.top() + scroll / content_h * list.height(),
                        ),
                        vec2(w, list.height() * list.height() / content_h),
                    );
                    painter.rect_filled(thumb, 0.0, text.gamma_multiply(SCROLLBAR_ALPHA));
                }
                focus::close(ctx, browser_id);
                // The audition strip is no part of the browser: a focus target of its own, over the picked row, in view
                // or not.
                if let (true, Some((rect, ..))) = (self.audition.on, picked) {
                    focus::register(ui, strip_id, rect, focus::Kind::Container);
                }
            });
        // The slide, on what the panel drew this frame (its layer only).
        ctx.transform_layer_shapes(
            LayerId::new(Order::Foreground, id),
            TSTransform::from_translation(vec2(dx, 0.0)),
        );
        if click.is_some() {
            // Nothing else redraws the screen without the panel, or with its new rows.
            ctx.request_repaint();
        }
        click
    }

    /// The search ends as a found folder opens: the bar hides, its query cleared, the list refreshed.
    pub(crate) fn end_search(&mut self) {
        self.search.end();
        self.refreshed();
    }

    /// The list was refreshed (a search, a folder entered or left, a sort, select mode): the audition strip turns off
    /// and the sample strip stops.
    pub(crate) fn refreshed(&mut self) {
        // The rows change: no flash stays on what is now another row.
        self.flash = None;
        self.audition.off();
        if let Some(strip) = &mut self.sample {
            strip.stop();
        }
    }
}

#[cfg(test)]
mod tests;
