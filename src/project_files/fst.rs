//! 3x Osc's `.fst` presets: FL desktop channel presets, imported
//! onto the module's params. [`import_fst`] reads the events as listed below; the
//! preset picker loads the result like a module preset ([`super::load_pick`]).
//!
//! - From the module defaults, after the header, events to the end of the file by size class (id 0–63 a u8, 64–127 a
//!   u16, 128–191 a u32, 192–255 a varint length and bytes). Used: u8 16, u32 138, and the blobs 196, 199, 209, 213,
//!   215, 218–221, 228, 229; everything else is skipped.
//! - A value is the file's i32 (u8 for flags) through the param's law in f32, clamped to 0..=1 as it is set. V, the FL
//!   version (199), picks the older conversions; it is 0 until the event.
//! - An abort ([`FstImport::ok`] false) keeps what was written before it. The app then discards it: the module goes to
//!   its defaults and reads the bytes as a native state ([`native_state`]).

use std::path::Path;

use crate::flm::chunk::{ChunkIter, ChunkWriter, ContentPath, ContentRoot};
use crate::song::ModuleInstance;

/// The file type.
pub const FST_EXT: &str = "fst";
const THREE_OSC: u16 = 39;
/// `FLhd`, a u32, the u16 format, 4 bytes, `FLdt`, a u32.
const HEADER: usize = 22;

/// What a `.fst` gives the 3x Osc module.
#[derive(Clone, Debug, PartialEq)]
pub struct FstImport {
    /// The module's params, from the registry defaults, each in 0..=1.
    pub params: Vec<f32>,
    /// No abort rule hit. On an abort the params written before it stay.
    pub ok: bool,
    /// The custom waveform (event 196) when its path names one: `@R@3xOsc Shapes/<rest>`.
    pub custom_shape: Option<ContentPath>,
    /// The file name without its last extension, and its parent folder's name: set even when the import fails.
    pub name: String,
    pub category: String,
}

/// Imports the `.fst` `bytes` of the file at `path`.
pub fn import_fst(bytes: &[u8], path: &Path) -> FstImport {
    let mut load = Load {
        params: ModuleInstance::new(THREE_OSC, 0).params,
        v: 0,
        blocks: 0,
        tracking: 0,
        shape: None,
    };
    let ok = load.file(bytes).is_some();
    let name =
        |p: Option<&std::ffi::OsStr>| p.map_or_else(String::new, |n| n.to_string_lossy().into());
    FstImport {
        params: load.params,
        ok,
        custom_shape: load.shape,
        name: name(path.file_stem()),
        category: name(path.parent().and_then(Path::file_name)),
    }
}

/// The fallback after a failed import: `bytes` read as a native module state, from 3x Osc's defaults,
/// through its chunks tagged HEAD, ADD1, DESC, PRST, PRMS, SMPR or CSTM; the others (`FLhd`, `FLdt`) are skipped.
/// `None` when there are none, or they do not read as a 3x Osc state (then the defaults stand).
pub(crate) fn native_state(bytes: &[u8]) -> Option<ModuleInstance> {
    const TAGS: [[u8; 4]; 7] = [
        *b"HEAD", *b"ADD1", *b"DESC", *b"PRST", *b"PRMS", *b"SMPR", *b"CSTM",
    ];
    let chunks: Vec<_> = (ChunkIter::new(bytes, "FST").map_while(Result::ok))
        .filter(|c| TAGS.contains(&c.id))
        .collect();
    if chunks.is_empty() {
        return None;
    }
    let mut writer = ChunkWriter::default();
    // The preset reader wants a HEAD: without one the state is 3x Osc's own.
    if !chunks.iter().any(|c| c.id == *b"HEAD") {
        let head = [&u32::from(THREE_OSC).to_le_bytes()[..], &[0; 4], &[1]].concat();
        writer.chunk(*b"HEAD", |w| w.raw(&head)).ok()?;
    }
    for c in &chunks {
        writer.chunk(c.id, |w| w.raw(c.payload)).ok()?;
    }
    crate::flm::read_preset(&writer.into_bytes(), Some(THREE_OSC)).ok()
}

