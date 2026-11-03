use std::path::{Path, PathBuf};

use eframe::egui::Context;

use super::*;
use crate::project_files::mem::MemFiles;
use crate::project_files::{OVERWRITE_TAG, SAVE_FAILED};
use crate::song::Song;
use crate::ui::views::{Answer, Overlay};

fn distortion() -> ModuleInstance {
    let mut module = ModuleInstance::new(6, 3);
    module.params[1] = 0.9;
    module
}

fn lead() -> PresetSource {
    PresetSource::Rack {
        name: "Lead 2".into(),
        modules: vec![ModuleInstance::new(51, 2), distortion()],
    }
}

/// The web's files: a download under the file name, a copy kept in the session.
fn web() -> (ProjectFiles, MemFiles) {
    let mem = MemFiles::default();
    mem.0.borrow_mut().names_only = true;
    (ProjectFiles::with_provider(Box::new(mem.clone())), mem)
}

/// Asks for `source`'s dialog and saves it as `name`; returns the name the dialog proposed.
fn save(files: &mut ProjectFiles, source: PresetSource, name: &str) -> String {
    files.ask_preset(source, 1);
    let Some(Overlay::PresetDialog { name: proposed, .. }) = files.take_overlay() else {
        panic!("no preset dialog");
    };
    assert!(files.save_preset(name), "the dialog stays open");
    assert_eq!(files.poll(&mut Song::default()), None);
    proposed
}

#[test]
fn the_dialogs_are_titled_as_the_spec_names_them() {
    assert_eq!(PresetKind::Module.title(), "Save preset");
    assert_eq!(PresetKind::Rack.title(), "Save Rack preset");
    assert_eq!(
        PresetSource::Module(distortion()).kind(),
        PresetKind::Module
    );
    assert_eq!(lead().kind(), PresetKind::Rack);
}

/// The `Effect Presets` and `Synth Presets2` folders, spelled out here, not taken from `presets.rs`'s own table (nor
/// The display names).
const SPEC_FOLDERS: [(u16, &str); 31] = [
    (3, "Equalizer"),
    (4, "Multi FX"),
    (5, "Limiter"),
    (6, "Distortion"),
    (7, "Graphic EQ"),
    (8, "Compressor"),
    (9, "Filter"),
    (10, "Autoduck"),
    (11, "Chorus"),
    (12, "Reverb"),
    (13, "Flanger"),
    (14, "Phaser"),
    (15, "Stereoizer"),
    (16, "Trance Delay"),
    (18, "Tape Delay"),
    (20, "Leveler"),
    (28, "Limiter 2"),
    (29, "Autoduck2"),
    (34, "Spacer"),
    (35, "Reverb 2"),
    (36, "MBand Comp"),
    (40, "Comb Filter"),
    (41, "Waveshaper"),
    (43, "Wow & Flutter"),
    (46, "Tremolo"),
    (51, "Arpeggiator"),
    (39, "3xOsc"),
    (2, "GMSynth"),
    (0, "MiniSynth"),
    (23, "SuperSaw"),
    (19, "TransistorBass"),
];

#[test]
fn a_module_folder_is_its_factory_folder_else_its_display_name() {
    // The folders whose display names collide or differ.
    assert_eq!(preset_folder(5), Some("Limiter"));
    assert_eq!(preset_folder(10), Some("Autoduck"));
    assert_eq!(preset_folder(12), Some("Reverb"));
    assert_eq!(preset_folder(4), Some("Multi FX"));
    assert_eq!(preset_folder(20), Some("Leveler"));
    assert_eq!(preset_folder(28), Some("Limiter 2"));
    assert_eq!(preset_folder(29), Some("Autoduck2"));
    assert_eq!(preset_folder(36), Some("MBand Comp"));
    assert_eq!(preset_folder(19), Some("TransistorBass"));
    assert_eq!(preset_folder(39), Some("3xOsc"));
    for (type_id, folder) in SPEC_FOLDERS {
        assert_eq!(preset_folder(type_id), Some(folder), "{type_id}");
    }
    // No factory folder: the display name.
    for (type_id, folder) in [(24, "Tuner"), (1, "DW Sampler"), (47, "Note Echo")] {
        assert_eq!(preset_folder(type_id), Some(folder), "{type_id}");
    }
    assert_eq!(preset_folder(17), None);
    // Reverb (32) has no folder and loads the legacy Reverb's presets: it shares `Reverb`. Every other
    // module has its own folder.
    assert_eq!(preset_folder(32), Some("Reverb"));
    let mut folders: Vec<_> = (0..=u16::from(u8::MAX))
        .filter(|&t| t != 32)
        .filter_map(preset_folder)
        .collect();
    let registered = folders.len();
    folders.sort();
    folders.dedup();
    assert_eq!(folders.len(), registered);
}

