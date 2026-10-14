//! A channel's instruments: one constructor per module type id. Each instrument lane appends its arm to
//! [`instrument`].

use std::hash::{DefaultHasher, Hash, Hasher};
use std::sync::Arc;
use std::sync::atomic::{AtomicU64, Ordering};

use super::samples::{dw_path, shape_key};
use crate::flm::sf2d::Sf2d;
use crate::panels::cstm::settings_from_cstm;
use crate::params::{Role, module_spec};
use crate::sample_cache::{SampleCache, SampleKey};
use crate::song::{Channel, ModuleInstance};
use honeydew_audio::dw_sampler::{DwSampler, DwZone};
use honeydew_audio::dwp::Cstm;
use honeydew_audio::gmsynth::{self, GmSynth};
use honeydew_audio::instrument::{Instrument, InstrumentModule};
use honeydew_audio::minisynth::MiniSynth;
use honeydew_audio::soundfont::SoundFontPlayer;
use honeydew_audio::supersaw::SuperSaw;
use honeydew_audio::three_osc::ThreeOsc;
use honeydew_audio::transistor_bass::TransistorBass;

/// The instrument for `module`'s type on `channel` at `sample_rate`, playing samples from `samples`; `None` for a type
/// without one yet. `origin` hears what it is built from besides its params ([`InstrumentModule::origin`]): the data a
/// type builds from (the DW Sampler's and SoundFont Player's `CSTM`, the Slicer's `CSTM` and first key) and each
/// sample it plays, by its identity, so a sample read again under the same path (an audio editor's rewrite) is a new
/// build. Its other settings arrive live ([`super::settings::rebuilds`]).
fn instrument(
    channel: &Channel,
    module: &ModuleInstance,
    sample_rate: f32,
    samples: &SampleCache,
    subdivision: u8,
    origin: &mut DefaultHasher,
) -> Option<Box<dyn Instrument>> {
    match module.type_id {
        0 => Some(Box::new(MiniSynth::new(sample_rate).ok()?)),
        1 => Some(Box::new(dw_sampler(module, sample_rate, samples, origin))),
        2 => Some(Box::new(gm_synth(module, sample_rate))),
        19 => Some(Box::new(TransistorBass::new(sample_rate))),
        // The Custom shape: its `CSTM` `PATH`'s table, built once by the sample cache; without it (or a file that is
        // missing) the table is all zeros.
        39 => {
            let wave = shape_key(module).and_then(|key| samples.shape(&key));
            wave.as_ref().map(Arc::as_ptr).hash(origin);
            Some(Box::new(ThreeOsc::with_custom_wave(
                sample_rate,
                subdivision,
                wave,
            )))
        }
        23 => Some(Box::new(SuperSaw::new(sample_rate))),
        33 => Some(Box::new(super::slicer::instrument(
            channel,
            module,
            sample_rate,
            samples,
            origin,
        )?)),
        52 => Some(Box::new(soundfont_player(
            module,
            sample_rate,
            samples,
            origin,
        ))),
        _ => None,
    }
}

/// The DW Sampler with its `CSTM`'s zones: its program's (`PATh`/`PATH`) or its `ZONE`s, never both ([`Cstm::read`]),
/// each with its decoded sample. A zone whose sample is missing is dropped.
fn dw_sampler(
    module: &ModuleInstance,
    sample_rate: f32,
    samples: &SampleCache,
    origin: &mut DefaultHasher,
) -> DwSampler {
    let cstm = Cstm::read(module.file.custom_data.as_deref().unwrap_or_default());
    let zones: Vec<DwZone> = match &cstm.program {
        Some(program) => (dw_path(program.path).and_then(|path| samples.program(&path)))
            .iter()
            .flat_map(|zones| zones.iter())
            .filter_map(|(zone, key)| {
                Some(DwZone {
                    zone: *zone,
                    sample: samples.get(key)?,
                })
            })
            .collect(),
        None => (cstm.zones.iter())
            .filter_map(|(path, zone)| {
                Some(DwZone {
                    zone: *zone,
                    sample: samples.get(&SampleKey::File(dw_path(path)?))?,
                })
            })
            .collect(),
    };
    module.file.custom_data.hash(origin);
    for zone in &zones {
        Arc::as_ptr(&zone.sample).hash(origin);
    }
    DwSampler::new(sample_rate, zones, cstm.midi)
}

