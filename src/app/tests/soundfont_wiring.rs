//! The SoundFont Player's patch selector in the app: the `Patch` spinner opens the SoundFont picker, whose patch
//! pick sets the module's `SF2D` with no undo step (the rack is not covered by undo), the picker staying open, and
//! `Prev` / `Next` step through the patches.

use std::path::Path;

use eframe::egui::Id;

use super::rack_wiring::{close_picker, worker_done};
use super::*;
use crate::flm::chunk::{ContentPath, ContentRoot};
use crate::flm::sf2d::Sf2d;
use crate::panels::inst::soundfont_player::{NEXT, PATCH, PREV};
use crate::sf2::tests::soundfont;
use crate::song::ModuleInstance;
use crate::ui::rack::PresetTarget;

/// The SoundFont Player's channel, and its instrument's instance id.
const CHANNEL: usize = 2;
const PLAYER: i32 = 2;

/// [`Rig::with_synth`] with the web's files, holding `My SoundFonts/Piano.sf2` and a broken `Broken.sf2` (the web
/// lists no folders), and a SoundFont Player channel (2), its rack open, with an empty log.
fn with_soundfonts() -> Rig {
    let mut rig = Rig::with_synth();
    let mem = MemFiles::default();
    mem.0.borrow_mut().names_only = true;
    rig.app.files = ProjectFiles::with_provider(Box::new(mem));
    let piano = soundfont(&[("Upright", 0, 0), ("Grand", 1, 0), ("Bright", 2, 0)]);
    let files = &mut rig.app.files;
    files.keep_user_file(Path::new("My SoundFonts/Piano.sf2"), piano);
    files.keep_user_file(Path::new("My SoundFonts/Broken.sf2"), b"junk".to_vec());
    rig.app
        .add_channel(ChannelKind::Synth(SynthKind::SoundFontPlayer), 0);
    rig.open_rack(CHANNEL);
    rig.slide_rack();
    rig.take();
    rig
}

/// Control `#n` of the player's panel.
fn control(n: u32) -> Id {
    Id::new(("rack", CHANNEL))
        .with(("module panel", PLAYER))
        .with(n)
}

/// Waits out the picker's slide-in and clicks its row `i`.
fn picker_row(rig: &mut Rig, i: usize) {
    for _ in 0..3 {
        rig.frame(vec![]);
    }
    rig.click_id(Id::new("preset_picker").with(("row", i)));
}

/// Where the last frame drew `text`.
fn text_rect(rig: &Rig, text: &str) -> egui::Rect {
    let found = rig.out.shapes.iter().find_map(|s| match &s.shape {
        egui::Shape::Text(t) if t.galley.text() == text => Some(t.visual_bounding_rect()),
        _ => None,
    });
    found.unwrap_or_else(|| panic!("{text} not drawn"))
}

fn shows(rig: &Rig, text: &str) -> bool {
    rig.out.shapes.iter().any(|s| match &s.shape {
        egui::Shape::Text(t) => t.galley.text() == text,
        _ => false,
    })
}

/// The player's `SF2D`.
fn sf2d(rig: &Rig) -> Option<Sf2d> {
    let module = rig.app.song.find_module(CHANNEL, PLAYER).unwrap();
    module.file.custom_data.as_deref().and_then(Sf2d::read)
}

/// Takes the player's SoundFont away, as a new player has none (an undo no longer does: the rack is not covered by
/// undo).
fn clear_soundfont(rig: &mut Rig) {
    let module = rig.app.song.channels[CHANNEL].rack.get_mut(PLAYER).unwrap();
    module.file.custom_data = None;
    rig.app.song_edited(SongEdit::Final, 0);
}

fn in_my_soundfonts(file: &str, patch: i32) -> Option<Sf2d> {
    let path = ContentPath::Shorthand {
        root: ContentRoot::User,
        relative: format!("My SoundFonts/{file}"),
    };
    Some(Sf2d { path, patch })
}

