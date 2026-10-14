//! The keymap's global actions on their default keys (`app/keys.rs`), and a user keymap remapping them.

use super::*;
use crate::app::keys::{self, Global, Panel};
use crate::keymap;
use crate::project_files::content::tests::{Temp, put};
use egui::{Key, Modifiers};

const NONE: Modifiers = Modifiers::NONE;
const CMD: Modifiers = Modifiers::COMMAND;

impl Rig {
    /// A press and its release.
    fn key(&mut self, key: Key, modifiers: Modifiers) {
        let release = egui::Event::Key {
            key,
            physical_key: None,
            pressed: false,
            repeat: false,
            modifiers,
        };
        self.frame(vec![key_event(key, modifiers), release]);
    }

    /// The commands sent since the last call, but the metronome's.
    fn sent(&mut self) -> Vec<String> {
        let mut log = self.take();
        log.retain(|c| !c.starts_with("Metronome"));
        log
    }
}

/// [`Rig::with_synth`] with a 16-beat clip, no count-in.
fn rig() -> Rig {
    let mut rig = Rig::with_synth();
    rig.app.settings.count_in = false;
    rig.app.song.channels[1].lanes[0].add_clip(0, 16.0);
    rig.frame(vec![]);
    rig.take();
    rig
}

#[test]
fn space_plays_and_pauses_as_the_play_button_does() {
    let mut rig = rig();
    rig.key(Key::Space, NONE);
    assert!(rig.app.main_screen.playing);
    rig.key(Key::Space, NONE);
    assert!(!rig.app.main_screen.playing);
    assert_eq!(rig.sent(), ["Play", "Stop"]);
}

#[test]
fn ctrl_left_is_rev_and_alt_r_arms_without_playing() {
    let mut rig = rig();
    rig.rec.0.borrow_mut().tick = 4 * 128;
    rig.key(Key::Space, NONE);
    rig.rec.0.borrow_mut().tick = 9 * 128;
    rig.key(Key::ArrowLeft, CMD);
    assert!(!rig.app.main_screen.playing);
    assert_eq!(rig.sent(), ["Play", "Stop", "Seek 512"]);
    rig.key(Key::R, Modifiers::ALT);
    assert!(rig.app.main_screen.armed);
    assert!(!rig.app.main_screen.playing, "arming never starts playback");
    rig.key(Key::R, Modifiers::ALT);
    assert!(!rig.app.main_screen.armed);
}

#[test]
fn f8_and_f10_open_the_main_menu_on_songs_and_settings_and_never_close_it() {
    let mut rig = Rig::new();
    rig.key(Key::F8, NONE);
    assert_eq!(rig.overlays(), [Overlay::MainMenu(MenuTab::Songs)]);
    rig.frame(vec![]);
    rig.key(Key::F10, NONE);
    assert_eq!(rig.overlays(), [Overlay::MainMenu(MenuTab::Settings)]);
    rig.key(Key::F10, NONE);
    assert_eq!(rig.overlays(), [Overlay::MainMenu(MenuTab::Settings)]);
    rig.key(Key::F8, NONE);
    assert_eq!(rig.overlays(), [Overlay::MainMenu(MenuTab::Songs)]);
}

#[test]
fn f6_f9_and_ctrl_k_show_and_hide_the_rack_the_mixer_and_the_keyboard_panel() {
    let mut rig = Rig::with_drums_and_synth();
    rig.key(Key::F6, NONE);
    assert_eq!(rig.overlays(), [Overlay::Rack(2)]);
    rig.slide_rack();
    rig.key(Key::F6, NONE);
    assert_eq!(rig.overlays(), []);
    rig.key(Key::F9, NONE);
    assert_eq!(rig.app.nav.view, View::Mixer);
    rig.key(Key::F9, NONE);
    assert_eq!(rig.app.nav.view, View::Playlist);
    // The keyboard panel shows already: the first press focuses it, the second hides it (three-way).
    rig.key(Key::K, CMD);
    assert!(!rig.app.main_screen.panel_hidden);
    let transport = egui::Id::new(crate::ui::focus::TRANSPORT);
    assert!(crate::ui::focus::within(&rig.ctx, transport));
    rig.key(Key::K, CMD);
    assert!(rig.app.main_screen.panel_hidden);
    assert_eq!(
        crate::ui::focus::focused(&rig.ctx),
        None,
        "back to the root"
    );
    rig.key(Key::K, CMD);
    assert!(!rig.app.main_screen.panel_hidden);
}

