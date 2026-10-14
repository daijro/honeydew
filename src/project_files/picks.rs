//! What the pickers list from the Original Content folder ([`super::content`]) and how a pick loads. Everything is
//! read-only.
//!
//! - A module's preset picker lists its factory folders at its root, after `My Files` and `Default`
//!   ([`ProjectFiles::factory_presets`]): `Synth Presets2/<instrument>` and `Effect Presets/<effect>` by
//!   [`super::preset_folder`]; the DW Sampler's `DirectWave Samples` and `DirectWave Samples EXP`; the Slicer's
//!   `Loop Samples`; 3x Osc's `Synth Presets2/3xOsc`, then its `3xOsc Shapes`; the SoundFont Player's `SoundFonts`. The
//!   category is the folder.
//! - The file types a module takes ([`super::preset_exts`]): its presets (`.flmpst`, MiniSynth's legacy `.flms`,
//!   GMSynth's `.flgsynth` and 3x Osc's FL desktop `.fst`); a DW program `.dwp` or an audio file for the DW Sampler; an
//!   audio loop for the Slicer; an audio shape for 3x Osc; a `.sf2` for the SoundFont Player.
//! - A pick ([`load_pick`]): a preset loads whole ([`crate::flm::read_preset`], the legacy ones too; a `.fst` through
//!   [`super::import_fst`], its custom waveform in the module's `CSTM` `PATH`); a program, loop or shape sets the
//!   module's `CSTM` path chunk to the file's shorthand (DirectWave `PATh`, Slicer and 3x Osc `PATH`), as the module
//!   loader does, and changes no param; a SoundFont sets the SoundFont Player's `SF2D` to the file at patch 0.
//! - GMSynth's waveshapes: the `.spectra` files of `GMSynthWaveshapes`, slot i the i-th by file name,
//!   replace our designed spectra at start ([`ProjectFiles::install_original_waveshapes`]).

use std::f32::consts::PI;
use std::path::{Path, PathBuf};

use super::{FST_EXT, ProjectFiles, SAMPLE_EXTS, SF2_EXT, SOUNDFONTS, import_fst};
use crate::flm::chunk::{ChunkIter, ChunkWriter, ContentPath, FlmError};
use crate::flm::sf2d::Sf2d;
use crate::params::Role;
use crate::song::ModuleInstance;
use honeydew_audio::gm_waveshapes::{PARTIALS, SLOTS, Spectrum};

/// A DW program.
pub const DWP_EXT: &str = "dwp";
/// A GMSynth waveshape.
pub const SPECTRA_EXT: &str = "spectra";
/// The waveshapes' folder in the content tree.
pub const WAVESHAPES_FOLDER: &str = "GMSynthWaveshapes";

/// The modules whose loader takes a file it names in its `CSTM`.
pub(crate) const DW_SAMPLER: u16 = 1;
pub(crate) const SLICER: u16 = 33;
pub(crate) const THREE_OSC: u16 = 39;
/// The SoundFont Player, whose loader names the file in its `SF2D`.
pub(crate) const SOUNDFONT_PLAYER: u16 = 52;

/// The file `name` is no preset but a file module `type_id`'s loader names in its `CSTM`: a DW program or audio file
/// for the DW Sampler, an audio file for the Slicer or 3x Osc, a `.sf2` for the SoundFont Player.
fn named_file(name: &Path, type_id: u16) -> bool {
    let audio = is_audio(name);
    match type_id {
        DW_SAMPLER => audio || is(name, DWP_EXT),
        SLICER | THREE_OSC => audio,
        SOUNDFONT_PLAYER => is(name, SF2_EXT),
        _ => false,
    }
}

impl ProjectFiles {
    /// The content folders the preset picker of module `type_id` lists at its root (those that are there): see the
    /// module docs.
    pub fn factory_presets(&self, type_id: u16) -> Vec<PathBuf> {
        let Some(spec) = crate::params::module_spec(type_id) else {
            return Vec::new();
        };
        let folders: Vec<String> = match type_id {
            DW_SAMPLER => vec![DW_SAMPLES.into(), format!("{DW_SAMPLES} EXP")],
            SLICER => vec!["Loop Samples".into()],
            THREE_OSC => vec!["Synth Presets2/3xOsc".into(), "3xOsc Shapes".into()],
            // Its factory presets are the content folder's SoundFonts.
            SOUNDFONT_PLAYER => vec![SOUNDFONTS.into()],
            _ => {
                let parent = match spec.role {
                    Role::Instrument => "Synth Presets2",
                    _ => "Effect Presets",
                };
                let folder = super::preset_folder(type_id).unwrap_or(spec.display_name);
                vec![format!("{parent}/{folder}")]
            }
        };
        (folders.iter())
            .filter_map(|f| self.content_folder(f))
            .collect()
    }

