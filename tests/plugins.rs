//! Third-party plugin hosting with real plugins built from source: nih-plug's Gain (an effect) and PolyModSynth (an
//! instrument), each a CLAP and a VST3 (`vst-test-plugins/README.md`). Every test is skipped when they are absent. No
//! audio device: songs render offline.
#![cfg(plugin_host)]

use std::path::{Path, PathBuf};

use honeydew::plugins::{self, scan};
use honeydew::render::{Remainder, Render, RenderRange, RenderSpec};
use honeydew::sample_cache::SampleCache;
use honeydew::song::plugin::{PLUGIN_EFFECT, PLUGIN_INSTRUMENT};
use honeydew::song::{ChannelKind, Event, Song};

/// The test plugins' folder: `HONEYDEW_TEST_PLUGINS`, else the shared scratch build. `None` (and the test skipped) when
/// they were not built.
fn test_plugins() -> Option<PathBuf> {
    let dir = std::env::var_os("HONEYDEW_TEST_PLUGINS")
        .map(PathBuf::from)
        .unwrap_or_else(|| "/mnt/storage/shared/playground/scratch/vst-test-plugins".into());
    let built = dir.join("clap/Gain.clap").is_file() && dir.join("vst3/Gain.vst3").is_dir();
    if !built {
        eprintln!("skipped: no test plugins in {}", dir.display());
    }
    built.then_some(dir)
}

/// `dir` scanned as the app scans: each file in a scanner process (this package's app).
fn scanned(dir: &Path) -> scan::Cache {
    let scanner = Path::new(env!("CARGO_BIN_EXE_honeydew-native"));
    scan::sweep(
        &scan::discover(&[dir.to_path_buf()]),
        &scan::Cache::default(),
        false,
        scanner,
    )
}

/// Lists `dir`'s plugins for this thread's host; the uid of the plugin `name` in `format`.
fn installed(dir: &Path) -> impl Fn(&str, &str) -> String {
    plugins::load_cache(&serde_json::to_string(&scanned(dir)).unwrap());
    let list = plugins::available();
    move |name: &str, format: &str| {
        let found = list.iter().find(|a| a.name == name && a.format == format);
        found
            .unwrap_or_else(|| panic!("{name} ({format}) not found"))
            .uid
            .clone()
    }
}

/// The index of plugin param `name` in the module's plugin params.
fn param(song: &Song, channel: usize, at: usize, name: &str) -> usize {
    let plugin = song.channels[channel].rack.modules[at]
        .file
        .plugin
        .as_ref()
        .unwrap();
    plugin.params.iter().position(|p| p.name == name).unwrap()
}

#[test]
fn the_scanner_lists_plugins_from_processes_of_its_own() {
    let Some(dir) = test_plugins() else { return };
    // A broken file next to them: it fails in its scanner, and the scan goes on.
    let broken = Path::new(env!("CARGO_TARGET_TMPDIR")).join("plugin-scan");
    std::fs::create_dir_all(&broken).unwrap();
    std::fs::write(broken.join("Broken.clap"), b"not a plugin").unwrap();
    std::fs::copy(dir.join("clap/Gain.clap"), broken.join("Gain.clap")).unwrap();
    let cache = scanned(&broken);
    let file = |name: &str| cache.files.iter().find(|f| f.path.ends_with(name)).unwrap();
    assert_eq!(file("Broken.clap").status, scan::Status::Error);
    let gain = file("Gain.clap");
    assert_eq!(
        (gain.status, gain.plugins[0].name.as_str()),
        (scan::Status::Ok, "Gain")
    );
    assert_eq!(gain.plugins[0].kind, "effect");
    // Both formats of the built folder, an instrument among them.
    let all = scanned(&dir);
    let kinds: Vec<(String, String, String)> = (all.files.iter())
        .flat_map(|f| {
            f.plugins
                .iter()
                .map(move |p| (f.format.clone(), p.name.clone(), p.kind.clone()))
        })
        .collect();
    for (format, name, kind) in [
        ("clap", "Gain", "effect"),
        ("vst3", "Gain", "effect"),
        ("clap", "Poly Mod Synth", "instrument"),
        ("vst3", "Poly Mod Synth", "instrument"),
    ] {
        let want = (format.to_string(), name.to_string(), kind.to_string());
        assert!(kinds.contains(&want), "{want:?} in {kinds:?}");
    }
}

/// The energy of `song` rendered offline (the export's renderer), its tail cut.
fn rendered_energy(song: &Song) -> f32 {
    let spec = RenderSpec {
        range: RenderRange::Full,
        remainder: Remainder::Cut,
        normalize: false,
        high_quality: true,
        rate: 48_000,
        solo: None,
        master_effects: true,
    };
    let mut stream = Render::new(song, &SampleCache::default(), spec)
        .unwrap()
        .finish()
        .unwrap();
    let mut energy = 0.0f32;
    while !stream.done() {
        if let Some([l, r]) = stream.step() {
            energy += l.iter().chain(r).map(|s| s * s).sum::<f32>();
        }
    }
    energy
}

