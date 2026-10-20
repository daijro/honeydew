//! The navigation scaffold's corners: a note held across a view or overlay change, the shortcuts
//! under the Save dialog, a message over the dialog, and tagged answers between two askers.

use eframe::egui::{Key, Modifiers};

use super::*;
use crate::project_files::{FileProvider, NEWER_TAG, Pick};

/// A rig with MiniSynth (1) selected and Drums (2), a clip on the MiniSynth, its audio started and its log empty.
fn synth_and_drums() -> Rig {
    let mut rig = Rig::with_synth();
    rig.app.song.channels[1].lanes[0].add_clip(0, 4.0);
    rig.app.add_default_drums(0);
    rig.app.selected_channel = 1;
    rig.frame(vec![]);
    rig.take();
    rig
}

#[test]
fn a_note_held_while_the_view_changes_or_an_overlay_opens_gets_its_note_off() {
    let held = ["NoteOn 1 52 108", "NoteOff 1 52"];
    // The current MiniSynth's own editor keeps its keyboard (an editor follows the current track): the note
    // sounds until its release.
    let mut rig = synth_and_drums();
    rig.press(E4);
    rig.edit(1);
    rig.slide_split();
    assert_eq!(rig.take(), held[..1], "piano roll");
    rig.release(E4);
    assert_eq!(rig.take(), held[1..], "piano roll");
    // The Drums channel made current puts its pads in the keyboard's place: the note stops with the switch.
    rig.press(E4);
    rig.app.selected_channel = 2;
    rig.frame(vec![]);
    assert_eq!(rig.take(), held, "pads");
    rig.release(E4);
    assert_eq!(rig.take(), Vec::<String>::new());
    // Elsewhere the keyboard stays, and the release stops the note.
    let message = Overlay::Message {
        tag: "notice",
        lines: vec!["Saved.".into()],
        confirm: None,
        extra: None,
    };
    let dialog = Overlay::SaveDialog {
        name: "Song".into(),
    };
    for change in [
        Nav::Show(View::Mixer),
        Nav::Edit(1),
        Nav::Open(Overlay::Rack(1)),
        Nav::Open(Overlay::MainMenu(MenuTab::Songs)),
        Nav::Open(message),
        Nav::Open(dialog),
    ] {
        let mut rig = synth_and_drums();
        rig.press(E4);
        rig.app.navigate(change.clone());
        rig.frame(vec![]);
        rig.release(E4);
        rig.frame(vec![]);
        assert_eq!(rig.take(), held, "{change:?}");
    }
}

#[test]
fn the_shortcuts_do_nothing_while_the_save_dialog_is_open() {
    let (mut rig, mem) = rig_with_file("/songs/Beat.flm", &synthetic_file(1, FORMAT_VERSION, ""));
    // An edit to undo, which also leaves changes that Ctrl+O would ask about.
    rig.app.add_default_drums(0);
    command(&mut rig, Key::S);
    let Some(Overlay::SaveDialog { name }) = rig.app.nav.overlays.last_mut() else {
        panic!("no dialog");
    };
    *name = "Typed".into();
    rig.frame(vec![]);
    let typed = [Overlay::SaveDialog {
        name: "Typed".into(),
    }];
    // Ctrl+S would offer the dialog again, with its name; Ctrl+O would ask about the unsaved changes.
    command(&mut rig, Key::S);
    command(&mut rig, Key::O);
    assert_eq!(rig.overlays(), typed);
    let channels = rig.app.song.channels.len();
    for modifiers in [Modifiers::COMMAND, Modifiers::COMMAND | Modifiers::SHIFT] {
        rig.frame(key_tap(Key::Z, modifiers));
        assert_eq!(rig.app.song.channels.len(), channels, "{modifiers:?}");
    }
    assert!(matches!(rig.overlays()[..], [Overlay::SaveDialog { .. }]));
    assert!(mem.0.borrow().saves.is_empty());
    rig.frame(escape());
    // Without the dialog they work again, once its field has let go of the focus.
    rig.frame(vec![]);
    rig.frame(key_tap(Key::Z, Modifiers::COMMAND));
    assert_eq!(rig.app.song.channels.len(), channels - 1);
}

