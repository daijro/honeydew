//! Issue #168: the files the built-in Lemon plugins open by their paths (their samples), read and relinked through
//! their states as the host keeps them ([`PluginRef::state`]: nih-plug's JSON, after an 8-byte length or not). Only
//! the plugins' own fields for the file to open are written, never the saved state inside them: `sample-path` (Lemon
//! Sampler, Lemon Trio's custom shape, Lemon Slicer) and `sample-paths` (Lemon Drum Sampler, one per layer). An empty
//! one opens the path in that saved state (Lemon Slicer's loop, Lemon Drum Sampler's layers), which is read for it.

use serde_json::Value;

use super::builtin;
use crate::song::Song;
use crate::song::plugin::PluginRef;

const SAMPLE: &str = "sample-path";
const SAMPLES: &str = "sample-paths";
const SLICER: &str = "org.openstudio.lemon-slicer";
const DRUMS: &str = "org.openstudio.lemon-drum-sampler";

/// Each file a built-in plugin of `song` opens goes to `map`, and the plugin opens the path `map` gives for it instead,
/// where it gives one.
pub fn relink(song: &mut Song, map: &mut dyn FnMut(&str) -> Option<String>) {
    let modules = song.channels.iter_mut().flat_map(|c| &mut c.rack.modules);
    for plugin in modules.filter_map(|m| m.file.plugin.as_deref_mut()) {
        relink_one(plugin, map);
    }
}

fn relink_one(plugin: &mut PluginRef, map: &mut dyn FnMut(&str) -> Option<String>) -> Option<()> {
    let id = builtin::of(plugin)?.clap_id();
    // As `os_hostlink::state::json` tells the length from the JSON.
    let (sized, json) = match plugin.state.split_first_chunk::<8>() {
        Some((len, json)) if u64::from_le_bytes(*len) == json.len() as u64 => (true, json),
        _ => (false, &plugin.state[..]),
    };
    let mut state: Value = serde_json::from_slice(json).ok()?;
    let fields = state.get_mut("fields")?.as_object_mut()?;
    let read = |field: &Value| serde_json::from_str::<Value>(field.as_str()?).ok();
    let mut moved = |path: &str| (!path.is_empty()).then(|| map(path)).flatten();
    let mut changed = false;
    if let Some(Value::String(given)) = fields.get(SAMPLE).and_then(read) {
        let stored = || {
            let raw = lemon_slicer::params::fl_from_plugin_state(&plugin.state)?;
            Some(lemon_slicer::state::State::parse(&raw)?.path())
        };
        let path = match given.is_empty() && id == SLICER {
            true => stored().unwrap_or_default(),
            false => given,
        };
        if let Some(new) = moved(&path).filter(|new| *new != path) {
            fields.insert(SAMPLE.into(), Value::String(Value::String(new).to_string()));
            changed = true;
        }
    }
    if id == DRUMS {
        let raw = lemon_drum_sampler::params::fl_from_plugin_state(&plugin.state);
        let stored = raw.map(|raw| lemon_drum_sampler::params::stored_paths(&raw));
        let stored = stored.unwrap_or_default();
        let given = fields.get(SAMPLES).and_then(read);
        let mut given: Vec<String> = given
            .and_then(|g| serde_json::from_value(g).ok())
            .unwrap_or_default();
        given.resize(given.len().max(stored.len()), String::new());
        let mut layers = false;
        for (layer, path) in given.iter_mut().enumerate() {
            let old = match path.is_empty() {
                true => stored.get(layer).cloned().unwrap_or_default(),
                false => path.clone(),
            };
            if let Some(new) = moved(&old).filter(|new| *new != old) {
                (*path, layers) = (new, true);
            }
        }
        if layers {
            let list = serde_json::to_string(&given).ok()?;
            fields.insert(SAMPLES.into(), Value::String(list));
            changed = true;
        }
    }
    if changed {
        let json = serde_json::to_vec(&state).ok()?;
        let len = (json.len() as u64).to_le_bytes();
        plugin.state = [if sized { &len[..] } else { &[] }, &json].concat().into();
    }
    Some(())
}
