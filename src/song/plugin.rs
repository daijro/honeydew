//! Third-party plugin modules (VST3, VST2, CLAP): which plugin a rack module hosts and its saved state. Pure data on
//! every platform, so a song keeps its plugins where none can be hosted (the web, Android, a machine without the
//! plugin) and saves them back unchanged. The hosting is `crate::plugins` (desktop only).
//!
//! A plugin module is a [`super::ModuleInstance`] of type [`PLUGIN_INSTRUMENT`] or [`PLUGIN_EFFECT`]: `params[0]` is
//! Enable, as on every module, and `params[1 + i]` is the plugin's parameter `i` (normalized), what automation targets.
//! The `.flm` writer keeps these modules out of the rack's `RMOd`s, in `HDPm` chunks (`flm::write`).

use std::sync::Arc;

/// The module type of an instrument plugin, Honeydew's own: the file keeps it in `HDPm`.
pub const PLUGIN_INSTRUMENT: u16 = 1000;
/// The module type of an effect plugin.
pub const PLUGIN_EFFECT: u16 = 1001;

/// Whether module type `type_id` hosts a plugin.
pub fn is_plugin(type_id: u16) -> bool {
    matches!(type_id, PLUGIN_INSTRUMENT | PLUGIN_EFFECT)
}

/// Whether module type `type_id` is Honeydew's own, outside the song format's module types: a plugin module, one of
/// Lemon Studio's effects (`honeydew_audio::lemon`) or the Dynamic EQ. The `.flm` writer keeps these in `HDPm` chunks.
pub fn is_own(type_id: u16) -> bool {
    type_id >= PLUGIN_INSTRUMENT
}

/// `s` as a `&'static str`, for the registry types a plugin's names fill ([`crate::params::ParamSpec`]).
// ponytail: interned for good, bounded by the distinct plugin and parameter names a session meets.
pub(crate) fn intern(s: &str) -> &'static str {
    use std::collections::HashSet;
    use std::sync::Mutex;
    static NAMES: Mutex<Option<HashSet<&'static str>>> = Mutex::new(None);
    let mut names = NAMES.lock().unwrap_or_else(|e| e.into_inner());
    let names = names.get_or_insert_with(HashSet::new);
    if let Some(&name) = names.get(s) {
        return name;
    }
    let name: &'static str = Box::leak(s.into());
    names.insert(name);
    name
}

/// The plugin a module hosts and its state, as the project keeps them.
#[derive(Clone, Debug, Default)]
pub struct PluginRef {
    /// "clap", "vst3" or "vst2".
    pub format: String,
    /// The format's id: the CLAP plugin id, the VST3 class id (32 hex digits), the VST2 unique id (8 hex digits).
    pub id: String,
    pub name: String,
    pub vendor: String,
    /// The file it was loaded from, as a hint: a plugin is found by format and id, then by name.
    pub path: String,
    /// The plugin's own state (CLAP state stream, VST3 component and controller states, VST2 chunk), as the module was
    /// added or read; a save writes the running plugin's (`plugins::store_states`). Not an edit: the plugin keeps its
    /// state itself, so two refs that differ only here compare equal (no undo step, no unsaved change, no shared
    /// change). Shared, so that song copies (undo steps) do not copy it.
    pub state: Arc<[u8]>,
    /// The parameters as the plugin last listed them: module param `1 + i` is `params[i]`.
    pub params: Vec<PluginParam>,
}

/// One plugin parameter: its stable id, its name and its default (normalized).
#[derive(Clone, Debug, PartialEq, Default)]
pub struct PluginParam {
    pub id: u32,
    pub name: String,
    pub default: f32,
}

impl PartialEq for PluginRef {
    /// All but the state ([`Self::state`]).
    fn eq(&self, other: &Self) -> bool {
        let Self {
            format,
            id,
            name,
            vendor,
            path,
            state: _,
            params,
        } = self;
        (format, id, name, vendor, path, params)
            == (
                &other.format,
                &other.id,
                &other.name,
                &other.vendor,
                &other.path,
                &other.params,
            )
    }
}

impl PluginRef {
    /// `format:id`, the identity plugin lists use.
    pub fn uid(&self) -> String {
        format!("{}:{}", self.format, self.id)
    }

    /// The format's display name.
    pub fn format_label(&self) -> &'static str {
        match self.format.as_str() {
            "clap" => "CLAP",
            "vst3" => "VST3",
            "vst2" => "VST2",
            _ => "Plugin",
        }
    }
}
