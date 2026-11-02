// Copyright (C) 2026 ItsAlphaNeon
// SPDX-License-Identifier: GPL-3.0-or-later
// This file is part of Lemon Studio: https://github.com/ItsAlphaNeon/Lemon-Studio
// Modified for Honeydew: ported from Lemon Studio (219ed4e): the new-mixer writer of `crates/os-flp/src/project.rs`
// (`mixer_events`, `track_block_events`, `track_records`, `slot_plugin_events`, `plugin_record`) reduced to what an
// export writes (named tracks of slot plugins, every track routed to the master), and Fruity Parametric EQ 2's saved
// state image from `plugins/lemon-parametric-eq-2/src/params.rs` (`default_state`, `fields_of`, `to_fl`).

//! A new project's mixer (the events after the arrangements): the master, the inserts and the hidden last track FL
//! stores, each with its slot plugins; and Fruity Parametric EQ 2's saved state ([`eq2_state`]).

use super::{Event, Value};

/// A mixer track: its name and its slot plugins (at most [`SLOTS`]).
#[derive(Clone, Debug, Default, PartialEq)]
pub struct Track {
    pub name: Option<String>,
    pub slots: Vec<Slot>,
}

/// A slot plugin: its internal name, its saved state (empty: the plugin starts at its defaults) and its switch.
#[derive(Clone, Debug, PartialEq)]
pub struct Slot {
    pub plugin: String,
    pub state: Vec<u8>,
    pub on: bool,
}

/// Slots a track holds.
pub const SLOTS: usize = 10;

const MX_COUNT: u8 = 103;
const MX_COLOR_FLAG: u8 = 42;
const MX_NAME: u8 = 204;
const MX_FLAGS: u8 = 236;
const MX_SLOT: u8 = 98;
const MX_ROUTES: u8 = 235;
const MX_PARAMS: u8 = 225;
const PLUGIN_NAME: u8 = 201;
const PLUGIN_REC: u8 = 212;
const PLUGIN_STATE: u8 = 213;
const PLUGIN_ICON: u8 = 155;
const COLOR: u8 = 128;
const COLOR_FLAG: u8 = 41;

const MX_KIND_PARAM: u8 = 0x1F;
const MX_VOLUME: u8 = 0xC0;
const MX_PAN: u8 = 0xC1;
const MX_STEREO: u8 = 0xC2;
const MX_SLOT_ON: u8 = 0;
const MX_SLOT_MIX: u8 = 1;
const SLOT_MIX_FULL: i32 = 12800;
/// The record at location 0x4000 (the main volume) and the parameter block of "track 501", the last stored track's.
const MX_SPECIAL_LOC: u16 = 0x4000;
const MX_SPECIAL_TRACK: u16 = 501;
const MAIN_VOLUME: i32 = 12800;
/// Track flags (event 236) byte 4 of the master and the last track, and of an insert; with the slots and the track on.
const MX_FLAGS_MASTER: u8 = 0x0C;
const MX_FLAGS_INSERT: u8 = 0x4C;
/// The colour FL stores for most slot plugins.
const SLOT_COLOR: u32 = 0x0056_5148;

/// Track parameter records of a default track, in file order: unity volume (0.8 of 16000), centred, no separation,
/// and the equaliser's flat defaults.
const TRACK_PARAMS: [(u8, i32); 12] = [
    (MX_VOLUME, 12800),
    (MX_PAN, 0),
    (MX_STEREO, 0),
    (0xD0, 0),
    (0xD1, 0),
    (0xD2, 0),
    (0xD8, 5777),
    (0xD9, 33145),
    (0xDA, 55825),
    (0xE0, 17500),
    (0xE1, 17500),
    (0xE2, 17500),
];

