//! The SoundFont Player (ID 52): the SoundFont 2 file of the public
//! SoundFont 2.01 standard, and its voices.
//!
//! [`SoundFont::parse`] runs on the app thread (bytes in, no I/O) and keeps the file: the 16-bit samples are read
//! from it in place. It survives hostile input: every size and index is checked, and what it allocates is bounded by
//! the file's length. [`SoundFontPlayer`] renders on the audio thread without allocating.
//!
//! An SFZ ([`crate::sfz`]) plays through the same player as a SoundFont of one patch ([`SoundFont::sfz`]): its regions
//! read decoded samples (mono or stereo, each at its own rate) instead of `smpl`, follow the SFZ's trigger, round-robin,
//! random, keyswitch, controller, crossfade, choke and polyphony rules, and the SFZ's DAHDSR envelope unless the module's
//! Custom envelope is on. An SFZ voice also runs its region's filter, LFOs, envelopes and CC routes, a control block
//! ([`CONTROL`] frames) at a time; the CCs come in as channel params 7 + n ([`Instrument::set_controller`]). A
//! SoundFont's regions play exactly as before.
//!
//! A Honeydew addition (#102): a SoundFont's vibrato LFO (its
//! vibLfoToPitch, freqVibLFO and delayVibLFO) moves its voices' pitch, the mod wheel deepening it by up to 50 cents as
//! the standard's default modulator says; the module's own Vib Rate, Depth and Delay add a vibrato to every voice; and
//! Quality High reads the samples by cubic interpolation instead of linear. Transpose moves every voice by whole
//! semitones, with Fine Tune.

use std::collections::HashMap;
use std::ops::Range;
use std::sync::Arc;

use crate::ProcessContext;
use crate::biquad::flush;
use crate::instrument::Instrument;
use crate::sample::SampleData;
use crate::sfz::{
    Eg, FilterShape, LoopMode, MAX_EGS, MAX_LFOS, SfzControl, SfzRegion, Source, Target, Trigger,
};
use crate::stream::{Pcm, Rings, STREAM_VOICES};

/// Why bytes are not a SoundFont.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ParseError {
    /// Not a `RIFF` `sfbk` file.
    NotSoundFont,
    /// A `RIFF` `sfbk` file with broken lists: a chunk runs past its list; `sdta` `smpl` or a `pdta` table is
    /// missing or not a whole number of records; `phdr` has fewer than two records (a patch and the terminal one);
    /// or a bag or generator index runs past its table or goes back.
    Corrupted,
}

/// The `phdr` record size.
pub const PHDR_SIZE: usize = 38;
/// The most patches read from one SoundFont.
// ponytail: 65 536 patches (2.5 MB of `phdr`), as many as a bank and program pair can name; past that a hostile file
// only costs memory. Raise it if a real bank ever holds more.
pub const MAX_PATCHES: usize = 65_536;
/// The most regions (a preset zone with one of its instrument's zones) one patch plays.
// ponytail: a hostile preset can pair 65 535 zones with an instrument of 65 535 zones; real presets hold a few
// hundred. Raise it if a real one is cut short.
pub const MAX_REGIONS: usize = 8_192;
/// The generators a patch's regions may visit before the rest is dropped, so that hostile zone products stay cheap.
const WORK_BUDGET: usize = 1 << 22;
/// Voices per player; when all sound, the oldest-started is stolen. A streamed SFZ sample plays in one of the first
/// [`STREAM_VOICES`], which have rings, and steals the oldest of those.
pub const VOICES: usize = 512;
const _: () = assert!(STREAM_VOICES <= VOICES);

// Generator operators (SoundFont 2.01 §8.1.2).
const START_ADDRS: usize = 0;
const END_ADDRS: usize = 1;
const START_LOOP: usize = 2;
const END_LOOP: usize = 3;
const START_ADDRS_COARSE: usize = 4;
const MOD_LFO_TO_PITCH: usize = 5;
const VIB_LFO_TO_PITCH: usize = 6;
const MOD_ENV_TO_PITCH: usize = 7;
const FILTER_FC: usize = 8;
const FILTER_Q: usize = 9;
const MOD_LFO_TO_FILTER_FC: usize = 10;
const MOD_ENV_TO_FILTER_FC: usize = 11;
const END_ADDRS_COARSE: usize = 12;
const MOD_LFO_TO_VOLUME: usize = 13;
const PAN: usize = 17;
const DELAY_MOD_LFO: usize = 21;
const FREQ_MOD_LFO: usize = 22;
const DELAY_VIB_LFO: usize = 23;
const FREQ_VIB_LFO: usize = 24;
const DELAY_MOD_ENV: usize = 25;
const ATTACK_MOD_ENV: usize = 26;
const HOLD_MOD_ENV: usize = 27;
const DECAY_MOD_ENV: usize = 28;
const SUSTAIN_MOD_ENV: usize = 29;
const RELEASE_MOD_ENV: usize = 30;
const KEY_TO_MOD_ENV_HOLD: usize = 31;
const KEY_TO_MOD_ENV_DECAY: usize = 32;
const DELAY_VOL_ENV: usize = 33;
const ATTACK: usize = 34;
const HOLD_VOL_ENV: usize = 35;
const DECAY: usize = 36;
const SUSTAIN: usize = 37;
const RELEASE: usize = 38;
const KEY_TO_VOL_ENV_HOLD: usize = 39;
const KEY_TO_VOL_ENV_DECAY: usize = 40;
const INSTRUMENT: u16 = 41;
const KEYS: usize = 43;
const VELS: usize = 44;
const START_LOOP_COARSE: usize = 45;
const KEYNUM: usize = 46;
const VELOCITY: usize = 47;
const ATTENUATION: usize = 48;
const END_LOOP_COARSE: usize = 50;
const COARSE: usize = 51;
const FINE: usize = 52;
const SAMPLE: u16 = 53;
const MODES: usize = 54;
const SCALE: usize = 56;
const EXCLUSIVE_CLASS: usize = 57;
const ROOT: usize = 58;
/// Operators 0..=60 are defined; others are ignored.
const GENERATORS: usize = 61;

pub const ZONE_FIELDS: usize = 38;
pub const ZONE_SELECT_KEY: u32 = 7000;
pub const ZONE_RESET_KEY: u32 = 7005;
pub const ZONE_CTL_BASE: u32 = 7100;

/// A zone's generator amounts by operator. Ranges (43, 44) hold their low and high bytes.
type Gens = [i16; GENERATORS];

const FULL_RANGE: i16 = i16::from_le_bytes([0, 127]);

/// The values a zone starts from: a preset's add nothing; an instrument's are the standard defaults (scaleTuning 100,
/// envelope and vibrato LFO delay times −12000 tc, sustain 0 cB, no overriding root key).
const fn defaults(instrument: bool) -> Gens {
    let mut gens = [0; GENERATORS];
    gens[KEYS] = FULL_RANGE;
    gens[VELS] = FULL_RANGE;
    if instrument {
        gens[ATTACK] = -12000;
        gens[DECAY] = -12000;
        gens[RELEASE] = -12000;
        gens[DELAY_MOD_LFO] = -12000;
        gens[DELAY_VIB_LFO] = -12000;
        gens[DELAY_MOD_ENV] = -12000;
        gens[ATTACK_MOD_ENV] = -12000;
        gens[HOLD_MOD_ENV] = -12000;
        gens[DECAY_MOD_ENV] = -12000;
        gens[RELEASE_MOD_ENV] = -12000;
        gens[DELAY_VOL_ENV] = -12000;
        gens[HOLD_VOL_ENV] = -12000;
        gens[KEYNUM] = -1;
        gens[VELOCITY] = -1;
        gens[FILTER_FC] = 13500;
        gens[SCALE] = 100;
        gens[ROOT] = -1;
    }
    gens
}

/// The attenuation and sustain range of the standard, in cB. Amounts outside it are clamped, so a
/// hostile file cannot amplify.
const MAX_CB: i32 = 1440;
/// `sampleType` bit of a ROM sample, which no file carries the data of. Its zones are skipped.
const ROM: u16 = 0x8000;
/// The fastest a voice may read its sample, in source frames per output frame. For hostile files, the
/// standard's tuning range stays far below it.
const MAX_STEP: f64 = 65_536.0;

/// One `phdr` record: a patch.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct PresetHeader {
    name: [u8; 20],
    /// `wPreset`, the MIDI program.
    pub preset: u16,
    /// `wBank`.
    pub bank: u16,
}

impl PresetHeader {
    /// `achPresetName` up to its first NUL.
    pub fn name(&self) -> &[u8] {
        let end = self.name.iter().position(|&b| b == 0);
        &self.name[..end.unwrap_or(self.name.len())]
    }
}

/// The patches of the SoundFont `bytes`, in file order, without the terminal `EOP` record and at most
/// [`MAX_PATCHES`]. The file is checked as [`SoundFont::parse`] checks it, and that is all of it: a file lists its
/// patches exactly when it parses, so the rack never offers the patches of a bank that would play silence. Nothing but
/// the patches is kept.
pub fn preset_headers(bytes: &[u8]) -> Result<Vec<PresetHeader>, ParseError> {
    Ok(Tables::read(bytes)?.presets)
}

fn le16(bytes: &[u8], at: usize) -> u16 {
    u16::from_le_bytes([bytes[at], bytes[at + 1]])
}

fn le32(bytes: &[u8], at: usize) -> u32 {
    u32::from_le_bytes([bytes[at], bytes[at + 1], bytes[at + 2], bytes[at + 3]])
}

/// The lists of the `RIFF` `sfbk` file `bytes`. A RIFF size past the end of the file: the lists that are there; one
/// short of its form type: none.
fn lists(bytes: &[u8]) -> Result<&[u8], ParseError> {
    if bytes.get(..4) != Some(b"RIFF") || bytes.get(8..12) != Some(b"sfbk") {
        return Err(ParseError::NotSoundFont);
    }
    let size = le32(bytes, 4) as usize;
    Ok(&bytes[12..size.saturating_add(8).clamp(12, bytes.len())])
}

/// The body of the first chunk of `list` that `wanted(id, body)` picks. A chunk that runs past the list is corrupt,
/// and so is a list without the chunk; a tail too short for a chunk header is ignored.
fn find(mut list: &[u8], wanted: impl Fn(&[u8], &[u8]) -> bool) -> Result<&[u8], ParseError> {
    while list.len() >= 8 {
        let size = le32(list, 4) as usize;
        let end = size.checked_add(8).filter(|&end| end <= list.len());
        let body = &list[8..end.ok_or(ParseError::Corrupted)?];
        if wanted(&list[..4], body) {
            return Ok(body);
        }
        // Odd sizes are padded to a word.
        list = list.get(8 + size + size % 2..).unwrap_or_default();
    }
    Err(ParseError::Corrupted)
}

/// The chunks of the `LIST` of `kind` in `lists`.
fn list<'a>(lists: &'a [u8], kind: &[u8; 4]) -> Result<&'a [u8], ParseError> {
    Ok(&find(lists, |id, body| id == b"LIST" && body.starts_with(kind))?[4..])
}

/// The `pdta` table `id`: a whole number of `size`-byte records, at least the terminal one.
fn table<'a>(pdta: &'a [u8], id: &[u8; 4], size: usize) -> Result<&'a [u8], ParseError> {
    let table = find(pdta, |found, _| found == id)?;
    (table.len() % size == 0 && table.len() >= size)
        .then_some(table)
        .ok_or(ParseError::Corrupted)
}

/// `phdr`, which needs a patch besides its terminal record (the standard rejects less).
fn phdr(pdta: &[u8]) -> Result<&[u8], ParseError> {
    let phdr = table(pdta, b"phdr", PHDR_SIZE)?;
    (phdr.len() >= 2 * PHDR_SIZE)
        .then_some(phdr)
        .ok_or(ParseError::Corrupted)
}

fn headers(phdr: &[u8]) -> Vec<PresetHeader> {
    let patches = (phdr.len() / PHDR_SIZE - 1).min(MAX_PATCHES);
    let header = |r: &[u8; PHDR_SIZE]| PresetHeader {
        name: r[..20].try_into().unwrap_or_default(),
        preset: le16(r, 20),
        bank: le16(r, 22),
    };
    phdr.as_chunks::<PHDR_SIZE>()
        .0
        .iter()
        .take(patches)
        .map(header)
        .collect()
}

/// The `u16` at `at` of every `size`-byte record of `table`.
fn column(table: &[u8], size: usize, at: usize) -> Vec<u16> {
    table.chunks_exact(size).map(|r| le16(r, at)).collect()
}

/// `indices` never go back and stay within `max`.
fn ordered(indices: &[u16], max: usize) -> Result<(), ParseError> {
    let back = indices.windows(2).any(|w| w[0] > w[1]);
    let past = indices.last().is_some_and(|&i| usize::from(i) > max);
    (!back && !past).then_some(()).ok_or(ParseError::Corrupted)
}

#[derive(Clone, Copy)]
struct Gen {
    op: u16,
    amount: [u8; 2],
}

fn gens(table: &[u8]) -> Vec<Gen> {
    let generator = |r: &[u8; 4]| Gen {
        op: le16(r, 0),
        amount: [r[2], r[3]],
    };
    table.as_chunks::<4>().0.iter().map(generator).collect()
}

/// `base` with the generators of `zone` set; a later generator wins over an earlier one.
fn with(mut base: Gens, zone: &[Gen]) -> Gens {
    for generator in zone {
        if let Some(amount) = base.get_mut(usize::from(generator.op)) {
            *amount = i16::from_le_bytes(generator.amount);
        }
    }
    base
}

/// The index `zone` names with its last generator `op` (instrument or sampleID).
fn index(zone: &[Gen], op: u16) -> Option<usize> {
    let generator = zone.iter().rev().find(|g| g.op == op)?;
    Some(usize::from(u16::from_le_bytes(generator.amount)))
}

/// The overlap of two key or velocity ranges, `None` when empty.
fn overlap(a: i16, b: i16) -> Option<[u8; 2]> {
    let ([a_lo, a_hi], [b_lo, b_hi]) = (a.to_le_bytes(), b.to_le_bytes());
    let (lo, hi) = (a_lo.max(b_lo), a_hi.min(b_hi));
    (lo <= hi).then_some([lo, hi])
}

