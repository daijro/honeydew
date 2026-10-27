//! The rack and the Modules browser in the app: what the user sees change, and what the audio hears.

use eframe::egui::{Event, Id, Rect};

use super::*;
use crate::panels::cstm::settings_from_cstm;
use crate::panels::{self, PanelState};
use crate::song::ModuleInstance;
use crate::ui::modules_browser::add_module;
use crate::ui::rack::{Flow, PresetTarget, card_height, content_rect, flow, rack_rect};
use crate::ui::views::preset_picker::{CHANNEL_LIMIT, join_workers};

/// The rig's rack, open: 20 U at the content area's right (x 530..1170, down to the panel).
fn rack() -> Rect {
    let m = Metrics::from_pixels_per_point(1.0, DEFAULT_UI_SCALE);
    let screen = Rect::from_min_size(Pos2::ZERO, vec2(1170.0, 540.0));
    rack_rect(Regions::new(screen, &m).area, 1.0, false, &m)
}

impl Rig {
    /// Runs the frames the rack takes to slide all the way in or out: 11 at this width (2⁻¹¹ < 1/1170).
    pub(super) fn slide_rack(&mut self) {
        for _ in 0..11 {
            self.frame(vec![]);
        }
    }
}

/// The centre of the side rack's collapse tab, which closes it.
pub(super) fn collapse_tab() -> Pos2 {
    crate::ui::rack::Chrome::new(rack(), U)
        .collapse
        .unwrap()
        .center()
}

/// How `channel`'s rack lays out, unscrolled.
pub(super) fn layout(rig: &Rig, channel: usize) -> Flow {
    let channel = &rig.app.song.channels[channel];
    let heights: Vec<f32> = channel.rack.modules.iter().map(card_height).collect();
    let sends = channel.kind != ChannelKind::Master;
    flow(rack(), 0.0, U, false, &heights, sends)
}

/// The module types in `channel`'s rack, top to bottom.
pub(super) fn types(rig: &Rig, channel: usize) -> Vec<u16> {
    let modules = &rig.app.song.channels[channel].rack.modules;
    modules.iter().map(|m| m.type_id).collect()
}

pub(super) fn instance_id(rig: &Rig, channel: usize, index: usize) -> i32 {
    rig.app.song.channels[channel].rack.modules[index].instance_id
}

/// The id scope of `channel`'s rack widgets.
fn rack_id(channel: usize) -> Id {
    Id::new(("rack", channel))
}

/// The ≡ button of module `instance_id`'s card in `channel`'s rack.
fn menu_button(channel: usize, instance_id: i32) -> Id {
    rack_id(channel)
        .with(("rack card", instance_id))
        .with("menu")
}

/// Item (`col`, `row`) of the ≡ menu of module `instance_id`'s card in `channel`'s rack.
fn menu_item(channel: usize, instance_id: i32, col: usize, row: usize) -> Id {
    rack_id(channel)
        .with(("rack card menu", instance_id))
        .with((col, row))
}

const MINIMIZE: (usize, usize) = (0, 0);
const SAVE_PRESET: (usize, usize) = (0, 1);
const MOVE_DOWN: (usize, usize) = (0, 3);
const INSERT_ABOVE: (usize, usize) = (1, 1);
const DELETE: (usize, usize) = (1, 4);

/// Picks `item` from the ≡ menu of the card at `index` in `channel`'s open rack.
fn card_menu(rig: &mut Rig, channel: usize, index: usize, item: (usize, usize)) {
    let id = instance_id(rig, channel, index);
    let card = layout(rig, channel).cards[index];
    rig.click(pos2(card.right() - 0.75 * U, card.top() + 0.75 * U));
    // The menu's first frame only sizes it.
    rig.frame(vec![]);
    rig.click_id(menu_item(channel, id, item.0, item.1));
}

pub(super) const BROWSER: &str = "modules_browser";

/// Waits out the browser's slide-in, searches for `query` and picks the first match (row 1, under its section).
/// Widgets are looked up two frames back (egui swaps its pass states), so each step waits a frame more.
/// Closes the open picker or Modules browser with Escape: they stay open after a pick.
pub(super) fn close_picker(rig: &mut Rig) {
    rig.frame(vec![]);
    rig.frame(escape());
    rig.frame(vec![]);
}

fn pick(rig: &mut Rig, query: &str) {
    for _ in 0..3 {
        rig.frame(vec![]);
    }
    rig.click_id(Id::new(BROWSER).with("search"));
    rig.frame(vec![Event::Text(query.into())]);
    rig.frame(vec![]);
    rig.click_id(Id::new(BROWSER).with(("row", 1_usize)));
}

/// Undoes the last undo step, the effect track's add ([`with_effects`]), and redoes it: the rack is not covered by
/// undo, so the track comes back with the rack it went with.
fn undo_redo_the_track(rig: &mut Rig) {
    let n = rig.app.song.channels.len();
    rig.frame(z(egui::Modifiers::NONE));
    assert_eq!(rig.app.song.channels.len(), n - 1, "the track's add undone");
    rig.frame(z(egui::Modifiers::SHIFT));
    assert_eq!(rig.app.song.channels.len(), n);
}

/// [`Rig::with_synth`] plus an effect track (2) holding `types`, selected, its rack open, with an empty log. Its last
/// undo step is the track's add: the modules' adds make none.
pub(super) fn with_effects(types: &[u16]) -> Rig {
    let mut rig = Rig::with_synth();
    rig.app.add_channel(ChannelKind::EffectTrack, 0);
    for &t in types {
        add_module(&mut rig.app.song.channels[2].rack, t, InsertAt::End);
    }
    rig.app.song_edited(SongEdit::Final, 0);
    rig.open_rack(2);
    rig.slide_rack();
    rig.take();
    rig
}

#[test]
fn add_module_opens_the_browser_whose_pick_adds_the_module_and_rebuilds_the_song() {
    let mut rig = Rig::with_synth();
    rig.open_rack(1);
    rig.slide_rack();
    let add = layout(&rig, 1).add_module;
    rig.click(add.center());
    let at = InsertAt::End;
    let browser = Overlay::ModulesBrowser { channel: 1, at };
    assert_eq!(rig.overlays(), [Overlay::Rack(1), browser.clone()]);
    rig.app.files.saved();
    pick(&mut rig, "Dist");
    // It stays open.
    assert_eq!(rig.overlays(), [Overlay::Rack(1), browser]);
    assert_eq!(types(&rig, 1), [0, 6]);
    // A pick marks the song changed.
    assert!(rig.app.files.modified());
    assert_eq!(rig.take(), ["ReplaceSong adopting no-resync"]);
}

#[test]
fn insert_above_opens_the_browser_there_and_escape_closes_the_browser_only() {
    let mut rig = Rig::with_synth();
    add_module(&mut rig.app.song.channels[1].rack, 6, InsertAt::End);
    rig.open_rack(1);
    rig.slide_rack();
    rig.take();
    let distortion = instance_id(&rig, 1, 1);
    card_menu(&mut rig, 1, 1, INSERT_ABOVE);
    let at = InsertAt::Above(distortion);
    let browser = Overlay::ModulesBrowser { channel: 1, at };
    assert_eq!(rig.overlays(), [Overlay::Rack(1), browser]);
    rig.frame(vec![]);
    rig.frame(escape());
    assert_eq!(rig.overlays(), [Overlay::Rack(1)]);

    // Opened again, the browser starts afresh: the slide-in and an empty search.
    card_menu(&mut rig, 1, 1, INSERT_ABOVE);
    pick(&mut rig, "Chor");
    assert_eq!(types(&rig, 1), [0, 11, 6]);
    assert_eq!(rig.take(), ["ReplaceSong adopting no-resync"]);
    // The first Escape hides the search bar, the second closes.
    close_picker(&mut rig);
    close_picker(&mut rig);
    let chorus = instance_id(&rig, 1, 1);
    card_menu(&mut rig, 1, 1, INSERT_ABOVE);
    let at = InsertAt::Above(chorus);
    let browser = Overlay::ModulesBrowser { channel: 1, at };
    assert_eq!(rig.overlays(), [Overlay::Rack(1), browser]);
    rig.frame(vec![]);
    rig.frame(vec![]);
    // The search is closed, so row 1 is `Rack Preset`, not the `Chorus` of the last search: it opens the rack
    // preset list in the browser's place.
    let query = Id::new(BROWSER).with("query");
    assert!(rig.ctx.read_response(query).is_none());
    rig.frame(vec![]);
    rig.click_id(Id::new(BROWSER).with(("row", 1_usize)));
    let target = PresetTarget::ModulesRack(at);
    let picker = Overlay::PresetPicker { channel: 1, target };
    assert_eq!(rig.overlays(), [Overlay::Rack(1), picker]);
    assert_eq!(types(&rig, 1), [0, 11, 6]);
}

#[test]
fn a_knob_drag_writes_the_song_sends_each_step_without_a_rebuild_and_saves_no_undo_step() {
    let mut rig = with_effects(&[6]);
    rig.app.files.saved();
    let steps = rig.app.history.steps();
    let id = instance_id(&rig, 2, 0);
    let content = content_rect(layout(&rig, 2).cards[0], 5.5, U);
    let module = rig.app.song.channels[2].rack.modules[0].clone();
    let panel = panels::panel_for(6).unwrap();
    let placed = panels::place(panel, &module, &PanelState::default(), content, U);
    // Drive, from 0.25 up a quarter of its range: 200 points (a knob moves 0.24 per 192-point inch).
    let drive = placed.iter().find(|(l, _)| l.id == 3).unwrap().1.center();
    rig.press(drive);
    for step in 1..=4 {
        rig.drag_to(drive - vec2(0.0, 200.0 * step as f32 / 4.0));
    }
    let value = rig.app.song.channels[2].rack.modules[0].params[1];
    assert!((value - 0.5).abs() < 1e-5, "{value}");
    let sent = rig.take();
    assert_eq!(sent.len(), 4, "{sent:?}");
    assert!(
        sent.iter()
            .all(|s| s.starts_with(&format!("SetParam 2 {id} 1 ")))
    );
    assert_eq!(sent[3], format!("SetParam 2 {id} 1 {value:.3}"));
    rig.release(drive - vec2(0.0, 200.0));
    assert_eq!(rig.take(), Vec::<String>::new());
    assert!(rig.app.files.modified());
    // No undo step, and no undo or redo puts the old value back.
    assert_eq!(rig.app.history.steps(), steps);
    undo_redo_the_track(&mut rig);
    assert_eq!(rig.app.song.channels[2].rack.modules[0].params[1], value);
}

#[test]
fn minimizing_a_card_or_all_of_them_marks_the_song_changed_with_no_undo_step_or_rebuild() {
    let mut rig = with_effects(&[6, 11]);
    rig.app.files.saved();
    let steps = rig.app.history.steps();
    let minimized = |rig: &Rig| -> Vec<bool> {
        let modules = &rig.app.song.channels[2].rack.modules;
        modules.iter().map(|m| m.minimized).collect()
    };
    card_menu(&mut rig, 2, 0, MINIMIZE);
    assert_eq!(minimized(&rig), [true, false]);
    // The rack ≡ menu's `Minimize all` (row 4 without `Load Drumset`).
    rig.click_id(rack_id(2).with("rack menu button"));
    rig.frame(vec![]);
    rig.click_id(rack_id(2).with("rack menu").with((0_usize, 4_usize)));
    assert_eq!(minimized(&rig), [true, true]);
    assert_eq!(rig.take(), Vec::<String>::new());
    // The rack is not covered by undo.
    assert!(rig.app.files.modified());
    assert_eq!(rig.app.history.steps(), steps);
}

#[test]
fn moving_or_deleting_a_card_rebuilds_the_song() {
    let mut rig = with_effects(&[6, 11]);
    card_menu(&mut rig, 2, 0, MOVE_DOWN);
    assert_eq!(types(&rig, 2), [11, 6]);
    assert_eq!(rig.take(), ["ReplaceSong adopting no-resync"]);
    // The swap slide comes to rest before the next menu.
    for _ in 0..20 {
        rig.frame(vec![]);
    }
    card_menu(&mut rig, 2, 0, DELETE);
    assert_eq!(types(&rig, 2), [6]);
    assert_eq!(rig.take(), ["ReplaceSong adopting no-resync"]);
    assert_eq!(rig.overlays(), [Overlay::Rack(2)]);
}

#[test]
fn a_note_effect_slider_drag_is_sent_at_each_step_rebuilds_the_song_once_the_pointer_is_up_and_saves_no_undo_step()
 {
    // Transpose (48), a note effect. No note effect has a panel yet: its level dialog's Mix slider (-2).
    let mut rig = with_effects(&[48]);
    let steps = rig.app.history.steps();
    let id = instance_id(&rig, 2, 0);
    let mix_of = |rig: &Rig| {
        let modules = &rig.app.song.channels[2].rack.modules;
        modules.first().and_then(|m| m.param(-2))
    };
    let before = mix_of(&rig);
    let card = layout(&rig, 2).cards[0];
    rig.click(pos2(card.left() + 2.25 * U, card.top() + 0.75 * U));
    rig.frame(vec![]);
    let slider = rack_id(2).with(("rack dialog", id)).with(-2_i32);
    let mix = rig
        .ctx
        .read_response(slider)
        .expect("the dialog is open")
        .rect;
    rig.press(mix.center());
    let mut sent = Vec::new();
    for step in 1..=4 {
        rig.drag_to(mix.center() - vec2(U * step as f32, 0.0));
        sent.extend(rig.take());
    }
    assert!(!sent.is_empty());
    assert!(
        sent.iter()
            .all(|s| s.starts_with(&format!("SetParam 2 {id} -2 "))),
        "no rebuild mid-drag: {sent:?}"
    );
    let value = rig.app.song.channels[2].rack.modules[0].param(-2).unwrap();
    assert!(value < 1.0, "{value}");
    assert_eq!(
        sent.last().unwrap(),
        &format!("SetParam 2 {id} -2 {value:.3}")
    );
    rig.release(mix.center() - vec2(U * 4.0, 0.0));
    assert_eq!(rig.take(), ["ReplaceSong adopting no-resync"]);
    rig.frame(vec![]);
    assert_eq!(rig.take(), Vec::<String>::new());
    // No undo step, and no undo or redo puts the old Mix back. Not under the level dialog: it closes first.
    assert_eq!(rig.app.history.steps(), steps);
    rig.frame(escape());
    undo_redo_the_track(&mut rig);
    assert_ne!(before, Some(value));
    assert_eq!(mix_of(&rig), Some(value));
}

#[test]
fn a_note_effect_param_moved_in_the_frame_of_its_rebuild_makes_no_undo_step() {
    // Regression: a touch on Transpose's Mix slider and Ctrl+− in one frame send SetParam and ReplaceSong
    // together; the rebuild's step took the move in, and an undo put the old Mix back.
    let mut rig = with_effects(&[48]);
    let id = instance_id(&rig, 2, 0);
    let mix_of = |rig: &Rig| rig.app.song.channels[2].rack.modules[0].param(-2);
    let before = mix_of(&rig);
    let card = layout(&rig, 2).cards[0];
    rig.click(pos2(card.left() + 2.25 * U, card.top() + 0.75 * U));
    rig.frame(vec![]);
    let slider = rack_id(2).with(("rack dialog", id)).with(-2_i32);
    let mix = (rig.ctx.read_response(slider))
        .expect("the dialog is open")
        .rect
        .center();
    rig.take();
    let steps = rig.app.history.steps();
    let minus = key_event(
        egui::Key::Minus,
        egui::Modifiers::COMMAND | egui::Modifiers::SHIFT,
    );
    rig.frame(vec![
        Event::PointerMoved(mix),
        button_event(mix, true),
        button_event(mix, false),
        minus,
    ]);
    rig.frame(vec![]);
    let sent = rig.take();
    let set = sent
        .iter()
        .any(|c| c.starts_with(&format!("SetParam 2 {id} -2 ")));
    assert!(
        set && sent.iter().any(|c| c.starts_with("ReplaceSong")),
        "{sent:?}"
    );
    let moved = mix_of(&rig);
    assert_ne!(moved, before);
    assert_eq!(rig.app.history.steps(), steps);
    rig.frame(escape());
    undo_redo_the_track(&mut rig);
    assert_eq!(mix_of(&rig), moved);
}

#[test]
fn an_undo_after_a_delete_and_a_pick_leaves_the_rack_as_it_is() {
    // The rack and presets are not covered by undo: delete a module, pick one, undo: the undo takes back the edit
    // before them, and the rack stays.
    let mut rig = with_effects(&[6, 11]);
    rig.app.song.channels[2].name = "Before".into();
    rig.app.song_edited(SongEdit::Final, 0);
    rig.frame(vec![]);
    rig.take();
    rig.app.files.saved();
    card_menu(&mut rig, 2, 0, DELETE);
    assert_eq!(types(&rig, 2), [11]);
    for _ in 0..20 {
        rig.frame(vec![]);
    }
    rig.click(layout(&rig, 2).add_module.center());
    pick(&mut rig, "Dist");
    assert_eq!(types(&rig, 2), [11, 6]);
    assert!(rig.app.files.modified());
    let rack = rig.app.song.channels[2].rack.clone();
    close_picker(&mut rig);
    close_picker(&mut rig);
    rig.frame(z(egui::Modifiers::NONE));
    assert_ne!(rig.app.song.channels[2].name, "Before", "the rename undone");
    assert_eq!(rig.app.song.channels[2].rack, rack);
    rig.frame(z(egui::Modifiers::SHIFT));
    assert_eq!(rig.app.song.channels[2].rack, rack);
}

#[test]
fn the_browser_opens_afresh_after_a_close_its_slot_did_not_see() {
    let mut rig = Rig::with_synth();
    let browser = Overlay::ModulesBrowser {
        channel: 1,
        at: InsertAt::End,
    };
    rig.app.navigate(Nav::Open(browser.clone()));
    for _ in 0..3 {
        rig.frame(vec![]);
    }
    rig.click_id(Id::new(BROWSER).with("search"));
    rig.frame(vec![Event::Text("Dist".into())]);
    rig.frame(vec![]);
    let row = Id::new(BROWSER).with(("row", 1_usize));
    assert!(rig.ctx.read_response(row).is_some(), "Distortion matches");
    // A view switch closes every overlay without drawing them.
    rig.app.navigate(Nav::Show(View::Playlist));
    rig.frame(vec![]);
    assert_eq!(rig.overlays(), []);

    rig.app.navigate(Nav::Open(browser));
    rig.frame(vec![]);
    rig.frame(vec![]);
    // The search is closed, so row 1 is `Rack Preset`, not the `Distortion` of the last search: the rack preset
    // list takes the browser's place.
    assert!(
        rig.ctx
            .read_response(Id::new(BROWSER).with("query"))
            .is_none()
    );
    rig.frame(vec![]);
    rig.click_id(row);
    let target = PresetTarget::ModulesRack(InsertAt::End);
    assert_eq!(
        rig.overlays(),
        [Overlay::PresetPicker { channel: 1, target }]
    );
    assert_eq!(types(&rig, 1), [0]);
}