    /// GMSynth's waveshapes from the content folder: the `.spectra` files of [`WAVESHAPES_FOLDER`] by file name, the
    /// i-th for menu slot i, each with its slot; a file that does not read is left out, and
    /// files past the menu's last slot too.
    pub fn original_waveshapes(&self) -> Vec<(usize, Spectrum)> {
        let Some(dir) = self.content_folder(WAVESHAPES_FOLDER) else {
            return Vec::new();
        };
        let library = self.preset_library(&[SPECTRA_EXT]);
        let mut names: Vec<_> = (library.list(Some(&dir)).unwrap_or_default())
            .into_iter()
            .filter(|e| !e.is_folder)
            .collect();
        names.sort_by(|a, b| a.name.cmp(&b.name));
        (names.iter().take(SLOTS).enumerate())
            .filter_map(|(slot, e)| Some((slot, read_spectra(&library.read(&e.path).ok()?)?)))
            .collect()
    }

    /// Hands [`Self::original_waveshapes`] to GMSynth in place of its designed shapes, once, before any GMSynth is
    /// built: the app calls it at start. Changes to the folder show after a restart.
    pub fn install_original_waveshapes(&self) {
        for (slot, spectrum) in self.original_waveshapes() {
            honeydew_audio::gmsynth::install_shape(slot, spectrum);
        }
    }
}

/// The DW Sampler's factory folder.
const DW_SAMPLES: &str = "DirectWave Samples";

/// `name` is an audio file: one of [`SAMPLE_EXTS`], in any case.
pub fn is_audio(name: &Path) -> bool {
    SAMPLE_EXTS.iter().any(|x| is(name, x))
}

/// `name`'s extension is `ext`, in any case.
fn is(name: &Path, ext: &str) -> bool {
    name.extension()
        .is_some_and(|e| e.eq_ignore_ascii_case(ext))
}

/// Whether the file `name` lists in the picker of module `type_id` (`None`: a rack or drum kit picker, any file of
/// its types). A preset is read (`read`) for the module it names ([`crate::flm::preset_fits`]); a file the module's
/// loader takes ([`named_file`]) is not, nor a `.fst`, which names no module (only 3x Osc lists them).
pub fn fits(name: &Path, type_id: Option<u16>, read: impl FnOnce() -> Option<Vec<u8>>) -> bool {
    match type_id {
        Some(t) if !named_file(name, t) && !is(name, FST_EXT) => {
            read().is_some_and(|b| crate::flm::preset_fits(&b, t))
        }
        _ => true,
    }
}

/// Loads the picked file `path` (its bytes `bytes`, its shorthand `shorthand`) into `module`: a preset, or the path
/// chunk of its `CSTM`. An error leaves `module` as it was. `.fst` always loads: a failed import
/// leaves the module at its defaults, then reads the bytes as native chunks ([`super::fst::native_state`]); its
/// custom waveform stays unless the file names one that resolves.
pub fn load_pick(
    module: &mut ModuleInstance,
    path: &Path,
    bytes: &[u8],
    shorthand: Option<ContentPath>,
) -> Result<(), String> {
    let t = module.type_id;
    if named_file(path, t) {
        let path = shorthand.ok_or("The file is outside the user and content folders.")?;
        let blob = module.file.custom_data.as_deref();
        module.file.custom_data = Some(path_cstm(t, blob, &path).map_err(|e| e.to_string())?);
        return Ok(());
    }
    if t == THREE_OSC && is(path, FST_EXT) {
        let import = import_fst(bytes, path);
        let mut preset = match import.ok {
            true => ModuleInstance {
                params: import.params,
                ..ModuleInstance::new(t, 0)
            },
            false => super::fst::native_state(bytes).unwrap_or_else(|| ModuleInstance::new(t, 0)),
        };
        // The custom waveform takes the shape slot (the engine plays it once it reads the slot, see `generators.rs`);
        // without one that resolves the module's stays.
        let kept = (preset.file.custom_data.take()).or_else(|| module.file.custom_data.clone());
        preset.file.custom_data = match import.custom_shape.filter(|_| import.ok) {
            Some(shape) => Some(path_cstm(t, kept.as_deref(), &shape).map_err(|e| e.to_string())?),
            None => kept,
        };
        (preset.file.preset_name, preset.file.category) = (import.name, import.category);
        module.load_preset(preset);
        return Ok(());
    }
    let preset = crate::flm::read_preset(bytes, Some(t)).map_err(|e| e.to_string())?;
    module.load_preset(preset);
    Ok(())
}