/// An import under way.
struct Load {
    params: Vec<f32>,
    /// V: `a·1 000 000 + b·1 000 + c` of the FL version "a.b.c" (199), 0 before it.
    v: i64,
    /// The 218 blocks (Pan, Volume, ModX, ModY, Pitch, then unbounded) and 228 events read so far.
    blocks: usize,
    tracking: usize,
    shape: Option<ContentPath>,
}

impl Load {
    /// The param setter: params 1..=148, clamped. A shifted 218 block's params past the last are dropped.
    fn set(&mut self, param: usize, value: f32) {
        if param >= 1
            && let Some(p) = self.params.get_mut(param)
        {
            *p = value.clamp(0.0, 1.0);
        }
    }

    fn on(&mut self, param: usize, on: bool) {
        self.set(param, f32::from(u8::from(on)));
    }

    /// The header, then every event; `None` on an abort.
    fn file(&mut self, bytes: &[u8]) -> Option<()> {
        let header = bytes.get(..HEADER)?;
        let format = u16::from_le_bytes([header[8], header[9]]);
        if &header[..4] != b"FLhd" || !matches!(format, 0x20 | 0x30) || &header[14..18] != b"FLdt" {
            return None;
        }
        let mut rest = &bytes[HEADER..];
        while let Some((&id, after)) = rest.split_first() {
            let size = match id {
                0..=63 => 1,
                64..=127 => 2,
                128..=191 => 4,
                _ => {
                    let (len, n) = varint(after)?;
                    let data = after.get(n..)?.get(..len)?;
                    self.blob(id, data)?;
                    rest = &after[n + len..];
                    continue;
                }
            };
            let data = after.get(..size)?;
            match (id, data) {
                (16, &[flags]) => self.ping_pong(flags),
                (138, &[a, b, c, d]) => {
                    let raw = i32::from_le_bytes([a, b, c, d]);
                    self.set(135, (raw >> 16) as f32 / 256.0);
                    self.set(136, (raw & 0xFFFF) as f32 / 256.0);
                }
                _ => {}
            }
            rest = &after[size..];
        }
        Some(())
    }

    fn blob(&mut self, id: u8, data: &[u8]) -> Option<()> {
        match id {
            196 => self.shape = shape(data).or(self.shape.take()),
            199 => self.v = version(data)?,
            209 => self.delay(data),
            213 => self.oscs(data)?,
            215 => self.arp(data),
            218 => {
                let raws = ints(data);
                if raws.len() < 14 {
                    return None;
                }
                self.block(self.blocks, &raws);
                self.blocks += 1;
            }
            219 => self.filter_219(data),
            220 => self.filter_220(data)?,
            221 => self.voices(data),
            228 => self.tracking(data),
            229 => self.levels(data),
            _ => {}
        }
        Some(())
    }

    /// Byte 10 of 215, or u8 event 16: bit 1 Panning will bounce, bit 2 Fat Mode.
    fn ping_pong(&mut self, flags: u8) {
        self.on(137, flags & 2 != 0);
        self.on(138, flags & 4 != 0);
    }