#[test]
fn a_module_preset_goes_into_my_presets_under_its_module_and_is_listed_and_read_back() {
    let (mut files, mem) = web();
    let mut song = Song::default();
    files.ask_preset(PresetSource::Module(distortion()), 1);
    assert_eq!(
        files.take_overlay(),
        Some(Overlay::PresetDialog {
            name: "Distortion".into(),
            kind: PresetKind::Module,
        })
    );
    // The typed name may end in the extension, as a song's may end in `.flm`.
    assert!(files.save_preset(" Crunch.flmpst "));
    assert_eq!(files.poll(&mut song), None);
    let path = PathBuf::from("My Presets/Distortion/Crunch.flmpst");
    assert_eq!(mem.0.borrow().saves, [(path.clone(), false)]);
    assert!(
        mem.0
            .borrow()
            .files
            .contains_key(Path::new("Crunch.flmpst"))
    );
    // The session keeps a copy where the preset picker lists it.
    let dir = files.presets_dir(6).unwrap();
    assert_eq!(dir, Path::new("My Presets/Distortion"));
    assert_eq!(
        files.user_files(&dir, PRESET_EXT),
        std::slice::from_ref(&path)
    );
    let bytes = files.read_user_file(&path).unwrap();
    let preset = crate::flm::read_preset(&bytes, None).unwrap();
    assert_eq!(preset.params, distortion().params);
    let desc = &preset.file;
    assert_eq!(
        (&*desc.category, &*desc.preset_name, &*desc.preset_path),
        (
            "My Presets/Distortion",
            "Crunch",
            "My Presets/Distortion/Crunch.flmpst"
        )
    );
    assert_eq!(
        (&*desc.source_path, &*desc.provenance),
        (
            "My Presets/Distortion/Crunch.flmpst",
            "My Presets/Distortion/Crunch"
        )
    );
    assert_eq!(files.take_flash().as_deref(), Some(SAVED));
    // Saving a preset is no song save: the song keeps its name, file and state.
    assert_eq!(song, Song::default());
    assert_eq!(files.path, None);
    assert_eq!(files.message, None);
}

/// Distortion with Bias at `bias`.
fn biased(bias: f32) -> PresetSource {
    let mut module = distortion();
    module.params[1] = bias;
    PresetSource::Module(module)
}

/// Answers the question `files` waits on, the overwrite question.
fn answer(files: &mut ProjectFiles, answer: Answer) {
    assert_eq!(files.waiting(), Some(OVERWRITE_TAG));
    files.update(
        &Context::default(),
        &mut Song::default(),
        false,
        Some(answer),
    );
}

/// A proposed name is always free; a typed name that is taken asks `The file already exists` with `Replace files`,
/// `Keep both` and `Cancel`. `dir` is where `files` puts Distortion presets.
fn a_taken_name_asks_before_it_is_written_over(mut files: ProjectFiles, dir: &Path) {
    let bias = |files: &ProjectFiles, name: &str| {
        let bytes = files.read_user_file(&dir.join(name)).unwrap();
        crate::flm::read_preset(&bytes, None).unwrap().params[1]
    };
    let listed = |files: &ProjectFiles| -> Vec<String> {
        (files.user_files(dir, PRESET_EXT).iter())
            .map(|p| p.file_name().unwrap().to_string_lossy().into_owned())
            .collect()
    };
    assert_eq!(save(&mut files, biased(0.1), "Distortion"), "Distortion");
    files.take_flash();
    files.ask_preset(biased(0.2), 1);
    let Some(Overlay::PresetDialog { name, .. }) = files.take_overlay() else {
        panic!("no preset dialog");
    };
    assert_eq!(name, "Distortion (1)", "the proposal is free");
    let ask = |files: &mut ProjectFiles| {
        assert!(!files.save_preset("Distortion"), "the dialog closed");
        assert_eq!(
            files.take_overlay(),
            Some(Overlay::Message {
                tag: OVERWRITE_TAG,
                lines: vec!["The file already exists".into(), "Distortion.flmpst".into()],
                confirm: Some(("Keep both".into(), "Cancel".into())),
                extra: Some("Replace files".into()),
            })
        );
    };
    // Cancel: nothing is written and the dialog stays, with what it saves.
    ask(&mut files);
    answer(&mut files, Answer::No);
    assert!(files.preset_open());
    assert_eq!(listed(&files), ["Distortion.flmpst"]);
    assert_eq!(files.take_flash(), None);
    // Keep both: the first free name; the dialog is done.
    ask(&mut files);
    answer(&mut files, Answer::Yes);
    assert!(!files.preset_open());
    assert_eq!(
        listed(&files),
        ["Distortion (1).flmpst", "Distortion.flmpst"]
    );
    assert_eq!(bias(&files, "Distortion (1).flmpst"), 0.2);
    assert_eq!(bias(&files, "Distortion.flmpst"), 0.1);
    assert_eq!(files.take_flash().as_deref(), Some(SAVED));
    // Replace files: written over.
    files.ask_preset(biased(0.3), 1);
    files.take_overlay();
    ask(&mut files);
    answer(&mut files, Answer::Extra);
    assert!(!files.preset_open());
    assert_eq!(
        listed(&files),
        ["Distortion (1).flmpst", "Distortion.flmpst"]
    );
    assert_eq!(bias(&files, "Distortion.flmpst"), 0.3);
    assert_eq!(files.message, None);
}

