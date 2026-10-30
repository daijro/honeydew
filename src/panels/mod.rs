//! Effect and instrument panels as data and the
//! renderer that draws them.
//!
//! A panel is a tree of [`Part`]s: `HStack`, `VStack` and `Layer` containers, and leaves that are controls. Every
//! part has its layout record and a [`Show`] predicate that decides when it shows; a control also has its widget, its
//! label and the parameter it is bound to. The builders are in [`build`], one call per node of the tree; each
//! module's panel is in its own file under `fx/` (effects, by type id) or `inst/` (instruments), next to the
//! reference rects its data-driven test checks.
//!
//! [`show`] draws a panel for a module instance: it resolves the predicates against the params and the selected
//! tab, lays the tree out in the content rect with [`layout`], draws each visible control with the widget
//! kit bound to `params[p]` (default, bipolar flag, menu steps and value text from the parameter registry), and
//! returns the pass's [`Edits`]: the params the user changed, then those the panel's [`Hook`] changed, and the edited
//! settings and tab changes. It never talks to the audio side; the caller sends the params on. A change that shows or
//! hides parts is laid out again in the same frame.
//!
//! A module-drawn display draws itself ([`Leaf::draw`], a [`Draw`]) in tree order, and may edit params like a control;
//! one without a draw is a [`display_area`](widgets::display_area) placeholder. A panel's [`Panel::underlay`] draws
//! before the controls and its [`Overlay`] after them.

use std::borrow::Cow;
use std::collections::HashMap;
use std::sync::LazyLock;

use eframe::egui::{Align2, Color32, Context, Id, Rect, Response, Ui, vec2};

use crate::icons::IconId;
use crate::metrics::{FontSize, Metrics};
use crate::midi::{CcLink, links};
use crate::params::{ParamKind, ParamSpec, TextCx, menu_index, menu_value, module_spec};
use crate::song::ModuleInstance;
use crate::theme::{Rgba, ThemeColors};
use crate::ui::ctrl_menu::CtrlAction;
use crate::ui::fine_tune;
use crate::ui::layout::{self, LayoutRecord, Node, NodeId};
use crate::ui::widgets::{self, ArcSlider, Bar, Button, Control, ItemList, ParamMenu, Pill};
use crate::ui::{UiCtx, primary_down_live};

/// Registers the module files of `fx` or `inst`, declared beside it as plain `mod` items (rustfmt does not follow a
/// `mod` declared by a macro): `IDS`, `panel(id)` and, for the tests, `reference(id)`.
macro_rules! modules {
    ($($id:literal => $file:ident,)*) => {
        /// The type ids with a module file, in id order.
        pub(super) const IDS: &[u16] = &[$($id),*];

        /// The panel of `type_id`, `None` while its file has none.
        pub(super) fn panel(type_id: u16) -> Option<super::Panel> {
            match type_id {
                $($id => $file::panel(),)*
                _ => None,
            }
        }

        /// The reference rects of `type_id`'s file.
        #[cfg(test)]
        pub(super) fn reference(type_id: u16) -> Option<&'static super::build::Reference> {
            match type_id {
                $($id => Some(&$file::REFERENCE),)*
                _ => None,
            }
        }
    };
}

pub mod build;
pub(crate) mod cstm;
mod fx;
pub(crate) use fx::couple_param;
pub(crate) use fx::couple_scale;
pub use fx::post_feed;
pub(crate) mod inst;
pub use inst::slicer::post_loop;
#[cfg(test)]
mod tests;