    /// 213, the osc blob, by its first i32, the version.
    fn oscs(&mut self, d: &[u8]) -> Option<()> {
        let r = ints(d);
        let version = *r.first()?;
        if version <= 9 {
            // No version field: shape, coarse, fine, then the next osc's mix, for each osc (osc 3 no mix).
            if d.len() < 44 {
                return None;
            }
            for o in 0..3 {
                let at = |k: usize| r[4 * o + k] as f32;
                self.set(1 + o, at(0) / 6.0);
                self.set(13 + o, 0.5 + at(1) / 48.0);
                self.set(16 + o, 0.5 + at(2) / 200.0);
                if o < 2 {
                    self.set(22 + o, at(3) / 128.0);
                }
            }
            return Some(());
        }
        if d.len() < if version <= 11 { 84 } else { 92 } {
            return None;
        }
        // The 20 i32 after the version: per osc pan, shape, coarse, fine, phase, detune, then osc 1's and 2's level,
        // the next osc's Mix.
        for o in 0..3 {
            let at = |k: usize| r[1 + 7 * o + k] as f32;
            self.set(19 + o, 0.5 + at(0) / 128.0);
            self.set(1 + o, at(1) / 6.0);
            self.set(13 + o, 0.5 + at(2) / 48.0);
            self.set(16 + o, 0.5 + at(3) / 200.0);
            self.set(7 + o, 0.5 + at(4) / 128.0);
            self.set(10 + o, 0.5 + at(5) / 100.0);
            if o < 2 {
                self.set(22 + o, at(6) / 128.0);
            }
        }
        match version {
            10 => {}
            // Up to 3 Invert bytes.
            11 => {
                for (o, &b) in d[84..].iter().take(3).enumerate() {
                    self.on(4 + o, b != 0);
                }
            }
            _ => {
                self.set(26, r[21] as f32 / 64.0);
                for (param, &b) in [4, 5, 6, 24].into_iter().zip(&d[88..92]) {
                    self.on(param, b != 0);
                }
                if let Some(&hq) = d.get(92) {
                    self.on(25, hq & 1 != 0);
                }
                // 3 bytes skipped, the five 17-i32 blocks, then the filter; without all of it the load goes on.
                if version >= 14
                    && let Some(extra) = d.get(96..96 + 91 * 4)
                {
                    let extra = ints(extra);
                    for (k, block) in extra[..85].chunks(17).enumerate() {
                        self.block(k, block);
                    }
                    self.set(111, extra[87] as f32 / 256.0);
                    self.set(112, extra[88] as f32 / 256.0);
                    self.set(110, extra[90] as f32 / 7.0);
                }
            }
        }
        Some(())
    }

    /// An envelope/LFO block of 14 or more i32 as block `k`: 0 Pan (LFO only), 1 Volume, 2 ModX, 3 ModY, 4 Pitch;
    /// further ones write shifted params, unbounded.
    fn block(&mut self, k: usize, r: &[i32]) {
        let raw = |i: usize| r[i] as f32;
        self.set(42 + k, time(r[9]));
        self.set(47 + k, time(r[10]));
        // ponytail: blocks past Pitch take Volume's Amount law (none of their own is known).
        let amount = match k {
            0 => 128.0,
            4 => 4800.0,
            _ => 256.0,
        };
        self.set(52 + k, 0.5 + raw(11) / amount);
        self.set(57 + k, time(r[12]));
        self.set(37 + k, raw(13) / 2.0);
        let Some(e) = k.checked_sub(1) else {
            return;
        };
        let flags = r[0];
        self.on(66 + e, flags & 1 != 0);
        self.on(32 + k, flags & 2 != 0);
        self.on(27 + k, flags & 32 != 0);
        self.on(62 + e, r[1] != 0);
        self.set(70 + e, time(r[2]));
        self.set(74 + e, time(r[3]));
        self.set(82 + e, time(r[4]));
        self.set(86 + e, time(r[5]));
        self.set(94 + e, raw(6) / 128.0);
        self.set(98 + e, time(r[7]));
        // The Volume envelope's Amount is never imported.
        if k >= 2 {
            self.set(106 + e, 0.5 + raw(8) / 256.0);
        }
        // Tensions: attack, decay, release; from the flags (bits 3, 6, 4) without their i32.
        let tensions = match r.get(14..17) {
            Some(&[a, d, r]) => [a, d, r],
            _ => [8, 64, 16].map(|bit| if flags & bit != 0 { -101 } else { 0 }),
        };
        for (param, raw) in [78, 90, 102].into_iter().zip(tensions) {
            self.set(param + e, 0.5 + raw as f32 / 256.0);
        }
    }

    /// 219: pan, volume and pitch skipped; Cutoff, Resonance, Type. Only from 24 bytes.
    fn filter_219(&mut self, d: &[u8]) {
        if let &[_, _, _, cutoff, resonance, kind, ..] = &ints(d)[..] {
            self.set(111, cutoff as f32 / 256.0);
            self.set(112, resonance as f32 / 256.0);
            self.set(110, kind as f32 / 7.0);
        }
    }