/// One `shdr` record.
#[derive(Clone, Copy)]
struct SampleHeader {
    name: [u8; 20],
    start: u32,
    end: u32,
    loop_start: u32,
    loop_end: u32,
    rate: u32,
    pitch: u8,
    correction: i8,
    kind: u16,
}

/// A parsed SoundFont: its tables, and the file, whose `smpl` holds the 16-bit samples (`sm24` is ignored).
pub struct SoundFont {
    presets: Vec<PresetHeader>,
    /// Per patch, and for the record after the last one: its first zone in `pbag`.
    preset_bags: Vec<u16>,
    /// Per preset zone, and the terminal one: its first generator in `pgen`.
    pbag: Vec<u16>,
    pgen: Vec<Gen>,
    /// Per instrument, and the terminal one: its first zone in `ibag`.
    inst_bags: Vec<u16>,
    ibag: Vec<u16>,
    igen: Vec<Gen>,
    /// Without the terminal `EOS`.
    samples: Vec<SampleHeader>,
    bytes: Vec<u8>,
    /// Where `smpl`'s data is in `bytes`.
    smpl: Range<usize>,
    /// An SFZ's regions and the samples they play, in place of the tables ([`SoundFont::sfz`]).
    sfz_regions: Vec<Region>,
    /// Whole in memory, or streamed from disk ([`crate::stream`]).
    pcm: Vec<Pcm>,
    /// Its `amp_velcurve`s, by [`Rules::curve`].
    curves: Vec<[f32; 128]>,
    /// The CC curves its routes read, by the index [`Self::sfz`] gave each route: the built-in ones and its `<curve>`s.
    cc_curves: Vec<[f32; 128]>,
    /// Each CC's value when it loads (`set_ccN`, else 0), and its smoothing: its routes' longest, seconds.
    cc_defaults: [f32; 128],
    cc_smooth: [f32; 128],
    /// The keys that are keyswitches (the `sw_lokey`…`sw_hikey` of every region with `sw_last`), and the one in force
    /// before any is played (the first `sw_default`).
    switches: Option<[u8; 2]>,
    switch: Option<u8>,
}

/// The tables of a SoundFont file, checked: everything [`SoundFont`] keeps besides the file.
struct Tables {
    presets: Vec<PresetHeader>,
    preset_bags: Vec<u16>,
    pbag: Vec<u16>,
    pgen: Vec<Gen>,
    inst_bags: Vec<u16>,
    ibag: Vec<u16>,
    igen: Vec<Gen>,
    samples: Vec<SampleHeader>,
    smpl: Range<usize>,
}

impl Tables {
    /// Reads and checks the tables of the SoundFont file `bytes`.
    fn read(bytes: &[u8]) -> Result<Self, ParseError> {
        let lists = lists(bytes)?;
        let pdta = list(lists, b"pdta")?;
        let smpl = find(list(lists, b"sdta")?, |id, _| id == b"smpl")?;
        let phdr = phdr(pdta)?;
        let presets = headers(phdr);
        let preset_bags = column(&phdr[..(presets.len() + 1) * PHDR_SIZE], PHDR_SIZE, 24);
        let pbag = column(table(pdta, b"pbag", 4)?, 4, 0);
        let pgen = gens(table(pdta, b"pgen", 4)?);
        let inst_bags = column(table(pdta, b"inst", 22)?, 22, 20);
        let ibag = column(table(pdta, b"ibag", 4)?, 4, 0);
        let igen = gens(table(pdta, b"igen", 4)?);
        ordered(&preset_bags, pbag.len() - 1)?;
        ordered(&pbag, pgen.len())?;
        ordered(&inst_bags, ibag.len() - 1)?;
        ordered(&ibag, igen.len())?;
        let shdr = table(pdta, b"shdr", 46)?;
        let sample = |r: &[u8; 46]| SampleHeader {
            name: r[..20].try_into().unwrap_or_default(),
            start: le32(r, 20),
            end: le32(r, 24),
            loop_start: le32(r, 28),
            loop_end: le32(r, 32),
            rate: le32(r, 36),
            pitch: r[40],
            correction: r[41] as i8,
            kind: le16(r, 44),
        };
        let samples = shdr.as_chunks::<46>().0.iter().map(sample);
        let samples = samples.take(shdr.len() / 46 - 1).collect();
        let start = smpl.as_ptr() as usize - bytes.as_ptr() as usize;
        let smpl = start..start + smpl.len();
        Ok(Self {
            presets,
            preset_bags,
            pbag,
            pgen,
            inst_bags,
            ibag,
            igen,
            samples,
            smpl,
        })
    }
}

impl SoundFont {
    /// Parses the SoundFont file `bytes`, which it keeps (app thread). `pmod`, `imod` and `INFO` are not read.
    pub fn parse(bytes: Vec<u8>) -> Result<Self, ParseError> {
        let t = Tables::read(&bytes)?;
        Ok(Self {
            presets: t.presets,
            preset_bags: t.preset_bags,
            pbag: t.pbag,
            pgen: t.pgen,
            inst_bags: t.inst_bags,
            ibag: t.ibag,
            igen: t.igen,
            samples: t.samples,
            bytes,
            smpl: t.smpl,
            sfz_regions: Vec::new(),
            pcm: Vec::new(),
            curves: Vec::new(),
            cc_curves: Vec::new(),
            cc_defaults: [0.0; 128],
            cc_smooth: [0.0; 128],
            switches: None,
            switch: None,
        })
    }

    /// An SFZ as a SoundFont of one patch, `name` (its first 20 bytes): each region with the sample it plays, decoded or
    /// streamed (regions sharing a file share its `Arc`), at most [`MAX_REGIONS`], and the SFZ's `control` (its CCs'
    /// values on load and its curves). A region that plays no frame (an `offset` at or past its `end`) or no key is left
    /// out. App thread.
    pub fn sfz(name: &str, regions: Vec<(SfzRegion, Pcm)>, control: &SfzControl) -> Self {
        let mut header = [0; 20];
        let name = &name.as_bytes()[..name.len().min(20)];
        header[..name.len()].copy_from_slice(name);
        let mut font = Self {
            presets: vec![PresetHeader {
                name: header,
                preset: 0,
                bank: 0,
            }],
            preset_bags: Vec::new(),
            pbag: Vec::new(),
            pgen: Vec::new(),
            inst_bags: Vec::new(),
            ibag: Vec::new(),
            igen: Vec::new(),
            samples: Vec::new(),
            bytes: Vec::new(),
            smpl: 0..0,
            sfz_regions: Vec::new(),
            pcm: Vec::new(),
            curves: Vec::new(),
            cc_curves: Vec::new(),
            cc_defaults: [0.0; 128],
            cc_smooth: [0.0; 128],
            switches: None,
            switch: None,
        };
        // Curve 0 first, then each other one there is: a route's curve becomes its table's index.
        let mut curve_at = [0; 256];
        for index in 0..=255 {
            let table = match control.curves.get(&index) {
                Some(points) => Some(velocity_curve(points)),
                None => builtin_curve(index),
            };
            if let Some(table) = table {
                curve_at[usize::from(index)] = font.cc_curves.len() as u8;
                font.cc_curves.push(table);
            }
        }
        for (&cc, &value) in &control.set_cc {
            font.cc_defaults[usize::from(cc.min(127))] = value;
        }
        let mut index = HashMap::new();
        for (region, data) in regions.into_iter().take(MAX_REGIONS) {
            let pcm = &mut font.pcm;
            let sample = *index.entry(data.ptr()).or_insert_with(|| {
                pcm.push(data.clone());
                pcm.len() - 1
            });
            let curve = (!region.amp_velcurve.is_empty()).then(|| {
                font.curves.push(velocity_curve(&region.amp_velcurve));
                font.curves.len() - 1
            });
            let mut def = region.clone();
            for route in &mut def.routes {
                route.curve = curve_at[usize::from(route.curve)];
                if let Source::Cc(cc) = route.source {
                    let smooth = &mut font.cc_smooth[usize::from(cc.min(127))];
                    *smooth = smooth.max(route.smooth / 1000.0);
                }
            }
            let Some(playable) = sfz_region(&region, &data, sample, curve, Arc::new(def)) else {
                continue;
            };
            let key = |k: i32| k.clamp(0, 127) as u8;
            if let Some(last) = region.sw_last {
                // Without `sw_lokey` and `sw_hikey`, its own keyswitches are the keys that switch.
                let keys = match region.sw_keys {
                    [0, 127] => [last, region.sw_hilast.unwrap_or(last)],
                    keys => keys,
                };
                let [lo, hi] = keys.map(key);
                font.switches = Some(
                    font.switches
                        .map_or([lo, hi], |[a, b]| [a.min(lo), b.max(hi)]),
                );
            }
            font.switch = font.switch.or(region.sw_default.map(key));
            font.sfz_regions.push(playable);
        }
        font
    }

    /// The length of the file this keeps, which is nearly all of what it holds (the tables are a few bytes per record
    /// of the file), and an SFZ's decoded samples and streamed samples' heads: what a cache counts against its memory
    /// budget.
    pub fn byte_len(&self) -> usize {
        self.bytes.len() + self.pcm.iter().map(Pcm::byte_len).sum::<usize>()
    }

    /// The patches in file order (the `SF2D` patch index counts them), as [`preset_headers`] lists them.
    pub fn presets(&self) -> &[PresetHeader] {
        &self.presets
    }

    /// What patch `patch` plays: every preset zone paired with each zone of its instrument whose key and velocity
    /// ranges overlap, at most [`MAX_REGIONS`]. A first zone without an instrument (preset) or a sample
    /// (instrument) is the global zone, whose generators every other zone starts from; any other such zone is
    /// ignored, and so is a zone naming an instrument or sample that does not exist or cannot play. App thread.
    pub fn regions(&self, patch: usize) -> Vec<Region> {
        if !self.sfz_regions.is_empty() {
            return self.sfz_regions.clone();
        }
        let mut regions = Vec::new();
        let Some(&[first, last]) = self.preset_bags.get(patch..).and_then(|s| s.get(..2)) else {
            return regions;
        };
        let mut budget = WORK_BUDGET;
        let mut preset_global = defaults(false);
        let preset_zones = zones(&self.pbag, &self.pgen, first, last);
        for (i, preset_zone) in preset_zones.enumerate() {
            let preset = with(preset_global, preset_zone);
            let Some(instrument) = index(preset_zone, INSTRUMENT) else {
                if i == 0 {
                    preset_global = preset;
                }
                continue;
            };
            let Some(&[first, last]) = self.inst_bags.get(instrument..instrument + 2) else {
                continue;
            };
            let mut global = defaults(true);
            for (j, zone) in zones(&self.ibag, &self.igen, first, last).enumerate() {
                budget = budget.saturating_sub(1 + zone.len());
                if budget == 0 || regions.len() == MAX_REGIONS {
                    return regions;
                }
                let gens = with(global, zone);
                match index(zone, SAMPLE) {
                    Some(sample) => regions.extend(self.region(&preset, &gens, sample)),
                    None if j == 0 => global = gens,
                    None => {}
                }
            }
        }
        regions
    }

