use std::path::PathBuf;
use std::sync::Arc;
use std::sync::atomic::{AtomicUsize, Ordering};

use super::*;
use crate::ui::main_menu::MemoryLibrary;

fn at(rel: &str) -> PathBuf {
    Path::new(ROOT).join(rel)
}

/// A fake factory of `rels`, each file's bytes its own path.
fn fake(rels: &[&'static str]) -> Factory {
    Factory::new(
        rels.iter()
            .map(|&rel| Entry::new(rel, move || rel.into()))
            .collect(),
    )
}

/// The rows of `folder` as (name, is folder), in the order listed.
fn rows(lib: &dyn SongLibrary, folder: Option<&Path>) -> Vec<(String, bool)> {
    let listed = lib.list(folder).unwrap();
    listed.into_iter().map(|e| (e.name, e.is_folder)).collect()
}

fn row(name: &str, folder: bool) -> (String, bool) {
    (name.into(), folder)
}

#[test]
fn the_folders_come_from_the_entry_paths_and_files_list_in_index_order() {
    let factory = fake(&[
        "Presets/MiniSynth/Pad/Wide.flmpst",
        "Presets/MiniSynth/b Soft.flmpst",
        "Presets/MiniSynth/Bass/Round.flmpst",
        "Presets/MiniSynth/Bright.flmpst",
        "Presets/MiniSynth/Bass/Deep.flmpst",
        "Drumsets/Dust Kit.tmpl",
    ]);
    assert_eq!(
        rows(&factory, None),
        [row(DRUMSETS, true), row(PRESETS, true)]
    );
    assert_eq!(rows(&factory, Some(Path::new(ROOT))), rows(&factory, None));
    // Bytewise: `Bright` before `b Soft`, where A-Z would put it after.
    let synth = at("Presets/MiniSynth");
    assert_eq!(
        rows(&factory, Some(&synth)),
        [
            row("Bass", true),
            row("Bright.flmpst", false),
            row("Pad", true),
            row("b Soft.flmpst", false),
        ]
    );
    let listed = factory.list(Some(&synth)).unwrap();
    assert_eq!(listed[0].path, at("Presets/MiniSynth/Bass"));
    assert_eq!(listed[1].path, at("Presets/MiniSynth/Bright.flmpst"));
    assert!(listed.iter().all(|e| e.recency == 0));
    assert_eq!(
        rows(&factory, Some(&at("Presets/MiniSynth/Bass"))),
        [row("Deep.flmpst", false), row("Round.flmpst", false)]
    );
    // A folder that is not there, or a path outside the factory, is not found.
    for missing in [at("Presets/Gone"), PathBuf::from("My Presets"), at("../x")] {
        let err = factory.list(Some(&missing)).unwrap_err();
        assert_eq!(err.kind(), io::ErrorKind::NotFound, "{missing:?}");
    }
    assert!(factory.exists(&synth) && factory.exists(&at("Drumsets/Dust Kit.tmpl")));
    assert!(!factory.exists(&at("Drumsets/Dust")) && !factory.exists(Path::new("Drumsets")));
    assert!(factory.read_only(&synth));
}

#[test]
fn a_read_runs_the_generator_and_a_missing_path_is_not_found() {
    let runs = Arc::new(AtomicUsize::new(0));
    let counted = runs.clone();
    let mut factory = Factory::new(vec![Entry::new("Loops/Beat.wav", move || {
        counted.fetch_add(1, Ordering::Relaxed);
        b"RIFF".to_vec()
    })]);
    assert_eq!(runs.load(Ordering::Relaxed), 0, "listing never runs one");
    factory.list(Some(&at(LOOPS))).unwrap();
    assert_eq!(runs.load(Ordering::Relaxed), 0, "listing never runs one");
    assert_eq!(factory.read(&at("Loops/Beat.wav")).unwrap(), b"RIFF");
    factory.read(&at("Loops/Beat.wav")).unwrap();
    assert_eq!(runs.load(Ordering::Relaxed), 2);
    for missing in [
        at("Loops/Gone.wav"),
        at(LOOPS),
        PathBuf::from("Loops/Beat.wav"),
    ] {
        let err = factory.read(&missing).unwrap_err();
        assert_eq!(err.kind(), io::ErrorKind::NotFound, "{missing:?}");
    }
    let err = factory.store("Song.flm", b"x").unwrap_err();
    assert_eq!(err.kind(), io::ErrorKind::Unsupported);
}

/// A user library holding `paths` (each file's bytes `user`), the factory beside it.
fn user_lib(paths: &[&str]) -> MemoryLibrary {
    let mut lib = MemoryLibrary::default();
    for path in paths {
        lib.put(PathBuf::from(path), b"user").unwrap();
    }
    lib
}

#[test]
fn the_union_sends_factory_paths_to_the_factory_and_the_rest_to_the_user() {
    let factory = fake(&["Presets/Reverb 2/Hall.flmpst"]);
    // On the desktop the user folder is absolute; on the web the root is the empty path.
    for dir in ["/home/u/.local/share/honeydew", ""] {
        let preset = Path::new(dir).join("My Presets/Reverb 2/Mine.flmpst");
        let mut user = user_lib(&[preset.to_str().unwrap()]);
        let mut union = Union {
            user: &mut user,
            factory: &factory,
        };
        let theirs = at("Presets/Reverb 2");
        assert_eq!(rows(&union, Some(&theirs)), [row("Hall.flmpst", false)]);
        assert_eq!(
            union.read(&theirs.join("Hall.flmpst")).unwrap(),
            b"Presets/Reverb 2/Hall.flmpst"
        );
        let mine = preset.parent().unwrap();
        assert_eq!(rows(&union, Some(mine)), [row("Mine.flmpst", false)]);
        assert_eq!(union.read(&preset).unwrap(), b"user");
        assert!(union.exists(&theirs) && union.exists(&preset));
        assert!(union.read_only(&theirs) && !union.read_only(&preset));
        // The factory takes no write; the user's files still do.
        let hall = theirs.join("Hall.flmpst");
        for refused in [
            union.make_folder(&theirs.join("New")),
            union.remove(&hall),
            union.move_to(&hall, &mine.join("Hall.flmpst")),
            union.copy_to(&hall, &mine.join("Hall.flmpst")),
            union.move_to(&preset, &theirs.join("Mine.flmpst")),
            union.store_in(&theirs, "Song.flm", b"x"),
        ] {
            assert_eq!(refused.unwrap_err().kind(), io::ErrorKind::PermissionDenied);
        }
        assert_eq!(union.bin_path(&hall), None);
        union.make_folder(&mine.join("New")).unwrap();
        union.copy_to(&preset, &mine.join("Copy.flmpst")).unwrap();
        assert!(union.exists(&mine.join("Copy.flmpst")) && union.factory.exists(&hall));
        assert_eq!(user.read(&mine.join("Copy.flmpst")).unwrap(), b"user");
    }
}

/// The union's places are the user library's: SONGS' `Unzipped songs` beside its bin stays a place (never selected,
/// deleted or renamed) when the factory sits beside it.
#[cfg(not(target_arch = "wasm32"))]
#[test]
fn the_union_has_the_user_librarys_places() {
    let factory = fake(&["Presets/Reverb 2/Hall.flmpst"]);
    let mut user = crate::ui::main_menu::NativeLibrary::at("/home/u/data/My Songs".into());
    let places = user.places();
    assert_eq!(places.len(), 2, "the bin and Unzipped songs");
    let union = Union {
        user: &mut user,
        factory: &factory,
    };
    assert_eq!(union.places(), places);
    assert!(union.shadowed(Path::new("/home/u/data/My Songs/Unzipped songs")));
}

#[test]
fn a_factory_file_is_stored_as_an_app_shorthand_below_built_in() {
    assert!(is_factory(&at("Loops/X.wav")) && is_factory(Path::new(ROOT)));
    assert!(!is_factory(Path::new("Loops/X.wav")));
    assert!(!is_factory(Path::new("/home/u/<built-in>/X.wav")));
    assert_eq!(
        content_path(&at("Loops/Dust Break 84.wav")),
        Some(ContentPath::Shorthand {
            root: ContentRoot::App,
            relative: "Built-in/Loops/Dust Break 84.wav".into(),
        })
    );
    assert_eq!(content_path(Path::new(ROOT)), None);
    assert_eq!(content_path(Path::new("My Samples/X.wav")), None);
}

/// What every lane's entries must hold: plain, unique `/`-separated paths below a top folder of ours.
#[test]
fn every_factory_entry_has_a_plain_unique_path_in_a_top_folder() {
    let tops = [PRESETS, DRUMSETS, LOOPS, SHAPES, CHORD_SETS, TEMPLATES];
    let entries = &factory().entries;
    for (i, entry) in entries.iter().enumerate() {
        let rel = &entry.rel;
        let parts: Vec<&str> = rel.split('/').collect();
        assert!(tops.contains(&parts[0]) && parts.len() > 1, "{rel}");
        let plain =
            |p: &&str| !p.is_empty() && !p.starts_with('.') && !p.contains(['\\', '<', '>']);
        assert!(parts.iter().all(plain), "{rel}");
        assert!(entries[..i].iter().all(|e| e.rel != *rel), "{rel} twice");
    }
}