#[test]
fn a_shown_panel_whose_focus_is_elsewhere_takes_the_focus_before_it_hides() {
    let mut rig = Rig::with_drums_and_synth();
    rig.key(Key::F6, NONE);
    rig.slide_rack();
    let rack = egui::Id::new(crate::ui::focus::RACK);
    assert!(crate::ui::focus::within(&rig.ctx, rack));
    // The focus goes to the playlist (a press there would do so too): F6 brings it back, then hides the rack.
    crate::ui::focus::focus(&rig.ctx, egui::Id::new(crate::ui::focus::PLAYLIST));
    rig.frame(vec![]);
    rig.key(Key::F6, NONE);
    assert!(crate::ui::focus::within(&rig.ctx, rack));
    assert_eq!(rig.overlays(), [Overlay::Rack(2)]);
    rig.key(Key::F6, NONE);
    assert_eq!(rig.overlays(), []);
}

#[test]
fn enter_kbd_focus_toggles_the_focus_on_the_keyboard_panel() {
    let mut rig = Rig::new();
    let ctx = rig.ctx.clone();
    let transport = egui::Id::new(crate::ui::focus::TRANSPORT);
    rig.app
        .global(&ctx, Global::KbdFocus, &mut false, &mut Vec::new());
    rig.frame(vec![]);
    assert!(crate::ui::focus::within(&ctx, transport));
    rig.app
        .global(&ctx, Global::KbdFocus, &mut false, &mut Vec::new());
    rig.frame(vec![]);
    assert_eq!(crate::ui::focus::focused(&ctx), None);
}

#[test]
fn shift_ctrl_t_opens_the_tempo_dialog_and_closes_it_from_on_top() {
    let mut rig = Rig::new();
    rig.key(Key::T, CMD | Modifiers::SHIFT);
    assert_eq!(rig.overlays(), [Overlay::TempoEditor]);
    // The dialog is modal: the other global actions wait under it, its own toggle closes it.
    rig.frame(vec![]);
    rig.key(Key::F9, NONE);
    assert_eq!(rig.app.nav.view, View::Playlist);
    rig.key(Key::T, CMD | Modifiers::SHIFT);
    assert_eq!(rig.overlays(), []);
}

#[test]
fn focus_editor_leaves_the_mixer_and_opens_no_editor() {
    let mut rig = Rig::with_drums_and_synth();
    rig.key(Key::F9, NONE);
    // No default key (the gamepad's RB+up only): the action as the frame applies it.
    let ctx = rig.ctx.clone();
    let mut navs = Vec::new();
    rig.app
        .global(&ctx, Global::FocusEditor, &mut false, &mut navs);
    assert_eq!(navs, [Nav::Show(View::Playlist)]);
    for nav in navs {
        rig.app.navigate(nav);
    }
    assert_eq!(rig.app.nav.view, View::Playlist);
    // With no bottom editor open it focuses the playlist's container, opening nothing.
    let mut navs = Vec::new();
    rig.app
        .global(&ctx, Global::FocusEditor, &mut false, &mut navs);
    assert_eq!(navs, []);
    rig.frame(vec![]);
    assert!(crate::ui::focus::within(
        &ctx,
        egui::Id::new(crate::ui::focus::PLAYLIST)
    ));
    assert!(!rig.app.focus_panel(&ctx, Panel::Editor), "inside already");
}