    /// The region of preset values `preset` (added) over instrument values `gens` playing sample `sample`.
    fn region(&self, preset: &Gens, gens: &Gens, sample: usize) -> Option<Region> {
        let header = self.samples.get(sample)?;
        let end_n = header.name.iter().position(|&b| b == 0).unwrap_or(20);
        let name = String::from_utf8_lossy(&header.name[..end_n]).into_owned();
        let (raw_start, raw_end) = (header.start as usize, header.end as usize);
        let playable = raw_start < raw_end && raw_end <= self.smpl.len() / 2 && header.rate > 0;
        if !playable || header.kind & ROM != 0 {
            return None;
        }
        let add = |op: usize| i32::from(preset[op]) + i32::from(gens[op]);
        let start = (i64::from(header.start)
            + i64::from(gens[START_ADDRS])
            + 32_768 * i64::from(gens[START_ADDRS_COARSE]))
        .clamp(raw_start as i64, raw_end as i64) as usize;
        let end = (i64::from(header.end)
            + i64::from(gens[END_ADDRS])
            + 32_768 * i64::from(gens[END_ADDRS_COARSE]))
        .clamp(raw_start as i64, raw_end as i64) as usize;
        if start >= end {
            return None;
        }
        let point = |at: u32, fine: usize, coarse: usize| {
            let offset = i64::from(gens[fine]) + 32_768 * i64::from(gens[coarse]);
            (i64::from(at) + offset).clamp(raw_start as i64, raw_end as i64) as usize
        };
        let raw_loop_start = point(header.loop_start, START_LOOP, START_LOOP_COARSE);
        let raw_loop_end = point(header.loop_end, END_LOOP, END_LOOP_COARSE);
        let loop_start = raw_loop_start.clamp(start, end);
        let loop_end = raw_loop_end.clamp(loop_start, end);
        let loop_mode = match gens[MODES] & 3 {
            1 => 1,
            3 => 2,
            _ => 0,
        };
        let can_loop = loop_end > loop_start;
        let fixed_key = match gens[KEYNUM] {
            k @ 0..=127 => Some(k as u8),
            _ => None,
        };
        let fixed_vel = match gens[VELOCITY] {
            v @ 0..=127 => Some(v as u8),
            _ => None,
        };
        // overridingRootKey, else the sample's pitch; 128–255 is unpitched, which the standard plays at 60.
        let root = match (gens[ROOT], header.pitch) {
            (key @ 0..=127, _) => key as u8,
            (_, pitch @ 0..=127) => pitch,
            _ => 60,
        };
        let centibels = |op: usize| add(op).clamp(0, MAX_CB) as f32;
        let filter_fc = add(FILTER_FC).clamp(1500, 13500);
        let filter_q = add(FILTER_Q).clamp(0, 960);
        let mod_lfo_to_filter = add(MOD_LFO_TO_FILTER_FC).clamp(-12000, 12000) as f32;
        let mod_env_to_filter = add(MOD_ENV_TO_FILTER_FC).clamp(-12000, 12000) as f32;
        let filter = (filter_fc < 13500
            || filter_q > 0
            || mod_lfo_to_filter != 0.0
            || mod_env_to_filter != 0.0)
            .then(|| {
                [
                    LFO_HZ * 2f32.powf(filter_fc as f32 / 1200.0),
                    filter_q as f32 / 10.0,
                ]
            });
        let q_attenuation = 0.5 * filter_q.min(400) as f32;
        let mod_lfo_to_vol = add(MOD_LFO_TO_VOLUME).clamp(-960, 960) as f32 / 10.0;
        let mod_env = ModEnv {
            delay: add(DELAY_MOD_ENV) as f32,
            attack: add(ATTACK_MOD_ENV) as f32,
            hold: add(HOLD_MOD_ENV) as f32,
            decay: add(DECAY_MOD_ENV) as f32,
            sustain: 1.0 - add(SUSTAIN_MOD_ENV).clamp(0, 1000) as f32 / 1000.0,
            release: add(RELEASE_MOD_ENV) as f32,
            key_hold: add(KEY_TO_MOD_ENV_HOLD).clamp(-1200, 1200) as f32,
            key_decay: add(KEY_TO_MOD_ENV_DECAY).clamp(-1200, 1200) as f32,
            to_pitch: add(MOD_ENV_TO_PITCH).clamp(-12000, 12000) as f32,
            to_filter: mod_env_to_filter,
        };
        Some(Region {
            name,
            keys: overlap(preset[KEYS], gens[KEYS])?,
            vels: overlap(preset[VELS], gens[VELS])?,
            start,
            end,
            loop_start,
            loop_end,
            raw_start,
            raw_end,
            raw_loop_start,
            raw_loop_end,
            loop_mode,
            fixed_key,
            fixed_vel,
            looping: loop_mode != 0 && can_loop,
            loop_sustain: loop_mode == 2 && can_loop,
            exclusive: gens[EXCLUSIVE_CLASS].max(0) as u16,
            rate: f64::from(header.rate),
            root,
            tune: f64::from(add(COARSE))
                + f64::from(add(FINE) + i32::from(header.correction)) / 100.0,
            scale: f64::from(gens[SCALE]),
            pan: add(PAN).clamp(-500, 500) as f32 / 1000.0,
            gain: 10f32.powf(-centibels(ATTENUATION) / 500.0 - q_attenuation / 200.0),
            vol_delay: add(DELAY_VOL_ENV) as f32,
            attack: add(ATTACK) as f32,
            vol_hold: add(HOLD_VOL_ENV) as f32,
            decay: add(DECAY) as f32,
            sustain: 10f32.powf(-centibels(SUSTAIN) / 200.0),
            release: add(RELEASE) as f32,
            key_vol_hold: add(KEY_TO_VOL_ENV_HOLD).clamp(-1200, 1200) as f32,
            key_vol_decay: add(KEY_TO_VOL_ENV_DECAY).clamp(-1200, 1200) as f32,
            vibrato: [
                add(VIB_LFO_TO_PITCH).clamp(-12000, 12000) as f32,
                LFO_HZ * 2f32.powf(add(FREQ_VIB_LFO).clamp(-16000, 4500) as f32 / 1200.0),
                2f32.powf(add(DELAY_VIB_LFO).clamp(-12000, 5000) as f32 / 1200.0),
            ],
            mod_lfo: [
                add(MOD_LFO_TO_PITCH).clamp(-12000, 12000) as f32,
                LFO_HZ * 2f32.powf(add(FREQ_MOD_LFO).clamp(-16000, 4500) as f32 / 1200.0),
                2f32.powf(add(DELAY_MOD_LFO).clamp(-12000, 5000) as f32 / 1200.0),
            ],
            filter,
            mod_lfo_fx: [mod_lfo_to_filter, mod_lfo_to_vol],
            mod_env,
            sfz: None,
        })
    }

    /// CC curve `index` ([`Self::cc_curves`]) at `x` (0…1), straight between its points.
    fn curve(&self, index: u8, x: f32) -> f32 {
        let Some(table) = self.cc_curves.get(usize::from(index)) else {
            return x;
        };
        let at = x.clamp(0.0, 1.0) * 127.0;
        let i = (at as usize).min(126);
        table[i] + (table[i + 1] - table[i]) * (at - i as f32)
    }

    /// Sample frame `i` of `smpl`, in −1..1; silence outside it.
    fn frame(&self, i: usize) -> f32 {
        let at = self.smpl.start + 2 * i;
        match self.bytes.get(at..at + 2) {
            Some(&[lo, hi]) => f32::from(i16::from_le_bytes([lo, hi])) / 32_768.0,
            _ => 0.0,
        }
    }

    pub fn zone_infos(&self, patch: usize) -> Vec<ZoneInfo> {
    self.regions(patch)
        .into_iter()
        .map(|r| ZoneInfo {
            defaults: region_defaults(&r),
            name: r.name,
            keys: r.keys,
            vels: r.vels,
        })
        .collect()
}
}

/// What `at(frame)` reads at `position` (in `start..end`) of `region`'s sample: straight between the two frames around
/// it, or with `cubic` (Quality High) on a Catmull-Rom curve through the four. With `looping` a loop's last frame leads
/// into its first (and comes before it); a sample's last frame is held, and so is its first before it.
fn interpolate(
    region: &Region,
    looping: bool,
    position: f64,
    cubic: bool,
    at: impl Fn(usize) -> f32,
) -> f32 {
    let i = position as usize;
    let t = (position - i as f64) as f32;
    let after = |k: usize| match looping && k >= region.loop_end {
        true => region.loop_start + (k - region.loop_start) % (region.loop_end - region.loop_start),
        false => k.min(region.end - 1),
    };
    let (b, c) = (at(i), at(after(i + 1)));
    if !cubic {
        return b + (c - b) * t;
    }
    let before = match looping && i == region.loop_start {
        true => region.loop_end - 1,
        false => i.saturating_sub(1).max(region.start),
    };
    let (a, d) = (at(before), at(after(i + 2)));
    b + 0.5 * t * (c - a + t * (2.0 * a - 5.0 * b + 4.0 * c - d + t * (3.0 * (b - c) + d - a)))
}

/// The zones `first..last` of a bag table, as generator slices (indices checked by [`SoundFont::parse`]).
fn zones<'a>(
    bags: &'a [u16],
    gens: &'a [Gen],
    first: u16,
    last: u16,
) -> impl Iterator<Item = &'a [Gen]> {
    (usize::from(first)..usize::from(last))
        .map(move |zone| &gens[usize::from(bags[zone])..usize::from(bags[zone + 1])])
}

#[derive(Clone, Debug, PartialEq)]
pub struct ZoneInfo {
    pub name: String,
    pub keys: [u8; 2],
    pub vels: [u8; 2],
    pub defaults: [f32; ZONE_FIELDS],
}

#[derive(Clone, Debug, PartialEq)]
pub struct ZoneOverride {
    pub zone: u16,
    pub mask: u64,
    pub values: [f32; ZONE_FIELDS],
}

fn tc_to_slider(tc: f32) -> f32 {
    let t = 2f32.powf(tc / 1200.0).clamp(0.005, 10.0);
    (((t / 10.0).cbrt() - 0.05) / 0.95).clamp(0.0, 1.0)
}

fn slider_to_tc(v: f32) -> f32 {
    let t = (10.0 * 0.95f32.mul_add(v.clamp(0.0, 1.0), 0.05).powi(3)).max(0.005);
    1200.0 * t.log2()
}

fn mod_tc_to_slider(tc: f32) -> f32 {
    if tc <= -11950.0 {
        0.0
    } else {
        let t = 2f32.powf(tc / 1200.0).clamp(0.0, 20.0);
        (t / 20.0).cbrt().clamp(0.0, 1.0)
    }
}

fn mod_slider_to_tc(v: f32) -> f32 {
    let v = v.clamp(0.0, 1.0);
    if v <= 0.001 {
        -12000.0
    } else {
        let t = (20.0 * v.powi(3)).max(0.001);
        1200.0 * t.log2()
    }
}

fn region_defaults(r: &Region) -> [f32; ZONE_FIELDS] {
    let default_hz = LFO_HZ * 2f32.powf(13500.0 / 1200.0);
    let (f_hz, f_q) = r.filter.map_or((default_hz, 0.0), |[hz, q]| (hz, q));
    let cutoff = ((1200.0 * (f_hz / LFO_HZ).max(1.0).log2() - 1500.0) / 12000.0).clamp(0.0, 1.0);
    let coarse = r.tune.round();
    let fine_cents = (r.tune - coarse) * 100.0;
    let gain_db = 20.0 * r.gain.max(1e-4).log10();
    let sample_mode = f32::from(r.loop_mode);
    let span = (r.raw_end.saturating_sub(r.raw_start)).max(1) as f32;
    let start_norm = ((r.start.saturating_sub(r.raw_start)) as f32 / span).clamp(0.0, 1.0);
    let end_norm = ((r.end.saturating_sub(r.raw_start)) as f32 / span).clamp(0.0, 1.0);
    let fixed_key = r.fixed_key.map_or(0.0, |k| f32::from(k + 1) / 128.0);
    let fixed_vel = r.fixed_vel.map_or(0.0, |v| f32::from(v + 1) / 128.0);
    [
        tc_to_slider(r.attack),
        tc_to_slider(r.decay),
        r.sustain.clamp(0.0, 1.0),
        tc_to_slider(r.release),
        (r.vibrato[1] / 20.0).clamp(0.0, 1.0),
        ((r.vibrato[0] + 100.0) / 200.0).clamp(0.0, 1.0),
        (r.vibrato[2] / 5.0).clamp(0.0, 1.0),
        (r.mod_lfo[1] / 20.0).clamp(0.0, 1.0),
        ((r.mod_lfo[0] + 1200.0) / 2400.0).clamp(0.0, 1.0),
        (r.mod_lfo[2] / 5.0).clamp(0.0, 1.0),
        cutoff,
        (f_q / 40.0).clamp(0.0, 1.0),
        ((coarse as f32 + 24.0) / 48.0).clamp(0.0, 1.0),
        ((fine_cents as f32 + 100.0) / 200.0).clamp(0.0, 1.0),
        (r.pan + 0.5).clamp(0.0, 1.0),
        ((gain_db + 48.0) / 60.0).clamp(0.0, 1.0),
        ((r.mod_lfo_fx[0] + 12000.0) / 24000.0).clamp(0.0, 1.0),
        ((r.mod_lfo_fx[1] + 24.0) / 48.0).clamp(0.0, 1.0),
        mod_tc_to_slider(r.mod_env.delay),
        mod_tc_to_slider(r.mod_env.attack),
        mod_tc_to_slider(r.mod_env.hold),
        mod_tc_to_slider(r.mod_env.decay),
        r.mod_env.sustain.clamp(0.0, 1.0),
        mod_tc_to_slider(r.mod_env.release),
        ((r.mod_env.to_pitch + 2400.0) / 4800.0).clamp(0.0, 1.0),
        ((r.mod_env.to_filter + 12000.0) / 24000.0).clamp(0.0, 1.0),
        ((r.mod_env.key_hold + 1200.0) / 2400.0).clamp(0.0, 1.0),
        ((r.mod_env.key_decay + 1200.0) / 2400.0).clamp(0.0, 1.0),
        mod_tc_to_slider(r.vol_delay),
        mod_tc_to_slider(r.vol_hold),
        ((r.key_vol_hold + 1200.0) / 2400.0).clamp(0.0, 1.0),
        ((r.key_vol_decay + 1200.0) / 2400.0).clamp(0.0, 1.0),
        sample_mode,
        f32::from(r.exclusive.min(32)),
        start_norm,
        end_norm,
        fixed_key,
        fixed_vel
    ]
}

fn apply_zone_field(r: &mut Region, field: usize, raw: f32) {
    let v = raw.clamp(0.0, 1.0);
    let default_hz = LFO_HZ * 2f32.powf(13500.0 / 1200.0);
    match field {
        0 => r.attack = slider_to_tc(v),
        1 => r.decay = slider_to_tc(v),
        2 => r.sustain = v,
        3 => r.release = slider_to_tc(v),
        4 => r.vibrato[1] = 20.0 * v,
        5 => r.vibrato[0] = 200.0 * v - 100.0,
        6 => r.vibrato[2] = 5.0 * v,
        7 => r.mod_lfo[1] = 20.0 * v,
        8 => r.mod_lfo[0] = 2400.0 * v - 1200.0,
        9 => r.mod_lfo[2] = 5.0 * v,
        10 => {
            let [_, q] = r.filter.unwrap_or([default_hz, 0.0]);
            let hz = LFO_HZ * 2f32.powf((1500.0 + 12000.0 * v) / 1200.0);
            r.filter = Some([hz, q]);
        }
        11 => {
            let [hz, _] = r.filter.unwrap_or([default_hz, 0.0]);
            r.filter = Some([hz, 40.0 * v]);
        }
        12 => {
            let fine = r.tune - r.tune.round();
            r.tune = f64::from((48.0 * v).round() - 24.0) + fine;
        }
        13 => {
            let coarse = r.tune.round();
            r.tune = coarse + f64::from(200.0 * v - 100.0) / 100.0;
        }
        14 => r.pan = v - 0.5,
        15 => r.gain = 10f32.powf((60.0 * v - 48.0) / 20.0),
        16 => {
            let depth = 24000.0 * v - 12000.0;
            r.mod_lfo_fx[0] = depth;
            if depth != 0.0 && r.filter.is_none() {
                r.filter = Some([default_hz, 0.0]);
            }
        }
        17 => r.mod_lfo_fx[1] = 48.0 * v - 24.0,
        18 => r.mod_env.delay = mod_slider_to_tc(v),
        19 => r.mod_env.attack = mod_slider_to_tc(v),
        20 => r.mod_env.hold = mod_slider_to_tc(v),
        21 => r.mod_env.decay = mod_slider_to_tc(v),
        22 => r.mod_env.sustain = v,
        23 => r.mod_env.release = mod_slider_to_tc(v),
        24 => r.mod_env.to_pitch = 4800.0 * v - 2400.0,
        25 => {
            let depth = 24000.0 * v - 12000.0;
            r.mod_env.to_filter = depth;
            if depth != 0.0 && r.filter.is_none() {
                r.filter = Some([default_hz, 0.0]);
            }
        }
        26 => r.mod_env.key_hold = 2400.0 * v - 1200.0,
        27 => r.mod_env.key_decay = 2400.0 * v - 1200.0,
        28 => r.vol_delay = mod_slider_to_tc(v),
        29 => r.vol_hold = mod_slider_to_tc(v),
        30 => r.key_vol_hold = 2400.0 * v - 1200.0,
        31 => r.key_vol_decay = 2400.0 * v - 1200.0,
        32 => {
            let can_loop = r.loop_end > r.loop_start;
            match raw.round() as i32 {
                1 => {
                    r.loop_mode = 1;
                    (r.looping, r.loop_sustain) = (can_loop, false);
                }
                2 => {
                    r.loop_mode = 2;
                    (r.looping, r.loop_sustain) = (can_loop, true);
                }
                _ => {
                    r.loop_mode = 0;
                    (r.looping, r.loop_sustain) = (false, false);
                }
            }
        }
        33 => r.exclusive = raw.round().clamp(0.0, 127.0) as u16,
        34 => {
            let len = r.raw_end.saturating_sub(r.raw_start) as f32;
            let pos = r.raw_start + (len * v).round() as usize;
            r.start = pos.clamp(r.raw_start, r.raw_end.saturating_sub(1));
            if r.end <= r.start {
                r.end = (r.start + 1).min(r.raw_end);
            }
            r.loop_start = r.raw_loop_start.clamp(r.start, r.end);
            r.loop_end = r.raw_loop_end.clamp(r.loop_start, r.end);
            let can_loop = r.loop_end > r.loop_start;
            r.looping = r.loop_mode != 0 && can_loop;
            r.loop_sustain = r.loop_mode == 2 && can_loop;
        }
        35 => {
            let len = r.raw_end.saturating_sub(r.raw_start) as f32;
            let pos = r.raw_start + (len * v).round() as usize;
            r.end = pos.clamp(r.raw_start + 1, r.raw_end);
            if r.start >= r.end {
                r.start = r.end.saturating_sub(1).max(r.raw_start);
            }
            r.loop_start = r.raw_loop_start.clamp(r.start, r.end);
            r.loop_end = r.raw_loop_end.clamp(r.loop_start, r.end);
            let can_loop = r.loop_end > r.loop_start;
            r.looping = r.loop_mode != 0 && can_loop;
            r.loop_sustain = r.loop_mode == 2 && can_loop;
        }
        36 => {
            let idx = (128.0 * v).round() as i32;
            r.fixed_key = (idx > 0).then_some((idx - 1).clamp(0, 127) as u8);
        }
        37 => {
            let idx = (128.0 * v).round() as i32;
            r.fixed_vel = (idx > 0).then_some((idx - 1).clamp(0, 127) as u8);
        }
        _ => {}
    }
}