#[test]
fn another_channels_rack_starts_afresh() {
    // Instance ids repeat across channels: both instruments are id 2.
    let mut rig = Rig::with_synth();
    for _ in 0..3 {
        add_module(&mut rig.app.song.channels[1].rack, 6, InsertAt::End);
    }
    rig.app
        .add_channel(ChannelKind::Synth(SynthKind::GmSynth), 0);
    rig.open_rack(1);
    rig.slide_rack();
    // Scroll rack 1 down by dragging its background (the left gutter, below the notch).
    let gutter = pos2(rack().left() + 0.75 * U, 300.0);
    rig.press(gutter);
    rig.drag_to(gutter - vec2(0.0, 50.0));
    rig.drag_to(gutter - vec2(0.0, 100.0));
    rig.release(gutter - vec2(0.0, 100.0));
    let scrolled = rig.ctx.read_response(menu_button(1, 2)).unwrap().rect;
    assert!(
        scrolled.top() < layout(&rig, 1).cards[0].top(),
        "{scrolled:?}"
    );

    rig.open_rack(2);
    assert_eq!(rig.overlays(), [Overlay::Rack(2)]);
    rig.frame(vec![]);
    rig.frame(vec![]);
    let card = layout(&rig, 2).cards[0];
    let button = rig.ctx.read_response(menu_button(2, 2)).unwrap().rect;
    assert_eq!(
        button.center().y,
        card.top() + 0.75 * U,
        "rack 2 is scrolled"
    );
}

/// The texts the last frame painted inside `rect`.
fn texts_in(rig: &Rig, rect: Rect) -> Vec<String> {
    rig.out
        .shapes
        .iter()
        .filter_map(|s| match &s.shape {
            egui::Shape::Text(t) if rect.contains(t.visual_bounding_rect().center()) => {
                Some(t.galley.text().to_owned())
            }
            _ => None,
        })
        .collect()
}

#[test]
fn a_racks_value_texts_read_the_song_tempo() {
    // Autoduck Attack is round(b·60000/BPM) ms, the rack lends the panels the song tempo.
    let mut rig = with_effects(&[29]);
    rig.app.song.tempo_bpm = 120.0;
    rig.app.song.channels[2].rack.modules[0].params[2] = 0.5;
    rig.frame(vec![]);
    let attack = rack_id(2)
        .with(("module panel", instance_id(&rig, 2, 0)))
        .with(2_u32);
    let rect = rig.ctx.read_response(attack).unwrap().rect;
    let texts = texts_in(&rig, rect);
    assert!(texts.iter().any(|t| t == "125 ms"), "{texts:?}");
}

/// While the song plays, a control shows the value its automation has set by the playhead (issue #14), and the song
/// keeps its own: the Autoduck's Attack at 0.25, automated to 0.5 (125 ms) from beat 0.
#[test]
fn a_playing_automation_moves_its_control_and_the_song_keeps_its_value() {
    let mut rig = with_effects(&[29]);
    rig.app.song.tempo_bpm = 120.0;
    let id = instance_id(&rig, 2, 0);
    rig.app.song.channels[2].rack.modules[0].params[2] = 0.25;
    let target = AutomationTarget {
        instance_id: id,
        param: 2,
    };
    let lane = rig.app.song.channels[2].add_automation_lane(target);
    lane.add_clip(0, 4.0)
        .events
        .push(crate::song::Event::Automation {
            pos: 0,
            value: 0.5,
            control: true,
        });
    rig.click(PLAY);
    rig.rec.0.borrow_mut().tick = 64;
    rig.frame(vec![]);
    let attack = rack_id(2).with(("module panel", id)).with(2_u32);
    let rect = rig.ctx.read_response(attack).unwrap().rect;
    let texts = texts_in(&rig, rect);
    assert!(texts.iter().any(|t| t == "125 ms"), "{texts:?}");
    assert_eq!(rig.app.song.channels[2].rack.modules[0].params[2], 0.25);
}

/// Control `n` of the Pitcher on top of `channel`'s rack.
fn pitcher(rig: &Rig, channel: usize, n: u32) -> Id {
    let id = instance_id(rig, channel, 0);
    rack_id(channel).with(("module panel", id)).with(n)
}

fn pitcher_rect(rig: &Rig, channel: usize, n: u32) -> Rect {
    rig.ctx
        .read_response(pitcher(rig, channel, n))
        .unwrap()
        .rect
}

/// Sets the Pitcher on top of `channel`'s open rack to Major (`Scale` `v` once) on D (`Key` `v` twice).
fn pitcher_major_d(rig: &mut Rig, channel: usize) {
    for n in [20, 21, 21] {
        // The `v` chevron: low in the spinner's body.
        let r = pitcher_rect(rig, channel, n);
        rig.click(pos2(r.center().x, r.bottom() - 0.1 * U));
    }
}

/// The texts the Pitcher on top of `channel`'s open rack shows in its `Scale` and `Key` spinners.
fn pitcher_scale_key(rig: &Rig, channel: usize) -> Vec<String> {
    [20, 21]
        .into_iter()
        .flat_map(|n| texts_in(rig, pitcher_rect(rig, channel, n)))
        .collect()
}

const MAJOR_D: [&str; 4] = ["Scale", "Major", "Key", "D"];
const MINOR_C: [&str; 4] = ["Scale", "Minor", "Key", "C"];

#[test]
fn panel_settings_and_tab_pages_survive_another_channels_rack() {
    // Regression: the Pitcher's Key and Scale (settings, not params) were reset when another rack opened.
    let mut rig = with_effects(&[25]);
    pitcher_major_d(&mut rig, 2);
    assert_eq!(pitcher_scale_key(&rig, 2), MAJOR_D);
    for channel in [1, 2] {
        rig.open_rack(channel);
        rig.frame(vec![]);
    }
    rig.frame(vec![]);
    assert_eq!(rig.overlays(), [Overlay::Rack(2)]);
    assert_eq!(pitcher_scale_key(&rig, 2), MAJOR_D);

    // The Advanced page, picked on the tab strip's right half, stays picked too.
    advanced_page(&mut rig, 2);
    for channel in [1, 2] {
        rig.open_rack(channel);
        rig.frame(vec![]);
    }
    rig.frame(vec![]);
    assert!(
        rig.ctx.read_response(pitcher(&rig, 2, 37)).is_some(),
        "Base Freq shows"
    );
    assert!(
        rig.ctx.read_response(pitcher(&rig, 2, 20)).is_none(),
        "Scale is hidden"
    );
}

#[test]
fn a_pitcher_note_press_goes_to_the_audio_as_its_setting() {
    // E (#9), off in C minor, on; then Major on D rewrites the notes, each going out.
    let mut rig = with_effects(&[25]);
    let id = instance_id(&rig, 2, 0);
    rig.click(pitcher_rect(&rig, 2, 9).center());
    assert_eq!(rig.take(), [format!("SetSetting 2 {id} 9 1.000")]);
    pitcher_major_d(&mut rig, 2);
    let sent = rig.take();
    assert!(
        sent.iter()
            .all(|c| c.starts_with(&format!("SetSetting 2 {id} "))),
        "{sent:?}"
    );
    assert!(
        sent.contains(&format!("SetSetting 2 {id} 21 2.000")),
        "Key D: {sent:?}"
    );
}

/// Picks the Advanced page of the Pitcher on top of `channel`'s open rack (the tab strip's right half).
fn advanced_page(rig: &mut Rig, channel: usize) {
    let tabs = pitcher_rect(rig, channel, 2);
    rig.click(pos2(tabs.right() - 0.25 * tabs.width(), tabs.center().y));
    rig.frame(vec![]);
    assert!(
        rig.ctx.read_response(pitcher(rig, channel, 37)).is_some(),
        "Base Freq shows"
    );
}

#[test]
fn a_channel_removed_before_a_rack_leaves_it_no_panel_states() {
    // Regression: panel states were keyed by channel index; deleting an earlier channel moved them onto
    // the next channel's Pitcher.
    let mut rig = with_effects(&[25]);
    rig.app.add_channel(ChannelKind::EffectTrack, 0);
    add_module(&mut rig.app.song.channels[3].rack, 25, InsertAt::End);
    rig.app.selected_channel = 2;
    rig.app.song_edited(SongEdit::Final, 0);
    rig.frame(vec![]);
    pitcher_major_d(&mut rig, 2);
    assert_eq!(pitcher_scale_key(&rig, 2), MAJOR_D);
    // Channel 2 goes, as a delete would: its rack shows the current channel, channel 3 now at index 2,
    // whose Pitcher has its own.
    rig.app.song.delete_channel(2);
    rig.app.song_edited(SongEdit::Final, 0);
    rig.frame(vec![]);
    assert_eq!(rig.overlays(), [Overlay::Rack(2)]);
    rig.app.navigate(Nav::Open(Overlay::Rack(2)));
    rig.slide_rack();
    rig.frame(vec![]);
    assert_eq!(pitcher_scale_key(&rig, 2), MINOR_C);
}

#[test]
fn a_clone_starts_with_its_sources_panel_settings_and_keeps_its_own() {
    // Regression: the clone's Pitcher showed the defaults.
    let mut rig = with_effects(&[25]);
    pitcher_major_d(&mut rig, 2);
    rig.app
        .track_op(2, 0, crate::ui::track_menu::TrackOp::CloneChannel, 0);
    rig.app.navigate(Nav::Open(Overlay::Rack(3)));
    rig.slide_rack();
    rig.frame(vec![]);
    assert_eq!(pitcher_scale_key(&rig, 3), MAJOR_D);
    advanced_page(&mut rig, 3);
    rig.app.navigate(Nav::Open(Overlay::Rack(2)));
    rig.slide_rack();
    rig.frame(vec![]);
    assert_eq!(pitcher_scale_key(&rig, 2), MAJOR_D, "the source's own page");
}

#[test]
fn panel_states_follow_their_channel_when_an_earlier_one_goes_and_comes_back() {
    // Re-keyed by channel id, not reset.
    let mut rig = with_effects(&[25]);
    rig.app.add_channel(ChannelKind::EffectTrack, 0);
    add_module(&mut rig.app.song.channels[3].rack, 25, InsertAt::End);
    rig.app.song_edited(SongEdit::Final, 0);
    rig.app.navigate(Nav::Open(Overlay::Rack(3)));
    rig.slide_rack();
    rig.frame(vec![]);
    pitcher_major_d(&mut rig, 3);
    rig.app.song.delete_channel(2);
    rig.app.song_edited(SongEdit::Final, 0);
    rig.frame(vec![]);
    rig.frame(vec![]);
    assert_eq!(rig.overlays(), [Overlay::Rack(2)], "the open rack follows");
    assert_eq!(pitcher_scale_key(&rig, 2), MAJOR_D);
    rig.frame(z(egui::Modifiers::NONE));
    rig.frame(vec![]);
    rig.frame(vec![]);
    assert_eq!(rig.overlays(), [Overlay::Rack(3)]);
    assert_eq!(pitcher_scale_key(&rig, 3), MAJOR_D);
}

#[test]
fn a_channel_undone_then_added_again_starts_with_fresh_panel_states() {
    // Regression: Ctrl+Z removed a channel, and a new one at its index inherited its tab page.
    let add = |rig: &mut Rig| {
        rig.app.song.add_channel(ChannelKind::EffectTrack);
        add_module(&mut rig.app.song.channels[2].rack, 25, InsertAt::End);
        rig.app.song_edited(SongEdit::Final, 0);
        rig.app.navigate(Nav::Open(Overlay::Rack(2)));
        rig.slide_rack();
        rig.frame(vec![]);
    };
    let mut rig = Rig::with_synth();
    add(&mut rig);
    advanced_page(&mut rig, 2);
    rig.frame(z(egui::Modifiers::NONE));
    assert_eq!(rig.app.song.channels.len(), 2);
    rig.frame(vec![]);
    // The rack follows the current channel.
    assert_eq!(rig.overlays(), [Overlay::Rack(1)]);
    add(&mut rig);
    assert!(
        rig.ctx.read_response(pitcher(&rig, 2, 20)).is_some(),
        "the Basic page"
    );
    assert!(rig.ctx.read_response(pitcher(&rig, 2, 37)).is_none());
}

#[test]
fn a_new_song_clears_the_panel_states_and_seeds_them_from_its_own() {
    let mut rig = with_effects(&[25]);
    let before = rig.app.song.clone();
    pitcher_major_d(&mut rig, 2);
    let edited = rig.app.song.clone();
    // The song's Pitcher has no blob: the defaults.
    for (song, scale_key) in [(before, MINOR_C), (edited, MAJOR_D)] {
        rig.app.song = song;
        rig.app.song_edited(SongEdit::Replaced, 0);
        rig.app.navigate(Nav::Open(Overlay::Rack(2)));
        rig.slide_rack();
        rig.frame(vec![]);
        assert_eq!(pitcher_scale_key(&rig, 2), scale_key);
    }
}

#[test]
fn escape_closes_the_keyboard_settings_over_a_rack_then_the_racks_menu() {
    // Regression: Escape went to the topmost overlay, the keyboard settings, which ignored it; the rack's card
    // menu under it could not be closed with Escape.
    let mut rig = Rig::with_synth();
    rig.open_rack(1);
    rig.slide_rack();
    rig.app.navigate(Nav::Open(Overlay::KeyboardSettings));
    rig.frame(vec![]);
    let id = instance_id(&rig, 1, 0);
    let card = layout(&rig, 1).cards[0];
    rig.click(pos2(card.right() - 0.75 * U, card.top() + 0.75 * U));
    rig.frame(vec![]);
    rig.frame(vec![]);
    let item = menu_item(1, id, MINIMIZE.0, MINIMIZE.1);
    assert!(
        rig.ctx.read_response(item).is_some(),
        "the card menu is open"
    );
    rig.frame(escape());
    assert_eq!(rig.overlays(), [Overlay::Rack(1)]);
    rig.frame(escape());
    rig.frame(vec![]);
    rig.frame(vec![]);
    assert!(
        rig.ctx.read_response(item).is_none(),
        "the card menu closed"
    );
    assert_eq!(rig.overlays(), [Overlay::Rack(1)]);
}

#[test]
fn the_wheel_scrolls_no_rack_under_the_main_menu() {
    let mut rig = Rig::with_synth();
    for _ in 0..3 {
        add_module(&mut rig.app.song.channels[1].rack, 6, InsertAt::End);
    }
    rig.app.navigate(Nav::Open(Overlay::Rack(1)));
    rig.app
        .navigate(Nav::Open(Overlay::MainMenu(MenuTab::Settings)));
    rig.frame(vec![]);
    rig.frame(vec![]);
    let top = |rig: &Rig| rig.ctx.read_response(menu_button(1, 2)).unwrap().rect.top();
    let unscrolled = top(&rig);
    rig.wheel(pos2(800.0, 200.0), -300.0);
    assert_eq!(top(&rig), unscrolled);
    // A short wheel, so that the card stays in view.
    rig.app.nav.overlays.pop();
    rig.frame(vec![]);
    rig.wheel(pos2(800.0, 200.0), -10.0);
    assert!(top(&rig) < unscrolled);
}

// ---- Module settings ↔ `CSTM` ----

/// The Tuner's `Reference` slider (#5) on the first card of `channel`'s open rack (not an audio input).
fn tuner_reference(rig: &Rig, channel: usize) -> Pos2 {
    let content = content_rect(layout(rig, channel).cards[0], 7.0, U);
    let module = &rig.app.song.channels[channel].rack.modules[0];
    let mut state = PanelState::default();
    state.flags.insert(panels::flag::CHANNEL_KIND, 2);
    let placed = panels::place(panels::panel_for(24).unwrap(), module, &state, content, U);
    placed.iter().find(|(l, _)| l.id == 5).unwrap().1.center()
}

/// Drags `channel`'s Tuner `Reference` all the way up (its thumb follows the pointer) and lets go; returns the
/// module's blob.
fn drag_reference(rig: &mut Rig, channel: usize) -> Option<Vec<u8>> {
    let at = tuner_reference(rig, channel);
    rig.press(at);
    rig.drag_to(at - vec2(0.0, 10.0 * U));
    rig.release(at - vec2(0.0, 10.0 * U));
    rig.app.song.channels[channel].rack.modules[0]
        .file
        .custom_data
        .clone()
}

#[test]
fn a_setting_edit_writes_the_cstm_with_no_undo_step() {
    let mut rig = with_effects(&[24]);
    rig.app.files.saved();
    let steps = rig.app.history.steps();
    let blob = drag_reference(&mut rig, 2);
    assert!(blob.is_some());
    // The audio gets what it stored, the one Reference value of the knob #4 and the slider #5.
    assert_eq!(
        rig.take(),
        ["SetSetting 2 2 4 1.000", "SetSetting 2 2 5 1.000"]
    );
    assert!(rig.app.files.modified(), "an unsaved change");
    // The rack is not covered by undo: no step, and an undo and redo keep the blob.
    assert_eq!(rig.app.history.steps(), steps);
    undo_redo_the_track(&mut rig);
    let cstm = &rig.app.song.channels[2].rack.modules[0].file.custom_data;
    assert_eq!(cstm, &blob);
}

#[test]
fn a_panel_setting_saved_and_opened_again_is_in_the_modules_cstm() {
    // The fixture song: its DirectWave has an 81-byte CSTM (`DW01`, `MIDI` and its zones).
    let (mut rig, _) = fresh_song();
    let cstm = |rig: &Rig| {
        rig.app.song.channels[1].rack.modules[0]
            .file
            .custom_data
            .clone()
    };
    let before = cstm(&rig).expect("the file's blob");
    let range = |blob: &[u8]| settings_from_cstm(1, blob).map(|s| s[&11800]);
    rig.app.navigate(Nav::Open(Overlay::Rack(1)));
    rig.slide_rack();
    rig.frame(vec![]);
    // The DW Sampler's `PB Range` (SAMPLE page), up a quarter of its range.
    let content = content_rect(layout(&rig, 1).cards[0], 7.5, U);
    let module = rig.app.song.channels[1].rack.modules[0].clone();
    let panel = panels::panel_for(1).unwrap();
    let placed = panels::place(panel, &module, &PanelState::default(), content, U);
    let knob = placed
        .iter()
        .find(|(l, _)| l.id == 11800)
        .unwrap()
        .1
        .center();
    rig.press(knob);
    rig.drag_to(knob - vec2(0.0, 200.0));
    rig.release(knob - vec2(0.0, 200.0));
    let edited = cstm(&rig).unwrap();
    assert_ne!(range(&edited), range(&before));
    // Its one byte (the `MIDI` chunk's semitones); the rest of the blob is kept.
    assert_eq!(edited.len(), before.len());
    assert_eq!(
        edited.iter().zip(&before).filter(|(a, b)| a != b).count(),
        1
    );
    // The DW Sampler reads its whole blob at build (its zones), so the edit rebuilds the engine.
    assert_eq!(rig.take(), ["ReplaceSong adopting no-resync"]);
    // Ctrl+S writes it over the file without asking; Ctrl+O opens the file again, as saved.
    command(&mut rig, egui::Key::S);
    assert_eq!(rig.overlays(), [Overlay::Rack(1)]);
    let saved = rig.app.song.clone();
    command(&mut rig, egui::Key::O);
    assert_eq!(rig.app.song, saved);
    assert_eq!(cstm(&rig), Some(edited));
}