#[test]
fn the_patch_spinner_picks_a_soundfonts_patch_and_prev_and_next_step_each_with_no_undo_step() {
    let mut rig = with_soundfonts();
    let steps = rig.app.history.steps();
    assert!(shows(&rig, "Load Patch"));
    rig.click_id(control(PATCH));
    let picker = Overlay::SoundFontPicker {
        channel: CHANNEL,
        module: PLAYER,
    };
    assert_eq!(rig.overlays(), [Overlay::Rack(CHANNEL), picker.clone()]);
    // My Files, then the SoundFont (after `Broken`), and its second patch: it loads at once, and the picker stays.
    for row in [0, 1, 1] {
        picker_row(&mut rig, row);
    }
    assert_eq!(rig.overlays(), [Overlay::Rack(CHANNEL), picker.clone()]);
    assert_eq!(sf2d(&rig), in_my_soundfonts("Piano.sf2", 1));
    assert_eq!(rig.take(), ["ReplaceSong adopting no-resync"]);
    close_picker(&mut rig);
    assert!(shows(&rig, "Grand") && !shows(&rig, "Load Patch"));
    // Another channel's rack and back: what the rack read stays, so the patch shows at once, never `...`.
    rig.open_rack(1);
    rig.frame(vec![]);
    rig.open_rack(CHANNEL);
    rig.frame(vec![]);
    assert_eq!(rig.overlays(), [Overlay::Rack(CHANNEL)]);
    assert!(shows(&rig, "Grand") && !shows(&rig, "..."));

    // Next, Next (round to the first), Prev (round to the last).
    for (button, patch, name) in [
        (NEXT, 2, "Bright"),
        (NEXT, 0, "Upright"),
        (PREV, 2, "Bright"),
    ] {
        rig.click_id(control(button));
        rig.frame(vec![]);
        assert_eq!(sf2d(&rig), in_my_soundfonts("Piano.sf2", patch));
        assert!(shows(&rig, name), "{name}");
        assert_eq!(rig.take(), ["ReplaceSong adopting no-resync"]);
    }

    // With a SoundFont loaded the spinner opens the picker in its patches, in file order; a pick is one edit, and
    // the picker stays.
    rig.click_id(control(PATCH));
    assert_eq!(rig.overlays(), [Overlay::Rack(CHANNEL), picker.clone()]);
    picker_row(&mut rig, 0);
    assert_eq!(sf2d(&rig), in_my_soundfonts("Piano.sf2", 0));
    assert_eq!(rig.take(), ["ReplaceSong adopting no-resync"]);
    assert_eq!(rig.overlays(), [Overlay::Rack(CHANNEL), picker]);
    close_picker(&mut rig);

    // No undo step for any of them.
    assert_eq!(rig.app.history.steps(), steps);
}

#[test]
fn a_soundfont_that_does_not_load_shows_its_error_in_the_panel() {
    let mut rig = with_soundfonts();
    rig.click_id(control(PATCH));
    // My Files, then `Broken`.
    picker_row(&mut rig, 0);
    picker_row(&mut rig, 0);
    assert_eq!(rig.overlays(), [Overlay::Rack(CHANNEL)]);
    assert_eq!(sf2d(&rig), in_my_soundfonts("Broken.sf2", 0));
    assert_eq!(rig.take(), ["ReplaceSong adopting no-resync"]);
    rig.frame(vec![]);
    assert!(shows(&rig, "Not a SoundFont file"));
    assert!(!shows(&rig, "Prev") && !shows(&rig, "Patch"));
    // With the patch selector hidden, a tap on the error opens the picker; Escape closes it.
    rig.click(text_rect(&rig, "Not a SoundFont file").center());
    let picker = Overlay::SoundFontPicker {
        channel: CHANNEL,
        module: PLAYER,
    };
    assert_eq!(rig.overlays(), [Overlay::Rack(CHANNEL), picker]);
    rig.frame(escape());
    assert_eq!(rig.overlays(), [Overlay::Rack(CHANNEL)]);
    assert_eq!(rig.take(), Vec::<String>::new());
    // No SoundFont brings the patch selector back.
    clear_soundfont(&mut rig);
    rig.frame(vec![]);
    assert!(shows(&rig, "Load Patch") && !shows(&rig, "Not a SoundFont file"));

    // A song naming a SoundFont that is not there.
    let module = rig.app.song.channels[CHANNEL].rack.get_mut(PLAYER).unwrap();
    let gone = in_my_soundfonts("Gone.sf2", 3).unwrap();
    module.file.custom_data = Some(gone.write(None).unwrap());
    rig.app.song_edited(SongEdit::Final, 0);
    rig.frame(vec![]);
    rig.frame(vec![]);
    assert!(shows(&rig, "SoundFont file not found"));
    // Once it is there, a pick shows its patches, not what the rack read before.
    let strings = soundfont(&[("Strings", 48, 0)]);
    (rig.app.files).keep_user_file(Path::new("My SoundFonts/Gone.sf2"), strings);
    clear_soundfont(&mut rig);
    rig.frame(vec![]);
    rig.click_id(control(PATCH));
    // My Files, `Gone` (after `Broken`), its patch.
    for row in [0, 1, 0] {
        picker_row(&mut rig, row);
    }
    assert_eq!(sf2d(&rig), in_my_soundfonts("Gone.sf2", 0));
    rig.frame(vec![]);
    assert!(shows(&rig, "Strings") && !shows(&rig, "SoundFont file not found"));
}