/// A GMSynth with `module`'s `CSTM` settings (shapes, PB RANGE, MW); a missing or short blob keeps the defaults. The
/// shapes are built here, on the app thread, before the synth takes them.
fn gm_synth(module: &ModuleInstance, sample_rate: f32) -> GmSynth {
    let mut synth = GmSynth::new(sample_rate);
    let blob = module.file.custom_data.as_deref();
    for (key, value) in blob
        .and_then(|blob| settings_from_cstm(2, blob))
        .into_iter()
        .flatten()
    {
        gmsynth::prepare_setting(key, value);
        synth.set_setting(key, value);
    }
    synth
}

/// A SoundFont Player playing the patch its `SF2D` names from the SoundFont `samples` loaded; silent without one.
fn soundfont_player(
    module: &ModuleInstance,
    sample_rate: f32,
    samples: &SampleCache,
    origin: &mut DefaultHasher,
) -> SoundFontPlayer {
    let sf2d = module.file.custom_data.as_deref().and_then(Sf2d::read);
    let bank = sf2d.and_then(|sf2d| Some((samples.soundfont(&sf2d.path)?, sf2d.patch)));
    module.file.custom_data.hash(origin);
    bank.as_ref()
        .map(|(font, _)| Arc::as_ptr(font))
        .hash(origin);
    SoundFontPlayer::new(sample_rate, bank)
}

/// `channel`'s instruments in rack order, by instance id, with scratch room for `max_block` frames. `subdivision` is
/// the song's TDIV byte 0 (3x Osc's D). Their params
/// arrive with the engine's initial state. An instrument type without a constructor is left out (silent) and logged
/// once per process.
pub(super) fn build(
    channel: &Channel,
    sample_rate: f32,
    tempo_bpm: f32,
    max_block: usize,
    samples: &SampleCache,
    subdivision: u8,
) -> Vec<(i32, InstrumentModule)> {
    let is_instrument = |module: &&ModuleInstance| {
        module_spec(module.type_id).is_some_and(|spec| spec.role == Role::Instrument)
    };
    (channel.rack.modules.iter().filter(is_instrument))
        .filter_map(|module| {
            let mut origin = DefaultHasher::new();
            module.type_id.hash(&mut origin);
            let built = instrument(
                channel,
                module,
                sample_rate,
                samples,
                subdivision,
                &mut origin,
            );
            let Some(mut instrument) = built else {
                log_silent(module.type_id);
                return None;
            };
            instrument.set_tempo_bpm(tempo_bpm);
            let mut wrapped = InstrumentModule::new(instrument, max_block);
            wrapped.origin = origin.finish();
            // `ADD1`: the reader keeps low ≤ high and low ≥ 0; a negative high hears nothing.
            let (low, high) = (module.file.key_low, module.file.key_high);
            wrapped.keys = (low.max(0) as u8, high.max(0) as u8);
            Some((module.instance_id, wrapped))
        })
        .collect()
}