const COPY: (usize, usize) = (1, 0);
const REPLACE: (usize, usize) = (1, 3);

/// The browser's row `i` (no search: 0 is `Paste`), once its slide-in is over.
pub(super) fn browser_row(rig: &mut Rig, i: usize) -> Option<Rect> {
    for _ in 0..3 {
        rig.frame(vec![]);
    }
    let row = rig.ctx.read_response(Id::new(BROWSER).with(("row", i)));
    row.map(|r| r.rect)
}

#[test]
fn copy_then_add_module_then_paste_gives_two_identical_modules_with_different_ids() {
    let mut rig = Rig::with_synth();
    rig.app.add_channel(ChannelKind::EffectTrack, 0);
    let rack = &mut rig.app.song.channels[2].rack;
    add_module(rack, 11, InsertAt::End);
    rack.modules[0].params[1] = 0.9;
    rig.app.song_edited(SongEdit::Final, 0);
    rig.open_rack(2);
    rig.slide_rack();
    let add = layout(&rig, 2).add_module.center();
    rig.take();
    let steps = rig.app.history.steps();
    // `Paste` is enabled with nothing on the clipboard: it closes the browser and flashes, with no edit.
    rig.click(add);
    let paste = browser_row(&mut rig, 0).expect("Paste is enabled");
    rig.click(paste.center());
    assert_eq!(rig.overlays(), [Overlay::Rack(2)]);
    assert_eq!(
        rig.app.flash.as_ref().map(|f| f.0.as_str()),
        Some("Cannot paste module")
    );
    assert_eq!(types(&rig, 2), [11]);
    assert_eq!(rig.take(), Vec::<String>::new());

    card_menu(&mut rig, 2, 0, COPY);
    assert_eq!(rig.take(), Vec::<String>::new(), "a copy is no song edit");
    assert_eq!(types(&rig, 2), [11]);
    rig.click(add);
    let paste = browser_row(&mut rig, 0).expect("Paste is enabled");
    rig.click(paste.center());
    assert_eq!(rig.overlays(), [Overlay::Rack(2)]);
    let modules = &rig.app.song.channels[2].rack.modules;
    assert_eq!(types(&rig, 2), [11, 11]);
    assert_ne!(modules[0].instance_id, modules[1].instance_id);
    let copy = ModuleInstance {
        instance_id: modules[1].instance_id,
        ..modules[0].clone()
    };
    assert_eq!(modules[1], copy);
    assert_eq!(modules[1].params[1], 0.9);
    assert_eq!(rig.take(), ["ReplaceSong adopting no-resync"]);
    // No undo step: the rack is not covered by undo.
    assert_eq!(rig.app.history.steps(), steps);
}

#[test]
fn the_clipboard_outlives_the_song() {
    let mut rig = with_effects(&[11]);
    card_menu(&mut rig, 2, 0, COPY);
    let copied = rig.app.song.channels[2].rack.modules[0].clone();
    rig.app.song = Song::default();
    rig.app.song_edited(SongEdit::Replaced, 0);
    assert_eq!(rig.app.slots.clipboard, [copied]);
}

/// `Copy Rack` (row 2 without `Load Drumset`) in `channel`'s open rack.
fn copy_rack(rig: &mut Rig, channel: usize) {
    rig.click_id(rack_id(channel).with("rack menu button"));
    rig.frame(vec![]);
    rig.click_id(rack_id(channel).with("rack menu").with((0_usize, 2_usize)));
}

#[test]
fn copy_rack_then_paste_adds_the_whole_rack_to_another_channel_with_no_undo_step() {
    let mut rig = Rig::with_synth();
    // The synth channel: Arpeggiator, MiniSynth, Distortion with its Drive at 0.9.
    let rack = &mut rig.app.song.channels[1].rack;
    for t in [51, 6] {
        add_module(rack, t, InsertAt::End);
    }
    rack.modules[2].params[1] = 0.9;
    // A DW Sampler channel: its card leaves `Add Module` on screen.
    rig.app
        .add_channel(ChannelKind::Synth(SynthKind::DwSampler), 0);
    rig.open_rack(1);
    rig.slide_rack();
    rig.take();
    let steps = rig.app.history.steps();
    copy_rack(&mut rig, 1);
    assert_eq!(rig.take(), Vec::<String>::new(), "a copy is no song edit");
    let copied = rig.app.song.channels[1].rack.modules.clone();
    assert_eq!(rig.app.slots.clipboard, copied);

    // Into the DW Sampler channel from `+`: appended at the bottom, in order, with fresh ids and the settings.
    rig.open_rack(2);
    rig.slide_rack();
    rig.click(layout(&rig, 2).add_module.center());
    let paste = browser_row(&mut rig, 0).expect("Paste");
    rig.click(paste.center());
    assert_eq!(rig.overlays(), [Overlay::Rack(2)]);
    assert_eq!(types(&rig, 2), [1, 51, 0, 6]);
    let modules = &rig.app.song.channels[2].rack.modules;
    for (pasted, copy) in modules[1..].iter().zip(&copied) {
        let id = pasted.instance_id;
        assert!(id > 2, "a fresh id: {id}");
        let copy = ModuleInstance {
            instance_id: id,
            ..copy.clone()
        };
        assert_eq!(*pasted, copy);
    }
    assert_eq!(rig.app.flash, None);
    assert_eq!(rig.take(), ["ReplaceSong adopting no-resync"]);
    // No undo step: the rack is not covered by undo.
    assert_eq!(rig.app.history.steps(), steps);
}

#[test]
fn a_paste_with_nothing_compatible_flashes_and_changes_nothing() {
    // The MiniSynth alone, copied, then pasted in place of an effect track's Distortion: the instrument is skipped.
    let mut rig = with_effects(&[6]);
    rig.open_rack(1);
    rig.slide_rack();
    copy_rack(&mut rig, 1);
    rig.open_rack(2);
    rig.slide_rack();
    rig.take();
    rig.app.files.saved();
    card_menu(&mut rig, 2, 0, REPLACE);
    let paste = browser_row(&mut rig, 0).expect("Paste");
    rig.click(paste.center());
    assert_eq!(rig.overlays(), [Overlay::Rack(2)]);
    assert_eq!(
        rig.app.flash.as_ref().map(|f| f.0.as_str()),
        Some("Cannot paste module")
    );
    assert_eq!(types(&rig, 2), [6], "Replace keeps its module");
    assert_eq!(rig.take(), Vec::<String>::new());
    assert!(!rig.app.files.modified());
}

#[test]
fn replace_puts_the_pick_in_the_cards_place_with_no_undo_step() {
    let mut rig = with_effects(&[6, 11]);
    rig.app.files.saved();
    let steps = rig.app.history.steps();
    let distortion = instance_id(&rig, 2, 0);
    card_menu(&mut rig, 2, 0, REPLACE);
    let at = InsertAt::Replace(distortion);
    let browser = Overlay::ModulesBrowser { channel: 2, at };
    assert_eq!(rig.overlays(), [Overlay::Rack(2), browser]);
    pick(&mut rig, "Tape D");
    assert_eq!(types(&rig, 2), [18, 11]);
    assert!(
        instance_id(&rig, 2, 0) > instance_id(&rig, 2, 1),
        "a fresh id"
    );
    assert_eq!(rig.take(), ["ReplaceSong adopting no-resync"]);
    // No undo step, and the song is marked changed.
    assert_eq!(rig.app.history.steps(), steps);
    assert!(rig.app.files.modified());
}

#[test]
fn clear_rack_keeps_the_instrument_rebuilds_the_song_with_no_undo_step() {
    let mut rig = Rig::with_synth();
    for t in [51, 6] {
        add_module(&mut rig.app.song.channels[1].rack, t, InsertAt::End);
    }
    rig.app.song_edited(SongEdit::Final, 0);
    rig.open_rack(1);
    rig.slide_rack();
    rig.take();
    rig.app.files.saved();
    let steps = rig.app.history.steps();
    // `Clear Rack`: row 3 without `Load Drumset`.
    let clear_rack = |rig: &mut Rig| {
        rig.click_id(rack_id(1).with("rack menu button"));
        rig.frame(vec![]);
        rig.click_id(rack_id(1).with("rack menu").with((0_usize, 3_usize)));
    };
    clear_rack(&mut rig);
    assert_eq!(types(&rig, 1), [0]);
    assert_eq!(rig.take(), ["ReplaceSong adopting no-resync"]);
    // The rack is not covered by undo: the song is marked changed, with no undo step.
    assert!(rig.app.files.modified());
    assert_eq!(rig.app.history.steps(), steps);
    // Again, with only the instrument left: no edit, so no rebuild.
    clear_rack(&mut rig);
    assert_eq!(types(&rig, 1), [0]);
    assert_eq!(rig.take(), Vec::<String>::new());
}

/// [`with_effects`] with an in-memory provider whose saves download under the file name (the web's), and a handle on
/// it.
pub(super) fn with_web_files(types: &[u16]) -> (Rig, MemFiles) {
    let mut rig = with_effects(types);
    let mem = MemFiles::default();
    mem.0.borrow_mut().names_only = true;
    rig.app.files = ProjectFiles::with_provider(Box::new(mem.clone()));
    (rig, mem)
}

/// `SAVE` in the open Save or preset dialog.
pub(super) fn dialog_save(rig: &mut Rig) {
    let m = Metrics::from_pixels_per_point(1.0, DEFAULT_UI_SCALE);
    let screen = Rect::from_min_size(Pos2::ZERO, vec2(1170.0, 540.0));
    rig.frame(vec![]);
    rig.click(save_dialog::layout(screen, &m).save.center());
}

#[test]
fn save_preset_opens_its_dialog_whose_save_writes_the_module_into_my_presets() {
    let (mut rig, mem) = with_web_files(&[6]);
    rig.app.song.channels[2].rack.modules[0].params[1] = 0.9;
    rig.app.song_edited(SongEdit::Final, 0);
    rig.frame(vec![]);
    rig.take();
    let song = rig.app.song.clone();
    card_menu(&mut rig, 2, 0, SAVE_PRESET);
    let dialog = Overlay::PresetDialog {
        name: "Distortion".into(),
        kind: crate::project_files::PresetKind::Module,
    };
    assert_eq!(rig.overlays(), [Overlay::Rack(2), dialog.clone()]);
    // Modal: Ctrl+S offers no song dialog over it.
    command(&mut rig, egui::Key::S);
    assert_eq!(rig.overlays(), [Overlay::Rack(2), dialog]);
    dialog_save(&mut rig);
    assert_eq!(rig.overlays(), [Overlay::Rack(2)]);
    let path = Path::new("My Presets/Distortion/Distortion.flmpst");
    assert_eq!(mem.0.borrow().saves, [(path.to_owned(), false)]);
    let bytes = rig.app.files.read_user_file(path).unwrap();
    let preset = crate::flm::read_preset(&bytes, None).unwrap();
    assert_eq!(preset.params, song.channels[2].rack.modules[0].params);
    // The card takes the preset's name: nothing for the audio, and no undo step (the rack).
    let steps = rig.app.history.steps();
    rig.frame(vec![]);
    assert_eq!(rig.take(), Vec::<String>::new());
    let mut named = song.clone();
    let module = &mut named.channels[2].rack.modules[0];
    module.name_preset(path, Path::new(""));
    assert_eq!(rig.app.song.channels, named.channels);
    let card = layout(&rig, 2).cards[0];
    let preset_button = Rect::from_min_max(
        pos2(card.right() - 6.5 * U, card.top()),
        pos2(card.right() - 1.5 * U, card.top() + 1.5 * U),
    );
    assert_eq!(texts_in(&rig, preset_button), ["Distortion"]);
    assert_eq!(rig.app.history.steps(), steps);
}

#[test]
fn save_rack_preset_writes_the_racks_chain_into_my_racks() {
    let (mut rig, mem) = with_web_files(&[51, 6]);
    let name = rig.app.song.channels[2].name.clone();
    // `Save Rack preset`: row 1 without `Load Drumset`.
    rig.click_id(rack_id(2).with("rack menu button"));
    rig.frame(vec![]);
    rig.click_id(rack_id(2).with("rack menu").with((0_usize, 1_usize)));
    let dialog = Overlay::PresetDialog {
        name: name.clone(),
        kind: crate::project_files::PresetKind::Rack,
    };
    assert_eq!(rig.overlays(), [Overlay::Rack(2), dialog]);
    dialog_save(&mut rig);
    assert_eq!(rig.overlays(), [Overlay::Rack(2)]);
    let path = Path::new("My Racks").join(format!("{name}.flmrack"));
    assert_eq!(mem.0.borrow().saves, [(path.clone(), false)]);
    let mut last = 1;
    let bytes = rig.app.files.read_user_file(&path).unwrap();
    let modules = crate::flm::read_rack(&bytes, &mut last).unwrap();
    assert_eq!(
        modules.iter().map(|m| m.type_id).collect::<Vec<_>>(),
        [51, 6]
    );
    assert_eq!(rig.take(), Vec::<String>::new());
}

// ---- Loading presets ----

pub(super) const PICKER: &str = "preset_picker";

/// The preset button of module `instance_id`'s card in `channel`'s rack.
pub(super) fn preset_button(channel: usize, instance_id: i32) -> Id {
    rack_id(channel)
        .with(("rack card", instance_id))
        .with("preset")
}

/// Waits out the picker's slide-in and clicks its row `i` (at the root: `My Files`, then `Default`). A sample picker's
/// pick, which decodes on the worker, lands before it returns ([`settle`]).
pub(super) fn picker_row(rig: &mut Rig, i: usize) {
    click_row(rig, i);
    settle(rig);
}

/// [`picker_row`] without waiting for the pick: a test that holds the worker sees it while it decodes.
pub(super) fn click_row(rig: &mut Rig, i: usize) {
    for _ in 0..3 {
        rig.frame(vec![]);
    }
    rig.click_id(Id::new(PICKER).with(("row", i)));
}

/// Frames until a sample picker's pick decoding on the worker has landed: at once when none decodes.
pub(super) fn settle(rig: &mut Rig) {
    // Joined, not timed: a loaded machine only makes the worker slower. Only while it decodes, as a test may hold
    // another picker's worker meanwhile.
    for _ in 0..10 {
        if !rig.app.slots.sample_picker.loading() {
            return;
        }
        join_workers();
        rig.frame(vec![]);
    }
    panic!("the pick never landed");
}

/// The rack ≡ menu's `Load Rack preset` (row 0 without `Load Drumset`) in `channel`'s open rack.
fn load_rack_preset(rig: &mut Rig, channel: usize) {
    rig.click_id(rack_id(channel).with("rack menu button"));
    rig.frame(vec![]);
    rig.click_id(rack_id(channel).with("rack menu").with((0_usize, 0_usize)));
}

/// `Save Rack preset` (row 1 without `Load Drumset`) in `channel`'s open rack, under the name proposed.
fn save_rack_preset(rig: &mut Rig, channel: usize) {
    rig.click_id(rack_id(channel).with("rack menu button"));
    rig.frame(vec![]);
    rig.click_id(rack_id(channel).with("rack menu").with((0_usize, 1_usize)));
    dialog_save(rig);
    rig.frame(vec![]);
}

fn picker(channel: usize, target: PresetTarget) -> Overlay {
    Overlay::PresetPicker { channel, target }
}

fn message(rig: &Rig) -> Option<Vec<String>> {
    rig.overlays().into_iter().find_map(|o| match o {
        Overlay::Message { lines, .. } => Some(lines),
        _ => None,
    })
}

#[test]
fn the_preset_button_loads_default_or_a_saved_preset_staying_open_with_no_undo_step() {
    let (mut rig, _) = with_web_files(&[24]);
    // A Tuner with a setting (its CSTM) and a common param of its own, saved as the preset `Tuner`.
    let blob = drag_reference(&mut rig, 2);
    rig.app.song.channels[2].rack.modules[0].common.0[1] = 0.3;
    rig.app.song_edited(SongEdit::Final, 0);
    card_menu(&mut rig, 2, 0, SAVE_PRESET);
    dialog_save(&mut rig);
    rig.frame(vec![]);
    let module = |rig: &Rig| rig.app.song.channels[2].rack.modules[0].clone();
    let saved = module(&rig);
    assert_eq!(saved.file.preset_name, "Tuner");
    rig.take();
    let id = saved.instance_id;
    let target = PresetTarget::Module(id);
    let steps = rig.app.history.steps();

    // `Default`: the module's defaults, kept in its place.
    rig.click_id(preset_button(2, id));
    assert_eq!(rig.overlays(), [Overlay::Rack(2), picker(2, target)]);
    rig.app.files.saved();
    picker_row(&mut rig, 1);
    assert!(rig.app.files.modified(), "a pick marks the song changed");
    // A pick applies at once and the picker stays open.
    assert_eq!(rig.overlays(), [Overlay::Rack(2), picker(2, target)]);
    let reset = module(&rig);
    let default = ModuleInstance::new(24, id);
    assert_eq!(reset, default);
    assert_eq!(rig.take(), ["ReplaceSong adopting no-resync"]);

    // `My Files`, then the preset saved: all of it is back, under its name.
    picker_row(&mut rig, 0);
    picker_row(&mut rig, 0);
    assert_eq!(rig.overlays(), [Overlay::Rack(2), picker(2, target)]);
    assert_eq!(module(&rig), saved);
    assert_eq!(module(&rig).file.custom_data, blob);
    assert_eq!(rig.take(), ["ReplaceSong adopting no-resync"]);
    // Closing keeps the last pick.
    close_picker(&mut rig);
    assert_eq!(rig.overlays(), [Overlay::Rack(2)]);
    assert_eq!(module(&rig), saved);
    // No undo step.
    assert_eq!(rig.app.history.steps(), steps);
}

#[test]
fn a_preset_that_does_not_load_shows_the_load_error_and_changes_nothing() {
    let (mut rig, _) = with_web_files(&[24]);
    // A Tuner preset whose CSTM is too short for a Tuner.
    let mut bytes = crate::flm::write_preset(&ModuleInstance::new(24, 0)).unwrap();
    bytes.extend(b"CSTM\x02\0\0\0\0\0");
    let path = Path::new("My Presets/Tuner/Broken.flmpst");
    rig.app.files.keep_user_file(path, bytes);
    let song = rig.app.song.clone();
    let steps = rig.app.history.steps();
    let id = instance_id(&rig, 2, 0);
    rig.click_id(preset_button(2, id));
    picker_row(&mut rig, 0);
    picker_row(&mut rig, 0);
    let lines = message(&rig).expect("a message");
    assert_eq!(lines[0], crate::project_files::LOAD_FAILED);
    assert_eq!(rig.overlays().len(), 2, "the message replaces the picker");
    assert_eq!(rig.app.song, song);
    assert_eq!(rig.take(), Vec::<String>::new());
    // No undo step.
    assert_eq!(rig.app.history.steps(), steps);
}