#[test]
fn a_message_raised_with_the_save_dialog_open_is_on_top_and_escape_closes_only_it() {
    let (mut rig, mem) = rig_with_file("/songs/Bad.flm", b"JUNK");
    let message_over_dialog = |rig: &Rig| {
        matches!(
            rig.overlays()[..],
            [Overlay::SaveDialog { .. }, Overlay::Message { .. }]
        )
    };
    // The dialog is open when a pick lands (the web's file picker answers later) and fails to load.
    command(&mut rig, Key::S);
    rig.frame(vec![]);
    mem.clone().open(Pick::Song.accept());
    rig.frame(vec![]);
    assert!(message_over_dialog(&rig), "{:?}", rig.overlays());
    rig.frame(escape());
    assert!(matches!(rig.overlays()[..], [Overlay::SaveDialog { .. }]));
    rig.frame(escape());
    assert_eq!(rig.overlays(), []);
    // Both in one frame: Ctrl+O, whose pick fails at once, and Ctrl+S on the untitled song.
    rig.frame(vec![
        key_event(Key::O, Modifiers::COMMAND),
        key_event(Key::S, Modifiers::COMMAND),
    ]);
    assert!(message_over_dialog(&rig), "{:?}", rig.overlays());
    rig.frame(vec![]);
    rig.frame(escape());
    assert!(matches!(rig.overlays()[..], [Overlay::SaveDialog { .. }]));
}

#[test]
fn a_tagged_answer_reaches_its_own_asker_only() {
    let other = |rig: &mut Rig| {
        rig.app.navigate(Nav::Open(Overlay::Message {
            tag: "other",
            lines: vec!["Go on?".into()],
            confirm: Some(("Yes".into(), "No".into())),
            extra: None,
        }));
        rig.frame(vec![]);
    };
    let (mut rig, mem) = rig_with_file("/songs/New.flm", &synthetic_file(2, "9.0", ""));
    // The newer-version question is replaced by another asker's: the open reads no and drops the project, and
    // that asker's Yes stays its own.
    command(&mut rig, Key::O);
    other(&mut rig);
    assert_eq!(rig.app.files.waiting(), None);
    rig.click_id(message_item(1));
    // Polled for the newer-version question, the other asker's stored Yes reads no and stays for its asker.
    assert_eq!(rig.app.nav.take_answer(NEWER_TAG), Some(false));
    assert_eq!(rig.app.nav.take_answer("other"), Some(true));
    assert_eq!(rig.app.song.channels.len(), 2, "loaded");
    // The other way round: a pick that lands under the other question replaces it with the newer-version one,
    // whose Yes loads the project and is no answer to the other asker.
    other(&mut rig);
    mem.clone().open(Pick::Song.accept());
    rig.frame(vec![]);
    assert!(matches!(
        rig.overlays()[..],
        [Overlay::Message { tag: NEWER_TAG, .. }]
    ));
    rig.frame(vec![]);
    rig.click_id(message_item(3));
    assert_eq!(rig.app.song.channels.len(), 3);
    assert_eq!(rig.app.nav.take_answer("other"), Some(false));
}