/// When a part is shown. Re-evaluated every frame, so automation that flips a param swaps the parts too.
#[derive(Clone, Copy, Debug, PartialEq)]
pub enum Show {
    Always,
    /// Hidden whatever the state: a control the panel builds but never shows (the Multiband's solo-state button
    /// `#38`).
    Never,
    /// Page `k` (from 0) of the module's tab strip is selected: the `{TAB}` pages.
    Tab(usize),
    /// Param `p` is on item `k`: a menu's item index ([`ParamKind::item`]), or for any other param 0 below 0.5 and
    /// 1 from there (a toggle's off and on).
    Item(usize, usize),
    /// The setting of control `#n`, one without a param, is on item `k`: the item index of a list or spinner, 0 off
    /// and 1 on for a toggle or button (and for a knob or slider 0 below 0.5 and 1 from there). The Analyzer's
    /// `Mode` swap, the MiniSynth's FX sub-tabs.
    Setting(u32, usize),
    /// Param `p` is above `x`: the SoundFont Player's Envelope is `Custom` above 0.5, and Tape
    /// Stop's `Sync` shows its step arcs above 0.5, where [`Self::Item`] would count 0.5 as
    /// on.
    Above(usize, f32),
    /// Module-state flag `key` (a [`flag`] key), which the card wiring sets in [`PanelState::flags`], is on item `k`;
    /// an absent flag is 0: what the panel cannot know from params and settings.
    Flag(&'static str, usize),
    Not(&'static Show),
    /// Every one of them holds.
    All(&'static [Show]),
}

impl Show {
    /// Whether the part is shown for `module` in panel state `state`. A param the module lacks, or a setting the
    /// state lacks, holds nothing.
    pub fn holds(&self, module: &ModuleInstance, state: &PanelState) -> bool {
        match *self {
            Self::Always => true,
            Self::Never => false,
            Self::Tab(k) => state.tab == k,
            Self::Item(p, k) => item(module, p) == Some(k),
            Self::Setting(n, k) => state.settings.get(&n).map(|&v| setting_item(v)) == Some(k),
            Self::Above(p, x) => module.params.get(p).is_some_and(|&v| v > x),
            Self::Flag(key, k) => state.flags.get(key).copied().unwrap_or(0) == k,
            Self::Not(show) => !show.holds(module, state),
            Self::All(all) => all.iter().all(|s| s.holds(module, state)),
        }
    }
}

/// The item param `p` is on, read as [`Show::Item`] says.
fn item(module: &ModuleInstance, p: usize) -> Option<usize> {
    let v = *module.params.get(p)?;
    let spec = module_spec(module.type_id).and_then(|m| m.params.get(p));
    Some(match spec.map(|s| s.kind) {
        Some(kind) if !kind.items().is_empty() => kind.item(v),
        _ => usize::from(v >= 0.5),
    })
}

/// Whether `leaf`, bound to the param `pspec` describes (if any), draws bipolar: the registry's rule
/// ([`ParamSpec::bipolar`], which marks every bipolar control), else for a control without a param a default
/// of exactly 0.5.
pub(crate) fn is_bipolar(leaf: &Leaf, pspec: Option<&ParamSpec>) -> bool {
    pspec.map_or(leaf.default == 0.5, ParamSpec::bipolar)
}

/// The Arpeggiator's module type.
const ARPEGGIATOR_TYPE: u16 = 51;

/// What a double tap and `Reset` set on `leaf` of a module of type `type_id`, bound to the param `pspec` describes
/// (if any): the registry's ([`ParamSpec::reset`]), else for a control without a param its default. The
/// Arpeggiator's knobs reset to 0; only the reset, their rings keep the registry's origin
/// ([`is_bipolar`]).
pub(crate) fn reset_of(leaf: &Leaf, pspec: Option<&ParamSpec>, type_id: u16) -> f32 {
    if type_id == ARPEGGIATOR_TYPE && leaf.widget == Widget::Knob {
        return 0.0;
    }
    pspec.map_or(leaf.default, |s| s.reset)
}

/// The item a setting is on: settings of lists and spinners hold the item index itself, toggles and buttons 0 or 1.
fn setting_item(v: f32) -> usize {
    // `max` also reads NaN as 0.
    v.max(0.0).round() as usize
}

/// The widget a control is drawn with.
#[derive(Clone, Copy, Debug, PartialEq)]
pub enum Widget {
    /// Ring knob; bipolar as [`is_bipolar`] says (a param whose reset is 0.5).
    Knob,
    VSlider,
    HSlider,
    /// Pill toggle with its label on top, or none with an empty label.
    Toggle,
    /// Pill toggle with its label on the right.
    ToggleRight,
    /// Text toggle: no background, `text` when on and at 25 % when off. [`Leaf::momentary`] makes it a
    /// text button.
    TextToggle,
    /// The items are the menu param's, or [`Leaf::items`] without a param.
    Spinner,
    /// Menu button: the caption `"<label>: "` beside a `controlFill` box with the item; it opens the
    /// spinner's popup. Items as for [`Widget::Spinner`].
    MenuButton,
    /// Item list: a vertical list selector (bar `Right` or `Left`), a horizontal selector (`Bottom`) or the boxed
    /// horizontal list (`Boxed`). The label holds the items. Bound to a menu param, or without one a module
    /// setting (UI state).
    List(Bar),
    /// The tab strip that switches the `{TAB}` pages: not a param. The label holds the tab names.
    Tabs,
    /// Toggles its param between 0 and 1, or with [`Leaf::momentary`] holds 1 while pressed.
    Button {
        round: bool,
    },
    /// A static string.
    Text,
    /// A module-drawn display (meter, curve, graph): its [`Leaf::draw`], else a placeholder.
    Display,
    /// Takes its space and draws nothing.
    Spacer,
    /// `text` block at 10 %.
    Separator,
    /// Arc slider: `start` and `span` in degrees, counter-clockwise from +x; a
    /// `mirrored` arc fills from its far end; `track` punches the ring out and draws the separators. On a menu param
    /// it is segmented, one segment per item.
    Arc {
        start: f32,
        span: f32,
        mirrored: bool,
        track: Ink,
    },
    /// Tag slider (Tremolo): a pointer tag over the full height, pointing right (to a display on its right) when
    /// `faces_right`.
    Tag {
        faces_right: bool,
    },
    /// Graph knob: a ring knob plotting the envelope curve of its value inside, `Env(1 − x, c)` when
    /// `falling` (the Gate's `R. Curve`).
    GraphKnob {
        falling: bool,
    },
    /// XY pad: X is the param, Y is [`Leaf::y_param`] (Y up).
    XyPad,
}

impl Widget {
    /// It edits a value: a param, or without one a module setting.
    fn has_value(self) -> bool {
        !matches!(
            self,
            Self::Tabs | Self::Text | Self::Display | Self::Spacer | Self::Separator | Self::XyPad
        )
    }
}

/// A colour by theme role, or the module's channel colour.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Ink {
    Channel,
    Text,
    ModuleFill,
    ControlFill,
    Background,
    /// Nothing drawn: the "no fill" of the Scale's note buttons.
    Clear,
    /// The theme colour that stands out most against the channel colour: Note Echo's `Stop` label on its channel
    /// fill.
    Contrast,
}

impl Ink {
    pub fn color(self, colors: &ThemeColors, channel: Color32) -> Color32 {
        match self {
            Self::Channel => channel,
            Self::Text => colors.text.to_color32(),
            Self::ModuleFill => colors.module_fill.to_color32(),
            Self::ControlFill => colors.control_fill.to_color32(),
            Self::Background => colors.background.to_color32(),
            Self::Clear => Color32::TRANSPARENT,
            Self::Contrast => {
                let [r, g, b, a] = channel
                    .to_srgba_unmultiplied()
                    .map(|c| f32::from(c) / 255.0);
                let channel = Rgba { r, g, b, a };
                colors
                    .best_contrasting(channel, &colors.contrast_candidates())
                    .to_color32()
            }
        }
    }
}

/// A button's colours per state (the slot mappings of the module sections).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct ButtonColors {
    pub fill_on: Ink,
    pub fill_off: Ink,
    pub text_on: Ink,
    /// `None`: the on label at 25 % alpha.
    pub text_off: Option<Ink>,
}

/// A control: a leaf of the tree. The builders in [`build`] set the fields; most controls need only a few.
#[derive(Clone, Debug)]
pub struct Leaf {
    /// The control's `#n`, unique in the panel.
    pub id: u32,
    pub widget: Widget,
    /// The control's label. Item lists and tab strips hold their items, joined by ` · `.
    pub label: &'static str,
    /// The bound param. A control with none edits a module setting kept in [`PanelState::settings`].
    pub param: Option<usize>,
    /// XY pads: the Y param.
    pub y_param: Option<usize>,
    /// Sliders: the value readout strip. Toggles: the `On` / `Off` strip.
    pub value_text: bool,
    /// The label's font, a text control's, a button's text; `None` is the widget's own: FontSize 1 for labels and text
    /// controls, FontSize 2 for button text, text toggles and right-label toggles.
    pub font: Option<FontSize>,
    /// Text controls: the point of the rect the text hangs from, left-centre unless set.
    pub align: Align2,
    /// Stands in for the channel colour in the control: labels, values, rings and fills (the Comb Filter's `?` is
    /// `text`).
    pub ink: Ink,
    /// The label at run time, from the params (Tuned EQ band labels, MultiFX axis names); `label` is what it gives
    /// at the registry defaults.
    pub label_fn: Option<fn(&[f32]) -> String>,
    /// The value text, for a control the registry has no text for (the Tuner's `Reference`, `%.0f Hz`).
    pub value_fn: Option<fn(f32) -> String>,
    /// Displays: what the module draws in it; `None` keeps the placeholder.
    pub draw: Option<Draw>,
    /// Spinners and menu buttons without a param: their items.
    pub items: &'static [&'static str],
    /// Spinners and menu buttons: items per popup column (the Arpeggiator's `Arp`: 8).
    pub column_len: Option<usize>,
    /// Spinners and menu buttons: items that show but cannot be picked (the Scale's `Custom`).
    pub disabled_items: &'static [usize],
    /// A control without a param starts at this: an item index (lists, spinners), 0 or 1 (toggles, buttons) or a
    /// normalised value. It is also what a double click resets it to.
    pub default: f32,
    /// Buttons: colours per state; `None` keeps [`Button::new`]'s.
    pub colors: Option<ButtonColors>,
    /// Round buttons: the disc's radius is `shape × 0.5 × min(w, h)`.
    pub shape: f32,
    pub icon: Option<IconId>,
    /// Buttons and text toggles: on only while pressed, 1 from the press and 0 from the release, both reported.
    pub momentary: bool,
    /// Spinners: a tap anywhere on it is the card wiring's, reported in [`Edits::settings`], and opens no popup (the
    /// SoundFont's `Patch`, which opens the SoundFont picker). It shows [`PanelState::texts`]' text, else its first item.
    pub picker: bool,
    /// Buttons: a halo disc (or rect) behind the button, bigger by the pad, in [`Leaf::halo_ink`].
    pub halo: Option<Pad>,
    /// Buttons: the halo's colour; `None` is the channel colour.
    pub halo_ink: Option<Ink>,
    /// Buttons and sliders: enabled while this holds; otherwise no input, and a button's every colour at 50 % alpha
    /// (Note Echo's `Stop` with no echoes pending), a slider's fill and thumb centre `menuFill` (the SoundFont
    /// Player's envelope sliders while it is `Original`).
    pub enabled: Show,
    /// Sliders: in their colours while this holds; otherwise track, thumb rim, value bar and thumb centre all
    /// `moduleFill`, still taking input (the Arpeggiator's `Rotation` with Rhythm Off).
    pub lit: Show,
}