#[test]
fn load_rack_preset_replaces_the_racks_modules_staying_open_with_no_undo_step() {
    let (mut rig, _) = with_web_files(&[51, 6]);
    save_rack_preset(&mut rig, 2);
    card_menu(&mut rig, 2, 0, DELETE);
    assert_eq!(types(&rig, 2), [6]);
    rig.take();
    rig.app.files.saved();
    let steps = rig.app.history.steps();
    load_rack_preset(&mut rig, 2);
    assert_eq!(
        rig.overlays(),
        [Overlay::Rack(2), picker(2, PresetTarget::RackReplace)]
    );
    picker_row(&mut rig, 0);
    picker_row(&mut rig, 0);
    assert_eq!(
        rig.overlays(),
        [Overlay::Rack(2), picker(2, PresetTarget::RackReplace)]
    );
    assert_eq!(types(&rig, 2), [51, 6]);
    // The song is marked changed.
    assert!(rig.app.files.modified());
    let last = rig.app.song.channels[2].rack.last_instance_id;
    assert_eq!(
        (instance_id(&rig, 2, 0), instance_id(&rig, 2, 1)),
        (last - 1, last)
    );
    assert_eq!(rig.take(), ["ReplaceSong adopting no-resync"]);
    // No undo step (the `Delete` before it made none either).
    assert_eq!(rig.app.history.steps(), steps);
}

/// Clicks row `i` of a list shown in the Modules browser's panel (`Rack Preset`, `Instrument`), once it has drawn.
pub(super) fn nested_row(rig: &mut Rig, i: usize) {
    browser_row(rig, i);
    rig.click_id(Id::new(BROWSER).with(("row", i)));
    rig.frame(vec![]);
}

#[test]
fn the_browsers_rack_preset_lists_in_its_panel_and_each_pick_replaces_the_rack_staying_open() {
    let (mut rig, _) = with_web_files(&[6]);
    // The synth channel's rack (MiniSynth, Chorus), saved; then a Distortion under them.
    add_module(&mut rig.app.song.channels[1].rack, 11, InsertAt::End);
    rig.app.song_edited(SongEdit::Final, 0);
    rig.open_rack(1);
    rig.slide_rack();
    save_rack_preset(&mut rig, 1);
    add_module(&mut rig.app.song.channels[1].rack, 6, InsertAt::End);
    rig.app.song_edited(SongEdit::Final, 0);
    rig.take();
    let steps = rig.app.history.steps();
    // Insert above the Distortion: the browser gives way to its rack list, whose Back at its root comes back.
    let at = InsertAt::Above(instance_id(&rig, 1, 2));
    rig.app
        .navigate(Nav::Open(Overlay::ModulesBrowser { channel: 1, at }));
    nested_row(&mut rig, 1);
    let list = picker(1, PresetTarget::ModulesRack(at));
    assert_eq!(rig.overlays(), [Overlay::Rack(1), list.clone()]);
    browser_row(&mut rig, 0);
    rig.click_id(Id::new(BROWSER).with("back"));
    let browser = Overlay::ModulesBrowser { channel: 1, at };
    assert_eq!(rig.overlays(), [Overlay::Rack(1), browser]);
    // My Files, the preset: it replaces the whole rack (the Distortion goes, the modules are new), and the list stays
    // open.
    let before = instance_id(&rig, 1, 2);
    nested_row(&mut rig, 1);
    nested_row(&mut rig, 0);
    nested_row(&mut rig, 0);
    assert_eq!(types(&rig, 1), [0, 11]);
    assert!(instance_id(&rig, 1, 0) > before);
    assert_eq!(rig.overlays(), [Overlay::Rack(1), list]);
    assert_eq!(rig.take(), ["ReplaceSong adopting no-resync"]);
    // No undo step.
    assert_eq!(rig.app.history.steps(), steps);
}

#[test]
fn closing_the_picker_the_browser_gave_way_to_goes_back_to_the_rack() {
    let (mut rig, _) = with_web_files(&[6]);
    let add = layout(&rig, 2).add_module;
    rig.click(add.center());
    browser_row(&mut rig, 1);
    rig.click_id(Id::new(BROWSER).with(("row", 1_usize)));
    let target = PresetTarget::ModulesRack(InsertAt::End);
    assert_eq!(rig.overlays(), [Overlay::Rack(2), picker(2, target)]);
    rig.frame(vec![]);
    rig.frame(escape());
    assert_eq!(rig.overlays(), [Overlay::Rack(2)]);
    assert_eq!(types(&rig, 2), [6]);
    assert_eq!(rig.take(), Vec::<String>::new());
}

#[test]
fn the_add_channel_rack_picks_a_rack_preset_for_a_new_selected_channel() {
    let (mut rig, _) = with_web_files(&[51, 6]);
    save_rack_preset(&mut rig, 2);
    rig.app.navigate(Nav::Close);
    rig.frame(vec![]);
    rig.take();
    let center = |rig: &Rig, id: Id| rig.ctx.read_response(id).unwrap().rect.center();
    let add_rack = |rig: &mut Rig| {
        rig.click(center(rig, Id::new("add_channel_button")));
        rig.frame(vec![]);
        rig.click(center(
            rig,
            Id::new("add_channel_popup").with((0_usize, 6_usize)),
        ));
    };
    // Closing the picker adds nothing.
    add_rack(&mut rig);
    let target = PresetTarget::NewChannel(None);
    assert_eq!(rig.overlays(), [picker(0, target)]);
    rig.frame(vec![]);
    rig.frame(escape());
    assert_eq!((rig.overlays(), rig.app.song.channels.len()), (vec![], 3));
    // A pick adds a Rack channel holding the preset, selected: one edit.
    add_rack(&mut rig);
    picker_row(&mut rig, 0);
    picker_row(&mut rig, 0);
    assert_eq!(rig.app.song.channels.len(), 4);
    assert_eq!(rig.app.song.channels[3].kind, ChannelKind::Rack);
    assert_eq!(types(&rig, 3), [51, 6]);
    // No clip: only a new Drums, Instrument or Synth channel gets one.
    assert!(
        rig.app.song.channels[3]
            .lanes
            .iter()
            .all(|l| l.clips.is_empty())
    );
    assert_eq!(rig.app.selected_channel, 3);
    assert_eq!(rig.take(), ["ReplaceSong adopting no-resync"]);
    rig.frame(z(egui::Modifiers::NONE));
    assert_eq!(rig.app.song.channels.len(), 3);

    // `Insert Channel` puts it at its index, and selects it there.
    rig.app.insert_channel(1, ChannelKind::Rack, 0);
    assert_eq!(
        rig.overlays(),
        [picker(0, PresetTarget::NewChannel(Some(1)))]
    );
    picker_row(&mut rig, 0);
    picker_row(&mut rig, 0);
    assert_eq!(rig.app.song.channels[1].kind, ChannelKind::Rack);
    assert_eq!(types(&rig, 1), [51, 6]);
    assert_eq!(rig.app.selected_channel, 1);
}

#[test]
fn an_instrument_preset_renames_a_channel_never_renamed_and_leaves_a_renamed_one() {
    let mut rig = Rig::with_synth();
    let mut bass = ModuleInstance::new(0, 0);
    bass.params[1] = 0.25;
    let path = Path::new("My Presets/MiniSynth/Bass.flmpst");
    rig.app
        .files
        .keep_user_file(path, crate::flm::write_preset(&bass).unwrap());
    rig.open_rack(1);
    rig.slide_rack();
    let id = instance_id(&rig, 1, 0);
    let pick = |rig: &mut Rig, rows: &[usize]| {
        rig.click_id(preset_button(1, id));
        for &row in rows {
            picker_row(rig, row);
        }
        close_picker(rig);
    };
    let names = |rig: &Rig| {
        let ch = &rig.app.song.channels[1];
        (ch.name.clone(), ch.file.preset_name.clone())
    };
    pick(&mut rig, &[0, 0]);
    assert_eq!(names(&rig), ("Bass".into(), "Bass".into()));
    assert_eq!(rig.app.song.channels[1].rack.modules[0].params[1], 0.25);
    // `Default` gives back the base name the writer would write.
    pick(&mut rig, &[1]);
    assert_eq!(names(&rig), ("MiniSynth 1".into(), String::new()));
    // Renamed by the user: the name stays.
    rig.app.song.channels[1].name = "Keys".into();
    rig.app.song_edited(SongEdit::Final, 0);
    pick(&mut rig, &[0, 0]);
    assert_eq!(names(&rig), ("Keys".into(), "Bass".into()));

    // A second MiniSynth ("MiniSynth 2", id 3 after a Drums channel) that never had a preset keeps its name on
    // `Default`: no duplicate name, and no base name either.
    rig.app.song.channels[1].name = "MiniSynth".into();
    rig.app.add_default_drums(0);
    rig.app
        .add_channel(ChannelKind::Synth(SynthKind::MiniSynth), 0);
    let ch = &rig.app.song.channels[3];
    assert_eq!((&*ch.name, ch.id), ("MiniSynth 2", 3));
    rig.open_rack(3);
    rig.slide_rack();
    let id = instance_id(&rig, 3, 0);
    rig.click_id(preset_button(3, id));
    picker_row(&mut rig, 1);
    let ch = &rig.app.song.channels[3];
    assert_eq!((&*ch.name, &*ch.file.preset_name), ("MiniSynth 2", ""));
}

/// The open preset dialog's `File Name` field: set to `name`.
fn type_name(rig: &mut Rig, name: &str) {
    let Some(Overlay::PresetDialog { name: field, .. }) = rig.app.nav.overlays.last_mut() else {
        panic!("no preset dialog");
    };
    *field = name.into();
}

#[test]
fn a_typed_name_that_is_taken_asks_and_replace_files_writes_over_it() {
    let (mut rig, mem) = with_web_files(&[6]);
    card_menu(&mut rig, 2, 0, SAVE_PRESET);
    dialog_save(&mut rig);
    rig.app.song.channels[2].rack.modules[0].params[1] = 0.25;
    card_menu(&mut rig, 2, 0, SAVE_PRESET);
    type_name(&mut rig, "Distortion");
    dialog_save(&mut rig);
    let [
        Overlay::Rack(2),
        Overlay::PresetDialog { .. },
        Overlay::Message { lines, .. },
    ] = &rig.overlays()[..]
    else {
        panic!("no question over the dialog: {:?}", rig.overlays());
    };
    assert_eq!(lines[0], "The file already exists");
    // Its choices follow its two lines: `Replace files`, `Keep both`, `Cancel`.
    rig.frame(vec![]);
    rig.click_id(message_item(2));
    rig.frame(vec![]);
    assert_eq!(rig.overlays(), [Overlay::Rack(2)]);
    let path = Path::new("My Presets/Distortion/Distortion.flmpst");
    let saves: Vec<_> = mem
        .0
        .borrow()
        .saves
        .iter()
        .map(|(p, _)| p.clone())
        .collect();
    assert_eq!(saves, [path, path]);
    let bytes = rig.app.files.read_user_file(path).unwrap();
    assert_eq!(
        crate::flm::read_preset(&bytes, None).unwrap().params[1],
        0.25
    );
}

#[test]
fn escape_closes_the_preset_dialog_and_drops_what_it_would_save() {
    let (mut rig, mem) = with_web_files(&[6]);
    card_menu(&mut rig, 2, 0, SAVE_PRESET);
    assert!(rig.app.files.preset_open());
    rig.frame(vec![]);
    rig.frame(escape());
    assert_eq!(rig.overlays(), [Overlay::Rack(2)]);
    assert!(!rig.app.files.preset_open());
    assert!(mem.0.borrow().saves.is_empty());
}

#[test]
fn default_never_gives_a_channel_another_channels_name() {
    // Drums (id 1), MiniSynth (id 2, "MiniSynth"), MiniSynth (id 3, "MiniSynth 2").
    let mut rig = Rig::new();
    rig.app.add_default_drums(0);
    for _ in 0..2 {
        rig.app
            .add_channel(ChannelKind::Synth(SynthKind::MiniSynth), 0);
    }
    let path = Path::new("My Presets/MiniSynth/Bass.flmpst");
    let bass = crate::flm::write_preset(&ModuleInstance::new(0, 0)).unwrap();
    rig.app.files.keep_user_file(path, bass);
    rig.open_rack(2);
    rig.slide_rack();
    let id = instance_id(&rig, 2, 0);
    rig.click_id(preset_button(2, id));
    picker_row(&mut rig, 0);
    picker_row(&mut rig, 0);
    assert_eq!(rig.app.song.channels[2].name, "Bass");
    // `Default`: its base name, "MiniSynth 2", is channel 3's, so the name stays.
    close_picker(&mut rig);
    rig.click_id(preset_button(2, id));
    picker_row(&mut rig, 1);
    let names: Vec<_> = (rig.app.song.channels.iter())
        .map(|c| c.name.as_str())
        .collect();
    assert_eq!(names, ["MASTER", "Drums", "Bass", "MiniSynth 2"]);
    assert_eq!(rig.app.song.channels[2].file.preset_name, "");
}

// ---- instruments from the Modules browser ----

/// [`Rig::with_synth`] plus a Rack channel (2) holding `types`, selected, its rack open, with an empty log.
fn with_rack_channel(types: &[u16]) -> Rig {
    let mut rig = Rig::with_synth();
    rig.app.song.add_channel(ChannelKind::Rack);
    for &t in types {
        add_module(&mut rig.app.song.channels[2].rack, t, InsertAt::End);
    }
    rig.app.song_edited(SongEdit::Final, 0);
    rig.open_rack(2);
    rig.slide_rack();
    rig.take();
    rig
}

/// The browser's rows without a search: `Paste` 0, `Rack Preset` 1, `Instrument` 2, SYNTH 3, then 3x Osc 4, Lemon Trio
/// 5, DW Sampler 6, Lemon Sampler 7, GMSynth 8, MiniSynth 9, Slicer 10, SoundFont Player 11, SuperSaw 12, Transistor
/// Bass 13.
const THREE_X_OSC: usize = 4;
const DW_SAMPLER: usize = 6;
const GM_SYNTH: usize = 8;
const INSERT_BELOW: (usize, usize) = (1, 2);

#[test]
fn an_instrument_is_added_from_plus_to_a_rack_or_synth_channel_with_its_defaults_and_no_undo_step()
{
    // A Rack channel, from its `+`: appended at the bottom, as any module.
    let mut rig = with_rack_channel(&[11]);
    let chorus = instance_id(&rig, 2, 0);
    rig.click(layout(&rig, 2).add_module.center());
    let osc = browser_row(&mut rig, THREE_X_OSC).expect("3x Osc is enabled on a Rack channel");
    rig.click(osc.center());
    assert_eq!(types(&rig, 2), [11, 39]);
    let added = rig.app.song.channels[2].rack.modules[1].clone();
    assert!(added.instance_id > chorus, "a fresh id");
    assert_eq!(added, ModuleInstance::new(39, added.instance_id));
    assert_eq!(rig.take(), ["ReplaceSong adopting no-resync"]);
    // No undo step: an undo takes back the Rack channel's setup.
    close_picker(&mut rig);
    assert_eq!(rig.overlays(), [Overlay::Rack(2)]);
    rig.frame(z(egui::Modifiers::NONE));
    assert_eq!(rig.app.song.channels.len(), 2);

    // A Synth channel, which already has its instrument: the second one joins it (no one-instrument rule).
    let mut rig = Rig::with_synth();
    rig.open_rack(1);
    rig.slide_rack();
    rig.click(layout(&rig, 1).add_module.center());
    let gm = browser_row(&mut rig, GM_SYNTH).expect("GMSynth is enabled on a Synth channel");
    rig.click(gm.center());
    assert_eq!(types(&rig, 1), [0, 2]);
    assert_ne!(instance_id(&rig, 1, 0), instance_id(&rig, 1, 1));
    assert_eq!(rig.take(), ["ReplaceSong adopting no-resync"]);
}

#[test]
fn insert_above_below_and_replace_take_an_instrument_like_any_module() {
    // A Rack channel holding Distortion, Chorus; the card menu of the Distortion (card 0).
    let cases = [
        (INSERT_ABOVE, DW_SAMPLER, vec![1, 6, 11]),
        (INSERT_BELOW, GM_SYNTH, vec![6, 2, 11]),
        (REPLACE, THREE_X_OSC, vec![39, 11]),
    ];
    for (item, row, expected) in cases {
        let mut rig = with_rack_channel(&[6, 11]);
        let (distortion, chorus) = (instance_id(&rig, 2, 0), instance_id(&rig, 2, 1));
        card_menu(&mut rig, 2, 0, item);
        let row = browser_row(&mut rig, row).expect("the SYNTH row is enabled");
        rig.click(row.center());
        assert_eq!(types(&rig, 2), expected, "{item:?}");
        assert_eq!(rig.take(), ["ReplaceSong adopting no-resync"], "{item:?}");
        // The new module has the registry defaults and an id none of the rack's modules had.
        let new = expected.iter().position(|&t| t != 6 && t != 11).unwrap();
        let added = &rig.app.song.channels[2].rack.modules[new];
        assert_eq!(
            *added,
            ModuleInstance::new(expected[new], added.instance_id)
        );
        assert!(added.instance_id > chorus.max(distortion), "a fresh id");
        // No undo step: an undo takes back the Rack channel's setup.
        close_picker(&mut rig);
        rig.frame(z(egui::Modifiers::NONE));
        assert_eq!(rig.app.song.channels.len(), 2, "{item:?}");
    }
}

#[test]
fn the_instruments_are_not_listed_on_a_drums_channel_and_an_effect_track() {
    let mut rig = with_effects(&[6]);
    rig.app.add_default_drums(0);
    let (effect_track, drums) = (2, 3);
    assert_eq!(rig.app.song.channels[drums].kind, ChannelKind::Drums);
    let open = |rig: &mut Rig, channel| {
        let at = InsertAt::End;
        rig.app
            .navigate(Nav::Open(Overlay::ModulesBrowser { channel, at }));
    };
    let search = |rig: &mut Rig, text: &str| {
        rig.click_id(Id::new(BROWSER).with("search"));
        rig.frame(vec![Event::Text(text.into())]);
        rig.frame(vec![]);
    };
    for channel in [effect_track, drums] {
        open(&mut rig, channel);
        // `Paste` and `Rack Preset`, then ECHO: no `Instrument`, no SYNTH.
        assert!(browser_row(&mut rig, 1).is_some(), "channel {channel}");
        assert!(
            browser_row(&mut rig, 2).is_none(),
            "channel {channel}: a header"
        );
        // A search finds none.
        search(&mut rig, "MiniSynth");
        assert!(browser_row(&mut rig, 0).is_none(), "channel {channel}");
        rig.app.navigate(Nav::Close);
        rig.frame(vec![]);
    }
    // A Synth channel's are listed, found by search too.
    open(&mut rig, 1);
    assert!(browser_row(&mut rig, 2).is_some() && browser_row(&mut rig, THREE_X_OSC).is_some());
    search(&mut rig, "MiniSynth");
    assert!(browser_row(&mut rig, 1).is_some());
}

