// Copyright (C) 2026 ItsAlphaNeon
// SPDX-License-Identifier: GPL-3.0-or-later
// This file is part of Lemon Studio: https://github.com/ItsAlphaNeon/Lemon-Studio
// Modified for Honeydew: vendored from Lemon Studio's `os-flp` crate (219ed4e); Honeydew's `PluginRef` (its state an
// `Arc<[u8]>`, its equality without the state, so the unchanged check compares the state itself); upstream's tests are
// not carried (Honeydew's FLP round trip covers the codec, `tests/plugins.rs`).

//! The plugin wrapper's state: event 213 of a plugin whose internal name (201) is [`WRAPPER`], which hosts every
//! third-party plugin (VST2, VST3, CLAP). A `u32` type, then `(u32 id, u64 size, bytes)` records. Unknown records
//! are kept verbatim when a state is changed.

use crate::song::plugin::PluginRef;

/// The wrapper's internal name (event 201): a file-format identifier.
pub const WRAPPER: &str = "Fruity Wrapper";

/// Record ids.
pub mod rec {
    pub const INFO: u32 = 50;
    pub const FOURCC: u32 = 51;
    pub const GUID: u32 = 52;
    pub const STATE: u32 = 53;
    pub const NAME: u32 = 54;
    pub const PATH: u32 = 55;
    pub const VENDOR: u32 = 56;
    /// A VST2 plugin's VST version (2400 in every VST2 wrapper state FL Studio wrote that has been seen).
    pub const VST_VERSION: u32 = 57;
}

/// What Lemon Studio writes as a new VST2 wrapper state's VST version.
const VST2_VERSION: u32 = 2400;

/// The type word Lemon Studio writes for a new wrapper state.
pub const NEW_TYPE: u32 = 10;

/// A parsed wrapper state: the type word and the records in file order.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Records {
    pub kind: u32,
    pub records: Vec<(u32, Vec<u8>)>,
}

impl Records {
    /// Parses a whole wrapper state; `None` when the bytes are not exactly a type word and records.
    pub fn parse(data: &[u8]) -> Option<Records> {
        let kind = u32::from_le_bytes(data.get(0..4)?.try_into().ok()?);
        let mut records = Vec::new();
        let mut p = 4;
        while p < data.len() {
            let id = u32::from_le_bytes(data.get(p..p + 4)?.try_into().ok()?);
            let size = u64::from_le_bytes(data.get(p + 4..p + 12)?.try_into().ok()?);
            let size = usize::try_from(size).ok()?;
            let body = data.get(p + 12..p.checked_add(12)?.checked_add(size)?)?;
            records.push((id, body.to_vec()));
            p += 12 + size;
        }
        Some(Records { kind, records })
    }

    pub fn to_bytes(&self) -> Vec<u8> {
        let mut out = self.kind.to_le_bytes().to_vec();
        for (id, body) in &self.records {
            out.extend(id.to_le_bytes());
            out.extend((body.len() as u64).to_le_bytes());
            out.extend(body);
        }
        out
    }

    pub fn get(&self, id: u32) -> Option<&[u8]> {
        self.records
            .iter()
            .find(|(i, _)| *i == id)
            .map(|(_, b)| b.as_slice())
    }

    /// Replaces record `id`'s bytes, or adds the record before the first one with a higher id.
    pub fn set(&mut self, id: u32, body: Vec<u8>) {
        if let Some(r) = self.records.iter_mut().find(|(i, _)| *i == id) {
            r.1 = body;
            return;
        }
        let at = self
            .records
            .iter()
            .position(|(i, _)| *i > id)
            .unwrap_or(self.records.len());
        self.records.insert(at, (id, body));
    }

    pub fn remove(&mut self, id: u32) {
        self.records.retain(|(i, _)| *i != id);
    }
}

fn text(b: Option<&[u8]>) -> String {
    let b = b.unwrap_or(&[]);
    String::from_utf8_lossy(b.strip_suffix(&[0]).unwrap_or(b)).into_owned()
}

fn hex(b: &[u8]) -> String {
    b.iter().map(|x| format!("{x:02X}")).collect()
}

