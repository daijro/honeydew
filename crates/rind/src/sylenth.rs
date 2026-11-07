// Copyright (C) 2026 The Honeydew contributors
// SPDX-License-Identifier: GPL-3.0-or-later

//! Reads Sylenth1 presets (`.fxp` programs, `.fxb` / bank files) into Rind's parameters.
//!
//! This is format compatibility only, written from the publicly documented layout of the files (the VST2 `.fxp` / `.fxb`
//! container, and the community-documented parameter layout of Sylenth1's chunk): no Sylenth1 code or assets are used, and
//! the sound is Rind's own, so an imported preset is an *approximation* of the original. What follows is what is known and
//! what is assumed, so that nothing is silently wrong:
//!
//! - **Container.** Big-endian `CcnK` + size + `FPCh` (one program) or `FBCh` (a bank), the plugin id `syl1`, then the
//!   chunk: a 20-byte header, then one record per program of little-endian 32-bit floats ([`KNOWN_VERSIONS`]: 2011, 2202,
//!   2211) ending in the program's 36-byte name. Every parameter is a normalised `0..=1` float at a 4-byte slot; choices
//!   are stored as particular normalised values ([`nearest`]).
//! - **Scalings are estimated.** The files hold normalised positions, not Hz or seconds, and Sylenth1's knob tapers are not
//!   published. Times, frequencies and rates use plausible exponential curves over the documented display ranges, so
//!   levels, envelope times and cutoffs can differ from the original. Every import says so ([`APPROX`]).
//! - Anything with no Rind equivalent is listed in the preset's `warnings`, never dropped silently.
//!
//! The parser never panics on bad input: every read is bounds-checked and a file that does not fit the layout is an
//! [`Error`]. It runs on the UI or a worker thread, never in the audio callback.

use crate::ids::*;
use crate::presets::{Preset, clamp_plain};

/// The preset data versions this reads (Sylenth1 2.0 and the 2.2 series).
pub const KNOWN_VERSIONS: [u32; 3] = [2011, 2202, 2211];

/// The note every import carries.
pub const APPROX: &str = "Knob scalings are estimated (not calibrated against Sylenth1), so levels, times and cutoffs can differ.";

/// True when `bytes` start like a Sylenth1 program or bank (the VST2 container's magic, kind and plugin id): a cheap look,
/// for listing files, that reads no further. [`parse`] says whether the rest is sound.
pub fn sniff(bytes: &[u8]) -> bool {
    bytes.len() >= 56
        && bytes.starts_with(b"CcnK")
        && matches!(&bytes[8..12], b"FPCh" | b"FBCh")
        && &bytes[16..20] == b"syl1"
}

/// Why a file was not read.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Error {
    /// Too short to be a preset file.
    TooShort,
    /// Not a VST2 preset container.
    NotFxp,
    /// A container of something other than Sylenth1, or of a layout this does not know.
    Unsupported(String),
    /// Sylenth1's container with data that does not add up.
    Corrupt(String),
}

impl std::fmt::Display for Error {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Error::TooShort => write!(f, "The file is too short to be a preset."),
            Error::NotFxp => write!(f, "Not a Sylenth1 preset file (.fxp or .fxb)."),
            Error::Unsupported(s) => write!(f, "Unsupported preset: {s}"),
            Error::Corrupt(s) => write!(f, "The preset file is damaged: {s}"),
        }
    }
}

impl std::error::Error for Error {}

fn be32(b: &[u8], at: usize) -> Option<u32> {
    Some(u32::from_be_bytes(b.get(at..at + 4)?.try_into().ok()?))
}

fn le32(b: &[u8], at: usize) -> Option<u32> {
    Some(u32::from_le_bytes(b.get(at..at + 4)?.try_into().ok()?))
}

/// A NUL-terminated name, printable characters only.
fn text(b: &[u8]) -> String {
    let end = b.iter().position(|&c| c == 0).unwrap_or(b.len());
    let s: String = b[..end]
        .iter()
        .map(|&c| {
            if (32..127).contains(&c) {
                c as char
            } else if c >= 160 {
                char::from(c)
            } else {
                ' '
            }
        })
        .collect();
    s.trim().to_owned()
}

/// The smallest record: the parameters up to the last one (`YPartSelect`, slot 986) and the 36-byte name.
const MIN_RECORD: usize = (988 - 20) + 36;
/// The largest a record plausibly is.
const MAX_RECORD: usize = 1200;
/// The most programs a bank may hold.
const MAX_PROGRAMS: usize = 4096;