#[derive(Clone, Copy, Debug, Default)]
struct ModEnv {
    delay: f32,
    attack: f32,
    hold: f32,
    decay: f32,
    sustain: f32,
    release: f32,
    key_hold: f32,
    key_decay: f32,
    to_pitch: f32,
    to_filter: f32,
}

/// A preset zone with one of its instrument's zones, resolved: what a voice plays.
#[derive(Clone, Debug)]
pub struct Region {
    name: String,
    keys: [u8; 2],
    vels: [u8; 2],
    /// Sample frames in `smpl`: the sample, and its loop.
    start: usize,
    end: usize,
    loop_start: usize,
    loop_end: usize,
    raw_start: usize,
    raw_end: usize,
    raw_loop_start: usize,
    raw_loop_end: usize,
    loop_mode: u8,
    fixed_key: Option<u8>,
    fixed_vel: Option<u8>,
    looping: bool,
    loop_sustain: bool,
    exclusive: u16,
    rate: f64,
    root: u8,
    /// Coarse and fine tune and the sample's correction, in semitones.
    tune: f64,
    /// Cents per key.
    scale: f64,
    /// −0.5..0.5, added to the note pan.
    pan: f32,
    /// From the attenuation.
    gain: f32,
    /// Timecents.
    attack: f32,
    decay: f32,
    /// Level.
    sustain: f32,
    release: f32,
    vol_delay: f32,
    vol_hold: f32,
    key_vol_hold: f32,
    key_vol_decay: f32,
    /// A SoundFont's vibrato LFO: its pitch depth (cents), rate (Hz) and delay (seconds); an SFZ's has no depth.
    vibrato: [f32; 3],
    /// A SoundFont's modulation LFO: pitch depth (cents), rate (Hz) and delay (seconds).
    mod_lfo: [f32; 3],
    filter: Option<[f32; 2]>,
    mod_lfo_fx: [f32; 2],
    mod_env: ModEnv,
    /// An SFZ region's sample and rules (its frames above are the sample's, its tune 0 and its scale the key
    /// tracking); `None` for a SoundFont's.
    sfz: Option<Rules>,
}

impl Region {
    fn plays(&self, key: u8, velocity: u8) -> bool {
        (self.keys[0]..=self.keys[1]).contains(&key)
            && (self.vels[0]..=self.vels[1]).contains(&velocity)
    }
}

/// What an SFZ region plays by besides a [`Region`]'s keys, velocities, frames, pitch and level.
#[derive(Clone, Debug)]
struct Rules {
    /// Its sample in [`SoundFont::pcm`].
    sample: usize,
    /// `transpose` and `tune`, in cents.
    cents: f64,
    mode: LoopMode,
    trigger: Trigger,
    group: i64,
    off_by: Option<i64>,
    off_normal: bool,
    /// `seq_length`, and `seq_position` from 0.
    seq: [u32; 2],
    rand: [f32; 2],
    /// `sw_last` (`sw_lolast`) … `sw_hilast`.
    sw_last: Option<[u8; 2]>,
    xfin: [u8; 2],
    xfout: [u8; 2],
    /// `amp_veltrack`, −1…1.
    veltrack: f32,
    /// Its `amp_velcurve` in [`SoundFont::curves`].
    curve: Option<usize>,
    /// Delay, attack, hold, decay (seconds), sustain (level), release (seconds).
    ampeg: [f32; 6],
    /// The region as the SFZ gave it, its routes' curves made [`SoundFont::cc_curves`] indices: what the rest of its
    /// rules and its modulation read.
    def: Arc<SfzRegion>,
}

impl Rules {
    /// The gain of `velocity` (0…1): the curve (`amp_velcurve`, else velocity²) scaled by `amp_veltrack`.
    fn velocity_gain(&self, curves: &[[f32; 128]], velocity: f32) -> f32 {
        let curve = match self.curve.and_then(|c| curves.get(c)) {
            Some(table) => table[(velocity * 127.0).round().clamp(0.0, 127.0) as usize],
            None => velocity * velocity,
        };
        match self.veltrack >= 0.0 {
            true => 1.0 - self.veltrack * (1.0 - curve),
            false => 1.0 + self.veltrack * curve,
        }
    }

    /// The velocity crossfade's gain at MIDI velocity `velocity`: in over `xfin`, out over `xfout`, by power.
    fn crossfade(&self, velocity: u8) -> f32 {
        let range = |r: [u8; 2]| r.map(f32::from);
        crossfade(f32::from(velocity), range(self.xfin), range(self.xfout))
    }
}

/// A crossfade's gain at `v`: in over `[lo, hi]` of `fade_in`, out over `fade_out`, by power.
fn crossfade(v: f32, fade_in: [f32; 2], fade_out: [f32; 2]) -> f32 {
    let [lo, hi] = fade_in;
    let gain_in = match v {
        v if v < lo => 0.0,
        v if v >= hi => 1.0,
        v => ((v - lo) / (hi - lo)).sqrt(),
    };
    let [lo, hi] = fade_out;
    let gain_out = match v {
        v if v <= lo => 1.0,
        v if v > hi => 0.0,
        v => ((hi - v) / (hi - lo)).sqrt(),
    };
    gain_in * gain_out
}

/// The built-in CC curves: 0 linear, 1 bipolar (−1…1), 2 linear down, 3 bipolar down, 4 x², 5 √x, 6 √(1 − x).
fn builtin_curve(index: u8) -> Option<[f32; 128]> {
    let f: fn(f32) -> f32 = match index {
        0 => |x| x,
        1 => |x| 2.0 * x - 1.0,
        2 => |x| 1.0 - x,
        3 => |x| 1.0 - 2.0 * x,
        4 => |x| x * x,
        5 => f32::sqrt,
        6 => |x| (1.0 - x).sqrt(),
        _ => return None,
    };
    Some(std::array::from_fn(|i| f(i as f32 / 127.0)))
}

/// `amp_velcurve_N` points as a gain per velocity: straight lines between them, from (0, 0) and to (127, 1) unless
/// the points set those.
fn velocity_curve(points: &[(u8, f32)]) -> [f32; 128] {
    let mut known = [None; 128];
    (known[0], known[127]) = (Some(0.0), Some(1.0));
    for &(velocity, gain) in points {
        known[usize::from(velocity.min(127))] = Some(gain);
    }
    let mut curve = [0.0; 128];
    let mut last = 0;
    for (v, gain) in known.into_iter().enumerate() {
        if let Some(gain) = gain {
            let from = curve[last];
            for (k, c) in curve.iter_mut().enumerate().take(v + 1).skip(last) {
                *c = if v == last {
                    gain
                } else {
                    from + (gain - from) * (k - last) as f32 / (v - last) as f32
                };
            }
            last = v;
        }
    }
    curve
}

/// The [`Region`] of SFZ region `r` (`def` as its modulation reads it) playing `data` (sample `sample`, curve `curve`),
/// `None` when it plays nothing. A loop end is the loop's last frame (the format's, and a WAV `smpl`'s), here exclusive;
/// without `loop_mode` a file with a loop loops.
fn sfz_region(
    r: &SfzRegion,
    data: &Pcm,
    sample: usize,
    curve: Option<usize>,
    def: Arc<SfzRegion>,
) -> Option<Region> {
    let keys = [r.keys[0].max(0), r.keys[1].min(127)];
    if keys[0] > keys[1] || r.end.is_some_and(|e| e <= 0) || data.sample_rate() == 0 {
        return None;
    }
    let frames = data.frames();
    let after = |last: u64| {
        usize::try_from(last)
            .unwrap_or(usize::MAX)
            .saturating_add(1)
    };
    let end = r.end.map_or(frames, |e| after(e as u64).min(frames));
    let start = usize::try_from(r.offset).unwrap_or(usize::MAX).min(end);
    if start >= end {
        return None;
    }
    let file_loop = (data.meta().loop_frames)
        .map(|(s, e)| (s as usize, after(u64::from(e)).min(frames)))
        .filter(|(s, e)| s < e);
    let loop_start = (r
        .loop_start
        .map(|s| usize::try_from(s).unwrap_or(usize::MAX)))
    .or(file_loop.map(|l| l.0))
    .unwrap_or(start)
    .clamp(start, end);
    let loop_end = (r.loop_end.map(after).or(file_loop.map(|l| l.1)))
        .unwrap_or(end)
        .clamp(start, end);
    let mode = r.loop_mode.unwrap_or(match file_loop {
        Some(_) => LoopMode::Continuous,
        None => LoopMode::NoLoop,
    });
    let root = r.pitch_keycenter.clamp(0, 127) as u8;
    let rate = match crate::sfz::pitched(&r.sample) {
        // One cycle a `CYCLE` frames: the key center's frequency.
        true => crate::sfz::CYCLE as f64 * 440.0 * 2f64.powf((f64::from(root) - 69.0) / 12.0),
        false => f64::from(data.sample_rate()),
    };
    let [delay, attack, hold, decay, sustain, release] = r.ampeg;
    Some(Region {
        name: String::new(),
        keys: [keys[0] as u8, keys[1] as u8],
        vels: r.vels,
        start,
        end,
        loop_start,
        loop_end,
        raw_start: start,
        raw_end: end,
        raw_loop_start: loop_start,
        raw_loop_end: loop_end,
        loop_mode: match mode {
            LoopMode::Continuous => 1,
            LoopMode::Sustain => 2,
            LoopMode::NoLoop | LoopMode::OneShot => 0,
        },
        fixed_key: None,
        fixed_vel: None,
        looping: matches!(mode, LoopMode::Continuous | LoopMode::Sustain) && loop_end > loop_start,
        loop_sustain: mode == LoopMode::Sustain && loop_end > loop_start,
        exclusive: 0,
        rate,
        root,
        tune: 0.0,
        scale: f64::from(r.pitch_keytrack),
        pan: r.pan / 200.0,
        gain: 10f32.powf(r.volume / 20.0) * r.amplitude / 100.0,
        attack: 0.0,
        decay: 0.0,
        sustain: 0.0,
        release: 0.0,
        vol_delay: -12000.0,
        vol_hold: -12000.0,
        key_vol_hold: 0.0,
        key_vol_decay: 0.0,
        vibrato: [0.0; 3],
        mod_lfo: [0.0; 3],
        filter: None,
        mod_lfo_fx: [0.0; 2],
        mod_env: ModEnv::default(),
        sfz: Some(Rules {
            sample,
            cents: f64::from(r.transpose) * 100.0 + f64::from(r.tune),
            mode,
            trigger: r.trigger,
            group: r.group,
            off_by: r.off_by,
            off_normal: r.off_normal,
            seq: [r.seq_length.max(1), r.seq_position.max(1) - 1],
            rand: r.rand,
            sw_last: (r.sw_last)
                .map(|lo| [lo, r.sw_hilast.unwrap_or(lo)].map(|k| k.clamp(0, 127) as u8)),
            xfin: r.xfin,
            xfout: r.xfout,
            veltrack: r.amp_veltrack / 100.0,
            curve,
            ampeg: [delay, attack, hold, decay, sustain / 100.0, release],
            def,
        }),
    })
}

/// `region`'s decoded sample `data` at `position` (in `start..end`), each channel [`interpolate`]d, mono as both.
fn read_pcm(
    data: &SampleData,
    region: &Region,
    looping: bool,
    position: f64,
    cubic: bool,
) -> (f32, f32) {
    let read = |x: &[f32]| {
        interpolate(region, looping, position, cubic, |k| {
            x.get(k).copied().unwrap_or(0.0)
        })
    };
    let left = read(&data.left);
    (left, data.right.as_deref().map_or(left, read))
}

/// [`read_pcm`] for a streamed sample, voice `voice`'s of `rings`, linear: `None` when a frame is not there yet.
// ponytail: streamed samples stay linear at Quality High, as the ring may no longer hold the frame before; read it
// from the head or keep a frame back if that is ever heard.
fn read_disk(
    rings: &Rings,
    voice: usize,
    region: &Region,
    looping: bool,
    position: f64,
) -> Option<(f32, f32)> {
    let i = position as usize;
    let next = if looping && i + 1 >= region.loop_end {
        region.loop_start
    } else {
        (i + 1).min(region.end - 1)
    };
    let t = (position - i as f64) as f32;
    let (a, b) = (rings.frame(voice, i)?, rings.frame(voice, next)?);
    Some((a.0 + (b.0 - a.0) * t, a.1 + (b.1 - a.1) * t))
}