#[test]
fn the_global_actions_wait_under_a_message_and_while_a_text_field_has_the_keyboard() {
    let mut rig = rig();
    rig.app
        .navigate(crate::ui::views::main_menu::quit_question());
    rig.frame(vec![]);
    rig.key(Key::Space, NONE);
    rig.key(Key::F6, NONE);
    assert!(!rig.app.main_screen.playing);
    assert_eq!(rig.overlays().len(), 1, "only the message");
    // The Save dialog's name field has the keyboard: Space is a character there.
    rig.app.navigate(Nav::Close);
    command(&mut rig, Key::S);
    rig.frame(vec![]);
    assert!(rig.ctx.wants_keyboard_input());
    rig.frame(vec![
        key_event(Key::Space, NONE),
        egui::Event::Text(" ".into()),
    ]);
    assert!(!rig.app.main_screen.playing);
}

#[test]
fn a_user_keymap_moves_a_global_action_to_another_key() {
    // `play` on `2` and off Space, `showRack` on `;` (Android key tokens).
    let tmp = Temp::new("ka-user-keymap");
    let user = r#"{"modifiers":[],"actions":[
        {"key":"2","action":"play"},{"key":"space","action":""},{"key":";","action":"showRack"}]}"#;
    put(&tmp.0.join("keymap.json"), user.as_bytes());
    let (map, log) = keymap::load(egui::os::OperatingSystem::Windows, None, Some(&tmp.0));
    assert!(log.is_empty(), "{log:?}");
    let fired = |key| {
        let mut input = egui::InputState::default();
        input.events.push(key_event(key, NONE));
        keys::take(&map, &mut input, |_| true)
    };
    assert_eq!(fired(Key::Num2), [Global::Play]);
    assert_eq!(fired(Key::Space), []);
    assert_eq!(fired(Key::Semicolon), [Global::Panel(Panel::Rack)]);
    assert_eq!(fired(Key::F6), [Global::Panel(Panel::Rack)]);
}

#[test]
fn an_editors_keys_edit_through_the_funnel_one_undo_step_each_and_wait_under_an_overlay() {
    let note = |pos, key| crate::song::Event::Note {
        pos,
        len_beats: 1.0,
        key,
        velocity: crate::song::Event::DEFAULT_VELOCITY,
        pan: crate::song::Event::DEFAULT_PAN,
        slide: false,
    };
    let mut rig = Rig::with_synth();
    rig.app.song.channels[1].lanes[0].add_clip(0, 4.0).events = vec![note(0, 60), note(128, 62)];
    rig.app.song_edited(SongEdit::Final, 0);
    rig.edit(1);
    rig.frame(vec![]);
    let notes = |rig: &Rig| rig.app.song.channels[1].lanes[0].clips[0].events.len();
    // Under the CTRL menu (an overlay over the view), the grid does not hold the keyboard.
    rig.app.navigate(Nav::Open(Overlay::CtrlMenu));
    rig.frame(vec![]);
    rig.key(Key::A, CMD);
    rig.key(Key::Delete, NONE);
    assert_eq!(notes(&rig), 2);
    rig.app.navigate(Nav::Close);
    rig.frame(vec![]);
    rig.key(Key::A, CMD);
    rig.key(Key::Delete, NONE);
    assert_eq!(notes(&rig), 0);
    // One undo step brings both back.
    rig.key(Key::Z, CMD);
    assert_eq!(notes(&rig), 2);
}