/// Reads every program in `bytes`: one for a `.fxp`, one per program of a bank.
pub fn parse(bytes: &[u8]) -> Result<Vec<Preset>, Error> {
    if bytes.len() < 60 {
        return Err(Error::TooShort);
    }
    if &bytes[0..4] != b"CcnK" {
        return Err(Error::NotFxp);
    }
    let kind: [u8; 4] = bytes[8..12].try_into().map_err(|_| Error::TooShort)?;
    let (id_at, count_at, name_at, size_at, data_at, bank) = match &kind {
        b"FPCh" => (16, 24, Some(28), 56, 60, false),
        b"FBCh" => (16, 24, None, 156, 160, true),
        b"FxCk" | b"FxBk" => {
            return Err(Error::Unsupported(
                "a parameter-list preset (this reads Sylenth1's chunk presets)".into(),
            ));
        }
        _ => return Err(Error::NotFxp),
    };
    if bytes.get(id_at..id_at + 4) != Some(b"syl1".as_slice()) {
        let id = text(bytes.get(id_at..id_at + 4).unwrap_or_default());
        return Err(Error::Unsupported(format!(
            "it belongs to the plugin \"{id}\", not Sylenth1"
        )));
    }
    let programs = be32(bytes, count_at).ok_or(Error::TooShort)? as usize;
    let size = be32(bytes, size_at).ok_or(Error::TooShort)? as usize;
    let data = bytes.get(data_at..).ok_or(Error::TooShort)?;
    // The size says how long the chunk is; a short file is damaged, a long one's tail is ignored.
    if size > data.len() {
        return Err(Error::Corrupt(
            "the file ends before the preset data does".into(),
        ));
    }
    let data = &data[..size.min(data.len())];
    if data.len() < 20 + MIN_RECORD {
        return Err(Error::Corrupt(format!(
            "the preset data is only {} bytes",
            data.len()
        )));
    }
    let mut warnings = Vec::new();
    if data.get(0..4) != Some(b"1lys".as_slice()) && data.get(0..4) != Some(b"syl1".as_slice()) {
        warnings.push("The preset header has an unexpected tag; read it anyway.".to_owned());
    }
    let version = le32(data, 4).unwrap_or(0);
    if !KNOWN_VERSIONS.contains(&version) {
        warnings.push(format!(
            "Unknown Sylenth1 data version {version}: read as the newest layout."
        ));
    }
    let new_layout = version != 2011;
    let n = if bank { programs } else { 1 };
    if n == 0 || n > MAX_PROGRAMS {
        return Err(Error::Corrupt(format!("{n} programs")));
    }
    let record = (data.len() - 20) / n;
    if !(MIN_RECORD..=MAX_RECORD).contains(&record) {
        return Err(Error::Unsupported(format!(
            "a program record of {record} bytes (expected about {})",
            MIN_RECORD + if new_layout { 4 } else { 0 }
        )));
    }
    let header_name = name_at.map(|a| text(&bytes[a..a + 28])).unwrap_or_default();
    let mut out = Vec::with_capacity(n);
    for k in 0..n {
        let base = 20 + k * record;
        let rec = &data[base..base + record];
        let name_field = text(&rec[record - 36..]);
        let name = if !bank && !header_name.is_empty() {
            header_name.clone()
        } else if !name_field.is_empty() {
            name_field
        } else {
            format!("Sylenth1 {}", k + 1)
        };
        let mut preset = map(&Rec { d: rec }, &name);
        preset.warnings.splice(0..0, warnings.iter().cloned());
        out.push(preset);
    }
    Ok(out)
}

/// One program's parameter floats: slot `off` (a template offset, rounded down to its 4-byte slot).
struct Rec<'a> {
    d: &'a [u8],
}

impl Rec<'_> {
    fn raw(&self, off: usize) -> Option<f32> {
        let at = (off & !3).checked_sub(20)?;
        let v = f32::from_bits(le32(self.d, at)?);
        v.is_finite().then_some(v)
    }
}

/// The index of the value in `table` nearest `n`, and how far off it was.
pub fn nearest(n: f32, table: &[f32]) -> (usize, f32) {
    let mut best = (0, f32::MAX);
    for (i, v) in table.iter().enumerate() {
        let d = (n - v).abs();
        if d < best.1 {
            best = (i, d);
        }
    }
    best
}

fn uniform(count: usize) -> Vec<f32> {
    (0..count).map(|i| i as f32 / (count - 1) as f32).collect()
}