/// A distance in device pixels or in `t`.
#[derive(Clone, Copy, Debug, PartialEq)]
pub enum Pad {
    Px(f32),
    T(f32),
}

impl Pad {
    /// In device pixels.
    fn px(self, m: &Metrics) -> f32 {
        match self {
            Self::Px(px) => px,
            Self::T(n) => n * m.t() * m.ppp(),
        }
    }
}

impl Leaf {
    /// The label as drawn with `params`.
    pub fn label_for(&self, params: &[f32]) -> Cow<'static, str> {
        match self.label_fn {
            Some(f) => Cow::Owned(f(params)),
            None => Cow::Borrowed(self.label),
        }
    }
}

// Panels are built once (`panel_for`), so a `Leaf` inline costs nothing worth a `Box`.
#[allow(clippy::large_enum_variant)]
#[derive(Clone, Debug)]
pub enum PartKind {
    HStack(Vec<Part>),
    VStack(Vec<Part>),
    Layer(Vec<Part>),
    Leaf(Leaf),
}

/// A node of a panel tree.
#[derive(Clone, Debug)]
pub struct Part {
    pub kind: PartKind,
    pub record: LayoutRecord,
    pub show: Show,
}

/// Coupled behaviour of a module's controls that data cannot say (the Multiband's `Solo`). It runs after the
/// controls of every pass, with the `#n` the user edited in that pass (often none), may change params and settings,
/// and returns the params to send: those whose value it actually changed, and those it re-applies unchanged (the Note
/// Echo's delay on a Sync toggle).
pub type Hook = fn(&mut ModuleInstance, &mut PanelState, edited: &[u32]) -> Vec<usize>;