#[test]
fn the_preset_picker_follows_its_channel_when_an_earlier_one_comes_back() {
    // FX A (2) and FX B (3), each with a Distortion at Drive 0.9. FX A goes, FX B's preset
    // button opens the picker on 2, and Ctrl+Z brings FX A back there: the picker moves with FX B to 3.
    let mut rig = with_effects(&[6]);
    rig.app.add_channel(ChannelKind::EffectTrack, 0);
    add_module(&mut rig.app.song.channels[3].rack, 6, InsertAt::End);
    for c in [2, 3] {
        rig.app.song.channels[c].rack.modules[0].params[1] = 0.9;
    }
    rig.app.song_edited(SongEdit::Final, 0);
    rig.app.song.delete_channel(2);
    rig.app.song_edited(SongEdit::Final, 0);
    rig.app.navigate(Nav::Open(Overlay::Rack(2)));
    rig.slide_rack();
    rig.frame(vec![]);
    let id = instance_id(&rig, 2, 0);
    let target = PresetTarget::Module(id);
    rig.click_id(preset_button(2, id));
    assert_eq!(rig.overlays(), [Overlay::Rack(2), picker(2, target)]);
    rig.undo_under_modal();
    assert_eq!(rig.overlays(), [Overlay::Rack(3), picker(3, target)]);
    // `Default` resets FX B's Distortion, not FX A's.
    picker_row(&mut rig, 1);
    let drive = |rig: &Rig, c: usize| rig.app.song.channels[c].rack.modules[0].params[1];
    let default = ModuleInstance::new(6, id).params[1];
    assert_eq!((drive(&rig, 2), drive(&rig, 3)), (0.9, default));
}

#[test]
fn the_new_channel_picker_follows_the_channel_it_inserts_above_and_closes_when_it_goes() {
    // `Insert Channel` on the synth (1) puts the Rack channel above it; an effect track added
    // above the synth while the picker is open moves that place to 2.
    let mut rig = Rig::with_synth();
    rig.app.insert_channel(1, ChannelKind::Rack, 0);
    let above = |at| picker(0, PresetTarget::NewChannel(Some(at)));
    assert_eq!(rig.overlays(), [above(1)]);
    rig.app.song.insert_channel(1, ChannelKind::EffectTrack);
    rig.app.song_edited(SongEdit::Final, 0);
    rig.frame(vec![]);
    assert_eq!(rig.overlays(), [above(2)]);
    rig.app.song.delete_channel(2);
    rig.app.song_edited(SongEdit::Final, 0);
    rig.frame(vec![]);
    assert_eq!(rig.overlays(), []);
}

#[test]
fn the_preset_picker_closes_when_its_module_changes_type() {
    // What it lists is for the module's type, as a card's panel state is.
    let mut rig = with_effects(&[6]);
    let id = instance_id(&rig, 2, 0);
    rig.click_id(preset_button(2, id));
    rig.frame(vec![]);
    assert_eq!(rig.overlays().len(), 2);
    rig.app.song.channels[2].rack.modules[0] = ModuleInstance::new(32, id);
    rig.app.song_edited(SongEdit::Final, 0);
    rig.frame(vec![]);
    rig.frame(vec![]);
    assert_eq!(rig.overlays(), [Overlay::Rack(2)]);
}

#[test]
fn saving_an_instrument_preset_names_its_channel_as_loading_it_does() {
    // `Save preset` "Bass" on the MiniSynth names the card and the channel, with nothing for the audio.
    let mut rig = Rig::with_synth();
    let mem = MemFiles::default();
    mem.0.borrow_mut().names_only = true;
    rig.app.files = ProjectFiles::with_provider(Box::new(mem));
    // An edit for the undo below.
    rig.app.song.tempo_bpm += 1.0;
    rig.app.song_edited(SongEdit::Final, 0);
    rig.open_rack(1);
    rig.slide_rack();
    rig.take();
    let names = |rig: &Rig| {
        let ch = &rig.app.song.channels[1];
        let card = &ch.rack.modules[0].file.preset_name;
        (card.clone(), ch.name.clone(), ch.file.preset_name.clone())
    };
    let before = names(&rig);
    card_menu(&mut rig, 1, 0, SAVE_PRESET);
    type_name(&mut rig, "Bass");
    dialog_save(&mut rig);
    rig.frame(vec![]);
    let bass = ("Bass".into(), "Bass".into(), "Bass".into());
    assert_eq!(names(&rig), bass);
    assert_eq!(rig.take(), Vec::<String>::new());
    // No undo step, as for a pick: an undo of the edit before keeps the names.
    assert_ne!(names(&rig), before);
    let tempo = rig.app.song.tempo_bpm;
    rig.frame(z(egui::Modifiers::NONE));
    assert_eq!((rig.app.song.tempo_bpm, names(&rig)), (tempo - 1.0, bass));
}

// ---- The DW Sampler's program picker ----

/// The DW Sampler's channel in [`with_dw_files`].
pub(super) const DW: usize = 2;

/// [`Rig::with_synth`] with the web's files and a DW Sampler channel (2), its rack open, Attack 0.3 and Release 0.4,
/// with an empty log.
pub(super) fn with_dw_files() -> Rig {
    let mut rig = Rig::with_synth();
    let mem = MemFiles::default();
    mem.0.borrow_mut().names_only = true;
    rig.app.files = ProjectFiles::with_provider(Box::new(mem));
    rig.app
        .add_channel(ChannelKind::Synth(SynthKind::DwSampler), 0);
    let module = &mut rig.app.song.channels[DW].rack.modules[0];
    (module.params[1], module.params[2]) = (0.3, 0.4);
    rig.app.song_edited(SongEdit::Final, 0);
    rig.open_rack(DW);
    rig.slide_rack();
    rig.take();
    rig
}

fn dw_record(tag: u32, data: &[u8]) -> Vec<u8> {
    [
        &tag.to_le_bytes()[..],
        &(data.len() as u64).to_le_bytes(),
        data,
    ]
    .concat()
}

/// `.dwp` with one zone whose 0x1FD floats are `floats`.
fn one_zone_dwp(floats: [f32; 4]) -> Vec<u8> {
    let floats = dw_record(0x1FD, &floats.map(f32::to_le_bytes).concat());
    let zone = dw_record(3, &[dw_record(0x1F6, b"C4.wav"), floats].concat());
    [&b"DwPr"[..], &38u32.to_le_bytes(), &dw_record(1, &zone)].concat()
}

fn dw_chunk(id: &[u8; 4], payload: &[u8]) -> Vec<u8> {
    [&id[..], &(payload.len() as u32).to_le_bytes(), payload].concat()
}

/// `PATh` naming `path`, with `tail` after it.
fn dw_path_chunk(path: &[u8], tail: &[u8]) -> Vec<u8> {
    dw_chunk(
        b"PATh",
        &[&(path.len() as u32).to_le_bytes()[..], path, tail].concat(),
    )
}

fn dw_pair(a: f32, r: f32) -> Vec<u8> {
    [a.to_le_bytes(), r.to_le_bytes()].concat()
}

/// Frames until the pickers' worker is done and its pick applied (the picker stays open).
pub(super) fn worker_done(rig: &mut Rig) {
    // Joined, not timed: a loaded machine only makes the worker slower.
    for _ in 0..10 {
        if !rig.app.slots.preset_picker.loading() && !rig.app.slots.sample_picker.loading() {
            rig.frame(vec![]);
            return;
        }
        join_workers();
        rig.frame(vec![]);
    }
    panic!("the pick never landed");
}

fn dw_module(rig: &Rig) -> ModuleInstance {
    rig.app.song.channels[DW].rack.modules[0].clone()
}

#[test]
fn a_picked_dwp_loads_off_the_ui_thread_and_sets_only_attack_and_release() {
    let mut rig = with_dw_files();
    let program = one_zone_dwp([0.25, 1.0, 1.0, 0.6]);
    rig.app
        .files
        .keep_user_file(Path::new("My Presets/DW Sampler/Keys.dwp"), program);
    let before = dw_module(&rig);
    let id = before.instance_id;
    let steps = rig.app.history.steps();
    rig.click_id(preset_button(DW, id));
    picker_row(&mut rig, 0);
    // The program picker lists the `.dwp` beside the module presets.
    picker_row(&mut rig, 0);
    // The worker's result is applied on a later frame, on the UI thread.
    assert_eq!(
        dw_module(&rig),
        before,
        "nothing applied in the pick's frame"
    );
    worker_done(&mut rig);
    let target = PresetTarget::Module(id);
    assert_eq!(rig.overlays(), [Overlay::Rack(DW), picker(DW, target)]);
    let module = dw_module(&rig);
    assert_eq!((module.params[1], module.params[2]), (0.25, 0.6));
    for (i, (now, was)) in module.params.iter().zip(&before.params).enumerate() {
        assert!(i == 1 || i == 2 || now == was, "param {i}");
    }
    let path = b"@L@\0My Presets/DW Sampler/Keys.dwp";
    let blob = [
        &b"10WD"[..],
        &dw_path_chunk(path, &dw_pair(0.25, 0.6)),
        &dw_chunk(b"MIDI", &[2, 0]),
    ]
    .concat();
    assert_eq!(module.file.custom_data, Some(blob));
    // The preset name and info strings are stored, and the instrument names its channel.
    assert_eq!(module.file.preset_name, "Keys");
    assert_eq!(module.file.provenance, "My Presets/DW Sampler/Keys");
    assert_eq!(rig.app.song.channels[DW].name, "Keys");
    assert_eq!(rig.take(), ["ReplaceSong adopting no-resync"]);
    // No undo step.
    assert_eq!(rig.app.history.steps(), steps);
}

#[test]
fn a_dw_preset_the_program_loader_rejects_loads_its_prms_then_its_path() {
    let mut rig = with_dw_files();
    // A module preset: PRMS Attack 0.1 and Release 0.2, its CSTM naming a program with the floats 0.7 and 0.8.
    let mut preset = ModuleInstance::new(1, 0);
    (preset.params[1], preset.params[2], preset.params[3]) = (0.1, 0.2, 0.9);
    let path = b"@L@\0My Presets/DW Sampler/Keys.dwp";
    let cstm = [&b"10WD"[..], &dw_path_chunk(path, &dw_pair(0.7, 0.8))].concat();
    preset.file.custom_data = Some(cstm);
    preset.file.program_init = Some([0.7, 0.8]);
    let bytes = crate::flm::write_preset(&preset).unwrap();
    let files = &mut rig.app.files;
    files.keep_user_file(Path::new("My Presets/DW Sampler/Piano.flmpst"), bytes);
    let id = instance_id(&rig, DW, 0);
    let pick = |rig: &mut Rig, row: usize| {
        rig.click_id(preset_button(DW, id));
        picker_row(rig, 0);
        picker_row(rig, row);
        worker_done(rig);
        close_picker(rig);
    };
    // Its program is missing: the saved PRMS stay, and the floats stay its init values.
    pick(&mut rig, 0);
    let module = dw_module(&rig);
    assert_eq!(&module.params[1..4], [0.1, 0.2, 0.9]);
    assert_eq!(module.file.program_init, Some([0.7, 0.8]));
    assert_eq!(module.file.preset_name, "Piano");
    // Its program is there: the floats set Attack and Release.
    let program = one_zone_dwp([0.25, 1.0, 1.0, 0.6]);
    let files = &mut rig.app.files;
    files.keep_user_file(Path::new("My Presets/DW Sampler/Keys.dwp"), program);
    // `Keys` (the program), then `Piano`.
    pick(&mut rig, 1);
    let module = dw_module(&rig);
    assert_eq!(&module.params[1..4], [0.7, 0.8, 0.9]);
    assert_eq!(module.file.program_init, None);
}

#[test]
fn a_file_no_dw_loader_takes_clears_its_zones_and_shows_the_load_error() {
    let mut rig = with_dw_files();
    let mut zone = b"@L@\0z.wav".to_vec();
    zone.resize(1065, 0);
    let midi = dw_chunk(b"MIDI", &[7, 0]);
    let blob = [&b"10WD"[..], &dw_chunk(b"ZONE", &zone), &midi].concat();
    rig.app.song.channels[DW].rack.modules[0].file.custom_data = Some(blob);
    rig.app.song_edited(SongEdit::Final, 0);
    rig.frame(vec![]);
    rig.take();
    // A DW Sampler preset whose CSTM is too short: neither the program loader nor a module preset takes it.
    let mut broken = crate::flm::write_preset(&ModuleInstance::new(1, 0)).unwrap();
    broken.extend(b"CSTM\x02\0\0\0\0\0");
    let files = &mut rig.app.files;
    files.keep_user_file(Path::new("My Presets/DW Sampler/Broken.flmpst"), broken);
    let before = dw_module(&rig);
    rig.click_id(preset_button(DW, before.instance_id));
    picker_row(&mut rig, 0);
    picker_row(&mut rig, 0);
    worker_done(&mut rig);
    let lines = message(&rig).expect("a message");
    assert_eq!(lines[0], crate::project_files::LOAD_FAILED);
    // Before rejecting, the DW loader has cleared the zones; MIDI and the params stay.
    let module = dw_module(&rig);
    assert_eq!(
        module.file.custom_data,
        Some([&b"10WD"[..], &midi].concat())
    );
    assert_eq!(module.params, before.params);
    assert_eq!(rig.take(), ["ReplaceSong adopting no-resync"]);
}

/// Clicks `Keys` in the DW Sampler's picker (its program, kept as a user file), leaving the worker's result to apply
/// on a later frame.
pub(super) fn pick_keys(rig: &mut Rig) {
    let program = one_zone_dwp([0.25, 1.0, 1.0, 0.6]);
    rig.app
        .files
        .keep_user_file(Path::new("My Presets/DW Sampler/Keys.dwp"), program);
    rig.click_id(preset_button(DW, instance_id(rig, DW, 0)));
    picker_row(rig, 0);
    picker_row(rig, 0);
}

#[test]
fn a_pick_lands_on_its_module_when_a_channel_is_inserted_before_it_applies() {
    let mut rig = with_dw_files();
    pick_keys(&mut rig);
    // Before the result applies, a DW Sampler channel goes in at the picked one's index; its module is id 2 too.
    rig.app
        .insert_bare_channel(DW, ChannelKind::Synth(SynthKind::DwSampler), 0);
    worker_done(&mut rig);
    let channels = &rig.app.song.channels;
    let inserted = &channels[DW];
    assert_eq!(inserted.rack.modules[0].file.custom_data, None);
    assert_ne!(inserted.name, "Keys");
    let picked = &channels[DW + 1];
    let module = &picked.rack.modules[0];
    assert_eq!((module.params[1], module.params[2]), (0.25, 0.6));
    assert_eq!(picked.name, "Keys");
}

#[test]
fn a_tap_outside_while_a_pick_loads_closes_the_picker_once_the_pick_lands_and_opens_nothing() {
    let mut rig = with_dw_files();
    // A Distortion under the DW Sampler, with its own preset button.
    add_module(&mut rig.app.song.channels[DW].rack, 6, InsertAt::End);
    rig.app.song_edited(SongEdit::Final, 0);
    rig.frame(vec![]);
    let (id, distortion) = (instance_id(&rig, DW, 0), instance_id(&rig, DW, 1));
    let before = dw_module(&rig);
    let hold = crate::ui::views::preset_picker::WORKER_HOLD
        .lock()
        .unwrap_or_else(std::sync::PoisonError::into_inner);
    pick_keys(&mut rig);
    // The Distortion's preset button while the pick loads: a tap outside the panel, which closes it and is consumed,
    // so no other picker opens in its place (which would cancel the pick); closing waits for the pick to land.
    rig.click_id(preset_button(DW, distortion));
    rig.frame(vec![]);
    let loading = picker(DW, PresetTarget::Module(id));
    assert_eq!(rig.overlays(), [Overlay::Rack(DW), loading]);
    assert_eq!(dw_module(&rig), before, "still loading");
    // Then the pick lands.
    drop(hold);
    worker_done(&mut rig);
    assert_eq!(rig.overlays(), [Overlay::Rack(DW)]);
    assert_eq!(dw_module(&rig).params[1], 0.25);
}

#[test]
fn a_knob_move_sets_the_kept_init_value_a_save_writes() {
    let mut rig = with_dw_files();
    // As a song whose program is missing opens: PRMS 0.3 and 0.4, its PATh floats 0.7 and 0.8 kept as init values.
    let path = b"@L@\0My Presets/DW Sampler/Keys.dwp";
    let module = &mut rig.app.song.channels[DW].rack.modules[0];
    module.file.custom_data =
        Some([&b"10WD"[..], &dw_path_chunk(path, &dw_pair(0.7, 0.8))].concat());
    module.file.program_init = Some([0.7, 0.8]);
    rig.app.song_edited(SongEdit::Final, 0);
    rig.frame(vec![]);
    rig.take();
    // Attack (p1), dragged up, not recording: its init value follows; Release's stays.
    let module = dw_module(&rig);
    let content = content_rect(layout(&rig, DW).cards[0], 7.5, U);
    let panel = panels::panel_for(1).unwrap();
    let placed = panels::place(panel, &module, &PanelState::default(), content, U);
    let slider = placed
        .iter()
        .find(|(l, _)| l.id == 11300)
        .unwrap()
        .1
        .center();
    rig.press(slider);
    for step in 1..=4 {
        rig.drag_to(slider - vec2(0.0, U * step as f32 / 4.0));
    }
    rig.release(slider - vec2(0.0, U));
    let moved = dw_module(&rig);
    let attack = moved.params[1];
    assert!(attack > 0.3, "{attack}");
    assert_eq!(moved.file.program_init, Some([attack, 0.8]));
    // Saved, then opened once the program is installed: the move is the Attack, and Release keeps its init value.
    let bytes = crate::flm::write(&rig.app.song).unwrap();
    let mut song = crate::flm::read(&bytes).unwrap();
    let program = one_zone_dwp([0.25, 1.0, 1.0, 0.6]);
    let keys = crate::flm::chunk::ContentPath::Shorthand {
        root: crate::flm::chunk::ContentRoot::User,
        relative: "My Presets/DW Sampler/Keys.dwp".into(),
    };
    crate::panels::cstm::load_dw_programs(&mut song, &|p| (*p == keys).then(|| program.clone()));
    let module = &song.channels[DW].rack.modules[0];
    assert_eq!((module.params[1], module.params[2]), (attack, 0.8));
}