// Sylenth1's stored choice values (normalised), by choice index in each list's documented order.
const ARP_MODES: [f32; 10] = [
    0.0,
    1.0 / 9.0,
    2.0 / 9.0,
    3.0 / 9.0,
    4.0 / 9.0,
    5.0 / 9.0,
    6.0 / 9.0,
    7.0 / 9.0,
    8.0 / 9.0,
    1.0,
];
/// Rind's arpeggiator mode for each of [`ARP_MODES`]: Up, Down, Up/Down, Up/Down 2, Down/Up, Down/Up 2, Random, Ordered, Step, Chord.
const ARP_MODE_MAP: [usize; 10] = [0, 1, 2, 4, 3, 5, 6, 7, 9, 8];
const FILTER_TYPES: [f32; 4] = [0.0, 1.0 / 3.0, 2.0 / 3.0, 1.0];
const FILTER_INPUTS: [f32; 3] = [0.0, 0.5, 1.0];
/// Sylenth1's modulation destinations by stored value, and Rind's target for each (`None`: no equivalent).
const DESTS: [(u32, Option<usize>); 28] = [
    (0x0000_0000, Some(mtgt::OFF)),
    (0x3D00_0000, Some(mtgt::LEVEL_A)),
    (0x3D80_0000, Some(mtgt::LEVEL_B)),
    (0x3DC0_0000, Some(mtgt::LEVEL_ALL)),
    (0x3E00_0000, Some(mtgt::PITCH_PART_A)),
    (0x3E20_0000, Some(mtgt::PITCH_PART_B)),
    (0x3E40_0000, Some(mtgt::PITCH_ALL)),
    (0x3E60_0000, Some(mtgt::PHASE_PART_A)),
    (0x3E80_0000, Some(mtgt::PHASE_PART_B)),
    (0x3E90_0000, Some(mtgt::PHASE_ALL)),
    (0x3EA0_0000, Some(mtgt::PAN_A)),
    (0x3EB0_0000, Some(mtgt::PAN_B)),
    (0x3EC0_0000, Some(mtgt::PAN_ALL)),
    (0x3ED0_0000, Some(mtgt::CUTOFF_1)),
    (0x3EE0_0000, Some(mtgt::CUTOFF_2)),
    (0x3EF0_0000, Some(mtgt::CUTOFF_ALL)),
    (0x3F00_0000, Some(mtgt::RESONANCE_1)),
    (0x3F08_0000, Some(mtgt::RESONANCE_2)),
    (0x3F10_0000, Some(mtgt::RESONANCE_ALL)),
    (0x3F18_0000, None),
    (0x3F20_0000, Some(mtgt::LEVEL_A)),
    (0x3F28_0000, Some(mtgt::LEVEL_B)),
    (0x3F30_0000, Some(mtgt::LEVEL_ALL)),
    (0x3F38_0000, Some(mtgt::LFO_1_RATE)),
    (0x3F40_0000, Some(mtgt::LFO_1_GAIN)),
    (0x3F48_0000, Some(mtgt::LFO_2_RATE)),
    (0x3F50_0000, Some(mtgt::LFO_2_GAIN)),
    (0x3F58_0000, None),
];
const DEST_NAMES: [&str; 28] = [
    "None",
    "Volume A",
    "Volume B",
    "Volume A+B",
    "Pitch A",
    "Pitch B",
    "Pitch A+B",
    "Phase A",
    "Phase B",
    "Phase A+B",
    "Pan A",
    "Pan B",
    "Pan A+B",
    "Cutoff A",
    "Cutoff B",
    "Cutoff A+B",
    "Reso A",
    "Reso B",
    "Reso A+B",
    "Phaser Freq",
    "Mix A",
    "Mix B",
    "Mix A+B",
    "LFO 1 Rate",
    "LFO 1 Gain",
    "LFO 2 Rate",
    "LFO 2 Gain",
    "Distortion Amount",
];
/// Rind's modulation source for each of Sylenth1's twelve (`uniform(12)`): None, Velocity, Mod Wheel, Key Track, Amp Env A,
/// Amp Env B, Mod Env 1, Mod Env 2, LFO 1, LFO 2, Aftertouch, Step Velocity.
const SOURCE_MAP: [usize; 12] = [
    msrc::OFF,
    msrc::VELOCITY,
    msrc::MOD_WHEEL,
    msrc::KEY,
    msrc::AMP_ENV_A,
    msrc::AMP_ENV_B,
    msrc::ENV_3,
    msrc::ENV_4,
    msrc::LFO_1,
    msrc::LFO_2,
    msrc::AFTERTOUCH,
    msrc::ARP_STEP,
];
/// Sylenth1's LFO waves (`uniform(11)`): Sine, Saw, Ramp, Triangle, Pulse, Half Pulse, Quarter Pulse, TriSaw, Lorenz, Sample & Hold, Random,
/// and Rind's wave for each (see [`LFO_APPROX`]).
const LFO_MAP: [usize; 11] = [0, 2, 3, 1, 4, 5, 6, 1, 8, 7, 8];
const LFO_NAMES: [&str; 11] = [
    "Sine",
    "Saw",
    "Ramp",
    "Triangle",
    "Pulse",
    "Half Pulse",
    "Quarter Pulse",
    "TriSaw",
    "Lorenz",
    "Sample & Hold",
    "Random",
];
/// Which of [`LFO_MAP`] stand in for a wave Rind does not have.
const LFO_APPROX: [bool; 11] = [
    false, false, false, false, false, false, false, true, true, false, true,
];

/// The note division (an index into [`DIV_BEATS`]) nearest `beats`, comparing in the log domain.
fn snap_div(beats: f32) -> usize {
    let l = beats.max(1e-4).ln();
    let mut best = (0, f32::MAX);
    for (i, b) in DIV_BEATS.iter().enumerate() {
        let d = (b.ln() - l).abs();
        if d < best.1 {
            best = (i, d);
        }
    }
    best.0
}

struct Mapper<'a> {
    rec: &'a Rec<'a>,
    v: Vec<f32>,
    w: Vec<String>,
    /// A parameter that was missing or out of range.
    bad: usize,
}

impl Mapper<'_> {
    /// The normalised value at `off`, `0..=1`.
    fn n(&mut self, off: usize) -> f32 {
        match self.rec.raw(off) {
            Some(v) if (-0.01..=1.01).contains(&v) => v.clamp(0.0, 1.0),
            Some(v) => {
                self.bad += 1;
                v.clamp(0.0, 1.0)
            }
            None => {
                self.bad += 1;
                0.0
            }
        }
    }

    fn on(&mut self, off: usize) -> bool {
        self.n(off) >= 0.5
    }

    fn set(&mut self, i: usize, plain: f32) {
        self.v[i] = clamp_plain(i, plain);
    }

    fn warn(&mut self, s: impl Into<String>) {
        let s = s.into();
        if !self.w.contains(&s) {
            self.w.push(s);
        }
    }

    /// The index of the choice at `off` among `table`'s values; warns for a value that is none of them.
    fn choice(&mut self, off: usize, table: &[f32], what: &str) -> usize {
        let n = self.n(off);
        let (i, d) = nearest(n, table);
        if d > 0.03 {
            self.warn(format!(
                "{what}: an unrecognised value; used the nearest choice."
            ));
        }
        i
    }
}

fn log_map(n: f32, lo: f32, hi: f32) -> f32 {
    lo * (hi / lo).powf(n.clamp(0.0, 1.0))
}

/// An envelope time in seconds for a knob position (estimated: 1 ms to 10 s).
fn env_time(n: f32) -> f32 {
    log_map(n, 0.001, 10.0)
}