    /// 220: Cutoff, Resonance, and from 2.5.1 Type.
    fn filter_220(&mut self, d: &[u8]) -> Option<()> {
        let typed = self.v >= 2_005_001;
        if d.len() < if typed { 12 } else { 8 } {
            return None;
        }
        let r = ints(d);
        self.set(111, r[0] as f32 / 256.0);
        let resonance = match self.v < 3_002_002 {
            true => 256 * i64::from(r[1]) / 80,
            false => r[1].into(),
        };
        self.set(112, resonance as f32 / 256.0);
        if typed {
            self.set(110, r[2] as f32 / 7.0);
        }
        Some(())
    }

    /// 221: Max Polyphony, Portamento Time; byte 8: bit 0 Monophonic, bit 1 Portamento.
    fn voices(&mut self, d: &[u8]) {
        let r = ints(d);
        if let Some(&voices) = r.first() {
            self.set(117, voices as f32 / 99.0);
        }
        if let Some(&raw) = r.get(1) {
            let raw = match self.v < 3_005_002 {
                true => log_time(raw.into()),
                false => raw.into(),
            };
            self.set(118, raw as f32 / 1660.0);
        }
        if let Some(&flags) = d.get(8) {
            self.on(120, flags & 1 != 0);
            self.on(119, flags & 2 != 0);
        }
    }

    /// 228: the first is velocity tracking, the second key tracking, later ones are ignored.
    fn tracking(&mut self, d: &[u8]) {
        let (middle, to) = match self.tracking {
            0 => (125, 126),
            1 => (121, 122),
            _ => return,
        };
        self.tracking += 1;
        let r = ints(d);
        if let Some(&raw) = r.first() {
            self.set(middle, raw as f32 / 120.0);
        }
        for (i, &raw) in r.iter().skip(1).take(3).enumerate() {
            self.set(to + i, 0.5 + raw as f32 / 512.0);
        }
    }

    /// Before 5.3.7 the levels' and delay's pan and volume (feedback) raws are ×100.
    fn scaled(&self, raw: i64) -> i64 {
        raw * if self.v < 5_003_007 { 100 } else { 1 }
    }

    /// 229: Pan Offset, Volume Multiplier, one skipped, Cutoff and Resonance Offsets.
    fn levels(&mut self, d: &[u8]) {
        let r: Vec<i64> = ints(d).into_iter().map(i64::from).collect();
        if let Some(&raw) = r.first() {
            self.set(113, 0.5 + self.scaled(raw) as f32 / 12800.0);
        }
        if let Some(&raw) = r.get(1) {
            self.set(114, self.scaled(raw) as f32 / 25600.0);
        }
        for (param, &raw) in [115, 116].into_iter().zip(r.iter().skip(3)) {
            self.set(param, 0.5 + raw as f32 / 512.0);
        }
    }

    /// 209, the echo: Feedback, Pan, Pitch, Echoes, Time.
    fn delay(&mut self, d: &[u8]) {
        let r: Vec<i64> = ints(d).into_iter().map(i64::from).collect();
        if let Some(&raw) = r.first() {
            self.set(131, self.scaled(raw) as f32 / 25600.0);
        }
        if let Some(&raw) = r.get(1) {
            self.set(133, (self.scaled(raw) + 6400) as f32 / 25600.0);
        }
        if let Some(&raw) = r.get(2) {
            let raw = if self.v < 3_003_001 {
                10 * raw - 1200
            } else {
                raw
            };
            self.set(134, 0.5 + raw as f32 / 2400.0);
        }
        if let Some(&raw) = r.get(3) {
            self.set(130, (raw - 1) as f32 / 9.0);
        }
        if let Some(&raw) = r.get(4) {
            self.set(132, raw as f32 / 384.0);
        }
    }