#[test]
fn an_export_renders_a_plugin_instrument_through_a_plugin_effect() {
    let Some(dir) = test_plugins() else { return };
    let uid = installed(&dir);
    let mut song = Song {
        tempo_bpm: 120.0,
        ..Song::default()
    };
    song.add_channel(ChannelKind::Instrument);
    let ch = &mut song.channels[1];
    let id = ch.id;
    plugins::add(&mut ch.rack, id, &uid("Poly Mod Synth", "CLAP"), 0).unwrap();
    plugins::add(&mut ch.rack, id, &uid("Gain", "VST3"), 1).unwrap();
    assert_eq!(ch.rack.modules[1].type_id, PLUGIN_EFFECT);
    let clip = ch.lanes[0].add_clip(0, 4.0);
    clip.events.push(Event::Note {
        pos: 0,
        len_beats: 2.0,
        key: 60,
        velocity: 204,
        pan: 127,
        slide: false,
    });
    plugins::sync(&song);
    let loud = rendered_energy(&song);
    assert!(loud > 1.0, "the synth plays: energy {loud}");
    // The Gain's own param at its lowest (−30 dB, which it glides to over its first 50 ms): far quieter.
    let gain = 1 + param(&song, 1, 1, "Gain");
    song.channels[1].rack.modules[1].params[gain] = 0.0;
    let quiet = rendered_energy(&song);
    assert!(quiet < loud * 0.01 && quiet > 0.0, "{quiet} against {loud}");
}

#[test]
fn a_saved_song_keeps_its_plugin_and_restores_its_state() {
    let Some(dir) = test_plugins() else { return };
    let uid = installed(&dir);
    let mut song = Song::default();
    song.add_channel(ChannelKind::Instrument);
    let ch = &mut song.channels[1];
    let id = ch.id;
    ch.rack.add(9);
    plugins::add(&mut ch.rack, id, &uid("Gain", "CLAP"), 0).unwrap();
    let gain = 1 + param(&song, 1, 0, "Gain");
    // A knob move in the song reaches the plugin, and its state follows at the save.
    song.channels[1].rack.modules[0].params[gain] = 0.25;
    plugins::sync(&song);
    plugins::store_states(&mut song);
    let bytes = honeydew::flm::write(&song).unwrap();
    // The plugin module is no `RMOd` of the rack, which a reader that does not know its type would fail on, but one
    // inside an `HDPm`, which such a reader skips.
    let rmods: Vec<usize> = bytes
        .windows(4)
        .enumerate()
        .filter(|(_, w)| w == b"RMOd")
        .map(|(i, _)| i)
        .collect();
    let plugin_rmods: Vec<usize> = (rmods.iter().copied())
        .filter(|&i| {
            u32::from_le_bytes(bytes[i + 8..i + 12].try_into().unwrap()) == u32::from(PLUGIN_EFFECT)
        })
        .collect();
    assert_eq!(plugin_rmods.len(), 1);
    assert_eq!(&bytes[plugin_rmods[0] - 16..plugin_rmods[0] - 12], b"HDPm");
    let mut back = honeydew::flm::read(&bytes).unwrap();
    assert_eq!(back, song);
    let state = |song: &Song| {
        song.channels[1].rack.modules[0]
            .file
            .plugin
            .as_ref()
            .unwrap()
            .state
            .clone()
    };
    assert_eq!(state(&back), state(&song));
    // The app opened again: the plugin comes back from its saved state alone.
    plugins::song_replaced();
    let module = &mut back.channels[1].rack.modules[0];
    module.params.truncate(1);
    module.file.plugin.as_mut().unwrap().params.clear();
    plugins::sync(&back);
    assert!(plugins::adopt_params(&mut back));
    let restored = back.channels[1].rack.modules[0].params[gain];
    assert!((restored - 0.25).abs() < 1e-3, "restored {restored}");
    // FL Studio desktop: an instrument plugin goes out as a wrapper channel with its state, and comes back as itself.
    let ch = &mut back.channels[1];
    let id = ch.id;
    let synth = plugins::add(&mut ch.rack, id, &uid("Poly Mod Synth", "VST3"), 0).unwrap();
    plugins::store_states(&mut back);
    let flp = honeydew::flp::export::export(&back, &SampleCache::default(), &[]).flp;
    let factory = honeydew::content::Factory::new(Vec::new());
    let imported = honeydew::flp::import::import(&flp, &factory).unwrap().song;
    let modules = || imported.channels.iter().flat_map(|c| &c.rack.modules);
    let came = (modules().find(|m| m.type_id == PLUGIN_INSTRUMENT)).expect("the synth came back");
    let (came, sent) = (
        came.file.plugin.as_deref().unwrap(),
        back.channels[1].rack.get(synth).unwrap(),
    );
    let sent = sent.file.plugin.as_deref().unwrap();
    assert_eq!(
        (&came.format, &came.id, &came.name),
        (&sent.format, &sent.id, &sent.name)
    );
    assert_eq!(came.state, sent.state);
}