/// The `CSTM` of module `type_id` once its loader took the file `path`: DirectWave's `10WD` with `PATh` and no
/// zones or older path, its `MIDI` kept (a new module's 2, 0 otherwise); the Slicer's `PATH`, its `SENS` kept (a new
/// module's 0.25 with AUTO on otherwise); 3x Osc's `PATH`. The other chunks of `previous` stay.
fn path_cstm(
    type_id: u16,
    previous: Option<&[u8]>,
    path: &ContentPath,
) -> Result<Vec<u8>, FlmError> {
    // The SoundFont Player's file is its `SF2D`, at patch 0.
    if type_id == SOUNDFONT_PLAYER {
        let sf2d = Sf2d {
            path: path.clone(),
            patch: 0,
        };
        return sf2d.write(previous);
    }
    let dw = type_id == DW_SAMPLER;
    let (magic, id): (&[u8], _) = if dw {
        (b"10WD", *b"PATh")
    } else {
        (&[], *b"PATH")
    };
    let drop: &[[u8; 4]] = if dw {
        &[*b"PATh", *b"PATH", *b"ZONE"]
    } else {
        &[*b"PATH"]
    };
    let default = match type_id {
        DW_SAMPLER => Some((*b"MIDI", vec![2, 0])),
        SLICER => Some((*b"SENS", [&0.25f32.to_le_bytes()[..], &[1]].concat())),
        _ => None,
    };
    let previous = previous.unwrap_or_default();
    let previous = match magic.is_empty() {
        true => previous,
        false => previous.strip_prefix(magic).unwrap_or_default(),
    };
    let kept: Vec<_> = (ChunkIter::new(previous, "CSTM").map_while(Result::ok))
        .filter(|c| !drop.contains(&c.id))
        .collect();
    let mut writer = ChunkWriter::default();
    writer.raw(magic)?;
    writer.chunk(id, |w| w.content_path32(path))?;
    for chunk in &kept {
        writer.chunk(chunk.id, |w| w.raw(chunk.payload))?;
    }
    if let Some((id, payload)) = default.filter(|(id, _)| !kept.iter().any(|c| c.id == *id)) {
        writer.chunk(id, |w| w.raw(&payload))?;
    }
    Ok(writer.into_bytes())
}

/// `.spectra` file: 512 magnitudes, then 512 phases. `None` for any other size, and for one
/// louder than `MAX_PEAK`. A magnitude that is negative or not finite is 0; a phase that is not finite is 0, and any
/// other is wrapped into −π..π.
pub fn read_spectra(bytes: &[u8]) -> Option<Spectrum> {
    /// The loudest shape a file may make: its peak is at most its magnitudes' sum. A square to partial 512 sums to
    /// about 3.8; past this the file is refused, so a table is never inf or ear-splitting.
    const MAX_PEAK: f32 = 64.0;
    if bytes.len() != 2 * PARTIALS * 4 {
        return None;
    }
    let value = |i: usize| f32::from_le_bytes(bytes[4 * i..4 * i + 4].try_into().expect("4 bytes"));
    let finite = |v: f32| if v.is_finite() { v } else { 0.0 };
    let wrap = |p: f32| (p + PI).rem_euclid(2.0 * PI) - PI;
    let spectrum = Spectrum {
        mag: std::array::from_fn(|i| finite(value(i)).max(0.0)),
        phase: std::array::from_fn(|i| wrap(finite(value(PARTIALS + i)))),
    };
    (spectrum.mag.iter().sum::<f32>() <= MAX_PEAK).then_some(spectrum)
}

#[cfg(all(test, not(target_arch = "wasm32")))]
pub(crate) mod tests {
    use super::*;
    use crate::project_files::CONTENT_FOLDER;
    use crate::project_files::content::tests::{Temp, app, put};