#[test]
fn a_rack_preset_loads_its_dw_samplers_program() {
    let mut rig = with_dw_files();
    let mut dw = ModuleInstance::new(1, 2);
    let path = b"@L@\0My Presets/DW Sampler/Keys.dwp";
    dw.file.custom_data = Some([&b"10WD"[..], &dw_path_chunk(path, &[])].concat());
    let files = &mut rig.app.files;
    files.keep_user_file(
        Path::new("My Racks/Keys.flmrack"),
        crate::flm::write_rack(&[dw]).unwrap(),
    );
    let program = one_zone_dwp([0.25, 1.0, 1.0, 0.6]);
    files.keep_user_file(Path::new("My Presets/DW Sampler/Keys.dwp"), program);
    load_rack_preset(&mut rig, DW);
    picker_row(&mut rig, 0);
    picker_row(&mut rig, 0);
    let modules = &rig.app.song.channels[DW].rack.modules;
    let loaded = modules.iter().filter(|m| m.instance_id != 2);
    let attack_release: Vec<_> = loaded
        .map(|m| (m.type_id, m.params[1], m.params[2]))
        .collect();
    assert_eq!(attack_release, [(1, 0.25, 0.6)]);
}

#[test]
fn a_failed_streams_meters_reach_no_panel() {
    // The rack hands its panels the snapshot's meters only while the stream runs; a failed stream's are
    // stale, and the panels then rest as without one.
    use honeydew_audio::MeterSnapshot;
    use honeydew_audio::device::{ModuleKey, ModuleMeter};
    let mut rig = with_effects(&[20]);
    let meter = ModuleMeter {
        key: ModuleKey {
            channel: 2,
            instance_id: instance_id(&rig, 2, 0),
        },
        value: MeterSnapshot {
            peak_left: 1.0,
            ..MeterSnapshot::default()
        },
        analysis: vec![0.5],
    };
    rig.rec.0.borrow_mut().meters = vec![meter.clone()];
    rig.frame(vec![]);
    assert_eq!(rig.app.slots.rack.meters(), Some(&[meter][..]));
    rig.rec.0.borrow_mut().failed = true;
    rig.frame(vec![]);
    assert_eq!(rig.app.slots.rack.meters(), None);
}

#[test]
fn a_minisynth_flms_preset_is_listed_and_loads_as_a_module_preset_does() {
    let (mut rig, _) = with_web_files(&[]);
    rig.open_rack(1);
    rig.slide_rack();
    rig.take();
    // The magic, then `DATA` = `0rev` + MiniSynth's params from 1.
    let mut flms = b"\x99FLS\r\n\x1a\nDATA\x0c\0\0\x000rev".to_vec();
    flms.extend([0.25_f32, 0.75].iter().flat_map(|v| v.to_le_bytes()));
    let path = Path::new("My Presets/MiniSynth/Fat Bass.flms");
    rig.app.files.keep_user_file(path, flms);
    let id = instance_id(&rig, 1, 0);
    rig.click_id(preset_button(1, id));
    picker_row(&mut rig, 0);
    picker_row(&mut rig, 0);
    let target = PresetTarget::Module(id);
    assert_eq!(rig.overlays(), [Overlay::Rack(1), picker(1, target)]);
    let module = &rig.app.song.channels[1].rack.modules[0];
    assert_eq!((module.type_id, module.instance_id), (0, id));
    assert_eq!(module.params[..3], [1.0, 0.25, 0.75]);
    assert_eq!(module.file.preset_name, "Fat Bass");
    assert_eq!(module.file.category, "My Presets/MiniSynth");
    assert_eq!(rig.take(), ["ReplaceSong adopting no-resync"]);
}

#[test]
fn the_preset_pickers_file_actions_change_the_users_presets_not_the_song() {
    use crate::project_files::PRESET_EXT;
    let (mut rig, _) = with_web_files(&[6]);
    let dir = Path::new("My Presets/Distortion");
    let bytes = crate::flm::write_preset(&ModuleInstance::new(6, 0)).unwrap();
    rig.app
        .files
        .keep_user_file(&dir.join("Crunch.flmpst"), bytes);
    let song = rig.app.song.clone();
    let id = instance_id(&rig, 2, 0);
    rig.click_id(preset_button(2, id));
    picker_row(&mut rig, 0);
    // `Select`, the preset, then the ≡ menu's `Delete` (after `Select all` and `Create folder`).
    let ids = Id::new(PICKER);
    rig.click_id(ids.with("select"));
    rig.click_id(ids.with(("row", 0_usize)));
    rig.click_id(ids.with("options"));
    rig.frame(vec![]);
    rig.click_id(ids.with("options menu").with((0_usize, 2_usize)));
    assert!(rig.app.files.user_files(dir, PRESET_EXT).is_empty());
    let bin = Path::new("/Recently Deleted/My Presets/Distortion");
    assert_eq!(
        rig.app.files.user_files(bin, PRESET_EXT),
        [bin.join("Crunch.flmpst")]
    );
    // The picker stays open; the song and the undo history are untouched.
    assert_eq!(
        rig.overlays(),
        [Overlay::Rack(2), picker(2, PresetTarget::Module(id))]
    );
    assert_eq!(rig.app.song, song);
    assert_eq!(rig.take(), Vec::<String>::new());
}

/// A user folder in the temp folder, removed with it.
struct TempUser(std::path::PathBuf);

impl Drop for TempUser {
    fn drop(&mut self) {
        let _ = std::fs::remove_dir_all(&self.0);
    }
}

/// The rig with its files on disk, as on the desktop, and no audio output (an engine build would load the samples itself,
/// on the UI thread: only the worker can fill the cache). `folder` of the user folder holds `Keys.dwp` and its zone file
/// `Keys/C4.wav` (4 frames). Returns the program's path and its zone sample's key.
fn keys_on_disk(
    rig: &mut Rig,
    tag: &str,
    folder: &str,
) -> (
    TempUser,
    crate::flm::chunk::ContentPath,
    crate::sample_cache::SampleKey,
) {
    use crate::flm::chunk::{ContentPath, ContentRoot};
    let user =
        TempUser(std::env::temp_dir().join(format!("honeydew-e14c-{tag}-{}", std::process::id())));
    let _ = std::fs::remove_dir_all(&user.0);
    let dir = user.0.join(folder);
    std::fs::create_dir_all(dir.join("Keys")).unwrap();
    std::fs::write(dir.join("Keys.dwp"), one_zone_dwp([0.25, 1.0, 1.0, 0.6])).unwrap();
    let mut wav =
        b"RIFF\x2c\0\0\0WAVEfmt \x10\0\0\0\x01\0\x01\0\x44\xac\0\0\x88\x58\x01\0\x02\0\x10\0"
            .to_vec();
    wav.extend(b"data\x08\0\0\0\x01\0\x02\0\x03\0\x04\0");
    std::fs::write(dir.join("Keys/C4.wav"), wav).unwrap();
    rig.app.files = ProjectFiles::on_disk(&user.0);
    (rig.app.audio, rig.no_device) = (Default::default(), true);
    let shorthand = |relative: String| ContentPath::Shorthand {
        root: ContentRoot::User,
        relative,
    };
    let keys = shorthand(format!("{folder}/Keys.dwp"));
    let c4 = crate::sample_cache::SampleKey::File(shorthand(format!("{folder}/Keys/C4.wav")));
    (user, keys, c4)
}

/// The program and its zone sample are in the cache, and the engine build that follows reads nothing.
fn assert_cached(
    rig: &mut Rig,
    keys: &crate::flm::chunk::ContentPath,
    c4: &crate::sample_cache::SampleKey,
) {
    let samples = &mut rig.app.audio.samples;
    assert!(samples.program(keys).is_some());
    assert_eq!(samples.get(c4).map(|s| s.frames()), Some(4));
    let reads = std::cell::Cell::new(0);
    samples.load(&rig.app.song, &|_| {
        reads.set(reads.get() + 1);
        None
    });
    assert_eq!(reads.get(), 0, "the build read a file");
    assert_eq!(samples.missing().count(), 0);
}

#[test]
fn a_picked_programs_samples_decode_on_the_worker_so_the_next_build_reads_nothing() {
    let mut rig = with_dw_files();
    let (_user, keys, c4) = keys_on_disk(&mut rig, "pick", "My Presets/DW Sampler");
    let hold = crate::ui::views::preset_picker::WORKER_HOLD
        .lock()
        .unwrap_or_else(std::sync::PoisonError::into_inner);
    rig.click_id(preset_button(DW, instance_id(&rig, DW, 0)));
    picker_row(&mut rig, 0);
    picker_row(&mut rig, 0);
    rig.frame(vec![]);
    assert!(
        rig.app.audio.samples.program(&keys).is_none(),
        "nothing is decoded on the UI thread"
    );
    drop(hold);
    worker_done(&mut rig);
    assert_eq!(dw_module(&rig).params[1], 0.25);
    assert_cached(&mut rig, &keys, &c4);
}

#[test]
fn a_zone_the_worker_found_missing_is_listed_and_not_read_again_at_the_build() {
    let mut rig = with_dw_files();
    let (user, keys, c4) = keys_on_disk(&mut rig, "missing", "My Presets/DW Sampler");
    // The program is there, its zone file is not.
    std::fs::remove_file(user.0.join("My Presets/DW Sampler/Keys/C4.wav")).unwrap();
    rig.click_id(preset_button(DW, instance_id(&rig, DW, 0)));
    picker_row(&mut rig, 0);
    picker_row(&mut rig, 0);
    worker_done(&mut rig);
    assert_eq!(dw_module(&rig).params[1], 0.25);
    // `finish` forgets the old misses first and takes the worker's after (the other way round, the build would read
    // the file again on the UI thread): the miss is the worker's, and stays one.
    let samples = &mut rig.app.audio.samples;
    assert!(samples.program(&keys).is_some());
    assert_eq!(samples.missing().collect::<Vec<_>>(), [&c4]);
    // The build reads only the grand the now silent sampler plays instead.
    let reads = std::cell::RefCell::new(Vec::new());
    samples.load(&rig.app.song, &|path| {
        reads.borrow_mut().push(path.clone());
        None
    });
    let stand_in = crate::audio_engine::samples::stand_in();
    assert_eq!(reads.into_inner(), [stand_in], "the build read a file");
    assert_eq!(samples.missing().collect::<Vec<_>>(), [&c4]);
}

#[test]
fn a_picked_dw_presets_program_samples_decode_on_the_worker_too() {
    let mut rig = with_dw_files();
    // The preset names a program elsewhere in the user folder, so that it is the only file the picker lists.
    let (_user, keys, c4) = keys_on_disk(&mut rig, "preset", "My Programs");
    let mut piano = ModuleInstance::new(1, 0);
    let path = b"@L@\0My Programs/Keys.dwp";
    piano.file.custom_data =
        Some([&b"10WD"[..], &dw_path_chunk(path, &dw_pair(0.7, 0.8))].concat());
    let bytes = crate::flm::write_preset(&piano).unwrap();
    let folder = rig
        .app
        .files
        .user_dir()
        .unwrap()
        .join("My Presets/DW Sampler");
    std::fs::create_dir_all(&folder).unwrap();
    std::fs::write(folder.join("Piano.flmpst"), bytes).unwrap();
    rig.click_id(preset_button(DW, instance_id(&rig, DW, 0)));
    picker_row(&mut rig, 0);
    picker_row(&mut rig, 0);
    worker_done(&mut rig);
    assert_eq!(dw_module(&rig).file.preset_name, "Piano");
    assert_cached(&mut rig, &keys, &c4);
}

/// `Set Range` learns its bounds from the keys played on the channel (the key range, ADD1).
#[test]
fn set_range_takes_the_next_two_keyboard_keys_and_the_engine_plays_only_inside_them() {
    let mut rig = Rig::with_synth();
    rig.open_rack(1);
    rig.slide_rack();
    card_menu(&mut rig, 1, 0, (0, 4));
    rig.frame(vec![]);
    let log = rig.take();
    let learn = |on: u8| {
        format!(
            "SetSetting 1 2 {} {on}.000",
            honeydew_audio::instrument::KEY_LEARN
        )
    };
    assert!(log.contains(&learn(1)), "{log:?}");
    // F4 then E4, from the on-screen keyboard: the range is E4..=F4 (keys 52, 53), stored in order.
    for key in [F4, E4] {
        rig.click(key);
        rig.frame(vec![]);
    }
    let module = &rig.app.song.channels[1].rack.modules[0];
    assert_eq!((module.file.key_low, module.file.key_high), (52, 53));
    assert!(rig.take().contains(&learn(0)));
    // The engine built from the song plays a key inside the range and not one outside it.
    use honeydew_audio::device::AudioRenderer;
    let sounds = |key| {
        let mut engine =
            crate::audio_engine::SongAudioEngine::new(&rig.app.song, 48_000.0).unwrap();
        engine.note_on(1, key, 1.0);
        let mut out = vec![0.0_f32; 4096 * 2];
        engine.render(&mut out, 2);
        out.iter().any(|s| s.abs() > 1e-4)
    };
    assert!(sounds(52) && sounds(53));
    assert!(!sounds(51) && !sounds(60));
}

/// A key-learn outlives an engine rebuild (`settings::screen_only`), so the learning module stays deaf.
#[test]
fn a_rebuild_during_set_range_keeps_the_learning_module_silent() {
    use honeydew_audio::device::AudioRenderer;
    let mut rig = Rig::with_synth();
    rig.open_rack(1);
    rig.slide_rack();
    card_menu(&mut rig, 1, 0, (0, 4));
    rig.frame(vec![]);
    // A rebuilding edit: a new engine replaces the one that got the learn.
    rig.app.song_edited(SongEdit::Final, 0);
    rig.frame(vec![]);
    // The learn goes to the new engine again (a module built afresh needs it).
    let learn = "SetSetting 1 2 4294967295 1.000";
    assert_eq!(rig.take(), [learn, "ReplaceSong adopting no-resync", learn]);
    // The first key, held: the lower bound, while the module still learns.
    rig.press(E4);
    let mut out = vec![0.0_f32; 4096 * 2];
    let mut rec = rig.rec.0.borrow_mut();
    rec.engine.as_mut().expect("an engine").render(&mut out, 2);
    assert!(
        out.iter().all(|s| *s == 0.0),
        "the learning MiniSynth sounded"
    );
}

/// Opening another song while `Set Range` waits ends the learn, so the new song's module with the same
/// ids is not left deaf by a re-sent `KEY_LEARN` 1.
#[test]
fn opening_a_song_during_set_range_ends_the_learn() {
    use honeydew_audio::device::AudioRenderer;
    use honeydew_audio::instrument::KEY_LEARN;
    let mut rig = Rig::with_synth();
    rig.open_rack(1);
    rig.slide_rack();
    card_menu(&mut rig, 1, 0, (0, 4));
    rig.frame(vec![]);
    rig.take();
    // Another song with the same ids: a MiniSynth channel 1 whose module is instance 2.
    let mut song = crate::song::Song::default();
    song.add_channel(ChannelKind::Synth(SynthKind::MiniSynth));
    rig.app.song = song;
    rig.app.song_edited(SongEdit::Replaced, 0);
    rig.frame(vec![]);
    let log = rig.take();
    let at = log
        .iter()
        .position(|c| c == "ReplaceSong")
        .expect("a new engine");
    let stale = format!("SetSetting 1 2 {KEY_LEARN} 1.000");
    assert!(!log[at..].contains(&stale), "{log:?}");
    rig.press(E4);
    let mut out = vec![0.0_f32; 4096 * 2];
    let mut rec = rig.rec.0.borrow_mut();
    rec.engine.as_mut().expect("an engine").render(&mut out, 2);
    assert!(
        out.iter().any(|s| s.abs() > 1e-4),
        "the new song's MiniSynth is silent"
    );
}

impl Rig {
    /// Opens channel `channel`'s rack and gives the channel the keyboard (the master never takes it), as a header tap
    /// once did; a header tap now only selects the track.
    pub(super) fn open_rack(&mut self, channel: usize) {
        if channel > 0 {
            self.app.selected_channel = channel;
        }
        self.app.navigate(Nav::Open(Overlay::Rack(channel)));
        self.frame(vec![]);
    }
}

/// The right edge of the master track's lane: the playlist grid's.
fn grid_right(rig: &Rig) -> f32 {
    let fill = rig.app.theme.colors.control_fill.to_color32();
    let lane = |s: &egui::epaint::ClippedShape| match &s.shape {
        egui::Shape::Rect(r) if r.fill == fill && r.rect.min == pos2(1.5 * U + 2.0, RULER) => {
            Some(r.rect.right())
        }
        _ => None,
    };
    rig.out
        .shapes
        .iter()
        .find_map(lane)
        .expect("the master lane")
}

#[test]
fn the_rack_slides_in_and_out_and_the_playlist_ends_at_its_left_edge() {
    let mut rig = Rig::with_synth();
    assert_eq!(grid_right(&rig), 1170.0);
    // Opened from the edge tab's path (a header tap only selects).
    rig.app.navigate(Nav::Open(Overlay::Rack(1)));
    assert_eq!(rig.overlays(), [Overlay::Rack(1)]);
    // Half way in on its first frame (20 U = 640 px), which asks for the next.
    rig.frame(vec![]);
    assert_eq!(grid_right(&rig), 1170.0 - 320.0);
    let repaint = |rig: &Rig| rig.out.viewport_output[&egui::ViewportId::ROOT].repaint_delay;
    assert_eq!(repaint(&rig), std::time::Duration::ZERO);
    // The second step asks again: nothing else wants that frame, so this is the slide's own request.
    rig.frame(vec![]);
    assert_eq!(grid_right(&rig), 1170.0 - 480.0);
    assert_eq!(repaint(&rig), std::time::Duration::ZERO);
    for _ in 0..11 {
        rig.frame(vec![]);
    }
    // In: the grid ends where the rack starts, and the rack is drawn where it is hit.
    let rack = rack();
    assert_eq!(grid_right(&rig), rack.left());
    let fill = |rig: &Rig| {
        let rack = |s: &egui::epaint::ClippedShape| matches!(&s.shape, egui::Shape::Rect(r) if r.rect == rack);
        rig.out.shapes.iter().any(rack)
    };
    assert!(fill(&rig), "the rack's fill");
    // The hit test (what keeps the wheel from the view) is that rect too.
    let m = Metrics::from_pixels_per_point(1.0, DEFAULT_UI_SCALE);
    let screen = Rect::from_min_size(Pos2::ZERO, vec2(1170.0, 540.0));
    let regions = Regions::new(screen, &m).with_rack(1.0, &m);
    let covered = |at: Pos2| {
        let (ctx, mut hit) = (egui::Context::default(), false);
        let raw = egui::RawInput {
            screen_rect: Some(screen),
            events: vec![Event::PointerMoved(at)],
            ..Default::default()
        };
        let _ = ctx.run(raw, |ctx| {
            egui::CentralPanel::default().show(ctx, |ui| {
                hit = views::covered(&[Overlay::Rack(1)], &regions, ui);
            });
        });
        hit
    };
    assert!(covered(pos2(rack.left() + 0.5, 100.0)));
    assert!(!covered(pos2(rack.left() - 0.5, 100.0)));
    assert!(
        !covered(pos2(rack.right() - 0.5, PANEL_TOP + 1.0)),
        "the transport bar"
    );
    // The notch closes it: it slides out the same way, still drawn, and the grid widens with it.
    rig.click(collapse_tab());
    assert_eq!(rig.overlays(), []);
    rig.frame(vec![]);
    assert_eq!(grid_right(&rig), 1170.0 - 320.0);
    let half = Rect::from_min_max(pos2(1170.0 - 320.0, 0.0), pos2(1170.0 + 320.0, PANEL_TOP));
    let drawn =
        |s: &egui::epaint::ClippedShape| matches!(&s.shape, egui::Shape::Rect(r) if r.rect == half);
    assert!(rig.out.shapes.iter().any(drawn), "the closing rack's fill");
    for _ in 0..12 {
        rig.frame(vec![]);
    }
    assert_eq!(grid_right(&rig), 1170.0);
    assert!(!rig.out.shapes.iter().any(drawn));
}