/// Logs a silent instrument type the first time this process meets it. Instrument type ids are below 64.
fn log_silent(type_id: u16) {
    static LOGGED: AtomicU64 = AtomicU64::new(0);
    let bit = 1 << (type_id % 64);
    if LOGGED.fetch_or(bit, Ordering::Relaxed) & bit == 0 {
        eprintln!("audio: instrument type {type_id} has no engine yet and stays silent");
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::flm::chunk::ContentPath;
    use crate::panels::cstm::GMSYNTH;
    use honeydew_audio::ProcessContext;

    #[test]
    fn gmsynth_takes_the_panels_cstm_keys() {
        let keys: Vec<u32> = GMSYNTH.iter().map(|field| field.n).collect();
        assert_eq!(
            keys,
            [
                gmsynth::WAVE_A,
                gmsynth::WAVE_B,
                gmsynth::PB_RANGE,
                gmsynth::MW_AMOUNT,
                gmsynth::MW_DEST
            ]
        );
    }

    #[test]
    fn a_new_gmsynth_seeds_its_fx_kinds_exactly_on_their_steps() {
        // FX 1 FLANGER, FX 2 DELAY, FX 3 LPF: exactly 1/9, 5/9 and 3/9, so the DSP's plain trunc(9v), and any reader of
        // A saved song, land on them.
        let module = ModuleInstance::new(2, 1);
        for (index, k) in [(46, 1), (53, 5), (60, 3)] {
            let v = module.params[index];
            assert_eq!(v, k as f32 / 9.0, "param {index}");
            assert_eq!((9.0 * v) as usize, k, "param {index}");
        }
    }

    #[test]
    fn gmsynth_defaults_match_the_registry() {
        let registry = &module_spec(2).unwrap().params;
        assert_eq!(registry.len(), gmsynth::PARAMS);
        for (index, (param, &dsp)) in registry.iter().zip(&gmsynth::DEFAULTS).enumerate() {
            assert_eq!(dsp, param.default, "param {index} ({})", param.name);
        }
    }

    /// A song whose channel 1 is a 3x Osc, its `CSTM` `PATH` naming `path` (none for `None`) as the sample picker writes
    /// it ([`crate::project_files::load_pick`]), with its samples loaded through `read`.
    fn three_osc_song(
        path: Option<ContentPath>,
        read: &crate::sample_cache::Reader<'_>,
    ) -> (crate::song::Song, SampleCache) {
        let mut song = crate::song::Song::default();
        song.add_channel(crate::song::ChannelKind::Synth(
            crate::song::SynthKind::ThreeXOsc,
        ));
        let module = &mut song.channels[1].rack.modules[0];
        assert_eq!(module.type_id, 39);
        if let Some(path) = path {
            let picked = std::path::Path::new("shape.wav");
            crate::project_files::load_pick(module, picked, &[], Some(path)).unwrap();
        }
        let mut samples = SampleCache::default();
        samples.load(&song, read);
        (song, samples)
    }

    /// Osc 1 alone on Shape `shape` (Custom 1.0, Saw 0.5) of `synth`, playing A4 for 0.1 s.
    fn play_a4(synth: &mut dyn Instrument, shape: f32) -> Vec<f32> {
        for (i, v) in [(1, shape), (22, 0.0), (23, 0.0)] {
            synth.set_parameter(i, v);
        }
        synth.note_on(69, 1.0, 0.5);
        let (mut left, mut right) = (vec![0.0; 4800], vec![0.0; 4800]);
        let context = ProcessContext {
            sample_rate: 48_000.0,
            tempo_bpm: 120.0,
            start_beat: 0.0,
            playing: false,
        };
        synth.process(&mut left, &mut right, context);
        left
    }

    fn app_path(relative: &str) -> ContentPath {
        ContentPath::Shorthand {
            root: crate::flm::chunk::ContentRoot::App,
            relative: relative.into(),
        }
    }

    #[test]
    fn three_osc_plays_the_shape_its_cstm_path_names() {
        use honeydew_audio::builtin_shapes::shape;
        let vowel = Some(app_path("Built-in/Shapes/Vowel A.wav"));
        let (song, samples) = three_osc_song(vowel, &|_| None);
        let build = || {
            let module = &song.channels[1].rack.modules[0];
            instrument(
                &song.channels[1],
                module,
                48_000.0,
                &samples,
                4,
                &mut DefaultHasher::new(),
            )
            .unwrap()
        };
        let played = play_a4(build().as_mut(), 1.0);
        let shape = shape("Vowel A").unwrap();
        let mut want = ThreeOsc::with_custom_shape(48_000.0, 4, Some(&shape));
        assert_eq!(played, play_a4(&mut want, 1.0));
        assert!(played.iter().any(|v| v.abs() > 0.05));
        assert_ne!(played, play_a4(build().as_mut(), 0.5), "not the Saw");
    }

    #[test]
    fn a_custom_table_is_built_once_and_shared_by_each_build() {
        use std::sync::Arc;
        let vowel = app_path("Built-in/Shapes/Vowel A.wav");
        let key = SampleKey::of_path(&vowel);
        let (song, mut samples) = three_osc_song(Some(vowel), &|_| None);
        let wave = samples.shape(&key).expect("built at load");
        // Only the table stays, and nothing is missing.
        assert!(samples.get(&key).is_none());
        assert_eq!(samples.missing().count(), 0);
        samples.load(&song, &|_| None);
        assert!(Arc::ptr_eq(&wave, &samples.shape(&key).unwrap()));
        let module = &song.channels[1].rack.modules[0];
        let built = instrument(
            &song.channels[1],
            module,
            48_000.0,
            &samples,
            4,
            &mut DefaultHasher::new(),
        )
        .unwrap();
        assert_eq!(
            Arc::strong_count(&wave),
            3,
            "the cache's, this one and the build's"
        );
        drop(built);
        // A song without it frees it.
        samples.load(&crate::song::Song::default(), &|_| None);
        assert!(samples.shape(&key).is_none());
        assert_eq!(Arc::strong_count(&wave), 1);
    }

    #[test]
    fn three_osc_custom_without_a_waveform_is_silent() {
        // The fresh table is all zeros; an unresolved path leaves it so.
        let missing = Some(app_path("3xOsc Shapes/Gone.wav"));
        for (path, gone) in [(None, 0), (missing, 1)] {
            let (song, samples) = three_osc_song(path, &|_| None);
            assert_eq!(samples.missing().count(), gone);
            let module = &song.channels[1].rack.modules[0];
            let mut built = instrument(
                &song.channels[1],
                module,
                48_000.0,
                &samples,
                4,
                &mut DefaultHasher::new(),
            )
            .unwrap();
            assert!(play_a4(built.as_mut(), 1.0).iter().all(|&v| v == 0.0));
            assert!(
                play_a4(built.as_mut(), 0.5).iter().any(|v| v.abs() > 0.05),
                "Saw sounds"
            );
        }
    }

    #[test]
    fn a_built_in_shape_path_keys_its_shape_without_the_extension() {
        let module = |path: ContentPath| {
            let (mut song, _) = three_osc_song(Some(path), &|_| None);
            song.channels[1].rack.modules.remove(0)
        };
        let builtin = SampleKey::Builtin("Built-in/Shapes/Vowel A".into());
        for path in [
            "Built-in/Shapes/Vowel A.wav",
            "Built-in\\Shapes\\Vowel A.wav",
        ] {
            assert_eq!(shape_key(&module(app_path(path))), Some(builtin.clone()));
        }
        let user = ContentPath::Shorthand {
            root: crate::flm::chunk::ContentRoot::User,
            relative: "Built-in/Shapes/Vowel A.wav".into(),
        };
        assert_eq!(
            shape_key(&module(user.clone())),
            Some(SampleKey::File(user))
        );
        let original = app_path("3xOsc Shapes/Basic/Saw.wav");
        assert_eq!(
            shape_key(&module(original.clone())),
            Some(SampleKey::File(original))
        );
    }

    /// The SoundFont Player's instrument for a song naming `bank` at `My SoundFonts/bank.sf2`, and the cache's view of it.
    fn soundfont_instrument(bank: &[u8]) -> (Box<dyn Instrument>, bool) {
        use crate::flm::chunk::{ContentPath, ContentRoot};
        use crate::flm::sf2d::Sf2d;
        let path = ContentPath::Shorthand {
            root: ContentRoot::User,
            relative: "My SoundFonts/bank.sf2".into(),
        };
        let mut song = crate::song::Song::default();
        song.add_channel(crate::song::ChannelKind::Synth(
            crate::song::SynthKind::SoundFontPlayer,
        ));
        let sf2d = Sf2d {
            path: path.clone(),
            patch: 0,
        };
        let module = &mut song.channels[1].rack.modules[0];
        module.file.custom_data = Some(sf2d.write(None).unwrap());
        let mut cache = SampleCache::default();
        cache.load(&song, &|p| (*p == path).then(|| bank.to_vec()));
        let module = &song.channels[1].rack.modules[0];
        let built = instrument(
            &song.channels[1],
            module,
            48_000.0,
            &cache,
            4,
            &mut DefaultHasher::new(),
        );
        (built.unwrap(), cache.soundfont(&path).is_some())
    }

    fn played(synth: &mut dyn Instrument) -> Vec<f32> {
        let (mut left, mut right) = (vec![0.0; 2048], vec![0.0; 2048]);
        synth.note_on(69, 1.0, 0.5);
        let context = ProcessContext {
            sample_rate: 48_000.0,
            tempo_bpm: 120.0,
            start_beat: 0.0,
            playing: false,
        };
        synth.process(&mut left, &mut right, context);
        [left, right].concat()
    }

    #[test]
    fn a_soundfont_player_naming_a_bank_the_engine_cannot_parse_builds_and_is_silent() {
        // The rack shows the error for such a bank (`sf2::presets` rejects what the engine rejects); the
        // engine itself builds the module without a SoundFont and renders nothing, finite and without a panic.
        let sine = include_bytes!("../../crates/audio/tests/fixtures/sine.sf2");
        let (mut whole, loaded) = soundfont_instrument(sine);
        assert!(loaded && played(whole.as_mut()).iter().any(|&s| s != 0.0));
        // Cut off in the samples (patch headers whole), cut off anywhere, and junk.
        let cut = crate::sf2::tests::cut_off_samples(&[("A", 0, 0)]);
        let half = sine[..sine.len() / 2].to_vec();
        for bank in [cut, half, b"junk".to_vec()] {
            let (mut synth, loaded) = soundfont_instrument(&bank);
            assert!(!loaded);
            assert!(played(synth.as_mut()).iter().all(|&s| s == 0.0));
        }
    }

    #[test]
    fn gmsynth_plays_its_cstm_shapes() {
        let render = |synth: &mut dyn Instrument| {
            let (mut left, mut right) = (vec![0.0; 2048], vec![0.0; 2048]);
            synth.note_on(69, 1.0, 0.5);
            let context = ProcessContext {
                sample_rate: 48_000.0,
                tempo_bpm: 120.0,
                start_beat: 0.0,
                playing: false,
            };
            synth.process(&mut left, &mut right, context);
            left
        };
        // waveA 18 (Sine), waveB 16 (Saw), PB RANGE 1/3, MW AMOUNT 0.5, MW DEST 1.0.
        let mut module = ModuleInstance::new(2, 1);
        let words: [u32; 5] = [
            18,
            16,
            (1.0_f32 / 3.0).to_bits(),
            0.5_f32.to_bits(),
            1.0_f32.to_bits(),
        ];
        module.file.custom_data = Some(words.iter().flat_map(|w| w.to_le_bytes()).collect());
        let mut song = crate::song::Song::default();
        song.add_channel(crate::song::ChannelKind::Synth(
            crate::song::SynthKind::GmSynth,
        ));
        let built = instrument(
            &song.channels[1],
            &module,
            48_000.0,
            &SampleCache::default(),
            4,
            &mut DefaultHasher::new(),
        );
        let mut built = built.unwrap();
        let mut sine = GmSynth::new(48_000.0);
        gmsynth::prepare_setting(gmsynth::WAVE_A, 18.0);
        gmsynth::prepare_setting(gmsynth::WAVE_B, 16.0);
        sine.set_setting(gmsynth::WAVE_A, 18.0);
        sine.set_setting(gmsynth::WAVE_B, 16.0);
        let played = render(built.as_mut());
        assert_eq!(played, render(&mut sine));
        assert_ne!(
            played,
            render(&mut GmSynth::new(48_000.0)),
            "not the default shape"
        );
    }
}