    /// `.flms` of `floats` (43 of them): the magic, `HEAD` and `DATA` chunks.
    pub(crate) fn flms(name: &str, floats: &[f32]) -> Vec<u8> {
        let utf16 = name.encode_utf16();
        let mut head: Vec<u8> = [1, 1, utf16.clone().count() as u16]
            .iter()
            .flat_map(|v| v.to_le_bytes())
            .collect();
        head.extend(utf16.flat_map(u16::to_le_bytes));
        let data = [
            &b"0rev"[..],
            &floats
                .iter()
                .flat_map(|f| f.to_le_bytes())
                .collect::<Vec<_>>(),
        ]
        .concat();
        let mut bytes = b"\x99FLS\r\n\x1A\n".to_vec();
        for (id, payload) in [(b"HEAD", head), (b"DATA", data)] {
            bytes.extend(id);
            bytes.extend((payload.len() as u32).to_le_bytes());
            bytes.extend(payload);
        }
        bytes
    }

    /// `.flgsynth` with shapes `a` and `b` (`Waveshapes\<a>.spectra`) and `floats` (74 of them).
    pub(crate) fn flgsynth(a: &str, b: &str, floats: &[f32]) -> Vec<u8> {
        let mut bytes = b"2nhC".to_vec();
        for name in [a, b] {
            let mut field = [0u8; 512];
            let path = format!("Waveshapes\\{name}.spectra");
            field[..path.len()].copy_from_slice(path.as_bytes());
            bytes.extend(field);
        }
        bytes.extend(floats.iter().flat_map(|f| f.to_le_bytes()));
        bytes
    }

    /// `.spectra`: partial `n`'s magnitude `1/n` up to `top`, phases 0.
    pub(crate) fn spectra(top: usize) -> Vec<u8> {
        let mags = (1..=512).map(|n| if n <= top { 1.0 / n as f32 } else { 0.0 });
        mags.chain(std::iter::repeat_n(0.0, 512))
            .flat_map(f32::to_le_bytes)
            .collect()
    }