/// A module-drawn display, drawn in tree order in place of the placeholder: given the
/// leaf's widget id (for input and egui temp data, such as animation memory), its rect (points) and the leaf's ink
/// (the channel colour unless the leaf sets [`Leaf::ink`]). It draws inside the card clip, not its own rect (labels
/// overhang it); a clip it sets ends with it. It may change params and [`PanelState::tab`], and returns the params it
/// changed: those the module has go out with the pass's [`Edits`], so they reach the audio and the song like any
/// control's. A changed page re-lays out the panel. Write the param, then return its index only when its value
/// changed; a change to `state.settings` is not reported.
pub type Draw =
    fn(&mut Ui, &UiCtx, Id, Rect, &mut ModuleInstance, &mut PanelState, Color32) -> Vec<usize>;

/// What a module draws itself after its controls (the 3x Osc's sample name, the Tape Stop's readouts), inside the
/// card clip, given its panel state, the visible controls' `#n` and rects (points), the content rect (points, the
/// module area) and the channel colour. A display leaf draws itself ([`Draw`]); a [`Panel::underlay`] has the same
/// signature.
pub type Overlay = fn(&Ui, &UiCtx, &ModuleInstance, &PanelState, &[(u32, Rect)], Rect, Color32);

/// A module's panel: the root of its tree, which fills the content rect, its hook, its overlay and its underlay.
#[derive(Clone, Debug)]
pub struct Panel {
    pub root: Part,
    pub hook: Option<Hook>,
    pub overlay: Option<Overlay>,
    /// Drawn after the layout and before the first control (Pitcher 2's note bars).
    pub underlay: Option<Overlay>,
}

impl Panel {
    pub fn new(root: Part) -> Self {
        Self {
            root,
            hook: None,
            overlay: None,
            underlay: None,
        }
    }

    pub fn overlay(self, overlay: Overlay) -> Self {
        Self {
            overlay: Some(overlay),
            ..self
        }
    }

    /// Draws `underlay` after the layout and before the first control, with [`Overlay`]'s arguments.
    pub fn underlay(self, underlay: Overlay) -> Self {
        Self {
            underlay: Some(underlay),
            ..self
        }
    }

    pub fn hook(self, hook: Hook) -> Self {
        Self {
            hook: Some(hook),
            ..self
        }
    }
}

impl Part {
    /// Every control of the subtree, in tree (`#n`) order.
    pub fn leaves(&self) -> Vec<&Leaf> {
        match &self.kind {
            PartKind::Leaf(leaf) => vec![leaf],
            PartKind::HStack(c) | PartKind::VStack(c) | PartKind::Layer(c) => {
                c.iter().flat_map(Part::leaves).collect()
            }
        }
    }
}

/// The panel of module type `type_id` (an effect or an instrument), `None` for a type without one.
pub fn panel_for(type_id: u16) -> Option<&'static Panel> {
    static PANELS: LazyLock<Vec<(u16, Panel)>> = LazyLock::new(|| {
        let fx = fx::IDS.iter().filter_map(|&id| Some((id, fx::panel(id)?)));
        let inst = inst::IDS
            .iter()
            .filter_map(|&id| Some((id, inst::panel(id)?)));
        let lemon = [
            (honeydew_audio::lemon::REVERB, fx::lemon::reverb()),
            (honeydew_audio::lemon::LIMITER, fx::lemon::limiter()),
            (
                honeydew_audio::dynamic_eq::DYNAMIC_EQ,
                fx::dynamic_eq::panel(),
            ),
            (honeydew_audio::modulator::MODULATOR, fx::modulator::panel()),
        ];
        fx.chain(inst).chain(lemon).collect()
    });
    PANELS.iter().find(|(id, _)| *id == type_id).map(|(_, p)| p)
}

/// The items of a list or tab strip label.
fn item_names(label: &str) -> Vec<&str> {
    label.split(" · ").collect()
}

/// The keys of [`PanelState::flags`] ([`Show::Flag`]), shared by the panels and the card wiring.
pub mod flag {
    /// 1 on an audio-input channel, 2 on any other (the Tuner).
    pub const CHANNEL_KIND: &str = "channel_kind";
    /// The Slicer has a loop: its `CSTM` `PATH` names one. Its wiring sets it.
    pub const SAMPLE_LOADED: &str = "sample_loaded";
    /// 1: the SoundFont Player (or the 3x Osc's custom waveform) is loading; 2: the SoundFont Player's SoundFont failed
    /// to load. The SoundFont Player's wiring sets it; the 3x Osc's leaves it 0 (its file decodes before the engine
    /// builds, so it never shows loading).
    pub const LOADING: &str = "loading";
    /// The SuperSaw card is in its construction state, which an updated card never is.
    pub const CONSTRUCTING: &str = "constructing";
    /// Note Echo has echoes pending. No source yet.
    pub const PENDING: &str = "pending";
    /// The app octave offset that note names with an octave subtract (the Tuner): 1 with
    /// `Set middle C to C4` (MIDI 60 is `C4`), else 0 (`C5`).
    pub const OCTAVE_OFFSET: &str = "octave_offset";
}