// Envelope laws. Every stage moves `level += (target − level)·c` each frame.
/// The attack's target; it stops at full (1).
const ATTACK_TARGET: f32 = 5.0;
/// Original: c = 8.82 / (Fs·t).
const ORIGINAL_RATE: f32 = 8.82;
/// The shortest stage time, in seconds.
const MIN_TIME: f32 = 0.005;
/// Below this level a releasing voice ends.
const SILENT: f32 = 0.01;
/// The release coefficient of a sample without a loop once it has played to its end.
const END_FADE: f32 = 0.01;
/// The param defaults: Enable, Attack, Decay, Sustain, Release, Envelope (Original), Fine Tune (0 cents), Vib Rate
/// (5 Hz), Vib Depth (0 cents), Vib Delay (0 s), Quality (Linear), Transpose (0).
const DEFAULTS: [f32; 16] = [
    1.0, 0.0, 0.0, 1.0, 0.0, 0.0, 0.5, 0.5, 0.0, 0.0, 0.0, 0.5, 0.0, 1.0, 0.0, 0.0,
];
/// Fine Tune, 200v − 100 cents on every voice: a Honeydew addition (#90).
const FINE_TUNE: usize = 6;
/// Honeydew additions (#102). Params 7–9, the module's own vibrato on every voice: a sine
/// of 10v Hz, 100v cents deep, from 2v s after the note-on.
const VIBRATO: usize = 7;
/// Param 10, Quality: above 0.5 (High) the samples are read by cubic interpolation, else linearly.
const QUALITY: usize = 10;
/// Param 11, Transpose: `round(48v) − 24` semitones on every voice, with Fine Tune. A Honeydew addition (#102).
const TRANSPOSE: usize = 11;
const FILTER_MODE: usize = 12;
const CUTOFF: usize = 13;
const RESONANCE: usize = 14;
const VIBRATO_MODE: usize = 15;
/// SoundFont 2's default modulator: the mod wheel (MIDI CC 1) deepens the vibrato LFO's pitch by up to 50 cents.
const WHEEL_CENTS: f32 = 50.0;
/// The vibrato LFO's rate at freqVibLFO 0 absolute cents.
const LFO_HZ: f32 = 8.176;
/// Channel param 4, Pitch Bend: v = 1 is 2 semitones up.
const PITCH_BEND: usize = 4;
const BEND_RANGE: f32 = 2.0;
// The SFZ's DAHDSR law: a linear attack; decay and release fall exponentially, e^−9 (−78 dB) in their time.
/// Below this level an SFZ voice's release ends: e^−9.
const SFZ_SILENT: f32 = 1.234e-4;
/// The shortest SFZ release, in seconds: a cut without a click.
const SFZ_MIN_RELEASE: f32 = 0.001;
/// How fast a voice another group's region stops (`off_mode=fast`) fades, in seconds.
const CHOKE_TIME: f32 = 0.006;
/// Frames per control block: an SFZ voice's modulation (LFOs, envelopes, CCs, its filter's tuning) moves once a block,
/// and its gain ramps across the block to where the modulation puts it.
const CONTROL: usize = 32;

/// The per-frame factor of a fall of e^−9 over `seconds`; 0 (at once) for none.
fn fall(seconds: f32, rate: f32) -> f32 {
    if seconds > 0.0 {
        (-9.0 / (seconds * rate)).exp()
    } else {
        0.0
    }
}

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
enum Stage {
    /// SFZ only: silent for `wait` frames.
    Delay,
    #[default]
    Attack,
    /// SFZ only: full for `wait` frames.
    Hold,
    Decay,
    Release,
}

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
enum ModStage {
    Delay,
    Attack,
    Hold,
    Decay,
    Sustain,
    Release,
    #[default]
    Done,
}

#[derive(Clone, Copy, Debug, Default)]
struct ModEnvState {
    stage: ModStage,
    wait: f32,
    attack: f32,
    hold: f32,
    decay: f32,
    sustain: f32,
    release: f32,
    level: f32,
}

impl ModEnvState {
    fn start(me: &ModEnv, key: u8) -> Self {
        if me.to_pitch == 0.0 && me.to_filter == 0.0 {
            return Self::default();
        }
        let tc_secs = |tc: f32| {
            if tc <= -11950.0 {
                0.0
            } else {
                2f32.powf(tc.clamp(-12000.0, 8000.0) / 1200.0)
            }
        };
        let key_delta = 60.0 - f32::from(key);
        let mut s = Self {
            stage: ModStage::Delay,
            wait: tc_secs(me.delay),
            attack: tc_secs(me.attack),
            hold: tc_secs(me.hold + me.key_hold * key_delta),
            decay: tc_secs(me.decay + me.key_decay * key_delta),
            sustain: me.sustain.clamp(0.0, 1.0),
            release: tc_secs(me.release),
            level: 0.0,
        };
        s.advance(0.0, false);
        s
    }

    fn advance(&mut self, mut dt: f32, released: bool) {
        if released && !matches!(self.stage, ModStage::Release | ModStage::Done) {
            self.stage = ModStage::Release;
        }
        loop {
            match self.stage {
                ModStage::Delay => {
                    if dt < self.wait {
                        self.wait -= dt;
                        break;
                    }
                    dt -= self.wait;
                    self.stage = ModStage::Attack;
                }
                ModStage::Attack => {
                    if self.attack <= 0.0 {
                        self.level = 1.0;
                        self.stage = ModStage::Hold;
                        self.wait = self.hold;
                    } else {
                        let need = (1.0 - self.level) * self.attack;
                        if dt < need {
                            self.level += dt / self.attack;
                            break;
                        }
                        dt -= need;
                        self.level = 1.0;
                        self.stage = ModStage::Hold;
                        self.wait = self.hold;
                    }
                }
                ModStage::Hold => {
                    if dt < self.wait {
                        self.wait -= dt;
                        break;
                    }
                    dt -= self.wait;
                    self.stage = ModStage::Decay;
                }
                ModStage::Decay => {
                    if self.decay <= 0.0 {
                        self.level = self.sustain;
                        self.stage = ModStage::Sustain;
                    } else {
                        let need = (self.level - self.sustain).max(0.0) * self.decay;
                        if dt < need {
                            self.level -= dt / self.decay;
                            break;
                        }
                        dt -= need;
                        self.level = self.sustain;
                        self.stage = ModStage::Sustain;
                    }
                }
                ModStage::Sustain => {
                    self.level = self.sustain;
                    break;
                }
                ModStage::Release => {
                    if self.release <= 0.0 {
                        self.level = 0.0;
                        self.stage = ModStage::Done;
                    } else {
                        self.level -= dt / self.release;
                        if self.level <= 0.0 {
                            self.level = 0.0;
                            self.stage = ModStage::Done;
                        }
                    }
                    break;
                }
                ModStage::Done => {
                    self.level = 0.0;
                    break;
                }
            }
        }
    }
}

#[derive(Clone, Copy, Debug, Default)]
struct Voice {
    on: bool,
    key: u8,
    /// When it started, for stealing.
    started: u64,
    region: usize,
    /// The sample frame it reads, and how far it moves per output frame before bend and slide.
    position: f64,
    step: f64,
    velocity: f32,
    pan: f32,
    /// Slide offsets: semitones, pan and velocity.
    slide: [f32; 3],
    level: f32,
    stage: Stage,
    /// Per-frame coefficients and the sustain level, latched at the note's start.
    attack: f32,
    decay: f32,
    sustain: f32,
    release: f32,
    // SFZ voices only.
    /// The SFZ's DAHDSR law ([`Self::dahdsr`]) with these coefficients: the attack's step, the decay's and release's
    /// factors. Else the law above (a SoundFont's, or the Custom envelope's).
    dahdsr: bool,
    /// Frames left of the delay or the hold, and the hold's length.
    wait: u32,
    hold: u32,
    /// A note-off does not release it: `one_shot`, or a release trigger's.
    one_shot: bool,
    /// Its key is up: a `loop_sustain` loop plays on to the end.
    released: bool,
    /// The velocity crossfade's gain, at the note's velocity.
    crossfade: f32,
    /// An SFZ voice's modulation.
    /// An SFZ voice's modulation.
    m: Motion,
    /// A SoundFont's modulation envelope.
    mod_env: ModEnvState,
    /// Its vibratos' phases (0…1), its region's and the module's ([`Self::vibrato`]), and the seconds since it started.
    vibrato: [f32; 3],
    time: f32,
}

/// What an SFZ voice's modulation keeps.
#[derive(Clone, Copy, Debug, Default)]
struct Motion {
    /// Frames since it started, for its LFOs' delay and fade; frames left before it plays (`delay`).
    age: u32,
    idle: u32,
    /// Its LFOs' phases (0…1), and its envelopes.
    lfo: [f32; MAX_LFOS],
    eg: [EgState; MAX_EGS],
    filter: Filter,
    /// What its note-on adds: cents, dB, the cutoff's cents, and pan (−100…100).
    pitch: f32,
    volume: f32,
    cutoff: f32,
    pan: f32,
    /// The level its amplifier envelope's attack starts from (`ampeg_start`).
    start: f32,
    /// Its left and right gains at the end of its last control block, which the next ramps from; `None` before its
    /// first.
    gain: Option<[f32; 2]>,
}

/// An envelope's place in its points: the point it moves to, from which level and how far in (seconds), and its level.
#[derive(Clone, Copy, Debug, Default)]
struct EgState {
    point: usize,
    from: f32,
    time: f32,
    level: f32,
    /// Its release has begun.
    released: bool,
}

impl EgState {
    fn new(eg: &Eg) -> Self {
        let level = eg.points.first().map_or(0.0, |p| p.1);
        Self {
            from: level,
            level,
            ..Self::default()
        }
    }

    /// Moves `dt` seconds on through `eg`'s points, straight from each to the next. It holds at the sustain point until
    /// `released`, which moves on from where it is to the point after.
    fn advance(&mut self, eg: &Eg, mut dt: f32, released: bool) {
        if released && !self.released {
            self.released = true;
            if let Some(sustain) = eg.sustain
                && self.point <= sustain + 1
            {
                (self.point, self.from, self.time) = (sustain + 1, self.level, 0.0);
            }
        }
        while let Some(&(seconds, to)) = eg.points.get(self.point) {
            if !self.released && eg.sustain.is_some_and(|s| self.point > s) {
                return;
            }
            let left = seconds - self.time;
            if dt < left {
                self.time += dt;
                self.level = self.from + (to - self.from) * self.time / seconds;
                return;
            }
            dt -= left.max(0.0);
            (self.point, self.from, self.time, self.level) = (self.point + 1, to, 0.0, to);
        }
    }
}

/// LFO wave `wave` ([`crate::sfz::Lfo::wave`]) at `phase` (0…1), −1…1.
fn wave(wave: u8, phase: f32) -> f32 {
    let pulse = |width: f32| if phase < width { 1.0 } else { -1.0 };
    match wave {
        0 => 1.0 - 4.0 * ((phase + 0.25).fract() - 0.5).abs(),
        2 => pulse(0.75),
        3 => pulse(0.5),
        4 => pulse(0.25),
        5 => pulse(0.125),
        6 => 2.0 * phase - 1.0,
        7 => 1.0 - 2.0 * phase,
        _ => (std::f32::consts::TAU * phase).sin(),
    }
}

/// An SFZ voice's filter (`fil_type`): one to three biquads in series (transposed direct form II) on one tuning, per
/// channel.
#[derive(Clone, Copy, Debug, Default)]
struct Filter {
    /// b0, b1, b2, a1, a2, over a0.
    c: [f32; 5],
    /// The biquads it runs; 0 passes the sound through.
    stages: usize,
    /// Per biquad and channel, its two states.
    z: [[[f32; 2]; 2]; 3],
}

impl Filter {
    /// Tunes it to `shape` and `poles` at `hz`, `resonance` dB over a Butterworth's Q (a shelf's or peak's gain `gain`
    /// dB), at `rate`. The 1-pole low- and high-pass are first-order; a band's 1 pole plays as 2.
    fn tune(
        &mut self,
        (shape, poles): (FilterShape, u8),
        hz: f32,
        resonance: f32,
        gain: f32,
        rate: f32,
    ) {
        use FilterShape::*;
        let w = std::f32::consts::TAU * hz.clamp(10.0, 0.45 * rate) / rate;
        let (sin, cos) = w.sin_cos();
        let q = std::f32::consts::FRAC_1_SQRT_2 * 10f32.powf(resonance.clamp(-40.0, 40.0) / 20.0);
        let alpha = sin / (2.0 * q);
        let a = 10f32.powf(gain.clamp(-96.0, 24.0) / 40.0);
        let (plus, minus, shelf) = (a + 1.0, a - 1.0, 2.0 * a.sqrt() * alpha);
        let [b0, b1, b2, a0, a1, a2] = match shape {
            LowPass | HighPass if poles == 1 => {
                let k = (w / 2.0).tan();
                let [b0, b1] = if shape == LowPass {
                    [k, k]
                } else {
                    [1.0, -1.0]
                };
                [b0, b1, 0.0, 1.0 + k, k - 1.0, 0.0]
            }
            LowPass => [
                (1.0 - cos) / 2.0,
                1.0 - cos,
                (1.0 - cos) / 2.0,
                1.0 + alpha,
                -2.0 * cos,
                1.0 - alpha,
            ],
            HighPass => [
                (1.0 + cos) / 2.0,
                -1.0 - cos,
                (1.0 + cos) / 2.0,
                1.0 + alpha,
                -2.0 * cos,
                1.0 - alpha,
            ],
            BandPass => [alpha, 0.0, -alpha, 1.0 + alpha, -2.0 * cos, 1.0 - alpha],
            BandReject => [1.0, -2.0 * cos, 1.0, 1.0 + alpha, -2.0 * cos, 1.0 - alpha],
            Peak => [
                1.0 + alpha * a,
                -2.0 * cos,
                1.0 - alpha * a,
                1.0 + alpha / a,
                -2.0 * cos,
                1.0 - alpha / a,
            ],
            LowShelf => [
                a * (plus - minus * cos + shelf),
                2.0 * a * (minus - plus * cos),
                a * (plus - minus * cos - shelf),
                plus + minus * cos + shelf,
                -2.0 * (minus + plus * cos),
                plus + minus * cos - shelf,
            ],
            HighShelf => [
                a * (plus + minus * cos + shelf),
                -2.0 * a * (minus + plus * cos),
                a * (plus + minus * cos - shelf),
                plus - minus * cos + shelf,
                2.0 * (minus - plus * cos),
                plus - minus * cos - shelf,
            ],
        };
        self.c = [b0 / a0, b1 / a0, b2 / a0, a1 / a0, a2 / a0];
        // ponytail: 4 and 6 poles are the 2-pole biquad two and three times over, its resonance compounding; give the
        // later ones a Butterworth Q if a resonant `lpf_4p` ever sounds too sharp.
        self.stages = match poles {
            4 => 2,
            6 => 3,
            _ => 1,
        };
    }

