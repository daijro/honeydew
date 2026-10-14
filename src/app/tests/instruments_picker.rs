//! The Instruments picker in the app: the add-channel `Instrument` opens it over our categories; a pick loads into the
//! top card or in its place at the bottom, names the channel, auditions, stays open and makes no undo step.

use std::path::Path;

use eframe::egui::{Id, Rect};

use super::rack_wiring::{PICKER, click_row, types, worker_done};
use super::*;
use crate::song::ModuleInstance;
use crate::ui::picker_preview::LOADING;
use crate::ui::rack::PresetTarget;

/// The root's rows with our factory: `My Files`, then our seven categories.
const CATEGORIES: [&str; 7] = ["Bass", "Bells", "Keyboard", "Lead", "Pad", "Pluck", "Synth"];
const MY_FILES: usize = 0;
const BELLS: usize = 2;
pub(super) const PLUCK: usize = 6;
const SYNTH: usize = 7;
/// The Instrument channel.
pub(super) const CH: usize = 2;

/// A MiniSynth channel, then the add-channel `Instrument` at index 2 with our factory: the Instruments picker is open
/// on it, the song as saved.
pub(super) fn instrument() -> Rig {
    let mut rig = Rig::with_synth();
    rig.app.files.factory = crate::content::factory();
    rig.app.insert_channel(CH, ChannelKind::Instrument, 0);
    for _ in 0..3 {
        rig.frame(vec![]);
    }
    rig.app.files.saved();
    rig.take();
    rig
}

fn open() -> Overlay {
    Overlay::PresetPicker {
        channel: CH,
        target: PresetTarget::Instruments,
    }
}

/// Row `i` of the open picker, as laid out.
fn row(rig: &mut Rig, i: usize) -> Rect {
    for _ in 0..3 {
        rig.frame(vec![]);
    }
    let id = Id::new(PICKER).with(("row", i));
    rig.ctx.read_response(id).expect("the row is drawn").rect
}

/// Our preset `name` of module type `type_id`, as the factory reads it.
fn ours(type_id: u16, folder: &str, name: &str) -> ModuleInstance {
    let path = Path::new(crate::content::ROOT)
        .join(crate::content::PRESETS)
        .join(folder)
        .join(format!("{name}.flmpst"));
    let bytes = crate::ui::main_menu::SongLibrary::read(crate::content::factory(), &path);
    crate::flm::read_preset(&bytes.unwrap(), Some(type_id)).unwrap()
}

#[test]
fn the_add_channel_instrument_opens_it_titled_with_my_files_then_our_categories() {
    let mut rig = instrument();
    assert_eq!(rig.overlays(), [open()]);
    rig.frame(vec![]);
    assert!(rig.painted("Instruments") && rig.painted("My Files"));
    for name in CATEGORIES {
        assert!(rig.painted(name), "{name}");
    }
    // No `Default` row, and no store.
    assert!(!rig.painted("Default"));
    // Root order: `My Files`, then the categories, top to bottom.
    let tops: Vec<f32> = (0..8).map(|i| row(&mut rig, i).top()).collect();
    assert!(tops.windows(2).all(|w| w[0] < w[1]), "{tops:?}");
}

#[test]
fn a_pick_of_the_top_cards_type_loads_into_it_names_the_channel_and_makes_no_undo_step() {
    let mut rig = instrument();
    let id = rig.app.song.channels[CH].rack.modules[0].instance_id;
    let steps = rig.app.history.steps();
    click_row(&mut rig, PLUCK);
    // Pluck: `Ice Pluck`, ours, a SuperSaw's (same type, same module, same id).
    click_row(&mut rig, 0);
    rig.frame(vec![]);
    let ch = &rig.app.song.channels[CH];
    let module = &ch.rack.modules[0];
    assert_eq!(ch.rack.modules.len(), 1);
    assert_eq!((module.instance_id, module.type_id), (id, 23));
    assert_eq!(
        module.params,
        ours(23, "SuperSaw", "Pluck/Ice Pluck").params
    );
    assert_eq!(module.file.preset_name, "Ice Pluck");
    assert_eq!(ch.name, "Ice Pluck", "the first pick renames `Instrument`");
    // No undo step; the song is changed; the picker stays open; the first tap is silent but for the new engine.
    assert_eq!(rig.app.history.steps(), steps);
    assert!(rig.app.files.modified());
    assert_eq!(rig.overlays(), [open()]);
    assert_eq!(rig.take(), ["ReplaceSong adopting no-resync"]);
    // The audition strip is on over the picked row: a touch plays its x note on the channel.
    let ice = row(&mut rig, 0);
    rig.press(ice.left_center());
    rig.frame(vec![]);
    assert_eq!(rig.take(), [format!("NoteOn {CH} 24 127")]);
    rig.release(ice.left_center());
    assert_eq!(rig.take(), [format!("NoteOff {CH} 24")]);
}

