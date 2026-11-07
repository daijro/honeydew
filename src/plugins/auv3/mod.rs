//! Apple AUv3 backend (macOS and iOS). System components are discovered by identity, never by file path.
//! The Objective-C bridge owns asynchronous instantiation and stereo AU rendering.
use std::ffi::{CStr, c_char, c_void};
use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex};

use os_engine::TransportInfo;
use os_host::{
    Descriptor, Format, HostedEffect, Instrument, Kind, Library, ParamEvent, ParamInfo, Plugin,
    SharedPlugin,
};
use serde::Deserialize;

unsafe extern "C" {
    fn hd_au_discover() -> *mut c_char;
    fn hd_au_free(s: *mut c_char);
    fn hd_au_create(kind: u32, subtype: u32, manufacturer: u32) -> *mut c_void;
    fn hd_au_destroy(unit: *mut c_void);
    fn hd_au_poll(unit: *mut c_void, rate: u32) -> i32;
    fn hd_au_error(unit: *mut c_void) -> *mut c_char;
    fn hd_au_parameters(unit: *mut c_void) -> *mut c_char;
    fn hd_au_take_value(unit: *mut c_void, index: u32) -> f32;
    fn hd_au_parameter(unit: *mut c_void, index: u32, value: f32, offset: u32);
    fn hd_au_set_value(unit: *mut c_void, index: u32, value: f32);
    fn hd_au_midi(unit: *mut c_void, offset: u32, bytes: *const u8, length: u32);
    fn hd_au_render(
        unit: *mut c_void,
        left: *mut f32,
        right: *mut f32,
        frames: u32,
        effect: i32,
    ) -> i32;
    fn hd_au_reset(unit: *mut c_void);
    fn hd_au_transport(
        unit: *mut c_void,
        tempo: f64,
        beat: f64,
        bar: f64,
        frame: f64,
        numerator: u32,
        denominator: u32,
        playing: i32,
    );
    fn hd_au_state(unit: *mut c_void, length: *mut usize) -> *mut u8;
    fn hd_au_load_state(unit: *mut c_void, bytes: *const u8, length: usize) -> i32;
    fn hd_au_latency(unit: *mut c_void) -> u32;
    fn hd_au_offline(unit: *mut c_void, offline: i32);
    fn hd_au_has_editor(unit: *mut c_void) -> i32;
    fn hd_au_open_editor(unit: *mut c_void) -> i32;
    fn hd_au_close_editor(unit: *mut c_void);
    fn hd_au_editor_open(unit: *mut c_void) -> i32;
}

// Every returned JSON string is allocated by the bridge and freed exactly once here.
unsafe fn read_json<T: serde::de::DeserializeOwned>(pointer: *mut c_char) -> Option<T> {
    if pointer.is_null() {
        return None;
    }
    let value = serde_json::from_slice(unsafe { CStr::from_ptr(pointer) }.to_bytes())
        .map_err(|error| {
            eprintln!("AUv3 bridge metadata could not be read: {error}");
        })
        .ok();
    unsafe { hd_au_free(pointer) };
    value
}

#[derive(Deserialize)]
struct Component {
    #[serde(rename = "type")]
    kind: u32,
    subtype: u32,
    manufacturer: u32,
    name: String,
    vendor: String,
    instrument: bool,
}

pub struct Components {
    descriptors: Vec<Descriptor>,
}
impl Components {
    pub fn discover() -> Arc<Self> {
        let components: Vec<Component> = unsafe { read_json(hd_au_discover()) }.unwrap_or_default();
        Arc::new(Self {
            descriptors: components
                .into_iter()
                .map(|c| Descriptor {
                    format: Format::Auv3,
                    id: format!("{:08X}-{:08X}-{:08X}", c.kind, c.subtype, c.manufacturer),
                    name: c.name,
                    vendor: c.vendor,
                    version: String::new(),
                    features: vec![],
                    kind: if c.instrument {
                        Kind::Instrument
                    } else {
                        Kind::Effect
                    },
                    path: PathBuf::new(),
                })
                .collect(),
        })
    }
}
impl Library for Components {
    fn path(&self) -> &Path {
        Path::new("")
    }
    fn plugins(&self) -> &[Descriptor] {
        &self.descriptors
    }
    fn create(self: Arc<Self>, id: &str) -> Result<SharedPlugin, String> {
        let desc = self
            .descriptors
            .iter()
            .find(|d| d.id == id)
            .ok_or("AUv3 is not installed")?
            .clone();
        let parts: Vec<_> = id
            .split('-')
            .map(|s| u32::from_str_radix(s, 16))
            .collect::<Result<_, _>>()
            .map_err(|e| e.to_string())?;
        if parts.len() != 3 {
            return Err("Invalid AUv3 component identity".into());
        }
        let handle = unsafe { hd_au_create(parts[0], parts[1], parts[2]) };
        if handle.is_null() {
            return Err("AUv3 could not be created".into());
        }
        Ok(Arc::new(Mutex::new(Unit {
            desc,
            handle,
            rate: 48_000,
            ready: false,
            failed: false,
            params: vec![],
            values: vec![],
            events: vec![],
            started: std::time::Instant::now(),
            midi: super::midi_state::MidiState::default(),
            replay_midi: true,
        })))
    }
}