/// What the panel keeps for one module instance besides its params. The caller keeps one per card.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct PanelState {
    /// The selected page of the tab strip.
    pub tab: usize,
    /// Values of the controls bound to no param (module settings), by `#n`, starting at their [`Leaf::default`]: the
    /// item index for lists and spinners, 0 or 1 for toggles and buttons, a normalised value for the rest. Those a
    /// module saves are in its `CSTM` blob ([`cstm`]).
    pub settings: HashMap<u32, f32>,
    /// Module-state flags for [`Show::Flag`] by [`flag`] key, set by the card wiring; absent is 0.
    pub flags: HashMap<&'static str, usize>,
    /// The file name of the module's loaded sample (the 3x Osc custom waveform, `CSTM` `PATH`), set by the card
    /// wiring; empty while none is loaded.
    pub sample_name: String,
    /// Text the card wiring gives controls by `#n`, in place of the panel's: a text control's label (the SoundFont's
    /// load error), a [`Leaf::picker`] spinner's value (the SoundFont's patch name).
    pub texts: HashMap<u32, String>,
}

impl PanelState {
    /// The state of a fresh card showing `panel`: the first page, and every control without a param at its default.
    pub fn new(panel: &Panel) -> Self {
        let mut state = Self::default();
        state.seed(panel);
        state
    }

    /// Gives every control of `panel` without a param and without a setting yet its default.
    fn seed(&mut self, panel: &Panel) {
        // ponytail: walks the tree into a fresh Vec every pass (tens of leaves per card); keep the unbound ids in the
        // Panel if a profile ever shows it.
        for leaf in panel.root.leaves() {
            if leaf.param.is_none() && leaf.widget.has_value() {
                self.settings.entry(leaf.id).or_insert(leaf.default);
            }
        }
    }
}

/// The visible controls of `panel` for `module` in panel state `state`, laid out in `rect` at `u` per `U`,
/// in tree order. Whatever unit `rect` and `u` are in, the rects come back in it; the whole-pixel rule wants device
/// pixels.
pub fn place<'a>(
    panel: &'a Panel,
    module: &ModuleInstance,
    state: &PanelState,
    rect: Rect,
    u: f32,
) -> Vec<(&'a Leaf, Rect)> {
    let mut leaves = Vec::new();
    let root = node(&panel.root, module, state, &mut leaves);
    layout::layout(&root, rect, u)
        .into_iter()
        .map(|(i, r)| (leaves[i as usize], r))
        .collect()
}

/// `part` as a layout node, with its visibility resolved. Each leaf's node id is its index in `leaves`.
fn node<'a>(
    part: &'a Part,
    module: &ModuleInstance,
    state: &PanelState,
    leaves: &mut Vec<&'a Leaf>,
) -> Node {
    let mut children =
        |c: &'a [Part]| -> Vec<Node> { c.iter().map(|c| node(c, module, state, leaves)).collect() };
    let node = match &part.kind {
        PartKind::HStack(c) => Node::hstack(children(c)),
        PartKind::VStack(c) => Node::vstack(children(c)),
        PartKind::Layer(c) => Node::layer(children(c)),
        PartKind::Leaf(leaf) => {
            leaves.push(leaf);
            Node::leaf((leaves.len() - 1) as NodeId)
        }
    };
    Node {
        record: part.record,
        visible: part.show.holds(module, state),
        ..node
    }
}

/// Lends the value texts of the panels drawn after it the song tempo and the audio's sample rate (0 without audio),
/// in egui temp data, as the rack does each frame; until then [`show`] reads 0 BPM and no audio ([`TextCx`]).
pub fn set_text_env(ctx: &Context, tempo_bpm: f64, sample_rate: f32) {
    let env = TextCx {
        params: &[],
        tempo_bpm,
        sample_rate,
    };
    ctx.data_mut(|d| d.insert_temp(text_env_id(), env));
}

fn text_env_id() -> Id {
    Id::new("panel value text env")
}

/// What [`set_text_env`] lent for this frame: the default (0 BPM, no audio) until it did.
fn text_env(ui: &Ui) -> TextCx<'static> {
    ui.data(|d| d.get_temp(text_env_id())).unwrap_or_default()
}

/// Lends the panels drawn after it their channel's MIDI CC links (its `CCLK`), as the rack does each frame: a bound
/// control whose param one of them drives has [`ParamMenu::Linked`], which enables its `Unlink from MIDI CC`. Until
/// then nothing is linked.
pub fn set_cc_links(ctx: &Context, links: Vec<CcLink>) {
    ctx.data_mut(|d| d.insert_temp(cc_links_id(), links));
}

fn cc_links_id() -> Id {
    Id::new("panel cc links")
}

/// What [`set_cc_links`] lent.
pub(crate) fn cc_links(ui: &Ui) -> Vec<CcLink> {
    ui.data(|d| d.get_temp(cc_links_id())).unwrap_or_default()
}

/// The audio's sample rate as [`set_text_env`] lent it, 0 without audio: for the displays whose ballistics run per
/// audio buffer.
pub(crate) fn sample_rate(ui: &Ui) -> f32 {
    text_env(ui).sample_rate
}

/// What one pass of [`show`] changed.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct Edits {
    /// The params the user changed, in drawing order, then those the panel's [`Hook`] changed: the caller sends
    /// them on.
    pub params: Vec<usize>,
    /// The `#n` of the controls without a param the user changed, in drawing order: presses on unbound buttons (the
    /// SoundFont's `Prev` and `Next`, the Slicer's `LOAD SAMPLE`) as well as settings, page changes on the tab
    /// strip (under the strip's `#n`: 0 on instrument panels), and displays that changed a param or the page.
    pub settings: Vec<u32>,
    /// The param whose control was pressed ([`widgets::Signal::pressed`]): the app's recent control.
    pub pressed: Option<usize>,
    /// The param whose control's parameter menu picked an item, and the item.
    pub pick: Option<(usize, CtrlAction)>,
}

