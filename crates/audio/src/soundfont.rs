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
const START_LOOP: usize = 2;
const END_LOOP: usize = 3;
const PAN: usize = 17;
const ATTACK: usize = 34;
const DECAY: usize = 36;
const SUSTAIN: usize = 37;
const RELEASE: usize = 38;
const INSTRUMENT: u16 = 41;
const KEYS: usize = 43;
const VELS: usize = 44;
const START_LOOP_COARSE: usize = 45;
const ATTENUATION: usize = 48;
const END_LOOP_COARSE: usize = 50;
const COARSE: usize = 51;
const FINE: usize = 52;
const SAMPLE: u16 = 53;
const MODES: usize = 54;
const SCALE: usize = 56;
const ROOT: usize = 58;
/// Operators 0..=60 are defined; others are ignored.
const GENERATORS: usize = 61;

/// A zone's generator amounts by operator. Ranges (43, 44) hold their low and high bytes.
type Gens = [i16; GENERATORS];

const FULL_RANGE: i16 = i16::from_le_bytes([0, 127]);

/// The values a zone starts from: a preset's add nothing; an instrument's are the standard defaults (scaleTuning 100,
/// envelope times −12000 tc, sustain 0 cB, no overriding root key).
const fn defaults(instrument: bool) -> Gens {
    let mut gens = [0; GENERATORS];
    gens[KEYS] = FULL_RANGE;
    gens[VELS] = FULL_RANGE;
    if instrument {
        gens[ATTACK] = -12000;
        gens[DECAY] = -12000;
        gens[RELEASE] = -12000;
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
        let (start, end) = (header.start as usize, header.end as usize);
        let playable = start < end && end <= self.smpl.len() / 2 && header.rate > 0;
        if !playable || header.kind & ROM != 0 {
            return None;
        }
        let add = |op: usize| i32::from(preset[op]) + i32::from(gens[op]);
        let point = |at: u32, fine: usize, coarse: usize| {
            let offset = i64::from(gens[fine]) + 32_768 * i64::from(gens[coarse]);
            (i64::from(at) + offset).clamp(start as i64, end as i64) as usize
        };
        let loop_start = point(header.loop_start, START_LOOP, START_LOOP_COARSE);
        let loop_end = point(header.loop_end, END_LOOP, END_LOOP_COARSE);
        // overridingRootKey, else the sample's pitch; 128–255 is unpitched, which the standard plays at 60.
        let root = match (gens[ROOT], header.pitch) {
            (key @ 0..=127, _) => key as u8,
            (_, pitch @ 0..=127) => pitch,
            _ => 60,
        };
        let centibels = |op: usize| add(op).clamp(0, MAX_CB) as f32;
        Some(Region {
            keys: overlap(preset[KEYS], gens[KEYS])?,
            vels: overlap(preset[VELS], gens[VELS])?,
            end,
            loop_start,
            loop_end,
            looping: gens[MODES] & 1 == 1 && loop_end > loop_start,
            start,
            rate: f64::from(header.rate),
            root,
            tune: f64::from(add(COARSE))
                + f64::from(add(FINE) + i32::from(header.correction)) / 100.0,
            scale: f64::from(gens[SCALE]),
            pan: add(PAN).clamp(-500, 500) as f32 / 1000.0,
            gain: 10f32.powf(-centibels(ATTENUATION) / 200.0),
            attack: add(ATTACK) as f32,
            decay: add(DECAY) as f32,
            sustain: 10f32.powf(-centibels(SUSTAIN) / 200.0),
            release: add(RELEASE) as f32,
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

    /// `region`'s sample at `position` (in `start..end`), linearly interpolated; a loop's last frame leads into its
    /// first, and a sample's last frame is held.
    fn read(&self, region: &Region, position: f64) -> f32 {
        let i = position as usize;
        let next = if region.looping && i + 1 >= region.loop_end {
            region.loop_start
        } else {
            (i + 1).min(region.end - 1)
        };
        let (a, b) = (self.frame(i), self.frame(next));
        a + (b - a) * (position - i as f64) as f32
    }
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

/// A preset zone with one of its instrument's zones, resolved: what a voice plays.
#[derive(Clone, Debug)]
pub struct Region {
    keys: [u8; 2],
    vels: [u8; 2],
    /// Sample frames in `smpl`: the sample, and its loop.
    start: usize,
    end: usize,
    loop_start: usize,
    loop_end: usize,
    looping: bool,
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
        keys: [keys[0] as u8, keys[1] as u8],
        vels: r.vels,
        start,
        end,
        loop_start,
        loop_end,
        looping: matches!(mode, LoopMode::Continuous | LoopMode::Sustain) && loop_end > loop_start,
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

/// `region`'s decoded sample `data` at `position` (in `start..end`), each channel linearly interpolated, mono as both;
/// with `looping` the loop's last frame leads into its first.
fn read_pcm(data: &SampleData, region: &Region, looping: bool, position: f64) -> (f32, f32) {
    let i = position as usize;
    let next = if looping && i + 1 >= region.loop_end {
        region.loop_start
    } else {
        (i + 1).min(region.end - 1)
    };
    let t = (position - i as f64) as f32;
    let lerp = |x: &[f32]| {
        let (a, b) = (
            x.get(i).copied().unwrap_or(0.0),
            x.get(next).copied().unwrap_or(0.0),
        );
        a + (b - a) * t
    };
    let left = lerp(&data.left);
    (left, data.right.as_deref().map_or(left, lerp))
}

/// [`read_pcm`] for a streamed sample, voice `voice`'s of `rings`: `None` when a frame is not there yet.
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
/// The param defaults: Enable, Attack, Decay, Sustain, Release, Envelope (Original), Fine Tune (0 cents).
const DEFAULTS: [f32; 7] = [1.0, 0.0, 0.0, 1.0, 0.0, 0.0, 0.5];
/// Fine Tune, 200v − 100 cents on every voice: a Honeydew addition (#90).
const FINE_TUNE: usize = 6;
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
    m: Motion,
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
}

/// What an SFZ voice's control block reads besides its region: the CCs (0…1) at the process block's start (`then`)
/// and end (`now`), the pitch bend (−1…1), Fine Tune (cents) and the rate.
struct Controls<'a> {
    then: &'a [f32; 128],
    now: &'a [f32; 128],
    bend: f32,
    fine: f32,
    rate: f32,
}

impl Voice {
    /// One frame of the envelope; false once the voice has ended.
    fn envelope(&mut self) -> bool {
        if self.dahdsr {
            return self.dahdsr();
        }
        match self.stage {
            Stage::Attack => {
                self.level += (ATTACK_TARGET - self.level) * self.attack;
                if self.level >= 1.0 {
                    self.level = 1.0;
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
            Stage::Delay | Stage::Hold => {}
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
        let cents = pitch + self.m.pitch + bend + ctl.fine + 100.0 * self.slide[0];
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

    /// Moves `step` frames on through `region`'s sample: round its loop, or up to its last frame, where it fades.
    fn advance(&mut self, region: &Region, step: f64) {
        self.position += step;
        if region.looping {
            if self.position >= region.loop_end as f64 {
                let start = region.loop_start as f64;
                let length = (region.loop_end - region.loop_start) as f64;
                self.position = start + (self.position - start) % length;
            }
        } else if self.position >= (region.end - 1) as f64 {
            self.position = (region.end - 1) as f64;
            self.stage = Stage::Release;
            self.release = END_FADE;
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
                (Pcm::Ram(data), _) => read_pcm(data, region, looping, voice.position),
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
        let custom = self.params[5] != 0.0;
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

    /// The envelope of a voice of `region` starting now: attack, decay and release coefficients and the sustain
    /// level. Envelope (p5) at any value ≠ 0 is Custom (the sliders), else Original (the SoundFont's): the panel reads
    /// `Custom` only above 0.5, but 0 < v ≤ 0.5 plays Custom too.
    fn envelope(&self, region: &Region) -> [f32; 4] {
        let rate = self.sample_rate;
        let [attack, decay, sustain, release] = if self.params[5] != 0.0 {
            let time = |v: f32| (10.0 * 0.95f32.mul_add(v, 0.05).powi(3)).max(MIN_TIME);
            let x = (127.0 * self.params[3]).round().clamp(1.0, 127.0) / 127.0;
            [
                1.25f32.ln() / (rate * time(self.params[1])),
                100f32.ln() / (rate * time(self.params[2])),
                10f32.powf(5.0 * x.powf(0.2) - 5.0),
                100f32.ln() / (rate * time(self.params[4])),
            ]
        } else {
            let c = |tc: f32| ORIGINAL_RATE / (rate * 2f32.powf(tc / 1200.0).max(MIN_TIME));
            [
                c(region.attack),
                c(region.decay),
                region.sustain,
                c(region.release),
            ]
        };
        [attack.min(1.0), decay.min(1.0), sustain, release.min(1.0)]
    }
}

impl Instrument for SoundFontPlayer {
    /// Params 1–4 (Attack, Decay, Sustain, Release), 5 (Envelope) and 6 (Fine Tune).
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
        let steps = (127.0 * velocity) as u8;
        let matches = self.regions.iter().filter(|r| r.plays(key, steps));
        let skip = matches.count().saturating_sub(VOICES);
        let regions = self.regions.iter().enumerate();
        for (index, region) in regions.filter(|(_, r)| r.plays(key, steps)).skip(skip) {
            let [attack, decay, sustain, release] = self.envelope(region);
            let octaves =
                (f64::from(key) - f64::from(region.root) + region.tune) * region.scale / 1200.0;
            let step = 2f64.powf(octaves) * region.rate / f64::from(self.sample_rate);
            let slot = slot(&self.voices, None, None);
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
                attack,
                decay,
                sustain,
                release,
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
        let fine = 200.0 * self.params[FINE_TUNE] - 100.0;
        let controls = Controls {
            then: &self.cc_then,
            now: &self.cc_now,
            bend: self.wheel,
            fine,
            rate: self.sample_rate,
        };
        for (slot, voice) in self.voices.iter_mut().enumerate().filter(|(_, v)| v.on) {
            let region = &self.regions[voice.region];
            if let Some(rules) = &region.sfz {
                let rings = self.rings.as_deref();
                render_sfz(
                    voice, slot, region, rules, font, rings, &controls, left, right,
                );
                continue;
            }
            let bend = 2f64.powf(f64::from(self.bend + voice.slide[0] + fine / 100.0) / 12.0);
            // `min` also turns a hostile file's infinite or NaN step into the cap.
            let step = (voice.step * bend).min(MAX_STEP);
            let p = (voice.pan + voice.slide[1] + region.pan).clamp(0.0, 1.0);
            let gain = (voice.velocity + voice.slide[2]).clamp(0.0, 1.0) * region.gain;
            let (gain_l, gain_r) = (gain * (2.0 - 2.0 * p).min(1.0), gain * (2.0 * p).min(1.0));
            for (l, r) in left.iter_mut().zip(right.iter_mut()) {
                if !voice.envelope() {
                    break;
                }
                let sample = font.read(region, voice.position) * voice.level;
                *l += sample * gain_l;
                *r += sample * gain_r;
                voice.advance(region, step);
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
}