#[derive(Deserialize)]
struct Parameter {
    address: String,
    name: String,
    min: f64,
    max: f64,
    value: f64,
    writable: bool,
    automatable: bool,
}
struct Unit {
    desc: Descriptor,
    handle: *mut c_void,
    rate: u32,
    ready: bool,
    failed: bool,
    params: Vec<ParamInfo>,
    values: Vec<f64>,
    events: Vec<ParamEvent>,
    started: std::time::Instant,
    midi: super::midi_state::MidiState,
    replay_midi: bool,
}
// The instance mutex serializes app/audio access. UIKit and resource calls are
// app-thread-only; only the documented AU render/scheduling API crosses to audio.
unsafe impl Send for Unit {}
impl Drop for Unit {
    fn drop(&mut self) {
        unsafe { hd_au_destroy(self.handle) };
    }
}

impl Unit {
    fn poll(&mut self) {
        if self.failed {
            return;
        }
        let status = unsafe { hd_au_poll(self.handle, self.rate) };
        if status < 0 || (status == 0 && self.started.elapsed().as_secs() >= 30) {
            self.failed = true;
            self.ready = false;
            let why: String = unsafe { read_json(hd_au_error(self.handle)) }
                .unwrap_or_else(|| "AUv3 timed out".into());
            eprintln!("AUv3 {}: {why}", self.desc.name);
            return;
        }
        if status == 1 && !self.ready {
            let Some(parameters): Option<Vec<Parameter>> =
                (unsafe { read_json(hd_au_parameters(self.handle)) })
            else {
                self.failed = true;
                return;
            };
            let mut ids = std::collections::HashSet::new();
            self.params.clear();
            self.values.clear();
            for p in parameters {
                // AUParameterAddress is 64-bit; the song format uses 32-bit IDs.
                // Hash the full stable address, and refuse collisions rather than automate another control.
                let Ok(address) = p.address.parse::<u64>() else {
                    self.failed = true;
                    return;
                };
                let id = parameter_id(address);
                if !ids.insert(id) {
                    self.failed = true;
                    return;
                }
                let value = if p.max > p.min {
                    ((p.value - p.min) / (p.max - p.min)).clamp(0.0, 1.0)
                } else {
                    0.0
                };
                self.params.push(ParamInfo {
                    id,
                    name: p.name,
                    unit: String::new(),
                    group: String::new(),
                    min: p.min,
                    max: p.max,
                    default: value,
                    steps: 0,
                    automatable: p.automatable && p.writable,
                    read_only: !p.writable,
                    hidden: false,
                    bypass: false,
                    enumerated: false,
                });
                self.values.push(value);
            }
            self.ready = true;
            self.replay_midi = true;
        }
    }
    fn render(&mut self, rate: u32, left: &mut [f32], right: &mut [f32], effect: bool) {
        let frames = left.len().min(right.len()).min(u32::MAX as usize);
        if self.ready && !self.failed && rate == self.rate && !effect && self.replay_midi {
            let handle = self.handle;
            self.midi.replay(|message| unsafe {
                hd_au_midi(
                    handle,
                    0,
                    message.as_ptr(),
                    if message[0] == 0xD0 { 2 } else { 3 },
                );
            });
            self.replay_midi = false;
        }
        let ok = self.ready
            && !self.failed
            && rate == self.rate
            && unsafe {
                hd_au_render(
                    self.handle,
                    left.as_mut_ptr(),
                    right.as_mut_ptr(),
                    frames as u32,
                    i32::from(effect),
                ) != 0
            };
        if !ok && !effect {
            left.fill(0.0);
            right.fill(0.0);
        }
    }
    fn schedule(&mut self, offset: u32, index: u32, value: f64) {
        if !value.is_finite() {
            return;
        }
        if let Some(p) = self.params.get(index as usize) {
            if p.read_only || !p.automatable {
                return;
            }
            let v = value.clamp(0.0, 1.0);
            self.values[index as usize] = v;
            unsafe { hd_au_parameter(self.handle, index, v as f32, offset) };
        }
    }
    fn position(&mut self, info: &TransportInfo) {
        unsafe {
            hd_au_transport(
                self.handle,
                info.tempo,
                info.beats,
                info.bar_start,
                info.frame as f64,
                u32::from(info.numerator),
                u32::from(info.denominator),
                i32::from(info.playing),
            )
        };
    }
}
fn parameter_id(address: u64) -> u32 {
    address
        .to_le_bytes()
        .into_iter()
        .fold(2166136261u32, |hash, b| {
            (hash ^ u32::from(b)).wrapping_mul(16777619)
        })
}
impl Instrument for Unit {
    fn note_on(&mut self, offset: u32, key: u8, velocity: f32) {
        self.midi(
            offset,
            [
                0x90,
                key.min(127),
                (velocity.clamp(0.0, 1.0) * 127.0).round() as u8,
            ],
        );
    }
    fn note_off(&mut self, offset: u32, key: u8) {
        self.midi(offset, [0x80, key.min(127), 0]);
    }
    fn midi(&mut self, offset: u32, bytes: [u8; 3]) {
        self.midi.remember(bytes);
        if !self.ready || self.failed || self.replay_midi {
            return;
        }
        unsafe {
            hd_au_midi(
                self.handle,
                offset,
                bytes.as_ptr(),
                if bytes[0] & 0xF0 == 0xD0 { 2 } else { 3 },
            )
        };
    }
    fn process(&mut self, rate: u32, left: &mut [f32], right: &mut [f32]) {
        self.render(rate, left, right, false);
    }
    fn param(&mut self, offset: u32, index: u32, value: f64) {
        self.schedule(offset, index, value);
    }
    fn reset(&mut self) {
        self.midi = super::midi_state::MidiState::default();
        self.replay_midi = true;
        unsafe { hd_au_reset(self.handle) };
    }
    fn transport(&mut self, info: &TransportInfo) {
        self.position(info);
    }
}
impl HostedEffect for Unit {
    fn process_fx(&mut self, rate: u32, left: &mut [f32], right: &mut [f32]) {
        self.render(rate, left, right, true);
    }
    fn param_fx(&mut self, offset: u32, index: u32, value: f64) {
        self.schedule(offset, index, value);
    }
    fn reset_fx(&mut self) {
        self.reset();
    }
    fn transport_fx(&mut self, info: &TransportInfo) {
        self.position(info);
    }
}
impl Plugin for Unit {
    fn desc(&self) -> &Descriptor {
        &self.desc
    }
    fn activate(&mut self, rate: u32) -> bool {
        if self.rate != rate {
            self.ready = false;
        }
        self.rate = rate;
        self.poll();
        !self.failed
    }
    fn idle(&mut self) {
        self.poll();
        if !self.ready || self.failed {
            return;
        }
        for (index, p) in self.params.iter().enumerate() {
            let value = f64::from(unsafe { hd_au_take_value(self.handle, index as u32) });
            if value.is_finite() && (value - self.values[index]).abs() > 0.00001 {
                self.values[index] = value;
                self.events.push(ParamEvent::Value(p.id, value));
            }
        }
    }
    fn has_editor(&self) -> bool {
        self.ready && !self.failed && unsafe { hd_au_has_editor(self.handle) != 0 }
    }
    fn editor_open(&self) -> bool {
        unsafe { hd_au_editor_open(self.handle) != 0 }
    }
    fn open_editor(&mut self, _: *mut c_void, _: f32) -> Result<(u32, u32), String> {
        if unsafe { hd_au_open_editor(self.handle) } == 0 {
            Err("AUv3 editor is not available yet".into())
        } else {
            Ok((0, 0))
        }
    }
    fn editor_size(&self) -> Option<(u32, u32)> {
        None
    }
    fn close_editor(&mut self) {
        unsafe { hd_au_close_editor(self.handle) };
    }
    fn save_state(&self) -> Option<Vec<u8>> {
        let mut length = 0;
        let bytes = unsafe { hd_au_state(self.handle, &mut length) };
        if bytes.is_null() {
            return None;
        }
        let state = unsafe { std::slice::from_raw_parts(bytes, length) }.to_vec();
        unsafe { hd_au_free(bytes.cast()) };
        Some(state)
    }
    fn load_state(&mut self, data: &[u8]) -> bool {
        unsafe { hd_au_load_state(self.handle, data.as_ptr(), data.len()) != 0 }
    }
    fn param_count(&self) -> usize {
        self.params.len()
    }
    fn param_info(&self, index: usize) -> Option<ParamInfo> {
        self.params.get(index).cloned()
    }
    fn param_value(&self, id: u32) -> Option<f64> {
        self.params
            .iter()
            .position(|p| p.id == id)
            .map(|i| self.values[i])
    }
    fn set_param(&mut self, id: u32, value: f64) -> bool {
        let Some(index) = self.params.iter().position(|p| p.id == id && !p.read_only) else {
            return false;
        };
        if !value.is_finite() {
            return false;
        }
        self.values[index] = value.clamp(0.0, 1.0);
        unsafe { hd_au_set_value(self.handle, index as u32, self.values[index] as f32) };
        true
    }
    fn take_param_events(&mut self) -> Vec<ParamEvent> {
        std::mem::take(&mut self.events)
    }
    fn latency(&self) -> u32 {
        unsafe { hd_au_latency(self.handle) }
    }
    fn set_offline(&mut self, value: bool) {
        unsafe { hd_au_offline(self.handle, i32::from(value)) };
    }
    fn stopped(&self) -> Option<u32> {
        self.failed.then_some(0x41557633)
    }
}