/// Draws `panel` for `module` in the content rect `rect` (points) in the `channel` colour, and returns what the user
/// (and the panel's hook) changed in this pass. `state` is the module's [`PanelState`]; its controls without a param
/// that have no setting yet get their default. Widget ids derive from `ui`'s id and the module's instance id. A rect
/// that is not finite draws nothing and changes nothing. Everything is clipped to the module card around `rect`,
/// and only to it: texts are plain text, so a label wider than its control overhangs it, but
/// never the card. Value texts read the module's params and the [`set_text_env`] tempo and sample rate; a bound
/// control's `Unlink from MIDI CC` is enabled when the [`set_cc_links`] links drive its param.
///
/// egui may run a frame in several passes (a re-layout asks for one, see below), and each pass returns its own
/// changes: act on the changes of every pass, not only the last one's.
pub fn show(
    ui: &mut Ui,
    cx: &UiCtx,
    panel: &Panel,
    module: &mut ModuleInstance,
    state: &mut PanelState,
    rect: Rect,
    channel: Color32,
) -> Edits {
    let (ppp, u_px) = (cx.m.ppp(), cx.m.u_px());
    let base = ui.id().with(("module panel", module.instance_id));
    let spec = module_spec(module.type_id);
    let Some(rect_px) = content_px(rect, ppp) else {
        return Edits::default();
    };
    state.seed(panel);
    let (mut edits, mut edited) = (Edits::default(), Vec::new());
    // The card is the content rect plus 2 px left and right, the 1.5 U header and 1 px above, 0.5 U below.
    let (content, px) = (rect_px / ppp, 1.0 / ppp);
    let card = Rect::from_min_max(
        content.min - vec2(2.0 * px, cx.m.u(1.5) + px),
        content.max + vec2(2.0 * px, cx.m.u(0.5)),
    );
    let outer_clip = ui.clip_rect();
    ui.set_clip_rect(outer_clip.intersect(card));
    let placed = place(panel, module, state, rect_px, u_px);
    let shown = |placed: &[(&Leaf, Rect)]| placed.iter().map(|(l, _)| l.id).collect::<Vec<_>>();
    let before = shown(&placed);
    let drawn: Vec<(u32, Rect)> = placed.iter().map(|(l, r)| (l.id, *r / ppp)).collect();
    // ponytail: the params as the pass began, copied once per card per frame (the controls hold them mutably); a text
    // that reads another param catches up with an edit earlier in the same pass on the repaint the edit asks for.
    let params = module.params.clone();
    let linked = cc_links(ui);
    let text_cx = TextCx {
        params: &params,
        ..text_env(ui)
    };
    if let Some(underlay) = panel.underlay {
        underlay(ui, cx, module, state, &drawn, content, channel);
    }
    for &(leaf, r) in &placed {
        let (id, rect) = (base.with(leaf.id), r / ppp);
        let ink = leaf.ink.color(&cx.theme.colors, channel);
        let bound = leaf.param.is_some_and(|p| p < module.params.len());
        let pspec = leaf.param.and_then(|p| spec?.params.get(p));
        let kind = pspec.map(|s| s.kind);
        let menu = kind.map_or(&[][..], ParamKind::items);
        let items = if bound { menu } else { leaf.items };
        let label = leaf.label_for(&module.params);
        let enabled = leaf.enabled.holds(module, state);
        // Labels are FontSize 1, but text toggles and right labels FontSize 2.
        let label_font = match leaf.widget {
            Widget::TextToggle | Widget::ToggleRight => FontSize::Pt10,
            _ => FontSize::Pt8,
        };
        // A DW Sampler's program resets its Attack and Release to the values it set.
        let program = leaf.param.and_then(|p| module.program_reset(p));
        let reset = program.unwrap_or_else(|| reset_of(leaf, pspec, module.type_id));
        let c = Control {
            label_font: leaf.font.unwrap_or(label_font),
            steps: (!menu.is_empty()).then_some(menu.len()),
            item: match kind {
                Some(ParamKind::MenuBy { index, .. }) => Some(index),
                _ => None,
            },
            value_text: leaf.value_text,
            disabled: !enabled,
            flat: (!leaf.lit.holds(module, state))
                .then(|| cx.theme.colors.module_fill.to_color32()),
            bipolar: is_bipolar(leaf, pspec),
            column_len: leaf.column_len,
            disabled_items: leaf.disabled_items,
            menu: match leaf.param.filter(|_| bound) {
                Some(p) => ParamMenu::bound(links::has(&linked, module.instance_id, p as i32)),
                None => ParamMenu::Unbound,
            },
            fine_tune: bound && pspec.is_some_and(fine_tune::module_param),
            ..Control::new(id, rect, &label, ink, reset)
        };
        let text = |v: f32| match (leaf.value_fn, pspec) {
            (Some(f), _) => f(v),
            (None, Some(s)) => s.display_text_in(v, text_cx),
            (None, None) => String::new(),
        };
        // A macro, not a closure: the borrow must end with each use.
        macro_rules! value {
            () => {
                slot(leaf, &mut module.params, &mut state.settings)
            };
        }
        let edit = match leaf.widget {
            Widget::Knob => widgets::knob(ui, cx, &c, value!(), text).changed(),
            Widget::Arc {
                start,
                span,
                mirrored,
                track,
            } => {
                let track = track.color(&cx.theme.colors, ink);
                let arc = ArcSlider {
                    start,
                    span,
                    mirrored,
                    track,
                };
                widgets::arc_slider(ui, cx, &c, &arc, value!(), text).changed()
            }
            Widget::GraphKnob { falling } => {
                widgets::graph_knob(ui, cx, &c, value!(), falling).changed()
            }
            Widget::VSlider => widgets::v_slider(ui, cx, &c, value!(), text).changed(),
            Widget::Tag { faces_right } => {
                widgets::tag_slider(ui, cx, &c, value!(), faces_right, text).changed()
            }
            Widget::HSlider => widgets::h_slider(ui, cx, &c, value!(), text).changed(),
            Widget::Toggle => widgets::toggle(ui, cx, &c, value!(), text).changed(),
            Widget::ToggleRight => {
                let pill = Pill {
                    label_right: true,
                    ..Pill::default()
                };
                widgets::pill_toggle(ui, cx, &c, pill, value!(), text).changed()
            }
            // A list, spinner or menu button edits its menu param's normalised value, or its setting's item index.
            Widget::Spinner if leaf.picker => {
                let first = items.first().copied().unwrap_or_default();
                let item = state.texts.get(&leaf.id).map_or(first, String::as_str);
                widgets::spinner_button(ui, cx, &c, item).clicked()
            }
            Widget::Spinner => match kind.filter(|_| bound) {
                Some(kind) => index_param(value!(), kind, |i| {
                    widgets::spinner_index(ui, cx, &c, items, i)
                }),
                None if bound => widgets::spinner(ui, cx, &c, items, value!()),
                None => index_setting(value!(), |i| widgets::spinner_index(ui, cx, &c, items, i)),
            }
            .changed(),
            // A menu param shows and writes the item its index law reads ([`ParamKind::item`]).
            Widget::MenuButton => {
                let pick = |i: &mut usize| {
                    let n = items.len();
                    let mut v = menu_value(*i, n);
                    let response = widgets::menu_button(ui, cx, &c, items, &mut v);
                    *i = menu_index(v, n);
                    response
                };
                match kind.filter(|_| bound) {
                    Some(kind) => index_param(value!(), kind, pick),
                    None if bound => widgets::menu_button(ui, cx, &c, items, value!()),
                    None => index_setting(value!(), pick),
                }
                .changed()
            }
            Widget::List(bar) => {
                let names = item_names(&label);
                let list = ItemList::new(id, rect, &names, bar, ink);
                let pick = |i: &mut usize| widgets::item_list_index(ui, cx, &list, i);
                match kind.filter(|_| bound) {
                    Some(kind) => index_param(value!(), kind, pick),
                    None if bound => widgets::item_list(ui, cx, &list, value!()),
                    None => index_setting(value!(), pick),
                }
                .changed()
            }
            Widget::Button { .. } => {
                let (b, value) = (button_look(leaf, &c, cx, enabled), value!());
                let response = widgets::button(ui, cx, &b, widgets::toggle_on(*value));
                press(ui, &response, leaf.momentary, value)
            }
            Widget::TextToggle if leaf.momentary => {
                let value = value!();
                let response = widgets::text_button(ui, cx, &c);
                press(ui, &response, true, value)
            }
            Widget::TextToggle => widgets::text_toggle(ui, cx, &c, value!()).changed(),
            Widget::Tabs => {
                let names = item_names(&label);
                let list = ItemList::new(id, rect, &names, Bar::Bottom, ink);
                widgets::item_list_index(ui, cx, &list, &mut state.tab).changed()
            }
            Widget::Text => {
                let font = leaf.font.unwrap_or(FontSize::Pt8);
                let text = state.texts.get(&leaf.id).map_or(&*label, String::as_str);
                widgets::text_label(ui, cx, rect, text, font, leaf.align, ink);
                false
            }
            Widget::Display => match leaf.draw {
                // Edited when it changed a param the module has, or the page.
                Some(draw) => {
                    let tab = state.tab;
                    let (mut edit, clip) = (false, ui.clip_rect());
                    let changed = draw(ui, cx, id, rect, module, state, ink);
                    // The card clip for the controls after it, whatever the draw did to it.
                    ui.set_clip_rect(clip);
                    for p in changed {
                        if p < module.params.len() {
                            add(&mut edits.params, p);
                            edit = true;
                        }
                    }
                    edit || state.tab != tab
                }
                None => {
                    widgets::display_area(ui, cx, rect);
                    false
                }
            },
            Widget::Spacer => false,
            Widget::Separator => {
                widgets::separator(ui, cx, rect);
                false
            }
            Widget::XyPad => {
                let pair = [leaf.param, leaf.y_param].map(|p| p.unwrap_or(usize::MAX));
                match module.params.get_disjoint_mut(pair) {
                    Ok([x, y]) => widgets::xy_pad(ui, cx, &c, x, y).changed(),
                    Err(_) => {
                        widgets::display_area(ui, cx, rect);
                        false
                    }
                }
            }
        };
        if let Some(p) = leaf.param.filter(|_| bound) {
            let signal = widgets::signal(ui, id);
            if signal.pressed {
                edits.pressed = Some(p);
            }
            if let Some(pick) = signal.pick {
                edits.pick = Some((p, pick));
            }
        }
        if edit {
            edited.push(leaf.id);
            if !bound {
                edits.settings.push(leaf.id);
            }
            let params = [leaf.param, leaf.y_param].into_iter().flatten();
            for p in params.filter(|&p| p < module.params.len()) {
                add(&mut edits.params, p);
            }
        }
    }
    if let Some(overlay) = panel.overlay {
        overlay(ui, cx, module, state, &drawn, content, channel);
    }
    let mut hooked = false;
    if let Some(hook) = panel.hook {
        let settings = state.settings.clone();
        for p in hook(module, state, &edited) {
            if p < module.params.len() {
                add(&mut edits.params, p);
                hooked = true;
            }
        }
        hooked |= state.settings != settings;
    }
    if !edited.is_empty() || hooked {
        // A change that shows or hides parts re-lays out the module. egui runs the frame again (without the
        // input, which is spent), so the new layout is what this frame shows.
        if shown(&place(panel, module, state, rect_px, u_px)) != before {
            ui.ctx()
                .request_discard("a panel's visible controls changed");
        }
        // In case egui declines the extra pass, and for the controls drawn before their edit (buttons): the next
        // frame shows it.
        ui.ctx().request_repaint();
    }
    ui.set_clip_rect(outer_clip);
    edits
}

