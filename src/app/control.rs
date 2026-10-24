//! Opt-in, loopback-only control for local music assistants. Socket work never touches the song:
//! commands are validated and applied on the egui thread as one ordinary undoable edit.

use std::io::{Read, Write};
use std::net::{TcpListener, TcpStream};
use std::sync::mpsc::{self, Receiver, Sender};
use std::time::Duration;

use eframe::egui;
use honeydew_audio::device::AudioCommand;
use serde::Deserialize;
use serde_json::{Value, json};

use super::HoneydewApp;
use crate::params::ParamKind;
use crate::song::{ChannelKind, Event, LaneKind, PPQ, Song, SynthKind};
use crate::ui::views::SongEdit;

const MAX_REQUEST: u64 = 256 * 1024;
const DEFAULT_PORT: u16 = 57831;

struct Request {
    value: Value,
    reply: Sender<Value>,
}

pub(super) struct Bridge {
    requests: Receiver<Request>,
}

impl Bridge {
    pub(super) fn start(ctx: egui::Context) -> Option<Self> {
        // Disabled by default. Both the app and the local MCP adapter need the same secret.
        let token = std::env::var("HONEYDEW_CONTROL_TOKEN").ok()?;
        if token.len() < 24 {
            eprintln!("Honeydew control disabled: HONEYDEW_CONTROL_TOKEN needs at least 24 characters");
            return None;
        }
        let port = std::env::var("HONEYDEW_CONTROL_PORT")
            .ok()
            .and_then(|s| s.parse::<u16>().ok())
            .unwrap_or(DEFAULT_PORT);
        let listener = match TcpListener::bind(("127.0.0.1", port)) {
            Ok(listener) => listener,
            Err(err) => {
                eprintln!("Honeydew control disabled: cannot bind loopback port {port}: {err}");
                return None;
            }
        };
        let (tx, requests) = mpsc::channel();
        std::thread::Builder::new()
            .name("honeydew-control".into())
            .spawn(move || {
                for connection in listener.incoming() {
                    let Ok(mut stream) = connection else { continue };
                    let _ = stream.set_read_timeout(Some(Duration::from_secs(3)));
                    let _ = stream.set_write_timeout(Some(Duration::from_secs(3)));
                    let answer = serve(&mut stream, &token, &tx, &ctx);
                    let _ = serde_json::to_writer(&mut stream, &answer);
                    let _ = stream.flush();
                }
            })
            .ok()?;
        eprintln!("Honeydew control enabled on 127.0.0.1:{port}");
        Some(Self { requests })
    }

    fn drain(&self) -> Vec<Request> {
        self.requests.try_iter().take(32).collect()
    }
}

