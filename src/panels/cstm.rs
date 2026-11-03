//! Module settings ↔ the module's `CSTM` blob: one pure call each way, for the loader ([`settings_from_cstm`] into
//! [`PanelState::settings`](super::PanelState)) and the saver ([`cstm_from_settings`]).
//!
//! Each module's owned fields are a table below: the field, its byte offset, the panel control `#n` that edits
//! it, its encoding and the panel's default. Every other byte of the previous blob is kept (DirectWave `PATh`/`ZONE`
//! chunks, the Slicer/3x Osc/SoundFont paths, the Slicer AUTO flag, the Analyzer view state, the Trance Delay flag,
//! the DirectWave MIDI flag), and a field whose setting still reads as its previous bytes keeps those bytes, so an
//! untouched blob saves byte for byte. Instrument `#n` follow the instrument panels' convention: tab·10000 plus the
//! table's `n.m` in four digits (DW Sampler SAMPLE `1.8` is 11800, Slicer `3.1` is 3100).
//!
//! The rack's cards go through [`seed`] and [`store`].

use std::collections::HashMap;
use std::ops::Range;

use super::{PanelState, setting_item};
use crate::audio_engine::samples::dw_path;
use crate::flm::chunk::{ChunkIter, ChunkWriter, ContentPath, MAGIC_DW01};
use crate::params::menu_value;
use crate::sample_cache::Reader;
use crate::song::{ChannelKind, ModuleInstance, Song};
use honeydew_audio::dwp::{self, Cstm};
use honeydew_audio::slicer::SENSITIVITY_BEFORE_CSTM;

/// How a field's bytes hold its setting (the setting is the item index for lists and spinners, 0 or 1 for toggles
/// and buttons, a normalised value otherwise).
#[derive(Clone, Copy, Debug, PartialEq)]
pub(crate) enum Enc {
    /// `f32`, the setting itself.
    Norm,
    /// `f32` i/(n − 1), read back as round(v·(n − 1)), halves away from zero: item i of n.
    Items(usize),
    /// `f32` i/(n − 1), read back as int((n − 1)·v): item i of n.
    Trunc(usize),
    /// `u8` 0 or 1.
    Bit,
    /// `u32` item index, of n.
    Index(usize),
    /// `u8` semitones 1…12; the setting is k/12.
    Semitones,
}

/// One field a panel control owns.
#[derive(Clone, Copy, Debug)]
pub(crate) struct Field {
    /// The field's name.
    #[cfg_attr(
        not(test),
        expect(dead_code, reason = "the tables' and the tests' messages")
    )]
    pub name: &'static str,
    /// Byte offset in the blob, or in the chunk payload for a chunked blob.
    pub at: usize,
    /// The panel control `#n` whose setting it is.
    pub n: u32,
    pub enc: Enc,
    /// The panel's `Leaf::default` for `#n`.
    pub default: f32,
    /// Applied on load; `false` for a field only written (Pitcher 2 `Scale`, re-derived from the note params).
    pub load: bool,
}

const fn f(name: &'static str, at: usize, n: u32, enc: Enc, default: f32) -> Field {
    Field {
        name,
        at,
        n,
        enc,
        default,
        load: true,
    }
}

impl Field {
    /// The setting `bytes` (the blob or chunk payload) hold, clamped into range; `None` when they are too short.
    fn read(&self, bytes: &[u8]) -> Option<f32> {
        let raw = bytes.get(self.at..)?;
        let word = || <[u8; 4]>::try_from(raw.get(..4)?).ok();
        let float = || word().map(f32::from_le_bytes);
        Some(match self.enc {
            Norm => float().map(|v| {
                if v.is_nan() {
                    self.default
                } else {
                    v.clamp(0.0, 1.0)
                }
            })?,
            // round(v·(n − 1)), halves away from zero.
            Items(n) => {
                let v = float()?;
                let v = if v.is_nan() { 0.0 } else { v.clamp(0.0, 1.0) };
                ((v * (n - 1) as f32).round() as usize).min(n - 1) as f32
            }
            // `as` saturates: NaN and negatives are item 0.
            Trunc(n) => ((float()? * (n - 1) as f32) as usize).min(n - 1) as f32,
            Bit => f32::from(*raw.first()? != 0),
            Index(n) => (u32::from_le_bytes(word()?) as usize).min(n - 1) as f32,
            Semitones => f32::from((*raw.first()?).clamp(1, 12)) / 12.0,
        })
    }

    /// Writes setting `v` into `out`, clamped into range (NaN as the default); nothing where `out` is too short.
    fn write(&self, v: f32, out: &mut [u8]) {
        let v = if v.is_nan() { self.default } else { v };
        let item = setting_item(v);
        let bytes = match self.enc {
            Norm => v.clamp(0.0, 1.0).to_le_bytes().to_vec(),
            Items(n) | Trunc(n) => menu_value(item, n).to_le_bytes().to_vec(),
            Bit => vec![u8::from(item != 0)],
            Index(n) => (item.min(n - 1) as u32).to_le_bytes().to_vec(),
            Semitones => vec![(v * 12.0).round().clamp(1.0, 12.0) as u8],
        };
        if let Some(out) = out.get_mut(self.at..self.at + bytes.len()) {
            out.copy_from_slice(&bytes);
        }
    }
}

/// Writes each field of `settings` that no longer reads as `out` does; the others, and fields without a setting,
/// keep their bytes.
fn put(fields: &[Field], settings: &HashMap<u32, f32>, out: &mut [u8]) {
    let before = out.to_vec();
    for field in fields {
        if let Some(&v) = settings.get(&field.n)
            && field.read(&before) != Some(v)
        {
            field.write(v, out);
        }
    }
}

/// The payload range of the first chunk `id` in the chunk list `list` (through [`ChunkIter`]).
fn find(list: &[u8], id: [u8; 4]) -> Option<Range<usize>> {
    let chunk = ChunkIter::new(list, "CSTM")
        .map_while(Result::ok)
        .find(|c| c.id == id)?;
    let start = chunk.payload.as_ptr() as usize - list.as_ptr() as usize;
    Some(start..start + chunk.payload.len())
}

/// The payload a load ends up with: `missing`, overridden by the bytes the chunk holds (a short chunk sets only what
/// it has).
fn effective(chunk: Option<&[u8]>, missing: &[u8]) -> Vec<u8> {
    let mut out = chunk.unwrap_or_default().to_vec();
    out.extend(missing.get(out.len()..).unwrap_or_default());
    out
}

/// `head`, chunk `id` with `payload`, then `tail`.
fn with_chunk(head: &[u8], id: [u8; 4], payload: &[u8], tail: &[u8]) -> Option<Vec<u8>> {
    let mut w = ChunkWriter::default();
    w.raw(head).ok()?;
    w.chunk(id, |w| w.raw(payload)).ok()?;
    w.raw(tail).ok()?;
    Some(w.into_bytes())
}

/// `words` as little-endian bytes, `B` = 4·`W`.
const fn le_words<const W: usize, const B: usize>(words: [u32; W]) -> [u8; B] {
    let mut out = [0; B];
    let mut i = 0;
    while i < B {
        out[i] = (words[i / 4] >> (8 * (i % 4))) as u8;
        i += 1;
    }
    out
}

const fn bits(v: f32) -> u32 {
    v.to_bits()
}

use Enc::*;

/// DirectWave (1): the `MIDI` chunk's pitch-bend range, edited by the DW Sampler's `PB Range` knob (SAMPLE `1.8`).
/// Its flag byte is kept. Any blob loads as 4 and 1 before its `MIDI` overrides them.
pub(crate) const DIRECTWAVE: &[Field] = &[f("PB range", 0, 11800, Semitones, 2.0 / 12.0)];

/// GMSynth (2), 20 bytes: OSC `A`/`B` (`1.1`/`1.2`), MOD `PB RANGE` (`6.1`, round(3v) of `1 smt`, `2 smt`,
/// `7 smt`, `Oct`), `MW AMOUNT` (`7`) and `MW DEST` (`6.2`, int(2v) of `Filter`, `LFO`, `Osc. Mod.`).
pub(crate) const GMSYNTH: &[Field] = &[
    f("waveA", 0, 21100, Index(29), 0.0),
    f("waveB", 4, 21200, Index(29), 0.0),
    f("PB RANGE", 8, 36100, Items(4), 1.0),
    f("MW AMOUNT", 12, 37000, Norm, 0.5),
    f("MW DEST", 16, 36200, Trunc(3), 2.0),
];
const GMSYNTH_NEW: [u8; 20] = le_words([0, 0, bits(1.0 / 3.0), bits(0.5), bits(1.0)]);

/// Tuner (24), 16 bytes, all required: `Mode` (#1, 1.0 = Reactive), `Visual` (#2, int(2v)), `Reference` (the knob
/// #4 and the slider #5, one value), `Input` (#6, starts on).
pub(crate) const TUNER: &[Field] = &[
    f("Mode", 0, 1, Trunc(2), 0.0),
    f("Visual", 4, 2, Trunc(3), 0.0),
    f("Reference", 8, 4, Norm, 0.5),
    f("Reference", 8, 5, Norm, 0.5),
    f("Input", 12, 6, Items(2), 1.0),
];
const TUNER_NEW: [u8; 16] = le_words([0, 0, bits(0.5), bits(1.0)]);