/// Maps one program.
fn map(rec: &Rec, name: &str) -> Preset {
    let mut m = Mapper {
        rec,
        v: SPECS.iter().map(|s| s.default).collect(),
        w: Vec::new(),
        bad: 0,
    };
    m.w.push(APPROX.to_owned());

    // ---- Main ----
    let n = m.n(292);
    m.set(MASTER_VOL, n);
    let (poly, porta, bend) = (m.n(576), m.n(584), m.n(572));
    m.set(POLY, (poly * 16.0).round().max(1.0));
    m.set(PORTA, porta * porta * 2.0);
    m.set(BEND, 1.0 + (bend * 23.0).round());
    let slide = m.n(582) < 0.5;
    m.set(PORTA_MODE, slide as u8 as f32);
    let legato = m.on(342);
    m.set(MONO, legato as u8 as f32);
    let sync = m.on(612);

    // ---- Parts (mixer, amp envelopes) ----
    for (part, (mix, env)) in [(296usize, 20usize), (300, 36)].into_iter().enumerate() {
        let level = m.n(mix);
        let b = PART_BASE + part * PART_STRIDE;
        m.set(b + part::ON, (level > 0.0005) as u8 as f32);
        m.set(b + part::LEVEL, level);
        let (a, d, r, s) = (m.n(env), m.n(env + 4), m.n(env + 8), m.n(env + 12));
        m.set(b + part::ATT, env_time(a));
        m.set(b + part::DEC, env_time(d));
        m.set(b + part::SUS, s);
        m.set(b + part::REL, env_time(r));
    }

    // ---- Oscillators ----
    for (o, base) in [344usize, 392, 440, 488].into_iter().enumerate() {
        let b = OSC_BASE + o * OSC_STRIDE;
        let detune = m.n(base);
        let fine = m.n(base + 4);
        let invert = m.on(base + 8);
        let note = m.n(base + 12);
        let oct = m.n(base + 16);
        let pan = m.n(base + 20);
        let phase = m.n(base + 24);
        let retrig = m.on(base + 28);
        let stereo = m.n(base + 32);
        let voices = m.n(base + 36);
        let vol = m.n(base + 40);
        let wave = m.choice(base + 44, &uniform(8), "Oscillator wave");
        m.set(b + osc::WAVE, wave as f32);
        m.set(b + osc::VOICES, (voices * 8.0).round());
        m.set(b + osc::DETUNE, detune);
        m.set(b + osc::STEREO, stereo);
        m.set(b + osc::OCT, (oct * 6.0 - 3.0).round());
        m.set(b + osc::SEMI, (note * 14.0 - 7.0).round());
        m.set(b + osc::FINE, (fine * 2.0 - 1.0) * 100.0);
        m.set(b + osc::PAN, pan * 2.0 - 1.0);
        m.set(b + osc::VOL, vol);
        m.set(b + osc::PHASE, phase);
        m.set(b + osc::RAND, if retrig { 0.0 } else { 1.0 });
        m.set(b + osc::INV, invert as u8 as f32);
    }

    // ---- Filters ----
    let key = m.n(240);
    let warm = m.on(250);
    for (f, (cut, drive, input, res, ty, slope)) in [
        (188usize, 192usize, 198usize, 200usize, 204usize, 210usize),
        (212, 216, 222, 224, 228, 234),
    ]
    .into_iter()
    .enumerate()
    {
        let b = FILT_BASE + f * FILT_STRIDE;
        let kind = m.choice(ty, &FILTER_TYPES, "Filter type");
        let inp = m.choice(input, &FILTER_INPUTS, "Filter input");
        let (c, d, r, s) = (m.n(cut), m.n(drive), m.n(res), m.n(slope));
        m.set(b + filt::ON, (kind != 0) as u8 as f32);
        m.set(b + filt::TYPE, kind.saturating_sub(1) as f32);
        m.set(b + filt::SLOPE, (s >= 0.5) as u8 as f32);
        m.set(
            b + filt::CUTOFF,
            log_map(c, 1.0, 21341.28).clamp(20.0, 20_000.0),
        );
        m.set(b + filt::RES, r);
        m.set(
            b + filt::DRIVE,
            (d + if warm { 0.15 } else { 0.0 }).min(1.0),
        );
        m.set(b + filt::KEY, key);
        // Input: this part only, both parts, or none. Filter A hears part A first, B hears part B first.
        let (own, other) = if f == 0 {
            (filt::IN_A, filt::IN_B)
        } else {
            (filt::IN_B, filt::IN_A)
        };
        let (own_on, other_on) = match inp {
            0 => (1.0, 0.0),
            1 => (1.0, 1.0),
            _ => (0.0, 0.0),
        };
        m.set(b + own, own_on);
        m.set(b + other, other_on);
        m.set(b + filt::VOL, 1.0);
    }
    let (ctl_c, ctl_r) = (m.n(236), m.n(244));
    let (a_c, b_c, a_r, b_r) = (m.n(188), m.n(212), m.n(200), m.n(224));
    if (ctl_c - a_c).abs() > 0.02 && (ctl_c - b_c).abs() > 0.02 {
        m.warn("Filter Control cutoff differs from both filters' cutoffs and is not applied.");
    }
    if (ctl_r - a_r).abs() > 0.02 && (ctl_r - b_r).abs() > 0.02 {
        m.warn("Filter Control resonance differs from both filters' resonance and is not applied.");
    }

    // ---- Modulation envelopes ----
    for (e, base) in [304usize, 320].into_iter().enumerate() {
        let b = ENV_BASE + e * ENV_STRIDE;
        let (a, d, r, s) = (m.n(base), m.n(base + 4), m.n(base + 8), m.n(base + 12));
        m.set(b + env::ATT, env_time(a));
        m.set(b + env::DEC, env_time(d));
        m.set(b + env::SUS, s);
        m.set(b + env::REL, env_time(r));
    }

    // ---- LFOs ----
    for (l, base) in [254usize, 274].into_iter().enumerate() {
        let b = LFO_BASE + l * LFO_STRIDE;
        let free = m.on(base);
        let (gain, offset, rate) = (m.n(base + 2), m.n(base + 6), m.n(base + 10));
        let wi = m.choice(base + 14, &uniform(11), "LFO wave");
        if LFO_APPROX[wi] {
            m.warn(format!(
                "LFO {} wave {} is approximated by {}.",
                l + 1,
                LFO_NAMES[wi],
                LFO_WAVE_NAMES[LFO_MAP[wi]]
            ));
        }
        m.set(b + lfo::WAVE, LFO_MAP[wi] as f32);
        m.set(b + lfo::GAIN, gain);
        m.set(b + lfo::OFFSET, offset * 2.0 - 1.0);
        m.set(b + lfo::RETRIG, (!free) as u8 as f32);
        m.set(b + lfo::SYNC, sync as u8 as f32);
        m.set(b + lfo::RATE, log_map(rate, 0.01, 27.5).clamp(0.02, 40.0));
        // Note-value rates run from 8 dotted bars down to a 256th triplet.
        m.set(
            b + lfo::DIV,
            snap_div(48.0 * (0.0104 / 48.0f32).powf(rate)) as f32,
        );
    }

    // ---- Modulation matrix ----
    // Slots: Mod Env 1 (two destinations), Mod Env 2, LFO 1, LFO 2, then the four free-source slots.
    let slot_defs: [(Option<usize>, usize, usize, usize, usize); 8] = [
        (Some(msrc::ENV_3), 808, 812, 922, 926),
        (Some(msrc::ENV_4), 816, 820, 930, 934),
        (Some(msrc::LFO_1), 824, 828, 938, 942),
        (Some(msrc::LFO_2), 832, 836, 946, 950),
        (None, 840, 844, 954, 958),
        (None, 852, 856, 962, 966),
        (None, 864, 868, 970, 974),
        (None, 876, 880, 978, 982),
    ];
    let source_offsets = [848usize, 860, 872, 884];
    let dest_values: Vec<f32> = DESTS.iter().map(|d| f32::from_bits(d.0)).collect();
    for (k, (fixed, a1, a2, d1, d2)) in slot_defs.into_iter().enumerate() {
        let src = match fixed {
            Some(s) => s,
            None => {
                let si = m.choice(source_offsets[k - 4], &uniform(12), "Modulation source");
                SOURCE_MAP[si]
            }
        };
        for (j, (am, de)) in [(a1, d1), (a2, d2)].into_iter().enumerate() {
            let amount = m.n(am) * 2.0 - 1.0;
            let di = m.choice(de, &dest_values, "Modulation destination");
            let slot = k * 2 + j;
            let b = MAT_BASE + slot * MAT_STRIDE;
            match DESTS[di].1 {
                Some(t) if t != mtgt::OFF && amount.abs() > 0.001 && src != msrc::OFF => {
                    m.set(b + mat::SRC, src as f32);
                    m.set(b + mat::DST, t as f32);
                    m.set(b + mat::AMT, amount);
                }
                None if amount.abs() > 0.001 && src != msrc::OFF => {
                    m.warn(format!(
                        "Modulation of {} is not supported and was left out.",
                        DEST_NAMES[di]
                    ));
                }
                _ => {}
            }
        }
    }

    // ---- Arpeggiator ----
    let arp_on = m.on(890);
    m.set(ARP_ON, arp_on as u8 as f32);
    let mode = m.choice(56, &ARP_MODES, "Arpeggiator mode");
    if mode == 8 {
        m.warn("Arpeggiator mode Step is approximated by Thumb.");
    }
    m.set(ARP_MODE, ARP_MODE_MAP[mode] as f32);
    let (gate, octave, time, wrap) = (m.n(52), m.n(60), m.n(64), m.n(72));
    m.set(ARP_GATE, gate.max(0.05));
    m.set(ARP_OCT, 1.0 + (octave * 3.0).round());
    m.set(ARP_DIV, snap_div(4.0 * (1.0f32 / 64.0).powf(time)) as f32);
    let steps = (wrap * 16.0).round();
    m.set(ARP_STEPS, if steps < 1.0 { 16.0 } else { steps });
    let vmode = m.choice(70, &uniform(5), "Arpeggiator velocity");
    m.set(ARP_VELMODE, vmode as f32);
    for s in 0..16 {
        let hold = m.on(618 + 4 * s);
        let tr = m.n(680 + 4 * s);
        let vel = m.n(744 + 4 * s);
        m.set(ARP_HOLD_BASE + s, hold as u8 as f32);
        m.set(ARP_TR_BASE + s, (tr * 48.0 - 24.0).round());
        m.set(ARP_STEP_BASE + s, vel);
    }

    // ---- Effects ----
    let dist_on = m.on(906);
    m.set(DIST_ON, dist_on as u8 as f32);
    let ty = m.choice(170, &uniform(5), "Distortion type");
    let (amt, mix) = (m.n(160), m.n(164));
    m.set(DIST_TYPE, ty as f32);
    m.set(DIST_DRIVE, amt);
    m.set(DIST_MIX, mix);

    let phs_on = m.on(914);
    m.set(PHS_ON, phs_on as u8 as f32);
    let (freq, mix, fb, depth, rate, spread) =
        (m.n(536), m.n(540), m.n(544), m.n(548), m.n(552), m.n(560));
    m.set(PHS_FREQ, log_map(freq, 100.0, 8000.0));
    m.set(PHS_MIX, mix);
    m.set(PHS_FB, fb * 0.95);
    m.set(PHS_DEPTH, depth);
    // The rate is a note value; at 120 BPM a beat is half a second.
    m.set(
        PHS_RATE,
        (2.0 / (4.0 * (1.0f32 / 128.0).powf(rate))).clamp(0.02, 10.0),
    );
    m.set(PHS_SPREAD, spread);
    if phs_on {
        m.warn("Phaser width and L/R offset are not supported; the rate is converted at 120 BPM.");
    }

    let cho_on = m.on(894);
    m.set(CHO_ON, cho_on as u8 as f32);
    let (delay, depth, mix, fb, rate) = (m.n(76), m.n(80), m.n(84), m.n(88), m.n(96));
    m.set(CHO_DELAY, 1.0 + delay * 39.0);
    m.set(CHO_DEPTH, depth);
    m.set(CHO_MIX, mix);
    m.set(CHO_FB, fb * 0.9);
    m.set(CHO_RATE, log_map(rate, 0.01, 27.5));
    if cho_on {
        m.warn("Chorus width and single mode are not supported (always dual).");
    }

    let cmp_on = m.on(898);
    m.set(CMP_ON, cmp_on as u8 as f32);
    let (att, ratio, rel, thr) = (m.n(104), m.n(108), m.n(112), m.n(116));
    m.set(CMP_ATT, log_map(att, 0.1, 300.0));
    m.set(CMP_RATIO, log_map(ratio, 1.0, 100.0));
    m.set(CMP_REL, log_map(rel, 1.0, 500.0));
    m.set(CMP_THR, thr * 30.0 - 30.0);

    let dly_on = m.on(902);
    m.set(DLY_ON, dly_on as u8 as f32);
    let (mix, fb, hi, lo, smear, tl, tr) = (
        m.n(120),
        m.n(124),
        m.n(128),
        m.n(132),
        m.n(140),
        m.n(148),
        m.n(152),
    );
    let pp = m.on(138);
    m.set(DLY_MIX, mix);
    m.set(DLY_FB, fb * 0.95);
    m.set(DLY_LP, log_map(hi, 82.5, 21120.0));
    m.set(DLY_HP, log_map(lo, 3.23, 26483.12));
    m.set(DLY_PP, pp as u8 as f32);
    m.set(DLY_SMEAR, smear);
    m.set(DLY_SYNC, 1.0);
    m.set(DLY_DIV_L, snap_div(0.0625 * 32f32.powf(tl)) as f32);
    m.set(DLY_DIV_R, snap_div(0.0625 * 32f32.powf(tr)) as f32);
    if dly_on {
        m.warn("Delay spread and width are not supported.");
    }

    let eq_on = m.on(910);
    m.set(EQ_ON, eq_on as u8 as f32);
    let (bass, bf, treble, tf) = (m.n(172), m.n(176), m.n(180), m.n(184));
    m.set(EQ_BASS_G, bass * 15.0);
    m.set(EQ_BASS_F, log_map(bf, 13.75, 880.0));
    m.set(EQ_TREB_G, treble * 15.0);
    m.set(EQ_TREB_F, log_map(tf, 440.0, 28160.0));

    let rev_on = m.on(918);
    m.set(REV_ON, rev_on as u8 as f32);
    let (damp, mix, pre, size, width) = (m.n(588), m.n(592), m.n(596), m.n(600), m.n(604));
    m.set(REV_DAMP, damp);
    m.set(REV_MIX, mix);
    m.set(REV_PRE, pre * 0.2);
    m.set(REV_SIZE, size);
    m.set(REV_WIDTH, width);

    if m.bad > 0 {
        let bad = m.bad;
        m.warn(format!(
            "{bad} value(s) were missing or out of range and were clamped."
        ));
    }
    Preset {
        name: name.to_owned(),
        source: "Sylenth1".to_owned(),
        values: m.v,
        warnings: m.w,
    }
}

