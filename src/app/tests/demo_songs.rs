//! Our demo song through the whole app: the first run puts it into the songs folder's `Demo songs` once
//! (`demoSongsVersion`), with no flash; SETTINGS `Restore demo songs` writes it back behind its loading screen, held
//! 1 s, then flashes `Restored`; and SONGS lists the folder and opens the song with no notices, ready to play.

use std::path::{Path, PathBuf};

use honeydew_audio::device::AudioRenderer;

use super::*;
use crate::app::demo_restore::{RESTORED, RESTORING_DEMO_SONGS};
use crate::audio_engine::SongAudioEngine;
use crate::content::{DEMO_FOLDER, demo_songs};
use crate::ui::main_menu::{MemoryLibrary, SongLibrary};

/// The demo's file in the session library, and its bytes.
fn demo_file() -> (PathBuf, Vec<u8>) {
    let (name, bytes) = demo_songs().remove(0);
    (Path::new(DEMO_FOLDER).join(name), bytes)
}

/// A start of the app with the settings `spy` keeps and the songs `library`: the first-run hook, then a frame (which
/// keeps the settings).
fn start(spy: &Spy, library: Box<dyn SongLibrary>) -> Rig {
    let mut rig = Rig::new();
    let settings = Settings::load(spy);
    rig.app.settings_saved = settings.clone();
    rig.app.settings = settings;
    rig.app.storage = Some(Box::new(spy.clone()));
    rig.app.files.library = library;
    rig.app.extract_demo_songs(false);
    rig.frame(vec![]);
    rig
}

#[test]
fn the_first_run_writes_the_demo_and_the_next_run_does_not() {
    let (file, bytes) = demo_file();
    let spy = Spy::default();
    let rig = start(&spy, Box::<MemoryLibrary>::default());
    assert_eq!(rig.app.files.library.read(&file).unwrap(), bytes);
    assert_eq!(Settings::load(&spy).demo_songs_version, 2, "kept");
    // The user deletes it; the next start leaves it deleted.
    let mut library = rig.app.files.library;
    library.remove(&file).unwrap();
    let mut rig = start(&spy, library);
    assert!(!rig.app.files.library.exists(&file));
    // The web's library lasts a session: there they go in at every start.
    rig.app.extract_demo_songs(true);
    assert_eq!(rig.app.files.library.read(&file).unwrap(), bytes);
}

#[test]
fn a_first_run_that_fails_tries_again_at_the_next_start() {
    /// A library that stores nothing (a desktop with no songs folder).
    struct Nowhere;
    impl SongLibrary for Nowhere {
        fn list(&self, _: Option<&Path>) -> std::io::Result<Vec<crate::ui::main_menu::SongEntry>> {
            Ok(Vec::new())
        }
        fn read(&self, _: &Path) -> std::io::Result<Vec<u8>> {
            Err(std::io::ErrorKind::NotFound.into())
        }
        fn store(&mut self, _: &str, _: &[u8]) -> std::io::Result<()> {
            Err(std::io::ErrorKind::NotFound.into())
        }
        fn root(&self) -> Option<&Path> {
            None
        }
    }
    let spy = Spy::default();
    let rig = start(&spy, Box::new(Nowhere));
    assert_eq!(rig.app.settings.demo_songs_version, 0);
    let rig = start(&spy, Box::<MemoryLibrary>::default());
    assert!(rig.app.files.library.exists(&demo_file().0));
}

/// The main menu's tab button `index` (SONGS 0, SETTINGS 2).
fn tab(index: usize) -> egui::Id {
    egui::Id::new("main_menu").with(("tab", index))
}

/// The `Restoring demo songs...` loading screen is up.
fn busy(rig: &Rig) -> bool {
    let screen = Overlay::Busy {
        title: RESTORING_DEMO_SONGS.into(),
    };
    rig.overlays().contains(&screen)
}

/// Taps SETTINGS `Restore demo songs` in the main menu (switching to SETTINGS when it is open, else opening it
/// there), below the screen's end until the tab is scrolled to its end.
fn tap_restore(rig: &mut Rig) {
    match (rig.overlays().iter()).any(|o| matches!(o, Overlay::MainMenu(_))) {
        true => rig.click_id(tab(2)),
        false => rig.open_menu(MenuTab::Settings),
    }
    rig.wheel(pos2(900.0, 300.0), -5000.0);
    // Back two rows (4 U) from the end, so that the button clears the tab row.
    rig.wheel(pos2(900.0, 300.0), 128.0);
    let label = "Restore demo songs";
    let at = (rig.output(vec![]).shapes.into_iter())
        .find_map(|c| match c.shape {
            egui::Shape::Text(t) if t.galley.text() == label => {
                Some(t.visual_bounding_rect().center())
            }
            _ => None,
        })
        .expect("the button is on screen");
    rig.click(at);
}

/// Runs frames until the loading screen goes (5 s at most): the rig times of the frame the demo landed in and of the
/// frame the screen closed in.
fn finish(rig: &mut Rig) -> (f64, f64) {
    let (file, bytes) = demo_file();
    let mut copied = None;
    for _ in 0..50 {
        let there = rig.app.files.library.read(&file).is_ok_and(|b| b == bytes);
        if copied.is_none() && there {
            copied = Some(rig.time);
        }
        if !busy(rig) {
            return (copied.expect("the copy, behind the screen"), rig.time);
        }
        rig.frame(vec![]);
    }
    panic!("the loading screen stays up");
}