#[test]
fn a_repick_tries_a_missing_soundfont_again_and_the_next_engine_has_it() {
    // A SoundFont missing at a build is not read again on each rebuild; picking it again (the same file and
    // patch) reads it, as a sample pick does.
    let mut rig = with_soundfonts();
    let sine = in_my_soundfonts("Sine.sf2", 0).unwrap();
    let module = rig.app.song.channels[CHANNEL].rack.get_mut(PLAYER).unwrap();
    module.file.custom_data = Some(sine.write(None).unwrap());
    rig.app.song_edited(SongEdit::Final, 0);
    rig.frame(vec![]);
    rig.frame(vec![]);
    assert!(rig.app.audio.samples.soundfont(&sine.path).is_none());
    // The file arrives (the crate's looped sine bank); a rebuild alone does not read it again.
    let bytes = include_bytes!("../../../crates/audio/tests/fixtures/sine.sf2");
    (rig.app.files).keep_user_file(Path::new("My SoundFonts/Sine.sf2"), bytes.to_vec());
    rig.app.song_edited(SongEdit::Final, 0);
    assert!(rig.app.audio.samples.soundfont(&sine.path).is_none());
    rig.frame(vec![]);
    rig.take();
    // The panel shows the error, a tap on which opens the picker: My Files, `Sine` (after `Broken` and `Piano`), its
    // patch.
    rig.click(text_rect(&rig, "SoundFont file not found").center());
    for row in [0, 2, 0] {
        picker_row(&mut rig, row);
    }
    assert_eq!(sf2d(&rig), Some(sine.clone()));
    assert_eq!(rig.take(), ["ReplaceSong adopting no-resync"]);
    assert!(rig.app.audio.samples.soundfont(&sine.path).is_some());
}

#[test]
fn the_soundfont_picker_follows_its_channel_when_an_earlier_one_comes_back() {
    // As the preset picker: players A (2) and B (3). A goes, B's spinner opens the picker on 2, and Ctrl+Z
    // brings A back there: the picker moves with B to 3, and its pick is B's.
    let mut rig = with_soundfonts();
    rig.app
        .add_channel(ChannelKind::Synth(SynthKind::SoundFontPlayer), 0);
    rig.app.song.delete_channel(CHANNEL);
    rig.app.song_edited(SongEdit::Final, 0);
    rig.app.navigate(Nav::Open(Overlay::Rack(CHANNEL)));
    rig.frame(vec![]);
    rig.frame(vec![]);
    rig.click_id(control(PATCH));
    let picker = |channel| Overlay::SoundFontPicker {
        channel,
        module: PLAYER,
    };
    assert_eq!(rig.overlays(), [Overlay::Rack(2), picker(2)]);
    rig.undo_under_modal();
    assert_eq!(rig.overlays(), [Overlay::Rack(3), picker(3)]);
    // My Files, `Piano` (after `Broken`), its second patch.
    for row in [0, 1, 1] {
        picker_row(&mut rig, row);
    }
    let sf2d = |rig: &Rig, c: usize| {
        let module = rig.app.song.find_module(c, PLAYER).unwrap();
        module.file.custom_data.as_deref().and_then(Sf2d::read)
    };
    assert_eq!(sf2d(&rig, 2), None);
    assert_eq!(sf2d(&rig, 3), in_my_soundfonts("Piano.sf2", 1));
}