#[test]
fn a_pick_of_another_type_deletes_the_top_card_and_appends_its_instrument_at_the_bottom_keeping_the_effects()
 {
    let mut rig = instrument();
    // An effect under the SuperSaw.
    let supersaw = rig.app.song.channels[CH].rack.modules[0].instance_id;
    let fx = rig.app.song.channels[CH].rack.add(6);
    rig.app.song_edited(SongEdit::Final, 0);
    let steps = rig.app.history.steps();
    rig.app.files.saved();
    click_row(&mut rig, BELLS);
    // Bells: `Frost Chime`, a GMSynth's.
    click_row(&mut rig, 0);
    rig.frame(vec![]);
    assert_eq!(types(&rig, CH), [6, 2]);
    let modules = &rig.app.song.channels[CH].rack.modules;
    assert_eq!(modules[0].instance_id, fx, "the effect is kept");
    assert!(modules[1].instance_id > fx && modules[1].instance_id != supersaw);
    assert_eq!(
        modules[1].params,
        ours(2, "GMSynth", "Bells/Frost Chime").params
    );
    assert_eq!(rig.app.song.channels[CH].name, "Frost Chime");
    assert_eq!(rig.app.history.steps(), steps);
    assert!(rig.app.files.modified());
    assert_eq!(rig.overlays(), [open()]);
    // The next pick of another preset of it: the top card is the effect now, so it goes too.
    click_row(&mut rig, 1);
    rig.frame(vec![]);
    assert_eq!(types(&rig, CH), [2, 2]);
    assert_eq!(rig.app.song.channels[CH].name, "Tin Carousel");
    // Closing keeps the last pick.
    rig.frame(escape());
    rig.frame(vec![]);
    assert_eq!(rig.overlays(), []);
    assert_eq!(types(&rig, CH), [2, 2]);
}

#[test]
fn a_pick_loading_on_a_worker_replaces_the_card_at_once_pulses_loading_and_names_the_channel_when_it_lands()
 {
    let mut rig = instrument();
    let hold = crate::ui::views::preset_picker::WORKER_HOLD
        .lock()
        .unwrap_or_else(std::sync::PoisonError::into_inner);
    click_row(&mut rig, SYNTH);
    // Synth: `Brass Swell`, a 3x Osc's preset, which loads on a worker.
    click_row(&mut rig, 0);
    rig.frame(vec![]);
    rig.frame(vec![]);
    assert!(rig.painted(LOADING), "the picked row pulses");
    assert_eq!(types(&rig, CH), [39]);
    // The swap marks the song changed at once, and so does the landing.
    assert!(rig.app.files.modified());
    rig.app.files.saved();
    // The swap rebuilds the engine at once, before the preset lands.
    assert_eq!(rig.take(), ["ReplaceSong adopting no-resync"]);
    assert_eq!(
        rig.app.song.channels[CH].name, "Instrument",
        "named when it lands"
    );
    // A touch during the load sends a note to the channel.
    let brass = row(&mut rig, 0);
    rig.take();
    rig.press(brass.center());
    rig.frame(vec![]);
    let note = rig.take();
    assert_eq!(note.len(), 1);
    assert!(note[0].starts_with(&format!("NoteOn {CH} ")), "{note:?}");
    rig.release(brass.center());
    drop(hold);
    worker_done(&mut rig);
    rig.frame(vec![]);
    assert!(rig.app.files.modified());
    assert!(!rig.painted(LOADING));
    let module = &rig.app.song.channels[CH].rack.modules[0];
    assert_eq!(module.params, ours(39, "3xOsc", "Synth/Brass Swell").params);
    assert_eq!(rig.app.song.channels[CH].name, "Brass Swell");
    assert_eq!(rig.overlays(), [open()]);
}