/// The mixer of `tracks` (the master first, then the inserts), with FL's hidden last track after them: the track
/// count (event 103), each track's block, then the parameter records (event 225).
pub fn events(tracks: &[Track]) -> Vec<Event> {
    let hidden = Track::default();
    let all: Vec<&Track> = tracks.iter().chain([&hidden]).collect();
    let last = all.len() - 1;
    let mut ev = vec![Event::new(MX_COUNT, Value::U16(all.len() as u16))];
    for (n, t) in all.iter().enumerate() {
        ev.extend(track_block(t, n as u16, n == last));
    }
    let mut recs = param_rec(0, 0, MX_SPECIAL_LOC, MAIN_VOLUME).to_vec();
    for (n, t) in all.iter().enumerate().take(last) {
        recs.extend(track_records(n as u16, t));
    }
    // The last track's records sit at track 501.
    recs.extend(track_records(MX_SPECIAL_TRACK, &hidden));
    ev.push(Event::new(MX_PARAMS, Value::var(recs)));
    ev
}

/// One track's block: its name, flags, the ten slots, its routing (the master; the master and the last track
/// nowhere) and the trailing events every untouched track has.
fn track_block(t: &Track, index: u16, last: bool) -> Vec<Event> {
    let mut ev = vec![Event::new(MX_COLOR_FLAG, Value::U8(0))];
    if let Some(name) = t.name.as_ref().filter(|n| !n.is_empty()) {
        ev.push(Event::new(MX_NAME, Value::var(utf16(name))));
    }
    let mut flags = [0u8; 12];
    flags[4] = if index == 0 || last {
        MX_FLAGS_MASTER
    } else {
        MX_FLAGS_INSERT
    };
    ev.push(Event::new(MX_FLAGS, Value::var(flags.to_vec())));
    let at = if last && index > 0 {
        MX_SPECIAL_TRACK
    } else {
        index
    };
    for k in 0..SLOTS {
        if let Some(slot) = t.slots.get(k) {
            ev.extend(slot_events(slot, at, k as u8));
        }
        ev.push(Event::new(MX_SLOT, Value::U16(k as u16)));
    }
    let routes = if index == 0 || last { vec![0] } else { vec![1] };
    ev.push(Event::new(MX_ROUTES, Value::var(routes)));
    ev.push(Event::new(165, Value::U32(3)));
    ev.push(Event::new(166, Value::U32(1)));
    ev.push(Event::new(49, Value::U8(0)));
    ev.push(Event::new(154, Value::U32(u32::MAX)));
    ev.push(Event::new(
        147,
        Value::U32(if index == 0 { 0 } else { u32::MAX }),
    ));
    ev
}

/// A new slot plugin's events: its name, window record (its track and slot), icon and colour, and its state when it
/// has one.
fn slot_events(slot: &Slot, track: u16, k: u8) -> Vec<Event> {
    let mut rec = vec![0u8; 52];
    rec[0..4].copy_from_slice(&u32::from(track).to_le_bytes());
    rec[4..8].copy_from_slice(&u32::from(k).to_le_bytes());
    rec[8] = 2;
    rec[16..20].copy_from_slice(&0x140u32.to_le_bytes());
    let mut ev = vec![
        Event::new(PLUGIN_NAME, Value::var(utf16(&slot.plugin))),
        Event::new(PLUGIN_REC, Value::var(rec)),
        Event::new(PLUGIN_ICON, Value::U32(0)),
        Event::new(COLOR, Value::U32(SLOT_COLOR)),
        Event::new(COLOR_FLAG, Value::U8(0)),
    ];
    if !slot.state.is_empty() {
        ev.push(Event::new(PLUGIN_STATE, Value::var(slot.state.clone())));
    }
    ev
}

fn param_rec(pid: u8, kind: u8, loc: u16, v: i32) -> [u8; 12] {
    let mut r = [0u8; 12];
    r[4] = pid;
    r[5] = kind;
    r[6..8].copy_from_slice(&loc.to_le_bytes());
    r[8..12].copy_from_slice(&v.to_le_bytes());
    r
}