/// The preset button of the player's card.
fn preset_button() -> Id {
    Id::new(("rack", CHANNEL))
        .with(("rack card", PLAYER))
        .with("preset")
}

fn player_module(rig: &Rig) -> ModuleInstance {
    rig.app.song.find_module(CHANNEL, PLAYER).unwrap().clone()
}

#[test]
fn the_preset_button_lists_default_presets_and_soundfonts_and_a_picked_sf2_loads_through_the_soundfont_loader()
 {
    // Picking a preset: one picker; the file goes to the module's own loader first (an `.sf2`:
    // `SF2D`, its path at patch 0), else it loads as a generic module preset. `Default` is the init preset.
    let mut rig = with_soundfonts();
    let presets = rig.app.files.presets_dir(52).unwrap();
    let mut warm = ModuleInstance::new(52, 0);
    warm.params[3] = 0.4;
    let bytes = crate::flm::write_preset(&warm).unwrap();
    (rig.app.files).keep_user_file(&presets.join("Warm.flmpst"), bytes);
    let target = PresetTarget::Module(PLAYER);
    let picker = Overlay::PresetPicker {
        channel: CHANNEL,
        target,
    };

    // The root: `My Files`, `My SoundFonts`, `Default`. A SoundFont in `My SoundFonts` (after `Broken`).
    rig.click_id(preset_button());
    assert_eq!(rig.overlays(), [Overlay::Rack(CHANNEL), picker.clone()]);
    for _ in 0..3 {
        rig.frame(vec![]);
    }
    for name in ["My Files", "My SoundFonts", "Default"] {
        assert!(shows(&rig, name), "{name}");
    }
    picker_row(&mut rig, 1);
    for _ in 0..3 {
        rig.frame(vec![]);
    }
    for name in ["Broken", "Piano"] {
        assert!(shows(&rig, name), "{name}");
    }
    picker_row(&mut rig, 1);
    // Loaded on a worker: the pick applies on a later frame. One final edit: `SF2D` at patch 0, the module named
    // for the file, and the patches the card lists.
    worker_done(&mut rig);
    assert_eq!(rig.overlays(), [Overlay::Rack(CHANNEL), picker.clone()]);
    assert_eq!(sf2d(&rig), in_my_soundfonts("Piano.sf2", 0));
    assert_eq!(player_module(&rig).file.preset_name, "Piano");
    assert_eq!(rig.take(), ["ReplaceSong adopting no-resync"]);
    close_picker(&mut rig);
    assert!(shows(&rig, "Upright") && !shows(&rig, "Load Patch"));
    // The patch is picked from the spinner's picker, open in the SoundFont's patches.
    rig.click_id(control(PATCH));
    picker_row(&mut rig, 1);
    assert_eq!(sf2d(&rig), in_my_soundfonts("Piano.sf2", 1));
    assert_eq!(rig.take(), ["ReplaceSong adopting no-resync"]);
    close_picker(&mut rig);

    // `My Files`, `Warm`: a generic preset, all of it loaded (the SoundFont goes with the old state).
    rig.click_id(preset_button());
    picker_row(&mut rig, 0);
    picker_row(&mut rig, 0);
    worker_done(&mut rig);
    assert_eq!(rig.overlays(), [Overlay::Rack(CHANNEL), picker]);
    assert_eq!(player_module(&rig).params[3], 0.4);
    assert_eq!(
        (player_module(&rig).file.preset_name.as_str(), sf2d(&rig)),
        ("Warm", None)
    );
    assert_eq!(rig.take(), ["ReplaceSong adopting no-resync"]);

    // `Default`: the init preset.
    close_picker(&mut rig);
    rig.click_id(preset_button());
    picker_row(&mut rig, 2);
    assert_eq!(player_module(&rig), ModuleInstance::new(52, PLAYER));
    assert_eq!(rig.take(), ["ReplaceSong adopting no-resync"]);
    // The picks make no undo step, nor does the patch's pick: an undo takes back the player's add.
    close_picker(&mut rig);
    let n = rig.app.song.channels.len();
    rig.frame(z(egui::Modifiers::NONE));
    assert_eq!(rig.app.song.channels.len(), n - 1);
}