    /// 215, the arp: from 11 bytes, each field only when the blob reaches it.
    fn arp(&mut self, d: &[u8]) {
        let Some(&flags) = d.get(10) else {
            return;
        };
        self.ping_pong(flags);
        let at = |offset: usize| Some(i64::from(ints(d.get(offset..offset + 4)?)[0]));
        // Arp Time and Gate: (raw − 450)/997, before 3.5.2 from the log law clamped to 450..=1447.
        let old = self.v < 3_005_002;
        let time = |raw: i64| {
            let raw = match old {
                true => log_time(raw).clamp(450, 1447),
                false => raw,
            };
            (raw - 450) as f32 / 997.0
        };
        let fields = [
            (139, at(40).map(|raw| raw as f32 / 5.0)),
            (142, at(44).map(|raw| raw as f32 / 6.0)),
            (145, at(48).map(|raw| (raw + 2) as f32 / 90.0)),
            (140, at(52).map(time)),
            (141, at(56).map(|raw| raw as f32 / 48.0)),
            (144, d.get(60).map(|&b| f32::from(u8::from(b != 0)))),
            (129, at(64).map(time)),
            (143, at(92).map(|raw| (raw - 1) as f32 / 5.0)),
        ];
        for (param, value) in fields {
            if let Some(value) = value {
                self.set(param, value);
            }
        }
    }
}

/// The time law T(raw) = (raw − 100)/65436.
fn time(raw: i32) -> f32 {
    (raw as f32).mul_add(1.5282108e-5, -1.5282108e-3)
}

/// An older file's time: llround(ln(raw·13/24 + 1)·310.69498), in f32. A log of 0 or less saturates.
fn log_time(raw: i64) -> i64 {
    ((raw as f32 * 13.0 / 24.0 + 1.0).ln() * 310.69498).round() as i64
}

/// The whole little-endian i32 of `data`.
fn ints(data: &[u8]) -> Vec<i32> {
    (data.as_chunks::<4>().0.iter())
        .map(|c| i32::from_le_bytes([c[0], c[1], c[2], c[3]]))
        .collect()
}

/// A blob's length, 7 bits a byte, low first, at most 32 bits: (length, bytes read). `None` when cut short or longer.
fn varint(bytes: &[u8]) -> Option<(usize, usize)> {
    let mut value = 0u64;
    for (i, &b) in bytes.iter().take(5).enumerate() {
        value |= u64::from(b & 0x7F) << (7 * i);
        if b & 0x80 == 0 {
            return Some((usize::try_from(u32::try_from(value).ok()?).ok()?, i + 1));
        }
    }
    None
}

/// V of the version string "a.b.c" (more parts ignored, each a decimal integer, 0 when it is none); `None` for fewer
/// than three parts.
fn version(data: &[u8]) -> Option<i64> {
    let text = String::from_utf8_lossy(data.split(|&b| b == 0).next().unwrap_or_default());
    let parts: Vec<i64> = (text.split('.')).map(|p| p.parse().unwrap_or(0)).collect();
    let &[a, b, c, ..] = &parts[..] else {
        return None;
    };
    Some(
        a.saturating_mul(1_000_000)
            .saturating_add(b.saturating_mul(1000))
            .saturating_add(c),
    )
}

/// The custom waveform of a 196 path: `\` and `/` alike, runs collapsed, no trailing one; only
/// `/Instruments/Shapes/<rest>` names one, `@R@3xOsc Shapes/<rest>`.
// ponytail: the path is read as 8-bit text, as the bundled 3.x files write it; FL 11.5+ files write UTF-16 there.
fn shape(data: &[u8]) -> Option<ContentPath> {
    let text = String::from_utf8_lossy(data.split(|&b| b == 0).next().unwrap_or_default());
    let mut path = String::new();
    for c in text.chars().map(|c| if c == '\\' { '/' } else { c }) {
        if !(c == '/' && path.ends_with('/')) {
            path.push(c);
        }
    }
    let path = path.strip_suffix('/').unwrap_or(&path);
    let rest = path.strip_prefix("/Instruments/Shapes/")?;
    Some(ContentPath::Shorthand {
        root: ContentRoot::App,
        relative: format!("3xOsc Shapes/{rest}"),
    })
}

#[cfg(test)]
pub(crate) mod tests;