#[test]
fn my_files_lists_the_whole_user_folder_and_an_audio_or_instr_pick_does_nothing() {
    let mut rig = instrument();
    let gm = crate::flm::write_preset(&ModuleInstance::new(2, 0)).unwrap();
    let files = &mut rig.app.files;
    files.keep_user_file(Path::new("My Presets/GMSynth/Mine.flmpst"), gm);
    files.keep_user_file(Path::new("My Samples/Hit.wav"), b"RIFF".to_vec());
    files.keep_user_file(Path::new("My Instruments/Old.instr"), b"x".to_vec());
    let song = rig.app.song.clone();
    click_row(&mut rig, MY_FILES);
    rig.frame(vec![]);
    for folder in ["My Instruments", "My Presets", "My Samples"] {
        assert!(rig.painted(folder), "{folder}");
    }
    // My Instruments: `Old`, a `.instr`, lists and loads nothing; My Samples' `Hit` likewise.
    let back = |rig: &mut Rig| rig.click_id(Id::new(PICKER).with("back"));
    for folder in [0, 2] {
        click_row(&mut rig, folder);
        click_row(&mut rig, 0);
        rig.frame(vec![]);
        assert_eq!(rig.app.song, song);
        assert_eq!(rig.overlays(), [open()]);
        back(&mut rig);
    }
    // My Presets / GMSynth / Mine loads: a GMSynth in the SuperSaw's place, named after it.
    click_row(&mut rig, 1);
    click_row(&mut rig, 0);
    click_row(&mut rig, 0);
    rig.frame(vec![]);
    assert_eq!(types(&rig, CH), [2]);
    assert_eq!(rig.app.song.channels[CH].name, "Mine");
}

#[test]
fn a_pick_of_a_preset_named_default_names_the_channel_by_its_instrument_and_id() {
    let mut rig = instrument();
    let gm = crate::flm::write_preset(&ModuleInstance::new(2, 0)).unwrap();
    let path = Path::new("My Presets/GMSynth/Default.flmpst");
    rig.app.files.keep_user_file(path, gm);
    let id = rig.app.song.channels[CH].id;
    // My Files › My Presets › GMSynth › Default.
    for row in [MY_FILES, 0, 0, 0] {
        click_row(&mut rig, row);
    }
    rig.frame(vec![]);
    assert_eq!(types(&rig, CH), [2]);
    assert_eq!(rig.app.song.channels[CH].name, format!("GMSynth {id}"));
    assert_eq!(rig.app.song.channels[CH].file.preset_name, "");
}

/// A folder the Instruments picker's `My Files` renames takes the open song's file along, as SONGS' file actions do.
/// With `My Songs/F/Beat.flm` open, `F` renamed to `Old` there, and a new `F` holding another `Beat.flm`, `Ctrl+S`
/// writes `Old/Beat.flm` and leaves the other song as it was.
#[cfg(not(target_arch = "wasm32"))]
#[test]
fn a_folder_renamed_in_the_picker_takes_the_open_songs_file_along() {
    use crate::project_files::content::tests::{Temp, put};
    use crate::ui::main_menu::{NativeLibrary, SONGS_FOLDER};
    let tmp = Temp::new("fd4-picker-rename");
    let songs = tmp.0.join(SONGS_FOLDER);
    put(
        &songs.join("F/Beat.flm"),
        &synthetic_file(1, FORMAT_VERSION, ""),
    );
    let mut rig = Rig::with_synth();
    let mut files = without_factory(ProjectFiles::on_disk(&tmp.0));
    files.library = Box::new(NativeLibrary::at(songs.clone()));
    files.codec = synthetic_codec();
    rig.app.files = files;
    rig.app.files.saved();
    let opened = (rig.app.files).replace(Replace::Open(songs.join("F/Beat.flm")));
    rig.app.song = opened.expect("the song opens");
    rig.app.navigate(Nav::Open(Overlay::PresetPicker {
        channel: 1,
        target: PresetTarget::Instruments,
    }));
    click_row(&mut rig, MY_FILES);
    // `My Songs`, then `F` selected.
    click_row(&mut rig, 0);
    let select = rig.text_rect("Select").center();
    rig.click(select);
    click_row(&mut rig, 0);
    options(&mut rig);
    let rename = rig.text_rect("Rename").center();
    rig.click(rename);
    for _ in 0..3 {
        rig.frame(vec![]);
    }
    rig.frame(key_tap(egui::Key::Backspace, egui::Modifiers::NONE));
    rig.frame(vec![egui::Event::Text("Old".into())]);
    rig.frame(key_tap(egui::Key::Enter, egui::Modifiers::NONE));
    rig.frame(vec![]);
    assert!(songs.join("Old/Beat.flm").is_file() && !songs.join("F").exists());
    // Another song where the open one was, as SONGS or a file manager puts it; then an edit, and Ctrl+S.
    let other = synthetic_file(5, FORMAT_VERSION, "");
    put(&songs.join("F/Beat.flm"), &other);
    rig.app.navigate(Nav::Close);
    rig.frame(vec![]);
    assert_eq!(rig.overlays(), []);
    rig.app.add_default_drums(0);
    // The web page would ask before it closes.
    assert!(rig.app.unsaved());
    command(&mut rig, egui::Key::S);
    for _ in 0..3 {
        rig.frame(vec![]);
    }
    assert_eq!(std::fs::read(songs.join("F/Beat.flm")).unwrap(), other);
    let saved = synthetic_file(2, FORMAT_VERSION, "");
    assert_eq!(std::fs::read(songs.join("Old/Beat.flm")).unwrap(), saved);
    assert!(!rig.app.unsaved());
}