/// `Restore demo songs` from the tap to its flash.
fn restore(rig: &mut Rig) {
    tap_restore(rig);
    finish(rig);
}

#[test]
fn restore_demo_songs_runs_behind_its_loading_screen_held_1_s_then_flashes_restored() {
    assert_eq!(
        RESTORING_DEMO_SONGS, "Restoring demo songs...",
        "loading title"
    );
    assert_eq!(RESTORED, "Restored", "completion toast");
    let (file, bytes) = demo_file();
    let mut rig = Rig::new();
    tap_restore(&mut rig);
    // No question: the loading screen at once, the copy behind it.
    assert!(busy(&rig), "{:?}", rig.overlays());
    assert!(rig.texts().contains(&RESTORING_DEMO_SONGS.to_owned()));
    let (copied, closed) = finish(&mut rig);
    // Held 1 s after the copy (the rig steps 0.1 s a frame).
    let held = closed - copied;
    assert!((1.0 - 1e-9..1.15).contains(&held), "held {held} s");
    assert_eq!(rig.app.files.library.read(&file).unwrap(), bytes);
    assert_eq!(rig.app.settings.demo_songs_version, 2);
    let texts = rig.texts();
    assert!(texts.contains(&RESTORED.to_owned()), "{texts:?}");
    assert!(!texts.contains(&RESTORING_DEMO_SONGS.to_owned()));
    assert!(!(rig.overlays().iter()).any(|o| matches!(o, Overlay::Message { .. })));
    // Deleted, then restored again.
    rig.app.files.library.remove(&file).unwrap();
    rig.app.flash = None;
    restore(&mut rig);
    assert_eq!(rig.app.files.library.read(&file).unwrap(), bytes);
    assert!(rig.texts().contains(&RESTORED.to_owned()));
}

/// The first launch's copy shows no flash (nothing raises `Demo songs extracted`).
#[test]
fn the_first_run_shows_no_flash_or_loading_screen() {
    let rig = start(&Spy::default(), Box::<MemoryLibrary>::default());
    assert!(rig.app.files.library.exists(&demo_file().0));
    assert_eq!(rig.app.flash, None);
    assert!(!busy(&rig));
}

/// Settings lost (a wiped or unwritable settings file) read as a first run: the hook keeps the user's copy, as it
/// replaces nothing; `Restore demo songs` replaces it, the user's copy going to Recently Deleted first.
#[test]
fn a_lost_flag_does_not_replace_the_users_copy_but_restore_does() {
    let (file, bytes) = demo_file();
    let mut library = MemoryLibrary::default();
    library.make_folder(Path::new(DEMO_FOLDER)).unwrap();
    let name = file.file_name().unwrap().to_str().unwrap();
    library
        .store_in(Path::new(DEMO_FOLDER), name, b"mine")
        .unwrap();
    let mut rig = start(&Spy::default(), Box::new(library));
    assert_eq!(rig.app.files.library.read(&file).unwrap(), b"mine");
    assert_eq!(rig.app.settings.demo_songs_version, 2);
    let binned = rig.app.files.library.bin_path(&file).unwrap();
    restore(&mut rig);
    assert_eq!(rig.app.files.library.read(&file).unwrap(), bytes);
    assert_eq!(rig.app.files.library.read(&binned).unwrap(), b"mine");
}

/// SONGS, open while `Restore demo songs` runs from SETTINGS, lists the new folder when it comes back.
#[test]
fn songs_lists_demo_songs_after_a_restore_from_settings() {
    let mut rig = Rig::new();
    rig.open_menu(MenuTab::Songs);
    assert!(!rig.texts().contains(&DEMO_FOLDER.to_owned()));
    restore(&mut rig);
    rig.click_id(tab(0));
    rig.frame(vec![]);
    assert!(rig.texts().contains(&DEMO_FOLDER.to_owned()));
}

#[test]
fn songs_lists_demo_songs_and_the_demo_opens_locked_with_no_notices_and_plays() {
    let (file, _) = demo_file();
    let mut rig = Rig::new();
    rig.app.extract_demo_songs(false);
    rig.open_menu(MenuTab::Songs);
    assert!(rig.texts().contains(&DEMO_FOLDER.to_owned()));
    rig.click_id(row(DEMO_FOLDER));
    rig.frame(vec![]);
    rig.click_id(songs_id(("row", file.clone())));
    rig.frame(vec![]);
    let song = rig.app.song.clone();
    let stem = file.file_stem().unwrap().to_string_lossy();
    assert_eq!(song.title, stem);
    assert_eq!(song.file.meta.display_title, stem);
    // The metadata lock read back (META@776), and no load notices.
    assert!(song.file.meta.locked);
    assert!(!(rig.overlays().iter()).any(|o| matches!(o, Overlay::Message { .. })));
    assert_eq!(rig.take(), ["Stop", "ReplaceSong"]);
    // It plays, on the app's own samples: the main part, from bar 5.
    let samples = &rig.app.audio.samples;
    assert_eq!(samples.missing().count(), 0);
    let mut engine = SongAudioEngine::with_samples(&song, 48_000.0, samples).unwrap();
    engine.seek(u64::from(4 * 4 * crate::song::PPQ));
    engine.play();
    let mut out = vec![0.0; (4.0 * 60.0 / song.tempo_bpm * 48_000.0) as usize * 2];
    engine.render(&mut out, 2);
    let peak = out.iter().fold(0.0_f32, |p, s| p.max(s.abs()));
    assert!(peak > 0.03, "{peak}");
}