/// Pitcher (25), 20 bytes, each part optional: the Basic page's note buttons C…B (#5–#16, settings, not params;
/// C minor), then `Key` (#21) and `Scale` (#20). The Advanced `BYP` buttons are params p10–p21, not in the blob.
pub(crate) const PITCHER: &[Field] = &[
    f("C", 0, 5, Bit, 1.0),
    f("C#", 1, 6, Bit, 0.0),
    f("D", 2, 7, Bit, 1.0),
    f("D#", 3, 8, Bit, 1.0),
    f("E", 4, 9, Bit, 0.0),
    f("F", 5, 10, Bit, 1.0),
    f("F#", 6, 11, Bit, 0.0),
    f("G", 7, 12, Bit, 1.0),
    f("G#", 8, 13, Bit, 1.0),
    f("A", 9, 14, Bit, 0.0),
    f("A#", 10, 15, Bit, 1.0),
    f("B", 11, 16, Bit, 0.0),
    f("Key", 12, 21, Items(12), 0.0),
    f("Scale", 16, 20, Items(5), 0.0),
];
/// A Pitcher blob under 12 bytes: no notes, and Key and Scale from offset 0.
const PITCHER_SHORT: &[Field] = &[
    f("Key", 0, 21, Items(12), 0.0),
    f("Scale", 4, 20, Items(5), 0.0),
];
const PITCHER_NEW: [u8; 20] = [1, 0, 1, 1, 0, 1, 0, 1, 1, 0, 1, 0, 0, 0, 0, 0, 0, 0, 0, 0];

/// Multiband Compressor (36) has no `CSTM`, but the band its Solo logic solos (UI state) reaches the engine as a
/// setting with this key, in the same `#n` numbering: the hidden `Solo` #38, which is bound to p4 and so never a
/// setting itself. Its value is 0 for none, k for band k (1 Low, 2 Mid, 3 High).
pub(crate) const MULTIBAND_SOLO_BAND: u32 = 38;

/// Analyzer (26), 128 bytes, all required: 11 slots, then each display's view state (kept as it was). Slot 0 is the
/// tab strip #19: its value is the page, which the panel keeps in [`PanelState::tab`](super::PanelState), not in
/// the settings, so the wiring copies it between the map's #19 and `tab` (a map without it keeps the saved page).
pub(crate) const ANALYZER: &[Field] = &[
    f("tab strip", 0, 19, Items(6), 0.0),
    f("Channel", 4, 14, Items(3), 2.0),
    f("Reactivity", 8, 15, Items(3), 1.0),
    f("DC Block", 12, 16, Items(2), 1.0),
    f("Input Gain", 16, 17, Norm, 0.5),
    f("SPECTRUM Mode", 20, 3, Items(2), 0.0),
    f("Scale", 24, 4, Items(2), 0.0),
    f("Threshold", 28, 6, Norm, 0.5),
    f("LOUDNESS Mode", 32, 8, Items(5), 0.0),
    f("Target", 36, 9, Norm, 1.0),
    f("Reference", 40, 12, Norm, 0.5),
];
/// A new Analyzer: the slots' new values, then the view state of Spectrum, Spectrogram, Oscilloscope (trigger
/// first), Loudness and Pitch track as x, w, y, h (the Vectorscope writes none).
const ANALYZER_NEW: [u8; 128] = le_words([
    bits(0.0),
    bits(1.0),
    bits(0.5),
    bits(1.0),
    bits(0.5),
    bits(0.0),
    bits(0.0),
    bits(0.5),
    bits(0.0),
    bits(1.0),
    bits(0.5),
    // Spectrum
    bits(0.0),
    bits(1.0),
    bits(0.294_117_7),
    bits(66.0 / 102.0),
    // Spectrogram
    bits(0.0),
    bits(1.0),
    bits(0.0),
    bits(1.0),
    // Oscilloscope
    bits(0.25),
    bits(0.0),
    bits(0.5),
    bits(0.166_666_69),
    bits(2.0 / 3.0),
    // Loudness
    bits(0.5),
    bits(0.5),
    bits(0.294_117_7),
    bits(66.0 / 102.0),
    // Pitch track
    bits(0.5),
    bits(0.5),
    bits(0.285_714_27),
    bits(0.5),
]);

/// Slicer (33): the `SENS` chunk's sensitivity, the `Sensitivity` knob (`3.1`). Its `AUTO` byte is kept (see
/// [`slicer_auto`]): the panel's `AUTO` (`3.7`) is a momentary button, not that flag.
pub(crate) const SLICER: &[Field] = &[f("sensitivity", 0, 3100, Norm, 0.25)];
/// The `SENS` payload of a new Slicer: sensitivity 0.25, `AUTO` 1.
const SENS_NEW: [u8; 5] = {
    let b = 0.25f32.to_le_bytes();
    [b[0], b[1], b[2], b[3], 1]
};

/// Pitcher 2 (45), 8 bytes, never an error: `Key` (#14) and `Scale` (#15, six items to `Custom`). Only Key loads:
/// The wiring re-derives Scale from the note params p1–p12 and Key.
pub(crate) const PITCHER_2: &[Field] = &[
    f("Key", 0, 14, Items(12), 0.0),
    Field {
        load: false,
        ..f("Scale", 4, 15, Items(6), 0.0)
    },
];

/// Where a module's fields live.
#[derive(Clone, Copy, Debug)]
enum Layout {
    /// A struct from offset 0. `new` is a new module's blob, whose length is the full size (a longer blob keeps its
    /// tail); `None` writes no new blob. A blob under `required` bytes reads as `None`; a field past its end keeps
    /// its default.
    Fixed {
        new: Option<&'static [u8]>,
        required: usize,
        fields: &'static [Field],
    },
    /// `magic`, then a chunk list whose chunk `id` holds the fields. `new` is its payload in a new module
    /// (`None`: no blob without a previous one); a load starts from `missing`, which the chunk's bytes override (and
    /// a chunk added later starts from it).
    Chunked {
        magic: &'static [u8],
        id: [u8; 4],
        new: Option<&'static [u8]>,
        missing: &'static [u8],
        fields: &'static [Field],
    },
}

/// The `CSTM` layout of module type `type_id`, `None` for the types that have none.
fn layout(type_id: u16) -> Option<Layout> {
    let fixed = |new: &'static [u8], required, fields| Layout::Fixed {
        new: Some(new),
        required,
        fields,
    };
    Some(match type_id {
        1 => Layout::Chunked {
            magic: &MAGIC_DW01,
            id: *b"MIDI",
            new: Some(&[2, 0]),
            missing: &[4, 1],
            fields: DIRECTWAVE,
        },
        // Only the two waves are required.
        2 => fixed(&GMSYNTH_NEW, 8, GMSYNTH),
        24 => fixed(&TUNER_NEW, 16, TUNER),
        25 => fixed(&PITCHER_NEW, 0, PITCHER),
        26 => fixed(&ANALYZER_NEW, 128, ANALYZER),
        // Written only with a loop, so never from nothing: the wiring makes the first blob with its `PATH`.
        33 => Layout::Chunked {
            magic: b"",
            id: *b"SENS",
            new: None,
            missing: &SENS_NEW,
            fields: SLICER,
        },
        45 => fixed(&[0; 8], 0, PITCHER_2),
        // Trance Delay's flag, the 3x Osc and SoundFont paths: nothing a panel setting owns.
        16 | 39 | 52 => Layout::Fixed {
            new: None,
            required: 0,
            fields: &[],
        },
        _ => return None,
    })
}

/// The settings module `type_id`'s `CSTM` blob holds, by panel `#n`. `None` for a type without a `CSTM` and for a
/// blob the format rejects (shorter than its required part, a DirectWave without its magic): the panel keeps its
/// defaults.
/// A field the blob is too short for reads as its default.
/// Not a validity gate for the loader: it reads only the settings and checks nothing else (DirectWave `ZONE` and
/// `PATh` lengths, for one).
pub fn settings_from_cstm(type_id: u16, blob: &[u8]) -> Option<HashMap<u32, f32>> {
    // A Pitcher under 12 bytes has no notes, and Key and Scale at offset 0.
    let short = type_id == 25 && blob.len() < 12;
    let (bytes, fields) = match layout(type_id)? {
        Layout::Fixed {
            required, fields, ..
        } => {
            if blob.len() < required {
                return None;
            }
            (blob.to_vec(), if short { PITCHER_SHORT } else { fields })
        }
        Layout::Chunked {
            magic,
            id,
            missing,
            fields,
            ..
        } => {
            let list = blob.strip_prefix(magic)?;
            (effective(find(list, id).map(|r| &list[r]), missing), fields)
        }
    };
    // Its missing notes keep their defaults too.
    let notes = if short { &PITCHER[..12] } else { &[] };
    let mut settings: HashMap<u32, f32> = fields
        .iter()
        .filter(|f| f.load)
        .map(|f| (f.n, f.read(&bytes).unwrap_or(f.default)))
        .chain(notes.iter().map(|f| (f.n, f.default)))
        .collect();
    // The Slicer's knob shows the Sensitivity the engine slices at, whole `SENS` or not.
    if type_id == 33 {
        settings.insert(SLICER[0].n, slicer_sensitivity(blob));
    }
    Some(settings)
}