/// The middle of the name in the header of the card at `index` in `channel`'s open, unscrolled rack.
fn card_name(rig: &Rig, channel: usize, index: usize) -> Pos2 {
    let card = layout(rig, channel).cards[index];
    pos2(card.left() + 5.0 * U, card.top() + 0.75 * U)
}

#[test]
fn a_double_tap_on_a_card_header_minimizes_it_with_no_undo_step_or_rebuild() {
    let mut rig = with_effects(&[6, 11]);
    rig.app.files.saved();
    let steps = rig.app.history.steps();
    let minimized = |rig: &Rig| rig.app.song.channels[2].rack.modules[0].minimized;
    let name = card_name(&rig, 2, 0);
    // Touch-downs 0.2 s apart (the rig's frames are 0.1 s apart).
    rig.click(name);
    rig.button(name, true);
    rig.release(name);
    assert!(minimized(&rig));
    assert!(rig.app.files.modified());
    assert_eq!(rig.take(), Vec::<String>::new());
    // A double tap again: maximized. The rack is not covered by undo.
    rig.frame(vec![]);
    rig.click(name);
    rig.button(name, true);
    rig.release(name);
    assert!(!minimized(&rig));
    assert_eq!(rig.app.history.steps(), steps);
}

#[test]
fn each_step_of_a_held_card_is_the_menus_move_with_its_own_rebuild() {
    let mut rig = with_effects(&[6, 11, 15]);
    rig.app.files.saved();
    let steps = rig.app.history.steps();
    let name = card_name(&rig, 2, 0);
    rig.press(name);
    let pressed = rig.time;
    rig.next_frame_at(pressed + 0.3);
    rig.frame(vec![]);
    // Held: the cards show minimized, but the song is as it was: nothing to undo, no modified mark, no rebuild.
    let minimized = |rig: &Rig| (rig.app.song.channels[2].rack.modules.iter()).any(|m| m.minimized);
    assert!(!minimized(&rig));
    assert!(!rig.app.files.modified());
    assert_eq!(rig.take(), Vec::<String>::new());
    // Down below the card, a move event every 0.2 s: a move each, into the last slot, where Move down swaps nothing.
    for i in 0..3 {
        rig.drag_to(pos2(name.x, rack().bottom() - 4.0 - i as f32));
        rig.frame(vec![]);
    }
    assert_eq!(types(&rig, 2), [11, 15, 6]);
    rig.release(pos2(name.x, rack().bottom() - 4.0));
    assert_eq!(
        rig.take(),
        [
            "ReplaceSong adopting no-resync",
            "ReplaceSong adopting no-resync"
        ]
    );
    assert!(!minimized(&rig));
    // The song is marked changed, with no undo step: the rack is not covered by undo.
    assert!(rig.app.files.modified());
    assert_eq!(rig.app.history.steps(), steps);
}

#[test]
fn a_hold_released_without_a_step_leaves_no_undo_step_or_modified_mark() {
    let mut rig = with_effects(&[6, 11]);
    rig.app.files.saved();
    let steps = rig.app.history.steps();
    let name = card_name(&rig, 2, 1);
    rig.press(name);
    let pressed = rig.time;
    rig.next_frame_at(pressed + 0.3);
    rig.frame(vec![]);
    rig.frame(vec![]);
    rig.release(name);
    rig.frame(vec![]);
    assert!(!rig.app.files.modified());
    assert_eq!(rig.take(), Vec::<String>::new());
    assert_eq!(rig.app.history.steps(), steps);
}

#[test]
fn a_rack_preset_for_a_new_channel_past_the_id_limit_adds_nothing_and_says_why() {
    // The add-channel `Rack preset` path, whose channel comes with the pick.
    let (mut rig, _) = with_web_files(&[51, 6]);
    save_rack_preset(&mut rig, 2);
    rig.app.navigate(Nav::Close);
    rig.frame(vec![]);
    rig.app.song.file.next_channel_id = crate::song::CHANNEL_ID_LIMIT;
    let song = rig.app.song.clone();
    rig.app.add_channel(ChannelKind::Rack, 0);
    picker_row(&mut rig, 0);
    picker_row(&mut rig, 0);
    assert_eq!(message(&rig), Some(vec![CHANNEL_LIMIT.to_owned()]));
    assert_eq!(rig.app.song, song);
}

#[test]
fn the_level_dialog_marks_its_frames_and_holds_undo_under_it() {
    // A dialog with no text field, so only its backdrop holds the keys.
    let mut rig = with_effects(&[48]);
    let card = layout(&rig, 2).cards[0];
    rig.click(pos2(card.left() + 2.25 * U, card.top() + 0.75 * U));
    rig.frame(vec![]);
    assert!(crate::ui::modal_was_open(&rig.ctx), "the dialog is open");
    rig.frame(z(egui::Modifiers::NONE));
    assert_eq!(types(&rig, 2), [48], "held under the dialog");
    rig.frame(escape());
    rig.frame(vec![]);
    // One step back: the effect track's add (the Transpose's made none).
    let n = rig.app.song.channels.len();
    rig.frame(z(egui::Modifiers::NONE));
    assert_eq!(rig.app.song.channels.len(), n - 1);
}

#[test]
fn a_slicer_loop_picked_in_the_real_picker_names_its_file_in_its_cstm() {
    // A loop is a file the module loader names in the `CSTM` (`load_pick`); its sample decodes on the worker first,
    // and the pick applies once it lands. A file that does not decode is named all the same.
    let mut rig = Rig::with_synth();
    let mem = MemFiles::default();
    mem.0.borrow_mut().names_only = true;
    rig.app.files = ProjectFiles::with_provider(Box::new(mem));
    rig.app
        .add_channel(ChannelKind::Synth(SynthKind::Slicer), 0);
    rig.app.song_edited(SongEdit::Final, 0);
    rig.open_rack(2);
    rig.slide_rack();
    rig.take();
    let path = Path::new("My Presets/Slicer/Loop.wav");
    rig.app.files.keep_user_file(path, b"RIFF".to_vec());
    rig.click_id(preset_button(2, instance_id(&rig, 2, 0)));
    // The worker is held: the loop waits for it.
    let hold = crate::ui::views::preset_picker::WORKER_HOLD
        .lock()
        .unwrap_or_else(std::sync::PoisonError::into_inner);
    picker_row(&mut rig, 0);
    picker_row(&mut rig, 0);
    let before = rig.app.song.channels[2].rack.modules[0].clone();
    rig.frame(vec![]);
    assert!(rig.app.slots.preset_picker.loading(), "on the worker");
    assert_eq!(rig.app.song.channels[2].rack.modules[0], before);
    drop(hold);
    worker_done(&mut rig);
    // The picker stays open.
    let id = instance_id(&rig, 2, 0);
    assert_eq!(
        rig.overlays(),
        [Overlay::Rack(2), picker(2, PresetTarget::Module(id))]
    );
    assert_eq!(message(&rig), None);
    let module = &rig.app.song.channels[2].rack.modules[0];
    let blob = module.file.custom_data.as_deref().expect("a CSTM");
    let named = b"@L@\0My Presets/Slicer/Loop.wav";
    assert!(blob.windows(named.len()).any(|w| w == named), "{blob:?}");
    assert_eq!(module.file.preset_name, "Loop");
    assert_eq!(rig.take(), ["ReplaceSong adopting no-resync"]);
}

#[test]
fn a_slicer_preset_loads_on_the_worker_and_changes_nothing_until_it_is_done() {
    let mut rig = Rig::with_synth();
    let mem = MemFiles::default();
    mem.0.borrow_mut().names_only = true;
    rig.app.files = ProjectFiles::with_provider(Box::new(mem));
    rig.app
        .add_channel(ChannelKind::Synth(SynthKind::Slicer), 0);
    rig.app.song_edited(SongEdit::Final, 0);
    rig.open_rack(2);
    rig.slide_rack();
    rig.take();
    let mut loud = ModuleInstance::new(33, 0);
    loud.params[1] = 0.77;
    let bytes = crate::flm::write_preset(&loud).unwrap();
    rig.app
        .files
        .keep_user_file(Path::new("My Presets/Slicer/Loud.flmpst"), bytes);
    let id = instance_id(&rig, 2, 0);
    let before = rig.app.song.channels[2].rack.modules[0].clone();
    assert_ne!(before.params[1], 0.77);
    let hold = crate::ui::views::preset_picker::WORKER_HOLD
        .lock()
        .unwrap_or_else(std::sync::PoisonError::into_inner);
    rig.click_id(preset_button(2, id));
    picker_row(&mut rig, 0);
    picker_row(&mut rig, 0);
    rig.frame(vec![]);
    // Still loading: the picker is open (drawing nothing) and the module is as it was.
    let loading = picker(2, PresetTarget::Module(id));
    assert_eq!(rig.overlays(), [Overlay::Rack(2), loading]);
    assert_eq!(rig.app.song.channels[2].rack.modules[0], before);
    assert_eq!(rig.take(), Vec::<String>::new());
    drop(hold);
    worker_done(&mut rig);
    assert_eq!(message(&rig), None);
    let module = &rig.app.song.channels[2].rack.modules[0];
    assert_eq!(module.params[1], 0.77);
    assert_eq!(module.file.preset_name, "Loud");
    assert_eq!(rig.take(), ["ReplaceSong adopting no-resync"]);
}

// ---- Chord sets ----

/// The `ChordSet` picker, opened over the rig's channel 1 (the song's set belongs to no channel). Its root rows are `My
/// Files`, `Default`, then our eight sets.
fn open_chord_sets(rig: &mut Rig) {
    let target = PresetTarget::ChordSet;
    rig.app.navigate(Nav::Open(picker(1, target)));
    assert_eq!(rig.overlays(), [picker(1, target)]);
}

#[test]
fn picking_a_factory_chord_set_saves_it_into_keyb_in_one_undo_step() {
    use crate::song::chords::ChordSet;
    let mut rig = Rig::with_synth();
    // About our factory's content, which the rig leaves out.
    rig.app.files.factory = crate::content::factory();
    let before = rig.app.song.file.keyboard.clone();
    assert_eq!(before.chord_set_name, "Major");
    rig.take();
    open_chord_sets(&mut rig);
    // `Soul Sevenths` is the last of our eight, at row 9 (after `My Files` and `Default`).
    picker_row(&mut rig, 9);
    assert_eq!(rig.overlays(), []);
    let keyb = rig.app.song.file.keyboard.clone();
    assert_eq!(keyb.chord_set_name, "Soul Sevenths");
    let set = ChordSet::parse(&keyb.chord_json).unwrap();
    assert_eq!(set.to_json(), keyb.chord_json);
    // Its first strip is the C major seventh chord, and it has all eight.
    let first = &set.chords[0];
    assert_eq!(
        (set.chords.len(), first.base_note, first.quality),
        (8, 0, 0)
    );
    assert_eq!(first.extensions, [11]);
    // Only the set, its name and its state changed: loaded.
    assert_eq!(keyb.set_state, crate::song::SET_LOADED);
    let rest = crate::song::KeyboardState {
        chord_json: before.chord_json.clone(),
        chord_set_name: before.chord_set_name.clone(),
        set_state: before.set_state,
        ..keyb.clone()
    };
    assert_eq!(rest, before);
    // KEYB is not the audio's: an undo step with no new engine.
    assert_eq!(rig.take(), Vec::<String>::new());
    // KEYB carries it through a save and a load.
    let saved = crate::flm::write(&rig.app.song).unwrap();
    let loaded = crate::flm::read(&saved).unwrap();
    assert_eq!(loaded.file.keyboard, keyb);
    // One undo step.
    rig.frame(z(egui::Modifiers::NONE));
    assert_eq!(rig.app.song.file.keyboard, before);
}

#[test]
fn default_puts_the_built_in_major_set_back() {
    let mut rig = Rig::with_synth();
    // About our factory's content, which the rig leaves out.
    rig.app.files.factory = crate::content::factory();
    let before = rig.app.song.file.keyboard.clone();
    open_chord_sets(&mut rig);
    picker_row(&mut rig, 4);
    assert_eq!(rig.app.song.file.keyboard.chord_set_name, "Epic Rise");
    open_chord_sets(&mut rig);
    picker_row(&mut rig, 1);
    assert_eq!(rig.overlays(), []);
    assert_eq!(rig.app.song.file.keyboard, before);
    assert_eq!(
        rig.app.song.file.keyboard.chord_json,
        crate::song::DEFAULT_CHORD_JSON
    );
}

#[test]
fn a_users_chord_set_loads_by_its_name_and_a_bad_file_shows_the_error_and_changes_nothing() {
    let mut rig = Rig::with_synth();
    let good = crate::song::chords::ChordSet::parse(crate::song::DEFAULT_CHORD_JSON).unwrap();
    let files = &mut rig.app.files;
    files.keep_user_file(
        Path::new("My Chord Sets/Mine.chords"),
        good.to_json().into_bytes(),
    );
    let empty = br#"{"chords": [], "rootNote": 0}"#;
    files.keep_user_file(Path::new("My Chord Sets/Empty.chords"), empty.to_vec());
    files.keep_user_file(
        Path::new("My Chord Sets/Garbled.chords"),
        b"{ not json".to_vec(),
    );
    let song = rig.app.song.clone();
    rig.take();

    // `My Files` lists the user's A-Z: Empty, Garbled, Mine. A bad file: the load error, no edit.
    for (row, error) in [(0, "needs 8 chords"), (1, "key must be a string")] {
        open_chord_sets(&mut rig);
        picker_row(&mut rig, 0);
        picker_row(&mut rig, row);
        let lines = message(&rig).expect("a message");
        assert_eq!(lines[0], crate::project_files::LOAD_FAILED);
        assert!(lines[1].contains(error), "{lines:?}");
        assert_eq!(rig.overlays().len(), 1, "the message replaces the picker");
        assert_eq!(rig.app.song, song);
        assert_eq!(rig.take(), Vec::<String>::new());
        rig.app.navigate(Nav::Close);
    }

    // A good one is named by its file.
    open_chord_sets(&mut rig);
    picker_row(&mut rig, 0);
    picker_row(&mut rig, 2);
    assert_eq!(rig.overlays(), []);
    assert_eq!(rig.app.song.file.keyboard.chord_set_name, "Mine");
    // KEYB is not the audio's: an undo step with no new engine.
    assert_eq!(rig.take(), Vec::<String>::new());
}

/// The web's module pickers that list audio files (the DW Sampler's, the Slicer's, 3x Osc's) offer `Import` as the
/// sample picker does: the file goes into the module's user folder, `My Presets/<module>`, and the open picker lists it
/// at once. A picker without audio files (MiniSynth's) has none, nor has the desktop's.
#[test]
fn the_web_module_pickers_that_list_audio_import_into_the_modules_user_folder() {
    for (kind, import) in [
        (SynthKind::DwSampler, true),
        (SynthKind::Slicer, true),
        (SynthKind::ThreeXOsc, true),
        (SynthKind::MiniSynth, false),
    ] {
        let mut rig = Rig::with_synth();
        let mem = MemFiles::with_file("Loop.wav", b"RIFF picked");
        rig.app.files = ProjectFiles::with_provider(Box::new(mem.clone()));
        rig.app.add_channel(ChannelKind::Synth(kind), 0);
        let module = &rig.app.song.channels[2].rack.modules[0];
        let (id, type_id) = (module.instance_id, module.type_id);
        rig.app
            .navigate(Nav::Open(picker(2, PresetTarget::Module(id))));
        picker_row(&mut rig, 0);
        rig.frame(vec![]);
        assert_eq!(rig.color_of("Import").is_some(), import, "{kind:?}");
        if !import {
            continue;
        }
        assert!(rig.color_of("Loop").is_none(), "{kind:?}");
        rig.click_id(Id::new(PICKER).with("import"));
        rig.frame(vec![]);
        let dir = rig.app.files.presets_dir(type_id).unwrap();
        let kept = rig.app.files.read_user_file(&dir.join("Loop.wav"));
        assert_eq!(kept.unwrap(), b"RIFF picked", "{kind:?}");
        assert_eq!(rig.overlays(), [picker(2, PresetTarget::Module(id))]);
        rig.frame(vec![]);
        assert!(rig.color_of("Loop").is_some(), "{kind:?} lists it");
    }
    // The desktop's DW Sampler picker has none.
    let mut rig = Rig::with_synth();
    rig.app.files = ProjectFiles::on_disk(&std::env::temp_dir().join("honeydew-l6-no-such-folder"));
    rig.app
        .add_channel(ChannelKind::Synth(SynthKind::DwSampler), 0);
    let id = rig.app.song.channels[2].rack.modules[0].instance_id;
    rig.app
        .navigate(Nav::Open(picker(2, PresetTarget::Module(id))));
    picker_row(&mut rig, 0);
    rig.frame(vec![]);
    assert!(rig.color_of("Search").is_some() && rig.color_of("Import").is_none());
}

/// `Set Range` learns from typed keys too, which take the touched keys' path.
#[test]
fn set_range_learns_from_typed_keys() {
    let mut rig = Rig::with_synth();
    rig.open_rack(1);
    rig.slide_rack();
    card_menu(&mut rig, 1, 0, (0, 4));
    rig.frame(vec![]);
    // V (53) then C (52): the range is 52..=53.
    for at in [egui::Key::V, egui::Key::C] {
        rig.type_key(at, true);
        rig.type_key(at, false);
        rig.frame(vec![]);
    }
    let module = &rig.app.song.channels[1].rack.modules[0];
    assert_eq!((module.file.key_low, module.file.key_high), (52, 53));
}