#[test]
fn the_wheel_scrolls_no_playlist_under_the_main_menu_or_the_rack() {
    let mut rig = Rig::new();
    for _ in 0..8 {
        rig.app.add_default_drums(0);
    }
    rig.frame(vec![]);
    // Left of the menu panel and of the rack, over the track rows.
    let rows = pos2(300.0, 200.0);
    rig.app
        .navigate(Nav::Open(Overlay::MainMenu(MenuTab::Settings)));
    rig.frame(vec![]);
    // Over the menu's panel, and beside it.
    rig.wheel(pos2(800.0, 200.0), -500.0);
    rig.wheel(rows, -500.0);
    assert_eq!(rig.app.main_screen.scroll_y, 0.0);
    // The rack covers the right half only: the wheel over it scrolls the rack alone, beside it the rows.
    rig.app.navigate(Nav::Show(View::Playlist));
    rig.app.navigate(Nav::Open(Overlay::Rack(1)));
    rig.frame(vec![]);
    rig.wheel(pos2(800.0, 200.0), -500.0);
    assert_eq!(rig.app.main_screen.scroll_y, 0.0);
    rig.wheel(rows, -500.0);
    assert!(rig.app.main_screen.scroll_y > 0.0);
}

#[test]
fn a_pinch_or_ctrl_wheel_zooms_no_playlist_under_the_main_menu() {
    let mut rig = Rig::with_synth();
    let zoom = |rig: &mut Rig| {
        let at = pos2(300.0, 100.0);
        rig.frame(vec![egui::Event::PointerMoved(at), egui::Event::Zoom(1.5)]);
        rig.frame(vec![]);
        rig.app.main_screen.zoom
    };
    rig.app
        .navigate(Nav::Open(Overlay::MainMenu(MenuTab::Settings)));
    rig.frame(vec![]);
    assert_eq!(zoom(&mut rig), 0.0);
    rig.app.nav.overlays.clear();
    rig.frame(vec![]);
    assert_ne!(zoom(&mut rig), 0.0);
}

#[test]
fn the_keyboard_settings_cover_nothing_that_scrolls_and_the_tempo_dialog_covers_the_screen() {
    let mut rig = Rig::new();
    for _ in 0..8 {
        rig.app.add_default_drums(0);
    }
    rig.frame(vec![]);
    // The keyboard settings sit in the transport bar: the rows above still scroll.
    rig.app.navigate(Nav::Open(Overlay::KeyboardSettings));
    rig.frame(vec![]);
    rig.wheel(pos2(300.0, 200.0), -100.0);
    let scrolled = rig.app.main_screen.scroll_y;
    assert!(scrolled > 0.0);
    // The tempo dialog is modal over the whole screen (its band is y 222..318): the wheel scrolls nothing under it,
    // over the band or above it.
    rig.app.navigate(Nav::Open(Overlay::TempoEditor));
    rig.frame(vec![]);
    rig.wheel(pos2(300.0, 270.0), -100.0);
    rig.wheel(pos2(300.0, 100.0), -100.0);
    assert_eq!(rig.app.main_screen.scroll_y, scrolled);
    // Closed, the rows scroll again.
    rig.app.nav.overlays.clear();
    rig.frame(vec![]);
    rig.wheel(pos2(300.0, 100.0), -100.0);
    assert!(rig.app.main_screen.scroll_y > scrolled);
}