/// The `CSTM` blob for module `type_id` with `settings` (by `#n`), keeping every byte of `previous` that no setting
/// owns. A setting missing from `settings` keeps its previous bytes, or its new-module value in a new blob. A
/// `previous` that [`settings_from_cstm`] rejects is not kept, and one too short to update in place (a partial
/// Pitcher or GMSynth) gives a whole new blob with what it held. `None` for a type without a `CSTM`, and for a Trance
/// Delay, Slicer, 3x Osc or SoundFont without a previous blob.
pub fn cstm_from_settings(
    type_id: u16,
    settings: &HashMap<u32, f32>,
    previous: Option<&[u8]>,
) -> Option<Vec<u8>> {
    let loaded = previous.and_then(|p| Some((p, settings_from_cstm(type_id, p)?)));
    match layout(type_id)? {
        Layout::Fixed { new, fields, .. } => {
            if let Some((p, _)) = loaded
                && p.len() >= new.map_or(0, <[u8]>::len)
            {
                let mut out = p.to_vec();
                put(fields, settings, &mut out);
                return Some(out);
            }
            let mut values = loaded.map(|(_, s)| s).unwrap_or_default();
            values.extend(settings);
            let mut out = new?.to_vec();
            put(fields, &values, &mut out);
            Some(out)
        }
        Layout::Chunked {
            magic,
            id,
            new,
            missing,
            fields,
        } => {
            let Some((p, _)) = loaded else {
                let mut payload = new?.to_vec();
                put(fields, settings, &mut payload);
                return with_chunk(magic, id, &payload, &[]);
            };
            let m = magic.len();
            let found = find(&p[m..], id).map(|r| m + r.start..m + r.end);
            let before = effective(found.clone().map(|r| &p[r]), missing);
            let mut payload = before.clone();
            put(fields, settings, &mut payload);
            match found {
                _ if payload == before => Some(p.to_vec()),
                Some(r) => with_chunk(&p[..r.start - 8], id, &payload, &p[r.end..]),
                // Added after the magic.
                None => with_chunk(&p[..m], id, &payload, &p[m..]),
            }
        }
    }
}

/// The Analyzer's type id and its tab strip, whose page is slot 0 of its blob.
const ANALYZER_ID: u16 = 26;
const ANALYZER_TAB: u32 = 19;

/// Gives a card's `state` the settings `module`'s `CSTM` holds (one without a blob: a new module's), as a card is
/// made and whenever the blob changed under it (an undo or redo that brings its channel back, a pick, a song opened).
/// The Analyzer's page is `state.tab`; Pitcher 2's `Scale` is derived from its notes and `Key`. A blob the format
/// rejects leaves `state` as it is.
pub fn seed(module: &ModuleInstance, state: &mut PanelState) {
    let blob = (module.file.custom_data.clone())
        .or_else(|| cstm_from_settings(module.type_id, &HashMap::new(), None));
    let Some(mut settings) = blob.and_then(|b| settings_from_cstm(module.type_id, &b)) else {
        return;
    };
    if module.type_id == ANALYZER_ID
        && let Some(page) = settings.remove(&ANALYZER_TAB)
    {
        state.tab = setting_item(page);
    }
    state.settings.extend(settings);
    if module.type_id == 45 {
        super::fx::derive_scale(module, state);
    }
}

/// Writes a card's `state` into `module`'s `CSTM` ([`cstm_from_settings`], so a Slicer without a blob gets none and
/// keeps its `AUTO`); returns whether the blob changed. A module without a blob gets none while its settings are a
/// new module's, which is how it loads.
pub fn store(module: &mut ModuleInstance, state: &PanelState) -> bool {
    let mut settings = state.settings.clone();
    if module.type_id == ANALYZER_ID {
        settings.insert(ANALYZER_TAB, state.tab as f32);
    }
    let previous = module.file.custom_data.as_deref();
    let Some(blob) = cstm_from_settings(module.type_id, &settings, previous) else {
        return false;
    };
    let unchanged = match previous {
        Some(previous) => blob == previous,
        None => Some(&blob) == cstm_from_settings(module.type_id, &HashMap::new(), None).as_ref(),
    };
    if !unchanged {
        module.file.custom_data = Some(blob);
    }
    !unchanged
}

/// Tuner: on a channel that is not an audio input the panel forces Input (#6) to 0, so such modules save 0.
/// A save applies it to every such Tuner, edited or not (the card never writes the hook's correction by itself).
pub fn tuner_inputs_off(song: &mut Song) {
    let channels = song.channels.iter_mut();
    for channel in channels.filter(|c| c.kind != ChannelKind::AudioRecord) {
        for module in channel.rack.modules.iter_mut().filter(|m| m.type_id == 24) {
            let off = HashMap::from([(6, 0.0)]);
            let previous = module.file.custom_data.as_deref();
            module.file.custom_data = cstm_from_settings(24, &off, previous);
        }
    }
}

/// The DW Sampler's module type id.
const DW: u16 = 1;

/// Loads a DW Sampler's `CSTM` into its Attack (p1) and Release (p2), after its PRMS. The `CSTM` first resets the
/// module (no zones, no path, so no init values of its own); its program (the `PATh` or `PATH` that counts,
/// [`Cstm::read`]) then loads through the program loader (`honeydew_audio::dwp::load_program`): a `.dwp` with zones
/// sets Attack and Release, init and current, from its last zone, and then the `PATh`'s two floats, when both are
/// there, set them. A program that is missing, or no `.dwp` (an audio file), leaves the saved PRMS: its floats stay the
/// init values a save writes. No other param changes. `read` reads the program file. Another module is left as it is.
pub fn load_dw_program(module: &mut ModuleInstance, read: &Reader<'_>) {
    if module.type_id != DW {
        return;
    }
    (module.file.program_init, module.file.program_reset) = (None, None);
    let blob = module.file.custom_data.as_deref().unwrap_or_default();
    let Some(program) = Cstm::read(blob).program else {
        return;
    };
    let (init, chunk, raw) = (program.init, program.chunk, program.path.to_vec());
    let bytes = dw_path(&raw).and_then(|path| read(&path));
    // Only whether it is a `.dwp` matters here: an audio file and a file no loader takes both keep the PRMS.
    let Some(zones) = bytes.as_deref().and_then(dwp::read) else {
        module.keep_program_init(init);
        return;
    };
    let last = zones.last().map(|zone| zone.zone.attack_release());
    (last.into_iter().chain(init)).for_each(|values| module.set_attack_release(values));
    // The init values are known now: a program chunk without its floats (a legacy `PATH`, an old `PATh`) becomes a
    // `PATh` with them, which a save then keeps up to date.
    if init.is_none() {
        let mut w = ChunkWriter::default();
        let blob = module.file.custom_data.as_deref().unwrap_or_default();
        let rewritten = (w.raw(&blob[..chunk.start]))
            .and_then(|()| {
                w.chunk(*b"PATh", |w| {
                    w.vstr32(&raw)?;
                    module.program_init().iter().try_for_each(|&v| w.f32(v))
                })
            })
            .and_then(|()| w.raw(&blob[chunk.end..]));
        if rewritten.is_ok() {
            module.file.custom_data = Some(w.into_bytes());
        }
    }
}

/// [`load_dw_program`] on every DW Sampler of `song`: what opening a song does once it is read.
pub fn load_dw_programs(song: &mut Song, read: &Reader<'_>) {
    let racks = song.channels.iter_mut().map(|c| &mut c.rack.modules);
    racks
        .flatten()
        .for_each(|module| load_dw_program(module, read));
}

/// The program loader took the file at `path` for a DW Sampler's pick: it is now the module's program, in place of its
/// zones and path, with its `MIDI` kept. `attack_release`: what a `.dwp` with zones sets, init and current; an audio
/// file or a `.dwp` without zones sets nothing. Returns `false`, changing nothing, for a path no `PATh` can hold.
pub fn pick_dw_program(
    module: &mut ModuleInstance,
    path: &ContentPath,
    attack_release: Option<(f32, f32)>,
) -> bool {
    let mut picked = module.clone();
    picked.file.program_reset = None;
    if let Some(values) = attack_release {
        picked.set_attack_release(values);
    }
    let mut w = ChunkWriter::default();
    let written = w.raw(&MAGIC_DW01).and_then(|()| {
        w.chunk(*b"PATh", |w| {
            w.content_path32(path)?;
            picked.program_init().iter().try_for_each(|&v| w.f32(v))
        })?;
        w.chunk(*b"MIDI", |w| w.raw(&dw_midi(module)))
    });
    if written.is_ok() {
        picked.file.custom_data = Some(w.into_bytes());
        *module = picked;
    }
    written.is_ok()
}

/// The program loader rejected a DW Sampler's pick: it has already cleared the module's zones and path, so only its
/// `MIDI` is left. Returns whether that changed anything; another module is left as it is.
pub fn clear_dw_program(module: &mut ModuleInstance) -> bool {
    if module.type_id != DW || module.file.custom_data.is_none() {
        return false;
    }
    let midi = dw_midi(module);
    let blob = [&MAGIC_DW01[..], b"MIDI", &2u32.to_le_bytes(), &midi].concat();
    (module.file.program_init, module.file.program_reset) = (None, None);
    let changed = module.file.custom_data.as_ref() != Some(&blob);
    module.file.custom_data = Some(blob);
    changed
}