// ---- Our preset banks ----

/// The texts painted inside each of the open picker's rows, every row the last frame drew. (A row low on the screen
/// also holds the transport bar's texts painted under the panel.)
fn picker_texts(rig: &Rig) -> Vec<Vec<String>> {
    let row = |i: usize| rig.ctx.read_response(Id::new(PICKER).with(("row", i)));
    (0..)
        .map_while(|i| row(i).map(|r| texts_in(rig, r.rect)))
        .collect()
}

#[test]
fn our_banks_list_at_each_synths_picker_root_and_a_pick_makes_no_undo_step() {
    use crate::content::{ROOT, factory};
    use crate::ui::main_menu::SongLibrary;
    let mut rig = Rig::with_synth();
    // About our factory's content, which the rig leaves out.
    rig.app.files.factory = crate::content::factory();
    rig.app
        .add_channel(ChannelKind::Synth(SynthKind::SuperSaw), 0);
    rig.app
        .add_channel(ChannelKind::Synth(SynthKind::TransistorBass), 0);
    // all five synths list a bank.
    rig.app
        .add_channel(ChannelKind::Synth(SynthKind::GmSynth), 0);
    rig.app
        .add_channel(ChannelKind::Synth(SynthKind::ThreeXOsc), 0);
    // Transistor Bass's presets list at its root, in index order.
    let tb = factory()
        .list(Some(&Path::new(ROOT).join("Presets/TransistorBass")))
        .unwrap();
    let tb: Vec<String> = tb.iter().map(|e| e.name.replace(".flmpst", "")).collect();
    assert_eq!(tb.len(), 8);
    for (channel, rows) in [
        (
            1,
            vec!["My Files", "Default", "Bass", "Keys", "Lead", "Pad"],
        ),
        (
            2,
            vec!["My Files", "Default", "Bass", "Lead", "Pad", "Pluck"],
        ),
        (
            3,
            ["My Files", "Default"]
                .into_iter()
                .chain(tb.iter().map(String::as_str))
                .collect(),
        ),
        (
            4,
            vec![
                "My Files", "Default", "Bass", "Bells", "Keys", "Lead", "Pad",
            ],
        ),
        (
            5,
            vec!["My Files", "Default", "Bass", "Lead", "Pad", "Synth"],
        ),
    ] {
        rig.open_rack(channel);
        rig.slide_rack();
        rig.click_id(preset_button(channel, instance_id(&rig, channel, 0)));
        for _ in 0..3 {
            rig.frame(vec![]);
        }
        let texts = picker_texts(&rig);
        assert_eq!(texts.len(), rows.len(), "channel {channel}: {texts:?}");
        for (texts, row) in texts.iter().zip(&rows) {
            assert!(
                texts.iter().any(|t| t == row),
                "channel {channel}: {row} in {texts:?}"
            );
        }
        rig.frame(escape());
        rig.frame(vec![]);
    }
    // Each `Bass`'s first preset: its params (GMSynth: and its CSTM shapes), the module and the channel named for it,
    // no undo step. MiniSynth and GMSynth load on the UI thread, 3x Osc on the worker.
    for (channel, type_id, folder, preset) in [
        (1, 0, "MiniSynth", "Gravel Bass"),
        (4, 2, "GMSynth", "Iron Sub"),
        (5, 39, "3xOsc", "Floorboard"),
    ] {
        rig.open_rack(channel);
        rig.slide_rack();
        rig.take();
        let before = rig.app.song.channels[channel].clone();
        let bass = Path::new(ROOT).join("Presets").join(folder).join("Bass");
        let first = factory().list(Some(&bass)).unwrap().remove(0).path;
        let bytes = factory().read(&first).unwrap();
        let mut expected = before.rack.modules[0].clone();
        expected.load_preset(crate::flm::read_preset(&bytes, Some(type_id)).unwrap());
        expected.name_preset(&first, Path::new(ROOT));
        // A pick keeps its source path relative to its root (`ProjectFiles::name_module`).
        expected.file.source_path = expected.file.preset_path.clone();
        let name = expected.file.preset_name.clone();
        // The preset `find` gives a template (`content::presets` tests: the same module but its instance id).
        assert_eq!(name, preset);
        let steps = rig.app.history.steps();
        rig.click_id(preset_button(channel, expected.instance_id));
        picker_row(&mut rig, 2);
        picker_row(&mut rig, 0);
        worker_done(&mut rig);
        // The picker stays open.
        let target = PresetTarget::Module(expected.instance_id);
        let open = [Overlay::Rack(channel), picker(channel, target)];
        assert_eq!(rig.overlays(), open, "{preset}");
        let now = |rig: &Rig| rig.app.song.channels[channel].clone();
        assert_eq!(now(&rig).rack.modules[0], expected, "{preset}");
        assert_ne!(expected.params, before.rack.modules[0].params, "{preset}");
        assert_eq!(expected.file.category, format!("Presets/{folder}/Bass"));
        if type_id == 2 {
            // Smooth Sub and Square Retro, the slots `gm_synth` gives Iron Sub.
            let blob = expected.file.custom_data.as_deref().expect("a CSTM");
            let shapes = crate::panels::cstm::settings_from_cstm(2, blob).unwrap();
            assert_eq!((shapes[&21100], shapes[&21200]), (19.0, 20.0));
        }
        assert_eq!(
            (&now(&rig).name, &now(&rig).file.preset_name),
            (&name, &name)
        );
        assert_eq!(rig.take(), ["ReplaceSong adopting no-resync"], "{preset}");
        assert_eq!(rig.app.history.steps(), steps, "{preset}: no undo step");
        close_picker(&mut rig);
    }
}

/// A GMSynth's OSC shape is a live setting (its table is built on the app thread first), not an engine rebuild,
/// and the running engine plays it as an engine built from the edited song does.
///
/// Its guard that the rack calls `settings::prepare` first (`GmSynth::set_setting`'s debug_assert, "prepare_setting
/// builds slot 1 first") only fires while shape slot 1 is unbuilt: the shape tables are process-global, so another test
/// in this binary that builds slot 1 hides a missing `prepare`. Run alone (`cargo test -p honeydew --lib
/// a_gmsynth_osc_shape`) it is exact. (`settings` is `pub(crate)`, so no `tests/` binary can reach `prepare`; the
/// "no ReplaceSong" half does not depend on the cache.)
#[test]
fn a_gmsynth_osc_shape_edit_is_live_and_the_running_engine_plays_it() {
    use crate::audio_engine::SongAudioEngine;
    use honeydew_audio::device::AudioRenderer;
    let mut rig = Rig::with_synth();
    rig.app
        .add_channel(ChannelKind::Synth(SynthKind::GmSynth), 0);
    rig.open_rack(2);
    rig.slide_rack();
    let id = instance_id(&rig, 2, 0);
    let control = |rig: &Rig, n: u32| {
        let panel = rack_id(2).with(("module panel", id));
        let found = rig.ctx.read_response(panel.with(n));
        found.expect("a control").rect
    };
    // The OSC tab, the second of eight.
    let tabs = control(&rig, 0);
    rig.click(pos2(
        tabs.left() + tabs.width() * 1.5 / 8.0,
        tabs.center().y,
    ));
    rig.frame(vec![]);
    let render = |engine: &mut SongAudioEngine| {
        engine.note_on(2, 60, 1.0);
        let mut output = vec![0.0; 4096 * 2];
        engine.render(&mut output, 2);
        output
    };
    let old = render(&mut SongAudioEngine::new(&rig.app.song, 48_000.0).unwrap());
    assert!(old.iter().any(|sample| sample.abs() > 0.01));
    rig.take();
    // `A`'s `v` chevron: the next shape.
    let spinner = control(&rig, 21100);
    rig.click(pos2(spinner.center().x, spinner.bottom() - 0.1 * U));
    rig.frame(vec![]);
    let wave = |rig: &Rig| {
        let blob = rig.app.song.channels[2].rack.modules[0]
            .file
            .custom_data
            .as_deref();
        settings_from_cstm(2, blob.unwrap()).map(|settings| settings[&21100])
    };
    assert_eq!(wave(&rig), Some(1.0), "stored in the song");
    // Sent as a setting, with no new engine.
    assert_eq!(rig.take(), [format!("SetSetting 2 {id} 21100 1.000")]);
    let live = render(rig.rec.0.borrow_mut().engine.as_mut().unwrap());
    let rebuilt = render(&mut SongAudioEngine::new(&rig.app.song, 48_000.0).unwrap());
    assert!(live != old, "the live engine still plays the old shape");
    assert!(live == rebuilt, "the live shape is not the song's");
}

/// Keyboard focus: a touched knob is the focused control, which takes Ctrl+= and Ctrl+− before anything else, with
/// no undo step; Tab turns the cursor on, a touch turns it off.
#[test]
fn a_touched_rack_knob_takes_increment_and_decrement_with_no_undo_step() {
    use crate::ui::focus;
    let mut rig = with_effects(&[6]);
    let steps = rig.app.history.steps();
    // As the app sets it up (`setup_context`): Ctrl+= is no egui zoom.
    rig.ctx.options_mut(|o| o.zoom_with_keyboard = false);
    let id = instance_id(&rig, 2, 0);
    let content = content_rect(layout(&rig, 2).cards[0], 5.5, U);
    let module = rig.app.song.channels[2].rack.modules[0].clone();
    let panel = panels::panel_for(6).unwrap();
    let placed = panels::place(panel, &module, &PanelState::default(), content, U);
    let drive_at = placed.iter().find(|(l, _)| l.id == 3).unwrap().1.center();
    let drive = |rig: &Rig| rig.app.song.channels[2].rack.modules[0].params[1];
    let cmd = egui::Modifiers::COMMAND;
    // Pressed and released: a key held over the next press makes that a repeat, which the focus actions ignore.
    let key_event = |key, modifiers| {
        let release = egui::Event::Key {
            key,
            physical_key: None,
            pressed: false,
            repeat: false,
            modifiers,
        };
        [key_event(key, modifiers), release]
    };
    // Nothing focused: Ctrl+= is not the knob's.
    rig.frame(key_event(egui::Key::Equals, cmd).to_vec());
    assert_eq!((drive(&rig), rig.take()), (0.25, vec![]));
    rig.click(drive_at);
    assert!(focus::focused(&rig.ctx).is_some());
    assert!(!focus::is_keyboard_mode(&rig.ctx));
    rig.frame(key_event(egui::Key::Equals, cmd).to_vec());
    assert!((drive(&rig) - 0.3).abs() < 1e-6, "{}", drive(&rig));
    assert_eq!(rig.take(), [format!("SetParam 2 {id} 1 0.300")]);
    rig.frame(key_event(egui::Key::Minus, cmd | egui::Modifiers::SHIFT).to_vec());
    assert!((drive(&rig) - 0.2875).abs() < 1e-6, "{}", drive(&rig));
    rig.take();
    // No undo step.
    assert_eq!(rig.app.history.steps(), steps);
    // Tab: the cursor shows, a 9 px frame over everything; a touch takes it away.
    let cursor = |rig: &Rig| {
        (rig.out.shapes.iter())
            .any(|s| matches!(&s.shape, egui::Shape::Rect(r) if r.stroke.width == 9.0))
    };
    assert!(!cursor(&rig));
    rig.frame(key_event(egui::Key::Tab, egui::Modifiers::NONE).to_vec());
    assert!(focus::is_keyboard_mode(&rig.ctx));
    assert!(cursor(&rig));
    rig.click(drive_at);
    assert!(!focus::is_keyboard_mode(&rig.ctx));
    assert!(!cursor(&rig));
}

#[test]
fn a_rack_sliding_out_takes_no_wheel() {
    let mut rig = with_effects(&[3, 7, 26, 30]);
    let top = |rig: &Rig| {
        let name = |s: &egui::epaint::ClippedShape| match &s.shape {
            egui::Shape::Text(t) if t.galley.text() == "Equalizer" => Some(t.pos.y),
            _ => None,
        };
        rig.out
            .shapes
            .iter()
            .find_map(name)
            .expect("the Equalizer card")
    };
    let before = top(&rig);
    rig.click(collapse_tab());
    assert_eq!(rig.overlays(), []);
    // Over what is left of the rack, half way out.
    let wheel = Event::MouseWheel {
        unit: egui::MouseWheelUnit::Point,
        delta: vec2(0.0, -200.0),
        modifiers: Default::default(),
    };
    rig.frame(vec![Event::PointerMoved(pos2(1100.0, 300.0)), wheel]);
    for _ in 0..3 {
        assert_eq!(top(&rig), before);
        rig.frame(vec![]);
    }
}

#[test]
fn the_rack_slides_a_step_a_frame_however_many_passes_the_frame_runs() {
    let ctx = egui::Context::default();
    let mut state = views::rack::State::default();
    let mut open = Vec::new();
    for _ in 0..2 {
        let _ = ctx.run(egui::RawInput::default(), |ctx| {
            open.push(state.slide(true, 1170.0, ctx));
            // A pass whose layout changed runs again.
            ctx.request_discard("a second pass");
        });
    }
    assert_eq!(open, [0.5, 0.5, 0.75, 0.75]);
}

#[test]
fn the_rack_lends_its_cards_set_middle_c_to_c4() {
    let mut rig = with_effects(&[24]);
    assert!(!rig.app.slots.rack.panel().middle_c_is_c4);
    rig.app.settings.middle_c_is_c4 = true;
    rig.frame(vec![]);
    assert!(rig.app.slots.rack.panel().middle_c_is_c4);
}

#[test]
fn picking_the_active_modules_sort_still_saves_its_id() {
    // Preset picker ordering: a pick writes its id at once, the active one included; on a
    // fresh install, picking Category stores 3 in place of the missing −1.
    use crate::ui::modules_browser::ModuleOrdering;
    let mut rig = Rig::with_synth();
    rig.open_rack(1);
    rig.slide_rack();
    rig.click(layout(&rig, 1).add_module.center());
    for _ in 0..3 {
        rig.frame(vec![]);
    }
    let fresh = rig.app.settings.module_browser_ordering;
    assert_eq!(ModuleOrdering::from_id(fresh), None);
    rig.click_id(Id::new(BROWSER).with("sort"));
    for _ in 0..25 {
        rig.frame(vec![]);
    }
    // The menu's second row: Category, the active one.
    rig.click_id(Id::new(BROWSER).with("sort menu").with((0, 1)));
    rig.frame(vec![]);
    let category = ModuleOrdering::Category.id();
    assert_eq!(rig.app.settings.module_browser_ordering, category);
    assert!(rig.painted("SYNTH"));
}

#[test]
fn the_modules_sort_is_saved_as_module_browser_ordering_on_a_pick_and_applied_at_each_opening() {
    // Preset picker ordering: each opening makes Category active, unless the saved
    // `ModuleBrowserOrdering` names an ordering; a pick from `Sort` writes its id at once, an opening writes nothing.
    use crate::ui::modules_browser::ModuleOrdering;
    let mut rig = Rig::with_synth();
    rig.open_rack(1);
    rig.slide_rack();
    let add = layout(&rig, 1).add_module;
    let open = |rig: &mut Rig| {
        rig.click(add.center());
        for _ in 0..3 {
            rig.frame(vec![]);
        }
    };
    let fresh = rig.app.settings.module_browser_ordering;
    assert_eq!(
        ModuleOrdering::from_id(fresh),
        None,
        "missing on a fresh install"
    );
    open(&mut rig);
    assert!(rig.painted("SYNTH"), "Category, with its sections");
    assert_eq!(rig.app.settings.module_browser_ordering, fresh);
    rig.click_id(Id::new(BROWSER).with("sort"));
    for _ in 0..25 {
        rig.frame(vec![]);
    }
    // The menu's first row: A-Z.
    rig.click_id(Id::new(BROWSER).with("sort menu").with((0, 0)));
    rig.frame(vec![]);
    let az = ModuleOrdering::AZ.id();
    assert_eq!(rig.app.settings.module_browser_ordering, az);
    assert!(!rig.painted("SYNTH"));
    close_picker(&mut rig);
    assert_eq!(rig.overlays(), [Overlay::Rack(1)]);
    open(&mut rig);
    assert!(!rig.painted("SYNTH"), "restored");
    close_picker(&mut rig);
    // The stored value is what an opening applies.
    rig.app.settings.module_browser_ordering = ModuleOrdering::Category.id();
    open(&mut rig);
    assert!(rig.painted("SYNTH"));
}

/// Tremolo (ID 46) `Max` (p3) and `Min` (p4) after a song edit to `(max, min)`, its rack open.
fn tremolo_rig(max: f32, min: f32) -> Rig {
    let mut rig = with_effects(&[46]);
    let params = &mut rig.app.song.channels[2].rack.modules[0].params;
    (params[3], params[4]) = (max, min);
    rig.app.song_edited(SongEdit::Final, 0);
    rig.frame(vec![]);
    rig.take();
    rig
}

/// The Tremolo's `(Max, Min)` in the song.
fn limits(rig: &Rig) -> (f32, f32) {
    let params = &rig.app.song.channels[2].rack.modules[0].params;
    (params[3], params[4])
}

#[test]
fn dragging_the_tremolos_max_below_min_lowers_min_in_the_song_live_with_no_undo_step() {
    // Max below Min lowers Min to it, as the effect does with the audio's copy.
    let mut rig = tremolo_rig(0.8, 0.5);
    rig.app.files.saved();
    let steps = rig.app.history.steps();
    let id = instance_id(&rig, 2, 0);
    let content = content_rect(layout(&rig, 2).cards[0], 9.0, U);
    let module = rig.app.song.channels[2].rack.modules[0].clone();
    let panel = panels::panel_for(46).unwrap();
    let placed = panels::place(panel, &module, &PanelState::default(), content, U);
    // The `Max` tag (#4) travels 0.88 of its height: from 0.8 down by 0.5.
    let max = placed.iter().find(|(l, _)| l.id == 4).unwrap().1;
    let travel = 0.88 * max.height();
    let at = pos2(
        max.center().x,
        max.bottom() - (0.06 + 0.88 * 0.8) * max.height(),
    );
    rig.press(at);
    for step in 1..=4 {
        rig.drag_to(at + vec2(0.0, 0.5 * travel * step as f32 / 4.0));
    }
    let (high, low) = limits(&rig);
    assert!((high - 0.3).abs() < 1e-3, "{high}");
    assert_eq!(low, high, "Min follows Max down, mid-drag");
    let sent = rig.take();
    assert_eq!(sent.last().unwrap(), &format!("SetParam 2 {id} 4 {low:.3}"));
    rig.release(at + vec2(0.0, 0.5 * travel));
    rig.frame(vec![]);
    // Channel params: the song-changed flag and no undo step; no undo or redo puts the old limits back.
    assert!(rig.app.files.modified());
    assert_eq!(rig.app.history.steps(), steps);
    undo_redo_the_track(&mut rig);
    assert_eq!(limits(&rig), (high, low));
}