#[test]
fn the_save_and_tempo_dialogs_dim_the_whole_app_screen_and_paint_their_band_as_the_floating_panel()
{
    // Dialog boxes, through the app: the screen dimmed at α 0.6 over everything the app drew, then the band as the
    // floating panel: the Save dialog's 20 U × 7 U band (y 158..382) and the tempo dialog's 3 U band (y 222..318).
    let colors = crate::theme::theme_by_id(0).colors;
    let screen = egui::Rect::from_min_size(pos2(0.0, 0.0), egui::vec2(1170.0, 540.0));
    let band = |top, bottom| egui::Rect::from_x_y_ranges(0.0..=1170.0, top..=bottom);
    let dialogs = [
        (
            Overlay::SaveDialog {
                name: "Song".into(),
            },
            egui::Rect::from_x_y_ranges(265.0..=905.0, 158.0..=382.0),
        ),
        (Overlay::TempoEditor, band(222.0, 318.0)),
    ];
    let m = crate::metrics::Metrics::from_pixels_per_point(1.0, crate::metrics::DEFAULT_UI_SCALE);
    let panel_top = crate::ui::main_screen::Regions::new(screen, &m)
        .transport
        .top();
    for (overlay, band) in dialogs {
        let mut rig = Rig::with_synth();
        rig.app.navigate(Nav::Open(overlay.clone()));
        rig.frame(vec![]);
        let clipped = crate::ui::dialog::painted_clipped(&rig.out.shapes);
        let painted: Vec<_> = clipped.iter().map(|&(r, f, _)| (r, f)).collect();
        crate::ui::dialog::assert_base(&painted, &colors, screen, band);
        let dim = (colors.background.to_color32().gamma_multiply(0.6), screen);
        let dim_at = painted
            .iter()
            .position(|&(r, f)| (f, r) == dim)
            .expect("the dim");
        // The dim is not clipped short of the screen (the app's panel clips its own children, not the dim).
        assert!(
            clipped[dim_at].2.contains_rect(screen),
            "{overlay:?}: clip {:?}",
            clipped[dim_at].2
        );
        // Everything drawn before it is under it, the transport bar and keyboard (from `panel_top` down) included; after it only the
        // dialog paints, inside its band.
        assert!(
            painted[..dim_at].iter().any(|&(r, _)| r.top() >= panel_top),
            "{overlay:?}: the panel is drawn under the dim"
        );
        for &(r, _) in &painted[dim_at + 1..] {
            assert!(
                band.contains_rect(r),
                "{overlay:?}: {r:?} drawn over the dim, outside the band"
            );
        }
    }
}

#[test]
fn a_pinch_or_ctrl_wheel_zooms_no_playlist_under_the_tempo_dialog() {
    let mut rig = Rig::with_synth();
    let zoom = |rig: &mut Rig| {
        // Over the playlist, above the dialog's band.
        let at = pos2(300.0, 100.0);
        rig.frame(vec![egui::Event::PointerMoved(at), egui::Event::Zoom(1.5)]);
        rig.frame(vec![]);
        rig.app.main_screen.zoom
    };
    rig.app.navigate(Nav::Open(Overlay::TempoEditor));
    rig.frame(vec![]);
    assert_eq!(zoom(&mut rig), 0.0);
    rig.app.nav.overlays.clear();
    rig.frame(vec![]);
    assert_ne!(zoom(&mut rig), 0.0);
}

#[test]
fn the_wheel_scrolls_no_mixer_under_the_main_menu() {
    let mut rig = Rig::with_synth();
    for _ in 0..12 {
        rig.app.add_default_drums(0);
    }
    rig.app.navigate(Nav::Show(View::Mixer));
    rig.frame(vec![]);
    rig.app
        .navigate(Nav::Open(Overlay::MainMenu(MenuTab::Settings)));
    rig.frame(vec![]);
    rig.wheel(pos2(300.0, 200.0), -800.0);
    assert_eq!(rig.app.slots.mixer.scroll, 0.0);
    rig.app.nav.overlays.clear();
    rig.frame(vec![]);
    rig.wheel(pos2(300.0, 200.0), -800.0);
    assert!(rig.app.slots.mixer.scroll > 0.0);
}

#[test]
fn undo_and_redo_wait_while_a_message_or_a_popup_is_open() {
    // An edit to undo (the Drums channel), and nothing holding the keyboard.
    let mut rig = synth_and_drums();
    let channels = rig.app.song.channels.len();
    let z = |rig: &mut Rig, modifiers| rig.frame(key_tap(Key::Z, modifiers));
    // An unfocused message: no text field has the keyboard, yet neither key acts under it.
    rig.app.navigate(Nav::Open(Overlay::Message {
        tag: "test",
        lines: vec!["A message".into()],
        confirm: None,
        extra: None,
    }));
    rig.frame(vec![]);
    for modifiers in [Modifiers::COMMAND, Modifiers::COMMAND | Modifiers::SHIFT] {
        z(&mut rig, modifiers);
        assert_eq!(rig.app.song.channels.len(), channels, "{modifiers:?}");
    }
    assert!(matches!(rig.overlays()[..], [Overlay::Message { .. }]));
    rig.frame(escape());
    rig.frame(vec![]);
    // A popup a view shows itself (a track header's menu) holds them too.
    rig.secondary_click(header(1));
    rig.frame(vec![]);
    z(&mut rig, Modifiers::COMMAND);
    assert_eq!(
        rig.app.song.channels.len(),
        channels,
        "under the track menu"
    );
    rig.frame(escape());
    rig.frame(vec![]);
    // With nothing open, they act again.
    z(&mut rig, Modifiers::COMMAND);
    assert_eq!(rig.app.song.channels.len(), channels - 1);
    z(&mut rig, Modifiers::COMMAND | Modifiers::SHIFT);
    assert_eq!(rig.app.song.channels.len(), channels);
}