/// Track `t`'s parameter records in file order: each slot's switch and mix, then the track parameters.
fn track_records(t: u16, track: &Track) -> Vec<u8> {
    let loc = |slot: u16| 0x7000 + t * 64 + slot;
    let mut out = Vec::new();
    for k in 0..SLOTS {
        let on = track.slots.get(k).is_none_or(|s| s.on);
        out.extend(param_rec(
            MX_SLOT_ON,
            MX_KIND_PARAM,
            loc(k as u16),
            i32::from(on),
        ));
        out.extend(param_rec(
            MX_SLOT_MIX,
            MX_KIND_PARAM,
            loc(k as u16),
            SLOT_MIX_FULL,
        ));
    }
    for (pid, v) in TRACK_PARAMS {
        out.extend(param_rec(pid, MX_KIND_PARAM, loc(0), v));
    }
    out
}

fn utf16(s: &str) -> Vec<u8> {
    let mut v: Vec<u8> = s.encode_utf16().flat_map(u16::to_le_bytes).collect();
    v.extend([0, 0]);
    v
}

/// One Fruity Parametric EQ 2 band: level in dB (±18), frequency position (0..1 = 20 Hz..20 kHz, `20·1000^n`), width
/// (0..1), type (0 off, 1 low pass, 2 band pass, 3 high pass, 4 band stop, 5 low shelf, 6 peaking, 7 high shelf).
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Band {
    pub db: f64,
    pub freq: f64,
    pub width: f64,
    pub kind: i32,
}

pub const LOW_SHELF: i32 = 5;
pub const PEAKING: i32 = 6;
pub const HIGH_SHELF: i32 = 7;
const EQ_BANDS: usize = 7;
const EQ_STATE_LEN: usize = 354;
const EQ_STATE_VERSION: i32 = 8;
const EQ_DEFAULT_KINDS: [i32; EQ_BANDS] = [
    LOW_SHELF, PEAKING, PEAKING, PEAKING, PEAKING, PEAKING, HIGH_SHELF,
];

/// The stored width a band of type `kind` resets to.
fn default_width(kind: i32) -> i32 {
    match kind {
        1 | 3 => 36068,
        LOW_SHELF | HIGH_SHELF => 40124,
        _ => 25412,
    }
}

/// Fruity Parametric EQ 2's saved state (354 bytes, version 8) of a new instance with `bands` in its first bands (the
/// rest at their defaults, order "2", main level 0 dB). Fields 1..36 are the parameters (7 levels in 0.01 dB, 7
/// frequencies and 7 widths ×65536, 7 types, 7 orders, the main level), 37..72 the spare bank, 73..87 the options.
pub fn eq2_state(bands: &[Band]) -> Vec<u8> {
    let mut b = vec![0u8; EQ_STATE_LEN];
    let mut set = |k: usize, v: i32| b[4 * k..4 * k + 4].copy_from_slice(&v.to_le_bytes());
    set(0, EQ_STATE_VERSION);
    let mut fields = [0i32; 36];
    for (k, kind) in EQ_DEFAULT_KINDS.into_iter().enumerate() {
        fields[7 + k] = ((k as f64 + 1.5) / 9.0 * 65536.0).round() as i32;
        fields[14 + k] = default_width(kind);
        fields[21 + k] = kind;
    }
    // The spare bank holds the defaults.
    for (i, v) in fields.into_iter().enumerate() {
        set(37 + i, v);
    }
    for (k, band) in bands.iter().take(EQ_BANDS).enumerate() {
        fields[k] = (band.db.clamp(-18.0, 18.0) * 100.0).round() as i32;
        fields[7 + k] = (band.freq.clamp(0.0, 1.0) * 65536.0).round() as i32;
        fields[14 + k] = (band.width.clamp(0.0, 1.0) * 65536.0).round() as i32;
        fields[21 + k] = band.kind;
    }
    for (i, v) in fields.into_iter().enumerate() {
        set(1 + i, v);
    }
    let options = [
        1, 0, 513, 0, 512, 0, 8_347_648, 65536, 0, 196_608, 65536, 131_072, 131_072, 131_072, 0,
    ];
    for (i, v) in options.into_iter().enumerate() {
        set(73 + i, v);
    }
    b
}