#[test]
fn an_sf2_the_loader_rejects_shows_the_load_error_and_changes_nothing() {
    // The module's loader rejects it and it is no module preset either: `Failed to load project file`, with the
    // SoundFont's own error, and the song as it was.
    let mut rig = with_soundfonts();
    let song = rig.app.song.clone();
    rig.click_id(preset_button());
    picker_row(&mut rig, 1);
    // `Broken` (the first file).
    picker_row(&mut rig, 0);
    worker_done(&mut rig);
    let message = rig.overlays().into_iter().find_map(|o| match o {
        Overlay::Message { lines, .. } => Some(lines),
        _ => None,
    });
    let want = [crate::project_files::LOAD_FAILED, "Not a SoundFont file"];
    assert_eq!(message.as_deref(), Some(&want.map(String::from)[..]));
    assert_eq!(rig.app.song, song);
    assert_eq!(rig.take(), Vec::<String>::new());
}

#[test]
fn a_soundfont_cut_off_in_its_samples_shows_the_error_and_the_engine_stays_silent() {
    // The patch headers are whole, so the rack once listed the patches while the engine, which parses the whole file,
    // had no SoundFont and played silence with nothing on the panel.
    let mut rig = with_soundfonts();
    let cut = crate::sf2::tests::cut_off_samples(&[("Upright", 0, 0), ("Grand", 1, 0)]);
    (rig.app.files).keep_user_file(Path::new("My SoundFonts/Cut.sf2"), cut);
    rig.click_id(control(PATCH));
    // My Files, `Cut` (after `Broken`).
    for row in [0, 1] {
        picker_row(&mut rig, row);
    }
    let want = in_my_soundfonts("Cut.sf2", 0);
    assert_eq!(sf2d(&rig), want);
    assert_eq!(rig.take(), ["ReplaceSong adopting no-resync"]);
    rig.frame(vec![]);
    // The panel shows the error in place of the controls, not the patch list.
    assert!(shows(&rig, "Corrupted SoundFont file"));
    assert!(!shows(&rig, "Upright") && !shows(&rig, "Prev") && !shows(&rig, "Load Patch"));
    // The engine has no SoundFont there: it builds, plays nothing, and the load message counts the file as missing.
    let path = want.unwrap().path;
    assert!(rig.app.audio.samples.soundfont(&path).is_none());
    let key = crate::sample_cache::SampleKey::File(path);
    assert!(rig.app.audio.samples.missing().any(|k| *k == key));
    // A tap on the error opens the picker, as for any other error; Escape closes it.
    rig.click(text_rect(&rig, "Corrupted SoundFont file").center());
    let picker = Overlay::SoundFontPicker {
        channel: CHANNEL,
        module: PLAYER,
    };
    assert_eq!(rig.overlays(), [Overlay::Rack(CHANNEL), picker]);
    rig.frame(escape());
    // A song that names it opens in the same state, with no pick.
    clear_soundfont(&mut rig);
    rig.frame(vec![]);
    assert!(shows(&rig, "Load Patch"));
    let module = rig.app.song.channels[CHANNEL].rack.get_mut(PLAYER).unwrap();
    module.file.custom_data = Some(in_my_soundfonts("Cut.sf2", 1).unwrap().write(None).unwrap());
    rig.app.song_edited(SongEdit::Final, 0);
    rig.frame(vec![]);
    rig.frame(vec![]);
    assert!(shows(&rig, "Corrupted SoundFont file"));
}