/// Appends `p` unless it is there already.
fn add(changed: &mut Vec<usize>, p: usize) {
    if !changed.contains(&p) {
        changed.push(p);
    }
}

/// Runs an index widget on a menu param's normalised value: on the item its kind reads ([`ParamKind::item`], so
/// Wow & Flutter's `trunc(9v)`), written back as `menu_value(index, n)` only when the user picked another.
fn index_param(
    value: &mut f32,
    kind: ParamKind,
    widget: impl FnOnce(&mut usize) -> Response,
) -> Response {
    let mut index = kind.item(*value);
    let response = widget(&mut index);
    if response.changed() {
        *value = menu_value(index, kind.items().len());
    }
    response
}

/// Runs an index widget on a setting that holds an item index (see [`PanelState::settings`]), writing it back only
/// when the user picked another item.
fn index_setting(value: &mut f32, widget: impl FnOnce(&mut usize) -> Response) -> Response {
    let mut index = setting_item(*value);
    let response = widget(&mut index);
    if response.changed() {
        *value = index as f32;
    }
    response
}

/// The button a button leaf is drawn as: the module's colours, shape, icon and font on [`Button::new`]'s, the
/// parameter menu when it is bound (`c.menu`), and unless `enabled` no input and every colour at half alpha.
fn button_look<'a>(leaf: &Leaf, c: &Control<'a>, cx: &UiCtx, enabled: bool) -> Button<'a> {
    let colors = &cx.theme.colors;
    let round = matches!(leaf.widget, Widget::Button { round: true });
    // The button's own halo colour, else the channel colour.
    let halo = leaf
        .halo_ink
        .map_or(c.channel, |i| i.color(colors, c.channel));
    let mut b = Button {
        shape: leaf.shape,
        icon: leaf.icon,
        menu: c.menu,
        halo: leaf.halo.map(|pad| (halo, pad.px(cx.m))),
        ..Button::new(c.id, c.rect, c.label, round, colors)
    };
    if let Some(set) = leaf.colors {
        let ink = |ink: Ink| ink.color(colors, c.channel);
        (b.fill_on, b.fill_off, b.text_on) =
            (ink(set.fill_on), ink(set.fill_off), ink(set.text_on));
        b.text_off = set.text_off.map(ink);
    }
    if let Some(font) = leaf.font {
        b.font = font;
    }
    if !enabled {
        let half = |c: Color32| c.gamma_multiply(0.5);
        (b.fill_on, b.fill_off, b.text_on) = (half(b.fill_on), half(b.fill_off), half(b.text_on));
        b.text_off = b.text_off.map(half);
        b.halo = b.halo.map(|(color, pad)| (half(color), pad));
        b.disabled = true;
    }
    b
}

