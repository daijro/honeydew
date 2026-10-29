//! Parameter registry: names, defaults and value text of the rack modules (effects and instruments), the drum pad
//! slots and the channel strip.
//!
//! Pure data and formatting; no UI. Parameter values are normalised `f32` in [0, 1] and the indices are
//! The stored param numbers (param 0 is always `Enable`), shared with the audio side.

mod channel;
mod display;
mod dynamic_eq;
mod effects;
mod effects_24_51;
#[cfg(test)]
mod inst_rows;
#[cfg(test)]
mod inst_tests;
mod instruments;
mod lemon;
#[cfg(test)]
mod spec_rows;
#[cfg(test)]
mod step_tests;
#[cfg(test)]
mod tests;

pub use channel::{CHANNEL_PARAMS, COMMON_PARAMS, DRUM_SLOT_PARAMS};
pub(crate) use display::clamp01;
pub use display::{At, Display, Fmt, Sign, TextCx, menu_index, menu_value};
/// The Dynamic EQ, Honeydew's own effect (`honeydew_audio::dynamic_eq`).
pub use dynamic_eq::MODULE as DYNAMIC_EQ;
pub(crate) use effects_24_51::ARP_RHYTHM_PATTERNS;
pub use effects_24_51::snap_echo_delay;
pub(crate) use effects_24_51::{arp_pattern, arp_steps};
pub(crate) use instruments::GMSYNTH_WAVESHAPES;
/// Lemon Studio's reverb and limiter, Honeydew's own effects (`honeydew_audio::lemon`).
pub use lemon::MODULES as LEMON_MODULES;

/// What a module does with the signal.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Role {
    /// Audio effect.
    Effect,
    /// Note effect: transforms note events, sits before the instrument.
    NoteEffect,
    /// Instrument (generator): first in the rack.
    Instrument,
}

// Only the tests compare kinds; a `MenuBy` index is compared by address, which is good enough there.
#[allow(unpredictable_function_pointer_comparisons)]
#[derive(Clone, Copy, Debug, PartialEq)]
pub enum ParamKind {
    /// A knob or slider; the text comes from the [`Display`] law.
    Continuous { display: Display },
    /// A spinner or list; item index = `round(v·(n − 1))`, halves away from zero ([`menu_index`]).
    Menu { items: &'static [&'static str] },
    /// A menu whose item is `index(v)` for the clamped `v` instead: the item its audio picks (Wow & Flutter's
    /// synced rates: `trunc(9v)`). Picking item `k` still writes
    /// [`menu_value`]`(k, n)`, which `index` must read back as `k`.
    MenuBy {
        items: &'static [&'static str],
        index: fn(f32) -> usize,
    },
    /// Off (below 0.5) / On.
    Toggle,
}

impl ParamKind {
    /// The value text for normalised `v` in context `cx`.
    pub fn text_in(self, v: f32, cx: TextCx) -> String {
        match self {
            Self::Continuous { display } => display.text_in(v, cx),
            Self::Menu { items } | Self::MenuBy { items, .. } => items[self.item(v)].to_owned(),
            Self::Toggle => if v >= 0.5 { "On" } else { "Off" }.to_owned(),
        }
    }

    /// A menu's items; empty for anything else.
    pub fn items(self) -> &'static [&'static str] {
        match self {
            Self::Menu { items } | Self::MenuBy { items, .. } => items,
            _ => &[],
        }
    }

    /// The item a menu shows at normalised `v`; 0 for anything else.
    pub fn item(self, v: f32) -> usize {
        match self {
            Self::Menu { items } => menu_index(v, items.len()),
            Self::MenuBy { items, index } => {
                index(display::clamp01(v)).min(items.len().saturating_sub(1))
            }
            _ => 0,
        }
    }
}

