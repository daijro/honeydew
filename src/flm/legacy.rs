//! The legacy preset files a module's picker also loads: MiniSynth's `.flms` and GMSynth's `.flgsynth`. Each holds the
//! params after `Enable`, so a preset loads as `PRMS = [1.0] ++ its floats`. 3x Osc's `.fst` has its own importer,
//! `crate::project_files::import_fst`.

use super::chunk::{ChunkIter, Cursor, FlmError, FlmErrorKind, MAX_FILE_SIZE};
use crate::params::GMSYNTH_WAVESHAPES;
use crate::song::ModuleInstance;

/// `.flms`'s 8-byte magic.
const FLMS_MAGIC: [u8; 8] = *b"\x99FLS\r\n\x1a\n";
/// `.flgsynth`'s magic, `Chn2` written as a LE u32.
const FLGSYNTH_MAGIC: [u8; 4] = *b"2nhC";
const MINISYNTH: u16 = 0;
const GMSYNTH: u16 = 2;

/// The module type a legacy preset is for, by its magic; `None` for any other file.
pub(super) fn legacy_type(bytes: &[u8]) -> Option<u16> {
    if bytes.starts_with(&FLMS_MAGIC) {
        Some(MINISYNTH)
    } else if bytes.starts_with(&FLGSYNTH_MAGIC) {
        Some(GMSYNTH)
    } else {
        None
    }
}

/// Reads `bytes` as the legacy preset its magic names; `None`: it is no legacy preset.
pub(super) fn read(bytes: &[u8]) -> Option<Result<ModuleInstance, FlmError>> {
    Some(match legacy_type(bytes)? {
        MINISYNTH => read_flms(bytes),
        _ => read_flgsynth(bytes),
    })
}

/// Reads a MiniSynth preset, `.flms`: the magic, then a chunk list holding `HEAD` (its own name, unused: the file's
/// stem names the preset) and `DATA`, `0rev` + f32[43], MiniSynth's params 1..=43. A short `DATA` leaves the rest at
/// The module defaults, extra values are ignored, and every value is kept in 0..=1. A chunk walk that finds no `DATA`
/// (a `HEAD` whose size runs over it) reads the floats after the first `0rev`. Fails without the magic, without a
/// `DATA` or a `0rev`, with another `DATA` version, and over the size limit.
pub fn read_flms(bytes: &[u8]) -> Result<ModuleInstance, FlmError> {
    const PATH: &str = "FLMS";
    let body = sized(bytes, PATH)?
        .strip_prefix(&FLMS_MAGIC)
        .ok_or_else(|| FlmError::new(PATH, FlmErrorKind::InvalidMagic))?;
    let mut chunks = ChunkIter::new(body, PATH).map_while(Result::ok);
    let values = match chunks.find(|c| c.id == *b"DATA") {
        Some(data) => {
            let mut values = data.reader();
            values.expect_magic(*b"0rev")?;
            values
        }
        None => {
            let at = (body.windows(4).position(|w| w == b"0rev"))
                .ok_or_else(|| FlmError::new(PATH, FlmErrorKind::MissingChunk))?;
            Cursor::new(&body[at + 4..], PATH)
        }
    };
    Ok(module(MINISYNTH, values, None))
}

/// Reads a GMSynth preset, `.flgsynth`: the magic, waveshape A's and B's paths (`char[512]` each,
/// `Waveshapes\<Name>.spectra`), then f32[74], GMSynth's params 1..=74. The waveshapes become the module's `CSTM`
/// (`waveA`, `waveB`; the other `CSTM` fields keep their defaults). A file cut in the floats leaves the rest at the
/// module defaults; values are kept in 0..=1. Fails without the magic or both paths, and over the size limit.
pub fn read_flgsynth(bytes: &[u8]) -> Result<ModuleInstance, FlmError> {
    const PATH: &str = "FLGSYNTH";
    let mut reader = Cursor::new(sized(bytes, PATH)?, PATH);
    reader.expect_magic(FLGSYNTH_MAGIC)?;
    let (a, b) = (reader.char::<512>()?, reader.char::<512>()?);
    let waves = [waveshape(&a), waveshape(&b)];
    let blob = waves.iter().flat_map(|w| w.to_le_bytes()).collect();
    Ok(module(GMSYNTH, reader, Some(blob)))
}