#[test]
fn every_modal_marks_its_frames_and_holds_undo_under_it() {
    // Each modal's frames are marked (`ui::backdrop`, or the main menu's and tempo dialog's own), the overlays here
    // and the views' own popups alike; none of them lets Ctrl+Z act. The rack's level dialog and the step view's
    // backdrop are in `rack_wiring` and `editor_wiring`.
    use crate::ui::modules_browser::InsertAt;
    use crate::ui::rack::PresetTarget;
    use crate::ui::views::MenuTab;
    let mut rig = synth_and_drums();
    let channels = rig.app.song.channels.len();
    let modals = [
        Overlay::Message {
            tag: "test",
            lines: vec!["A message".into()],
            confirm: None,
            extra: None,
        },
        Overlay::SaveDialog {
            name: "Song".into(),
        },
        Overlay::ChannelSettings {
            channel: 1,
            name: "Name".into(),
        },
        Overlay::ModulesBrowser {
            channel: 1,
            at: InsertAt::End,
        },
        Overlay::PresetPicker {
            channel: 2,
            target: PresetTarget::Drumset,
        },
        Overlay::MainMenu(MenuTab::Songs),
        Overlay::CtrlMenu,
        Overlay::TempoEditor,
    ];
    for modal in modals {
        rig.app.navigate(Nav::Open(modal.clone()));
        rig.frame(vec![]);
        rig.frame(vec![]);
        assert!(crate::ui::modal_was_open(&rig.ctx), "{modal:?}");
        rig.frame(key_tap(Key::Z, Modifiers::COMMAND));
        assert_eq!(rig.app.song.channels.len(), channels, "{modal:?}");
        rig.app.nav.overlays.clear();
        rig.frame(vec![]);
        rig.frame(vec![]);
        assert!(!crate::ui::modal_was_open(&rig.ctx), "{modal:?} closed");
    }
    rig.frame(key_tap(Key::Z, Modifiers::COMMAND));
    assert_eq!(rig.app.song.channels.len(), channels - 1);
}

#[test]
fn a_held_escape_closes_one_level_and_its_repeats_close_nothing() {
    // Key event routing: a press of a key already held is ignored.
    let (mut rig, mem) = rig_with_file("/songs/Bad.flm", b"JUNK");
    command(&mut rig, Key::S);
    rig.frame(vec![]);
    mem.clone().open(Pick::Song.accept());
    rig.frame(vec![]);
    let dialog = |rig: &Rig| matches!(rig.overlays()[..], [Overlay::SaveDialog { .. }]);
    let [press, release] = <[_; 2]>::try_from(escape()).unwrap();
    rig.frame(vec![press.clone()]);
    assert!(dialog(&rig), "{:?}", rig.overlays());
    // Still held: the OS's auto-repeats (egui marks a press of a held key a repeat).
    for _ in 0..3 {
        rig.frame(vec![press.clone()]);
        assert!(dialog(&rig), "{:?}", rig.overlays());
    }
    rig.frame(vec![release]);
    rig.frame(vec![press]);
    assert_eq!(rig.overlays(), []);
}