/// A DW Sampler's `MIDI` chunk payload as it loads: pitch-bend range and the attack flag.
fn dw_midi(module: &ModuleInstance) -> [u8; 2] {
    let midi = Cstm::read(module.file.custom_data.as_deref().unwrap_or_default()).midi;
    [midi.pb_range, midi.attack_flag]
}

/// A Slicer's Sensitivity from its `CSTM` `blob`: its `SENS` f32 clamped, a non-finite one 0 (as every file f32 reads);
/// without a whole one, the engine's value before any `CSTM` is read (0.33). The engine slices at it and the panel's
/// knob shows it.
pub(crate) fn slicer_sensitivity(blob: &[u8]) -> f32 {
    let sens = find(blob, *b"SENS").and_then(|r| blob[r].first_chunk().copied());
    sens.map(f32::from_le_bytes)
        .map_or(SENSITIVITY_BEFORE_CSTM, |s| {
            if s.is_finite() {
                s.clamp(0.0, 1.0)
            } else {
                0.0
            }
        })
}

/// The Slicer's `SENS` `AUTO` flag (1 in a new module and in a blob without it). [`cstm_from_settings`] keeps it:
/// it is not a panel setting, since the panel's `AUTO` button is momentary: a press rewrites the current clip once and
/// leaves the flag as it is ([`crate::audio_engine::slicer::write_auto_clip`]).
pub fn slicer_auto(blob: &[u8]) -> bool {
    effective(find(blob, *b"SENS").map(|r| &blob[r]), &SENS_NEW)[4] != 0
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::panels::{PanelState, Widget, panel_for};

    /// The types with owned fields, and their tables.
    const OWNED: [(u16, &[Field]); 7] = [
        (1, DIRECTWAVE),
        (2, GMSYNTH),
        (24, TUNER),
        (25, PITCHER),
        (26, ANALYZER),
        (33, SLICER),
        (45, PITCHER_2),
    ];

    fn le(values: &[f32]) -> Vec<u8> {
        values.iter().flat_map(|v| v.to_le_bytes()).collect()
    }

    fn chunk(id: &[u8; 4], payload: &[u8]) -> Vec<u8> {
        [id, &(payload.len() as u32).to_le_bytes()[..], payload].concat()
    }

    fn map<const N: usize>(pairs: [(u32, f32); N]) -> HashMap<u32, f32> {
        HashMap::from(pairs)
    }

    /// The defaults of the loaded fields.
    fn defaults(fields: &[Field]) -> HashMap<u32, f32> {
        fields
            .iter()
            .filter(|f| f.load)
            .map(|f| (f.n, f.default))
            .collect()
    }

    /// A value of `field` other than its default.
    fn other(field: &Field) -> f32 {
        match field.enc {
            Norm => 0.8,
            Items(n) | Trunc(n) | Index(n) => (n - 1) as f32,
            Bit => 1.0 - field.default,
            Semitones => 7.0 / 12.0,
        }
    }

    /// The Analyzer's new-module view state, written out.
    fn analyzer_view() -> Vec<u8> {
        le(&[
            0.0,
            1.0,
            0.294_117_7,
            66.0 / 102.0, // Spectrum
            0.0,
            1.0,
            0.0,
            1.0, // Spectrogram
            0.25,
            0.0,
            0.5,
            0.166_666_69,
            2.0 / 3.0, // Oscilloscope: trigger, then x w y h
            0.5,
            0.5,
            0.294_117_7,
            66.0 / 102.0, // Loudness
            0.5,
            0.5,
            0.285_714_27,
            0.5, // Pitch track
        ])
    }

    /// An Analyzer blob with distinct values in every slot and a recognisable view state.
    fn analyzer_blob() -> Vec<u8> {
        let mut blob = le(&[0.6, 0.5, 1.0, 0.0, 0.7, 1.0, 0.0, 0.2, 0.75, 0.4, 0.9]);
        blob.extend((0..84).map(|i| i as u8 ^ 0xA5));
        blob
    }

    /// The blob a new module of `type_id` saves with its default settings.
    fn default_blob(type_id: u16) -> Vec<u8> {
        save(type_id, &HashMap::new())
    }

    /// A new module of `type_id` saving `settings`: from no blob, or for a Slicer from the one the wiring makes when
    /// a loop is loaded (its `PATH` and a new module's `SENS`, `project_files::load_pick`).
    fn save(type_id: u16, settings: &HashMap<u32, f32>) -> Vec<u8> {
        let start = (type_id == 33).then(|| {
            let sens = chunk(b"SENS", &SENS_NEW);
            [chunk(b"PATH", &[1, 0, 0, 0, b'x']), sens].concat()
        });
        cstm_from_settings(type_id, settings, start.as_deref()).unwrap()
    }

    #[test]
    fn defaults_round_trip_through_a_new_blob() {
        for (type_id, fields) in OWNED {
            let blob = default_blob(type_id);
            assert_eq!(
                settings_from_cstm(type_id, &blob),
                Some(defaults(fields)),
                "type {type_id}"
            );
        }
    }

    #[test]
    fn edited_settings_round_trip() {
        for (type_id, fields) in OWNED {
            let settings: HashMap<u32, f32> = fields.iter().map(|f| (f.n, other(f))).collect();
            let blob = save(type_id, &settings);
            let expected = fields
                .iter()
                .filter(|f| f.load)
                .map(|f| (f.n, other(f)))
                .collect();
            assert_eq!(
                settings_from_cstm(type_id, &blob),
                Some(expected),
                "type {type_id}"
            );
        }
    }

    /// Blobs written by hand from the layouts, with a distinct value in every field, land on the right controls: a
    /// table with two offsets swapped fails here.
    #[test]
    fn hand_built_blobs_read_into_the_right_controls() {
        let mut gm = 5u32.to_le_bytes().to_vec();
        gm.extend(17u32.to_le_bytes());
        gm.extend(le(&[2.0 / 3.0, 0.3, 0.5]));
        let mut pitcher = vec![0, 1, 0, 0, 1, 0, 1, 0, 0, 1, 0, 1];
        pitcher.extend(le(&[7.0 / 11.0, 0.75]));
        let cases = [
            (
                1,
                [&MAGIC_DW01[..], &chunk(b"MIDI", &[9, 0])].concat(),
                map([(11800, 9.0 / 12.0)]),
            ),
            (
                2,
                gm,
                map([
                    (21100, 5.0),
                    (21200, 17.0),
                    (36100, 2.0),
                    (37000, 0.3),
                    (36200, 1.0),
                ]),
            ),
            (
                24,
                le(&[1.0, 0.5, 0.3, 0.0]),
                map([(1, 1.0), (2, 1.0), (4, 0.3), (5, 0.3), (6, 0.0)]),
            ),
            (
                25,
                pitcher,
                map([
                    (5, 0.0),
                    (6, 1.0),
                    (7, 0.0),
                    (8, 0.0),
                    (9, 1.0),
                    (10, 0.0),
                    (11, 1.0),
                    (12, 0.0),
                    (13, 0.0),
                    (14, 1.0),
                    (15, 0.0),
                    (16, 1.0),
                    (21, 7.0),
                    (20, 3.0),
                ]),
            ),
            (
                26,
                analyzer_blob(),
                map([
                    (19, 3.0),
                    (14, 1.0),
                    (15, 2.0),
                    (16, 0.0),
                    (17, 0.7),
                    (3, 1.0),
                    (4, 0.0),
                    (6, 0.2),
                    (8, 3.0),
                    (9, 0.4),
                    (12, 0.9),
                ]),
            ),
            (
                33,
                chunk(b"SENS", &[0x9A, 0x99, 0x19, 0x3F, 0]),
                map([(3100, 0.6)]),
            ),
            (45, le(&[5.0 / 11.0, 0.2]), map([(14, 5.0)])),
        ];
        for (type_id, blob, expected) in cases {
            assert_eq!(
                settings_from_cstm(type_id, &blob),
                Some(expected),
                "type {type_id}"
            );
        }
    }

    #[test]
    fn new_blobs_hold_the_spec_new_module_values() {
        let dw = [&MAGIC_DW01[..], &chunk(b"MIDI", &[2, 0])].concat();
        assert_eq!(default_blob(1), dw);
        let mut gm = vec![0; 8];
        gm.extend(le(&[1.0 / 3.0, 0.5, 1.0]));
        assert_eq!(default_blob(2), gm);
        assert_eq!(default_blob(24), le(&[0.0, 0.0, 0.5, 1.0]));
        let mut pitcher = vec![1, 0, 1, 1, 0, 1, 0, 1, 1, 0, 1, 0];
        pitcher.extend(le(&[0.0, 0.0]));
        assert_eq!(default_blob(25), pitcher);
        let mut analyzer = le(&[0.0, 1.0, 0.5, 1.0, 0.5, 0.0, 0.0, 0.5, 0.0, 1.0, 0.5]);
        analyzer.extend(analyzer_view());
        assert_eq!(default_blob(26), analyzer);
        // A Slicer writes no blob of its own: only one with a loop, which the wiring makes.
        assert_eq!(cstm_from_settings(33, &map([(3100, 0.6)]), None), None);
        assert_eq!(default_blob(45), le(&[0.0, 0.0]));
    }

    #[test]
    fn gmsynth_pb_range_and_mw_dest_read_as_the_spec_says() {
        let read = |pb: f32, dest: f32| {
            let mut blob = vec![0; 8];
            blob.extend(le(&[pb, 0.5, dest]));
            let s = settings_from_cstm(2, &blob).unwrap();
            (s[&36100], s[&36200])
        };
        // PB RANGE round(3v) → `1 smt`, `2 smt`, `7 smt`, `Oct`; the default 1/3 is `2 smt`.
        for (v, item) in [
            (0.0, 0.0),
            (1.0 / 3.0, 1.0),
            (0.4, 1.0),
            (2.0 / 3.0, 2.0),
            (0.9, 3.0),
            (1.0, 3.0),
        ] {
            assert_eq!(read(v, 1.0).0, item, "PB {v}");
        }
        // MW DEST int(2v) → `Filter`, `LFO`, `Osc. Mod.`; the default 1.0 is `Osc. Mod.`.
        for (v, item) in [(0.0, 0.0), (0.3, 0.0), (0.5, 1.0), (0.99, 1.0), (1.0, 2.0)] {
            assert_eq!(read(1.0 / 3.0, v).1, item, "MW DEST {v}");
        }
    }

    #[test]
    fn tuner_clamps_and_reads_reactive_only_at_one() {
        let s = settings_from_cstm(24, &le(&[0.99, 0.49, -0.5, 1.0])).unwrap();
        assert_eq!(s, map([(1, 0.0), (2, 0.0), (4, 0.0), (5, 0.0), (6, 1.0)]));
        let s = settings_from_cstm(24, &le(&[1.5, 2.0, 1.5, f32::NAN])).unwrap();
        assert_eq!(s, map([(1, 1.0), (2, 2.0), (4, 1.0), (5, 1.0), (6, 0.0)]));
    }

    #[test]
    fn pitcher_short_blobs_load_the_parts_they_hold() {
        let minor = defaults(PITCHER);
        assert_eq!(settings_from_cstm(25, &[]), Some(minor.clone()));
        // Under 12 bytes Key and Scale are read from offset 0; a missing part keeps its default.
        let mut expected = minor.clone();
        expected.extend([(21, 3.0), (20, 2.0)]);
        assert_eq!(
            settings_from_cstm(25, &le(&[3.0 / 11.0, 0.5])),
            Some(expected)
        );
        let mut expected = minor.clone();
        expected.insert(21, 3.0);
        assert_eq!(settings_from_cstm(25, &le(&[3.0 / 11.0])), Some(expected));
        let mut notes = vec![1; 12];
        let all_on = settings_from_cstm(25, &notes).unwrap();
        assert!((5..17).all(|n| all_on[&n] == 1.0) && all_on[&21] == 0.0);
        notes.extend(le(&[3.0 / 11.0]));
        let s = settings_from_cstm(25, &notes).unwrap();
        assert_eq!((s[&5], s[&21], s[&20]), (1.0, 3.0, 0.0));
        // Saving over a short blob writes a whole one.
        let saved = cstm_from_settings(25, &map([(20, 4.0)]), Some(&le(&[3.0 / 11.0]))).unwrap();
        let mut expected = vec![1, 0, 1, 1, 0, 1, 0, 1, 1, 0, 1, 0];
        expected.extend(le(&[3.0 / 11.0, 1.0]));
        assert_eq!(saved, expected);
    }

    /// Each note byte lands on its own button.
    #[test]
    fn pitcher_note_bytes_read_one_hot() {
        for k in 0..12 {
            let mut blob = vec![0; 20];
            blob[k] = 1;
            let s = settings_from_cstm(25, &blob).unwrap();
            for j in 0..12 {
                assert_eq!(
                    s[&(5 + j as u32)],
                    f32::from(u8::from(j == k)),
                    "byte {k}, #{}",
                    5 + j
                );
            }
        }
    }

    /// Only the waves are required; a missing float keeps its default, and saving writes a whole blob.
    #[test]
    fn gmsynth_short_blobs_load_the_waves_and_default_the_rest() {
        let mut blob = 5u32.to_le_bytes().to_vec();
        blob.extend(17u32.to_le_bytes());
        let waves = map([(21100, 5.0), (21200, 17.0)]);
        let with = |extra: &[(u32, f32)]| {
            let mut m = waves.clone();
            m.extend([(36100, 1.0), (37000, 0.5), (36200, 2.0)]);
            m.extend(extra.iter().copied());
            Some(m)
        };
        assert_eq!(settings_from_cstm(2, &blob[..7]), None);
        assert_eq!(settings_from_cstm(2, &blob), with(&[]));
        blob.extend(le(&[0.0]));
        assert_eq!(settings_from_cstm(2, &blob), with(&[(36100, 0.0)]));
        blob.extend(le(&[0.3]));
        assert_eq!(
            settings_from_cstm(2, &blob),
            with(&[(36100, 0.0), (37000, 0.3)])
        );
        let saved = cstm_from_settings(2, &map([(36200, 0.0)]), Some(&blob)).unwrap();
        let mut expected = blob.clone();
        expected.extend(le(&[0.0]));
        assert_eq!(saved, expected);
    }

    /// Spinners load round(v·(n − 1)) with halves away from zero.
    #[test]
    fn spinner_ties_round_away_from_zero() {
        let mut pitcher = vec![0; 12];
        pitcher.extend(le(&[0.0, 0.625]));
        assert_eq!(settings_from_cstm(25, &pitcher).unwrap()[&20], 3.0);
        assert_eq!(
            settings_from_cstm(24, &le(&[0.0, 0.0, 0.5, 0.5])).unwrap()[&6],
            1.0
        );
    }

    #[test]
    fn directwave_loads_4_and_1_then_midi_overrides() {
        let zone = chunk(b"ZONE", &[7; 1065]);
        for (midi, pb) in [
            (None, 4),
            (Some(&[][..]), 4),
            (Some(&[9][..]), 9),
            (Some(&[9, 0][..]), 9),
        ] {
            let mut blob = [&MAGIC_DW01[..], &zone].concat();
            if let Some(midi) = midi {
                blob.extend(chunk(b"MIDI", midi));
            }
            assert_eq!(
                settings_from_cstm(1, &blob),
                Some(map([(11800, pb as f32 / 12.0)])),
                "{midi:?}"
            );
        }
        // A 1-byte MIDI loads flag 1; a change writes it out whole, flag 1 kept.
        let blob = [&MAGIC_DW01[..], &zone, &chunk(b"MIDI", &[9])].concat();
        let saved = cstm_from_settings(1, &map([(11800, 0.5)]), Some(&blob)).unwrap();
        assert_eq!(
            saved,
            [&MAGIC_DW01[..], &zone, &chunk(b"MIDI", &[6, 1])].concat()
        );
    }

    #[test]
    fn an_untouched_blob_saves_byte_for_byte() {
        let dw = [
            &MAGIC_DW01[..],
            &chunk(
                b"PATh",
                &[3, 0, 0, 0, b'@', b'R', b'@', 0, 0, 0, 0x80, 0x3F],
            ),
            &chunk(b"MIDI", &[0, 1]),
        ]
        .concat();
        let mut gm = 31u32.to_le_bytes().to_vec();
        gm.extend(7u32.to_le_bytes());
        gm.extend(le(&[0.4, 0.123, 0.3]));
        gm.extend([9, 9]);
        let mut pitcher = vec![2; 12];
        pitcher.extend(le(&[0.3, 0.6]));
        for (type_id, blob) in [
            (1, dw),
            (2, gm),
            (16, vec![1]),
            (16, Vec::new()),
            (24, le(&[0.7, 0.8, 0.123, 0.2])),
            (25, pitcher),
            (26, analyzer_blob()),
            (
                33,
                [
                    chunk(b"PATH", &[1, 0, 0, 0, b'x']),
                    chunk(b"SENS", &[0, 0, 0, 0x3F, 0]),
                ]
                .concat(),
            ),
            (39, chunk(b"PATH", &[0; 4])),
            (45, le(&[0.31, 0.77])),
            (52, chunk(b"SF2D", &[0; 8])),
        ] {
            let settings = settings_from_cstm(type_id, &blob).unwrap();
            assert_eq!(
                cstm_from_settings(type_id, &settings, Some(&blob)),
                Some(blob),
                "type {type_id}"
            );
        }
    }

    #[test]
    fn saving_keeps_the_bytes_no_setting_owns() {
        // DirectWave: PATh and ZONE stay, PB range changes, the MIDI flag stays.
        let zone = chunk(b"ZONE", &[7; 1065]);
        let path = chunk(b"PATh", &[0; 12]);
        let dw = [&MAGIC_DW01[..], &path, &zone, &chunk(b"MIDI", &[2, 1])].concat();
        let settings = map([(11800, 12.0 / 12.0)]);
        let expected = [&MAGIC_DW01[..], &path, &zone, &chunk(b"MIDI", &[12, 1])].concat();
        assert_eq!(cstm_from_settings(1, &settings, Some(&dw)), Some(expected));

        // DirectWave without MIDI: a change adds the chunk with flag 1, the zones stay.
        let bare = [&MAGIC_DW01[..], &zone].concat();
        let added = cstm_from_settings(1, &settings, Some(&bare)).unwrap();
        assert_eq!(
            added,
            [&MAGIC_DW01[..], &chunk(b"MIDI", &[12, 1]), &zone].concat()
        );

        // Slicer: PATH and AUTO stay.
        let path = chunk(b"PATH", &[3, 0, 0, 0, b'a', b'b', b'c']);
        let slicer = [chunk(b"SENS", &[0, 0, 0x80, 0x3E, 0]), path.clone()].concat();
        let sens = [&0.5f32.to_le_bytes()[..], &[0]].concat();
        assert_eq!(
            cstm_from_settings(33, &map([(3100, 0.5)]), Some(&slicer)),
            Some([chunk(b"SENS", &sens), path].concat())
        );

        // Analyzer: the view state and the untouched slots stay.
        let analyzer = analyzer_blob();
        let blob = cstm_from_settings(26, &map([(6, 0.9), (14, 0.0), (19, 5.0)]), Some(&analyzer))
            .unwrap();
        assert_eq!(blob[44..], analyzer[44..]);
        assert_eq!(blob[..12], le(&[1.0, 0.0, 1.0])[..]);
        assert_eq!(blob[12..28], analyzer[12..28]);
        assert_eq!(blob[28..32], 0.9f32.to_le_bytes());
        assert_eq!(blob[32..44], analyzer[32..44]);

        // A longer fixed blob keeps its tail; the opaque ones come back as they were.
        let mut gm = default_blob(2);
        gm.extend([1, 2, 3]);
        let blob = cstm_from_settings(2, &map([(21100, 5.0)]), Some(&gm)).unwrap();
        assert_eq!(blob[..4], 5u32.to_le_bytes());
        assert_eq!(blob[20..], [1, 2, 3]);
        for (type_id, blob) in [(16, vec![1]), (39, chunk(b"PATH", &[0; 9]))] {
            assert_eq!(
                cstm_from_settings(type_id, &HashMap::new(), Some(&blob)),
                Some(blob)
            );
        }
    }

    #[test]
    fn out_of_range_settings_save_clamped_and_nan_as_the_default() {
        let settings = map([(1, 7.0), (2, -3.0), (4, f32::NAN), (5, f32::NAN), (6, 0.4)]);
        let blob = cstm_from_settings(24, &settings, None).unwrap();
        assert_eq!(blob, le(&[1.0, 0.0, 0.5, 0.0]));
        let blob = cstm_from_settings(1, &map([(11800, 5.0)]), None).unwrap();
        assert_eq!(settings_from_cstm(1, &blob), Some(map([(11800, 1.0)])));
        let blob = cstm_from_settings(2, &map([(21100, 99.0), (37000, -1.0)]), None).unwrap();
        assert_eq!(
            (blob[..4].to_vec(), blob[12..16].to_vec()),
            (28u32.to_le_bytes().to_vec(), le(&[0.0]))
        );
    }

    #[test]
    fn tuner_reference_takes_whichever_control_changed() {
        let blob = default_blob(24);
        for (n, other) in [(4, 5), (5, 4)] {
            let settings = map([(n, 0.8), (other, 0.5)]);
            let saved = cstm_from_settings(24, &settings, Some(&blob)).unwrap();
            assert_eq!(settings_from_cstm(24, &saved).unwrap()[&5], 0.8, "#{n}");
        }
    }

    #[test]
    fn slicer_auto_is_kept_and_read_on_its_own() {
        assert!(slicer_auto(&[]));
        assert!(slicer_auto(&chunk(b"PATH", &[0; 4])));
        assert!(slicer_auto(&chunk(b"SENS", &[0; 2])));
        let off = chunk(b"SENS", &[0, 0, 0x80, 0x3E, 0]);
        assert!(!slicer_auto(&off));
        let saved = cstm_from_settings(33, &map([(3100, 0.9), (3700, 1.0)]), Some(&off)).unwrap();
        assert!(!slicer_auto(&saved));
    }

    #[test]
    fn pitcher_2_loads_the_key_only_and_never_fails() {
        assert_eq!(
            settings_from_cstm(45, &le(&[3.0 / 11.0, 1.0])),
            Some(map([(14, 3.0)]))
        );
        assert_eq!(
            settings_from_cstm(45, &le(&[3.0 / 11.0])),
            Some(map([(14, 3.0)]))
        );
        assert_eq!(settings_from_cstm(45, &[1, 2]), Some(map([(14, 0.0)])));
    }

    #[test]
    fn short_or_malformed_blobs_read_as_none() {
        let bad: [(u16, Vec<u8>); 5] = [
            (1, Vec::new()),
            (1, b"10WX".to_vec()),
            (2, vec![0; 7]),
            (24, vec![0; 15]),
            (26, vec![0; 127]),
        ];
        for (type_id, blob) in &bad {
            assert_eq!(
                settings_from_cstm(*type_id, blob),
                None,
                "type {type_id} {blob:?}"
            );
            // Saving over it writes a new blob.
            let saved = cstm_from_settings(*type_id, &HashMap::new(), Some(blob));
            assert_eq!(saved, Some(default_blob(*type_id)), "type {type_id}");
        }
        // Nothing panics on junk, and NaN or out-of-range values read as settings in range.
        for type_id in 0..60 {
            for blob in [
                vec![0xFF; 3],
                vec![0xFF; 200],
                b"10WDMIDI\xFF\xFF\xFF\xFF\x01".to_vec(),
            ] {
                if let Some(s) = settings_from_cstm(type_id, &blob) {
                    assert!(
                        s.values().all(|v| (0.0..=28.0).contains(v)),
                        "type {type_id} {s:?}"
                    );
                }
                let _ = cstm_from_settings(type_id, &HashMap::new(), Some(&blob));
            }
        }
    }

    #[test]
    fn types_without_a_cstm_have_none() {
        for type_id in [0, 3, 4, 19, 23, 27, 51, 99] {
            assert_eq!(settings_from_cstm(type_id, &[0; 20]), None);
            assert_eq!(cstm_from_settings(type_id, &HashMap::new(), None), None);
            assert_eq!(
                cstm_from_settings(type_id, &HashMap::new(), Some(&[0; 20])),
                None
            );
        }
        // An opaque one without a previous blob writes none.
        for type_id in [16, 33, 39, 52] {
            assert_eq!(cstm_from_settings(type_id, &HashMap::new(), None), None);
        }
    }

    // ---- The card wiring ----

    use crate::song::ModuleInstance;

    /// A module of `type_id` with `blob`, and its card's state seeded from it.
    fn seeded(type_id: u16, blob: Option<Vec<u8>>) -> (ModuleInstance, PanelState) {
        let mut module = ModuleInstance::new(type_id, 2);
        module.file.custom_data = blob;
        let mut state = PanelState::new(panel_for(type_id).unwrap());
        seed(&module, &mut state);
        (module, state)
    }

    #[test]
    fn a_card_is_seeded_from_its_cstm_and_stores_its_settings_back() {
        let blob = cstm_from_settings(24, &map([(2, 2.0), (5, 0.25)]), None).unwrap();
        let (mut module, mut state) = seeded(24, Some(blob.clone()));
        assert_eq!((state.settings[&2], state.settings[&5]), (2.0, 0.25));
        // Nothing changed: nothing to store.
        assert!(!store(&mut module, &state));
        assert_eq!(module.file.custom_data.as_ref(), Some(&blob));
        state.settings.insert(5, 0.75);
        assert!(store(&mut module, &state));
        let stored = module.file.custom_data.as_deref().unwrap();
        assert_eq!(settings_from_cstm(24, stored).unwrap()[&5], 0.75);
        assert_eq!(settings_from_cstm(24, stored).unwrap()[&2], 2.0);
        // Without a blob: the new module's settings; one is made at the first change.
        let (mut module, mut state) = seeded(24, None);
        assert_eq!(state.settings[&5], 0.5);
        state.settings.insert(1, 1.0);
        assert!(store(&mut module, &state));
        assert_eq!(
            settings_from_cstm(24, module.file.custom_data.as_deref().unwrap()).unwrap()[&1],
            1.0
        );
        // A blob the format rejects leaves the card as it is.
        let (_, state) = seeded(24, Some(vec![1; 3]));
        assert_eq!(state, PanelState::new(panel_for(24).unwrap()));
    }

    #[test]
    fn the_analyzers_page_is_slot_0() {
        let blob = cstm_from_settings(26, &map([(19, 3.0), (14, 0.0)]), None).unwrap();
        let (mut module, mut state) = seeded(26, Some(blob));
        assert_eq!(state.tab, 3);
        assert!(!state.settings.contains_key(&19), "the page is no setting");
        assert_eq!(state.settings[&14], 0.0);
        state.tab = 5;
        assert!(store(&mut module, &state));
        let stored = settings_from_cstm(26, module.file.custom_data.as_deref().unwrap()).unwrap();
        assert_eq!((stored[&19], stored[&14]), (5.0, 0.0));
    }

    #[test]
    fn pitcher_2_derives_its_scale_from_the_notes_and_key() {
        // D major on the notes, Key D, and `Minor` in the blob, which is not read.
        let mut module = ModuleInstance::new(45, 2);
        for (i, v) in module.params[1..13].iter_mut().enumerate() {
            *v = f32::from(u8::from([1, 2, 4, 6, 7, 9, 11].contains(&i)));
        }
        let blob = cstm_from_settings(45, &map([(14, 2.0), (15, 0.0)]), None).unwrap();
        module.file.custom_data = Some(blob);
        let mut state = PanelState::new(panel_for(45).unwrap());
        seed(&module, &mut state);
        assert_eq!((state.settings[&14], state.settings[&15]), (2.0, 1.0));
        // Notes that fit no scale: `Custom`.
        module.params[1] = 1.0;
        seed(&module, &mut state);
        assert_eq!(state.settings[&15], 5.0);
    }

    #[test]
    fn a_slicer_stores_only_into_a_blob_with_its_path_and_keeps_auto() {
        let (mut module, mut state) = seeded(33, None);
        assert_eq!(state.settings[&3100], 0.25);
        state.settings.insert(3100, 0.9);
        assert!(!store(&mut module, &state));
        assert_eq!(module.file.custom_data, None, "no CSTM without a PATH");
        let blob = [
            chunk(b"PATH", &[0; 4]),
            chunk(b"SENS", &[0, 0, 0x80, 0x3E, 0]),
        ]
        .concat();
        let (mut module, mut state) = seeded(33, Some(blob));
        state.settings.insert(3100, 0.9);
        assert!(store(&mut module, &state));
        let stored = module.file.custom_data.as_deref().unwrap();
        assert!(stored.starts_with(&chunk(b"PATH", &[0; 4])));
        assert!(!slicer_auto(stored), "AUTO is the previous blob's");
        assert_eq!(settings_from_cstm(33, stored).unwrap()[&3100], 0.9);
    }

    /// A module's unbound controls (and, for the Analyzer, its tab strip) start where its new blob reads.
    #[test]
    fn default_blobs_read_as_the_panels_defaults() {
        for (type_id, fields) in OWNED {
            let Some(panel) = panel_for(type_id) else {
                continue;
            };
            let state = PanelState::new(panel);
            let leaves = panel.root.leaves();
            let start = |n: u32| match leaves.iter().find(|l| l.id == n) {
                Some(l) if l.widget == Widget::Tabs => Some(state.tab as f32),
                Some(l) if l.param.is_none() => state.settings.get(&n).copied(),
                _ => None,
            };
            for f in fields {
                assert!(
                    start(f.n).is_some(),
                    "type {type_id} #{} {} is no unbound control",
                    f.n,
                    f.name
                );
            }
            for (n, v) in settings_from_cstm(type_id, &default_blob(type_id)).unwrap() {
                assert_eq!(start(n), Some(v), "type {type_id} #{n}");
            }
        }
    }

    // ---- DirectWave program load and pick ----

    use crate::flm::chunk::ContentRoot;
    use crate::song::ModuleFileState;

    fn dwp_record(tag: u32, data: &[u8]) -> Vec<u8> {
        [
            &tag.to_le_bytes()[..],
            &(data.len() as u64).to_le_bytes(),
            data,
        ]
        .concat()
    }

    /// `.dwp` whose zones have these 0x1FD floats.
    fn dwp(zones: &[[f32; 4]]) -> Vec<u8> {
        let zones: Vec<u8> = (zones.iter())
            .flat_map(|floats| {
                let floats = dwp_record(0x1FD, &floats.map(f32::to_le_bytes).concat());
                dwp_record(3, &[dwp_record(0x1F6, b"z.wav"), floats].concat())
            })
            .collect();
        [&b"DwPr"[..], &38u32.to_le_bytes(), &dwp_record(1, &zones)].concat()
    }

    fn path_chunk(id: &[u8; 4], path: &[u8], tail: &[u8]) -> Vec<u8> {
        chunk(
            id,
            &[&(path.len() as u32).to_le_bytes()[..], path, tail].concat(),
        )
    }

    fn pair(a: f32, r: f32) -> Vec<u8> {
        [a.to_le_bytes(), r.to_le_bytes()].concat()
    }

    const PROGRAM: &[u8] = b"@L@\0My Presets/DW Sampler/Keys.dwp";

    fn keys() -> ContentPath {
        ContentPath::Shorthand {
            root: ContentRoot::User,
            relative: "My Presets/DW Sampler/Keys.dwp".into(),
        }
    }

    /// A DW Sampler whose saved PRMS hold Attack 0.3 and Release 0.4 and other params off their defaults, with the
    /// `CSTM` chunks `chunks`.
    fn dw(chunks: &[Vec<u8>]) -> ModuleInstance {
        let mut module = ModuleInstance::new(1, 2);
        for (i, p) in module.params.iter_mut().enumerate() {
            *p = i as f32 / 40.0;
        }
        (module.params[1], module.params[2]) = (0.3, 0.4);
        module.file.custom_data = Some([b"10WD".to_vec(), chunks.concat()].concat());
        module
    }

    /// `module` after its `CSTM` loads with the file `file` (`None`: missing) at [`keys`].
    fn loaded(mut module: ModuleInstance, file: Option<Vec<u8>>) -> ModuleInstance {
        let before = module.clone();
        load_dw_program(&mut module, &|path: &ContentPath| {
            (*path == keys()).then(|| file.clone()).flatten()
        });
        // No other param is ever overridden, nor anything but the init values.
        for (i, (now, was)) in module.params.iter().zip(&before.params).enumerate() {
            assert!(i == 1 || i == 2 || now == was, "param {i}");
        }
        let file_state = |m: &ModuleInstance| ModuleFileState {
            program_init: None,
            program_reset: None,
            custom_data: None,
            ..m.file.clone()
        };
        assert_eq!(file_state(&module), file_state(&before));
        module
    }

    fn attack_release(module: &ModuleInstance) -> (f32, f32) {
        (module.params[1], module.params[2])
    }

    #[test]
    fn a_dw_program_sets_attack_and_release_from_its_last_zone_then_the_path_floats() {
        let program = dwp(&[[0.1, 1.0, 1.0, 0.2], [0.25, 0.0, 1.0, 0.6]]);
        // `.dwp` with zones: its last zone's #1 and #4, init and current.
        let bare = loaded(
            dw(&[path_chunk(b"PATh", PROGRAM, &[])]),
            Some(program.clone()),
        );
        assert_eq!(attack_release(&bare), (0.25, 0.6));
        assert_eq!(bare.file.program_init, None);
        // Then both floats, when both are there.
        let both = dw(&[path_chunk(b"PATh", PROGRAM, &pair(0.7, 0.8))]);
        let both = loaded(both, Some(program.clone()));
        assert_eq!(attack_release(&both), (0.7, 0.8));
        assert_eq!(both.file.program_init, None);
        for short in 1..8 {
            let cut = dw(&[path_chunk(b"PATh", PROGRAM, &pair(0.7, 0.8)[..short])]);
            assert_eq!(
                attack_release(&loaded(cut, Some(program.clone()))),
                (0.25, 0.6)
            );
        }
        // Legacy PATH: the same, without floats.
        let legacy = dw(&[path_chunk(b"PATH", PROGRAM, &pair(0.7, 0.8))]);
        assert_eq!(
            attack_release(&loaded(legacy, Some(program.clone()))),
            (0.25, 0.6)
        );
        // `.dwp` without zones sets nothing itself, but its floats apply.
        let empty = dwp(&[]);
        let unset = loaded(
            dw(&[path_chunk(b"PATh", PROGRAM, &[])]),
            Some(empty.clone()),
        );
        assert_eq!(attack_release(&unset), (0.3, 0.4));
        let set = dw(&[path_chunk(b"PATh", PROGRAM, &pair(0.7, 0.8))]);
        assert_eq!(attack_release(&loaded(set, Some(empty))), (0.7, 0.8));
        // Once loaded, a program chunk without its floats holds the init values, for a save to keep up to date.
        let midi = chunk(b"MIDI", &[3, 1]);
        for id in [b"PATh", b"PATH"] {
            let old = dw(&[path_chunk(id, PROGRAM, &[]), midi.clone()]);
            let module = loaded(old, Some(program.clone()));
            let blob = [
                b"10WD".to_vec(),
                path_chunk(b"PATh", PROGRAM, &pair(0.25, 0.6)),
                midi.clone(),
            ];
            assert_eq!(module.file.custom_data, Some(blob.concat()));
        }
        // Values past 0…1 are held to it.
        let wild = dwp(&[[5.0, 1.0, 1.0, -2.0]]);
        let held = loaded(dw(&[path_chunk(b"PATh", PROGRAM, &[])]), Some(wild));
        assert_eq!(attack_release(&held), (1.0, 0.0));
    }

    #[test]
    fn a_missing_or_audio_program_keeps_the_saved_prms_and_its_floats_as_init_values() {
        let path = dw(&[path_chunk(b"PATh", PROGRAM, &pair(0.7, 0.8))]);
        let wav = {
            let mut bytes = b"RIFF\x2c\0\0\0WAVEfmt \x10\0\0\0\x01\0\x01\0\x44\xac\0\0\x88\x58\x01\0\x02\0\x10\0".to_vec();
            bytes.extend(b"data\x08\0\0\0\x01\0\x02\0\x03\0\x04\0");
            bytes
        };
        for file in [None, Some(wav), Some(b"junk".to_vec())] {
            let module = loaded(path.clone(), file);
            assert_eq!(attack_release(&module), (0.3, 0.4));
            assert_eq!(module.file.program_init, Some([0.7, 0.8]));
            assert_eq!(module.program_init(), [0.7, 0.8]);
        }
        // Without floats (or with floats that are the PRMS) there are no init values of their own.
        let bare = loaded(dw(&[path_chunk(b"PATh", PROGRAM, &[])]), None);
        assert_eq!(
            (attack_release(&bare), bare.file.program_init),
            ((0.3, 0.4), None)
        );
        let same = loaded(dw(&[path_chunk(b"PATh", PROGRAM, &pair(0.3, 0.4))]), None);
        assert_eq!(same.file.program_init, None);
    }

    #[test]
    fn a_path_after_zones_loads_nothing_and_a_new_load_resets_the_init_values() {
        let mut zone = b"@L@\0z.wav".to_vec();
        zone.resize(1065, 0);
        let program = dwp(&[[0.25, 1.0, 1.0, 0.6]]);
        let zone_first = dw(&[
            chunk(b"ZONE", &zone),
            path_chunk(b"PATh", PROGRAM, &pair(0.7, 0.8)),
        ]);
        let module = loaded(zone_first, Some(program.clone()));
        assert_eq!(
            (attack_release(&module), module.file.program_init),
            ((0.3, 0.4), None)
        );
        // The first path counts: the second names no file.
        let other = path_chunk(b"PATh", b"@L@\0Other.dwp", &pair(0.9, 0.9));
        let two = dw(&[path_chunk(b"PATh", PROGRAM, &[]), other]);
        assert_eq!(attack_release(&loaded(two, Some(program))), (0.25, 0.6));
        // Without a program, init values kept from an earlier one go (the CSTM reset).
        let mut none = dw(&[chunk(b"MIDI", &[2, 0])]);
        none.file.program_init = Some([0.5, 0.5]);
        assert_eq!(loaded(none, None).file.program_init, None);
        let mut no_cstm = ModuleInstance::new(1, 2);
        no_cstm.file.program_init = Some([0.5, 0.5]);
        assert_eq!(loaded(no_cstm, None).file.program_init, None);
    }

    #[test]
    fn a_song_load_applies_every_dw_samplers_program() {
        let mut song = Song::default();
        song.add_channel(ChannelKind::Synth(crate::song::SynthKind::DwSampler));
        let blob = dw(&[path_chunk(b"PATh", PROGRAM, &[])]).file.custom_data;
        let rack = &mut song.channels[1].rack;
        rack.modules[0].file.custom_data = blob.clone();
        let other = rack.add(1);
        rack.get_mut(other).unwrap().file.custom_data = blob;
        let program = dwp(&[[0.25, 1.0, 1.0, 0.6]]);
        load_dw_programs(&mut song, &|path: &ContentPath| {
            (*path == keys()).then(|| program.clone())
        });
        for module in &song.channels[1].rack.modules {
            assert_eq!(attack_release(module), (0.25, 0.6));
        }
    }

    #[test]
    fn a_picked_program_replaces_the_zones_and_path_and_keeps_midi() {
        let mut zone = b"@L@\0z.wav".to_vec();
        zone.resize(1065, 0);
        let midi = chunk(b"MIDI", &[7, 0]);
        let module = dw(&[chunk(b"ZONE", &zone), midi.clone()]);
        let saved = |a: f32, r: f32| {
            Some(
                [
                    b"10WD".to_vec(),
                    path_chunk(b"PATh", PROGRAM, &pair(a, r)),
                    midi.clone(),
                ]
                .concat(),
            )
        };
        // `.dwp` with zones: Attack and Release, init and current.
        let mut picked = module.clone();
        assert!(pick_dw_program(&mut picked, &keys(), Some((0.25, 0.6))));
        assert_eq!(attack_release(&picked), (0.25, 0.6));
        assert_eq!(picked.file.custom_data, saved(0.25, 0.6));
        // An audio file, or a `.dwp` without zones: no params.
        let mut audio = module.clone();
        audio.file.program_init = Some([0.1, 0.2]);
        assert!(pick_dw_program(&mut audio, &keys(), None));
        assert_eq!(attack_release(&audio), (0.3, 0.4));
        assert_eq!(audio.file.program_init, Some([0.1, 0.2]));
        assert_eq!(audio.file.custom_data, saved(0.1, 0.2));
        // A new module (no CSTM) writes its MIDI 2 and 0.
        let mut new = ModuleInstance::new(1, 2);
        assert!(pick_dw_program(&mut new, &keys(), None));
        let midi_new = chunk(b"MIDI", &[2, 0]);
        let expected = [
            b"10WD".to_vec(),
            path_chunk(b"PATh", PROGRAM, &pair(0.0, 0.1)),
            midi_new,
        ]
        .concat();
        assert_eq!(new.file.custom_data, Some(expected));
        // A path no PATh can hold leaves the module as it was.
        let mut temp = module.clone();
        let nowhere = ContentPath::Shorthand {
            root: ContentRoot::Temp,
            relative: "x.dwp".into(),
        };
        assert!(!pick_dw_program(&mut temp, &nowhere, Some((0.25, 0.6))));
        assert_eq!(temp, module);
    }

    #[test]
    fn a_rejected_pick_clears_the_zones_and_path_and_keeps_midi() {
        let midi = chunk(b"MIDI", &[7, 0]);
        let mut module = dw(&[path_chunk(b"PATh", PROGRAM, &pair(0.7, 0.8)), midi.clone()]);
        module.file.program_init = Some([0.7, 0.8]);
        assert!(clear_dw_program(&mut module));
        assert_eq!(
            module.file.custom_data,
            Some([b"10WD".to_vec(), midi].concat())
        );
        assert_eq!(module.file.program_init, None);
        assert_eq!(attack_release(&module), (0.3, 0.4));
        assert!(!clear_dw_program(&mut module), "nothing left to clear");
        assert!(!clear_dw_program(&mut ModuleInstance::new(1, 2)));
    }

    fn resets(module: &ModuleInstance) -> [Option<f32>; 4] {
        [0, 1, 2, 3].map(|i| module.program_reset(i))
    }

    #[test]
    fn a_program_that_set_attack_and_release_is_what_they_reset_to() {
        // A program sets both their value and their default, and the reset value is the default.
        let program = dwp(&[[0.1, 1.0, 1.0, 0.2], [0.25, 0.0, 1.0, 0.6]]);
        let bare = loaded(
            dw(&[path_chunk(b"PATh", PROGRAM, &[])]),
            Some(program.clone()),
        );
        assert_eq!(resets(&bare), [None, Some(0.25), Some(0.6), None]);
        // The floats set them last.
        let both = dw(&[path_chunk(b"PATh", PROGRAM, &pair(0.7, 0.8))]);
        let both = loaded(both, Some(program.clone()));
        assert_eq!(resets(&both), [None, Some(0.7), Some(0.8), None]);
        // A move changes the value, not the reset.
        let mut moved = both.clone();
        moved.params[1] = 0.1;
        moved.init_follows(1);
        assert_eq!(resets(&moved), resets(&both));
        // Nothing set them: a program that is missing, an audio file, or a `.dwp` without zones or floats.
        for file in [None, Some(b"junk".to_vec())] {
            let kept = dw(&[path_chunk(b"PATh", PROGRAM, &pair(0.7, 0.8))]);
            assert_eq!(resets(&loaded(kept, file)), [None; 4]);
        }
        let bare = dw(&[path_chunk(b"PATh", PROGRAM, &[])]);
        assert_eq!(resets(&loaded(bare, Some(dwp(&[])))), [None; 4]);
        // The floats set them even where the `.dwp` has no zones.
        let floats = dw(&[path_chunk(b"PATh", PROGRAM, &pair(0.7, 0.8))]);
        let zoneless = loaded(floats, Some(dwp(&[])));
        assert_eq!(resets(&zoneless), [None, Some(0.7), Some(0.8), None]);
        // A pick sets them, or (an audio file) replaces them with none; a rejected pick clears them.
        let mut picked = both.clone();
        assert!(pick_dw_program(&mut picked, &keys(), Some((0.3, 0.4))));
        assert_eq!(resets(&picked), [None, Some(0.3), Some(0.4), None]);
        assert!(pick_dw_program(&mut picked, &keys(), None));
        assert_eq!(resets(&picked), [None; 4]);
        let mut rejected = both.clone();
        assert!(clear_dw_program(&mut rejected));
        assert_eq!(resets(&rejected), [None; 4]);
        // Opening a song again: the program loads from the CSTM's floats, and no module loads another's.
        assert_eq!(resets(&loaded(both.clone(), None)), [None; 4]);
        let mut distortion = ModuleInstance::new(6, 2);
        distortion.file.program_reset = both.file.program_reset;
        assert_eq!(resets(&distortion), [None; 4]);
    }
}
