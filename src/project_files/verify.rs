//! The opt-in check of a real Original Content folder: walks the tree and loads every file through the app's own
//! loaders, then prints counts per top folder (ok / failed / unsupported), each song's and template's samples
//! (referenced / resolved), and our own error messages with each file's relative path. It prints no file contents.
//!
//! It runs only when `HONEYDEW_ORIGINAL_CONTENT` (or the former `STUDIO_ORIGINAL_CONTENT`) names a content folder, as
//! an ignored test:
//!
//! ```text
//! HONEYDEW_ORIGINAL_CONTENT="$HOME/Documents/Honeydew/Original Content" \
//!     cargo test --lib verify_original_content -- --ignored --nocapture
//! ```
//!
//! Its own tests run it on the fake tree only ([`super::content::tests::fake_tree`]).

use std::collections::BTreeMap;
use std::fmt;
use std::path::{Path, PathBuf};

use super::content::KEYBOARD_SHORTCUTS;
use super::{CONTENT_FOLDER, ProjectFiles, import_fst, load_pick, read_spectra};
use crate::audio_engine::SongAudioEngine;
use crate::sample_cache::{SampleCache, SampleKey, reader};
use crate::song::{DrumKit, ModuleInstance, Song};
use honeydew_audio::device::AudioRenderer;

/// The most files walked, and the failures listed.
const MAX_FILES: usize = 100_000;
const MAX_FAILURES: usize = 500;
/// Seconds of each song and template rendered.
const RENDER_SECONDS: usize = 2;
const RATE: f32 = 44_100.0;

/// How one file went.
enum Outcome {
    Ok,
    Failed(String),
    /// No loader in the app for this type yet: why.
    Unsupported(&'static str),
}

/// What the walk found.
#[derive(Default)]
pub(crate) struct Report {
    /// Per top folder: ok, failed, unsupported.
    pub(crate) folders: BTreeMap<String, [usize; 3]>,
    /// `relative path: message`, in walk order.
    pub(crate) failures: Vec<String>,
    /// Per reason: how many files.
    pub(crate) unsupported: BTreeMap<&'static str, usize>,
    /// Per song and template played: `relative path: n referenced, m resolved` (its samples, DW programs and
    /// SoundFonts), in walk order.
    pub(crate) songs: Vec<String>,
}

impl fmt::Display for Report {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        writeln!(
            f,
            "{:<32} {:>6} {:>6} {:>11}",
            "folder", "ok", "failed", "unsupported"
        )?;
        for (folder, [ok, failed, unsupported]) in &self.folders {
            writeln!(f, "{folder:<32} {ok:>6} {failed:>6} {unsupported:>11}")?;
        }
        for line in &self.songs {
            writeln!(f, "song: {line}")?;
        }
        for (why, n) in &self.unsupported {
            writeln!(f, "unsupported: {n} × {why}")?;
        }
        for line in self.failures.iter().take(MAX_FAILURES) {
            writeln!(f, "failed: {line}")?;
        }
        if self.failures.len() > MAX_FAILURES {
            writeln!(
                f,
                "failed: … and {} more",
                self.failures.len() - MAX_FAILURES
            )?;
        }
        Ok(())
    }
}

/// Checks every file under `content`, a content folder.
pub(crate) fn verify(content: &Path) -> Report {
    let (files, _link) = files_over(content);
    let mut report = Report::default();
    let mut paths = Vec::new();
    walk(content, &mut paths);
    paths.sort();
    for path in paths.iter().take(MAX_FILES) {
        let relative = path.strip_prefix(content).unwrap_or(path);
        let top = relative.components().next().map_or_else(String::new, |c| {
            c.as_os_str().to_string_lossy().into_owned()
        });
        let top = if relative.components().count() == 1 {
            "(top)".to_owned()
        } else {
            top
        };
        let counts = report.folders.entry(top).or_default();
        let mut samples = None;
        match check(&files, path, relative, &mut samples) {
            Outcome::Ok => counts[0] += 1,
            Outcome::Failed(why) => {
                counts[1] += 1;
                report
                    .failures
                    .push(format!("{}: {why}", relative.display()));
            }
            Outcome::Unsupported(why) => {
                counts[2] += 1;
                *report.unsupported.entry(why).or_default() += 1;
            }
        }
        if let Some((referenced, resolved)) = samples {
            let line = format!("{referenced} referenced, {resolved} resolved");
            report.songs.push(format!("{}: {line}", relative.display()));
        }
    }
    report
}