fn sized<'a>(bytes: &'a [u8], path: &str) -> Result<&'a [u8], FlmError> {
    match bytes.len() > MAX_FILE_SIZE {
        true => Err(FlmError::new(path, FlmErrorKind::TooLarge)),
        false => Ok(bytes),
    }
}

/// A module of `type_id`, on, its next params read from `values` while they last (each kept in 0..=1), with `custom`
/// as its `CSTM`.
fn module(type_id: u16, mut values: Cursor, custom: Option<Vec<u8>>) -> ModuleInstance {
    let mut module = ModuleInstance::new(type_id, 0);
    module.params[0] = 1.0;
    for (param, value) in module.params[1..]
        .iter_mut()
        .zip(std::iter::from_fn(|| values.f32().ok()))
    {
        *param = value.clamp(0.0, 1.0);
    }
    module.file.custom_data = custom;
    module
}

/// The waveshape menu index of a `.flgsynth` path: its file name without the extension, matched without case,
/// `Devils Horn` being `Double Spike`'s file. Anything else is 0, `Beast`, the default.
fn waveshape(path: &str) -> u32 {
    let name = path.rsplit(['\\', '/']).next().unwrap_or_default();
    let name = name.rsplit_once('.').map_or(name, |(stem, _)| stem);
    let name = match name.eq_ignore_ascii_case("Devils Horn") {
        true => "Double Spike",
        false => name,
    };
    let index = GMSYNTH_WAVESHAPES
        .iter()
        .position(|w| w.eq_ignore_ascii_case(name));
    index.unwrap_or(0) as u32
}

#[cfg(test)]
mod tests {
    use super::*;

    /// `.flms`: the magic, `HEAD` (version 1, 1, the name in UTF-16LE) and `DATA` (`0rev` + `values`).
    fn flms(name: &str, values: &[f32]) -> Vec<u8> {
        let utf16: Vec<u8> = name.encode_utf16().flat_map(u16::to_le_bytes).collect();
        let mut head = [1u16, 1, name.encode_utf16().count() as u16]
            .iter()
            .flat_map(|v| v.to_le_bytes())
            .collect::<Vec<u8>>();
        head.extend(utf16);
        let mut data = b"0rev".to_vec();
        data.extend(values.iter().flat_map(|v| v.to_le_bytes()));
        let mut bytes = FLMS_MAGIC.to_vec();
        for (id, payload) in [(b"HEAD", head), (b"DATA", data)] {
            bytes.extend(id);
            bytes.extend((payload.len() as u32).to_le_bytes());
            bytes.extend(payload);
        }
        bytes
    }

    /// `.flgsynth` of 1324 bytes: the magic, the two waveshape paths, `values`.
    fn flgsynth(a: &str, b: &str, values: &[f32]) -> Vec<u8> {
        let mut bytes = FLGSYNTH_MAGIC.to_vec();
        for path in [a, b] {
            let mut field = [0u8; 512];
            field[..path.len()].copy_from_slice(path.as_bytes());
            bytes.extend(field);
        }
        bytes.extend(values.iter().flat_map(|v| v.to_le_bytes()));
        bytes
    }

    fn waves(module: &ModuleInstance) -> [u32; 2] {
        let blob = module.file.custom_data.as_deref().expect("a CSTM");
        assert_eq!(blob.len(), 8);
        [0, 4].map(|at| u32::from_le_bytes(blob[at..at + 4].try_into().unwrap()))
    }