fn serve(
    stream: &mut TcpStream,
    token: &str,
    tx: &Sender<Request>,
    ctx: &egui::Context,
) -> Value {
    let mut bytes = Vec::new();
    if stream.take(MAX_REQUEST + 1).read_to_end(&mut bytes).is_err() || bytes.len() as u64 > MAX_REQUEST {
        return json!({"error": "request too large or incomplete"});
    }
    let Ok(mut value) = serde_json::from_slice::<Value>(&bytes) else {
        return json!({"error": "invalid JSON"});
    };
    let Some(provided) = value.get("token").and_then(Value::as_str) else {
        return json!({"error": "authentication required"});
    };
    if provided != token {
        return json!({"error": "authentication failed"});
    }
    value.as_object_mut().expect("token was present").remove("token");
    let (reply, receive) = mpsc::channel();
    if tx.send(Request { value, reply }).is_err() {
        return json!({"error": "Honeydew closed"});
    }
    ctx.request_repaint();
    receive.recv_timeout(Duration::from_secs(3)).unwrap_or_else(|_| json!({"error": "Honeydew did not respond"}))
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct Arrangement {
    tempo_bpm: Option<f64>,
    tracks: Vec<TrackPattern>,
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct TrackPattern {
    target: Target,
    /// Zero-based bar in the song.
    start_bar: u32,
    bars: u32,
    notes: Vec<Note>,
}

#[derive(Deserialize)]
#[serde(tag = "kind", rename_all = "snake_case", deny_unknown_fields)]
enum Target {
    Existing { channel_id: i32, replace_clip_id: Option<u32> },
    New { instrument: String, name: Option<String> },
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct Note {
    /// Position from the start of this clip, measured in quarter-note beats.
    beat: f64,
    length_beats: f64,
    /// MIDI key for a pitched instrument; drum pad index for a drum kit.
    key: u8,
    velocity: u8,
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct ParameterEdit {
    index: usize,
    value: f64,
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct SetInstrumentParams {
    channel_id: i32,
    module_id: i32,
    changes: Vec<ParameterEdit>,
}

fn built_in_instrument(type_id: u16) -> Option<&'static str> {
    SynthKind::ALL.iter().copied()
        .find(|s| s.type_id() == type_id)
        .map(SynthKind::label)
}

fn parameter_name(type_id: u16, index: usize, name: &str) -> String {
    if !name.is_empty() {
        return name.to_owned();
    }
    if type_id == SynthKind::TransistorBass.type_id() {
        if let Some(label) = ["Distortion", "Drive HP", "Drive", "Drive Vol.", "PW", "LFO Speed", "LFO to PW"]
            .get(index.saturating_sub(7)).filter(|_| index >= 7) {
            return (*label).to_owned();
        }
    }
    format!("Parameter {index}")
}

/// Validate every change before touching the song, so a bad index/value cannot leave a partial patch.
fn set_instrument_params(song: &mut Song, request: SetInstrumentParams) -> Result<(usize, Vec<(usize, f32)>), String> {
    if request.changes.is_empty() || request.changes.len() > 32 {
        return Err("provide 1–32 parameter changes".into());
    }
    let channel_index = song.channels.iter().position(|ch| ch.id == request.channel_id && ch.id != 0)
        .ok_or_else(|| format!("channel {} is not in the open song", request.channel_id))?;
    let channel = &mut song.channels[channel_index];
    let module = channel.rack.get_mut(request.module_id)
        .ok_or_else(|| format!("module {} is not on channel {}", request.module_id, request.channel_id))?;
    if built_in_instrument(module.type_id).is_none() {
        return Err("module is not a built-in instrument".into());
    }
    let mut seen = std::collections::HashSet::new();
    let mut changes = Vec::with_capacity(request.changes.len());
    for change in request.changes {
        if !seen.insert(change.index) || module.param_spec(change.index).is_none()
            || module.params.get(change.index).is_none()
            || !change.value.is_finite() || !(0.0..=1.0).contains(&change.value) {
            return Err(format!("invalid or duplicate parameter {}: use a unique index and a normalized value from 0 to 1", change.index));
        }
        changes.push((change.index, change.value as f32));
    }
    for &(index, value) in &changes {
        module.params[index] = value;
        module.init_follows(index as i32);
    }
    Ok((channel_index, changes))
}

fn synth(label: &str) -> Option<ChannelKind> {
    if label.eq_ignore_ascii_case("Drums") {
        return Some(ChannelKind::Drums);
    }
    SynthKind::ALL.iter().copied()
        .find(|s| s.label().eq_ignore_ascii_case(label))
        .map(ChannelKind::Synth)
}

/// Mutates only a clone. If any track is invalid, the caller's song stays untouched.
fn arrange(song: &Song, request: Arrangement) -> Result<(Song, Vec<Value>), String> {
    if request.tracks.is_empty() || request.tracks.len() > 16 {
        return Err("provide 1–16 tracks".into());
    }
    let mut result = song.clone();
    if let Some(bpm) = request.tempo_bpm {
        if !bpm.is_finite() || !(1.0..=999.0).contains(&bpm) {
            return Err("tempo_bpm must be 1–999".into());
        }
        result.set_tempo(bpm);
    }
    let bar_beats = f64::from(song.time_sig.bar_beats());
    let mut made = Vec::new();
    let mut event_count = 0usize;
    for pattern in request.tracks {
        if pattern.bars == 0 || pattern.bars > 16 || pattern.start_bar > 4095 {
            return Err("bars must be 1–16 and start_bar at most 4095".into());
        }
        if pattern.notes.len() > 512 {
            return Err("at most 512 notes per track".into());
        }
        event_count += pattern.notes.len();
        if event_count > 2048 {
            return Err("at most 2048 notes per arrangement".into());
        }
        let (index, replace) = match pattern.target {
            Target::Existing { channel_id, replace_clip_id } => {
                let index = result.channels.iter().position(|ch| ch.id == channel_id)
                    .ok_or_else(|| format!("channel {channel_id} is not in the open song"))?;
                (index, replace_clip_id)
            }
            Target::New { instrument, name } => {
                let kind = synth(&instrument)
                    .ok_or_else(|| format!("unknown built-in instrument: {instrument}"))?;
                let index = result.insert_channel(result.channels.len(), kind)
                    .ok_or("channel limit reached")?;
                if let Some(name) = name {
                    if name.is_empty() || name.chars().count() > 80 {
                        return Err("track name must be 1–80 characters".into());
                    }
                    result.channels[index].name = name;
                }
                (index, None)
            }
        };
        let channel = &mut result.channels[index];
        let lane = channel.lanes.first_mut().ok_or("channel has no lane")?;
        if !matches!(lane.kind, LaneKind::Notes | LaneKind::Drums) {
            return Err(format!("channel {} is not a note or drum track", channel.id));
        }
        let start = f64::from(pattern.start_bar) * bar_beats * f64::from(PPQ);
        let start_tick = u32::try_from(start as u64).map_err(|_| "clip start out of range")?;
        let length = f64::from(pattern.bars) * bar_beats;
        let max_ticks = length * f64::from(PPQ);
        let mut events = Vec::with_capacity(pattern.notes.len());
        for note in pattern.notes {
            if !note.beat.is_finite() || !note.length_beats.is_finite()
                || note.beat < 0.0 || note.length_beats <= 0.0
                || (note.beat + note.length_beats) * f64::from(PPQ) > max_ticks
                || note.velocity == 0
            {
                return Err("note beat/length must fit in the clip and velocity must be 1–255".into());
            }
            let pos = (note.beat * f64::from(PPQ)).round() as u32;
            let event = if lane.kind == LaneKind::Drums {
                if channel.drum_kit.as_ref().is_none_or(|kit| kit.pad_note(note.key as usize).is_none()) {
                    return Err(format!("drum pad {} is not present on channel {}", note.key, channel.id));
                }
                Event::Drum { pos, len_beats: note.length_beats, pad: note.key,
                    velocity: note.velocity, pan: Event::DEFAULT_PAN,
                    value: Event::DEFAULT_PITCH, repeat: 0 }
            } else {
                if note.key > 127 { return Err("MIDI keys must be 0–127".into()); }
                Event::Note { pos, len_beats: note.length_beats, key: note.key,
                    velocity: note.velocity, pan: Event::DEFAULT_PAN, slide: false }
            };
            events.push(event);
        }
        events.sort_by_key(Event::pos);
        let clip = if let Some(clip_id) = replace {
            let clip = lane.clips.iter_mut().find(|c| c.id == clip_id)
                .ok_or_else(|| format!("clip {clip_id} is not on channel {}", channel.id))?;
            if clip.start_tick != start_tick || (clip.loop_len_beats - length).abs() > 0.001 {
                return Err("replacement clip must have the given start_bar and bars".into());
            }
            clip
        } else {
            if lane.clips.iter().any(|c| c.start_tick == start_tick) {
                return Err(format!("channel {} already has a clip at bar {}: use replace_clip_id", channel.id, pattern.start_bar));
            }
            lane.add_clip(start_tick, length)
        };
        clip.events = events;
        let clip_id = clip.id;
        made.push(json!({"channel_id": channel.id, "channel_name": channel.name, "clip_id": clip_id}));
    }
    Ok((result, made))
}

impl HoneydewApp {
    pub(super) fn control_frame(&mut self, tick: u64) {
        let requests = self.control.as_ref().map(Bridge::drain).unwrap_or_default();
        for request in requests {
            let response = self.control_call(request.value, tick);
            let _ = request.reply.send(response);
        }
    }

    fn control_call(&mut self, value: Value, tick: u64) -> Value {
        let answer = match value.get("action").and_then(Value::as_str) {
            Some("song") => Ok(self.control_song()),
            Some("instruments") => Ok(json!({
                "built_in": SynthKind::ALL.iter().map(|s| s.label()).chain(["Drums"]).collect::<Vec<_>>(),
                "plugins": crate::plugins::available().into_iter()
                    .filter(|p| p.kind == crate::plugins::PluginKind::Instrument)
                    .map(|p| json!({"uid": p.uid, "name": p.name, "format": p.format}))
                    .collect::<Vec<_>>()
            })),
            Some("instrument_params") => value.get("channel_id").and_then(Value::as_i64)
                .and_then(|id| i32::try_from(id).ok())
                .ok_or_else(|| "channel_id must be an integer".to_owned())
                .and_then(|id| self.control_instrument_params(id)),
            Some("set_instrument_params") => {
                serde_json::from_value::<SetInstrumentParams>(value.get("edit").cloned().unwrap_or(Value::Null))
                    .map_err(|e| e.to_string())
                    .and_then(|edit| {
                        let module_id = edit.module_id;
                        let (channel, changes) = set_instrument_params(&mut self.song, edit)?;
                        for &(index, value) in &changes {
                            self.audio.send(AudioCommand::SetParam {
                                channel, instance_id: module_id, index: index as i32, value,
                            });
                        }
                        self.song_edited(SongEdit::ChannelParam, tick);
                        Ok(json!({"channel_id": self.song.channels[channel].id,
                            "module_id": module_id, "changed": changes.len(), "undoable": false}))
                    })
            }
            Some("arrange") => {
                let parsed = serde_json::from_value::<Arrangement>(value.get("arrangement").cloned().unwrap_or(Value::Null));
                parsed.map_err(|e| e.to_string()).and_then(|arrangement| {
                    let (song, made) = arrange(&self.song, arrangement)?;
                    self.song = song;
                    self.song_edited(SongEdit::Final, tick);
                    Ok(json!({"created": made, "undo_steps": 1}))
                })
            }
            _ => Err("unknown action".into()),
        };
        match answer {
            Ok(result) => json!({"result": result}),
            Err(error) => json!({"error": error}),
        }
    }

    fn control_song(&self) -> Value {
        json!({
            "title": self.song.title,
            "tempo_bpm": self.song.tempo_bpm,
            "beats_per_bar": self.song.time_sig.bar_beats(),
            "channels": self.song.channels.iter().skip(1).map(|ch| json!({
                "id": ch.id,
                "name": ch.name,
                "lane_kind": format!("{:?}", ch.lanes[0].kind).to_lowercase(),
                "instrument": ch.rack.modules.iter().filter_map(|m| {
                    (m.type_id == crate::song::plugin::PLUGIN_INSTRUMENT
                        || SynthKind::ALL.iter().any(|s| s.type_id() == m.type_id))
                        .then(|| m.title())
                }).collect::<Vec<_>>(),
                "drum_pads": ch.drum_kit.as_ref().map(|kit| kit.slots.iter()
                    .enumerate().map(|(index, slot)| json!({"index": index, "name": slot.display_name()}))
                    .collect::<Vec<_>>()),
                "clips": ch.lanes[0].clips.iter().map(|c| json!({
                    "id": c.id,
                    "start_bar": f64::from(c.start_tick) / f64::from(PPQ) / f64::from(self.song.time_sig.bar_beats()),
                    "bars": c.loop_len_beats / f64::from(self.song.time_sig.bar_beats()),
                    "note_count": c.events.len()
                })).collect::<Vec<_>>()
            })).collect::<Vec<_>>()
        })
    }

    fn control_instrument_params(&self, channel_id: i32) -> Result<Value, String> {
        let channel = self.song.channels.iter().find(|ch| ch.id == channel_id && ch.id != 0)
            .ok_or_else(|| format!("channel {channel_id} is not in the open song"))?;
        let modules = channel.rack.modules.iter().filter_map(|module| {
            let instrument = built_in_instrument(module.type_id)?;
            let params = module.params.iter().enumerate().filter_map(|(index, &value)| {
                let spec = module.param_spec(index)?;
                let kind = match spec.kind {
                    ParamKind::Continuous { .. } => "continuous",
                    ParamKind::Menu { .. } | ParamKind::MenuBy { .. } => "menu",
                    ParamKind::Toggle => "toggle",
                };
                Some(json!({"index": index,
                    "name": parameter_name(module.type_id, index, spec.name),
                    "kind": kind, "options": spec.kind.items(),
                    "value": value, "default": spec.default,
                    "display": spec.display_text(value)}))
            }).collect::<Vec<_>>();
            Some(json!({"module_id": module.instance_id, "instrument": instrument, "parameters": params}))
        }).collect::<Vec<_>>();
        Ok(json!({"channel_id": channel_id, "channel_name": channel.name, "modules": modules}))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn arrangement_is_one_valid_song_with_builtin_drum_and_synth() {
        let before = Song::default();
        let request: Arrangement = serde_json::from_value(json!({"tracks": [
            {"target": {"kind": "new", "instrument": "Drums", "name": "Beat"},
             "start_bar": 0, "bars": 1, "notes": [{"beat": 0.0, "length_beats": 0.25, "key": 0, "velocity": 220}]},
            {"target": {"kind": "new", "instrument": "MiniSynth", "name": "Bass"},
             "start_bar": 0, "bars": 1, "notes": [{"beat": 0.5, "length_beats": 0.5, "key": 48, "velocity": 180}]}
        ]})).unwrap();
        let (song, made) = arrange(&before, request).unwrap();
        assert_eq!(made.len(), 2);
        assert_eq!(song.channels[1].lanes[0].clips[0].events.len(), 1);
        assert_eq!(song.channels[2].lanes[0].clips[0].events.len(), 1);
        assert!(crate::flm::write(&song).is_ok());
        assert_eq!(before.channels.len(), 1);
    }

    #[test]
    fn invalid_note_does_not_change_input_song() {
        let before = Song::default();
        let request: Arrangement = serde_json::from_value(json!({"tracks": [{
            "target": {"kind": "new", "instrument": "MiniSynth"},
            "start_bar": 0, "bars": 1,
            "notes": [{"beat": 4.0, "length_beats": 1.0, "key": 60, "velocity": 100}]
        }]})).unwrap();
        assert!(arrange(&before, request).is_err());
        assert_eq!(before.channels.len(), 1);
    }

    #[test]
    fn existing_plugin_sound_is_kept_when_adding_and_replacing_notes() {
        let mut before = Song::default();
        before.add_channel(ChannelKind::Instrument);
        let channel_id = before.channels[1].id;
        let module_id = before.channels[1].rack.add(crate::song::plugin::PLUGIN_INSTRUMENT);
        let module = before.channels[1].rack.get_mut(module_id).unwrap();
        module.file.plugin = Some(Box::new(crate::song::plugin::PluginRef {
            name: "Vital".into(), format: "clap".into(), id: "vital.test".into(),
            state: vec![1, 2, 3].into(), ..Default::default()
        }));
        let first: Arrangement = serde_json::from_value(json!({"tracks": [{
            "target": {"kind": "existing", "channel_id": channel_id},
            "start_bar": 0, "bars": 1,
            "notes": [{"beat": 0.0, "length_beats": 1.0, "key": 60, "velocity": 200}]
        }]})).unwrap();
        let (song, made) = arrange(&before, first).unwrap();
        let clip_id = made[0]["clip_id"].as_u64().unwrap();
        let second: Arrangement = serde_json::from_value(json!({"tracks": [{
            "target": {"kind": "existing", "channel_id": channel_id, "replace_clip_id": clip_id},
            "start_bar": 0, "bars": 1,
            "notes": [{"beat": 0.5, "length_beats": 0.5, "key": 48, "velocity": 180}]
        }]})).unwrap();
        let (changed, _) = arrange(&song, second).unwrap();
        assert_eq!(changed.channels[1].lanes[0].clips.len(), 1);
        assert_eq!(changed.channels[1].rack, before.channels[1].rack);
        assert_eq!(changed.channels[1].rack.get(module_id).unwrap().file.plugin.as_ref().unwrap().state.as_ref(), &[1, 2, 3]);
    }

    #[test]
    fn control_action_updates_open_song_through_edit_path() {
        let mut app = HoneydewApp::default();
        let answer = app.control_call(json!({"action": "arrange", "arrangement": {
            "tempo_bpm": 138.0,
            "tracks": [{"target": {"kind": "new", "instrument": "Drums"},
                "start_bar": 0, "bars": 1,
                "notes": [{"beat": 0.0, "length_beats": 0.25, "key": 0, "velocity": 220}] }]
        }}), 0);
        assert!(answer.get("error").is_none(), "{answer}");
        assert_eq!(app.song.tempo_bpm, 138.0);
        assert_eq!(app.song.channels.len(), 2);
        assert_eq!(app.song.channels[1].lanes[0].clips.len(), 1);
        let undone = app.history.undo().expect("one control call is one undo step");
        assert_eq!(undone.channels.len(), 1);
        assert_eq!(undone.tempo_bpm, 120.0);
        assert!(app.history.undo().is_none());
    }

    #[test]
    fn built_in_parameters_are_read_and_applied_atomically() {
        let mut app = HoneydewApp::default();
        let channel = app.song.insert_channel(1, ChannelKind::Synth(SynthKind::TransistorBass)).unwrap();
        let channel_id = app.song.channels[channel].id;
        let module_id = app.song.channels[channel].rack.modules[0].instance_id;
        let found = app.control_call(json!({"action": "instrument_params", "channel_id": channel_id}), 0);
        let params = &found["result"]["modules"][0]["parameters"];
        assert_eq!(found["result"]["modules"][0]["module_id"], module_id);
        assert_eq!(params[9]["name"], "Drive");
        let bad = app.control_call(json!({"action": "set_instrument_params", "edit": {
            "channel_id": channel_id, "module_id": module_id,
            "changes": [{"index": 2, "value": 0.45}, {"index": 99, "value": 0.5}]
        }}), 0);
        assert!(bad.get("error").is_some(), "{bad}");
        assert_eq!(app.song.channels[channel].rack.modules[0].params[2], 1.0);
        let changed = app.control_call(json!({"action": "set_instrument_params", "edit": {
            "channel_id": channel_id, "module_id": module_id,
            "changes": [{"index": 2, "value": 0.45}, {"index": 9, "value": 0.8}]
        }}), 0);
        assert_eq!(changed["result"]["changed"], 2, "{changed}");
        assert_eq!(app.song.channels[channel].rack.modules[0].params[2], 0.45);
        assert_eq!(app.song.channels[channel].rack.modules[0].params[9], 0.8);
    }

    #[test]
    fn external_plugin_module_cannot_be_edited_as_built_in() {
        let mut song = Song::default();
        let index = song.insert_channel(1, ChannelKind::Instrument).unwrap();
        let module_id = song.channels[index].rack.add(crate::song::plugin::PLUGIN_INSTRUMENT);
        let request: SetInstrumentParams = serde_json::from_value(json!({
            "channel_id": song.channels[index].id, "module_id": module_id,
            "changes": [{"index": 0, "value": 0.0}]
        })).unwrap();
        assert!(set_instrument_params(&mut song, request).is_err());
    }
}