/// Project files whose content folder is `content`: its parent when it is named [`CONTENT_FOLDER`], else a temp user
/// folder holding a link to it (kept until the second value drops).
fn files_over(content: &Path) -> (ProjectFiles, Option<Unlink>) {
    if content.file_name() == Some(CONTENT_FOLDER.as_ref())
        && let Some(user) = content.parent()
    {
        return (ProjectFiles::on_disk(user), None);
    }
    let user = std::env::temp_dir().join(format!("honeydew-verify-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&user);
    std::fs::create_dir_all(&user).expect("a temp folder");
    #[cfg(unix)]
    std::os::unix::fs::symlink(content, user.join(CONTENT_FOLDER)).expect("a link");
    (ProjectFiles::on_disk(&user), Some(Unlink(user)))
}

/// Removes the temp user folder.
struct Unlink(PathBuf);

impl Drop for Unlink {
    fn drop(&mut self) {
        let _ = std::fs::remove_dir_all(&self.0);
    }
}

/// Every file under `dir`, no link followed.
fn walk(dir: &Path, out: &mut Vec<PathBuf>) {
    for entry in std::fs::read_dir(dir).into_iter().flatten().flatten() {
        let Ok(kind) = entry.file_type() else {
            continue;
        };
        if kind.is_dir() {
            walk(&entry.path(), out);
        } else if kind.is_file() && out.len() < MAX_FILES {
            out.push(entry.path());
        }
    }
}

/// Loads `path` (`relative` in the content folder) the way the app does for its type. A song or template sets
/// `samples` to how many samples it references, and how many of them resolved.
fn check(
    files: &ProjectFiles,
    path: &Path,
    relative: &Path,
    samples: &mut Option<(usize, usize)>,
) -> Outcome {
    let ext = path
        .extension()
        .map(|e| e.to_string_lossy().to_lowercase())
        .unwrap_or_default();
    let bytes = match files.read_user_file(path) {
        Ok(bytes) => bytes,
        Err(err) => return Outcome::Failed(err),
    };
    let failed = |e: &dyn fmt::Display| Outcome::Failed(e.to_string());
    match ext.as_str() {
        "wav" | "flac" | "mp3" | "ogg" => match honeydew_audio::sample::decode(&bytes) {
            Ok(_) => Outcome::Ok,
            Err(e) => Outcome::Failed(format!("{e:?}")),
        },
        "tmpl" => match crate::flm::read_drumset(&bytes) {
            Ok(kit) => kit_samples(files, kit),
            Err(e) => failed(&e),
        },
        "sf2" => match crate::sf2::presets(&bytes) {
            Ok(_) if honeydew_audio::soundfont::SoundFont::parse(bytes).is_ok() => Outcome::Ok,
            Ok(_) => Outcome::Failed("the SoundFont's samples do not parse".into()),
            Err(e) => Outcome::Failed(format!("{e:?}")),
        },
        "flmpst" => match crate::flm::read_preset(&bytes, None) {
            Ok(_) => Outcome::Ok,
            Err(e) => failed(&e),
        },
        "flms" | "flgsynth" => {
            let mut module = ModuleInstance::new(if ext == "flms" { 0 } else { 2 }, 2);
            match load_pick(&mut module, path, &bytes, None) {
                Ok(()) => Outcome::Ok,
                Err(e) => Outcome::Failed(e),
            }
        }
        "flmrack" => match crate::flm::read_rack(&bytes, &mut 1) {
            Ok(_) => Outcome::Ok,
            Err(e) => failed(&e),
        },
        "spectra" => match read_spectra(&bytes) {
            Some(_) => Outcome::Ok,
            None => Outcome::Failed(format!("{} bytes, not 4096", bytes.len())),
        },
        "flm" | "songtmpl" => match crate::flm::read(&bytes) {
            Ok(song) => play(files, &song, samples),
            Err(e) => failed(&e),
        },
        "dwp" => program(files, path),
        "fst" => fst(files, path, &bytes),
        "chords" => match std::str::from_utf8(&bytes).map_err(|e| e.to_string()) {
            Ok(text) => match crate::song::chords::ChordSet::parse(text) {
                Ok(_) => Outcome::Ok,
                Err(e) => Outcome::Failed(e),
            },
            Err(e) => Outcome::Failed(e),
        },
        "json" if relative.parent() == Some(KEYBOARD_SHORTCUTS.as_ref()) => shortcuts(path),
        _ => Outcome::Unsupported("other file types (art, video, data tables, notes)"),
    }
}

/// A shortcut file: the keymap loads (on Windows and on a Mac, [`crate::keymap::load`]) merge it and log nothing about
/// it but its skipped gamepad entries. A file they do not read is unsupported.
fn shortcuts(path: &Path) -> Outcome {
    use eframe::egui::os::OperatingSystem;
    let name = path.file_name().unwrap_or_default().to_string_lossy();
    if !["windows.json", "mac.json", "gamepad.json"].contains(&&*name) {
        return Outcome::Unsupported(".json: a shortcut file the keymap loads do not read");
    }
    let head = format!("{name}: ");
    let mut lines: Vec<String> = [OperatingSystem::Windows, OperatingSystem::Mac]
        .into_iter()
        .flat_map(|os| crate::keymap::load(os, path.parent(), None).1)
        .filter_map(|line| Some(line.strip_prefix(&head)?.to_owned()))
        .filter(|line| !line.contains("gamepad buttons skipped"))
        .collect();
    // The gamepad file is read on both.
    lines.sort_unstable();
    lines.dedup();
    match lines.is_empty() {
        true => Outcome::Ok,
        false => Outcome::Failed(lines.join("; ")),
    }
}

/// A 3x Osc `.fst`: the import reads to the end, and a custom waveform it names is in the content folder (the paths
/// outside `/Instruments/Shapes/` name none).
fn fst(files: &ProjectFiles, path: &Path, bytes: &[u8]) -> Outcome {
    let import = import_fst(bytes, path);
    match import.custom_shape {
        _ if !import.ok => Outcome::Failed("the .fst import stops short".into()),
        Some(shape) if files.resolve(&shape).is_none() => {
            Outcome::Failed(format!("its custom waveform is missing: {shape:?}"))
        }
        _ => Outcome::Ok,
    }
}

/// A DW program (`.dwp`): picked into a DW Sampler, it loads with all its zones' samples.
fn program(files: &ProjectFiles, path: &Path) -> Outcome {
    let Some(shorthand) = files.shorthand(path).or_else(|| linked(files, path)) else {
        return Outcome::Failed("outside the content folder".into());
    };
    let mut song = Song::default();
    song.add_channel(crate::song::ChannelKind::Synth(
        crate::song::SynthKind::DwSampler,
    ));
    let module = &mut song.channels[1].rack.modules[0];
    if let Err(e) = load_pick(module, path, &[], Some(shorthand.clone())) {
        return Outcome::Failed(e);
    }
    let mut cache = SampleCache::default();
    cache.load(&song, &reader(files));
    match cache.program(&shorthand) {
        Some(zones) if zones.is_empty() => Outcome::Failed("a program with no zones".into()),
        Some(_) => missing(&cache, "zone samples"),
        None => Outcome::Failed("not a DW program".into()),
    }
}

/// The `@R@` path of `path` under a content folder the check reached through its link ([`files_over`]).
fn linked(files: &ProjectFiles, path: &Path) -> Option<crate::flm::chunk::ContentPath> {
    let real = files.content_dir()?.canonicalize().ok()?;
    let relative = path
        .canonicalize()
        .ok()?
        .strip_prefix(real)
        .ok()?
        .to_owned();
    let parts: Vec<_> = relative.iter().map(|p| p.to_string_lossy()).collect();
    Some(crate::flm::chunk::ContentPath::Shorthand {
        root: crate::flm::chunk::ContentRoot::App,
        relative: parts.join("/"),
    })
}

/// A kit's pads: each sample it names loads.
fn kit_samples(files: &ProjectFiles, kit: DrumKit) -> Outcome {
    let mut song = Song::default();
    song.add_channel(crate::song::ChannelKind::Drums);
    let pads = kit.slots.len();
    song.channels[1].drum_kit = Some(kit);
    let mut cache = SampleCache::default();
    cache.load(&song, &reader(files));
    missing(&cache, &format!("of {pads} pads"))
}

/// The song's samples load, an engine builds, and it renders [`RENDER_SECONDS`] of finite audio. `samples`: how many
/// samples the song references, and how many resolved.
fn play(files: &ProjectFiles, song: &Song, samples: &mut Option<(usize, usize)>) -> Outcome {
    let mut cache = SampleCache::default();
    cache.load(song, &reader(files));
    *samples = Some((cache.met(), cache.met() - cache.missing().count()));
    if let Outcome::Failed(why) = missing(&cache, "samples") {
        return Outcome::Failed(why);
    }
    let mut engine = match SongAudioEngine::with_samples(song, RATE, &cache) {
        Ok(engine) => engine,
        Err(e) => return Outcome::Failed(format!("engine: {e:?}")),
    };
    engine.play();
    let mut block = vec![0.0; 2 * 1024];
    for _ in 0..(RENDER_SECONDS * RATE as usize).div_ceil(1024) {
        engine.render(&mut block, 2);
        if !block.iter().all(|s| s.is_finite()) {
            return Outcome::Failed("the render is not finite".into());
        }
    }
    Outcome::Ok
}

/// Failed when `cache` misses anything: how many, and the content paths (the first few).
fn missing(cache: &SampleCache, what: &str) -> Outcome {
    let missing: Vec<String> = (cache.missing())
        .map(|key| match key {
            SampleKey::File(path) => format!("{path:?}"),
            other => format!("{other:?}"),
        })
        .collect();
    match missing.len() {
        0 => Outcome::Ok,
        n => Outcome::Failed(format!(
            "{n} {what} missing: {}",
            missing[..n.min(3)].join(", ")
        )),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::project_files::content::tests::{Temp, app, fake_tree, put};

    #[test]
    fn the_check_loads_every_file_of_the_fake_tree_and_names_what_fails() {
        let tmp = Temp::new("verify");
        let content = tmp.0.join(CONTENT_FOLDER);
        fake_tree(&content);
        put(
            &content.join("Drum Samples/Kick/Broken.wav"),
            b"RIFF0000WAVE",
        );
        put(&content.join("README.txt"), b"x");
        // `.fst` cut short, and one whose custom shape is not in the folder.
        use crate::project_files::fst::tests::{fst, shape};
        put(&content.join("Synth Presets2/3xOsc/Lead/Cut.fst"), b"FLhd");
        put(
            &content.join("Synth Presets2/3xOsc/Lead/Gone.fst"),
            &fst(&[shape(r"\Instruments\Shapes\Gone.wav")]),
        );
        // A chord set short of its eight chords. Shortcut files: an entry with an unknown key fails; the gamepad's
        // skipped entries do not.
        put(
            &content.join("Chord Sets/Short.chords"),
            br#"{ "chords": [], "rootNote": 0 }"#,
        );
        put(
            &content.join("Keyboard Shortcuts/mac.json"),
            br#"{ "modifiers": ["cmd"], "actions": [{ "key": "cmd+nokey", "action": "undo" }] }"#,
        );
        put(
            &content.join("Keyboard Shortcuts/gamepad.json"),
            br#"{ "modifiers": ["gamepad_lb"], "actions": [{ "key": "gamepad_a", "action": "play" }] }"#,
        );
        put(
            &content.join("Keyboard Shortcuts/windows.json"),
            br#"{ "modifiers": ["ctrl"], "actions": [{ "key": "ctrl+nokey", "action": "undo" }] }"#,
        );
        // A shortcut file's name below the top folder is no shortcut file.
        put(&content.join("Songs/Keyboard Shortcuts/windows.json"), b"{");
        // A program whose zone sample is not there (its zone names `Gen Guitar\Gen Zone.wav`, which it looks for in
        // `Gen Lost/`), and a demo song playing it and, in another DW Sampler, a plain audio file.
        let guitar =
            std::fs::read(content.join("DirectWave Samples/Guitar/Gen Guitar.dwp")).unwrap();
        put(
            &content.join("DirectWave Samples/Guitar/Gen Lost.dwp"),
            &guitar,
        );
        let mut song = Song::default();
        for program in [
            "DirectWave Samples/Guitar/Gen Lost.dwp",
            "Drum Samples/Kick/Gen Kick.wav",
        ] {
            let dw = crate::song::ChannelKind::Synth(crate::song::SynthKind::DwSampler);
            song.add_channel(dw);
            let module = &mut song.channels.last_mut().unwrap().rack.modules[0];
            load_pick(module, Path::new(program), &[], Some(app(program))).unwrap();
        }
        put(
            &content.join("Songs/Demo Songs/Gen Lost.flm"),
            &crate::flm::write(&song).unwrap(),
        );
        let report = verify(&content);
        let shown = report.to_string();
        println!("{shown}");
        let row = |folder: &str| report.folders.get(folder).copied();
        assert_eq!(row("Drum Samples"), Some([2, 1, 0]), "{shown}");
        assert_eq!(row("Drum Templates"), Some([1, 0, 0]), "{shown}");
        // A program and its zone sample; a program embedding its zone.
        assert_eq!(row("DirectWave Samples"), Some([2, 1, 0]), "{shown}");
        assert_eq!(row("DirectWave Samples EXP"), Some([1, 0, 0]), "{shown}");
        assert_eq!(row("Synth Presets2"), Some([5, 2, 0]), "{shown}");
        assert_eq!(row("New Song Templates"), Some([2, 0, 0]), "{shown}");
        assert_eq!(row("Songs"), Some([1, 1, 1]), "{shown}");
        assert_eq!(row("GMSynthWaveshapes"), Some([1, 0, 0]), "{shown}");
        for folder in [
            "SoundFonts",
            "Rack Presets",
            "Effect Presets",
            "3xOsc Shapes",
            "Loop Samples",
        ] {
            assert_eq!(row(folder), Some([1, 0, 0]), "{folder}: {shown}");
        }
        assert_eq!(row("Chord Sets"), Some([1, 1, 0]), "{shown}");
        assert_eq!(row("Keyboard Shortcuts"), Some([1, 2, 0]), "{shown}");
        // The README.
        assert_eq!(row("(top)"), Some([0, 0, 1]), "{shown}");
        assert_eq!(report.failures.len(), 8, "{shown}");
        let failed = |i: usize, start: &str| {
            assert!(report.failures[i].starts_with(start), "{i}: {shown}");
        };
        failed(0, "Chord Sets/Short.chords: A chord set needs 8 chords");
        failed(
            1,
            "DirectWave Samples/Guitar/Gen Lost.dwp: 1 zone samples missing",
        );
        failed(2, "Drum Samples/Kick/Broken.wav: ");
        failed(
            3,
            "Keyboard Shortcuts/mac.json: `cmd+nokey`: unknown key token",
        );
        failed(
            4,
            "Keyboard Shortcuts/windows.json: `ctrl+nokey`: unknown key token",
        );
        failed(5, "Songs/Demo Songs/Gen Lost.flm: 1 samples missing");
        failed(6, "Synth Presets2/3xOsc/Lead/Cut.fst: ");
        assert!(
            report.failures[7].contains("3xOsc Shapes/Gone.wav"),
            "{shown}"
        );
        // Each song and template played, with the samples it references and how many resolved, failing or not.
        assert_eq!(
            report.songs,
            [
                "New Song Templates/Gen Ambient.songtmpl: 2 referenced, 2 resolved",
                "New Song Templates/Gen Beat.songtmpl: 2 referenced, 2 resolved",
                "Songs/Demo Songs/Gen Demo.flm: 2 referenced, 2 resolved",
                // The program and its missing zone; the audio file, a sample and a program of one zone, once.
                "Songs/Demo Songs/Gen Lost.flm: 3 referenced, 2 resolved",
            ],
            "{shown}"
        );
        assert!(
            shown.contains("song: Songs/Demo Songs/Gen Demo.flm: 2 referenced, 2 resolved\n"),
            "{shown}"
        );
        // Through a link to a folder of another name too.
        let elsewhere = Temp::new("verify-elsewhere");
        fake_tree(&elsewhere.0.join("tree"));
        let linked = verify(&elsewhere.0.join("tree"));
        assert_eq!(linked.folders.get("Songs").copied(), Some([1, 0, 0]));
        assert!(linked.failures.is_empty(), "{linked}");
    }

    /// The opt-in check of a real content folder (see the module docs). Never run on a content folder in CI.
    #[test]
    #[ignore = "opt-in: set HONEYDEW_ORIGINAL_CONTENT to a content folder"]
    fn verify_original_content() {
        let var = |key| std::env::var_os(key);
        let Some(dir) = var("HONEYDEW_ORIGINAL_CONTENT").or_else(|| var("STUDIO_ORIGINAL_CONTENT"))
        else {
            eprintln!("HONEYDEW_ORIGINAL_CONTENT is not set: nothing to check");
            return;
        };
        let report = verify(Path::new(&dir));
        println!("{report}");
    }
}