#[derive(Clone, Copy, Debug, PartialEq)]
pub struct ParamSpec {
    /// Automation name; empty for params without one.
    pub name: &'static str,
    pub default: f32,
    pub kind: ParamKind,
    /// The value the control is created with besides the default: the
    /// default, except for a few MiniSynth and 3x Osc controls. The knob ring and slider fill
    /// grow away from it.
    pub reset: f32,
    /// The automation lane's value texts where they differ from the control's: `kind` holds the UI
    /// labels the panels show, this the automation texts (MiniSynth `OSC Kind` "SQR+SAW" for the UI's
    /// "SAW + SQUARE"; SuperSaw's mode menu reads "0…100"). `None`: the same as [`ParamSpec::display_text`].
    pub automation: Option<ParamKind>,
    /// Recorded automation snaps to trunc(n·v)/n (the Gate's Asymmetry: its 66 steps only snap recorded
    /// automation, to trunc(65v)/65); the control itself stays continuous. `None`: the kind's own steps, if any.
    pub record_snap: Option<u16>,
}

impl ParamSpec {
    /// Parameter 0 of every module.
    pub const ENABLE: Self = Self::toggle("Enable", 1.0);

    pub const fn toggle(name: &'static str, default: f32) -> Self {
        Self {
            name,
            default,
            kind: ParamKind::Toggle,
            reset: default,
            automation: None,
            record_snap: None,
        }
    }

    pub const fn menu(name: &'static str, default: f32, items: &'static [&'static str]) -> Self {
        Self {
            name,
            default,
            kind: ParamKind::Menu { items },
            reset: default,
            automation: None,
            record_snap: None,
        }
    }

    /// A menu whose item is `index(v)` ([`ParamKind::MenuBy`]).
    pub const fn menu_by(
        name: &'static str,
        default: f32,
        items: &'static [&'static str],
        index: fn(f32) -> usize,
    ) -> Self {
        Self {
            kind: ParamKind::MenuBy { items, index },
            ..Self::menu(name, default, items)
        }
    }

    pub const fn cont(name: &'static str, default: f32, display: Display) -> Self {
        Self {
            name,
            default,
            kind: ParamKind::Continuous { display },
            reset: default,
            automation: None,
            record_snap: None,
        }
    }

    /// The generic display: `v × 100` truncated, no unit.
    pub const fn plain(name: &'static str, default: f32) -> Self {
        Self::cont(name, default, Display::Generic)
    }

    /// The same param with reset value `reset` ([`ParamSpec::reset`]).
    pub const fn with_reset(self, reset: f32) -> Self {
        Self { reset, ..self }
    }

    /// The same param with automation texts `kind` ([`ParamSpec::automation`]).
    pub const fn with_automation(self, kind: ParamKind) -> Self {
        Self {
            automation: Some(kind),
            ..self
        }
    }

    /// The same param with recorded automation snapped to trunc(n·v)/n ([`ParamSpec::record_snap`]).
    pub const fn with_record_snap(self, n: u16) -> Self {
        Self {
            record_snap: Some(n),
            ..self
        }
    }

    /// Rests at the middle: knob rings and slider thumbs grow with |v − 0.5| and fills start at the centre. That is a
    /// reset value of exactly 0.5, whatever the range.
    pub fn bipolar(&self) -> bool {
        self.reset == 0.5
    }

    /// The value text the control shows for normalised `v`, in the default [`TextCx`] (no module, 0 BPM, no audio).
    pub fn display_text(&self, v: f32) -> String {
        self.display_text_in(v, TextCx::default())
    }

    /// The value text the control shows for normalised `v` in context `cx`.
    pub fn display_text_in(&self, v: f32, cx: TextCx) -> String {
        self.kind.text_in(v, cx)
    }

    /// The value text of an automation lane for normalised `v` ([`ParamSpec::automation`]), in the default
    /// [`TextCx`].
    pub fn automation_text(&self, v: f32) -> String {
        self.automation_text_in(v, TextCx::default())
    }