    /// One frame of each channel through it.
    fn run(&mut self, (l, r): (f32, f32)) -> (f32, f32) {
        let [b0, b1, b2, a1, a2] = self.c;
        let mut out = [l, r];
        for stage in &mut self.z[..self.stages] {
            for (x, z) in out.iter_mut().zip(stage) {
                let y = b0 * *x + z[0];
                z[0] = flush(b1 * *x - a1 * y + z[1]);
                z[1] = flush(b2 * *x - a2 * y);
                *x = y;
            }
        }
        (out[0], out[1])
    }

    /// One mono frame through channel 0.
    fn run_mono(&mut self, mut x: f32) -> f32 {
        let [b0, b1, b2, a1, a2] = self.c;
        for stage in &mut self.z[..self.stages] {
            let z = &mut stage[0];
            let y = b0 * x + z[0];
            z[0] = flush(b1 * x - a1 * y + z[1]);
            z[1] = flush(b2 * x - a2 * y);
            x = y;
        }
        x
    }
}

/// What an SFZ voice's control block reads besides its region: the CCs (0…1) at the process block's start (`then`)
/// and end (`now`), the pitch bend (−1…1), Fine Tune and Transpose (cents), the module's vibrato (cents, Hz, seconds),
/// Quality High (`cubic`) and the rate.
struct Controls<'a> {
    then: &'a [f32; 128],
    now: &'a [f32; 128],
    bend: f32,
    fine: f32,
    vibrato: [f32; 3],
    cubic: bool,
    rate: f32,
}

impl Voice {
    /// The cents its vibratos move its pitch by now, then moves them `dt` seconds on: its region's (a SoundFont's
    /// vibrato LFO, a triangle, `wheel` cents deeper) and the module's (`module`, a sine), each from 0, rising, once its
    /// delay is over.
    fn vibrato(
        &mut self,
        region: &Region,
        wheel: f32,
        module: [f32; 3],
        custom: bool,
        dt: f32,
    ) -> (f32, f32) {
        let [depth, hz, delay] = region.vibrato;
        let [mod_depth, mod_hz, mod_delay] = region.mod_lfo;
        let lfos = if custom {
            [(0, [0.0; 3]), (0, [0.0, mod_hz, mod_delay]), (1, module)]
        } else {
            [
                (0, [depth + wheel, hz, delay]),
                (0, [mod_depth, mod_hz, mod_delay]),
                (1, module),
            ]
        };
        let mut cents = 0.0;
        let mut mod_val = 0.0;
        for (i, (phase, (shape, [depth, hz, delay]))) in
            self.vibrato.iter_mut().zip(lfos).enumerate()
        {
            if self.time >= delay {
                let w = wave(shape, *phase);
                cents += depth * w;
                if i == 1 {
                    mod_val = w;
                }
                *phase = (*phase + hz * dt).fract();
            }
        }
        self.time += dt;
        (cents, mod_val)
    }

    /// One frame of the envelope; false once the voice has ended.
    fn envelope(&mut self) -> bool {
        if self.dahdsr {
            return self.dahdsr();
        }
        match self.stage {
            Stage::Delay => {
                if self.wait > 0 {
                    self.wait -= 1;
                }
                if self.wait == 0 {
                    self.stage = Stage::Attack;
                }
            }
            Stage::Attack => {
                self.level += (ATTACK_TARGET - self.level) * self.attack;
                if self.level >= 1.0 {
                    self.level = 1.0;
                    self.stage = if self.hold > 0 {
                        self.wait = self.hold;
                        Stage::Hold
                    } else {
                        Stage::Decay
                    };
                }
            }
            Stage::Hold => {
                if self.wait > 0 {
                    self.wait -= 1;
                }
                if self.wait == 0 {
                    self.stage = Stage::Decay;
                }
            }
            Stage::Decay => self.level += (self.sustain - self.level) * self.decay,
            Stage::Release => {
                self.level -= self.level * self.release;
                if self.level < SILENT {
                    self.on = false;
                    return false;
                }
            }
        }
        true
    }

    /// One frame of the SFZ's DAHDSR envelope; false once the voice has ended.
    fn dahdsr(&mut self) -> bool {
        match self.stage {
            Stage::Delay | Stage::Hold if self.wait > 0 => self.wait -= 1,
            Stage::Delay => (self.stage, self.level) = (Stage::Attack, self.m.start),
            Stage::Hold => self.stage = Stage::Decay,
            Stage::Attack => {
                self.level += self.attack;
                if self.level >= 1.0 {
                    (self.level, self.stage, self.wait) = (1.0, Stage::Hold, self.hold);
                }
            }
            Stage::Decay => self.level = (self.level * self.decay).max(self.sustain),
            Stage::Release => {
                self.level *= self.release;
                if self.level < SFZ_SILENT {
                    self.on = false;
                    return false;
                }
            }
        }
        true
    }

    /// Stops it for another group's region: its own release with `normal`, else a fade of [`CHOKE_TIME`].
    fn choke(&mut self, normal: bool, rate: f32) {
        (self.stage, self.one_shot, self.released) = (Stage::Release, false, true);
        if !normal {
            let factor = fall(CHOKE_TIME, rate);
            self.release = if self.dahdsr { factor } else { 1.0 - factor };
        }
    }

    /// Whether an SFZ voice of `region` (rules `rules`) loops now: a `loop_continuous` one, unless it is a release
    /// trigger's (which plays through), or a `loop_sustain` one while its key is held.
    fn loops(&self, region: &Region, rules: &Rules) -> bool {
        region.loop_end > region.loop_start
            && match rules.mode {
                LoopMode::Continuous => !(self.one_shot && self.released),
                LoopMode::Sustain => !self.released,
                LoopMode::NoLoop | LoopMode::OneShot => false,
            }
    }

    /// Moves an SFZ voice `step` frames on through `region`'s sample: round its loop while `looping`, else to its end,
    /// where it ends.
    fn advance_pcm(&mut self, region: &Region, looping: bool, step: f64) {
        self.position += step;
        if looping {
            if self.position >= region.loop_end as f64 {
                let start = region.loop_start as f64;
                let length = (region.loop_end - region.loop_start) as f64;
                self.position = start + (self.position - start) % length;
            }
        } else if self.position >= region.end as f64 {
            self.on = false;
        }
    }

    /// An SFZ voice's next control block, `frames` frames ending `at` (0…1) through the process block: moves its LFOs
    /// and envelopes on, sums its routes, tunes its filter, and gives its step and its left and right gains there.
    fn control(
        &mut self,
        region: &Region,
        rules: &Rules,
        font: &SoundFont,
        ctl: &Controls,
        at: f32,
        frames: usize,
    ) -> (f64, [f32; 2]) {
        let def = &*rules.def;
        let cc = |c: u8| {
            let c = usize::from(c.min(127));
            ctl.then[c] + (ctl.now[c] - ctl.then[c]) * at
        };
        let dt = frames as f32 / ctl.rate;
        let age = self.m.age as f32 / ctl.rate;
        let mut lfo = [0.0; MAX_LFOS];
        for ((out, phase), l) in lfo.iter_mut().zip(&mut self.m.lfo).zip(&def.lfos) {
            let fade = match age - l.delay {
                t if t < 0.0 => 0.0,
                t if l.fade > 0.0 => (t / l.fade).min(1.0),
                _ => 1.0,
            };
            *out = wave(l.wave, *phase) * fade;
            let hz = l.freq + l.freq_cc.map_or(0.0, |(c, d)| cc(c) * d);
            *phase = (*phase + hz * dt).rem_euclid(1.0);
        }
        for (state, eg) in self.m.eg.iter_mut().zip(&def.egs) {
            state.advance(eg, dt, self.released);
        }
        self.m.age = self.m.age.saturating_add(frames as u32);
        // By [`Target`]: volume, (amplitude), pan, pitch, cutoff and resonance add up; the amplitude multiplies.
        let (mut sum, mut amplitude) = ([0.0_f32; 6], 1.0_f32);
        for r in &def.routes {
            let x = match r.source {
                Source::Cc(c) => font.curve(r.curve, cc(c)),
                Source::Lfo(i) => lfo.get(i).copied().unwrap_or(0.0),
                Source::Eg(i) => self.m.eg.get(i).map_or(0.0, |e| e.level),
            };
            let depth = r.depth + r.depth_cc.map_or(0.0, |(c, d)| cc(c) * d);
            match (r.target, r.source) {
                (Target::Amplitude, Source::Lfo(_)) => amplitude *= 1.0 + x * depth / 100.0,
                (Target::Amplitude, _) => amplitude *= x * depth / 100.0,
                (target, _) => sum[target as usize] += x * depth,
            }
        }
        for &(c, fade_in, fade_out) in &def.xf_cc {
            amplitude *= crossfade(cc(c), fade_in, fade_out);
        }
        let amplitude = if amplitude.is_finite() {
            amplitude.clamp(-16.0, 16.0)
        } else {
            0.0
        };
        let [volume, _, pan, pitch, cutoff, resonance] = sum;
        let bend = match ctl.bend >= 0.0 {
            true => ctl.bend * def.bend[0],
            false => -ctl.bend * def.bend[1],
        };
        let (vibrato, _) = self.vibrato(region, 0.0, ctl.vibrato, false, dt);
        let cents = pitch + self.m.pitch + bend + ctl.fine + vibrato + 100.0 * self.slide[0];
        // `min` also turns an infinite or NaN step into the cap.
        let step = (self.step * 2f64.powf(f64::from(cents) / 1200.0)).min(MAX_STEP);
        if let (Some(hz), Some(fil)) = (def.cutoff, def.fil_type) {
            let hz = hz * 2f32.powf((cutoff + self.m.cutoff) / 1200.0);
            (self.m.filter).tune(fil, hz, def.resonance + resonance, def.fil_gain, ctl.rate);
        }
        let velocity = (self.velocity + self.slide[2]).clamp(0.0, 1.0);
        let db = (volume + self.m.volume).clamp(-144.0, 24.0);
        let gain = rules.velocity_gain(&font.curves, velocity)
            * self.crossfade
            * region.gain
            * amplitude
            * 10f32.powf(db / 20.0);
        let p =
            (self.pan + self.slide[1] + region.pan + (pan + self.m.pan) / 200.0).clamp(0.0, 1.0);
        (
            step,
            [gain * (2.0 - 2.0 * p).min(1.0), gain * (2.0 * p).min(1.0)],
        )
    }

    /// Moves `step` frames on through `region`'s sample: round its loop while `looping`, or up to its last frame, where it fades.
    fn advance(&mut self, region: &Region, looping: bool, step: f64) {
        self.position += step;
        if looping {
            if self.position >= region.loop_end as f64 {
                let start = region.loop_start as f64;
                let length = (region.loop_end - region.loop_start) as f64;
                self.position = start + (self.position - start) % length;
            }
        } else if self.position >= (region.end - 1) as f64 {
            self.position = (region.end - 1) as f64;
            self.stage = Stage::Release;
            self.release = self.release.max(END_FADE);
        }
    }
}

/// Renders SFZ voice `voice` (slot `slot`) of `region` into `left` and `right`, a control block at a time
/// ([`Voice::control`]): its sample, from memory or `rings`, through its filter, at its envelope's level and its gains
/// ramped across each block.
#[allow(clippy::too_many_arguments)]
fn render_sfz(
    voice: &mut Voice,
    slot: usize,
    region: &Region,
    rules: &Rules,
    font: &SoundFont,
    rings: Option<&Rings>,
    controls: &Controls,
    left: &mut [f32],
    right: &mut [f32],
) {
    let sample = &font.pcm[rules.sample];
    let disk = rings.filter(|_| matches!(sample, Pcm::Disk(_)));
    if let Some(rings) = disk {
        rings.advance(slot, voice.position as usize);
    }
    let (frames, mut short, mut at) = (left.len(), false, 0);
    while at < frames && voice.on {
        let n = CONTROL.min(frames - at);
        let end = (at + n) as f32 / frames as f32;
        let (step, gain) = voice.control(region, rules, font, controls, end, n);
        let mut g = voice.m.gain.unwrap_or(gain);
        let ramp = [0, 1].map(|c| (gain[c] - g[c]) / n as f32);
        let looping = voice.loops(region, rules);
        for (l, r) in left[at..at + n].iter_mut().zip(&mut right[at..at + n]) {
            g = [g[0] + ramp[0], g[1] + ramp[1]];
            if voice.m.idle > 0 {
                voice.m.idle -= 1;
                continue;
            }
            if !voice.on || !voice.envelope() {
                break;
            }
            let (a, b) = match (sample, disk) {
                (Pcm::Ram(data), _) => {
                    read_pcm(data, region, looping, voice.position, controls.cubic)
                }
                (Pcm::Disk(_), Some(rings)) => {
                    let read = read_disk(rings, slot, region, looping, voice.position);
                    short |= read.is_none();
                    read.unwrap_or_default()
                }
                (Pcm::Disk(_), None) => (0.0, 0.0),
            };
            let (a, b) = voice.m.filter.run((a, b));
            *l += a * voice.level * g[0];
            *r += b * voice.level * g[1];
            let before = voice.position;
            voice.advance_pcm(region, looping, step);
            // A wrap reads the loop from its start: its head, then a new segment, unless it is all in memory.
            if let Some(rings) = disk
                && voice.position < before
                && !rings.in_memory(rules.sample, region.loop_start, region.loop_end)
            {
                rings.start(slot, rules.sample, region.loop_start, false);
            }
        }
        voice.m.gain = Some(gain);
        at += n;
    }
    if let Some(rings) = rings {
        if short {
            rings.underrun();
        }
        if !voice.on {
            rings.stop(slot);
        }
    }
}