#[test]
fn the_open_editor_keeps_its_keys_under_the_keyboard_settings_strip_and_the_rack() {
    // The focused control takes keys first: the strip covers nothing and the rack only its panel, as for the pointer
    // (`views::covered`), so the roll keeps its editing keys under them, and Escape deselects before it would close
    // either.
    let note = |pos, key| crate::song::Event::Note {
        pos,
        len_beats: 1.0,
        key,
        velocity: crate::song::Event::DEFAULT_VELOCITY,
        pan: crate::song::Event::DEFAULT_PAN,
        slide: false,
    };
    for overlay in [Overlay::KeyboardSettings, Overlay::Rack(1)] {
        let mut rig = Rig::with_synth();
        rig.app.song.channels[1].lanes[0].add_clip(0, 4.0).events =
            vec![note(0, 60), note(128, 62)];
        rig.app.song_edited(SongEdit::Final, 0);
        rig.edit(1);
        rig.app.navigate(Nav::Open(overlay.clone()));
        rig.slide_rack();
        let notes = |rig: &Rig| rig.app.song.channels[1].lanes[0].clips[0].events.len();
        // Ctrl+A, then Escape: deselected, and the overlay stays; Delete then deletes nothing.
        rig.key(Key::A, CMD);
        rig.key(Key::Escape, NONE);
        assert_eq!(rig.overlays(), std::slice::from_ref(&overlay));
        rig.key(Key::Delete, NONE);
        assert_eq!(notes(&rig), 2, "{overlay:?}");
        // Ctrl+A, Ctrl+C, Delete: both go; Enter adds a note at the grid cursor; Ctrl+V pastes the two.
        rig.key(Key::A, CMD);
        rig.key(Key::C, CMD);
        rig.key(Key::Delete, NONE);
        assert_eq!(notes(&rig), 0, "{overlay:?}");
        rig.key(Key::Enter, NONE);
        assert_eq!(notes(&rig), 1, "{overlay:?}");
        rig.key(Key::V, CMD);
        assert_eq!(notes(&rig), 3, "{overlay:?}");
        assert_eq!(rig.overlays(), [overlay]);
    }
}

#[test]
fn a_user_keymap_moves_close_for_the_dialogs_too() {
    // `close` (`Close dialog`, `Dismiss menu`): Escape unbound, Q bound.
    let tmp = Temp::new("ka-user-close");
    let user =
        r#"{"modifiers":[],"actions":[{"key":"escape","action":""},{"key":"q","action":"close"}]}"#;
    put(&tmp.0.join("keymap.json"), user.as_bytes());
    let (map, log) = keymap::load(egui::os::OperatingSystem::Windows, None, Some(&tmp.0));
    assert!(log.is_empty(), "{log:?}");
    let _map = keymap::tests::use_map(map);
    let mut rig = Rig::new();
    rig.app
        .navigate(crate::ui::views::main_menu::quit_question());
    rig.frame(vec![]);
    rig.key(Key::Escape, NONE);
    assert_eq!(rig.overlays().len(), 1, "Escape closes nothing now");
    rig.key(Key::Q, NONE);
    assert_eq!(rig.overlays(), []);
    // A popup over the view: the tempo dialog closes on Q too.
    rig.key(Key::T, CMD | Modifiers::SHIFT);
    rig.frame(vec![]);
    rig.key(Key::Q, NONE);
    assert_eq!(rig.overlays(), []);
}

#[test]
fn a_focused_text_field_keeps_space_from_play() {
    // The text-field gate on its own (a modal holds the global actions too): a field focused on the screen.
    let mut app = Rig::new().app;
    let ctx = egui::Context::default();
    let mut text = String::new();
    let mut frame = |app: &mut HoneydewApp, focus: bool, events: Vec<egui::Event>| {
        let (mut rewind, mut navs) = (false, Vec::new());
        let input = egui::RawInput {
            events,
            ..Default::default()
        };
        let _ = ctx.run(input, |ctx| {
            egui::CentralPanel::default().show(ctx, |ui| {
                let field = ui.text_edit_singleline(&mut text);
                if focus {
                    field.request_focus();
                } else {
                    field.surrender_focus();
                }
            });
            app.global_keys(ctx, &mut rewind, &mut navs);
        });
    };
    frame(&mut app, true, vec![]);
    frame(&mut app, true, key_tap(Key::Space, NONE));
    assert!(!app.main_screen.playing);
    frame(&mut app, false, vec![]);
    frame(&mut app, false, key_tap(Key::Space, NONE));
    assert!(app.main_screen.playing);
}