    /// The first `id` chunk's payload in a `CSTM` chunk list.
    fn chunk<'a>(blob: &'a [u8], id: &[u8; 4]) -> Option<&'a [u8]> {
        let found = ChunkIter::new(blob, "CSTM")
            .map_while(Result::ok)
            .find(|c| c.id == *id)?;
        Some(found.payload)
    }

    fn vstr32(path: &str) -> Vec<u8> {
        let shorthand = [b"@R@\0", path.as_bytes()].concat();
        [&(shorthand.len() as u32).to_le_bytes()[..], &shorthand].concat()
    }

    #[test]
    fn each_module_takes_its_own_file_types() {
        use crate::project_files::{FLGSYNTH_EXT, FLMS_EXT, PRESET_EXT, preset_exts};
        let audio = SAMPLE_EXTS.to_vec();
        assert_eq!(preset_exts(32), [PRESET_EXT]); // Reverb 2
        assert_eq!(preset_exts(23), [PRESET_EXT]); // SuperSaw
        assert_eq!(preset_exts(0), [PRESET_EXT, FLMS_EXT]);
        assert_eq!(preset_exts(2), [PRESET_EXT, FLGSYNTH_EXT]);
        assert_eq!(
            preset_exts(1),
            [[PRESET_EXT, DWP_EXT].as_slice(), &audio].concat()
        );
        assert_eq!(preset_exts(52), [PRESET_EXT, SF2_EXT]);
        assert_eq!(preset_exts(33), [[PRESET_EXT].as_slice(), &audio].concat());
        // 3x Osc: its `.fst` presets too.
        assert_eq!(
            preset_exts(39),
            [
                [PRESET_EXT, crate::project_files::FST_EXT].as_slice(),
                &audio
            ]
            .concat()
        );
    }

    #[test]
    fn the_factory_folders_are_the_content_trees_for_the_module() {
        let tmp = Temp::new("factory");
        let files = ProjectFiles::on_disk(&tmp.0);
        let c = tmp.0.join(CONTENT_FOLDER);
        for folder in [
            "Synth Presets2/GMSynth",
            "Synth Presets2/MiniSynth",
            "Synth Presets2/TransistorBass",
            "Synth Presets2/3xOsc",
            "Effect Presets/Reverb",
            "Effect Presets/Limiter 2",
            "DirectWave Samples",
            "DirectWave Samples EXP",
            "Loop Samples",
            "3xOsc Shapes",
            "SoundFonts",
        ] {
            std::fs::create_dir_all(c.join(folder)).unwrap();
        }
        assert_eq!(files.factory_presets(2), [c.join("Synth Presets2/GMSynth")]);
        assert_eq!(
            files.factory_presets(0),
            [c.join("Synth Presets2/MiniSynth")]
        );
        assert_eq!(
            files.factory_presets(19),
            [c.join("Synth Presets2/TransistorBass")]
        );
        // 3x Osc: its `.fst` presets, then its custom shapes.
        assert_eq!(
            files.factory_presets(39),
            [c.join("Synth Presets2/3xOsc"), c.join("3xOsc Shapes")]
        );
        assert_eq!(files.factory_presets(32), [c.join("Effect Presets/Reverb")]);
        assert_eq!(
            files.factory_presets(28),
            [c.join("Effect Presets/Limiter 2")]
        );
        assert_eq!(
            files.factory_presets(1),
            [
                c.join("DirectWave Samples"),
                c.join("DirectWave Samples EXP")
            ]
        );
        assert_eq!(files.factory_presets(33), [c.join("Loop Samples")]);
        // The SoundFont Player's: the content folder's `SoundFonts`, not a `Synth Presets2` folder.
        assert_eq!(files.factory_presets(52), [c.join("SoundFonts")]);
        // Not there: none. An unknown type: none.
        assert_eq!(files.factory_presets(23), Vec::<PathBuf>::new());
        assert_eq!(files.factory_presets(999), Vec::<PathBuf>::new());
    }

    #[test]
    fn a_preset_is_read_to_tell_its_module_and_a_named_file_is_not() {
        let reverb = crate::flm::write_preset(&ModuleInstance::new(32, 2)).unwrap();
        let unread = || -> Option<Vec<u8>> { panic!("read") };
        assert!(fits(Path::new("Hall.flmpst"), Some(32), || Some(
            reverb.clone()
        )));
        assert!(!fits(Path::new("Hall.flmpst"), Some(2), || Some(
            reverb.clone()
        )));
        assert!(!fits(Path::new("Gone.flmpst"), Some(32), || None));
        // A legacy preset is told by its magic.
        let old = flgsynth("Saw", "Saw", &[0.5; 74]);
        assert!(fits(Path::new("Bass.flgsynth"), Some(2), || Some(
            old.clone()
        )));
        assert!(!fits(Path::new("Bass.flgsynth"), Some(0), || Some(
            old.clone()
        )));
        assert!(fits(Path::new("Guitar.dwp"), Some(1), unread));
        // `.fst` lists for 3x Osc unread: its loader takes any file of the type.
        assert!(fits(Path::new("Gen.fst"), Some(39), unread));
        assert!(fits(Path::new("Loop.flac"), Some(33), unread));
        // The SoundFont Player takes a `.sf2` of any case unread, and reads anything else for its `HEAD`.
        assert!(fits(Path::new("Piano.SF2"), Some(52), unread));
        assert!(!fits(Path::new("Piano.sf2"), Some(2), || Some(
            reverb.clone()
        )));
        assert!(!fits(Path::new("Hall.flmpst"), Some(52), || Some(
            reverb.clone()
        )));
        // A rack or drum kit picker takes any file of its types.
        assert!(fits(Path::new("Kit.tmpl"), None, unread));
    }

    #[test]
    fn a_legacy_preset_pick_loads_through_the_preset_reader() {
        let floats: Vec<f32> = (0..74).map(|i| i as f32 / 100.0).collect();
        let mut keys = ModuleInstance::new(0, 5);
        let bytes = flms("Gen Keys", &floats[..43]);
        load_pick(&mut keys, Path::new("Gen Keys.flms"), &bytes, None).unwrap();
        assert_eq!(
            (keys.instance_id, keys.params[0], &keys.params[1..]),
            (5, 1.0, &floats[..43])
        );
        let mut gm = ModuleInstance::new(2, 3);
        let bytes = flgsynth("Devils Horn", "Saw", &floats);
        load_pick(&mut gm, Path::new("Gen.flgsynth"), &bytes, None).unwrap();
        assert_eq!(&gm.params[1..], &floats[..]);
        assert!(gm.file.custom_data.is_some(), "the shapes");
        // Into another module, or cut short: an error, and nothing changes.
        let before = ModuleInstance::new(0, 5);
        let mut other = before.clone();
        assert!(load_pick(&mut other, Path::new("Gen.flgsynth"), &bytes, None).is_err());
        assert!(load_pick(&mut other, Path::new("x.flms"), b"RIFF", None).is_err());
        assert_eq!(other, before);
    }

    #[test]
    fn an_fst_pick_loads_into_3x_osc_with_its_custom_shape() {
        use crate::project_files::fst::tests::{blob, fst, ints, shape, version};
        let osc = blob(213, &ints(&[3, -12, 50, 32, 6, 24, -100, 96, 0, 0, 0]));
        let custom = shape(r"\Instruments\Shapes\HQ\Gen Shape.wav");
        let bytes = fst(&[version("3.1.1"), osc.clone(), custom]);
        let path = Path::new("Original Content/Synth Presets2/3xOsc/Bass/Gen.fst");
        // A module with a picked shape and an edited param: the preset replaces both, the id and place stay.
        let mut module = ModuleInstance::new(39, 4);
        module.minimized = true;
        let shape_path = app("3xOsc Shapes/Other.wav");
        load_pick(
            &mut module,
            Path::new("Other.wav"),
            b"RIFF",
            Some(shape_path),
        )
        .unwrap();
        module.params[120] = 1.0;
        load_pick(&mut module, path, &bytes, None).unwrap();
        let import = crate::project_files::import_fst(&bytes, path);
        assert_eq!(module.params, import.params);
        assert_eq!((module.params[1], module.params[120]), (0.5, 0.0));
        assert_eq!((module.instance_id, module.minimized), (4, true));
        let blob = module.file.custom_data.clone().unwrap();
        assert_eq!(
            chunk(&blob, b"PATH"),
            Some(&vstr32("3xOsc Shapes/HQ/Gen Shape.wav")[..])
        );
        assert_eq!(
            (&*module.file.preset_name, &*module.file.category),
            ("Gen", "Bass")
        );
        // No custom waveform, or one that does not resolve: the module's stays.
        let kept = module.file.custom_data.clone();
        load_pick(&mut module, path, &fst(std::slice::from_ref(&osc)), None).unwrap();
        assert_eq!(module.file.custom_data, kept);
        let unresolved = fst(&[osc, shape(r"\Patches\Shapes\x.wav")]);
        load_pick(&mut module, path, &unresolved, None).unwrap();
        assert_eq!(module.file.custom_data, kept);
        assert_eq!(module.params[1], 0.5);
        // A failed import (cut short) loads: the module at its defaults, the partial import discarded, the custom
        // waveform untouched, the name set.
        module.params[2] = 0.75;
        let mut cut = bytes.clone();
        cut.truncate(cut.len() - 1);
        load_pick(&mut module, path, &cut, None).unwrap();
        assert_eq!(module.params, ModuleInstance::new(39, 0).params);
        assert_eq!(module.params[0], 1.0);
        assert_eq!(module.file.custom_data, kept);
        assert_eq!((module.instance_id, &*module.file.preset_name), (4, "Gen"));
        // Then the bytes are read as native chunks: a 3x Osc preset in a `.fst` file loads.
        let mut native = ModuleInstance::new(39, 0);
        native.params[2] = 0.25;
        let native = crate::flm::write_preset(&native).unwrap();
        load_pick(&mut module, path, &native, None).unwrap();
        assert_eq!(module.params[2], 0.25);
        // `.fst` for another module: an error, and nothing changes.
        let mut gm = ModuleInstance::new(2, 3);
        assert!(load_pick(&mut gm, path, &bytes, None).is_err());
        assert_eq!(gm, ModuleInstance::new(2, 3));
    }

    #[test]
    fn a_module_preset_pick_loads_the_preset() {
        let mut hall = ModuleInstance::new(32, 9);
        hall.params[1] = 0.25;
        let bytes = crate::flm::write_preset(&hall).unwrap();
        let mut module = ModuleInstance::new(32, 4);
        load_pick(&mut module, Path::new("Hall.flmpst"), &bytes, None).unwrap();
        assert_eq!((module.instance_id, module.params[1]), (4, 0.25));
        // Another module's preset does not load.
        let mut synth = ModuleInstance::new(2, 4);
        assert!(load_pick(&mut synth, Path::new("Hall.flmpst"), &bytes, None).is_err());
    }

    #[test]
    fn a_soundfont_pick_sets_the_players_sf2d_at_patch_0_and_keeps_its_other_chunks() {
        // `SF2D` is the shorthand path and the patch index. A user or content file.
        let mut player = ModuleInstance::new(52, 2);
        player.params[3] = 0.4;
        let params = player.params.clone();
        let other = [b"XTRA".as_slice(), &2_u32.to_le_bytes(), &[7, 9]].concat();
        player.file.custom_data = Some(other.clone());
        let mine = ContentPath::Shorthand {
            root: crate::flm::chunk::ContentRoot::User,
            relative: "My SoundFonts/Pad.sf2".into(),
        };
        for path in [app("SoundFonts/Keys/Piano.sf2"), mine] {
            let mut module = player.clone();
            load_pick(&mut module, Path::new("x.sf2"), &[], Some(path.clone())).unwrap();
            let blob = module.file.custom_data.clone().unwrap();
            let sf2d = crate::flm::sf2d::Sf2d::read(&blob).unwrap();
            assert_eq!((sf2d.path, sf2d.patch), (path, 0));
            assert_eq!(chunk(&blob, b"XTRA"), Some(&[7, 9][..]));
            assert_eq!(module.params, params, "a SoundFont pick changes no param");
        }
        // Without a shorthand (outside the user and content folders) there is nothing to name: an error, no change.
        let mut module = player.clone();
        assert!(load_pick(&mut module, Path::new("x.sf2"), &[], None).is_err());
        assert_eq!(module, player);
    }

    #[test]
    fn a_program_loop_or_shape_pick_sets_the_modules_path_chunk() {
        let path = app("DirectWave Samples/Guitar/Gen Guitar.dwp");
        let mut dw = ModuleInstance::new(1, 2);
        // An earlier program's zones and its MIDI chunk: the zones go, MIDI stays.
        let old = [
            &b"10WD"[..],
            b"ZONE",
            &1065u32.to_le_bytes(),
            &[0; 1065],
            b"MIDI",
            &2u32.to_le_bytes(),
            &[7, 1],
        ]
        .concat();
        dw.file.custom_data = Some(old);
        let params = dw.params.clone();
        load_pick(&mut dw, Path::new("Gen Guitar.dwp"), b"DwPr", Some(path)).unwrap();
        let blob = dw.file.custom_data.clone().unwrap();
        assert!(blob.starts_with(b"10WD"));
        assert_eq!(
            chunk(&blob[4..], b"PATh"),
            Some(&vstr32("DirectWave Samples/Guitar/Gen Guitar.dwp")[..])
        );
        assert_eq!(chunk(&blob[4..], b"ZONE"), None);
        assert_eq!(chunk(&blob[4..], b"MIDI"), Some(&[7, 1][..]));
        assert_eq!(dw.params, params, "a program pick changes no param");
        // A new DW Sampler writes the new module's MIDI (2, 0).
        let mut dw = ModuleInstance::new(1, 2);
        let loop_path = app("Loop Samples/Sliced Drums/Gen Loop.flac");
        load_pick(
            &mut dw,
            Path::new("Gen Loop.flac"),
            b"fLaC",
            Some(loop_path.clone()),
        )
        .unwrap();
        let blob = dw.file.custom_data.clone().unwrap();
        assert_eq!(chunk(&blob[4..], b"MIDI"), Some(&[2, 0][..]));
        // The Slicer: its PATH, and a new module's SENS (0.25, AUTO on) where it has none.
        let mut slicer = ModuleInstance::new(33, 2);
        load_pick(
            &mut slicer,
            Path::new("Gen Loop.flac"),
            b"fLaC",
            Some(loop_path),
        )
        .unwrap();
        let blob = slicer.file.custom_data.clone().unwrap();
        assert_eq!(
            chunk(&blob, b"PATH"),
            Some(&vstr32("Loop Samples/Sliced Drums/Gen Loop.flac")[..])
        );
        assert_eq!(
            chunk(&blob, b"SENS"),
            Some(&[&0.25f32.to_le_bytes()[..], &[1]].concat()[..])
        );
        // 3x Osc: its PATH only.
        let mut osc = ModuleInstance::new(39, 2);
        let shape = app("3xOsc Shapes/HQ/Gen Shape.wav");
        load_pick(&mut osc, Path::new("Gen Shape.wav"), b"RIFF", Some(shape)).unwrap();
        let blob = osc.file.custom_data.clone().unwrap();
        assert_eq!(
            chunk(&blob, b"PATH"),
            Some(&vstr32("3xOsc Shapes/HQ/Gen Shape.wav")[..])
        );
        assert_eq!(chunk(&blob, b"SENS"), None);
        // Without a shorthand (a file outside both roots) nothing loads.
        let before = ModuleInstance::new(33, 2);
        let mut slicer = before.clone();
        assert!(load_pick(&mut slicer, Path::new("a.wav"), b"RIFF", None).is_err());
        assert_eq!(slicer, before);
        // A file type the module does not take fails.
        let mut reverb = ModuleInstance::new(32, 2);
        let wav = Some(app("a.wav"));
        assert!(load_pick(&mut reverb, Path::new("a.wav"), b"RIFF", wav).is_err());
    }

    #[test]
    fn a_spectra_file_reads_its_magnitudes_and_phases() {
        let spectrum = read_spectra(&spectra(3)).unwrap();
        assert_eq!(&spectrum.mag[..4], &[1.0, 0.5, 1.0 / 3.0, 0.0]);
        assert!(spectrum.phase.iter().all(|&p| p == 0.0));
        assert_eq!(read_spectra(&spectra(3)[..4095]), None);
        // Hostile values: a negative or non-finite magnitude is 0, a phase is kept in −π..π.
        let mut bytes = spectra(1);
        bytes[..4].copy_from_slice(&f32::NAN.to_le_bytes());
        bytes[4..8].copy_from_slice(&(-1.0f32).to_le_bytes());
        bytes[2048..2052].copy_from_slice(&f32::INFINITY.to_le_bytes());
        bytes[2052..2056].copy_from_slice(&10.0f32.to_le_bytes());
        let spectrum = read_spectra(&bytes).unwrap();
        assert_eq!((spectrum.mag[0], spectrum.mag[1]), (0.0, 0.0));
        assert_eq!(spectrum.phase[0], 0.0);
        assert!((-std::f32::consts::PI..=std::f32::consts::PI).contains(&spectrum.phase[1]));
        // Huge finite magnitudes would make a table that is inf or ear-splitting: the file is refused. Its loudest
        // possible peak is the magnitudes' sum.
        let mut loud = spectra(1);
        loud[8..12].copy_from_slice(&1e7f32.to_le_bytes());
        assert_eq!(read_spectra(&loud), None);
        let mut many = Vec::new();
        for _ in 0..512 {
            many.extend(1.0f32.to_le_bytes());
        }
        many.extend([0; 2048]);
        assert_eq!(read_spectra(&many), None, "512 partials at 1.0");
        // A square to its 512th partial (Σ 1/n over odd n ≈ 3.8) is fine.
        let square: Vec<u8> = (1..=512)
            .map(|n| if n % 2 == 1 { 1.0 / n as f32 } else { 0.0 })
            .chain(std::iter::repeat_n(0.0, 512))
            .flat_map(f32::to_le_bytes)
            .collect();
        assert!(read_spectra(&square).is_some());
    }

    #[test]
    fn the_waveshapes_take_their_slots_by_file_name() {
        let tmp = Temp::new("spectra");
        let files = ProjectFiles::on_disk(&tmp.0);
        assert!(files.original_waveshapes().is_empty());
        let dir = tmp.0.join(CONTENT_FOLDER).join(WAVESHAPES_FOLDER);
        put(&dir.join("Saw.spectra"), &spectra(3));
        put(&dir.join("Beast.spectra"), &spectra(1));
        put(&dir.join("Saw Mean.spectra"), &spectra(2));
        put(&dir.join("Broken.spectra"), b"short");
        put(&dir.join("notes.txt"), b"x");
        let shapes = files.original_waveshapes();
        let slots: Vec<(usize, f32)> = shapes.iter().map(|(i, s)| (*i, s.mag[2])).collect();
        // Beast 0, Broken 1 (left out), Saw Mean 2, Saw 3: by file name, a space before the dot.
        assert_eq!(slots, [(0, 0.0), (2, 0.0), (3, 1.0 / 3.0)]);
    }
}
