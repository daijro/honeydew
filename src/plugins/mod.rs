//! Plugins: Lemon Studio's own, built into the app on every platform ([`builtin`]), and third-party ones (VST3, VST2,
//! CLAP) on the desktop: scanning, an instance per plugin module of the song, the engines' share of it, the plugins'
//! own editors. Built on Lemon Studio's plugin hosts (`vendor/lemon-studio`).
//!
//! On the web and Android only the built-in plugins are hosted: a song's other plugin modules are kept as they are
//! (their state saved back untouched) and play nothing.
//!
//! Windows plugins on Linux: yabridge and its fork vstbridge wrap them as native Linux VST2 and VST3 plugins in
//! `~/.vst/yabridge` and `~/.vst3/yabridge`, which the default search folders include ([`scan::default_folders`]), so
//! they load like any Linux plugin.

pub mod builtin;
mod engine;
pub(crate) use engine::{begin_callback, lock_stats};
mod host;
#[cfg(plugin_host)]
pub mod scan;
mod window;
pub use host::*;

/// The settings store's key for the scan results.
pub const CACHE_KEY: &str = "honeydew.pluginCache";

/// What a plugin is for.
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord)]
pub enum PluginKind {
    Instrument,
    Effect,
}

/// The editor and processing facts a rack card shows.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct Info {
    /// The plugin has an editor, and this platform has editor windows (Linux only for now).
    pub has_editor: bool,
    pub editor_open: bool,
    /// Processing latency in frames; the engine has no delay compensation, so it is shown, not compensated.
    pub latency: u32,
    /// The plugin crashed or stopped itself: it plays silence (an instrument) or passes the sound (an effect).
    pub stopped: Option<String>,
    /// Its editor could not be opened, and why: the card offers it no more.
    pub editor_failed: Option<String>,
}

/// A rack module's plugin as the app thread sees it.
#[derive(Clone, Debug, PartialEq)]
pub enum State {
    Live(Info),
    /// Not installed here, or it failed to start: the module keeps its state and saves it back.
    Missing(String),
}

/// An installed plugin the Modules browser offers.
#[derive(Clone, Debug, PartialEq)]
pub struct Available {
    /// `format:id`.
    pub uid: String,
    pub name: String,
    pub vendor: String,
    /// "CLAP", "VST3" or "VST2".
    pub format: &'static str,
    pub kind: PluginKind,
}

/// A param change a plugin made itself (its editor), for the song: module param `param` (≥ 1) of the module with
/// instance id `instance_id` on the channel with id `channel_id`. `gesture`: 0 a press began, 1 a value, 2 the press
/// ended.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct PluginMove {
    pub channel_id: i32,
    pub instance_id: i32,
    pub param: i32,
    pub value: f32,
    pub gesture: u8,
}