    /// The value text of an automation lane for normalised `v` in context `cx`: the target module's params, the song
    /// tempo and the sample rate, as the panels pass them. The automation editor's value axis, point labels and typed
    /// values read it (`ui::automation::value`).
    pub fn automation_text_in(&self, v: f32, cx: TextCx) -> String {
        match self.automation {
            // A menu's automation texts name the same items: the control's index law picks one.
            Some(ParamKind::Menu { items }) if items.len() == self.kind.items().len() => {
                items[self.kind.item(v)].to_owned()
            }
            automation => automation.unwrap_or(self.kind).text_in(v, cx),
        }
    }
}

#[derive(Clone, Copy, Debug, PartialEq)]
pub struct ModuleSpec {
    /// Type id stored in presets and projects.
    pub id: u16,
    /// Title shown on the module.
    pub display_name: &'static str,
    /// Internal/automation name.
    pub unique_name: &'static str,
    pub role: Role,
    /// Kept so old projects keep loading; hidden from the add-effect list.
    pub legacy: bool,
    /// Indexed by param number; `params[0]` is `Enable`.
    pub params: &'static [ParamSpec],
    /// Tab strip labels of the overview "Pages" column. For MultiFX and Trance Delay (parameter 1) and
    /// Tuned EQ (parameter 9) the strip is bound to that parameter and the labels equal its menu items.
    pub tabs: &'static [&'static str],
}

impl ModuleSpec {
    pub const fn effect(
        id: u16,
        display_name: &'static str,
        unique_name: &'static str,
        legacy: bool,
        params: &'static [ParamSpec],
        tabs: &'static [&'static str],
    ) -> Self {
        Self {
            id,
            display_name,
            unique_name,
            role: Role::Effect,
            legacy,
            params,
            tabs,
        }
    }

    pub const fn note_effect(
        id: u16,
        display_name: &'static str,
        unique_name: &'static str,
        params: &'static [ParamSpec],
        tabs: &'static [&'static str],
    ) -> Self {
        Self {
            role: Role::NoteEffect,
            ..Self::effect(id, display_name, unique_name, false, params, tabs)
        }
    }

    pub const fn instrument(
        id: u16,
        display_name: &'static str,
        unique_name: &'static str,
        params: &'static [ParamSpec],
        tabs: &'static [&'static str],
    ) -> Self {
        Self {
            role: Role::Instrument,
            ..Self::effect(id, display_name, unique_name, false, params, tabs)
        }
    }
}

/// Every registered effect and note effect in id order (the two data tables are consecutive id ranges).
pub fn modules() -> impl Iterator<Item = &'static ModuleSpec> {
    effects::MODULES.iter().chain(effects_24_51::MODULES)
}

/// Every registered instrument in id order.
pub fn instruments() -> impl Iterator<Item = &'static ModuleSpec> {
    instruments::MODULES.iter()
}

/// The module with type id `id` (effect, note effect, instrument, a plugin module, one of Lemon Studio's effects or the
/// Dynamic EQ), or `None` for ids that are not registered.
pub fn module_spec(id: u16) -> Option<&'static ModuleSpec> {
    modules()
        .chain(instruments())
        .chain(&PLUGINS)
        .chain(&LEMON_MODULES)
        .chain([&DYNAMIC_EQ])
        .find(|m| m.id == id)
}

/// The plugin module types ([`crate::song::plugin`]): Enable alone here; a plugin's own params follow it, named by the
/// plugin ([`crate::song::ModuleInstance::param_spec`]). Not in [`modules`] or [`instruments`]: no panel, no preset.
static PLUGINS: [ModuleSpec; 2] = [
    ModuleSpec::instrument(
        crate::song::plugin::PLUGIN_INSTRUMENT,
        "Plugin",
        "Plugin",
        &[ParamSpec::ENABLE],
        &[],
    ),
    ModuleSpec::effect(
        crate::song::plugin::PLUGIN_EFFECT,
        "Plugin",
        "Plugin FX",
        false,
        &[ParamSpec::ENABLE],
        &[],
    ),
];