    #[test]
    fn a_minisynth_preset_is_enable_then_its_43_floats() {
        let values: Vec<f32> = (0..43).map(|i| i as f32 / 43.0).collect();
        let preset = read_flms(&flms("Fat Bass", &values)).unwrap();
        assert_eq!((preset.type_id, preset.instance_id), (0, 0));
        assert_eq!(preset.params.len(), 44);
        assert_eq!(preset.params[0], 1.0);
        assert_eq!(preset.params[1..], values[..]);
        assert_eq!(preset.file.custom_data, None);
        // A short DATA keeps the defaults of the rest; extra values are ignored; values stay in 0..=1.
        let defaults = ModuleInstance::new(0, 0);
        let short = read_flms(&flms("", &[0.25, 7.0, -1.0, f32::NAN])).unwrap();
        assert_eq!(short.params[1..5], [0.25, 1.0, 0.0, 0.0]);
        assert_eq!(short.params[5..], defaults.params[5..]);
        let long = read_flms(&flms("", &[0.5; 50])).unwrap();
        assert_eq!(long.params.len(), 44);
        // HEAD is optional; unknown chunks are skipped.
        let mut bare = FLMS_MAGIC.to_vec();
        bare.extend(b"JUNK\x01\0\0\0x");
        bare.extend(b"DATA\x08\0\0\x000rev");
        bare.extend(0.75f32.to_le_bytes());
        assert_eq!(read_flms(&bare).unwrap().params[1], 0.75);
    }

    /// `HEAD` whose size runs over `DATA` hides it from the chunk walk: the floats after `0rev` still read.
    #[test]
    fn a_minisynth_preset_whose_data_the_chunk_walk_misses_reads_after_its_version() {
        let mut bytes = flms("Fat", &[0.25, 0.75]);
        let size = FLMS_MAGIC.len() + 4;
        bytes[size..size + 4].copy_from_slice(&1000u32.to_le_bytes());
        let preset = read_flms(&bytes).unwrap();
        assert_eq!(preset.params[1..3], [0.25, 0.75]);
        // Without `0rev` anywhere it is still refused.
        let at = bytes.windows(4).position(|w| w == b"0rev").unwrap();
        bytes[at] = b'1';
        assert!(read_flms(&bytes).is_err());
    }

    #[test]
    fn a_minisynth_preset_without_its_magic_data_or_version_is_refused() {
        let good = flms("A", &[0.5]);
        assert!(read_flms(&good[1..]).is_err(), "magic");
        assert!(read_flms(&FLMS_MAGIC).is_err(), "no DATA");
        assert!(
            read_flms(&good[..good.len() - 8]).is_err(),
            "DATA cut before 0rev"
        );
        let mut version = good.clone();
        let at = version.windows(4).position(|w| w == b"0rev").unwrap();
        version[at] = b'1';
        assert!(read_flms(&version).is_err(), "another version");
        assert!(read_flms(b"").is_err());
    }

    #[test]
    fn a_gmsynth_preset_is_enable_then_its_74_floats_and_its_two_waveshapes() {
        let values: Vec<f32> = (0..74).map(|i| i as f32 / 74.0).collect();
        let bytes = flgsynth(
            r"Waveshapes\Rhodes.spectra",
            "Waveshapes/square.SPECTRA",
            &values,
        );
        assert_eq!(bytes.len(), 1324);
        let preset = read_flgsynth(&bytes).unwrap();
        assert_eq!((preset.type_id, preset.instance_id), (2, 0));
        assert_eq!(preset.params.len(), 75);
        assert_eq!(preset.params[0], 1.0);
        assert_eq!(preset.params[1..], values[..]);
        // Menu order: Rhodes 13, Square 22, any case.
        assert_eq!(waves(&preset), [13, 22]);
        // `Devils Horn` is Double Spike's file (2); an unknown or empty path is Beast (0), the default.
        let odd = read_flgsynth(&flgsynth(r"Waveshapes\Devils Horn.spectra", "", &[])).unwrap();
        assert_eq!(waves(&odd), [2, 0]);
        let odd = read_flgsynth(&flgsynth("Nope.spectra", "Waveshapes\\Vocal", &[0.5])).unwrap();
        assert_eq!(waves(&odd), [0, 28]);
        // A short tail keeps the defaults.
        let defaults = ModuleInstance::new(2, 0);
        assert_eq!(odd.params[1], 0.5);
        assert_eq!(odd.params[2..], defaults.params[2..]);
    }

