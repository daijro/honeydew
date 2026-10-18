//! No plugin hosting (the web, Android): the same calls as the desktop's `host`, hosting nothing.

use super::{Available, PluginMove, State};
use crate::song::{Channel, ModuleInstance, Rack, Song};

pub fn start(_: &eframe::egui::Context, _: Option<String>, _: &[String]) {}

pub fn rescan_folders(_: &[String], _: bool) {}

pub fn poll_scan() -> bool {
    false
}

pub fn scanning() -> bool {
    false
}

pub fn scan_counts() -> (usize, usize) {
    (0, 0)
}

pub fn cache_json() -> String {
    String::new()
}

pub fn installed(_: &crate::song::plugin::PluginRef) -> bool {
    false
}

pub fn available() -> Vec<Available> {
    Vec::new()
}

thread_local! {
    /// The plugin modules met (channel id, instance id), and the notices for them not taken yet.
    static SEEN: std::cell::RefCell<(std::collections::HashSet<(i32, i32)>, Vec<String>)> = Default::default();
}

/// Each plugin module met for the first time gets a notice: it plays nothing here and its settings are kept.
pub fn sync(song: &Song) {
    SEEN.with_borrow_mut(|(seen, notices)| {
        for channel in &song.channels {
            let plugins = channel
                .rack
                .modules
                .iter()
                .filter(|m| crate::song::plugin::is_plugin(m.type_id));
            for module in plugins {
                if seen.insert((channel.id, module.instance_id))
                    && let Some(p) = &module.file.plugin
                {
                    let line = format!(
                        "{} ({}) runs in the desktop app only; its settings are kept.",
                        p.name,
                        p.format_label()
                    );
                    notices.push(line);
                }
            }
        }
    });
}

pub fn adopt_params(_: &mut Song) -> bool {
    false
}

pub fn song_replaced() {
    SEEN.with_borrow_mut(|(seen, notices)| {
        seen.clear();
        notices.clear();
    });
}

pub fn take_notices() -> Vec<String> {
    let mut lines = SEEN.with_borrow_mut(|(_, notices)| std::mem::take(notices));
    lines.dedup();
    lines
}

pub fn idle() -> Vec<PluginMove> {
    Vec::new()
}

pub fn busy() -> bool {
    false
}

pub fn store_states(_: &mut Song) {}

pub fn set_rate(_: u32) {}

pub fn state(_: i32, _: i32) -> Option<State> {
    Some(State::Missing(
        "Plugins run in the desktop app only; this one's settings are kept.".into(),
    ))
}

pub fn param_text(_: i32, _: i32, _: usize, _: f32) -> Option<String> {
    None
}

pub fn show_editor(_: i32, _: i32, _: bool) -> Result<(), String> {
    Err("Plugins run in the desktop app only".into())
}

pub fn add(_: &mut Rack, _: i32, uid: &str, _: usize) -> Result<i32, String> {
    Err(format!("{uid}: plugins run in the desktop app only"))
}

pub fn instantiate(_: i32, _: i32, uid: &str) -> Result<ModuleInstance, String> {
    Err(format!("{uid}: plugins run in the desktop app only"))
}

pub fn rendering<T>(f: impl FnOnce() -> T) -> T {
    f()
}

pub fn instrument(
    _: &Channel,
    _: &ModuleInstance,
    _: f32,
    _: u32,
    _: usize,
) -> Option<Box<dyn honeydew_audio::instrument::Instrument>> {
    None
}

pub fn effect(
    _: &Channel,
    _: &ModuleInstance,
    _: f32,
    _: u32,
) -> Option<Box<dyn honeydew_audio::ExternalEffect>> {
    None
}