/// The voice a note takes: a free one, else the oldest-started. With `rings` (a player that streams), a streamed
/// sample's takes one of the first [`STREAM_VOICES`], which have rings; any other takes a free one past them first.
fn slot(voices: &[Voice], rings: Option<&Rings>, sample: Option<usize>) -> usize {
    let pick = |slots: Range<usize>| {
        let oldest = slots.min_by_key(|&s| (voices[s].on, voices[s].started));
        oldest.unwrap_or(0)
    };
    let Some(rings) = rings else {
        return pick(0..VOICES);
    };
    if sample.is_some_and(|s| rings.streams(s)) {
        return pick(0..STREAM_VOICES);
    }
    Some(pick(STREAM_VOICES..VOICES))
        .filter(|&s| !voices[s].on)
        .unwrap_or_else(|| pick(0..VOICES))
}

/// The SoundFont Player: the regions of one patch of a [`SoundFont`], and [`VOICES`] voices.
pub struct SoundFontPlayer {
    font: Option<Arc<SoundFont>>,
    /// The first [`STREAM_VOICES`] voices' rings, when an SFZ sample streams from disk.
    rings: Option<Arc<Rings>>,
    regions: Vec<Region>,
    /// Boxed: [`VOICES`] voices are too big for a small stack.
    voices: Box<[Voice]>,
    started: u64,
    sample_rate: f32,
    params: [f32; DEFAULTS.len()],
    /// Pitch bend in semitones, and as the wheel stands (−1…1), which an SFZ's `bend_up` and `bend_down` scale.
    bend: f32,
    wheel: f32,
    // An SFZ's state.
    /// The notes each region has met, for its round robin.
    seq: Vec<u32>,
    /// The keys held, with their note-on velocity, pan and frame: for release triggers, `first`, `legato`, `sw_down`
    /// and `sw_up`.
    held: [Option<(f32, f32, u64)>; 128],
    /// The keyswitch in force, and the last note-on's key (`sw_previous`).
    switch: Option<u8>,
    previous: Option<u8>,
    /// The random layers' generator (xorshift32).
    random: u32,
    /// The frames rendered.
    clock: u64,
    /// The CCs (0…1): as last set, where their smoothing stood at the last block's start, and where at its end.
    cc: [f32; 128],
    cc_then: [f32; 128],
    cc_now: [f32; 128],
    base_regions: Vec<Region>,
    zone: usize,
}

/// The next of xorshift32 `state`, 0…1.
fn dice(state: &mut u32) -> f32 {
    *state ^= *state << 13;
    *state ^= *state >> 17;
    *state ^= *state << 5;
    (*state >> 8) as f32 / (1 << 24) as f32
}

impl SoundFontPlayer {
    /// A player of patch `patch` of `font` (the `SF2D` index into its patches in file order; out of range is
    /// the nearest end), or a silent one without a SoundFont. App thread: the patch's regions are resolved here.
    pub fn new(sample_rate: f32, bank: Option<(Arc<SoundFont>, i32)>) -> Self {
        let regions = bank.as_ref().map(|(font, patch)| {
            let last = font.presets.len().saturating_sub(1);
            font.regions(usize::try_from(*patch).unwrap_or(0).min(last))
        });
        let regions = regions.unwrap_or_default();
        let disk = bank
            .as_ref()
            .map(|(font, _)| font.pcm.iter().map(Pcm::disk).collect());
        let cc = bank
            .as_ref()
            .map_or([0.0; 128], |(font, _)| font.cc_defaults);
        let base_regions = regions.clone();
        Self {
            rings: disk.and_then(Rings::new),
            switch: bank.as_ref().and_then(|(font, _)| font.switch),
            seq: vec![0; regions.len()],
            font: bank.map(|(font, _)| font),
            regions,
            voices: vec![Voice::default(); VOICES].into_boxed_slice(),
            started: 0,
            sample_rate,
            params: DEFAULTS,
            bend: 0.0,
            wheel: 0.0,
            held: [None; 128],
            previous: None,
            random: 0x9E37_79B9,
            clock: 0,
            cc,
            cc_then: cc,
            cc_now: cc,
            base_regions,
            zone: 0,
        }
    }

    /// Its first [`STREAM_VOICES`] voices' rings when it streams a sample from disk: for the engine's streamer to fill
    /// ([`crate::stream::Streamer::register`]).
    pub fn rings(&self) -> Option<&Arc<Rings>> {
        self.rings.as_ref()
    }

    /// Whether it plays an SFZ.
    fn is_sfz(&self) -> bool {
        self.font
            .as_ref()
            .is_some_and(|font| !font.sfz_regions.is_empty())
    }

    /// Starts the SFZ regions that a note-on of `key` at `velocity` (0…1) and `pan` plays, or with `held` (the seconds
    /// since its note-on) its note-off, at the note-on's velocity. A region plays when its key, velocity, round robin,
    /// random, keyswitch (`sw_last`, `sw_down`, `sw_up`, `sw_previous`), CC (`loccN`) and bend ranges all hold. Of the
    /// regions it plays, past [`VOICES`] only the last ones start. Each stops the voices whose region's `off_by` names
    /// its group first.
    fn sfz_note(&mut self, key: u8, velocity: f32, pan: f32, held: Option<f32>) {
        let Some(font) = &self.font else {
            return;
        };
        let release = held.is_some();
        let steps = (127.0 * velocity).round() as u8;
        if !release
            && font
                .switches
                .is_some_and(|[lo, hi]| (lo..=hi).contains(&key))
        {
            self.switch = Some(key);
        }
        let others =
            (self.held.iter().enumerate()).any(|(k, h)| h.is_some() && k != usize::from(key));
        if !release && let Some(held) = self.held.get_mut(usize::from(key)) {
            *held = Some((velocity, pan, self.clock));
        }
        let dice = dice(&mut self.random);
        let down = |k: i32| {
            usize::try_from(k).is_ok_and(|k| self.held.get(k).is_some_and(Option::is_some))
        };
        let bend = (self.wheel * 8192.0).round() as i32;
        // A ring of the last [`VOICES`] chosen (4 KiB of stack), the `count`th at `count % VOICES`.
        let (mut chosen, mut count) = ([0; VOICES], 0);
        for (index, region) in self.regions.iter().enumerate() {
            let Some(rules) = &region.sfz else {
                continue;
            };
            let fires = match rules.trigger {
                Trigger::Attack => !release,
                Trigger::Release => release,
                Trigger::First => !release && !others,
                Trigger::Legato => !release && others,
            };
            if !fires || !region.plays(key, steps) {
                continue;
            }
            let turn = self.seq[index];
            self.seq[index] = turn.wrapping_add(1);
            let def = &rules.def;
            let switched = |[lo, hi]: [u8; 2]| self.switch.is_some_and(|s| (lo..=hi).contains(&s));
            let in_range = |&(c, [lo, hi]): &(u8, [f32; 2])| {
                let v = self.cc[usize::from(c.min(127))];
                lo - 1e-4 <= v && v <= hi + 1e-4
            };
            let plays = turn % rules.seq[0] == rules.seq[1]
                && (rules.rand[0] <= dice && dice < rules.rand[1])
                && rules.sw_last.is_none_or(switched)
                && def.sw_down.is_none_or(down)
                && def.sw_up.is_none_or(|k| !down(k))
                && (def.sw_previous).is_none_or(|k| self.previous.map(i32::from) == Some(k))
                && def.cc_ranges.iter().all(in_range)
                && (def.bends[0]..=def.bends[1]).contains(&bend);
            if plays {
                chosen[count % VOICES] = index;
                count += 1;
            }
        }
        if !release {
            self.previous = Some(key);
        }
        let chosen = (count.saturating_sub(VOICES)..count).map(|i| chosen[i % VOICES]);
        for index in chosen.clone() {
            let group = self.regions[index].sfz.as_ref().map(|r| r.group);
            for voice in self.voices.iter_mut().filter(|v| v.on) {
                let rules = self.regions[voice.region].sfz.as_ref();
                if let Some(rules) = rules.filter(|r| r.off_by.is_some() && r.off_by == group) {
                    voice.choke(rules.off_normal, self.sample_rate);
                }
            }
        }
        for index in chosen {
            self.sfz_start(index, key, velocity, pan, held);
        }
    }

    /// Starts a voice of SFZ region `index` (a release trigger's with `held`, the seconds since its note-on). Past its
    /// group's `polyphony` (and `note_polyphony` on its key) the oldest of its group's voices still held stop first.
    fn sfz_start(&mut self, index: usize, key: u8, velocity: f32, pan: f32, held: Option<f32>) {
        let region = &self.regions[index];
        let Some(rules) = &region.sfz else {
            return;
        };
        let def = &*rules.def;
        let rate = self.sample_rate;
        let regions = &self.regions;
        let group = rules.group;
        for (limit, one_key) in [(def.polyphony, false), (def.note_polyphony, true)] {
            let Some(limit) = limit else {
                continue;
            };
            let ours = |v: &Voice| {
                v.on && v.stage != Stage::Release
                    && (!one_key || v.key == key)
                    && regions[v.region]
                        .sfz
                        .as_ref()
                        .is_some_and(|r| r.group == group)
            };
            let sounding = self.voices.iter().filter(|v| ours(v)).count();
            for _ in limit as usize..=sounding {
                let oldest = self.voices.iter_mut().filter(|v| ours(v));
                if let Some(voice) = oldest.min_by_key(|v| v.started) {
                    voice.choke(false, rate);
                }
            }
        }
        let cents = (f64::from(key) - f64::from(region.root)) * region.scale + rules.cents;
        let step = 2f64.powf(cents / 1200.0) * region.rate / f64::from(rate);
        let custom = self.params[5] > 0.5;
        // The stages as velocity (`ampeg_vel2*`) and the CCs (`ampeg_*ccN`) move them; sustain is a level, the rest
        // seconds.
        let mut ampeg = rules.ampeg;
        let scale = |stage: usize| if stage == 4 { 0.01 } else { 1.0 };
        for (stage, add) in def.ampeg_vel2.iter().enumerate() {
            ampeg[stage] += add * velocity * scale(stage);
        }
        for &(stage, cc, add) in &def.ampeg_cc {
            ampeg[stage] += add * self.cc[usize::from(cc.min(127))] * scale(stage);
        }
        let [delay, attack, hold, decay, sustain, release_time] = ampeg.map(|x| x.max(0.0));
        let start = if custom { 0.0 } else { def.ampeg_start / 100.0 };
        let frames = |seconds: f32| (seconds * rate) as u32;
        let envelope = match custom {
            true => self.envelope(region),
            false => [
                ((1.0 - start) / (attack * rate)).min(1.0),
                fall(decay, rate),
                sustain.min(1.0),
                fall(release_time.max(SFZ_MIN_RELEASE), rate),
            ],
        };
        let mut spread = |x: f32| x * (2.0 * dice(&mut self.random) - 1.0);
        let [amp_random, pitch_random, fil_random, pan_random] = def.random;
        let from_center = |center: i32| f32::from(key) - center as f32;
        let m = Motion {
            idle: frames(def.delay),
            lfo: std::array::from_fn(|i| def.lfos.get(i).map_or(0.0, |l| l.phase)),
            eg: std::array::from_fn(|i| def.egs.get(i).map(EgState::new).unwrap_or_default()),
            pitch: def.pitch_veltrack * velocity + spread(pitch_random),
            volume: def.amp_keytrack * from_center(def.amp_keycenter)
                + amp_random * (spread(0.5) + 0.5)
                - def.rt_decay * held.unwrap_or(0.0),
            cutoff: def.fil_keytrack * from_center(def.fil_keycenter)
                + def.fil_veltrack * velocity
                + spread(fil_random),
            pan: spread(pan_random),
            start,
            ..Motion::default()
        };
        let slot = slot(&self.voices, self.rings.as_deref(), Some(rules.sample));
        if let Some(rings) = &self.rings {
            rings.start(slot, rules.sample, region.start, false);
        }
        self.started += 1;
        self.voices[slot] = Voice {
            on: true,
            key,
            started: self.started,
            region: index,
            position: region.start as f64,
            step,
            velocity,
            pan,
            level: if custom || delay > 0.0 { 0.0 } else { start },
            stage: if custom || delay <= 0.0 {
                Stage::Attack
            } else {
                Stage::Delay
            },
            attack: envelope[0],
            decay: envelope[1],
            sustain: envelope[2],
            release: envelope[3],
            dahdsr: !custom,
            wait: frames(delay),
            hold: frames(hold),
            one_shot: held.is_some() || rules.mode == LoopMode::OneShot,
            released: held.is_some(),
            crossfade: rules.crossfade((127.0 * velocity).round() as u8),
            m,
            ..Voice::default()
        };
    }

    /// Slide offsets for every sounding voice: `pitch` semitones, `pan` and `velocity` added to the
    /// note's. A voice that starts later starts without them.
    pub fn set_slide(&mut self, pitch: f32, pan: f32, velocity: f32) {
        let finite = |x: f32| if x.is_finite() { x } else { 0.0 };
        for voice in self.voices.iter_mut().filter(|v| v.on) {
            voice.slide = [finite(pitch), finite(pan), finite(velocity)];
        }
    }

    /// The voices sounding.
    pub fn active_voices(&self) -> usize {
        self.voices.iter().filter(|v| v.on).count()
    }