#[test]
fn a_held_key_acts_once_its_auto_repeat_does_nothing() {
    // A press of a key already held is ignored (egui marks it a repeat).
    let mut rig = rig();
    let press = || vec![key_event(Key::Space, NONE)];
    rig.frame(press());
    rig.frame(press());
    assert!(rig.app.main_screen.playing, "one toggle, not two");
    // An editing action too: a held Ctrl+V pastes once.
    let note = crate::song::Event::Note {
        pos: 0,
        len_beats: 1.0,
        key: 60,
        velocity: crate::song::Event::DEFAULT_VELOCITY,
        pan: crate::song::Event::DEFAULT_PAN,
        slide: false,
    };
    rig.app.song.channels[1].lanes[0].clips[0].events = vec![note];
    rig.app.song_edited(SongEdit::Final, 0);
    rig.edit(1);
    rig.frame(vec![]);
    rig.key(Key::A, CMD);
    rig.key(Key::C, CMD);
    let notes = |rig: &Rig| rig.app.song.channels[1].lanes[0].clips[0].events.len();
    for _ in 0..3 {
        rig.frame(vec![key_event(Key::V, CMD)]);
    }
    assert_eq!(notes(&rig), 2);
    // Undo: a held Ctrl+Z undoes the paste only.
    for _ in 0..3 {
        rig.frame(vec![key_event(Key::Z, CMD)]);
    }
    assert_eq!(notes(&rig), 1);
}

#[test]
fn a_held_ctrl_s_saves_once() {
    let (mut rig, mem) = rig_with_file("/songs/Beat.flm", &synthetic_file(1, FORMAT_VERSION, ""));
    command(&mut rig, Key::O);
    for _ in 0..3 {
        rig.frame(vec![key_event(Key::S, CMD)]);
    }
    assert_eq!(mem.0.borrow().saves.len(), 1);
}

#[test]
fn a_user_plain_binding_takes_its_key_position_from_the_typed_notes() {
    // Notes are not keymap actions: a user's plain `q` binding takes the key at Q from the note map (60), by
    // position: AZERTY's A key (logical A at Q) plays nothing, its Q key (logical Q at W) plays W's 62.
    let tmp = Temp::new("ka-user-plain-q");
    let user = r#"{"modifiers":[],"actions":[{"key":"q","action":"startSearch"}]}"#;
    put(&tmp.0.join("keymap.json"), user.as_bytes());
    let (map, log) = keymap::load(egui::os::OperatingSystem::Windows, None, Some(&tmp.0));
    assert!(log.is_empty(), "{log:?}");
    let _map = keymap::tests::use_map(map);
    let mut rig = Rig::with_synth();
    let azerty = |key, at, pressed| egui::Event::Key {
        key,
        physical_key: Some(at),
        pressed,
        repeat: false,
        modifiers: NONE,
    };
    rig.frame(vec![
        azerty(Key::A, Key::Q, true),
        azerty(Key::A, Key::Q, false),
    ]);
    assert!(!rig.take().iter().any(|c| c.starts_with("NoteOn")));
    rig.frame(vec![azerty(Key::Q, Key::W, true)]);
    assert_eq!(rig.take(), ["NoteOn 1 62 89"]);
}