    #[test]
    fn a_gmsynth_preset_without_its_magic_or_paths_is_refused() {
        let good = flgsynth("", "", &[0.5; 74]);
        assert!(read_flgsynth(&good[1..]).is_err(), "magic");
        assert!(
            read_flgsynth(&good[..1027]).is_err(),
            "cut in the second path"
        );
        assert!(
            read_flgsynth(&good[..1028]).is_ok(),
            "both paths, no floats"
        );
        assert!(read_flgsynth(b"").is_err());
    }

    #[test]
    fn the_magic_names_the_module_type() {
        assert_eq!(legacy_type(&flms("A", &[])), Some(MINISYNTH));
        assert_eq!(legacy_type(&flgsynth("", "", &[])), Some(GMSYNTH));
        assert_eq!(legacy_type(b"HEAD\x09\0\0\0"), None);
        assert_eq!(legacy_type(b""), None);
    }

    #[test]
    fn a_legacy_preset_reads_and_fits_as_a_module_preset_of_its_own_type_only() {
        use crate::flm::{preset_fits, read_preset};
        let (mini, gm) = (flms("A", &[0.25]), flgsynth("", "", &[0.75]));
        assert!(preset_fits(&mini, MINISYNTH) && !preset_fits(&mini, GMSYNTH));
        assert!(preset_fits(&gm, GMSYNTH) && !preset_fits(&gm, MINISYNTH));
        assert_eq!(read_preset(&mini, Some(MINISYNTH)).unwrap().params[1], 0.25);
        assert_eq!(read_preset(&gm, None).unwrap().params[1], 0.75);
        assert!(read_preset(&mini, Some(GMSYNTH)).is_err());
        assert!(read_preset(&gm, Some(39)).is_err());
        // A broken one fits by its magic, but does not read.
        assert!(read_preset(&FLMS_MAGIC, Some(MINISYNTH)).is_err());
    }

    /// A small xorshift, so the loop needs no dependency.
    struct Rng(u64);

    impl Rng {
        fn next(&mut self) -> u64 {
            self.0 ^= self.0 << 13;
            self.0 ^= self.0 >> 7;
            self.0 ^= self.0 << 17;
            self.0
        }

        fn below(&mut self, n: usize) -> usize {
            (self.next() % n.max(1) as u64) as usize
        }
    }

    /// Mutated and cut valid files and random bytes: never a panic, and whatever reads has the module's param count
    /// with every value in 0..=1.
    #[test]
    fn hostile_legacy_presets_are_refused_never_a_panic() {
        let bases = [
            flms("Fuzz", &[0.5; 43]),
            flgsynth(
                r"Waveshapes\Saw.spectra",
                r"Waveshapes\Sine.spectra",
                &[0.5; 74],
            ),
        ];
        let mut rng = Rng(0x9e37_79b9_7f4a_7c15);
        let (mut tried, mut read) = (0, 0);
        for i in 0..4000 {
            let mut bytes = match i % 3 {
                2 => (0..rng.below(64)).map(|_| rng.next() as u8).collect(),
                k => bases[k].clone(),
            };
            for _ in 0..1 + rng.below(8) {
                let at = rng.below(bytes.len());
                match rng.below(4) {
                    0 if !bytes.is_empty() => bytes[at] = rng.next() as u8,
                    1 => bytes.truncate(at),
                    2 => bytes.insert(at.min(bytes.len()), rng.next() as u8),
                    _ => {
                        let word = (rng.next() as u32).to_le_bytes();
                        let end = (at + 4).min(bytes.len());
                        bytes[at.min(end)..end].copy_from_slice(&word[..end - at.min(end)]);
                    }
                }
            }
            for result in [read_flms(&bytes), read_flgsynth(&bytes)] {
                tried += 1;
                if let Ok(module) = result {
                    read += 1;
                    let n = ModuleInstance::new(module.type_id, 0).params.len();
                    assert_eq!(module.params.len(), n, "#{i}");
                    assert!(
                        module.params.iter().all(|v| (0.0..=1.0).contains(v)),
                        "#{i}"
                    );
                }
            }
        }
        assert!(read > 0 && read < tried, "{read} of {tried}");
    }
}
