//! The SoundFont Player (ID 52): the SoundFont 2 file of the public
//! SoundFont 2.01 standard, and its voices.
//!
//! [`SoundFont::parse`] runs on the app thread (bytes in, no I/O) and keeps the file: the 16-bit samples are read
//! from it in place. It survives hostile input: every size and index is checked, and what it allocates is bounded by
//! the file's length. [`SoundFontPlayer`] renders on the audio thread without allocating.
//!
//! An SFZ ([`crate::sfz`]) plays through the same player as a SoundFont of one patch ([`SoundFont::sfz`]): its regions
//! read decoded samples (mono or stereo, each at its own rate) instead of `smpl`, follow the SFZ's trigger, round-robin,
//! random, keyswitch, crossfade and choke rules, and the SFZ's DAHDSR envelope unless the module's Custom envelope is
//! on. A SoundFont's regions play exactly as before.

use std::collections::HashMap;
use std::ops::Range;
use std::sync::Arc;

use crate::ProcessContext;
use crate::instrument::Instrument;
use crate::sample::SampleData;
use crate::sfz::{LoopMode, SfzRegion, Trigger};

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
/// Voices per player; when all sound, the oldest-started is stolen.
pub const VOICES: usize = 32;

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
    pcm: Vec<Arc<SampleData>>,
    /// Its `amp_velcurve`s, by [`Rules::curve`].
    curves: Vec<[f32; 128]>,
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
            switches: None,
            switch: None,
        })
    }

    /// An SFZ as a SoundFont of one patch, `name` (its first 20 bytes): each region with the sample it plays, decoded
    /// (regions sharing a file share its `Arc`), at most [`MAX_REGIONS`]. A region that plays no frame (an `offset` at
    /// or past its `end`) or no key is left out. App thread.
    pub fn sfz(name: &str, regions: Vec<(SfzRegion, Arc<SampleData>)>) -> Self {
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
            switches: None,
            switch: None,
        };
        let mut index = HashMap::new();
        for (region, data) in regions.into_iter().take(MAX_REGIONS) {
            let pcm = &mut font.pcm;
            let sample = *index.entry(Arc::as_ptr(&data)).or_insert_with(|| {
                pcm.push(data.clone());
                pcm.len() - 1
            });
            let curve = (!region.amp_velcurve.is_empty()).then(|| {
                font.curves.push(velocity_curve(&region.amp_velcurve));
                font.curves.len() - 1
            });
            let Some(playable) = sfz_region(&region, &data, sample, curve) else {
                continue;
            };
            let key = |k: i32| k.clamp(0, 127) as u8;
            if region.sw_last.is_some() {
                let [lo, hi] = region.sw_keys.map(key);
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
    /// of the file), and an SFZ's decoded samples: what a cache counts against its memory budget.
    pub fn byte_len(&self) -> usize {
        let pcm = self
            .pcm
            .iter()
            .map(|s| 4 * (s.left.len() + s.right.as_ref().map_or(0, |r| r.len())));
        self.bytes.len() + pcm.sum::<usize>()
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
#[derive(Clone, Copy, Debug)]
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
#[derive(Clone, Copy, Debug)]
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
    sw_last: Option<u8>,
    xfin: [u8; 2],
    xfout: [u8; 2],
    /// `amp_veltrack`, −1…1.
    veltrack: f32,
    /// Its `amp_velcurve` in [`SoundFont::curves`].
    curve: Option<usize>,
    /// Delay, attack, hold, decay (seconds), sustain (level), release (seconds).
    ampeg: [f32; 6],
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
        let v = f32::from(velocity);
        let [lo, hi] = self.xfin.map(f32::from);
        let fade_in = match v {
            v if v < lo => 0.0,
            v if v >= hi => 1.0,
            v => ((v - lo) / (hi - lo)).sqrt(),
        };
        let [lo, hi] = self.xfout.map(f32::from);
        let fade_out = match v {
            v if v <= lo => 1.0,
            v if v > hi => 0.0,
            v => ((hi - v) / (hi - lo)).sqrt(),
        };
        fade_in * fade_out
    }
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

/// The [`Region`] of SFZ region `r` playing `data` (sample `sample`, curve `curve`), `None` when it plays nothing. A
/// loop end is the loop's last frame (the format's, and a WAV `smpl`'s), here exclusive; without `loop_mode` a file
/// with a loop loops.
fn sfz_region(
    r: &SfzRegion,
    data: &SampleData,
    sample: usize,
    curve: Option<usize>,
) -> Option<Region> {
    let keys = [r.keys[0].max(0), r.keys[1].min(127)];
    if keys[0] > keys[1] || r.end.is_some_and(|e| e <= 0) || data.sample_rate == 0 {
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
    let file_loop = (data.meta.loop_frames)
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
        false => f64::from(data.sample_rate),
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
            sw_last: r.sw_last.map(|k| k.clamp(0, 127) as u8),
            xfin: r.xfin,
            xfout: r.xfout,
            veltrack: r.amp_veltrack / 100.0,
            curve,
            ampeg: [delay, attack, hold, decay, sustain / 100.0, release],
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
/// The param defaults: Enable, Attack, Decay, Sustain, Release, Envelope (Original).
const DEFAULTS: [f32; 6] = [1.0, 0.0, 0.0, 1.0, 0.0, 0.0];
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
            Stage::Delay => self.stage = Stage::Attack,
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

/// The SoundFont Player: the regions of one patch of a [`SoundFont`], and [`VOICES`] voices.
pub struct SoundFontPlayer {
    font: Option<Arc<SoundFont>>,
    regions: Vec<Region>,
    voices: [Voice; VOICES],
    started: u64,
    sample_rate: f32,
    params: [f32; 6],
    /// Pitch bend in semitones.
    bend: f32,
    // An SFZ's state.
    /// The notes each region has met, for its round robin.
    seq: Vec<u32>,
    /// The keys held, with their note-on velocity and pan: for release triggers, `first` and `legato`.
    held: [Option<(f32, f32)>; 128],
    /// The keyswitch in force.
    switch: Option<u8>,
    /// The random layers' generator (xorshift32).
    random: u32,
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
        Self {
            switch: bank.as_ref().and_then(|(font, _)| font.switch),
            seq: vec![0; regions.len()],
            font: bank.map(|(font, _)| font),
            regions,
            voices: [Voice::default(); VOICES],
            started: 0,
            sample_rate,
            params: DEFAULTS,
            bend: 0.0,
            held: [None; 128],
            random: 0x9E37_79B9,
        }
    }

    /// Whether it plays an SFZ.
    fn is_sfz(&self) -> bool {
        self.font
            .as_ref()
            .is_some_and(|font| !font.sfz_regions.is_empty())
    }

    /// Starts the SFZ regions that a note-on of `key` at `velocity` (0…1) and `pan` plays, or with `release` its
    /// note-off (at the note-on's velocity). Of the regions it plays, past [`VOICES`] only the last ones start. Each
    /// stops the voices whose region's `off_by` names its group first.
    fn sfz_note(&mut self, key: u8, velocity: f32, pan: f32, release: bool) {
        let Some(font) = &self.font else {
            return;
        };
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
            *held = Some((velocity, pan));
        }
        self.random ^= self.random << 13;
        self.random ^= self.random >> 17;
        self.random ^= self.random << 5;
        let dice = (self.random >> 8) as f32 / (1 << 24) as f32;
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
            let plays = turn % rules.seq[0] == rules.seq[1]
                && (rules.rand[0] <= dice && dice < rules.rand[1])
                && rules.sw_last.is_none_or(|s| self.switch == Some(s));
            if plays {
                if count == VOICES {
                    chosen.copy_within(1.., 0);
                    count -= 1;
                }
                chosen[count] = index;
                count += 1;
            }
        }
        for &index in &chosen[..count] {
            let group = self.regions[index].sfz.map(|r| r.group);
            for voice in self.voices.iter_mut().filter(|v| v.on) {
                let rules = self.regions[voice.region].sfz;
                if let Some(rules) = rules.filter(|r| r.off_by.is_some() && r.off_by == group) {
                    voice.choke(rules.off_normal, self.sample_rate);
                }
            }
        }
        for &index in &chosen[..count] {
            self.sfz_start(index, key, velocity, pan, release);
        }
    }

    /// Starts a voice of SFZ region `index` (a release trigger's with `release`).
    fn sfz_start(&mut self, index: usize, key: u8, velocity: f32, pan: f32, release: bool) {
        let region = &self.regions[index];
        let Some(rules) = region.sfz else {
            return;
        };
        let rate = self.sample_rate;
        let cents = (f64::from(key) - f64::from(region.root)) * region.scale + rules.cents;
        let step = 2f64.powf(cents / 1200.0) * region.rate / f64::from(rate);
        let custom = self.params[5] != 0.0;
        let [delay, attack, hold, decay, sustain, release_time] = rules.ampeg;
        let frames = |seconds: f32| (seconds * rate) as u32;
        let envelope = match custom {
            true => self.envelope(region),
            false => [
                (1.0 / (attack * rate)).min(1.0),
                fall(decay, rate),
                sustain,
                fall(release_time.max(SFZ_MIN_RELEASE), rate),
            ],
        };
        let slot = (self.voices.iter().enumerate())
            .min_by_key(|(_, v)| (v.on, v.started))
            .map_or(0, |(slot, _)| slot);
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
            one_shot: release || rules.mode == LoopMode::OneShot,
            released: release,
            crossfade: rules.crossfade((127.0 * velocity).round() as u8),
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
    /// Params 1–4 (Attack, Decay, Sustain, Release) and 5 (Envelope).
    fn set_parameter(&mut self, index: usize, value: f32) {
        if let Some(param) = self.params.get_mut(index) {
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
            return self.sfz_note(key, velocity, pan, false);
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
            // A free voice, else the oldest-started.
            let slot = (self.voices.iter().enumerate())
                .min_by_key(|(_, v)| (v.on, v.started))
                .map_or(0, |(slot, _)| slot);
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
            && let Some((velocity, pan)) =
                self.held.get_mut(usize::from(key)).and_then(Option::take)
        {
            self.sfz_note(key, velocity, pan, true);
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
        for voice in &mut self.voices {
            voice.on = false;
        }
        self.held = [None; 128];
    }

    fn process(&mut self, left: &mut [f32], right: &mut [f32], _context: ProcessContext) {
        let Some(font) = &self.font else {
            return;
        };
        for voice in self.voices.iter_mut().filter(|v| v.on) {
            let region = &self.regions[voice.region];
            let bend = 2f64.powf(f64::from(self.bend + voice.slide[0]) / 12.0);
            // `min` also turns a hostile file's infinite or NaN step into the cap.
            let step = (voice.step * bend).min(MAX_STEP);
            let p = (voice.pan + voice.slide[1] + region.pan).clamp(0.0, 1.0);
            if let Some(rules) = &region.sfz {
                let sample = &font.pcm[rules.sample];
                let velocity = (voice.velocity + voice.slide[2]).clamp(0.0, 1.0);
                let gain =
                    rules.velocity_gain(&font.curves, velocity) * voice.crossfade * region.gain;
                let (gain_l, gain_r) = (gain * (2.0 - 2.0 * p).min(1.0), gain * (2.0 * p).min(1.0));
                let looping = voice.loops(region, rules);
                for (l, r) in left.iter_mut().zip(right.iter_mut()) {
                    if !voice.on || !voice.envelope() {
                        break;
                    }
                    let (a, b) = read_pcm(sample, region, looping, voice.position);
                    *l += a * voice.level * gain_l;
                    *r += b * voice.level * gain_r;
                    voice.advance_pcm(region, looping, step);
                }
                continue;
            }
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

    /// Channel param 4, Pitch Bend: ±2 semitones. Others do nothing here.
    fn set_controller(&mut self, index: usize, value: f32) {
        if index == PITCH_BEND && value.is_finite() {
            self.bend = BEND_RANGE * (2.0 * value.clamp(0.0, 1.0) - 1.0);
        }
    }
}