/// Applies a button's `response` to `value` (on for any value but 0, [`widgets::toggle_on`]): a click writes 1 from
/// exactly 0 and 0 from anything else, or for a `momentary` button it is 1 while a live primary press holds it down
/// ([`primary_down_live`]: not one held over a focus loss) and 0 otherwise. A tap whose press and release land in one
/// frame is 1 in that frame; the repaint [`show`] asks for brings the 0. Returns whether it changed.
fn press(ui: &Ui, response: &Response, momentary: bool, value: &mut f32) -> bool {
    let on = widgets::toggle_on(*value);
    let next = if momentary {
        // Every frame, before the `&&`: the latch must see each frame.
        let live = primary_down_live(ui, response.id);
        let tap = response.clicked() && ui.input(|i| i.pointer.primary_pressed());
        let held = (response.is_pointer_button_down_on() && live) || tap;
        (held != on).then_some(held)
    } else {
        response.clicked().then_some(!on)
    };
    match next {
        Some(next) => {
            *value = f32::from(u8::from(next));
            true
        }
        None => false,
    }
}

/// `rect` (points) in device pixels, moved so its origin is a whole pixel: the layout truncates relative to it,
/// so the controls land on whole pixels. `None` when it is not finite.
fn content_px(rect: Rect, ppp: f32) -> Option<Rect> {
    let px = rect * ppp;
    px.is_finite()
        .then(|| px.translate(px.min.round() - px.min))
}

/// The value `leaf` edits: its param, or for a control without one (or a param the module lacks) its module
/// setting.
fn slot<'a>(
    leaf: &Leaf,
    params: &'a mut [f32],
    settings: &'a mut HashMap<u32, f32>,
) -> &'a mut f32 {
    match leaf.param.and_then(|p| params.get_mut(p)) {
        Some(v) => v,
        None => settings.entry(leaf.id).or_insert(leaf.default),
    }
}