#[test]
fn on_the_web_a_taken_name_asks_before_it_is_written_over() {
    let (files, _) = web();
    a_taken_name_asks_before_it_is_written_over(files, Path::new("My Presets/Distortion"));
}

#[cfg(not(target_arch = "wasm32"))]
#[test]
fn on_the_desktop_a_taken_name_asks_before_it_is_written_over() {
    let user = std::env::temp_dir().join(format!("honeydew-r2-overwrite-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&user);
    a_taken_name_asks_before_it_is_written_over(
        ProjectFiles::on_disk(&user),
        &user.join("My Presets").join("Distortion"),
    );
    std::fs::remove_dir_all(&user).unwrap();
}

#[test]
fn closing_the_dialog_lets_go_of_what_it_would_save() {
    let (mut files, mem) = web();
    files.ask_preset(lead(), 1);
    files.take_overlay();
    assert!(files.preset_open());
    files.close_preset();
    assert!(!files.preset_open());
    assert!(files.save_preset("Lead"), "nothing to save closes");
    assert!(mem.0.borrow().saves.is_empty());
}

#[test]
fn only_the_preset_saves_own_events_finish_it() {
    let (mut files, _) = web();
    files.preset_saving = Some((PathBuf::from("My Racks/Lead.flmrack"), None));
    let saved = |p: &str| FileEvent::Saved(PathBuf::from(p));
    assert!(files.is_preset_event(&saved("My Racks/Lead.flmrack")));
    assert!(
        files.is_preset_event(&saved("Lead.flmrack")),
        "the web's download"
    );
    assert!(!files.is_preset_event(&saved("Lead.flm")));
    assert!(!files.is_preset_event(&saved("Other/Lead.flmrack")));
    let failed = |detail: &str| FileEvent::Failed(SAVE_FAILED, detail.into());
    // While no song save waits, a failed save is the preset's, whatever it names.
    assert!(files.is_preset_event(&failed("disk full")));
    files.saving = Some((PathBuf::from("Beat.flm"), None, Song::default()));
    assert!(!files.is_preset_event(&failed("Beat.flm: disk full")));
    assert!(!files.is_preset_event(&failed("disk full")));
    assert!(files.is_preset_event(&failed("Lead.flmrack: disk full")));
    assert!(files.is_preset_event(&failed("My Racks/Lead.flmrack: disk full")));
    // With no preset save under way, nothing is the preset's.
    files.preset_saving = None;
    assert!(!files.is_preset_event(&saved("Lead.flmrack")));
}

#[test]
fn a_bad_name_keeps_the_dialog_open_behind_the_message_and_saves_nothing() {
    let (mut files, mem) = web();
    files.ask_preset(PresetSource::Module(distortion()), 1);
    files.take_overlay();
    assert!(!files.save_preset("a:b"));
    assert_eq!(
        files.message,
        Some(vec!["Symbol ':' is not allowed in a file name.".to_owned()])
    );
    assert!(!files.save_preset("  "));
    assert!(!files.save_preset("CON"));
    assert!(mem.0.borrow().saves.is_empty());
}

#[test]
fn a_rack_preset_goes_into_my_racks_with_the_whole_chain() {
    let (mut files, mem) = web();
    assert_eq!(save(&mut files, lead(), "Lead 2"), "Lead 2");
    let path = PathBuf::from("My Racks/Lead 2.flmrack");
    assert_eq!(mem.0.borrow().saves, [(path.clone(), false)]);
    assert_eq!(files.racks_dir(), Some(PathBuf::from("My Racks")));
    assert_eq!(
        files.user_files(Path::new("My Racks"), RACK_EXT),
        std::slice::from_ref(&path)
    );
    let mut last = 9;
    let modules = crate::flm::read_rack(&files.read_user_file(&path).unwrap(), &mut last).unwrap();
    let PresetSource::Rack { modules: saved, .. } = lead() else {
        unreachable!()
    };
    let types: Vec<u16> = modules.iter().map(|m| m.type_id).collect();
    assert_eq!(types, [51, 6]);
    assert_eq!(modules[1].params, saved[1].params);
    assert_eq!(last, 11);
    // Not a module preset's folder, and no listing of what is not there.
    assert!(
        files
            .user_files(Path::new("My Presets"), PRESET_EXT)
            .is_empty()
    );
    assert!(
        files
            .read_user_file(Path::new("My Racks/Gone.flmrack"))
            .is_err()
    );
}

#[test]
fn a_failed_write_shows_failed_to_save_file_and_keeps_no_copy() {
    let (mut files, mem) = web();
    mem.0.borrow_mut().write_error = Some("disk full".into());
    save(&mut files, lead(), "Lead");
    assert_eq!(
        files.message,
        Some(vec![SAVE_FILE_FAILED.to_owned(), "disk full".to_owned()])
    );
    assert!(files.user_files(Path::new("My Racks"), RACK_EXT).is_empty());
    assert_eq!(files.take_flash(), None);
}

#[test]
fn a_web_copy_that_cannot_be_kept_shows_failed_to_save_file() {
    let (mut files, _) = web();
    files.ask_preset(lead(), 1);
    files.take_overlay();
    assert!(files.save_preset("Lead"));
    // A folder of the copy's name took its place meanwhile.
    let path = Path::new("My Racks/Lead.flmrack");
    files.user_copies.make_folder(path).unwrap();
    assert_eq!(files.poll(&mut Song::default()), None);
    let message = files.message.clone().expect("a message");
    assert_eq!(message[0], SAVE_FILE_FAILED);
    assert!(
        message[1].starts_with("My Racks/Lead.flmrack: "),
        "{message:?}"
    );
    assert_eq!(files.take_flash(), None);
}

#[cfg(not(target_arch = "wasm32"))]
#[test]
fn on_the_desktop_presets_land_in_the_user_folder_beside_my_songs() {
    // The user folder holds My Songs.
    let songs = crate::ui::main_menu::songs_dir();
    assert_eq!(
        ProjectFiles::default().user,
        songs.as_deref().and_then(Path::parent).map(Path::to_owned)
    );

    let user = std::env::temp_dir().join(format!("honeydew-r2-presets-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&user);
    let mut files = ProjectFiles::on_disk(&user);
    save(&mut files, PresetSource::Module(distortion()), "Crunch");
    save(&mut files, lead(), "Lead");
    let preset = user
        .join("My Presets")
        .join("Distortion")
        .join("Crunch.flmpst");
    let rack = user.join("My Racks").join("Lead.flmrack");
    assert!(preset.is_file() && rack.is_file());
    let dir = files.presets_dir(6).unwrap();
    assert_eq!(
        files.user_files(&dir, PRESET_EXT),
        std::slice::from_ref(&preset)
    );
    assert_eq!(
        files.user_files(&user.join("My Racks"), RACK_EXT),
        std::slice::from_ref(&rack)
    );
    let bytes = files.read_user_file(&preset).unwrap();
    assert_eq!(
        crate::flm::read_preset(&bytes, None).unwrap().params,
        distortion().params
    );
    std::fs::remove_dir_all(&user).unwrap();
}

#[cfg(not(target_arch = "wasm32"))]
#[test]
fn on_the_desktop_the_preset_pickers_list_and_read_the_user_folder() {
    let user = std::env::temp_dir().join(format!("honeydew-r3b-library-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&user);
    let mut files = ProjectFiles::on_disk(&user);
    save(&mut files, PresetSource::Module(distortion()), "Crunch");
    save(&mut files, lead(), "Lead");
    let dir = files.presets_dir(6).unwrap();
    // A folder the user made, and files of other kinds.
    std::fs::create_dir(dir.join("Live")).unwrap();
    std::fs::write(dir.join("Song.flm"), b"x").unwrap();
    std::fs::write(dir.join(".hidden.flmpst"), b"x").unwrap();
    let library = files.preset_library(&[PRESET_EXT]);
    let mut listed: Vec<_> = (library.list(Some(&dir)).unwrap().into_iter())
        .map(|e| (e.name, e.path, e.is_folder))
        .collect();
    listed.sort();
    let path = dir.join("Crunch.flmpst");
    assert_eq!(
        listed,
        [
            ("Crunch.flmpst".into(), path.clone(), false),
            ("Live".into(), dir.join("Live"), true)
        ]
    );
    assert_eq!(library.read(&path).unwrap(), std::fs::read(&path).unwrap());
    let racks = files.preset_library(&[RACK_EXT]);
    let mine = files.racks_dir().unwrap();
    let names: Vec<_> = (racks.list(Some(&mine)).unwrap().into_iter())
        .map(|e| e.name)
        .collect();
    assert_eq!(names, ["Lead.flmrack"]);
    std::fs::remove_dir_all(&user).unwrap();
}

/// The pickers' file actions on disk: `Recently Deleted` is the user folder's, beside `My Presets`,
/// and a MiniSynth folder lists both its preset types.
#[cfg(not(target_arch = "wasm32"))]
#[test]
fn on_the_desktop_the_pickers_file_actions_use_the_user_folders_bin() {
    use crate::ui::main_menu::{FileActions, Opt, SongEntry, Step};
    let user = std::env::temp_dir().join(format!("honeydew-r3c-files-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&user);
    let mut files = ProjectFiles::on_disk(&user);
    let dir = files.presets_dir(0).unwrap();
    std::fs::create_dir_all(&dir).unwrap();
    for name in ["Fat.flms", "Lead.flmpst", "Pad.flgsynth"] {
        std::fs::write(dir.join(name), name).unwrap();
    }
    let fat = dir.join("Fat.flms");
    let bin = user
        .join("Recently Deleted")
        .join(PRESETS_FOLDER)
        .join("MiniSynth");
    files.with_preset_files(PRESETS_FOLDER, preset_exts(0), |lib| {
        let mut names: Vec<_> = (lib.list(Some(&dir)).unwrap().into_iter())
            .map(|e| e.name)
            .collect();
        names.sort();
        assert_eq!(names, ["Fat.flms", "Lead.flmpst"]);
        assert_eq!(lib.bin_path(&dir), Some(bin.clone()));
        let entry = |path: &Path| SongEntry {
            name: path.file_name().unwrap().to_string_lossy().into(),
            path: path.to_owned(),
            is_folder: false,
            recency: 0,
        };
        let mut actions = FileActions::default();
        let deleted = actions.apply(Opt::Delete, vec![entry(&fat)], Some(dir.clone()), lib);
        assert_eq!(deleted, Step::Changed(Ok(())));
        assert!(!fat.exists() && bin.join("Fat.flms").exists());
        let restored = actions.apply(Opt::Restore, vec![entry(&bin.join("Fat.flms"))], None, lib);
        assert_eq!(restored, Step::Changed(Ok(())));
        assert!(fat.exists() && !bin.join("Fat.flms").exists());
    });
    std::fs::remove_dir_all(&user).unwrap();
}

#[test]
fn on_the_web_the_preset_pickers_list_and_read_this_sessions_copies() {
    let (mut files, _) = web();
    save(&mut files, PresetSource::Module(distortion()), "Crunch");
    save(&mut files, lead(), "Lead");
    let dir = files.presets_dir(6).unwrap();
    let path = dir.join("Crunch.flmpst");
    let library = files.preset_library(&[PRESET_EXT]);
    let crunch = crate::ui::main_menu::SongEntry {
        name: "Crunch.flmpst".into(),
        path: path.clone(),
        is_folder: false,
        recency: 0,
    };
    assert_eq!(library.list(Some(&dir)).unwrap(), [crunch]);
    assert_eq!(
        library.read(&path).unwrap(),
        files.read_user_file(&path).unwrap()
    );
    assert!(library.read(&dir.join("Gone.flmpst")).is_err());
    // The root is nobody's folder, and a scope lists its own files only.
    assert_eq!(library.list(None).unwrap(), []);
    let racks = files.preset_library(&[RACK_EXT]);
    assert_eq!(racks.list(Some(&dir)).unwrap(), []);
    let mine = files.racks_dir().unwrap();
    assert_eq!(racks.list(Some(&mine)).unwrap().len(), 1);
}

#[test]
fn a_saved_module_preset_names_the_card_it_came_from() {
    let (mut files, _) = web();
    let mut song = Song::default();
    song.add_channel(crate::song::ChannelKind::EffectTrack);
    let id = song.channels[1].rack.add(6);
    let module = song.channels[1].rack.get(id).unwrap().clone();
    files.ask_preset(PresetSource::Module(module), song.channels[1].id);
    files.take_overlay();
    assert!(files.save_preset("Crunch"));
    assert!(!files.take_preset_named(), "not before it is written");
    assert_eq!(files.poll(&mut song), None);
    assert!(files.take_preset_named());
    assert!(!files.take_preset_named(), "once");
    let file = &song.channels[1].rack.get(id).unwrap().file;
    assert_eq!(
        (&*file.category, &*file.preset_name, &*file.preset_path),
        (
            "My Presets/Distortion",
            "Crunch",
            "My Presets/Distortion/Crunch.flmpst"
        )
    );
    // A rack preset names nothing; nor does a module that is gone.
    save(&mut files, lead(), "Lead");
    assert!(!files.take_preset_named());
    let gone = ModuleInstance::new(6, 99);
    files.ask_preset(PresetSource::Module(gone), song.channels[1].id);
    files.take_overlay();
    files.save_preset("Other");
    let before = song.clone();
    files.poll(&mut song);
    assert!(!files.take_preset_named());
    assert_eq!(song, before);
}

// ---- Drumsets ----

fn drums() -> PresetSource {
    PresetSource::Drumset {
        name: "Drums".into(),
        kit: crate::song::DrumKit::default_kit(),
    }
}

#[test]
fn save_drumset_writes_the_kit_into_my_drumsets_and_asks_before_writing_over_one() {
    assert_eq!(PresetKind::Drumset.title(), "Save Drumset");
    assert_eq!(PresetKind::Drumset.format(), "Drumset (.tmpl)");
    assert_eq!(drums().kind(), PresetKind::Drumset);
    let (mut files, mem) = web();
    let mut song = Song::default();
    // The channel's name is proposed.
    files.ask_preset(drums(), 1);
    assert_eq!(
        files.take_overlay(),
        Some(Overlay::PresetDialog {
            name: "Drums".into(),
            kind: PresetKind::Drumset,
        })
    );
    assert!(files.save_preset("Drums"));
    assert_eq!(files.poll(&mut song), None);
    let path = PathBuf::from("My Drumsets/Drums.tmpl");
    assert_eq!(mem.0.borrow().saves, [(path.clone(), false)]);
    let dir = files.drumsets_dir().unwrap();
    assert_eq!(dir, Path::new("My Drumsets"));
    assert_eq!(
        files.user_files(&dir, DRUMSET_EXT),
        std::slice::from_ref(&path)
    );
    let read = |files: &ProjectFiles| {
        crate::flm::read_drumset(&files.read_user_file(&path).unwrap()).unwrap()
    };
    assert_eq!(read(&files), crate::song::DrumKit::default_kit());
    assert_eq!(files.take_flash().as_deref(), Some(SAVED));
    assert_eq!(song, Song::default(), "saving a kit is no song edit");
    // A taken name asks first; `Replace files` writes over it.
    let PresetSource::Drumset { name, mut kit } = drums() else {
        unreachable!()
    };
    kit.slots.truncate(2);
    files.ask_preset(PresetSource::Drumset { name, kit }, 1);
    let Some(Overlay::PresetDialog { name, .. }) = files.take_overlay() else {
        panic!("no dialog");
    };
    assert_eq!(name, "Drums (1)");
    assert!(!files.save_preset("Drums"));
    let Some(Overlay::Message { tag, lines, .. }) = files.take_overlay() else {
        panic!("no question");
    };
    assert_eq!(
        (tag, lines),
        (
            OVERWRITE_TAG,
            vec!["The file already exists".into(), "Drums.tmpl".into()]
        )
    );
    answer(&mut files, Answer::Extra);
    assert_eq!(
        files.user_files(&dir, DRUMSET_EXT),
        std::slice::from_ref(&path)
    );
    assert_eq!(read(&files).slots.len(), 2);
}