/// Opens the picker's ≡ menu (the left column's third button) and lets its items show.
fn options(rig: &mut Rig) {
    let m = Metrics::from_pixels_per_point(1.0, DEFAULT_UI_SCALE_INDEX);
    let screen = Rect::from_min_size(Pos2::ZERO, vec2(1170.0, 540.0));
    rig.click(crate::ui::modules_browser::layout(screen, 1.0, &m).button(2));
    for _ in 0..30 {
        rig.frame(vec![]);
    }
}

/// `My Files` › `Recently Deleted` in the Instruments picker is the bin, as the other pickers' bin row is: its ≡ menu
/// offers `Restore` and `Delete permanently`, which asks first, and never the `Delete` that would remove a binned file
/// for good without a question.
#[cfg(not(target_arch = "wasm32"))]
#[test]
fn in_the_instruments_pickers_bin_only_delete_permanently_removes_and_it_asks_first() {
    use crate::project_files::content::tests::{Temp, put};
    use crate::ui::main_menu::library::RECENTLY_DELETED;
    let tmp = Temp::new("fd4-picker-bin");
    let binned = tmp.0.join(RECENTLY_DELETED).join("My Presets/Lead.flmpst");
    put(&binned, b"lead");
    let mut rig = Rig::with_synth();
    rig.app.files = without_factory(ProjectFiles::on_disk(&tmp.0));
    rig.app.navigate(Nav::Open(Overlay::PresetPicker {
        channel: 1,
        target: PresetTarget::Instruments,
    }));
    click_row(&mut rig, MY_FILES);
    // `Recently Deleted`, whose crumb reads as the bin's note, cut in its middle on this
    // screen; then its `My Presets` selected.
    click_row(&mut rig, 0);
    for _ in 0..3 {
        rig.frame(vec![]);
    }
    let header = (rig.out.shapes.iter()).find_map(|s| match &s.shape {
        egui::Shape::Text(t) if t.galley.text().starts_with("     Instruments") => {
            Some(t.galley.text().to_owned())
        }
        _ => None,
    });
    assert!(
        header
            .as_deref()
            .is_some_and(|h| h.ends_with("after 30 days)")),
        "{header:?}"
    );
    let select = rig.text_rect("Select").center();
    rig.click(select);
    click_row(&mut rig, 0);
    options(&mut rig);
    assert!(rig.painted("Restore") && rig.painted("Delete permanently"));
    assert!(!rig.painted("Delete"));
    let purge = rig.text_rect("Delete permanently").center();
    rig.click(purge);
    for _ in 0..30 {
        rig.frame(vec![]);
    }
    assert!(rig.painted("Delete 1 file?"));
    assert_eq!(std::fs::read(&binned).unwrap(), b"lead");
}

#[test]
fn a_later_pick_on_the_ui_thread_replaces_an_instruments_pick_still_loading() {
    // The Instruments twin of `a_later_pick_on_the_ui_thread_replaces_a_pick_still_loading`: `Brass Swell` (a 3x Osc
    // preset, on a worker), then a 3x Osc `.fst` (on the UI thread, at once): `Brass Swell` must not land over it.
    let mut rig = instrument();
    let fst = [
        &b"FLhd"[..],
        &6u32.to_le_bytes(),
        &0x30u16.to_le_bytes(),
        &[1, 0, 96, 0],
        b"FLdt",
        &0u32.to_le_bytes(),
    ]
    .concat();
    (rig.app.files).keep_user_file(Path::new("My Presets/3xOsc/Gen.fst"), fst);
    let hold = crate::ui::views::preset_picker::WORKER_HOLD
        .lock()
        .unwrap_or_else(std::sync::PoisonError::into_inner);
    click_row(&mut rig, SYNTH);
    click_row(&mut rig, 0);
    rig.frame(vec![]);
    assert!(rig.app.slots.preset_picker.loading());
    // Back to the root, then My Files › My Presets › 3xOsc › Gen.
    rig.click_id(Id::new(PICKER).with("back"));
    for row in [MY_FILES, 0, 0, 0] {
        click_row(&mut rig, row);
    }
    let last = rig.app.song.channels[CH].rack.modules[0].clone();
    assert_eq!((last.type_id, &*last.file.preset_name), (39, "Gen"));
    assert!(
        !rig.app.slots.preset_picker.loading(),
        "the load is dropped"
    );
    drop(hold);
    crate::ui::views::preset_picker::join_workers();
    for _ in 0..3 {
        rig.frame(vec![]);
    }
    assert_eq!(
        rig.app.song.channels[CH].rack.modules[0], last,
        "the last pick stays"
    );
}