fn unhex(s: &str) -> Option<Vec<u8>> {
    if !s.len().is_multiple_of(2) {
        return None;
    }
    (0..s.len())
        .step_by(2)
        .map(|i| u8::from_str_radix(s.get(i..i + 2)?, 16).ok())
        .collect()
}

/// The format of a wrapped plugin: by the file extension, else a VST2 id record = VST2, else a class-id record = VST3,
/// else VST2. FL Studio 2026 keeps a 16-byte class-id record next to some VST2 plugins' id, so the id decides first.
fn format_of(r: &Records, path: &str) -> &'static str {
    let lower = path.to_ascii_lowercase();
    if lower.ends_with(".vst3") {
        "vst3"
    } else if lower.ends_with(".clap") {
        "clap"
    } else if r.get(rec::FOURCC).is_some() {
        "vst2"
    } else if r.get(rec::GUID).is_some_and(|g| g.len() == 16) {
        "vst3"
    } else {
        "vst2"
    }
}

/// A VST2 unique id record (a little-endian 32-bit number: `'4FTP'` is stored `PTF4`) as 8 hex digits, `34465450`.
fn vst2_id(b: &[u8]) -> Option<String> {
    Some(format!("{:08X}", u32::from_le_bytes(b.try_into().ok()?)))
}

/// The plugin a wrapper state names. `None` when `data` is not a wrapper state.
pub fn decode(data: &[u8]) -> Option<PluginRef> {
    let r = Records::parse(data)?;
    let path = text(r.get(rec::PATH));
    let format = format_of(&r, &path);
    let id = match format {
        "vst3" => r
            .get(rec::GUID)
            .filter(|g| g.len() == 16)
            .map(hex)
            .unwrap_or_default(),
        "vst2" => r.get(rec::FOURCC).and_then(vst2_id).unwrap_or_default(),
        _ => String::new(),
    };
    Some(PluginRef {
        format: format.into(),
        id,
        name: text(r.get(rec::NAME)),
        vendor: text(r.get(rec::VENDOR)),
        path,
        state: r.get(rec::STATE).unwrap_or(&[]).into(),
        params: Vec::new(),
    })
}

/// A wrapper state for plugin `p`. With a `base` wrapper state (the one the file held), only the identity and state
/// records are replaced and every other record keeps its bytes and place; without one, a new state is built.
pub fn encode(base: Option<&[u8]>, p: &PluginRef) -> Vec<u8> {
    let mut r = base.and_then(Records::parse).unwrap_or_else(|| Records {
        kind: NEW_TYPE,
        records: vec![(rec::INFO, vec![0; 16])],
    });
    if p.format == "vst3" {
        if let Some(g) = unhex(&p.id).filter(|g| g.len() == 16) {
            r.set(rec::GUID, g);
        }
    } else {
        r.remove(rec::GUID);
    }
    if p.format == "vst2" {
        if let Ok(id) = u32::from_str_radix(&p.id, 16) {
            r.set(rec::FOURCC, id.to_le_bytes().to_vec());
        }
        if r.get(rec::VST_VERSION).is_none() {
            r.set(rec::VST_VERSION, VST2_VERSION.to_le_bytes().to_vec());
        }
    } else {
        r.remove(rec::FOURCC);
        r.remove(rec::VST_VERSION);
    }
    r.set(rec::STATE, p.state.to_vec());
    r.set(rec::NAME, p.name.as_bytes().to_vec());
    r.set(rec::PATH, p.path.as_bytes().to_vec());
    if !p.vendor.is_empty() || r.get(rec::VENDOR).is_some() {
        r.set(rec::VENDOR, p.vendor.as_bytes().to_vec());
    }
    let out = r.to_bytes();
    // An unchanged plugin keeps the exact bytes it had.
    let same = |d: &PluginRef| {
        (&d.format, &d.id, &d.name, &d.vendor, &d.path, &d.state)
            == (&p.format, &p.id, &p.name, &p.vendor, &p.path, &p.state)
    };
    match base {
        Some(b) if decode(b).as_ref().is_some_and(same) => b.to_vec(),
        _ => out,
    }
}

/// The name a wrapper state shows for its plugin (record 54), if `data` is one.
pub fn plugin_name(data: &[u8]) -> Option<String> {
    Records::parse(data)
        .map(|r| text(r.get(rec::NAME)))
        .filter(|n| !n.is_empty())
}