#[test]
fn the_editing_keys_still_work_after_a_tap_puts_the_focus_in_the_editor() {
    // The `within(view)` half of the grid's gate: a touch moves the focus to what it lands on, inside the
    // view's container; the grid keeps its keys there.
    let mut rig = Rig::with_synth();
    rig.edit(1);
    rig.frame(vec![]);
    let editor = egui::Id::new(crate::ui::focus::EDITOR);
    let at = pos2(600.0, 200.0);
    // A tap on empty grid adds a note; a tap on it selects it.
    rig.click(at);
    rig.click(at);
    rig.frame(vec![]);
    let focused = crate::ui::focus::focused(&rig.ctx);
    assert!(focused.is_some(), "the tap took the focus");
    assert!(crate::ui::focus::within(&rig.ctx, editor), "{focused:?}");
    let notes = |rig: &Rig| rig.app.song.channels[1].lanes[0].clips[0].events.len();
    assert_eq!(notes(&rig), 1);
    // Copy, paste (on the touched cell), then Delete the pasted selection.
    rig.key(Key::C, CMD);
    rig.key(Key::V, CMD);
    assert_eq!(notes(&rig), 2);
    rig.key(Key::Delete, NONE);
    assert_eq!(notes(&rig), 1);
}

#[test]
fn toggle_keyboard_panel_is_a_global_key_that_an_auto_repeat_never_toggles() {
    // Bound by a user keymap (it is a gamepad action in ours): `k` toggles the panel's size, with the release snap.
    let tmp = Temp::new("ck-toggle-panel");
    let user = r#"{"modifiers":[],"actions":[{"key":"k","action":"toggleKeyboardPanel"}]}"#;
    put(&tmp.0.join("keymap.json"), user.as_bytes());
    let (map, log) = keymap::load(egui::os::OperatingSystem::Windows, None, Some(&tmp.0));
    assert!(log.is_empty(), "{log:?}");
    let fired = |repeat| {
        let mut input = egui::InputState::default();
        input.events.push(egui::Event::Key {
            key: Key::K,
            physical_key: None,
            pressed: true,
            repeat,
            modifiers: NONE,
        });
        keys::take(&map, &mut input, |_| true)
    };
    assert_eq!(fired(false), [Global::TogglePanel]);
    assert_eq!(fired(true), [], "an auto-repeat is spent without acting");
    // Applied: the 5 U panel collapses to the bar; again, it opens to the recent 5 U.
    let mut rig = Rig::new();
    let ctx = rig.ctx.clone();
    let (mut rewind, mut navs) = (false, Vec::new());
    rig.app
        .global(&ctx, Global::TogglePanel, &mut rewind, &mut navs);
    assert_eq!(rig.app.settings.preferred_kbd_size, 1.75);
    rig.app
        .global(&ctx, Global::TogglePanel, &mut rewind, &mut navs);
    assert_eq!(rig.app.settings.preferred_kbd_size, 5.0);
}

#[test]
fn shift_ctrl_t_mid_drag_closes_the_tempo_dialog_and_ends_the_drag_as_one_edit() {
    // The toggle that closes the dialog under a dial drag ends it as Escape does: a final edit, one undo step; the
    // release then ends nothing more. Its ongoing tempo set the song-changed flag.
    let mut rig = Rig::with_synth();
    rig.app.files.saved();
    rig.click(BPM);
    rig.frame(vec![]);
    rig.press(dial(0));
    rig.drag_to(dial(1));
    assert_eq!(rig.app.song.tempo_bpm, 120.25);
    assert!(rig.app.files.modified(), "the ongoing drag");
    rig.take();
    rig.key(Key::T, CMD | Modifiers::SHIFT);
    assert_eq!(rig.overlays(), []);
    assert_eq!(rig.sent(), ["ReplaceSong adopting no-resync"]);
    rig.release(dial(1));
    rig.frame(vec![]);
    assert_eq!(rig.sent(), Vec::<String>::new());
    rig.frame(z(NONE));
    assert_eq!(rig.app.song.tempo_bpm, 120.0);
    rig.frame(z(Modifiers::SHIFT));
    assert_eq!(rig.app.song.tempo_bpm, 120.25);
}