/// Builds a Sylenth1 `.fxp` / bank file from `programs` of (name, floats by template offset), for tests. Anything not
/// listed is 0. `new_layout` picks the 2202/2211 record (with the extra 4 bytes before the name).
pub fn build_fixture(
    programs: &[(&str, Vec<(usize, f32)>)],
    bank: bool,
    new_layout: bool,
) -> Vec<u8> {
    let record = (988 - 20) + if new_layout { 4 } else { 0 } + 36;
    let mut data = Vec::new();
    data.extend_from_slice(b"1lys");
    data.extend_from_slice(&(if new_layout { 2211u32 } else { 2011 }).to_le_bytes());
    data.extend_from_slice(&1u32.to_le_bytes());
    data.extend_from_slice(&(programs.len() as u32).to_le_bytes());
    data.extend_from_slice(&(if new_layout { 2211u32 } else { 2011 }).to_le_bytes());
    for (name, floats) in programs {
        let mut rec = vec![0u8; record];
        for (off, v) in floats {
            let at = (off & !3) - 20;
            rec[at..at + 4].copy_from_slice(&v.to_le_bytes());
        }
        let nb = name.as_bytes();
        let n = nb.len().min(35);
        rec[record - 36..record - 36 + n].copy_from_slice(&nb[..n]);
        data.extend_from_slice(&rec);
    }
    let mut out = Vec::new();
    out.extend_from_slice(b"CcnK");
    out.extend_from_slice(&[0; 4]);
    out.extend_from_slice(if bank { b"FBCh" } else { b"FPCh" });
    out.extend_from_slice(&1u32.to_be_bytes());
    out.extend_from_slice(b"syl1");
    out.extend_from_slice(&1u32.to_be_bytes());
    out.extend_from_slice(&(programs.len() as u32).to_be_bytes());
    if bank {
        out.extend_from_slice(&[0; 128]);
    } else {
        let mut name = [0u8; 28];
        let nb = programs[0].0.as_bytes();
        name[..nb.len().min(27)].copy_from_slice(&nb[..nb.len().min(27)]);
        out.extend_from_slice(&name);
    }
    out.extend_from_slice(&(data.len() as u32).to_be_bytes());
    out.extend_from_slice(&data);
    let size = (out.len() - 8) as u32;
    out[4..8].copy_from_slice(&size.to_be_bytes());
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    fn one(floats: Vec<(usize, f32)>) -> Preset {
        parse(&build_fixture(&[("Test", floats)], false, true))
            .unwrap()
            .remove(0)
    }

    #[test]
    fn the_enum_tables_are_sorted_and_complete() {
        for t in [&ARP_MODES[..], &FILTER_TYPES[..], &FILTER_INPUTS[..]] {
            assert!(t.windows(2).all(|w| w[0] < w[1]));
        }
        let d: Vec<f32> = DESTS.iter().map(|d| f32::from_bits(d.0)).collect();
        assert!(d.windows(2).all(|w| w[0] < w[1]), "{d:?}");
        assert_eq!(DESTS.len(), DEST_NAMES.len());
    }

    #[test]
    fn a_program_parses_with_its_name_and_the_estimate_note() {
        let p = one(vec![]);
        assert_eq!((p.name.as_str(), p.source.as_str()), ("Test", "Sylenth1"));
        assert_eq!(p.values.len(), COUNT);
        assert!(p.warnings.iter().any(|w| w == APPROX));
    }

    #[test]
    fn oscillators_map_wave_voices_tuning_and_levels() {
        let p = one(vec![
            (388, 1.0 / 7.0),        // A1 wave: Saw
            (382, 1.0),              // 8 voices
            (344, 0.4),              // detune
            (376, 0.75),             // stereo
            (360, 5.0 / 6.0),        // octave +2
            (356, 0.5 + 3.0 / 14.0), // note +3
            (348, 0.75),             // fine +50 ct
            (384, 0.6),              // volume
            (354, 1.0),              // invert
            (374, 1.0),              // retrig on
            (436, 3.0 / 7.0),        // A2 wave: Pulse
            (430, 0.5),              // 4 voices
        ]);
        let b = OSC_BASE;
        assert_eq!(p.values[b + osc::WAVE], 1.0);
        assert_eq!(p.values[b + osc::VOICES], 8.0);
        assert!((p.values[b + osc::DETUNE] - 0.4).abs() < 1e-6);
        assert!((p.values[b + osc::STEREO] - 0.75).abs() < 1e-6);
        assert_eq!(p.values[b + osc::OCT], 2.0);
        assert_eq!(p.values[b + osc::SEMI], 3.0);
        assert!((p.values[b + osc::FINE] - 50.0).abs() < 1e-3);
        assert!((p.values[b + osc::VOL] - 0.6).abs() < 1e-6);
        assert_eq!(p.values[b + osc::INV], 1.0);
        assert_eq!(p.values[b + osc::RAND], 0.0);
        let b2 = OSC_BASE + OSC_STRIDE;
        assert_eq!(
            (p.values[b2 + osc::WAVE], p.values[b2 + osc::VOICES]),
            (3.0, 4.0)
        );
    }

    #[test]
    fn filters_map_type_slope_input_and_bypass() {
        let p = one(vec![
            (204, 1.0 / 3.0), // A: lowpass
            (210, 1.0),       // 24 dB
            (198, 0.5),       // A + B
            (228, 1.0),       // B: highpass
            (222, 1.0),       // B hears nothing
            (188, 0.5),
            (200, 0.3),
        ]);
        let a = FILT_BASE;
        let b = FILT_BASE + FILT_STRIDE;
        assert_eq!(
            (
                p.values[a + filt::ON],
                p.values[a + filt::TYPE],
                p.values[a + filt::SLOPE]
            ),
            (1.0, 0.0, 1.0)
        );
        assert_eq!(
            (p.values[a + filt::IN_A], p.values[a + filt::IN_B]),
            (1.0, 1.0)
        );
        assert_eq!(
            (p.values[b + filt::ON], p.values[b + filt::TYPE]),
            (1.0, 2.0)
        );
        assert_eq!(
            (p.values[b + filt::IN_A], p.values[b + filt::IN_B]),
            (0.0, 0.0)
        );
        assert!((p.values[a + filt::RES] - 0.3).abs() < 1e-6);
        assert!(p.values[a + filt::CUTOFF] > 20.0 && p.values[a + filt::CUTOFF] < 20_000.0);
        // Bypass turns a filter off.
        let q = one(vec![(204, 0.0)]);
        assert_eq!(q.values[a + filt::ON], 0.0);
    }

    #[test]
    fn modulation_routing_reads_sources_destinations_and_amounts() {
        let p = one(vec![
            (808, 1.0),                         // Mod Env 1, destination 1, +1
            (922, f32::from_bits(0x3ED0_0000)), // -> Cutoff A
            (824, 0.75),                        // LFO 1, destination 1, +0.5
            (938, f32::from_bits(0x3E40_0000)), // -> Pitch A+B
            (840, 0.25),                        // misc 1A, destination 1, -0.5
            (954, f32::from_bits(0x3F00_0000)), // -> Reso A
            (848, 1.0 / 11.0),                  // misc source: Velocity
            (844, 1.0),                         // misc 1A destination 2
            (958, f32::from_bits(0x3F58_0000)), // -> Distortion Amount (unsupported)
        ]);
        let slot = |k: usize| {
            let b = MAT_BASE + k * MAT_STRIDE;
            (
                p.values[b + mat::SRC] as usize,
                p.values[b + mat::DST] as usize,
                p.values[b + mat::AMT],
            )
        };
        assert_eq!(slot(0), (msrc::ENV_3, mtgt::CUTOFF_1, 1.0));
        assert_eq!(slot(4), (msrc::LFO_1, mtgt::PITCH_ALL, 0.5));
        assert_eq!(slot(8), (msrc::VELOCITY, mtgt::RESONANCE_1, -0.5));
        assert!(
            p.warnings.iter().any(|w| w.contains("Distortion Amount")),
            "{:?}",
            p.warnings
        );
    }

    #[test]
    fn arpeggiator_effects_and_voice_settings_map() {
        let p = one(vec![
            (890, 1.0),  // arp on
            (56, 1.0),   // chord
            (60, 1.0),   // 4 octaves
            (72, 0.5),   // 8 steps
            (680, 0.75), // step 1 transpose +12
            (744, 0.5),  // step 1 velocity
            (618, 1.0),  // step 1 tie
            (906, 1.0),  // distortion on
            (170, 1.0),  // bitcrush
            (902, 1.0),  // delay on
            (138, 1.0),  // ping-pong
            (918, 1.0),  // reverb on
            (342, 1.0),  // mono legato
            (576, 0.5),  // 8 voices of polyphony
            (572, 1.0),  // bend 24
        ]);
        assert_eq!(
            (
                p.values[ARP_ON],
                p.values[ARP_MODE],
                p.values[ARP_OCT],
                p.values[ARP_STEPS]
            ),
            (1.0, 8.0, 4.0, 8.0)
        );
        assert_eq!(
            (p.values[ARP_TR_BASE], p.values[ARP_HOLD_BASE]),
            (12.0, 1.0)
        );
        assert!((p.values[ARP_STEP_BASE] - 0.5).abs() < 1e-6);
        assert_eq!((p.values[DIST_ON], p.values[DIST_TYPE]), (1.0, 4.0));
        assert_eq!(
            (p.values[DLY_ON], p.values[DLY_PP], p.values[DLY_SYNC]),
            (1.0, 1.0, 1.0)
        );
        assert_eq!(p.values[REV_ON], 1.0);
        assert_eq!(
            (p.values[MONO], p.values[POLY], p.values[BEND]),
            (1.0, 8.0, 24.0)
        );
    }

    #[test]
    fn every_mapped_value_is_in_range() {
        // Extreme but valid floats everywhere.
        for fill in [0.0f32, 0.5, 1.0] {
            let floats: Vec<(usize, f32)> = (5..247).map(|s| (s * 4, fill)).collect();
            let p = one(floats);
            for (i, v) in p.values.iter().enumerate() {
                assert!(v.is_finite(), "{}", SPECS[i].id);
                assert_eq!(
                    *v,
                    clamp_plain(i, *v),
                    "{} = {v} at fill {fill}",
                    SPECS[i].id
                );
            }
        }
    }

    #[test]
    fn banks_give_every_program_with_its_own_name() {
        let progs: Vec<(String, Vec<(usize, f32)>)> = (0..40)
            .map(|i| (format!("Preset {i}"), vec![(384usize, i as f32 / 40.0)]))
            .collect();
        let refs: Vec<(&str, Vec<(usize, f32)>)> =
            progs.iter().map(|(n, f)| (n.as_str(), f.clone())).collect();
        for new_layout in [false, true] {
            let all = parse(&build_fixture(&refs, true, new_layout)).unwrap();
            assert_eq!(all.len(), 40);
            assert_eq!(all[7].name, "Preset 7");
            assert!((all[20].values[OSC_BASE + osc::VOL] - 0.5).abs() < 1e-6);
        }
    }

    #[test]
    fn old_layout_files_read_too() {
        let p = parse(&build_fixture(&[("Old", vec![(382, 1.0)])], false, false))
            .unwrap()
            .remove(0);
        assert_eq!(
            (p.name.as_str(), p.values[OSC_BASE + osc::VOICES]),
            ("Old", 8.0)
        );
    }

    #[test]
    fn bad_files_are_errors_not_panics() {
        assert_eq!(parse(&[]), Err(Error::TooShort));
        assert_eq!(parse(&[0; 100]), Err(Error::NotFxp));
        let good = build_fixture(&[("X", vec![])], false, true);
        // Truncated at every length: never a panic.
        for len in 0..good.len() {
            let _ = parse(&good[..len]);
        }
        // Flipped bytes everywhere: never a panic.
        for i in (0..good.len()).step_by(7) {
            let mut bad = good.clone();
            bad[i] ^= 0xFF;
            let _ = parse(&bad);
        }
        // Another plugin's preset.
        let mut other = good.clone();
        other[16..20].copy_from_slice(b"abcd");
        assert!(matches!(parse(&other), Err(Error::Unsupported(_))));
        // A parameter-list preset.
        let mut fxck = good.clone();
        fxck[8..12].copy_from_slice(b"FxCk");
        assert!(matches!(parse(&fxck), Err(Error::Unsupported(_))));
        // A size that overruns the file.
        let mut big = good.clone();
        big[56..60].copy_from_slice(&u32::MAX.to_be_bytes());
        assert!(matches!(parse(&big), Err(Error::Corrupt(_))));
        // NaN and absurd floats are clamped and reported.
        let nan = parse(&build_fixture(
            &[("N", vec![(384, f32::NAN), (388, 1.0e30)])],
            false,
            true,
        ))
        .unwrap()
        .remove(0);
        assert!(nan.warnings.iter().any(|w| w.contains("clamped")));
        assert!(nan.values.iter().all(|v| v.is_finite()));
    }

    #[test]
    fn a_bank_of_the_largest_size_parses_quickly() {
        let progs: Vec<(String, Vec<(usize, f32)>)> = (0..MAX_PROGRAMS)
            .map(|i| (format!("P{i}"), vec![(384usize, 0.5)]))
            .collect();
        let refs: Vec<(&str, Vec<(usize, f32)>)> =
            progs.iter().map(|(n, f)| (n.as_str(), f.clone())).collect();
        let bytes = build_fixture(&refs, true, true);
        let t = std::time::Instant::now();
        assert_eq!(parse(&bytes).unwrap().len(), MAX_PROGRAMS);
        assert!(t.elapsed().as_secs() < 5);
    }

    #[test]
    fn sniff_tells_sylenth1_files_by_their_header() {
        let program = build_fixture(&[("A", vec![])], false, false);
        let bank = build_fixture(&[("A", vec![]), ("B", vec![])], true, true);
        assert!(sniff(&program) && sniff(&bank));
        assert!(!sniff(&program[..40]));
        assert!(!sniff(b"junk"));
        let mut other = program.clone();
        other[16..20].copy_from_slice(b"abcd");
        assert!(!sniff(&other));
    }
}