    fn vol_env(rate: f32, params: &[f32], region: &Region, key: u8) -> ([f32; 4], u32, u32) {
        if params[5] > 0.5 {
            let time = |v: f32| (10.0 * 0.95f32.mul_add(v, 0.05).powi(3)).max(MIN_TIME);
            let x = (127.0 * params[3]).round().clamp(1.0, 127.0) / 127.0;
            return (
                [
                    (1.25f32.ln() / (rate * time(params[1]))).min(1.0),
                    (100f32.ln() / (rate * time(params[2]))).min(1.0),
                    10f32.powf(5.0 * x.powf(0.2) - 5.0),
                    (100f32.ln() / (rate * time(params[4]))).min(1.0),
                ],
                0,
                0,
            );
        }
        let stage = |num: f32, tc: f32| num / (rate * 2f32.powf(tc / 1200.0).max(MIN_TIME));
        let frames = |tc: f32| {
            if tc <= -11950.0 {
                0
            } else {
                (2f32.powf(tc.clamp(-12000.0, 5000.0) / 1200.0) * rate).round() as u32
            }
        };
        let key_delta = 60.0 - f32::from(key);
        let delay = frames(region.vol_delay);
        let hold = frames(region.vol_hold + region.key_vol_hold * key_delta);
        let decay_tc = (region.decay + region.key_vol_decay * key_delta).clamp(-12000.0, 8000.0);
        (
            [
                stage(1.25f32.ln(), region.attack).min(1.0),
                stage(ORIGINAL_RATE, decay_tc).min(1.0),
                region.sustain,
                stage(ORIGINAL_RATE, region.release).min(1.0),
            ],
            delay,
            hold,
        )
    }

    fn vol_envelope(&self, region: &Region, key: u8) -> ([f32; 4], u32, u32) {
        Self::vol_env(self.sample_rate, &self.params, region, key)
    }

    fn envelope(&self, region: &Region) -> [f32; 4] {
        self.vol_envelope(region, 60).0
    }

    pub fn apply_zone_overrides(&mut self, overrides: &[ZoneOverride]) {
        self.regions = self.base_regions.clone();
        for ov in overrides {
            if let Some(r) = self.regions.get_mut(usize::from(ov.zone)) {
                for field in 0..ZONE_FIELDS {
                    if ov.mask & (1u64 << field) != 0 {
                        apply_zone_field(r, field, ov.values[field]);
                    }
                }
            }
        }
    }

    fn refresh_zone_voices(&mut self, zone: usize) {
        let Some(r) = self.regions.get(zone).cloned() else {
            return;
        };
        let (rate_f32, params) = (self.sample_rate, self.params);
        let rate = f64::from(rate_f32);
        for voice in self.voices.iter_mut().filter(|v| v.on && v.region == zone) {
            let eff_key = r.fixed_key.unwrap_or(voice.key);
            let ([attack, decay, sustain, release], _, hold) =
                Self::vol_env(rate_f32, &params, &r, eff_key);
            let octaves =
                (f64::from(eff_key) - f64::from(r.root) + r.tune) * r.scale / 1200.0;
            voice.step = 2f64.powf(octaves) * r.rate / rate;
            if let Some(fv) = r.fixed_vel {
                voice.velocity = f32::from(fv) / 127.0;
            }
            voice.position = voice.position.clamp(r.start as f64, (r.end - 1) as f64);
            (voice.attack, voice.decay, voice.sustain, voice.release, voice.hold) =
                (attack, decay, sustain, release, hold);
        }
    }
}

impl Instrument for SoundFontPlayer {
    /// Params 1–4 (Attack, Decay, Sustain, Release), 5 (Envelope), 6 (Fine Tune), 7–9 (Vib Rate, Depth, Delay), 10
    /// (Quality) and 11 (Transpose).
    fn set_parameter(&mut self, index: usize, value: f32) {
        if let Some(param) = self.params.get_mut(index).filter(|_| value.is_finite()) {
            *param = value;
        }
    }

    /// One voice per region that plays `key` at `velocity` (checked as trunc(127·velocity)). Past [`VOICES`] matches,
    /// only the last ones start: each later voice would steal the earlier ones anyway.
    fn note_on(&mut self, key: u8, velocity: f32, pan: f32) {
        let velocity = if velocity.is_finite() {
            velocity.clamp(0.0, 1.0)
        } else {
            0.0
        };
        let pan = if pan.is_finite() {
            pan.clamp(0.0, 1.0)
        } else {
            0.5
        };
        if self.is_sfz() {
            return self.sfz_note(key, velocity, pan, None);
        }
        let before = self.started;
        let steps = (127.0 * velocity) as u8;
        let matches = self.regions.iter().filter(|r| r.plays(key, steps));
        let skip = matches.count().saturating_sub(VOICES);
        for (_, region) in self.regions.iter().enumerate().filter(|(_, r)| r.plays(key, steps)).skip(skip) {
            if region.exclusive != 0 {
                let target = region.exclusive;
                let regions = &self.regions;
                for voice in self.voices.iter_mut().filter(|v| {
                    v.on && v.started <= before && regions[v.region].exclusive == target
                }) {
                    voice.choke(false, self.sample_rate);
                }
            }
        }
        let regions = self.regions.iter().enumerate();
        for (index, region) in regions.filter(|(_, r)| r.plays(key, steps)).skip(skip) {
            let eff_key = region.fixed_key.unwrap_or(key);
            let eff_vel = region.fixed_vel.map_or(velocity, |v| f32::from(v) / 127.0);
            let ([attack, decay, sustain, release], delay, hold) =
                self.vol_envelope(region, eff_key);
            let octaves =
                (f64::from(eff_key) - f64::from(region.root) + region.tune) * region.scale / 1200.0;
            let step = 2f64.powf(octaves) * region.rate / f64::from(self.sample_rate);
            let slot = slot(&self.voices, None, None);
            self.started += 1;
            let mod_env = ModEnvState::start(&region.mod_env, eff_key);
            let mut filter = Filter::default();
            if let Some([hz, resonance]) = region.filter {
                let hz = hz * 2f32.powf((mod_env.level * region.mod_env.to_filter) / 1200.0);
                filter.tune(
                    (FilterShape::LowPass, 2),
                    hz,
                    resonance,
                    0.0,
                    self.sample_rate,
                );
            }
            
            self.voices[slot] = Voice {
                on: true,
                key,
                started: self.started,
                region: index,
                position: region.start as f64,
                step,
                velocity: eff_vel,
                pan,
                attack,
                decay,
                sustain,
                release,
                mod_env,
                stage: if delay > 0 { Stage::Delay } else { Stage::Attack },
                wait: delay,
                hold,
                m: Motion {
                    filter,
                    ..Motion::default()
                },
                ..Voice::default()
            };
        }
    }

    /// An SFZ's one-shot voices play on, and its release triggers start.
    fn note_off(&mut self, key: u8) {
        let sfz = self.is_sfz();
        for voice in self
            .voices
            .iter_mut()
            .filter(|v| v.on && v.key == key && !v.one_shot)
        {
            voice.stage = Stage::Release;
            voice.released = true;
        }
        if sfz
            && let Some((velocity, pan, on)) =
                self.held.get_mut(usize::from(key)).and_then(Option::take)
        {
            let held = self.clock.saturating_sub(on) as f32 / self.sample_rate;
            self.sfz_note(key, velocity, pan, Some(held));
        }
    }

    fn all_notes_off(&mut self) {
        for voice in &mut self.voices {
            voice.stage = Stage::Release;
            voice.released = true;
        }
        self.held = [None; 128];
    }

    fn reset(&mut self) {
        for (slot, voice) in self.voices.iter_mut().enumerate() {
            voice.on = false;
            if let Some(rings) = &self.rings {
                rings.stop(slot);
            }
        }
        self.held = [None; 128];
    }

    fn process(&mut self, left: &mut [f32], right: &mut [f32], _context: ProcessContext) {
        let Some(font) = &self.font else {
            return;
        };
        let frames = left.len();
        self.clock += frames as u64;
        // The CCs' smoothing, one block on: the voices read them between where this block starts and ends.
        self.cc_then = self.cc_now;
        let smoothing = (self.cc_now.iter_mut()).zip(&self.cc).zip(&font.cc_smooth);
        for ((now, &to), &seconds) in smoothing {
            *now = match seconds > 0.0 {
                true => {
                    *now + (to - *now)
                        * (1.0 - (-(frames as f32) / (seconds * self.sample_rate)).exp())
                }
                false => to,
            };
        }
        let transpose = 100.0 * ((48.0 * self.params[TRANSPOSE]).round() - 24.0);
        let fine = 200.0 * self.params[FINE_TUNE] - 100.0 + transpose;
        let [rate, depth, delay] = [VIBRATO, VIBRATO + 1, VIBRATO + 2].map(|p| self.params[p]);
        let cubic = self.params[QUALITY] > 0.5;

        let custom_vib = self.params[VIBRATO_MODE] > 0.5;
        let custom_filter = self.params[FILTER_MODE] > 0.5;
        let custom_hz = LFO_HZ * 2f32.powf((1500.0 + 12000.0 * self.params[CUTOFF]) / 1200.0);
        let custom_q = 40.0 * self.params[RESONANCE];
        
        let controls = Controls {
            then: &self.cc_then,
            now: &self.cc_now,
            bend: self.wheel,
            fine,
            vibrato: if custom_vib {
                [100.0 * depth, 10.0 * rate, 2.0 * delay]
            } else {
                [0.0; 3]
            },
            cubic,
            rate: self.sample_rate,
        };
        let wheel = WHEEL_CENTS * self.cc_now[1];
        for (slot, voice) in self.voices.iter_mut().enumerate().filter(|(_, v)| v.on) {
            let region = &self.regions[voice.region];
            if let Some(rules) = &region.sfz {
                let rings = self.rings.as_deref();
                render_sfz(
                    voice, slot, region, rules, font, rings, &controls, left, right,
                );
                continue;
            }
            let p = (voice.pan + voice.slide[1] + region.pan).clamp(0.0, 1.0);
            let vel = (voice.velocity + voice.slide[2]).clamp(0.0, 1.0);
            let gain = vel * vel * region.gain;
            let (gain_l, gain_r) = (gain * (2.0 - 2.0 * p).min(1.0), gain * (2.0 * p).min(1.0));
            // A control block at a time, which the vibratos move.
            let blocks = left.chunks_mut(CONTROL).zip(right.chunks_mut(CONTROL));

            let mod_filter =
                !custom_filter && (region.mod_lfo_fx[0] != 0.0 || region.mod_env.to_filter != 0.0);
            if custom_filter {
                voice.m.filter.tune(
                    (FilterShape::LowPass, 2),
                    custom_hz,
                    custom_q,
                    0.0,
                    self.sample_rate,
                );
            } else if let Some([hz, resonance]) = region.filter {
                if !mod_filter {
                    voice.m.filter.tune(
                        (FilterShape::LowPass, 2),
                        hz,
                        resonance,
                        0.0,
                        self.sample_rate,
                    );
                }
            } else {
                voice.m.filter.stages = 0;
            }

            let looping = region.looping && (!region.loop_sustain || !voice.released);

            'voice: for (left, right) in blocks {
                let dt = left.len() as f32 / self.sample_rate;
                let (vibrato, mod_val) =
                    voice.vibrato(region, wheel, controls.vibrato, custom_vib, dt);
                let env_val = voice.mod_env.level;
                voice.mod_env.advance(dt, voice.released);

                if mod_filter {
                    if let Some([base_hz, res]) = region.filter {
                        let fc_delta =
                            mod_val * region.mod_lfo_fx[0] + env_val * region.mod_env.to_filter;
                        let hz = base_hz * 2f32.powf(fc_delta / 1200.0);
                        voice.m.filter.tune(
                            (FilterShape::LowPass, 2),
                            hz,
                            res,
                            0.0,
                            self.sample_rate,
                        );
                    }
                }

                let lfo_gain = if region.mod_lfo_fx[1] != 0.0 {
                    10f32.powf((mod_val * region.mod_lfo_fx[1]).clamp(-96.0, 24.0) / 20.0)
                } else {
                    1.0
                };
                let prev_lfo = voice.m.gain.map_or(lfo_gain, |[g, _]| g);
                let gain_ramp = (lfo_gain - prev_lfo) / left.len() as f32;
                let mut mod_gain = prev_lfo;
                voice.m.gain = Some([lfo_gain, lfo_gain]);

                let env_pitch = env_val * region.mod_env.to_pitch;
                let semitones = self.bend + voice.slide[0] + (fine + vibrato + env_pitch) / 100.0;
                // `min` also turns a hostile file's infinite or NaN step into the cap.
                let step = (voice.step * 2f64.powf(f64::from(semitones) / 12.0)).min(MAX_STEP);
                for (l, r) in left.iter_mut().zip(right.iter_mut()) {
                    if !voice.envelope() {
                        break 'voice;
                    }
                    mod_gain += gain_ramp;
                    if voice.stage == Stage::Delay {
                        continue;
                    }
                    let at = |k| font.frame(k);
                    let sample = interpolate(region, looping, voice.position, cubic, at);
                    let sample = voice.m.filter.run_mono(sample);
                    *l += sample * voice.level * mod_gain * gain_l;
                    *r += sample * voice.level * mod_gain * gain_r;
                    voice.advance(region, looping, step);
                }
            }
        }
    }

    /// Enable off freezes the voices; they resume when it is back on.
    fn freezes_when_disabled(&self) -> bool {
        true
    }

    fn set_slide(&mut self, pitch: f32, pan: f32, velocity: f32) {
        SoundFontPlayer::set_slide(self, pitch, pan, velocity);
    }

    /// Channel param 4, Pitch Bend: ±2 semitones (an SFZ's `bend_up` and `bend_down`). 7 + n, MIDI CC n: an SFZ's
    /// controllers. Others do nothing here.
    fn set_controller(&mut self, index: usize, value: f32) {
        if !value.is_finite() {
            return;
        }
        let value = value.clamp(0.0, 1.0);
        if index == PITCH_BEND {
            self.wheel = 2.0 * value - 1.0;
            self.bend = BEND_RANGE * self.wheel;
        } else if let Some(cc) = index.checked_sub(7).and_then(|n| self.cc.get_mut(n)) {
            *cc = value;
        }
    }

    fn set_setting(&mut self, key: u32, value: f32) {
        if !value.is_finite() {
            return;
        }
        if key == ZONE_SELECT_KEY {
            self.zone = value.round().max(0.0) as usize;
        } else if key == ZONE_RESET_KEY {
            if let (Some(r), Some(base)) =
                (self.regions.get_mut(self.zone), self.base_regions.get(self.zone))
            {
                *r = base.clone();
                self.refresh_zone_voices(self.zone);
            }
        } else if let Some(field) = key.checked_sub(ZONE_CTL_BASE)
            && (field as usize) < ZONE_FIELDS
        {
            if let Some(r) = self.regions.get_mut(self.zone) {
                apply_zone_field(r, field as usize, value);
                self.refresh_zone_voices(self.zone);
            }
        }
    }
}
